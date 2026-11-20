//! `skillcache-run` and `skillcache-tool` as the agent meets them: separate
//! short-lived commands over one state file, with fixture programs.
#[path = "support/scratch.rs"]
mod scratch;
use infra_skillcache::{SkillCapsule, package};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};

const FIXTURE: &str = env!("CARGO_BIN_EXE_infra-skillcache-program-fixture");
const SKILL: &str = ".claude/skills/pdf/SKILL.md";

fn runtime() -> String {
    Path::new(FIXTURE)
        .file_stem()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string()
}

/// The reference plan: an instruction that produces `quarter_files`, two
/// programs, a verification program and an inspection.
fn capsule() -> Value {
    let program = |id: &str, inputs: Value, outputs: Value, read_only: bool| {
        json!({
            "id": id,
            "description": format!("fixture stage {id}"),
            "entry": [runtime(), format!("programs/{id}/program.json")],
            "inputs": inputs,
            "outputs": outputs,
            "check": [runtime(), format!("programs/{id}/program.json"), "--check"],
            "timeout_seconds": 20,
            "read_only": read_only
        })
    };
    json!({
        "schema_version": 1,
        "form": "hybrid",
        "mu": {"name": "pdf-invoice-totals", "description": "Extract totals from invoice PDFs into a CSV summary"},
        "source": {"skills": [{"name": "pdf", "path": SKILL, "tree_sha256": "0".repeat(64)}]},
        "capabilities": {"network": "none", "runtimes": [runtime()]},
        "bindings": [
            {"name": "input_dir", "type": "path", "origin": "request", "description": "Folder with the invoices"},
            {"name": "currency", "type": "string", "origin": "request"}
        ],
        "programs": [
            program("extract", json!(["input_dir", "quarter_files"]), json!(["rows_file"]), false),
            program("summarize", json!(["rows_file", "currency"]), json!(["summary_file"]), false),
            program("verify_csv", json!(["summary_file", "quarter_files"]), json!([]), true)
        ],
        "steps": [
            {"kind": "instruction", "text": "Confirm which invoices belong to the requested quarter.", "source": {"path": SKILL, "section": "Selecting files"}, "produces": ["quarter_files"]},
            {"kind": "program", "program": "extract"},
            {"kind": "program", "program": "summarize"}
        ],
        "verification": [
            {"kind": "program", "program": "verify_csv"},
            {"kind": "inspection", "text": "Open the CSV and confirm one row per invoice."}
        ]
    })
}

fn specs() -> Vec<(&'static str, Value)> {
    vec![
        (
            "extract",
            json!({"result": {"rows_file": "out/rows.json", "invoices": 2},
                   "write": {"out/rows.json": "[]"}, "echo_input": true}),
        ),
        (
            "summarize",
            json!({"result": {"summary_file": "out/summary.csv", "total": "18240.00"},
                   "require_file": "out/rows.json", "write": {"out/summary.csv": "a,b\n"}}),
        ),
        ("verify_csv", json!({"result": {"rows": 2}})),
    ]
}

fn examples() -> Value {
    json!({
        "bindings": {"input_dir": "invoices/", "currency": "EUR", "quarter_files": ["q3-01.pdf", "q3-02.pdf"]},
        "steps": {
            "1": {"result": {"rows_file": "observed/rows.json"}},
            "2": {"result": {"summary_file": "observed/summary.csv"}}
        }
    })
}

struct Stand {
    root: scratch::Scratch,
    package: PathBuf,
    workspace: PathBuf,
    state_file: PathBuf,
}

impl Stand {
    fn new() -> Self {
        Self::with(capsule(), specs())
    }
    /// The folders of a run, without a capsule yet.
    fn empty() -> Self {
        let root = scratch::scratch();
        let workspace = root.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let state = root.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        Self {
            package: root.path().join("capsule/1/warm"),
            workspace,
            state_file: state.join("plan-state.json"),
            root,
        }
    }
    fn install(&self, capsule: Value, specs: Vec<(&str, Value)>) {
        let mut files = package::Files::new();
        for (id, spec) in specs {
            files.insert(
                &format!("programs/{id}/program.json"),
                serde_json::to_vec_pretty(&spec).unwrap(),
            );
        }
        files.insert("assets/columns.json", b"[]".to_vec());
        let parsed = SkillCapsule::parse(&serde_json::to_vec(&capsule).unwrap()).unwrap();
        package::write_warm(&self.package, &parsed, None, &files).unwrap();
    }
    fn with(capsule: Value, specs: Vec<(&str, Value)>) -> Self {
        let stand = Self::empty();
        stand.install(capsule, specs);
        stand
    }
    /// A copy of the reference stand whose program `id` behaves as `spec`.
    fn with_program(id: &'static str, spec: Value) -> Self {
        let mut specs = specs();
        specs.iter_mut().find(|(name, _)| *name == id).unwrap().1 = spec;
        Self::with(capsule(), specs)
    }
    fn command(&self, name: &str) -> Command {
        let mut command = Command::new(FIXTURE);
        let folder = Path::new(FIXTURE).parent().unwrap();
        let path = std::env::join_paths(
            std::iter::once(folder.to_path_buf())
                .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        command
            .arg(name)
            .current_dir(&self.workspace)
            .env("PATH", path)
            .env("SKILLCACHE_PACKAGE", &self.package)
            .env("SKILLCACHE_WORKSPACE", &self.workspace)
            .env("SKILLCACHE_STATE_FILE", &self.state_file)
            .env("SKILLCACHE_GENERATION", "generation-1")
            .env("GITHUB_TOKEN", "not-for-programs")
            .env("Database_Password", "not-for-programs")
            .env("OPENAI_API_KEY", "not-for-programs")
            .env("FIXTURE_PLAIN", "kept")
            .stdin(Stdio::null());
        command
    }
    fn run(&self, arguments: &[&str]) -> (i32, Value) {
        let output = self
            .command("skillcache-run")
            .args(arguments)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&output.stdout);
        let lines = text.lines().collect::<Vec<_>>();
        assert_eq!(
            lines.len(),
            1,
            "one JSON object on standard output, got {text:?} / {}",
            String::from_utf8_lossy(&output.stderr)
        );
        (
            output.status.code().unwrap(),
            serde_json::from_str(lines[0]).unwrap(),
        )
    }
    /// The answer of a command that must be accepted.
    fn ok(&self, arguments: &[&str]) -> Value {
        let (code, answer) = self.run(arguments);
        assert_eq!(code, 0, "{answer}");
        answer
    }
    fn refused(&self, arguments: &[&str]) -> String {
        let (code, answer) = self.run(arguments);
        assert_eq!(
            (code, answer["status"].as_str()),
            (2, Some("error")),
            "{answer}"
        );
        answer["code"].as_str().unwrap().to_string()
    }
    fn tool(&self, arguments: &[&str]) -> Output {
        self.command("skillcache-tool")
            .args(arguments)
            .output()
            .unwrap()
    }
    fn state(&self) -> Value {
        serde_json::from_slice(&std::fs::read(&self.state_file).unwrap()).unwrap()
    }
    fn events(&self) -> Vec<String> {
        infra_skillcache::state::read_log(&self.state_file, 0)
            .unwrap()
            .iter()
            .map(|event| {
                let step = event["step_index"]
                    .as_u64()
                    .map_or(String::new(), |index| format!(":{index}"));
                let attempt = event["attempt"]
                    .as_u64()
                    .filter(|_| event["event"] == "program_started")
                    .map_or(String::new(), |attempt| format!("#{attempt}"));
                format!("{}{step}{attempt}", event["event"].as_str().unwrap())
            })
            .collect()
    }
    fn starts(&self, program: &str) -> u64 {
        std::fs::read_to_string(self.workspace.join(format!(".fixture/{program}.count")))
            .map_or(0, |text| text.trim().parse().unwrap())
    }
    /// Everything the reference plan needs, as the argument of `run`.
    fn all_values() -> &'static str {
        r#"{"input_dir":"invoices/","currency":"EUR","quarter_files":["q3-01.pdf","q3-02.pdf"]}"#
    }
}

#[test]
fn executor_transitions_through_every_answer() {
    let stand = Stand::new();
    // Nothing is known yet: the opening instruction is the agent's.
    let answer = stand.ok(&["run"]);
    assert_eq!(answer["status"], "your_steps");
    assert_eq!(answer["steps"][0]["step_index"], 0);
    assert_eq!(answer["steps"][0]["kind"], "instruction");
    assert_eq!(answer["steps"][0]["source"]["section"], "Selecting files");
    assert_eq!(answer["produces"], json!(["quarter_files"]));
    assert_eq!(stand.state()["awaiting"]["kind"], "steps");
    // The step is not done until what it produces is supplied.
    let answer = stand.ok(&["continue"]);
    assert_eq!(answer["status"], "your_steps");
    assert_eq!(answer["missing"], json!(["quarter_files"]));
    // The first program needs a binding nobody supplied.
    let answer = stand.ok(&["continue", r#"{"quarter_files":["q3-01.pdf","q3-02.pdf"]}"#]);
    assert_eq!(answer["status"], "needs_bindings");
    assert_eq!(answer["step_index"], 1);
    assert_eq!(answer["names"], json!(["input_dir"]));
    assert_eq!(answer["bindings"][0]["type"], "path");
    assert_eq!(
        answer["bindings"][0]["description"],
        "Folder with the invoices"
    );
    // One program runs, the next needs another binding; the first one's
    // result comes back with the question.
    let answer = stand.ok(&["continue", r#"{"input_dir":"invoices/"}"#]);
    assert_eq!(answer["status"], "needs_bindings");
    assert_eq!(answer["names"], json!(["currency"]));
    assert_eq!(answer["results"]["extract"]["invoices"], 2);
    assert_eq!(
        answer["results"]["extract"]["input"],
        json!({"input_dir": "invoices/", "quarter_files": ["q3-01.pdf", "q3-02.pdf"]}),
        "a program receives exactly its declared inputs"
    );
    // Two consecutive programs run without returning to the agent.
    let answer = stand.ok(&["continue", r#"{"currency":"EUR"}"#]);
    assert_eq!(answer["status"], "your_steps");
    assert_eq!(answer["steps"][0]["kind"], "inspection");
    assert_eq!(answer["steps"][0]["step_index"], 4);
    assert_eq!(answer["results"]["summarize"]["total"], "18240.00");
    assert_eq!(answer["results"]["verify_csv"]["rows"], 2);
    assert!(answer["results"].get("extract").is_none());
    let status = stand.ok(&["status"]);
    assert_eq!(status["position"], 4);
    assert_eq!(status["finished"], false);
    let answer = stand.ok(&["continue"]);
    assert_eq!(answer["status"], "complete");
    assert_eq!(
        answer["result"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["extract", "summarize", "verify_csv"]
    );
    assert_eq!(answer["failed"], json!([]));
    assert_eq!(stand.ok(&["continue"])["status"], "complete");
    assert_eq!(stand.ok(&["status"])["finished"], true);
    let state = stand.state();
    assert_eq!(state["position"], 5);
    assert_eq!(state["awaiting"], json!({"kind": "none"}));
    assert_eq!(state["completed"], json!([0, 1, 2, 3, 4]));
    assert_eq!(state["generation"], "generation-1");
    assert_eq!(state["bindings"]["rows_file"], "out/rows.json");
    assert!(
        state["bindings"].get("total").is_none(),
        "an answer is not bound"
    );
    assert_eq!(
        stand.events(),
        [
            "started",
            "your_steps",
            "needs_bindings:1",
            "program_started:1#1",
            "program_completed:1",
            "needs_bindings:2",
            "program_started:2#1",
            "program_completed:2",
            "program_started:3#1",
            "program_completed:3",
            "your_steps",
            "complete",
        ]
    );
    // The journal names phases and never carries a value.
    let log =
        std::fs::read_to_string(infra_skillcache::state::log_path(&stand.state_file)).unwrap();
    for value in ["invoices/", "EUR", "q3-01.pdf", "18240.00", "out/rows.json"] {
        assert!(!log.contains(value), "the journal repeats `{value}`");
    }
}

#[test]
fn executor_runs_the_whole_plan_when_the_opening_values_are_known() {
    let stand = Stand::new();
    let answer = stand.ok(&["run", Stand::all_values()]);
    assert_eq!(answer["status"], "your_steps");
    assert_eq!(answer["steps"][0]["step_index"], 4);
    assert_eq!(
        answer["results"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["extract", "summarize", "verify_csv"]
    );
    assert_eq!(stand.ok(&["continue"])["status"], "complete");
    assert!(stand.workspace.join("out/summary.csv").is_file());
}

#[test]
fn executor_refuses_commands_outside_the_protocol() {
    let stand = Stand::new();
    assert_eq!(stand.refused(&["continue"]), "not_started");
    assert_eq!(stand.refused(&["status"]), "not_started");
    assert_eq!(stand.refused(&["run", "{not json"]), "invalid_values");
    assert_eq!(stand.refused(&["run", "[1]"]), "invalid_values");
    assert_eq!(
        stand.refused(&["run", r#"{"rows_file":"x"}"#]),
        "unexpected_name"
    );
    assert_eq!(
        stand.refused(&["run", r#"{"currency":7}"#]),
        "type_mismatch"
    );
    assert_eq!(
        stand.refused(&["run", r#"{"currency":{"a":1}}"#]),
        "invalid_values"
    );
    let large = format!(r#"{{"currency":"{}"}}"#, "x".repeat(5000));
    assert_eq!(stand.refused(&["run", &large]), "value_too_large");
    assert_eq!(stand.refused(&["bogus"]), "usage");
    // A refused command starts nothing.
    assert!(!stand.state_file.exists());
    stand.ok(&["run", r#"{"currency":"EUR"}"#]);
    assert_eq!(stand.refused(&["run"]), "already_started");
    assert_eq!(
        stand.refused(&["continue", r#"{"currency":"USD"}"#]),
        "already_bound"
    );
    // The same value again is not a change.
    assert_eq!(
        stand.ok(&["continue", r#"{"currency":"EUR"}"#])["status"],
        "your_steps"
    );
    // The values can come on standard input, for text a shell would mangle.
    let mut child = stand
        .command("skillcache-run")
        .args(["continue", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(br#"{"quarter_files":["it's.pdf"]}"#)
            .unwrap();
    }
    let output = child.wait_with_output().unwrap();
    let answer: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(answer["status"], "needs_bindings");
    assert_eq!(
        stand.state()["bindings"]["quarter_files"],
        json!(["it's.pdf"])
    );
}

#[test]
fn executor_persists_state_before_program() {
    // The program copies the state file it is never told about, at the moment
    // it runs: what it sees is what was on disk before it started.
    let stand = Stand::empty();
    let seen = stand.root.path().join("seen-state.json");
    let mut specs = specs();
    specs[0].1 = json!({
        "result": {"rows_file": "out/rows.json"},
        "write": {"out/rows.json": "[]"},
        "copy": [[stand.state_file, seen]],
        "env_names_to": "env-names.txt"
    });
    stand.install(capsule(), specs);
    let answer = stand.ok(&["run", Stand::all_values()]);
    assert_eq!(answer["status"], "your_steps", "{answer}");
    let state: Value = serde_json::from_slice(&std::fs::read(&seen).unwrap()).unwrap();
    assert_eq!(
        state["awaiting"],
        json!({"kind": "running", "step_index": 1, "attempt": 1})
    );
    assert_eq!(state["position"], 1);
    assert_eq!(state["completed"], json!([0]));
    // The program's environment: the contract's variables, nothing that looks
    // like a credential, and not the executor's own state.
    let names = std::fs::read_to_string(stand.workspace.join("env-names.txt")).unwrap();
    let names = names.lines().collect::<Vec<_>>();
    for expected in [
        "SKILLCACHE_INPUT",
        "SKILLCACHE_WORKSPACE",
        "SKILLCACHE_PACKAGE",
        "SKILLCACHE_ASSETS",
        "PATH",
        "FIXTURE_PLAIN",
    ] {
        assert!(
            names.contains(&expected),
            "{expected} is missing from {names:?}"
        );
    }
    for withheld in [
        "GITHUB_TOKEN",
        "Database_Password",
        "OPENAI_API_KEY",
        "SKILLCACHE_STATE_FILE",
        "SKILLCACHE_GENERATION",
    ] {
        assert!(!names.contains(&withheld), "{withheld} reached a program");
    }
}

#[test]
fn executor_retries_one_nonzero_exit() {
    let stand = Stand::with_program(
        "extract",
        json!({"result": {"rows_file": "out/rows.json"}, "write": {"out/rows.json": "[]"}, "fail_times": 1}),
    );
    let answer = stand.ok(&["run", Stand::all_values()]);
    assert_eq!(answer["status"], "your_steps", "{answer}");
    assert_eq!(stand.starts("extract"), 2);
    assert_eq!(stand.state()["failed"], json!([]));
    assert_eq!(
        &stand.events()[1..4],
        [
            "program_started:1#1",
            "program_started:1#2",
            "program_completed:1"
        ]
    );
}

#[test]
fn executor_deopt_after_two_nonzero() {
    let stand = Stand::with_program(
        "extract",
        json!({"exit": 3, "stderr": "boom: no reader for PDF"}),
    );
    let answer = stand.ok(&["run", Stand::all_values()]);
    assert_eq!(answer["status"], "program_failed");
    assert_eq!(answer["step_index"], 1);
    assert_eq!(answer["program"], "extract");
    assert_eq!(answer["diagnostics"]["reason"], "nonzero_exit");
    assert_eq!(answer["diagnostics"]["exit_code"], 3);
    assert_eq!(answer["diagnostics"]["attempts"], 2);
    assert!(
        answer["diagnostics"]["detail"]
            .as_str()
            .unwrap()
            .contains("boom: no reader for PDF")
    );
    assert_eq!(answer["required_outputs"], json!(["rows_file"]));
    // A plan step's source is offered, so the agent can see what it was for.
    let source = PathBuf::from(answer["program_source_path"].as_str().unwrap());
    assert_eq!(source, stand.package.join("programs/extract/program.json"));
    assert!(source.is_file());
    let instruction = answer["instruction"].as_str().unwrap();
    assert!(instruction.contains(SKILL) && instruction.contains("fixture stage extract"));
    assert!(instruction.contains("Do not repair or re-run the program"));
    let state = stand.state();
    assert_eq!(
        state["failed"],
        json!([{"step_index": 1, "reason": "nonzero_exit"}])
    );
    assert_eq!(
        state["awaiting"],
        json!({"kind": "outputs", "step_index": 1, "names": ["rows_file"]})
    );
    assert_eq!(state["position"], 1);
    assert_eq!(
        &stand.events()[1..],
        [
            "program_started:1#1",
            "program_started:1#2",
            "program_failed:1"
        ]
    );
}

#[test]
fn executor_deopt_timeout() {
    let mut value = capsule();
    value["programs"][0]["timeout_seconds"] = 1.into();
    let mut specs = specs();
    specs[0].1 = json!({"sleep_ms": 30_000, "result": {"rows_file": "x"}});
    let stand = Stand::with(value, specs);
    let started = Instant::now();
    let answer = stand.ok(&["run", Stand::all_values()]);
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "the program was not stopped"
    );
    assert_eq!(answer["status"], "program_failed");
    assert_eq!(answer["diagnostics"]["reason"], "timeout");
    // A timeout is not retried.
    assert_eq!(answer["diagnostics"]["attempts"], 1);
    assert_eq!(
        &stand.events()[1..],
        ["program_started:1#1", "program_failed:1"]
    );
}

#[test]
fn executor_deopt_missing_output() {
    let stand = Stand::with_program("extract", json!({"result": {"invoices": 2}}));
    let answer = stand.ok(&["run", Stand::all_values()]);
    assert_eq!(answer["status"], "program_failed");
    assert_eq!(answer["diagnostics"]["reason"], "missing_output");
    assert_eq!(answer["diagnostics"]["attempts"], 1);
    assert!(
        answer["diagnostics"]["detail"]
            .as_str()
            .unwrap()
            .contains("rows_file")
    );
    // Text around the JSON object breaks the contract just the same.
    let stand = Stand::with_program(
        "extract",
        json!({"stdout": "done!\n{\"status\":\"complete\",\"result\":{\"rows_file\":\"x\"}}\n"}),
    );
    let answer = stand.ok(&["run", Stand::all_values()]);
    assert_eq!(answer["diagnostics"]["reason"], "invalid_stdout");
    assert_eq!(answer["diagnostics"]["attempts"], 1);
}

#[test]
fn continue_after_program_failed_requires_declared_outputs() {
    let stand = Stand::with_program("extract", json!({"exit": 3, "fail_times": 99}));
    assert_eq!(
        stand.ok(&["run", Stand::all_values()])["status"],
        "program_failed"
    );
    assert_eq!(stand.starts("extract"), 2);
    // Without the outputs later programs read, the step is still open.
    let answer = stand.ok(&["continue"]);
    assert_eq!(answer["status"], "program_failed");
    assert_eq!(answer["required_outputs"], json!(["rows_file"]));
    assert_eq!(answer["diagnostics"]["reason"], "nonzero_exit");
    assert_eq!(stand.state()["position"], 1);
    assert_eq!(
        stand.refused(&["continue", r#"{"summary_file":"x"}"#]),
        "unexpected_name"
    );
    // The agent did the step by hand and hands over what the next program reads.
    std::fs::create_dir_all(stand.workspace.join("out")).unwrap();
    std::fs::write(stand.workspace.join("out/rows.json"), "[]").unwrap();
    let answer = stand.ok(&["continue", r#"{"rows_file":"out/rows.json"}"#]);
    // continue_skips_failed_step: the failed program is not started again and
    // the rest of the plan still runs compiled.
    assert_eq!(answer["status"], "your_steps", "{answer}");
    assert_eq!(answer["steps"][0]["step_index"], 4);
    assert_eq!(stand.starts("extract"), 2);
    assert_eq!(answer["results"]["summarize"]["total"], "18240.00");
    let complete = stand.ok(&["continue"]);
    assert_eq!(complete["status"], "complete");
    assert_eq!(
        complete["failed"],
        json!([{"step_index": 1, "reason": "nonzero_exit"}])
    );
    assert!(complete["result"].get("extract").is_none());
    let state = stand.state();
    assert_eq!(state["completed"], json!([0, 2, 3, 4]));
    assert_eq!(state["bindings"]["rows_file"], "out/rows.json");
}

#[test]
fn tool_refuses_failed_program() {
    let stand = Stand::with_program("extract", json!({"exit": 3, "fail_times": 99}));
    assert_eq!(
        stand.ok(&["run", Stand::all_values()])["status"],
        "program_failed"
    );
    let output = stand.tool(&[
        "extract",
        "--input",
        r#"{"input_dir":"invoices/","quarter_files":[]}"#,
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("refused_failed_program"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        stand.starts("extract"),
        2,
        "the failed program must not start"
    );
    // Another program of the same capsule is not affected.
    let output = stand.tool(&[
        "verify_csv",
        "--input",
        r#"{"summary_file":"x","quarter_files":[]}"#,
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let answer: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(answer, json!({"status": "complete", "result": {"rows": 2}}));
    // The self-test of a program runs through the same tool.
    let output = stand.tool(&["verify_csv", "--check"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({"status": "complete", "result": {}})
    );
    assert_eq!(stand.tool(&["unknown", "--check"]).status.code(), Some(2));
    assert_eq!(stand.tool(&["verify_csv"]).status.code(), Some(2));
    // A failing program is exit code 1 with its reason on standard error.
    let stand = Stand::with_program("verify_csv", json!({"exit": 5, "stderr": "rows differ"}));
    let output = stand.tool(&[
        "verify_csv",
        "--input",
        r#"{"summary_file":"x","quarter_files":[]}"#,
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("nonzero_exit") && error.contains("rows differ"),
        "{error}"
    );
}

#[test]
fn program_failed_verification_has_no_source_path() {
    let stand = Stand::with_program(
        "verify_csv",
        json!({"exit": 7, "stderr": "one row is missing"}),
    );
    let answer = stand.ok(&["run", Stand::all_values()]);
    assert_eq!(answer["status"], "program_failed");
    assert_eq!(answer["step_index"], 3);
    assert!(answer.get("program_source_path").is_none(), "{answer}");
    assert_eq!(answer["required_outputs"], json!([]));
    // The plan's own programs still report their results.
    assert_eq!(answer["results"]["summarize"]["total"], "18240.00");
    // A verification step without outputs just lets the plan go on.
    let answer = stand.ok(&["continue"]);
    assert_eq!(answer["status"], "your_steps");
    assert_eq!(answer["steps"][0]["step_index"], 4);
    let complete = stand.ok(&["continue"]);
    assert_eq!(
        complete["failed"],
        json!([{"step_index": 3, "reason": "nonzero_exit"}])
    );
}

#[cfg(unix)]
#[test]
fn state_attempt_survives_crash() {
    // The first start of `extract` hangs; the command is killed while it runs.
    let stand = Stand::with_program(
        "extract",
        json!({"result": {"rows_file": "out/rows.json"}, "write": {"out/rows.json": "[]"},
               "sleep_first_ms": 60_000}),
    );
    let mut child = stand
        .command("skillcache-run")
        .args(["run", Stand::all_values()])
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while stand.starts("extract") == 0 {
        assert!(Instant::now() < deadline, "the program never started");
        std::thread::sleep(Duration::from_millis(20));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(
        stand.state()["awaiting"],
        json!({"kind": "running", "step_index": 1, "attempt": 1}),
        "the attempt was on disk before the program started"
    );
    // The program does not outlive the command that started it.
    #[cfg(target_os = "linux")]
    {
        let package = stand.package.display().to_string();
        let running = || {
            std::fs::read_dir("/proc")
                .unwrap()
                .filter_map(|entry| std::fs::read(entry.ok()?.path().join("cmdline")).ok())
                .any(|line| String::from_utf8_lossy(&line).contains(&package))
        };
        let deadline = Instant::now() + Duration::from_secs(20);
        while running() {
            assert!(
                Instant::now() < deadline,
                "the program outlived its command"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    // `continue` runs the interrupted step once more, as its second attempt.
    let answer = stand.ok(&["continue"]);
    assert_eq!(answer["status"], "your_steps", "{answer}");
    assert_eq!(stand.starts("extract"), 2);
    let events = stand.events();
    assert_eq!(
        &events[1..5],
        [
            "program_started:1#1",
            "interrupted:1",
            "program_started:1#2",
            "program_completed:1"
        ]
    );
}

#[test]
fn an_interrupted_second_attempt_is_the_agents_step() {
    let stand = Stand::with_program(
        "extract",
        json!({"result": {"rows_file": "out/rows.json"}, "fail_times": 0}),
    );
    assert_eq!(
        stand.ok(&["run", r#"{"currency":"EUR"}"#])["status"],
        "your_steps"
    );
    // The state a command killed during a second attempt leaves behind.
    let mut state = stand.state();
    state["awaiting"] = json!({"kind": "running", "step_index": 1, "attempt": 2});
    state["position"] = 1.into();
    state["completed"] = json!([0]);
    state["bindings"] = json!({"currency": "EUR", "input_dir": "invoices/", "quarter_files": []});
    std::fs::write(&stand.state_file, serde_json::to_vec(&state).unwrap()).unwrap();
    let answer = stand.ok(&["continue"]);
    assert_eq!(answer["status"], "program_failed");
    assert_eq!(answer["diagnostics"]["reason"], "interrupted");
    assert_eq!(answer["required_outputs"], json!(["rows_file"]));
    assert_eq!(
        stand.starts("extract"),
        0,
        "an interrupted second attempt is not run a third time"
    );
    assert!(
        answer["diagnostics"]["note"]
            .as_str()
            .unwrap()
            .contains("nothing was rolled back")
    );
    assert_eq!(
        stand.state()["failed"],
        json!([{"step_index": 1, "reason": "interrupted"}])
    );
}

#[test]
fn a_second_command_on_the_same_run_is_refused_while_the_first_works() {
    let stand = Stand::with_program(
        "extract",
        json!({"result": {"rows_file": "out/rows.json"}, "write": {"out/rows.json": "[]"},
               "sleep_first_ms": 3000}),
    );
    let child = stand
        .command("skillcache-run")
        .args(["run", Stand::all_values()])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while stand.starts("extract") == 0 {
        assert!(Instant::now() < deadline, "the program never started");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(stand.refused(&["status"]), "busy");
    assert_eq!(stand.refused(&["continue"]), "busy");
    let output = child.wait_with_output().unwrap();
    let answer: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(answer["status"], "your_steps");
    assert_eq!(stand.ok(&["status"])["position"], 4);
}

#[test]
fn host_mode_digest_unchanged() {
    // The package is read-only: a program that tries to write into it cannot,
    // and the run goes on.
    let stand = Stand::with_program(
        "extract",
        json!({"result": {"rows_file": "out/rows.json"}, "write": {"out/rows.json": "[]"},
               "touch_package": "assets/columns.json"}),
    );
    let before = package::digest(&stand.package).unwrap();
    let answer = stand.ok(&["run", Stand::all_values()]);
    #[cfg(unix)]
    {
        assert_eq!(answer["status"], "your_steps", "{answer}");
        assert_eq!(package::digest(&stand.package).unwrap(), before);
    }
    #[cfg(not(unix))]
    let _ = (answer, before);
    // A package that was made writable and then changed by its own program is
    // noticed: the step fails and names why.
    let stand = Stand::with_program(
        "extract",
        json!({"result": {"rows_file": "out/rows.json"}, "touch_package": "assets/columns.json"}),
    );
    make_writable(&stand.package);
    let answer = stand.ok(&["run", Stand::all_values()]);
    assert_eq!(answer["status"], "program_failed", "{answer}");
    assert_eq!(answer["diagnostics"]["reason"], "package_modified");
    assert_eq!(answer["diagnostics"]["attempts"], 1);
}

fn make_writable(root: &Path) {
    let mut permissions = std::fs::metadata(root).unwrap().permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(if root.is_dir() { 0o755 } else { 0o644 });
    }
    #[cfg(not(unix))]
    permissions.set_readonly(false);
    std::fs::set_permissions(root, permissions).unwrap();
    if root.is_dir() {
        for entry in std::fs::read_dir(root).unwrap() {
            make_writable(&entry.unwrap().path());
        }
    }
}

#[test]
fn a_changed_or_invalid_capsule_is_not_run() {
    let stand = Stand::new();
    stand.ok(&["run", r#"{"currency":"EUR"}"#]);
    // Another capsule under the same state: the run does not continue on it.
    let mut value = capsule();
    value["mu"]["description"] = "Another capsule".into();
    let changed = Stand::with(value, specs());
    let output = stand
        .command("skillcache-run")
        .env("SKILLCACHE_PACKAGE", &changed.package)
        .arg("continue")
        .output()
        .unwrap();
    let answer: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(answer["code"], "capsule_changed", "{answer}");
    // A package edited by hand into something the gate refuses does not run.
    let tampered = Stand::new();
    make_writable(&tampered.package);
    let path = tampered.package.join("capsule.json");
    let mut value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    value["programs"][0]["entry"] = json!([runtime(), "-c", "anything"]);
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(tampered.refused(&["run"]), "invalid_capsule");
    let output = tampered.tool(&["extract", "--check"]);
    assert_eq!(output.status.code(), Some(2));
    // Without the session's environment the commands explain themselves.
    let output = Command::new(FIXTURE)
        .args(["skillcache-run", "status"])
        .env_remove("SKILLCACHE_PACKAGE")
        .output()
        .unwrap();
    let answer: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(answer["code"], "not_configured");
}

#[test]
fn replay_passes_with_the_observed_values() {
    let stand = Stand::new();
    let observed = stand.root.path().join("examples.json");
    std::fs::write(&observed, serde_json::to_vec(&examples()).unwrap()).unwrap();
    let (code, report) = stand.run(&["run", "--replay", observed.to_str().unwrap()]);
    assert_eq!(code, 0, "{report}");
    assert_eq!(report["status"], "replay");
    assert_eq!(report["passed"], true);
    assert_eq!(report["completed"], json!([0, 1, 2, 3, 4]));
    assert_eq!(report["failures"], json!([]));
    assert_eq!(report["substituted"], json!([]));
    // The replay ran the programs for real, in the workspace it was given.
    assert!(stand.workspace.join("out/summary.csv").is_file());
    assert_eq!(stand.state()["bindings"]["rows_file"], "out/rows.json");
    // A replay starts from nothing.
    assert_eq!(
        stand.refused(&["run", "--replay", observed.to_str().unwrap()]),
        "already_started"
    );
}

#[test]
fn gate3_substitutes_examples_and_collects_downstream() {
    // `extract` fails, and `summarize` fails for a reason of its own. One
    // pass reports both: the observed output stands in for the first failure.
    let mut specs = specs();
    specs[0].1 = json!({"exit": 3, "stderr": "extract cannot read"});
    specs[1].1 = json!({"exit": 4, "stderr": "summarize cannot write"});
    // The verification program is declared read-only and writes a file.
    specs[2].1 = json!({"result": {"rows": 2}, "write": {"verified.txt": "x"}});
    let stand = Stand::with(capsule(), specs);
    let observed = stand.root.path().join("examples.json");
    std::fs::write(&observed, serde_json::to_vec(&examples()).unwrap()).unwrap();
    let (code, report) = stand.run(&["run", "--replay", observed.to_str().unwrap()]);
    assert_eq!(code, 1, "{report}");
    assert_eq!(report["passed"], false);
    let failures = report["failures"].as_array().unwrap();
    assert_eq!(
        failures
            .iter()
            .map(|failure| (
                failure["step_index"].as_u64().unwrap(),
                failure["diagnostics"]["reason"].as_str().unwrap()
            ))
            .collect::<Vec<_>>(),
        [
            (1, "nonzero_exit"),
            (2, "nonzero_exit"),
            (3, "read_only_violation")
        ]
    );
    assert!(
        failures[0]["diagnostics"]["detail"]
            .as_str()
            .unwrap()
            .contains("extract cannot read")
    );
    assert_eq!(report["substituted"], json!([1, 2, 3]));
    assert_eq!(report["completed"], json!([0, 4]));
    // The observed outputs were bound in place of the failed programs'.
    let state = stand.state();
    assert_eq!(state["bindings"]["rows_file"], "observed/rows.json");
    assert_eq!(state["bindings"]["summary_file"], "observed/summary.csv");
}

#[test]
fn replay_needs_an_observed_value_for_every_question() {
    let stand = Stand::new();
    let mut observed = examples();
    observed["bindings"]
        .as_object_mut()
        .unwrap()
        .remove("currency");
    let path = stand.root.path().join("examples.json");
    std::fs::write(&path, serde_json::to_vec(&observed).unwrap()).unwrap();
    assert_eq!(
        stand.refused(&["run", "--replay", path.to_str().unwrap()]),
        "example_missing"
    );
    std::fs::write(&path, b"{\"bindings\": []}").unwrap();
    let fresh = Stand::new();
    assert_eq!(
        fresh.refused(&["run", "--replay", path.to_str().unwrap()]),
        "invalid_examples"
    );
}
