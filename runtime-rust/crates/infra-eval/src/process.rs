use crate::{Result, RuntimeError, artifact};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
pub const USAGE_FIELDS: [&str; 5] = [
    "input_tokens",
    "output_tokens",
    "cached_input_tokens",
    "cache_write_input_tokens",
    "reasoning_output_tokens",
];
pub fn token(v: &Value) -> Option<u64> {
    v.as_u64().filter(|n| *n <= 9_007_199_254_740_991)
}
#[derive(Clone, Debug)]
pub struct CommandSpec {
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub input: Vec<u8>,
}
#[derive(Clone, Debug)]
pub struct Budget {
    pub timeout: Duration,
    pub cap: usize,
    pub max_bytes: usize,
    pub native: bool,
    pub strict_events: bool,
    pub error_diagnostics_terminal: bool,
}
impl Budget {
    pub fn native(timeout: Duration, cap: usize) -> Self {
        Self {
            timeout,
            cap,
            max_bytes: 8 * 1024 * 1024,
            native: true,
            strict_events: true,
            error_diagnostics_terminal: true,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Session {
    pub stopped: String,
    pub completed: bool,
    pub calls: usize,
    pub exit: Option<i32>,
    pub seconds: f64,
    pub known_usage: BTreeMap<String, u64>,
    pub usage_field_turns: BTreeMap<String, usize>,
    pub usage_events: Vec<Value>,
    pub usage_complete: bool,
    pub sessions: Vec<String>,
    pub turns: usize,
    pub answer: String,
    pub messages: Vec<String>,
    pub message_overflow: bool,
    pub events: Vec<Value>,
    seen: BTreeSet<String>,
    turn_finished: bool,
    failed: bool,
}
impl Default for Session {
    fn default() -> Self {
        Self {
            stopped: "finished".into(),
            completed: false,
            calls: 0,
            exit: None,
            seconds: 0.0,
            known_usage: BTreeMap::new(),
            usage_field_turns: BTreeMap::new(),
            usage_events: vec![],
            usage_complete: true,
            sessions: vec![],
            turns: 0,
            answer: String::new(),
            messages: vec![],
            message_overflow: false,
            events: vec![],
            seen: BTreeSet::new(),
            turn_finished: false,
            failed: false,
        }
    }
}
impl Session {
    pub fn feed(
        &mut self,
        event: &Value,
        strict: bool,
        terminal_diagnostics: bool,
    ) -> Option<String> {
        let kind = event["type"].as_str().unwrap_or("");
        let item = &event["item"];
        match kind {
            "thread.started" => {
                if let Some(s) = event["thread_id"].as_str()
                    && !self.sessions.iter().any(|p| p == s)
                {
                    self.sessions.push(s.into());
                }
            }
            "turn.started" => {
                self.turn_finished = false;
                self.answer.clear();
                self.seen.clear();
            }
            "turn.failed" => {
                self.failed = true;
                return Some("native-failed".into());
            }
            "error" if terminal_diagnostics => {
                self.failed = true;
                return Some("native-failed".into());
            }
            "item.started" | "item.completed" => {
                if !item.is_object() || strict && self.turn_finished {
                    return Some("invalid-events".into());
                }
                if [
                    "command_execution",
                    "file_change",
                    "mcp_tool_call",
                    "web_search",
                    "collab_tool_call",
                ]
                .contains(&item["type"].as_str().unwrap_or(""))
                {
                    if let Some(id) = item["id"].as_str().filter(|s| !s.is_empty()) {
                        if self.seen.insert(id.into()) {
                            self.calls += 1;
                        }
                    } else if strict {
                        return Some("invalid-events".into());
                    }
                }
                if kind == "item.completed"
                    && item["type"] == "agent_message"
                    && let Some(text) = item["text"].as_str()
                {
                    if text.len() > 262144 {
                        self.message_overflow = true;
                    } else if self.messages.len() < 64
                        && self.messages.iter().map(String::len).sum::<usize>() + text.len()
                            <= 1048576
                    {
                        self.messages.push(text.into());
                    } else {
                        self.message_overflow = true;
                    }
                    self.answer = text.into();
                }
            }
            "turn.completed" => {
                if strict && self.turn_finished {
                    return Some("invalid-events".into());
                }
                self.turn_finished = true;
                self.turns += 1;
                let mut safe = serde_json::Map::new();
                for key in USAGE_FIELDS {
                    if let Some(n) = token(&event["usage"][key]) {
                        safe.insert(key.into(), n.into());
                    }
                }
                for (part, whole) in [
                    ("cached_input_tokens", "input_tokens"),
                    ("cache_write_input_tokens", "input_tokens"),
                    ("reasoning_output_tokens", "output_tokens"),
                ] {
                    if safe
                        .get(part)
                        .and_then(token)
                        .is_some_and(|a| safe.get(whole).and_then(token).is_none_or(|b| a > b))
                    {
                        safe.remove(part);
                        self.usage_complete = false;
                    }
                }
                for key in USAGE_FIELDS {
                    if let Some(n) = safe.get(key).and_then(token) {
                        *self.known_usage.entry(key.into()).or_default() += n;
                        *self.usage_field_turns.entry(key.into()).or_default() += 1;
                    }
                }
                if !safe.contains_key("input_tokens") || !safe.contains_key("output_tokens") {
                    self.usage_complete = false;
                }
                self.usage_events.push(Value::Object(safe));
            }
            _ => {}
        }
        if strict && (self.turns > 16 || self.sessions.len() > 2) {
            Some("output-limit".into())
        } else {
            None
        }
    }
    pub fn value(&self) -> Value {
        let usage = if self.completed && self.usage_complete {
            json!(
                self.known_usage
                    .iter()
                    .filter(|(key, _)| self.usage_field_turns.get(*key) == Some(&self.turns))
                    .map(|(key, value)| (key.clone(), *value))
                    .collect::<BTreeMap<_, _>>()
            )
        } else {
            Value::Null
        };
        json!({"stopped":self.stopped,"completed":self.completed,"calls":self.calls,"exit":self.exit,"seconds":self.seconds,"known_usage":self.known_usage,"usage":usage,"usage_complete":self.completed&&self.usage_complete,"usage_events":self.usage_events,"usage_coverage":{"completed_turns":self.turns,"field_turns":self.usage_field_turns},"sessions":self.sessions,"turns":self.turns,"answer":self.answer})
    }
}
struct CustodyState {
    lost: std::sync::atomic::AtomicBool,
    first: std::sync::Mutex<Option<RuntimeError>>,
}
impl CustodyState {
    const fn new() -> Self {
        Self {
            lost: std::sync::atomic::AtomicBool::new(false),
            first: std::sync::Mutex::new(None),
        }
    }
    fn lost(&self) -> bool {
        self.lost.load(std::sync::atomic::Ordering::Acquire)
    }
    fn record(&self, error: &RuntimeError) {
        if error.code.as_deref() != Some("cleanup-unconfirmed") {
            return;
        }
        let mut first = self
            .first
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if first.is_none() {
            *first = Some(copy_error(error));
        }
        self.lost.store(true, std::sync::atomic::Ordering::Release);
    }
    fn error(&self) -> RuntimeError {
        let first = self
            .first
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        first.as_ref().map(copy_error).unwrap_or_else(|| {
            RuntimeError::coded(
                infra_core::ErrorKind::Unavailable,
                "cleanup-unconfirmed",
                "Native Eval cleanup could not be confirmed; retained work requires inspection.",
            )
        })
    }
}
fn copy_error(error: &RuntimeError) -> RuntimeError {
    let mut copy = RuntimeError::new(error.kind, error.message.clone());
    copy.code = error.code.clone();
    copy.os_code = error.os_code;
    copy
}
static CUSTODY: CustodyState = CustodyState::new();
pub(crate) fn custody_lost() -> bool {
    CUSTODY.lost()
}
pub(crate) fn custody_error() -> RuntimeError {
    CUSTODY.error()
}
pub(crate) fn record_custody_error(error: &RuntimeError) {
    CUSTODY.record(error);
}
pub(crate) fn ensure_custody() -> Result<()> {
    if custody_lost() {
        Err(custody_error())
    } else {
        Ok(())
    }
}
fn capture_failure(error: infra_host::HostError, state: &CustodyState) -> RuntimeError {
    let converted = RuntimeError::coded(
        infra_core::ErrorKind::Unavailable,
        error.code,
        error.message,
    );
    state.record(&converted);
    converted
}
pub fn execute(
    spec: &CommandSpec,
    budget: &Budget,
    events_path: Option<&Path>,
    stderr_path: Option<&Path>,
    mut observe: impl FnMut(&Value, &Session) -> Option<String> + Send,
) -> Result<Session> {
    ensure_custody()?;
    if spec.argv.is_empty()
        || budget.timeout.is_zero()
        || budget.max_bytes == 0
        || budget.max_bytes > 8 * 1024 * 1024
        || budget.cap == 0
    {
        return Err(artifact::invalid("invalid process budget"));
    }
    let started = Instant::now();
    let deadline = started
        .checked_add(budget.timeout)
        .ok_or_else(|| artifact::invalid("invalid process deadline"))?;
    let mut log = events_path
        .map(|path| {
            artifact::fresh_file(path, &[], 0o600)?;
            std::fs::OpenOptions::new()
                .append(true)
                .open(path)
                .map_err(RuntimeError::from)
        })
        .transpose()?;
    let mut errors = stderr_path
        .map(|path| {
            artifact::fresh_file(path, &[], 0o600)?;
            std::fs::OpenOptions::new()
                .append(true)
                .open(path)
                .map_err(RuntimeError::from)
        })
        .transpose()?;
    let mut result = Session::default();
    let mut pending = Vec::new();
    let mut callback_error = None;
    let mut observed_native = None;
    let output = infra_host::capture::run(
        infra_host::capture::Request {
            argv: &spec.argv,
            cwd: &spec.cwd,
            environment: &spec.env,
            input: &spec.input,
            output_limit: budget.max_bytes,
            deadline,
        },
        crate::cancel::interrupted,
        |native| observed_native = Some(native),
        |stream, block| {
            let target = match stream {
                infra_host::capture::Stream::Stdout => log.as_mut(),
                infra_host::capture::Stream::Stderr => errors.as_mut(),
            };
            if let Some(file) = target
                && let Err(error) = file.write_all(block)
            {
                callback_error = Some(RuntimeError::from(error));
                return Err(infra_host::HostError::unavailable(
                    "Eval output log write failed.",
                ));
            }
            if stream == infra_host::capture::Stream::Stderr || !budget.native {
                return Ok(());
            }
            pending.extend_from_slice(block);
            if pending.len() > 1024 * 1024 {
                result.stopped = "output-limit".into();
            } else {
                while let Some(at) = pending.iter().position(|byte| *byte == b'\n') {
                    let line = pending.drain(..=at).collect::<Vec<_>>();
                    let event = match artifact::strict(&line) {
                        Ok(event) if event.is_object() => event,
                        _ => {
                            if budget.strict_events {
                                result.stopped = "invalid-events".into();
                                break;
                            }
                            continue;
                        }
                    };
                    let verdict = result.feed(
                        &event,
                        budget.strict_events,
                        budget.error_diagnostics_terminal,
                    );
                    let observed = observe(&event, &result);
                    result.events.push(event);
                    if let Some(reason) = verdict.or(observed) {
                        result.stopped = reason;
                        break;
                    }
                    if result.calls >= budget.cap {
                        result.stopped = "cap".into();
                        break;
                    }
                }
            }
            if result.stopped != "finished" {
                return Err(infra_host::HostError {
                    status: 409,
                    code: "eval-stop",
                    message: "The bounded Eval observer stopped this run.".into(),
                });
            }
            Ok(())
        },
    );
    if let Some(native) = observed_native {
        result.exit = native.code.or_else(|| native.signal.map(|signal| -signal));
    }
    if output
        .as_ref()
        .err()
        .is_some_and(|error| error.code == "cleanup-unconfirmed")
    {
        return Err(capture_failure(
            output.err().expect("observed fatal capture error"),
            &CUSTODY,
        ));
    }
    if let Some(error) = callback_error {
        return Err(error);
    }
    match output {
        Ok(output) => {
            result.exit = output.exit.or_else(|| output.signal.map(|signal| -signal));
        }
        Err(error) => match error.code {
            "eval-stop" => {}
            "native-timeout" => {
                if result.stopped == "finished" {
                    result.stopped = "timeout".into();
                }
            }
            "native-cancelled" => {
                if result.stopped == "finished" {
                    result.stopped = "interrupted".into();
                }
            }
            "output-limit" => {
                result.stopped = "output-limit".into();
            }
            _ => return Err(capture_failure(error, &CUSTODY)),
        },
    }
    if !pending.is_empty() && budget.native && budget.strict_events && result.stopped == "finished"
    {
        result.stopped = "invalid-events".into();
    }
    result.completed = result.stopped == "finished"
        && result.exit == Some(0)
        && (!budget.native || result.turn_finished)
        && !result.failed;
    result.usage_complete &= result.completed;
    result.seconds = (started.elapsed().as_secs_f64() * 1000.0).round() / 1000.0;
    Ok(result)
}
pub fn capture(spec: &CommandSpec, timeout: Duration, limit: usize) -> Result<(i32, Vec<u8>)> {
    let scratch = artifact::Scratch::new(&std::env::temp_dir(), "eval-process")?;
    let output = scratch.0.join("output");
    let b = Budget {
        timeout,
        cap: usize::MAX,
        max_bytes: limit,
        native: false,
        strict_events: false,
        error_diagnostics_terminal: false,
    };
    let r = execute(spec, &b, Some(&output), None, |_, _| None)?;
    Ok((
        if r.stopped == "finished" {
            r.exit.unwrap_or(-1)
        } else {
            -1
        },
        artifact::read_file(&output, limit)?.0,
    ))
}

pub fn experiment(spec: &CommandSpec, stop: infra_host::capture::ExperimentStop) -> Result<i32> {
    ensure_custody()?;
    if spec.argv.is_empty() || stop.grace.is_zero() {
        return Err(artifact::invalid("Invalid lifecycle experiment budget"));
    }
    // Phase timers start after validated native admission. The hard transport
    // deadline separately permits five seconds of startup overhead.
    let total = stop
        .after
        .checked_add(stop.grace)
        .and_then(|time| time.checked_add(Duration::from_secs(5)))
        .ok_or_else(|| artifact::invalid("Invalid lifecycle experiment deadline"))?;
    let deadline = Instant::now()
        .checked_add(total)
        .ok_or_else(|| artifact::invalid("Invalid lifecycle experiment deadline"))?;
    let output = infra_host::capture::run_experiment(
        infra_host::capture::Request {
            argv: &spec.argv,
            cwd: &spec.cwd,
            environment: &spec.env,
            input: &spec.input,
            output_limit: 8 * 1024 * 1024,
            deadline,
        },
        stop,
        crate::cancel::interrupted,
        |_| {},
        |_, _| Ok(()),
    )
    .map_err(|error| capture_failure(error, &CUSTODY))?;
    output
        .exit
        .or_else(|| output.signal.map(|signal| -signal))
        .ok_or_else(|| artifact::invalid("Lifecycle experiment has no observed native status"))
}
