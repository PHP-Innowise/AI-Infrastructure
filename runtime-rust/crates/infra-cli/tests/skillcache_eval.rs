//! The sealed SkillCache evaluation, end to end through the engine, with a
//! scripted driver: no provider is called and every number is fixed.
#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};

const ENGINE: &str = env!("CARGO_BIN_EXE_infra-engine");

/// Answers a preparation with a compile usage and an attempt with a cold or
/// a warm usage, and appends each request to `$CALLS`. It always claims that
/// its work was accepted. With `$WORK`, it leaves a workspace whose `out.txt`
/// is bad for the warm arm of task `t3`. With `$FAIL_ON`, it fails on the
/// warm rename attempt of that task.
const DRIVER: &str = r#"#!/bin/sh
request=$(cat)
printf '%s\n' "$request" >> "$CALLS"
usage() {
  printf '{"input_tokens":%s,"output_tokens":%s,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}' "$1" "$2"
}
case "$request" in
  *'"phase":"prepare"'*)
    printf '{"prepared":true,"usage":%s,"seconds":12}\n' "$(usage 10000 2000)"
    exit 0 ;;
esac
workspace=
if [ -n "${WORK:-}" ]; then
  workspace=$(mktemp -d "$WORK/attempt.XXXXXX")
  word=good
  case "$request" in *'"arm":"skillcache_executor"'*'"task":"t3"'*|*'"task":"t3"'*'"arm":"skillcache_executor"'*) word=bad ;; esac
  printf '%s\n' "$word" > "$workspace/out.txt"
fi
case "$request" in
  *'"arm":"cold"'*)
    printf '{"accepted":true,"workspace":"%s","usage":%s,"model_calls":9,"seconds":30,"note":"dropped"}\n' "$workspace" "$(usage 100000 10000)" ;;
  *)
    if [ -n "${FAIL_ON:-}" ]; then
      case "$request" in *'"probe":"rename"'*) case "$request" in *"\"task\":\"$FAIL_ON\""*) exit 3 ;; esac ;; esac
    fi
    printf '{"accepted":true,"workspace":"%s","usage":%s,"model_calls":3,"seconds":8,"program_steps":2,"failed_steps":1}\n' "$workspace" "$(usage 20000 2000)" ;;
esac
"#;

/// Judges a workspace by its `out.txt`, with the receipt of the outcome
/// evaluation, and fails if the sealed folder passed in `$1` is visible inside
/// the verifier's read isolation.
const VERIFIER: &str = r#"if [ -n "${1:-}" ] && [ -e "$1" ]; then
  printf 'sealed folder visible inside verifier\n' >&2
  exit 2
fi
correct=false
if grep -q good out.txt 2>/dev/null; then correct=true; fi
printf '{"schema":"task-outcome-verifier-v1","completed":true,"correct":%s}\n' "$correct"
"#;

struct Stand {
    _root: tempfile::TempDir,
    base: PathBuf,
}

impl Stand {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let base = infra_core::fs::canonicalize(root.path()).unwrap();
        for folder in ["sealed", "work", "verifiers"] {
            std::fs::create_dir(base.join(folder)).unwrap();
        }
        let driver = base.join("driver.sh");
        std::fs::write(&driver, DRIVER).unwrap();
        std::fs::set_permissions(&driver, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(base.join("verifiers/check.sh"), VERIFIER).unwrap();
        Self { _root: root, base }
    }
    fn plan(&self, verified: bool) -> Value {
        let mut plan = json!({
            "schema": "skillcache-eval-protocol-v1",
            "question": "Does a warm turn cost less than a cold one for the same accepted work?",
            "design": {
                "primary": "correctness",
                "quality_margin": 0.1,
                "minimum_tasks": 3,
                "power_rationale": "A regression fixture, not a powered study.",
                "selection": "Every task of the fixture.",
                "exclusion": "None.",
                "secondary_benefit": "List price per accepted reuse.",
                "stopping_rule": "fixed-horizon",
                "unknown_policy": "insufficient"
            },
            "arms": ["cold", "skillcache_executor"],
            "probes": ["exact", "rename"],
            "matrix": [{"compile_model": "compiler", "executor_model": "worker"}],
            "tasks": [
                {"id": "t1", "skill": "pdf", "observe": "Extract rows",
                 "probes": {"rename": "Extract rows of b"}},
                {"id": "t2", "skill": "pdf", "observe": "Extract totals",
                 "probes": {"rename": "Extract totals of b"}},
                {"id": "t3", "skill": "pdf", "observe": "Extract dates",
                 "probes": {"rename": "Extract dates of b"}}
            ],
            "prices": {
                "compiler": {"input": 10.0, "output": 50.0, "cache_read": 1.0, "cache_write": 12.5},
                "worker": {"input": 2.0, "output": 10.0, "cache_read": 0.2, "cache_write": 2.5}
            },
            "reuses": [1, 10],
            "bootstrap": {"rounds": 200, "seed": 7},
            "driver": {"argv": [self.base.join("driver.sh")], "timeout_seconds": 60}
        });
        if verified {
            plan["verifiers"] = json!({"root": self.base.join("verifiers"), "timeout_seconds": 30});
            for task in plan["tasks"].as_array_mut().unwrap() {
                task["verifier"] = json!(["sh", "{verifiers}/check.sh", self.base.join("sealed")]);
            }
        }
        plan
    }
    fn engine(&self, arguments: &[&str], environment: &[(&str, &Path)]) -> Output {
        let mut command = Command::new(ENGINE);
        command
            .args(["eval", "skillcache-ab"])
            .args(arguments)
            .current_dir(&self.base)
            .env("CALLS", self.base.join("calls.jsonl"))
            .env_remove("WORK")
            .env_remove("FAIL_ON");
        for (name, value) in environment {
            command.env(name, value);
        }
        command.output().unwrap()
    }
    /// Seals `plan` as `sealed/<name>.json` and returns that path.
    fn seal(&self, name: &str, plan: &Value) -> PathBuf {
        let source = self.base.join(format!("{name}-plan.json"));
        std::fs::write(&source, plan.to_string()).unwrap();
        let sealed = self.base.join(format!("sealed/{name}.json"));
        let output = self.engine(
            &[
                "seal",
                "--plan",
                source.to_str().unwrap(),
                "--out",
                sealed.to_str().unwrap(),
            ],
            &[],
        );
        assert!(output.status.success(), "{}", text(&output));
        sealed
    }
    fn calls(&self) -> Vec<Value> {
        std::fs::read_to_string(self.base.join("calls.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

fn text(output: &Output) -> String {
    format!(
        "exit {:?}\n{}\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}
fn answer(output: &Output) -> Value {
    assert!(output.status.success(), "{}", text(output));
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| panic!("{}", text(output)))
}

#[test]
fn a_sealed_plan_runs_once_through_its_driver_and_reports_a_verdict() {
    let stand = Stand::new();
    let sealed = stand.seal("first", &stand.plan(false));
    let stored: Value = serde_json::from_slice(&std::fs::read(&sealed).unwrap()).unwrap();
    let seal = stored["seal_sha256"].as_str().unwrap();
    assert_eq!(seal.len(), 64);
    // A seal is written once.
    let again = stand.engine(
        &[
            "seal",
            "--plan",
            stand.base.join("first-plan.json").to_str().unwrap(),
            "--out",
            sealed.to_str().unwrap(),
        ],
        &[],
    );
    assert!(!again.status.success());

    let result = stand.base.join("result.json");
    let run = [
        "run",
        "--protocol",
        sealed.to_str().unwrap(),
        "--out",
        result.to_str().unwrap(),
    ];
    let done = answer(&stand.engine(&run, &[]));
    // 3 tasks: one preparation of the warm arm each; 2 probes x 2 arms.
    assert_eq!(done["preparations"], 3);
    assert_eq!(done["attempts"], 12);
    assert_eq!(done["unanswered"], 0);
    let calls = stand.calls();
    assert_eq!(calls.len(), 15);
    assert!(calls.iter().all(|call| call["seal_sha256"] == seal));
    // A task is prepared before any of its attempts, and the probe's own
    // request is what an attempt is asked.
    for task in ["t1", "t2", "t3"] {
        let mine = calls
            .iter()
            .filter(|call| call["task"] == task)
            .collect::<Vec<_>>();
        assert_eq!(mine[0]["phase"], "prepare", "{mine:?}");
        assert_eq!(mine[0]["arm"], "skillcache_executor");
        assert_eq!(mine[0]["compile_model"], "compiler");
        assert!(mine[1..].iter().all(|call| call["phase"] == "attempt"));
    }
    let renamed = calls
        .iter()
        .find(|call| call["task"] == "t2" && call["probe"] == "rename")
        .unwrap();
    assert_eq!(renamed["request"], "Extract totals of b");
    let cold = calls
        .iter()
        .find(|call| call["phase"] == "attempt" && call["arm"] == "cold")
        .unwrap();
    assert!(
        cold["compile_model"].is_null(),
        "the cold arm compiles nothing"
    );
    // The result keeps what an attempt is, and nothing else the driver said.
    let stored: Value = serde_json::from_slice(&std::fs::read(&result).unwrap()).unwrap();
    assert_eq!(stored["schema"], "skillcache-eval-result-v1");
    assert!(stored["isolation"].is_null());
    assert!(
        stored["attempts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|attempt| attempt.get("note").is_none() && attempt["judged_by"] == "driver")
    );
    // A sealed plan is run once.
    let refused = stand.engine(&run, &[]);
    assert!(!refused.status.success());
    assert!(text(&refused).contains("run once"), "{}", text(&refused));
    assert_eq!(stand.calls().len(), 15, "nothing was asked again");

    let report = answer(&stand.engine(
        &[
            "report",
            "--protocol",
            sealed.to_str().unwrap(),
            "--result",
            result.to_str().unwrap(),
        ],
        &[],
    ));
    assert_eq!(report["schema"], "skillcache-eval-report-v1");
    assert_eq!(report["seal_sha256"], seal);
    let arms = &report["cells"][0]["arms"];
    assert_eq!(arms["cold"]["list_price_usd"], 0.3);
    assert_eq!(arms["cold"]["model_calls"], 9.0);
    assert!(arms["cold"]["driver_seconds"].as_f64().is_some());
    let warm = &arms["skillcache_executor"];
    assert_eq!(warm["list_price_usd"], 0.06);
    assert_eq!(warm["model_calls"], 3.0);
    assert_eq!(warm["deopt_rate"], 0.5);
    assert_eq!(warm["preparation"]["compile_usd"], 0.2);
    assert_eq!(warm["against_cold"]["saving_per_reuse_usd"], 0.24);
    assert_eq!(warm["against_cold"]["breakeven_reuses"], 1);
    assert_eq!(warm["against_cold"]["net_saving_usd"]["10"], 2.2);
    assert_eq!(warm["against_cold"]["quality"], "non-inferior");
    assert_eq!(warm["against_cold"]["verdict"], "saves");

    let viewer_out = stand.base.join("benchmark.json");
    let exported = answer(&stand.engine(
        &[
            "viewer",
            "--protocol",
            sealed.to_str().unwrap(),
            "--result",
            result.to_str().unwrap(),
            "--out",
            viewer_out.to_str().unwrap(),
        ],
        &[],
    ));
    assert_eq!(exported["arm"], "skillcache_executor");
    let benchmark: Value = serde_json::from_slice(&std::fs::read(&viewer_out).unwrap()).unwrap();
    assert_eq!(
        benchmark["configuration_order"],
        json!(["cold", "skillcache_executor"])
    );
    assert_eq!(benchmark["run_summary"]["cold"]["list_price_usd"], 0.3);
    assert_eq!(
        benchmark["run_summary"]["skillcache_executor"]["deopt_rate"],
        0.5
    );
    assert_eq!(benchmark["runs"].as_array().unwrap().len(), 12);

    // A result is reported under its own sealed protocol only, and a protocol
    // changed after its seal reports nothing.
    let mut other = stand.plan(false);
    other["prices"]["worker"]["input"] = json!(0.1);
    let other = stand.seal("cheaper", &other);
    let mismatch = stand.engine(
        &[
            "report",
            "--protocol",
            other.to_str().unwrap(),
            "--result",
            result.to_str().unwrap(),
        ],
        &[],
    );
    assert!(!mismatch.status.success());
    let mut changed: Value = serde_json::from_slice(&std::fs::read(&sealed).unwrap()).unwrap();
    changed["prices"]["worker"]["input"] = json!(0.1);
    std::fs::remove_file(&sealed).unwrap();
    std::fs::write(&sealed, changed.to_string()).unwrap();
    let tampered = stand.engine(
        &[
            "report",
            "--protocol",
            sealed.to_str().unwrap(),
            "--result",
            result.to_str().unwrap(),
        ],
        &[],
    );
    assert!(!tampered.status.success());
    assert!(
        text(&tampered).contains("changed after it was sealed"),
        "{}",
        text(&tampered)
    );
}

#[test]
fn a_driver_that_fails_leaves_an_unknown_and_the_answer_is_insufficient() {
    let stand = Stand::new();
    let sealed = stand.seal("failing", &stand.plan(false));
    let result = stand.base.join("result.json");
    let done = answer(&stand.engine(
        &[
            "run",
            "--protocol",
            sealed.to_str().unwrap(),
            "--out",
            result.to_str().unwrap(),
        ],
        &[("FAIL_ON", Path::new("t2"))],
    ));
    assert_eq!(done["attempts"], 12);
    assert_eq!(done["unanswered"], 1);
    let stored: Value = serde_json::from_slice(&std::fs::read(&result).unwrap()).unwrap();
    let failed = stored["attempts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|attempt| attempt["status"] != "finished")
        .collect::<Vec<_>>();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0]["status"], "driver-failed");
    assert_eq!(failed[0]["task"], "t2");
    assert!(failed[0]["accepted"].is_null());
    assert!(failed[0]["usage"]["input_tokens"].is_null());
    let report = answer(&stand.engine(
        &[
            "report",
            "--protocol",
            sealed.to_str().unwrap(),
            "--result",
            result.to_str().unwrap(),
        ],
        &[],
    ));
    let warm = &report["cells"][0]["arms"]["skillcache_executor"];
    assert_eq!(warm["planned"], 6);
    assert_eq!(warm["answered"], 5);
    assert_eq!(warm["judged"], 5);
    assert!(warm["acceptance_rate"].is_null());
    assert_eq!(warm["against_cold"]["quality"], "insufficient");
    assert_eq!(warm["against_cold"]["verdict"], "insufficient");
    // What was measured is still shown.
    assert_eq!(warm["against_cold"]["pairs_priced"], 5);
    assert_eq!(warm["against_cold"]["saving_per_reuse_usd"], 0.24);
}

/// Whether this host lets an unprivileged process make the namespaces the
/// verifier isolation needs.
#[cfg(target_os = "linux")]
fn isolation_works() -> bool {
    Command::new("bwrap")
        .args(["--ro-bind", "/", "/", "--unshare-net", "/usr/bin/true"])
        .output()
        .is_ok_and(|output| output.status.success())
}

#[cfg(target_os = "linux")]
#[test]
fn with_verifiers_an_isolated_verifier_judges_and_the_driver_is_not_asked() {
    if !isolation_works() {
        assert!(
            std::env::var_os("INFRA_REQUIRE_BWRAP").is_none(),
            "bubblewrap namespaces are required under INFRA_REQUIRE_BWRAP"
        );
        eprintln!("skipped: this host does not allow bubblewrap namespaces");
        return;
    }
    let stand = Stand::new();
    let sealed = stand.seal("verified", &stand.plan(true));
    let stored: Value = serde_json::from_slice(&std::fs::read(&sealed).unwrap()).unwrap();
    assert_eq!(stored["verifiers"]["sha256"].as_str().unwrap().len(), 64);
    let work = stand.base.join("work");
    let result = stand.base.join("result.json");
    let done = answer(&stand.engine(
        &[
            "run",
            "--protocol",
            sealed.to_str().unwrap(),
            "--out",
            result.to_str().unwrap(),
        ],
        &[("WORK", &work)],
    ));
    assert_eq!(done["unanswered"], 0);
    let stored: Value = serde_json::from_slice(&std::fs::read(&result).unwrap()).unwrap();
    assert_eq!(stored["isolation"], "linux-bwrap-allowlist-v1");
    let attempts = stored["attempts"].as_array().unwrap();
    assert!(
        attempts
            .iter()
            .all(|attempt| attempt["judged_by"] == "verifier")
    );
    // The driver said "accepted" twelve times; the verifier found the two
    // warm attempts of `t3` wrong.
    let rejected = attempts
        .iter()
        .filter(|attempt| attempt["accepted"] == false)
        .collect::<Vec<_>>();
    assert_eq!(rejected.len(), 2, "{attempts:?}");
    assert!(
        rejected
            .iter()
            .all(|attempt| attempt["task"] == "t3" && attempt["arm"] == "skillcache_executor")
    );
    assert_eq!(
        attempts
            .iter()
            .filter(|attempt| attempt["accepted"] == true)
            .count(),
        10
    );
    let report = answer(&stand.engine(
        &[
            "report",
            "--protocol",
            sealed.to_str().unwrap(),
            "--result",
            result.to_str().unwrap(),
        ],
        &[],
    ));
    assert_eq!(report["judged_by"], "verifier");
    let against = &report["cells"][0]["arms"]["skillcache_executor"]["against_cold"];
    // One task of three lost its work: that is not within a margin of 0.1.
    assert_eq!(against["pairs_accepted"], 4);
    assert_eq!(against["tasks_priced"], 2);
    assert_ne!(against["quality"], "non-inferior");
    assert_ne!(against["verdict"], "saves");

    // Verifiers changed after the seal judge nothing.
    let changed = stand.seal("changed", &stand.plan(true));
    std::fs::write(
        stand.base.join("verifiers/check.sh"),
        "printf '{\"schema\":\"task-outcome-verifier-v1\",\"completed\":true,\"correct\":true}\\n'\n",
    )
    .unwrap();
    let before = stand.calls().len();
    let refused = stand.engine(
        &[
            "run",
            "--protocol",
            changed.to_str().unwrap(),
            "--out",
            stand.base.join("result-2.json").to_str().unwrap(),
        ],
        &[("WORK", &work)],
    );
    assert!(!refused.status.success());
    assert!(
        text(&refused).contains("verifiers changed"),
        "{}",
        text(&refused)
    );
    assert_eq!(stand.calls().len(), before, "nothing was run");
    assert!(!stand.base.join("result-2.json").exists());
}

/// One verifier judges every probe of its task and sees nothing but the
/// workspace, so the probe is given to it. Here every workspace holds the
/// same file, as after a capsule that replayed the observed request, and
/// only the observed request's own probe may accept it.
#[cfg(target_os = "linux")]
#[test]
fn a_verifier_is_told_the_probe_it_judges() {
    if !isolation_works() {
        assert!(
            std::env::var_os("INFRA_REQUIRE_BWRAP").is_none(),
            "bubblewrap namespaces are required under INFRA_REQUIRE_BWRAP"
        );
        eprintln!("skipped: this host does not allow bubblewrap namespaces");
        return;
    }
    let stand = Stand::new();
    std::fs::write(
        stand.base.join("verifiers/probe.sh"),
        "correct=false\nif [ \"$1\" = exact ] && grep -q . out.txt 2>/dev/null; then correct=true; fi\nprintf '{\"schema\":\"task-outcome-verifier-v1\",\"completed\":true,\"correct\":%s}\\n' \"$correct\"\n",
    )
    .unwrap();
    let mut plan = stand.plan(true);
    for task in plan["tasks"].as_array_mut().unwrap() {
        task["verifier"] = json!(["sh", "{verifiers}/probe.sh", "{probe}"]);
    }
    let sealed = stand.seal("probes", &plan);
    let result = stand.base.join("result.json");
    let done = answer(&stand.engine(
        &[
            "run",
            "--protocol",
            sealed.to_str().unwrap(),
            "--out",
            result.to_str().unwrap(),
        ],
        &[("WORK", &stand.base.join("work"))],
    ));
    assert_eq!(done["unanswered"], 0);
    let stored: Value = serde_json::from_slice(&std::fs::read(&result).unwrap()).unwrap();
    let attempts = stored["attempts"].as_array().unwrap();
    assert_eq!(attempts.len(), 12);
    for attempt in attempts {
        assert_eq!(
            attempt["accepted"],
            attempt["probe"] == "exact",
            "{attempt}"
        );
    }
}

#[test]
fn driver_inputs_tamper_reused_workspace_and_dense_positions_are_enforced() {
    let stand = Stand::new();
    let inputs = stand.base.join("inputs");
    std::fs::create_dir(&inputs).unwrap();
    std::fs::write(inputs.join("config.json"), "{\"v\":1}").unwrap();
    let mut plan = stand.plan(false);
    plan["driver"]["inputs"] = json!({"root": inputs});
    plan["matrix"] = json!([
        {"compile_model": "compiler", "executor_model": "worker"},
        {"compile_model": "worker", "executor_model": "worker"}
    ]);
    let sealed = stand.seal("matrix", &plan);
    let result = stand.base.join("matrix-result.json");
    answer(&stand.engine(
        &[
            "run",
            "--protocol",
            sealed.to_str().unwrap(),
            "--out",
            result.to_str().unwrap(),
        ],
        &[],
    ));
    let stored: Value = serde_json::from_slice(&std::fs::read(&result).unwrap()).unwrap();
    // Second matrix cell skips compile-free `cold`, so `skillcache_executor` gets dense position 0.
    let second_cell_positions = stored["attempts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["compile_model"] == "worker")
        .map(|a| a["position"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert!(
        second_cell_positions.iter().all(|pos| *pos == 0),
        "{second_cell_positions:?}"
    );
    // Journal is removed when result finishes cleanly.
    assert!(!stand.base.join("matrix-result.json.journal.jsonl").exists());

    // Tampering with driver.inputs stops run() before invoking the driver.
    std::fs::write(inputs.join("config.json"), "{\"v\":2}").unwrap();
    let before = stand.calls().len();
    let refused = stand.engine(
        &[
            "run",
            "--protocol",
            sealed.to_str().unwrap(),
            "--out",
            stand.base.join("refused.json").to_str().unwrap(),
        ],
        &[],
    );
    assert!(!refused.status.success());
    assert!(text(&refused).contains("driver inputs changed"));
    assert_eq!(stand.calls().len(), before);

    // Reusing one workspace across attempts invalidates the duplicate attempt.
    let shared_ws = stand.base.join("shared-ws");
    std::fs::create_dir(&shared_ws).unwrap();
    let reuse_driver = stand.base.join("reuse-driver.sh");
    std::fs::write(
        &reuse_driver,
        format!(
            "#!/bin/sh\nreq=$(cat)\ncase \"$req\" in *'\"phase\":\"prepare\"'*) printf '{{\"prepared\":true,\"usage\":{{\"input_tokens\":10,\"output_tokens\":5,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}}\\n' ;; *) printf '{{\"accepted\":true,\"workspace\":\"{}\",\"usage\":{{\"input_tokens\":10,\"output_tokens\":5,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}}\\n' ;; esac\n",
            shared_ws.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&reuse_driver, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut reuse_plan = stand.plan(false);
    reuse_plan["driver"]["argv"] = json!([reuse_driver]);
    let sealed_reuse = stand.seal("reuse", &reuse_plan);
    let reuse_result = stand.base.join("reuse-result.json");
    answer(&stand.engine(
        &[
            "run",
            "--protocol",
            sealed_reuse.to_str().unwrap(),
            "--out",
            reuse_result.to_str().unwrap(),
        ],
        &[],
    ));
    let reuse_stored: Value =
        serde_json::from_slice(&std::fs::read(&reuse_result).unwrap()).unwrap();
    let reused_count = reuse_stored["attempts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["status"] == "workspace-reused" && a["accepted"].is_null())
        .count();
    assert_eq!(reused_count, 11);
}
