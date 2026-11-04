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
    pub projects: Vec<PathBuf>,
    pub providers: BTreeMap<String, String>,
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
impl crate::updates::Restart for ServerControl {
    fn refresh_application(&self, engine: &Path, expected_sha256: &str) -> Result<Vec<String>> {
        let directory = config_directory()?;
        if !directory.is_dir() {
            return Ok(vec![]);
        }
        let Some(record) = crate::creator_files::object(&directory, "app.json", false)? else {
            return Ok(vec![]);
        };
        if record["clone"] != serde_json::to_value(&self.config.source)?
            || record["installed"]
                .as_array()
                .is_none_or(|paths| paths.is_empty())
        {
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
fn options(arguments: &[String], source: &Path) -> Result<(String, ServerConfig)> {
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
        projects: vec![],
        providers: BTreeMap::new(),
    };
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
            "--source" => config.source = infra_core::fs::canonicalize(value)?,
            "--state-dir" => config.state = PathBuf::from(value),
            "--project" => config.projects.push(infra_core::fs::canonicalize(value)?),
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
            _ => return Err(HarnessError::invalid("Unknown server option.")),
        }
        i += 2;
    }
    if !(1..=86400).contains(&config.timeout) {
        return Err(HarnessError::invalid("Invalid timeout (1–86400 seconds)."));
    }
    if config.projects.is_empty() {
        let current = std::env::current_dir()?;
        if infra_core::fs::canonicalize(&current)? != config.source {
            config.projects.push(current);
        }
    }
    Ok((command, config))
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
fn spawn(config: &ServerConfig) -> Result<()> {
    spawn_engine(config, &std::env::current_exe()?, false)
}
pub fn spawn_engine(config: &ServerConfig, executable: &Path, relaunch: bool) -> Result<()> {
    let log = private_file(&config.state.join("server.log"))?;
    if log.metadata()?.len() > 256 * 1024 {
        log.set_len(0)?;
    }
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
        .current_dir(&config.source);
    for project in &config.projects {
        command.arg("--project").arg(project);
    }
    for (provider, executable) in &config.providers {
        command.arg(format!("--{provider}-bin")).arg(executable);
    }
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
pub async fn server(
    args: &[String],
    source: &Path,
    factory: Arc<dyn ServicesFactory>,
) -> Result<i32> {
    let (command, mut config) = options(args, source)?;
    infra_host::util::private_directory(&config.state)?;
    config.state = infra_core::fs::python_realpath(&config.state)?;
    match command.as_str() {
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
            Ok(0)
        }
        "status" => {
            if let Some(info) = running(&config.state).await? {
                println!("Running: http://127.0.0.1:{}", info["port"]);
                Ok(0)
            } else {
                println!("Stopped");
                Ok(1)
            }
        }
        "stop" => {
            if let Some(info) = running(&config.state).await? {
                request(&info, "/api/shutdown", Some(&json!({}))).await?;
                println!("Stop requested.");
            } else {
                println!("Already stopped");
            }
            Ok(0)
        }
        "update" => {
            let info = running(&config.state).await?.ok_or_else(|| {
                HarnessError::unavailable("Start the server before applying an update.")
            })?;
            let result = request(&info, "/api/app/update", Some(&json!({}))).await?;
            println!("{result}");
            Ok(0)
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
                println!("Already running: http://127.0.0.1:{}", info["port"]);
                return Ok(0);
            }
            spawn(&config)?;
            for _ in 0..175 {
                if let Some(info) = running(&config.state).await? {
                    println!("Started: http://127.0.0.1:{}", info["port"]);
                    return Ok(0);
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
    // A missing late icon, native helper or engine is refused before any
    // application config/desktop mkdir, chmod, write or registry operation.
    let source = infra_core::fs::canonicalize(source)?;
    let mut inputs = publication::Inputs::admit(&source, engine)?;
    let config = infra_core::fs::absolute(&config_directory()?)?;
    let publication_paths = publication::install_paths(&config)?;
    let executable = inputs.executable.clone();
    let record = publication::installation_record(&source, &executable)?;
    #[cfg(target_os = "linux")]
    let desktop_command = publication::desktop_exec(&executable)?;
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
            "[Desktop Entry]\nType=Application\nName=AI Accelerator\nName[ru]=AI Акселератор\nComment=Work with native coding agents in your browser\nExec={desktop_command}\nIcon=ai-accelerator\nTerminal=false\nCategories=Development;\nStartupNotify=true\n"
        );
        let svg = data.join("icons/hicolor/scalable/apps/ai-accelerator.svg");
        publisher.write(&svg, inputs.svg(), 0o644)?;
        paths.push(svg);
        for size in [16, 22, 24, 32, 48, 64, 128, 256, 512] {
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
        let icons = [16, 24, 32, 48, 64, 128, 256]
            .into_iter()
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
            let mut server_args = vec!["start".into()];
            server_args.extend_from_slice(args.get(1..).unwrap_or(&[]));
            let (_, selected) = options(&server_args, source)?;
            server(&server_args, source, factory).await?;
            let info = running(&selected.state).await?.ok_or_else(|| {
                HarnessError::unavailable("The application server did not start.")
            })?;
            let url = format!("http://127.0.0.1:{}/", info["port"]);
            #[cfg(target_os = "linux")]
            let status = tokio::process::Command::new("xdg-open")
                .arg(&url)
                .status()
                .await?;
            #[cfg(target_os = "macos")]
            let status = tokio::process::Command::new("open")
                .arg(&url)
                .status()
                .await?;
            #[cfg(windows)]
            let status = tokio::process::Command::new("rundll32.exe")
                .args(["url.dll,FileProtocolHandler", &url])
                .status()
                .await?;
            if !status.success() {
                println!("Open {url} in your browser.");
            }
            Ok(0)
        }
        _ => Err(HarnessError::invalid("Unknown application command.")),
    }
}
