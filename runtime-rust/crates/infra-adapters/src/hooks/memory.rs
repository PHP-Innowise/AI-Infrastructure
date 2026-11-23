use super::{Context, valid_path};
use crate::TailTrim;
use crate::output::Output;
use infra_core::{Result, Store, storage::OpenOptions};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
const UNSAVED: &str = "Previous native memory result was not saved completely.";
const HOOK_PREFLIGHT_RACY_NS: i128 = 2_000_000_000;
const HOOK_PREFLIGHT_CACHE_FILE: &str = "memory-bank/local/hook-refresh-preflight.json";
const HOOK_WRITE_PREFLIGHT_CACHE_FILE: &str = "memory-bank/local/hook-write-preflight.json";
const HOOK_CONTINUITY_PREFLIGHT_CACHE_FILE: &str =
    "memory-bank/local/hook-continuity-preflight.json";

fn session_id_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"^[A-Za-z0-9._:-]{1,128}$").expect("constant session regex")
    })
}

fn time_to_unix_ns(time: SystemTime) -> Option<i128> {
    match time.duration_since(UNIX_EPOCH) {
        Ok(delta) => i128::try_from(delta.as_nanos()).ok(),
        Err(err) => i128::try_from(err.duration().as_nanos())
            .ok()
            .and_then(|ns| ns.checked_neg()),
    }
}

fn now_unix_ns() -> i128 {
    time_to_unix_ns(SystemTime::now()).unwrap_or(0)
}

fn settled_mtime(mtime_ns: i128, now_ns: i128) -> bool {
    mtime_ns > 0 && now_ns > HOOK_PREFLIGHT_RACY_NS && mtime_ns <= now_ns - HOOK_PREFLIGHT_RACY_NS
}

fn max_dir_mtime_ns(dir: &Path, remaining: &mut usize) -> i128 {
    if !dir.is_dir() || *remaining == 0 {
        return 0;
    }
    let mut max_ns = dir
        .metadata()
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(time_to_unix_ns)
        .unwrap_or(0);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return max_ns;
    };
    for entry in entries.flatten() {
        if *remaining == 0 {
            break;
        }
        *remaining -= 1;
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            return i128::MAX;
        }
        if let Some(ns) = meta.modified().ok().and_then(time_to_unix_ns) {
            max_ns = max_ns.max(ns);
        }
        if meta.is_dir() && path.file_name().is_none_or(|n| n != "local") {
            max_ns = max_ns.max(max_dir_mtime_ns(&path, remaining));
        }
    }
    max_ns
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct RefreshPreflightRecord {
    task_id: String,
    index_gen: String,
    git_head: String,
    host: String,
    query_digest: String,
    db_mtime_ns: i128,
    db_len: u64,
    pending_deltas: i64,
    task_updated_at: String,
    tree_max_mtime_ns: i128,
    transcript_stamp: String,
    stdout: Vec<u8>,
}

fn preflight_key_snapshot(
    scope: &Context,
    query: &str,
    now_ns: i128,
) -> Option<RefreshPreflightRecord> {
    if std::env::var("HARNESS_DISABLE_HOOK_PREFLIGHT").as_deref() == Ok("1") {
        return None;
    }
    let task_id = scope.task.as_deref().filter(|t| !t.is_empty())?.to_owned();
    if query.trim().is_empty() || query.starts_with("INTERNAL_NATIVE_MEMORY_RESULT ") {
        return None;
    }
    if scope.host == "cursor"
        && scope.roots.state == scope.roots.tooling
        && !scope
            .roots
            .tooling
            .join(".cursor/rules/working-memory.mdc")
            .is_file()
    {
        return None;
    }
    let db_path = infra_core::storage::default_database(&scope.roots).ok()?;
    let db_meta = std::fs::symlink_metadata(&db_path).ok()?;
    if db_meta.file_type().is_symlink() || !db_meta.is_file() {
        return None;
    }
    let db_mtime_ns = db_meta.modified().ok().and_then(time_to_unix_ns)?;
    if !settled_mtime(db_mtime_ns, now_ns) {
        return None;
    }

    let mut budget = 1500usize;
    let chunks_mtime = max_dir_mtime_ns(&scope.roots.state.join("memory-bank/chunks"), &mut budget);
    let brain_mtime = max_dir_mtime_ns(&scope.roots.brain_root(), &mut budget);
    let mut tree_max_mtime_ns = chunks_mtime.max(brain_mtime).max(db_mtime_ns);
    let (git_head, git_idx_mtime) = git_head_and_index_mtime(&scope.roots.project);
    tree_max_mtime_ns = tree_max_mtime_ns.max(git_idx_mtime);
    if tree_max_mtime_ns == i128::MAX || !settled_mtime(tree_max_mtime_ns, now_ns) {
        return None;
    }

    let store = Store::open_read_only(&db_path, Duration::from_millis(250))
        .or_else(|_| Store::open(&db_path, OpenOptions::default()))
        .ok()?;
    let index_gen = infra_core::storage::index_generation(&store.conn).ok()?;
    let pending_deltas: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM turn_deltas WHERE task_id=?1",
            [&task_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let task_updated_at: String = store
        .conn
        .query_row(
            "SELECT COALESCE(MAX(updated_at), '') FROM working_tasks WHERE task_id=?1",
            [&task_id],
            |r| r.get(0),
        )
        .unwrap_or_default();

    let transcript_stamp = transcript_stamp(scope, None).unwrap_or_default();

    Some(RefreshPreflightRecord {
        task_id,
        index_gen,
        git_head,
        host: scope.host.clone(),
        query_digest: infra_core::query::sha256(query.as_bytes()),
        db_mtime_ns,
        db_len: db_meta.len(),
        pending_deltas,
        task_updated_at,
        tree_max_mtime_ns,
        transcript_stamp,
        stdout: Vec::new(),
    })
}

fn preflight_cache_matches(
    cached: &RefreshPreflightRecord,
    current: &RefreshPreflightRecord,
) -> bool {
    cached.task_id == current.task_id
        && cached.index_gen == current.index_gen
        && cached.git_head == current.git_head
        && cached.host == current.host
        && cached.query_digest == current.query_digest
        && cached.db_mtime_ns == current.db_mtime_ns
        && cached.db_len == current.db_len
        && cached.pending_deltas == current.pending_deltas
        && cached.task_updated_at == current.task_updated_at
        && cached.tree_max_mtime_ns == current.tree_max_mtime_ns
        && cached.transcript_stamp == current.transcript_stamp
}

fn load_json_cache<T: for<'de> Deserialize<'de>>(scope: &Context, relative: &str) -> Option<T> {
    let path = scope.roots.state.join(relative);
    let bytes = infra_core::fs::read_regular(&path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn save_json_cache<T: Serialize>(scope: &Context, relative: &str, record: &T) {
    let path = scope.roots.state.join(relative);
    if let Ok(bytes) = serde_json::to_vec(record) {
        let _ = infra_core::fs::atomic_write(&path, &bytes);
    }
}

fn git_head_and_index_mtime(project: &Path) -> (String, i128) {
    if let Ok(Some(frame)) = infra_core::git::GitFrame::discover(project) {
        let idx_mtime = std::fs::symlink_metadata(frame.admin.join("index"))
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(time_to_unix_ns)
            .unwrap_or(0);
        let head_txt = std::fs::read_to_string(frame.admin.join("HEAD")).unwrap_or_default();
        let trimmed = head_txt.trim();
        let resolved = if let Some(ref_name) = trimmed.strip_prefix("ref:") {
            let ref_rel = ref_name.trim();
            frame
                .common_dir()
                .ok()
                .and_then(|common| {
                    std::fs::read_to_string(common.join(ref_rel))
                        .ok()
                        .or_else(|| std::fs::read_to_string(common.join("packed-refs")).ok())
                })
                .unwrap_or_default()
        } else {
            String::new()
        };
        (format!("{trimmed}:{}", resolved.trim()), idx_mtime)
    } else {
        (String::new(), 0)
    }
}

fn transcript_stamp(scope: &Context, require_settled_now: Option<i128>) -> Option<String> {
    let Some(path) = valid_path(scope.payload["transcript_path"].as_str(), 4096) else {
        return Some(String::new());
    };
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return None;
    }
    let mt = meta.modified().ok().and_then(time_to_unix_ns)?;
    if let Some(now_ns) = require_settled_now
        && !settled_mtime(mt, now_ns)
    {
        return None;
    }
    Some(format!("{}:{mt}", meta.len()))
}

fn file_stamp_if_present(path: &Path, now_ns: i128) -> Option<String> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Some("missing".into()),
        Err(_) => return None,
    };
    if meta.file_type().is_symlink() || !meta.is_file() {
        return None;
    }
    let mt = meta.modified().ok().and_then(time_to_unix_ns)?;
    if !settled_mtime(mt, now_ns) {
        return None;
    }
    Some(format!("{}:{mt}", meta.len()))
}

fn load_preflight_cache(scope: &Context) -> Option<RefreshPreflightRecord> {
    load_json_cache(scope, HOOK_PREFLIGHT_CACHE_FILE)
}

fn save_preflight_cache(scope: &Context, mut record: RefreshPreflightRecord, stdout: Vec<u8>) {
    record.stdout = stdout;
    save_json_cache(scope, HOOK_PREFLIGHT_CACHE_FILE, &record);
}
fn task_args(scope: &Context) -> Vec<String> {
    scope
        .task
        .as_ref()
        .map(|task| vec!["--task-id".into(), task.clone()])
        .unwrap_or_default()
}
fn cursor(scope: &Context, args: &[String], input: &[u8]) -> (Vec<u8>, Option<Value>) {
    let Some((operation, args)) = args.split_first() else {
        return (vec![], None);
    };
    let mut args = args.to_vec();
    args.push("--diagnostic".into());
    let output = scope.call(operation, &args, input, Duration::from_secs(3));
    let report = match output {
        Ok(output) if output.code == 0 && output.stdout.len() <= 4096 => {
            serde_json::from_slice::<Value>(&output.stdout).ok()
        }
        _ => None,
    };
    let reason = match report.as_ref().and_then(|report| report["state"].as_str()) {
        Some("written" | "unchanged" | "kept" | "removed" | "absent" | "attached") => None,
        Some("preserved") => Some(
            if report
                .as_ref()
                .is_some_and(|report| report["reason"] == "linked_directory")
            {
                "linked_directory"
            } else {
                "custom_or_unavailable"
            },
        ),
        Some("incomplete") => Some("maintenance_incomplete"),
        _ => Some("maintenance_unavailable"),
    };
    (
        reason
            .map(|reason| format!("Cursor memory rule was not updated: {reason}.\n").into_bytes())
            .unwrap_or_default(),
        report,
    )
}
pub fn read(scope: &Context) -> Result<Output> {
    read_with_runner(scope, now_unix_ns(), true, |ctx, args| {
        ctx.call("refresh", args, &[], ctx.budget)
    })
}

fn read_with_runner<F>(
    scope: &Context,
    now_ns: i128,
    run_native_prompt: bool,
    mut refresh_runner: F,
) -> Result<Output>
where
    F: FnMut(&Context, &[String]) -> Result<Output>,
{
    let cursor_host = scope.host == "cursor";
    let pass = || {
        if cursor_host {
            Output::text("{\"continue\": true}\n")
        } else {
            Output::default()
        }
    };
    if std::env::var("CONTEXT_CAPSULE_DELIVERED").as_deref() == Ok("1") {
        if cursor_host {
            let (stderr, _) = cursor(scope, &["cursor-rule-retire".into()], &[]);
            let mut output = pass();
            output.stderr = stderr;
            return Ok(output);
        }
        return Ok(pass());
    }
    if scope.other_worktree {
        if cursor_host {
            return Ok(pass());
        }
        return Ok(Output::text(
            "Memory not consulted: this session works in another worktree of the repository, and this memory follows the checkout its hooks are installed in.\nWorking memory was NOT consulted this turn. Read the canonical sources directly.\n",
        ));
    }
    let mut query = scope.payload["prompt"].as_str().unwrap_or("").to_string();
    if query.chars().count() > 24000 {
        query = format!(
            "{}\n...\n{}",
            query.chars().take(16000).collect::<String>(),
            query
                .chars()
                .rev()
                .take(8000)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>()
        );
    }
    let native_args = {
        let mut args = vec!["--host".into(), scope.host.clone()];
        args.extend(task_args(scope));
        args
    };
    let mut prompt_notice = String::new();
    let native = if run_native_prompt && scope.task.is_some() {
        if cursor_host {
            if let Ok(output) = scope.call("native-prompt", &native_args, &scope.raw, scope.budget)
                && output.code == 0
                && String::from_utf8_lossy(&output.stdout).tail() == UNSAVED
            {
                prompt_notice = format!("{UNSAVED}\n");
            }
            None
        } else {
            let scope = scope.clone();
            Some(std::thread::spawn(move || {
                scope.call("native-prompt", &native_args, &scope.raw, scope.budget)
            }))
        }
    } else {
        None
    };
    if query.starts_with("INTERNAL_NATIVE_MEMORY_RESULT ") {
        if let Some(native) = native {
            let _ = native.join();
        }
        return Ok(pass());
    }

    let preflight = preflight_key_snapshot(scope, &query, now_ns);
    if let Some(current) = &preflight
        && let Some(cached) = load_preflight_cache(scope)
        && preflight_cache_matches(&cached, current)
    {
        if let Some(native) = native
            && let Ok(Ok(output)) = native.join()
            && output.code == 0
            && String::from_utf8_lossy(&output.stdout).tail() == UNSAVED
        {
            prompt_notice = format!("{UNSAVED}\n");
        }
        scope.append("refresh-health.ndjson",&json!({"at":infra_core::domain::clock::proof_now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),"hook_status":0,"source":"hook-preflight-cache"}));
        if cursor_host {
            return Ok(pass());
        }
        let mut rendered = prompt_notice;
        if !cached.stdout.is_empty() {
            rendered.push_str(&String::from_utf8_lossy(&cached.stdout));
            if !rendered.ends_with('\n') {
                rendered.push('\n');
            }
        }
        return Ok(Output::text(rendered));
    }

    let mut args = vec!["--host".into(), scope.host.clone()];
    if cursor_host {
        args.extend([
            "--delivery-profile".into(),
            "cursor-rule".into(),
            "--json".into(),
        ]);
    }
    if !query.is_empty() && scope.task.is_some() {
        args.extend([
            "--query".into(),
            query,
            "--task-id".into(),
            scope.task.clone().unwrap_or_default(),
            "--ephemeral".into(),
            "--sanitize".into(),
        ]);
        args.extend(scope.branch_args());
        if !cursor_host {
            let session = scope.payload["session_id"]
                .as_str()
                .filter(|id| session_id_re().is_match(id));
            if let Some(session) = session {
                args.extend(["--session-id".into(), session.into()]);
                if let Some(transcript) =
                    valid_path(scope.payload["transcript_path"].as_str(), 4096)
                {
                    args.extend([
                        "--transcript".into(),
                        transcript.to_string_lossy().into_owned(),
                    ]);
                }
            }
        }
    }
    let result = refresh_runner(scope, &args);
    if let Some(native) = native
        && let Ok(Ok(output)) = native.join()
        && output.code == 0
        && String::from_utf8_lossy(&output.stdout).tail() == UNSAVED
    {
        prompt_notice = format!("{UNSAVED}\n");
    }
    let output = match result {
        Ok(output) => output,
        Err(error) => Output::error(1, error.to_string()),
    };
    if output.code == 0
        && let Some(record) = preflight
    {
        save_preflight_cache(scope, record, output.stdout.clone());
    }
    scope.append("refresh-health.ndjson",&json!({"at":infra_core::domain::clock::proof_now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),"hook_status":output.code,"source":"hook"}));
    if cursor_host {
        let mut pass = pass();
        if scope.roots.state == scope.roots.tooling {
            let maintenance = vec![
                "cursor-rule".into(),
                "--action".into(),
                "write".into(),
                "--phase".into(),
                "prompt".into(),
                "--task-id".into(),
                scope.task.clone().unwrap_or_default(),
                "--report-json".into(),
            ];
            let payload = if output.code == 124 {
                &[][..]
            } else {
                output.stdout.as_slice()
            };
            let (stderr, _) = cursor(scope, &maintenance, payload);
            pass.stderr = stderr;
        }
        return Ok(pass);
    }
    let mut rendered = prompt_notice;
    if output.code != 124 && !output.stdout.is_empty() {
        rendered.push_str(&String::from_utf8_lossy(&output.stdout));
        if !rendered.ends_with('\n') {
            rendered.push('\n');
        }
    } else if output.code != 0 {
        if output.code == 124 {
            rendered.push_str(&format!(
                "Memory refresh unavailable: it exceeded its {}s budget.\n",
                scope.budget.as_secs_f64()
            ));
            if scope.roots.project == scope.roots.tooling {
                rendered.push_str("If this repeats on every prompt, build the index once outside the hook: memory-bank/bin/infra-engine memory index\n");
            }
        } else {
            let detail = String::from_utf8_lossy(&output.stderr)
                .lines()
                .rev()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("")
                .replace('\t', " ")
                .chars()
                .take(160)
                .collect::<String>();
            rendered.push_str(&format!(
                "Memory refresh unavailable: it exited {}{}.\n",
                output.code,
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(" — {detail}")
                }
            ));
        }
        rendered.push_str(
            "Working memory was NOT consulted this turn. Read the canonical sources directly.\n",
        );
    }
    Ok(Output::text(rendered))
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct WritePreflightRecord {
    task_id: String,
    host: String,
    entered: bool,
    recovery: bool,
    delivered: bool,
    flush_after: String,
    index_gen: String,
    git_head: String,
    git_dirty_digest: String,
    db_mtime_ns: i128,
    db_len: u64,
    pending_deltas: i64,
    task_updated_at: String,
    tree_max_mtime_ns: i128,
    cursor_rule_stamp: String,
    transcript_stamp: String,
    payload_sha256: String,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn git_dirty_settled_digest(project: &Path, now_ns: i128) -> Option<String> {
    let ok_frame = infra_core::git::GitFrame::discover(project).ok()?;
    let Some(_) = ok_frame else {
        return Some(String::from("no-git"));
    };
    let output = std::process::Command::new("git")
        .args([
            "--no-optional-locks",
            "status",
            "--porcelain=v1",
            "-z",
            "-uall",
        ])
        .current_dir(project)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let mut parts = Vec::<String>::new();
    let entries = output.stdout.split(|b| *b == 0);
    let mut skip_next = false;
    for raw_entry in entries {
        if raw_entry.is_empty() {
            continue;
        }
        if skip_next {
            skip_next = false;
            continue;
        }
        let Ok(entry_str) = std::str::from_utf8(raw_entry) else {
            return None;
        };
        if entry_str.len() < 4 {
            return None;
        }
        let status_code = &entry_str[..2];
        if status_code.starts_with('R') || status_code.starts_with('C') {
            skip_next = true;
        }
        let rel = entry_str[3..].trim_end_matches('/');
        if rel == "memory-bank/local" || rel.starts_with("memory-bank/local/") {
            continue;
        }
        let full = project.join(rel);
        let stamp = match std::fs::symlink_metadata(&full) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return None;
                }
                let mt = meta.modified().ok().and_then(time_to_unix_ns)?;
                if !settled_mtime(mt, now_ns) {
                    return None;
                }
                format!("{status_code}:{rel}:{}:{mt}", meta.len())
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                format!("{status_code}:{rel}:deleted")
            }
            Err(_) => return None,
        };
        parts.push(stamp);
    }
    parts.sort();
    Some(infra_core::query::sha256(parts.join("\n").as_bytes()))
}

fn write_preflight_snapshot(
    scope: &Context,
    recovery: bool,
    delivered: bool,
    flush_after: &str,
    now_ns: i128,
) -> Option<WritePreflightRecord> {
    if std::env::var("HARNESS_DISABLE_HOOK_PREFLIGHT").as_deref() == Ok("1") {
        return None;
    }
    let task_id = scope.task.as_deref().filter(|t| !t.is_empty())?.to_owned();
    let db_path = infra_core::storage::default_database(&scope.roots).ok()?;
    let db_meta = std::fs::symlink_metadata(&db_path).ok()?;
    if db_meta.file_type().is_symlink() || !db_meta.is_file() {
        return None;
    }
    let db_mtime_ns = db_meta.modified().ok().and_then(time_to_unix_ns)?;
    if !settled_mtime(db_mtime_ns, now_ns) {
        return None;
    }
    let mut budget = 1500usize;
    let chunks_mtime = max_dir_mtime_ns(&scope.roots.state.join("memory-bank/chunks"), &mut budget);
    let brain_mtime = max_dir_mtime_ns(&scope.roots.brain_root(), &mut budget);
    let (git_head, git_idx_mtime) = git_head_and_index_mtime(&scope.roots.project);
    let tree_max_mtime_ns = chunks_mtime
        .max(brain_mtime)
        .max(db_mtime_ns)
        .max(git_idx_mtime);
    if tree_max_mtime_ns == i128::MAX || !settled_mtime(tree_max_mtime_ns, now_ns) {
        return None;
    }
    let git_dirty_digest = git_dirty_settled_digest(&scope.roots.project, now_ns)?;
    let transcript_stamp = transcript_stamp(scope, Some(now_ns))?;
    let cursor_rule_stamp = if scope.host == "cursor" && scope.roots.state == scope.roots.tooling {
        file_stamp_if_present(
            &scope.roots.tooling.join(".cursor/rules/working-memory.mdc"),
            now_ns,
        )?
    } else {
        "n/a".into()
    };

    let store = Store::open_read_only(&db_path, Duration::from_millis(250))
        .or_else(|_| Store::open(&db_path, OpenOptions::default()))
        .ok()?;
    let index_gen = infra_core::storage::index_generation(&store.conn).ok()?;
    let pending_deltas: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM turn_deltas WHERE task_id=?1",
            [&task_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let task_updated_at: String = store
        .conn
        .query_row(
            "SELECT COALESCE(MAX(updated_at), '') FROM working_tasks WHERE task_id=?1",
            [&task_id],
            |r| r.get(0),
        )
        .unwrap_or_default();

    Some(WritePreflightRecord {
        task_id,
        host: scope.host.clone(),
        entered: scope.entered,
        recovery,
        delivered,
        flush_after: flush_after.to_string(),
        index_gen,
        git_head,
        git_dirty_digest,
        db_mtime_ns,
        db_len: db_meta.len(),
        pending_deltas,
        task_updated_at,
        tree_max_mtime_ns,
        cursor_rule_stamp,
        transcript_stamp,
        payload_sha256: infra_core::query::sha256(&scope.raw),
        stdout: Vec::new(),
        stderr: Vec::new(),
    })
}

fn write_preflight_matches(cached: &WritePreflightRecord, current: &WritePreflightRecord) -> bool {
    cached.task_id == current.task_id
        && cached.host == current.host
        && cached.entered == current.entered
        && cached.recovery == current.recovery
        && cached.delivered == current.delivered
        && cached.flush_after == current.flush_after
        && cached.index_gen == current.index_gen
        && cached.git_head == current.git_head
        && cached.git_dirty_digest == current.git_dirty_digest
        && cached.db_mtime_ns == current.db_mtime_ns
        && cached.db_len == current.db_len
        && cached.pending_deltas == current.pending_deltas
        && cached.task_updated_at == current.task_updated_at
        && cached.tree_max_mtime_ns == current.tree_max_mtime_ns
        && cached.cursor_rule_stamp == current.cursor_rule_stamp
        && cached.transcript_stamp == current.transcript_stamp
        && cached.payload_sha256 == current.payload_sha256
}

pub fn write(scope: &Context) -> Result<Output> {
    write_with_runner(scope, now_unix_ns(), |ctx, operation, args, input| {
        ctx.call(operation, args, input, ctx.budget)
    })
}

fn write_with_runner<F>(scope: &Context, now_ns: i128, mut call_runner: F) -> Result<Output>
where
    F: FnMut(&Context, &str, &[String], &[u8]) -> Result<Output>,
{
    if scope.other_worktree || scope.task.is_none() {
        return Ok(Output::default());
    }
    let recovery = if scope.host == "cursor" {
        scope.payload["loop_count"]
            .as_i64()
            .is_some_and(|count| count > 0)
    } else {
        scope.payload["stop_hook_active"] == true
    };
    let flush = std::env::var("CONTEXT_FLUSH_AFTER").unwrap_or_else(|_| "5".into());
    let delivered = std::env::var("CONTEXT_CAPSULE_DELIVERED").as_deref() == Ok("1");

    let preflight = write_preflight_snapshot(scope, recovery, delivered, &flush, now_ns);
    if let Some(current) = &preflight
        && let Some(cached) =
            load_json_cache::<WritePreflightRecord>(scope, HOOK_WRITE_PREFLIGHT_CACHE_FILE)
        && write_preflight_matches(&cached, current)
    {
        scope.append("native-results.ndjson",&json!({"at":infra_core::domain::clock::proof_now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),"host":scope.host,"stage":"stop-hook","event":"preflight-cache","status":0}));
        return Ok(Output {
            code: 0,
            stdout: cached.stdout,
            stderr: cached.stderr,
            ..Default::default()
        });
    }

    let mut all_ok = true;
    if !recovery {
        let mut args = task_args(scope);
        args.extend(["--flush-after".into(), flush]);
        if let Some(transcript) = valid_path(scope.payload["transcript_path"].as_str(), 4096)
            .filter(|path| path.is_file())
        {
            args.extend([
                "--transcript".into(),
                transcript.to_string_lossy().into_owned(),
            ]);
        }
        if !call_runner(scope, "turn", &args, &[]).is_ok_and(|output| output.code == 0) {
            all_ok = false;
        }
    }
    let mut response = Output::default();
    if !delivered {
        let mut args = vec!["--host".into(), scope.host.clone()];
        args.extend(task_args(scope));
        if scope.entered {
            args.push("--entered".into());
        }
        let native = call_runner(scope, "native-stop", &args, &scope.raw);
        let output = match native {
            Ok(output) => output,
            Err(_) => Output {
                code: 1,
                ..Default::default()
            },
        };
        if output.code == 0 {
            response.stdout = output.stdout;
        } else {
            all_ok = false;
        }
        scope.append("native-results.ndjson",&json!({"at":infra_core::domain::clock::proof_now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),"host":scope.host,"stage":"stop-hook","event":"exited","status":output.code}));
    } else {
        scope.append("native-results.ndjson",&json!({"at":infra_core::domain::clock::proof_now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),"host":scope.host,"stage":"stop-hook","event":"skipped","code":"capsule-delivered"}));
    }
    if scope.host != "cursor" || scope.roots.state != scope.roots.tooling {
        if all_ok && let Some(mut record) = preflight {
            record.stdout = response.stdout.clone();
            record.stderr = response.stderr.clone();
            save_json_cache(scope, HOOK_WRITE_PREFLIGHT_CACHE_FILE, &record);
        }
        return Ok(response);
    }
    if delivered {
        let (stderr, _) = cursor(scope, &["cursor-rule-retire".into()], &[]);
        response.stderr.extend(stderr);
        if all_ok
            && response.stderr.is_empty()
            && let Some(mut record) = preflight
        {
            record.stdout = response.stdout.clone();
            record.stderr = response.stderr.clone();
            save_json_cache(scope, HOOK_WRITE_PREFLIGHT_CACHE_FILE, &record);
        }
        return Ok(response);
    }
    let mut held = vec![
        "cursor-rule".into(),
        "--action".into(),
        "prompt-held".into(),
    ];
    held.extend(task_args(scope));
    let (_, report) = cursor(scope, &held, &[]);
    if report.is_some_and(|report| report["state"] == "kept") {
        if all_ok && let Some(mut record) = preflight {
            record.stdout = response.stdout.clone();
            record.stderr = response.stderr.clone();
            save_json_cache(scope, HOOK_WRITE_PREFLIGHT_CACHE_FILE, &record);
        }
        return Ok(response);
    }
    let mut arguments = vec![
        "--host".into(),
        "cursor".into(),
        "--delivery-profile".into(),
        "cursor-rule".into(),
    ];
    arguments.extend(task_args(scope));
    arguments.extend(scope.branch_args());
    let capsule = call_runner(scope, "hook-context", &arguments, &[]);
    let mut maintenance = vec![];
    let input = match &capsule {
        Ok(output) if output.code == 3 => {
            maintenance.push("cursor-rule-retire".into());
            &[][..]
        }
        Ok(output) if output.code == 0 => {
            maintenance.extend([
                "cursor-rule".into(),
                "--action".into(),
                "write".into(),
                "--phase".into(),
                "turn".into(),
            ]);
            maintenance.extend(task_args(scope));
            output.stdout.as_slice()
        }
        _ => {
            all_ok = false;
            maintenance.extend(["cursor-rule".into(), "--action".into(), "keep".into()]);
            maintenance.extend(task_args(scope));
            &[][..]
        }
    };
    let (stderr, _) = cursor(scope, &maintenance, input);
    response.stderr.extend(stderr);
    if all_ok
        && response.stderr.is_empty()
        && let Some(mut record) = preflight
    {
        record.stdout = response.stdout.clone();
        record.stderr = response.stderr.clone();
        save_json_cache(scope, HOOK_WRITE_PREFLIGHT_CACHE_FILE, &record);
    }
    Ok(response)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ContinuityPreflightRecord {
    host: String,
    owner: String,
    env_digest: String,
    git_head: String,
    config_stamp: String,
    handoff_tree_digest: String,
    handoff_max_mtime_ns: i128,
    transcript_stamp: String,
    payload_sha256: String,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn handoff_settled_fingerprint(handoff: &Path, now_ns: i128) -> Option<(i128, String)> {
    let meta = std::fs::symlink_metadata(handoff).ok()?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return None;
    }
    let mut max_mtime = meta.modified().ok().and_then(time_to_unix_ns)?;
    let mut files = Vec::<String>::new();
    let mut stack = vec![handoff.to_path_buf()];
    let mut remaining = 512usize;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return None;
        };
        for entry in entries.flatten() {
            if remaining == 0 {
                return None;
            }
            remaining -= 1;
            let path = entry.path();
            let Ok(m) = std::fs::symlink_metadata(&path) else {
                return None;
            };
            if m.file_type().is_symlink() {
                return None;
            }
            let mt = m.modified().ok().and_then(time_to_unix_ns)?;
            max_mtime = max_mtime.max(mt);
            let rel = path
                .strip_prefix(handoff)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if m.is_dir() {
                files.push(format!("D:{rel}:{mt}"));
                stack.push(path);
            } else {
                files.push(format!("F:{rel}:{}:{mt}", m.len()));
            }
        }
    }
    if !settled_mtime(max_mtime, now_ns) {
        return None;
    }
    files.sort();
    Some((
        max_mtime,
        infra_core::query::sha256(files.join("\n").as_bytes()),
    ))
}

fn continuity_preflight_snapshot(
    scope: &Context,
    now_ns: i128,
) -> Option<ContinuityPreflightRecord> {
    if std::env::var("HARNESS_DISABLE_HOOK_PREFLIGHT").as_deref() == Ok("1") {
        return None;
    }
    let handoff = scope.roots.state.join(".context-handoff");
    let (handoff_max_mtime_ns, handoff_tree_digest) =
        handoff_settled_fingerprint(&handoff, now_ns)?;
    let (git_head, git_idx_mtime) = git_head_and_index_mtime(&scope.roots.project);
    if git_idx_mtime > 0 && !settled_mtime(git_idx_mtime, now_ns) {
        return None;
    }
    let config_stamp = file_stamp_if_present(
        &scope.roots.brain_root().join("config/runtime.json"),
        now_ns,
    )?;
    let transcript_stamp = transcript_stamp(scope, Some(now_ns))?;
    let owner = std::env::var("PROJECT_BRAIN_OWNER").unwrap_or_else(|_| "local".into());
    let env_digest = infra_core::query::sha256(
        format!(
            "{}:{}:{}",
            std::env::var("CONTEXT_CONTINUITY_DISABLED").unwrap_or_default(),
            std::env::var("CONTEXT_CONTINUITY_CAPTURE_DISABLED").unwrap_or_default(),
            std::env::var("CONTEXT_CONTINUITY_RESTORE_DISABLED").unwrap_or_default(),
        )
        .as_bytes(),
    );

    Some(ContinuityPreflightRecord {
        host: scope.host.clone(),
        owner,
        env_digest,
        git_head,
        config_stamp,
        handoff_tree_digest,
        handoff_max_mtime_ns,
        transcript_stamp,
        payload_sha256: infra_core::query::sha256(&scope.raw),
        stdout: Vec::new(),
        stderr: Vec::new(),
    })
}

fn continuity_preflight_matches(
    cached: &ContinuityPreflightRecord,
    current: &ContinuityPreflightRecord,
) -> bool {
    cached.host == current.host
        && cached.owner == current.owner
        && cached.env_digest == current.env_digest
        && cached.git_head == current.git_head
        && cached.config_stamp == current.config_stamp
        && cached.handoff_tree_digest == current.handoff_tree_digest
        && cached.handoff_max_mtime_ns == current.handoff_max_mtime_ns
        && cached.transcript_stamp == current.transcript_stamp
        && cached.payload_sha256 == current.payload_sha256
}

pub fn continuity(context: &Context) -> Result<Output> {
    continuity_with_runner(context, now_unix_ns(), |ctx| {
        let value = infra_core::chat_continuity::execute(
            &infra_core::RuntimeContext::new(
                ctx.roots.clone(),
                std::env::var("PROJECT_BRAIN_OWNER").unwrap_or_else(|_| "local".into()),
                infra_core::RuntimeConfig::load(&ctx.roots)?.mode(None)?,
            )?,
            &ctx.host,
            "hook",
            &ctx.raw,
            &json!({}),
        )?;
        Output::envelope(value)
    })
}

fn continuity_with_runner<F>(context: &Context, now_ns: i128, mut runner: F) -> Result<Output>
where
    F: FnMut(&Context) -> Result<Output>,
{
    if context.other_worktree {
        return Ok(Output::default());
    }
    let preflight = continuity_preflight_snapshot(context, now_ns);
    if let Some(current) = &preflight
        && let Some(cached) = load_json_cache::<ContinuityPreflightRecord>(
            context,
            HOOK_CONTINUITY_PREFLIGHT_CACHE_FILE,
        )
        && continuity_preflight_matches(&cached, current)
    {
        return Ok(Output {
            code: 0,
            stdout: cached.stdout,
            stderr: cached.stderr,
            ..Default::default()
        });
    }
    let output = runner(context)?;
    if output.code == 0
        && let Some(mut record) = preflight
    {
        record.stdout = output.stdout.clone();
        record.stderr = output.stderr.clone();
        save_json_cache(context, HOOK_CONTINUITY_PREFLIGHT_CACHE_FILE, &record);
    }
    Ok(output)
}
pub fn session_cursor(scope: &Context) -> Result<Vec<u8>> {
    if scope.host != "cursor" || scope.roots.state != scope.roots.tooling {
        return Ok(vec![]);
    }
    if std::env::var("CONTEXT_CAPSULE_DELIVERED").as_deref() == Ok("1") {
        return Ok(cursor(scope, &["cursor-rule-retire".into()], &[]).0);
    }
    let mut arguments = vec![
        "--host".into(),
        "cursor".into(),
        "--delivery-profile".into(),
        "cursor-rule".into(),
        "--task-id".into(),
        scope.task.clone().unwrap_or_default(),
    ];
    arguments.extend(scope.branch_args());
    let capsule = scope.call("hook-context", &arguments, &[], scope.budget);
    let mut maintenance = vec![];
    let input = match &capsule {
        Ok(output) if output.code == 3 => {
            maintenance.push("cursor-rule-retire".into());
            &[][..]
        }
        Ok(output) if output.code == 0 => {
            maintenance.extend([
                "cursor-rule".into(),
                "--action".into(),
                "write".into(),
                "--phase".into(),
                "session".into(),
                "--task-id".into(),
                scope.task.clone().unwrap_or_default(),
            ]);
            output.stdout.as_slice()
        }
        _ => {
            maintenance.extend([
                "cursor-rule".into(),
                "--action".into(),
                "keep".into(),
                "--task-id".into(),
                scope.task.clone().unwrap_or_default(),
            ]);
            &[][..]
        }
    };
    Ok(cursor(scope, &maintenance, input).0)
}

#[cfg(test)]
mod hook_preflight_tests {
    use super::*;
    use infra_core::Roots;

    fn test_git_project() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = infra_core::fs::python_realpath(dir.path()).unwrap();
        let init = std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .status()
            .unwrap();
        assert!(init.success());
        std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(&root)
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&root)
            .status()
            .unwrap();
        std::fs::write(root.join("README.md"), "init\n").unwrap();
        std::process::Command::new("git")
            .args(["add", "README.md"])
            .current_dir(&root)
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "--quiet", "-m", "init"])
            .current_dir(&root)
            .status()
            .unwrap();
        std::fs::create_dir_all(root.join("memory-bank/chunks")).unwrap();
        let db_path = root.join("memory-bank/local/context.db");
        let _store = Store::open(&db_path, OpenOptions::refresh(None)).unwrap();
        (dir, root, db_path)
    }

    #[test]
    fn memory_read_skips_heavy_refresh_when_task_index_and_head_settled_and_invalidates_on_drift() {
        let (_dir, root, db_path) = test_git_project();
        let roots = Roots::discover(&root, &root).unwrap();
        let payload = json!({"prompt": "How do invoices number per tenant?"});
        let scope = Context {
            roots,
            host: "claude".into(),
            payload: payload.clone(),
            raw: serde_json::to_vec(&payload).unwrap(),
            task: Some("TASK-42".into()),
            inferred: false,
            entered: false,
            other_worktree: false,
            outside: None,
            budget: Duration::from_secs(3),
        };
        let prompt_txt = scope.payload["prompt"].as_str().unwrap();

        let db_mtime = std::fs::symlink_metadata(&db_path)
            .unwrap()
            .modified()
            .ok()
            .and_then(time_to_unix_ns)
            .unwrap();
        let racy_now = db_mtime + HOOK_PREFLIGHT_RACY_NS - 1;
        assert!(preflight_key_snapshot(&scope, prompt_txt, racy_now).is_none());

        let settled_now = db_mtime + HOOK_PREFLIGHT_RACY_NS + 500_000_000;
        let mut calls = 0usize;
        let first = read_with_runner(&scope, settled_now, false, |_s, _args| {
            calls += 1;
            Ok(Output::text("CAPSULE v1\n"))
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(String::from_utf8_lossy(&first.stdout), "CAPSULE v1\n");

        // Second turn with unchanged (task_id, index_gen, git_head) outside RACY_NS skips refresh!
        let second = read_with_runner(&scope, settled_now + 100, false, |_s, _args| {
            calls += 1;
            Ok(Output::text("SHOULD_NOT_RUN\n"))
        })
        .unwrap();
        assert_eq!(
            calls, 1,
            "unchanged settled turn must skip refresh subprocess"
        );
        assert_eq!(String::from_utf8_lossy(&second.stdout), "CAPSULE v1\n");

        // Advancing git HEAD invalidates preflight cleanly and runs refresh again.
        std::fs::write(root.join("README.md"), "v2\n").unwrap();
        std::process::Command::new("git")
            .args(["add", "README.md"])
            .current_dir(&root)
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "--quiet", "-m", "v2"])
            .current_dir(&root)
            .status()
            .unwrap();
        let after_git = now_unix_ns() + HOOK_PREFLIGHT_RACY_NS + 1_000_000_000;
        let third = read_with_runner(&scope, after_git, false, |_s, _args| {
            calls += 1;
            Ok(Output::text("CAPSULE v2\n"))
        })
        .unwrap();
        assert_eq!(calls, 2, "changed git_head must trigger fresh refresh");
        assert_eq!(String::from_utf8_lossy(&third.stdout), "CAPSULE v2\n");
    }

    #[test]
    fn memory_write_preflight_respects_racy_window_settled_cache_and_git_dirty_drift() {
        let (_dir, root, db_path) = test_git_project();
        let roots = Roots::discover(&root, &root).unwrap();
        let payload = json!({"session_id": "write-s1", "stop_hook_active": false});
        let scope = Context {
            roots,
            host: "claude".into(),
            payload: payload.clone(),
            raw: serde_json::to_vec(&payload).unwrap(),
            task: Some("TASK-42".into()),
            inferred: false,
            entered: false,
            other_worktree: false,
            outside: None,
            budget: Duration::from_secs(3),
        };

        assert!(session_id_re().is_match("sess_1.abc:def-9"));
        assert!(!session_id_re().is_match("bad/id"));

        let db_mtime = std::fs::symlink_metadata(&db_path)
            .unwrap()
            .modified()
            .ok()
            .and_then(time_to_unix_ns)
            .unwrap();
        assert!(
            write_preflight_snapshot(
                &scope,
                false,
                false,
                "5",
                db_mtime + HOOK_PREFLIGHT_RACY_NS - 1
            )
            .is_none()
        );

        let settled = db_mtime + HOOK_PREFLIGHT_RACY_NS + 500_000_000;
        let snap1 = write_preflight_snapshot(&scope, false, false, "5", settled)
            .expect("settled write preflight snapshot must build");
        let mut calls = 0usize;
        let first = write_with_runner(&scope, settled, |_s, op, _args, _in| {
            calls += 1;
            if op == "native-stop" {
                Ok(Output::text("STOP CHECK OK\n"))
            } else {
                Ok(Output::default())
            }
        })
        .unwrap();
        assert_eq!(calls, 2); // turn + native-stop
        assert_eq!(String::from_utf8_lossy(&first.stdout), "STOP CHECK OK\n");
        let cached =
            load_json_cache::<WritePreflightRecord>(&scope, HOOK_WRITE_PREFLIGHT_CACHE_FILE)
                .expect("write preflight cache file must exist");
        let snap2 = write_preflight_snapshot(&scope, false, false, "5", settled + 1000)
            .expect("second settled write preflight snapshot must build");
        assert_eq!(snap1, snap2);
        assert!(write_preflight_matches(&cached, &snap2));

        // Second call with unchanged workspace & payload skips spawning both `turn` and `native-stop`.
        let second = write_with_runner(&scope, settled + 1000, |_s, _op, _args, _in| {
            calls += 1;
            Ok(Output::text("UNEXPECTED\n"))
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(String::from_utf8_lossy(&second.stdout), "STOP CHECK OK\n");

        // Modifying a working-tree file outside racy window invalidates preflight cache.
        std::fs::write(root.join("README.md"), "dirty edit\n").unwrap();
        let file_mt = std::fs::symlink_metadata(root.join("README.md"))
            .unwrap()
            .modified()
            .ok()
            .and_then(time_to_unix_ns)
            .unwrap();
        assert!(
            write_preflight_snapshot(
                &scope,
                false,
                false,
                "5",
                file_mt + HOOK_PREFLIGHT_RACY_NS - 1
            )
            .is_none(),
            "dirty file inside 2s racy window must bypass cache"
        );
        let third = write_with_runner(
            &scope,
            file_mt + HOOK_PREFLIGHT_RACY_NS + 500_000_000,
            |_s, op, _args, _in| {
                calls += 1;
                if op == "native-stop" {
                    Ok(Output::text("STOP CHECK v2\n"))
                } else {
                    Ok(Output::default())
                }
            },
        )
        .unwrap();
        assert_eq!(calls, 4);
        assert_eq!(String::from_utf8_lossy(&third.stdout), "STOP CHECK v2\n");
    }

    #[test]
    fn continuity_preflight_caches_settled_handoff_without_touching_sqlite() {
        let dir = tempfile::tempdir().unwrap();
        let root = infra_core::fs::python_realpath(dir.path()).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::create_dir_all(root.join(".git/refs/heads")).unwrap();
        std::fs::write(
            root.join(".git/refs/heads/main"),
            format!("{}\n", "a".repeat(40)),
        )
        .unwrap();
        let handoff = root.join(".context-handoff");
        std::fs::create_dir_all(&handoff).unwrap();
        let snap = handoff.join("snap.json");
        std::fs::write(&snap, "{\"ok\":true}\n").unwrap();

        let roots = Roots::discover(&root, &root).unwrap();
        let payload = json!({"session_id": "cont-1", "hook_event_name": "SessionStart"});
        let scope = Context {
            roots,
            host: "claude".into(),
            payload: payload.clone(),
            raw: serde_json::to_vec(&payload).unwrap(),
            task: None,
            inferred: false,
            entered: false,
            other_worktree: false,
            outside: None,
            budget: Duration::from_secs(3),
        };

        let snap_mt = std::fs::symlink_metadata(&snap)
            .unwrap()
            .modified()
            .ok()
            .and_then(time_to_unix_ns)
            .unwrap();
        assert!(
            continuity_preflight_snapshot(&scope, snap_mt + HOOK_PREFLIGHT_RACY_NS - 1).is_none()
        );

        let settled = snap_mt + HOOK_PREFLIGHT_RACY_NS + 500_000_000;
        let mut calls = 0usize;
        let first = continuity_with_runner(&scope, settled, |_ctx| {
            calls += 1;
            Ok(Output::text("RESTORED v1\n"))
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(String::from_utf8_lossy(&first.stdout), "RESTORED v1\n");
        assert!(
            !root.join("memory-bank/local/context.db").exists(),
            "continuity preflight must never create SQLite"
        );

        let second = continuity_with_runner(&scope, settled + 100, |_ctx| {
            calls += 1;
            Ok(Output::text("SHOULD_NOT_RUN\n"))
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(String::from_utf8_lossy(&second.stdout), "RESTORED v1\n");

        // Touching handoff state inside racy window and beyond invalidates cache cleanly.
        std::fs::write(&snap, "{\"ok\":true,\"n\":2}\n").unwrap();
        let new_mt = std::fs::symlink_metadata(&snap)
            .unwrap()
            .modified()
            .ok()
            .and_then(time_to_unix_ns)
            .unwrap();
        let third = continuity_with_runner(
            &scope,
            new_mt + HOOK_PREFLIGHT_RACY_NS + 500_000_000,
            |_ctx| {
                calls += 1;
                Ok(Output::text("RESTORED v2\n"))
            },
        )
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(String::from_utf8_lossy(&third.stdout), "RESTORED v2\n");
    }
}
