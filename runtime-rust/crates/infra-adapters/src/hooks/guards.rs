use super::{Context, first, project_root, strings};
use crate::TailTrim;
use crate::{counters, output::Output, shell};
use infra_core::{Result, Roots, RuntimeConfig};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::UNIX_EPOCH,
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
const CHECK_TOOLS: &[&str] = &["bash", "shell", "terminal", "run_command"];
const SKILL_CACHE_RACY_NS: i128 = 2_000_000_000;

#[cfg(test)]
thread_local! {
    static SKILL_DIR_SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
fn bump_skill_dir_scans() {
    SKILL_DIR_SCANS.with(|cell| cell.set(cell.get() + 1));
}
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
fn integer_field(value: &Value, keys: &[&str]) -> Option<i64> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(found) = map.get(*key).and_then(Value::as_i64) {
                    return Some(found);
                }
            }
            map.values().find_map(|child| integer_field(child, keys))
        }
        Value::Array(items) => items.iter().find_map(|child| integer_field(child, keys)),
        _ => None,
    }
}
fn bool_field(value: &Value, keys: &[&str]) -> Option<bool> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(found) = map.get(*key).and_then(Value::as_bool) {
                    return Some(found);
                }
            }
            map.values().find_map(|child| bool_field(child, keys))
        }
        Value::Array(items) => items.iter().find_map(|child| bool_field(child, keys)),
        _ => None,
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CheckOutcome {
    PreExecution,
    Passing,
    Failing,
}
fn check_outcome(payload: &Value, command: &str, err_sig: &str) -> CheckOutcome {
    let exit_code = integer_field(
        payload,
        &["exit_code", "returncode", "exitCode", "status_code"],
    );
    let explicit_ok = bool_field(payload, &["passed", "ok", "success"]);
    let status_str = first(payload, "status").map(str::to_ascii_lowercase);
    if !err_sig.is_empty()
        || exit_code.is_some_and(|code| code != 0)
        || explicit_ok == Some(false)
        || matches!(
            status_str.as_deref(),
            Some("failed" | "failure" | "error" | "errored")
        )
    {
        CheckOutcome::Failing
    } else if !shell::polling(command)
        && (exit_code == Some(0)
            || explicit_ok == Some(true)
            || matches!(
                status_str.as_deref(),
                Some("passed" | "pass" | "ok" | "success" | "succeeded")
            ))
    {
        CheckOutcome::Passing
    } else {
        CheckOutcome::PreExecution
    }
}
fn combine_command_error_signature(normalized_cmd: &str, err_sig: &str) -> String {
    if normalized_cmd.is_empty() && err_sig.is_empty() {
        String::new()
    } else if err_sig.is_empty() {
        format!("norm:{normalized_cmd}")
    } else if normalized_cmd.is_empty() {
        format!("err:{err_sig}")
    } else {
        format!("norm:{normalized_cmd}\0{err_sig}")
    }
}
fn is_check_event(payload: &Value) -> bool {
    let has_cmd = first(payload, "command").is_some() || first(payload, "cmd").is_some();
    if !has_cmd {
        return false;
    }
    let tools = strings(payload, &["tool_name"]);
    (!tools.is_empty() && allowed_tool(payload, CHECK_TOOLS))
        || integer_field(
            payload,
            &["exit_code", "returncode", "exitCode", "status_code"],
        )
        .is_some()
        || bool_field(payload, &["passed", "ok", "success"]).is_some()
        || first(payload, "status").is_some()
        || !normalized_error_signature(payload).is_empty()
}
pub(super) fn payload_noop(role: &str, payload: &Value) -> bool {
    match role {
        // These borrowed searches have the same existence test as strings(),
        // without copying a possibly large command before shell validation.
        "bash-validator" => first(payload, "command").is_none() && first(payload, "cmd").is_none(),
        "file-naming-validator" => !allowed_tool(payload, EDIT_TOOLS),
        "loop-detection" => !allowed_tool(payload, EDIT_TOOLS) && !is_check_event(payload),
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
    let normalized = normalize_command_signature(&command);
    let err_sig = normalized_error_signature(&scope.payload);
    if check_outcome(&scope.payload, &command, &err_sig) == CheckOutcome::Passing {
        let _ = counters::reset(&root, &scope.payload);
        return Ok(Output::default());
    }
    let norm_identity = combine_command_error_signature(&normalized, &err_sig);
    let _ = counters::record_signature(&root, &scope.payload, &norm_identity);
    let exact_identity = if err_sig.is_empty() {
        command.clone()
    } else {
        format!("{command}\0{err_sig}")
    };
    let exact_count = match counters::advance(&root, &scope.payload, "cmd", &exact_identity, true) {
        Ok(count) => count,
        Err(_) => return Ok(Output::default()),
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
fn memory_filename_re() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"^MEM-[0-9]{4,}-[a-z0-9]+(?:-[a-z0-9]+)*\.md$")
            .expect("constant memory filename regex")
    })
}
fn task_directory_re() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"^TASK-[0-9]{3,}$").expect("constant task directory regex")
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct SkillDirStamp {
    mtime_ns: i128,
    ctime_ns: i128,
    size: u64,
    ino: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct SkillDirCacheEntry {
    skills_dir: String,
    stamp: SkillDirStamp,
    prefixes: Vec<String>,
}

fn dir_stamp(path: &Path) -> Option<(SkillDirStamp, i128)> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return None;
    }
    let mtime_ns = i128::try_from(
        meta.modified()
            .ok()?
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_nanos(),
    )
    .ok()?;
    #[cfg(unix)]
    let (ctime_ns, size, ino) = {
        use std::os::unix::fs::MetadataExt;
        let ctime = i128::from(meta.ctime()) * 1_000_000_000 + i128::from(meta.ctime_nsec());
        (ctime, meta.size(), meta.ino())
    };
    #[cfg(not(unix))]
    let (ctime_ns, size, ino) = (mtime_ns, meta.len(), 0u64);
    let settled = mtime_ns.max(ctime_ns);
    Some((
        SkillDirStamp {
            mtime_ns,
            ctime_ns,
            size,
            ino,
        },
        settled,
    ))
}

fn current_unix_ns() -> i128 {
    i128::try_from(
        std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    )
    .unwrap_or(0)
}

fn scan_skill_prefixes(skills: &Path) -> Vec<String> {
    #[cfg(test)]
    bump_skill_dir_scans();
    let mut prefixes = std::fs::read_dir(skills)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().to_str().map(|name| format!("{name}-")))
        .collect::<Vec<_>>();
    prefixes.sort();
    prefixes
}

fn skill_prefixes_with_now(skills: &Path, cache_file: Option<&Path>, now_ns: i128) -> Vec<String> {
    static CACHE: OnceLock<Mutex<BTreeMap<PathBuf, SkillDirCacheEntry>>> = OnceLock::new();
    let Some((stamp, settled_ns)) = dir_stamp(skills) else {
        return Vec::new();
    };
    let key_str = skills.to_string_lossy().into_owned();
    let racy_settled = now_ns - settled_ns >= SKILL_CACHE_RACY_NS;
    let memory = CACHE.get_or_init(|| Mutex::new(BTreeMap::new()));
    if racy_settled
        && let Ok(guard) = memory.lock()
        && let Some(entry) = guard.get(skills)
        && entry.stamp == stamp
    {
        return entry.prefixes.clone();
    }
    if racy_settled
        && let Some(cache_path) = cache_file
        && let Ok(meta) = std::fs::symlink_metadata(cache_path)
        && !meta.file_type().is_symlink()
        && meta.is_file()
        && let Ok(raw) = std::fs::read(cache_path)
        && let Ok(entry) = serde_json::from_slice::<SkillDirCacheEntry>(&raw)
        && entry.skills_dir == key_str
        && entry.stamp == stamp
    {
        if let Ok(mut guard) = memory.lock() {
            guard.insert(skills.to_path_buf(), entry.clone());
        }
        return entry.prefixes;
    }

    let prefixes = scan_skill_prefixes(skills);
    let entry = SkillDirCacheEntry {
        skills_dir: key_str,
        stamp,
        prefixes: prefixes.clone(),
    };
    if racy_settled {
        if let Ok(mut guard) = memory.lock() {
            guard.insert(skills.to_path_buf(), entry.clone());
        }
        if let Some(cache_path) = cache_file
            && cache_path.parent().is_some_and(|parent| {
                std::fs::symlink_metadata(parent)
                    .is_ok_and(|meta| !meta.file_type().is_symlink() && meta.is_dir())
            })
            && let Ok(bytes) = serde_json::to_vec(&entry)
        {
            let _ = std::fs::write(cache_path, bytes);
        }
    }
    prefixes
}

pub fn file_naming(scope: &Context) -> Result<Output> {
    if !allowed_tool(&scope.payload, EDIT_TOOLS) {
        return Ok(Output::default());
    }
    let mut paths = strings(&scope.payload, &["file_path", "path"])
        .into_iter()
        .collect::<BTreeSet<_>>();
    patch_paths(&scope.payload, &mut paths);
    let mut cached_prefixes: Option<Vec<String>> = None;
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
            } else if !memory_filename_re().is_match(file) {
                reason = Some(format!(
                    "Memory chunks must use memory-bank/chunks/MEM-0001-short-slug.md: '{path}'"
                ));
            }
        } else if normalized.starts_with("tasks/") || normalized.starts_with("specs/") {
            if let Some(relative) = normalized.strip_prefix("tasks/") {
                if let Some((directory, _)) = relative.split_once('/') {
                    if !task_directory_re().is_match(directory) {
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
            if reason.is_none() && !["README.md", "CHANGELOG.md", "MANIFEST.md"].contains(&file) {
                let prefixes = cached_prefixes.get_or_insert_with(|| {
                    let subdir = match scope.host.as_str() {
                        "cursor" => ".cursor/skills",
                        "codex" => ".agents/skills",
                        _ => ".claude/skills",
                    };
                    let skills = scope.roots.tooling.join(subdir);
                    let cache_file = scope.roots.state.join(format!(
                        "memory-bank/local/skill-naming-cache-{}.json",
                        scope.host
                    ));
                    skill_prefixes_with_now(&skills, Some(&cache_file), current_unix_ns())
                });
                if !prefixes.iter().any(|prefix| file.starts_with(prefix)) {
                    reason = Some(format!(
                        "'{file}' must start with a discovered skill name followed by '-'"
                    ));
                }
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
    let root = project_root(&scope.roots.project);
    if is_check_event(&scope.payload) {
        let command = strings(&scope.payload, &["command", "cmd"]).join("\n");
        let err_sig = normalized_error_signature(&scope.payload);
        let normalized = normalize_command_signature(&command);
        match check_outcome(&scope.payload, &command, &err_sig) {
            CheckOutcome::Passing => {
                let _ = counters::reset(&root, &scope.payload);
            }
            CheckOutcome::Failing | CheckOutcome::PreExecution => {
                let sig = combine_command_error_signature(&normalized, &err_sig);
                let _ = counters::record_signature(&root, &scope.payload, &sig);
            }
        }
        return Ok(Output::default());
    }
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
    let inline_cmd = strings(&scope.payload, &["command", "cmd"]).join("\n");
    let inline_err = normalized_error_signature(&scope.payload);
    let inline_sig =
        combine_command_error_signature(&normalize_command_signature(&inline_cmd), &inline_err);
    let active_sig = if !inline_sig.is_empty() {
        Some(inline_sig)
    } else {
        counters::active_signature(&root, &scope.payload)
    };
    let mut warnings = vec![];
    let mut blocked = vec![];
    let debugger = debugger(&scope.host);
    for path in paths {
        let identity = match active_sig.as_deref() {
            Some(sig) => format!("{path}\0{sig}"),
            None => path.clone(),
        };
        let count = match counters::advance(&root, &scope.payload, "edit", &identity, false) {
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

    #[test]
    fn file_naming_defers_non_markdown_and_reuses_settled_skill_cache() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let skills = root.join(".claude/skills");
        std::fs::create_dir_all(skills.join("alpha-skill")).unwrap();
        std::fs::create_dir_all(root.join("memory-bank/local")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();

        let (stamp, settled_ns) = dir_stamp(&skills).unwrap();
        let cache_file = root.join("memory-bank/local/skill-naming-cache-claude.json");

        SKILL_DIR_SCANS.with(|c| c.set(0));
        // Inside racy window (< 2s) always scans and avoids persisting stale cache.
        let racy_prefixes = skill_prefixes_with_now(
            &skills,
            Some(&cache_file),
            settled_ns + SKILL_CACHE_RACY_NS - 1,
        );
        assert_eq!(racy_prefixes, vec!["alpha-skill-".to_string()]);
        assert_eq!(SKILL_DIR_SCANS.with(|c| c.get()), 1);
        assert!(!cache_file.exists());

        // Outside racy window: first call scans once and caches; second call hits cache with 0 scans!
        let settled_now = settled_ns + SKILL_CACHE_RACY_NS + 1;
        let first = skill_prefixes_with_now(&skills, Some(&cache_file), settled_now);
        assert_eq!(first, vec!["alpha-skill-".to_string()]);
        assert_eq!(SKILL_DIR_SCANS.with(|c| c.get()), 2);
        assert!(cache_file.is_file());

        let second = skill_prefixes_with_now(&skills, Some(&cache_file), settled_now + 500);
        assert_eq!(second, first);
        assert_eq!(SKILL_DIR_SCANS.with(|c| c.get()), 2);

        // Editing a non-Markdown file through file-naming-validator skips skill dir scanning altogether.
        let payload = json!({
            "session_id": "naming-skip",
            "tool_name": "Edit",
            "file_path": root.join("src/App.php")
        });
        let before_scans = SKILL_DIR_SCANS.with(|c| c.get());
        let out = hooks::execute(
            root,
            "claude",
            "file-naming-validator",
            &serde_json::to_vec(&payload).unwrap(),
        )
        .unwrap();
        assert_eq!(out.code, 0);
        assert_eq!(SKILL_DIR_SCANS.with(|c| c.get()), before_scans);
        let _ = stamp;
    }

    #[test]
    fn edit_loop_discriminates_signatures_resets_on_passing_checks_and_isolates_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join(".git")).unwrap();

        let sess_a = format!("loop-a-{}", infra_core::domain::uuid4());
        let sess_b = format!("loop-b-{}", infra_core::domain::uuid4());
        let target_file = root.join("src/Service.php").to_string_lossy().into_owned();

        // Fail under normalized signature A (`cargo test foo`), edit 6 times -> below warning threshold.
        let fail_a = json!({
            "session_id": sess_a,
            "tool_name": "Bash",
            "command": "cargo test foo",
            "exit_code": 1,
            "stderr": "error[E0308]: mismatch"
        });
        hooks::execute(
            root,
            "claude",
            "bash-validator",
            &serde_json::to_vec(&fail_a).unwrap(),
        )
        .unwrap();
        let edit_payload = json!({
            "session_id": sess_a,
            "tool_name": "Edit",
            "file_path": target_file
        });
        for i in 0..6 {
            let out = hooks::execute(
                root,
                "claude",
                "loop-detection",
                &serde_json::to_vec(&edit_payload).unwrap(),
            )
            .unwrap();
            assert_eq!(out.code, 0);
            assert!(
                out.stdout.is_empty(),
                "edit {i} under sig A should be silent"
            );
        }

        // Switching to distinct signature B (`cargo test bar`) tracks (file, sig B) separately rather than blocking!
        let fail_b = json!({
            "session_id": sess_a,
            "tool_name": "Bash",
            "command": "cargo test bar",
            "exit_code": 1,
            "stderr": "error[E0425]: missing name"
        });
        hooks::execute(
            root,
            "claude",
            "bash-validator",
            &serde_json::to_vec(&fail_b).unwrap(),
        )
        .unwrap();
        for i in 0..6 {
            let out = hooks::execute(
                root,
                "claude",
                "loop-detection",
                &serde_json::to_vec(&edit_payload).unwrap(),
            )
            .unwrap();
            assert_eq!(out.code, 0);
            assert!(
                out.stdout.is_empty(),
                "edit {i} under distinct sig B should not warn: {out:?}"
            );
        }

        // 7th edit under sig B warns, and 10th edit blocks when stuck under the same signature.
        let warn = hooks::execute(
            root,
            "claude",
            "loop-detection",
            &serde_json::to_vec(&edit_payload).unwrap(),
        )
        .unwrap();
        assert!(
            !warn.stdout.is_empty(),
            "7th edit under same sig should warn"
        );

        // Passing check via PostToolUse loop-detection resets session edit counters completely.
        let pass_check = json!({
            "session_id": sess_a,
            "tool_name": "Bash",
            "command": "cargo test bar",
            "exit_code": 0
        });
        let pass_out = hooks::execute(
            root,
            "claude",
            "loop-detection",
            &serde_json::to_vec(&pass_check).unwrap(),
        )
        .unwrap();
        assert_eq!(pass_out.code, 0);
        for i in 0..6 {
            let out = hooks::execute(
                root,
                "claude",
                "loop-detection",
                &serde_json::to_vec(&edit_payload).unwrap(),
            )
            .unwrap();
            assert!(
                out.stdout.is_empty(),
                "post-reset edit {i} must start fresh: {out:?}"
            );
        }

        // Edits in session B must not reset session A's bash counter (`reset_after_edit` session isolation).
        let cmd_a =
            json!({"session_id": sess_a, "tool_name": "Bash", "command": "phpunit --filter A"});
        for _ in 0..6 {
            hooks::execute(
                root,
                "claude",
                "bash-validator",
                &serde_json::to_vec(&cmd_a).unwrap(),
            )
            .unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
        let edit_b = json!({"session_id": sess_b, "tool_name": "Edit", "file_path": target_file});
        hooks::execute(
            root,
            "claude",
            "loop-detection",
            &serde_json::to_vec(&edit_b).unwrap(),
        )
        .unwrap();
        let next_a = hooks::execute(
            root,
            "claude",
            "bash-validator",
            &serde_json::to_vec(&cmd_a).unwrap(),
        )
        .unwrap();
        let stdout_a = String::from_utf8_lossy(&next_a.stdout);
        assert!(
            stdout_a.contains("WARNING:"),
            "session B edit leaked into session A reset: {stdout_a}"
        );
    }
}
