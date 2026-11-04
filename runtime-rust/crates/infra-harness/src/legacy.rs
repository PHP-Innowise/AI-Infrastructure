//! One-way, idempotent import of the fixed Fleet graph's legacy data.
use crate::{
    HarnessError, Result, legacy_msgpack as wire,
    store::{self, SessionStore},
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
type LegacyCheckpointRow = (String, String, Vec<u8>, Option<Vec<u8>>);
const NULL_TASK: &str = "00000000-0000-0000-0000-000000000000";
const STATE: &[&str] = &[
    "scope",
    "task_id",
    "thread_id",
    "lenses",
    "findings",
    "cost_usd",
    "skipped",
    "approved",
    "report",
];
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportedTask {
    pub id: String,
    pub lens: String,
    pub ordinal: usize,
    pub status: String,
    pub input: Value,
    pub output: Option<Value>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportedRun {
    pub thread: String,
    pub checkpoint: String,
    pub import_id: String,
    pub source_digest: String,
    pub stage: String,
    pub state: Value,
    pub decision: Option<bool>,
    pub tasks: Vec<ImportedTask>,
}
fn unsupported() -> HarnessError {
    HarnessError::invalid(
        "Legacy Fleet checkpoint uses an unsupported or ambiguous state; its bytes were preserved.",
    )
}
fn bytes_uuid(s: &str) -> Result<Vec<u8>> {
    let hex = s.replace('-', "");
    if hex.len() != 32 {
        return Err(unsupported());
    }
    (0..32)
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| unsupported()))
        .collect()
}
fn task_id(checkpoint: &str, v: u64, node: &str, step: i64, push: Option<usize>) -> Result<String> {
    let mut bytes = bytes_uuid(checkpoint)?;
    let kind = if push.is_some() {
        "__pregel_push"
    } else {
        "__pregel_pull"
    };
    let tail = push.map(|n| n.to_string()).unwrap_or_else(|| {
        if node == "__start__" {
            "__start__".into()
        } else {
            format!("branch:to:{node}")
        }
    });
    bytes.extend_from_slice(format!("{node}{step}{node}{kind}{tail}").as_bytes());
    let hex = if v > 1 {
        format!("{:032x}", xxhash_rust::xxh3::xxh3_128(&bytes))
    } else {
        use sha1::{Digest, Sha1};
        format!("{:x}", Sha1::digest(bytes))[..32].to_owned()
    };
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}
fn version_gt(left: &Value, right: Option<&Value>) -> Result<bool> {
    match (left, right) {
        (Value::String(a), Some(Value::String(b))) => Ok(a > b),
        (Value::String(a), None) => Ok(!a.is_empty()),
        (Value::Number(a), Some(Value::Number(b))) => {
            Ok(a.as_f64().ok_or_else(unsupported)? > b.as_f64().ok_or_else(unsupported)?)
        }
        (Value::Number(a), None) => Ok(a.as_f64().ok_or_else(unsupported)? > 0.),
        _ => Err(unsupported()),
    }
}
fn put_decision(current: &mut Option<bool>, candidate: bool) -> Result<()> {
    if current.is_some_and(|b| b != candidate) {
        return Err(unsupported());
    }
    *current = Some(candidate);
    Ok(())
}
fn fold(state: &mut Value, writes: &[(String, Value)]) -> Result<()> {
    for (channel, value) in writes {
        match channel.as_str() {
            "findings" | "skipped" => {
                let incoming = value.as_array().ok_or_else(unsupported)?;
                if state[channel].is_null() {
                    state[channel] = json!([]);
                }
                let array = state[channel].as_array_mut().ok_or_else(unsupported)?;
                array.extend(incoming.iter().cloned());
            }
            "cost_usd" => {
                if value.is_null() || state.get("cost_usd").is_some_and(Value::is_null) {
                    state["cost_usd"] = Value::Null;
                } else {
                    let add = value
                        .as_f64()
                        .filter(|n| n.is_finite() && *n >= 0.)
                        .ok_or_else(unsupported)?;
                    let previous = state["cost_usd"].as_f64().unwrap_or(0.);
                    state["cost_usd"] = json!(previous + add);
                }
            }
            key if STATE.contains(&key) => {
                state[key] = value.clone();
            }
            key if key.starts_with("branch:to:")
                || matches!(
                    key,
                    "__no_writes__"
                        | "__return__"
                        | "__resume__"
                        | "__scheduled__"
                        | "__error__"
                        | "__interrupt__"
                ) => {}
            _ => return Err(unsupported()),
        }
    }
    Ok(())
}
pub fn decode_checkpoint(
    path: &Path,
    thread: &str,
    saved_action: Option<&str>,
) -> Result<Option<ImportedRun>> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() || infra_core::fs::linked(&metadata) {
        return Err(unsupported());
    }
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let raw:Option<LegacyCheckpointRow>=db.query_row("SELECT checkpoint_id,type,checkpoint,metadata FROM checkpoints WHERE thread_id=?1 AND checkpoint_ns='' AND length(checkpoint)<=16777216 AND (metadata IS NULL OR length(metadata)<=1048576) ORDER BY checkpoint_id DESC LIMIT 1",[thread],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    let Some((checkpoint, kind, image, meta_image)) = raw else {
        let count: i64 = db.query_row(
            "SELECT count(*) FROM checkpoints WHERE thread_id=?1 AND checkpoint_ns='' LIMIT 1",
            [thread],
            |r| r.get(0),
        )?;
        if count != 0 {
            return Err(unsupported());
        }
        return Ok(None);
    };
    let mut payload = wire::typed(&kind, &image)?;
    let v = payload["v"]
        .as_u64()
        .filter(|v| (1..=4).contains(v))
        .ok_or_else(unsupported)?;
    if payload["id"] != checkpoint {
        return Err(unsupported());
    }
    let metadata = meta_image
        .as_deref()
        .map(|s| crate::strict_json::parse(s, 1024 * 1024))
        .transpose()?
        .unwrap_or(json!({}));
    let step = metadata["step"]
        .as_i64()
        .unwrap_or(-1)
        .checked_add(1)
        .ok_or_else(unsupported)?;
    let mut values = payload["channel_values"]
        .as_object()
        .cloned()
        .ok_or_else(unsupported)?;
    let mut versions = payload["channel_versions"]
        .as_object()
        .cloned()
        .ok_or_else(unsupported)?;
    let mut seen = payload["versions_seen"]
        .as_object()
        .cloned()
        .ok_or_else(unsupported)?;
    if v < 4
        && let Some(sends) = payload
            .get_mut("pending_sends")
            .and_then(Value::as_array_mut)
        && !sends.is_empty()
    {
        values.insert("__pregel_tasks".into(), json!(sends));
        let max = versions
            .values()
            .cloned()
            .reduce(|a, b| {
                if version_gt(&a, Some(&b)).unwrap_or(false) {
                    a
                } else {
                    b
                }
            })
            .ok_or_else(unsupported)?;
        versions.insert("__pregel_tasks".into(), max);
    }
    if v < 3 {
        let keys = versions.keys().cloned().collect::<Vec<_>>();
        for old in keys {
            let targets = if let Some(node) = old.strip_prefix("start:") {
                vec![node.to_string()]
            } else if old.starts_with("branch:") && old.matches(':').count() == 3 {
                vec![old.rsplit(':').next().unwrap_or("").into()]
            } else {
                match old.as_str() {
                    "review" => vec!["collect".into()],
                    "collect" => vec!["gate".into()],
                    "gate" => vec!["record".into()],
                    _ => vec![],
                }
            };
            for node in targets {
                if !wire::node(&node) {
                    return Err(unsupported());
                }
                let new = format!("branch:to:{node}");
                if let Some(old_version) = versions.get(&old).cloned()
                    && versions.get(&new).is_none_or(|new_version| {
                        version_gt(&old_version, Some(new_version)).unwrap_or(false)
                    })
                {
                    versions.insert(new.clone(), old_version);
                }
                if !values.contains_key(&new)
                    && let Some(value) = values.get(&old)
                {
                    values.insert(new.clone(), value.clone());
                }
                for observer in [node.as_str(), "__interrupt__"] {
                    if let Some(map) = seen.get_mut(observer).and_then(Value::as_object_mut)
                        && let Some(old_version) = map.remove(&old)
                        && map
                            .get(&new)
                            .is_none_or(|n| version_gt(&old_version, Some(n)).unwrap_or(false))
                    {
                        map.insert(new.clone(), old_version);
                    }
                }
            }
            if old.starts_with("start:")
                || old.starts_with("branch:") && old.matches(':').count() == 3
                || ["review", "collect", "gate", "record"].contains(&old.as_str())
            {
                versions.remove(&old);
                values.remove(&old);
            }
        }
    }
    let mut q=db.prepare("SELECT task_id,idx,channel,type,value FROM writes WHERE thread_id=?1 AND checkpoint_ns='' AND checkpoint_id=?2 ORDER BY task_id,idx LIMIT 10001")?;
    let mut rows = q.query(params![thread, checkpoint])?;
    let mut writes: BTreeMap<String, Vec<(String, Value)>> = BTreeMap::new();
    let mut raw_writes = Vec::new();
    let mut total = 0usize;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        count += 1;
        if count > 10000 {
            return Err(unsupported());
        }
        let tid: String = row.get(0)?;
        let idx: i64 = row.get(1)?;
        let channel: String = row.get(2)?;
        let kind: String = row.get(3)?;
        let bytes: Vec<u8> = row.get(4)?;
        total = total.checked_add(bytes.len()).ok_or_else(unsupported)?;
        if total > 64 * 1024 * 1024 {
            return Err(unsupported());
        }
        let decoded = wire::typed(&kind, &bytes)?;
        raw_writes.push(json!([tid, idx, channel, kind, store::sha(&bytes)]));
        writes.entry(tid).or_default().push((channel, decoded));
    }
    let mut state = Value::Object(
        values
            .iter()
            .filter(|(k, _)| STATE.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    );
    if state["thread_id"] != thread
        || !state["scope"].is_string()
        || !state["task_id"].is_string()
        || !state["lenses"].is_array()
    {
        return Err(unsupported());
    }
    let mut decision = None;
    if let Some(value) = state.get("approved") {
        put_decision(&mut decision, value.as_bool().ok_or_else(unsupported)?)?;
    }
    if matches!(saved_action, Some("approve" | "reject")) {
        put_decision(&mut decision, saved_action == Some("approve"))?;
    }
    let scope_tid = task_id(&checkpoint, v, "scope", step, None)?;
    if let Some(scope_writes) = writes.get(&scope_tid) {
        let packets = scope_writes
            .iter()
            .filter(|(key, _)| key == "__pregel_tasks")
            .map(|(_, value)| value.clone())
            .collect::<Vec<_>>();
        if !packets.is_empty() {
            if values
                .get("__pregel_tasks")
                .is_some_and(|v| v.as_array().is_some_and(|a| !a.is_empty()))
            {
                return Err(unsupported());
            }
            values.insert("__pregel_tasks".into(), json!(packets));
        }
    }
    let mut tasks = Vec::new();
    let sends = values.get("__pregel_tasks").cloned().unwrap_or(json!([]));
    let send_list = if let Some(a) = sends.as_array() {
        if a.len() == 2 && wire::tag(&a[0], "set") {
            a[1].as_array().ok_or_else(unsupported)?.clone()
        } else {
            a.clone()
        }
    } else {
        return Err(unsupported());
    };
    let mut consumed = BTreeSet::new();
    for (ordinal, send) in send_list.iter().enumerate() {
        if !wire::tag(send, "send") || send["node"] != "review" {
            return Err(unsupported());
        }
        let input = send["arg"].clone();
        let tid = task_id(&checkpoint, v, "review", step, Some(ordinal))?;
        let pending = writes.get(&tid).cloned().unwrap_or_default();
        let normal = pending
            .iter()
            .filter(|(key, _)| STATE.contains(&key.as_str()) || key.starts_with("branch:to:"))
            .cloned()
            .collect::<Vec<_>>();
        let done = !normal.is_empty();
        let mut output = json!({});
        if done {
            fold(&mut output, &normal)?;
            fold(&mut state, &normal)?;
            consumed.insert(tid.clone());
        }
        let error = pending
            .iter()
            .find(|(k, _)| k == "__error__")
            .map(|(_, v)| v.as_str().unwrap_or("Legacy reviewer failed.").to_string());
        tasks.push(ImportedTask {
            id: tid,
            lens: input["lens"].as_str().ok_or_else(unsupported)?.into(),
            ordinal,
            status: if done { "completed" } else { "pending" }.into(),
            input,
            output: done.then_some(output),
            error,
        });
    }
    let mut ready = Vec::new();
    for node in ["__start__", "scope", "collect", "gate", "record"] {
        let channel = if node == "__start__" {
            "__start__".into()
        } else {
            format!("branch:to:{node}")
        };
        if !values.contains_key(&channel) {
            continue;
        }
        let Some(version) = versions.get(&channel) else {
            return Err(unsupported());
        };
        let observed = seen
            .get(node)
            .and_then(Value::as_object)
            .and_then(|m| m.get(&channel));
        if !version_gt(version, observed)? {
            continue;
        }
        let tid = task_id(&checkpoint, v, node, step, None)?;
        let pending = writes.get(&tid).cloned().unwrap_or_default();
        let normal = pending
            .iter()
            .filter(|(k, _)| {
                STATE.contains(&k.as_str()) || k.starts_with("branch:to:") || k == "__no_writes__"
            })
            .cloned()
            .collect::<Vec<_>>();
        for (key, value) in &pending {
            if key == "__resume__" {
                let candidate = if let Some(a) = value.as_array() {
                    a.first().and_then(Value::as_bool)
                } else {
                    value.as_bool()
                }
                .ok_or_else(unsupported)?;
                put_decision(&mut decision, candidate)?;
            }
            if key == "approved" {
                put_decision(&mut decision, value.as_bool().ok_or_else(unsupported)?)?;
            }
        }
        if !normal.is_empty() {
            fold(&mut state, &normal)?;
            consumed.insert(tid);
            for (key, _) in normal {
                if let Some(next) = key.strip_prefix("branch:to:") {
                    ready.push(next.to_string());
                }
            }
        } else {
            ready.push(node.to_string());
        }
    }
    if let Some(global) = writes.get(NULL_TASK) {
        for (key, value) in global {
            if key == "__resume__" {
                put_decision(&mut decision, value.as_bool().ok_or_else(unsupported)?)?;
            } else if STATE.contains(&key.as_str()) {
                fold(&mut state, &[(key.clone(), value.clone())])?;
            }
        }
    }
    for (tid, pending) in &writes {
        if tid == NULL_TASK || consumed.contains(tid) || tasks.iter().any(|t| &t.id == tid) {
            continue;
        }
        if pending.iter().any(|(key, _)| STATE.contains(&key.as_str())) {
            return Err(unsupported());
        }
    }
    let stage = if tasks.iter().any(|t| t.status != "completed") {
        "review"
    } else if !tasks.is_empty() && ready.is_empty() {
        "collect"
    } else if ready.len() > 1 {
        return Err(unsupported());
    } else {
        match ready.first().map(String::as_str) {
            Some("__start__" | "scope") => "scope",
            Some("collect") => "collect",
            Some("gate") => {
                if decision.is_some() {
                    "record"
                } else {
                    "gate"
                }
            }
            Some("record") => "record",
            None => "done",
            _ => return Err(unsupported()),
        }
    };
    if let Some(value) = decision {
        state["approved"] = value.into();
    }
    let source_digest=store::sha(json!({"checkpoint":store::sha(&image),"metadata":meta_image.map(|v|store::sha(&v)),"writes":raw_writes}).to_string().as_bytes());
    let import_id = store::sha(format!("{thread}\0{checkpoint}\0{source_digest}").as_bytes());
    Ok(Some(ImportedRun {
        thread: thread.into(),
        checkpoint,
        import_id,
        source_digest,
        stage: stage.into(),
        state,
        decision,
        tasks,
    }))
}
pub fn persist(store: &SessionStore, run: &ImportedRun) -> Result<()> {
    let mut db = store.connection()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some(existing) = tx
        .query_row(
            "SELECT import_id FROM native_fleet_runs WHERE thread_id=?1",
            [&run.thread],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
    {
        if existing.as_deref() == Some(&run.import_id) {
            return Ok(());
        }
        return Err(HarnessError::conflict(
            "Native Fleet state already exists; refusing to replace it.",
        ));
    }
    tx.execute("INSERT INTO native_fleet_runs(thread_id,state,stage,decision,import_id)VALUES(?1,?2,?3,?4,?5)",params![run.thread,run.state.to_string(),run.stage,run.decision.map(i64::from),run.import_id])?;
    for task in &run.tasks {
        tx.execute("INSERT INTO native_fleet_tasks(thread_id,task_id,lens,ordinal,status,input,output,error)VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![run.thread,task.id,task.lens,task.ordinal as i64,task.status,task.input.to_string(),task.output.as_ref().map(Value::to_string),task.error])?;
    }
    tx.execute(
        "INSERT INTO native_checkpoint_imports VALUES(?1,?2,?3,?4,?5)",
        params![
            run.import_id,
            run.thread,
            run.checkpoint,
            run.source_digest,
            store::now()
        ],
    )?;
    tx.commit()?;
    Ok(())
}
