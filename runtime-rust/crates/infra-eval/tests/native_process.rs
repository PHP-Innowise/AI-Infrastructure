#![cfg(target_os = "linux")]
//! Real native ownership and I/O regressions; no providers or shell programs.
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    sync::Once,
    time::{Duration, Instant},
};
static SUBREAPER: Once = Once::new();
fn subreaper() {
    SUBREAPER.call_once(|| {
        assert_eq!(
            unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) },
            0
        )
    });
}
#[derive(Clone, Debug)]
struct Image {
    pid: i32,
    ppid: i32,
    pgid: i32,
    start: u64,
    state: char,
}
fn current(pid: i32) -> io::Result<Option<Image>> {
    let raw = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let fields = raw
        .rsplit_once(") ")
        .ok_or_else(|| io::Error::other("invalid proc image"))?
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    let value = |i: usize| {
        fields
            .get(i)
            .ok_or_else(|| io::Error::other("short proc image"))
    };
    Ok(Some(Image {
        pid,
        state: value(0)?
            .chars()
            .next()
            .ok_or_else(|| io::Error::other("missing process state"))?,
        ppid: value(1)?.parse().map_err(io::Error::other)?,
        pgid: value(2)?.parse().map_err(io::Error::other)?,
        start: value(19)?.parse().map_err(io::Error::other)?,
    }))
}
fn from_json(value: &Value) -> Image {
    Image {
        pid: value["pid"].as_i64().unwrap() as i32,
        ppid: value["ppid"].as_i64().unwrap() as i32,
        pgid: value["pgid"].as_i64().unwrap() as i32,
        start: value["start"].as_u64().unwrap(),
        state: '?',
    }
}
fn same_identity(image: &Image) -> bool {
    current(image.pid)
        .expect("inspect owned identity")
        .is_some_and(|now| now.start == image.start && now.pgid == image.pgid)
}
fn alive(image: &Image) -> bool {
    current(image.pid)
        .expect("inspect owned process")
        .is_some_and(|now| {
            now.start == image.start && now.pgid == image.pgid && !matches!(now.state, 'Z' | 'X')
        })
}
struct Pinned {
    image: Image,
    fd: OwnedFd,
}
impl Pinned {
    fn open(image: Image) -> Option<Self> {
        if !same_identity(&image) {
            return None;
        }
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, image.pid, 0) } as i32;
        if raw < 0 {
            if io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                return None;
            }
            panic!("pin known fixture process: {}", io::Error::last_os_error());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        if !same_identity(&image) {
            return None;
        }
        Some(Self { image, fd })
    }
    fn signal_result(&self, signal: i32) -> io::Result<()> {
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.fd.as_raw_fd(),
                signal,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if result == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    fn signal(&self, signal: i32) {
        self.signal_result(signal)
            .expect("signal only held fixture identity");
    }
    fn reap(&self) {
        if current(self.image.pid).ok().flatten().is_some_and(|now| {
            now.start == self.image.start
                && now.pgid == self.image.pgid
                && matches!(now.state, 'Z' | 'X')
        }) {
            unsafe {
                libc::waitpid(self.image.pid, std::ptr::null_mut(), libc::WNOHANG);
            }
        }
    }
}
struct Fixture {
    root: PathBuf,
    child: Option<Child>,
    driver: Image,
    driver_fd: Pinned,
    images: BTreeMap<i32, Image>,
    held: BTreeMap<i32, Pinned>,
}
impl Fixture {
    fn start(scenario: &str) -> Self {
        subreaper();
        let root = std::env::temp_dir().join(format!(
            "infra-eval-captured-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir(&root).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(root.join("stray"), b"owned descriptor sentinel").unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_infra-eval-process-fixture"))
            .arg("process")
            .arg(&root)
            .arg(scenario)
            .env("EVAL_CAPTURE_AMBIENT", "must-be-absent")
            .env("RUST_BACKTRACE", "0")
            .stdin(Stdio::null())
            .stdout(fs::File::create(root.join("driver.stdout")).unwrap())
            .stderr(fs::File::create(root.join("driver.stderr")).unwrap())
            .spawn()
            .unwrap();
        let driver = current(child.id() as i32).unwrap().unwrap();
        let driver_fd = Pinned::open(driver.clone()).expect("pin own direct child");
        Self {
            root,
            child: Some(child),
            driver,
            driver_fd,
            images: BTreeMap::new(),
            held: BTreeMap::new(),
        }
    }
    fn collect(&mut self) {
        let Ok(entries) = fs::read_dir(self.root.join("receipts")) else {
            return;
        };
        for entry in entries {
            let entry = entry.unwrap();
            let Ok(raw) = fs::read(entry.path()) else {
                continue;
            };
            let Ok(value) = serde_json::from_slice::<Value>(&raw) else {
                continue;
            };
            let image = from_json(&value);
            self.images
                .entry(image.pid)
                .or_insert_with(|| image.clone());
            if !self.held.contains_key(&image.pid)
                && let Some(pin) = Pinned::open(image)
            {
                self.held.insert(pin.image.pid, pin);
            }
        }
        for name in ["guardian.json", "anchor.json"] {
            if let Ok(raw) = fs::read(self.root.join(name))
                && let Ok(value) = serde_json::from_slice::<Value>(&raw)
            {
                let image = from_json(&value);
                if image.pid != self.driver.pid
                    && image.ppid == self.driver.pid
                    && image.pgid == image.pid
                {
                    self.images
                        .entry(image.pid)
                        .or_insert_with(|| image.clone());
                    if !self.held.contains_key(&image.pid)
                        && let Some(pin) = Pinned::open(image)
                    {
                        self.held.insert(pin.image.pid, pin);
                    }
                }
            }
        }
    }
    fn wait_ready(&mut self) {
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            self.collect();
            if self.root.join("ready").exists() {
                return;
            }
            assert!(
                Instant::now() < until,
                "fixture did not report real native readiness"
            );
            if self.child.as_mut().unwrap().try_wait().unwrap().is_some() {
                self.child.take();
                panic!("driver exited before native readiness");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn wait(&mut self) -> (ExitStatus, Value) {
        let until = Instant::now() + Duration::from_secs(8);
        let status = loop {
            self.collect();
            if let Some(status) = self.child.as_mut().unwrap().try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < until,
                "native controller exceeded test deadline"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        self.child.take();
        self.collect();
        let raw = self.read("driver.stdout");
        let value = serde_json::from_slice(&raw).unwrap_or_else(|_| {
            panic!(
                "missing driver receipt: stdout {}; stderr {}",
                String::from_utf8_lossy(&raw),
                String::from_utf8_lossy(&self.read("driver.stderr"))
            )
        });
        (status, value)
    }
    fn read(&self, name: &str) -> Vec<u8> {
        let mut raw = Vec::new();
        fs::File::open(self.root.join(name))
            .unwrap()
            .take(1024 * 1024 + 1)
            .read_to_end(&mut raw)
            .unwrap();
        assert!(raw.len() <= 1024 * 1024, "fixture capture read bound");
        raw
    }
    fn report(&mut self) -> Value {
        let (status, value) = self.wait();
        assert_eq!(
            status.code(),
            Some(0),
            "driver failed: {value}; {}",
            String::from_utf8_lossy(&self.read("driver.stderr"))
        );
        assert_eq!(value["ok"], true, "driver operation failed: {value}");
        assert_eq!(
            value["live_at_return"],
            json!([]),
            "API returned before owned-tree cleanup: {value}"
        );
        self.assert_stopped();
        value
    }
    fn assert_stopped(&mut self) {
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            self.collect();
            if self.images.values().all(|image| !alive(image)) {
                return;
            }
            assert!(
                Instant::now() < until,
                "owned native tree still live at controller completion: {:?}",
                self.images
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            // Cleanup must remain safe during a failed assertion. Every signal
            // uses a previously verified pidfd; never a saved numeric PGID.
            let collected =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.collect())).is_ok();
            for pin in self.held.values() {
                let _ = pin.signal_result(libc::SIGCONT);
                let _ = pin.signal_result(libc::SIGKILL);
                pin.reap();
            }
            let all_dead = self.images.values().all(|image| {
                current(image.pid).is_ok_and(|now| {
                    now.is_none_or(|now| {
                        now.start != image.start
                            || now.pgid != image.pgid
                            || matches!(now.state, 'Z' | 'X')
                    })
                })
            });
            if all_dead && collected {
                break;
            }
            if Instant::now() >= until {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn native125_binary_io_and_complete_environment_are_preserved() {
    let mut fixture = Fixture::start("io125");
    let value = fixture.report();
    assert_eq!(value["session"]["exit"], 125);
    assert_eq!(value["session"]["stopped"], "finished");
    assert_eq!(fixture.read("input.bin"), vec![0, 1, 2, 0, 255]);
    assert_eq!(fixture.read("stdout.bin"), b"out\0\xff");
    assert_eq!(fixture.read("stderr.bin"), b"err\0\xfe");
    let env: Value = serde_json::from_slice(&fixture.read("environment.json")).unwrap();
    assert_eq!(env, json!({"present":"yes","ambient":null}));
}
#[test]
fn unread_large_stdin_does_not_replace_native0_or125_with_broken_pipe() {
    for (scenario, code) in [("unread0", 0), ("unread125", 125)] {
        let mut fixture = Fixture::start(scenario);
        let value = fixture.report();
        assert_eq!(value["session"]["exit"], code);
        assert_eq!(value["session"]["stopped"], "finished");
    }
}
#[test]
fn native_closing_both_streams_still_waits_for_actual125() {
    let mut fixture = Fixture::start("early-eof125");
    let value = fixture.report();
    assert_eq!(value["session"]["exit"], 125);
    assert!(value["session"]["seconds"].as_f64().unwrap() >= 0.1);
}
#[test]
fn descendants_late_native_event_is_drained_after_the_leader_exits() {
    let mut fixture = Fixture::start("late-event");
    let value = fixture.report();
    assert_eq!(value["session"]["completed"], true);
    assert_eq!(value["session"]["exit"], 0);
    assert_eq!(value["session"]["answer"], "late descendant answer");
    assert_eq!(
        value["session"]["usage"],
        json!({"input_tokens":1,"output_tokens":2})
    );
}
#[test]
fn observed125_survives_descendant_stream_timeout() {
    let mut fixture = Fixture::start("held125");
    let value = fixture.report();
    assert_eq!(value["session"]["stopped"], "timeout");
    assert_eq!(value["session"]["exit"], 125);
}
#[test]
fn native125_is_observed_before_cleanup_resumes_a_stopped_guardian() {
    let mut fixture = Fixture::start("held125-stopped-guardian");
    let value = fixture.report();
    assert_eq!(
        fixture.read("guardian-stopped"),
        b"confirmed-stopped-before-exit"
    );
    assert_eq!(value["session"]["stopped"], "timeout");
    assert_eq!(value["session"]["exit"], 125);
    assert!(value["session"]["seconds"].as_f64().unwrap() < 5.0);
}
#[test]
fn timeout_and_backpressured_stdin_are_bounded_and_clean_the_tree() {
    for scenario in ["timeout", "input-backpressure"] {
        let mut fixture = Fixture::start(scenario);
        let value = fixture.report();
        assert_eq!(value["session"]["stopped"], "timeout");
        assert!(value["session"]["seconds"].as_f64().unwrap() < 5.0);
    }
}
#[test]
fn one_output_budget_covers_separate_stdout_and_stderr() {
    let mut fixture = Fixture::start("output-cap");
    let value = fixture.report();
    assert_eq!(value["session"]["stopped"], "output-limit");
    assert!(fixture.read("stdout.bin").len() + fixture.read("stderr.bin").len() <= 16384);
}
#[test]
fn real_readiness_precedes_cancellation_of_the_owned_tree() {
    let mut fixture = Fixture::start("cancel");
    fixture.wait_ready();
    fixture.driver_fd.signal(libc::SIGTERM);
    let value = fixture.report();
    assert_eq!(value["session"]["stopped"], "interrupted");
}
#[test]
fn capture_minimizes_native_descriptors_without_leaking_a_stray_file() {
    let mut fixture = Fixture::start("fd-scrub");
    fixture.report();
    let fds: Vec<i32> = serde_json::from_slice(&fixture.read("fds.json")).unwrap();
    assert_eq!(fds, vec![0, 1, 2]);
}
#[test]
fn observer_panic_retains_state_and_refuses_new_admission() {
    let mut fixture = Fixture::start("callback-panic");
    let (status, value) = fixture.wait();
    assert_eq!(
        status.code(),
        Some(0),
        "missing caught-panic receipt: {value}"
    );
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["code"], "cleanup-unconfirmed");
    assert_eq!(value["retained_exists"], true);
    assert_eq!(value["admission_blocked"], true);
    assert_eq!(value["next_log_absent"], true);
    assert_eq!(value["next_spawn_absent"], true);
    fixture.assert_stopped();
}
#[test]
fn unexpected_guardian_loss_cannot_leave_an_owned_native_tree() {
    let mut fixture = Fixture::start("guardian-loss");
    fixture.wait_ready();
    let guardian: Value = serde_json::from_slice(&fixture.read("guardian.json")).unwrap();
    let image = from_json(&guardian);
    assert_ne!(
        image.pid, fixture.driver.pid,
        "capture must use the real guardian fixture, not libtest/direct-child fallback"
    );
    assert_eq!(image.ppid, fixture.driver.pid);
    assert_eq!(image.pgid, image.pid);
    let pin = Pinned::open(image).expect("pin actual live guardian");
    pin.signal(libc::SIGKILL);
    let (status, value) = fixture.wait();
    assert_eq!(status.code(), Some(0));
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["code"], "native-guardian-exited");
    fixture.assert_stopped();
}

fn experiment_layout(fixture: &Fixture, value: &Value) {
    let anchor: Value = serde_json::from_slice(&fixture.read("anchor.json")).unwrap();
    let observer: Value = serde_json::from_slice(&fixture.read("guardian.json")).unwrap();
    let leaf = fixture
        .images
        .values()
        .find(|image| {
            Some(image.ppid as i64) == observer["pid"].as_i64()
                && Some(image.pgid as i64) == anchor["pid"].as_i64()
        })
        .expect("actual native child of the outside observer");
    assert_eq!(anchor["ppid"], fixture.driver.pid);
    assert_eq!(observer["ppid"], fixture.driver.pid);
    assert_eq!(anchor["pid"], anchor["pgid"]);
    assert_eq!(observer["pid"], observer["pgid"]);
    assert_eq!(anchor["pid"], leaf.pgid);
    assert_eq!(observer["pid"], leaf.ppid);
    assert_ne!(anchor["pid"], observer["pid"]);
    assert_ne!(
        anchor["pgid"], value["caller_pgid"],
        "native group must be isolated from caller"
    );
}
#[test]
fn eval_preserves_genuine_native_signal_exit_values() {
    for (scenario, signal) in [("signal-int", libc::SIGINT), ("signal-kill", libc::SIGKILL)] {
        let mut fixture = Fixture::start(scenario);
        let value = fixture.report();
        assert_eq!(value["session"]["exit"], -signal);
        assert_eq!(value["session"]["completed"], false);
        assert_eq!(value["session"]["usage_complete"], false);
    }
}
#[test]
fn scheduled_experiment_preserves_natural125_without_waiting_for_the_signal() {
    let mut fixture = Fixture::start("experiment-natural125");
    let value = fixture.report();
    experiment_layout(&fixture, &value);
    assert_eq!(value["session"]["exit"], 125);
    assert_eq!(value["native"]["code"], 125);
    assert_eq!(value["native"]["signal"], Value::Null);
    assert!(value["seconds"].as_f64().unwrap() < 0.8);
}
#[test]
fn scheduled_term_only_ends_native_leader_and_drains_descendant_during_grace() {
    let mut fixture = Fixture::start("experiment-term");
    let value = fixture.report();
    experiment_layout(&fixture, &value);
    assert_eq!(value["native"]["code"], Value::Null);
    assert_eq!(value["native"]["signal"], libc::SIGTERM);
    assert_eq!(value["session"]["exit"], -libc::SIGTERM);
    assert_eq!(fixture.read("experiment-late"), b"descendant-completed");
    assert_eq!(fixture.read("stdout.bin"), b"late-experiment-output");
    assert!(value["seconds"].as_f64().unwrap() >= 0.2);
}
#[test]
fn scheduled_group_signals_keep_actual_native_status_outside_the_killed_group() {
    for (scenario, signal) in [
        ("experiment-int", libc::SIGINT),
        ("experiment-kill", libc::SIGKILL),
    ] {
        let mut fixture = Fixture::start(scenario);
        let value = fixture.report();
        experiment_layout(&fixture, &value);
        assert_eq!(value["native"]["code"], Value::Null);
        assert_eq!(value["native"]["signal"], signal);
        assert_eq!(value["session"]["exit"], -signal);
    }
}
#[test]
fn scheduled_term_never_retargets_an_already_exited_native_leader() {
    let mut fixture = Fixture::start("experiment-leader-exited");
    let value = fixture.report();
    experiment_layout(&fixture, &value);
    assert_eq!(value["session"]["exit"], 0);
    assert_eq!(fixture.read("experiment-late"), b"descendant-completed");
    assert_eq!(fixture.read("stdout.bin"), b"late-experiment-output");
}
#[test]
fn scheduled_experiment_waits_for_backpressured_input_after_native_exit_and_output_eof() {
    let mut fixture = Fixture::start("experiment-input");
    let value = fixture.report();
    experiment_layout(&fixture, &value);
    assert_eq!(value["session"]["exit"], 0);
    assert_eq!(
        fixture.read("experiment-input-started"),
        b"reading-after-delayed-backpressure"
    );
    assert!(value["seconds"].as_f64().unwrap() >= 0.15);
}
#[test]
fn scheduled_grace_expiry_kills_stubborn_descendants_without_replacing_native_status() {
    let mut fixture = Fixture::start("experiment-grace");
    let value = fixture.report();
    experiment_layout(&fixture, &value);
    assert_eq!(value["session"]["exit"], -libc::SIGTERM);
    assert!(value["seconds"].as_f64().unwrap() >= 0.15);
    assert!(value["seconds"].as_f64().unwrap() < 2.0);
}
#[test]
fn lifecycle_adapter_returns_the_actual_signed_native_exit() {
    let mut fixture = Fixture::start("experiment-adapter-term");
    let value = fixture.report();
    experiment_layout(&fixture, &value);
    assert_eq!(value["session"]["exit"], -libc::SIGTERM);
    assert_eq!(fixture.read("experiment-late"), b"descendant-completed");
}
#[test]
fn scheduled_owner_cleans_native_group_when_observer_or_anchor_dies() {
    for (scenario, entry) in [
        ("experiment-observer-loss", "guardian.json"),
        ("experiment-anchor-loss", "anchor.json"),
    ] {
        let mut fixture = Fixture::start(scenario);
        fixture.wait_ready();
        let image: Value = serde_json::from_slice(&fixture.read(entry)).unwrap();
        let pin = Pinned::open(from_json(&image)).expect("pin actual direct-child custody anchor");
        assert_eq!(image["ppid"], fixture.driver.pid);
        pin.signal(libc::SIGKILL);
        let (status, value) = fixture.wait();
        assert_eq!(status.code(), Some(0));
        assert_eq!(value["ok"], false);
        assert_eq!(value["error"]["code"], "native-guardian-exited");
        assert_eq!(value["live_at_return"], json!([]));
        fixture.assert_stopped();
    }
}
#[test]
fn scheduled_anchor_and_observer_clean_the_owned_tree_after_driver_death() {
    let mut fixture = Fixture::start("experiment-parent-loss");
    fixture.wait_ready();
    fixture.driver_fd.signal(libc::SIGKILL);
    let mut child = fixture.child.take().unwrap();
    let status = child.wait().unwrap();
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(status.signal(), Some(libc::SIGKILL));
    fixture.assert_stopped();
}

#[test]
fn zero_delay_is_valid_after_actual_native_admission() {
    let mut fixture = Fixture::start("experiment-zero-delay");
    let value = fixture.report();
    assert_eq!(value["session"]["exit"], -libc::SIGKILL);
    assert_eq!(value["native"]["code"], Value::Null);
    assert_eq!(value["native"]["signal"], libc::SIGKILL);
    let anchor: Value = serde_json::from_slice(&fixture.read("anchor.json")).unwrap();
    let observer: Value = serde_json::from_slice(&fixture.read("guardian.json")).unwrap();
    assert_eq!(anchor["ppid"], fixture.driver.pid);
    assert_eq!(observer["ppid"], fixture.driver.pid);
    assert_ne!(anchor["pgid"], value["caller_pgid"]);
    assert_ne!(anchor["pgid"], observer["pgid"]);
}
#[test]
fn experiment_delay_cannot_run_before_the_stopped_observer_admits_native() {
    let mut fixture = Fixture::start("experiment-start-admission");
    let until = Instant::now() + Duration::from_secs(3);
    let pin = loop {
        fixture.collect();
        if fixture.root.join("observer-boot-stop").exists() {
            let value: Value = serde_json::from_slice(&fixture.read("guardian.json")).unwrap();
            let image = from_json(&value);
            if current(image.pid)
                .unwrap()
                .is_some_and(|now| now.start == image.start && now.state == 'T')
            {
                break Pinned::open(image).expect("pin actual stopped observer helper");
            }
        }
        assert!(
            Instant::now() < until,
            "observer did not stop before native admission"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    // More than the configured80ms passes while native has not been spawned.
    std::thread::sleep(Duration::from_millis(180));
    assert!(
        !fixture
            .images
            .values()
            .any(|image| image.ppid == pin.image.pid),
        "native work must await helper bootstrap"
    );
    pin.signal(libc::SIGCONT);
    let value = fixture.report();
    experiment_layout(&fixture, &value);
    assert_eq!(value["session"]["exit"], -libc::SIGKILL);
    assert!(
        !fixture
            .root
            .join("experiment-start-was-not-interrupted")
            .exists(),
        "the scheduled signal ran before native admission instead of after its delay"
    );
}
