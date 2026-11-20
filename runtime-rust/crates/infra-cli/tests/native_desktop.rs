//! Actual desktop CLI effects are confined to child-only HOME/XDG directories.
#![cfg(target_os = "linux")]
#[path = "support/executable.rs"]
mod executable;
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};
struct PrivateDirectory(PathBuf);
impl PrivateDirectory {
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Sandbox {
    root: PrivateDirectory,
    source: PathBuf,
    home: PathBuf,
    data: PathBuf,
    config: PathBuf,
    binary: PathBuf,
}
impl Sandbox {
    fn new() -> Self {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "infra-desktop-{}-{nonce}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let base = infra_core::fs::canonicalize(&path).unwrap();
        let root = PrivateDirectory(base.clone());
        let source = base.join("source fixture");
        let home = base.join("private home");
        let data = base.join("redirected desktop data");
        let config = base.join("redirected config");
        fs::create_dir_all(source.join("harness/web/icons")).unwrap();
        fs::create_dir_all(source.join("Infrastructure-Creator")).unwrap();
        fs::write(source.join("harness/web/index.html"), b"source-root marker").unwrap();
        fs::write(
            source.join("Infrastructure-Creator/AGENTS.md"),
            b"source-root marker",
        )
        .unwrap();
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .unwrap();
        for name in [
            "ai-accelerator.svg",
            "ai-accelerator-16.png",
            "ai-accelerator-22.png",
            "ai-accelerator-24.png",
            "ai-accelerator-32.png",
            "ai-accelerator-48.png",
            "ai-accelerator-64.png",
            "ai-accelerator-128.png",
            "ai-accelerator-256.png",
            "ai-accelerator-512.png",
            "ai-accelerator-1024.png",
        ] {
            fs::copy(
                repository.join("harness/web/icons").join(name),
                source.join("harness/web/icons").join(name),
            )
            .unwrap();
        }
        fs::create_dir(&home).unwrap();
        fs::create_dir(&data).unwrap();
        fs::create_dir(&config).unwrap();
        let binary = base.join("infra-engine");
        executable::copy(env!("CARGO_BIN_EXE_infra-engine"), &binary);
        Self {
            root,
            source,
            home,
            data,
            config,
            binary,
        }
    }
    fn record(&self) -> PathBuf {
        self.config.join("ai-infrastructure-harness/app.json")
    }
    fn entry(&self) -> PathBuf {
        self.data.join("applications/ai-accelerator.desktop")
    }
    fn svg(&self) -> PathBuf {
        self.data
            .join("icons/hicolor/scalable/apps/ai-accelerator.svg")
    }
    fn png(&self, size: u32) -> PathBuf {
        self.data.join(format!(
            "icons/hicolor/{size}x{size}/apps/ai-accelerator.png"
        ))
    }
    fn write(&self, path: &Path, bytes: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    fn legacy_record(&self) {
        self.write(
            &self.record(),
            serde_json::to_vec(&json!({
                "clone":self.source,"path":"","installed_at":"2026-01-01T00:00:00Z"
            }))
            .unwrap()
            .as_slice(),
        );
    }
    fn native_record(&self, paths: Vec<PathBuf>) {
        self.write(
            &self.record(),
            serde_json::to_vec(&json!({
                "clone":self.source,"path":"","executable":self.binary,"installed":paths
            }))
            .unwrap()
            .as_slice(),
        );
    }
    fn run(&self, action: &str) -> Output {
        self.run_with_path(action, "")
    }
    fn run_with_path(&self, action: &str, path: &str) -> Output {
        Command::new(&self.binary)
            .args(["app", action, "--source"])
            .arg(&self.source)
            .current_dir(&self.home)
            .env("HOME", &self.home)
            .env("XDG_DATA_HOME", &self.data)
            .env("XDG_CONFIG_HOME", &self.config)
            .env("XDG_STATE_HOME", self.root.path().join("private state"))
            .env("CODEX_HOME", self.root.path().join("private codex"))
            .env("PATH", path)
            .env_remove("INFRA_SOURCE_ROOT")
            .env_remove("ACCELERATOR_HOME")
            .env_remove("ACCELERATOR_STATE_DIR")
            .env_remove("ACCELERATOR_PROJECT_DIR")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
    fn assert_ok(&self, result: &Output) {
        assert!(
            result.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
#[derive(Debug, PartialEq, Eq)]
struct Image {
    kind: u8,
    bytes: Vec<u8>,
    mode: u32,
}
fn tree(path: &Path) -> BTreeMap<PathBuf, Image> {
    use std::os::unix::fs::PermissionsExt;
    fn walk(root: &Path, path: &Path, result: &mut BTreeMap<PathBuf, Image>) {
        let image = fs::symlink_metadata(path).unwrap();
        let (kind, bytes) = if image.file_type().is_symlink() {
            (
                2,
                fs::read_link(path)
                    .unwrap()
                    .as_os_str()
                    .as_encoded_bytes()
                    .to_vec(),
            )
        } else if image.is_file() {
            (1, fs::read(path).unwrap())
        } else {
            (0, Vec::new())
        };
        result.insert(
            path.strip_prefix(root).unwrap().to_path_buf(),
            Image {
                kind,
                bytes,
                mode: image.permissions().mode() & 0o777,
            },
        );
        if image.is_dir() {
            for child in fs::read_dir(path).unwrap() {
                walk(root, &child.unwrap().path(), result);
            }
        }
    }
    let mut result = BTreeMap::new();
    if path.exists() {
        walk(path, path, &mut result);
    }
    result
}

#[test]
fn missing_or_linked_late_icon_refuses_before_any_desktop_or_config_effect() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let fixture = Sandbox::new();
    fixture.legacy_record();
    fixture.write(&fixture.entry(), b"existing desktop entry");
    fixture.write(&fixture.png(16), b"existing icon");
    fs::set_permissions(
        fixture.record().parent().unwrap(),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    fs::set_permissions(fixture.record(), fs::Permissions::from_mode(0o640)).unwrap();
    let before = (
        tree(&fixture.home),
        tree(&fixture.data),
        tree(&fixture.config),
    );
    let late = fixture
        .source
        .join("harness/web/icons/ai-accelerator-512.png");
    fs::remove_file(&late).unwrap();
    assert!(!fixture.run("install").status.success());
    assert_eq!(
        (
            tree(&fixture.home),
            tree(&fixture.data),
            tree(&fixture.config)
        ),
        before
    );
    let sentinel = fixture.root.path().join("not an icon");
    fs::write(&sentinel, b"outside input must not be followed").unwrap();
    symlink(&sentinel, &late).unwrap();
    assert!(!fixture.run("install").status.success());
    assert_eq!(
        (
            tree(&fixture.home),
            tree(&fixture.data),
            tree(&fixture.config)
        ),
        before
    );
    assert_eq!(
        fs::read(sentinel).unwrap(),
        b"outside input must not be followed"
    );
}

#[test]
fn legacy_uninstall_uses_redirected_roster_preserves_sentinels_and_is_retryable() {
    let fixture = Sandbox::new();
    fixture.legacy_record();
    fixture.write(&fixture.entry(), b"legacy desktop");
    fixture.write(&fixture.svg(), b"legacy SVG");
    fixture.write(&fixture.png(32), b"legacy PNG");
    let sentinel = fixture.data.join("icons/hicolor/32x32/apps/unrelated.png");
    fixture.write(&sentinel, b"unrelated icon");
    fixture.assert_ok(&fixture.run("uninstall"));
    assert!(!fixture.entry().exists());
    assert!(!fixture.svg().exists());
    assert!(!fixture.png(32).exists());
    assert!(!fixture.record().exists());
    assert_eq!(fs::read(&sentinel).unwrap(), b"unrelated icon");
    fixture.assert_ok(&fixture.run("uninstall"));
    assert_eq!(fs::read(&sentinel).unwrap(), b"unrelated icon");
}

#[test]
fn whole_record_validation_precedes_the_first_owned_target_deletion() {
    let mut fixture = Sandbox::new();
    fixture.data = fixture.home.join(".local/share");
    fs::create_dir_all(&fixture.data).unwrap();
    fixture.write(&fixture.entry(), b"owned desktop image");
    let foreign = fixture.root.path().join("unrelated sentinel");
    fs::write(&foreign, b"preserve foreign file").unwrap();
    fixture.native_record(vec![fixture.entry(), foreign.clone()]);
    let before = (tree(&fixture.data), tree(&fixture.config));
    assert!(!fixture.run("uninstall").status.success());
    assert_eq!((tree(&fixture.data), tree(&fixture.config)), before);
    assert_eq!(fs::read(&foreign).unwrap(), b"preserve foreign file");
}

#[test]
fn late_linked_target_is_refused_before_removing_an_earlier_owned_target() {
    use std::os::unix::fs::symlink;
    let mut fixture = Sandbox::new();
    fixture.data = fixture.home.join(".local/share");
    fs::create_dir_all(&fixture.data).unwrap();
    let foreign = fixture.root.path().join("unrelated sentinel");
    fs::write(&foreign, b"preserve foreign bytes").unwrap();
    fixture.native_record(vec![fixture.entry(), fixture.png(512)]);
    fixture.write(&fixture.entry(), b"owned desktop image");
    let linked = fixture.png(512);
    fs::create_dir_all(linked.parent().unwrap()).unwrap();
    symlink(&foreign, &linked).unwrap();
    let before = (tree(&fixture.data), tree(&fixture.config));
    assert!(!fixture.run("uninstall").status.success());
    assert_eq!((tree(&fixture.data), tree(&fixture.config)), before);
    assert_eq!(fs::read(foreign).unwrap(), b"preserve foreign bytes");
}

#[test]
fn native_uninstall_accepts_already_missing_owned_targets() {
    let mut fixture = Sandbox::new();
    fixture.data = fixture.home.join(".local/share");
    fs::create_dir_all(&fixture.data).unwrap();
    fixture.write(&fixture.entry(), b"remaining desktop image");
    fixture.native_record(vec![fixture.entry(), fixture.png(16)]);
    fixture.assert_ok(&fixture.run("uninstall"));
    assert!(!fixture.entry().exists());
    assert!(!fixture.record().exists());
}

#[test]
fn a_record_larger_than_the_uninstall_read_boundary_is_refused_before_publication() {
    let fixture = Sandbox::new();
    let before = (
        tree(&fixture.home),
        tree(&fixture.data),
        tree(&fixture.config),
    );
    assert!(
        !fixture
            .run_with_path("install", &"p".repeat(70_000))
            .status
            .success()
    );
    assert_eq!(
        (
            tree(&fixture.home),
            tree(&fixture.data),
            tree(&fixture.config)
        ),
        before
    );
    assert!(!fixture.record().exists());
}

#[test]
fn an_xdg_config_parent_named_attached_keeps_its_existing_mode() {
    use std::os::unix::fs::PermissionsExt;
    let mut fixture = Sandbox::new();
    fixture.config = fixture.root.path().join("attached");
    fs::create_dir(&fixture.config).unwrap();
    fs::set_permissions(&fixture.config, fs::Permissions::from_mode(0o775)).unwrap();
    fs::set_permissions(&fixture.data, fs::Permissions::from_mode(0o755)).unwrap();
    fixture.assert_ok(&fixture.run("install"));
    assert_eq!(
        fs::symlink_metadata(&fixture.config)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o775
    );
    assert_eq!(
        fs::symlink_metadata(fixture.record().parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::symlink_metadata(&fixture.data)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    assert!(fixture.entry().is_file());
    fixture.assert_ok(&fixture.run("uninstall"));
}

#[test]
fn a_missing_literal_unix_xdg_base_uses_its_exact_native_name() {
    let mut fixture = Sandbox::new();
    fixture.data = fixture.home.join(r"xdg\literal data");
    assert!(!fixture.data.exists());
    fixture.assert_ok(&fixture.run("install"));
    assert!(fixture.data.is_dir());
    assert!(fixture.entry().is_file());
    assert!(fixture.record().is_file());
    assert!(
        !fixture.home.join("xdg").exists(),
        "native base was split into an alternate tree"
    );
    fixture.assert_ok(&fixture.run("uninstall"));
    assert!(!fixture.entry().exists());
    assert!(!fixture.record().exists());
    assert!(!fixture.home.join("xdg").exists());
}

#[test]
fn native_desktop_exec_has_one_key_and_canonical_two_layer_path_escaping() {
    let mut fixture = Sandbox::new();
    let directory = fixture.root.path().join("trusted\nsecond\nliteral $`\\\t");
    fs::create_dir(&directory).unwrap();
    let engine = directory.join("infra-engine");
    executable::copy(&fixture.binary, &engine);
    fixture.binary = engine;
    fixture.assert_ok(&fixture.run("install"));
    let desktop = fs::read_to_string(fixture.entry()).unwrap();
    let exec = desktop
        .lines()
        .filter(|line| line.starts_with("Exec="))
        .collect::<Vec<_>>();
    assert_eq!(
        exec.len(),
        2,
        "native executable path injected unexpected Desktop Entry keys: {desktop}"
    );
    assert!(desktop.contains("\nActions=Stop;\n"));
    assert!(desktop.contains("\n[Desktop Action Stop]\nName=Stop the accelerator server\n"));
    assert!(desktop.contains("\nName[ru]=Остановить сервер акселератора\n"));
    for (index, suffix) in ["\" app open", "\" app stop"].into_iter().enumerate() {
        assert!(exec[index].starts_with("Exec=\""));
        assert!(exec[index].contains(r"trusted\nsecond\nliteral "));
        // Independent canonical vector: two slashes before $/backtick, four for
        // the literal backslash, then the key-level tab escape.
        assert!(exec[index].contains(r"literal \\$\\`\\\\\t/infra-engine"));
        assert!(exec[index].ends_with(suffix));
    }
    // Never open or execute the generated desktop entry.
    fixture.assert_ok(&fixture.run("uninstall"));

    // '=' is forbidden in an Exec executable path; other control bytes without
    // a supported string-value escape are refused before any publication.
    for name in ["blocked=exec", "blocked\u{1b}control"] {
        let directory = fixture.root.path().join(name);
        fs::create_dir(&directory).unwrap();
        let engine = directory.join("infra-engine");
        executable::copy(&fixture.binary, &engine);
        fixture.binary = engine;
        let before = (
            tree(&fixture.home),
            tree(&fixture.data),
            tree(&fixture.config),
        );
        let outcome = fixture.run("install");
        assert!(!outcome.status.success());
        assert_eq!(
            (
                tree(&fixture.home),
                tree(&fixture.data),
                tree(&fixture.config)
            ),
            before
        );
    }
}

fn write_executable_script(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

struct MockHealthServer {
    port: u16,
    shutdown_requested: std::sync::Arc<std::sync::atomic::AtomicBool>,
    forwarded_requests: std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl MockHealthServer {
    fn start(state_dir: &Path, instance: &str, token: &str) -> Self {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        };
        use std::time::Duration;

        {
            use std::os::unix::fs::PermissionsExt;
            fs::create_dir_all(state_dir).unwrap();
            fs::set_permissions(state_dir, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();

        let server_json = state_dir.join("server.json");
        fs::write(
            &server_json,
            serde_json::to_vec(&json!({
                "pid": std::process::id(),
                "port": port,
                "instance": instance,
                "token": token,
            }))
            .unwrap(),
        )
        .unwrap();

        let shutdown_requested = Arc::new(AtomicBool::new(false));
        let forwarded_requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let shutdown_flag = Arc::clone(&shutdown_requested);
        let requests_log = Arc::clone(&forwarded_requests);
        let stop_flag = Arc::clone(&stop);
        let instance_owned = instance.to_string();
        let expected_token = token.to_string();

        let handle = std::thread::spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let mut buf = [0u8; 4096];
                        let n = stream.read(&mut buf).unwrap_or(0);
                        let req = String::from_utf8_lossy(&buf[..n]);
                        let first_line = req.lines().next().unwrap_or("");
                        let body_text = req
                            .split_once("\r\n\r\n")
                            .map(|(_, body)| body.trim())
                            .unwrap_or("");
                        let has_token = req.lines().any(|line| {
                            line.to_ascii_lowercase()
                                == format!("x-harness-token: {expected_token}")
                        });
                        let body = if !has_token {
                            r#"{"error":"unauthorized"}"#.to_string()
                        } else if first_line.starts_with("GET /api/health ") {
                            json!({"ok": true, "instance": instance_owned}).to_string()
                        } else if first_line.starts_with("POST /api/shutdown ") {
                            shutdown_flag.store(true, Ordering::SeqCst);
                            "{}".to_string()
                        } else if first_line.starts_with("POST /api/projects ") {
                            requests_log
                                .lock()
                                .unwrap()
                                .push(("POST /api/projects".to_string(), body_text.to_string()));
                            json!({"project": {"id": "proj-e2e-forwarded"}}).to_string()
                        } else if first_line.starts_with("POST /api/projects/") {
                            requests_log.lock().unwrap().push((
                                first_line
                                    .split_whitespace()
                                    .take(2)
                                    .collect::<Vec<_>>()
                                    .join(" "),
                                body_text.to_string(),
                            ));
                            "{}".to_string()
                        } else {
                            "{}".to_string()
                        };
                        let status = if has_token {
                            "200 OK"
                        } else {
                            "401 Unauthorized"
                        };
                        let reply = format!(
                            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = stream.write_all(reply.as_bytes());
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            port,
            shutdown_requested,
            forwarded_requests,
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for MockHealthServer {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[test]
fn linux_install_preflights_icons_up_to_512_without_requiring_1024() {
    let fixture = Sandbox::new();
    fs::remove_file(
        fixture
            .source
            .join("harness/web/icons/ai-accelerator-1024.png"),
    )
    .unwrap();
    fixture.assert_ok(&fixture.run("install"));
    assert!(fixture.entry().is_file());
    assert!(fixture.png(512).is_file());
    assert!(!fixture.png(1024).exists());
    fixture.assert_ok(&fixture.run("uninstall"));
}

#[test]
fn app_status_and_stop_report_stopped_and_roundtrip_running_server() {
    let fixture = Sandbox::new();
    let stopped_status = fixture.run("status");
    assert!(!stopped_status.status.success());
    assert_eq!(
        String::from_utf8_lossy(&stopped_status.stdout).trim(),
        "Stopped"
    );

    let already_stopped = fixture.run("stop");
    fixture.assert_ok(&already_stopped);
    assert_eq!(
        String::from_utf8_lossy(&already_stopped.stdout).trim(),
        "Already stopped"
    );

    let state_dir = fixture
        .root
        .path()
        .join("private state/ai-infrastructure-harness");
    let server = MockHealthServer::start(&state_dir, "inst-1", "tok-123");

    let running_status = fixture.run("status");
    fixture.assert_ok(&running_status);
    assert_eq!(
        String::from_utf8_lossy(&running_status.stdout).trim(),
        format!("Running: http://127.0.0.1:{}", server.port)
    );

    let stopped = fixture.run("stop");
    fixture.assert_ok(&stopped);
    assert_eq!(
        String::from_utf8_lossy(&stopped.stdout).trim(),
        "Stop requested."
    );
    assert!(
        server
            .shutdown_requested
            .load(std::sync::atomic::Ordering::SeqCst)
    );
}

#[test]
fn app_open_probes_shell_path_rebinds_moved_clone_and_preserves_existing_first_clone() {
    let fixture = Sandbox::new();
    let state_dir = fixture
        .root
        .path()
        .join("private state/ai-infrastructure-harness");
    let server = MockHealthServer::start(&state_dir, "inst-open", "tok-open");

    let clone1 = fixture.root.path().join("clone1");
    let clone2 = fixture.root.path().join("clone2");
    for clone in [&clone1, &clone2] {
        fs::create_dir_all(clone.join("harness/web/icons")).unwrap();
        fs::create_dir_all(clone.join("Infrastructure-Creator")).unwrap();
        fs::copy(
            fixture.source.join("harness/web/index.html"),
            clone.join("harness/web/index.html"),
        )
        .unwrap();
        fs::copy(
            fixture.source.join("Infrastructure-Creator/AGENTS.md"),
            clone.join("Infrastructure-Creator/AGENTS.md"),
        )
        .unwrap();
        for name in [
            "ai-accelerator.svg",
            "ai-accelerator-16.png",
            "ai-accelerator-22.png",
            "ai-accelerator-24.png",
            "ai-accelerator-32.png",
            "ai-accelerator-48.png",
            "ai-accelerator-64.png",
            "ai-accelerator-128.png",
            "ai-accelerator-256.png",
            "ai-accelerator-512.png",
        ] {
            fs::copy(
                fixture.source.join("harness/web/icons").join(name),
                clone.join("harness/web/icons").join(name),
            )
            .unwrap();
        }
    }
    let bin1 = clone1.join("bin/infra-engine");
    let bin2 = clone2.join("bin/infra-engine");
    fs::create_dir_all(bin1.parent().unwrap()).unwrap();
    fs::create_dir_all(bin2.parent().unwrap()).unwrap();
    executable::copy(&fixture.binary, &bin1);
    executable::copy(&fixture.binary, &bin2);

    // Install clone1 with an empty PATH so fallback cache starts empty.
    let install_out = Command::new(&bin1)
        .args(["app", "install"])
        .current_dir(&clone1)
        .env("HOME", &fixture.home)
        .env("XDG_DATA_HOME", &fixture.data)
        .env("XDG_CONFIG_HOME", &fixture.config)
        .env("XDG_STATE_HOME", fixture.root.path().join("private state"))
        .env("PATH", "")
        .env_remove("ACCELERATOR_HOME")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    fixture.assert_ok(&install_out);

    let shell_bin = fixture.root.path().join("shell-extra-bin");
    let opened_log = fixture.root.path().join("opened-url.txt");
    write_executable_script(
        &shell_bin.join("xdg-open"),
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$1\" >> \"{}\"\n",
            opened_log.display()
        ),
    );
    let mock_shell = fixture.root.path().join("mock-shell.sh");
    write_executable_script(
        &mock_shell,
        &format!(
            "#!/bin/sh\nprintf 'Login greeting\\n'\nPATH=\"{}:$PATH\"\neval \"$2\"\n",
            shell_bin.display()
        ),
    );

    let empty_desktop_bin = fixture.root.path().join("empty-desktop-bin");
    fs::create_dir_all(&empty_desktop_bin).unwrap();

    // Open from clone1: shell probe provides xdg-open + updates app.json["path"].
    let open1 = Command::new(&bin1)
        .args(["app", "open"])
        .current_dir(&fixture.home)
        .env("HOME", &fixture.home)
        .env("XDG_DATA_HOME", &fixture.data)
        .env("XDG_CONFIG_HOME", &fixture.config)
        .env("XDG_STATE_HOME", fixture.root.path().join("private state"))
        .env("SHELL", &mock_shell)
        .env("PATH", &empty_desktop_bin)
        .env_remove("ACCELERATOR_HOME")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    fixture.assert_ok(&open1);
    let opened = fs::read_to_string(&opened_log).unwrap();
    assert!(opened.contains(&format!("http://127.0.0.1:{}/", server.port)));
    let record1: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.record()).unwrap()).unwrap();
    assert_eq!(record1["clone"], clone1.to_string_lossy().as_ref());
    assert!(
        record1["path"]
            .as_str()
            .unwrap()
            .contains(&*shell_bin.to_string_lossy())
    );

    // Open from a second clone while clone1 still exists: installed app stays on clone1.
    let open2_second = Command::new(&bin2)
        .args(["app", "open"])
        .current_dir(&clone2)
        .env("HOME", &fixture.home)
        .env("XDG_DATA_HOME", &fixture.data)
        .env("XDG_CONFIG_HOME", &fixture.config)
        .env("XDG_STATE_HOME", fixture.root.path().join("private state"))
        .env("SHELL", &mock_shell)
        .env("PATH", &empty_desktop_bin)
        .env_remove("ACCELERATOR_HOME")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    fixture.assert_ok(&open2_second);
    let record_second: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.record()).unwrap()).unwrap();
    assert_eq!(record_second["clone"], clone1.to_string_lossy().as_ref());

    // Remove clone1 to simulate moving it, break SHELL, and open from clone2:
    // cached app.json["path"] still finds xdg-open AND desktop entry follows clone2.
    fs::remove_dir_all(&clone1).unwrap();
    let open2_moved = Command::new(&bin2)
        .args(["app", "open"])
        .current_dir(&clone2)
        .env("HOME", &fixture.home)
        .env("XDG_DATA_HOME", &fixture.data)
        .env("XDG_CONFIG_HOME", &fixture.config)
        .env("XDG_STATE_HOME", fixture.root.path().join("private state"))
        .env("SHELL", fixture.root.path().join("missing-shell"))
        .env("PATH", &empty_desktop_bin)
        .env_remove("ACCELERATOR_HOME")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    fixture.assert_ok(&open2_moved);
    let record_moved: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.record()).unwrap()).unwrap();
    assert_eq!(record_moved["clone"], clone2.to_string_lossy().as_ref());
    assert_eq!(record_moved["executable"], bin2.to_string_lossy().as_ref());
    assert!(
        record_moved["path"]
            .as_str()
            .unwrap()
            .contains(&*shell_bin.to_string_lossy())
    );
    let desktop = fs::read_to_string(fixture.entry()).unwrap();
    assert!(desktop.contains(&format!("Exec={} app open", bin2.display())));
    assert!(desktop.contains(&format!("Exec={} app stop", bin2.display())));
}

#[test]
fn app_open_failure_writes_launcher_log_and_invokes_desktop_notification() {
    let fixture = Sandbox::new();
    let stub_bin = fixture.root.path().join("notify-bin");
    let notify_log = fixture.root.path().join("notify-args.txt");
    write_executable_script(
        &stub_bin.join("notify-send"),
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"{}\"\n",
            notify_log.display()
        ),
    );
    let failed = Command::new(&fixture.binary)
        .args(["app", "open", "--source"])
        .arg(&fixture.source)
        .args(["--unknown-open-flag", "value"])
        .current_dir(&fixture.home)
        .env("HOME", &fixture.home)
        .env("XDG_DATA_HOME", &fixture.data)
        .env("XDG_CONFIG_HOME", &fixture.config)
        .env("XDG_STATE_HOME", fixture.root.path().join("private state"))
        .env("SHELL", "")
        .env("PATH", &stub_bin)
        .env_remove("ACCELERATOR_HOME")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!failed.status.success());

    let launcher_log = fixture
        .config
        .join("ai-infrastructure-harness/launcher.log");
    assert!(launcher_log.is_file());
    let log_text = fs::read_to_string(&launcher_log).unwrap();
    assert!(log_text.contains("Unknown server option."), "{log_text}");
    let notify_text = fs::read_to_string(&notify_log).unwrap();
    assert!(notify_text.contains("AI Accelerator"), "{notify_text}");
    assert!(
        notify_text.contains("Unknown server option."),
        "{notify_text}"
    );
}

#[test]
fn app_open_skips_slow_shell_probe_when_running_and_forwards_project_and_logs_browser_failure() {
    let fixture = Sandbox::new();
    let state_dir = fixture
        .root
        .path()
        .join("private state/ai-infrastructure-harness");
    let server = MockHealthServer::start(&state_dir, "inst-fast", "tok-fast");

    let clone1 = fixture.root.path().join("clone1");
    let clone2 = fixture.root.path().join("clone2");
    for clone in [&clone1, &clone2] {
        fs::create_dir_all(clone.join("harness/web/icons")).unwrap();
        fs::create_dir_all(clone.join("Infrastructure-Creator")).unwrap();
        fs::copy(
            fixture.source.join("harness/web/index.html"),
            clone.join("harness/web/index.html"),
        )
        .unwrap();
        fs::copy(
            fixture.source.join("Infrastructure-Creator/AGENTS.md"),
            clone.join("Infrastructure-Creator/AGENTS.md"),
        )
        .unwrap();
        for name in [
            "ai-accelerator.svg",
            "ai-accelerator-16.png",
            "ai-accelerator-22.png",
            "ai-accelerator-24.png",
            "ai-accelerator-32.png",
            "ai-accelerator-48.png",
            "ai-accelerator-64.png",
            "ai-accelerator-128.png",
            "ai-accelerator-256.png",
            "ai-accelerator-512.png",
        ] {
            fs::copy(
                fixture.source.join("harness/web/icons").join(name),
                clone.join("harness/web/icons").join(name),
            )
            .unwrap();
        }
    }
    let bin1 = clone1.join("bin/infra-engine");
    let bin2 = clone2.join("bin/infra-engine");
    fs::create_dir_all(bin1.parent().unwrap()).unwrap();
    fs::create_dir_all(bin2.parent().unwrap()).unwrap();
    executable::copy(&fixture.binary, &bin1);
    executable::copy(&fixture.binary, &bin2);

    // Install clone1, then delete only clone1's binary (clone1 source dir still exists on disk).
    let inst = Command::new(&bin1)
        .args(["app", "install"])
        .current_dir(&clone1)
        .env("HOME", &fixture.home)
        .env("XDG_DATA_HOME", &fixture.data)
        .env("XDG_CONFIG_HOME", &fixture.config)
        .env("XDG_STATE_HOME", fixture.root.path().join("private state"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    fixture.assert_ok(&inst);
    fs::remove_file(&bin1).unwrap();

    // Slow/blocking $SHELL (would sleep 15s and hit timeout if called when already running!)
    let slow_shell = fixture.root.path().join("slow-shell.sh");
    write_executable_script(&slow_shell, "#!/bin/sh\nsleep 15\n");

    let target_project = fixture.root.path().join("target-project");
    fs::create_dir(&target_project).unwrap();
    let desktop_bin = fixture.root.path().join("desktop-bin");
    let opened_log = fixture.root.path().join("opened-project-url.txt");
    write_executable_script(
        &desktop_bin.join("xdg-open"),
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$1\" >> \"{}\"\n",
            opened_log.display()
        ),
    );

    let start = std::time::Instant::now();
    let fast_open = Command::new(&bin2)
        .args(["app", "open", "--project"])
        .arg(&target_project)
        .current_dir(&clone2)
        .env("HOME", &fixture.home)
        .env("XDG_DATA_HOME", &fixture.data)
        .env("XDG_CONFIG_HOME", &fixture.config)
        .env("XDG_STATE_HOME", fixture.root.path().join("private state"))
        .env("SHELL", &slow_shell)
        .env("PATH", &desktop_bin)
        .env_remove("ACCELERATOR_HOME")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let elapsed = start.elapsed();
    fixture.assert_ok(&fast_open);
    assert!(
        elapsed < std::time::Duration::from_secs(4),
        "app open against running server blocked on $SHELL probe ({elapsed:?})"
    );

    // `--project` was forwarded to running server and included in opened URL.
    let sent = server.forwarded_requests.lock().unwrap().clone();
    assert!(
        sent.iter().any(|(req, body)| {
            req == "POST /api/projects" && body.contains(target_project.to_string_lossy().as_ref())
        }),
        "expected POST /api/projects forwarding for running server: {sent:?}"
    );
    assert!(
        sent.iter()
            .any(|(req, _)| req == "POST /api/projects/proj-e2e-forwarded/select"),
        "expected POST /api/projects/{{id}}/select forwarding: {sent:?}"
    );
    let opened_url = fs::read_to_string(&opened_log).unwrap();
    assert!(
        opened_url.contains(&format!(
            "http://127.0.0.1:{}/#/sessions?project=proj-e2e-forwarded",
            server.port
        )),
        "expected project deep link in browser URL: {opened_url}"
    );

    // Deleting bin1 rebound app.json to clone2 even though clone1 source directory stayed on disk.
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.record()).unwrap()).unwrap();
    assert_eq!(record["clone"], clone2.to_string_lossy().as_ref());
    assert_eq!(record["executable"], bin2.to_string_lossy().as_ref());

    // Pre-fill launcher.log > 256 KiB, fail xdg-open with stderr, verify rotation + launcher.log + notify-send.
    let launcher_log = fixture
        .config
        .join("ai-infrastructure-harness/launcher.log");
    fs::create_dir_all(launcher_log.parent().unwrap()).unwrap();
    let oversized_marker = "OLD_LAUNCHER_HISTORY\n";
    let mut padded = oversized_marker.as_bytes().to_vec();
    padded.resize(260 * 1024, b'x');
    fs::write(&launcher_log, &padded).unwrap();

    let fail_bin = fixture.root.path().join("fail-browser-bin");
    let notify_log = fixture.root.path().join("browser-fail-notify.txt");
    write_executable_script(
        &fail_bin.join("xdg-open"),
        "#!/bin/sh\necho 'no graphical display' >&2\nexit 3\n",
    );
    write_executable_script(
        &fail_bin.join("notify-send"),
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"{}\"\n",
            notify_log.display()
        ),
    );
    let fail_open = Command::new(&bin2)
        .args(["app", "open"])
        .current_dir(&clone2)
        .env("HOME", &fixture.home)
        .env("XDG_DATA_HOME", &fixture.data)
        .env("XDG_CONFIG_HOME", &fixture.config)
        .env("XDG_STATE_HOME", fixture.root.path().join("private state"))
        .env("SHELL", &slow_shell)
        .env("PATH", &fail_bin)
        .env_remove("ACCELERATOR_HOME")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    fixture.assert_ok(&fail_open);
    let stdout = String::from_utf8_lossy(&fail_open.stdout);
    assert!(stdout.contains(&format!("Open http://127.0.0.1:{}/", server.port)));

    let rotated = fixture
        .config
        .join("ai-infrastructure-harness/launcher.log.1");
    assert!(rotated.is_file(), "expected single-backup launcher.log.1");
    assert!(
        fs::read_to_string(&rotated)
            .unwrap()
            .starts_with(oversized_marker)
    );
    let fresh_log = fs::read_to_string(&launcher_log).unwrap();
    assert!(fresh_log.contains("xdg-open exited with"), "{fresh_log}");
    assert!(fresh_log.contains("no graphical display"), "{fresh_log}");
    let notify_out = fs::read_to_string(&notify_log).unwrap();
    assert!(notify_out.contains("xdg-open exited with"), "{notify_out}");
    assert!(notify_out.contains("no graphical display"), "{notify_out}");
}
