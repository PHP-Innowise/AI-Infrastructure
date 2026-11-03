//! Disposable retrieval/session bookkeeping committed after publication.
use crate::{
    Result, RuntimeError,
    domain::uuid4,
    query::{Candidate, folded, sha256},
    storage::Store,
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::{Duration, Instant},
};

pub const SESSION_NOVELTY_TURNS: u64 = 4;
pub const SESSION_DELIVERIES_RETENTION: usize = 50;
pub const LAST_RETRIEVAL_RETENTION: usize = 200;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Signature {
    pub query: String,
    pub paths: String,
    pub task_revision: String,
}
pub fn signature(query: &str, identities: &[(String, String)], revision: u64) -> Signature {
    let ordered = identities.iter().cloned().collect::<BTreeSet<_>>();
    let paths = ordered
        .into_iter()
        .map(|(key, revision)| format!("{key}\0{revision}"))
        .collect::<Vec<_>>()
        .join("\n");
    Signature {
        query: sha256(query.as_bytes()),
        paths: sha256(paths.as_bytes()),
        task_revision: revision.to_string(),
    }
}
pub fn baseline_key(task: &str, host: &str, entry: &str, persistent: bool) -> String {
    format!(
        "{task}:{host}:{entry}{}",
        if persistent { ":cursor-rule" } else { "" }
    )
}
#[derive(Clone, Debug)]
pub struct BaselinePlan {
    pub key: String,
    pub signature: Signature,
    pub expected: Option<Value>,
}
#[derive(Clone, Debug)]
pub struct SessionPlan {
    pub id: String,
    pub turn: u64,
    pub expected: Option<Value>,
    pub delivered: BTreeMap<String, (String, u64)>,
    pub transcript: Option<Value>,
    pub reset: bool,
    pub exact_next_turn: bool,
    pub window_receipts: Option<Value>,
}
#[derive(Clone, Debug)]
pub struct ReceiptSnapshot {
    pub baselines: serde_json::Map<String, Value>,
    pub sessions: serde_json::Map<String, Value>,
    pub baseline_order: Vec<String>,
    pub session_order: Vec<String>,
}
pub fn load(conn: &Connection) -> Result<ReceiptSnapshot> {
    let mut values = BTreeMap::<String, String>::new();
    let mut statement = conn.prepare(
        "SELECT key,value FROM index_state WHERE key IN ('last-retrieval','session-deliveries')",
    )?;
    for row in statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })? {
        let (key, value) = row?;
        values.insert(key, value);
    }
    Ok(ReceiptSnapshot {
        baselines: decode_object(values.get("last-retrieval")),
        sessions: normalize_sessions(decode_object(values.get("session-deliveries"))),
        baseline_order: object_order(values.get("last-retrieval")),
        session_order: object_order(values.get("session-deliveries")),
    })
}
fn decode_object(raw: Option<&String>) -> serde_json::Map<String, Value> {
    raw.and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default()
}
fn normalize_sessions(sessions: serde_json::Map<String, Value>) -> serde_json::Map<String, Value> {
    sessions
        .into_iter()
        .filter_map(|(id, entry)| {
            if !entry.is_object() {
                return None;
            }
            let turn = entry["turn"].as_u64().unwrap_or(0);
            let items = entry["items"]
                .as_object()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|(key, seen)| {
                            let seen = seen.as_array()?;
                            if seen.len() != 2 || !seen[0].is_string() || seen[1].as_u64().is_none()
                            {
                                return None;
                            }
                            Some((key.clone(), json!(seen)))
                        })
                        .collect::<serde_json::Map<_, _>>()
                })
                .unwrap_or_default();
            let mut clean = json!({"turn":turn,"items":items});
            if entry["transcript"].is_object() {
                clean["transcript"] = entry["transcript"].clone();
            }
            let refs = window_references(&entry, &clean["items"]);
            if !refs.is_empty() {
                clean["window_observation"] = json!({"version":1,"receipts":refs});
            }
            Some((id, clean))
        })
        .collect()
}
pub fn valid_session(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
}
pub fn identity(item: &Candidate) -> (String, String) {
    if let Some(identity) = &item.delivery_identity {
        return identity.clone();
    }
    let mut key = item.document.path.clone();
    let mut revision = item.document.source_hash.clone();
    if item.document.layer != "procedural"
        && item.match_strength.as_deref() != Some("distinctive")
        && !(matches!(
            item.selection.as_deref(),
            Some("prompt-link" | "touch-link")
        ) && item.document.kind == "brain-task")
    {
        let body = item
            .document
            .content
            .strip_prefix("---\n")
            .and_then(|tail| tail.split_once("\n---\n").map(|(_, body)| body))
            .unwrap_or(&item.document.content);
        let words = crate::query::query_tokens(&item.snippet).unwrap_or_default();
        let mut sections = Vec::<(String, String)>::new();
        let mut heading = String::new();
        let mut content = String::new();
        for line in body.lines() {
            if line.starts_with('#') {
                if !content.is_empty() {
                    sections.push((heading.clone(), std::mem::take(&mut content)));
                }
                heading = line.trim_start_matches('#').trim().into();
            } else {
                content.push_str(line);
                content.push('\n');
            }
        }
        if !content.is_empty() {
            sections.push((heading, content));
        }
        let selected = if item.document.kind == "changelog" {
            sections.first()
        } else {
            sections.iter().max_by_key(|(_, text)| {
                words
                    .iter()
                    .filter(|word| folded(text).contains(&folded(word)))
                    .count()
            })
        };
        if let Some((heading, content)) = selected {
            let slug = folded(heading)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join("-");
            key = format!("{key}#{slug}");
            revision = sha256(content.as_bytes());
        }
    }
    if !item.document.conflicts.is_empty() || item.match_strength.as_deref() == Some("conflict") {
        revision = sha256(format!("recorded-conflict\0{revision}").as_bytes());
    }
    (key, revision)
}
pub fn was_handed(entry: Option<&Value>, key: &str, revision: &str, turn: u64) -> bool {
    entry
        .and_then(|entry| entry["items"].get(key))
        .and_then(Value::as_array)
        .is_some_and(|seen| {
            seen.len() == 2
                && seen[0].as_str() == Some(revision)
                && seen[1]
                    .as_u64()
                    .is_some_and(|previous| turn.saturating_sub(previous) < SESSION_NOVELTY_TURNS)
        })
}
pub fn transcript_compacted(
    entry: Option<&Value>,
    transcript: Option<&Path>,
) -> (bool, Option<Value>) {
    use std::io::{Read, Seek, SeekFrom};
    let Some(path) = transcript else {
        return (false, None);
    };
    if !path.is_absolute() {
        return (false, None);
    }
    let size = match std::fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.is_file() && !crate::fs::linked_at(path, &metadata).unwrap_or(true) =>
        {
            metadata.len()
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        _ => return (false, None),
    };
    #[cfg(unix)]
    let bytes = {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    };
    #[cfg(not(unix))]
    let bytes = path.to_string_lossy().as_bytes().to_vec();
    let file_key = sha256(&bytes)[..16].to_string();
    let state = json!({"file":file_key,"offset":size});
    let previous = entry.map(|entry| &entry["transcript"]);
    let Some(start) = previous
        .filter(|previous| previous["file"] == file_key)
        .and_then(|previous| previous["offset"].as_u64())
    else {
        return (false, Some(state));
    };
    if size < start || size - start > 8 * 1024 * 1024 {
        return (true, Some(state));
    }
    if size == start {
        return (false, Some(state));
    }
    let capture = (|| -> std::io::Result<Vec<u8>> {
        let mut file = std::fs::File::open(path)?;
        let mut begin = start;
        if start > 0 {
            let back = start.min(65536);
            file.seek(SeekFrom::Start(start - back))?;
            let mut before = vec![0; back as usize];
            file.read_exact(&mut before)?;
            if before.last() != Some(&b'\n') {
                begin = before
                    .iter()
                    .rposition(|byte| *byte == b'\n')
                    .map(|position| start - back + position as u64 + 1)
                    .unwrap_or(start - back);
            }
        }
        file.seek(SeekFrom::Start(begin))?;
        let mut bytes = Vec::new();
        file.take(size - begin).read_to_end(&mut bytes)?;
        Ok(bytes)
    })();
    let compacted = capture
        .map(|bytes| {
            [
                b"\"subtype\":\"compact_boundary\"".as_slice(),
                b"\"type\":\"compacted\"".as_slice(),
            ]
            .iter()
            .any(|marker| bytes.windows(marker.len()).any(|window| window == *marker))
        })
        .unwrap_or(false);
    (compacted, Some(state))
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReceiptStatus {
    Skipped,
    Committed,
    Deferred,
    Unknown,
}
pub fn commit(
    store: &mut Store,
    baseline: Option<&BaselinePlan>,
    session: Option<&SessionPlan>,
) -> ReceiptStatus {
    if baseline.is_none() && session.is_none() {
        return ReceiptStatus::Skipped;
    }
    if store.read_only {
        return ReceiptStatus::Unknown;
    }
    let conn = &store.conn;
    let old_timeout = conn
        .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, u64>(0))
        .unwrap_or(5000);
    let deadline = Instant::now() + Duration::from_millis(150);
    let ambient = !conn.is_autocommit();
    let name = format!("ordinary_receipts_{}", uuid4().replace('-', ""));
    let mut begun = false;
    let operation = (|| -> Result<()> {
        conn.busy_timeout(Duration::from_millis(old_timeout.min(150)))?;
        conn.execute_batch(&if ambient {
            format!("SAVEPOINT {name}")
        } else {
            "BEGIN IMMEDIATE".into()
        })?;
        begun = true;
        let mut latest = load(conn)?;
        if let Some(plan) = baseline {
            if latest.baselines.get(&plan.key) != plan.expected.as_ref() {
                return Err(RuntimeError::conflict("Retrieval receipt changed"));
            }
            latest
                .baselines
                .insert(plan.key.clone(), serde_json::to_value(&plan.signature)?);
            refresh_order(&mut latest.baseline_order, &plan.key);
            prune(
                &mut latest.baselines,
                &mut latest.baseline_order,
                LAST_RETRIEVAL_RETENTION,
            );
        }
        if let Some(plan) = session {
            if latest.sessions.get(&plan.id) != plan.expected.as_ref() {
                return Err(RuntimeError::conflict("Session receipt changed"));
            }
            let previous = latest.sessions.get(&plan.id);
            let old_turn = previous
                .and_then(|entry| entry["turn"].as_u64())
                .unwrap_or(0);
            if plan.turn <= old_turn || (plan.exact_next_turn && plan.turn != old_turn + 1) {
                return Err(RuntimeError::conflict("Session turn changed"));
            }
            let mut items = if plan.reset {
                serde_json::Map::new()
            } else {
                previous
                    .and_then(|entry| entry["items"].as_object())
                    .cloned()
                    .unwrap_or_default()
            };
            items.retain(|_, seen| {
                seen.as_array()
                    .and_then(|seen| seen.get(1))
                    .and_then(Value::as_u64)
                    .is_some_and(|turn| plan.turn.saturating_sub(turn) < SESSION_NOVELTY_TURNS)
            });
            for (key, (revision, turn)) in &plan.delivered {
                items.insert(key.clone(), json!([revision, turn]));
            }
            let mut entry = json!({"turn":plan.turn,"items":items});
            let mut refs = if plan.reset {
                serde_json::Map::new()
            } else {
                previous
                    .map(|previous| window_references(previous, &entry["items"]))
                    .unwrap_or_default()
            };
            if let Some(proposed) = &plan.window_receipts {
                let offered = json!({"window_observation":{"version":1,"receipts":proposed}});
                refs.extend(window_references(&offered, &entry["items"]));
            }
            if !refs.is_empty() {
                let mut rows = refs.into_iter().collect::<Vec<_>>();
                rows.sort_by_key(|(_, reference)| {
                    std::cmp::Reverse(reference["handed_turn"].as_u64().unwrap_or(0))
                });
                entry["window_observation"] = json!({"version":1,"receipts":rows.into_iter().take(32).collect::<serde_json::Map<String,Value>>()});
            }
            if let Some(transcript) = &plan.transcript {
                entry["transcript"] = transcript.clone();
            }
            latest.sessions.insert(plan.id.clone(), entry);
            refresh_order(&mut latest.session_order, &plan.id);
            prune(
                &mut latest.sessions,
                &mut latest.session_order,
                SESSION_DELIVERIES_RETENTION,
            );
        }
        for (key, value, needed) in [
            (
                "last-retrieval",
                encode_ordered(&latest.baselines, &latest.baseline_order)?,
                baseline.is_some(),
            ),
            (
                "session-deliveries",
                encode_ordered(&latest.sessions, &latest.session_order)?,
                session.is_some(),
            ),
        ] {
            if !needed {
                continue;
            }
            if Instant::now() >= deadline {
                return Err(RuntimeError::unavailable("Receipt budget exhausted"));
            }
            conn.busy_timeout(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(old_timeout)),
            )?;
            conn.execute("INSERT INTO index_state(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value])?;
        }
        conn.execute_batch(&if ambient {
            format!("RELEASE SAVEPOINT {name}")
        } else {
            "COMMIT".into()
        })?;
        begun = false;
        Ok(())
    })();
    if begun {
        let _ = conn.execute_batch(&if ambient {
            format!("ROLLBACK TO {name}; RELEASE {name}")
        } else {
            "ROLLBACK".into()
        });
    }
    let _ = conn.busy_timeout(Duration::from_millis(old_timeout));
    if operation.is_ok() {
        if ambient {
            ReceiptStatus::Deferred
        } else {
            ReceiptStatus::Committed
        }
    } else {
        ReceiptStatus::Unknown
    }
}
fn refresh_order(order: &mut Vec<String>, key: &str) {
    order.retain(|previous| previous != key);
    order.push(key.into());
}
fn prune(values: &mut serde_json::Map<String, Value>, order: &mut Vec<String>, limit: usize) {
    order.retain(|key| values.contains_key(key));
    for key in values.keys() {
        if !order.contains(key) {
            order.push(key.clone());
        }
    }
    while order.len() > limit {
        let key = order.remove(0);
        values.remove(&key);
    }
}
fn encode_ordered(values: &serde_json::Map<String, Value>, order: &[String]) -> Result<String> {
    let mut pairs = Vec::new();
    for key in order {
        if let Some(value) = values.get(key) {
            pairs.push(format!(
                "{}:{}",
                serde_json::to_string(key)?,
                serde_json::to_string(value)?
            ));
        }
    }
    Ok(format!("{{{}}}", pairs.join(",")))
}
fn object_order(raw: Option<&String>) -> Vec<String> {
    use serde::de::{MapAccess, Visitor};
    struct Ordered;
    impl<'de> Visitor<'de> for Ordered {
        type Value = Vec<String>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a JSON object")
        }
        fn visit_map<A: MapAccess<'de>>(
            self,
            mut map: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut keys = Vec::new();
            while let Some((key, _)) = map.next_entry::<String, Value>()? {
                keys.push(key);
            }
            Ok(keys)
        }
    }
    let Some(raw) = raw else {
        return vec![];
    };
    let mut parser = serde_json::Deserializer::from_str(raw);
    serde::de::Deserializer::deserialize_map(&mut parser, Ordered).unwrap_or_default()
}
pub fn window_references(entry: &Value, items: &Value) -> serde_json::Map<String, Value> {
    let mut result = serde_json::Map::new();
    if entry["window_observation"]["version"].as_u64() != Some(1) {
        return result;
    }
    let live = items
        .as_object()
        .map(|items| {
            items
                .iter()
                .map(|(key, seen)| (sha256(key.as_bytes()), seen))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    if let Some(refs) = entry["window_observation"]["receipts"].as_object() {
        for (key, value) in refs.iter().take(32) {
            let revision = value["revision"].as_str().unwrap_or("");
            let digest = |text: &str, length: usize| {
                text.len() == length
                    && text
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            };
            if !digest(key, 64)
                || value.as_object().is_none_or(|object| object.len() != 4)
                || !([16, 64].iter().any(|length| digest(revision, *length)))
                || value["handed_turn"].as_u64().is_none()
                || value["manifest_id"]
                    .as_str()
                    .is_none_or(|id| uuid::Uuid::parse_str(id).is_err())
                || !digest(value["doc_ref"].as_str().unwrap_or(""), 64)
                || live.get(key).copied() != Some(&json!([revision, value["handed_turn"]]))
            {
                continue;
            }
            result.insert(key.clone(), value.clone());
        }
    }
    result
}
