//! Local, offline-only scheduler experiment controller. Not an installed runtime.
mod corpus;
mod driver;
mod oracle;
use infra_harness::{HarnessError, Result};
use serde_json::{Value as V, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn require(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(HarnessError::invalid(message))
    }
}
fn digest(bytes: &[u8]) -> String {
    infra_harness::store::sha(bytes)
}
fn ordinary(file: &Path, cap: u64) -> Result<Vec<u8>> {
    let meta = fs::symlink_metadata(file)?;
    require(
        meta.is_file() && !infra_core::fs::linked(&meta) && meta.len() <= cap,
        "Expected a bounded ordinary file.",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        require(meta.nlink() == 1, "Hardlinked input refused.")?;
    }
    let bytes = fs::read(file)?;
    require(bytes.len() as u64 <= cap, "File grew beyond the limit.")?;
    Ok(bytes)
}
fn executable_bytes(file: &Path) -> Result<Vec<u8>> {
    // Cargo legitimately hardlinks binaries into target/debug. Unlike mutable
    // candidate artifacts, executable custody is bound by content digest.
    let real = infra_core::fs::canonicalize(file)?;
    let meta = fs::metadata(&real)?;
    require(
        meta.is_file() && meta.len() <= 512 * 1024 * 1024,
        "Executable is not a bounded file.",
    )?;
    let bytes = fs::read(real)?;
    require(
        bytes.len() <= 512 * 1024 * 1024,
        "Executable grew beyond the limit.",
    )?;
    Ok(bytes)
}
fn write_json(file: &Path, value: &V) -> Result<()> {
    // The parent is a freshly created, controller-owned directory; candidate
    // code is never mounted there. Keep a parseable journal on interruption.
    let next = file.with_extension("next");
    fs::write(&next, serde_json::to_vec_pretty(value)?)?;
    fs::rename(next, file)?;
    Ok(())
}
fn new_directory(raw: &str) -> Result<PathBuf> {
    let requested = PathBuf::from(raw);
    let absolute = if requested.is_absolute() {
        requested
    } else {
        std::env::current_dir()?.join(requested)
    };
    let parent = infra_core::fs::canonicalize(
        absolute
            .parent()
            .ok_or_else(|| HarnessError::invalid("Output parent missing."))?,
    )?;
    let name = absolute
        .file_name()
        .ok_or_else(|| HarnessError::invalid("Output name missing."))?;
    let target = parent.join(name);
    fs::create_dir(&target)?; // Refuse existing outputs, even if empty: no retries/overwrite.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&target, fs::Permissions::from_mode(0o700))?;
    }
    Ok(target)
}
fn design(policy: &str, tools: V) -> Result<V> {
    require(
        corpus::POLICIES.contains(&policy),
        "Unknown fixture policy.",
    )?;
    let executable = executable_bytes(&std::env::current_exe()?)?;
    Ok(
        json!({"dataset":corpus::dataset(),"runner_sha256":digest(&executable),"tools":tools,
        "worker":{"kind":"offline-fixture","policy":policy,"provider_protocol":"claude","model":null,"effort":null},
        "settings":{"mode":"edit","agents_enabled":false,"project_context":false,"turn_seconds":15,"native_execution_deadline_seconds":if policy=="deadline"{1}else{40},"oracle_child_seconds":8,"oracle_supervisor_seconds":20,"oracle_child_output_bytes":65536,"oracle_supervisor_output_bytes":524288},
        "events":["create A1","wait for A1 native init","create B","accept A2 inbox","release fixture work","drain","verify final A chain then B"],
        "slots":[{"pair":0,"arm":"serial","concurrency":1},{"pair":0,"arm":"parallel","concurrency":2},{"pair":1,"arm":"parallel","concurrency":2},{"pair":1,"arm":"serial","concurrency":1}],
        "retry_policy":"never","cache_state":"not_applicable_fixture","acceptance":"runner qualification only; model efficacy not evaluated","unknown_policy":"retain planned denominator; insufficient evidence"}),
    )
}
async fn prepare(output: &str, policy: &str) -> Result<V> {
    let body = design(policy, oracle::tools().await?)?;
    let plan = json!({"schema":"harness-session-ab-plan-v1","sha256":digest(&serde_json::to_vec(&body)?),"body":body});
    let root = new_directory(output)?;
    write_json(&root.join("plan.json"), &plan)?;
    Ok(
        json!({"plan":root.join("plan.json"),"sha256":plan["sha256"],"worker":"offline-fixture","planned_streams":4,"independent_streams":1}),
    )
}
fn summary(slots: &[V], valid: bool) -> V {
    let correct = slots.iter().filter(|slot| slot["correct"] == true).count();
    let incorrect = slots.iter().filter(|slot| slot["correct"] == false).count();
    json!({"planned_streams":slots.len(),"attempted_streams":slots.iter().filter(|s|s["state"]!="unattempted").count(),"evidence_valid":valid,"observed_correct_streams":correct,"observed_incorrect_streams":incorrect,"correct_streams":if valid{Some(correct)}else{None},"incorrect_streams":if valid{Some(incorrect)}else{None},"unknown_streams":if valid{slots.len()-correct-incorrect}else{slots.len()},"independent_streams":1,"paired_unit":"whole_stream","efficacy_verdict":"not_evaluated","latency_benefit":null,"total_provider_tokens":null,"total_usd":null,"cost_per_correct":null,"provider_attempt_coverage":"fixture invocations and launch receipts only; no model billing"})
}
async fn bindings(body: &V) -> Result<bool> {
    Ok(design(
        body["worker"]["policy"].as_str().unwrap(),
        oracle::tools().await?,
    )? == *body)
}
async fn run(plan_file: &Path, output: &str) -> Result<V> {
    let plan = infra_harness::strict_json::parse(&ordinary(plan_file, 256 * 1024)?, 256 * 1024)?;
    require(
        plan["schema"] == "harness-session-ab-plan-v1",
        "Unsupported plan schema.",
    )?;
    let body = &plan["body"];
    require(
        plan["sha256"] == digest(&serde_json::to_vec(body)?),
        "Plan seal changed.",
    )?;
    let policy = body["worker"]["policy"]
        .as_str()
        .ok_or_else(|| HarnessError::invalid("Missing fixture policy."))?;
    let expected = design(policy, oracle::tools().await?)?;
    require(
        *body == expected && plan.as_object().is_some_and(|m| m.len() == 3),
        "Plan inputs/runtime/toolchain changed, or unsupported design.",
    )?;
    let root = new_directory(output)?;
    write_json(&root.join("plan.json"), &plan)?;
    let mut slots = body["slots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|slot| {
            let mut row = slot.clone();
            row["state"] = "unattempted".into();
            row["correct"] = V::Null;
            row
        })
        .collect::<Vec<_>>();
    let mut report = json!({"schema":"harness-session-ab-report-v1","plan_sha256":plan["sha256"],"worker":body["worker"],"slots":slots,"summary":summary(&slots,false),"preflight":null,"finished":false,"inputs_still_bound":null,"binding_error":null});
    write_json(&root.join("report.json"), &report)?;
    let controls = oracle::controls(&root.join("controls"), &body["tools"]).await?;
    report["preflight"] = controls.clone();
    write_json(&root.join("report.json"), &report)?;
    if controls["passed"] == true {
        for index in 0..slots.len() {
            match bindings(body).await {
                Ok(true) => {}
                other => {
                    report["binding_error"] = match other {
                        Err(error) => error.message.into(),
                        _ => "Pinned inputs changed.".into(),
                    };
                    break;
                }
            }
            slots[index]["state"] = "running".into();
            report["slots"] = json!(slots);
            report["summary"] = summary(&slots, false);
            write_json(&root.join("report.json"), &report)?;
            let arm_root = root.join(format!(
                "slot-{index}-{}",
                slots[index]["arm"].as_str().unwrap()
            ));
            let observed = driver::run(
                &arm_root,
                slots[index]["concurrency"].as_u64().unwrap() as u32,
                policy,
                &body["tools"],
                &body["settings"],
            )
            .await;
            match observed {
                Ok(observed) => {
                    slots[index]["correct"] = observed["correct"].clone();
                    slots[index]["observation"] = observed;
                    slots[index]["state"] = "finished".into();
                }
                Err(error) => {
                    slots[index]["state"] = if error.code == "pilot-deadline-proved" {
                        "failed"
                    } else {
                        "unknown"
                    }
                    .into();
                    if error.code == "pilot-deadline-proved" {
                        slots[index]["correct"] = false.into();
                    }
                    slots[index]["error_code"] = error.code.into();
                    slots[index]["error"] = error.message.into();
                }
            }
            report["slots"] = json!(slots);
            report["summary"] = summary(&slots, false);
            write_json(&root.join("report.json"), &report)?;
        }
    }
    report["inputs_still_bound"] = match bindings(body).await {
        Ok(valid) => valid.into(),
        Err(error) => {
            report["binding_error"] = error.message.into();
            false.into()
        }
    };
    let valid = report["inputs_still_bound"] == true && report["binding_error"].is_null();
    report["finished"] = true.into();
    report["summary"] = summary(&slots, valid);
    write_json(&root.join("report.json"), &report)?;
    Ok(
        json!({"report":root.join("report.json"),"preflight_passed":controls["passed"],"summary":report["summary"]}),
    )
}
async fn execute(args: &[String]) -> Result<V> {
    match args.first().map(String::as_str) {
        Some("prepare") if (2..=3).contains(&args.len()) => {
            prepare(&args[1], args.get(2).map(String::as_str).unwrap_or("gold")).await
        }
        Some("run") if args.len() == 3 => run(Path::new(&args[1]), &args[2]).await,
        _ => Err(HarnessError::invalid(
            "Use: infra-harness-ab prepare NEW_DIR [gold|no-op|partial|wrong-b|forbidden|exit-zero|deadline]; run PLAN_JSON NEW_DIR. Offline only.",
        )),
    }
}
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|arg| arg == "internal-guard") {
        std::process::exit(infra_host::process::guard_entry(&args[1..]).unwrap_or(125));
    }
    if args.first().is_some_and(|arg| arg == "--print") {
        if let Err(error) = driver::fake(&args) {
            eprintln!("{}", error.message);
            std::process::exit(2);
        }
        return;
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .build()
        .unwrap();
    match runtime.block_on(execute(&args)) {
        Ok(value) => println!("{value}"),
        Err(error) => {
            eprintln!("{}", error.message);
            std::process::exit(2);
        }
    }
}
