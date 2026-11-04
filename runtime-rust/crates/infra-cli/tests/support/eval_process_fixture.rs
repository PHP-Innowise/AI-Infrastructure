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
// Explicit fake provider for the skill-evaluation job contracts. The parent
// selects this mode only in its child environment; no provider CLI is invoked.
fn skill_eval_provider(arguments: &[String]) -> Result<i32> {
    use std::io::Write;
    let root = std::env::var_os("INFRA_SKILL_EVAL_FIXTURE_ROOT")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_dir())
        .ok_or_else(|| RuntimeError::invalid("Missing private skill-eval fixture root"))?;
    let native = arguments.first().map(String::as_str) == Some("exec");
    let query = if native {
        arguments.last()
    } else {
        arguments.get(1)
    }
    .map(String::as_str)
    .unwrap_or("");
    if ![
        "first",
        "second",
        "hit",
        "miss",
        "duplicate",
        "native-error-one",
        "native-error-two",
        "block",
    ]
    .contains(&query)
    {
        return Err(RuntimeError::invalid("Unknown skill-eval fixture query"));
    }
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("calls.jsonl"))?;
    log.write_all((serde_json::to_string(query)? + "\n").as_bytes())?;
    std::fs::write(root.join(format!("{query}.started")), b"started")?;
    if query == "first" || query == "block" {
        let marker = root.join(if query == "first" {
            "second.started"
        } else {
            "release-block"
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !marker.is_file() {
            if std::time::Instant::now() >= deadline {
                return Err(RuntimeError::unavailable(
                    "Skill-eval fixture synchronization deadline",
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    if query.starts_with("native-error-") {
        eprintln!("{query}");
        return Ok(23);
    }
    let hit = matches!(query, "first" | "hit");
    let event = if native {
        let mut marker = String::new();
        if hit {
            for entry in std::fs::read_dir(".agents/skills")? {
                let text = std::fs::read_to_string(entry?.path().join("SKILL.md"))?;
                if let Some(start) = text.find("SKILL_TRIGGERED_") {
                    marker = text[start..]
                        .chars()
                        .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
                        .collect();
                    break;
                }
            }
            if marker.is_empty() {
                return Err(RuntimeError::invalid("Native skill probe marker is absent"));
            }
        }
        json!({"type":"result","result":marker})
    } else if hit {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(".codex/commands")? {
            let path = entry?.path();
            if path.extension().is_some_and(|extension| extension == "md") {
                names.push(path.file_stem().unwrap().to_string_lossy().into_owned());
            }
        }
        json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Skill","input":{"skill":names.join(" ")}}]}})
    } else {
        json!({"type":"result"})
    };
    println!("{}", serde_json::to_string(&event)?);
    Ok(0)
}

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    // The real guardian is dispatched before any runtime or signal scope.
    if args.first().map(String::as_str) == Some("internal-guard") {
        let code = infra_harness::process::guard_entry(&args[1..]).unwrap_or(125);
        std::process::exit(code);
    }
    if std::env::var("INFRA_SKILL_EVAL_FIXTURE_MODE").as_deref() == Ok("provider-v1")
        && matches!(args.first().map(String::as_str), Some("exec" | "-p"))
    {
        let code = match skill_eval_provider(&args) {
            Ok(code) => code,
            Err(error) => {
                eprintln!("skill-eval fixture: {error}");
                2
            }
        };
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
