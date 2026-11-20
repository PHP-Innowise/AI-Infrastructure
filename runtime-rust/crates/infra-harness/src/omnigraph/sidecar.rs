//! Out-of-process Omnigraph sidecar/Docker/external lifecycle manager (`OmnigraphManager`)
//! and Harness service domain (`OmnigraphDomain`).

use super::{
    client::{
        BranchCreateRequest, BranchDiffQuery, BranchMergeRequest, ChangeBaselineRequest,
        ChangeFeedQuery, ChangeOp, ChangeRequest, CommitListQuery, CommitOutput, EntityKind,
        MergeConflict, MergeConflictKind, OmnigraphClient, QueryRequest, StoredQueryRequest,
    },
    schema::{
        self, CANONICAL_KNOWLEDGE_BRANCH, GQ_CLEAR_ATOM_SYMBOLS, GQ_CLEAR_BRANCH_CONTEXT,
        GQ_DELETE_CANONICAL_ATOM, GQ_LINK_CONSUMED_BY, GQ_LINK_DERIVED_FROM,
        GQ_LINK_MENTIONS_SYMBOL, GQ_LINK_PUBLISHED_BY, GQ_LINK_RELATES_TO, GQ_LINK_SESSION_FOLLOWS,
        GQ_LINK_TOUCHES_FILE, GQ_LIST_ATOM_RELATIONS, GQ_LIST_CONSUMED_EDGES, GQ_LIST_FILE_EDGES,
        GQ_LIST_FILES, GQ_LIST_PUBLISHED_EDGES, GQ_LIST_SESSION_FOLLOWS,
        GQ_LIST_SESSION_TURN_SNAPSHOTS, GQ_LIST_SESSIONS, GQ_LIST_SYMBOL_EDGES, GQ_LIST_SYMBOLS,
        GQ_UPSERT_ATOM, GQ_UPSERT_FILE, GQ_UPSERT_SESSION, GQ_UPSERT_SYMBOL,
        GQ_UPSERT_TURN_SNAPSHOT, HARNESS_GRAPH_ID, OMNIGRAPH_DOCKER_IMAGE,
        OMNIGRAPH_RELEASE_URL_LINUX_X86_64, OMNIGRAPH_VERSION, RECENT_ATOMS_LIMIT,
    },
    session_branch_name,
    transfer::{
        self, BUNDLE_FORMAT_V1, filter_graph_for_export, filter_ndjson_excluding_atoms,
        normalize_link_target_branch, normalize_shared_space_branch, parse_bool_param,
        parse_i64_opt, parse_import_body, parse_ndjson_records, prepare_import_ndjson,
        serialize_graph_ndjson, slugify_segment, validate_branch_token,
    },
};
use crate::{
    HarnessError, Result,
    services::{Domain, Payload, Reply},
    store::{self, SessionStore},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

const CONFIG_FILE_NAME: &str = "omnigraph.json";
const MAX_TARBALL_BYTES: usize = 512 * 1024 * 1024;
const MAX_EXPANDED_BYTES: usize = 1024 * 1024 * 1024;
const MAX_ARCHIVE_FILES: usize = 64;
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
    runtime: Mutex<RuntimeState>,
    process: Arc<StdMutex<ProcessHandle>>,
    commit_cache: Arc<StdMutex<BTreeMap<String, CommitDigest>>>,
}

impl OmnigraphManager {
    /// Construct a manager using `state_dir` for `<state>/omnigraph.json`, tools, and cluster data.
    pub fn new(state_dir: PathBuf, store: Option<SessionStore>) -> Result<Self> {
        let config = load_config_file(&state_dir.join(CONFIG_FILE_NAME))?;
        Ok(Self {
            state_dir,
            store,
            runtime: Mutex::new(RuntimeState {
                config,
                last_error: None,
            }),
            process: Arc::new(StdMutex::new(ProcessHandle::default())),
            commit_cache: Arc::new(StdMutex::new(BTreeMap::new())),
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

    /// Detect `omnigraph` and `omnigraph-server` in `<state>/tools/omnigraph/v0.15.0/` or `$PATH`.
    pub fn detect_binaries(&self) -> BinaryDiscovery {
        let tools = self.tools_dir();
        let cli = find_binary(&tools, "omnigraph");
        let server = find_binary(&tools, "omnigraph-server");
        let version = cli.as_deref().and_then(probe_omnigraph_version);
        BinaryDiscovery {
            cli,
            server,
            version,
        }
    }

    /// Check whether `docker` is available and its daemon is reachable.
    pub fn docker_available() -> bool {
        let Some(docker) = infra_host::provider::find_executable("docker") else {
            return false;
        };
        Command::new(docker)
            .args(["info", "--format", "{{.ServerVersion}}"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    /// Download and extract Omnigraph v0.15.0 binaries into `<state>/tools/omnigraph/v0.15.0/`.
    pub async fn install_local(&self) -> Result<Value> {
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            return Err(HarnessError::invalid(
                "Automatic local download currently supports Linux x86_64; install `omnigraph` on PATH or use Docker/External mode.",
            ));
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            self.install_local_from(OMNIGRAPH_RELEASE_URL_LINUX_X86_64)
                .await
        }
    }

    /// Download and extract Omnigraph release tarball from `url` into `<state>/tools/omnigraph/v0.15.0/`.
    pub async fn install_local_from(&self, url: &str) -> Result<Value> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(180))
            .user_agent("AI-Infrastructure-Harness")
            .build()
            .map_err(|err| HarnessError::unavailable(format!("HTTP client error: {err}")))?;
        let resp = http.get(url).send().await.map_err(|err| {
            HarnessError::unavailable(format!("Failed downloading Omnigraph tarball: {err}"))
        })?;
        if !resp.status().is_success() {
            return Err(HarnessError::unavailable(format!(
                "Omnigraph download returned HTTP {}.",
                resp.status()
            )));
        }
        if let Some(len) = resp.content_length()
            && len as usize > MAX_TARBALL_BYTES
        {
            return Err(HarnessError::invalid(
                "Omnigraph archive exceeds maximum allowed download size.",
            ));
        }
        let compressed = resp.bytes().await.map_err(|err| {
            HarnessError::unavailable(format!("Failed reading Omnigraph download stream: {err}"))
        })?;
        if compressed.len() > MAX_TARBALL_BYTES {
            return Err(HarnessError::invalid(
                "Omnigraph archive exceeds maximum allowed download size.",
            ));
        }

        let tools_dir = self.tools_dir();
        let extracted =
            tokio::task::spawn_blocking(move || extract_omnigraph_tarball(&compressed, &tools_dir))
                .await
                .map_err(|_| {
                    HarnessError::unavailable("Omnigraph archive extraction task failed.")
                })??;

        let discovery = self.detect_binaries();
        Ok(json!({
            "ok": true,
            "version": discovery.version.unwrap_or_else(|| OMNIGRAPH_VERSION.to_string()),
            "installed_dir": self.tools_dir(),
            "files": extracted,
            "omnigraph": discovery.cli,
            "omnigraph_server": discovery.server,
        }))
    }

    /// Build a typed `OmnigraphClient` for the currently active/configured server.
    pub async fn client(&self) -> Result<OmnigraphClient> {
        let cfg = self.runtime.lock().await.config.clone();
        if cfg.mode == OmnigraphMode::Off {
            return Err(HarnessError::unavailable(
                "Omnigraph is currently disabled (mode=off).",
            ));
        }
        let endpoint = cfg.endpoint.as_deref().ok_or_else(|| {
            HarnessError::unavailable("Omnigraph endpoint is not configured or started.")
        })?;
        OmnigraphClient::new(endpoint, cfg.token.clone(), cfg.graph_id.clone())
            .map(|c| c.with_actor(Some(cfg.actor_id)))
            .map_err(Into::into)
    }

    /// Start the managed local `omnigraph-server` sidecar in cluster mode.
    pub async fn start_local(&self, graph_id: Option<&str>) -> Result<Value> {
        let binaries = self.detect_binaries();
        let cli_path = binaries.cli.ok_or_else(|| {
            HarnessError::missing(
                "Omnigraph CLI (`omnigraph`) not found. Install local v0.15.0 first.",
            )
        })?;
        let server_path = binaries.server.ok_or_else(|| {
            HarnessError::missing(
                "Omnigraph server (`omnigraph-server`) not found. Install local v0.15.0 first.",
            )
        })?;

        self.close().await?;

        let mut cfg = self.config().await;
        if let Some(gid) = graph_id.filter(|s| !s.trim().is_empty()) {
            schema::validate_graph_id(gid)?;
            cfg.graph_id = gid.trim().to_string();
        }
        let cfg_dir = self.cluster_config_dir();
        let data_dir = self.cluster_data_dir();
        schema::write_cluster_bundle(&cfg_dir, &cfg.graph_id)?;
        std::fs::create_dir_all(&data_dir)?;

        let actor = cfg.actor_id.clone();
        let cli_apply = cli_path.clone();
        let cfg_apply = cfg_dir.clone();
        let data_apply = data_dir.clone();
        tokio::task::spawn_blocking(move || {
            apply_and_unlock_cluster(&cli_apply, &cfg_apply, &data_apply, &actor)
        })
        .await
        .map_err(|_| HarnessError::unavailable("Cluster apply task failed."))??;

        let port = match cfg.bind_port {
            Some(p) if p > 0 => p,
            _ => pick_free_port()?,
        };
        let token = cfg
            .token
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| format!("{}{}", store::hexid(), store::hexid()));
        let tokens_json = build_tokens_json(&cfg.actor_id, &token);
        let bind_addr = format!("127.0.0.1:{port}");
        let abs_data = std::fs::canonicalize(&data_dir).unwrap_or(data_dir);

        let child = Command::new(&server_path)
            .args([
                "--cluster",
                &abs_data.to_string_lossy(),
                "--bind",
                &bind_addr,
            ])
            .env("OMNIGRAPH_SERVER_BEARER_TOKENS_JSON", &tokens_json)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| {
                HarnessError::unavailable(format!("Failed spawning omnigraph-server: {err}"))
            })?;

        {
            let mut proc_guard = self.process.lock().map_err(|_| {
                HarnessError::unavailable("Omnigraph process state lock was poisoned.")
            })?;
            proc_guard.child = Some(child);
        }

        let endpoint = format!("http://{bind_addr}");
        let client = OmnigraphClient::new(&endpoint, Some(token.clone()), &cfg.graph_id)?
            .with_actor(Some(cfg.actor_id.clone()));

        if let Err(err) = self
            .wait_ready_local(&client, Duration::from_secs(15))
            .await
        {
            let _ = self.close().await;
            let mut rt = self.runtime.lock().await;
            rt.last_error = Some(err.message.clone());
            return Err(err);
        }

        cfg.mode = OmnigraphMode::LocalSidecar;
        cfg.endpoint = Some(endpoint);
        cfg.token = Some(token);
        cfg.auto_start = true;
        {
            let mut rt = self.runtime.lock().await;
            rt.last_error = None;
        }
        self.save_config(cfg.clone()).await?;
        self.status_json("").await
    }

    /// Start Omnigraph via `ghcr.io/modernrelay/omnigraph-server:v0.15.0` Docker container.
    pub async fn start_docker(&self, graph_id: Option<&str>) -> Result<Value> {
        let docker = infra_host::provider::find_executable("docker")
            .ok_or_else(|| HarnessError::missing("Docker CLI was not found on PATH."))?;
        if !Self::docker_available() {
            return Err(HarnessError::unavailable(
                "Docker daemon is not running or unreachable.",
            ));
        }

        self.close().await?;

        let mut cfg = self.config().await;
        if let Some(gid) = graph_id.filter(|s| !s.trim().is_empty()) {
            schema::validate_graph_id(gid)?;
            cfg.graph_id = gid.trim().to_string();
        }
        let cfg_dir = self.cluster_config_dir();
        let data_dir = self.cluster_data_dir();
        schema::write_cluster_bundle(&cfg_dir, &cfg.graph_id)?;
        std::fs::create_dir_all(&data_dir)?;

        let abs_cfg = std::fs::canonicalize(&cfg_dir).unwrap_or(cfg_dir.clone());
        let abs_data = std::fs::canonicalize(&data_dir).unwrap_or(data_dir.clone());
        let image = cfg.docker_image.clone();
        let actor = cfg.actor_id.clone();

        if let Some(cli_path) = self.detect_binaries().cli {
            let apply_data = abs_data.clone();
            tokio::task::spawn_blocking(move || {
                apply_and_unlock_cluster(&cli_path, &abs_cfg, &apply_data, &actor)
            })
            .await
            .map_err(|_| HarnessError::unavailable("Cluster apply task failed."))??;
        } else {
            let docker_init = docker.clone();
            let img_init = image.clone();
            let cfg_vol = format!("{}:/config:ro", abs_cfg.display());
            let data_vol = format!("{}:/data", abs_data.display());
            tokio::task::spawn_blocking(move || {
                run_checked(
                    Command::new(&docker_init).args([
                        "run",
                        "--rm",
                        "--entrypoint",
                        "omnigraph",
                        "-v",
                        &cfg_vol,
                        "-v",
                        &data_vol,
                        &img_init,
                        "cluster",
                        "apply",
                        "--target",
                        "/data",
                        "--config",
                        "/config",
                        "--as",
                        &actor,
                        "--json",
                    ]),
                    "docker cluster apply",
                )?;
                let status_out = run_checked_output(
                    Command::new(&docker_init).args([
                        "run",
                        "--rm",
                        "--entrypoint",
                        "omnigraph",
                        "-v",
                        &data_vol,
                        &img_init,
                        "cluster",
                        "status",
                        "--target",
                        "/data",
                        "--json",
                    ]),
                    "docker cluster status",
                )?;
                if let Some(lock_id) = parse_lock_id(&status_out) {
                    run_checked(
                        Command::new(&docker_init).args([
                            "run",
                            "--rm",
                            "--entrypoint",
                            "omnigraph",
                            "-v",
                            &data_vol,
                            &img_init,
                            "cluster",
                            "force-unlock",
                            &lock_id,
                            "--target",
                            "/data",
                            "--json",
                        ]),
                        "docker cluster force-unlock",
                    )?;
                }
                Ok::<_, HarnessError>(())
            })
            .await
            .map_err(|_| HarnessError::unavailable("Docker cluster init task failed."))??;
        }

        let port = match cfg.bind_port {
            Some(p) if p > 0 => p,
            _ => pick_free_port()?,
        };
        let token = cfg
            .token
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| format!("{}{}", store::hexid(), store::hexid()));
        let tokens_json = build_tokens_json(&cfg.actor_id, &token);
        let container_name = cfg
            .container_name
            .clone()
            .unwrap_or_else(|| format!("harness-omnigraph-{}", &store::hexid()[..8]));
        let port_map = format!("127.0.0.1:{port}:8080");
        let data_vol = format!("{}:/data", abs_data.display());

        let docker_run = docker.clone();
        let name_run = container_name.clone();
        let img_run = image.clone();
        tokio::task::spawn_blocking(move || {
            let _ = Command::new(&docker_run)
                .args(["rm", "-f", &name_run])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            run_checked(
                Command::new(&docker_run).args([
                    "run",
                    "-d",
                    "--rm",
                    "--name",
                    &name_run,
                    "-p",
                    &port_map,
                    "-v",
                    &data_vol,
                    "-e",
                    &format!("OMNIGRAPH_SERVER_BEARER_TOKENS_JSON={tokens_json}"),
                    &img_run,
                    "--cluster",
                    "/data",
                    "--bind",
                    "0.0.0.0:8080",
                ]),
                "docker run omnigraph-server",
            )
        })
        .await
        .map_err(|_| HarnessError::unavailable("Docker startup task failed."))??;

        {
            let mut proc_guard = self.process.lock().map_err(|_| {
                HarnessError::unavailable("Omnigraph process state lock was poisoned.")
            })?;
            proc_guard.docker_container = Some(container_name.clone());
        }

        let endpoint = format!("http://127.0.0.1:{port}");
        let client = OmnigraphClient::new(&endpoint, Some(token.clone()), &cfg.graph_id)?
            .with_actor(Some(cfg.actor_id.clone()));

        if let Err(err) = self
            .wait_ready_local(&client, Duration::from_secs(20))
            .await
        {
            let _ = self.close().await;
            let mut rt = self.runtime.lock().await;
            rt.last_error = Some(err.message.clone());
            return Err(err);
        }

        cfg.mode = OmnigraphMode::Docker;
        cfg.endpoint = Some(endpoint);
        cfg.token = Some(token);
        cfg.container_name = Some(container_name);
        cfg.auto_start = true;
        {
            let mut rt = self.runtime.lock().await;
            rt.last_error = None;
        }
        self.save_config(cfg).await?;
        self.status_json("").await
    }

    /// Connect to an external Omnigraph HTTP `0.14` endpoint and persist configuration.
    pub async fn connect_external(
        &self,
        endpoint: &str,
        token: Option<&str>,
        graph_id: Option<&str>,
    ) -> Result<Value> {
        self.close().await?;

        let mut cfg = self.config().await;
        if let Some(gid) = graph_id.filter(|s| !s.trim().is_empty()) {
            schema::validate_graph_id(gid)?;
            cfg.graph_id = gid.trim().to_string();
        }
        let clean_endpoint = endpoint.trim().trim_end_matches('/').to_string();
        let clean_token = token
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .or_else(|| cfg.token.clone());

        let client = OmnigraphClient::new(&clean_endpoint, clean_token.clone(), &cfg.graph_id)?
            .with_actor(Some(cfg.actor_id.clone()));

        let health = client.health().await.map_err(|e| {
            HarnessError::unavailable(format!("Failed reaching Omnigraph /healthz: {e}"))
        })?;
        if health.status.is_empty() {
            return Err(HarnessError::unavailable(
                "Omnigraph /healthz returned an empty status.",
            ));
        }
        let _ = client.ready().await?;

        cfg.mode = OmnigraphMode::External;
        cfg.endpoint = Some(clean_endpoint);
        cfg.token = clean_token;
        cfg.auto_start = false;
        {
            let mut rt = self.runtime.lock().await;
            rt.last_error = None;
        }
        self.save_config(cfg).await?;
        self.status_json("").await
    }

    /// Re-apply the embedded `.pg`, `.gq`, and Cedar policy bundle to the configured cluster.
    pub async fn apply_schema(&self) -> Result<Value> {
        let cfg = self.config().await;
        let cfg_dir = self.cluster_config_dir();
        let data_dir = self.cluster_data_dir();
        schema::write_cluster_bundle(&cfg_dir, &cfg.graph_id)?;
        std::fs::create_dir_all(&data_dir)?;

        match cfg.mode {
            OmnigraphMode::LocalSidecar | OmnigraphMode::Off => {
                let cli_path = self.detect_binaries().cli.ok_or_else(|| {
                    HarnessError::missing(
                        "Omnigraph CLI (`omnigraph`) is required to apply cluster schema locally.",
                    )
                })?;
                let was_running = self.has_running_child();
                if was_running {
                    self.close().await?;
                }
                let actor = cfg.actor_id.clone();
                let res = tokio::task::spawn_blocking(move || {
                    apply_and_unlock_cluster(&cli_path, &cfg_dir, &data_dir, &actor)
                })
                .await
                .map_err(|_| HarnessError::unavailable("Cluster apply task failed."))?;
                res?;
                if was_running {
                    self.start_local(Some(&cfg.graph_id)).await?;
                }
                self.status_json("").await
            }
            OmnigraphMode::Docker => {
                let was_running = self.has_running_container();
                self.start_docker(Some(&cfg.graph_id)).await?;
                if !was_running {
                    self.close().await?;
                }
                self.status_json("").await
            }
            OmnigraphMode::External => {
                let client = self.client().await?;
                let remote_schema = client.schema().await?;
                Ok(json!({
                    "ok": true,
                    "mode": "external",
                    "graph_id": cfg.graph_id,
                    "schema_bytes": remote_schema.schema_source.len(),
                    "note": "Bundle written locally; apply to remote clusters using `omnigraph cluster apply`.",
                }))
            }
        }
    }

    /// Gracefully stop any spawned local `omnigraph-server` child or Docker container.
    pub async fn close(&self) -> Result<()> {
        let (child_opt, container_opt) = {
            let mut proc_guard = self.process.lock().map_err(|_| {
                HarnessError::unavailable("Omnigraph process state lock was poisoned.")
            })?;
            (proc_guard.child.take(), proc_guard.docker_container.take())
        };

        if let Some(mut child) = child_opt {
            tokio::task::spawn_blocking(move || terminate_child_sync(&mut child))
                .await
                .map_err(|_| HarnessError::unavailable("Failed waiting for Omnigraph shutdown."))?;
        }
        if let Some(container) = container_opt {
            tokio::task::spawn_blocking(move || {
                let _ = Command::new("docker")
                    .args(["stop", "-t", "3", &container])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            })
            .await
            .map_err(|_| {
                HarnessError::unavailable("Failed waiting for Docker Omnigraph shutdown.")
            })?;
        }
        Ok(())
    }

    fn has_running_child(&self) -> bool {
        self.process
            .lock()
            .ok()
            .and_then(|mut g| g.child.as_mut().map(|c| c.try_wait().ok() == Some(None)))
            .unwrap_or(false)
    }

    fn has_running_container(&self) -> bool {
        self.process
            .lock()
            .ok()
            .is_some_and(|g| g.docker_container.is_some())
    }

    async fn wait_ready_local(&self, client: &OmnigraphClient, timeout: Duration) -> Result<()> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if let Ok(mut guard) = self.process.lock()
                && let Some(child) = guard.child.as_mut()
                && let Ok(Some(status)) = child.try_wait()
            {
                return Err(HarnessError::unavailable(format!(
                    "omnigraph-server exited during startup with status {status}."
                )));
            }
            if let Ok(ready) = client.ready().await
                && ready.ready
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(120)).await;
        }
        Err(HarnessError::unavailable(
            "Timed out waiting for omnigraph-server /readyz.",
        ))
    }

    /// Compute status JSON expected by `GET /api/omnigraph/status`.
    pub async fn status_json(&self, project_id: &str) -> Result<Value> {
        let (cfg, last_error) = {
            let rt = self.runtime.lock().await;
            (rt.config.clone(), rt.last_error.clone())
        };
        let binaries = self.detect_binaries();
        let docker_available = Self::docker_available();

        let mut connected = false;
        let mut state = if cfg.mode == OmnigraphMode::Off {
            "unconfigured"
        } else {
            "stopped"
        };
        let mut error = last_error;
        let mut mcp_url: Option<String> = None;
        let mut stats = json!({
            "branches": 0,
            "atoms": 0,
            "sessions": 0,
            "edges": 0,
            "commits": 0,
            "unmerged_branches": 0
        });

        if cfg.mode != OmnigraphMode::Off
            && let Some(endpoint) = cfg.endpoint.as_deref()
            && let Ok(client) = OmnigraphClient::new(endpoint, cfg.token.clone(), &cfg.graph_id)
                .map(|c| c.with_actor(Some(cfg.actor_id.clone())))
        {
            match client.health().await {
                Ok(_) => {
                    let ready_ok = client.ready().await.map(|r| r.ready).unwrap_or(true);
                    if ready_ok {
                        connected = true;
                        state = "connected";
                        error = None;
                        mcp_url = Some(client.mcp_url());
                        if let Ok(computed) = compute_stats(&client, project_id).await {
                            stats = computed;
                        }
                    } else {
                        state = "starting";
                    }
                }
                Err(err) => {
                    state = "error";
                    error = Some(err.message);
                }
            }
        }

        Ok(json!({
            "connected": connected,
            "state": state,
            "mode": cfg.mode.as_str(),
            "api_contract": super::client::HTTP_API_CONTRACT_VERSION,
            "endpoint": cfg.endpoint,
            "graph_id": cfg.graph_id,
            "mcp_url": mcp_url,
            "binary_installed": binaries.cli.is_some() && binaries.server.is_some(),
            "binary_version": binaries.version,
            "docker_available": docker_available,
            "error": error,
            "stats": stats,
        }))
    }

    /// Execute setup action (`install`, `start_local`, `start_docker`, `connect`, `stop`, `apply_schema`).
    pub async fn setup_json(&self, body: &Value) -> Result<Value> {
        let action = body["action"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| HarnessError::invalid("Missing setup `action`."))?;
        let graph_id = body["graph_id"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty());

        match action {
            "install" => {
                if let Some(url) = body["release_url"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                {
                    self.install_local_from(url).await
                } else {
                    self.install_local().await
                }
            }
            "start_local" => self.start_local(graph_id).await,
            "start_docker" => self.start_docker(graph_id).await,
            "connect" => {
                let endpoint = body["endpoint"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| HarnessError::invalid("`endpoint` is required to connect."))?;
                let token = body["token"].as_str();
                self.connect_external(endpoint, token, graph_id).await
            }
            "stop" => {
                self.close().await?;
                let mut cfg = self.config().await;
                cfg.mode = OmnigraphMode::Off;
                cfg.auto_start = false;
                self.save_config(cfg).await?;
                self.status_json("").await
            }
            "apply_schema" => self.apply_schema().await,
            _ => Err(HarnessError::invalid(format!(
                "Unknown Omnigraph setup action '{action}'."
            ))),
        }
    }

    /// Build full constellation graph + branch/commit DAG payload for `/api/omnigraph/graph`
    /// using default pagination (`limit = 250`, `offset = 0`).
    pub async fn graph_view_json(
        &self,
        project_id: &str,
        branch: Option<&str>,
        snapshot: Option<&str>,
        kind: Option<&str>,
        q: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<Value> {
        self.graph_view_json_paginated(
            project_id, branch, snapshot, kind, q, session_id, None, None, None,
        )
        .await
    }

    /// Build constellation graph + branch/commit DAG payload for `/api/omnigraph/graph`
    /// with optional `limit` (`1..=1000`, default `250`) and `offset` / `cursor` pagination,
    /// `SymbolRef` / `MentionsSymbol` exposure, and latency/truncation telemetry.
    #[expect(
        clippy::too_many_arguments,
        reason = "Mirrors the /api/omnigraph/graph query parameter contract."
    )]
    pub async fn graph_view_json_paginated(
        &self,
        project_id: &str,
        branch: Option<&str>,
        snapshot: Option<&str>,
        kind: Option<&str>,
        q: Option<&str>,
        session_id: Option<&str>,
        limit: Option<usize>,
        offset: Option<usize>,
        cursor: Option<&str>,
    ) -> Result<Value> {
        let started = Instant::now();
        let cfg = self.config().await;
        let active_branch = clean_opt(branch).unwrap_or("main").to_string();
        let snapshot_commit = clean_opt(snapshot).map(str::to_string);
        let effective_limit = limit
            .unwrap_or(DEFAULT_GRAPH_ATOM_LIMIT)
            .clamp(1, MAX_GRAPH_ATOM_LIMIT);
        let parsed_cursor = parse_graph_cursor(cursor)?;
        let effective_offset = offset.or(parsed_cursor).unwrap_or(0);

        let Ok(client) = self.client().await else {
            return Ok(empty_graph_view(
                &cfg.graph_id,
                &active_branch,
                snapshot_commit,
                effective_limit,
                effective_offset,
                started.elapsed().as_millis() as u64,
            ));
        };

        let target_branch = if snapshot_commit.is_some() {
            None
        } else {
            Some(active_branch.clone())
        };
        let params = json!({ "project_id": project_id });
        let branch_ref = target_branch.as_deref();
        let snap_ref = snapshot_commit.as_deref();
        let kind_filter = clean_opt(kind).map(str::to_ascii_lowercase);
        let q_filter = clean_opt(q).map(str::to_ascii_lowercase);
        let sid_filter = clean_opt(session_id);
        let need_session_filter_edges = sid_filter.is_some();
        let need_q_filter_edges = q_filter.is_some();

        let commits_fut = fetch_commits_with_digests(&client, &self.commit_cache, &active_branch);
        let atoms_fut = run_recent_atoms_query(
            &client,
            project_id,
            target_branch.clone(),
            snapshot_commit.clone(),
        );
        let sessions_fut = query_edges_safe(
            &client,
            GQ_LIST_SESSIONS,
            "list_sessions",
            &params,
            branch_ref,
            snap_ref,
        );
        let pub_stage1_fut = async {
            if need_session_filter_edges {
                Some(
                    query_edges_safe(
                        &client,
                        GQ_LIST_PUBLISHED_EDGES,
                        "list_published_edges",
                        &params,
                        branch_ref,
                        snap_ref,
                    )
                    .await,
                )
            } else {
                None
            }
        };
        let con_stage1_fut = async {
            if need_session_filter_edges {
                Some(
                    query_edges_safe(
                        &client,
                        GQ_LIST_CONSUMED_EDGES,
                        "list_consumed_edges",
                        &params,
                        branch_ref,
                        snap_ref,
                    )
                    .await,
                )
            } else {
                None
            }
        };
        let file_edges_stage1_fut = async {
            if need_q_filter_edges {
                Some(
                    query_edges_safe(
                        &client,
                        GQ_LIST_FILE_EDGES,
                        "list_file_edges",
                        &params,
                        branch_ref,
                        snap_ref,
                    )
                    .await,
                )
            } else {
                None
            }
        };
        let symbol_edges_stage1_fut = async {
            if need_q_filter_edges {
                Some(
                    query_edges_safe(
                        &client,
                        GQ_LIST_SYMBOL_EDGES,
                        "list_symbol_edges",
                        &params,
                        branch_ref,
                        snap_ref,
                    )
                    .await,
                )
            } else {
                None
            }
        };

        let (
            branch_res,
            (commits_resp, commit_digests),
            atoms_res,
            session_rows,
            pub_edges_stage1,
            con_edges_stage1,
            file_edges_stage1,
            symbol_edges_stage1,
        ) = tokio::join!(
            client.list_branches(true),
            commits_fut,
            atoms_fut,
            sessions_fut,
            pub_stage1_fut,
            con_stage1_fut,
            file_edges_stage1_fut,
            symbol_edges_stage1_fut,
        );

        let Ok(branch_list) = branch_res else {
            return Ok(empty_graph_view(
                &cfg.graph_id,
                &active_branch,
                snapshot_commit,
                effective_limit,
                effective_offset,
                started.elapsed().as_millis() as u64,
            ));
        };
        let atoms_res = atoms_res?;

        let branches_json = build_branches_json(&branch_list);
        let commits_json: Vec<Value> = commits_resp
            .commits
            .iter()
            .zip(commit_digests.iter())
            .map(|(c, digest)| commit_to_graph_json(c, &active_branch, digest))
            .collect();

        let head_commit = atoms_res.graph_commit_id.clone().or_else(|| {
            branch_list
                .details
                .as_ref()
                .and_then(|ds| ds.iter().find(|d| d.name == active_branch))
                .map(|d| d.head.graph_commit_id.clone())
        });

        let (raw_atom_rows, backend_capped) = if atoms_res.rows.len() >= RECENT_ATOMS_LIMIT {
            expand_capped_graph_atoms(
                &client,
                project_id,
                &active_branch,
                snapshot_commit.as_deref(),
                atoms_res.rows,
            )
            .await
        } else {
            (atoms_res.rows, false)
        };

        let mut files_by_atom = file_edges_stage1
            .as_deref()
            .map(|edges| group_edge_targets_by_source(edges, None))
            .unwrap_or_default();
        let mut symbols_by_atom_all = symbol_edges_stage1
            .as_deref()
            .map(|edges| group_edge_targets_by_source(edges, None))
            .unwrap_or_default();

        let mut session_linked_atoms = BTreeSet::new();
        if let Some(sid) = sid_filter {
            for e in pub_edges_stage1
                .iter()
                .flatten()
                .chain(con_edges_stage1.iter().flatten())
            {
                if str_field(e, "target_id").as_deref() == Some(sid)
                    && let Some(aid) = str_field(e, "source_id")
                {
                    session_linked_atoms.insert(aid);
                }
            }
        }

        let mut filtered_atoms = Vec::new();
        for row in &raw_atom_rows {
            let mut atom = normalize_atom_row(row);
            let aid = str_field(&atom, "atom_id").unwrap_or_default();
            if file_edges_stage1.is_some() {
                atom["files"] = files_by_atom
                    .get(&aid)
                    .map(|files| json!(files))
                    .unwrap_or_else(|| json!([]));
            }
            if symbol_edges_stage1.is_some() {
                atom["symbols"] = symbols_by_atom_all
                    .get(&aid)
                    .map(|syms| json!(syms))
                    .unwrap_or_else(|| json!([]));
            }
            if let Some(ref kf) = kind_filter
                && atom["kind"]
                    .as_str()
                    .map(str::to_ascii_lowercase)
                    .as_deref()
                    != Some(kf.as_str())
            {
                continue;
            }
            if let Some(sid) = sid_filter
                && atom["source_session_id"].as_str() != Some(sid)
                && !session_linked_atoms.contains(&aid)
            {
                continue;
            }
            if let Some(ref qf) = q_filter
                && !atom_matches_query(&atom, qf)
            {
                continue;
            }
            filtered_atoms.push(atom);
        }

        let total_atoms = filtered_atoms.len();
        let page_start = effective_offset.min(total_atoms);
        let page_end = page_start.saturating_add(effective_limit).min(total_atoms);
        let mut atoms = filtered_atoms[page_start..page_end].to_vec();
        let kept_atom_ids: BTreeSet<String> = atoms
            .iter()
            .filter_map(|atom| str_field(atom, "atom_id"))
            .collect();
        let has_more = page_end < total_atoms || backend_capped;
        let truncated = has_more || atoms.len() < total_atoms || backend_capped;
        let next_cursor = has_more.then(|| page_end.to_string());

        let has_filters = kind_filter.is_some() || q_filter.is_some() || sid_filter.is_some();
        let is_paginated_subset = page_start > 0 || has_more;
        let need_unlinked_view = !has_filters && !is_paginated_subset;
        let allow_symbol_q_match = kind_filter.is_none() && sid_filter.is_none();
        let need_edges_for_kept = !kept_atom_ids.is_empty();
        let need_symbols_query = need_unlinked_view || (allow_symbol_q_match && q_filter.is_some());
        let need_follows_query = !session_rows.is_empty()
            && !(sid_filter.is_some()
                && kept_atom_ids.is_empty()
                && !session_rows
                    .iter()
                    .any(|s| str_field(s, "session_id").as_deref() == sid_filter));

        let (
            pub_edges_stage2,
            con_edges_stage2,
            file_edges_stage2,
            symbol_edges_stage2,
            rel_edges,
            follow_edges,
            symbol_rows,
        ) = tokio::join!(
            async {
                if pub_edges_stage1.is_none() && need_edges_for_kept {
                    query_edges_safe(
                        &client,
                        GQ_LIST_PUBLISHED_EDGES,
                        "list_published_edges",
                        &params,
                        branch_ref,
                        snap_ref,
                    )
                    .await
                } else {
                    Vec::new()
                }
            },
            async {
                if con_edges_stage1.is_none() && need_edges_for_kept {
                    query_edges_safe(
                        &client,
                        GQ_LIST_CONSUMED_EDGES,
                        "list_consumed_edges",
                        &params,
                        branch_ref,
                        snap_ref,
                    )
                    .await
                } else {
                    Vec::new()
                }
            },
            async {
                if file_edges_stage1.is_none() && need_edges_for_kept {
                    query_edges_safe(
                        &client,
                        GQ_LIST_FILE_EDGES,
                        "list_file_edges",
                        &params,
                        branch_ref,
                        snap_ref,
                    )
                    .await
                } else {
                    Vec::new()
                }
            },
            async {
                if symbol_edges_stage1.is_none() && need_edges_for_kept {
                    query_edges_safe(
                        &client,
                        GQ_LIST_SYMBOL_EDGES,
                        "list_symbol_edges",
                        &params,
                        branch_ref,
                        snap_ref,
                    )
                    .await
                } else {
                    Vec::new()
                }
            },
            async {
                if need_edges_for_kept {
                    query_edges_safe(
                        &client,
                        GQ_LIST_ATOM_RELATIONS,
                        "list_atom_relations",
                        &params,
                        branch_ref,
                        snap_ref,
                    )
                    .await
                } else {
                    Vec::new()
                }
            },
            async {
                if need_follows_query {
                    query_edges_safe(
                        &client,
                        GQ_LIST_SESSION_FOLLOWS,
                        "list_session_follows",
                        &params,
                        branch_ref,
                        snap_ref,
                    )
                    .await
                } else {
                    Vec::new()
                }
            },
            async {
                if need_symbols_query {
                    query_edges_safe(
                        &client,
                        GQ_LIST_SYMBOLS,
                        "list_symbols",
                        &params,
                        branch_ref,
                        snap_ref,
                    )
                    .await
                } else {
                    Vec::new()
                }
            },
        );

        let pub_edges = pub_edges_stage1.unwrap_or(pub_edges_stage2);
        let con_edges = con_edges_stage1.unwrap_or(con_edges_stage2);
        let file_edges_fetched = file_edges_stage1.is_none();
        let symbol_edges_fetched = symbol_edges_stage1.is_none();
        let file_edges = file_edges_stage1.unwrap_or(file_edges_stage2);
        let symbol_edge_rows = symbol_edges_stage1.unwrap_or(symbol_edges_stage2);

        let has_kept_edge_files = (need_edges_for_kept
            && (file_edges
                .iter()
                .any(|e| str_field(e, "source_id").is_some_and(|s| kept_atom_ids.contains(&s)))
                || symbol_edge_rows.iter().any(|e| {
                    str_field(e, "source_id").is_some_and(|s| kept_atom_ids.contains(&s))
                })))
            || !symbol_rows.is_empty();
        let need_files_query = need_unlinked_view
            || (allow_symbol_q_match && q_filter.is_some())
            || has_kept_edge_files;
        let file_rows = if need_files_query {
            query_edges_safe(
                &client,
                GQ_LIST_FILES,
                "list_files",
                &params,
                branch_ref,
                snap_ref,
            )
            .await
        } else {
            Vec::new()
        };

        if file_edges_fetched || symbol_edges_fetched {
            if file_edges_fetched {
                files_by_atom = group_edge_targets_by_source(&file_edges, Some(&kept_atom_ids));
            }
            if symbol_edges_fetched {
                symbols_by_atom_all =
                    group_edge_targets_by_source(&symbol_edge_rows, Some(&kept_atom_ids));
            }
            for atom in &mut atoms {
                let aid = str_field(atom, "atom_id").unwrap_or_default();
                if file_edges_fetched {
                    atom["files"] = files_by_atom
                        .get(&aid)
                        .map(|files| json!(files))
                        .unwrap_or_else(|| json!([]));
                }
                if symbol_edges_fetched {
                    atom["symbols"] = symbols_by_atom_all
                        .get(&aid)
                        .map(|syms| json!(syms))
                        .unwrap_or_else(|| json!([]));
                }
            }
        }

        let mut kept_session_ids_filter: Option<BTreeSet<String>> = sid_filter.map(|sid| {
            let mut keep = BTreeSet::new();
            keep.insert(sid.to_owned());
            for atom in &atoms {
                if let Some(author_sid) = str_field(atom, "source_session_id") {
                    keep.insert(author_sid);
                }
            }
            for e in pub_edges.iter().chain(con_edges.iter()) {
                if let (Some(src), Some(dst)) =
                    (str_field(e, "source_id"), str_field(e, "target_id"))
                    && kept_atom_ids.contains(&src)
                {
                    keep.insert(dst);
                }
            }
            for e in &follow_edges {
                if let (Some(src), Some(dst)) =
                    (str_field(e, "source_id"), str_field(e, "target_id"))
                    && (src == sid || dst == sid)
                {
                    keep.insert(src);
                    keep.insert(dst);
                }
            }
            keep
        });
        let filtered_sessions: Vec<Value> = session_rows
            .iter()
            .filter(|row| {
                kept_session_ids_filter.as_mut().is_none_or(|keep| {
                    str_field(row, "session_id").is_some_and(|sid| keep.contains(&sid))
                })
            })
            .map(normalize_session_row)
            .collect();
        let session_meta_by_id = self
            .store
            .as_ref()
            .map(|store| {
                let unique_sids: BTreeSet<String> = filtered_sessions
                    .iter()
                    .filter_map(|obj| str_field(obj, "session_id"))
                    .collect();
                load_session_omnigraph_metadata_batch(store, &unique_sids)
            })
            .unwrap_or_default();
        let sessions: Vec<Value> = filtered_sessions
            .into_iter()
            .map(|mut obj| {
                if let Some(sid) = str_field(&obj, "session_id")
                    && let Some(og) = session_meta_by_id.get(&sid)
                {
                    if let Some(space) = og["shared_space"].as_str().filter(|s| !s.is_empty()) {
                        obj["shared_space"] = json!(space);
                    }
                    if og["linked_session_ids"].is_array() {
                        obj["linked_session_ids"] = og["linked_session_ids"].clone();
                    }
                }
                obj
            })
            .collect();

        let (mut symbols, mut symbol_edges) = build_symbol_graph_entries(
            project_id,
            &symbol_rows,
            &symbol_edge_rows,
            &kept_atom_ids,
            if has_filters || is_paginated_subset {
                q_filter.as_deref()
            } else {
                None
            },
            need_unlinked_view,
            allow_symbol_q_match,
        );
        if !symbols.is_empty()
            && let Ok(Some(ir)) =
                crate::graph_ir::GraphIrStore::open_readonly(&self.state_dir, project_id)
        {
            let file_paths: BTreeSet<String> = symbols
                .iter()
                .filter_map(|s| str_field(s, "file_path"))
                .collect();
            if let Ok(id_map) = ir.canonical_symbol_ids_for_files(&file_paths) {
                for sym in &mut symbols {
                    if let Some(fqn) = str_field(sym, "fqn")
                        && let Some(cid) = id_map.get(&fqn)
                    {
                        sym["symbol_id"] = json!(cid);
                    }
                }
                for edge in &mut symbol_edges {
                    if let Some(fqn) = str_field(edge, "fqn")
                        && let Some(cid) = id_map.get(&fqn)
                    {
                        edge["symbol_id"] = json!(cid);
                    }
                }
            }
        }

        let kept_file_paths: BTreeSet<String> = if need_unlinked_view {
            BTreeSet::new()
        } else {
            let mut paths = BTreeSet::new();
            for atom in &atoms {
                for f in atom["files"].as_array().into_iter().flatten() {
                    if let Some(p) = f.as_str().filter(|p| !p.is_empty()) {
                        paths.insert(p.to_owned());
                    }
                }
            }
            for sym in &symbols {
                if let Some(p) = sym["file_path"].as_str().filter(|p| !p.is_empty()) {
                    paths.insert(p.to_owned());
                }
            }
            paths
        };
        let files: Vec<Value> = file_rows
            .iter()
            .filter(|row| {
                if need_unlinked_view {
                    return true;
                }
                let Some(path) = str_field(row, "path") else {
                    return false;
                };
                kept_file_paths.contains(&path)
                    || (allow_symbol_q_match
                        && q_filter
                            .as_deref()
                            .is_some_and(|qf| path.to_ascii_lowercase().contains(qf)))
            })
            .map(normalize_file_row)
            .collect();

        let mut edges = build_graph_edges(
            &pub_edges,
            &con_edges,
            &file_edges,
            &rel_edges,
            &follow_edges,
            &kept_atom_ids,
            sid_filter,
        );
        edges.extend(symbol_edges.iter().cloned());

        let elapsed_ms = started.elapsed().as_millis() as u64;
        let telemetry = json!({
            "elapsed_ms": elapsed_ms,
            "atom_count": atoms.len(),
            "total_atoms": total_atoms,
            "session_count": sessions.len(),
            "file_count": files.len(),
            "symbol_count": symbols.len(),
            "edge_count": edges.len(),
            "symbol_edge_count": symbol_edges.len(),
            "commit_count": commits_json.len(),
            "truncated": truncated,
            "has_more": has_more,
        });

        Ok(json!({
            "connected": true,
            "graph_id": cfg.graph_id,
            "active_branch": active_branch,
            "snapshot_commit": snapshot_commit,
            "head_commit": head_commit,
            "branches": branches_json,
            "commits": commits_json,
            "sessions": sessions,
            "atoms": atoms,
            "files": files,
            "symbols": symbols,
            "edges": edges,
            "symbol_edges": symbol_edges,
            "total_atoms": total_atoms,
            "limit": effective_limit,
            "offset": page_start,
            "has_more": has_more,
            "truncated": truncated,
            "next_cursor": next_cursor,
            "telemetry": telemetry,
        }))
    }

    /// Hybrid BM25 + fallback ranking search returning `{ query, branch, hits }`.
    pub async fn search_json(
        &self,
        project_id: &str,
        q: &str,
        branch: Option<&str>,
        kind: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Value> {
        let target_branch = clean_opt(branch).unwrap_or("main").to_string();
        let limit = limit.unwrap_or(25).clamp(1, 100);
        let client = self.client().await?;
        let kind_filter = clean_opt(kind).map(str::to_ascii_lowercase);
        let q_trimmed = q.trim();

        let mut raw_hits: Vec<(Value, f64, &'static str)> = Vec::new();
        if !q_trimmed.is_empty()
            && let Ok(resp) = run_stored_or_inline(
                &client,
                "context_search",
                schema::QUERY_CONTEXT_SEARCH,
                json!({ "q": q_trimmed, "project_id": project_id }),
                Some(target_branch.clone()),
                None,
            )
            .await
        {
            for row in &resp.rows {
                let atom = normalize_atom_row(row);
                let score = row["score"].as_f64().unwrap_or(0.5);
                raw_hits.push((atom, score, "bm25"));
            }
        }

        if raw_hits.is_empty() {
            let recent = run_stored_or_inline(
                &client,
                "recent_atoms",
                schema::QUERY_RECENT_ATOMS,
                json!({ "project_id": project_id }),
                Some(target_branch.clone()),
                None,
            )
            .await?;
            let q_lower = q_trimmed.to_ascii_lowercase();
            for row in &recent.rows {
                let atom = normalize_atom_row(row);
                let score = if q_lower.is_empty() {
                    atom["confidence"].as_f64().unwrap_or(0.5)
                } else {
                    score_atom_fallback(&atom, &q_lower)
                };
                if q_lower.is_empty() || score > 0.0 {
                    raw_hits.push((
                        atom,
                        score,
                        if q_lower.is_empty() {
                            "pinned"
                        } else {
                            "lexical"
                        },
                    ));
                }
            }
            raw_hits.sort_by(|a, b| b.1.total_cmp(&a.1));
        }

        let file_edges = query_edges_safe(
            &client,
            GQ_LIST_FILE_EDGES,
            "list_file_edges",
            &json!({ "project_id": project_id }),
            Some(&target_branch),
            None,
        )
        .await;
        let rel_edges = query_edges_safe(
            &client,
            GQ_LIST_ATOM_RELATIONS,
            "list_atom_relations",
            &json!({ "project_id": project_id }),
            Some(&target_branch),
            None,
        )
        .await;

        let mut hits = Vec::new();
        for (mut atom, score, matched_via) in raw_hits {
            if let Some(ref kf) = kind_filter
                && atom["kind"]
                    .as_str()
                    .map(str::to_ascii_lowercase)
                    .as_deref()
                    != Some(kf.as_str())
            {
                continue;
            }
            let aid = str_field(&atom, "atom_id").unwrap_or_default();
            let mut files = Vec::new();
            let mut neighbors = Vec::new();
            for fe in &file_edges {
                if str_field(fe, "source_id").as_deref() == Some(aid.as_str())
                    && let Some(path) = str_field(fe, "target_id")
                {
                    if !files.contains(&path) {
                        files.push(path.clone());
                    }
                    neighbors
                        .push(json!({ "kind": "file", "id": path, "relation": "touches_file" }));
                }
            }
            atom["files"] = json!(files);
            for re in &rel_edges {
                if str_field(re, "source_id").as_deref() == Some(aid.as_str())
                    && let Some(other) = str_field(re, "target_id")
                {
                    neighbors.push(json!({
                        "kind": "atom",
                        "id": other,
                        "relation": re["relation"].as_str().unwrap_or("relates_to"),
                    }));
                }
            }
            hits.push(json!({
                "atom": atom,
                "score": score,
                "matched_via": matched_via,
                "neighbors": neighbors,
            }));
            if hits.len() >= limit {
                break;
            }
        }

        Ok(json!({
            "query": q_trimmed,
            "branch": target_branch,
            "hits": hits,
        }))
    }

    /// Compare `source` branch against `target` (default `main`), including 3-way dry-run conflict checks.
    pub async fn branch_diff_json(
        &self,
        project_id: &str,
        source: &str,
        target: Option<&str>,
    ) -> Result<Value> {
        let source = source.trim();
        if source.is_empty() {
            return Err(HarnessError::invalid("`source` branch is required."));
        }
        let target = clean_opt(target).unwrap_or("main");
        let client = self.client().await?;

        let diff = client
            .branch_diff(&BranchDiffQuery {
                source: source.to_string(),
                target: Some(target.to_string()),
                limit: Some(500),
                page_token: None,
            })
            .await?;

        let mut added_atoms = Vec::new();
        let mut updated_atoms = Vec::new();
        let mut deleted_atoms = Vec::new();
        let mut added_edges = Vec::new();
        let mut deleted_edges = Vec::new();

        for ch in &diff.changes {
            match (ch.kind, ch.op) {
                (EntityKind::Node, ChangeOp::Insert) => {
                    if ch.entity_type.name == "ContextAtom" {
                        added_atoms.push(change_to_atom_entry(ch));
                    }
                }
                (EntityKind::Node, ChangeOp::Update) => {
                    if ch.entity_type.name == "ContextAtom" {
                        updated_atoms.push(change_to_atom_entry(ch));
                    }
                }
                (EntityKind::Node, ChangeOp::Delete) => {
                    if ch.entity_type.name == "ContextAtom" {
                        deleted_atoms.push(change_to_atom_entry(ch));
                    }
                }
                (EntityKind::Edge, ChangeOp::Insert | ChangeOp::Update) => {
                    added_edges.push(diff_edge_entry(ch));
                }
                (EntityKind::Edge, ChangeOp::Delete) => {
                    deleted_edges.push(diff_edge_entry(ch));
                }
            }
        }

        let (outcome, conflicts) = match client
            .merge_branches(&BranchMergeRequest {
                source: source.to_string(),
                target: Some(target.to_string()),
                delete_branch: false,
                as_commit: false,
                dry_run: true,
                settings: None,
            })
            .await
        {
            Ok(resp) => (resp.outcome.as_str().to_string(), Vec::new()),
            Err(err) if err.is_merge_conflict() => (
                "conflict".to_string(),
                conflicts_with_values(&client, project_id, source, target, &err.merge_conflicts)
                    .await,
            ),
            Err(err) => return Err(err.into()),
        };

        Ok(json!({
            "source": source,
            "target": target,
            "merge_base": diff.base_commit_id.unwrap_or(diff.from_commit_id),
            "to_commit_id": diff.to_commit_id,
            "outcome": outcome,
            "added_atoms": added_atoms,
            "updated_atoms": updated_atoms,
            "deleted_atoms": deleted_atoms,
            "added_edges": added_edges,
            "deleted_edges": deleted_edges,
            "conflicts": conflicts,
        }))
    }

    /// Create, merge, or delete an Omnigraph branch.
    pub async fn branch_action_json(&self, body: &Value) -> Result<Value> {
        let action = body["action"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| HarnessError::invalid("Missing branch `action`."))?;
        let client = self.client().await?;

        match action {
            "create" => {
                let name = store::text(&body["name"], "branch name", 128)?;
                let from = clean_opt(body["from"].as_str()).map(str::to_string);
                let from_commit = clean_opt(body["from_commit"].as_str()).map(str::to_string);
                let resp = client
                    .create_branch(&BranchCreateRequest {
                        name,
                        from,
                        from_commit,
                    })
                    .await?;
                Ok(json!({
                    "ok": true,
                    "action": "create",
                    "name": resp.name,
                    "from": resp.from,
                    "from_commit_id": resp.from_commit_id,
                    "actor_id": resp.actor_id,
                }))
            }
            "merge" => {
                let source = store::text(&body["source"], "source branch", 128)?;
                let target = clean_opt(body["target"].as_str())
                    .unwrap_or("main")
                    .to_string();
                let dry_run = body["dry_run"].as_bool().unwrap_or(false);
                let delete_branch = body["delete_branch"].as_bool().unwrap_or(false);
                let as_commit = body["as_commit"].as_bool().unwrap_or(false);

                match client
                    .merge_branches(&BranchMergeRequest {
                        source: source.clone(),
                        target: Some(target.clone()),
                        delete_branch,
                        as_commit,
                        dry_run,
                        settings: None,
                    })
                    .await
                {
                    Ok(resp) => Ok(json!({
                        "ok": true,
                        "action": "merge",
                        "source": resp.source,
                        "target": resp.target,
                        "outcome": resp.outcome.as_str(),
                        "dry_run": resp.dry_run,
                        "branch_deleted": resp.branch_deleted,
                        "commit": resp.commit,
                        "conflicts": [],
                    })),
                    Err(err) if err.is_merge_conflict() => {
                        // A conflicting merge publishes nothing, so both
                        // branches still hold the atoms in conflict.
                        let project_id = clean_opt(body["project_id"].as_str()).unwrap_or("");
                        let conflicts = conflicts_with_values(
                            &client,
                            project_id,
                            &source,
                            &target,
                            &err.merge_conflicts,
                        )
                        .await;
                        Ok(json!({
                            "ok": false,
                            "action": "merge",
                            "source": source,
                            "target": target,
                            "outcome": "conflict",
                            "dry_run": dry_run,
                            "added_atoms": [],
                            "updated_atoms": [],
                            "deleted_atoms": [],
                            "added_edges": [],
                            "deleted_edges": [],
                            "conflicts": conflicts,
                            "error": err.message,
                        }))
                    }
                    Err(err) => Err(err.into()),
                }
            }
            "keep" => {
                // Copy one atom onto `to` exactly as `from` stores it.
                // Omnigraph settles a `divergent_update` only once both
                // branches hold the same row; an ordinary update stamps its
                // own branch and time, so it would leave the conflict.
                let required = |key: &str| {
                    clean_opt(body[key].as_str())
                        .map(str::to_string)
                        .ok_or_else(|| {
                            HarnessError::invalid(format!("Keeping an atom needs `{key}`."))
                        })
                };
                let project_id = required("project_id")?;
                let atom_id = required("atom_id")?;
                let from = required("from")?;
                let to = required("to")?;
                if from == to {
                    return Err(HarnessError::invalid(
                        "Keep copies an atom from one branch to another.",
                    ));
                }
                let kept = find_atom_on_branch(&client, &project_id, &from, &atom_id)
                    .await
                    .ok_or_else(|| {
                        HarnessError::missing(format!("The atom {atom_id} is not on {from}."))
                    })?;
                // The person chose between the two versions the preview
                // showed; a session may have published a newer one on either
                // branch since, and neither is copied or overwritten unseen.
                let changed = |branch: &str| {
                    HarnessError::conflict(format!(
                        "The atom {atom_id} changed on {branch} after the merge preview. Preview the merge again."
                    ))
                };
                if let Some(expected) = clean_opt(body["expected_updated_at"].as_str())
                    && kept["updated_at"].as_str() != Some(expected)
                {
                    return Err(changed(&from));
                }
                if let Some(replaced) = clean_opt(body["replaced_updated_at"].as_str()) {
                    let current = find_atom_on_branch(&client, &project_id, &to, &atom_id).await;
                    if current
                        .as_ref()
                        .and_then(|atom| atom["updated_at"].as_str())
                        != Some(replaced)
                    {
                        return Err(changed(&to));
                    }
                }
                let params = stored_atom_params(&kept, &project_id);
                let resp = client
                    .mutate(&ChangeRequest {
                        query: GQ_UPSERT_ATOM.into(),
                        name: Some("upsert_atom".into()),
                        params: Some(params.clone()),
                        branch: Some(to.clone()),
                        settings: None,
                    })
                    .await?;
                Ok(json!({
                    "ok": true,
                    "action": "keep",
                    "atom_id": atom_id,
                    "from": from,
                    "to": to,
                    "atom": normalize_atom_row(&params),
                    "commit": resp.commit,
                }))
            }
            "delete" => {
                let name = store::text(&body["name"], "branch name", 128)?;
                if name == "main" {
                    return Err(HarnessError::invalid("Cannot delete the `main` branch."));
                }
                let resp = client.delete_branch(&name).await?;
                Ok(json!({
                    "ok": true,
                    "action": "delete",
                    "name": resp.name,
                    "actor_id": resp.actor_id,
                }))
            }
            _ => Err(HarnessError::invalid(format!(
                "Unknown branch action '{action}'."
            ))),
        }
    }

    /// Publish a new or updated `ContextAtom` (with optional linked files, session provenance, and relations).
    /// The declarations it names are read from the project the store knows
    /// for it, never from a path in the request.
    pub async fn publish_atom_json(&self, body: &Value) -> Result<Value> {
        let root = self.project_root(
            clean_opt(body["project_id"].as_str()).unwrap_or(""),
            clean_opt(body["session_id"].as_str()),
        );
        self.publish_atom_in(body, root.as_deref()).await
    }

    /// The folder a project's files are read from: the session's workspace
    /// when the session is the project's, else the registered project.
    fn project_root(&self, project_id: &str, session_id: Option<&str>) -> Option<PathBuf> {
        let store = self.store.as_ref()?;
        if project_id.is_empty() {
            return None;
        }
        if let Some(session) = session_id.and_then(|sid| store.get(sid).ok())
            && session["project_id"].as_str() == Some(project_id)
            && let Some(path) = clean_opt(session["project_path"].as_str())
        {
            return Some(PathBuf::from(path));
        }
        store
            .project(project_id)
            .ok()
            .and_then(|project| clean_opt(project["path"].as_str()).map(PathBuf::from))
    }

    async fn publish_atom_in(&self, body: &Value, root: Option<&Path>) -> Result<Value> {
        let project_id = clean_opt(body["project_id"].as_str())
            .unwrap_or("")
            .to_string();
        let branch = clean_opt(body["branch"].as_str())
            .unwrap_or("main")
            .to_string();
        let kind = clean_opt(body["kind"].as_str())
            .unwrap_or("insight")
            .to_ascii_lowercase();
        if !VALID_ATOM_KINDS.contains(&kind.as_str()) {
            return Err(HarnessError::invalid(format!(
                "Invalid ContextAtom kind '{kind}'."
            )));
        }
        let title = store::text(&body["title"], "atom title", 240)?;
        let summary = clean_opt(body["summary"].as_str())
            .map(str::to_string)
            .unwrap_or_else(|| title.clone());
        let atom_body = clean_opt(body["body"].as_str())
            .map(str::to_string)
            .unwrap_or_else(|| summary.clone());
        let atom_id = clean_opt(body["atom_id"].as_str())
            .or_else(|| clean_opt(body["id"].as_str()))
            .map(str::to_string)
            .unwrap_or_else(|| format!("atom-{}", &store::hexid()[..12]));
        let session_id = clean_opt(body["session_id"].as_str()).map(str::to_string);
        let tags = parse_tags_opt(&body["tags"]);
        let files = parse_string_list(&body["files"]);
        let confidence = body["confidence"].as_f64().unwrap_or(0.85).clamp(0.0, 1.0);
        let pin_priority = body["pin_priority"].as_i64().unwrap_or(10) as i32;
        let expected_commit = clean_opt(body["expected_commit"].as_str()).map(str::to_string);
        let ts = now_utc_iso_ms();

        let client = self.client().await?;
        // An update that leaves out a provenance field, the verification or
        // the creation time keeps the atom's own: the upsert writes every
        // field, and an edit of the text must not turn a Project Brain atom
        // into an unverified one with no source. A field sent as null clears.
        let named = clean_opt(body["atom_id"].as_str())
            .or_else(|| clean_opt(body["id"].as_str()))
            .is_some();
        let existing = if named {
            find_atom_on_branch(&client, &project_id, &branch, &atom_id).await
        } else {
            None
        };
        let mut merged = body.clone();
        if let Some(existing) = &existing
            && merged.is_object()
        {
            for key in [
                "origin_layer",
                "authority",
                "privacy",
                "brain_record_id",
                "memory_id",
                "record_revision",
                "verified",
            ] {
                if body.get(key).is_none() && !existing[key].is_null() {
                    merged[key] = existing[key].clone();
                }
            }
        }
        let verified = merged["verified"].as_bool().unwrap_or(false);
        let provenance = AtomProvenance::from_value(&merged);
        let created_at = existing
            .as_ref()
            .and_then(|atom| clean_opt(atom["created_at"].as_str()).map(str::to_string))
            .unwrap_or_else(|| ts.clone());
        // So does the session that wrote it: an update without `session_id`
        // keeps the stored one and links no session, so the Keep buttons of
        // the merge preview do not erase who published the atom.
        let source_session_id = if body.get("session_id").is_some() {
            session_id.clone()
        } else {
            existing
                .as_ref()
                .and_then(|atom| clean_opt(atom["source_session_id"].as_str()).map(str::to_string))
        };
        let atom_params = build_upsert_atom_params(
            &atom_id,
            &project_id,
            &kind,
            &title,
            &summary,
            &atom_body,
            tags,
            &branch,
            source_session_id.as_deref(),
            confidence,
            pin_priority,
            verified,
            &provenance,
            &created_at,
            &ts,
        );
        let req = ChangeRequest {
            query: GQ_UPSERT_ATOM.into(),
            name: Some("upsert_atom".into()),
            params: Some(atom_params.clone()),
            branch: Some(branch.clone()),
            settings: None,
        };
        let mut last_commit = if let Some(expected) = expected_commit.as_deref() {
            client.mutate_if_commit(&req, expected).await?.commit
        } else {
            client.mutate(&req).await?.commit
        };

        if let Some(sid) = session_id.as_deref() {
            let res = client
                .mutate(&ChangeRequest {
                    query: GQ_LINK_PUBLISHED_BY.into(),
                    name: Some("link_published_by".into()),
                    params: Some(json!({
                        "atom_id": atom_id,
                        "session_id": sid,
                        "role": "author",
                        "published_at": ts,
                    })),
                    branch: Some(branch.clone()),
                    settings: None,
                })
                .await;
            if let Ok(r) = res
                && r.commit.is_some()
            {
                last_commit = r.commit;
            }
        }

        if let Some(commit) =
            upsert_atom_files(&client, &project_id, &branch, &atom_id, &files, &ts).await
        {
            last_commit = Some(commit);
        }

        let symbols = atom_symbols_for_project(
            Some(&self.state_dir),
            &project_id,
            root,
            &files,
            &format!("{title}\n{summary}\n{atom_body}"),
        );
        let previous_symbols = if existing.is_some() {
            list_atom_symbols(&client, &project_id, &branch)
                .await
                .remove(&atom_id)
                .unwrap_or_default()
        } else {
            BTreeSet::new()
        };
        if let Some(commit) = upsert_atom_symbols(
            &client,
            &project_id,
            &branch,
            &atom_id,
            &symbols,
            &previous_symbols,
        )
        .await
        {
            last_commit = Some(commit);
        }

        if let Some(relations) = body["relates_to"].as_array() {
            for rel in relations {
                let Some((target_id, relation, note, rel_confidence)) = parse_relation_entry(rel)
                else {
                    continue;
                };
                if let Ok(r) = client
                    .mutate(&ChangeRequest {
                        query: GQ_LINK_RELATES_TO.into(),
                        name: Some("link_relates_to".into()),
                        params: Some(json!({
                            "source_id": atom_id,
                            "target_id": target_id,
                            "relation": relation,
                            "note": note,
                            "confidence": rel_confidence,
                            "created_at": ts,
                        })),
                        branch: Some(branch.clone()),
                        settings: None,
                    })
                    .await
                    && r.commit.is_some()
                {
                    last_commit = r.commit;
                }
            }
        }

        let atom_resp = normalize_atom_row(&atom_params);
        Ok(json!({
            "ok": true,
            "id": atom_id,
            "atom_id": atom_id,
            "branch": branch,
            "atom": atom_resp,
            "files": files,
            "symbols": symbols,
            "commit": last_commit,
        }))
    }

    /// Explicitly share selected `atom_ids` into `target_session_id` (recording `ConsumedBy` edges).
    pub async fn share_into_session_json(&self, body: &Value) -> Result<Value> {
        let target_session_id = store::text(&body["target_session_id"], "target_session_id", 128)?;
        let atom_ids: Vec<String> = body["atom_ids"]
            .as_array()
            .ok_or_else(|| HarnessError::invalid("`atom_ids` must be a non-empty array."))?
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        if atom_ids.is_empty() {
            return Err(HarnessError::invalid(
                "`atom_ids` must contain at least one atom id.",
            ));
        }
        let source_branch = clean_opt(body["source_branch"].as_str())
            .unwrap_or("main")
            .to_string();
        let mode = clean_opt(body["mode"].as_str())
            .unwrap_or("reference")
            .to_ascii_lowercase();
        if !matches!(
            mode.as_str(),
            "inject" | "reference" | "inject_and_link" | "copy"
        ) {
            return Err(HarnessError::invalid(format!(
                "Unknown share mode '{mode}'."
            )));
        }
        let project_id = clean_opt(body["project_id"].as_str())
            .unwrap_or("")
            .to_string();

        let target_branch = session_branch_name(&target_session_id);
        let client = self.client().await?;
        ensure_branch_exists(&client, &target_branch, Some(&source_branch)).await?;

        let ts = now_utc_iso_ms();
        let session_meta = self
            .store
            .as_ref()
            .and_then(|s| s.get(&target_session_id).ok());
        let sess_project_id = session_meta
            .as_ref()
            .and_then(|s| s["project_id"].as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(&project_id);
        let sess_title = session_meta
            .as_ref()
            .and_then(|s| s["title"].as_str())
            .unwrap_or("Harness Session");
        let sess_provider = session_meta
            .as_ref()
            .and_then(|s| s["provider"].as_str())
            .unwrap_or("codex");
        let sess_workflow = session_meta
            .as_ref()
            .and_then(|s| s["workflow"].as_str())
            .unwrap_or("native");
        let sess_status = session_meta
            .as_ref()
            .and_then(|s| s["status"].as_str())
            .unwrap_or("running");

        let _ = client
            .mutate(&ChangeRequest {
                query: GQ_UPSERT_SESSION.into(),
                name: Some("upsert_session".into()),
                params: Some(json!({
                    "session_id": target_session_id,
                    "project_id": sess_project_id,
                    "title": sess_title,
                    "branch": target_branch,
                    "provider": sess_provider,
                    "workflow": sess_workflow,
                    "status": sess_status,
                    "goal": Value::Null,
                    "created_at": ts,
                    "updated_at": ts,
                })),
                branch: Some(target_branch.clone()),
                settings: None,
            })
            .await?;

        let mut shared = Vec::new();
        let mut last_commit = None;
        if mode == "copy" && source_branch != target_branch {
            let source_atoms = run_stored_or_inline(
                &client,
                "recent_atoms",
                schema::QUERY_RECENT_ATOMS,
                json!({ "project_id": sess_project_id }),
                Some(source_branch.clone()),
                None,
            )
            .await?;
            let source_file_edges = query_edges_safe(
                &client,
                GQ_LIST_FILE_EDGES,
                "list_file_edges",
                &json!({ "project_id": sess_project_id }),
                Some(&source_branch),
                None,
            )
            .await;
            for row in &source_atoms.rows {
                let atom = normalize_atom_row(row);
                let aid = str_field(&atom, "atom_id").unwrap_or_default();
                if atom_ids.contains(&aid) {
                    let prov = AtomProvenance::from_value(&atom);
                    let params = build_upsert_atom_params(
                        &aid,
                        sess_project_id,
                        atom["kind"].as_str().unwrap_or("insight"),
                        atom["title"].as_str().unwrap_or(""),
                        atom["summary"].as_str().unwrap_or(""),
                        atom["body"].as_str().unwrap_or(""),
                        parse_tags_opt(&atom["tags"]),
                        &source_branch,
                        atom["source_session_id"].as_str(),
                        atom["confidence"].as_f64().unwrap_or(0.8),
                        atom["pin_priority"].as_i64().unwrap_or(10) as i32,
                        atom["verified"].as_bool().unwrap_or(false),
                        &prov,
                        &ts,
                        &ts,
                    );
                    if let Ok(r) = client
                        .mutate(&ChangeRequest {
                            query: GQ_UPSERT_ATOM.into(),
                            name: Some("upsert_atom".into()),
                            params: Some(params),
                            branch: Some(target_branch.clone()),
                            settings: None,
                        })
                        .await
                    {
                        if r.commit.is_some() {
                            last_commit = r.commit;
                        }
                        let mut copied_paths = BTreeSet::new();
                        for fe in source_file_edges.iter().filter(|fe| {
                            str_field(fe, "source_id").as_deref() == Some(aid.as_str())
                        }) {
                            let Some(path) = str_field(fe, "target_id") else {
                                continue;
                            };
                            if !copied_paths.insert(path.clone()) {
                                continue;
                            }
                            let _ = client
                                .mutate(&ChangeRequest {
                                    query: GQ_UPSERT_FILE.into(),
                                    name: Some("upsert_file".into()),
                                    params: Some(json!({
                                        "path": path,
                                        "project_id": sess_project_id,
                                        "language": detect_language(&path),
                                        "last_touched_at": ts,
                                    })),
                                    branch: Some(target_branch.clone()),
                                    settings: None,
                                })
                                .await;
                            if let Ok(fr) = client
                                .mutate(&ChangeRequest {
                                    query: GQ_LINK_TOUCHES_FILE.into(),
                                    name: Some("link_touches_file".into()),
                                    params: Some(json!({
                                        "atom_id": aid,
                                        "path": path,
                                        "access": str_field(fe, "access")
                                            .unwrap_or_else(|| "referenced".to_string()),
                                        "line_start": fe
                                            .get("line_start")
                                            .cloned()
                                            .unwrap_or(Value::Null),
                                        "line_end": fe
                                            .get("line_end")
                                            .cloned()
                                            .unwrap_or(Value::Null),
                                    })),
                                    branch: Some(target_branch.clone()),
                                    settings: None,
                                })
                                .await
                                && fr.commit.is_some()
                            {
                                last_commit = fr.commit;
                            }
                        }
                    }
                }
            }
        }

        for aid in &atom_ids {
            let res = client
                .mutate(&ChangeRequest {
                    query: GQ_LINK_CONSUMED_BY.into(),
                    name: Some("link_consumed_by".into()),
                    params: Some(json!({
                        "atom_id": aid,
                        "session_id": target_session_id,
                        "consumed_at": ts,
                        "injection_mode": mode,
                        "relevance": 1.0,
                    })),
                    branch: Some(target_branch.clone()),
                    settings: None,
                })
                .await;
            if let Ok(r) = res {
                shared.push(aid.clone());
                if r.commit.is_some() {
                    last_commit = r.commit;
                }
            }
        }

        Ok(json!({
            "ok": true,
            "target_session_id": target_session_id,
            "source_branch": source_branch,
            "target_branch": target_branch,
            "mode": mode,
            "shared_count": shared.len(),
            "atom_ids": shared,
            "commit": last_commit,
        }))
    }

    /// Create `session/<sid12>` branch, upsert `Session`, pull configured branches, and retrieve a ranked context capsule.
    pub async fn prepare_session_context(
        &self,
        session: &Value,
        project_path: &Path,
    ) -> Result<Value> {
        let cfg = self.config().await;
        if cfg.mode == OmnigraphMode::Off {
            return Ok(json!({
                "enabled": false,
                "capsule_text": "",
                "atoms": [],
            }));
        }
        let client = self.client().await?;
        let sid = session["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| HarnessError::invalid("Session object is missing `id`."))?;
        let project_id = session["project_id"].as_str().unwrap_or("");
        let omni_opts = &session["omnigraph"];
        let branch = clean_opt(omni_opts["branch"].as_str())
            .filter(|b| *b != "auto")
            .map(str::to_string)
            .unwrap_or_else(|| session_branch_name(sid));

        ensure_branch_exists(&client, &branch, Some("main")).await?;
        if let Some(raw_space) = clean_opt(omni_opts["shared_space"].as_str())
            && let Ok(space_branch) = normalize_shared_space_branch(raw_space)
            && space_branch != "main"
            && space_branch != branch
        {
            let _ = ensure_branch_exists(&client, &space_branch, Some("main")).await;
        }

        if let Some(pull_branches) = omni_opts["pull_branches"].as_array() {
            for pull in pull_branches.iter().filter_map(Value::as_str) {
                let pull = pull.trim();
                if !pull.is_empty() && pull != branch {
                    let _ = merge_or_sync_branch(&client, pull, &branch, project_id).await;
                }
            }
        }

        let ts = now_utc_iso_ms();
        // The search text is written into the shared graph's Session node, so
        // it is screened as an automatic memory query first.
        let goal = [
            omni_opts["query"].as_str(),
            session["goal"].as_str(),
            session["prompt"].as_str(),
            session["title"].as_str(),
        ]
        .into_iter()
        .flatten()
        .find_map(graph_session_text)
        .unwrap_or_default();
        let goal = goal.as_str();
        let title = session["title"].as_str().unwrap_or("Harness Session");
        let provider = session["provider"].as_str().unwrap_or("codex");
        let workflow = session["workflow"].as_str().unwrap_or("native");
        let status = session["status"].as_str().unwrap_or("running");
        let goal_opt = (!goal.is_empty()).then_some(goal);

        let session_upsert = client
            .mutate(&ChangeRequest {
                query: GQ_UPSERT_SESSION.into(),
                name: Some("upsert_session".into()),
                params: Some(json!({
                    "session_id": sid,
                    "project_id": project_id,
                    "title": title,
                    "branch": branch,
                    "provider": provider,
                    "workflow": workflow,
                    "status": status,
                    "goal": goal_opt,
                    "created_at": ts,
                    "updated_at": ts,
                })),
                branch: Some(branch.clone()),
                settings: None,
            })
            .await?;

        if let Some(peers) = omni_opts["linked_session_ids"].as_array()
            && !peers.is_empty()
        {
            let rel = canonical_session_follows_relation(omni_opts);
            for peer_id in peers.iter().filter_map(Value::as_str) {
                let peer_id = peer_id.trim();
                if peer_id.is_empty() || peer_id == sid {
                    continue;
                }
                let peer_meta = self.store.as_ref().and_then(|s| s.get(peer_id).ok());
                let _ = upsert_session_node(
                    &client,
                    peer_meta.as_ref(),
                    peer_id,
                    project_id,
                    &branch,
                    &ts,
                )
                .await;
                let _ = link_session_follows(&client, sid, peer_id, rel, &branch, &ts).await;
                let _ = link_session_follows(&client, peer_id, sid, rel, &branch, &ts).await;
            }
        }

        let has_canonical_branch = branch != CANONICAL_KNOWLEDGE_BRANCH
            && client.list_branches(false).await.is_ok_and(|list| {
                list.branches
                    .iter()
                    .any(|b| b == CANONICAL_KNOWLEDGE_BRANCH)
            });
        let mut queried_branches = vec![branch.clone()];
        if has_canonical_branch {
            queried_branches.push(CANONICAL_KNOWLEDGE_BRANCH.to_string());
        }

        let mut ranked_atoms: Vec<(Value, f64, String)> = Vec::new();
        let mut seen_ids = BTreeSet::new();
        if !goal.is_empty()
            && let Ok(search) = self
                .search_json(project_id, goal, Some(&branch), None, Some(8))
                .await
            && let Some(hits) = search["hits"].as_array()
        {
            for hit in hits {
                let atom = hit["atom"].clone();
                if let Some(aid) = str_field(&atom, "atom_id")
                    && seen_ids.insert(aid)
                {
                    ranked_atoms.push((atom, hit["score"].as_f64().unwrap_or(0.5), branch.clone()));
                }
            }
        }
        if has_canonical_branch
            && !goal.is_empty()
            && let Ok(search) = self
                .search_json(
                    project_id,
                    goal,
                    Some(CANONICAL_KNOWLEDGE_BRANCH),
                    None,
                    Some(6),
                )
                .await
            && let Some(hits) = search["hits"].as_array()
        {
            for hit in hits {
                let atom = hit["atom"].clone();
                if let Some(aid) = str_field(&atom, "atom_id")
                    && seen_ids.insert(aid)
                {
                    ranked_atoms.push((
                        atom,
                        hit["score"].as_f64().unwrap_or(0.5),
                        CANONICAL_KNOWLEDGE_BRANCH.to_string(),
                    ));
                }
            }
        }
        let branch_recent_cap = if has_canonical_branch { 8 } else { 10 };
        if ranked_atoms.len() < branch_recent_cap
            && let Ok(recent) = run_stored_or_inline(
                &client,
                "recent_atoms",
                schema::QUERY_RECENT_ATOMS,
                json!({ "project_id": project_id }),
                Some(branch.clone()),
                None,
            )
            .await
        {
            for row in &recent.rows {
                let atom = normalize_atom_row(row);
                if let Some(aid) = str_field(&atom, "atom_id")
                    && seen_ids.insert(aid)
                {
                    let prio = atom["pin_priority"].as_f64().unwrap_or(0.0);
                    ranked_atoms.push((atom, prio / 100.0, branch.clone()));
                }
                if ranked_atoms.len() >= branch_recent_cap {
                    break;
                }
            }
        }
        if has_canonical_branch
            && (goal.is_empty() || ranked_atoms.len() < 10)
            && let Ok(recent) = run_stored_or_inline(
                &client,
                "recent_atoms",
                schema::QUERY_RECENT_ATOMS,
                json!({ "project_id": project_id }),
                Some(CANONICAL_KNOWLEDGE_BRANCH.to_string()),
                None,
            )
            .await
        {
            let mut added_canonical = 0usize;
            for row in &recent.rows {
                let atom = normalize_atom_row(row);
                if let Some(aid) = str_field(&atom, "atom_id")
                    && seen_ids.insert(aid)
                {
                    let prio = atom["pin_priority"].as_f64().unwrap_or(0.0);
                    ranked_atoms.push((atom, prio / 100.0, CANONICAL_KNOWLEDGE_BRANCH.to_string()));
                    added_canonical += 1;
                }
                if ranked_atoms.len() >= 12 || added_canonical >= 6 {
                    break;
                }
            }
        }

        let mut files_by_branch_atom: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
        let mut files_by_atom: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for q_branch in &queried_branches {
            let file_edges = query_edges_safe(
                &client,
                GQ_LIST_FILE_EDGES,
                "list_file_edges",
                &json!({ "project_id": project_id }),
                Some(q_branch),
                None,
            )
            .await;
            for edge in &file_edges {
                if let (Some(src), Some(dst)) =
                    (str_field(edge, "source_id"), str_field(edge, "target_id"))
                {
                    let per_branch = files_by_branch_atom
                        .entry((q_branch.clone(), src.clone()))
                        .or_default();
                    if !per_branch.contains(&dst) {
                        per_branch.push(dst.clone());
                    }
                    let all = files_by_atom.entry(src).or_default();
                    if !all.contains(&dst) {
                        all.push(dst);
                    }
                }
            }
        }

        let mut consumed_count = 0usize;
        let mut canonical_consumed = Vec::new();
        let mut atoms_out = Vec::new();
        for (raw_atom, score, origin_query_branch) in ranked_atoms {
            let mut atom = normalize_atom_row(&raw_atom);
            if let Some(aid) = str_field(&atom, "atom_id") {
                let atom_files = files_by_branch_atom
                    .get(&(origin_query_branch.clone(), aid.clone()))
                    .or_else(|| files_by_atom.get(&aid))
                    .cloned()
                    .unwrap_or_else(|| parse_string_list(&atom["files"]));
                atom["files"] = json!(atom_files);
                if origin_query_branch == branch {
                    let _ = client
                        .mutate(&ChangeRequest {
                            query: GQ_LINK_CONSUMED_BY.into(),
                            name: Some("link_consumed_by".into()),
                            params: Some(json!({
                                "atom_id": aid,
                                "session_id": sid,
                                "consumed_at": ts,
                                "injection_mode": "auto_capsule",
                                "relevance": score,
                            })),
                            branch: Some(branch.clone()),
                            settings: None,
                        })
                        .await;
                    consumed_count += 1;
                } else if origin_query_branch == CANONICAL_KNOWLEDGE_BRANCH {
                    canonical_consumed.push((aid, score));
                }
            } else {
                atom["files"] = json!([]);
            }
            atoms_out.push(atom);
        }
        // A canonical atom stays on its branch: the session that consumed it
        // is recorded there, not copied onto the session branch.
        if !canonical_consumed.is_empty()
            && upsert_session_node(
                &client,
                Some(session),
                sid,
                project_id,
                CANONICAL_KNOWLEDGE_BRANCH,
                &ts,
            )
            .await
            .is_ok()
        {
            for (aid, score) in canonical_consumed {
                if client
                    .mutate(&ChangeRequest {
                        query: GQ_LINK_CONSUMED_BY.into(),
                        name: Some("link_consumed_by".into()),
                        params: Some(json!({
                            "atom_id": aid,
                            "session_id": sid,
                            "consumed_at": ts,
                            "injection_mode": "auto_capsule",
                            "relevance": score,
                        })),
                        branch: Some(CANONICAL_KNOWLEDGE_BRANCH.to_string()),
                        settings: None,
                    })
                    .await
                    .is_ok()
                {
                    consumed_count += 1;
                }
            }
        }

        // An injected atom that another contradicts is delivered with a
        // caution; the relation itself is left for a person to resolve.
        let injected_ids = atoms_out
            .iter()
            .filter_map(|atom| str_field(atom, "atom_id"))
            .collect::<BTreeSet<_>>();
        let contradiction_warnings =
            contradictions_touching(&client, project_id, &queried_branches, &injected_ids).await;
        let capsule_text = format_context_capsule(
            &cfg.graph_id,
            &branch,
            project_path,
            &atoms_out,
            &contradiction_warnings,
        );
        Ok(json!({
            "enabled": true,
            "graph_id": cfg.graph_id,
            "branch": branch,
            "head_commit": session_upsert.commit.map(|c| c.graph_commit_id),
            "capsule_text": capsule_text,
            "atoms": atoms_out,
            "consumed_count": consumed_count,
            "contradiction_warnings": contradiction_warnings,
        }))
    }

    /// Publish completed session learnings, touched files, and turn summary onto the session's Omnigraph branch.
    pub async fn publish_session_outputs(
        &self,
        session: &Value,
        learnings: &[Value],
        files: &[String],
        summary: &str,
    ) -> Result<Value> {
        let cfg = self.config().await;
        if cfg.mode == OmnigraphMode::Off {
            return Ok(json!({
                "ok": false,
                "skipped": "omnigraph-off",
                "published_atom_ids": [],
            }));
        }
        let client = self.client().await?;
        let sid = session["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| HarnessError::invalid("Session object is missing `id`."))?;
        let project_id = session["project_id"].as_str().unwrap_or("");
        let omni_opts = &session["omnigraph"];
        let branch = clean_opt(omni_opts["branch"].as_str())
            .filter(|b| *b != "auto")
            .map(str::to_string)
            .unwrap_or_else(|| session_branch_name(sid));
        ensure_branch_exists(&client, &branch, Some("main")).await?;
        // The learnings name declarations of the files in the session's own
        // workspace, which may be a worktree of the project.
        let root = clean_opt(session["project_path"].as_str())
            .map(PathBuf::from)
            .or_else(|| self.project_root(project_id, Some(sid)));

        let ts = now_utc_iso_ms();
        let mut last_commit = None;

        for file_path in files {
            let trimmed = file_path.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(r) = client
                .mutate(&ChangeRequest {
                    query: GQ_UPSERT_FILE.into(),
                    name: Some("upsert_file".into()),
                    params: Some(json!({
                        "path": trimmed,
                        "project_id": project_id,
                        "language": detect_language(trimmed),
                        "last_touched_at": ts,
                    })),
                    branch: Some(branch.clone()),
                    settings: None,
                })
                .await
                && r.commit.is_some()
            {
                last_commit = r.commit;
            }
        }

        let snapshot_id =
            (!summary.trim().is_empty()).then(|| format!("snap-{}", &store::hexid()[..12]));
        let turn_index = if snapshot_id.is_some() {
            next_turn_index(&client, sid, &branch).await
        } else {
            1
        };
        if let Some(ref snap_id) = snapshot_id
            && let Ok(r) = client
                .mutate(&ChangeRequest {
                    query: GQ_UPSERT_TURN_SNAPSHOT.into(),
                    name: Some("upsert_turn_snapshot".into()),
                    params: Some(json!({
                        "snapshot_id": snap_id,
                        "session_id": sid,
                        "turn_index": turn_index,
                        "summary": summary.trim(),
                        "created_at": ts,
                    })),
                    branch: Some(branch.clone()),
                    settings: None,
                })
                .await
            && r.commit.is_some()
        {
            last_commit = r.commit;
        }

        let mut published_atom_ids = Vec::new();
        let mut symbol_count = 0usize;
        for learning in learnings {
            let raw_kind = learning["kind"]
                .as_str()
                .or_else(|| learning["type"].as_str())
                .unwrap_or("finding")
                .to_ascii_lowercase();
            let kind = if VALID_ATOM_KINDS.contains(&raw_kind.as_str()) {
                raw_kind
            } else {
                "insight".to_string()
            };
            let Some(title) = clean_opt(learning["title"].as_str()) else {
                continue;
            };
            let body_text = clean_opt(learning["body"].as_str())
                .or_else(|| clean_opt(learning["consequence"].as_str()))
                .or_else(|| clean_opt(learning["summary"].as_str()))
                .unwrap_or(title);
            let summary_text = clean_opt(learning["summary"].as_str())
                .or_else(|| clean_opt(learning["consequence"].as_str()))
                .unwrap_or(title);
            let mut atom_files: Vec<String> = learning["files"]
                .as_array()
                .or_else(|| learning["sources"].as_array())
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            if atom_files.is_empty() {
                atom_files.extend(files.iter().take(4).cloned());
            }

            let published = self
                .publish_atom_in(&json!({
                    "project_id": project_id,
                    "branch": branch,
                    "session_id": sid,
                    "kind": kind,
                    "title": title,
                    "summary": summary_text,
                    "body": body_text,
                    "tags": learning.get("tags").cloned().unwrap_or(Value::Null),
                    "files": atom_files,
                    "confidence": learning["confidence"].as_f64().unwrap_or(0.9),
                    "pin_priority": learning["pin_priority"].as_i64().unwrap_or(15),
                    "verified": learning["verified"].as_bool().unwrap_or(false),
                    "origin_layer": learning.get("origin_layer").cloned().unwrap_or(Value::Null),
                    "authority": learning.get("authority").cloned().unwrap_or(Value::Null),
                    "privacy": learning.get("privacy").cloned().unwrap_or(Value::Null),
                    "brain_record_id": learning.get("brain_record_id").cloned().unwrap_or(Value::Null),
                    "memory_id": learning.get("memory_id").cloned().unwrap_or(Value::Null),
                    "record_revision": learning.get("record_revision").cloned().unwrap_or(Value::Null),
                }), root.as_deref())
                .await?;
            symbol_count += published["symbols"].as_array().map_or(0, Vec::len);
            if let Some(aid) = published["atom"]["atom_id"].as_str() {
                published_atom_ids.push(aid.to_string());
                if let Some(ref snap_id) = snapshot_id {
                    let _ = client
                        .mutate(&ChangeRequest {
                            query: GQ_LINK_DERIVED_FROM.into(),
                            name: Some("link_derived_from".into()),
                            params: Some(json!({
                                "atom_id": aid,
                                "snapshot_id": snap_id,
                                "recorded_at": ts,
                            })),
                            branch: Some(branch.clone()),
                            settings: None,
                        })
                        .await;
                }
            }
            if published.get("commit").is_some_and(|c| !c.is_null()) {
                last_commit = serde_json::from_value(published["commit"].clone()).ok();
            }
        }

        let mut merged = false;
        let mut merge_outcome = Value::Null;
        if omni_opts["auto_merge"].as_bool().unwrap_or(false)
            && branch != "main"
            && let Ok(resp) = client
                .merge_branches(&BranchMergeRequest {
                    source: branch.clone(),
                    target: Some("main".to_string()),
                    delete_branch: false,
                    as_commit: false,
                    dry_run: false,
                    settings: None,
                })
                .await
        {
            merged = true;
            merge_outcome = json!(resp.outcome.as_str());
            if resp.commit.is_some() {
                last_commit = resp.commit;
            }
        }

        Ok(json!({
            "ok": true,
            "branch": branch,
            "published_atom_ids": published_atom_ids,
            "symbol_count": symbol_count,
            "file_count": files.len(),
            "snapshot_id": snapshot_id,
            "merged": merged,
            "merge_outcome": merge_outcome,
            "commit": last_commit,
        }))
    }

    /// Poll incremental changes for live UI pulse (`GET /api/omnigraph/changes`).
    pub async fn changes_json(
        &self,
        _project_id: &str,
        cursor: Option<&str>,
        branch: Option<&str>,
    ) -> Result<Value> {
        let client = self.client().await?;
        let target_branch = clean_opt(branch).unwrap_or("main").to_string();
        let fresh_query = ChangeFeedQuery {
            branch: Some(target_branch.clone()),
            cursor: None,
            start: Some("beginning".to_string()),
            page_token: None,
            limit: Some(100),
        };
        let mut cursor_reset = false;
        let feed_res = if let Some(cur) = clean_opt(cursor) {
            let query = ChangeFeedQuery {
                branch: Some(target_branch.clone()),
                cursor: Some(cur.to_string()),
                start: None,
                page_token: None,
                limit: Some(100),
            };
            match client.changes(&query).await {
                Ok(feed) => Ok(feed),
                Err(err)
                    if err.is_change_feed_gap()
                        || err.status == 400
                        || err.code.as_deref() == Some("change_cursor_rejected") =>
                {
                    cursor_reset = true;
                    client.changes(&fresh_query).await
                }
                Err(err) => Err(err),
            }
        } else {
            client.changes(&fresh_query).await
        };
        let feed = match feed_res {
            Ok(feed) => feed,
            Err(err) if err.is_change_feed_gap() => {
                let baseline = client
                    .changes_baseline(&ChangeBaselineRequest {
                        branch: Some(target_branch.clone()),
                        kind: Vec::new(),
                        r#type: Vec::new(),
                        op: Vec::new(),
                    })
                    .await?;
                return Ok(json!({
                    "cursor": baseline.resume_cursor,
                    "caught_up": true,
                    "reset": true,
                    "snapshot_commit_id": baseline.snapshot_commit_id,
                    "events": [],
                }));
            }
            Err(err) => return Err(err.into()),
        };

        let mut events = Vec::new();
        for block in &feed.blocks {
            let authored_at = block.cause.authored_at;
            let created_at =
                format_omnigraph_ts(authored_at).unwrap_or_else(|| authored_at.to_string());
            for ch in &block.changes {
                let props = ch
                    .after
                    .as_ref()
                    .or(ch.before.as_ref())
                    .map(|img| img.properties.clone())
                    .unwrap_or(Value::Null);
                let title = props["title"]
                    .as_str()
                    .or_else(|| props["path"].as_str())
                    .unwrap_or(&ch.id)
                    .to_string();
                let summary = props["summary"]
                    .as_str()
                    .or_else(|| props["relation"].as_str())
                    .unwrap_or("")
                    .to_string();
                let kind = if ch.kind == EntityKind::Node {
                    props["kind"]
                        .as_str()
                        .unwrap_or(ch.entity_type.name.as_str())
                        .to_string()
                } else {
                    "edge".to_string()
                };
                let node_id = if ch.kind == EntityKind::Node {
                    Value::String(ch.id.clone())
                } else {
                    Value::Null
                };
                let atom_id = if ch.kind == EntityKind::Node && ch.entity_type.name == "ContextAtom"
                {
                    Value::String(ch.id.clone())
                } else {
                    Value::Null
                };
                let session_id = if ch.kind == EntityKind::Node && ch.entity_type.name == "Session"
                {
                    Value::String(ch.id.clone())
                } else {
                    props
                        .get("source_session_id")
                        .or_else(|| props.get("session_id"))
                        .cloned()
                        .unwrap_or(Value::Null)
                };
                events.push(json!({
                    "graph_commit_id": block.cause.graph_commit_id,
                    "commit_id": block.cause.graph_commit_id,
                    "branch": block.cause.authored_branch,
                    "actor_id": block.cause.actor_id,
                    "authored_at": authored_at,
                    "timestamp": authored_at,
                    "created_at": created_at,
                    "entity_kind": ch.kind,
                    "type_name": ch.entity_type.name,
                    "entity_id": ch.id,
                    "kind": kind,
                    "node_id": node_id,
                    "atom_id": atom_id,
                    "session_id": session_id,
                    "op": ch.op.as_str(),
                    "title": title,
                    "summary": summary,
                    "properties": props,
                }));
            }
        }

        Ok(json!({
            "cursor": feed.cursor,
            "caught_up": feed.caught_up.unwrap_or(true),
            "reset": cursor_reset,
            "events": events,
        }))
    }

    /// Inspect commit entity changes and affected atoms (`GET /api/omnigraph/commits/{commit_id}`).
    pub async fn commit_detail_json(&self, _project_id: &str, commit_id: &str) -> Result<Value> {
        let commit_id = commit_id.trim();
        if commit_id.is_empty() {
            return Err(HarnessError::invalid("`commit_id` is required."));
        }
        let client = self.client().await?;
        match client.commit_changes(commit_id, Some(200), None).await {
            Ok(detail) => {
                let mut atoms = Vec::new();
                let mut changes = Vec::new();
                for ch in &detail.changes {
                    if ch.kind == EntityKind::Node && ch.entity_type.name == "ContextAtom" {
                        atoms.push(change_to_atom_entry(ch));
                    }
                    changes.push(enrich_commit_change(ch));
                }
                Ok(json!({
                    "commit": {
                        "graph_commit_id": detail.cause.graph_commit_id,
                        "commit_id": detail.cause.graph_commit_id,
                        "parent_commit_id": detail.cause.parent_commit_id,
                        "merged_parent_commit_id": detail.cause.merged_parent_commit_id,
                        "branch": detail.cause.authored_branch,
                        "actor_id": detail.cause.actor_id,
                        "authored_at": detail.cause.authored_at,
                        "created_at": format_omnigraph_ts(detail.cause.authored_at)
                            .unwrap_or_else(|| detail.cause.authored_at.to_string()),
                    },
                    "changes": changes,
                    "atoms": atoms,
                }))
            }
            Err(err) if err.status == 409 => Ok(json!({
                "commit": {
                    "graph_commit_id": commit_id,
                    "commit_id": commit_id,
                    "parentless": true,
                },
                "changes": [],
                "atoms": [],
            })),
            Err(err) => Err(err.into()),
        }
    }

    /// Export a scoped task/branch/graph Omnigraph context bundle + raw NDJSON (`GET|POST /api/omnigraph/export`).
    pub async fn export_context_json(&self, params: &Value) -> Result<Value> {
        let cfg = self.config().await;
        let client = self.client().await?;
        let project_id = clean_opt(params["project_id"].as_str())
            .unwrap_or("")
            .to_string();
        let explicit_branch = clean_opt(params["branch"].as_str())
            .filter(|b| *b != "auto")
            .map(str::to_string);
        let session_id = clean_opt(params["session_id"].as_str()).map(str::to_string);

        let format = clean_opt(params["format"].as_str())
            .unwrap_or("bundle")
            .to_ascii_lowercase();
        if !matches!(format.as_str(), "bundle" | "ndjson") {
            return Err(HarnessError::invalid(format!(
                "Unsupported export format '{format}'; expected `bundle` or `ndjson`."
            )));
        }

        let default_scope = if session_id.is_some() {
            "task"
        } else if explicit_branch.is_some() {
            "branch"
        } else {
            "all"
        };
        let scope = clean_opt(params["scope"].as_str())
            .unwrap_or(default_scope)
            .to_ascii_lowercase();
        if !matches!(scope.as_str(), "task" | "branch" | "all") {
            return Err(HarnessError::invalid(format!(
                "Unsupported export scope '{scope}'; expected `task`, `branch`, or `all`."
            )));
        }

        let include_inherited =
            parse_bool_param(&params["include_inherited"]).unwrap_or(scope != "task");

        let mut source_branch = if scope == "all" && explicit_branch.is_none() {
            "main".to_string()
        } else if let Some(ref b) = explicit_branch {
            validate_branch_token(b)?
        } else if let Some(ref sid) = session_id {
            self.resolve_session_branch(sid)
        } else {
            "main".to_string()
        };

        let raw_ndjson = match client.export_branch(Some(&source_branch), &[], None).await {
            Ok(text) => text,
            Err(err)
                if err.is_not_found() && explicit_branch.is_none() && source_branch != "main" =>
            {
                source_branch = "main".to_string();
                client.export_branch(Some("main"), &[], None).await?
            }
            Err(err) => return Err(err.into()),
        };

        let parsed = parse_ndjson_records(&raw_ndjson)?;
        let filtered = filter_graph_for_export(
            &parsed,
            &project_id,
            &scope,
            session_id.as_deref(),
            &source_branch,
            include_inherited,
        );

        let mut files_by_atom: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for edge in &filtered.edges {
            if edge.edge_type == "TouchesFile" {
                files_by_atom
                    .entry(edge.from.clone())
                    .or_default()
                    .push(edge.to.clone());
            }
        }

        let atoms: Vec<Value> = filtered
            .atoms
            .values()
            .map(|row| {
                let mut atom = normalize_atom_row(row);
                let aid = str_field(&atom, "atom_id").unwrap_or_default();
                let atom_files = files_by_atom.get(&aid).cloned().unwrap_or_default();
                atom["files"] = json!(atom_files);
                atom
            })
            .collect();
        let files: Vec<Value> = filtered.files.values().map(normalize_file_row).collect();
        let symbols: Vec<Value> = filtered
            .symbols
            .values()
            .map(normalize_symbol_row)
            .collect();
        let sessions: Vec<Value> = filtered
            .sessions
            .values()
            .map(normalize_session_row)
            .collect();
        let turns: Vec<Value> = filtered.turns.values().map(normalize_turn_row).collect();
        let edges: Vec<Value> = filtered
            .edges
            .iter()
            .map(transfer::EdgeRecord::to_bundle_json)
            .collect();
        let ndjson = serialize_graph_ndjson(&filtered);

        let exported_at = now_utc_iso_ms();
        let label_slug = session_id
            .as_deref()
            .map(slugify_segment)
            .unwrap_or_else(|| slugify_segment(&source_branch));
        let stamp = exported_at
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(15)
            .collect::<String>();
        let ext = if format == "ndjson" { "ndjson" } else { "json" };
        let filename = format!("omnigraph-{label_slug}-{stamp}.{ext}");

        let bundle = json!({
            "format": BUNDLE_FORMAT_V1,
            "graph_id": cfg.graph_id,
            "project_id": project_id,
            "source_branch": source_branch,
            "source_session_id": session_id,
            "scope": scope,
            "include_inherited": include_inherited,
            "exported_at": exported_at,
            "atoms": atoms,
            "files": files,
            "symbols": symbols,
            "sessions": sessions,
            "turns": turns,
            "edges": edges,
            "ndjson": ndjson,
        });

        let counts = json!({
            "atoms": bundle["atoms"].as_array().map(Vec::len).unwrap_or(0),
            "files": bundle["files"].as_array().map(Vec::len).unwrap_or(0),
            "symbols": bundle["symbols"].as_array().map(Vec::len).unwrap_or(0),
            "sessions": bundle["sessions"].as_array().map(Vec::len).unwrap_or(0),
            "turns": bundle["turns"].as_array().map(Vec::len).unwrap_or(0),
            "edges": bundle["edges"].as_array().map(Vec::len).unwrap_or(0),
        });

        Ok(json!({
            "ok": true,
            "format": if format == "ndjson" { "ndjson" } else { BUNDLE_FORMAT_V1 },
            "requested_format": format,
            "filename": filename,
            "graph_id": cfg.graph_id,
            "project_id": project_id,
            "source_branch": source_branch,
            "source_session_id": session_id,
            "exported_at": exported_at,
            "counts": counts,
            "bundle": bundle,
            "ndjson": ndjson,
        }))
    }

    /// Export context as JSON metadata or downloadable attachment bytes (`download=1`).
    pub async fn export_context_payload(&self, query: &Value, body: &Value) -> Result<Payload> {
        let mut merged = Map::new();
        if let Some(qm) = query.as_object() {
            for (k, v) in qm {
                merged.insert(k.clone(), v.clone());
            }
        }
        if let Some(bm) = body.as_object() {
            for (k, v) in bm {
                merged.insert(k.clone(), v.clone());
            }
        }
        let params = Value::Object(merged);
        let download = parse_bool_param(&params["download"]).unwrap_or(false);
        let out = self.export_context_json(&params).await?;
        if !download {
            return Ok(Payload::Json(out));
        }

        let filename = out["filename"]
            .as_str()
            .unwrap_or("omnigraph-context.json")
            .to_string();
        if out["requested_format"] == "ndjson" {
            let text = out["ndjson"].as_str().unwrap_or("").to_string();
            Ok(Payload::Bytes {
                bytes: text.into_bytes(),
                content_type: "application/x-ndjson; charset=utf-8".into(),
                filename: Some(filename),
                etag: None,
                cache: Some("no-store".into()),
            })
        } else {
            let bytes = serde_json::to_vec_pretty(&out["bundle"])?;
            Ok(Payload::Bytes {
                bytes,
                content_type: "application/json; charset=utf-8".into(),
                filename: Some(filename),
                etag: None,
                cache: Some("no-store".into()),
            })
        }
    }

    /// Import an exported `harness.omnigraph.bundle.v1` JSON bundle or raw NDJSON onto an Omnigraph branch.
    pub async fn import_context_json(&self, body: &Value) -> Result<Value> {
        let client = self.client().await?;
        let project_id = clean_opt(body["project_id"].as_str())
            .unwrap_or("")
            .to_string();
        let mode = clean_opt(body["mode"].as_str())
            .unwrap_or("merge")
            .to_ascii_lowercase();
        if !matches!(mode.as_str(), "merge" | "replace" | "append") {
            return Err(HarnessError::invalid(format!(
                "Unsupported import mode '{mode}'; expected `merge` or `replace`."
            )));
        }

        let session_id = clean_opt(body["session_id"].as_str()).map(str::to_string);
        let target_branch =
            if let Some(b) = clean_opt(body["target_branch"].as_str()).filter(|b| *b != "auto") {
                validate_branch_token(b)?
            } else if let Some(ref sid) = session_id {
                self.resolve_session_branch(sid)
            } else {
                "main".to_string()
            };

        let explicit_from = clean_opt(body["from_branch"].as_str())
            .map(validate_branch_token)
            .transpose()?;
        let from_branch = explicit_from.clone().unwrap_or_else(|| "main".to_string());
        let create_branch_flag = body.get("create_branch").and_then(Value::as_bool);

        let parsed = parse_import_body(body)?;

        if target_branch != "main" && (create_branch_flag != Some(false) || explicit_from.is_some())
        {
            ensure_branch_exists(&client, &target_branch, Some(&from_branch)).await?;
        }

        let existing_on_branch = if mode != "replace" {
            client
                .export_branch(Some(&target_branch), &[], None)
                .await
                .ok()
                .and_then(|txt| parse_ndjson_records(&txt).ok())
        } else {
            None
        };

        let ts = now_utc_iso_ms();
        let (normalized, ndjson_payload) = prepare_import_ndjson(
            &parsed,
            &project_id,
            &target_branch,
            existing_on_branch.as_ref(),
            &ts,
        );

        let mut commit_id: Option<String> = None;
        if mode == "replace" {
            let clear_res = client
                .mutate(&ChangeRequest {
                    query: GQ_CLEAR_BRANCH_CONTEXT.into(),
                    name: Some("clear_branch_context".into()),
                    params: None,
                    branch: Some(target_branch.clone()),
                    settings: None,
                })
                .await?;
            if let Some(c) = clear_res.commit {
                commit_id = Some(c.graph_commit_id);
            }
            if !ndjson_payload.trim().is_empty() {
                let load_res = client
                    .load_ndjson(&target_branch, None, "overwrite", &ndjson_payload, None)
                    .await?;
                if let Some(cid) = load_res["commit"]["graph_commit_id"].as_str() {
                    commit_id = Some(cid.to_string());
                }
            }
        } else if !ndjson_payload.trim().is_empty() {
            let load_mode = if mode == "append" { "append" } else { "merge" };
            let load_res = client
                .load_ndjson(&target_branch, None, load_mode, &ndjson_payload, None)
                .await?;
            if let Some(cid) = load_res["commit"]["graph_commit_id"].as_str() {
                commit_id = Some(cid.to_string());
            }
        }

        let mut target_sids: Vec<String> = Vec::new();
        let explicit_targets = body["target_session_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        let inject_flag = body.get("inject_into_sessions").and_then(Value::as_bool);
        let should_inject = match inject_flag {
            Some(true) => true,
            Some(false) => false,
            None => !explicit_targets.is_empty(),
        };
        if should_inject {
            for sid in explicit_targets {
                if !target_sids.contains(&sid) {
                    target_sids.push(sid);
                }
            }
            if let Some(ref sid) = session_id
                && !target_sids.contains(sid)
            {
                target_sids.push(sid.clone());
            }
        }

        let atom_ids: Vec<String> = normalized.atoms.keys().cloned().collect();
        let mut shared_session_ids = Vec::new();
        for sid in &target_sids {
            let sess_meta = self.store.as_ref().and_then(|s| s.get(sid).ok());
            let sess_proj = sess_meta
                .as_ref()
                .and_then(|s| s["project_id"].as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(&project_id);
            let sess_branch = self.resolve_session_branch(sid);
            upsert_session_node(
                &client,
                sess_meta.as_ref(),
                sid,
                sess_proj,
                &target_branch,
                &ts,
            )
            .await?;

            for aid in &atom_ids {
                if let Ok(r) = client
                    .mutate(&ChangeRequest {
                        query: GQ_LINK_CONSUMED_BY.into(),
                        name: Some("link_consumed_by".into()),
                        params: Some(json!({
                            "atom_id": aid,
                            "session_id": sid,
                            "consumed_at": ts,
                            "injection_mode": "import",
                            "relevance": 1.0,
                        })),
                        branch: Some(target_branch.clone()),
                        settings: None,
                    })
                    .await
                    && let Some(c) = r.commit
                {
                    commit_id = Some(c.graph_commit_id);
                }
            }

            if sess_branch != target_branch {
                ensure_branch_exists(&client, &sess_branch, Some(&target_branch)).await?;
                if !ndjson_payload.trim().is_empty() {
                    let _ = client
                        .load_ndjson(&sess_branch, None, "merge", &ndjson_payload, None)
                        .await;
                }
                let _ = upsert_session_node(
                    &client,
                    sess_meta.as_ref(),
                    sid,
                    sess_proj,
                    &sess_branch,
                    &ts,
                )
                .await;
                for aid in &atom_ids {
                    let _ = client
                        .mutate(&ChangeRequest {
                            query: GQ_LINK_CONSUMED_BY.into(),
                            name: Some("link_consumed_by".into()),
                            params: Some(json!({
                                "atom_id": aid,
                                "session_id": sid,
                                "consumed_at": ts,
                                "injection_mode": "import",
                                "relevance": 1.0,
                            })),
                            branch: Some(sess_branch.clone()),
                            settings: None,
                        })
                        .await;
                }
            }
            shared_session_ids.push(sid.clone());
        }

        let mut files_by_atom: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for edge in &normalized.edges {
            if edge.edge_type == "TouchesFile" {
                let entry = files_by_atom.entry(edge.from.clone()).or_default();
                if !entry.contains(&edge.to) {
                    entry.push(edge.to.clone());
                }
            }
        }
        let atoms_out: Vec<Value> = normalized
            .atoms
            .values()
            .map(|row| {
                let mut atom = normalize_atom_row(row);
                let aid = str_field(&atom, "atom_id").unwrap_or_default();
                let files = files_by_atom.get(&aid).cloned().unwrap_or_default();
                atom["files"] = json!(files);
                atom
            })
            .collect();
        Ok(json!({
            "ok": true,
            "target_branch": target_branch,
            "mode": mode,
            "commit_id": commit_id,
            "imported_atoms": normalized.atoms.len(),
            "imported_files": normalized.files.len(),
            "imported_symbols": normalized.symbols.len(),
            "imported_sessions": normalized.sessions.len(),
            "imported_turns": normalized.turns.len(),
            "imported_edges": normalized.edges.len(),
            "atom_ids": atom_ids,
            "atoms": atoms_out,
            "shared_session_ids": shared_session_ids,
        }))
    }

    /// Link multiple Harness sessions in Omnigraph via mutual `shared_branch` or `broadcast` mode.
    pub async fn link_sessions_json(&self, body: &Value, store: &SessionStore) -> Result<Value> {
        let client = self.client().await?;
        let mode = clean_opt(body["mode"].as_str())
            .unwrap_or("shared_branch")
            .to_ascii_lowercase();
        if !matches!(mode.as_str(), "shared_branch" | "broadcast") {
            return Err(HarnessError::invalid(format!(
                "Unsupported session link mode '{mode}'; expected `shared_branch` or `broadcast`."
            )));
        }

        let requested_project_id = clean_opt(body["project_id"].as_str())
            .unwrap_or("")
            .to_string();
        let source_session_id = clean_opt(body["source_session_id"].as_str()).map(str::to_string);

        let mut selected_ids: Vec<String> = Vec::new();
        if let Some(arr) = body["session_ids"].as_array() {
            for item in arr {
                if let Some(s) = item.as_str().map(str::trim).filter(|s| !s.is_empty())
                    && !selected_ids.contains(&s.to_string())
                {
                    selected_ids.push(s.to_string());
                }
            }
        }

        let mut all_sids = Vec::new();
        if mode == "broadcast" {
            let mut target_ids = Vec::new();
            let src = source_session_id.clone().ok_or_else(|| {
                HarnessError::invalid("`source_session_id` is required in `broadcast` mode.")
            })?;
            all_sids.push(src.clone());
            for sid in selected_ids {
                if sid != src {
                    target_ids.push(sid.clone());
                    all_sids.push(sid);
                }
            }
            if target_ids.is_empty() {
                return Err(HarnessError::invalid(
                    "Select at least one target session distinct from `source_session_id`.",
                ));
            }
        } else {
            if let Some(ref src) = source_session_id {
                all_sids.push(src.clone());
            }
            for sid in selected_ids {
                if !all_sids.contains(&sid) {
                    all_sids.push(sid);
                }
            }
            if all_sids.len() < 2 {
                return Err(HarnessError::invalid(
                    "Select at least 2 sessions to share context.",
                ));
            }
        }

        let mut session_rows: Vec<(String, Value, String)> = Vec::with_capacity(all_sids.len());
        let mut resolved_project_id = requested_project_id.clone();
        for sid in &all_sids {
            let sess = store.get(sid)?;
            let sess_proj = sess["project_id"].as_str().unwrap_or("").to_string();
            if resolved_project_id.is_empty() {
                resolved_project_id = sess_proj.clone();
            } else if sess_proj != resolved_project_id {
                return Err(HarnessError::invalid(format!(
                    "Session `{sid}` does not belong to project `{resolved_project_id}`."
                )));
            }
            let sess_branch = clean_opt(sess["omnigraph"]["branch"].as_str())
                .filter(|b| *b != "auto")
                .map(str::to_string)
                .unwrap_or_else(|| session_branch_name(sid));
            session_rows.push((sid.clone(), sess, sess_branch));
        }

        let space_name_opt = clean_opt(body["space_name"].as_str()).map(str::to_string);
        let target_branch_opt = match body.get("target_branch") {
            None | Some(Value::Null) => None,
            Some(Value::String(raw)) => Some(normalize_link_target_branch(raw)?),
            Some(_) => return Err(HarnessError::invalid("Invalid Omnigraph branch name.")),
        };

        let (shared_branch, shared_space) = if mode == "broadcast" {
            let source_branch = session_rows[0].2.clone();
            let sb = target_branch_opt.unwrap_or(source_branch);
            let sp = space_name_opt.unwrap_or_else(|| sb.clone());
            (sb, sp)
        } else {
            let sb = if let Some(tb) = target_branch_opt {
                tb
            } else if let Some(ref sp) = space_name_opt {
                normalize_shared_space_branch(sp)
                    .map_err(|_| HarnessError::invalid("Invalid Omnigraph `space_name`."))?
            } else {
                let mut sorted = all_sids.clone();
                sorted.sort();
                let digest = store::sha(sorted.join(",").as_bytes());
                format!("shared/{}", &digest[..10])
            };
            let sp = space_name_opt.unwrap_or_else(|| sb.clone());
            (sb, sp)
        };

        let ts = now_utc_iso_ms();
        for (sid, sess, sess_branch) in &session_rows {
            ensure_branch_exists(&client, sess_branch, Some("main")).await?;
            upsert_session_node(
                &client,
                Some(sess),
                sid,
                &resolved_project_id,
                sess_branch,
                &ts,
            )
            .await?;
        }
        ensure_branch_exists(&client, &shared_branch, Some("main")).await?;
        for (sid, sess, _) in &session_rows {
            upsert_session_node(
                &client,
                Some(sess),
                sid,
                &resolved_project_id,
                &shared_branch,
                &ts,
            )
            .await?;
        }

        let mut last_commit: Option<String> = None;
        let mut conflicts: Vec<Value> = Vec::new();

        if mode == "shared_branch" {
            for (_, _, sess_branch) in &session_rows {
                if sess_branch != &shared_branch {
                    let (commit, mut cfl) = merge_or_sync_branch(
                        &client,
                        sess_branch,
                        &shared_branch,
                        &resolved_project_id,
                    )
                    .await?;
                    if let Some(c) = commit {
                        last_commit = Some(c.graph_commit_id);
                    }
                    conflicts.append(&mut cfl);
                }
            }

            let shared_atoms =
                fetch_branch_atoms(&client, &resolved_project_id, &shared_branch).await?;
            for i in 0..all_sids.len() {
                for j in 0..all_sids.len() {
                    if i != j
                        && let Ok(r) = link_session_follows(
                            &client,
                            &all_sids[i],
                            &all_sids[j],
                            "shared_branch",
                            &shared_branch,
                            &ts,
                        )
                        .await
                        && let Some(c) = r.commit
                    {
                        last_commit = Some(c.graph_commit_id);
                    }
                }
            }
            for atom in &shared_atoms {
                if let Some(aid) = str_field(atom, "atom_id") {
                    for sid in &all_sids {
                        if let Ok(r) = link_consumed_by(
                            &client,
                            &aid,
                            sid,
                            "shared_branch",
                            &shared_branch,
                            &ts,
                        )
                        .await
                            && let Some(c) = r.commit
                        {
                            last_commit = Some(c.graph_commit_id);
                        }
                    }
                }
            }

            for (_, _, sess_branch) in &session_rows {
                if sess_branch != &shared_branch {
                    let (commit, mut cfl) = merge_or_sync_branch(
                        &client,
                        &shared_branch,
                        sess_branch,
                        &resolved_project_id,
                    )
                    .await?;
                    if let Some(c) = commit {
                        last_commit = Some(c.graph_commit_id);
                    }
                    conflicts.append(&mut cfl);
                }
            }

            let atom_ids: Vec<String> = shared_atoms
                .iter()
                .filter_map(|a| str_field(a, "atom_id"))
                .collect();
            return Ok(json!({
                "ok": true,
                "mode": mode,
                "shared_branch": shared_branch,
                "shared_space": shared_space,
                "source_session_id": Value::Null,
                "commit_id": last_commit,
                "synced_atoms": shared_atoms.len(),
                "atom_ids": atom_ids,
                "atoms": shared_atoms,
                "linked_session_ids": all_sids,
                "conflicts": conflicts,
            }));
        }

        // Broadcast mode: push `source_session_id`'s branch/atoms into all target sessions
        let (source_sid, _, source_branch) = &session_rows[0];
        if &shared_branch != source_branch {
            let (commit, mut cfl) =
                merge_or_sync_branch(&client, source_branch, &shared_branch, &resolved_project_id)
                    .await?;
            if let Some(c) = commit {
                last_commit = Some(c.graph_commit_id);
            }
            conflicts.append(&mut cfl);
        }
        let source_atoms = fetch_branch_atoms(&client, &resolved_project_id, source_branch).await?;
        for (target_sid, _, target_branch) in session_rows.iter().skip(1) {
            let (commit, mut cfl) =
                merge_or_sync_branch(&client, source_branch, target_branch, &resolved_project_id)
                    .await?;
            if let Some(c) = commit {
                last_commit = Some(c.graph_commit_id);
            }
            conflicts.append(&mut cfl);

            for b in [target_branch.as_str(), shared_branch.as_str()] {
                let _ = link_session_follows(&client, target_sid, source_sid, "broadcast", b, &ts)
                    .await;
            }
            for atom in &source_atoms {
                if let Some(aid) = str_field(atom, "atom_id")
                    && let Ok(r) =
                        link_consumed_by(&client, &aid, target_sid, "broadcast", target_branch, &ts)
                            .await
                    && let Some(c) = r.commit
                {
                    last_commit = Some(c.graph_commit_id);
                }
            }
        }

        let atom_ids: Vec<String> = source_atoms
            .iter()
            .filter_map(|a| str_field(a, "atom_id"))
            .collect();
        Ok(json!({
            "ok": true,
            "mode": mode,
            "shared_branch": shared_branch,
            "shared_space": shared_space,
            "source_session_id": source_sid,
            "commit_id": last_commit,
            "synced_atoms": source_atoms.len(),
            "atom_ids": atom_ids,
            "atoms": source_atoms,
            "linked_session_ids": all_sids,
            "conflicts": conflicts,
        }))
    }

    /// Best-effort helper to create `Session` nodes and `SessionFollows` edges when a new session is
    /// created with `omnigraph.linked_session_ids`.
    pub async fn sync_new_session_links(
        &self,
        store: &SessionStore,
        session: &Value,
    ) -> Result<()> {
        let cfg = self.config().await;
        if cfg.mode == OmnigraphMode::Off {
            return Ok(());
        }
        let peers = session["omnigraph"]["linked_session_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        if peers.is_empty() {
            return Ok(());
        }
        let Ok(client) = self.client().await else {
            return Ok(());
        };
        let sid = session["id"].as_str().unwrap_or("");
        if sid.is_empty() {
            return Ok(());
        }
        let project_id = session["project_id"].as_str().unwrap_or("");
        let branch = self.resolve_session_branch(sid);
        let ts = now_utc_iso_ms();
        ensure_branch_exists(&client, &branch, Some("main")).await?;
        upsert_session_node(&client, Some(session), sid, project_id, &branch, &ts).await?;
        let rel = canonical_session_follows_relation(&session["omnigraph"]);
        for peer_id in &peers {
            if peer_id == sid {
                continue;
            }
            let peer_meta = store.get(peer_id).ok();
            let _ = upsert_session_node(
                &client,
                peer_meta.as_ref(),
                peer_id,
                project_id,
                &branch,
                &ts,
            )
            .await;
            let _ = link_session_follows(&client, sid, peer_id, rel, &branch, &ts).await;
            let _ = link_session_follows(&client, peer_id, sid, rel, &branch, &ts).await;
            if let Some(ref pm) = peer_meta {
                let peer_branch = clean_opt(pm["omnigraph"]["branch"].as_str())
                    .filter(|b| *b != "auto")
                    .map(str::to_string)
                    .unwrap_or_else(|| session_branch_name(peer_id));
                if ensure_branch_exists(&client, &peer_branch, Some("main"))
                    .await
                    .is_ok()
                {
                    let _ = upsert_session_node(
                        &client,
                        Some(session),
                        sid,
                        project_id,
                        &peer_branch,
                        &ts,
                    )
                    .await;
                    let _ = upsert_session_node(
                        &client,
                        Some(pm),
                        peer_id,
                        project_id,
                        &peer_branch,
                        &ts,
                    )
                    .await;
                    let _ =
                        link_session_follows(&client, sid, peer_id, rel, &peer_branch, &ts).await;
                    let _ =
                        link_session_follows(&client, peer_id, sid, rel, &peer_branch, &ts).await;
                }
            }
        }
        Ok(())
    }

    /// Sync canonical `Project Brain` and `Memory Bank` knowledge items onto `CANONICAL_KNOWLEDGE_BRANCH`.
    pub async fn sync_canonical_knowledge(
        &self,
        project_id: &str,
        items: &[Value],
    ) -> Result<Value> {
        self.sync_canonical_knowledge_with_options(project_id, items, &[])
            .await
    }

    /// `sync_canonical_knowledge` that also deletes, for each of
    /// `prune_layers` (`project-brain`, `memory-bank`), the project's synced
    /// atoms of that layer that `items` no longer carries: a retired,
    /// archived, superseded or stale record or chunk leaves the branch. Pass
    /// only a layer read whole, with every page of it in `items`.
    pub async fn sync_canonical_knowledge_with_options(
        &self,
        project_id: &str,
        items: &[Value],
        prune_layers: &[String],
    ) -> Result<Value> {
        let cfg = self.config().await;
        if cfg.mode == OmnigraphMode::Off {
            return Ok(json!({
                "ok": false,
                "skipped": "omnigraph-off",
                "synced": 0,
                "atom_ids": [],
            }));
        }
        let client = self.client().await?;
        let branch = CANONICAL_KNOWLEDGE_BRANCH;
        ensure_branch_exists(&client, branch, Some("main")).await?;

        let mut existing_ids = collect_branch_atom_ids(&client, project_id, branch).await;
        // A record or chunk names declarations of the project's own files;
        // what each synced atom named before is replaced only when it changed.
        let root = self.project_root(project_id, None);
        let mut previous_symbols = list_atom_symbols(&client, project_id, branch).await;
        let mut symbol_count = 0usize;
        let ts = now_utc_iso_ms();
        let mut atom_ids = Vec::new();
        let mut pending_supersedes: Vec<(String, Vec<String>)> = Vec::new();

        for item in items {
            let Some(atom_id) = str_field(item, "atom_id").or_else(|| str_field(item, "id")) else {
                continue;
            };
            let raw_title = str_field(item, "title")
                .or_else(|| str_field(item, "summary"))
                .unwrap_or_else(|| atom_id.clone());
            let title: String = raw_title.chars().take(240).collect();
            let raw_kind = str_field(item, "kind")
                .unwrap_or_else(|| "finding".to_string())
                .to_ascii_lowercase();
            let kind = if VALID_ATOM_KINDS.contains(&raw_kind.as_str()) {
                raw_kind
            } else {
                "insight".to_string()
            };
            let summary = str_field(item, "summary").unwrap_or_else(|| title.clone());
            let body = str_field(item, "body").unwrap_or_else(|| summary.clone());
            let tags = parse_tags_opt(&item["tags"]);
            let files = parse_string_list(&item["files"]);
            let confidence = item["confidence"].as_f64().unwrap_or(0.9).clamp(0.0, 1.0);
            let pin_priority = item["pin_priority"].as_i64().unwrap_or(20) as i32;
            let prov = AtomProvenance::from_value(item);
            let verified = item
                .get("verified")
                .and_then(Value::as_bool)
                .unwrap_or_else(|| prov.authority.as_deref() == Some("verified"));
            let effective_project_id = clean_opt(Some(project_id))
                .or_else(|| clean_opt(item["project_id"].as_str()))
                .unwrap_or("");
            let source_session_id =
                str_field(item, "source_session_id").or_else(|| str_field(item, "session_id"));

            let atom_params = build_upsert_atom_params(
                &atom_id,
                effective_project_id,
                &kind,
                &title,
                &summary,
                &body,
                tags,
                branch,
                source_session_id.as_deref(),
                confidence,
                pin_priority,
                verified,
                &prov,
                &ts,
                &ts,
            );
            client
                .mutate(&ChangeRequest {
                    query: GQ_UPSERT_ATOM.into(),
                    name: Some("upsert_atom".into()),
                    params: Some(atom_params),
                    branch: Some(branch.to_string()),
                    settings: None,
                })
                .await?;

            upsert_atom_files(&client, effective_project_id, branch, &atom_id, &files, &ts).await;
            let symbols = atom_symbols_for_project(
                Some(&self.state_dir),
                effective_project_id,
                root.as_deref(),
                &files,
                &format!("{title}\n{summary}\n{body}"),
            );
            symbol_count += symbols.len();
            upsert_atom_symbols(
                &client,
                effective_project_id,
                branch,
                &atom_id,
                &symbols,
                &previous_symbols.remove(&atom_id).unwrap_or_default(),
            )
            .await;
            existing_ids.insert(atom_id.clone());
            let supersedes = parse_string_list(&item["supersedes_atom_ids"]);
            if !supersedes.is_empty() {
                pending_supersedes.push((atom_id.clone(), supersedes));
            }
            atom_ids.push(atom_id);
        }

        for (source_id, targets) in pending_supersedes {
            for target_id in targets {
                if target_id != source_id && existing_ids.contains(&target_id) {
                    let _ = client
                        .mutate(&ChangeRequest {
                            query: GQ_LINK_RELATES_TO.into(),
                            name: Some("link_relates_to".into()),
                            params: Some(json!({
                                "source_id": source_id,
                                "target_id": target_id,
                                "relation": "supersedes",
                                "note": "Superseded in canonical knowledge sync",
                                "confidence": 1.0,
                                "created_at": ts,
                            })),
                            branch: Some(branch.to_string()),
                            settings: None,
                        })
                        .await;
                }
            }
        }

        let mut deleted_atom_ids = Vec::new();
        let mut prune_error = None;
        if !prune_layers.is_empty() {
            let current = atom_ids.iter().cloned().collect::<BTreeSet<_>>();
            match collect_branch_atom_rows(&client, project_id, branch).await {
                Ok(rows) => {
                    for row in rows {
                        let Some(atom_id) = str_field(&row, "atom_id") else {
                            continue;
                        };
                        if current.contains(&atom_id)
                            || !synced_atom_of(&row, &atom_id, project_id, prune_layers)
                        {
                            continue;
                        }
                        match client
                            .mutate(&ChangeRequest {
                                query: GQ_DELETE_CANONICAL_ATOM.into(),
                                name: Some("delete_canonical_atom".into()),
                                params: Some(json!({ "atom_id": atom_id })),
                                branch: Some(branch.to_string()),
                                settings: None,
                            })
                            .await
                        {
                            Ok(_) => deleted_atom_ids.push(atom_id),
                            Err(error) => {
                                prune_error = Some(error.to_string());
                                break;
                            }
                        }
                    }
                }
                Err(error) => prune_error = Some(error.to_string()),
            }
        }

        let mut synced = json!({
            "ok": true,
            "branch": branch,
            "synced": atom_ids.len(),
            "symbols": symbol_count,
            "atom_ids": atom_ids,
            "deleted": deleted_atom_ids.len(),
            "deleted_atom_ids": deleted_atom_ids,
        });
        if let Some(error) = prune_error {
            synced["prune_error"] = json!(error);
        }
        Ok(synced)
    }

    /// The context atoms that touch one file, on `main` and on the canonical
    /// knowledge branch, each once: what the shared graph knows about it,
    /// the declarations of the file each names (`symbols`), and the
    /// `contradicts` relations with one of them at either end.
    pub async fn file_context_json(&self, project_id: &str, path: &str) -> Result<Value> {
        let path = path.trim().trim_start_matches("./");
        if path.is_empty()
            || path.starts_with('/')
            || path.contains('\\')
            || path.split('/').any(|part| part == "..")
        {
            return Err(HarnessError::invalid("Choose a file of the project."));
        }
        let cfg = self.config().await;
        if cfg.mode == OmnigraphMode::Off {
            return Ok(json!({"ok": false, "skipped": "omnigraph-off", "atoms": []}));
        }
        let client = self.client().await?;
        let branches = ["main".to_string(), CANONICAL_KNOWLEDGE_BRANCH.to_string()];
        let mut atoms = Vec::new();
        let mut seen = BTreeSet::new();
        for branch in &branches {
            let Ok(found) = run_stored_or_inline(
                &client,
                "file_context",
                schema::QUERY_FILE_CONTEXT,
                json!({ "path": path }),
                Some(branch.clone()),
                None,
            )
            .await
            else {
                continue;
            };
            if found.rows.is_empty() {
                continue;
            }
            let mut named: BTreeMap<String, Vec<String>> = BTreeMap::new();
            let prefix = format!("{path}::");
            for edge in symbol_edges(&client, project_id, branch).await {
                if edge["file_path"].as_str() == Some(path)
                    && let (Some(aid), Some(fqn)) =
                        (str_field(&edge, "source_id"), str_field(&edge, "target_id"))
                {
                    let label = fqn.strip_prefix(&prefix).unwrap_or(&fqn).to_string();
                    let labels = named.entry(aid).or_default();
                    if !labels.contains(&label) {
                        labels.push(label);
                    }
                }
            }
            for row in &found.rows {
                let mut atom = normalize_atom_row(row);
                if let Some(aid) = str_field(&atom, "atom_id")
                    && seen.insert(aid.clone())
                {
                    if atom["source_branch"].is_null() {
                        atom["source_branch"] = json!(branch);
                    }
                    atom["capsule_line"] = json!(format_atom_line_with_authority(&atom).trim_end());
                    atom["symbols"] = json!(named.remove(&aid).unwrap_or_default());
                    atoms.push(atom);
                }
            }
        }
        let contradictions = contradictions_touching(&client, project_id, &branches, &seen).await;
        Ok(json!({
            "ok": true,
            "path": path,
            "atoms": atoms,
            "contradictions": contradictions,
        }))
    }

    /// Locate `atom_id` on `branch` (or across `"main"`, `CANONICAL_KNOWLEDGE_BRANCH`, and listed branches)
    /// and attach its `TouchesFile` paths from `GQ_LIST_FILE_EDGES`.
    pub async fn fetch_atom_with_files(
        &self,
        project_id: &str,
        atom_id: &str,
        branch: Option<&str>,
    ) -> Result<Value> {
        let atom_id = atom_id.trim();
        if atom_id.is_empty() {
            return Err(HarnessError::invalid("`atom_id` is required."));
        }
        let client = self.client().await?;
        let mut candidate_branches = Vec::new();
        if let Some(b) = clean_opt(branch).filter(|b| *b != "auto") {
            candidate_branches.push(validate_branch_token(b)?);
        }
        for default_branch in ["main", CANONICAL_KNOWLEDGE_BRANCH] {
            if !candidate_branches.iter().any(|b| b == default_branch) {
                candidate_branches.push(default_branch.to_string());
            }
        }
        if let Ok(list) = client.list_branches(false).await {
            for b in list.branches {
                if !candidate_branches.contains(&b) {
                    candidate_branches.push(b);
                }
            }
        }

        for resolved_branch in candidate_branches {
            let Some(mut atom) =
                find_atom_on_branch(&client, project_id, &resolved_branch, atom_id).await
            else {
                continue;
            };
            let edge_pid = str_field(&atom, "project_id").unwrap_or_else(|| project_id.to_string());
            let files = list_atom_files(&client, &edge_pid, &resolved_branch, atom_id).await;
            atom["files"] = json!(files);
            return Ok(json!({
                "atom": atom,
                "branch": resolved_branch,
                "files": files,
            }));
        }

        Err(HarnessError::missing(format!(
            "Omnigraph ContextAtom '{atom_id}' was not found."
        )))
    }

    /// Re-upsert `atom` on `branch` preserving content, tags, and files while linking it to a
    /// verified `Project Brain` record (`origin_layer: "project-brain"`, `authority: "verified"`, `verified: true`).
    pub async fn link_atom_to_brain(
        &self,
        project_id: &str,
        branch: &str,
        atom: &Value,
        brain_record_id: &str,
        revision: i64,
        privacy: Option<&str>,
    ) -> Result<Value> {
        let atom_id = str_field(atom, "atom_id")
            .or_else(|| str_field(atom, "id"))
            .ok_or_else(|| HarnessError::invalid("`atom` is missing `atom_id`."))?;
        let branch = validate_branch_token(branch)?;
        let brain_record_id = clean_opt(Some(brain_record_id))
            .ok_or_else(|| HarnessError::invalid("`brain_record_id` is required."))?;
        let effective_project_id = clean_opt(Some(project_id))
            .or_else(|| clean_opt(atom["project_id"].as_str()))
            .unwrap_or("");
        let raw_kind = str_field(atom, "kind")
            .unwrap_or_else(|| "insight".to_string())
            .to_ascii_lowercase();
        let kind = if VALID_ATOM_KINDS.contains(&raw_kind.as_str()) {
            raw_kind
        } else {
            "insight".to_string()
        };
        let title = str_field(atom, "title").unwrap_or_else(|| atom_id.clone());
        let summary = str_field(atom, "summary").unwrap_or_else(|| title.clone());
        let body = str_field(atom, "body").unwrap_or_else(|| summary.clone());
        let tags = parse_tags_opt(&atom["tags"]);
        let source_branch = str_field(atom, "source_branch")
            .or_else(|| str_field(atom, "branch"))
            .unwrap_or_else(|| branch.clone());
        let source_session_id =
            str_field(atom, "source_session_id").or_else(|| str_field(atom, "session_id"));
        let confidence = atom["confidence"].as_f64().unwrap_or(0.9).clamp(0.0, 1.0);
        let pin_priority = atom["pin_priority"].as_i64().unwrap_or(15) as i32;
        let created_at = str_field(atom, "created_at").unwrap_or_else(now_utc_iso_ms);
        let updated_at = now_utc_iso_ms();

        let client = self.client().await?;
        ensure_branch_exists(&client, &branch, Some("main")).await?;

        let mut files = parse_string_list(&atom["files"]);
        if files.is_empty() {
            files = list_atom_files(&client, effective_project_id, &branch, &atom_id).await;
        }

        let mut prov = AtomProvenance::from_value(atom);
        prov.origin_layer = Some("project-brain".to_string());
        prov.authority = Some("verified".to_string());
        prov.brain_record_id = Some(brain_record_id.to_string());
        prov.record_revision = Some(revision);
        if let Some(p) = clean_opt(privacy) {
            prov.privacy = Some(p.to_string());
        }

        let atom_params = build_upsert_atom_params(
            &atom_id,
            effective_project_id,
            &kind,
            &title,
            &summary,
            &body,
            tags,
            &source_branch,
            source_session_id.as_deref(),
            confidence,
            pin_priority,
            true,
            &prov,
            &created_at,
            &updated_at,
        );
        let mut last_commit = client
            .mutate(&ChangeRequest {
                query: GQ_UPSERT_ATOM.into(),
                name: Some("upsert_atom".into()),
                params: Some(atom_params.clone()),
                branch: Some(branch.clone()),
                settings: None,
            })
            .await?
            .commit;

        if let Some(commit) = upsert_atom_files(
            &client,
            effective_project_id,
            &branch,
            &atom_id,
            &files,
            &updated_at,
        )
        .await
        {
            last_commit = Some(commit);
        }

        let mut atom_resp = normalize_atom_row(&atom_params);
        atom_resp["files"] = json!(files);
        Ok(json!({
            "ok": true,
            "atom_id": atom_id,
            "branch": branch,
            "brain_record_id": brain_record_id,
            "record_revision": revision,
            "origin_layer": "project-brain",
            "authority": "verified",
            "privacy": prov.privacy,
            "verified": true,
            "atom": atom_resp,
            "files": files,
            "commit": last_commit,
        }))
    }

    fn resolve_session_branch(&self, sid: &str) -> String {
        self.store
            .as_ref()
            .and_then(|s| s.get(sid).ok())
            .and_then(|sess| {
                clean_opt(sess["omnigraph"]["branch"].as_str())
                    .filter(|b| *b != "auto")
                    .map(str::to_string)
            })
            .unwrap_or_else(|| session_branch_name(sid))
    }
}

/// Discovered local `omnigraph` CLI and `omnigraph-server` binary locations.
#[derive(Clone, Debug, Default)]
pub struct BinaryDiscovery {
    pub cli: Option<PathBuf>,
    pub server: Option<PathBuf>,
    pub version: Option<String>,
}

/// Harness `Domain` router serving `omnigraph.*` operations.
pub struct OmnigraphDomain {
    pub manager: Arc<OmnigraphManager>,
}

impl OmnigraphDomain {
    pub fn new(manager: Arc<OmnigraphManager>) -> Self {
        Self { manager }
    }
}

impl Domain for OmnigraphDomain {
    fn execute<'a>(&'a self, operation: &'a str, request: Value) -> Reply<'a> {
        Box::pin(async move {
            let params = &request["params"];
            let query = &request["query"];
            let body = &request["body"];
            let project_id = query["project_id"]
                .as_str()
                .or_else(|| params["project_id"].as_str())
                .or_else(|| body["project_id"].as_str())
                .unwrap_or("");

            let result = match operation {
                "omnigraph.status" => self.manager.status_json(project_id).await?,
                "omnigraph.setup" => self.manager.setup_json(body).await?,
                "omnigraph.export" => {
                    return self.manager.export_context_payload(query, body).await;
                }
                "omnigraph.import" => self.manager.import_context_json(body).await?,
                "omnigraph.link_sessions" => {
                    let store = self.manager.store.as_ref().ok_or_else(|| {
                        HarnessError::unavailable(
                            "Session store is required for omnigraph.link_sessions.",
                        )
                    })?;
                    self.manager.link_sessions_json(body, store).await?
                }
                "omnigraph.graph" => {
                    let limit = parse_opt_usize(&query["limit"], "limit")?;
                    let offset = parse_opt_usize(&query["offset"], "offset")?;
                    let cursor = clean_opt(query["cursor"].as_str());
                    self.manager
                        .graph_view_json_paginated(
                            project_id,
                            query["branch"].as_str(),
                            query["snapshot"].as_str(),
                            query["kind"].as_str(),
                            query["q"].as_str(),
                            query["session_id"].as_str(),
                            limit,
                            offset,
                            cursor,
                        )
                        .await?
                }
                "omnigraph.search" => {
                    let q = query["q"].as_str().unwrap_or("");
                    let limit = parse_limit(&query["limit"]);
                    self.manager
                        .search_json(
                            project_id,
                            q,
                            query["branch"].as_str(),
                            query["kind"].as_str(),
                            limit,
                        )
                        .await?
                }
                "omnigraph.branch_diff" => {
                    let source = query["source"].as_str().unwrap_or("");
                    self.manager
                        .branch_diff_json(project_id, source, query["target"].as_str())
                        .await?
                }
                "omnigraph.branch_act" => self.manager.branch_action_json(body).await?,
                "omnigraph.publish" => self.manager.publish_atom_json(body).await?,
                "omnigraph.share" => self.manager.share_into_session_json(body).await?,
                "omnigraph.sync_items" => {
                    let pid = body["project_id"]
                        .as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or(project_id);
                    let items = body["items"].as_array().ok_or_else(|| {
                        HarnessError::invalid("`items` must be an array for omnigraph.sync_items.")
                    })?;
                    let prune_layers = body["prune_layers"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .filter(|layer| matches!(*layer, "project-brain" | "memory-bank"))
                        .map(String::from)
                        .collect::<Vec<_>>();
                    self.manager
                        .sync_canonical_knowledge_with_options(pid, items, &prune_layers)
                        .await?
                }
                "omnigraph.file_context" => {
                    let pid = query["project_id"]
                        .as_str()
                        .or_else(|| body["project_id"].as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or(project_id);
                    let path = query["path"]
                        .as_str()
                        .or_else(|| body["path"].as_str())
                        .unwrap_or("");
                    self.manager.file_context_json(pid, path).await?
                }
                "omnigraph.fetch_atom" => {
                    let pid = body["project_id"]
                        .as_str()
                        .or_else(|| query["project_id"].as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or(project_id);
                    let atom_id = body["atom_id"]
                        .as_str()
                        .or_else(|| query["atom_id"].as_str())
                        .or_else(|| params["atom_id"].as_str())
                        .unwrap_or("");
                    let branch = body["branch"].as_str().or_else(|| query["branch"].as_str());
                    self.manager
                        .fetch_atom_with_files(pid, atom_id, branch)
                        .await?
                }
                "omnigraph.link_brain" => {
                    let pid = body["project_id"]
                        .as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or(project_id);
                    let branch = body["branch"].as_str().unwrap_or("");
                    let atom = &body["atom"];
                    if !atom.is_object() {
                        return Err(HarnessError::invalid(
                            "`atom` must be an object for omnigraph.link_brain.",
                        ));
                    }
                    let brain_record_id = body["brain_record_id"].as_str().unwrap_or("");
                    let privacy = clean_opt(body["privacy"].as_str());
                    let revision =
                        body.get("revision")
                            .and_then(parse_i64_opt)
                            .ok_or_else(|| {
                                HarnessError::invalid(
                                    "`revision` must be an integer for omnigraph.link_brain.",
                                )
                            })?;
                    self.manager
                        .link_atom_to_brain(pid, branch, atom, brain_record_id, revision, privacy)
                        .await?
                }
                "omnigraph.changes" => {
                    self.manager
                        .changes_json(
                            project_id,
                            query["cursor"].as_str(),
                            query["branch"].as_str(),
                        )
                        .await?
                }
                "omnigraph.commit_detail" => {
                    let commit_id = params["commit_id"]
                        .as_str()
                        .or_else(|| query["commit_id"].as_str())
                        .unwrap_or("");
                    self.manager
                        .commit_detail_json(project_id, commit_id)
                        .await?
                }
                _ => return Err(HarnessError::missing("Unknown Omnigraph operation.")),
            };
            Ok(Payload::Json(result))
        })
    }
}

fn empty_graph_view(
    graph_id: &str,
    active_branch: &str,
    snapshot_commit: Option<String>,
    limit: usize,
    offset: usize,
    elapsed_ms: u64,
) -> Value {
    json!({
        "connected": false,
        "graph_id": graph_id,
        "active_branch": active_branch,
        "snapshot_commit": snapshot_commit,
        "head_commit": Value::Null,
        "branches": [],
        "commits": [],
        "sessions": [],
        "atoms": [],
        "files": [],
        "symbols": [],
        "edges": [],
        "symbol_edges": [],
        "total_atoms": 0,
        "limit": limit,
        "offset": offset,
        "has_more": false,
        "truncated": false,
        "next_cursor": Value::Null,
        "telemetry": {
            "elapsed_ms": elapsed_ms,
            "atom_count": 0,
            "total_atoms": 0,
            "session_count": 0,
            "file_count": 0,
            "symbol_count": 0,
            "edge_count": 0,
            "symbol_edge_count": 0,
            "commit_count": 0,
            "truncated": false,
            "has_more": false,
        },
    })
}

async fn compute_stats(client: &OmnigraphClient, project_id: &str) -> Result<Value> {
    let params = json!({ "project_id": project_id });
    let commit_query = CommitListQuery {
        branch: Some("main".into()),
        limit: Some(100),
        page_token: None,
    };
    let (
        branches_res,
        commits_res,
        atoms_res,
        sessions_res,
        symbols_rows,
        pub_edges,
        con_edges,
        file_edges,
        rel_edges,
        follow_edges,
        symbol_edges,
    ) = tokio::join!(
        client.list_branches(true),
        client.list_commits(&commit_query),
        run_recent_atoms_query(client, project_id, Some("main".into()), None),
        query_edges_safe(
            client,
            GQ_LIST_SESSIONS,
            "list_sessions",
            &params,
            Some("main"),
            None,
        ),
        query_edges_safe(
            client,
            GQ_LIST_SYMBOLS,
            "list_symbols",
            &params,
            Some("main"),
            None,
        ),
        query_edges_safe(
            client,
            GQ_LIST_PUBLISHED_EDGES,
            "list_published_edges",
            &params,
            Some("main"),
            None,
        ),
        query_edges_safe(
            client,
            GQ_LIST_CONSUMED_EDGES,
            "list_consumed_edges",
            &params,
            Some("main"),
            None,
        ),
        query_edges_safe(
            client,
            GQ_LIST_FILE_EDGES,
            "list_file_edges",
            &params,
            Some("main"),
            None,
        ),
        query_edges_safe(
            client,
            GQ_LIST_ATOM_RELATIONS,
            "list_atom_relations",
            &params,
            Some("main"),
            None,
        ),
        query_edges_safe(
            client,
            GQ_LIST_SESSION_FOLLOWS,
            "list_session_follows",
            &params,
            Some("main"),
            None,
        ),
        query_edges_safe(
            client,
            GQ_LIST_SYMBOL_EDGES,
            "list_symbol_edges",
            &params,
            Some("main"),
            None,
        ),
    );

    let branches_resp = branches_res?;
    let branches_count = branches_resp.branches.len();
    let main_head = branches_resp
        .details
        .as_ref()
        .and_then(|ds| ds.iter().find(|d| d.name == "main"))
        .map(|d| d.head.graph_commit_id.as_str());
    let unmerged_branches = branches_resp
        .details
        .as_ref()
        .map(|ds| {
            ds.iter()
                .filter(|d| d.name != "main" && Some(d.head.graph_commit_id.as_str()) != main_head)
                .count()
        })
        .unwrap_or_else(|| branches_count.saturating_sub(1));

    let commits_count = commits_res.map(|c| c.commits.len()).unwrap_or(0);
    let atoms_count = atoms_res.map(|r| r.rows.len()).unwrap_or(0);
    let sessions_count = sessions_res.len();
    let symbols_count = symbols_rows.len();
    let edges_count = pub_edges.len()
        + con_edges.len()
        + file_edges.len()
        + rel_edges.len()
        + follow_edges.len()
        + symbol_edges.len();

    Ok(json!({
        "branches": branches_count,
        "atoms": atoms_count,
        "sessions": sessions_count,
        "symbols": symbols_count,
        "edges": edges_count,
        "commits": commits_count,
        "unmerged_branches": unmerged_branches,
    }))
}

async fn run_stored_or_inline(
    client: &OmnigraphClient,
    stored_name: &str,
    inline_gq: &str,
    params: Value,
    branch: Option<String>,
    snapshot: Option<String>,
) -> Result<super::client::QueryResponse> {
    match client
        .stored_query(
            stored_name,
            &StoredQueryRequest {
                params: Some(params.clone()),
                branch: branch.clone(),
                snapshot: snapshot.clone(),
                expect_mutation: Some(false),
            },
        )
        .await
    {
        Ok(resp) => Ok(resp),
        Err(err) if err.is_not_found() => client
            .query(&QueryRequest {
                query: inline_gq.to_string(),
                name: Some(stored_name.to_string()),
                params: Some(params),
                branch,
                snapshot,
                settings: None,
            })
            .await
            .map_err(Into::into),
        Err(err) => Err(err.into()),
    }
}

/// Session text the shared graph may hold: screened as an automatic memory
/// query (secrets, personal data and transcript markers removed), or none.
fn graph_session_text(raw: &str) -> Option<String> {
    crate::brain_settings::query(raw)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// `SessionFollows.relation` says how two sessions are linked: sessions that
/// share a space are on a shared branch, others are linked. The space's own
/// name stays on the session, never on the edge.
pub(crate) fn canonical_session_follows_relation(omnigraph: &Value) -> &'static str {
    if clean_opt(omnigraph["shared_space"].as_str()).is_some() {
        "shared_branch"
    } else {
        "linked"
    }
}

/// The `turn_index` after the session's latest snapshot on `branch`, or 1.
async fn next_turn_index(client: &OmnigraphClient, sid: &str, branch: &str) -> i32 {
    query_edges_safe(
        client,
        GQ_LIST_SESSION_TURN_SNAPSHOTS,
        "list_session_turn_snapshots",
        &json!({ "session_id": sid }),
        Some(branch),
        None,
    )
    .await
    .iter()
    .filter_map(|row| row["turn_index"].as_i64())
    .max()
    .map_or(1, |latest| {
        i32::try_from(latest.saturating_add(1)).unwrap_or(i32::MAX)
    })
}

async fn query_edges_safe(
    client: &OmnigraphClient,
    query: &str,
    name: &str,
    params: &Value,
    branch: Option<&str>,
    snapshot: Option<&str>,
) -> Vec<Value> {
    client
        .query(&QueryRequest {
            query: query.to_string(),
            name: Some(name.to_string()),
            params: Some(params.clone()),
            branch: branch.map(str::to_string),
            snapshot: snapshot.map(str::to_string),
            settings: None,
        })
        .await
        .map(|r| r.rows)
        .unwrap_or_default()
}

async fn upsert_session_node(
    client: &OmnigraphClient,
    session_meta: Option<&Value>,
    sid: &str,
    project_id: &str,
    branch: &str,
    ts: &str,
) -> Result<super::client::ChangeResponse> {
    let sess_project_id = session_meta
        .and_then(|s| s["project_id"].as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(project_id);
    let sess_title = session_meta
        .and_then(|s| s["title"].as_str())
        .unwrap_or("Harness Session");
    let sess_provider = session_meta
        .and_then(|s| s["provider"].as_str())
        .unwrap_or("codex");
    let sess_workflow = session_meta
        .and_then(|s| s["workflow"].as_str())
        .unwrap_or("native");
    let sess_status = session_meta
        .and_then(|s| s["status"].as_str())
        .unwrap_or("running");
    let sess_branch = session_meta
        .and_then(|s| clean_opt(s["omnigraph"]["branch"].as_str()))
        .filter(|b| *b != "auto")
        .map(str::to_string)
        .unwrap_or_else(|| session_branch_name(sid));
    let goal = session_meta.and_then(|s| {
        [s["omnigraph"]["query"].as_str(), s["goal"].as_str()]
            .into_iter()
            .flatten()
            .find_map(graph_session_text)
    });
    client
        .mutate(&ChangeRequest {
            query: GQ_UPSERT_SESSION.into(),
            name: Some("upsert_session".into()),
            params: Some(json!({
                "session_id": sid,
                "project_id": sess_project_id,
                "title": sess_title,
                "branch": sess_branch,
                "provider": sess_provider,
                "workflow": sess_workflow,
                "status": sess_status,
                "goal": goal,
                "created_at": ts,
                "updated_at": ts,
            })),
            branch: Some(branch.to_string()),
            settings: None,
        })
        .await
        .map_err(Into::into)
}

async fn link_session_follows(
    client: &OmnigraphClient,
    from_sid: &str,
    to_sid: &str,
    relation: &str,
    branch: &str,
    ts: &str,
) -> Result<super::client::ChangeResponse> {
    client
        .mutate(&ChangeRequest {
            query: GQ_LINK_SESSION_FOLLOWS.into(),
            name: Some("link_session_follows".into()),
            params: Some(json!({
                "from_session_id": from_sid,
                "to_session_id": to_sid,
                "relation": relation,
                "created_at": ts,
            })),
            branch: Some(branch.to_string()),
            settings: None,
        })
        .await
        .map_err(Into::into)
}

async fn link_consumed_by(
    client: &OmnigraphClient,
    atom_id: &str,
    session_id: &str,
    injection_mode: &str,
    branch: &str,
    ts: &str,
) -> Result<super::client::ChangeResponse> {
    client
        .mutate(&ChangeRequest {
            query: GQ_LINK_CONSUMED_BY.into(),
            name: Some("link_consumed_by".into()),
            params: Some(json!({
                "atom_id": atom_id,
                "session_id": session_id,
                "consumed_at": ts,
                "injection_mode": injection_mode,
                "relevance": 1.0,
            })),
            branch: Some(branch.to_string()),
            settings: None,
        })
        .await
        .map_err(Into::into)
}

async fn fetch_branch_atoms(
    client: &OmnigraphClient,
    project_id: &str,
    branch: &str,
) -> Result<Vec<Value>> {
    let resp = run_stored_or_inline(
        client,
        "recent_atoms",
        schema::QUERY_RECENT_ATOMS,
        json!({ "project_id": project_id }),
        Some(branch.to_string()),
        None,
    )
    .await?;
    Ok(resp.rows.iter().map(normalize_atom_row).collect())
}

async fn merge_or_sync_branch(
    client: &OmnigraphClient,
    source_branch: &str,
    target_branch: &str,
    project_id: &str,
) -> Result<(Option<CommitOutput>, Vec<Value>)> {
    if source_branch == target_branch {
        return Ok((None, Vec::new()));
    }
    match client
        .merge_branches(&BranchMergeRequest {
            source: source_branch.to_string(),
            target: Some(target_branch.to_string()),
            delete_branch: false,
            as_commit: false,
            dry_run: false,
            settings: None,
        })
        .await
    {
        Ok(resp) => Ok((resp.commit, Vec::new())),
        Err(err) if err.is_not_found() => Ok((None, Vec::new())),
        Err(err) if err.is_merge_conflict() => {
            let mut conflicts = Vec::new();
            let mut excluded_atoms = BTreeSet::new();
            for mc in &err.merge_conflicts {
                if mc.type_name == "ContextAtom" {
                    if let Some(eid) = mc.entity_id.as_ref().filter(|s| !s.is_empty()) {
                        excluded_atoms.insert(eid.clone());
                    }
                    if let Ok(v) = serde_json::to_value(mc) {
                        conflicts.push(v);
                    }
                }
            }
            let exported = client
                .export_branch(Some(source_branch), &[], None)
                .await
                .unwrap_or_default();
            let filtered_ndjson =
                filter_ndjson_excluding_atoms(&exported, project_id, &excluded_atoms);
            let mut commit_out = None;
            if !filtered_ndjson.trim().is_empty()
                && let Ok(load_res) = client
                    .load_ndjson(target_branch, None, "merge", &filtered_ndjson, None)
                    .await
                && let Ok(parsed_commit) =
                    serde_json::from_value::<CommitOutput>(load_res["commit"].clone())
            {
                commit_out = Some(parsed_commit);
            }
            Ok((commit_out, conflicts))
        }
        Err(err) => Err(err.into()),
    }
}

async fn ensure_branch_exists(
    client: &OmnigraphClient,
    branch: &str,
    from: Option<&str>,
) -> Result<()> {
    if branch == "main" {
        return Ok(());
    }
    match client
        .create_branch(&BranchCreateRequest {
            name: branch.to_string(),
            from: from.map(str::to_string),
            from_commit: None,
        })
        .await
    {
        Ok(_) => Ok(()),
        Err(err) if err.status == 409 => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn copy_alias(obj: &mut Map<String, Value>, a: &str, b: &str) {
    let a_val = obj.get(a).filter(|v| !v.is_null()).cloned();
    let b_val = obj.get(b).filter(|v| !v.is_null()).cloned();
    match (a_val, b_val) {
        (Some(v), None) => {
            obj.insert(b.to_string(), v);
        }
        (None, Some(v)) => {
            obj.insert(a.to_string(), v);
        }
        (None, None) => {
            if obj.contains_key(a) && !obj.contains_key(b) {
                obj.insert(b.to_string(), Value::Null);
            } else if obj.contains_key(b) && !obj.contains_key(a) {
                obj.insert(a.to_string(), Value::Null);
            }
        }
        (Some(_), Some(_)) => {}
    }
}

#[derive(Clone, Debug, Default)]
struct AtomProvenance {
    origin_layer: Option<String>,
    authority: Option<String>,
    privacy: Option<String>,
    brain_record_id: Option<String>,
    memory_id: Option<String>,
    record_revision: Option<i64>,
}

impl AtomProvenance {
    fn from_value(v: &Value) -> Self {
        Self {
            origin_layer: str_field(v, "origin_layer"),
            authority: str_field(v, "authority"),
            privacy: str_field(v, "privacy"),
            brain_record_id: str_field(v, "brain_record_id"),
            memory_id: str_field(v, "memory_id"),
            record_revision: v.get("record_revision").and_then(parse_i64_opt),
        }
    }

    fn apply_to_map(&self, obj: &mut Map<String, Value>) {
        let str_or_null = |opt: &Option<String>| {
            opt.as_ref()
                .map(|s| Value::String(s.clone()))
                .unwrap_or(Value::Null)
        };
        obj.insert("origin_layer".into(), str_or_null(&self.origin_layer));
        obj.insert("authority".into(), str_or_null(&self.authority));
        obj.insert("privacy".into(), str_or_null(&self.privacy));
        obj.insert("brain_record_id".into(), str_or_null(&self.brain_record_id));
        obj.insert("memory_id".into(), str_or_null(&self.memory_id));
        obj.insert(
            "record_revision".into(),
            self.record_revision
                .map(|n| Value::Number(n.into()))
                .unwrap_or(Value::Null),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn build_upsert_atom_params(
    atom_id: &str,
    project_id: &str,
    kind: &str,
    title: &str,
    summary: &str,
    body: &str,
    tags: Option<Vec<String>>,
    source_branch: &str,
    source_session_id: Option<&str>,
    confidence: f64,
    pin_priority: i32,
    verified: bool,
    provenance: &AtomProvenance,
    created_at: &str,
    updated_at: &str,
) -> Value {
    let mut val = json!({
        "atom_id": atom_id,
        "project_id": project_id,
        "kind": kind,
        "title": title,
        "summary": summary,
        "body": body,
        "tags": tags,
        "source_branch": source_branch,
        "source_session_id": source_session_id,
        "confidence": confidence,
        "pin_priority": pin_priority,
        "verified": verified,
        "created_at": created_at,
        "updated_at": updated_at,
    });
    if let Some(obj) = val.as_object_mut() {
        provenance.apply_to_map(obj);
    }
    val
}

fn parse_string_list(val: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for item in val.as_array().into_iter().flatten() {
        if let Some(s) = item.as_str().map(str::trim).filter(|s| !s.is_empty())
            && !out.iter().any(|existing| existing == s)
        {
            out.push(s.to_string());
        }
    }
    out
}

fn parse_tags_opt(val: &Value) -> Option<Vec<String>> {
    val.as_array().map(|arr| {
        arr.iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    })
}

async fn upsert_atom_files(
    client: &OmnigraphClient,
    project_id: &str,
    branch: &str,
    atom_id: &str,
    files: &[String],
    ts: &str,
) -> Option<CommitOutput> {
    let mut last_commit = None;
    for file_path in files {
        let lang = detect_language(file_path);
        let _ = client
            .mutate(&ChangeRequest {
                query: GQ_UPSERT_FILE.into(),
                name: Some("upsert_file".into()),
                params: Some(json!({
                    "path": file_path,
                    "project_id": project_id,
                    "language": lang,
                    "last_touched_at": ts,
                })),
                branch: Some(branch.to_string()),
                settings: None,
            })
            .await;
        if let Ok(r) = client
            .mutate(&ChangeRequest {
                query: GQ_LINK_TOUCHES_FILE.into(),
                name: Some("link_touches_file".into()),
                params: Some(json!({
                    "atom_id": atom_id,
                    "path": file_path,
                    "access": "referenced",
                    "line_start": Value::Null,
                    "line_end": Value::Null,
                })),
                branch: Some(branch.to_string()),
                settings: None,
            })
            .await
            && r.commit.is_some()
        {
            last_commit = r.commit;
        }
    }
    last_commit
}

/// The declarations `text` names in the files an atom touches, read from the
/// fresh `GraphIrStore` when available (canonical `symbol_id`), falling back to
/// `graph_live::declarations_named_in` on `root`.
fn atom_symbols_for_project(
    state_dir: Option<&Path>,
    project_id: &str,
    root: Option<&Path>,
    files: &[String],
    text: &str,
) -> Vec<Value> {
    let Some(root) = root else {
        return Vec::new();
    };
    let ir_store = state_dir
        .filter(|_| !project_id.is_empty())
        .and_then(|state| {
            crate::graph_ir::GraphIrStore::open_readonly(state, project_id)
                .ok()
                .flatten()
        });
    let mut symbols = Vec::new();
    let format_fqn = |rel: &str, owner: Option<&str>, name: &str| -> String {
        match owner.filter(|o| !o.is_empty() && *o != rel) {
            Some(owner) => format!("{rel}::{owner}::{name}"),
            None => format!("{rel}::{name}"),
        }
    };
    for file in files.iter().take(MAX_SYMBOL_FILES) {
        let rel_key = crate::graph_live::clean_rel_key(file.split('#').next().unwrap_or(file));
        if !rel_key.is_empty()
            && let Some(ref ir) = ir_store
            && let Ok(Some(ir_syms)) = ir.fresh_file_symbols(root, &rel_key)
        {
            let mut seen_names: BTreeSet<(Option<String>, String)> = BTreeSet::new();
            for (symbol_id, kind, name, parent_name, start_line) in ir_syms {
                if symbols.len() >= MAX_ATOM_SYMBOLS {
                    break;
                }
                if kind == "file"
                    || name.chars().count() < 3
                    || !crate::graph_live::names_declaration(text, &name)
                    || !seen_names.insert((parent_name.clone(), name.clone()))
                {
                    continue;
                }
                let fqn = format_fqn(&rel_key, parent_name.as_deref(), &name);
                symbols.push(json!({
                    "fqn": fqn,
                    "symbol_id": symbol_id,
                    "file_path": rel_key,
                    "kind": kind,
                    "name": name,
                    "line": start_line,
                }));
            }
            if symbols.len() >= MAX_ATOM_SYMBOLS {
                break;
            }
            continue;
        }
        let Some((rel, source)) = crate::graph_live::read_workspace_file(root, file) else {
            continue;
        };
        let room = MAX_ATOM_SYMBOLS - symbols.len();
        for entity in crate::graph_live::declarations_named_in(&rel, &source, text, room) {
            let fqn = format_fqn(&rel, entity.parent.as_deref(), &entity.name);
            let mut row = json!({
                "fqn": fqn,
                "file_path": rel,
                "kind": entity.kind,
                "name": entity.name,
            });
            if !entity.id.is_empty() {
                row["symbol_id"] = json!(entity.id);
            }
            if let Some(line) = entity.line {
                row["line"] = json!(line);
            }
            symbols.push(row);
        }
        if symbols.len() >= MAX_ATOM_SYMBOLS {
            break;
        }
    }
    symbols
}

#[cfg(test)]
#[allow(dead_code)]
fn atom_symbols(root: Option<&Path>, files: &[String], text: &str) -> Vec<Value> {
    atom_symbols_for_project(None, "", root, files, text)
}

/// Links `atom_id` to `symbols` on `branch`. The mentions it had
/// (`previous`) are replaced when the set differs; the same set writes
/// nothing.
async fn upsert_atom_symbols(
    client: &OmnigraphClient,
    project_id: &str,
    branch: &str,
    atom_id: &str,
    symbols: &[Value],
    previous: &BTreeSet<String>,
) -> Option<CommitOutput> {
    let wanted = symbols
        .iter()
        .filter_map(|symbol| str_field(symbol, "fqn"))
        .collect::<BTreeSet<_>>();
    if wanted == *previous {
        return None;
    }
    let mut last_commit = None;
    if !previous.is_empty()
        && let Ok(r) = client
            .mutate(&ChangeRequest {
                query: GQ_CLEAR_ATOM_SYMBOLS.into(),
                name: Some("clear_atom_symbols".into()),
                params: Some(json!({ "atom_id": atom_id })),
                branch: Some(branch.to_string()),
                settings: None,
            })
            .await
        && r.commit.is_some()
    {
        last_commit = r.commit;
    }
    for symbol in symbols {
        let Some(fqn) = str_field(symbol, "fqn") else {
            continue;
        };
        let _ = client
            .mutate(&ChangeRequest {
                query: GQ_UPSERT_SYMBOL.into(),
                name: Some("upsert_symbol".into()),
                params: Some(json!({
                    "fqn": fqn,
                    "project_id": project_id,
                    "file_path": symbol["file_path"],
                    "kind": symbol["kind"],
                    "name": symbol["name"],
                })),
                branch: Some(branch.to_string()),
                settings: None,
            })
            .await;
        if let Ok(r) = client
            .mutate(&ChangeRequest {
                query: GQ_LINK_MENTIONS_SYMBOL.into(),
                name: Some("link_mentions_symbol".into()),
                params: Some(json!({
                    "atom_id": atom_id,
                    "fqn": fqn,
                    "role": "named",
                })),
                branch: Some(branch.to_string()),
                settings: None,
            })
            .await
            && r.commit.is_some()
        {
            last_commit = r.commit;
        }
    }
    last_commit
}

/// The `MentionsSymbol` rows of the project's atoms on `branch`.
async fn symbol_edges(client: &OmnigraphClient, project_id: &str, branch: &str) -> Vec<Value> {
    query_edges_safe(
        client,
        GQ_LIST_SYMBOL_EDGES,
        "list_symbol_edges",
        &json!({ "project_id": project_id }),
        Some(branch),
        None,
    )
    .await
}

/// The declarations each of the project's atoms names on `branch`.
async fn list_atom_symbols(
    client: &OmnigraphClient,
    project_id: &str,
    branch: &str,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut named: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for edge in symbol_edges(client, project_id, branch).await {
        if let (Some(atom_id), Some(fqn)) =
            (str_field(&edge, "source_id"), str_field(&edge, "target_id"))
        {
            named.entry(atom_id).or_default().insert(fqn);
        }
    }
    named
}

/// The `contradicts` relations on `branches` with one of `atom_ids` at
/// either end, each pair once: what a person checks before relying on
/// either atom. Nothing is retired or changed.
async fn contradictions_touching(
    client: &OmnigraphClient,
    project_id: &str,
    branches: &[String],
    atom_ids: &BTreeSet<String>,
) -> Vec<Value> {
    let mut found = Vec::new();
    if atom_ids.is_empty() {
        return found;
    }
    let mut seen = BTreeSet::new();
    for branch in branches {
        let Ok(rows) = run_stored_or_inline(
            client,
            "contradictions",
            schema::QUERY_CONTRADICTIONS,
            json!({ "project_id": project_id }),
            Some(branch.clone()),
            None,
        )
        .await
        else {
            continue;
        };
        for row in &rows.rows {
            let (Some(source), Some(target)) =
                (str_field(row, "source_id"), str_field(row, "target_id"))
            else {
                continue;
            };
            if source == target
                || !(atom_ids.contains(&source) || atom_ids.contains(&target))
                || !seen.insert((source.clone(), target.clone()))
            {
                continue;
            }
            let source_title = str_field(row, "source_title").unwrap_or_else(|| source.clone());
            let target_title = str_field(row, "target_title").unwrap_or_else(|| target.clone());
            let note = str_field(row, "note");
            let text = match &note {
                Some(note) => format!(
                    "\"{source_title}\" contradicts \"{target_title}\" ({note}); check both against the code before relying on either."
                ),
                None => format!(
                    "\"{source_title}\" contradicts \"{target_title}\"; check both against the code before relying on either."
                ),
            };
            found.push(json!({
                "kind": "atom-contradiction",
                "source_id": source,
                "source_title": source_title,
                "target_id": target,
                "target_title": target_title,
                "note": note,
                "confidence": row["confidence"],
                "branch": branch,
                "recommended_action": "review",
                "text": text,
            }));
        }
    }
    found
}

/// The caution line under an injected atom that a `contradicts` relation
/// links to another atom, in either direction.
fn contradiction_caution(atom_id: &str, warnings: &[Value]) -> Option<String> {
    let parts = warnings
        .iter()
        .filter_map(|warning| {
            let (source, target) = (
                warning["source_id"].as_str()?,
                warning["target_id"].as_str()?,
            );
            let (verb, other, title) = if source == atom_id {
                ("contradicts", target, warning["target_title"].as_str())
            } else if target == atom_id {
                (
                    "is contradicted by",
                    source,
                    warning["source_title"].as_str(),
                )
            } else {
                return None;
            };
            let title = title
                .filter(|title| !title.trim().is_empty())
                .unwrap_or(other);
            Some(
                match warning["note"].as_str().filter(|n| !n.trim().is_empty()) {
                    Some(note) => format!("{verb} \"{title}\" (`{other}`: {note})"),
                    None => format!("{verb} \"{title}\" (`{other}`)"),
                },
            )
        })
        .collect::<Vec<_>>();
    (!parts.is_empty()).then(|| {
        format!(
            "  Caution: this {} — verify both against code and `project-brain/` before relying on either.\n",
            parts.join("; ")
        )
    })
}

async fn list_atom_files(
    client: &OmnigraphClient,
    project_id: &str,
    branch: &str,
    atom_id: &str,
) -> Vec<String> {
    let edges = query_edges_safe(
        client,
        GQ_LIST_FILE_EDGES,
        "list_file_edges",
        &json!({ "project_id": project_id }),
        Some(branch),
        None,
    )
    .await;
    let mut files = Vec::new();
    for edge in &edges {
        if str_field(edge, "source_id").as_deref() == Some(atom_id)
            && let Some(path) = str_field(edge, "target_id")
            && !files.contains(&path)
        {
            files.push(path);
        }
    }
    files
}

async fn collect_branch_atom_ids(
    client: &OmnigraphClient,
    project_id: &str,
    branch: &str,
) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    let rows = fetch_branch_atoms(client, project_id, branch)
        .await
        .unwrap_or_default();
    for atom in &rows {
        if let Some(aid) = str_field(atom, "atom_id") {
            ids.insert(aid);
        }
    }
    if (rows.len() == 100 || rows.len() >= RECENT_ATOMS_LIMIT)
        && let Ok(ndjson) = client
            .export_branch(Some(branch), &["ContextAtom"], None)
            .await
        && let Ok(parsed) = parse_ndjson_records(&ndjson)
    {
        for (aid, row) in parsed.atoms {
            let pid_ok = project_id.is_empty()
                || str_field(&row, "project_id").is_none_or(|p| p == project_id);
            if pid_ok {
                ids.insert(aid);
            }
        }
    }
    ids
}

/// Every atom of `project_id` on `branch`, or an error where the listing may
/// be partial: pruning acts only on a whole listing.
async fn collect_branch_atom_rows(
    client: &OmnigraphClient,
    project_id: &str,
    branch: &str,
) -> Result<Vec<Value>> {
    let mut rows = BTreeMap::new();
    let recent = fetch_branch_atoms(client, project_id, branch).await?;
    // `recent_atoms` stops at 100 on legacy clusters or `RECENT_ATOMS_LIMIT`; past that the export has them all.
    let capped = recent.len() == 100 || recent.len() >= RECENT_ATOMS_LIMIT;
    for mut row in recent {
        if row["project_id"].as_str().is_none() {
            // The query is scoped to the project.
            row["project_id"] = json!(project_id);
        }
        if let Some(atom_id) = str_field(&row, "atom_id") {
            rows.insert(atom_id, row);
        }
    }
    if capped {
        let ndjson = client
            .export_branch(Some(branch), &["ContextAtom"], None)
            .await?;
        for (atom_id, row) in parse_ndjson_records(&ndjson)?.atoms {
            if str_field(&row, "project_id").as_deref() == Some(project_id) {
                rows.insert(atom_id, row);
            }
        }
    }
    Ok(rows.into_values().collect())
}

/// Whether `row` is an atom the canonical sync wrote for `project_id` from
/// one of `layers`: its layer, its id prefix and its project all agree.
fn synced_atom_of(row: &Value, atom_id: &str, project_id: &str, layers: &[String]) -> bool {
    let layer = row["origin_layer"].as_str().unwrap_or("");
    let prefix = match layer {
        "project-brain" => "brain-",
        "memory-bank" => "mem-",
        _ => return false,
    };
    layers.iter().any(|held| held == layer)
        && atom_id.starts_with(prefix)
        && row["project_id"].as_str() == Some(project_id)
}

async fn find_atom_on_branch(
    client: &OmnigraphClient,
    project_id: &str,
    branch: &str,
    atom_id: &str,
) -> Option<Value> {
    let ids = BTreeSet::from([atom_id.to_string()]);
    atoms_on_branch(client, project_id, branch, &ids)
        .await
        .remove(atom_id)
}

/// The atoms of `ids` as `branch` holds them for `project_id`, by id: one
/// `recent_atoms` read, and one export of the branch's atoms only when an
/// id is missing from a listing that may be partial (100 rows, no project
/// to scope it, or a failed read). An id the branch does not hold is left
/// out.
async fn atoms_on_branch(
    client: &OmnigraphClient,
    project_id: &str,
    branch: &str,
    ids: &BTreeSet<String>,
) -> BTreeMap<String, Value> {
    let mut found = BTreeMap::new();
    if ids.is_empty() {
        return found;
    }
    let mut whole = false;
    if let Ok(rows) = fetch_branch_atoms(client, project_id, branch).await {
        whole = rows.len() != 100 && rows.len() < RECENT_ATOMS_LIMIT && !project_id.is_empty();
        for row in rows {
            if let Some(atom_id) = str_field(&row, "atom_id").filter(|id| ids.contains(id)) {
                found.entry(atom_id).or_insert(row);
            }
        }
    }
    if whole || found.len() == ids.len() {
        return found;
    }
    let Ok(ndjson) = client
        .export_branch(Some(branch), &["ContextAtom"], None)
        .await
    else {
        return found;
    };
    let Ok(parsed) = parse_ndjson_records(&ndjson) else {
        return found;
    };
    for atom_id in ids {
        if found.contains_key(atom_id) {
            continue;
        }
        if let Some(row) = parsed.atoms.get(atom_id)
            && (project_id.is_empty()
                || str_field(row, "project_id").is_none_or(|p| p == project_id))
        {
            found.insert(atom_id.clone(), normalize_atom_row(row));
        }
    }
    found
}

/// Omnigraph's merge conflicts, each Context Atom `divergent_update` with
/// the atom as the source and the target branch hold it (`source_value`,
/// `target_value`), which the merge preview's Keep source and Keep target
/// need. The atoms come from one read per branch whatever the number of
/// conflicts. A Keep writes the atom back under the caller's project, so
/// there are values only for an atom the branch holds for a named
/// `project_id`; every other conflict passes through as Omnigraph returned
/// it.
async fn conflicts_with_values(
    client: &OmnigraphClient,
    project_id: &str,
    source: &str,
    target: &str,
    conflicts: &[MergeConflict],
) -> Vec<Value> {
    let ids: BTreeSet<String> = if project_id.is_empty() {
        BTreeSet::new()
    } else {
        conflicts
            .iter()
            .filter_map(updated_atom_id)
            .map(str::to_string)
            .collect()
    };
    let (source_atoms, target_atoms) = tokio::join!(
        atoms_on_branch(client, project_id, source, &ids),
        atoms_on_branch(client, project_id, target, &ids),
    );
    conflicts
        .iter()
        .map(|conflict| {
            let mut out = json!(conflict);
            if let Some(atom_id) = updated_atom_id(conflict) {
                for (key, atoms) in [
                    ("source_value", &source_atoms),
                    ("target_value", &target_atoms),
                ] {
                    if let Some(atom) = atoms.get(atom_id) {
                        out[key] = whole_atom(atom, project_id);
                    }
                }
            }
            out
        })
        .collect()
}

/// `atom` with every column a branch may hold, as null where it holds none:
/// Omnigraph leaves null columns out of a row and `recent_atoms` returns no
/// project, and a Keep of the whole atom carries what the kept side holds.
fn whole_atom(atom: &Value, project_id: &str) -> Value {
    let mut whole = atom.clone();
    if let Some(columns) = whole.as_object_mut() {
        columns
            .entry("project_id")
            .or_insert_with(|| json!(project_id));
        for key in ["tags", "source_session_id", "session_id"] {
            columns.entry(key).or_insert(Value::Null);
        }
    }
    whole
}

/// Every `ContextAtom` property, as `GQ_UPSERT_ATOM` takes them.
const ATOM_COLUMNS: [&str; 20] = [
    "atom_id",
    "project_id",
    "kind",
    "title",
    "summary",
    "body",
    "tags",
    "source_branch",
    "source_session_id",
    "confidence",
    "pin_priority",
    "verified",
    "origin_layer",
    "authority",
    "privacy",
    "brain_record_id",
    "memory_id",
    "record_revision",
    "created_at",
    "updated_at",
];

/// The upsert parameters that write `atom` back as a branch stores it,
/// every property included and nothing restamped.
fn stored_atom_params(atom: &Value, project_id: &str) -> Value {
    let atom = whole_atom(atom, project_id);
    let columns = ATOM_COLUMNS
        .iter()
        .map(|key| {
            (
                key.to_string(),
                atom.get(*key).cloned().unwrap_or(Value::Null),
            )
        })
        .collect();
    Value::Object(columns)
}

/// The Context Atom a merge conflict names when both branches updated it.
fn updated_atom_id(conflict: &MergeConflict) -> Option<&str> {
    let both_updated = conflict.entity_kind == EntityKind::Node
        && conflict.type_name == "ContextAtom"
        && conflict.kind == MergeConflictKind::DivergentUpdate;
    clean_opt(conflict.entity_id.as_deref()).filter(|_| both_updated)
}

fn normalize_atom_row(row: &Value) -> Value {
    let mut obj = match row.as_object() {
        Some(m) => m.clone(),
        None => Map::new(),
    };
    copy_alias(&mut obj, "atom_id", "id");
    copy_alias(&mut obj, "source_branch", "branch");
    copy_alias(&mut obj, "source_session_id", "session_id");
    let prov = AtomProvenance::from_value(&Value::Object(obj.clone()));
    prov.apply_to_map(&mut obj);
    Value::Object(obj)
}

fn normalize_session_row(row: &Value) -> Value {
    let mut obj = match row.as_object() {
        Some(m) => m.clone(),
        None => Map::new(),
    };
    if let Some(id) = obj
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_string)
    {
        obj.entry("id").or_insert_with(|| Value::String(id));
    }
    Value::Object(obj)
}

fn normalize_file_row(row: &Value) -> Value {
    let mut obj = match row.as_object() {
        Some(m) => m.clone(),
        None => Map::new(),
    };
    if let Some(path) = obj.get("path").and_then(Value::as_str).map(str::to_string) {
        obj.entry("id").or_insert_with(|| Value::String(path));
    }
    Value::Object(obj)
}

fn normalize_symbol_row(row: &Value) -> Value {
    let mut obj = match row.as_object() {
        Some(m) => m.clone(),
        None => Map::new(),
    };
    if let Some(fqn) = obj.get("fqn").and_then(Value::as_str).map(str::to_string) {
        obj.entry("id").or_insert_with(|| Value::String(fqn));
    }
    Value::Object(obj)
}

fn normalize_turn_row(row: &Value) -> Value {
    let mut obj = match row.as_object() {
        Some(m) => m.clone(),
        None => Map::new(),
    };
    if let Some(snap_id) = obj
        .get("snapshot_id")
        .and_then(Value::as_str)
        .map(str::to_string)
    {
        obj.entry("id").or_insert_with(|| Value::String(snap_id));
    }
    Value::Object(obj)
}

fn change_to_atom_entry(ch: &super::client::EntityChange) -> Value {
    let props = ch
        .after
        .as_ref()
        .or(ch.before.as_ref())
        .map(|img| img.properties.clone())
        .unwrap_or_else(|| json!({}));
    let mut out = normalize_atom_row(&props);
    if out
        .get("atom_id")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        out["atom_id"] = json!(ch.id);
        out["id"] = json!(ch.id);
    }
    out = normalize_atom_row(&out);
    out["op"] = json!(ch.op.as_str());
    if let Some(before) = &ch.before {
        out["before"] = before.properties.clone();
    }
    if let Some(after) = &ch.after {
        out["after"] = after.properties.clone();
    }
    out
}

fn diff_edge_entry(ch: &super::client::EntityChange) -> Value {
    let img = match ch.op {
        ChangeOp::Delete => ch.before.as_ref().or(ch.after.as_ref()),
        _ => ch.after.as_ref().or(ch.before.as_ref()),
    };
    let endpoints = img.and_then(|i| i.endpoints.clone());
    let props = img.map(|i| i.properties.clone()).unwrap_or(Value::Null);
    let from = endpoints
        .as_ref()
        .map(|e| e.from.clone())
        .or_else(|| str_field(&props, "from"))
        .or_else(|| str_field(&props, "source_id"))
        .unwrap_or_default();
    let to = endpoints
        .as_ref()
        .map(|e| e.to.clone())
        .or_else(|| str_field(&props, "to"))
        .or_else(|| str_field(&props, "target_id"))
        .unwrap_or_default();
    let relation = diff_edge_relation(&ch.entity_type.name, &props);
    json!({
        "id": ch.id,
        "type": ch.entity_type.name,
        "op": ch.op.as_str(),
        "from": from,
        "to": to,
        "relation": relation,
        "endpoints": endpoints,
        "properties": props,
    })
}

fn diff_edge_relation(entity_type: &str, props: &Value) -> String {
    if entity_type == "RelatesTo" {
        str_field(props, "relation").unwrap_or_else(|| "relates_to".to_string())
    } else {
        pascal_to_snake(entity_type)
    }
}

pub(super) fn pascal_to_snake(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 4);
    for (idx, ch) in raw.chars().enumerate() {
        if ch.is_uppercase() {
            if idx > 0 {
                out.push('_');
            }
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn enrich_commit_change(ch: &super::client::EntityChange) -> Value {
    let props = ch
        .after
        .as_ref()
        .or(ch.before.as_ref())
        .map(|img| img.properties.clone())
        .unwrap_or(Value::Null);
    let title = props["title"]
        .as_str()
        .or_else(|| props["path"].as_str())
        .unwrap_or(&ch.id)
        .to_string();
    let summary = props["summary"]
        .as_str()
        .or_else(|| props["relation"].as_str())
        .unwrap_or("")
        .to_string();
    let mut obj = serde_json::to_value(ch)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    obj.insert("entity_kind".into(), json!(ch.kind));
    obj.insert("type_name".into(), json!(ch.entity_type.name));
    obj.insert("entity_id".into(), json!(ch.id));
    obj.insert("title".into(), json!(title));
    obj.insert("summary".into(), json!(summary));
    if ch.kind == EntityKind::Node {
        obj.insert("node_id".into(), json!(ch.id));
        if ch.entity_type.name == "ContextAtom" {
            obj.insert("atom_id".into(), json!(ch.id));
        }
    }
    Value::Object(obj)
}

pub(super) fn normalize_relation_name(raw: &str) -> Option<String> {
    let normalized = match raw.trim().to_ascii_lowercase().as_str() {
        "relates_to" | "related" => "supports".to_string(),
        other => other.to_string(),
    };
    VALID_RELATIONS
        .contains(&normalized.as_str())
        .then_some(normalized)
}

fn parse_relation_entry(rel: &Value) -> Option<(String, String, Option<String>, Option<f64>)> {
    if let Some(raw) = rel.as_str().map(str::trim).filter(|s| !s.is_empty()) {
        if let Some((prefix, rest)) = raw.split_once(':') {
            let target = rest.trim();
            if !target.is_empty()
                && let Some(relation) = normalize_relation_name(prefix)
            {
                return Some((target.to_string(), relation, None, None));
            }
        }
        return Some((raw.to_string(), "supports".to_string(), None, None));
    }
    let target_id = clean_opt(rel["target_id"].as_str().or_else(|| rel["id"].as_str()))?;
    let relation = match clean_opt(rel["relation"].as_str()) {
        Some(raw) => normalize_relation_name(raw)?,
        None => "supports".to_string(),
    };
    Some((
        target_id.to_string(),
        relation,
        clean_opt(rel["note"].as_str()).map(str::to_string),
        rel["confidence"].as_f64(),
    ))
}

fn atom_matches_query(atom: &Value, q_lower: &str) -> bool {
    for key in ["title", "summary", "body", "source_branch", "kind"] {
        if atom[key]
            .as_str()
            .is_some_and(|s| s.to_ascii_lowercase().contains(q_lower))
        {
            return true;
        }
    }
    for array_key in ["tags", "files", "symbols"] {
        if let Some(items) = atom[array_key].as_array() {
            for item in items.iter().filter_map(Value::as_str) {
                if item.to_ascii_lowercase().contains(q_lower) {
                    return true;
                }
            }
        }
    }
    false
}

fn score_atom_fallback(atom: &Value, q_lower: &str) -> f64 {
    let terms: Vec<&str> = q_lower.split_whitespace().collect();
    if terms.is_empty() {
        return 0.0;
    }
    let title = atom["title"].as_str().unwrap_or("").to_ascii_lowercase();
    let summary = atom["summary"].as_str().unwrap_or("").to_ascii_lowercase();
    let body = atom["body"].as_str().unwrap_or("").to_ascii_lowercase();
    let mut score = 0.0;
    for term in &terms {
        if title.contains(term) {
            score += 0.5;
        }
        if summary.contains(term) {
            score += 0.3;
        }
        if body.contains(term) {
            score += 0.2;
        }
    }
    score / (terms.len() as f64)
}

fn format_context_capsule(
    graph_id: &str,
    branch: &str,
    project_path: &Path,
    atoms: &[Value],
    contradictions: &[Value],
) -> String {
    if atoms.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "Omnigraph Context Capsule (graph `{graph_id}`, branch `{branch}`, project `{}` — discovery aid only; subordinate to canonical code/config/tests, `project-brain/`, and `memory-bank/`):\n",
        project_path.display()
    );
    for atom in atoms {
        out.push_str(&format_atom_line_with_authority(atom));
        if let Some(atom_id) = str_field(atom, "atom_id")
            && let Some(caution) = contradiction_caution(&atom_id, contradictions)
        {
            out.push_str(&caution);
        }
    }
    out
}

/// One capsule line with the atom's authority tag: a verified Memory Bank or
/// Project Brain atom names its source, and anything else is marked as an
/// unverified reference to check against code and the Brain.
pub(crate) fn format_atom_line_with_authority(atom: &Value) -> String {
    {
        let kind = atom["kind"].as_str().unwrap_or("insight");
        let title = atom["title"].as_str().unwrap_or("Untitled");
        let summary = atom["summary"].as_str().unwrap_or("");
        let src = atom["source_branch"].as_str().unwrap_or("main");
        let verified = atom["verified"].as_bool().unwrap_or(false);
        let origin = clean_opt(atom["origin_layer"].as_str()).unwrap_or("");
        let brain_id = clean_opt(atom["brain_record_id"].as_str());
        let mem_id = clean_opt(atom["memory_id"].as_str());
        let is_imported = origin == "imported" || src.starts_with("imported/");

        let tag = if !is_imported && verified && origin == "memory-bank" {
            match mem_id {
                Some(mid) => format!("[memory-bank {kind} · {mid}]"),
                None => format!("[memory-bank {kind}]"),
            }
        } else if !is_imported && verified && origin == "project-brain" {
            match brain_id {
                Some(bid) => {
                    let short: String = bid.chars().take(8).collect();
                    format!("[project-brain {kind} · {short}]")
                }
                None => format!("[project-brain {kind}]"),
            }
        } else if !is_imported
            && verified
            && src == CANONICAL_KNOWLEDGE_BRANCH
            && (mem_id.is_some() || brain_id.is_some())
        {
            if let Some(mid) = mem_id {
                format!("[memory-bank {kind} · {mid}]")
            } else {
                let short: String = brain_id.unwrap_or("").chars().take(8).collect();
                format!("[project-brain {kind} · {short}]")
            }
        } else {
            format!("[unverified reference — verify against code & Brain] [{kind}]")
        };

        if summary.trim().is_empty() {
            format!("- {tag} {title} (from `{src}`)\n")
        } else {
            format!("- {tag} {title} (from `{src}`): {summary}\n")
        }
    }
}

#[cfg(test)]
pub(super) fn format_context_capsule_for_test(
    graph_id: &str,
    branch: &str,
    project_path: &Path,
    atoms: &[Value],
) -> String {
    format_context_capsule(graph_id, branch, project_path, atoms, &[])
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn clean_opt(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|s| !s.is_empty())
}

fn parse_limit(v: &Value) -> Option<usize> {
    v.as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .or_else(|| v.as_str().and_then(|s| s.trim().parse::<usize>().ok()))
}

fn now_utc_iso_ms() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn fallback_commit_digest(c: &CommitOutput) -> CommitDigest {
    let short: String = c.graph_commit_id.chars().take(12).collect();
    CommitDigest {
        added_count: 0,
        updated_count: 0,
        deleted_count: 0,
        atom_delta: 0,
        summary: format!("Commit {short}"),
    }
}

fn build_commit_digest(c: &CommitOutput, changes: &[super::client::EntityChange]) -> CommitDigest {
    let mut added_count = 0usize;
    let mut updated_count = 0usize;
    let mut deleted_count = 0usize;
    let mut node_changes = 0usize;
    let mut edge_changes = 0usize;
    let mut atom_delta = 0isize;
    let mut atom_titles = Vec::new();
    let mut session_label: Option<String> = None;
    let mut snapshot_summary: Option<String> = None;

    for ch in changes {
        match ch.op {
            ChangeOp::Insert => added_count += 1,
            ChangeOp::Update => updated_count += 1,
            ChangeOp::Delete => deleted_count += 1,
        }
        match ch.kind {
            EntityKind::Node => node_changes += 1,
            EntityKind::Edge => edge_changes += 1,
        }
        let props = ch
            .after
            .as_ref()
            .or(ch.before.as_ref())
            .map(|img| &img.properties);
        if ch.kind == EntityKind::Node && ch.entity_type.name == "ContextAtom" {
            match ch.op {
                ChangeOp::Insert => atom_delta += 1,
                ChangeOp::Delete => atom_delta -= 1,
                ChangeOp::Update => {}
            }
            if let Some(title) = props
                .and_then(|p| p["title"].as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                atom_titles.push(title.to_string());
            }
        } else if ch.kind == EntityKind::Node
            && ch.entity_type.name == "Session"
            && session_label.is_none()
        {
            session_label = props
                .and_then(|p| p["title"].as_str().or_else(|| p["branch"].as_str()))
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| Some(ch.id.clone()));
        } else if ch.kind == EntityKind::Node
            && ch.entity_type.name == "TurnSnapshot"
            && snapshot_summary.is_none()
        {
            snapshot_summary = props
                .and_then(|p| p["summary"].as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
        }
    }

    let summary = if c.merged_parent_commit_id.is_some() {
        let branch = c.graph_branch.as_deref().unwrap_or("main");
        if changes.is_empty() {
            format!("Merged into {branch}")
        } else {
            format!("Merged into {branch} (+{node_changes} nodes · +{edge_changes} edges)")
        }
    } else if let Some(first_title) = atom_titles.first() {
        let clipped: String = if first_title.chars().count() > 56 {
            let prefix: String = first_title.chars().take(55).collect();
            format!("{prefix}…")
        } else {
            first_title.clone()
        };
        let extra_atoms = atom_titles.len().saturating_sub(1);
        match (extra_atoms, edge_changes) {
            (0, 0) => format!("Published: {clipped}"),
            (0, e) => format!(
                "Published: {clipped} (+{e} edge{})",
                if e == 1 { "" } else { "s" }
            ),
            (a, 0) => format!("Published: {clipped} (+{a} more)"),
            (a, e) => format!("Published: {clipped} (+{a} atoms, +{e} edges)"),
        }
    } else if let Some(label) = session_label {
        let clipped: String = if label.chars().count() > 52 {
            let prefix: String = label.chars().take(51).collect();
            format!("{prefix}…")
        } else {
            label
        };
        if edge_changes > 0 {
            format!("Session: {clipped} (+{edge_changes} edges)")
        } else {
            format!("Session: {clipped}")
        }
    } else if snapshot_summary.is_some() {
        if edge_changes > 0 {
            format!("Turn snapshot (+{edge_changes} edges)")
        } else {
            "Turn snapshot".to_string()
        }
    } else if changes.is_empty() {
        "Empty commit".to_string()
    } else {
        format!("+{node_changes} nodes · +{edge_changes} edges")
    };

    CommitDigest {
        added_count,
        updated_count,
        deleted_count,
        atom_delta,
        summary,
    }
}

fn parse_opt_usize(v: &Value, label: &str) -> Result<Option<usize>> {
    match v {
        Value::Null => Ok(None),
        Value::Number(n) => n
            .as_u64()
            .and_then(|val| usize::try_from(val).ok())
            .map(Some)
            .ok_or_else(|| HarnessError::invalid(format!("Invalid `{label}` parameter."))),
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                trimmed
                    .parse::<usize>()
                    .map(Some)
                    .map_err(|_| HarnessError::invalid(format!("Invalid `{label}` parameter.")))
            }
        }
        _ => Err(HarnessError::invalid(format!(
            "Invalid `{label}` parameter."
        ))),
    }
}

fn parse_graph_cursor(cursor: Option<&str>) -> Result<Option<usize>> {
    let Some(raw) = clean_opt(cursor) else {
        return Ok(None);
    };
    let numeric = raw.strip_prefix("offset:").unwrap_or(raw).trim();
    numeric
        .parse::<usize>()
        .map(Some)
        .map_err(|_| HarnessError::invalid("Invalid `cursor` parameter."))
}

async fn run_recent_atoms_query(
    client: &OmnigraphClient,
    project_id: &str,
    branch: Option<String>,
    snapshot: Option<String>,
) -> Result<super::client::QueryResponse> {
    let params = json!({ "project_id": project_id });
    match client
        .query(&QueryRequest {
            query: schema::QUERY_RECENT_ATOMS.into(),
            name: Some("recent_atoms".into()),
            params: Some(params.clone()),
            branch: branch.clone(),
            snapshot: snapshot.clone(),
            settings: None,
        })
        .await
    {
        Ok(resp) => Ok(resp),
        Err(err) if matches!(err.status, 401 | 403) => client
            .stored_query(
                "recent_atoms",
                &StoredQueryRequest {
                    params: Some(params),
                    branch,
                    snapshot,
                    expect_mutation: Some(false),
                },
            )
            .await
            .map_err(Into::into),
        Err(err) => Err(err.into()),
    }
}

async fn fetch_commits_with_digests(
    client: &OmnigraphClient,
    cache: &Arc<StdMutex<BTreeMap<String, CommitDigest>>>,
    active_branch: &str,
) -> (super::client::CommitListResponse, Vec<CommitDigest>) {
    let commits_resp = client
        .list_commits(&CommitListQuery {
            branch: Some(active_branch.to_string()),
            limit: Some(50),
            page_token: None,
        })
        .await
        .unwrap_or_default();
    let digests = summarize_commits_concurrent(client, cache, &commits_resp.commits).await;
    (commits_resp, digests)
}

async fn summarize_commits_concurrent(
    client: &OmnigraphClient,
    cache: &Arc<StdMutex<BTreeMap<String, CommitDigest>>>,
    commits: &[CommitOutput],
) -> Vec<CommitDigest> {
    let mut digests: Vec<Option<CommitDigest>> = vec![None; commits.len()];
    let mut to_fetch = Vec::new();
    for (idx, c) in commits.iter().enumerate() {
        if idx >= COMMIT_DIGEST_PREFETCH_COUNT {
            digests[idx] = Some(fallback_commit_digest(c));
            continue;
        }
        if let Ok(guard) = cache.lock()
            && let Some(hit) = guard.get(&c.graph_commit_id)
        {
            digests[idx] = Some(hit.clone());
            continue;
        }
        to_fetch.push((idx, c.clone()));
    }

    if !to_fetch.is_empty() {
        let sem = Arc::new(tokio::sync::Semaphore::new(COMMIT_DIGEST_CONCURRENCY));
        let mut set = tokio::task::JoinSet::new();
        for (idx, c) in to_fetch {
            let client = client.clone();
            let cache = Arc::clone(cache);
            let permit_sem = Arc::clone(&sem);
            set.spawn(async move {
                let _permit = permit_sem.acquire_owned().await.ok();
                let digest = summarize_commit_cached(&client, &cache, &c).await;
                (idx, digest)
            });
        }
        while let Some(res) = set.join_next().await {
            if let Ok((idx, digest)) = res {
                digests[idx] = Some(digest);
            }
        }
    }

    commits
        .iter()
        .zip(digests)
        .map(|(c, d)| d.unwrap_or_else(|| fallback_commit_digest(c)))
        .collect()
}

fn build_branches_json(branch_list: &super::client::BranchListResponse) -> Vec<Value> {
    let main_head = branch_list
        .details
        .as_ref()
        .and_then(|details| details.iter().find(|d| d.name == "main"))
        .map(|d| d.head.graph_commit_id.clone());

    if let Some(details) = &branch_list.details {
        details
            .iter()
            .map(|d| {
                let unmerged = d.name != "main"
                    && main_head
                        .as_deref()
                        .is_some_and(|mh| mh != d.head.graph_commit_id);
                json!({
                    "name": d.name,
                    "head_commit": d.head.graph_commit_id,
                    "head_commit_id": d.head.graph_commit_id,
                    "from": d.from,
                    "from_commit_id": d.from_commit_id,
                    "created_at": d.created_at,
                    "updated_at": d.updated_at,
                    "is_main": d.name == "main",
                    "unmerged": unmerged,
                })
            })
            .collect()
    } else {
        branch_list
            .branches
            .iter()
            .map(|name| {
                json!({
                    "name": name,
                    "is_main": name == "main",
                    "unmerged": name != "main",
                })
            })
            .collect()
    }
}

fn commit_to_graph_json(c: &CommitOutput, active_branch: &str, digest: &CommitDigest) -> Value {
    json!({
        "graph_commit_id": c.graph_commit_id,
        "commit_id": c.graph_commit_id,
        "branch": c.graph_branch.as_deref().unwrap_or(active_branch),
        "graph_branch": c.graph_branch,
        "parent_commit_id": c.parent_commit_id,
        "merged_parent_commit_id": c.merged_parent_commit_id,
        "actor_id": c.actor_id,
        "authored_at": c.created_at,
        "created_at": format_omnigraph_ts(c.created_at)
            .unwrap_or_else(|| c.created_at.to_string()),
        "published": c.published,
        "added_count": digest.added_count,
        "updated_count": digest.updated_count,
        "deleted_count": digest.deleted_count,
        "atom_delta": digest.atom_delta,
        "summary": digest.summary,
    })
}

async fn expand_capped_graph_atoms(
    client: &OmnigraphClient,
    project_id: &str,
    active_branch: &str,
    snapshot_commit: Option<&str>,
    initial_rows: Vec<Value>,
) -> (Vec<Value>, bool) {
    if snapshot_commit.is_some() {
        return (initial_rows, true);
    }
    let Ok(ndjson) = client
        .export_branch(Some(active_branch), &["ContextAtom"], None)
        .await
    else {
        return (initial_rows, true);
    };
    let Ok(parsed) = parse_ndjson_records(&ndjson) else {
        return (initial_rows, true);
    };
    let mut by_id: BTreeMap<String, Value> = BTreeMap::new();
    for row in initial_rows {
        if let Some(aid) = str_field(&row, "atom_id").or_else(|| str_field(&row, "id")) {
            by_id.insert(aid, row);
        }
    }
    for (aid, row) in parsed.atoms {
        let proj_ok =
            project_id.is_empty() || str_field(&row, "project_id").is_none_or(|p| p == project_id);
        if proj_ok {
            by_id.entry(aid).or_insert(row);
        }
    }
    let mut combined: Vec<Value> = by_id.into_values().collect();
    sort_atoms_recent_order(&mut combined);
    (combined, false)
}

fn sort_atoms_recent_order(atoms: &mut [Value]) {
    atoms.sort_by(|a, b| {
        let pa = a["pin_priority"].as_i64().unwrap_or(0);
        let pb = b["pin_priority"].as_i64().unwrap_or(0);
        pb.cmp(&pa)
            .then_with(|| {
                atom_timestamp_sort_key(&b["updated_at"])
                    .cmp(&atom_timestamp_sort_key(&a["updated_at"]))
            })
            .then_with(|| {
                let ia = str_field(a, "atom_id")
                    .or_else(|| str_field(a, "id"))
                    .unwrap_or_default();
                let ib = str_field(b, "atom_id")
                    .or_else(|| str_field(b, "id"))
                    .unwrap_or_default();
                ia.cmp(&ib)
            })
    });
}

fn atom_timestamp_sort_key(v: &Value) -> (i64, String) {
    if let Some(n) = v.as_i64() {
        let ms = if n >= 1_000_000_000_000_000 {
            n / 1_000
        } else if n >= 1_000_000_000_000 {
            n
        } else {
            n.saturating_mul(1_000)
        };
        return (ms, String::new());
    }
    let Some(s) = v.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
        return (0, String::new());
    };
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return (dt.timestamp_millis(), s.to_string());
    }
    if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
        return (ndt.and_utc().timestamp_millis(), s.to_string());
    }
    (0, s.to_string())
}

fn symbol_owner_and_label(fqn: &str, file_path: &str, name: &str) -> (Option<String>, String) {
    let scoped = if !file_path.is_empty() {
        fqn.strip_prefix(&format!("{file_path}::"))
    } else {
        None
    };
    if let Some(rest) = scoped {
        let owner = (!name.is_empty())
            .then(|| rest.strip_suffix(&format!("::{name}")))
            .flatten()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        return (owner, rest.to_string());
    }
    let label = if !name.is_empty() {
        name.to_string()
    } else {
        fqn.to_string()
    };
    (None, label)
}

fn symbol_matches_query(symbol: &Value, q_lower: &str) -> bool {
    for key in ["fqn", "name", "label", "owner", "file_path", "kind"] {
        if symbol[key]
            .as_str()
            .is_some_and(|s| s.to_ascii_lowercase().contains(q_lower))
        {
            return true;
        }
    }
    false
}

fn group_edge_targets_by_source(
    edges: &[Value],
    kept_sources: Option<&BTreeSet<String>>,
) -> BTreeMap<String, Vec<String>> {
    let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for edge in edges {
        if let (Some(src), Some(dst)) = (str_field(edge, "source_id"), str_field(edge, "target_id"))
            && kept_sources.is_none_or(|kept| kept.contains(&src))
        {
            let entry = grouped.entry(src).or_default();
            if !entry.contains(&dst) {
                entry.push(dst);
            }
        }
    }
    grouped
}

fn load_session_omnigraph_metadata_batch(
    store: &SessionStore,
    session_ids: &BTreeSet<String>,
) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    if session_ids.is_empty() {
        return out;
    }
    let Ok(db) = store.connection() else {
        return out;
    };
    let ids: Vec<&str> = session_ids.iter().map(String::as_str).collect();
    for chunk in ids.chunks(200) {
        let placeholders = std::iter::repeat_n("?", chunk.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT id, omnigraph FROM sessions WHERE id IN ({placeholders}) AND omnigraph IS NOT NULL"
        );
        let Ok(mut stmt) = db.prepare(&sql) else {
            continue;
        };
        let Ok(rows) = stmt.query_map(rusqlite::params_from_iter(chunk.iter().copied()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }) else {
            continue;
        };
        for row in rows.flatten() {
            if let Ok(parsed) = crate::strict_json::parse(row.1.as_bytes(), 16 * 1024 * 1024) {
                out.insert(row.0, parsed);
            }
        }
    }
    out
}

fn build_symbol_graph_entries(
    project_id: &str,
    symbol_rows: &[Value],
    symbol_edge_rows: &[Value],
    kept_atom_ids: &BTreeSet<String>,
    q_filter: Option<&str>,
    include_unlinked_symbols: bool,
    allow_symbol_q_match: bool,
) -> (Vec<Value>, Vec<Value>) {
    let mut symbol_map: BTreeMap<String, Value> = BTreeMap::new();
    let upsert_symbol_entry = |map: &mut BTreeMap<String, Value>,
                               fqn: &str,
                               file_path: String,
                               kind: String,
                               name: String,
                               proj: String| {
        let (owner, label) = symbol_owner_and_label(fqn, &file_path, &name);
        map.entry(fqn.to_string())
            .and_modify(|existing| {
                if str_field(existing, "file_path").is_none() && !file_path.is_empty() {
                    existing["file_path"] = json!(file_path);
                    existing["path"] = json!(file_path);
                }
                if str_field(existing, "name").is_none() && !name.is_empty() {
                    existing["name"] = json!(name);
                }
                if str_field(existing, "kind").is_none_or(|k| k == "symbol") && !kind.is_empty() {
                    existing["kind"] = json!(kind);
                }
            })
            .or_insert_with(|| {
                json!({
                    "id": fqn,
                    "fqn": fqn,
                    "project_id": proj,
                    "file_path": file_path,
                    "path": file_path,
                    "kind": if kind.is_empty() { "symbol".to_string() } else { kind },
                    "name": name,
                    "owner": owner,
                    "label": label,
                })
            });
    };

    for row in symbol_rows {
        let Some(fqn) = str_field(row, "fqn").or_else(|| str_field(row, "id")) else {
            continue;
        };
        let file_path = str_field(row, "file_path")
            .or_else(|| str_field(row, "path"))
            .unwrap_or_default();
        let kind = str_field(row, "kind").unwrap_or_else(|| "symbol".to_string());
        let name = str_field(row, "name").unwrap_or_default();
        let proj = str_field(row, "project_id").unwrap_or_else(|| project_id.to_string());
        upsert_symbol_entry(&mut symbol_map, &fqn, file_path, kind, name, proj);
    }

    let mut symbol_edges = Vec::new();
    let mut seen_edges: BTreeSet<(String, String)> = BTreeSet::new();
    let mut mentions_by_symbol: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e in symbol_edge_rows {
        let (Some(src), Some(fqn)) = (str_field(e, "source_id"), str_field(e, "target_id")) else {
            continue;
        };
        let keep_src = kept_atom_ids.contains(&src);
        if !keep_src && !(include_unlinked_symbols || (allow_symbol_q_match && q_filter.is_some()))
        {
            continue;
        }
        let file_path = str_field(e, "file_path")
            .or_else(|| str_field(e, "path"))
            .unwrap_or_default();
        let kind = str_field(e, "kind").unwrap_or_else(|| "symbol".to_string());
        let name = str_field(e, "name").unwrap_or_default();
        upsert_symbol_entry(
            &mut symbol_map,
            &fqn,
            file_path,
            kind,
            name,
            project_id.to_string(),
        );

        if !keep_src || !seen_edges.insert((src.clone(), fqn.clone())) {
            continue;
        }
        mentions_by_symbol
            .entry(fqn.clone())
            .or_default()
            .push(src.clone());

        let sym = symbol_map.get(&fqn);
        let resolved_file = sym
            .and_then(|s| str_field(s, "file_path"))
            .unwrap_or_default();
        let resolved_label = sym
            .and_then(|s| str_field(s, "label"))
            .unwrap_or_else(|| fqn.clone());
        let resolved_kind = sym
            .and_then(|s| str_field(s, "kind"))
            .unwrap_or_else(|| "symbol".to_string());
        let role = str_field(e, "role").unwrap_or_else(|| "named".to_string());
        symbol_edges.push(json!({
            "id": format!("sym:{src}->{fqn}"),
            "kind": "mentions_symbol",
            "type": "MentionsSymbol",
            "relation": "mentions_symbol",
            "source": src,
            "target": fqn,
            "from": src,
            "to": fqn,
            "atom_id": src,
            "fqn": fqn,
            "role": role,
            "label": role,
            "file_path": resolved_file,
            "symbol_name": resolved_label,
            "symbol_kind": resolved_kind,
        }));
    }

    let mut symbols = Vec::new();
    for (fqn, mut sym) in symbol_map {
        let atom_ids = mentions_by_symbol.remove(&fqn).unwrap_or_default();
        let mention_count = atom_ids.len();
        let q_match =
            allow_symbol_q_match && q_filter.is_some_and(|qf| symbol_matches_query(&sym, qf));
        if include_unlinked_symbols || mention_count > 0 || q_match {
            sym["mention_count"] = json!(mention_count);
            sym["atom_ids"] = json!(atom_ids);
            symbols.push(sym);
        }
    }

    (symbols, symbol_edges)
}

fn build_graph_edges(
    pub_edges: &[Value],
    con_edges: &[Value],
    file_edges: &[Value],
    rel_edges: &[Value],
    follow_edges: &[Value],
    kept_atom_ids: &BTreeSet<String>,
    sid_filter: Option<&str>,
) -> Vec<Value> {
    let mut edges = Vec::new();
    for e in pub_edges {
        if let (Some(src), Some(dst)) = (str_field(e, "source_id"), str_field(e, "target_id"))
            && kept_atom_ids.contains(&src)
        {
            edges.push(json!({
                "id": format!("pub:{src}->{dst}"),
                "kind": "published_by",
                "relation": "published_by",
                "source": src,
                "target": dst,
                "from": src,
                "to": dst,
                "label": e["role"].as_str().unwrap_or("published_by"),
                "published_at": e.get("published_at").cloned().unwrap_or(Value::Null),
            }));
        }
    }
    for e in con_edges {
        if let (Some(src), Some(dst)) = (str_field(e, "source_id"), str_field(e, "target_id"))
            && kept_atom_ids.contains(&src)
        {
            edges.push(json!({
                "id": format!("con:{src}->{dst}"),
                "kind": "consumed_by",
                "relation": "consumed_by",
                "source": src,
                "target": dst,
                "from": src,
                "to": dst,
                "label": e["injection_mode"].as_str().unwrap_or("consumed_by"),
                "relevance": e.get("relevance").cloned().unwrap_or(Value::Null),
            }));
        }
    }
    for e in file_edges {
        if let (Some(src), Some(dst)) = (str_field(e, "source_id"), str_field(e, "target_id"))
            && kept_atom_ids.contains(&src)
        {
            edges.push(json!({
                "id": format!("file:{src}->{dst}"),
                "kind": "touches_file",
                "relation": "touches_file",
                "source": src,
                "target": dst,
                "from": src,
                "to": dst,
                "label": e["access"].as_str().unwrap_or("touches_file"),
            }));
        }
    }
    for e in rel_edges {
        if let (Some(src), Some(dst)) = (str_field(e, "source_id"), str_field(e, "target_id"))
            && kept_atom_ids.contains(&src)
            && kept_atom_ids.contains(&dst)
        {
            let rel = e["relation"].as_str().unwrap_or("relates_to");
            edges.push(json!({
                "id": format!("rel:{src}:{rel}:{dst}"),
                "kind": "relates_to",
                "relation": rel,
                "source": src,
                "target": dst,
                "from": src,
                "to": dst,
                "label": rel,
                "note": e.get("note").cloned().unwrap_or(Value::Null),
                "confidence": e.get("confidence").cloned().unwrap_or(Value::Null),
            }));
        }
    }
    for e in follow_edges {
        if let (Some(src), Some(dst)) = (str_field(e, "source_id"), str_field(e, "target_id"))
            && sid_filter.is_none_or(|sid| src == sid || dst == sid)
        {
            // Edges written before the relation was normalized carry a
            // shared space's name.
            let rel = e["relation"]
                .as_str()
                .filter(|rel| matches!(*rel, "broadcast" | "shared_branch" | "linked"))
                .unwrap_or("shared_branch");
            edges.push(json!({
                "id": format!("follow:{src}->{dst}"),
                "kind": "session_follows",
                "relation": rel,
                "source": src,
                "target": dst,
                "from": src,
                "to": dst,
                "label": rel,
                "created_at": e.get("created_at").cloned().unwrap_or(Value::Null),
            }));
        }
    }
    edges
}

async fn summarize_commit_cached(
    client: &OmnigraphClient,
    cache: &StdMutex<BTreeMap<String, CommitDigest>>,
    c: &CommitOutput,
) -> CommitDigest {
    if let Ok(guard) = cache.lock()
        && let Some(hit) = guard.get(&c.graph_commit_id)
    {
        return hit.clone();
    }
    let digest = match client
        .commit_changes(&c.graph_commit_id, Some(200), None)
        .await
    {
        Ok(detail) => build_commit_digest(c, &detail.changes),
        Err(err) if err.status == 409 => CommitDigest {
            added_count: 0,
            updated_count: 0,
            deleted_count: 0,
            atom_delta: 0,
            summary: "Initial root commit".to_string(),
        },
        Err(_) => return fallback_commit_digest(c),
    };
    if let Ok(mut guard) = cache.lock() {
        if guard.len() >= 512 {
            guard.clear();
        }
        guard.insert(c.graph_commit_id.clone(), digest.clone());
    }
    digest
}

fn format_omnigraph_ts(ts: i64) -> Option<String> {
    let dt = if ts >= 1_000_000_000_000_000 {
        chrono::DateTime::from_timestamp_micros(ts)
    } else if ts >= 1_000_000_000_000 {
        chrono::DateTime::from_timestamp_millis(ts)
    } else if ts > 0 {
        chrono::DateTime::from_timestamp(ts, 0)
    } else {
        None
    }?;
    Some(dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

pub(super) fn detect_language(path: &str) -> Option<&'static str> {
    let ext = Path::new(path).extension().and_then(|e| e.to_str())?;
    match ext.to_ascii_lowercase().as_str() {
        "rs" => Some("rust"),
        "php" | "phtml" | "inc" => Some("php"),
        "ts" | "tsx" | "mts" | "cts" => Some("typescript"),
        "js" | "jsx" | "mjs" | "cjs" => Some("javascript"),
        "py" | "pyi" => Some("python"),
        "go" => Some("go"),
        "java" => Some("java"),
        "kt" | "kts" => Some("kotlin"),
        "cs" => Some("csharp"),
        "rb" => Some("ruby"),
        "swift" => Some("swift"),
        "scala" | "sc" => Some("scala"),
        "sql" => Some("sql"),
        "md" => Some("markdown"),
        "css" => Some("css"),
        "html" => Some("html"),
        "json" => Some("json"),
        "yaml" | "yml" => Some("yaml"),
        "toml" => Some("toml"),
        _ => None,
    }
}

fn build_tokens_json(actor_id: &str, primary_token: &str) -> String {
    let mut map = BTreeMap::new();
    map.insert(actor_id.to_string(), primary_token.to_string());
    if actor_id != "harness-ui" {
        map.insert(
            "harness-ui".to_string(),
            format!("ui-{}", &store::hexid()[..16]),
        );
    }
    if actor_id != "harness-session" {
        map.insert(
            "harness-session".to_string(),
            format!("sess-{}", &store::hexid()[..16]),
        );
    }
    serde_json::to_string(&map).unwrap_or_else(|_| {
        format!(
            "{{\"{}\":\"{}\"}}",
            actor_id.replace('"', ""),
            primary_token.replace('"', "")
        )
    })
}

fn pick_free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

fn load_config_file(path: &Path) -> Result<OmnigraphConfig> {
    if !path.is_file() {
        return Ok(OmnigraphConfig::default());
    }
    let bytes = std::fs::read(path)?;
    if bytes.is_empty() {
        return Ok(OmnigraphConfig::default());
    }
    serde_json::from_slice(&bytes)
        .map_err(|err| HarnessError::invalid(format!("Invalid omnigraph.json: {err}")))
}

fn save_config_file(path: &Path, config: &OmnigraphConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(config)?;
    let tmp = path.with_extension("json.tmp");
    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.create(true).write(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn find_binary(tools_dir: &Path, name: &str) -> Option<PathBuf> {
    let bin_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let local = tools_dir.join(&bin_name);
    if local.is_file() {
        return Some(local);
    }
    infra_host::provider::find_executable(name)
}

fn probe_omnigraph_version(cli_path: &Path) -> Option<String> {
    let out = Command::new(cli_path)
        .args(["version", "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if out.status.success()
        && let Ok(v) = serde_json::from_slice::<Value>(&out.stdout)
        && let Some(ver) = v["version"].as_str().or_else(|| v["omnigraph"].as_str())
    {
        return Some(ver.to_string());
    }
    let fallback = Command::new(cli_path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if fallback.status.success() {
        let text = String::from_utf8_lossy(&fallback.stdout).trim().to_string();
        if !text.is_empty() {
            return Some(text);
        }
    }
    None
}

fn apply_and_unlock_cluster(
    cli_path: &Path,
    config_dir: &Path,
    data_dir: &Path,
    actor: &str,
) -> Result<()> {
    run_checked(
        Command::new(cli_path).args([
            "cluster",
            "apply",
            "--target",
            &data_dir.to_string_lossy(),
            "--config",
            &config_dir.to_string_lossy(),
            "--as",
            actor,
            "--json",
        ]),
        "omnigraph cluster apply",
    )?;
    let status_out = run_checked_output(
        Command::new(cli_path).args([
            "cluster",
            "status",
            "--target",
            &data_dir.to_string_lossy(),
            "--json",
        ]),
        "omnigraph cluster status",
    )?;
    if let Some(lock_id) = parse_lock_id(&status_out) {
        run_checked(
            Command::new(cli_path).args([
                "cluster",
                "force-unlock",
                &lock_id,
                "--target",
                &data_dir.to_string_lossy(),
                "--json",
            ]),
            "omnigraph cluster force-unlock",
        )?;
    }
    Ok(())
}

fn parse_lock_id(raw: &[u8]) -> Option<String> {
    let val: Value = serde_json::from_slice(raw).ok()?;
    val["lock_id"]
        .as_str()
        .or_else(|| val["state"]["lock_id"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn run_checked(cmd: &mut Command, label: &str) -> Result<()> {
    let _ = run_checked_output(cmd, label)?;
    Ok(())
}

fn run_checked_output(cmd: &mut Command, label: &str) -> Result<Vec<u8>> {
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .map_err(|err| HarnessError::unavailable(format!("{label} failed to spawn: {err}")))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let detail = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        return Err(HarnessError::unavailable(format!(
            "{label} exited with {}: {detail}",
            out.status
        )));
    }
    Ok(out.stdout)
}

fn extract_omnigraph_tarball(compressed: &[u8], dest_dir: &Path) -> Result<Vec<String>> {
    let parent = dest_dir
        .parent()
        .ok_or_else(|| HarnessError::invalid("Invalid Omnigraph tools directory."))?;
    std::fs::create_dir_all(parent)?;
    let stage = parent.join(format!(".stage-{}", &store::hexid()[..12]));
    std::fs::create_dir_all(&stage)?;

    let result = (|| -> Result<Vec<String>> {
        let gz = flate2::read::GzDecoder::new(Cursor::new(compressed));
        let mut archive = tar::Archive::new(gz.take(MAX_EXPANDED_BYTES as u64 + 1));
        let mut extracted = Vec::new();
        let mut file_count = 0usize;
        let mut total_bytes = 0usize;

        for item in archive
            .entries()
            .map_err(|err| HarnessError::invalid(format!("Invalid Omnigraph archive: {err}")))?
        {
            file_count += 1;
            if file_count > MAX_ARCHIVE_FILES {
                return Err(HarnessError::invalid(
                    "Omnigraph archive contains too many entries.",
                ));
            }
            let mut entry = item.map_err(|err| {
                HarnessError::invalid(format!("Invalid Omnigraph archive entry: {err}"))
            })?;
            let path = entry
                .path()
                .map_err(|_| HarnessError::invalid("Invalid archive entry path."))?;
            for comp in path.components() {
                if matches!(
                    comp,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                ) {
                    return Err(HarnessError::invalid(
                        "Unsafe path traversal in Omnigraph archive.",
                    ));
                }
            }
            let entry_type = entry.header().entry_type();
            if entry_type.is_dir() {
                continue;
            }
            if !entry_type.is_file() {
                return Err(HarnessError::invalid(
                    "Omnigraph archive contains unsupported non-regular file entries.",
                ));
            }
            let size = usize::try_from(entry.size())
                .map_err(|_| HarnessError::invalid("Omnigraph entry size overflow."))?;
            total_bytes = total_bytes
                .checked_add(size)
                .ok_or_else(|| HarnessError::invalid("Omnigraph archive size overflow."))?;
            if total_bytes > MAX_EXPANDED_BYTES {
                return Err(HarnessError::invalid(
                    "Expanded Omnigraph archive exceeds size limit.",
                ));
            }
            let Some(file_name) = path
                .file_name()
                .and_then(|s| s.to_str())
                .map(str::to_string)
            else {
                continue;
            };
            if !matches!(
                file_name.as_str(),
                "omnigraph"
                    | "omnigraph-server"
                    | "omnigraph-azure-admission"
                    | "omnigraph.exe"
                    | "omnigraph-server.exe"
            ) {
                continue;
            }
            let target_file = stage.join(&file_name);
            let mut out_file = {
                let mut opts = std::fs::OpenOptions::new();
                opts.create(true).write(true).truncate(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    opts.mode(0o755);
                }
                opts.open(&target_file)?
            };
            std::io::copy(&mut entry, &mut out_file)?;
            out_file.sync_all()?;
            extracted.push(file_name);
        }

        if !extracted
            .iter()
            .any(|name| name == "omnigraph" || name == "omnigraph.exe")
            || !extracted
                .iter()
                .any(|name| name == "omnigraph-server" || name == "omnigraph-server.exe")
        {
            return Err(HarnessError::invalid(
                "Omnigraph archive is missing expected `omnigraph` or `omnigraph-server` binary.",
            ));
        }
        if dest_dir.exists() {
            std::fs::remove_dir_all(dest_dir)?;
        }
        std::fs::rename(&stage, dest_dir)?;
        Ok(extracted)
    })();

    if result.is_err() {
        let _ = std::fs::remove_dir_all(&stage);
    }
    result
}

fn terminate_child_sync(child: &mut Child) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }
    #[cfg(unix)]
    {
        if let Ok(pid) = i32::try_from(child.id()) {
            // SAFETY: sending SIGTERM to the child process we spawned and still own before reaping.
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                if child.try_wait().ok().flatten().is_some() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}
