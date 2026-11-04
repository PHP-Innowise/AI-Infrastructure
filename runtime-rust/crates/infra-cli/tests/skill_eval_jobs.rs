//! Actual skill-evaluation worker contracts using an explicit Rust fake provider.
//! Provider selection and fixture controls exist only in child environments.
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture {
    root: PathBuf,
    project: PathBuf,
    skill: PathBuf,
    eval: PathBuf,
}
impl Fixture {
    fn new(items: &Value) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "infra-skill-eval-jobs-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let project = root.join("private project");
        fs::create_dir_all(project.join(".agents")).unwrap();
        fs::create_dir_all(project.join(".codex")).unwrap();
        let skill = project.clone();
        fs::write(skill.join("SKILL.md"), "---\nname: job-fixture\ndescription: Exercise bounded fake evaluation jobs.\n---\n\n# Fixture\n").unwrap();
        let eval = project.join("eval.json");
        fs::write(&eval, serde_json::to_vec(items).unwrap()).unwrap();
        Self {
            root,
            project,
            skill,
            eval,
        }
    }
    fn command(&self, platform: &str, runs: usize, workers: usize) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_infra-engine"));
        command
            .args([
                "skill-tools",
                "run-eval",
                "--platform",
                platform,
                "--skill-path",
            ])
            .arg(&self.skill)
            .arg("--eval-set")
            .arg(&self.eval)
            .arg("--runs-per-query")
            .arg(runs.to_string())
            .arg("--num-workers")
            .arg(workers.to_string())
            .args(["--timeout", "10"])
            .current_dir(&self.project)
            .env(
                "SKILL_CREATOR_CODEX_CLI",
                env!("CARGO_BIN_EXE_infra-cli-eval-process-fixture"),
            )
            .env(
                "SKILL_CREATOR_CLAUDE_CLI",
                env!("CARGO_BIN_EXE_infra-cli-eval-process-fixture"),
            )
            .env("INFRA_SKILL_EVAL_FIXTURE_MODE", "provider-v1")
            .env("INFRA_SKILL_EVAL_FIXTURE_ROOT", &self.root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }
    fn run(&self, platform: &str, runs: usize, workers: usize, code: i32) -> Output {
        let output = self.command(platform, runs, workers).output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(code),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
    fn calls(&self) -> Vec<String> {
        match fs::read_to_string(self.root.join("calls.jsonl")) {
            Ok(text) => text
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => vec![],
            Err(error) => panic!("cannot inspect fake launches: {error}"),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn result(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid CLI JSON: {error}: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

#[test]
fn actual_native_workers_keep_input_order_and_repeat_each_item() {
    let fixture = Fixture::new(&json!([
        {"query":"first","should_trigger":true},
        {"query":"second","should_trigger":false}
    ]));
    // Both first-item repeats wait for second.started, so a third real
    // worker must reach the second item before the first two can finish.
    let output = fixture.run("codex", 2, 3, 0);
    let value = result(&output);
    assert_eq!(
        value["results"],
        json!([
            {"query":"first","should_trigger":true,"trigger_rate":1.0,"triggers":2,"runs":2,"pass":true},
            {"query":"second","should_trigger":false,"trigger_rate":0.0,"triggers":0,"runs":2,"pass":true}
        ])
    );
    assert_eq!(value["platform"], "codex");
    assert_eq!(value["summary"], json!({"total":2,"passed":2,"failed":0}));
    let calls = fixture.calls();
    assert_eq!(calls.iter().filter(|query| *query == "first").count(), 2);
    assert_eq!(calls.iter().filter(|query| *query == "second").count(), 2);
    assert!(output.stderr.is_empty());
}

#[test]
fn actual_legacy_workers_keep_duplicate_item_semantics() {
    let fixture = Fixture::new(&json!([
        {"query":"hit","should_trigger":true},
        {"query":"duplicate","should_trigger":true},
        {"query":"duplicate","should_trigger":false}
    ]));
    let output = fixture.run("claude", 1, 1, 0);
    let value = result(&output);
    assert_eq!(
        value["results"],
        json!([
            {"query":"hit","should_trigger":true,"trigger_rate":1.0,"triggers":1,"runs":1,"pass":true},
            {"query":"duplicate","should_trigger":false,"trigger_rate":0.0,"triggers":0,"runs":2,"pass":true}
        ])
    );
    assert!(value.get("platform").is_none());
    assert_eq!(fixture.calls(), ["hit", "duplicate", "duplicate"]);
    assert!(output.stderr.is_empty());
}

#[test]
fn invalid_or_oversized_jobs_launch_no_fake_providers() {
    for (items, runs, workers, message) in [
        (
            json!([{"query":"hit","should_trigger":true}]),
            1_000_001,
            1,
            "Evaluation exceeds the bounded job limit",
        ),
        (
            json!([{"query":"hit","should_trigger":true},{"query":"hit","should_trigger":false}]),
            1_000_001,
            1,
            "Duplicate eval query: hit",
        ),
        (
            json!([{"query":false,"should_trigger":true}]),
            1_000_001,
            1,
            "Every eval item requires a string query",
        ),
        (
            json!([{"query":"hit","should_trigger":true}]),
            1,
            0,
            "positive",
        ),
        (
            json!([{"query":"hit","should_trigger":true}]),
            0,
            1,
            "positive",
        ),
    ] {
        let fixture = Fixture::new(&items);
        let output = fixture.run("codex", runs, workers, 2);
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(message),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(fixture.calls().is_empty());
    }
}

#[test]
fn actual_native_failure_reports_the_first_received_error() {
    let fixture = Fixture::new(&json!([
        {"query":"native-error-one","should_trigger":false},
        {"query":"native-error-two","should_trigger":false}
    ]));
    let output = fixture.run("codex", 1, 1, 1);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Trigger probe failed for \"native-error-one\""),
        "{stderr}"
    );
    assert!(stderr.contains("native-error-one"));
    assert!(!stderr.contains("native-error-two"));
    assert_eq!(fixture.calls(), ["native-error-one", "native-error-two"]);
}

#[cfg(unix)]
#[test]
fn interrupted_cli_returns_before_warning_output() {
    use std::time::{Duration, Instant};
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    let fixture = Fixture::new(&json!([{"query":"block","should_trigger":false}]));
    let mut child = fixture.command("claude", 1, 1).spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !fixture.root.join("block.started").is_file() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "fake provider did not start: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    // Only this test's owned engine process receives POSIX SIGINT.
    let signalled = unsafe { kill(i32::try_from(child.id()).unwrap(), 2) };
    assert_eq!(signalled, 0);
    let output = child.wait_with_output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(130),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Trigger evaluation interrupted"),
        "{stderr}"
    );
    assert!(!stderr.contains("Warning:"), "{stderr}");
    assert_eq!(fixture.calls(), ["block"]);
}
