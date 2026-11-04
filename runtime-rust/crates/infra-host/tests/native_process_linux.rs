//! Real Linux native processes exercise the public host supervisor.
//! Helpers use native Rust leaves and current_exe's internal-guard dispatch.
#![cfg(target_os = "linux")]
use serde_json::Value;
use std::{
    fs::{self, File, OpenOptions},
    os::fd::{AsRawFd, FromRawFd},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Once,
    time::{Duration, Instant},
};

const INPUT: &[u8] = b"native stdin\r\nsecond\n\0\xfftail";
const LITERAL: &str = "literal $(false); --value=$HOME";
static SUBREAPER: Once = Once::new();
fn subreaper() {
    SUBREAPER.call_once(|| {
        assert_eq!(
            unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) },
            0,
            "own disposable orphan descendants"
        )
    });
}
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("infra native process {}", uuid::Uuid::new_v4()));
        infra_host::util::private_directory(&path).expect("private process fixture root");
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn json_file(path: &Path) -> Option<Value> {
    fs::read(path)
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok())
}
fn pid(image: &Value) -> i32 {
    image["pid"]
        .as_i64()
        .filter(|pid| *pid > 0)
        .expect("actual native PID") as i32
}
fn state_details(image: &Value) -> Option<(char, u64)> {
    let path = format!("/proc/{}/stat", pid(image));
    let stat = match fs::read_to_string(&path) {
        Ok(stat) => stat,
        Err(error) if matches!(error.kind(), std::io::ErrorKind::NotFound) => return None,
        Err(error) if error.raw_os_error() == Some(libc::ESRCH) => return None,
        Err(error) => panic!("cannot establish native process state: {error}"),
    };
    let (_, fields) = stat
        .rsplit_once(") ")
        .expect("native process stat identity");
    let fields = fields.split_whitespace().collect::<Vec<_>>();
    let birth = fields[19].parse::<u64>().expect("native process birth");
    if Some(birth) != image["birth"].as_u64() {
        return None;
    }
    let flags = fields[6].parse::<u64>().expect("native process flags");
    fields[0].chars().next().map(|state| (state, flags))
}
fn state(image: &Value) -> Option<char> {
    state_details(image).map(|(state, _)| state)
}
fn alive(image: &Value) -> bool {
    state(image).is_some_and(|state| state != 'Z' && state != 'X')
}
struct ProcessPin {
    image: Value,
    descriptor: File,
}
impl ProcessPin {
    fn open(image: &Value) -> std::io::Result<Option<Self>> {
        let descriptor = unsafe { libc::syscall(libc::SYS_pidfd_open, pid(image), 0) };
        if descriptor < 0 {
            let error = std::io::Error::last_os_error();
            return if error.raw_os_error() == Some(libc::ESRCH) {
                Ok(None)
            } else {
                Err(error)
            };
        }
        let descriptor = unsafe { File::from_raw_fd(descriptor as i32) };
        // The PID may have exited/recycled between publishing its image and
        // pidfd_open. Only the captured birth identity admits this handle.
        if state(image).is_none() {
            return Ok(None);
        }
        Ok(Some(Self {
            image: image.clone(),
            descriptor,
        }))
    }
    fn matches(&self, image: &Value) -> bool {
        self.image["pid"] == image["pid"] && self.image["birth"] == image["birth"]
    }
    fn signal(&self, signal: i32) -> std::io::Result<()> {
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.descriptor.as_raw_fd(),
                signal,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
        }
        Ok(())
    }
    fn reap(&self) -> std::io::Result<()> {
        loop {
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            let result = unsafe {
                libc::waitid(
                    libc::P_PIDFD,
                    self.descriptor.as_raw_fd() as libc::id_t,
                    &mut info,
                    libc::WEXITED | libc::WNOHANG,
                )
            };
            if result == 0 {
                return Ok(());
            }
            let error = std::io::Error::last_os_error();
            match error.raw_os_error() {
                Some(libc::EINTR) => continue,
                // A live fixture owns its guardian; this subreaper can reap
                // only descendants adopted after their actual parent exits.
                Some(libc::ECHILD | libc::ESRCH) => return Ok(()),
                _ => return Err(error),
            }
        }
    }
}
fn reap(image: &Value) {
    if let Some(pin) = ProcessPin::open(image).expect("pin only the recorded native process") {
        pin.reap().expect("reap only the pinned adopted process");
    }
}
fn stopped(image: &Value) {
    reap(image);
    let observed = state_details(image);
    assert!(
        !observed.is_some_and(|(state, _)| state != 'Z' && state != 'X'),
        "native process survived confirmed cleanup: {image}; state/flags: {observed:?}"
    );
}
fn records(root: &Path) -> Vec<Value> {
    let mut records = Vec::new();
    if let Some(group) = json_file(&root.join("group.json")) {
        records.push(group["leader"].clone());
        records.push(group["descendant"].clone());
    }
    for name in ["guardian.json", "leaf.json"] {
        if let Some(image) = json_file(&root.join(name)) {
            records.push(image);
        }
    }
    records
        .into_iter()
        .filter(|image| image["pid"].as_i64().is_some_and(|pid| pid > 0))
        .collect()
}
struct Driver {
    child: Option<Child>,
    root: PathBuf,
    pins: Vec<ProcessPin>,
}
impl Driver {
    fn launch(root: &Path, scenario: &str, owner: bool) -> Self {
        subreaper();
        let mut command = Command::new(env!("CARGO_BIN_EXE_infra-host-sandbox-fixture"));
        if owner {
            command.arg("process-owner").arg(root);
        } else {
            command.arg("process-case").arg(scenario).arg(root);
        }
        let child = command
            .env("INFRA_PROCESS_REMOVED", "inherited-value")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn native Rust process fixture");
        Self {
            child: Some(child),
            root: root.to_path_buf(),
            pins: Vec::new(),
        }
    }
    fn refresh(&mut self) -> std::io::Result<()> {
        for image in records(&self.root) {
            if !self.pins.iter().any(|pin| pin.matches(&image))
                && let Some(pin) = ProcessPin::open(&image)?
            {
                self.pins.push(pin);
            }
        }
        Ok(())
    }
    fn signal(&self, image: &Value, signal: i32) {
        self.pins
            .iter()
            .find(|pin| pin.matches(image))
            .expect("fixture identity must be pinned before signaling")
            .signal(signal)
            .expect("signal only the pinned fixture process");
    }
    fn wait(&mut self) -> Value {
        let until = Instant::now() + Duration::from_secs(15);
        while self
            .child
            .as_mut()
            .unwrap()
            .try_wait()
            .expect("poll native fixture")
            .is_none()
        {
            self.refresh().expect("retain native process identity pins");
            for pin in &self.pins {
                pin.reap().expect("reap only pinned adopted descendants");
            }
            assert!(
                Instant::now() < until,
                "native process fixture exceeded its hard test deadline"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        self.refresh()
            .expect("retain final native process identity pins");
        let output = self
            .child
            .take()
            .unwrap()
            .wait_with_output()
            .expect("reap native fixture");
        assert!(
            output.status.success(),
            "native fixture failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stdout.len() <= 32768,
            "native fixture receipt is unexpectedly large"
        );
        serde_json::from_slice(&output.stdout).expect("native API result receipt")
    }
    fn ready(&mut self) -> (Value, Value) {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            if let (Some(group), Some(guardian)) = (
                json_file(&self.root.join("group.json")),
                json_file(&self.root.join("guardian.json")),
            ) {
                self.refresh().expect("pin the ready fixture tree");
                for image in [&group["leader"], &group["descendant"], &guardian] {
                    assert!(
                        alive(image),
                        "fixture process exited before adversarial signal"
                    );
                    assert!(
                        self.pins.iter().any(|pin| pin.matches(image)),
                        "every adversarial fixture process requires its own pidfd"
                    );
                }
                return (group, guardian);
            }
            assert!(
                self.child
                    .as_mut()
                    .unwrap()
                    .try_wait()
                    .expect("poll process owner")
                    .is_none(),
                "owner exited before real descendant readiness"
            );
            assert!(
                Instant::now() < until,
                "native descendants did not publish actual PIDs"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn kill_owner(&mut self) {
        let mut child = self.child.take().unwrap();
        child.kill().expect("kill only native API owner");
        child.wait().expect("reap terminated native API owner");
    }
}
impl Drop for Driver {
    fn drop(&mut self) {
        let _ = self.refresh();
        for pin in &self.pins {
            let _ = pin.signal(libc::SIGCONT);
            let _ = pin.signal(libc::SIGKILL);
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            // If an assertion raced readiness, capture any image published
            // before owner death. No cleanup signal uses a numeric PID/PGID.
            let _ = self.refresh();
            for pin in &self.pins {
                let _ = pin.signal(libc::SIGCONT);
                let _ = pin.signal(libc::SIGKILL);
                let _ = pin.reap();
            }
            if records(&self.root).iter().all(|image| !alive(image)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
fn bytes(value: &Value) -> Vec<u8> {
    serde_json::from_value(value.clone()).expect("raw native bytes")
}
fn packets(value: &Value) -> Vec<u8> {
    value["packets"]
        .as_array()
        .expect("native callback packets")
        .iter()
        .flat_map(bytes)
        .collect()
}
fn run(name: &str) -> Value {
    let root = Root::new();
    let mut driver = Driver::launch(&root.0, name, false);
    let value = driver.wait();
    for image in records(&root.0) {
        stopped(&image);
    }
    value
}
fn code(value: &Value, expected: i64) {
    assert!(
        value["error"].is_null(),
        "native API refused unexpectedly: {value}"
    );
    assert_eq!(value["code"], expected);
}
fn group_stopped(value: &Value) {
    let group = &value["group"];
    assert!(
        group.is_object(),
        "native process tree never reached readiness"
    );
    assert_eq!(group["leader"]["group"], group["descendant"]["group"]);
    assert_eq!(
        value["guardian"]["group"], value["guardian"]["pid"],
        "the actual pinned guardian must anchor its process group"
    );
    assert_eq!(
        group["leader"]["group"], value["guardian"]["pid"],
        "the native tree must remain in its pinned guardian's group"
    );
    stopped(&group["leader"]);
    stopped(&group["descendant"]);
    stopped(&value["guardian"]);
}
#[test]
fn binary_stdin_argument_exit_code_and_line_callbacks_are_preserved() {
    let value = run("io");
    code(&value, 7);
    let mut expected = LITERAL.as_bytes().to_vec();
    expected.push(b'\n');
    expected.extend_from_slice(INPUT);
    assert_eq!(bytes(&value["bytes"]), expected);
    assert_eq!(packets(&value), expected);
    let mut first_packet = LITERAL.as_bytes().to_vec();
    first_packet.push(b'\n');
    let actual_packets = value["packets"]
        .as_array()
        .unwrap()
        .iter()
        .map(bytes)
        .collect::<Vec<_>>();
    assert_eq!(
        actual_packets,
        vec![
            first_packet,
            b"native stdin\r\n".to_vec(),
            b"second\n".to_vec(),
            b"\0\xfftail".to_vec(),
        ]
    );
    let signals = value["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|event| event["signal"].as_str())
        .collect::<Vec<_>>();
    for signal in [
        "guardian_spawned",
        "input_written",
        "input_closed",
        "exited",
        "cleanup_confirmed",
    ] {
        assert!(
            signals.contains(&signal),
            "missing actual process boundary: {signal}"
        );
    }
    assert_eq!(signals.last(), Some(&"cleanup_confirmed"));
}
#[test]
fn native_exit_125_is_preserved_after_confirmed_cleanup() {
    let value = run("exit-125");
    code(&value, 125);
    assert!(
        value["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| { event["signal"] == "cleanup_confirmed" }),
        "a native exit code must not collide with the cleanup protocol"
    );
}
#[test]
fn cancellation_from_final_unterminated_callback_is_not_lost_at_eof() {
    let value = run("tail-cancel");
    assert_eq!(packets(&value), b"FINAL");
    assert_eq!(value["error"], "native-cancelled");
    stopped(&value["leaf"]);
}
fn rejected_child_signal_policy(name: &str, handler: libc::sighandler_t, no_wait: bool) {
    let value = run(name);
    assert_eq!(value["error"], "unavailable");
    assert!(value["code"].is_null());
    assert!(
        value["guardian"].is_null(),
        "refusal must precede guardian creation"
    );
    assert!(
        value["leaf"].is_null(),
        "refusal must precede native leaf creation"
    );
    assert!(value["events"].as_array().unwrap().is_empty());
    assert!(packets(&value).is_empty());
    let policy = &value["signal_policy"];
    assert_eq!(
        policy["before"], policy["after"],
        "refusal must preserve the driver's signal policy"
    );
    assert_eq!(policy["after"]["handler"].as_u64(), Some(handler as u64));
    assert_eq!(
        policy["after"]["flags"].as_i64().unwrap() & i64::from(libc::SA_NOCLDWAIT) != 0,
        no_wait
    );
    let mask: Vec<i32> = serde_json::from_value(policy["after"]["mask"].clone()).unwrap();
    assert!(mask.contains(&libc::SIGUSR1) && mask.contains(&libc::SIGUSR2));
    assert_eq!(
        policy["lock_released"], true,
        "pre-launch refusal must release runner exclusion"
    );
}
#[test]
fn explicit_sigchld_ignore_refuses_before_native_launch_without_side_effects() {
    rejected_child_signal_policy("child-signal-ignore", libc::SIG_IGN, false);
}
#[test]
fn no_child_wait_refuses_before_native_launch_without_side_effects() {
    rejected_child_signal_policy("child-no-cld-wait", libc::SIG_DFL, true);
}
#[test]
fn raw_capture_merges_stderr_and_preserves_non_utf8_without_newline() {
    let value = run("raw");
    code(&value, 0);
    let expected = b"stdout\0\xffstderr\r\n";
    assert_eq!(bytes(&value["bytes"]), expected);
    assert_eq!(packets(&value), expected);
}
#[test]
fn raw_output_limit_delivers_only_the_bounded_prefix_before_error() {
    let value = run("output-cap");
    assert_eq!(value["error"], "output-limit");
    let expected = (0u8..=255).cycle().take(1024).collect::<Vec<_>>();
    assert_eq!(packets(&value), expected);
    stopped(&value["leaf"]);
}
#[test]
fn line_without_newline_is_bounded_and_native_leaf_is_stopped() {
    let value = run("line-cap");
    assert_eq!(value["error"], "output-line-limit");
    assert!(packets(&value).is_empty());
    stopped(&value["leaf"]);
}
#[test]
fn explicit_environment_removal_and_override_reach_the_native_leaf() {
    let value = run("environment");
    code(&value, 0);
    let environment: Value = serde_json::from_slice(&bytes(&value["bytes"])).unwrap();
    assert_eq!(environment["kept"], "override-value");
    assert!(environment["removed"].is_null());
}
#[test]
fn signal_death_preserves_shell_status() {
    let value = run("signal");
    code(&value, 128 + libc::SIGKILL as i64);
}
#[test]
fn native_inherits_only_stdio_and_runner_lock() {
    let value = run("descriptors");
    code(&value, 0);
    let image: Value = serde_json::from_slice(&bytes(&value["bytes"])).unwrap();
    let runner = image["runner"].as_str().unwrap().parse::<i32>().unwrap();
    let stray = image["stray"].as_str().unwrap().parse::<i32>().unwrap();
    let mut expected = vec![0, 1, 2, runner];
    expected.sort();
    expected.dedup();
    assert_eq!(
        serde_json::from_value::<Vec<i32>>(image["open"].clone()).unwrap(),
        expected
    );
    assert!(!expected.contains(&stray));
}
#[test]
fn normal_leader_exit_stops_stubborn_descendant_before_return() {
    let value = run("leader-exit");
    code(&value, 0);
    assert_eq!(packets(&value), b"READY\n");
    group_stopped(&value);
}
#[test]
fn cancellation_after_actual_readiness_stops_the_whole_group() {
    let value = run("cancel");
    assert_eq!(value["error"], "native-cancelled");
    assert_eq!(packets(&value), b"READY\n");
    group_stopped(&value);
}
#[test]
fn absolute_deadline_stops_an_actual_native_process_group() {
    let value = run("deadline");
    assert_eq!(value["error"], "native-timeout");
    assert_eq!(packets(&value), b"READY\n");
    assert!(
        value["seconds"].as_f64().unwrap() < 8.0,
        "deadline cleanup exceeded the fixture hard budget"
    );
    group_stopped(&value);
}
#[test]
fn callback_failure_keeps_its_error_after_confirmed_group_cleanup() {
    let value = run("callback-error");
    assert_eq!(value["error"], "fixture-callback");
    assert_eq!(packets(&value), b"READY\n");
    group_stopped(&value);
}
fn guardian_loss(name: &str, signal: i32, cancel: bool) {
    let root = Root::new();
    let mut driver = Driver::launch(&root.0, name, false);
    let (group, guardian) = driver.ready();
    let lock = root.0.join("runner.lock");
    assert!(
        open_lock(&lock).try_lock().is_err(),
        "a live native tree must retain runner exclusion"
    );
    driver.signal(&guardian, signal);
    let until = Instant::now() + Duration::from_secs(2);
    while alive(&guardian) {
        assert!(
            Instant::now() < until,
            "guardian did not settle the actual signal"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    if cancel {
        fs::write(root.0.join("request-cancel"), b"cancel")
            .expect("request cancellation only after guardian signal");
    }
    let value = driver.wait();
    assert_eq!(
        value["group"], group,
        "receipt must describe the ready fixture tree"
    );
    assert_eq!(value["guardian"], guardian);
    // Assert before Driver's RAII fallback so fixture cleanup cannot conceal
    // any native processes leaked by the supervisor.
    group_stopped(&value);
    if name == "protocol-guardian-kill" {
        assert!(
            packets(&value).is_empty(),
            "raw protocol bytes are not run callbacks"
        );
    } else {
        assert_eq!(packets(&value), b"READY\n");
    }
    assert!(
        value["error"].is_string(),
        "guardian loss must not become a successful native result: {value}"
    );
    assert!(
        value["seconds"].as_f64().unwrap() < 12.0,
        "guardian-loss cleanup exceeded its hard fixture budget"
    );
    let replacement = open_lock(&lock);
    replacement
        .try_lock()
        .expect("replacement admission follows confirmed native cleanup");
}
#[test]
fn guardian_sigterm_at_timeout_cannot_leave_a_live_native_tree() {
    guardian_loss("guardian-term-timeout", libc::SIGTERM, false);
}
#[test]
fn guardian_sigkill_at_timeout_cannot_leave_a_live_native_tree() {
    guardian_loss("guardian-kill-timeout", libc::SIGKILL, false);
}
#[test]
fn guardian_sigterm_before_cancellation_cannot_leave_a_live_native_tree() {
    guardian_loss("guardian-term-cancel", libc::SIGTERM, true);
}
#[test]
fn guardian_sigkill_before_cancellation_cannot_leave_a_live_native_tree() {
    guardian_loss("guardian-kill-cancel", libc::SIGKILL, true);
}
#[test]
fn protocol_completion_stops_the_actual_native_server() {
    let value = run("protocol-done");
    code(&value, 0);
    let reply: Value = serde_json::from_slice(&bytes(&value["bytes"])).unwrap();
    assert_eq!(reply["ok"], true);
    stopped(&value["leaf"]);
}
#[test]
fn cancellation_interrupts_an_actually_backpressured_protocol_write() {
    let value = run("protocol-write-cancel");
    let evidence = &value["backpressure"];
    let capacity = evidence["capacity"]
        .as_u64()
        .expect("actual native stdin capacity");
    assert!(capacity > 0);
    assert_eq!(evidence["full"]["capacity"].as_u64(), Some(capacity));
    assert_eq!(evidence["full"]["unread"].as_u64(), Some(capacity));
    let packet = evidence["packet_bytes"].as_u64().unwrap();
    assert!(
        packet > capacity,
        "the actual protocol write must exceed the full pipe"
    );
    assert!(packet + evidence["initial_bytes"].as_u64().unwrap() <= 65536);
    assert_eq!(evidence["cancel_requested"], true);
    assert_eq!(value["error"], "native-cancelled", "{value}");
    assert!(
        evidence["after_cancel_seconds"].as_f64().unwrap() < 1.5,
        "write cancellation must not wait for the three-second protocol deadline: {value}"
    );
    assert_eq!(evidence["lock_released"], true);
    stopped(&value["leaf"]);
    stopped(&value["guardian"]);
}
#[test]
fn protocol_guardian_loss_cannot_leave_a_live_native_tree() {
    guardian_loss("protocol-guardian-kill", libc::SIGKILL, false);
}
fn open_lock(path: &Path) -> File {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("open actual runner lock")
}
#[test]
fn owner_death_retains_runner_exclusion_until_all_native_descendants_stop() {
    let root = Root::new();
    let mut owner = Driver::launch(&root.0, "owner", true);
    let (group, guardian) = owner.ready();
    for image in [&group["leader"], &group["descendant"], &guardian] {
        assert!(
            alive(image),
            "all real native processes must be alive before owner loss"
        );
    }
    let lock = root.0.join("runner.lock");
    assert!(
        open_lock(&lock).try_lock().is_err(),
        "live owner must exclude replacement"
    );
    owner.signal(&guardian, libc::SIGSTOP);
    let until = Instant::now() + Duration::from_secs(2);
    while state(&guardian) != Some('T') {
        assert!(
            Instant::now() < until,
            "guardian did not stop at kernel boundary"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    owner.kill_owner();
    for image in [&group["leader"], &group["descendant"]] {
        assert!(
            alive(image),
            "paused guardian must not yet have cleaned the native group"
        );
    }
    assert!(
        open_lock(&lock).try_lock().is_err(),
        "inherited custody must retain exclusion after owner death"
    );
    owner.signal(&guardian, libc::SIGCONT);
    let until = Instant::now() + Duration::from_secs(8);
    let replacement = loop {
        reap(&group["descendant"]);
        reap(&guardian);
        let replacement = open_lock(&lock);
        if replacement.try_lock().is_ok() {
            break replacement;
        }
        assert!(
            Instant::now() < until,
            "runner exclusion was not released after native cleanup"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    stopped(&group["leader"]);
    stopped(&group["descendant"]);
    stopped(&guardian);
    drop(replacement);
}
