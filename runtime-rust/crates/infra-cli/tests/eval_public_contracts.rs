//! Public migration contracts use disposable local state and never invoke models.
#[path = "support/executable.rs"]
mod executable;
use infra_cli::arguments;
use infra_core::ErrorKind;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

struct Fixture {
    root: PathBuf,
    project: PathBuf,
    claude: PathBuf,
    codex: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "infra-eval-contracts-{}",
            infra_core::domain::uuid4()
        ));
        fs::create_dir(&root).unwrap();
        let root = infra_core::fs::canonicalize(root).unwrap();
        let project = root.join("project with spaces");
        let claude = root.join("empty claude transcripts");
        let codex = root.join("empty codex transcripts");
        for directory in [
            project.join("project-brain/config"),
            claude.clone(),
            codex.clone(),
            root.join("tmp"),
        ] {
            fs::create_dir_all(directory).unwrap();
        }
        fs::write(
            project.join("project-brain/config/runtime.json"),
            br#"{"mode":"lightweight"}"#,
        )
        .unwrap();
        Self {
            root,
            project,
            claude,
            codex,
        }
    }
    fn run(&self, args: &[String]) -> Output {
        self.run_with(Path::new(env!("CARGO_BIN_EXE_infra-engine")), args)
    }
    fn run_with(&self, binary: &Path, args: &[String]) -> Output {
        Command::new(binary)
            .args(args)
            .current_dir(&self.project)
            .env("HOME", self.root.join("home"))
            .env("USERPROFILE", self.root.join("home"))
            .env("CODEX_HOME", self.root.join("codex-home"))
            .env("TMPDIR", self.root.join("tmp"))
            .env("TMP", self.root.join("tmp"))
            .env("TEMP", self.root.join("tmp"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("INFRA_SOURCE_ROOT")
            .env_remove("INFRA_TOOLING_ROOT")
            .env_remove("ACCELERATOR_HOME")
            .env_remove("ACCELERATOR_STATE_DIR")
            .env_remove("ACCELERATOR_PROJECT_DIR")
            .env_remove("ACCELERATOR_TOOLING_DIR")
            .env_remove("PROJECT_BRAIN_OWNER")
            .env_remove("PROJECT_BRAIN_MODE")
            .env_remove("CONTEXT_TASK_ID")
            .env_remove("MEMORY_EVAL_CLOCK")
            .env_remove("GIT_DIR")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_CONFIG")
            .env_remove("GIT_CONFIG_PARAMETERS")
            .env_remove("GIT_CONFIG_COUNT")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
    fn expect(&self, args: &[String], code: i32) -> Output {
        let output = self.run(args);
        assert_eq!(
            output.status.code(),
            Some(code),
            "{args:?}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        output
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn bench_optional_values_keep_constants_and_explicit_choices() {
    for (operation, flag, key, expected) in [
        ("eval/codex-bench", "--worktree", "worktree", "inside"),
        ("eval/codex-bench", "--attached", "attached", ""),
        ("eval/claude-code-bench", "--attached", "attached", ""),
        ("eval/claude-code-bench", "--cli", "cli", "retrieve"),
    ] {
        for args in [strings(&[flag]), strings(&[flag, "--keep"])] {
            let parsed = arguments::parse(&args, operation).unwrap();
            assert_eq!(parsed.value[key], expected, "{operation} {flag}");
            if args.len() == 2 {
                assert_eq!(parsed.value["keep"], true);
            }
        }
    }
    assert_eq!(
        arguments::parse(&strings(&["--worktree=beside"]), "eval/codex-bench")
            .unwrap()
            .value["worktree"],
        "beside"
    );
    assert_eq!(
        arguments::parse(&strings(&["--cli", "checkpoint"]), "eval/claude-code-bench")
            .unwrap()
            .value["cli"],
        "checkpoint"
    );
    assert!(arguments::parse(&strings(&["--worktree", "elsewhere"]), "eval/codex-bench").is_err());
    assert!(arguments::parse(&strings(&["--cli", "elsewhere"]), "eval/claude-code-bench").is_err());
}

#[test]
fn onnx_remainder_preserves_the_exact_inner_command() {
    let remainder = strings(&[
        "retrieve",
        "literal ; $(text)",
        "--json",
        "--host",
        "cursor",
        "-h",
    ]);
    for separator in [false, true] {
        let mut raw = strings(&[
            "--context",
            "/native/engine",
            "--assets",
            "/assets",
            "--ort-library",
            "/ort",
            "--stats",
            "/private/stats",
        ]);
        if separator {
            raw.push("--".into());
        }
        raw.extend(remainder.clone());
        let parsed = arguments::parse(&raw, "eval/memory-onnx-pilot").unwrap();
        assert_eq!(parsed.value["_remainder"], json!(remainder));
        assert!(parsed.value["json"].is_null());
        assert!(parsed.value["host"].is_null());
        assert!(parsed.value["help"].is_null());
        assert!(!arguments::help_requested(&raw, "eval/memory-onnx-pilot").unwrap());
    }
}

#[test]
fn invalid_eval_choices_refuse_before_output_or_cache_creation() {
    let f = Fixture::new();
    let set = f.root.join("set.json");
    let judgments = f.root.join("judgments.json");
    let out = f.root.join("result.json");
    let cache = f.root.join("must not create cache");
    fs::write(&set, b"[]").unwrap();
    fs::write(&judgments, b"{}").unwrap();
    let args = vec![
        "eval".into(),
        "memory-eval".into(),
        "run".into(),
        "--set".into(),
        set.display().to_string(),
        "--judgments".into(),
        judgments.display().to_string(),
        "--out".into(),
        out.display().to_string(),
        "--cache".into(),
        cache.display().to_string(),
        "--clock".into(),
        "typo".into(),
    ];
    let output = f.expect(&args, 2);
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--clock"));
    assert!(!out.exists());
    assert!(!cache.exists());
    for key in ["host", "as_of", "clock", "updated_after"] {
        let mut request = json!({"cache":cache,"out":out});
        request[key] = "typo".into();
        let error = infra_eval::control::eval_run(&request).unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidInput);
        assert!(error.message.contains(&key.replace('_', "-")));
        assert!(!out.exists());
        assert!(!cache.exists());
    }
}

#[test]
fn realized_rejects_missing_or_regular_file_state_without_a_report() {
    let f = Fixture::new();
    let missing = f.root.join("missing state");
    let file = f.root.join("regular state file");
    fs::write(&file, b"not a state directory").unwrap();
    for state in [&missing, &file] {
        let args = vec![
            "eval".into(),
            "memory-eval".into(),
            "realized".into(),
            "--project".into(),
            f.project.display().to_string(),
            "--state".into(),
            state.display().to_string(),
            "--claude-root".into(),
            f.claude.display().to_string(),
            "--codex-root".into(),
            f.codex.display().to_string(),
            "--json".into(),
        ];
        let output = f.expect(&args, 2);
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("--state"));
        assert!(!missing.exists());
        assert!(!f.project.join("memory-bank/local").exists());
        let error = infra_eval::field::realized_options(
            &f.project, state, &f.claude, &f.codex, None, false, false,
        )
        .unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidInput);
    }
}

#[test]
fn onnx_status_remainder_does_not_load_assets_or_run_a_worker() {
    let f = Fixture::new();
    let engine_dir = f.root.join("native executable");
    fs::create_dir(&engine_dir).unwrap();
    let engine = engine_dir.join(if cfg!(windows) {
        "infra-engine.exe"
    } else {
        "infra-engine"
    });
    executable::copy(env!("CARGO_BIN_EXE_infra-engine"), &engine);
    let assets = f.root.join("absent model assets");
    let ort = f.root.join("absent ORT library");
    let worker = f.root.join("absent worker");
    for separator in [false, true] {
        let stats = f.root.join(format!("stats-{separator}.json"));
        let mut args = vec![
            "eval".into(),
            "memory-onnx-pilot".into(),
            "--context".into(),
            engine.display().to_string(),
            "--root".into(),
            f.project.display().to_string(),
            "--assets".into(),
            assets.display().to_string(),
            "--ort-library".into(),
            ort.display().to_string(),
            "--worker".into(),
            worker.display().to_string(),
            "--stats".into(),
            stats.display().to_string(),
        ];
        if separator {
            args.push("--".into());
        }
        args.extend(strings(&["status", "--json"]));
        let output = f.expect(&args, 0);
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(value["documents"].is_number());
        assert_eq!(value["workspace"]["project"], json!(f.project));
        let counters: Value = serde_json::from_slice(&fs::read(&stats).unwrap()).unwrap();
        assert_eq!(counters, json!({}));
        assert!(!assets.exists());
        assert!(!ort.exists());
        assert!(!worker.exists());
    }
}

#[test]
fn unified_pilot_input_errors_exit_two_and_unavailable_probe_remains_one() {
    let f = Fixture::new();
    let lexical = f.root.join("lexical.json");
    let dense = f.root.join("dense.json");
    fs::write(&lexical, br#"["a.md"]"#).unwrap();
    fs::write(&dense, br#"["b.md"]"#).unwrap();
    let args = |k: &str| {
        vec![
            "pilot".into(),
            "sqlite-vec".into(),
            "fuse".into(),
            "--lexical".into(),
            lexical.display().to_string(),
            "--dense".into(),
            dense.display().to_string(),
            "--k".into(),
            k.into(),
            "--json".into(),
        ]
    };
    let output = f.expect(&args("0"), 2);
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("k must be positive"));
    fs::write(&lexical, b"[1]").unwrap();
    assert!(f.expect(&args("60"), 2).stdout.is_empty());
    let probe = f.expect(&strings(&["pilot", "sqlite-vec", "probe", "--json"]), 1);
    let value: Value = serde_json::from_slice(&probe.stdout).unwrap();
    assert_eq!(value["available"], false);
    assert_eq!(value["reason"], "vec-extension-missing");
}

#[test]
fn sealing_options_include_dynamic_design_and_have_command_scoped_requiredness() {
    let raw = strings(&[
        "--set",
        "set.json",
        "--verifiers",
        "cases.json",
        "--design",
        "design.json",
        "--out",
        "protocol.json",
        "--cache",
        "private-cache",
        "--model",
        "model-name",
    ]);
    let parsed = arguments::parse(&raw, "eval/memory-outcome-ab/seal").unwrap();
    assert_eq!(parsed.value["design"], "design.json");
    assert_eq!(parsed.value["repeat"], 1);
    assert_eq!(parsed.value["cap"], 40);
    assert_eq!(parsed.value["arms"], "off,capsule");
    let without_design = raw
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != 4 && *index != 5)
        .map(|(_, value)| value.clone())
        .collect::<Vec<_>>();
    let error = arguments::parse(&without_design, "eval/memory-outcome-ab/seal").unwrap_err();
    assert!(error.message.contains("--design"));
    let error = arguments::parse(
        &strings(&["result.json", "--design", "design.json"]),
        "eval/memory-outcome-ab/report",
    )
    .unwrap_err();
    assert!(error.message.contains("--design"));
}

#[test]
fn missing_recall_selection_arguments_leave_no_output_or_cache() {
    let f = Fixture::new();
    let out = f.root.join("must not write recall.json");
    let sources = f.root.join("sources.json");
    let scenarios = f.root.join("scenarios.json");
    fs::write(&sources, br#"{"sources":[{"id":"one"}]}"#).unwrap();
    fs::write(&scenarios, b"[]").unwrap();
    for (operation, tail, missing) in [
        (
            "recall-sources",
            strings(&["--before", "2026-01-01"]),
            "--projects",
        ),
        (
            "recall-set",
            vec![
                "--sources".into(),
                sources.display().to_string(),
                "--scenarios".into(),
                scenarios.display().to_string(),
                "--seed".into(),
                "7".into(),
            ],
            "--n",
        ),
    ] {
        let mut args = strings(&["eval", "memory-outcome-ab", operation]);
        args.extend(tail);
        args.extend(vec!["--out".into(), out.display().to_string()]);
        let output = f.expect(&args, 2);
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains(missing));
        assert!(!out.exists());
        assert_eq!(fs::read_dir(f.root.join("tmp")).unwrap().count(), 0);
    }
}

#[test]
fn invalid_outcome_endpoint_refuses_before_creating_work_or_result() {
    let f = Fixture::new();
    let set = f.root.join("empty tasks.json");
    let out = f.root.join("must not write outcome.json");
    let cache = f.root.join("must not create cache");
    fs::write(&set, br#"{"tasks":[]}"#).unwrap();
    let args = vec![
        "eval".into(),
        "memory-outcome-ab".into(),
        "run".into(),
        "--set".into(),
        set.display().to_string(),
        "--out".into(),
        out.display().to_string(),
        "--cache".into(),
        cache.display().to_string(),
        "--endpoint".into(),
        "task-complet".into(),
    ];
    let output = f.expect(&args, 2);
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--endpoint"));
    assert!(!out.exists());
    assert!(!cache.exists());
    let error = infra_eval::execute(
        "memory-outcome-ab/run",
        &json!({
            "set":set,"out":out,"cache":cache,"endpoint":"task-complet"
        }),
    )
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::InvalidInput);
    assert!(error.message.contains("--endpoint"));
    assert!(!out.exists());
    assert!(!cache.exists());
}

#[test]
fn cost_attribution_default_uses_home_transcripts_outside_a_checkout() {
    let f = Fixture::new();
    let binary = f.root.join(if cfg!(windows) {
        "standalone-engine.exe"
    } else {
        "standalone-engine"
    });
    executable::copy(env!("CARGO_BIN_EXE_infra-engine"), &binary);
    let transcripts = f.root.join("home/.claude/projects/cost-project");
    fs::create_dir_all(&transcripts).unwrap();
    fs::write(
        transcripts.join("one.jsonl"),
        serde_json::to_vec(&json!({
            "type":"assistant","isSidechain":false,
            "message":{"id":"cost-one","usage":{"input_tokens":17,"output_tokens":3}}
        }))
        .unwrap(),
    )
    .unwrap();
    let output = f.run_with(&binary, &strings(&["eval", "cost-attribution", "--json"]));
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["calls"], 1);
    assert_eq!(value["tiers"]["input"], 17);
    assert_eq!(value["tiers"]["output"], 3);
    assert_eq!(value["total_bte"].as_f64(), Some(32.0));
}

#[test]
fn foreign_subcommand_flags_are_rejected_before_collecting_or_writing() {
    let f = Fixture::new();
    let out = f.root.join("must not write sources.json");
    let args = vec![
        "eval".into(),
        "memory-outcome-ab".into(),
        "recall-sources".into(),
        "--projects".into(),
        "example".into(),
        "--before".into(),
        "2026-01-01".into(),
        "--out".into(),
        out.display().to_string(),
        "--compare".into(),
        "off,capsule".into(),
    ];
    let output = f.expect(&args, 2);
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--compare"));
    assert!(!out.exists());
    assert_eq!(fs::read_dir(f.root.join("tmp")).unwrap().count(), 0);
}

#[test]
fn scoped_defaults_preserve_absent_nullable_options_and_report_positionals() {
    let run = arguments::parse(
        &strings(&["--set", "set.json", "--out", "out.json"]),
        "eval/memory-outcome-ab/run",
    )
    .unwrap()
    .value;
    assert_eq!(run["endpoint"], "first-touch");
    assert_eq!(run["repeat"], 1);
    assert_eq!(run["cap"], 40);
    assert_eq!(run["timeout"].as_f64(), Some(900.0));
    assert!(run.get("ids").is_none());
    assert!(run.get("events").is_none());
    let bench = arguments::parse(&[], "eval/claude-code-bench")
        .unwrap()
        .value;
    assert!(bench.get("queued").is_none());
    assert!(bench.get("worktree").is_none());
    assert!(bench.get("attached").is_none());
    assert_eq!(bench["mode"], "default");
    let report = arguments::parse(
        &strings(&["result.json", "--json"]),
        "eval/memory-outcome-ab/report",
    )
    .unwrap();
    assert_eq!(report.value["result"], "result.json");
    assert!(arguments::parse(&[], "eval/memory-outcome-ab/report").is_err());
    assert!(
        arguments::parse(
            &strings(&["first.json", "second.json"]),
            "eval/memory-outcome-ab/report"
        )
        .is_err()
    );
}
