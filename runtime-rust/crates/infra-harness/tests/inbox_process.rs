//! Two actual CLI processes, native guardians, lifecycle and automatic inbox handoff.
use serde_json::{Value, json};
use std::{
    fs::File,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
fn offline_native_inbox_resumes_once_and_conflicting_source_identity_blocks() {
    for scenario in ["success", "conflict"] {
        let temporary = tempfile::tempdir().unwrap();
        let stdout = temporary.path().join("stdout");
        let stderr = temporary.path().join("stderr");
        let mut child = Command::new(env!("CARGO_BIN_EXE_infra-harness-inbox-fixture"))
            .env_remove("RUST_MIN_STACK")
            .arg(temporary.path())
            .arg(scenario)
            .stdin(Stdio::null())
            .stdout(File::create(&stdout).unwrap())
            .stderr(File::create(&stderr).unwrap())
            .spawn()
            .unwrap();
        let until = Instant::now() + Duration::from_secs(35);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= until {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "Inbox fixture timed out: {}",
                    std::fs::read_to_string(&stderr).unwrap()
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(
            status.success(),
            "{scenario}: {}",
            std::fs::read_to_string(&stderr).unwrap()
        );
        let result: Value = serde_json::from_slice(&std::fs::read(stdout).unwrap()).unwrap();
        assert_eq!(
            result,
            json!({"scenario":scenario,"calls":if scenario=="success" {2} else {1},"receipt":if scenario=="success" {"completed"} else {"blocked"}})
        );
    }
}
