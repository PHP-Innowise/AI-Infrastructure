use crate::common::*;
use infra_core::{Result, RuntimeError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
}
/// Only registration/session bookkeeping crosses the Harness boundary. Source
/// discovery, plans, approvals, editor effects and workers are native here.
pub trait SessionBridge: Send + Sync {
    fn projects(&self) -> Result<BTreeMap<String, Project>>;
    fn provider(&self, id: &str) -> Result<Option<PathBuf>>;
    fn active(&self) -> Result<bool>;
    fn admit(&self) -> Result<()>;
    fn runner_lock(&self) -> Result<Arc<std::fs::File>>;
    fn launch(
        &self,
        request: &Value,
        system_run: Option<&Value>,
        discovery: Option<&Value>,
    ) -> Result<Value>;
    fn session(&self, id: &str) -> Result<Value>;
    fn cancel(&self, id: &str) -> Result<Value>;
    fn events(&self, id: &str, kinds: &[&str]) -> Result<Vec<Value>>;
    fn find_launch(&self, run_id: &str, nonce: &str) -> Result<Option<Value>>;
}
pub struct Database {
    connection: Mutex<rusqlite::Connection>,
}
impl Database {
    pub fn new(state: &Path) -> Result<Self> {
        crate::native::private_directory(state)?;
        let path = state.join("sessions.sqlite3");
        let mut custody = infra_core::bank::publication::PublicationFiles::new(state)?;
        #[cfg(unix)]
        let before = custody.metadata_image_identity("sessions.sqlite3")?;
        #[cfg(not(unix))]
        let before = custody.image_identity("sessions.sqlite3")?;
        if let Some(identity) = before
            && identity.3 != 1
        {
            return Err(fail("Session database must have one ordinary file link"));
        }
        #[cfg(unix)]
        let connection = rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::default() | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        #[cfg(not(unix))]
        let connection = rusqlite::Connection::open(&path)?;
        #[cfg(unix)]
        let opened = {
            let opened = custody
                .metadata_image_identity("sessions.sqlite3")?
                .ok_or_else(|| RuntimeError::conflict("Opened session database disappeared"))?;
            if before.is_some_and(|before| before != opened) {
                return Err(RuntimeError::conflict(
                    "Session database changed while opening",
                ));
            }
            custody.check()?;
            opened
        };
        connection.busy_timeout(std::time::Duration::from_secs(10))?;
        connection.execute_batch("CREATE TABLE IF NOT EXISTS system_runs(id TEXT PRIMARY KEY,data TEXT NOT NULL,project_id TEXT);CREATE TABLE IF NOT EXISTS system_edits(id TEXT PRIMARY KEY,data TEXT NOT NULL);")?;
        let mut statement = connection.prepare("PRAGMA table_info(system_runs)")?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        if !columns.iter().any(|column| column == "project_id") {
            connection.execute("ALTER TABLE system_runs ADD COLUMN project_id TEXT", [])?;
            let mut statement = connection.prepare("SELECT id,data FROM system_runs")?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(statement);
            for (id, data) in rows {
                let data = parse_json(data.as_bytes(), 8 * MAX_BYTES)?;
                connection.execute(
                    "UPDATE system_runs SET project_id=?1 WHERE id=?2",
                    rusqlite::params![string(&data["project_id"]), id],
                )?;
            }
        }
        connection.execute(
            "CREATE INDEX IF NOT EXISTS system_runs_project ON system_runs(project_id)",
            [],
        )?;
        #[cfg(unix)]
        custody.secure_private_file_metadata("sessions.sqlite3", opened)?;
        #[cfg(not(unix))]
        infra_host::util::private_file(&path).map_err(host)?;
        custody.check()?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }
    pub fn get(&self, table: &str, id: &str) -> Result<Option<Value>> {
        let query = match table {
            "system_runs" => "SELECT data FROM system_runs WHERE id=?1",
            "system_edits" => "SELECT data FROM system_edits WHERE id=?1",
            _ => return Err(fail("Unknown system table")),
        };
        let connection = self
            .connection
            .lock()
            .map_err(|_| fail("System database lock poisoned"))?;
        let mut statement = connection.prepare(query)?;
        let mut rows = statement.query([id])?;
        if let Some(row) = rows.next()? {
            let raw: String = row.get(0)?;
            Ok(Some(parse_json(raw.as_bytes(), 8 * MAX_BYTES)?))
        } else {
            Ok(None)
        }
    }
    pub fn put(&self, table: &str, id: &str, value: &Value) -> Result<()> {
        let raw = encoded(value)?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| fail("System database lock poisoned"))?;
        match table {
            "system_runs" => {
                connection.execute(
                    "INSERT OR REPLACE INTO system_runs(id,data,project_id)VALUES(?1,?2,?3)",
                    rusqlite::params![id, raw, string(&value["project_id"])],
                )?;
            }
            "system_edits" => {
                connection.execute(
                    "INSERT OR REPLACE INTO system_edits(id,data)VALUES(?1,?2)",
                    rusqlite::params![id, raw],
                )?;
            }
            _ => return Err(fail("Unknown system table")),
        }
        Ok(())
    }
    pub fn delete(&self, table: &str, id: &str) -> Result<()> {
        let query = match table {
            "system_runs" => "DELETE FROM system_runs WHERE id=?1",
            "system_edits" => "DELETE FROM system_edits WHERE id=?1",
            _ => return Err(fail("Unknown system table")),
        };
        self.connection
            .lock()
            .map_err(|_| fail("System database lock poisoned"))?
            .execute(query, [id])?;
        Ok(())
    }
    pub fn all(&self, table: &str, project: Option<&str>) -> Result<Vec<(String, Value)>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| fail("System database lock poisoned"))?;
        let sql = match (table, project.is_some()) {
            ("system_runs", true) => {
                "SELECT id,data FROM system_runs WHERE project_id=?1 ORDER BY rowid DESC LIMIT 200"
            }
            ("system_runs", false) => "SELECT id,data FROM system_runs ORDER BY rowid",
            ("system_edits", _) => "SELECT id,data FROM system_edits ORDER BY rowid",
            _ => return Err(fail("Unknown system table")),
        };
        let mut statement = connection.prepare(sql)?;
        let mut rows = if let Some(project) = project.filter(|_| table == "system_runs") {
            statement.query([project])?
        } else {
            statement.query([])?
        };
        let mut result = vec![];
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let raw: String = row.get(1)?;
            result.push((id, parse_json(raw.as_bytes(), 8 * MAX_BYTES)?));
        }
        Ok(result)
    }
}
pub struct State {
    pub source: PathBuf,
    pub state: PathBuf,
    pub db: Database,
    pub host: Arc<dyn SessionBridge>,
}
impl State {
    pub fn project(&self, id: &str) -> Result<Project> {
        self.host
            .projects()?
            .remove(id)
            .ok_or_else(|| RuntimeError::not_found("Unknown registered project"))
    }
    pub fn allowed(&self) -> Result<Vec<PathBuf>> {
        self.host
            .projects()?
            .into_values()
            .map(|project| absolute(&project.path))
            .collect()
    }
    pub fn folder(&self, id: &str, folder: &str) -> Result<PathBuf> {
        let project = self.project(id)?;
        let root = absolute(&project.path)?;
        let path = if folder == "." {
            root.clone()
        } else {
            if folder.is_empty()
                || folder.starts_with('/')
                || folder.contains('\\')
                || folder
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..")
            {
                return Err(fail("Choose ordinary relative service folder"));
            }
            root.join(folder)
        };
        infra_core::bank::publication::PublicationFiles::new(&path)?;
        Ok(path)
    }
    pub fn registered(&self, path: &Path) -> Result<(String, String)> {
        let mut projects = self.host.projects()?.into_values().collect::<Vec<_>>();
        projects.sort_by_key(|project| std::cmp::Reverse(project.path.components().count()));
        for project in projects {
            let root = absolute(&project.path)?;
            if let Ok(relative) = path.strip_prefix(&root) {
                return Ok((
                    project.id,
                    if relative.as_os_str().is_empty() {
                        ".".into()
                    } else {
                        path_string(relative)?.replace('\\', "/")
                    },
                ));
            }
        }
        Err(fail("Register every service folder before editing system"))
    }
}
