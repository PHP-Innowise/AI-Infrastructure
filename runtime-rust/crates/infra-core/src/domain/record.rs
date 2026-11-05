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
    let items = value.get(key).and_then(Value::as_array).ok_or_else(|| {
        RuntimeError::invalid(format!("{key} must be a list of non-empty strings"))
    })?;
    let mut output: Vec<String> = Vec::new();
    for item in items {
        output.push(
            item.as_str()
                .filter(|text| !text.is_empty())
                .ok_or_else(|| {
                    RuntimeError::invalid(format!("{key} must be a list of non-empty strings"))
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

fn invalid(message: impl Into<String>) -> RuntimeError {
    RuntimeError::invalid(message)
}

/// Missing, then unexpected keys, each list named in sorted order as the
/// retired validator named it: "record missing keys: goal".
fn keys(value: &Value, what: &str, required: &[&str], optional: &[&str]) -> Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| RuntimeError::invalid(format!("{what} must be an object")))?;
    let joined = |keys: BTreeSet<&str>| keys.into_iter().collect::<Vec<_>>().join(", ");
    let missing = required
        .iter()
        .copied()
        .filter(|key| !object.contains_key(*key))
        .collect::<BTreeSet<_>>();
    if !missing.is_empty() {
        return Err(RuntimeError::invalid(format!(
            "{what} missing keys: {}",
            joined(missing)
        )));
    }
    let extra = object
        .keys()
        .map(String::as_str)
        .filter(|key| !required.contains(key) && !optional.contains(key))
        .collect::<BTreeSet<_>>();
    if !extra.is_empty() {
        return Err(RuntimeError::invalid(format!(
            "{what} has unexpected keys: {}",
            joined(extra)
        )));
    }
    Ok(())
}

/// A non-empty RFC 3339 timestamp.
fn timestamp(value: &Value, key: &str) -> Result<()> {
    if !valid_datetime(string(value, key)?) {
        return Err(RuntimeError::invalid(format!(
            "{key} must be an RFC3339 timestamp"
        )));
    }
    Ok(())
}

/// A positive integer, as Python's `type(value) is int` saw one.
fn positive(value: &Value, key: &str) -> bool {
    value[key].as_u64().is_some_and(|number| number > 0)
}

/// The retired `validate_record`, check by check and in its order: its
/// sentence is all a person repairing a hand-edited or half-merged record is
/// told, and the schema names a field first wherever it can.
pub fn validate_record(record: &Value, archived: bool) -> Result<()> {
    keys(
        record,
        "record",
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
    if let Some(phase) = record.get("phase") {
        if record["type"] != "task" {
            return Err(invalid("only a task may declare a phase"));
        }
        if !phase.as_str().is_some_and(|phase| PHASES.contains(&phase)) {
            return Err(invalid(format!(
                "phase must be one of: {}",
                PHASES.join(", ")
            )));
        }
    }
    if record.get("auto_checkpoint").is_some() {
        if record["type"] != "task" {
            return Err(invalid("only a task may record an auto checkpoint"));
        }
        string(record, "auto_checkpoint")?;
    }
    if record["schema_version"].as_u64() != Some(1) {
        return Err(invalid("unsupported schema_version"));
    }
    if !record["id"].as_str().is_some_and(is_uuid4) {
        return Err(invalid("record id must be UUIDv4"));
    }
    let kind = record["type"]
        .as_str()
        .filter(|kind| RECORD_TYPES.contains(kind))
        .ok_or_else(|| invalid(format!("type must be one of: {}", RECORD_TYPES.join(", "))))?;
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
        return Err(invalid("progress must be text"));
    }
    for key in ["created_at", "updated_at"] {
        timestamp(record, key)?;
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
        return Err(invalid("owner must be included in authorized_owners"));
    }
    if !PRIVACY.contains(&string(record, "privacy")?) {
        return Err(invalid("invalid privacy"));
    }
    if !AUTHORITIES.contains(&string(record, "authority")?) {
        return Err(invalid("invalid authority"));
    }
    if !record["confidence"]
        .as_f64()
        .is_some_and(|number| number.is_finite() && (0.0..=1.0).contains(&number))
    {
        return Err(invalid("confidence must be between 0 and 1"));
    }
    if !positive(record, "revision") {
        return Err(invalid("revision must be a positive integer"));
    }
    let status = string(record, "status")?;
    if !lifecycle_states(kind).contains(&status) {
        return Err(invalid(format!("invalid {kind} status")));
    }
    let transitions = record["transitions"]
        .as_array()
        .filter(|items| !items.is_empty())
        .ok_or_else(|| invalid("transitions must be a non-empty append-only list"))?;
    let mut previous = None;
    for (index, transition) in transitions.iter().enumerate() {
        let entry = transition
            .as_object()
            .ok_or_else(|| invalid("transition must be an object"))?;
        if entry.len() != 5
            || ["from", "to", "at", "actor", "reason"]
                .iter()
                .any(|key| !entry.contains_key(*key))
        {
            return Err(invalid("transition does not match strict schema"));
        }
        timestamp(transition, "at")?;
        string(transition, "actor")?;
        string(transition, "reason")?;
        let (from, to) = match (&transition["from"], &transition["to"]) {
            (Value::Null, Value::String(to)) => (None, to.as_str()),
            (Value::String(from), Value::String(to)) => (Some(from.as_str()), to.as_str()),
            _ => {
                return Err(invalid(
                    "transition endpoints must be text, with a null initial source",
                ));
            }
        };
        if index == 0 {
            let initial = initial_state(kind)?;
            if from.is_some() || to != initial {
                return Err(invalid(format!(
                    "first {kind} transition must create {initial} state"
                )));
            }
        } else if from.is_some_and(|from| AUTHORITIES.contains(&from)) && AUTHORITIES.contains(&to)
        {
            if from != Some("observed") || to != "verified" {
                return Err(invalid(format!(
                    "illegal authority transition: {} -> {to}",
                    from.unwrap_or("None")
                )));
            }
            continue;
        } else if from != previous || !from.is_some_and(|from| transition_allowed(kind, from, to)) {
            return Err(invalid(format!(
                "illegal lifecycle transition: {} -> {to}",
                from.unwrap_or("None")
            )));
        }
        previous = Some(to);
    }
    if previous != Some(status) {
        return Err(invalid("transition history does not match status"));
    }
    let fingerprints = record["source_fingerprints"]
        .as_array()
        .ok_or_else(|| invalid("source_fingerprints must be a list"))?;
    for fingerprint in fingerprints {
        if !fingerprint.as_object().is_some_and(|entry| {
            entry.len() == 2
                && entry["path"].is_string()
                && entry
                    .get("sha256")
                    .and_then(Value::as_str)
                    .is_some_and(valid_digest)
        }) {
            return Err(invalid("source_fingerprints does not match strict schema"));
        }
    }
    for key in ["conflicts", "supersedes"] {
        if string_list(record, key, true)?
            .iter()
            .any(|id| !is_uuid4(id))
        {
            return Err(invalid("record relationships must use UUIDv4 IDs"));
        }
    }
    if !record["superseded_by"].is_null() && !record["superseded_by"].as_str().is_some_and(is_uuid4)
    {
        return Err(invalid("superseded_by must be null or UUIDv4"));
    }
    if archived && !is_terminal(kind, status) {
        return Err(invalid("only terminal records may be archived"));
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
    keys(
        record,
        "handoff",
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
    if record["schema_version"].as_u64() != Some(1) {
        return Err(invalid("unsupported handoff schema_version"));
    }
    if let Some(phase) = record.get("phase")
        && !phase.as_str().is_some_and(|phase| PHASES.contains(&phase))
    {
        return Err(invalid(format!(
            "phase must be one of: {}",
            PHASES.join(", ")
        )));
    }
    if !record["id"].as_str().is_some_and(is_uuid4)
        || !record["task_id"].as_str().is_some_and(is_uuid4)
    {
        return Err(invalid("handoff and task IDs must be UUIDv4"));
    }
    let status = string(record, "status")?;
    if record["type"] != "handoff" || !["active", "closed"].contains(&status) {
        return Err(invalid("invalid handoff type or status"));
    }
    if !record["privacy"]
        .as_str()
        .is_some_and(|privacy| PRIVACY.contains(&privacy))
    {
        return Err(invalid("invalid handoff privacy"));
    }
    for key in ["owner", "objective"] {
        string(record, key)?;
    }
    if !record["current_state"].is_string() {
        return Err(invalid("current_state must be text"));
    }
    for key in ["created_at", "updated_at"] {
        timestamp(record, key)?;
    }
    for key in ["task_revision", "revision"] {
        if !positive(record, key) {
            return Err(invalid(format!("{key} must be a positive integer")));
        }
    }
    for key in ["next_actions", "files", "sources"] {
        string_list(record, key, false)?;
    }
    if let Some(task) = task
        && (record["task_id"] != task["id"] || record["task_revision"] != task["revision"])
    {
        return Err(RuntimeError::conflict("handoff task binding is stale"));
    }
    Ok(())
}

/// A record's frontmatter and body, refused in the retired parser's words.
pub fn parse_markdown_record(text: &str) -> Result<(Value, String)> {
    let split = memory_kernel_probe::split_frontmatter(text).map_err(|refusal| {
        invalid(match refusal {
            memory_kernel_probe::Refusal::MissingOpening => "missing JSON frontmatter",
            _ => "invalid JSON frontmatter",
        })
    })?;
    let mut metadata: Value = super::json::parse(split.metadata_text).map_err(|_| {
        // Python read a lone surrogate escape and refused the text it held.
        if split.metadata_text.contains("\\u")
            && crate::python_json::first_problem(split.metadata_text).is_none()
        {
            invalid("invalid Unicode metadata")
        } else {
            invalid("invalid JSON frontmatter")
        }
    })?;
    if !metadata.is_object() {
        return Err(invalid("frontmatter must be an object"));
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
