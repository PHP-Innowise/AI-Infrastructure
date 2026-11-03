use super::{is_uuid4, valid_datetime};
use crate::{Result, RuntimeError};
use serde_json::Value;
use std::collections::BTreeSet;

pub const RECORD_TYPES: &[&str] = &["task", "finding", "bug", "incident", "decision", "event"];
pub const PHASES: &[&str] = &[
    "understanding",
    "planning",
    "implementation",
    "verification",
    "finalization",
];
pub const PRIVACY: &[&str] = &["public", "team", "restricted", "private"];
pub const AUTHORITIES: &[&str] = &["inferred", "observed", "verified"];

pub fn utc_now() -> String {
    super::clock::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, false)
}
pub fn uuid4() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn normalize_phase(phase: &str) -> Result<&str> {
    let phase = match phase {
        "implementing" | "execution" => "implementation",
        "review" | "quality" => "verification",
        value => value,
    };
    if !PHASES.contains(&phase) {
        return Err(RuntimeError::invalid("Invalid task phase"));
    }
    Ok(phase)
}

pub fn initial_state(kind: &str) -> Result<&'static str> {
    match kind {
        "task" => Ok("active"),
        "finding" | "incident" => Ok("open"),
        "bug" => Ok("reported"),
        "decision" => Ok("proposed"),
        "event" => Ok("recorded"),
        _ => Err(RuntimeError::invalid("Invalid Brain record type")),
    }
}

pub fn lifecycle_states(kind: &str) -> &'static [&'static str] {
    match kind {
        "task" => &["active", "blocked", "verifying", "completed", "cancelled"],
        "finding" => &["open", "investigating", "resolved", "superseded"],
        "bug" => &[
            "reported",
            "triaged",
            "fixing",
            "verifying",
            "resolved",
            "cancelled",
        ],
        "incident" => &[
            "open",
            "contained",
            "recovering",
            "resolved",
            "closed",
            "cancelled",
        ],
        "decision" => &["proposed", "accepted", "rejected", "superseded"],
        "event" => &["recorded", "superseded"],
        _ => &[],
    }
}

pub fn is_terminal(kind: &str, status: &str) -> bool {
    match kind {
        "task" => ["completed", "cancelled"].contains(&status),
        "finding" => ["resolved", "superseded"].contains(&status),
        "bug" => ["resolved", "cancelled"].contains(&status),
        "incident" => ["closed", "cancelled"].contains(&status),
        "decision" => ["rejected", "superseded"].contains(&status),
        "event" => status == "superseded",
        _ => false,
    }
}

pub fn transition_allowed(kind: &str, from: &str, to: &str) -> bool {
    match (kind, from) {
        ("task", "active") => ["blocked", "verifying", "completed", "cancelled"].contains(&to),
        ("task", "blocked") => ["active", "cancelled"].contains(&to),
        ("task", "verifying") => ["active", "completed", "cancelled"].contains(&to),
        ("finding", "open") => ["investigating", "resolved", "superseded"].contains(&to),
        ("finding", "investigating") => ["open", "resolved", "superseded"].contains(&to),
        ("bug", "reported") => ["triaged", "cancelled"].contains(&to),
        ("bug", "triaged") => ["fixing", "cancelled"].contains(&to),
        ("bug", "fixing") => ["verifying", "triaged", "cancelled"].contains(&to),
        ("bug", "verifying") => ["fixing", "resolved", "cancelled"].contains(&to),
        ("incident", "open") => ["contained", "cancelled"].contains(&to),
        ("incident", "contained") => ["recovering", "resolved"].contains(&to),
        ("incident", "recovering") => ["contained", "resolved"].contains(&to),
        ("incident", "resolved") => to == "closed",
        ("decision", "proposed") => ["accepted", "rejected"].contains(&to),
        ("decision", "accepted") | ("event", "recorded") => to == "superseded",
        _ => false,
    }
}

pub fn unique_strings(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

pub(crate) fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| RuntimeError::invalid(format!("{key} must be a non-empty string")))
}

pub(crate) fn string_list(value: &Value, key: &str, unique: bool) -> Result<Vec<String>> {
    let items = value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| RuntimeError::invalid(format!("{key} must be a string list")))?;
    let mut output: Vec<String> = Vec::new();
    for item in items {
        output.push(
            item.as_str()
                .filter(|text| !text.is_empty())
                .ok_or_else(|| {
                    RuntimeError::invalid(format!("{key} must contain non-empty strings"))
                })?
                .into(),
        );
    }
    if unique && output.iter().collect::<BTreeSet<_>>().len() != output.len() {
        return Err(RuntimeError::invalid(format!(
            "{key} must contain unique values"
        )));
    }
    Ok(output)
}

fn strict_keys(value: &Value, required: &[&str], optional: &[&str]) -> Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| RuntimeError::invalid("Record must be an object"))?;
    if required.iter().any(|key| !object.contains_key(*key)) {
        return Err(RuntimeError::invalid("Record is missing required keys"));
    }
    if object
        .keys()
        .any(|key| !required.contains(&key.as_str()) && !optional.contains(&key.as_str()))
    {
        return Err(RuntimeError::invalid("Record has unexpected keys"));
    }
    Ok(())
}

pub fn validate_record(record: &Value, archived: bool) -> Result<()> {
    strict_keys(
        record,
        &[
            "schema_version",
            "id",
            "type",
            "external_id",
            "title",
            "status",
            "revision",
            "owner",
            "authorized_owners",
            "privacy",
            "authority",
            "confidence",
            "created_at",
            "updated_at",
            "transitions",
            "source_fingerprints",
            "conflicts",
            "supersedes",
            "superseded_by",
            "goal",
            "progress",
            "next_steps",
            "files",
            "sources",
        ],
        &["phase", "auto_checkpoint"],
    )?;
    if record["schema_version"].as_u64() != Some(1) {
        return Err(RuntimeError::invalid("Unsupported schema_version"));
    }
    if !is_uuid4(record["id"].as_str().unwrap_or("")) {
        return Err(RuntimeError::invalid("Record id must be UUIDv4"));
    }
    let kind = string(record, "type")?;
    if !RECORD_TYPES.contains(&kind) {
        return Err(RuntimeError::invalid("Invalid record type"));
    }
    for key in [
        "external_id",
        "title",
        "owner",
        "goal",
        "status",
        "privacy",
        "authority",
    ] {
        string(record, key)?;
    }
    if !record["progress"].is_string() {
        return Err(RuntimeError::invalid("Progress must be text"));
    }
    for key in ["created_at", "updated_at"] {
        if !valid_datetime(string(record, key)?) {
            return Err(RuntimeError::invalid(format!("Invalid {key} timestamp")));
        }
    }
    for key in [
        "authorized_owners",
        "conflicts",
        "supersedes",
        "next_steps",
        "files",
        "sources",
    ] {
        string_list(record, key, true)?;
    }
    if !string_list(record, "authorized_owners", true)?
        .iter()
        .any(|owner| owner == record["owner"].as_str().unwrap_or(""))
    {
        return Err(RuntimeError::invalid(
            "Owner must be included in authorized_owners",
        ));
    }
    if !PRIVACY.contains(&string(record, "privacy")?)
        || !AUTHORITIES.contains(&string(record, "authority")?)
    {
        return Err(RuntimeError::invalid("Invalid privacy or authority"));
    }
    if !record["confidence"]
        .as_f64()
        .is_some_and(|number| number.is_finite() && (0.0..=1.0).contains(&number))
    {
        return Err(RuntimeError::invalid("Confidence must be between 0 and 1"));
    }
    if !record["revision"].as_u64().is_some_and(|number| number > 0) {
        return Err(RuntimeError::invalid("Revision must be a positive integer"));
    }
    let status = string(record, "status")?;
    if !lifecycle_states(kind).contains(&status) {
        return Err(RuntimeError::invalid("Invalid lifecycle status"));
    }
    if let Some(phase) = record.get("phase")
        && (kind != "task" || !PHASES.contains(&phase.as_str().unwrap_or("")))
    {
        return Err(RuntimeError::invalid(
            "Only tasks may declare a canonical phase",
        ));
    }
    if record.get("auto_checkpoint").is_some() {
        if kind != "task" {
            return Err(RuntimeError::invalid(
                "Only tasks may record auto checkpoints",
            ));
        }
        string(record, "auto_checkpoint")?;
    }
    let transitions = record["transitions"]
        .as_array()
        .filter(|items| !items.is_empty())
        .ok_or_else(|| RuntimeError::invalid("Transitions must be non-empty"))?;
    let mut previous = None;
    for (index, transition) in transitions.iter().enumerate() {
        strict_keys(transition, &["from", "to", "at", "actor", "reason"], &[])?;
        let to = string(transition, "to")?;
        let from = match &transition["from"] {
            Value::Null => None,
            Value::String(text) => Some(text.as_str()),
            _ => return Err(RuntimeError::invalid("Invalid transition source")),
        };
        string(transition, "actor")?;
        string(transition, "reason")?;
        if !valid_datetime(string(transition, "at")?) {
            return Err(RuntimeError::invalid("Invalid transition timestamp"));
        }
        if index == 0 {
            if from.is_some() || to != initial_state(kind)? {
                return Err(RuntimeError::invalid(
                    "First transition must create the initial state",
                ));
            }
        } else if from.is_some_and(|from| AUTHORITIES.contains(&from)) && AUTHORITIES.contains(&to)
        {
            if from != Some("observed") || to != "verified" {
                return Err(RuntimeError::invalid("Illegal authority transition"));
            }
            continue;
        } else if from != previous || !from.is_some_and(|from| transition_allowed(kind, from, to)) {
            return Err(RuntimeError::invalid("Illegal lifecycle transition"));
        }
        previous = Some(to);
    }
    if previous != Some(status) {
        return Err(RuntimeError::invalid(
            "Transition history does not match status",
        ));
    }
    let fingerprints = record["source_fingerprints"]
        .as_array()
        .ok_or_else(|| RuntimeError::invalid("source_fingerprints must be a list"))?;
    for fingerprint in fingerprints {
        strict_keys(fingerprint, &["path", "sha256"], &[])?;
        if !fingerprint["path"].is_string()
            || !valid_digest(fingerprint["sha256"].as_str().unwrap_or(""))
        {
            return Err(RuntimeError::invalid("Invalid source fingerprint"));
        }
    }
    for key in ["conflicts", "supersedes"] {
        if string_list(record, key, true)?
            .iter()
            .any(|id| !is_uuid4(id))
        {
            return Err(RuntimeError::invalid("Relationships must use UUIDv4 IDs"));
        }
    }
    if !record["superseded_by"].is_null() && !record["superseded_by"].as_str().is_some_and(is_uuid4)
    {
        return Err(RuntimeError::invalid(
            "superseded_by must be null or UUIDv4",
        ));
    }
    if archived && !is_terminal(kind, status) {
        return Err(RuntimeError::invalid(
            "Only terminal records may be archived",
        ));
    }
    Ok(())
}

pub(crate) fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn validate_handoff(record: &Value, task: Option<&Value>) -> Result<()> {
    strict_keys(
        record,
        &[
            "schema_version",
            "id",
            "type",
            "task_id",
            "task_revision",
            "revision",
            "owner",
            "privacy",
            "created_at",
            "updated_at",
            "objective",
            "current_state",
            "next_actions",
            "files",
            "sources",
            "status",
        ],
        &["phase"],
    )?;
    if record["schema_version"].as_u64() != Some(1)
        || record["type"] != "handoff"
        || !["active", "closed"].contains(&string(record, "status")?)
    {
        return Err(RuntimeError::invalid("Invalid handoff schema/type/status"));
    }
    for key in ["id", "task_id"] {
        if !is_uuid4(string(record, key)?) {
            return Err(RuntimeError::invalid("Handoff IDs must be UUIDv4"));
        }
    }
    for key in ["task_revision", "revision"] {
        if !record[key].as_u64().is_some_and(|n| n > 0) {
            return Err(RuntimeError::invalid(
                "Handoff revisions must be positive integers",
            ));
        }
    }
    for key in ["created_at", "updated_at"] {
        if !valid_datetime(string(record, key)?) {
            return Err(RuntimeError::invalid("Invalid handoff timestamp"));
        }
    }
    for key in ["owner", "objective"] {
        string(record, key)?;
    }
    for key in ["next_actions", "files", "sources"] {
        string_list(record, key, false)?;
    }
    if !record["current_state"].is_string() || !PRIVACY.contains(&string(record, "privacy")?) {
        return Err(RuntimeError::invalid("Invalid handoff state/privacy"));
    }
    if let Some(phase) = record.get("phase")
        && !PHASES.contains(&phase.as_str().unwrap_or(""))
    {
        return Err(RuntimeError::invalid("Invalid handoff phase"));
    }
    if let Some(task) = task
        && (record["task_id"] != task["id"] || record["task_revision"] != task["revision"])
    {
        return Err(RuntimeError::conflict("Handoff task binding is stale"));
    }
    Ok(())
}

pub fn parse_markdown_record(text: &str) -> Result<(Value, String)> {
    let split = memory_kernel_probe::split_frontmatter(text)
        .map_err(|_| RuntimeError::invalid("Missing or invalid JSON frontmatter delimiters"))?;
    let mut metadata: Value = super::json::parse(split.metadata_text)?;
    if !metadata.is_object() {
        return Err(RuntimeError::invalid("Frontmatter must be an object"));
    }
    if metadata["phase"] == "execution" {
        metadata["phase"] = Value::String("implementation".into());
    }
    Ok((metadata, split.body.trim_start_matches('\n').into()))
}

/// Python-compatible trailing whitespace removal, including ASCII separators.
pub fn rstrip(text: &str) -> &str {
    text.char_indices()
        .rev()
        .find(|(_, character)| {
            !character.is_whitespace() && !matches!(character, '\u{1c}'..='\u{1f}')
        })
        .map(|(index, character)| &text[..index + character.len_utf8()])
        .unwrap_or("")
}

pub fn render_markdown_record(metadata: &Value, body: &str) -> Result<String> {
    Ok(format!(
        "---\n{}\n---\n\n{}\n",
        super::json::sorted_pretty(metadata)?,
        rstrip(body)
    ))
}

pub fn render_current_state(progress: &str, checkpoint: Option<&str>) -> String {
    match (progress.is_empty(), checkpoint.filter(|s| !s.is_empty())) {
        (false, Some(checkpoint)) => format!("{progress}\nSince checkpoint: {checkpoint}"),
        (true, Some(checkpoint)) => checkpoint.into(),
        _ => progress.into(),
    }
}

pub fn record_attestation(record: &Value) -> &str {
    let Some(transitions) = record["transitions"].as_array() else {
        return "";
    };
    for transition in transitions.iter().rev() {
        if transition["from"] != "observed" || transition["to"] != "verified" {
            continue;
        }
        let reason = transition["reason"].as_str().unwrap_or("");
        let agent = reason.rfind("[attestation:agent]");
        let person = reason.rfind("[attestation:person]");
        return match (agent, person) {
            (Some(a), Some(p)) => {
                if a > p {
                    "agent"
                } else {
                    "person"
                }
            }
            (Some(_), None) => "agent",
            (None, Some(_)) => "person",
            _ => {
                if reason.contains("agent-attested") {
                    "agent"
                } else {
                    ""
                }
            }
        };
    }
    ""
}
