use super::context_sync::query_edges_safe;
use super::{
    CachedStats, MAX_ARCHIVE_FILES, MAX_EXPANDED_BYTES, MAX_TARBALL_BYTES, OmnigraphConfig,
    OmnigraphManager, OmnigraphMode, STATS_CACHE_TTL, StartingGuard,
};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use crate::omnigraph::schema::OMNIGRAPH_RELEASE_URL_LINUX_X86_64;
use crate::omnigraph::{
    client::{CommitListQuery, OmnigraphClient, QueryRequest, StoredQueryRequest},
    schema::{
        self, GQ_COUNT_ATOM_IDS, GQ_COUNT_ATOM_RELATION_IDS, GQ_COUNT_CONSUMED_EDGE_IDS,
        GQ_COUNT_FILE_EDGE_IDS, GQ_COUNT_PUBLISHED_EDGE_IDS, GQ_COUNT_SESSION_FOLLOW_IDS,
        GQ_COUNT_SESSION_IDS, GQ_COUNT_SYMBOL_EDGE_IDS, GQ_COUNT_SYMBOL_IDS, OMNIGRAPH_VERSION,
    },
};
use crate::{HarnessError, Result, store};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Cursor, Read},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

impl OmnigraphManager {
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

    fn client_from_parts(
        &self,
        endpoint: &str,
        token: Option<String>,
        graph_id: &str,
        actor_id: &str,
    ) -> Result<OmnigraphClient> {
        OmnigraphClient::with_http_client(endpoint, token, graph_id, self.http.clone())
            .map(|c| c.with_actor(Some(actor_id.to_string())))
            .map_err(Into::into)
    }

    /// Download and extract Omnigraph release tarball from `url` into `<state>/tools/omnigraph/v0.15.0/`.
    pub async fn install_local_from(&self, url: &str) -> Result<Value> {
        self.install_local_with_sha256(url, None).await
    }

    /// Download, SHA-256 verify (against `expected_sha256` or `<url>.sha256` when published),
    /// and extract Omnigraph release tarball into `<state>/tools/omnigraph/v0.15.0/`.
    pub async fn install_local_with_sha256(
        &self,
        url: &str,
        expected_sha256: Option<&str>,
    ) -> Result<Value> {
        let normalized_expected = expected_sha256
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(normalize_sha256_hex)
            .transpose()?;

        let resp = self
            .http
            .get(url)
            .timeout(Duration::from_secs(180))
            .send()
            .await
            .map_err(|err| {
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

        let actual_sha256 = sha256_hex(&compressed);
        let (sha256_verified, sha256_source) = if let Some(ref expected) = normalized_expected {
            if &actual_sha256 != expected {
                return Err(HarnessError::invalid(format!(
                    "Omnigraph archive SHA-256 mismatch: expected {expected}, got {actual_sha256}."
                )));
            }
            (true, Some("expected"))
        } else if let Some(sidecar_url) = sidecar_sha256_url(url) {
            match self
                .http
                .get(&sidecar_url)
                .timeout(Duration::from_secs(5))
                .send()
                .await
            {
                Ok(sha_resp) if sha_resp.status().is_success() => {
                    let sha_text = sha_resp.text().await.map_err(|err| {
                        HarnessError::unavailable(format!(
                            "Failed reading Omnigraph checksum from {sidecar_url}: {err}"
                        ))
                    })?;
                    let expected = parse_sha256_checksum_text(&sha_text, url)?;
                    if actual_sha256 != expected {
                        return Err(HarnessError::invalid(format!(
                            "Omnigraph archive SHA-256 mismatch against {sidecar_url}: expected {expected}, got {actual_sha256}."
                        )));
                    }
                    (true, Some("sidecar_sha256"))
                }
                _ => (false, None),
            }
        } else {
            (false, None)
        };

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
            "sha256": actual_sha256,
            "sha256_verified": sha256_verified,
            "sha256_source": sha256_source,
        }))
    }

    /// Build a typed `OmnigraphClient` for the currently active/configured server,
    /// reusing the manager's pooled HTTP client.
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
        self.client_from_parts(endpoint, cfg.token.clone(), &cfg.graph_id, &cfg.actor_id)
    }

    /// Start the managed local `omnigraph-server` sidecar in cluster mode.
    pub async fn start_local(&self, graph_id: Option<&str>) -> Result<Value> {
        let starting_guard = StartingGuard::enter(&self.starting);
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
        if let Ok(mut ring) = self.stderr_ring.lock() {
            ring.clear();
        }

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

        let mut child = Command::new(&server_path)
            .args([
                "--cluster",
                &abs_data.to_string_lossy(),
                "--bind",
                &bind_addr,
            ])
            .env("OMNIGRAPH_SERVER_BEARER_TOKENS_JSON", &tokens_json)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| {
                HarnessError::unavailable(format!("Failed spawning omnigraph-server: {err}"))
            })?;

        if let Some(stderr) = child.stderr.take() {
            let ring = Arc::clone(&self.stderr_ring);
            let _ = std::thread::Builder::new()
                .name("omnigraph-stderr".into())
                .spawn(move || {
                    let reader = BufReader::new(stderr);
                    for line in reader.lines().map_while(|r| r.ok()) {
                        if let Ok(mut guard) = ring.lock() {
                            guard.push_line(&line);
                        }
                    }
                });
        }

        {
            let mut proc_guard = self.process.lock().map_err(|_| {
                HarnessError::unavailable("Omnigraph process state lock was poisoned.")
            })?;
            proc_guard.child = Some(child);
        }

        let endpoint = format!("http://{bind_addr}");
        let client =
            self.client_from_parts(&endpoint, Some(token.clone()), &cfg.graph_id, &cfg.actor_id)?;

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
        self.invalidate_stats_cache();
        drop(starting_guard);
        self.status_json("").await
    }

    /// Start Omnigraph via `ghcr.io/modernrelay/omnigraph-server:v0.15.0` Docker container.
    pub async fn start_docker(&self, graph_id: Option<&str>) -> Result<Value> {
        let starting_guard = StartingGuard::enter(&self.starting);
        let docker = infra_host::provider::find_executable("docker")
            .ok_or_else(|| HarnessError::missing("Docker CLI was not found on PATH."))?;
        if !tokio::task::spawn_blocking(Self::docker_available)
            .await
            .unwrap_or(false)
        {
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
        let client =
            self.client_from_parts(&endpoint, Some(token.clone()), &cfg.graph_id, &cfg.actor_id)?;

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
        self.invalidate_stats_cache();
        drop(starting_guard);
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

        let client = self.client_from_parts(
            &clean_endpoint,
            clean_token.clone(),
            &cfg.graph_id,
            &cfg.actor_id,
        )?;

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
        self.invalidate_stats_cache();
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
                let was_running = self
                    .has_running_container_async(cfg.container_name.as_deref())
                    .await;
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

    /// Gracefully stop any spawned local `omnigraph-server` child or Docker container,
    /// including pre-restart Docker containers tracked via `config.container_name`.
    pub async fn close(&self) -> Result<()> {
        let cfg = self.config().await;
        let (child_opt, mut container_opt) = {
            let mut proc_guard = self.process.lock().map_err(|_| {
                HarnessError::unavailable("Omnigraph process state lock was poisoned.")
            })?;
            (proc_guard.child.take(), proc_guard.docker_container.take())
        };

        if container_opt.is_none()
            && cfg.mode == OmnigraphMode::Docker
            && let Some(ref persisted_name) = cfg.container_name
            && tokio::task::spawn_blocking({
                let name = persisted_name.clone();
                move || docker_container_running(&name)
            })
            .await
            .unwrap_or(false)
        {
            container_opt = Some(persisted_name.clone());
        }

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
        self.invalidate_stats_cache();
        Ok(())
    }

    fn has_running_child(&self) -> bool {
        self.process
            .lock()
            .ok()
            .and_then(|mut g| g.child.as_mut().map(|c| c.try_wait().ok() == Some(None)))
            .unwrap_or(false)
    }

    async fn has_running_container_async(&self, configured_name: Option<&str>) -> bool {
        let in_mem = self
            .process
            .lock()
            .ok()
            .and_then(|g| g.docker_container.clone());
        let configured = configured_name.map(str::to_string);
        tokio::task::spawn_blocking(move || {
            resolve_running_container(in_mem.as_deref(), configured.as_deref(), None).is_some()
        })
        .await
        .unwrap_or(false)
    }

    #[cfg(test)]
    pub(in crate::omnigraph) fn detect_running_container_with_docker_for_test(
        &self,
        configured_name: Option<&str>,
        docker_bin: Option<&Path>,
    ) -> Option<String> {
        let in_mem = self
            .process
            .lock()
            .ok()
            .and_then(|g| g.docker_container.clone());
        resolve_running_container(in_mem.as_deref(), configured_name, docker_bin)
    }

    async fn wait_ready_local(&self, client: &OmnigraphClient, timeout: Duration) -> Result<()> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if let Ok(mut guard) = self.process.lock()
                && let Some(child) = guard.child.as_mut()
                && let Ok(Some(status)) = child.try_wait()
            {
                let tail = self.stderr_tail(8);
                let detail = if tail.is_empty() {
                    String::new()
                } else {
                    format!(" stderr: {}", tail.join(" | "))
                };
                return Err(HarnessError::unavailable(format!(
                    "omnigraph-server exited during startup with status {status}.{detail}"
                )));
            }
            if let Ok(ready) = client.ready().await
                && ready.ready
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(120)).await;
        }
        let tail = self.stderr_tail(8);
        let detail = if tail.is_empty() {
            String::new()
        } else {
            format!(" stderr: {}", tail.join(" | "))
        };
        Err(HarnessError::unavailable(format!(
            "Timed out waiting for omnigraph-server /readyz.{detail}"
        )))
    }

    /// Compute status JSON expected by `GET /api/omnigraph/status`.
    pub async fn status_json(&self, project_id: &str) -> Result<Value> {
        let (cfg, last_error) = {
            let rt = self.runtime.lock().await;
            (rt.config.clone(), rt.last_error.clone())
        };
        let is_starting_flag = self.starting.load(Ordering::Acquire);
        let stderr_tail = self.stderr_tail(64);
        let tools_dir = self.tools_dir();
        let configured_container = cfg.container_name.clone();
        let in_mem_container = self
            .process
            .lock()
            .ok()
            .and_then(|g| g.docker_container.clone());
        let is_docker_mode = cfg.mode == OmnigraphMode::Docker;

        let (binaries, docker_available, docker_running) = tokio::task::spawn_blocking(move || {
            let cli = find_binary(&tools_dir, "omnigraph");
            let server = find_binary(&tools_dir, "omnigraph-server");
            let version = cli.as_deref().and_then(probe_omnigraph_version);
            let binaries = BinaryDiscovery {
                cli,
                server,
                version,
            };
            let docker_avail = Self::docker_available();
            let docker_up = if is_docker_mode && docker_avail {
                in_mem_container
                    .as_deref()
                    .or(configured_container.as_deref())
                    .is_some_and(docker_container_running)
            } else {
                false
            };
            (binaries, docker_avail, docker_up)
        })
        .await
        .unwrap_or_else(|_| (BinaryDiscovery::default(), false, false));

        let mut connected = false;
        let mut starting = is_starting_flag;
        let mut state = if is_starting_flag {
            "starting"
        } else if cfg.mode == OmnigraphMode::Off {
            "unconfigured"
        } else if docker_running {
            "starting"
        } else {
            "stopped"
        };
        let mut error = if is_starting_flag { None } else { last_error };
        let mut mcp_url: Option<String> = None;
        let mut server_version: Option<String> = None;
        let mut stats = json!({
            "branches": 0,
            "atoms": 0,
            "sessions": 0,
            "edges": 0,
            "commits": 0,
            "unmerged_branches": 0
        });

        if !is_starting_flag
            && cfg.mode != OmnigraphMode::Off
            && let Some(endpoint) = cfg.endpoint.as_deref()
            && let Ok(client) =
                self.client_from_parts(endpoint, cfg.token.clone(), &cfg.graph_id, &cfg.actor_id)
        {
            match client.health().await {
                Ok(health) => {
                    server_version = (!health.version.trim().is_empty()).then_some(health.version);
                    let ready_ok = client.ready().await.map(|r| r.ready).unwrap_or(true);
                    if ready_ok {
                        connected = true;
                        starting = false;
                        state = "connected";
                        error = None;
                        mcp_url = Some(client.mcp_url());
                        if let Ok(computed) = self
                            .cached_or_compute_stats(&client, endpoint, &cfg.graph_id, project_id)
                            .await
                        {
                            stats = computed;
                        }
                    } else {
                        starting = true;
                        state = "starting";
                    }
                }
                Err(err) => {
                    state = "error";
                    error = Some(if stderr_tail.is_empty() {
                        err.message
                    } else {
                        format!(
                            "{} (stderr: {})",
                            err.message,
                            stderr_tail
                                .iter()
                                .rev()
                                .take(4)
                                .cloned()
                                .collect::<Vec<_>>()
                                .into_iter()
                                .rev()
                                .collect::<Vec<_>>()
                                .join(" | ")
                        )
                    });
                }
            }
        }

        let auto_download_supported = cfg!(all(target_os = "linux", target_arch = "x86_64"));
        let version = server_version.or_else(|| binaries.version.clone());

        Ok(json!({
            "connected": connected,
            "state": state,
            "starting": starting,
            "mode": cfg.mode.as_str(),
            "api_contract": crate::omnigraph::client::HTTP_API_CONTRACT_VERSION,
            "target_version": OMNIGRAPH_VERSION,
            "version": version,
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "auto_download_supported": auto_download_supported,
            "platform": {
                "os": std::env::consts::OS,
                "arch": std::env::consts::ARCH,
                "auto_download_supported": auto_download_supported,
            },
            "endpoint": cfg.endpoint,
            "graph_id": cfg.graph_id,
            "mcp_url": mcp_url,
            "binary_installed": binaries.cli.is_some() && binaries.server.is_some(),
            "binary_version": binaries.version,
            "docker_available": docker_available,
            "docker_container": cfg.container_name,
            "docker_running": docker_running,
            "error": error,
            "stderr_tail": stderr_tail,
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
                let expected_sha256 = body["sha256"].as_str();
                if let Some(url) = body["release_url"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                {
                    self.install_local_with_sha256(url, expected_sha256).await
                } else if expected_sha256.is_some() {
                    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
                    {
                        Err(HarnessError::invalid(
                            "Automatic local download currently supports Linux x86_64; install `omnigraph` on PATH or use Docker/External mode.",
                        ))
                    }
                    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
                    {
                        self.install_local_with_sha256(
                            OMNIGRAPH_RELEASE_URL_LINUX_X86_64,
                            expected_sha256,
                        )
                        .await
                    }
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

    pub(super) async fn cached_or_compute_stats(
        &self,
        client: &OmnigraphClient,
        endpoint: &str,
        graph_id: &str,
        project_id: &str,
    ) -> Result<Value> {
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
                    .filter(|d| {
                        d.name != "main" && Some(d.head.graph_commit_id.as_str()) != main_head
                    })
                    .count()
            })
            .unwrap_or_else(|| branches_count.saturating_sub(1));

        let branches_sig = build_branches_signature(&branches_resp);
        if let Ok(guard) = self.stats_cache.lock()
            && let Some(cached) = guard.as_ref()
            && cached.endpoint == endpoint
            && cached.graph_id == graph_id
            && cached.project_id == project_id
            && cached.branches_sig == branches_sig
            && cached.computed_at.elapsed() < STATS_CACHE_TTL
        {
            return Ok(json!({
                "branches": branches_count,
                "atoms": cached.atoms,
                "sessions": cached.sessions,
                "symbols": cached.symbols,
                "edges": cached.edges,
                "commits": cached.commits,
                "unmerged_branches": unmerged_branches,
            }));
        }

        let counts = compute_lightweight_counts(client, project_id).await;
        if let Ok(mut guard) = self.stats_cache.lock() {
            *guard = Some(CachedStats {
                endpoint: endpoint.to_string(),
                graph_id: graph_id.to_string(),
                project_id: project_id.to_string(),
                branches_sig,
                computed_at: Instant::now(),
                atoms: counts.atoms,
                sessions: counts.sessions,
                symbols: counts.symbols,
                edges: counts.edges,
                commits: counts.commits,
            });
        }

        Ok(json!({
            "branches": branches_count,
            "atoms": counts.atoms,
            "sessions": counts.sessions,
            "symbols": counts.symbols,
            "edges": counts.edges,
            "commits": counts.commits,
            "unmerged_branches": unmerged_branches,
        }))
    }
}

/// Discovered local `omnigraph` CLI and `omnigraph-server` binary locations.
#[derive(Clone, Debug, Default)]
pub struct BinaryDiscovery {
    pub cli: Option<PathBuf>,
    pub server: Option<PathBuf>,
    pub version: Option<String>,
}

struct LightweightStatsCounts {
    atoms: usize,
    sessions: usize,
    symbols: usize,
    edges: usize,
    commits: usize,
}

pub(super) fn build_branches_signature(
    branches_resp: &crate::omnigraph::client::BranchListResponse,
) -> String {
    if let Some(details) = branches_resp.details.as_ref() {
        let mut parts: Vec<String> = details
            .iter()
            .map(|d| format!("{}={}", d.name, d.head.graph_commit_id))
            .collect();
        parts.sort();
        parts.join("|")
    } else {
        let mut names = branches_resp.branches.clone();
        names.sort();
        names.join("|")
    }
}

async fn count_atom_ids(client: &OmnigraphClient, project_id: &str) -> Result<usize> {
    let params = json!({ "project_id": project_id });
    match client
        .query(&QueryRequest {
            query: GQ_COUNT_ATOM_IDS.into(),
            name: Some("recent_atoms".into()),
            params: Some(params.clone()),
            branch: Some("main".into()),
            snapshot: None,
            settings: None,
        })
        .await
    {
        Ok(resp) => Ok(resp.rows.len()),
        Err(err) if matches!(err.status, 401 | 403) => client
            .stored_query(
                "recent_atoms",
                &StoredQueryRequest {
                    params: Some(params),
                    branch: Some("main".into()),
                    snapshot: None,
                    expect_mutation: Some(false),
                },
            )
            .await
            .map(|r| r.rows.len())
            .map_err(Into::into),
        Err(err) => Err(err.into()),
    }
}

async fn compute_lightweight_counts(
    client: &OmnigraphClient,
    project_id: &str,
) -> LightweightStatsCounts {
    let params = json!({ "project_id": project_id });
    let commit_query = CommitListQuery {
        branch: Some("main".into()),
        limit: Some(100),
        page_token: None,
    };
    let (
        commits_res,
        atoms_res,
        sessions_rows,
        symbols_rows,
        pub_edges,
        con_edges,
        file_edges,
        rel_edges,
        follow_edges,
        symbol_edges,
    ) = tokio::join!(
        client.list_commits(&commit_query),
        count_atom_ids(client, project_id),
        query_edges_safe(
            client,
            GQ_COUNT_SESSION_IDS,
            "list_sessions",
            &params,
            Some("main"),
            None,
        ),
        query_edges_safe(
            client,
            GQ_COUNT_SYMBOL_IDS,
            "list_symbols",
            &params,
            Some("main"),
            None,
        ),
        query_edges_safe(
            client,
            GQ_COUNT_PUBLISHED_EDGE_IDS,
            "list_published_edges",
            &params,
            Some("main"),
            None,
        ),
        query_edges_safe(
            client,
            GQ_COUNT_CONSUMED_EDGE_IDS,
            "list_consumed_edges",
            &params,
            Some("main"),
            None,
        ),
        query_edges_safe(
            client,
            GQ_COUNT_FILE_EDGE_IDS,
            "list_file_edges",
            &params,
            Some("main"),
            None,
        ),
        query_edges_safe(
            client,
            GQ_COUNT_ATOM_RELATION_IDS,
            "list_atom_relations",
            &params,
            Some("main"),
            None,
        ),
        query_edges_safe(
            client,
            GQ_COUNT_SESSION_FOLLOW_IDS,
            "list_session_follows",
            &params,
            Some("main"),
            None,
        ),
        query_edges_safe(
            client,
            GQ_COUNT_SYMBOL_EDGE_IDS,
            "list_symbol_edges",
            &params,
            Some("main"),
            None,
        ),
    );

    LightweightStatsCounts {
        commits: commits_res.map(|c| c.commits.len()).unwrap_or(0),
        atoms: atoms_res.unwrap_or(0),
        sessions: sessions_rows.len(),
        symbols: symbols_rows.len(),
        edges: pub_edges.len()
            + con_edges.len()
            + file_edges.len()
            + rel_edges.len()
            + follow_edges.len()
            + symbol_edges.len(),
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

pub(super) fn load_config_file(path: &Path) -> Result<OmnigraphConfig> {
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

pub(super) fn save_config_file(path: &Path, config: &OmnigraphConfig) -> Result<()> {
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

pub(super) fn find_binary(tools_dir: &Path, name: &str) -> Option<PathBuf> {
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

pub(super) fn probe_omnigraph_version(cli_path: &Path) -> Option<String> {
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

pub(super) fn terminate_child_sync(child: &mut Child) {
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

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn normalize_sha256_hex(raw: &str) -> Result<String> {
    let trimmed = raw.trim().trim_start_matches("sha256:").trim();
    if trimmed.len() != 64 || !trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(HarnessError::invalid(
            "Invalid SHA-256 digest: expected 64 hexadecimal characters.",
        ));
    }
    Ok(trimmed.to_ascii_lowercase())
}

fn sidecar_sha256_url(url: &str) -> Option<String> {
    let without_frag = url.split('#').next().unwrap_or(url);
    let (base, query) = match without_frag.split_once('?') {
        Some((b, q)) => (b, Some(q)),
        None => (without_frag, None),
    };
    if base.is_empty() || base.ends_with(".sha256") {
        return None;
    }
    Some(match query {
        Some(q) => format!("{base}.sha256?{q}"),
        None => format!("{base}.sha256"),
    })
}

fn parse_sha256_checksum_text(text: &str, url: &str) -> Result<String> {
    let target_name = url
        .split('#')
        .next()
        .unwrap_or(url)
        .split('?')
        .next()
        .unwrap_or(url)
        .rsplit('/')
        .next()
        .unwrap_or("");
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(hex) = parts.next() else {
            continue;
        };
        let digest = normalize_sha256_hex(hex)?;
        if let Some(listed_file) = parts.next() {
            let clean = listed_file.trim_start_matches('*');
            let base = clean.rsplit('/').next().unwrap_or(clean);
            if !target_name.is_empty() && !base.is_empty() && base != target_name {
                return Err(HarnessError::invalid(format!(
                    "SHA-256 manifest entry '{clean}' does not match '{target_name}'."
                )));
            }
        }
        return Ok(digest);
    }
    Err(HarnessError::invalid(
        "Omnigraph SHA-256 checksum file was empty.",
    ))
}

fn docker_container_running(container_name: &str) -> bool {
    docker_container_running_with(None, container_name)
}

pub(super) fn docker_container_running_with(
    docker_override: Option<&Path>,
    container_name: &str,
) -> bool {
    let trimmed = container_name.trim();
    if trimmed.is_empty() {
        return false;
    }
    let discovered;
    let docker = match docker_override {
        Some(path) => path,
        None => {
            let Some(found) = infra_host::provider::find_executable("docker") else {
                return false;
            };
            discovered = found;
            discovered.as_path()
        }
    };
    Command::new(docker)
        .args(["inspect", "-f", "{{.State.Running}}", trimmed])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|out| {
            out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "true"
        })
}

pub(super) fn resolve_running_container(
    in_mem: Option<&str>,
    configured: Option<&str>,
    docker_override: Option<&Path>,
) -> Option<String> {
    if let Some(name) = in_mem.map(str::trim).filter(|s| !s.is_empty()) {
        if docker_override.is_some() || infra_host::provider::find_executable("docker").is_some() {
            return docker_container_running_with(docker_override, name).then(|| name.to_string());
        }
        return Some(name.to_string());
    }
    configured
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter(|name| docker_container_running_with(docker_override, name))
        .map(str::to_string)
}
