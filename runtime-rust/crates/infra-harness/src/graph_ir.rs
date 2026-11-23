//! Per-project incremental SQLite Code Graph IR backed by 14-language tree-sitter AST extraction.
//!
//! Stores per-file metadata, extracted declarations (`ir_symbols`), resolved cross-file
//! dependency edges (`ir_edges`), framework/database semantic edges (`ir_semantic_edges`),
//! re-exports (`ir_reexports`), and O(1) counters (`ir_summary`) under
//! `<state>/graphs/<project>/graph_ir.sqlite3`.

use crate::{
    HarnessError, Result,
    graph::file_types::{IGNORED_CODE_DIRS, is_minified_js},
    graph_ast::{
        self, AstFileExtraction, AstGrammar, AstRef, is_graph_architecture_path,
        is_graph_manifest_path, normalize_route_contract,
    },
    graph_crates::CratePackageMap,
    graph_deps::{self, DeclaredMember, DepMemberRef, DepRef, ReexportEntry, ReexportIndex},
    graph_live::clean_rel_key,
    graph_manifests::ManifestIndex,
    graph_map::is_test_path,
    store,
};
use infra_core::bank::publication::PublicationFiles;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    fs::{self, File, OpenOptions},
    io::{BufWriter, Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, UNIX_EPOCH},
};

pub(crate) const NATIVE_IR_TOOL: &str = "native-tree-sitter-ir";
const SCHEMA_VERSION: &str = "3";
const DB_FILENAME: &str = "graph_ir.sqlite3";
pub(crate) const MAX_IR_FILES: usize = 30_000;
const MAX_WALK_DEPTH: usize = 64;
const MAX_FILE_BYTES: u64 = crate::graph_map::SOURCE_LIMIT;
const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(10);

fn default_max_ir_files() -> usize {
    MAX_IR_FILES
}

/// Fast O(1) summary counters stored in `ir_summary`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IrSummary {
    pub files: u64,
    pub symbols: u64,
    pub edges: u64,
    pub semantic_edges: u64,
    pub updated_at: String,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default = "default_max_ir_files")]
    pub max_files: usize,
}

impl Default for IrSummary {
    fn default() -> Self {
        Self {
            files: 0,
            symbols: 0,
            edges: 0,
            semantic_edges: 0,
            updated_at: String::new(),
            truncated: false,
            max_files: MAX_IR_FILES,
        }
    }
}

/// Typed node payload emitted by `GraphIrStore` for both `graph_map::Index` and `graph.json`.
#[derive(Clone, Debug)]
pub(crate) struct IrNodeSpec {
    pub id: String,
    pub label: String,
    pub source_file: String,
    pub source_location: Option<String>,
    pub end_line: u32,
    pub file_type: Option<String>,
    pub node_kind: Option<String>,
    pub community: Option<u32>,
    pub community_name: Option<String>,
    pub callable_class: bool,
    pub is_abstract: bool,
    pub is_test: bool,
}

/// Typed link payload emitted by `GraphIrStore` for both `graph_map::Index` and `graph.json`.
#[derive(Clone, Debug)]
pub(crate) struct IrLinkSpec {
    pub source: String,
    pub target: String,
    pub relation: Option<String>,
    pub confidence: Option<String>,
    pub source_file: Option<String>,
    pub context: Option<String>,
    pub evidence: Option<String>,
}

pub(crate) type CallerEdgeRow = (String, Option<String>, String, Option<String>, u32);
pub(crate) type SystemSemanticEdgeRow = (String, String, String, Option<String>, u32, String);

struct LoadedGraphRows {
    files: Vec<String>,
    symbols: Vec<StoredSymbolRow>,
    edges: Vec<StoredEdgeRow>,
    sem_edges: Vec<StoredSemanticRow>,
}

/// Statistics returned by `sync_workspace` and `update_paths`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IrSyncStats {
    pub files_scanned: usize,
    pub files_parsed: usize,
    pub files_skipped: usize,
    pub files_removed: usize,
    pub summary: IrSummary,
}

/// Incremental update summary + affected file lists returned by `sync_workspace_with_delta` /
/// `update_paths_with_delta` and propagated over the Graph-IR invalidation bus.
#[derive(Clone, Debug, Default)]
pub(crate) struct IrUpdateDelta {
    pub stats: IrSyncStats,
    pub changed_paths: Vec<String>,
    pub edited_files: Vec<String>,
    pub re_resolved_files: Vec<String>,
    pub removed_files: Vec<String>,
    pub incoming_caller_files: Vec<String>,
    pub manifest_changed: bool,
    pub semantic_changed: bool,
    pub ir_revision: u64,
    pub touched_semantic_targets: Vec<String>,
    pub prev_semantic_anchor_files: Vec<String>,
    pub touched_di_files: Vec<String>,
    #[allow(dead_code)]
    pub reexport_changed: bool,
}

/// Result returned by [`GraphIrStore::verify_path_or_symbol`] for read-only verification
/// of workspace paths and declared symbols against indexed Graph IR.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IrPathSymbolVerification {
    pub path_exists: bool,
    pub symbol_exists: bool,
    pub matched_files: Vec<String>,
    pub matched_symbols: Vec<String>,
}

/// Incremental Graph IR elements exported strictly for dirty files and their incident edges.
#[derive(Clone, Debug)]
pub(crate) struct IrDeltaElements {
    pub files: Vec<String>,
    pub edited_files: BTreeSet<String>,
    pub dirty_files: BTreeSet<String>,
    pub removed_files: BTreeSet<String>,
    pub neighbor_files: Vec<String>,
    pub rs_touched: bool,
    pub node_specs: Vec<IrNodeSpec>,
    pub link_specs: Vec<IrLinkSpec>,
}

const MAX_READ_POOL_CONNS: usize = 4;

/// Persistent per-project SQLite Code Graph IR store.
pub(crate) struct GraphIrStore {
    db_path: PathBuf,
    conn: Mutex<Connection>,
    read_pool: Mutex<Vec<Connection>>,
    read_only: bool,
    write_epoch: AtomicU64,
    reexport_cache: Mutex<Option<(u64, ReexportIndex)>>,
}

struct WalkedFile {
    rel_path: String,
    size: i64,
    mtime_ns: i64,
}

struct ParsedUnit {
    rel_path: String,
    language: String,
    size: i64,
    mtime_ns: i64,
    sha256: String,
    ir: AstFileExtraction,
}

struct IrEdgeRow {
    source_file: String,
    source_symbol: Option<String>,
    target_file: String,
    target_symbol: Option<String>,
    relation: String,
    context: Option<String>,
    line: u32,
    confidence: String,
}

struct UnresolvedImportRow {
    source_file: String,
    specifier: String,
    kind: String,
    symbols_json: String,
    line: u32,
}

struct StoredSymbolRow {
    row_id: i64,
    file_path: String,
    symbol_id: String,
    kind: String,
    name: String,
    parent_name: Option<String>,
    start_line: u32,
    end_line: u32,
    is_abstract: bool,
    is_test: bool,
}

struct StoredEdgeRow {
    source_file: String,
    source_symbol: Option<String>,
    target_file: String,
    target_symbol: Option<String>,
    relation: String,
    context: Option<String>,
    line: u32,
    confidence: String,
}

struct StoredSemanticRow {
    file_path: String,
    kind: String,
    target: String,
    source_symbol: Option<String>,
    line: u32,
    evidence: String,
}

fn map_stored_symbol_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredSymbolRow> {
    let kind: String = row.get(3)?;
    let mods_json: String = row.get(9)?;
    let has_abstract = serde_json::from_str::<Vec<String>>(&mods_json)
        .unwrap_or_default()
        .iter()
        .any(|m| m.eq_ignore_ascii_case("abstract"));
    let is_abstract = matches!(kind.as_str(), "interface" | "trait" | "protocol") || has_abstract;
    Ok(StoredSymbolRow {
        row_id: row.get(0)?,
        file_path: row.get(1)?,
        symbol_id: row.get(2)?,
        kind,
        name: row.get(4)?,
        parent_name: row.get(5)?,
        start_line: row.get::<_, i64>(6)?.max(0) as u32,
        end_line: row.get::<_, i64>(7)?.max(0) as u32,
        is_abstract,
        is_test: row.get::<_, i64>(8)? != 0,
    })
}

const MAX_CACHED_RO_STORES: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
struct CachedRoIdentity {
    len: u64,
    mtime_ns: u64,
    ino: u64,
}

type CachedRoEntry = (PathBuf, CachedRoIdentity, Arc<GraphIrStore>);

fn ro_store_cache() -> &'static Mutex<Vec<CachedRoEntry>> {
    static RO_CACHE: OnceLock<Mutex<Vec<CachedRoEntry>>> = OnceLock::new();
    RO_CACHE.get_or_init(|| Mutex::new(Vec::new()))
}

impl GraphIrStore {
    /// Opens (or initializes) `<state>/graphs/<project>/graph_ir.sqlite3` with custody checks and WAL mode.
    pub(crate) fn open(state: &Path, project: &str) -> Result<Self> {
        validate_project_id(project)?;
        let home = state.join("graphs").join(project);
        let db_path = ensure_private_sqlite_path(&home, DB_FILENAME)?;
        Self::open_verified(&db_path)
    }

    /// Opens (or initializes) a SQLite IR store at an explicit file path with parent custody checks.
    pub(crate) fn open_at(db_path: &Path) -> Result<Self> {
        let parent = db_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| HarnessError::invalid("Graph IR database path has no parent."))?;
        let name = db_path
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty() && !s.contains(['/', '\\', '\0']))
            .ok_or_else(|| HarnessError::invalid("Invalid Graph IR database filename."))?;
        let verified = ensure_private_sqlite_path(parent, name)?;
        Self::open_verified(&verified)
    }

    fn open_verified(db_path: &Path) -> Result<Self> {
        let mut conn = Connection::open(db_path)?;
        conn.busy_timeout(SQLITE_BUSY_TIMEOUT)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS ir_meta(
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS ir_files(
                 path TEXT PRIMARY KEY,
                 language TEXT NOT NULL,
                 size INTEGER NOT NULL,
                 mtime_ns INTEGER NOT NULL,
                 content_sha256 TEXT NOT NULL,
                 namespace TEXT,
                 indexed_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS ir_symbols(
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 file_path TEXT NOT NULL,
                 symbol_id TEXT NOT NULL,
                 kind TEXT NOT NULL,
                 name TEXT NOT NULL,
                 parent_name TEXT,
                 signature TEXT,
                 visibility TEXT,
                 start_line INTEGER NOT NULL,
                 end_line INTEGER NOT NULL,
                 modifiers_json TEXT NOT NULL,
                 body_fingerprint TEXT,
                 is_test INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX IF NOT EXISTS idx_ir_symbols_file_path ON ir_symbols(file_path);
             CREATE TABLE IF NOT EXISTS ir_edges(
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 source_file TEXT NOT NULL,
                 source_symbol TEXT,
                 target_file TEXT NOT NULL,
                 target_symbol TEXT,
                 relation TEXT NOT NULL,
                 context TEXT,
                 line INTEGER NOT NULL,
                 confidence TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_ir_edges_source_file ON ir_edges(source_file);
             CREATE INDEX IF NOT EXISTS idx_ir_edges_target_file ON ir_edges(target_file);
             CREATE TABLE IF NOT EXISTS ir_semantic_edges(
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 file_path TEXT NOT NULL,
                 kind TEXT NOT NULL,
                 target TEXT NOT NULL,
                 source_symbol TEXT,
                 line INTEGER NOT NULL,
                 evidence TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_ir_semantic_edges_file_path ON ir_semantic_edges(file_path);
             CREATE INDEX IF NOT EXISTS idx_ir_semantic_edges_kind ON ir_semantic_edges(kind);
             CREATE TABLE IF NOT EXISTS ir_reexports(
                 file_path TEXT NOT NULL,
                 alias TEXT NOT NULL,
                 orig_path TEXT NOT NULL,
                 orig_symbol TEXT NOT NULL,
                 is_wildcard INTEGER NOT NULL,
                 line INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_ir_reexports_file_path ON ir_reexports(file_path);
             CREATE TABLE IF NOT EXISTS ir_file_extractions(
                 file_path TEXT PRIMARY KEY,
                 summary_json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS ir_symbol_refs(
                 token_lower TEXT NOT NULL,
                 file_path TEXT NOT NULL,
                 PRIMARY KEY (token_lower, file_path)
             );
             CREATE INDEX IF NOT EXISTS idx_ir_symbol_refs_file_path ON ir_symbol_refs(file_path);
             CREATE TABLE IF NOT EXISTS ir_unresolved_imports(
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 source_file TEXT NOT NULL,
                 specifier TEXT NOT NULL,
                 kind TEXT NOT NULL,
                 symbols_json TEXT NOT NULL,
                 line INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_ir_unresolved_source ON ir_unresolved_imports(source_file);
             CREATE INDEX IF NOT EXISTS idx_ir_unresolved_specifier ON ir_unresolved_imports(specifier);
             CREATE TABLE IF NOT EXISTS ir_summary(
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 files INTEGER NOT NULL,
                 symbols INTEGER NOT NULL,
                 edges INTEGER NOT NULL,
                 semantic_edges INTEGER NOT NULL,
                 updated_at TEXT NOT NULL,
                 truncated INTEGER NOT NULL DEFAULT 0,
                 max_files INTEGER NOT NULL DEFAULT 30000
             );",
        )?;
        ensure_column(&conn, "ir_symbols", "is_test", "INTEGER NOT NULL DEFAULT 0")?;
        ensure_column(
            &conn,
            "ir_summary",
            "truncated",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        ensure_column(
            &conn,
            "ir_summary",
            "max_files",
            "INTEGER NOT NULL DEFAULT 30000",
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO ir_meta(key, value) VALUES ('write_epoch', '0')",
            [],
        )?;
        {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let prev_schema_num: u32 = tx
                .query_row(
                    "SELECT value FROM ir_meta WHERE key = 'schema_version'",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            if prev_schema_num < 3 {
                // Force full backfill when upgrading from pre-v3 schema so `ir_file_extractions`
                // and `ir_symbol_refs` are never trusted in a partially indexed state.
                let has_legacy_files: bool = tx
                    .query_row("SELECT EXISTS(SELECT 1 FROM ir_files LIMIT 1)", [], |r| {
                        r.get(0)
                    })
                    .optional()?
                    .unwrap_or(false);
                if has_legacy_files {
                    tx.execute_batch(
                        "DELETE FROM ir_files;
                         DELETE FROM ir_symbols;
                         DELETE FROM ir_edges;
                         DELETE FROM ir_semantic_edges;
                         DELETE FROM ir_reexports;
                         DELETE FROM ir_unresolved_imports;
                         DELETE FROM ir_file_extractions;
                         DELETE FROM ir_symbol_refs;
                         DELETE FROM ir_summary;",
                    )?;
                    let _ = bump_meta_write_epoch(&tx)?;
                }
            }
            upsert_meta(&tx, "schema_version", SCHEMA_VERSION)?;
            upsert_meta(&tx, "backend", NATIVE_IR_TOOL)?;
            tx.commit()?;
        }
        let epoch = read_meta_write_epoch(&conn);
        Ok(Self {
            db_path: db_path.to_path_buf(),
            conn: Mutex::new(conn),
            read_pool: Mutex::new(Vec::new()),
            read_only: false,
            write_epoch: AtomicU64::new(epoch),
            reexport_cache: Mutex::new(None),
        })
    }

    /// Opens an existing `<state>/graphs/<project>/graph_ir.sqlite3` strictly read-only without
    /// creating directories/files or executing any DDL/`ir_meta` writes. Returns `Ok(None)` if missing
    /// or uninitialized.
    pub(crate) fn open_readonly(state: &Path, project: &str) -> Result<Option<Self>> {
        validate_project_id(project)?;
        if !Self::exists(state, project) {
            return Ok(None);
        }
        let db_path = state.join("graphs").join(project).join(DB_FILENAME);
        Self::open_at_readonly(&db_path)
    }

    /// Opens an existing `<state>/graphs/<project>/graph_ir.sqlite3` strictly read-only without
    /// creating directories/files or executing any DDL/`ir_meta` writes, returning a shared cached
    /// handle when the verified SQLite file identity is unchanged.
    pub(crate) fn open_readonly_cached(state: &Path, project: &str) -> Result<Option<Arc<Self>>> {
        validate_project_id(project)?;
        if !Self::exists(state, project) {
            return Ok(None);
        }
        let db_path = state.join("graphs").join(project).join(DB_FILENAME);
        Self::open_at_readonly_cached(&db_path)
    }

    /// Opens an existing SQLite IR store at `db_path` strictly read-only with bounded process-wide
    /// handle caching so concurrent readers reuse WAL read pools across calls.
    pub(crate) fn open_at_readonly_cached(db_path: &Path) -> Result<Option<Arc<Self>>> {
        let Some(verified) = verify_existing_sqlite_path(db_path)? else {
            if let Ok(mut guard) = ro_store_cache().lock() {
                guard.retain(|(p, _, _)| p != db_path);
            }
            return Ok(None);
        };
        let meta = fs::symlink_metadata(&verified)?;
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
        #[cfg(unix)]
        let ino = {
            use std::os::unix::fs::MetadataExt;
            meta.ino()
        };
        #[cfg(not(unix))]
        let ino = 0;
        let ident = CachedRoIdentity {
            len: meta.len(),
            mtime_ns,
            ino,
        };

        let cache = ro_store_cache();
        if let Ok(mut guard) = cache.lock()
            && let Some(pos) = guard
                .iter()
                .position(|(p, id, _)| *p == verified && *id == ident)
        {
            let hit = guard.remove(pos);
            let cloned = Arc::clone(&hit.2);
            guard.push(hit);
            return Ok(Some(cloned));
        }

        let Some(opened) = Self::open_at_readonly(&verified)? else {
            if let Ok(mut guard) = cache.lock() {
                guard.retain(|(p, _, _)| *p != verified);
            }
            return Ok(None);
        };
        let shared = Arc::new(opened);
        if let Ok(mut guard) = cache.lock() {
            guard.retain(|(p, _, _)| *p != verified);
            if guard.len() >= MAX_CACHED_RO_STORES {
                guard.remove(0);
            }
            guard.push((verified, ident, Arc::clone(&shared)));
        }
        Ok(Some(shared))
    }

    /// Evicts any cached read-only SQLite handle rooted under `dir` so deletes succeed
    /// immediately on Windows without sharing violations on `graph_ir.sqlite3`.
    pub(crate) fn evict_readonly_cache_for_dir(dir: &Path) {
        let canonical = infra_core::fs::python_realpath(dir).ok();
        let matches_dir = |candidate: &Path| -> bool {
            let check = |base: &Path| -> bool {
                if candidate.starts_with(base) {
                    return true;
                }
                if cfg!(windows) {
                    let cand_s = candidate
                        .to_string_lossy()
                        .replace('\\', "/")
                        .to_lowercase();
                    let base_s = base
                        .to_string_lossy()
                        .replace('\\', "/")
                        .trim_end_matches('/')
                        .to_lowercase();
                    !base_s.is_empty()
                        && (cand_s == base_s
                            || cand_s
                                .strip_prefix(&base_s)
                                .is_some_and(|rest| rest.starts_with('/')))
                } else {
                    false
                }
            };
            check(dir) || canonical.as_ref().is_some_and(|c| check(c))
        };
        if let Ok(mut guard) = ro_store_cache().lock() {
            guard.retain(|(p, _, _)| !matches_dir(p));
        }
    }

    /// Opens an existing SQLite IR store at `db_path` strictly read-only without creating files
    /// or executing any DDL/`ir_meta` writes. Returns `Ok(None)` if missing or uninitialized.
    pub(crate) fn open_at_readonly(db_path: &Path) -> Result<Option<Self>> {
        let Some(verified) = verify_existing_sqlite_path(db_path)? else {
            return Ok(None);
        };
        let conn = Connection::open_with_flags(
            &verified,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(SQLITE_BUSY_TIMEOUT)?;
        let initialized: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='ir_files')",
            [],
            |r| r.get(0),
        )?;
        if !initialized {
            return Ok(None);
        }
        let epoch = read_meta_write_epoch(&conn);
        Ok(Some(Self {
            db_path: verified,
            conn: Mutex::new(conn),
            read_pool: Mutex::new(Vec::new()),
            read_only: true,
            write_epoch: AtomicU64::new(epoch),
            reexport_cache: Mutex::new(None),
        }))
    }

    /// Returns a pooled read-only connection on WAL mode (falling back to `self.conn`).
    fn with_read_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let pooled = if let Ok(mut pool) = self.read_pool.lock() {
            pool.pop()
        } else {
            None
        };
        let conn = match pooled {
            Some(c) => c,
            None => match open_readonly_pooled_conn(&self.db_path) {
                Ok(c) => c,
                Err(_) => {
                    let guard = lock_conn(&self.conn)?;
                    return f(&guard);
                }
            },
        };
        let res = f(&conn);
        if let Ok(mut pool) = self.read_pool.lock()
            && pool.len() < MAX_READ_POOL_CONNS
        {
            pool.push(conn);
        }
        res
    }

    /// Returns the monotonic persisted SQLite write epoch (queried live against `ir_meta`).
    pub(crate) fn current_write_epoch(&self) -> u64 {
        self.with_read_conn(|conn| Ok(read_meta_write_epoch(conn)))
            .unwrap_or_else(|_| self.write_epoch.load(Ordering::Acquire))
    }

    /// Returns whether `<state>/graphs/<project>/graph_ir.sqlite3` already exists on disk.
    pub(crate) fn exists(state: &Path, project: &str) -> bool {
        if validate_project_id(project).is_err() {
            return false;
        }
        let path = state.join("graphs").join(project).join(DB_FILENAME);
        fs::symlink_metadata(&path)
            .is_ok_and(|meta| meta.is_file() && !meta.file_type().is_symlink())
    }

    #[allow(dead_code)]
    pub(crate) fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Synchronizes all bounded workspace code files under `root` into SQLite incrementally.
    ///
    /// Files matching `(size, mtime_ns)` in `ir_files` are skipped; changed/new files are parsed
    /// via `graph_ast::parse_file_ir`, deleted files are pruned, and cross-file dependencies +
    /// `ir_summary` are updated atomically in one SQLite transaction. All CPU/disk AST parsing
    /// completes before acquiring the `Immediate` SQLite write transaction.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn sync_workspace(&self, root: &Path) -> Result<IrSyncStats> {
        self.sync_workspace_with_delta(root)
            .map(|delta| delta.stats)
    }

    /// Synchronizes all bounded workspace code files under `root` into SQLite incrementally and
    /// returns exact `edited_files`, `removed_files`, `re_resolved_files`, `incoming_caller_files`,
    /// `changed_paths` (including changed manifests), and `IrSyncStats`.
    pub(crate) fn sync_workspace_with_delta(&self, root: &Path) -> Result<IrUpdateDelta> {
        if self.read_only {
            return Err(HarnessError::invalid("Graph IR store is read-only."));
        }
        if !root.is_dir() {
            return Err(HarnessError::invalid(
                "Workspace directory does not exist for Graph IR sync.",
            ));
        }
        let real_root = fs::canonicalize(root)?;
        let (walked, truncated) = walk_workspace_code_files(&real_root)?;
        let files_scanned = walked.len();
        let known_files: BTreeSet<String> = walked.iter().map(|w| w.rel_path.clone()).collect();
        let manifest_stamp = compute_root_manifest_stamp(&real_root);
        let git_head = current_git_head(&real_root);

        let (existing, prev_manifest_stamp, prev_git_head, mut epoch_before) = {
            let conn = lock_conn(&self.conn)?;
            let mut stmt = conn.prepare("SELECT path, size, mtime_ns FROM ir_files")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    (row.get::<_, i64>(1)?, row.get::<_, i64>(2)?),
                ))
            })?;
            let mut map = BTreeMap::new();
            for row in rows {
                let (path, stamp) = row?;
                map.insert(path, stamp);
            }
            let prev_stamp: Option<String> = conn
                .query_row(
                    "SELECT value FROM ir_meta WHERE key = 'manifest_stamp'",
                    [],
                    |r| r.get(0),
                )
                .optional()?;
            let prev_head: Option<String> = conn
                .query_row(
                    "SELECT value FROM ir_meta WHERE key = 'git_head'",
                    [],
                    |r| r.get(0),
                )
                .optional()?;
            let epoch = read_meta_write_epoch(&conn);
            self.write_epoch.store(epoch, Ordering::Release);
            (map, prev_stamp, prev_head, epoch)
        };

        let removed: Vec<String> = existing
            .keys()
            .filter(|path| !known_files.contains(*path))
            .cloned()
            .collect();
        let stale_paths: Vec<String> = walked
            .iter()
            .filter(|w| existing.get(&w.rel_path) != Some(&(w.size, w.mtime_ns)))
            .map(|w| w.rel_path.clone())
            .collect();
        let manifest_changed = prev_manifest_stamp.as_deref() != Some(manifest_stamp.as_str());

        if removed.is_empty()
            && stale_paths.is_empty()
            && !manifest_changed
            && let Some(mut summary) = self.summary_counts()?
        {
            if summary.truncated != truncated || prev_git_head != git_head {
                let mut conn = lock_conn(&self.conn)?;
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                if summary.truncated != truncated {
                    tx.execute(
                        "UPDATE ir_summary SET truncated = ?1, max_files = ?2 WHERE id = 1",
                        params![i64::from(truncated), MAX_IR_FILES as i64],
                    )?;
                    summary.truncated = truncated;
                }
                if let Some(ref head) = git_head {
                    upsert_meta(&tx, "git_head", head)?;
                } else if prev_git_head.is_some() {
                    tx.execute("DELETE FROM ir_meta WHERE key = 'git_head'", [])?;
                }
                let next_epoch = bump_meta_write_epoch(&tx)?;
                tx.commit()?;
                self.write_epoch.store(next_epoch, Ordering::Release);
            }
            return Ok(IrUpdateDelta {
                stats: IrSyncStats {
                    files_scanned,
                    files_parsed: 0,
                    files_skipped: files_scanned,
                    files_removed: 0,
                    summary,
                },
                ..IrUpdateDelta::default()
            });
        }

        let (parsed_units, _) = parse_paths_parallel(&real_root, &stale_paths);
        let files_parsed = parsed_units.len();
        let files_skipped = files_scanned.saturating_sub(stale_paths.len());
        let files_removed = removed.len();

        let (manifests, crate_map) =
            graph_deps::cached_manifests_for_root(&real_root, &known_files);

        let mut affected_targets = BTreeSet::new();
        for path in &removed {
            affected_targets.insert(path.clone());
        }
        let mut skip_re_resolve = BTreeSet::new();
        for unit in &parsed_units {
            affected_targets.insert(unit.rel_path.clone());
            skip_re_resolve.insert(unit.rel_path.clone());
        }

        let is_incremental_update = !existing.is_empty();
        let mut parsed_re_resolve: BTreeMap<String, ParsedUnit> = BTreeMap::new();
        let (mut pending_re_resolve, mut pre_tx_incoming) =
            if is_incremental_update && (!affected_targets.is_empty() || manifest_changed) {
                let conn = lock_conn(&self.conn)?;
                epoch_before = read_meta_write_epoch(&conn);
                self.write_epoch.store(epoch_before, Ordering::Release);
                let callers = query_incoming_callers_from_conn(&conn, &affected_targets)?;
                let re_resolve = collect_affected_dependents(
                    &conn,
                    &affected_targets,
                    &skip_re_resolve,
                    manifest_changed,
                    &[],
                )?;
                (re_resolve, callers)
            } else {
                (Vec::new(), BTreeSet::new())
            };

        let mut cached_re_resolve =
            self.with_read_conn(|conn| load_persisted_ir_extractions(conn, &pending_re_resolve))?;
        let (
            summary,
            re_resolved_out,
            semantic_changed,
            pre_tx_incoming,
            touched_semantic_targets,
            prev_semantic_anchor_files,
            touched_di_files,
            reexport_changed,
        ) = loop {
            let missing_re_resolve: Vec<String> = pending_re_resolve
                .iter()
                .filter(|p| {
                    !cached_re_resolve.contains_key(*p) && !parsed_re_resolve.contains_key(*p)
                })
                .cloned()
                .collect();
            if !missing_re_resolve.is_empty() {
                let (units, _) = parse_paths_parallel(&real_root, &missing_re_resolve);
                for unit in units {
                    parsed_re_resolve.insert(unit.rel_path.clone(), unit);
                }
            }

            let mut conn = lock_conn(&self.conn)?;
            let db_epoch = read_meta_write_epoch(&conn);
            if db_epoch != epoch_before || self.write_epoch.load(Ordering::Acquire) != epoch_before
            {
                epoch_before = db_epoch;
                self.write_epoch.store(db_epoch, Ordering::Release);
                let has_rows: bool =
                    conn.query_row("SELECT EXISTS(SELECT 1 FROM ir_files)", [], |r| r.get(0))?;
                if has_rows && (!affected_targets.is_empty() || manifest_changed) {
                    pre_tx_incoming = query_incoming_callers_from_conn(&conn, &affected_targets)?;
                    pending_re_resolve = collect_affected_dependents(
                        &conn,
                        &affected_targets,
                        &skip_re_resolve,
                        manifest_changed,
                        &[],
                    )?;
                } else {
                    pre_tx_incoming.clear();
                    pending_re_resolve.clear();
                }
                let missing_cached: Vec<String> = pending_re_resolve
                    .iter()
                    .filter(|p| {
                        !cached_re_resolve.contains_key(*p) && !parsed_re_resolve.contains_key(*p)
                    })
                    .cloned()
                    .collect();
                if !missing_cached.is_empty() {
                    let extra_cached = load_persisted_ir_extractions(&conn, &missing_cached)?;
                    cached_re_resolve.extend(extra_cached);
                }
                if pending_re_resolve.iter().any(|p| {
                    !cached_re_resolve.contains_key(p) && !parsed_re_resolve.contains_key(p)
                }) {
                    continue;
                }
            }

            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let now = store::now();
            let touched_before: Vec<String> = removed
                .iter()
                .chain(parsed_units.iter().map(|u| &u.rel_path))
                .cloned()
                .collect();
            let PrevReexportSemanticState {
                prev_reexports,
                prev_sem_targets,
                prev_di_targets,
                prev_sem_anchors,
            } = collect_prev_reexport_and_semantic_state(&tx, &touched_before)?;
            let semantic_changed = detect_semantic_changed(&tx, removed.iter(), &parsed_units)?;

            for path in &removed {
                delete_file_records(&tx, path)?;
            }
            for unit in &parsed_units {
                upsert_parsed_file(&tx, unit, &now)?;
            }

            let mut reexports = obtain_tx_reexport_index(&self.reexport_cache, &tx)?;
            update_reexport_index_for_units(&mut reexports, &removed, &parsed_units);

            for unit in &parsed_units {
                replace_file_edges(&tx, unit, &known_files, &manifests, &reexports, &crate_map)?;
            }
            for path in &pending_re_resolve {
                if let Some(ir) = cached_re_resolve.get(path) {
                    replace_file_edges_for_ir(
                        &tx,
                        path,
                        ir,
                        &known_files,
                        &manifests,
                        &reexports,
                        &crate_map,
                    )?;
                } else if let Some(unit) = parsed_re_resolve.get(path) {
                    upsert_parsed_file(&tx, unit, &now)?;
                    update_reexport_index_for_units(
                        &mut reexports,
                        &[],
                        std::slice::from_ref(unit),
                    );
                    replace_file_edges(
                        &tx,
                        unit,
                        &known_files,
                        &manifests,
                        &reexports,
                        &crate_map,
                    )?;
                }
            }

            let reexp_changed =
                !compute_reexport_changed_files(&prev_reexports, &removed, &parsed_units)
                    .is_empty();
            let (post_sem_targets, post_di_files) = collect_post_semantic_delta(
                &tx,
                &prev_sem_targets,
                &prev_di_targets,
                &parsed_units,
            )?;

            let summary = recompute_summary(&tx, &now, truncated)?;
            upsert_meta(&tx, "updated_at", &now)?;
            upsert_meta(&tx, "manifest_stamp", &manifest_stamp)?;
            if let Some(ref head) = git_head {
                upsert_meta(&tx, "git_head", head)?;
            } else {
                tx.execute("DELETE FROM ir_meta WHERE key = 'git_head'", [])?;
            }
            let next_epoch = bump_meta_write_epoch(&tx)?;
            tx.commit()?;
            self.write_epoch.store(next_epoch, Ordering::Release);
            store_reexport_cache(&self.reexport_cache, next_epoch, reexports);
            break (
                summary,
                pending_re_resolve,
                semantic_changed,
                pre_tx_incoming,
                post_sem_targets,
                prev_sem_anchors,
                post_di_files,
                reexp_changed,
            );
        };

        let edited_files: Vec<String> = parsed_units.into_iter().map(|u| u.rel_path).collect();
        let mut changed_set: BTreeSet<String> = edited_files.iter().cloned().collect();
        changed_set.extend(removed.iter().cloned());
        if manifest_changed {
            changed_set.extend(changed_manifest_paths_from_stamps(
                prev_manifest_stamp.as_deref(),
                &manifest_stamp,
            ));
        }
        let incoming_caller_files =
            self.collect_post_tx_incoming(pre_tx_incoming, &edited_files, &removed);

        Ok(IrUpdateDelta {
            stats: IrSyncStats {
                files_scanned,
                files_parsed,
                files_skipped,
                files_removed,
                summary,
            },
            changed_paths: changed_set.into_iter().collect(),
            edited_files,
            re_resolved_files: re_resolved_out,
            removed_files: removed,
            incoming_caller_files,
            manifest_changed,
            semantic_changed,
            ir_revision: 0,
            touched_semantic_targets,
            prev_semantic_anchor_files,
            touched_di_files,
            reexport_changed,
        })
    }

    /// Incrementally re-indexes ONLY the specified changed or deleted relative paths in one transaction,
    /// and re-resolves reverse dependents, re-export consumers, and matching unresolved imports.
    /// All CPU/disk parsing and manifest reads run before opening the `Immediate` write transaction.
    #[cfg(test)]
    pub(crate) fn update_paths(&self, root: &Path, rel_paths: &[String]) -> Result<IrSyncStats> {
        self.update_paths_with_delta(root, rel_paths)
            .map(|delta| delta.stats)
    }

    /// Incrementally re-indexes changed/deleted relative paths and reports exact edited,
    /// re-resolved, and removed file lists for downstream incremental `Index` updates.
    pub(crate) fn update_paths_with_delta(
        &self,
        root: &Path,
        rel_paths: &[String],
    ) -> Result<IrUpdateDelta> {
        if self.read_only {
            return Err(HarnessError::invalid("Graph IR store is read-only."));
        }
        if !root.is_dir() {
            return Err(HarnessError::invalid(
                "Workspace directory does not exist for Graph IR update.",
            ));
        }
        let real_root = fs::canonicalize(root)?;
        let mut normalized = BTreeSet::new();
        for raw in rel_paths {
            if let Some(rel) = normalize_rel_path(&real_root, raw) {
                normalized.insert(rel);
            }
        }
        if normalized.is_empty() {
            let summary = self.summary_counts()?.unwrap_or_default();
            return Ok(IrUpdateDelta {
                stats: IrSyncStats {
                    summary,
                    ..IrSyncStats::default()
                },
                ..IrUpdateDelta::default()
            });
        }

        let mut manifest_changed = false;
        let mut manifest_langs: BTreeSet<&'static str> = BTreeSet::new();
        let mut parse_candidates = BTreeSet::new();
        let mut removed = Vec::new();
        let mut removed_prefixes = Vec::new();
        let mut skipped = 0usize;

        for rel in &normalized {
            let p = Path::new(rel);
            if is_graph_architecture_path(p) {
                continue;
            }
            if is_graph_manifest_path(p) {
                manifest_changed = true;
                collect_manifest_languages(p, &mut manifest_langs);
                continue;
            }
            let full = real_root.join(rel);
            match fs::symlink_metadata(&full) {
                Ok(meta) if !meta.file_type().is_symlink() && meta.is_file() => {
                    if is_supported_code_rel(rel) && meta.len() <= MAX_FILE_BYTES {
                        parse_candidates.insert(rel.clone());
                    } else {
                        removed.push(rel.clone());
                    }
                }
                Ok(meta) if !meta.file_type().is_symlink() && meta.is_dir() => {
                    collect_dir_update_candidates(
                        &full,
                        rel,
                        &mut parse_candidates,
                        &mut manifest_changed,
                        &mut manifest_langs,
                    );
                }
                Ok(_) | Err(_) => {
                    let full_err = fs::symlink_metadata(&full).err();
                    if full_err
                        .as_ref()
                        .is_some_and(|e| e.kind() != std::io::ErrorKind::NotFound)
                    {
                        skipped += 1;
                    } else {
                        removed.push(rel.clone());
                        let trimmed = rel.trim_end_matches('/');
                        if !trimmed.is_empty() {
                            removed_prefixes.push(format!("{trimmed}/"));
                        }
                    }
                }
            }
        }

        let parse_candidates_vec: Vec<String> = parse_candidates.into_iter().collect();
        let (parsed_units, parse_skipped) = parse_paths_parallel(&real_root, &parse_candidates_vec);
        skipped += parse_skipped;

        let git_head = current_git_head(&real_root);
        let manifest_lang_vec: Vec<&str> = manifest_langs.into_iter().collect();

        let files_scanned = normalized.len();
        let files_parsed = parsed_units.len();

        let mut parsed_re_resolve: BTreeMap<String, ParsedUnit> = BTreeMap::new();
        let mut manifests = Arc::new(ManifestIndex::default());
        let mut crate_map = Arc::new(CratePackageMap::default());
        let mut cached_bundle_key_len: Option<usize> = None;
        let mut cached_re_resolve: BTreeMap<String, AstFileExtraction> = BTreeMap::new();
        let (
            files_removed,
            summary,
            removed_out,
            re_resolved_out,
            semantic_changed,
            pre_tx_incoming,
            touched_semantic_targets,
            prev_semantic_anchor_files,
            touched_di_files,
            reexport_changed,
        ) = loop {
            let (
                existing_files,
                removed_files,
                actually_removed,
                affected_targets,
                skip_re_resolve,
                re_resolve_paths,
                pre_tx_incoming,
                epoch_before,
            ) = {
                let conn = lock_conn(&self.conn)?;
                let mut existing_files = BTreeSet::new();
                {
                    let mut stmt = conn.prepare("SELECT path FROM ir_files")?;
                    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                    for row in rows {
                        existing_files.insert(row?);
                    }
                }
                let mut affected_targets = BTreeSet::new();
                let mut skip_re_resolve = BTreeSet::new();
                let mut removed_files = BTreeSet::new();
                for path in &removed {
                    if existing_files.contains(path) {
                        removed_files.insert(path.clone());
                    }
                }
                if !removed_prefixes.is_empty() {
                    for existing in &existing_files {
                        if removed_prefixes
                            .iter()
                            .any(|pref| existing.starts_with(pref))
                        {
                            removed_files.insert(existing.clone());
                        }
                    }
                }
                let mut actually_removed = 0usize;
                for path in &removed_files {
                    if existing_files.remove(path) {
                        actually_removed += 1;
                        affected_targets.insert(path.clone());
                    }
                }
                for unit in &parsed_units {
                    existing_files.insert(unit.rel_path.clone());
                    affected_targets.insert(unit.rel_path.clone());
                    skip_re_resolve.insert(unit.rel_path.clone());
                }
                let (re_resolve_paths, pre_tx_incoming) =
                    if !affected_targets.is_empty() || manifest_changed {
                        let callers = query_incoming_callers_from_conn(&conn, &affected_targets)?;
                        let re_resolve = collect_affected_dependents(
                            &conn,
                            &affected_targets,
                            &skip_re_resolve,
                            manifest_changed,
                            &manifest_lang_vec,
                        )?;
                        (re_resolve, callers)
                    } else {
                        (Vec::new(), BTreeSet::new())
                    };
                let epoch = read_meta_write_epoch(&conn);
                self.write_epoch.store(epoch, Ordering::Release);
                (
                    existing_files,
                    removed_files,
                    actually_removed,
                    affected_targets,
                    skip_re_resolve,
                    re_resolve_paths,
                    pre_tx_incoming,
                    epoch,
                )
            };

            if parsed_units.is_empty()
                && removed_files.is_empty()
                && re_resolve_paths.is_empty()
                && !manifest_changed
            {
                let summary = self.summary_counts()?.unwrap_or_default();
                break (
                    0,
                    summary,
                    Vec::new(),
                    Vec::new(),
                    false,
                    BTreeSet::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    false,
                );
            }

            let need_cached: Vec<String> = re_resolve_paths
                .iter()
                .filter(|p| {
                    !cached_re_resolve.contains_key(*p) && !parsed_re_resolve.contains_key(*p)
                })
                .cloned()
                .collect();
            if !need_cached.is_empty() {
                let loaded =
                    self.with_read_conn(|conn| load_persisted_ir_extractions(conn, &need_cached))?;
                cached_re_resolve.extend(loaded);
            }
            let missing_re_resolve: Vec<String> = re_resolve_paths
                .iter()
                .filter(|p| {
                    !cached_re_resolve.contains_key(*p) && !parsed_re_resolve.contains_key(*p)
                })
                .cloned()
                .collect();
            if !missing_re_resolve.is_empty() {
                let (units, _) = parse_paths_parallel(&real_root, &missing_re_resolve);
                for unit in units {
                    parsed_re_resolve.insert(unit.rel_path.clone(), unit);
                }
            }

            let manifest_stamp = if manifest_changed {
                Some(compute_root_manifest_stamp(&real_root))
            } else {
                None
            };
            if (!parsed_units.is_empty() || !re_resolve_paths.is_empty())
                && cached_bundle_key_len != Some(existing_files.len())
            {
                let (m, cm) = graph_deps::cached_manifests_for_root(&real_root, &existing_files);
                manifests = m;
                crate_map = cm;
                cached_bundle_key_len = Some(existing_files.len());
            }

            let mut conn = lock_conn(&self.conn)?;
            let db_epoch = read_meta_write_epoch(&conn);
            if db_epoch != epoch_before || self.write_epoch.load(Ordering::Acquire) != epoch_before
            {
                self.write_epoch.store(db_epoch, Ordering::Release);
                continue;
            }

            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let now = store::now();
            let removed_vec_pre: Vec<String> = removed_files.iter().cloned().collect();
            let touched_before: Vec<String> = removed_vec_pre
                .iter()
                .chain(parsed_units.iter().map(|u| &u.rel_path))
                .cloned()
                .collect();
            let PrevReexportSemanticState {
                prev_reexports,
                prev_sem_targets,
                prev_di_targets,
                prev_sem_anchors,
            } = collect_prev_reexport_and_semantic_state(&tx, &touched_before)?;
            let semantic_changed =
                detect_semantic_changed(&tx, removed_files.iter(), &parsed_units)?;
            for path in &removed_files {
                delete_file_records(&tx, path)?;
            }
            for unit in &parsed_units {
                upsert_parsed_file(&tx, unit, &now)?;
            }

            let _ = (&affected_targets, &skip_re_resolve);
            let mut reexports = obtain_tx_reexport_index(&self.reexport_cache, &tx)?;
            if !parsed_units.is_empty() || !removed_files.is_empty() || !re_resolve_paths.is_empty()
            {
                update_reexport_index_for_units(&mut reexports, &removed_vec_pre, &parsed_units);
                for unit in &parsed_units {
                    replace_file_edges(
                        &tx,
                        unit,
                        &existing_files,
                        &manifests,
                        &reexports,
                        &crate_map,
                    )?;
                }
                for path in &re_resolve_paths {
                    if let Some(ir) = cached_re_resolve.get(path) {
                        replace_file_edges_for_ir(
                            &tx,
                            path,
                            ir,
                            &existing_files,
                            &manifests,
                            &reexports,
                            &crate_map,
                        )?;
                    } else if let Some(unit) = parsed_re_resolve.get(path) {
                        upsert_parsed_file(&tx, unit, &now)?;
                        update_reexport_index_for_units(
                            &mut reexports,
                            &[],
                            std::slice::from_ref(unit),
                        );
                        replace_file_edges(
                            &tx,
                            unit,
                            &existing_files,
                            &manifests,
                            &reexports,
                            &crate_map,
                        )?;
                    }
                }
            }

            let reexp_changed =
                !compute_reexport_changed_files(&prev_reexports, &removed_vec_pre, &parsed_units)
                    .is_empty();
            let (post_sem_targets, post_di_files) = collect_post_semantic_delta(
                &tx,
                &prev_sem_targets,
                &prev_di_targets,
                &parsed_units,
            )?;

            let prev_truncated: bool = tx
                .query_row("SELECT truncated FROM ir_summary WHERE id = 1", [], |r| {
                    Ok(r.get::<_, i64>(0)? != 0)
                })
                .optional()?
                .unwrap_or(false)
                && existing_files.len() >= MAX_IR_FILES;
            let summary = recompute_summary(&tx, &now, prev_truncated)?;
            upsert_meta(&tx, "updated_at", &now)?;
            if let Some(stamp) = &manifest_stamp {
                upsert_meta(&tx, "manifest_stamp", stamp)?;
            }
            if let Some(ref head) = git_head {
                upsert_meta(&tx, "git_head", head)?;
            }
            let next_epoch = bump_meta_write_epoch(&tx)?;
            tx.commit()?;
            self.write_epoch.store(next_epoch, Ordering::Release);
            store_reexport_cache(&self.reexport_cache, next_epoch, reexports);
            break (
                actually_removed,
                summary,
                removed_vec_pre,
                re_resolve_paths,
                semantic_changed,
                pre_tx_incoming,
                post_sem_targets,
                prev_sem_anchors,
                post_di_files,
                reexp_changed,
            );
        };

        let changed_paths: Vec<String> = normalized.into_iter().collect();
        let edited_files: Vec<String> = parsed_units.into_iter().map(|u| u.rel_path).collect();
        let incoming_caller_files =
            self.collect_post_tx_incoming(pre_tx_incoming, &edited_files, &removed_out);
        Ok(IrUpdateDelta {
            stats: IrSyncStats {
                files_scanned,
                files_parsed,
                files_skipped: skipped,
                files_removed,
                summary,
            },
            changed_paths,
            edited_files,
            re_resolved_files: re_resolved_out,
            removed_files: removed_out,
            incoming_caller_files,
            manifest_changed,
            semantic_changed,
            ir_revision: 0,
            touched_semantic_targets,
            prev_semantic_anchor_files,
            touched_di_files,
            reexport_changed,
        })
    }

    fn collect_post_tx_incoming(
        &self,
        pre_tx_incoming: BTreeSet<String>,
        edited_files: &[String],
        removed_files: &[String],
    ) -> Vec<String> {
        let mut touched_targets: BTreeSet<String> = edited_files.iter().cloned().collect();
        touched_targets.extend(removed_files.iter().cloned());
        let mut incoming_set: BTreeSet<String> = pre_tx_incoming
            .into_iter()
            .filter(|p| !touched_targets.contains(p))
            .collect();
        if !touched_targets.is_empty() {
            let target_list: Vec<String> = touched_targets.iter().cloned().collect();
            if let Ok(post_callers) = self.incoming_caller_files_for_paths(&target_list) {
                for src in post_callers {
                    if !touched_targets.contains(&src) {
                        incoming_set.insert(src);
                    }
                }
            }
        }
        incoming_set.into_iter().collect()
    }

    /// Reads the fast O(1) `ir_summary` counters from SQLite.
    pub(crate) fn summary_counts(&self) -> Result<Option<IrSummary>> {
        self.with_read_conn(read_summary_from_conn)
    }

    /// Returns all indexed file paths in `ir_files`.
    pub(crate) fn known_files(&self) -> Result<BTreeSet<String>> {
        self.with_read_conn(|conn| {
            let mut stmt = conn.prepare("SELECT path FROM ir_files")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            let mut set = BTreeSet::new();
            for row in rows {
                set.insert(row?);
            }
            Ok(set)
        })
    }

    /// Builds or returns the epoch-cached `ReexportIndex` from SQLite `ir_reexports` and `ir_symbols`.
    pub(crate) fn reexport_index(&self) -> Result<ReexportIndex> {
        let epoch = self.current_write_epoch();
        if let Ok(guard) = self.reexport_cache.lock()
            && let Some((cached_epoch, ref idx)) = *guard
            && cached_epoch == epoch
        {
            return Ok(idx.clone());
        }
        let fresh = self.with_read_conn(build_reexport_index_from_db)?;
        if let Ok(mut guard) = self.reexport_cache.lock() {
            *guard = Some((epoch, fresh.clone()));
        }
        Ok(fresh)
    }

    /// Queries reverse-caller edges targeting `target_rel` directly from SQLite `ir_edges`.
    pub(crate) fn incoming_caller_files(&self, target_rel: &str) -> Result<Vec<CallerEdgeRow>> {
        self.with_read_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT source_file, target_symbol, relation, context, line
                 FROM ir_edges
                 WHERE target_file = ?1 AND source_file != ?1
                 ORDER BY source_file, line, id",
            )?;
            let rows = stmt.query_map([target_rel], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?.max(0) as u32,
                ))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Into::into)
        })
    }

    /// Queries deduplicated 1-hop reverse-caller relative paths targeting any file in `target_paths`,
    /// excluding self-matches within `target_paths`.
    pub(crate) fn incoming_caller_files_for_paths(
        &self,
        target_paths: &[String],
    ) -> Result<Vec<String>> {
        let mut targets = BTreeSet::new();
        for raw in target_paths {
            let rel = clean_rel_key(raw);
            if !rel.is_empty() {
                targets.insert(rel);
            }
        }
        if targets.is_empty() {
            return Ok(Vec::new());
        }
        let callers =
            self.with_read_conn(|conn| query_incoming_callers_from_conn(conn, &targets))?;
        Ok(callers
            .into_iter()
            .filter(|src| !targets.contains(src))
            .collect())
    }

    /// Read-only verification of an indexed relative file path and/or symbol declaration against
    /// `ir_files` and `ir_symbols` (safe on cached read-only `GraphIrStore` handles).
    ///
    /// When `rel_path` is `None`, `path_exists` stays `false` while `matched_files` reports every
    /// indexed file defining a matching symbol.
    pub(crate) fn verify_path_or_symbol(
        &self,
        rel_path: Option<&str>,
        symbol: Option<&str>,
    ) -> Result<IrPathSymbolVerification> {
        let norm_path = rel_path.map(clean_rel_key).filter(|p| !p.is_empty());
        let raw_sym = symbol.map(str::trim).filter(|s| !s.is_empty());
        if norm_path.is_none() && raw_sym.is_none() {
            return Ok(IrPathSymbolVerification::default());
        }
        self.with_read_conn(|conn| {
            let mut matched_files = BTreeSet::new();
            let mut matched_symbols = BTreeSet::new();
            let path_exists = if let Some(ref rel) = norm_path {
                let exists: bool = conn
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM ir_files WHERE path = ?1)",
                        [rel],
                        |r| r.get(0),
                    )
                    .unwrap_or(false);
                if exists {
                    matched_files.insert(rel.clone());
                }
                exists
            } else {
                false
            };

            if let Some(sym_query) = raw_sym {
                let (qualifier, leaf) = split_symbol_query(sym_query);
                let mut collect_row =
                    |file_path: String,
                     symbol_id: String,
                     name: String,
                     parent_name: Option<String>| {
                        if symbol_matches_query(
                            sym_query,
                            qualifier,
                            leaf,
                            &symbol_id,
                            &name,
                            parent_name.as_deref(),
                        ) {
                            matched_files.insert(file_path);
                            matched_symbols.insert(symbol_id);
                        }
                    };
                if let Some(ref rel) = norm_path {
                    let mut stmt = conn.prepare(
                        "SELECT file_path, symbol_id, name, parent_name
                         FROM ir_symbols
                         WHERE file_path = ?1
                           AND (symbol_id = ?2 OR name = ?2 OR name = ?3)
                         ORDER BY start_line, id
                         LIMIT 64",
                    )?;
                    let mut rows = stmt.query(params![rel, sym_query, leaf])?;
                    while let Some(row) = rows.next()? {
                        collect_row(row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?);
                    }
                } else {
                    let mut stmt = conn.prepare(
                        "SELECT file_path, symbol_id, name, parent_name
                         FROM ir_symbols
                         WHERE symbol_id = ?1 OR name = ?1 OR name = ?2
                         ORDER BY file_path, start_line, id
                         LIMIT 64",
                    )?;
                    let mut rows = stmt.query(params![sym_query, leaf])?;
                    while let Some(row) = rows.next()? {
                        collect_row(row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?);
                    }
                }
            }

            Ok(IrPathSymbolVerification {
                path_exists,
                symbol_exists: !matched_symbols.is_empty(),
                matched_files: matched_files.into_iter().collect(),
                matched_symbols: matched_symbols.into_iter().collect(),
            })
        })
    }

    /// Returns all indexed file `(size, mtime_ns)` stamps alongside `manifest_stamp` and `git_head`.
    pub(crate) fn file_stamps_and_meta(&self) -> Result<FileStampsSnapshot> {
        self.with_read_conn(|conn| {
            let mut stmt = conn.prepare("SELECT path, size, mtime_ns FROM ir_files")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    (row.get::<_, i64>(1)?, row.get::<_, i64>(2)?),
                ))
            })?;
            let mut stamps = BTreeMap::new();
            for row in rows {
                let (path, stamp) = row?;
                stamps.insert(path, stamp);
            }
            let manifest_stamp: Option<String> = conn
                .query_row(
                    "SELECT value FROM ir_meta WHERE key = 'manifest_stamp'",
                    [],
                    |r| r.get(0),
                )
                .optional()?;
            let git_head: Option<String> = conn
                .query_row(
                    "SELECT value FROM ir_meta WHERE key = 'git_head'",
                    [],
                    |r| r.get(0),
                )
                .optional()?;
            Ok((stamps, manifest_stamp, git_head))
        })
    }

    /// Returns deduplicated `(source_file, target_file)` dependency pairs from `ir_edges`.
    pub(crate) fn all_resolved_file_edges(&self) -> Result<Vec<(String, String)>> {
        self.with_read_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT DISTINCT source_file, target_file FROM ir_edges WHERE source_file != target_file",
            )?;
            let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Into::into)
        })
    }

    /// Batch-loads fresh outgoing `reads`/`writes` dependencies for `rel_paths` using ONE lock and
    /// bounded `ir_files` + `ir_edges` SQL queries, validating source, target, and manifest freshness.
    pub(crate) fn batch_access_deps_if_fresh(
        &self,
        root: &Path,
        rel_paths: &[&str],
    ) -> Result<BTreeMap<String, Vec<DepRef>>> {
        let mut disk_stamps: BTreeMap<String, (i64, i64)> = BTreeMap::new();
        for &raw in rel_paths {
            let rel = clean_rel_key(raw);
            if rel.is_empty() || disk_stamps.contains_key(&rel) {
                continue;
            }
            if let Ok(meta) = fs::symlink_metadata(root.join(&rel))
                && !meta.file_type().is_symlink()
                && meta.is_file()
            {
                disk_stamps.insert(
                    rel,
                    (
                        i64::try_from(meta.len()).unwrap_or(i64::MAX),
                        mtime_nanos(&meta),
                    ),
                );
            }
        }
        if disk_stamps.is_empty() {
            return Ok(BTreeMap::new());
        }

        let keys: Vec<String> = disk_stamps.keys().cloned().collect();
        self.with_read_conn(|conn| {
            if !manifests_fresh_against_meta(root, conn) {
                return Ok(BTreeMap::new());
            }
            let mut out: BTreeMap<String, Vec<DepRef>> = BTreeMap::new();
            let mut target_fresh: HashMap<String, bool> = HashMap::new();
            for chunk in keys.chunks(500) {
                let placeholders = std::iter::repeat_n("?", chunk.len())
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!(
                    "SELECT f.path, f.size, f.mtime_ns, e.target_file, e.target_symbol, e.relation, e.context, e.line
                     FROM ir_files f
                     LEFT JOIN ir_edges e
                       ON e.source_file = f.path
                      AND e.target_file != e.source_file
                      AND e.relation IN ('reads', 'writes')
                     WHERE f.path IN ({placeholders})
                     ORDER BY f.path, e.id"
                );
                let mut stmt = conn.prepare(&sql)?;
                let params_vec: Vec<&dyn rusqlite::ToSql> =
                    chunk.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
                let mut grouped_by_source: BTreeMap<String, BTreeMap<(String, String), DepRef>> =
                    BTreeMap::new();
                let mut fresh_paths: BTreeSet<String> = BTreeSet::new();
                let mut stale_sources: BTreeSet<String> = BTreeSet::new();
                let rows = stmt.query_map(params_vec.as_slice(), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<i64>>(7)?,
                    ))
                })?;
                for row in rows {
                    let (
                        source_file,
                        db_size,
                        db_mtime,
                        target_file,
                        target_symbol,
                        relation,
                        context,
                        line,
                    ) = row?;
                    if disk_stamps.get(&source_file) != Some(&(db_size, db_mtime))
                        || stale_sources.contains(&source_file)
                    {
                        continue;
                    }
                    if let Some(ref tf) = target_file
                        && !is_ir_file_fresh_cached(conn, root, tf, &mut target_fresh)
                    {
                        stale_sources.insert(source_file.clone());
                        fresh_paths.remove(&source_file);
                        grouped_by_source.remove(&source_file);
                        continue;
                    }
                    fresh_paths.insert(source_file.clone());
                    let grouped = grouped_by_source.entry(source_file).or_default();
                    if let (Some(target_file), Some(relation)) = (target_file, relation) {
                        let line_u32 = line.unwrap_or(0).max(0) as u32;
                        push_stored_edge_into_dep_map(
                            grouped,
                            target_file,
                            target_symbol,
                            relation,
                            context,
                            line_u32,
                            None,
                        );
                    }
                }
                for path in fresh_paths {
                    let deps = grouped_by_source
                        .remove(&path)
                        .map(|g| g.into_values().collect())
                        .unwrap_or_default();
                    out.insert(path, deps);
                }
            }
            Ok(out)
        })
    }

    /// Returns stored declarations and outgoing dependencies for `rel_path` when source, target
    /// files, and manifest stamps still match SQLite records.
    pub(crate) fn file_outgoing_deps_if_fresh(
        &self,
        root: &Path,
        rel_path: &str,
    ) -> Result<Option<(Vec<DepRef>, Vec<DeclaredMember>)>> {
        let rel = clean_rel_key(rel_path);
        if rel.is_empty() {
            return Ok(None);
        }
        let disk_stamp = match fs::symlink_metadata(root.join(&rel)) {
            Ok(meta) if !meta.file_type().is_symlink() && meta.is_file() => (
                i64::try_from(meta.len()).unwrap_or(i64::MAX),
                mtime_nanos(&meta),
            ),
            _ => return Ok(None),
        };
        self.with_read_conn(|conn| {
            if !manifests_fresh_against_meta(root, conn) {
                return Ok(None);
            }
            let stored_stamp: Option<(i64, i64)> = conn
                .query_row(
                    "SELECT size, mtime_ns FROM ir_files WHERE path = ?1",
                    [&rel],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if stored_stamp != Some(disk_stamp) {
                return Ok(None);
            }

            let declared = read_declared_members_from_conn(conn, &rel)?;

            let mut target_members_cache: BTreeMap<String, Vec<DeclaredMember>> = BTreeMap::new();
            let mut target_fresh: HashMap<String, bool> = HashMap::new();
            let mut grouped: BTreeMap<(String, String), DepRef> = BTreeMap::new();

            {
                let mut stmt = conn.prepare(
                    "SELECT target_file, target_symbol, relation, context, line
                     FROM ir_edges
                     WHERE source_file = ?1 AND target_file != ?1
                     ORDER BY id",
                )?;
                let rows = stmt.query_map([&rel], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, i64>(4)?.max(0) as u32,
                    ))
                })?;
                for row in rows {
                    let (target_file, target_symbol, relation, context, line) = row?;
                    if !is_ir_file_fresh_cached(conn, root, &target_file, &mut target_fresh) {
                        return Ok(None);
                    }
                    let mut load_members = |target: &str| {
                        target_members_cache
                            .entry(target.to_owned())
                            .or_insert_with(|| {
                                read_declared_members_from_conn(conn, target).unwrap_or_default()
                            })
                            .clone()
                    };
                    push_stored_edge_into_dep_map(
                        &mut grouped,
                        target_file,
                        target_symbol,
                        relation,
                        context,
                        line,
                        Some(&mut load_members),
                    );
                }
            }

            {
                let mut stmt = conn.prepare(
                    "SELECT specifier, kind, symbols_json, line
                     FROM ir_unresolved_imports
                     WHERE source_file = ?1
                     ORDER BY id",
                )?;
                let rows = stmt.query_map([&rel], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?.max(0) as u32,
                    ))
                })?;
                for row in rows {
                    let (specifier, kind, symbols_json, line) = row?;
                    let symbols: Vec<String> =
                        serde_json::from_str(&symbols_json).unwrap_or_default();
                    let entry = grouped
                        .entry((specifier.clone(), kind.clone()))
                        .or_insert_with(|| DepRef {
                            target: specifier,
                            resolved: false,
                            exists: false,
                            kind,
                            symbols: Vec::new(),
                            line: Some(line),
                            evidence: None,
                            member_refs: Vec::new(),
                            target_members: Vec::new(),
                        });
                    for sym in symbols {
                        if !entry.symbols.contains(&sym) {
                            entry.symbols.push(sym);
                        }
                    }
                }
            }

            Ok(Some((grouped.into_values().collect(), declared)))
        })
    }

    /// Returns stored `(symbol_id, kind, name, parent_name, start_line)` rows for `rel_path`
    /// when `ir_files` stamp matches disk `(size, mtime_ns)`.
    #[allow(clippy::type_complexity)]
    pub(crate) fn fresh_file_symbols(
        &self,
        root: &Path,
        rel_path: &str,
    ) -> Result<Option<Vec<(String, String, String, Option<String>, u32)>>> {
        let rel = clean_rel_key(rel_path);
        if rel.is_empty() {
            return Ok(None);
        }
        let disk_stamp = match fs::symlink_metadata(root.join(&rel)) {
            Ok(meta) if !meta.file_type().is_symlink() && meta.is_file() => (
                i64::try_from(meta.len()).unwrap_or(i64::MAX),
                mtime_nanos(&meta),
            ),
            _ => return Ok(None),
        };
        self.with_read_conn(|conn| {
            let stored_stamp: Option<(i64, i64)> = conn
                .query_row(
                    "SELECT size, mtime_ns FROM ir_files WHERE path = ?1",
                    [&rel],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if stored_stamp != Some(disk_stamp) {
                return Ok(None);
            }
            let mut stmt = conn.prepare(
                "SELECT symbol_id, kind, name, parent_name, start_line
                 FROM ir_symbols
                 WHERE file_path = ?1
                 ORDER BY start_line, id",
            )?;
            let rows = stmt.query_map([&rel], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?.max(0) as u32,
                ))
            })?;
            Ok(Some(rows.collect::<std::result::Result<Vec<_>, _>>()?))
        })
    }

    /// Maps canonical `{file_path}::{owner}::{name}` / `{file_path}::{name}` keys to
    /// stored `ir_symbols.symbol_id` values across the supplied `file_paths`.
    pub(crate) fn canonical_symbol_ids_for_files(
        &self,
        file_paths: &BTreeSet<String>,
    ) -> Result<BTreeMap<String, String>> {
        let mut out = BTreeMap::new();
        if file_paths.is_empty() {
            return Ok(out);
        }
        self.with_read_conn(|conn| {
            let keys: Vec<&String> = file_paths.iter().collect();
            for chunk in keys.chunks(200) {
                let placeholders = std::iter::repeat_n("?", chunk.len())
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!(
                    "SELECT file_path, symbol_id, name, parent_name
                     FROM ir_symbols
                     WHERE file_path IN ({placeholders})
                     ORDER BY id"
                );
                let mut stmt = conn.prepare(&sql)?;
                let params_vec: Vec<&dyn rusqlite::ToSql> =
                    chunk.iter().map(|s| *s as &dyn rusqlite::ToSql).collect();
                let rows = stmt.query_map(params_vec.as_slice(), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                })?;
                for row in rows {
                    let (file_path, symbol_id, name, parent_name) = row?;
                    let owner = parent_name
                        .as_deref()
                        .map(str::trim)
                        .filter(|p| !p.is_empty() && *p != file_path);
                    let fqn = match owner {
                        Some(o) => format!("{file_path}::{o}::{name}"),
                        None => format!("{file_path}::{name}"),
                    };
                    out.entry(fqn).or_insert(symbol_id);
                }
            }
            Ok(out)
        })
    }

    /// Reads indexed file paths and semantic edges directly for System Graph aggregation.
    pub(crate) fn semantic_edges_for_system(
        &self,
    ) -> Result<(Vec<String>, Vec<SystemSemanticEdgeRow>)> {
        self.with_read_conn(|conn| {
            let files = {
                let mut stmt = conn.prepare("SELECT path FROM ir_files ORDER BY path")?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                rows.collect::<std::result::Result<Vec<_>, _>>()?
            };
            let sem_edges = {
                let mut stmt = conn.prepare(
                    "SELECT file_path, kind, target, source_symbol, line, evidence
                     FROM ir_semantic_edges
                     ORDER BY id",
                )?;
                let rows = stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, i64>(4)?.max(0) as u32,
                        row.get::<_, String>(5)?,
                    ))
                })?;
                rows.collect::<std::result::Result<Vec<_>, _>>()?
            };
            Ok((files, sem_edges))
        })
    }

    /// Exports typed node and link specs directly from SQLite so `graph_map::Index` avoids JSON.
    pub(crate) fn export_ir_graph_elements(
        &self,
    ) -> Result<(Vec<IrNodeSpec>, Vec<IrLinkSpec>, u64, u64)> {
        let rows = self.load_graph_rows()?;
        Ok(collect_graph_elements(
            &rows.files,
            &rows.symbols,
            &rows.edges,
            &rows.sem_edges,
        ))
    }

    /// Exports incremental nodes/links for `Index::update_files`, supporting regular code
    /// edges, `ir_reexports` propagation, and `ir_semantic_edges` (`sem:*` & `di_binds`).
    pub(crate) fn export_ir_delta_for_files(
        &self,
        delta: &IrUpdateDelta,
    ) -> Result<Option<IrDeltaElements>> {
        if delta.manifest_changed {
            return Ok(None);
        }
        let mut effective_edited: BTreeSet<String> = delta.edited_files.iter().cloned().collect();
        let removed_files: BTreeSet<String> = delta.removed_files.iter().cloned().collect();
        let mut dirty_files: BTreeSet<String> = effective_edited.clone();
        dirty_files.extend(removed_files.iter().cloned());
        dirty_files.extend(delta.re_resolved_files.iter().cloned());
        dirty_files.extend(delta.incoming_caller_files.iter().cloned());
        dirty_files.extend(delta.touched_di_files.iter().cloned());
        if dirty_files.is_empty() || dirty_files.len() > 256 {
            return Ok(None);
        }
        let rs_touched = effective_edited
            .iter()
            .chain(removed_files.iter())
            .any(|p| p.ends_with(".rs") || p.ends_with(".rs.modules"));

        self.with_read_conn(|conn| {
            let files = {
                let mut stmt = conn.prepare("SELECT path FROM ir_files ORDER BY path")?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                rows.collect::<std::result::Result<Vec<_>, _>>()?
            };
            let file_set: BTreeSet<&str> = files.iter().map(String::as_str).collect();

            for prev_anchor in &delta.prev_semantic_anchor_files {
                if file_set.contains(prev_anchor.as_str()) {
                    effective_edited.insert(prev_anchor.clone());
                    dirty_files.insert(prev_anchor.clone());
                }
            }

            // Discover all canonical semantic targets & DI bindings touched by dirty or edited files.
            let mut touched_sem_targets: BTreeSet<String> =
                delta.touched_semantic_targets.iter().cloned().collect();
            let mut touched_di_targets: BTreeSet<String> = BTreeSet::new();
            let seed_sem_files: Vec<String> = effective_edited
                .iter()
                .chain(dirty_files.iter())
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            for chunk in seed_sem_files.chunks(200) {
                let placeholders = (1..=chunk.len())
                    .map(|i| format!("?{i}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!(
                    "SELECT kind, target FROM ir_semantic_edges WHERE file_path IN ({placeholders})"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?;
                for row in rows {
                    let (kind, target) = row?;
                    if kind == "di_binds" {
                        touched_di_targets.insert(target);
                    } else {
                        touched_sem_targets.insert(canonical_sem_target(&kind, &target));
                    }
                }
            }

            let sem_targets_vec: Vec<String> = touched_sem_targets.iter().cloned().collect();
            let mut sem_rows = query_semantic_rows_for_targets(conn, &sem_targets_vec)?;

            // Add all `di_binds` rows touching `touched_di_targets` or files in `dirty_files`.
            let mut seen_sem_row_keys: BTreeSet<(String, String, String, u32)> = sem_rows
                .iter()
                .map(|r| (r.file_path.clone(), r.kind.clone(), r.target.clone(), r.line))
                .collect();
            if !touched_di_targets.is_empty() {
                let di_vec: Vec<String> = touched_di_targets.iter().cloned().collect();
                for chunk in di_vec.chunks(200) {
                    let placeholders = (1..=chunk.len())
                        .map(|i| format!("?{i}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let sql = format!(
                        "SELECT file_path, kind, target, source_symbol, line, evidence
                         FROM ir_semantic_edges
                         WHERE kind = 'di_binds' AND target IN ({placeholders})
                         ORDER BY id"
                    );
                    let mut stmt = conn.prepare(&sql)?;
                    let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |row| {
                        Ok(StoredSemanticRow {
                            file_path: row.get(0)?,
                            kind: row.get(1)?,
                            target: row.get(2)?,
                            source_symbol: row.get(3)?,
                            line: u32::try_from(row.get::<_, i64>(4)?).unwrap_or(1),
                            evidence: row.get(5)?,
                        })
                    })?;
                    for r in rows {
                        let r = r?;
                        if seen_sem_row_keys.insert((
                            r.file_path.clone(),
                            r.kind.clone(),
                            r.target.clone(),
                            r.line,
                        )) {
                            sem_rows.push(r);
                        }
                    }
                }
            }

            // Compute new anchors for every touched semantic target and ensure new anchor files
            // are in `effective_edited` so `sem:*` nodes are created/moved/removed cleanly.
            let new_anchors = compute_semantic_anchor_map(&sem_rows, Some(&file_set));
            for (_, anchor_file, _) in new_anchors.values() {
                effective_edited.insert((*anchor_file).to_owned());
                dirty_files.insert((*anchor_file).to_owned());
            }
            for row in &sem_rows {
                if file_set.contains(row.file_path.as_str()) {
                    dirty_files.insert(row.file_path.clone());
                }
            }

            // Ensure any file in `effective_edited` also has ALL of its own semantic rows loaded so
            // unrelated `sem:*` nodes anchored on an edited file are never dropped on replacement.
            let edited_vec: Vec<String> = effective_edited.iter().cloned().collect();
            let mut extra_targets = BTreeSet::new();
            for chunk in edited_vec.chunks(200) {
                let placeholders = (1..=chunk.len())
                    .map(|i| format!("?{i}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!(
                    "SELECT kind, target FROM ir_semantic_edges WHERE file_path IN ({placeholders})"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?;
                for row in rows {
                    let (kind, target) = row?;
                    if kind == "di_binds" {
                        touched_di_targets.insert(target);
                    } else {
                        let canon = canonical_sem_target(&kind, &target);
                        if !touched_sem_targets.contains(&canon) {
                            extra_targets.insert(canon);
                        }
                    }
                }
            }
            if !extra_targets.is_empty() {
                let extra_vec: Vec<String> = extra_targets.into_iter().collect();
                for r in query_semantic_rows_for_targets(conn, &extra_vec)? {
                    if seen_sem_row_keys.insert((
                        r.file_path.clone(),
                        r.kind.clone(),
                        r.target.clone(),
                        r.line,
                    )) {
                        sem_rows.push(r);
                    }
                }
            }

            if dirty_files.len() > 256 {
                return Ok(None);
            }

            let mut di_concrete_files = BTreeSet::new();
            if !touched_di_targets.is_empty() {
                let short_names: Vec<String> = touched_di_targets
                    .iter()
                    .filter_map(|t| t.strip_prefix("service:"))
                    .flat_map(|s| {
                        let tail = s.rsplit(['\\', ':', '.']).next().unwrap_or(s).trim();
                        (!tail.is_empty()).then(|| tail.to_owned())
                    })
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                for chunk in short_names.chunks(200) {
                    let placeholders = (1..=chunk.len())
                        .map(|i| format!("?{i}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let sql = format!(
                        "SELECT DISTINCT file_path FROM ir_symbols WHERE name IN ({placeholders})"
                    );
                    let mut stmt = conn.prepare(&sql)?;
                    let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
                        r.get::<_, String>(0)
                    })?;
                    for r in rows {
                        let fp = r?;
                        if file_set.contains(fp.as_str()) {
                            di_concrete_files.insert(fp);
                        }
                    }
                }
            }

            let dirty_vec: Vec<&str> = dirty_files.iter().map(String::as_str).collect();
            let placeholders = vec!["?"; dirty_vec.len()].join(", ");
            let edge_sql = format!(
                "SELECT DISTINCT id, source_file, source_symbol, target_file, target_symbol, relation, context, line, confidence
                 FROM ir_edges
                 WHERE source_file IN ({placeholders}) OR target_file IN ({placeholders})
                 ORDER BY id"
            );
            let mut edge_params: Vec<&str> = Vec::with_capacity(dirty_vec.len() * 2);
            edge_params.extend(dirty_vec.iter().copied());
            edge_params.extend(dirty_vec.iter().copied());
            let edges: Vec<StoredEdgeRow> = {
                let mut stmt = conn.prepare(&edge_sql)?;
                let rows = stmt.query_map(rusqlite::params_from_iter(edge_params), |row| {
                    Ok(StoredEdgeRow {
                        source_file: row.get(1)?,
                        source_symbol: row.get(2)?,
                        target_file: row.get(3)?,
                        target_symbol: row.get(4)?,
                        relation: row.get(5)?,
                        context: row.get(6)?,
                        line: row.get::<_, i64>(7)?.max(0) as u32,
                        confidence: row.get(8)?,
                    })
                })?;
                rows.collect::<std::result::Result<Vec<_>, _>>()?
            };

            let mut needed_symbol_files: BTreeSet<String> = effective_edited.clone();
            for fp in &di_concrete_files {
                needed_symbol_files.insert(fp.clone());
            }
            for row in &sem_rows {
                if file_set.contains(row.file_path.as_str()) {
                    needed_symbol_files.insert(row.file_path.clone());
                }
            }
            for edge in &edges {
                if file_set.contains(edge.source_file.as_str()) {
                    needed_symbol_files.insert(edge.source_file.clone());
                }
                if file_set.contains(edge.target_file.as_str()) {
                    needed_symbol_files.insert(edge.target_file.clone());
                }
            }
            let sym_files_vec: Vec<&str> =
                needed_symbol_files.iter().map(String::as_str).collect();
            let symbols = if sym_files_vec.is_empty() {
                Vec::new()
            } else if sym_files_vec.len() > 400 {
                return Ok(None);
            } else {
                let sym_placeholders = vec!["?"; sym_files_vec.len()].join(", ");
                let sym_sql = format!(
                    "SELECT id, file_path, symbol_id, kind, name, parent_name, start_line, end_line, is_test, modifiers_json
                     FROM ir_symbols
                     WHERE file_path IN ({sym_placeholders})
                     ORDER BY file_path, start_line, id"
                );
                let mut stmt = conn.prepare(&sym_sql)?;
                let rows = stmt.query_map(
                    rusqlite::params_from_iter(sym_files_vec),
                    map_stored_symbol_row,
                )?;
                rows.collect::<std::result::Result<Vec<_>, _>>()?
            };

            let emit_files: Vec<String> = effective_edited
                .iter()
                .filter(|f| file_set.contains(f.as_str()))
                .cloned()
                .collect();
            let communities = compute_file_communities_for_delta(conn, &files)?;
            let mut used_node_ids = BTreeSet::new();
            for path in &needed_symbol_files {
                if !effective_edited.contains(path) {
                    used_node_ids.insert(format!("file:{path}"));
                }
            }
            let (mut node_specs, mut link_specs, maps) = collect_symbol_nodes_and_maps(
                &emit_files,
                Some(&effective_edited),
                &symbols,
                &communities,
                &mut used_node_ids,
            );
            let (edge_links, resolved_targets_by_source) =
                collect_resolved_edge_links(&edges, &file_set, &maps);
            link_specs.extend(edge_links);

            emit_semantic_nodes_and_links(
                &sem_rows,
                &file_set,
                &maps,
                &resolved_targets_by_source,
                &communities,
                Some(&effective_edited),
                Some(&dirty_files),
                &mut node_specs,
                &mut link_specs,
            );

            let neighbor_files: Vec<String> = needed_symbol_files
                .into_iter()
                .filter(|f| !effective_edited.contains(f) && !removed_files.contains(f))
                .collect();

            Ok(Some(IrDeltaElements {
                files,
                edited_files: effective_edited,
                dirty_files,
                removed_files,
                neighbor_files,
                rs_touched,
                node_specs,
                link_specs,
            }))
        })
    }

    fn load_graph_rows(&self) -> Result<LoadedGraphRows> {
        self.with_read_conn(|conn| {
            let files = {
                let mut stmt = conn.prepare("SELECT path FROM ir_files ORDER BY path")?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                rows.collect::<std::result::Result<Vec<_>, _>>()?
            };

            let symbols = {
                let mut stmt = conn.prepare(
                    "SELECT id, file_path, symbol_id, kind, name, parent_name, start_line, end_line, is_test, modifiers_json
                     FROM ir_symbols
                     ORDER BY file_path, start_line, id",
                )?;
                let rows = stmt.query_map([], map_stored_symbol_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()?
            };

            let edges = {
                let mut stmt = conn.prepare(
                    "SELECT source_file, source_symbol, target_file, target_symbol, relation, context, line, confidence
                     FROM ir_edges
                     ORDER BY id",
                )?;
                let rows = stmt.query_map([], |row| {
                    Ok(StoredEdgeRow {
                        source_file: row.get(0)?,
                        source_symbol: row.get(1)?,
                        target_file: row.get(2)?,
                        target_symbol: row.get(3)?,
                        relation: row.get(4)?,
                        context: row.get(5)?,
                        line: row.get::<_, i64>(6)?.max(0) as u32,
                        confidence: row.get(7)?,
                    })
                })?;
                rows.collect::<std::result::Result<Vec<_>, _>>()?
            };

            let sem_edges = {
                let mut stmt = conn.prepare(
                    "SELECT file_path, kind, target, source_symbol, line, evidence
                     FROM ir_semantic_edges
                     ORDER BY id",
                )?;
                let rows = stmt.query_map([], |row| {
                    Ok(StoredSemanticRow {
                        file_path: row.get(0)?,
                        kind: row.get(1)?,
                        target: row.get(2)?,
                        source_symbol: row.get(3)?,
                        line: row.get::<_, i64>(4)?.max(0) as u32,
                        evidence: row.get(5)?,
                    })
                })?;
                rows.collect::<std::result::Result<Vec<_>, _>>()?
            };

            Ok(LoadedGraphRows {
                files,
                symbols,
                edges,
                sem_edges,
            })
        })
    }

    /// Exports Graphify-compatible `{"nodes": [...], "links": [...]}` JSON for `graph_map::Index::read`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn export_node_link_json(&self) -> Result<Value> {
        self.export_node_link_json_with_counts().map(|(v, ..)| v)
    }

    /// Exports node-link JSON alongside the exported `(nodes, links)` counts.
    pub(crate) fn export_node_link_json_with_counts(&self) -> Result<(Value, u64, u64)> {
        let rows = self.load_graph_rows()?;
        Ok(build_node_link_payload(
            &rows.files,
            &rows.symbols,
            &rows.edges,
            &rows.sem_edges,
        ))
    }

    /// Writes `live/graph.json` via a private temporary file and atomic rename without removing
    /// the previous published `live/graph.json` while building, streaming directly without
    /// building an intermediate `serde_json::Value` DOM.
    pub(crate) fn write_live_graph_atomic(&self, live_graph_path: &Path) -> Result<(u64, u64)> {
        let rows = self.load_graph_rows()?;
        let (node_specs, link_specs, node_count, link_count) =
            collect_graph_elements(&rows.files, &rows.symbols, &rows.edges, &rows.sem_edges);
        write_live_graph_from_elements_atomic(live_graph_path, &node_specs, &link_specs)?;
        Ok((node_count, link_count))
    }
}

/// Streams already-exported `IrNodeSpec` / `IrLinkSpec` slices directly into `live/graph.json`
/// via a buffered temporary file and atomic rename without allocating `Value` or `Vec<u8>`.
pub(crate) fn write_live_graph_from_elements_atomic(
    live_graph_path: &Path,
    node_specs: &[IrNodeSpec],
    link_specs: &[IrLinkSpec],
) -> Result<()> {
    write_live_graph_json_stream_atomic(live_graph_path, |writer| {
        write_node_link_payload_to_writer(writer, node_specs, link_specs)
    })
}

/// Encodes already-exported `IrNodeSpec` / `IrLinkSpec` slices into `live/graph.json` bytes
/// using a streaming serializer without allocating `serde_json::Value` DOM trees.
pub(crate) fn encode_node_link_payload_from_elements(
    node_specs: &[IrNodeSpec],
    link_specs: &[IrLinkSpec],
) -> Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(
        64 + node_specs.len().saturating_mul(160) + link_specs.len().saturating_mul(128),
    );
    write_node_link_payload_to_writer(&mut bytes, node_specs, link_specs)?;
    Ok(bytes)
}

/// Atomically writes serialized `live/graph.json` bytes with directory/file custody checks.
pub(crate) fn write_live_graph_json_bytes_atomic(
    live_graph_path: &Path,
    bytes: &[u8],
) -> Result<()> {
    write_live_graph_json_stream_atomic(live_graph_path, |writer| {
        writer.write_all(bytes)?;
        Ok(())
    })
}

fn write_live_graph_json_stream_atomic<F>(live_graph_path: &Path, write_fn: F) -> Result<()>
where
    F: FnOnce(&mut BufWriter<File>) -> Result<()>,
{
    let parent = live_graph_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| HarnessError::invalid("Live graph path has no parent directory."))?;
    let leaf = live_graph_path
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty() && !s.contains(['/', '\\', '\0']))
        .ok_or_else(|| HarnessError::invalid("Invalid live graph filename."))?;

    infra_host::util::private_directory(parent)?;
    let custody = PublicationFiles::new(parent)?;
    let real_parent = infra_core::fs::python_realpath(parent)?;
    let dest = real_parent.join(leaf);
    infra_core::fs::admit_descendant(&real_parent, &dest)?;
    if let Ok(meta) = fs::symlink_metadata(&dest)
        && (meta.file_type().is_symlink() || !meta.is_file())
    {
        return Err(HarnessError::forbidden(
            "Live graph destination is not an ordinary file.",
        ));
    }

    let tmp_name = format!(".{leaf}.{}.tmp", uuid::Uuid::new_v4().simple());
    let tmp_path = real_parent.join(&tmp_name);
    let mut opts = OpenOptions::new();
    opts.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let write_result = (|| -> Result<()> {
        let file = opts.open(&tmp_path)?;
        let mut writer = BufWriter::with_capacity(64 * 1024, file);
        write_fn(&mut writer)?;
        writer.flush()?;
        let file = writer
            .into_inner()
            .map_err(|err| HarnessError::from(err.into_error()))?;
        file.sync_all()?;
        drop(file);
        custody.check()?;
        fs::rename(&tmp_path, &dest)?;
        #[cfg(unix)]
        if let Ok(dir_handle) = File::open(&real_parent) {
            let _ = dir_handle.sync_all();
        }
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    write_result
}

/// Normalizes a raw workspace or absolute file path into a clean relative repo path.
///
/// Rejects path traversal (`..`), ignored build/dependency directories, and paths outside `root`.
pub(crate) fn normalize_rel_path(root: &Path, raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        return None;
    }
    let normalized_raw = trimmed.replace('\\', "/");
    let candidate = Path::new(&normalized_raw);
    let rel_str = if candidate.is_absolute() {
        if let Ok(rel) = candidate.strip_prefix(root) {
            rel.to_string_lossy().replace('\\', "/")
        } else {
            let canon_root = fs::canonicalize(root).ok()?;
            if let Ok(rel) = candidate.strip_prefix(&canon_root) {
                rel.to_string_lossy().replace('\\', "/")
            } else if let Ok(canon_file) = fs::canonicalize(candidate) {
                canon_file
                    .strip_prefix(&canon_root)
                    .ok()?
                    .to_string_lossy()
                    .replace('\\', "/")
            } else {
                let parent = candidate.parent()?;
                let leaf = candidate.file_name()?.to_str()?;
                let canon_parent = fs::canonicalize(parent).ok()?;
                let rel_parent = canon_parent.strip_prefix(&canon_root).ok()?;
                if rel_parent.as_os_str().is_empty() {
                    leaf.to_owned()
                } else {
                    format!(
                        "{}/{}",
                        rel_parent.to_string_lossy().replace('\\', "/"),
                        leaf
                    )
                }
            }
        }
    } else {
        normalized_raw
    };

    let clean = clean_rel_key(&rel_str);
    if clean.is_empty() {
        return None;
    }
    if clean.split('/').any(|seg| IGNORED_CODE_DIRS.contains(&seg)) {
        return None;
    }
    Some(clean)
}

#[derive(Deserialize)]
struct LiveNodeDeser {
    id: String,
    label: String,
    source_file: String,
    #[serde(default)]
    source_location: Option<String>,
    #[serde(default)]
    end_line: u32,
    #[serde(default)]
    file_type: Option<String>,
    #[serde(default)]
    node_kind: Option<String>,
    #[serde(default)]
    community: Option<u32>,
    #[serde(default)]
    community_name: Option<String>,
    #[serde(rename = "_callable_class", default)]
    callable_class: bool,
    #[serde(rename = "abstract", default)]
    is_abstract: bool,
    #[serde(default)]
    is_test: bool,
}

impl From<LiveNodeDeser> for IrNodeSpec {
    fn from(n: LiveNodeDeser) -> Self {
        Self {
            id: n.id,
            label: n.label,
            source_file: n.source_file,
            source_location: n.source_location,
            end_line: n.end_line,
            file_type: n.file_type,
            node_kind: n.node_kind,
            community: n.community,
            community_name: n.community_name,
            callable_class: n.callable_class,
            is_abstract: n.is_abstract,
            is_test: n.is_test,
        }
    }
}

#[derive(Deserialize)]
struct LiveLinkDeser {
    source: String,
    target: String,
    #[serde(default)]
    relation: Option<String>,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    confidence: Option<String>,
    #[serde(default)]
    source_file: Option<String>,
    #[serde(default)]
    evidence: Option<String>,
}

impl From<LiveLinkDeser> for IrLinkSpec {
    fn from(l: LiveLinkDeser) -> Self {
        Self {
            source: l.source,
            target: l.target,
            relation: l.relation,
            context: l.context,
            confidence: l.confidence,
            source_file: l.source_file,
            evidence: l.evidence,
        }
    }
}

#[derive(Deserialize)]
struct LiveGraphDocDeser {
    #[serde(default)]
    nodes: Vec<LiveNodeDeser>,
    #[serde(default)]
    links: Vec<LiveLinkDeser>,
}

impl GraphIrStore {
    /// Returns the persisted AST extraction for `rel_path` without disk parsing when file stamp matches.
    pub(crate) fn load_file_extraction_if_fresh(
        &self,
        root: &Path,
        rel_path: &str,
    ) -> Result<Option<AstFileExtraction>> {
        let rel = clean_rel_key(rel_path);
        if rel.is_empty() {
            return Ok(None);
        }
        let Ok(real_root) = fs::canonicalize(root) else {
            return Ok(None);
        };
        let full = real_root.join(&rel);
        let Ok(meta) = fs::symlink_metadata(&full) else {
            return Ok(None);
        };
        if !meta.is_file() {
            return Ok(None);
        }
        let disk_size = i64::try_from(meta.len()).unwrap_or(i64::MAX);
        let disk_mtime = mtime_nanos(&meta);

        self.with_read_conn(|conn| {
            let row: Option<(i64, i64, String)> = conn
                .query_row(
                    "SELECT f.size, f.mtime_ns, e.summary_json
                     FROM ir_files f
                     JOIN ir_file_extractions e ON e.file_path = f.path
                     WHERE f.path = ?1",
                    [&rel],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let Some((size_bytes, mtime_ns, json_str)) = row else {
                return Ok(None);
            };
            if size_bytes != disk_size || mtime_ns != disk_mtime {
                return Ok(None);
            }
            Ok(serde_json::from_str::<AstFileExtraction>(&json_str).ok())
        })
    }

    /// Returns `true` when the indexed v3 `ir_symbol_refs` table exists and has at least one row.
    pub(crate) fn has_symbol_refs_index(&self) -> Result<bool> {
        self.with_read_conn(|conn| {
            let schema_num: u32 = conn
                .query_row(
                    "SELECT value FROM ir_meta WHERE key = 'schema_version'",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            if schema_num < 3 {
                return Ok(false);
            }
            let table_exists: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='ir_symbol_refs')",
                    [],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or(false);
            if !table_exists {
                return Ok(false);
            }
            let exists: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM ir_symbol_refs LIMIT 1)",
                    [],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or(false);
            Ok(exists)
        })
    }

    /// Queries indexed files referencing any of `tokens` via SQLite `ir_symbol_refs`.
    pub(crate) fn lookup_symbol_ref_callers(&self, tokens: &[&str]) -> Result<Vec<String>> {
        let lowered: BTreeSet<String> = tokens
            .iter()
            .map(|t| t.trim().to_ascii_lowercase())
            .filter(|t| t.len() >= 2)
            .collect();
        if lowered.is_empty() {
            return Ok(Vec::new());
        }
        self.with_read_conn(|conn| {
            let mut found = BTreeSet::new();
            let list: Vec<String> = lowered.into_iter().collect();
            for chunk in list.chunks(200) {
                let placeholders = (1..=chunk.len())
                    .map(|i| format!("?{i}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!(
                    "SELECT DISTINCT file_path FROM ir_symbol_refs WHERE token_lower IN ({placeholders})"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
                    r.get::<_, String>(0)
                })?;
                for row in rows {
                    found.insert(row?);
                }
            }
            Ok(found.into_iter().collect())
        })
    }

    /// Matches lowercased probe keywords against `ir_symbol_refs` in SQLite without disk reads.
    pub(crate) fn find_probe_matching_files(
        &self,
        tokens: &[&str],
    ) -> Result<Vec<(String, Vec<String>)>> {
        let lowered: Vec<String> = tokens
            .iter()
            .map(|t| t.trim().to_ascii_lowercase())
            .filter(|t| t.len() >= 2)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if lowered.is_empty() {
            return Ok(Vec::new());
        }
        self.with_read_conn(|conn| {
            let mut by_file: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for chunk in lowered.chunks(200) {
                let placeholders = (1..=chunk.len())
                    .map(|i| format!("?{i}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let sql = format!(
                    "SELECT file_path, token_lower FROM ir_symbol_refs WHERE token_lower IN ({placeholders})"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?;
                for row in rows {
                    let (fp, tok) = row?;
                    by_file.entry(fp).or_default().push(tok);
                }
            }
            Ok(by_file.into_iter().collect())
        })
    }

    /// Patches `live/graph.json` incrementally from queued IR deltas, falling back to a full
    /// export when the existing file is missing or cannot be read cleanly.
    pub(crate) fn patch_live_graph_from_deltas_atomic(
        &self,
        live_graph_path: &Path,
        deltas: &[&IrDeltaElements],
    ) -> Result<(u64, u64)> {
        if deltas.is_empty() {
            return self.write_live_graph_atomic(live_graph_path);
        }
        let Ok(file) = File::open(live_graph_path) else {
            return self.write_live_graph_atomic(live_graph_path);
        };
        let mut doc: LiveGraphDocDeser =
            match serde_json::from_reader(std::io::BufReader::new(file)) {
                Ok(d) => d,
                Err(_) => return self.write_live_graph_atomic(live_graph_path),
            };
        let mut nodes: Vec<IrNodeSpec> = doc.nodes.drain(..).map(IrNodeSpec::from).collect();
        let mut links: Vec<IrLinkSpec> = doc.links.drain(..).map(IrLinkSpec::from).collect();

        for d in deltas {
            let drop_files: BTreeSet<&str> = d
                .removed_files
                .iter()
                .chain(d.edited_files.iter())
                .map(String::as_str)
                .collect();
            let dirty_files: BTreeSet<&str> = d
                .dirty_files
                .iter()
                .chain(d.removed_files.iter())
                .chain(d.edited_files.iter())
                .map(String::as_str)
                .collect();
            let new_node_ids: BTreeSet<&str> = d.node_specs.iter().map(|n| n.id.as_str()).collect();
            let dirty_node_ids: BTreeSet<String> = nodes
                .iter()
                .filter(|n| {
                    dirty_files.contains(n.source_file.as_str())
                        || new_node_ids.contains(n.id.as_str())
                })
                .map(|n| n.id.clone())
                .collect();

            nodes.retain(|n| {
                !drop_files.contains(n.source_file.as_str())
                    && !new_node_ids.contains(n.id.as_str())
            });
            nodes.extend(d.node_specs.iter().cloned());

            links.retain(|l| {
                let src_file_dirty = l
                    .source_file
                    .as_deref()
                    .is_some_and(|sf| dirty_files.contains(sf));
                !src_file_dirty
                    && !dirty_node_ids.contains(&l.source)
                    && !dirty_node_ids.contains(&l.target)
            });
            links.extend(d.link_specs.iter().cloned());
        }

        // Deduplicate nodes and links deterministically.
        let mut node_map: BTreeMap<String, IrNodeSpec> = BTreeMap::new();
        for n in nodes {
            node_map.insert(n.id.clone(), n);
        }
        let final_nodes: Vec<IrNodeSpec> = node_map.into_values().collect();
        let mut seen_links: BTreeSet<(String, String, Option<String>, Option<String>)> =
            BTreeSet::new();
        let mut final_links = Vec::with_capacity(links.len());
        for l in links {
            let key = (
                l.source.clone(),
                l.target.clone(),
                l.relation.clone(),
                l.context.clone(),
            );
            if seen_links.insert(key) {
                final_links.push(l);
            }
        }
        let node_count = final_nodes.len() as u64;
        let link_count = final_links.len() as u64;
        write_live_graph_json_stream_atomic(live_graph_path, |writer| {
            write_node_link_payload_to_writer(writer, &final_nodes, &final_links)
        })?;
        Ok((node_count, link_count))
    }
}

fn collect_symbol_ref_tokens(ir: &AstFileExtraction) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut push_tok = |raw: &str| {
        for part in raw.split(['\\', '/', '.', ':', ' ', '(', ')', ',', '<', '>']) {
            let trimmed = part.trim().trim_start_matches('$');
            if trimmed.len() >= 2 && trimmed.len() <= 96 {
                out.insert(trimmed.to_ascii_lowercase());
            }
        }
    };
    for ident in &ir.identifiers {
        push_tok(ident);
    }
    for decl in &ir.declarations {
        push_tok(&decl.entity.name);
        if let Some(ref p) = decl.entity.parent {
            push_tok(p);
        }
    }
    for imp in &ir.imports {
        push_tok(&imp.path);
        for s in &imp.symbols {
            push_tok(s);
        }
    }
    for r in &ir.calls_and_refs {
        push_tok(&r.target_name);
        if let Some(ref m) = r.member {
            push_tok(m);
        }
        if let Some(ref o) = r.owner_type {
            push_tok(o);
        }
    }
    for sem in &ir.semantic_edges {
        if let Some((_, rest)) = sem.target.split_once(':') {
            push_tok(rest);
        } else {
            push_tok(&sem.target);
        }
    }
    out
}

fn ensure_column(conn: &Connection, table: &str, col: &str, col_def: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        if row.get::<_, String>(1)? == col {
            return Ok(());
        }
    }
    conn.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {col} {col_def}"),
        [],
    )?;
    Ok(())
}

fn collect_dir_update_candidates(
    dir: &Path,
    dir_rel: &str,
    parse_candidates: &mut BTreeSet<String>,
    manifest_changed: &mut bool,
    manifest_langs: &mut BTreeSet<&'static str>,
) {
    let mut queue = VecDeque::new();
    queue.push_back((
        dir.to_path_buf(),
        dir_rel.trim_end_matches('/').to_owned(),
        0usize,
    ));
    while let Some((cur, prefix, depth)) = queue.pop_front() {
        if parse_candidates.len() >= MAX_IR_FILES {
            break;
        }
        let Ok(entries) = fs::read_dir(&cur) else {
            continue;
        };
        let mut children: Vec<_> = entries.flatten().collect();
        children.sort_by_key(|e| e.file_name());
        for entry in children {
            let path = entry.path();
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            let rel = if prefix.is_empty() {
                name_str.into_owned()
            } else {
                format!("{prefix}/{}", name_str.as_ref())
            };
            if meta.is_dir() {
                let leaf = rel.rsplit('/').next().unwrap_or(&rel);
                if depth < MAX_WALK_DEPTH
                    && !IGNORED_CODE_DIRS.contains(&leaf)
                    && rel != "memory-bank/local"
                {
                    queue.push_back((path, rel, depth + 1));
                }
            } else if meta.is_file() {
                let p = Path::new(&rel);
                if is_graph_manifest_path(p) {
                    *manifest_changed = true;
                    collect_manifest_languages(p, manifest_langs);
                } else if is_supported_code_rel(&rel) && meta.len() <= MAX_FILE_BYTES {
                    parse_candidates.insert(rel);
                    if parse_candidates.len() >= MAX_IR_FILES {
                        break;
                    }
                }
            }
        }
    }
}

pub(crate) fn compute_root_manifest_stamp(real_root: &Path) -> String {
    let mut entries = Vec::new();
    let mut queue = VecDeque::new();
    queue.push_back((real_root.to_path_buf(), String::new(), 0usize));
    while let Some((dir, prefix, depth)) = queue.pop_front() {
        if entries.len() >= 2048 {
            break;
        }
        let Ok(dir_entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in dir_entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let Some(name_str) = name.to_str() else {
                continue;
            };
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            let rel = if prefix.is_empty() {
                name_str.to_owned()
            } else {
                format!("{prefix}/{name_str}")
            };
            if meta.is_dir() {
                if depth < MAX_WALK_DEPTH
                    && !IGNORED_CODE_DIRS.contains(&name_str)
                    && rel != "memory-bank/local"
                {
                    queue.push_back((path, rel, depth + 1));
                }
                continue;
            }
            if !meta.is_file() || !is_graph_manifest_path(Path::new(name_str)) {
                continue;
            }
            entries.push(format!("{rel}:{}:{}", meta.len(), mtime_nanos(&meta)));
        }
    }
    entries.sort();
    entries.join("|")
}

fn collect_manifest_languages(path: &Path, langs: &mut BTreeSet<&'static str>) {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    match name.to_ascii_lowercase().as_str() {
        "cargo.toml" => {
            langs.insert("rust");
        }
        "package.json" | "tsconfig.json" | "jsconfig.json" => {
            langs.insert("typescript");
            langs.insert("tsx");
            langs.insert("javascript");
        }
        "composer.json" => {
            langs.insert("php");
        }
        "pyproject.toml" => {
            langs.insert("python");
        }
        "go.mod" => {
            langs.insert("go");
        }
        _ => {}
    }
}

fn parse_paths_parallel(real_root: &Path, paths: &[String]) -> (Vec<ParsedUnit>, usize) {
    if paths.len() <= 8 {
        let mut units = Vec::with_capacity(paths.len());
        let mut skipped = 0usize;
        for rel in paths {
            if let Some(unit) = parse_workspace_file(real_root, rel) {
                units.push(unit);
            } else {
                skipped += 1;
            }
        }
        return (units, skipped);
    }

    let workers = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .clamp(2, 8)
        .min(paths.len());
    let next_idx = AtomicUsize::new(0);
    let skipped_count = AtomicUsize::new(0);
    let mut chunks: Vec<Vec<ParsedUnit>> = (0..workers).map(|_| Vec::new()).collect();

    std::thread::scope(|scope| {
        for slot in &mut chunks {
            let next_ref = &next_idx;
            let skip_ref = &skipped_count;
            scope.spawn(move || {
                loop {
                    let i = next_ref.fetch_add(1, Ordering::Relaxed);
                    if i >= paths.len() {
                        break;
                    }
                    if let Some(unit) = parse_workspace_file(real_root, &paths[i]) {
                        slot.push(unit);
                    } else {
                        skip_ref.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
        }
    });

    let mut combined: Vec<ParsedUnit> = chunks.into_iter().flatten().collect();
    combined.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    (combined, skipped_count.into_inner())
}

fn candidate_path_tokens(rel_path: &str) -> Vec<String> {
    let lower = rel_path.to_ascii_lowercase();
    let mut tokens = Vec::new();
    let without_ext = lower
        .rsplit_once('.')
        .map_or(lower.as_str(), |(stem, _)| stem);
    if !without_ext.is_empty() {
        tokens.push(without_ext.to_owned());
    }
    if let Some(file_stem) = without_ext.rsplit('/').next()
        && !file_stem.is_empty()
    {
        if !matches!(file_stem, "mod" | "lib" | "index" | "__init__") {
            tokens.push(file_stem.to_owned());
        } else if let Some(parent_dir) = without_ext.rsplit('/').nth(1).filter(|s| !s.is_empty()) {
            tokens.push(parent_dir.to_owned());
        }
    }
    tokens
}

fn specifier_might_match_paths(
    specifier: &str,
    symbols_json: &str,
    affected_tokens: &[String],
) -> bool {
    if affected_tokens.is_empty() {
        return false;
    }
    let spec_lower = specifier.to_ascii_lowercase();
    let syms_lower = symbols_json.to_ascii_lowercase();
    affected_tokens
        .iter()
        .any(|tok| spec_lower.contains(tok.as_str()) || syms_lower.contains(tok.as_str()))
}

pub(crate) fn current_git_head(real_root: &Path) -> Option<String> {
    let out = crate::projects::git(
        real_root,
        &[
            "rev-parse".to_owned(),
            "--verify".to_owned(),
            "--quiet".to_owned(),
            "HEAD".to_owned(),
        ],
        5,
    )
    .ok()?;
    if out.code != 0 {
        return None;
    }
    let head = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!head.is_empty()).then_some(head)
}

fn parse_manifest_stamp_entries(stamp: &str) -> BTreeMap<&str, (&str, &str)> {
    let mut map = BTreeMap::new();
    for entry in stamp.split('|') {
        if entry.is_empty() {
            continue;
        }
        let mut parts = entry.rsplitn(3, ':');
        if let (Some(mtime_s), Some(size_s), Some(rel)) = (parts.next(), parts.next(), parts.next())
        {
            map.insert(rel, (size_s, mtime_s));
        }
    }
    map
}

fn changed_manifest_paths_from_stamps(prev_stamp: Option<&str>, curr_stamp: &str) -> Vec<String> {
    let prev = prev_stamp
        .map(parse_manifest_stamp_entries)
        .unwrap_or_default();
    let curr = parse_manifest_stamp_entries(curr_stamp);
    let mut changed = BTreeSet::new();
    for (rel, vals) in &curr {
        if prev.get(rel) != Some(vals) {
            changed.insert((*rel).to_owned());
        }
    }
    for rel in prev.keys() {
        if !curr.contains_key(rel) {
            changed.insert((*rel).to_owned());
        }
    }
    changed.into_iter().collect()
}

fn detect_semantic_changed<'a>(
    tx: &Transaction<'_>,
    removed_files: impl Iterator<Item = &'a String>,
    parsed_units: &'a [ParsedUnit],
) -> Result<bool> {
    if parsed_units.iter().any(|u| !u.ir.semantic_edges.is_empty()) {
        return Ok(true);
    }
    for path in removed_files.chain(parsed_units.iter().map(|u| &u.rel_path)) {
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM ir_semantic_edges WHERE file_path = ?1)",
                [path],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(false);
        if exists {
            return Ok(true);
        }
    }
    Ok(false)
}

fn split_symbol_query(query: &str) -> (Option<&str>, &str) {
    let trimmed = query.trim();
    if let Some((q, leaf)) = trimmed.rsplit_once("::") {
        return (Some(q.trim()).filter(|s| !s.is_empty()), leaf.trim());
    }
    if let Some((q, leaf)) = trimmed.rsplit_once('.') {
        return (Some(q.trim()).filter(|s| !s.is_empty()), leaf.trim());
    }
    if trimmed.contains('\\') {
        let leaf = trimmed.rsplit('\\').next().unwrap_or(trimmed).trim();
        return (None, leaf);
    }
    (None, trimmed)
}

fn symbol_matches_query(
    raw_query: &str,
    qualifier: Option<&str>,
    leaf: &str,
    symbol_id: &str,
    name: &str,
    parent_name: Option<&str>,
) -> bool {
    if symbol_id == raw_query {
        return true;
    }
    if let Some(q) = qualifier {
        if name != leaf {
            return false;
        }
        let Some(parent) = parent_name.map(str::trim).filter(|p| !p.is_empty()) else {
            return false;
        };
        if parent == q {
            return true;
        }
        let q_short = q.rsplit([':', '\\', '.']).next().unwrap_or(q).trim();
        return parent == q_short
            || parent.ends_with(&format!("::{q}"))
            || parent.ends_with(&format!("\\{q}"));
    }
    name == raw_query || name == leaf
}

fn query_incoming_callers_from_conn(
    conn: &Connection,
    affected_targets: &BTreeSet<String>,
) -> Result<BTreeSet<String>> {
    let mut callers = BTreeSet::new();
    if affected_targets.is_empty() {
        return Ok(callers);
    }
    let targets: Vec<&String> = affected_targets.iter().collect();
    for chunk in targets.chunks(400) {
        let placeholders = std::iter::repeat_n("?", chunk.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT DISTINCT source_file FROM ir_edges WHERE target_file IN ({placeholders}) AND source_file != target_file"
        );
        let mut stmt = conn.prepare(&sql)?;
        let params_vec: Vec<&dyn rusqlite::ToSql> =
            chunk.iter().map(|s| *s as &dyn rusqlite::ToSql).collect();
        let rows = stmt.query_map(params_vec.as_slice(), |row| row.get::<_, String>(0))?;
        for row in rows {
            callers.insert(row?);
        }
    }
    Ok(callers)
}

fn collect_affected_dependents(
    conn: &Connection,
    affected_targets: &BTreeSet<String>,
    skip_files: &BTreeSet<String>,
    manifest_changed: bool,
    manifest_langs: &[&str],
) -> Result<Vec<String>> {
    let mut candidates = BTreeSet::new();
    for src in query_incoming_callers_from_conn(conn, affected_targets)? {
        if !skip_files.contains(&src) {
            candidates.insert(src);
        }
    }

    // Re-export propagation: files referencing an edited module/barrel token (via `ir_symbol_refs`)
    // or importing from a re-exported module must also be considered when re-exports exist on
    // affected targets.
    let reexp_tokens: Vec<String> = {
        let mut set = BTreeSet::new();
        for target in affected_targets {
            let has_reexp: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM ir_reexports WHERE file_path = ?1)",
                    [target],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or(false);
            if has_reexp {
                for tok in candidate_path_tokens(target) {
                    let t = tok.trim().to_ascii_lowercase();
                    if t.len() >= 2 {
                        set.insert(t);
                    }
                }
            }
        }
        set.into_iter().collect()
    };
    for chunk in reexp_tokens.chunks(200) {
        let placeholders = (1..=chunk.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT DISTINCT file_path FROM ir_symbol_refs WHERE token_lower IN ({placeholders})"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
            r.get::<_, String>(0)
        })?;
        for r in rows {
            let fp = r?;
            if !skip_files.contains(&fp) {
                candidates.insert(fp);
            }
        }
    }

    let tokens: Vec<String> = affected_targets
        .iter()
        .flat_map(|p| candidate_path_tokens(p))
        .collect();
    if manifest_changed || !tokens.is_empty() {
        let mut stmt =
            conn.prepare("SELECT source_file, specifier, symbols_json FROM ir_unresolved_imports")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (src, spec, syms_json) = row?;
            if skip_files.contains(&src) {
                continue;
            }
            if manifest_changed || specifier_might_match_paths(&spec, &syms_json, &tokens) {
                candidates.insert(src);
            }
        }
    }

    if manifest_changed {
        let mut stmt = conn.prepare("SELECT path, language FROM ir_files")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (path, lang) = row?;
            if skip_files.contains(&path) {
                continue;
            }
            if manifest_langs.is_empty()
                || manifest_langs
                    .iter()
                    .any(|expected| lang.eq_ignore_ascii_case(expected))
            {
                candidates.insert(path);
            }
        }
    }

    Ok(candidates.into_iter().collect())
}

fn read_declared_members_from_conn(
    conn: &Connection,
    rel_path: &str,
) -> Result<Vec<DeclaredMember>> {
    let mut stmt = conn.prepare_cached(
        "SELECT name, kind, parent_name, start_line, signature, visibility, modifiers_json
         FROM ir_symbols
         WHERE file_path = ?1
         ORDER BY start_line, id",
    )?;
    let rows = stmt.query_map([rel_path], |row| {
        let modifiers_json: String = row.get(6)?;
        let modifiers: Vec<String> = serde_json::from_str(&modifiers_json).unwrap_or_default();
        Ok(DeclaredMember {
            name: row.get(0)?,
            kind: row.get(1)?,
            parent: row.get(2)?,
            line: Some(row.get::<_, i64>(3)?.max(0) as u32),
            signature: row.get(4)?,
            visibility: row.get(5)?,
            previous_visibility: None,
            modifiers,
            change: "unchanged".to_owned(),
            signature_changed: false,
            visibility_narrowed: false,
            used_by_caller: false,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

pub(crate) type FileStampsSnapshot = (BTreeMap<String, (i64, i64)>, Option<String>, Option<String>);
type TargetMembersLoader<'a> = &'a mut dyn FnMut(&str) -> Vec<DeclaredMember>;

fn push_stored_edge_into_dep_map(
    grouped: &mut BTreeMap<(String, String), DepRef>,
    target_file: String,
    target_symbol: Option<String>,
    relation: String,
    context: Option<String>,
    line: u32,
    target_members_loader: Option<TargetMembersLoader<'_>>,
) {
    let key = (target_file.clone(), relation.clone());
    let entry = grouped.entry(key).or_insert_with(|| {
        let target_members = target_members_loader
            .map(|loader| loader(&target_file))
            .unwrap_or_default();
        DepRef {
            target: target_file,
            resolved: true,
            exists: true,
            kind: relation.clone(),
            symbols: Vec::new(),
            line: Some(line),
            evidence: None,
            member_refs: Vec::new(),
            target_members,
        }
    });
    if entry.line.is_none_or(|prev| line < prev) {
        entry.line = Some(line);
    }
    if let Some(sym) = target_symbol {
        let short = sym.rsplit("::").next().unwrap_or(&sym).to_owned();
        if !short.is_empty() && !entry.symbols.contains(&short) {
            entry.symbols.push(short);
        }
        if let Some((parent, member)) = sym.split_once("::") {
            push_ir_member_ref(
                &mut entry.member_refs,
                member,
                context.as_deref().unwrap_or("method"),
                Some(parent),
                &relation,
                line,
            );
        } else if context
            .as_deref()
            .is_some_and(|c| !matches!(c, "import" | "inheritance"))
        {
            push_ir_member_ref(
                &mut entry.member_refs,
                &sym,
                context.as_deref().unwrap_or("symbol"),
                None,
                &relation,
                line,
            );
        }
    }
}

fn push_ir_member_ref(
    out: &mut Vec<DepMemberRef>,
    name: &str,
    kind: &str,
    parent: Option<&str>,
    relation: &str,
    line: u32,
) {
    let access = match relation {
        "writes" => "write",
        "reads" => "read",
        _ => "call",
    };
    if let Some(existing) = out
        .iter_mut()
        .find(|m| m.name == name && m.parent.as_deref() == parent && m.access == access)
    {
        if !existing.lines.contains(&line) {
            existing.lines.push(line);
        }
        return;
    }
    out.push(DepMemberRef {
        name: name.to_owned(),
        kind: kind.to_owned(),
        parent: parent.map(str::to_owned),
        access: access.to_owned(),
        line: Some(line),
        lines: vec![line],
        visibility: None,
        previous_visibility: None,
        change: None,
        modifiers: Vec::new(),
        access_risk: None,
    });
}

fn lock_conn(mutex: &Mutex<Connection>) -> Result<std::sync::MutexGuard<'_, Connection>> {
    mutex
        .lock()
        .map_err(|_| HarnessError::unavailable("The Graph IR database connection is unavailable."))
}

fn validate_project_id(project: &str) -> Result<()> {
    if project.is_empty()
        || project.len() > 128
        || project == "."
        || project == ".."
        || project.contains(['/', '\\', '\0'])
        || project.chars().any(char::is_control)
    {
        return Err(HarnessError::invalid(
            "Invalid project identity for Graph IR.",
        ));
    }
    Ok(())
}

fn verify_existing_sqlite_path(db_path: &Path) -> Result<Option<PathBuf>> {
    let parent = db_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| HarnessError::invalid("Graph IR database path has no parent."))?;
    let name = db_path
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty() && !s.contains(['/', '\\', '\0']))
        .ok_or_else(|| HarnessError::invalid("Invalid Graph IR database filename."))?;
    if !parent.is_dir() {
        return Ok(None);
    }
    let custody = PublicationFiles::new(parent)?;
    let real_dir = infra_core::fs::python_realpath(parent)?;
    let verified = real_dir.join(name);
    infra_core::fs::admit_descendant(&real_dir, &verified)?;
    match fs::symlink_metadata(&verified) {
        Ok(meta) => {
            if meta.file_type().is_symlink() || !meta.is_file() {
                return Err(HarnessError::forbidden(
                    "Graph IR database must be an ordinary regular file.",
                ));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if meta.nlink() != 1 {
                    return Err(HarnessError::forbidden(
                        "Graph IR database must not be hard-linked.",
                    ));
                }
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    }
    custody.check()?;
    Ok(Some(verified))
}

fn ensure_private_sqlite_path(dir: &Path, filename: &str) -> Result<PathBuf> {
    infra_host::util::private_directory(dir)?;
    let custody = PublicationFiles::new(dir)?;
    let real_dir = infra_core::fs::python_realpath(dir)?;
    let db_path = real_dir.join(filename);
    infra_core::fs::admit_descendant(&real_dir, &db_path)?;

    match fs::symlink_metadata(&db_path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() || !meta.is_file() {
                return Err(HarnessError::forbidden(
                    "Graph IR database must be an ordinary regular file.",
                ));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if meta.nlink() != 1 {
                    return Err(HarnessError::forbidden(
                        "Graph IR database must not be hard-linked.",
                    ));
                }
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let mut opts = OpenOptions::new();
            opts.create_new(true).read(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
            }
            drop(opts.open(&db_path)?);
        }
        Err(err) => return Err(err.into()),
    }
    custody.check()?;
    Ok(db_path)
}

fn is_supported_code_rel(rel: &str) -> bool {
    !is_minified_js(rel) && AstGrammar::from_rel_path(rel).is_some()
}

fn mtime_nanos(meta: &fs::Metadata) -> i64 {
    let nanos = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0u128, |d| d.as_nanos());
    i64::try_from(nanos).unwrap_or(i64::MAX)
}

fn walk_workspace_code_files(real_root: &Path) -> Result<(Vec<WalkedFile>, bool)> {
    let mut out = Vec::new();
    let mut truncated = false;
    let mut queue = VecDeque::new();
    queue.push_back((real_root.to_path_buf(), String::new(), 0usize));

    while let Some((dir, prefix, depth)) = queue.pop_front() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut children: Vec<_> = entries.flatten().collect();
        children.sort_by_key(|e| e.file_name());
        for entry in children {
            let path = entry.path();
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            let rel = if prefix.is_empty() {
                name_str.into_owned()
            } else {
                format!("{prefix}/{}", name_str.as_ref())
            };
            if meta.is_dir() {
                let leaf = rel.rsplit('/').next().unwrap_or(&rel);
                if depth < MAX_WALK_DEPTH && !IGNORED_CODE_DIRS.contains(&leaf) {
                    queue.push_back((path, rel, depth + 1));
                }
            } else if meta.is_file() && is_supported_code_rel(&rel) && meta.len() <= MAX_FILE_BYTES
            {
                if out.len() >= MAX_IR_FILES {
                    truncated = true;
                    break;
                }
                out.push(WalkedFile {
                    rel_path: rel,
                    size: i64::try_from(meta.len()).unwrap_or(i64::MAX),
                    mtime_ns: mtime_nanos(&meta),
                });
            }
        }
        if truncated {
            break;
        }
    }
    out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok((out, truncated))
}

fn read_bounded_bytes(
    real_root: &Path,
    rel_path: &str,
    max_bytes: u64,
) -> Option<(Vec<u8>, i64, i64)> {
    let candidate = Path::new(rel_path);
    if candidate.is_absolute()
        || candidate
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return None;
    }
    let full = real_root.join(candidate);
    infra_core::fs::admit_descendant(real_root, &full).ok()?;
    let canon = fs::canonicalize(&full).ok()?;
    if !canon.starts_with(real_root) {
        return None;
    }
    let meta = fs::symlink_metadata(&canon).ok()?;
    if meta.file_type().is_symlink() || !meta.is_file() || meta.len() > max_bytes {
        return None;
    }
    let file = File::open(&canon).ok()?;
    let mut buf = Vec::with_capacity((meta.len().min(max_bytes)) as usize);
    file.take(max_bytes + 1).read_to_end(&mut buf).ok()?;
    if buf.len() as u64 > max_bytes {
        return None;
    }
    let size = i64::try_from(buf.len()).unwrap_or(i64::MAX);
    let mtime_ns = mtime_nanos(&meta);
    Some((buf, size, mtime_ns))
}

fn parse_workspace_file(real_root: &Path, rel_path: &str) -> Option<ParsedUnit> {
    let (bytes, size, mtime_ns) = read_bounded_bytes(real_root, rel_path, MAX_FILE_BYTES)?;
    let sha256 = store::sha(&bytes);
    let source = String::from_utf8(bytes).ok()?;
    let ir = graph_ast::parse_file_ir(rel_path, &source)?;
    Some(ParsedUnit {
        rel_path: rel_path.to_owned(),
        language: ir.language.clone(),
        size,
        mtime_ns,
        sha256,
        ir,
    })
}

fn delete_file_records(tx: &Transaction<'_>, rel_path: &str) -> Result<()> {
    tx.execute("DELETE FROM ir_files WHERE path = ?1", [rel_path])?;
    tx.execute("DELETE FROM ir_symbols WHERE file_path = ?1", [rel_path])?;
    tx.execute("DELETE FROM ir_reexports WHERE file_path = ?1", [rel_path])?;
    tx.execute(
        "DELETE FROM ir_unresolved_imports WHERE source_file = ?1",
        [rel_path],
    )?;
    tx.execute(
        "DELETE FROM ir_semantic_edges WHERE file_path = ?1",
        [rel_path],
    )?;
    tx.execute(
        "DELETE FROM ir_edges WHERE source_file = ?1 OR target_file = ?1",
        [rel_path],
    )?;
    tx.execute(
        "DELETE FROM ir_file_extractions WHERE file_path = ?1",
        [rel_path],
    )?;
    tx.execute(
        "DELETE FROM ir_symbol_refs WHERE file_path = ?1",
        [rel_path],
    )?;
    Ok(())
}

fn upsert_parsed_file(tx: &Transaction<'_>, unit: &ParsedUnit, now: &str) -> Result<()> {
    tx.execute(
        "INSERT OR REPLACE INTO ir_files(path, language, size, mtime_ns, content_sha256, namespace, indexed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            unit.rel_path,
            unit.language,
            unit.size,
            unit.mtime_ns,
            unit.sha256,
            unit.ir.namespace,
            now
        ],
    )?;
    tx.execute(
        "DELETE FROM ir_symbols WHERE file_path = ?1",
        [&unit.rel_path],
    )?;
    tx.execute(
        "DELETE FROM ir_reexports WHERE file_path = ?1",
        [&unit.rel_path],
    )?;
    tx.execute(
        "DELETE FROM ir_semantic_edges WHERE file_path = ?1",
        [&unit.rel_path],
    )?;

    let file_is_test = is_test_path(&unit.rel_path);
    {
        let mut sym_stmt = tx.prepare_cached(
            "INSERT INTO ir_symbols(
                file_path, symbol_id, kind, name, parent_name, signature,
                visibility, start_line, end_line, modifiers_json, body_fingerprint, is_test
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )?;
        for decl in &unit.ir.declarations {
            let parent_name = decl
                .entity
                .parent
                .as_deref()
                .filter(|p| !p.is_empty() && *p != unit.rel_path);
            let signature = (!decl.signature.is_empty()).then_some(decl.signature.as_str());
            let body_fp =
                (!decl.body_fingerprint.is_empty()).then_some(decl.body_fingerprint.as_str());
            let modifiers_json = serde_json::to_string(&decl.modifiers)?;
            let sym_is_test = file_is_test
                || unit
                    .ir
                    .test_spans
                    .iter()
                    .any(|(from, to)| (*from..=*to).contains(&decl.start_line));
            sym_stmt.execute(params![
                unit.rel_path,
                decl.entity.id,
                decl.entity.kind,
                decl.entity.name,
                parent_name,
                signature,
                decl.visibility,
                i64::from(decl.start_line),
                i64::from(decl.end_line),
                modifiers_json,
                body_fp,
                i64::from(sym_is_test)
            ])?;
        }
    }

    {
        let mut reexp_stmt = tx.prepare_cached(
            "INSERT INTO ir_reexports(file_path, alias, orig_path, orig_symbol, is_wildcard, line)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for imp in &unit.ir.imports {
            if !imp.is_reexport {
                continue;
            }
            if imp.is_wildcard {
                reexp_stmt.execute(params![
                    unit.rel_path,
                    "*",
                    imp.path,
                    "*",
                    1i64,
                    i64::from(imp.line)
                ])?;
            } else if !imp.symbols.is_empty() {
                for orig in &imp.symbols {
                    let alias = imp.alias.as_deref().unwrap_or(orig);
                    reexp_stmt.execute(params![
                        unit.rel_path,
                        alias,
                        imp.path,
                        orig,
                        0i64,
                        i64::from(imp.line)
                    ])?;
                }
            } else if let Some(alias) = &imp.alias {
                reexp_stmt.execute(params![
                    unit.rel_path,
                    alias,
                    imp.path,
                    alias,
                    0i64,
                    i64::from(imp.line)
                ])?;
            }
        }
    }

    {
        let mut sem_stmt = tx.prepare_cached(
            "INSERT INTO ir_semantic_edges(file_path, kind, target, source_symbol, line, evidence)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for edge in &unit.ir.semantic_edges {
            let source_symbol =
                resolve_source_symbol_for_line(&unit.ir, edge.source_symbol.as_deref(), edge.line);
            sem_stmt.execute(params![
                unit.rel_path,
                edge.kind,
                edge.target,
                source_symbol,
                i64::from(edge.line),
                edge.evidence
            ])?;
        }
    }

    persist_file_extraction_and_refs(tx, &unit.rel_path, &unit.ir)?;

    Ok(())
}

fn persist_file_extraction_and_refs(
    tx: &Transaction<'_>,
    rel_path: &str,
    ir: &AstFileExtraction,
) -> Result<()> {
    let summary_json = serde_json::to_string(ir)?;
    tx.execute(
        "INSERT OR REPLACE INTO ir_file_extractions(file_path, summary_json) VALUES (?1, ?2)",
        params![rel_path, summary_json],
    )?;
    tx.execute(
        "DELETE FROM ir_symbol_refs WHERE file_path = ?1",
        [rel_path],
    )?;
    let tokens = collect_symbol_ref_tokens(ir);
    if !tokens.is_empty() {
        let mut stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO ir_symbol_refs(token_lower, file_path) VALUES (?1, ?2)",
        )?;
        for tok in tokens {
            stmt.execute(params![tok, rel_path])?;
        }
    }
    Ok(())
}

fn load_persisted_ir_extractions(
    conn: &Connection,
    paths: &[String],
) -> Result<BTreeMap<String, AstFileExtraction>> {
    let mut out = BTreeMap::new();
    if paths.is_empty() {
        return Ok(out);
    }
    for chunk in paths.chunks(200) {
        let placeholders = (1..=chunk.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT file_path, summary_json FROM ir_file_extractions WHERE file_path IN ({placeholders})"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (fp, json_str) = row?;
            if let Ok(ir) = serde_json::from_str::<AstFileExtraction>(&json_str) {
                out.insert(fp, ir);
            }
        }
    }
    Ok(out)
}

fn update_reexport_index_for_units(
    index: &mut ReexportIndex,
    removed: &[String],
    parsed_units: &[ParsedUnit],
) {
    for rel in removed {
        index.by_file.remove(rel);
        index.declared_by_file.remove(rel);
    }
    for unit in parsed_units {
        let rel = &unit.rel_path;
        let decls = index.declared_by_file.entry(rel.clone()).or_default();
        decls.clear();
        for d in &unit.ir.declarations {
            if d.entity.parent.as_deref() == Some(rel.as_str())
                || !matches!(d.entity.kind.as_str(), "method" | "field" | "property")
            {
                decls.insert(d.entity.name.clone());
            }
        }
        let entries = index.by_file.entry(rel.clone()).or_default();
        entries.clear();
        for imp in &unit.ir.imports {
            if !imp.is_reexport {
                continue;
            }
            if imp.is_wildcard {
                entries.push(ReexportEntry {
                    source_module_path: imp.path.clone(),
                    exported_symbol: "*".to_owned(),
                    original_symbol: "*".to_owned(),
                    is_wildcard: true,
                });
            } else {
                for orig in &imp.symbols {
                    let exported = imp.alias.clone().unwrap_or_else(|| orig.clone());
                    entries.push(ReexportEntry {
                        source_module_path: imp.path.clone(),
                        exported_symbol: exported,
                        original_symbol: orig.clone(),
                        is_wildcard: false,
                    });
                }
            }
        }
        if entries.is_empty() {
            index.by_file.remove(rel);
        }
    }
}

fn canonical_sem_target(kind: &str, target: &str) -> String {
    if matches!(kind, "exposes_route" | "route" | "calls_route")
        && let Some(raw) = target.strip_prefix("route:")
    {
        let trimmed = raw.trim();
        let (verb, path) = match trimmed.split_once(' ') {
            Some((v, p)) if p.trim_start().starts_with('/') => (Some(v), p.trim()),
            _ => (None, trimmed),
        };
        if let Some(norm) = normalize_route_contract(verb, path, false) {
            return norm;
        }
    }
    target.to_owned()
}

fn semantic_anchor_priority(kind: &str) -> u8 {
    match kind {
        "writes_table" | "publishes_event" | "exposes_route" | "di_binds" => 0u8,
        _ => 1u8,
    }
}

fn compute_semantic_anchor_map<'a>(
    sem_edges: &'a [StoredSemanticRow],
    file_set: Option<&BTreeSet<&str>>,
) -> BTreeMap<String, (u8, &'a str, u32)> {
    let mut semantic_anchors: BTreeMap<String, (u8, &'a str, u32)> = BTreeMap::new();
    for edge in sem_edges {
        if let Some(allowed) = file_set
            && !allowed.contains(edge.file_path.as_str())
        {
            continue;
        }
        let target_key = canonical_sem_target(&edge.kind, &edge.target);
        let prio = semantic_anchor_priority(&edge.kind);
        let cand = (prio, edge.file_path.as_str(), edge.line);
        match semantic_anchors.get(target_key.as_str()) {
            Some(prev) if *prev <= cand => {}
            _ => {
                semantic_anchors.insert(target_key, cand);
            }
        }
    }
    semantic_anchors
}

#[allow(clippy::too_many_arguments)]
fn emit_semantic_nodes_and_links(
    sem_edges: &[StoredSemanticRow],
    file_set: &BTreeSet<&str>,
    maps: &SymbolResolutionMaps<'_>,
    resolved_targets_by_source: &BTreeMap<&str, Vec<(&str, Option<&str>)>>,
    communities: &BTreeMap<&str, (u32, String)>,
    emit_node_files: Option<&BTreeSet<String>>,
    emit_link_dirty_files: Option<&BTreeSet<String>>,
    nodes: &mut Vec<IrNodeSpec>,
    links: &mut Vec<IrLinkSpec>,
) {
    let semantic_anchors = compute_semantic_anchor_map(sem_edges, Some(file_set));

    let resolve_di_target = |source_file: &str, src_id: &str, svc: &str| -> Option<String> {
        let short = svc.rsplit(['\\', ':', '.']).next().unwrap_or(svc).trim();
        if short.is_empty() {
            return None;
        }
        if let Some(same_file) = maps.type_by_file_and_name.get(&(source_file, short))
            && same_file != src_id
        {
            return Some(same_file.clone());
        }
        if let Some(imports) = resolved_targets_by_source.get(source_file) {
            let mut candidates: Vec<String> = Vec::new();
            for &(target_file, target_sym) in imports {
                let sym_matches = target_sym.is_some_and(|s| {
                    s == short
                        || s.rsplit(['\\', ':', '.'])
                            .next()
                            .is_some_and(|tail| tail == short)
                });
                if (sym_matches || target_sym.is_none())
                    && let Some(id) = maps.type_by_file_and_name.get(&(target_file, short))
                    && id != src_id
                    && !candidates.contains(id)
                {
                    candidates.push(id.clone());
                }
            }
            if candidates.len() == 1 {
                return candidates.into_iter().next();
            }
        }
        if let Some(all_matches) = maps.type_ids_by_name.get(short) {
            let qual_norm = svc.replace(['\\', '.'], "/").replace("::", "/");
            if qual_norm.contains('/') {
                let mut qual_hits: Vec<String> = all_matches
                    .iter()
                    .filter(|(id, f)| {
                        id != src_id
                            && f.to_ascii_lowercase()
                                .contains(&qual_norm.to_ascii_lowercase())
                    })
                    .map(|(id, _)| id.clone())
                    .collect();
                qual_hits.sort();
                qual_hits.dedup();
                if qual_hits.len() == 1 {
                    return qual_hits.into_iter().next();
                }
            }
            if all_matches.len() == 1 && all_matches[0].0 != src_id {
                return Some(all_matches[0].0.clone());
            }
        }
        None
    };

    let mut emitted_semantic_nodes: BTreeMap<String, String> = BTreeMap::new();
    let mut di_groups: BTreeMap<(&str, Option<&str>, u32, &str), Vec<String>> = BTreeMap::new();
    for edge in sem_edges {
        if !file_set.contains(edge.file_path.as_str()) {
            continue;
        }
        let target_key = canonical_sem_target(&edge.kind, &edge.target);
        let src_id = maps.resolve_symbol(
            &edge.file_path,
            edge.source_symbol.as_deref(),
            "calls",
            edge.line,
        );
        let matched_type_id = if edge.kind == "di_binds"
            && let Some(svc) = target_key.strip_prefix("service:")
        {
            resolve_di_target(&edge.file_path, &src_id, svc)
        } else {
            None
        };
        if let Some(ref type_id) = matched_type_id {
            di_groups
                .entry((
                    edge.file_path.as_str(),
                    edge.source_symbol.as_deref(),
                    edge.line,
                    edge.evidence.as_str(),
                ))
                .or_default()
                .push(type_id.clone());
        }

        let dst_id = if let Some(type_id) = matched_type_id {
            type_id
        } else if let Some(existing) = emitted_semantic_nodes.get(target_key.as_str()) {
            existing.clone()
        } else {
            let sem_id = format!("sem:{target_key}");
            let (_, anchor_file, anchor_line) = semantic_anchors
                .get(target_key.as_str())
                .copied()
                .unwrap_or((1, edge.file_path.as_str(), edge.line));
            let should_emit_node =
                emit_node_files.is_none_or(|allowed| allowed.contains(anchor_file));
            if should_emit_node {
                let (comm_id, comm_name) = communities
                    .get(anchor_file)
                    .map_or((None, None), |(id, name)| (Some(*id), Some(name.clone())));
                nodes.push(IrNodeSpec {
                    id: sem_id.clone(),
                    label: target_key.clone(),
                    source_file: anchor_file.to_owned(),
                    source_location: Some(format!("L{anchor_line}")),
                    end_line: 0,
                    file_type: None,
                    node_kind: Some("semantic".to_owned()),
                    community: comm_id,
                    community_name: comm_name,
                    callable_class: false,
                    is_abstract: false,
                    is_test: is_test_path(anchor_file),
                });
            }
            emitted_semantic_nodes.insert(target_key.clone(), sem_id.clone());
            sem_id
        };

        let should_emit_link =
            emit_link_dirty_files.is_none_or(|dirty| dirty.contains(&edge.file_path));
        if should_emit_link && src_id != dst_id {
            links.push(IrLinkSpec {
                source: src_id,
                target: dst_id,
                relation: Some(edge.kind.clone()),
                confidence: Some("EXTRACTED".to_owned()),
                source_file: Some(edge.file_path.clone()),
                context: Some(edge.kind.clone()),
                evidence: Some(edge.evidence.clone()),
            });
        }
    }

    let mut seen_di_pairs: BTreeSet<(String, String)> = BTreeSet::new();
    for ((src_file, _, _, evidence), targets) in di_groups {
        if let Some(dirty) = emit_link_dirty_files
            && !dirty.contains(src_file)
        {
            continue;
        }
        if targets.len() < 2 {
            continue;
        }
        let iface_id = &targets[0];
        for impl_id in &targets[1..] {
            if iface_id != impl_id && seen_di_pairs.insert((iface_id.clone(), impl_id.clone())) {
                links.push(IrLinkSpec {
                    source: iface_id.clone(),
                    target: impl_id.clone(),
                    relation: Some("di_binds".to_owned()),
                    confidence: Some("EXTRACTED".to_owned()),
                    source_file: None,
                    context: Some("di_binds".to_owned()),
                    evidence: Some(evidence.to_owned()),
                });
            }
        }
    }
}

fn obtain_tx_reexport_index(
    cache: &Mutex<Option<(u64, ReexportIndex)>>,
    conn: &Connection,
) -> Result<ReexportIndex> {
    let epoch = read_meta_write_epoch(conn);
    if let Ok(guard) = cache.lock()
        && let Some((cached_epoch, ref idx)) = *guard
        && cached_epoch == epoch
    {
        return Ok(idx.clone());
    }
    let idx = build_reexport_index_from_db(conn)?;
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((epoch, idx.clone()));
    }
    Ok(idx)
}

fn store_reexport_cache(
    cache: &Mutex<Option<(u64, ReexportIndex)>>,
    epoch: u64,
    index: ReexportIndex,
) {
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((epoch, index));
    }
}

fn query_semantic_rows_for_targets(
    conn: &Connection,
    canonical_targets: &[String],
) -> Result<Vec<StoredSemanticRow>> {
    if canonical_targets.is_empty() {
        return Ok(Vec::new());
    }
    let target_set: BTreeSet<&str> = canonical_targets.iter().map(String::as_str).collect();
    let has_route = canonical_targets.iter().any(|t| t.starts_with("route:"));
    let mut out = Vec::new();
    let non_route: Vec<&String> = canonical_targets
        .iter()
        .filter(|t| !t.starts_with("route:"))
        .collect();
    for chunk in non_route.chunks(200) {
        let placeholders = (1..=chunk.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT file_path, kind, target, source_symbol, line, evidence
             FROM ir_semantic_edges
             WHERE target IN ({placeholders})
             ORDER BY id"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter().copied()), |row| {
            Ok(StoredSemanticRow {
                file_path: row.get(0)?,
                kind: row.get(1)?,
                target: row.get(2)?,
                source_symbol: row.get(3)?,
                line: u32::try_from(row.get::<_, i64>(4)?).unwrap_or(1),
                evidence: row.get(5)?,
            })
        })?;
        for r in rows {
            out.push(r?);
        }
    }
    if has_route {
        let mut stmt = conn.prepare(
            "SELECT file_path, kind, target, source_symbol, line, evidence
             FROM ir_semantic_edges
             WHERE kind IN ('exposes_route', 'calls_route', 'route')
             ORDER BY id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(StoredSemanticRow {
                file_path: row.get(0)?,
                kind: row.get(1)?,
                target: row.get(2)?,
                source_symbol: row.get(3)?,
                line: u32::try_from(row.get::<_, i64>(4)?).unwrap_or(1),
                evidence: row.get(5)?,
            })
        })?;
        for r in rows {
            let row = r?;
            let canon = canonical_sem_target(&row.kind, &row.target);
            if target_set.contains(canon.as_str()) {
                out.push(row);
            }
        }
    }
    Ok(out)
}

struct PrevReexportSemanticState {
    prev_reexports: BTreeMap<String, Vec<(String, String, String, bool)>>,
    prev_sem_targets: BTreeSet<String>,
    prev_di_targets: BTreeSet<String>,
    prev_sem_anchors: Vec<String>,
}

fn collect_prev_reexport_and_semantic_state(
    conn: &Connection,
    touched_files: &[String],
) -> Result<PrevReexportSemanticState> {
    let mut prev_reexports: BTreeMap<String, Vec<(String, String, String, bool)>> = BTreeMap::new();
    let mut prev_sem_targets = BTreeSet::new();
    let mut prev_di_targets = BTreeSet::new();
    let mut prev_decl_names = BTreeSet::new();

    for chunk in touched_files.chunks(200) {
        let placeholders = (1..=chunk.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let reexp_sql = format!(
            "SELECT file_path, alias, orig_path, orig_symbol, is_wildcard
             FROM ir_reexports WHERE file_path IN ({placeholders})"
        );
        let mut stmt = conn.prepare(&reexp_sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)? != 0,
            ))
        })?;
        for row in rows {
            let (fp, alias, orig_path, orig_symbol, is_wild) = row?;
            prev_reexports
                .entry(fp)
                .or_default()
                .push((alias, orig_path, orig_symbol, is_wild));
        }

        let sem_sql = format!(
            "SELECT kind, target FROM ir_semantic_edges WHERE file_path IN ({placeholders})"
        );
        let mut sem_stmt = conn.prepare(&sem_sql)?;
        let sem_rows = sem_stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in sem_rows {
            let (kind, target) = row?;
            if kind == "di_binds" {
                prev_di_targets.insert(target);
            } else {
                prev_sem_targets.insert(canonical_sem_target(&kind, &target));
            }
        }

        let sym_sql =
            format!("SELECT DISTINCT name FROM ir_symbols WHERE file_path IN ({placeholders})");
        let mut sym_stmt = conn.prepare(&sym_sql)?;
        let sym_rows = sym_stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
            r.get::<_, String>(0)
        })?;
        for r in sym_rows {
            prev_decl_names.insert(r?);
        }
    }
    for list in prev_reexports.values_mut() {
        list.sort();
    }

    if !prev_decl_names.is_empty() {
        let mut di_stmt =
            conn.prepare("SELECT DISTINCT target FROM ir_semantic_edges WHERE kind = 'di_binds'")?;
        let di_rows = di_stmt.query_map([], |r| r.get::<_, String>(0))?;
        for r in di_rows {
            let target = r?;
            if let Some(svc) = target.strip_prefix("service:") {
                let short = svc.rsplit(['\\', ':', '.']).next().unwrap_or(svc).trim();
                if prev_decl_names.contains(short) {
                    prev_di_targets.insert(target);
                }
            }
        }
    }

    let target_list: Vec<String> = prev_sem_targets.iter().cloned().collect();
    let prev_rows = query_semantic_rows_for_targets(conn, &target_list)?;
    let prev_anchors = compute_semantic_anchor_map(&prev_rows, None);
    let prev_anchor_files: BTreeSet<String> = prev_anchors
        .into_values()
        .map(|(_, fp, _)| fp.to_owned())
        .collect();

    Ok(PrevReexportSemanticState {
        prev_reexports,
        prev_sem_targets,
        prev_di_targets,
        prev_sem_anchors: prev_anchor_files.into_iter().collect(),
    })
}

fn compute_reexport_changed_files(
    prev_reexports: &BTreeMap<String, Vec<(String, String, String, bool)>>,
    removed: &[String],
    parsed_units: &[ParsedUnit],
) -> Vec<String> {
    let mut changed = BTreeSet::new();
    for rem in removed {
        if prev_reexports.contains_key(rem) {
            changed.insert(rem.clone());
        }
    }
    for unit in parsed_units {
        let mut next: Vec<(String, String, String, bool)> = Vec::new();
        for imp in &unit.ir.imports {
            if !imp.is_reexport {
                continue;
            }
            if imp.is_wildcard {
                next.push(("*".to_owned(), imp.path.clone(), "*".to_owned(), true));
            } else {
                for orig in &imp.symbols {
                    let alias = imp.alias.clone().unwrap_or_else(|| orig.clone());
                    next.push((alias, imp.path.clone(), orig.clone(), false));
                }
            }
        }
        next.sort();
        let prev = prev_reexports
            .get(&unit.rel_path)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if prev != next.as_slice() {
            changed.insert(unit.rel_path.clone());
        }
    }
    changed.into_iter().collect()
}

fn collect_post_semantic_delta(
    conn: &Connection,
    prev_sem_targets: &BTreeSet<String>,
    prev_di_targets: &BTreeSet<String>,
    parsed_units: &[ParsedUnit],
) -> Result<(Vec<String>, Vec<String>)> {
    let mut all_sem_targets = prev_sem_targets.clone();
    let mut all_di_targets = prev_di_targets.clone();
    let mut edited_decl_names = BTreeSet::new();
    for unit in parsed_units {
        for d in &unit.ir.declarations {
            edited_decl_names.insert(d.entity.name.clone());
        }
        for edge in &unit.ir.semantic_edges {
            if edge.kind == "di_binds" {
                all_di_targets.insert(edge.target.clone());
            } else {
                all_sem_targets.insert(canonical_sem_target(&edge.kind, &edge.target));
            }
        }
    }
    if !edited_decl_names.is_empty() {
        let mut di_stmt =
            conn.prepare("SELECT DISTINCT target FROM ir_semantic_edges WHERE kind = 'di_binds'")?;
        let di_rows = di_stmt.query_map([], |r| r.get::<_, String>(0))?;
        for r in di_rows {
            let target = r?;
            if let Some(svc) = target.strip_prefix("service:") {
                let short = svc.rsplit(['\\', ':', '.']).next().unwrap_or(svc).trim();
                if edited_decl_names.contains(short) {
                    all_di_targets.insert(target);
                }
            }
        }
    }

    let mut touched_di_files = BTreeSet::new();
    if !all_di_targets.is_empty() {
        let di_vec: Vec<String> = all_di_targets.iter().cloned().collect();
        for chunk in di_vec.chunks(200) {
            let placeholders = (1..=chunk.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT DISTINCT file_path FROM ir_semantic_edges
                 WHERE kind = 'di_binds' AND target IN ({placeholders})"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
                r.get::<_, String>(0)
            })?;
            for r in rows {
                touched_di_files.insert(r?);
            }
        }
        let sym_names: Vec<String> = all_di_targets
            .iter()
            .filter_map(|t| t.strip_prefix("service:"))
            .flat_map(|s| {
                let tail = s.rsplit(['\\', ':', '.']).next().unwrap_or(s).trim();
                (!tail.is_empty()).then(|| tail.to_owned())
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        for chunk in sym_names.chunks(200) {
            let placeholders = (1..=chunk.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(", ");
            let sql =
                format!("SELECT DISTINCT file_path FROM ir_symbols WHERE name IN ({placeholders})");
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
                r.get::<_, String>(0)
            })?;
            for r in rows {
                touched_di_files.insert(r?);
            }
        }
    }

    Ok((
        all_sem_targets.into_iter().collect(),
        touched_di_files.into_iter().collect(),
    ))
}

fn build_reexport_index_from_db(conn: &Connection) -> Result<ReexportIndex> {
    let mut index = ReexportIndex::default();
    {
        let mut stmt = conn.prepare("SELECT file_path, kind, name, parent_name FROM ir_symbols")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;
        for row in rows {
            let (file_path, kind, name, parent_name) = row?;
            if parent_name.is_none() || !matches!(kind.as_str(), "method" | "field" | "property") {
                index
                    .declared_by_file
                    .entry(file_path)
                    .or_default()
                    .insert(name);
            }
        }
    }
    {
        let mut stmt = conn.prepare(
            "SELECT file_path, alias, orig_path, orig_symbol, is_wildcard FROM ir_reexports",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)? != 0,
            ))
        })?;
        for row in rows {
            let (file_path, alias, orig_path, orig_symbol, is_wildcard) = row?;
            index
                .by_file
                .entry(file_path)
                .or_default()
                .push(ReexportEntry {
                    source_module_path: orig_path,
                    exported_symbol: alias,
                    original_symbol: orig_symbol,
                    is_wildcard,
                });
        }
    }
    Ok(index)
}

fn replace_file_edges(
    tx: &Transaction<'_>,
    unit: &ParsedUnit,
    known_files: &BTreeSet<String>,
    manifests: &ManifestIndex,
    reexports: &ReexportIndex,
    crate_map: &CratePackageMap,
) -> Result<()> {
    replace_file_edges_for_ir(
        tx,
        &unit.rel_path,
        &unit.ir,
        known_files,
        manifests,
        reexports,
        crate_map,
    )
}

fn replace_file_edges_for_ir(
    tx: &Transaction<'_>,
    rel_path: &str,
    ir: &AstFileExtraction,
    known_files: &BTreeSet<String>,
    manifests: &ManifestIndex,
    reexports: &ReexportIndex,
    crate_map: &CratePackageMap,
) -> Result<()> {
    tx.execute("DELETE FROM ir_edges WHERE source_file = ?1", [rel_path])?;
    tx.execute(
        "DELETE FROM ir_unresolved_imports WHERE source_file = ?1",
        [rel_path],
    )?;
    let (rows, unresolved) =
        extract_ir_edges_for_file(rel_path, ir, known_files, manifests, reexports, crate_map);
    let mut stmt = tx.prepare_cached(
        "INSERT INTO ir_edges(
            source_file, source_symbol, target_file, target_symbol,
            relation, context, line, confidence
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    for row in rows {
        stmt.execute(params![
            row.source_file,
            row.source_symbol,
            row.target_file,
            row.target_symbol,
            row.relation,
            row.context,
            i64::from(row.line),
            row.confidence
        ])?;
    }
    let mut unres_stmt = tx.prepare_cached(
        "INSERT INTO ir_unresolved_imports(
            source_file, specifier, kind, symbols_json, line
         ) VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for row in unresolved {
        unres_stmt.execute(params![
            row.source_file,
            row.specifier,
            row.kind,
            row.symbols_json,
            i64::from(row.line)
        ])?;
    }
    Ok(())
}

fn extract_ir_edges_for_file(
    rel_path: &str,
    ir: &AstFileExtraction,
    known_files: &BTreeSet<String>,
    manifests: &ManifestIndex,
    reexports: &ReexportIndex,
    crate_map: &CratePackageMap,
) -> (Vec<IrEdgeRow>, Vec<UnresolvedImportRow>) {
    let deps = graph_deps::extract_file_deps_with_intra_from_ir(
        rel_path,
        ir,
        known_files,
        Some(manifests),
        Some(reexports),
        Some(crate_map),
    );
    type EdgeDedupeKey = (
        Option<String>,
        String,
        Option<String>,
        String,
        Option<String>,
        u32,
    );
    let mut rows = Vec::new();
    let mut unresolved = Vec::new();
    let mut seen: BTreeSet<EdgeDedupeKey> = BTreeSet::new();
    let mut seen_unresolved: BTreeSet<(String, String, u32)> = BTreeSet::new();

    let mut push_row = |row: IrEdgeRow| {
        if row.target_file == rel_path
            && (row.source_symbol.is_none()
                || row.target_symbol.is_none()
                || row.source_symbol == row.target_symbol)
        {
            return;
        }
        let key = (
            row.source_symbol.clone(),
            row.target_file.clone(),
            row.target_symbol.clone(),
            row.relation.clone(),
            row.context.clone(),
            row.line,
        );
        if seen.insert(key) {
            rows.push(row);
        }
    };

    for dep in deps {
        let dep_line = dep.line.unwrap_or(1);
        if !dep.resolved || !dep.exists || !known_files.contains(&dep.target) {
            if dep.target != rel_path {
                let unresolved_kind = if dep.resolved && dep.exists {
                    format!("{}:truncated_target", dep.kind)
                } else if !dep.resolved {
                    dep.kind
                } else {
                    format!("{}:missing_target", dep.kind)
                };
                let key = (dep.target.clone(), unresolved_kind.clone(), dep_line);
                if seen_unresolved.insert(key) {
                    unresolved.push(UnresolvedImportRow {
                        source_file: rel_path.to_owned(),
                        specifier: dep.target,
                        kind: unresolved_kind,
                        symbols_json: serde_json::to_string(&dep.symbols)
                            .unwrap_or_else(|_| "[]".to_owned()),
                        line: dep_line,
                    });
                }
            }
            continue;
        }
        let mut emitted_for_dep = false;

        if dep.kind == "imports" {
            let target_symbol = dep.symbols.first().cloned();
            push_row(IrEdgeRow {
                source_file: rel_path.to_owned(),
                source_symbol: None,
                target_file: dep.target.clone(),
                target_symbol,
                relation: "imports".to_owned(),
                context: Some("import".to_owned()),
                line: dep_line,
                confidence: "EXTRACTED".to_owned(),
            });
            emitted_for_dep = true;
        }

        if matches!(dep.kind.as_str(), "extends" | "implements" | "conforms") {
            let matching_inh = ir.inheritance.iter().filter(|inh| {
                inh.kind == dep.kind
                    && dep.symbols.iter().any(|s| {
                        let short = inh
                            .parent
                            .rsplit(['\\', ':', '.'])
                            .next()
                            .unwrap_or(&inh.parent);
                        short == s || inh.parent == *s
                    })
            });
            for inh in matching_inh {
                let src_sym = resolve_source_symbol_for_line(ir, Some(&inh.child), inh.line);
                let dst_sym = inh
                    .parent
                    .rsplit(['\\', ':', '.'])
                    .next()
                    .unwrap_or(&inh.parent)
                    .to_owned();
                push_row(IrEdgeRow {
                    source_file: rel_path.to_owned(),
                    source_symbol: src_sym,
                    target_file: dep.target.clone(),
                    target_symbol: Some(dst_sym),
                    relation: dep.kind.clone(),
                    context: Some("inheritance".to_owned()),
                    line: inh.line,
                    confidence: "EXTRACTED".to_owned(),
                });
                emitted_for_dep = true;
            }
        }

        for member in &dep.member_refs {
            let relation = graph_deps::access_kind(&member.access)
                .unwrap_or(dep.kind.as_str())
                .to_owned();
            let target_symbol = match member.parent.as_deref().filter(|p| !p.is_empty()) {
                Some(parent) => Some(format!("{parent}::{}", member.name)),
                None => Some(member.name.clone()),
            };
            let lines: Vec<u32> = if member.lines.is_empty() {
                vec![member.line.unwrap_or(dep_line)]
            } else {
                member.lines.clone()
            };
            for line in lines {
                let src_sym = resolve_source_symbol_for_line(ir, None, line);
                push_row(IrEdgeRow {
                    source_file: rel_path.to_owned(),
                    source_symbol: src_sym,
                    target_file: dep.target.clone(),
                    target_symbol: target_symbol.clone(),
                    relation: relation.clone(),
                    context: Some(member.kind.clone()),
                    line,
                    confidence: "EXTRACTED".to_owned(),
                });
                emitted_for_dep = true;
            }
        }

        // Preserve type-level and constructor ties (`field`, `parameter_type`, `return_type`, `generic_arg`, `constructor`)
        // so `graph_map::patterns` discovers Strategy, Decorator, Factory, Builder, and Repository patterns.
        for r in matching_ast_type_refs(ir, &dep.symbols) {
            let src_sym = resolve_source_symbol_for_line(ir, r.enclosing_symbol.as_deref(), r.line);
            let dst_sym = r
                .target_name
                .rsplit(['\\', ':', '.'])
                .next()
                .unwrap_or(&r.target_name)
                .to_owned();
            let relation = if r.context == "constructor" && r.access != "type" {
                "calls"
            } else {
                "uses"
            };
            push_row(IrEdgeRow {
                source_file: rel_path.to_owned(),
                source_symbol: src_sym,
                target_file: dep.target.clone(),
                target_symbol: Some(dst_sym),
                relation: relation.to_owned(),
                context: Some(r.context.clone()),
                line: r.line,
                confidence: "EXTRACTED".to_owned(),
            });
            emitted_for_dep = true;
        }

        if !emitted_for_dep {
            let src_sym = resolve_source_symbol_for_line(ir, None, dep_line);
            push_row(IrEdgeRow {
                source_file: rel_path.to_owned(),
                source_symbol: src_sym,
                target_file: dep.target,
                target_symbol: dep.symbols.first().cloned(),
                relation: dep.kind,
                context: None,
                line: dep_line,
                confidence: "EXTRACTED".to_owned(),
            });
        }
    }

    (rows, unresolved)
}

fn matching_ast_type_refs<'a>(
    ir: &'a AstFileExtraction,
    dep_symbols: &[String],
) -> Vec<&'a AstRef> {
    if dep_symbols.is_empty() {
        return Vec::new();
    }
    ir.calls_and_refs
        .iter()
        .filter(|r| {
            matches!(
                r.context.as_str(),
                "field" | "parameter_type" | "return_type" | "generic_arg" | "constructor"
            ) && {
                let short = r
                    .target_name
                    .rsplit(['\\', ':', '.'])
                    .next()
                    .unwrap_or(&r.target_name);
                dep_symbols
                    .iter()
                    .any(|s| s == short || s == &r.target_name)
            }
        })
        .collect()
}

fn decl_matches_hint(decl: &crate::graph_live::ExtractedDecl, hint: Option<&str>) -> bool {
    let Some(hint) = hint else {
        return false;
    };
    decl.entity.id == hint
        || decl.entity.name == hint
        || decl
            .entity
            .parent
            .as_deref()
            .is_some_and(|p| format!("{p}::{}", decl.entity.name) == hint)
}

fn resolve_source_symbol_for_line(
    ir: &AstFileExtraction,
    hint: Option<&str>,
    line: u32,
) -> Option<String> {
    let norm_hint = hint.map(str::trim).filter(|s| !s.is_empty());
    let is_callable = |decl: &crate::graph_live::ExtractedDecl| {
        matches!(
            decl.entity.kind.as_str(),
            "method" | "function" | "constructor"
        )
    };
    let mut best: Option<&crate::graph_live::ExtractedDecl> = None;
    for decl in &ir.declarations {
        if line >= decl.start_line && line <= decl.end_line {
            let span = decl.end_line.saturating_sub(decl.start_line);
            let replace = match best {
                None => true,
                Some(prev) => {
                    let prev_span = prev.end_line.saturating_sub(prev.start_line);
                    if span != prev_span {
                        span < prev_span
                    } else {
                        let decl_call = is_callable(decl);
                        let prev_call = is_callable(prev);
                        if decl_call != prev_call {
                            decl_call
                        } else {
                            let decl_hint = decl_matches_hint(decl, norm_hint);
                            let prev_hint = decl_matches_hint(prev, norm_hint);
                            if decl_hint != prev_hint {
                                decl_hint
                            } else {
                                decl.start_line > prev.start_line
                            }
                        }
                    }
                }
            };
            if replace {
                best = Some(decl);
            }
        }
    }
    if let Some(found) = best {
        return Some(found.entity.id.clone());
    }
    norm_hint
        .and_then(|h| {
            ir.declarations.iter().find(|d| {
                decl_matches_hint(d, Some(h))
                    && (line == 0 || (line >= d.start_line && d.end_line == 0))
            })
        })
        .map(|d| d.entity.id.clone())
}

fn recompute_summary(tx: &Transaction<'_>, now: &str, truncated: bool) -> Result<IrSummary> {
    let files: i64 = tx.query_row("SELECT COUNT(*) FROM ir_files", [], |r| r.get(0))?;
    let symbols: i64 = tx.query_row("SELECT COUNT(*) FROM ir_symbols", [], |r| r.get(0))?;
    let edges: i64 = tx.query_row("SELECT COUNT(*) FROM ir_edges", [], |r| r.get(0))?;
    let semantic_edges: i64 =
        tx.query_row("SELECT COUNT(*) FROM ir_semantic_edges", [], |r| r.get(0))?;
    tx.execute(
        "INSERT OR REPLACE INTO ir_summary(
            id, files, symbols, edges, semantic_edges, updated_at, truncated, max_files
         ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            files,
            symbols,
            edges,
            semantic_edges,
            now,
            i64::from(truncated),
            MAX_IR_FILES as i64
        ],
    )?;
    Ok(IrSummary {
        files: files.max(0) as u64,
        symbols: symbols.max(0) as u64,
        edges: edges.max(0) as u64,
        semantic_edges: semantic_edges.max(0) as u64,
        updated_at: now.to_owned(),
        truncated,
        max_files: MAX_IR_FILES,
    })
}

fn upsert_meta(tx: &Transaction<'_>, key: &str, value: &str) -> Result<()> {
    tx.execute(
        "INSERT OR REPLACE INTO ir_meta(key, value) VALUES (?1, ?2)",
        params![key, value],
    )?;
    Ok(())
}

fn read_meta_write_epoch(conn: &Connection) -> u64 {
    conn.query_row(
        "SELECT value FROM ir_meta WHERE key = 'write_epoch'",
        [],
        |r| r.get::<_, String>(0),
    )
    .ok()
    .and_then(|s| s.parse::<u64>().ok())
    .unwrap_or(0)
}

fn bump_meta_write_epoch(tx: &Transaction<'_>) -> Result<u64> {
    let next = read_meta_write_epoch(tx).saturating_add(1);
    upsert_meta(tx, "write_epoch", &next.to_string())?;
    Ok(next)
}

fn open_readonly_pooled_conn(db_path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(SQLITE_BUSY_TIMEOUT)?;
    let _ = conn.execute_batch("PRAGMA query_only=ON;");
    Ok(conn)
}

fn manifests_fresh_against_meta(root: &Path, conn: &Connection) -> bool {
    let stored: Option<String> = conn
        .query_row(
            "SELECT value FROM ir_meta WHERE key = 'manifest_stamp'",
            [],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten();
    let Some(stamp) = stored else {
        return true;
    };
    let mut recorded = BTreeSet::new();
    for entry in stamp.split('|') {
        if entry.is_empty() {
            continue;
        }
        let mut parts = entry.rsplitn(3, ':');
        let (Some(mtime_s), Some(size_s), Some(rel)) = (parts.next(), parts.next(), parts.next())
        else {
            return false;
        };
        let (Ok(expected_size), Ok(expected_mtime)) =
            (size_s.parse::<i64>(), mtime_s.parse::<i64>())
        else {
            return false;
        };
        let Ok(meta) = fs::symlink_metadata(root.join(rel)) else {
            return false;
        };
        if meta.file_type().is_symlink() || !meta.is_file() {
            return false;
        }
        let actual_size = i64::try_from(meta.len()).unwrap_or(i64::MAX);
        if actual_size != expected_size || mtime_nanos(&meta) != expected_mtime {
            return false;
        }
        recorded.insert(rel.to_owned());
    }
    for root_manifest in [
        "Cargo.toml",
        "package.json",
        "tsconfig.json",
        "jsconfig.json",
        "composer.json",
        "pyproject.toml",
        "go.mod",
    ] {
        if !recorded.contains(root_manifest)
            && fs::symlink_metadata(root.join(root_manifest))
                .is_ok_and(|meta| !meta.file_type().is_symlink() && meta.is_file())
        {
            return false;
        }
    }
    true
}

fn is_ir_file_fresh_cached(
    conn: &Connection,
    root: &Path,
    rel: &str,
    cache: &mut HashMap<String, bool>,
) -> bool {
    if let Some(&fresh) = cache.get(rel) {
        return fresh;
    }
    let stored: Option<(i64, i64)> = conn
        .query_row(
            "SELECT size, mtime_ns FROM ir_files WHERE path = ?1",
            [rel],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .ok()
        .flatten();
    let fresh = match stored {
        None => true,
        Some((db_size, db_mtime)) => fs::symlink_metadata(root.join(rel)).is_ok_and(|meta| {
            !meta.file_type().is_symlink()
                && meta.is_file()
                && i64::try_from(meta.len()).unwrap_or(i64::MAX) == db_size
                && mtime_nanos(&meta) == db_mtime
        }),
    };
    cache.insert(rel.to_owned(), fresh);
    fresh
}

fn read_summary_from_conn(conn: &Connection) -> Result<Option<IrSummary>> {
    match conn.query_row(
        "SELECT files, symbols, edges, semantic_edges, updated_at, truncated, max_files
         FROM ir_summary WHERE id = 1",
        [],
        |row| {
            Ok(IrSummary {
                files: row.get::<_, i64>(0)?.max(0) as u64,
                symbols: row.get::<_, i64>(1)?.max(0) as u64,
                edges: row.get::<_, i64>(2)?.max(0) as u64,
                semantic_edges: row.get::<_, i64>(3)?.max(0) as u64,
                updated_at: row.get(4)?,
                truncated: row.get::<_, i64>(5)? != 0,
                max_files: usize::try_from(row.get::<_, i64>(6)?.max(1)).unwrap_or(MAX_IR_FILES),
            })
        },
    ) {
        Ok(summary) => Ok(Some(summary)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(err) if err.to_string().contains("no such column") => conn
            .query_row(
                "SELECT files, symbols, edges, semantic_edges, updated_at FROM ir_summary WHERE id = 1",
                [],
                |row| {
                    Ok(IrSummary {
                        files: row.get::<_, i64>(0)?.max(0) as u64,
                        symbols: row.get::<_, i64>(1)?.max(0) as u64,
                        edges: row.get::<_, i64>(2)?.max(0) as u64,
                        semantic_edges: row.get::<_, i64>(3)?.max(0) as u64,
                        updated_at: row.get(4)?,
                        truncated: false,
                        max_files: MAX_IR_FILES,
                    })
                },
            )
            .optional()
            .map_err(Into::into),
        Err(err) => Err(err.into()),
    }
}

fn is_type_kind(kind: &str) -> bool {
    matches!(
        kind,
        "class"
            | "interface"
            | "trait"
            | "struct"
            | "enum"
            | "protocol"
            | "object"
            | "record"
            | "type"
            | "module"
    )
}

fn raw_dir_labels(files: &[String]) -> (Vec<String>, bool) {
    const GENERIC_ROOTS: &[&str] = &[
        "crates", "packages", "services", "apps", "modules", "src", "app", "lib", "pkg",
        "internal", "include",
    ];
    if files.is_empty() {
        return (Vec::new(), false);
    }

    let dir_segments: Vec<Vec<&str>> = files
        .iter()
        .map(|path| match path.rsplit_once('/') {
            Some((dir, _)) => dir.split('/').filter(|s| !s.is_empty()).collect(),
            None => Vec::new(),
        })
        .collect();

    // Strip directory prefix shared across every file when deeper subdirs exist.
    let mut common_len = 0usize;
    if files.len() > 1 {
        let min_dirs = dir_segments.iter().map(Vec::len).min().unwrap_or(0);
        while common_len < min_dirs {
            let candidate = dir_segments[0][common_len];
            if dir_segments
                .iter()
                .all(|segs| segs[common_len] == candidate)
            {
                common_len += 1;
            } else {
                break;
            }
        }
    }

    let mut raw_labels: Vec<String> = Vec::with_capacity(files.len());
    for (idx, path) in files.iter().enumerate() {
        let remaining = dir_segments[idx]
            .get(common_len..)
            .unwrap_or(&[])
            .iter()
            .copied()
            .skip_while(|seg| GENERIC_ROOTS.contains(&seg.to_ascii_lowercase().as_str()))
            .collect::<Vec<_>>();
        let label = if let Some(first_dir) = remaining.first() {
            (*first_dir).to_owned()
        } else {
            let file_name = path.rsplit('/').next().unwrap_or(path);
            let stem = file_name
                .rsplit_once('.')
                .map_or(file_name, |(s, _)| s)
                .trim();
            if stem.is_empty() {
                "root".to_owned()
            } else {
                stem.to_owned()
            }
        };
        raw_labels.push(label);
    }

    let distinct: BTreeSet<&str> = raw_labels.iter().map(String::as_str).collect();
    let needs_edge_components = distinct.len() <= 1 && files.len() >= 2;
    (raw_labels, needs_edge_components)
}

fn finish_file_communities<'a>(
    files: &'a [String],
    mut raw_labels: Vec<String>,
    split_components: bool,
    edge_pairs: &[(&str, &str)],
) -> BTreeMap<&'a str, (u32, String)> {
    if files.is_empty() {
        return BTreeMap::new();
    }

    // Group connected flat files inside the same generic directory if directory structure alone
    // did not distinguish separate components.
    if split_components {
        let mut idx_by_file: BTreeMap<&str, usize> = BTreeMap::new();
        for (i, f) in files.iter().enumerate() {
            idx_by_file.insert(f.as_str(), i);
        }
        let mut parent: Vec<usize> = (0..files.len()).collect();
        fn find_root(parent: &mut [usize], mut x: usize) -> usize {
            while parent[x] != x {
                parent[x] = parent[parent[x]];
                x = parent[x];
            }
            x
        }
        for &(src, dst) in edge_pairs {
            if let (Some(&u), Some(&v)) = (idx_by_file.get(src), idx_by_file.get(dst)) {
                let ru = find_root(&mut parent, u);
                let rv = find_root(&mut parent, v);
                if ru != rv {
                    parent[rv] = ru.min(rv);
                    parent[ru] = ru.min(rv);
                }
            }
        }
        let mut comp_min_stem: BTreeMap<usize, String> = BTreeMap::new();
        for (i, path) in files.iter().enumerate() {
            let root = find_root(&mut parent, i);
            let leaf = path.rsplit('/').next().unwrap_or(path);
            let stem = leaf.rsplit_once('.').map_or(leaf, |(s, _)| s);
            comp_min_stem
                .entry(root)
                .and_modify(|prev| {
                    if stem < prev.as_str() {
                        *prev = stem.to_owned();
                    }
                })
                .or_insert_with(|| stem.to_owned());
        }
        if comp_min_stem.len() > 1 {
            for (i, label) in raw_labels.iter_mut().enumerate().take(files.len()) {
                let root = find_root(&mut parent, i);
                if let Some(stem) = comp_min_stem.get(&root) {
                    *label = stem.clone();
                }
            }
        }
    }

    let mut ordered_names: BTreeSet<(String, String)> = BTreeSet::new();
    for label in &raw_labels {
        ordered_names.insert((label.to_ascii_lowercase(), label.clone()));
    }
    let mut id_by_name: BTreeMap<String, u32> = BTreeMap::new();
    for (idx, (_, name)) in ordered_names.into_iter().enumerate() {
        id_by_name.insert(name, idx as u32);
    }

    let mut out = BTreeMap::new();
    for (path, label) in files.iter().zip(raw_labels) {
        let id = id_by_name.get(&label).copied().unwrap_or(0);
        out.insert(path.as_str(), (id, label));
    }
    out
}

fn compute_file_communities<'a>(
    files: &'a [String],
    edges: &[StoredEdgeRow],
) -> BTreeMap<&'a str, (u32, String)> {
    let (raw_labels, needs_edge_components) = raw_dir_labels(files);
    let edge_pairs: Vec<(&str, &str)> = if needs_edge_components {
        edges
            .iter()
            .map(|e| (e.source_file.as_str(), e.target_file.as_str()))
            .collect()
    } else {
        Vec::new()
    };
    finish_file_communities(files, raw_labels, needs_edge_components, &edge_pairs)
}

fn compute_file_communities_for_delta<'a>(
    conn: &Connection,
    files: &'a [String],
) -> Result<BTreeMap<&'a str, (u32, String)>> {
    let (raw_labels, needs_edge_components) = raw_dir_labels(files);
    if !needs_edge_components {
        return Ok(finish_file_communities(files, raw_labels, false, &[]));
    }
    let owned_pairs: Vec<(String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT source_file, target_file FROM ir_edges WHERE source_file != target_file",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };
    let borrowed_pairs: Vec<(&str, &str)> = owned_pairs
        .iter()
        .map(|(s, t)| (s.as_str(), t.as_str()))
        .collect();
    Ok(finish_file_communities(
        files,
        raw_labels,
        true,
        &borrowed_pairs,
    ))
}

type SymbolCandidate<'a> = (&'a str, u32, u32, String);

struct SymbolResolutionMaps<'a> {
    by_symbol_id: BTreeMap<(&'a str, &'a str), String>,
    by_member: BTreeMap<(&'a str, &'a str, &'a str), Vec<SymbolCandidate<'a>>>,
    by_name: BTreeMap<(&'a str, &'a str), Vec<SymbolCandidate<'a>>>,
    type_by_file_and_name: BTreeMap<(&'a str, &'a str), String>,
    type_ids_by_name: BTreeMap<String, Vec<(String, &'a str)>>,
}

fn pick_parent_node_id(
    sym_kind: &str,
    sym_line: u32,
    candidates: &[SymbolCandidate<'_>],
) -> Option<String> {
    let kind_rank = |k: &str| match sym_kind {
        "calls" | "method" => u8::from(!matches!(k, "function" | "method" | "constructor")),
        "type" => u8::from(!is_type_kind(k)),
        _ => 0,
    };
    candidates
        .iter()
        .min_by_key(|(k, start, end, id)| {
            let enclosing_rank = if sym_line > 0 && *end >= *start && *end > 0 {
                u8::from(!(sym_line >= *start && sym_line <= *end))
            } else {
                1
            };
            (
                kind_rank(k),
                enclosing_rank,
                if sym_line > 0 {
                    start.abs_diff(sym_line)
                } else {
                    *start
                },
                *start,
                id.as_str(),
            )
        })
        .map(|(_, _, _, id)| id.clone())
}

impl<'a> SymbolResolutionMaps<'a> {
    fn resolve_symbol(
        &self,
        file_path: &str,
        sym_hint: Option<&str>,
        pref_kind: &str,
        pref_line: u32,
    ) -> String {
        if let Some(hint) = sym_hint.map(str::trim).filter(|s| !s.is_empty()) {
            if let Some(id) = self.by_symbol_id.get(&(file_path, hint)) {
                return id.clone();
            }
            if let Some((parent, member)) = hint.split_once("::") {
                let clean_member = member.trim_start_matches('$');
                if let Some(cands) = self
                    .by_member
                    .get(&(file_path, parent, member))
                    .or_else(|| self.by_member.get(&(file_path, parent, clean_member)))
                    && let Some(id) = pick_parent_node_id(pref_kind, pref_line, cands)
                {
                    return id;
                }
            }
            let clean_hint = hint.trim_start_matches('$');
            if let Some(cands) = self
                .by_name
                .get(&(file_path, hint))
                .or_else(|| self.by_name.get(&(file_path, clean_hint)))
                && let Some(id) = pick_parent_node_id(pref_kind, pref_line, cands)
            {
                return id;
            }
        }
        format!("file:{file_path}")
    }
}

fn collect_symbol_nodes_and_maps<'a>(
    emit_file_nodes_for: &[String],
    emit_struct_filter: Option<&BTreeSet<String>>,
    symbols: &'a [StoredSymbolRow],
    communities: &BTreeMap<&str, (u32, String)>,
    used_node_ids: &mut BTreeSet<String>,
) -> (Vec<IrNodeSpec>, Vec<IrLinkSpec>, SymbolResolutionMaps<'a>) {
    let mut nodes = Vec::with_capacity(emit_file_nodes_for.len() + symbols.len());
    let mut links = Vec::with_capacity(symbols.len());

    let mut by_symbol_id: BTreeMap<(&'a str, &'a str), String> = BTreeMap::new();
    let mut by_member: BTreeMap<(&'a str, &'a str, &'a str), Vec<SymbolCandidate<'a>>> =
        BTreeMap::new();
    let mut by_name: BTreeMap<(&'a str, &'a str), Vec<SymbolCandidate<'a>>> = BTreeMap::new();
    let mut type_by_file_and_name: BTreeMap<(&'a str, &'a str), String> = BTreeMap::new();
    let mut type_ids_by_name: BTreeMap<String, Vec<(String, &'a str)>> = BTreeMap::new();

    for path in emit_file_nodes_for {
        let fid = format!("file:{path}");
        used_node_ids.insert(fid.clone());
        let label = path.rsplit('/').next().unwrap_or(path).to_owned();
        let (comm_id, comm_name) = communities
            .get(path.as_str())
            .map_or((None, None), |(id, name)| (Some(*id), Some(name.clone())));
        nodes.push(IrNodeSpec {
            id: fid,
            label,
            source_file: path.clone(),
            source_location: None,
            end_line: 0,
            file_type: Some("code".to_owned()),
            node_kind: Some("page".to_owned()),
            community: comm_id,
            community_name: comm_name,
            callable_class: false,
            is_abstract: false,
            is_test: is_test_path(path),
        });
    }

    let mut assigned_ids: Vec<String> = Vec::with_capacity(symbols.len());
    for sym in symbols {
        let node_id = if used_node_ids.insert(sym.symbol_id.clone()) {
            sym.symbol_id.clone()
        } else {
            let disambiguated = format!("{}#{}", sym.symbol_id, sym.row_id);
            used_node_ids.insert(disambiguated.clone());
            disambiguated
        };
        assigned_ids.push(node_id.clone());

        by_symbol_id
            .entry((&sym.file_path, &sym.symbol_id))
            .or_insert_with(|| node_id.clone());
        by_name
            .entry((&sym.file_path, &sym.name))
            .or_default()
            .push((
                sym.kind.as_str(),
                sym.start_line,
                sym.end_line,
                node_id.clone(),
            ));
        if let Some(parent) = sym.parent_name.as_deref() {
            by_member
                .entry((&sym.file_path, parent, &sym.name))
                .or_default()
                .push((
                    sym.kind.as_str(),
                    sym.start_line,
                    sym.end_line,
                    node_id.clone(),
                ));
        }
        if is_type_kind(&sym.kind) {
            type_by_file_and_name
                .entry((&sym.file_path, &sym.name))
                .or_insert_with(|| node_id.clone());
            type_ids_by_name
                .entry(sym.name.clone())
                .or_default()
                .push((node_id.clone(), sym.file_path.as_str()));
        }

        if emit_struct_filter.is_some_and(|filter| !filter.contains(&sym.file_path)) {
            continue;
        }
        let is_method = matches!(sym.kind.as_str(), "method" | "constructor");
        let has_parent = sym.parent_name.is_some();
        let label = if is_method && has_parent {
            format!(".{}()", sym.name)
        } else if sym.kind == "function" || is_method {
            format!("{}()", sym.name)
        } else {
            sym.name.clone()
        };
        let (comm_id, comm_name) = communities
            .get(sym.file_path.as_str())
            .map_or((None, None), |(id, name)| (Some(*id), Some(name.clone())));

        nodes.push(IrNodeSpec {
            id: node_id,
            label,
            source_file: sym.file_path.clone(),
            source_location: Some(format!("L{}", sym.start_line)),
            end_line: sym.end_line,
            file_type: None,
            node_kind: Some(sym.kind.clone()),
            community: comm_id,
            community_name: comm_name,
            callable_class: is_type_kind(&sym.kind),
            is_abstract: sym.is_abstract,
            is_test: sym.is_test || is_test_path(&sym.file_path),
        });
    }

    for (sym, node_id) in symbols.iter().zip(assigned_ids.iter()) {
        if emit_struct_filter.is_some_and(|filter| !filter.contains(&sym.file_path)) {
            continue;
        }
        let Some(parent_name) = sym.parent_name.as_deref() else {
            continue;
        };
        let parent_id = if let Some(id) = type_by_file_and_name.get(&(&sym.file_path, parent_name))
        {
            Some(id.clone())
        } else {
            by_name
                .get(&(&sym.file_path, parent_name))
                .and_then(|cands| pick_parent_node_id("type", sym.start_line, cands))
        };
        if let Some(parent_id) = parent_id
            && &parent_id != node_id
        {
            let rel = if matches!(sym.kind.as_str(), "method" | "constructor") {
                "method"
            } else {
                "contains"
            };
            links.push(IrLinkSpec {
                source: parent_id,
                target: node_id.clone(),
                relation: Some(rel.to_owned()),
                confidence: None,
                source_file: Some(sym.file_path.clone()),
                context: None,
                evidence: None,
            });
        }
    }

    (
        nodes,
        links,
        SymbolResolutionMaps {
            by_symbol_id,
            by_member,
            by_name,
            type_by_file_and_name,
            type_ids_by_name,
        },
    )
}

type ResolvedEdgeTargets<'a> = BTreeMap<&'a str, Vec<(&'a str, Option<&'a str>)>>;

fn collect_resolved_edge_links<'a>(
    edges: &'a [StoredEdgeRow],
    file_set: &BTreeSet<&str>,
    maps: &SymbolResolutionMaps<'_>,
) -> (Vec<IrLinkSpec>, ResolvedEdgeTargets<'a>) {
    let mut links = Vec::with_capacity(edges.len());
    let mut resolved_targets_by_source: ResolvedEdgeTargets<'a> = BTreeMap::new();
    for edge in edges {
        if !file_set.contains(edge.source_file.as_str())
            || !file_set.contains(edge.target_file.as_str())
        {
            continue;
        }
        resolved_targets_by_source
            .entry(edge.source_file.as_str())
            .or_default()
            .push((edge.target_file.as_str(), edge.target_symbol.as_deref()));

        let dst_pref = if matches!(
            edge.relation.as_str(),
            "extends" | "implements" | "conforms"
        ) || edge.context.as_deref().is_some_and(|c| {
            matches!(
                c,
                "field" | "parameter_type" | "return_type" | "generic_arg" | "constructor"
            )
        }) {
            "type"
        } else if edge.relation == "calls" {
            "calls"
        } else {
            ""
        };
        let src_id = maps.resolve_symbol(
            &edge.source_file,
            edge.source_symbol.as_deref(),
            "calls",
            edge.line,
        );
        let dst_line = if edge.source_file == edge.target_file {
            edge.line
        } else {
            0
        };
        let dst_id = maps.resolve_symbol(
            &edge.target_file,
            edge.target_symbol.as_deref(),
            dst_pref,
            dst_line,
        );
        if src_id == dst_id {
            continue;
        }
        let emit_as_reference = edge.relation == "uses"
            && edge.context.as_deref().is_some_and(|c| {
                matches!(
                    c,
                    "field" | "parameter_type" | "return_type" | "generic_arg" | "constructor"
                )
            });
        let relation = if emit_as_reference {
            "references"
        } else {
            edge.relation.as_str()
        };
        links.push(IrLinkSpec {
            source: src_id,
            target: dst_id,
            relation: Some(relation.to_owned()),
            confidence: Some(edge.confidence.clone()),
            source_file: Some(edge.source_file.clone()),
            context: edge.context.clone(),
            evidence: None,
        });
    }
    (links, resolved_targets_by_source)
}

fn collect_graph_elements(
    files: &[String],
    symbols: &[StoredSymbolRow],
    edges: &[StoredEdgeRow],
    sem_edges: &[StoredSemanticRow],
) -> (Vec<IrNodeSpec>, Vec<IrLinkSpec>, u64, u64) {
    let file_set: BTreeSet<&str> = files.iter().map(String::as_str).collect();
    let communities = compute_file_communities(files, edges);
    let mut used_node_ids: BTreeSet<String> = BTreeSet::new();
    let (mut nodes, mut links, maps) =
        collect_symbol_nodes_and_maps(files, None, symbols, &communities, &mut used_node_ids);
    let (edge_links, resolved_targets_by_source) =
        collect_resolved_edge_links(edges, &file_set, &maps);
    links.extend(edge_links);

    emit_semantic_nodes_and_links(
        sem_edges,
        &file_set,
        &maps,
        &resolved_targets_by_source,
        &communities,
        None,
        None,
        &mut nodes,
        &mut links,
    );

    let node_count = nodes.len() as u64;
    let link_count = links.len() as u64;
    (nodes, links, node_count, link_count)
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

fn is_false_flag(v: &bool) -> bool {
    !*v
}

#[derive(Serialize)]
struct IrNodeWire<'a> {
    id: &'a str,
    label: &'a str,
    source_file: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_location: Option<&'a str>,
    #[serde(skip_serializing_if = "is_zero_u32")]
    end_line: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    file_type: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    node_kind: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    community: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    community_name: Option<&'a str>,
    #[serde(rename = "_callable_class", skip_serializing_if = "is_false_flag")]
    callable_class: bool,
    #[serde(rename = "abstract", skip_serializing_if = "is_false_flag")]
    is_abstract: bool,
    #[serde(skip_serializing_if = "is_false_flag")]
    is_test: bool,
}

#[derive(Serialize)]
struct IrLinkWire<'a> {
    source: &'a str,
    target: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    relation: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    confidence: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_file: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    evidence: Option<&'a str>,
}

struct IrNodesSliceWire<'a>(&'a [IrNodeSpec]);

impl Serialize for IrNodesSliceWire<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.iter().map(|spec| IrNodeWire {
            id: &spec.id,
            label: &spec.label,
            source_file: &spec.source_file,
            source_location: spec.source_location.as_deref(),
            end_line: spec.end_line,
            file_type: spec.file_type.as_deref(),
            node_kind: spec.node_kind.as_deref(),
            community: spec.community,
            community_name: spec.community_name.as_deref(),
            callable_class: spec.callable_class,
            is_abstract: spec.is_abstract,
            is_test: spec.is_test,
        }))
    }
}

struct IrLinksSliceWire<'a>(&'a [IrLinkSpec]);

impl Serialize for IrLinksSliceWire<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.iter().map(|spec| IrLinkWire {
            source: &spec.source,
            target: &spec.target,
            relation: spec.relation.as_deref(),
            context: spec.context.as_deref(),
            confidence: spec.confidence.as_deref(),
            source_file: spec.source_file.as_deref(),
            evidence: spec.evidence.as_deref(),
        }))
    }
}

#[derive(Serialize)]
struct IrGraphPayloadWire<'a> {
    directed: bool,
    multigraph: bool,
    backend: &'a str,
    nodes: IrNodesSliceWire<'a>,
    links: IrLinksSliceWire<'a>,
}

pub(crate) fn write_node_link_payload_to_writer<W: Write>(
    writer: W,
    node_specs: &[IrNodeSpec],
    link_specs: &[IrLinkSpec],
) -> Result<()> {
    let payload = IrGraphPayloadWire {
        directed: true,
        multigraph: false,
        backend: NATIVE_IR_TOOL,
        nodes: IrNodesSliceWire(node_specs),
        links: IrLinksSliceWire(link_specs),
    };
    serde_json::to_writer(writer, &payload)?;
    Ok(())
}

fn node_link_payload_from_elements(node_specs: &[IrNodeSpec], link_specs: &[IrLinkSpec]) -> Value {
    let nodes: Vec<Value> = node_specs
        .iter()
        .map(|spec| {
            let mut node = json!({
                "id": spec.id,
                "label": spec.label,
                "source_file": spec.source_file,
            });
            if let Some(loc) = &spec.source_location {
                node["source_location"] = json!(loc);
            }
            if spec.end_line > 0 {
                node["end_line"] = json!(spec.end_line);
            }
            if let Some(ft) = &spec.file_type {
                node["file_type"] = json!(ft);
            }
            if let Some(nk) = &spec.node_kind {
                node["node_kind"] = json!(nk);
            }
            if let Some(comm) = spec.community {
                node["community"] = json!(comm);
            }
            if let Some(comm_name) = &spec.community_name {
                node["community_name"] = json!(comm_name);
            }
            if spec.callable_class {
                node["_callable_class"] = json!(true);
            }
            if spec.is_abstract {
                node["abstract"] = json!(true);
            }
            if spec.is_test {
                node["is_test"] = json!(true);
            }
            node
        })
        .collect();

    let links: Vec<Value> = link_specs
        .iter()
        .map(|spec| {
            let mut link = json!({
                "source": spec.source,
                "target": spec.target,
            });
            if let Some(rel) = &spec.relation {
                link["relation"] = json!(rel);
            }
            if let Some(ctx) = &spec.context {
                link["context"] = json!(ctx);
            }
            if let Some(ev) = &spec.confidence {
                link["confidence"] = json!(ev);
            }
            if let Some(src_file) = &spec.source_file {
                link["source_file"] = json!(src_file);
            }
            if let Some(evidence) = &spec.evidence {
                link["evidence"] = json!(evidence);
            }
            link
        })
        .collect();

    json!({
        "directed": true,
        "multigraph": false,
        "backend": NATIVE_IR_TOOL,
        "nodes": nodes,
        "links": links,
    })
}

fn build_node_link_payload(
    files: &[String],
    symbols: &[StoredSymbolRow],
    edges: &[StoredEdgeRow],
    sem_edges: &[StoredSemanticRow],
) -> (Value, u64, u64) {
    let (node_specs, link_specs, node_count, link_count) =
        collect_graph_elements(files, symbols, edges, sem_edges);
    let payload = node_link_payload_from_elements(&node_specs, &link_specs);
    (payload, node_count, link_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_map::Index;

    #[test]
    fn sync_workspace_indexes_multi_language_project_and_skips_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let root = dir.path().join("project");
        fs::create_dir_all(root.join("src/Services")).unwrap();
        fs::create_dir_all(root.join("src/Contracts")).unwrap();
        fs::create_dir_all(root.join("node_modules/ignored")).unwrap();

        fs::write(
            root.join("composer.json"),
            r#"{"autoload":{"psr-4":{"App\\":"src/"}}}"#,
        )
        .unwrap();
        fs::write(
            root.join("src/Contracts/Notifier.php"),
            "<?php\nnamespace App\\Contracts;\ninterface Notifier {\n    public function send(string $msg): void;\n}\n",
        )
        .unwrap();
        fs::write(
            root.join("src/Services/OrderService.php"),
            "<?php\nnamespace App\\Services;\nuse App\\Contracts\\Notifier;\nclass OrderService {\n    private Notifier $notifier;\n    public function create(Notifier $notifier): void {\n        $notifier->send('ok');\n    }\n}\n",
        )
        .unwrap();
        fs::write(
            root.join("node_modules/ignored/bad.js"),
            "export function ignoreMe() {}\n",
        )
        .unwrap();

        let store = GraphIrStore::open(&state, "proj1").unwrap();
        let first = store.sync_workspace(&root).unwrap();
        assert_eq!(first.files_scanned, 2, "{first:?}");
        assert_eq!(first.files_parsed, 2, "{first:?}");
        assert_eq!(first.files_skipped, 0, "{first:?}");
        assert!(first.summary.symbols >= 4, "{first:?}");
        assert!(first.summary.edges >= 1, "{first:?}");

        let summary = store.summary_counts().unwrap().expect("summary row");
        assert_eq!(summary.files, 2);
        assert_eq!(summary, first.summary);

        // Second sync with untouched files skips every file cleanly.
        let second = store.sync_workspace(&root).unwrap();
        assert_eq!(second.files_scanned, 2);
        assert_eq!(second.files_parsed, 0);
        assert_eq!(second.files_skipped, 2);
        assert_eq!(second.files_removed, 0);

        // Exported node-link JSON parses cleanly into `graph_map::Index`.
        let node_link = store.export_node_link_json().unwrap();
        assert_eq!(node_link["backend"], NATIVE_IR_TOOL);
        let live_graph = state.join("graphs/proj1/live/graph.json");
        let (nc, lc) = store.write_live_graph_atomic(&live_graph).unwrap();
        assert!(nc >= 4 && lc >= 1);
        assert!(live_graph.is_file());

        let index = Index::read(File::open(&live_graph).unwrap(), &root).unwrap();
        assert_eq!(index.backend(), Some(NATIVE_IR_TOOL));
        assert!(
            index
                .files()
                .contains(&"src/Services/OrderService.php".to_owned())
        );
        assert!(
            index
                .files()
                .contains(&"src/Contracts/Notifier.php".to_owned())
        );
        assert!(!index.files().iter().any(|f| f.contains("node_modules")));
    }

    #[test]
    fn update_paths_handles_rust_reexports_semantic_edges_and_deletions() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let root = dir.path().join("repo");
        fs::create_dir_all(root.join("src")).unwrap();

        fs::write(
            root.join("src/inner.rs"),
            "pub struct TokenService;\nimpl TokenService {\n    pub fn issue(&self) {}\n}\n",
        )
        .unwrap();
        fs::write(
            root.join("src/facade.rs"),
            "pub use crate::inner::TokenService;\n",
        )
        .unwrap();
        fs::write(
            root.join("src/api.rs"),
            "use crate::facade::TokenService;\npub fn login(svc: &TokenService) {\n    svc.issue();\n}\n",
        )
        .unwrap();

        let store = GraphIrStore::open(&state, "rustproj").unwrap();
        let initial = store.sync_workspace(&root).unwrap();
        assert_eq!(initial.summary.files, 3);

        // Verify re-export propagation placed an edge directly from `src/api.rs` -> `src/inner.rs`.
        {
            let conn = lock_conn(&store.conn).unwrap();
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM ir_edges WHERE source_file = 'src/api.rs' AND target_file = 'src/inner.rs'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(
                count >= 1,
                "expected resolved re-export edge to src/inner.rs"
            );
        }

        // Edit `src/api.rs` to add a semantic route + SQL table access, and delete `src/facade.rs`.
        fs::write(
            root.join("src/api.rs"),
            "use crate::inner::TokenService;\n\
             pub fn login(svc: &TokenService) {\n\
                 let _q = \"SELECT id FROM users\";\n\
                 svc.issue();\n\
             }\n",
        )
        .unwrap();
        fs::remove_file(root.join("src/facade.rs")).unwrap();

        let updated = store
            .update_paths(
                &root,
                &[
                    "src/api.rs".to_owned(),
                    "src/facade.rs".to_owned(),
                    "../outside.rs".to_owned(),
                ],
            )
            .unwrap();
        assert_eq!(updated.files_parsed, 1);
        assert_eq!(updated.files_removed, 1);
        assert_eq!(updated.summary.files, 2);
        assert!(updated.summary.semantic_edges >= 1, "{updated:?}");

        let node_link = store.export_node_link_json().unwrap();
        let index = Index::read(node_link.to_string().as_bytes(), &root).unwrap();
        assert!(!index.files().contains(&"src/facade.rs".to_owned()));
        assert!(
            index.files().contains(&"src/api.rs".to_owned())
                && index.files().contains(&"src/inner.rs".to_owned())
        );
    }

    #[cfg(unix)]
    #[test]
    fn open_rejects_symlinked_database_and_atomic_write_preserves_prior_live_graph() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let graph_dir = state.join("graphs").join("sym");
        fs::create_dir_all(&graph_dir).unwrap();
        let outside = dir.path().join("outside.sqlite3");
        fs::write(&outside, b"").unwrap();
        std::os::unix::fs::symlink(&outside, graph_dir.join(DB_FILENAME)).unwrap();

        assert!(GraphIrStore::open(&state, "sym").is_err());

        // Atomic write preserves prior `live/graph.json` when destination parent is valid.
        let store = GraphIrStore::open(&state, "real").unwrap();
        let root = dir.path().join("proj");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("app.py"), "def greet():\n    return 'hi'\n").unwrap();
        store.sync_workspace(&root).unwrap();

        let live_graph = state.join("graphs/real/live/graph.json");
        fs::create_dir_all(live_graph.parent().unwrap()).unwrap();
        fs::write(&live_graph, br#"{"nodes":[],"links":[]}"#).unwrap();

        let (nodes, _) = store.write_live_graph_atomic(&live_graph).unwrap();
        assert!(nodes >= 2);
        let parsed: Value = serde_json::from_slice(&fs::read(&live_graph).unwrap()).unwrap();
        assert_eq!(parsed["backend"], NATIVE_IR_TOOL);
    }

    #[test]
    fn emits_deterministic_communities_and_reresolves_unresolved_and_di_ambiguity() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let root = dir.path().join("proj");
        fs::create_dir_all(root.join("src/Billing")).unwrap();
        fs::create_dir_all(root.join("src/Orders")).unwrap();

        fs::write(
            root.join("composer.json"),
            r#"{"autoload":{"psr-4":{"App\\":"src/"}}}"#,
        )
        .unwrap();
        fs::write(
            root.join("src/Billing/Repository.php"),
            "<?php\nnamespace App\\Billing;\nclass Repository {}\n",
        )
        .unwrap();
        fs::write(
            root.join("src/Orders/Repository.php"),
            "<?php\nnamespace App\\Orders;\nclass Repository {}\n",
        )
        .unwrap();
        fs::write(
            root.join("src/Orders/OrderProvider.php"),
            "<?php\nnamespace App\\Orders;\nuse App\\Orders\\Repository;\nuse App\\Billing\\InvoiceService;\nclass OrderProvider {\n    public function register(): void {\n        $this->app->bind(Repository::class, SqlRepository::class);\n    }\n}\n",
        )
        .unwrap();

        let store = GraphIrStore::open(&state, "comm").unwrap();
        let stats1 = store.sync_workspace(&root).unwrap();
        assert!(!stats1.summary.truncated);
        assert_eq!(stats1.summary.max_files, MAX_IR_FILES);

        // Before `InvoiceService.php` exists, `OrderProvider.php` has an unresolved import.
        let payload1 = store.export_node_link_json().unwrap();
        let nodes1 = payload1["nodes"].as_array().unwrap();
        assert!(
            nodes1.iter().all(
                |n| n["community"].as_u64().is_some() && n["community_name"].as_str().is_some()
            ),
            "every node should carry community metadata: {nodes1:?}"
        );
        // `di_binds` disambiguates `Repository` to `App\Orders\Repository` via file import,
        // rather than falling back to `sem:service:Repository`.
        let links1 = payload1["links"].as_array().unwrap();
        assert!(
            links1.iter().any(|l| {
                l["relation"] == "di_binds"
                    && l["target"]
                        .as_str()
                        .is_some_and(|t| t.contains("Repository") && !t.starts_with("sem:"))
            }),
            "expected disambiguated DI target: {links1:?}"
        );

        // Now create `src/Billing/InvoiceService.php` and incrementally update ONLY that path!
        fs::write(
            root.join("src/Billing/InvoiceService.php"),
            "<?php\nnamespace App\\Billing;\nclass InvoiceService {}\n",
        )
        .unwrap();
        store
            .update_paths(&root, &["src/Billing/InvoiceService.php".to_owned()])
            .unwrap();

        let payload2 = store.export_node_link_json().unwrap();
        let links2 = payload2["links"].as_array().unwrap();
        assert!(
            links2.iter().any(|l| {
                l["source_file"] == "src/Orders/OrderProvider.php"
                    && l["target"]
                        .as_str()
                        .is_some_and(|t| t.contains("InvoiceService"))
            }),
            "unresolved importer should resolve after target creation: {links2:?}"
        );
    }

    #[test]
    fn open_readonly_does_not_create_or_mutate_and_batch_reads_fresh_deps() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let root = dir.path().join("proj");
        fs::create_dir_all(root.join("src")).unwrap();

        // Missing DB returns None without creating any sqlite file.
        assert!(
            GraphIrStore::open_readonly(&state, "missing")
                .unwrap()
                .is_none()
        );
        assert!(!state.join("graphs/missing").join(DB_FILENAME).exists());

        // Empty dummy file returns None without mutating it.
        let empty_db = state.join("graphs/empty").join(DB_FILENAME);
        fs::create_dir_all(empty_db.parent().unwrap()).unwrap();
        fs::write(&empty_db, b"").unwrap();
        assert!(
            GraphIrStore::open_readonly(&state, "empty")
                .unwrap()
                .is_none()
        );
        assert_eq!(fs::metadata(&empty_db).unwrap().len(), 0);

        fs::write(
            root.join("composer.json"),
            r#"{"autoload":{"psr-4":{"App\\":"src/"}}}"#,
        )
        .unwrap();
        fs::write(
            root.join("src/B.php"),
            "<?php\nnamespace App;\nclass B {\n    public static int $count = 0;\n}\n",
        )
        .unwrap();
        fs::write(
            root.join("src/A.php"),
            "<?php\nnamespace App;\nuse App\\B;\nclass A {\n    public function go(): int { B::$count = 1; return B::$count; }\n}\n",
        )
        .unwrap();
        fs::write(root.join("src/C.php"), "<?php\nclass C {}\n").unwrap();

        let rw = GraphIrStore::open(&state, "ro").unwrap();
        rw.sync_workspace(&root).unwrap();
        let updated_before: String = rw
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT value FROM ir_meta WHERE key = 'updated_at'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        drop(rw);

        // Open existing DB strictly read-only.
        let ro = GraphIrStore::open_readonly(&state, "ro")
            .unwrap()
            .expect("existing populated IR store");
        assert!(ro.read_only);
        assert!(ro.summary_counts().unwrap().is_some());
        assert!(ro.sync_workspace(&root).is_err());
        assert!(ro.update_paths(&root, &["src/A.php".to_owned()]).is_err());
        let updated_after: String = ro
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT value FROM ir_meta WHERE key = 'updated_at'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(updated_before, updated_after);

        // Make src/C.php stale on disk and verify batch lookup filters it out.
        fs::write(
            root.join("src/C.php"),
            "<?php\nclass C { public int $x = 1; }\n",
        )
        .unwrap();
        let batch = ro
            .batch_access_deps_if_fresh(
                &root,
                &["src/A.php", "src/B.php", "src/C.php", "src/missing.php"],
            )
            .unwrap();
        assert!(
            batch
                .get("src/A.php")
                .is_some_and(|deps| deps.iter().any(|d| d.target == "src/B.php")),
            "expected batched A->B dep: {batch:?}"
        );
        assert!(batch.contains_key("src/B.php"));
        assert!(!batch.contains_key("src/C.php"));
        assert!(!batch.contains_key("src/missing.php"));
    }

    #[test]
    fn update_paths_prunes_removed_directories_and_nested_manifest_stamp_and_preserves_truncated_links()
     {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let root = dir.path().join("mono");
        fs::create_dir_all(root.join("apps/web")).unwrap();
        fs::create_dir_all(root.join("src/legacy")).unwrap();
        fs::write(
            root.join("apps/web/package.json"),
            r#"{"name":"@mono/web","version":"1.0.0"}"#,
        )
        .unwrap();
        fs::write(root.join("src/legacy/one.rs"), "pub fn one() {}\n").unwrap();
        fs::write(root.join("src/legacy/two.rs"), "pub fn two() {}\n").unwrap();
        fs::write(root.join("src/keep.rs"), "pub fn keep() {}\n").unwrap();

        let stamp_before = compute_root_manifest_stamp(&root);
        fs::write(
            root.join("apps/web/package.json"),
            r#"{"name":"@mono/web","version":"2.0.0"}"#,
        )
        .unwrap();
        let stamp_after = compute_root_manifest_stamp(&root);
        assert_ne!(
            stamp_before, stamp_after,
            "nested monorepo manifest edit must alter stamp"
        );

        let ir = GraphIrStore::open(&state, "dirprune").unwrap();
        ir.sync_workspace(&root).unwrap();
        assert!(ir.known_files().unwrap().contains("src/legacy/one.rs"));

        fs::remove_dir_all(root.join("src/legacy")).unwrap();
        let update = ir.update_paths(&root, &["src/legacy".to_owned()]).unwrap();
        assert_eq!(update.files_removed, 2);
        let known = ir.known_files().unwrap();
        assert!(!known.contains("src/legacy/one.rs"));
        assert!(!known.contains("src/legacy/two.rs"));
        assert!(known.contains("src/keep.rs"));

        // Preserving unresolved targets when missing or truncated instead of silently dropping them.
        fs::write(
            root.join("src/keep.rs"),
            "use missing_mod::Thing;\npub fn keep() {}\n",
        )
        .unwrap();
        ir.update_paths(&root, &["src/keep.rs".to_owned()]).unwrap();
        let (deps, _) = ir
            .file_outgoing_deps_if_fresh(&root, "src/keep.rs")
            .unwrap()
            .expect("fresh outgoing deps");
        assert!(
            deps.iter()
                .any(|d| !d.resolved && d.target.contains("missing_mod"))
        );
    }

    #[test]
    fn write_epoch_read_pool_abstract_modifiers_manifest_freshness_and_route_hubs() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let root = dir.path().join("ws");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("package.json"),
            r#"{"name":"demo","version":"1.0.0"}"#,
        )
        .unwrap();
        fs::write(
            root.join("src/Contracts.php"),
            "<?php\nabstract class BaseHandler {}\ninterface PaymentGateway {}\ntrait AuditTrait {}\n",
        )
        .unwrap();
        fs::write(
            root.join("src/client.ts"),
            "export async function fetchOrder(id: string) { return fetch(`/api/orders/${id}`); }\n",
        )
        .unwrap();

        let ir = GraphIrStore::open(&state, "p1").unwrap();
        assert_eq!(ir.current_write_epoch(), 0);
        ir.sync_workspace(&root).unwrap();
        let epoch1 = ir.current_write_epoch();
        assert!(epoch1 >= 1);

        // Open read-only pool handle and verify exported abstract flags + canonical route contract hubs.
        let ro = GraphIrStore::open_readonly(&state, "p1")
            .unwrap()
            .expect("readonly ir");
        assert_eq!(ro.current_write_epoch(), epoch1);
        let (ir_nodes, ir_links, _, _) = ro.export_ir_graph_elements().unwrap();
        assert!(
            ir_nodes
                .iter()
                .any(|n| n.label.contains("BaseHandler") && n.is_abstract),
            "expected BaseHandler marked abstract: {ir_nodes:?}"
        );
        assert!(
            ir_nodes
                .iter()
                .any(|n| n.label.contains("PaymentGateway") && n.is_abstract),
            "expected PaymentGateway marked abstract: {ir_nodes:?}"
        );
        assert!(
            ir_nodes
                .iter()
                .any(|n| n.id == "sem:route:GET /api/orders/{param}"),
            "expected normalized semantic route hub: {ir_nodes:?}"
        );
        assert!(
            ir_links
                .iter()
                .any(|l| l.target == "sem:route:GET /api/orders/{param}"
                    && l.relation.as_deref() == Some("calls_route")),
            "expected calls_route link onto canonical hub: {ir_links:?}"
        );

        // Concurrent `update_paths` increments `write_epoch`.
        fs::write(root.join("src/extra.ts"), "export const ping = () => 1;\n").unwrap();
        ir.update_paths(&root, &["src/extra.ts".to_owned()])
            .unwrap();
        let epoch2 = ir.current_write_epoch();
        assert!(epoch2 > epoch1);

        // External manifest edit invalidates `file_outgoing_deps_if_fresh`.
        assert!(
            ir.file_outgoing_deps_if_fresh(&root, "src/client.ts")
                .unwrap()
                .is_some()
        );
        fs::write(
            root.join("package.json"),
            r#"{"name":"demo","version":"2.0.0"}"#,
        )
        .unwrap();
        assert!(
            ir.file_outgoing_deps_if_fresh(&root, "src/client.ts")
                .unwrap()
                .is_none(),
            "manifest edit must invalidate cached outgoing deps"
        );
    }

    #[test]
    fn di_binds_links_interface_to_impl_and_callsite_line_disambiguates_same_file_symbols() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let state = dir.path().join("state");
        fs::create_dir_all(root.join("src/Contracts")).unwrap();
        fs::create_dir_all(root.join("src/Infra")).unwrap();
        fs::create_dir_all(root.join("src/Providers")).unwrap();

        fs::write(
            root.join("src/Contracts/PaymentGateway.php"),
            "<?php\nnamespace App\\Contracts;\ninterface PaymentGateway { public function charge(): void; }\n",
        )
        .unwrap();
        fs::write(
            root.join("src/Infra/StripeGateway.php"),
            "<?php\nnamespace App\\Infra;\nclass StripeGateway { public function charge(): void {} }\n",
        )
        .unwrap();
        fs::write(
            root.join("src/Providers/BillingProvider.php"),
            "<?php\nnamespace App\\Providers;\nuse App\\Contracts\\PaymentGateway;\nuse App\\Infra\\StripeGateway;\nclass BillingProvider {\n    public function register(): void {\n        $this->app->bind(PaymentGateway::class, StripeGateway::class);\n    }\n}\n",
        )
        .unwrap();

        let store = GraphIrStore::open(&state, "p-di").unwrap();
        store.sync_workspace(&root).unwrap();
        let (_, links, _, _) = store.export_ir_graph_elements().unwrap();
        let (nodes, _, _, _) = store.export_ir_graph_elements().unwrap();

        let iface_id = nodes
            .iter()
            .find(|n| {
                n.source_file == "src/Contracts/PaymentGateway.php" && n.label == "PaymentGateway"
            })
            .expect("interface node")
            .id
            .clone();
        let impl_id = nodes
            .iter()
            .find(|n| n.source_file == "src/Infra/StripeGateway.php" && n.label == "StripeGateway")
            .expect("impl node")
            .id
            .clone();
        assert!(
            links.iter().any(|l| {
                l.source == iface_id
                    && l.target == impl_id
                    && l.relation.as_deref() == Some("di_binds")
            }),
            "expected direct Interface -> Impl di_binds link: {links:?}"
        );

        let files = vec!["src/same.ts".to_owned()];
        let symbols = vec![
            StoredSymbolRow {
                row_id: 1,
                symbol_id: "sym:first".to_owned(),
                file_path: "src/same.ts".to_owned(),
                kind: "function".to_owned(),
                name: "dup".to_owned(),
                parent_name: None,
                start_line: 10,
                end_line: 20,
                is_abstract: false,
                is_test: false,
            },
            StoredSymbolRow {
                row_id: 2,
                symbol_id: "sym:second".to_owned(),
                file_path: "src/same.ts".to_owned(),
                kind: "function".to_owned(),
                name: "dup".to_owned(),
                parent_name: None,
                start_line: 40,
                end_line: 60,
                is_abstract: false,
                is_test: false,
            },
            StoredSymbolRow {
                row_id: 3,
                symbol_id: "sym:caller".to_owned(),
                file_path: "src/same.ts".to_owned(),
                kind: "function".to_owned(),
                name: "run".to_owned(),
                parent_name: None,
                start_line: 45,
                end_line: 55,
                is_abstract: false,
                is_test: false,
            },
        ];
        let edges = vec![StoredEdgeRow {
            source_file: "src/same.ts".to_owned(),
            target_file: "src/same.ts".to_owned(),
            source_symbol: Some("run".to_owned()),
            target_symbol: Some("dup".to_owned()),
            relation: "calls".to_owned(),
            confidence: "EXTRACTED".to_owned(),
            context: None,
            line: 48,
        }];
        let (_, same_links, _, _) = collect_graph_elements(&files, &symbols, &edges, &[]);
        assert!(
            same_links
                .iter()
                .any(|l| l.source == "sym:caller" && l.target == "sym:second"),
            "same-file edge should resolve to line-closest/enclosing target: {same_links:?}"
        );
    }

    #[test]
    fn incremental_delta_preserves_communities_and_streaming_json_matches_dom() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join("src/auth")).unwrap();
        std::fs::create_dir_all(root.join("src/billing")).unwrap();
        std::fs::write(
            root.join("src/auth/login.rs"),
            "pub fn login() -> i32 { 1 }\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/billing/pay.rs"),
            "pub fn pay() -> i32 { crate::auth::login() }\n",
        )
        .unwrap();

        let store = GraphIrStore::open(&state, "comm-test").unwrap();
        store.sync_workspace(&root).unwrap();

        std::fs::write(
            root.join("src/auth/login.rs"),
            "pub fn login() -> i32 { 2 }\npub fn logout() -> i32 { 3 }\n",
        )
        .unwrap();
        let delta = store
            .update_paths_with_delta(&root, &["src/auth/login.rs".to_owned()])
            .unwrap();
        let ir_delta = store
            .export_ir_delta_for_files(&delta)
            .unwrap()
            .expect("delta elements");

        let auth_nodes: Vec<&IrNodeSpec> = ir_delta
            .node_specs
            .iter()
            .filter(|n| n.source_file == "src/auth/login.rs")
            .collect();
        assert!(!auth_nodes.is_empty());
        for node in &auth_nodes {
            assert_eq!(
                node.community_name.as_deref(),
                Some("auth"),
                "expected preserved directory community on {node:?}"
            );
            assert!(node.community.is_some());
        }

        // Streaming JSON encoding matches DOM value output identically.
        let (all_nodes, all_links, _, _) = store.export_ir_graph_elements().unwrap();
        let streamed_bytes =
            encode_node_link_payload_from_elements(&all_nodes, &all_links).unwrap();
        let streamed_val: Value = serde_json::from_slice(&streamed_bytes).unwrap();
        let dom_val = node_link_payload_from_elements(&all_nodes, &all_links);
        assert_eq!(streamed_val, dom_val);

        // Read-only cached store returns the same Arc until DB file is replaced.
        let ro1 = GraphIrStore::open_readonly_cached(&state, "comm-test")
            .unwrap()
            .expect("cached ro1");
        let ro2 = GraphIrStore::open_readonly_cached(&state, "comm-test")
            .unwrap()
            .expect("cached ro2");
        assert!(Arc::ptr_eq(&ro1, &ro2));
    }

    #[test]
    fn verify_path_or_symbol_and_delta_incoming_callers_work_over_readonly_cache() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub mod auth;\npub mod api;\n").unwrap();
        std::fs::write(
            root.join("src/auth.rs"),
            "pub struct TokenService;\nimpl TokenService {\n    pub fn issue() -> i32 { 1 }\n}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/api.rs"),
            "use crate::auth::TokenService;\npub fn handler() -> i32 { TokenService::issue() }\n",
        )
        .unwrap();

        let store = GraphIrStore::open(&state, "verify-test").unwrap();
        store.sync_workspace(&root).unwrap();

        let ro = GraphIrStore::open_readonly_cached(&state, "verify-test")
            .unwrap()
            .expect("readonly cached IR");

        let exact = ro
            .verify_path_or_symbol(Some("src/auth.rs"), Some("TokenService::issue"))
            .unwrap();
        assert!(exact.path_exists);
        assert!(exact.symbol_exists);
        assert_eq!(exact.matched_files, vec!["src/auth.rs".to_owned()]);
        assert!(!exact.matched_symbols.is_empty());

        let wrong_owner = ro
            .verify_path_or_symbol(Some("src/auth.rs"), Some("OtherService::issue"))
            .unwrap();
        assert!(wrong_owner.path_exists);
        assert!(!wrong_owner.symbol_exists);

        let sym_only = ro.verify_path_or_symbol(None, Some("handler")).unwrap();
        assert!(!sym_only.path_exists);
        assert!(sym_only.symbol_exists);
        assert_eq!(sym_only.matched_files, vec!["src/api.rs".to_owned()]);

        let missing = ro
            .verify_path_or_symbol(Some("src/missing.rs"), Some("Ghost"))
            .unwrap();
        assert!(!missing.path_exists);
        assert!(!missing.symbol_exists);

        std::fs::write(
            root.join("src/auth.rs"),
            "pub struct TokenService;\nimpl TokenService {\n    pub fn issue() -> i32 { 2 }\n}\n",
        )
        .unwrap();
        let edit_delta = store
            .update_paths_with_delta(&root, &["src/auth.rs".to_owned()])
            .unwrap();
        assert_eq!(edit_delta.changed_paths, vec!["src/auth.rs".to_owned()]);
        assert!(
            edit_delta
                .incoming_caller_files
                .contains(&"src/api.rs".to_owned()),
            "expected src/api.rs in incoming_caller_files: {:?}",
            edit_delta.incoming_caller_files
        );

        std::fs::remove_file(root.join("src/auth.rs")).unwrap();
        let remove_delta = store
            .update_paths_with_delta(&root, &["src/auth.rs".to_owned()])
            .unwrap();
        assert_eq!(remove_delta.removed_files, vec!["src/auth.rs".to_owned()]);
        assert!(
            remove_delta
                .incoming_caller_files
                .contains(&"src/api.rs".to_owned()),
            "expected pre-removal caller src/api.rs in incoming_caller_files: {:?}",
            remove_delta.incoming_caller_files
        );
    }

    #[test]
    fn resolve_source_symbol_for_line_prefers_inner_method_over_outer_class_hint() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("tsconfig.json"),
            "{\"compilerOptions\":{\"baseUrl\":\".\"}}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/dep.ts"),
            "export class Dep {\n    ping(): number { return 1; }\n}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/service.ts"),
            "import { Dep } from './dep';\nexport class OuterService {\n    build(): Dep {\n        return new Dep();\n    }\n}\n",
        )
        .unwrap();

        let store = GraphIrStore::open(&state, "enclosing-method").unwrap();
        let delta = store.sync_workspace_with_delta(&root).unwrap();
        assert!(delta.edited_files.contains(&"src/service.ts".to_owned()));
        assert!(delta.changed_paths.contains(&"tsconfig.json".to_owned()));

        let conn = lock_conn(&store.conn).unwrap();
        let edges: Vec<(Option<String>, Option<String>, String)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT source_symbol, target_symbol, relation
                     FROM ir_edges
                     WHERE source_file = 'src/service.ts' AND target_file = 'src/dep.ts'",
                )
                .unwrap();
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };
        assert!(
            edges.iter().any(|(src, _, rel)| {
                rel == "calls" && src.as_deref().is_some_and(|s| s.ends_with(":build"))
            }),
            "expected constructor call inside OuterService.build to attribute to method `build`, got: {edges:?}"
        );
        assert!(
            !edges.iter().any(|(src, _, rel)| {
                rel == "calls" && src.as_deref().is_some_and(|s| s.ends_with(":OuterService"))
            }),
            "outer class hint must not clobber method-level attribution: {edges:?}"
        );
    }

    #[test]
    fn sync_workspace_with_delta_reports_manifest_and_code_changes() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("package.json"),
            "{\"name\":\"app\",\"version\":\"1.0.0\"}\n",
        )
        .unwrap();
        std::fs::write(root.join("src/dep.ts"), "export const x = 1;\n").unwrap();
        std::fs::write(
            root.join("src/use.ts"),
            "import { x } from './dep';\nexport const y = x;\n",
        )
        .unwrap();

        let store = GraphIrStore::open(&state, "sync-delta").unwrap();
        let first = store.sync_workspace_with_delta(&root).unwrap();
        assert_eq!(first.edited_files.len(), 2);
        assert!(first.manifest_changed);
        assert!(first.changed_paths.contains(&"package.json".to_owned()));

        let clean = store.sync_workspace_with_delta(&root).unwrap();
        assert!(clean.changed_paths.is_empty());
        assert!(!clean.manifest_changed);

        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(
            root.join("package.json"),
            "{\"name\":\"app\",\"version\":\"1.0.1\"}\n",
        )
        .unwrap();
        std::fs::write(root.join("src/dep.ts"), "export const x = 2;\n").unwrap();
        let second = store.sync_workspace_with_delta(&root).unwrap();
        assert!(second.manifest_changed);
        assert!(second.changed_paths.contains(&"package.json".to_owned()));
        assert!(second.edited_files.contains(&"src/dep.ts".to_owned()));
        assert!(
            second
                .incoming_caller_files
                .contains(&"src/use.ts".to_owned()),
            "expected incoming caller src/use.ts: {:?}",
            second.incoming_caller_files
        );
    }

    #[test]
    fn persisted_extractions_and_symbol_refs_power_lookup_past_300_files() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join("src/callers")).unwrap();
        std::fs::write(
            root.join("src/target.ts"),
            "export function hotSpot(v: number): number { return v + 1; }\n",
        )
        .unwrap();
        for i in 0..320 {
            std::fs::write(
                root.join(format!("src/callers/c{i:03}.ts")),
                "import { hotSpot } from '../target';\nexport const r = hotSpot(1);\n",
            )
            .unwrap();
        }

        let store = GraphIrStore::open(&state, "persisted-refs").unwrap();
        store.sync_workspace(&root).unwrap();

        assert!(store.has_symbol_refs_index().unwrap());
        let callers = store.lookup_symbol_ref_callers(&["hotSpot"]).unwrap();
        assert_eq!(
            callers.len(),
            321,
            "target + 320 callers must all be indexed"
        );
        assert!(
            store
                .load_file_extraction_if_fresh(&root, "src/callers/c000.ts")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn intra_file_edges_surface_in_symbol_graph_without_polluting_file_coupling() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/calc.ts"),
            r#"
export class Calc {
    compute(v: number): number {
        return helper(v) + this.double(v);
    }
    double(v: number): number {
        return v * 2;
    }
}
function helper(v: number): number { return v + 1; }
"#,
        )
        .unwrap();

        let store = GraphIrStore::open(&state, "intra-file").unwrap();
        store.sync_workspace(&root).unwrap();

        // Cross-file coupling metrics must stay empty (no self file edge).
        assert!(store.all_resolved_file_edges().unwrap().is_empty());
        assert!(
            store
                .incoming_caller_files("src/calc.ts")
                .unwrap()
                .is_empty()
        );

        // Symbol-level graph export must surface intra-file edges.
        let (nodes, links, _, _) = store.export_ir_graph_elements().unwrap();
        assert!(
            links.iter().any(|l| {
                l.source.contains("compute")
                    && l.target.contains("double")
                    && l.relation.as_deref() == Some("calls")
            }),
            "expected intra-file Calc.compute -> Calc.double symbol link in: {links:?}"
        );
        assert!(
            links.iter().any(|l| {
                l.source.contains("compute")
                    && l.target.contains("helper")
                    && l.relation.as_deref() == Some("calls")
            }),
            "expected intra-file Calc.compute -> helper symbol link in: {links:?}"
        );
        let idx = crate::graph_map::Index::from_ir_parts(&root, None, None, nodes, links).unwrap();
        let sym_view = idx.view_at_lod_unfiltered(
            "intra-file",
            crate::graph_map::Lod::Symbol,
            &crate::graph_map::LodFilter {
                scope: "",
                focus: None,
                filter: "",
                certain: false,
                offset: 0,
                limit: None,
            },
        );
        let sym_edges = sym_view["edges"].as_array().unwrap();
        assert!(
            !sym_edges.is_empty(),
            "expected intra-file symbol edges at Lod::Symbol: {sym_view}"
        );
        assert!(
            idx.pairs().is_empty(),
            "intra-file symbol edges must not pollute cross-file coupling pairs: {:?}",
            idx.pairs()
        );
    }

    #[test]
    fn incremental_semantic_reexport_deltas_and_live_json_patch_match_full_export() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let root = dir.path().join("repo");
        let live_json = state.join("live/graph.json");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/impl_a.ts"),
            "export function op(): number { return 1; }\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/impl_b.ts"),
            "export function op(): number { return 2; }\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/barrel.ts"),
            "export { op } from './impl_a';\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/consumer.ts"),
            "import { op } from './barrel';\nexport function run(): number { return op(); }\n",
        )
        .unwrap();
        std::fs::write(root.join("app.php"), "<?php\nDB::table('orders')->get();\n").unwrap();
        std::fs::write(
            root.join("writer.php"),
            "<?php\nclass OrderWriter { public function save() { DB::table('orders')->insert([]); } }\n",
        )
        .unwrap();

        let store = GraphIrStore::open(&state, "inc-sem-reexp").unwrap();
        store.sync_workspace(&root).unwrap();
        store.write_live_graph_atomic(&live_json).unwrap();
        let (base_nodes, base_links, _, _) = store.export_ir_graph_elements().unwrap();
        let mut live_index =
            crate::graph_map::Index::from_ir_parts(&root, None, None, base_nodes, base_links)
                .unwrap();

        // 1. Re-export target swap in barrel.ts propagates incrementally to consumer.ts.
        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(
            root.join("src/barrel.ts"),
            "export { op } from './impl_b';\n",
        )
        .unwrap();
        let d_reexp = store
            .update_paths_with_delta(&root, &["src/barrel.ts".to_owned()])
            .unwrap();
        assert!(d_reexp.reexport_changed);
        assert!(
            d_reexp
                .re_resolved_files
                .contains(&"src/consumer.ts".to_owned()),
            "re-export consumer should be re-resolved: {:?}",
            d_reexp.re_resolved_files
        );
        let ir_d_reexp = store
            .export_ir_delta_for_files(&d_reexp)
            .unwrap()
            .expect("reexport delta must be incremental");
        store
            .patch_live_graph_from_deltas_atomic(&live_json, &[&ir_d_reexp])
            .unwrap();
        live_index = live_index.update_files(&root, ir_d_reexp.clone()).unwrap();

        // 2. Semantic table write addition updates sem:table:invoices incrementally without Ok(None).
        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(
            root.join("writer.php"),
            "<?php\nclass OrderWriter { public function save() { DB::table('orders')->insert([]); DB::table('invoices')->insert([]); } }\n",
        )
        .unwrap();
        let d_sem = store
            .update_paths_with_delta(&root, &["writer.php".to_owned()])
            .unwrap();
        assert!(d_sem.semantic_changed);
        let ir_d_sem = store
            .export_ir_delta_for_files(&d_sem)
            .unwrap()
            .expect("semantic delta must export incrementally instead of Ok(None)");
        store
            .patch_live_graph_from_deltas_atomic(&live_json, &[&ir_d_sem])
            .unwrap();
        live_index = live_index.update_files(&root, ir_d_sem.clone()).unwrap();

        // Patched live/graph.json & incrementally updated Index must reflect impl_b and invoices.
        let patched_bytes = std::fs::read(&live_json).unwrap();
        let patched_val: Value = serde_json::from_slice(&patched_bytes).unwrap();
        let has_invoices = patched_val["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["id"] == "sem:table:invoices");
        assert!(
            has_invoices,
            "patched live/graph.json missing sem:table:invoices"
        );
        assert!(
            patched_val["links"]
                .as_array()
                .unwrap()
                .iter()
                .any(|l| l["source_file"] == "src/consumer.ts"
                    && l["target"]
                        .as_str()
                        .is_some_and(|t| t.contains("impl_b.ts"))),
            "patched live/graph.json missing consumer -> impl_b link: {}",
            patched_val["links"]
        );
        let (full_nodes, full_links, _, _) = store.export_ir_graph_elements().unwrap();
        let full_idx =
            crate::graph_map::Index::from_ir_parts(&root, None, None, full_nodes, full_links)
                .unwrap();
        assert_eq!(live_index.node_link_counts(), full_idx.node_link_counts());
    }

    #[test]
    fn upgrades_v2_db_without_partial_symbol_refs_corruption() {
        let tmp = tempfile::tempdir().unwrap();
        let state = tmp.path().join("state");
        let root = tmp.path().join("proj");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/target.rs"),
            "pub fn target_op() -> i32 { 1 }\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/caller.rs"),
            "pub fn call_site() -> i32 { crate::target::target_op() }\n",
        )
        .unwrap();

        let store_v2 = GraphIrStore::open(&state, "p-v2").unwrap();
        store_v2.sync_workspace(&root).unwrap();
        let db_path = store_v2.db_path().to_path_buf();
        drop(store_v2);

        // Simulate a v2 DB that has populated `ir_files` / `ir_symbols` / `ir_edges`
        // while lacking `ir_file_extractions` and `ir_symbol_refs`.
        {
            let raw = Connection::open(&db_path).unwrap();
            raw.execute_batch(
                "DELETE FROM ir_file_extractions;
                 DELETE FROM ir_symbol_refs;
                 INSERT OR REPLACE INTO ir_meta(key, value) VALUES ('schema_version', '2');",
            )
            .unwrap();
        }

        // Re-opening upgrades schema to v3 and invalidates stale v2 `ir_files` so partial
        // `update_paths_with_delta` never trusts `ir_symbol_refs` as covering un-backfilled files.
        let upgraded = GraphIrStore::open(&state, "p-v2").unwrap();
        assert!(upgraded.known_files().unwrap().is_empty());
        assert!(!upgraded.has_symbol_refs_index().unwrap());

        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(
            root.join("src/target.rs"),
            "pub fn target_op() -> i32 { 2 }\n",
        )
        .unwrap();
        upgraded
            .update_paths_with_delta(&root, &["src/target.rs".to_owned()])
            .unwrap();
        assert_eq!(
            upgraded.known_files().unwrap(),
            BTreeSet::from(["src/target.rs".to_owned()])
        );
        // After editing 1 file before full sync, `graph_impact` still discovers `src/caller.rs` via
        // unindexed fallback because `src/caller.rs` is not falsely claimed as indexed.
        let impact = crate::graph_impact::file_impact_with_ir(
            &root,
            None,
            Some(&upgraded),
            None,
            None,
            "src/target.rs",
            false,
        )
        .unwrap();
        let used_by = impact["used_by"].as_array().unwrap();
        assert!(
            used_by
                .iter()
                .any(|u| u["path"] == "src/caller.rs" || u == "src/caller.rs"),
            "expected caller discovery via unindexed fallback on upgraded partial DB: {impact}"
        );

        // Full `sync_workspace_with_delta` backfills all remaining v2 files completely.
        upgraded.sync_workspace_with_delta(&root).unwrap();
        let callers = upgraded.lookup_symbol_ref_callers(&["target_op"]).unwrap();
        assert!(
            callers.contains(&"src/caller.rs".to_owned()),
            "expected backfilled caller in ir_symbol_refs: {callers:?}"
        );
        assert!(
            upgraded
                .load_file_extraction_if_fresh(&root, "src/caller.rs")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn external_handle_invalidates_reexport_cache_via_meta_epoch() {
        let tmp = tempfile::tempdir().unwrap();
        let state = tmp.path().join("state");
        let root = tmp.path().join("proj");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/impl_a.ts"),
            "export function foo() { return 1; }\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/impl_b.ts"),
            "export function foo() { return 2; }\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/barrel.ts"),
            "export { foo } from './impl_a';\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/consumer.ts"),
            "import { foo } from './barrel';\nexport function run() { return foo(); }\n",
        )
        .unwrap();

        let handle_a = GraphIrStore::open(&state, "p-reexp-ext").unwrap();
        handle_a.sync_workspace_with_delta(&root).unwrap();
        assert!(handle_a.reexport_index().is_ok());

        // Separate writer handle modifies `barrel.ts` in SQLite, advancing `ir_meta.write_epoch`
        // while `handle_a`'s in-memory `AtomicU64` still holds the earlier epoch.
        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(
            root.join("src/barrel.ts"),
            "export { foo } from './impl_b';\n",
        )
        .unwrap();
        let handle_b = GraphIrStore::open(&state, "p-reexp-ext").unwrap();
        handle_b
            .update_paths_with_delta(&root, &["src/barrel.ts".to_owned()])
            .unwrap();

        // Editing `consumer.ts` on `handle_a` must see `handle_b`'s bumped DB epoch and resolve to `impl_b.ts`.
        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(
            root.join("src/consumer.ts"),
            "import { foo } from './barrel';\nexport function run() { return foo() + 1; }\n",
        )
        .unwrap();
        handle_a
            .update_paths_with_delta(&root, &["src/consumer.ts".to_owned()])
            .unwrap();

        let edges = handle_a.all_resolved_file_edges().unwrap();
        assert!(
            edges.contains(&("src/consumer.ts".to_owned(), "src/impl_b.ts".to_owned())),
            "handle_a must invalidate reexport_cache against external DB write_epoch: {edges:?}"
        );
    }
}
