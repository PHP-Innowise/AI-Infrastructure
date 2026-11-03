use crate::{HarnessError, Result};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeSet,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};
#[derive(Clone)]
pub struct SessionStore {
    pub state: PathBuf,
    db: Arc<Mutex<Connection>>,
    _runner: Arc<File>,
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
fn generation_event(generation: &str, event: &Value) -> Result<String> {
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
    Ok(raw)
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
            ("budgets", "TEXT"),
            ("budget_usage", "TEXT"),
            ("budget_revision", "INTEGER NOT NULL DEFAULT 0"),
            ("result_base", "TEXT"),
            ("sdd", "TEXT"),
            ("model_routing", "TEXT"),
            ("clash", "TEXT"),
            ("clash_result", "TEXT"),
            ("cost_totals", "TEXT"),
        ] {
            if !columns.contains(name) {
                db.execute_batch(&format!(
                    "ALTER TABLE sessions ADD COLUMN {name} {definition}"
                ))?;
            }
        }
        let interrupted = {
            let mut q =
                db.prepare("SELECT id FROM sessions WHERE status IN ('queued','running')")?;
            q.query_map([], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        for sid in interrupted {
            db.execute("INSERT INTO events(session_id,data)VALUES(?1,?2)",params![sid,json!({"kind":"status","text":"Server restarted; the previous run was interrupted."}).to_string()])?;
        }
        db.execute("UPDATE sessions SET status='interrupted',updated_at=?1 WHERE status IN ('queued','running')",[now()])?;
        Ok(Self {
            state,
            db: Arc::new(Mutex::new(db)),
            _runner: Arc::new(runner),
        })
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
        let ids = {
            let db = self.connection()?;
            let mut q = db
                .prepare("SELECT id FROM sessions ORDER BY updated_at DESC,rowid DESC LIMIT 200")?;
            q.query_map([], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        ids.iter().map(|s| self.get(s)).collect()
    }
    pub fn events(&self, sid: &str, after: i64) -> Result<Vec<Value>> {
        if after < 0 {
            return Err(HarnessError::invalid("Invalid event cursor."));
        }
        self.get(sid)?;
        let db = self.connection()?;
        let mut q = db.prepare(
            "SELECT id,data FROM events WHERE session_id=?1 AND id>?2 ORDER BY id LIMIT 250",
        )?;
        let rows = q.query_map(params![sid, after], |r| {
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
        let db = self.connection()?;
        db.execute(
            "INSERT INTO events(session_id,data)VALUES(?1,?2)",
            params![sid, raw],
        )?;
        Ok(db.last_insert_rowid())
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
        for key in ["creator", "system_run", "system_discovery"] {
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
        self.get(&sid)
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
            return Ok(false);
        }
        let raw = generation_event(generation, event)?;
        tx.execute(
            "INSERT INTO events(session_id,data)VALUES(?1,?2)",
            params![sid, raw],
        )?;
        tx.commit()?;
        Ok(true)
    }
    pub fn cancel_current(&self, sid: &str) -> Result<Value> {
        let mut db = self.connection()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let session = get_session(&tx, sid)?;
        if !["queued", "running"].contains(&session["status"].as_str().unwrap_or("")) {
            return Ok(session);
        }
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
        let event = generation_event(
            &generation,
            &json!({"kind":"status","text":"Cancellation requested."}),
        )?;
        tx.execute("UPDATE sessions SET status='cancelled',updated_at=?1 WHERE id=?2 AND status IN ('queued','running')", params![now(),sid])?;
        tx.execute(
            "INSERT INTO events(session_id,data)VALUES(?1,?2)",
            params![sid, event],
        )?;
        let updated = get_session(&tx, sid)?;
        tx.commit()?;
        Ok(updated)
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
        for event in events {
            let mut event = event.clone();
            if event.get("status").is_some() {
                event["status"] = status.into();
            }
            let event = generation_event(generation, &event)?;
            tx.execute(
                "INSERT INTO events(session_id,data)VALUES(?1,?2)",
                params![sid, event],
            )?;
        }
        let updated = get_session(&tx, sid)?;
        tx.commit()?;
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
            "budgets",
            "budget_usage",
            "budget_revision",
            "result_base",
            "sdd",
            "model_routing",
            "clash",
            "clash_result",
            "cost_totals",
        ];
        if fields.keys().any(|key| !allowed.contains(&key.as_str())) {
            return Err(HarnessError::invalid("Unknown session update."));
        }
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
            "budgets",
            "budget_usage",
            "budget_revision",
            "result_base",
            "sdd",
            "model_routing",
            "clash",
            "clash_result",
            "cost_totals",
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
        self.get(sid)
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
        if let Some(prompt) = prompt {
            tx.execute(
                "INSERT INTO events(session_id,data)VALUES(?1,?2)",
                params![
                    sid,
                    json!({"kind":"user","text":prompt,"attachments":attached}).to_string()
                ],
            )?;
        }
        tx.commit()?;
        guard.commit();
        drop(db);
        self.get(sid)
    }
    pub fn merge_archive(&self, sid: &str) -> Result<Value> {
        crate::chat_merge::ChatMerge::new(self.clone()).archive(sid)
    }
    pub fn delete_merge(&self, sid: &str) -> Result<Value> {
        crate::chat_merge::ChatMerge::new(self.clone()).delete(sid)
    }
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
    tx.execute("INSERT INTO sessions(id,title,project_id,project_path,provider,model,mode,workflow,project_context,status,native_session_id,created_at,updated_at,agents_enabled,agent_count,thinking_effort,workspace,branch,git_common_dir,fleet,fleet_action,brain,budgets,sdd,model_routing,clash)VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'queued',NULL,?10,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)",
   params![sid,prompt.chars().take(80).collect::<String>(),project["id"].as_str(),options["_workspace_path"].as_str().or_else(||project["path"].as_str()),provider,options["model"].as_str(),
   options["mode"].as_str().unwrap_or("plan"),options["workflow"].as_str().unwrap_or("native"),i64::from(options["project_context"].as_bool().unwrap_or(false)),
   at,i64::from(options["agents_enabled"].as_bool().unwrap_or(false)),options["agent_count"].as_i64().unwrap_or(3),options["thinking_effort"].as_str(),
   options["workspace"].as_str().unwrap_or("project"),options["branch"].as_str(),options["git_common_dir"].as_str(),encode_option(&options["fleet"]),if options["fleet"].is_null(){None}else{Some("start")},
   encode_option(&options["brain"]),encode_option(&options["budgets"]),encode_option(&options["sdd"]),encode_option(&options["model_routing"]),encode_option(&options["clash"])])?;
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
    let mut q = db.prepare("SELECT * FROM sessions WHERE id=?1")?;
    let names = q
        .column_names()
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    let mut rows = q.query([sid])?;
    let row = rows
        .next()?
        .ok_or_else(|| HarnessError::missing("Session not found."))?;
    let mut result = Map::new();
    for (i, name) in names.into_iter().enumerate() {
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
        result.insert(name, value);
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
        "budgets",
        "budget_usage",
        "result_base",
        "sdd",
        "model_routing",
        "clash",
        "clash_result",
        "cost_totals",
    ] {
        if let Some(raw) = result.get(key).and_then(Value::as_str) {
            let parsed = crate::strict_json::parse(raw.as_bytes(), 16 * 1024 * 1024)?;
            result.insert(key.into(), parsed);
        }
    }
    result.remove("fleet_action");
    let summary: Option<String> = db
        .query_row(
            "SELECT summary FROM session_merges WHERE session_id=?1",
            [sid],
            |r| r.get(0),
        )
        .optional()?;
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
    result.insert(
        "agent_budget_plan".into(),
        agent_plan(&Value::Object(result.clone())),
    );
    Ok(Value::Object(result))
}
