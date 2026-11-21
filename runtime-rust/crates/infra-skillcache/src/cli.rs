//! The command lines of the executor and the program tool.
//!
//! `skillcache-run` and `skillcache-tool` are names the engine answers to;
//! the Harness stages them for a warm session and says where the capsule, the
//! workspace and the run's state are through the environment. Each command
//! prints one JSON object on standard output.
use crate::{
    capsule::Examples,
    env_filter,
    executor::{self, Command, Refusal, Session},
    gate1,
    package::{self, Package},
    state::{RunLog, RunState},
    tool,
};
use serde_json::{Value, json};
use std::path::PathBuf;

/// The command an executable name stands for when the engine was staged
/// under it for a warm session: `skillcache-run` or `skillcache-tool`, in any
/// letter case, with or without the platform's executable suffix. Every
/// executable that answers to those names asks this one function.
pub fn staged_tool(name: Option<&std::ffi::OsStr>) -> Option<&'static str> {
    let stem = std::path::Path::new(name?).file_stem()?.to_str()?;
    if stem.eq_ignore_ascii_case("skillcache-run") {
        Some("run-command")
    } else if stem.eq_ignore_ascii_case("skillcache-tool") {
        Some("tool")
    } else {
        None
    }
}

const SUPPLIED_BYTES: usize = 256 * 1024;
const HELP: &str = "Usage: infra-engine skillcache <operation>\n\
  run ['<JSON>'] [--replay <examples.json>]   start a capsule's plan with the values known so far\n\
  continue ['<JSON>']                         supply what the last answer asked for and go on\n\
  status                                      where the run stands\n\
  tool <program> --input '<JSON>' | --check   run one program of the capsule\n\
  gate1 <package directory> [--examples <file>]   check a package's structure and dataflow\n\
A JSON argument may be `-` to read it from standard input.\n\
Environment: SKILLCACHE_PACKAGE (the capsule's folder), SKILLCACHE_WORKSPACE (default: the\n\
working directory), SKILLCACHE_STATE_FILE (the run's state), SKILLCACHE_GENERATION.\n\
A warm session stages the engine as `skillcache-run` and `skillcache-tool`.";

fn print(value: &Value) {
    println!("{value}");
}
fn refuse(refusal: &Refusal) -> i32 {
    print(&refusal.to_value());
    2
}
fn usage(message: &str) -> i32 {
    refuse(&Refusal::new("usage", message))
}
fn variable(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}
fn workspace() -> Result<PathBuf, Refusal> {
    match variable(env_filter::WORKSPACE) {
        Some(path) => Ok(path),
        None => std::env::current_dir()
            .map_err(|error| Refusal::new("unavailable", format!("No working directory: {error}"))),
    }
}
fn load_package() -> Result<Package, Refusal> {
    let root = variable(env_filter::PACKAGE).ok_or_else(|| {
        Refusal::new(
            "not_configured",
            "SKILLCACHE_PACKAGE is not set: this command runs inside a warm SkillCache session.",
        )
    })?;
    Ok(package::load(&root)?)
}
/// One JSON value from an argument, or from standard input for `-`.
fn supplied(argument: &str) -> Result<Value, Refusal> {
    let mut text = argument.to_string();
    if argument == "-" {
        use std::io::Read;
        text.clear();
        std::io::stdin()
            .lock()
            .take(SUPPLIED_BYTES as u64 + 1)
            .read_to_string(&mut text)
            .map_err(|_| Refusal::new("invalid_values", "Standard input is not UTF-8 text."))?;
    }
    infra_core::strict_json::parse(text.trim().as_bytes(), SUPPLIED_BYTES, 8, 10_000).map_err(|_| {
        Refusal::new(
            "invalid_values",
            "The values are not one JSON object. Quote it as a single argument, or pass `-` and write it to standard input.",
        )
    })
}

/// `skillcache-run run ['<JSON>'] [--replay <examples.json>]`,
/// `skillcache-run continue ['<JSON>']`, `skillcache-run status`.
///
/// Exit code 0 for every answer of the protocol, 1 for a replay that found
/// failures, 2 for a refused command.
pub fn execute_run(arguments: &[String]) -> i32 {
    let Some((operation, rest)) = arguments.split_first() else {
        return usage("Usage: skillcache-run run|continue|status ['<JSON>']");
    };
    let mut values = None;
    let mut replay = None;
    let mut rest = rest.iter();
    while let Some(argument) = rest.next() {
        if argument == "--replay" {
            match rest.next() {
                Some(path) => replay = Some(PathBuf::from(path)),
                None => return usage("--replay takes the path of the observed values."),
            }
        } else if values.is_none() && (argument == "-" || !argument.starts_with("--")) {
            match supplied(argument) {
                Ok(value) => values = Some(value),
                Err(refusal) => return refuse(&refusal),
            }
        } else {
            return usage("Usage: skillcache-run run|continue|status ['<JSON>']");
        }
    }
    let prepared = (|| {
        let package = load_package()?;
        let state_file = variable(env_filter::STATE_FILE).ok_or_else(|| {
            Refusal::new(
                "not_configured",
                "SKILLCACHE_STATE_FILE is not set: this command runs inside a warm SkillCache session.",
            )
        })?;
        Ok::<_, Refusal>((package, workspace()?, state_file))
    })();
    let (package, workspace, state_file) = match prepared {
        Ok(prepared) => prepared,
        Err(refusal) => return refuse(&refusal),
    };
    let generation = std::env::var(env_filter::GENERATION).unwrap_or_default();
    let session = Session {
        package: &package,
        workspace: &workspace,
        state_file: &state_file,
        generation: &generation,
    };
    if let Some(path) = replay {
        if operation != "run" || values.is_some() {
            return usage("A replay is `skillcache-run run --replay <examples.json>`.");
        }
        let examples = match infra_core::fs::read_regular(&path)
            .map_err(Refusal::from)
            .and_then(|bytes| {
                Examples::parse(&bytes).map_err(|violations| {
                    Refusal::new(
                        "invalid_examples",
                        violations.first().map_or(
                            "The observed values are not readable.".to_string(),
                            |violation| format!("{}: {}", violation.at, violation.message),
                        ),
                    )
                })
            }) {
            Ok(examples) => examples,
            Err(refusal) => return refuse(&refusal),
        };
        return match executor::replay(&session, &examples) {
            Ok(report) => {
                print(&report);
                i32::from(report["passed"] != true)
            }
            Err(refusal) => refuse(&refusal),
        };
    }
    let command = match operation.as_str() {
        "run" => Command::Run(values),
        "continue" => Command::Continue(values),
        "status" if values.is_none() => Command::Status,
        _ => return usage("Usage: skillcache-run run|continue|status ['<JSON>']"),
    };
    match executor::execute(&session, command) {
        Ok(answer) => {
            print(&answer);
            0
        }
        Err(refusal) => refuse(&refusal),
    }
}

/// `skillcache-tool <program> --input '<JSON>'` or `skillcache-tool <program> --check`.
///
/// On success the program's one JSON object is printed and the exit code is 0.
/// A failed program is exit code 1 with a bounded, redacted diagnostic on
/// standard error; a refused command is exit code 2.
pub fn execute_tool(arguments: &[String]) -> i32 {
    let fail = |code: i32, message: String| {
        eprintln!("skillcache-tool: {message}");
        code
    };
    let (id, check, input) = match arguments {
        [id, flag] if flag == "--check" => (id, true, None),
        [id, flag, input] if flag == "--input" => (id, false, Some(input)),
        _ => {
            return fail(
                2,
                "usage: skillcache-tool <program> --input '<JSON>' | --check".into(),
            );
        }
    };
    let package = match load_package() {
        Ok(package) => package,
        Err(refusal) => return fail(2, refusal.message),
    };
    if let Some(violation) = gate1::structure(&package.capsule, &package.files).first() {
        return fail(
            2,
            format!(
                "the capsule does not pass its structural gate ({} at {})",
                violation.code, violation.at
            ),
        );
    }
    let Some(program) = package.capsule.program(id) else {
        return fail(2, format!("the capsule has no program `{id}`"));
    };
    let state_file = variable(env_filter::STATE_FILE);
    // A program that failed in this run is the agent's step now; it is not
    // started again, neither by the executor nor by hand.
    if let Some(state_file) = &state_file {
        match RunState::load(state_file) {
            Ok(Some(state)) => {
                let failed = package
                    .capsule
                    .plan()
                    .enumerate()
                    .any(|(index, step)| step.program() == Some(id) && state.is_failed(index));
                if failed {
                    return fail(
                        2,
                        format!(
                            "refused_failed_program: `{id}` failed in this run and is not run again; do the step yourself and continue with `skillcache-run continue`"
                        ),
                    );
                }
            }
            Ok(None) => {}
            Err(error) => return fail(2, error.message),
        }
    }
    let workspace = match workspace() {
        Ok(workspace) => workspace,
        Err(refusal) => return fail(2, refusal.message),
    };
    let inputs = match input.map(|input| supplied(input)) {
        Some(Ok(Value::Object(inputs))) => inputs,
        Some(_) => return fail(2, "--input takes one JSON object".into()),
        None => Default::default(),
    };
    let mode = if check {
        tool::Mode::Check
    } else {
        tool::Mode::Run(&inputs)
    };
    let log_file = (!check && state_file.is_none())
        .then(|| variable(env_filter::LOG_FILE))
        .flatten();
    let step_index = package
        .capsule
        .plan()
        .position(|step| step.program() == Some(id));
    let verification = step_index.is_some_and(|index| package.capsule.is_verification(index));
    let outcome = tool::run(&package, program, mode, &workspace);
    if let Some(log_file) = log_file
        && outcome.is_ok()
    {
        let _lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(log_file.with_extension("lock"))
            .ok()
            .inspect(|file| {
                let _ = file.lock();
            });
        let generation = std::env::var(env_filter::GENERATION).unwrap_or_default();
        if let Ok(mut log) = RunLog::open_log(log_file, &generation) {
            match &outcome {
                Ok(tool::Outcome::Completed(completed)) => {
                    let _ = log.append(
                        "program_completed",
                        json!({
                            "step_index": step_index,
                            "program": program.id,
                            "seconds": completed.seconds,
                        }),
                    );
                }
                Ok(tool::Outcome::Failed(failed)) => {
                    let _ = log.append(
                        "program_failed",
                        json!({
                            "step_index": step_index,
                            "program": program.id,
                            "reason": failed.reason,
                            "verification": verification,
                        }),
                    );
                }
                Err(_) => {}
            }
        }
    }
    match outcome {
        Ok(tool::Outcome::Completed(completed)) => {
            print(&json!({"status": "complete", "result": completed.result}));
            0
        }
        Ok(tool::Outcome::Failed(failed)) => {
            fail(1, format!("{}: {}", failed.reason, failed.diagnostics))
        }
        Err(error) => fail(2, error.message),
    }
}

/// `gate1 <package directory> [--examples <examples.json>]`: Gate 1 over a
/// package on disk. Exit code 0 when it passes, 1 when it does not.
pub fn execute_gate1(arguments: &[String]) -> i32 {
    let (root, examples) = match arguments {
        [root] => (root, None),
        [root, flag, examples] if flag == "--examples" => (root, Some(examples)),
        _ => return usage("Usage: skillcache gate1 <package directory> [--examples <file>]"),
    };
    let read = (|| {
        let root = PathBuf::from(root);
        let package = package::load(&root)?;
        let capsule = infra_core::fs::read_regular(&root.join(package::CAPSULE_FILE))?;
        let examples = examples
            .map(|path| infra_core::fs::read_regular(std::path::Path::new(path)))
            .transpose()?;
        Ok::<_, infra_core::RuntimeError>((package, capsule, examples))
    })();
    let (package, capsule, examples) = match read {
        Ok(read) => read,
        Err(error) => return refuse(&Refusal::from(error)),
    };
    let report = match package.markdown.as_deref() {
        Some(markdown) => gate1::check_textual(&capsule, markdown.as_bytes(), None),
        None => gate1::check(
            &capsule,
            &package.files,
            examples.as_deref(),
            &gate1::Context::default(),
        ),
    };
    print(&serde_json::to_value(&report).unwrap_or_default());
    i32::from(!report.passed)
}

/// The engine's `skillcache` command group and its two staged names.
/// `name` is `run`, `continue`, `status`, `tool` or `gate1`.
pub fn execute(name: &str, arguments: &[String]) -> i32 {
    if name.is_empty() || matches!(name, "--help" | "-h" | "help") {
        println!("{HELP}");
        return 0;
    }
    match name {
        "run" | "continue" | "status" => {
            let mut all = vec![name.to_string()];
            all.extend_from_slice(arguments);
            execute_run(&all)
        }
        "tool" => execute_tool(arguments),
        "gate1" => execute_gate1(arguments),
        _ => usage("Usage: skillcache run|continue|status|tool|gate1"),
    }
}
