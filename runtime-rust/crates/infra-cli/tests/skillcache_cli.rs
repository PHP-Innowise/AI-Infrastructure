//! The engine under the names a warm SkillCache session stages: the plan
//! executor and the program tool, and the guardian of the programs they start.
use infra_skillcache::{SkillCapsule, package};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

const ENGINE: &str = env!("CARGO_BIN_EXE_infra-engine");

/// A program in a scripting language every supported host ships.
#[cfg(unix)]
fn program() -> (&'static str, &'static str, &'static str, Vec<&'static str>) {
    (
        "sh",
        "programs/echo/main.sh",
        "if [ \"${1:-}\" = \"--check\" ]; then\n  printf '{\"status\":\"complete\",\"result\":{}}\\n'\n  exit 0\nfi\nprintf '{\"status\":\"complete\",\"result\":{\"greeting\":\"hello\",\"input\":%s}}\\n' \"$SKILLCACHE_INPUT\"\n",
        vec!["sh", "programs/echo/main.sh"],
    )
}
#[cfg(windows)]
fn program() -> (&'static str, &'static str, &'static str, Vec<&'static str>) {
    (
        "powershell",
        "programs/echo/main.ps1",
        // `powershell -File` reads an argument that starts with a dash as a
        // parameter's name, so `--check` arrives in `$args`, not in `$Mode`.
        // Both are looked at, whatever number of dashes the host kept.
        "param([string]$Mode)\r\nif (\"$Mode $args\" -match '-check') {\r\n  Write-Output '{\"status\":\"complete\",\"result\":{}}'\r\n  exit 0\r\n}\r\nWrite-Output ('{\"status\":\"complete\",\"result\":{\"greeting\":\"hello\",\"input\":' + $env:SKILLCACHE_INPUT + '}}')\r\n",
        vec![
            "powershell",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            "programs/echo/main.ps1",
        ],
    )
}

struct Stand {
    _root: tempfile::TempDir,
    bin: PathBuf,
    package: PathBuf,
    workspace: PathBuf,
    state_file: PathBuf,
}

/// The stand's folder holds a sealed package, whose folders are read-only:
/// it is given back to its owner before the temporary folder removes itself.
impl Drop for Stand {
    fn drop(&mut self) {
        package::release(self._root.path());
    }
}

/// Stages the engine under a tool's name. The shared engine itself is never
/// linked: a second name would change its link count and change time, which
/// tests running beside this one check as the engine's identity. The stand
/// takes one copy of its own, and a tool's name is a link to that copy, as the
/// Harness stages them.
fn staged(bin: &Path, name: &str) -> PathBuf {
    let suffix = std::env::consts::EXE_SUFFIX;
    let own = bin.join(format!("engine-copy{suffix}"));
    if !own.is_file() {
        std::fs::copy(ENGINE, &own).unwrap();
    }
    let path = bin.join(format!("{name}{suffix}"));
    if std::fs::hard_link(&own, &path).is_err() {
        std::fs::copy(&own, &path).unwrap();
    }
    path
}

fn stand() -> Stand {
    let root = tempfile::tempdir().unwrap();
    let base = infra_core::fs::canonicalize(root.path()).unwrap();
    let (runtime, file, source, entry) = program();
    let mut check = entry.clone();
    check.push("--check");
    let capsule = json!({
        "schema_version": 1,
        "form": "hybrid",
        "mu": {"name": "echo-greeting", "description": "Greets by name"},
        "source": {"skills": [{"name": "echo", "path": ".claude/skills/echo/SKILL.md", "tree_sha256": "0".repeat(64)}]},
        "capabilities": {"network": "none", "runtimes": [runtime]},
        "bindings": [{"name": "name", "type": "string", "origin": "request"}],
        "programs": [{
            "id": "echo", "entry": entry, "inputs": ["name"], "outputs": [],
            "check": check, "timeout_seconds": 60, "read_only": true
        }],
        "steps": [{"kind": "program", "program": "echo"}],
        "verification": []
    });
    let parsed = SkillCapsule::parse(&serde_json::to_vec(&capsule).unwrap()).unwrap();
    let mut files = package::Files::new();
    files.insert(file, source.as_bytes().to_vec());
    let package = base.join("capsule/1/warm");
    package::write_warm(&package, &parsed, None, &files).unwrap();
    let bin = base.join("bin");
    let workspace = base.join("workspace");
    let state = base.join("state");
    for folder in [&bin, &workspace, &state] {
        std::fs::create_dir_all(folder).unwrap();
    }
    Stand {
        _root: root,
        bin,
        package,
        workspace,
        state_file: state.join("plan-state.json"),
    }
}

impl Stand {
    fn command(&self, executable: &Path) -> Command {
        let mut command = Command::new(executable);
        command
            .current_dir(&self.workspace)
            .env("SKILLCACHE_PACKAGE", &self.package)
            .env("SKILLCACHE_WORKSPACE", &self.workspace)
            .env("SKILLCACHE_STATE_FILE", &self.state_file)
            .stdin(Stdio::null());
        command
    }
}

fn json_line(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "expected one JSON object, got {:?} / {:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn multicall_internal_guard_passthrough() {
    let stand = stand();
    let tool = staged(&stand.bin, "skillcache-tool");
    let run = staged(&stand.bin, "skillcache-run");
    // The tool starts the program through native capture, whose guardian is
    // this same file under its staged name with the engine's internal command.
    let output = stand
        .command(&tool)
        .args(["echo", "--input", r#"{"name":"Ada"}"#])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        json_line(&output),
        json!({"status": "complete", "result": {"greeting": "hello", "input": {"name": "Ada"}}})
    );
    let output = stand
        .command(&tool)
        .args(["echo", "--check"])
        .output()
        .unwrap();
    assert_eq!(
        json_line(&output),
        json!({"status": "complete", "result": {}})
    );
    // The executor under its staged name runs the same program from a plan.
    let output = stand
        .command(&run)
        .args(["run", r#"{"name":"Ada"}"#])
        .output()
        .unwrap();
    let answer = json_line(&output);
    assert_eq!(answer["status"], "complete", "{answer}");
    assert_eq!(answer["result"]["echo"]["input"]["name"], "Ada");
    let output = stand.command(&run).arg("status").output().unwrap();
    assert_eq!(json_line(&output)["finished"], true);
    // An internal command given to a staged name is the engine's, not the
    // tool's: it is not answered with the tool's usage.
    let output = stand.command(&tool).arg("internal-guard").output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("skillcache-tool <program>"), "{stderr}");
    assert_ne!(output.status.code(), Some(0));
    let output = stand
        .command(&tool)
        .args(["internal", "nonsense"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Unknown internal command"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Any other first argument of a staged name is the tool's own.
    let output = stand.command(&tool).arg("memory").output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage: skillcache-tool"));
}

#[test]
fn the_engine_has_a_skillcache_command_group() {
    let stand = stand();
    let engine = Path::new(ENGINE);
    let output = stand
        .command(engine)
        .args(["skillcache", "--help"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let help = String::from_utf8_lossy(&output.stdout);
    for operation in ["run", "continue", "status", "tool", "gate1"] {
        assert!(help.contains(operation), "{help}");
    }
    assert!(
        String::from_utf8_lossy(&stand.command(engine).arg("--help").output().unwrap().stdout)
            .contains("skillcache")
    );
    let output = stand
        .command(engine)
        .args(["skillcache", "gate1"])
        .arg(&stand.package)
        .output()
        .unwrap();
    let report = json_line(&output);
    assert_eq!(report["gate"], "gate1");
    assert_eq!(report["passed"], true, "{report}");
    assert_eq!(output.status.code(), Some(0));
    let output = stand
        .command(engine)
        .args(["skillcache", "run", r#"{"name":"Lin"}"#])
        .output()
        .unwrap();
    assert_eq!(json_line(&output)["result"]["echo"]["input"]["name"], "Lin");
    let output = stand
        .command(engine)
        .args(["skillcache", "tool", "echo", "--input", r#"{"name":"Lin"}"#])
        .output()
        .unwrap();
    assert_eq!(json_line(&output)["result"]["greeting"], "hello");
    let output = stand
        .command(engine)
        .args(["skillcache", "bogus"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(json_line(&output)["code"], "usage");
    // Outside a warm session the commands say what is missing.
    let output = Command::new(engine)
        .args(["skillcache", "status"])
        .env_remove("SKILLCACHE_PACKAGE")
        .output()
        .unwrap();
    assert_eq!(json_line(&output)["code"], "not_configured");
    let caps = Command::new(engine)
        .args(["capabilities", "--json"])
        .output()
        .unwrap();
    let caps: Value = serde_json::from_slice(&caps.stdout).unwrap();
    assert!(
        caps["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item == "skillcache"),
        "{caps}"
    );
    assert_eq!(
        infra_cli::staged_tool(Some(std::ffi::OsStr::new("SKILLCACHE-RUN.EXE"))),
        Some("run-command")
    );
    assert_eq!(
        infra_cli::staged_tool(Some(std::ffi::OsStr::new("SkillCache-Tool.exe"))),
        Some("tool")
    );
}
