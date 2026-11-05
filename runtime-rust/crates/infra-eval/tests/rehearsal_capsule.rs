//! The rollout rehearsal reads what a prompt hook gave the model.
use infra_eval::rehearsal::capsule;
use serde_json::json;
use std::path::Path;

/// Claude Code and Codex take a prompt hook's plain stdout as context, and
/// the native hooks print the capsule that way; a JSON answer gives its
/// additional context. Reading JSON alone reported every recall missing.
#[test]
fn a_prompt_hooks_plain_or_json_output_is_its_capsule() {
    let root = Path::new("/nonexistent-rehearsal-root");
    let plain = json!({"stdout": "working: TASK-1 — Goal\nmemory: rehearsal1234abcd\n"});
    let structured = json!({"stdout": json!({
        "hookSpecificOutput": {"hookEventName": "UserPromptSubmit", "additionalContext": "recalled rehearsal5678"}
    }).to_string()});
    let silent = json!({"stdout": ""});
    let text = capsule(root, "claude", &[plain, structured, silent]);
    assert!(text.contains("rehearsal1234abcd"), "{text}");
    assert!(text.contains("recalled rehearsal5678"), "{text}");
    assert!(!text.contains("hookSpecificOutput"), "{text}");
}
