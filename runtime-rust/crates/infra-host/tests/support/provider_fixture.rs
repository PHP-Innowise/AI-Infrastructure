//! Noninstalled fake provider/driver. No real CLI, account, environment override or network.
use infra_host::{HostError, Result, provider};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const IDS: [&str; 3] = ["claude", "codex", "cursor"];
const INPUT_LIMIT: u64 = 65536;
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Fake {
    gate_version: bool,
    delay_ms: u64,
    version_code: i32,
    help: Option<String>,
    output_bytes: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    overrides: BTreeMap<String, String>,
    #[serde(default)]
    drop_after_signal: bool,
}
fn fail(message: &str) -> HostError {
    HostError::invalid(message)
}
fn read<T: serde::de::DeserializeOwned>(source: &Path) -> Result<T> {
    if fs::metadata(source)?.len() > INPUT_LIMIT {
        return Err(fail("Fixture input exceeds bound"));
    }
    let value = infra_host::strict_json::parse(&fs::read(source)?, INPUT_LIMIT as usize)?;
    serde_json::from_value(value).map_err(Into::into)
}
fn root(value: &str) -> Result<PathBuf> {
    let selected = Path::new(value);
    if !selected.is_absolute() || !selected.is_dir() {
        return Err(fail("Expected an existing absolute fixture directory"));
    }
    Ok(infra_core::fs::canonicalize(selected)?)
}
fn executable(root: &Path, prefix: &str, id: &str) -> PathBuf {
    root.join(format!("{prefix}-{id}{}", std::env::consts::EXE_SUFFIX))
}
fn prepare(selected: &Path) -> Result<Value> {
    let source = std::env::current_exe()?;
    for id in IDS {
        let target = executable(selected, "fake", id);
        if target.exists() {
            return Err(fail("Fixture executable already exists"));
        }
        fs::copy(&source, target)?;
    }
    // This short-lived process exits before discovery can inherit a descriptor
    // opened while copying an executable.
    Ok(json!({"prepared":true}))
}
async fn wait(selected: &Path, marker: &str) -> Result<()> {
    let until = Instant::now() + Duration::from_secs(15);
    while !selected.join(marker).is_file() {
        if Instant::now() >= until {
            return Err(fail("Fixture synchronization deadline"));
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    Ok(())
}
fn report(result: Result<Vec<provider::Provider>>, started: Instant) -> Value {
    match result {
        Ok(providers) => {
            json!({"ok":true,"providers":providers,"seconds":started.elapsed().as_secs_f64()})
        }
        Err(error) => {
            json!({"ok":false,"error":{"status":error.status,"code":error.code,"message":error.message}})
        }
    }
}
async fn discover(source: &Path) -> Result<Value> {
    let selected = source
        .parent()
        .ok_or_else(|| fail("Missing fixture request parent"))?;
    let selected = infra_core::fs::canonicalize(selected)?;
    let request: Request = read(source)?;
    if request.overrides.len() != IDS.len() {
        return Err(fail("Every fake provider override is required"));
    }
    for id in IDS {
        let value = request
            .overrides
            .get(id)
            .ok_or_else(|| fail("Missing fake provider override"))?;
        let given = Path::new(value);
        if given != executable(&selected, "fake", id)
            && given != executable(&selected, "missing", id)
        {
            return Err(fail(
                "Only fixture-owned absolute executable overrides are permitted",
            ));
        }
    }
    let started = Instant::now();
    let mut probe = Box::pin(provider::discover(&request.overrides));
    if !request.drop_after_signal {
        return Ok(report(probe.await, started));
    }
    let completed = tokio::select! {
        result = probe.as_mut() => Some(result),
        signal = wait(&selected, "drop-discovery") => { signal?; None },
    };
    if let Some(result) = completed {
        return Ok(report(result, started));
    }
    drop(probe);
    // Stay alive until the parent has observed cleanup of every pinned fake
    // leaf and guardian. Runtime teardown is not our cleanup evidence.
    wait(&selected, "finish-driver").await?;
    Ok(json!({"ok":true,"dropped":true}))
}
fn record(selected: &Path, id: &str, event: Value) -> Result<()> {
    let target = selected.join(format!("{id}.calls.jsonl"));
    if fs::metadata(&target).is_ok_and(|metadata| metadata.len() > INPUT_LIMIT) {
        return Err(fail("Fixture event log exceeds bound"));
    }
    let mut output = OpenOptions::new().create(true).append(true).open(target)?;
    writeln!(output, "{}", serde_json::to_string(&event)?)?;
    Ok(())
}
fn fake(arguments: &[String]) -> Result<i32> {
    let own = std::env::current_exe()?;
    let id = own
        .file_stem()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("fake-"))
        .filter(|id| IDS.contains(id))
        .ok_or_else(|| fail("Unknown fake provider identity"))?;
    let selected = own
        .parent()
        .ok_or_else(|| fail("Missing fake provider parent"))?;
    let flag = if id == "cursor" {
        if arguments.len() != 2 || arguments[0] != "--disable-auto-update" {
            return Err(fail("Cursor fixture requires the disabled-update flag"));
        }
        &arguments[1]
    } else {
        if arguments.len() != 1 {
            return Err(fail("Unexpected fake provider arguments"));
        }
        &arguments[0]
    };
    if !["--version", "--help"].contains(&flag.as_str()) {
        return Err(fail("Only identity probes are permitted"));
    }
    let spec: Fake = read(&selected.join(format!("{id}.json")))?;
    if spec.delay_ms > 1000 || spec.output_bytes > 131072 {
        return Err(fail("Fake workload exceeds fixture bounds"));
    }
    let pid = std::process::id();
    #[cfg(target_os = "linux")]
    let guardian_pid = Some(unsafe { libc::getppid() } as u32);
    #[cfg(not(target_os = "linux"))]
    let guardian_pid: Option<u32> = None;
    record(
        selected,
        id,
        json!({"event":"entered","args":arguments,"pid":pid}),
    )?;
    if flag == "--version" {
        fs::write(
            selected.join(format!("{id}.version.ready")),
            serde_json::to_vec(&json!({"pid":pid,"guardian_pid":guardian_pid}))?,
        )?;
        if spec.gate_version {
            let until = Instant::now() + Duration::from_secs(15);
            while !selected.join("release-versions").is_file() {
                if Instant::now() >= until {
                    return Err(fail("Fake version gate exceeded its deadline"));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    } else if !selected.join(format!("{id}.version.finished")).is_file() {
        return Err(fail("Help started before version completed"));
    }
    if spec.delay_ms > 0 {
        std::thread::sleep(Duration::from_millis(spec.delay_ms));
    }
    if spec.output_bytes > 0 && flag == "--version" {
        std::io::stdout().write_all(&vec![b'x'; spec.output_bytes])?;
    } else if flag == "--version" {
        println!("fake {id} version 1");
    } else {
        println!(
            "{}",
            spec.help.as_deref().unwrap_or(match id {
                "claude" => "Claude Code",
                "codex" => "Codex",
                _ => "Cursor --print --output-format --resume --workspace --sandbox --mode",
            })
        );
    }
    std::io::stdout().flush()?;
    record(
        selected,
        id,
        json!({"event":"finished","args":arguments,"pid":pid}),
    )?;
    if flag == "--version" {
        fs::write(selected.join(format!("{id}.version.finished")), b"finished")?;
    }
    Ok(if flag == "--version" {
        spec.version_code
    } else {
        0
    })
}
fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    #[cfg(any(unix, windows))]
    if arguments.first().map(String::as_str) == Some("internal-guard") {
        std::process::exit(infra_host::process::guard_entry(&arguments[1..]).unwrap_or(125));
    }
    let result = match arguments.as_slice() {
        [mode, selected] if mode == "prepare" => {
            root(selected).and_then(|selected| prepare(&selected))
        }
        [mode, source] if mode == "discover" => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            match runtime {
                Ok(runtime) => runtime.block_on(discover(Path::new(source))),
                Err(error) => Err(error.into()),
            }
        }
        _ => match fake(&arguments) {
            Ok(code) => std::process::exit(code),
            Err(error) => Err(error),
        },
    };
    match result {
        Ok(value) => println!("{}", serde_json::to_string(&value).unwrap()),
        Err(error) => {
            eprintln!("provider fixture: {}", error.message);
            std::process::exit(2);
        }
    }
}
