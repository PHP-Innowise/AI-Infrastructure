//! The guard hooks Claude Code, Codex and Cursor run on every tool call,
//! through the actual executable: only roster agents spawn, write-capable
//! agents run one at a time, repeated commands and edits warn then stop, and
//! memory and task files keep their layout. Each fixture is a repository of
//! its own with the PHP Core roster; counters and locks stay in its TMPDIR.
//! POSIX only: Windows paths and Git Bash payloads need a native runner.
#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const HOSTS: [&str; 3] = ["claude", "codex", "cursor"];
const INHERITED: [&str; 8] = [
    "ACCELERATOR_HOME",
    "ACCELERATOR_STATE_DIR",
    "ACCELERATOR_PROJECT_DIR",
    "CLAUDE_PROJECT_DIR",
    "CONTEXT_TASK_ID",
    "CONTEXT_MEMORY_RECOVERY",
    "SUBAGENT_WRITE_LOCK_TTL_MINUTES",
    "INFRA_SOURCE_ROOT",
];

fn edition() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../PHP Core")
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&path, &to.join(entry.file_name()));
        } else {
            fs::copy(&path, to.join(entry.file_name())).unwrap();
        }
    }
}

/// Agent names of a roster folder and whether each declares `writes: true`.
fn roster(folder: &Path) -> Vec<(String, bool)> {
    let mut result = Vec::new();
    for entry in fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|extension| extension != "md")
            || path.file_name().is_some_and(|name| name == "README.md")
        {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        let front = text
            .strip_prefix("---\n")
            .and_then(|rest| rest.split_once("\n---\n"))
            .map(|(front, _)| front)
            .unwrap_or_else(|| panic!("{} has no frontmatter", path.display()));
        let field = |key: &str| {
            front.lines().find_map(|line| {
                line.strip_prefix(&format!("{key}:"))
                    .map(|value| value.trim().trim_matches('"').to_string())
            })
        };
        result.push((
            field("name").unwrap(),
            field("writes").as_deref() == Some("true"),
        ));
    }
    result.sort();
    result
}

fn described(output: &Output) -> String {
    format!(
        "exit {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Whether a guard message sends the agent to the host's debugger: Codex has
/// no slash commands, so its messages name the skill.
fn names_debugger(host: &str, text: &str) -> bool {
    if host == "codex" {
        text.contains("systematic-debugger") && !text.contains("/debugger")
    } else {
        text.contains("/debugger")
    }
}

/// Exit code, stdout and stderr of a hook that let the call through silently.
fn silent(output: &Output) -> bool {
    output.status.code() == Some(0) && output.stdout.is_empty() && output.stderr.is_empty()
}

/// The warning Claude Code and Codex hand the model next to the tool result.
fn model_context(output: &Output) -> String {
    let value: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("no hook JSON: {}", described(output)));
    value["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_else(|| panic!("no additionalContext: {}", described(output)))
        .to_string()
}

fn verdict(output: &Output) -> Value {
    assert_eq!(output.status.code(), Some(0), "{}", described(output));
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("no Cursor verdict: {}", described(output)))
}

fn shell(session: &str, command: &str) -> Value {
    json!({"session_id": session, "hook_event_name": "PreToolUse", "tool_name": "Bash",
        "tool_input": {"command": command}})
}

fn edit(session: &str, path: &str) -> Value {
    json!({"session_id": session, "tool_name": "Edit",
        "tool_input": {"file_path": path, "old_string": "a", "new_string": "b"}})
}

fn patch(session: &str, body: &str) -> Value {
    json!({"session_id": session, "tool_name": "apply_patch",
        "tool_input": {"command": format!("*** Begin Patch\n{body}*** End Patch\n")}})
}

fn spawn(host: &str, agent: &str) -> Value {
    if host == "cursor" {
        json!({"subagent_type": agent})
    } else {
        json!({"tool_name": "Agent", "tool_input": {"subagent_type": agent, "prompt": "x"}})
    }
}

struct Fixture {
    root: PathBuf,
    project: PathBuf,
    temporary: PathBuf,
    locks: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "infra-hook-guards-{}-{nonce}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let project = root.join("shop with spaces");
        let temporary = root.join("tmp");
        let locks = root.join("locks");
        for path in [&project, &temporary, &locks, &root.join("home")] {
            fs::create_dir_all(path).unwrap();
        }
        let source = edition();
        for tree in [".claude/agents", ".cursor/agents"] {
            copy_tree(&source.join(tree), &project.join(tree));
        }
        // Task and spec file names start with a skill each tool discovers.
        for skills in [".claude/skills", ".cursor/skills", ".agents/skills"] {
            for entry in fs::read_dir(source.join(skills)).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    fs::create_dir_all(project.join(skills).join(entry.file_name())).unwrap();
                }
            }
        }
        let fixture = Self {
            root,
            project,
            temporary,
            locks,
        };
        let mut git = Command::new("git");
        fixture.isolated(&mut git);
        let status = git
            .args(["-c", "init.templateDir=", "init", "--quiet"])
            .current_dir(&fixture.project)
            .status()
            .unwrap();
        assert!(status.success());
        fixture
    }

    fn isolated(&self, command: &mut Command) {
        command
            .env("HOME", self.root.join("home"))
            .env("USERPROFILE", self.root.join("home"))
            .env("TMPDIR", &self.temporary)
            .env("SUBAGENT_WRITE_LOCK_DIR", &self.locks)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.root.join("home/gitconfig"));
        for name in INHERITED {
            command.env_remove(name);
        }
    }

    fn raw(&self, host: &str, role: &str, input: &[u8], environment: &[(&str, &Path)]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_infra-engine"));
        self.isolated(&mut command);
        for (name, value) in environment {
            command.env(name, value);
        }
        let mut child = command
            .args(["hook", host, role, "--root"])
            .arg(&self.project)
            .current_dir(&self.project)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        child.wait_with_output().unwrap()
    }

    fn hook(&self, host: &str, role: &str, payload: &Value) -> Output {
        self.raw(host, role, &serde_json::to_vec(payload).unwrap(), &[])
    }

    fn lock(&self, host: &str) -> Option<String> {
        let prefix = format!("{host}-write-agent-lock-");
        fs::read_dir(&self.locks)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&prefix) && !name.ends_with(".guard"))
            })
            .and_then(|path| fs::read_to_string(path).ok())
    }

    fn clear_locks(&self) {
        for entry in fs::read_dir(&self.locks).unwrap() {
            let _ = fs::remove_file(entry.unwrap().path());
        }
    }

    /// The edit counter files the loop detection keeps for this repository.
    fn edit_counters(&self) -> Vec<PathBuf> {
        let mut result = Vec::new();
        for folder in fs::read_dir(&self.temporary).unwrap() {
            let folder = folder.unwrap().path();
            if folder
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("claude-loop-detection-"))
            {
                for entry in fs::read_dir(folder).unwrap() {
                    let path = entry.unwrap().path();
                    if path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("edit-"))
                    {
                        result.push(path);
                    }
                }
            }
        }
        result
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn subagent_gate_admits_only_the_roster_on_each_host() {
    let fixture = Fixture::new();
    let claude = roster(&fixture.project.join(".claude/agents"));
    let cursor = roster(&fixture.project.join(".cursor/agents"));
    assert!(!claude.is_empty() && !cursor.is_empty());

    for builtin in ["Explore", "Plan", "general-purpose", "claude"] {
        let output = fixture.hook("claude", "subagent-gate", &spawn("claude", builtin));
        assert_eq!(
            output.status.code(),
            Some(2),
            "{builtin}: {}",
            described(&output)
        );
        assert!(
            stderr(&output).contains("BLOCKED"),
            "{}",
            described(&output)
        );
    }
    let legacy = json!({"tool_name": "Task", "tool_input": {"subagent_type": "general-purpose"}});
    let output = fixture.hook("claude", "subagent-gate", &legacy);
    assert_eq!(output.status.code(), Some(2), "{}", described(&output));
    for (name, _) in &claude {
        let output = fixture.hook("claude", "subagent-gate", &spawn("claude", name));
        assert_eq!(
            output.status.code(),
            Some(0),
            "{name}: {}",
            described(&output)
        );
        // A write-capable agent takes the lock; the sweep releases it.
        fixture.clear_locks();
    }
    for input in [
        serde_json::to_vec(&json!({"tool_name": "Bash", "tool_input": {"command": "ls"}})).unwrap(),
        serde_json::to_vec(&json!({"tool_name": "Agent", "tool_input": {}})).unwrap(),
        b"not json".to_vec(),
        Vec::new(),
    ] {
        let output = fixture.raw("claude", "subagent-gate", &input, &[]);
        assert_eq!(output.status.code(), Some(0), "{}", described(&output));
    }

    for builtin in ["explore", "shell", "bash", "browser", "generalPurpose"] {
        let decision = verdict(&fixture.hook("cursor", "subagent-gate", &spawn("cursor", builtin)));
        assert_eq!(decision["permission"], "deny", "{builtin}: {decision}");
        assert!(decision["user_message"].is_string(), "{decision}");
    }
    for (name, _) in &cursor {
        let decision = verdict(&fixture.hook("cursor", "subagent-gate", &spawn("cursor", name)));
        assert_eq!(decision["permission"], "allow", "{name}: {decision}");
        fixture.clear_locks();
    }
    let decision = verdict(&fixture.hook("cursor", "subagent-gate", &json!({"task": "x"})));
    assert_eq!(decision["permission"], "allow", "{decision}");

    for tool in [
        "spawn_agent",
        "Agent",
        "send_input",
        "resume_agent",
        "wait_agent",
        "close_agent",
    ] {
        let output = fixture.hook(
            "codex",
            "subagent-gate",
            &json!({"tool_name": tool, "tool_input": {}}),
        );
        assert_eq!(
            output.status.code(),
            Some(2),
            "{tool}: {}",
            described(&output)
        );
        assert!(
            stderr(&output).contains("BLOCKED"),
            "{}",
            described(&output)
        );
    }
    for input in [
        serde_json::to_vec(&json!({"tool_name": "Shell", "tool_input": {"command": "ls"}}))
            .unwrap(),
        serde_json::to_vec(&json!({"tool_name": "Edit", "tool_input": {"file_path": "a.php"}}))
            .unwrap(),
        Vec::new(),
    ] {
        let output = fixture.raw("codex", "subagent-gate", &input, &[]);
        assert_eq!(output.status.code(), Some(0), "{}", described(&output));
    }
}

#[test]
fn write_capable_agents_run_one_at_a_time() {
    let fixture = Fixture::new();
    let claude = roster(&fixture.project.join(".claude/agents"));
    let writers = claude
        .iter()
        .filter(|(_, writes)| *writes)
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    let reader = claude
        .iter()
        .find(|(_, writes)| !*writes)
        .map(|(name, _)| name.as_str())
        .unwrap();
    assert!(writers.len() >= 3, "the roster has write-capable agents");
    let (first, second) = (writers[0], writers[1]);

    let output = fixture.hook("claude", "subagent-gate", &spawn("claude", first));
    assert_eq!(output.status.code(), Some(0), "{}", described(&output));
    assert_eq!(fixture.lock("claude").as_deref(), Some(first));
    for agent in [second, first] {
        let output = fixture.hook("claude", "subagent-gate", &spawn("claude", agent));
        assert_eq!(
            output.status.code(),
            Some(2),
            "{agent}: {}",
            described(&output)
        );
        assert!(
            stderr(&output).contains("already running"),
            "{}",
            described(&output)
        );
    }
    let output = fixture.hook("claude", "subagent-gate", &spawn("claude", reader));
    assert_eq!(output.status.code(), Some(0), "{}", described(&output));
    assert_eq!(fixture.lock("claude").as_deref(), Some(first));

    // Completion releases only the holder's lock.
    let stopped = |agent: &str| json!({"hook_event_name": "SubagentStop", "session_id": "gate", "agent_type": agent});
    let output = fixture.hook("claude", "subagent-dispatch", &stopped(second));
    assert_eq!(output.status.code(), Some(0), "{}", described(&output));
    assert_eq!(fixture.lock("claude").as_deref(), Some(first));
    let output = fixture.hook("claude", "subagent-dispatch", &stopped(first));
    assert_eq!(output.status.code(), Some(0), "{}", described(&output));
    assert_eq!(
        fixture.lock("claude"),
        None,
        "the holder's completion keeps the lock"
    );

    // A lock older than its time to live is a holder that never finished.
    let output = fixture.hook("claude", "subagent-gate", &spawn("claude", first));
    assert_eq!(output.status.code(), Some(0), "{}", described(&output));
    let held = fs::read_dir(&fixture.locks)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| !path.to_string_lossy().ends_with(".guard"))
        .unwrap();
    fs::File::options()
        .write(true)
        .open(&held)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(3600))
        .unwrap();
    let output = fixture.hook("claude", "subagent-gate", &spawn("claude", second));
    assert_eq!(output.status.code(), Some(0), "{}", described(&output));
    assert_eq!(fixture.lock("claude").as_deref(), Some(second));
    fixture.clear_locks();

    // Cursor answers with a permission and never names the lock folder.
    assert_eq!(
        verdict(&fixture.hook("cursor", "subagent-gate", &spawn("cursor", first)))["permission"],
        "allow"
    );
    let denied = fixture.hook("cursor", "subagent-gate", &spawn("cursor", second));
    let decision = verdict(&denied);
    assert_eq!(decision["permission"], "deny", "{decision}");
    assert!(
        decision["user_message"]
            .as_str()
            .unwrap()
            .contains("one at a time"),
        "{decision}"
    );
    assert!(!stdout(&denied).contains(&*fixture.locks.to_string_lossy()));
    assert_eq!(
        verdict(&fixture.hook("cursor", "subagent-gate", &spawn("cursor", reader)))["permission"],
        "allow"
    );
    fixture.clear_locks();

    // A lock folder that is not an existing absolute folder refuses the spawn.
    let missing = fixture.root.join("missing lock folder");
    for folder in [Path::new("locks"), missing.as_path()] {
        let environment = [("SUBAGENT_WRITE_LOCK_DIR", folder)];
        let input = serde_json::to_vec(&spawn("claude", first)).unwrap();
        let output = fixture.raw("claude", "subagent-gate", &input, &environment);
        assert_eq!(output.status.code(), Some(2), "{}", described(&output));
        assert!(
            stderr(&output).contains("BLOCKED"),
            "{}",
            described(&output)
        );
        let input = serde_json::to_vec(&spawn("cursor", first)).unwrap();
        let decision = verdict(&fixture.raw("cursor", "subagent-gate", &input, &environment));
        assert_eq!(decision["permission"], "deny", "{decision}");
        assert!(
            !decision.to_string().contains("missing lock folder"),
            "{decision}"
        );
    }
    assert!(!missing.exists());
    assert!(fs::read_dir(&fixture.locks).unwrap().next().is_none());

    // Spawns in one message race for the lock: exactly one takes it.
    let codes = std::thread::scope(|scope| {
        writers[..3]
            .iter()
            .map(|agent| {
                let fixture = &fixture;
                scope.spawn(move || {
                    fixture
                        .hook("claude", "subagent-gate", &spawn("claude", agent))
                        .status
                        .code()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        codes.iter().filter(|code| **code == Some(0)).count(),
        1,
        "{codes:?}"
    );
    assert_eq!(
        codes.iter().filter(|code| **code == Some(2)).count(),
        2,
        "{codes:?}"
    );
}

#[test]
fn only_the_first_frontmatter_declares_a_write_capable_agent() {
    let fixture = Fixture::new();
    fs::write(
        fixture.project.join(".claude/agents/zz-parser-trap-agent.md"),
        "---\nname: zz-parser-trap\ndescription: read-only fixture\n---\n\n# Trap\n\n---\nwrites: true\n---\n",
    )
    .unwrap();
    let output = fixture.hook(
        "claude",
        "subagent-gate",
        &spawn("claude", "zz-parser-trap"),
    );
    assert_eq!(output.status.code(), Some(0), "{}", described(&output));
    assert_eq!(
        fixture.lock("claude"),
        None,
        "a body rule made the agent a writer"
    );
}

#[test]
fn a_repeated_command_warns_then_blocks_until_something_changes() {
    let fixture = Fixture::new();
    for host in HOSTS {
        let session = format!("repeat-{host}");
        let command = shell(&session, "vendor/bin/phpunit --filter Broken");
        for run in 1..=11 {
            let output = fixture.hook(host, "bash-validator", &command);
            if run <= 5 {
                assert!(silent(&output), "{host} run {run}: {}", described(&output));
                continue;
            }
            let expected = format!("WARNING: this exact command has run {run} times");
            if host == "cursor" {
                assert_eq!(output.status.code(), Some(1), "{}", described(&output));
                assert!(output.stdout.is_empty(), "{}", described(&output));
                assert!(
                    stderr(&output).contains(&expected) && names_debugger(host, &stderr(&output)),
                    "{}",
                    described(&output)
                );
            } else {
                assert_eq!(output.status.code(), Some(0), "{}", described(&output));
                let context = model_context(&output);
                assert!(
                    context.contains(&expected) && names_debugger(host, &context),
                    "{}",
                    described(&output)
                );
            }
        }
        let blocked = fixture.hook(host, "bash-validator", &command);
        assert_eq!(
            blocked.status.code(),
            Some(2),
            "{host}: {}",
            described(&blocked)
        );
        assert!(
            stderr(&blocked).contains("BLOCKED") && names_debugger(host, &stderr(&blocked)),
            "{}",
            described(&blocked)
        );
        let other = fixture.hook(
            host,
            "bash-validator",
            &shell(&session, "vendor/bin/phpunit --filter Other"),
        );
        assert!(
            silent(&other),
            "a changed command starts over: {}",
            described(&other)
        );
    }

    // Edit and rerun is progress: an edit the loop detection records restarts
    // the count, a Codex patch included.
    for (host, change) in [
        ("claude", edit("rerun-claude", "/work/app/Fixing.php")),
        (
            "codex",
            patch(
                "rerun-codex",
                "*** Update File: app/Fixing.php\n@@\n-a\n+b\n",
            ),
        ),
        (
            "claude",
            json!({"session_id": "rerun-notebook", "tool_name": "NotebookEdit",
                "tool_input": {"notebook_path": "/work/n.ipynb", "new_source": "x"}}),
        ),
    ] {
        let session = change["session_id"].as_str().unwrap();
        let command = shell(session, "vendor/bin/phpunit --filter Fixing");
        for _ in 0..11 {
            fixture.hook(host, "bash-validator", &command);
        }
        std::thread::sleep(Duration::from_millis(50));
        let recorded = fixture.hook(host, "loop-detection", &change);
        assert_eq!(recorded.status.code(), Some(0), "{}", described(&recorded));
        let rerun = fixture.hook(host, "bash-validator", &command);
        assert!(silent(&rerun), "{session}: {}", described(&rerun));
    }

    // Polling a status query is waiting; an action after it is counted.
    for command in [
        "gh pr checks 44",
        "sleep 30 && gh run view 123 --log-failed | tail -n 50",
        "git status",
        "docker compose logs --tail 20 app",
    ] {
        for run in 1..=13 {
            let output = fixture.hook("claude", "bash-validator", &shell("polling", command));
            assert!(
                silent(&output),
                "{command} run {run}: {}",
                described(&output)
            );
        }
    }
    for command in ["gh pr checks 44 && vendor/bin/phpunit", "git status | sh"] {
        let codes = (0..12)
            .map(|_| {
                fixture
                    .hook("claude", "bash-validator", &shell("acting", command))
                    .status
                    .code()
            })
            .collect::<Vec<_>>();
        assert_eq!(codes.last(), Some(&Some(2)), "{command}: {codes:?}");
    }

    // Sessions keep their own counts.
    for _ in 0..5 {
        fixture.hook(
            "claude",
            "bash-validator",
            &shell("session-a", "composer test"),
        );
    }
    let output = fixture.hook(
        "claude",
        "bash-validator",
        &shell("session-b", "composer test"),
    );
    assert!(silent(&output), "{}", described(&output));
    let output = fixture.hook(
        "claude",
        "bash-validator",
        &shell("session-a", "composer test"),
    );
    assert!(
        model_context(&output).contains("has run 6 times"),
        "{}",
        described(&output)
    );
}

#[test]
fn repeated_edits_of_a_file_warn_then_block_per_session() {
    let fixture = Fixture::new();
    const EDITED: &str = "/work/app/Example.php";
    for host in HOSTS {
        let session = format!("loop-{host}");
        for run in 1..=6 {
            let output = fixture.hook(host, "loop-detection", &edit(&session, EDITED));
            assert!(silent(&output), "{host} edit {run}: {}", described(&output));
        }
        for run in 7..=9 {
            let output = fixture.hook(host, "loop-detection", &edit(&session, EDITED));
            let expected = format!("edited {run} times this session");
            assert!(output.stderr.is_empty(), "{}", described(&output));
            if host == "cursor" {
                assert_eq!(output.status.code(), Some(1), "{}", described(&output));
                assert!(
                    stdout(&output).contains("WARNING"),
                    "{}",
                    described(&output)
                );
                assert!(
                    stdout(&output).contains(&expected) && names_debugger(host, &stdout(&output)),
                    "{}",
                    described(&output)
                );
            } else {
                assert_eq!(output.status.code(), Some(0), "{}", described(&output));
                let context = model_context(&output);
                assert!(
                    context.contains(&expected) && names_debugger(host, &context),
                    "{}",
                    described(&output)
                );
            }
        }
        let blocked = fixture.hook(host, "loop-detection", &edit(&session, EDITED));
        assert_eq!(
            blocked.status.code(),
            Some(2),
            "{host}: {}",
            described(&blocked)
        );
        assert!(
            stderr(&blocked).contains("BLOCKED") && names_debugger(host, &stderr(&blocked)),
            "{}",
            described(&blocked)
        );
        assert!(blocked.stdout.is_empty(), "{}", described(&blocked));
    }

    // Cursor and Codex payloads may name the edited file `path`; Claude
    // Code's own edit tools never do.
    for host in HOSTS {
        let session = format!("named{host}");
        let payload = json!({"session_id": session, "tool_name": "Edit",
            "tool_input": {"path": EDITED}});
        let output = fixture.hook(host, "loop-detection", &payload);
        assert!(silent(&output), "{host}: {}", described(&output));
        let counted = fixture
            .edit_counters()
            .iter()
            .filter(|path| path.to_string_lossy().contains(&format!("edit-{session}-")))
            .count();
        assert_eq!(counted, usize::from(host != "claude"), "{host}");
    }

    // Every file a Codex patch touches is an edit of that file.
    let body = "*** Update File: app/A.php\n@@\n-a\n+b\n*** Update File: app/B.php\n@@\n-a\n+b\n*** Add File: app/C.php\n+new\n";
    for run in 1..=6 {
        let output = fixture.hook("codex", "loop-detection", &patch("patching", body));
        assert!(silent(&output), "patch {run}: {}", described(&output));
    }
    let seventh = fixture.hook("codex", "loop-detection", &patch("patching", body));
    let context = model_context(&seventh);
    for name in ["app/A.php", "app/B.php", "app/C.php"] {
        assert!(
            context.contains(&format!("File '{name}' edited 7 times")),
            "{context}"
        );
    }

    // Another session's count is its own.
    for _ in 0..9 {
        fixture.hook("claude", "loop-detection", &edit("one", EDITED));
    }
    let other = fixture.hook("claude", "loop-detection", &edit("two", EDITED));
    assert!(silent(&other), "{}", described(&other));
    let tenth = fixture.hook("claude", "loop-detection", &edit("one", EDITED));
    assert_eq!(tenth.status.code(), Some(2), "{}", described(&tenth));

    // A read names a file but edits none; a payload without a path counts nothing.
    let counters = fixture.edit_counters().len();
    for payload in [
        json!({"session_id": "reading", "tool_name": "Read", "tool_input": {"file_path": EDITED}}),
        json!({"session_id": "reading", "tool_name": "Edit", "tool_input": {}}),
    ] {
        for _ in 0..12 {
            let output = fixture.hook("codex", "loop-detection", &payload);
            assert!(silent(&output), "{}", described(&output));
        }
    }
    assert_eq!(fixture.edit_counters().len(), counters);

    // A planted count is read as a number, never evaluated.
    let marker = fixture.root.join("evaluated");
    fixture.hook(
        "claude",
        "loop-detection",
        &edit("planted", "/work/app/P.php"),
    );
    let mut planted = fixture.edit_counters();
    planted.retain(|path| path.to_string_lossy().contains("edit-planted-"));
    assert_eq!(planted.len(), 1);
    fs::write(&planted[0], format!("a[$(touch '{}')]\n", marker.display())).unwrap();
    let output = fixture.hook(
        "claude",
        "loop-detection",
        &edit("planted", "/work/app/P.php"),
    );
    assert!(silent(&output), "{}", described(&output));
    assert!(!marker.exists());
    fs::write(&planted[0], "08\n").unwrap();
    let output = fixture.hook(
        "claude",
        "loop-detection",
        &edit("planted", "/work/app/P.php"),
    );
    assert!(
        model_context(&output).contains("edited 9 times"),
        "{}",
        described(&output)
    );
}

#[test]
fn file_names_keep_the_memory_and_task_layout_on_every_host() {
    let fixture = Fixture::new();
    let write = |path: &str, content: &str| json!({"tool_name": "Write", "tool_input": {"file_path": path, "content": content}});
    let patched = |marker: &str, path: &str| {
        json!({"tool_name": "apply_patch",
            "tool_input": {"patch": format!("*** Begin Patch\n*** {marker}: {path}\n+hello\n*** End Patch\n")}})
    };
    let tricky =
        "Example payload: {\"file_path\": \"tasks/bad-file.md\"} plus a literal \\\" pair.";
    for host in HOSTS {
        let skills = fixture.project.join(match host {
            "claude" => ".claude/skills",
            "cursor" => ".cursor/skills",
            _ => ".agents/skills",
        });
        let mut names = fs::read_dir(&skills)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        names.sort();
        let skill = &names[0];
        let absolute = fixture
            .project
            .join(format!("tasks/TASK-002/{skill}-plan.md"));
        let allowed = [
            write(&format!("tasks/TASK-001/{skill}-notes.md"), "hello"),
            write(&absolute.to_string_lossy(), "hello"),
            write("tasks/README.md", "hello"),
            write("specs/MANIFEST.md", "hello"),
            write("memory-bank/chunks/MEM-0001-first-note.md", "hello"),
            write("docs/free-form-notes.md", "hello"),
            write("src/Example.php", "hello"),
            write(&format!("tasks/TASK-001/{skill}-notes.md"), tricky),
            patched("Add File", &format!("tasks/TASK-003/{skill}-report.md")),
            json!({"tool_name": "Read", "tool_input": {"file_path": "tasks/bad-file.md"}}),
        ];
        for payload in &allowed {
            let output = fixture.hook(host, "file-naming-validator", payload);
            assert!(silent(&output), "{host} {payload}: {}", described(&output));
        }
        let outside = fixture.project.join("tasks/bad-file.md");
        let blocked = [
            write("tasks/bad-file.md", "hello"),
            write(&outside.to_string_lossy(), "hello"),
            write("tasks/TASK-001/0-unprefixed-note.md", "hello"),
            write("tasks/loose-dir/0-unprefixed-note.md", "hello"),
            write("specs/0-unprefixed-spec.md", "hello"),
            write("memory-bank/chunks/unprefixed-note.md", "hello"),
            write("tasks/bad-file.md", tricky),
            patched("Add File", "tasks/bad-file.md"),
            patched("Update File", "tasks/bad-file.md"),
        ];
        for payload in &blocked {
            let output = fixture.hook(host, "file-naming-validator", payload);
            assert_eq!(
                output.status.code(),
                Some(2),
                "{host} {payload}: {}",
                described(&output)
            );
            assert!(
                stderr(&output).contains("BLOCKED"),
                "{}",
                described(&output)
            );
            assert!(output.stdout.is_empty(), "{}", described(&output));
        }
    }
}
