use memory_kernel_probe::MAX_INPUT_BYTES;
use serde_json::{Value, json};
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn invoke(arguments: &[&str], input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_memory-kernel-probe"))
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

fn assert_error(arguments: &[&str], input: &[u8], reason: &str) {
    let output = invoke(arguments, input);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    let expected = format!("{{\"protocol\":1,\"error\":\"{reason}\"}}\n");
    assert_eq!(output.stdout, expected.as_bytes());
}

#[test]
fn binary_serves_batch_and_keeps_stderr_empty() {
    let input = br#"{"protocol":1,"requests":[{"operation":"split-frontmatter-v1","text":"---\nNaN\n---\nbody"},{"operation":"fingerprint-bytes-v1","bytes_hex":"bad"}]}"#;
    let output = invoke(&[], input);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({
            "protocol":1,"results":[
                {"status":"ok","metadata_text":"NaN","body":"body"},
                {"status":"refused","reason":"invalid-hex"}
            ]
        })
    );
}

#[test]
fn binary_errors_are_fixed_and_do_not_echo_input() {
    assert_error(&[], b"private malformed input", "invalid-request");
    assert_error(&[], b"", "invalid-request");
    assert_error(&[], b"\xff", "invalid-request");
    assert_error(&["--help"], b"", "invalid-arguments");
    assert_error(&["--version", "extra"], b"", "invalid-arguments");
    assert_error(&[], &vec![b' '; MAX_INPUT_BYTES + 1], "input-limit");
}

#[test]
fn binary_version_does_not_need_an_envelope() {
    let output = invoke(&["--version"], b"");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({
            "protocol":1,"version":env!("CARGO_PKG_VERSION")
        })
    );
}
