//! One task across sessions, a commit and a second branch, through the hooks
//! an installation commits. The PHP Core edition is installed from this
//! repository into a disposable project; each turn runs the prompt and Stop
//! launchers with the host's payload, and a save runs the exact command the
//! Stop continuation hands the model. No model, provider or network is used.
//! The committed launchers are POSIX shell; native Windows wiring differs.
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

const FIRST: &str = "feature/SHOP-7-invoices";

fn repository() -> PathBuf {
    fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")).unwrap()
}

fn described(output: &Output) -> String {
    format!(
        "exit {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// The command a Stop continuation tells the model to pipe its result into.
fn save_command(continuation: &str) -> String {
    let (_, rest) = continuation
        .split_once("this exact command: ")
        .unwrap_or_else(|| panic!("no save command in: {continuation}"));
    let command = rest.lines().next().unwrap();
    assert!(command.ends_with("--json"), "{command}");
    assert!(
        command.contains(" --input - "),
        "the result travels on stdin: {command}"
    );
    command.into()
}

struct Project {
    root: PathBuf,
    path: PathBuf,
}

impl Project {
    fn new() -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "infra-memory-lifecycle-{}-{nonce}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        for name in ["home", "tmp"] {
            fs::create_dir(root.join(name)).unwrap();
        }
        let project = Self {
            path: root.join("shop"),
            root,
        };
        fs::create_dir_all(project.path.join("src")).unwrap();
        fs::write(
            project.path.join("composer.json"),
            "{\"name\": \"acme/shop\"}\n",
        )
        .unwrap();
        fs::write(
            project.path.join("src/Invoice.php"),
            "<?php\nfinal class Invoice {}\n",
        )
        .unwrap();
        fs::write(
            project.path.join("src/Ledger.php"),
            "<?php\nfinal class Ledger {}\n",
        )
        .unwrap();
        project.git(&["init", "--quiet", "--initial-branch", FIRST]);
        project.git(&["add", "--all"]);
        project.git(&["commit", "--quiet", "-m", "Shop baseline"]);
        let mut command = project.command(env!("CARGO_BIN_EXE_infra-engine"));
        let output = command
            .args(["install", "--source-root"])
            .arg(repository())
            .args([
                "--edition",
                "PHP Core",
                "--merge-existing",
                "--json",
                "--target",
            ])
            .arg(&project.path)
            .output()
            .unwrap();
        assert!(output.status.success(), "install: {}", described(&output));
        project
    }

    /// A command with nothing of the calling session: a suite run from a
    /// Claude Code or Codex session would otherwise hand the hooks its marks.
    fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut command = Command::new(program);
        command
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", self.root.join("home"))
            .env("TMPDIR", self.root.join("tmp"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.root.join("home/gitconfig"))
            .env("GIT_OPTIONAL_LOCKS", "0")
            .current_dir(&self.path)
            .stdin(Stdio::null());
        command
    }

    fn git(&self, args: &[&str]) -> String {
        let output = self
            .command("git")
            .args([
                "-c",
                "init.templateDir=",
                "-c",
                "user.name=Lifecycle test",
                "-c",
                "user.email=lifecycle-test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            described(&output)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    /// Whether the project's Git ignores `path` (`git check-ignore`).
    fn ignored(&self, path: &str) -> bool {
        let output = self
            .command("git")
            .args(["check-ignore", "--quiet", "--", path])
            .output()
            .unwrap();
        match output.status.code() {
            Some(0) => true,
            Some(1) => false,
            _ => panic!("git check-ignore {path}: {}", described(&output)),
        }
    }

    /// Runs a committed hook launcher the way the host does.
    fn hook(&self, host: &str, role: &str, payload: &Value) -> String {
        let tree = match host {
            "claude" => ".claude",
            "cursor" => ".cursor",
            _ => ".codex",
        };
        let mut payload = payload.clone();
        let mut command = self.command("sh");
        match host {
            "claude" => {
                command
                    .env("CLAUDE_PROJECT_DIR", &self.path)
                    .env("CLAUDE_CODE_SESSION_ATTENDED", "1")
                    .env("CLAUDE_CODE_ENTRYPOINT", "claude-desktop");
            }
            "cursor" => {
                payload["workspace_roots"] = json!([self.path]);
                payload["cursor_version"] = "2.0.0".into();
            }
            _ => {
                payload["turn_id"] = infra_core::domain::uuid4().into();
                payload["model"] = "test".into();
                payload["permission_mode"] = "default".into();
            }
        }
        let output = run(
            command
                .arg(
                    self.path
                        .join(tree)
                        .join("hooks")
                        .join(format!("{role}.sh")),
                )
                .stdin(Stdio::piped()),
            &serde_json::to_vec(&payload).unwrap(),
        );
        assert_eq!(
            output.status.code(),
            Some(0),
            "{host} {role}: {}",
            described(&output)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    /// One turn: the prompt's capsule, the Stop continuation, the save the
    /// model would run, and the Stop that follows it.
    fn turn(&self, host: &str, session: &str, prompt: &str, result: Option<Value>) -> String {
        let base = json!({"session_id": session, "cwd": self.path});
        let mut asked = base.clone();
        asked["hook_event_name"] = "UserPromptSubmit".into();
        asked["prompt"] = prompt.into();
        let capsule = self.hook(host, "working-memory-read", &asked);
        let mut stop = base.clone();
        stop["hook_event_name"] = "Stop".into();
        stop["stop_hook_active"] = false.into();
        let stopped = self.hook(host, "working-memory-write", &stop);
        if let Some(result) = result {
            let output: Value = serde_json::from_str(stopped.lines().next().unwrap_or(""))
                .unwrap_or_else(|_| panic!("no Stop continuation: {stopped}"));
            let continuation = if host == "codex" {
                assert_eq!(output["decision"], "block", "{output}");
                let reason = output["reason"].as_str().unwrap().to_string();
                // Fed back as a prompt, the continuation gets no second capsule.
                let mut echoed = base.clone();
                echoed["hook_event_name"] = "UserPromptSubmit".into();
                echoed["prompt"] = reason.clone().into();
                assert_eq!(self.hook(host, "working-memory-read", &echoed).trim(), "");
                reason
            } else {
                // Hook feedback, not a block: Claude Code labels a block an error.
                assert!(output.get("decision").is_none(), "{output}");
                output["hookSpecificOutput"]["additionalContext"]
                    .as_str()
                    .unwrap()
                    .to_string()
            };
            self.save(&continuation, &result);
        }
        stop["stop_hook_active"] = true.into();
        self.hook(host, "working-memory-write", &stop);
        capsule
    }

    /// Runs the save a continuation asks for with `result` on its stdin.
    fn save(&self, continuation: &str, result: &Value) {
        let mut command = self.command("sh");
        let saved = run(
            command
                .args(["-c", &save_command(continuation)])
                .stdin(Stdio::piped()),
            &serde_json::to_vec(result).unwrap(),
        );
        assert!(saved.status.success(), "save: {}", described(&saved));
    }

    /// Saves recorded in the native result journal.
    fn saves(&self) -> usize {
        fs::read_to_string(self.path.join("memory-bank/local/native-results.ndjson"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|event| event["stage"] == "save" && event["event"] == "saved")
            .count()
    }

    fn validate(&self) {
        let output = self
            .command(self.path.join("memory-bank/bin/infra-engine"))
            .args(["memory", "validate"])
            .output()
            .unwrap();
        assert!(output.status.success(), "validate: {}", described(&output));
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn run(command: &mut Command, input: &[u8]) -> Output {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

fn follow_a_task(host: &str) {
    let project = Project::new();
    let one = format!("one-{host}");
    project.turn(
        host,
        &one,
        "How are invoices numbered?",
        Some(
            json!({"progress": "Read src/Invoice.php; numbering is per tenant.", "verified": true,
            "learnings": [{"type": "finding", "title": "Invoice numbers are per tenant",
                "consequence": "Invoice numbering restarts for each tenant.",
                "sources": ["src/Invoice.php"]}]}),
        ),
    );
    let ledger = project.path.join("src/Ledger.php");
    fs::write(
        &ledger,
        fs::read_to_string(&ledger).unwrap() + "// per tenant\n",
    )
    .unwrap();
    project.turn(
        host,
        &one,
        "Make the ledger per tenant.",
        Some(
            json!({"progress": "Made src/Ledger.php per tenant; tests next.", "verified": true,
            "learnings": [], "next_steps": ["Run the ledger tests."]}),
        ),
    );
    project.git(&["add", "--", "src/Ledger.php"]);
    project.git(&["commit", "--quiet", "-m", "Ledger per tenant"]);

    let capsule = project.turn(
        host,
        &format!("two-{host}"),
        "What do we know about invoice numbers per tenant?",
        None,
    );
    for expected in [
        "Made src/Ledger.php per tenant",
        "Run the ledger tests.",
        "Invoice numbers are per tenant",
    ] {
        assert!(
            capsule.contains(expected),
            "{host} recalls {expected:?}:\n{capsule}"
        );
    }

    project.git(&["checkout", "--quiet", "-b", "feature/SHOP-8-refunds"]);
    let capsule = project.turn(
        host,
        &format!("three-{host}"),
        "Start the refunds.",
        Some(json!({"progress": "Refunds started.", "verified": true, "learnings": []})),
    );
    assert!(
        !capsule.contains("Made src/Ledger.php"),
        "another branch got this task's progress:\n{capsule}"
    );

    project.git(&["checkout", "--quiet", FIRST]);
    let capsule = project.turn(
        host,
        &format!("four-{host}"),
        "Continue with the ledger.",
        None,
    );
    assert!(
        capsule.contains("Made src/Ledger.php per tenant"),
        "{capsule}"
    );
    assert!(!capsule.contains("Refunds started"), "{capsule}");

    assert_eq!(project.saves(), 3, "every save reached the journal once");
    project.validate();
}

#[test]
fn claude_code_recalls_a_task_in_the_next_session_and_only_on_its_branch() {
    follow_a_task("claude");
}

#[test]
fn codex_recalls_a_task_in_the_next_session_and_only_on_its_branch() {
    follow_a_task("codex");
}

/// Cursor reads its rule before the prompt hook runs, so the rule a prompt
/// writes is what the next request carries. Its save continuation comes back
/// as a prompt and must not replace that rule.
#[test]
fn cursor_keeps_the_last_prompts_rule_through_a_save_continuation() {
    let project = Project::new();
    fs::write(
        project.path.join("billing.md"),
        "# Billing\n\nInvoices are numbered per tenant and restart each year.\n",
    )
    .unwrap();
    project.git(&["add", "--", "billing.md"]);
    project.git(&["commit", "--quiet", "-m", "Billing"]);
    let cursor = |role: &str, generation: &str, mut payload: Value| {
        payload["conversation_id"] = "lifecycle".into();
        payload["generation_id"] = generation.into();
        project.hook("cursor", role, &payload)
    };
    let prompt = |generation: &str, text: &str| {
        cursor(
            "working-memory-read",
            generation,
            json!({"hook_event_name": "beforeSubmitPrompt", "prompt": text}),
        )
    };
    let stop = |generation: &str, turn: u64| {
        let output = cursor(
            "working-memory-write",
            generation,
            json!({"hook_event_name": "stop", "status": "completed", "loop_count": turn}),
        );
        serde_json::from_str::<Value>(output.trim()).unwrap_or(Value::Null)["followup_message"]
            .as_str()
            .map(str::to_string)
    };

    prompt("g1", "How are invoices numbered per tenant?");
    let followup = stop("g1", 0).expect("the first stop asks for a save");
    project.save(
        &followup,
        &json!({"progress": "Read billing.md; invoice numbering is per tenant.", "verified": true,
            "learnings": [{"type": "finding", "title": "Invoices are numbered per tenant",
                "consequence": "Invoice numbering restarts per tenant and per year.",
                "sources": ["billing.md"]}]}),
    );
    stop("g1", 1);

    prompt("g2", "Which tenant rules apply to invoice numbers?");
    let followup = stop("g2", 0).expect("the next stop asks for a save");
    let rule = project.path.join(".cursor/rules/working-memory.mdc");
    let before = fs::read(&rule).expect("the prompt rendered the rule");
    let text = String::from_utf8_lossy(&before);
    assert!(text.contains("Invoices are numbered per tenant"), "{text}");
    assert!(
        text.starts_with("---\n") && text.contains("\nalwaysApply: true\n"),
        "{text}"
    );
    let echoed: Value = serde_json::from_str(prompt("g3", &followup).trim()).unwrap();
    assert_eq!(echoed, json!({"continue": true}));
    assert_eq!(
        fs::read(&rule).unwrap(),
        before,
        "the continuation replaced the rule"
    );
    assert!(
        project.ignored(".cursor/rules/working-memory.mdc"),
        "the rendered rule is never committed"
    );
}

/// The prompt hook's capsule for a session on this checkout.
fn prompt(project: &Project, host: &str, session: &str) -> String {
    project.hook(
        host,
        "working-memory-read",
        &json!({"session_id": session, "cwd": project.path,
            "hook_event_name": "UserPromptSubmit", "prompt": "resolve the ledger conflict"}),
    )
}

#[test]
fn a_save_during_a_rebase_reaches_the_task_of_the_rebased_branch() {
    let project = Project::new();
    project.turn(
        "claude",
        "before",
        "Where is the ledger?",
        Some(json!({"progress": "Ledger lives in src/Ledger.php.", "verified": true, "learnings": []})),
    );
    project.git(&["branch", "main"]);
    let ledger = project.path.join("src/Ledger.php");
    fs::write(&ledger, "<?php\nfinal class Ledger { /* feature */ }\n").unwrap();
    project.git(&["commit", "--quiet", "--all", "-m", "feature ledger"]);
    project.git(&["checkout", "--quiet", "main"]);
    fs::write(&ledger, "<?php\nfinal class Ledger { /* main */ }\n").unwrap();
    project.git(&["commit", "--quiet", "--all", "-m", "main ledger"]);
    project.git(&["checkout", "--quiet", FIRST]);
    let rebase = project
        .command("git")
        .args(["rebase", "main"])
        .output()
        .unwrap();
    assert!(!rebase.status.success(), "the rebase stops at its conflict");
    let admin = PathBuf::from(project.git(&["rev-parse", "--absolute-git-dir"]));
    assert_eq!(
        fs::read_to_string(admin.join("rebase-merge/head-name"))
            .unwrap()
            .trim(),
        format!("refs/heads/{FIRST}")
    );

    project.turn(
        "claude",
        "rebasing",
        "Resolve the conflict.",
        Some(
            json!({"progress": "Resolved the ledger conflict during the rebase.",
            "verified": true, "learnings": []}),
        ),
    );
    project.git(&["rebase", "--abort"]);
    let capsule = prompt(&project, "claude", "after");
    assert!(
        capsule.contains(&format!("working: {FIRST}"))
            && capsule.contains("Resolved the ledger conflict during the rebase."),
        "the save made during the rebase reached the branch's task:\n{capsule}"
    );

    // Any other detached HEAD names no task; neither does a bisect started
    // on one, which names a commit.
    let head = project.git(&["rev-parse", "HEAD"]);
    project.git(&["checkout", "--quiet", "--detach"]);
    let plain = prompt(&project, "claude", "detached");
    assert!(!plain.contains("Resolved the ledger conflict"), "{plain}");
    fs::write(admin.join("BISECT_START"), format!("{head}\n")).unwrap();
    let bisect = prompt(&project, "claude", "bisect-detached");
    assert!(!bisect.contains("Resolved the ledger conflict"), "{bisect}");
}

#[test]
fn a_prompt_during_a_rebase_or_a_bisect_reads_the_task_of_its_branch() {
    let project = Project::new();
    project.turn(
        "claude",
        "before",
        "Where is the ledger?",
        Some(json!({"progress": "Ledger lives in src/Ledger.php.", "verified": true, "learnings": []})),
    );
    let admin = PathBuf::from(project.git(&["rev-parse", "--absolute-git-dir"]));
    project.git(&["checkout", "--quiet", "--detach"]);
    for state in ["rebase-merge", "rebase-apply", "BISECT_START"] {
        if state == "BISECT_START" {
            fs::write(admin.join(state), format!("{FIRST}\n")).unwrap();
        } else {
            fs::create_dir(admin.join(state)).unwrap();
            fs::write(
                admin.join(state).join("head-name"),
                format!("refs/heads/{FIRST}\n"),
            )
            .unwrap();
        }
        for host in ["claude", "codex"] {
            let capsule = prompt(&project, host, &format!("{state}-{host}"));
            assert!(
                capsule.contains(&format!("working: {FIRST}"))
                    && capsule.contains("Ledger lives in src/Ledger.php."),
                "{host} during {state} reads the branch's task:\n{capsule}"
            );
        }
        let state = admin.join(state);
        if state.is_dir() {
            fs::remove_dir_all(state).unwrap();
        } else {
            fs::remove_file(state).unwrap();
        }
    }
}
