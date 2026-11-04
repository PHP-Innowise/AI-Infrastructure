//! Explicit fake executables exercise public discovery and actual native guardians.
//! Overrides deliberately suppress Cursor's PATH-based candidate fallback; that
//! unchanged production loop is not claimed as exercised by these fixtures.
#![cfg(any(target_os = "linux", target_os = "macos", windows))]
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const IDS: [&str; 3] = ["claude", "codex", "cursor"];
struct Driver(Option<Child>);
impl Driver {
    fn spawn(root: &Path, arguments: &[&std::ffi::OsStr]) -> Self {
        Self(Some(
            Command::new(env!("CARGO_BIN_EXE_infra-host-provider-fixture"))
                .args(arguments)
                .current_dir(root)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        ))
    }
    fn wait(mut self) -> Value {
        let until = Instant::now() + Duration::from_secs(20);
        loop {
            if self.0.as_mut().unwrap().try_wait().unwrap().is_some() {
                break;
            }
            assert!(
                Instant::now() < until,
                "Fake provider driver exceeded its deadline"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let output = self.0.take().unwrap().wait_with_output().unwrap();
        assert!(output.stdout.len() + output.stderr.len() <= 65536);
        assert!(
            output.status.success(),
            "driver: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}
impl Drop for Driver {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let selected =
            std::env::temp_dir().join(format!("infra-provider-discovery-{}", uuid::Uuid::new_v4()));
        infra_host::util::private_directory(&selected).unwrap();
        let selected = fs::canonicalize(selected).unwrap();
        assert_eq!(
            Driver::spawn(&selected, &["prepare".as_ref(), selected.as_os_str()]).wait(),
            json!({"prepared":true})
        );
        let root = Self(selected);
        for id in IDS {
            root.spec(id, json!({}));
        }
        root
    }
    fn spec(&self, id: &str, value: Value) {
        fs::write(
            self.0.join(format!("{id}.json")),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
    }
    fn start(&self, present: &[&str], drop_after_signal: bool) -> Driver {
        let overrides = IDS
            .into_iter()
            .map(|id| {
                let prefix = if present.contains(&id) {
                    "fake"
                } else {
                    "missing"
                };
                (
                    id,
                    self.0
                        .join(format!("{prefix}-{id}{}", std::env::consts::EXE_SUFFIX)),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let request = self.0.join("request.json");
        fs::write(
            &request,
            serde_json::to_vec(
                &json!({"overrides":overrides,"drop_after_signal":drop_after_signal}),
            )
            .unwrap(),
        )
        .unwrap();
        Driver::spawn(&self.0, &["discover".as_ref(), request.as_os_str()])
    }
    fn signal(&self, name: &str) {
        fs::write(self.0.join(name), b"ready").unwrap();
    }
    fn ready(&self) -> Option<Vec<Value>> {
        IDS.into_iter()
            .map(|id| {
                fs::read(self.0.join(format!("{id}.version.ready")))
                    .ok()
                    .and_then(|raw| serde_json::from_slice(&raw).ok())
            })
            .collect()
    }
    fn wait_ready(&self) -> Option<Vec<Value>> {
        // Release before the existing three-second native probe deadline even
        // when the old serial implementation cannot enter all three leaves.
        let until = Instant::now() + Duration::from_millis(2400);
        loop {
            if let Some(ready) = self.ready() {
                return Some(ready);
            }
            if Instant::now() >= until {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn calls(&self, id: &str) -> Vec<Value> {
        fs::read_to_string(self.0.join(format!("{id}.calls.jsonl")))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    fn entered(&self, id: &str) -> Vec<Value> {
        self.calls(id)
            .into_iter()
            .filter(|event| event["event"] == "entered")
            .map(|event| event["args"].clone())
            .collect()
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn expected_args(id: &str, flag: &str) -> Value {
    if id == "cursor" {
        json!(["--disable-auto-update", flag])
    } else {
        json!([flag])
    }
}
fn providers(value: &Value, available: [bool; 3]) {
    assert_eq!(value["ok"], true, "{value}");
    let rows = value["providers"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    for ((row, id), expected) in rows.iter().zip(IDS).zip(available) {
        assert_eq!(row["id"], id);
        assert_eq!(row["available"], expected, "{id}: {value}");
        assert_eq!(row["executable"].is_string(), expected);
    }
}

#[test]
fn providers_overlap_but_each_keeps_its_version_help_order_and_exact_flags() {
    let root = Root::new();
    for id in IDS {
        root.spec(id, json!({"gate_version":true}));
    }
    let driver = root.start(&IDS, false);
    let ready = root.wait_ready();
    if ready.is_some() {
        for id in IDS {
            assert_eq!(root.entered(id), vec![expected_args(id, "--version")]);
        }
    }
    root.signal("release-versions");
    let value = driver.wait();
    assert!(
        ready.is_some(),
        "All three version probes must enter before release"
    );
    providers(&value, [true; 3]);
    for id in IDS {
        assert_eq!(
            root.entered(id),
            vec![expected_args(id, "--version"), expected_args(id, "--help")]
        );
        let events = root.calls(id);
        assert_eq!(
            events
                .iter()
                .map(|event| event["event"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["entered", "finished", "entered", "finished"]
        );
        assert_eq!(events[1]["args"], expected_args(id, "--version"));
        assert_eq!(events[2]["args"], expected_args(id, "--help"));
    }
}

#[test]
fn nonzero_version_wrong_identity_and_missing_cursor_flags_stay_unavailable() {
    let root = Root::new();
    root.spec("claude", json!({"version_code":7}));
    root.spec("codex", json!({"help":"A different CLI"}));
    root.spec(
        "cursor",
        json!({"help":"Cursor --print --output-format --resume --workspace --sandbox"}),
    );
    providers(&root.start(&IDS, false).wait(), [false; 3]);
    assert_eq!(
        root.entered("claude"),
        vec![expected_args("claude", "--version")]
    );
    for id in ["codex", "cursor"] {
        assert_eq!(
            root.entered(id),
            vec![expected_args(id, "--version"), expected_args(id, "--help")]
        );
    }
}

#[test]
fn zero_or_one_available_provider_never_uses_a_real_path_lookup() {
    for present in [vec![], vec!["claude"]] {
        let root = Root::new();
        let expected = [!present.is_empty(), false, false];
        providers(&root.start(&present, false).wait(), expected);
        for (id, available) in IDS.into_iter().zip(expected) {
            assert_eq!(root.entered(id).len(), if available { 2 } else { 0 });
        }
    }
}

#[test]
fn output_limit_failure_stops_that_provider_without_hiding_healthy_peers() {
    let root = Root::new();
    root.spec("claude", json!({"output_bytes":65537}));
    providers(&root.start(&IDS, false).wait(), [false, true, true]);
    assert_eq!(
        root.entered("claude"),
        vec![expected_args("claude", "--version")]
    );
}

#[cfg(target_os = "linux")]
mod linux_cleanup {
    use super::*;
    use std::{
        collections::BTreeSet,
        fs::File,
        os::fd::{AsRawFd, FromRawFd},
    };

    fn start_identity(pid: libc::pid_t) -> Option<u64> {
        let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    || error.raw_os_error() == Some(libc::ESRCH) =>
            {
                return None;
            }
            Err(error) => panic!("Cannot establish captured process identity: {error}"),
        };
        let (_, fields) = stat
            .rsplit_once(") ")
            .expect("Captured process stat fields");
        Some(
            fields
                .split_whitespace()
                .nth(19)
                .expect("Captured process start time")
                .parse()
                .expect("Captured process start time is an integer"),
        )
    }
    struct Pin {
        descriptor: File,
        pid: libc::pid_t,
        started: u64,
    }
    struct Pins(Vec<Pin>);
    impl Pins {
        fn capture(ready: &[Value]) -> Self {
            let pids = ready
                .iter()
                .flat_map(|event| {
                    [
                        event["pid"].as_u64().unwrap(),
                        event["guardian_pid"].as_u64().unwrap(),
                    ]
                })
                .collect::<BTreeSet<_>>();
            assert_eq!(pids.len(), 6, "Pin each fake leaf and its actual guardian");
            Self(
                pids.into_iter()
                    .map(|pid| {
                        let pid = libc::pid_t::try_from(pid).expect("Captured native PID");
                        assert!(pid > 0);
                        let started =
                            start_identity(pid).expect("Captured fixture process is live");
                        let descriptor = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
                        assert!(
                            descriptor >= 0,
                            "pidfd_open({pid}): {}",
                            std::io::Error::last_os_error()
                        );
                        let descriptor = unsafe { File::from_raw_fd(descriptor as i32) };
                        assert!(
                            start_identity(pid).is_none_or(|current| current == started),
                            "Captured PID changed identity while opening its pidfd"
                        );
                        Pin {
                            descriptor,
                            pid,
                            started,
                        }
                    })
                    .collect(),
            )
        }
        fn wait_exited(&self) {
            let until = Instant::now() + Duration::from_secs(10);
            let mut waiting = self
                .0
                .iter()
                .map(|pin| libc::pollfd {
                    fd: pin.descriptor.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                })
                .collect::<Vec<_>>();
            while !waiting.is_empty() {
                let remaining = until.saturating_duration_since(Instant::now());
                assert!(
                    !remaining.is_zero(),
                    "Every pinned fake leaf and guardian must exit"
                );
                let result = unsafe {
                    libc::poll(
                        waiting.as_mut_ptr(),
                        waiting.len() as libc::nfds_t,
                        remaining.as_millis().min(i32::MAX as u128) as i32,
                    )
                };
                if result < 0 {
                    assert_eq!(
                        std::io::Error::last_os_error().raw_os_error(),
                        Some(libc::EINTR)
                    );
                    continue;
                }
                waiting.retain(|pin| pin.revents & (libc::POLLIN | libc::POLLHUP) == 0);
            }
        }
        fn wait_retired(&self) {
            let until = Instant::now() + Duration::from_secs(10);
            loop {
                let retained = self
                    .0
                    .iter()
                    .filter(|pin| start_identity(pin.pid) == Some(pin.started))
                    .map(|pin| pin.pid)
                    .collect::<Vec<_>>();
                if retained.is_empty() {
                    return;
                }
                // Exit notification includes zombies. The owner's final wait
                // must retire every original identity before its driver exits.
                assert!(
                    Instant::now() < until,
                    "Captured identities were not reaped/retired: {retained:?}"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    #[test]
    fn timed_out_identity_probes_keep_unavailable_results_and_reap_every_group() {
        let root = Root::new();
        for id in IDS {
            root.spec(id, json!({"gate_version":true}));
        }
        let driver = root.start(&IDS, false);
        let ready = root
            .wait_ready()
            .expect("All fake probes entered before their deadlines");
        let pins = Pins::capture(&ready);
        providers(&driver.wait(), [false; 3]);
        pins.wait_exited();
        pins.wait_retired();
        for id in IDS {
            assert_eq!(root.entered(id), vec![expected_args(id, "--version")]);
        }
    }

    #[test]
    fn dropping_discovery_reaps_every_fake_group_while_the_driver_stays_alive() {
        let root = Root::new();
        for id in IDS {
            root.spec(id, json!({"gate_version":true}));
        }
        let mut driver = root.start(&IDS, true);
        let ready = root
            .wait_ready()
            .expect("All fake probes entered before cancellation");
        let pins = Pins::capture(&ready);
        root.signal("drop-discovery");
        pins.wait_exited();
        pins.wait_retired();
        assert!(
            driver.0.as_mut().unwrap().try_wait().unwrap().is_none(),
            "Cleanup must precede driver/runtime teardown"
        );
        root.signal("finish-driver");
        assert_eq!(driver.wait(), json!({"ok":true,"dropped":true}));
        for id in IDS {
            assert_eq!(root.entered(id), vec![expected_args(id, "--version")]);
        }
    }
}
