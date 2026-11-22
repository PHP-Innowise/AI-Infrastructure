//! Hosts that keep talking while they run: Claude Code's stream-json control
//! channel, Codex's app server and Cursor's ACP server.
//!
//! A one-shot launch writes its prompt, closes the host's input and reads what
//! the host prints. A launch that may ask a person, or whose host must be
//! told its permission mode after it starts, keeps the input open instead.
//! Each driver here speaks one host's protocol and turns what the host says
//! into the events that host prints in a one-shot launch, so the rest of a
//! run reads them unchanged, and turns a permission request into a question.
use crate::permission::{Answer, Decision, PermissionAsk, PermissionMode, bound_diff_preview};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

/// The longest summary or detail a request shows.
const SUMMARY_LIMIT: usize = 400;
const DETAIL_LIMIT: usize = 800;
const CURSOR_SESSION_RULE_SUMMARY: &str =
    "Remember exact command/path in Harness for this session (never writes ~/.cursor)";

/// How a host request's reply is built from a person's [`Answer`].
pub(crate) enum AskReply {
    Static {
        allow: Value,
        allow_session: Option<Value>,
        deny: Value,
    },
    Dynamic(Box<dyn Fn(&Answer) -> Value + Send + Sync>),
}

/// A host's request as a person sees it, with the line that answers it each way.
pub(crate) struct Ask {
    pub public: PermissionAsk,
    pub reply: AskReply,
}
impl Ask {
    pub fn reply_for(&self, answer: &Answer) -> Value {
        match &self.reply {
            AskReply::Static {
                allow,
                allow_session,
                deny,
            } => match answer.decision {
                Decision::Allow => allow.clone(),
                Decision::AllowSession => allow_session.clone().unwrap_or_else(|| allow.clone()),
                Decision::Deny => deny.clone(),
            },
            AskReply::Dynamic(format) => format(answer),
        }
    }
    pub fn deny_reply(&self) -> Value {
        self.reply_for(&Answer::from_decision(Decision::Deny))
    }
    #[cfg(test)]
    pub fn allow_reply(&self) -> Value {
        self.reply_for(&Answer::from_decision(Decision::Allow))
    }
    #[cfg(test)]
    pub fn allow_session_reply(&self) -> Option<Value> {
        match &self.reply {
            AskReply::Static { allow_session, .. } => allow_session.clone(),
            AskReply::Dynamic(_) => self
                .public
                .session_option
                .then(|| self.reply_for(&Answer::from_decision(Decision::AllowSession))),
        }
    }
}

/// What one host message asks of a run.
#[derive(Default)]
pub(crate) struct Step {
    /// Events in the host's one-shot format, for the run to read as usual.
    pub events: Vec<Value>,
    /// Session events, emitted as they are.
    pub notices: Vec<Value>,
    /// Lines for the host's input, in order.
    pub replies: Vec<Value>,
    /// Requests that wait for a person.
    pub asks: Vec<Ask>,
    /// Requests the host no longer waits for.
    pub withdrawn: Vec<String>,
    /// The host needs no more input.
    pub close_input: bool,
    /// The exchange is complete; a host still serving may be stopped.
    pub finished: bool,
}

pub(crate) trait Driver: Send {
    fn opening(&mut self) -> Step;
    fn receive(&mut self, message: Value) -> Step;
    /// The person's answer to one of the driver's asks, by its id, heard
    /// before the host can act on it.
    fn answered(&mut self, _ask: &str, _allowed: bool) {}
}

pub(crate) struct Setup {
    pub mode: PermissionMode,
    pub prompt: String,
    pub cwd: String,
    pub model: Option<String>,
    pub resume: Option<String>,
    pub auto_asks: bool,
}

pub(crate) fn claude(setup: Setup) -> Box<dyn Driver> {
    Box::new(Claude {
        mode: setup.mode,
        auto_asks: setup.auto_asks,
        prompt: setup.prompt,
        sent: false,
        fallen_back: false,
    })
}
pub(crate) fn codex(setup: Setup) -> Box<dyn Driver> {
    Box::new(Codex {
        setup,
        next: 0,
        pending: BTreeMap::new(),
        thread: None,
        turn: None,
        turn_started: false,
        files: BTreeMap::new(),
        baseline: None,
        latest: None,
        done: false,
    })
}
pub(crate) fn cursor(setup: Setup) -> Box<dyn Driver> {
    Box::new(Cursor {
        setup,
        next: 0,
        pending: BTreeMap::new(),
        session: None,
        ready: false,
        text: String::new(),
        said: String::new(),
        calls: BTreeMap::new(),
        diffs: BTreeMap::new(),
        asked: BTreeMap::new(),
        rejected: BTreeSet::new(),
        started: std::time::Instant::now(),
        plans: 0,
        done: false,
    })
}

fn bounded(value: &str, limit: usize) -> String {
    let clean: String = value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let clean = clean.trim();
    if clean.chars().count() <= limit {
        return clean.to_owned();
    }
    let mut out: String = clean.chars().take(limit - 1).collect();
    out.push('…');
    out
}
fn some_text(value: &Value, limit: usize) -> Option<String> {
    value
        .as_str()
        .map(|s| bounded(s, limit))
        .filter(|s| !s.is_empty())
}
fn compact(value: &Value, limit: usize) -> String {
    bounded(&value.to_string(), limit)
}
fn request_id(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn format_hunk_lines(old: &str, new: &str) -> Option<String> {
    let old = old.replace("\r\n", "\n").replace('\r', "\n");
    let new = new.replace("\r\n", "\n").replace('\r', "\n");
    if old == new {
        return None;
    }
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let mut prefix = 0;
    while prefix < a.len().min(b.len()) && a[prefix] == b[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix
        < a.len()
            .saturating_sub(prefix)
            .min(b.len().saturating_sub(prefix))
        && a[a.len() - 1 - suffix] == b[b.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let start = prefix.saturating_sub(3);
    let end_a = (a.len() - suffix + 3).min(a.len());
    let end_b = (b.len() - suffix + 3).min(b.len());
    let old_len = end_a - start;
    let new_len = end_b - start;
    let old_start = if old_len == 0 { 0 } else { start + 1 };
    let new_start = if new_len == 0 { 0 } else { start + 1 };
    let mut out = format!("@@ -{old_start},{old_len} +{new_start},{new_len} @@\n");
    for line in &a[start..prefix] {
        out.push(' ');
        out.push_str(line);
        out.push('\n');
    }
    for line in &a[prefix..a.len() - suffix] {
        out.push('-');
        out.push_str(line);
        out.push('\n');
    }
    for line in &b[prefix..b.len() - suffix] {
        out.push('+');
        out.push_str(line);
        out.push('\n');
    }
    for line in &a[a.len() - suffix..end_a] {
        out.push(' ');
        out.push_str(line);
        out.push('\n');
    }
    Some(out)
}

fn display_diff_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return "file".into();
    }
    if let Some(norm) = normalize_lexical(Path::new(trimmed))
        && let Some(s) = norm.to_str()
        && !s.is_empty()
    {
        // A diff names its files with forward slashes on every host; a path
        // rebuilt from its components comes back with the host's separator.
        return if cfg!(windows) {
            s.replace('\\', "/")
        } else {
            s.to_owned()
        };
    }
    trimmed.to_owned()
}

fn format_replacement_diff(path: &str, old: &str, new: &str) -> Option<String> {
    let hunk = format_hunk_lines(old, new)?;
    let display = display_diff_path(path);
    bound_diff_preview(&format!("--- a/{display}\n+++ b/{display}\n{hunk}"))
}

fn format_write_diff(path: &str, content: &str) -> Option<String> {
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    let display = display_diff_path(path);
    let lines: Vec<&str> = normalized.lines().collect();
    if lines.is_empty() {
        return bound_diff_preview(&format!("--- /dev/null\n+++ b/{display}\n@@ -0,0 +0,0 @@"));
    }
    let mut out = format!(
        "--- /dev/null\n+++ b/{display}\n@@ -0,0 +1,{} @@\n",
        lines.len()
    );
    for line in lines {
        out.push('+');
        out.push_str(line);
        out.push('\n');
        if out.chars().count() > crate::permission::DIFF_PREVIEW_LIMIT + 64 {
            break;
        }
    }
    bound_diff_preview(&out)
}

fn format_patch_for_path(path: &str, patch: &str) -> Option<String> {
    let normalized = patch.replace("\r\n", "\n").replace('\r', "\n");
    let trimmed = normalized.trim_matches('\n').trim_end();
    if trimmed.trim().is_empty() {
        return None;
    }
    if trimmed.starts_with("--- ")
        || trimmed.starts_with("diff --git ")
        || trimmed.starts_with("*** ")
    {
        return Some(trimmed.to_owned());
    }
    let display = display_diff_path(path);
    Some(format!("--- a/{display}\n+++ b/{display}\n{trimmed}"))
}

#[derive(Clone)]
struct ParsedOption {
    id: Option<String>,
    label: String,
}

#[derive(Clone)]
struct ParsedQuestion {
    id: String,
    header: String,
    question: String,
    options: Vec<ParsedOption>,
    public: Value,
}

fn parse_questions(raw: &Value, default_is_other: bool) -> Vec<ParsedQuestion> {
    let Some(items) = raw.as_array() else {
        return vec![];
    };
    let mut parsed = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        if !item.is_object() {
            continue;
        }
        let question = item["question"]
            .as_str()
            .or_else(|| item["prompt"].as_str())
            .unwrap_or("")
            .trim()
            .to_owned();
        let header = item["header"]
            .as_str()
            .or_else(|| item["title"].as_str())
            .unwrap_or("")
            .trim()
            .to_owned();
        let mut opts = Vec::new();
        let mut public_opts = Vec::new();
        for opt in item["options"].as_array().into_iter().flatten() {
            match opt {
                Value::String(s) => {
                    let label = s.trim();
                    if !label.is_empty() {
                        opts.push(ParsedOption {
                            id: None,
                            label: label.to_owned(),
                        });
                        public_opts.push(json!({"label": label, "description": ""}));
                    }
                }
                Value::Object(_) => {
                    let opt_id = opt["id"]
                        .as_str()
                        .or_else(|| opt["optionId"].as_str())
                        .or_else(|| opt["value"].as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned);
                    let label = opt["label"]
                        .as_str()
                        .or_else(|| opt["name"].as_str())
                        .or_else(|| opt["text"].as_str())
                        .or_else(|| opt["title"].as_str())
                        .or(opt_id.as_deref())
                        .unwrap_or("")
                        .trim()
                        .to_owned();
                    let description = opt["description"].as_str().unwrap_or("").trim().to_owned();
                    if label.is_empty() && opt_id.is_none() {
                        continue;
                    }
                    let shown = if label.is_empty() {
                        opt_id.clone().unwrap_or_default()
                    } else {
                        label
                    };
                    opts.push(ParsedOption {
                        id: opt_id,
                        label: shown.clone(),
                    });
                    public_opts.push(json!({"label": shown, "description": description}));
                }
                _ => {}
            }
        }
        if question.is_empty() && header.is_empty() && opts.is_empty() {
            continue;
        }
        let id = item["id"]
            .as_str()
            .or_else(|| item["questionId"].as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                if !question.is_empty() {
                    question.clone()
                } else {
                    format!("q-{}", idx + 1)
                }
            });
        let multi_select = item["multi_select"]
            .as_bool()
            .or_else(|| item["multiSelect"].as_bool())
            .or_else(|| item["allowMultiple"].as_bool())
            .unwrap_or(false);
        let is_other = item["is_other"]
            .as_bool()
            .or_else(|| item["isOther"].as_bool())
            .or_else(|| item["allowFreeform"].as_bool())
            .unwrap_or(default_is_other || opts.is_empty());
        let public = json!({
            "id": id,
            "header": header,
            "question": question,
            "multi_select": multi_select,
            "is_other": is_other,
            "options": public_opts,
        });
        parsed.push(ParsedQuestion {
            id,
            header,
            question,
            options: opts,
            public,
        });
    }
    parsed
}

fn question_summary(questions: &[ParsedQuestion]) -> String {
    let first = questions
        .first()
        .map(|q| {
            if !q.question.is_empty() {
                q.question.as_str()
            } else if !q.header.is_empty() {
                q.header.as_str()
            } else {
                q.id.as_str()
            }
        })
        .unwrap_or("Answer questions");
    if questions.len() > 1 {
        bounded(
            &format!("{first} (+{} more)", questions.len() - 1),
            SUMMARY_LIMIT,
        )
    } else {
        bounded(first, SUMMARY_LIMIT)
    }
}

fn collect_answer_strings(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let mut push = |raw: &str| {
        let trimmed = raw.trim();
        if !trimmed.is_empty() && !out.iter().any(|item| item == trimmed) {
            out.push(trimmed.to_owned());
        }
    };
    match value {
        Value::String(s) => push(s),
        Value::Array(items) => {
            for item in items {
                match item {
                    Value::String(s) => push(s),
                    Value::Object(obj) => {
                        if let Some(s) = obj
                            .get("label")
                            .or_else(|| obj.get("id"))
                            .or_else(|| obj.get("value"))
                            .and_then(Value::as_str)
                        {
                            push(s);
                        }
                    }
                    _ => {}
                }
            }
        }
        Value::Object(obj) => {
            if let Some(inner) = obj.get("answers") {
                for item in collect_answer_strings(inner) {
                    push(&item);
                }
            }
            if let Some(selected) = obj.get("selectedOptionIds") {
                for item in collect_answer_strings(selected) {
                    push(&item);
                }
            }
            for key in ["freeform", "other", "value", "label"] {
                if let Some(s) = obj.get(key).and_then(Value::as_str) {
                    push(s);
                }
            }
        }
        _ => {}
    }
    out
}

fn find_question_answer<'a>(
    source: &'a serde_json::Map<String, Value>,
    q: &ParsedQuestion,
) -> Option<&'a Value> {
    source
        .get(&q.id)
        .or_else(|| {
            (!q.question.is_empty())
                .then(|| source.get(&q.question))
                .flatten()
        })
        .or_else(|| {
            (!q.header.is_empty())
                .then(|| source.get(&q.header))
                .flatten()
        })
}

fn extract_selected_for_question(answers: Option<&Value>, q: &ParsedQuestion) -> Vec<String> {
    let Some(map) = answers.and_then(Value::as_object) else {
        return vec![];
    };
    let nested = map.get("answers").and_then(Value::as_object);
    find_question_answer(map, q)
        .or_else(|| nested.and_then(|inner| find_question_answer(inner, q)))
        .map(collect_answer_strings)
        .unwrap_or_default()
}

// ---------------------------------------------------------------- Claude Code

const CLAUDE_INIT: &str = "harness-initialize";
const CLAUDE_AUTO: &str = "harness-auto-mode";
const CLAUDE_MODE: &str = "harness-permission-mode";
const DECLINED_BY_PERSON: &str = "The person declined this request in the Harness.";
const DECLINED_AUTOMATICALLY: &str = "Declined automatically: this session runs with Auto permissions, where a request that needs a person is declined. Continue another way, or say what you need so the person can allow it.";

struct Claude {
    mode: PermissionMode,
    auto_asks: bool,
    prompt: String,
    sent: bool,
    /// Claude Code was told to accept edits because auto mode is unavailable.
    fallen_back: bool,
}
impl Claude {
    /// Where auto mode is unavailable (for a model, plan or setting) Claude
    /// Code runs in its default mode, and with nobody to ask it would refuse
    /// every edit: it is told to accept edits instead, and the chat says so.
    fn fall_back(&mut self, step: &mut Step, reason: &str) {
        if self.fallen_back {
            return;
        }
        self.fallen_back = true;
        step.replies.push(json!({
            "type": "control_request",
            "request_id": CLAUDE_MODE,
            "request": {"subtype": "set_permission_mode", "mode": "acceptEdits"},
        }));
        step.notices.push(json!({
            "kind": "status",
            "text": format!(
                "Auto mode is unavailable here ({}), so Claude Code accepts file edits on its own and declines anything else that needs approval. Choose Manual to answer those requests yourself.",
                reason.trim_end_matches('.')
            ),
        }));
    }
    fn prompt(&mut self, step: &mut Step) {
        if !self.sent {
            self.sent = true;
            step.replies.push(
                json!({"type":"user","message":{"role":"user","content":self.prompt.clone()}}),
            );
        }
    }
}
impl Driver for Claude {
    fn opening(&mut self) -> Step {
        Step {
            replies: vec![
                json!({"type":"control_request","request_id":CLAUDE_INIT,"request":{"subtype":"initialize"}}),
            ],
            ..Step::default()
        }
    }
    fn receive(&mut self, message: Value) -> Step {
        let mut step = Step::default();
        match message["type"].as_str().unwrap_or("") {
            "control_response" => {
                let response = &message["response"];
                let ok = response["subtype"] == "success";
                match response["request_id"].as_str().unwrap_or("") {
                    CLAUDE_INIT if !ok => {
                        step.events.push(json!({
                            "type": "result",
                            "subtype": "error_during_execution",
                            "is_error": true,
                            "result": format!(
                                "Claude Code did not open its permission channel: {}",
                                some_text(&response["error"], DETAIL_LIMIT).unwrap_or_else(|| "no reason given".into())
                            ),
                        }));
                        step.close_input = true;
                    }
                    CLAUDE_INIT if self.mode == PermissionMode::Auto => {
                        // The mode reported at start can still change once
                        // Claude Code checks the model; asking for auto mode
                        // explicitly gets a definite answer.
                        step.replies.push(json!({
                            "type": "control_request",
                            "request_id": CLAUDE_AUTO,
                            "request": {"subtype": "set_permission_mode", "mode": "auto"},
                        }));
                    }
                    CLAUDE_INIT => self.prompt(&mut step),
                    CLAUDE_AUTO => {
                        let mode = response["response"]["mode"].as_str().unwrap_or("");
                        if ok && mode == "auto" {
                            self.prompt(&mut step);
                        } else {
                            let reason = some_text(&response["error"], DETAIL_LIMIT)
                                .map(|error| {
                                    error
                                        .strip_prefix("Cannot set permission mode to auto: ")
                                        .unwrap_or(&error)
                                        .to_owned()
                                })
                                .unwrap_or_else(|| {
                                    format!(
                                        "Claude Code stayed in its {} mode",
                                        if mode.is_empty() { "default" } else { mode }
                                    )
                                });
                            self.fall_back(&mut step, &reason);
                        }
                    }
                    CLAUDE_MODE => {
                        if !ok {
                            step.notices.push(json!({
                                "kind": "status",
                                "text": "Claude Code kept its default mode, so it declines edits that need approval.",
                            }));
                        }
                        self.prompt(&mut step);
                    }
                    _ => {}
                }
            }
            "control_request" => {
                let request = &message["request"];
                if request["subtype"] == "can_use_tool" {
                    let ask = claude_ask(&message["request_id"], request, self.mode);
                    if self.mode == PermissionMode::Manual
                        || (self.mode == PermissionMode::Auto && self.auto_asks)
                    {
                        step.asks.push(ask);
                    } else {
                        step.notices
                            .push(ask.public.resolved_event("declined", "auto"));
                        step.replies.push(ask.deny_reply());
                    }
                } else {
                    step.replies.push(json!({
                        "type": "control_response",
                        "response": {
                            "subtype": "error",
                            "request_id": message["request_id"],
                            "error": "The Harness does not answer this control request.",
                        },
                    }));
                }
            }
            "control_cancel_request" => {
                step.withdrawn
                    .push(format!("claude-{}", request_id(&message["request_id"])));
            }
            "keep_alive" => {}
            "result" => {
                // Claude Code exits once its input closes, after its own
                // end-of-session work.
                step.events.push(message);
                step.close_input = true;
            }
            "system" => {
                // Auto mode can still be withdrawn after it was granted, for
                // a model or a setting: Claude Code reports its new mode.
                if self.mode == PermissionMode::Auto
                    && self.sent
                    && message["permissionMode"] == "default"
                {
                    self.fall_back(&mut step, "Claude Code left auto mode");
                }
                step.events.push(message);
            }
            _ => step.events.push(message),
        }
        step
    }
}
fn claude_kind(name: &str) -> &'static str {
    match name {
        "AskUserQuestion" => "question",
        "Bash" | "PowerShell" => "command",
        "Edit" | "MultiEdit" | "Write" | "NotebookEdit" => "file_change",
        _ => "tool",
    }
}
fn claude_diff_preview(name: &str, input: &Value) -> Option<String> {
    let path = input["file_path"]
        .as_str()
        .or_else(|| input["notebook_path"].as_str())
        .or_else(|| input["path"].as_str())
        .unwrap_or("");
    match name {
        "Edit" => {
            if let (Some(old), Some(new)) =
                (input["old_string"].as_str(), input["new_string"].as_str())
            {
                return format_replacement_diff(path, old, new);
            }
            input["diff"]
                .as_str()
                .or_else(|| input["patch"].as_str())
                .and_then(|patch| format_patch_for_path(path, patch))
                .and_then(|patch| bound_diff_preview(&patch))
        }
        "MultiEdit" => {
            let edits = input["edits"].as_array()?;
            let mut hunks = Vec::new();
            for edit in edits {
                let old = edit["old_string"]
                    .as_str()
                    .or_else(|| edit["old_text"].as_str())
                    .unwrap_or("");
                let new = edit["new_string"]
                    .as_str()
                    .or_else(|| edit["new_text"].as_str())
                    .unwrap_or("");
                if let Some(hunk) = format_hunk_lines(old, new) {
                    hunks.push(hunk);
                }
            }
            if hunks.is_empty() {
                return None;
            }
            let display = display_diff_path(path);
            let mut raw = format!("--- a/{display}\n+++ b/{display}\n");
            for hunk in hunks {
                raw.push_str(&hunk);
            }
            bound_diff_preview(&raw)
        }
        "Write" => input["content"]
            .as_str()
            .and_then(|content| format_write_diff(path, content)),
        "NotebookEdit" => input["new_source"].as_str().and_then(|new| {
            if let Some(old) = input["old_source"].as_str() {
                format_replacement_diff(path, old, new)
            } else {
                format_write_diff(path, new)
            }
        }),
        _ => None,
    }
}
fn claude_rule_summary(remembered: &[Value]) -> Option<String> {
    if remembered.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    for suggestion in remembered {
        if let Some(rules) = suggestion["rules"].as_array() {
            for rule in rules {
                let Some(tool_name) = rule["toolName"].as_str().map(str::trim) else {
                    continue;
                };
                if tool_name.is_empty() {
                    continue;
                }
                let content = rule["ruleContent"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                match content {
                    Some(content) => parts.push(format!("{tool_name}({content})")),
                    None => parts.push(tool_name.to_owned()),
                }
            }
        }
        if let Some(dirs) = suggestion["directories"].as_array() {
            for dir in dirs.iter().filter_map(Value::as_str).map(str::trim) {
                if !dir.is_empty() {
                    parts.push(format!("dir({dir})"));
                }
            }
        }
        if let Some(mode) = suggestion["mode"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            parts.push(format!("mode({mode})"));
        }
    }
    if parts.is_empty() {
        Some("Session rule: remember for this Claude Code session".into())
    } else {
        Some(bounded(
            &format!("Session rule: {}", parts.join(", ")),
            SUMMARY_LIMIT,
        ))
    }
}
fn claude_summary(tool: &str, input: &Value, request: &Value) -> String {
    let field = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| some_text(&input[*key], SUMMARY_LIMIT))
    };
    let found = match tool {
        "Bash" | "PowerShell" => field(&["command"]),
        "Edit" | "Write" | "MultiEdit" | "Read" | "NotebookEdit" => {
            field(&["file_path", "notebook_path", "path"])
        }
        "WebFetch" => field(&["url"]),
        "WebSearch" => field(&["query"]),
        "Glob" | "Grep" => field(&["pattern"]),
        "ExitPlanMode" => Some("Leave plan mode and start changing files".into()),
        _ => None,
    };
    found
        .or_else(|| some_text(&request["title"], SUMMARY_LIMIT))
        .or_else(|| some_text(&request["description"], SUMMARY_LIMIT))
        .unwrap_or_else(|| compact(input, SUMMARY_LIMIT))
}
fn claude_ask(id: &Value, request: &Value, mode: PermissionMode) -> Ask {
    let name = request["tool_name"].as_str().unwrap_or("");
    let tool = some_text(&request["display_name"], 80)
        .or_else(|| some_text(&request["tool_name"], 80))
        .unwrap_or_else(|| "Tool".into());
    let input = if request["input"].is_object() {
        request["input"].clone()
    } else {
        json!({})
    };
    let reply_id = id.clone();
    let reply = move |response: Value| {
        json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": reply_id,
                "response": response,
            }
        })
    };
    let deny_message = if mode == PermissionMode::Manual {
        DECLINED_BY_PERSON
    } else {
        DECLINED_AUTOMATICALLY
    };
    let deny_reply = reply(json!({"behavior": "deny", "message": deny_message}));

    if name == "AskUserQuestion" {
        let parsed = parse_questions(&input["questions"], true);
        let summary = if parsed.is_empty() {
            claude_summary(name, &input, request)
        } else {
            question_summary(&parsed)
        };
        let detail = ["decision_reason", "description", "blocked_path"]
            .iter()
            .find_map(|key| some_text(&request[*key], DETAIL_LIMIT))
            .filter(|detail| detail != &summary);
        let questions =
            (!parsed.is_empty()).then(|| parsed.iter().map(|q| q.public.clone()).collect());
        let original_input = input.clone();
        let parsed_for_reply = parsed;
        let reply_id = id.clone();
        return Ask {
            public: PermissionAsk {
                id: format!("claude-{}", request_id(id)),
                provider: "claude".into(),
                tool,
                summary,
                detail,
                session_option: false,
                kind: "question".into(),
                diff_preview: None,
                rule_summary: None,
                session_rule_key: None,
                questions,
            },
            reply: AskReply::Dynamic(Box::new(move |answer: &Answer| {
                if answer.decision == Decision::Deny {
                    return json!({
                        "type": "control_response",
                        "response": {
                            "subtype": "success",
                            "request_id": reply_id,
                            "response": {
                                "behavior": "deny",
                                "message": deny_message,
                            }
                        }
                    });
                }
                let mut answers_map = serde_json::Map::new();
                for q in &parsed_for_reply {
                    let selected = extract_selected_for_question(answer.answers.as_ref(), q);
                    if !selected.is_empty() {
                        let key = if !q.question.is_empty() {
                            q.question.clone()
                        } else {
                            q.id.clone()
                        };
                        answers_map.insert(key, Value::String(selected.join(", ")));
                    }
                }
                if parsed_for_reply.is_empty()
                    && let Some(raw_map) = answer.answers.as_ref().and_then(Value::as_object)
                {
                    for (k, v) in raw_map {
                        let selected = collect_answer_strings(v);
                        if !selected.is_empty() {
                            answers_map.insert(k.clone(), Value::String(selected.join(", ")));
                        }
                    }
                }
                let mut updated_input = original_input.clone();
                if !updated_input["questions"].is_array() {
                    updated_input["questions"] = json!([]);
                }
                updated_input["answers"] = Value::Object(answers_map);
                json!({
                    "type": "control_response",
                    "response": {
                        "subtype": "success",
                        "request_id": reply_id,
                        "response": {
                            "behavior": "allow",
                            "updatedInput": updated_input,
                        }
                    }
                })
            })),
        };
    }

    let summary = claude_summary(name, &input, request);
    let detail = ["decision_reason", "description", "blocked_path"]
        .iter()
        .find_map(|key| some_text(&request[*key], DETAIL_LIMIT))
        .filter(|detail| detail != &summary);
    // Claude Code proposes rules for its local settings file; the Harness
    // keeps an approval in the running session and writes nothing to the
    // project.
    let remembered: Vec<Value> = request["permission_suggestions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|suggestion| suggestion.is_object())
        .map(|suggestion| {
            let mut suggestion = suggestion.clone();
            suggestion["destination"] = json!("session");
            suggestion
        })
        .collect();
    let rule_summary = claude_rule_summary(&remembered);
    let diff_preview = claude_diff_preview(name, &input);
    Ask {
        public: PermissionAsk {
            id: format!("claude-{}", request_id(id)),
            provider: "claude".into(),
            tool,
            summary,
            detail,
            session_option: !remembered.is_empty(),
            kind: claude_kind(name).into(),
            diff_preview,
            rule_summary,
            session_rule_key: None,
            questions: None,
        },
        reply: AskReply::Static {
            allow: reply(json!({"behavior":"allow","updatedInput":input})),
            allow_session: (!remembered.is_empty()).then(|| {
                reply(json!({"behavior":"allow","updatedInput":input,"updatedPermissions":remembered}))
            }),
            deny: deny_reply,
        },
    }
}

// --------------------------------------------------------------------- Codex

/// Codex's app server, answering every approval a thread asks for. A Manual
/// launch keeps the workspace read-only, so no change happens without the
/// person: Codex asks to apply each edit and to run each command that writes.
struct Codex {
    setup: Setup,
    next: u64,
    pending: BTreeMap<u64, &'static str>,
    thread: Option<String>,
    turn: Option<String>,
    turn_started: bool,
    /// The changes each file edit item proposes, for its approval request.
    files: BTreeMap<String, Vec<Value>>,
    baseline: Option<[u64; 4]>,
    latest: Option<[u64; 4]>,
    done: bool,
}
impl Codex {
    fn request(&mut self, method: &'static str, params: Value) -> Value {
        self.next += 1;
        self.pending.insert(self.next, method);
        json!({"id": self.next, "method": method, "params": params})
    }
    fn failed(&mut self, step: &mut Step, message: String) {
        step.events
            .push(json!({"type":"turn.failed","error":{"message":message}}));
        self.done = true;
        step.close_input = true;
        step.finished = true;
    }
    fn ours(&self, params: &Value) -> bool {
        params["threadId"]
            .as_str()
            .is_none_or(|thread| self.thread.as_deref() == Some(thread))
    }
}
fn usage_counts(breakdown: &Value) -> [u64; 4] {
    [
        breakdown["inputTokens"].as_u64().unwrap_or(0),
        breakdown["cachedInputTokens"].as_u64().unwrap_or(0),
        breakdown["outputTokens"].as_u64().unwrap_or(0),
        breakdown["reasoningOutputTokens"].as_u64().unwrap_or(0),
    ]
}
fn codex_status(status: &Value) -> &str {
    match status.as_str() {
        Some("inProgress") => "in_progress",
        Some(other) => other,
        None => "completed",
    }
}
fn joined(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part.as_str().or_else(|| part["text"].as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}
/// An app-server item as `codex exec --json` prints it.
fn codex_item(item: &Value) -> Option<Value> {
    let id = &item["id"];
    Some(match item["type"].as_str()? {
        "agentMessage" => json!({"id":id,"type":"agent_message","text":item["text"]}),
        "reasoning" => {
            let summary = joined(&item["summary"]);
            let text = if summary.is_empty() {
                joined(&item["content"])
            } else {
                summary
            };
            json!({"id":id,"type":"reasoning","text":text})
        }
        "commandExecution" => json!({
            "id": id,
            "type": "command_execution",
            "command": item["command"],
            "aggregated_output": item["aggregatedOutput"],
            "exit_code": item["exitCode"],
            "status": codex_status(&item["status"]),
        }),
        "fileChange" => json!({
            "id": id,
            "type": "file_change",
            "changes": codex_changes(item)
                .into_iter()
                .map(|c| json!({"path": c["path"], "kind": c["kind"]}))
                .collect::<Vec<_>>(),
            "status": codex_status(&item["status"]),
        }),
        "mcpToolCall" => json!({
            "id": id,
            "type": "mcp_tool_call",
            "server": item["server"],
            "tool": item["tool"],
            "status": codex_status(&item["status"]),
        }),
        "webSearch" => json!({"id":id,"type":"web_search","query":item["query"]}),
        "collabAgentToolCall" => json!({
            "id": id,
            "type": "collab_tool_call",
            "tool": item["tool"],
            "status": codex_status(&item["status"]),
        }),
        _ => return None,
    })
}
fn codex_changes(item: &Value) -> Vec<Value> {
    item["changes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|change| {
            let path = change["path"].as_str()?;
            let kind = change["kind"]["type"]
                .as_str()
                .or_else(|| change["kind"].as_str())
                .unwrap_or("update");
            let mut entry = json!({"path": path, "kind": kind});
            if let Some(diff) = change["diff"]
                .as_str()
                .filter(|diff| !diff.trim().is_empty())
            {
                entry["diff"] = json!(diff);
            }
            Some(entry)
        })
        .collect()
}
fn codex_diff_preview(params: &Value, files: &BTreeMap<String, Vec<Value>>) -> Option<String> {
    let from_state = params["itemId"].as_str().and_then(|item| files.get(item));
    let from_params: Vec<Value> = params["changes"]
        .as_array()
        .map(|_| codex_changes(params))
        .unwrap_or_default();
    let changes = from_state
        .map(Vec::as_slice)
        .filter(|c| !c.is_empty())
        .unwrap_or(&from_params);
    let mut patches = Vec::new();
    for change in changes {
        let path = change["path"].as_str().unwrap_or("");
        if let Some(patch) = change["diff"]
            .as_str()
            .and_then(|diff| format_patch_for_path(path, diff))
        {
            patches.push(patch);
        }
    }
    if patches.is_empty()
        && let Some(raw_diff) = params["diff"]
            .as_str()
            .and_then(|diff| format_patch_for_path("", diff))
    {
        patches.push(raw_diff);
    }
    if patches.is_empty() {
        None
    } else {
        bound_diff_preview(&patches.join("\n"))
    }
}
fn codex_ask(
    id: &Value,
    method: &str,
    params: &Value,
    files: &BTreeMap<String, Vec<Value>>,
) -> Ask {
    let reason = some_text(&params["reason"], DETAIL_LIMIT);
    let reply = |result: Value| json!({"id": id, "result": result});
    let rule_summary = Some("Remember approval for this Codex thread".to_owned());
    let public = |tool: &str,
                  kind: &str,
                  summary: String,
                  detail: Option<String>,
                  diff_preview: Option<String>| PermissionAsk {
        id: format!("codex-{}", request_id(id)),
        provider: "codex".into(),
        tool: tool.into(),
        summary,
        detail,
        session_option: true,
        kind: kind.into(),
        diff_preview,
        rule_summary: rule_summary.clone(),
        session_rule_key: None,
        questions: None,
    };
    match method {
        "item/commandExecution/requestApproval" => {
            let network = params["networkApprovalContext"]["host"]
                .as_str()
                .map(|host| format!("network access to {host}"));
            let summary = some_text(&params["command"], SUMMARY_LIMIT)
                .or(network)
                .unwrap_or_else(|| "a command".into());
            let cwd = some_text(&params["cwd"], 300).map(|cwd| format!("in {cwd}"));
            let detail = match (reason, cwd) {
                (Some(reason), Some(cwd)) => Some(format!("{reason} · {cwd}")),
                (reason, cwd) => reason.or(cwd),
            };
            Ask {
                public: public("Shell", "command", summary, detail, None),
                reply: AskReply::Static {
                    allow: reply(json!({"decision":"accept"})),
                    allow_session: Some(reply(json!({"decision":"acceptForSession"}))),
                    deny: reply(json!({"decision":"decline"})),
                },
            }
        }
        "item/fileChange/requestApproval" => {
            let changes = params["itemId"]
                .as_str()
                .and_then(|item| files.get(item))
                .map(|changes| {
                    changes
                        .iter()
                        .map(|change| {
                            format!(
                                "{} {}",
                                change["kind"].as_str().unwrap_or("update"),
                                change["path"].as_str().unwrap_or("")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .filter(|summary| !summary.is_empty());
            let root = some_text(&params["grantRoot"], 300)
                .map(|root| format!("write access under {root}"));
            let diff_preview = codex_diff_preview(params, files);
            Ask {
                public: public(
                    "File change",
                    "file_change",
                    bounded(
                        &changes
                            .or(root)
                            .unwrap_or_else(|| "apply a change to files".into()),
                        SUMMARY_LIMIT,
                    ),
                    reason,
                    diff_preview,
                ),
                reply: AskReply::Static {
                    allow: reply(json!({"decision":"accept"})),
                    allow_session: Some(reply(json!({"decision":"acceptForSession"}))),
                    deny: reply(json!({"decision":"decline"})),
                },
            }
        }
        _ => {
            // item/permissions/requestApproval: more file system or network access.
            let requested = &params["permissions"];
            let mut parts = vec![];
            for (key, label) in [("write", "write"), ("read", "read")] {
                let paths = requested["fileSystem"][key]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>();
                if !paths.is_empty() {
                    parts.push(format!("{label} {}", paths.join(", ")));
                }
            }
            if requested["network"]["enabled"] == true {
                parts.push("network access".into());
            }
            let summary = if parts.is_empty() {
                compact(requested, SUMMARY_LIMIT)
            } else {
                bounded(&parts.join("; "), SUMMARY_LIMIT)
            };
            let granted = if requested.is_object() {
                requested.clone()
            } else {
                json!({})
            };
            Ask {
                public: public("Permissions", "permissions", summary, reason, None),
                reply: AskReply::Static {
                    allow: reply(json!({"permissions":granted,"scope":"turn"})),
                    allow_session: Some(reply(json!({"permissions":granted,"scope":"session"}))),
                    deny: reply(json!({"permissions":{}})),
                },
            }
        }
    }
}
fn codex_question_ask(id: &Value, params: &Value) -> Option<Ask> {
    let parsed = parse_questions(&params["questions"], false);
    if parsed.is_empty() {
        return None;
    }
    let summary = question_summary(&parsed);
    let detail = some_text(&params["reason"], DETAIL_LIMIT).filter(|d| d != &summary);
    let questions = Some(parsed.iter().map(|q| q.public.clone()).collect());
    let reply_id = id.clone();
    Some(Ask {
        public: PermissionAsk {
            id: format!("codex-{}", request_id(id)),
            provider: "codex".into(),
            tool: "Question".into(),
            summary,
            detail,
            session_option: false,
            kind: "question".into(),
            diff_preview: None,
            rule_summary: None,
            session_rule_key: None,
            questions,
        },
        reply: AskReply::Dynamic(Box::new(move |answer: &Answer| {
            if answer.decision == Decision::Deny {
                return json!({"id": reply_id, "result": {"answers": {}}});
            }
            let mut answers_map = serde_json::Map::new();
            for q in &parsed {
                let selected = extract_selected_for_question(answer.answers.as_ref(), q);
                if !selected.is_empty() {
                    answers_map.insert(q.id.clone(), json!({"answers": selected}));
                }
            }
            json!({"id": reply_id, "result": {"answers": answers_map}})
        })),
    })
}
impl Driver for Codex {
    fn opening(&mut self) -> Step {
        let initialize = self.request(
            "initialize",
            json!({
                "clientInfo": {"name": "infra-harness", "title": "AI Infrastructure Harness", "version": env!("CARGO_PKG_VERSION")},
                "capabilities": {"experimentalApi": true},
            }),
        );
        Step {
            replies: vec![initialize],
            ..Step::default()
        }
    }
    fn receive(&mut self, message: Value) -> Step {
        let mut step = Step::default();
        if self.done {
            return step;
        }
        let method = message["method"].as_str();
        let id = message.get("id").filter(|id| !id.is_null());
        match (method, id) {
            // A response to a request of ours.
            (None, Some(id)) => {
                let Some(asked) = id.as_u64().and_then(|id| self.pending.remove(&id)) else {
                    return step;
                };
                if !message["error"].is_null() {
                    let reason = some_text(&message["error"]["message"], DETAIL_LIMIT)
                        .unwrap_or_else(|| compact(&message["error"], DETAIL_LIMIT));
                    self.failed(&mut step, format!("Codex app server refused {asked}: {reason}"));
                    return step;
                }
                let result = &message["result"];
                match asked {
                    "initialize" => {
                        step.replies.push(json!({"method":"initialized"}));
                        let mut params = json!({
                            "cwd": self.setup.cwd,
                            "approvalPolicy": "on-request",
                            "approvalsReviewer": "user",
                            "sandbox": "read-only",
                        });
                        if let Some(model) = &self.setup.model {
                            params["model"] = json!(model);
                        }
                        let request = match self.setup.resume.clone() {
                            Some(thread) => {
                                params["threadId"] = json!(thread);
                                self.request("thread/resume", params)
                            }
                            None => self.request("thread/start", params),
                        };
                        step.replies.push(request);
                    }
                    "thread/start" | "thread/resume" => {
                        let Some(thread) = result["thread"]["id"].as_str().map(String::from)
                        else {
                            self.failed(&mut step, "Codex app server named no thread.".into());
                            return step;
                        };
                        step.events
                            .push(json!({"type":"thread.started","thread_id":thread}));
                        let turn = self.request(
                            "turn/start",
                            json!({
                                "threadId": thread,
                                "input": [{"type":"text","text":self.setup.prompt,"text_elements":[]}],
                            }),
                        );
                        self.thread = Some(thread);
                        step.replies.push(turn);
                    }
                    "turn/start" => {
                        self.turn = result["turn"]["id"].as_str().map(String::from);
                        if !self.turn_started {
                            self.turn_started = true;
                            step.events.push(json!({"type":"turn.started"}));
                        }
                    }
                    _ => {}
                }
            }
            // A request the server makes of us.
            (Some(method), Some(id)) => match method {
                "item/commandExecution/requestApproval"
                | "item/fileChange/requestApproval"
                | "item/permissions/requestApproval" => {
                    step.asks
                        .push(codex_ask(id, method, &message["params"], &self.files));
                }
                "execCommandApproval" | "applyPatchApproval" => {
                    step.replies
                        .push(json!({"id":id,"result":{"decision":"denied"}}));
                }
                "item/tool/requestUserInput" => {
                    if self.setup.mode == PermissionMode::Manual
                        && let Some(ask) = codex_question_ask(id, &message["params"])
                    {
                        step.asks.push(ask);
                    } else {
                        step.replies.push(json!({"id":id,"result":{"answers":{}}}));
                    }
                }
                "mcpServer/elicitation/request" => {
                    step.replies
                        .push(json!({"id":id,"result":{"action":"decline"}}));
                }
                _ => step.replies.push(json!({
                    "id": id,
                    "error": {"code": -32601, "message": "The Harness does not answer this request."},
                })),
            },
            // A notification.
            (Some(method), None) => {
                let params = &message["params"];
                if !self.ours(params) {
                    return step;
                }
                match method {
                    "turn/started" => {
                        if !self.turn_started {
                            self.turn_started = true;
                            step.events.push(json!({"type":"turn.started"}));
                        }
                    }
                    "item/started" | "item/completed" => {
                        let item = &params["item"];
                        if item["type"] == "fileChange"
                            && let Some(id) = item["id"].as_str()
                        {
                            self.files.insert(id.into(), codex_changes(item));
                        }
                        if let Some(item) = codex_item(item) {
                            step.events.push(json!({
                                "type": if method == "item/started" { "item.started" } else { "item.completed" },
                                "item": item,
                            }));
                        }
                    }
                    "turn/plan/updated" => {
                        let items: Vec<Value> = params["plan"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|entry| {
                                json!({"text":entry["step"],"completed":entry["status"]=="completed"})
                            })
                            .collect();
                        step.events.push(json!({
                            "type": "item.updated",
                            "item": {"id": "harness-plan", "type": "todo_list", "items": items},
                        }));
                    }
                    "thread/tokenUsage/updated" => {
                        let total = usage_counts(&params["tokenUsage"]["total"]);
                        if self.baseline.is_none() {
                            let last = usage_counts(&params["tokenUsage"]["last"]);
                            self.baseline = Some([
                                total[0].saturating_sub(last[0]),
                                total[1].saturating_sub(last[1]),
                                total[2].saturating_sub(last[2]),
                                total[3].saturating_sub(last[3]),
                            ]);
                        }
                        self.latest = Some(total);
                    }
                    "error" => {
                        if params["willRetry"] != true {
                            step.events.push(json!({
                                "type": "error",
                                "message": some_text(&params["error"]["message"], DETAIL_LIMIT)
                                    .unwrap_or_else(|| "Codex reported an error.".into()),
                            }));
                        }
                    }
                    "turn/completed" => {
                        let turn = &params["turn"];
                        if self
                            .turn
                            .as_deref()
                            .is_some_and(|ours| turn["id"].as_str().is_some_and(|id| id != ours))
                        {
                            return step;
                        }
                        let status = turn["status"].as_str().unwrap_or("completed");
                        if status == "completed" {
                            let (base, latest) = (
                                self.baseline.unwrap_or_default(),
                                self.latest.unwrap_or_default(),
                            );
                            let used = |i: usize| latest[i].saturating_sub(base[i]);
                            step.events.push(json!({
                                "type": "turn.completed",
                                "usage": {
                                    "input_tokens": used(0),
                                    "cached_input_tokens": used(1),
                                    "output_tokens": used(2),
                                    "reasoning_output_tokens": used(3),
                                },
                            }));
                        } else {
                            let reason = some_text(&turn["error"]["message"], DETAIL_LIMIT)
                                .unwrap_or_else(|| format!("The Codex turn ended as {status}."));
                            step.events
                                .push(json!({"type":"turn.failed","error":{"message":reason}}));
                        }
                        self.done = true;
                        step.close_input = true;
                        step.finished = true;
                    }
                    _ => {}
                }
            }
            (None, None) => {}
        }
        step
    }
}

// -------------------------------------------------------------------- Cursor

/// Cursor's ACP server. Print mode cannot ask anyone: it declines what needs
/// approval. Over ACP, Cursor asks the client for each request instead.
struct Cursor {
    setup: Setup,
    next: u64,
    pending: BTreeMap<u64, &'static str>,
    session: Option<String>,
    /// Updates before a loaded session answers replay its history.
    ready: bool,
    /// Text not yet shown.
    text: String,
    /// The turn's text, for its result.
    said: String,
    calls: BTreeMap<String, (String, Value)>,
    /// Bounded diff previews captured from tool_call updates, by toolCallId.
    diffs: BTreeMap<String, String>,
    /// The tool call each unanswered ask is about.
    asked: BTreeMap<String, String>,
    /// Calls the person declined. Cursor still reports them completed.
    rejected: BTreeSet<String>,
    started: std::time::Instant,
    plans: u64,
    done: bool,
}
impl Cursor {
    /// What a finished call came to; a call the person declined did not run.
    fn outcome(&mut self, id: &str, update: &Value) -> Option<Value> {
        let outcome = cursor_outcome(update)?;
        Some(if self.rejected.remove(id) {
            json!({"rejected": {"reason": DECLINED_BY_PERSON}})
        } else {
            outcome
        })
    }
    fn request(&mut self, method: &'static str, params: Value) -> Value {
        self.next += 1;
        self.pending.insert(self.next, method);
        json!({"jsonrpc": "2.0", "id": self.next, "method": method, "params": params})
    }
    fn flush(&mut self, step: &mut Step) {
        if self.text.trim().is_empty() {
            self.text.clear();
            return;
        }
        let text = std::mem::take(&mut self.text);
        if !self.said.is_empty() {
            self.said.push_str("\n\n");
        }
        self.said.push_str(text.trim());
        step.events.push(json!({
            "type": "assistant",
            "message": {"role": "assistant", "content": [{"type": "text", "text": text.trim()}]},
            "session_id": self.session,
        }));
    }
    fn result(&mut self, step: &mut Step, ok: bool, text: String) {
        self.flush(step);
        step.events.push(json!({
            "type": "result",
            "subtype": if ok { "success" } else { "error_during_execution" },
            "is_error": !ok,
            "result": if ok { self.said.clone() } else { text },
            "session_id": self.session,
            "duration_ms": self.started.elapsed().as_millis() as u64,
        }));
        self.done = true;
        step.close_input = true;
        step.finished = true;
    }
    fn prompt(&mut self, step: &mut Step) {
        let session = self.session.clone();
        let prompt = self.setup.prompt.clone();
        let request = self.request(
            "session/prompt",
            json!({"sessionId": session, "prompt": [{"type": "text", "text": prompt}]}),
        );
        step.replies.push(request);
    }
    fn update(&mut self, step: &mut Step, update: &Value) {
        match update["sessionUpdate"].as_str().unwrap_or("") {
            "agent_message_chunk" => {
                if let Some(text) = update["content"]["text"].as_str() {
                    self.text.push_str(text);
                }
            }
            "tool_call" => {
                self.flush(step);
                let id = request_id(&update["toolCallId"]);
                let (name, args) = cursor_tool(update);
                if name == "editToolCall" {
                    let path = args["path"].as_str().unwrap_or("");
                    if let Some(diff) =
                        cursor_diff_preview(path, &args, &update["rawInput"], &update["content"])
                    {
                        self.diffs.insert(id.clone(), diff);
                    }
                }
                step.events
                    .push(cursor_call(&id, "started", &name, &args, None));
                self.calls.insert(id.clone(), (name, args));
                if let Some(outcome) = self.outcome(&id, update) {
                    let (name, args) = self.calls.remove(&id).unwrap_or_default();
                    self.diffs.remove(&id);
                    step.events
                        .push(cursor_call(&id, "completed", &name, &args, Some(outcome)));
                }
            }
            "tool_call_update" => {
                let id = request_id(&update["toolCallId"]);
                let existing_path = self
                    .calls
                    .get(&id)
                    .and_then(|(_, args)| args["path"].as_str())
                    .or_else(|| update["locations"][0]["path"].as_str())
                    .or_else(|| update["rawInput"]["path"].as_str())
                    .unwrap_or("");
                if (self
                    .calls
                    .get(&id)
                    .is_some_and(|(name, _)| name == "editToolCall")
                    || update["kind"].as_str() == Some("edit"))
                    && let Some(diff) = cursor_diff_preview(
                        existing_path,
                        &Value::Null,
                        &update["rawInput"],
                        &update["content"],
                    )
                {
                    self.diffs.insert(id.clone(), diff);
                }
                if let Some(outcome) = self.outcome(&id, update) {
                    let (name, args) = self
                        .calls
                        .remove(&id)
                        .unwrap_or_else(|| cursor_tool(update));
                    self.diffs.remove(&id);
                    step.events
                        .push(cursor_call(&id, "completed", &name, &args, Some(outcome)));
                }
            }
            "plan" => {
                let todos: Vec<Value> = update["entries"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|entry| json!({"content": entry["content"], "status": entry["status"]}))
                    .collect();
                self.plan(step, todos);
            }
            _ => {}
        }
    }
    fn plan(&mut self, step: &mut Step, todos: Vec<Value>) {
        self.plans += 1;
        let id = format!("harness-plan-{}", self.plans);
        let args = json!({"todos": todos});
        step.events.push(cursor_call(
            &id,
            "completed",
            "updateTodosToolCall",
            &args,
            Some(json!({"success": {"todos": args["todos"]}})),
        ));
    }
}
/// A tool call's name and arguments as Cursor's print mode names them.
fn cursor_tool(update: &Value) -> (String, Value) {
    let raw = &update["rawInput"];
    let title = cursor_title(update);
    let title = title.as_str();
    let path = update["locations"][0]["path"]
        .as_str()
        .or_else(|| raw["path"].as_str())
        .or_else(|| raw["file_path"].as_str())
        .or_else(|| raw["filePath"].as_str());
    let text = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| raw[*key].as_str())
            .unwrap_or(title)
            .to_owned()
    };
    match update["kind"].as_str().unwrap_or("other") {
        "execute" => (
            "shellToolCall".into(),
            json!({"command": text(&["command", "cmd"])}),
        ),
        "edit" => ("editToolCall".into(), json!({"path": path})),
        "delete" => ("deleteToolCall".into(), json!({"path": path})),
        "read" => ("readToolCall".into(), json!({"path": path})),
        "search" => (
            "grepToolCall".into(),
            json!({"pattern": text(&["pattern", "query", "globPattern"]), "path": path}),
        ),
        "fetch" => ("webFetchToolCall".into(), json!({"url": text(&["url"])})),
        _ => (
            "function".into(),
            json!({"name": if title.is_empty() { "Tool" } else { title }, "arguments": raw}),
        ),
    }
}
/// A tool call's title without the backticks Cursor writes a command in.
fn cursor_title(call: &Value) -> String {
    call["title"]
        .as_str()
        .unwrap_or("")
        .trim()
        .trim_matches('`')
        .trim()
        .to_owned()
}
/// The result a finished tool call carries, or none while it runs.
fn cursor_outcome(update: &Value) -> Option<Value> {
    match update["status"].as_str()? {
        "completed" => Some(json!({"success": {}})),
        "failed" => Some(
            json!({"error": {"message": some_text(&update["rawOutput"]["error"], DETAIL_LIMIT)}}),
        ),
        _ => None,
    }
}
fn cursor_call(id: &str, phase: &str, name: &str, args: &Value, outcome: Option<Value>) -> Value {
    let mut call = if name == "function" {
        json!({"name": args["name"], "arguments": args["arguments"]})
    } else {
        json!({"args": args})
    };
    if let Some(outcome) = outcome {
        call["result"] = outcome;
    }
    let mut tool_call = serde_json::Map::new();
    tool_call.insert(name.to_owned(), call);
    json!({"type": "tool_call", "subtype": phase, "call_id": id, "tool_call": tool_call})
}
fn cursor_diff_from_obj(path: &str, obj: &Value) -> Option<String> {
    if !obj.is_object() {
        return None;
    }
    let item_path = obj["path"]
        .as_str()
        .or_else(|| obj["file_path"].as_str())
        .or_else(|| obj["filePath"].as_str())
        .unwrap_or(path);
    for key in ["diff", "patch", "unifiedDiff"] {
        if let Some(diff) = obj[key].as_str()
            && let Some(patch) = format_patch_for_path(item_path, diff)
            && let Some(bounded) = bound_diff_preview(&patch)
        {
            return Some(bounded);
        }
    }
    for (old_key, new_key) in [
        ("oldText", "newText"),
        ("old_text", "new_text"),
        ("oldString", "newString"),
        ("old_string", "new_string"),
        ("oldContent", "newContent"),
        ("old_content", "new_content"),
    ] {
        if let (Some(old), Some(new)) = (obj[old_key].as_str(), obj[new_key].as_str()) {
            return format_replacement_diff(item_path, old, new);
        }
    }
    if let Some(edits) = obj["edits"].as_array() {
        let mut hunks = Vec::new();
        for edit in edits {
            let old = edit["oldText"]
                .as_str()
                .or_else(|| edit["old_text"].as_str())
                .or_else(|| edit["old_string"].as_str())
                .or_else(|| edit["oldString"].as_str())
                .unwrap_or("");
            let new = edit["newText"]
                .as_str()
                .or_else(|| edit["new_text"].as_str())
                .or_else(|| edit["new_string"].as_str())
                .or_else(|| edit["newString"].as_str())
                .unwrap_or("");
            if let Some(hunk) = format_hunk_lines(old, new) {
                hunks.push(hunk);
            }
        }
        if !hunks.is_empty() {
            let display = display_diff_path(item_path);
            let mut raw = format!("--- a/{display}\n+++ b/{display}\n");
            for hunk in hunks {
                raw.push_str(&hunk);
            }
            return bound_diff_preview(&raw);
        }
    }
    for key in [
        "newText",
        "new_text",
        "newContent",
        "new_content",
        "content",
        "fileContent",
        "fileText",
        "text",
    ] {
        if let Some(content) = obj[key].as_str() {
            return format_write_diff(item_path, content);
        }
    }
    None
}
fn cursor_diff_preview(
    path: &str,
    args: &Value,
    raw_input: &Value,
    content: &Value,
) -> Option<String> {
    if let Some(diff) = cursor_diff_from_obj(path, args) {
        return Some(diff);
    }
    if let Some(diff) = cursor_diff_from_obj(path, raw_input) {
        return Some(diff);
    }
    if let Some(items) = content.as_array() {
        let mut parts = Vec::new();
        for item in items {
            let is_diff_block = item["type"].as_str() == Some("diff")
                || [
                    "diff",
                    "patch",
                    "unifiedDiff",
                    "oldText",
                    "newText",
                    "old_text",
                    "new_text",
                    "old_string",
                    "new_string",
                ]
                .iter()
                .any(|key| item.get(*key).is_some_and(Value::is_string));
            if is_diff_block && let Some(diff) = cursor_diff_from_obj(path, item) {
                parts.push(diff);
            }
        }
        if !parts.is_empty() {
            return bound_diff_preview(&parts.join("\n"));
        }
    } else if content.is_object() {
        return cursor_diff_from_obj(path, content);
    }
    None
}
fn normalize_lexical(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                out.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => return None,
        }
    }
    Some(out)
}
fn clean_workspace_relative_path(raw: &str, cwd: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty()
        || raw.chars().any(char::is_control)
        || raw == "~"
        || raw.starts_with("~/")
        || raw.starts_with("~\\")
        || (raw.contains('\\') && !cfg!(windows))
    {
        return None;
    }
    let candidate = Path::new(raw);
    let has_parent = candidate
        .components()
        .any(|c| matches!(c, Component::ParentDir));
    if has_parent {
        return None;
    }
    let rel_buf: PathBuf = if candidate.is_absolute()
        || candidate
            .components()
            .any(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
    {
        let cwd_path = Path::new(cwd.trim());
        if cwd.trim().is_empty() || !cwd_path.is_absolute() {
            return None;
        }
        let norm_cwd = normalize_lexical(cwd_path)?;
        let norm_candidate = normalize_lexical(candidate)?;
        norm_candidate.strip_prefix(&norm_cwd).ok()?.to_path_buf()
    } else {
        normalize_lexical(candidate)?
    };
    let mut parts = Vec::new();
    for component in rel_buf.components() {
        match component {
            Component::Normal(seg) => {
                let s = seg.to_str()?;
                if !s.is_empty() {
                    parts.push(s);
                }
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}
fn clean_exact_shell_command(raw: &str) -> Option<&str> {
    let cmd = raw.trim();
    if cmd.is_empty()
        || cmd.chars().any(|c| {
            c.is_control()
                || matches!(
                    c,
                    ';' | '|' | '&' | '`' | '$' | '>' | '<' | '\n' | '\r' | '\0'
                )
        })
    {
        return None;
    }
    Some(cmd)
}
fn cursor_session_rule_key(name: &str, args: &Value, call: &Value, cwd: &str) -> Option<String> {
    let raw_input = &call["rawInput"];
    match name {
        "editToolCall" | "readToolCall" | "deleteToolCall" => {
            let op = match name {
                "editToolCall" => "edit",
                "readToolCall" => "read",
                _ => "delete",
            };
            let raw_path = args["path"]
                .as_str()
                .or_else(|| call["locations"][0]["path"].as_str())
                .or_else(|| raw_input["path"].as_str())
                .or_else(|| raw_input["file_path"].as_str())
                .or_else(|| raw_input["filePath"].as_str())?;
            let clean = clean_workspace_relative_path(raw_path, cwd)?;
            Some(format!("cursor:{op}:{clean}"))
        }
        "shellToolCall" => {
            let raw_cmd = args["command"]
                .as_str()
                .or_else(|| raw_input["command"].as_str())
                .or_else(|| raw_input["cmd"].as_str())?;
            let clean = clean_exact_shell_command(raw_cmd)?;
            Some(format!("cursor:shell:{clean}"))
        }
        _ => None,
    }
}
/// `known` is the call as its `tool_call` update started it: Cursor's request
/// names the call by its title alone, without its input.
fn cursor_ask(
    id: &Value,
    params: &Value,
    known: Option<(String, Value)>,
    known_diff: Option<String>,
    cwd: &str,
) -> Ask {
    let call = &params["toolCall"];
    let (name, mut args) = known.unwrap_or_else(|| cursor_tool(call));
    if args.is_object() {
        let (fallback_name, fallback_args) = cursor_tool(call);
        if fallback_name == name
            && let Some(obj) = args.as_object_mut()
            && let Some(fb) = fallback_args.as_object()
        {
            for (k, v) in fb {
                if obj.get(k).is_none_or(Value::is_null) && !v.is_null() {
                    obj.insert(k.clone(), v.clone());
                }
            }
        }
    }
    let (tool, kind) = match name.as_str() {
        "shellToolCall" => ("Shell".to_owned(), "command"),
        "editToolCall" => ("Edit".to_owned(), "file_change"),
        "deleteToolCall" => ("Delete".to_owned(), "file_change"),
        "readToolCall" => ("Read".to_owned(), "tool"),
        "grepToolCall" => ("Search".to_owned(), "tool"),
        "webFetchToolCall" => ("Fetch".to_owned(), "tool"),
        _ => (
            some_text(&call["kind"], 40)
                .filter(|kind| kind != "other")
                .unwrap_or_else(|| "Tool".into()),
            "tool",
        ),
    };
    let title = some_text(&json!(cursor_title(call)), SUMMARY_LIMIT);
    let summary = ["command", "path", "pattern", "url"]
        .iter()
        .find_map(|key| some_text(&args[*key], SUMMARY_LIMIT))
        .or_else(|| title.clone())
        .unwrap_or_else(|| compact(&call["rawInput"], SUMMARY_LIMIT));
    // Why Cursor asks, such as "Not in allowlist: touch".
    let reason = call["content"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|item| some_text(&item["content"]["text"], DETAIL_LIMIT));
    let detail = reason.or_else(|| title.filter(|title| title != &summary));
    let diff_preview = if name == "editToolCall" {
        let path = args["path"].as_str().unwrap_or("");
        cursor_diff_preview(path, &args, &call["rawInput"], &call["content"]).or(known_diff)
    } else {
        None
    };
    let session_rule_key = cursor_session_rule_key(&name, &args, call, cwd);
    let session_option = session_rule_key.is_some();
    let rule_summary = session_option.then(|| CURSOR_SESSION_RULE_SUMMARY.to_owned());
    // Cursor offers allow-once, allow-always and reject-once. Allow-always is
    // persisted to ~/.cursor/cli-config.json by Cursor CLI, so Harness ALWAYS
    // maps AllowSession back to allow_once and caches the rule locally instead.
    let option = |kind: &str, fallback: &str| {
        params["options"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|option| option["kind"] == kind)
            .and_then(|option| option["optionId"].as_str())
            .unwrap_or(fallback)
            .to_owned()
    };
    let select = |option: String| json!({"jsonrpc":"2.0","id":id,"result":{"outcome":{"outcome":"selected","optionId":option}}});
    let allow_once_reply = select(option("allow_once", "allow-once"));
    Ask {
        public: PermissionAsk {
            id: format!("cursor-{}", request_id(id)),
            provider: "cursor".into(),
            tool,
            summary,
            detail,
            session_option,
            kind: kind.into(),
            diff_preview,
            rule_summary,
            session_rule_key,
            questions: None,
        },
        reply: AskReply::Static {
            allow: allow_once_reply.clone(),
            allow_session: session_option.then_some(allow_once_reply),
            deny: select(option("reject_once", "reject-once")),
        },
    }
}
fn cursor_question_ask(id: &Value, params: &Value) -> Option<Ask> {
    let mut parsed = parse_questions(&params["questions"], false);
    if parsed.is_empty() {
        let has_single = params["prompt"]
            .as_str()
            .is_some_and(|s| !s.trim().is_empty())
            || params["question"]
                .as_str()
                .is_some_and(|s| !s.trim().is_empty())
            || params["options"].as_array().is_some_and(|a| !a.is_empty());
        if has_single {
            parsed = parse_questions(&Value::Array(vec![params.clone()]), false);
        }
    }
    if parsed.is_empty() {
        return None;
    }
    let summary = question_summary(&parsed);
    let detail = some_text(&params["title"], DETAIL_LIMIT).filter(|d| d != &summary);
    let questions = Some(parsed.iter().map(|q| q.public.clone()).collect());
    let reply_id = id.clone();
    Some(Ask {
        public: PermissionAsk {
            id: format!("cursor-{}", request_id(id)),
            provider: "cursor".into(),
            tool: "Question".into(),
            summary,
            detail,
            session_option: false,
            kind: "question".into(),
            diff_preview: None,
            rule_summary: None,
            session_rule_key: None,
            questions,
        },
        reply: AskReply::Dynamic(Box::new(move |answer: &Answer| {
            if answer.decision == Decision::Deny {
                return json!({
                    "jsonrpc": "2.0",
                    "id": reply_id,
                    "result": {
                        "outcome": {
                            "outcome": "cancelled"
                        }
                    }
                });
            }
            let mut entries = Vec::new();
            for q in &parsed {
                let selected = extract_selected_for_question(answer.answers.as_ref(), q);
                if selected.is_empty() {
                    continue;
                }
                let mut selected_option_ids = Vec::new();
                let mut freeform_parts = Vec::new();
                for val in selected {
                    if let Some(matched) = q
                        .options
                        .iter()
                        .find(|opt| opt.id.as_deref() == Some(val.as_str()) || opt.label == val)
                    {
                        let opt_id = matched.id.as_ref().unwrap_or(&matched.label);
                        if !selected_option_ids
                            .iter()
                            .any(|existing| existing == opt_id)
                        {
                            selected_option_ids.push(opt_id.clone());
                        }
                    } else {
                        freeform_parts.push(val);
                    }
                }
                let mut entry = json!({
                    "questionId": q.id,
                    "selectedOptionIds": selected_option_ids,
                });
                if !freeform_parts.is_empty() {
                    entry["freeform"] = json!(freeform_parts.join(", "));
                }
                entries.push(entry);
            }
            json!({
                "jsonrpc": "2.0",
                "id": reply_id,
                "result": {
                    "outcome": {
                        "outcome": "answered",
                        "answers": entries,
                    }
                }
            })
        })),
    })
}
impl Driver for Cursor {
    fn answered(&mut self, ask: &str, allowed: bool) {
        if let Some(call) = self.asked.remove(ask)
            && !allowed
        {
            self.rejected.insert(call);
        }
    }
    fn opening(&mut self) -> Step {
        let initialize = self.request(
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false},
                "clientInfo": {"name": "infra-harness", "title": "AI Infrastructure Harness", "version": env!("CARGO_PKG_VERSION")},
            }),
        );
        Step {
            replies: vec![initialize],
            ..Step::default()
        }
    }
    fn receive(&mut self, message: Value) -> Step {
        let mut step = Step::default();
        if self.done {
            return step;
        }
        let method = message["method"].as_str();
        let id = message.get("id").filter(|id| !id.is_null());
        match (method, id) {
            (None, Some(id)) => {
                let Some(asked) = id.as_u64().and_then(|id| self.pending.remove(&id)) else {
                    return step;
                };
                if !message["error"].is_null() {
                    let reason = some_text(&message["error"]["message"], DETAIL_LIMIT)
                        .unwrap_or_else(|| compact(&message["error"], DETAIL_LIMIT));
                    if asked == "session/set_model" {
                        step.notices.push(json!({"kind":"status","text":format!("Cursor kept its own model: {reason}")}));
                        self.prompt(&mut step);
                    } else {
                        let hint = if asked == "initialize"
                            || asked == "session/new"
                            || asked == "session/load"
                        {
                            " If Cursor is not signed in, run `cursor-agent login` once."
                        } else {
                            ""
                        };
                        self.result(
                            &mut step,
                            false,
                            format!("Cursor refused {asked}: {reason}.{hint}"),
                        );
                    }
                    return step;
                }
                let result = &message["result"];
                match asked {
                    "initialize" => {
                        let cwd = self.setup.cwd.clone();
                        let request = match self.setup.resume.clone() {
                            Some(session) => self.request(
                                "session/load",
                                json!({"sessionId": session, "cwd": cwd, "mcpServers": []}),
                            ),
                            None => {
                                self.request("session/new", json!({"cwd": cwd, "mcpServers": []}))
                            }
                        };
                        step.replies.push(request);
                    }
                    "session/new" | "session/load" => {
                        let session = result["sessionId"]
                            .as_str()
                            .map(String::from)
                            .or_else(|| self.setup.resume.clone());
                        let Some(session) = session else {
                            self.result(&mut step, false, "Cursor named no session.".into());
                            return step;
                        };
                        self.session = Some(session.clone());
                        self.ready = true;
                        // Model ids carry their settings, such as
                        // `gpt-5.5[context=272k,reasoning=medium]`; the name is `gpt-5.5`.
                        let current = &result["models"]["currentModelId"];
                        let model = result["models"]["availableModels"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .find(|option| option["modelId"] == *current)
                            .map(|option| option["name"].clone())
                            .filter(Value::is_string)
                            .unwrap_or_else(|| current.clone());
                        step.events.push(json!({
                            "type": "system",
                            "subtype": "init",
                            "session_id": session,
                            "model": model,
                        }));
                        let wanted = self.setup.model.clone().filter(|model| {
                            result["models"]["currentModelId"] != model.as_str()
                                && result["models"]["availableModels"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .any(|option| option["modelId"] == model.as_str())
                        });
                        if let Some(model) = wanted {
                            let request = self.request(
                                "session/set_model",
                                json!({"sessionId": session, "modelId": model}),
                            );
                            step.replies.push(request);
                        } else {
                            self.prompt(&mut step);
                        }
                    }
                    "session/set_model" => self.prompt(&mut step),
                    "session/prompt" => {
                        let reason = result["stopReason"].as_str().unwrap_or("end_turn");
                        let ok = reason == "end_turn";
                        self.result(
                            &mut step,
                            ok,
                            format!("Cursor stopped the turn: {}.", reason.replace('_', " ")),
                        );
                    }
                    _ => {}
                }
            }
            (Some(method), Some(id)) => {
                if method == "session/request_permission" {
                    self.flush(&mut step);
                    let call = request_id(&message["params"]["toolCall"]["toolCallId"]);
                    let known = self.calls.get(&call).cloned();
                    let known_diff = self.diffs.get(&call).cloned();
                    let ask =
                        cursor_ask(id, &message["params"], known, known_diff, &self.setup.cwd);
                    self.asked.insert(ask.public.id.clone(), call);
                    step.asks.push(ask);
                } else if method == "cursor/ask_question"
                    && let Some(ask) = cursor_question_ask(id, &message["params"])
                {
                    if self.setup.mode == PermissionMode::Manual {
                        self.flush(&mut step);
                        step.asks.push(ask);
                    } else {
                        step.notices
                            .push(ask.public.resolved_event("declined", "auto"));
                        step.replies.push(ask.deny_reply());
                    }
                } else {
                    step.replies.push(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {"code": -32601, "message": "The Harness does not answer this request."},
                    }));
                }
            }
            (Some(method), None) => {
                let params = &message["params"];
                match method {
                    "session/update"
                        if self.ready
                            && params["sessionId"]
                                .as_str()
                                .is_none_or(|session| self.session.as_deref() == Some(session)) =>
                    {
                        self.update(&mut step, &params["update"]);
                    }
                    "cursor/update_todos" if self.ready => {
                        let todos: Vec<Value> = params["todos"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|todo| {
                                json!({"content": todo["content"].as_str().or_else(|| todo["text"].as_str()), "status": todo["status"]})
                            })
                            .collect();
                        self.plan(&mut step, todos);
                    }
                    _ => {}
                }
            }
            (None, None) => {}
        }
        step
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(mode: PermissionMode, resume: Option<&str>) -> Setup {
        Setup {
            mode,
            prompt: "Fix the bug".into(),
            cwd: "/work/project".into(),
            model: None,
            resume: resume.map(String::from),
            auto_asks: false,
        }
    }

    #[test]
    fn claude_sends_the_prompt_after_its_channel_opens() {
        let mut driver = claude(setup(PermissionMode::Manual, None));
        let opening = driver.opening();
        assert_eq!(opening.replies[0]["request"]["subtype"], "initialize");
        let step = driver.receive(json!({"type":"control_response","response":{"subtype":"success","request_id":CLAUDE_INIT,"response":{"current_permission_mode":"default"}}}));
        assert!(step.notices.is_empty());
        assert_eq!(step.replies.len(), 1);
        assert_eq!(step.replies[0]["type"], "user");
        assert_eq!(step.replies[0]["message"]["content"], "Fix the bug");
    }

    #[test]
    fn claude_auto_falls_back_to_accepting_edits_when_auto_mode_is_unavailable() {
        let initialized = json!({"type":"control_response","response":{"subtype":"success","request_id":CLAUDE_INIT,"response":{"current_permission_mode":"auto"}}});
        let mut driver = claude(setup(PermissionMode::Auto, None));
        driver.opening();
        // The mode reported at start is not trusted: auto mode is asked for.
        let step = driver.receive(initialized.clone());
        assert!(step.notices.is_empty());
        assert_eq!(step.replies[0]["request_id"], CLAUDE_AUTO);
        assert_eq!(step.replies[0]["request"]["mode"], "auto");
        let step = driver.receive(json!({"type":"control_response","response":{"subtype":"error","request_id":CLAUDE_AUTO,"error":"Cannot set permission mode to auto: auto mode unavailable for this model"}}));
        assert_eq!(step.replies[0]["request"]["mode"], "acceptEdits");
        let notice = step.notices[0]["text"].as_str().unwrap();
        assert!(
            notice.starts_with(
                "Auto mode is unavailable here (auto mode unavailable for this model)"
            ),
            "{notice}"
        );
        let step = driver.receive(json!({"type":"control_response","response":{"subtype":"success","request_id":CLAUDE_MODE,"response":{"mode":"acceptEdits"}}}));
        assert_eq!(step.replies[0]["type"], "user");
        // A later report of the default mode does not fall back twice.
        let step =
            driver.receive(json!({"type":"system","subtype":"status","permissionMode":"default"}));
        assert!(step.replies.is_empty() && step.notices.is_empty());

        // Where auto mode is granted, the prompt goes out at once …
        let mut driver = claude(setup(PermissionMode::Auto, None));
        driver.receive(initialized);
        let step = driver.receive(json!({"type":"control_response","response":{"subtype":"success","request_id":CLAUDE_AUTO,"response":{"mode":"auto"}}}));
        assert_eq!(step.replies[0]["type"], "user");
        assert!(step.notices.is_empty());
        // … and a later withdrawal of auto mode falls back then.
        let step =
            driver.receive(json!({"type":"system","subtype":"status","permissionMode":"default"}));
        assert_eq!(step.replies[0]["request"]["mode"], "acceptEdits");
        assert_eq!(step.events.len(), 1);
        // Manual does not ask for a mode: its command line names it.
        let mut manual = claude(setup(PermissionMode::Manual, None));
        let step = manual.receive(json!({"type":"control_response","response":{"subtype":"success","request_id":CLAUDE_INIT,"response":{"current_permission_mode":"default"}}}));
        assert_eq!(step.replies[0]["type"], "user");
    }

    #[test]
    fn claude_permission_requests_wait_for_a_person_in_manual_and_are_declined_in_auto() {
        let request = json!({
            "type": "control_request",
            "request_id": "req-1",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "Bash",
                "display_name": "Bash",
                "input": {"command": "touch probe.txt", "description": "Create a file"},
                "description": "Create a file",
                "tool_use_id": "toolu_1",
                "permission_suggestions": [{"type":"addRules","rules":[{"toolName":"Bash","ruleContent":"touch probe.txt"}],"behavior":"allow","destination":"localSettings"}],
            },
        });
        let mut manual = claude(setup(PermissionMode::Manual, None));
        let step = manual.receive(request.clone());
        assert!(step.replies.is_empty());
        let ask = &step.asks[0];
        assert_eq!(ask.public.id, "claude-req-1");
        assert_eq!(ask.public.tool, "Bash");
        assert_eq!(ask.public.kind, "command");
        assert_eq!(ask.public.summary, "touch probe.txt");
        assert_eq!(ask.public.detail.as_deref(), Some("Create a file"));
        assert_eq!(
            ask.public.rule_summary.as_deref(),
            Some("Session rule: Bash(touch probe.txt)")
        );
        assert!(ask.public.session_option);
        assert_eq!(ask.allow_reply()["response"]["request_id"], "req-1");
        assert_eq!(
            ask.allow_reply()["response"]["response"]["behavior"],
            "allow"
        );
        assert_eq!(
            ask.allow_reply()["response"]["response"]["updatedInput"]["command"],
            "touch probe.txt"
        );
        let session = ask.allow_session_reply().unwrap();
        // The approval stays in the session; nothing is written to the project.
        assert_eq!(
            session["response"]["response"]["updatedPermissions"][0]["destination"],
            "session"
        );
        assert_eq!(ask.deny_reply()["response"]["response"]["behavior"], "deny");

        let mut auto = claude(setup(PermissionMode::Auto, None));
        let step = auto.receive(request.clone());
        assert!(step.asks.is_empty());
        assert_eq!(step.replies[0]["response"]["response"]["behavior"], "deny");
        assert_eq!(step.notices[0]["state"], "declined");
        assert_eq!(step.notices[0]["by"], "auto");

        // When `auto_asks` is enabled (e.g. SystemBoundaryApprovals), Claude forwards
        // `can_use_tool` to `step.asks` instead of declining on its own.
        let mut auto_with_asks = claude(Setup {
            auto_asks: true,
            ..setup(PermissionMode::Auto, None)
        });
        let step = auto_with_asks.receive(request);
        assert!(step.replies.is_empty());
        assert!(step.notices.is_empty());
        assert_eq!(step.asks.len(), 1);
        assert_eq!(step.asks[0].public.id, "claude-req-1");
        assert_eq!(step.asks[0].public.summary, "touch probe.txt");
        assert_eq!(
            step.asks[0].allow_reply()["response"]["response"]["behavior"],
            "allow"
        );
    }

    #[test]
    fn claude_edit_multiedit_write_and_rule_summaries_enrich_permission_asks() {
        let mut driver = claude(setup(PermissionMode::Manual, None));

        // Edit
        let step = driver.receive(json!({
            "type": "control_request",
            "request_id": "edit-1",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "Edit",
                "input": {
                    "file_path": "src/lib.rs",
                    "old_string": "fn one() -> i32 {\n    1\n}",
                    "new_string": "fn one() -> i32 {\n    2\n}"
                },
                "permission_suggestions": [
                    {"type": "addDirectories", "directories": ["src"], "destination": "localSettings"},
                    {"type": "setMode", "mode": "acceptEdits", "destination": "localSettings"}
                ]
            }
        }));
        let edit_ask = &step.asks[0];
        assert_eq!(edit_ask.public.kind, "file_change");
        let diff = edit_ask.public.diff_preview.as_deref().unwrap();
        assert!(diff.contains("--- a/src/lib.rs"), "{diff}");
        assert!(diff.contains("-    1\n+    2"), "{diff}");
        assert_eq!(
            edit_ask.public.rule_summary.as_deref(),
            Some("Session rule: dir(src), mode(acceptEdits)")
        );

        // MultiEdit
        let step = driver.receive(json!({
            "type": "control_request",
            "request_id": "edit-2",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "MultiEdit",
                "input": {
                    "file_path": "src/main.rs",
                    "edits": [
                        {"old_string": "let a = 1;", "new_string": "let a = 2;"},
                        {"old_string": "let b = 3;", "new_string": "let b = 4;"}
                    ]
                }
            }
        }));
        let multi_ask = &step.asks[0];
        assert_eq!(multi_ask.public.kind, "file_change");
        let multi_diff = multi_ask.public.diff_preview.as_deref().unwrap();
        assert!(
            multi_diff.contains("-let a = 1;\n+let a = 2;"),
            "{multi_diff}"
        );
        assert!(
            multi_diff.contains("-let b = 3;\n+let b = 4;"),
            "{multi_diff}"
        );

        // Write (oversized content bounded <= 6144)
        let huge = "line\n".repeat(3000);
        let step = driver.receive(json!({
            "type": "control_request",
            "request_id": "write-1",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "Write",
                "input": {
                    "file_path": "big.txt",
                    "content": huge
                }
            }
        }));
        let write_ask = &step.asks[0];
        assert_eq!(write_ask.public.kind, "file_change");
        let write_diff = write_ask.public.diff_preview.as_deref().unwrap();
        assert!(write_diff.starts_with("--- /dev/null\n+++ b/big.txt\n"));
        assert!(write_diff.contains("+line"));
        assert_eq!(
            write_diff.chars().count(),
            crate::permission::DIFF_PREVIEW_LIMIT
        );

        // WebFetch -> kind "tool"
        let step = driver.receive(json!({
            "type": "control_request",
            "request_id": "fetch-1",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "WebFetch",
                "input": {"url": "https://example.com"}
            }
        }));
        assert_eq!(step.asks[0].public.kind, "tool");
        assert_eq!(step.asks[0].public.diff_preview, None);
    }

    #[test]
    fn claude_ask_user_question_routes_questions_and_formats_updated_input() {
        let request = json!({
            "type": "control_request",
            "request_id": "ask-q-1",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "AskUserQuestion",
                "input": {
                    "questions": [
                        {
                            "header": "Database",
                            "question": "Which database should we use?",
                            "multiSelect": false,
                            "options": [
                                {"label": "PostgreSQL", "description": "Relational DB"},
                                {"label": "SQLite", "description": "Embedded DB"}
                            ]
                        },
                        {
                            "header": "Features",
                            "question": "Which features?",
                            "multiSelect": true,
                            "options": [
                                {"label": "Auth", "description": "JWT auth"},
                                {"label": "Audit", "description": "Audit log"}
                            ]
                        }
                    ]
                }
            }
        });
        let mut manual = claude(setup(PermissionMode::Manual, None));
        let step = manual.receive(request.clone());
        let ask = &step.asks[0];
        assert_eq!(ask.public.kind, "question");
        assert!(!ask.public.session_option);
        assert_eq!(ask.public.rule_summary, None);
        let qs = ask.public.questions.as_ref().unwrap();
        assert_eq!(qs.len(), 2);
        assert_eq!(qs[0]["header"], "Database");
        assert_eq!(qs[0]["multi_select"], false);
        assert_eq!(qs[0]["options"][0]["label"], "PostgreSQL");
        assert_eq!(qs[1]["multi_select"], true);

        let reply = ask.reply_for(&Answer {
            decision: Decision::Allow,
            answers: Some(json!({
                "Which database should we use?": "PostgreSQL",
                "Which features?": ["Auth", "Audit"]
            })),
        });
        let updated = &reply["response"]["response"]["updatedInput"];
        assert_eq!(updated["questions"].as_array().unwrap().len(), 2);
        assert_eq!(
            updated["answers"]["Which database should we use?"],
            "PostgreSQL"
        );
        assert_eq!(updated["answers"]["Which features?"], "Auth, Audit");
        assert_eq!(ask.deny_reply()["response"]["response"]["behavior"], "deny");

        let mut auto = claude(setup(PermissionMode::Auto, None));
        let step = auto.receive(request);
        assert!(step.asks.is_empty());
        assert_eq!(step.replies[0]["response"]["response"]["behavior"], "deny");
    }

    #[test]
    fn claude_closes_its_input_after_the_result_and_refuses_unknown_control_requests() {
        let mut driver = claude(setup(PermissionMode::Manual, None));
        let step = driver.receive(json!({"type":"control_request","request_id":"x","request":{"subtype":"hook_callback"}}));
        assert_eq!(step.replies[0]["response"]["subtype"], "error");
        let step = driver.receive(json!({"type":"control_cancel_request","request_id":"req-1"}));
        assert_eq!(step.withdrawn, vec!["claude-req-1".to_owned()]);
        let step = driver.receive(json!({"type":"assistant","message":{"content":[]}}));
        assert_eq!(step.events.len(), 1);
        let step = driver.receive(json!({"type":"result","subtype":"success","is_error":false}));
        assert!(step.close_input);
        assert!(!step.finished);
    }

    fn respond(driver: &mut Box<dyn Driver>, request: &Value, result: Value) -> Step {
        driver.receive(json!({"id": request["id"], "result": result}))
    }

    #[test]
    fn codex_starts_a_read_only_thread_that_asks_and_reads_like_exec() {
        let mut driver = codex(setup(PermissionMode::Manual, None));
        let opening = driver.opening();
        let initialize = &opening.replies[0];
        assert_eq!(initialize["method"], "initialize");
        let step = respond(&mut driver, initialize, json!({"userAgent":"codex"}));
        assert_eq!(step.replies[0]["method"], "initialized");
        let start = &step.replies[1];
        assert_eq!(start["method"], "thread/start");
        assert_eq!(start["params"]["sandbox"], "read-only");
        assert_eq!(start["params"]["approvalPolicy"], "on-request");
        assert_eq!(start["params"]["approvalsReviewer"], "user");
        let step = respond(&mut driver, start, json!({"thread":{"id":"0199-thread"}}));
        assert_eq!(
            step.events[0],
            json!({"type":"thread.started","thread_id":"0199-thread"})
        );
        let turn = &step.replies[0];
        assert_eq!(turn["method"], "turn/start");
        assert_eq!(turn["params"]["input"][0]["text"], "Fix the bug");
        let step = respond(&mut driver, turn, json!({"turn":{"id":"turn-1"}}));
        assert_eq!(step.events[0]["type"], "turn.started");

        let step = driver.receive(json!({"method":"item/started","params":{"threadId":"0199-thread","turnId":"turn-1","item":{"type":"fileChange","id":"item-3","status":"inProgress","changes":[{"path":"src/a.rs","kind":{"type":"update"},"diff":"@@"}]}}}));
        assert_eq!(step.events[0]["type"], "item.started");
        assert_eq!(step.events[0]["item"]["type"], "file_change");
        assert_eq!(step.events[0]["item"]["status"], "in_progress");
        assert_eq!(
            step.events[0]["item"]["changes"][0],
            json!({"path":"src/a.rs","kind":"update"})
        );
        let step = driver.receive(json!({"id":7,"method":"item/fileChange/requestApproval","params":{"threadId":"0199-thread","turnId":"turn-1","itemId":"item-3","startedAtMs":1}}));
        let ask = &step.asks[0];
        assert_eq!(ask.public.id, "codex-7");
        assert_eq!(ask.public.kind, "file_change");
        assert_eq!(ask.public.summary, "update src/a.rs");
        assert_eq!(
            ask.public.diff_preview.as_deref(),
            Some("--- a/src/a.rs\n+++ b/src/a.rs\n@@")
        );
        assert_eq!(
            ask.public.rule_summary.as_deref(),
            Some("Remember approval for this Codex thread")
        );
        assert_eq!(
            ask.allow_reply(),
            json!({"id":7,"result":{"decision":"accept"}})
        );
        assert_eq!(
            ask.allow_session_reply().unwrap()["result"]["decision"],
            "acceptForSession"
        );
        assert_eq!(ask.deny_reply()["result"]["decision"], "decline");
        let step = driver.receive(json!({"id":8,"method":"item/commandExecution/requestApproval","params":{"threadId":"0199-thread","turnId":"turn-1","itemId":"item-4","command":"cargo test","cwd":"/work/project","reason":"Run the tests","startedAtMs":2}}));
        assert_eq!(step.asks[0].public.kind, "command");
        assert_eq!(step.asks[0].public.summary, "cargo test");
        assert_eq!(
            step.asks[0].public.detail.as_deref(),
            Some("Run the tests · in /work/project")
        );
        assert_eq!(
            step.asks[0].public.rule_summary.as_deref(),
            Some("Remember approval for this Codex thread")
        );

        driver.receive(json!({"method":"thread/tokenUsage/updated","params":{"threadId":"0199-thread","turnId":"turn-1","tokenUsage":{"total":{"inputTokens":150,"cachedInputTokens":20,"outputTokens":30,"reasoningOutputTokens":5,"totalTokens":180},"last":{"inputTokens":50,"cachedInputTokens":0,"outputTokens":10,"reasoningOutputTokens":5,"totalTokens":60}}}}));
        driver.receive(json!({"method":"thread/tokenUsage/updated","params":{"threadId":"0199-thread","turnId":"turn-1","tokenUsage":{"total":{"inputTokens":220,"cachedInputTokens":40,"outputTokens":45,"reasoningOutputTokens":9,"totalTokens":265},"last":{"inputTokens":70,"cachedInputTokens":20,"outputTokens":15,"reasoningOutputTokens":4,"totalTokens":85}}}}));
        let step = driver.receive(json!({"method":"item/completed","params":{"threadId":"0199-thread","turnId":"turn-1","item":{"type":"agentMessage","id":"item-5","text":"Done."}}}));
        assert_eq!(
            step.events[0]["item"],
            json!({"id":"item-5","type":"agent_message","text":"Done."})
        );
        let step = driver.receive(json!({"method":"turn/completed","params":{"threadId":"0199-thread","turn":{"id":"turn-1","status":"completed"}}}));
        assert_eq!(step.events[0]["type"], "turn.completed");
        // The previous turns' 100 input tokens are not this turn's.
        assert_eq!(step.events[0]["usage"]["input_tokens"], 120);
        assert_eq!(step.events[0]["usage"]["cached_input_tokens"], 20);
        assert_eq!(step.events[0]["usage"]["reasoning_output_tokens"], 9);
        assert_eq!(step.events[0]["usage"]["output_tokens"], 25);
        assert!(step.finished && step.close_input);
    }

    #[test]
    fn codex_resumes_the_thread_and_reports_a_refusal_as_a_failed_turn() {
        let mut driver = codex(setup(PermissionMode::Manual, Some("0199-thread")));
        let opening = driver.opening();
        let step = respond(&mut driver, &opening.replies[0], json!({}));
        let resume = &step.replies[1];
        assert_eq!(resume["method"], "thread/resume");
        assert_eq!(resume["params"]["threadId"], "0199-thread");
        let step = driver.receive(
            json!({"id": resume["id"], "error": {"code": -32600, "message": "thread not found"}}),
        );
        assert_eq!(step.events[0]["type"], "turn.failed");
        assert!(
            step.events[0]["error"]["message"]
                .as_str()
                .unwrap()
                .contains("thread not found")
        );
        assert!(step.finished);
        // Requests it cannot answer get a protocol answer, never silence.
        let mut driver = codex(setup(PermissionMode::Manual, None));
        let step = driver.receive(json!({"id": 3, "method": "item/tool/call", "params": {}}));
        assert_eq!(step.replies[0]["error"]["code"], -32601);
        let step = driver
            .receive(json!({"id": 4, "method": "mcpServer/elicitation/request", "params": {}}));
        assert_eq!(step.replies[0]["result"]["action"], "decline");
    }

    #[test]
    fn codex_permissions_and_request_user_input_route_rich_asks() {
        let mut driver = codex(setup(PermissionMode::Manual, None));
        let step = driver.receive(json!({
            "id": 10,
            "method": "item/permissions/requestApproval",
            "params": {
                "permissions": {
                    "fileSystem": {"write": ["/tmp/out"]},
                    "network": {"enabled": true}
                }
            }
        }));
        assert_eq!(step.asks[0].public.kind, "permissions");
        assert_eq!(
            step.asks[0].public.rule_summary.as_deref(),
            Some("Remember approval for this Codex thread")
        );

        // Multi-file oversized diff is bounded <= 6144 chars.
        let big_patch = format!("@@ -1,1 +1,1 @@\n{}", "+z".repeat(4000));
        driver.receive(json!({
            "method": "item/started",
            "params": {
                "item": {
                    "type": "fileChange",
                    "id": "item-big",
                    "status": "inProgress",
                    "changes": [
                        {"path": "src/one.rs", "kind": {"type": "update"}, "diff": big_patch},
                        {"path": "src/two.rs", "kind": {"type": "add"}, "diff": "@@ -0,0 +1,1 @@\n+hi"}
                    ]
                }
            }
        }));
        let big_step = driver.receive(json!({
            "id": 12,
            "method": "item/fileChange/requestApproval",
            "params": {"itemId": "item-big"}
        }));
        let big_diff = big_step.asks[0].public.diff_preview.as_deref().unwrap();
        assert!(big_diff.starts_with("--- a/src/one.rs\n+++ b/src/one.rs\n"));
        assert_eq!(
            big_diff.chars().count(),
            crate::permission::DIFF_PREVIEW_LIMIT
        );

        let user_input = json!({
            "id": 11,
            "method": "item/tool/requestUserInput",
            "params": {
                "questions": [
                    {
                        "id": "db",
                        "header": "Database",
                        "question": "Which database?",
                        "isOther": true,
                        "options": [
                            {"label": "PostgreSQL", "description": "Full SQL"},
                            {"label": "SQLite", "description": "File DB"}
                        ]
                    },
                    {
                        "id": "flags",
                        "header": "Flags",
                        "question": "Select flags",
                        "multiSelect": true,
                        "isOther": false,
                        "options": [
                            {"label": "Verbose", "description": "More logs"},
                            {"label": "Strict", "description": "Fail fast"}
                        ]
                    }
                ]
            }
        });
        let step = driver.receive(user_input.clone());
        let ask = &step.asks[0];
        assert_eq!(ask.public.kind, "question");
        assert!(!ask.public.session_option);
        let qs = ask.public.questions.as_ref().unwrap();
        assert_eq!(qs.len(), 2);
        assert_eq!(qs[0]["id"], "db");
        assert_eq!(qs[0]["is_other"], true);
        assert_eq!(qs[1]["multi_select"], true);
        assert_eq!(qs[1]["is_other"], false);

        let reply = ask.reply_for(&Answer {
            decision: Decision::Allow,
            answers: Some(json!({
                "db": "PostgreSQL",
                "flags": ["Verbose", "Strict"]
            })),
        });
        assert_eq!(
            reply,
            json!({
                "id": 11,
                "result": {
                    "answers": {
                        "db": {"answers": ["PostgreSQL"]},
                        "flags": {"answers": ["Verbose", "Strict"]}
                    }
                }
            })
        );
        assert_eq!(
            ask.deny_reply(),
            json!({"id": 11, "result": {"answers": {}}})
        );
        assert_eq!(
            ask.reply_for(&Answer::from_decision(Decision::Allow)),
            json!({"id": 11, "result": {"answers": {}}})
        );

        let mut auto = codex(setup(PermissionMode::Auto, None));
        let step = auto.receive(user_input);
        assert!(step.asks.is_empty());
        assert_eq!(
            step.replies[0],
            json!({"id": 11, "result": {"answers": {}}})
        );
    }

    #[test]
    fn cursor_prompts_over_acp_and_reads_like_print_mode() {
        let mut driver = cursor(setup(PermissionMode::Manual, None));
        let opening = driver.opening();
        assert_eq!(opening.replies[0]["method"], "initialize");
        assert_eq!(opening.replies[0]["jsonrpc"], "2.0");
        let step = respond(
            &mut driver,
            &opening.replies[0],
            json!({"protocolVersion":1}),
        );
        let new = &step.replies[0];
        assert_eq!(new["method"], "session/new");
        assert_eq!(new["params"]["cwd"], "/work/project");
        // As Cursor 2026.11 answers: model ids carry their settings.
        let step = respond(
            &mut driver,
            new,
            json!({"sessionId":"chat-1","models":{"currentModelId":"gpt-5.5[context=272k,reasoning=medium]","availableModels":[{"modelId":"default[]","name":"Auto"},{"modelId":"gpt-5.5[context=272k,reasoning=medium]","name":"gpt-5.5"}]}}),
        );
        assert_eq!(step.events[0]["type"], "system");
        assert_eq!(step.events[0]["session_id"], "chat-1");
        assert_eq!(step.events[0]["model"], "gpt-5.5");
        let prompt = step.replies[0].clone();
        assert_eq!(prompt["method"], "session/prompt");
        assert_eq!(prompt["params"]["prompt"][0]["text"], "Fix the bug");

        driver.receive(json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"chat-1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Running "}}}}));
        driver.receive(json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"chat-1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"the tests."}}}}));
        let step = driver.receive(json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"chat-1","update":{"sessionUpdate":"tool_call","toolCallId":"call-1","title":"`cargo test`","kind":"execute","status":"pending","rawInput":{"command":"cargo test"}}}}));
        assert_eq!(step.events[0]["type"], "assistant");
        assert_eq!(
            step.events[0]["message"]["content"][0]["text"],
            "Running the tests."
        );
        assert_eq!(step.events[1]["type"], "tool_call");
        assert_eq!(step.events[1]["subtype"], "started");
        assert_eq!(
            step.events[1]["tool_call"]["shellToolCall"]["args"]["command"],
            "cargo test"
        );
        driver.receive(json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"chat-1","update":{"sessionUpdate":"tool_call_update","toolCallId":"call-1","status":"in_progress"}}}));
        // The request names the call by its title and says why it asks; the
        // input came with the call's start.
        let step = driver.receive(json!({"jsonrpc":"2.0","id":0,"method":"session/request_permission","params":{"sessionId":"chat-1","toolCall":{"toolCallId":"call-1","title":"`cargo test`","kind":"execute","status":"pending","content":[{"type":"content","content":{"type":"text","text":"Not in allowlist: cargo"}}]},"options":[{"optionId":"allow-once","name":"Allow once","kind":"allow_once"},{"optionId":"allow-always","name":"Allow always","kind":"allow_always"},{"optionId":"reject-once","name":"Reject","kind":"reject_once"}]}}));
        let ask = &step.asks[0];
        assert_eq!(ask.public.id, "cursor-0");
        assert_eq!(ask.public.tool, "Shell");
        assert_eq!(ask.public.kind, "command");
        assert_eq!(ask.public.summary, "cargo test");
        assert_eq!(
            ask.public.detail.as_deref(),
            Some("Not in allowlist: cargo")
        );
        assert!(ask.public.session_option);
        assert_eq!(
            ask.public.session_rule_key.as_deref(),
            Some("cursor:shell:cargo test")
        );
        assert_eq!(
            ask.public.rule_summary.as_deref(),
            Some(CURSOR_SESSION_RULE_SUMMARY)
        );
        assert_eq!(
            ask.allow_reply()["result"]["outcome"]["optionId"],
            "allow-once"
        );
        assert_eq!(
            ask.allow_session_reply().unwrap()["result"]["outcome"]["optionId"],
            "allow-once"
        );
        assert_eq!(
            ask.deny_reply()["result"]["outcome"]["optionId"],
            "reject-once"
        );
        // A request for a call Cursor never announced reads its title.
        let step = driver.receive(json!({"jsonrpc":"2.0","id":1,"method":"session/request_permission","params":{"sessionId":"chat-1","toolCall":{"toolCallId":"call-9","title":"`rm -r build`","kind":"execute"},"options":[]}}));
        assert_eq!(step.asks[0].public.summary, "rm -r build");
        assert_eq!(step.asks[0].public.detail, None);
        driver.answered("cursor-0", true);
        driver.answered("cursor-1", false);
        let step = driver.receive(json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"chat-1","update":{"sessionUpdate":"tool_call_update","toolCallId":"call-1","status":"completed","rawOutput":{"exitCode":0,"stdout":"","stderr":""}}}}));
        assert_eq!(step.events[0]["subtype"], "completed");
        assert_eq!(
            step.events[0]["tool_call"]["shellToolCall"]["result"],
            json!({"success":{}})
        );
        // Cursor reports a declined call completed; it never ran.
        let step = driver.receive(json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"chat-1","update":{"sessionUpdate":"tool_call_update","toolCallId":"call-9","status":"completed"}}}));
        let declined = step.events[0]["tool_call"]
            .as_object()
            .and_then(|call| call.values().next())
            .unwrap();
        assert_eq!(declined["result"]["rejected"]["reason"], DECLINED_BY_PERSON);
        driver.receive(json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"chat-1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"All green."}}}}));
        let step = respond(&mut driver, &prompt, json!({"stopReason":"end_turn"}));
        let result = step
            .events
            .iter()
            .find(|event| event["type"] == "result")
            .unwrap();
        assert_eq!(result["subtype"], "success");
        assert_eq!(result["is_error"], false);
        assert_eq!(result["result"], "Running the tests.\n\nAll green.");
        assert!(step.finished);
    }

    #[test]
    fn cursor_skips_the_replayed_history_of_a_loaded_session() {
        let mut driver = cursor(setup(PermissionMode::Manual, Some("chat-9")));
        let opening = driver.opening();
        let step = respond(&mut driver, &opening.replies[0], json!({}));
        let load = step.replies[0].clone();
        assert_eq!(load["method"], "session/load");
        assert_eq!(load["params"]["sessionId"], "chat-9");
        let step = driver.receive(json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"chat-9","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"old answer"}}}}));
        assert!(step.events.is_empty());
        let step = respond(&mut driver, &load, Value::Null);
        assert_eq!(step.events[0]["session_id"], "chat-9");
        let prompt = step.replies[0].clone();
        let step = respond(&mut driver, &prompt, json!({"stopReason":"cancelled"}));
        let result = step
            .events
            .iter()
            .find(|event| event["type"] == "result")
            .unwrap();
        assert_eq!(result["is_error"], true);
        assert_eq!(result["result"], "Cursor stopped the turn: cancelled.");
        let mut driver = cursor(setup(PermissionMode::Manual, None));
        let step = driver
            .receive(json!({"jsonrpc":"2.0","id":5,"method":"cursor/ask_question","params":{}}));
        assert_eq!(step.replies[0]["error"]["code"], -32601);
    }

    #[test]
    fn cursor_edit_diff_preview_and_strict_session_rule_keys() {
        // The workspace is absolute on this host. `/work/project` names no
        // drive, so on Windows nothing can be told to lie inside it.
        let workspace = if cfg!(windows) {
            "C:\\work\\project"
        } else {
            "/work/project"
        };
        let mut driver = cursor(Setup {
            cwd: workspace.into(),
            ..setup(PermissionMode::Manual, None)
        });
        let init = driver.opening().replies[0].clone();
        respond(&mut driver, &init, json!({"protocolVersion": 1}));
        let new_req = json!({"id": 2});
        respond(&mut driver, &new_req, json!({"sessionId": "chat-2"}));

        // 1. Diff preview captured from session/update tool_call rawInput and surfaced in request_permission
        driver.receive(json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "chat-2",
                "update": {
                    "sessionUpdate": "tool_call",
                    "toolCallId": "edit-1",
                    "title": "Edit src/lib.rs",
                    "kind": "edit",
                    "status": "pending",
                    "rawInput": {
                        "path": "./src/./lib.rs",
                        "oldText": "pub fn x() -> i32 {\n    1\n}",
                        "newText": "pub fn x() -> i32 {\n    2\n}"
                    }
                }
            }
        }));
        let step = driver.receive(json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "session/request_permission",
            "params": {
                "sessionId": "chat-2",
                "toolCall": {"toolCallId": "edit-1", "title": "Edit src/lib.rs", "kind": "edit"},
                "options": [
                    {"optionId": "allow-once", "kind": "allow_once"},
                    {"optionId": "allow-always", "kind": "allow_always"},
                    {"optionId": "reject-once", "kind": "reject_once"}
                ]
            }
        }));
        let edit_ask = &step.asks[0];
        assert_eq!(edit_ask.public.kind, "file_change");
        let diff = edit_ask.public.diff_preview.as_deref().unwrap();
        assert!(diff.contains("--- a/src/lib.rs"), "{diff}");
        assert!(diff.contains("-    1\n+    2"), "{diff}");
        assert!(edit_ask.public.session_option);
        assert_eq!(
            edit_ask.public.session_rule_key.as_deref(),
            Some("cursor:edit:src/lib.rs")
        );
        assert_eq!(
            edit_ask.public.rule_summary.as_deref(),
            Some(CURSOR_SESSION_RULE_SUMMARY)
        );
        // Never sends allow-always to Cursor CLI!
        assert_eq!(
            edit_ask.allow_session_reply().unwrap()["result"]["outcome"]["optionId"],
            "allow-once"
        );

        // 2. Workspace-contained absolute path normalizes to canonical relative key;
        //    content diff block is also extracted.
        let step = driver.receive(json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "session/request_permission",
            "params": {
                "sessionId": "chat-2",
                "toolCall": {
                    "toolCallId": "edit-2",
                    "kind": "edit",
                    "locations": [{"path": format!("{workspace}/src/b.rs")}],
                    "content": [{"type": "diff", "path": "src/b.rs", "diff": "@@ -1 +1 @@\n-old\n+new"}]
                }
            }
        }));
        assert_eq!(
            step.asks[0].public.session_rule_key.as_deref(),
            Some("cursor:edit:src/b.rs")
        );
        assert!(
            step.asks[0]
                .public
                .diff_preview
                .as_deref()
                .unwrap()
                .contains("-old\n+new")
        );

        // 3. Unsafe paths & non-path tools must NOT get session_rule_key or session_option.
        let sibling = format!("{workspace}-other/src/a.rs");
        for bad_path in [
            "../secret.txt",
            "src/../../secret.txt",
            "/etc/passwd",
            sibling.as_str(),
            workspace,
            ".",
            "~/notes.md",
        ] {
            let step = driver.receive(json!({
                "jsonrpc": "2.0",
                "id": 12,
                "method": "session/request_permission",
                "params": {
                    "toolCall": {
                        "toolCallId": "bad-path",
                        "kind": "edit",
                        "rawInput": {"path": bad_path}
                    }
                }
            }));
            let ask = &step.asks[0];
            assert_eq!(ask.public.session_rule_key, None, "{bad_path}");
            assert!(!ask.public.session_option, "{bad_path}");
            assert_eq!(ask.public.rule_summary, None, "{bad_path}");
            assert_eq!(ask.allow_session_reply(), None, "{bad_path}");
        }

        // 4. Chained/unsafe shell commands must NOT get session_rule_key or session_option.
        for bad_cmd in [
            "cargo test; rm -rf /",
            "cargo test | cat",
            "cargo test &",
            "cargo test && ls",
            "cargo test || ls",
            "echo `whoami`",
            "echo $HOME",
            "echo x > out.txt",
            "cat < in.txt",
            "echo a\nb",
            "echo a\rb",
        ] {
            let step = driver.receive(json!({
                "jsonrpc": "2.0",
                "id": 13,
                "method": "session/request_permission",
                "params": {
                    "toolCall": {
                        "toolCallId": "bad-cmd",
                        "kind": "execute",
                        "rawInput": {"command": bad_cmd}
                    }
                }
            }));
            let ask = &step.asks[0];
            assert_eq!(ask.public.session_rule_key, None, "{bad_cmd:?}");
            assert!(!ask.public.session_option, "{bad_cmd:?}");
            assert_eq!(ask.allow_session_reply(), None, "{bad_cmd:?}");
        }

        // 5. readToolCall & deleteToolCall work on clean workspace-relative paths; grep/fetch do not.
        for (kind, path, expected) in [
            ("read", "README.md", Some("cursor:read:README.md")),
            ("delete", "tmp/old.txt", Some("cursor:delete:tmp/old.txt")),
            ("search", "src", None),
        ] {
            let step = driver.receive(json!({
                "jsonrpc": "2.0",
                "id": 14,
                "method": "session/request_permission",
                "params": {
                    "toolCall": {
                        "toolCallId": "t-14",
                        "kind": kind,
                        "rawInput": {"path": path, "pattern": "foo"}
                    }
                }
            }));
            assert_eq!(
                step.asks[0].public.session_rule_key.as_deref(),
                expected,
                "{kind}"
            );
            assert_eq!(step.asks[0].public.session_option, expected.is_some());
        }
    }

    #[test]
    fn cursor_ask_question_routes_questions_and_formats_acp_reply() {
        let mut driver = cursor(setup(PermissionMode::Manual, None));
        let step = driver.receive(json!({
            "jsonrpc": "2.0",
            "id": 21,
            "method": "cursor/ask_question",
            "params": {
                "toolCallId": "call-q",
                "title": "Configuration",
                "questions": [
                    {
                        "id": "q1",
                        "prompt": "Pick runtime",
                        "allowMultiple": false,
                        "options": [
                            {"id": "opt_tokio", "label": "Tokio"},
                            {"id": "opt_async", "label": "Async-std"}
                        ]
                    },
                    {
                        "id": "q2",
                        "prompt": "Extra flags?",
                        "allowMultiple": true,
                        "options": [
                            {"id": "opt_a", "label": "Flag A"}
                        ]
                    }
                ]
            }
        }));
        let ask = &step.asks[0];
        assert_eq!(ask.public.kind, "question");
        assert!(!ask.public.session_option);
        assert_eq!(ask.public.questions.as_ref().unwrap().len(), 2);

        let reply = ask.reply_for(&Answer {
            decision: Decision::Allow,
            answers: Some(json!({
                "q1": "Tokio",
                "q2": ["opt_a", "custom-flag"]
            })),
        });
        assert_eq!(
            reply,
            json!({
                "jsonrpc": "2.0",
                "id": 21,
                "result": {
                    "outcome": {
                        "outcome": "answered",
                        "answers": [
                            {"questionId": "q1", "selectedOptionIds": ["opt_tokio"]},
                            {"questionId": "q2", "selectedOptionIds": ["opt_a"], "freeform": "custom-flag"}
                        ]
                    }
                }
            })
        );
        assert_eq!(
            ask.deny_reply(),
            json!({
                "jsonrpc": "2.0",
                "id": 21,
                "result": {
                    "outcome": {
                        "outcome": "cancelled"
                    }
                }
            })
        );

        // Single-question params form (prompt + options) also works.
        let step = driver.receive(json!({
            "jsonrpc": "2.0",
            "id": 22,
            "method": "cursor/ask_question",
            "params": {
                "prompt": "Proceed?",
                "options": ["Yes", "No"]
            }
        }));
        assert_eq!(step.asks[0].public.kind, "question");
        assert_eq!(step.asks[0].public.summary, "Proceed?");
    }
}
