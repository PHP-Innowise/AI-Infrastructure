//! Phase 2C Wave 5B Part 1 integration tests:
//! - Native tree-sitter SQLite Code Graph IR fallback when `graphify` CLI is absent
//! - Preservation of explicit missing binary `state == "missing"`
//! - Non-destructive rebuilds (previous `live/graph.json` never deleted before replacement)
//! - Incremental `record_live_paths` updates and deletions + `graph_map` IR metadata
//! - Lifecycle-safe debounced `notify` FS watcher (`enable` -> auto-update -> `disable`/`delete`/`close`)

use infra_harness::{
    graph::{Graphs, Shown, live},
    store::SessionStore,
};
use rusqlite::Connection;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

struct TempWorkspace {
    root: PathBuf,
}

impl TempWorkspace {
    fn new(name: &str) -> Self {
        let unique = format!(
            "infra-phase2c-ir-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        );
        let root = std::env::temp_dir().join(unique);
        fs::create_dir_all(&root).expect("create temp workspace");
        Self { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn write_file(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(path, body).expect("write workspace file");
}

fn wait_until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if cond() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(40));
    }
}

fn wait_for_ready(graphs: &Graphs, project_id: &str, timeout: Duration) -> Value {
    let deadline = Instant::now() + timeout;
    loop {
        let status = graphs.status(project_id, false).expect("read graph status");
        let state = status["state"].as_str().unwrap_or("");
        if state == "ready" || state == "failed" {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for graph build, last status: {status}"
        );
        thread::sleep(Duration::from_millis(40));
    }
}

#[test]
fn native_ir_auto_starts_populates_sqlite_and_exposes_map_metadata() {
    let tmp = TempWorkspace::new("native-build");
    let state_dir = tmp.path().join("state");
    let workspace = tmp.path().join("repo");
    fs::create_dir_all(&state_dir).unwrap();
    fs::create_dir_all(&workspace).unwrap();

    write_file(
        &workspace,
        "app/Models/Invoice.php",
        r#"<?php
namespace App\Models;

class Invoice
{
    public function total(): int
    {
        return 100;
    }
}
"#,
    );
    write_file(
        &workspace,
        "app/Services/BillingService.php",
        r#"<?php
namespace App\Services;

use App\Models\Invoice;

class BillingService
{
    public function chargeInvoice(Invoice $invoice): int
    {
        return $invoice->total();
    }
}
"#,
    );
    write_file(
        &workspace,
        "routes/api.php",
        r#"<?php
Route::get('/invoices', [\App\Services\BillingService::class, 'chargeInvoice']);
"#,
    );

    let store = SessionStore::open(&state_dir).expect("open store");
    let project = store.register(&workspace).expect("register workspace");
    let project_id = project["id"].as_str().expect("project id").to_owned();

    let graphs = Graphs::with_native_backend(store.clone(), Duration::ZERO);
    graphs.auto_start(&project_id).expect("auto_start native");

    let status = wait_for_ready(&graphs, &project_id, Duration::from_secs(15));
    assert_eq!(status["state"], "ready", "unexpected status: {status}");
    assert_eq!(
        status["message"],
        "Code graph & dependency map ready (native tree-sitter SQLite IR)."
    );
    assert_eq!(status["backend"], "native-tree-sitter-ir");
    assert_eq!(status["tool"]["available"], true);
    assert_eq!(status["tool"]["backend"], "native-tree-sitter-ir");
    assert_eq!(status["graph"]["backend"], "native-tree-sitter-ir");

    let ir_summary = &status["graph"]["ir_summary"];
    assert_eq!(ir_summary["files"].as_u64(), Some(3));
    assert!(
        ir_summary["symbols"].as_u64().unwrap_or(0) >= 3,
        "expected >= 3 symbols, got {ir_summary}"
    );
    assert!(
        ir_summary["edges"].as_u64().unwrap_or(0) >= 1,
        "expected >= 1 dependency edge, got {ir_summary}"
    );

    let live_graph = live(&store.state, &project_id);
    assert!(live_graph.is_file(), "live/graph.json must exist");
    let ir_db = store
        .state
        .join("graphs")
        .join(&project_id)
        .join("graph_ir.sqlite3");
    assert!(ir_db.is_file(), "graph_ir.sqlite3 must exist");

    let conn = Connection::open(&ir_db).expect("open ir sqlite");
    let file_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ir_files", [], |r| r.get(0))
        .unwrap();
    let sym_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ir_symbols", [], |r| r.get(0))
        .unwrap();
    assert_eq!(file_count, 3);
    assert!(sym_count >= 3);

    let map = graphs
        .map(
            &project_id,
            None,
            &Shown {
                patterns: true,
                ..Default::default()
            },
        )
        .expect("render map");
    assert_eq!(map["backend"], "native-tree-sitter-ir");
    assert_eq!(map["ir_summary"]["files"].as_u64(), Some(3));

    let found = graphs
        .find(&project_id, None, "chargeInvoice", false)
        .expect("find symbol");
    let rows = found["rows"].as_array().expect("rows array");
    assert!(
        rows.iter().any(|row| row["label"]
            .as_str()
            .is_some_and(|s| s.contains("chargeInvoice"))
            || row["name"]
                .as_str()
                .is_some_and(|s| s.contains("chargeInvoice"))),
        "expected chargeInvoice in find results: {found}"
    );

    graphs.close();
}

#[test]
fn explicit_missing_binary_preserves_missing_state() {
    let tmp = TempWorkspace::new("explicit-missing");
    let state_dir = tmp.path().join("state");
    let workspace = tmp.path().join("repo");
    fs::create_dir_all(&state_dir).unwrap();
    fs::create_dir_all(&workspace).unwrap();
    write_file(&workspace, "src/lib.rs", "pub fn ping() -> u32 { 1 }\n");

    let store = SessionStore::open(&state_dir).expect("open store");
    let project = store.register(&workspace).expect("register workspace");
    let project_id = project["id"].as_str().expect("project id").to_owned();

    let graphs = Graphs::with(
        store,
        Some(PathBuf::from("/nonexistent/graphify-missing")),
        Duration::ZERO,
    );
    let status = graphs.act(&project_id, "enable").expect("enable");
    assert_eq!(status["state"], "missing", "unexpected status: {status}");
    graphs.close();
}

#[test]
fn record_live_paths_and_non_destructive_rebuild_keep_live_graph_intact() {
    let tmp = TempWorkspace::new("incremental-rebuild");
    let state_dir = tmp.path().join("state");
    let workspace = tmp.path().join("repo");
    fs::create_dir_all(&state_dir).unwrap();
    fs::create_dir_all(&workspace).unwrap();

    write_file(
        &workspace,
        "src/alpha.rs",
        "pub fn alpha_fn() -> i32 { 1 }\n",
    );
    write_file(&workspace, "src/beta.rs", "pub fn beta_fn() -> i32 { 2 }\n");

    let store = SessionStore::open(&state_dir).expect("open store");
    let project = store.register(&workspace).expect("register workspace");
    let project_id = project["id"].as_str().expect("project id").to_owned();

    let graphs = Graphs::with_native_backend(store.clone(), Duration::ZERO);
    graphs.auto_start(&project_id).expect("auto_start");
    let status = wait_for_ready(&graphs, &project_id, Duration::from_secs(15));
    assert_eq!(status["state"], "ready");

    // Incremental addition + modification via record_live_paths
    write_file(
        &workspace,
        "src/gamma.rs",
        "pub fn gamma_new() -> i32 { 3 }\n",
    );
    graphs.record_live_paths(&project_id, &["src/gamma.rs".to_owned()]);

    let found_gamma = graphs.find(&project_id, None, "gamma_new", false).unwrap();
    assert!(
        found_gamma["found"].as_u64().unwrap_or(0) >= 1,
        "expected gamma_new after record_live_paths: {found_gamma}"
    );

    // Incremental deletion via record_live_paths
    fs::remove_file(workspace.join("src/beta.rs")).unwrap();
    graphs.record_live_paths(&project_id, &["src/beta.rs".to_owned()]);
    let found_beta = graphs.find(&project_id, None, "beta_fn", false).unwrap();
    assert_eq!(
        found_beta["found"].as_u64(),
        Some(0),
        "deleted file symbols must disappear after record_live_paths: {found_beta}"
    );

    // Verify live/graph.json is NEVER missing during rebuild.
    let live_path = live(&store.state, &project_id);
    assert!(live_path.is_file());
    let stop_probe = Arc::new(AtomicBool::new(false));
    let missing_observed = Arc::new(AtomicBool::new(false));
    let probe_stop = Arc::clone(&stop_probe);
    let probe_missing = Arc::clone(&missing_observed);
    let probe_path = live_path.clone();
    let probe = thread::spawn(move || {
        while !probe_stop.load(Ordering::Acquire) {
            if !probe_path.is_file() {
                probe_missing.store(true, Ordering::Release);
            }
            thread::sleep(Duration::from_millis(2));
        }
    });

    let _ = graphs.act(&project_id, "rebuild").expect("rebuild");
    let rebuilt = wait_for_ready(&graphs, &project_id, Duration::from_secs(15));
    stop_probe.store(true, Ordering::Release);
    probe.join().unwrap();

    assert_eq!(rebuilt["state"], "ready");
    assert!(
        !missing_observed.load(Ordering::Acquire),
        "live/graph.json must never be deleted during rebuild"
    );

    // Clean lifecycle teardown across disable / delete / close
    let disabled = graphs.act(&project_id, "disable").expect("disable");
    assert_eq!(disabled["state"], "off");

    let deleted = graphs.act(&project_id, "delete").expect("delete");
    assert_eq!(deleted["state"], "off");
    assert!(!store.state.join("graphs").join(&project_id).exists());

    graphs.close();
}

#[test]
fn notify_watcher_tracks_disk_edits_and_shuts_down_cleanly() {
    let tmp = TempWorkspace::new("watcher-sync");
    let state_dir = tmp.path().join("state");
    let workspace = tmp.path().join("repo");
    fs::create_dir_all(&state_dir).unwrap();
    fs::create_dir_all(&workspace).unwrap();

    write_file(&workspace, "src/main.rs", "fn main() {}\n");

    let store = SessionStore::open(&state_dir).expect("open store");
    let project = store.register(&workspace).expect("register workspace");
    let project_id = project["id"].as_str().expect("project id").to_owned();

    let graphs = Graphs::with_native_backend(store.clone(), Duration::ZERO);
    graphs.auto_start(&project_id).expect("auto_start");
    let status = wait_for_ready(&graphs, &project_id, Duration::from_secs(15));
    assert_eq!(status["state"], "ready");

    // Edit workspace directly on disk and let the debounced notify watcher sync IR + live/graph.json.
    write_file(
        &workspace,
        "src/watched.rs",
        "pub fn watched_symbol_phase2c() -> bool { true }\n",
    );

    let detected = wait_until(Duration::from_secs(5), || {
        graphs
            .find(&project_id, None, "watched_symbol_phase2c", false)
            .ok()
            .and_then(|v| v["found"].as_u64())
            .unwrap_or(0)
            >= 1
    });
    assert!(
        detected,
        "debounced notify watcher should update IR + live/graph.json for src/watched.rs"
    );

    let ir_db = store
        .state
        .join("graphs")
        .join(&project_id)
        .join("graph_ir.sqlite3");
    let conn = Connection::open(&ir_db).expect("open ir db");
    let indexed_watched: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM ir_files WHERE path = 'src/watched.rs'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(indexed_watched, 1);

    graphs.close();
}
