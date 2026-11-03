//! Native acceptance only: this target requires real Windows and an initialized
//! elevated Codex sandbox. It never calls a model or changes Codex trust records.
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
#[cfg(windows)]
use std::{
    fs::{File, OpenOptions},
    io,
};

struct FixtureRoot(PathBuf);
impl FixtureRoot {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "infra native sandbox {label} {}",
            uuid::Uuid::new_v4()
        ));
        infra_host::util::private_directory(&root).expect("create private native sandbox fixture");
        Self(root)
    }
}
fn remove_fixture(path: &Path) {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.is_dir() && !infra_core::fs::linked(&metadata) {
            if let Ok(entries) = fs::read_dir(path) {
                for entry in entries.flatten() {
                    remove_fixture(&entry.path());
                }
            }
            let _ = fs::remove_dir(path);
        } else if metadata.is_file() && !infra_core::fs::linked(&metadata) {
            #[cfg(windows)]
            #[expect(
                clippy::permissions_set_readonly_false,
                reason = "Windows fixture cleanup only clears the native ReadOnly attribute; this block is excluded on Unix."
            )]
            {
                let mut permissions = metadata.permissions();
                permissions.set_readonly(false);
                let _ = fs::set_permissions(path, permissions);
            }
            let _ = fs::remove_file(path);
        }
    }
}
impl Drop for FixtureRoot {
    fn drop(&mut self) {
        remove_fixture(&self.0);
    }
}
struct OwnedDriver(Option<Child>);
impl OwnedDriver {
    fn launch(mode: &str, root: &Path, codex: &Path) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_infra-host-sandbox-fixture"))
            .arg(mode)
            .arg(root)
            .arg(codex)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("launch native Rust fixture");
        Self(Some(child))
    }
    fn wait(&mut self) -> std::process::Output {
        let until = Instant::now() + Duration::from_secs(210);
        loop {
            if self
                .0
                .as_mut()
                .unwrap()
                .try_wait()
                .expect("poll fixture")
                .is_some()
            {
                break;
            }
            assert!(Instant::now() < until, "native fixture timed out");
            std::thread::sleep(Duration::from_millis(25));
        }
        self.0
            .take()
            .unwrap()
            .wait_with_output()
            .expect("collect native fixture")
    }
    #[cfg(windows)]
    fn terminate_owner(&mut self) {
        let mut child = self.0.take().unwrap();
        child.kill().expect("terminate only the owner process");
        child.wait().expect("reap owner");
    }
}
impl Drop for OwnedDriver {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
#[cfg(not(windows))]
fn require_codex() -> PathBuf {
    panic!("this ignored acceptance target must run on actual Windows")
}
#[cfg(windows)]
fn require_codex() -> PathBuf {
    assert_eq!(
        std::env::var("HARNESS_WINDOWS_CREATOR_SANDBOX").as_deref(),
        Ok("1"),
        "explicit HARNESS_WINDOWS_CREATOR_SANDBOX=1 is required"
    );
    let configured = std::env::var_os("HARNESS_CODEX_BIN")
        .map(PathBuf::from)
        .or_else(|| infra_host::provider::find_executable("codex"))
        .expect("native Codex must be installed; HARNESS_CODEX_BIN may select it");
    assert!(
        configured.is_file(),
        "configured Codex executable is unavailable"
    );
    configured
}
fn run_case(mode: &str) -> Value {
    let codex = require_codex();
    let root = FixtureRoot::new(mode);
    let output = OwnedDriver::launch(mode, &root.0, &codex).wait();
    assert!(
        output.status.success(),
        "native {mode} fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("bounded native fixture receipt JSON")
}
#[test]
#[ignore = "requires actual Windows with configured elevated Codex sandbox and HARNESS_WINDOWS_CREATOR_SANDBOX=1"]
fn creator_read_write_rename_descendants_and_binary_stdio() {
    let result = run_case("creator");
    assert_eq!(result["creator_boundary"], true);
    assert_eq!(result["binary_stdio"], true);
    assert_eq!(result["descendant_denied"], true);
    assert_eq!(result["originals_unchanged"], true);
    assert_eq!(result["private_temp"], true);
    assert_eq!(result["random_profile"], true);
}
#[test]
#[ignore = "requires actual Windows with configured elevated Codex sandbox and HARNESS_WINDOWS_CREATOR_SANDBOX=1"]
fn discovery_denies_every_original_and_keeps_evidence_read_only() {
    let result = run_case("discovery");
    assert_eq!(result["boundary_verified"], true);
    assert_eq!(result["scan_completed"], true);
    assert!(result["denied_paths"].as_u64().is_some_and(|n| n >= 4));
    assert_eq!(result["originals_unchanged"], true);
    assert_eq!(result["evidence_read_only"], true);
}
#[test]
#[ignore = "requires actual Windows with configured elevated Codex sandbox and HARNESS_WINDOWS_CREATOR_SANDBOX=1"]
fn explicit_everyone_file_acl_cannot_bypass_discovery_admission() {
    let result = run_case("discovery-explicit-acl");
    match result["outcome"].as_str() {
        Some("preflight-refused") => assert_eq!(result["scan_started"], false),
        Some("scan-denied-original") => assert_eq!(result["scan_completed"], true),
        other => panic!("unexpected ACL acceptance outcome: {other:?}"),
    }
    assert_eq!(result["originals_unchanged"], true);
    assert_eq!(result["evidence_read_only"], true);
}
#[cfg(windows)]
fn exclusive_lock(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(path)
}
#[cfg(windows)]
fn process_stopped(pid: u32) -> bool {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::Threading::{OpenProcess, WaitForSingleObject},
    };
    let handle = unsafe { OpenProcess(0x00100000, 0, pid) };
    if handle.is_null() {
        assert_eq!(
            unsafe { GetLastError() },
            87,
            "cannot establish process exit: OpenProcess failed"
        );
        return true;
    }
    let state = unsafe { WaitForSingleObject(handle, 0) };
    unsafe { CloseHandle(handle) };
    assert!(
        state == WAIT_OBJECT_0 || state == WAIT_TIMEOUT,
        "cannot establish process exit"
    );
    state == WAIT_OBJECT_0
}
#[cfg(windows)]
#[test]
#[ignore = "requires actual Windows with configured elevated Codex sandbox and HARNESS_WINDOWS_CREATOR_SANDBOX=1"]
fn owner_death_keeps_runner_exclusion_until_elevated_descendants_exit() {
    let codex = require_codex();
    let root = FixtureRoot::new("owner death");
    let mut owner = OwnedDriver::launch("owner", &root.0, &codex);
    let marker = root.0.join("control/agent/pids.json");
    let lock = root.0.join("runner.lock");
    let until = Instant::now() + Duration::from_secs(180);
    let pids: Vec<u32> = loop {
        if let Ok(bytes) = fs::read(&marker)
            && let Ok(pids) = serde_json::from_slice::<Vec<u32>>(&bytes)
            && pids.len() == 2
        {
            break pids;
        }
        assert!(
            owner
                .0
                .as_mut()
                .unwrap()
                .try_wait()
                .expect("poll owner")
                .is_none(),
            "sandbox owner exited before publishing native PIDs"
        );
        assert!(
            Instant::now() < until,
            "elevated sandbox did not launch both native processes"
        );
        std::thread::sleep(Duration::from_millis(25));
    };
    assert!(
        pids.iter().all(|pid| *pid > 0 && !process_stopped(*pid)),
        "both sandbox processes must actually be alive before owner death"
    );
    assert!(
        exclusive_lock(&lock).is_err(),
        "running owner must hold runner exclusion"
    );
    owner.terminate_owner();
    let until = Instant::now() + Duration::from_secs(30);
    let replacement = loop {
        match exclusive_lock(&lock) {
            Ok(file) => break file,
            Err(error) => {
                assert!(
                    matches!(error.raw_os_error(), Some(32) | Some(33)),
                    "unexpected runner-lock failure: {error}"
                );
                assert!(
                    Instant::now() < until,
                    "guardian did not release runner exclusion after owner death"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    };
    for pid in pids {
        assert!(
            process_stopped(pid),
            "native sandbox descendant survived replacement admission: {pid}"
        );
    }
    drop(replacement);
}
#[cfg(not(windows))]
#[test]
#[ignore = "this acceptance target requires actual Windows; it must not report a native pass on another OS"]
fn owner_death_requires_actual_windows() {
    panic!("native Windows execution is required");
}
