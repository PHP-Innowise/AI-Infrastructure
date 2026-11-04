use crate::{
    HarnessError, Result,
    process::{self, Limits},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub available: bool,
    pub executable: Option<PathBuf>,
    pub detail: String,
}
#[derive(Clone)]
pub struct Launch {
    pub provider: String,
    pub executable: PathBuf,
    pub project: PathBuf,
    pub prompt: String,
    pub mode: String,
    pub model: Option<String>,
    pub session: Option<String>,
    pub agents: bool,
    pub count: u32,
    pub effort: Option<String>,
    pub usd: Option<f64>,
    pub hook_events: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkerResult {
    pub text: String,
    pub ok: bool,
    pub cost_usd: Option<f64>,
    pub session_id: Option<String>,
    pub duration_seconds: f64,
    pub error: Option<String>,
    pub limit_reached: Option<String>,
}
fn argument(value: &str) -> Result<()> {
    if value.trim().is_empty() || value.starts_with('-') || value.chars().any(|c| c.is_control()) {
        return Err(HarnessError::invalid("Invalid native CLI argument."));
    }
    Ok(())
}
pub fn environment(provider: &str, agents: bool, count: u32) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    if provider == "claude" {
        env.insert(
            "CLAUDE_CODE_MAX_CONCURRENT_SUBAGENTS".into(),
            if agents {
                count.to_string()
            } else {
                "0".into()
            },
        );
    }
    env
}
#[derive(Clone)]
pub struct PreparedCommand {
    pub argv: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub input: Vec<u8>,
    pub readable: Vec<PathBuf>,
    pub remove: Vec<String>,
}
pub trait LaunchAdapter: Send + Sync {
    fn adapt(&self, input: &Launch, command: PreparedCommand) -> Result<PreparedCommand>;
}
pub trait RunEvents: process::RunObserver {
    fn observe(&self, raw: Option<&Value>, normalized: Option<&Value>) -> Result<()>;
    fn enrich(&self, event: &Value) -> Result<Value> {
        Ok(event.clone())
    }
}
pub fn command_with_adapter(
    input: &Launch,
    adapter: &dyn LaunchAdapter,
) -> Result<PreparedCommand> {
    let (argv, environment, bytes) = command(input)?;
    adapter.adapt(
        input,
        PreparedCommand {
            argv,
            environment,
            input: bytes,
            readable: vec![],
            remove: vec![],
        },
    )
}
pub type CommandParts = (Vec<String>, BTreeMap<String, String>, Vec<u8>);
pub fn command(input: &Launch) -> Result<CommandParts> {
    if !["claude", "codex", "cursor"].contains(&input.provider.as_str())
        || !["plan", "edit"].contains(&input.mode.as_str())
        || input.count < 1
        || input.count > 40
        || !input.project.is_dir()
        || input.prompt.trim().is_empty()
        || input.prompt.contains('\0')
    {
        return Err(HarnessError::invalid("Invalid native launch."));
    }
    for value in [&input.model, &input.effort, &input.session]
        .into_iter()
        .flatten()
    {
        argument(value)?;
    }
    if input.usd.is_some_and(|v| {
        input.provider != "claude" || !v.is_finite() || !(0.01..=1000.).contains(&v)
    }) {
        return Err(HarnessError::invalid(
            "A native USD cap is supported only for Claude.",
        ));
    }
    let exe = input
        .executable
        .to_str()
        .ok_or_else(|| HarnessError::invalid("Native executable path is not UTF-8."))?;
    let mut args = command_argv(&[exe.into()], Some(&input.provider))?;
    let mut env = environment(&input.provider, input.agents, input.count);
    match input.provider.as_str() {
        "claude" => {
            args.extend(
                [
                    "--print",
                    "--output-format",
                    "stream-json",
                    "--verbose",
                    "--permission-mode",
                    if input.mode == "plan" {
                        "plan"
                    } else {
                        "acceptEdits"
                    },
                ]
                .map(String::from),
            );
            if input.effort.as_deref() == Some("ultracode") {
                if !input.agents {
                    return Err(HarnessError::invalid(
                        "Ultracode requires additional agents.",
                    ));
                }
                args.extend(["--allowedTools", "Workflow"].map(String::from));
            } else {
                args.push("--disallowedTools".into());
                args.extend(
                    if input.agents {
                        vec!["Workflow"]
                    } else {
                        vec!["Agent", "Task", "Workflow"]
                    }
                    .into_iter()
                    .map(String::from),
                );
            }
            if let Some(effort) = &input.effort {
                env.insert(
                    "CLAUDE_CODE_EFFORT_LEVEL".into(),
                    if effort == "ultracode" {
                        "xhigh".into()
                    } else {
                        effort.clone()
                    },
                );
                args.extend(["--effort".into(), effort.clone()]);
            }
            args.extend(["--settings".into(),json!({"env":env,"ultracode":input.effort.as_deref()==Some("ultracode"),"disableWorkflows":input.effort.as_deref()!=Some("ultracode")}).to_string()]);
            if input.agents && input.effort.as_deref() != Some("ultracode") {
                args.extend(["--allowedTools", "Agent", "Task"].map(String::from));
            }
            if let Some(session) = &input.session {
                args.extend(["--resume".into(), session.clone()]);
            }
            if input.hook_events {
                args.push("--include-hook-events".into());
            }
        }
        "codex" => {
            args.extend([
                "--ask-for-approval".into(),
                "never".into(),
                "--sandbox".into(),
                if input.mode == "plan" {
                    "read-only".into()
                } else {
                    "workspace-write".into()
                },
                "--cd".into(),
                input.project.to_string_lossy().into(),
            ]);
            let settings = if input.agents {
                vec![
                    "agents.enabled=true".into(),
                    "features.multi_agent=true".into(),
                    format!("agents.max_concurrent_threads_per_session={}", input.count),
                    format!(
                        "features.multi_agent_v2={{enabled=true,max_concurrent_threads_per_session={}}}",
                        input.count + 1
                    ),
                ]
            } else {
                vec![
                    "agents.enabled=false".into(),
                    "features.multi_agent=false".into(),
                    "features.multi_agent_v2=false".into(),
                ]
            };
            for setting in settings {
                args.extend(["-c".into(), setting]);
            }
            if let Some(effort) = &input.effort {
                args.extend([
                    "-c".into(),
                    format!("model_reasoning_effort={}", json!(effort)),
                ]);
            }
            let size = ["AGENTS.override.md", "AGENTS.md"]
                .iter()
                .find_map(|name| std::fs::symlink_metadata(input.project.join(name)).ok())
                .filter(|m| m.is_file())
                .map(|m| m.len())
                .unwrap_or(0);
            if size > 32768 {
                args.extend([
                    "-c".into(),
                    format!(
                        "project_doc_max_bytes={}",
                        (size + 32768).clamp(131072, 262144)
                    ),
                ]);
            }
            args.push("exec".into());
            if input.session.is_some() {
                args.push("resume".into());
            }
            args.push("--json".into());
        }
        "cursor" => {
            args.extend(
                [
                    "--disable-auto-update",
                    "--print",
                    "--output-format",
                    "stream-json",
                    "--workspace",
                ]
                .map(String::from),
            );
            args.push(input.project.to_string_lossy().into());
            args.extend(["--sandbox", "enabled"].map(String::from));
            if input.mode == "plan" {
                args.extend(["--mode", "plan"].map(String::from));
            }
            if let Some(session) = &input.session {
                args.extend(["--resume".into(), session.clone()]);
            }
        }
        _ => return Err(HarnessError::invalid("Unknown provider.")),
    }
    if let Some(usd) = input.usd {
        args.extend(["--max-budget-usd".into(), usd.to_string()]);
    }
    if let Some(model) = &input.model {
        args.extend(["--model".into(), model.clone()]);
    }
    match input.provider.as_str() {
        "codex" => {
            if let Some(session) = &input.session {
                args.push(session.clone());
            }
            args.push("-".into());
        }
        "cursor" => args.extend(["--".into(), input.prompt.clone()]),
        _ => {}
    }
    let stdin = if input.provider == "cursor" {
        vec![]
    } else {
        input.prompt.as_bytes().to_vec()
    };
    Ok((args, env, stdin))
}
pub fn normalize(provider: &str, event: &Value) -> Vec<Value> {
    crate::activity::normalize(provider, event, true)
}
pub async fn run_observed_locked(
    input: &Launch,
    limits: Limits,
    cancel: Arc<AtomicBool>,
    previous: Option<f64>,
    runner_lock: Option<Arc<std::fs::File>>,
    emit: impl FnMut(Value) -> Result<()> + Send,
) -> Result<WorkerResult> {
    run_monitored_locked(
        input,
        limits,
        cancel,
        previous,
        runner_lock,
        None,
        &BTreeMap::new(),
        None,
        emit,
    )
    .await
}
struct Observer {
    events: Arc<dyn RunEvents>,
}
impl process::RunObserver for Observer {
    fn boundary(&self, signal: &str, details: &Value) -> Result<()> {
        self.events.boundary(signal, details)
    }
}
#[expect(
    clippy::too_many_arguments,
    reason = "The stable provider bridge passes immutable adapter, observations and runner custody per call."
)]
pub async fn run_monitored_locked(
    input: &Launch,
    limits: Limits,
    cancel: Arc<AtomicBool>,
    previous: Option<f64>,
    runner_lock: Option<Arc<std::fs::File>>,
    adapter: Option<&dyn LaunchAdapter>,
    extra_environment: &BTreeMap<String, Option<String>>,
    events: Option<Arc<dyn RunEvents>>,
    emit: impl FnMut(Value) -> Result<()> + Send,
) -> Result<WorkerResult> {
    run_monitored_until(
        input,
        limits,
        cancel,
        previous,
        runner_lock,
        adapter,
        extra_environment,
        events,
        None,
        emit,
    )
    .await
}
#[expect(
    clippy::too_many_arguments,
    reason = "The stable provider bridge keeps the recovery deadline, observations and runner custody explicit."
)]
pub async fn run_monitored_until(
    input: &Launch,
    limits: Limits,
    cancel: Arc<AtomicBool>,
    previous: Option<f64>,
    runner_lock: Option<Arc<std::fs::File>>,
    adapter: Option<&dyn LaunchAdapter>,
    extra_environment: &BTreeMap<String, Option<String>>,
    events: Option<Arc<dyn RunEvents>>,
    deadline: Option<std::time::Instant>,
    mut emit: impl FnMut(Value) -> Result<()> + Send,
) -> Result<WorkerResult> {
    let mut plan = if let Some(adapter) = adapter {
        command_with_adapter(input, adapter)?
    } else {
        let (argv, environment, bytes) = command(input)?;
        PreparedCommand {
            argv,
            environment,
            input: bytes,
            readable: vec![],
            remove: vec![],
        }
    };
    let mut remove = plan.remove.clone();
    for (name, value) in extra_environment {
        if name.is_empty() || name.contains(['\0', '=']) {
            return Err(HarnessError::invalid("Invalid scoped launch environment."));
        }
        match value {
            Some(value) => {
                if value.contains('\0') {
                    return Err(HarnessError::invalid("NUL in launch environment."));
                }
                plan.environment.insert(name.clone(), value.clone());
            }
            None => {
                plan.environment.remove(name);
                remove.push(name.clone());
            }
        }
    }
    let observer = events.as_ref().map(|events| {
        Arc::new(Observer {
            events: events.clone(),
        }) as Arc<dyn process::RunObserver>
    });
    let mut accounting = RunCost::new(&input.provider, input.session.clone(), previous);
    let mut text = String::new();
    let mut terminal = false;
    let mut failed = false;
    let mut session = None;
    let mut cost = None;
    let mut limit = None;
    let observe = |line: &[u8]| -> Result<()> {
        let Ok(event) = crate::strict_json::parse(line, 2 * 1024 * 1024) else {
            return Ok(());
        };
        if let Some(events) = &events {
            events.observe(Some(&event), None)?;
        }
        accounting.observe(&event);
        if input.provider == "claude" && event["subtype"] == "error_max_budget_usd" {
            limit = Some("USD".to_string());
        }
        for mut event in normalize(&input.provider, &event) {
            if event["kind"] == "usage"
                && let Some(reported) = event["cost_usd"].as_f64()
            {
                event["cost_usd_reported"] = reported.into();
                event["cost_session_total"] = json!(accounting.total(reported));
                event["cost_usd"] = json!(accounting.own(reported));
            }
            match event["kind"].as_str() {
                Some("session") => {
                    session = event["native_session_id"].as_str().map(String::from);
                }
                Some("text") => {
                    text = event["text"]
                        .as_str()
                        .unwrap_or("")
                        .chars()
                        .take(32000)
                        .collect();
                }
                Some("usage") => {
                    if let Some(value) = event["cost_usd"]
                        .as_f64()
                        .filter(|v| v.is_finite() && *v >= 0.)
                    {
                        cost = Some(value);
                    }
                }
                Some("error") => failed = true,
                Some("result") => {
                    terminal = true;
                    failed |= event["ok"] != true;
                    if text.is_empty() && event["ok"] == true {
                        text = event["text"]
                            .as_str()
                            .unwrap_or("")
                            .chars()
                            .take(32000)
                            .collect();
                    }
                }
                _ => {}
            }
            if let Some(events) = &events {
                events.observe(None, Some(&event))?;
                event = events.enrich(&event)?;
            }
            emit(event)?;
        }
        Ok(())
    };
    let output = if let Some(deadline) = deadline {
        process::run_monitored_until(
            &plan.argv,
            &input.project,
            &plan.environment,
            &remove,
            &plan.input,
            limits,
            cancel,
            runner_lock,
            observer,
            deadline,
            observe,
        )
        .await?
    } else {
        process::run_monitored(
            &plan.argv,
            &input.project,
            &plan.environment,
            &remove,
            &plan.input,
            limits,
            cancel,
            runner_lock,
            observer,
            observe,
        )
        .await?
    };
    let over = input
        .usd
        .zip(cost)
        .is_some_and(|(limit, cost)| cost > limit);
    if over {
        limit = Some("USD".into());
    }
    let ok = output.code == 0 && terminal && !failed && !over;
    Ok(WorkerResult {
        text,
        ok,
        cost_usd: cost,
        session_id: session,
        duration_seconds: output.seconds,
        error: if ok {
            None
        } else {
            Some(
                if limit.is_some() {
                    "Native reviewer reached its USD budget; review incomplete."
                } else {
                    "Native reviewer did not return a completed result."
                }
                .into(),
            )
        },
        limit_reached: limit,
    })
}
pub fn find_executable(name: &str) -> Option<PathBuf> {
    let path = Path::new(name);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_path_buf());
    }
    for folder in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let candidate = folder.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        for ext in [".exe", ".cmd", ".bat"] {
            let candidate = folder.join(format!("{name}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}
pub async fn discover(overrides: &BTreeMap<String, String>) -> Result<Vec<Provider>> {
    // Only providers overlap: each retains its candidate and version/help
    // sequence. Await all three before selecting the original error order so
    // an ordinary error never abandons another provider's guardian cleanup.
    // Peers may already be probing when a provider reports a filesystem error.
    let (claude, codex, cursor) = tokio::join!(
        discover_provider(overrides, "claude", "Claude Code"),
        discover_provider(overrides, "codex", "Codex"),
        discover_provider(overrides, "cursor", "Cursor Agent"),
    );
    Ok(vec![claude?, codex?, cursor?])
}
async fn discover_provider(
    overrides: &BTreeMap<String, String>,
    id: &str,
    name: &str,
) -> Result<Provider> {
    let candidates = overrides
        .get(id)
        .map(|s| vec![s.clone()])
        .unwrap_or_else(|| {
            if id == "cursor" {
                vec!["cursor-agent".into(), "agent".into()]
            } else {
                vec![id.into()]
            }
        });
    let mut executable = None;
    for candidate in candidates {
        let Some(path) = find_executable(&candidate) else {
            continue;
        };
        let mut verified = true;
        for flag in ["--version", "--help"] {
            let mut argv = match command_argv(&[path.to_string_lossy().into_owned()], Some(id)) {
                Ok(argv) => argv,
                Err(_) => {
                    verified = false;
                    break;
                }
            };
            if id == "cursor" {
                argv.push("--disable-auto-update".into());
            }
            argv.push(flag.into());
            match process::run_raw_with_lock(
                &argv,
                &std::env::current_dir()?,
                &BTreeMap::new(),
                &[],
                Limits {
                    seconds: Some(3),
                    output: 65536,
                    line: 65536,
                },
                Arc::new(AtomicBool::new(false)),
                None,
                |_| Ok(()),
            )
            .await
            {
                Ok(result) if result.code == 0 => {
                    let body = String::from_utf8_lossy(&result.bytes);
                    if flag == "--help" {
                        verified &= match id {
                            "claude" => body.contains("Claude Code"),
                            "codex" => body.contains("Codex") || body.contains("codex"),
                            _ => {
                                [
                                    "--print",
                                    "--output-format",
                                    "--resume",
                                    "--workspace",
                                    "--sandbox",
                                    "--mode",
                                ]
                                .iter()
                                .all(|v| body.contains(v))
                                    && body.contains("Cursor")
                            }
                        };
                    }
                }
                _ => {
                    verified = false;
                    break;
                }
            }
        }
        if verified {
            executable = Some(std::fs::canonicalize(path)?);
            break;
        }
    }
    let available = executable.is_some();
    Ok(Provider {
        id: id.into(),
        name: name.into(),
        available,
        executable: executable.clone(),
        detail: if executable.is_some() {
            "CLI identity verified; sign-in is queried separately.".into()
        } else {
            "CLI not found or identity/required flags could not be verified.".into()
        },
    })
}

pub fn command_argv(command: &[String], _provider: Option<&str>) -> Result<Vec<String>> {
    if command.is_empty() || command.iter().any(|s| s.contains('\0')) {
        return Err(HarnessError::invalid("A native executable is required."));
    }
    #[cfg(windows)]
    {
        let executable = find_executable(&command[0]).unwrap_or_else(|| PathBuf::from(&command[0]));
        if executable
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("cmd") || s.eq_ignore_ascii_case("bat"))
        {
            let name = executable
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            let package = match name.as_str() {
                "codex" => "@openai/codex",
                "claude" => "@anthropic-ai/claude-code",
                "npm" | "npx" => "npm",
                _ => {
                    return Err(HarnessError::forbidden(
                        "Unsupported Windows batch launcher.",
                    ));
                }
            };
            if _provider.is_some_and(|p| p != name) {
                return Err(HarnessError::forbidden(
                    "Windows launcher identity differs from its provider.",
                ));
            }
            let parent = executable
                .parent()
                .ok_or_else(|| HarnessError::invalid("Invalid launcher path."))?;
            let package_root = parent.join("node_modules").join(package);
            infra_core::fs::admit_descendant(parent, &package_root)?;
            let metadata = infra_core::fs::read_regular(&package_root.join("package.json"))?;
            let metadata = crate::strict_json::parse(&metadata, 1024 * 1024)?;
            if metadata["name"] != package {
                return Err(HarnessError::forbidden(
                    "Windows npm package identity does not match.",
                ));
            }
            let bin = metadata["bin"][&name]
                .as_str()
                .ok_or_else(|| HarnessError::forbidden("Windows npm bin metadata is missing."))?;
            if bin.contains('\\')
                || bin.contains(':')
                || Path::new(bin).is_absolute()
                || Path::new(bin)
                    .components()
                    .any(|p| !matches!(p, std::path::Component::Normal(_)))
            {
                return Err(HarnessError::forbidden("Unsafe Windows npm bin path."));
            }
            let target = package_root.join(bin);
            infra_core::fs::admit_descendant(&package_root, &target)?;
            if !target.is_file() {
                return Err(HarnessError::forbidden(
                    "Windows npm bin target is missing.",
                ));
            }
            let mut out = if target
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.eq_ignore_ascii_case("exe"))
            {
                vec![target.to_string_lossy().into()]
            } else {
                if !target
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.eq_ignore_ascii_case("js") || s.eq_ignore_ascii_case("cjs"))
                {
                    return Err(HarnessError::forbidden("Unsupported Windows npm bin type."));
                }
                let node = find_executable("node.exe").ok_or_else(|| {
                    HarnessError::unavailable("Node.js is required for this Windows npm launcher.")
                })?;
                let metadata = std::fs::symlink_metadata(&node)?;
                if infra_core::fs::linked(&metadata) || !metadata.is_file() {
                    return Err(HarnessError::forbidden("Unsafe Node.js executable."));
                }
                vec![
                    node.to_string_lossy().into(),
                    target.to_string_lossy().into(),
                ]
            };
            out.extend_from_slice(&command[1..]);
            return Ok(out);
        }
        let mut out = vec![executable.to_string_lossy().into()];
        out.extend_from_slice(&command[1..]);
        Ok(out)
    }
    #[cfg(not(windows))]
    {
        Ok(command.to_vec())
    }
}
#[derive(Clone, Debug)]
pub struct RunCost {
    pub provider: String,
    pub resumed: Option<String>,
    pub previous: Option<f64>,
    pub restores: bool,
    pub session: Option<String>,
}
impl RunCost {
    pub fn new(provider: &str, resumed: Option<String>, previous: Option<f64>) -> Self {
        Self {
            provider: provider.into(),
            session: resumed.clone(),
            resumed,
            previous,
            restores: false,
        }
    }
    pub fn observe(&mut self, event: &Value) {
        if self.provider != "claude" || event["type"] != "system" || event["subtype"] != "init" {
            return;
        }
        if let Some(version) = event["claude_code_version"].as_str() {
            let parts = version
                .split('.')
                .take(3)
                .map(|s| {
                    s.chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect::<String>()
                        .parse::<u64>()
                        .ok()
                })
                .collect::<Option<Vec<_>>>();
            self.restores =
                parts.is_some_and(|v| v.len() == 3 && (v[0], v[1], v[2]) >= (2, 1, 277));
        }
        if let Some(session) = event["session_id"].as_str().filter(|s| !s.is_empty()) {
            self.session = Some(session.into());
        }
    }
    pub fn own(&self, reported: f64) -> Option<f64> {
        if self.provider != "claude" || self.resumed.is_none() || !self.restores {
            return Some(reported);
        }
        self.previous
            .filter(|p| reported >= *p)
            .map(|p| ((reported - p) * 1_000_000_000.).round() / 1_000_000_000.)
    }
    pub fn total(&self, reported: f64) -> Option<f64> {
        if self.provider != "claude" || self.resumed.is_none() {
            return Some(reported);
        }
        if !self.restores {
            return None;
        }
        Some(self.previous.filter(|p| reported < *p).unwrap_or(reported))
    }
}
pub fn total_tokens(event: &Value) -> Option<u64> {
    let input = event["input_tokens"].as_u64()?;
    let output = event["output_tokens"].as_u64()?;
    input
        .checked_add(output)?
        .checked_add(event["cache_read_input_tokens"].as_u64().unwrap_or(0))?
        .checked_add(event["cache_creation_input_tokens"].as_u64().unwrap_or(0))
}

pub async fn run(
    input: &Launch,
    limits: Limits,
    cancel: Arc<AtomicBool>,
    emit: impl FnMut(Value) -> Result<()> + Send,
) -> Result<WorkerResult> {
    run_observed(input, limits, cancel, None, emit).await
}

pub async fn run_observed(
    input: &Launch,
    limits: Limits,
    cancel: Arc<AtomicBool>,
    previous: Option<f64>,
    emit: impl FnMut(Value) -> Result<()> + Send,
) -> Result<WorkerResult> {
    run_observed_locked(input, limits, cancel, previous, None, emit).await
}
