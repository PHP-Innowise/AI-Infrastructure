//! Out-of-process Omnigraph sidecar/Docker/external lifecycle manager (`OmnigraphManager`)
//! and Harness service domain (`OmnigraphDomain`).

use super::{
    client::{
        BranchCreateRequest, BranchDiffQuery, BranchMergeRequest, ChangeBaselineRequest,
        ChangeFeedQuery, ChangeOp, ChangeRequest, CommitListQuery, EntityKind, OmnigraphClient,
        QueryRequest, StoredQueryRequest,
    },
    schema::{
        self, GQ_LINK_CONSUMED_BY, GQ_LINK_DERIVED_FROM, GQ_LINK_PUBLISHED_BY, GQ_LINK_RELATES_TO,
        GQ_LINK_TOUCHES_FILE, GQ_LIST_ATOM_RELATIONS, GQ_LIST_CONSUMED_EDGES, GQ_LIST_FILE_EDGES,
        GQ_LIST_FILES, GQ_LIST_PUBLISHED_EDGES, GQ_LIST_SESSIONS, GQ_UPSERT_ATOM, GQ_UPSERT_FILE,
        GQ_UPSERT_SESSION, GQ_UPSERT_TURN_SNAPSHOT, HARNESS_GRAPH_ID, OMNIGRAPH_DOCKER_IMAGE,
        OMNIGRAPH_RELEASE_URL_LINUX_X86_64, OMNIGRAPH_VERSION,
    },
    session_branch_name,
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

/// Manages Omnigraph lifecycle, config persistence, and high-level Harness context operations.
pub struct OmnigraphManager {
    state_dir: PathBuf,
    store: Option<SessionStore>,
    runtime: Mutex<RuntimeState>,
    process: Arc<StdMutex<ProcessHandle>>,
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

    /// Build full constellation graph + branch/commit DAG payload for `/api/omnigraph/graph`.
    pub async fn graph_view_json(
        &self,
        project_id: &str,
        branch: Option<&str>,
        snapshot: Option<&str>,
        kind: Option<&str>,
        q: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<Value> {
        let cfg = self.config().await;
        let active_branch = clean_opt(branch).unwrap_or("main").to_string();
        let snapshot_commit = clean_opt(snapshot).map(str::to_string);

        let Ok(client) = self.client().await else {
            return Ok(empty_graph_view(
                &cfg.graph_id,
                &active_branch,
                snapshot_commit,
            ));
        };

        let branch_list = match client.list_branches(true).await {
            Ok(list) => list,
            Err(_) => {
                return Ok(empty_graph_view(
                    &cfg.graph_id,
                    &active_branch,
                    snapshot_commit,
                ));
            }
        };

        let main_head = branch_list
            .details
            .as_ref()
            .and_then(|details| details.iter().find(|d| d.name == "main"))
            .map(|d| d.head.graph_commit_id.clone());

        let branches_json: Vec<Value> = if let Some(details) = &branch_list.details {
            details
                .iter()
                .map(|d| {
                    let unmerged = d.name != "main"
                        && main_head
                            .as_deref()
                            .is_some_and(|mh| mh != d.head.graph_commit_id);
                    json!({
                        "name": d.name,
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
        };

        let commits_resp = client
            .list_commits(&CommitListQuery {
                branch: Some(active_branch.clone()),
                limit: Some(50),
                page_token: None,
            })
            .await
            .unwrap_or_default();

        let commits_json: Vec<Value> = commits_resp
            .commits
            .iter()
            .map(|c| {
                json!({
                    "graph_commit_id": c.graph_commit_id,
                    "commit_id": c.graph_commit_id,
                    "branch": c.graph_branch.as_deref().unwrap_or(&active_branch),
                    "graph_branch": c.graph_branch,
                    "parent_commit_id": c.parent_commit_id,
                    "merged_parent_commit_id": c.merged_parent_commit_id,
                    "actor_id": c.actor_id,
                    "created_at": c.created_at,
                    "published": c.published,
                    "summary": format!("Commit {}", &c.graph_commit_id.chars().take(12).collect::<String>()),
                    "atom_delta": 0,
                })
            })
            .collect();

        let target_branch = if snapshot_commit.is_some() {
            None
        } else {
            Some(active_branch.clone())
        };
        let params = json!({ "project_id": project_id });

        let atoms_res = run_stored_or_inline(
            &client,
            "recent_atoms",
            schema::QUERY_RECENT_ATOMS,
            params.clone(),
            target_branch.clone(),
            snapshot_commit.clone(),
        )
        .await?;
        let head_commit = atoms_res.graph_commit_id.clone().or_else(|| {
            branch_list
                .details
                .as_ref()
                .and_then(|ds| ds.iter().find(|d| d.name == active_branch))
                .map(|d| d.head.graph_commit_id.clone())
        });

        let sessions_res = client
            .query(&QueryRequest {
                query: GQ_LIST_SESSIONS.into(),
                name: Some("list_sessions".into()),
                params: Some(params.clone()),
                branch: target_branch.clone(),
                snapshot: snapshot_commit.clone(),
                settings: None,
            })
            .await
            .unwrap_or_default();

        let files_res = client
            .query(&QueryRequest {
                query: GQ_LIST_FILES.into(),
                name: Some("list_files".into()),
                params: Some(params.clone()),
                branch: target_branch.clone(),
                snapshot: snapshot_commit.clone(),
                settings: None,
            })
            .await
            .unwrap_or_default();

        let pub_edges = query_edges_safe(
            &client,
            GQ_LIST_PUBLISHED_EDGES,
            "list_published_edges",
            &params,
            target_branch.as_deref(),
            snapshot_commit.as_deref(),
        )
        .await;
        let con_edges = query_edges_safe(
            &client,
            GQ_LIST_CONSUMED_EDGES,
            "list_consumed_edges",
            &params,
            target_branch.as_deref(),
            snapshot_commit.as_deref(),
        )
        .await;
        let file_edges = query_edges_safe(
            &client,
            GQ_LIST_FILE_EDGES,
            "list_file_edges",
            &params,
            target_branch.as_deref(),
            snapshot_commit.as_deref(),
        )
        .await;
        let rel_edges = query_edges_safe(
            &client,
            GQ_LIST_ATOM_RELATIONS,
            "list_atom_relations",
            &params,
            target_branch.as_deref(),
            snapshot_commit.as_deref(),
        )
        .await;

        let mut files_by_atom: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for edge in &file_edges {
            if let (Some(src), Some(dst)) =
                (str_field(edge, "source_id"), str_field(edge, "target_id"))
            {
                files_by_atom.entry(src).or_default().push(dst);
            }
        }

        let kind_filter = clean_opt(kind).map(str::to_ascii_lowercase);
        let q_filter = clean_opt(q).map(str::to_ascii_lowercase);
        let sid_filter = clean_opt(session_id);

        let mut session_linked_atoms = BTreeSet::new();
        if let Some(sid) = sid_filter {
            for e in pub_edges.iter().chain(con_edges.iter()) {
                if str_field(e, "target_id").as_deref() == Some(sid)
                    && let Some(aid) = str_field(e, "source_id")
                {
                    session_linked_atoms.insert(aid);
                }
            }
        }

        let mut atoms = Vec::new();
        let mut kept_atom_ids = BTreeSet::new();
        for row in &atoms_res.rows {
            let mut atom = normalize_atom_row(row);
            let aid = str_field(&atom, "atom_id").unwrap_or_default();
            if let Some(files) = files_by_atom.get(&aid) {
                atom["files"] = json!(files);
            } else {
                atom["files"] = json!([]);
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
            if !aid.is_empty() {
                kept_atom_ids.insert(aid);
            }
            atoms.push(atom);
        }

        let sessions: Vec<Value> = sessions_res
            .rows
            .iter()
            .map(normalize_session_row)
            .collect();
        let files: Vec<Value> = files_res.rows.iter().map(normalize_file_row).collect();

        let mut edges = Vec::new();
        for e in &pub_edges {
            if let (Some(src), Some(dst)) = (str_field(e, "source_id"), str_field(e, "target_id"))
                && (kept_atom_ids.is_empty() || kept_atom_ids.contains(&src))
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
        for e in &con_edges {
            if let (Some(src), Some(dst)) = (str_field(e, "source_id"), str_field(e, "target_id"))
                && (kept_atom_ids.is_empty() || kept_atom_ids.contains(&src))
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
        for e in &file_edges {
            if let (Some(src), Some(dst)) = (str_field(e, "source_id"), str_field(e, "target_id"))
                && (kept_atom_ids.is_empty() || kept_atom_ids.contains(&src))
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
        for e in &rel_edges {
            if let (Some(src), Some(dst)) = (str_field(e, "source_id"), str_field(e, "target_id"))
                && (kept_atom_ids.is_empty()
                    || kept_atom_ids.contains(&src)
                    || kept_atom_ids.contains(&dst))
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
            "edges": edges,
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
        for (atom, score, matched_via) in raw_hits {
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
            let mut neighbors = Vec::new();
            for fe in &file_edges {
                if str_field(fe, "source_id").as_deref() == Some(aid.as_str())
                    && let Some(path) = str_field(fe, "target_id")
                {
                    neighbors
                        .push(json!({ "kind": "file", "id": path, "relation": "touches_file" }));
                }
            }
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
        _project_id: &str,
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
                    added_edges.push(json!({
                        "id": ch.id,
                        "type": ch.entity_type.name,
                        "op": ch.op.as_str(),
                        "endpoints": ch.after.as_ref().and_then(|a| a.endpoints.clone()),
                        "properties": ch.after.as_ref().map(|a| a.properties.clone()).unwrap_or(Value::Null),
                    }));
                }
                (EntityKind::Edge, ChangeOp::Delete) => {}
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
            Err(err) if err.is_merge_conflict() => ("conflict".to_string(), err.merge_conflicts),
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
                    Err(err) if err.is_merge_conflict() => Ok(json!({
                        "ok": false,
                        "action": "merge",
                        "source": source,
                        "target": target,
                        "outcome": "conflict",
                        "dry_run": dry_run,
                        "conflicts": err.merge_conflicts,
                        "error": err.message,
                    })),
                    Err(err) => Err(err.into()),
                }
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
    pub async fn publish_atom_json(&self, body: &Value) -> Result<Value> {
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
            .map(str::to_string)
            .unwrap_or_else(|| format!("atom-{}", &store::hexid()[..12]));
        let session_id = clean_opt(body["session_id"].as_str()).map(str::to_string);
        let tags: Option<Vec<String>> = body["tags"].as_array().map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        });
        let files: Vec<String> = body["files"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        let confidence = body["confidence"].as_f64().unwrap_or(0.85).clamp(0.0, 1.0);
        let pin_priority = body["pin_priority"].as_i64().unwrap_or(10) as i32;
        let verified = body["verified"].as_bool().unwrap_or(false);
        let expected_commit = clean_opt(body["expected_commit"].as_str()).map(str::to_string);
        let ts = now_utc_iso_ms();

        let client = self.client().await?;
        let atom_params = json!({
            "atom_id": atom_id,
            "project_id": project_id,
            "kind": kind,
            "title": title,
            "summary": summary,
            "body": atom_body,
            "tags": tags,
            "source_branch": branch,
            "source_session_id": session_id,
            "confidence": confidence,
            "pin_priority": pin_priority,
            "verified": verified,
            "created_at": ts,
            "updated_at": ts,
        });
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

        for file_path in &files {
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
                    branch: Some(branch.clone()),
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
                    branch: Some(branch.clone()),
                    settings: None,
                })
                .await
                && r.commit.is_some()
            {
                last_commit = r.commit;
            }
        }

        if let Some(relations) = body["relates_to"].as_array() {
            for rel in relations {
                let Some(target_id) = clean_opt(rel["target_id"].as_str()) else {
                    continue;
                };
                let relation = clean_opt(rel["relation"].as_str())
                    .unwrap_or("supports")
                    .to_ascii_lowercase();
                if !VALID_RELATIONS.contains(&relation.as_str()) {
                    continue;
                }
                if let Ok(r) = client
                    .mutate(&ChangeRequest {
                        query: GQ_LINK_RELATES_TO.into(),
                        name: Some("link_relates_to".into()),
                        params: Some(json!({
                            "source_id": atom_id,
                            "target_id": target_id,
                            "relation": relation,
                            "note": clean_opt(rel["note"].as_str()),
                            "confidence": rel["confidence"].as_f64(),
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

        Ok(json!({
            "ok": true,
            "branch": branch,
            "atom": atom_params,
            "files": files,
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
            .to_string();
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
            for row in &source_atoms.rows {
                let atom = normalize_atom_row(row);
                let aid = str_field(&atom, "atom_id").unwrap_or_default();
                if atom_ids.contains(&aid) {
                    let _ = client
                        .mutate(&ChangeRequest {
                            query: GQ_UPSERT_ATOM.into(),
                            name: Some("upsert_atom".into()),
                            params: Some(json!({
                                "atom_id": aid,
                                "project_id": sess_project_id,
                                "kind": atom["kind"].as_str().unwrap_or("insight"),
                                "title": atom["title"].as_str().unwrap_or(""),
                                "summary": atom["summary"].as_str().unwrap_or(""),
                                "body": atom["body"].as_str().unwrap_or(""),
                                "tags": atom.get("tags").cloned().unwrap_or(Value::Null),
                                "source_branch": source_branch,
                                "source_session_id": atom.get("source_session_id").cloned().unwrap_or(Value::Null),
                                "confidence": atom["confidence"].as_f64().unwrap_or(0.8),
                                "pin_priority": atom["pin_priority"].as_i64().unwrap_or(10),
                                "verified": atom["verified"].as_bool().unwrap_or(false),
                                "created_at": ts,
                                "updated_at": ts,
                            })),
                            branch: Some(target_branch.clone()),
                            settings: None,
                        })
                        .await;
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

        if let Some(pull_branches) = omni_opts["pull_branches"].as_array() {
            for pull in pull_branches.iter().filter_map(Value::as_str) {
                let pull = pull.trim();
                if !pull.is_empty() && pull != branch {
                    let _ = client
                        .merge_branches(&BranchMergeRequest {
                            source: pull.to_string(),
                            target: Some(branch.clone()),
                            delete_branch: false,
                            as_commit: false,
                            dry_run: false,
                            settings: None,
                        })
                        .await;
                }
            }
        }

        let ts = now_utc_iso_ms();
        let goal = clean_opt(omni_opts["query"].as_str())
            .or_else(|| clean_opt(session["goal"].as_str()))
            .or_else(|| clean_opt(session["prompt"].as_str()))
            .or_else(|| clean_opt(session["title"].as_str()))
            .unwrap_or("");
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

        let mut ranked_atoms: Vec<(Value, f64)> = Vec::new();
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
                    ranked_atoms.push((atom, hit["score"].as_f64().unwrap_or(0.5)));
                }
            }
        }
        if ranked_atoms.len() < 8
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
                    ranked_atoms.push((atom, prio / 100.0));
                }
                if ranked_atoms.len() >= 10 {
                    break;
                }
            }
        }

        let mut consumed_count = 0usize;
        let mut atoms_out = Vec::new();
        for (atom, score) in ranked_atoms {
            if let Some(aid) = str_field(&atom, "atom_id") {
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
            }
            atoms_out.push(atom);
        }

        let capsule_text = format_context_capsule(&cfg.graph_id, &branch, project_path, &atoms_out);
        Ok(json!({
            "enabled": true,
            "graph_id": cfg.graph_id,
            "branch": branch,
            "head_commit": session_upsert.commit.map(|c| c.graph_commit_id),
            "capsule_text": capsule_text,
            "atoms": atoms_out,
            "consumed_count": consumed_count,
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
        if let Some(ref snap_id) = snapshot_id
            && let Ok(r) = client
                .mutate(&ChangeRequest {
                    query: GQ_UPSERT_TURN_SNAPSHOT.into(),
                    name: Some("upsert_turn_snapshot".into()),
                    params: Some(json!({
                        "snapshot_id": snap_id,
                        "session_id": sid,
                        "turn_index": 1,
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
                .publish_atom_json(&json!({
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
                    "verified": learning["verified"].as_bool().unwrap_or(true),
                }))
                .await?;
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
        let query = ChangeFeedQuery {
            branch: Some(target_branch.clone()),
            cursor: clean_opt(cursor).map(str::to_string),
            start: clean_opt(cursor).is_none().then(|| "beginning".to_string()),
            page_token: None,
            limit: Some(100),
        };
        let feed = match client.changes(&query).await {
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
                events.push(json!({
                    "graph_commit_id": block.cause.graph_commit_id,
                    "branch": block.cause.authored_branch,
                    "actor_id": block.cause.actor_id,
                    "authored_at": block.cause.authored_at,
                    "entity_kind": ch.kind,
                    "type_name": ch.entity_type.name,
                    "entity_id": ch.id,
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
                for ch in &detail.changes {
                    if ch.kind == EntityKind::Node && ch.entity_type.name == "ContextAtom" {
                        atoms.push(change_to_atom_entry(ch));
                    }
                }
                Ok(json!({
                    "commit": {
                        "graph_commit_id": detail.cause.graph_commit_id,
                        "parent_commit_id": detail.cause.parent_commit_id,
                        "merged_parent_commit_id": detail.cause.merged_parent_commit_id,
                        "branch": detail.cause.authored_branch,
                        "actor_id": detail.cause.actor_id,
                        "authored_at": detail.cause.authored_at,
                    },
                    "changes": detail.changes,
                    "atoms": atoms,
                }))
            }
            Err(err) if err.status == 409 => Ok(json!({
                "commit": {
                    "graph_commit_id": commit_id,
                    "parentless": true,
                },
                "changes": [],
                "atoms": [],
            })),
            Err(err) => Err(err.into()),
        }
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
                "omnigraph.graph" => {
                    self.manager
                        .graph_view_json(
                            project_id,
                            query["branch"].as_str(),
                            query["snapshot"].as_str(),
                            query["kind"].as_str(),
                            query["q"].as_str(),
                            query["session_id"].as_str(),
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

fn empty_graph_view(graph_id: &str, active_branch: &str, snapshot_commit: Option<String>) -> Value {
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
        "edges": [],
    })
}

async fn compute_stats(client: &OmnigraphClient, project_id: &str) -> Result<Value> {
    let branches_resp = client.list_branches(true).await?;
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

    let commits_count = client
        .list_commits(&CommitListQuery {
            branch: Some("main".into()),
            limit: Some(100),
            page_token: None,
        })
        .await
        .map(|c| c.commits.len())
        .unwrap_or(0);

    let params = json!({ "project_id": project_id });
    let atoms_count = run_stored_or_inline(
        client,
        "recent_atoms",
        schema::QUERY_RECENT_ATOMS,
        params.clone(),
        Some("main".into()),
        None,
    )
    .await
    .map(|r| r.rows.len())
    .unwrap_or(0);
    let sessions_count = client
        .query(&QueryRequest {
            query: GQ_LIST_SESSIONS.into(),
            name: Some("list_sessions".into()),
            params: Some(params.clone()),
            branch: Some("main".into()),
            snapshot: None,
            settings: None,
        })
        .await
        .map(|r| r.rows.len())
        .unwrap_or(0);
    let pub_cnt = query_edges_safe(
        client,
        GQ_LIST_PUBLISHED_EDGES,
        "list_published_edges",
        &params,
        Some("main"),
        None,
    )
    .await
    .len();
    let con_cnt = query_edges_safe(
        client,
        GQ_LIST_CONSUMED_EDGES,
        "list_consumed_edges",
        &params,
        Some("main"),
        None,
    )
    .await
    .len();
    let file_cnt = query_edges_safe(
        client,
        GQ_LIST_FILE_EDGES,
        "list_file_edges",
        &params,
        Some("main"),
        None,
    )
    .await
    .len();
    let rel_cnt = query_edges_safe(
        client,
        GQ_LIST_ATOM_RELATIONS,
        "list_atom_relations",
        &params,
        Some("main"),
        None,
    )
    .await
    .len();

    Ok(json!({
        "branches": branches_count,
        "atoms": atoms_count,
        "sessions": sessions_count,
        "edges": pub_cnt + con_cnt + file_cnt + rel_cnt,
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

fn normalize_atom_row(row: &Value) -> Value {
    let mut obj = match row.as_object() {
        Some(m) => m.clone(),
        None => Map::new(),
    };
    if let Some(id) = obj
        .get("atom_id")
        .and_then(Value::as_str)
        .map(str::to_string)
    {
        obj.entry("id").or_insert_with(|| Value::String(id));
    }
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

fn change_to_atom_entry(ch: &super::client::EntityChange) -> Value {
    let props = ch
        .after
        .as_ref()
        .or(ch.before.as_ref())
        .map(|img| img.properties.clone())
        .unwrap_or_else(|| json!({}));
    let mut out = normalize_atom_row(&props);
    if out.get("atom_id").is_none() {
        out["atom_id"] = json!(ch.id);
        out["id"] = json!(ch.id);
    }
    out["op"] = json!(ch.op.as_str());
    if let Some(before) = &ch.before {
        out["before"] = before.properties.clone();
    }
    if let Some(after) = &ch.after {
        out["after"] = after.properties.clone();
    }
    out
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
    if let Some(tags) = atom["tags"].as_array() {
        for tag in tags.iter().filter_map(Value::as_str) {
            if tag.to_ascii_lowercase().contains(q_lower) {
                return true;
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
) -> String {
    if atoms.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "Omnigraph Context Capsule (graph `{graph_id}`, branch `{branch}`, project `{}`):\n",
        project_path.display()
    );
    for atom in atoms {
        let kind = atom["kind"].as_str().unwrap_or("insight");
        let title = atom["title"].as_str().unwrap_or("Untitled");
        let summary = atom["summary"].as_str().unwrap_or("");
        let src = atom["source_branch"].as_str().unwrap_or("main");
        out.push_str(&format!("- [{kind}] {title} (from `{src}`): {summary}\n"));
    }
    out
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

fn detect_language(path: &str) -> Option<&'static str> {
    let ext = Path::new(path).extension().and_then(|e| e.to_str())?;
    match ext {
        "rs" => Some("rust"),
        "php" => Some("php"),
        "ts" | "tsx" => Some("typescript"),
        "js" | "jsx" => Some("javascript"),
        "py" => Some("python"),
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
