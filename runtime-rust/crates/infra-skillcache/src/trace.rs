//! The observed trace `τ_obs` of one cold run.
//!
//! A trace is the ordered list of what the agent did and what came back: each
//! tool call with its arguments paired with its result, errors included, and
//! the agent's messages. It never holds provider reasoning. Every text passes
//! a redactor before it is kept, each observation is bounded, and so is the
//! whole trace; what was cut or replaced is counted, not hidden.
use regex::Regex;
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, sync::OnceLock};

pub const TRACE_SCHEMA: u64 = 1;
/// One result of one tool call.
pub const OBSERVATION_BYTES: usize = 64 * 1024;
/// The arguments of one tool call.
pub const INPUT_BYTES: usize = 16 * 1024;
pub const MESSAGE_BYTES: usize = 16 * 1024;
/// The whole trace: beyond it, results are left out and the trace says so.
pub const TRACE_BYTES: usize = 8 * 1024 * 1024;
pub const STEPS: usize = 4000;

/// Removes what must not be stored from one text and says how many places it
/// changed. The Harness supplies the session's redactor; it knows the launch's
/// secret environment values and the host paths to strip.
pub trait Redactor: Send + Sync {
    fn redact(&self, text: &str) -> (String, usize);
}

/// The shared secret policy alone.
pub struct PolicyRedactor;
impl Redactor for PolicyRedactor {
    fn redact(&self, text: &str) -> (String, usize) {
        match infra_core::domain::mask_secrets(text) {
            Ok(masked) => masked,
            Err(_) => (
                "[redacted: a possible secret could not be masked]".into(),
                1,
            ),
        }
    }
}

/// A skill the run used, as the trace shows it.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct SkillRef {
    /// The name given to the provider's skill tool, when that is how it was used.
    pub name: Option<String>,
    /// The path of a `SKILL.md` the run read, as the provider spelled it.
    pub path: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Counters {
    /// Places a redactor changed.
    pub redacted: usize,
    /// Observations or arguments cut to their limit.
    pub truncated: usize,
    /// Reasoning blocks that were dropped unread.
    pub reasoning_dropped: usize,
    /// Results left out because the whole trace reached its limit.
    pub omitted: usize,
}

#[derive(Clone, Debug)]
pub struct Trace {
    pub provider: String,
    pub steps: Vec<Value>,
    pub skills: Vec<SkillRef>,
    pub counters: Counters,
    /// Whether every started tool call was seen to finish.
    pub complete: bool,
}

impl Trace {
    /// What a compiler reads as `tau_obs.json`.
    pub fn to_value(&self) -> Value {
        json!({
            "schema_version": TRACE_SCHEMA,
            "provider": self.provider,
            "steps": self.steps,
            "redaction": self.redaction(),
        })
    }
    pub fn redaction(&self) -> Value {
        json!({
            "redacted": self.counters.redacted,
            "truncated": self.counters.truncated,
            "reasoning_dropped": self.counters.reasoning_dropped,
            "omitted": self.counters.omitted,
        })
    }
    /// Contents of the files the run read whole, as far as the trace shows:
    /// what a file-reading tool or a plain `cat` returned. A binding equal to
    /// one of them is file content, not a parameter.
    pub fn observed_files(&self) -> Vec<String> {
        self.steps
            .iter()
            .filter(|step| step["kind"] == "tool" && step["error"] != true)
            .filter_map(|step| {
                let output = step["output"].as_str()?;
                let tool = step["tool"].as_str().unwrap_or("");
                let command = step["input"]["command"].as_str().unwrap_or("");
                if ["Read", "read", "readFile", "read_file"].contains(&tool) {
                    Some(unnumbered(output))
                } else if reads_one_file(command) {
                    Some(output.to_string())
                } else {
                    None
                }
            })
            .collect()
    }
}

/// A text's first `head` and last `tail` bytes, with a marker between them.
fn shortened(text: &str, head: usize, tail: usize) -> String {
    if text.len() <= head + tail {
        return text.to_string();
    }
    let mut first = head;
    while !text.is_char_boundary(first) {
        first -= 1;
    }
    let mut last = text.len() - tail;
    while !text.is_char_boundary(last) {
        last += 1;
    }
    format!(
        "{}\n[... {} bytes not shown ...]\n{}",
        &text[..first],
        last - first,
        &text[last..]
    )
}
fn shorten_strings(value: &Value, limit: usize) -> Value {
    match value {
        Value::String(text) => Value::String(shortened(text, limit * 2 / 3, limit / 3)),
        Value::Array(items) => items
            .iter()
            .map(|item| shorten_strings(item, limit))
            .collect(),
        Value::Object(members) => members
            .iter()
            .map(|(key, member)| (key.clone(), shorten_strings(member, limit)))
            .collect::<Map<_, _>>()
            .into(),
        other => other.clone(),
    }
}

/// What a compiler reads of a stored trace. A model cannot take megabytes of
/// tool output, and a recipe does not need them: every result keeps its
/// beginning and its end, and results shrink further until the whole view
/// fits `budget` bytes. The order and the pairing of the steps stay as stored.
pub fn compiler_view(steps: &[Value], budget: usize) -> Value {
    let mut limit = 6 * 1024;
    loop {
        let view = steps
            .iter()
            .map(|step| {
                let mut step = step.clone();
                if let Some(output) = step["output"].as_str() {
                    step["output"] = shortened(output, limit * 2 / 3, limit / 3).into();
                }
                if let Some(text) = step["text"].as_str() {
                    step["text"] = shortened(text, 3000, 1000).into();
                }
                if step["input"].is_object() || step["input"].is_array() {
                    step["input"] = shorten_strings(&step["input"], limit.min(2 * 1024));
                }
                step
            })
            .collect::<Vec<_>>();
        let size = serde_json::to_vec(&view).map_or(usize::MAX, |bytes| bytes.len());
        if size <= budget || limit <= 256 {
            return Value::Array(view);
        }
        limit /= 2;
    }
}

/// A file reader's line-number gutter (`   12→text` or `   12\ttext`) removed.
fn unnumbered(text: &str) -> String {
    static GUTTER: OnceLock<Regex> = OnceLock::new();
    let gutter = GUTTER.get_or_init(|| Regex::new(r"^\s*\d+(?:→|\t)").expect("a fixed expression"));
    if text.lines().take(3).all(|line| gutter.is_match(line)) {
        text.lines()
            .map(|line| gutter.replace(line, "").into_owned())
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        text.to_string()
    }
}

pub fn unwrap_shell_command(command: &str) -> &str {
    static SHELL: OnceLock<Regex> = OnceLock::new();
    let trimmed = command.trim();
    if let Some(captures) = SHELL
        .get_or_init(|| {
            Regex::new(r#"^(?:(?:/usr(?:/local)?)?/bin/)?(?:bash|sh|zsh|dash)\s+-(?:lc|c)\s+(.+)$"#)
                .expect("a fixed expression")
        })
        .captures(trimmed)
        && let Some(inner) = captures.get(1)
    {
        let body = inner.as_str().trim();
        if body.len() >= 2
            && ((body.starts_with('\'') && body.ends_with('\''))
                || (body.starts_with('"') && body.ends_with('"')))
        {
            return body[1..body.len() - 1].trim();
        }
        return body;
    }
    trimmed
}

/// Whether `name` is a provider shell / terminal execution tool whose
/// read-only safety depends on `input["command"]`.
pub fn is_shell_tool_name(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "bash"
            | "shell"
            | "shelltoolcall"
            | "command_execution"
            | "run_terminal"
            | "run_terminal_cmd"
            | "run_command"
            | "exec_command"
    )
}

/// Extracts the shell command string from a recorded tool step, if present.
pub fn step_shell_command(step: &Value) -> Option<&str> {
    step["input"]["command"]
        .as_str()
        .or_else(|| step["input"]["cmd"].as_str())
        .map(str::trim)
        .filter(|cmd| !cmd.is_empty())
}

/// Unwraps outer shell wrappers (`bash -lc`, `sh -c`, etc.) and splits
/// deterministic `&&` / `;` / `|` chains outside quotes. Returns `None` for
/// redirects, command substitution, subshells, backgrounding, or unclosed quotes.
pub fn split_read_only_shell_chain(command: &str) -> Option<Vec<String>> {
    let mut body = command.trim();
    for _ in 0..2 {
        let unwrapped = unwrap_shell_command(body);
        if unwrapped == body {
            break;
        }
        body = unwrapped;
    }
    if body.is_empty() {
        return None;
    }
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut chars = body.chars().peekable();
    while let Some(ch) = chars.next() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        match quote {
            Some('\'') => {
                current.push(ch);
                if ch == '\'' {
                    quote = None;
                }
            }
            Some('"') => {
                if ch == '`' || ch == '\0' {
                    return None;
                }
                if ch == '$' && chars.peek() == Some(&'(') {
                    return None;
                }
                current.push(ch);
                if ch == '\\' {
                    let next = chars.next()?;
                    current.push(next);
                } else if ch == '"' {
                    quote = None;
                }
            }
            Some(_) => unreachable!(),
            None => match ch {
                '`' | '>' | '<' | '(' | ')' | '\n' | '\r' | '\0' => return None,
                '$' if chars.peek() == Some(&'(') => return None,
                '\\' => {
                    current.push(ch);
                    escaped = true;
                }
                '\'' | '"' => {
                    current.push(ch);
                    quote = Some(ch);
                }
                '&' => {
                    if chars.peek() == Some(&'&') {
                        chars.next();
                        let seg = current.trim();
                        if seg.is_empty() {
                            return None;
                        }
                        segments.push(seg.to_owned());
                        current.clear();
                    } else {
                        return None;
                    }
                }
                '|' => {
                    if chars.peek() == Some(&'|') {
                        return None;
                    }
                    let seg = current.trim();
                    if seg.is_empty() {
                        return None;
                    }
                    segments.push(seg.to_owned());
                    current.clear();
                }
                ';' => {
                    let seg = current.trim();
                    if seg.is_empty() {
                        return None;
                    }
                    segments.push(seg.to_owned());
                    current.clear();
                }
                _ => current.push(ch),
            },
        }
    }
    if quote.is_some() || escaped {
        return None;
    }
    let tail = current.trim();
    if tail.is_empty() {
        return None;
    }
    segments.push(tail.to_owned());
    Some(segments)
}

fn reads_python_inline(body: &str) -> bool {
    static PY: OnceLock<Regex> = OnceLock::new();
    let Some(found) = PY
        .get_or_init(|| {
            Regex::new(r#"^(?:(?:/usr(?:/local)?)?/bin/)?python3?\s+-c\s+"#)
                .expect("a fixed expression")
        })
        .find(body)
    else {
        return false;
    };
    let rest = body[found.end()..].trim();
    let code = if rest.len() >= 2
        && ((rest.starts_with('\'') && rest.ends_with('\''))
            || (rest.starts_with('"') && rest.ends_with('"')))
    {
        &rest[1..rest.len() - 1]
    } else {
        return false;
    };
    let reads =
        code.contains("open(") || code.contains(".read_text(") || code.contains(".read_bytes(");
    let writes = code.contains(".write_text(")
        || code.contains(".write_bytes(")
        || code.contains("'w'")
        || code.contains("\"w\"");
    reads && !writes
}

fn reads_one_file(command: &str) -> bool {
    let body = unwrap_shell_command(command);
    if reads_python_inline(body) {
        return true;
    }
    if body.is_empty() || body.contains(['|', ';', '&', '<', '>', '\n']) {
        return false;
    }
    let parts: Vec<&str> = body.split_whitespace().collect();
    let Some(cmd) = parts.first().copied() else {
        return false;
    };
    let cmd = cmd.rsplit('/').next().unwrap_or(cmd);
    let args = &parts[1..];
    match cmd {
        "cat" | "type" | "Get-Content" => !args.is_empty(),
        "head" | "tail" => {
            !args.iter().any(|arg| *arg == "-f" || *arg == "--follow")
                && args.iter().any(|arg| !arg.starts_with('-'))
        }
        "sed" => {
            let mutating = args.iter().any(|arg| {
                *arg == "-i"
                    || *arg == "--in-place"
                    || arg.starts_with("--in-place=")
                    || (arg.starts_with('-') && !arg.starts_with("--") && arg[1..].contains('i'))
            });
            !mutating && args.iter().filter(|arg| !arg.starts_with('-')).count() >= 2
        }
        "jq" => args.iter().filter(|arg| !arg.starts_with('-')).count() >= 2,
        _ => false,
    }
}

fn skill_paths(text: &str) -> Vec<String> {
    static DOUBLE_QUOTED: OnceLock<Regex> = OnceLock::new();
    static SINGLE_QUOTED: OnceLock<Regex> = OnceLock::new();
    static PATH: OnceLock<Regex> = OnceLock::new();
    let double_quoted = DOUBLE_QUOTED.get_or_init(|| {
        Regex::new(
            r#"(?:^|[\s=:(,])"((?:/|~[/\\]|\.{1,2}[/\\]|\.[A-Za-z0-9_-]+[/\\]|[A-Za-z]:[/\\])(?:[^"\n\r]*[/\\])?SKILL\.md)""#,
        )
        .expect("a fixed expression")
    });
    let single_quoted = SINGLE_QUOTED.get_or_init(|| {
        Regex::new(
            r#"(?:^|[\s=:(,])'((?:/|~[/\\]|\.{1,2}[/\\]|\.[A-Za-z0-9_-]+[/\\]|[A-Za-z]:[/\\])(?:[^'\n\r]*[/\\])?SKILL\.md)'"#,
        )
        .expect("a fixed expression")
    });
    let plain = PATH.get_or_init(|| {
        Regex::new(r#"[A-Za-z0-9_.~:/\\-]*[/\\]SKILL\.md"#).expect("a fixed expression")
    });
    let mut spans = Vec::new();
    let mut paths = Vec::new();
    for regex in [double_quoted, single_quoted] {
        for captures in regex.captures_iter(text) {
            if let Some(inner) = captures.get(1) {
                spans.push(inner.start()..inner.end());
                let path = inner.as_str().replace('\\', "/");
                if !paths.contains(&path) {
                    paths.push(path);
                }
            }
        }
    }
    for found in plain.find_iter(text) {
        if spans
            .iter()
            .any(|span| found.start() >= span.start && found.end() <= span.end)
        {
            continue;
        }
        let path = found.as_str().replace('\\', "/");
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}

fn collect_value_skills(value: &Value, found: &mut Vec<SkillRef>) {
    match value {
        Value::String(text) => {
            found.extend(skill_paths(text).into_iter().map(|path| SkillRef {
                name: None,
                path: Some(path),
            }));
        }
        Value::Array(items) => {
            for item in items {
                collect_value_skills(item, found);
            }
        }
        Value::Object(object) => {
            for (key, value) in object {
                if ["file_path", "path", "target_file"].contains(&key.as_str())
                    && let Some(raw) = value.as_str()
                {
                    let path = raw.replace('\\', "/");
                    if path == "SKILL.md" || path.ends_with("/SKILL.md") {
                        found.push(SkillRef {
                            name: None,
                            path: Some(path),
                        });
                        continue;
                    }
                }
                collect_value_skills(value, found);
            }
        }
        _ => {}
    }
}

/// How far past its limit a text is kept until it is redacted, so that a
/// secret lying across the limit is still seen whole by the redactor.
const SLACK: usize = 4 * 1024;

/// A text as it arrived, cut to its limit plus [`SLACK`], and its full size.
#[derive(Clone)]
struct Held {
    text: String,
    bytes: usize,
}
fn hold(text: &str, limit: usize) -> Held {
    let mut end = text.len().min(limit + SLACK);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Held {
        text: text[..end].to_string(),
        bytes: text.len(),
    }
}
/// The arguments of a call with every long string cut the same way.
fn hold_input(value: &Value, limit: usize) -> Value {
    match value {
        Value::String(text) => Value::String(hold(text, limit).text),
        Value::Array(items) => items.iter().map(|item| hold_input(item, limit)).collect(),
        Value::Object(members) => members
            .iter()
            .map(|(key, member)| (key.clone(), hold_input(member, limit)))
            .collect::<Map<_, _>>()
            .into(),
        other => other.clone(),
    }
}

struct RawStep {
    tool: Option<String>,
    input: Value,
    /// Per-string limit of `input`.
    input_limit: usize,
    text: Option<Held>,
    output: Option<Held>,
    omitted: bool,
    error: Option<bool>,
    exit_code: Option<i64>,
    agent: Option<String>,
}

/// Pairs a provider's events into steps as they arrive. Observing is cheap:
/// texts are only cut to size. Redaction happens once, in [`Recorder::finish`].
pub struct Recorder {
    provider: String,
    steps: Vec<RawStep>,
    /// Open tool calls: the provider's call identity to the step's index.
    open: BTreeMap<String, usize>,
    skills: Vec<SkillRef>,
    counters: Counters,
    bytes: usize,
}

struct Finisher<'a> {
    redactor: &'a dyn Redactor,
    counters: Counters,
}
impl Finisher<'_> {
    fn text(&mut self, held: &Held, limit: usize) -> String {
        let (clean, changed) = self.redactor.redact(&held.text);
        self.counters.redacted += changed;
        if held.bytes <= limit {
            return clean;
        }
        self.counters.truncated += 1;
        let mut end = clean.len().min(limit);
        while !clean.is_char_boundary(end) {
            end -= 1;
        }
        format!(
            "{}\n[truncated: {} of {} bytes omitted]",
            &clean[..end],
            held.bytes.saturating_sub(end),
            held.bytes
        )
    }
    fn input(&mut self, value: &Value, limit: usize) -> Value {
        match value {
            Value::String(text) => {
                // A string that arrived longer than its limit was cut to the
                // limit plus the slack; that length marks it as cut.
                let held = Held {
                    bytes: text.len(),
                    text: text.clone(),
                };
                Value::String(self.text(&held, limit))
            }
            Value::Array(items) => items.iter().map(|item| self.input(item, limit)).collect(),
            Value::Object(members) => members
                .iter()
                .map(|(key, member)| (key.clone(), self.input(member, limit)))
                .collect::<Map<_, _>>()
                .into(),
            other => other.clone(),
        }
    }
}

impl Recorder {
    pub fn new(provider: &str) -> Self {
        Self {
            provider: provider.into(),
            steps: Vec::new(),
            open: BTreeMap::new(),
            skills: Vec::new(),
            counters: Counters::default(),
            bytes: 0,
        }
    }
    fn push(&mut self, mut step: RawStep) -> Option<usize> {
        let step_bytes = step.text.as_ref().map_or(0, |text| text.text.len())
            + serde_json::to_vec(&step.input).map_or(0, |bytes| bytes.len());
        if self.steps.len() >= STEPS || self.bytes + step_bytes > TRACE_BYTES {
            self.counters.omitted += 1;
            if self.steps.len() < STEPS
                && step.tool.is_some()
                && self.steps.last().is_none_or(|last| !last.omitted)
            {
                step.input = Value::Null;
                step.text = None;
                step.omitted = true;
                self.steps.push(step);
                return Some(self.steps.len() - 1);
            }
            return None;
        }
        self.bytes += step_bytes;
        self.steps.push(step);
        Some(self.steps.len() - 1)
    }
    fn message(&mut self, text: &str, agent: Option<&str>) {
        if text.trim().is_empty() {
            return;
        }
        self.push(RawStep {
            tool: None,
            input: Value::Null,
            input_limit: 0,
            text: Some(hold(text, MESSAGE_BYTES)),
            output: None,
            omitted: false,
            error: None,
            exit_code: None,
            agent: agent.map(String::from),
        });
    }
    fn started(
        &mut self,
        id: &str,
        tool: &str,
        input: &Value,
        agent: Option<&str>,
    ) -> Option<usize> {
        self.note_skills(tool, input);
        // Oversized arguments keep their shape and lose the ends of their
        // longest strings.
        let size = serde_json::to_vec(input).map_or(usize::MAX, |bytes| bytes.len());
        let input_limit = if size > INPUT_BYTES {
            INPUT_BYTES / 8
        } else {
            INPUT_BYTES
        };
        let step = RawStep {
            tool: Some(tool.into()),
            input: hold_input(input, input_limit),
            input_limit,
            text: None,
            output: None,
            omitted: false,
            error: None,
            exit_code: None,
            agent: agent.map(String::from),
        };
        let index = self.push(step)?;
        if !id.is_empty() {
            self.open.insert(id.into(), index);
        }
        Some(index)
    }
    fn complete_index(&mut self, index: usize, output: &str, error: bool, exit_code: Option<i64>) {
        let step = &mut self.steps[index];
        if !step.omitted {
            if self.bytes >= TRACE_BYTES {
                self.counters.omitted += 1;
                step.omitted = true;
            } else {
                let held = hold(output, OBSERVATION_BYTES);
                self.bytes += held.text.len();
                step.output = Some(held);
            }
        }
        step.error = Some(error);
        step.exit_code = exit_code;
    }
    fn completed(&mut self, id: &str, output: &str, error: bool, exit_code: Option<i64>) {
        let Some(index) = self.open.remove(id) else {
            return;
        };
        self.complete_index(index, output, error, exit_code);
    }
    fn note_skills(&mut self, tool: &str, input: &Value) {
        let mut found = Vec::new();
        if tool == "Skill" {
            if let Some(name) = input["skill"]
                .as_str()
                .or_else(|| input["command"].as_str())
                .or_else(|| input["name"].as_str())
            {
                found.push(SkillRef {
                    name: Some(name.trim_start_matches('/').to_string()),
                    path: None,
                });
            }
        } else {
            collect_value_skills(input, &mut found);
        }
        for skill in found {
            if !self.skills.contains(&skill) {
                self.skills.push(skill);
            }
        }
    }
    /// One event in the provider's own format.
    pub fn observe(&mut self, event: &Value) {
        match self.provider.as_str() {
            "claude" => self.claude(event),
            "codex" => self.codex(event),
            "cursor" => self.cursor(event),
            _ => {}
        }
    }
    fn claude(&mut self, event: &Value) {
        let agent = event["parent_tool_use_id"].as_str();
        let blocks = event["message"]["content"].as_array();
        match event["type"].as_str() {
            Some("assistant") => {
                for block in blocks.into_iter().flatten() {
                    match block["type"].as_str() {
                        Some("text") => self.message(block["text"].as_str().unwrap_or(""), agent),
                        Some("tool_use") => {
                            self.started(
                                block["id"].as_str().unwrap_or(""),
                                block["name"].as_str().unwrap_or("Tool"),
                                &block["input"],
                                agent,
                            );
                        }
                        Some("thinking" | "redacted_thinking") => {
                            self.counters.reasoning_dropped += 1;
                        }
                        _ => {}
                    }
                }
            }
            Some("user") => {
                for block in blocks.into_iter().flatten() {
                    if block["type"] != "tool_result" {
                        continue;
                    }
                    let output = match &block["content"] {
                        Value::String(text) => text.clone(),
                        Value::Array(parts) => parts
                            .iter()
                            .filter_map(|part| part["text"].as_str())
                            .collect::<Vec<_>>()
                            .join("\n"),
                        _ => String::new(),
                    };
                    self.completed(
                        block["tool_use_id"].as_str().unwrap_or(""),
                        &output,
                        block["is_error"] == true,
                        None,
                    );
                }
            }
            _ => {}
        }
    }
    fn codex(&mut self, event: &Value) {
        let item = &event["item"];
        let id = item["id"].as_str().unwrap_or("");
        let kind = item["type"].as_str().unwrap_or("");
        let phase = event["type"].as_str().unwrap_or("");
        if kind == "reasoning" {
            if phase == "item.completed" {
                self.counters.reasoning_dropped += 1;
            }
            return;
        }
        if kind == "agent_message" {
            if phase == "item.completed" {
                self.message(item["text"].as_str().unwrap_or(""), None);
            }
            return;
        }
        if ![
            "command_execution",
            "file_change",
            "mcp_tool_call",
            "web_search",
            "collab_tool_call",
        ]
        .contains(&kind)
            || !["item.started", "item.completed"].contains(&phase)
        {
            return;
        }
        let input = match kind {
            "command_execution" => json!({"command": item["command"]}),
            "file_change" => json!({"changes": item["changes"]}),
            "mcp_tool_call" => {
                json!({"server": item["server"], "tool": item["tool"], "arguments": item["arguments"]})
            }
            "web_search" => json!({"query": item["query"]}),
            _ => json!({"tool": item["tool"]}),
        };
        let index = if !id.is_empty()
            && let Some(&existing) = self.open.get(id)
        {
            Some(existing)
        } else {
            self.started(id, kind, &input, None)
        };
        if phase == "item.completed" {
            let output = match &item["aggregated_output"] {
                Value::String(text) => text.clone(),
                _ => match &item["result"] {
                    Value::Null => match &item["error"] {
                        Value::Null => String::new(),
                        Value::String(text) => text.clone(),
                        other => other["message"]
                            .as_str()
                            .map_or_else(|| other.to_string(), String::from),
                    },
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                },
            };
            let exit_code = item["exit_code"].as_i64();
            let error = item["status"] == "failed"
                || exit_code.is_some_and(|code| code != 0)
                || !item["error"].is_null();
            if !id.is_empty() {
                self.completed(id, &output, error, exit_code);
            } else if let Some(index) = index {
                self.complete_index(index, &output, error, exit_code);
            }
        }
    }
    fn cursor(&mut self, event: &Value) {
        match event["type"].as_str() {
            Some("thinking") => {
                if event["subtype"] == "completed" {
                    self.counters.reasoning_dropped += 1;
                }
            }
            Some("assistant") => {
                // Streaming fragments repeat the final message; only that one is kept.
                if event.get("timestamp_ms").is_some() && event.get("model_call_id").is_none() {
                    return;
                }
                for block in event["message"]["content"].as_array().into_iter().flatten() {
                    if block["type"] == "text" {
                        self.message(block["text"].as_str().unwrap_or(""), None);
                    }
                }
            }
            Some("tool_call") => {
                let id = event["call_id"].as_str().unwrap_or("").to_string();
                for (name, call) in event["tool_call"].as_object().into_iter().flatten() {
                    let (tool, arguments) = if name == "function" {
                        (
                            call["name"].as_str().unwrap_or("Tool").to_string(),
                            match &call["arguments"] {
                                Value::String(text) => {
                                    serde_json::from_str(text).unwrap_or_else(|_| json!(text))
                                }
                                other => other.clone(),
                            },
                        )
                    } else {
                        (
                            name.trim_end_matches("ToolCall").to_string(),
                            call["args"].clone(),
                        )
                    };
                    let index = if !id.is_empty()
                        && let Some(&existing) = self.open.get(&id)
                    {
                        Some(existing)
                    } else {
                        self.started(&id, &tool, &arguments, None)
                    };
                    if event["subtype"] == "completed" {
                        let outcome = &call["result"];
                        let error = outcome.get("error").is_some();
                        let body = if error {
                            &outcome["error"]
                        } else {
                            &outcome["success"]
                        };
                        let output = match body {
                            Value::Null => String::new(),
                            Value::String(text) => text.clone(),
                            other => other["content"]
                                .as_str()
                                .or_else(|| other["output"].as_str())
                                .map(String::from)
                                .or_else(|| {
                                    let stdout = other["stdout"].as_str().unwrap_or("");
                                    let stderr = other["stderr"].as_str().unwrap_or("");
                                    match (stdout.is_empty(), stderr.is_empty()) {
                                        (false, true) => Some(stdout.to_string()),
                                        (true, false) => Some(stderr.to_string()),
                                        (false, false) => Some(format!("{stdout}\n{stderr}")),
                                        (true, true) => None,
                                    }
                                })
                                .unwrap_or_else(|| other.to_string()),
                        };
                        let exit_code = outcome["success"]["exitCode"]
                            .as_i64()
                            .or_else(|| outcome["error"]["exitCode"].as_i64());
                        if !id.is_empty() {
                            self.completed(&id, &output, error, exit_code);
                        } else if let Some(index) = index {
                            self.complete_index(index, &output, error, exit_code);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    pub fn steps(&self) -> usize {
        self.steps.len()
    }
    /// The trace, with every kept text passed through `redactor` and then cut
    /// to its limit.
    pub fn finish(self, redactor: &dyn Redactor) -> Trace {
        let mut finisher = Finisher {
            redactor,
            counters: self.counters,
        };
        let steps = self
            .steps
            .iter()
            .enumerate()
            .map(|(index, raw)| {
                let mut step = Map::new();
                match &raw.tool {
                    Some(tool) => {
                        step.insert("kind".into(), "tool".into());
                        step.insert("tool".into(), tool.clone().into());
                        step.insert("input".into(), finisher.input(&raw.input, raw.input_limit));
                    }
                    None => {
                        step.insert("kind".into(), "message".into());
                    }
                }
                if let Some(text) = &raw.text {
                    step.insert("text".into(), finisher.text(text, MESSAGE_BYTES).into());
                }
                if let Some(agent) = &raw.agent {
                    step.insert("agent".into(), agent.clone().into());
                }
                if raw.omitted {
                    step.insert(
                        "output".into(),
                        "[omitted: the trace reached its size limit]".into(),
                    );
                } else if let Some(output) = &raw.output {
                    step.insert(
                        "output".into(),
                        finisher.text(output, OBSERVATION_BYTES).into(),
                    );
                }
                if let Some(error) = raw.error {
                    step.insert("error".into(), error.into());
                }
                if let Some(code) = raw.exit_code {
                    step.insert("exit_code".into(), code.into());
                }
                step.insert("index".into(), index.into());
                Value::Object(step)
            })
            .collect();
        Trace {
            provider: self.provider,
            complete: self.open.is_empty(),
            steps,
            skills: self.skills,
            counters: finisher.counters,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(provider: &str, events: &[Value]) -> Trace {
        let mut recorder = Recorder::new(provider);
        for event in events {
            recorder.observe(event);
        }
        recorder.finish(&PolicyRedactor)
    }

    fn claude_events() -> Vec<Value> {
        vec![
            json!({"type": "system", "subtype": "init", "session_id": "s1"}),
            json!({"type": "assistant", "parent_tool_use_id": null, "message": {"role": "assistant", "content": [
                {"type": "thinking", "thinking": "PRIVATE REASONING about the invoices"},
                {"type": "text", "text": "I will use the pdf skill."},
                {"type": "tool_use", "id": "toolu_1", "name": "Skill", "input": {"skill": "pdf"}}
            ]}}),
            json!({"type": "user", "message": {"content": [
                {"type": "tool_result", "tool_use_id": "toolu_1", "content": "Launching skill: pdf", "is_error": false}
            ]}}),
            json!({"type": "assistant", "parent_tool_use_id": null, "message": {"role": "assistant", "content": [
                {"type": "redacted_thinking", "data": "opaque"},
                {"type": "tool_use", "id": "toolu_2", "name": "Read", "input": {"file_path": "/work/.claude/skills/pdf/SKILL.md"}},
                {"type": "tool_use", "id": "toolu_3", "name": "Bash", "input": {"command": "python3 extract.py invoices/"}}
            ]}}),
            json!({"type": "user", "message": {"content": [
                {"type": "tool_result", "tool_use_id": "toolu_3", "content": [{"type": "text", "text": "Traceback: no such file"}], "is_error": true},
                {"type": "tool_result", "tool_use_id": "toolu_2", "content": "     1→# PDF\n     2→Use scripts.\n     3→Done.", "is_error": false}
            ]}}),
            json!({"type": "assistant", "parent_tool_use_id": "toolu_9", "message": {"role": "assistant", "content": [
                {"type": "text", "text": "helper speaking"}
            ]}}),
            json!({"type": "result", "subtype": "success", "is_error": false, "result": "done"}),
        ]
    }

    #[test]
    fn trace_pairs_claude_tool_use_with_its_result() {
        let trace = record("claude", &claude_events());
        assert!(trace.complete);
        let kinds = trace
            .steps
            .iter()
            .map(|step| {
                (
                    step["kind"].as_str().unwrap(),
                    step["tool"].as_str().unwrap_or(""),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            [
                ("message", ""),
                ("tool", "Skill"),
                ("tool", "Read"),
                ("tool", "Bash"),
                ("message", "")
            ]
        );
        // Results are paired by identity, whatever order they arrive in.
        assert_eq!(trace.steps[3]["output"], "Traceback: no such file");
        assert_eq!(trace.steps[3]["error"], true);
        assert_eq!(
            trace.steps[3]["input"]["command"],
            "python3 extract.py invoices/"
        );
        assert_eq!(trace.steps[2]["error"], false);
        assert_eq!(trace.steps[4]["agent"], "toolu_9");
        assert_eq!(
            trace
                .steps
                .iter()
                .map(|step| step["index"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            [0, 1, 2, 3, 4]
        );
        // No reasoning is kept, in any form.
        assert_eq!(trace.counters.reasoning_dropped, 2);
        let text = trace.to_value().to_string();
        assert!(
            !text.contains("PRIVATE REASONING") && !text.contains("opaque"),
            "{text}"
        );
        assert_eq!(
            trace.skills,
            [
                SkillRef {
                    name: Some("pdf".into()),
                    path: None
                },
                SkillRef {
                    name: None,
                    path: Some("/work/.claude/skills/pdf/SKILL.md".into())
                },
            ]
        );
        // A file reader's output is the file, without its line numbers.
        assert_eq!(trace.observed_files(), ["# PDF\nUse scripts.\nDone."]);
    }

    #[test]
    fn trace_pairs_codex_items() {
        let trace = record(
            "codex",
            &[
                json!({"type": "thread.started", "thread_id": "t1"}),
                json!({"type": "item.completed", "item": {"id": "i0", "type": "reasoning", "text": "PRIVATE REASONING"}}),
                json!({"type": "item.started", "item": {"id": "i1", "type": "command_execution", "command": "bash -lc 'cat .agents/skills/pdf/SKILL.md'", "status": "in_progress"}}),
                json!({"type": "item.completed", "item": {"id": "i1", "type": "command_execution", "command": "bash -lc 'cat .agents/skills/pdf/SKILL.md'", "aggregated_output": "# PDF\n", "exit_code": 0, "status": "completed"}}),
                json!({"type": "item.started", "item": {"id": "i2", "type": "command_execution", "command": "python3 x.py", "status": "in_progress"}}),
                json!({"type": "item.completed", "item": {"id": "i2", "type": "command_execution", "command": "python3 x.py", "aggregated_output": "boom", "exit_code": 2, "status": "failed"}}),
                json!({"type": "item.completed", "item": {"id": "i3", "type": "file_change", "changes": [{"path": "out.csv", "kind": "add"}], "status": "completed"}}),
                json!({"type": "item.completed", "item": {"id": "i4", "type": "agent_message", "text": "Done."}}),
                json!({"type": "turn.completed", "usage": {}}),
            ],
        );
        assert!(trace.complete);
        assert_eq!(trace.steps.len(), 4);
        assert_eq!(trace.steps[0]["tool"], "command_execution");
        assert_eq!(trace.steps[0]["output"], "# PDF\n");
        assert_eq!(trace.steps[0]["exit_code"], 0);
        assert_eq!(trace.steps[1]["error"], true);
        assert_eq!(trace.steps[1]["exit_code"], 2);
        // An item seen only when it completed is a whole step all the same.
        assert_eq!(trace.steps[2]["tool"], "file_change");
        assert_eq!(trace.steps[2]["input"]["changes"][0]["path"], "out.csv");
        assert_eq!(
            trace.steps[3],
            json!({"kind": "message", "text": "Done.", "index": 3})
        );
        assert_eq!(trace.counters.reasoning_dropped, 1);
        assert!(!trace.to_value().to_string().contains("PRIVATE REASONING"));
        assert_eq!(
            trace.skills,
            [SkillRef {
                name: None,
                path: Some(".agents/skills/pdf/SKILL.md".into())
            }]
        );
        assert_eq!(trace.observed_files(), ["# PDF\n"]);
    }

    #[test]
    fn trace_pairs_cursor_tool_calls() {
        let trace = record(
            "cursor",
            &[
                json!({"type": "thinking", "subtype": "delta", "text": "PRIVATE"}),
                json!({"type": "thinking", "subtype": "completed"}),
                json!({"type": "assistant", "timestamp_ms": 1, "message": {"role": "assistant", "content": [{"type": "text", "text": "partial"}]}}),
                json!({"type": "tool_call", "subtype": "started", "call_id": "c1", "tool_call": {"readToolCall": {"args": {"path": ".cursor/skills/pdf/SKILL.md"}}}}),
                json!({"type": "tool_call", "subtype": "completed", "call_id": "c1", "tool_call": {"readToolCall": {"args": {"path": ".cursor/skills/pdf/SKILL.md"}, "result": {"success": {"content": "# PDF"}}}}}),
                json!({"type": "tool_call", "subtype": "started", "call_id": "c2", "tool_call": {"function": {"name": "run_terminal", "arguments": "{\"command\":\"ls\"}"}}}),
                json!({"type": "tool_call", "subtype": "completed", "call_id": "c2", "tool_call": {"function": {"name": "run_terminal", "arguments": "{\"command\":\"ls\"}", "result": {"error": {"message": "denied"}}}}}),
                json!({"type": "assistant", "model_call_id": "m1", "timestamp_ms": 2, "message": {"role": "assistant", "content": [{"type": "text", "text": "Final answer."}]}}),
            ],
        );
        assert!(trace.complete);
        assert_eq!(trace.steps.len(), 3, "{:?}", trace.steps);
        assert_eq!(trace.steps[0]["tool"], "read");
        assert_eq!(trace.steps[0]["output"], "# PDF");
        assert_eq!(trace.steps[1]["tool"], "run_terminal");
        assert_eq!(trace.steps[1]["input"]["command"], "ls");
        assert_eq!(trace.steps[1]["error"], true);
        assert_eq!(trace.steps[2]["text"], "Final answer.");
        assert_eq!(trace.counters.reasoning_dropped, 1);
        assert!(!trace.to_value().to_string().contains("PRIVATE"));
        assert_eq!(
            trace.skills,
            [SkillRef {
                name: None,
                path: Some(".cursor/skills/pdf/SKILL.md".into())
            }]
        );
    }

    #[test]
    fn trace_masks_secrets_and_bounds_every_observation() {
        let secret = format!(
            "api_key = \"{}\"",
            ["sk", "live", &"9fQ2xLm7".repeat(4)].join("_")
        );
        let long = "x".repeat(OBSERVATION_BYTES + 5000);
        let trace = record(
            "claude",
            &[
                json!({"type": "assistant", "message": {"role": "assistant", "content": [
                    {"type": "text", "text": format!("Found {secret} in the file.")},
                    {"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": format!("curl -d '{secret}' http://localhost")}},
                    {"type": "tool_use", "id": "t2", "name": "Bash", "input": {"command": "cat big.txt"}},
                    {"type": "tool_use", "id": "t3", "name": "Write", "input": {"file_path": "a.txt", "content": "y".repeat(INPUT_BYTES * 2)}}
                ]}}),
                json!({"type": "user", "message": {"content": [
                    {"type": "tool_result", "tool_use_id": "t1", "content": format!("echo {secret}")},
                    {"type": "tool_result", "tool_use_id": "t2", "content": long}
                ]}}),
            ],
        );
        let text = trace.to_value().to_string();
        assert!(!text.contains("9fQ2xLm7"), "a secret survived redaction");
        assert!(trace.counters.redacted >= 3, "{:?}", trace.counters);
        let output = trace.steps[2]["output"].as_str().unwrap();
        assert!(output.len() < OBSERVATION_BYTES + 200);
        assert!(
            output.ends_with("bytes omitted]"),
            "{}",
            &output[output.len() - 60..]
        );
        let content = trace.steps[3]["input"]["content"].as_str().unwrap();
        assert!(content.len() < INPUT_BYTES, "oversized arguments are cut");
        assert_eq!(trace.counters.truncated, 2);
        // The call without a result leaves the trace incomplete.
        assert!(!trace.complete);
        assert_eq!(trace.to_value()["redaction"]["truncated"], 2);
    }

    #[test]
    fn trace_stops_growing_at_its_limits() {
        let big = "z".repeat(OBSERVATION_BYTES);
        let mut recorder = Recorder::new("codex");
        for index in 0..(TRACE_BYTES / OBSERVATION_BYTES + 20) {
            recorder.observe(&json!({"type": "item.completed", "item": {
                "id": format!("i{index}"), "type": "command_execution", "command": "cat f",
                "aggregated_output": big, "exit_code": 0, "status": "completed"}}));
        }
        let trace = recorder.finish(&PolicyRedactor);
        assert!(trace.counters.omitted >= 10, "{:?}", trace.counters);
        let size = serde_json::to_vec(&trace.to_value()).unwrap().len();
        assert!(size < TRACE_BYTES + 2 * OBSERVATION_BYTES, "{size}");
        assert_eq!(
            trace.steps.last().unwrap()["output"],
            "[omitted: the trace reached its size limit]"
        );
    }

    #[test]
    fn the_compiler_view_fits_its_budget_and_keeps_every_step() {
        let steps = (0..40)
            .map(|index| {
                json!({"index": index, "kind": "tool", "tool": "Bash",
                    "input": {"command": format!("cat file-{index}"), "content": "c".repeat(9000)},
                    "output": format!("START-{index} {} END-{index}", "o".repeat(60_000)), "error": false})
            })
            .collect::<Vec<_>>();
        let view = compiler_view(&steps, 64 * 1024);
        let size = serde_json::to_vec(&view).unwrap().len();
        assert!(size <= 64 * 1024, "{size}");
        let view = view.as_array().unwrap();
        assert_eq!(view.len(), 40);
        for (index, step) in view.iter().enumerate() {
            assert_eq!(step["index"], index);
            assert_eq!(step["input"]["command"], format!("cat file-{index}"));
            let output = step["output"].as_str().unwrap();
            // The beginning and the end of a result are what a recipe needs.
            assert!(output.starts_with(&format!("START-{index} ")), "{output}");
            assert!(output.ends_with(&format!(" END-{index}")), "{output}");
            assert!(output.contains("bytes not shown"));
        }
        // A small trace is passed as it is.
        let small = vec![json!({"index": 0, "kind": "message", "text": "Done."})];
        assert_eq!(compiler_view(&small, 64 * 1024), Value::Array(small));
    }

    #[test]
    fn only_a_plain_file_read_counts_as_file_content() {
        for command in [
            "cat notes.txt",
            "bash -lc 'cat src/a.rs'",
            "Get-Content a.txt",
            "/bin/cat a",
            "head -n 20 src/a.rs",
            "tail -n 10 logs/out.log",
            "sed -n '1,20p' config.yaml",
            "jq '.routes' routes.json",
            "python3 -c \"print(open('data.txt').read())\"",
            "python -c \"from pathlib import Path; print(Path('a.txt').read_text())\"",
        ] {
            assert!(reads_one_file(command), "{command}");
        }
        for command in [
            "cat a | grep b",
            "ls",
            "python3 x.py",
            "bash -c \"",
            "bash -lc '",
            "sed -i 's/a/b/' src/a.rs",
            "tail -f logs/out.log",
            "python3 -c \"Path('a.txt').write_text('x')\"",
            "cat a > b",
            "cat a; rm b",
        ] {
            assert!(!reads_one_file(command), "{command}");
        }
        assert_eq!(unnumbered("plain\ntext"), "plain\ntext");
    }

    #[test]
    fn trace_bounds_inputs_and_messages_and_handles_spaces_errors_and_empty_ids() {
        let mut recorder = Recorder::new("claude");
        let big = "a".repeat(MESSAGE_BYTES);
        for _ in 0..(TRACE_BYTES / MESSAGE_BYTES + 20) {
            recorder.observe(
                &json!({"type": "assistant", "message": {"role": "assistant", "content": [
                    {"type": "text", "text": big}
                ]}}),
            );
        }
        let bounded = recorder.finish(&PolicyRedactor);
        let size = serde_json::to_vec(&bounded.to_value()).unwrap().len();
        assert!(size < TRACE_BYTES + 2 * MESSAGE_BYTES, "{size}");
        assert!(bounded.counters.omitted >= 10, "{:?}", bounded.counters);

        let claude = record(
            "claude",
            &[
                json!({"type": "assistant", "message": {"role": "assistant", "content": [
                    {"type": "tool_use", "id": "t1", "name": "Read", "input": {"file_path": "/work/My Project/.claude/skills/pdf/SKILL.md"}},
                    {"type": "tool_use", "id": "t2", "name": "Bash", "input": {"command": "cat \"/work/My Other Project/.claude/skills/csv/SKILL.md\""}}
                ]}}),
            ],
        );
        assert_eq!(
            claude.skills,
            [
                SkillRef {
                    name: None,
                    path: Some("/work/My Project/.claude/skills/pdf/SKILL.md".into()),
                },
                SkillRef {
                    name: None,
                    path: Some("/work/My Other Project/.claude/skills/csv/SKILL.md".into()),
                },
            ]
        );

        let codex = record(
            "codex",
            &[
                json!({"type": "item.completed", "item": {"id": "", "type": "mcp_tool_call", "server": "s", "tool": "t", "arguments": {}, "error": {"message": "mcp failed"}, "status": "failed"}}),
            ],
        );
        assert_eq!(codex.steps.len(), 1);
        assert_eq!(codex.steps[0]["output"], "mcp failed");
        assert_eq!(codex.steps[0]["error"], true);

        let cursor = record(
            "cursor",
            &[
                json!({"type": "tool_call", "subtype": "completed", "call_id": "", "tool_call": {"shellToolCall": {"args": {"command": "false"}, "result": {"error": {"stdout": "out", "stderr": "err", "exitCode": 7}}}}}),
            ],
        );
        assert_eq!(cursor.steps.len(), 1);
        assert_eq!(cursor.steps[0]["output"], "out\nerr");
        assert_eq!(cursor.steps[0]["exit_code"], 7);
        assert_eq!(cursor.steps[0]["error"], true);
    }

    #[test]
    fn shell_chain_splitter_unwraps_wrappers_and_rejects_redirects_and_subshells() {
        assert!(is_shell_tool_name("command_execution"));
        assert!(is_shell_tool_name("Bash"));
        assert!(is_shell_tool_name("shellToolCall"));
        assert!(is_shell_tool_name("shell"));
        assert!(!is_shell_tool_name("file_change"));

        assert_eq!(
            split_read_only_shell_chain("bash -lc 'git status --short && cat composer.json'"),
            Some(vec![
                "git status --short".to_string(),
                "cat composer.json".to_string()
            ])
        );
        assert_eq!(
            split_read_only_shell_chain("/bin/sh -c \"ls -la; rg 'a|b' src\""),
            Some(vec!["ls -la".to_string(), "rg 'a|b' src".to_string()])
        );
        assert_eq!(
            split_read_only_shell_chain("cat a.txt | grep foo"),
            Some(vec!["cat a.txt".to_string(), "grep foo".to_string()])
        );
        for unsafe_cmd in [
            "cat a.txt > out.txt",
            "echo $(whoami)",
            "echo `whoami`",
            "git status && ",
            "git status &",
            "git status || ls",
            "bash -lc 'unclosed",
            "(cd sub && ls)",
        ] {
            assert_eq!(
                split_read_only_shell_chain(unsafe_cmd),
                None,
                "{unsafe_cmd}"
            );
        }
    }
}
