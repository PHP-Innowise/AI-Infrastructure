//! Read-only admission uses real disposable SQLite files and a separate writer probe.
//! The child case is a support entry point, never a substitute for the parent assertions.
#![cfg(unix)]
use infra_core::Store;
use rusqlite::{Connection, ErrorCode};
use std::{
    ffi::CString,
    fs,
    os::unix::{ffi::OsStrExt, fs::symlink},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    time::{Duration, Instant},
};

const PROBE_PATH: &str = "INFRA_SQLITE_READONLY_PROBE_PATH";
const PROBE_MODE: &str = "INFRA_SQLITE_READONLY_PROBE_MODE";
const CHILD: &str = "native_sqlite_probe_child";

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "infra-readonly-state-{}",
            infra_core::domain::uuid4()
        ));
        infra_core::fs::ensure_directory(&path, true).unwrap();
        Self(path)
    }
    fn database(&self) -> PathBuf {
        self.0.join("custom unfinished state.sqlite")
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
fn probe(path: &Path, mode: &str) -> Output {
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", CHILD, "--nocapture", "--test-threads=1"])
        .env(PROBE_PATH, path)
        .env(PROBE_MODE, mode)
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
                "Native SQLite probe failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("1 passed"),
                "The exact child support case must actually run"
            );
            return output;
        }
        assert!(
            Instant::now() < deadline,
            "Native SQLite probe blocked on {mode}; no FIFO writer or provider exists"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn observed(output: &Output, marker: &str) {
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(marker),
        "Expected {marker}; native probe receipt: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn native_sqlite_probe_child() {
    let Some(path) = std::env::var_os(PROBE_PATH) else {
        return;
    };
    match std::env::var(PROBE_MODE).unwrap().as_str() {
        "writer" => {
            let connection = Connection::open(PathBuf::from(path)).unwrap();
            connection.busy_timeout(Duration::ZERO).unwrap();
            match connection.execute_batch("BEGIN IMMEDIATE") {
                Ok(()) => {
                    connection.execute_batch("ROLLBACK").unwrap();
                    println!("native-sqlite-probe-v1:writer-admitted");
                }
                Err(rusqlite::Error::SqliteFailure(error, _))
                    if matches!(
                        error.code,
                        ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
                    ) =>
                {
                    println!("native-sqlite-probe-v1:writer-busy");
                }
                Err(error) => panic!("Unexpected SQLite probe refusal: {error}"),
            }
        }
        "readonly" => {
            let result = Store::open_read_only(PathBuf::from(path), Duration::from_millis(100));
            if result.is_ok() {
                println!("native-sqlite-probe-v1:readonly-admitted");
            } else {
                println!("native-sqlite-probe-v1:readonly-refused");
            }
        }
        _ => panic!("Unknown native SQLite probe mode"),
    }
}

#[test]
fn readonly_admission_preserves_an_existing_connections_reserved_lock_and_unfinished_rows() {
    let root = Root::new();
    let database = root.database();
    let holder = Store::open(&database, Default::default()).unwrap();
    holder.conn.execute_batch(
        "INSERT INTO working_tasks(task_id,goal,progress,next_steps,files,sources,created_at,updated_at)
         VALUES('TASK-READONLY','Retain unfinished state','Manual narrative','[]','[]','[]',
                '2026-01-01T00:00:00+00:00','2026-01-01T00:00:00+00:00');
         INSERT INTO turn_deltas(task_id,created_at,files)
         VALUES('TASK-READONLY','2026-01-01T00:01:00+00:00','[\"src/Queued.php\"]');"
    ).unwrap();
    let before = serde_json::to_value(holder.working("TASK-READONLY").unwrap().unwrap()).unwrap();
    let pending_before =
        serde_json::to_value(holder.pending_turns("TASK-READONLY").unwrap()).unwrap();
    holder.conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    observed(
        &probe(&database, "writer"),
        "native-sqlite-probe-v1:writer-busy",
    );

    let reader = Store::open_read_only(&database, Duration::from_millis(100)).unwrap();
    assert!(reader.read_only);
    assert!(
        serde_json::to_value(reader.working("TASK-READONLY").unwrap().unwrap()).unwrap() == before
    );
    // This is a DIFFERENT PROCESS. An in-process Connection could trust SQLite's
    // stale lock registry and conceal a raw close that removed the OS lock.
    observed(
        &probe(&database, "writer"),
        "native-sqlite-probe-v1:writer-busy",
    );
    drop(reader);
    observed(
        &probe(&database, "writer"),
        "native-sqlite-probe-v1:writer-busy",
    );
    holder.conn.execute_batch("ROLLBACK").unwrap();
    assert!(
        serde_json::to_value(holder.working("TASK-READONLY").unwrap().unwrap()).unwrap() == before
    );
    assert!(
        serde_json::to_value(holder.pending_turns("TASK-READONLY").unwrap()).unwrap()
            == pending_before
    );
}

#[test]
fn readonly_fifo_is_refused_without_waiting_for_an_external_writer() {
    let root = Root::new();
    let fifo = root.0.join("not a database.fifo");
    let encoded = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(encoded.as_ptr(), 0o600) }, 0);
    observed(
        &probe(&fifo, "readonly"),
        "native-sqlite-probe-v1:readonly-refused",
    );
}

#[test]
fn readonly_symlink_to_a_valid_database_is_refused() {
    let root = Root::new();
    let database = root.database();
    drop(Store::open(&database, Default::default()).unwrap());
    let alias = root.0.join("redirected.sqlite");
    symlink(&database, &alias).unwrap();
    observed(
        &probe(&alias, "readonly"),
        "native-sqlite-probe-v1:readonly-refused",
    );
}

#[test]
fn readonly_wal_header_is_refused_without_creating_wal_or_shared_memory_sidecars() {
    let root = Root::new();
    let database = root.database();
    {
        let store = Store::open(&database, Default::default()).unwrap();
        let mode: String = store
            .conn
            .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
    }
    // No SQLite connection is alive here; reading fixture bytes cannot cancel
    // a retained lock. Header 2/2 persists after the last WAL connection closes.
    let header = fs::read(&database).unwrap();
    assert!(header.get(18..20) == Some(&[2, 2][..]));
    let listing = || {
        let mut names = fs::read_dir(&root.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        names.sort();
        names
    };
    let before = listing();
    assert!(!root.0.join("custom unfinished state.sqlite-wal").exists());
    assert!(!root.0.join("custom unfinished state.sqlite-shm").exists());
    observed(
        &probe(&database, "readonly"),
        "native-sqlite-probe-v1:readonly-refused",
    );
    assert!(
        listing() == before,
        "WAL refusal must not create sidecars or replace the database"
    );
}
