//! Native installer regression contracts. Subprocesses execute the actual Cargo
//! engine; sources, projects, HOME and CODEX_HOME stay in isolated temp roots.
use infra_core::RuntimeError;
use infra_tooling::installer::{InstallPlan, Installer};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const ENGINE_PATH: &str = if cfg!(windows) {
    "memory-bank/bin/infra-engine.exe"
} else {
    "memory-bank/bin/infra-engine"
};
const RECORD: &str = "memory-bank/local/accelerator-install.json";
static INSTALL_TEST: Mutex<()> = Mutex::new(());
fn exclusive() -> MutexGuard<'static, ()> {
    INSTALL_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn engine() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_infra-engine"))
}
fn repository() -> PathBuf {
    infra_core::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")).unwrap()
}
fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}
#[cfg(unix)]
fn chmod(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}
fn mode(path: &Path) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Some(fs::metadata(path).unwrap().permissions().mode() & 0o777)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}
#[derive(Debug, PartialEq, Eq)]
struct Image {
    bytes: Vec<u8>,
    mode: Option<u32>,
}
fn images(root: &Path) -> BTreeMap<String, Image> {
    fn visit(root: &Path, here: &Path, result: &mut BTreeMap<String, Image>) {
        if !here.exists() {
            return;
        }
        for entry in fs::read_dir(here).unwrap() {
            let path = entry.unwrap().path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            assert!(
                !infra_core::fs::linked(&metadata),
                "fixture entry became a link"
            );
            if metadata.is_dir() {
                visit(root, &path, result);
            } else {
                assert!(metadata.is_file());
                let name = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .replace('\\', "/");
                result.insert(
                    name,
                    Image {
                        bytes: fs::read(&path).unwrap(),
                        mode: mode(&path),
                    },
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}
struct Sandbox {
    root: PathBuf,
    home: PathBuf,
    codex_home: PathBuf,
}
struct Reply {
    code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
impl Reply {
    fn expect(self, code: i32) -> Self {
        assert_eq!(
            self.code,
            code,
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&self.stdout),
            String::from_utf8_lossy(&self.stderr)
        );
        self
    }
    fn json(&self) -> Value {
        serde_json::from_slice(&self.stdout).unwrap_or_else(|error| {
            panic!(
                "invalid native JSON: {error}\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&self.stdout),
                String::from_utf8_lossy(&self.stderr)
            )
        })
    }
}
impl Sandbox {
    fn new() -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "infra-native-install-{}-{nonce}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let root = infra_core::fs::canonicalize(root).unwrap();
        let home = root.join("private home");
        let codex_home = root.join("private Codex home");
        fs::create_dir(&home).unwrap();
        fs::create_dir(&codex_home).unwrap();
        write(
            &codex_home.join("config.toml"),
            "[test_fixture]\nkeep = true\n",
        );
        Self {
            root,
            home,
            codex_home,
        }
    }
    fn run_at(&self, program: &Path, args: &[OsString], cwd: &Path) -> Reply {
        static CALL: AtomicU64 = AtomicU64::new(0);
        let id = CALL.fetch_add(1, Ordering::Relaxed);
        let stdout = self.root.join(format!("command-{id}.stdout"));
        let stderr = self.root.join(format!("command-{id}.stderr"));
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(cwd)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CODEX_HOME", &self.codex_home)
            .env("XDG_CONFIG_HOME", self.home.join("config"))
            .env("XDG_DATA_HOME", self.home.join("data"))
            .env("XDG_STATE_HOME", self.home.join("state"))
            .env("APPDATA", self.home.join("appdata"))
            .env("LOCALAPPDATA", self.home.join("localappdata"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.home.join("gitconfig"))
            .env_remove("INFRA_SOURCE_ROOT")
            .env_remove("ACCELERATOR_HOME")
            .env_remove("ACCELERATOR_STATE_DIR")
            .env_remove("ACCELERATOR_PROJECT_DIR")
            .stdin(Stdio::null())
            .stdout(File::create(&stdout).unwrap())
            .stderr(File::create(&stderr).unwrap());
        let mut child = command.spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("native fixture command exceeded 60 seconds: {args:?}");
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        fn bounded(path: &Path) -> Vec<u8> {
            let mut bytes = Vec::new();
            File::open(path)
                .unwrap()
                .take(16 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .unwrap();
            assert!(
                bytes.len() <= 16 * 1024 * 1024,
                "native test output exceeded 16 MiB"
            );
            bytes
        }
        Reply {
            code: status.code().unwrap_or(-1),
            stdout: bounded(&stdout),
            stderr: bounded(&stderr),
        }
    }
    fn run(&self, args: &[OsString]) -> Reply {
        self.run_at(engine(), args, &self.root)
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn install_args(
    source: &Path,
    target: &Path,
    action: Option<&str>,
    extra: &[&str],
) -> Vec<OsString> {
    let mut args = vec!["install".into()];
    if let Some(action) = action {
        args.push(action.into());
    }
    args.extend([
        "--source-root".into(),
        source.as_os_str().into(),
        "--target".into(),
        target.as_os_str().into(),
        "--json".into(),
    ]);
    if action != Some("apply") {
        args.extend([
            "--edition".into(),
            "PHP Core".into(),
            "--tool".into(),
            "codex".into(),
        ]);
    }
    args.extend(extra.iter().map(OsString::from));
    args
}
fn plan(boxed: &Sandbox, source: &Path, target: &Path, extra: &[&str]) -> InstallPlan {
    let output = boxed
        .run(&install_args(source, target, Some("plan"), extra))
        .expect(0);
    let plan: InstallPlan = serde_json::from_slice(&output.stdout).unwrap();
    plan.verify().unwrap();
    plan
}
fn apply_cli(boxed: &Sandbox, source: &Path, target: &Path, plan: &InstallPlan) -> Reply {
    let receipt = boxed.root.join("reviewed install plan.json");
    write(&receipt, serde_json::to_vec(plan).unwrap());
    let mut args = install_args(source, target, Some("apply"), &[]);
    args.extend(["--plan".into(), receipt.as_os_str().into()]);
    boxed.run(&args)
}
struct Fixture {
    boxed: Sandbox,
    source: PathBuf,
    target: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let boxed = Sandbox::new();
        let source = boxed.root.join("isolated release");
        let target = boxed.root.join("target parent/project with spaces");
        fs::create_dir(&source).unwrap();
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        let repo = repository();
        let mut inventory: Value = serde_json::from_slice(
            &fs::read(repo.join("install/inventories/php-core.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(inventory["schema_version"], 3);
        let wanted = [
            ".gitattributes",
            ".gitignore",
            "AGENTS.md",
            "README.md",
            "VERSION",
            "memory-bank/INDEX.md",
            "project-brain/config/runtime.json",
            "tasks/.task-counter",
            ".codex/config.toml",
            ".codex/hooks.json",
            ".codex/hooks/working-memory-read.sh",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
        let mut selected = BTreeSet::new();
        for paths in inventory["installed"].as_object_mut().unwrap().values_mut() {
            paths.as_array_mut().unwrap().retain(|value| {
                let path = value.as_str().unwrap();
                if wanted.contains(path) {
                    selected.insert(path.to_string());
                    true
                } else {
                    false
                }
            });
        }
        assert_eq!(
            selected, wanted,
            "fixture must derive real declared production payloads"
        );
        inventory["source_overrides"]
            .as_object_mut()
            .unwrap()
            .retain(|name, _| selected.contains(name));
        for name in &selected {
            let origin = inventory["source_overrides"][name].as_str().unwrap_or(name);
            let destination = source.join("PHP Core").join(origin);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(repo.join("PHP Core").join(origin), destination).unwrap();
        }
        write(
            &source.join("install/inventories/php-core.json"),
            serde_json::to_vec_pretty(&inventory).unwrap(),
        );
        write(
            &source.join("install/native-payload.json"),
            fs::read(repo.join("install/native-payload.json")).unwrap(),
        );
        write(
            &source.join("PHP Core/memory-bank/scripts/context.py"),
            "# unlisted retired payload\n",
        );
        Self {
            boxed,
            source,
            target,
        }
    }
    fn installer(&self) -> Installer {
        Installer::new(&self.source)
            .unwrap()
            .with_engine(engine())
            .unwrap()
    }
    fn reviewed(&self, extra: &[&str]) -> InstallPlan {
        plan(&self.boxed, &self.source, &self.target, extra)
    }
    fn project(&self) {
        write(&self.target.join("README.md"), "# Client project\n");
        write(&self.target.join("AGENTS.md"), "# Client rules\n");
        write(&self.target.join(".gitignore"), "/client-build/\n");
        write(
            &self.target.join("client.txt"),
            "protected project source\n",
        );
        write(
            &self.target.join("project-brain/config/runtime.json"),
            "{\"client_state\":true}\n",
        );
        write(
            &self.target.join("memory-bank/INDEX.md"),
            "# Client memories\n",
        );
        write(&self.target.join("tasks/.task-counter"), "91\n");
        #[cfg(unix)]
        {
            chmod(&self.target.join(".gitignore"), 0o640);
            chmod(&self.target.join("AGENTS.md"), 0o640);
        }
    }
}

#[test]
fn real_php_inventory_public_cli_plan_dry_run_apply_and_installed_engine() {
    let _serial = exclusive();
    let boxed = Sandbox::new();
    let source = repository();
    let target = boxed.root.join("fresh actual edition");
    let reviewed = plan(&boxed, &source, &target, &[]);
    assert!(
        reviewed.files.len() > 100,
        "exercise the actual edition, not a stub inventory"
    );
    assert!(!target.exists(), "planning must not create the target");
    let dry = boxed
        .run(&install_args(&source, &target, None, &["--dry-run"]))
        .expect(0)
        .json();
    assert_eq!(dry["status"], "planned");
    assert_eq!(dry["dry_run"], true);
    assert!(!target.exists(), "dry-run must not create the target");
    let applied = apply_cli(&boxed, &source, &target, &reviewed)
        .expect(0)
        .json();
    assert_eq!(applied["status"], "complete");
    assert_eq!(applied["dry_run"], false);
    let paths = images(&target);
    assert!(
        paths.keys().all(|path| !path.ends_with(".py")),
        "Python is not installed"
    );
    assert!(!target.join("memory-bank/scripts/context.py").exists());
    assert!(!target.join("project-brain/scripts/validate.py").exists());
    let binary = target.join(ENGINE_PATH);
    let caps = boxed
        .run_at(&binary, &["capabilities".into(), "--json".into()], &target)
        .expect(0)
        .json();
    assert_eq!(caps["engine"], "infra-engine");
    assert!(
        caps["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item == "install")
    );
    let record: Value = serde_json::from_slice(&fs::read(target.join(RECORD)).unwrap()).unwrap();
    assert_eq!(record["edition"], "PHP Core");
    assert_eq!(record["engine_sha256"], reviewed.engine["artifact_sha256"]);
    #[cfg(unix)]
    {
        assert_eq!(mode(&binary), Some(0o755));
        assert_eq!(mode(&target.join("AGENTS.md")), Some(0o644));
        assert_eq!(mode(&target.join(RECORD)), Some(0o600));
    }
}

#[test]
fn collision_refuses_before_creating_any_payload() {
    let _serial = exclusive();
    let fixture = Fixture::new();
    fixture.project();
    let before = images(&fixture.target);
    let result = fixture
        .boxed
        .run(&install_args(&fixture.source, &fixture.target, None, &[]))
        .expect(2)
        .json();
    assert_eq!(result["status"], "refused");
    assert!(!result["collisions"].as_array().unwrap().is_empty());
    assert_eq!(images(&fixture.target), before);
    assert!(!fixture.target.join(ENGINE_PATH).exists());
    assert!(!fixture.target.join(RECORD).exists());
}

#[test]
fn reviewed_source_edits_and_same_byte_replacements_are_rejected_before_write() {
    let _serial = exclusive();
    for replace_identity in [false, true] {
        let fixture = Fixture::new();
        let reviewed = fixture.reviewed(&[]);
        let source = fixture.source.join("PHP Core/AGENTS.md");
        if replace_identity {
            let bytes = fs::read(&source).unwrap();
            fs::rename(&source, source.with_extension("old.md")).unwrap();
            write(&source, bytes);
        } else {
            write(&source, "# Edited after review\n");
        }
        let output =
            apply_cli(&fixture.boxed, &fixture.source, &fixture.target, &reviewed).expect(1);
        assert!(String::from_utf8_lossy(&output.stderr).contains("changed"));
        assert!(
            !fixture.target.exists(),
            "a stale source must cause zero publication"
        );
    }
}

#[test]
fn reviewed_target_edit_is_rejected_without_overwriting_the_new_project_bytes() {
    let _serial = exclusive();
    let fixture = Fixture::new();
    fixture.project();
    let reviewed = fixture.reviewed(&["--merge-existing"]);
    write(
        &fixture.target.join("AGENTS.md"),
        "# A concurrent client edit\n",
    );
    let before = images(&fixture.target);
    let output = apply_cli(&fixture.boxed, &fixture.source, &fixture.target, &reviewed).expect(1);
    assert!(!output.stderr.is_empty());
    assert_eq!(images(&fixture.target), before);
    assert!(!fixture.target.join(ENGINE_PATH).exists());
}

#[test]
fn an_absent_target_receipt_rejects_replacement_of_its_existing_parent() {
    let _serial = exclusive();
    let fixture = Fixture::new();
    let reviewed = fixture.reviewed(&[]);
    let parent = fixture.target.parent().unwrap();
    fs::rename(parent, parent.with_extension("original")).unwrap();
    fs::create_dir(parent).unwrap();
    let output = apply_cli(&fixture.boxed, &fixture.source, &fixture.target, &reviewed);
    assert_ne!(
        output.code, 0,
        "reviewed installation must not follow a replacement parent"
    );
    assert!(
        !fixture.target.exists(),
        "no bytes may enter the unreviewed namespace"
    );
}

#[test]
fn observer_rejection_rolls_back_confirmed_writes_and_original_modes() {
    let _serial = exclusive();
    let fixture = Fixture::new();
    fixture.project();
    let reviewed = fixture.reviewed(&["--merge-existing"]);
    let before = images(&fixture.target);
    let mut events = Vec::new();
    let result = fixture
        .installer()
        .apply_plan_observed(&reviewed, |path, action| {
            events.push((path.to_string(), action.to_string()));
            if path == ".gitignore" && action != "rolled-back" && action != "rollback-failed" {
                return Err(RuntimeError::conflict(
                    "injected native installer rejection",
                ));
            }
            Ok(())
        });
    let error = result.unwrap_err();
    assert!(
        error
            .message
            .contains("injected native installer rejection")
    );
    assert!(!error.message.contains("rollback requires attention"));
    assert!(
        events
            .iter()
            .any(|(path, action)| path == ENGINE_PATH && action == "rolled-back")
    );
    assert!(
        events
            .iter()
            .any(|(path, action)| path == ".gitignore" && action == "rolled-back")
    );
    assert_eq!(images(&fixture.target), before);
    assert!(!fixture.target.join(RECORD).exists());
}

#[test]
fn a_race_after_first_durable_write_rolls_back_ours_and_preserves_the_client_edit() {
    let _serial = exclusive();
    let fixture = Fixture::new();
    fixture.project();
    let reviewed = fixture.reviewed(&["--merge-existing"]);
    let mut expected = images(&fixture.target);
    expected.get_mut(".gitignore").unwrap().bytes = b"/concurrent-client/\n".to_vec();
    let mut raced = false;
    let mut rolled_back = false;
    let result = fixture
        .installer()
        .apply_plan_observed(&reviewed, |path, action| {
            if path == ENGINE_PATH && action != "rolled-back" && !raced {
                write(&fixture.target.join(".gitignore"), "/concurrent-client/\n");
                raced = true;
            }
            if path == ENGINE_PATH && action == "rolled-back" {
                rolled_back = true;
            }
            Ok(())
        });
    let error = result.unwrap_err();
    assert!(
        raced && rolled_back,
        "race must occur after an actual artifact publication"
    );
    assert!(!error.message.contains("rollback requires attention"));
    assert_eq!(images(&fixture.target), expected);
    assert!(!fixture.target.join(RECORD).exists());
}

fn listing(project: &Path) -> Value {
    let config: Value =
        serde_json::from_slice(&fs::read(project.join(".codex/hooks.json")).unwrap()).unwrap();
    let mut rows = Vec::new();
    for (event, groups) in config["hooks"].as_object().unwrap() {
        for group in groups.as_array().unwrap() {
            for handler in group["hooks"].as_array().unwrap() {
                let mut row = handler.clone();
                let index = rows.len();
                row["key"] = format!("native-fixture-{index}").into();
                row["currentHash"] = format!("host-opaque-hash-{index}").into();
                row["eventName"] = event.clone().into();
                row["source"] = "project".into();
                row["sourcePath"] = project
                    .join(".codex/hooks.json")
                    .display()
                    .to_string()
                    .into();
                row["trustStatus"] = "untrusted".into();
                rows.push(row);
            }
        }
    }
    json!({"data":[{"hooks":rows}]})
}

#[test]
fn merge_preserves_tracked_project_state_and_native_hook_trust_uses_materialized_launches() {
    let _serial = exclusive();
    let fixture = Fixture::new();
    fixture.project();
    fixture
        .boxed
        .run_at(
            Path::new("git"),
            &["init".into(), "--quiet".into()],
            &fixture.target,
        )
        .expect(0);
    fixture
        .boxed
        .run_at(
            Path::new("git"),
            &[
                "add".into(),
                "--".into(),
                "README.md".into(),
                "AGENTS.md".into(),
                ".gitignore".into(),
                "client.txt".into(),
                "project-brain/config/runtime.json".into(),
            ],
            &fixture.target,
        )
        .expect(0);
    let index = fs::read(fixture.target.join(".git/index")).unwrap();
    let original = images(&fixture.target);
    let private_trust = images(&fixture.boxed.codex_home);
    let report = fixture
        .boxed
        .run(&install_args(
            &fixture.source,
            &fixture.target,
            None,
            &["--merge-existing"],
        ))
        .expect(0)
        .json();
    assert_eq!(report["status"], "complete");
    assert_eq!(fs::read(fixture.target.join(".git/index")).unwrap(), index);
    for path in [
        "README.md",
        "client.txt",
        "project-brain/config/runtime.json",
        "memory-bank/INDEX.md",
        "tasks/.task-counter",
    ] {
        assert_eq!(
            fs::read(fixture.target.join(path)).unwrap(),
            original[path].bytes
        );
    }
    assert!(fixture.target.join("ACCELERATOR.md").is_file());
    assert!(
        fs::read_to_string(fixture.target.join("AGENTS.md"))
            .unwrap()
            .starts_with("# Client rules\n")
    );
    assert!(
        !fixture
            .target
            .join("memory-bank/scripts/context.py")
            .exists()
    );
    #[cfg(unix)]
    {
        assert_eq!(mode(&fixture.target.join("AGENTS.md")), Some(0o640));
        assert_eq!(mode(&fixture.target.join(".gitignore")), Some(0o640));
    }

    let rows = listing(&fixture.target);
    let count = rows["data"][0]["hooks"].as_array().unwrap().len();
    assert!(count > 0);
    let select = |rows: &Value| {
        infra_tooling::attach::installed_hook_trust_selection(
            &fixture.source,
            "PHP Core",
            &fixture.target,
            engine(),
            rows,
        )
        .unwrap()
    };
    let selected = select(&rows);
    assert_eq!(selected["pending"].as_object().unwrap().len(), count);
    assert!(selected["left"].as_array().unwrap().is_empty());
    for (field, value) in [
        ("command", json!("exec /unreviewed/native-engine")),
        ("args", json!(["unreviewed-argument"])),
        ("shell", json!("unreviewed-shell")),
        ("commandWindows", json!("unreviewed-windows-command")),
        ("eventName", json!("UnreviewedEvent")),
    ] {
        let mut changed = rows.clone();
        changed["data"][0]["hooks"][0][field] = value;
        let rejected = select(&changed);
        assert_eq!(
            rejected["pending"].as_object().unwrap().len(),
            count - 1,
            "{field}"
        );
        assert_eq!(rejected["left"], json!(["native-fixture-0"]), "{field}");
    }
    let mut trusted = rows.clone();
    trusted["data"][0]["hooks"][0]["trustStatus"] = "trusted".into();
    assert_eq!(select(&trusted)["already"], 1);
    OpenOptions::new()
        .append(true)
        .open(fixture.target.join(ENGINE_PATH))
        .unwrap()
        .write_all(b"\nchanged native artifact\n")
        .unwrap();
    let changed_binary = select(&rows);
    assert!(changed_binary["pending"].as_object().unwrap().is_empty());
    assert_eq!(changed_binary["left"].as_array().unwrap().len(), count);
    assert_eq!(
        images(&fixture.boxed.codex_home),
        private_trust,
        "selection never writes user trust"
    );
}

#[test]
fn native_mcp_source_templates_materialize_but_edited_managed_blocks_are_refused() {
    let _serial = exclusive();
    let boxed = Sandbox::new();
    let target = boxed.root.join("future installed project");
    let repo = repository().join("PHP Core");
    for (host, path) in [("claude", ".mcp.json"), ("cursor", ".cursor/mcp.json")] {
        let source = fs::read(repo.join(path)).unwrap();
        let merged = infra_tooling::mcp_config::merge(path, None, &source, &target).unwrap();
        let document: Value = serde_json::from_slice(&merged).unwrap();
        assert_eq!(
            document["mcpServers"]["harness-memory"],
            infra_tooling::mcp_config::entry(host, &target).unwrap()
        );
        let mut edited: Value = serde_json::from_slice(&source).unwrap();
        edited["mcpServers"]["harness-memory"]["command"] = "client-owned-server".into();
        assert!(
            infra_tooling::mcp_config::merge(
                path,
                Some(&serde_json::to_vec(&edited).unwrap()),
                &source,
                &target
            )
            .is_err()
        );
    }
    let source = fs::read(repo.join(".codex/config.toml")).unwrap();
    let materialized =
        infra_tooling::mcp_config::merge(".codex/config.toml", None, &source, &target).unwrap();
    let materialized = String::from_utf8(materialized).unwrap();
    assert!(materialized.contains(&serde_json::to_string(&target.join(ENGINE_PATH)).unwrap()));
    let source = String::from_utf8(source).unwrap();
    for edited in [
        source.replace("# Source template: installation writes the portable launch (a bound one on native Windows).",
            "# A client-edited managed annotation"),
        source.replace("./memory-bank/bin/infra-engine", "./client-server"),
    ] {
        assert!(infra_tooling::mcp_config::merge(".codex/config.toml", Some(edited.as_bytes()),
            source.as_bytes(), &target).is_err());
    }
    assert!(
        !target.exists(),
        "pure materialization must not write a target"
    );
}

fn reviewed_test_identity(
    root: &Path,
    relative: &str,
) -> infra_core::bank::publication::FileImageIdentity {
    infra_core::bank::publication::PublicationFiles::new(root)
        .unwrap()
        .image_identity(relative)
        .unwrap()
        .unwrap()
}
fn replace_with_same_image(
    root: &Path,
    relative: &str,
    retained: &Path,
) -> infra_core::bank::publication::FileImageIdentity {
    let path = root.join(relative);
    let old = reviewed_test_identity(root, relative);
    let bytes = fs::read(&path).unwrap();
    let permissions = fs::metadata(&path).unwrap().permissions();
    let old_mode = mode(&path);
    // Keep the original inode alive so the negative control cannot recycle it.
    fs::rename(&path, retained).unwrap();
    write(&path, &bytes);
    fs::set_permissions(&path, permissions).unwrap();
    let replacement = reviewed_test_identity(root, relative);
    assert_ne!((old.0, old.1), (replacement.0, replacement.1));
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(mode(&path), old_mode);
    replacement
}

#[test]
fn reviewed_same_image_replacements_after_engine_publication_are_refused() {
    let _serial = exclusive();
    for victim in [".gitignore", RECORD] {
        let fixture = Fixture::new();
        fixture.project();
        if victim == RECORD {
            write(
                &fixture.target.join(RECORD),
                serde_json::to_vec(&json!({"schema":2,"edition":"PHP Core","files":{}})).unwrap(),
            );
        }
        let reviewed = fixture.reviewed(&["--merge-existing"]);
        assert!(
            reviewed
                .files
                .iter()
                .any(|file| file.path == ".gitignore" && file.before.is_some())
        );
        if victim == RECORD {
            assert!(reviewed.control.is_some());
        }
        let expected = images(&fixture.target);
        let retained = fixture.boxed.root.join("retained-reviewed-target");
        let mut replacement = None;
        let mut rolled_back = false;
        let mut victim_published = false;
        let result = fixture
            .installer()
            .apply_plan_observed(&reviewed, |path, action| {
                if path == ENGINE_PATH
                    && action != "rolled-back"
                    && action != "rollback-failed"
                    && replacement.is_none()
                {
                    replacement = Some(replace_with_same_image(&fixture.target, victim, &retained));
                }
                if path == ENGINE_PATH && action == "rolled-back" {
                    rolled_back = true;
                }
                if victim == ".gitignore"
                    && path == victim
                    && action != "rolled-back"
                    && action != "rollback-failed"
                {
                    victim_published = true;
                }
                Ok(())
            });
        let error = result.expect_err("a new inode cannot inherit the reviewed image's authority");
        assert_eq!(error.kind, infra_core::ErrorKind::Conflict);
        assert!(!error.message.contains("rollback requires attention"));
        assert!(
            rolled_back,
            "the actual installed engine must have been rolled back"
        );
        assert!(!victim_published);
        assert_eq!(images(&fixture.target), expected);
        assert_eq!(
            reviewed_test_identity(&fixture.target, victim),
            replacement.unwrap()
        );
        assert!(!fixture.target.join(ENGINE_PATH).exists());
        if victim != RECORD {
            assert!(!fixture.target.join(RECORD).exists());
        }
    }
}

#[test]
fn rollback_preserves_foreign_same_image_replacements_of_created_and_existing_files() {
    let _serial = exclusive();
    for victim in [ENGINE_PATH, ".gitignore"] {
        let fixture = Fixture::new();
        fixture.project();
        let reviewed = fixture.reviewed(&["--merge-existing"]);
        let mut expected = images(&fixture.target);
        let retained = fixture.boxed.root.join("retained-published-effect");
        let mut replacement = None;
        let mut reported_collision = false;
        let result = fixture
            .installer()
            .apply_plan_observed(&reviewed, |path, action| {
                if path == victim
                    && action != "rolled-back"
                    && action != "rollback-failed"
                    && replacement.is_none()
                {
                    let mut current = images(&fixture.target);
                    expected.insert(victim.to_string(), current.remove(victim).unwrap());
                    replacement = Some(replace_with_same_image(&fixture.target, victim, &retained));
                    return Err(RuntimeError::conflict(
                        "injected post-publication same-image replacement",
                    ));
                }
                if path == victim && action == "rollback-failed" {
                    reported_collision = true;
                }
                Ok(())
            });
        let error = result.expect_err("the injected post-publication refusal must propagate");
        assert!(error.message.contains("rollback requires attention"));
        assert!(
            reported_collision,
            "rollback must report ownership loss instead of deleting/replacing it"
        );
        assert_eq!(images(&fixture.target), expected);
        assert_eq!(
            reviewed_test_identity(&fixture.target, victim),
            replacement.unwrap()
        );
        assert!(!fixture.target.join(RECORD).exists());
        if victim != ENGINE_PATH {
            assert!(!fixture.target.join(ENGINE_PATH).exists());
        }
    }
}

// Frozen schema-1 installation data. The settings are the canonical 6628d976
// PHP Core release bytes; historical executable bodies below are inert fixtures.
// None of these scripts is executed by these tests.
const LEGACY_SYNC_SETTINGS: &[u8] = br###"{
  "env": {
    "APP_ENV": "local",
    "FORCE_COLOR": "1",
    "CLAUDE_CODE_DISABLE_EXPLORE_PLAN_AGENTS": "1",
    "CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH": "1"
  },
  "permissions": {
    "allow": [
      "Read(**)",
      "Edit(**)",
      "Grep(**)",
      "Bash(php:*)",
      "Bash(composer:*)",
      "Bash(vendor/bin/phpunit:*)",
      "Bash(vendor/bin/pest:*)",
      "Bash(vendor/bin/php-cs-fixer:*)",
      "Bash(vendor/bin/phpcs:*)",
      "Bash(vendor/bin/phpcbf:*)",
      "Bash(vendor/bin/phpstan:*)",
      "Bash(vendor/bin/psalm:*)",
      "Bash(vendor/bin/rector:*)",
      "Bash(vendor/bin/phpbench:*)",
      "Bash(git:*)",
      "Bash(python3 memory-bank/scripts/context.py:*)",
      "Bash(python memory-bank/scripts/context.py:*)",
      "Bash(py -3 memory-bank/scripts/context.py:*)",
      "Bash(gh:*)",
      "Bash(ls:*)",
      "Bash(mkdir:*)",
      "Bash(cp:*)",
      "Bash(mv:*)",
      "Bash(echo:*)",
      "Bash(make:*)",
      "WebFetch(domain:github.com)",
      "WebFetch(domain:api.github.com)",
      "WebFetch(domain:raw.githubusercontent.com)",
      "WebFetch(domain:php.net)",
      "WebFetch(domain:www.php.net)",
      "WebFetch(domain:php-fig.org)",
      "WebFetch(domain:www.php-fig.org)",
      "WebFetch(domain:getcomposer.org)",
      "WebFetch(domain:packagist.org)",
      "WebFetch(domain:pestphp.com)",
      "WebFetch(domain:phpunit.de)",
      "WebFetch(domain:phpstan.org)",
      "WebFetch(domain:psalm.dev)"
    ],
    "deny": [
      "Agent(Explore)",
      "Agent(Plan)",
      "Agent(general-purpose)",
      "Agent(claude)",
      "Agent(statusline-setup)",
      "Agent(claude-code-guide)",
      "Read(.env)",
      "Read(.env.*)",
      "Edit(.env)",
      "Edit(.env.*)",
      "Read(./secrets/**)",
      "Bash(rm -rf:*)",
      "Bash(sudo:*)",
      "Bash(chmod 777:*)"
    ]
  },
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "\"${CLAUDE_PROJECT_DIR}\"/.claude/hooks/local-context.sh",
            "timeout": 10
          },
          {
            "type": "command",
            "command": "\"${CLAUDE_PROJECT_DIR}\"/.claude/hooks/context-continuity.sh",
            "timeout": 8
          }
        ]
      }
    ],
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "\"${CLAUDE_PROJECT_DIR}\"/.claude/hooks/working-memory-read.sh",
            "timeout": 8
          },
          {
            "type": "command",
            "command": "\"${CLAUDE_PROJECT_DIR}\"/.claude/hooks/context-continuity.sh",
            "timeout": 8
          }
        ]
      }
    ],
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "\"${CLAUDE_PROJECT_DIR}\"/.claude/hooks/working-memory-write.sh",
            "timeout": 8
          },
          {
            "type": "command",
            "command": "\"${CLAUDE_PROJECT_DIR}\"/.claude/hooks/context-continuity.sh",
            "timeout": 8
          }
        ]
      }
    ],
    "PreToolUse": [
      {
        "matcher": "Agent|Task",
        "hooks": [
          {
            "type": "command",
            "command": "\"${CLAUDE_PROJECT_DIR}\"/.claude/hooks/subagent-gate.sh",
            "timeout": 5
          }
        ]
      },
      {
        "matcher": "Write|Edit",
        "hooks": [
          {
            "type": "command",
            "command": "\"${CLAUDE_PROJECT_DIR}\"/.claude/hooks/file-naming-validator.sh",
            "timeout": 5
          }
        ]
      },
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "\"${CLAUDE_PROJECT_DIR}\"/.claude/hooks/bash-validator.sh",
            "timeout": 5
          }
        ]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "Edit|Write|MultiEdit|NotebookEdit",
        "hooks": [
          {
            "type": "command",
            "command": "\"${CLAUDE_PROJECT_DIR}\"/.claude/hooks/loop-detection.sh",
            "timeout": 5
          }
        ]
      },
      {
        "matcher": "Agent|Task",
        "hooks": [
          {
            "type": "command",
            "command": "\"${CLAUDE_PROJECT_DIR}\"/.claude/hooks/subagent-dispatch.sh",
            "timeout": 5
          }
        ]
      }
    ],
    "SubagentStop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "\"${CLAUDE_PROJECT_DIR}\"/.claude/hooks/subagent-dispatch.sh",
            "timeout": 8
          }
        ]
      }
    ],
    "Notification": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "case \"$(uname -s)\" in Darwin) osascript -e 'display notification \"Claude needs your attention\" with title \"Claude Code\"' 2>/dev/null || echo 'Claude Code notification';; Linux) notify-send 'Claude Code' 'Claude needs your attention' 2>/dev/null || echo 'Claude Code notification';; *) echo 'Claude Code notification';; esac",
            "timeout": 5
          }
        ]
      }
    ]
  }
}
"###;

const LEGACY_SYNC_GUARD: &[u8] =
    b"#!/bin/sh\n# Historical guard fixture; never executed.\nexit 0\n";
const LEGACY_SYNC_CONTEXT: &[u8] = b"# Historical context CLI marker; never executed.\n";
const LEGACY_SYNC_MEMORY_HOOK: &[u8] = br###"#!/bin/sh
# Historical memory hook fixture; never executed.
exec python3 "$(dirname "$0")/../../memory-bank/scripts/context.py" refresh
"###;
const LEGACY_SYNC_EDITED_GUARD: &[u8] = b"#!/bin/sh\n# Client-owned self-contained safety gate.\nprintf '%s\\n' 'Client guard requires manual review' >&2\nexit 2\n";
const LEGACY_SYNC_HOOKS: &[&str] = &[
    "bash-validator",
    "context-continuity",
    "file-naming-validator",
    "local-context",
    "loop-detection",
    "subagent-dispatch",
    "subagent-gate",
    "working-memory-read",
    "working-memory-write",
];

struct LegacySyncUpgrade {
    boxed: Sandbox,
    source: PathBuf,
    target: PathBuf,
    authority: BTreeMap<String, Image>,
}
impl LegacySyncUpgrade {
    fn new(edited_guard: bool) -> Self {
        let boxed = Sandbox::new();
        let source = boxed.root.join("packaged native upgrade");
        let target = boxed.root.join("existing schema one project");
        let repo = repository();
        let mut inventory: Value = serde_json::from_slice(
            &fs::read(repo.join("install/inventories/php-core.json")).unwrap(),
        )
        .unwrap();
        let mut wanted = [
            ".claude/settings.json",
            "memory-bank/INDEX.md",
            "memory-bank/bin/.gitignore",
            "project-brain/config/runtime.json",
            "tasks/.task-counter",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
        wanted.extend(
            LEGACY_SYNC_HOOKS
                .iter()
                .map(|role| format!(".claude/hooks/{role}.sh")),
        );
        let mut selected = BTreeSet::new();
        for paths in inventory["installed"].as_object_mut().unwrap().values_mut() {
            paths.as_array_mut().unwrap().retain(|value| {
                let name = value.as_str().unwrap();
                if wanted.contains(name) {
                    selected.insert(name.to_string());
                    true
                } else {
                    false
                }
            });
        }
        assert_eq!(
            selected, wanted,
            "use actual declared native upgrade resources"
        );
        inventory["source_overrides"]
            .as_object_mut()
            .unwrap()
            .retain(|name, _| selected.contains(name));
        for name in &selected {
            let origin = inventory["source_overrides"][name].as_str().unwrap_or(name);
            let destination = source.join("PHP Core").join(origin);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(repo.join("PHP Core").join(origin), &destination).unwrap();
        }
        write(
            &source.join("install/inventories/php-core.json"),
            serde_json::to_vec_pretty(&inventory).unwrap(),
        );
        write(
            &source.join("install/native-payload.json"),
            fs::read(repo.join("install/native-payload.json")).unwrap(),
        );
        // The temp root may itself live below the real checkout. Give this
        // release its OWN history instead of borrowing any ancestor repository.
        // The runner clears ambient Git selectors and disables hooks; these
        // commits touch only the freshly created fixture's .git and source files.
        assert!(!source.join(".git").exists());
        let history_commands: &[&[&str]] = &[
            &["init", "--quiet", "--template=", "--initial-branch=main"],
            &[
                "-c",
                "core.autocrlf=false",
                "add",
                "--",
                "PHP Core",
                "install",
            ],
            &[
                "-c",
                "user.name=Native Sync Fixture",
                "-c",
                "user.email=native-sync-fixture@example.invalid",
                "-c",
                "commit.gpgSign=false",
                "commit",
                "--quiet",
                "--no-verify",
                "-m",
                "Private native release fixture",
            ],
        ];
        for args in history_commands {
            let result = infra_core::git::run(
                &source,
                &infra_core::git::strings(args),
                None,
                Duration::from_secs(15),
                64 * 1024,
            )
            .unwrap();
            assert_eq!(
                result.code, 0,
                "private release history setup failed: {args:?}"
            );
        }
        assert!(source.join(".git/HEAD").is_file());

        write(&target.join(".claude/settings.json"), LEGACY_SYNC_SETTINGS);
        write(
            &target.join("memory-bank/scripts/context.py"),
            LEGACY_SYNC_CONTEXT,
        );
        let mut known = serde_json::Map::new();
        known.insert(
            ".claude/settings.json".into(),
            json!("6adf0d9ccf1165b06baa7f680df402d0cfb2ee5a"),
        );
        known.insert(
            "memory-bank/scripts/context.py".into(),
            json!("26860d4b0f4b25c496d0c6e25357c9b991f38cb9"),
        );
        for role in LEGACY_SYNC_HOOKS {
            let path = format!(".claude/hooks/{role}.sh");
            let memory = matches!(
                *role,
                "context-continuity"
                    | "local-context"
                    | "working-memory-read"
                    | "working-memory-write"
            );
            let bytes = if memory {
                LEGACY_SYNC_MEMORY_HOOK
            } else {
                LEGACY_SYNC_GUARD
            };
            write(&target.join(&path), bytes);
            known.insert(
                path,
                json!(if memory {
                    "10095a5a53ce1e67c79fcb4b168854226fcc0f93"
                } else {
                    "8581b8e8b459e5ad7eb3ddd43668a8c0340e52db"
                }),
            );
        }
        if edited_guard {
            write(
                &target.join(".claude/hooks/bash-validator.sh"),
                LEGACY_SYNC_EDITED_GUARD,
            );
        }
        // A packaged Python-era release had no Git source commit. Its exact
        // install-record schema and Git-blob hashes remain migration inputs.
        write(
            &target.join(RECORD),
            serde_json::to_vec_pretty(&json!({
                "schema":1, "edition":"PHP Core", "release":"2.0.0",
                "source_commit":null, "synced_at":"2026-10-30T00:00:00+00:00",
                "files":known
            }))
            .unwrap(),
        );
        write(
            &target.join("project-brain/config/runtime.json"),
            b"{\"framework\":\"core\",\"automatic_promotion\":false,\"client_state\":true}\n",
        );
        write(
            &target.join("memory-bank/INDEX.md"),
            b"# Existing client memory index\n",
        );
        write(&target.join("tasks/.task-counter"), b"91\n");
        write(
            &target.join("project-brain/dynamic/tasks/client-open-task.md"),
            b"# Client task\n\nState: active\nProgress: unfinished authority\n",
        );
        write(
            &target.join("memory-bank/chunks/MEM-20261030-client.md"),
            b"# Client reviewed memory\n\nPreserve these bytes.\n",
        );
        {
            use infra_core::storage::{OpenOptions as StoreOptions, StartTask, Store};
            let mut store = Store::open(
                target.join("memory-bank/local/context.db"),
                StoreOptions::refresh(None),
            )
            .unwrap();
            store
                .start(StartTask {
                    task_id: "legacy-sync-pending".into(),
                    goal: "Preserve the unfinished client task".into(),
                    files: vec!["client/source.php".into()],
                    ..StartTask::default()
                })
                .unwrap();
            store
                .append_turn("legacy-sync-pending", &["client/pending.php".into()])
                .unwrap();
            assert_eq!(store.pending_turns("legacy-sync-pending").unwrap().len(), 1);
            assert!(store.working("legacy-sync-pending").unwrap().is_some());
        }
        let authority = images(&target)
            .into_iter()
            .filter(|(name, _)| {
                !name.starts_with(".claude/")
                    && name != RECORD
                    && name != "memory-bank/scripts/context.py"
            })
            .collect();
        Self {
            boxed,
            source,
            target,
            authority,
        }
    }
    fn sync(&self, dry_run: bool) -> Reply {
        let mut args = vec![
            "install".into(),
            "--sync".into(),
            "--source-root".into(),
            self.source.as_os_str().into(),
            "--target".into(),
            self.target.as_os_str().into(),
            "--json".into(),
        ];
        if dry_run {
            args.push("--dry-run".into());
        }
        self.boxed.run(&args)
    }
    fn assert_authority_unchanged(&self) {
        for (name, image) in &self.authority {
            let path = self.target.join(name);
            assert_eq!(
                fs::read(&path).unwrap(),
                image.bytes,
                "authority changed: {name}"
            );
            assert_eq!(mode(&path), image.mode, "authority mode changed: {name}");
        }
    }
}
fn legacy_sync_directories(root: &Path) -> BTreeMap<PathBuf, Option<u32>> {
    fn visit(root: &Path, here: &Path, result: &mut BTreeMap<PathBuf, Option<u32>>) {
        result.insert(here.strip_prefix(root).unwrap().to_path_buf(), mode(here));
        for entry in fs::read_dir(here).unwrap() {
            let path = entry.unwrap().path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            assert!(
                !infra_core::fs::linked(&metadata),
                "upgrade created a linked entry"
            );
            if metadata.is_dir() {
                visit(root, &path, result);
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

#[test]
fn schema_one_sync_preserves_client_guard_and_entire_old_runtime_until_review() {
    let _serial = exclusive();
    let fixture = LegacySyncUpgrade::new(true);
    let before = images(&fixture.target);
    let directories = legacy_sync_directories(&fixture.target);
    let home = images(&fixture.boxed.home);
    let codex_home = images(&fixture.boxed.codex_home);
    let planned = fixture.sync(true).expect(0).json();
    assert!(
        images(&fixture.target) == before,
        "dry-run wrote target bytes or modes"
    );
    assert_eq!(legacy_sync_directories(&fixture.target), directories);
    // Exercise real apply before asserting the reported decision: the broken
    // implementation completes migration and silently bypasses the kept guard.
    let applied = fixture.sync(false).expect(0).json();
    let after = images(&fixture.target);
    let changes = before
        .keys()
        .chain(after.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|name| before.get(*name) != after.get(*name))
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        changes.is_empty(),
        "a kept client guard must stop migration before writes: {changes:?}"
    );
    assert_eq!(legacy_sync_directories(&fixture.target), directories);
    for report in [&planned, &applied] {
        assert_eq!(report["partial"], true, "{report}");
        assert_eq!(report["requires_review"], true, "{report}");
        assert_eq!(report["degraded"], true, "{report}");
        assert_eq!(report["changed"], json!([]), "{report}");
        assert!(
            report["held_wiring"]
                .as_array()
                .unwrap()
                .contains(&json!(".claude/settings.json")),
            "{report}"
        );
        assert!(
            report["kept"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["path"] == ".claude/hooks/bash-validator.sh"),
            "{report}"
        );
        // Every file it keeps back names the hook that holds the host and
        // the way out, not only "unchanged host wiring".
        assert!(
            report["kept"].as_array().unwrap().iter().any(|entry| {
                entry["reason"]
                    == "held back by unchanged host wiring (.claude/settings.json): .claude/hooks/bash-validator.sh is edited in the project: move it aside or merge your change into the release, then sync again"
            }),
            "{report}"
        );
    }
    assert_eq!(planned["status"], "planned");
    assert_eq!(applied["status"], "partial");
    assert_eq!(
        fs::read(fixture.target.join(".claude/hooks/bash-validator.sh")).unwrap(),
        LEGACY_SYNC_EDITED_GUARD
    );
    assert_eq!(
        fs::read(fixture.target.join(".claude/settings.json")).unwrap(),
        LEGACY_SYNC_SETTINGS
    );
    assert_eq!(
        fs::read(fixture.target.join("memory-bank/scripts/context.py")).unwrap(),
        LEGACY_SYNC_CONTEXT
    );
    assert!(!fixture.target.join(ENGINE_PATH).exists());
    fixture.assert_authority_unchanged();
    assert!(images(&fixture.boxed.home) == home);
    assert!(images(&fixture.boxed.codex_home) == codex_home);
}

#[test]
fn pristine_schema_one_sync_installs_native_engine_rewires_hooks_and_preserves_authority() {
    let _serial = exclusive();
    let fixture = LegacySyncUpgrade::new(false);
    // The project's own root ignore file has no managed block and never named
    // the engine; the sync keeps it.
    let root_ignore = b"vendor/\n.env\n";
    write(&fixture.target.join(".gitignore"), root_ignore);
    let before = images(&fixture.target);
    let home = images(&fixture.boxed.home);
    let codex_home = images(&fixture.boxed.codex_home);
    let planned = fixture.sync(true).expect(0).json();
    assert_eq!(planned["status"], "planned");
    assert_eq!(planned["partial"], false, "{planned}");
    assert!(
        images(&fixture.target) == before,
        "dry-run wrote the legacy target"
    );
    let applied = fixture.sync(false).expect(0).json();
    assert_eq!(applied["status"], "complete", "{applied}");
    assert_eq!(applied["partial"], false);
    assert!(applied["held_wiring"].as_array().unwrap().is_empty());
    let installed = fixture.target.join(ENGINE_PATH);
    assert_eq!(
        infra_core::query::sha256(&fs::read(&installed).unwrap()),
        infra_core::query::sha256(&fs::read(engine()).unwrap())
    );
    assert!(
        !fixture
            .target
            .join("memory-bank/scripts/context.py")
            .exists()
    );
    let settings: Value =
        serde_json::from_slice(&fs::read(fixture.target.join(".claude/settings.json")).unwrap())
            .unwrap();
    for (event, role) in [
        ("PreToolUse", "bash-validator"),
        ("UserPromptSubmit", "working-memory-read"),
    ] {
        let expected = infra_tooling::attach::HookLaunch::installed(
            &fixture.target,
            &installed,
            cfg!(windows),
        )
        .handler("claude", role)
        .unwrap();
        assert!(
            settings["hooks"][event]
                .as_array()
                .unwrap()
                .iter()
                .any(|group| {
                    group["hooks"].as_array().is_some_and(|handlers| {
                        handlers.iter().any(|handler| {
                            expected
                                .as_object()
                                .unwrap()
                                .iter()
                                .all(|(key, value)| handler[key] == *value)
                        })
                    })
                }),
            "native {role} handler is missing"
        );
    }
    // Only native launches remain. On POSIX they are the generated exec-only
    // launchers named through $CLAUDE_PROJECT_DIR, not the legacy scripts.
    if cfg!(windows) {
        assert!(!settings["hooks"].to_string().contains(".sh"));
    } else {
        assert!(
            !settings["hooks"]
                .to_string()
                .contains("${CLAUDE_PROJECT_DIR}")
        );
        assert_eq!(
            fs::read_to_string(fixture.target.join(".claude/hooks/bash-validator.sh")).unwrap(),
            infra_cli::hooks::shim("claude", "bash-validator", false).unwrap()
        );
    }
    assert!(
        !settings["hooks"]
            .to_string()
            .contains("memory-bank/scripts")
    );
    let record: Value =
        serde_json::from_slice(&fs::read(fixture.target.join(RECORD)).unwrap()).unwrap();
    assert_eq!(record["schema"], 2);
    assert!(
        record["files"]
            .get("memory-bank/scripts/context.py")
            .is_none()
    );
    for name in [
        ENGINE_PATH,
        ".claude/settings.json",
        ".claude/hooks/bash-validator.sh",
    ] {
        assert_eq!(record["files"][name].as_str().unwrap().len(), 64, "{name}");
    }
    // The engine never enters the project's history: its ignore rules are
    // installed beside it.
    assert_eq!(
        fs::read(fixture.target.join(".gitignore")).unwrap(),
        root_ignore
    );
    assert_eq!(
        fs::read_to_string(fixture.target.join("memory-bank/bin/.gitignore")).unwrap(),
        infra_tooling::launch::ENGINE_IGNORE
    );
    let git = |args: &[&str]| {
        infra_core::git::run(
            &fixture.target,
            &infra_core::git::strings(args),
            None,
            Duration::from_secs(15),
            1024 * 1024,
        )
        .unwrap()
    };
    assert_eq!(git(&["init", "--quiet", "--template="]).code, 0);
    assert_eq!(
        git(&["check-ignore", "--quiet", "--", ENGINE_PATH]).code,
        0,
        "the installed engine is not ignored"
    );
    let untracked = git(&["status", "--porcelain", "--untracked-files=all"]).stdout;
    let untracked = String::from_utf8_lossy(&untracked);
    assert!(
        untracked.contains("memory-bank/bin/.gitignore") && !untracked.contains("infra-engine"),
        "{untracked}"
    );
    fixture.assert_authority_unchanged();
    assert!(images(&fixture.boxed.home) == home);
    assert!(images(&fixture.boxed.codex_home) == codex_home);
}

#[test]
fn native_sync_holds_legacy_style_config_for_custom_guard_without_python_retirement() {
    let _serial = exclusive();
    let fixture = LegacySyncUpgrade::new(true);
    // This is the zero-retirement boundary: all native wrappers, the engine and
    // its ignore rules already exist. Only the recorded old-style host
    // configuration would change.
    let installed = fixture.target.join(ENGINE_PATH);
    fs::create_dir_all(installed.parent().unwrap()).unwrap();
    fs::copy(engine(), &installed).unwrap();
    #[cfg(unix)]
    chmod(&installed, 0o755);
    let mut record: Value =
        serde_json::from_slice(&fs::read(fixture.target.join(RECORD)).unwrap()).unwrap();
    let known = record["files"].as_object_mut().unwrap();
    known.remove("memory-bank/scripts/context.py");
    fs::remove_file(fixture.target.join("memory-bank/scripts/context.py")).unwrap();
    known.insert(
        ENGINE_PATH.into(),
        json!(infra_core::query::sha256(&fs::read(&installed).unwrap())),
    );
    let ignore = "memory-bank/bin/.gitignore";
    let bytes = fs::read(fixture.source.join("PHP Core").join(ignore)).unwrap();
    write(&fixture.target.join(ignore), &bytes);
    known.insert(ignore.into(), json!(infra_core::query::sha256(&bytes)));
    for role in LEGACY_SYNC_HOOKS {
        if *role == "bash-validator" {
            continue;
        }
        let path = format!(".claude/hooks/{role}.sh");
        let bytes = fs::read(fixture.source.join("PHP Core").join(&path)).unwrap();
        write(&fixture.target.join(&path), &bytes);
        known.insert(path, json!(infra_core::query::sha256(&bytes)));
    }
    assert!(
        known
            .keys()
            .all(|path| !path.starts_with("memory-bank/scripts/")
                && !path.starts_with("project-brain/scripts/"))
    );
    // Schema 1 and its pristine legacy settings hash remain intentional. Native
    // SHA-256 entries describe the separately installed engine and wrappers.
    assert_eq!(record["schema"], 1);
    write(
        &fixture.target.join(RECORD),
        serde_json::to_vec_pretty(&record).unwrap(),
    );
    let before = images(&fixture.target);
    let directories = legacy_sync_directories(&fixture.target);
    let home = images(&fixture.boxed.home);
    let codex_home = images(&fixture.boxed.codex_home);
    let planned = fixture.sync(true).expect(0).json();
    assert!(
        images(&fixture.target) == before,
        "dry-run changed the native target"
    );
    let applied = fixture.sync(false).expect(0).json();
    let after = images(&fixture.target);
    let changes = before
        .keys()
        .chain(after.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|name| before.get(*name) != after.get(*name))
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        changes.is_empty(),
        "zero-retirement sync bypassed the kept client guard: {changes:?}"
    );
    assert_eq!(legacy_sync_directories(&fixture.target), directories);
    for report in [&planned, &applied] {
        assert_eq!(report["partial"], true, "{report}");
        assert_eq!(report["requires_review"], true, "{report}");
        assert_eq!(report["changed"], json!([]), "{report}");
        assert!(
            report["held_wiring"]
                .as_array()
                .unwrap()
                .contains(&json!(".claude/settings.json")),
            "{report}"
        );
    }
    assert_eq!(applied["status"], "partial");
    assert_eq!(
        fs::read(fixture.target.join(".claude/settings.json")).unwrap(),
        LEGACY_SYNC_SETTINGS
    );
    assert_eq!(
        fs::read(fixture.target.join(".claude/hooks/bash-validator.sh")).unwrap(),
        LEGACY_SYNC_EDITED_GUARD
    );
    fixture.assert_authority_unchanged();
    assert!(images(&fixture.boxed.home) == home);
    assert!(images(&fixture.boxed.codex_home) == codex_home);
}
