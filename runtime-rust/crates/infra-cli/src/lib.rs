//! One native executable over the shared Rust services. No Python fallbacks.
pub mod arguments;
pub mod memory;
pub mod tooling;
pub use infra_adapters::{bank, counters, dispatch, generator_gate, hooks, output, process, shell};
use infra_core::{Result, RuntimeError};
use output::Output;
use serde_json::{Value, json};
use std::{
    io::{IsTerminal, Read},
    path::{Path, PathBuf},
};
const GROUPS: &[&str] = &[
    "memory",
    "bank",
    "hook",
    "mcp",
    "repo",
    "install",
    "attach",
    "kit",
    "collect",
    "creator",
    "system",
    "harness",
    "serve",
    "app",
    "eval",
    "qa",
    "pilot",
    "bench",
    "internal",
    "continuity",
    "dispatch",
    "dispatch-install",
    "capabilities",
    "skill-tools",
    "routing-eval",
    "release",
];
fn input(limit: usize, needed: bool) -> Result<Vec<u8>> {
    if !needed {
        return Ok(vec![]);
    }
    let stdin = std::io::stdin();
    let mut bytes = vec![];
    stdin
        .lock()
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(RuntimeError::invalid(
            "Standard input exceeds the command limit",
        ));
    }
    Ok(bytes)
}
/// Hook input within the command limit. A memory role given more passes the
/// turn (`None`): a refusal there would block the person's prompt or stop.
fn hook_input(host: &str, role: &str) -> Result<Option<Vec<u8>>> {
    match input(4 * 1024 * 1024, !std::io::stdin().is_terminal()) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error)
            if error.kind == infra_core::ErrorKind::InvalidInput
                && hooks::MEMORY_ROLES.contains(&role)
                && ["claude", "codex", "cursor"].contains(&host) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}
fn inherited(code: i32) -> Output {
    Output {
        code,
        ..Default::default()
    }
}
pub fn source_root(request: &Value) -> Result<PathBuf> {
    if let Some(root) = request["source_root"]
        .as_str()
        .or_else(|| request["tooling_root"].as_str())
    {
        std::fs::metadata(root)?;
        return infra_core::fs::python_realpath(Path::new(root));
    }
    if let Some(root) = std::env::var_os("INFRA_SOURCE_ROOT")
        .map(PathBuf::from)
        .filter(|root| root.is_absolute())
        && root.join("runtime-rust/Cargo.toml").is_file()
    {
        return infra_core::fs::python_realpath(&root);
    }
    let executable = std::env::current_exe()?;
    let current = std::env::current_dir()?;
    for root in executable.ancestors().chain(current.ancestors()) {
        if root.join("runtime-rust/Cargo.toml").is_file() {
            return infra_core::fs::python_realpath(root);
        }
    }
    Err(RuntimeError::unavailable(
        "Repository tooling root unavailable; pass --source-root",
    ))
}
fn source_arguments(raw: &[String]) -> Result<(Value, Vec<String>)> {
    let mut request = json!({});
    let mut arguments = vec![];
    let mut at = 0;
    let mut ended = false;
    while at < raw.len() {
        let item = &raw[at];
        if item == "--" {
            ended = true;
            arguments.push(item.clone());
            at += 1;
            continue;
        }
        let (name, inline) = item
            .split_once('=')
            .map(|(name, value)| (name, Some(value)))
            .unwrap_or((item.as_str(), None));
        if !ended && ["--source-root", "--tooling-root"].contains(&name) {
            let value = if let Some(value) = inline {
                value.to_string()
            } else {
                at += 1;
                raw.get(at)
                    .filter(|value| !value.starts_with("--"))
                    .cloned()
                    .ok_or_else(|| RuntimeError::invalid(format!("{name} requires a path")))?
            };
            request[name.trim_start_matches('-').replace('-', "_")] = value.into();
        } else {
            arguments.push(item.clone());
        }
        at += 1;
    }
    Ok((request, arguments))
}
fn source_request(mut value: Value) -> Result<Value> {
    if value["root"].is_null() {
        value["root"] = source_root(&value)?.display().to_string().into();
    }
    Ok(value)
}
fn eval_request(mut value: Value) -> Result<Value> {
    if value["source_root"].is_string() || value["tooling_root"].is_string() {
        value["tooling_root"] = source_root(&value)?.display().to_string().into();
    } else if let Ok(root) = source_root(&value) {
        value["tooling_root"] = root.display().to_string().into();
    }
    Ok(value)
}
fn parse(raw: &[String], operation: &str) -> Result<arguments::Arguments> {
    arguments::parse(raw, operation)
}
fn operation<'a>(
    arguments: &'a [String],
    default: Option<&'a str>,
) -> Result<(&'a str, &'a [String])> {
    if let Some((first, rest)) = arguments.split_first()
        && !first.starts_with('-')
    {
        return Ok((first, rest));
    }
    default
        .map(|value| (value, arguments))
        .ok_or_else(|| RuntimeError::invalid("A command operation is required"))
}
fn cli_options(request: &mut Value) {
    let options = [
        "dry_run",
        "overwrite",
        "merge_existing",
        "update_tracked",
        "rewire_codex",
        "allow_downgrade",
    ]
    .into_iter()
    .filter_map(|field| {
        request
            .get(field)
            .map(|value| (field.to_string(), value.clone()))
    })
    .collect::<serde_json::Map<_, _>>();
    request["options"] = Value::Object(options);
}
fn singular(request: &mut Value, key: &str) -> Result<()> {
    if let Some(values) = request[key].as_array() {
        if values.len() != 1 {
            return Err(RuntimeError::invalid(format!(
                "Use one --{key} for this operation"
            )));
        }
        request[key] = values[0].clone();
    }
    Ok(())
}
fn help() -> Output {
    Output::text(format!(
        "Usage: infra-engine <command> [options]\nCommands: {}\n\nInstalled memory: memory-bank/bin/infra-engine memory <command>\n",
        GROUPS.join(", ")
    ))
}
fn memory_help() -> Output {
    Output::text(format!(
        "Usage: infra-engine memory [--root ROOT] [--db DB] [--project PROJECT] [--mode MODE] [--owner OWNER] <command> [options]\nCommands: {}\n",
        arguments::MEMORY_COMMANDS.join(", ")
    ))
}
pub fn capabilities() -> Value {
    // Embedded by build.rs; empty when the build could not read its sources.
    let source = env!("INFRA_ENGINE_SOURCE_SHA256");
    let source = (!source.is_empty())
        .then(|| json!({"scheme":infra_tooling::source_identity::SCHEME,"sha256":source}));
    let mut descriptor = json!({"engine":"infra-engine","version":env!("CARGO_PKG_VERSION"),"capability_profile":"infra-engine-v1","capabilities":["memory","brain","bank","hooks","mcp","install","attach","kit","collect","creator","system","harness","serve","app","eval","qa","pilot","bench","continuity","skill-tools","routing-eval","release","memory-queue-worker-v1","memory-eval-clock-v1","hook-policy-v1"],"source":source});
    // Report dependency truth: workspace feature unification can enable ONNX
    // in pilots even when this package's forwarding feature was not selected.
    if infra_pilots::onnx::compiled() {
        descriptor["capabilities"]
            .as_array_mut()
            .expect("the native capability descriptor contains an array")
            .push("onnx-inference-v1".into());
    }
    descriptor
}
fn memory_command(rest: &[String], controlled: bool) -> Result<Output> {
    let arguments = match arguments::memory(rest) {
        Ok(arguments) => arguments,
        Err(error) => {
            return Ok(Output::error(
                2,
                format!("infra-engine memory: error: {error}"),
            ));
        }
    };
    if arguments.operation == "help" || arguments.value["help"] == true {
        return Ok(memory_help());
    }
    let operation = &arguments.operation;
    // A skipped native result carries no data: its command runs without a
    // pipe and must not wait on a standard input the host left open.
    let needs_input = matches!(
        operation.as_str(),
        "native-prompt" | "native-stop" | "cursor-rule"
    ) || operation == "native-result" && arguments.value["skip"] != true
        || arguments.value["input"] == "-"
        || operation == "capsule" && arguments.value["file"] == "-";
    let needed = needs_input
        && (!std::io::stdin().is_terminal()
            || arguments.value["input"] == "-"
            || arguments.value["file"] == "-");
    let input = input(4 * 1024 * 1024, needed)?;
    if controlled {
        memory::execute_in_eval(arguments, &input)
    } else {
        memory::execute(arguments, &input)
    }
}
pub fn run(raw: &[String]) -> Result<Output> {
    // The trusted watchdog must run before clock/config/runtime initialization.
    if raw.first().map(String::as_str) == Some("internal-guard") {
        let code = infra_harness::process::guard_entry(&raw[1..])
            .map_err(|error| RuntimeError::unavailable(error.to_string()))?;
        return Ok(inherited(code));
    }
    if raw.first().map(String::as_str) == Some("internal")
        && raw.get(1).map(String::as_str) == Some("guard")
    {
        let code = infra_harness::process::guard_entry(&raw[2..])
            .map_err(|error| RuntimeError::unavailable(error.to_string()))?;
        return Ok(inherited(code));
    }
    let _source_frame = infra_core::git::source_frame_scope();
    if raw.is_empty() || matches!(raw[0].as_str(), "--help" | "-h" | "help") {
        return Ok(help());
    }
    if raw[0] == "--version" {
        return Ok(Output::text(format!(
            "infra-engine {}\n",
            env!("CARGO_PKG_VERSION")
        )));
    }
    // Historical calendars are explicit and affect only this same-thread native
    // memory call. Public commands never import a calendar from the environment.
    if raw[0] == "__memory-eval-runtime" {
        if raw.len() < 6
            || raw[1] != "--clock"
            || raw[3] != "--"
            || raw[4] != "memory"
            || raw[2].is_empty()
            || raw[2].len() > 128
        {
            return Err(RuntimeError::invalid(
                "Expected __memory-eval-runtime --clock ISO -- memory <canonical arguments>",
            ));
        }
        let anchor = infra_core::domain::clock::parse_python_anchor(&raw[2])?;
        let _calendar = infra_core::domain::clock::enter(
            infra_core::domain::clock::WallClock::Offset(anchor - chrono::Utc::now()),
        );
        return memory_command(&raw[5..], true);
    }
    // These finite workers own their input contracts; they precede all generic
    // option parsing and never reuse user-selected runtime/database overrides.
    if raw[0] == "__memory-queue-capabilities" {
        if raw.len() != 1 {
            return Err(RuntimeError::invalid(
                "Queue capabilities takes no arguments",
            ));
        }
        return Output::json(&infra_core::queue::worker_capabilities());
    }
    if raw[0] == "__desktop-shortcut-worker" {
        if raw.len() != 1 {
            return Err(RuntimeError::invalid("Invalid desktop worker arguments"));
        }
        let input = input(1024 * 1024, true)?;
        return Ok(match infra_harness::desktop::shortcut_worker(&input) {
            Ok(stdout) => Output {
                stdout,
                ..Default::default()
            },
            Err(error) => Output::error(
                if error.code == "desktop-staging-custody" {
                    74
                } else {
                    1
                },
                error.message,
            ),
        });
    }
    if raw[0] == "__memory-queue-worker" {
        if raw.len() != 5 || raw[1] != "--tooling-root" || raw[3] != "--project-root" {
            return Err(RuntimeError::invalid("Invalid queue worker arguments"));
        }
        let input = input(1024 * 1024, true)?;
        return Output::json(&infra_core::queue::worker_entry(
            Path::new(&raw[2]),
            Path::new(&raw[4]),
            &input,
        )?);
    }
    for worker in [
        infra_eval::intake::worker as fn(&[String]) -> Result<Option<i32>>,
        infra_eval::onnx_adapter::worker,
        infra_eval::bench_driver::worker,
        infra_eval::treatments::worker,
    ] {
        if let Some(code) = worker(raw)? {
            return Ok(inherited(code));
        }
    }
    let group = raw[0].as_str();
    let rest = &raw[1..];
    if ![
        "memory",
        "qa",
        "pilot",
        "onnx-worker",
        "harness",
        "serve",
        "app",
        "internal-sandbox-child",
        "internal-discovery-probe",
    ]
    .contains(&group)
        && rest
            .iter()
            .take_while(|argument| argument.as_str() != "--")
            .any(|argument| argument == "--help" || argument == "-h")
    {
        let nested = rest
            .first()
            .filter(|argument| !argument.starts_with('-'))
            .map(String::as_str);
        let route = match group {
            "repo" | "creator" | "kit" | "attach" | "system" | "skill-tools" | "bank"
            | "release" => nested
                .map(|operation| format!("{group}/{operation}"))
                .unwrap_or_else(|| group.into()),
            "eval" | "bench" => nested
                .map(|family| {
                    let family = match (group, family) {
                        ("bench", "codex") => "codex-bench",
                        ("bench", "claude" | "claude-code") => "claude-code-bench",
                        _ => family,
                    };
                    let command = if group == "eval"
                        && ["memory-eval", "memory-outcome-ab", "memory-native-ab"]
                            .contains(&family)
                    {
                        rest.get(1).filter(|command| !command.starts_with('-'))
                    } else {
                        None
                    };
                    command
                        .map(|command| format!("eval/{family}/{command}"))
                        .unwrap_or_else(|| format!("eval/{family}"))
                })
                .unwrap_or_else(|| group.into()),
            other => other.into(),
        };
        let options = if nested.is_some() { &rest[1..] } else { rest };
        if arguments::help_requested(options, &route)? {
            return Ok(Output::text(arguments::option_help(&route)?));
        }
    }
    match group {
        "capabilities" => {
            if rest.iter().any(|argument| argument != "--json") {
                return Err(RuntimeError::invalid("Unknown capabilities option"));
            }
            Output::json(&capabilities())
        }
        "memory" => memory_command(rest, false),
        "bank" => {
            let (op, args) = operation(rest, Some("validate"))?;
            let request = parse(args, &format!("bank/{op}"))?.value;
            let context = if let Some(project) = request["source_root"].as_str() {
                let root = request["root"]
                    .as_str()
                    .map(PathBuf::from)
                    .unwrap_or(std::env::current_dir()?);
                std::fs::metadata(&root)?;
                std::fs::metadata(project)?;
                let root = infra_core::fs::python_realpath(&root)?;
                let project = infra_core::fs::python_realpath(Path::new(project))?;
                if !root.is_dir() || !project.is_dir() {
                    return Err(RuntimeError::invalid(
                        "Bank and canonical source roots must be directories",
                    ));
                }
                infra_core::RuntimeContext::new(
                    infra_core::Roots {
                        tooling: root.clone(),
                        state: root,
                        project,
                        shared: false,
                    },
                    request["owner"].as_str().unwrap_or("local"),
                    request["mode"].as_str().unwrap_or("governed"),
                )?
            } else {
                memory::context(&request)?
            };
            bank::execute(context, op, &request)
        }
        "hook" => {
            if rest.len() < 2 {
                return Err(RuntimeError::invalid("hook requires HOST ROLE"));
            }
            let (host, role) = (&rest[0], &rest[1]);
            let request = parse(&rest[2..], "hook")?.value;
            // An explicit root is used as given: resolving the fallback eagerly
            // failed for a missing root and let the shell guard's command run.
            let root = match request["root"].as_str() {
                Some(root) => PathBuf::from(root),
                None => memory::tooling(&request)?,
            };
            let Some(bytes) = hook_input(host, role)? else {
                return Ok(hooks::passed(host, role));
            };
            hooks::execute(&root, host, role, &bytes)
        }
        "hook-find" => {
            if rest.len() != 2 {
                return Err(RuntimeError::invalid("hook-find requires HOST ROLE"));
            }
            let root = std::env::current_dir()?
                .ancestors()
                .find(|root| root.join(format!(".{}/hooks.json", rest[0])).is_file())
                .map(Path::to_path_buf)
                .ok_or_else(|| {
                    RuntimeError::unavailable("No host hook wiring above the working directory")
                })?;
            let Some(bytes) = hook_input(&rest[0], &rest[1])? else {
                return Ok(hooks::passed(&rest[0], &rest[1]));
            };
            hooks::execute(&root, &rest[0], &rest[1], &bytes)
        }
        "mcp" => {
            let request = parse(rest, "mcp")?.value;
            let context = memory::context(&request)?;
            let host = request["host"].as_str().unwrap_or("codex");
            let server = infra_core::McpServer::new(context, host)?;
            server.serve(std::io::stdin().lock(), std::io::stdout().lock())?;
            Ok(Output::default())
        }
        "continuity" => {
            let request = parse(rest, "continuity")?.value;
            let context = memory::context(&request)?;
            let host = request["host"]
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("--host required"))?;
            let event = request["event"]
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("--event required"))?;
            let bytes = input(1024 * 1024, !std::io::stdin().is_terminal())?;
            Output::envelope(infra_core::chat_continuity::execute(
                &context, host, event, &bytes, &request,
            )?)
        }
        "dispatch" => {
            let (host, args) = operation(rest, None)?;
            let request = parse(args, "dispatch")?.value;
            let store = request["store"]
                .as_str()
                .map(PathBuf::from)
                .ok_or_else(|| RuntimeError::invalid("--store required"))?;
            let hook = request["_positionals"]
                .as_array()
                .and_then(|args| args.first())
                .and_then(Value::as_str)
                .ok_or_else(|| RuntimeError::invalid("Dispatcher role required"))?;
            let bytes = input(1024 * 1024, !std::io::stdin().is_terminal())?;
            dispatch::run(host, &store, hook, &bytes)
        }
        "dispatch-install" => {
            let (host, args) = operation(rest, None)?;
            let request = parse(args, "dispatch-install")?.value;
            let root = match request["root"].as_str() {
                Some(root) => PathBuf::from(root),
                None => memory::tooling(&request)?,
            };
            Output::json(&dispatch::install(&root, host, None)?)
        }
        "repo" => {
            let (op, args) = operation(rest, None)?;
            let mut request = source_request(parse(args, &format!("repo/{op}"))?.value)?;
            cli_options(&mut request);
            if [
                "install",
                "overlay",
                "install-plan",
                "install-apply",
                "sync",
            ]
            .contains(&op)
                && request["engine"].is_null()
            {
                request["engine"] = std::env::current_exe()?.display().to_string().into();
            }
            if op == "registry" {
                let positionals = request["_positionals"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                if let Some(action) = positionals.first() {
                    request["action"] = action.clone();
                }
                if request["id"].is_null() {
                    request["id"] = positionals.get(1).cloned().unwrap_or(Value::Null);
                }
            }
            if op == "routing-eval" {
                if request["source_root"].is_null() && request["tooling_root"].is_null() {
                    request["source_root"] = request["root"].clone();
                }
                let request = routing_request(request)?;
                return Output::envelope(infra_skill_tools::execute("routing-eval", &request)?);
            }
            if op == "build-ui" {
                let result = infra_tooling::ui_build::build(
                    Path::new(request["root"].as_str().unwrap_or(".")),
                    &request,
                )?;
                return tooling::reply("build-ui", &request, result);
            }
            let op = if op == "sync" { "core-sync" } else { op };
            tooling::execute(op, request)
        }
        "install" => {
            let (sub, args) = if rest
                .first()
                .is_some_and(|arg| ["plan", "apply", "preview-images"].contains(&arg.as_str()))
            {
                (rest[0].as_str(), &rest[1..])
            } else {
                ("", rest)
            };
            let mut request = source_request(parse(args, "install")?.value)?;
            singular(&mut request, "edition")?;
            cli_options(&mut request);
            tooling::normalise(&mut request)?;
            if request["engine"].is_null() {
                request["engine"] = std::env::current_exe()?.display().to_string().into();
            }
            let modes = [
                request["sync"] == true,
                request["verify_inventories"] == true,
                request["write_inventories"] == true,
                sub.is_empty() && !request["edition"].is_null(),
                !sub.is_empty(),
            ];
            if modes.iter().filter(|mode| **mode).count() != 1 {
                return Err(RuntimeError::invalid(
                    "Choose exactly one installation mode: --edition, --sync, --verify-inventories, --write-inventories or plan/apply/preview-images",
                ));
            }
            if request["overwrite"] == true && request["merge_existing"] == true {
                return Err(RuntimeError::invalid(
                    "--overwrite and --merge-existing are mutually exclusive",
                ));
            }
            if !request["inventory_out"].is_null() && request["write_inventories"] != true {
                return Err(RuntimeError::invalid(
                    "--inventory-out requires --write-inventories",
                ));
            }
            if request["sync"] != true
                && sub.is_empty()
                && ["rewire_codex", "update_tracked", "allow_downgrade"]
                    .iter()
                    .any(|key| request[*key] == true)
            {
                return Err(RuntimeError::invalid(
                    "Rewire/update/downgrade options require --sync",
                ));
            }
            let op = match sub {
                "plan" => "install-plan",
                "apply" => "install-apply",
                "preview-images" => "install-preview-images",
                _ => {
                    if request["sync"] == true {
                        "sync"
                    } else if request["verify_inventories"] == true {
                        "inventory-check"
                    } else if request["write_inventories"] == true {
                        "inventory-write"
                    } else {
                        "install"
                    }
                }
            };
            // Only the explicit terminal installer has this continuation.
            // Library/repository services and scripted installs stay project-local.
            let register = op == "install"
                && request["dry_run"] != true
                && std::io::stdin().is_terminal()
                && std::io::stderr().is_terminal();
            let target = request["target"].as_str().map(PathBuf::from);
            let tools = tooling::strings(&request["tools"])?;
            let mut output = tooling::execute(op, request)?;
            if register
                && output.code == 0
                && let Some(target) = target
            {
                for (host, label) in [
                    ("claude", "CLAUDE\tbelow-root"),
                    ("codex", "CODEX\tworktrees"),
                ] {
                    if !tools.iter().any(|tool| tool == host) {
                        continue;
                    }
                    // Registration is best effort, as the original terminal
                    // installer was. It never approves or trusts host hooks.
                    if let Ok(result) = dispatch::install(&target, host, None)
                        && result["state"] == "registered"
                    {
                        let folder = dispatch::home(host).join("ai-accelerator");
                        let suffix = if host == "codex" {
                            "\ttrust its hooks once in Codex (/hooks)"
                        } else {
                            ""
                        };
                        output.stderr.extend_from_slice(
                            format!("{label}\t{}{suffix}\n", folder.display()).as_bytes(),
                        );
                    }
                }
            }
            Ok(output)
        }
        "attach" => {
            let (action, args) = operation(rest, Some("overlay"))?;
            let mut request = source_request(parse(args, &format!("attach/{action}"))?.value)?;
            request["action"] = action.into();
            singular(&mut request, "edition")?;
            singular(&mut request, "project")?;
            let mut positionals = request["_positionals"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if action == "run" {
                if request["tool"].is_null() {
                    request["tool"] = positionals.first().cloned().unwrap_or(Value::Null);
                    if !positionals.is_empty() {
                        positionals.remove(0);
                    }
                }
                if request["tool"].is_array() {
                    singular(&mut request, "tool")?;
                }
                request["arguments"] = Value::Array(positionals);
            } else if !positionals.is_empty() {
                return Err(RuntimeError::invalid(
                    "Passthrough arguments are accepted only by attach run",
                ));
            }
            let result = infra_tooling::execute("attach", &request)?;
            tooling::reply(&format!("attach-{action}"), &request, result)
        }
        "kit" => {
            let (action, args) = operation(rest, Some("catalog"))?;
            let mut request = source_request(parse(args, &format!("kit/{action}"))?.value)?;
            request["action"] = action.into();
            let positionals = request["_positionals"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            match action {
                "find" | "search" => {
                    if request["query"].is_null() {
                        request["query"] = positionals
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(" ")
                            .into();
                    }
                }
                "show" => {
                    if request["id"].is_null() {
                        request["id"] = positionals.first().cloned().unwrap_or(Value::Null);
                    }
                }
                "record" | "select" => {
                    if request["ids"].is_null() {
                        request["ids"] = Value::Array(positionals.clone());
                    }
                }
                "add" | "install" => {
                    if request["source"].is_null() {
                        request["source"] = positionals.first().cloned().unwrap_or(Value::Null);
                    }
                }
                "remove" | "uninstall" | "update" => {
                    request["names"] = Value::Array(positionals.clone());
                }
                _ => {}
            }
            if !request["pin"].is_null() {
                request["pins"] = request["pin"].clone();
            }
            if request["global"] == true {
                request["global_scope"] = true.into();
            }
            tooling::execute("kit", request)
        }
        "collect" => {
            let (scope, args) = operation(rest, Some(""))?;
            let mut request = source_request(parse(args, "collect")?.value)?;
            if !scope.is_empty() {
                request["scope"] = scope.into();
            }
            singular(&mut request, "edition")?;
            if request["paths"].is_null() {
                request["paths"] = request
                    .get("path")
                    .cloned()
                    .unwrap_or_else(|| request["_positionals"].clone());
            }
            tooling::execute("collect", request)
        }
        "creator" => {
            let (op, args) = operation(rest, None)?;
            let mut request = creator_request(parse(args, &format!("creator/{op}"))?.value, op)?;
            if [
                "ownership",
                "infra-ownership",
                "publication",
                "publish-staging",
            ]
            .contains(&op)
                && request["command"].is_null()
            {
                request["command"] = request["_positionals"]
                    .as_array()
                    .and_then(|values| values.first())
                    .cloned()
                    .unwrap_or(Value::Null);
            }
            if op == "seed-runtime" {
                singular(&mut request, "edition")?;
                tooling::normalise(&mut request)?;
            }
            let gate = generator_gate::Gate::current()?;
            let output = infra_generator::execute(op, &request, &gate)?;
            Ok(Output {
                code: output.code,
                stdout: output.stdout,
                stderr: output.stderr,
                ..Default::default()
            })
        }
        "skill-tools" | "routing-eval" => {
            let (op, args) = if group == "skill-tools" {
                operation(rest, None)?
            } else {
                ("routing-eval", rest)
            };
            let parsed = parse(
                args,
                &if group == "skill-tools" {
                    format!("skill-tools/{op}")
                } else {
                    op.to_string()
                },
            )?
            .value;
            let mut request = if group == "skill-tools" {
                working_request(parsed)?
            } else {
                routing_request(parsed)?
            };
            let positionals = request["_positionals"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let field = match op {
                "aggregate-benchmark" => "benchmark_dir",
                "init-skill" => "skill_name",
                "package-skill" | "quick-validate" => "skill_path",
                "generate-report" => "input",
                "generate-review" => "workspace",
                _ => "",
            };
            if !field.is_empty() && request[field].is_null() {
                request[field] = positionals.first().cloned().unwrap_or(Value::Null);
            }
            if op == "package-skill" && request["output_dir"].is_null() {
                request["output_dir"] = positionals.get(1).cloned().unwrap_or(Value::Null);
            }
            let command = if group == "skill-tools" {
                format!("skill-tools/{op}")
            } else {
                op.to_string()
            };
            Output::envelope(infra_skill_tools::execute(&command, &request)?)
        }
        "release" => {
            let (op, args) = operation(rest, None)?;
            if op != "categorize" {
                return Err(RuntimeError::invalid("release requires categorize"));
            }
            let request = working_request(parse(args, "release/categorize")?.value)?;
            tooling::execute("release-categorize", request)
        }
        "system" => {
            let (op, args) = operation(rest, None)?;
            let request = parse(args, &format!("system/{op}"))?.value;
            Output::envelope(infra_system::execute(op, &request)?)
        }
        "eval" | "bench" => {
            let (family, args) = operation(rest, None)?;
            let (sub, args) = if group == "eval"
                && matches!(
                    family,
                    "memory-eval" | "memory-outcome-ab" | "memory-native-ab"
                ) {
                let (op, args) = operation(args, None)?;
                (Some(op), args)
            } else {
                (None, args)
            };
            let operation = sub.map(|sub| format!("{family}/{sub}")).unwrap_or_else(|| {
                match family {
                    "codex" => "codex-bench",
                    "claude" | "claude-code" => "claude-code-bench",
                    other => other,
                }
                .to_string()
            });
            let mut request = eval_request(parse(args, &format!("eval/{operation}"))?.value)?;
            if operation == "memory-onnx-pilot" {
                request["arguments"] = request["_remainder"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into();
            }
            if operation.ends_with("/report") && request["result"].is_null() {
                request["result"] = request["_positionals"]
                    .as_array()
                    .and_then(|args| args.first())
                    .cloned()
                    .unwrap_or(Value::Null);
            }
            if request["runtime_binary"].is_null() {
                request["runtime_binary"] = std::env::current_exe()?.display().to_string().into();
            }
            Output::envelope(infra_eval::execute(&operation, &request)?)
        }
        "qa" => {
            let (command, args) = operation(rest, None)?;
            let (request, args) = source_arguments(args)?;
            let root = source_root(&request)?;
            let code = infra_qa::run_at(&root, command, &args)
                .map_err(|error| RuntimeError::unavailable(error.to_string()))?;
            Ok(inherited(code))
        }
        "pilot" => {
            let (command, args) = operation(rest, None)?;
            let code = infra_pilots::run_cli(command, args)?;
            Ok(inherited(code))
        }
        "onnx-worker" => {
            let code = infra_pilots::run_cli("onnx-worker", rest)?;
            Ok(inherited(code))
        }
        "harness" | "serve" | "app" | "internal-sandbox-child" | "internal-discovery-probe" => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            let code = runtime.block_on(infra_harness::run(group, rest))?;
            Ok(inherited(code))
        }
        "internal" => {
            let (command, args) = operation(rest, None)?;
            let target = match command {
                "guard" => "internal-guard",
                "sandbox-child" => "internal-sandbox-child",
                "discovery-probe" => "internal-discovery-probe",
                _ => return Err(RuntimeError::invalid("Unknown internal command")),
            };
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            let code = runtime.block_on(infra_harness::run(target, args))?;
            Ok(inherited(code))
        }
        "check" => {
            let request = source_request(parse(rest, "check")?.value)?;
            tooling::execute("check", request)
        }
        operation if arguments::MEMORY_COMMANDS.contains(&operation) => {
            let mut args = vec!["memory".into()];
            args.extend_from_slice(raw);
            run(&args)
        }
        operation
            if operation.starts_with("memory-")
                || matches!(
                    operation,
                    "cost-attribution" | "codex-bench" | "claude-code-bench"
                ) =>
        {
            let mut args = vec!["eval".into()];
            args.extend_from_slice(raw);
            run(&args)
        }
        _ => Err(RuntimeError::invalid(format!("Unknown command: {group}"))),
    }
}
/// Compiled path helpers resolve relative files in the caller's working directory.
/// Repository resources are optional metadata unless that operation requires them.
fn working_request(mut value: Value) -> Result<Value> {
    if value["root"].is_null() {
        value["root"] = infra_core::fs::python_realpath(&std::env::current_dir()?)?
            .display()
            .to_string()
            .into();
    }
    if value["source_root"].is_string() || value["tooling_root"].is_string() {
        value["source_root"] = source_root(&value)?.display().to_string().into();
    } else if let Ok(root) = source_root(&value) {
        value["source_root"] = root.display().to_string().into();
    }
    Ok(value)
}
fn routing_request(mut value: Value) -> Result<Value> {
    let source = source_root(&value)?;
    value["source_root"] = source.display().to_string().into();
    if value["root"].is_null() {
        value["root"] = source.display().to_string().into();
    }
    Ok(value)
}
fn creator_request(value: Value, operation: &str) -> Result<Value> {
    let mut value = working_request(value)?;
    if [
        "validate-generated",
        "validate-skill-quality",
        "validate-plan-mutations",
    ]
    .contains(&operation)
        && value["generator_root"].is_null()
        && value["registry"].is_null()
        && value["candidate_registry"].is_null()
        && let Some(source) = value["source_root"].as_str()
    {
        let generator = Path::new(source).join("Infrastructure-Creator");
        if generator
            .join(".agents/skills/skill-forge/references/candidate-registry.json")
            .is_file()
        {
            value["generator_root"] = generator.display().to_string().into();
        }
    }
    Ok(value)
}
