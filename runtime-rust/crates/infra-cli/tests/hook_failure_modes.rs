//! Hook failure modes through the actual executable. The shell guard never
//! lets a command through because its own context failed, a memory role never
//! refuses a prompt or a stop, and a session outside the project the hooks are
//! bound to is not served that project's memory.
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const OUTSIDE: &str = "outside the project these hooks are bound to";
/// Label, bound root, session directory and extra environment.
type Case<'a> = (&'a str, &'a Path, &'a Path, &'a [(&'a str, &'a Path)]);
const INHERITED: [&str; 12] = [
    "ACCELERATOR_HOME",
    "ACCELERATOR_STATE_DIR",
    "ACCELERATOR_PROJECT_DIR",
    "CLAUDE_PROJECT_DIR",
    "CONTEXT_TASK_ID",
    "CONTEXT_HOOK_BUDGET",
    "CONTEXT_MEMORY_RECOVERY",
    "CONTEXT_CAPSULE_DELIVERED",
    "CONTEXT_FLUSH_AFTER",
    "PROJECT_BRAIN_MODE",
    "PROJECT_BRAIN_OWNER",
    "INFRA_SOURCE_ROOT",
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
            "infra-hook-failure-{}-{nonce}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("create private test root");
        let root = infra_core::fs::canonicalize(root).expect("resolve private test root");
        assert!(
            !root
                .ancestors()
                .any(|parent| parent.join("runtime-rust/Cargo.toml").is_file()),
            "hook fixture must be outside the source checkout"
        );
        let home = root.join("private home");
        fs::create_dir(&home).expect("create private home");
        Self { root, home }
    }

    fn isolated(&self, command: &mut Command) {
        command
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CODEX_HOME", self.home.join("codex"))
            .env("CLAUDE_CONFIG_DIR", self.home.join("claude"))
            .env("XDG_CONFIG_HOME", self.home.join("config"))
            .env("XDG_DATA_HOME", self.home.join("data"))
            .env("XDG_STATE_HOME", self.home.join("state"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.home.join("gitconfig"));
        for name in INHERITED {
            command.env_remove(name);
        }
    }

    fn git(&self, cwd: &Path, args: &[&str]) {
        let mut command = Command::new("git");
        self.isolated(&mut command);
        let output = command
            .args([
                "-c",
                "init.templateDir=",
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// An independent repository with one commit.
    fn project(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        fs::create_dir_all(path.join("src")).unwrap();
        fs::write(path.join("composer.json"), "{\"name\":\"acme/shop\"}\n").unwrap();
        fs::write(path.join("src/Invoice.php"), "<?php\n").unwrap();
        self.git(&path, &["init", "--quiet"]);
        self.git(&path, &["add", "--all"]);
        self.git(&path, &["commit", "--quiet", "-m", "fixture"]);
        path
    }

    fn hook(
        &self,
        host: &str,
        role: &str,
        root: &Path,
        cwd: &Path,
        payload: Vec<u8>,
        environment: &[(&str, &Path)],
    ) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_infra-engine"));
        self.isolated(&mut command);
        for (name, value) in environment {
            command.env(name, value);
        }
        let mut child = command
            .args(["hook", host, role, "--root"])
            .arg(root)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start the hook");
        let mut stdin = child.stdin.take().unwrap();
        // The engine stops reading an oversized payload; a closed pipe is expected then.
        let writer = std::thread::spawn(move || {
            let _ = stdin.write_all(&payload);
        });
        let output = child.wait_with_output().expect("finish the hook");
        writer.join().unwrap();
        output
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn payload(value: Value) -> Vec<u8> {
    serde_json::to_vec(&value).unwrap()
}

fn shell(command: &str, cwd: &Path) -> Vec<u8> {
    payload(json!({
        "session_id": "hook-failure-modes",
        "cwd": cwd,
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": command}
    }))
}

fn prompt(event: &str, cwd: &Path) -> Vec<u8> {
    payload(json!({
        "session_id": "hook-failure-modes",
        "transcript_path": cwd.join("absent transcript.jsonl"),
        "cwd": cwd,
        "hook_event_name": event,
        "stop_hook_active": true,
        "prompt": "resolve the conflict in runtime.json"
    }))
}

fn described(output: &Output) -> String {
    format!(
        "exit {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// A merge left in the shared runtime configuration.
fn conflict(project: &Path) {
    let config = project.join("project-brain/config");
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("runtime.json"),
        "{\n<<<<<<< HEAD\n  \"mode\": \"governed\",\n=======\n  \"mode\": \"lightweight\",\n>>>>>>> feature\n  \"framework\": \"php-core\"\n}\n",
    )
    .unwrap();
}

#[test]
fn shell_guard_checks_commands_when_its_context_cannot_be_built() {
    let fixture = Fixture::new();
    // A checkout whose .git names a missing repository, as a linked worktree
    // does once its main checkout moved.
    let detached = fixture.root.join("detached checkout");
    fs::create_dir(&detached).unwrap();
    fs::write(detached.join(".git"), "gitdir: /nonexistent/worktrees/x\n").unwrap();
    // An intact project whose session directory carries the same defect.
    let intact = fixture.project("intact project");
    let nested = intact.join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join(".git"), "gitdir: /nonexistent/nested\n").unwrap();
    let conflicted = fixture.project("conflicted project");
    conflict(&conflicted);
    let moved = fixture.root.join("moved project");
    let missing_state = fixture.root.join("missing state");
    let cases: [Case; 5] = [
        ("broken project Git", &detached, &detached, &[]),
        ("broken session Git", &intact, &nested, &[]),
        (
            "missing state directory",
            &intact,
            &intact,
            &[("ACCELERATOR_STATE_DIR", &missing_state)],
        ),
        (
            "conflicted runtime configuration",
            &conflicted,
            &conflicted,
            &[],
        ),
        ("moved project root", &moved, &intact, &[]),
    ];
    for (label, root, cwd, environment) in cases {
        for (command, code) in [
            ("rm -rf /", 2),
            ("git push --force origin main", 2),
            ("git status", 0),
        ] {
            let output = fixture.hook(
                "claude",
                "bash-validator",
                root,
                cwd,
                shell(command, cwd),
                environment,
            );
            assert_eq!(
                output.status.code(),
                Some(code),
                "{label}: {command}\n{}",
                described(&output)
            );
            if code == 2 {
                assert!(
                    String::from_utf8_lossy(&output.stderr).contains("BLOCKED: Command refused"),
                    "{label}: {command}\n{}",
                    described(&output)
                );
            }
        }
    }
}

#[test]
fn memory_roles_pass_the_turn_when_they_fail() {
    let fixture = Fixture::new();
    let project = fixture.project("conflicted project");
    conflict(&project);
    for host in ["claude", "codex", "cursor"] {
        for role in [
            "working-memory-read",
            "context-continuity",
            "working-memory-write",
            "local-context",
        ] {
            for event in ["UserPromptSubmit", "Stop"] {
                let output =
                    fixture.hook(host, role, &project, &project, prompt(event, &project), &[]);
                assert_eq!(
                    output.status.code(),
                    Some(0),
                    "{host} {role} {event}\n{}",
                    described(&output)
                );
            }
        }
    }
    // The configuration error names the file to repair.
    let mut status = Command::new(env!("CARGO_BIN_EXE_infra-engine"));
    fixture.isolated(&mut status);
    let status = status
        .args(["memory", "--root"])
        .arg(&project)
        .arg("status")
        .current_dir(&project)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_ne!(status.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&status.stderr).contains("runtime.json: invalid JSON"),
        "{}",
        described(&status)
    );
    // Input over the hook limit passes a memory turn and still refuses a command.
    let oversized = payload(json!({
        "session_id": "hook-failure-modes",
        "cwd": project,
        "hook_event_name": "UserPromptSubmit",
        "prompt": "x".repeat(4 * 1024 * 1024 + 1)
    }));
    for role in ["working-memory-read", "context-continuity"] {
        let output = fixture.hook("claude", role, &project, &project, oversized.clone(), &[]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{role}\n{}",
            described(&output)
        );
    }
    let output = fixture.hook(
        "cursor",
        "working-memory-read",
        &project,
        &project,
        oversized,
        &[],
    );
    assert_eq!(output.status.code(), Some(0), "{}", described(&output));
    assert_eq!(output.stdout, b"{\"continue\": true}\n");
    let command = format!("rm -rf / {}", "x".repeat(4 * 1024 * 1024));
    let output = fixture.hook(
        "claude",
        "bash-validator",
        &project,
        &project,
        shell(&command, &project),
        &[],
    );
    assert_eq!(output.status.code(), Some(2), "{}", described(&output));
}

#[test]
fn memory_hooks_do_not_serve_a_session_outside_the_bound_project() {
    let fixture = Fixture::new();
    let project = fixture.project("bound project");
    let copy = fixture.project("second clone");
    for host in ["claude", "codex"] {
        for role in ["working-memory-read", "local-context"] {
            let output = fixture.hook(
                host,
                role,
                &project,
                &copy,
                prompt("UserPromptSubmit", &copy),
                &[],
            );
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert_eq!(
                output.status.code(),
                Some(0),
                "{host} {role}\n{}",
                described(&output)
            );
            assert!(
                stdout.contains(OUTSIDE),
                "{host} {role}\n{}",
                described(&output)
            );
        }
        for (role, event) in [
            ("context-continuity", "UserPromptSubmit"),
            ("working-memory-write", "Stop"),
        ] {
            let output = fixture.hook(host, role, &project, &copy, prompt(event, &copy), &[]);
            assert_eq!(
                output.status.code(),
                Some(0),
                "{host} {role}\n{}",
                described(&output)
            );
            assert!(
                output.stdout.is_empty(),
                "{host} {role}\n{}",
                described(&output)
            );
        }
    }
    // Cursor names its workspace roots instead of a working directory.
    let output = fixture.hook(
        "cursor",
        "working-memory-read",
        &project,
        &copy,
        payload(json!({"conversation_id": "c", "workspace_roots": [copy], "prompt": "hi"})),
        &[],
    );
    assert_eq!(
        output.stdout,
        b"{\"continue\": true}\n",
        "{}",
        described(&output)
    );
    // Claude Code names the directory its session started in; a session
    // started in the copy is outside even when its shell is in the project.
    let output = fixture.hook(
        "claude",
        "working-memory-read",
        &project,
        &project,
        prompt("UserPromptSubmit", &project),
        &[("CLAUDE_PROJECT_DIR", &copy)],
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(OUTSIDE),
        "{}",
        described(&output)
    );
    // Nothing was read into or recorded in the bound project.
    assert!(!project.join("memory-bank").exists());
    assert!(!project.join("project-brain").exists());
    // The shell guard still applies to the other checkout.
    let output = fixture.hook(
        "claude",
        "bash-validator",
        &project,
        &copy,
        shell("rm -rf /", &copy),
        &[],
    );
    assert_eq!(output.status.code(), Some(2), "{}", described(&output));
    // The project itself, a directory below it and its linked worktree are served.
    let linked = fixture.root.join("linked worktree");
    fixture.git(
        &project,
        &["worktree", "add", "--quiet", "-b", "feature/linked"]
            .into_iter()
            .chain(linked.to_str())
            .collect::<Vec<_>>(),
    );
    // A session of the project whose shell moved to the copy stays served.
    let output = fixture.hook(
        "claude",
        "working-memory-read",
        &project,
        &copy,
        prompt("UserPromptSubmit", &copy),
        &[("CLAUDE_PROJECT_DIR", &project)],
    );
    assert_eq!(output.status.code(), Some(0), "{}", described(&output));
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains(OUTSIDE),
        "{}",
        described(&output)
    );
    for cwd in [project.clone(), project.join("src"), linked] {
        let output = fixture.hook(
            "claude",
            "working-memory-read",
            &project,
            &cwd,
            prompt("UserPromptSubmit", &cwd),
            &[],
        );
        assert_eq!(output.status.code(), Some(0), "{}", described(&output));
        assert!(
            !String::from_utf8_lossy(&output.stdout).contains(OUTSIDE),
            "{}: {}",
            cwd.display(),
            described(&output)
        );
    }
}
