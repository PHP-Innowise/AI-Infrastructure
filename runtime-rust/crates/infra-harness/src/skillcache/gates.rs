//! Gates 2 and 3: a candidate's programs run for real, without a model.
//!
//! Both gates run the engine's own `skillcache` commands inside the gate
//! sandbox: a minimal root, the candidate package read-only, the network off
//! unless the capsule declares the need. Gate 2 runs every program's own
//! check in an empty folder. Gate 3 replays the whole plan on a fresh copy of
//! the task with the observed values answering for the agent, and collects
//! every failure in one pass. A host without a sandbox qualifies nothing:
//! there is no unconfined fallback.
use crate::{HarnessError, Result};
use infra_host::{
    capture::{self, Stream},
    sandbox::{self, Spec},
};
use infra_skillcache::{candidate::Candidate, capsule::Network, env_filter};
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

fn spec(
    workspace: &Path,
    readable: Vec<PathBuf>,
    writable: Vec<PathBuf>,
    codex: Option<&Path>,
    network: bool,
) -> Spec {
    Spec {
        workspace: workspace.to_path_buf(),
        target: workspace.to_path_buf(),
        protected: vec![],
        readable,
        accounts: writable,
        runtime: vec![],
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
                &spec(&workspace, readable, vec![], engine.codex.as_deref(), false),
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
pub fn gate3(
    engine: &Engine,
    round: &Round,
    candidate: &Candidate,
    copy: &dyn Fn(&Path) -> Result<Value>,
    cancel: &Arc<AtomicBool>,
) -> Value {
    let package = round.package();
    let top = round.workspace("gate3");
    let attempt = (|| -> Result<(Ran, Value)> {
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
        let ran = run_sandboxed(
            &spec(
                &top,
                readable,
                vec![round.state.clone()],
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
        Ok((ran, copied))
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
        Ok((ran, copied)) => {
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
            json!({
                "gate": "gate3",
                "passed": violations.is_empty(),
                "seconds": ran.seconds,
                "network": candidate.capsule.capabilities.network.as_str(),
                "completed": replay["completed"],
                "substituted": replay["substituted"],
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
}
