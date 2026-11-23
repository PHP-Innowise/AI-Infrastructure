//! The executor inside the gate sandbox: a replay with its programs and their
//! guardian runs in a minimal root, without the network, on a copy of the
//! task. This is how Gates 2 and 3 run a capsule.
#![cfg(target_os = "linux")]
#[path = "support/scratch.rs"]
mod scratch;
use infra_host::sandbox::{self, Spec};
use infra_skillcache::{SkillCapsule, package};
use serde_json::{Value, json};
use std::{
    path::Path,
    process::{Command, Stdio},
};

const FIXTURE: &str = env!("CARGO_BIN_EXE_infra-skillcache-program-fixture");

fn bubblewrap_usable() -> bool {
    Command::new("bwrap")
        .args([
            "--unshare-pid",
            "--unshare-net",
            "--ro-bind",
            "/",
            "/",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "true",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn capsule(runtime: &str) -> Value {
    let program = |id: &str, inputs: Value, outputs: Value, read_only: bool| {
        json!({
            "id": id,
            "entry": [runtime, format!("programs/{id}/program.json")],
            "inputs": inputs,
            "outputs": outputs,
            "check": [runtime, format!("programs/{id}/program.json"), "--check"],
            "timeout_seconds": 30,
            "read_only": read_only
        })
    };
    json!({
        "schema_version": 1,
        "form": "hybrid",
        "mu": {"name": "sandboxed-replay", "description": "A two-stage plan for the gate sandbox"},
        "source": {"skills": [{"name": "pdf", "path": ".claude/skills/pdf/SKILL.md", "tree_sha256": "0".repeat(64)}]},
        "capabilities": {"network": "none", "runtimes": [runtime]},
        "bindings": [{"name": "input_dir", "type": "path", "origin": "request"}],
        "programs": [
            program("extract", json!(["input_dir"]), json!(["rows_file"]), false),
            program("verify", json!(["rows_file"]), json!([]), true)
        ],
        "steps": [{"kind": "program", "program": "extract"}],
        "verification": [{"kind": "program", "program": "verify"}]
    })
}

#[test]
fn a_replay_runs_inside_the_gate_sandbox() {
    if !bubblewrap_usable() {
        eprintln!("skipped: bubblewrap cannot create a sandbox on this host");
        return;
    }
    let root = scratch::scratch();
    let base = infra_core::fs::canonicalize(root.path()).unwrap();
    let fixture = Path::new(FIXTURE);
    let runtime = fixture.file_stem().unwrap().to_str().unwrap();
    let mut files = package::Files::new();
    files.insert(
        "programs/extract/program.json",
        serde_json::to_vec(&json!({
            "result": {"rows_file": "out/rows.json"},
            "write": {"out/rows.json": "[]"},
            "env_names_to": "env-names.txt",
            "touch_package": "assets/columns.json"
        }))
        .unwrap(),
    );
    files.insert(
        "programs/verify/program.json",
        serde_json::to_vec(&json!({"result": {"rows": 0}, "require_file": "out/rows.json"}))
            .unwrap(),
    );
    files.insert("assets/columns.json", b"[]".to_vec());
    let parsed = SkillCapsule::parse(&serde_json::to_vec(&capsule(runtime)).unwrap()).unwrap();
    let package = base.join("capsules/c1/1/warm");
    let digest = package::write_warm(&package, &parsed, None, &files).unwrap();
    let qualification = base.join("qualification/c1/1");
    std::fs::create_dir_all(&qualification).unwrap();
    let examples = qualification.join("examples.json");
    std::fs::write(
        &examples,
        serde_json::to_vec(&json!({
            "bindings": {"input_dir": "invoices/"},
            "steps": {"0": {"result": {"rows_file": "observed/rows.json"}}}
        }))
        .unwrap(),
    )
    .unwrap();
    let workspace = base.join("replay/r1/workspace");
    let state = base.join("replay-state/r1");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&state).unwrap();
    let state_file = state.join("plan-state.json");
    let spec = Spec {
        workspace: workspace.clone(),
        target: workspace.clone(),
        protected: vec![],
        readable: vec![
            package.clone(),
            qualification.clone(),
            fixture.parent().unwrap().to_path_buf(),
        ],
        accounts: vec![state.clone()],
        runtime: vec![],
        codex: None,
        evidence: None,
        network: false,
    };
    let argv = sandbox::gate(
        &spec,
        &[
            FIXTURE.to_string(),
            "skillcache-run".into(),
            "run".into(),
            "--replay".into(),
            examples.display().to_string(),
        ],
    )
    .unwrap();
    let output = Command::new(&argv[0])
        .args(&argv[1..])
        .env_clear()
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", fixture.parent().unwrap().display()),
        )
        .env("SKILLCACHE_PACKAGE", &package)
        .env("SKILLCACHE_WORKSPACE", &workspace)
        .env("SKILLCACHE_STATE_FILE", &state_file)
        .env("SKILLCACHE_GENERATION", "gate3")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "no replay report: {:?} / {:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert_eq!(report["status"], "replay", "{report}");
    assert_eq!(report["passed"], true, "{report}");
    assert_eq!(report["completed"], json!([0, 1]));
    assert_eq!(output.status.code(), Some(0));
    // The programs worked on the copy, the state was written outside it, and
    // the package came out exactly as it went in.
    assert!(workspace.join("out/rows.json").is_file());
    assert!(state_file.is_file());
    assert_eq!(package::digest(&package).unwrap(), digest);
    let state: Value = serde_json::from_slice(&std::fs::read(&state_file).unwrap()).unwrap();
    assert_eq!(state["results"]["extract"]["package_written"], false);
    assert_eq!(state["bindings"]["rows_file"], "out/rows.json");
    package::remove(&package).unwrap();
}

#[test]
fn read_only_live_run_keeps_workspace_read_only_and_state_writable_with_scratch_home() {
    if !bubblewrap_usable() {
        eprintln!("skipped: bubblewrap cannot create a sandbox on this host");
        return;
    }
    let root = scratch::scratch();
    let base = infra_core::fs::canonicalize(root.path()).unwrap();
    let fixture = Path::new(FIXTURE);
    let runtime = fixture.file_stem().unwrap().to_str().unwrap();

    let mut files = package::Files::new();
    files.insert(
        "programs/probe/program.json",
        serde_json::to_vec(&json!({
            "result": {"ok": true},
            "write": {"attempted-write.txt": "should-fail"},
            "env_names_to": "env-names.txt"
        }))
        .unwrap(),
    );
    let ro_cap = SkillCapsule::parse(
        &serde_json::to_vec(&json!({
            "schema_version": 1,
            "form": "hybrid",
            "kind": "tool_chain",
            "mu": {"name": "ro-probe", "description": "Read-only probe"},
            "source": {"skills": []},
            "capabilities": {"network": "none", "runtimes": [runtime]},
            "bindings": [],
            "programs": [{
                "id": "probe",
                "entry": [runtime, "programs/probe/program.json"],
                "inputs": [],
                "outputs": [],
                "check": [runtime, "programs/probe/program.json", "--check"],
                "timeout_seconds": 30,
                "read_only": true
            }],
            "steps": [{"kind": "program", "program": "probe"}],
            "verification": []
        }))
        .unwrap(),
    )
    .unwrap();
    let package_dir = base.join("capsules/c2/1/warm");
    package::write_warm(&package_dir, &ro_cap, None, &files).unwrap();

    let project_ws = base.join("project-ws");
    std::fs::create_dir_all(&project_ws).unwrap();
    std::fs::write(project_ws.join("existing.txt"), b"keep").unwrap();

    let sandbox_ws = base.join("sandbox-ctl/work");
    let scratch_home = sandbox_ws.join(".harness-tmp");
    let state_dir = base.join("run-state");
    std::fs::create_dir_all(&scratch_home).unwrap();
    std::fs::create_dir_all(&state_dir).unwrap();
    let state_file = state_dir.join("plan-state.json");

    let spec = Spec {
        workspace: sandbox_ws.clone(),
        target: sandbox_ws.clone(),
        protected: vec![],
        readable: vec![
            package_dir.clone(),
            project_ws.clone(),
            fixture.parent().unwrap().to_path_buf(),
        ],
        accounts: vec![state_dir.clone()],
        runtime: vec![],
        codex: None,
        evidence: None,
        network: false,
    };
    let argv = sandbox::gate(
        &spec,
        &[FIXTURE.to_string(), "skillcache".into(), "run".into()],
    )
    .unwrap();
    let output = Command::new(&argv[0])
        .args(&argv[1..])
        .env_clear()
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", fixture.parent().unwrap().display()),
        )
        .env("HOME", &scratch_home)
        .env("TMPDIR", &scratch_home)
        .env("SKILLCACHE_PACKAGE", &package_dir)
        .env("SKILLCACHE_WORKSPACE", &project_ws)
        .env("SKILLCACHE_STATE_FILE", &state_file)
        .env("SKILLCACHE_GENERATION", "live-ro")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let answer: Value = serde_json::from_slice(&output.stdout).unwrap();
    // Because `project_ws` was mounted read-only via `spec.readable`, neither
    // `attempted-write.txt` nor `env-names.txt` could be written into `project_ws`,
    // while `state_file` in `spec.accounts` was written.
    assert_eq!(answer["status"], "complete", "{answer}");
    assert!(!project_ws.join("attempted-write.txt").exists());
    assert!(!project_ws.join("env-names.txt").exists());
    assert!(state_file.is_file());
    package::remove(&package_dir).unwrap();
}
