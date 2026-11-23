//! Gates 2 and 3: a candidate's programs run for real, without a model.
//!
//! Both gates run the engine's own `skillcache` commands inside the gate
//! sandbox: a minimal root, the candidate package read-only, the network off
//! unless the capsule declares the need. Gate 2 runs every program's own
//! check in an empty folder. Gate 3 replays the whole plan on a fresh copy of
//! the task with the observed values answering for the agent, and checks
//! its workspace effects against the observation. A host without a sandbox qualifies nothing:
//! there is no unconfined fallback.
use super::replay_result::{self, ObservedResult};
use crate::{HarnessError, Result};
use infra_host::{
    capture::{self, Stream},
    sandbox::{self, Spec},
};
use infra_skillcache::{
    SkillCapsule,
    candidate::Candidate,
    capsule::{Examples, Network, Step},
    env_filter,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const DETAIL_BYTES: usize = 4 * 1024;

/// Where one round's gates work, all below one compilation's private folder.
pub struct Round {
    /// The gates' control folder: the candidate package and both workspaces.
    pub gatebox: PathBuf,
    /// The replay's state, writable in Gate 3 and outside the control folder.
    pub state: PathBuf,
    /// The observed values, readable in Gate 3 only.
    pub examples: PathBuf,
    pub round: i64,
}
impl Round {
    pub fn new(compile_dir: &Path, round: i64) -> Self {
        Self {
            gatebox: compile_dir.join("gatebox"),
            state: compile_dir.join(format!("gatestate/r{round}")),
            examples: compile_dir.join(format!("examples/r{round}")),
            round,
        }
    }
    pub fn package(&self) -> PathBuf {
        self.gatebox.join(format!("r{}-package/warm", self.round))
    }
    fn workspace(&self, gate: &str) -> PathBuf {
        self.gatebox.join(format!("r{}-{gate}", self.round))
    }
}

/// The engine the gates run and what must be readable for it.
pub struct Engine {
    /// `None`: this executable, through its pinned copy in the sandbox.
    pub path: Option<PathBuf>,
    /// The Codex runtime tree needed on Windows when gates run inside AppContainer.
    pub codex: Option<PathBuf>,
}
impl Engine {
    fn command(&self, workspace: &Path) -> Result<(String, Vec<PathBuf>)> {
        let text = |path: &Path| {
            path.to_str()
                .map(String::from)
                .ok_or_else(|| HarnessError::invalid("The engine path is not UTF-8."))
        };
        match &self.path {
            Some(path) => Ok((
                text(path)?,
                path.parent().map(Path::to_path_buf).into_iter().collect(),
            )),
            None => Ok((text(&sandbox::trusted_helper(workspace)?)?, vec![])),
        }
    }
}

struct Ran {
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    seconds: f64,
    stopped: Option<&'static str>,
}

fn run_sandboxed(
    spec: &Spec,
    argv: &[String],
    environment: &BTreeMap<String, String>,
    seconds: u64,
    cancel: &Arc<AtomicBool>,
) -> Result<Ran> {
    let command = sandbox::gate(spec, argv)?;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let started = Instant::now();
    let ran = capture::run(
        capture::Request {
            argv: &command,
            cwd: &spec.workspace,
            environment,
            input: &[],
            output_limit: OUTPUT_BYTES,
            deadline: started + Duration::from_secs(seconds),
        },
        || cancel.load(Ordering::Acquire),
        |_| {},
        |stream, bytes| {
            match stream {
                Stream::Stdout => stdout.extend_from_slice(bytes),
                Stream::Stderr => stderr.extend_from_slice(bytes),
            }
            Ok(())
        },
    );
    let seconds = started.elapsed().as_secs_f64();
    match ran {
        Ok(outcome) => Ok(Ran {
            code: outcome.exit,
            stdout,
            stderr,
            seconds,
            stopped: None,
        }),
        Err(error) if error.code == "native-timeout" => Ok(Ran {
            code: None,
            stdout,
            stderr,
            seconds,
            stopped: Some("timeout"),
        }),
        Err(error) if error.code == "output-limit" => Ok(Ran {
            code: None,
            stdout,
            stderr,
            seconds,
            stopped: Some("output_limit"),
        }),
        Err(error) => Err(error),
    }
}

fn detail(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim();
    let start = text.len().saturating_sub(DETAIL_BYTES);
    let mut start = start;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    match infra_core::domain::mask_secrets(&text[start..]) {
        Ok((masked, _)) => masked,
        Err(_) => "[diagnostics withheld: a possible secret could not be masked]".into(),
    }
}

/// Whether a failure reads like the sandbox's minimal root rather than the
/// program's own fault. The gate is deliberately stricter than a session;
/// the diagnostic says so instead of leaving a bare "not found".
fn sandbox_denied(text: &str) -> bool {
    [
        "Permission denied",
        "Operation not permitted",
        "Read-only file system",
        "No such file or directory",
        "command not found",
        "is not installed on this machine",
        "Network is unreachable",
        "Temporary failure in name resolution",
        "Could not resolve host",
    ]
    .iter()
    .any(|sign| text.contains(sign))
}

fn environment(
    engine: &Engine,
    package: &Path,
    workspace: &Path,
    scratch: &Path,
    state_file: Option<&Path>,
) -> BTreeMap<String, String> {
    let mut environment = std::env::vars()
        .filter(|(name, _)| {
            !env_filter::withheld(name) && !name.to_ascii_uppercase().starts_with("SKILLCACHE_")
        })
        .collect::<BTreeMap<_, _>>();
    let text = |path: &Path| path.display().to_string();
    // A named engine's folder is where its runtimes are looked for first.
    if let Some(folder) = engine.path.as_deref().and_then(Path::parent) {
        let rest = env_filter::take_path(&mut environment).unwrap_or_default();
        let joined = std::env::join_paths(
            std::iter::once(folder.to_path_buf()).chain(std::env::split_paths(&rest)),
        )
        .ok()
        .and_then(|path| path.into_string().ok());
        if let Some(path) = joined {
            environment.insert("PATH".into(), path);
        }
    }
    environment.insert(env_filter::PACKAGE.into(), text(package));
    environment.insert(env_filter::WORKSPACE.into(), text(workspace));
    if let Some(state_file) = state_file {
        environment.insert(env_filter::STATE_FILE.into(), text(state_file));
    }
    environment.insert(env_filter::GENERATION.into(), "gate".into());
    // The host's home is not in the gate's root; programs get a scratch one.
    for name in ["HOME", "TMPDIR", "TMP", "TEMP"] {
        environment.insert(name.into(), text(scratch));
    }
    environment
}

/// Resolves declared runtime executables into read-only sandbox directories
/// without exposing `$HOME`.
fn gate_runtime_dirs(runtimes: &[String]) -> Vec<PathBuf> {
    let path_env = std::env::var_os("PATH");
    let path_str = path_env.as_ref().and_then(|value| value.to_str());
    let mut dirs = Vec::new();
    for runtime in runtimes {
        for dir in sandbox::runtime_dirs_for_argv(std::slice::from_ref(runtime), path_str) {
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        if let Some(exe) = infra_host::provider::find_executable(runtime)
            && let Ok(canon) = std::fs::canonicalize(&exe)
        {
            let canon_str = canon.to_string_lossy().into_owned();
            for dir in sandbox::runtime_dirs_for_argv(&[canon_str], path_str) {
                if !dirs.contains(&dir) {
                    dirs.push(dir);
                }
            }
        }
    }
    dirs
}

fn spec(
    workspace: &Path,
    readable: Vec<PathBuf>,
    writable: Vec<PathBuf>,
    runtime: Vec<PathBuf>,
    codex: Option<&Path>,
    network: bool,
) -> Spec {
    Spec {
        workspace: workspace.to_path_buf(),
        target: workspace.to_path_buf(),
        protected: vec![],
        readable,
        accounts: writable,
        runtime,
        codex: codex.map(Path::to_path_buf),
        evidence: None,
        network,
    }
}

fn unavailable(error: &HarnessError) -> Value {
    json!({
        "gate": "",
        "passed": false,
        "violations": [{
            "code": "sandbox_unavailable",
            "at": "",
            "message": format!(
                "The gate needs an operating-system sandbox and runs nothing without one: {error}"
            ),
        }],
    })
}
fn named(mut report: Value, gate: &str) -> Value {
    report["gate"] = gate.into();
    report
}

/// Gate 2: every program's `check`, each in a fresh sandbox with the package
/// read-only, an empty folder to work in, no task file and no network.
pub fn gate2(
    engine: &Engine,
    round: &Round,
    candidate: &Candidate,
    cancel: &Arc<AtomicBool>,
) -> Value {
    let package = round.package();
    let workspace = round.workspace("gate2");
    let mut programs = Vec::new();
    let mut violations = Vec::new();
    for (index, program) in candidate.capsule.programs.iter().enumerate() {
        let attempt = (|| -> Result<Ran> {
            let _ = std::fs::remove_dir_all(&workspace);
            infra_core::fs::ensure_directory(&workspace, true)?;
            let (executable, readable) = engine.command(&workspace)?;
            let scratch = workspace.join(".harness-tmp");
            infra_core::fs::ensure_directory(&scratch, true)?;
            run_sandboxed(
                &spec(
                    &workspace,
                    readable,
                    vec![],
                    gate_runtime_dirs(&candidate.capsule.capabilities.runtimes),
                    engine.codex.as_deref(),
                    false,
                ),
                &[
                    executable,
                    "skillcache".into(),
                    "tool".into(),
                    program.id.clone(),
                    "--check".into(),
                ],
                &environment(engine, &package, &workspace, &scratch, None),
                program.timeout_seconds + 15,
                cancel,
            )
        })();
        let ran = match attempt {
            Ok(ran) => ran,
            Err(error) => return named(unavailable(&error), "gate2"),
        };
        let answered = infra_core::strict_json::parse(
            String::from_utf8_lossy(&ran.stdout).trim().as_bytes(),
            1024 * 1024,
            16,
            10_000,
        )
        .ok()
        .is_some_and(|answer| answer["status"] == "complete");
        let passed = ran.code == Some(0) && answered && ran.stopped.is_none();
        let text = detail(&ran.stderr);
        let denied = !passed && sandbox_denied(&text);
        if !passed {
            violations.push(json!({
                "code": if denied { "sandbox_denied" } else { ran.stopped.unwrap_or("check_failed") },
                "at": format!("programs[{index}].check"),
                "message": format!(
                    "The self-test of `{}` did not pass in an empty sandbox without the network{}: {}",
                    program.id,
                    if denied { " (the gate shows only the system's runtime and the package)" } else { "" },
                    if text.is_empty() { "no diagnostic on standard error".into() } else { text.clone() }
                ),
            }));
        }
        programs.push(json!({
            "id": program.id,
            "passed": passed,
            "exit_code": ran.code,
            "seconds": ran.seconds,
            "sandbox_denied": denied,
        }));
    }
    let _ = std::fs::remove_dir_all(&workspace);
    json!({
        "gate": "gate2",
        "passed": violations.is_empty(),
        "programs": programs,
        "violations": violations,
    })
}

/// Gate 3: the end-to-end replay on a fresh copy of the task. `copy` makes
/// that copy below the folder it is given and returns the copy's workspace.
pub(super) fn gate3(
    engine: &Engine,
    round: &Round,
    candidate: &Candidate,
    copy: &dyn Fn(&Path) -> Result<Value>,
    expected: Option<&ObservedResult>,
    cancel: &Arc<AtomicBool>,
) -> Value {
    let Some(expected) = expected else {
        return json!({
            "gate": "gate3",
            "passed": false,
            "result_comparison": {"status": "unavailable"},
            "violations": [result_unavailable("Observe a new turn with a capturable workspace result.")],
        });
    };
    let package = round.package();
    let top = round.workspace("gate3");
    let attempt = (|| {
        let _ = std::fs::remove_dir_all(&top);
        let _ = std::fs::remove_dir_all(&round.state);
        infra_core::fs::ensure_directory(&round.state, true)?;
        let copied = copy(&top)?;
        let workspace = copied["workspace"]
            .as_str()
            .map(PathBuf::from)
            .ok_or_else(|| HarnessError::unavailable("The replay copy has no workspace."))?;
        let (executable, mut readable) = engine.command(&top)?;
        readable.push(round.examples.clone());
        let scratch = top.join(".harness-tmp");
        infra_core::fs::ensure_directory(&scratch, true)?;
        let seconds = candidate
            .capsule
            .programs
            .iter()
            .map(|program| 2 * program.timeout_seconds + 5)
            .sum::<u64>()
            + 30;
        let before = replay_result::capture_with(
            &workspace,
            None,
            &candidate.capsule.capabilities.allow_ignored,
        );
        let ran = run_sandboxed(
            &spec(
                &top,
                readable,
                vec![round.state.clone()],
                gate_runtime_dirs(&candidate.capsule.capabilities.runtimes),
                engine.codex.as_deref(),
                candidate.capsule.capabilities.network == Network::Required,
            ),
            &[
                executable,
                "skillcache".into(),
                "run".into(),
                "--replay".into(),
                round.examples.join("examples.json").display().to_string(),
            ],
            &environment(
                engine,
                &package,
                &workspace,
                &scratch,
                Some(&round.state.join("plan-state.json")),
            ),
            seconds,
            cancel,
        )?;
        Ok::<_, HarnessError>((ran, copied, workspace, before))
    })();
    let report = match attempt {
        Err(error) if error.code == "unavailable" && error.message.contains("isolation") => {
            named(unavailable(&error), "gate3")
        }
        Err(error) => json!({
            "gate": "gate3",
            "passed": false,
            "violations": [{"code": "replay_unavailable", "at": "", "message": error.to_string()}],
        }),
        Ok((ran, copied, workspace, before)) => {
            let replay = infra_core::strict_json::parse(
                String::from_utf8_lossy(&ran.stdout).trim().as_bytes(),
                8 * 1024 * 1024,
                32,
                200_000,
            )
            .unwrap_or(Value::Null);
            let mut violations = Vec::new();
            if replay["status"] == "replay" {
                for failure in replay["failures"].as_array().into_iter().flatten() {
                    let index = failure["step_index"].as_u64().unwrap_or_default() as usize;
                    let at = if candidate.capsule.is_verification(index) {
                        format!("verification[{}]", index - candidate.capsule.steps.len())
                    } else {
                        format!("steps[{index}]")
                    };
                    let text = failure["diagnostics"]["detail"].as_str().unwrap_or("");
                    let denied = sandbox_denied(text);
                    violations.push(json!({
                        "code": if denied { "sandbox_denied" } else { failure["diagnostics"]["reason"].as_str().unwrap_or("program_failed") },
                        "at": at,
                        "program": failure["program"],
                        "message": format!(
                            "Program `{}` failed in the replay{}: {text}",
                            failure["program"].as_str().unwrap_or(""),
                            if denied { " (the gate shows only the system's runtime, the package and the task copy)" } else { "" },
                        ),
                    }));
                }
            } else {
                let stopped = ran.stopped.unwrap_or("replay_failed");
                let text = replay["message"]
                    .as_str()
                    .map(String::from)
                    .unwrap_or_else(|| detail(&ran.stderr));
                violations.push(json!({
                    "code": replay["code"].as_str().unwrap_or(stopped),
                    "at": "",
                    "message": format!("The replay did not run to the end of the plan: {text}"),
                }));
            }
            let comparison = if violations.is_empty() {
                match before.and_then(|before| {
                    let after = replay_result::capture_with(
                        &workspace,
                        None,
                        &candidate.capsule.capabilities.allow_ignored,
                    )?;
                    Ok(replay_result::diff(&before, &after))
                }) {
                    Ok(actual) => compare_results(
                        expected,
                        &actual,
                        &replay,
                        &candidate.capsule,
                        candidate.examples.as_ref(),
                        &mut violations,
                    ),
                    Err(_) => {
                        violations.push(result_unavailable(
                            "The replay workspace could not be captured within the comparison limits.",
                        ));
                        json!({"status": "unavailable"})
                    }
                }
            } else {
                json!({"status": "not_checked"})
            };
            json!({
                "gate": "gate3",
                "passed": violations.is_empty(),
                "seconds": ran.seconds,
                "network": candidate.capsule.capabilities.network.as_str(),
                "completed": replay["completed"],
                "substituted": replay["substituted"],
                "result_comparison": comparison,
                "copy": {
                    "ignored_files": copied["ignored_files"],
                    "ignored_truncated": copied["ignored_truncated"],
                },
                "violations": violations,
            })
        }
    };
    let _ = std::fs::remove_dir_all(&top);
    let _ = std::fs::remove_dir_all(&round.state);
    report
}

fn result_unavailable(message: &str) -> Value {
    json!({"code": "replay_result_unavailable", "at": "", "message": message})
}

fn format_path_sample(paths: &[String]) -> String {
    const LIMIT: usize = 5;
    if paths.is_empty() {
        return String::new();
    }
    let shown: Vec<&str> = paths.iter().take(LIMIT).map(String::as_str).collect();
    if paths.len() > LIMIT {
        format!(" ({}, +{} more)", shown.join(", "), paths.len() - LIMIT)
    } else {
        format!(" ({})", shown.join(", "))
    }
}

fn non_empty_output(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
        Value::Bool(_) | Value::Number(_) => true,
    }
}

fn json_matches_observed(expected: &Value, actual: &Value) -> bool {
    match (expected, actual) {
        (Value::Object(exp), Value::Object(act)) => {
            !exp.is_empty()
                && exp.iter().all(|(key, exp_val)| {
                    act.get(key)
                        .is_some_and(|act_val| json_matches_observed(exp_val, act_val))
                })
        }
        (Value::Array(exp), Value::Array(act)) => {
            if exp.is_empty() {
                act.is_empty()
            } else {
                !act.is_empty()
                    && (replay_result::values_structurally_equal(expected, actual)
                        || exp
                            .iter()
                            .zip(act.iter())
                            .all(|(e, a)| json_matches_observed(e, a)))
            }
        }
        _ => replay_result::values_structurally_equal(expected, actual),
    }
}

fn step_outputs_match_observed_json(
    outputs: &serde_json::Map<String, Value>,
    observed_json: &Value,
) -> bool {
    if json_matches_observed(observed_json, &Value::Object(outputs.clone())) {
        return true;
    }
    outputs.values().any(|val| {
        json_matches_observed(observed_json, val)
            || val
                .as_str()
                .and_then(|s| {
                    infra_core::strict_json::parse(s.trim().as_bytes(), 16 * 1024, 32, 10_000).ok()
                })
                .is_some_and(|parsed| json_matches_observed(observed_json, &parsed))
    })
}

fn collect_leaf_strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Null => {}
        Value::Bool(b) => out.push(b.to_string()),
        Value::Number(n) => out.push(n.to_string()),
        Value::String(s) if !s.trim().is_empty() => out.push(s.clone()),
        Value::String(_) => {}
        Value::Array(items) => {
            for item in items {
                collect_leaf_strings(item, out);
            }
        }
        Value::Object(map) => {
            for val in map.values() {
                collect_leaf_strings(val, out);
            }
        }
    }
}

fn step_maps_match_observed_text(
    step_maps: &[&serde_json::Map<String, Value>],
    observed_text: &str,
) -> bool {
    let mut leaves = Vec::new();
    for map in step_maps {
        for val in map.values() {
            if non_empty_output(val) {
                collect_leaf_strings(val, &mut leaves);
            }
        }
    }
    if leaves.is_empty() {
        return false;
    }
    let expected_val = Value::String(observed_text.to_owned());
    if leaves.iter().any(|leaf| {
        replay_result::values_structurally_equal(&expected_val, &Value::String(leaf.clone()))
    }) {
        return true;
    }
    let combined = leaves.join("\n").to_ascii_lowercase();
    let mut salient: Vec<String> = Vec::new();
    for raw_tok in observed_text.split(|c: char| c.is_whitespace() || ",;()[]{}<>\"'`".contains(c))
    {
        let tok = raw_tok.trim_matches(|c: char| !c.is_alphanumeric() && !"/._-".contains(c));
        let is_number = tok.chars().all(|c| c.is_ascii_digit() || c == '.')
            && tok.chars().filter(char::is_ascii_digit).count() >= 1
            && (tok.len() >= 2 || tok.chars().all(|c| c.is_ascii_digit()));
        let is_path = tok.len() >= 3 && (tok.contains('/') || tok.contains('.'));
        if (is_number || is_path) && !salient.iter().any(|seen| seen == tok) {
            salient.push(tok.to_ascii_lowercase());
            if salient.len() >= 8 {
                break;
            }
        }
    }
    if !salient.is_empty() {
        let hits = salient
            .iter()
            .filter(|tok| combined.contains(tok.as_str()))
            .count();
        return hits * 2 >= salient.len();
    }
    let words: Vec<String> = observed_text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 4)
        .map(str::to_ascii_lowercase)
        .take(8)
        .collect();
    if words.is_empty() {
        let trimmed = observed_text.trim().to_ascii_lowercase();
        return !trimmed.is_empty() && combined.contains(&trimmed);
    }
    let hits = words
        .iter()
        .filter(|w| combined.contains(w.as_str()))
        .count();
    hits * 2 >= words.len()
}

fn verify_zero_file_outputs(
    expected: &ObservedResult,
    replay: &Value,
    capsule: Option<&SkillCapsule>,
    examples: Option<&Examples>,
    violations: &mut Vec<Value>,
) -> bool {
    let results = &replay["results"];
    if let Some(examples) = examples
        && !examples.steps.is_empty()
    {
        for (step_idx, expected_map) in &examples.steps {
            if expected_map.is_empty() {
                violations.push(json!({
                    "code": "replay_result_mismatch",
                    "at": format!("steps[{step_idx}]"),
                    "message": format!("Observed step `{step_idx}` example outputs are empty."),
                }));
                return false;
            }
            let program_key = capsule
                .and_then(|cap| cap.plan_step(*step_idx))
                .and_then(Step::program);
            let actual_step = program_key
                .and_then(|id| results.get(id))
                .unwrap_or(&results[step_idx.to_string()]);
            if !actual_step.is_object() {
                violations.push(json!({
                    "code": "replay_result_mismatch",
                    "at": format!("steps[{step_idx}]"),
                    "message": format!("Workspace made no file changes, and replay step `{step_idx}` returned no structured outputs."),
                }));
                return false;
            }
            for (key, expected_val) in expected_map {
                let actual_val = &actual_step[key];
                if actual_val.is_null()
                    || !replay_result::values_structurally_equal(expected_val, actual_val)
                {
                    violations.push(json!({
                        "code": "replay_result_mismatch",
                        "at": format!("steps[{step_idx}]"),
                        "message": format!(
                            "Workspace made no file changes, and replay output `{step_idx}.{key}` differs from the observed example."
                        ),
                    }));
                    return false;
                }
            }
        }
        return true;
    }

    let step_maps: Vec<&serde_json::Map<String, Value>> = results
        .as_object()
        .into_iter()
        .flat_map(|steps| steps.values())
        .filter_map(Value::as_object)
        .collect();
    if let Some(obs_json) = &expected.observed_json {
        if step_maps
            .iter()
            .any(|outputs| step_outputs_match_observed_json(outputs, obs_json))
        {
            return true;
        }
        violations.push(json!({
            "code": "replay_result_mismatch",
            "at": "",
            "message": "Workspace made no file changes, and replay step outputs do not match the observed JSON schema/values.",
        }));
        return false;
    }
    if let Some(obs_text) = &expected.observed_text {
        if step_maps_match_observed_text(&step_maps, obs_text) {
            return true;
        }
        violations.push(json!({
            "code": "replay_result_mismatch",
            "at": "",
            "message": "Workspace made no file changes, and replay step outputs do not match the observed output.",
        }));
        return false;
    }
    violations.push(json!({
        "code": "replay_result_mismatch",
        "at": "",
        "message": "Workspace made no file changes, and neither examples.steps nor an observed output was captured to verify replay equivalence.",
    }));
    false
}

fn compare_results(
    expected: &ObservedResult,
    actual: &ObservedResult,
    replay: &Value,
    capsule: &SkillCapsule,
    examples: Option<&Examples>,
    violations: &mut Vec<Value>,
) -> Value {
    compare_results_inner(
        expected,
        actual,
        replay,
        Some(capsule),
        examples,
        violations,
    )
}

fn compare_results_inner(
    expected: &ObservedResult,
    actual: &ObservedResult,
    replay: &Value,
    capsule: Option<&SkillCapsule>,
    examples: Option<&Examples>,
    violations: &mut Vec<Value>,
) -> Value {
    let label_for = |path_hash: &str| {
        expected
            .paths
            .get(path_hash)
            .or_else(|| actual.paths.get(path_hash))
            .cloned()
            .unwrap_or_else(|| format!("hash:{}", &path_hash[..path_hash.len().min(12)]))
    };
    let mut missing_paths = Vec::new();
    let mut different_paths = Vec::new();
    for (path_hash, digest) in &expected.changes {
        match actual.changes.get(path_hash) {
            None => missing_paths.push(label_for(path_hash)),
            Some(found) if found == digest => {}
            Some(Some(_))
                if digest.is_some()
                    && expected
                        .structural
                        .get(path_hash)
                        .is_some_and(|s| actual.structural.get(path_hash) == Some(s)) => {}
            Some(_) => different_paths.push(label_for(path_hash)),
        }
    }
    let unexpected_paths: Vec<String> = actual
        .changes
        .keys()
        .filter(|path| !expected.changes.contains_key(*path))
        .map(|path| label_for(path))
        .collect();
    let missing = missing_paths.len();
    let different = different_paths.len();
    let unexpected = unexpected_paths.len();
    let status = if missing + different + unexpected != 0 {
        violations.push(json!({
            "code": "replay_result_mismatch",
            "at": "",
            "message": format!(
                "Workspace changes differ from the observed turn: {missing} missing{}, {different} with different contents or deletion{}, {unexpected} unexpected{}. Reproduce the observed file changes.",
                format_path_sample(&missing_paths),
                format_path_sample(&different_paths),
                format_path_sample(&unexpected_paths),
            ),
        }));
        "mismatch"
    } else if expected.changes.is_empty() {
        if verify_zero_file_outputs(expected, replay, capsule, examples, violations) {
            "no_file_changes"
        } else {
            "mismatch"
        }
    } else {
        "matched"
    };
    json!({
        "status": status,
        "observed_changes": expected.changes.len(),
        "replay_changes": actual.changes.len(),
        "missing": missing,
        "different": different,
        "unexpected": unexpected,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minimal_root_failure_is_told_from_a_program_fault() {
        for text in [
            "sh: 1: jq: command not found",
            "python3: can't open file '/home/dev/x.py': [Errno 2] No such file or directory",
            "OSError: [Errno 30] Read-only file system: '/pkg/out'",
            "curl: (6) Could not resolve host: example.invalid",
        ] {
            assert!(sandbox_denied(text), "{text}");
        }
        for text in ["AssertionError: totals differ", "ValueError: bad row 3", ""] {
            assert!(!sandbox_denied(text), "{text}");
        }
    }

    #[test]
    fn a_gate_process_gets_no_credential_and_a_scratch_home() {
        let engine = Engine {
            path: None,
            codex: None,
        };
        let environment = environment(
            &engine,
            Path::new("/pkg"),
            Path::new("/work"),
            Path::new("/work/.harness-tmp"),
            Some(Path::new("/state/plan-state.json")),
        );
        assert!(environment.keys().all(|name| !env_filter::withheld(name)));
        assert_eq!(environment["HOME"], "/work/.harness-tmp");
        assert_eq!(environment["SKILLCACHE_PACKAGE"], "/pkg");
        assert_eq!(
            environment["SKILLCACHE_STATE_FILE"],
            "/state/plan-state.json"
        );
        assert!(
            !super::environment(
                &engine,
                Path::new("/p"),
                Path::new("/w"),
                Path::new("/s"),
                None
            )
            .contains_key("SKILLCACHE_STATE_FILE")
        );
    }

    #[test]
    fn compare_results_handles_structural_json_and_zero_file_output_checks() {
        let h_cfg = infra_skillcache::fingerprint::sha256(b"out/config.json");
        let cfg_observed = br#"{"z": 1, "a": {"k": "v"}}"#;
        let cfg_reformatted = b"{\n  \"a\": {\n    \"k\": \"v\"\n  },\n  \"z\": 1\n}\n";
        let cfg_dropped = br#"{"z": 1}"#;

        let expected = ObservedResult {
            changes: [(
                h_cfg.clone(),
                Some(infra_skillcache::fingerprint::sha256(cfg_observed)),
            )]
            .into_iter()
            .collect(),
            structural: [(
                h_cfg.clone(),
                replay_result::structural_digest("out/config.json", cfg_observed).unwrap(),
            )]
            .into_iter()
            .collect(),
            paths: [(h_cfg.clone(), "out/config.json".into())]
                .into_iter()
                .collect(),
            ..ObservedResult::default()
        };
        let actual_same_structure = ObservedResult {
            changes: [(
                h_cfg.clone(),
                Some(infra_skillcache::fingerprint::sha256(cfg_reformatted)),
            )]
            .into_iter()
            .collect(),
            structural: [(
                h_cfg.clone(),
                replay_result::structural_digest("out/config.json", cfg_reformatted).unwrap(),
            )]
            .into_iter()
            .collect(),
            paths: [(h_cfg.clone(), "out/config.json".into())]
                .into_iter()
                .collect(),
            ..ObservedResult::default()
        };
        let mut violations = Vec::new();
        let cmp = compare_results_inner(
            &expected,
            &actual_same_structure,
            &json!({"results": {}}),
            None,
            None,
            &mut violations,
        );
        assert_eq!(cmp["status"], "matched");
        assert!(violations.is_empty(), "{violations:?}");

        let actual_dropped = ObservedResult {
            changes: [(
                h_cfg.clone(),
                Some(infra_skillcache::fingerprint::sha256(cfg_dropped)),
            )]
            .into_iter()
            .collect(),
            structural: [(
                h_cfg.clone(),
                replay_result::structural_digest("out/config.json", cfg_dropped).unwrap(),
            )]
            .into_iter()
            .collect(),
            paths: [(h_cfg.clone(), "out/config.json".into())]
                .into_iter()
                .collect(),
            ..ObservedResult::default()
        };
        let mut violations = Vec::new();
        let cmp = compare_results_inner(
            &expected,
            &actual_dropped,
            &json!({"results": {}}),
            None,
            None,
            &mut violations,
        );
        assert_eq!(cmp["status"], "mismatch");
        assert!(
            violations[0]["message"]
                .as_str()
                .unwrap()
                .contains("out/config.json")
        );

        // Zero file changes requires matching/non-empty step output.
        let empty = ObservedResult::default();
        let mut ex = Examples::default();
        ex.steps.insert(
            0,
            serde_json::from_value(json!({"routes": "GET /a\nPOST /b\n"})).unwrap(),
        );
        let mut violations = Vec::new();
        let ok = compare_results_inner(
            &empty,
            &empty,
            &json!({"results": {"0": {"routes": "GET /a\r\nPOST /b\r\n"}}}),
            None,
            Some(&ex),
            &mut violations,
        );
        assert_eq!(ok["status"], "no_file_changes");
        assert!(violations.is_empty(), "{violations:?}");

        let mut violations = Vec::new();
        let dropped = compare_results_inner(
            &empty,
            &empty,
            &json!({"results": {"0": {"routes": "GET /a\n"}}}),
            None,
            Some(&ex),
            &mut violations,
        );
        assert_eq!(dropped["status"], "mismatch");
        assert_eq!(violations[0]["code"], "replay_result_mismatch");

        let mut violations = Vec::new();
        let noop = compare_results_inner(
            &empty,
            &empty,
            &json!({"results": {}}),
            None,
            None,
            &mut violations,
        );
        assert_eq!(noop["status"], "mismatch");
        assert_eq!(violations[0]["code"], "replay_result_mismatch");

        // Arbitrary non-empty results are rejected when neither examples.steps nor
        // observed output exists, or when they disagree with observed output.
        let mut violations = Vec::new();
        let bogus_without_expected = compare_results_inner(
            &empty,
            &empty,
            &json!({"results": {"0": {"bogus": "ok"}}}),
            None,
            None,
            &mut violations,
        );
        assert_eq!(bogus_without_expected["status"], "mismatch");
        assert_eq!(violations[0]["code"], "replay_result_mismatch");

        let obs_json_expected = ObservedResult {
            observed_json: Some(json!({"count": 3, "file": "out/summary.csv"})),
            ..ObservedResult::default()
        };
        let mut violations = Vec::new();
        assert_eq!(
            compare_results_inner(
                &obs_json_expected,
                &empty,
                &json!({"results": {"0": {"count": 99, "file": "out/summary.csv"}}}),
                None,
                None,
                &mut violations,
            )["status"],
            "mismatch"
        );
        let mut violations = Vec::new();
        assert_eq!(
            compare_results_inner(
                &obs_json_expected,
                &empty,
                &json!({"results": {"0": {"count": 3, "file": "out/summary.csv"}}}),
                None,
                None,
                &mut violations,
            )["status"],
            "no_file_changes"
        );
    }
}
