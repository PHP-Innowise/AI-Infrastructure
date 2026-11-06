//! The Linux AI discovery preflight runs for real: bubblewrap, the native guardian
//! and the in-sandbox probe. A host without usable bubblewrap skips it, as Harness
//! itself refuses AI discovery there.
#![cfg(target_os = "linux")]
use std::{
    path::PathBuf,
    process::{Command, Stdio},
};

fn bubblewrap_usable() -> bool {
    Command::new("bwrap")
        .args([
            "--unshare-pid",
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

struct Root(PathBuf);
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn discovery_probe_attests_the_boundary_inside_bubblewrap() {
    if !bubblewrap_usable() {
        eprintln!("skipped: bubblewrap cannot create a sandbox on this host");
        return;
    }
    let root = Root(
        std::env::temp_dir().join(format!("infra-discovery-boundary-{}", uuid::Uuid::new_v4())),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_infra-host-sandbox-fixture"))
        .arg("discovery-boundary")
        .arg(&root.0)
        .stdin(Stdio::null())
        .output()
        .expect("run the discovery boundary fixture");
    assert!(
        output.status.success(),
        "discovery boundary fixture failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("\"workspace_writable\":true"),
        "probe receipt missing: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}
