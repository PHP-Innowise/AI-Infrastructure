//! A committed installation works in every clone: the wiring names no path of
//! the installing machine, a clone without its own engine refuses shell
//! commands until the engine is installed there, a teammate's install only adds
//! the engine, and a linked worktree uses its main checkout's engine.
#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const INHERITED: [&str; 10] = [
    "ACCELERATOR_HOME",
    "ACCELERATOR_STATE_DIR",
    "ACCELERATOR_PROJECT_DIR",
    "CLAUDE_PROJECT_DIR",
    "CONTEXT_TASK_ID",
    "CONTEXT_HOOK_BUDGET",
    "CONTEXT_MEMORY_RECOVERY",
    "CONTEXT_CAPSULE_DELIVERED",
    "PROJECT_BRAIN_MODE",
    "INFRA_SOURCE_ROOT",
];
const WIRING: [&str; 6] = [
    ".claude/settings.json",
    ".codex/hooks.json",
    ".cursor/hooks.json",
    ".mcp.json",
    ".cursor/mcp.json",
    ".codex/config.toml",
];

struct Fixture {
    root: PathBuf,
    home: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system calendar")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "infra-portable-wiring-{}-{nonce}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("create private test root");
        let root = fs::canonicalize(root).expect("resolve private test root");
        assert!(
            !root
                .ancestors()
                .any(|parent| parent.join("runtime-rust/Cargo.toml").is_file()),
            "fixture must be outside the source checkout"
        );
        let home = root.join("home");
        fs::create_dir(&home).unwrap();
        Self { root, home }
    }

    fn command(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        command
            .env("HOME", &self.home)
            .env("CODEX_HOME", self.home.join("codex"))
            .env("CLAUDE_CONFIG_DIR", self.home.join("claude"))
            .env("XDG_CONFIG_HOME", self.home.join("config"))
            .env("XDG_DATA_HOME", self.home.join("data"))
            .env("XDG_STATE_HOME", self.home.join("state"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.home.join("gitconfig"))
            .env("GIT_AUTHOR_NAME", "Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .stdin(Stdio::null());
        for name in INHERITED {
            command.env_remove(name);
        }
        command
    }

    fn git(&self, cwd: &Path, args: &[&str]) {
        let output = self
            .command("git")
            .args(["-c", "init.templateDir=", "-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            text(&output.stderr)
        );
    }

    fn install(&self, target: &Path, dry_run: bool) -> Value {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let mut command = self.command(env!("CARGO_BIN_EXE_infra-engine"));
        command
            .args([
                "install",
                "--edition",
                "PHP Core",
                "--merge-existing",
                "--target",
            ])
            .arg(target)
            .arg("--source-root")
            .arg(fs::canonicalize(source).unwrap())
            .current_dir(&self.root);
        if dry_run {
            command.arg("--dry-run");
        }
        let output = command.output().unwrap();
        serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
            panic!(
                "install output: {}{}",
                text(&output.stdout),
                text(&output.stderr)
            )
        })
    }

    /// Run one committed hook command through a shell, as each host does
    /// (Codex through the login shell, in the session directory).
    fn hook(
        &self,
        host: &str,
        project: &Path,
        cwd: &Path,
        command: &str,
        payload: &Value,
    ) -> Output {
        let mut process = self.command("sh");
        process.args(["-c", command]);
        if host == "claude" {
            process.env("CLAUDE_PROJECT_DIR", project);
        }
        let mut child = process
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(payload).unwrap())
            .unwrap();
        child.wait_with_output().unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn json_file(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

/// The committed command for a role on the host's shell-guard event.
fn guard_command(project: &Path, host: &str) -> String {
    match host {
        "claude" => json_file(&project.join(".claude/settings.json"))["hooks"]["PreToolUse"]
            .as_array()
            .unwrap()
            .iter()
            .find(|group| group["matcher"] == "Bash")
            .unwrap()["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .into(),
        "codex" => json_file(&project.join(".codex/hooks.json"))["hooks"]["PreToolUse"][0]["hooks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|hook| hook["command"].as_str().unwrap())
            .find(|command| command.contains("bash-validator"))
            .unwrap()
            .into(),
        _ => json_file(&project.join(".cursor/hooks.json"))["hooks"]["beforeShellExecution"][0]
            ["command"]
            .as_str()
            .unwrap()
            .into(),
    }
}

fn shell(command: &str) -> Value {
    json!({"session_id": "portable", "hook_event_name": "PreToolUse", "tool_name": "Bash",
        "tool_input": {"command": command}, "command": command})
}

#[test]
fn a_clone_works_after_installing_only_its_engine() {
    let fixture = Fixture::new();
    let first = fixture.root.join("first machine/shop");
    fs::create_dir_all(first.join("src")).unwrap();
    fs::write(first.join("composer.json"), "{\"name\":\"acme/shop\"}\n").unwrap();
    fs::write(first.join("src/Invoice.php"), "<?php\n").unwrap();
    fixture.git(&first, &["init", "--quiet"]);
    fixture.git(&first, &["add", "--all"]);
    fixture.git(&first, &["commit", "--quiet", "-m", "project"]);
    assert_eq!(fixture.install(&first, false)["status"], "complete");
    for path in WIRING {
        let content = fs::read_to_string(first.join(path)).unwrap();
        assert!(
            !content.contains(fixture.root.to_str().unwrap()),
            "{path} names a path of the installing machine:\n{content}"
        );
    }
    fixture.git(&first, &["add", "--all"]);
    fixture.git(&first, &["commit", "--quiet", "-m", "accelerator"]);

    let clone = fixture.root.join("second machine/shop");
    fs::create_dir_all(clone.parent().unwrap()).unwrap();
    fixture.git(
        &fixture.root,
        &[
            "clone",
            "--quiet",
            first.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    assert!(!clone.join("memory-bank/bin/infra-engine").exists());
    // Without its own engine the clone refuses shell commands on every host.
    for host in ["claude", "codex", "cursor"] {
        let cwd = if host == "codex" {
            clone.join("src")
        } else {
            clone.clone()
        };
        let output = fixture.hook(
            host,
            &clone,
            &cwd,
            &guard_command(&clone, host),
            &shell("git status"),
        );
        assert_eq!(
            output.status.code(),
            Some(2),
            "{host}: {}",
            text(&output.stderr)
        );
        assert!(
            text(&output.stderr).contains("no accelerator engine is installed"),
            "{host}"
        );
    }

    // A teammate's install adds only the engine and leaves no diff.
    let plan = fixture.install(&clone, true);
    assert_eq!(plan["collisions"], json!([]), "{plan}");
    let changed = plan["changed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["path"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(changed, ["memory-bank/bin/infra-engine"], "{plan}");
    assert_eq!(fixture.install(&clone, false)["status"], "complete");
    let status = fixture
        .command("git")
        .args(["status", "--porcelain"])
        .current_dir(&clone)
        .output()
        .unwrap();
    assert_eq!(
        text(&status.stdout),
        "",
        "teammate install changed tracked files"
    );

    for host in ["claude", "codex", "cursor"] {
        let cwd = if host == "codex" {
            clone.join("src")
        } else {
            clone.clone()
        };
        let command = guard_command(&clone, host);
        let output = fixture.hook(host, &clone, &cwd, &command, &shell("git status"));
        assert_eq!(
            output.status.code(),
            Some(0),
            "{host}: {}",
            text(&output.stderr)
        );
        let output = fixture.hook(
            host,
            &clone,
            &cwd,
            &command,
            &shell("git push --force origin main"),
        );
        assert_eq!(output.status.code(), Some(2), "{host}");
        assert!(
            text(&output.stderr).contains("Command refused"),
            "{host}: {}",
            text(&output.stderr)
        );
    }

    // A linked worktree has no engine of its own and uses the main checkout's.
    let linked = fixture.root.join("second machine/linked");
    fixture.git(
        &clone,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "feature/linked",
            linked.to_str().unwrap(),
        ],
    );
    assert!(!linked.join("memory-bank/bin/infra-engine").exists());
    let output = fixture.hook(
        "claude",
        &linked,
        &linked,
        &guard_command(&linked, "claude"),
        &shell("git push --force origin main"),
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(
        text(&output.stderr).contains("Command refused"),
        "{}",
        text(&output.stderr)
    );

    // The committed memory MCP registration starts the clone's own server.
    let entry = &json_file(&clone.join(".mcp.json"))["mcpServers"]["harness-memory"];
    let args = entry["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| arg.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    let mut server = fixture.command(entry["command"].as_str().unwrap());
    let mut child = server
        .args(&args)
        .env("CLAUDE_PROJECT_DIR", &clone)
        .current_dir(&clone)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            concat!(
                "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{},\"clientInfo\":{\"name\":\"t\",\"version\":\"1\"}}}\n",
                "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
                "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"memory_status\",\"arguments\":{}}}\n"
            )
            .as_bytes(),
        )
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let status = text(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|reply| reply["id"] == 2)
        .unwrap_or_else(|| {
            panic!(
                "no MCP reply: {}{}",
                text(&output.stdout),
                text(&output.stderr)
            )
        });
    let report: Value = serde_json::from_str(
        status["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("MCP status reply: {status}")),
    )
    .unwrap();
    assert!(
        report["database"]
            .as_str()
            .unwrap()
            .starts_with(fs::canonicalize(&clone).unwrap().to_str().unwrap()),
        "{report}"
    );
}

#[test]
fn committed_launchers_are_the_generated_ones_and_find_their_root() {
    let source = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")).unwrap();
    let mut compared = 0;
    for edition in [
        "Laravel",
        "Symfony",
        "PHP Core",
        "Cms/wordpress",
        "Infrastructure-Creator",
    ] {
        for host in ["claude", "cursor", "codex"] {
            for role in infra_cli::hooks::ROLES {
                let path = source.join(format!("{edition}/.{host}/hooks/{role}.sh"));
                let Ok(committed) = fs::read_to_string(&path) else {
                    continue;
                };
                assert_eq!(
                    committed,
                    infra_cli::hooks::shim(host, role, false).unwrap(),
                    "{}",
                    path.display()
                );
                compared += 1;
            }
        }
    }
    assert!(compared >= 123, "{compared} launchers");
    // Hosts name launchers by a path; one run by its bare name from its own
    // directory finds the same root, as `dirname` did.
    let fixture = Fixture::new();
    let root = fixture.root.join("project");
    let hooks = root.join(".claude/hooks");
    fs::create_dir_all(&hooks).unwrap();
    let launcher = hooks.join("loop-detection.sh");
    fs::write(
        &launcher,
        infra_cli::hooks::shim("claude", "loop-detection", false).unwrap(),
    )
    .unwrap();
    let engine = root.join("memory-bank/bin/infra-engine");
    fs::create_dir_all(engine.parent().unwrap()).unwrap();
    fs::write(&engine, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&engine, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let expected = fs::canonicalize(&root).unwrap();
    for (cwd, name) in [
        (&hooks, "loop-detection.sh"),
        (&root, ".claude/hooks/loop-detection.sh"),
        (&root, launcher.to_str().unwrap()),
    ] {
        let output = Command::new("sh")
            .arg(name)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert_eq!(
            text(&output.stdout),
            format!(
                "hook\nclaude\nloop-detection\n--root\n{}\n",
                expected.display()
            ),
            "{name}: {}",
            text(&output.stderr)
        );
    }
}

#[test]
fn every_edition_ships_the_ignore_rules_beside_its_engine() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    for edition in ["Laravel", "Symfony", "PHP Core", "Cms/wordpress"] {
        assert_eq!(
            fs::read_to_string(source.join(edition).join("memory-bank/bin/.gitignore")).unwrap(),
            infra_tooling::launch::ENGINE_IGNORE,
            "{edition}"
        );
    }
}
