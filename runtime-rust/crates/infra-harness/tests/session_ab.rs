//! The controller is a separate native process, as are its provider and oracle.
#![cfg(target_os = "linux")]
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Evidence(Option<tempfile::TempDir>);
impl Evidence {
    fn new() -> Self {
        Self(Some(
            tempfile::Builder::new()
                .prefix("session-ab-test-")
                .tempdir()
                .unwrap(),
        ))
    }
    fn root(&self) -> &Path {
        self.0.as_ref().unwrap().path()
    }
}
impl Drop for Evidence {
    fn drop(&mut self) {
        if std::thread::panicking()
            && let Some(root) = self.0.take()
        {
            eprintln!("Retained A/B failure evidence: {}", root.keep().display());
        }
    }
}
struct Owned(Child);
impl Drop for Owned {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
fn executable() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_infra-harness-ab"))
}
fn spawn(exe: &Path, root: &Path, args: &[&str], label: &str) -> Owned {
    Owned(
        Command::new(exe)
            .env_remove("RUST_MIN_STACK")
            .args(args)
            .stdin(Stdio::null())
            .stdout(File::create(root.join(format!("{label}.stdout"))).unwrap())
            .stderr(File::create(root.join(format!("{label}.stderr"))).unwrap())
            .spawn()
            .unwrap(),
    )
}
fn finish(mut child: Owned, root: &Path, label: &str, success: bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(150);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "controller timeout: {}",
            root.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    let err = fs::read_to_string(root.join(format!("{label}.stderr"))).unwrap();
    assert_eq!(status.success(), success, "{label}: {err}");
    if !success {
        return Value::Null;
    }
    serde_json::from_slice(&fs::read(root.join(format!("{label}.stdout"))).unwrap()).unwrap()
}
fn command(root: &Path, args: &[&str], label: &str, success: bool) -> Value {
    finish(
        spawn(&executable(), root, args, label),
        root,
        label,
        success,
    )
}
fn strpath(path: &Path) -> &str {
    path.to_str().unwrap()
}
fn read(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
fn prepare(root: &Path, policy: &str) -> PathBuf {
    let sealed = root.join(format!("sealed-{policy}"));
    command(
        root,
        &["prepare", strpath(&sealed), policy],
        &format!("prepare-{policy}"),
        true,
    );
    sealed.join("plan.json")
}
fn run(root: &Path, policy: &str) -> Value {
    let plan = prepare(root, policy);
    let output = root.join(format!("run-{policy}"));
    command(
        root,
        &["run", strpath(&plan), strpath(&output)],
        &format!("run-{policy}"),
        true,
    );
    read(&output.join("report.json"))
}
#[test]
fn gold_controls_and_counterbalanced_sessions_produce_independently_verified_artifacts() {
    let evidence = Evidence::new();
    let root = evidence.root();
    let report = run(root, "gold");
    assert_eq!(report["finished"], true);
    assert_eq!(report["inputs_still_bound"], true);
    assert_eq!(report["preflight"]["passed"], true);
    assert_eq!(report["summary"]["correct_streams"], 4);
    assert_eq!(report["summary"]["unknown_streams"], 0);
    assert_eq!(report["summary"]["independent_streams"], 1);
    assert_eq!(report["summary"]["efficacy_verdict"], "not_evaluated");
    assert!(
        report["summary"]["total_usd"].is_null() && report["summary"]["latency_benefit"].is_null()
    );
    let slots = report["slots"].as_array().unwrap();
    assert_eq!(
        slots.iter().map(|s| s["arm"].clone()).collect::<Vec<_>>(),
        vec![
            json!("serial"),
            json!("parallel"),
            json!("parallel"),
            json!("serial")
        ]
    );
    assert_eq!(
        slots
            .iter()
            .map(|s| s["concurrency"].clone())
            .collect::<Vec<_>>(),
        vec![json!(1), json!(2), json!(2), json!(1)]
    );
    for slot in slots {
        let observation = &slot["observation"];
        assert_eq!(observation["inbox"]["status"], "completed");
        assert_eq!(observation["launches"].as_array().unwrap().len(), 3);
        assert_eq!(
            observation["attempts"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r["phase"] == "invoke")
                .count(),
            3
        );
        for launch in observation["launches"].as_array().unwrap() {
            assert_eq!(launch["settings"]["mode"], "edit");
            assert_eq!(launch["status"], "completed");
            assert!(launch["receipt"]["full_cost"]["native_diagnostics"]["main"]["boundary_signals"]["cleanup_confirmed"].as_u64().unwrap()>0);
        }
        for kind in ["a", "b"] {
            assert_eq!(
                observation["outcomes"][kind]["verification"]["correct"],
                true
            );
            assert!(
                observation["outcomes"][kind]["request_to_verification_seconds"]
                    .as_f64()
                    .unwrap()
                    > 0.0
            );
        }
    }
    let snapshot = fs::read(root.join("run-gold/report.json")).unwrap();
    let plan = root.join("sealed-gold/plan.json");
    let output = root.join("run-gold");
    command(
        root,
        &["run", strpath(&plan), strpath(&output)],
        "overwrite-refused",
        false,
    );
    assert_eq!(fs::read(output.join("report.json")).unwrap(), snapshot);
}
#[test]
fn provider_success_does_not_hide_wrong_missing_partial_or_early_exit_artifacts() {
    let evidence = Evidence::new();
    let root = evidence.root();
    for policy in [
        "no-op",
        "partial",
        "wrong-b",
        "forbidden",
        "exit-zero",
        "deadline",
    ] {
        let report = run(root, policy);
        assert_eq!(report["preflight"]["passed"], true, "{policy}: {report}");
        assert_eq!(report["summary"]["evidence_valid"], true);
        assert_eq!(report["summary"]["planned_streams"], 4);
        assert_eq!(
            report["summary"]["incorrect_streams"], 4,
            "{policy}: {report}"
        );
        assert_eq!(
            report["summary"]["unknown_streams"], 0,
            "{policy}: {report}"
        );
        if policy == "deadline" {
            assert!(
                report["slots"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|s| s["error_code"] == "pilot-deadline-proved")
            );
        } else {
            assert!(report["slots"].as_array().unwrap().iter().all(|s| {
                s["observation"]["launches"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|l| l["status"] == "completed")
            }));
        }
    }
}
#[test]
fn changed_or_resealed_inputs_are_refused_before_output_creation() {
    let evidence = Evidence::new();
    let root = evidence.root();
    let plan = prepare(root, "gold");
    let mut changed = read(&plan);
    changed["body"]["slots"][0]["concurrency"] = 2.into();
    fs::write(&plan, serde_json::to_vec(&changed).unwrap()).unwrap();
    let output = root.join("refused");
    command(
        root,
        &["run", strpath(&plan), strpath(&output)],
        "changed-seal",
        false,
    );
    assert!(!output.exists());
    changed["sha256"] =
        infra_harness::store::sha(&serde_json::to_vec(&changed["body"]).unwrap()).into();
    fs::write(&plan, serde_json::to_vec(&changed).unwrap()).unwrap();
    command(
        root,
        &["run", strpath(&plan), strpath(&output)],
        "unsupported-reseal",
        false,
    );
    assert!(!output.exists());
    command(
        root,
        &["prepare", strpath(plan.parent().unwrap())],
        "prepare-overwrite",
        false,
    );
}
#[test]
fn losing_the_owned_runner_mid_run_journals_invalid_evidence_instead_of_success() {
    let evidence = Evidence::new();
    let root = evidence.root();
    let copied = root.join("owned-runner");
    fs::copy(executable(), &copied).unwrap();
    fs::copy(
        env!("CARGO_BIN_EXE_infra-harness-ab-oracle"),
        root.join("infra-harness-ab-oracle"),
    )
    .unwrap();
    let sealed = root.join("sealed");
    let plan = sealed.join("plan.json");
    finish(
        spawn(&copied, root, &["prepare", strpath(&sealed)], "prepare"),
        root,
        "prepare",
        true,
    );
    let output = root.join("run");
    let mut child = spawn(
        &copied,
        root,
        &["run", strpath(&plan), strpath(&output)],
        "run",
    );
    let until = Instant::now() + Duration::from_secs(30);
    while !output.join("controls/a-baseline-oracle").exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "controller finished before injection"
        );
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(5));
    }
    fs::remove_file(&copied).unwrap();
    finish(child, root, "run", true);
    let report = read(&output.join("report.json"));
    assert_eq!(report["finished"], true);
    assert_eq!(report["inputs_still_bound"], false);
    assert_eq!(report["summary"]["evidence_valid"], false);
    assert_eq!(report["summary"]["unknown_streams"], 4);
    assert!(report["summary"]["correct_streams"].is_null());
    assert!(report["binding_error"].is_string());
}
