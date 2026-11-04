//! Native check catalog and bounded concurrent runner. Python is oracle-only.
use crate::{common, process, temp::Temporary};
use infra_core::{Result, RuntimeError, fs as safe};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, VecDeque},
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
static INTERRUPTED: AtomicBool = AtomicBool::new(false);
#[cfg(windows)]
unsafe extern "system" fn console_interrupted(event: u32) -> i32 {
    if [0, 1, 2, 5, 6].contains(&event) {
        INTERRUPTED.store(true, Ordering::Release);
        1
    } else {
        0
    }
}
#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetConsoleCtrlHandler(
        handler: Option<unsafe extern "system" fn(u32) -> i32>,
        add: i32,
    ) -> i32;
}
#[cfg(unix)]
extern "C" fn interrupted(_: i32) {
    INTERRUPTED.store(true, Ordering::Release);
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub schema_version: u32,
    pub groups: Vec<Group>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub name: String,
    pub workflow: String,
    pub job: String,
    pub timeout_minutes: u64,
    #[serde(default = "yes")]
    pub default: bool,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default = "stable_toolchain")]
    pub rust_toolchain: String,
    #[serde(default)]
    pub rust_targets: Vec<String>,
    pub gates: Vec<Gate>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Gate {
    Tool {
        name: String,
        operation: String,
        #[serde(default)]
        request: Value,
    },
    Process {
        name: String,
        program: String,
        #[serde(default)]
        arguments: Vec<String>,
        #[serde(default = "dot")]
        cwd: String,
        #[serde(default)]
        tools: Vec<String>,
        #[serde(default)]
        paths: Vec<String>,
        #[serde(default = "seconds")]
        timeout_seconds: u64,
        #[serde(default)]
        oracle: bool,
    },
}
fn yes() -> bool {
    true
}
fn dot() -> String {
    ".".into()
}
fn seconds() -> u64 {
    300
}
fn stable_toolchain() -> String {
    "stable".into()
}
fn valid_rust_toolchain(value: &str) -> bool {
    if value == "stable" {
        return true;
    }
    let mut parts = value.split('.');
    let numbers = [parts.next(), parts.next(), parts.next()].map(|part| {
        part.filter(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (*part == "0" || !part.starts_with('0'))
        })
        .and_then(|part| part.parse::<u16>().ok())
    });
    parts.next().is_none() && matches!(numbers, [Some(1), Some(minor), Some(_)] if minor >= 85)
}
fn validate_rust_configuration(group: &Group) -> Result<()> {
    if !valid_rust_toolchain(&group.rust_toolchain) {
        return Err(RuntimeError::invalid(
            "CI toolchain must be stable or a pinned Rust 1.85+ release",
        ));
    }
    if group.rust_targets.iter().collect::<BTreeSet<_>>().len() != group.rust_targets.len()
        || group.rust_targets.iter().any(|target| {
            target.is_empty()
                || target.len() > 128
                || !target
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
        })
    {
        return Err(RuntimeError::invalid(
            "Native CI Rust targets must be unique target identifiers",
        ));
    }
    Ok(())
}
fn process_argument(value: &str, root: &Path, base: &str, toolchain: &str) -> String {
    // Expand the selected compiler before inserting caller-owned path strings.
    value
        .replace("{toolchain}", toolchain)
        .replace("{root}", &root.to_string_lossy())
        .replace("{base}", base)
        .replace(
            "{engine}",
            &std::env::current_exe()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_default(),
        )
}
impl Gate {
    fn name(&self) -> &str {
        match self {
            Self::Tool { name, .. } | Self::Process { name, .. } => name,
        }
    }
}
#[derive(Clone, Debug, Serialize)]
struct Outcome {
    group: String,
    name: String,
    status: String,
    seconds: f64,
    code: Option<i32>,
    reason: Option<String>,
    log: Option<PathBuf>,
    /// The last lines of a failed check's streams. A CI job's retained log
    /// ends with its runner, so the run's own output must show the failure.
    #[serde(skip)]
    excerpt: Vec<(&'static str, String)>,
}
const EXCERPT_LINES: usize = 150;
const EXCERPT_BYTES: usize = 48 * 1024;
/// The end of each nonempty stream, at most `EXCERPT_LINES` lines of its last
/// `EXCERPT_BYTES` bytes.
fn excerpt(streams: &[(&'static str, &[u8])]) -> Vec<(&'static str, String)> {
    streams
        .iter()
        .filter(|(_, bytes)| !bytes.is_empty())
        .map(|(stream, bytes)| {
            let window = &bytes[bytes.len().saturating_sub(EXCERPT_BYTES)..];
            let text = window.strip_suffix(b"\n").unwrap_or(window);
            let start = text
                .iter()
                .enumerate()
                .rev()
                .filter(|(_, byte)| **byte == b'\n')
                .nth(EXCERPT_LINES - 1)
                .map_or(0, |(at, _)| at + 1);
            (
                *stream,
                String::from_utf8_lossy(&text[start..]).into_owned(),
            )
        })
        .collect()
}
/// What each failed check printed last; empty when nothing failed.
fn failure_report(results: &[Outcome]) -> String {
    let mut report = String::new();
    for result in results.iter().filter(|result| result.status == "fail") {
        report.push_str(&format!("FAILED {} / {}", result.group, result.name));
        if let Some(code) = result.code {
            report.push_str(&format!(" (exit {code})"));
        }
        if let Some(reason) = &result.reason {
            report.push_str(&format!(": {reason}"));
        }
        report.push('\n');
        for (stream, text) in &result.excerpt {
            report.push_str(&format!("--- end of {stream} ---\n{text}\n"));
        }
    }
    report
}
fn interpolate(value: &Value, root: &Path, base: &str) -> Value {
    match value {
        Value::String(value) => Value::String(
            value
                .replace("{root}", &root.to_string_lossy())
                .replace("{base}", base),
        ),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| interpolate(value, root, base))
                .collect(),
        ),
        Value::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), interpolate(value, root, base)))
                .collect(),
        ),
        _ => value.clone(),
    }
}
fn available(tool: &str, root: &Path) -> bool {
    let path = Path::new(tool);
    if path.is_absolute() || tool.contains('/') || tool.contains('\\') {
        return root.join(path).is_file();
    }
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|directory| {
            let path = directory.join(tool);
            if path.is_file() {
                return true;
            }
            cfg!(windows)
                && ["exe", "cmd", "bat"]
                    .iter()
                    .any(|extension| directory.join(format!("{tool}.{extension}")).is_file())
        })
    })
}
fn load_catalog(root: &Path, require_workflows: bool) -> Result<Catalog> {
    let path = root.join("install/check-catalog.json");
    let catalog: Catalog = serde_json::from_slice(&safe::read_regular(&path)?)?;
    if catalog.schema_version != 1 || catalog.groups.is_empty() {
        return Err(RuntimeError::invalid(
            "Unsupported or empty native check catalog",
        ));
    }
    let mut names = BTreeSet::new();
    let mut jobs = BTreeSet::new();
    for group in &catalog.groups {
        if group.name.is_empty()
            || !names.insert(&group.name)
            || !jobs.insert((&group.workflow, &group.job))
            || group.gates.is_empty()
            || group.timeout_minutes == 0
        {
            return Err(RuntimeError::invalid(
                "Native check catalog has duplicate or empty groups",
            ));
        }
        validate_rust_configuration(group)?;
        common::relative(&group.workflow)?;
        if require_workflows
            && !root
                .join(".github/workflows")
                .join(&group.workflow)
                .is_file()
        {
            return Err(RuntimeError::invalid(
                "Check catalog names an absent workflow",
            ));
        }
        for gate in &group.gates {
            if gate.name().is_empty() {
                return Err(RuntimeError::invalid("Check catalog gate has no name"));
            }
            if let Gate::Process {
                program, oracle, ..
            } = gate
                && (program == "python" || program.starts_with("python3"))
                && !*oracle
            {
                return Err(RuntimeError::policy(
                    "Python command is allowed only as an explicitly selected verification oracle",
                ));
            }
        }
    }
    Ok(catalog)
}
struct GateContext<'a> {
    root: &'a Path,
    base: &'a str,
    group: &'a Group,
    strict: bool,
    oracle_allowed: bool,
    logs: &'a Path,
    cancel: Arc<AtomicBool>,
}
fn gate(context: GateContext<'_>, gate: &Gate) -> Outcome {
    let GateContext {
        root,
        base,
        group,
        strict,
        oracle_allowed,
        logs,
        cancel,
    } = context;
    let started = Instant::now();
    let mut outcome = Outcome {
        group: group.name.clone(),
        name: gate.name().into(),
        status: "pass".into(),
        seconds: 0.0,
        code: Some(0),
        reason: None,
        log: None,
        excerpt: Vec::new(),
    };
    let executed = (|| -> Result<(Option<i32>, Vec<u8>)> {
        if cancel.load(Ordering::Acquire) || INTERRUPTED.load(Ordering::Acquire) {
            outcome.status = "cancelled".into();
            return Ok((None, vec![]));
        }
        if group
            .platform
            .as_ref()
            .is_some_and(|platform| platform != std::env::consts::OS)
        {
            outcome.status = "skipped".into();
            outcome.reason = Some("Platform-specific check".into());
            return Ok((None, vec![]));
        }
        match gate {
            Gate::Tool {
                operation, request, ..
            } => {
                let mut request = interpolate(request, root, base);
                if request.is_null() {
                    request = json!({});
                }
                request
                    .as_object_mut()
                    .ok_or_else(|| RuntimeError::invalid("Tool check request must be an object"))?
                    .insert("root".into(), json!(root));
                let report =
                    process::with_cancel(cancel.clone(), || crate::execute(operation, &request))?;
                if report["status"] == "skipped" {
                    outcome.status = "skipped".into();
                    outcome.reason = Some(
                        report["reason"]
                            .as_str()
                            .unwrap_or("Explicitly skipped")
                            .into(),
                    );
                } else {
                    let code = crate::dispatch::report_exit_code(&report);
                    if code != 0 {
                        outcome.status = "fail".into();
                        outcome.code = Some(code);
                    }
                }
                let bytes = common::json_bytes(&report)?;
                if outcome.status == "fail" {
                    outcome.excerpt = excerpt(&[("report", bytes.as_slice())]);
                }
                Ok((outcome.code, bytes))
            }
            Gate::Process {
                program,
                arguments,
                cwd,
                tools,
                paths,
                timeout_seconds,
                oracle,
                ..
            } => {
                if *oracle && !oracle_allowed {
                    outcome.status = "skipped".into();
                    outcome.reason = Some("Legacy oracle was not explicitly requested".into());
                    return Ok((None, vec![]));
                }
                let expand =
                    |value: &str| process_argument(value, root, base, &group.rust_toolchain);
                for path in paths {
                    if !root.join(expand(path)).exists() {
                        outcome.status = if strict { "fail" } else { "skipped" }.into();
                        outcome.reason = Some(format!("Required check file is absent: {path}"));
                        return Ok((if strict { Some(1) } else { None }, vec![]));
                    }
                }
                let program = expand(program);
                for tool in
                    std::iter::once(program.as_str()).chain(tools.iter().map(String::as_str))
                {
                    if !available(tool, root) {
                        outcome.status = if strict { "fail" } else { "skipped" }.into();
                        outcome.reason =
                            Some(format!("Required executable is unavailable: {tool}"));
                        return Ok((if strict { Some(1) } else { None }, vec![]));
                    }
                }
                let arguments = arguments
                    .iter()
                    .map(|value| OsString::from(expand(value)))
                    .collect::<Vec<_>>();
                let cwd = root.join(expand(cwd));
                let output = process::with_cancel(cancel, || {
                    process::run(
                        std::ffi::OsStr::new(&program),
                        &arguments,
                        &cwd,
                        None,
                        Duration::from_secs(*timeout_seconds),
                        16 * 1024 * 1024,
                    )
                })?;
                if output.code != 0 {
                    outcome.status = "fail".into();
                    // Separately: a test harness prints its failures to stdout,
                    // while Cargo names the failed target on stderr.
                    outcome.excerpt = excerpt(&[
                        ("stdout", output.stdout.as_slice()),
                        ("stderr", output.stderr.as_slice()),
                    ]);
                }
                let mut bytes = output.stdout;
                bytes.extend_from_slice(&output.stderr);
                Ok((Some(output.code), bytes))
            }
        }
    })();
    let (code, bytes) = match executed {
        Ok(value) => value,
        Err(error) => {
            outcome.status = if INTERRUPTED.load(Ordering::Acquire) {
                "cancelled"
            } else {
                "fail"
            }
            .into();
            outcome.reason = Some(error.to_string());
            (Some(1), error.to_string().into_bytes())
        }
    };
    outcome.code = code;
    outcome.seconds = started.elapsed().as_secs_f64();
    if outcome.status == "fail" {
        let path = logs.join(format!(
            "{}-{}.log",
            group.name,
            uuid::Uuid::new_v4().simple()
        ));
        if safe::atomic_write(&path, &bytes).is_ok() {
            outcome.log = Some(path);
        }
    }
    outcome
}
pub fn execute(request: &Value) -> Result<Value> {
    let root = infra_core::fs::canonicalize(request["root"].as_str().unwrap_or("."))?;
    let catalog = load_catalog(&root, true)?;
    let names = request["groups"]
        .as_array()
        .map(|values| {
            values
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(String::from)
                        .ok_or_else(|| RuntimeError::invalid("Check groups must be strings"))
                })
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    for name in &names {
        if !catalog.groups.iter().any(|group| &group.name == name) {
            return Err(RuntimeError::invalid(format!(
                "Unknown native check group: {name}"
            )));
        }
    }
    let oracle = request["include_oracles"].as_bool().unwrap_or(false);
    let groups = catalog
        .groups
        .into_iter()
        .filter(|group| {
            if names.is_empty() {
                group.default
            } else {
                names.contains(&group.name)
            }
        })
        .collect::<Vec<_>>();
    if request["list"].as_bool().unwrap_or(false) {
        return Ok(
            json!({"groups":groups.iter().map(|group|json!({"name":group.name,"workflow":group.workflow,"job":group.job,"timeout_minutes":group.timeout_minutes,"gates":group.gates.iter().map(|gate|gate.name()).collect::<Vec<_>>()})).collect::<Vec<_>>()}),
        );
    }
    let requested_jobs = request["jobs"].as_u64().unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|value| (value.get() / 2).max(2) as u64)
            .unwrap_or(2)
    });
    if requested_jobs == 0 {
        return Err(RuntimeError::invalid("Check worker count must be positive"));
    }
    let jobs = requested_jobs.min(groups.len().max(1) as u64) as usize;
    let strict = request["strict"].as_bool().unwrap_or(false);
    let fail_fast = request["fail_fast"].as_bool().unwrap_or(false);
    let base = request["base"]
        .as_str()
        .unwrap_or("origin/main")
        .to_string();
    let (mut temporary, logs) = match request["log_dir"].as_str() {
        // A named directory keeps failed logs where a CI job can collect them.
        Some(directory) => {
            let directory = PathBuf::from(directory);
            safe::ensure_directory(&directory, false)?;
            (None, directory)
        }
        None => {
            let temporary = Temporary::new("infra-check")?;
            let logs = temporary.path().to_path_buf();
            (Some(temporary), logs)
        }
    };
    let queue = Arc::new(Mutex::new(VecDeque::from(groups)));
    let cancelled = Arc::new(AtomicBool::new(false));
    let outcomes = Arc::new(Mutex::new(Vec::new()));
    let started = Instant::now();
    INTERRUPTED.store(false, Ordering::Release);
    #[cfg(unix)]
    let signals = unsafe {
        [
            (
                libc::SIGINT,
                libc::signal(libc::SIGINT, interrupted as *const () as libc::sighandler_t),
            ),
            (
                libc::SIGTERM,
                libc::signal(
                    libc::SIGTERM,
                    interrupted as *const () as libc::sighandler_t,
                ),
            ),
            (
                libc::SIGHUP,
                libc::signal(libc::SIGHUP, interrupted as *const () as libc::sighandler_t),
            ),
        ]
    };
    #[cfg(windows)]
    if unsafe { SetConsoleCtrlHandler(Some(console_interrupted), 1) } == 0 {
        return Err(RuntimeError::unavailable(
            "Native check cancellation handler unavailable",
        ));
    }
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            let queue = queue.clone();
            let cancelled = cancelled.clone();
            let outcomes = outcomes.clone();
            let root = &root;
            let base = &base;
            let logs = &logs;
            scope.spawn(move || {
                loop {
                    if INTERRUPTED.load(Ordering::Acquire) {
                        cancelled.store(true, Ordering::Release);
                    }
                    let group = queue.lock().ok().and_then(|mut queue| queue.pop_front());
                    let Some(group) = group else {
                        break;
                    };
                    for check in &group.gates {
                        let result = gate(
                            GateContext {
                                root,
                                base,
                                group: &group,
                                strict,
                                oracle_allowed: oracle,
                                logs,
                                cancel: cancelled.clone(),
                            },
                            check,
                        );
                        if fail_fast && result.status == "fail" {
                            cancelled.store(true, Ordering::Release);
                        }
                        if let Ok(mut output) = outcomes.lock() {
                            output.push(result);
                        }
                    }
                }
            });
        }
    });
    #[cfg(unix)]
    unsafe {
        for (signal, previous) in signals {
            libc::signal(signal, previous);
        }
    }
    #[cfg(windows)]
    unsafe {
        SetConsoleCtrlHandler(Some(console_interrupted), 0);
    }
    let mut results = outcomes
        .lock()
        .map_err(|_| RuntimeError::unavailable("Check result collector was poisoned"))?
        .clone();
    results.sort_by(|a, b| (&a.group, &a.name).cmp(&(&b.group, &b.name)));
    let failed = results
        .iter()
        .filter(|result| result.status == "fail")
        .count();
    let skipped = results
        .iter()
        .filter(|result| result.status == "skipped")
        .count();
    let passed = results
        .iter()
        .filter(|result| result.status == "pass")
        .count();
    let interrupted = INTERRUPTED.load(Ordering::Acquire);
    let code = if interrupted {
        130
    } else if failed > 0 {
        1
    } else {
        0
    };
    let retained = if failed > 0 {
        Some(
            temporary
                .as_mut()
                .map_or_else(|| logs.clone(), Temporary::preserve),
        )
    } else {
        None
    };
    Ok(
        json!({"_cli":{"code":code,"stdout":serde_json::to_string_pretty(&json!({"passed":passed,"failed":failed,"skipped":skipped,"interrupted":interrupted,"results":results,"logs":retained,"wall_seconds":started.elapsed().as_secs_f64()}))?+"\n","stderr":failure_report(&results)},"result":{"exit_code":code,"passed":passed,"failed":failed,"skipped":skipped,"interrupted":interrupted,"results":results,"logs":retained}}),
    )
}

// Syntax-only admission keeps json.tool's duplicate-key, nonfinite-number and
// escaped-surrogate acceptance without constructing runtime authority values.
fn validate_json_syntax(bytes: &[u8]) -> std::result::Result<(), String> {
    std::str::from_utf8(bytes).map_err(|error| format!("Invalid UTF-8: {error}"))?;
    let mut normalized = std::borrow::Cow::Borrowed(bytes);
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'"' {
            at += 1;
            while at < bytes.len() {
                match bytes[at] {
                    b'\\' => at += 2,
                    b'"' => {
                        at += 1;
                        break;
                    }
                    _ => at += 1,
                }
            }
        } else if b" \t\r\n,]}:[{".contains(&bytes[at]) {
            at += 1;
        } else {
            let start = at;
            while at < bytes.len() && !b" \t\r\n,]}:[{\"".contains(&bytes[at]) {
                at += 1;
            }
            if [b"NaN".as_slice(), b"Infinity", b"-Infinity"].contains(&&bytes[start..at]) {
                let output = normalized.to_mut();
                output[start] = b'0';
                output[start + 1..at].fill(b' ');
            }
        }
    }
    let mut parser = serde_json::Deserializer::from_slice(normalized.as_ref());
    serde::de::IgnoredAny::deserialize(&mut parser).map_err(|error| error.to_string())?;
    parser.end().map_err(|error| error.to_string())
}

/// Inspect the current worktree images of exactly Git's indexed JSON members.
/// The 512 MiB per-file ceiling is an explicit read admission bound, not a schema.
pub fn check_tracked_json(root: &Path) -> Result<Value> {
    const MAX_JSON_BYTES: usize = 512 * 1024 * 1024;
    let root = infra_core::fs::canonicalize(root)?;
    let names = common::tracked(&root, &["*.json".into()])?;
    let mut files = infra_core::bank::publication::PublicationFiles::new(&root)?;
    let mut findings = Vec::new();
    for name in &names {
        let result = (|| -> std::result::Result<(), String> {
            let (bytes, _) = files
                .capture_source(name, MAX_JSON_BYTES)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "Indexed JSON file is absent from the worktree".to_string())?;
            validate_json_syntax(&bytes)
        })();
        if let Err(message) = result {
            findings.push(json!({"path":name,"message":message}));
        }
    }
    files.check()?;
    Ok(json!({
        "passed":findings.is_empty(),
        "files":names.len(),
        "findings":findings,
        "syntax_only":true,
        "maximum_file_bytes":MAX_JSON_BYTES,
        "selection":"git ls-files --cached -- '*.json'"
    }))
}

fn yaml_string(value: &str) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}
fn workflow(catalog: &Catalog, filename: &str) -> Result<String> {
    let mut groups = catalog
        .groups
        .iter()
        .filter(|group| group.workflow == filename)
        .collect::<Vec<_>>();
    groups.sort_by(|a, b| a.job.cmp(&b.job));
    let mut output = String::from(
        "# Generated by infra-engine write-ci from install/check-catalog.json.\n# Edit the catalog or native emitter; regenerate through the native command.\nname: ",
    );
    output.push_str(&yaml_string(if filename == "rust.yml" {
        "Rust native contracts"
    } else {
        "Native repository checks"
    })?);
    output.push_str("\non:\n  push:\n  pull_request:\n  workflow_dispatch:\n    inputs:\n      legacy_oracles:\n        description: Run retained verification references\n        required: false\n        type: boolean\n        default: false\npermissions:\n  contents: read\njobs:\n");
    for group in groups {
        if group.job.is_empty()
            || !group
                .job
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        {
            return Err(RuntimeError::invalid(
                "Native CI job name is not a GitHub identifier",
            ));
        }
        let oracle = group
            .gates
            .iter()
            .any(|gate| matches!(gate, Gate::Process { oracle: true, .. }));
        if oracle && group.default {
            return Err(RuntimeError::policy("Legacy reference jobs must be opt-in"));
        }
        output.push_str(&format!("  {}:\n", group.job));
        if oracle {
            output.push_str("    if: github.event_name == 'workflow_dispatch' && inputs.legacy_oracles == true\n");
        }
        let platform = group.platform.as_deref().unwrap_or("linux");
        let runner = match platform {
            "linux" => "ubuntu-latest",
            "windows" => "windows-latest",
            "macos" => "macos-latest",
            _ => {
                return Err(RuntimeError::invalid(
                    "Unsupported native CI catalog platform",
                ));
            }
        };
        // Job-level env admits only github, needs, strategy, matrix, vars,
        // secrets and inputs; any runner reference there makes GitHub reject
        // the whole file before a job exists. The target directory is set by
        // a step below instead.
        output.push_str(&format!(
            "    runs-on: {runner}\n    timeout-minutes: {}\n    env:\n      NATIVE_CHECK_GROUP: ",
            group.timeout_minutes
        ));
        output.push_str(&yaml_string(&group.name)?);
        // Match the test profile's debuginfo without changing local dev builds
        // or the debug/ engine path used by the catalog bootstrap. A job
        // compiles each changed crate once, so incremental state would only
        // add writes and cache weight.
        output.push_str("\n      CARGO_PROFILE_DEV_DEBUG: \"0\"\n      CARGO_INCREMENTAL: \"0\"\n    defaults:\n      run:\n        shell: bash\n    steps:\n      - uses: actions/checkout@v4\n        with:\n          fetch-depth: 0\n      - uses: dtolnay/rust-toolchain@stable\n        id: native-toolchain\n        with:\n          components: rustfmt, clippy\n");
        validate_rust_configuration(group)?;
        if group.rust_toolchain != "stable" {
            output.push_str("          toolchain: ");
            output.push_str(&yaml_string(&group.rust_toolchain)?);
            output.push('\n');
        }
        if !group.rust_targets.is_empty() {
            output.push_str("          targets: ");
            output.push_str(&yaml_string(&group.rust_targets.join(", "))?);
            output.push('\n');
        }
        if oracle {
            output.push_str("      - uses: actions/setup-python@v5\n        with:\n          python-version: '3.x'\n");
        }
        if group.gates.iter().any(|gate|matches!(gate,Gate::Tool{operation,request,..}if operation=="php-snippets"&&request["require_php"]==true)){
   output.push_str("      - uses: shivammathur/setup-php@v2\n        with:\n          php-version: '8.3'\n          coverage: none\n          tools: none\n");
  }
        // RUNNER_TEMP is a backslash path on Windows; Cargo and Git Bash both
        // accept the forward-slash spelling.
        output.push_str("      - name: Select the native target directory\n        run: echo \"CARGO_TARGET_DIR=${RUNNER_TEMP//\\\\//}/infra-native-target\" >> \"$GITHUB_ENV\"\n");
        // The toolchain action exports CARGO_HOME and an identity of the actual
        // installed compiler. Keep jobs separate so a lightweight engine-only
        // job cannot occupy the immutable cache key of the complete test job.
        // Cached artifacts are inputs to Cargo, never authority to skip it.
        // A missed key starts from the job's newest entry for this compiler,
        // and Cargo rebuilds only what changed since. v2 entries hold no
        // incremental state, so no v1 entry is restored into them.
        let prefix = format!(
            "native-cargo-v2-${{{{ runner.os }}}}-${{{{ runner.arch }}}}-${{{{ steps.native-toolchain.outputs.cachekey }}}}-{}-dev0-",
            group.job
        );
        output.push_str(concat!(
            "      - name: Restore native Cargo artifacts\n",
            "        id: native-cache\n",
            "        uses: actions/cache/restore@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6\n",
            "        with:\n",
        ));
        output.push_str(CARGO_CACHE_PATHS);
        output.push_str("          key: ");
        output.push_str(&prefix);
        output.push_str("${{ hashFiles('runtime-rust/Cargo.lock', 'runtime-rust/**/Cargo.toml', 'install/check-catalog.json', '.cargo/**', 'runtime-rust/.cargo/**') }}\n          restore-keys: ");
        output.push_str(&prefix);
        output.push('\n');
        // The group's own Cargo builds rewrite this engine in the target
        // directory, and Windows cannot replace a running executable: the
        // catalog runs from a copy outside it.
        output.push_str("      - name: Build the native engine\n        run: cargo build --manifest-path runtime-rust/Cargo.toml --locked --bin infra-engine\n      - name: Run the catalog group\n        run: |\n          suffix=''\n          if [ \"$RUNNER_OS\" = 'Windows' ]; then suffix='.exe'; fi\n          engine=\"${RUNNER_TEMP//\\\\//}/infra-check-engine$suffix\"\n          cp \"$CARGO_TARGET_DIR/debug/infra-engine$suffix\" \"$engine\"\n          \"$engine\" check --group \"$NATIVE_CHECK_GROUP\" --strict --base origin/main --log-dir \"${RUNNER_TEMP//\\\\//}/infra-check-logs\"");
        if oracle {
            output.push_str(" --include-oracles");
        }
        output.push('\n');
        // The run's own output shows only the end of each failed stream; the
        // job keeps every failed check's complete output.
        output.push_str(concat!(
            "      - name: Keep the complete output of failed checks\n",
            "        if: failure()\n",
            "        uses: actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7.0.2\n",
            "        with:\n",
            "          name: check-logs-${{ github.job }}\n",
            "          path: ${{ runner.temp }}/infra-check-logs\n",
            "          if-no-files-found: ignore\n",
            "          retention-days: 14\n",
        ));
        // A failed check still leaves the dependencies it compiled, and
        // keeping them spares the next run a cold build. An entry found under
        // its exact key is immutable and stays as it is.
        output.push_str(concat!(
            "      - name: Save native Cargo artifacts\n",
            "        if: ${{ !cancelled() && steps.native-cache.outputs.cache-hit != 'true' }}\n",
            "        uses: actions/cache/save@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6\n",
            "        with:\n",
        ));
        output.push_str(CARGO_CACHE_PATHS);
        output.push_str("          key: ${{ steps.native-cache.outputs.cache-primary-key }}\n");
    }
    Ok(output)
}
const CARGO_CACHE_PATHS: &str = concat!(
    "          path: |\n",
    "            ${{ env.CARGO_HOME }}/registry/index\n",
    "            ${{ env.CARGO_HOME }}/registry/cache\n",
    "            ${{ env.CARGO_HOME }}/git/db\n",
    "            ${{ env.CARGO_TARGET_DIR }}\n",
);
/// Emit only catalog-owned workflow files; unrelated workflows are untouched.
pub fn write_ci(root: &Path, write: bool) -> Result<Value> {
    let root = infra_core::fs::canonicalize(root)?;
    let catalog = load_catalog(&root, false)?;
    let mut names = BTreeSet::new();
    for group in &catalog.groups {
        let path = Path::new(&group.workflow);
        if path.components().count() != 1 || !group.workflow.ends_with(".yml") {
            return Err(RuntimeError::invalid(
                "Native catalog workflow must be one .yml filename",
            ));
        }
        names.insert(group.workflow.clone());
    }
    let mut files = infra_core::bank::publication::PublicationFiles::new(&root)?;
    let mut changed = Vec::new();
    let mut prepared = Vec::new();
    for name in names {
        let relative = format!(".github/workflows/{name}");
        let expected = workflow(&catalog, &name)?.into_bytes();
        let previous = files.read(&relative, 8 * 1024 * 1024)?;
        if previous.as_deref() != Some(expected.as_slice()) {
            changed.push(json!({"path":relative,"before_sha256":previous.as_ref().map(|bytes|common::sha256(bytes)),"after_sha256":common::sha256(&expected)}));
            prepared.push((relative, previous, expected));
        }
    }
    if write {
        for (relative, previous, expected) in prepared {
            let mode = files.read_mode(&relative)?.unwrap_or(0o644);
            files.write_expected_mode(
                &relative,
                &expected,
                previous.as_deref(),
                mode,
                8 * 1024 * 1024,
            )?;
        }
    }
    Ok(
        json!({"passed":write||changed.is_empty(),"written":write,"changed":changed,"catalog":"install/check-catalog.json"}),
    )
}

#[cfg(test)]
mod compiler_workflow_tests {
    use super::*;

    fn catalog(groups: Value) -> Catalog {
        serde_json::from_value(json!({"schema_version":1,"groups":groups})).unwrap()
    }
    fn group(name: &str, job: &str) -> Value {
        json!({
            "name":name,"workflow":"rust.yml","job":job,"timeout_minutes":15,
            "gates":[{"kind":"process","name":"native check","program":"cargo","arguments":["check"]}]
        })
    }

    #[test]
    fn workflow_selects_the_catalog_compiler_and_wasm_target_without_changing_stable_defaults() {
        let stable = group("rust", "rust-contract");
        let mut msrv = group("msrv", "rust-msrv");
        msrv["default"] = false.into();
        msrv["rust_toolchain"] = "1.89.0".into();
        msrv["gates"][0]["program"] = "rustup".into();
        msrv["gates"][0]["arguments"] = json!([
            "run",
            "{toolchain}",
            "cargo",
            "check",
            "--workspace",
            "--all-targets",
            "--locked"
        ]);
        let mut wasm = group("wasm", "rust-wasm");
        wasm["rust_targets"] = json!(["wasm32-unknown-unknown"]);
        let source = catalog(json!([stable, msrv, wasm]));
        let rendered = workflow(&source, "rust.yml").unwrap();
        let stable_job = rendered
            .split("  rust-contract:\n")
            .nth(1)
            .unwrap()
            .split("  rust-msrv:\n")
            .next()
            .unwrap();
        assert!(stable_job.contains("uses: dtolnay/rust-toolchain@stable"));
        assert!(!stable_job.contains("          toolchain:"));
        assert!(!stable_job.contains("          targets:"));
        let msrv_job = rendered
            .split("  rust-msrv:\n")
            .nth(1)
            .unwrap()
            .split("  rust-wasm:\n")
            .next()
            .unwrap();
        assert!(msrv_job.contains("          toolchain: \"1.89.0\"\n"));
        assert!(msrv_job.contains("NATIVE_CHECK_GROUP: \"msrv\""));
        let wasm_job = rendered.split("  rust-wasm:\n").nth(1).unwrap();
        assert!(wasm_job.contains("          targets: \"wasm32-unknown-unknown\"\n"));
        assert!(!wasm_job.contains("          toolchain:"));
        // The same catalog value selects the compiler for explicit local runs.
        assert_eq!(
            process_argument(
                "{toolchain}",
                Path::new("root"),
                "main",
                &source.groups[1].rust_toolchain
            ),
            "1.89.0"
        );
        assert_eq!(
            process_argument(
                "{root}/src",
                Path::new("root/{toolchain}"),
                "main",
                "1.89.0"
            ),
            "root/{toolchain}/src"
        );
    }

    #[test]
    fn invalid_compiler_or_target_metadata_is_refused_by_the_emitter() {
        for value in [
            "",
            "1.84.0",
            "1.89",
            "01.89.0",
            "1.89.0\nother",
            "stable; other",
            "nightly",
        ] {
            let mut bad = group("bad", "bad");
            bad["rust_toolchain"] = value.into();
            assert!(
                workflow(&catalog(json!([bad])), "rust.yml").is_err(),
                "{value:?}"
            );
        }
        for targets in [
            json!(["wasm32-unknown-unknown", "wasm32-unknown-unknown"]),
            json!(["wasm32-unknown-unknown\nother"]),
            json!([""]),
        ] {
            let mut bad = group("bad", "bad");
            bad["rust_targets"] = targets;
            assert!(workflow(&catalog(json!([bad])), "rust.yml").is_err());
        }
    }

    /// Context names used by `${{ }}` expressions in each job-level env block.
    fn job_env_contexts(rendered: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut inside = false;
        for line in rendered.lines() {
            if line == "    env:" {
                inside = true;
                continue;
            }
            if !line.starts_with("      ") {
                inside = false;
            }
            if !inside {
                continue;
            }
            for expression in line.split("${{").skip(1) {
                found.push(
                    expression
                        .trim_start()
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect(),
                );
            }
        }
        found
    }

    fn workflow_jobs(rendered: &str) -> Vec<String> {
        let mut jobs: Vec<String> = Vec::new();
        for line in rendered.split("\njobs:\n").nth(1).unwrap().lines() {
            if line.starts_with("  ") && !line.starts_with("   ") {
                jobs.push(String::new());
            }
            if let Some(job) = jobs.last_mut() {
                job.push_str(line);
                job.push('\n');
            }
        }
        jobs
    }

    #[test]
    fn job_level_env_uses_only_contexts_github_provides_there() {
        // GitHub rejects the whole workflow file otherwise, and no job starts.
        let allowed = [
            "github", "needs", "strategy", "matrix", "vars", "secrets", "inputs",
        ];
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let source = load_catalog(&root, false).unwrap();
        let files = source
            .groups
            .iter()
            .map(|group| group.workflow.clone())
            .collect::<BTreeSet<_>>();
        assert!(!files.is_empty());
        for name in files {
            let rendered = workflow(&source, &name).unwrap();
            for context in job_env_contexts(&rendered) {
                assert!(allowed.contains(&context.as_str()), "{name}: {context}");
            }
            let jobs = workflow_jobs(&rendered);
            assert!(!jobs.is_empty(), "{name}");
            for job in jobs {
                let compiler = job.find("id: native-toolchain").unwrap();
                let target = job
                    .find("name: Select the native target directory")
                    .unwrap_or_else(|| panic!("{name}: no target directory step"));
                let cache = job.find("name: Restore native Cargo artifacts").unwrap();
                let build = job.find("name: Build the native engine").unwrap();
                let check = job.find("name: Run the catalog group").unwrap();
                assert!(
                    compiler < target && target < cache && cache < build && build < check,
                    "{name}"
                );
                assert!(job.contains("      CARGO_PROFILE_DEV_DEBUG: \"0\"\n"));
                assert!(job.contains("      CARGO_INCREMENTAL: \"0\"\n"), "{name}");
                // A missed key falls back to the newest entry of this job and
                // compiler, never to another job's or a v1 entry.
                let prefix = job[cache..build]
                    .split("          restore-keys: ")
                    .nth(1)
                    .and_then(|rest| rest.lines().next())
                    .unwrap_or_else(|| panic!("{name}: a missed key restores nothing"));
                let job_name = job.lines().next().unwrap().trim().trim_end_matches(':');
                assert!(prefix.starts_with("native-cargo-v2-"), "{name}");
                assert!(prefix.ends_with(&format!("-{job_name}-dev0-")), "{name}");
                assert!(
                    job[cache..build].contains(&format!("          key: {prefix}${{{{ hashFiles(")),
                    "{name}"
                );
                assert!(job.contains("\"$CARGO_TARGET_DIR/debug/infra-engine$suffix\""));
                // Cargo in the group replaces the built engine; Windows refuses
                // that while the engine runs, so only a copy may run the group.
                let copy = job[check..]
                    .find("          cp \"$CARGO_TARGET_DIR/debug/infra-engine$suffix\" \"$engine\"\n")
                    .unwrap_or_else(|| panic!("{name}: the group runs the built engine"));
                let run = job[check..]
                    .find("          \"$engine\" check --group \"$NATIVE_CHECK_GROUP\"")
                    .unwrap();
                assert!(copy < run, "{name}");
                assert!(
                    job[check..]
                        .contains("engine=\"${RUNNER_TEMP//\\\\//}/infra-check-engine$suffix\"\n")
                );
                // Failed logs go where the job's last step collects them.
                assert!(
                    job[check..].contains(" --log-dir \"${RUNNER_TEMP//\\\\//}/infra-check-logs\""),
                    "{name}"
                );
                let keep = job
                    .find("      - name: Keep the complete output of failed checks\n")
                    .unwrap_or_else(|| panic!("{name}: failed logs are not kept"));
                assert!(check < keep, "{name}");
                assert!(job[keep..].contains("        if: failure()\n"), "{name}");
                assert!(
                    job[keep..].contains("          path: ${{ runner.temp }}/infra-check-logs\n"),
                    "{name}"
                );
                assert!(!job[build..check].contains("        if:"), "{name}");
                // Dependencies compiled before a failure are kept too; only
                // the save step reads whether the exact key was found.
                let save = job
                    .find("      - name: Save native Cargo artifacts\n")
                    .unwrap_or_else(|| panic!("{name}: compiled dependencies are not kept"));
                assert!(keep < save, "{name}");
                assert!(
                    job[save..].contains(
                        "        if: ${{ !cancelled() && steps.native-cache.outputs.cache-hit != 'true' }}\n"
                    ),
                    "{name}"
                );
                assert!(
                    job[save..].contains(
                        "          key: ${{ steps.native-cache.outputs.cache-primary-key }}\n"
                    ),
                    "{name}"
                );
                assert!(!job[..save].contains("cache-hit"), "{name}");
                assert_eq!(job.matches(CARGO_CACHE_PATHS).count(), 2, "{name}");
            }
        }
        assert_eq!(
            job_env_contexts("    env:\n      A: ${{ runner.temp }}/x\n    steps:\n"),
            ["runner"]
        );
    }

    #[test]
    fn native_platform_jobs_keep_their_existing_stable_compiler() {
        for (platform, runner) in [("windows", "windows-latest"), ("macos", "macos-latest")] {
            let mut native = group("native", "native");
            native["platform"] = platform.into();
            let rendered = workflow(&catalog(json!([native])), "rust.yml").unwrap();
            assert!(rendered.contains(&format!("runs-on: {runner}")));
            assert!(rendered.contains("uses: dtolnay/rust-toolchain@stable"));
            assert!(!rendered.contains("          toolchain:"));
        }
    }
}

#[cfg(test)]
mod tracked_json_syntax_tests {
    use super::*;

    fn repository() -> Temporary {
        let temporary = Temporary::new("infra-json-syntax").unwrap();
        common::git_ok(
            temporary.path(),
            &["-c", "init.templateDir=", "init", "--quiet"],
            None,
        )
        .unwrap();
        temporary
    }
    fn indexed(root: &Path, name: &str, bytes: &[u8]) {
        let path = root.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, bytes).unwrap();
        common::git_ok(
            root,
            &[
                "-c",
                "core.autocrlf=false",
                "-c",
                "core.attributesFile=",
                "add",
                "--",
                name,
            ],
            None,
        )
        .unwrap();
    }

    #[test]
    fn syntax_accepts_legacy_values_and_consumes_the_complete_input() {
        for value in [
            r#"{"key":1,"key":2}"#,
            r#"[NaN,Infinity,-Infinity,1e9999,{"value":NaN}]"#,
            r#""\uD800""#,
            r#"{"\uDC00":"\uD800"}"#,
            r#""NaN Infinity -Infinity are ordinary text""#,
            r#""escaped quote: \" and slash: \\""#,
            "42",
            "-0",
            "null",
            "true",
            r#"{"unicode":"λ"}"#,
        ] {
            assert!(validate_json_syntax(value.as_bytes()).is_ok(), "{value}");
        }
        for value in [
            "",
            "null trailing",
            "NaNprefix",
            "prefixNaN",
            "InfinityNaN",
            "+Infinity",
            "- Infinity",
            "[NaN,]",
            "{NaN:1}",
            r#""unterminated"#,
            r#""bad \q escape""#,
            r#""bad \u123 escape""#,
            r#"{"x":}"#,
            "[01]",
        ] {
            assert!(validate_json_syntax(value.as_bytes()).is_err(), "{value}");
        }
        assert!(validate_json_syntax(&[b'"', 0xff, b'"']).is_err());
    }

    #[test]
    fn indexed_members_read_worktree_bytes_and_ignore_untracked_json() {
        let temporary = repository();
        let root = temporary.path();
        indexed(root, "nested/valid file.json", b"{\"key\":1,\"key\":2}\n");
        indexed(root, "changed.json", b"{}\n");
        indexed(root, "removed.json", b"{}\n");
        indexed(root, "staged-deletion.json", b"{}\n");
        indexed(root, "invalid-utf8.json", b"{}\n");
        // Selection follows the index; syntax follows each current worktree image.
        std::fs::write(root.join("changed.json"), b"{} trailing").unwrap();
        std::fs::write(root.join("invalid-utf8.json"), [b'"', 0xff, b'"']).unwrap();
        std::fs::remove_file(root.join("removed.json")).unwrap();
        common::git_ok(
            root,
            &["rm", "--cached", "--quiet", "--", "staged-deletion.json"],
            None,
        )
        .unwrap();
        std::fs::write(root.join("staged-deletion.json"), b"invalid").unwrap();
        std::fs::write(root.join("untracked.json"), b"invalid").unwrap();
        let report = crate::execute("json-syntax", &json!({"root":root})).unwrap();
        assert_eq!(report["files"], 4);
        assert_eq!(report["passed"], false);
        assert_eq!(crate::report_exit_code(&report), 1);
        let paths = report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|finding| finding["path"].as_str().unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            paths,
            BTreeSet::from(["changed.json", "invalid-utf8.json", "removed.json"])
        );
        assert_eq!(
            std::fs::read(root.join("changed.json")).unwrap(),
            b"{} trailing"
        );
        assert_eq!(
            std::fs::read(root.join("untracked.json")).unwrap(),
            b"invalid"
        );
    }

    #[test]
    fn a_valid_indexed_repository_passes_without_reformatting() {
        let temporary = repository();
        let root = temporary.path();
        let original = br#" { "duplicate": 1, "duplicate": NaN, "text": "\uD800" } "#;
        indexed(root, "valid.json", original);
        std::fs::write(root.join("ignored.json"), b"not JSON").unwrap();
        let report = check_tracked_json(root).unwrap();
        assert_eq!(report["files"], 1);
        assert_eq!(report["passed"], true);
        assert_eq!(crate::report_exit_code(&report), 0);
        assert_eq!(std::fs::read(root.join("valid.json")).unwrap(), original);
    }

    #[cfg(unix)]
    #[test]
    fn nul_delimited_membership_keeps_newlines_inside_filenames() {
        let temporary = repository();
        let root = temporary.path();
        let name = "line\nbreak.json";
        indexed(root, name, b"{}\n");
        std::fs::write(root.join(name), b"[").unwrap();
        let report = check_tracked_json(root).unwrap();
        assert_eq!(report["files"], 1);
        assert_eq!(report["findings"][0]["path"], name);
        assert_eq!(report["passed"], false);
    }
}

#[cfg(test)]
mod failure_report_tests {
    use super::*;

    fn outcome(status: &str, excerpt: Vec<(&'static str, String)>) -> Outcome {
        Outcome {
            group: "rust".into(),
            name: "Workspace Rust tests".into(),
            status: status.into(),
            seconds: 1.0,
            code: Some(101),
            reason: None,
            log: None,
            excerpt,
        }
    }

    #[test]
    fn excerpts_keep_the_last_lines_of_each_nonempty_stream() {
        let long = (1..=EXCERPT_LINES + 5)
            .map(|line| format!("line {line}\n"))
            .collect::<String>();
        let kept = excerpt(&[
            ("stdout", long.as_bytes()),
            ("stderr", b""),
            ("report", b"no final newline"),
        ]);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].0, "stdout");
        let lines = kept[0].1.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), EXCERPT_LINES);
        assert_eq!(lines[0], "line 6");
        assert_eq!(
            lines[EXCERPT_LINES - 1],
            format!("line {}", EXCERPT_LINES + 5)
        );
        assert_eq!(kept[1], ("report", "no final newline".to_string()));
    }

    #[test]
    fn excerpts_keep_the_last_bytes_of_an_overlong_line() {
        // Two-byte characters put the byte bound inside one of them.
        let mut bytes = "é".repeat(EXCERPT_BYTES).into_bytes();
        bytes.extend_from_slice(b"\nend\n");
        let kept = excerpt(&[("stderr", bytes.as_slice())]);
        let text = &kept[0].1;
        assert!(text.starts_with('\u{FFFD}'), "{}", &text[..8]);
        assert!(text.ends_with("é\nend"));
        // The cut continuation byte becomes one three-byte replacement.
        assert_eq!(text.len(), EXCERPT_BYTES + 1);
    }

    #[test]
    fn the_report_names_only_failed_checks_with_their_streams() {
        assert_eq!(failure_report(&[outcome("pass", vec![])]), "");
        let mut absent = outcome("fail", vec![]);
        absent.name = "PHP snippets".into();
        absent.code = Some(1);
        absent.reason = Some("Required executable is unavailable: php".into());
        let report = failure_report(&[
            outcome("pass", vec![]),
            outcome(
                "fail",
                vec![
                    ("stdout", "test locks ... FAILED".into()),
                    ("stderr", "error: test failed".into()),
                ],
            ),
            absent,
            outcome("skipped", vec![]),
        ]);
        assert_eq!(
            report,
            concat!(
                "FAILED rust / Workspace Rust tests (exit 101)\n",
                "--- end of stdout ---\ntest locks ... FAILED\n",
                "--- end of stderr ---\nerror: test failed\n",
                "FAILED rust / PHP snippets (exit 1): Required executable is unavailable: php\n",
            )
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_process_check_reports_the_end_of_both_streams() {
        let temporary = Temporary::new("infra-check-report").unwrap();
        let group: Group = serde_json::from_value(json!({
            "name":"probe","workflow":"ci.yml","job":"probe","timeout_minutes":1,
            "gates":[{"kind":"process","name":"failing probe","program":"sh",
                "arguments":["-c","echo first; echo last; echo cause >&2; exit 3"]}]
        }))
        .unwrap();
        let outcome = gate(
            GateContext {
                root: temporary.path(),
                base: "origin/main",
                group: &group,
                strict: true,
                oracle_allowed: false,
                logs: temporary.path(),
                cancel: Arc::new(AtomicBool::new(false)),
            },
            &group.gates[0],
        );
        assert_eq!((outcome.status.as_str(), outcome.code), ("fail", Some(3)));
        assert_eq!(
            outcome.excerpt,
            [
                ("stdout", "first\nlast".to_string()),
                ("stderr", "cause".to_string())
            ]
        );
        // The retained log still holds the complete output.
        assert_eq!(
            std::fs::read(outcome.log.as_ref().unwrap()).unwrap(),
            b"first\nlast\ncause\n"
        );
        assert_eq!(
            failure_report(&[outcome]),
            "FAILED probe / failing probe (exit 3)\n--- end of stdout ---\nfirst\nlast\n--- end of stderr ---\ncause\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_named_log_directory_keeps_failed_logs_for_the_job_to_collect() {
        let temporary = Temporary::new("infra-check-named-logs").unwrap();
        let root = temporary.path();
        std::fs::create_dir_all(root.join(".github/workflows")).unwrap();
        std::fs::write(root.join(".github/workflows/probe.yml"), b"name: probe\n").unwrap();
        std::fs::create_dir_all(root.join("install")).unwrap();
        std::fs::write(
            root.join("install/check-catalog.json"),
            serde_json::to_vec(&json!({"schema_version":1,"groups":[{
                "name":"probe","workflow":"probe.yml","job":"probe","timeout_minutes":1,
                "gates":[{"kind":"process","name":"failing probe","program":"sh",
                    "arguments":["-c","echo kept; exit 3"]}]
            }]}))
            .unwrap(),
        )
        .unwrap();
        let logs = root.join("named logs");
        let value = execute(&json!({
            "root":root,"groups":["probe"],"strict":true,"log_dir":logs
        }))
        .unwrap();
        assert_eq!(value["result"]["exit_code"], 1);
        assert_eq!(value["result"]["logs"], json!(logs));
        let kept = std::fs::read_dir(&logs)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert_eq!(std::fs::read(&kept[0]).unwrap(), b"kept\n");
        assert_eq!(
            value["_cli"]["stderr"],
            "FAILED probe / failing probe (exit 3)\n--- end of stdout ---\nkept\n"
        );
    }
}
