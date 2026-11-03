//! Noninstalled process driver for explicit integration-test requests.
use infra_eval::{Result, RuntimeError, artifact, process};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf, time::Duration};

const REQUEST_LIMIT: usize = 8 * 1024 * 1024;
const OUTPUT_LIMIT: usize = 8 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BudgetRequest {
    milliseconds: u64,
    cap: usize,
    max_bytes: usize,
    native: bool,
    strict_events: bool,
    error_diagnostics_terminal: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    argv: Vec<String>,
    cwd: PathBuf,
    env: BTreeMap<String, String>,
    input: Vec<u8>,
    budget: BudgetRequest,
    stdout: PathBuf,
    stderr: PathBuf,
}
fn run(path: &std::path::Path) -> Result<Value> {
    let value = artifact::read_json(path, REQUEST_LIMIT)?;
    let request: Request = serde_json::from_value(value)?;
    if request.budget.milliseconds == 0
        || request.budget.milliseconds > 120_000
        || request.budget.max_bytes == 0
        || request.budget.max_bytes > OUTPUT_LIMIT
        || request.input.len() > REQUEST_LIMIT
    {
        return Err(RuntimeError::invalid(
            "Fixture budget is outside its explicit bounds",
        ));
    }
    let _signals = infra_eval::cancel::Scope::install()?;
    let session = process::execute(
        &process::CommandSpec {
            argv: request.argv,
            cwd: request.cwd,
            env: request.env,
            input: request.input,
        },
        &process::Budget {
            timeout: Duration::from_millis(request.budget.milliseconds),
            cap: request.budget.cap,
            max_bytes: request.budget.max_bytes,
            native: request.budget.native,
            strict_events: request.budget.strict_events,
            error_diagnostics_terminal: request.budget.error_diagnostics_terminal,
        },
        Some(&request.stdout),
        Some(&request.stderr),
        |_, _| None,
    )?;
    Ok(json!({"ok":true,"session":session.value()}))
}
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    // The real guardian is dispatched before any runtime or signal scope.
    if args.first().map(String::as_str) == Some("internal-guard") {
        let code = infra_harness::process::guard_entry(&args[1..]).unwrap_or(125);
        std::process::exit(code);
    }
    let result = match args.as_slice() {
        [mode, request] if mode == "execute" => run(std::path::Path::new(request)),
        _ => Err(RuntimeError::invalid("Unknown fixture operation")),
    };
    let (receipt, code) = match result {
        Ok(receipt) => (receipt, 0),
        Err(error) => (
            json!({"ok":false,"error":{"message":error.message,"kind":format!("{:?}",error.kind),"code":error.code,"os_code":error.os_code}}),
            2,
        ),
    };
    println!("{}", serde_json::to_string(&receipt).unwrap());
    std::process::exit(code);
}
