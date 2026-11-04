//! Noninstalled native driver. It dispatches the real Host guardian before any
//! runtime or signal scope, and never invokes a provider.
use infra_eval::{Result, RuntimeError, artifact, graph, process, reconstruction};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};
fn invalid(message: &str) -> RuntimeError {
    RuntimeError::invalid(message)
}
fn failure(error: &RuntimeError) -> Value {
    json!({"ok":false,"error":{"message":error.message,"kind":format!("{:?}",error.kind),"code":error.code,"os_code":error.os_code}})
}
fn flag(arguments: &[String], name: &str) -> Result<Option<PathBuf>> {
    let mut found = None;
    let mut at = 0;
    while at < arguments.len() {
        match arguments[at].as_str() {
            "--build" => at += 1,
            "--base" | "--cache" | "--runtime" => {
                let key = &arguments[at];
                let value = arguments
                    .get(at + 1)
                    .ok_or_else(|| invalid("Missing fixture option value"))?;
                if key == name {
                    if found.is_some() {
                        return Err(invalid("Duplicate fixture option"));
                    }
                    found = Some(PathBuf::from(value));
                }
                at += 2;
            }
            _ => return Err(invalid("Unknown fixture option")),
        }
    }
    Ok(found)
}
fn graph_scenario(mode: &str, arguments: &[String]) -> Result<Value> {
    let base = flag(arguments, "--base")?.ok_or_else(|| invalid("Missing fixture base"))?;
    let runtime = flag(arguments, "--runtime")?;
    if mode == "graph-baseline" {
        let selected = graph::native_baseline(
            &base,
            runtime.as_deref().and_then(Path::to_str),
            arguments.iter().any(|v| v == "--build"),
        )?;
        return Ok(json!({"ok":true,"path":selected}));
    }
    let runtime = runtime.ok_or_else(|| invalid("Missing fixture runtime"))?;
    let cache = flag(arguments, "--cache")?.ok_or_else(|| invalid("Missing fixture cache"))?;
    let r = reconstruction::Reconstructor::new(&base, &cache, &runtime)?;
    let (code, stdout, seconds) = r.runtime_at(
        &base,
        &["index".into(), "--json".into()],
        &reconstruction::clean_env(),
        Duration::from_secs(5),
        Some(reconstruction::instant("2026-01-02T03:04:05Z").unwrap()),
    )?;
    Ok(
        json!({"ok":true,"code":code,"stdout":String::from_utf8(stdout).map_err(|_|invalid("Fixture runtime output is not UTF-8"))?,"seconds":seconds}),
    )
}
#[cfg(target_os = "linux")]
fn image(pid: u32, role: &str) -> Value {
    let text = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let tail = text
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    json!({"pid":pid,"ppid":tail[1].parse::<u32>().unwrap(),"pgid":tail[2].parse::<u32>().unwrap(),"start":tail[19].parse::<u64>().unwrap(),"role":role})
}
#[cfg(not(target_os = "linux"))]
fn image(pid: u32, role: &str) -> Value {
    json!({"pid":pid,"role":role})
}
fn record(root: &Path, role: &str) {
    fs::create_dir_all(root.join("receipts")).unwrap();
    let pid = std::process::id();
    fs::write(
        root.join("receipts").join(format!("{pid}-{role}.json")),
        serde_json::to_vec(&image(pid, role)).unwrap(),
    )
    .unwrap();
    #[cfg(target_os = "linux")]
    if role == "leaf" {
        let own = image(pid, role);
        let ppid = own["ppid"].as_u64().unwrap() as u32;
        fs::write(
            root.join("guardian.json"),
            serde_json::to_vec(&image(ppid, "guardian")).unwrap(),
        )
        .unwrap();
        let group = own["pgid"].as_u64().unwrap() as u32;
        if group != ppid {
            fs::write(
                root.join("anchor.json"),
                serde_json::to_vec(&image(group, "anchor")).unwrap(),
            )
            .unwrap();
        }
    }
}
fn ready(root: &Path) {
    fs::write(root.join("ready"), b"native-ready").unwrap();
}
fn descendant(root: &Path, scenario: &str) {
    record(root, "descendant");
    if scenario == "experiment-input" {
        #[cfg(unix)]
        unsafe {
            libc::close(1);
            libc::close(2);
        }
        std::thread::sleep(Duration::from_millis(180));
        fs::write(
            root.join("experiment-input-started"),
            b"reading-after-delayed-backpressure",
        )
        .unwrap();
        let mut input = Vec::new();
        std::io::stdin().read_to_end(&mut input).unwrap();
        return;
    }
    if matches!(
        scenario,
        "experiment-term" | "experiment-leader-exited" | "experiment-adapter-term"
    ) {
        std::thread::sleep(Duration::from_millis(250));
        fs::write(root.join("experiment-late"), b"descendant-completed").unwrap();
        std::io::stdout()
            .write_all(b"late-experiment-output")
            .unwrap();
        return;
    }
    if scenario == "late-event" {
        std::thread::sleep(Duration::from_millis(200));
        println!(
            "{}",
            json!({"type":"item.completed","item":{"type":"agent_message","text":"late descendant answer"}})
        );
        println!(
            "{}",
            json!({"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":2}})
        );
        std::io::stdout().flush().unwrap();
        return;
    }
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGTERM, libc::SIG_IGN);
    }
    loop {
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn leaf(root: &Path, scenario: &str) -> Result<i32> {
    record(root, "leaf");
    if scenario.starts_with("experiment-") {
        return experiment_leaf(root, scenario);
    }
    match scenario {
        "signal-int" | "signal-kill" => {
            #[cfg(unix)]
            unsafe {
                libc::raise(if scenario == "signal-int" {
                    libc::SIGINT
                } else {
                    libc::SIGKILL
                });
            }
            Err(invalid("Native signal did not terminate the fixture"))
        }
        "io125" => {
            let mut bytes = Vec::new();
            std::io::stdin().read_to_end(&mut bytes)?;
            fs::write(root.join("input.bin"), bytes)?;
            fs::write(
                root.join("environment.json"),
                serde_json::to_vec(&json!({
                    "present":std::env::var("FIXTURE_PRESENT").ok(),
                    "ambient":std::env::var("EVAL_CAPTURE_AMBIENT").ok(),
                }))?,
            )?;
            std::io::stdout().write_all(b"out\0\xff")?;
            std::io::stderr().write_all(b"err\0\xfe")?;
            Ok(125)
        }
        "unread0" => Ok(0),
        "unread125" => Ok(125),
        "early-eof125" => {
            #[cfg(unix)]
            unsafe {
                libc::close(1);
                libc::close(2);
            }
            std::thread::sleep(Duration::from_millis(150));
            Ok(125)
        }
        "held125" | "held125-stopped-guardian" => {
            Command::new(std::env::current_exe()?)
                .arg("descendant")
                .arg(root)
                .arg(scenario)
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .and_then(|child| {
                    let until = std::time::Instant::now() + Duration::from_secs(1);
                    let receipt = root
                        .join("receipts")
                        .join(format!("{}-descendant.json", child.id()));
                    while !receipt.exists() && std::time::Instant::now() < until {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    if !receipt.exists() {
                        return Err(std::io::Error::other(
                            "descendant did not publish its identity",
                        ));
                    }
                    Ok(child)
                })?;
            ready(root);
            #[cfg(unix)]
            if scenario == "held125-stopped-guardian" {
                // Stop only the actual guardian after the descendant has
                // published its identity. The leaf exits before another
                // guardian poll; timeout then resumes owned cleanup.
                if unsafe { libc::kill(libc::getppid(), libc::SIGSTOP) } < 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                #[cfg(target_os = "linux")]
                {
                    let until = std::time::Instant::now() + Duration::from_secs(1);
                    loop {
                        let stat = fs::read_to_string(format!("/proc/{}/stat", unsafe {
                            libc::getppid()
                        }))?;
                        if stat
                            .rsplit_once(") ")
                            .is_some_and(|(_, tail)| tail.starts_with("T "))
                        {
                            fs::write(
                                root.join("guardian-stopped"),
                                b"confirmed-stopped-before-exit",
                            )?;
                            break;
                        }
                        if std::time::Instant::now() >= until {
                            return Err(invalid("Guardian did not enter the stopped state"));
                        }
                        std::thread::sleep(Duration::from_millis(2));
                    }
                }
            }
            Ok(125)
        }
        "late-event" => {
            println!(
                "{}",
                json!({"type":"thread.started","thread_id":"fixture-thread"})
            );
            println!("{}", json!({"type":"turn.started"}));
            std::io::stdout().flush()?;
            Command::new(std::env::current_exe()?)
                .arg("descendant")
                .arg(root)
                .arg(scenario)
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .and_then(|child| {
                    let until = std::time::Instant::now() + Duration::from_secs(1);
                    let receipt = root
                        .join("receipts")
                        .join(format!("{}-descendant.json", child.id()));
                    while !receipt.exists() && std::time::Instant::now() < until {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    if !receipt.exists() {
                        return Err(std::io::Error::other(
                            "descendant did not publish its identity",
                        ));
                    }
                    Ok(child)
                })?;
            ready(root);
            Ok(0)
        }
        "timeout" | "cancel" | "guardian-loss" | "callback-panic" => {
            Command::new(std::env::current_exe()?)
                .arg("descendant")
                .arg(root)
                .arg(scenario)
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .and_then(|child| {
                    let until = std::time::Instant::now() + Duration::from_secs(1);
                    let receipt = root
                        .join("receipts")
                        .join(format!("{}-descendant.json", child.id()));
                    while !receipt.exists() && std::time::Instant::now() < until {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    if !receipt.exists() {
                        return Err(std::io::Error::other(
                            "descendant did not publish its identity",
                        ));
                    }
                    Ok(child)
                })?;
            if scenario == "callback-panic" {
                println!(
                    "{}",
                    json!({"type":"thread.started","thread_id":"fixture-thread"})
                );
                std::io::stdout().flush()?;
            }
            ready(root);
            loop {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        "input-backpressure" => {
            ready(root);
            loop {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        "output-cap" => {
            ready(root);
            // Each stream is below the configured cap; only their combined
            // admission crosses it. A per-stream cap would incorrectly pass.
            std::io::stdout().write_all(&[b'o'; 12288])?;
            std::io::stderr().write_all(&[b'e'; 12288])?;
            Ok(0)
        }
        "fd-scrub" => {
            #[cfg(target_os = "linux")]
            {
                let directory = fs::read_dir("/proc/self/fd")?;
                let mut fds = directory
                    .filter_map(|entry| entry.ok())
                    .filter_map(|entry| entry.file_name().to_str()?.parse::<i32>().ok())
                    .collect::<Vec<_>>();
                fds.sort_unstable();
                fds.retain(|fd| unsafe { libc::fcntl(*fd, libc::F_GETFD) } >= 0);
                fs::write(root.join("fds.json"), serde_json::to_vec(&fds)?)?;
            }
            Ok(0)
        }
        "must-not-run" => {
            fs::write(root.join("second-spawned"), b"unexpected")?;
            Ok(0)
        }
        _ => Err(invalid("Unknown native fixture scenario")),
    }
}
#[cfg(target_os = "linux")]
fn live_receipts(root: &Path) -> Vec<u32> {
    let mut live = Vec::new();
    if let Ok(entries) = fs::read_dir(root.join("receipts")) {
        for entry in entries.flatten() {
            let Ok(bytes) = fs::read(entry.path()) else {
                continue;
            };
            let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
                continue;
            };
            let Some(pid) = value["pid"].as_u64() else {
                continue;
            };
            match fs::read_to_string(format!("/proc/{pid}/stat")) {
                Ok(raw) => {
                    if let Some((_, tail)) = raw.rsplit_once(") ") {
                        let values = tail.split_whitespace().collect::<Vec<_>>();
                        if values.get(19).and_then(|v| v.parse::<u64>().ok())
                            == value["start"].as_u64()
                            && !values.first().is_some_and(|v| matches!(*v, "Z" | "X"))
                        {
                            live.push(pid as u32);
                        }
                    } else {
                        live.push(pid as u32);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => live.push(pid as u32),
            }
        }
    }
    live
}

fn experiment_leaf(root: &Path, scenario: &str) -> Result<i32> {
    if scenario == "experiment-natural125" {
        return Ok(125);
    }
    #[cfg(target_os = "linux")]
    if scenario == "experiment-input" && unsafe { libc::fcntl(0, libc::F_SETPIPE_SZ, 4096) } < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let child = Command::new(std::env::current_exe()?)
        .arg("descendant")
        .arg(root)
        .arg(scenario)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;
    let receipt = root
        .join("receipts")
        .join(format!("{}-descendant.json", child.id()));
    let until = std::time::Instant::now() + Duration::from_secs(1);
    while !receipt.exists() && std::time::Instant::now() < until {
        std::thread::sleep(Duration::from_millis(2));
    }
    if !receipt.exists() {
        return Err(invalid(
            "Experiment descendant did not publish its identity",
        ));
    }
    ready(root);
    if matches!(scenario, "experiment-leader-exited" | "experiment-input") {
        if scenario == "experiment-input" {
            #[cfg(unix)]
            unsafe {
                libc::close(0);
                libc::close(1);
                libc::close(2);
            }
        }
        return Ok(0);
    }
    if scenario == "experiment-start-admission" {
        std::thread::sleep(Duration::from_millis(220));
        fs::write(
            root.join("experiment-start-was-not-interrupted"),
            b"signal-ran-before-native-admission",
        )?;
    }
    loop {
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn result_is_fatal(report: &Value) -> bool {
    report["error"]["code"] == "cleanup-unconfirmed"
}
fn experiment_scenario(root: &Path, name: &str) -> Result<Value> {
    use infra_host::capture::{ExperimentSignal, ExperimentStop};
    let _signals = infra_eval::cancel::Scope::install()?;
    let scratch = artifact::Scratch::new(root, "experiment-cwd")?;
    let input = if name == "experiment-input" {
        vec![b'x'; 1024 * 1024]
    } else {
        Vec::new()
    };
    let spec = process::CommandSpec {
        argv: vec![
            std::env::current_exe()?.to_string_lossy().into(),
            "leaf".into(),
            root.to_string_lossy().into(),
            name.into(),
        ],
        cwd: scratch.0.clone(),
        env: BTreeMap::from([(
            "FIXTURE_EXPERIMENT_ROOT".into(),
            root.to_string_lossy().into_owned(),
        )]),
        input,
    };
    let signal = match name {
        "experiment-int" => ExperimentSignal::InterruptGroup,
        "experiment-kill" | "experiment-zero-delay" | "experiment-start-admission" => {
            ExperimentSignal::KillGroup
        }
        _ => ExperimentSignal::TerminateLeader,
    };
    let stop = ExperimentStop {
        after: Duration::from_millis(if name == "experiment-zero-delay" {
            0
        } else if matches!(
            name,
            "experiment-natural125"
                | "experiment-observer-loss"
                | "experiment-anchor-loss"
                | "experiment-parent-loss"
        ) {
            1000
        } else {
            80
        }),
        signal,
        grace: Duration::from_millis(if name == "experiment-grace" { 120 } else { 600 }),
    };
    let mut metadata = None;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let started = std::time::Instant::now();
    let result = if name == "experiment-adapter-term" {
        process::experiment(&spec, stop).map(|exit| json!({"exit":exit}))
    } else {
        infra_host::capture::run_experiment(
            infra_host::capture::Request {
                argv: &spec.argv,
                cwd: &spec.cwd,
                environment: &spec.env,
                input: &spec.input,
                output_limit: 1024 * 1024,
                deadline: std::time::Instant::now() + Duration::from_secs(4),
            },
            stop,
            infra_eval::cancel::interrupted,
            |native| metadata = Some(native),
            |stream, bytes| {
                match stream {
                    infra_host::capture::Stream::Stdout => stdout.extend_from_slice(bytes),
                    infra_host::capture::Stream::Stderr => stderr.extend_from_slice(bytes),
                };
                Ok(())
            },
        )
        .map(|output| json!({"exit":output.exit.or_else(||output.signal.map(|signal|-signal))}))
        .map_err(|error| {
            RuntimeError::coded(
                infra_core::ErrorKind::Unavailable,
                error.code,
                error.message,
            )
        })
    };
    fs::write(root.join("stdout.bin"), stdout)?;
    fs::write(root.join("stderr.bin"), stderr)?;
    let mut report = match result {
        Ok(session) => json!({"ok":true,"session":session}),
        Err(error) => failure(&error),
    };
    if result_is_fatal(&report) {
        std::mem::forget(scratch);
    } else {
        drop(scratch);
    }
    report["seconds"] = json!(started.elapsed().as_secs_f64());
    report["native"]=metadata.map(|native|json!({"code":native.code,"signal":native.signal,"shell_code":native.shell_code})).unwrap_or(Value::Null);
    #[cfg(target_os = "linux")]
    {
        report["caller_pgid"] = json!(unsafe { libc::getpgrp() });
        report["live_at_return"] = json!(live_receipts(root));
    }
    Ok(report)
}

fn scenario(root: &Path, name: &str) -> Result<Value> {
    if name.starts_with("experiment-") {
        return experiment_scenario(root, name);
    }
    let _signals = infra_eval::cancel::Scope::install()?;
    let mut environment = BTreeMap::new();
    environment.insert("FIXTURE_PRESENT".into(), "yes".into());
    #[cfg(windows)]
    for key in ["SYSTEMROOT", "WINDIR", "SYSTEMDRIVE"] {
        if let Ok(value) = std::env::var(key) {
            environment.insert(key.into(), value);
        }
    }
    let input = match name {
        "io125" => vec![0, 1, 2, 0, 255],
        "unread0" | "unread125" | "input-backpressure" => vec![b'x'; 1024 * 1024],
        _ => Vec::new(),
    };
    let scratch = artifact::Scratch::new(root, "owned-cwd")?;
    fs::write(scratch.0.join("retained-state"), b"private fixture state")?;
    let cwd = scratch.0.clone();
    let spec = process::CommandSpec {
        argv: vec![
            std::env::current_exe()?.to_string_lossy().into(),
            "leaf".into(),
            root.to_string_lossy().into(),
            name.into(),
        ],
        cwd,
        env: environment,
        input,
    };
    let budget = process::Budget {
        timeout: Duration::from_millis(
            if matches!(
                name,
                "timeout" | "input-backpressure" | "held125" | "held125-stopped-guardian"
            ) {
                300
            } else {
                2000
            },
        ),
        cap: 100,
        max_bytes: if name == "output-cap" {
            16384
        } else {
            1024 * 1024
        },
        native: matches!(name, "late-event" | "callback-panic"),
        strict_events: true,
        error_diagnostics_terminal: true,
    };
    #[cfg(unix)]
    let _stray = if name == "fd-scrub" {
        use std::os::fd::AsRawFd;
        let file = fs::File::open(root.join("stray"))?;
        let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFD) };
        if flags < 0
            || unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, flags & !libc::FD_CLOEXEC) }
                < 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        Some(file)
    } else {
        None
    };
    let result = process::execute(
        &spec,
        &budget,
        Some(&root.join("stdout.bin")),
        Some(&root.join("stderr.bin")),
        |_, _| {
            assert_ne!(name, "callback-panic", "isolated native observer panic");
            None
        },
    );
    let mut report = match result {
        Ok(session) => json!({"ok":true,"session":session.value()}),
        Err(error) => failure(&error),
    };
    #[cfg(target_os = "linux")]
    {
        report["live_at_return"] = json!(live_receipts(root));
    }
    let retained = scratch.0.clone();
    drop(scratch);
    report["retained"] = json!(retained);
    report["retained_exists"] = json!(retained.exists());
    if name == "callback-panic" {
        let mut next = spec.clone();
        next.argv[3] = "must-not-run".into();
        let next_log = root.join("next-must-not-exist");
        let refused = process::execute(&next, &budget, Some(&next_log), None, |_, _| None);
        report["admission_blocked"] = json!(
            refused
                .as_ref()
                .err()
                .is_some_and(|e| e.code.as_deref() == Some("cleanup-unconfirmed"))
        );
        report["next_log_absent"] = json!(!next_log.exists());
        report["next_spawn_absent"] = json!(!root.join("second-spawned").exists());
    }
    Ok(report)
}
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str) == Some("internal-guard") {
        #[cfg(target_os = "linux")]
        if args.get(1).map(String::as_str) == Some("--capture-observer")
            && let Some(root) = std::env::var_os("FIXTURE_EXPERIMENT_ROOT")
        {
            let root = PathBuf::from(root);
            record(&root, "observer-boot");
            let group = args
                .get(5)
                .and_then(|value| value.parse::<u32>().ok())
                .expect("actual fixture anchor group");
            fs::write(
                root.join("anchor.json"),
                serde_json::to_vec(&image(group, "anchor")).unwrap(),
            )
            .unwrap();
            fs::write(
                root.join("guardian.json"),
                serde_json::to_vec(&image(std::process::id(), "guardian")).unwrap(),
            )
            .unwrap();
            if args.last().map(String::as_str) == Some("experiment-start-admission") {
                fs::write(
                    root.join("observer-boot-stop"),
                    b"private-observer-bootstrap",
                )
                .unwrap();
                unsafe {
                    libc::raise(libc::SIGSTOP);
                }
            }
        }
        let code = infra_host::process::guard_entry(&args[1..]).unwrap_or(125);
        std::process::exit(code);
    }
    if args.first().map(String::as_str) == Some("descendant") && args.len() == 3 {
        descendant(Path::new(&args[1]), &args[2]);
        return;
    }
    if args.first().map(String::as_str) == Some("leaf") && args.len() == 3 {
        let code = leaf(Path::new(&args[1]), &args[2]).unwrap_or(126);
        std::process::exit(code);
    }
    let result = match args.first().map(String::as_str) {
        Some(mode @ ("graph-baseline" | "graph-runtime-at")) => graph_scenario(mode, &args[1..]),
        Some("process") if args.len() == 3 => scenario(Path::new(&args[1]), &args[2]),
        _ => Err(invalid("Unknown fixture driver operation")),
    };
    let (value, code) = match result {
        Ok(value) => (value, 0),
        Err(error) => (failure(&error), 2),
    };
    println!("{}", serde_json::to_string(&value).unwrap());
    std::process::exit(code);
}
