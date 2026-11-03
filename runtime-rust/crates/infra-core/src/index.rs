//! Source discovery and the native SQLite FTS5 document index.
use crate::{
    BrainRepository, Result, RuntimeConfig, RuntimeContext, RuntimeError,
    domain::*,
    fs,
    query::sha256,
    storage::{Store, store_state},
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Exclusion {
    pub path: String,
    pub reason: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SourceStamp {
    pub mtime_ns: i64,
    pub size: i64,
    pub eligible_until: Option<String>,
}
pub type SourceState = BTreeMap<String, SourceStamp>;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Document {
    pub path: String,
    pub layer: String,
    pub kind: String,
    pub title: String,
    pub summary: String,
    pub content: String,
    pub category: String,
    pub privacy: String,
    pub owner: String,
    pub authority: String,
    pub lifecycle: String,
    pub source_hash: String,
    pub record_id: Option<String>,
    pub conflicts: Vec<String>,
    pub source_fingerprints: Value,
    pub updated_at: String,
    pub confidence: f64,
    pub attestation: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IndexReport {
    pub documents: usize,
    pub removed: usize,
    pub reused: usize,
    pub incremental: bool,
    pub layers: BTreeMap<String, usize>,
    pub brain: usize,
    pub excluded: Vec<Exclusion>,
    pub canonical_edition: String,
    pub parity_drift: Vec<Value>,
    #[serde(default)]
    pub phase_seconds: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub source_notices: BTreeMap<String, u64>,
    #[serde(default)]
    pub redacted: Vec<Value>,
}
#[derive(Default)]
pub struct Discovery {
    pub documents: Vec<Document>,
    pub retained: SourceState,
    pub state: SourceState,
    pub excluded: Vec<Exclusion>,
    pub links: Vec<(String, String, String)>,
    pub extra_state: BTreeMap<String, String>,
    pub source_notices: BTreeMap<String, u64>,
    pub redacted: Vec<Value>,
}

pub fn category_for(kind: &str) -> &'static str {
    match kind {
        "policy" | "skill" => "policy",
        "memory" => "durable",
        "brain-handoff" => "handoff",
        value if value.starts_with("brain-") => "dynamic",
        _ => "evidence",
    }
}
fn stamp(path: &Path) -> Result<SourceStamp> {
    let metadata = std::fs::symlink_metadata(path)?;
    if fs::linked_at(path, &metadata)? || !metadata.is_file() {
        return Err(RuntimeError::policy("Linked source is refused"));
    }
    let modified = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| RuntimeError::invalid("Invalid source timestamp"))?;
    Ok(SourceStamp {
        mtime_ns: i64::try_from(modified.as_nanos())
            .map_err(|_| RuntimeError::invalid("Source timestamp overflow"))?,
        size: i64::try_from(metadata.len())
            .map_err(|_| RuntimeError::invalid("Source size overflow"))?,
        eligible_until: None,
    })
}
fn walk_markdown(directory: &Path, output: &mut Vec<PathBuf>) -> Result<()> {
    if !fs::admit_directory(directory)? {
        return Ok(());
    }
    let mut children = std::fs::read_dir(directory)?.collect::<std::io::Result<Vec<_>>>()?;
    children.sort_by_key(|entry| entry.file_name());
    for entry in children {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        if fs::linked_at(&path, &metadata)? {
            continue;
        }
        if metadata.is_dir() {
            walk_markdown(&path, output)?;
        } else if metadata.is_file() && path.extension().is_some_and(|extension| extension == "md")
        {
            output.push(path);
        }
    }
    Ok(())
}
fn append_candidate(
    roots: &crate::Roots,
    base: &Path,
    path: PathBuf,
    layer: &str,
    kind: &str,
    output: &mut Vec<(String, PathBuf, String, String)>,
) -> Result<()> {
    if std::fs::symlink_metadata(&path).is_ok() {
        if path.to_str().is_none() {
            output.push((String::new(), path, layer.into(), kind.into()));
        } else {
            let key = roots.key_for(base, &path)?;
            output.push((key, path, layer.into(), kind.into()));
        }
    }
    Ok(())
}
fn candidates(context: &RuntimeContext) -> Result<Vec<(String, PathBuf, String, String)>> {
    let roots = &context.roots;
    let mut result = Vec::new();
    for name in ["AGENTS.md", "CLAUDE.md"] {
        append_candidate(
            roots,
            &roots.project,
            roots.project.join(name),
            "procedural",
            "policy",
            &mut result,
        )?;
    }
    if roots.attached() {
        for name in ["AGENTS.md", "CLAUDE.md"] {
            append_candidate(
                roots,
                &roots.tooling,
                roots.tooling.join(name),
                "procedural",
                "policy",
                &mut result,
            )?;
        }
    }
    for edition in [".agents", ".claude", ".cursor", ".codex"] {
        for base in if roots.attached() {
            vec![&roots.tooling, &roots.project]
        } else {
            vec![&roots.project]
        } {
            let mut paths = Vec::new();
            walk_markdown(&base.join(edition).join("skills"), &mut paths)?;
            for path in paths {
                append_candidate(roots, base, path, "procedural", "skill", &mut result)?;
            }
        }
    }
    append_candidate(
        roots,
        &roots.project,
        roots.project.join("README.md"),
        "semantic",
        "overview",
        &mut result,
    )?;
    for (tree, kind) in [
        ("specs", "spec"),
        ("docs", "spec"),
        ("codebase", "codebase"),
        ("tasks", "task"),
    ] {
        let mut paths = Vec::new();
        walk_markdown(&roots.project.join(tree), &mut paths)?;
        for path in paths {
            append_candidate(roots, &roots.project, path, "semantic", kind, &mut result)?;
        }
    }
    let chunk_root = roots.state.join("memory-bank/chunks");
    if fs::admit_directory(&chunk_root)? {
        let mut entries = std::fs::read_dir(&chunk_root)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if path.extension().is_some_and(|extension| extension == "md") {
                append_candidate(roots, &roots.state, path, "semantic", "memory", &mut result)?;
            }
        }
    }
    append_candidate(
        roots,
        &roots.project,
        roots.project.join("CHANGELOG.md"),
        "episodic",
        "changelog",
        &mut result,
    )?;
    Ok(result)
}

pub fn parse_memory_metadata(text: &str) -> Result<Value> {
    let text = text
        .strip_prefix("---\n")
        .ok_or_else(|| RuntimeError::invalid("Memory frontmatter is missing"))?;
    let (header, _) = text
        .split_once("\n---\n")
        .ok_or_else(|| RuntimeError::invalid("Memory frontmatter is unterminated"))?;
    if let Ok(value) = serde_json::from_str::<Value>(header)
        && value.is_object()
    {
        return Ok(value);
    }
    let mut object = serde_json::Map::new();
    let mut current = None::<String>;
    for line in header.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("- ") {
            let key = current
                .as_ref()
                .ok_or_else(|| RuntimeError::invalid("Invalid memory metadata list"))?;
            object
                .get_mut(key)
                .and_then(Value::as_array_mut)
                .ok_or_else(|| RuntimeError::invalid("Invalid memory metadata list"))?
                .push(parse_scalar(value));
        } else {
            let (key, value) = trimmed
                .split_once(':')
                .ok_or_else(|| RuntimeError::invalid("Invalid memory metadata"))?;
            if key.is_empty() || object.contains_key(key) {
                return Err(RuntimeError::invalid(
                    "Duplicate or empty memory metadata key",
                ));
            }
            current = Some(key.into());
            object.insert(
                key.into(),
                if value.trim().is_empty() {
                    json!([])
                } else {
                    parse_scalar(value.trim())
                },
            );
        }
    }
    Ok(Value::Object(object))
}
fn parse_scalar(value: &str) -> Value {
    serde_json::from_str(value)
        .unwrap_or_else(|_| Value::String(value.trim_matches('\'').trim_matches('"').into()))
}
pub fn memory_temporal_reason(metadata: &Value) -> Option<String> {
    if metadata["status"].as_str() != Some("active") {
        return Some(metadata["status"].as_str().unwrap_or("invalid").into());
    }
    let today = crate::domain::clock::latest_today();
    for (key, reason) in [("valid_to", "retired"), ("review_after", "overdue-review")] {
        if let Some(value) = metadata[key].as_str() {
            match chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d") {
                Ok(date) if today > date => return Some(reason.into()),
                Err(_) => return Some("invalid".into()),
                _ => {}
            }
        } else if key == "review_after" {
            return Some("invalid".into());
        }
    }
    if let Some(value) = metadata["valid_from"].as_str() {
        match chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d") {
            Ok(date) if today < date => return Some("not-yet-valid".into()),
            Err(_) => return Some("invalid".into()),
            _ => {}
        }
    }
    None
}
pub fn record_eligibility(
    context: &RuntimeContext,
    config: &RuntimeConfig,
    record: &Value,
) -> Result<Option<String>> {
    let privacy = record["privacy"].as_str().unwrap_or("");
    if !config
        .strings("allowed_privacy")?
        .iter()
        .any(|allowed| allowed == privacy)
    {
        return Ok(Some("privacy".into()));
    }
    let owners = config.strings("owners")?;
    if privacy == "restricted"
        && !owners
            .iter()
            .any(|owner| owner == "*" || Some(owner.as_str()) == record["owner"].as_str())
    {
        return Ok(Some("owner".into()));
    }
    if !config
        .strings("allowed_authority")?
        .iter()
        .any(|allowed| Some(allowed.as_str()) == record["authority"].as_str())
    {
        return Ok(Some("authority".into()));
    }
    if is_terminal(
        record["type"].as_str().unwrap_or(""),
        record["status"].as_str().unwrap_or(""),
    ) {
        return Ok(Some("lifecycle".into()));
    }
    if sensitive_label(&serde_json::to_string(record)?)?.is_some() {
        return Ok(Some("secret".into()));
    }
    if let Some(reason) = citation_problem(&context.roots, record)? {
        return Ok(Some(reason.into()));
    }
    Ok(None)
}
fn memory_fingerprints(metadata: &Value) -> Value {
    metadata
        .get("chunk_source_digests")
        .or_else(|| metadata.get("source_fingerprints"))
        .filter(|value| value.is_array())
        .cloned()
        .unwrap_or(json!([]))
}
fn links(document: &Document, sources: &Value, kind: &str) -> Vec<(String, String, String)> {
    sources
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|source| !source.starts_with("http://") && !source.starts_with("https://"))
        .map(|source| {
            (
                document.path.clone(),
                source
                    .split('#')
                    .next()
                    .unwrap_or(source)
                    .trim_start_matches("./")
                    .into(),
                kind.into(),
            )
        })
        .collect()
}
fn skill_identity(key: &str) -> Option<String> {
    let parts = key.split('/').collect::<Vec<_>>();
    parts
        .windows(2)
        .position(|parts| {
            [".agents", ".claude", ".cursor", ".codex"].contains(&parts[0]) && parts[1] == "skills"
        })
        .map(|index| parts[index + 2..].join("/"))
}
fn title(path: &Path, content: &str) -> String {
    content
        .lines()
        .find_map(|line| line.strip_prefix("# "))
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(String::from)
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("")
                .replace(['-', '_'], " ")
        })
}
fn summary(content: &str) -> String {
    if let Some(description) = content
        .lines()
        .find_map(|line| line.strip_prefix("description:"))
    {
        return description
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(400)
            .collect();
    }
    let body = content
        .strip_prefix("---\n")
        .and_then(|tail| tail.split_once("\n---\n").map(|(_, body)| body))
        .unwrap_or(content);
    body.lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(400)
        .collect()
}

pub fn discover_sources(context: &RuntimeContext, reusable: &SourceState) -> Result<Discovery> {
    let config = RuntimeConfig::load(&context.roots)?;
    let candidates = candidates(context)?;
    let mut result = Discovery::default();
    let ignored = if let Some(frame) = crate::git::GitFrame::discover(&context.roots.project)? {
        let keys = candidates
            .iter()
            .filter(|(key, _, _, _)| {
                !key.is_empty()
                    && !Path::new(key).is_absolute()
                    && !crate::workspace::accelerator_owned_source(key)
            })
            .map(|(key, _, _, _)| key.clone())
            .collect::<Vec<_>>();
        frame.ignored(&keys)?.into_iter().collect::<BTreeSet<_>>()
    } else {
        BTreeSet::new()
    };
    let mut claimed = BTreeSet::new();
    let mut skills = BTreeSet::new();
    for (key, path, layer, kind) in candidates {
        if key.is_empty() {
            *result
                .source_notices
                .entry("invalid-utf8-path".into())
                .or_default() += 1;
            continue;
        }
        if ignored.contains(&key) || !claimed.insert(key.clone()) {
            continue;
        }
        let skill = if kind == "skill" {
            skill_identity(&key)
        } else {
            None
        };
        if skill
            .as_ref()
            .is_some_and(|identity| skills.contains(identity))
        {
            continue;
        }
        let mut current = match stamp(&path) {
            Ok(current) => current,
            Err(_) => {
                result.excluded.push(Exclusion {
                    path: key,
                    reason: "unsafe-source".into(),
                });
                continue;
            }
        };
        if kind != "memory"
            && reusable.get(&key).is_some_and(|cached| {
                cached.mtime_ns == current.mtime_ns && cached.size == current.size
            })
        {
            result.retained.insert(key.clone(), current.clone());
            result.state.insert(key, current);
            if let Some(skill) = skill {
                skills.insert(skill);
            }
            continue;
        }
        let bytes = match fs::read_regular(&path) {
            Ok(bytes) => bytes,
            Err(_) => {
                result.excluded.push(Exclusion {
                    path: key,
                    reason: "unsafe-source".into(),
                });
                continue;
            }
        };
        let raw = match String::from_utf8(bytes) {
            Ok(raw) => raw.replace("\r\n", "\n").replace('\r', "\n"),
            Err(_) => {
                *result
                    .source_notices
                    .entry("invalid-utf8".into())
                    .or_default() += 1;
                result.excluded.push(Exclusion {
                    path: key,
                    reason: "invalid-utf8".into(),
                });
                continue;
            }
        };
        let metadata = if kind == "memory" {
            match parse_memory_metadata(&raw) {
                Ok(meta) => Some(meta),
                Err(_) => {
                    result.excluded.push(Exclusion {
                        path: key,
                        reason: "invalid".into(),
                    });
                    continue;
                }
            }
        } else {
            None
        };
        if let Some(metadata) = &metadata {
            if let Some(reason) = memory_temporal_reason(metadata) {
                result.excluded.push(Exclusion { path: key, reason });
                continue;
            }
            if crate::bank::validation::metadata(
                &crate::BankService::new(context.clone()),
                &key,
                metadata,
                false,
            )
            .is_err()
            {
                result.excluded.push(Exclusion {
                    path: key,
                    reason: "invalid".into(),
                });
                continue;
            }
            if crate::domain::indexed_secret_policy_reason(&[&raw]).is_some() {
                result.excluded.push(Exclusion {
                    path: key,
                    reason: "secret".into(),
                });
                continue;
            }
            let fingerprints = memory_fingerprints(metadata);
            let (_, missing) = source_changes(&context.roots, &fingerprints)?;
            if !missing.is_empty() {
                result.excluded.push(Exclusion {
                    path: key,
                    reason: "source-missing".into(),
                });
                continue;
            }
            current.eligible_until = ["review_after", "valid_to"]
                .iter()
                .filter_map(|name| metadata[*name].as_str())
                .min()
                .map(String::from);
        }
        if kind == "memory"
            && reusable.get(&key).is_some_and(|cached| {
                cached.mtime_ns == current.mtime_ns
                    && cached.size == current.size
                    && cached.eligible_until == current.eligible_until
            })
        {
            result.retained.insert(key.clone(), current.clone());
            result.state.insert(key, current);
            continue;
        }
        if let Ok((meta, _)) = parse_markdown_record(&raw)
            && meta["type"] == "context-handoff"
        {
            result.excluded.push(Exclusion {
                path: key,
                reason: "explicit-context-only".into(),
            });
            continue;
        }
        let content = if metadata.is_none() {
            match crate::domain::mask_secrets(&raw) {
                Ok((masked, count)) => {
                    if count > 0 {
                        result.redacted.push(json!({"path":key,"values":count}));
                    }
                    masked
                }
                Err(_) => {
                    result.excluded.push(Exclusion {
                        path: key,
                        reason: "secret".into(),
                    });
                    continue;
                }
            }
        } else {
            raw.clone()
        };
        let document = Document {
            path: key.clone(),
            layer,
            kind: kind.clone(),
            title: title(&path, &content),
            summary: summary(&content),
            content,
            category: category_for(&kind).into(),
            privacy: "public".into(),
            owner: "*".into(),
            authority: "verified".into(),
            lifecycle: "active".into(),
            source_hash: sha256(raw.as_bytes()),
            record_id: None,
            conflicts: vec![],
            source_fingerprints: metadata
                .as_ref()
                .map(memory_fingerprints)
                .unwrap_or(json!([])),
            updated_at: String::new(),
            confidence: 1.0,
            attestation: if metadata
                .as_ref()
                .and_then(|metadata| metadata["tags"].as_array())
                .is_some_and(|tags| tags.iter().any(|tag| tag == "agent-attested"))
            {
                "agent".into()
            } else {
                String::new()
            },
        };
        if let Some(metadata) = metadata {
            result
                .links
                .extend(links(&document, &metadata["sources"], "source"));
        }
        if let Some(skill) = skill {
            skills.insert(skill);
        }
        result.state.insert(key, current);
        result.documents.push(document);
    }
    append_brain(context, &config, &mut result)?;
    Ok(result)
}

fn append_brain(
    context: &RuntimeContext,
    config: &RuntimeConfig,
    result: &mut Discovery,
) -> Result<()> {
    let brain = BrainRepository::new(context.clone())?;
    let inventory = brain.inventory(false)?;
    let mut tasks = BTreeMap::<String, Value>::new();
    let mut twins = BTreeMap::<String, Vec<(String, String)>>::new();
    let mut prefixes = BTreeSet::new();
    for image in inventory.records {
        let key = image
            .path
            .strip_prefix(&context.roots.state)
            .map_err(|_| RuntimeError::policy("Brain source escapes state"))?
            .to_str()
            .ok_or_else(|| RuntimeError::invalid("Brain source path is not UTF-8"))?
            .replace('\\', "/");
        let record = &image.record;
        if record["type"] == "task" && !is_terminal("task", record["status"].as_str().unwrap_or(""))
        {
            twins
                .entry(record["external_id"].as_str().unwrap_or("").into())
                .or_default()
                .push((
                    record["created_at"].as_str().unwrap_or("").into(),
                    record["id"].as_str().unwrap_or("").into(),
                ));
        }
        if let Some(reason) = record_eligibility(context, config, record)? {
            result.excluded.push(Exclusion { path: key, reason });
            continue;
        }
        let raw = fs::read_text(&image.path)?;
        if sensitive_label(&raw)?.is_some() {
            result.excluded.push(Exclusion {
                path: key,
                reason: "secret".into(),
            });
            continue;
        }
        let document = Document {
            path: key,
            layer: if record["type"] == "event" {
                "episodic"
            } else {
                "semantic"
            }
            .into(),
            kind: format!("brain-{}", record["type"].as_str().unwrap_or("")),
            title: record["title"].as_str().unwrap_or("").into(),
            summary: format!(
                "{} {}",
                record["title"].as_str().unwrap_or(""),
                record["goal"].as_str().unwrap_or("")
            ),
            content: image.body,
            category: "dynamic".into(),
            privacy: record["privacy"].as_str().unwrap_or("").into(),
            owner: record["owner"].as_str().unwrap_or("").into(),
            authority: record["authority"].as_str().unwrap_or("").into(),
            lifecycle: record["status"].as_str().unwrap_or("").into(),
            source_hash: sha256(raw.as_bytes()),
            record_id: record["id"].as_str().map(String::from),
            conflicts: strings(record, "conflicts"),
            source_fingerprints: record["source_fingerprints"].clone(),
            updated_at: record["updated_at"].as_str().unwrap_or("").into(),
            confidence: record["confidence"].as_f64().unwrap_or(1.0),
            attestation: record_attestation(record).into(),
        };
        result
            .links
            .extend(links(&document, &record["sources"], "source"));
        if record["type"] == "task" {
            let files = strings(record, "files");
            let mut admissible = Vec::new();
            for file in files.iter().rev().take(50) {
                if source_path_problem(file).is_none() {
                    admissible.push(file.clone());
                }
            }
            if !admissible.is_empty() {
                let mut arguments = crate::git::strings(&["ls-files", "-z", "--"]);
                arguments.extend(admissible.iter().map(std::ffi::OsString::from));
                let tracked = crate::git::run(
                    &context.roots.project,
                    &arguments,
                    None,
                    std::time::Duration::from_secs(3),
                    8 * 1024 * 1024,
                )?;
                if tracked.code == 0 {
                    for path in tracked
                        .stdout
                        .split(|byte| *byte == 0)
                        .filter(|path| !path.is_empty())
                    {
                        let path = std::str::from_utf8(path)
                            .map_err(|_| RuntimeError::invalid("Tracked source is not UTF-8"))?;
                        result
                            .links
                            .push((document.path.clone(), path.into(), "file".into()));
                    }
                }
            }
            tasks.insert(record["id"].as_str().unwrap_or("").into(), record.clone());
        }
        prefixes.extend(crate::history::work_prefixes(record));
        result.documents.push(document);
    }
    for problem in inventory.problems {
        result.excluded.push(Exclusion {
            path: problem
                .path
                .strip_prefix(&context.roots.state)
                .unwrap_or(&problem.path)
                .to_string_lossy()
                .replace('\\', "/"),
            reason: "invalid".into(),
        });
    }
    let handoffs = context.roots.brain_root().join("control/handoffs");
    if fs::admit_directory(&handoffs)? {
        let mut paths = Vec::new();
        walk_markdown(&handoffs, &mut paths)?;
        for path in paths {
            let key = path
                .strip_prefix(&context.roots.state)
                .map_err(|_| RuntimeError::policy("Handoff escapes state"))?
                .to_string_lossy()
                .replace('\\', "/");
            let raw = match fs::read_text(&path) {
                Ok(raw) => raw,
                Err(_) => {
                    result.excluded.push(Exclusion {
                        path: key,
                        reason: "invalid-utf8".into(),
                    });
                    continue;
                }
            };
            let (handoff, body) = match parse_markdown_record(&raw) {
                Ok(value) => value,
                Err(_) => {
                    result.excluded.push(Exclusion {
                        path: key,
                        reason: "task-filter-or-invalid".into(),
                    });
                    continue;
                }
            };
            let Some(task) = handoff["task_id"].as_str().and_then(|id| tasks.get(id)) else {
                result.excluded.push(Exclusion {
                    path: key,
                    reason: "task-filter-or-invalid".into(),
                });
                continue;
            };
            if validate_handoff(&handoff, Some(task)).is_err() || sensitive_label(&raw)?.is_some() {
                result.excluded.push(Exclusion {
                    path: key,
                    reason: "task-filter-or-invalid".into(),
                });
                continue;
            }
            let title = format!("Handoff {}", task["external_id"].as_str().unwrap_or(""));
            let document = Document {
                path: key,
                layer: "semantic".into(),
                kind: "brain-handoff".into(),
                summary: title.clone(),
                title,
                content: body,
                category: "handoff".into(),
                privacy: task["privacy"].as_str().unwrap_or("").into(),
                owner: task["owner"].as_str().unwrap_or("").into(),
                authority: task["authority"].as_str().unwrap_or("").into(),
                lifecycle: handoff["status"].as_str().unwrap_or("").into(),
                source_hash: sha256(raw.as_bytes()),
                record_id: handoff["id"].as_str().map(String::from),
                conflicts: vec![],
                source_fingerprints: task["source_fingerprints"].clone(),
                updated_at: handoff["updated_at"].as_str().unwrap_or("").into(),
                confidence: task["confidence"].as_f64().unwrap_or(1.0),
                attestation: String::new(),
            };
            result
                .links
                .extend(links(&document, &handoff["sources"], "source"));
            result.documents.push(document);
        }
    }
    let twins = twins
        .into_iter()
        .filter_map(|(external, mut values)| {
            if values.len() < 2 {
                return None;
            }
            values.sort();
            Some((
                external,
                values.into_iter().map(|(_, id)| id).collect::<Vec<_>>(),
            ))
        })
        .collect::<BTreeMap<_, _>>();
    result
        .extra_state
        .insert("task-twins".into(), serde_json::to_string(&twins)?);
    result
        .extra_state
        .insert("ticket-prefixes".into(), serde_json::to_string(&prefixes)?);
    Ok(())
}
fn strings(value: &Value, key: &str) -> Vec<String> {
    value[key]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect()
}
pub fn load_source_state(conn: &Connection) -> Result<SourceState> {
    let mut statement =
        conn.prepare("SELECT path,mtime_ns,size,eligible_until FROM document_source_state")?;
    Ok(statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                SourceStamp {
                    mtime_ns: row.get(1)?,
                    size: row.get(2)?,
                    eligible_until: row.get(3)?,
                },
            ))
        })?
        .collect::<std::result::Result<_, _>>()?)
}

pub fn refresh_index(
    store: &mut Store,
    context: &RuntimeContext,
    incremental: bool,
) -> Result<IndexReport> {
    if store.read_only {
        return Err(RuntimeError::unavailable("SQLite writes unavailable"));
    }
    let scan_started = std::time::Instant::now();
    let config = RuntimeConfig::load(&context.roots)?;
    let stored = store.state()?;
    let config_fingerprint = sha256(serde_json::to_string(&config.value)?.as_bytes());
    let policy_fingerprint = crate::domain::secret_policy_fingerprint()?;
    let effective = effective_canonical_edition(
        &context.roots.tooling,
        config.value["canonical_edition"]
            .as_str()
            .unwrap_or(".agents"),
    );
    let canonical = effective.as_str();
    let (parity_drift, skill_fingerprint) = skill_parity(&context.roots, canonical)?;
    let reusable = if incremental
        && stored.get("config-fingerprint") == Some(&config_fingerprint)
        && stored.get("secret-policy-fingerprint") == Some(&policy_fingerprint)
        && stored.get("skill-tree-fingerprint") == Some(&skill_fingerprint)
    {
        load_source_state(&store.conn)?
    } else {
        BTreeMap::new()
    };
    let discovery = discover_sources(context, &reusable)?;
    let stat_seconds = scan_started.elapsed().as_secs_f64();
    let index_started = std::time::Instant::now();
    let changed = discovery
        .documents
        .iter()
        .map(|doc| doc.path.clone())
        .collect::<BTreeSet<_>>();
    let retained = discovery.retained.keys().cloned().collect::<BTreeSet<_>>();
    let current = changed.union(&retained).cloned().collect::<BTreeSet<_>>();
    let brain_documents = discovery
        .documents
        .iter()
        .filter(|doc| doc.kind.starts_with("brain-"))
        .collect::<Vec<_>>();
    let brain_digest = sha256(serde_json::to_string(&brain_documents)?.as_bytes());
    let existing = paths(&store.conn)?;
    let unchanged = existing == current
        && discovery
            .documents
            .iter()
            .all(|doc| doc.kind.starts_with("brain-"))
        && stored.get("brain-rows-digest") == Some(&brain_digest)
        && stored.get("config-fingerprint") == Some(&config_fingerprint)
        && stored.get("secret-policy-fingerprint") == Some(&policy_fingerprint)
        && stored.get("skill-tree-fingerprint") == Some(&skill_fingerprint);
    let removed = existing.difference(&current).count();
    if !unchanged {
        let rebuilt=store.transaction(|conn|{
            let latest=paths(conn)?;if !retained.is_subset(&latest){return Err(RuntimeError::coded(crate::ErrorKind::Conflict,"index-cache-inconsistent","Index cache is inconsistent; a full refresh is required"));}
            for path in latest.difference(&retained){for table in ["documents","document_metadata","document_links","identifier_terms"]{conn.execute(&format!("DELETE FROM {table} WHERE path=?1"),[path])?;}}
            for document in &discovery.documents{
                conn.execute("INSERT INTO documents(path,layer,kind,title,summary,content) VALUES(?1,?2,?3,?4,?5,?6)",params![document.path,document.layer,document.kind,document.title,document.summary,document.content])?;
                conn.execute("INSERT INTO document_metadata(path,category,privacy,owner,authority,lifecycle,source_hash,record_id,conflicts,source_fingerprints,updated_at,confidence,attestation) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",params![document.path,document.category,document.privacy,document.owner,document.authority,document.lifecycle,document.source_hash,document.record_id,serde_json::to_string(&document.conflicts)?,serde_json::to_string(&document.source_fingerprints)?,document.updated_at,document.confidence,document.attestation])?;
                for (identifier,expansion) in crate::query::identifier_terms(&document.path,&document.content){conn.execute("INSERT OR REPLACE INTO identifier_terms(path,identifier,expansion) VALUES(?1,?2,?3)",params![document.path,identifier,expansion])?;}
            }
            for (path,reference,kind) in &discovery.links{conn.execute("INSERT OR REPLACE INTO document_links(path,ref_path,ref_kind) VALUES(?1,?2,?3)",params![path,reference,kind])?;}
            conn.execute("DELETE FROM document_source_state",[])?;
            for (path,stamp) in &discovery.state{conn.execute("INSERT INTO document_source_state(path,mtime_ns,size,eligible_until) VALUES(?1,?2,?3,?4)",params![path,stamp.mtime_ns,stamp.size,stamp.eligible_until])?;}
            let mut state=discovery.extra_state.clone();state.insert("config-fingerprint".into(),config_fingerprint.clone());state.insert("skill-tree-fingerprint".into(),skill_fingerprint.clone());state.insert("secret-policy-fingerprint".into(),policy_fingerprint.clone());state.insert("provenance-policy-fingerprint".into(),sha256(include_str!("domain/sources.rs").as_bytes()));state.insert("brain-rows-digest".into(),brain_digest.clone());state.insert("index-generation".into(),uuid4());state.insert("identifier_terms_ready".into(),"1".into());state.insert("skill-parity-drift".into(),serde_json::to_string(&parity_drift)?);store_state(conn,&state)?;Ok(())
        });
        if let Err(error) = rebuilt {
            if incremental && error.code.as_deref() == Some("index-cache-inconsistent") {
                let failed_index_seconds = index_started.elapsed().as_secs_f64();
                let mut report = refresh_index(store, context, false)?;
                report.incremental = true;
                let previous_index = report.phase_seconds.get("index").copied().unwrap_or(0.0);
                report.phase_seconds.insert(
                    "index".into(),
                    ((previous_index + failed_index_seconds) * 1_000_000.0).round() / 1_000_000.0,
                );
                let previous = report.phase_seconds.get("stat").copied().unwrap_or(0.0);
                report.phase_seconds.insert(
                    "stat".into(),
                    ((previous + stat_seconds) * 1_000_000.0).round() / 1_000_000.0,
                );
                return Ok(report);
            }
            return Err(error);
        }
    }
    let mut layers = BTreeMap::new();
    let mut statement = store
        .conn
        .prepare("SELECT layer,COUNT(*) FROM documents GROUP BY layer")?;
    for row in statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
    })? {
        let (layer, count) = row?;
        layers.insert(layer, count);
    }
    let documents = layers.values().sum();
    let mut redacted = discovery.redacted;
    redacted.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    Ok(IndexReport {
        documents,
        removed: if unchanged { 0 } else { removed },
        reused: retained.len(),
        incremental,
        layers,
        brain: brain_documents.len(),
        excluded: discovery.excluded,
        canonical_edition: canonical.into(),
        parity_drift,
        phase_seconds: BTreeMap::from([
            (
                "stat".into(),
                (stat_seconds * 1_000_000.0).round() / 1_000_000.0,
            ),
            (
                "index".into(),
                (index_started.elapsed().as_secs_f64() * 1_000_000.0).round() / 1_000_000.0,
            ),
        ]),
        source_notices: discovery.source_notices,
        redacted,
    })
}
pub fn effective_canonical_edition(repository: &Path, configured: &str) -> String {
    if repository.join(configured).join("skills").is_dir() {
        return configured.into();
    }
    for edition in [".agents", ".claude", ".cursor", ".codex"] {
        if repository.join(edition).join("skills").is_dir() {
            return edition.into();
        }
    }
    configured.into()
}
pub fn skill_parity(roots: &crate::Roots, canonical: &str) -> Result<(Vec<Value>, String)> {
    let effective = effective_canonical_edition(&roots.tooling, canonical);
    let canonical = effective.as_str();
    let mut logical = BTreeMap::<String, BTreeMap<String, Vec<u8>>>::new();
    let mut editions = BTreeSet::new();
    for edition in [".agents", ".claude", ".cursor", ".codex"] {
        let directory = roots.tooling.join(edition).join("skills");
        if !fs::admit_directory(&directory)? {
            continue;
        }
        editions.insert(edition.to_string());
        let mut paths = Vec::new();
        walk_markdown(&directory, &mut paths)?;
        for path in paths {
            let relative = path
                .strip_prefix(&directory)
                .map_err(|_| RuntimeError::policy("Skill path escapes root"))?;
            let Some(relative) = relative.to_str() else {
                continue;
            };
            let key = relative.replace('\\', "/");
            if key == "SKILL FLOW.md" || key.starts_with("skill-creator/") {
                continue;
            }
            logical
                .entry(key)
                .or_default()
                .insert(edition.into(), fs::read_regular(&path)?);
        }
    }
    let fingerprint = sha256(serde_json::to_string(&logical)?.as_bytes());
    let mut drift = Vec::new();
    for (key, copies) in logical {
        let Some(authority) = copies.get(canonical) else {
            drift.push(json!({"logical_path":key,"canonical":canonical,"mismatched":copies.keys().collect::<Vec<_>>(),"reason":"absent from canonical"}));
            continue;
        };
        let mismatched = copies
            .iter()
            .filter(|(edition, bytes)| edition.as_str() != canonical && *bytes != authority)
            .map(|(edition, _)| edition.clone())
            .collect::<BTreeSet<_>>();
        let missing = editions
            .iter()
            .filter(|edition| !copies.contains_key(*edition))
            .cloned()
            .collect::<Vec<_>>();
        if !mismatched.is_empty() || !missing.is_empty() {
            let all = mismatched
                .iter()
                .cloned()
                .chain(missing.iter().cloned())
                .collect::<BTreeSet<_>>();
            let mut entry = json!({"logical_path":key,"canonical":canonical,"mismatched":all,"reason":if !mismatched.is_empty(){"content differs"}else{"missing from mirror"}});
            if !missing.is_empty() {
                entry["missing"] = json!(missing);
            }
            drift.push(entry);
        }
    }
    Ok((drift, fingerprint))
}
fn paths(conn: &Connection) -> Result<BTreeSet<String>> {
    let mut statement = conn.prepare("SELECT path FROM documents")?;
    Ok(statement
        .query_map([], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()?)
}

pub(crate) fn document_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Document> {
    let conflicts: String = row.get("conflicts")?;
    let fingerprints: String = row.get("source_fingerprints")?;
    let invalid = |error: serde_json::Error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    };
    Ok(Document {
        path: row.get("path")?,
        layer: row.get("layer")?,
        kind: row.get("kind")?,
        title: row.get("title")?,
        summary: row.get("summary")?,
        content: row.get("content")?,
        category: row.get("category")?,
        privacy: row.get("privacy")?,
        owner: row.get("owner")?,
        authority: row.get("authority")?,
        lifecycle: row.get("lifecycle")?,
        source_hash: row.get("source_hash")?,
        record_id: row.get("record_id")?,
        conflicts: serde_json::from_str(&conflicts).map_err(invalid)?,
        source_fingerprints: serde_json::from_str(&fingerprints).map_err(invalid)?,
        updated_at: row.get("updated_at")?,
        confidence: row.get("confidence")?,
        attestation: row.get("attestation")?,
    })
}
pub(crate) const DOCUMENT_SELECT: &str = "d.path,d.layer,d.kind,d.title,d.summary,d.content,m.category,m.privacy,m.owner,m.authority,m.lifecycle,m.source_hash,m.record_id,m.conflicts,m.source_fingerprints,m.updated_at,m.confidence,m.attestation";

pub fn load_document(conn: &Connection, path: &str) -> Result<Option<Document>> {
    use rusqlite::OptionalExtension;
    Ok(conn.query_row(&format!("SELECT {DOCUMENT_SELECT} FROM documents d JOIN document_metadata m ON m.path=d.path WHERE d.path=?1"),[path],document_from_row).optional()?)
}
