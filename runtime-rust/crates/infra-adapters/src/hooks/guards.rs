use super::{Context, first, project_root, strings};
use crate::TailTrim;
use crate::{counters, output::Output, shell};
use infra_core::{Result, Roots, RuntimeConfig, RuntimeError};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
const EDIT_TOOLS: &[&str] = &[
    "write",
    "edit",
    "multiedit",
    "multi_edit",
    "notebookedit",
    "notebook_edit",
    "apply_patch",
];
fn allowed_tool(payload: &Value, names: &[&str]) -> bool {
    let tools = strings(payload, &["tool_name"]);
    tools.is_empty()
        || tools.iter().any(|tool| {
            let name = tool
                .rsplit(['.', ':', '/'])
                .next()
                .unwrap_or(tool)
                .to_ascii_lowercase();
            names.contains(&name.as_str())
        })
}
pub(super) fn payload_noop(role: &str, payload: &Value) -> bool {
    match role {
        // These borrowed searches have the same existence test as strings(),
        // without copying a possibly large command before shell validation.
        "bash-validator" => first(payload, "command").is_none() && first(payload, "cmd").is_none(),
        "file-naming-validator" | "loop-detection" => !allowed_tool(payload, EDIT_TOOLS),
        _ => false,
    }
}
fn context(event: &str, message: &str) -> Result<Output> {
    Output::json(&json!({"hookSpecificOutput":{"hookEventName":event,"additionalContext":message}}))
}
/// The framework the runtime configuration names. An unreadable
/// configuration names none, and the generic rules then apply.
fn framework(roots: &Roots) -> Option<String> {
    RuntimeConfig::load(roots)
        .ok()
        .map(|config| config.value["framework"].as_str().unwrap_or("core").into())
}
/// Where a guard sends a repeated failure. Codex has no slash commands, so
/// its messages name the skill instead.
fn debugger(host: &str) -> &'static str {
    if host == "codex" {
        "systematic-debugger"
    } else {
        "/debugger"
    }
}
const INVALID_POLICY: &str = "BLOCKED: native hook policy data or custody is invalid. Restore the approved generated policy before running shell commands.";
/// The shell rules for a root; `None` when its generated policy is invalid.
fn shell_policy(tooling: &Path, framework: Option<&str>) -> Option<shell::Policy> {
    match crate::hook_policy::load(tooling, framework) {
        Ok(Some(policy)) => Some(shell::Policy::from_hook_policy(policy)),
        Ok(None) => Some(shell::Policy::for_framework(framework.unwrap_or("core"))),
        Err(_) => None,
    }
}
fn refusal(command: &str, policy: &shell::Policy) -> Option<Output> {
    let category = match shell::validate(command, policy) {
        Ok(category) => category,
        Err(error) => Some(error.to_string()),
    };
    category.map(|category| {
        Output::error(
            2,
            format!(
                "BLOCKED: Command refused (rule category: {category}).\n   This operation is blocked. See AGENTS.md."
            ),
        )
    })
}
/// Checks the command when the hook context itself cannot be built (Git
/// metadata, a state directory or the root that cannot be read). It uses the
/// bound root's policy when that root still exists and the generic rules
/// otherwise; the repetition counters are skipped.
pub fn bash_without_context(root: &Path, raw: &[u8]) -> Result<Output> {
    let payload = serde_json::from_slice::<Value>(raw)
        .ok()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let command = strings(&payload, &["command", "cmd"]).join("\n");
    if command.is_empty() {
        return Ok(Output::default());
    }
    let policy = if root.is_dir() {
        let framework = Roots::installed(root)
            .ok()
            .and_then(|roots| framework(&roots));
        let Some(policy) = shell_policy(root, framework.as_deref()) else {
            return Ok(Output::error(2, INVALID_POLICY));
        };
        policy
    } else {
        shell::Policy::for_framework("core")
    };
    Ok(refusal(&command, &policy).unwrap_or_default())
}
/// Normalizes a shell command into a canonical signature so trivial output paging/redirection
/// tweaks (`2>&1`, `| head -n 20`, extra whitespace) still register as repeated retries.
pub(crate) fn normalize_command_signature(command: &str) -> String {
    let mut trimmed = command.trim();
    loop {
        let next = trimmed.trim_end();
        let stripped = if let Some(idx) = next.rfind('|') {
            let tail = next[idx + 1..].trim_start();
            let head_word = tail.split_whitespace().next().unwrap_or("");
            if [
                "head", "tail", "cat", "less", "more", "wc", "grep", "rg", "tee",
            ]
            .contains(&head_word)
            {
                &next[..idx]
            } else {
                next
            }
        } else {
            next
        };
        let without_redir = stripped
            .trim_end()
            .strip_suffix("2>&1")
            .or_else(|| stripped.trim_end().strip_suffix("1>/dev/null"))
            .or_else(|| stripped.trim_end().strip_suffix(">/dev/null"))
            .map(str::trim_end);
        match without_redir {
            Some(shorter) if shorter.len() < next.len() => trimmed = shorter,
            _ if stripped.len() < next.len() => trimmed = stripped,
            _ => break,
        }
    }
    trimmed.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalized_error_signature(payload: &Value) -> String {
    for key in ["error", "stderr", "last_error", "tool_error"] {
        if let Some(raw) = first(payload, key) {
            let sig: String = raw
                .split_whitespace()
                .filter(|w| !w.chars().all(|c| c.is_ascii_digit()))
                .take(12)
                .collect::<Vec<_>>()
                .join(" ");
            if !sig.is_empty() {
                return sig;
            }
        }
    }
    String::new()
}

fn diagnostic_hint(normalized: &str, err_sig: &str, debugger_name: &str) -> String {
    let lower = normalized.to_ascii_lowercase();
    let base = if lower.contains("cargo test")
        || lower.contains("cargo check")
        || lower.contains("cargo clippy")
    {
        "Hint: isolate one failing test with `-- --nocapture` or inspect the first compiler diagnostic instead of rerunning the full workspace."
    } else if lower.contains("phpunit")
        || lower.contains("pest")
        || lower.contains("phpstan")
        || lower.contains("composer")
    {
        "Hint: narrow to the failing test/file (`--filter <Name>` or single path) and verify dependencies/config before retrying."
    } else if lower.contains("pytest") || lower.contains("python") {
        "Hint: isolate the failing test with `-k <name> --tb=short` to inspect the underlying assertion or traceback."
    } else if lower.contains("npm ")
        || lower.contains("pnpm ")
        || lower.contains("yarn ")
        || lower.contains("vitest")
        || lower.contains("jest")
        || lower.contains("tsc")
    {
        "Hint: run a single test file or standalone typecheck to read the first failing diagnostic cleanly."
    } else if lower.starts_with("git ") {
        "Hint: inspect `git status` and `git diff --stat` to resolve conflicts or hooks before retrying."
    } else {
        "Hint: narrow the scope, read full stderr without truncation, or check recent edits before retrying."
    };
    if err_sig.is_empty() {
        format!("{base} Escalate to {debugger_name} if the failure repeats.")
    } else {
        format!(
            "{base} Repeated error signature: `{}`. Escalate to {debugger_name} if unresolved.",
            err_sig.chars().take(80).collect::<String>()
        )
    }
}

pub fn bash(scope: &Context) -> Result<Output> {
    let command = strings(&scope.payload, &["command", "cmd"]).join("\n");
    if command.is_empty() {
        return Ok(Output::default());
    }
    let framework = framework(&scope.roots);
    let Some(policy) = shell_policy(&scope.roots.tooling, framework.as_deref()) else {
        return Ok(Output::error(2, INVALID_POLICY));
    };
    if let Some(blocked) = refusal(&command, &policy) {
        return Ok(blocked);
    }
    if shell::polling(&command) {
        return Ok(Output::default());
    }
    let root = project_root(&scope.roots.project);
    let exact_count = match counters::advance(&root, &scope.payload, "cmd", &command, true) {
        Ok(count) => count,
        Err(_) => return Ok(Output::default()),
    };
    let normalized = normalize_command_signature(&command);
    let err_sig = normalized_error_signature(&scope.payload);
    let norm_identity = if err_sig.is_empty() {
        format!("norm:{normalized}")
    } else {
        format!("norm:{normalized}\0{err_sig}")
    };
    let norm_count = if normalized != command.trim() || !err_sig.is_empty() {
        counters::advance(&root, &scope.payload, "cmd", &norm_identity, true).unwrap_or(exact_count)
    } else {
        exact_count
    };
    let count = exact_count.max(norm_count);
    let debugger = debugger(&scope.host);
    let hint = diagnostic_hint(&normalized, &err_sig, debugger);
    if count >= 12 {
        return Ok(Output::error(
            2,
            format!(
                "BLOCKED: this {} has run {count} times this session.\n   Repeating it again is not a new attempt. Either change the\n   command (narrow it, add the failing case, read the output\n   differently) or escalate to {debugger} for a root cause.\n   {hint}",
                if exact_count >= 12 {
                    "exact command"
                } else {
                    "normalized command signature"
                }
            ),
        ));
    }
    if count >= 6 {
        let kind_label = if exact_count >= 6 {
            "exact command"
        } else {
            "command signature"
        };
        let message =
            format!("WARNING: this {kind_label} has run {count} times this session. {hint}");
        return if scope.host == "cursor" {
            Ok(Output::error(1, message))
        } else {
            context("PreToolUse", &message)
        };
    }
    Ok(Output::default())
}
fn patch_paths(value: &Value, paths: &mut BTreeSet<String>) {
    match value {
        Value::String(text) => {
            for line in text.lines() {
                for prefix in ["*** Add File: ", "*** Update File: ", "*** Move to: "] {
                    if let Some(path) = line.strip_prefix(prefix) {
                        paths.insert(path.tail_matches('\r').into());
                    }
                }
            }
        }
        Value::Object(map) => {
            for child in map.values() {
                patch_paths(child, paths);
            }
        }
        Value::Array(items) => {
            for child in items {
                patch_paths(child, paths);
            }
        }
        _ => {}
    }
}
pub fn file_naming(scope: &Context) -> Result<Output> {
    if !allowed_tool(&scope.payload, EDIT_TOOLS) {
        return Ok(Output::default());
    }
    let mut paths = strings(&scope.payload, &["file_path", "path"])
        .into_iter()
        .collect::<BTreeSet<_>>();
    patch_paths(&scope.payload, &mut paths);
    let skills = scope.roots.tooling.join(match scope.host.as_str() {
        "cursor" => ".cursor/skills",
        "codex" => ".agents/skills",
        _ => ".claude/skills",
    });
    let prefixes = std::fs::read_dir(&skills)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().to_str().map(|name| format!("{name}-")))
        .collect::<Vec<_>>();
    let memory_filename = regex::Regex::new(r"^MEM-[0-9]{4,}-[a-z0-9]+(?:-[a-z0-9]+)*\.md$");
    let task_directory = regex::Regex::new(r"^TASK-[0-9]{3,}$");
    for path in paths {
        if !path.ends_with(".md") {
            continue;
        }
        let normalized = path
            .strip_prefix(&format!("{}/", scope.roots.project.display()))
            .unwrap_or_else(|| path.strip_prefix("./").unwrap_or(&path));
        let file = normalized.rsplit('/').next().unwrap_or(normalized);
        let mut reason = None;
        if normalized.starts_with("memory-bank/chunks/") {
            if normalized.rsplit_once('/').map(|(parent, _)| parent) != Some("memory-bank/chunks") {
                reason = Some(format!(
                    "Memory chunks must be stored directly in memory-bank/chunks/: '{path}'"
                ));
            } else if !memory_filename
                .as_ref()
                .map_err(|_| RuntimeError::invalid("Invalid memory filename expression"))?
                .is_match(file)
            {
                reason = Some(format!(
                    "Memory chunks must use memory-bank/chunks/MEM-0001-short-slug.md: '{path}'"
                ));
            }
        } else if normalized.starts_with("tasks/") || normalized.starts_with("specs/") {
            if let Some(relative) = normalized.strip_prefix("tasks/") {
                if let Some((directory, _)) = relative.split_once('/') {
                    if !task_directory
                        .as_ref()
                        .map_err(|_| RuntimeError::invalid("Invalid task directory expression"))?
                        .is_match(directory)
                    {
                        reason = Some(format!(
                            "Task Markdown must be inside a zero-padded directory such as tasks/TASK-001/: '{path}'"
                        ));
                    }
                } else if !["README.md", "CHANGELOG.md"].contains(&file) {
                    reason = Some(format!(
                        "Task Markdown must be inside tasks/TASK-N/: '{path}'"
                    ));
                }
            }
            if reason.is_none()
                && !["README.md", "CHANGELOG.md", "MANIFEST.md"].contains(&file)
                && !prefixes.iter().any(|prefix| file.starts_with(prefix))
            {
                reason = Some(format!(
                    "'{file}' must start with a discovered skill name followed by '-'"
                ));
            }
        }
        if let Some(reason) = reason {
            return Ok(Output::error(
                2,
                format!("BLOCKED: {reason}\n  See AGENTS.md for repository file-naming rules."),
            ));
        }
    }
    Ok(Output::default())
}
pub fn edit_loop(scope: &Context) -> Result<Output> {
    if !allowed_tool(&scope.payload, EDIT_TOOLS) {
        return Ok(Output::default());
    }
    let mut paths = BTreeSet::new();
    // Cursor and Codex payloads may name the edited file `path`; Claude
    // Code's NotebookEdit names it `notebook_path`.
    if let Some(path) = first(&scope.payload, "file_path")
        .or_else(|| {
            if scope.host == "claude" {
                None
            } else {
                first(&scope.payload, "path")
            }
        })
        .or_else(|| first(&scope.payload, "notebook_path"))
    {
        paths.insert(path.into());
    } else {
        patch_paths(&scope.payload, &mut paths);
    }
    let mut warnings = vec![];
    let mut blocked = vec![];
    let root = project_root(&scope.roots.project);
    let debugger = debugger(&scope.host);
    for path in paths {
        let count = match counters::advance(&root, &scope.payload, "edit", &path, false) {
            Ok(count) => count,
            Err(_) => continue,
        };
        let shown = path.replace(['\\', '"'], "");
        if count >= 10 {
            blocked.push(format!("BLOCKED: File '{shown}' edited {count} times this session.\n   This looks like a repeated-edit loop. Consider:\n   - {debugger} to investigate the root cause\n   - Reassessing your approach"));
        } else if count >= 7 {
            warnings.push(format!("WARNING: File '{shown}' edited {count} times this session. If you're stuck in a loop, consider using {debugger}."));
        }
    }
    if !blocked.is_empty() {
        blocked.extend(warnings);
        return Ok(Output::error(2, blocked.join("\n")));
    }
    if warnings.is_empty() {
        return Ok(Output::default());
    }
    let message = warnings.join(" ");
    if scope.host == "cursor" {
        let mut output = Output::text(format!("{message}\n"));
        output.code = 1;
        Ok(output)
    } else {
        context("PostToolUse", &message)
    }
}
fn roster(scope: &Context) -> Result<BTreeMap<String, bool>> {
    let path = scope.roots.tooling.join(if scope.host == "cursor" {
        ".cursor/agents"
    } else {
        ".claude/agents"
    });
    let mut result = BTreeMap::new();
    if !path.is_dir() {
        return Ok(result);
    }
    for entry in std::fs::read_dir(path)?.filter_map(std::result::Result::ok) {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "md")
            || path.file_name().is_some_and(|name| name == "README.md")
        {
            continue;
        }
        let Ok(text) = infra_core::fs::read_text(&path) else {
            continue;
        };
        let mut lines = text.lines();
        if lines.next() != Some("---") {
            continue;
        }
        let front = lines.take_while(|line| *line != "---").collect::<Vec<_>>();
        let field = |key: &str| {
            front.iter().find_map(|line| {
                line.strip_prefix(&format!("{key}:"))
                    .map(|value| value.trim_start().replace('"', "").tail().to_string())
            })
        };
        if let Some(name) = field("name").filter(|name| !name.is_empty()) {
            result.insert(name, field("writes").as_deref() == Some("true"));
        }
    }
    Ok(result)
}
fn safe_type(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || " _.:/-".contains(*c))
        .take(64)
        .collect()
}
fn gate(scope: &Context, allow: bool, message: &str) -> Result<Output> {
    if scope.host == "cursor" {
        if allow {
            Output::json(&json!({"permission":"allow"}))
        } else {
            Output::json(&json!({"permission":"deny","user_message":message}))
        }
    } else if allow {
        Ok(Output::default())
    } else {
        Ok(Output::error(2, message))
    }
}
pub fn subagent_gate(scope: &Context) -> Result<Output> {
    if scope.host == "codex" {
        let forbidden = [
            "spawn_agent",
            "agent",
            "send_input",
            "resume_agent",
            "wait_agent",
            "close_agent",
        ];
        if strings(&scope.payload, &["tool_name"])
            .iter()
            .any(|name| forbidden.contains(&name.to_ascii_lowercase().as_str()))
        {
            return gate(
                scope,
                false,
                "BLOCKED: Codex built-in subagents are disabled in this project.\n  Use the skills in .agents/skills/ (see AGENTS.md) or do the\n  work in the main conversation instead of spawning agents.",
            );
        }
        return gate(scope, true, "");
    }
    if scope.host != "cursor" && !allowed_tool(&scope.payload, &["agent", "task"]) {
        return gate(scope, true, "");
    }
    let Some(agent) = first(&scope.payload, "subagent_type") else {
        return gate(scope, true, "");
    };
    let roster = roster(scope)?;
    if roster.is_empty() {
        return gate(scope, true, "");
    }
    let Some(writes) = roster.get(agent) else {
        // Cursor shows its person one line, in Cursor's own terms, as the
        // retired gate wrote it; Claude Code's model reads the longer text.
        let message = if scope.host == "cursor" {
            format!(
                "Blocked subagent \"{}\": built-in Cursor subagents are disabled in this project. Delegate via the project agents in .cursor/agents/ (invoke with /agent-name) or work in the main conversation.",
                safe_type(agent)
            )
        } else {
            format!(
                "BLOCKED: subagent type \"{}\" is not part of this accelerator.\n  Delegate only to the project agents defined in .{}/agents/\n  (see its README for the roster), or do the work in the main\n  conversation instead of spawning a built-in agent.",
                safe_type(agent),
                scope.host
            )
        };
        return gate(scope, false, &message);
    };
    if !writes {
        return gate(scope, true, "");
    }
    match counters::take_writer(&project_root(&scope.roots.project), &scope.host, agent) {
        Ok(None) => gate(scope, true, ""),
        Ok(Some(holder)) if scope.host == "cursor" => gate(
            scope,
            false,
            &format!(
                "Write-capable agent \"{}\" is already running; write-capable agents run one at a time. Wait for its completion or remove the stale write-agent lock.",
                safe_type(&holder)
            ),
        ),
        Ok(Some(holder)) => gate(
            scope,
            false,
            &format!(
                "BLOCKED: write-capable agent \"{}\" is already running.\n  Write-capable agents run one at a time. Wait for it to\n  finish (its completion clears the lock), or remove a stale lock.",
                safe_type(&holder)
            ),
        ),
        Err(error) => gate(scope, false, &format!("BLOCKED: {error}")),
    }
}
pub fn subagent_dispatch(scope: &Context) -> Result<Output> {
    let relative = "memory-bank/local/unrecorded-completions";
    let relay = scope.roots.state.join(relative);
    if first(&scope.payload, "hook_event_name") == Some("PostToolUse") {
        if !relay.is_file() {
            return Ok(Output::default());
        }
        let mut files = infra_core::bank::publication::PublicationFiles::new(&scope.roots.state)?;
        if files.read(relative, 1024 * 1024)?.is_none() {
            return Ok(Output::default());
        }
        let taking = scope
            .roots
            .state
            .join(format!("{relative}.relay.{}", infra_core::domain::uuid4()));
        if std::fs::rename(&relay, &taking).is_err() {
            return Ok(Output::default());
        }
        let current = infra_core::fs::read_regular(&taking)?;
        let _ = std::fs::remove_file(taking);
        let text = String::from_utf8_lossy(&current);
        let lines = text
            .lines()
            .rev()
            .take(20)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        return if lines.is_empty() {
            Ok(Output::default())
        } else {
            context("PostToolUse", &lines)
        };
    }
    let Some(agent) =
        first(&scope.payload, "agent_type").or_else(|| first(&scope.payload, "subagent_type"))
    else {
        return Ok(Output::default());
    };
    let _ = counters::release_writer(&project_root(&scope.roots.project), agent);
    let Some(task) = &scope.task else {
        return Ok(Output::default());
    };
    let note = first(&scope.payload, "last_assistant_message")
        .or_else(|| first(&scope.payload, "status"))
        .unwrap_or("")
        .replace("\\n", " ")
        .replace("\\t", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(160)
        .collect::<String>();
    let mut args = vec![
        "--task-id".into(),
        task.clone(),
        "--agent".into(),
        agent.into(),
        "--event".into(),
        "complete".into(),
    ];
    if !note.is_empty() {
        args.extend(["--note".into(), note]);
    }
    let result = scope.call("msg-dispatch", &args, &[], scope.budget);
    let (code, stderr) = match result {
        Ok(output) => (
            output.code,
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ),
        Err(error) => (1, error.to_string()),
    };
    if code == 0 || stderr.contains("task not found") || stderr.contains("require governed mode") {
        return Ok(Output::default());
    }
    let detail = stderr
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .replace('\t', " ")
        .chars()
        .take(160)
        .collect::<String>();
    let message = format!(
        "subagent-dispatch: completion of \"{}\" was NOT recorded in the channel (exit {code}{}). Record it with infra-engine memory msg-dispatch --event complete before trusting msg-read.",
        safe_type(agent),
        if detail.is_empty() {
            String::new()
        } else {
            format!(" — {detail}")
        }
    );
    if first(&scope.payload, "hook_event_name") == Some("SubagentStop") {
        let operation = (|| -> Result<()> {
            let mut files =
                infra_core::bank::publication::PublicationFiles::new(&scope.roots.state)?;
            let previous = files.read(relative, 1024 * 1024)?;
            let mut next = previous.clone().unwrap_or_default();
            next.extend_from_slice(message.as_bytes());
            next.push(b'\n');
            files.write_expected(relative, &next, previous.as_deref())
        })();
        let _ = operation;
    }
    Ok(Output {
        code: 0,
        stdout: format!("{message}\n").into_bytes(),
        stderr: format!("{message}\n").into_bytes(),
        ..Default::default()
    })
}

#[cfg(test)]
mod smart_loop_breaker_tests {
    use super::*;
    use crate::hooks;

    #[test]
    fn normalized_bash_signature_warns_with_diagnostic_hints_before_blocking_at_12() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".git")).unwrap();

        let session = format!("smart-loop-{}", infra_core::domain::uuid4());
        let variants = [
            "cargo test foo --lib | head -n 20",
            "cargo test foo --lib 2>&1 | tail -n 10",
            "cargo   test   foo   --lib >/dev/null",
        ];

        // 5 invocations produce zero warning.
        for i in 0..5 {
            let cmd = variants[i % variants.len()];
            let payload = json!({
                "session_id": session,
                "tool_name": "Bash",
                "command": cmd,
                "stderr": "error[E0308]: mismatched types"
            });
            let out = hooks::execute(
                root,
                "claude",
                "bash-validator",
                &serde_json::to_vec(&payload).unwrap(),
            )
            .unwrap();
            assert_eq!(out.code, 0, "attempt {i} should pass: {out:?}");
            assert!(
                out.stdout.is_empty(),
                "attempt {i} should be silent: {out:?}"
            );
        }

        // Attempts 6..=11 emit deterministic diagnostic hint before blocking.
        for i in 6..=11 {
            let cmd = variants[i % variants.len()];
            let payload = json!({
                "session_id": session,
                "tool_name": "Bash",
                "command": cmd,
                "stderr": "error[E0308]: mismatched types"
            });
            let out = hooks::execute(
                root,
                "claude",
                "bash-validator",
                &serde_json::to_vec(&payload).unwrap(),
            )
            .unwrap();
            assert_eq!(out.code, 0, "attempt {i} should warn, not block: {out:?}");
            let stdout = String::from_utf8(out.stdout).unwrap();
            assert!(stdout.contains("WARNING:"), "{stdout}");
            assert!(
                stdout.contains("Hint:") && stdout.contains("--nocapture"),
                "{stdout}"
            );
            assert!(stdout.contains("mismatched types"), "{stdout}");
        }

        // Attempt 12 blocks cleanly.
        let payload_12 = json!({
            "session_id": session,
            "tool_name": "Bash",
            "command": variants[0],
            "stderr": "error[E0308]: mismatched types"
        });
        let blocked = hooks::execute(
            root,
            "claude",
            "bash-validator",
            &serde_json::to_vec(&payload_12).unwrap(),
        )
        .unwrap();
        assert_eq!(blocked.code, 2);
        let stderr = String::from_utf8(blocked.stderr).unwrap();
        assert!(
            stderr.contains("BLOCKED:") && stderr.contains("Hint:"),
            "{stderr}"
        );
    }
}
