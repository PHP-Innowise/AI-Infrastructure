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

fn reads_one_file(command: &str) -> bool {
    static CAT: OnceLock<Regex> = OnceLock::new();
    CAT.get_or_init(|| {
        Regex::new(r#"^\s*(?:(?:/usr)?/bin/)?(?:bash|sh|zsh)?\s*(?:-lc\s+)?["']?\s*(?:cat|type|Get-Content)\s+[^|;&<>]+["']?\s*$"#)
            .expect("a fixed expression")
    })
    .is_match(command)
}

fn skill_paths(text: &str) -> Vec<String> {
    static PATH: OnceLock<Regex> = OnceLock::new();
    PATH.get_or_init(|| {
        Regex::new(r#"[A-Za-z0-9_.~:/\\-]*[/\\]SKILL\.md"#).expect("a fixed expression")
    })
    .find_iter(text)
    .map(|found| found.as_str().replace('\\', "/"))
    .collect()
}

/// Pairs a provider's events into steps as they arrive.
pub struct Recorder<'a> {
    provider: String,
    redactor: &'a dyn Redactor,
    steps: Vec<Map<String, Value>>,
    /// Open tool calls: the provider's call identity to the step's index.
    open: BTreeMap<String, usize>,
    skills: Vec<SkillRef>,
    counters: Counters,
    bytes: usize,
}

impl<'a> Recorder<'a> {
    pub fn new(provider: &str, redactor: &'a dyn Redactor) -> Self {
        Self {
            provider: provider.into(),
            redactor,
            steps: Vec::new(),
            open: BTreeMap::new(),
            skills: Vec::new(),
            counters: Counters::default(),
            bytes: 0,
        }
    }
    fn text(&mut self, raw: &str, limit: usize) -> String {
        let (clean, changed) = self.redactor.redact(raw);
        self.counters.redacted += changed;
        if clean.len() <= limit {
            return clean;
        }
        self.counters.truncated += 1;
        let mut end = limit;
        while !clean.is_char_boundary(end) {
            end -= 1;
        }
        format!(
            "{}\n[truncated: {} of {} bytes omitted]",
            &clean[..end],
            clean.len() - end,
            clean.len()
        )
    }
    /// Arguments with every string redacted; oversized arguments keep their
    /// shape and lose the ends of their longest strings.
    fn input(&mut self, value: &Value) -> Value {
        let size = serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len());
        let limit = if size > INPUT_BYTES {
            INPUT_BYTES / 8
        } else {
            INPUT_BYTES
        };
        self.walk(value, limit)
    }
    fn walk(&mut self, value: &Value, limit: usize) -> Value {
        match value {
            Value::String(text) => Value::String(self.text(text, limit)),
            Value::Array(items) => items.iter().map(|item| self.walk(item, limit)).collect(),
            Value::Object(members) => members
                .iter()
                .map(|(key, member)| (key.clone(), self.walk(member, limit)))
                .collect::<Map<_, _>>()
                .into(),
            other => other.clone(),
        }
    }
    fn push(&mut self, mut step: Map<String, Value>) -> Option<usize> {
        if self.steps.len() >= STEPS {
            self.counters.omitted += 1;
            return None;
        }
        step.insert("index".into(), self.steps.len().into());
        self.bytes += serde_json::to_vec(&step).map_or(0, |bytes| bytes.len());
        self.steps.push(step);
        Some(self.steps.len() - 1)
    }
    fn message(&mut self, text: &str, agent: Option<&str>) {
        if text.trim().is_empty() {
            return;
        }
        let text = self.text(text, MESSAGE_BYTES);
        let mut step = Map::new();
        step.insert("kind".into(), "message".into());
        step.insert("text".into(), text.into());
        if let Some(agent) = agent {
            step.insert("agent".into(), agent.into());
        }
        self.push(step);
    }
    fn started(&mut self, id: &str, tool: &str, input: &Value, agent: Option<&str>) {
        self.note_skills(tool, input);
        let input = self.input(input);
        let mut step = Map::new();
        step.insert("kind".into(), "tool".into());
        step.insert("tool".into(), tool.into());
        step.insert("input".into(), input);
        if let Some(agent) = agent {
            step.insert("agent".into(), agent.into());
        }
        if let Some(index) = self.push(step)
            && !id.is_empty()
        {
            self.open.insert(id.into(), index);
        }
    }
    fn completed(&mut self, id: &str, output: &str, error: bool, exit_code: Option<i64>) {
        let Some(index) = self.open.remove(id) else {
            return;
        };
        let output = if self.bytes >= TRACE_BYTES {
            self.counters.omitted += 1;
            "[omitted: the trace reached its size limit]".to_string()
        } else {
            self.text(output, OBSERVATION_BYTES)
        };
        self.bytes += output.len();
        let step = &mut self.steps[index];
        step.insert("output".into(), output.into());
        step.insert("error".into(), error.into());
        if let Some(code) = exit_code {
            step.insert("exit_code".into(), code.into());
        }
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
            let text = match input {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            found.extend(skill_paths(&text).into_iter().map(|path| SkillRef {
                name: None,
                path: Some(path),
            }));
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
                        Some("tool_use") => self.started(
                            block["id"].as_str().unwrap_or(""),
                            block["name"].as_str().unwrap_or("Tool"),
                            &block["input"],
                            agent,
                        ),
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
        if !self.open.contains_key(id) {
            self.started(id, kind, &input, None);
        }
        if phase == "item.completed" {
            let output = match &item["aggregated_output"] {
                Value::String(text) => text.clone(),
                _ => match &item["result"] {
                    Value::Null => String::new(),
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                },
            };
            let exit_code = item["exit_code"].as_i64();
            let error = item["status"] == "failed" || exit_code.is_some_and(|code| code != 0);
            self.completed(id, &output, error, exit_code);
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
                    if !self.open.contains_key(&id) {
                        self.started(&id, &tool, &arguments, None);
                    }
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
                                .or_else(|| other["stdout"].as_str())
                                .map_or_else(|| other.to_string(), String::from),
                        };
                        self.completed(
                            &id,
                            &output,
                            error,
                            outcome["success"]["exitCode"].as_i64(),
                        );
                    }
                }
            }
            _ => {}
        }
    }
    pub fn steps(&self) -> usize {
        self.steps.len()
    }
    pub fn finish(self) -> Trace {
        Trace {
            provider: self.provider,
            complete: self.open.is_empty(),
            steps: self.steps.into_iter().map(Value::Object).collect(),
            skills: self.skills,
            counters: self.counters,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(provider: &str, events: &[Value]) -> Trace {
        let mut recorder = Recorder::new(provider, &PolicyRedactor);
        for event in events {
            recorder.observe(event);
        }
        recorder.finish()
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
        let mut recorder = Recorder::new("codex", &PolicyRedactor);
        for index in 0..(TRACE_BYTES / OBSERVATION_BYTES + 20) {
            recorder.observe(&json!({"type": "item.completed", "item": {
                "id": format!("i{index}"), "type": "command_execution", "command": "cat f",
                "aggregated_output": big, "exit_code": 0, "status": "completed"}}));
        }
        let trace = recorder.finish();
        assert!(trace.counters.omitted >= 10, "{:?}", trace.counters);
        let size = serde_json::to_vec(&trace.to_value()).unwrap().len();
        assert!(size < TRACE_BYTES + 2 * OBSERVATION_BYTES, "{size}");
        assert_eq!(
            trace.steps.last().unwrap()["output"],
            "[omitted: the trace reached its size limit]"
        );
    }

    #[test]
    fn only_a_plain_file_read_counts_as_file_content() {
        for command in [
            "cat notes.txt",
            "bash -lc 'cat src/a.rs'",
            "Get-Content a.txt",
            "/bin/cat a",
        ] {
            assert!(reads_one_file(command), "{command}");
        }
        for command in [
            "cat a | grep b",
            "ls",
            "python3 x.py",
            "cat a > b",
            "cat a; rm b",
        ] {
            assert!(!reads_one_file(command), "{command}");
        }
        assert_eq!(unnumbered("plain\ntext"), "plain\ntext");
    }
}
