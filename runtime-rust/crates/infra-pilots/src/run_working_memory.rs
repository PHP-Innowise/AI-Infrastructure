//! Opt-in, receipt-linked working-memory mechanics. No ordinary runner enables this module.
use crate::strict;
use anyhow::{Context, Result, bail};
use infra_core::bank::publication::PublicationFiles;
use infra_system::{common, execution, native};
use rusqlite::{Connection, OpenFlags, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub mod live;
pub mod rehearsal;

pub const SNAPSHOT_BYTES: usize = 8192;
pub const MAX_ENTRIES: usize = 12;
const MAX_RECORDS: usize = 256;
const JOURNAL_BYTES: usize = 2 * 1024 * 1024;
const MAX_TTL_MS: u64 = 86_400_000;
const JOURNAL: &str = "journal.json";
const CITATION_RACY_NS: i128 = 2_000_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileStamp {
    mtime_ns: i128,
    ctime_ns: i128,
    len: u64,
    ino: u64,
}

fn current_unix_ns() -> i128 {
    i128::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    )
    .unwrap_or(0)
}

fn file_stamp(path: &Path) -> Option<(FileStamp, i128)> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return None;
    }
    let mtime_ns = i128::try_from(
        meta.modified()
            .ok()?
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_nanos(),
    )
    .ok()?;
    #[cfg(unix)]
    let (ctime_ns, len, ino) = {
        use std::os::unix::fs::MetadataExt;
        let ctime = i128::from(meta.ctime()) * 1_000_000_000 + i128::from(meta.ctime_nsec());
        (ctime, meta.size(), meta.ino())
    };
    #[cfg(not(unix))]
    let (ctime_ns, len, ino) = (mtime_ns, meta.len(), 0u64);
    let settled = mtime_ns.max(ctime_ns);
    Some((
        FileStamp {
            mtime_ns,
            ctime_ns,
            len,
            ino,
        },
        settled,
    ))
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn source_path(path: &str) -> Result<()> {
    if path.len() > 4096
        || common::sensitive(path)
        || infra_core::domain::secret_label(path)?.is_some()
    {
        bail!("working_memory_source_path")
    }
    common::relative(path).map_err(|_| anyhow::anyhow!("working_memory_source_path"))?;
    if path.split('/').any(|part| {
        part.eq_ignore_ascii_case("memory-bank") || part.eq_ignore_ascii_case("project-brain")
    }) {
        bail!("working_memory_source")
    }
    Ok(())
}
fn selected_services(state: &Value) -> Result<Vec<String>> {
    let mut selected = std::collections::BTreeSet::new();
    for value in common::items(&state["selected"], common::MAX_SERVICES)? {
        let service = common::identifier(value.as_str().context("working_memory_selected")?)?;
        if !selected.insert(service) {
            bail!("working_memory_duplicate_service")
        }
    }
    Ok(selected.into_iter().collect())
}
fn directory_identity(path: &Path) -> Result<Vec<u64>> {
    let metadata = std::fs::metadata(path)?;
    if !metadata.is_dir() {
        bail!("working_memory_directory")
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(vec![metadata.dev(), metadata.ino()])
    }
    #[cfg(not(unix))]
    {
        Ok(Vec::new())
    }
}
fn wire<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    strict::wire(&serde_json::to_value(value)?)
}
fn hash<T: Serialize>(value: &T) -> Result<String> {
    Ok(strict::sha(&wire(value)?))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Progress,
    Hypothesis,
    OpenQuestion,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Open,
    Resolved,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Citation {
    pub service: String,
    pub path: String,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub key: String,
    pub kind: Kind,
    pub status: Status,
    pub text: String,
    pub sources: Vec<Citation>,
}
impl Observation {
    fn validate(&self) -> Result<()> {
        // Bound serialization, then screen every caller-controlled field before
        // validators whose diagnostics may include the rejected input.
        if self.key.len() > 80
            || self.text.len() > 1280
            || self.sources.is_empty()
            || self.sources.len() > 4
            || self.sources.iter().any(|source| {
                source.service.len() > 80 || source.path.len() > 4096 || source.sha256.len() != 64
            })
        {
            bail!("working_memory_claim_shape")
        }
        let claim = String::from_utf8(wire(self)?)?;
        if common::sensitive(&claim) || infra_core::domain::secret_label(&claim)?.is_some() {
            bail!("working_memory_sensitive_claim")
        }
        common::identifier(&self.key)?;
        common::text(&json!(self.text), "working-memory claim", 320)?;
        for source in &self.sources {
            source_path(&source.path)?;
            if !strict::digest(&source.sha256) {
                bail!("working_memory_source")
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Scope {
    project: String,
    selected: Vec<String>,
    workspace: String,
    workspace_identity: Vec<u64>,
    directory: String,
    run_id: String,
    generation: u64,
    roots_sha256: String,
    root_identities: BTreeMap<String, Vec<u64>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    phase: String,
    service: String,
    attempt: u64,
    dispatch_id: String,
    input_sha256: String,
    receipt_sha256: String,
    created_ms: u64,
    expires_ms: u64,
    ttl_ms: u64,
    observations: Vec<Observation>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema_version: u64,
    scope: Scope,
    records: Vec<Record>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SnapshotValidation {
    pub citations: Vec<Citation>,
    #[serde(default)]
    pub graph_ir_revisions: BTreeMap<String, i64>,
}

fn read_graph_ir_epoch(db_path: &Path) -> Option<i64> {
    let meta = std::fs::symlink_metadata(db_path).ok()?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return None;
    }
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let value: String = conn
        .query_row(
            "SELECT value FROM ir_meta WHERE key='write_epoch'",
            [],
            |row| row.get(0),
        )
        .ok()?;
    value.parse::<i64>().ok()
}

fn collect_graph_ir_revisions<'a>(
    roots: impl IntoIterator<Item = &'a Path>,
) -> BTreeMap<String, i64> {
    let candidates = [
        "graph_ir.sqlite3",
        "memory-bank/local/graph_ir.sqlite3",
        "ir.sqlite",
        "memory-bank/local/ir.sqlite",
    ];
    let mut revisions = BTreeMap::new();
    for root in roots {
        for rel in candidates {
            let db_path = root.join(rel);
            let Ok(canonical) = infra_core::fs::canonicalize(&db_path) else {
                continue;
            };
            let key = canonical.to_string_lossy().replace('\\', "/");
            if revisions.contains_key(&key) {
                continue;
            }
            if let Some(epoch) = read_graph_ir_epoch(&canonical) {
                revisions.insert(key, epoch);
            }
        }
    }
    revisions
}

/// Both adapters store a disposable, versioned view. They never acknowledge a worker result.
pub trait SnapshotStore: Send {
    fn load(&mut self, key: &str) -> Result<Option<Vec<u8>>>;
    fn publish(&mut self, key: &str, bytes: &[u8], ttl: Duration) -> Result<()>;

    fn load_validated(
        &mut self,
        key: &str,
        expected_validation: &SnapshotValidation,
    ) -> Result<Option<Vec<u8>>> {
        let _ = expected_validation;
        self.load(key)
    }

    fn publish_validated(
        &mut self,
        key: &str,
        bytes: &[u8],
        ttl: Duration,
        validation: &SnapshotValidation,
    ) -> Result<()> {
        let _ = validation;
        self.publish(key, bytes, ttl)
    }
}

pub struct SqliteStore {
    connection: Connection,
}
impl SqliteStore {
    pub fn open(directory: &Path) -> Result<Self> {
        native::private_directory(directory)?;
        let mut custody = PublicationFiles::new(directory)?;
        let name = "working-memory.sqlite3";
        #[cfg(unix)]
        let before = custody.metadata_image_identity(name)?;
        #[cfg(not(unix))]
        let before = custody.image_identity(name)?;
        if before.is_some_and(|image| image.3 != 1) {
            bail!("working_memory_cache_file")
        }
        let connection = Connection::open_with_flags(
            directory.join(name),
            OpenFlags::default() | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        #[cfg(unix)]
        let opened = custody
            .metadata_image_identity(name)?
            .context("working_memory_cache_file")?;
        #[cfg(not(unix))]
        let opened = custody
            .image_identity(name)?
            .context("working_memory_cache_file")?;
        if opened.3 != 1 || before.is_some_and(|before| before != opened) {
            bail!("working_memory_cache_file_changed")
        }
        #[cfg(unix)]
        custody.secure_private_file_metadata(name, opened)?;
        #[cfg(not(unix))]
        custody.secure_private_file(name)?;
        custody.check()?;
        connection.busy_timeout(Duration::from_millis(100))?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS snapshots(key TEXT PRIMARY KEY, value BLOB NOT NULL, expires_ms INTEGER NOT NULL);\
             CREATE TABLE IF NOT EXISTS snapshot_validations(key TEXT PRIMARY KEY, validation TEXT NOT NULL, expires_ms INTEGER NOT NULL);",
        )?;
        Ok(Self { connection })
    }
}
impl SnapshotStore for SqliteStore {
    fn load(&mut self, key: &str) -> Result<Option<Vec<u8>>> {
        let mut statement = self.connection.prepare(
            "SELECT value FROM snapshots WHERE key=?1 AND expires_ms>?2 AND length(value)<=?3",
        )?;
        let mut rows = statement.query(params![key, now_ms(), SNAPSHOT_BYTES])?;
        rows.next()?
            .map(|row| row.get(0).map_err(Into::into))
            .transpose()
    }
    fn publish(&mut self, key: &str, bytes: &[u8], ttl: Duration) -> Result<()> {
        if bytes.len() > SNAPSHOT_BYTES {
            bail!("working_memory_snapshot_bound")
        }
        self.connection
            .execute("DELETE FROM snapshots WHERE expires_ms<=?1", [now_ms()])?;
        self.connection.execute(
            "INSERT OR REPLACE INTO snapshots(key,value,expires_ms) VALUES(?1,?2,?3)",
            params![key, bytes, now_ms().saturating_add(ttl.as_millis() as u64)],
        )?;
        Ok(())
    }
    fn load_validated(
        &mut self,
        key: &str,
        expected_validation: &SnapshotValidation,
    ) -> Result<Option<Vec<u8>>> {
        let now = now_ms();
        let mut statement = self.connection.prepare(
            "SELECT s.value, v.validation FROM snapshots s \
             LEFT JOIN snapshot_validations v ON v.key = s.key AND v.expires_ms > ?2 \
             WHERE s.key=?1 AND s.expires_ms>?2 AND length(s.value)<=?3",
        )?;
        let mut rows = statement.query(params![key, now, SNAPSHOT_BYTES])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let bytes: Vec<u8> = row.get(0)?;
        let validation_raw: Option<String> = row.get(1)?;
        drop(rows);
        drop(statement);
        let valid = validation_raw
            .as_deref()
            .and_then(|raw| serde_json::from_str::<SnapshotValidation>(raw).ok())
            .is_some_and(|stored| &stored == expected_validation);
        if !valid {
            let _ = self
                .connection
                .execute("DELETE FROM snapshots WHERE key=?1", [key]);
            let _ = self
                .connection
                .execute("DELETE FROM snapshot_validations WHERE key=?1", [key]);
            return Ok(None);
        }
        Ok(Some(bytes))
    }
    fn publish_validated(
        &mut self,
        key: &str,
        bytes: &[u8],
        ttl: Duration,
        validation: &SnapshotValidation,
    ) -> Result<()> {
        if bytes.len() > SNAPSHOT_BYTES {
            bail!("working_memory_snapshot_bound")
        }
        let now = now_ms();
        let expires_ms = now.saturating_add(ttl.as_millis() as u64);
        let validation_raw = serde_json::to_string(validation)?;
        let tx = self.connection.unchecked_transaction()?;
        tx.execute("DELETE FROM snapshots WHERE expires_ms<=?1", [now])?;
        tx.execute(
            "DELETE FROM snapshot_validations WHERE expires_ms<=?1",
            [now],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO snapshots(key,value,expires_ms) VALUES(?1,?2,?3)",
            params![key, bytes, expires_ms],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO snapshot_validations(key,validation,expires_ms) VALUES(?1,?2,?3)",
            params![key, validation_raw, expires_ms],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(feature = "run-memory-redis")]
pub struct RedisStore {
    client: redis::Client,
    timeout: Duration,
}
#[cfg(feature = "run-memory-redis")]
impl RedisStore {
    /// Pilot transport deliberately accepts only a numeric loopback endpoint, without credentials.
    pub fn loopback(endpoint: std::net::SocketAddr) -> Result<Self> {
        if !endpoint.ip().is_loopback() || endpoint.port() == 0 {
            bail!("working_memory_redis_endpoint")
        }
        let client = redis::Client::open(format!("redis://{endpoint}/"))
            .map_err(|_| anyhow::anyhow!("working_memory_redis_config"))?;
        Ok(Self {
            client,
            timeout: Duration::from_millis(100),
        })
    }
    fn connection(&self) -> Result<redis::Connection> {
        let connection = self
            .client
            .get_connection_with_timeout(self.timeout)
            .map_err(|_| anyhow::anyhow!("working_memory_redis_unavailable"))?;
        connection
            .set_read_timeout(Some(self.timeout))
            .map_err(|_| anyhow::anyhow!("working_memory_redis_unavailable"))?;
        connection
            .set_write_timeout(Some(self.timeout))
            .map_err(|_| anyhow::anyhow!("working_memory_redis_unavailable"))?;
        Ok(connection)
    }
}
#[cfg(feature = "run-memory-redis")]
impl SnapshotStore for RedisStore {
    fn load(&mut self, key: &str) -> Result<Option<Vec<u8>>> {
        // Bound the response atomically on the server; an oversized value is a miss/fallback.
        redis::cmd("EVAL").arg("local v=redis.call('GET',KEYS[1]); if v and string.len(v)>8192 then return redis.error_reply('snapshot_bound') end; return v")
            .arg(1).arg(key).query(&mut self.connection()?).map_err(|_| anyhow::anyhow!("working_memory_redis_read"))
    }
    fn publish(&mut self, key: &str, bytes: &[u8], ttl: Duration) -> Result<()> {
        if bytes.len() > SNAPSHOT_BYTES {
            bail!("working_memory_snapshot_bound")
        }
        redis::cmd("SET")
            .arg(key)
            .arg(bytes)
            .arg("PX")
            .arg(ttl.as_millis().max(1) as u64)
            .query::<()>(&mut self.connection()?)
            .map_err(|_| anyhow::anyhow!("working_memory_redis_write"))
    }
    fn load_validated(
        &mut self,
        key: &str,
        expected_validation: &SnapshotValidation,
    ) -> Result<Option<Vec<u8>>> {
        let Some(bytes) = self.load(key)? else {
            return Ok(None);
        };
        let meta_key = format!("{key}:validation");
        let mut conn = self.connection()?;
        let meta_raw: Option<Vec<u8>> = redis::cmd("GET")
            .arg(&meta_key)
            .query(&mut conn)
            .map_err(|_| anyhow::anyhow!("working_memory_redis_read"))?;
        let valid = meta_raw
            .as_deref()
            .and_then(|raw| serde_json::from_slice::<SnapshotValidation>(raw).ok())
            .is_some_and(|stored| &stored == expected_validation);
        if !valid {
            let _ = redis::cmd("DEL")
                .arg(key)
                .arg(&meta_key)
                .query::<()>(&mut conn);
            return Ok(None);
        }
        Ok(Some(bytes))
    }
    fn publish_validated(
        &mut self,
        key: &str,
        bytes: &[u8],
        ttl: Duration,
        validation: &SnapshotValidation,
    ) -> Result<()> {
        if bytes.len() > SNAPSHOT_BYTES {
            bail!("working_memory_snapshot_bound")
        }
        let ttl_ms = ttl.as_millis().max(1) as u64;
        let validation_bytes = serde_json::to_vec(validation)?;
        let meta_key = format!("{key}:validation");
        let mut conn = self.connection()?;
        redis::pipe()
            .atomic()
            .cmd("SET")
            .arg(key)
            .arg(bytes)
            .arg("PX")
            .arg(ttl_ms)
            .ignore()
            .cmd("SET")
            .arg(&meta_key)
            .arg(validation_bytes)
            .arg("PX")
            .arg(ttl_ms)
            .ignore()
            .query::<()>(&mut conn)
            .map_err(|_| anyhow::anyhow!("working_memory_redis_write"))
    }
}

#[derive(Debug)]
pub struct SnapshotRead {
    pub snapshot: Value,
    pub backend: &'static str,
    pub key: String,
}
#[derive(Debug)]
pub struct AppendReceipt {
    pub accepted: bool,
    pub replayed: bool,
    pub projection: &'static str,
}

/// Journal acceptance and current-source admission live here, behind the same interface for both stores.
pub struct RunWorkingMemory {
    directory: PathBuf,
    run_directory: PathBuf,
    scope: Scope,
    roots: BTreeMap<String, PathBuf>,
    store: Box<dyn SnapshotStore>,
    citation_cache: BTreeMap<(String, String), (FileStamp, Citation)>,
}
impl RunWorkingMemory {
    pub fn open(
        run_directory: &Path,
        state: &Value,
        workspace: &Path,
        generation: u64,
        store: Box<dyn SnapshotStore>,
    ) -> Result<Self> {
        if generation == 0 {
            bail!("working_memory_generation")
        }
        let run_directory = infra_core::fs::canonicalize(run_directory)?;
        let workspace = infra_core::fs::canonicalize(workspace)?;
        common::text(&state["run_id"], "run identity", 100)?;
        let project = native::working_memory_project_id(state)?;
        let selected = selected_services(state)?;
        let mut roots = BTreeMap::new();
        for service in std::iter::once("__system__".to_string()).chain(selected.clone()) {
            let path = state["roots"][&service]
                .as_str()
                .context("working_memory_root")?;
            let canonical = infra_core::fs::canonicalize(path)?;
            if canonical != Path::new(path) {
                bail!("working_memory_noncanonical_root")
            }
            roots.insert(service, canonical);
        }
        #[cfg(unix)]
        let workspace_identity = {
            use std::os::unix::fs::MetadataExt;
            let m = std::fs::metadata(&workspace)?;
            vec![m.dev(), m.ino()]
        };
        #[cfg(not(unix))]
        let workspace_identity = Vec::new();
        let scope = Scope {
            project,
            selected,
            workspace: common::path_string(&workspace)?,
            workspace_identity,
            directory: common::path_string(&run_directory)?,
            run_id: state["run_id"].as_str().unwrap().into(),
            generation,
            roots_sha256: hash(&state["roots"])?,
            root_identities: roots
                .iter()
                .map(|(id, root)| Ok((id.clone(), directory_identity(root)?)))
                .collect::<Result<_>>()?,
        };
        let directory = run_directory.join("working-memory-pilot");
        native::private_directory(&directory)?;
        let memory = Self {
            directory,
            run_directory,
            scope,
            roots,
            store,
            citation_cache: BTreeMap::new(),
        };
        let _lock = execution::lock(&memory.directory, ".journal.lock")?;
        memory.journal()?;
        Ok(memory)
    }
    fn check_scope(&self, state: &Value) -> Result<()> {
        if state["run_id"] != self.scope.run_id
            || native::working_memory_project_id(state)? != self.scope.project
            || selected_services(state)? != self.scope.selected
            || hash(&state["roots"])? != self.scope.roots_sha256
        {
            bail!("working_memory_scope")
        }
        for (id, root) in &self.roots {
            if infra_core::fs::canonicalize(root)? != *root
                || self.scope.root_identities.get(id) != Some(&directory_identity(root)?)
            {
                bail!("working_memory_root_changed")
            }
        }
        let workspace = infra_core::fs::canonicalize(&self.scope.workspace)?;
        if workspace != Path::new(&self.scope.workspace) {
            bail!("working_memory_workspace")
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let m = std::fs::metadata(workspace)?;
            if self.scope.workspace_identity != [m.dev(), m.ino()] {
                bail!("working_memory_workspace")
            }
        }
        Ok(())
    }
    fn journal(&self) -> Result<Journal> {
        let mut custody = PublicationFiles::new(&self.directory)?;
        let Some(raw) = custody.read_checked(JOURNAL, JOURNAL_BYTES, |_| Ok(()))? else {
            return Ok(Journal {
                schema_version: 1,
                scope: self.scope.clone(),
                records: vec![],
            });
        };
        let value = strict::decode(&raw, JOURNAL_BYTES, 24, 65536)?;
        let journal: Journal = serde_json::from_value(value)
            .map_err(|_| anyhow::anyhow!("working_memory_journal_schema"))?;
        if journal.schema_version != 1
            || journal.scope != self.scope
            || journal.records.len() > MAX_RECORDS
        {
            bail!("working_memory_journal_scope")
        }
        for record in &journal.records {
            if record.observations.len() > MAX_ENTRIES
                || record.ttl_ms == 0
                || record.ttl_ms > MAX_TTL_MS
                || record.created_ms.checked_add(record.ttl_ms) != Some(record.expires_ms)
                || !strict::digest(&record.receipt_sha256)
            {
                bail!("working_memory_journal_record")
            }
            for observation in &record.observations {
                observation.validate()?;
            }
        }
        Ok(journal)
    }
    pub fn citation(&self, service: &str, path: &str) -> Result<Citation> {
        let root = self
            .roots
            .get(service)
            .context("working_memory_source_scope")?;
        source_path(path)?;
        let raw = common::read_file(root, path, common::MAX_SOURCE_BYTES)?;
        let text = std::str::from_utf8(&raw).context("working_memory_source_encoding")?;
        if text.contains('\0')
            || common::sensitive(text)
            || infra_core::domain::secret_label(text)?.is_some()
        {
            bail!("working_memory_sensitive_source")
        }
        Ok(Citation {
            service: service.into(),
            path: path.into(),
            sha256: strict::sha(&raw),
        })
    }
    fn citation_cached_with_now(
        &mut self,
        service: &str,
        path: &str,
        now_ns: i128,
    ) -> Result<Citation> {
        let root = self
            .roots
            .get(service)
            .context("working_memory_source_scope")?;
        source_path(path)?;
        let target = root.join(path);
        let stamp_info = file_stamp(&target);
        let cache_key = (service.to_string(), path.to_string());
        if let Some((stamp, settled_ns)) = &stamp_info
            && now_ns - *settled_ns >= CITATION_RACY_NS
            && let Some((cached_stamp, cached_citation)) = self.citation_cache.get(&cache_key)
            && cached_stamp == stamp
        {
            return Ok(cached_citation.clone());
        }
        let fresh = self.citation(service, path)?;
        if let Some((stamp, settled_ns)) = stamp_info
            && now_ns - settled_ns >= CITATION_RACY_NS
        {
            self.citation_cache
                .insert(cache_key, (stamp, fresh.clone()));
        }
        Ok(fresh)
    }
    pub fn append(
        &mut self,
        state: &Value,
        step: &Value,
        observations: &[Observation],
        ttl: Duration,
    ) -> Result<AppendReceipt> {
        self.append_at(state, step, observations, ttl, now_ms())
    }
    fn append_at(
        &mut self,
        state: &Value,
        step: &Value,
        observations: &[Observation],
        ttl: Duration,
        at: u64,
    ) -> Result<AppendReceipt> {
        self.check_scope(state)?;
        let ttl_ms = u64::try_from(ttl.as_millis())?;
        if ttl_ms == 0
            || ttl_ms > MAX_TTL_MS
            || observations.len() > MAX_ENTRIES
            || state["status"] == "completed"
            || step["status"] != "completed"
        {
            bail!("working_memory_append")
        }
        if !state["steps"]
            .as_array()
            .is_some_and(|steps| steps.contains(step))
            || !self.roots.contains_key(common::string(&step["service"]))
        {
            bail!("working_memory_dispatch")
        }
        let receipt = execution::dispatch_receipt(&self.run_directory, state, step)?;
        if !execution::completed_receipt(&receipt, step) {
            bail!("working_memory_receipt")
        }
        let receipt_sha256 = hash(&receipt)?;
        for observation in observations {
            observation.validate()?;
        }
        let lock = execution::lock(&self.directory, ".journal.lock")?;
        let mut journal = self.journal()?;
        let replayed = if let Some(previous) = journal
            .records
            .iter()
            .find(|record| record.dispatch_id == step["dispatch_id"])
        {
            if previous.receipt_sha256 != receipt_sha256
                || previous.observations != observations
                || previous.ttl_ms != ttl_ms
            {
                bail!("working_memory_conflicting_replay")
            }
            true
        } else {
            if journal.records.len() >= MAX_RECORDS {
                bail!("working_memory_journal_bound")
            }
            let mut keys = std::collections::BTreeSet::new();
            for observation in observations {
                if !keys.insert(&observation.key) {
                    bail!("working_memory_duplicate_topic")
                }
                for source in &observation.sources {
                    if self.citation(&source.service, &source.path)? != *source {
                        bail!("working_memory_stale_source")
                    }
                }
            }
            journal.records.push(Record {
                phase: common::string(&step["id"]).into(),
                service: common::string(&step["service"]).into(),
                attempt: step["attempt"].as_u64().context("working_memory_attempt")?,
                dispatch_id: common::string(&step["dispatch_id"]).into(),
                input_sha256: common::string(&step["input_sha256"]).into(),
                receipt_sha256,
                created_ms: at,
                expires_ms: at.checked_add(ttl_ms).context("working_memory_expiry")?,
                ttl_ms,
                observations: observations.to_vec(),
            });
            let raw = wire(&journal)?;
            if raw.len() > JOURNAL_BYTES {
                bail!("working_memory_journal_bound")
            }
            strict::decode(&raw, JOURNAL_BYTES, 24, 65536)?;
            let mut files = PublicationFiles::new(&self.directory)?;
            let before = files.read_checked(JOURNAL, JOURNAL_BYTES, |_| Ok(()))?;
            files.write_expected_limit(JOURNAL, &raw, before.as_deref(), JOURNAL_BYTES)?;
            false
        };
        drop(lock);
        // Acceptance has already committed. Cache publication cannot undo it.
        let projection = self
            .snapshot_at(state, at)
            .map(|read| read.backend)
            .unwrap_or("projection_unavailable");
        Ok(AppendReceipt {
            accepted: true,
            replayed,
            projection,
        })
    }
    pub fn snapshot(&mut self, state: &Value) -> Result<SnapshotRead> {
        self.snapshot_at(state, now_ms())
    }
    fn snapshot_at(&mut self, state: &Value, at: u64) -> Result<SnapshotRead> {
        self.check_scope(state)?;
        let lock = execution::lock(&self.directory, ".journal.lock")?;
        let journal = self.journal()?;
        drop(lock);
        let mut latest = BTreeMap::new();
        let mut receipt_valid = BTreeMap::new();
        for (sequence, record) in journal.records.iter().enumerate() {
            let Some(position) = state["steps"]
                .as_array()
                .and_then(|steps| steps.iter().position(|step| step["id"] == record.phase))
            else {
                continue;
            };
            let step = state["steps"].as_array().and_then(|steps| {
                steps.iter().find(|step| {
                    step["id"] == record.phase
                        && step["status"] == "completed"
                        && step["attempt"] == record.attempt
                        && step["dispatch_id"] == record.dispatch_id
                        && step["input_sha256"] == record.input_sha256
                        && step["service"] == record.service
                })
            });
            let valid = step.is_some_and(|step| {
                execution::dispatch_receipt(&self.run_directory, state, step)
                    .ok()
                    .is_some_and(|receipt| {
                        execution::completed_receipt(&receipt, step)
                            && hash(&receipt).ok().as_deref() == Some(&record.receipt_sha256)
                    })
            });
            receipt_valid.insert(sequence, valid);
            // Receipt chronology wins over arrival order. A delayed first delivery
            // from an older dispatch cannot reopen a topic resolved by a later one.
            let priority = (position, record.attempt, sequence);
            for observation in &record.observations {
                let key = (record.service.clone(), observation.key.clone());
                if latest
                    .get(&key)
                    .is_none_or(|(previous, _, _, _)| priority > *previous)
                {
                    latest.insert(key, (priority, sequence, record, observation));
                }
            }
        }
        let count = latest.len();
        let mut entries = vec![];
        let mut cited = BTreeMap::<(String, String), Citation>::new();
        let now_ns = current_unix_ns();
        let mut until = at.saturating_add(1000);
        if state["status"] != "completed" {
            let mut rows = latest.into_values().collect::<Vec<_>>();
            rows.sort_by_key(|(priority, _, _, observation)| {
                (std::cmp::Reverse(*priority), observation.key.clone())
            });
            for (_, sequence, record, observation) in rows {
                if !receipt_valid[&sequence] || record.created_ms > at || record.expires_ms <= at {
                    continue;
                }
                let mut current_sources = Vec::with_capacity(observation.sources.len());
                let mut stale = false;
                for source in &observation.sources {
                    match self.citation_cached_with_now(&source.service, &source.path, now_ns) {
                        Ok(current) if &current == source => current_sources.push(current),
                        _ => {
                            stale = true;
                            break;
                        }
                    }
                }
                if stale {
                    continue;
                }
                if entries.len() >= MAX_ENTRIES {
                    continue;
                }
                for current in current_sources {
                    cited.insert((current.service.clone(), current.path.clone()), current);
                }
                until = if entries.is_empty() {
                    record.expires_ms
                } else {
                    until.min(record.expires_ms)
                };
                entries.push(json!({"service":record.service,"key":observation.key,"kind":observation.kind,"status":observation.status,"text":observation.text,"sources":observation.sources,"receipt":common::path_string(&self.run_directory.join(common::receipt_name(&json!({"id":record.phase,"attempt":record.attempt}))))?,"receipt_sha256":record.receipt_sha256,"dispatch_id":record.dispatch_id,"input_sha256":record.input_sha256,"expires_ms":record.expires_ms}));
            }
        }
        let mut snapshot = json!({"schema_version":1,"kind":"run-working-memory","run_id":self.scope.run_id,"project_id":self.scope.project,"selected_sha256":hash(&self.scope.selected)?,"roots_sha256":self.scope.roots_sha256,"generation":self.scope.generation,"entries":entries,"omitted":count.saturating_sub(entries.len()),"untrusted":true});
        while wire(&snapshot)?.len() > SNAPSHOT_BYTES {
            snapshot["entries"]
                .as_array_mut()
                .context("working_memory_snapshot")?
                .pop()
                .context("working_memory_snapshot_bound")?;
            snapshot["omitted"] = json!(snapshot["omitted"].as_u64().unwrap() + 1);
        }
        let expected = wire(&snapshot)?;
        let key = format!(
            "infra:pilot:run-memory:v1:{}:{}",
            hash(&self.scope)?,
            strict::sha(&expected)
        );
        let graph_ir_revisions = collect_graph_ir_revisions(
            std::iter::once(self.run_directory.as_path())
                .chain(self.roots.values().map(PathBuf::as_path)),
        );
        let validation = SnapshotValidation {
            citations: cited.into_values().collect(),
            graph_ir_revisions,
        };
        // Canonical replay and current sources determine the exact allowed view.
        // Backend bytes can never omit/replace a claim or bypass source admission.
        let backend = match self.store.load_validated(&key, &validation) {
            Ok(Some(raw)) if raw == expected => "hit",
            _ => {
                if self
                    .store
                    .publish_validated(
                        &key,
                        &expected,
                        Duration::from_millis(until.saturating_sub(at).clamp(1, MAX_TTL_MS)),
                        &validation,
                    )
                    .is_ok()
                {
                    "rebuilt"
                } else {
                    "local_fallback"
                }
            }
        };
        Ok(SnapshotRead {
            snapshot,
            backend,
            key,
        })
    }
    pub fn prompt(
        &mut self,
        state: &Value,
        plan: &Value,
        step: &Value,
        prior: &[Value],
    ) -> Result<(String, SnapshotRead)> {
        let read = self.snapshot(state)?;
        let prompt =
            native::prompt_for_with_working_memory(state, plan, step, prior, None, &read.snapshot)?;
        Ok((prompt, read))
    }
}

#[cfg(test)]
mod tests;
