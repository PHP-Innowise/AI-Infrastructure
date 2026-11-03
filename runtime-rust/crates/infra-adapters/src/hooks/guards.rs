use super::{Context, first, project_root, strings};
use crate::TailTrim;
use crate::{counters, output::Output, shell};
use infra_core::{Result, Roots, RuntimeConfig, RuntimeError};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
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
    let count = match counters::advance(
        &project_root(&scope.roots.project),
        &scope.payload,
        "cmd",
        &command,
        true,
    ) {
        Ok(count) => count,
        Err(_) => return Ok(Output::default()),
    };
    if count >= 12 {
        return Ok(Output::error(
            2,
            format!(
                "BLOCKED: this exact command has run {count} times this session.\n   Repeating it again is not a new attempt. Either change the\n   command (narrow it, add the failing case, read the output\n   differently) or escalate to /debugger for a root cause."
            ),
        ));
    }
    if count >= 6 {
        let message = format!(
            "WARNING: this exact command has run {count} times this session. If it keeps failing, /debugger instead of another rerun."
        );
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
    if !allowed_tool(
        &scope.payload,
        &[
            "write",
            "edit",
            "multiedit",
            "multi_edit",
            "notebookedit",
            "notebook_edit",
            "apply_patch",
        ],
    ) {
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
    if !allowed_tool(
        &scope.payload,
        &[
            "write",
            "edit",
            "multiedit",
            "multi_edit",
            "notebookedit",
            "notebook_edit",
            "apply_patch",
        ],
    ) {
        return Ok(Output::default());
    }
    let mut paths = BTreeSet::new();
    if let Some(path) =
        first(&scope.payload, "file_path").or_else(|| first(&scope.payload, "notebook_path"))
    {
        paths.insert(path.into());
    } else {
        patch_paths(&scope.payload, &mut paths);
    }
    let mut warnings = vec![];
    let mut blocked = vec![];
    let root = project_root(&scope.roots.project);
    for path in paths {
        let count = match counters::advance(&root, &scope.payload, "edit", &path, false) {
            Ok(count) => count,
            Err(_) => continue,
        };
        let shown = path.replace(['\\', '"'], "");
        if count >= 10 {
            blocked.push(format!("BLOCKED: File '{shown}' edited {count} times this session.\n   This looks like a repeated-edit loop. Consider:\n   - /debugger to investigate the root cause\n   - Reassessing your approach"));
        } else if count >= 7 {
            warnings.push(format!("WARNING: File '{shown}' edited {count} times this session. If you're stuck in a loop, consider using /debugger."));
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
        return gate(
            scope,
            false,
            &format!(
                "BLOCKED: subagent type \"{}\" is not part of this accelerator.\n  Delegate only to the project agents defined in .{}/agents/\n  (see its README for the roster), or do the work in the main\n  conversation instead of spawning a built-in agent.",
                safe_type(agent),
                scope.host
            ),
        );
    };
    if !writes {
        return gate(scope, true, "");
    }
    match counters::take_writer(&project_root(&scope.roots.project), &scope.host, agent) {
        Ok(None) => gate(scope, true, ""),
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
