use crate::{HarnessError, Result};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

pub const DEFAULT_EVENT_RING_CAPACITY: usize = 1024;
pub const DEFAULT_EVENT_PAGE_LIMIT: usize = 250;
pub const MAX_EVENT_PAGE_LIMIT: usize = 2000;

/// Typed real-time SSE frame emitted by backend mutations.
#[derive(Clone, Debug)]
pub struct StreamFrame {
    pub seq: u64,
    pub event: &'static str,
    pub session_id: Option<String>,
    pub project_id: Option<String>,
    pub data: Value,
}

/// Replay/subscription result when connecting to the SSE stream.
pub enum ReplayStart {
    Frames {
        frames: Vec<StreamFrame>,
        rx: tokio::sync::broadcast::Receiver<StreamFrame>,
    },
    Resync {
        oldest_id: u64,
        latest_id: u64,
        reason: &'static str,
        rx: tokio::sync::broadcast::Receiver<StreamFrame>,
    },
}

struct StreamRing {
    next_seq: u64,
    capacity: usize,
    frames: VecDeque<StreamFrame>,
}

/// Bounded sequence-numbered replay ring + live broadcast hub for SSE clients.
pub struct StreamHub {
    inner: Mutex<StreamRing>,
    tx: tokio::sync::broadcast::Sender<StreamFrame>,
    graph_bridge_started: std::sync::atomic::AtomicBool,
}

impl StreamHub {
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(16);
        let (tx, _) = tokio::sync::broadcast::channel(cap.max(1024));
        Self {
            inner: Mutex::new(StreamRing {
                next_seq: 1,
                capacity: cap,
                frames: VecDeque::with_capacity(cap),
            }),
            tx,
            graph_bridge_started: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn claim_graph_bridge(&self) -> bool {
        !self
            .graph_bridge_started
            .swap(true, std::sync::atomic::Ordering::AcqRel)
    }

    pub fn publish(
        &self,
        event: &'static str,
        session_id: Option<String>,
        project_id: Option<String>,
        data: Value,
    ) -> u64 {
        let frame = {
            let Ok(mut guard) = self.inner.lock() else {
                return 0;
            };
            let seq = guard.next_seq;
            guard.next_seq = seq.saturating_add(1);
            let frame = StreamFrame {
                seq,
                event,
                session_id,
                project_id,
                data,
            };
            guard.frames.push_back(frame.clone());
            while guard.frames.len() > guard.capacity {
                guard.frames.pop_front();
            }
            frame
        };
        let seq = frame.seq;
        let _ = self.tx.send(frame);
        seq
    }

    pub fn subscribe(&self, last_event_id: Option<u64>) -> ReplayStart {
        let Ok(guard) = self.inner.lock() else {
            return ReplayStart::Frames {
                frames: Vec::new(),
                rx: self.tx.subscribe(),
            };
        };
        // Subscribe inside the ring lock so no frame can land between the
        // replay snapshot and the broadcast receiver cursor.
        let rx = self.tx.subscribe();
        let latest_id = guard.next_seq.saturating_sub(1);
        let Some(cursor) = last_event_id.filter(|&id| id > 0) else {
            return ReplayStart::Frames {
                frames: Vec::new(),
                rx,
            };
        };
        if cursor > latest_id {
            let oldest_id = guard
                .frames
                .front()
                .map_or(latest_id.saturating_add(1), |f| f.seq);
            return ReplayStart::Resync {
                oldest_id,
                latest_id,
                reason: "server_restarted",
                rx,
            };
        }
        if let Some(front) = guard.frames.front()
            && cursor < front.seq.saturating_sub(1)
        {
            return ReplayStart::Resync {
                oldest_id: front.seq,
                latest_id,
                reason: "cursor_expired",
                rx,
            };
        }
        if guard.frames.is_empty() && cursor < latest_id {
            return ReplayStart::Resync {
                oldest_id: latest_id.saturating_add(1),
                latest_id,
                reason: "cursor_expired",
                rx,
            };
        }
        let frames = guard
            .frames
            .iter()
            .filter(|f| f.seq > cursor)
            .cloned()
            .collect();
        ReplayStart::Frames { frames, rx }
    }
}

/// Paginated session events window returned by `SessionStore::session_events_page`.
pub struct EventPage {
    pub session: Value,
    pub events: Vec<Value>,
    pub has_more: bool,
    pub has_older: bool,
    pub total_events: usize,
    pub oldest_event_id: Option<i64>,
    pub latest_event_id: Option<i64>,
}

type WaitingCounter = Arc<dyn Fn(&str) -> usize + Send + Sync>;

#[derive(Clone)]
pub struct SessionStore {
    pub state: PathBuf,
    db: Arc<Mutex<Connection>>,
    _runner: Arc<File>,
    stream: Arc<StreamHub>,
    waiting_counter: Arc<Mutex<Option<WaitingCounter>>>,
}
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
pub fn hexid() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}
pub fn sha(bytes: &[u8]) -> String {
    infra_core::query::sha256(bytes)
}
pub fn exact(value: &Value, required: &[&str], optional: &[&str]) -> bool {
    value.as_object().is_some_and(|m| {
        required.iter().all(|k| m.contains_key(*k))
            && m.keys()
                .all(|k| required.contains(&k.as_str()) || optional.contains(&k.as_str()))
    })
}
pub fn text(value: &Value, name: &str, max: usize) -> Result<String> {
    let s = value
        .as_str()
        .ok_or_else(|| HarnessError::invalid(format!("{name} must be text.")))?;
    if s.trim().is_empty() || s.chars().count() > max || s.contains('\0') {
        return Err(HarnessError::invalid(format!("Invalid {name}.")));
    }
    Ok(s.into())
}
fn generation_event_pair(generation: &str, event: &Value) -> Result<(Value, String)> {
    let mut event = event.clone();
    if !event.is_object() {
        return Err(HarnessError::invalid(
            "A generation event must be an object.",
        ));
    }
    if event
        .get("launch_id")
        .is_some_and(|v| v.as_str() != Some(generation))
    {
        return Err(HarnessError::conflict(
            "A generation event has a different launch identity.",
        ));
    }
    event["launch_id"] = generation.into();
    if event.get("at").is_none() {
        event["at"] = chrono::Utc::now()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
            .into();
    }
    let raw = event.to_string();
    if raw.len() > 4 * 1024 * 1024 {
        return Err(HarnessError::invalid("Event byte limit exceeded."));
    }
    Ok((event, raw))
}
fn confined(state: &Path, name: &str) -> Result<PathBuf> {
    let path = state.join(name);
    infra_core::fs::admit_descendant(state, &path)?;
    Ok(path)
}
impl SessionStore {
    pub fn open(state: &Path) -> Result<Self> {
        infra_host::util::private_directory(state)?;
        let state = infra_core::fs::python_realpath(state)?;
        let lockpath = confined(&state, "runner.lock")?;
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0).custom_flags(0x00200000);
        }
        let runner = options.open(lockpath)?;
        if !runner.metadata()?.is_file() {
            return Err(HarnessError::forbidden(
                "Runner lock must be a regular file.",
            ));
        }
        #[cfg(unix)]
        runner.try_lock().map_err(|_| {
            HarnessError::conflict("Another runner still owns this state directory.")
        })?;
        let database = confined(&state, "sessions.sqlite3")?;
        let db = Connection::open(database)?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.execute_batch("PRAGMA journal_mode=WAL;PRAGMA foreign_keys=ON;
   CREATE TABLE IF NOT EXISTS sessions(
    id TEXT PRIMARY KEY,title TEXT NOT NULL,project_id TEXT NOT NULL,project_path TEXT NOT NULL,
    provider TEXT NOT NULL,model TEXT,mode TEXT NOT NULL,workflow TEXT NOT NULL,project_context INTEGER NOT NULL,
    status TEXT NOT NULL,native_session_id TEXT,created_at TEXT NOT NULL,updated_at TEXT NOT NULL);
   CREATE TABLE IF NOT EXISTS events(id INTEGER PRIMARY KEY AUTOINCREMENT,session_id TEXT NOT NULL,data TEXT NOT NULL);
   CREATE INDEX IF NOT EXISTS events_session ON events(session_id,id);
   CREATE TABLE IF NOT EXISTS session_merges(session_id TEXT PRIMARY KEY,request_id TEXT UNIQUE NOT NULL,request_hash TEXT NOT NULL,bundle TEXT NOT NULL,summary TEXT NOT NULL,context TEXT NOT NULL);
   CREATE TABLE IF NOT EXISTS registered_projects(id TEXT PRIMARY KEY,path TEXT NOT NULL UNIQUE);
   CREATE TABLE IF NOT EXISTS memory_result_attempts(session_id TEXT NOT NULL,generation TEXT NOT NULL,binding TEXT NOT NULL,PRIMARY KEY(session_id,generation));
   CREATE TABLE IF NOT EXISTS attachments(id TEXT PRIMARY KEY,session_id TEXT NOT NULL,name TEXT NOT NULL,size INTEGER NOT NULL,digest TEXT NOT NULL);
   CREATE TABLE IF NOT EXISTS harness_settings(name TEXT PRIMARY KEY,value TEXT NOT NULL);
   CREATE TABLE IF NOT EXISTS native_fleet_runs(thread_id TEXT PRIMARY KEY,state TEXT NOT NULL,stage TEXT NOT NULL,decision INTEGER,import_id TEXT,revision INTEGER NOT NULL DEFAULT 1);
   CREATE TABLE IF NOT EXISTS native_fleet_tasks(thread_id TEXT NOT NULL,task_id TEXT NOT NULL,lens TEXT NOT NULL,ordinal INTEGER NOT NULL,status TEXT NOT NULL,input TEXT NOT NULL,output TEXT,error TEXT,PRIMARY KEY(thread_id,task_id));
   CREATE TABLE IF NOT EXISTS native_checkpoint_imports(import_id TEXT PRIMARY KEY,thread_id TEXT NOT NULL,checkpoint_id TEXT NOT NULL,source_digest TEXT NOT NULL,created_at TEXT NOT NULL);
   CREATE TABLE IF NOT EXISTS harness_generations(session_id TEXT PRIMARY KEY,generation TEXT NOT NULL);")?;
        let columns = {
            let mut q = db.prepare("PRAGMA table_info(sessions)")?;
            q.query_map([], |r| r.get::<_, String>(1))?
                .collect::<std::result::Result<BTreeSet<_>, _>>()?
        };
        for (name, definition) in [
            ("agents_enabled", "INTEGER NOT NULL DEFAULT 0"),
            ("agent_count", "INTEGER NOT NULL DEFAULT 3"),
            ("thinking_effort", "TEXT"),
            ("workspace", "TEXT NOT NULL DEFAULT 'project'"),
            ("branch", "TEXT"),
            ("git_common_dir", "TEXT"),
            ("fleet", "TEXT"),
            ("fleet_result", "TEXT"),
            ("fleet_action", "TEXT"),
            ("brain", "TEXT"),
            ("creator", "TEXT"),
            ("system_run", "TEXT"),
            ("system_discovery", "TEXT"),
            ("system_explain", "TEXT"),
            ("budgets", "TEXT"),
            ("budget_usage", "TEXT"),
            ("budget_revision", "INTEGER NOT NULL DEFAULT 0"),
            ("result_base", "TEXT"),
            ("sdd", "TEXT"),
            ("model_routing", "TEXT"),
            ("clash", "TEXT"),
            ("clash_result", "TEXT"),
            ("cost_totals", "TEXT"),
            ("omnigraph", "TEXT"),
            ("permissions", "TEXT"),
            ("skillcache", "TEXT"),
        ] {
            if !columns.contains(name) {
                db.execute_batch(&format!(
                    "ALTER TABLE sessions ADD COLUMN {name} {definition}"
                ))?;
            }
        }
        crate::inbox::initialize(&db)?;
        crate::skillcache::db::initialize(&db)?;
        let restart = db.unchecked_transaction()?;
        let interrupted = {
            let mut q =
                restart.prepare("SELECT id FROM sessions WHERE status IN ('queued','running')")?;
            q.query_map([], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        for sid in interrupted {
            restart.execute("INSERT INTO events(session_id,data)VALUES(?1,?2)",params![sid,json!({"kind":"status","text":"Server restarted; the previous run was interrupted."}).to_string()])?;
        }
        restart.execute("UPDATE sessions SET status='interrupted',updated_at=?1 WHERE status IN ('queued','running')",[now()])?;
        expire_pending_permissions(&restart)?;
        crate::inbox::recover(&restart)?;
        crate::skillcache::db::recover(&restart)?;
        restart.commit()?;
        // Files of SkillCache that no row owns any more: snapshots of traces
        // that are gone, and the folders of compilations a restart interrupted.
        if let Ok(kept) = crate::skillcache::db::trace_generations(&db) {
            crate::skillcache::snapshot::collect(&state, &kept);
        }
        crate::skillcache::compile::clear_workspaces(&state);
        crate::skillcache::warm::clear_sessions(&state);
        Ok(Self {
            state,
            db: Arc::new(Mutex::new(db)),
            _runner: Arc::new(runner),
            stream: Arc::new(StreamHub::new(DEFAULT_EVENT_RING_CAPACITY)),
            waiting_counter: Arc::new(Mutex::new(None)),
        })
    }
    /// Shared real-time SSE stream hub for this store.
    pub fn stream(&self) -> &Arc<StreamHub> {
        &self.stream
    }
    /// Registers a waiting-permission count provider used when enriching
    /// emitted `session_status`, `system_run`, and `creator_run` SSE frames.
    pub fn set_waiting_counter(&self, counter: WaitingCounter) {
        if let Ok(mut slot) = self.waiting_counter.lock() {
            *slot = Some(counter);
        }
    }
    /// Current waiting-permission count for `sid`, if a provider is installed.
    pub fn waiting_count(&self, sid: &str) -> usize {
        self.waiting_counter
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().cloned())
            .map_or(0, |counter| counter(sid))
    }
    /// Publishes an arbitrary typed frame on the SSE stream.
    pub fn publish_event(
        &self,
        event: &'static str,
        session_id: Option<String>,
        project_id: Option<String>,
        data: Value,
    ) -> u64 {
        self.stream.publish(event, session_id, project_id, data)
    }
    /// Emits `session_status` (plus linked `system_run` / `creator_run`) for `sid`.
    pub fn emit_session_status(&self, sid: &str) {
        if let Ok(session) = self.get(sid) {
            self.emit_session_status_frame(session);
        }
    }
    /// Emits `waiting_counts` plus updated `session_status` / run summaries for
    /// `sid` after pending permissions change.
    pub fn emit_waiting_change(&self, sid: &str, counts: BTreeMap<String, usize>) {
        let total: usize = counts.values().sum();
        let project_id = self.session_project_id(sid);
        self.stream.publish(
            "waiting_counts",
            Some(sid.to_owned()),
            project_id,
            json!({
                "counts": counts,
                "total": total,
                "session_id": sid,
            }),
        );
        if let Ok(session) = self.get(sid) {
            self.emit_session_status_frame(session);
        }
    }
    pub fn session_project_id(&self, sid: &str) -> Option<String> {
        let db = self.connection().ok()?;
        db.query_row("SELECT project_id FROM sessions WHERE id=?1", [sid], |r| {
            r.get::<_, String>(0)
        })
        .optional()
        .ok()
        .flatten()
    }
    fn emit_session_event_frame(
        &self,
        sid: &str,
        project_id: Option<String>,
        event_id: i64,
        mut event: Value,
    ) {
        event["id"] = json!(event_id);
        let project_id = project_id.or_else(|| self.session_project_id(sid));
        self.stream.publish(
            "session_event",
            Some(sid.to_owned()),
            project_id.clone(),
            json!({
                "session_id": sid,
                "project_id": project_id,
                "id": event_id,
                "event": event,
            }),
        );
    }
    fn emit_session_status_frame(&self, mut session: Value) {
        let sid = session["id"].as_str().unwrap_or("").to_owned();
        if sid.is_empty() {
            return;
        }
        let project_id = session["project_id"].as_str().map(str::to_owned);
        let waiting = self.waiting_count(&sid);
        session["waiting_permissions"] = json!(waiting);
        let status = session["status"].as_str().unwrap_or("").to_owned();
        let active = matches!(status.as_str(), "queued" | "running");
        if let Some(run_id) = session["creator"]["run_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
        {
            let run = self
                .creator_run_record(&run_id)
                .ok()
                .flatten()
                .map(|mut rec| {
                    if !active
                        && matches!(
                            rec["status"].as_str().unwrap_or(""),
                            "scanning" | "generating" | "applying" | "rolling_back"
                        )
                    {
                        rec["session_status"] = json!(status);
                    }
                    rec
                });
            let revision = run.as_ref().and_then(|r| r["revision"].as_u64());
            let mut payload = json!({
                "run_id": run_id,
                "id": run_id,
                "project_id": project_id,
                "status": run.as_ref().and_then(|r| r["status"].as_str()).unwrap_or(status.as_str()),
                "active": active,
                "waiting_permissions": waiting,
                "session_id": sid,
            });
            if let Some(rev) = revision {
                payload["revision"] = json!(rev);
            }
            if let Some(run) = run {
                payload["run"] = run;
            }
            self.stream.publish(
                "creator_run",
                Some(sid.clone()),
                project_id.clone(),
                payload,
            );
        }
        if let Some(run_id) = session["system_run"]["run_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
        {
            let run = self.system_run_record(&run_id).ok().flatten();
            let mut payload = json!({
                "run_id": run_id,
                "id": run_id,
                "project_id": project_id,
                "status": status,
                "active": active,
                "waiting_permissions": waiting,
                "session_id": sid,
            });
            if let Some(run) = run {
                payload["run"] = run;
            }
            self.stream
                .publish("system_run", Some(sid.clone()), project_id.clone(), payload);
        }
        self.stream.publish(
            "session_status",
            Some(sid),
            project_id,
            json!({ "session": session }),
        );
    }
    /// Reads one stored creator run record if `creator_runs` exists.
    pub fn creator_run_record(&self, run_id: &str) -> Result<Option<Value>> {
        let db = self.connection()?;
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='creator_runs')",
            [],
            |r| r.get(0),
        )?;
        if !exists {
            return Ok(None);
        }
        let raw: Option<String> = db
            .query_row("SELECT data FROM creator_runs WHERE id=?1", [run_id], |r| {
                r.get(0)
            })
            .optional()?;
        raw.map(|s| crate::strict_json::parse(s.as_bytes(), 2 * 1024 * 1024))
            .transpose()
    }
    /// Reads one stored system run record if `system_runs` exists.
    pub fn system_run_record(&self, run_id: &str) -> Result<Option<Value>> {
        let db = self.connection()?;
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='system_runs')",
            [],
            |r| r.get(0),
        )?;
        if !exists {
            return Ok(None);
        }
        let raw: Option<String> = db
            .query_row("SELECT data FROM system_runs WHERE id=?1", [run_id], |r| {
                r.get(0)
            })
            .optional()?;
        raw.map(|s| crate::strict_json::parse(s.as_bytes(), 4 * 1024 * 1024))
            .transpose()
    }
    /// Reads one stored results check record if `checks` exists.
    pub fn get_check(&self, check_id: &str) -> Result<Option<Value>> {
        let db = self.connection()?;
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='checks')",
            [],
            |r| r.get(0),
        )?;
        if !exists {
            return Ok(None);
        }
        let raw: Option<String> = db
            .query_row("SELECT data FROM checks WHERE id=?1", [check_id], |r| {
                r.get(0)
            })
            .optional()?;
        raw.map(|s| crate::strict_json::parse(s.as_bytes(), 4 * 1024 * 1024))
            .transpose()
    }
    pub fn runner_lock(&self) -> Arc<File> {
        self._runner.clone()
    }
    pub fn connection(&self) -> Result<MutexGuard<'_, Connection>> {
        self.db
            .lock()
            .map_err(|_| HarnessError::unavailable("Session store lock was poisoned."))
    }
    pub fn setting(&self, name: &str) -> Result<Option<Value>> {
        let db = self.connection()?;
        let raw: Option<String> = db
            .query_row(
                "SELECT value FROM harness_settings WHERE name=?1",
                [name],
                |r| r.get(0),
            )
            .optional()?;
        raw.map(|s| crate::strict_json::parse(s.as_bytes(), 16 * 1024 * 1024))
            .transpose()
    }
    pub fn set_setting(&self, name: &str, value: &Value) -> Result<()> {
        self.connection()?.execute("INSERT INTO harness_settings(name,value)VALUES(?1,?2)ON CONFLICT(name)DO UPDATE SET value=excluded.value",params![name,value.to_string()])?;
        Ok(())
    }
    pub fn register(&self, path: &Path) -> Result<Value> {
        std::fs::metadata(path)?;
        let root = infra_core::fs::python_realpath(path)?;
        if !root.is_dir() {
            return Err(HarnessError::invalid(
                "Each project must be an existing directory.",
            ));
        }
        let path = root
            .to_str()
            .ok_or_else(|| HarnessError::invalid("Project path is not UTF-8."))?;
        let identity = if cfg!(windows) {
            path.replace('/', "\\").to_lowercase()
        } else {
            path.to_owned()
        };
        let key = sha(identity.as_bytes())[..16].to_string();
        self.connection()?.execute("INSERT INTO registered_projects(id,path)VALUES(?1,?2)ON CONFLICT(id)DO UPDATE SET path=excluded.path",params![key,path])?;
        Ok(
            json!({"id":key,"name":root.file_name().unwrap_or_default().to_string_lossy(),"path":path}),
        )
    }
    pub fn projects(&self) -> Result<Vec<Value>> {
        let db = self.connection()?;
        let mut q = db.prepare("SELECT id,path FROM registered_projects ORDER BY rowid")?;
        let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (key, path) = row?;
            out.push(json!({"id":key,"name":Path::new(&path).file_name().unwrap_or_default().to_string_lossy(),"path":path}));
        }
        Ok(out)
    }
    pub fn project(&self, key: &str) -> Result<Value> {
        self.projects()?
            .into_iter()
            .find(|v| v["id"] == key)
            .ok_or_else(|| HarnessError::missing("Project not found."))
    }
    pub fn get(&self, sid: &str) -> Result<Value> {
        let db = self.connection()?;
        get_session(&db, sid)
    }
    pub fn summaries(&self) -> Result<Vec<Value>> {
        let db = self.connection()?;
        let mut q = db.prepare(
            "SELECT s.*,m.summary FROM sessions s LEFT JOIN session_merges m ON m.session_id=s.id ORDER BY s.updated_at DESC,s.rowid DESC LIMIT 200",
        )?;
        let names = session_columns(&q);
        let mut rows = q.query([])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(decode_session(row, &names)?);
        }
        Ok(out)
    }
    pub fn session_events(&self, sid: &str, after: i64) -> Result<(Value, Vec<Value>)> {
        let db = self.connection()?;
        // The HTTP route has always validated the session before rejecting a
        // negative cursor. Standalone `events` keeps its cursor-first order.
        let session = get_session(&db, sid)?;
        let events = session_events(&db, sid, after)?;
        Ok((session, events))
    }
    /// Reads one paginated window of session events without breaking the
    /// fixed-250 behavior of [`SessionStore::session_events`].
    pub fn session_events_page(
        &self,
        sid: &str,
        after: Option<i64>,
        before: Option<i64>,
        limit: usize,
        tail: bool,
    ) -> Result<EventPage> {
        let limit = limit.clamp(1, MAX_EVENT_PAGE_LIMIT);
        let db = self.connection()?;
        let session = get_session(&db, sid)?;
        if after.is_some_and(|v| v < 0) || before.is_some_and(|v| v <= 0) {
            return Err(HarnessError::invalid("Invalid event cursor."));
        }
        if after.is_some() && before.is_some() {
            return Err(HarnessError::invalid(
                "Pass either 'after' or 'before', not both.",
            ));
        }
        let (total_i64, oldest_event_id, latest_event_id): (i64, Option<i64>, Option<i64>) = db
            .query_row(
                "SELECT count(*), min(id), max(id) FROM events WHERE session_id=?1",
                [sid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
        let total_events = usize::try_from(total_i64.max(0)).unwrap_or(0);
        let fetch_limit = i64::try_from(limit.saturating_add(1)).unwrap_or(i64::MAX);
        if let Some(before_id) = before {
            let mut events = decode_event_rows(
                &db,
                "SELECT id,data FROM events WHERE session_id=?1 AND id<?2 ORDER BY id DESC LIMIT ?3",
                params![sid, before_id, fetch_limit],
            )?;
            let has_older = events.len() > limit;
            if has_older {
                events.truncate(limit);
            }
            events.reverse();
            let has_more = events
                .last()
                .and_then(|e| e["id"].as_i64())
                .is_some_and(|last| latest_event_id.is_some_and(|max_id| last < max_id));
            return Ok(EventPage {
                session,
                events,
                has_more,
                has_older,
                total_events,
                oldest_event_id,
                latest_event_id,
            });
        }
        if tail && after.is_none() {
            let mut events = decode_event_rows(
                &db,
                "SELECT id,data FROM events WHERE session_id=?1 ORDER BY id DESC LIMIT ?2",
                params![sid, fetch_limit],
            )?;
            let has_older = events.len() > limit;
            if has_older {
                events.truncate(limit);
            }
            events.reverse();
            return Ok(EventPage {
                session,
                events,
                has_more: false,
                has_older,
                total_events,
                oldest_event_id,
                latest_event_id,
            });
        }
        let after_id = after.unwrap_or(0);
        let mut events = decode_event_rows(
            &db,
            "SELECT id,data FROM events WHERE session_id=?1 AND id>?2 ORDER BY id ASC LIMIT ?3",
            params![sid, after_id, fetch_limit],
        )?;
        let has_more = events.len() > limit;
        if has_more {
            events.truncate(limit);
        }
        let has_older = events
            .first()
            .and_then(|e| e["id"].as_i64())
            .is_some_and(|first| oldest_event_id.is_some_and(|min_id| first > min_id));
        Ok(EventPage {
            session,
            events,
            has_more,
            has_older,
            total_events,
            oldest_event_id,
            latest_event_id,
        })
    }
    pub fn events(&self, sid: &str, after: i64) -> Result<Vec<Value>> {
        if after < 0 {
            return Err(HarnessError::invalid("Invalid event cursor."));
        }
        let db = self.connection()?;
        get_session(&db, sid)?;
        session_events(&db, sid, after)
    }
    pub fn event(&self, sid: &str, event: &Value) -> Result<i64> {
        if !event.is_object() {
            return Err(HarnessError::invalid("Event must be an object."));
        }
        let mut event = event.clone();
        if event.get("at").is_none() {
            event["at"] = chrono::Utc::now()
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                .into();
        }
        let raw = event.to_string();
        if raw.len() > 4 * 1024 * 1024 {
            return Err(HarnessError::invalid("Event byte limit exceeded."));
        }
        let (id, project_id) = {
            let db = self.connection()?;
            db.execute(
                "INSERT INTO events(session_id,data)VALUES(?1,?2)",
                params![sid, raw],
            )?;
            let id = db.last_insert_rowid();
            let project_id = db
                .query_row("SELECT project_id FROM sessions WHERE id=?1", [sid], |r| {
                    r.get::<_, String>(0)
                })
                .optional()?
                .filter(|s| !s.is_empty());
            (id, project_id)
        };
        self.emit_session_event_frame(sid, project_id, id, event);
        Ok(id)
    }
    pub fn create(&self, options: &Value, prompt: &str, attachments: &[Value]) -> Result<Value> {
        self.create_transaction(options, prompt, attachments, None)
    }
    pub fn create_uploaded(
        &self,
        options: &Value,
        prompt: &str,
        uploads: &[crate::attachments::Upload],
    ) -> Result<Value> {
        self.create_transaction(options, prompt, &[], Some(uploads))
    }
    fn create_transaction(
        &self,
        options: &Value,
        prompt: &str,
        attachments: &[Value],
        uploads: Option<&[crate::attachments::Upload]>,
    ) -> Result<Value> {
        let project = self.project(
            options["project_id"]
                .as_str()
                .ok_or_else(|| HarnessError::invalid("Select a registered project."))?,
        )?;
        let provider = options["provider"]
            .as_str()
            .ok_or_else(|| HarnessError::invalid("Select a provider."))?;
        if !["claude", "codex", "cursor"].contains(&provider) {
            return Err(HarnessError::invalid("Unknown provider."));
        }
        let prompt = text(&json!(prompt), "prompt", 100_000)?;
        let sid = options["_session_id"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(id);
        let at = now();
        let generation = hexid();
        let mut db = self.connection()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM sessions WHERE status='queued'",
            [],
            |r| r.get(0),
        )?;
        if count >= 16 {
            return Err(HarnessError::conflict("The run queue is full."));
        }
        let (staged, mut upload_guard) = if let Some(uploads) = uploads {
            crate::attachments::stage(&tx, &self.state, &sid, uploads)?
        } else {
            (
                attachments.to_vec(),
                crate::attachments::UploadGuard {
                    folders: vec![],
                    committed: false,
                },
            )
        };
        insert_session(
            &tx,
            options,
            &project,
            &prompt,
            &staged,
            &sid,
            &at,
            &generation,
        )?;
        for key in [
            "creator",
            "system_run",
            "system_discovery",
            "system_explain",
            "omnigraph",
            "skillcache",
        ] {
            if !options[key].is_null() {
                tx.execute(
                    &format!("UPDATE sessions SET {key}=?1 WHERE id=?2"),
                    params![options[key].to_string(), sid],
                )?;
            }
        }
        tx.commit()?;
        upload_guard.commit();
        drop(db);
        let session = self.get(&sid)?;
        if let Ok(initial) = self.events(&sid, 0) {
            let proj = session["project_id"].as_str().map(str::to_owned);
            for mut ev in initial {
                if let Some(eid) = ev["id"].as_i64() {
                    ev.as_object_mut().map(|m| m.remove("id"));
                    self.emit_session_event_frame(&sid, proj.clone(), eid, ev);
                }
            }
        }
        self.emit_session_status_frame(session.clone());
        Ok(session)
    }
    pub fn generation(&self, sid: &str) -> Result<String> {
        self.connection()?
            .query_row(
                "SELECT generation FROM harness_generations WHERE session_id=?1",
                [sid],
                |r| r.get(0),
            )
            .map_err(Into::into)
    }
    pub fn guarded_event(&self, sid: &str, generation: &str, event: &Value) -> Result<bool> {
        Ok(self.guarded_event_id(sid, generation, event)?.is_some())
    }
    pub fn guarded_event_id(
        &self,
        sid: &str,
        generation: &str,
        event: &Value,
    ) -> Result<Option<u64>> {
        let committed = {
            let mut db = self.connection()?;
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let current: Option<String> = tx
                .query_row(
                    "SELECT generation FROM harness_generations WHERE session_id=?1",
                    [sid],
                    |r| r.get(0),
                )
                .optional()?;
            if current.as_deref() != Some(generation) {
                None
            } else {
                let (normalized, raw) = generation_event_pair(generation, event)?;
                tx.execute(
                    "INSERT INTO events(session_id,data)VALUES(?1,?2)",
                    params![sid, raw],
                )?;
                let rowid = tx.last_insert_rowid();
                let project_id = tx
                    .query_row("SELECT project_id FROM sessions WHERE id=?1", [sid], |r| {
                        r.get::<_, String>(0)
                    })
                    .optional()?
                    .filter(|s| !s.is_empty());
                tx.commit()?;
                Some((rowid, normalized, project_id))
            }
        };
        let Some((rowid, normalized, project_id)) = committed else {
            return Ok(None);
        };
        self.emit_session_event_frame(sid, project_id, rowid, normalized);
        Ok(Some(u64::try_from(rowid).unwrap_or(0)))
    }
    pub fn cancel_current(&self, sid: &str) -> Result<Value> {
        enum CancelOutcome {
            NoOp(Value),
            Cancelled {
                updated: Value,
                event_id: i64,
                event: Value,
            },
        }
        let outcome = {
            let mut db = self.connection()?;
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let session = get_session(&tx, sid)?;
            if !["queued", "running"].contains(&session["status"].as_str().unwrap_or("")) {
                crate::inbox::cancel_session(&tx, sid)?;
                tx.commit()?;
                CancelOutcome::NoOp(session)
            } else {
                if session["creator"]["phase"]
                    .as_str()
                    .is_some_and(|phase| ["apply", "rollback"].contains(&phase))
                {
                    return Err(HarnessError::conflict(
                        "Publication is completing verification or rollback.",
                    ));
                }
                let generation: String = tx.query_row(
                    "SELECT generation FROM harness_generations WHERE session_id=?1",
                    [sid],
                    |row| row.get(0),
                )?;
                let (event_val, event) = generation_event_pair(
                    &generation,
                    &json!({"kind":"status","text":"Cancellation requested."}),
                )?;
                tx.execute("UPDATE sessions SET status='cancelled',updated_at=?1 WHERE id=?2 AND status IN ('queued','running')", params![now(),sid])?;
                crate::inbox::cancel_session(&tx, sid)?;
                tx.execute(
                    "INSERT INTO events(session_id,data)VALUES(?1,?2)",
                    params![sid, event],
                )?;
                let event_id = tx.last_insert_rowid();
                let updated = get_session(&tx, sid)?;
                tx.commit()?;
                CancelOutcome::Cancelled {
                    updated,
                    event_id,
                    event: event_val,
                }
            }
        };
        match outcome {
            CancelOutcome::NoOp(session) => Ok(session),
            CancelOutcome::Cancelled {
                updated,
                event_id,
                event,
            } => {
                let project_id = updated["project_id"].as_str().map(str::to_owned);
                self.emit_session_event_frame(sid, project_id, event_id, event);
                self.emit_session_status_frame(updated.clone());
                Ok(updated)
            }
        }
    }
    pub fn finish_generation(
        &self,
        sid: &str,
        generation: &str,
        fields: &Value,
        events: &[Value],
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<Option<Value>> {
        use std::sync::atomic::Ordering;
        let fields = fields
            .as_object()
            .ok_or_else(|| HarnessError::invalid("Terminal session update must be an object."))?;
        if fields
            .keys()
            .any(|key| !["status", "fleet_result"].contains(&key.as_str()))
            || fields
                .get("status")
                .is_some_and(|status| status.as_str().is_none_or(|s| s.is_empty() || s.len() > 64))
        {
            return Err(HarnessError::invalid("Invalid terminal session update."));
        }
        let outcome = {
            let mut db = self.connection()?;
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let current: Option<(String,String)> = tx.query_row(
                "SELECT g.generation,s.status FROM harness_generations g JOIN sessions s ON s.id=g.session_id WHERE s.id=?1",
                [sid], |row| Ok((row.get(0)?,row.get(1)?)),
            ).optional()?;
            let Some((current, status)) = current else {
                return Ok(None);
            };
            if current != generation {
                return Ok(None);
            }
            // Cancellation is durable even when the queued job never acquired a
            // live token. Its old completion must neither resurrect it nor append
            // a spurious failure/check-finished event.
            if status == "cancelled" {
                return Ok(Some(get_session(&tx, sid)?));
            }
            let status = if cancel.load(Ordering::Acquire) {
                "cancelled"
            } else {
                fields
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or(&status)
            };
            if fields.contains_key("status") || status == "cancelled" {
                tx.execute(
                    "UPDATE sessions SET status=?1,updated_at=?2 WHERE id=?3 AND EXISTS(SELECT 1 FROM harness_generations WHERE session_id=?3 AND generation=?4)",
                    params![status,now(),sid,generation],
                )?;
            }
            if let Some(value) = fields.get("fleet_result") {
                let value = if value.is_null() {
                    None
                } else {
                    Some(value.to_string())
                };
                tx.execute(
                    "UPDATE sessions SET fleet_result=?1,updated_at=?2 WHERE id=?3",
                    params![value, now(), sid],
                )?;
            }
            let mut emitted = Vec::with_capacity(events.len());
            for event in events {
                let mut event = event.clone();
                if event.get("status").is_some() {
                    event["status"] = status.into();
                }
                let (normalized, raw) = generation_event_pair(generation, &event)?;
                tx.execute(
                    "INSERT INTO events(session_id,data)VALUES(?1,?2)",
                    params![sid, raw],
                )?;
                emitted.push((tx.last_insert_rowid(), normalized));
            }
            let updated = get_session(&tx, sid)?;
            tx.commit()?;
            (updated, emitted)
        };
        let (updated, emitted) = outcome;
        let project_id = updated["project_id"].as_str().map(str::to_owned);
        for (eid, ev) in emitted {
            self.emit_session_event_frame(sid, project_id.clone(), eid, ev);
        }
        self.emit_session_status_frame(updated.clone());
        Ok(Some(updated))
    }
    pub fn patch_guarded(
        &self,
        sid: &str,
        generation: &str,
        fields: &Value,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<Value> {
        use std::sync::atomic::Ordering;
        let fields = fields
            .as_object()
            .ok_or_else(|| HarnessError::invalid("Session update must be an object."))?;
        let allowed = [
            "status",
            "native_session_id",
            "model",
            "thinking_effort",
            "mode",
            "workspace",
            "branch",
            "git_common_dir",
            "agents_enabled",
            "agent_count",
            "fleet",
            "fleet_result",
            "fleet_action",
            "brain",
            "creator",
            "system_run",
            "system_discovery",
            "system_explain",
            "budgets",
            "budget_usage",
            "budget_revision",
            "result_base",
            "sdd",
            "model_routing",
            "clash",
            "clash_result",
            "cost_totals",
            "omnigraph",
            "permissions",
            "skillcache",
        ];
        if fields.keys().any(|key| !allowed.contains(&key.as_str())) {
            return Err(HarnessError::invalid("Unknown session update."));
        }
        let updated = {
            let mut db = self.connection()?;
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let current:Option<(String,String)>=tx.query_row("SELECT g.generation,s.status FROM harness_generations g JOIN sessions s ON s.id=g.session_id WHERE s.id=?1",[sid],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            if cancel.load(Ordering::Acquire)
                || current
                    .as_ref()
                    .is_none_or(|(g, status)| g != generation || status == "cancelled")
            {
                return Err(HarnessError::conflict(
                    "This session generation is no longer current.",
                ));
            }
            for (key, value) in fields {
                let value = match value {
                    Value::Null => rusqlite::types::Value::Null,
                    Value::String(s) => rusqlite::types::Value::Text(s.clone()),
                    Value::Bool(b) => rusqlite::types::Value::Integer(i64::from(*b)),
                    Value::Number(n) if n.as_i64().is_some() => {
                        rusqlite::types::Value::Integer(n.as_i64().unwrap_or_default())
                    }
                    _ => rusqlite::types::Value::Text(value.to_string()),
                };
                tx.execute(
                    &format!("UPDATE sessions SET {key}=?1,updated_at=?2 WHERE id=?3"),
                    params![value, now(), sid],
                )?;
            }
            if cancel.load(Ordering::Acquire) {
                return Err(HarnessError::conflict("Cancellation requested."));
            }
            let updated = get_session(&tx, sid)?;
            tx.commit()?;
            updated
        };
        self.emit_session_status_frame(updated.clone());
        Ok(updated)
    }
    pub fn patch(&self, sid: &str, fields: &Value) -> Result<Value> {
        let map = fields
            .as_object()
            .ok_or_else(|| HarnessError::invalid("Session update must be an object."))?;
        let allowed = [
            "status",
            "native_session_id",
            "model",
            "thinking_effort",
            "mode",
            "workspace",
            "branch",
            "git_common_dir",
            "agents_enabled",
            "agent_count",
            "fleet",
            "fleet_result",
            "fleet_action",
            "brain",
            "creator",
            "system_run",
            "system_discovery",
            "system_explain",
            "budgets",
            "budget_usage",
            "budget_revision",
            "result_base",
            "sdd",
            "model_routing",
            "clash",
            "clash_result",
            "cost_totals",
            "omnigraph",
            "permissions",
            "skillcache",
        ];
        if map.keys().any(|key| !allowed.contains(&key.as_str())) {
            return Err(HarnessError::invalid("Unknown session update."));
        }
        let mut db = self.connection()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for (key, value) in map {
            let value = match value {
                Value::Null => rusqlite::types::Value::Null,
                Value::String(s) => rusqlite::types::Value::Text(s.clone()),
                Value::Bool(b) => rusqlite::types::Value::Integer(i64::from(*b)),
                Value::Number(n) if n.as_i64().is_some() => {
                    rusqlite::types::Value::Integer(n.as_i64().unwrap_or_default())
                }
                _ => rusqlite::types::Value::Text(value.to_string()),
            };
            tx.execute(
                &format!("UPDATE sessions SET {key}=?1,updated_at=?2 WHERE id=?3"),
                params![value, now(), sid],
            )?;
        }
        tx.commit()?;
        drop(db);
        let updated = self.get(sid)?;
        self.emit_session_status_frame(updated.clone());
        Ok(updated)
    }
    pub fn requeue(&self, sid: &str, prompt: Option<&str>, settings: &Value) -> Result<Value> {
        self.requeue_uploaded(sid, prompt, settings, &[])
    }
    pub fn requeue_uploaded(
        &self,
        sid: &str,
        prompt: Option<&str>,
        settings: &Value,
        uploads: &[crate::attachments::Upload],
    ) -> Result<Value> {
        let old = self.get(sid)?;
        if ["running", "queued"].contains(&old["status"].as_str().unwrap_or("")) {
            return Err(HarnessError::conflict("Wait for this session to finish."));
        }
        let fields = settings
            .as_object()
            .ok_or_else(|| HarnessError::invalid("Session options must be an object."))?;
        let allowed = [
            "model",
            "thinking_effort",
            "sdd",
            "mode",
            "model_routing",
            "agents_enabled",
            "agent_count",
            "clash",
            "brain",
            "fleet_action",
            "omnigraph",
            "permissions",
            "skillcache",
        ];
        if fields.keys().any(|k| !allowed.contains(&k.as_str())) {
            return Err(HarnessError::invalid("Unknown follow-up option."));
        }
        let mut db = self.connection()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = get_session(&tx, sid)?;
        if ["queued", "running"].contains(&current["status"].as_str().unwrap_or("")) {
            return Err(HarnessError::conflict(
                "The session became active before enqueue.",
            ));
        }
        if settings["brain"]["approved"] == true
            && current["brain"]["context_id"] != settings["brain"]["context_id"]
        {
            return Err(HarnessError::conflict(
                "Prepared context changed; review it again.",
            ));
        }
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM sessions WHERE status='queued'",
            [],
            |r| r.get(0),
        )?;
        if count >= 16 {
            return Err(HarnessError::conflict("The run queue is full."));
        }
        let (attached, mut guard) = crate::attachments::stage(&tx, &self.state, sid, uploads)?;
        for (key, value) in fields {
            let value = match value {
                Value::Null => rusqlite::types::Value::Null,
                Value::String(v) => rusqlite::types::Value::Text(v.clone()),
                Value::Bool(v) => rusqlite::types::Value::Integer(i64::from(*v)),
                Value::Number(v) if v.is_i64() => {
                    rusqlite::types::Value::Integer(v.as_i64().unwrap_or_default())
                }
                _ => rusqlite::types::Value::Text(value.to_string()),
            };
            tx.execute(
                &format!("UPDATE sessions SET {key}=?1 WHERE id=?2"),
                params![value, sid],
            )?;
        }
        tx.execute(
            "UPDATE sessions SET status='queued',updated_at=?1 WHERE id=?2",
            params![now(), sid],
        )?;
        tx.execute("INSERT INTO harness_generations(session_id,generation)VALUES(?1,?2)ON CONFLICT(session_id)DO UPDATE SET generation=excluded.generation",params![sid,hexid()])?;
        let prompt_event = if let Some(prompt) = prompt {
            let mut ev = json!({"kind":"user","text":prompt,"attachments":attached});
            ev["at"] = chrono::Utc::now()
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                .into();
            tx.execute(
                "INSERT INTO events(session_id,data)VALUES(?1,?2)",
                params![sid, ev.to_string()],
            )?;
            Some((tx.last_insert_rowid(), ev))
        } else {
            None
        };
        tx.commit()?;
        guard.commit();
        drop(db);
        let session = self.get(sid)?;
        if let Some((eid, ev)) = prompt_event {
            let proj = session["project_id"].as_str().map(str::to_owned);
            self.emit_session_event_frame(sid, proj, eid, ev);
        }
        self.emit_session_status_frame(session.clone());
        Ok(session)
    }
    pub fn merge_archive(&self, sid: &str) -> Result<Value> {
        crate::chat_merge::ChatMerge::new(self.clone()).archive(sid)
    }
    pub fn delete_merge(&self, sid: &str) -> Result<Value> {
        crate::chat_merge::ChatMerge::new(self.clone()).delete(sid)
    }
}
fn expire_pending_permissions(tx: &Connection) -> Result<()> {
    // No PermissionDesk waiters survive a process restart, so any still-pending
    // permission event is closed before clients reload or poll again.
    let rows = {
        let mut q = tx.prepare(
            "SELECT id,session_id,data FROM events WHERE data LIKE '%\"permission\"%' ORDER BY id",
        )?;
        q.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?
    };
    let mut resolved = BTreeMap::<(String, String), i64>::new();
    let mut pending = Vec::new();
    for (row_id, sid, raw) in rows {
        let Ok(parsed) = crate::strict_json::parse(raw.as_bytes(), 4 * 1024 * 1024) else {
            continue;
        };
        if parsed["kind"] != "permission" {
            continue;
        }
        let request_id = parsed["request_id"].as_str().unwrap_or("").to_owned();
        if parsed["state"] == "pending" {
            pending.push((row_id, sid, request_id, parsed));
        } else if !request_id.is_empty() {
            resolved
                .entry((sid, request_id))
                .and_modify(|last| *last = (*last).max(row_id))
                .or_insert(row_id);
        }
    }
    for (row_id, sid, request_id, mut event) in pending {
        if !request_id.is_empty()
            && resolved
                .get(&(sid, request_id))
                .is_some_and(|&resolved_id| resolved_id > row_id)
        {
            continue;
        }
        if let Some(obj) = event.as_object_mut() {
            obj.insert("state".into(), json!("expired"));
            obj.insert("by".into(), json!("harness"));
            if let (Some(tool), Some(summary)) = (
                obj.get("tool").and_then(Value::as_str).map(str::to_owned),
                obj.get("summary")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            ) {
                obj.insert(
                    "text".into(),
                    json!(format!("{tool} no longer waiting: {summary}")),
                );
            }
        }
        tx.execute(
            "UPDATE events SET data=?1 WHERE id=?2",
            params![event.to_string(), row_id],
        )?;
    }
    Ok(())
}
fn encode_option(value: &Value) -> Option<String> {
    if value.is_null() {
        None
    } else {
        Some(value.to_string())
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "A single SQLite transaction binds the frozen session row, first event, uploads and generation together."
)]
pub(crate) fn insert_session(
    tx: &Connection,
    options: &Value,
    project: &Value,
    prompt: &str,
    attachments: &[Value],
    sid: &str,
    at: &str,
    generation: &str,
) -> Result<()> {
    let provider = options["provider"]
        .as_str()
        .ok_or_else(|| HarnessError::invalid("Select a provider."))?;
    tx.execute("INSERT INTO sessions(id,title,project_id,project_path,provider,model,mode,workflow,project_context,status,native_session_id,created_at,updated_at,agents_enabled,agent_count,thinking_effort,workspace,branch,git_common_dir,fleet,fleet_action,brain,budgets,sdd,model_routing,clash,permissions)VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'queued',NULL,?10,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24)",
   params![sid,prompt.chars().take(80).collect::<String>(),project["id"].as_str(),options["_workspace_path"].as_str().or_else(||project["path"].as_str()),provider,options["model"].as_str(),
   options["mode"].as_str().unwrap_or("plan"),options["workflow"].as_str().unwrap_or("native"),i64::from(options["project_context"].as_bool().unwrap_or(false)),
   at,i64::from(options["agents_enabled"].as_bool().unwrap_or(false)),options["agent_count"].as_i64().unwrap_or(3),options["thinking_effort"].as_str(),
   options["workspace"].as_str().unwrap_or("project"),options["branch"].as_str(),options["git_common_dir"].as_str(),encode_option(&options["fleet"]),if options["fleet"].is_null(){None}else{Some("start")},
   encode_option(&options["brain"]),encode_option(&options["budgets"]),encode_option(&options["sdd"]),encode_option(&options["model_routing"]),encode_option(&options["clash"]),options["permissions"].as_str()])?;
    tx.execute(
        "INSERT INTO events(session_id,data)VALUES(?1,?2)",
        params![
            sid,
            json!({"kind":"user","text":prompt,"attachments":attachments}).to_string()
        ],
    )?;
    tx.execute(
        "INSERT INTO harness_generations(session_id,generation)VALUES(?1,?2)",
        params![sid, generation],
    )?;
    Ok(())
}

pub fn agent_plan(session: &Value) -> Value {
    let fleet = &session["fleet"];
    let helpers = session["agent_count"].as_u64().unwrap_or(3);
    let count = if let Some(lenses) = fleet["lenses"].as_array() {
        lenses.len() as u64
    } else {
        1 + if session["agents_enabled"] == true {
            helpers
        } else {
            0
        }
    }
    .max(1);
    let concurrent = if fleet.is_null() {
        count
    } else {
        count.min(helpers)
    }
    .max(1);
    let tokens = session["budgets"]["tokens"].as_u64();
    let seconds = session["budgets"]["seconds"].as_u64();
    let worker = fleet["worker_timeout"].as_u64();
    let seconds = if fleet.is_null() {
        seconds
    } else {
        match (seconds, worker) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (_, b) => b,
        }
    };
    json!({"agents":count,"concurrent":concurrent,"waves":count.div_ceil(concurrent),"usd":session["budgets"]["usd"].as_f64().map(|v|(v/count as f64*1_000_000.).floor()/1_000_000.),"tokens":tokens.map(|n|n/count),"unallocated_tokens":tokens.map(|n|n%count),"seconds":seconds,"shared_seconds":session["budgets"]["seconds"],"scope":if fleet.is_null(){"main_and_helper_slots"}else{"reviewers"}})
}

pub(crate) fn get_session(db: &Connection, sid: &str) -> Result<Value> {
    let mut q = db.prepare(
        "SELECT s.*,m.summary FROM sessions s LEFT JOIN session_merges m ON m.session_id=s.id WHERE s.id=?1",
    )?;
    let names = session_columns(&q);
    let mut rows = q.query([sid])?;
    let row = rows
        .next()?
        .ok_or_else(|| HarnessError::missing("Session not found."))?;
    decode_session(row, &names)
}

fn session_columns(statement: &rusqlite::Statement<'_>) -> Vec<String> {
    // The final column belongs to the join, rather than the public session.
    statement
        .column_names()
        .iter()
        .take(statement.column_count() - 1)
        .map(|s| s.to_string())
        .collect()
}

fn decode_session(row: &rusqlite::Row<'_>, names: &[String]) -> Result<Value> {
    let mut result = Map::new();
    for (i, name) in names.iter().enumerate() {
        use rusqlite::types::ValueRef;
        let value = match row.get_ref(i)? {
            ValueRef::Null => Value::Null,
            ValueRef::Integer(n) => json!(n),
            ValueRef::Real(n) => json!(n),
            ValueRef::Text(s) => Value::String(
                std::str::from_utf8(s)
                    .map_err(|_| HarnessError::invalid("Invalid session text."))?
                    .into(),
            ),
            _ => return Err(HarnessError::invalid("Invalid session field.")),
        };
        result.insert(name.clone(), value);
    }
    for key in ["project_context", "agents_enabled"] {
        let value = result.get(key).and_then(Value::as_i64).unwrap_or(0) != 0;
        result.insert(key.into(), value.into());
    }
    for key in [
        "fleet",
        "fleet_result",
        "brain",
        "creator",
        "system_run",
        "system_discovery",
        "system_explain",
        "budgets",
        "budget_usage",
        "result_base",
        "sdd",
        "model_routing",
        "clash",
        "clash_result",
        "cost_totals",
        "omnigraph",
        "skillcache",
    ] {
        if let Some(raw) = result.get(key).and_then(Value::as_str) {
            let parsed = crate::strict_json::parse(raw.as_bytes(), 16 * 1024 * 1024)?;
            result.insert(key.into(), parsed);
        }
    }
    result.remove("fleet_action");
    let summary: Option<String> = row.get(names.len())?;
    result.insert(
        "merge".into(),
        summary
            .map(|s| crate::strict_json::parse(s.as_bytes(), 2 * 1024 * 1024))
            .transpose()?
            .unwrap_or(Value::Null),
    );
    if result.get("budgets").is_none_or(Value::is_null) {
        let usd = result
            .get("fleet")
            .map(|v| v["budget_usd"].clone())
            .unwrap_or(Value::Null);
        result.insert(
            "budgets".into(),
            json!({"usd":usd,"tokens":null,"seconds":900}),
        );
    }
    let mut result = Value::Object(result);
    let plan = agent_plan(&result);
    result["agent_budget_plan"] = plan;
    Ok(result)
}

fn decode_event_rows(
    db: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<Value>> {
    let mut q = db.prepare(sql)?;
    let rows = q.query_map(params, |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, raw) = row?;
        let mut v = crate::strict_json::parse(raw.as_bytes(), 4 * 1024 * 1024)?;
        v["id"] = id.into();
        out.push(v);
    }
    Ok(out)
}

fn session_events(db: &Connection, sid: &str, after: i64) -> Result<Vec<Value>> {
    if after < 0 {
        return Err(HarnessError::invalid("Invalid event cursor."));
    }
    decode_event_rows(
        db,
        "SELECT id,data FROM events WHERE session_id=?1 AND id>?2 ORDER BY id LIMIT 250",
        params![sid, after],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, SessionStore) {
        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&directory.path().join("state")).unwrap();
        (directory, store)
    }

    fn insert(db: &Connection, sid: &str, updated: &str) {
        db.execute(
            "INSERT INTO sessions(id,title,project_id,project_path,provider,mode,workflow,project_context,status,created_at,updated_at) VALUES(?1,'Example','project','/project','codex','plan','native',0,'completed','created',?2)",
            params![sid, updated],
        )
        .unwrap();
    }

    fn merge(db: &Connection, sid: &str, summary: &str) {
        db.execute(
            "INSERT INTO session_merges(session_id,request_id,request_hash,bundle,summary,context) VALUES(?1,?1,'hash','{}',?2,'context')",
            params![sid, summary],
        )
        .unwrap();
    }

    fn error<T>(result: Result<T>) -> (u16, &'static str, String) {
        match result {
            Ok(_) => panic!("expected an error"),
            Err(error) => (error.status, error.code, error.message),
        }
    }

    #[test]
    fn summaries_preserve_timestamp_and_rowid_order_and_cap() {
        let (_directory, store) = fixture();
        {
            let db = store.connection().unwrap();
            for i in 0..205 {
                insert(&db, &format!("session-{i}"), &format!("updated-{}", i % 3));
            }
            merge(&db, "session-204", r#"{"sources":["earlier"]}"#);
        }
        let summaries = store.summaries().unwrap();
        let mut expected = (0..205).collect::<Vec<_>>();
        expected.sort_by_key(|i| std::cmp::Reverse((i % 3, *i)));
        let expected = expected
            .into_iter()
            .take(200)
            .map(|i| format!("session-{i}"))
            .collect::<Vec<_>>();
        assert_eq!(summaries.len(), 200);
        for (summary, sid) in summaries.iter().zip(&expected) {
            assert_eq!(summary["id"], sid.as_str());
            assert_eq!(*summary, store.get(sid).unwrap());
            assert!(!summary.as_object().unwrap().contains_key("summary"));
        }
        assert_eq!(
            summaries.iter().find(|s| s["id"] == "session-204").unwrap()["merge"],
            json!({"sources":["earlier"]})
        );
    }

    #[test]
    fn joined_sessions_keep_normalization_defaults_and_null_merge() {
        let (_directory, store) = fixture();
        {
            let db = store.connection().unwrap();
            insert(&db, "session", "updated");
            db.execute(
                "UPDATE sessions SET project_context=2,agents_enabled=-1,fleet=?1,fleet_action='private',brain=?2 WHERE id='session'",
                params![r#"{"budget_usd":12}"#, r#"{"task":"TASK-1"}"#],
            )
            .unwrap();
        }
        let session = store.get("session").unwrap();
        assert_eq!(session["project_context"], true);
        assert_eq!(session["agents_enabled"], true);
        assert_eq!(session["brain"], json!({"task":"TASK-1"}));
        assert_eq!(
            session["budgets"],
            json!({"usd":12,"tokens":null,"seconds":900})
        );
        assert_eq!(session["agent_budget_plan"]["agents"], 4);
        assert_eq!(session["agent_budget_plan"]["concurrent"], 3);
        assert_eq!(session["agent_budget_plan"]["usd"], 3.0);
        assert_eq!(session["merge"], Value::Null);
        assert!(!session.as_object().unwrap().contains_key("fleet_action"));
        assert!(!session.as_object().unwrap().contains_key("summary"));
        assert_eq!(store.summaries().unwrap(), vec![session]);
    }

    #[test]
    fn summaries_validate_selected_json_without_reading_older_corruption() {
        let (_directory, store) = fixture();
        {
            let db = store.connection().unwrap();
            for i in 0..205 {
                insert(&db, &format!("session-{i}"), "updated");
            }
            db.execute(
                "UPDATE sessions SET brain=?1 WHERE id='session-0'",
                [r#"{"task":1,"task":2}"#],
            )
            .unwrap();
            merge(&db, "session-1", r#"{"sources":1,"sources":2}"#);
        }
        assert_eq!(store.summaries().unwrap().len(), 200);
        store
            .connection()
            .unwrap()
            .execute(
                "UPDATE sessions SET updated_at='z-newest' WHERE id='session-0'",
                [],
            )
            .unwrap();
        assert_eq!(error(store.summaries()), error(store.get("session-0")));
        {
            let db = store.connection().unwrap();
            db.execute("UPDATE sessions SET brain=NULL WHERE id='session-0'", [])
                .unwrap();
            db.execute(
                "UPDATE sessions SET updated_at='z-newest' WHERE id='session-1'",
                [],
            )
            .unwrap();
        }
        assert_eq!(error(store.summaries()), error(store.get("session-1")));
    }

    #[test]
    fn combined_poll_preserves_event_order_cursor_limit_and_session() {
        let (_directory, store) = fixture();
        {
            let db = store.connection().unwrap();
            insert(&db, "session", "updated");
            insert(&db, "other", "updated");
        }
        store.event("other", &json!({"kind":"status"})).unwrap();
        for sequence in 0..251 {
            store
                .event(
                    "session",
                    &json!({"kind":"status","sequence":sequence,"id":"untrusted"}),
                )
                .unwrap();
        }
        let (session, events) = store.session_events("session", 0).unwrap();
        assert_eq!(session, store.get("session").unwrap());
        assert_eq!(events, store.events("session", 0).unwrap());
        assert_eq!(events.len(), 250);
        assert_eq!(events.first().unwrap()["id"], 2);
        assert_eq!(events.first().unwrap()["sequence"], 0);
        assert_eq!(events.last().unwrap()["id"], 251);
        assert_eq!(events.last().unwrap()["sequence"], 249);
        let (_, tail) = store.session_events("session", 251).unwrap();
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0]["id"], 252);
        assert_eq!(tail[0]["sequence"], 250);
        assert!(store.session_events("session", 252).unwrap().1.is_empty());
    }

    #[test]
    fn poll_error_order_and_strict_session_and_event_json_are_preserved() {
        let (_directory, store) = fixture();
        assert_eq!(
            error(store.events("missing", -1)).2,
            "Invalid event cursor."
        );
        assert_eq!(
            error(store.session_events("missing", -1)).2,
            "Session not found."
        );
        assert_eq!(error(store.events("missing", 0)).0, 404);
        assert_eq!(error(store.session_events("missing", 0)).0, 404);
        {
            let db = store.connection().unwrap();
            insert(&db, "session", "updated");
            db.execute(
                "UPDATE sessions SET brain=?1 WHERE id='session'",
                [r#"{"a":1,"a":2}"#],
            )
            .unwrap();
        }
        let invalid_session = error(store.get("session"));
        assert_eq!(error(store.events("session", 0)), invalid_session);
        assert_eq!(error(store.session_events("session", -1)), invalid_session);
        assert_eq!(
            error(store.events("session", -1)).2,
            "Invalid event cursor."
        );
        {
            let db = store.connection().unwrap();
            db.execute("UPDATE sessions SET brain=NULL WHERE id='session'", [])
                .unwrap();
            db.execute(
                "INSERT INTO events(session_id,data) VALUES('session',?1)",
                [r#"{"kind":"status","kind":"other"}"#],
            )
            .unwrap();
        }
        assert_eq!(
            error(store.session_events("session", -1)).2,
            "Invalid event cursor."
        );
        let invalid_event = error(store.events("session", 0));
        assert_eq!(invalid_event.0, 400);
        assert_eq!(error(store.session_events("session", 0)), invalid_event);
        assert!(store.session_events("session", 1).unwrap().1.is_empty());
    }

    #[test]
    fn reopen_expires_dangling_pending_permissions_without_touching_resolved_ones() {
        let (directory, store) = fixture();
        let state = directory.path().join("state");
        {
            let db = store.connection().unwrap();
            insert(&db, "s1", "updated");
            db.execute("UPDATE sessions SET status='running' WHERE id='s1'", [])
                .unwrap();
            // Already resolved request must keep its pending + allowed history.
            db.execute(
                "INSERT INTO events(session_id,data) VALUES('s1',?1)",
                [json!({
                    "kind": "permission",
                    "state": "pending",
                    "request_id": "req-resolved",
                    "provider": "claude",
                    "tool": "Bash",
                    "summary": "ls",
                    "session_option": true,
                    "text": "Claude Code asks to run Bash: ls"
                })
                .to_string()],
            )
            .unwrap();
            db.execute(
                "INSERT INTO events(session_id,data) VALUES('s1',?1)",
                [json!({
                    "kind": "permission",
                    "state": "allowed",
                    "request_id": "req-resolved",
                    "provider": "claude",
                    "tool": "Bash",
                    "summary": "ls",
                    "by": "person",
                    "text": "Bash allowed: ls"
                })
                .to_string()],
            )
            .unwrap();
            // Dangling request still pending when the process stopped.
            db.execute(
                "INSERT INTO events(session_id,data) VALUES('s1',?1)",
                [json!({
                    "kind": "permission",
                    "state": "pending",
                    "request_id": "req-dangling",
                    "provider": "codex",
                    "tool": "Shell",
                    "summary": "touch probe.txt",
                    "session_option": true,
                    "text": "Codex asks to run Shell: touch probe.txt"
                })
                .to_string()],
            )
            .unwrap();
            // Corrupt non-permission event must not break startup recovery.
            db.execute(
                "INSERT INTO events(session_id,data) VALUES('s1',?1)",
                [r#"{"kind":"permission","kind":"dup"}"#],
            )
            .unwrap();
        }
        drop(store);
        let reopened = SessionStore::open(&state).unwrap();
        assert_eq!(reopened.get("s1").unwrap()["status"], "interrupted");
        let rows = {
            let db = reopened.connection().unwrap();
            let mut q = db
                .prepare("SELECT data FROM events WHERE session_id='s1' ORDER BY id")
                .unwrap();
            q.query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap()
        };
        let valid: Vec<Value> = rows
            .into_iter()
            .filter_map(|raw| crate::strict_json::parse(raw.as_bytes(), 65536).ok())
            .collect();
        let resolved_pending = valid
            .iter()
            .find(|e| e["request_id"] == "req-resolved" && e["state"] == "pending");
        assert!(resolved_pending.is_some());
        let dangling = valid
            .iter()
            .find(|e| e["request_id"] == "req-dangling")
            .unwrap();
        assert_eq!(dangling["state"], "expired");
        assert_eq!(dangling["by"], "harness");
        assert_eq!(dangling["text"], "Shell no longer waiting: touch probe.txt");
        assert!(valid.iter().any(|e| {
            e["kind"] == "status"
                && e["text"]
                    .as_str()
                    .is_some_and(|t| t.contains("Server restarted"))
        }));
    }

    #[test]
    fn session_events_page_supports_after_before_and_tail_beyond_250() {
        let (_dir, store) = fixture();
        {
            let db = store.connection().unwrap();
            insert(&db, "s-page", "t1");
        }
        for i in 1..=320 {
            store
                .event(
                    "s-page",
                    &json!({"kind": "status", "text": format!("e-{i}")}),
                )
                .unwrap();
        }
        // Existing session_events API stays capped at 250.
        let (_, legacy) = store.session_events("s-page", 0).unwrap();
        assert_eq!(legacy.len(), 250);
        assert_eq!(legacy[0]["text"], "e-1");

        // Tail returns newest 100 events in chronological ascending order.
        let tail = store
            .session_events_page("s-page", None, None, 100, true)
            .unwrap();
        assert_eq!(tail.total_events, 320);
        assert_eq!(tail.events.len(), 100);
        assert!(!tail.has_more);
        assert!(tail.has_older);
        assert_eq!(tail.events[0]["text"], "e-221");
        assert_eq!(tail.events[99]["text"], "e-320");

        // Before cursor paginates backward cleanly in chronological order.
        let first_tail_id = tail.events[0]["id"].as_i64().unwrap();
        let older = store
            .session_events_page("s-page", None, Some(first_tail_id), 150, false)
            .unwrap();
        assert_eq!(older.events.len(), 150);
        assert!(older.has_more);
        assert!(older.has_older);
        assert_eq!(older.events[0]["text"], "e-71");
        assert_eq!(older.events[149]["text"], "e-220");

        // Remaining oldest 70.
        let oldest_id = older.events[0]["id"].as_i64().unwrap();
        let earliest = store
            .session_events_page("s-page", None, Some(oldest_id), 150, false)
            .unwrap();
        assert_eq!(earliest.events.len(), 70);
        assert!(earliest.has_more);
        assert!(!earliest.has_older);
        assert_eq!(earliest.events[0]["text"], "e-1");
        assert_eq!(earliest.events[69]["text"], "e-70");
    }

    #[test]
    fn stream_hub_ring_buffer_replays_and_signals_resync() {
        let hub = StreamHub::new(16);
        for i in 1..=25 {
            hub.publish(
                "session_event",
                Some("s1".into()),
                Some("p1".into()),
                json!({"n": i}),
            );
        }
        // Within ring window [10..=25]
        match hub.subscribe(Some(22)) {
            ReplayStart::Frames { frames, .. } => {
                assert_eq!(frames.len(), 3);
                assert_eq!(frames[0].seq, 23);
                assert_eq!(frames[2].seq, 25);
            }
            ReplayStart::Resync { .. } => panic!("expected frames inside ring buffer"),
        }
        // Evicted cursor < front.seq - 1 (front is 10, cursor 5 < 9)
        match hub.subscribe(Some(5)) {
            ReplayStart::Resync {
                oldest_id,
                latest_id,
                reason,
                ..
            } => {
                assert_eq!(oldest_id, 10);
                assert_eq!(latest_id, 25);
                assert_eq!(reason, "cursor_expired");
            }
            ReplayStart::Frames { .. } => panic!("expected resync on evicted cursor"),
        }
        // Cursor ahead of latest (e.g. server restarted)
        match hub.subscribe(Some(99)) {
            ReplayStart::Resync { reason, .. } => assert_eq!(reason, "server_restarted"),
            ReplayStart::Frames { .. } => panic!("expected resync on future cursor"),
        }
    }
}
