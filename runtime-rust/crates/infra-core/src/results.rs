//! Native result normalization, task CAS, exact replay and recoverable writes.
use crate::{
    BankService, BrainRepository, CreateRecord, ErrorKind, Result, RuntimeConfig, RuntimeContext,
    RuntimeError, UpdateRecord,
    domain::*,
    result_store::{self, Attempt, EvidenceStore, Receipt},
    serving::TaskService,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json as object};
use std::collections::{BTreeMap, BTreeSet};

pub fn refused(code: &str, message: impl Into<String>) -> RuntimeError {
    RuntimeError::coded(
        ErrorKind::InvalidInput,
        format!("memory-result-v1:{code}"),
        message,
    )
}
pub fn py_space(value: char) -> bool {
    value.is_whitespace() || matches!(value, '\u{1c}'..='\u{1f}')
}
pub fn clean(value: &str) -> String {
    value.trim_matches(py_space).into()
}
pub fn collapse(value: &str) -> String {
    value
        .split(py_space)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn text(value: &Value, label: &str, limit: usize) -> Result<String> {
    let value = value
        .as_str()
        .filter(|value| !clean(value).is_empty() && value.chars().count() <= limit)
        .ok_or_else(|| {
            refused(
                "text_limit",
                format!("{label} must be nonempty text within {limit} characters"),
            )
        })?;
    guard_shared_text(label, &[value.into()])
        .map_err(|error| refused("sensitive", error.message))?;
    let raw=regex::Regex::new(r"(?im)^\s*(?:user|assistant|system|developer|tool|prompt|response|reasoning|stdout|stderr|log)\s*:").map_err(|_|RuntimeError::invalid("Invalid raw text expression"))?;
    if raw.is_match(value) {
        return Err(refused(
            "raw_text",
            format!("{label} looks like a pasted log or transcript; summarize it instead"),
        ));
    }
    Ok(clean(value))
}
pub fn strings(value: &Value, label: &str, count: usize, limit: usize) -> Result<Vec<String>> {
    let items = value
        .as_array()
        .filter(|items| items.len() <= count)
        .ok_or_else(|| {
            refused(
                "schema",
                format!("{label} must be a list of at most {count} items"),
            )
        })?;
    Ok(unique_strings(
        items
            .iter()
            .map(|value| text(value, label, limit))
            .collect::<Result<Vec<_>>>()?,
    ))
}
/// Whether `value` is `^[\p{L}\p{N}][\p{L}\p{N}_\u{0300}-\u{036f}./#+-]{0,limit-1}$`:
/// the bounded repetition is a length check, since unrolling it makes the
/// expression costly to build.
pub fn task_grammar(value: &str, limit: usize) -> bool {
    static GRAMMAR: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    GRAMMAR
        .get_or_init(|| {
            regex::Regex::new(r"^[\p{L}\p{N}][\p{L}\p{N}_\u{0300}-\u{036f}./#+-]*$")
                .expect("constant task grammar")
        })
        .is_match(value)
        && value.chars().count() <= limit
}
pub fn task_id(value: &str) -> Result<String> {
    let id = text(&object!(value), "task ID", 200)?;
    if !task_grammar(&id, 200) || id.split('/').any(|part| part == "..") {
        return Err(refused("task_id", "Invalid task ID"));
    }
    Ok(id)
}
pub fn result_id(value: &str) -> Result<String> {
    let id = text(&object!(value), "result ID", 64)?;
    if !id
        .bytes()
        .enumerate()
        .all(|(index, byte)| byte.is_ascii_alphanumeric() || index > 0 && b"._-".contains(&byte))
    {
        return Err(refused("result_id", "Invalid result ID"));
    }
    Ok(id)
}
pub fn canonical_image(values: &[Value]) -> Result<Vec<Value>> {
    let mut image = BTreeMap::new();
    for value in values {
        if !result_store::exact(value, &["path", "sha256"]) {
            return Err(refused("reference", "Invalid checked source image"));
        }
        let (path, digest) = (
            value["path"]
                .as_str()
                .ok_or_else(|| refused("reference", "Invalid checked source path"))?,
            value["sha256"]
                .as_str()
                .ok_or_else(|| refused("reference", "Invalid checked source digest"))?,
        );
        if path.contains('#')
            || !result_store::hex(digest, 64)
            || image.get(path).is_some_and(|prior| prior != digest)
        {
            return Err(refused("reference", "Inconsistent checked source image"));
        }
        image.insert(path.to_string(), digest.to_string());
    }
    Ok(image
        .into_iter()
        .map(|(path, sha256)| object!({"path":path,"sha256":sha256}))
        .collect())
}

/// Strict evidence JSON: unlike document JSON, duplicate member names refuse.
pub fn strict_json(raw: &[u8]) -> Result<Value> {
    struct Scan<'a> {
        raw: &'a [u8],
        at: usize,
    }
    impl Scan<'_> {
        fn space(&mut self) {
            while self
                .raw
                .get(self.at)
                .is_some_and(|byte| b" \t\r\n".contains(byte))
            {
                self.at += 1;
            }
        }
        fn string(&mut self) -> Result<String> {
            self.space();
            let begin = self.at;
            if self.raw.get(self.at) != Some(&b'"') {
                return Err(RuntimeError::invalid("Invalid evidence JSON member"));
            }
            self.at += 1;
            while let Some(byte) = self.raw.get(self.at) {
                self.at += 1;
                if *byte == b'\\' {
                    self.at += 1;
                } else if *byte == b'"' {
                    return serde_json::from_slice(&self.raw[begin..self.at]).map_err(Into::into);
                }
            }
            Err(RuntimeError::invalid("Unterminated evidence JSON string"))
        }
        fn value(&mut self, depth: usize) -> Result<()> {
            if depth > 128 {
                return Err(RuntimeError::invalid("Evidence JSON depth exceeded"));
            }
            self.space();
            match self.raw.get(self.at) {
                Some(b'{') => {
                    self.at += 1;
                    self.space();
                    if self.raw.get(self.at) == Some(&b'}') {
                        self.at += 1;
                        return Ok(());
                    }
                    let mut names = BTreeSet::new();
                    loop {
                        let key = self.string()?;
                        if !names.insert(key) {
                            return Err(RuntimeError::invalid("Duplicate evidence JSON member"));
                        }
                        self.space();
                        if self.raw.get(self.at) != Some(&b':') {
                            return Err(RuntimeError::invalid("Invalid evidence JSON"));
                        }
                        self.at += 1;
                        self.value(depth + 1)?;
                        self.space();
                        match self.raw.get(self.at) {
                            Some(b'}') => {
                                self.at += 1;
                                break;
                            }
                            Some(b',') => self.at += 1,
                            _ => return Err(RuntimeError::invalid("Invalid evidence JSON")),
                        }
                    }
                }
                Some(b'[') => {
                    self.at += 1;
                    self.space();
                    if self.raw.get(self.at) == Some(&b']') {
                        self.at += 1;
                        return Ok(());
                    }
                    loop {
                        self.value(depth + 1)?;
                        self.space();
                        match self.raw.get(self.at) {
                            Some(b']') => {
                                self.at += 1;
                                break;
                            }
                            Some(b',') => self.at += 1,
                            _ => return Err(RuntimeError::invalid("Invalid evidence JSON")),
                        }
                    }
                }
                Some(b'"') => {
                    self.string()?;
                }
                Some(_) => {
                    let start = self.at;
                    while self
                        .raw
                        .get(self.at)
                        .is_some_and(|byte| !b" \t\r\n,]}".contains(byte))
                    {
                        self.at += 1;
                    }
                    if start == self.at {
                        return Err(RuntimeError::invalid("Invalid evidence JSON"));
                    }
                }
                None => return Err(RuntimeError::invalid("Incomplete evidence JSON")),
            }
            Ok(())
        }
    }
    let mut scan = Scan { raw, at: 0 };
    scan.value(0)?;
    scan.space();
    if scan.at != raw.len() {
        return Err(RuntimeError::invalid("Trailing evidence JSON"));
    }
    json::parse(
        std::str::from_utf8(raw)
            .map_err(|_| RuntimeError::invalid("Evidence JSON is not UTF-8"))?,
    )
}

/// Strict command/protocol input. The duplicate-only legacy metadata reader
/// above intentionally retains its established nonfinite-number policy.
/// `repr(text)` as Python prints a string: single quotes unless the text
/// holds one and no double quote, with escapes for the quote, backslash and
/// control characters.
fn python_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::from(quote);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                out.push_str(&format!("\\x{:02x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}
pub fn strict_input(raw: &[u8]) -> Result<Value> {
    crate::strict_json::parse(raw, raw.len(), 128, 1_000_000)
        .map_err(|error| RuntimeError::invalid(error.message))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Learning {
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub consequence: String,
    pub sources: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_fingerprints: Option<Vec<Value>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NormalizedResult {
    pub progress: Option<String>,
    pub next_steps: Option<Vec<String>>,
    pub learnings: Vec<Learning>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_decision: Option<String>,
}
#[derive(Clone, Debug)]
pub enum Revision {
    Auto,
    Exact(u64),
}
#[derive(Clone, Debug)]
pub struct RecordResult {
    pub task_id: String,
    pub result_id: String,
    pub revision: Revision,
    pub data: Value,
    pub reason: String,
    pub attestation: String,
    pub prepared: Option<String>,
    pub replay_only: bool,
}
#[derive(Clone, Debug)]
pub struct PrepareResult {
    pub task_id: String,
    pub result_id: String,
    pub revision: u64,
    pub data: Value,
    pub reason: String,
    pub attestation: String,
}
#[derive(Clone)]
pub struct ResultService {
    pub context: RuntimeContext,
}
pub fn request_digest(task: &str, result: &NormalizedResult) -> Result<String> {
    let mut value = serde_json::to_value(result)?;
    value
        .as_object_mut()
        .ok_or_else(|| refused("schema", "Invalid normalized result"))?
        .insert("task".into(), task.into());
    result_store::legacy_digest(&value, true, false)
}
pub fn learning_id(task: &str, learning: &Learning) -> Result<String> {
    let key = if let Some(id) = &learning.record_id {
        object!([task, "record", id])
    } else {
        let mut sources = learning.sources.clone();
        sources.sort();
        object!([
            task,
            learning.kind,
            collapse(&crate::query::folded(&learning.title)),
            sources
        ])
    };
    Ok(format!(
        "learning-{}",
        &result_store::legacy_digest(&key, false, false)?[..24]
    ))
}
pub fn consequence(record: &Value) -> String {
    collapse(
        record["progress"]
            .as_str()
            .filter(|value| !value.is_empty())
            .or_else(|| record["goal"].as_str())
            .unwrap_or(""),
    )
}
impl ResultService {
    pub fn new(context: RuntimeContext) -> Self {
        Self { context }
    }
    pub fn brain(&self) -> Result<BrainRepository> {
        BrainRepository::new(self.context.clone())
    }
    pub fn source(&self, value: &str, require_file: bool) -> Result<String> {
        let value = text(&object!(value), "source path", 1024)?;
        if let Some(problem) = source_path_problem(&value) {
            return Err(refused("source_path", problem));
        }
        let (head, anchor) = value
            .split_once('#')
            .map(|(head, tail)| (head, Some(tail)))
            .unwrap_or((&value, None));
        let normalized = head
            .split('/')
            .filter(|part| !part.is_empty() && *part != ".")
            .collect::<Vec<_>>();
        if normalized.is_empty() {
            return Err(refused("source_path", "Use project-relative source paths"));
        }
        let base = if self.context.roots.shared
            && crate::workspace::accelerator_owned_source(head)
            && !self.context.roots.project.join(head).exists()
        {
            &self.context.roots.tooling
        } else {
            &self.context.roots.project
        };
        let mut path = base.clone();
        let mut spelled = Vec::new();
        let mut observed = true;
        for part in normalized {
            let names = std::fs::read_dir(&path).ok().map(|entries| {
                entries
                    .flatten()
                    .filter_map(|entry| entry.file_name().into_string().ok())
                    .collect::<Vec<_>>()
            });
            let actual = names.and_then(|names| {
                if names.iter().any(|name| name == part) {
                    Some(part.into())
                } else {
                    let matches = names
                        .into_iter()
                        .filter(|name| crate::query::folded(name) == crate::query::folded(part))
                        .collect::<Vec<_>>();
                    if matches.len() == 1 {
                        matches.into_iter().next()
                    } else {
                        None
                    }
                }
            });
            let component = actual.unwrap_or_else(|| {
                observed = false;
                part.into()
            });
            path.push(&component);
            if require_file
                && std::fs::symlink_metadata(&path)
                    .is_ok_and(|image| crate::fs::linked_at(&path, &image).unwrap_or(true))
            {
                return Err(refused("source_path", "Symlink source paths are refused"));
            }
            spelled.push(component);
        }
        if require_file && !path.is_file() {
            return Err(refused(
                "source_missing",
                format!(
                    "Source is missing or is not a regular file: {}",
                    head.chars().take(200).collect::<String>()
                ),
            ));
        }
        if !require_file && !observed {
            return Ok(value);
        }
        Ok(format!(
            "{}{}",
            spelled.join("/"),
            anchor.map(|tail| format!("#{tail}")).unwrap_or_default()
        ))
    }
    pub fn checked_fingerprints(&self, sources: &[String]) -> Result<Vec<Value>> {
        if sources.is_empty() || sources.len() > 10 {
            return Err(refused(
                "no_sources",
                "Cite one through ten sources to fingerprint",
            ));
        }
        let paths = sources
            .iter()
            .map(|value| self.source(value, true))
            .collect::<Result<Vec<_>>>()?;
        canonical_image(&source_fingerprints(&self.context.roots, &paths)?)
    }
    pub fn normalize(&self, data: &Value, check_files: bool) -> Result<NormalizedResult> {
        let map = data
            .as_object()
            .ok_or_else(|| refused("schema", "A result is an object"))?;
        if map.keys().any(|key| {
            ![
                "progress",
                "next_steps",
                "learnings",
                "verified",
                "capture_decision",
            ]
            .contains(&key.as_str())
        }) {
            return Err(refused("schema", "Unknown result fields"));
        }
        if map.get("verified").is_some_and(|value| !value.is_boolean())
            || map
                .get("progress")
                .is_some_and(|value| !value.is_null() && !value.is_string())
        {
            return Err(refused(
                "schema",
                "Progress must be text and verification must be boolean",
            ));
        }
        let progress = map
            .get("progress")
            .filter(|value| !value.is_null() && value.as_str() != Some(""))
            .map(|value| text(value, "progress", 1000))
            .transpose()?;
        let next_steps = map
            .get("next_steps")
            .map(|value| strings(value, "next steps", 3, 300))
            .transpose()?;
        let empty = vec![];
        let rows = map
            .get("learnings")
            .map(|value| {
                value
                    .as_array()
                    .ok_or_else(|| refused("schema", "Learnings must be an array"))
            })
            .transpose()?
            .unwrap_or(&empty);
        if rows.len() > 3 {
            return Err(refused("schema", "Keep at most 3 learnings"));
        }
        let mut learnings = Vec::new();
        let mut identities = BTreeMap::new();
        for row in rows {
            const FIELDS: [&str; 4] = ["consequence", "sources", "title", "type"];
            const OPTIONAL: [&str; 3] = ["expected_revision", "record_id", "source_fingerprints"];
            let Some(keys) = row.as_object() else {
                return Err(refused(
                    "learning_fields",
                    "Each learning needs type, title, consequence and sources",
                ));
            };
            let missing = FIELDS
                .iter()
                .filter(|key| !keys.contains_key(**key))
                .copied()
                .collect::<Vec<_>>();
            let mut extra = keys
                .keys()
                .filter(|key| !FIELDS.contains(&key.as_str()) && !OPTIONAL.contains(&key.as_str()))
                .map(|key| key.chars().take(40).collect::<String>())
                .collect::<Vec<_>>();
            if !missing.is_empty() || !extra.is_empty() {
                // Name what to correct, as the retired refusal did: `verified`
                // inside a learning is the usual one.
                extra.sort();
                let mut detail = String::new();
                if !missing.is_empty() {
                    detail.push_str(&format!("; missing: {}", missing.join(", ")));
                }
                if !extra.is_empty() {
                    detail.push_str(&format!(
                        "; unexpected: {}",
                        extra.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
                    ));
                }
                if extra.iter().any(|key| key == "verified") {
                    detail.push_str(" (verified is a top-level boolean, never a learning field)");
                }
                return Err(refused(
                    "learning_fields",
                    format!("Each learning needs type, title, consequence and sources{detail}"),
                ));
            }
            let kind = row["type"]
                .as_str()
                .filter(|kind| ["finding", "decision"].contains(kind))
                .ok_or_else(|| refused("learning_type", "Use finding or decision"))?
                .to_string();
            let raw_sources = strings(&row["sources"], "sources", 10, 1024)?;
            if raw_sources.is_empty() {
                return Err(refused("no_sources", "Cite sources for each learning"));
            }
            let sources = unique_strings(
                raw_sources
                    .iter()
                    .map(|value| self.source(value, check_files))
                    .collect::<Result<Vec<_>>>()?,
            );
            let expected_revision = keys
                .get("expected_revision")
                .map(|value| {
                    value.as_u64().filter(|value| *value > 0).ok_or_else(|| {
                        refused(
                            "schema",
                            "Learning expected_revision must be a positive original revision",
                        )
                    })
                })
                .transpose()?;
            let mut learning = Learning {
                kind,
                title: text(&row["title"], "title", 200)?,
                consequence: text(&row["consequence"], "consequence", 1000)?,
                sources,
                expected_revision,
                record_id: None,
                source_fingerprints: None,
            };
            if keys.contains_key("record_id") || keys.contains_key("source_fingerprints") {
                let id = row["record_id"]
                    .as_str()
                    .filter(|id| is_uuid4(id))
                    .ok_or_else(|| {
                        refused(
                            "reference",
                            "Existing knowledge needs its exact record_id and expected_revision",
                        )
                    })?;
                if expected_revision.is_none() {
                    return Err(refused(
                        "reference",
                        "Existing knowledge needs expected_revision",
                    ));
                }
                let pins = row["source_fingerprints"]
                    .as_array()
                    .filter(|pins| !pins.is_empty() && pins.len() <= 10)
                    .ok_or_else(|| {
                        refused(
                            "reference",
                            "Existing knowledge needs the checked source_fingerprints image",
                        )
                    })?;
                let mut normalized = Vec::new();
                for pin in pins {
                    if !result_store::exact(pin, &["path", "sha256"]) {
                        return Err(refused(
                            "reference",
                            "Invalid checked source_fingerprints image",
                        ));
                    }
                    let path = self.source(
                        pin["path"].as_str().ok_or_else(|| {
                            refused("reference", "Checked source path must be text")
                        })?,
                        false,
                    )?;
                    normalized.push(object!({"path":path,"sha256":pin["sha256"]}));
                }
                let image = canonical_image(&normalized)?;
                let wanted = learning
                    .sources
                    .iter()
                    .map(|source| source.split('#').next().unwrap_or(source))
                    .collect::<BTreeSet<_>>();
                let actual = image
                    .iter()
                    .filter_map(|row| row["path"].as_str())
                    .collect::<BTreeSet<_>>();
                if actual != wanted {
                    return Err(refused(
                        "reference",
                        "Check the complete source_fingerprints image",
                    ));
                }
                learning.record_id = Some(id.into());
                learning.source_fingerprints = Some(image);
            }
            let identity = learning_id("00000000-0000-4000-8000-000000000000", &learning)?;
            let wire = serde_json::to_value(&learning)?;
            if identities
                .get(&identity)
                .is_some_and(|previous| previous != &wire)
            {
                return Err(refused(
                    "conflicting_learnings",
                    format!(
                        "Conflicting learning identities in one result: two {}s titled {} cite the same sources; merge them",
                        learning.kind,
                        python_repr(&learning.title.chars().take(80).collect::<String>())
                    ),
                ));
            }
            identities.insert(identity, wire);
            learnings.push(learning);
        }
        if !learnings.is_empty() && data["verified"] != true {
            return Err(refused(
                "unverified",
                "Attest that each learning was checked against its sources with verified=true",
            ));
        }
        let capture_decision = map
            .get("capture_decision")
            .map(|value| {
                value
                    .as_str()
                    .filter(|value| {
                        [
                            "selected",
                            "no_reusable_knowledge",
                            "insufficiently_checked",
                        ]
                        .contains(value)
                    })
                    .map(String::from)
                    .ok_or_else(|| refused("capture_decision", "Invalid capture decision"))
            })
            .transpose()?;
        if capture_decision
            .as_ref()
            .is_some_and(|value| (value == "selected") != !learnings.is_empty())
        {
            return Err(refused(
                "capture_decision",
                "selected needs a checked learning; other capture decisions need empty learnings",
            ));
        }
        if progress.is_none() && next_steps.is_none() && learnings.is_empty() {
            return Err(refused(
                "nothing",
                "Nothing to record: give a progress summary, next steps or a checked learning",
            ));
        }
        Ok(NormalizedResult {
            progress,
            next_steps,
            learnings,
            capture_decision,
        })
    }
    pub fn scope(&self, reason: &str, attestation: &str) -> Result<String> {
        text(&object!(self.context.actor), "owner", 200)?;
        text(&object!(reason), "reason", 2000)?;
        if !["agent", "person"].contains(&attestation) {
            return Err(refused("schema", "attestation must be agent or person"));
        }
        let value = object!({"profile":"rust-memory-request-scope-v1","runtime":env!("CARGO_PKG_VERSION"),"engine_sha256":result_store::sha(concat!(include_str!("results.rs"),include_str!("unicode_fold.rs"),include_str!("result_store.rs"),include_str!("brain.rs"),include_str!("domain/json.rs"),include_str!("domain/sources.rs")).as_bytes()),"root":self.context.roots.state,"project":self.context.roots.project,"tooling":self.context.roots.tooling,"config":RuntimeConfig::load(&self.context.roots)?.value,"owner":self.context.actor,"reason":reason,"attestation":attestation});
        result_store::legacy_digest(&value, true, false)
    }
    pub fn task(&self, id: &str, archive: bool) -> Result<crate::brain::RecordImage> {
        let brain = self.brain()?;
        let task = if archive {
            if is_uuid4(&id.to_lowercase()) {
                brain.find(id, Some("task"), true)?
            } else {
                match brain.find(id, Some("task"), false) {
                    Ok(task) => task,
                    Err(error) if error.kind == ErrorKind::NotFound => {
                        brain.find(id, Some("task"), true)?
                    }
                    Err(error) => return Err(error),
                }
            }
        } else {
            brain.find(id, Some("task"), false)?
        };
        TaskService::new(self.context.clone(), None).authorized(task.record.clone(), false)?;
        Ok(task)
    }
    pub fn prepare(&self, input: PrepareResult) -> Result<Value> {
        let id = task_id(&input.task_id)?;
        let result = result_id(&input.result_id)?;
        if input.revision == 0 {
            return Err(refused("revision", "Supply the current task revision"));
        }
        let normalized = self.normalize(&input.data, false)?;
        let brain = self.brain()?;
        let _guard = brain.lock()?;
        let scope = self.scope(&input.reason, &input.attestation)?;
        let task = self.task(&id, false)?.record;
        if matches!(task["status"].as_str(), Some("completed" | "cancelled")) {
            return Err(refused(
                "task_closed",
                "Use an active task to record new memory",
            ));
        }
        if task["revision"].as_u64() != Some(input.revision) {
            return Err(result_store::fail(
                "memory-result-v1:prepared_stale_no_write",
            ));
        }
        let mut wire = object!({"learnings":normalized.learnings,"verified":true});
        if let Some(value) = &normalized.progress {
            wire["progress"] = value.clone().into();
        }
        if let Some(value) = &normalized.next_steps {
            wire["next_steps"] = object!(value);
        }
        if let Some(value) = &normalized.capture_decision {
            wire["capture_decision"] = value.clone().into();
        }
        Ok(
            object!({"profile":result_store::PREPARE_PROFILE,"task_id":task["id"],"result_id":result,"revision":input.revision,"owner":self.context.actor,"reason":input.reason,"attestation":input.attestation,"scope":scope,"digest":request_digest(task["id"].as_str().unwrap_or(""),&normalized)?,"request":wire}),
        )
    }
    fn sources_image(&self, result: &NormalizedResult) -> Result<String> {
        let mut images = Vec::new();
        for learning in &result.learnings {
            for source in &learning.sources {
                self.source(source, true)?;
            }
            images.push(object!([
                learning_id("00000000-0000-4000-8000-000000000000", learning)?,
                source_fingerprints(&self.context.roots, &learning.sources)?
            ]));
        }
        result_store::legacy_digest(&object!(images), true, false)
    }
    pub fn record(&self, input: RecordResult) -> Result<Value> {
        let id = task_id(&input.task_id)?;
        let rid = result_id(&input.result_id)?;
        let result = self.normalize(&input.data, false)?;
        if input.replay_only && input.prepared.is_none() {
            return Err(refused(
                "schema",
                "Receipt-only replay needs a prepared scope",
            ));
        }
        if input
            .prepared
            .as_deref()
            .is_some_and(|scope| !result_store::hex(scope, 64))
        {
            return Err(refused("schema", "Invalid prepared scope"));
        }
        let brain = self.brain()?;
        let _guard = brain.lock()?;
        let scope = self.scope(&input.reason, &input.attestation)?;
        if input
            .prepared
            .as_ref()
            .is_some_and(|prepared| prepared != &scope)
        {
            return Err(result_store::fail("memory-result-v1:scope_changed"));
        }
        let image = self.task(&id, true)?;
        let task = image.record;
        let canonical = task["id"]
            .as_str()
            .ok_or_else(|| refused("schema", "Task UUID is unavailable"))?
            .to_string();
        let mut revision = match input.revision {
            Revision::Auto => task["revision"].as_u64().unwrap_or(0),
            Revision::Exact(value) => value,
        };
        if revision == 0 {
            return Err(refused("revision", "Supply the current task revision"));
        }
        let digest = request_digest(&canonical, &result)?;
        let store = EvidenceStore::new(self.context.clone());
        let aliases = task["external_id"]
            .as_str()
            .map(|value| vec![value.into()])
            .unwrap_or_default();
        let (prior, receipt_present) = store.receipt(&canonical, &rid, &digest, &aliases)?;
        let mut prior = prior;
        let mut attempt = store.attempt(&canonical, &rid, &digest)?;
        let (mut resuming, mut restarted) = (false, false);
        if receipt_present && prior.is_none() && attempt.is_none() {
            return Err(result_store::fail("memory-result-v1:receipt_unavailable"));
        }
        if let Some(proof) = &attempt {
            if proof.owner != self.context.actor || proof.attestation != input.attestation {
                return Err(result_store::fail("memory-result-v1:attempt_scope_changed"));
            }
            if proof.state == "complete" {
                prior = Some(Receipt {
                    task_id: canonical.clone(),
                    result_id: rid.clone(),
                    digest: digest.clone(),
                    task_revision: proof.task_revision,
                });
            } else if prior.is_none() {
                resuming = proof.profile == result_store::FRONTIER_PROFILE
                    && proof.domain_sha256.is_some()
                    && proof.domain_sha256 == store.domain_image()
                    && proof.scope_sha256.as_deref() == Some(scope.as_str())
                    && proof.source_sha256.as_deref()
                        == Some(self.sources_image(&result)?.as_str());
                restarted = !resuming;
                if resuming {
                    revision = proof.original_revision.unwrap_or(revision);
                }
            }
        }
        if input.replay_only && prior.is_none() && !resuming && !restarted {
            return Err(result_store::fail("memory-result-v1:receipt_unavailable"));
        }
        if prior.is_none()
            && (image.archived
                || matches!(task["status"].as_str(), Some("completed" | "cancelled")))
        {
            return Err(refused(
                "task_closed",
                "Use an active task to record new memory",
            ));
        }
        let task_fields = result.progress.is_some() || result.next_steps.is_some();
        let holds = |record: &Value| -> bool {
            result
                .progress
                .as_ref()
                .is_none_or(|progress| record["progress"].as_str() == Some(progress))
                && result
                    .next_steps
                    .as_ref()
                    .is_none_or(|steps| record["next_steps"] == object!(steps))
        };
        let task_stage = if resuming {
            attempt
                .as_ref()
                .and_then(|value| value.stages.get("task"))
                .cloned()
        } else {
            None
        };
        if let Some(stage) = &task_stage
            && (stage.record_id != canonical
                || Some(stage.revision) != task["revision"].as_u64()
                || store.record_image(&canonical).as_deref() != Some(stage.sha256.as_str())
                || !holds(&task))
        {
            return Err(result_store::fail("memory-result-v1:attempt_incomplete"));
        }
        if prior.is_none()
            && task_stage.is_none()
            && task["revision"].as_u64() != Some(revision)
            && !(restarted
                && task_fields
                && holds(&task)
                && attempt.as_ref().and_then(|value| value.original_revision) == Some(revision))
        {
            return Err(if input.prepared.is_some() {
                result_store::fail("memory-result-v1:prepared_stale_no_write")
            } else {
                refused("stale_revision", "Stale task revision; nothing was written")
            });
        }
        let allowed = RuntimeConfig::load(&self.context.roots)?
            .strings("allowed_privacy")?
            .into_iter()
            .filter(|value| ["public", "team"].contains(&value.as_str()))
            .collect::<BTreeSet<_>>();
        let mut held = BTreeMap::<String, String>::new();
        let mut owned = BTreeSet::new();
        let mut implicit = BTreeSet::new();
        let mut plan = Vec::new();
        let mut resolved = BTreeMap::new();
        if prior.is_none() {
            for learning in &result.learnings {
                let key = learning_id(&canonical, learning)?;
                for source in &learning.sources {
                    self.source(source, true)?;
                }
                let fingerprints = canonical_image(&source_fingerprints(
                    &self.context.roots,
                    &learning.sources,
                )?)?;
                let target = learning.record_id.as_deref().unwrap_or(&key);
                let current = existing(&brain, target)?;
                let stage = if resuming {
                    attempt.as_ref().and_then(|value| value.stages.get(&key))
                } else {
                    None
                };
                if let Some(stage) = stage {
                    let image = current
                        .as_ref()
                        .ok_or_else(|| result_store::fail("memory-result-v1:attempt_incomplete"))?;
                    if image.archived
                        || image.record["id"] != stage.record_id
                        || image.record["revision"].as_u64() != Some(stage.revision)
                        || store.record_image(target).as_deref() != Some(stage.sha256.as_str())
                    {
                        return Err(result_store::fail("memory-result-v1:attempt_incomplete"));
                    }
                    if stage.done {
                        if image.record["status"] != closed(&learning.kind)
                            || image.record["authority"] != "verified"
                            || consequence(&image.record) != collapse(&learning.consequence)
                        {
                            return Err(result_store::fail("memory-result-v1:attempt_incomplete"));
                        }
                    } else if !own_open(&image.record, learning, &self.context.actor) {
                        return Err(result_store::fail("memory-result-v1:attempt_incomplete"));
                    } else {
                        owned.insert(key.clone());
                    }
                }
                let mut why = None;
                if let Some(image) = &current {
                    let record = &image.record;
                    let record_uuid = record["id"].as_str().unwrap_or("");
                    if resolved
                        .insert(record_uuid.to_string(), key.clone())
                        .is_some_and(|previous| previous != key)
                    {
                        return Err(refused(
                            "conflicting_learnings",
                            "One record was targeted by different learning identities",
                        ));
                    }
                    let unfinished = record["status"] == initial_state(&learning.kind)?
                        || (record["status"] == closed(&learning.kind)
                            && record["authority"] != "verified");
                    let same = consequence(record) == collapse(&learning.consequence);
                    if !allowed.contains(record["privacy"].as_str().unwrap_or("")) {
                        why = Some("not-authorized");
                    } else if image.archived {
                        why = Some("archived");
                    } else if record["type"] != learning.kind {
                        why = Some("kind-conflict");
                    } else if learning
                        .expected_revision
                        .is_some_and(|revision| record["revision"].as_u64() != Some(revision))
                    {
                        why = Some("revision-conflict");
                    } else if !authorized(record, &self.context.actor) {
                        why = Some("not-authorized");
                    } else if learning.record_id.is_some()
                        && (record["title"] != learning.title
                            || sorted_sources(record) != sorted(&learning.sources))
                    {
                        why = Some("reference-mismatch");
                    } else if learning.record_id.is_some()
                        && (record["status"] != closed(&learning.kind)
                            || record["authority"] != "verified")
                    {
                        why = Some("not-verified-terminal");
                    } else if input.attestation == "agent"
                        && record["authority"] == "verified"
                        && (learning.record_id.is_some() || unfinished || !same)
                        && record_attestation(record) != "agent"
                    {
                        why = Some("person-or-unknown-attestation");
                    } else if learning.expected_revision.is_none()
                        && (unfinished || !same)
                        && !owned.contains(&key)
                    {
                        let terminal = !unfinished && record["status"] == closed(&learning.kind);
                        if terminal
                            && (input.attestation == "person"
                                || record["authority"] == "verified"
                                    && record_attestation(record) == "agent"
                                    && store.owns_image(&key, record, &self.context.actor))
                        {
                            implicit.insert(key.clone());
                        } else if unfinished && own_open(record, learning, &self.context.actor) {
                            owned.insert(key.clone());
                        } else {
                            why = Some("original-image-unproven");
                        }
                    }
                } else if learning.record_id.is_some() {
                    why = Some("record-not-found");
                }
                if let Some(why) = why {
                    held.insert(key.clone(), why.into());
                }
                if learning.record_id.is_some()
                    && !held.contains_key(&key)
                    && !stage.is_some_and(|stage| stage.done)
                    && learning.source_fingerprints.as_ref() != Some(&fingerprints)
                {
                    return Err(refused(
                        "source_changed",
                        "Checked source image changed; inspect current sources again",
                    ));
                }
                plan.push(object!({"learning_id":key,"record_id":current.as_ref().map(|image|image.record["id"].clone()),"revision":current.as_ref().map(|image|image.record["revision"].clone()),"archived":current.as_ref().is_some_and(|image|image.archived),"fingerprints":fingerprints}));
            }
            if !resuming {
                let next = Attempt::new(
                    &canonical,
                    &rid,
                    &digest,
                    &self.context.actor,
                    &input.attestation,
                    revision,
                    result_store::legacy_digest(
                        &object!({"task":task,"learnings":plan}),
                        true,
                        false,
                    )?,
                    scope.clone(),
                    self.sources_image(&result)?,
                    store.domain_image(),
                );
                store.write_attempt(&next)?;
                attempt = Some(next);
            }
        }
        let mut records = Vec::new();
        let mut emitted = BTreeSet::new();
        let mut wrote = false;
        for learning in &result.learnings {
            let key = learning_id(&canonical, learning)?;
            if !emitted.insert(key.clone()) {
                continue;
            }
            let target = learning.record_id.as_deref().unwrap_or(&key);
            let current = existing(&brain, target)?;
            let stage = if resuming {
                attempt
                    .as_ref()
                    .and_then(|value| value.stages.get(&key))
                    .cloned()
            } else {
                None
            };
            if let Some(image) = &current {
                if !allowed.contains(image.record["privacy"].as_str().unwrap_or("")) {
                    records.push(summary(
                        Some(&image.record),
                        learning,
                        &key,
                        "differs",
                        Some("not-authorized"),
                    ));
                    continue;
                }
                if let Some(stage) = &stage
                    && stage.done
                {
                    records.push(summary(
                        Some(&image.record),
                        learning,
                        &key,
                        &stage.state,
                        None,
                    ));
                    continue;
                }
                if image.archived {
                    records.push(summary(
                        Some(&image.record),
                        learning,
                        &key,
                        if consequence(&image.record) == collapse(&learning.consequence) {
                            "archived"
                        } else {
                            "archived-differs"
                        },
                        None,
                    ));
                    continue;
                }
            }
            if let Some(why) = held.get(&key) {
                records.push(summary(
                    current.as_ref().map(|image| &image.record),
                    learning,
                    &key,
                    "differs",
                    Some(why),
                ));
                continue;
            }
            if prior.is_some() {
                let state = match &current {
                    None => "missing",
                    Some(image)
                        if consequence(&image.record) == collapse(&learning.consequence) =>
                    {
                        "existing"
                    }
                    _ => "differs",
                };
                records.push(summary(
                    current.as_ref().map(|image| &image.record),
                    learning,
                    &key,
                    state,
                    None,
                ));
                continue;
            }
            let created = current.is_none();
            let mut record = match current {
                Some(image) => image.record,
                None => {
                    let record = brain.create(CreateRecord {
                        record_type: learning.kind.clone(),
                        external_id: key.clone(),
                        title: learning.title.clone(),
                        sources: learning.sources.clone(),
                        owner: self.context.actor.clone(),
                        goal: Some(learning.consequence.clone()),
                        ..CreateRecord::default()
                    })?;
                    if let Some(attempt) = attempt.as_mut() {
                        store.checkpoint(attempt, &key, &record, "created", false)?;
                    }
                    record
                }
            };
            let mut state = if created { "created" } else { "existing" };
            let unfinished = record["status"] == initial_state(&learning.kind)?
                || (record["status"] == closed(&learning.kind)
                    && record["authority"] != "verified");
            let same = consequence(&record) == collapse(&learning.consequence);
            if unfinished
                && !created
                && learning.expected_revision.is_none()
                && !owned.contains(&key)
            {
                state = "differs";
            } else if unfinished {
                record = brain.update(
                    record["id"].as_str().unwrap_or(""),
                    UpdateRecord {
                        expected_revision: record["revision"].as_u64(),
                        progress: Some(learning.consequence.clone()),
                        actor: self.context.actor.clone(),
                        reason: input.reason.clone(),
                        authority: if record["authority"] != "verified" {
                            Some("verified".into())
                        } else {
                            None
                        },
                        transition_to: if record["status"] != closed(&learning.kind) {
                            Some(closed(&learning.kind).into())
                        } else {
                            None
                        },
                        reverify: !created && record["authority"] == "verified",
                        attestation: Some(input.attestation.clone()),
                        ..UpdateRecord::default()
                    },
                )?;
                state = if created { "created" } else { "completed" };
            } else if learning.record_id.is_some()
                && same
                && canonical_image(
                    record["source_fingerprints"]
                        .as_array()
                        .map(Vec::as_slice)
                        .unwrap_or(&[]),
                )? != learning.source_fingerprints.clone().unwrap_or_default()
            {
                record = brain.update(
                    record["id"].as_str().unwrap_or(""),
                    UpdateRecord {
                        expected_revision: record["revision"].as_u64(),
                        actor: self.context.actor.clone(),
                        reason: input.reason.clone(),
                        reverify: true,
                        attestation: Some(input.attestation.clone()),
                        checked_source_fingerprints: learning.source_fingerprints.clone(),
                        ..UpdateRecord::default()
                    },
                )?;
                state = "updated";
            } else if !same {
                if (learning.expected_revision.is_none() && !implicit.contains(&key))
                    || !(input.attestation == "person" || record_attestation(&record) == "agent")
                    || !authorized(&record, &self.context.actor)
                    || record["status"] != closed(&learning.kind)
                {
                    state = "differs";
                } else {
                    record = brain.update(
                        record["id"].as_str().unwrap_or(""),
                        UpdateRecord {
                            expected_revision: record["revision"].as_u64(),
                            progress: Some(learning.consequence.clone()),
                            actor: self.context.actor.clone(),
                            reason: input.reason.clone(),
                            reverify: true,
                            attestation: Some(input.attestation.clone()),
                            checked_source_fingerprints: learning.source_fingerprints.clone(),
                            ..UpdateRecord::default()
                        },
                    )?;
                    state = "updated";
                }
            }
            if matches!(state, "created" | "completed" | "updated") {
                if learning.record_id.is_none() {
                    store.remember_image(&key, &record, &self.context.actor);
                }
                if let Some(attempt) = attempt.as_mut() {
                    store.checkpoint(attempt, &key, &record, state, true)?;
                }
            }
            wrote |= !matches!(state, "existing" | "differs");
            records.push(summary(Some(&record), learning, &key, state, None));
        }
        let mut updated = if task_fields {
            Some(task.clone())
        } else {
            None
        };
        if task_fields && prior.is_none() && !holds(&task) && task_stage.is_none() {
            let record = brain.update(
                &canonical,
                UpdateRecord {
                    expected_revision: Some(revision),
                    progress: result.progress.clone(),
                    next_steps: result.next_steps.clone().unwrap_or_default(),
                    replace_next_steps: result.next_steps.is_some(),
                    actor: self.context.actor.clone(),
                    reason: input.reason.clone(),
                    ..UpdateRecord::default()
                },
            )?;
            if let Some(attempt) = attempt.as_mut() {
                store.checkpoint(attempt, "task", &record, "task", true)?;
            }
            updated = Some(record);
            wrote = true;
        }
        if let Some(attempt) = attempt.as_ref()
            && attempt.state == "started"
        {
            store.complete(attempt)?;
        }
        store.write_receipt(&Receipt {
            task_id: canonical,
            result_id: rid,
            digest,
            task_revision: updated
                .as_ref()
                .and_then(|record| record["revision"].as_u64()),
        })?;
        let promotion = if result.learnings.is_empty() {
            Value::Null
        } else {
            self.promotion_summary(
                BankService::new(self.context.clone()).auto_promote(
                    crate::bank::promotion::AUTOMATIC_PROMOTION_LIMIT,
                    None,
                    None,
                )?,
                &allowed,
            )?
        };
        let replayed = prior.is_some()
            || !wrote
                && !records.iter().any(|row| {
                    matches!(row["state"].as_str(), Some("differs" | "archived-differs"))
                });
        let mut output = object!({"task":updated.map(|record|object!({"id":record["id"],"revision":record["revision"]})),"records":records,"promotion":promotion,"attestation":input.attestation,"replayed":replayed});
        if let Some(decision) = result.capture_decision {
            output["capture_decision"] = decision.into();
        }
        Ok(output)
    }
    fn promotion_summary(&self, mut promotion: Value, allowed: &BTreeSet<String>) -> Result<Value> {
        let records = self.brain()?.records(true)?;
        let mut visible = BTreeMap::new();
        for image in records {
            let id = image.record["id"].as_str().unwrap_or("").to_string();
            let admitted = allowed.contains(image.record["privacy"].as_str().unwrap_or(""));
            if visible.insert(id.clone(), admitted).is_some() {
                visible.insert(id, false);
            }
        }
        let mut withheld = false;
        for field in ["promoted", "failed", "blocked", "closed"] {
            if let Some(rows) = promotion[field].as_array_mut() {
                let before = rows.len();
                rows.retain(|row| {
                    visible
                        .get(row["record_id"].as_str().unwrap_or(""))
                        .copied()
                        .unwrap_or(false)
                });
                withheld |= before != rows.len();
            }
        }
        if withheld {
            promotion["details_withheld"] = true.into();
        }
        Ok(promotion)
    }
}
fn existing(brain: &BrainRepository, id: &str) -> Result<Option<crate::brain::RecordImage>> {
    match brain.find(id, None, true) {
        Ok(image) => Ok(Some(image)),
        Err(error) if error.kind == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}
fn closed(kind: &str) -> &'static str {
    if kind == "finding" {
        "resolved"
    } else {
        "accepted"
    }
}
fn authorized(record: &Value, owner: &str) -> bool {
    record["authorized_owners"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|value| value.as_str() == Some(owner))
}
fn sorted(values: &[String]) -> Vec<String> {
    let mut values = values.to_vec();
    values.sort();
    values
}
fn sorted_sources(record: &Value) -> Vec<String> {
    let mut values = record["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect::<Vec<_>>();
    values.sort();
    values
}
fn own_open(record: &Value, learning: &Learning, owner: &str) -> bool {
    record["revision"] == 1
        && record["status"].as_str() == initial_state(&learning.kind).ok()
        && record["authority"] == "observed"
        && record["goal"] == learning.consequence
        && sorted_sources(record) == sorted(&learning.sources)
        && authorized(record, owner)
        && record["transitions"]
            .as_array()
            .is_some_and(|rows| rows.len() == 1 && rows[0]["actor"] == owner)
}
fn summary(
    record: Option<&Value>,
    learning: &Learning,
    key: &str,
    state: &str,
    reason: Option<&str>,
) -> Value {
    let mut value = match record {
        Some(record) => {
            object!({"id":record["id"],"external_id":record["external_id"],"type":record["type"],"title":record["title"],"status":record["status"],"revision":record["revision"],"state":state})
        }
        None => {
            object!({"id":null,"external_id":key,"type":learning.kind,"title":learning.title,"status":null,"revision":null,"state":state})
        }
    };
    if reason == Some("not-authorized") {
        value = object!({"id":null,"external_id":key,"type":learning.kind,"title":learning.title,"status":null,"revision":null,"state":"differs"});
    }
    if let Some(reason) = reason {
        value["reason"] = reason.into();
    }
    value
}
