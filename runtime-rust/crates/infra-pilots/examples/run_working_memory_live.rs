//! Explicit real-worker experiment. Contacts the selected installed provider CLI.
use anyhow::{Context, Result, bail};
use infra_pilots::run_working_memory::{
    RunWorkingMemory, SnapshotStore, SqliteStore,
    live::{LiveContext, event_metric},
};
use infra_system::{catalog::System, common, native, runner};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::Instant,
};
struct Ephemeral {
    model: String,
    effort: String,
}
impl infra_host::provider::LaunchAdapter for Ephemeral {
    fn adapt(
        &self,
        input: &infra_host::provider::Launch,
        mut command: infra_host::provider::PreparedCommand,
    ) -> infra_host::Result<infra_host::provider::PreparedCommand> {
        if input.provider == "codex" {
            let at = command.argv.len().saturating_sub(1);
            command.argv.splice(
                at..at,
                [
                    "--ephemeral".into(),
                    "--ignore-user-config".into(),
                    "--model".into(),
                    self.model.clone(),
                    "-c".into(),
                    format!(
                        "model_reasoning_effort={}",
                        serde_json::to_string(&self.effort).map_err(|_| {
                            infra_host::HarnessError::invalid("Invalid experiment effort")
                        })?
                    ),
                ],
            );
        }
        Ok(command)
    }
}
fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    // The process supervisor re-enters its host executable. Serve it before
    // creating Tokio threads, just as infra-engine does.
    if args.first().map(String::as_str) == Some("internal-guard") {
        std::process::exit(infra_host::process::guard_entry(&args[1..])?);
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run(args))
}
async fn run(args: Vec<String>) -> Result<()> {
    let mut options = BTreeMap::new();
    let mut at = 0;
    while at < args.len() {
        let key = &args[at];
        if ![
            "--system",
            "--workspace",
            "--output",
            "--task-file",
            "--services",
            "--provider",
            "--arm",
            "--redis-endpoint",
            "--timeout",
            "--change-id",
            "--executable",
            "--model",
            "--effort",
        ]
        .contains(&key.as_str())
        {
            bail!("unknown_option")
        }
        let value = args.get(at + 1).context("missing_option_value")?;
        if options.insert(key.clone(), value.clone()).is_some() {
            bail!("duplicate_option")
        }
        at += 2;
    }
    let required = |name| {
        options
            .get(name)
            .context("missing_required_option")
            .cloned()
    };
    let workspace = infra_core::fs::canonicalize(PathBuf::from(required("--workspace")?))?;
    let config = infra_core::fs::canonicalize(PathBuf::from(required("--system")?))?;
    let directory = PathBuf::from(required("--output")?);
    let task = std::fs::read_to_string(required("--task-file")?)?;
    let services = required("--services")?
        .split(',')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let provider = required("--provider")?;
    if provider != "codex" {
        bail!("controlled_experiment_requires_codex")
    }
    let arm = required("--arm")?;
    if !["A", "B", "C"].contains(&arm.as_str()) {
        bail!("invalid_arm")
    }
    let mut system = System::new(&config, std::slice::from_ref(&workspace))?;
    let change = required("--change-id")?;
    let plan = system.plan(task.trim(), &change, &services, &[], 24000, 0)?;
    let timeout = options
        .get("--timeout")
        .map(|v| v.parse())
        .transpose()?
        .unwrap_or(600);
    let model = required("--model")?;
    common::identifier(&model)?;
    let effort = required("--effort")?;
    if !["low", "medium", "high", "xhigh", "ultra"].contains(&effort.as_str()) {
        bail!("invalid_effort")
    }
    let adapter: Arc<dyn infra_host::provider::LaunchAdapter> = Arc::new(Ephemeral {
        model: model.clone(),
        effort: effort.clone(),
    });
    let state = native::create_run_with_adapter(
        &mut system,
        &plan,
        &directory,
        &provider,
        options.get("--executable").map(String::as_str),
        "edit",
        timeout,
        "service",
        adapter.as_ref(),
    )
    .await?;
    let store: Box<dyn SnapshotStore> = match arm.as_str() {
        "A" | "B" => Box::new(SqliteStore::open(&directory.join("pilot-view-cache"))?),
        #[cfg(feature = "run-memory-redis")]
        "C" => Box::new(infra_pilots::run_working_memory::RedisStore::loopback(
            required("--redis-endpoint")?.parse()?,
        )?),
        _ => bail!("redis_feature_required"),
    };
    let memory = RunWorkingMemory::open(&directory, &state, &workspace, 1, store)?;
    let context = Arc::new(Mutex::new(LiveContext::new(memory, arm != "A")));
    let started = Instant::now();
    let mut events = vec![];
    let result = runner::drive_run_with_context(
        &mut system,
        &directory,
        Arc::new(AtomicBool::new(false)),
        context.clone(),
        Some(adapter),
        &mut |event| {
            if event["type"] == "output" {
                if let Some(metric) = event_metric(&event) { events.push(metric); }
            } else {
                let safe = serde_json::json!({"type":event["type"],"phase":event["phase"],"service":event["service"],"attempt":event["attempt"],"ok":event["ok"],"duration_seconds":event["duration_seconds"]});
                eprintln!("{}", common::encoded(&safe)?);
                events.push(safe);
            }
            Ok(())
        },
    )
    .await;
    common::save(
        &directory,
        "pilot-events.json",
        &serde_json::json!(events),
        true,
    )?;
    let context = context.lock().map_err(|_| anyhow::anyhow!("pilot_lock"))?;
    let report = serde_json::json!({"schema_version":1,"arm":arm,"provider":provider,"model":model,"effort":effort,"scope":"real-worker development experiment; not held-out adoption evidence","duration_seconds":started.elapsed().as_secs_f64(),"state":result.as_ref().ok(),"error":result.as_ref().err().map(|_| "native_run_failed"),"imports":context.imports,"deliveries":context.deliveries});
    common::save(&directory, "pilot-result.json", &report, true)?;
    println!("{}", common::encoded(&report)?);
    let state = result?;
    if state["status"] != "completed" {
        bail!("native_run_not_completed")
    }
    Ok(())
}
