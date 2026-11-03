//! Portable launches for installed and generated projects.
//!
//! Committed hook and MCP wiring names no path of the machine it was installed
//! on: the host names the checkout (`$CLAUDE_PROJECT_DIR`, Cursor's project-root
//! working directory and `${workspaceFolder}`, Git's top level for Codex) and
//! the engine is found there. A linked worktree has no engine of its own, since
//! the engine is ignored by Git, and uses the one installed in its
//! repository's main checkout. Native Windows installations keep their
//! machine-bound launch (see `attach::hook_handler`).
use infra_core::{Result, RuntimeError, git::GitFrame};
use serde_json::{Value, json};
use std::path::Path;

/// First line of every portable MCP launcher script; it marks ownership.
pub const MARKER: &str = "# infra-engine portable launcher v1";
const ENGINE: &str = "memory-bank/bin/infra-engine";

/// Whether this platform's installations use the portable form.
pub fn enabled() -> bool {
    !cfg!(windows)
}

/// The installed project's path below its Git top level, with a trailing
/// slash (`""` at the top or outside Git). `None` when the prefix cannot be
/// written inside quoted shell, JSON and TOML unchanged.
pub fn prefix(root: &Path) -> Result<Option<String>> {
    let prefix = match GitFrame::discover(root)? {
        Some(frame) => frame.prefix,
        None => String::new(),
    };
    Ok(prefix
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || " ._+@/-".contains(c))
        .then_some(prefix))
}

/// POSIX shell lines that move the `root` variable to the same directory of
/// the main checkout when a linked worktree has no executable engine. They
/// contain no `${`, no single quote and no backslash: hosts expand `${...}` in
/// configuration, and the lines are embedded in single-quoted shell.
pub fn worktree_fallback(root: &str, engine: &str, binary: &str) -> String {
    format!(
        "if [ ! -x \"${engine}\" ] && common=$(git -C \"${root}\" rev-parse --path-format=absolute --git-common-dir 2>/dev/null) && below=$(git -C \"${root}\" rev-parse --show-prefix 2>/dev/null) && main=$(CDPATH= cd -- \"$(dirname -- \"$common\")/$below\" && pwd -P); then\n  {root}=$main\n  {engine}=${root}/memory-bank/bin/{binary}\nfi\n"
    )
}

/// The committed hook launch for one host and role.
pub fn hook_handler(host: &str, role: &str, prefix: &str) -> Result<Value> {
    if role.is_empty()
        || !role
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(RuntimeError::invalid("Invalid native hook role"));
    }
    let command = match (host, role) {
        // The notification role has no launcher script; a worktree session
        // without an engine only misses the desktop notice.
        ("claude", "notification") => format!(
            "\"$CLAUDE_PROJECT_DIR\"/{ENGINE} hook claude notification --root \"$CLAUDE_PROJECT_DIR\""
        ),
        ("claude", _) => format!("\"$CLAUDE_PROJECT_DIR\"/.claude/hooks/{role}.sh"),
        // Cursor runs project hooks from the project root.
        ("cursor", _) => format!(".cursor/hooks/{role}.sh"),
        // Codex runs hooks in the session directory through the user's login
        // shell; Codex documents resolving repository scripts from Git's top.
        ("codex", _) => format!(
            "sh -c 'exec \"$(git rev-parse --show-toplevel 2>/dev/null || pwd)/{prefix}.codex/hooks/{role}.sh\"'"
        ),
        _ => return Err(RuntimeError::invalid("Invalid native handler host")),
    };
    Ok(json!({ "command": command }))
}

/// The committed memory MCP registration for one host.
pub fn mcp_entry(host: &str, prefix: &str) -> Result<Value> {
    let (start, mut tail) = match host {
        // Claude Code sets CLAUDE_PROJECT_DIR in the server's environment.
        "claude" => (
            "root=$CLAUDE_PROJECT_DIR\n[ -n \"$root\" ] || root=$(pwd)".to_string(),
            vec![],
        ),
        // Cursor substitutes the project root into the first argument.
        "cursor" => (
            "root=$1\nshift".to_string(),
            vec![json!("${workspaceFolder}")],
        ),
        "codex" => (
            format!("root=$(git rev-parse --show-toplevel 2>/dev/null || pwd)/{prefix}"),
            vec![],
        ),
        _ => return Err(RuntimeError::invalid("Unknown MCP host")),
    };
    let script = format!(
        "{MARKER}\n{start}\nengine=$root/{ENGINE}\n{}exec \"$engine\" \"$@\" --root \"$root\"\n",
        worktree_fallback("root", "engine", "infra-engine")
    );
    let mut args = vec![json!("-c"), json!(script), json!("sh")];
    args.append(&mut tail);
    args.extend([json!("mcp"), json!("--host"), json!(host)]);
    Ok(json!({"type": "stdio", "command": "sh", "args": args}))
}

/// Whether an MCP entry is a portable launcher of this kind for `host`.
pub fn owned_mcp(command: &str, args: &[Value], host: &str) -> bool {
    let tail: &[Value] = &[json!("mcp"), json!("--host"), json!(host)];
    let leading = if host == "cursor" { 4 } else { 3 };
    command == "sh"
        && args.len() == leading + tail.len()
        && args[0] == "-c"
        && args[1]
            .as_str()
            .is_some_and(|script| script.starts_with(&format!("{MARKER}\n")))
        && args[2] == "sh"
        && (host != "cursor" || args[3] == "${workspaceFolder}")
        && args[leading..] == *tail
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launches_name_no_machine_path_and_avoid_host_expansion() {
        for host in ["claude", "cursor", "codex"] {
            let entry = mcp_entry(host, "apps/shop/").unwrap();
            let args = entry["args"].as_array().unwrap();
            let script = args[1].as_str().unwrap();
            assert!(!script.contains("${") && !script.contains('\'') && !script.contains('\\'));
            assert!(owned_mcp("sh", args, host));
            assert!(!owned_mcp(
                "sh",
                args,
                if host == "codex" { "claude" } else { "codex" }
            ));
            for role in ["bash-validator", "working-memory-read"] {
                let command = hook_handler(host, role, "apps/shop/").unwrap()["command"]
                    .as_str()
                    .unwrap()
                    .to_string();
                assert!(
                    command.ends_with(&format!("{role}.sh"))
                        || command.ends_with(&format!("{role}.sh\"'"))
                );
                assert!(!command.contains("/home/") && !command.contains("/tmp/"));
            }
        }
        assert!(
            hook_handler("codex", "bash-validator", "apps/shop/").unwrap()["command"]
                .as_str()
                .unwrap()
                .contains("/apps/shop/.codex/hooks/bash-validator.sh")
        );
        assert!(hook_handler("other", "bash-validator", "").is_err());
        assert!(hook_handler("claude", "bash validator", "").is_err());
    }
}
