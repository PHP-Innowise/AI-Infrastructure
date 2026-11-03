//! Bounded Codex exec journals, bound to native thread, workspace and canonical provider storage.
use infra_core::bank::publication::{PublicationFiles, archive::ArchiveReader};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::{Value as V, json};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
pub struct Rollout {
    pub path: PathBuf,
    reader: ArchiveReader,
    pub header_end: u64,
}
pub struct RolloutEnd {
    pub path: PathBuf,
    pub position: u64,
    pub bytes: Vec<u8>,
}
fn database_identity(
    files: &mut PublicationFiles,
    name: &str,
) -> infra_core::Result<Option<infra_core::bank::publication::FileImageIdentity>> {
    #[cfg(unix)]
    {
        files.metadata_image_identity(name)
    }
    #[cfg(not(unix))]
    {
        files.image_identity(name)
    }
}
pub fn open(native_id: &str, project: &Path) -> Option<Rollout> {
    open_inner(native_id, project).ok().flatten()
}
fn open_inner(native_id: &str, project: &Path) -> crate::Result<Option<Rollout>> {
    if !crate::delegation::identity(native_id) || !project.is_absolute() {
        return Ok(None);
    }
    let home = crate::model_catalog::codex_home();
    let mut custody = PublicationFiles::new(&home)?;
    let mut databases = custody
        .archive_entries("", 20000)?
        .into_iter()
        .filter_map(|name| {
            let number = name
                .strip_prefix("state_")?
                .strip_suffix(".sqlite")?
                .parse::<u64>()
                .ok()?;
            Some((number, name))
        })
        .collect::<Vec<_>>();
    databases.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    let mut found = None;
    for (_, name) in databases.into_iter().take(3) {
        let identity = match database_identity(&mut custody, &name) {
            Ok(Some(value)) => value,
            _ => continue,
        };
        let database = home.join(&name);
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::from_bits_retain(0x01000000);
        let db = match Connection::open_with_flags(&database, flags) {
            Ok(v) => v,
            Err(_) => continue,
        };
        db.busy_timeout(Duration::from_millis(200))?;
        unsafe {
            rusqlite::ffi::sqlite3_progress_handler(
                db.handle(),
                10000,
                Some(abort_large_query),
                std::ptr::null_mut(),
            );
        }
        let row = db
            .query_row(
                "SELECT rollout_path,cwd FROM threads WHERE id=?",
                [native_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional();
        if database_identity(&mut custody, &name).ok().flatten() != Some(identity) {
            continue;
        }
        if let Ok(Some(row)) = row {
            found = Some(row);
            break;
        }
    }
    let Some((recorded, cwd)) = found else {
        return Ok(None);
    };
    if cwd != project.to_string_lossy() {
        return Ok(None);
    }
    let path = PathBuf::from(recorded);
    if path.extension().and_then(|s| s.to_str()) != Some("jsonl")
        || !path.is_absolute()
        || std::fs::canonicalize(&path).ok().as_ref() != Some(&path)
    {
        return Ok(None);
    }
    let relative = path
        .strip_prefix(&home)
        .ok()
        .and_then(|p| p.to_str())
        .map(|s| s.replace('\\', "/"));
    let Some(relative) = relative else {
        return Ok(None);
    };
    if !relative.starts_with("sessions/") && !relative.starts_with("archived_sessions/") {
        return Ok(None);
    }
    if relative
        .split('/')
        .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Ok(None);
    }
    let Some(mut reader) = PublicationFiles::new(&home)?.archive_reader(&relative)? else {
        return Ok(None);
    };
    let first = reader.prefix_line(65536)?;
    let meta = crate::strict_json::parse(&first, 65536).unwrap_or(V::Null);
    if meta["type"] != "session_meta"
        || meta["payload"]["id"] != native_id
        || meta["payload"]["cwd"] != cwd
        || meta["payload"]["source"] != "exec"
    {
        return Ok(None);
    }
    Ok(Some(Rollout {
        path,
        reader,
        header_end: first.len() as u64,
    }))
}
extern "C" fn abort_large_query(_: *mut std::ffi::c_void) -> i32 {
    1
}
impl Rollout {
    pub fn read(&mut self, offset: u64, limit: usize) -> crate::Result<Vec<u8>> {
        Ok(self.reader.read(offset, limit)?)
    }
    pub fn identity(&self) -> (u64, u64, u32) {
        self.reader.identity()
    }
    pub fn end(&mut self) -> crate::Result<RolloutEnd> {
        let (position, bytes) = self.reader.tail(self.header_end, 4 * 1024 * 1024)?;
        Ok(RolloutEnd {
            path: self.path.clone(),
            position,
            bytes,
        })
    }
    pub fn check(&self) -> crate::Result<()> {
        Ok(self.reader.check()?)
    }
}
pub fn end(native_id: &str, project: &Path) -> Option<RolloutEnd> {
    open(native_id, project)?.end().ok()
}
pub fn tail(native_id: &str, project: &Path) -> Option<Vec<u8>> {
    end(native_id, project).map(|v| v.bytes)
}
pub fn journal_activity(native_id: &str, project: &Path, since: f64, until: f64) -> Vec<V> {
    let Some(raw) = tail(native_id, project) else {
        return vec![];
    };
    let mut turns = std::collections::BTreeSet::new();
    let mut activities = vec![];
    for line in raw.split(|b| *b == b'\n') {
        let Ok(record) = crate::strict_json::parse(line, 4 * 1024 * 1024) else {
            continue;
        };
        let Some(stamp) = record["timestamp"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.timestamp() as f64 + d.timestamp_subsec_nanos() as f64 / 1_000_000_000.)
        else {
            continue;
        };
        if stamp < since || stamp > until || record["type"] != "event_msg" {
            continue;
        }
        let event = &record["payload"];
        let Some(turn) = event["turn_id"].as_str() else {
            continue;
        };
        if event["type"] == "task_started" {
            turns.insert(turn.to_owned());
        }
        let item = &event["item"];
        if event["type"] == "item_completed"
            && event["thread_id"] == native_id
            && turns.contains(turn)
            && item["type"] == "SubAgentActivity"
        {
            let mut normalized = item.clone();
            normalized["type"] = json!("sub_agent_activity");
            if let Some(phase) =
                crate::delegation::codex_helper(&json!({"type":"item.completed","item":normalized}))
            {
                let row = json!({"phase":phase,"agent_id":item["agent_thread_id"]});
                if !activities.contains(&row) {
                    activities.push(row);
                }
            }
        }
    }
    activities
}
