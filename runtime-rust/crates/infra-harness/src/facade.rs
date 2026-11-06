//! Native public entrypoints; command arguments are parsed without shell expansion.
use crate::{HarnessError, Result, desktop, native::NativeFactory, store::SessionStore};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};
fn runtime(error: HarnessError) -> infra_core::RuntimeError {
    match error.status {
        400 => infra_core::RuntimeError::invalid(error.message),
        403 => infra_core::RuntimeError::policy(error.message),
        404 => infra_core::RuntimeError::not_found(error.message),
        409 => infra_core::RuntimeError::conflict(error.message),
        _ => infra_core::RuntimeError::unavailable(error.message),
    }
}
fn root_source(args: &[String]) -> Result<PathBuf> {
    let mut explicit = None;
    for (index, arg) in args.iter().enumerate() {
        if arg == "--source" {
            explicit =
                Some(PathBuf::from(args.get(index + 1).ok_or_else(|| {
                    HarnessError::invalid("Missing source root.")
                })?));
        } else if let Some(value) = arg.strip_prefix("--source=") {
            explicit = Some(PathBuf::from(value));
        }
    }
    let mut candidates = vec![];
    if let Some(path) = explicit {
        candidates.push(path);
    } else {
        if let Ok(directory) = desktop::config_directory()
            && let Some(value) = crate::creator_files::object(&directory, "app.json", false)
                .ok()
                .flatten()
            && let Some(path) = value["clone"].as_str()
        {
            candidates.push(PathBuf::from(path));
        }
        if let Some(path) = std::env::var_os("ACCELERATOR_HOME") {
            let path = PathBuf::from(path);
            candidates.extend(path.ancestors().map(Path::to_path_buf));
        }
        for path in [
            std::env::current_exe()?.parent().map(Path::to_path_buf),
            Some(std::env::current_dir()?),
        ]
        .into_iter()
        .flatten()
        {
            candidates.extend(path.ancestors().map(Path::to_path_buf));
        }
    }
    for path in candidates {
        if let Ok(path) = infra_core::fs::canonicalize(path)
            && path.join("harness/web/index.html").is_file()
            && path.join("Infrastructure-Creator/AGENTS.md").is_file()
        {
            infra_core::bank::publication::PublicationFiles::new(&path)?;
            return Ok(path);
        }
    }
    Err(HarnessError::missing(
        "The accelerator clone is unavailable. Pass --source or restore the desktop application's clone.",
    ))
}
fn parse(args: &[String], allowed: &[&str], multiple: &[&str], booleans: &[&str]) -> Result<Value> {
    let mut out = json!({});
    let mut at = 0usize;
    while at < args.len() {
        let option = &args[at];
        if !option.starts_with("--") {
            return Err(HarnessError::invalid(format!(
                "Unexpected argument: {option}"
            )));
        }
        let spelling = &option[2..];
        let (name, inline) = spelling
            .split_once('=')
            .map(|(a, b)| (a, Some(b)))
            .unwrap_or((spelling, None));
        if !allowed.contains(&name) && !booleans.contains(&name) {
            return Err(HarnessError::invalid(format!("Unknown option: --{name}")));
        }
        let key = name.replace('-', "_");
        if booleans.contains(&name) {
            if inline.is_some() {
                return Err(HarnessError::invalid(format!("--{name} takes no value.")));
            }
            out[&key] = true.into();
        } else {
            let value = if let Some(value) = inline {
                value.to_owned()
            } else {
                at += 1;
                args.get(at)
                    .ok_or_else(|| HarnessError::invalid(format!("Missing --{name} value.")))?
                    .clone()
            };
            if value.contains('\0') {
                return Err(HarnessError::invalid("Arguments cannot contain NUL."));
            }
            if multiple.contains(&name) {
                if !out[&key].is_array() {
                    out[&key] = json!([]);
                }
                out[&key].as_array_mut().unwrap().push(value.into());
            } else {
                if out.get(&key).is_some() {
                    return Err(HarnessError::invalid(format!("Repeated --{name} option.")));
                }
                out[&key] = value.into();
            }
        }
        at += 1;
    }
    Ok(out)
}
fn required<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| HarnessError::invalid(format!("Missing --{}.", key.replace('_', "-"))))
}
fn absolute(value: &str) -> Result<PathBuf> {
    let path = PathBuf::from(value);
    Ok(if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    })
}
fn number(value: &Value, key: &str, default: u64, min: u64, max: u64) -> Result<u64> {
    let number = match value.get(key) {
        None => default,
        Some(Value::String(value)) => value
            .parse()
            .map_err(|_| HarnessError::invalid(format!("Invalid {key}.")))?,
        Some(value) => value
            .as_u64()
            .ok_or_else(|| HarnessError::invalid(format!("Invalid {key}.")))?,
    };
    if !(min..=max).contains(&number) {
        return Err(HarnessError::invalid(format!(
            "{key} is outside its supported range."
        )));
    }
    Ok(number)
}
async fn fleet(args: &[String]) -> Result<i32> {
    let command = args
        .first()
        .map(String::as_str)
        .ok_or_else(|| HarnessError::invalid("Use harness run or resume."))?;
    if !["run", "resume"].contains(&command) {
        return Err(HarnessError::invalid("Use harness run or resume."));
    }
    let data = parse(
        &args[1..],
        &[
            "project",
            "worker",
            "lenses",
            "model",
            "budget-usd",
            "worker-timeout",
            "state-dir",
            "reports-dir",
            "scope",
            "task-id",
            "thread",
        ],
        &[],
        &["approve", "reject"],
    )?;
    let project = infra_core::fs::canonicalize(required(&data, "project")?)?;
    let state = absolute(data["state_dir"].as_str().unwrap_or("state"))?;
    let reports = absolute(data["reports_dir"].as_str().unwrap_or("reports"))?;
    infra_core::fs::ensure_directory(&reports, false)?;
    infra_core::bank::publication::PublicationFiles::new(&reports)?;
    let worker = data["worker"].as_str().unwrap_or("claude-cli");
    let (provider, dry) = match worker {
        "dry-run" => ("claude", true),
        "claude-cli" => ("claude", false),
        "codex-cli" => ("codex", false),
        "cursor-cli" => ("cursor", false),
        _ => {
            return Err(HarnessError::invalid(
                "Choose dry-run, claude-cli, codex-cli or cursor-cli.",
            ));
        }
    };
    let budget = match data["budget_usd"].as_str() {
        None | Some("none") => None,
        Some(value) => {
            let value = value
                .parse::<f64>()
                .map_err(|_| HarnessError::invalid("Invalid USD budget."))?;
            if !value.is_finite() || value <= 0. || provider != "claude" && !dry {
                return Err(HarnessError::invalid(
                    "A finite positive USD cap requires Claude or dry-run.",
                ));
            }
            Some(value)
        }
    };
    let lenses = data["lenses"]
        .as_str()
        .unwrap_or("code-reviewer,security-reviewer,performance-optimization")
        .split(',')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if lenses.is_empty()
        || lenses.len() > 40
        || lenses
            .iter()
            .any(|s| s.is_empty() || s.chars().any(char::is_control))
        || lenses.iter().collect::<BTreeSet<_>>().len() != lenses.len()
    {
        return Err(HarnessError::invalid(
            "Choose distinct bounded reviewer lenses.",
        ));
    }
    let executable = if dry {
        PathBuf::from("offline")
    } else {
        let providers = crate::provider::discover(&BTreeMap::new()).await?;
        providers
            .into_iter()
            .find(|p| p.id == provider && p.available)
            .and_then(|p| p.executable)
            .ok_or_else(|| {
                HarnessError::unavailable("The selected native provider is unavailable.")
            })?
    };
    let store = SessionStore::open(&state)?;
    let roots = infra_core::Roots::installed(&project)?;
    let mode = infra_core::RuntimeConfig::load(&roots)?.mode(None)?;
    let actor = std::env::var("PROJECT_BRAIN_OWNER")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "main".into());
    let context = infra_core::RuntimeContext::new(roots, actor, mode)?;
    let board = Arc::new(crate::blackboard::CoreBlackboard {
        available: context
            .roots
            .state
            .join("project-brain/config/runtime.json")
            .is_file(),
        context,
    });
    let config = crate::fleet::Config {
        project,
        provider: provider.into(),
        executable,
        lenses,
        model: data["model"].as_str().map(str::to_owned),
        effort: None,
        budget,
        seconds: number(&data, "worker_timeout", 1800, 1, 86400)?,
        concurrency: 3,
        dry_run: dry,
        reports,
        adapter: None,
        runner_lock: Some(store.runner_lock()),
    };
    let thread = data["thread"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            format!(
                "fleet-review-{}",
                chrono::Utc::now().format("%Y%m%d-%H%M%S")
            )
        });
    if thread.is_empty()
        || thread.len() > 128
        || !thread
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(HarnessError::invalid(
            "Choose a bounded thread ID without path separators.",
        ));
    }
    if command == "resume" && (data["thread"].is_null() || data["approve"] == data["reject"]) {
        return Err(HarnessError::invalid(
            "Resume needs --thread and exactly one of --approve or --reject.",
        ));
    }
    let engine = crate::fleet::Fleet {
        store: store.clone(),
        blackboard: board,
    };
    let scope = data["scope"]
        .as_str()
        .unwrap_or("the current branch's changes against the default branch");
    let task = data["task_id"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("harness/{thread}"));
    engine.initialize(
        &thread,
        scope,
        &task,
        &config,
        Some(&state.join("checkpoints.sqlite")),
        None,
    )?;
    if command == "resume" {
        engine.decide(&thread, data["approve"] == true)?;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    let listener = tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        signal.store(true, std::sync::atomic::Ordering::Release);
    });
    let result = engine
        .drive(
            &thread,
            &config,
            cancel,
            Arc::new(|event| {
                if event["kind"] == "error" {
                    eprintln!(
                        "{}",
                        event["text"].as_str().unwrap_or("Fleet worker failed.")
                    );
                }
                Ok(())
            }),
        )
        .await;
    listener.abort();
    let result = result?;
    println!("Thread: {thread}");
    if result["status"] == "awaiting_approval" {
        println!(
            "PAUSED at the approval gate:\n{}",
            serde_json::to_string_pretty(&result)?
        );
        println!(
            "Resume with: harness resume --project <PATH> --thread {thread} --approve|--reject"
        );
    } else if let Some(report) = result["report"].as_str().filter(|s| !s.is_empty()) {
        println!("{report}");
    } else {
        println!("Run finished; report rejected or empty.");
    }
    Ok(0)
}
async fn system(args: &[String]) -> Result<i32> {
    let op = args
        .first()
        .map(String::as_str)
        .ok_or_else(|| HarnessError::invalid("Choose a native system operation."))?;
    let options = parse(
        &args[1..],
        &[
            "root",
            "name",
            "system",
            "task",
            "change-id",
            "service",
            "contract",
            "context-budget",
            "impact-depth",
            "plan",
            "run-dir",
            "provider",
            "executable",
            "mode",
            "timeout",
            "access",
            "retry-step",
            "output",
            "allow-root",
        ],
        &["service", "contract", "allow-root"],
        &["accept-source-changes", "accept-unrun-checks"],
    )?;
    let cancel = Arc::new(AtomicBool::new(false));
    let control = cancel.clone();
    let signal = tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        control.store(true, std::sync::atomic::Ordering::Release);
    });
    let result = infra_system::execute_async(op, &options, cancel, &mut |event| {
        eprintln!("{}", event);
        Ok(())
    })
    .await;
    signal.abort();
    let result = result?;
    if let Some(cli) = result.get("_cli") {
        if let Some(stdout) = cli["stdout"].as_str() {
            print!("{stdout}");
        }
        if let Some(stderr) = cli["stderr"].as_str() {
            eprint!("{stderr}");
        }
        return Ok(cli["code"].as_i64().unwrap_or(1) as i32);
    }
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(0)
}
fn validate_app_mutator_args(args: &[String]) -> Result<()> {
    if !args
        .first()
        .is_some_and(|command| matches!(command.as_str(), "install" | "uninstall"))
    {
        return Ok(());
    }
    let mut options = args[1..].iter();
    while let Some(option) = options.next() {
        let value = if option == "--source" {
            options
                .next()
                .filter(|value| !value.starts_with("--"))
                .ok_or_else(|| HarnessError::invalid("Missing --source value."))?
                .as_str()
        } else if let Some(value) = option.strip_prefix("--source=") {
            value
        } else {
            return Err(HarnessError::invalid(format!(
                "Unknown application option: {option}"
            )));
        };
        if value.is_empty() || value.contains('\0') {
            return Err(HarnessError::invalid("Invalid --source value."));
        }
    }
    Ok(())
}

fn public_help(group: &str, args: &[String]) -> Option<&'static str> {
    let text = match group {
        "harness" => {
            r#"Usage: infra-engine harness run --project PATH [OPTIONS]
       infra-engine harness resume --project PATH --thread ID (--approve | --reject) [OPTIONS]

Options:
  --project PATH          Existing target project root (required).
  --worker NAME           dry-run, claude-cli, codex-cli or cursor-cli (default: claude-cli).
  --lenses LIST           Distinct comma-separated reviewer names (at most 40).
                          Default: code-reviewer,security-reviewer,performance-optimization.
  --model NAME            Native provider model.
  --budget-usd AMOUNT     Positive USD cap for Claude or dry-run; none means no cap (default: none).
  --worker-timeout N      Worker limit in seconds, 1-86400 (default: 1800).
  --state-dir PATH        Durable runner state (default: state).
  --reports-dir PATH      Review reports (default: reports).
  --scope TEXT            Review scope (default: current branch changes against the default branch).
  --task-id ID            Project Brain task (default: harness/<thread>).
  --thread ID             Saved run ID; required for resume, generated from UTC time for run.
  --approve               Resume and approve the saved report.
  --reject                Resume and reject the saved report.
  -h, --help              Show this help without starting a run.

Value options also accept --option=value. Resume requires exactly one verdict.
"#
        }
        "serve" => {
            r#"Usage: infra-engine serve [start | serve | status | stop | update | relaunch] [OPTIONS]

Default: start in the background. The serve command runs in the foreground.

Options:
  --source PATH           Accelerator source clone.
  --state-dir PATH        Private server state (default: the platform user state directory).
  --project PATH          Register an existing project; repeat for multiple projects.
                          Default: the current directory when it is outside the source clone.
  --port N                Loopback port, 0-65535 (default: 8766; 0 chooses a free port).
  --timeout N             Initial session time budget, 1-86400 seconds (default: 900).
  --session-concurrency N  Experimental independent-session slots, 1-4 (default: 1).
  --claude-bin PATH       Explicit Claude CLI executable.
  --codex-bin PATH        Explicit Codex CLI executable.
  --cursor-bin PATH       Explicit Cursor CLI executable.
  -h, --help              Show this help without starting or contacting a server.

Value options also accept --option=value.
"#
        }
        "app" => {
            r#"Usage: infra-engine app [install | uninstall | open | status | stop | update] [OPTIONS]

Default: open. install/uninstall manage the desktop entry and keep project data.
open starts the server and opens its URL; status/stop/update control the server.

Options:
  --source PATH           Accelerator source clone.
  -h, --help              Show this help without installing, removing or opening the application.

Server options follow open, status, stop or update:
  --state-dir PATH        Private server state (default: the platform user state directory).
  --project PATH          Register an existing project; repeat for multiple projects.
                          Default: the current directory when it is outside the source clone.
  --port N                Loopback port, 0-65535 (default: 8766; 0 chooses a free port).
  --timeout N             Initial session time budget, 1-86400 seconds (default: 900).
  --session-concurrency N  Experimental independent-session slots, 1-4 (default: 1).
  --claude-bin PATH       Explicit Claude CLI executable.
  --codex-bin PATH        Explicit Codex CLI executable.
  --cursor-bin PATH       Explicit Cursor CLI executable.

Value options also accept --option=value.
"#
        }
        _ => return None,
    };
    args.iter()
        .take_while(|argument| argument.as_str() != "--")
        .any(|argument| matches!(argument.as_str(), "-h" | "--help"))
        .then_some(text)
}

pub async fn run(group: &str, args: &[String]) -> infra_core::Result<i32> {
    if let Some(help) = public_help(group, args) {
        print!("{help}");
        return Ok(0);
    }
    if group == "app" {
        validate_app_mutator_args(args).map_err(runtime)?;
    }
    let result = match group {
        "internal-guard" => crate::process::guard_entry(args),
        "internal-sandbox-child" => crate::sandbox::child_entry(args),
        "internal-discovery-probe" => crate::sandbox::probe_entry(args),
        "harness" => fleet(args).await,
        "system" => system(args).await,
        "serve" | "app" => {
            let control_only = (group == "app"
                && args.first().is_some_and(|c| {
                    ["uninstall", "status", "stop", "update"].contains(&c.as_str())
                }))
                || (group == "serve"
                    && args
                        .first()
                        .is_some_and(|c| ["status", "stop", "update"].contains(&c.as_str())));
            let source = if control_only {
                root_source(args)
                    .unwrap_or(std::env::current_dir().map_err(infra_core::RuntimeError::from)?)
            } else {
                root_source(args).map_err(runtime)?
            };
            let factory = Arc::new(NativeFactory);
            if group == "serve" {
                desktop::server(args, &source, factory).await
            } else {
                desktop::app(args, &source, factory).await
            }
        }
        _ => Err(HarnessError::invalid(
            "Unknown native Harness command group.",
        )),
    };
    result.map_err(runtime)
}
