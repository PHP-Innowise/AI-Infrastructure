//! Turn boundaries, clone-local bindings and authoritative checkpoint writes.
use crate::{
    BankService, BrainRepository, ErrorKind, Result, RuntimeContext, RuntimeError, Store,
    UpdateRecord, advisory, continuity,
    domain::{self, json},
    query, storage,
};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json as object};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::Duration,
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct TurnRequest {
    pub task_id: String,
    pub flush: bool,
    pub flush_after: usize,
    pub max_files: usize,
    pub transcript: Option<PathBuf>,
}
impl Default for TurnRequest {
    fn default() -> Self {
        Self {
            task_id: String::new(),
            flush: false,
            flush_after: 5,
            max_files: 20,
            transcript: None,
        }
    }
}
pub fn active_claimants(context: &RuntimeContext, id: &str) -> Result<Vec<Value>> {
    let inventory = BrainRepository::new(context.clone())?.inventory(false)?;
    if inventory.problems.iter().any(|problem| {
        problem.id.as_deref() == Some(id) || problem.external_id.as_deref() == Some(id)
    }) {
        return Err(RuntimeError::invalid(
            "A task claiming this identity cannot be read",
        ));
    }
    let mut tasks = inventory
        .records
        .into_iter()
        .filter(|image| {
            image.record["type"] == "task"
                && image.record["external_id"] == id
                && !domain::is_terminal("task", image.record["status"].as_str().unwrap_or(""))
        })
        .map(|image| image.record)
        .collect::<Vec<_>>();
    tasks.sort_by(|left, right| {
        left["created_at"]
            .as_str()
            .cmp(&right["created_at"].as_str())
            .then_with(|| left["id"].as_str().cmp(&right["id"].as_str()))
    });
    Ok(tasks)
}
pub fn served_uuid(
    store: Option<&Store>,
    context: &RuntimeContext,
    id: &str,
) -> Result<Option<String>> {
    let brain = BrainRepository::new(context.clone())?;
    if let Some(store) = store
        && let Some(binding) = store.binding(id)?
    {
        match brain.get_task(&binding.task_uuid) {
            Ok(task) => {
                if task["external_id"] != binding.external_id {
                    return Err(RuntimeError::conflict(
                        "Task binding names a different alias",
                    ));
                }
                if !domain::is_terminal("task", task["status"].as_str().unwrap_or("")) {
                    return Ok(Some(binding.task_uuid));
                }
            }
            Err(error) if error.kind == ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(active_claimants(context, id)?
        .first()
        .and_then(|task| task["id"].as_str())
        .map(String::from))
}
pub fn ensure(store: &mut Store, context: &RuntimeContext, id: &str) -> Result<Value> {
    let id = storage::validate_task_id(id)?;
    let goal = storage::derive_goal(&id);
    domain::guard_shared_text("Working task", &[id.clone(), goal.clone()])?;
    if context.mode == "lightweight" {
        if store.working(&id)?.is_some() {
            return Ok(object!({"provisioned":false,"rebound":null}));
        }
        let task = store.start(storage::StartTask {
            task_id: id,
            goal,
            ..Default::default()
        })?;
        return Ok(
            object!({"provisioned":true,"rebound":null,"created":{"task_uuid":null,"generation":task.created_at}}),
        );
    }
    let brain = BrainRepository::new(context.clone())?;
    let _guard = brain.lock()?;
    if let Some(binding) = store.binding(&id)? {
        match brain.get_task(&binding.task_uuid) {
            Ok(task) if !domain::is_terminal("task", task["status"].as_str().unwrap_or("")) => {
                if task["external_id"] != binding.external_id {
                    return Err(RuntimeError::conflict("Working task binding mismatch"));
                }
                return Ok(object!({"provisioned":false,"rebound":null}));
            }
            Ok(_)
            | Err(RuntimeError {
                kind: ErrorKind::NotFound,
                ..
            }) => {
                store.remove_binding(&binding.external_id)?;
            }
            Err(error) => return Err(error),
        }
    }
    if let Some(task) = active_claimants(context, &id)?.into_iter().next() {
        store.bind(
            &id,
            task["id"].as_str().unwrap_or(""),
            task["revision"].as_u64().unwrap_or(1),
        )?;
        return Ok(object!({"provisioned":false,"rebound":task["id"]}));
    }
    let existing = brain
        .inventory(false)?
        .records
        .into_iter()
        .filter(|image| image.record["type"] == "task" && image.record["external_id"] == id)
        .map(|image| image.record)
        .collect::<Vec<_>>();
    if !existing.is_empty() {
        if existing
            .iter()
            .all(|task| domain::is_terminal("task", task["status"].as_str().unwrap_or("")))
        {
            BankService::new(context.clone()).compact(Some(&id))?;
        } else {
            return Err(RuntimeError::conflict("Working task identity is ambiguous"));
        }
    }
    let task = brain.create_task(&id, &goal, vec![], vec![], "team")?;
    if let Err(error) = store.bind(
        &id,
        task["id"].as_str().unwrap_or(""),
        task["revision"].as_u64().unwrap_or(1),
    ) {
        let _ = store.transaction(|conn| {
            conn.execute(
                "DELETE FROM task_bindings WHERE task_uuid=?1",
                [task["id"].as_str().unwrap_or("")],
            )?;
            conn.execute(
                "DELETE FROM working_tasks WHERE task_id=?1 AND goal=?2",
                params![id, task["id"].as_str().unwrap_or("")],
            )?;
            Ok(())
        });
        brain.rollback_created(task["id"].as_str().unwrap_or(""))?;
        return Err(error);
    }
    let generation = store
        .working(&id)?
        .ok_or_else(|| RuntimeError::not_found("Working pointer unavailable"))?
        .created_at;
    Ok(
        object!({"provisioned":true,"rebound":null,"created":{"task_uuid":task["id"],"generation":generation}}),
    )
}
pub fn rebind(
    store: &mut Store,
    context: &RuntimeContext,
    id: &str,
    uuid: Option<&str>,
) -> Result<Value> {
    let brain = BrainRepository::new(context.clone())?;
    let _guard = brain.lock()?;
    let task = if let Some(uuid) = uuid {
        brain.get_task(uuid)?
    } else {
        let uuid = served_uuid(Some(store), context, id)?
            .ok_or_else(|| RuntimeError::not_found("Working task not found"))?;
        brain.get_task(&uuid)?
    };
    if task["external_id"] != id
        || domain::is_terminal("task", task["status"].as_str().unwrap_or(""))
    {
        return Err(RuntimeError::conflict("Task is unavailable for rebind"));
    }
    if let Some(binding) = store.binding(id)? {
        if binding.task_uuid == task["id"].as_str().unwrap_or("") {
            store.refresh_binding(id, task["revision"].as_u64().unwrap_or(1))?;
            return Ok(serde_json::to_value(store.binding(id)?)?);
        }
        return Err(RuntimeError::conflict(
            "Working task already bound to another record",
        ));
    }
    store.bind(
        id,
        task["id"].as_str().unwrap_or(""),
        task["revision"].as_u64().unwrap_or(1),
    )?;
    Ok(serde_json::to_value(store.binding(id)?)?)
}
pub fn footprint(context: &RuntimeContext) -> BTreeMap<String, String> {
    for name in [
        "memory-bank/local/accelerator-install.json",
        ".infra-manifest.json",
    ] {
        let path = context.roots.state.join(name);
        let Ok(image) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if crate::fs::linked_at(&path, &image).unwrap_or(true)
            || !image.is_file()
            || image.len() > 4 * 1024 * 1024
        {
            continue;
        }
        let Ok(bytes) = crate::fs::read_regular(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        if let Some(files) = value["files"].as_object() {
            return files
                .iter()
                .filter_map(|(path, value)| {
                    value
                        .as_str()
                        .filter(|value| [40, 64].contains(&value.len()))
                        .map(|value| (path.clone(), value.into()))
                })
                .collect();
        }
    }
    BTreeMap::new()
}
pub fn installed_as_written(
    context: &RuntimeContext,
    path: &str,
    footprint: &BTreeMap<String, String>,
) -> bool {
    if path.ends_with('/') {
        return footprint.keys().any(|key| key.starts_with(path));
    }
    if path == ".infra-manifest.json" {
        return true;
    }
    let Some(expected) = footprint.get(path) else {
        return false;
    };
    let target = context.roots.project.join(path);
    let Ok(metadata) = std::fs::symlink_metadata(&target) else {
        return false;
    };
    if crate::fs::linked_at(&target, &metadata).unwrap_or(true)
        || !metadata.is_file()
        || metadata.len() > 1024 * 1024
    {
        return false;
    }
    let Ok(bytes) = crate::fs::read_regular(&target) else {
        return false;
    };
    if expected.len() == 64 {
        query::sha256(&bytes) == *expected
    } else {
        let mut framed = format!("blob {}\0", bytes.len()).into_bytes();
        framed.extend(bytes);
        sha1(&framed) == *expected
    }
}
pub fn runtime_owned(path: &str) -> bool {
    path == "project-brain/"
        || path == "memory-bank/INDEX.md"
        || [
            "memory-bank/chunks/",
            "memory-bank/local/",
            "project-brain/dynamic/",
            "project-brain/control/",
            "project-brain/indexes/",
            "project-brain/archive/",
            "project-brain/local/",
        ]
        .iter()
        .any(|prefix| path.starts_with(prefix))
}
pub fn status_paths(context: &RuntimeContext) -> Result<Vec<String>> {
    let output = crate::git::run(
        &context.roots.project,
        &crate::git::strings(&["status", "--porcelain=v1", "-z", "--untracked-files=normal"]),
        None,
        Duration::from_secs(5),
        16 * 1024 * 1024,
    )?;
    if output.code != 0 {
        return Err(RuntimeError::unavailable("Git status unavailable"));
    }
    let parts = output.stdout.split(|byte| *byte == 0).collect::<Vec<_>>();
    let mut output = Vec::new();
    let mut at = 0;
    while at < parts.len() {
        let part = parts[at];
        if part.len() >= 4 {
            let path = std::str::from_utf8(&part[3..])
                .map_err(|_| RuntimeError::invalid("Git path is not UTF-8"))?;
            output.push(path.into());
            if part[..2].iter().any(|byte| b"RC".contains(byte)) {
                at += 1;
            }
        }
        at += 1;
    }
    Ok(output)
}
pub fn changed_paths(context: &RuntimeContext) -> Result<(Vec<String>, Vec<String>)> {
    let footprint = footprint(context);
    let mut allowed = Vec::new();
    let mut excluded = Vec::new();
    for path in status_paths(context)? {
        if runtime_owned(&path) || installed_as_written(context, &path, &footprint) {
            continue;
        }
        if domain::source_path_problem(&path).is_some() {
            excluded.push(path);
        } else if !allowed.contains(&path) {
            allowed.push(path);
        }
    }
    excluded.sort();
    excluded.dedup();
    Ok((allowed, excluded))
}
pub fn head(context: &RuntimeContext) -> Option<String> {
    crate::git::run(
        &context.roots.project,
        &crate::git::strings(&["rev-parse", "--verify", "--quiet", "HEAD"]),
        None,
        Duration::from_secs(2),
        16384,
    )
    .ok()
    .filter(|output| output.code == 0)
    .and_then(|output| String::from_utf8(output.stdout).ok())
    .map(|text| text.trim().to_string())
    .filter(|text| !text.is_empty())
}
pub fn head_advanced(store: &mut Store, context: &RuntimeContext, id: &str) -> bool {
    let Some(head) = head(context) else {
        return false;
    };
    let operation = (|| -> Result<bool> {
        let mut value = store
            .state()?
            .get("turn-heads")
            .and_then(|raw| json::parse(raw).ok())
            .filter(Value::is_object)
            .unwrap_or(object!({}));
        let previous = value.as_object_mut().and_then(|object| object.remove(id));
        value[id] = object!(head);
        if let Some(object) = value.as_object_mut() {
            while object.len() > 200 {
                let Some(key) = object.keys().next().cloned() else {
                    break;
                };
                object.remove(&key);
            }
        }
        store.put_state(&BTreeMap::from([(
            "turn-heads".into(),
            json::canonical(&value)?,
        )]))?;
        Ok(previous
            .as_ref()
            .and_then(Value::as_str)
            .is_some_and(|previous| previous != head))
    })();
    operation.unwrap_or(false)
}
pub fn default_branch(store: &mut Store, context: &RuntimeContext) -> Option<String> {
    let origin_head = crate::git::run(
        &context.roots.project,
        &crate::git::strings(&["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"]),
        None,
        Duration::from_secs(2),
        16384,
    )
    .ok()
    .filter(|output| output.code == 0)
    .and_then(|output| String::from_utf8(output.stdout).ok())
    .map(|value| value.trim().to_string());
    choose_default_branch(store, origin_head, |candidate| {
        crate::git::run(
            &context.roots.project,
            &crate::git::strings(&["rev-parse", "--verify", "--quiet", candidate]),
            None,
            Duration::from_secs(2),
            16384,
        )
        .is_ok_and(|output| output.code == 0)
    })
}
/// The first candidate `exists` admits: the target of origin's HEAD, then
/// main, then master. The store keeps the choice.
fn choose_default_branch(
    store: &mut Store,
    origin_head: Option<String>,
    exists: impl Fn(&str) -> bool,
) -> Option<String> {
    let chosen = origin_head
        .into_iter()
        .chain(["refs/heads/main".into(), "refs/heads/master".into()])
        .find(|candidate| exists(candidate))?;
    // Rewriting an unchanged choice would still cost a synced write each turn.
    if store
        .state()
        .ok()
        .and_then(|state| state.get("default_branch").cloned())
        .as_deref()
        != Some(chosen.as_str())
    {
        let _ = store.put_state(&BTreeMap::from([("default_branch".into(), chosen.clone())]));
    }
    Some(chosen)
}
pub fn branch_subjects(store: &mut Store, context: &RuntimeContext) -> (Vec<String>, usize) {
    let operation = (|| -> Result<(Vec<String>, usize)> {
        let target = default_branch(store, context)
            .ok_or_else(|| RuntimeError::unavailable("Default branch unavailable"))?;
        let output = crate::git::run(
            &context.roots.project,
            &crate::git::strings(&["merge-base", "HEAD", &target]),
            None,
            Duration::from_secs(3),
            16384,
        )?;
        if output.code != 0 {
            return Ok((vec![], 0));
        }
        let base = std::str::from_utf8(&output.stdout)
            .map_err(|_| RuntimeError::invalid("Invalid merge-base"))?
            .trim();
        if head(context).as_deref() == Some(base) {
            return Ok((vec![], 0));
        }
        let output = crate::git::run(
            &context.roots.project,
            &crate::git::strings(&[
                "log",
                "--no-merges",
                "-z",
                "--format=%s",
                "--max-count=500",
                &format!("{base}..HEAD"),
                "--",
                ".",
            ]),
            None,
            Duration::from_secs(3),
            1024 * 1024,
        )?;
        if output.code != 0 {
            return Ok((vec![], 0));
        }
        let subjects = output
            .stdout
            .split(|byte| *byte == 0)
            .filter_map(|bytes| std::str::from_utf8(bytes).ok())
            .map(crate::results::collapse)
            .filter(|subject| !subject.is_empty())
            .collect::<Vec<_>>();
        let total = subjects.len();
        let shown=subjects.into_iter().filter(|subject|domain::guard_shared_text("Checkpoint subject",std::slice::from_ref(subject)).is_ok()&&!regex::Regex::new(r"(?im)^\s*(?:user|assistant|system|developer|tool|prompt|response|reasoning|stdout|stderr|log)\s*:").is_ok_and(|pattern|pattern.is_match(subject))).take(5).map(|subject|if subject.chars().count()>80{format!("{}…",domain::rstrip(&query::chars(&subject,79)))}else{subject}).collect();
        Ok((shown, total))
    })();
    operation.unwrap_or((vec![], 0))
}
pub fn flush(
    store: &mut Store,
    context: &RuntimeContext,
    id: &str,
    file_limit: usize,
) -> Result<Option<Value>> {
    let pending = store.pending_turns(id)?;
    if pending.is_empty() {
        return Ok(None);
    }
    let footprint = footprint(context);
    let mut files = domain::unique_strings(pending.iter().flat_map(|turn| turn.files.clone()))
        .into_iter()
        .filter(|path| !runtime_owned(path) && !installed_as_written(context, path, &footprint))
        .collect::<Vec<_>>();
    let count = files.len();
    let omitted = count.saturating_sub(file_limit);
    files.truncate(file_limit);
    let first = chrono::DateTime::parse_from_rfc3339(&pending[0].created_at)
        .map(|at| {
            at.with_timezone(&chrono::Utc)
                .format("%Y-%m-%d %H:%M UTC")
                .to_string()
        })
        .unwrap_or(pending[0].created_at.clone());
    let mut summary = format!(
        "Auto-checkpoint: {} turn(s) since {first}, {count} file(s) touched.",
        pending.len()
    );
    if omitted > 0 {
        summary.push_str(&format!(
            " {omitted} path(s) beyond the per-flush limit are not listed."
        ));
    }
    let (subjects, total) = branch_subjects(store, context);
    if !subjects.is_empty() {
        summary.push_str(&format!(
            " Branch commits, newest first: {}",
            subjects.join("; ")
        ));
        if total > subjects.len() {
            summary.push_str(&format!(" (+{} more)", total - subjects.len()));
        }
        summary.push('.');
    } else if total > 0 {
        summary.push_str(&format!(" {total} commit(s) on this branch."));
    }
    let topic = continuity::topic_begin(store, context, id);
    let ensured = ensure(store, context, id)?;
    let revision = if context.mode == "lightweight" {
        store.update(storage::UpdateTask {
            task_id: id.into(),
            auto_checkpoint: Some(summary),
            files,
            ..Default::default()
        })?;
        None
    } else {
        let brain = BrainRepository::new(context.clone())?;
        let _guard = brain.lock()?;
        let binding = store
            .binding(id)?
            .ok_or_else(|| RuntimeError::not_found("Working binding unavailable"))?;
        let before = brain.snapshot_record(&binding.task_uuid)?;
        let current = brain.get_task(&binding.task_uuid)?;
        let expected_revision = current["revision"]
            .as_u64()
            .filter(|revision| *revision > 0)
            .ok_or_else(|| RuntimeError::invalid("Authoritative task revision is unavailable"))?;
        let updated = brain.update(
            &binding.task_uuid,
            UpdateRecord {
                auto_checkpoint: Some(summary),
                files,
                expected_revision: Some(expected_revision),
                ..Default::default()
            },
        )?;
        let revision = updated["revision"].as_u64().unwrap_or(1);
        if let Err(error) = store.refresh_binding(&binding.external_id, revision) {
            brain.restore_record(&before)?;
            return Err(error);
        }
        Some(revision)
    };
    store.transaction(|conn| {
        for turn in &pending {
            conn.execute(
                "DELETE FROM turn_deltas WHERE id=?1 AND task_id=?2",
                params![turn.id, id],
            )?;
        }
        Ok(())
    })?;
    if let Some(created) = ensured.get("created") {
        continuity::topic_handoff(
            store,
            context,
            id,
            &topic,
            created["task_uuid"].as_str(),
            created["generation"].as_str().unwrap_or(""),
        );
    }
    Ok(Some(
        object!({"revision":revision,"files_omitted":omitted,"provisioned":ensured["provisioned"],"rebound":ensured["rebound"],"closed_task":null,"discarded_turns":0}),
    ))
}
pub fn run_turn(
    store: &mut Store,
    context: &RuntimeContext,
    request: &TurnRequest,
) -> Result<Value> {
    if request.flush_after == 0 || request.max_files == 0 {
        return Err(RuntimeError::invalid("Turn thresholds must be positive"));
    }
    let id = storage::validate_task_id(&request.task_id)?;
    let bank = BankService::new(context.clone());
    let _ = heal_bank(context, &bank);
    let (files, excluded) = changed_paths(context)?;
    let advanced = head_advanced(store, context, &id);
    let delta = if !files.is_empty() || advanced {
        Some(store.append_turn(&id, &files)?)
    } else {
        None
    };
    let buffered = store.pending_turns(&id)?.len();
    let mut ensured = object!({"provisioned":false,"rebound":null});
    let mut closed = None;
    let flushed = if buffered > 0 && (request.flush || buffered >= request.flush_after) {
        if context.mode == "governed" && active_claimants(context, &id)?.is_empty() {
            let brain = BrainRepository::new(context.clone())?;
            let records = brain.records(true)?;
            closed = records
                .iter()
                .find(|image| {
                    image.record["type"] == "task"
                        && image.record["external_id"] == id
                        && domain::is_terminal(
                            "task",
                            image.record["status"].as_str().unwrap_or(""),
                        )
                })
                .map(|image| image.record["id"].clone());
        }
        if let Some(closed) = &closed {
            let captured = store.pending_turns(&id)?;
            store.transaction(|conn| {
                for turn in captured {
                    conn.execute("DELETE FROM turn_deltas WHERE id=?1", [turn.id])?;
                }
                Ok(())
            })?;
            Some(
                object!({"revision":null,"closed_task":closed,"discarded_turns":buffered,"files_omitted":0,"provisioned":false,"rebound":null}),
            )
        } else {
            flush(store, context, &id, request.max_files)?
        }
    } else {
        if request.flush {
            ensured = ensure(store, context, &id)?;
        }
        None
    };
    let mut consolidation = object!({"enabled":false,"created":0,"updated":0,"superseded":0,"blocked":0,"tickets":0,"error":null});
    let mut promotion =
        object!({"enabled":false,"promoted":[],"failed":[],"blocked":[],"skipped":0});
    let mut compaction = object!({"enabled":false,"moved":0,"pending":0,"error":null});
    if context.mode == "governed" {
        if flushed.is_some() {
            consolidation = bank.auto_consolidate()?;
            promotion = bank.auto_promote(10, None, None)?;
            compaction = bank.auto_compact()?;
        } else {
            promotion = bank.retry_pending()?;
        }
    }
    let recent = continuity::record_recent(
        store,
        context,
        &continuity::transcript_files(request.transcript.as_deref(), context),
    )
    .unwrap_or(0);
    let result = object!({"task_id":id,"delta_id":delta,"files":files.len(),"excluded":excluded,"recent_files":recent,"consolidation":consolidation,"pending":if flushed.is_some(){0}else{buffered},"flushed":flushed.is_some()&&closed.is_none(),"revision":flushed.as_ref().and_then(|value|value["revision"].as_u64()),"files_omitted":flushed.as_ref().and_then(|value|value["files_omitted"].as_u64()).unwrap_or(0),"provisioned":flushed.as_ref().unwrap_or(&ensured)["provisioned"],"rebound":flushed.as_ref().unwrap_or(&ensured)["rebound"],"closed_task":closed,"discarded_turns":if closed.is_some(){buffered}else{0},"completion_candidates":merge_candidates(store,context)?,"promoted":promotion["promoted"],"promotion_failed":promotion["failed"],"promotion_blocked":promotion["blocked"],"promotion_skipped":promotion["skipped"],"archived":compaction["moved"],"archivable_pending":compaction["pending"]});
    let mut report = result.clone();
    report["schema_version"] = object!(1);
    report["timestamp"] = object!(domain::utc_now());
    report["excluded_paths"] = object!(excluded);
    report["compaction_errors"] = if let Some(error) = compaction["error"].as_str() {
        object!([error])
    } else {
        object!([])
    };
    advisory::write_last_turn(context, &report);
    Ok(result)
}
fn heal_bank(context: &RuntimeContext, bank: &BankService) -> Result<()> {
    let directory = context.roots.state.join("memory-bank/chunks");
    if !directory.is_dir() {
        return Ok(());
    }
    let newest = std::fs::read_dir(directory)?
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("MEM-"))
        .filter_map(|entry| {
            entry
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok())
        })
        .max();
    let indexed = std::fs::metadata(context.roots.state.join("memory-bank/INDEX.md"))
        .and_then(|metadata| metadata.modified())
        .ok();
    if newest.is_some() && newest > indexed {
        bank.index()?;
    }
    Ok(())
}
pub fn merge_candidates(store: &mut Store, context: &RuntimeContext) -> Result<Vec<Value>> {
    let tasks = BrainRepository::new(context.clone())?
        .records(false)?
        .into_iter()
        .filter(|image| {
            image.record["type"] == "task"
                && !domain::is_terminal("task", image.record["status"].as_str().unwrap_or(""))
        })
        .collect::<Vec<_>>();
    if tasks.is_empty() {
        return Ok(vec![]);
    }
    let query = |arguments: &[&str]| -> Option<Vec<u8>> {
        let output = crate::git::run(
            &context.roots.project,
            &crate::git::strings(arguments),
            None,
            Duration::from_secs(2),
            8 * 1024 * 1024,
        )
        .ok()?;
        if output.code == 0 {
            Some(output.stdout)
        } else {
            None
        }
    };
    // One listing tells which default branch exists, the commit it names and
    // where every task's ref points: four fewer Git processes per turn than
    // asking `default_branch` and `rev-parse` first.
    let Some(listing) = query(&[
        "for-each-ref",
        "--format=%(objectname) %(*objectname) %(objecttype) %(*objecttype) %(symref) %(refname)",
    ]) else {
        return Ok(vec![]);
    };
    let mut listed = BTreeMap::new();
    // Task identifiers are UTF-8. An unrelated byte-only Unix ref must not
    // invalidate the whole snapshot or collide with a lossy replacement name.
    for line in listing
        .split(|byte| *byte == b'\n')
        .filter_map(|line| std::str::from_utf8(line).ok())
    {
        // Ref names hold no spaces, so only the last field may contain one.
        if let [pointed, peeled, kind, peeled_kind, symref, reference] =
            line.splitn(6, ' ').collect::<Vec<_>>()[..]
        {
            listed.insert(reference, [pointed, peeled, kind, peeled_kind, symref]);
        }
    }
    let origin_head = listed
        .get("refs/remotes/origin/HEAD")
        .map(|fields| fields[4].to_string())
        .filter(|target| !target.is_empty());
    let Some(target) = choose_default_branch(store, origin_head, |candidate| {
        listed.contains_key(candidate)
    }) else {
        return Ok(vec![]);
    };
    let target_commit = match listed.get(target.as_str()) {
        Some([pointed, _, "commit", _, _]) => Some(pointed.to_string()),
        Some([_, peeled, "tag", "commit", _]) => Some(peeled.to_string()),
        _ => query(&[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{target}^{{commit}}"),
        ])
        .and_then(|value| String::from_utf8(value).ok())
        .map(|value| value.trim().to_owned()),
    };
    let Some(target_commit) = target_commit.filter(|value| !value.is_empty()) else {
        return Ok(vec![]);
    };
    let merged_argument = format!("--merged={target}");
    let Some(merged) = query(&["for-each-ref", "--format=%(refname)", &merged_argument]) else {
        return Ok(vec![]);
    };
    let references = listed
        .iter()
        .map(|(reference, [pointed, peeled, ..])| {
            (
                *reference,
                if peeled.is_empty() { *pointed } else { *peeled },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let reachable = merged
        .split(|byte| *byte == b'\n')
        .filter_map(|line| std::str::from_utf8(line).ok())
        .filter(|line| !line.is_empty())
        .collect::<BTreeSet<_>>();
    let mut candidates = Vec::new();
    for image in tasks {
        let id = image.record["external_id"].as_str().unwrap_or("");
        if id == target || id == target.rsplit('/').next().unwrap_or(target.as_str()) {
            continue;
        }
        let reference = [
            format!("refs/heads/{id}"),
            format!("refs/{id}"),
            format!("refs/tags/{id}"),
            format!("refs/remotes/{id}"),
            format!("refs/remotes/{id}/HEAD"),
        ]
        .into_iter()
        .find(|name| references.contains_key(name.as_str()));
        if let Some(reference) = reference
            && reachable.contains(reference.as_str())
            && references.get(reference.as_str()).copied() != Some(target_commit.as_str())
        {
            // Equal tips include never-diverged and fast-forward branches.
            // Advisory merge evidence requires that the target moved ahead.
            candidates.push(object!({"task_id":id,"task_uuid":image.record["id"],"revision":image.record["revision"],"target":target,"evidence":"branch is an ancestor of the merge target"}));
        }
    }
    Ok(candidates)
}
fn sha1(bytes: &[u8]) -> String {
    let mut data = bytes.to_vec();
    let length = bytes.len() as u64 * 8;
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&length.to_be_bytes());
    let mut h = [
        0x67452301u32,
        0xefcdab89,
        0x98badcfe,
        0x10325476,
        0xc3d2e1f0,
    ];
    for block in data.as_chunks::<64>().0 {
        let mut w = [0u32; 80];
        for (index, part) in block.as_chunks::<4>().0.iter().enumerate() {
            w[index] = u32::from_be_bytes([part[0], part[1], part[2], part[3]]);
        }
        for index in 16..80 {
            w[index] = (w[index - 3] ^ w[index - 8] ^ w[index - 14] ^ w[index - 16]).rotate_left(1);
        }
        let [a0, b0, c0, d0, e0] = h;
        let (mut a, mut b, mut c, mut d, mut e) = (a0, b0, c0, d0, e0);
        for (index, word) in w.into_iter().enumerate() {
            let (f, k) = match index {
                0..=19 => ((b & c) | (!b & d), 0x5a827999),
                20..=39 => (b ^ c ^ d, 0x6ed9eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1bbcdc),
                _ => (b ^ c ^ d, 0xca62c1d6),
            };
            let next = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = next;
        }
        for (index, value) in [a, b, c, d, e].into_iter().enumerate() {
            h[index] = h[index].wrapping_add(value);
        }
    }
    h.iter().map(|word| format!("{word:08x}")).collect()
}
