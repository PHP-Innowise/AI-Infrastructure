//! Actual native attached-state admission with child-only configuration isolation.
#[path = "support/executable.rs"]
mod executable;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

struct Sandbox {
    root: PathBuf,
    source: PathBuf,
    project: PathBuf,
    state: PathBuf,
    binary: PathBuf,
}
impl Sandbox {
    fn new() -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "infra-native-attach-{}-{nonce}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let root = infra_core::fs::canonicalize(root).unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .unwrap()
            .to_path_buf();
        let project = root.join("target PHP project");
        fs::create_dir(&project).unwrap();
        fs::write(
            project.join("composer.json"),
            br#"{"name":"fixture/project"}"#,
        )
        .unwrap();
        let binary = root.join(if cfg!(windows) {
            "infra-engine.exe"
        } else {
            "infra-engine"
        });
        executable::copy(env!("CARGO_BIN_EXE_infra-engine"), &binary);
        Self {
            state: root.join("private state base"),
            root,
            source,
            project,
            binary,
        }
    }
    fn run(&self, action: &str, host: Option<&str>) -> Output {
        let mut argv = vec![OsString::from("attach"), action.into()];
        if let Some(host) = host {
            argv.push(host.into());
        }
        argv.extend([
            "--source-root".into(),
            self.source.as_os_str().into(),
            "--project".into(),
            self.project.as_os_str().into(),
            "--edition".into(),
            "PHP Core".into(),
            "--state-base".into(),
            self.state.as_os_str().into(),
        ]);
        let home = self.root.join("private user home");
        Command::new(&self.binary)
            .args(argv)
            .current_dir(&self.project)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("XDG_DATA_HOME", home.join("data"))
            .env("XDG_STATE_HOME", home.join("state"))
            .env("APPDATA", home.join("appdata"))
            .env("LOCALAPPDATA", home.join("localappdata"))
            .env("CODEX_HOME", home.join("codex"))
            .env("PATH", "")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", home.join("absent-gitconfig"))
            .env_remove("ACCELERATOR_HOME")
            .env_remove("ACCELERATOR_STATE_DIR")
            .env_remove("ACCELERATOR_PROJECT_DIR")
            .env_remove("INFRA_SOURCE_ROOT")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
    fn state_root(&self) -> PathBuf {
        self.state
            .join("attached")
            .join(infra_tooling::attach::project_key(&self.project).unwrap())
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Image {
    kind: u8,
    bytes: Vec<u8>,
    mode: u32,
}
fn tree(root: &Path) -> BTreeMap<PathBuf, Image> {
    fn descend(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Image>) {
        let metadata = fs::symlink_metadata(path).unwrap();
        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode() & 0o777
        };
        #[cfg(not(unix))]
        let mode = if metadata.permissions().readonly() {
            0o444
        } else {
            0o644
        };
        let (kind, bytes) = if metadata.file_type().is_symlink() {
            (
                2,
                fs::read_link(path)
                    .unwrap()
                    .as_os_str()
                    .to_string_lossy()
                    .as_bytes()
                    .to_vec(),
            )
        } else if metadata.is_file() {
            (1, fs::read(path).unwrap())
        } else {
            (0, vec![])
        };
        out.insert(
            path.strip_prefix(root).unwrap().to_path_buf(),
            Image { kind, bytes, mode },
        );
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            for child in fs::read_dir(path).unwrap() {
                descend(root, &child.unwrap().path(), out);
            }
        }
    }
    let mut out = BTreeMap::new();
    if root.exists() {
        descend(root, root, &mut out);
    }
    out
}

#[cfg(unix)]
#[test]
fn linked_attached_ancestor_refuses_before_chmod_lock_or_seed_and_normal_state_still_works() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let boxed = Sandbox::new();
    fs::create_dir(&boxed.state).unwrap();
    let redirected = boxed.project.join("redirected state");
    fs::create_dir(&redirected).unwrap();
    let hash = boxed.state_root().file_name().unwrap().to_os_string();
    let state = redirected.join(hash);
    fs::create_dir_all(state.join("project-brain/local")).unwrap();
    fs::write(
        state.join("project-brain/local/keep.txt"),
        b"project-owned bytes",
    )
    .unwrap();
    fs::write(
        state.join("accelerator-attach.json"),
        b"client-owned marker",
    )
    .unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(
        state.join("project-brain"),
        fs::Permissions::from_mode(0o775),
    )
    .unwrap();
    fs::set_permissions(
        state.join("project-brain/local/keep.txt"),
        fs::Permissions::from_mode(0o640),
    )
    .unwrap();
    symlink(&redirected, boxed.state.join("attached")).unwrap();
    let project_before = tree(&boxed.project);
    let state_before = tree(&boxed.state);
    let result = boxed.run("overlay", None);
    assert!(
        !result.status.success(),
        "linked state admitted: {}",
        String::from_utf8_lossy(&result.stdout)
    );
    assert_eq!(
        tree(&boxed.project),
        project_before,
        "refusal changed redirected project bytes, modes or tree"
    );
    assert_eq!(
        tree(&boxed.state),
        state_before,
        "refusal changed the named state chain"
    );
    assert!(!boxed.root.join("private user home").exists());

    // Positive control reaches real marker/seed/lock effects through ordinary custody.
    fs::remove_file(boxed.state.join("attached")).unwrap();
    let result = boxed.run("overlay", None);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let record: serde_json::Value = serde_json::from_slice(
        &fs::read(boxed.state_root().join("accelerator-attach.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(record["project"], serde_json::json!(boxed.project));
    assert!(
        boxed
            .state_root()
            .join("project-brain/config/runtime.json")
            .is_file()
    );
    assert!(
        boxed
            .state_root()
            .join("project-brain/local/runtime.lock")
            .is_file()
    );
    assert_eq!(
        tree(&boxed.project),
        project_before,
        "normal attachment changed project sources"
    );
}

#[test]
fn unknown_attach_operation_preserves_absent_and_existing_private_state() {
    let boxed = Sandbox::new();
    let before = tree(&boxed.project);
    let result = boxed.run("invented-operation", None);
    assert_eq!(
        result.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stderr).contains("Unknown attach operation"));
    assert!(!boxed.state.exists());
    fs::create_dir_all(boxed.state_root()).unwrap();
    fs::write(
        boxed.state_root().join("accelerator-attach.json"),
        b"unchanged marker",
    )
    .unwrap();
    let state_before = tree(&boxed.state);
    let result = boxed.run("invented-operation", None);
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(tree(&boxed.state), state_before);
    assert_eq!(tree(&boxed.project), before);
    assert!(!boxed.root.join("private user home").exists());
}

#[test]
fn unknown_attach_host_is_rejected_before_preparation_or_provider_launch() {
    let boxed = Sandbox::new();
    let before = tree(&boxed.project);
    let result = boxed.run("run", Some("invented-host"));
    assert_eq!(
        result.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stderr).contains("Unknown attached host"));
    assert!(!boxed.state.exists(), "invalid host created attached state");
    fs::create_dir_all(boxed.state_root()).unwrap();
    fs::write(
        boxed.state_root().join("accelerator-attach.json"),
        b"unchanged marker",
    )
    .unwrap();
    let state_before = tree(&boxed.state);
    let result = boxed.run("run", Some("invented-host"));
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(
        tree(&boxed.state),
        state_before,
        "invalid host modified existing private state"
    );
    assert_eq!(tree(&boxed.project), before);
    assert!(!boxed.root.join("private user home").exists());
}

#[cfg(unix)]
#[test]
fn linked_ancestor_above_an_existing_state_base_is_refused_without_project_effects() {
    use std::os::unix::fs::symlink;
    let mut boxed = Sandbox::new();
    let redirected = boxed.project.join("redirected ancestor");
    fs::create_dir_all(redirected.join("ordinary base")).unwrap();
    symlink(&redirected, boxed.root.join("redirect")).unwrap();
    boxed.state = boxed.root.join("redirect/ordinary base");
    let expected = redirected
        .join("ordinary base/attached")
        .join(boxed.state_root().file_name().unwrap());
    fs::create_dir_all(expected.join("project-brain/local")).unwrap();
    fs::write(
        expected.join("accelerator-attach.json"),
        b"project-owned marker",
    )
    .unwrap();
    fs::write(
        expected.join("project-brain/local/sentinel"),
        b"unchanged bytes",
    )
    .unwrap();
    let project_before = tree(&boxed.project);
    let whole_before = tree(&boxed.root);
    let result = boxed.run("overlay", None);
    assert!(
        !result.status.success(),
        "higher linked ancestor was admitted"
    );
    assert_eq!(tree(&boxed.project), project_before);
    assert_eq!(
        tree(&boxed.root),
        whole_before,
        "refusal changed bytes, modes or tree"
    );
}

#[cfg(unix)]
#[test]
fn a_literal_unix_backslash_state_base_is_supported_without_an_alternate_tree() {
    let mut boxed = Sandbox::new();
    boxed.state = boxed.root.join(r"missing\literal base");
    let project_before = tree(&boxed.project);
    let result = boxed.run("overlay", None);
    assert!(
        result.status.success(),
        "literal Unix base refused: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        boxed.state.is_dir(),
        "the literal state base was not created"
    );
    assert!(
        !boxed.root.join("missing").exists(),
        "backslash became a path separator"
    );
    let state = boxed.state_root();
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(state.join("accelerator-attach.json")).unwrap()).unwrap();
    assert_eq!(record["project"], serde_json::json!(boxed.project));
    assert!(state.join("project-brain/config/runtime.json").is_file());
    assert!(state.join("project-brain/local/runtime.lock").is_file());
    assert_eq!(
        tree(&boxed.project),
        project_before,
        "literal-root attachment changed project sources"
    );
}

#[cfg(windows)]
#[test]
#[ignore = "requires native Windows directory-symlink privilege or Developer Mode"]
fn linked_windows_ancestor_above_existing_state_base_is_refused_before_publication() {
    use std::os::windows::fs::symlink_dir;
    let mut boxed = Sandbox::new();
    let redirected = boxed.project.join("redirected ancestor");
    fs::create_dir_all(redirected.join("ordinary base")).unwrap();
    symlink_dir(&redirected, boxed.root.join("redirect")).unwrap();
    boxed.state = boxed.root.join("redirect/ordinary base");
    let expected = redirected
        .join("ordinary base/attached")
        .join(boxed.state_root().file_name().unwrap());
    fs::create_dir_all(expected.join("project-brain/local")).unwrap();
    fs::write(
        expected.join("accelerator-attach.json"),
        b"project-owned marker",
    )
    .unwrap();
    fs::write(
        expected.join("project-brain/local/sentinel"),
        b"unchanged bytes",
    )
    .unwrap();
    let before = tree(&boxed.project);
    let result = boxed.run("overlay", None);
    assert!(
        !result.status.success(),
        "a higher Windows directory reparse was admitted"
    );
    assert_eq!(
        tree(&boxed.project),
        before,
        "refusal changed redirected project bytes or tree"
    );
}
