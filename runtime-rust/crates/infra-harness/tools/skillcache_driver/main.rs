//! Real Harness-backed SkillCache evaluation pilot driver.
//!
//! Reads one JSON request (`prepare` or `attempt`) on standard input, drives
//! isolated in-process Harness sessions/compilations under `state_root`, and
//! writes one JSON answer to standard output. Also dispatches `skillcache-run`
//! / `skillcache-tool` and `internal-*` custody/sandbox commands so standalone
//! tests work without a separate `infra-engine` binary.
mod arms;
mod config;
mod usage;
mod workspace;

use infra_core::fs as core_fs;
use infra_harness::{HarnessError, Result, strict_json};
use serde_json::Value;
use std::{io::Read, path::PathBuf};

const REQUEST_BYTES: usize = 1024 * 1024;

fn read_request() -> Result<Value> {
    let mut raw = Vec::new();
    std::io::stdin()
        .take((REQUEST_BYTES + 1) as u64)
        .read_to_end(&mut raw)?;
    if raw.len() > REQUEST_BYTES {
        return Err(HarnessError::invalid("Driver request exceeds byte limit."));
    }
    strict_json::parse(&raw, REQUEST_BYTES)
}

fn parse_protected(request: &Value) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let Some(items) = request.get("protected").and_then(Value::as_array) else {
        return Ok(out);
    };
    for item in items {
        let raw = item
            .as_str()
            .filter(|s| !s.trim().is_empty() && !s.contains('\0'))
            .ok_or_else(|| HarnessError::invalid("`protected` entries must be path strings."))?;
        let path = PathBuf::from(raw);
        if !path.is_absolute() {
            return Err(HarnessError::invalid(
                "`protected` entries must be absolute paths.",
            ));
        }
        out.push(core_fs::canonical_or_missing(&path)?);
    }
    Ok(out)
}

fn run_driver() -> Result<Value> {
    let request = read_request()?;
    let protected = parse_protected(&request)?;
    let cfg = config::load(request["inputs"].as_str(), &protected)?;
    infra_host::process::use_guardian(cfg.engine.clone())?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .thread_stack_size(8 * 1024 * 1024)
        .enable_all()
        .worker_threads(2)
        .build()?;
    runtime.block_on(arms::handle(&cfg, &request, &protected))
}

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str) != Some("internal-guard")
        && let Some(kind) =
            infra_skillcache::cli::staged_tool(std::env::args_os().next().as_deref())
    {
        let code = match kind {
            "run-command" => infra_skillcache::cli::execute_run(&args),
            other => infra_skillcache::cli::execute(other, &args),
        };
        std::process::exit(code);
    }
    match args.first().map(String::as_str) {
        Some("internal-guard") => {
            std::process::exit(infra_host::process::guard_entry(&args[1..]).unwrap_or(125));
        }
        Some("internal-discovery-probe") => {
            std::process::exit(infra_host::sandbox::probe_entry(&args[1..]).unwrap_or(2));
        }
        Some("internal-sandbox-child") => {
            std::process::exit(infra_host::sandbox::child_entry(&args[1..]).unwrap_or(2));
        }
        Some("skillcache") => {
            let code = infra_skillcache::cli::execute(
                args.get(1).map_or("", String::as_str),
                args.get(2..).unwrap_or(&[]),
            );
            std::process::exit(code);
        }
        _ => {}
    }
    match run_driver() {
        Ok(answer) => println!("{answer}"),
        Err(error) => {
            eprintln!("skillcache-driver: {}", error.message);
            std::process::exit(1);
        }
    }
}
