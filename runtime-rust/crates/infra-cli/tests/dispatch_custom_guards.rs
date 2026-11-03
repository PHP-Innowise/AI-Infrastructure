//! Global dispatcher updates preserve hooks explicitly declared by another project.
//! Legacy context.py files are inert markers: no old Python runtime is executed.
#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::{self, File},
    io::Read,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
const ENGINE: &str = "memory-bank/bin/infra-engine";
const GUARD_OUT: &[u8] = b"{\"hookSpecificOutput\":{\"hookEventName\":\"PreToolUse\",\"permissionDecision\":\"deny\",\"permissionDecisionReason\":\"private fixture guard\"}}\n";
const GUARD_ERR: &[u8] = b"fixture custom guard diagnostic\n";
fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}
struct Reply {
    code: i32,
    out: Vec<u8>,
    err: Vec<u8>,
}
impl Reply {
    fn success(self) -> Self {
        assert_eq!(
            self.code,
            0,
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&self.out),
            String::from_utf8_lossy(&self.err)
        );
        self
    }
    fn json(&self) -> Value {
        serde_json::from_slice(&self.out).unwrap()
    }
}
struct Fixture {
    root: PathBuf,
    home: PathBuf,
    claude: PathBuf,
    codex: PathBuf,
    binary: PathBuf,
    input_witness: PathBuf,
    environment_witness: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "infra-dispatch-custom-{}",
            infra_core::domain::uuid4()
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let home = root.join("private HOME");
        let claude = root.join("private Claude");
        let codex = root.join("private Codex");
        for path in [&home, &claude, &codex, &root.join("tmp")] {
            fs::DirBuilder::new().mode(0o700).create(path).unwrap();
        }
        write(
            &codex.join("config.toml"),
            "[private_fixture]\nkeep = true\n",
        );
        write(
            &claude.join("settings.json"),
            b"{\"fixture\":\"preserve user Claude settings\",\"hooks\":{}}",
        );
        write(
            &codex.join("hooks.json"),
            b"{\"fixture\":\"preserve user Codex settings\",\"hooks\":{}}",
        );
        let binary = root.join("selected infra-engine");
        fs::copy(env!("CARGO_BIN_EXE_infra-engine"), &binary).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            input_witness: root.join("guard.input"),
            environment_witness: root.join("guard.environment"),
            root,
            home,
            claude,
            codex,
            binary,
        }
    }
    fn call(
        &self,
        program: &Path,
        args: &[OsString],
        cwd: &Path,
        host: Option<&str>,
        payload: &[u8],
    ) -> Reply {
        let id = infra_core::domain::uuid4();
        let input = self.root.join(format!("{id}.input"));
        let out = self.root.join(format!("{id}.out"));
        let err = self.root.join(format!("{id}.err"));
        write(&input, payload);
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
            .env("TMPDIR", self.root.join("tmp"))
            .env("XDG_CONFIG_HOME", self.home.join("config"))
            .env("XDG_DATA_HOME", self.home.join("data"))
            .env("XDG_STATE_HOME", self.home.join("state"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.home.join("unused-gitconfig"))
            .env("LC_ALL", "C")
            .env("INFRA_DISPATCH_GUARD_INPUT", &self.input_witness)
            .env("INFRA_DISPATCH_GUARD_ENV", &self.environment_witness)
            .stdin(File::open(&input).unwrap())
            .stdout(File::create(&out).unwrap())
            .stderr(File::create(&err).unwrap());
        if host == Some("claude") {
            command.env("CLAUDE_PROJECT_DIR", cwd);
        }
        let mut child = command.spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(40);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("private dispatcher command exceeded 40 seconds: {args:?}");
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        fn read(path: &Path) -> Vec<u8> {
            let mut bytes = Vec::new();
            File::open(path)
                .unwrap()
                .take(4 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .unwrap();
            assert!(bytes.len() <= 4 * 1024 * 1024);
            bytes
        }
        Reply {
            code: status.code().unwrap_or(-1),
            out: read(&out),
            err: read(&err),
        }
    }
    fn git(&self, cwd: &Path, args: &[OsString]) {
        self.call(Path::new("git"), args, cwd, None, b"").success();
    }
    fn store(&self, host: &str) -> PathBuf {
        if host == "claude" {
            self.claude.join("ai-accelerator")
        } else {
            self.codex.join("ai-accelerator")
        }
    }
    fn config(root: &Path, host: &str) -> PathBuf {
        root.join(if host == "claude" {
            ".claude/settings.json"
        } else {
            ".codex/hooks.json"
        })
    }
    fn legacy_command(host: &str, name: &str) -> String {
        if host == "claude" {
            format!("\"{}{}\"/.claude/hooks/{name}", '$', "{CLAUDE_PROJECT_DIR}")
        } else {
            format!(
                "sh -c 'd=$(pwd); while [ ! -f \"$d/memory-bank/scripts/context.py\" ]; do [ \"$d\" = / ] && exit 0; d=$(dirname \"$d\"); done; exec bash \"$d/.codex/hooks/{name}\"'"
            )
        }
    }
    fn script(root: &Path, host: &str, name: &str) -> PathBuf {
        root.join(if host == "claude" {
            ".claude/hooks"
        } else {
            ".codex/hooks"
        })
        .join(name)
    }
    fn guard(&self, root: &Path, host: &str, name: &str) {
        let path = Self::script(root, host, name);
        write(
            &path,
            concat!(
                "#!/bin/sh\n",
                "cat > \"$INFRA_DISPATCH_GUARD_INPUT\"\n",
                "printf '%s\\n' \"$PWD\" \"$CLAUDE_PROJECT_DIR\" \"$ACCELERATOR_HOME\" ",
                "\"$ACCELERATOR_STATE_DIR\" \"$ACCELERATOR_PROJECT_DIR\" > \"$INFRA_DISPATCH_GUARD_ENV\"\n",
                "printf '%s\\n' '{\"hookSpecificOutput\":{\"hookEventName\":\"PreToolUse\",\"permissionDecision\":\"deny\",\"permissionDecisionReason\":\"private fixture guard\"}}'\n",
                "printf '%s\\n' 'fixture custom guard diagnostic' >&2\n",
                "exit 73\n",
            ),
        );
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fn legacy(&self, host: &str, name: &str) -> PathBuf {
        let root = self.root.join(format!("legacy {host} project"));
        write(
            &root.join("memory-bank/scripts/context.py"),
            b"# inert legacy installation marker: never executed\n",
        );
        write(
            &root.join("project-brain/config/runtime.json"),
            b"{\"mode\":\"governed\",\"framework\":\"core\"}\n",
        );
        write(&root.join("README.md"), b"private project\n");
        write(&root.join("src/keep.txt"), b"tracked worktree content\n");
        let command = Self::legacy_command(host, name);
        write(&Self::config(&root, host), serde_json::to_vec(&json!({
            "hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":command,"timeout":7}]}]}
        })).unwrap());
        self.guard(&root, host, name);
        root
    }
    fn native(&self, host: &str) -> PathBuf {
        let root = self.root.join(format!("new native {host} project"));
        write(
            &root.join("project-brain/config/runtime.json"),
            b"{\"mode\":\"governed\",\"framework\":\"core\"}\n",
        );
        let engine = root.join(ENGINE);
        fs::create_dir_all(engine.parent().unwrap()).unwrap();
        fs::copy(&self.binary, &engine).unwrap();
        let mut handler =
            infra_tooling::attach::hook_handler(&engine, &root, host, "bash-validator", false)
                .unwrap();
        handler["type"] = "command".into();
        handler["timeout"] = 7.into();
        write(
            &Self::config(&root, host),
            serde_json::to_vec(&json!({
                "hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[handler]}]}
            }))
            .unwrap(),
        );
        root
    }
    fn register(&self, root: &Path, host: &str) -> Value {
        let args = vec![
            "dispatch-install".into(),
            host.into(),
            "--root".into(),
            root.as_os_str().into(),
        ];
        self.call(&self.binary, &args, &self.root, None, b"")
            .success()
            .json()
    }
    fn update_other_project(&self, legacy: &Path, host: &str) -> PathBuf {
        // Emulate a previously registered root without executing the old
        // dispatcher. A real public registration of another project updates it.
        let folder = self.store(host);
        write(&folder.join("projects"), format!("{}\n", legacy.display()));
        let native = self.native(host);
        assert_eq!(self.register(&native, host)["state"], "registered");
        let listed = fs::read_to_string(folder.join("projects")).unwrap();
        assert!(listed.lines().any(|line| line == legacy.to_str().unwrap()));
        assert!(folder.join("native-dispatch.json").is_file());
        folder
    }
    fn dispatch(
        &self,
        folder: &Path,
        start: &Path,
        host: &str,
        name: &str,
        payload: &[u8],
    ) -> Reply {
        let args = vec![
            "dispatch".into(),
            host.into(),
            "--store".into(),
            folder.as_os_str().into(),
            name.into(),
        ];
        self.call(
            &folder.join("infra-engine"),
            &args,
            start,
            Some(host),
            payload,
        )
    }
    fn input(start: &Path) -> Vec<u8> {
        serde_json::to_vec(&json!({"hook_event_name":"PreToolUse","tool_name":"Bash",
            "tool_input":{"command":"printf harmless-fixture"},
            "session_id":"private-dispatch-test","cwd":start}))
        .unwrap()
    }
    fn assert_guard(
        &self,
        reply: Reply,
        payload: &[u8],
        cwd: &Path,
        claude: Option<&Path>,
        authority: Option<&Path>,
    ) {
        assert_eq!(
            reply.code,
            73,
            "dispatcher replaced the declared guard; stdout={} stderr={}",
            String::from_utf8_lossy(&reply.out),
            String::from_utf8_lossy(&reply.err)
        );
        assert_eq!(reply.out, GUARD_OUT);
        assert_eq!(reply.err, GUARD_ERR);
        assert_eq!(fs::read(&self.input_witness).unwrap(), payload);
        let expected = [
            Some(cwd),
            claude,
            authority,
            authority,
            authority.map(|_| cwd),
        ]
        .map(|path| path.map(|path| path.to_str().unwrap()).unwrap_or(""))
        .join("\n")
            + "\n";
        assert_eq!(
            fs::read_to_string(&self.environment_witness).unwrap(),
            expected
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn authority(root: &Path, host: &str, name: &str) -> BTreeMap<PathBuf, Vec<u8>> {
    [
        root.join("memory-bank/scripts/context.py"),
        Fixture::config(root, host),
        Fixture::script(root, host, name),
    ]
    .into_iter()
    .map(|path| {
        let bytes = fs::read(&path).unwrap();
        (path, bytes)
    })
    .collect()
}

#[test]
fn global_update_preserves_another_legacy_claude_custom_guard_below_its_root() {
    let f = Fixture::new();
    let legacy = f.legacy("claude", "bash-validator.sh");
    let before = authority(&legacy, "claude", "bash-validator.sh");
    let folder = f.update_other_project(&legacy, "claude");
    assert_eq!(authority(&legacy, "claude", "bash-validator.sh"), before);
    let start = legacy.join("src");
    let payload = Fixture::input(&start);
    let reply = f.dispatch(&folder, &start, "claude", "bash-validator.sh", &payload);
    f.assert_guard(reply, &payload, &legacy, Some(&legacy), None);
    assert_eq!(authority(&legacy, "claude", "bash-validator.sh"), before);
}
#[test]
fn global_update_preserves_a_codex_custom_guard_and_its_served_worktree_scope() {
    let f = Fixture::new();
    let legacy = f.legacy("codex", "bash-validator.sh");
    f.git(
        &legacy,
        &["-c", "init.templateDir=", "init", "-q", "-b", "main"].map(OsString::from),
    );
    f.git(
        &legacy,
        &["add", "--", "README.md", "src/keep.txt"].map(OsString::from),
    );
    f.git(
        &legacy,
        &[
            "-c",
            "user.name=Private Fixture",
            "-c",
            "user.email=fixture@invalid",
            "commit",
            "-q",
            "-m",
            "private fixture",
        ]
        .map(OsString::from),
    );
    let served = f.root.join("served worktree");
    f.git(
        &legacy,
        &[
            "worktree".into(),
            "add".into(),
            "-q".into(),
            "-b".into(),
            "served".into(),
            "--".into(),
            served.as_os_str().into(),
            "HEAD".into(),
        ],
    );
    let before = authority(&legacy, "codex", "bash-validator.sh");
    let folder = f.update_other_project(&legacy, "codex");
    let start = served.join("src");
    let payload = Fixture::input(&start);
    let reply = f.dispatch(&folder, &start, "codex", "bash-validator.sh", &payload);
    f.assert_guard(reply, &payload, &served, None, Some(&legacy));
    assert_eq!(authority(&legacy, "codex", "bash-validator.sh"), before);
    assert!(!served.join("memory-bank").exists());
}
#[test]
fn exact_native_declaration_does_not_execute_a_decoy_legacy_role_script() {
    let f = Fixture::new();
    let native = f.native("claude");
    f.guard(&native, "claude", "bash-validator.sh");
    let folder = f.store("claude");
    assert_eq!(f.register(&native, "claude")["state"], "registered");
    let start = native.join("src");
    fs::create_dir_all(&start).unwrap();
    let payload = Fixture::input(&start);
    let reply = f
        .dispatch(&folder, &start, "claude", "bash-validator.sh", &payload)
        .success();
    assert!(reply.out.is_empty());
    assert!(!f.input_witness.exists());
    assert!(!f.environment_witness.exists());
}
#[test]
fn registration_and_dispatch_preserve_a_safe_custom_named_project_guard() {
    let mut observed = Vec::new();
    for name in ["team-policy.sh", "policy-1.sh"] {
        let f = Fixture::new();
        let legacy = f.legacy("claude", name);
        write(
            &legacy.join("AGENTS.md"),
            b"# Private fixture policy witness\n",
        );
        write(&legacy.join(".claude/CLAUDE.md"), b"@../AGENTS.md\n");
        let before = authority(&legacy, "claude", name);
        let folder = f.update_other_project(&legacy, "claude");
        assert_eq!(f.register(&legacy, "claude")["state"], "registered");
        let settings: Value =
            serde_json::from_slice(&fs::read(f.claude.join("settings.json")).unwrap()).unwrap();
        let registered = settings["hooks"]["PreToolUse"].to_string().contains(name);
        let start = legacy.join("src");
        let payload = Fixture::input(&start);
        // The bare generated token remains internal policy delivery, distinct
        // from the explicitly declared script whose name happens to be similar.
        let bare = f
            .dispatch(&folder, &start, "claude", "policy-1", &payload)
            .success();
        assert!(String::from_utf8_lossy(&bare.out).contains("# Private fixture policy witness"));
        assert!(!f.input_witness.exists() && !f.environment_witness.exists());
        // Run BOTH script names before assertions so the BEFORE implementation
        // actually reaches the prefix collision as well as generic named routing.
        let reply = f.dispatch(&folder, &start, "claude", name, &payload);
        eprintln!(
            "declared custom hook {name}: registered={registered}, code={}",
            reply.code
        );
        observed.push((f, legacy, before, payload, reply, registered, name));
    }
    for (f, legacy, before, payload, reply, registered, name) in observed {
        f.assert_guard(reply, &payload, &legacy, Some(&legacy), None);
        assert!(
            registered,
            "declared custom hook {name} was silently omitted from registration"
        );
        assert_eq!(authority(&legacy, "claude", name), before);
        assert_eq!(
            fs::read(legacy.join("AGENTS.md")).unwrap(),
            b"# Private fixture policy witness\n"
        );
        assert_eq!(
            fs::read(legacy.join(".claude/CLAUDE.md")).unwrap(),
            b"@../AGENTS.md\n"
        );
    }
}

#[test]
fn mixed_native_and_legacy_declarations_for_the_same_hook_require_explicit_review() {
    let f = Fixture::new();
    let native = f.native("claude");
    f.guard(&native, "claude", "bash-validator.sh");
    let folder = f.store("claude");
    assert_eq!(f.register(&native, "claude")["state"], "registered");
    let config_path = Fixture::config(&native, "claude");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["hooks"]["PreToolUse"][0]["hooks"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "type":"command",
            "command":Fixture::legacy_command("claude", "bash-validator.sh"),
            "timeout":7,
        }));
    write(&config_path, serde_json::to_vec(&config).unwrap());
    let config_before = fs::read(&config_path).unwrap();
    let script = Fixture::script(&native, "claude", "bash-validator.sh");
    let script_before = fs::read(&script).unwrap();
    let start = native.join("src");
    fs::create_dir_all(&start).unwrap();
    let payload = Fixture::input(&start);
    let reply = f.dispatch(&folder, &start, "claude", "bash-validator.sh", &payload);
    assert_eq!(
        reply.code, 2,
        "PreToolUse ambiguity must fail closed, not return a nonblocking generic CLI error"
    );
    let diagnostic = String::from_utf8_lossy(&reply.err).to_ascii_lowercase();
    assert!(
        diagnostic.contains("ambiguous") && diagnostic.contains("review"),
        "mixed launch authority needs an explicit review diagnostic: {diagnostic}"
    );
    assert!(!f.input_witness.exists());
    assert!(!f.environment_witness.exists());
    assert_eq!(fs::read(config_path).unwrap(), config_before);
    assert_eq!(fs::read(script).unwrap(), script_before);
}
