//! The gate profile: a minimal root for a process that is not an agent, with
//! the network off unless the caller asks for it.
use infra_host::sandbox::{self, Spec};
use std::path::{Path, PathBuf};

struct Layout {
    _root: tempfile::TempDir,
    workspace: PathBuf,
    package: PathBuf,
    state: PathBuf,
}

fn layout() -> Layout {
    let root = tempfile::tempdir().unwrap();
    let base = infra_core::fs::canonicalize(root.path()).unwrap();
    let workspace = base.join("control/workspace");
    let package = base.join("package");
    let state = base.join("state");
    for folder in [&workspace, &package, &state] {
        std::fs::create_dir_all(folder).unwrap();
    }
    std::fs::write(package.join("capsule.json"), b"{}").unwrap();
    Layout {
        _root: root,
        workspace,
        package,
        state,
    }
}

fn spec(layout: &Layout, network: bool, codex: Option<PathBuf>) -> Spec {
    Spec {
        workspace: layout.workspace.clone(),
        target: layout.workspace.clone(),
        protected: vec![],
        readable: vec![layout.package.clone()],
        accounts: vec![layout.state.clone()],
        runtime: vec![],
        codex,
        evidence: None,
        network,
    }
}

#[cfg(target_os = "linux")]
fn pairs<'a>(argv: &'a [String], flag: &str) -> Vec<(&'a str, &'a str)> {
    argv.windows(3)
        .filter(|window| window[0] == flag)
        .map(|window| (window[1].as_str(), window[2].as_str()))
        .collect()
}

#[cfg(target_os = "linux")]
#[test]
fn sandbox_gate_unshare_net_argv() {
    if infra_host::provider::find_executable("bwrap").is_none() {
        eprintln!("skipped: bubblewrap is not installed on this host");
        return;
    }
    let layout = layout();
    let leaf = vec!["/bin/true".to_string()];
    let argv = sandbox::gate(&spec(&layout, false, None), &leaf).unwrap();
    assert!(
        !layout
            .workspace
            .parent()
            .unwrap()
            .join("native-helpers")
            .exists(),
        "Linux Gate profile should not copy trusted_helper when unused"
    );
    assert!(argv.contains(&"--unshare-net".to_string()), "{argv:?}");
    assert!(argv.contains(&"--unshare-pid".to_string()));
    // A minimal root: the host's `/` is never bound.
    assert!(
        !pairs(&argv, "--ro-bind").contains(&("/", "/")),
        "the gate must not show the whole filesystem: {argv:?}"
    );
    let text = |path: &Path| path.to_str().unwrap().to_string();
    let package = text(&layout.package);
    let workspace = text(&layout.workspace);
    let state = text(&layout.state);
    assert!(pairs(&argv, "--ro-bind").contains(&(package.as_str(), package.as_str())));
    assert!(pairs(&argv, "--bind").contains(&(workspace.as_str(), workspace.as_str())));
    assert!(pairs(&argv, "--bind").contains(&(state.as_str(), state.as_str())));
    assert!(!pairs(&argv, "--bind").contains(&(package.as_str(), package.as_str())));
    let end = argv.iter().position(|argument| argument == "--").unwrap();
    assert_eq!(&argv[end + 1..], &leaf[..]);
    assert_eq!(
        argv[end - 1],
        workspace,
        "the command starts in the workspace"
    );
    // The network stays for a caller that declares the need.
    let argv = sandbox::gate(&spec(&layout, true, None), &leaf).unwrap();
    assert!(!argv.contains(&"--unshare-net".to_string()), "{argv:?}");
    // Agents keep their network: the existing profiles do not unshare it.
    let agent = sandbox::creator(&spec(&layout, true, None), &leaf).unwrap();
    assert!(!agent.contains(&"--unshare-net".to_string()));
    assert!(pairs(&agent, "--ro-bind").contains(&("/", "/")));
}

#[cfg(target_os = "linux")]
#[test]
fn sandbox_gate_assets_read_only() {
    use std::process::{Command, Stdio};
    let usable = Command::new("bwrap")
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
        .is_ok_and(|status| status.success());
    if !usable {
        eprintln!("skipped: bubblewrap cannot create a sandbox on this host");
        return;
    }
    let layout = layout();
    // Something of the host that the minimal root must not show.
    // A read-only home leaves that one check out; the others still run.
    let outside = std::env::var_os("HOME")
        .and_then(|home| tempfile::tempdir_in(home).ok())
        .unwrap_or_else(|| tempfile::tempdir().unwrap());
    std::fs::write(outside.path().join("private.txt"), b"host file").unwrap();
    let script = format!(
        r#"set -u
if echo x > "{package}/capsule.json" 2>/dev/null; then echo package-writable; fi
if echo x > "{package}/new-file" 2>/dev/null; then echo package-writable; fi
cat "{package}/capsule.json" >/dev/null || echo package-unreadable
echo x > "{workspace}/out.txt" || echo workspace-unwritable
echo x > "{state}/plan-state.json" || echo state-unwritable
if cat "{outside}/private.txt" 2>/dev/null; then echo host-readable; fi
echo "interfaces=$(grep -c : /proc/net/dev)"
echo done"#,
        package = layout.package.display(),
        workspace = layout.workspace.display(),
        state = layout.state.display(),
        outside = outside.path().display(),
    );
    let run = |network: bool| {
        let argv = sandbox::gate(
            &spec(&layout, network, None),
            &["/bin/sh".to_string(), "-c".to_string(), script.clone()],
        )
        .unwrap();
        let output = Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        (
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };
    // Without the network the sandbox has its own namespace: loopback only.
    let (stdout, stderr) = run(false);
    assert_eq!(
        stdout, "interfaces=1\ndone",
        "the gate sandbox leaked or over-restricted\n{stderr}"
    );
    assert_eq!(
        std::fs::read(layout.package.join("capsule.json")).unwrap(),
        b"{}"
    );
    assert!(layout.workspace.join("out.txt").is_file());
    assert!(layout.state.join("plan-state.json").is_file());
    // With it, the host's interfaces are there; the files stay as closed.
    let host = std::fs::read_to_string("/proc/net/dev")
        .unwrap()
        .lines()
        .filter(|line| line.contains(':'))
        .count();
    let (stdout, stderr) = run(true);
    assert_eq!(
        stdout,
        format!("interfaces={host}\ndone"),
        "a gate that declares the network shares the host's\n{stderr}"
    );
}

#[cfg(windows)]
#[test]
fn sandbox_gate_disables_the_network_in_the_windows_profile() {
    let layout = layout();
    let codex = layout.state.join("codex.exe");
    std::fs::write(&codex, b"").unwrap();
    let leaf = vec!["leaf.exe".to_string()];
    let permissions = |network: bool| {
        let argv = sandbox::gate(&spec(&layout, network, Some(codex.clone())), &leaf).unwrap();
        argv.iter()
            .find(|argument| argument.starts_with("permissions="))
            .cloned()
            .unwrap()
    };
    let closed = permissions(false);
    assert!(
        closed.contains(r#""network"={"enabled"=false}"#),
        "{closed}"
    );
    assert!(closed.contains(r#"":minimal"="read""#), "{closed}");
    // The workspace is the gate's own folder: no entry may deny it.
    assert!(!closed.contains(r#"="deny""#), "{closed}");
    let open = permissions(true);
    assert!(open.contains(r#""network"={"enabled"=true}"#), "{open}");
}
