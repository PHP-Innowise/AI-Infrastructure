//! Gates and observed cleanup receipts qualify coordination, not task efficacy.
use serde_json::Value;
use std::{
    fs::File,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Owned {
    child: Child,
    root: Option<tempfile::TempDir>,
}
impl Drop for Owned {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        if std::thread::panicking()
            && let Some(root) = self.root.take()
        {
            eprintln!("Retained flow failure evidence: {}", root.keep().display());
        }
    }
}
fn run(name: &str) {
    let root = tempfile::tempdir().unwrap();
    let out = root.path().join("stdout");
    let err = root.path().join("stderr");
    let process = Command::new(env!("CARGO_BIN_EXE_infra-harness-flow-fixture"))
        .env_remove("RUST_MIN_STACK")
        .arg(root.path())
        .arg(name)
        .stdin(Stdio::null())
        .stdout(File::create(&out).unwrap())
        .stderr(File::create(&err).unwrap())
        .spawn()
        .unwrap();
    let mut child = Owned {
        child: process,
        root: Some(root),
    };
    let until = Instant::now() + Duration::from_secs(70);
    let status = loop {
        if let Some(status) = child.child.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < until,
            "Flow timeout: {}",
            std::fs::read_to_string(&err).unwrap()
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        status.success(),
        "{name}: {}",
        std::fs::read_to_string(&err).unwrap()
    );
    let value: Value = serde_json::from_slice(&std::fs::read(out).unwrap()).unwrap();
    assert_eq!(value["scenario"], name);
    assert_eq!(
        value["provider_attempts"],
        if ["cancel-source", "identity-failure"].contains(&name) {
            2
        } else {
            3
        }
    );
    assert_eq!(value["runner_reopened"], true);
    assert_eq!(value["neighbor_status"], "completed");
}
#[test]
fn serial_sessions_preserve_one_process_scope_through_cleanup() {
    run("serial");
}
#[test]
fn independent_sessions_overlap_and_resume_only_their_own_native_identity() {
    run("parallel");
}
#[test]
fn linked_worktrees_share_execution_exclusion_at_concurrency_two() {
    run("shared");
}
#[test]
fn cancelling_source_discards_pending_input_and_preserves_neighbor() {
    run("cancel-source");
}
#[test]
fn cancelling_assigned_turn_retains_uncertainty_and_preserves_neighbor() {
    run("cancel-next");
}
#[test]
fn contradictory_native_identity_blocks_continuation_without_stopping_neighbor() {
    run("identity-failure");
}
