//! A worker CLI that exits non-zero says why, and the receipt keeps it: a
//! real Claude Code whose sign-in had expired stopped a run as only
//! `worker_exit_failure`.
use infra_system::native::worker_failure;

#[test]
fn a_failed_claude_result_names_its_cause() {
    let output = concat!(
        r#"{"type":"system","subtype":"init","session_id":"s"}"#,
        "\n",
        r#"{"type":"result","subtype":"success","is_error":true,"terminal_reason":"api_error","result":"Failed to authenticate: OAuth session expired and could not be refreshed","session_id":"s"}"#,
        "\n"
    );
    assert_eq!(
        worker_failure(output.as_bytes(), "claude").as_deref(),
        Some("Failed to authenticate: OAuth session expired and could not be refreshed")
    );
}

#[test]
fn a_failed_codex_turn_names_its_cause() {
    let output = concat!(
        r#"{"type":"thread.started","thread_id":"t"}"#,
        "\n",
        r#"{"type":"turn.failed","error":{"message":"You've hit your usage limit. Try again later."}}"#,
        "\n"
    );
    assert_eq!(
        worker_failure(output.as_bytes(), "codex").as_deref(),
        Some("You've hit your usage limit. Try again later.")
    );
}

#[test]
fn plain_output_falls_back_to_its_last_line_and_secrets_are_withheld() {
    assert_eq!(
        worker_failure(b"starting\nNot signed in: run the CLI's login\n", "command").as_deref(),
        Some("Not signed in: run the CLI's login")
    );
    assert_eq!(
        worker_failure(b"api_key = sk-abcdefghijklmnopqrstuvwxyz0123\n", "command").as_deref(),
        Some("(withheld: the message looks like a secret)")
    );
    assert_eq!(worker_failure(b"\n  \n", "codex"), None);
}
