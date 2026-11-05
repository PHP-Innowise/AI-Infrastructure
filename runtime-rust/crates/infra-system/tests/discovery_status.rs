//! A Fill with AI scan that stops without a proposal says why, and the sandbox
//! preflight names the condition it refused.
use infra_core::{Result, RuntimeError};
use infra_system::{Project, SessionBridge, SystemManager, workers::probe_failure};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    path::PathBuf,
    sync::Arc,
};

const RUN: &str = "0123456789abcdef0123456789abcdef";
const NONCE: &str = "fedcba9876543210fedcba9876543210";

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "infra-system-discovery-status-{}",
            uuid::Uuid::new_v4()
        ));
        infra_core::fs::ensure_directory(&root, true).unwrap();
        Self(infra_core::fs::canonicalize(root).unwrap())
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Bridge {
    root: PathBuf,
    status: &'static str,
    events: Vec<Value>,
}
fn unused<T>() -> Result<T> {
    Err(RuntimeError::unavailable("Unexpected fixture operation"))
}
impl SessionBridge for Bridge {
    fn projects(&self) -> Result<BTreeMap<String, Project>> {
        Ok(BTreeMap::from([(
            "fixture".into(),
            Project {
                id: "fixture".into(),
                name: "Fixture".into(),
                path: self.root.clone(),
            },
        )]))
    }
    fn provider(&self, _: &str) -> Result<Option<PathBuf>> {
        unused()
    }
    fn active(&self) -> Result<bool> {
        Ok(false)
    }
    fn admit(&self) -> Result<()> {
        unused()
    }
    fn runner_lock(&self) -> Result<Arc<File>> {
        unused()
    }
    fn launch(&self, _: &Value, _: Option<&Value>, _: Option<&Value>) -> Result<Value> {
        unused()
    }
    fn session(&self, id: &str) -> Result<Value> {
        Ok(json!({"id":id,"provider":"codex","status":self.status,
            "system_discovery":{"run_id":RUN,"nonce":NONCE}}))
    }
    fn cancel(&self, _: &str) -> Result<Value> {
        unused()
    }
    fn events(&self, _: &str, _: &[&str]) -> Result<Vec<Value>> {
        Ok(self.events.clone())
    }
    fn find_launch(&self, _: &str, _: &str) -> Result<Option<Value>> {
        Ok(None)
    }
}

fn scan(status: &'static str, events: Vec<Value>) -> Value {
    let root = Root::new();
    let state = root.0.join("state");
    let bridge = Arc::new(Bridge {
        root: root.0.clone(),
        status,
        events,
    });
    let manager = SystemManager::new(root.0.clone(), state.clone(), bridge).unwrap();
    let directory = state.join("system-discovery").join(RUN);
    infra_core::fs::ensure_directory(&directory, true).unwrap();
    infra_core::fs::atomic_write(
        &directory.join(format!("request-{NONCE}.json")),
        json!({"nonce":NONCE,"editor":{"project_id":"fixture","config_path":"system.json","services":[]},"roots":{}})
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    manager
        .execute("discovery.get", &json!({"params":{"id":"scan-session"}}))
        .unwrap()
}

#[test]
fn a_failed_scan_reports_the_error_that_stopped_it() {
    let job = scan(
        "failed",
        vec![
            json!({"kind":"status","text":"Checking that the sandbox keeps original service folders unreadable."}),
            json!({"kind":"error","text":"AI discovery failed: The AI discovery sandbox check failed: The sandbox workspace is not writable"}),
        ],
    );
    assert_eq!(job["status"], "failed");
    assert_eq!(
        job["error"],
        "The AI discovery sandbox check failed: The sandbox workspace is not writable"
    );
    assert!(job["proposal"].is_null());
}

#[test]
fn a_running_scan_has_no_error_even_after_a_reported_one() {
    let job = scan(
        "running",
        vec![json!({"kind":"error","text":"transient provider warning"})],
    );
    assert_eq!(job["active"], true);
    assert!(job.get("error").is_none());
}

#[test]
fn a_cancelled_scan_without_error_events_reports_none() {
    let job = scan(
        "cancelled",
        vec![json!({"kind":"status","text":"AI is inspecting captured service evidence."})],
    );
    assert!(job.get("error").is_none());
}

#[test]
fn the_preflight_names_a_readable_original_path() {
    let stdout = br#"{"ok":false,"error":"The sandbox could read an original file or folder: /srv/orders/src/order.rs"}"#;
    let message = probe_failure(stdout, 1);
    assert!(
        message
            .starts_with("The AI discovery sandbox did not keep the original folders unreadable."),
        "{message}"
    );
    assert!(message.contains("/srv/orders/src/order.rs"), "{message}");
}

#[test]
fn the_preflight_names_its_own_failure() {
    let stdout = b"noise\n{\"ok\":false,\"error\":\"The sandbox workspace is not writable (/w/.harness-tmp/probe-1): No such file or directory (os error 2)\"}\n";
    assert_eq!(
        probe_failure(stdout, 1),
        "The AI discovery sandbox check failed: The sandbox workspace is not writable (/w/.harness-tmp/probe-1): No such file or directory (os error 2)"
    );
}

#[test]
fn a_preflight_that_never_ran_says_the_sandbox_could_not_start() {
    let message = probe_failure(b"", 1);
    assert!(
        message.starts_with(
            "The AI discovery sandbox could not start its boundary check (exit code 1)."
        ),
        "{message}"
    );
    assert!(message.contains("bubblewrap"), "{message}");
}
