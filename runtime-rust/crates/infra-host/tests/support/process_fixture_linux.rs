//! Native Linux process leaves and API driver; no shell, Python or provider.
use infra_host::{
    HostError, Result,
    process::{self, Limits, Output, RunObserver},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const INPUT: &[u8] = b"native stdin\r\nsecond\n\0\xfftail";
const LITERAL: &str = "literal $(false); --value=$HOME";
fn fail(message: impl Into<String>) -> HostError {
    HostError::unavailable(message)
}
fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(fail(message))
    }
}
fn path(args: &[String], at: usize) -> Result<PathBuf> {
    args.get(at)
        .map(PathBuf::from)
        .ok_or_else(|| fail("native process fixture path missing"))
}
fn argv(action: &str, args: &[String]) -> Result<Vec<String>> {
    let exe = std::env::current_exe()?
        .into_os_string()
        .into_string()
        .map_err(|_| fail("native process fixture executable path must be UTF-8"))?;
    let mut out = vec![exe, "process-leaf".into(), action.into()];
    out.extend_from_slice(args);
    Ok(out)
}
fn subreaper() -> Result<()> {
    if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
fn pause() -> ! {
    loop {
        unsafe { libc::pause() };
    }
}
fn ignore_term() -> Result<()> {
    if unsafe { libc::signal(libc::SIGTERM, libc::SIG_IGN) } == libc::SIG_ERR {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
fn child_signal_policy() -> Result<Value> {
    let mut action = std::mem::MaybeUninit::<libc::sigaction>::uninit();
    if unsafe { libc::sigaction(libc::SIGCHLD, std::ptr::null(), action.as_mut_ptr()) } < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let action = unsafe { action.assume_init() };
    let mut mask = Vec::new();
    for signal in 1..=libc::SIGRTMAX() {
        match unsafe { libc::sigismember(&action.sa_mask, signal) } {
            0 => {}
            1 => mask.push(signal),
            _ => return Err(std::io::Error::last_os_error().into()),
        }
    }
    Ok(json!({"handler":action.sa_sigaction as u64,"flags":action.sa_flags,"mask":mask}))
}
fn set_child_signal_policy(ignore: bool) -> Result<()> {
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = if ignore { libc::SIG_IGN } else { libc::SIG_DFL };
    action.sa_flags = if ignore {
        libc::SA_RESTART
    } else {
        libc::SA_RESTART | libc::SA_NOCLDWAIT
    };
    if unsafe { libc::sigemptyset(&mut action.sa_mask) } < 0
        || unsafe { libc::sigaddset(&mut action.sa_mask, libc::SIGUSR1) } < 0
        || unsafe { libc::sigaddset(&mut action.sa_mask, libc::SIGUSR2) } < 0
        || unsafe { libc::sigaction(libc::SIGCHLD, &action, std::ptr::null_mut()) } < 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
fn process_image(pid: u32) -> Result<Value> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let (_, fields) = stat
        .rsplit_once(") ")
        .ok_or_else(|| fail("native process stat has no identity"))?;
    let fields = fields.split_whitespace().collect::<Vec<_>>();
    let group = fields
        .get(2)
        .and_then(|value| value.parse::<i32>().ok())
        .ok_or_else(|| fail("native process group is unavailable"))?;
    let birth = fields
        .get(19)
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| fail("native process birth identity is unavailable"))?;
    Ok(json!({"pid":pid,"group":group,"birth":birth}))
}
fn mark(path: &Path, value: &Value) -> Result<()> {
    fs::write(path, serde_json::to_vec(value)?)?;
    Ok(())
}
fn lock(root: &Path) -> Result<Arc<File>> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(root.join("runner.lock"))?;
    file.try_lock()
        .map_err(|_| fail("fixture runner lock is already held"))?;
    Ok(Arc::new(file))
}
#[derive(Default)]
struct Observation {
    events: Mutex<Vec<Value>>,
    root: PathBuf,
}
impl RunObserver for Observation {
    fn boundary(&self, signal: &str, details: &Value) -> Result<()> {
        self.events
            .lock()
            .map_err(|_| fail("fixture observations are unavailable"))?
            .push(json!({"signal":signal,"details":details}));
        if signal == "guardian_spawned" {
            let pid = details["guardian_pid"]
                .as_u64()
                .ok_or_else(|| fail("actual guardian PID missing"))? as u32;
            mark(&self.root.join("guardian.json"), &process_image(pid)?)?;
        }
        Ok(())
    }
}
struct Reaper {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Reaper {
    fn new(root: &Path) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let marker = root.join("group.json");
        let thread = std::thread::spawn(move || {
            while !stopping.load(Ordering::Acquire) {
                let pid = fs::read(&marker)
                    .ok()
                    .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
                    .and_then(|v| v["descendant"]["pid"].as_u64());
                if let Some(pid) = pid {
                    let mut status = 0;
                    // Reap only this fixture's adopted orphan, never Tokio's
                    // directly owned guardian. Containers need not have an init
                    // that promptly reaps orphaned test descendants.
                    if unsafe { libc::waitpid(pid as i32, &mut status, libc::WNOHANG) }
                        == pid as i32
                    {
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Reaper {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn response(
    result: Result<Output>,
    packets: Vec<Vec<u8>>,
    observation: &Observation,
    root: &Path,
    seconds: f64,
) -> Result<()> {
    let mut value = match result {
        Ok(output) => {
            json!({"code":output.code,"error":null,"bytes":output.bytes,"seconds":output.seconds})
        }
        Err(error) => json!({"code":null,"error":error.code,"bytes":[],"seconds":seconds}),
    };
    value["packets"] = json!(packets);
    value["events"] = json!(
        observation
            .events
            .lock()
            .map_err(|_| fail("fixture observations unavailable"))?
            .clone()
    );
    for (name, file) in [
        ("group", "group.json"),
        ("guardian", "guardian.json"),
        ("leaf", "leaf.json"),
        ("signal_policy", "signal-policy.json"),
        ("backpressure", "backpressure-report.json"),
    ] {
        if let Ok(raw) = fs::read(root.join(file)) {
            value[name] = serde_json::from_slice(&raw)?;
        }
    }
    std::io::stdout().write_all(&serde_json::to_vec(&value)?)?;
    std::io::stdout().write_all(b"\n")?;
    std::io::stdout().flush()?;
    Ok(())
}
fn protocol_output(result: Result<Value>, started: Instant) -> Result<Output> {
    Ok(Output {
        code: 0,
        bytes: serde_json::to_vec(&result?)?,
        seconds: started.elapsed().as_secs_f64(),
    })
}
async fn protocol_backpressure(
    root: &Path,
    cancel: Arc<AtomicBool>,
    started: Instant,
) -> Result<Output> {
    let runner = lock(root)?;
    let initial = json!({"initialize":true});
    let initial_bytes = serde_json::to_vec(&initial)?.len() + 1;
    let marker = root.join("backpressure-full.json");
    let cancellation = cancel.clone();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let relay = tokio::spawn(async move {
        loop {
            let full = fs::read(&marker)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .filter(|value| {
                    value["capacity"].as_u64().is_some_and(|capacity| {
                        capacity > 0 && value["unread"].as_u64() == Some(capacity)
                    })
                });
            if let Some(full) = full {
                let _ = sender.send((full, Instant::now()));
                cancellation.store(true, Ordering::Release);
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    });
    let command = argv("backpressure", &[root.to_string_lossy().into_owned()])?;
    let mut capacity = 0usize;
    let mut packet_bytes = 0usize;
    let result = process::converse_environment(
        &command,
        root,
        &BTreeMap::new(),
        &[],
        &[initial],
        Limits {
            seconds: Some(3),
            output: 8192,
            line: 4096,
        },
        cancel.clone(),
        Some(runner),
        |message| {
            require(
                message["ready"] == true,
                "native server must first drain initialization",
            )?;
            capacity = message["capacity"]
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| *value > 0 && *value < 65500)
                .ok_or_else(|| {
                    fail("native stdin capacity cannot fit a bounded backpressure packet")
                })?;
            let packet = json!({"payload":"X".repeat(capacity)});
            packet_bytes = serde_json::to_vec(&packet)?.len() + 1;
            require(
                packet_bytes > capacity && initial_bytes + packet_bytes <= 65536,
                "the valid bounded packet must exceed actual stdin capacity",
            )?;
            Ok(process::Control::Write(vec![packet]))
        },
    )
    .await;
    relay.abort();
    let _ = relay.await;
    let full = receiver.try_recv().ok();
    let after_cancel = full.as_ref().map(|(_, at)| at.elapsed().as_secs_f64());
    let replacement = OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.join("runner.lock"))?;
    let lock_released = replacement.try_lock().is_ok();
    mark(
        &root.join("backpressure-report.json"),
        &json!({
            "capacity":capacity,"packet_bytes":packet_bytes,"initial_bytes":initial_bytes,
            "full":full.map(|(value, _)| value),"cancel_requested":cancel.load(Ordering::Acquire),
            "after_cancel_seconds":after_cancel,"lock_released":lock_released
        }),
    )?;
    protocol_output(result, started)
}
async fn case(name: &str, root: &Path) -> Result<()> {
    let observation = Arc::new(Observation {
        root: root.to_path_buf(),
        ..Default::default()
    });
    let observer: Arc<dyn RunObserver> = observation.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut packets = Vec::new();
    let started = Instant::now();
    let mut env = BTreeMap::new();
    let limits = Limits {
        seconds: Some(5),
        output: 8192,
        line: 4096,
    };
    let result = match name {
        "protocol-write-cancel" => protocol_backpressure(root, cancel, started).await,
        "child-signal-ignore" | "child-no-cld-wait" => {
            // This changes only the disposable driver's signal disposition.
            // A nonempty action mask checks that refusal does not rewrite it.
            set_child_signal_policy(name == "child-signal-ignore")?;
            let before = child_signal_policy()?;
            let runner = lock(root)?;
            let replacement = OpenOptions::new()
                .read(true)
                .write(true)
                .open(root.join("runner.lock"))?;
            require(
                replacement.try_lock().is_err(),
                "fixture runner must begin exclusively owned",
            )?;
            let command = argv(
                "tail",
                &[root.join("leaf.json").to_string_lossy().into_owned()],
            )?;
            let result = process::run_monitored(
                &command,
                root,
                &env,
                &[],
                b"",
                limits,
                cancel,
                Some(runner),
                Some(observer),
                |packet| {
                    packets.push(packet.to_vec());
                    Ok(())
                },
            )
            .await;
            let after = child_signal_policy()?;
            let lock_released = replacement.try_lock().is_ok();
            mark(
                &root.join("signal-policy.json"),
                &json!({
                    "before":before,"after":after,"lock_released":lock_released
                }),
            )?;
            result
        }
        "protocol-done" => {
            let command = argv(
                "protocol",
                &[root.join("leaf.json").to_string_lossy().into_owned()],
            )?;
            protocol_output(
                process::converse_environment(
                    &command,
                    root,
                    &env,
                    &[],
                    &[],
                    limits,
                    cancel,
                    None,
                    |message| Ok(process::Control::Done(message)),
                )
                .await,
                started,
            )
        }
        "io" | "exit-125" => {
            let code = if name == "exit-125" { "125" } else { "7" };
            let command = argv("io", &[LITERAL.into(), code.into()])?;
            process::run_monitored(
                &command,
                root,
                &env,
                &[],
                INPUT,
                limits,
                cancel,
                None,
                Some(observer),
                |packet| {
                    packets.push(packet.to_vec());
                    Ok(())
                },
            )
            .await
        }
        "tail-cancel" => {
            let command = argv(
                "tail",
                &[root.join("leaf.json").to_string_lossy().into_owned()],
            )?;
            let cancellation = cancel.clone();
            process::run_monitored(
                &command,
                root,
                &env,
                &[],
                b"",
                limits,
                cancel,
                None,
                Some(observer),
                |packet| {
                    packets.push(packet.to_vec());
                    if packet == b"FINAL" {
                        cancellation.store(true, Ordering::Release);
                    }
                    Ok(())
                },
            )
            .await
        }
        "raw" => {
            let command = argv("raw", &[])?;
            process::run_raw_with_lock(&command, root, &env, b"", limits, cancel, None, |packet| {
                packets.push(packet.to_vec());
                Ok(())
            })
            .await
        }
        "output-cap" => {
            let command = argv(
                "flood",
                &[root.join("leaf.json").to_string_lossy().into_owned()],
            )?;
            process::run_raw_with_lock(
                &command,
                root,
                &env,
                b"",
                Limits {
                    seconds: Some(5),
                    output: 1024,
                    line: 8192,
                },
                cancel,
                None,
                |packet| {
                    packets.push(packet.to_vec());
                    Ok(())
                },
            )
            .await
        }
        "line-cap" => {
            let command = argv(
                "line",
                &[root.join("leaf.json").to_string_lossy().into_owned()],
            )?;
            process::run_with_lock(
                &command,
                root,
                &env,
                b"",
                Limits {
                    seconds: Some(5),
                    output: 8192,
                    line: 64,
                },
                cancel,
                None,
                |packet| {
                    packets.push(packet.to_vec());
                    Ok(())
                },
            )
            .await
        }
        "signal" => {
            let command = argv("signal", &[])?;
            process::run_with_lock(&command, root, &env, b"", limits, cancel, None, |packet| {
                packets.push(packet.to_vec());
                Ok(())
            })
            .await
        }
        "environment" => {
            env.insert("INFRA_PROCESS_KEPT".into(), "override-value".into());
            let command = argv("environment", &[])?;
            process::run_environment(
                &command,
                root,
                &env,
                &["INFRA_PROCESS_REMOVED".into()],
                b"",
                limits,
                cancel,
                None,
                |packet| {
                    packets.push(packet.to_vec());
                    Ok(())
                },
            )
            .await
        }
        "descriptors" => {
            let runner = lock(root)?;
            let runner_fd = runner.as_raw_fd();
            let stray = File::open("/dev/null")?;
            if unsafe { libc::fcntl(stray.as_raw_fd(), libc::F_SETFD, 0) } < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let command = argv(
                "descriptors",
                &[runner_fd.to_string(), stray.as_raw_fd().to_string()],
            )?;
            let result = process::run_with_lock(
                &command,
                root,
                &env,
                b"",
                limits,
                cancel,
                Some(runner),
                |packet| {
                    packets.push(packet.to_vec());
                    Ok(())
                },
            )
            .await;
            drop(stray);
            result
        }
        "cancel" | "deadline" | "leader-exit" | "callback-error" => {
            let _reaper = Reaper::new(root);
            let command = argv(
                "tree",
                &[
                    root.join("group.json").to_string_lossy().into_owned(),
                    if name == "leader-exit" {
                        "exit".into()
                    } else {
                        "stay".into()
                    },
                ],
            )?;
            let cancellation = cancel.clone();
            let callback = |packet: &[u8]| -> Result<()> {
                packets.push(packet.to_vec());
                if packet == b"READY\n" {
                    if name == "cancel" {
                        cancellation.store(true, Ordering::Release)
                    }
                    if name == "callback-error" {
                        return Err(HostError {
                            status: 409,
                            code: "fixture-callback",
                            message: "Fixture callback refusal.".into(),
                        });
                    }
                }
                Ok(())
            };
            if name == "deadline" {
                process::run_monitored_until(
                    &command,
                    root,
                    &env,
                    &[],
                    b"",
                    Limits {
                        seconds: None,
                        output: 8192,
                        line: 4096,
                    },
                    cancel,
                    None,
                    Some(observer),
                    Instant::now() + Duration::from_millis(1200),
                    callback,
                )
                .await
            } else {
                process::run_monitored(
                    &command,
                    root,
                    &env,
                    &[],
                    b"",
                    limits,
                    cancel,
                    None,
                    Some(observer),
                    callback,
                )
                .await
            }
        }
        "guardian-term-timeout"
        | "guardian-kill-timeout"
        | "guardian-term-cancel"
        | "guardian-kill-cancel"
        | "protocol-guardian-kill" => {
            let _reaper = Reaper::new(root);
            let runner = lock(root)?;
            let command = argv(
                "tree",
                &[
                    root.join("group.json").to_string_lossy().into_owned(),
                    "stay".into(),
                ],
            )?;
            let marker = root.join("request-cancel");
            let cancellation = cancel.clone();
            let relay = tokio::spawn(async move {
                while !marker.exists() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                cancellation.store(true, Ordering::Release);
            });
            let result = if name == "protocol-guardian-kill" {
                protocol_output(
                    process::converse_environment(
                        &command,
                        root,
                        &env,
                        &[],
                        &[],
                        limits,
                        cancel,
                        Some(runner),
                        |_| Ok(process::Control::Continue),
                    )
                    .await,
                    started,
                )
            } else {
                process::run_monitored_until(
                    &command,
                    root,
                    &env,
                    &[],
                    b"",
                    Limits {
                        seconds: None,
                        output: 8192,
                        line: 4096,
                    },
                    cancel,
                    Some(runner),
                    Some(observer),
                    Instant::now() + Duration::from_secs(4),
                    |packet| {
                        packets.push(packet.to_vec());
                        Ok(())
                    },
                )
                .await
            };
            relay.abort();
            let _ = relay.await;
            result
        }
        _ => return Err(fail("unknown native process fixture case")),
    };
    response(
        result,
        packets,
        &observation,
        root,
        started.elapsed().as_secs_f64(),
    )
}
async fn owner(root: &Path) -> Result<()> {
    let runner = lock(root)?;
    let observation: Arc<dyn RunObserver> = Arc::new(Observation {
        root: root.to_path_buf(),
        ..Default::default()
    });
    let command = argv(
        "tree",
        &[
            root.join("group.json").to_string_lossy().into_owned(),
            "stay".into(),
        ],
    )?;
    let output = process::run_monitored(
        &command,
        root,
        &BTreeMap::new(),
        &[],
        b"",
        Limits {
            seconds: Some(60),
            output: 8192,
            line: 4096,
        },
        Arc::new(AtomicBool::new(false)),
        Some(runner),
        Some(observation),
        |_| Ok(()),
    )
    .await?;
    Err(fail(format!(
        "native owner returned before owner-loss test: {}",
        output.code
    )))
}
fn leaf(args: &[String]) -> Result<i32> {
    let action = args
        .first()
        .ok_or_else(|| fail("native leaf action missing"))?;
    match action.as_str() {
        "io" => {
            let literal = args
                .get(1)
                .ok_or_else(|| fail("literal native argument missing"))?;
            std::io::stdout().write_all(literal.as_bytes())?;
            std::io::stdout().write_all(b"\n")?;
            let mut input = Vec::new();
            std::io::stdin().take(65536).read_to_end(&mut input)?;
            std::io::stdout().write_all(&input)?;
            std::io::stdout().flush()?;
            std::io::stderr().write_all(b"private-stderr-marker\n")?;
            std::io::stderr().flush()?;
            return args
                .get(2)
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| fail("native exit code missing"));
        }
        "raw" => {
            std::io::stdout().write_all(b"stdout\0\xff")?;
            std::io::stdout().flush()?;
            std::io::stderr().write_all(b"stderr\r\n")?;
            std::io::stderr().flush()?;
        }
        "tail" => {
            mark(&path(args, 1)?, &process_image(std::process::id())?)?;
            std::io::stdout().write_all(b"FINAL")?;
            std::io::stdout().flush()?;
        }
        "protocol" => {
            ignore_term()?;
            mark(&path(args, 1)?, &process_image(std::process::id())?)?;
            std::io::stdout().write_all(b"{\"ok\":true}\n")?;
            std::io::stdout().flush()?;
            pause()
        }
        "backpressure" => {
            ignore_term()?;
            let root = path(args, 1)?;
            mark(&root.join("leaf.json"), &process_image(std::process::id())?)?;
            mark(
                &root.join("guardian.json"),
                &process_image(unsafe { libc::getppid() } as u32)?,
            )?;
            // Drain exactly the small first line before shrinking this pipe.
            let mut initial = Vec::new();
            loop {
                let mut byte = [0u8; 1];
                std::io::stdin().read_exact(&mut byte)?;
                initial.push(byte[0]);
                require(
                    initial.len() <= 1024,
                    "initial native request must be small",
                )?;
                if byte[0] == b'\n' {
                    break;
                }
            }
            let initialize: Value = serde_json::from_slice(&initial)?;
            require(
                initialize["initialize"] == true,
                "native initialization request missing",
            )?;
            let resized = unsafe { libc::fcntl(0, libc::F_SETPIPE_SZ, 1) };
            if resized < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let capacity = unsafe { libc::fcntl(0, libc::F_GETPIPE_SZ) };
            require(
                capacity > 0 && capacity == resized,
                "native pipe capacity was not established",
            )?;
            println!("{}", json!({"ready":true,"capacity":capacity}));
            std::io::stdout().flush()?;
            loop {
                let mut unread = 0i32;
                if unsafe { libc::ioctl(0, libc::FIONREAD, &mut unread) } < 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                require(
                    unread <= capacity,
                    "native unread count exceeds pipe capacity",
                )?;
                if unread == capacity {
                    mark(
                        &root.join("backpressure-full.json"),
                        &json!({"capacity":capacity,"unread":unread}),
                    )?;
                    pause()
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        "flood" | "line" => {
            mark(&path(args, 1)?, &process_image(std::process::id())?)?;
            if action == "line" {
                std::io::stdout().write_all(&[b'Q'; 4096])?;
                std::io::stdout().flush()?;
            } else {
                let pattern = (0u8..=255).collect::<Vec<_>>();
                for _ in 0..4096 {
                    std::io::stdout().write_all(&pattern)?;
                }
                std::io::stdout().flush()?;
            }
        }
        "signal" => {
            unsafe { libc::kill(libc::getpid(), libc::SIGKILL) };
            return Err(fail("native signal did not terminate process"));
        }
        "environment" => {
            println!(
                "{}",
                serde_json::to_string(
                    &json!({"kept":std::env::var("INFRA_PROCESS_KEPT").ok(),"removed":std::env::var("INFRA_PROCESS_REMOVED").ok()})
                )?
            );
        }
        "descriptors" => {
            let candidates = fs::read_dir("/proc/self/fd")?
                .map(|entry| {
                    entry.map(|entry| entry.file_name().to_string_lossy().parse::<i32>().ok())
                })
                .collect::<std::io::Result<Vec<_>>>()?;
            let mut open = candidates
                .into_iter()
                .flatten()
                .filter(|fd| (unsafe { libc::fcntl(*fd, libc::F_GETFD) }) >= 0)
                .collect::<Vec<_>>();
            open.sort();
            println!(
                "{}",
                serde_json::to_string(
                    &json!({"open":open,"runner":args.get(1),"stray":args.get(2)})
                )?
            );
        }
        "stubborn" => {
            ignore_term()?;
            std::io::stdout().write_all(b"READY\n")?;
            std::io::stdout().flush()?;
            pause()
        }
        "tree" => {
            ignore_term()?;
            let command = argv("stubborn", &[])?;
            let mut child = Command::new(&command[0])
                .args(&command[1..])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()?;
            let mut ready = [0u8; 6];
            child
                .stdout
                .take()
                .ok_or_else(|| fail("native descendant readiness pipe missing"))?
                .read_exact(&mut ready)?;
            require(
                &ready == b"READY\n",
                "native descendant did not publish readiness",
            )?;
            let leader = process_image(std::process::id())?;
            let descendant = process_image(child.id())?;
            // converse has no public observer argument. The native leaf can
            // still publish its actual direct guardian parent for this test.
            let marker = path(args, 1)?;
            let guardian = marker
                .parent()
                .ok_or_else(|| fail("fixture root missing"))?
                .join("guardian.json");
            if !guardian.exists() {
                mark(
                    &guardian,
                    &process_image(unsafe { libc::getppid() } as u32)?,
                )?;
            }
            require(
                leader["group"] == descendant["group"],
                "native descendant escaped its owned group",
            )?;
            mark(
                &path(args, 1)?,
                &json!({"leader":leader,"descendant":descendant}),
            )?;
            std::io::stdout().write_all(b"READY\n")?;
            std::io::stdout().flush()?;
            if args.get(2).map(String::as_str) == Some("exit") {
                return Ok(0);
            }
            pause()
        }
        _ => return Err(fail("unknown native process leaf")),
    }
    Ok(0)
}
pub fn entry() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let (action, rest) = args
        .split_first()
        .ok_or_else(|| fail("native process fixture action missing"))?;
    match action.as_str() {
        "internal-guard" => std::process::exit(process::guard_entry(rest)?),
        "process-leaf" => std::process::exit(leaf(rest)?),
        "process-case" | "process-owner" => {}
        _ => return Err(fail("unknown native Linux fixture entrypoint")),
    }
    subreaper()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    if action == "process-owner" {
        return runtime.block_on(owner(&path(rest, 0)?));
    }
    let name = rest
        .first()
        .ok_or_else(|| fail("native process case missing"))?;
    runtime.block_on(case(name, &path(rest, 1)?))
}
