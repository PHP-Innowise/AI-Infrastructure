use super::{Context, valid_path};
use crate::TailTrim;
use crate::output::Output;
use infra_core::Result;
use serde_json::{Value, json};
use std::time::Duration;
const UNSAVED: &str = "Previous native memory result was not saved completely.";
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
    let native = if scope.task.is_some() {
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
    let result = scope.call("refresh", &args, &[], scope.budget);
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
