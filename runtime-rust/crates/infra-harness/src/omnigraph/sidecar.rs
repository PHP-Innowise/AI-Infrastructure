//! Out-of-process Omnigraph sidecar/Docker/external lifecycle manager (`OmnigraphManager`)
//! and Harness service domain (`OmnigraphDomain`).

mod context_sync;
mod domain;
mod graph_ops;
mod helpers;
mod lifecycle;

pub use domain::OmnigraphDomain;
pub(crate) use helpers::format_atom_line_with_authority;
#[cfg(test)]
pub(super) use helpers::format_context_capsule_for_test;
pub(super) use helpers::{detect_language, normalize_relation_name, pascal_to_snake};
pub use lifecycle::BinaryDiscovery;

use super::client::OmnigraphClient;
use super::schema::{self, HARNESS_GRAPH_ID, OMNIGRAPH_DOCKER_IMAGE, OMNIGRAPH_VERSION};
use crate::{Result, store::SessionStore};
use lifecycle::{load_config_file, save_config_file, terminate_child_sync};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

const CONFIG_FILE_NAME: &str = "omnigraph.json";
const MAX_TARBALL_BYTES: usize = 512 * 1024 * 1024;
const MAX_EXPANDED_BYTES: usize = 1024 * 1024 * 1024;
const MAX_ARCHIVE_FILES: usize = 64;
const STDERR_RING_MAX_LINES: usize = 128;
const STDERR_RING_MAX_BYTES: usize = 32 * 1024;
const STDERR_LINE_MAX_CHARS: usize = 512;
const STATS_CACHE_TTL: Duration = Duration::from_secs(10);
const VALID_ATOM_KINDS: &[&str] = &[
    "finding", "decision", "warning", "contract", "insight", "artifact", "question",
];
const VALID_RELATIONS: &[&str] = &[
    "supports",
    "contradicts",
    "supersedes",
    "depends_on",
    "refines",
];
/// At most this many declarations are linked to one atom, read from at most
/// this many of the files it touches.
const MAX_ATOM_SYMBOLS: usize = 12;
const MAX_SYMBOL_FILES: usize = 8;
const DEFAULT_GRAPH_ATOM_LIMIT: usize = 250;
const MAX_GRAPH_ATOM_LIMIT: usize = 1000;
const COMMIT_DIGEST_PREFETCH_COUNT: usize = 16;
const COMMIT_DIGEST_CONCURRENCY: usize = 8;

/// Operating mode for Harness Omnigraph integration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OmnigraphMode {
    #[default]
    Off,
    LocalSidecar,
    Docker,
    External,
}

impl OmnigraphMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::LocalSidecar => "local_sidecar",
            Self::Docker => "docker",
            Self::External => "external",
        }
    }
}

/// Persisted configuration stored at `<state>/omnigraph.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OmnigraphConfig {
    #[serde(default)]
    pub mode: OmnigraphMode,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default = "default_graph_id")]
    pub graph_id: String,
    #[serde(default = "default_actor_id")]
    pub actor_id: String,
    #[serde(default)]
    pub bind_port: Option<u16>,
    #[serde(default = "default_docker_image")]
    pub docker_image: String,
    #[serde(default)]
    pub container_name: Option<String>,
    #[serde(default)]
    pub auto_start: bool,
}

fn default_graph_id() -> String {
    HARNESS_GRAPH_ID.to_string()
}

fn default_actor_id() -> String {
    "harness".to_string()
}

fn default_docker_image() -> String {
    OMNIGRAPH_DOCKER_IMAGE.to_string()
}

impl Default for OmnigraphConfig {
    fn default() -> Self {
        Self {
            mode: OmnigraphMode::Off,
            endpoint: None,
            token: None,
            graph_id: default_graph_id(),
            actor_id: default_actor_id(),
            bind_port: None,
            docker_image: default_docker_image(),
            container_name: None,
            auto_start: false,
        }
    }
}

#[derive(Default)]
struct ProcessHandle {
    child: Option<Child>,
    docker_container: Option<String>,
}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            terminate_child_sync(&mut child);
        }
        if let Some(container) = self.docker_container.take() {
            let _ = Command::new("docker")
                .args(["stop", "-t", "2", &container])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

#[derive(Default)]
struct StderrRingBuffer {
    lines: VecDeque<String>,
    total_bytes: usize,
}

impl StderrRingBuffer {
    fn clear(&mut self) {
        self.lines.clear();
        self.total_bytes = 0;
    }

    fn push_line(&mut self, raw: &str) {
        let trimmed = raw.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            return;
        }
        let line: String = if trimmed.chars().count() > STDERR_LINE_MAX_CHARS {
            let prefix: String = trimmed.chars().take(STDERR_LINE_MAX_CHARS - 1).collect();
            format!("{prefix}…")
        } else {
            trimmed.to_string()
        };
        self.total_bytes += line.len();
        self.lines.push_back(line);
        while self.lines.len() > STDERR_RING_MAX_LINES || self.total_bytes > STDERR_RING_MAX_BYTES {
            let Some(dropped) = self.lines.pop_front() else {
                self.total_bytes = 0;
                break;
            };
            self.total_bytes = self.total_bytes.saturating_sub(dropped.len());
        }
    }

    fn tail(&self, max_lines: usize) -> Vec<String> {
        let skip = self.lines.len().saturating_sub(max_lines);
        self.lines.iter().skip(skip).cloned().collect()
    }
}

struct StartingGuard {
    flag: Arc<AtomicBool>,
}

impl StartingGuard {
    fn enter(flag: &Arc<AtomicBool>) -> Self {
        flag.store(true, Ordering::Release);
        Self {
            flag: Arc::clone(flag),
        }
    }
}

impl Drop for StartingGuard {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::Release);
    }
}

#[derive(Clone, Debug)]
struct CachedStats {
    endpoint: String,
    graph_id: String,
    project_id: String,
    branches_sig: String,
    computed_at: Instant,
    atoms: usize,
    sessions: usize,
    symbols: usize,
    edges: usize,
    commits: usize,
}

#[derive(Default)]
struct RuntimeState {
    config: OmnigraphConfig,
    last_error: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct CommitDigest {
    added_count: usize,
    updated_count: usize,
    deleted_count: usize,
    atom_delta: isize,
    summary: String,
}

/// Manages Omnigraph lifecycle, config persistence, and high-level Harness context operations.
pub struct OmnigraphManager {
    state_dir: PathBuf,
    store: Option<SessionStore>,
    http: reqwest::Client,
    runtime: Mutex<RuntimeState>,
    starting: Arc<AtomicBool>,
    process: Arc<StdMutex<ProcessHandle>>,
    stderr_ring: Arc<StdMutex<StderrRingBuffer>>,
    commit_cache: Arc<StdMutex<BTreeMap<String, CommitDigest>>>,
    stats_cache: Arc<StdMutex<Option<CachedStats>>>,
}

impl OmnigraphManager {
    /// Construct a manager using `state_dir` for `<state>/omnigraph.json`, tools, and cluster data.
    pub fn new(state_dir: PathBuf, store: Option<SessionStore>) -> Result<Self> {
        let config = load_config_file(&state_dir.join(CONFIG_FILE_NAME))?;
        let http = OmnigraphClient::build_http_client(Duration::from_secs(30))?;
        Ok(Self {
            state_dir,
            store,
            http,
            runtime: Mutex::new(RuntimeState {
                config,
                last_error: None,
            }),
            starting: Arc::new(AtomicBool::new(false)),
            process: Arc::new(StdMutex::new(ProcessHandle::default())),
            stderr_ring: Arc::new(StdMutex::new(StderrRingBuffer::default())),
            commit_cache: Arc::new(StdMutex::new(BTreeMap::new())),
            stats_cache: Arc::new(StdMutex::new(None)),
        })
    }

    /// Construct a manager attached to an existing `SessionStore`.
    pub fn with_store(store: SessionStore) -> Result<Self> {
        let state_dir = store.state.clone();
        Self::new(state_dir, Some(store))
    }

    /// Path to persisted `<state>/omnigraph.json`.
    pub fn config_path(&self) -> PathBuf {
        self.state_dir.join(CONFIG_FILE_NAME)
    }

    /// Path to managed v0.15.0 binary installation directory (`<state>/tools/omnigraph/v0.15.0/`).
    pub fn tools_dir(&self) -> PathBuf {
        self.state_dir
            .join("tools")
            .join("omnigraph")
            .join(OMNIGRAPH_VERSION)
    }

    /// Path to materialized cluster configuration (`cluster.yaml`, `.pg`, `.gq`, policies).
    pub fn cluster_config_dir(&self) -> PathBuf {
        self.state_dir.join("omnigraph").join("config")
    }

    /// Path to cluster storage data directory.
    pub fn cluster_data_dir(&self) -> PathBuf {
        self.state_dir.join("omnigraph").join("data")
    }

    /// Read a snapshot of the current persisted configuration.
    pub async fn config(&self) -> OmnigraphConfig {
        self.runtime.lock().await.config.clone()
    }

    /// Save updated configuration to `<state>/omnigraph.json` and update in-memory state.
    pub async fn save_config(&self, config: OmnigraphConfig) -> Result<()> {
        schema::validate_graph_id(&config.graph_id)?;
        save_config_file(&self.config_path(), &config)?;
        let mut rt = self.runtime.lock().await;
        rt.config = config;
        Ok(())
    }

    /// Snapshot bounded stderr lines captured from the local `omnigraph-server` sidecar.
    pub fn stderr_tail(&self, max_lines: usize) -> Vec<String> {
        self.stderr_ring
            .lock()
            .ok()
            .map(|ring| ring.tail(max_lines))
            .unwrap_or_default()
    }

    /// Clear cached `/status` graph statistics after mutations or shutdown.
    pub fn invalidate_stats_cache(&self) {
        if let Ok(mut guard) = self.stats_cache.lock() {
            *guard = None;
        }
    }

    #[cfg(test)]
    pub(super) fn enter_starting_for_test(&self) -> impl Drop {
        StartingGuard::enter(&self.starting)
    }

    #[cfg(test)]
    pub(super) fn push_stderr_for_test(&self, line: &str) {
        if let Ok(mut ring) = self.stderr_ring.lock() {
            ring.push_line(line);
        }
    }
}
