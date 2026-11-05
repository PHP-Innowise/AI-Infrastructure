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
    infra_core::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")).unwrap()
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
    // Claude Code refuses a program fed a descriptor it names once a session
    // entered a worktree: a save naming the worktree reads stdin unnamed.
    assert!(
        command.contains(" --input - ") || command.contains(" --project "),
        "the result travels on stdin: {command}"
    );
    command.into()
}

/// Claude Code runs a command without asking only when each part is a cd or
/// a program its allow rules name, and the command holds no subshell, { }
/// group, heredoc, substitution or printf; an absolute engine path matched no
/// installed rule, so every save asked, and `claude -p` refused it.
fn assert_claude_runs_without_asking(command: &str, project: &Path) {
    let settings: Value =
        serde_json::from_slice(&fs::read(project.join(".claude/settings.json")).unwrap()).unwrap();
    let allowed = settings["permissions"]["allow"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|rule| rule.as_str()?.strip_prefix("Bash(")?.strip_suffix(":*)"))
        .map(|prefix| format!("{prefix} "))
        .collect::<Vec<_>>();
    let shape = command.replace("'<JSON>'", "JSON");
    for asked in ["(", ")", "{", "}", "<<", "$", "`", ";", "printf"] {
        assert!(
            !shape.contains(asked),
            "Claude Code asks before {asked:?}: {command}"
        );
    }
    for part in shape.split(" && ").flat_map(|part| part.split(" | ")) {
        assert!(
            part.starts_with("cd ") || allowed.iter().any(|prefix| part.starts_with(prefix)),
            "no installed allow rule covers {part:?} of: {command}"
        );
    }
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
        let root = infra_core::fs::canonicalize(root).unwrap();
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
        self.turn_in(host, session, &self.path, prompt, result)
    }

    /// A turn of a session working in `cwd`, whose hooks this checkout runs.
    fn turn_in(
        &self,
        host: &str,
        session: &str,
        cwd: &Path,
        prompt: &str,
        result: Option<Value>,
    ) -> String {
        let base = json!({"session_id": session, "cwd": cwd});
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
                let continuation = output["hookSpecificOutput"]["additionalContext"]
                    .as_str()
                    .unwrap()
                    .to_string();
                // The spelling its allow rules cover, never an engine path.
                let command = save_command(&continuation);
                assert!(
                    command.starts_with("cd ")
                        && command.contains(
                            "echo -E '<JSON>' | memory-bank/bin/infra-engine memory --root "
                        ),
                    "{command}"
                );
                continuation
            };
            self.save(&continuation, &result);
        }
        stop["stop_hook_active"] = true.into();
        self.hook(host, "working-memory-write", &stop);
        capsule
    }

    /// Runs the save a continuation asks for: Claude Code's command carries
    /// `result` in place of <JSON> and runs in bash, as Claude Code runs it;
    /// the others read `result` on their stdin.
    fn save(&self, continuation: &str, result: &Value) {
        let command = save_command(continuation);
        let data = serde_json::to_string(result).unwrap();
        let saved = if command.contains("'<JSON>'") {
            assert_claude_runs_without_asking(&command, &self.path);
            let quoted = format!("'{}'", data.replace('\'', "'\\''"));
            run(
                self.command("bash")
                    .args(["-c", &command.replace("'<JSON>'", &quoted)])
                    .stdin(Stdio::piped()),
                b"",
            )
        } else {
            run(
                self.command("sh")
                    .args(["-c", &command])
                    .stdin(Stdio::piped()),
                data.as_bytes(),
            )
        };
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

    /// The native journal's Stop lines as `event:code`.
    fn stops(&self) -> Vec<String> {
        fs::read_to_string(self.path.join("memory-bank/local/native-results.ndjson"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|row| row["stage"] == "stop")
            .map(|row| {
                format!(
                    "{}:{}",
                    row["event"].as_str().unwrap_or(""),
                    row["code"].as_str().unwrap_or("")
                )
            })
            .collect()
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
    // The funnel reads why each Stop ended as it did: three continuations
    // found their save, two turns left the request unanswered.
    let stops = project.stops();
    let count = |line: &str| stops.iter().filter(|stop| *stop == line).count();
    assert_eq!(count("requested:"), 5, "{stops:?}");
    assert_eq!(count("skipped:recovery-saved"), 3, "{stops:?}");
    assert_eq!(count("failed:unsaved"), 2, "{stops:?}");
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

/// The first request of a new Cursor chat carries the rule its session start
/// rendered, since Cursor reads the rule before the prompt hook runs. The
/// start renders the branch task's memory; it once asked for an empty task
/// and removed the rule, so every new chat began without memory.
#[test]
fn cursor_session_start_renders_the_branch_tasks_rule() {
    let project = Project::new();
    let cursor = |role: &str, chat: &str, generation: &str, mut payload: Value| {
        payload["conversation_id"] = chat.into();
        payload["generation_id"] = generation.into();
        project.hook("cursor", role, &payload)
    };
    let stop = |turn: u64| {
        let output = cursor(
            "working-memory-write",
            "first",
            "g1",
            json!({"hook_event_name": "stop", "status": "completed", "loop_count": turn}),
        );
        serde_json::from_str::<Value>(output.trim()).unwrap_or(Value::Null)["followup_message"]
            .as_str()
            .map(str::to_string)
    };
    cursor(
        "working-memory-read",
        "first",
        "g1",
        json!({"hook_event_name": "beforeSubmitPrompt", "prompt": "How are invoices numbered?"}),
    );
    let followup = stop(0).expect("the stop asks for a save");
    project.save(
        &followup,
        &json!({"progress": "Read src/Invoice.php; invoice numbering is per tenant.",
            "verified": true, "learnings": []}),
    );
    stop(1);
    let rule = project.path.join(".cursor/rules/working-memory.mdc");
    assert!(rule.is_file(), "the prompt rendered the rule");
    // A checkout whose rule was never rendered: only the start can render it.
    fs::remove_file(&rule).unwrap();

    cursor(
        "local-context",
        "second",
        "s1",
        json!({"hook_event_name": "sessionStart"}),
    );
    let text = fs::read_to_string(&rule).expect("the session start rendered the rule");
    assert!(
        text.contains(&format!("(task: {FIRST})"))
            && text.contains("progress: Read src/Invoice.php; invoice numbering is per tenant."),
        "{text}"
    );
}

/// Claude Code names the main checkout as the project directory of a session
/// that works in a linked worktree: the main checkout's hooks serve the
/// worktree, and the save they ask for runs from the main checkout, naming
/// the worktree. That save was refused as naming no served worktree.
#[test]
fn claude_code_saves_and_recalls_in_a_worktree_the_main_checkout_serves() {
    let project = Project::new();
    let worktree = project.root.join("shop credit notes");
    project.git(&[
        "worktree",
        "add",
        "--quiet",
        "-b",
        "feature/SHOP-9-credit-notes",
        worktree.to_str().unwrap(),
    ]);
    let worktree = infra_core::fs::canonicalize(&worktree).unwrap();
    // The worktree has no memory-bank/bin/: the session hears the engine
    // that runs the CLI the policy and skills name.
    let banner = project.hook(
        "claude",
        "local-context",
        &json!({"session_id": "worktree-one", "cwd": worktree,
            "hook_event_name": "SessionStart", "source": "startup"}),
    );
    let engine = project.path.join("memory-bank/bin/infra-engine");
    assert!(
        banner.contains(&format!(
            "Memory CLI: this worktree has none - run the commands the policy and skills name with `{} memory` in place of `memory-bank/bin/infra-engine memory`.",
            engine.display()
        )),
        "{banner}"
    );
    // The worktree shares the installation's memory; it is no attached
    // project with nothing installed, as the banner once called it.
    assert!(
        banner.contains(&format!(
            "Accelerator: the installation in {}, whose memory this worktree shares",
            project.path.display()
        )) && !banner.contains("attached from"),
        "{banner}"
    );
    project.turn_in(
        "claude",
        "worktree-one",
        &worktree,
        "Do credit notes keep the invoice date?",
        Some(
            json!({"progress": "Credit notes keep the invoice date.", "verified": true,
            "learnings": [], "next_steps": ["Add the CreditNote class."]}),
        ),
    );
    let capsule = project.turn_in(
        "claude",
        "worktree-two",
        &worktree,
        "Continue with credit notes.",
        None,
    );
    assert!(
        capsule.contains("feature/SHOP-9-credit-notes")
            && capsule.contains("Credit notes keep the invoice date.")
            && capsule.contains("Add the CreditNote class."),
        "{capsule}"
    );
    assert_eq!(
        project.saves(),
        1,
        "the worktree's save reached the journal"
    );
}

/// The session banner lists the session's own host tree, as each host's hook
/// did, and gives an index that was never built no freshness.
#[test]
fn the_session_banner_lists_its_hosts_own_tree() {
    let project = Project::new();
    for (host, event, own, others) in [
        (
            "claude",
            "SessionStart",
            &[".claude/ ("][..],
            &[".codex/ (", ".cursor/ ("][..],
        ),
        (
            "codex",
            "SessionStart",
            &[".agents/ (", ".codex/ ("][..],
            &[".claude/ (", ".cursor/ ("][..],
        ),
        (
            "cursor",
            "sessionStart",
            &[".cursor/ ("][..],
            &[".claude/ (", ".codex/ ("][..],
        ),
    ] {
        let banner = project.hook(
            host,
            "local-context",
            &json!({"session_id": format!("banner-{host}"), "cwd": project.path,
                "hook_event_name": event, "source": "startup"}),
        );
        assert!(
            own.iter().all(|tree| banner.contains(tree))
                && !others.iter().any(|tree| banner.contains(tree)),
            "{host}: {banner}"
        );
        for state in ["missing", "empty"] {
            if let Some(at) = banner.find(&format!("index={state}/")) {
                assert!(
                    banner[at..].starts_with(&format!("index={state}/unknown")),
                    "{host}: {banner}"
                );
            }
        }
    }
}

/// Each edition's session banner reads the project as that edition's retired
/// hook did: Symfony names its framework or says bin/console is missing,
/// Laravel reads its version from composer.lock and names its frontend,
/// WordPress names the site's shape and its release alone, and each counts
/// its own folders. The native banner was PHP Core's for every edition.
#[test]
fn the_session_banner_reads_the_project_as_its_edition_did() {
    let project = Project::new();
    let config = project.path.join("project-brain/config/runtime.json");
    let base: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    let banner = |framework: &str| {
        let mut value = base.clone();
        value["framework"] = framework.into();
        fs::write(&config, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        project.hook(
            "claude",
            "local-context",
            &json!({"session_id": format!("edition-{framework}"), "cwd": project.path,
                "hook_event_name": "SessionStart", "source": "startup"}),
        )
    };
    let write = |path: &str, text: &str| {
        let file = project.path.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text).unwrap();
    };
    fs::create_dir_all(project.path.join("app")).unwrap();
    let symfony = banner("symfony");
    assert!(
        symfony.contains("\n\nNOTE: Symfony bin/console was not detected. Apply these Symfony rules only after confirming this is a Symfony project.\n")
            && symfony.contains("\n  src/ (")
            && !symfony.contains("project-brain/ (")
            && symfony.contains("(policy "),
        "{symfony}"
    );
    write("bin/console", "#!/usr/bin/env php\n");
    let symfony = banner("symfony");
    assert!(
        symfony.contains("\n\nFramework: Symfony detected. Use Symfony Controller -> Service -> Repository conventions.\n"),
        "{symfony}"
    );
    write("artisan", "#!/usr/bin/env php\n");
    write(
        "composer.lock",
        "{\n    \"packages\": [\n        {\n            \"name\": \"laravel/framework\",\n            \"version\": \"v11.9.2\"\n        }\n    ]\n}\n",
    );
    write("vite.config.js", "");
    write(
        "composer.json",
        "{\"require\": {\"livewire/livewire\": \"^3\"}}\n",
    );
    let laravel = banner("laravel");
    for line in [
        "\nLaravel: laravel/framework v11.9.2 (composer.lock)\n",
        "\nFrontend build: Vite present\nFrontend stack: Livewire\n\nNOTE: Symfony console detected alongside Laravel. Verify project intent.\n",
        "\n  app/ (",
    ] {
        assert!(laravel.contains(line), "{line:?} in {laravel}");
    }
    assert!(
        !laravel.contains("\n  src/ (") && !laravel.contains("Framework: Symfony"),
        "{laravel}"
    );
    write("wp-config.php", "<?php\n");
    write("blocks/block.json", "{}\n");
    write("composer.json", "{\"type\": \"wordpress-plugin\"}\n");
    let wordpress = banner("wordpress");
    for line in [
        "\nWordPress: full-site configuration present\nWordPress: block metadata present\n",
        "\nWordPress shape: Composer plugin package\n",
        "\n  project-brain/ (",
    ] {
        assert!(wordpress.contains(line), "{line:?} in {wordpress}");
    }
    assert!(
        !wordpress.contains("(policy ") && !wordpress.contains("NOTE:"),
        "{wordpress}"
    );
    let core = banner("php-core");
    assert!(
        core.contains(
            "\n\nNOTE: Symfony detected. This is the native-PHP base folder (PHP Core/).\n"
        ) && core.contains("\n  project-brain/ (")
            && core.contains("(policy ")
            && !core.contains("WordPress:"),
        "{core}"
    );
}

/// A save whose input is not one JSON object is refused with what to correct
/// in the one retry, worded as the retired runtime worded it, and the refusal
/// is journaled: a plain echo in zsh turns the \n of a JSON string into a
/// line break.
#[test]
fn a_malformed_save_is_refused_with_what_to_correct_and_journaled() {
    let project = Project::new();
    let base = json!({"session_id": "malformed", "cwd": project.path});
    let mut asked = base.clone();
    asked["hook_event_name"] = "UserPromptSubmit".into();
    asked["prompt"] = "Fix the ledger".into();
    project.hook("codex", "working-memory-read", &asked);
    let mut stop = base.clone();
    stop["hook_event_name"] = "Stop".into();
    stop["stop_hook_active"] = false.into();
    let stopped = project.hook("codex", "working-memory-write", &stop);
    let output: Value = serde_json::from_str(stopped.lines().next().unwrap_or(""))
        .unwrap_or_else(|_| panic!("no Stop continuation: {stopped}"));
    let command = save_command(output["reason"].as_str().unwrap());
    let refused = run(
        project
            .command("sh")
            .args(["-c", &command])
            .stdin(Stdio::piped()),
        b"{\"progress\": \"line one\nline two\", \"learnings\": []}",
    );
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        !refused.status.success()
            && stderr.contains("native-result:invalid-input: not one JSON object (Invalid control character at line 1 column 23); inside a string, write a newline as \\n and a tab as \\t (a plain echo in zsh turns \\n into a line break: pipe with echo -E)"),
        "{}",
        described(&refused)
    );
    let journal =
        fs::read_to_string(project.path.join("memory-bank/local/native-results.ndjson")).unwrap();
    assert!(
        journal
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .any(|row| row["stage"] == "save"
                && row["event"] == "refused"
                && row["code"] == "native-result:invalid-input"),
        "{journal}"
    );
}

/// A save that would start no task and record nothing says what to do
/// instead, as the retired refusal did; its code alone left the agent to
/// guess.
#[test]
fn an_empty_save_before_any_task_says_what_to_do_instead() {
    let project = Project::new();
    let base = json!({"session_id": "empty-save", "cwd": project.path});
    let mut asked = base.clone();
    asked["hook_event_name"] = "UserPromptSubmit".into();
    asked["prompt"] = "Where is the ledger?".into();
    project.hook("codex", "working-memory-read", &asked);
    let mut stop = base.clone();
    stop["hook_event_name"] = "Stop".into();
    stop["stop_hook_active"] = false.into();
    let stopped = project.hook("codex", "working-memory-write", &stop);
    let output: Value = serde_json::from_str(stopped.lines().next().unwrap_or(""))
        .unwrap_or_else(|_| panic!("no Stop continuation: {stopped}"));
    let command = save_command(output["reason"].as_str().unwrap());
    let refused = run(
        project
            .command("sh")
            .args(["-c", &command])
            .stdin(Stdio::piped()),
        b"{\"verified\": true, \"next_steps\": [], \"learnings\": []}",
    );
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        !refused.status.success()
            && stderr.contains("native-result:nothing-to-record: no task yet and nothing to start one with; give a progress summary or a learning, or run the command with --skip"),
        "{}",
        described(&refused)
    );
}

/// The Memory Bank has its own validator: a chunk citing a file Git renamed
/// fails `bank validate`, which names where the file went, while Project
/// Brain stays valid and the session banner says so. A retired chunk whose
/// file is gone only warns.
#[test]
fn a_broken_chunk_fails_the_bank_and_leaves_the_brain_valid() {
    let project = Project::new();
    fs::write(
        project.path.join("src/Old.php"),
        "<?php\nfinal class Old {}\n",
    )
    .unwrap();
    project.git(&["add", "src/Old.php"]);
    project.git(&["commit", "--quiet", "-m", "Old"]);
    project.git(&["mv", "src/Old.php", "src/New.php"]);
    project.git(&["commit", "--quiet", "-m", "Rename Old"]);
    let mut index = String::from(
        "# Memory Index\n\n| ID | Title | Type | Scope | Tags | Status | Last Verified | File |\n| --- | --- | --- | --- | --- | --- | --- | --- |\n",
    );
    fs::create_dir_all(project.path.join("memory-bank/chunks")).unwrap();
    for (id, status, source) in [
        ("MEM-20261104-0000000a", "active", "src/Old.php"),
        ("MEM-20261104-0000000b", "archived", "src/Gone.php"),
    ] {
        let metadata = json!({"id": id, "title": "Old keeps the date", "type": "decision",
            "status": status, "scope": ["billing"], "tags": ["billing"], "created": "2026-01-01",
            "last_verified": "2026-01-01", "review_after": "2999-01-01", "sources": [source],
            "supersedes": [], "superseded_by": null});
        fs::write(
            project
                .path
                .join(format!("memory-bank/chunks/{id}-old-keeps-the-date.md")),
            format!(
                "---\n{}\n---\n\nOld keeps the date.\n",
                serde_json::to_string_pretty(&metadata).unwrap()
            ),
        )
        .unwrap();
        index.push_str(&format!("| {id} | Old keeps the date | decision | billing | billing | {status} | 2026-01-01 | chunks/{id}-old-keeps-the-date.md |\n"));
    }
    fs::write(project.path.join("memory-bank/INDEX.md"), index).unwrap();
    let engine = project.path.join("memory-bank/bin/infra-engine");
    let bank = project
        .command(&engine)
        .args(["bank", "validate"])
        .output()
        .unwrap();
    assert_eq!(
        (
            bank.status.code(),
            String::from_utf8_lossy(&bank.stdout).as_ref(),
            String::from_utf8_lossy(&bank.stderr).as_ref()
        ),
        (
            Some(1),
            "",
            "Memory bank validation warnings (2):\n\
             - memory-bank/chunks/MEM-20261104-0000000b-old-keeps-the-date.md: cited source no longer exists: src/Gone.php (chunk is archived and no longer retrievable)\n\
             - memory-bank/INDEX.md: MEM-20261104-0000000a index row retained for invalid chunk (chunks/MEM-20261104-0000000a-old-keeps-the-date.md)\n\
             Memory bank validation failed (1 error(s)):\n\
             - memory-bank/chunks/MEM-20261104-0000000a-old-keeps-the-date.md: source path does not exist: src/Old.php - Git moved it to src/New.php\n"
        ),
        "{}",
        described(&bank)
    );
    let brain = project
        .command(&engine)
        .args(["memory", "validate"])
        .output()
        .unwrap();
    assert!(
        brain.status.success()
            && String::from_utf8_lossy(&brain.stdout)
                .starts_with("Project Brain validation passed."),
        "{}",
        described(&brain)
    );
    let banner = project.hook(
        "claude",
        "local-context",
        &json!({"session_id": "bank-broken", "cwd": project.path,
            "hook_event_name": "SessionStart", "source": "startup"}),
    );
    assert!(banner.contains("brain-validation=valid."), "{banner}");
}

/// `memory validate` runs the retired command's whole check and words each
/// problem as it did: the field the schema rejects, a credential stored in a
/// record, and a deterministic index left half-merged. The native command
/// checked less and passed all three.
#[test]
fn a_broken_brain_record_is_named_by_its_field_and_a_stored_key_fails() {
    let project = Project::new();
    let engine = project.path.join("memory-bank/bin/infra-engine");
    let memory = |args: &[&str]| {
        let output = project
            .command(&engine)
            .arg("memory")
            .args(args)
            .output()
            .unwrap();
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        )
    };
    let (code, started) = memory(&["start", "--task-id", "SHOP-7", "--goal", "Fix refunds"]);
    assert_eq!(code, Some(0), "{started}");
    let tasks = project.path.join("project-brain/dynamic/tasks");
    let record = fs::read_dir(&tasks)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|extension| extension == "md"))
        .unwrap();
    let original = fs::read_to_string(&record).unwrap();
    let relative = format!(
        "project-brain/dynamic/tasks/{}",
        record.file_name().unwrap().to_string_lossy()
    );
    assert_eq!(memory(&["validate"]).0, Some(0));

    fs::write(
        &record,
        original.replacen(
            "  \"authority\": ",
            "  \"layer\": \"x\",\n  \"authority\": ",
            1,
        ),
    )
    .unwrap();
    let (code, report) = memory(&["validate"]);
    assert_eq!(code, Some(1), "{report}");
    for line in [
        format!("{}: $: unexpected keys layer\n", record.display()),
        format!("Project Brain record invalid: {relative}: $: unexpected keys layer\n"),
    ] {
        assert!(report.contains(&line), "{line:?} in {report}");
    }

    fs::write(
        &record,
        original.replacen(
            "\"progress\": \"\"",
            "\"progress\": \"key AKIAIOSFODNN7EXAMPLE\"",
            1,
        ),
    )
    .unwrap();
    let (code, report) = memory(&["validate"]);
    assert_eq!(
        (code, report.as_str()),
        (
            Some(1),
            format!(
                "{}: record contains a possible AWS access key; remove it from the record and rotate the credential, since Git history keeps it\n",
                record.display()
            )
            .as_str()
        )
    );

    fs::write(&record, &original).unwrap();
    let index = project.path.join("project-brain/indexes/active.json");
    let built = fs::read_to_string(&index).unwrap();
    fs::write(
        &index,
        format!("<<<<<<< HEAD\n{built}=======\n{{}}\n>>>>>>> theirs\n"),
    )
    .unwrap();
    let (code, report) = memory(&["validate"]);
    assert_eq!(
        (code, report.as_str()),
        (
            Some(1),
            "project-brain/indexes/active.json: deterministic index is not valid JSON (Expecting value: line 1 column 1 (char 0)); a merge can leave conflict markers in it. It is derived from the records: the next Brain write rebuilds it - a checkpoint, `update`, `record-result` or `compact`.\n"
        )
    );
}

/// A refused update says what it expected and what to do, as the retired
/// runtime said it: the revision it found, the status it cannot reach, and
/// for an evidence level passed as a status, the flag that sets one (agents
/// pass `--transition verified` often enough to have measured it).
#[test]
fn a_refused_update_names_what_it_found_and_what_to_pass() {
    let project = Project::new();
    let engine = project.path.join("memory-bank/bin/infra-engine");
    let memory = |args: &[&str]| {
        let output = project
            .command(&engine)
            .arg("memory")
            .args(args)
            .output()
            .unwrap();
        assert!(!output.stderr.is_empty() || output.status.success());
        String::from_utf8_lossy(&output.stderr).into_owned()
    };
    memory(&["start", "--task-id", "SHOP-7", "--goal", "Fix refunds"]);
    for (args, refusal) in [
        (
            &[
                "update",
                "--task-id",
                "SHOP-7",
                "--revision",
                "9",
                "--progress",
                "x",
            ][..],
            "Stale task revision: expected 9, current 1",
        ),
        (
            &[
                "brain-update",
                "--record-id",
                "SHOP-7",
                "--revision",
                "1",
                "--transition",
                "verified",
            ][..],
            "Illegal task transition: active -> verified; verified is an evidence level, not a task status: set it with brain-update --authority verified",
        ),
        (
            &[
                "brain-update",
                "--record-id",
                "SHOP-7",
                "--revision",
                "1",
                "--transition",
                "resolved",
            ][..],
            "Illegal task transition: active -> resolved",
        ),
        (
            &[
                "brain-update",
                "--record-id",
                "SHOP-7",
                "--revision",
                "1",
                "--authority",
                "inferred",
            ][..],
            "Illegal authority transition: observed -> inferred; only observed -> verified is allowed",
        ),
    ] {
        let stderr = memory(args);
        assert!(stderr.contains(refusal), "{args:?}: {stderr}");
    }
    memory(&[
        "update",
        "--task-id",
        "SHOP-7",
        "--revision",
        "auto",
        "--phase",
        "verification",
    ]);
    let stderr = memory(&[
        "update",
        "--task-id",
        "SHOP-7",
        "--revision",
        "auto",
        "--phase",
        "planning",
    ]);
    assert!(
        stderr.contains("Phase may not move backward: verification -> planning; pass allow_phase_regression to state the regression is deliberate"),
        "{stderr}"
    );
}

/// `start` of a task under way says how to read and update it; after the
/// task is completed, `start` of the same ID archives the finished record
/// and begins a new task, as the retired start did. An update that changes
/// nothing, or blanks the progress, is refused and leaves the record alone.
#[test]
fn start_after_complete_begins_again_and_an_empty_update_is_refused() {
    let project = Project::new();
    let engine = project.path.join("memory-bank/bin/infra-engine");
    let memory = |args: &[&str]| {
        let output = project
            .command(&engine)
            .arg("memory")
            .args(args)
            .output()
            .unwrap();
        (
            output.status.success(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };
    let task = |id: &str| {
        let output = project
            .command(&engine)
            .args(["memory", "get", "--task-id", id, "--json"])
            .output()
            .unwrap();
        serde_json::from_slice::<Value>(&output.stdout).unwrap_or(Value::Null)
    };
    assert!(memory(&["start", "--task-id", "SHOP-7", "--goal", "Fix refunds"]).0);
    let (started, refusal) = memory(&["start", "--task-id", "SHOP-7", "--goal", "Fix refunds"]);
    assert!(
        !started
            && refusal.contains("Project Brain record already exists: SHOP-7; it is the task under way: memory-bank/bin/infra-engine memory get --task-id SHOP-7 reads it, and update --task-id SHOP-7 --revision auto records progress"),
        "{refusal}"
    );
    for (args, refusal) in [
        (
            &["update", "--task-id", "SHOP-7", "--revision", "auto"][..],
            "Working task update requires a changed field",
        ),
        (
            &[
                "update",
                "--task-id",
                "SHOP-7",
                "--revision",
                "auto",
                "--progress",
                " ",
            ][..],
            "Working task progress must not be empty",
        ),
        (
            &["brain-update", "--record-id", "SHOP-7", "--revision", "1"][..],
            "Brain record update requires a changed field",
        ),
        (
            &[
                "complete",
                "--task-id",
                "SHOP-7",
                "--revision",
                "auto",
                "--outcome",
                "Done",
            ][..],
            "Governed completion requires the current numeric --revision; memory-bank/bin/infra-engine memory get --task-id SHOP-7 shows it",
        ),
    ] {
        let (done, stderr) = memory(args);
        assert!(!done && stderr.contains(refusal), "{args:?}: {stderr}");
    }
    let first = task("SHOP-7");
    assert_eq!(first["revision"], 1, "{first}");
    assert!(
        memory(&[
            "complete",
            "--task-id",
            "SHOP-7",
            "--revision",
            "1",
            "--outcome",
            "Done"
        ])
        .0
    );
    let (restarted, stderr) = memory(&[
        "start",
        "--task-id",
        "SHOP-7",
        "--goal",
        "Fix refunds again",
    ]);
    assert!(restarted, "{stderr}");
    let second = task("SHOP-7");
    assert_eq!(second["goal"], "Fix refunds again", "{second}");
    assert_ne!(second["task_uuid"], first["task_uuid"], "{second}");
    let archived = project.path.join(format!(
        "project-brain/archive/task/{}.md",
        first["task_uuid"].as_str().unwrap()
    ));
    assert!(archived.is_file(), "{}", archived.display());
}

/// A refused `context-save` names what to correct in the retired words:
/// which fields are unexpected or missing, which flag the detail needs,
/// which cited path is refused and why, and the output that already exists.
#[test]
fn a_refused_handoff_names_the_field_the_flag_or_the_path() {
    let project = Project::new();
    let engine = project.path.join("memory-bank/bin/infra-engine");
    let input = project.root.join("tmp/handoff.json");
    let output = project.path.join("tasks/handoff-refused.md");
    let save = |curated: Value, extra: &[&str]| {
        fs::write(&input, serde_json::to_vec(&curated).unwrap()).unwrap();
        let refused = project
            .command(&engine)
            .args(["memory", "context-save", "--input"])
            .arg(&input)
            .arg("--output")
            .arg(&output)
            .args(["--source-client", "claude"])
            .args(extra)
            .output()
            .unwrap();
        assert!(!refused.status.success(), "{}", described(&refused));
        String::from_utf8_lossy(&refused.stderr).into_owned()
    };
    let good = json!({"goal": "Fix refunds", "summary": "Refunds keep the invoice date.",
        "next_steps": ["Add the test"]});
    let mut unexpected = good.clone();
    unexpected["mood"] = "fine".into();
    unexpected["layer"] = "x".into();
    let mut sensitive = good.clone();
    sensitive["files"] = json!([".env"]);
    let mut escaping = good.clone();
    escaping["files"] = json!(["../outside.php"]);
    for (curated, extra, refusal) in [
        (
            unexpected,
            &["--detail", "summary"][..],
            "input has unexpected fields: layer, mood",
        ),
        (
            json!({"goal": "Fix refunds"}),
            &["--detail", "summary"][..],
            "input is missing required fields: next_steps, summary",
        ),
        (
            good.clone(),
            &["--detail", "topic"][..],
            "--topic is required when --detail topic",
        ),
        (
            sensitive,
            &["--detail", "summary"][..],
            "file path is sensitive or invalid: .env",
        ),
        (
            escaping,
            &["--detail", "summary"][..],
            "file path escapes the repository: ../outside.php",
        ),
    ] {
        let stderr = save(curated, extra);
        assert!(stderr.contains(refusal), "{refusal:?} in {stderr}");
    }
    fs::create_dir_all(output.parent().unwrap()).unwrap();
    fs::write(&output, "kept\n").unwrap();
    let stderr = save(good, &["--detail", "summary"]);
    assert!(
        stderr.contains(&format!(
            "handoff output already exists: {}",
            output.display()
        )),
        "{stderr}"
    );
    assert_eq!(fs::read_to_string(&output).unwrap(), "kept\n");
}

/// A handoff edited after it was saved is refused on load by what no longer
/// holds, in the retired words.
#[test]
fn an_edited_handoff_is_refused_by_what_no_longer_holds() {
    let project = Project::new();
    let engine = project.path.join("memory-bank/bin/infra-engine");
    let input = project.root.join("tmp/handoff.json");
    let output = project.path.join("tasks/handoff-edited.md");
    fs::write(
        &input,
        r#"{"goal": "Fix refunds", "summary": "Refunds keep the invoice date.", "next_steps": ["Add the test"]}"#,
    )
    .unwrap();
    let saved = project
        .command(&engine)
        .args(["memory", "context-save", "--input"])
        .arg(&input)
        .arg("--output")
        .arg(&output)
        .args(["--source-client", "claude", "--detail", "summary"])
        .output()
        .unwrap();
    assert!(saved.status.success(), "{}", described(&saved));
    let original = fs::read_to_string(&output).unwrap();
    for (edited, refusal) in [
        (
            {
                // The body alone, after the frontmatter.
                let (head, body) = original[4..].split_once("\n---\n").unwrap();
                format!(
                    "---\n{head}\n---\n{}",
                    body.replace(
                        "Refunds keep the invoice date.",
                        "Refunds keep the order date."
                    )
                )
            },
            "handoff body does not match its curated context",
        ),
        (
            original.replacen("{\"body_sha256\"", "{\"layer\":\"x\",\"body_sha256\"", 1),
            "handoff frontmatter has an unsupported schema",
        ),
        (
            original.replacen(
                "\"source_client\": \"claude\"",
                "\"source_client\": \"vim\"",
                1,
            ),
            "handoff source client is invalid",
        ),
    ] {
        assert_ne!(edited, original, "{refusal}");
        fs::write(&output, &edited).unwrap();
        let loaded = project
            .command(&engine)
            .args(["memory", "context-load", "--input"])
            .arg(&output)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&loaded.stderr);
        assert!(
            !loaded.status.success() && stderr.contains(refusal),
            "{refusal:?}: {}",
            described(&loaded)
        );
    }
}

/// A credential handed to a command is refused before anything is written,
/// under the name of what would have held it; personal data in a new task's
/// goal is refused by the task record's own guard. The words are the retired
/// ones.
#[test]
fn a_credential_is_refused_under_the_name_of_what_would_hold_it() {
    let project = Project::new();
    let engine = project.path.join("memory-bank/bin/infra-engine");
    let refused = |args: &[&str]| {
        let output = project
            .command(&engine)
            .arg("memory")
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{}", described(&output));
        String::from_utf8_lossy(&output.stderr).into_owned()
    };
    let key = "key AKIAIOSFODNN7EXAMPLE";
    for (args, refusal) in [
        (
            vec!["start", "--task-id", "SHOP-7", "--goal", key],
            "possible AWS access key detected; working task not stored",
        ),
        (
            vec![
                "start",
                "--task-id",
                "SHOP-7",
                "--goal",
                "mail jane.doe@example.com",
            ],
            "Task record contains personal data (email address); replace it with a sanitized summary - nothing was stored",
        ),
    ] {
        let stderr = refused(&args);
        assert!(stderr.contains(refusal), "{args:?}: {stderr}");
    }
    let started = project
        .command(&engine)
        .args([
            "memory",
            "start",
            "--task-id",
            "SHOP-7",
            "--goal",
            "Fix refunds",
        ])
        .output()
        .unwrap();
    assert!(started.status.success(), "{}", described(&started));
    for (args, refusal) in [
        (
            vec![
                "update",
                "--task-id",
                "SHOP-7",
                "--revision",
                "auto",
                "--progress",
                key,
            ],
            "possible AWS access key detected; working task not stored",
        ),
        (
            vec![
                "brain-update",
                "--record-id",
                "SHOP-7",
                "--revision",
                "1",
                "--progress",
                key,
            ],
            "possible AWS access key detected; Brain record not stored",
        ),
        (
            vec![
                "complete",
                "--task-id",
                "SHOP-7",
                "--revision",
                "1",
                "--outcome",
                key,
            ],
            "possible AWS access key detected; episode not stored",
        ),
        (
            vec![
                "msg-send",
                "--task-id",
                "SHOP-7",
                "--from",
                "main",
                "--to",
                "*",
                "--type",
                "finding",
                "--body",
                key,
            ],
            "possible AWS access key detected; message not stored",
        ),
    ] {
        let stderr = refused(&args);
        assert!(stderr.contains(refusal), "{args:?}: {stderr}");
    }
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
