//! Eval diagnostics must not cancel another connection's POSIX reservation.
//! Every database is disposable; the lock witness is a separate native process.
#![cfg(unix)]
use infra_core::Store;
use infra_eval::{diagnostic, native};
use rusqlite::{Connection, ErrorCode};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const PROBE_PATH: &str = "INFRA_EVAL_SQLITE_PROBE_PATH";
const CHILD: &str = "native_sqlite_writer_probe_child";
const BUSY: &str = "eval-sqlite-probe-v1:writer-busy";
const ADMITTED: &str = "eval-sqlite-probe-v1:writer-admitted";
const TASK: &str = "TASK-EVAL-READONLY";

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "infra-eval-readonly-{}",
            infra_core::domain::uuid4()
        ));
        infra_core::fs::ensure_directory(&path, true).unwrap();
        Self(path)
    }
    fn database(&self) -> PathBuf {
        self.0.join("memory-bank/local/context.db")
    }
    fn store(&self) -> Store {
        let store = Store::open(self.database(), Default::default()).unwrap();
        store.conn.execute_batch(
            "INSERT INTO working_tasks(task_id,goal,progress,next_steps,files,sources,created_at,updated_at)
             VALUES('TASK-EVAL-READONLY','Retain unfinished evaluation state','Manual narrative','[]','[]','[]',
                    '2026-01-01T00:00:00+00:00','2026-01-01T00:00:00+00:00');
             INSERT INTO turn_deltas(task_id,created_at,files)
             VALUES('TASK-EVAL-READONLY','2026-01-01T00:01:00+00:00','[\"src/Queued.php\"]');"
        ).unwrap();
        store
    }
    fn diagnostic_manifest(&self) -> String {
        let schema = self
            .0
            .join("project-brain/schemas/retrieval-manifest.schema.json");
        fs::create_dir_all(schema.parent().unwrap()).unwrap();
        fs::write(
            &schema,
            include_bytes!(
                "../../../../PHP Core/project-brain/schemas/retrieval-manifest.schema.json"
            ),
        )
        .unwrap();
        let id = infra_core::domain::uuid4();
        let reference = format!("memory-bank/local/retrieval-manifests/{id}.json");
        let path = self.0.join(&reference);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let manifest = json!({
            "schema_version": 1,
            "id": id,
            "created_at": "2026-01-01T00:00:00+00:00",
            "query": "Locate the retained unfinished checkpoint",
            "task_id": null,
            "task_revision": null,
            "filters": {
                "privacy": [], "owners": [], "authority": [],
                "freshness": true, "active_only": true
            },
            "selected": [],
            "excluded": [],
            "token_estimates": {
                "policy": 0, "handoff": 0, "durable": 0, "dynamic": 0, "evidence": 0,
                "total": 0, "target": 8000, "hard": 12000
            },
            "provider": "local",
            "escalation_reason": null
        });
        let roots = infra_core::Roots::installed(&self.0).unwrap();
        infra_core::domain::validate_schema_file(
            &roots,
            "retrieval-manifest.schema.json",
            &manifest,
        )
        .unwrap();
        fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        fs::create_dir_all(self.0.join("docs")).unwrap();
        fs::write(
            self.0.join("docs/lock-witness.md"),
            "The diagnostic source exists and has not been indexed.\n",
        )
        .unwrap();
        reference
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Probe(Option<Child>);
impl Drop for Probe {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
fn writer_is_busy(database: &Path) {
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", CHILD, "--nocapture", "--test-threads=1"])
        .env(PROBE_PATH, database)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut owned = Probe(Some(child));
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if owned.0.as_mut().unwrap().try_wait().unwrap().is_some() {
            let output = owned.0.take().unwrap().wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "The native writer probe failed: {}",
                String::from_utf8_lossy(&output.stderr),
            );
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                stdout.contains("1 passed"),
                "The exact probe must actually execute"
            );
            assert!(
                stdout.contains(BUSY),
                "The held reservation was lost; probe: {stdout}"
            );
            assert!(
                !stdout.contains(ADMITTED),
                "The separate writer unexpectedly acquired admission"
            );
            return;
        }
        assert!(
            Instant::now() < deadline,
            "The native writer probe exceeded its hard deadline"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn snapshot(store: &Store) -> Value {
    json!({
        "working": store.working(TASK).unwrap(),
        "turns": store.pending_turns(TASK).unwrap(),
    })
}

#[test]
fn native_sqlite_writer_probe_child() {
    let Some(path) = std::env::var_os(PROBE_PATH) else {
        return;
    };
    let connection = Connection::open(PathBuf::from(path)).unwrap();
    connection.busy_timeout(Duration::ZERO).unwrap();
    match connection.execute_batch("BEGIN IMMEDIATE") {
        Ok(()) => {
            connection.execute_batch("ROLLBACK").unwrap();
            println!("{ADMITTED}");
        }
        Err(rusqlite::Error::SqliteFailure(error, _))
            if matches!(
                error.code,
                ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
            ) =>
        {
            println!("{BUSY}");
        }
        Err(error) => panic!("Unexpected native writer refusal: {error}"),
    }
}

#[test]
fn pending_preserves_existing_reservation_and_unfinished_rows() {
    let root = Root::new();
    let holder = root.store();
    let before = snapshot(&holder);
    holder.conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    writer_is_busy(&root.database());
    let result = native::pending(&root.0).unwrap();
    assert_eq!(result["turn_deltas"], 1);
    assert_eq!(result["turn_task_ids"], json!([TASK]));
    assert_eq!(result["unresolved_requests"], 0);
    assert_eq!(result["queued_requests"], 0);
    writer_is_busy(&root.database());
    holder.conn.execute_batch("ROLLBACK").unwrap();
    assert!(
        snapshot(&holder) == before,
        "Eval pending must preserve the unfinished task and turn"
    );
}

#[test]
fn diagnostic_audit_preserves_existing_reservation_after_reaching_healthy_database_path() {
    let root = Root::new();
    let holder = root.store();
    let reference = root.diagnostic_manifest();
    let paths = vec!["docs/lock-witness.md".to_string()];
    let passages = BTreeMap::new();
    let before = snapshot(&holder);
    holder.conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    writer_is_busy(&root.database());
    let result = diagnostic::audit(diagnostic::Audit {
        root: &root.0,
        manifest: Some(&reference),
        paths: &paths,
        query_withheld: false,
        passages: &passages,
        capsule: "",
        history: None,
        seed_git: false,
    })
    .unwrap();
    // These witnesses exclude manifest/schema/early-guard failure as the cause.
    assert_eq!(result["status"], "ok");
    assert_eq!(result["items"][0]["path"], paths[0]);
    assert_eq!(result["items"][0]["stage"], "not-indexed");
    assert_eq!(result["items"][0]["answer"], "not-applicable");
    writer_is_busy(&root.database());
    holder.conn.execute_batch("ROLLBACK").unwrap();
    assert!(
        snapshot(&holder) == before,
        "Eval audit must preserve the unfinished task and turn"
    );
}
