//! The sealed evaluation of SkillCache: what a capsule costs to compile, what
//! it saves on each reuse, and whether the work is still accepted.
//!
//! A protocol names the question, the correctness design, the arms, the
//! probes, the model pairs, the tasks, the price table, the driver and the
//! verifiers. It is sealed before anything runs, and a run and a report both
//! refuse a protocol whose content no longer matches its seal.
//!
//! A run asks the driver for one preparation or one attempt at a time. With
//! verifiers, an attempt is judged by an isolated verifier that sees the
//! attempt's workspace and nothing else; the driver's own opinion is not
//! asked. A report prices attempts with the sealed table only, counts a
//! saving only where both attempts were accepted and fully priced, and never
//! turns an unknown into a zero or into a success.
use crate::{
    Result, artifact, isolation, outcome,
    process::{self, CommandSpec},
    rng,
};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Duration,
};

pub const PROTOCOL: &str = "skillcache-eval-protocol-v1";
pub const RESULT: &str = "skillcache-eval-result-v1";
pub const REPORT: &str = "skillcache-eval-report-v1";
/// `cold` is the unchanged skill. `raw_trace` and `written_procedure` give the
/// agent the observed trace or a prose procedure instead of a capsule.
/// `skillcache_agent` gives it the capsule without the executor,
/// `skillcache_executor` is the full warm turn, and `skillcache_no_replay` is
/// a capsule that passed Gates 1 and 2 only.
pub const ARMS: [&str; 6] = [
    "cold",
    "raw_trace",
    "written_procedure",
    "skillcache_agent",
    "skillcache_executor",
    "skillcache_no_replay",
];
/// How far a probe request is from the one the capsule was compiled from.
/// `exact` is the observed request itself.
pub const PROBES: [&str; 4] = ["exact", "instance", "rename", "reword"];
/// Arms that involve no compiler: they run once for an executor model,
/// whatever the compile model of the cell.
const COMPILE_FREE: [&str; 2] = ["cold", "raw_trace"];
/// The token components a provider may report, and their price columns.
const COMPONENTS: [(&str, &str); 4] = [
    ("input_tokens", "input"),
    ("output_tokens", "output"),
    ("cache_read_input_tokens", "cache_read"),
    ("cache_creation_input_tokens", "cache_write"),
];
/// The placeholder a verifier's argument uses for the sealed verifier folder.
const VERIFIERS: &str = "{verifiers}";
const SLOTS: usize = 20_000;
const LIMIT: usize = 16 * 1024 * 1024;

/// task, probe, arm, compile model, executor model.
type Slot = (String, String, String, Option<String>, String);
/// task, arm, compile model, executor model.
type Preparation = (String, String, Option<String>, String);

fn invalid(message: impl AsRef<str>) -> crate::RuntimeError {
    artifact::invalid(message.as_ref())
}
fn text<'a>(value: &'a Value, name: &str, limit: usize) -> Result<&'a str> {
    value[name]
        .as_str()
        .filter(|text| !text.trim().is_empty() && text.len() <= limit)
        .ok_or_else(|| invalid(format!("`{name}` is a text of at most {limit} bytes")))
}
fn only(value: &Value, what: &str, known: &[&str]) -> Result<()> {
    let members = value
        .as_object()
        .ok_or_else(|| invalid(format!("{what} is a JSON object")))?;
    match members.keys().find(|key| !known.contains(&key.as_str())) {
        Some(unknown) => Err(invalid(format!("{what} has an unknown field `{unknown}`"))),
        None => Ok(()),
    }
}
fn names(value: &Value, name: &str, known: &[&str]) -> Result<Vec<String>> {
    let listed = value[name]
        .as_array()
        .ok_or_else(|| invalid(format!("`{name}` is a list")))?;
    let mut chosen = Vec::new();
    for item in listed {
        let item = item
            .as_str()
            .filter(|item| known.contains(item))
            .ok_or_else(|| invalid(format!("`{name}` holds only: {}", known.join(", "))))?;
        if chosen.iter().any(|known| known == item) {
            return Err(invalid(format!("`{name}` lists `{item}` twice")));
        }
        chosen.push(item.to_string());
    }
    if chosen.is_empty() {
        return Err(invalid(format!("`{name}` is empty")));
    }
    Ok(chosen)
}
fn seconds(value: &Value, limit: f64) -> Option<Duration> {
    value
        .as_f64()
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0 && *seconds <= limit)
        .map(Duration::from_secs_f64)
}
fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item.as_str().map(String::from))
        .collect()
}
fn items(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().into_iter().flatten()
}

/// Checks a protocol. Everything a run and a report later need is required
/// here, so that nothing is chosen after results exist.
pub fn validate(protocol: &Value, sealed: bool) -> Result<()> {
    let mut known = vec![
        "schema",
        "question",
        "design",
        "arms",
        "probes",
        "matrix",
        "tasks",
        "prices",
        "reuses",
        "bootstrap",
        "driver",
        "verifiers",
    ];
    if sealed {
        known.extend(["sealed_at", "seal_sha256"]);
    }
    only(protocol, "the protocol", &known)?;
    if protocol["schema"] != PROTOCOL {
        return Err(invalid(format!("`schema` is `{PROTOCOL}`")));
    }
    text(protocol, "question", 1000)?;
    // The correctness design of the outcome evaluation: correctness first,
    // a fixed horizon, and an unknown that makes the answer insufficient.
    outcome::validate_design(&protocol["design"])?;
    let arms = names(protocol, "arms", &ARMS)?;
    if arms.len() < 2 || !arms.iter().any(|arm| arm == "cold") {
        return Err(invalid(
            "`arms` holds `cold` and at least one other arm: every arm is measured against it",
        ));
    }
    let probes = names(protocol, "probes", &PROBES)?;
    let matrix = protocol["matrix"]
        .as_array()
        .filter(|cells| !cells.is_empty() && cells.len() <= 16)
        .ok_or_else(|| invalid("`matrix` lists 1 to 16 model pairs"))?;
    let mut models = BTreeSet::new();
    let mut pairs = BTreeSet::new();
    for cell in matrix {
        only(
            cell,
            "a `matrix` cell",
            &["compile_model", "executor_model"],
        )?;
        let pair = (
            text(cell, "compile_model", 200)?.to_string(),
            text(cell, "executor_model", 200)?.to_string(),
        );
        if !pairs.insert(pair.clone()) {
            return Err(invalid("`matrix` lists one model pair twice"));
        }
        models.insert(pair.0);
        models.insert(pair.1);
    }
    let verified = match &protocol["verifiers"] {
        Value::Null => false,
        verifiers => {
            let mut known = vec!["root", "timeout_seconds"];
            if sealed {
                known.push("sha256");
            }
            only(verifiers, "`verifiers`", &known)?;
            if !Path::new(text(verifiers, "root", 4096)?).is_absolute() {
                return Err(invalid("`verifiers.root` is an absolute folder"));
            }
            if seconds(&verifiers["timeout_seconds"], 3600.0).is_none() {
                return Err(invalid("`verifiers.timeout_seconds` is from 0 to 3600"));
            }
            if sealed
                && !verifiers["sha256"]
                    .as_str()
                    .is_some_and(|digest| artifact::hex(digest, 64))
            {
                return Err(invalid("a sealed protocol records the verifiers' digest"));
            }
            true
        }
    };
    let tasks = protocol["tasks"]
        .as_array()
        .filter(|tasks| !tasks.is_empty() && tasks.len() <= 1000)
        .ok_or_else(|| invalid("`tasks` lists 1 to 1000 tasks"))?;
    let mut ids = BTreeSet::new();
    for task in tasks {
        only(
            task,
            "a task",
            &["id", "skill", "observe", "probes", "verifier"],
        )?;
        let id = text(task, "id", 80)?;
        if !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
            || !ids.insert(id)
        {
            return Err(invalid(format!(
                "task id `{id}` is not a distinct name of letters, digits, `.`, `_` and `-`"
            )));
        }
        text(task, "skill", 200)?;
        text(task, "observe", 8000)?;
        // `exact` is the observed request; every other probe has its own.
        let wanted = probes
            .iter()
            .filter(|probe| *probe != "exact")
            .map(String::as_str)
            .collect::<Vec<_>>();
        if wanted.is_empty() {
            if !task["probes"].is_null() {
                return Err(invalid(format!(
                    "task `{id}` gives requests for no listed probe"
                )));
            }
        } else {
            only(
                &task["probes"],
                &format!("`probes` of task `{id}`"),
                &wanted,
            )?;
            for probe in wanted {
                text(&task["probes"], probe, 8000)?;
            }
        }
        // One experiment is judged one way: by verifiers or by the driver.
        match &task["verifier"] {
            Value::Null if !verified => {}
            Value::Array(argv)
                if verified
                    && !argv.is_empty()
                    && argv
                        .iter()
                        .all(|word| word.as_str().is_some_and(|word| !word.is_empty())) => {}
            _ => {
                return Err(invalid(format!(
                    "task `{id}`: with `verifiers` every task names its verifier command, without them none does"
                )));
            }
        }
    }
    // The prices are part of what is sealed: a report cannot pick a cheaper
    // table afterwards.
    let prices = protocol["prices"]
        .as_object()
        .ok_or_else(|| invalid("`prices` maps each model to its USD per million tokens"))?;
    if let Some(unused) = prices.keys().find(|model| !models.contains(*model)) {
        return Err(invalid(format!(
            "`prices` has a row for `{unused}`, which the matrix does not name"
        )));
    }
    for model in &models {
        let row = prices
            .get(model)
            .ok_or_else(|| invalid(format!("`prices` has no row for model `{model}`")))?;
        only(
            row,
            &format!("`prices.{model}`"),
            &COMPONENTS.map(|(_, column)| column),
        )?;
        for (_, column) in COMPONENTS {
            if !row[column]
                .as_f64()
                .is_some_and(|price| price.is_finite() && price >= 0.0)
            {
                return Err(invalid(format!(
                    "`prices.{model}.{column}` is a price in USD per million tokens"
                )));
            }
        }
    }
    let reuses = protocol["reuses"]
        .as_array()
        .filter(|reuses| !reuses.is_empty() && reuses.len() <= 12)
        .ok_or_else(|| invalid("`reuses` lists the reuse counts to report"))?;
    let counts = reuses
        .iter()
        .filter_map(Value::as_u64)
        .filter(|count| (1..=100_000).contains(count))
        .collect::<BTreeSet<_>>();
    if counts.len() != reuses.len() {
        return Err(invalid(
            "`reuses` holds distinct whole numbers from 1 to 100000",
        ));
    }
    only(&protocol["bootstrap"], "`bootstrap`", &["rounds", "seed"])?;
    if !protocol["bootstrap"]["rounds"]
        .as_u64()
        .is_some_and(|rounds| (100..=100_000).contains(&rounds))
        || !protocol["bootstrap"]["seed"].is_u64()
    {
        return Err(invalid("`bootstrap` is {rounds: 100..100000, seed}"));
    }
    only(
        &protocol["driver"],
        "`driver`",
        &["argv", "timeout_seconds"],
    )?;
    if !protocol["driver"]["argv"].as_array().is_some_and(|argv| {
        !argv.is_empty()
            && argv
                .iter()
                .all(|word| word.as_str().is_some_and(|word| !word.is_empty()))
    }) || seconds(&protocol["driver"]["timeout_seconds"], 86_400.0).is_none()
    {
        return Err(invalid(
            "`driver` is {argv, timeout_seconds}: the command that runs one preparation or one attempt",
        ));
    }
    if sealed
        && (!protocol["seal_sha256"]
            .as_str()
            .is_some_and(|digest| artifact::hex(digest, 64))
            || !protocol["sealed_at"].is_string())
    {
        return Err(invalid("not a sealed SkillCache protocol"));
    }
    let (preparations, attempts) = planned(protocol);
    if preparations.len() + attempts.len() > SLOTS {
        return Err(invalid(format!(
            "the protocol plans {} driver calls; the limit is {SLOTS}",
            preparations.len() + attempts.len()
        )));
    }
    Ok(())
}

/// The compile and executor models an arm runs with in one matrix cell.
fn models(arm: &str, cell: &Value) -> (Option<String>, String) {
    (
        (!COMPILE_FREE.contains(&arm)).then(|| {
            cell["compile_model"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        }),
        cell["executor_model"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
    )
}

/// Every preparation and attempt a protocol plans, each once.
fn planned(protocol: &Value) -> (BTreeSet<Preparation>, BTreeSet<Slot>) {
    let arms = strings(&protocol["arms"]);
    let probes = strings(&protocol["probes"]);
    let mut preparations = BTreeSet::new();
    let mut attempts = BTreeSet::new();
    for cell in items(&protocol["matrix"]) {
        for task in items(&protocol["tasks"]) {
            let id = task["id"].as_str().unwrap_or_default();
            for arm in &arms {
                let (compile_model, executor_model) = models(arm, cell);
                if arm != "cold" {
                    preparations.insert((
                        id.to_string(),
                        arm.clone(),
                        compile_model.clone(),
                        executor_model.clone(),
                    ));
                }
                for probe in &probes {
                    attempts.insert((
                        id.to_string(),
                        probe.clone(),
                        arm.clone(),
                        compile_model.clone(),
                        executor_model.clone(),
                    ));
                }
            }
        }
    }
    (preparations, attempts)
}

fn seal_digest(protocol: &Value) -> Result<String> {
    let mut body = protocol.clone();
    if let Some(members) = body.as_object_mut() {
        members.remove("seal_sha256");
    }
    artifact::digest(&body)
}

/// Seals a protocol: checks it, records the digest of the verifiers' folder,
/// and writes it with its seal to `out`, which must not exist.
pub fn seal(protocol: &Path, out: &Path) -> Result<Value> {
    let mut document = artifact::read_json(protocol, LIMIT)?;
    validate(&document, false)?;
    if out.exists() {
        return Err(invalid(
            "the sealed protocol already exists; a seal is written once",
        ));
    }
    if let Some(root) = document["verifiers"]["root"].as_str().map(PathBuf::from) {
        if !root.is_dir() || artifact::overlaps(&root, out)? {
            return Err(invalid(
                "`verifiers.root` is an existing folder that does not hold the sealed protocol",
            ));
        }
        document["verifiers"]["sha256"] = artifact::candidate_digest(&root)?.into();
    }
    document["sealed_at"] = chrono::Utc::now()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        .into();
    document["seal_sha256"] = seal_digest(&document)?.into();
    artifact::write_json(out, &document)?;
    let (preparations, attempts) = planned(&document);
    Ok(json!({
        "protocol": out,
        "seal_sha256": document["seal_sha256"],
        "planned": {"preparations": preparations.len(), "attempts": attempts.len()},
    }))
}

/// A sealed protocol whose content still matches its seal.
fn load(path: &Path) -> Result<Value> {
    let sealed = artifact::read_json(path, LIMIT)?;
    validate(&sealed, true)?;
    if sealed["seal_sha256"].as_str() != Some(seal_digest(&sealed)?.as_str()) {
        return Err(invalid("the protocol was changed after it was sealed"));
    }
    Ok(sealed)
}

fn count(value: &Value) -> Option<u64> {
    process::token(value)
}
/// The four token components of a usage; a component that was not reported
/// stays null.
fn usage(source: &Value) -> Value {
    COMPONENTS
        .iter()
        .map(|(component, _)| ((*component).to_string(), json!(count(&source[*component]))))
        .collect::<Map<_, _>>()
        .into()
}
fn known_seconds(value: &Value) -> Value {
    json!(
        value
            .as_f64()
            .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
    )
}

struct Driver {
    argv: Vec<String>,
    timeout: Duration,
    cwd: PathBuf,
    environment: BTreeMap<String, String>,
}
impl Driver {
    /// One call: a JSON object in, a JSON object out. A driver that fails,
    /// runs out of time or answers with something else has not answered; only
    /// a loss of process custody stops the run.
    fn ask(&self, request: &Value) -> Result<std::result::Result<Value, &'static str>> {
        let spec = CommandSpec {
            argv: self.argv.clone(),
            cwd: self.cwd.clone(),
            env: self.environment.clone(),
            input: request.to_string().into_bytes(),
        };
        let (code, raw) = process::capture(&spec, self.timeout, 1024 * 1024)?;
        if code != 0 {
            return Ok(Err("driver-failed"));
        }
        Ok(artifact::strict(&raw)
            .ok()
            .filter(Value::is_object)
            .ok_or("driver-answer-invalid"))
    }
}

struct Verifiers {
    root: PathBuf,
    timeout: Duration,
    /// The folder of the sealed protocol: no workspace may hold it, and no
    /// verifier sees it.
    private: PathBuf,
}
impl Verifiers {
    fn view(&self, workspace: &Path, argv: &[String]) -> Result<isolation::View> {
        if artifact::overlaps(workspace, &self.root)? {
            return Err(invalid("the workspace overlaps the verifiers"));
        }
        let mut view = isolation::prepare(workspace, &self.private, argv, false, None)?;
        let root = self.root.to_string_lossy().into_owned();
        view.prefix.extend([
            "--ro-bind".into(),
            root.clone(),
            root,
            "--unshare-net".into(),
        ]);
        Ok(view)
    }
    /// Proves on an empty folder that a verifier cannot see the sealed
    /// protocol, before anything is spent on an attempt.
    fn preflight(&self, sealed: &Path) -> Result<()> {
        let scratch = artifact::Scratch::new(&std::env::temp_dir(), "skillcache-eval-probe")?;
        let view = self.view(&scratch.0, &["/usr/bin/test".into()])?;
        if !isolation::probe(&view, sealed) {
            return Err(invalid(
                "the verifier isolation is not in force here; nothing was run",
            ));
        }
        Ok(())
    }
    /// Runs one task's verifier over a workspace. It answers with the receipt
    /// of the outcome evaluation; anything else is not a judgment.
    fn judge(&self, workspace: &Path, command: &[String]) -> Result<Option<bool>> {
        let argv = command
            .iter()
            .map(|word| word.replace(VERIFIERS, &self.root.to_string_lossy()))
            .collect::<Vec<_>>();
        let Ok(view) = self.view(workspace, &argv) else {
            return Ok(None);
        };
        let (code, raw) = process::capture(&view.spec(), self.timeout, 65536)?;
        if code != 0 {
            return Ok(None);
        }
        Ok(artifact::strict(&raw)
            .ok()
            .filter(|receipt| {
                artifact::exact(receipt, &["schema", "completed", "correct"])
                    && receipt["schema"] == "task-outcome-verifier-v1"
                    && receipt["completed"] == true
            })
            .and_then(|receipt| receipt["correct"].as_bool()))
    }
}

fn shuffled(arms: &[String], order: &mut rng::Random) -> Vec<String> {
    let mut arms = arms.to_vec();
    for index in (1..arms.len()).rev() {
        arms.swap(index, order.below(index + 1));
    }
    arms
}
fn stop_if_interrupted() -> Result<()> {
    if crate::cancel::interrupted() {
        return Err(invalid(
            "the run was interrupted; no result was written, and a sealed plan is not resumed",
        ));
    }
    Ok(())
}

/// Runs every planned preparation and attempt through the driver and writes
/// the result, which must not exist: a sealed plan is run once.
pub fn run(protocol: &Path, out: &Path) -> Result<Value> {
    let sealed = load(protocol)?;
    if out.exists() {
        return Err(invalid(
            "the result already exists; a sealed plan is run once",
        ));
    }
    let private = infra_core::fs::absolute(protocol)?
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| invalid("the sealed protocol has no folder"))?;
    let verifiers = match &sealed["verifiers"] {
        Value::Null => None,
        settings => {
            let verifiers = Verifiers {
                root: PathBuf::from(settings["root"].as_str().unwrap_or_default()),
                timeout: seconds(&settings["timeout_seconds"], 3600.0).unwrap_or_default(),
                private,
            };
            if !verifiers.root.is_dir()
                || settings["sha256"].as_str()
                    != Some(artifact::candidate_digest(&verifiers.root)?.as_str())
            {
                return Err(invalid(
                    "the verifiers changed after the protocol was sealed",
                ));
            }
            verifiers.preflight(protocol)?;
            Some(verifiers)
        }
    };
    let driver = Driver {
        argv: strings(&sealed["driver"]["argv"]),
        timeout: seconds(&sealed["driver"]["timeout_seconds"], 86_400.0).unwrap_or_default(),
        cwd: std::env::current_dir()?,
        environment: std::env::vars_os()
            .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
            .collect(),
    };
    let arms = strings(&sealed["arms"]);
    let probes = strings(&sealed["probes"]);
    let mut order = rng::Random::new(sealed["bootstrap"]["seed"].as_u64().unwrap_or_default());
    let started = chrono::Utc::now();
    let mut preparations = Vec::new();
    let mut attempts = Vec::new();
    let mut prepared = BTreeSet::new();
    let mut attempted = BTreeSet::new();
    for cell in items(&sealed["matrix"]) {
        for task in items(&sealed["tasks"]) {
            let id = task["id"].as_str().unwrap_or_default();
            // The arms run in a seeded order, so that no arm is always first
            // on a warm provider cache.
            for arm in shuffled(&arms, &mut order) {
                let (compile_model, executor_model) = models(&arm, cell);
                if arm == "cold"
                    || !prepared.insert((
                        id.to_string(),
                        arm.clone(),
                        compile_model.clone(),
                        executor_model.clone(),
                    ))
                {
                    continue;
                }
                stop_if_interrupted()?;
                let answer = driver.ask(&json!({
                    "phase": "prepare",
                    "seal_sha256": sealed["seal_sha256"],
                    "task": id,
                    "skill": task["skill"],
                    "observe": task["observe"],
                    "arm": arm,
                    "compile_model": compile_model,
                    "executor_model": executor_model,
                }))?;
                stop_if_interrupted()?;
                let mut record = json!({
                    "task": id,
                    "arm": arm,
                    "compile_model": compile_model,
                    "executor_model": executor_model,
                    "status": "finished",
                    "prepared": null,
                    "usage": usage(&Value::Null),
                    "seconds": null,
                });
                match answer {
                    Ok(answer) => {
                        record["prepared"] = json!(answer["prepared"].as_bool());
                        record["usage"] = usage(&answer["usage"]);
                        record["seconds"] = known_seconds(&answer["seconds"]);
                    }
                    Err(reason) => record["status"] = reason.into(),
                }
                preparations.push(record);
            }
            for probe in &probes {
                let request = if probe == "exact" {
                    &task["observe"]
                } else {
                    &task["probes"][probe]
                };
                for (position, arm) in shuffled(&arms, &mut order).into_iter().enumerate() {
                    let (compile_model, executor_model) = models(&arm, cell);
                    if !attempted.insert((
                        id.to_string(),
                        probe.clone(),
                        arm.clone(),
                        compile_model.clone(),
                        executor_model.clone(),
                    )) {
                        continue;
                    }
                    stop_if_interrupted()?;
                    let answer = driver.ask(&json!({
                        "phase": "attempt",
                        "seal_sha256": sealed["seal_sha256"],
                        "task": id,
                        "skill": task["skill"],
                        "observe": task["observe"],
                        "probe": probe,
                        "request": request,
                        "arm": arm,
                        "compile_model": compile_model,
                        "executor_model": executor_model,
                        "position": position,
                    }))?;
                    stop_if_interrupted()?;
                    let mut record = json!({
                        "task": id,
                        "probe": probe,
                        "arm": arm,
                        "compile_model": compile_model,
                        "executor_model": executor_model,
                        "position": position,
                        "status": "finished",
                        "accepted": null,
                        "judged_by": null,
                        "usage": usage(&Value::Null),
                        "model_calls": null,
                        "seconds": null,
                        "program_steps": null,
                        "failed_steps": null,
                    });
                    match answer {
                        Ok(answer) => {
                            record["usage"] = usage(&answer["usage"]);
                            record["model_calls"] = json!(count(&answer["model_calls"]));
                            record["seconds"] = known_seconds(&answer["seconds"]);
                            record["program_steps"] = json!(count(&answer["program_steps"]));
                            record["failed_steps"] = json!(count(&answer["failed_steps"]));
                            let accepted = match &verifiers {
                                // With verifiers the driver is not asked what
                                // it thinks of its own work.
                                Some(verifiers) => {
                                    record["judged_by"] = "verifier".into();
                                    match answer["workspace"].as_str().map(Path::new) {
                                        Some(workspace)
                                            if workspace.is_absolute() && workspace.is_dir() =>
                                        {
                                            let judged = verifiers
                                                .judge(workspace, &strings(&task["verifier"]))?;
                                            stop_if_interrupted()?;
                                            judged
                                        }
                                        _ => None,
                                    }
                                }
                                None => {
                                    record["judged_by"] = "driver".into();
                                    answer["accepted"].as_bool()
                                }
                            };
                            record["accepted"] = json!(accepted);
                        }
                        Err(reason) => record["status"] = reason.into(),
                    }
                    attempts.push(record);
                }
            }
        }
    }
    let result = json!({
        "schema": RESULT,
        "seal_sha256": sealed["seal_sha256"],
        "started_at": started.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "finished_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "isolation": verifiers.as_ref().map(|_| isolation::BACKEND),
        "preparations": preparations,
        "attempts": attempts,
    });
    artifact::write_json(out, &result)?;
    Ok(json!({
        "result": out,
        "seal_sha256": sealed["seal_sha256"],
        "preparations": result["preparations"].as_array().map_or(0, Vec::len),
        "attempts": result["attempts"].as_array().map_or(0, Vec::len),
        "unanswered": items(&result["preparations"])
            .chain(items(&result["attempts"]))
            .filter(|record| record["status"] != "finished")
            .count(),
    }))
}

/// The list price of a usage under one model's sealed row, in USD: the sum
/// over the components that were reported, and whether that was all of them.
fn price(usage: &Value, row: &Value) -> (Option<f64>, bool) {
    let mut total = 0.0;
    let mut known = 0;
    for (component, column) in COMPONENTS {
        if let (Some(tokens), Some(per_million)) = (count(&usage[component]), row[column].as_f64())
        {
            total += tokens as f64 * per_million / 1_000_000.0;
            known += 1;
        }
    }
    ((known > 0).then_some(total), known == COMPONENTS.len())
}
fn full_price(usage: &Value, row: &Value) -> Option<f64> {
    let (total, complete) = price(usage, row);
    total.filter(|_| complete)
}
fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}
fn rounded(value: Option<f64>) -> Value {
    value.map_or(Value::Null, |value| json!((value * 1e9).round() / 1e9))
}
fn interval(values: &[f64], rounds: usize, seed: u64) -> Option<[f64; 2]> {
    (values.len() >= 2)
        .then(|| rng::bootstrap(values, rounds, seed))
        .flatten()
}
fn shown(interval: Option<[f64; 2]>) -> Value {
    interval.map_or(Value::Null, |[low, high]| {
        json!([rounded(Some(low)), rounded(Some(high))])
    })
}

/// What one arm's attempts in one cell say by themselves.
fn arm_summary(attempts: &[&Value], row: &Value) -> Value {
    let judged = attempts
        .iter()
        .filter(|attempt| attempt["accepted"].is_boolean())
        .count();
    let accepted = attempts
        .iter()
        .filter(|attempt| attempt["accepted"] == true)
        .count();
    let mut tokens = Map::new();
    let mut coverage = Map::new();
    for (component, _) in COMPONENTS {
        let known = attempts
            .iter()
            .filter_map(|attempt| count(&attempt["usage"][component]).map(|tokens| tokens as f64))
            .collect::<Vec<_>>();
        coverage.insert(component.into(), known.len().into());
        tokens.insert(component.into(), rounded(mean(&known)));
    }
    let known = |field: &str| {
        attempts
            .iter()
            .filter_map(|attempt| attempt[field].as_f64())
            .collect::<Vec<_>>()
    };
    let priced = attempts
        .iter()
        .filter_map(|attempt| full_price(&attempt["usage"], row))
        .collect::<Vec<_>>();
    let lower = attempts
        .iter()
        .filter_map(|attempt| price(&attempt["usage"], row).0)
        .collect::<Vec<_>>();
    // A deoptimization is a program step handed back to the agent.
    let stepped = attempts
        .iter()
        .filter_map(|attempt| {
            Some((
                count(&attempt["program_steps"])?,
                count(&attempt["failed_steps"])?,
            ))
        })
        .collect::<Vec<_>>();
    let steps = stepped.iter().map(|(steps, _)| *steps).sum::<u64>();
    let failed = stepped.iter().map(|(_, failed)| *failed).sum::<u64>();
    json!({
        "planned": attempts.len(),
        "answered": attempts.iter().filter(|attempt| attempt["status"] == "finished").count(),
        "judged": judged,
        "accepted": accepted,
        // A rate over a partly judged arm would count an unknown as a failure.
        "acceptance_rate": rounded(
            (judged == attempts.len() && judged > 0).then(|| accepted as f64 / judged as f64),
        ),
        "tokens": tokens,
        "coverage": {
            "usage": coverage,
            "fully_priced": priced.len(),
            "model_calls": known("model_calls").len(),
            "seconds": known("seconds").len(),
            "program_steps": stepped.len(),
        },
        "model_calls": rounded(mean(&known("model_calls"))),
        "seconds": rounded(mean(&known("seconds"))),
        "list_price_usd": rounded(mean(&priced)),
        "list_price_usd_reported_components": rounded(mean(&lower)),
        "deopt_rate": rounded((steps > 0).then(|| failed as f64 / steps as f64)),
    })
}

/// Summarizes a result under the protocol it was run for.
pub fn report(protocol: &Path, result: &Path) -> Result<Value> {
    let sealed = load(protocol)?;
    let result = artifact::read_json(result, artifact::MAX_TREE)?;
    if result["schema"] != RESULT || result["seal_sha256"] != sealed["seal_sha256"] {
        return Err(invalid("this result was not run for this sealed protocol"));
    }
    // A result holds exactly the plan: nothing lost, added or run twice.
    let (planned_preparations, planned_attempts) = planned(&sealed);
    let model = |value: &Value| value.as_str().map(String::from);
    let word = |value: &Value| value.as_str().unwrap_or_default().to_string();
    let mut attempts = BTreeMap::new();
    for attempt in items(&result["attempts"]) {
        attempts.insert(
            (
                word(&attempt["task"]),
                word(&attempt["probe"]),
                word(&attempt["arm"]),
                model(&attempt["compile_model"]),
                word(&attempt["executor_model"]),
            ),
            attempt,
        );
    }
    let mut preparations = BTreeMap::new();
    for preparation in items(&result["preparations"]) {
        preparations.insert(
            (
                word(&preparation["task"]),
                word(&preparation["arm"]),
                model(&preparation["compile_model"]),
                word(&preparation["executor_model"]),
            ),
            preparation,
        );
    }
    if attempts.len() != items(&result["attempts"]).count()
        || preparations.len() != items(&result["preparations"]).count()
        || !attempts.keys().eq(planned_attempts.iter())
        || !preparations.keys().eq(planned_preparations.iter())
    {
        return Err(invalid(
            "the result does not hold exactly the planned preparations and attempts",
        ));
    }
    let design = &sealed["design"];
    let margin = design["quality_margin"].as_f64().unwrap_or_default();
    let minimum = design["minimum_tasks"].as_u64().unwrap_or(u64::MAX) as usize;
    let rounds = sealed["bootstrap"]["rounds"].as_u64().unwrap_or(1000) as usize;
    let seed = sealed["bootstrap"]["seed"].as_u64().unwrap_or_default();
    let arms = strings(&sealed["arms"]);
    let probes = strings(&sealed["probes"]);
    let tasks = items(&sealed["tasks"])
        .map(|task| word(&task["id"]))
        .collect::<Vec<_>>();
    let mut cells = Vec::new();
    for cell in items(&sealed["matrix"]) {
        let executor_row = &sealed["prices"][cell["executor_model"].as_str().unwrap_or_default()];
        let of = |arm: &str, task: &str, probe: &str| {
            let (compile_model, executor_model) = models(arm, cell);
            attempts[&(
                task.to_string(),
                probe.to_string(),
                arm.to_string(),
                compile_model,
                executor_model,
            )]
        };
        let mut summaries = Map::new();
        for arm in &arms {
            let mine = tasks
                .iter()
                .flat_map(|task| probes.iter().map(|probe| of(arm, task, probe)))
                .collect::<Vec<_>>();
            let mut summary = arm_summary(&mine, executor_row);
            if arm == "cold" {
                summaries.insert(arm.clone(), summary);
                continue;
            }
            // What the arm cost before its first reuse, at the price of the
            // model that did the work.
            let (compile_model, executor_model) = models(arm, cell);
            let compile_row =
                &sealed["prices"][compile_model.as_deref().unwrap_or(&executor_model)];
            let prepared = tasks
                .iter()
                .map(|task| {
                    preparations[&(
                        task.clone(),
                        arm.clone(),
                        compile_model.clone(),
                        executor_model.clone(),
                    )]
                })
                .collect::<Vec<_>>();
            let compile_prices = prepared
                .iter()
                .filter_map(|preparation| full_price(&preparation["usage"], compile_row))
                .collect::<Vec<_>>();
            let compile = mean(&compile_prices);
            summary["preparation"] = json!({
                "planned": prepared.len(),
                "prepared": prepared.iter().filter(|p| p["prepared"] == true).count(),
                "not_prepared": prepared.iter().filter(|p| p["prepared"] == false).count(),
                "unknown": prepared.iter().filter(|p| !p["prepared"].is_boolean()).count(),
                "fully_priced": compile_prices.len(),
                "compile_usd": rounded(compile),
            });
            // Correctness first. The paired unit is the task: its probes are
            // not independent of each other.
            let mut valid = tasks.len() >= minimum;
            let mut differences = Vec::new();
            let mut pairs = 0;
            let mut pairs_accepted = 0;
            let mut pairs_priced = 0;
            let mut savings = Vec::new();
            for task in &tasks {
                let mut ours = 0.0;
                let mut theirs = 0.0;
                let mut saved = Vec::new();
                for probe in &probes {
                    let attempt = of(arm, task, probe);
                    let baseline = of("cold", task, probe);
                    pairs += 1;
                    match (
                        attempt["accepted"].as_bool(),
                        baseline["accepted"].as_bool(),
                    ) {
                        (Some(attempt), Some(baseline)) => {
                            ours += f64::from(u8::from(attempt));
                            theirs += f64::from(u8::from(baseline));
                        }
                        _ => valid = false,
                    }
                    // A cheaper attempt that was not accepted saved nothing.
                    if attempt["accepted"] != true || baseline["accepted"] != true {
                        continue;
                    }
                    pairs_accepted += 1;
                    if let (Some(warm), Some(cold)) = (
                        full_price(&attempt["usage"], executor_row),
                        full_price(&baseline["usage"], executor_row),
                    ) {
                        pairs_priced += 1;
                        saved.push(cold - warm);
                    }
                }
                differences.push((ours - theirs) / probes.len() as f64);
                if let Some(saved) = mean(&saved) {
                    savings.push(saved);
                }
            }
            let difference = valid
                .then(|| interval(&differences, rounds, seed))
                .flatten();
            let quality = match difference {
                None => "insufficient",
                Some([_, high]) if high < -margin => "worse",
                Some([low, _]) if low >= -margin => "non-inferior",
                Some(_) => "not-shown",
            };
            let saving = mean(&savings);
            let saving_interval = interval(&savings, rounds, seed);
            let verdict = match (quality, saving_interval) {
                ("insufficient", _) => "insufficient",
                ("worse" | "not-shown", _) => "not-accepted",
                (_, Some(_)) if savings.len() < minimum => "quality-holds-cost-unknown",
                (_, None) => "quality-holds-cost-unknown",
                (_, Some([low, _])) if low > 0.0 => "saves",
                (_, Some(_)) => "no-saving-shown",
            };
            let breakeven = match (saving, compile) {
                (Some(saving), Some(compile)) if saving > 0.0 => Some((compile / saving).ceil()),
                _ => None,
            };
            let net = items(&sealed["reuses"])
                .filter_map(Value::as_u64)
                .map(|reuses| {
                    (
                        reuses.to_string(),
                        rounded(
                            saving
                                .zip(compile)
                                .map(|(saving, compile)| reuses as f64 * saving - compile),
                        ),
                    )
                })
                .collect::<Map<_, _>>();
            summary["against_cold"] = json!({
                "pairs": pairs,
                "pairs_accepted": pairs_accepted,
                "pairs_priced": pairs_priced,
                "tasks_priced": savings.len(),
                "acceptance_difference": rounded(valid.then(|| mean(&differences)).flatten()),
                "acceptance_difference_ci95": shown(difference),
                "quality": quality,
                "saving_per_reuse_usd": rounded(saving),
                "saving_ci95_usd": shown(saving_interval),
                "breakeven_reuses": breakeven.map_or(Value::Null, |reuses| json!(reuses as u64)),
                "net_saving_usd": net,
                "verdict": verdict,
            });
            summaries.insert(arm.clone(), summary);
        }
        cells.push(json!({
            "compile_model": cell["compile_model"],
            "executor_model": cell["executor_model"],
            "arms": summaries,
        }));
    }
    Ok(json!({
        "schema": REPORT,
        "seal_sha256": sealed["seal_sha256"],
        "question": sealed["question"],
        "design": design,
        "tasks": tasks.len(),
        "paired_unit": "task",
        "judged_by": if sealed["verifiers"].is_null() { "driver" } else { "verifier" },
        "cells": cells,
        // Applying a verdict is a person's decision, and never one for a
        // default.
        "note": "A report is evidence for the sealed question. It enables nothing by default.",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn protocol() -> Value {
        json!({
            "schema": PROTOCOL,
            "question": "Does a warm turn cost less than a cold one for the same accepted work?",
            "design": {
                "primary": "correctness",
                "quality_margin": 0.1,
                "minimum_tasks": 3,
                "power_rationale": "A regression fixture, not a powered study.",
                "selection": "Every task of the fixture.",
                "exclusion": "None.",
                "secondary_benefit": "List price per accepted reuse.",
                "stopping_rule": "fixed-horizon",
                "unknown_policy": "insufficient"
            },
            "arms": ["cold", "skillcache_executor", "raw_trace"],
            "probes": ["exact", "rename"],
            "matrix": [{"compile_model": "compiler", "executor_model": "worker"}],
            "tasks": [
                {"id": "t1", "skill": "pdf", "observe": "Extract rows",
                 "probes": {"rename": "Extract rows of b"}},
                {"id": "t2", "skill": "pdf", "observe": "Extract totals",
                 "probes": {"rename": "Extract totals of b"}},
                {"id": "t3", "skill": "pdf", "observe": "Extract dates",
                 "probes": {"rename": "Extract dates of b"}}
            ],
            "prices": {
                "compiler": {"input": 10.0, "output": 50.0, "cache_read": 1.0, "cache_write": 12.5},
                "worker": {"input": 2.0, "output": 10.0, "cache_read": 0.2, "cache_write": 2.5}
            },
            "reuses": [1, 10],
            "bootstrap": {"rounds": 200, "seed": 7},
            "driver": {"argv": ["driver"], "timeout_seconds": 60}
        })
    }
    fn tokens(input: u64, output: u64, read: Option<u64>, write: Option<u64>) -> Value {
        json!({"input_tokens": input, "output_tokens": output,
               "cache_read_input_tokens": read, "cache_creation_input_tokens": write})
    }
    fn sealed(folder: &Path, document: &Value) -> PathBuf {
        let source = folder.join("protocol.json");
        std::fs::write(&source, document.to_string()).unwrap();
        let out = folder.join("sealed.json");
        seal(&source, &out).unwrap();
        out
    }
    /// A complete result: every planned record, made by the two closures.
    fn result(
        folder: &Path,
        sealed_path: &Path,
        attempt: impl Fn(&Slot) -> Value,
        preparation: impl Fn(&Preparation) -> Value,
    ) -> PathBuf {
        let sealed = load(sealed_path).unwrap();
        let (preparations, attempts) = planned(&sealed);
        let attempts = attempts
            .iter()
            .map(|slot| {
                let mut record = json!({
                    "task": slot.0, "probe": slot.1, "arm": slot.2, "compile_model": slot.3,
                    "executor_model": slot.4, "status": "finished",
                });
                for (name, value) in attempt(slot).as_object().unwrap() {
                    record[name] = value.clone();
                }
                record
            })
            .collect::<Vec<_>>();
        let preparations = preparations
            .iter()
            .map(|key| {
                let mut record = json!({
                    "task": key.0, "arm": key.1, "compile_model": key.2, "executor_model": key.3,
                    "status": "finished",
                });
                for (name, value) in preparation(key).as_object().unwrap() {
                    record[name] = value.clone();
                }
                record
            })
            .collect::<Vec<_>>();
        let path = folder.join("result.json");
        std::fs::write(
            &path,
            json!({"schema": RESULT, "seal_sha256": sealed["seal_sha256"],
                   "preparations": preparations, "attempts": attempts})
            .to_string(),
        )
        .unwrap();
        path
    }
    fn scratch() -> artifact::Scratch {
        artifact::Scratch::new(&std::env::temp_dir(), "skillcache-eval").unwrap()
    }
    /// Cold costs 0.30 USD, warm 0.06 USD, a compilation 0.20 USD.
    fn plain_attempt(slot: &Slot) -> Value {
        if slot.2 == "cold" {
            json!({"accepted": true, "usage": tokens(100_000, 10_000, Some(0), Some(0)),
                   "model_calls": 9, "seconds": 30.0})
        } else {
            json!({"accepted": true, "usage": tokens(20_000, 2_000, Some(0), Some(0)),
                   "model_calls": 3, "seconds": 8.0, "program_steps": 2, "failed_steps": 1})
        }
    }
    fn plain_preparation(key: &Preparation) -> Value {
        if key.1 == "raw_trace" {
            json!({"prepared": true, "usage": tokens(0, 0, Some(0), Some(0))})
        } else {
            json!({"prepared": true, "usage": tokens(10_000, 2_000, Some(0), Some(0))})
        }
    }

    #[test]
    fn a_protocol_is_checked_before_it_is_sealed() {
        validate(&protocol(), false).unwrap();
        let refused = |change: &dyn Fn(&mut Value)| {
            let mut document = protocol();
            change(&mut document);
            validate(&document, false).is_err()
        };
        assert!(refused(
            &|d| d["arms"] = json!(["skillcache_executor", "raw_trace"])
        ));
        assert!(
            refused(&|d| d["arms"] = json!(["cold"])),
            "nothing to compare"
        );
        assert!(refused(&|d| d["arms"] = json!(["cold", "magic"])));
        assert!(refused(&|d| d["probes"] = json!(["exact", "exact"])));
        assert!(refused(&|d| {
            d["prices"].as_object_mut().unwrap().remove("compiler");
        }));
        assert!(refused(&|d| {
            d["prices"]["worker"]
                .as_object_mut()
                .unwrap()
                .remove("cache_write");
        }));
        assert!(refused(
            &|d| d["prices"]["other"] = d["prices"]["worker"].clone()
        ));
        assert!(refused(&|d| d["tasks"][1]["id"] = "t1".into()));
        assert!(refused(&|d| d["tasks"][0]["id"] = "../t1".into()));
        // Every probe but `exact` has its own request in every task.
        assert!(refused(&|d| {
            d["tasks"][0].as_object_mut().unwrap().remove("probes");
        }));
        assert!(refused(
            &|d| d["tasks"][0]["probes"]["exact"] = "Extract rows".into()
        ));
        assert!(refused(&|d| d["reuses"] = json!([0])));
        assert!(refused(&|d| d["reuses"] = json!([5, 5])));
        assert!(refused(
            &|d| d["bootstrap"] = json!({"rounds": 5, "seed": 1})
        ));
        assert!(refused(
            &|d| d["driver"] = json!({"argv": [], "timeout_seconds": 60})
        ));
        assert!(refused(&|d| d["driver"]["timeout_seconds"] = json!(0)));
        // The design is the outcome evaluation's: an unknown is never a pass.
        assert!(refused(&|d| d["design"]["unknown_policy"] = "ignore".into()));
        assert!(refused(
            &|d| d["design"]["stopping_rule"] = "when-it-looks-good".into()
        ));
        assert!(
            refused(&|d| d["threshold"] = json!(0.5)),
            "nothing is added"
        );
        assert!(refused(
            &|d| d["schema"] = "skillcache-eval-protocol-v2".into()
        ));
        // One experiment is judged one way.
        assert!(refused(&|d| d["tasks"][0]["verifier"] = json!(["true"])));
        assert!(refused(&|d| {
            d["verifiers"] = json!({"root": "/verifiers", "timeout_seconds": 60});
        }));
        let mut verified = protocol();
        verified["verifiers"] = json!({"root": "/verifiers", "timeout_seconds": 60});
        for task in verified["tasks"].as_array_mut().unwrap() {
            task["verifier"] = json!(["sh", "{verifiers}/check.sh"]);
        }
        validate(&verified, false).unwrap();
        verified["verifiers"]["root"] = "verifiers".into();
        assert!(validate(&verified, false).is_err(), "an absolute folder");
    }

    #[test]
    fn compile_free_arms_are_planned_once_for_an_executor_model() {
        let mut document = protocol();
        document["matrix"] = json!([
            {"compile_model": "compiler", "executor_model": "worker"},
            {"compile_model": "worker", "executor_model": "worker"}
        ]);
        let (preparations, attempts) = planned(&document);
        // cold and raw_trace: 3 tasks x 2 probes each; the compiled arm twice.
        assert_eq!(attempts.len(), 6 + 6 + 12);
        assert_eq!(preparations.len(), 3 + 6);
        assert!(
            attempts
                .iter()
                .filter(|slot| slot.2 == "cold" || slot.2 == "raw_trace")
                .all(|slot| slot.3.is_none())
        );
    }

    #[test]
    fn a_seal_is_written_once_and_detects_a_changed_protocol() {
        let root = scratch();
        let path = sealed(&root.0, &protocol());
        assert!(seal(&root.0.join("protocol.json"), &path).is_err());
        let mut stored: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(stored["schema"], PROTOCOL);
        assert!(stored["sealed_at"].is_string());
        load(&path).unwrap();
        // The price table is part of the seal.
        stored["prices"]["worker"]["input"] = json!(0.1);
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, stored.to_string()).unwrap();
        let error = load(&path).unwrap_err();
        assert!(
            error.message.contains("changed after it was sealed"),
            "{error:?}"
        );
        // An unsealed protocol is not run.
        assert!(load(&root.0.join("protocol.json")).is_err());
    }

    #[test]
    fn a_report_prices_accepted_pairs_and_names_its_verdict() {
        let root = scratch();
        let sealed_path = sealed(&root.0, &protocol());
        let result_path = result(&root.0, &sealed_path, plain_attempt, plain_preparation);
        let summary = report(&sealed_path, &result_path).unwrap();
        assert_eq!(summary["schema"], REPORT);
        assert_eq!(summary["judged_by"], "driver");
        let arms = &summary["cells"][0]["arms"];
        let cold = &arms["cold"];
        assert_eq!(cold["planned"], 6);
        assert_eq!(cold["acceptance_rate"], 1.0);
        assert_eq!(cold["list_price_usd"], 0.3);
        assert_eq!(cold["model_calls"], 9.0);
        assert!(
            cold["deopt_rate"].is_null(),
            "no program steps were reported"
        );
        assert!(cold.get("against_cold").is_none());
        let warm = &arms["skillcache_executor"];
        assert_eq!(warm["list_price_usd"], 0.06);
        assert_eq!(warm["deopt_rate"], 0.5);
        assert_eq!(warm["preparation"]["prepared"], 3);
        // 0.01 M x 10 + 0.002 M x 50, at the compile model's price.
        assert_eq!(warm["preparation"]["compile_usd"], 0.2);
        let against = &warm["against_cold"];
        assert_eq!(against["pairs"], 6);
        assert_eq!(against["pairs_priced"], 6);
        assert_eq!(against["tasks_priced"], 3);
        assert_eq!(against["quality"], "non-inferior");
        assert_eq!(against["acceptance_difference"], 0.0);
        assert_eq!(against["saving_per_reuse_usd"], 0.24);
        assert_eq!(against["saving_ci95_usd"], json!([0.24, 0.24]));
        assert_eq!(against["breakeven_reuses"], 1);
        assert_eq!(against["net_saving_usd"]["1"], 0.04);
        assert_eq!(against["net_saving_usd"]["10"], 2.2);
        assert_eq!(against["verdict"], "saves");
        // The trace arm compiled nothing, at the executor's price.
        assert_eq!(arms["raw_trace"]["preparation"]["compile_usd"], 0.0);
        assert_eq!(arms["raw_trace"]["against_cold"]["breakeven_reuses"], 0);
    }

    #[test]
    fn an_unknown_is_never_a_pass_and_never_a_zero() {
        let root = scratch();
        let sealed_path = sealed(&root.0, &protocol());
        // One warm attempt was not judged; cache writes were never reported.
        let result_path = result(
            &root.0,
            &sealed_path,
            |slot| {
                let mut attempt = plain_attempt(slot);
                attempt["usage"]["cache_creation_input_tokens"] = Value::Null;
                if slot.0 == "t2" && slot.1 == "rename" && slot.2 == "skillcache_executor" {
                    attempt["accepted"] = Value::Null;
                }
                attempt
            },
            plain_preparation,
        );
        let summary = report(&sealed_path, &result_path).unwrap();
        let warm = &summary["cells"][0]["arms"]["skillcache_executor"];
        assert_eq!(warm["judged"], 5);
        assert!(warm["acceptance_rate"].is_null());
        assert_eq!(warm["coverage"]["usage"]["cache_creation_input_tokens"], 0);
        assert_eq!(warm["coverage"]["usage"]["input_tokens"], 6);
        assert_eq!(warm["coverage"]["fully_priced"], 0);
        assert!(warm["list_price_usd"].is_null());
        // What was reported is still shown, as a lower bound.
        assert_eq!(warm["list_price_usd_reported_components"], 0.06);
        assert!(warm["tokens"]["cache_creation_input_tokens"].is_null());
        let against = &warm["against_cold"];
        assert_eq!(against["quality"], "insufficient");
        assert_eq!(against["verdict"], "insufficient");
        assert_eq!(against["pairs_accepted"], 5);
        assert_eq!(against["pairs_priced"], 0);
        assert!(against["saving_per_reuse_usd"].is_null());
        assert!(against["breakeven_reuses"].is_null());
        assert!(against["net_saving_usd"]["10"].is_null());
        assert!(against["acceptance_difference_ci95"].is_null());
    }

    #[test]
    fn worse_work_is_not_accepted_however_cheap() {
        let root = scratch();
        let sealed_path = sealed(&root.0, &protocol());
        let result_path = result(
            &root.0,
            &sealed_path,
            |slot| {
                let mut attempt = plain_attempt(slot);
                if slot.2 == "skillcache_executor" {
                    attempt["accepted"] = false.into();
                    attempt["usage"] = tokens(100, 10, Some(0), Some(0));
                }
                attempt
            },
            plain_preparation,
        );
        let summary = report(&sealed_path, &result_path).unwrap();
        let against = &summary["cells"][0]["arms"]["skillcache_executor"]["against_cold"];
        assert_eq!(against["quality"], "worse");
        assert_eq!(against["verdict"], "not-accepted");
        assert_eq!(against["pairs_accepted"], 0);
        assert!(against["saving_per_reuse_usd"].is_null());
        assert!(against["breakeven_reuses"].is_null());
    }

    #[test]
    fn no_saving_means_no_breakeven() {
        let root = scratch();
        let sealed_path = sealed(&root.0, &protocol());
        let result_path = result(
            &root.0,
            &sealed_path,
            |slot| {
                let mut attempt = plain_attempt(slot);
                if slot.2 == "skillcache_executor" {
                    attempt["usage"] = tokens(200_000, 20_000, Some(0), Some(0));
                }
                attempt
            },
            plain_preparation,
        );
        let summary = report(&sealed_path, &result_path).unwrap();
        let against = &summary["cells"][0]["arms"]["skillcache_executor"]["against_cold"];
        assert_eq!(against["quality"], "non-inferior");
        assert_eq!(against["saving_per_reuse_usd"], -0.3);
        assert!(against["breakeven_reuses"].is_null());
        assert_eq!(against["net_saving_usd"]["10"], -3.2);
        assert_eq!(against["verdict"], "no-saving-shown");
    }

    #[test]
    fn a_result_holds_exactly_the_plan_of_its_own_protocol() {
        let root = scratch();
        let sealed_path = sealed(&root.0, &protocol());
        let result_path = result(&root.0, &sealed_path, plain_attempt, plain_preparation);
        let stored: Value = serde_json::from_slice(&std::fs::read(&result_path).unwrap()).unwrap();
        let rewrite = |change: &dyn Fn(&mut Value)| {
            let mut changed = stored.clone();
            change(&mut changed);
            std::fs::remove_file(&result_path).unwrap();
            std::fs::write(&result_path, changed.to_string()).unwrap();
            report(&sealed_path, &result_path).is_err()
        };
        assert!(rewrite(&|result| {
            result["attempts"].as_array_mut().unwrap().pop();
        }));
        assert!(rewrite(&|result| {
            let again = result["attempts"][0].clone();
            result["attempts"].as_array_mut().unwrap().push(again);
        }));
        assert!(rewrite(
            &|result| result["attempts"][0]["arm"] = "written_procedure".into()
        ));
        assert!(rewrite(&|result| {
            result["preparations"].as_array_mut().unwrap().remove(0);
        }));
        assert!(!rewrite(&|_| {}));
        // A result of another sealed protocol is refused.
        let mut other = protocol();
        other["question"] = "Another question".into();
        let folder = scratch();
        let other_sealed = sealed(&folder.0, &other);
        assert!(report(&other_sealed, &result_path).is_err());
    }
}
