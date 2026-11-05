//! SQLite state, migrations and native working-memory transactions.
use crate::{Result, Roots, RuntimeContext, RuntimeError, domain::*, fs};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Duration,
};

pub const TOKENIZER: &str = "porter unicode61";
pub const DEFAULT_TURN_FLUSH_AFTER: usize = 5;
pub const DEFAULT_TURN_FILE_LIMIT: usize = 20;

#[derive(Clone, Debug)]
pub struct OpenOptions {
    pub startup_wait: Duration,
    pub busy_wait: Duration,
    pub require_write: bool,
}
impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            startup_wait: Duration::from_secs(5),
            busy_wait: Duration::from_secs(5),
            require_write: false,
        }
    }
}
impl OpenOptions {
    pub fn refresh(budget: Option<f64>) -> Self {
        let budget = budget.filter(|b| b.is_finite() && *b > 0.0);
        Self {
            startup_wait: Duration::from_secs_f64(
                budget.map(|b| (b * 0.25).clamp(0.05, 2.0)).unwrap_or(2.0),
            ),
            busy_wait: Duration::from_secs_f64(
                budget.map(|b| (b * 0.5).clamp(0.05, 5.0)).unwrap_or(5.0),
            ),
            require_write: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkingTask {
    pub task_id: String,
    pub goal: String,
    pub progress: String,
    #[serde(default)]
    pub auto_checkpoint: String,
    pub next_steps: Vec<String>,
    pub files: Vec<String>,
    pub sources: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StartTask {
    pub task_id: String,
    pub goal: String,
    pub files: Vec<String>,
    pub sources: Vec<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateTask {
    pub task_id: String,
    pub progress: Option<String>,
    pub auto_checkpoint: Option<String>,
    pub next_steps: Vec<String>,
    pub files: Vec<String>,
    pub sources: Vec<String>,
    pub replace_next_steps: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EpisodeInput {
    pub summary: String,
    pub outcome: String,
    pub files: Vec<String>,
    pub verification: Vec<String>,
    pub sources: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Episode {
    pub id: i64,
    pub layer: String,
    pub summary: String,
    pub outcome: String,
    pub files: Vec<String>,
    pub verification: Vec<String>,
    pub sources: Vec<String>,
    pub created_at: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CompleteTask {
    pub task_id: String,
    pub summary: Option<String>,
    pub outcome: String,
    pub files: Vec<String>,
    pub verification: Vec<String>,
    pub sources: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnDelta {
    pub id: i64,
    pub task_id: String,
    pub created_at: String,
    pub files: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Binding {
    pub external_id: String,
    pub task_uuid: String,
    pub revision: u64,
    pub updated_at: String,
}

pub struct Store {
    pub conn: Connection,
    pub path: PathBuf,
    pub read_only: bool,
}

fn columns(conn: &Connection, table: &str) -> Result<BTreeSet<String>> {
    let mut statement = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    Ok(rows.collect::<std::result::Result<BTreeSet<_>, _>>()?)
}
fn schema(conn: &Connection, name: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name=?1",
            [name],
            |row| row.get(0),
        )
        .optional()?)
}

/// Inspect the physical main-file header through the connection's own VFS.
/// Opening and closing another ordinary descriptor for this file would release
/// this process's POSIX locks, including locks owned by another connection.
fn read_only_header(conn: &Connection) -> Result<[u8; 20]> {
    use rusqlite::ffi;

    let unavailable = || RuntimeError::unavailable("Cannot inspect read-only local state header");
    let mut file: *mut ffi::sqlite3_file = std::ptr::null_mut();
    // SAFETY: `conn` owns the live SQLite handle. FILE_POINTER writes one
    // sqlite3_file pointer to the provided output slot and does not transfer
    // ownership. This synchronous helper performs no SQL or concurrent access
    // to the connection, and never closes or retains the returned file.
    let status = unsafe {
        ffi::sqlite3_file_control(
            conn.handle(),
            c"main".as_ptr(),
            ffi::SQLITE_FCNTL_FILE_POINTER,
            std::ptr::from_mut(&mut file).cast(),
        )
    };
    if status != ffi::SQLITE_OK || file.is_null() {
        return Err(unavailable());
    }
    // SAFETY: successful FILE_POINTER returned a live main-file object owned
    // by `conn`. Read only the stable prefix of its VFS method table, then
    // reject unavailable or unsupported callback tables before invoking one.
    let methods = unsafe { (*file).pMethods };
    if methods.is_null() || unsafe { (*methods).iVersion } < 1 {
        return Err(unavailable());
    }
    let read = unsafe { (*methods).xRead }.ok_or_else(unavailable)?;
    let mut header = [0_u8; 20];
    // SAFETY: xRead belongs to this live VFS file, the destination is exactly
    // 20 writable bytes, and offset zero and amount 20 are in range. The
    // borrowed connection outlives the call. Short reads and every VFS error
    // fail closed; no xClose or ordinary file-descriptor operation occurs.
    let status = unsafe { read(file, header.as_mut_ptr().cast(), 20, 0) };
    if status != ffi::SQLITE_OK {
        return Err(unavailable());
    }
    Ok(header)
}

pub fn default_database(roots: &Roots) -> Result<PathBuf> {
    let own = roots.shared
        || (roots.attached()
            && crate::git::GitFrame::discover(&roots.project)?
                .is_some_and(|frame| frame.admin != frame.top.join(".git")));
    if own {
        let canonical = crate::fs::canonicalize(&roots.project)?;
        #[cfg(unix)]
        let bytes = {
            use std::os::unix::ffi::OsStrExt;
            canonical.as_os_str().as_bytes().to_vec()
        };
        #[cfg(not(unix))]
        let bytes = canonical.to_string_lossy().as_bytes().to_vec();
        let digest = crate::query::sha256(&bytes);
        Ok(roots
            .state
            .join("memory-bank/local/worktrees")
            .join(&digest[..16])
            .join("context.db"))
    } else {
        Ok(roots.state.join("memory-bank/local/context.db"))
    }
}

impl Store {
    pub fn open(path: impl AsRef<Path>, options: OpenOptions) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::ensure_directory(parent, true)?;
        }
        if let Ok(metadata) = std::fs::symlink_metadata(path)
            && (fs::linked_at(path, &metadata)? || !metadata.is_file())
        {
            return Err(RuntimeError::policy(
                "Local database must be a regular file",
            ));
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(options.startup_wait)?;
        conn.execute_batch("BEGIN IMMEDIATE")?;
        // Inspecting and migrating the whole schema is most of an open; a
        // store this source already brought current only proves it is writable.
        let initialized = if schema_current(&conn) {
            writable(&conn, options.require_write)
        } else {
            initialize(&conn, options.require_write).map(|()| {
                // A store that cannot keep the mark is initialized again next time.
                let _ = mark_schema(&conn);
            })
        };
        if let Err(error) = initialized {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(error);
        }
        conn.execute_batch("COMMIT")?;
        conn.busy_timeout(options.busy_wait)?;
        Ok(Self {
            conn,
            path: path.to_path_buf(),
            read_only: false,
        })
    }

    pub fn open_read_only(path: impl AsRef<Path>, timeout: Duration) -> Result<Self> {
        let path = path.as_ref();
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.is_file() || fs::linked_at(path, &metadata)? {
            return Err(RuntimeError::policy(
                "Local database must be a regular file",
            ));
        }
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        // Opening the pager reads the physical header without initializing WAL.
        // Reject non-rollback profiles before even the first PRAGMA/schema SQL.
        let header = read_only_header(&conn)?;
        if &header[..16] != b"SQLite format 3\0" || header[18..20] != [1, 1] {
            return Err(RuntimeError::unavailable(
                "Unsupported read-only local state profile",
            ));
        }
        conn.busy_timeout(timeout)?;
        conn.execute_batch("PRAGMA query_only=ON")?;
        let expected: &[(&str, &[&str])] = &[
            (
                "documents",
                &["path", "layer", "kind", "title", "summary", "content"],
            ),
            (
                "episodes",
                &[
                    "summary",
                    "outcome",
                    "files",
                    "verification",
                    "sources",
                    "created_at",
                ],
            ),
            (
                "working_tasks",
                &[
                    "task_id",
                    "goal",
                    "progress",
                    "auto_checkpoint",
                    "next_steps",
                    "files",
                    "sources",
                    "created_at",
                    "updated_at",
                ],
            ),
            ("turn_deltas", &["id", "task_id", "created_at", "files"]),
            (
                "task_bindings",
                &["external_id", "task_uuid", "revision", "updated_at"],
            ),
            (
                "document_metadata",
                &[
                    "path",
                    "category",
                    "privacy",
                    "owner",
                    "authority",
                    "lifecycle",
                    "source_hash",
                    "record_id",
                    "conflicts",
                    "source_fingerprints",
                    "updated_at",
                    "confidence",
                    "attestation",
                ],
            ),
            (
                "document_source_state",
                &["path", "mtime_ns", "size", "eligible_until"],
            ),
            ("document_links", &["path", "ref_path", "ref_kind"]),
            ("index_state", &["key", "value"]),
        ];
        for (table, names) in expected {
            let actual = columns(&conn, table)?;
            if actual != names.iter().map(|name| name.to_string()).collect() {
                return Err(RuntimeError::unavailable(
                    "Unsupported read-only local state profile",
                ));
            }
            let sql = schema(&conn, table)?.ok_or_else(|| {
                RuntimeError::unavailable("Unsupported read-only local state profile")
            })?;
            let kind: String = conn.query_row(
                "SELECT type FROM sqlite_master WHERE name=?1",
                [*table],
                |row| row.get(0),
            )?;
            if kind != "table" {
                return Err(RuntimeError::unavailable(
                    "Unsupported read-only local state profile",
                ));
            }
            if ["documents", "episodes"].contains(table) {
                let fields = names
                    .iter()
                    .map(|name| {
                        format!(
                            "{name}{}",
                            if (*table == "documents" && ["path", "layer", "kind"].contains(name))
                                || (*table == "episodes" && *name == "created_at")
                            {
                                " UNINDEXED"
                            } else {
                                ""
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                let canonical = format!(
                    "CREATE VIRTUAL TABLE {table} USING fts5({fields},tokenize='{TOKENIZER}')"
                );
                let normalize = |text: &str| {
                    text.chars()
                        .filter(|character| !character.is_whitespace())
                        .flat_map(char::to_lowercase)
                        .collect::<String>()
                };
                if normalize(&sql) != normalize(&canonical) {
                    return Err(RuntimeError::unavailable(
                        "Unsupported read-only local state profile",
                    ));
                }
            } else {
                if sql.to_ascii_lowercase().contains("create virtual table") {
                    return Err(RuntimeError::unavailable(
                        "Unsupported read-only local state profile",
                    ));
                }
                let expected_keys: &[&str] = match *table {
                    "working_tasks" => &["task_id"],
                    "turn_deltas" => &["id"],
                    "task_bindings" => &["external_id"],
                    "document_metadata" | "document_source_state" => &["path"],
                    "document_links" => &["path", "ref_path", "ref_kind"],
                    "index_state" => &["key"],
                    _ => &[],
                };
                let mut statement = conn.prepare(&format!("PRAGMA table_info({table})"))?;
                let mut keys = statement
                    .query_map([], |row| {
                        Ok((row.get::<_, usize>(5)?, row.get::<_, String>(1)?))
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                keys.retain(|(position, _)| *position > 0);
                keys.sort_by_key(|(position, _)| *position);
                if keys.into_iter().map(|(_, name)| name).collect::<Vec<_>>() != expected_keys {
                    return Err(RuntimeError::unavailable(
                        "Unsupported read-only local state profile",
                    ));
                }
            }
        }
        let mut unique_uuid = false;
        let mut statement =
            conn.prepare("SELECT name,\"unique\",partial FROM pragma_index_list('task_bindings')")?;
        for row in statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, bool>(1)?,
                row.get::<_, bool>(2)?,
            ))
        })? {
            let (name, unique, partial) = row?;
            if !unique || partial {
                continue;
            }
            let mut index =
                conn.prepare("SELECT name FROM pragma_index_info(?1) ORDER BY seqno")?;
            let fields = index
                .query_map([name], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if fields == ["task_uuid"] {
                unique_uuid = true;
            }
        }
        drop(statement);
        if !unique_uuid {
            return Err(RuntimeError::unavailable(
                "Unsupported read-only local state profile",
            ));
        }
        for (name, fields) in [
            ("document_metadata_conflicts", vec!["conflicts"]),
            ("document_links_ref", vec!["ref_path"]),
            (
                "document_links_source_path",
                vec!["ref_path", "ref_kind", "path"],
            ),
        ] {
            let mut statement =
                conn.prepare("SELECT name FROM pragma_index_info(?1) ORDER BY seqno")?;
            let actual = statement
                .query_map([name], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if actual != fields {
                return Err(RuntimeError::unavailable(
                    "Unsupported read-only local state profile",
                ));
            }
        }
        Ok(Self {
            conn,
            path: path.to_path_buf(),
            read_only: true,
        })
    }

    pub fn transaction<T>(
        &mut self,
        operation: impl FnOnce(&Connection) -> Result<T>,
    ) -> Result<T> {
        if self.read_only {
            return Err(RuntimeError::unavailable("SQLite writes unavailable"));
        }
        let ambient = !self.conn.is_autocommit();
        let savepoint = format!("state_{}", uuid4().replace('-', ""));
        self.conn.execute_batch(&if ambient {
            format!("SAVEPOINT {savepoint}")
        } else {
            "BEGIN IMMEDIATE".into()
        })?;
        match operation(&self.conn) {
            Ok(value) => {
                let committed = self.conn.execute_batch(&if ambient {
                    format!("RELEASE SAVEPOINT {savepoint}")
                } else {
                    "COMMIT".into()
                });
                if let Err(error) = committed {
                    let _ = self.conn.execute_batch(&if ambient {
                        format!("ROLLBACK TO {savepoint}; RELEASE {savepoint}")
                    } else {
                        "ROLLBACK".into()
                    });
                    return Err(error.into());
                }
                Ok(value)
            }
            Err(error) => {
                let _ = self.conn.execute_batch(&if ambient {
                    format!("ROLLBACK TO {savepoint}; RELEASE {savepoint}")
                } else {
                    "ROLLBACK".into()
                });
                Err(error)
            }
        }
    }

    pub fn state(&self) -> Result<BTreeMap<String, String>> {
        load_state(&self.conn)
    }
    pub fn put_state(&mut self, values: &BTreeMap<String, String>) -> Result<()> {
        self.transaction(|conn| store_state(conn, values))
    }
    pub fn working(&self, id: &str) -> Result<Option<WorkingTask>> {
        working(&self.conn, &validate_task_id(id)?)
    }
    pub fn start(&mut self, request: StartTask) -> Result<WorkingTask> {
        let id = validate_task_id(&request.task_id)?;
        let goal = request.goal.trim().to_string();
        if goal.is_empty() {
            return Err(RuntimeError::invalid("Working task goal must not be empty"));
        }
        guard_shared_text(
            "Working task",
            &[
                vec![id.clone(), goal.clone()],
                request.files.clone(),
                request.sources.clone(),
            ]
            .concat(),
        )?;
        validate_values(&request.files)?;
        let now = utc_now();
        self.transaction(|conn| {
            if working(conn,&id)?.is_some() { return Err(RuntimeError::conflict(format!("Working task already exists: {id}"))); }
            conn.execute("INSERT INTO working_tasks(task_id,goal,progress,next_steps,files,sources,created_at,updated_at) VALUES(?1,?2,'','[]',?3,?4,?5,?5)",params![id,goal,serde_json::to_string(&request.files)?,serde_json::to_string(&unique_strings(request.sources))?,now])?;
            working(conn,&id)?.ok_or_else(|| RuntimeError::not_found("Working task disappeared"))
        })
    }
    pub fn update(&mut self, request: UpdateTask) -> Result<WorkingTask> {
        let id = validate_task_id(&request.task_id)?;
        if request.progress.is_none()
            && request.auto_checkpoint.is_none()
            && request.next_steps.is_empty()
            && request.files.is_empty()
            && request.sources.is_empty()
            && !request.replace_next_steps
        {
            return Err(RuntimeError::invalid(
                "Working task update requires a changed field",
            ));
        }
        for (prose, name) in [
            (&request.progress, "progress"),
            (&request.auto_checkpoint, "auto checkpoint"),
        ] {
            if prose.as_ref().is_some_and(|prose| prose.trim().is_empty()) {
                return Err(RuntimeError::invalid(format!(
                    "Working task {name} must not be empty"
                )));
            }
        }
        validate_values(&request.files)?;
        let mut introduced = vec![id.clone()];
        introduced.extend(request.progress.iter().cloned());
        introduced.extend(request.auto_checkpoint.iter().cloned());
        introduced.extend(request.next_steps.clone());
        introduced.extend(request.files.clone());
        introduced.extend(request.sources.clone());
        guard_shared_text("Working task", &introduced)?;
        self.transaction(|conn| {
            let mut task = working(conn,&id)?.ok_or_else(|| RuntimeError::not_found(format!("Working task not found: {id}")))?;
            if let Some(progress) = request.progress { task.progress = progress.trim().into(); }
            if let Some(checkpoint) = request.auto_checkpoint { task.auto_checkpoint = checkpoint.trim().into(); }
            task.next_steps = if request.replace_next_steps { unique_strings(request.next_steps) } else { unique_strings(task.next_steps.into_iter().chain(request.next_steps)) };
            task.files = merge_recent(&task.files,&request.files); task.sources = unique_strings(task.sources.into_iter().chain(request.sources)); task.updated_at=utc_now();
            conn.execute("UPDATE working_tasks SET progress=?1,auto_checkpoint=?2,next_steps=?3,files=?4,sources=?5,updated_at=?6 WHERE task_id=?7",params![task.progress,task.auto_checkpoint,serde_json::to_string(&task.next_steps)?,serde_json::to_string(&task.files)?,serde_json::to_string(&task.sources)?,task.updated_at,id])?;
            Ok(task)
        })
    }
    pub fn clear(&mut self, id: &str) -> Result<()> {
        let id = validate_task_id(id)?;
        self.transaction(|conn| {
            if conn.execute("DELETE FROM working_tasks WHERE task_id=?1", [&id])? != 1 {
                return Err(RuntimeError::not_found(format!(
                    "Working task not found: {id}"
                )));
            }
            Ok(())
        })
    }
    pub fn record_episode(&mut self, input: EpisodeInput) -> Result<i64> {
        self.transaction(|conn| insert_episode(conn, &input))
    }
    pub fn complete(&mut self, input: CompleteTask) -> Result<i64> {
        let id = validate_task_id(&input.task_id)?;
        self.transaction(|conn| {
            let task = working(conn, &id)?
                .ok_or_else(|| RuntimeError::not_found(format!("Working task not found: {id}")))?;
            let episode = EpisodeInput {
                summary: input.summary.unwrap_or(task.goal),
                outcome: input.outcome,
                files: unique_strings(task.files.into_iter().chain(input.files)),
                verification: input.verification,
                sources: unique_strings(task.sources.into_iter().chain(input.sources)),
            };
            let rowid = insert_episode(conn, &episode)?;
            if conn.execute("DELETE FROM working_tasks WHERE task_id=?1", [id])? != 1 {
                return Err(RuntimeError::conflict(
                    "Working task changed during completion",
                ));
            }
            Ok(rowid)
        })
    }
    pub fn binding(&self, id: &str) -> Result<Option<Binding>> {
        let id = validate_task_id(id)?;
        Ok(self.conn.query_row("SELECT external_id,task_uuid,revision,updated_at FROM task_bindings WHERE external_id=?1 OR task_uuid=?1",[id],|row|Ok(Binding { external_id:row.get(0)?,task_uuid:row.get(1)?,revision:row.get(2)?,updated_at:row.get(3)? })).optional()?)
    }
    pub fn bind(&mut self, external_id: &str, task_uuid: &str, revision: u64) -> Result<()> {
        let external = validate_task_id(external_id)?;
        if !crate::domain::is_uuid4(task_uuid) || revision == 0 {
            return Err(RuntimeError::invalid("Invalid task binding"));
        }
        self.transaction(|conn| { let now=utc_now(); conn.execute("INSERT INTO task_bindings(external_id,task_uuid,revision,updated_at) VALUES(?1,?2,?3,?4)",params![external,task_uuid,revision,now])?; conn.execute("INSERT INTO working_tasks(task_id,goal,progress,next_steps,files,sources,created_at,updated_at) VALUES(?1,?2,'','[]','[]','[]',?3,?3)",params![external,task_uuid,now])?; Ok(()) })
    }
    pub fn refresh_binding(&mut self, external_id: &str, revision: u64) -> Result<()> {
        self.transaction(|conn| {
            if conn.execute(
                "UPDATE task_bindings SET revision=?1,updated_at=?2 WHERE external_id=?3",
                params![revision, utc_now(), external_id],
            )? != 1
            {
                return Err(RuntimeError::not_found("Working task not found"));
            }
            Ok(())
        })
    }
    pub fn remove_binding(&mut self, external_id: &str) -> Result<()> {
        self.transaction(|conn| {
            if conn.execute(
                "DELETE FROM task_bindings WHERE external_id=?1",
                [external_id],
            )? != 1
            {
                return Err(RuntimeError::not_found("Working task not found"));
            }
            conn.execute("DELETE FROM working_tasks WHERE task_id=?1", [external_id])?;
            Ok(())
        })
    }
    pub fn append_turn(&mut self, id: &str, files: &[String]) -> Result<i64> {
        let id = validate_task_id(id)?;
        validate_values(files)?;
        guard_shared_text("Turn paths", files)?;
        self.transaction(|conn| {
            conn.execute(
                "INSERT INTO turn_deltas(task_id,created_at,files) VALUES(?1,?2,?3)",
                params![id, utc_now(), serde_json::to_string(files)?],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }
    pub fn pending_turns(&self, id: &str) -> Result<Vec<TurnDelta>> {
        let mut statement = self.conn.prepare(
            "SELECT id,task_id,created_at,files FROM turn_deltas WHERE task_id=?1 ORDER BY id",
        )?;
        let rows = statement.query_map([id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (id, task_id, created_at, files) = row?;
            Ok(TurnDelta {
                id,
                task_id,
                created_at,
                files: serde_json::from_str(&files)?,
            })
        })
        .collect()
    }
    pub fn recent_touched(&self, id: &str, limit: usize) -> Result<Vec<String>> {
        let mut statement = self
            .conn
            .prepare("SELECT files FROM turn_deltas WHERE task_id=?1 ORDER BY id DESC LIMIT 5")?;
        let mut output = Vec::new();
        for row in statement.query_map([id], |row| row.get::<_, String>(0))? {
            if let Ok(files) = serde_json::from_str::<Vec<String>>(&row?) {
                for file in files.into_iter().rev() {
                    if !output.contains(&file) {
                        output.push(file);
                    }
                    if output.len() >= limit {
                        return Ok(output);
                    }
                }
            }
        }
        Ok(output)
    }
    pub fn flush_turns(
        &mut self,
        context: &RuntimeContext,
        id: &str,
        file_limit: usize,
    ) -> Result<Option<Value>> {
        crate::checkpoint::flush(self, context, id, file_limit)
    }
}

fn initialize(conn: &Connection, require_write: bool) -> Result<()> {
    let docs = columns(conn, "documents")?;
    let stale = schema(conn, "documents")?.is_some_and(|schema| !schema.contains(TOKENIZER));
    if !docs.is_empty() && (!docs.contains("layer") || !docs.contains("summary") || stale) {
        conn.execute_batch("DROP TABLE documents; DROP TABLE IF EXISTS document_source_state;")?;
    }
    if schema(conn, "documents")?.is_none() {
        conn.execute_batch("CREATE VIRTUAL TABLE documents USING fts5(path UNINDEXED,layer UNINDEXED,kind UNINDEXED,title,summary,content,tokenize='porter unicode61')")?;
    }
    if schema(conn, "document_links")?.is_none() {
        conn.execute_batch("DROP TABLE IF EXISTS document_source_state")?;
    }
    let meta = columns(conn, "document_metadata")?;
    if !meta.is_empty() && !meta.contains("attestation") {
        conn.execute_batch("DROP TABLE IF EXISTS document_source_state")?;
    }
    let episode = schema(conn, "episodes")?;
    if let Some(schema) = episode {
        let legacy_unindexed =
            regex::Regex::new(r"(?i)\b(?:files|verification|sources)\s+UNINDEXED\b")
                .map_err(|_| RuntimeError::invalid("Invalid legacy episode schema pattern"))?;
        if !schema.contains(TOKENIZER) || legacy_unindexed.is_match(&schema) {
            conn.execute_batch("CREATE TEMP TABLE migrated_episodes AS SELECT rowid AS saved_rowid,summary,outcome,files,verification,sources,created_at FROM episodes; DROP TABLE episodes;")?;
            create_episodes(conn)?;
            conn.execute_batch("INSERT INTO episodes(rowid,summary,outcome,files,verification,sources,created_at) SELECT saved_rowid,summary,outcome,files,verification,sources,created_at FROM migrated_episodes; DROP TABLE migrated_episodes;")?;
        }
    } else {
        create_episodes(conn)?;
    }
    conn.execute_batch("CREATE TABLE IF NOT EXISTS working_tasks(task_id TEXT PRIMARY KEY,goal TEXT NOT NULL,progress TEXT NOT NULL,auto_checkpoint TEXT NOT NULL DEFAULT '',next_steps TEXT NOT NULL,files TEXT NOT NULL,sources TEXT NOT NULL,created_at TEXT NOT NULL,updated_at TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS turn_deltas(id INTEGER PRIMARY KEY AUTOINCREMENT,task_id TEXT NOT NULL,created_at TEXT NOT NULL,files TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS task_bindings(external_id TEXT PRIMARY KEY,task_uuid TEXT NOT NULL UNIQUE,revision INTEGER NOT NULL,updated_at TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS document_metadata(path TEXT PRIMARY KEY,category TEXT NOT NULL,privacy TEXT NOT NULL,owner TEXT NOT NULL,authority TEXT NOT NULL,lifecycle TEXT NOT NULL,source_hash TEXT NOT NULL,record_id TEXT,conflicts TEXT NOT NULL,source_fingerprints TEXT NOT NULL DEFAULT '[]',updated_at TEXT NOT NULL DEFAULT '',confidence REAL NOT NULL DEFAULT 1.0,attestation TEXT NOT NULL DEFAULT '');
        CREATE TABLE IF NOT EXISTS document_source_state(path TEXT PRIMARY KEY,mtime_ns INTEGER NOT NULL,size INTEGER NOT NULL,eligible_until TEXT);
        CREATE TABLE IF NOT EXISTS document_links(path TEXT NOT NULL,ref_path TEXT NOT NULL,ref_kind TEXT NOT NULL,PRIMARY KEY(path,ref_path,ref_kind));
        CREATE TABLE IF NOT EXISTS index_state(key TEXT PRIMARY KEY,value TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS identifier_terms(path TEXT NOT NULL,identifier TEXT NOT NULL,expansion TEXT NOT NULL,PRIMARY KEY(path,identifier,expansion));
        CREATE INDEX IF NOT EXISTS document_links_ref ON document_links(ref_path);
        CREATE INDEX IF NOT EXISTS document_links_source_path ON document_links(ref_path,ref_kind,path);
        CREATE INDEX IF NOT EXISTS identifier_terms_expansion ON identifier_terms(expansion);")?;
    if !columns(conn, "working_tasks")?.contains("auto_checkpoint") {
        conn.execute_batch(
            "ALTER TABLE working_tasks ADD COLUMN auto_checkpoint TEXT NOT NULL DEFAULT ''",
        )?;
    }
    for (name, definition) in [
        ("source_fingerprints", "TEXT NOT NULL DEFAULT '[]'"),
        ("updated_at", "TEXT NOT NULL DEFAULT ''"),
        ("confidence", "REAL NOT NULL DEFAULT 1.0"),
        ("attestation", "TEXT NOT NULL DEFAULT ''"),
    ] {
        if !columns(conn, "document_metadata")?.contains(name) {
            conn.execute_batch(&format!(
                "ALTER TABLE document_metadata ADD COLUMN {name} {definition}"
            ))?;
        }
    }
    if !columns(conn, "document_source_state")?.contains("eligible_until") {
        conn.execute_batch("ALTER TABLE document_source_state ADD COLUMN eligible_until TEXT")?;
    }
    conn.execute_batch("CREATE INDEX IF NOT EXISTS document_metadata_conflicts ON document_metadata(conflicts) WHERE conflicts != '[]'")?;
    writable(conn, require_write)
}
fn writable(conn: &Connection, require_write: bool) -> Result<()> {
    if require_write {
        conn.execute("UPDATE index_state SET value=value WHERE 0", [])?;
    }
    Ok(())
}
/// Index state naming the source whose `initialize` last ran on a store and
/// the schema cookie it left: any later schema change moves the cookie.
const SCHEMA_MARK: &str = "store-schema";
fn schema_mark(conn: &Connection) -> Result<String> {
    static SOURCE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let cookie = conn.query_row("PRAGMA schema_version", [], |row| row.get::<_, i64>(0))?;
    let source = SOURCE.get_or_init(|| crate::query::sha256(include_str!("storage.rs").as_bytes()));
    Ok(format!("{source}:{cookie}"))
}
fn schema_current(conn: &Connection) -> bool {
    let Ok(expected) = schema_mark(conn) else {
        return false;
    };
    conn.query_row(
        "SELECT value FROM index_state WHERE key=?1",
        [SCHEMA_MARK],
        |row| row.get::<_, String>(0),
    )
    .is_ok_and(|kept| kept == expected)
}
fn mark_schema(conn: &Connection) -> Result<()> {
    let mark = schema_mark(conn)?;
    store_state(conn, &BTreeMap::from([(SCHEMA_MARK.to_string(), mark)]))
}
fn create_episodes(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE VIRTUAL TABLE episodes USING fts5(summary,outcome,files,verification,sources,created_at UNINDEXED,tokenize='porter unicode61')")?;
    Ok(())
}
pub(crate) fn load_state(conn: &Connection) -> Result<BTreeMap<String, String>> {
    let mut statement = conn.prepare("SELECT key,value FROM index_state")?;
    Ok(statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?)
}
pub(crate) fn store_state(conn: &Connection, values: &BTreeMap<String, String>) -> Result<()> {
    for (key, value) in values {
        conn.execute("INSERT INTO index_state(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value])?;
    }
    Ok(())
}
fn working(conn: &Connection, id: &str) -> Result<Option<WorkingTask>> {
    let row=conn.query_row("SELECT task_id,goal,progress,auto_checkpoint,next_steps,files,sources,created_at,updated_at FROM working_tasks WHERE task_id=?1",[id],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?,row.get::<_,String>(5)?,row.get::<_,String>(6)?,row.get::<_,String>(7)?,row.get::<_,String>(8)?))).optional()?;
    row.map(
        |(
            task_id,
            goal,
            progress,
            auto_checkpoint,
            next_steps,
            files,
            sources,
            created_at,
            updated_at,
        )| {
            let task = WorkingTask {
                task_id,
                goal,
                progress,
                auto_checkpoint,
                next_steps: serde_json::from_str(&next_steps)?,
                files: serde_json::from_str(&files)?,
                sources: serde_json::from_str(&sources)?,
                created_at,
                updated_at,
            };
            // Validate the complete persisted state without substituting the
            // bounded capsule projection for the operator's original fields.
            crate::capsule::project_working(
                Some(serde_json::to_value(&task)?),
                crate::capsule::DeliveryProfile::Ordinary,
            )?;
            Ok(task)
        },
    )
    .transpose()
}
pub(crate) fn insert_episode(conn: &Connection, input: &EpisodeInput) -> Result<i64> {
    if input.summary.trim().is_empty() || input.outcome.trim().is_empty() {
        return Err(RuntimeError::invalid(
            "Episode summary and outcome must not be empty",
        ));
    }
    validate_values(&input.files)?;
    guard_shared_text(
        "Episode",
        &[
            vec![input.summary.clone(), input.outcome.clone()],
            input.files.clone(),
            input.verification.clone(),
            input.sources.clone(),
        ]
        .concat(),
    )?;
    conn.execute("INSERT INTO episodes(summary,outcome,files,verification,sources,created_at) VALUES(?1,?2,?3,?4,?5,?6)",params![input.summary.trim(),input.outcome.trim(),serde_json::to_string(&input.files)?,serde_json::to_string(&input.verification)?,serde_json::to_string(&input.sources)?,utc_now()])?;
    Ok(conn.last_insert_rowid())
}
pub fn validate_task_id(id: &str) -> Result<String> {
    let id = id.trim();
    let mut chars = id.chars();
    if !chars.next().is_some_and(char::is_alphanumeric)
        || id.chars().count() > 128
        || !id.chars().all(|c| {
            c.is_alphanumeric()
                || c == '_'
                || ('\u{300}'..='\u{36f}').contains(&c)
                || ".//#+-".contains(c)
        })
    {
        return Err(RuntimeError::invalid(
            "Task ID must use letters, digits, '.', '_', '/', '#', '+' or '-'",
        ));
    }
    guard_shared_text("Working task", &[id.into()])?;
    Ok(id.into())
}
pub fn merge_recent(existing: &[String], additions: &[String]) -> Vec<String> {
    let additions = unique_strings(additions.iter().cloned());
    let mut merged = existing
        .iter()
        .filter(|value| !additions.contains(value))
        .cloned()
        .collect::<Vec<_>>();
    merged.extend(additions);
    merged
}
fn validate_values(values: &[String]) -> Result<()> {
    if values
        .iter()
        .any(|value| value.is_empty() || value.contains('\0'))
    {
        return Err(RuntimeError::invalid(
            "Path must not be empty or contain NUL",
        ));
    }
    Ok(())
}
pub fn derive_goal(id: &str) -> String {
    if id.starts_with("codex-thread/")
        && id
            .strip_prefix("codex-thread/")
            .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
    {
        return format!(
            "Work of the Codex thread that owns its worktree (auto-provisioned from {id})"
        );
    }
    let mut label = id.to_string();
    for prefix in [
        "feature/",
        "feat/",
        "fix/",
        "bugfix/",
        "hotfix/",
        "chore/",
        "release/",
        "refactor/",
        "docs/",
        "test/",
        "merge/",
    ] {
        if label.to_lowercase().starts_with(prefix) {
            label = label[prefix.len()..].into();
            break;
        }
    }
    let tickets = regex::Regex::new(r"[A-Za-z][A-Za-z0-9]*-[0-9]+")
        .map(|pattern| {
            pattern
                .find_iter(&label)
                .map(|found| found.as_str().to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for (index, ticket) in tickets.iter().enumerate() {
        label = label.replacen(ticket, &format!("\0{index}\0"), 1);
    }
    label = label
        .split(['-', '_', '/'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    for (index, ticket) in tickets.iter().enumerate() {
        label = label.replace(&format!("\0{index}\0"), ticket);
    }
    let mut characters = label.chars();
    let label = characters
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + characters.as_str())
        .unwrap_or_else(|| id.into());
    format!("{label} (auto-provisioned from {id})")
}

#[cfg(test)]
mod schema_mark_tests {
    use super::*;
    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn database() -> (Directory, PathBuf) {
        let root = std::env::temp_dir().join(format!("infra-schema-mark-{}", uuid4()));
        std::fs::create_dir_all(&root).unwrap();
        (Directory(root.clone()), root.join("context.db"))
    }
    fn kept(store: &Store) -> Option<String> {
        store.state().unwrap().get(SCHEMA_MARK).cloned()
    }
    #[test]
    fn a_marked_store_reopens_without_migrating_until_its_schema_changes() {
        let (_temp, path) = database();
        let store = Store::open(&path, OpenOptions::refresh(None)).unwrap();
        let mark = kept(&store).expect("a fresh store is marked");
        assert!(schema_current(&store.conn));
        drop(store);
        let store = Store::open(&path, OpenOptions::refresh(None)).unwrap();
        assert_eq!(kept(&store), Some(mark.clone()));
        // A schema change made by anything else moves the cookie, so the next
        // open migrates again and restores what was removed.
        store
            .conn
            .execute_batch("DROP INDEX document_links_ref")
            .unwrap();
        assert!(!schema_current(&store.conn));
        drop(store);
        let store = Store::open(&path, OpenOptions::refresh(None)).unwrap();
        let restored: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name='document_links_ref'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(restored, 1);
        assert!(schema_current(&store.conn));
        assert_ne!(kept(&store), Some(mark));
    }
    #[test]
    fn a_mark_left_by_another_source_migrates_again() {
        let (_temp, path) = database();
        let mut store = Store::open(&path, OpenOptions::default()).unwrap();
        let cookie: i64 = store
            .conn
            .query_row("PRAGMA schema_version", [], |row| row.get(0))
            .unwrap();
        store
            .put_state(&BTreeMap::from([(
                SCHEMA_MARK.to_string(),
                format!("another source:{cookie}"),
            )]))
            .unwrap();
        assert!(!schema_current(&store.conn));
        drop(store);
        let store = Store::open(&path, OpenOptions::default()).unwrap();
        assert!(schema_current(&store.conn));
    }
}
