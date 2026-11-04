//! Loaded run lists must retain the same durable launch recovery as single reads.
use infra_core::{Result, RuntimeError};
use infra_system::{Project, SessionBridge, SystemManager, backend::Database};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    path::PathBuf,
    sync::{Arc, Mutex},
};

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "infra-system-manager-recovery-{}",
            uuid::Uuid::new_v4()
        ));
        infra_core::fs::ensure_directory(&root, true).unwrap();
        Self(fs::canonicalize(root).unwrap())
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Bridge {
    root: PathBuf,
    recovered: BTreeMap<(String, String), Value>,
    lookups: Mutex<Vec<(String, String)>>,
}
impl Bridge {
    fn new(root: &Root) -> Self {
        Self {
            root: root.0.clone(),
            recovered: BTreeMap::new(),
            lookups: Mutex::new(vec![]),
        }
    }
    fn recover(&mut self, run: &str, nonce: &str, session: &str) {
        self.recovered
            .insert((run.into(), nonce.into()), json!({"id":session}));
    }
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
        Ok(json!({"id":id,"provider":"codex","status":"completed"}))
    }
    fn cancel(&self, _: &str) -> Result<Value> {
        unused()
    }
    fn events(&self, _: &str, _: &[&str]) -> Result<Vec<Value>> {
        unused()
    }
    fn find_launch(&self, run: &str, nonce: &str) -> Result<Option<Value>> {
        let key = (run.to_string(), nonce.to_string());
        self.lookups.lock().unwrap().push(key.clone());
        Ok(self.recovered.get(&key).cloned())
    }
}

fn id(index: u32) -> String {
    format!("{index:032x}")
}
fn record(index: u32) -> Value {
    json!({"id":id(index),"project_id":"fixture","pending_nonce":null,
        "session_id":null,"launches":[]})
}
fn manager(root: &Root, bridge: Arc<Bridge>) -> SystemManager {
    SystemManager::new(root.0.clone(), root.0.join("state"), bridge).unwrap()
}
fn list(manager: &SystemManager) -> Result<Value> {
    manager.execute("systems.list", &json!({"query":{"project_id":"fixture"}}))
}

#[test]
fn startup_recovers_loaded_pending_launches_once_and_persists_them() {
    let root = Root::new();
    let mut bridge = Bridge::new(&root);
    bridge.recover(&id(1), "found", "new-session");
    bridge.recover(&id(3), "duplicate", "new-session");
    let bridge = Arc::new(bridge);
    {
        let db = Database::new(&root.0.join("state")).unwrap();
        let mut found = record(1);
        found["pending_nonce"] = json!("found");
        found["session_id"] = json!("prior-session");
        db.put("system_runs", &id(1), &found).unwrap();
        let mut missing = record(2);
        missing["pending_nonce"] = json!("missing");
        missing["session_id"] = json!("prior-session");
        db.put("system_runs", &id(2), &missing).unwrap();
        let mut duplicate = record(3);
        duplicate["pending_nonce"] = json!("duplicate");
        duplicate["session_id"] = json!("new-session");
        duplicate["launches"] = json!(["prior-session", "new-session"]);
        db.put("system_runs", &id(3), &duplicate).unwrap();
    }
    {
        let manager = manager(&root, bridge.clone());
        let found = manager
            .state
            .db
            .get("system_runs", &id(1))
            .unwrap()
            .unwrap();
        assert!(found["pending_nonce"].is_null());
        assert_eq!(found["session_id"], "new-session");
        assert_eq!(found["launches"], json!(["prior-session", "new-session"]));
        let missing = manager
            .state
            .db
            .get("system_runs", &id(2))
            .unwrap()
            .unwrap();
        assert!(missing["pending_nonce"].is_null());
        assert_eq!(missing["session_id"], "prior-session");
        assert_eq!(missing["launches"], json!([]));
        let duplicate = manager
            .state
            .db
            .get("system_runs", &id(3))
            .unwrap()
            .unwrap();
        assert!(duplicate["pending_nonce"].is_null());
        assert_eq!(
            duplicate["launches"],
            json!(["prior-session", "new-session"])
        );
    }
    let restarted = manager(&root, bridge.clone());
    assert_eq!(
        list(&restarted).unwrap()["runs"].as_array().unwrap().len(),
        3
    );
    assert_eq!(bridge.lookups.lock().unwrap().len(), 3);
}

#[test]
fn list_recovers_loaded_rows_and_keeps_project_filter_order_and_limit() {
    let root = Root::new();
    let mut bridge = Bridge::new(&root);
    bridge.recover(&id(203), "found", "new-session");
    let bridge = Arc::new(bridge);
    let manager = manager(&root, bridge.clone());
    for index in 1..=203 {
        let mut value = record(index);
        if index == 203 {
            value["pending_nonce"] = json!("found");
            value["session_id"] = json!("prior-session");
        }
        manager
            .state
            .db
            .put("system_runs", &id(index), &value)
            .unwrap();
    }
    let mut other_project = record(204);
    other_project["project_id"] = json!("another-project");
    manager
        .state
        .db
        .put("system_runs", &id(204), &other_project)
        .unwrap();
    let output = list(&manager).unwrap();
    let runs = output["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 200);
    assert_eq!(runs.first().unwrap()["id"], id(203));
    assert_eq!(runs.last().unwrap()["id"], id(4));
    assert!(runs.iter().all(|run| run["project_id"] == "fixture"));
    assert!(runs[0]["pending_nonce"].is_null());
    assert_eq!(runs[0]["session_id"], "new-session");
    assert_eq!(runs[0]["launches"], json!(["prior-session", "new-session"]));
    let persisted = manager
        .state
        .db
        .get("system_runs", &id(203))
        .unwrap()
        .unwrap();
    assert!(persisted["pending_nonce"].is_null());
    assert_eq!(persisted["session_id"], "new-session");
    list(&manager).unwrap();
    assert_eq!(bridge.lookups.lock().unwrap().len(), 1);
}

#[test]
fn loaded_records_still_validate_database_keys_before_recovery() {
    let root = Root::new();
    let bridge = Arc::new(Bridge::new(&root));
    let manager = manager(&root, bridge.clone());
    let mut value = record(1);
    value["pending_nonce"] = json!("must-not-recover");
    manager
        .state
        .db
        .put("system_runs", "invalid-key", &value)
        .unwrap();
    assert_eq!(
        list(&manager).unwrap_err().to_string(),
        "Invalid system run ID"
    );
    drop(manager);
    let restarted = SystemManager::new(root.0.clone(), root.0.join("state"), bridge.clone());
    assert!(matches!(restarted, Err(error) if error.to_string() == "Invalid system run ID"));
    assert!(bridge.lookups.lock().unwrap().is_empty());
}

#[test]
fn malformed_stored_json_remains_an_error_for_list_and_startup() {
    let root = Root::new();
    let bridge = Arc::new(Bridge::new(&root));
    let manager = manager(&root, bridge.clone());
    manager
        .state
        .db
        .put("system_runs", &id(1), &record(1))
        .unwrap();
    {
        let db = rusqlite::Connection::open(root.0.join("state/sessions.sqlite3")).unwrap();
        db.execute(
            "UPDATE system_runs SET data = ?1 WHERE id = ?2",
            rusqlite::params![
                r#"{"pending_nonce":"first","pending_nonce":"second"}"#,
                id(1)
            ],
        )
        .unwrap();
    }
    assert!(list(&manager).is_err());
    drop(manager);
    assert!(SystemManager::new(root.0.clone(), root.0.join("state"), bridge.clone()).is_err());
    assert!(bridge.lookups.lock().unwrap().is_empty());
}
