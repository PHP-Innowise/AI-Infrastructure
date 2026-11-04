//! Numeric launch context metadata; text is measured then discarded.
use serde_json::{Value, json as object};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
fn count(value: &Value) -> Option<u64> {
    value.as_u64()
}
fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs_f64())
        .unwrap_or(0.)
}
fn kind(label: &str) -> &'static str {
    let path = label.split(" § ").next().unwrap_or("").trim();
    if path.starts_with("memory-bank/chunks/") {
        "bank"
    } else if path.starts_with("project-brain/") || path.starts_with("episode ") {
        "brain"
    } else {
        "rules"
    }
}
fn lines_with_ends(value: &str) -> Vec<&str> {
    let mut lines = vec![];
    let mut start = 0usize;
    let mut chars = value.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if matches!(
            c,
            '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{1c}'
                ..='\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}'
        ) {
            let mut end = at + c.len_utf8();
            if c == '\r'
                && chars.peek().is_some_and(|(_, next)| *next == '\n')
                && let Some((at, _)) = chars.next()
            {
                end = at + 1;
            }
            lines.push(&value[start..end]);
            start = end;
        }
    }
    if start < value.len() {
        lines.push(&value[start..]);
    }
    lines
}
pub fn capsule_text_parts(text: &str) -> Value {
    let mut brain = 0usize;
    let mut bank = 0usize;
    let mut rules = 0usize;
    let mut item = None;
    for line in lines_with_ends(text) {
        let mut found = None;
        for prefix in ["- memory ", "- history ", "- skill ", "- policy "] {
            if let Some(rest) = line.strip_prefix(prefix) {
                for (at, separator) in rest.match_indices(" —") {
                    let label = &rest[..at];
                    let after = &rest[at + separator.len()..];
                    if !label.is_empty()
                        && (after.is_empty() || after == "\n" || after.starts_with(' '))
                    {
                        found = Some(kind(label));
                        break;
                    }
                }
            }
        }
        let category = if let Some(category) = found {
            item = Some(category);
            category
        } else if item.is_some() && line.starts_with("  ") {
            item.unwrap_or("brain")
        } else if line.starts_with("  ") && line.contains(" — ") {
            item = None;
            kind(line.trim().split(" — ").next().unwrap_or(""))
        } else {
            item = None;
            "brain"
        };
        let length = line.chars().count();
        match category {
            "bank" => bank += length,
            "rules" => rules += length,
            _ => brain += length,
        };
    }
    object!({"brain":brain,"bank":bank,"rules":rules})
}
#[derive(Clone)]
pub struct ContextTracker {
    pub provider: String,
    pub start: Option<u64>,
    pub end: Option<u64>,
    pub peak: Option<u64>,
    pub window: Option<u64>,
    pub model: Option<String>,
    pub cache_share: Option<f64>,
    pub calls: u64,
    messages: BTreeSet<String>,
    pub compactions: Vec<Value>,
    pub hooks: Option<Value>,
    written_at: Instant,
    written_end: Option<u64>,
    never_written: bool,
}
impl ContextTracker {
    pub fn new(provider: &str) -> Self {
        Self {
            provider: provider.into(),
            start: None,
            end: None,
            peak: None,
            window: None,
            model: None,
            cache_share: None,
            calls: 0,
            messages: BTreeSet::new(),
            compactions: vec![],
            hooks: None,
            written_at: Instant::now(),
            written_end: None,
            never_written: true,
        }
    }
    fn call(&mut self, fill: u64) {
        self.calls = self.calls.saturating_add(1);
        if self.start.is_none() {
            self.start = Some(fill);
        }
        self.end = Some(fill);
        self.peak = Some(self.peak.unwrap_or(0).max(fill));
    }
    pub fn observe(&mut self, event: &Value) -> bool {
        if !event.is_object() {
            return false;
        }
        let kind = event["type"].as_str().unwrap_or("");
        if self.provider == "claude" {
            if kind == "assistant"
                && event["parent_tool_use_id"].is_null()
                && !truth(&event["is_meta"])
                && !truth(&event["historical"])
            {
                let message = &event["message"];
                let Some(id) = message["id"].as_str() else {
                    return false;
                };
                let usage = &message["usage"];
                if self.messages.contains(id) || !usage.is_object() {
                    return false;
                }
                let parts = [
                    "input_tokens",
                    "cache_read_input_tokens",
                    "cache_creation_input_tokens",
                ]
                .map(|key| usage.get(key));
                if parts[0].and_then(count).is_none()
                    || parts.iter().any(|value| {
                        value.is_some_and(|value| !value.is_null() && count(value).is_none())
                    })
                {
                    return false;
                }
                let Some(fill) = parts.iter().try_fold(0u64, |sum, value| {
                    sum.checked_add(value.and_then(count).unwrap_or(0))
                }) else {
                    return false;
                };
                if fill == 0 {
                    return false;
                }
                self.messages.insert(id.into());
                if let Some(model) = message["model"].as_str() {
                    self.model = Some(model.into());
                }
                self.call(fill);
                return true;
            }
            if kind == "system" && event["subtype"] == "compact_boundary" {
                let meta = &event["compact_metadata"];
                if let Some(pre) = count(&meta["pre_tokens"]) {
                    self.compactions.push(
                        object!({"pre":pre,"post":count(&meta["post_tokens"]),"call":self.calls}),
                    );
                    self.peak = Some(self.peak.unwrap_or(0).max(pre));
                    return true;
                }
                return false;
            }
            if kind == "system"
                && event["subtype"] == "hook_response"
                && ["UserPromptSubmit", "SessionStart"]
                    .contains(&event["hook_event"].as_str().unwrap_or(""))
            {
                if let Some(text) = event["stdout"].as_str().filter(|text| !text.is_empty()) {
                    let found = capsule_text_parts(text);
                    let old = self.hooks.clone().unwrap_or(object!({}));
                    self.hooks = Some(
                        object!({"brain":old["brain"].as_u64().unwrap_or(0).saturating_add(found["brain"].as_u64().unwrap_or(0)),"bank":old["bank"].as_u64().unwrap_or(0).saturating_add(found["bank"].as_u64().unwrap_or(0)),"rules":old["rules"].as_u64().unwrap_or(0).saturating_add(found["rules"].as_u64().unwrap_or(0))}),
                    );
                    return true;
                }
                return false;
            }
            if kind == "result" {
                if let Some(windows) = event["modelUsage"].as_object() {
                    let selected = self
                        .model
                        .as_ref()
                        .and_then(|model| windows.get(model))
                        .and_then(|entry| count(&entry["contextWindow"]))
                        .filter(|value| *value > 0);
                    let known = windows
                        .values()
                        .filter_map(|entry| count(&entry["contextWindow"]))
                        .filter(|value| *value > 0)
                        .max();
                    self.window = selected.or(known).or(self.window);
                }
                let parts = [
                    "input_tokens",
                    "cache_read_input_tokens",
                    "cache_creation_input_tokens",
                ]
                .map(|key| count(&event["usage"][key]));
                if let [Some(input), Some(read), Some(create)] = parts
                    && let Some(sum) = input
                        .checked_add(read)
                        .and_then(|sum| sum.checked_add(create))
                        .filter(|value| *value > 0)
                {
                    self.cache_share = Some(read as f64 / sum as f64);
                }
                return true;
            }
            return false;
        }
        if self.provider == "codex"
            && kind == "turn.completed"
            && let (Some(cached), Some(total)) = (
                count(&event["usage"]["cached_input_tokens"]),
                count(&event["usage"]["input_tokens"]).filter(|value| *value > 0),
            )
        {
            self.cache_share = Some((cached as f64 / total as f64).min(1.));
            return true;
        }
        false
    }
    pub fn update(&mut self, fill: Option<&Value>) {
        if let Some(fill) = fill {
            self.start = count(&fill["start"]);
            self.end = count(&fill["end"]);
            self.peak = count(&fill["peak"]);
            self.calls = count(&fill["calls"]).unwrap_or(0);
            self.window = count(&fill["window"])
                .filter(|value| *value > 0)
                .or(self.window);
            self.compactions = fill["compactions"].as_array().cloned().unwrap_or_default();
        }
    }
    pub fn snapshot(&self) -> Value {
        object!({"start":self.start,"end":self.end,"peak":self.peak,"calls":if self.calls>0{Some(self.calls)}else{None},"window":self.window,"compactions":self.compactions.iter().take(20).cloned().collect::<Vec<_>>(),"cache_share":self.cache_share.filter(|value|value.is_finite()).and_then(|value|format!("{value:.4}").parse::<f64>().ok()),"hooks":self.hooks})
    }
    pub fn due(&mut self) -> bool {
        let moved = match (self.window, self.end, self.written_end) {
            (Some(window), Some(end), Some(written)) => {
                end.abs_diff(written) as f64 >= 0.005 * window as f64
            }
            _ => false,
        };
        if self.never_written || self.written_at.elapsed() >= Duration::from_secs(2) || moved {
            self.never_written = false;
            self.written_at = Instant::now();
            self.written_end = self.end;
            true
        } else {
            false
        }
    }
}
fn truth(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.),
    }
}
pub struct CodexFill {
    pub since: f64,
    pub until: Option<f64>,
    fills: Vec<u64>,
    window: Option<u64>,
    compactions: Vec<Value>,
}
impl CodexFill {
    pub fn new(since: f64, until: Option<f64>) -> Self {
        Self {
            since,
            until,
            fills: vec![],
            window: None,
            compactions: vec![],
        }
    }
    pub fn feed(&mut self, line: &[u8]) -> bool {
        let Ok(record) = infra_host::strict_json::parse(line, 4 * 1024 * 1024) else {
            return false;
        };
        let Some(stamp) = record["timestamp"]
            .as_str()
            .and_then(|stamp| chrono::DateTime::parse_from_rfc3339(stamp).ok())
            .map(|stamp| stamp.timestamp_micros() as f64 / 1_000_000.)
        else {
            return false;
        };
        if stamp < self.since || self.until.is_some_and(|until| stamp > until) {
            return false;
        }
        let payload = &record["payload"];
        if record["type"] == "event_msg"
            && payload["type"] == "token_count"
            && payload["info"].is_object()
        {
            let info = &payload["info"];
            let mut changed = false;
            if let Some(window) = count(&info["model_context_window"]).filter(|value| *value > 0) {
                changed = self.window != Some(window);
                self.window = Some(window);
            }
            if let Some(fill) =
                count(&info["last_token_usage"]["input_tokens"]).filter(|value| *value > 0)
            {
                self.fills.push(fill);
                for compaction in &mut self.compactions {
                    if compaction["post"].is_null()
                        && compaction["call"].as_u64() == Some(self.fills.len() as u64 - 1)
                    {
                        compaction["post"] = fill.into();
                    }
                }
                return true;
            }
            return changed;
        }
        if record["type"] == "compacted" {
            self.compactions
                .push(object!({"pre":self.fills.last(),"post":null,"call":self.fills.len()}));
            return true;
        }
        false
    }
    pub fn snapshot(&self) -> Option<Value> {
        if self.fills.is_empty() {
            return None;
        }
        let peak = self
            .fills
            .iter()
            .copied()
            .chain(
                self.compactions
                    .iter()
                    .filter_map(|item| count(&item["pre"]))
                    .filter(|value| *value > 0),
            )
            .max();
        Some(
            object!({"start":self.fills.first(),"end":self.fills.last(),"peak":peak,"calls":self.fills.len(),"window":self.window,"compactions":self.compactions.iter().take(20).cloned().collect::<Vec<_>>()}),
        )
    }
}
pub struct CodexLive {
    project: PathBuf,
    pub fill: CodexFill,
    rollout: Option<infra_host::rollout::Rollout>,
    offset: u64,
    partial: Vec<u8>,
    retry_at: Option<Instant>,
}
impl CodexLive {
    pub fn new(project: &Path, since: f64) -> Self {
        Self {
            project: project.into(),
            fill: CodexFill::new(since, None),
            rollout: None,
            offset: 0,
            partial: vec![],
            retry_at: None,
        }
    }
    pub fn poll(&mut self, native_id: Option<&str>) -> bool {
        let Some(id) = native_id.filter(|value| !value.is_empty()) else {
            return false;
        };
        if self.rollout.is_none() {
            if self.retry_at.is_some_and(|time| Instant::now() < time) {
                return false;
            }
            if let Some(rollout) = infra_host::rollout::open(id, &self.project) {
                self.offset = rollout.header_end;
                self.rollout = Some(rollout);
            } else {
                self.retry_at = Some(Instant::now() + Duration::from_secs(2));
                return false;
            }
        }
        let Some(rollout) = &mut self.rollout else {
            return false;
        };
        let Ok(data) = rollout.read(self.offset, 4 * 1024 * 1024) else {
            return false;
        };
        self.offset = self.offset.saturating_add(data.len() as u64);
        let mut full = std::mem::take(&mut self.partial);
        full.extend(data);
        let mut changed = false;
        let mut start = 0;
        for (at, byte) in full.iter().enumerate() {
            if *byte == b'\n' {
                changed = self.fill.feed(&full[start..at]) || changed;
                start = at + 1;
            }
        }
        self.partial = full[start..].to_vec();
        if self.partial.len() > 4 * 1024 * 1024 {
            self.partial.clear();
        }
        changed
    }
    pub fn settle(&mut self, native_id: Option<&str>, until: f64) -> bool {
        self.fill.until = Some(until);
        self.retry_at = None;
        let mut changed = false;
        for _ in 0..8 {
            let previous = self.offset;
            changed = self.poll(native_id) || changed;
            if previous == self.offset {
                break;
            }
        }
        let end = if let Some(rollout) = &mut self.rollout {
            rollout.end().ok()
        } else {
            native_id.and_then(|id| infra_host::rollout::end(id, &self.project))
        };
        if let Some(end) = end.filter(|end| {
            self.rollout
                .as_ref()
                .is_none_or(|rollout| rollout.path == end.path)
        }) {
            let read = if self.rollout.is_some() {
                self.offset.saturating_sub(self.partial.len() as u64)
            } else {
                0
            };
            let mut position = end.position;
            for line in end.bytes.split(|byte| *byte == b'\n') {
                if position >= read {
                    changed = self.fill.feed(line) || changed;
                }
                position = position.saturating_add(line.len() as u64 + 1);
            }
            if self.rollout.is_none()
                || end.position.saturating_add(end.bytes.len() as u64) >= self.offset
            {
                if self.rollout.is_none()
                    && let Some(id) = native_id
                {
                    self.rollout = infra_host::rollout::open(id, &self.project);
                }
                self.offset = end.position.saturating_add(end.bytes.len() as u64);
                self.partial.clear();
            }
        }
        changed
    }
}
pub fn launch_files(project: &Path, provider: &str) -> Value {
    let paths: &[&str] = match provider {
        "claude" => &["CLAUDE.md", ".claude/CLAUDE.md"],
        "codex" => &["AGENTS.md"],
        "cursor" => &["AGENTS.md", "CLAUDE.md"],
        _ => &[],
    };
    let mut files = vec![];
    let mut reader = match infra_core::bank::publication::PublicationFiles::new(project) {
        Ok(reader) => reader,
        Err(_) => {
            return object!({"cli_files":[],"hooks":{"installed":false,"measured":false,"bytes":null}});
        }
    };
    for path in paths {
        if let Ok(Some((bytes, _))) = reader.read_prefix(path, 0) {
            files.push(object!({"name":path,"bytes":bytes}));
        }
    }
    if provider == "cursor" {
        let rule = reader
            .read_prefix(".cursor/rules/working-memory.mdc", 0)
            .ok()
            .flatten();
        return object!({"cli_files":files,"hooks":{"installed":rule.is_some(),"measured":rule.is_some(),"bytes":rule.map(|value|value.0)}});
    }
    let path = match provider {
        "claude" => ".claude/settings.json",
        "codex" => ".codex/hooks.json",
        _ => "",
    };
    let installed = !path.is_empty()
        && reader
            .read_prefix(path, 256 * 1024)
            .ok()
            .flatten()
            .is_some_and(|(_, raw)| String::from_utf8_lossy(&raw).contains("UserPromptSubmit"));
    object!({"cli_files":files,"hooks":{"installed":installed,"measured":installed&&provider=="claude","bytes":null}})
}
pub struct PromptLedger {
    value: Value,
    total: u64,
}
impl PromptLedger {
    pub fn new() -> Self {
        Self {
            value: object!({"message":0,"capsule":null,"excerpts":null,"attachments":null}),
            total: 0,
        }
    }
    pub fn push(&mut self, category: &str, text: &str) {
        let size = text.chars().count() as u64;
        self.total = self.total.saturating_add(size);
        match category {
            "message" => {
                self.value["message"] = (self.value["message"]
                    .as_u64()
                    .unwrap_or(0)
                    .saturating_add(size))
                .into()
            }
            "capsule" => {
                let inserted = self.value["capsule"]["inserted"]
                    .as_u64()
                    .unwrap_or(0)
                    .saturating_add(size);
                self.value["capsule"] = object!({"characters":null,"inserted":inserted,"kinds":null,"repeats":null,"dropped":null,"items":null});
            }
            "attachments" => {
                let size = self.value["attachments"]["characters"]
                    .as_u64()
                    .unwrap_or(0)
                    .saturating_add(size);
                self.value["attachments"] = object!({"characters":size,"count":null});
            }
            _ => {}
        }
    }
    pub fn capsule(&mut self, detail: &Value, text: &str) {
        let size = text.chars().count() as u64;
        self.total = self.total.saturating_add(size);
        let mut capsule = object!({"characters":count(&detail["characters"]),"inserted":size,"kinds":null,"repeats":count(&detail["repeats"]),"dropped":count(&detail["dropped"]),"items":null});
        if detail["kinds"].is_object() {
            capsule["kinds"] = object!({"brain":count(&detail["kinds"]["brain"]),"bank":count(&detail["kinds"]["bank"]),"rules":count(&detail["kinds"]["rules"])});
        }
        if detail["items"].is_object() {
            capsule["items"] = object!({"brain":count(&detail["items"]["brain"]),"bank":count(&detail["items"]["bank"]),"rules":count(&detail["items"]["rules"])});
        }
        self.value["capsule"] = capsule;
    }
    pub fn excerpt(&mut self, name: &str, sent: u64, full: u64, text: &str) {
        let size = text.chars().count() as u64;
        self.total = self.total.saturating_add(size);
        if !self.value["excerpts"].is_array() {
            self.value["excerpts"] = object!([]);
        }
        let name = if name.len() <= 1024
            && !Path::new(name).is_absolute()
            && !name.contains('\\')
            && !name.split('/').any(|part| part == "..")
        {
            Some(name)
        } else {
            None
        };
        self.value["excerpts"]
            .as_array_mut()
            .unwrap()
            .push(object!({"name":name,"sent":sent,"full":full,"characters":size}));
    }
    pub fn attachments(&mut self, count: u64, text: &str) {
        self.total = self.total.saturating_add(text.chars().count() as u64);
        self.value["attachments"] = object!({"characters":text.chars().count(),"count":count});
    }
    pub fn value(&self) -> Value {
        let mut value = self.value.clone();
        let excerpts = value["excerpts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|entry| count(&entry["characters"]))
            .sum::<u64>();
        let accounted = count(&value["message"])
            .unwrap_or(0)
            .saturating_add(count(&value["capsule"]["inserted"]).unwrap_or(0))
            .saturating_add(count(&value["attachments"]["characters"]).unwrap_or(0))
            .saturating_add(excerpts);
        value["total"] = self.total.into();
        value["instructions"] = self.total.saturating_sub(accounted).into();
        value
    }
}
impl Default for PromptLedger {
    fn default() -> Self {
        Self::new()
    }
}
pub fn unix_now() -> f64 {
    now()
}
