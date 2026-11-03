//! Real Linux terminal installation, with private host configuration only.
//! No host/model client is started and no trust/approval file is written.
#![cfg(target_os = "linux")]
#[path = "support/executable.rs"]
mod executable;
use infra_core::domain::uuid4;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{CStr, OsString, c_char, c_int},
    fs::{self, File, OpenOptions},
    io::{IsTerminal, Read},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

unsafe extern "C" {
    fn posix_openpt(flags: c_int) -> c_int;
    fn grantpt(fd: c_int) -> c_int;
    fn unlockpt(fd: c_int) -> c_int;
    fn ptsname_r(fd: c_int, buffer: *mut c_char, size: usize) -> c_int;
    fn fcntl(fd: c_int, command: c_int, ...) -> c_int;
}
const O_RDWR: c_int = 2;
const O_NOCTTY: c_int = 0x100;
const O_NONBLOCK: c_int = 0x800;
const O_CLOEXEC: c_int = 0x80000;
const F_GETFL: c_int = 3;
const F_SETFL: c_int = 4;
const LIMIT: u64 = 16 * 1024 * 1024;

struct Pty {
    master: File,
    slave: Option<File>,
}
impl Pty {
    fn new() -> Self {
        // The PTY is never the test process's controlling terminal.
        let fd = unsafe { posix_openpt(O_RDWR | O_NOCTTY | O_CLOEXEC) };
        assert!(fd >= 0, "posix_openpt: {}", std::io::Error::last_os_error());
        let master = unsafe { File::from_raw_fd(fd) };
        assert_eq!(unsafe { grantpt(fd) }, 0);
        assert_eq!(unsafe { unlockpt(fd) }, 0);
        let mut name = [0 as c_char; 4096];
        assert_eq!(unsafe { ptsname_r(fd, name.as_mut_ptr(), name.len()) }, 0);
        let name = unsafe { CStr::from_ptr(name.as_ptr()) }.to_str().unwrap();
        let slave = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(O_NOCTTY | O_CLOEXEC)
            .open(name)
            .unwrap();
        assert!(slave.is_terminal(), "the fixture must supply a real TTY");
        let flags = unsafe { fcntl(master.as_raw_fd(), F_GETFL) };
        assert!(flags >= 0);
        assert_eq!(
            unsafe { fcntl(master.as_raw_fd(), F_SETFL, flags | O_NONBLOCK) },
            0
        );
        Self {
            master,
            slave: Some(slave),
        }
    }
    fn stdio(&self) -> Stdio {
        Stdio::from(self.slave.as_ref().unwrap().try_clone().unwrap())
    }
    fn drain(&mut self, bytes: &mut Vec<u8>) {
        loop {
            let mut chunk = [0; 8192];
            match self.master.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => {
                    assert!((bytes.len() as u64).saturating_add(count as u64) <= LIMIT);
                    bytes.extend_from_slice(&chunk[..count]);
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.raw_os_error() == Some(5) =>
                {
                    break;
                } // Linux PTY slave closed.
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => panic!("PTY read: {error}"),
            }
        }
    }
}
struct Reply {
    code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
impl Reply {
    fn success(self) -> Self {
        assert_eq!(
            self.code,
            0,
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&self.stdout),
            String::from_utf8_lossy(&self.stderr)
        );
        self
    }
    fn json(&self) -> Value {
        serde_json::from_slice(&self.stdout)
            .unwrap_or_else(|error| panic!("stdout must remain separately captured JSON: {error}"))
    }
}
fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}
fn tree(root: &Path) -> BTreeMap<PathBuf, (Vec<u8>, u32)> {
    fn walk(root: &Path, here: &Path, result: &mut BTreeMap<PathBuf, (Vec<u8>, u32)>) {
        if !here.exists() {
            return;
        }
        for entry in fs::read_dir(here).unwrap() {
            let path = entry.unwrap().path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            assert!(!metadata.file_type().is_symlink());
            if metadata.is_dir() {
                walk(root, &path, result);
            } else {
                assert!(metadata.is_file());
                result.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    (
                        fs::read(path).unwrap(),
                        metadata.permissions().mode() & 0o777,
                    ),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    walk(root, root, &mut result);
    result
}
struct Fixture {
    root: PathBuf,
    source: PathBuf,
    home: PathBuf,
    claude: PathBuf,
    codex: PathBuf,
    binary: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("infra-install-tty-{}", uuid4()));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let source = root.join("isolated release");
        let home = root.join("private HOME");
        let claude = root.join("private Claude configuration");
        let codex = root.join("private Codex configuration");
        for directory in [&source, &home, &claude, &codex, &root.join("tmp")] {
            fs::DirBuilder::new().mode(0o700).create(directory).unwrap();
        }
        write(&claude.join("settings.json"), serde_json::to_vec(&json!({
            "fixture":"preserve Claude settings",
            "hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"fixture-user-hook"}]}]}
        })).unwrap());
        write(&codex.join("config.toml"), "[test_fixture]\nkeep = true\n");
        write(&codex.join("hooks.json"), serde_json::to_vec(&json!({
            "fixture":"preserve Codex settings",
            "hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"fixture-user-hook"}]}]}
        })).unwrap());
        // The source image is deliberately hardlinked, independently of Cargo's
        // own links. This is an executable source, never a data-file exception.
        let binary = root.join("selected native engine");
        executable::copy(env!("CARGO_BIN_EXE_infra-engine"), &binary);
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        fs::hard_link(&binary, root.join("same selected native image")).unwrap();
        assert_eq!(fs::metadata(&binary).unwrap().nlink(), 2);
        let repo =
            fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")).unwrap();
        let mut inventory: Value = serde_json::from_slice(
            &fs::read(repo.join("install/inventories/php-core.json")).unwrap(),
        )
        .unwrap();
        let wanted = [
            ".gitattributes",
            ".gitignore",
            "AGENTS.md",
            "README.md",
            "VERSION",
            "memory-bank/INDEX.md",
            "project-brain/config/runtime.json",
            "tasks/.task-counter",
            ".claude/settings.json",
            ".claude/CLAUDE.md",
            ".claude/hooks/local-context.sh",
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
            "use actual declared native release resources"
        );
        inventory["source_overrides"]
            .as_object_mut()
            .unwrap()
            .retain(|name, _| selected.contains(name));
        for name in &selected {
            let origin = inventory["source_overrides"][name].as_str().unwrap_or(name);
            let target = source.join("PHP Core").join(origin);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(repo.join("PHP Core").join(origin), target).unwrap();
        }
        write(
            &source.join("install/inventories/php-core.json"),
            serde_json::to_vec_pretty(&inventory).unwrap(),
        );
        write(
            &source.join("install/native-payload.json"),
            fs::read(repo.join("install/native-payload.json")).unwrap(),
        );
        Self {
            root,
            source,
            home,
            claude,
            codex,
            binary,
        }
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "The isolated driver varies stdin and stderr terminal membership independently of command, cwd and input."
    )]
    fn run(
        &self,
        program: &Path,
        args: &[OsString],
        cwd: &Path,
        tty_in: bool,
        tty_err: bool,
        project: Option<&Path>,
        input: &[u8],
    ) -> Reply {
        let id = uuid4();
        let stdout_path = self.root.join(format!("{id}.stdout"));
        let stderr_path = self.root.join(format!("{id}.stderr"));
        let input_path = self.root.join(format!("{id}.input"));
        write(&input_path, input);
        let mut pty = (tty_in || tty_err).then(Pty::new);
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(cwd)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CLAUDE_CONFIG_DIR", &self.claude)
            .env("CODEX_HOME", &self.codex)
            .env("XDG_CONFIG_HOME", self.home.join("config"))
            .env("XDG_DATA_HOME", self.home.join("data"))
            .env("XDG_STATE_HOME", self.home.join("state"))
            .env("APPDATA", self.home.join("appdata"))
            .env("LOCALAPPDATA", self.home.join("localappdata"))
            .env("TMPDIR", self.root.join("tmp"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.home.join("unused-gitconfig"))
            .env("LC_ALL", "C")
            .stdin(if tty_in {
                pty.as_ref().unwrap().stdio()
            } else {
                Stdio::from(File::open(&input_path).unwrap())
            })
            .stdout(File::create(&stdout_path).unwrap())
            .stderr(if tty_err {
                pty.as_ref().unwrap().stdio()
            } else {
                Stdio::from(File::create(&stderr_path).unwrap())
            });
        if let Some(project) = project {
            command.env("CLAUDE_PROJECT_DIR", project);
        }
        let mut child = command.spawn().unwrap();
        drop(command);
        if let Some(pty) = pty.as_mut() {
            drop(pty.slave.take());
        }
        let mut terminal_output = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(60);
        let status = loop {
            if let Some(pty) = pty.as_mut() {
                pty.drain(&mut terminal_output);
            }
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("isolated native installer exceeded 60 seconds: {args:?}");
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        if let Some(pty) = pty.as_mut() {
            pty.drain(&mut terminal_output);
        }
        fn bounded(path: &Path) -> Vec<u8> {
            let mut bytes = Vec::new();
            File::open(path)
                .unwrap()
                .take(LIMIT + 1)
                .read_to_end(&mut bytes)
                .unwrap();
            assert!(bytes.len() as u64 <= LIMIT);
            bytes
        }
        Reply {
            code: status.code().unwrap_or(-1),
            stdout: bounded(&stdout_path),
            stderr: if tty_err {
                terminal_output
            } else {
                bounded(&stderr_path)
            },
        }
    }
    fn args(&self, target: &Path, tools: &str, extra: &[&str]) -> Vec<OsString> {
        let mut args = vec![
            "install".into(),
            "--source-root".into(),
            self.source.as_os_str().into(),
            "--target".into(),
            target.as_os_str().into(),
            "--edition".into(),
            "PHP Core".into(),
            "--tool".into(),
            tools.into(),
            "--json".into(),
        ];
        args.extend(extra.iter().map(OsString::from));
        args
    }
    fn install(
        &self,
        target: &Path,
        tools: &str,
        extra: &[&str],
        input_tty: bool,
        error_tty: bool,
    ) -> Reply {
        self.run(
            &self.binary,
            &self.args(target, tools, extra),
            &self.root,
            input_tty,
            error_tty,
            None,
            b"",
        )
    }
    fn assert_registered(&self, host: &str, target: &Path) -> PathBuf {
        let config = if host == "claude" {
            &self.claude
        } else {
            &self.codex
        };
        let folder = config.join("ai-accelerator");
        let listed = fs::read_to_string(folder.join("projects")).unwrap();
        assert_eq!(
            listed.lines().collect::<Vec<_>>(),
            [target.to_str().unwrap()]
        );
        let image = fs::read(folder.join("infra-engine")).unwrap();
        let expected = infra_core::query::sha256(&fs::read(&self.binary).unwrap());
        assert_eq!(infra_core::query::sha256(&image), expected);
        let metadata: Value =
            serde_json::from_slice(&fs::read(folder.join("native-dispatch.json")).unwrap())
                .unwrap();
        assert_eq!(metadata["sha256"], expected);
        let settings: Value = serde_json::from_slice(
            &fs::read(config.join(if host == "claude" {
                "settings.json"
            } else {
                "hooks.json"
            }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            settings["fixture"],
            if host == "claude" {
                "preserve Claude settings"
            } else {
                "preserve Codex settings"
            }
        );
        assert!(
            settings["hooks"]["SessionStart"]
                .as_array()
                .unwrap()
                .iter()
                .any(|group| group["hooks"].as_array().is_some_and(|hooks| hooks
                    .iter()
                    .any(|hook| hook["command"] == "fixture-user-hook")))
        );
        assert!(settings["hooks"].to_string().contains("infra-engine"));
        folder
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn tty_install_registers_selected_native_dispatchers_before_first_subdirectory_session() {
    let f = Fixture::new();
    let target = f.root.join("first installed project");
    let codex_config = fs::read(f.codex.join("config.toml")).unwrap();
    let reply = f
        .install(&target, "claude,codex", &[], true, true)
        .success();
    assert_eq!(reply.json()["status"], "complete");
    let errors = String::from_utf8_lossy(&reply.stderr);
    assert!(errors.contains("CLAUDE\tbelow-root\t"));
    assert!(errors.contains("CODEX\tworktrees\t"));
    let claude = f.assert_registered("claude", &target);
    f.assert_registered("codex", &target);
    assert_eq!(fs::read(f.codex.join("config.toml")).unwrap(), codex_config);
    // All Codex additions are dispatcher wiring; hook approval/trust records
    // and client session databases are not created.
    let names = tree(&f.codex).into_keys().collect::<BTreeSet<_>>();
    assert!(names.iter().all(|path| path == Path::new("config.toml")
        || path == Path::new("hooks.json")
        || path.starts_with("ai-accelerator")));
    assert!(
        names.iter().all(|path| {
            let name = path.to_string_lossy().to_ascii_lowercase();
            !name.contains("trust") && !name.contains("approval")
        }),
        "dispatcher registration must not create hook trust/approval records"
    );
    let subdir = target.join("src/deeper");
    fs::create_dir_all(&subdir).unwrap();
    // No root SessionStart has run: dispatch policy directly from the newly
    // registered global native image, as the host does below the project root.
    let args = ["dispatch", "claude", "--store"]
        .into_iter()
        .map(OsString::from)
        .chain([claude.as_os_str().into(), "policy-1".into()])
        .collect::<Vec<_>>();
    let policy = f
        .run(
            &claude.join("infra-engine"),
            &args,
            &subdir,
            false,
            false,
            Some(&subdir),
            b"{}",
        )
        .success();
    assert!(
        String::from_utf8_lossy(&policy.stdout).starts_with(&format!(
            "Contents of {} (the project's policy), part 1 of ",
            target.join("AGENTS.md").display()
        ))
    );
    assert!(!f.home.join(".claude.json").exists());
    assert!(
        !f.claude.join(".claude.json").exists(),
        "import approvals stay the person's decision"
    );
}

#[test]
fn terminal_registration_requires_both_ttys_and_never_runs_for_dry_run() {
    let f = Fixture::new();
    let claude = tree(&f.claude);
    let codex = tree(&f.codex);
    for (label, input, error) in [
        ("script", false, false),
        ("stdin-only", true, false),
        ("stderr-only", false, true),
    ] {
        let target = f.root.join(label);
        let reply = f
            .install(&target, "claude,codex", &[], input, error)
            .success();
        assert_eq!(reply.json()["status"], "complete");
        assert!(!String::from_utf8_lossy(&reply.stderr).contains("below-root"));
        assert_eq!(tree(&f.claude), claude);
        assert_eq!(tree(&f.codex), codex);
    }
    let target = f.root.join("dry target");
    let dry = f
        .install(&target, "claude,codex", &["--dry-run"], true, true)
        .success();
    assert_eq!(dry.json()["status"], "planned");
    assert!(!target.exists());
    assert_eq!(tree(&f.claude), claude);
    assert_eq!(tree(&f.codex), codex);
}

#[test]
fn tty_registration_respects_selected_tools_existing_configuration_and_opt_out() {
    let f = Fixture::new();
    let codex = tree(&f.codex);
    let selected = f.root.join("Claude only");
    f.install(&selected, "claude", &[], true, true).success();
    f.assert_registered("claude", &selected);
    assert_eq!(
        tree(&f.codex),
        codex,
        "unselected Codex configuration is untouched"
    );

    let missing = Fixture::new();
    fs::remove_dir_all(&missing.claude).unwrap();
    let target = missing.root.join("no Claude configuration");
    missing
        .install(&target, "claude", &[], true, true)
        .success();
    assert!(
        !missing.claude.exists(),
        "installer never creates a host configuration root"
    );

    let disabled = Fixture::new();
    write(
        &disabled.claude.join("ai-accelerator/off"),
        b"preserve opt-out\n",
    );
    write(
        &disabled.codex.join("ai-accelerator/off"),
        b"preserve opt-out\n",
    );
    let claude = tree(&disabled.claude);
    let codex = tree(&disabled.codex);
    let target = disabled.root.join("disabled dispatch");
    disabled
        .install(&target, "claude,codex", &[], true, true)
        .success();
    assert_eq!(tree(&disabled.claude), claude);
    assert_eq!(tree(&disabled.codex), codex);
}

#[test]
fn explicit_dispatch_install_captures_only_the_hardlinked_running_executable_source() {
    let f = Fixture::new();
    let target = f.root.join("explicit registration");
    f.install(&target, "claude", &[], false, false).success();
    assert!(!f.claude.join("ai-accelerator").exists());
    let args = vec![
        "dispatch-install".into(),
        "claude".into(),
        "--root".into(),
        target.as_os_str().into(),
    ];
    let reply = f
        .run(&f.binary, &args, &f.root, false, false, None, b"")
        .success();
    assert_eq!(reply.json()["state"], "registered");
    f.assert_registered("claude", &target);
    assert_eq!(fs::metadata(&f.binary).unwrap().nlink(), 2);
    let mut strict_data = infra_core::bank::publication::PublicationFiles::new(&f.root).unwrap();
    assert!(
        strict_data
            .read("selected native engine", 512 * 1024 * 1024)
            .is_err(),
        "the strict publication data reader must retain its one-link refusal"
    );
}
