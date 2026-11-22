use crate::{
    HarnessError, Result,
    http::{App, serve_listener},
    services::Services,
    store,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
mod publication;
#[cfg(any(windows, test))]
mod windows_shortcut;

/// Closed native subprocess protocol; the target executable is never launched.
pub fn shortcut_worker(input: &[u8]) -> Result<Vec<u8>> {
    #[cfg(windows)]
    {
        windows_shortcut::worker(input)
    }
    #[cfg(not(windows))]
    {
        let _ = input;
        Err(HarnessError::unavailable(
            "Native Windows shortcuts are unavailable on this platform.",
        ))
    }
}
#[derive(Clone)]
pub struct ServerConfig {
    pub source: PathBuf,
    pub state: PathBuf,
    pub port: u16,
    pub timeout: u64,
    pub session_concurrency: usize,
    pub projects: Vec<PathBuf>,
    pub providers: BTreeMap<String, String>,
    pub path_env: Option<std::ffi::OsString>,
}
pub type FactoryFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Arc<dyn Services>>> + Send + 'a>>;
pub trait ServicesFactory: Send + Sync {
    fn create<'a>(
        &'a self,
        config: &'a ServerConfig,
        control: Arc<ServerControl>,
    ) -> FactoryFuture<'a>;
}
pub struct ServerControl {
    pub config: ServerConfig,
    pub signal: tokio::sync::watch::Sender<bool>,
}
impl ServerControl {
    fn pin_engine(&self, engine: &Path, expected_sha256: &str) -> Result<PathBuf> {
        let parent = engine
            .parent()
            .ok_or_else(|| HarnessError::invalid("Native release engine has no parent."))?;
        let name = engine
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| HarnessError::invalid("Native release engine name is invalid."))?;
        let mut source = infra_core::bank::publication::PublicationFiles::new(parent)?;
        let bytes = source
            .read(name, 512 * 1024 * 1024)?
            .ok_or_else(|| HarnessError::missing("The verified release engine disappeared."))?;
        let digest = store::sha(&bytes);
        if digest != expected_sha256 {
            return Err(HarnessError::conflict(
                "The release engine changed after artifact verification.",
            ));
        }
        let filename = if cfg!(windows) {
            "infra-engine.exe"
        } else {
            "infra-engine"
        };
        let relative = format!("engines/{digest}/{filename}");
        let mut state = infra_core::bank::publication::PublicationFiles::new(&self.config.state)?;
        let old = state.read(&relative, 512 * 1024 * 1024)?;
        if old.as_ref().is_some_and(|old| old != &bytes) {
            return Err(HarnessError::conflict(
                "A private engine path has different bytes.",
            ));
        }
        if old.is_none() {
            state.write_expected_mode_checked(
                &relative,
                &bytes,
                None,
                None,
                0o755,
                512 * 1024 * 1024,
            )?;
        }
        let destination = self.config.state.join(&relative);
        infra_host::util::private_directory(
            destination
                .parent()
                .ok_or_else(|| HarnessError::invalid("Private engine directory missing."))?,
        )?;
        let mut custody =
            infra_core::bank::publication::PublicationFiles::new(destination.parent().unwrap())?;
        if custody.read(filename, 512 * 1024 * 1024)?.as_deref() != Some(bytes.as_slice()) {
            return Err(HarnessError::conflict(
                "The private release engine changed.",
            ));
        }
        source.check()?;
        state.check()?;
        Ok(destination)
    }
}
fn installed_application_record() -> Result<Option<(PathBuf, Value)>> {
    let directory = config_directory()?;
    if !directory.is_dir() {
        return Ok(None);
    }
    let Some(record) = crate::creator_files::object(&directory, "app.json", false)? else {
        return Ok(None);
    };
    if record["installed"]
        .as_array()
        .is_none_or(|paths| paths.is_empty())
    {
        return Ok(None);
    }
    Ok(Some((directory, record)))
}
pub(crate) fn resolve_clone_root(path: &Path) -> Result<Option<PathBuf>> {
    let Ok(path) = infra_core::fs::canonicalize(path) else {
        return Ok(None);
    };
    if path.join("harness/web/index.html").is_file()
        && path.join("Infrastructure-Creator/AGENTS.md").is_file()
    {
        infra_core::bank::publication::PublicationFiles::new(&path)?;
        return Ok(Some(path));
    }
    Ok(None)
}
impl crate::updates::Restart for ServerControl {
    fn refresh_application(&self, engine: &Path, expected_sha256: &str) -> Result<Vec<String>> {
        let Some((_, record)) = installed_application_record()? else {
            return Ok(vec![]);
        };
        if record["clone"] != serde_json::to_value(&self.config.source)? {
            return Ok(vec![]);
        }
        let pinned = self.pin_engine(engine, expected_sha256)?;
        Ok(install_engine(&self.config.source, &pinned)?
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect())
    }
    fn handoff(&self, engine: &Path, report: &Value) -> Result<bool> {
        let destination = self.pin_engine(
            engine,
            report["engine_sha256"]
                .as_str()
                .ok_or_else(|| HarnessError::invalid("Release handoff digest missing."))?,
        )?;
        spawn_engine(&self.config, &destination, true)?;
        let _ = self.signal.send(true);
        Ok(true)
    }
}
fn home() -> Result<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .ok_or_else(|| HarnessError::unavailable("User home directory is unavailable."))
}
pub fn config_directory() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        Ok(std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or(home()?.join("AppData/Roaming"))
            .join("ai-infrastructure-harness"))
    }
    #[cfg(target_os = "macos")]
    {
        return Ok(home()?.join("Library/Application Support/ai-infrastructure-harness"));
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        Ok(std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or(home()?.join(".config"))
            .join("ai-infrastructure-harness"))
    }
}
pub fn default_state() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        Ok(std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or(home()?.join("AppData/Local"))
            .join("ai-infrastructure-harness"))
    }
    #[cfg(not(windows))]
    {
        Ok(std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or(home()?.join(".local/state"))
            .join("ai-infrastructure-harness"))
    }
}
fn parse_options(arguments: &[String], source: &Path) -> Result<ParsedOptions> {
    let args = arguments
        .iter()
        .flat_map(|value| {
            if value.starts_with("--")
                && let Some((name, value)) = value.split_once('=')
            {
                return vec![name.to_string(), value.to_string()];
            }
            vec![value.clone()]
        })
        .collect::<Vec<_>>();
    let mut config = ServerConfig {
        source: {
            std::fs::metadata(source)?;
            infra_core::fs::python_realpath(source)?
        },
        state: default_state()?,
        port: 8766,
        timeout: 900,
        session_concurrency: 1,
        projects: vec![],
        providers: BTreeMap::new(),
        path_env: None,
    };
    let mut explicit_source = false;
    let mut explicit_projects = vec![];
    let mut i = 0;
    let command = if args.first().is_some_and(|v| !v.starts_with('-')) {
        i = 1;
        args[0].clone()
    } else {
        "start".into()
    };
    if !["start", "serve", "status", "stop", "update", "relaunch"].contains(&command.as_str()) {
        return Err(HarnessError::invalid("Unknown Harness server command."));
    }
    while i < args.len() {
        let flag = &args[i];
        let value = args
            .get(i + 1)
            .ok_or_else(|| HarnessError::invalid("Missing server option value."))?;
        match flag.as_str() {
            "--source" => {
                config.source = infra_core::fs::canonicalize(value)?;
                explicit_source = true;
            }
            "--state-dir" => config.state = PathBuf::from(value),
            "--project" => {
                let project = infra_core::fs::canonicalize(value)?;
                config.projects.push(project.clone());
                explicit_projects.push(project);
            }
            "--port" => {
                config.port = value
                    .parse()
                    .map_err(|_| HarnessError::invalid("Invalid server port."))?
            }
            "--timeout" => {
                config.timeout = value
                    .parse()
                    .map_err(|_| HarnessError::invalid("Invalid server timeout."))?
            }
            "--claude-bin" | "--codex-bin" | "--cursor-bin" => {
                config
                    .providers
                    .insert(flag[2..flag.len() - 4].into(), value.clone());
            }
            "--session-concurrency" => {
                config.session_concurrency = value
                    .parse()
                    .map_err(|_| HarnessError::invalid("Invalid session concurrency (1–4)."))?;
            }
            _ => return Err(HarnessError::invalid("Unknown server option.")),
        }
        i += 2;
    }
    if !(1..=86400).contains(&config.timeout) {
        return Err(HarnessError::invalid("Invalid timeout (1–86400 seconds)."));
    }
    if !(1..=crate::session_scheduler::MAX_CONCURRENCY).contains(&config.session_concurrency) {
        return Err(HarnessError::invalid("Invalid session concurrency (1–4)."));
    }
    if config.projects.is_empty() {
        let current = std::env::current_dir()?;
        if infra_core::fs::canonicalize(&current)? != config.source {
            config.projects.push(current);
        }
    }
    Ok(ParsedOptions {
        command,
        config,
        explicit_source,
        explicit_projects,
    })
}
struct ParsedOptions {
    command: String,
    config: ServerConfig,
    explicit_source: bool,
    explicit_projects: Vec<PathBuf>,
}
pub fn metadata(state: &Path) -> Result<Option<Value>> {
    let path = state.join("server.json");
    match infra_core::fs::read_regular(&path) {
        Ok(bytes) => {
            let data = crate::strict_json::parse(&bytes, 65536)?;
            if data["port"].as_u64().is_none_or(|p| p == 0 || p > 65535)
                || !data["instance"].is_string()
                || !data["token"].is_string()
            {
                return Err(HarnessError::invalid("Invalid server metadata."));
            }
            Ok(Some(data))
        }
        Err(e) if e.kind == infra_core::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
pub async fn request(info: &Value, path: &str, body: Option<&Value>) -> Result<Value> {
    let port = info["port"]
        .as_u64()
        .filter(|p| *p > 0 && *p < 65536)
        .ok_or_else(|| HarnessError::invalid("Invalid server port."))?;
    let token = info["token"]
        .as_str()
        .ok_or_else(|| HarnessError::invalid("Missing server token."))?;
    if !token.is_ascii()
        || token.chars().any(|c| c.is_control())
        || path.chars().any(|c| c.is_control())
    {
        return Err(HarnessError::invalid("Invalid local request."));
    }
    let mut stream = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::net::TcpStream::connect(("127.0.0.1", port as u16)),
    )
    .await
    .map_err(|_| HarnessError::unavailable("Server did not answer."))??;
    let bytes = body.map(Value::to_string).unwrap_or_default();
    let method = if body.is_some() { "POST" } else { "GET" };
    let message = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Harness-Token: {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{bytes}",
        bytes.len()
    );
    stream.write_all(message.as_bytes()).await?;
    let mut response = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(120),
        stream.take(2 * 1024 * 1024 + 1).read_to_end(&mut response),
    )
    .await
    .map_err(|_| HarnessError::unavailable("Local request timed out."))??;
    if response.len() > 2 * 1024 * 1024 {
        return Err(HarnessError::unavailable(
            "Local response exceeds its limit.",
        ));
    }
    let end = response
        .windows(4)
        .position(|s| s == b"\r\n\r\n")
        .ok_or_else(|| HarnessError::unavailable("Invalid local HTTP response."))?;
    let header = std::str::from_utf8(&response[..end])
        .map_err(|_| HarnessError::unavailable("Invalid local HTTP response."))?;
    let status = header
        .lines()
        .next()
        .and_then(|s| s.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| HarnessError::unavailable("Invalid local HTTP status."))?;
    let value = crate::strict_json::parse(&response[end + 4..], 2 * 1024 * 1024)?;
    if !(200..300).contains(&status) {
        return Err(HarnessError {
            status,
            code: "local-server",
            message: value["error"]
                .as_str()
                .unwrap_or("The local server refused this operation.")
                .into(),
        });
    }
    Ok(value)
}
pub async fn running(state: &Path) -> Result<Option<Value>> {
    let Some(info) = metadata(state)? else {
        return Ok(None);
    };
    match request(&info, "/api/health", None).await {
        Ok(health) if health["ok"] == true && health["instance"] == info["instance"] => {
            Ok(Some(info))
        }
        _ => Ok(None),
    }
}
fn private_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(HarnessError::forbidden("State file must be regular."));
    }
    Ok(file)
}
const MAX_LOG_BYTES: u64 = 256 * 1024;

fn rotated_log_path(path: &Path) -> PathBuf {
    let mut raw = path.as_os_str().to_os_string();
    raw.push(".1");
    PathBuf::from(raw)
}

fn ensure_regular_or_absent(path: &Path, label: &str) -> Result<Option<std::fs::Metadata>> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            if !meta.file_type().is_file() {
                return Err(HarnessError::forbidden(label));
            }
            Ok(Some(meta))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

fn open_rotated_private_log(path: &Path, label: &str) -> Result<File> {
    if let Some(meta) = ensure_regular_or_absent(path, label)?
        && meta.len() > MAX_LOG_BYTES
    {
        let backup = rotated_log_path(path);
        ensure_regular_or_absent(&backup, label)?;
        std::fs::rename(path, &backup)?;
    }
    let file = private_file(path)?;
    if !file.metadata()?.is_file() {
        return Err(HarnessError::forbidden(label));
    }
    Ok(file)
}

async fn forward_projects(info: &Value, projects: &[PathBuf]) -> Result<Option<String>> {
    let mut last_id = None;
    for project in projects {
        let response = request(info, "/api/projects", Some(&json!({ "path": project }))).await?;
        if let Some(id) = response
            .get("project")
            .and_then(|value| value.get("id"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            let _ = request(
                info,
                &format!("/api/projects/{id}/select"),
                Some(&json!({})),
            )
            .await;
            last_id = Some(id.to_owned());
        }
    }
    Ok(last_id)
}

fn spawn(config: &ServerConfig) -> Result<()> {
    spawn_engine(config, &std::env::current_exe()?, false)
}
pub fn spawn_engine(config: &ServerConfig, executable: &Path, relaunch: bool) -> Result<()> {
    let log = open_rotated_private_log(
        &config.state.join("server.log"),
        "Server log must be regular.",
    )?;
    let mut command = engine_command(config, executable, relaunch);
    command
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(
            windows_sys::Win32::System::Threading::DETACHED_PROCESS
                | windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP,
        );
    }
    command.spawn()?;
    Ok(())
}
fn engine_command(
    config: &ServerConfig,
    executable: &Path,
    relaunch: bool,
) -> std::process::Command {
    let mut command = std::process::Command::new(executable);
    command
        .args([
            "serve",
            if relaunch { "relaunch" } else { "serve" },
            "--source",
        ])
        .arg(&config.source)
        .arg("--state-dir")
        .arg(&config.state)
        .arg("--port")
        .arg(config.port.to_string())
        .arg("--timeout")
        .arg(config.timeout.to_string())
        .arg("--session-concurrency")
        .arg(config.session_concurrency.to_string())
        .current_dir(&config.source);
    for project in &config.projects {
        command.arg("--project").arg(project);
    }
    for (provider, executable) in &config.providers {
        command.arg(format!("--{provider}-bin")).arg(executable);
    }
    if let Some(path) = &config.path_env {
        command.env("PATH", path);
    }
    command
}
pub async fn server(
    args: &[String],
    source: &Path,
    factory: Arc<dyn ServicesFactory>,
) -> Result<i32> {
    let parsed = parse_options(args, source)?;
    server_with_parsed(parsed, factory)
        .await
        .map(|(code, _)| code)
}
#[allow(dead_code)]
async fn server_with_config(
    command: &str,
    config: ServerConfig,
    factory: Arc<dyn ServicesFactory>,
) -> Result<i32> {
    server_with_parsed(
        ParsedOptions {
            command: command.to_owned(),
            config,
            explicit_source: false,
            explicit_projects: vec![],
        },
        factory,
    )
    .await
    .map(|(code, _)| code)
}
async fn server_with_parsed(
    mut parsed: ParsedOptions,
    factory: Arc<dyn ServicesFactory>,
) -> Result<(i32, Option<String>)> {
    infra_host::util::private_directory(&parsed.config.state)?;
    parsed.config.state = infra_core::fs::python_realpath(&parsed.config.state)?;
    let command = parsed.command.as_str();
    let mut config = parsed.config;
    match command {
        "serve" => {
            let lock = private_file(&config.state.join("server.lock"))?;
            lock.try_lock().map_err(|_| {
                HarnessError::conflict("A server already owns this state directory.")
            })?;
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", config.port)).await?;
            config.port = listener.local_addr()?.port();
            let (shutdown, _) = tokio::sync::watch::channel(false);
            let control = Arc::new(ServerControl {
                config: config.clone(),
                signal: shutdown.clone(),
            });
            let services = factory.create(&config, control).await?;
            let app = App::new(
                services.clone(),
                config.port,
                config.source.clone(),
                shutdown.clone(),
            )?;
            let info = json!({"pid":std::process::id(),"port":config.port,"instance":app.instance,"token":app.token});
            infra_core::fs::atomic_json(&config.state.join("server.json"), &info)?;
            println!("Harness: http://127.0.0.1:{}", config.port);
            let signals = tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                let _ = shutdown.send(true);
            });
            let result = serve_listener(listener, app).await;
            signals.abort();
            let closed = services.close().await;
            if metadata(&config.state)?.is_some_and(|v| v["instance"] == info["instance"]) {
                std::fs::remove_file(config.state.join("server.json"))?;
            }
            drop(lock);
            result?;
            closed?;
            Ok((0, None))
        }
        "status" => {
            if let Some(info) = running(&config.state).await? {
                println!("Running: http://127.0.0.1:{}", info["port"]);
                Ok((0, None))
            } else {
                println!("Stopped");
                Ok((1, None))
            }
        }
        "stop" => {
            if let Some(info) = running(&config.state).await? {
                request(&info, "/api/shutdown", Some(&json!({}))).await?;
                println!("Stop requested.");
            } else {
                println!("Already stopped");
            }
            Ok((0, None))
        }
        "update" => {
            let info = running(&config.state).await?.ok_or_else(|| {
                HarnessError::unavailable("Start the server before applying an update.")
            })?;
            let result = request(&info, "/api/app/update", Some(&json!({}))).await?;
            println!("{result}");
            Ok((0, None))
        }
        "start" | "relaunch" => {
            if command == "relaunch" {
                for _ in 0..450 {
                    if running(&config.state).await?.is_none() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
            if let Some(info) = running(&config.state).await? {
                let project_id = forward_projects(&info, &parsed.explicit_projects).await?;
                println!("Already running: http://127.0.0.1:{}", info["port"]);
                return Ok((0, project_id));
            }
            spawn(&config)?;
            for _ in 0..175 {
                if let Some(info) = running(&config.state).await? {
                    let project_id = forward_projects(&info, &parsed.explicit_projects).await?;
                    println!("Started: http://127.0.0.1:{}", info["port"]);
                    return Ok((0, project_id));
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Err(HarnessError::unavailable(format!(
                "Startup is still in progress. Inspect {}.",
                config.state.join("server.log").display()
            )))
        }
        _ => Err(HarnessError::invalid("Unknown server command.")),
    }
}
#[cfg(target_os = "macos")]
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
#[cfg(target_os = "macos")]
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
pub fn install(source: &Path) -> Result<Vec<PathBuf>> {
    install_engine(source, &std::env::current_exe()?)
}
pub fn install_engine(source: &Path, engine: &Path) -> Result<Vec<PathBuf>> {
    install_engine_with_path(source, engine, None)
}
fn install_engine_with_path(
    source: &Path,
    engine: &Path,
    path_override: Option<&str>,
) -> Result<Vec<PathBuf>> {
    // A missing late icon, native helper or engine is refused before any
    // application config/desktop mkdir, chmod, write or registry operation.
    let source = infra_core::fs::canonicalize(source)?;
    let mut inputs = publication::Inputs::admit(&source, engine)?;
    let config = infra_core::fs::absolute(&config_directory()?)?;
    let publication_paths = publication::install_paths(&config)?;
    let executable = inputs.executable.clone();
    let record = publication::installation_record_with_path(&source, &executable, path_override)?;
    #[cfg(target_os = "linux")]
    let (desktop_command, stop_command) = (
        publication::desktop_exec(&executable)?,
        publication::desktop_exec_with_args(&executable, "app stop")?,
    );
    #[cfg(windows)]
    let staged_shortcut = publication::stage_shortcut(&source, &config, &mut inputs)?;
    inputs.check()?;
    let roots = publication::install_roots(&config)?;
    let mut publisher = publication::Publisher::admit(publication_paths, roots)?;
    inputs.check()?;
    let mut paths = Vec::new();
    #[cfg(target_os = "linux")]
    {
        let data = infra_core::fs::absolute(
            &std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or(home()?.join(".local/share")),
        )?;
        let path = data.join("applications/ai-accelerator.desktop");

        let body = format!(
            "[Desktop Entry]\nType=Application\nName=AI Accelerator\nName[ru]=AI Акселератор\nComment=Work with native coding agents in your browser\nExec={desktop_command}\nIcon=ai-accelerator\nTerminal=false\nCategories=Development;\nStartupNotify=true\nActions=Stop;\n\n[Desktop Action Stop]\nName=Stop the accelerator server\nName[ru]=Остановить сервер акселератора\nExec={stop_command}\n"
        );
        let svg = data.join("icons/hicolor/scalable/apps/ai-accelerator.svg");
        publisher.write(&svg, inputs.svg(), 0o644)?;
        paths.push(svg);
        for &size in publication::PNG_SIZES {
            let target = data.join(format!(
                "icons/hicolor/{size}x{size}/apps/ai-accelerator.png"
            ));
            publisher.write(&target, inputs.icon(size)?, 0o644)?;
            paths.push(target);
        }
        // Preserve the canonical desktop trust/executable-entry contract.
        publisher.write(&path, body.as_bytes(), 0o755)?;
        paths.push(path);
        for command in ["update-desktop-database", "gtk-update-icon-cache"] {
            if let Some(executable) = crate::provider::find_executable(command) {
                let target = if command == "update-desktop-database" {
                    data.join("applications")
                } else {
                    data.join("icons/hicolor")
                };
                let _ = std::process::Command::new(executable).arg(target).status();
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let bundle = infra_core::fs::absolute(&home()?.join("Applications/AI Accelerator.app"))?;
        let contents = bundle.join("Contents");
        let launcher = contents.join("MacOS/accelerator");
        let body = format!(
            "#!/bin/sh\nexec {} app open\n",
            quote(&executable.to_string_lossy())
        );
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>local.ai-infrastructure.accelerator</string><key>CFBundleName</key><string>AI Accelerator</string><key>CFBundleExecutable</key><string>accelerator</string><key>CFBundleIconFile</key><string>ai-accelerator.icns</string><key>CFBundleVersion</key><string>{}</string></dict></plist>",
            xml(env!("CARGO_PKG_VERSION"))
        );
        let mut chunks = Vec::new();
        for (kind, size) in [
            ("icp4", 16),
            ("icp5", 32),
            ("icp6", 64),
            ("ic07", 128),
            ("ic08", 256),
            ("ic09", 512),
            ("ic10", 1024),
            ("ic11", 32),
            ("ic12", 64),
            ("ic13", 256),
            ("ic14", 512),
        ] {
            let image = inputs.icon(size)?;
            chunks.extend_from_slice(kind.as_bytes());
            chunks.extend_from_slice(&((image.len() + 8) as u32).to_be_bytes());
            chunks.extend_from_slice(image);
        }
        let mut icon = b"icns".to_vec();
        icon.extend_from_slice(&((chunks.len() + 8) as u32).to_be_bytes());
        icon.extend(chunks);
        publisher.write(&launcher, body.as_bytes(), 0o755)?;
        publisher.write(
            &contents.join("Resources/ai-accelerator.icns"),
            &icon,
            0o644,
        )?;
        publisher.write(&contents.join("Info.plist"), plist.as_bytes(), 0o644)?;
        paths.push(bundle);
    }
    #[cfg(windows)]
    {
        let appdata = infra_core::fs::absolute(
            &std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .unwrap_or(home()?.join("AppData/Roaming")),
        )?;
        let shortcut = appdata.join("Microsoft/Windows/Start Menu/Programs/AI Accelerator.lnk");
        let icon_path = config.join("ai-accelerator.ico");
        let icons = publication::PNG_SIZES
            .iter()
            .copied()
            .map(|size| inputs.icon(size).map(|image| (size, image)))
            .collect::<Result<Vec<_>>>()?;
        let mut icon = vec![0, 0, 1, 0, icons.len() as u8, 0];
        let mut offset = 6 + 16 * icons.len();
        for (size, image) in &icons {
            icon.extend_from_slice(&[
                if *size == 256 { 0 } else { *size as u8 },
                if *size == 256 { 0 } else { *size as u8 },
                0,
                0,
                1,
                0,
                32,
                0,
            ]);
            icon.extend_from_slice(&(image.len() as u32).to_le_bytes());
            icon.extend_from_slice(&(offset as u32).to_le_bytes());
            offset += image.len();
        }
        for (_, image) in icons {
            icon.extend_from_slice(image);
        }
        publisher.write(&icon_path, &icon, 0o644)?;
        publisher.write(&shortcut, &staged_shortcut, 0o644)?;
        windows_shortcut::register(&source, &icon_path, &executable)?;
        paths.extend([shortcut, icon_path]);
    }
    publisher.check()?;
    publisher.write(&config.join("app.json"), &record, 0o600)?;
    publisher.check()?;
    Ok(paths)
}
pub fn uninstall() -> Result<Vec<PathBuf>> {
    match publication::Uninstall::prepare()? {
        Some(plan) => plan.finish(publication::remove_registry),
        None => Ok(vec![]),
    }
}
const PATH_MARKER_BEGIN: &str = "__AI_ACCEL_PATH_BEGIN__";
const PATH_MARKER_END: &str = "__AI_ACCEL_PATH_END__";
const LOGIN_SHELL_TIMEOUT: Duration = Duration::from_secs(10);

fn extract_marked_path(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    let start = text.rfind(PATH_MARKER_BEGIN)?;
    let tail = &text[start + PATH_MARKER_BEGIN.len()..];
    let end = tail.find(PATH_MARKER_END)?;
    let path = tail[..end].trim_matches(['\r', '\n']);
    if path.is_empty() || path.contains('\0') {
        return None;
    }
    Some(path.to_owned())
}

#[cfg(unix)]
async fn probe_login_shell_path() -> Option<String> {
    let shell = std::env::var_os("SHELL").filter(|value| !value.is_empty())?;
    let name = Path::new(&shell)
        .file_name()
        .and_then(|part| part.to_str())
        .unwrap_or("");
    let probe = if name == "fish" {
        format!(
            "printf '%s' {PATH_MARKER_BEGIN}; string join : $PATH; printf '%s\\n' {PATH_MARKER_END}"
        )
    } else {
        format!("printf '%s%s%s\\n' {PATH_MARKER_BEGIN} \"$PATH\" {PATH_MARKER_END}")
    };
    let mut command = tokio::process::Command::new(&shell);
    command
        .args(["-ilc", &probe])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn().ok()?;
    let stdout = child.stdout.take()?;
    let run = async move {
        let mut buf = Vec::new();
        stdout
            .take(512 * 1024 + 1)
            .read_to_end(&mut buf)
            .await
            .ok()?;
        let status = child.wait().await.ok()?;
        if !status.success() || buf.len() > 512 * 1024 {
            return None;
        }
        extract_marked_path(&buf)
    };
    tokio::time::timeout(LOGIN_SHELL_TIMEOUT, run)
        .await
        .ok()
        .flatten()
}

#[cfg(not(unix))]
async fn probe_login_shell_path() -> Option<String> {
    None
}

fn collect_unique_paths<I>(segments: I) -> Option<std::ffi::OsString>
where
    I: IntoIterator<Item = PathBuf>,
{
    let mut merged = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for segment in segments {
        if !segment.as_os_str().is_empty() && seen.insert(segment.clone()) {
            merged.push(segment);
        }
    }
    if merged.is_empty() {
        return None;
    }
    std::env::join_paths(merged).ok()
}

fn merge_search_paths(
    preferred: Option<&str>,
    current: Option<std::ffi::OsString>,
) -> Option<std::ffi::OsString> {
    let preferred = preferred.filter(|value| !value.is_empty())?;
    collect_unique_paths(
        std::env::split_paths(preferred).chain(current.iter().flat_map(std::env::split_paths)),
    )
}

fn merge_current_then_fallback(
    current: Option<std::ffi::OsString>,
    fallback: Option<&str>,
) -> Option<std::ffi::OsString> {
    collect_unique_paths(
        current
            .iter()
            .flat_map(std::env::split_paths)
            .chain(fallback.into_iter().flat_map(std::env::split_paths)),
    )
}

fn read_cached_app_path() -> Option<String> {
    let directory = config_directory().ok()?;
    let record = crate::creator_files::object(&directory, "app.json", false)
        .ok()
        .flatten()?;
    record["path"]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn update_cached_app_path(probed: &str) -> Result<()> {
    let directory = config_directory()?;
    if !directory.is_dir() {
        return Ok(());
    }
    let Some(mut record) = crate::creator_files::object(&directory, "app.json", false)? else {
        return Ok(());
    };
    if record["path"].as_str() == Some(probed) {
        return Ok(());
    }
    record["path"] = json!(probed);
    let bytes = format!("{}\n", infra_core::domain::json::sorted_pretty(&record)?).into_bytes();
    if bytes.len() > 65536 {
        return Err(HarnessError::invalid(
            "Desktop installation record exceeds its read boundary.",
        ));
    }
    let mut files = infra_core::bank::publication::PublicationFiles::open_directory_root(
        &directory,
        infra_core::bank::publication::private::RootProtection::SelectedRoot,
    )?;
    let expected = files.read("app.json", 65536)?;
    let expected_mode = files.read_mode("app.json")?;
    files.write_expected_mode_checked(
        "app.json",
        &bytes,
        expected.as_deref(),
        expected_mode,
        0o600,
        65536,
    )?;
    files.check()?;
    Ok(())
}

fn resolve_open_path(probed: Option<&str>, cached: Option<&str>) -> Option<std::ffi::OsString> {
    if let Some(shell_path) = probed.filter(|value| !value.is_empty()) {
        let _ = update_cached_app_path(shell_path);
        return merge_search_paths(Some(shell_path), std::env::var_os("PATH"));
    }
    merge_search_paths(cached, std::env::var_os("PATH"))
}

fn canonical_or_raw(path: &Path) -> PathBuf {
    infra_core::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn rebind_moved_clone(
    source: &Path,
    path_override: Option<&str>,
    explicit_source: bool,
) -> Result<Vec<PathBuf>> {
    let Some((_, record)) = installed_application_record()? else {
        return Ok(vec![]);
    };
    let current_source = infra_core::fs::canonicalize(source)?;
    let current_exe = canonical_or_raw(&std::env::current_exe()?);
    let recorded_clone = record["clone"].as_str().map(PathBuf::from);
    let same_clone = recorded_clone
        .as_deref()
        .is_some_and(|old| canonical_or_raw(old) == current_source);
    let recorded_exe = record["executable"].as_str().map(PathBuf::from);
    let old_exe_valid = recorded_exe.as_deref().is_some_and(|path| {
        ensure_regular_or_absent(path, "Launcher executable invalid.")
            .ok()
            .flatten()
            .is_some()
    });
    let same_executable = recorded_exe
        .as_deref()
        .is_some_and(|old| canonical_or_raw(old) == current_exe);

    if same_clone && same_executable {
        return Ok(vec![]);
    }
    if !same_clone
        && !explicit_source
        && old_exe_valid
        && let Some(old_clone) = recorded_clone.as_deref()
        && resolve_clone_root(old_clone).ok().flatten().is_some()
    {
        return Ok(vec![]);
    }
    install_engine_with_path(&current_source, &current_exe, path_override)
}

fn append_launcher_log(message: &str) -> Result<()> {
    let directory = config_directory()?;
    infra_host::util::private_directory(&directory)?;
    let log_path = directory.join("launcher.log");
    let mut file = open_rotated_private_log(&log_path, "Launcher log must be regular.")?;
    std::io::Seek::seek(&mut file, std::io::SeekFrom::End(0))?;
    let line = format!(
        "[{}] {}\n",
        chrono::Utc::now().to_rfc3339(),
        message.trim().replace('\n', " ")
    );
    std::io::Write::write_all(&mut file, line.as_bytes())?;
    Ok(())
}

#[cfg(windows)]
fn encode_wide(value: &str) -> Vec<u16> {
    value
        .replace('\0', "")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
}

async fn notify_launch_failure(message: &str, path_env: Option<&std::ffi::OsStr>) {
    #[cfg(target_os = "linux")]
    {
        let mut command = tokio::process::Command::new("notify-send");
        command
            .args([
                "--urgency=critical",
                "--icon=ai-accelerator",
                "AI Accelerator",
                message,
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        if let Some(path) = path_env {
            command.env("PATH", path);
        }
        let _ = tokio::time::timeout(Duration::from_secs(3), command.status()).await;
    }
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "display notification \"{}\" with title \"AI Accelerator\"",
            message.replace('\\', "\\\\").replace('"', "\\\"")
        );
        let mut command = tokio::process::Command::new("osascript");
        command
            .args(["-e", &script])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        if let Some(path) = path_env {
            command.env("PATH", path);
        }
        let _ = tokio::time::timeout(Duration::from_secs(5), command.status()).await;
    }
    #[cfg(windows)]
    {
        let _ = path_env;
        if std::env::var_os("HARNESS_NO_GUI").is_none() {
            #[link(name = "user32")]
            unsafe extern "system" {
                fn MessageBoxW(
                    hwnd: windows_sys::Win32::Foundation::HWND,
                    text: *const u16,
                    caption: *const u16,
                    utype: u32,
                ) -> i32;
            }
            let caption = encode_wide("AI Accelerator");
            let text = encode_wide(message);
            unsafe {
                MessageBoxW(
                    std::ptr::null_mut(),
                    text.as_ptr(),
                    caption.as_ptr(),
                    0x0000_0010,
                );
            }
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    let _ = (message, path_env);
}

const BROWSER_OPEN_TIMEOUT: Duration = Duration::from_secs(10);

#[cfg(target_os = "linux")]
const BROWSER_OPENER: &str = "xdg-open";
#[cfg(target_os = "macos")]
const BROWSER_OPENER: &str = "open";
#[cfg(windows)]
const BROWSER_OPENER: &str = "rundll32.exe";

fn browser_launcher_available(path_env: Option<&std::ffi::OsStr>) -> bool {
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    {
        let Some(paths) = path_env
            .map(std::ffi::OsString::from)
            .or_else(|| std::env::var_os("PATH"))
        else {
            return false;
        };
        std::env::split_paths(&paths)
            .filter(|dir| !dir.as_os_str().is_empty())
            .map(|dir| dir.join(BROWSER_OPENER))
            .any(|candidate| candidate.is_file())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = path_env;
        false
    }
}

fn percent_encode_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn open_url(port: &Value, project_id: Option<&str>) -> String {
    match project_id.filter(|id| !id.is_empty()) {
        Some(id) => format!(
            "http://127.0.0.1:{port}/#/sessions?project={}",
            percent_encode_component(id)
        ),
        None => format!("http://127.0.0.1:{port}/"),
    }
}

async fn open_server(
    mut parsed: ParsedOptions,
    factory: Arc<dyn ServicesFactory>,
    path_env: Option<&std::ffi::OsStr>,
) -> Result<(ServerConfig, Value, Option<String>)> {
    parsed.config.path_env = path_env.map(std::ffi::OsStr::to_os_string);
    let selected = parsed.config.clone();
    let (_, project_id) = server_with_parsed(parsed, factory).await?;
    let info = running(&selected.state)
        .await?
        .ok_or_else(|| HarnessError::unavailable("The application server did not start."))?;
    Ok((selected, info, project_id))
}

async fn open_browser(url: &str, path_env: Option<&std::ffi::OsStr>) -> Result<()> {
    if let Some(message) = try_open_browser(url, path_env, BROWSER_OPEN_TIMEOUT).await {
        report_launch_error(&message, path_env).await;
        println!("Open {url} in your browser.");
    }
    Ok(())
}

async fn try_open_browser(
    url: &str,
    path_env: Option<&std::ffi::OsStr>,
    timeout: Duration,
) -> Option<String> {
    #[cfg(target_os = "linux")]
    let mut command = {
        let mut command = tokio::process::Command::new(BROWSER_OPENER);
        command.arg(url);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = tokio::process::Command::new(BROWSER_OPENER);
        command.arg(url);
        command
    };
    #[cfg(windows)]
    let mut command = {
        let mut command = tokio::process::Command::new(BROWSER_OPENER);
        command.args(["url.dll,FileProtocolHandler", url]);
        command
    };
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = (url, path_env, timeout);
        println!("Open {url} in your browser.");
        return None;
    }
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    {
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        if let Some(path) = path_env {
            command.env("PATH", path);
        }
        match command.spawn() {
            Ok(mut child) => {
                let stderr = child.stderr.take();
                match tokio::time::timeout(timeout, child.wait()).await {
                    Ok(Ok(status)) if status.success() => None,
                    Ok(Ok(status)) => {
                        let mut detail = String::new();
                        if let Some(err_stream) = stderr {
                            let mut buf = Vec::new();
                            let _ = tokio::time::timeout(
                                Duration::from_millis(150),
                                err_stream.take(4096).read_to_end(&mut buf),
                            )
                            .await;
                            let text = String::from_utf8_lossy(&buf).trim().to_owned();
                            if !text.is_empty() {
                                detail = format!(": {text}");
                            }
                        }
                        Some(format!(
                            "Could not open {url} in your browser ({BROWSER_OPENER} exited with {status}{detail})."
                        ))
                    }
                    Ok(Err(err)) => Some(format!(
                        "Could not open {url} in your browser ({BROWSER_OPENER}: {err})."
                    )),
                    Err(_) => Some(format!(
                        "Timed out waiting for {BROWSER_OPENER} to open {url}."
                    )),
                }
            }
            Err(err) => Some(format!(
                "Could not launch browser opener ({BROWSER_OPENER}) for {url}: {err}."
            )),
        }
    }
}

async fn resolve_running_open_path(
    cached: Option<&str>,
) -> (Option<std::ffi::OsString>, Option<String>) {
    let current = std::env::var_os("PATH");
    if browser_launcher_available(current.as_deref()) {
        // Caller's live PATH already locates the browser opener; append cached fallback paths after it.
        return (
            merge_current_then_fallback(current, cached),
            cached.map(str::to_owned),
        );
    }
    let cached_resolved = merge_search_paths(cached, current);
    if browser_launcher_available(cached_resolved.as_deref()) {
        return (cached_resolved, cached.map(str::to_owned));
    }
    let probed = probe_login_shell_path().await;
    let resolved = resolve_open_path(probed.as_deref(), cached);
    let record_path = probed.or_else(|| cached.map(str::to_owned));
    (resolved, record_path)
}

async fn report_launch_error(message: &str, path_env: Option<&std::ffi::OsStr>) {
    let _ = append_launcher_log(message);
    notify_launch_failure(message, path_env).await;
}

pub async fn app(args: &[String], source: &Path, factory: Arc<dyn ServicesFactory>) -> Result<i32> {
    let command = args.first().map(String::as_str).unwrap_or("open");
    match command {
        "install" => {
            for path in install(source)? {
                println!("{}", path.display());
            }
            Ok(0)
        }
        "uninstall" => {
            for path in uninstall()? {
                println!("{}", path.display());
            }
            println!("Projects, sessions and their memory are kept.");
            Ok(0)
        }
        "status" | "stop" | "update" => {
            let mut server_args = vec![command.into()];
            server_args.extend_from_slice(args.get(1..).unwrap_or(&[]));
            server(&server_args, source, factory).await
        }
        "open" => {
            let cached = read_cached_app_path();
            let cached_fallback = resolve_open_path(None, cached.as_deref());
            let mut server_args = vec!["start".into()];
            server_args.extend_from_slice(args.get(1..).unwrap_or(&[]));
            let mut parsed = match parse_options(&server_args, source) {
                Ok(parsed) => parsed,
                Err(error) => {
                    report_launch_error(&error.message, cached_fallback.as_deref()).await;
                    return Err(error);
                }
            };
            if let Err(error) = infra_host::util::private_directory(&parsed.config.state) {
                report_launch_error(&error.message, cached_fallback.as_deref()).await;
                return Err(error);
            }
            parsed.config.state = match infra_core::fs::python_realpath(&parsed.config.state)
                .map_err(HarnessError::from)
            {
                Ok(path) => path,
                Err(error) => {
                    report_launch_error(&error.message, cached_fallback.as_deref()).await;
                    return Err(error);
                }
            };

            let explicit_source = parsed.explicit_source;
            let running_info = match running(&parsed.config.state).await {
                Ok(info) => info,
                Err(error) => {
                    report_launch_error(&error.message, cached_fallback.as_deref()).await;
                    return Err(error);
                }
            };

            let (selected, info, project_id, resolved_path, record_path) = if let Some(info) =
                running_info
            {
                let project_id = match forward_projects(&info, &parsed.explicit_projects).await {
                    Ok(id) => id,
                    Err(error) => {
                        report_launch_error(&error.message, cached_fallback.as_deref()).await;
                        return Err(error);
                    }
                };
                let (resolved, record_path) = resolve_running_open_path(cached.as_deref()).await;
                (parsed.config, info, project_id, resolved, record_path)
            } else {
                let probed = probe_login_shell_path().await;
                let resolved = resolve_open_path(probed.as_deref(), cached.as_deref());
                let record_path = probed.or_else(|| cached.clone());
                match open_server(parsed, factory, resolved.as_deref()).await {
                    Ok((selected, info, project_id)) => {
                        (selected, info, project_id, resolved, record_path)
                    }
                    Err(error) => {
                        report_launch_error(&error.message, resolved.as_deref()).await;
                        return Err(error);
                    }
                }
            };

            if let Err(error) =
                rebind_moved_clone(&selected.source, record_path.as_deref(), explicit_source)
            {
                report_launch_error(&error.message, resolved_path.as_deref()).await;
                return Err(error);
            }
            let url = open_url(&info["port"], project_id.as_deref());
            open_browser(&url, resolved_path.as_deref()).await?;
            Ok(0)
        }
        _ => Err(HarnessError::invalid("Unknown application command.")),
    }
}

#[cfg(test)]
mod session_concurrency_tests {
    use super::*;
    fn parsed(root: &Path, extra: &[&str]) -> Result<ServerConfig> {
        let mut arguments = vec![
            "serve".to_string(),
            "--project".into(),
            root.to_string_lossy().into_owned(),
        ];
        arguments.extend(extra.iter().map(|s| s.to_string()));
        parse_options(&arguments, root).map(|parsed| parsed.config)
    }
    #[test]
    fn concurrency_is_explicit_bounded_and_preserved_in_background_and_relaunch() {
        let temporary = tempfile::tempdir().unwrap();
        let root = infra_core::fs::canonicalize(temporary.path()).unwrap();
        assert_eq!(parsed(&root, &[]).unwrap().session_concurrency, 1);
        for (arguments, expected) in [
            (vec!["--session-concurrency", "1"], 1),
            (vec!["--session-concurrency=2"], 2),
            (vec!["--session-concurrency", "4"], 4),
        ] {
            let config = parsed(&root, &arguments).unwrap();
            assert_eq!(config.session_concurrency, expected);
            for relaunch in [false, true] {
                let command = engine_command(&config, Path::new("fixture-engine"), relaunch);
                let args = command
                    .get_args()
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect::<Vec<_>>();
                assert_eq!(args[1], if relaunch { "relaunch" } else { "serve" });
                assert!(
                    args.windows(2)
                        .any(|pair| pair == ["--session-concurrency", &expected.to_string()])
                );
            }
        }
        for value in ["0", "5", "-1", "1.5", "true", "", "184467440737095516160"] {
            assert!(
                parsed(&root, &["--session-concurrency", value]).is_err(),
                "{value}"
            );
        }
        assert!(parsed(&root, &["--session-concurrency"]).is_err());
    }
    #[test]
    fn extract_marked_path_and_merge_paths_handle_greetings_and_dedup() {
        let output =
            b"Welcome to zsh!\n\x1b[32mprompt\x1b[0m\n__AI_ACCEL_PATH_BEGIN__/home/u/.local/bin:/usr/bin__AI_ACCEL_PATH_END__\r\nbye\n";
        assert_eq!(
            extract_marked_path(output).as_deref(),
            Some("/home/u/.local/bin:/usr/bin")
        );
        assert_eq!(extract_marked_path(b"no markers"), None);
        assert_eq!(
            extract_marked_path(b"__AI_ACCEL_PATH_BEGIN____AI_ACCEL_PATH_END__"),
            None
        );
        assert_eq!(
            extract_marked_path(b"__AI_ACCEL_PATH_BEGIN__/a\0/b__AI_ACCEL_PATH_END__"),
            None
        );
        // A search path is joined with the host's own separator, `:` or `;`.
        let joined = |parts: &[&str]| std::env::join_paths(parts).unwrap();
        let merged = merge_search_paths(
            joined(&["/home/u/.local/bin", "/usr/bin"]).to_str(),
            Some(joined(&["/usr/bin", "/bin"])),
        )
        .unwrap();
        let parts = std::env::split_paths(&merged)
            .map(|p| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(parts, vec!["/home/u/.local/bin", "/usr/bin", "/bin"]);
        assert_eq!(
            merge_search_paths(Some(""), Some(std::ffi::OsString::from("/bin"))),
            None
        );
    }

    #[test]
    fn rotated_private_log_moves_oversized_log_to_dot1_and_refuses_symlinks() {
        let temporary = tempfile::tempdir().unwrap();
        let root = infra_core::fs::canonicalize(temporary.path()).unwrap();
        let log = root.join("server.log");
        let rotated = rotated_log_path(&log);
        assert_eq!(rotated, root.join("server.log.1"));

        let oversized = vec![b'a'; (MAX_LOG_BYTES as usize) + 64];
        std::fs::write(&log, &oversized).unwrap();

        let opened = open_rotated_private_log(&log, "Server log must be regular.").unwrap();
        drop(opened);
        assert!(rotated.is_file());
        assert_eq!(std::fs::read(&rotated).unwrap().len(), oversized.len());
        assert_eq!(std::fs::metadata(&log).unwrap().len(), 0);

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let outside = root.join("outside-target");
            std::fs::write(&outside, b"secret").unwrap();
            let linked = root.join("launcher.log");
            symlink(&outside, &linked).unwrap();
            assert!(open_rotated_private_log(&linked, "Launcher log must be regular.").is_err());
            assert_eq!(std::fs::read(&outside).unwrap(), b"secret");
        }
    }

    #[test]
    fn open_url_formats_root_or_project_deep_link() {
        assert_eq!(open_url(&json!(8766), None), "http://127.0.0.1:8766/");
        assert_eq!(open_url(&json!(8766), Some("")), "http://127.0.0.1:8766/");
        assert_eq!(
            open_url(&json!(8766), Some("proj/1")),
            "http://127.0.0.1:8766/#/sessions?project=proj%2F1"
        );
    }

    #[tokio::test]
    async fn open_browser_timeout_terminates_hung_opener() {
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::PermissionsExt;
            let temporary = tempfile::tempdir().unwrap();
            let root = infra_core::fs::canonicalize(temporary.path()).unwrap();
            let bin_dir = root.join("bin");
            std::fs::create_dir(&bin_dir).unwrap();
            let opener = bin_dir.join(BROWSER_OPENER);
            std::fs::write(&opener, "#!/bin/sh\nwhile :; do :; done\n").unwrap();
            std::fs::set_permissions(&opener, std::fs::Permissions::from_mode(0o755)).unwrap();
            let start = std::time::Instant::now();
            let err = try_open_browser(
                "http://127.0.0.1:8766/",
                Some(bin_dir.as_os_str()),
                Duration::from_millis(120),
            )
            .await;
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "try_open_browser hung for {:?}",
                start.elapsed()
            );
            assert!(
                err.as_deref()
                    .is_some_and(|msg| msg.contains("Timed out waiting")),
                "expected timeout message, got {err:?}"
            );
        }
    }
}
