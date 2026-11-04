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
        1,
        "native executable path injected another Desktop Entry key: {desktop}"
    );
    assert!(exec[0].starts_with("Exec=\""));
    assert!(exec[0].contains(r"trusted\nsecond\nliteral "));
    // Independent canonical vector: two slashes before $/backtick, four for
    // the literal backslash, then the key-level tab escape.
    assert!(exec[0].contains(r"literal \\$\\`\\\\\t/infra-engine"));
    assert!(exec[0].ends_with("\" app open"));
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
