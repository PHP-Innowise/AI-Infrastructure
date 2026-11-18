//! Per-project incremental SQLite Code Graph IR backed by 14-language tree-sitter AST extraction.
//!
//! Stores per-file metadata, extracted declarations (`ir_symbols`), resolved cross-file
//! dependency edges (`ir_edges`), framework/database semantic edges (`ir_semantic_edges`),
//! re-exports (`ir_reexports`), and O(1) counters (`ir_summary`) under
//! `<state>/graphs/<project>/graph_ir.sqlite3`.

use crate::{
    HarnessError, Result,
    graph_ast::{self, AstFileExtraction, AstGrammar, AstRef},
    graph_crates::CratePackageMap,
    graph_deps::{self, ReexportEntry, ReexportIndex},
    graph_live::clean_rel_key,
    graph_manifests::ManifestIndex,
    store,
};
use infra_core::bank::publication::PublicationFiles;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::Mutex,
    time::{Duration, UNIX_EPOCH},
};

pub(crate) const NATIVE_IR_TOOL: &str = "native-tree-sitter-ir";
const SCHEMA_VERSION: &str = "1";
const DB_FILENAME: &str = "graph_ir.sqlite3";
const MAX_IR_FILES: usize = 30_000;
const MAX_WALK_DEPTH: usize = 64;
const MAX_FILE_BYTES: u64 = crate::graph_map::SOURCE_LIMIT;
const MAX_MANIFEST_READ_BYTES: u64 = 512 * 1024;
const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(10);

const IGNORED_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "graphify-out",
    "node_modules",
    "vendor",
    "target",
    "dist",
    "build",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
    ".nuxt",
    ".cargo",
    ".cache",
    ".pytest_cache",
    ".mypy_cache",
    ".idea",
    ".vscode",
    ".c2p",
];

/// Fast O(1) summary counters stored in `ir_summary`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IrSummary {
    pub files: u64,
    pub symbols: u64,
    pub edges: u64,
    pub semantic_edges: u64,
    pub updated_at: String,
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

/// Persistent per-project SQLite Code Graph IR store.
pub(crate) struct GraphIrStore {
    db_path: PathBuf,
    conn: Mutex<Connection>,
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
    source: String,
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

struct StoredSymbolRow {
    row_id: i64,
    file_path: String,
    symbol_id: String,
    kind: String,
    name: String,
    parent_name: Option<String>,
    start_line: u32,
}

struct StoredEdgeRow {
    source_file: String,
    source_symbol: Option<String>,
    target_file: String,
    target_symbol: Option<String>,
    relation: String,
    context: Option<String>,
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
        let conn = Connection::open(db_path)?;
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
                 body_fingerprint TEXT
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
             CREATE TABLE IF NOT EXISTS ir_summary(
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 files INTEGER NOT NULL,
                 symbols INTEGER NOT NULL,
                 edges INTEGER NOT NULL,
                 semantic_edges INTEGER NOT NULL,
                 updated_at TEXT NOT NULL
             );",
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO ir_meta(key, value) VALUES (?1, ?2)",
            params!["schema_version", SCHEMA_VERSION],
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO ir_meta(key, value) VALUES (?1, ?2)",
            params!["backend", NATIVE_IR_TOOL],
        )?;
        Ok(Self {
            db_path: db_path.to_path_buf(),
            conn: Mutex::new(conn),
        })
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

    /// Reads the fast O(1) `ir_summary` row for an existing project DB without creating one.
    pub(crate) fn read_summary_if_exists(state: &Path, project: &str) -> Result<Option<IrSummary>> {
        if !Self::exists(state, project) {
            return Ok(None);
        }
        let home = state.join("graphs").join(project);
        let Ok(_custody) = PublicationFiles::new(&home) else {
            return Ok(None);
        };
        let real_home = infra_core::fs::python_realpath(&home)?;
        let db_path = real_home.join(DB_FILENAME);
        infra_core::fs::admit_descendant(&real_home, &db_path)?;
        let conn = Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(SQLITE_BUSY_TIMEOUT)?;
        read_summary_from_conn(&conn)
    }

    #[allow(dead_code)]
    pub(crate) fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Synchronizes all bounded workspace code files under `root` into SQLite incrementally.
    ///
    /// Files matching `(size, mtime_ns)` in `ir_files` are skipped; changed/new files are parsed
    /// via `graph_ast::parse_file_ir`, deleted files are pruned, and cross-file dependencies +
    /// `ir_summary` are updated atomically in one SQLite transaction.
    pub(crate) fn sync_workspace(&self, root: &Path) -> Result<IrSyncStats> {
        if !root.is_dir() {
            return Err(HarnessError::invalid(
                "Workspace directory does not exist for Graph IR sync.",
            ));
        }
        let real_root = fs::canonicalize(root)?;
        let walked = walk_workspace_code_files(&real_root)?;
        let files_scanned = walked.len();
        let known_files: BTreeSet<String> = walked.iter().map(|w| w.rel_path.clone()).collect();

        let existing: BTreeMap<String, (i64, i64)> = {
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
            map
        };

        let removed: Vec<String> = existing
            .keys()
            .filter(|path| !known_files.contains(*path))
            .cloned()
            .collect();
        let stale: Vec<&WalkedFile> = walked
            .iter()
            .filter(|w| existing.get(&w.rel_path) != Some(&(w.size, w.mtime_ns)))
            .collect();

        if removed.is_empty()
            && stale.is_empty()
            && let Some(summary) = self.summary_counts()?
        {
            return Ok(IrSyncStats {
                files_scanned,
                files_parsed: 0,
                files_skipped: files_scanned,
                files_removed: 0,
                summary,
            });
        }

        let mut parsed_units = Vec::with_capacity(stale.len());
        for entry in &stale {
            if let Some(unit) = parse_workspace_file(&real_root, &entry.rel_path) {
                parsed_units.push(unit);
            }
        }
        let files_parsed = parsed_units.len();
        let files_skipped = files_scanned.saturating_sub(stale.len());
        let files_removed = removed.len();

        let manifests = ManifestIndex::from_root(&real_root);
        let known_vec: Vec<String> = known_files.iter().cloned().collect();
        let crate_map = CratePackageMap::from_reader(&known_vec, &|rel| {
            read_bounded_text_file(&real_root, rel, MAX_MANIFEST_READ_BYTES)
        });

        let summary = {
            let mut conn = lock_conn(&self.conn)?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let now = store::now();

            for path in &removed {
                delete_file_records(&tx, path)?;
            }
            for unit in &parsed_units {
                upsert_parsed_file(&tx, unit, &now)?;
            }

            let reexports = build_reexport_index_from_db(&tx)?;
            for unit in &parsed_units {
                replace_file_edges(&tx, unit, &known_files, &manifests, &reexports, &crate_map)?;
            }

            let summary = recompute_summary(&tx, &now)?;
            upsert_meta(&tx, "updated_at", &now)?;
            tx.commit()?;
            summary
        };

        Ok(IrSyncStats {
            files_scanned,
            files_parsed,
            files_skipped,
            files_removed,
            summary,
        })
    }

    /// Incrementally re-indexes ONLY the specified changed or deleted relative paths in one transaction.
    pub(crate) fn update_paths(&self, root: &Path, rel_paths: &[String]) -> Result<IrSyncStats> {
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
            return Ok(IrSyncStats {
                summary,
                ..IrSyncStats::default()
            });
        }

        let mut parsed_units = Vec::new();
        let mut removed = Vec::new();
        let mut skipped = 0usize;

        for rel in &normalized {
            let full = real_root.join(rel);
            match fs::symlink_metadata(&full) {
                Ok(meta) if !meta.file_type().is_symlink() && meta.is_file() => {
                    if is_supported_code_rel(rel) && meta.len() <= MAX_FILE_BYTES {
                        if let Some(unit) = parse_workspace_file(&real_root, rel) {
                            parsed_units.push(unit);
                        } else {
                            skipped += 1;
                        }
                    } else {
                        removed.push(rel.clone());
                    }
                }
                Ok(_) => removed.push(rel.clone()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => removed.push(rel.clone()),
                Err(_) => skipped += 1,
            }
        }

        let manifests = if parsed_units.is_empty() {
            ManifestIndex::default()
        } else {
            ManifestIndex::from_root(&real_root)
        };

        let files_scanned = normalized.len();
        let files_parsed = parsed_units.len();
        let files_removed;

        let summary = {
            let mut conn = lock_conn(&self.conn)?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let now = store::now();

            let mut existing_files = BTreeSet::new();
            {
                let mut stmt = tx.prepare("SELECT path FROM ir_files")?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                for row in rows {
                    existing_files.insert(row?);
                }
            }

            let mut actually_removed = 0usize;
            for path in &removed {
                if existing_files.remove(path) {
                    actually_removed += 1;
                }
                delete_file_records(&tx, path)?;
            }
            files_removed = actually_removed;

            for unit in &parsed_units {
                existing_files.insert(unit.rel_path.clone());
                upsert_parsed_file(&tx, unit, &now)?;
            }

            if !parsed_units.is_empty() {
                let known_vec: Vec<String> = existing_files.iter().cloned().collect();
                let crate_map = CratePackageMap::from_reader(&known_vec, &|rel| {
                    read_bounded_text_file(&real_root, rel, MAX_MANIFEST_READ_BYTES)
                });
                let reexports = build_reexport_index_from_db(&tx)?;
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
            }

            let summary = recompute_summary(&tx, &now)?;
            upsert_meta(&tx, "updated_at", &now)?;
            tx.commit()?;
            summary
        };

        Ok(IrSyncStats {
            files_scanned,
            files_parsed,
            files_skipped: skipped,
            files_removed,
            summary,
        })
    }

    /// Reads the fast O(1) `ir_summary` counters from SQLite.
    pub(crate) fn summary_counts(&self) -> Result<Option<IrSummary>> {
        let conn = lock_conn(&self.conn)?;
        read_summary_from_conn(&conn)
    }

    /// Exports Graphify-compatible `{"nodes": [...], "links": [...]}` JSON for `graph_map::Index::read`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn export_node_link_json(&self) -> Result<Value> {
        self.export_node_link_json_with_counts().map(|(v, ..)| v)
    }

    /// Exports node-link JSON alongside the exported `(nodes, links)` counts.
    pub(crate) fn export_node_link_json_with_counts(&self) -> Result<(Value, u64, u64)> {
        let (files, symbols, edges, sem_edges) = {
            let mut conn = lock_conn(&self.conn)?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;

            let files = {
                let mut stmt = tx.prepare("SELECT path FROM ir_files ORDER BY path")?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                rows.collect::<std::result::Result<Vec<_>, _>>()?
            };

            let symbols = {
                let mut stmt = tx.prepare(
                    "SELECT id, file_path, symbol_id, kind, name, parent_name, start_line
                     FROM ir_symbols
                     ORDER BY file_path, start_line, id",
                )?;
                let rows = stmt.query_map([], |row| {
                    Ok(StoredSymbolRow {
                        row_id: row.get(0)?,
                        file_path: row.get(1)?,
                        symbol_id: row.get(2)?,
                        kind: row.get(3)?,
                        name: row.get(4)?,
                        parent_name: row.get(5)?,
                        start_line: row.get::<_, i64>(6)?.max(0) as u32,
                    })
                })?;
                rows.collect::<std::result::Result<Vec<_>, _>>()?
            };

            let edges = {
                let mut stmt = tx.prepare(
                    "SELECT source_file, source_symbol, target_file, target_symbol, relation, context, confidence
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
                        confidence: row.get(6)?,
                    })
                })?;
                rows.collect::<std::result::Result<Vec<_>, _>>()?
            };

            let sem_edges = {
                let mut stmt = tx.prepare(
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

            (files, symbols, edges, sem_edges)
        };

        Ok(build_node_link_payload(
            &files, &symbols, &edges, &sem_edges,
        ))
    }

    /// Writes `live/graph.json` via a private temporary file and atomic rename without removing
    /// the previous published `live/graph.json` while building.
    pub(crate) fn write_live_graph_atomic(&self, live_graph_path: &Path) -> Result<(u64, u64)> {
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

        let (payload, node_count, link_count) = self.export_node_link_json_with_counts()?;
        let bytes = serde_json::to_vec(&payload)?;

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
            let mut file = opts.open(&tmp_path)?;
            file.write_all(&bytes)?;
            file.flush()?;
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
        write_result?;
        Ok((node_count, link_count))
    }
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
    if clean.split('/').any(|seg| IGNORED_DIRS.contains(&seg)) {
        return None;
    }
    Some(clean)
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
    let fname = rel.rsplit('/').next().unwrap_or(rel);
    if fname.ends_with(".min.js") || fname.ends_with(".min.mjs") || fname.ends_with(".min.cjs") {
        return false;
    }
    AstGrammar::from_rel_path(rel).is_some()
}

fn mtime_nanos(meta: &fs::Metadata) -> i64 {
    let nanos = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0u128, |d| d.as_nanos());
    i64::try_from(nanos).unwrap_or(i64::MAX)
}

fn walk_workspace_code_files(real_root: &Path) -> Result<Vec<WalkedFile>> {
    let mut out = Vec::new();
    let mut queue = VecDeque::new();
    queue.push_back((real_root.to_path_buf(), String::new(), 0usize));

    while let Some((dir, prefix, depth)) = queue.pop_front() {
        if out.len() >= MAX_IR_FILES {
            break;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut children: Vec<_> = entries.flatten().collect();
        children.sort_by_key(|e| e.file_name());
        for entry in children {
            if out.len() >= MAX_IR_FILES {
                break;
            }
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
                if depth < MAX_WALK_DEPTH && !IGNORED_DIRS.contains(&leaf) {
                    queue.push_back((path, rel, depth + 1));
                }
            } else if meta.is_file() && is_supported_code_rel(&rel) && meta.len() <= MAX_FILE_BYTES
            {
                out.push(WalkedFile {
                    rel_path: rel,
                    size: i64::try_from(meta.len()).unwrap_or(i64::MAX),
                    mtime_ns: mtime_nanos(&meta),
                });
            }
        }
    }
    out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(out)
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

fn read_bounded_text_file(real_root: &Path, rel_path: &str, max_bytes: u64) -> Option<String> {
    let (bytes, _, _) = read_bounded_bytes(real_root, rel_path, max_bytes)?;
    String::from_utf8(bytes).ok()
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
        source,
        ir,
    })
}

fn delete_file_records(tx: &Transaction<'_>, rel_path: &str) -> Result<()> {
    tx.execute("DELETE FROM ir_files WHERE path = ?1", [rel_path])?;
    tx.execute("DELETE FROM ir_symbols WHERE file_path = ?1", [rel_path])?;
    tx.execute("DELETE FROM ir_reexports WHERE file_path = ?1", [rel_path])?;
    tx.execute(
        "DELETE FROM ir_semantic_edges WHERE file_path = ?1",
        [rel_path],
    )?;
    tx.execute(
        "DELETE FROM ir_edges WHERE source_file = ?1 OR target_file = ?1",
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

    {
        let mut sym_stmt = tx.prepare_cached(
            "INSERT INTO ir_symbols(
                file_path, symbol_id, kind, name, parent_name, signature,
                visibility, start_line, end_line, modifiers_json, body_fingerprint
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
                body_fp
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

    Ok(())
}

fn build_reexport_index_from_db(tx: &Transaction<'_>) -> Result<ReexportIndex> {
    let mut index = ReexportIndex::default();
    {
        let mut stmt = tx.prepare("SELECT file_path, kind, name, parent_name FROM ir_symbols")?;
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
        let mut stmt = tx.prepare(
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
    tx.execute(
        "DELETE FROM ir_edges WHERE source_file = ?1",
        [&unit.rel_path],
    )?;
    let rows = extract_ir_edges_for_file(
        &unit.rel_path,
        &unit.source,
        &unit.ir,
        known_files,
        manifests,
        reexports,
        crate_map,
    );
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
    Ok(())
}

fn extract_ir_edges_for_file(
    rel_path: &str,
    source: &str,
    ir: &AstFileExtraction,
    known_files: &BTreeSet<String>,
    manifests: &ManifestIndex,
    reexports: &ReexportIndex,
    crate_map: &CratePackageMap,
) -> Vec<IrEdgeRow> {
    let deps = graph_deps::extract_file_deps_with_context(
        rel_path,
        source,
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
    let mut seen: BTreeSet<EdgeDedupeKey> = BTreeSet::new();

    let mut push_row = |row: IrEdgeRow| {
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
        if !dep.resolved
            || !dep.exists
            || dep.target == rel_path
            || !known_files.contains(&dep.target)
        {
            continue;
        }
        let dep_line = dep.line.unwrap_or(1);
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

    rows
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

fn resolve_source_symbol_for_line(
    ir: &AstFileExtraction,
    hint: Option<&str>,
    line: u32,
) -> Option<String> {
    if let Some(hint) = hint.map(str::trim).filter(|s| !s.is_empty())
        && let Some(found) = ir.declarations.iter().find(|d| {
            d.entity.id == hint
                || d.entity.name == hint
                || d.entity
                    .parent
                    .as_deref()
                    .is_some_and(|p| format!("{p}::{}", d.entity.name) == hint)
        })
        && line >= found.start_line
        && line <= found.end_line
    {
        return Some(found.entity.id.clone());
    }
    let mut best: Option<&crate::graph_live::ExtractedDecl> = None;
    for decl in &ir.declarations {
        if line >= decl.start_line && line <= decl.end_line {
            let span = decl.end_line.saturating_sub(decl.start_line);
            let replace = match best {
                None => true,
                Some(prev) => {
                    let prev_span = prev.end_line.saturating_sub(prev.start_line);
                    span < prev_span
                        || (span == prev_span
                            && matches!(
                                decl.entity.kind.as_str(),
                                "method" | "function" | "constructor"
                            ))
                }
            };
            if replace {
                best = Some(decl);
            }
        }
    }
    best.map(|d| d.entity.id.clone())
}

fn recompute_summary(tx: &Transaction<'_>, now: &str) -> Result<IrSummary> {
    let files: i64 = tx.query_row("SELECT COUNT(*) FROM ir_files", [], |r| r.get(0))?;
    let symbols: i64 = tx.query_row("SELECT COUNT(*) FROM ir_symbols", [], |r| r.get(0))?;
    let edges: i64 = tx.query_row("SELECT COUNT(*) FROM ir_edges", [], |r| r.get(0))?;
    let semantic_edges: i64 =
        tx.query_row("SELECT COUNT(*) FROM ir_semantic_edges", [], |r| r.get(0))?;
    tx.execute(
        "INSERT OR REPLACE INTO ir_summary(id, files, symbols, edges, semantic_edges, updated_at)
         VALUES (1, ?1, ?2, ?3, ?4, ?5)",
        params![files, symbols, edges, semantic_edges, now],
    )?;
    Ok(IrSummary {
        files: files.max(0) as u64,
        symbols: symbols.max(0) as u64,
        edges: edges.max(0) as u64,
        semantic_edges: semantic_edges.max(0) as u64,
        updated_at: now.to_owned(),
    })
}

fn upsert_meta(tx: &Transaction<'_>, key: &str, value: &str) -> Result<()> {
    tx.execute(
        "INSERT OR REPLACE INTO ir_meta(key, value) VALUES (?1, ?2)",
        params![key, value],
    )?;
    Ok(())
}

fn read_summary_from_conn(conn: &Connection) -> Result<Option<IrSummary>> {
    conn.query_row(
        "SELECT files, symbols, edges, semantic_edges, updated_at FROM ir_summary WHERE id = 1",
        [],
        |row| {
            Ok(IrSummary {
                files: row.get::<_, i64>(0)?.max(0) as u64,
                symbols: row.get::<_, i64>(1)?.max(0) as u64,
                edges: row.get::<_, i64>(2)?.max(0) as u64,
                semantic_edges: row.get::<_, i64>(3)?.max(0) as u64,
                updated_at: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
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

fn build_node_link_payload(
    files: &[String],
    symbols: &[StoredSymbolRow],
    edges: &[StoredEdgeRow],
    sem_edges: &[StoredSemanticRow],
) -> (Value, u64, u64) {
    let mut nodes = Vec::with_capacity(files.len() + symbols.len());
    let mut links = Vec::with_capacity(symbols.len() + edges.len() + sem_edges.len());

    let file_set: BTreeSet<&str> = files.iter().map(String::as_str).collect();
    let mut used_node_ids: BTreeSet<String> = BTreeSet::new();
    let mut by_symbol_id: BTreeMap<(&str, &str), String> = BTreeMap::new();
    let mut by_member: BTreeMap<(&str, &str, &str), String> = BTreeMap::new();
    let mut by_name: BTreeMap<(&str, &str), String> = BTreeMap::new();
    let mut type_by_file_and_name: BTreeMap<(&str, &str), String> = BTreeMap::new();
    let mut global_type_by_name: BTreeMap<String, Option<String>> = BTreeMap::new();

    for path in files {
        let fid = format!("file:{path}");
        used_node_ids.insert(fid.clone());
        let label = path.rsplit('/').next().unwrap_or(path);
        nodes.push(json!({
            "id": fid,
            "label": label,
            "source_file": path,
            "file_type": "code",
            "node_kind": "page",
        }));
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
            .or_insert_with(|| node_id.clone());
        if let Some(parent) = sym.parent_name.as_deref() {
            by_member
                .entry((&sym.file_path, parent, &sym.name))
                .or_insert_with(|| node_id.clone());
        }
        if is_type_kind(&sym.kind) {
            type_by_file_and_name
                .entry((&sym.file_path, &sym.name))
                .or_insert_with(|| node_id.clone());
            match global_type_by_name.get(&sym.name) {
                None => {
                    global_type_by_name.insert(sym.name.clone(), Some(node_id.clone()));
                }
                Some(Some(existing)) if existing != &node_id => {
                    global_type_by_name.insert(sym.name.clone(), None);
                }
                _ => {}
            }
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

        let mut node = json!({
            "id": node_id,
            "label": label,
            "source_file": sym.file_path,
            "source_location": format!("L{}", sym.start_line),
            "node_kind": sym.kind,
        });
        if is_type_kind(&sym.kind) {
            node["_callable_class"] = json!(true);
        }
        nodes.push(node);
    }

    for (sym, node_id) in symbols.iter().zip(assigned_ids.iter()) {
        let Some(parent_name) = sym.parent_name.as_deref() else {
            continue;
        };
        if let Some(parent_id) = type_by_file_and_name
            .get(&(sym.file_path.as_str(), parent_name))
            .or_else(|| by_name.get(&(sym.file_path.as_str(), parent_name)))
            && parent_id != node_id
        {
            let rel = if matches!(sym.kind.as_str(), "method" | "constructor") {
                "method"
            } else {
                "contains"
            };
            links.push(json!({
                "source": parent_id,
                "target": node_id,
                "relation": rel,
            }));
        }
    }

    let resolve_symbol = |file_path: &str, sym_hint: Option<&str>| -> String {
        if let Some(hint) = sym_hint.map(str::trim).filter(|s| !s.is_empty()) {
            if let Some(id) = by_symbol_id.get(&(file_path, hint)) {
                return id.clone();
            }
            if let Some((parent, member)) = hint.split_once("::") {
                let clean_member = member.trim_start_matches('$');
                if let Some(id) = by_member
                    .get(&(file_path, parent, member))
                    .or_else(|| by_member.get(&(file_path, parent, clean_member)))
                {
                    return id.clone();
                }
            }
            let clean_hint = hint.trim_start_matches('$');
            if let Some(id) = by_name
                .get(&(file_path, hint))
                .or_else(|| by_name.get(&(file_path, clean_hint)))
            {
                return id.clone();
            }
        }
        format!("file:{file_path}")
    };

    for edge in edges {
        if !file_set.contains(edge.source_file.as_str())
            || !file_set.contains(edge.target_file.as_str())
        {
            continue;
        }
        let src_id = resolve_symbol(&edge.source_file, edge.source_symbol.as_deref());
        let dst_id = resolve_symbol(&edge.target_file, edge.target_symbol.as_deref());
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
        let mut link = json!({
            "source": src_id,
            "target": dst_id,
            "relation": relation,
            "confidence": edge.confidence,
            "source_file": edge.source_file,
        });
        if let Some(ctx) = &edge.context {
            link["context"] = json!(ctx);
        }
        links.push(link);
    }

    // Choose a deterministic anchor file for each shared semantic target (e.g., writers/publishers
    // before readers/listeners) so `graph_map::patterns` and `system_graph` join across files.
    let mut semantic_anchors: BTreeMap<&str, (u8, &str, u32)> = BTreeMap::new();
    for edge in sem_edges {
        if !file_set.contains(edge.file_path.as_str()) {
            continue;
        }
        let prio = match edge.kind.as_str() {
            "writes_table" | "publishes_event" | "exposes_route" | "di_binds" => 0u8,
            _ => 1u8,
        };
        let cand = (prio, edge.file_path.as_str(), edge.line);
        match semantic_anchors.get(edge.target.as_str()) {
            Some(prev) if *prev <= cand => {}
            _ => {
                semantic_anchors.insert(edge.target.as_str(), cand);
            }
        }
    }

    let mut emitted_semantic_nodes: BTreeMap<&str, String> = BTreeMap::new();
    for edge in sem_edges {
        if !file_set.contains(edge.file_path.as_str()) {
            continue;
        }
        let src_id = resolve_symbol(&edge.file_path, edge.source_symbol.as_deref());
        let matched_type_id = if edge.kind == "di_binds"
            && let Some(svc) = edge.target.strip_prefix("service:")
        {
            let short = svc.rsplit(['\\', ':', '.']).next().unwrap_or(svc);
            global_type_by_name
                .get(short)
                .and_then(|opt| opt.as_ref())
                .filter(|id| *id != &src_id)
                .cloned()
        } else {
            None
        };

        let dst_id = if let Some(type_id) = matched_type_id {
            type_id
        } else if let Some(existing) = emitted_semantic_nodes.get(edge.target.as_str()) {
            existing.clone()
        } else {
            let sem_id = format!("sem:{}", edge.target);
            let (_, anchor_file, anchor_line) = semantic_anchors
                .get(edge.target.as_str())
                .copied()
                .unwrap_or((1, edge.file_path.as_str(), edge.line));
            nodes.push(json!({
                "id": sem_id,
                "label": edge.target,
                "source_file": anchor_file,
                "source_location": format!("L{anchor_line}"),
                "node_kind": "semantic",
            }));
            emitted_semantic_nodes.insert(edge.target.as_str(), sem_id.clone());
            sem_id
        };

        if src_id == dst_id {
            continue;
        }
        links.push(json!({
            "source": src_id,
            "target": dst_id,
            "relation": edge.kind,
            "context": edge.kind,
            "evidence": edge.evidence,
            "confidence": "EXTRACTED",
            "source_file": edge.file_path,
        }));
    }

    let node_count = nodes.len() as u64;
    let link_count = links.len() as u64;
    (
        json!({
            "directed": true,
            "multigraph": false,
            "backend": NATIVE_IR_TOOL,
            "nodes": nodes,
            "links": links,
        }),
        node_count,
        link_count,
    )
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
}
