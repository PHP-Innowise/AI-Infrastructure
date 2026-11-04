//! Insert-only, generation-owned safe-wire bindings. No automatic eviction.
use crate::{HarnessError, Result, memory_draft, store::SessionStore};
use infra_core::{self as core, domain::json as wire};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
pub const PROFILE: &str = "harness-memory-request-v1";
pub const BYTE_LIMIT: usize = 128 * 1024;
pub const CONFLICT: &str = "This memory attempt changed. Its original safe request was retained; no new writes were started.";
pub const INCOMPLETE: &str =
    "Memory replay is incomplete: the original attempt cannot be safely resumed.";
pub fn encoded(value: &Value) -> Result<String> {
    let text = wire::python_default(value, true, false, false)?;
    if text.len() > BYTE_LIMIT {
        return Err(HarnessError::invalid(
            "The private memory attempt exceeds its safe JSON limits.",
        ));
    }
    Ok(text)
}
pub fn fingerprint(data: &Value) -> Result<String> {
    let mut data = data.clone();
    data.as_object_mut()
        .ok_or_else(|| HarnessError::invalid("Invalid memory draft."))?
        .remove("used_memory");
    Ok(core::query::sha256(encoded(&data)?.as_bytes()))
}
fn fields(value: &Value, expected: &[&str]) -> bool {
    value.as_object().is_some_and(|object| {
        object.len() == expected.len() && expected.iter().all(|key| object.contains_key(*key))
    })
}
fn incomplete() -> HarnessError {
    HarnessError::conflict(INCOMPLETE)
}
pub fn lookup_conn(db: &Connection, sid: &str, generation: &str) -> Result<Option<Value>> {
    let raw = db
        .query_row(
            "SELECT binding FROM memory_result_attempts WHERE session_id=?1 AND generation=?2",
            params![sid, generation],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let operation = (|| -> Result<Value> {
        if raw.chars().count() > BYTE_LIMIT || raw.len() > BYTE_LIMIT {
            return Err(incomplete());
        }
        let value = wire::parse(&raw)?;
        encoded(&value)?;
        if !fields(
            &value,
            &["profile", "identity", "prepared", "skipped_count"],
        ) || value["profile"] != PROFILE
            || !value["identity"].is_object()
            || !value["prepared"].is_object()
            || !value["skipped_count"]
                .as_u64()
                .is_some_and(|count| count <= 3)
        {
            return Err(incomplete());
        }
        Ok(value)
    })();
    operation.map(Some).map_err(|_| incomplete())
}
pub fn lookup(store: &SessionStore, sid: &str, generation: &str) -> Result<Option<Value>> {
    let db = store.connection()?;
    lookup_conn(&db, sid, generation)
}
pub fn guard_conn(
    db: &Connection,
    sid: &str,
    generation: &str,
    cancel: &AtomicBool,
    finalized: bool,
) -> Result<()> {
    if cancel.load(Ordering::Acquire) || finalized {
        return Err(incomplete());
    }
    let current = db
        .query_row(
            "SELECT generation FROM harness_generations WHERE session_id=?1",
            [sid],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let status = db
        .query_row("SELECT status FROM sessions WHERE id=?1", [sid], |row| {
            row.get::<_, String>(0)
        })
        .optional()?;
    if current.as_deref() != Some(generation) || status.as_deref() == Some("cancelled") {
        return Err(incomplete());
    }
    let kind = db
        .query_row(
            "SELECT kind FROM launches WHERE id=?1 AND session_id=?2",
            params![generation, sid],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if kind.as_deref() != Some("native") {
        return Err(incomplete());
    }
    Ok(())
}
pub fn identity_conn(
    db: &Connection,
    sid: &str,
    generation: &str,
    event_id: i64,
    data: &Value,
    scope: &Value,
) -> Result<Value> {
    let digest = fingerprint(data)?;
    if event_id < 1 {
        return Err(incomplete());
    }
    let raw = db
        .query_row(
            "SELECT data FROM events WHERE id=?1 AND session_id=?2",
            params![event_id, sid],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let checked = (|| -> Result<()> {
        let event = raw
            .as_deref()
            .map(wire::parse)
            .transpose()?
            .ok_or_else(|| HarnessError::conflict(CONFLICT))?;
        if event["launch_id"] != generation
            || !matches!(event["kind"].as_str(), Some("text" | "result"))
        {
            return Err(HarnessError::conflict(CONFLICT));
        }
        let draft = event["text"]
            .as_str()
            .and_then(memory_draft::parse)
            .ok_or_else(|| HarnessError::conflict(CONFLICT))?;
        if fingerprint(&draft)? != digest {
            return Err(HarnessError::conflict(CONFLICT));
        }
        Ok(())
    })();
    checked.map_err(|_| HarnessError::conflict(CONFLICT))?;
    Ok(
        json!({"session":sid,"generation":generation,"event":event_id,"input_digest":digest,"scope":scope}),
    )
}
pub fn prepared(
    value: &Value,
    task_id: &str,
    result_id: &str,
    revision: u64,
    owner: &str,
    reason: &str,
) -> Result<Value> {
    let operation = (|| -> Result<Value> {
        if !fields(
            value,
            &[
                "profile",
                "task_id",
                "result_id",
                "revision",
                "owner",
                "reason",
                "attestation",
                "scope",
                "digest",
                "request",
            ],
        ) || value["profile"] != PROFILE
            || value["task_id"] != task_id
            || value["result_id"] != result_id
            || value["revision"].as_u64() != Some(revision)
            || value["owner"] != owner
            || value["reason"] != reason
            || value["attestation"] != "agent"
            || !core::domain::is_uuid4(task_id)
            || ["scope", "digest"].into_iter().any(|field| {
                !value[field].as_str().is_some_and(|value| {
                    value.len() == 64
                        && value
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                })
            })
        {
            return Err(incomplete());
        }
        let request = &value["request"];
        if request.as_object().is_none_or(|object| {
            object.keys().any(|key| {
                ![
                    "progress",
                    "next_steps",
                    "learnings",
                    "verified",
                    "capture_decision",
                ]
                .contains(&key.as_str())
            })
        }) || request["verified"] != true
            || request.get("learnings").is_none()
        {
            return Err(incomplete());
        }
        let draft = memory_draft::submission(request)?;
        let progress = draft["progress"]
            .as_str()
            .filter(|value| !value.is_empty())
            .map(String::from);
        let next = if request.get("next_steps").is_some() {
            Some(draft["next_steps"].clone())
        } else {
            None
        };
        let mut normalized =
            json!({"progress":progress,"next_steps":next,"learnings":draft["learnings"]});
        let mut safe = json!({"learnings":draft["learnings"],"verified":true});
        if let Some(decision) = draft.get("capture_decision") {
            normalized["capture_decision"] = decision.clone();
            safe["capture_decision"] = decision.clone();
        }
        if let Some(progress) = progress {
            safe["progress"] = json!(progress);
        }
        if let Some(next) = next {
            safe["next_steps"] = next;
        }
        let mut content = normalized;
        content["task"] = json!(task_id);
        let digest =
            core::query::sha256(wire::python_default(&content, true, false, false)?.as_bytes());
        if safe != *request || value["digest"] != digest {
            return Err(incomplete());
        }
        encoded(value)?;
        Ok(value.clone())
    })();
    operation.map_err(|_| incomplete())
}
pub fn insert_conn(
    db: &Connection,
    identity: &Value,
    value: &Value,
    skipped_count: usize,
) -> Result<(Value, bool)> {
    if skipped_count > 3 {
        return Err(incomplete());
    }
    let sid = identity["session"].as_str().ok_or_else(incomplete)?;
    let generation = identity["generation"].as_str().ok_or_else(incomplete)?;
    let binding = json!({"profile":PROFILE,"identity":identity,"prepared":value,"skipped_count":skipped_count});
    let text = encoded(&binding)?;
    if let Some(previous) = lookup_conn(db, sid, generation)? {
        if previous["identity"] != *identity {
            return Err(HarnessError::conflict(CONFLICT));
        }
        return Ok((previous, false));
    }
    db.execute(
        "INSERT INTO memory_result_attempts(session_id,generation,binding) VALUES(?1,?2,?3)",
        params![sid, generation, text],
    )?;
    Ok((binding, true))
}
#[derive(Clone, Debug)]
pub struct LatestDraft {
    pub state: String,
    pub draft: Option<Value>,
    pub event_id: Option<i64>,
}
pub fn latest(store: &SessionStore, sid: &str) -> Result<LatestDraft> {
    let db = store.connection()?;
    let mut statement =
        db.prepare("SELECT id,data FROM events WHERE session_id=?1 ORDER BY id DESC LIMIT 6000")?;
    let mut events = Vec::new();
    for row in statement.query_map([sid], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })? {
        let (id, raw) = row?;
        let event = wire::parse(&raw)?;
        if matches!(event["kind"].as_str(), Some("user" | "text" | "result")) {
            events.push((id, event));
            if events.len() >= 200 {
                break;
            }
        }
    }
    events.reverse();
    let mut run = Vec::new();
    for (id, event) in events {
        if event["kind"] == "user" {
            run.clear();
        } else if event["text"].as_str().is_some_and(|text| !text.is_empty()) {
            run.push((id, event));
        }
    }
    for (id, event) in run.iter().rev() {
        if let Some(draft) = event["text"].as_str().and_then(memory_draft::parse) {
            return Ok(LatestDraft {
                state: "drafted".into(),
                draft: Some(draft),
                event_id: Some(*id),
            });
        }
    }
    let grave = char::from(96);
    let marker = format!("{grave}{grave}{grave}memory-draft");
    Ok(LatestDraft {
        state: if run.iter().any(|(_, event)| {
            event["text"]
                .as_str()
                .is_some_and(|text| text.contains(&marker))
        }) {
            "unreadable"
        } else if run.is_empty() {
            "none"
        } else {
            "missing"
        }
        .into(),
        draft: None,
        event_id: None,
    })
}
