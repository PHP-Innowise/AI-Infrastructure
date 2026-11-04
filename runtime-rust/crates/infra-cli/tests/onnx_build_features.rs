//! Optional inference changes the embedded worker, not its JSON transport or
//! request validation. These assertions also run when workspace features unify.
use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

fn worker(request: Value) -> Value {
    let absent = std::env::temp_dir().join(format!(
        "infra-onnx-absent-assets-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    assert!(!absent.exists());
    let mut child = Command::new(env!("CARGO_BIN_EXE_infra-engine"))
        .arg("onnx-worker")
        .arg("--assets")
        .arg(&absent)
        .arg("--ort-library")
        .arg(absent.join("onnxruntime"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&request).unwrap())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "worker errors use JSON: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert!(!absent.exists(), "the worker created an absent asset root");
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn valid_requests_preserve_compiled_and_disabled_worker_failures() {
    let expected = if infra_pilots::onnx::compiled() {
        "asset-missing"
    } else {
        "dependency-unavailable"
    };
    assert_eq!(
        worker(json!({"query":"invoice", "passages":["An invoice passage."]})),
        json!({"error":expected})
    );
}

#[test]
fn invalid_requests_remain_invalid_with_or_without_inference() {
    for request in [
        json!({"query":42, "passages":["An invoice passage."]}),
        json!({"query":"invoice", "passages":[]}),
        json!({"query":"invoice", "passages":["An invoice passage."], "unexpected":true}),
    ] {
        assert_eq!(worker(request), json!({"error":"invalid-request"}));
    }
}
