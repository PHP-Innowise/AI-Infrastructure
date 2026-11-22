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
    let git_head =
        if let Ok(Some(frame)) = infra_core::git::GitFrame::discover(&scope.roots.project) {
            if let Some(git_idx_mtime) = std::fs::symlink_metadata(frame.admin.join("index"))
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(time_to_unix_ns)
            {
                tree_max_mtime_ns = tree_max_mtime_ns.max(git_idx_mtime);
            }
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
            format!("{trimmed}:{}", resolved.trim())
        } else {
            String::new()
        };
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

    let transcript_stamp = valid_path(scope.payload["transcript_path"].as_str(), 4096)
        .and_then(|p| std::fs::symlink_metadata(p).ok())
        .and_then(|m| {
            let mt = m.modified().ok().and_then(time_to_unix_ns)?;
            Some(format!("{}:{mt}", m.len()))
        })
        .unwrap_or_default();

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

fn load_preflight_cache(scope: &Context) -> Option<RefreshPreflightRecord> {
    let path = scope.roots.state.join(HOOK_PREFLIGHT_CACHE_FILE);
    let bytes = infra_core::fs::read_regular(&path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn save_preflight_cache(scope: &Context, mut record: RefreshPreflightRecord, stdout: Vec<u8>) {
    record.stdout = stdout;
    let path = scope.roots.state.join(HOOK_PREFLIGHT_CACHE_FILE);
    if let Ok(bytes) = serde_json::to_vec(&record) {
        let _ = infra_core::fs::atomic_write(&path, &bytes);
    }
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
            let session = scope.payload["session_id"].as_str().filter(|id| {
                regex::Regex::new(r"^[A-Za-z0-9._:-]{1,128}$")
                    .is_ok_and(|pattern| pattern.is_match(id))
            });
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
pub fn write(scope: &Context) -> Result<Output> {
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
    if !recovery {
        let flush = std::env::var("CONTEXT_FLUSH_AFTER").unwrap_or_else(|_| "5".into());
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
        let _ = scope.call("turn", &args, &[], scope.budget);
    }
    let delivered = std::env::var("CONTEXT_CAPSULE_DELIVERED").as_deref() == Ok("1");
    let mut response = Output::default();
    if !delivered {
        let mut args = vec!["--host".into(), scope.host.clone()];
        args.extend(task_args(scope));
        if scope.entered {
            args.push("--entered".into());
        }
        let native = scope.call("native-stop", &args, &scope.raw, scope.budget);
        let output = match native {
            Ok(output) => output,
            Err(_) => Output {
                code: 1,
                ..Default::default()
            },
        };
        if output.code == 0 {
            response.stdout = output.stdout;
        }
        scope.append("native-results.ndjson",&json!({"at":infra_core::domain::clock::proof_now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),"host":scope.host,"stage":"stop-hook","event":"exited","status":output.code}));
    } else {
        scope.append("native-results.ndjson",&json!({"at":infra_core::domain::clock::proof_now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),"host":scope.host,"stage":"stop-hook","event":"skipped","code":"capsule-delivered"}));
    }
    if scope.host != "cursor" || scope.roots.state != scope.roots.tooling {
        return Ok(response);
    }
    if delivered {
        let (stderr, _) = cursor(scope, &["cursor-rule-retire".into()], &[]);
        response.stderr.extend(stderr);
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
                "turn".into(),
            ]);
            maintenance.extend(task_args(scope));
            output.stdout.as_slice()
        }
        _ => {
            maintenance.extend(["cursor-rule".into(), "--action".into(), "keep".into()]);
            maintenance.extend(task_args(scope));
            &[][..]
        }
    };
    let (stderr, _) = cursor(scope, &maintenance, input);
    response.stderr.extend(stderr);
    Ok(response)
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
}
