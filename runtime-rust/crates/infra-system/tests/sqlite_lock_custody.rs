//! Real rollback-mode system-store admission must retain other connections' locks.
#![cfg(unix)]
use infra_system::backend::Database;
use rusqlite::{Connection, ErrorCode};
use serde_json::{Value, json};
use std::os::unix::process::CommandExt;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    time::{Duration, Instant},
};

const ROOT_ENV: &str = "INFRA_SYSTEM_SQLITE_FIXTURE_ROOT";
const MODE_ENV: &str = "INFRA_SYSTEM_SQLITE_FIXTURE_MODE";
const WRITER_PATH: &str = "INFRA_SYSTEM_SQLITE_WRITER_PATH";
const SCENARIO: &str = "system_constructor_scenario_child";
const WRITER: &str = "sqlite_writer_probe_child";
const BUSY: &str = "sqlite-lock-probe-v1:busy";
const ADMITTED: &str = "sqlite-lock-probe-v1:admitted";

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "infra-system-sqlite-custody-{}",
            uuid::Uuid::new_v4()
        ));
        infra_core::fs::ensure_directory(&path, true).unwrap();
        Self(infra_core::fs::canonicalize(path).unwrap())
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct OwnedChild {
    child: Option<Child>,
    group: bool,
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            // The owned leader is still unreaped here. Its fresh process group
            // cannot be recycled before this signal, including on timeout.
            if self.group {
                let leader = i32::try_from(child.id()).unwrap();
                if leader > 0 {
                    unsafe {
                        libc::kill(-leader, libc::SIGKILL);
                    }
                }
            } else {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}
fn completed(child: Child, seconds: u64, group: bool) -> Output {
    let mut owned = OwnedChild {
        child: Some(child),
        group,
    };
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        if owned.child.as_mut().unwrap().try_wait().unwrap().is_some() {
            let output = owned.child.take().unwrap().wait_with_output().unwrap();
            assert!(
                output.stdout.len() + output.stderr.len() <= 65536,
                "Fixture output exceeded its bounded receipt size"
            );
            assert!(
                output.status.success(),
                "Fixture failed: stdout={} stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("1 passed"),
                "The exact child support case must actually execute"
            );
            return output;
        }
        assert!(
            Instant::now() < deadline,
            "Native fixture exceeded its {seconds}s hard deadline"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn assert_reapable_children() {
    let mut action = std::mem::MaybeUninit::<libc::sigaction>::uninit();
    assert_eq!(
        unsafe { libc::sigaction(libc::SIGCHLD, std::ptr::null(), action.as_mut_ptr()) },
        0
    );
    let action = unsafe { action.assume_init() };
    assert!(
        action.sa_sigaction != libc::SIG_IGN && action.sa_flags & libc::SA_NOCLDWAIT == 0,
        "The fixture requires retained, waitable child identities before group cleanup",
    );
}
fn scenario(mode: &str) {
    assert_reapable_children();
    let root = Root::new();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", SCENARIO, "--nocapture", "--test-threads=1"])
        .env(ROOT_ENV, &root.0)
        .env(MODE_ENV, mode)
        .env_remove(WRITER_PATH)
        .env("HOME", root.0.join("home"))
        .env("USERPROFILE", root.0.join("home"))
        .env("CODEX_HOME", root.0.join("codex"))
        .env("PATH", "")
        .env("RUST_BACKTRACE", "0")
        .current_dir(&root.0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .unwrap();
    let output = completed(child, 15, true);
    assert!(String::from_utf8_lossy(&output.stdout).contains("sqlite-lock-scenario-v1:complete"));
}
fn writer_is_busy(database: &Path) {
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", WRITER, "--nocapture", "--test-threads=1"])
        .env(WRITER_PATH, database)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let output = completed(child, 2, false);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(BUSY),
        "Held rollback reservation was lost: {stdout}"
    );
    assert!(
        !stdout.contains(ADMITTED),
        "An independent writer acquired the held reservation"
    );
}
#[test]
fn sqlite_writer_probe_child() {
    let Some(path) = std::env::var_os(WRITER_PATH) else {
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
fn query_rows(connection: &Connection, sql: &str, width: usize) -> Vec<Vec<String>> {
    let mut statement = connection.prepare(sql).unwrap();
    statement
        .query_map([], |row| {
            (0..width)
                .map(|index| row.get::<_, String>(index))
                .collect()
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}
fn schema(connection: &Connection) -> Vec<Vec<String>> {
    query_rows(
        connection,
        "SELECT type,name,tbl_name,coalesce(sql,'') FROM sqlite_master ORDER BY type,name",
        4,
    )
}

fn snapshot(connection: &Connection, refused: bool) -> Value {
    let table = if refused {
        "backing_runs"
    } else {
        "system_runs"
    };
    json!({
        "schema": schema(connection),
        "runs": query_rows(connection,
            &format!("SELECT id,data,coalesce(project_id,'') FROM {table} ORDER BY id"), 3),
        "edits": query_rows(connection, "SELECT id,data FROM system_edits ORDER BY id", 2),
        "turns": query_rows(connection, "SELECT task_id,files FROM buffered_turns ORDER BY rowid", 2),
    })
}
#[test]
fn system_constructor_scenario_child() {
    let Some(root) = std::env::var_os(ROOT_ENV) else {
        return;
    };
    let root = PathBuf::from(root);
    let mode = std::env::var(MODE_ENV).unwrap();
    assert!(["ordinary", "refused-view"].contains(&mode.as_str()));
    let refused = mode == "refused-view";
    let state = root.join("private system state");
    infra_core::fs::ensure_directory(&state, true).unwrap();
    let database = state.join("sessions.sqlite3");
    let holder = Connection::open(&database).unwrap();
    holder.busy_timeout(Duration::ZERO).unwrap();
    holder
        .execute_batch(
            "PRAGMA journal_mode=DELETE;
         CREATE TABLE system_edits(id TEXT PRIMARY KEY,data TEXT NOT NULL);
         CREATE TABLE buffered_turns(task_id TEXT NOT NULL,files TEXT NOT NULL);
         INSERT INTO system_edits VALUES('retained-edit','{\"status\":\"awaiting-review\"}');
         INSERT INTO buffered_turns VALUES('retained-task','[\"src/Unfinished.php\"]');",
        )
        .unwrap();
    if refused {
        holder.execute_batch(
            "CREATE TABLE backing_runs(id TEXT PRIMARY KEY,data TEXT NOT NULL,project_id TEXT);
             INSERT INTO backing_runs VALUES('retained-run','{\"status\":\"awaiting-review\"}','fixture');
             CREATE VIEW system_runs AS SELECT id,data,project_id FROM backing_runs;"
        ).unwrap();
        // The final index statement is invalid for this view. Admission must
        // not release a reservation even when the public constructor refuses.
        assert!(
            holder
                .prepare("CREATE INDEX system_runs_project ON system_runs(project_id)")
                .is_err()
        );
    } else {
        holder.execute_batch(
            "CREATE TABLE system_runs(id TEXT PRIMARY KEY,data TEXT NOT NULL,project_id TEXT);
             CREATE INDEX system_runs_project ON system_runs(project_id);
             INSERT INTO system_runs VALUES('retained-run','{\"status\":\"awaiting-review\"}','fixture');"
        ).unwrap();
    }
    let journal: String = holder
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal, "delete", "This case does not make a WAL SHM claim");
    let before = snapshot(&holder, refused);
    holder
        .execute_batch(
            "BEGIN IMMEDIATE;
         INSERT INTO buffered_turns VALUES('pending-task','[\"src/Pending.php\"]');",
        )
        .unwrap();
    let pending = snapshot(&holder, refused);
    writer_is_busy(&database);
    let backend = if refused {
        let error = Database::new(&state)
            .err()
            .expect("A view cannot acquire a table index");
        assert_eq!(error.kind, infra_core::ErrorKind::Unavailable);
        assert!(
            error.code.is_none(),
            "The fixture must reach its ordinary SQLite schema refusal"
        );
        None
    } else {
        let backend = Database::new(&state).expect("An already-current schema needs no write");
        assert_eq!(
            backend.get("system_runs", "retained-run").unwrap().unwrap()["status"],
            "awaiting-review",
        );
        Some(backend)
    };
    writer_is_busy(&database);
    drop(backend);
    writer_is_busy(&database);
    assert!(
        snapshot(&holder, refused) == pending,
        "Admission changed the live transaction image"
    );
    holder.execute_batch("ROLLBACK").unwrap();
    assert!(
        snapshot(&holder, refused) == before,
        "Unfinished committed state was changed"
    );
    let integrity: String = holder
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    println!("sqlite-lock-scenario-v1:complete");
}
#[test]
fn system_constructor_keeps_an_existing_rollback_reservation_and_unfinished_rows() {
    scenario("ordinary");
}
#[test]
fn refused_system_constructor_keeps_an_existing_rollback_reservation_and_unfinished_rows() {
    scenario("refused-view");
}
