//! Real SQLite and installer calls run against one private, child-isolated driver.
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const DEADLINE: Duration = Duration::from_secs(30);
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let selected =
            std::env::temp_dir().join(format!("infra-setup-domain-{}", uuid::Uuid::new_v4()));
        infra_host::util::private_directory(&selected).unwrap();
        let root = Self(infra_core::fs::canonicalize(selected).unwrap());
        assert_eq!(
            Driver::spawn(&root.0, "prepare", false).wait(),
            json!({"prepared":true})
        );
        root
    }
    fn signal(&self, name: &str) {
        fs::write(self.0.join(name), b"ready").unwrap();
    }
    fn wait(&self, name: &str, duration: Duration) -> bool {
        let until = Instant::now() + duration;
        while !self.0.join(name).is_file() {
            if Instant::now() >= until {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        true
    }
    fn gate(&self, scope: &str) {
        assert!(["source", "project"].contains(&scope));
        self.signal(&format!("gate-{scope}"));
    }
    fn start(&self, scenario: &str) -> Driver {
        Driver::spawn(&self.0, scenario, true)
    }
    fn assert_no_fake_children(&self) {
        let until = Instant::now() + DEADLINE;
        while fake_children(&self.0) {
            assert!(Instant::now() < until, "Fake Git children did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
fn fake_children(root: &Path) -> bool {
    fs::read_dir(root).unwrap().any(|entry| {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        name.starts_with("fake-git-") && name.ends_with(".live")
    })
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::write(self.0.join("release-git"), b"cleanup");
        let until = Instant::now() + DEADLINE;
        while fake_children(&self.0) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        // Preserve evidence rather than delete synchronization under a live
        // fixture child if a platform failed to confirm bounded cleanup.
        if !fake_children(&self.0) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
struct Driver {
    child: Option<Child>,
    root: PathBuf,
    stdout: PathBuf,
    stderr: PathBuf,
}
impl Driver {
    fn spawn(root: &Path, scenario: &str, fake_git: bool) -> Self {
        let stdout = root.join(format!("{scenario}.stdout"));
        let stderr = root.join(format!("{scenario}.stderr"));
        let mut command = Command::new(env!("CARGO_BIN_EXE_infra-harness-setup-fixture"));
        command
            .args([std::ffi::OsStr::new(scenario), root.as_os_str()])
            .current_dir(root)
            .stdin(Stdio::null())
            .stdout(File::create(&stdout).unwrap())
            .stderr(File::create(&stderr).unwrap());
        if fake_git {
            // Deliberately select only our fake executable in this child. The
            // test process and HOME/CODEX_HOME/provider environment are intact.
            command.env("PATH", root.join("bin"));
        }
        Self {
            child: Some(command.spawn().unwrap()),
            root: root.into(),
            stdout,
            stderr,
        }
    }
    fn wait(mut self) -> Value {
        let until = Instant::now() + DEADLINE * 2;
        let status = loop {
            if let Some(status) = self.child.as_mut().unwrap().try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < until, "Setup driver exceeded deadline");
            std::thread::sleep(Duration::from_millis(5));
        };
        self.child.take().unwrap().wait().unwrap();
        fn read(selected: &Path) -> Vec<u8> {
            let mut bytes = Vec::new();
            File::open(selected)
                .unwrap()
                .take(65537)
                .read_to_end(&mut bytes)
                .unwrap();
            assert!(bytes.len() <= 65536, "Fixture output exceeded bound");
            bytes
        }
        let stdout = read(&self.stdout);
        let stderr = read(&self.stderr);
        assert!(
            status.success(),
            "driver stderr: {}",
            String::from_utf8_lossy(&stderr)
        );
        assert!(
            !fake_children(&self.root),
            "Driver returned before fake Git completion"
        );
        serde_json::from_slice(&stdout).unwrap()
    }
}
impl Drop for Driver {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = fs::write(self.root.join("release-git"), b"cleanup");
            let until = Instant::now() + DEADLINE;
            while child.try_wait().ok().flatten().is_none() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
fn complete(root: &Root, driver: Driver, scenario: &str) {
    assert_eq!(driver.wait(), json!({"ok":true,"scenario":scenario}));
    root.assert_no_fake_children();
}
fn gated(scenario: &str, scope: &str, proof: &str) {
    let root = Root::new();
    root.gate(scope);
    let driver = root.start(scenario);
    let entered = root.wait("git-entered", DEADLINE);
    let proved_before_release = entered && root.wait(proof, Duration::from_secs(5));
    root.signal("release-git");
    complete(&root, driver, scenario);
    assert!(
        entered,
        "The real Setup call never entered its fake Git boundary"
    );
    assert!(
        proved_before_release,
        "{proof} must precede the external Git release"
    );
}

#[test]
fn actual_session_poll_progresses_while_status_is_in_synchronous_git() {
    gated("responsive", "project", "session-polled");
}
#[test]
fn independent_http_session_poll_completes_before_gated_setup_response() {
    gated("http-responsive", "project", "http-session-polled");
}
#[test]
fn dropped_queued_setup_requests_do_not_submit_work_or_keep_reservations() {
    gated("queued", "project", "queue-checked");
}
#[test]
fn aborted_active_preview_retains_custody_until_close_drains_it() {
    gated("active-cancel", "source", "close-pending");
}
#[test]
fn busy_setup_preserves_preview_and_install_validation_priority() {
    gated("busy", "source", "busy-checked");
}
#[test]
fn close_drains_an_earlier_live_waiter_and_refuses_a_later_one() {
    gated("fifo", "project", "fifo-checked");
}
#[test]
fn real_reviewed_install_verifies_bytes_and_consumes_its_preview_once() {
    let root = Root::new();
    let driver = root.start("install");
    complete(&root, driver, "install");
}
#[test]
fn closing_after_idle_rolls_back_an_observed_publication_before_drain() {
    let root = Root::new();
    let driver = root.start("rollback");
    let entered = root.wait("git-entered", DEADLINE);
    let pending_before_release = entered && root.wait("rollback-pending", Duration::from_secs(5));
    root.signal("release-git");
    complete(&root, driver, "rollback");
    assert!(entered);
    assert!(
        pending_before_release,
        "Close must wait for the actual installer rollback"
    );
}
#[test]
fn wrapper_keeps_runtime_free_synchronous_calls_and_terminal_close() {
    let root = Root::new();
    let driver = root.start("runtime-free");
    complete(&root, driver, "runtime-free");
}
