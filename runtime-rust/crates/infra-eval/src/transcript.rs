use crate::{Result, artifact, reconstruction, scoring};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
};
pub const KINDS: [&str; 7] = [
    "skill",
    "brain",
    "task-record",
    "chunk",
    "task",
    "spec-doc",
    "changelog",
];
pub fn records(path: &Path) -> Result<Vec<Value>> {
    let file = std::fs::File::open(path)?;
    let reader: Box<dyn Read> = if path.extension().is_some_and(|e| e == "zst") {
        Box::new(zstd::stream::read::Decoder::new(file)?)
    } else {
        Box::new(file)
    };
    let mut out = vec![];
    for line in BufReader::new(reader).split(b'\n') {
        let line = line?;
        if let Ok(v) = artifact::strict(&line)
            && v.is_object()
        {
            out.push(v);
        }
    }
    Ok(out)
}
pub fn kind(path: &str) -> &'static str {
    if scoring::skill(path) {
        "skill"
    } else if path.starts_with("project-brain/dynamic/tasks/")
        || path.starts_with("project-brain/archive/task/")
    {
        "task-record"
    } else if path.starts_with("project-brain/") {
        "brain"
    } else if path.starts_with("memory-bank/chunks/") {
        "chunk"
    } else if path.starts_with("tasks/") {
        "task"
    } else if path == "CHANGELOG.md" {
        "changelog"
    } else {
        "spec-doc"
    }
}
pub fn doc_paths(text: &str) -> BTreeSet<String> {
    let text = text.replace("\\/", "/");
    let Ok(re) = regex::Regex::new(
        r"(?:project-brain|memory-bank|specs|docs|tasks|\.agents/skills|\.claude/skills|\.cursor/skills|\.codex/skills)/[\w./@-]+\.md|CHANGELOG\.md",
    ) else {
        return BTreeSet::new();
    };
    re.find_iter(&text)
        .filter(|m| {
            m.start() == 0
                || !text[..m.start()]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-')
        })
        .map(|m| scoring::normalize(m.as_str()))
        .collect()
}
pub fn mentioned(path: &str, text: &str) -> bool {
    let p = Path::new(path);
    let filename = p.file_name().unwrap_or_default().to_string_lossy();
    let mut keys = vec![path.to_string()];
    if ["skill.md", "readme.md", "index.md", "manifest.md"]
        .contains(&filename.to_lowercase().as_str())
    {
        if let Some(n) = p.parent().and_then(Path::file_name) {
            keys.push(n.to_string_lossy().into());
        }
    } else {
        keys.push(filename.into());
        let stem = p.file_stem().unwrap_or_default().to_string_lossy();
        if let Ok(re) = regex::Regex::new(r"^MEM-\d{8}-[0-9a-f]{8}")
            && let Some(m) = re.find(&stem)
        {
            keys.push(m.as_str().into());
        }
        if uuid::Uuid::parse_str(&stem).is_ok() {
            keys.push(stem.into());
        }
    }
    keys.iter().any(|key| {
        text.match_indices(key).any(|(i, _)| {
            let left = text[..i].chars().next_back();
            let right = text[i + key.len()..].chars().next();
            !left.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-')
                && !right.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-')
        })
    })
}
pub fn capsule(text: &str) -> Option<String> {
    let text = text.replace("\r\n", "\n");
    let lines = text
        .lines()
        .skip_while(|l| {
            l.trim().is_empty()
                || l.trim() == "Previous native memory result was not saved completely."
        })
        .collect::<Vec<_>>();
    let first = *lines.first()?;
    if ![
        "Memory refresh",
        "working:",
        "warming:",
        "warning:",
        "procedural:",
        "semantic:",
        "episodic:",
        "memory review:",
        "codebase map:",
        "phases:",
        "brain-validation:",
    ]
    .iter()
    .any(|p| first.starts_with(p))
    {
        return None;
    }
    if first.starts_with("Memory refresh unavailable")
        || lines
            .iter()
            .any(|l| l.starts_with("warning: Capsule unavailable:"))
    {
        Some(String::new())
    } else {
        Some(lines.join("\n"))
    }
}
pub fn capsule_items(text: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    let (mut items, mut records) = (BTreeSet::new(), BTreeSet::new());
    let mut layer = false;
    for line in text.lines() {
        if ["semantic:", "episodic:", "procedural:"].contains(&line) {
            layer = true;
            continue;
        }
        if line.starts_with("  ") {
            if layer && !line.trim().is_empty() {
                items.extend(doc_paths(
                    line.split(" — ")
                        .next()
                        .unwrap_or(line)
                        .split(" § ")
                        .next()
                        .unwrap_or(line),
                ));
            }
            continue;
        }
        layer = false;
        if let Some(tail) = [
            "- memory ",
            "- history ",
            "- skill ",
            "- policy ",
            "weak-match: ",
        ]
        .iter()
        .find_map(|p| line.strip_prefix(p))
        {
            items.extend(doc_paths(
                tail.split(" — ")
                    .next()
                    .unwrap_or(tail)
                    .split(" § ")
                    .next()
                    .unwrap_or(tail),
            ));
        } else if let Some(tail) = line.strip_prefix("task record: ") {
            records.extend(doc_paths(tail));
        }
    }
    (items, records)
}
pub fn output_text(v: &Value) -> String {
    match v {
        Value::String(s) => {
            if let Some((head, tail)) = s.split_once("Output:\n")
                && ["Script ", "Exit code", "Process exited", "Chunk ID"]
                    .iter()
                    .any(|p| head.starts_with(p))
                && head.lines().count() <= 6
            {
                return tail.into();
            }
            if let Ok(v) = artifact::strict(s.as_bytes())
                && let Some(text) = v["output"].as_str()
            {
                return text.into();
            }
            s.clone()
        }
        Value::Array(a) => a
            .iter()
            .map(|p| output_text(p.get("text").unwrap_or(p)))
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(m) => ["output", "content", "text"]
            .iter()
            .find_map(|k| m.get(*k))
            .map(output_text)
            .unwrap_or_else(|| v.to_string()),
        _ => String::new(),
    }
}
pub fn retrieved(text: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut items = BTreeSet::new();
    let mut records = BTreeSet::new();
    let mut found = false;
    let stream = serde_json::Deserializer::from_str(text).into_iter::<Value>();
    for value in stream {
        let Ok(v) = value else {
            break;
        };
        let c = v.get("capsule").filter(|v| v.is_object()).unwrap_or(&v);
        if ["semantic", "episodic", "procedural"]
            .iter()
            .any(|l| c.get(*l).is_some())
        {
            found = true;
            for p in scoring::delivered(c) {
                items.extend(doc_paths(&p));
            }
            if let Some(r) = c["task_record"].as_str() {
                records.extend(doc_paths(r));
            }
        }
    }
    if found {
        (items, records)
    } else {
        capsule_items(text)
    }
}
fn native_command_fragments(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => out.push(s.clone()),
        Value::Array(values) if values.iter().all(Value::is_string) => out.push(
            values
                .iter()
                .filter_map(Value::as_str)
                .map(|s| shell_words::quote(s).into_owned())
                .collect::<Vec<_>>()
                .join(" "),
        ),
        Value::Array(values) => {
            for value in values {
                native_command_fragments(value, out);
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                native_command_fragments(value, out);
            }
        }
        _ => {}
    }
}
fn native_context_calls(blob: &str) -> (usize, usize) {
    let mut fragments = vec![];
    if let Ok(value) = artifact::strict(blob.as_bytes()) {
        native_command_fragments(&value, &mut fragments);
    } else {
        fragments.push(blob.into());
    }
    let (mut all, mut retrieves) = (0, 0);
    for fragment in fragments {
        for (at, _) in fragment.match_indices("infra-engine") {
            if at > 0
                && fragment[..at]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                continue;
            }
            let mut tail = &fragment[at + "infra-engine".len()..];
            if let Some(rest) = tail.strip_prefix(".exe") {
                tail = rest;
            }
            if tail
                .chars()
                .next()
                .is_some_and(|c| !c.is_whitespace() && !"\"',\\][".contains(c))
            {
                continue;
            }
            let tail =
                tail.trim_start_matches(|c: char| c.is_whitespace() || "\"',\\][".contains(c));
            let Ok(words) = shell_words::split(tail) else {
                continue;
            };
            let Some(first) = words.first().map(String::as_str) else {
                continue;
            };
            let mut index = if first == "memory" {
                1
            } else if ["retrieve", "context"].contains(&first) {
                0
            } else {
                continue;
            };
            all += 1;
            while let Some(word) = words.get(index) {
                if word == "--" {
                    index += 1;
                    break;
                }
                if ["--root", "--db", "--project", "--mode", "--owner"].contains(&word.as_str()) {
                    index += 2;
                    continue;
                }
                if ["--root=", "--db=", "--project=", "--mode=", "--owner="]
                    .iter()
                    .any(|prefix| word.starts_with(prefix))
                {
                    index += 1;
                    continue;
                }
                break;
            }
            if words
                .get(index)
                .is_some_and(|word| word == "retrieve" || word == "context")
            {
                retrieves += 1;
            }
        }
    }
    (all, retrieves)
}
pub fn context_calls(blob: &str) -> (usize, usize) {
    let mut all = 0;
    let mut retrieves = 0;
    for spelling in ["context.py", "infra-cli", "infra-runtime"] {
        for (i, _) in blob.match_indices(spelling) {
            all += 1;
            let tail = blob[i + spelling.len()..]
                .chars()
                .take(160)
                .collect::<String>();
            let words =
                tail.trim_start_matches(|c: char| c.is_whitespace() || "\"',\\][".contains(c));
            if words.starts_with("retrieve") || words.starts_with("context") {
                retrieves += 1;
            }
        }
    }
    let (native_all, native_retrieves) = native_context_calls(blob);
    (all + native_all, retrieves + native_retrieves)
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Turn {
    pub started: Option<DateTime<Utc>>,
    pub output: bool,
    pub delivered: BTreeSet<String>,
    pub records: BTreeSet<String>,
    pub digests: BTreeSet<String>,
    pub touched: BTreeSet<String>,
    pub text: Vec<String>,
    pub active: bool,
    pub retrieve_cli: usize,
    pub retrieve_mcp: usize,
    pub context_cli: usize,
    pub unassembled: bool,
}
impl Turn {
    fn deliver(&mut self, c: Option<String>) {
        if let Some(c) = c {
            if c.is_empty() {
                self.unassembled = true;
            } else {
                self.output = true;
                let (i, r) = capsule_items(&c);
                self.delivered.extend(i);
                self.records.extend(r);
                if let Ok(re) = regex::Regex::new(r"[0-9a-f]{64}") {
                    self.digests
                        .extend(re.find_iter(&c).map(|m| m.as_str().to_string()));
                }
            }
        }
    }
    fn call(&mut self, blob: &str, name: &str) {
        self.active = true;
        self.touched.extend(doc_paths(blob));
        let (all, retrieves) = context_calls(blob);
        self.context_cli += all;
        self.retrieve_cli += retrieves;
        self.retrieve_mcp += usize::from(name.ends_with("memory_retrieve"));
    }
}
#[derive(Clone, Debug, Default)]
pub struct Retrievals {
    pub pending: BTreeSet<String>,
    pub counts: BTreeMap<String, u64>,
    pub returned: BTreeMap<String, String>,
    pub opened: BTreeSet<String>,
    pub touched: BTreeSet<String>,
    pub answers: Vec<String>,
}
impl Retrievals {
    fn call(&mut self, id: Option<&str>, blob: &str, name: &str) {
        if name.ends_with("memory_retrieve") || context_calls(blob).1 > 0 {
            *self.counts.entry("calls".into()).or_default() += 1;
            if let Some(id) = id {
                self.pending.insert(id.into());
            }
            return;
        }
        let paths = doc_paths(blob);
        self.opened.extend(
            paths
                .iter()
                .filter(|p| self.returned.contains_key(*p))
                .cloned(),
        );
        self.touched.extend(paths);
    }
    fn answer(&mut self, id: Option<&str>, text: String) {
        if !id.is_some_and(|id| self.pending.remove(id)) {
            return;
        }
        *self.counts.entry("answers".into()).or_default() += 1;
        let (i, r) = retrieved(&text);
        *self.counts.entry("answers_naming".into()).or_default() +=
            u64::from(!i.is_empty() || !r.is_empty());
        for p in r {
            self.returned.entry(p).or_insert("record".into());
        }
        for p in i {
            self.returned.entry(p).or_insert("item".into());
        }
        self.answers.push(text);
    }
}
pub struct Parsed {
    pub turns: Vec<Turn>,
    pub retrievals: Retrievals,
    pub hooks: BTreeMap<(String, String), u64>,
}
pub fn parse(rows: &[Value], host: &str, since: Option<DateTime<Utc>>) -> Parsed {
    let mut p = Parsed {
        turns: vec![],
        retrievals: Retrievals::default(),
        hooks: BTreeMap::new(),
    };
    let mut current = None::<usize>;
    for r in rows {
        let at = r["timestamp"].as_str().and_then(reconstruction::instant);
        let within = since.is_none_or(|s| at.is_none_or(|at| at >= s));
        if host == "claude" {
            let kind = r["type"].as_str().unwrap_or("");
            let a = &r["attachment"];
            if kind == "attachment" {
                let outcome = match a["type"].as_str() {
                    Some("hook_success") => Some("ok"),
                    Some("hook_non_blocking_error") => Some("error"),
                    Some("hook_blocking_error") => Some("blocked"),
                    _ => None,
                };
                if let Some(outcome) = outcome {
                    let event = a["hookEvent"].as_str().unwrap_or("unknown");
                    *p.hooks.entry((event.into(), outcome.into())).or_default() += 1;
                    if a["exitCode"] == 127 {
                        *p.hooks
                            .entry((event.into(), "not_found".into()))
                            .or_default() += 1;
                    }
                }
                if a["type"] == "hook_success" && a["hookEvent"] == "UserPromptSubmit" {
                    if current.is_none_or(|i| p.turns[i].active) {
                        p.turns.push(Turn {
                            started: at,
                            ..Default::default()
                        });
                        current = Some(p.turns.len() - 1);
                    }
                    let mut c = None;
                    for text in [output_text(&a["stdout"]), output_text(&a["content"])] {
                        let text = artifact::strict(text.as_bytes())
                            .ok()
                            .and_then(|v| {
                                v["hookSpecificOutput"]["additionalContext"]
                                    .as_str()
                                    .map(String::from)
                            })
                            .unwrap_or(text);
                        c = capsule(&text);
                        if c.is_some() {
                            break;
                        }
                    }
                    p.turns[current.unwrap()].deliver(c);
                }
                continue;
            }
            let message = &r["message"];
            if kind == "user" {
                if let Some(parts) = message["content"].as_array() {
                    for part in parts {
                        if part["type"] == "tool_result" {
                            p.retrievals.answer(
                                part["tool_use_id"].as_str(),
                                output_text(&part["content"]),
                            );
                        }
                    }
                }
                if current.is_some_and(|i| p.turns[i].active)
                    && r["toolUseResult"].is_null()
                    && r["isMeta"] != true
                    && message["content"].is_string()
                {
                    current = None;
                }
                continue;
            }
            if kind != "assistant" {
                continue;
            }
            for part in message["content"].as_array().into_iter().flatten() {
                if part["type"] == "tool_use" {
                    let blob = part["input"].to_string();
                    let name = part["name"].as_str().unwrap_or("");
                    if within {
                        p.retrievals.call(part["id"].as_str(), &blob, name);
                    }
                    if let Some(i) = current {
                        p.turns[i].call(&blob, name);
                    }
                } else if part["type"] == "text"
                    && let (Some(i), Some(t)) = (current, part["text"].as_str())
                {
                    p.turns[i].active = true;
                    p.turns[i].text.push(t.into());
                }
            }
        } else {
            if r["type"] != "response_item" {
                continue;
            }
            let v = &r["payload"];
            let kind = v["type"].as_str().unwrap_or("");
            if kind == "message" {
                let text = v["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                let role = v["role"].as_str().unwrap_or("");
                let kinds = &v["internal_chat_message_metadata_passthrough"]["content_item_kinds"];
                let hook = !kinds.is_array()
                    || kinds
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|v| v == "hooks.additional_context");
                let c = if role == "developer" && hook {
                    capsule(&text)
                } else {
                    None
                };
                if c.is_some() {
                    if current.is_none_or(|i| p.turns[i].active) {
                        p.turns.push(Turn {
                            started: at,
                            ..Default::default()
                        });
                        current = Some(p.turns.len() - 1);
                    }
                    p.turns[current.unwrap()].deliver(c);
                } else if role == "user" && !text.trim_start().starts_with('<') {
                    if current.is_some_and(|i| p.turns[i].active) {
                        current = None;
                    }
                } else if role == "assistant"
                    && let Some(i) = current
                {
                    p.turns[i].active = true;
                    p.turns[i].text.push(text);
                }
            } else if ["function_call", "custom_tool_call", "local_shell_call"].contains(&kind) {
                let raw = v
                    .get("arguments")
                    .or_else(|| v.get("input"))
                    .or_else(|| v.get("action"))
                    .unwrap_or(&Value::Null);
                let blob = raw
                    .as_str()
                    .map(String::from)
                    .unwrap_or_else(|| raw.to_string());
                let name = v["name"].as_str().unwrap_or("");
                if within {
                    p.retrievals.call(v["call_id"].as_str(), &blob, name);
                }
                if let Some(i) = current {
                    p.turns[i].call(&blob, name);
                }
            } else if [
                "function_call_output",
                "custom_tool_call_output",
                "local_shell_call_output",
            ]
            .contains(&kind)
            {
                p.retrievals
                    .answer(v["call_id"].as_str(), output_text(&v["output"]));
            }
        }
    }
    p.turns
        .retain(|t| since.is_none_or(|s| t.started.is_none_or(|at| at >= s)));
    p
}
pub fn tally(turns: &[Turn]) -> Value {
    let mut counts = BTreeMap::<String, u64>::new();
    let mut kinds = BTreeMap::<String, BTreeMap<String, u64>>::new();
    for kind in KINDS {
        kinds.insert(
            kind.into(),
            BTreeMap::from([
                ("delivered".into(), 0),
                ("touched".into(), 0),
                ("mentioned".into(), 0),
            ]),
        );
    }
    let mut add = |key: &str, n: usize| {
        *counts.entry(key.into()).or_default() += n as u64;
    };
    for t in turns {
        add("hook_turns", 1);
        add("turns_with_capsule", usize::from(t.output));
        add(
            "turns_no_capsule_assembled",
            usize::from(t.unassembled && !t.output),
        );
        add("agent_retrieve_cli", t.retrieve_cli);
        add("agent_retrieve_mcp", t.retrieve_mcp);
        add("agent_context_cli", t.context_cli);
        add("record_pointers", t.records.len());
        add(
            "record_pointers_touched",
            t.records.intersection(&t.touched).count(),
        );
        if t.delivered.is_empty() {
            continue;
        }
        add("turns_with_documents", 1);
        let touched = t
            .delivered
            .intersection(&t.touched)
            .cloned()
            .collect::<BTreeSet<_>>();
        let text = t.text.join("\n");
        let said = t
            .delivered
            .iter()
            .filter(|p| mentioned(p, &text))
            .cloned()
            .collect::<BTreeSet<_>>();
        add("delivered_items", t.delivered.len());
        add("delivered_touched", touched.len());
        add("delivered_mentioned", said.len());
        add("turns_touching_delivered", usize::from(!touched.is_empty()));
        add("turns_mentioning_delivered", usize::from(!said.is_empty()));
        let other = t.touched.difference(&t.delivered).count();
        add("touched_not_delivered", other);
        add("turns_touching_other_docs", usize::from(other > 0));
        for path in &t.delivered {
            let row = kinds.get_mut(kind(path)).unwrap();
            *row.get_mut("delivered").unwrap() += 1;
            *row.get_mut("touched").unwrap() += u64::from(touched.contains(path));
            *row.get_mut("mentioned").unwrap() += u64::from(said.contains(path));
        }
    }
    for name in [
        "hook_turns",
        "turns_no_capsule_assembled",
        "turns_with_capsule",
        "turns_with_documents",
        "delivered_items",
        "delivered_touched",
        "delivered_mentioned",
        "turns_touching_delivered",
        "turns_mentioning_delivered",
        "touched_not_delivered",
        "turns_touching_other_docs",
        "record_pointers",
        "record_pointers_touched",
        "agent_retrieve_cli",
        "agent_retrieve_mcp",
        "agent_context_cli",
    ] {
        counts.entry(name.into()).or_default();
    }
    let mut out = json!(counts);
    out["by_kind"] = json!(kinds);
    out
}
pub fn retrieval_summary(sessions: &[Retrievals]) -> Value {
    let mut counts = BTreeMap::<String, u64>::new();
    let mut kinds = BTreeMap::<String, BTreeMap<String, u64>>::new();
    for k in KINDS {
        kinds.insert(
            k.into(),
            BTreeMap::from([
                ("returned".into(), 0),
                ("opened".into(), 0),
                ("otherwise".into(), 0),
            ]),
        );
    }
    for s in sessions {
        if s.counts.get("calls").copied().unwrap_or(0) == 0 {
            continue;
        }
        *counts.entry("sessions".into()).or_default() += 1;
        for (k, n) in &s.counts {
            *counts.entry(k.clone()).or_default() += n;
        }
        for (path, how) in &s.returned {
            let opened = u64::from(s.opened.contains(path));
            if how == "record" {
                *counts.entry("records_pointed".into()).or_default() += 1;
                *counts.entry("records_opened".into()).or_default() += opened;
            } else {
                *counts.entry("items_returned".into()).or_default() += 1;
                *counts.entry("items_opened".into()).or_default() += opened;
                let row = kinds.get_mut(kind(path)).unwrap();
                *row.get_mut("returned").unwrap() += 1;
                *row.get_mut("opened").unwrap() += opened;
            }
        }
        for path in s.touched.iter().filter(|p| !s.returned.contains_key(*p)) {
            *kinds
                .get_mut(kind(path))
                .unwrap()
                .get_mut("otherwise")
                .unwrap() += 1;
        }
    }
    for k in [
        "sessions",
        "calls",
        "answers",
        "answers_naming",
        "items_returned",
        "items_opened",
        "records_pointed",
        "records_opened",
    ] {
        counts.entry(k.into()).or_default();
    }
    let mut out = json!(counts);
    out["by_kind"] = json!(kinds);
    out
}
pub fn files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = vec![];
    fn walk(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        if !root.is_dir() {
            return Ok(());
        }
        for e in std::fs::read_dir(root)? {
            let p = e?.path();
            let m = std::fs::symlink_metadata(&p)?;
            if infra_core::fs::linked(&m) {
                continue;
            }
            if m.is_dir() {
                walk(&p, out)?;
            } else if p.file_name().is_some_and(|n| {
                let n = n.to_string_lossy();
                n.ends_with(".jsonl") || n.ends_with(".jsonl.zst")
            }) {
                out.push(p);
            }
        }
        Ok(())
    }
    walk(root, &mut out)?;
    out.sort();
    Ok(out)
}
pub fn project_member(project: &Path, cwd: &str) -> bool {
    let p = Path::new(cwd);
    if p == project || p.starts_with(project) {
        return true;
    }
    let common = |p: &Path| {
        reconstruction::git(p, &["rev-parse", "--git-common-dir"])
            .ok()
            .and_then(|s| infra_core::fs::canonicalize(p.join(s)).ok())
    };
    common(project).zip(common(p)).is_some_and(|(a, b)| a == b)
}
pub fn project_files(
    project: &Path,
    claude: &Path,
    codex: &Path,
) -> Result<(Vec<PathBuf>, Vec<PathBuf>, usize)> {
    let mut unread = 0;
    let mangled = project
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>();
    let mut cc = vec![];
    let mut cx = vec![];
    if claude.is_dir() {
        for e in std::fs::read_dir(claude)? {
            let folder = e?.path();
            let n = folder.file_name().unwrap_or_default().to_string_lossy();
            if !n.starts_with(&mangled) || !folder.is_dir() {
                continue;
            }
            let direct = n == mangled || n.starts_with(&format!("{mangled}--claude-worktrees-"));
            let files = files(&folder)?;
            let member = direct
                || files
                    .iter()
                    .find_map(|p| records(p).ok())
                    .into_iter()
                    .flatten()
                    .take(202)
                    .find_map(|r| r["cwd"].as_str().map(String::from))
                    .is_some_and(|cwd| project_member(project, &cwd));
            if member {
                cc.extend(files);
            }
        }
    }
    for p in files(codex)? {
        match records(&p) {
            Ok(rows) => {
                if rows
                    .first()
                    .and_then(|r| r["payload"]["cwd"].as_str())
                    .is_some_and(|cwd| project_member(project, cwd))
                {
                    cx.push(p);
                }
            }
            Err(_) => {
                if p.extension().is_some_and(|e| e == "zst") {
                    unread += 1;
                }
            }
        }
    }
    cc.sort();
    cx.sort();
    Ok((cc, cx, unread))
}
