use crate::{
    HarnessError, Result,
    conversation::{self, Step},
    permission::{Approvals, Decision, PermissionMode},
    process::{self, Limits},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
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
    /// What an editing launch may do without asking. None keeps the fixed
    /// behaviour of launches without a person: workspace edits only.
    pub permissions: Option<PermissionMode>,
    /// Where a Manual launch puts the requests a person answers.
    pub approvals: Option<Arc<dyn Approvals>>,
}
/// How a launch talks to its host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    /// The prompt is written, the input closes, the host prints its events.
    Text,
    /// Claude Code's stream-json input with its control channel on stdio.
    ClaudeControl,
    /// Codex's app server.
    CodexServer,
    /// Cursor's ACP server.
    CursorAcp,
}
/// The transport a launch needs: a person answering requests needs a host
/// that keeps listening, and Claude Code is told to accept edits where its
/// auto mode is unavailable.
pub fn transport(input: &Launch) -> Transport {
    match (
        input.provider.as_str(),
        input.permissions.filter(|_| input.mode == "edit"),
    ) {
        ("claude", Some(PermissionMode::Auto | PermissionMode::Manual)) => Transport::ClaudeControl,
        ("codex", Some(PermissionMode::Manual)) => Transport::CodexServer,
        ("cursor", Some(PermissionMode::Manual)) => Transport::CursorAcp,
        _ => Transport::Text,
    }
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
    /// Text placed before the prompt of a host that is prompted over its
    /// protocol rather than on its command line.
    pub prompt_prefix: String,
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
            prompt_prefix: String::new(),
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
    // A planning launch stays read-only whatever the permissions say.
    let permission = input.permissions.filter(|_| input.mode == "edit");
    let transport = transport(input);
    match input.provider.as_str() {
        "claude" => {
            args.extend(
                [
                    "--print",
                    "--output-format",
                    "stream-json",
                    "--verbose",
                    "--permission-mode",
                    match (input.mode.as_str(), permission) {
                        ("plan", _) => "plan",
                        (_, None) => "acceptEdits",
                        (_, Some(PermissionMode::Auto)) => "auto",
                        (_, Some(PermissionMode::Manual)) => "default",
                        (_, Some(PermissionMode::Bypass)) => "bypassPermissions",
                    },
                ]
                .map(String::from),
            );
            if permission == Some(PermissionMode::Bypass) {
                args.push("--allow-dangerously-skip-permissions".into());
            }
            if transport == Transport::ClaudeControl {
                args.extend(
                    [
                        "--input-format",
                        "stream-json",
                        "--permission-prompt-tool",
                        "stdio",
                    ]
                    .map(String::from),
                );
            }
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
            match (transport, permission) {
                // The app server takes approvals and sandbox per thread.
                (Transport::CodexServer, _) => {}
                // Codex's automatic reviewer answers what needs approval,
                // inside the workspace-write sandbox.
                (_, Some(PermissionMode::Auto)) => args.push("--approve-for-me".into()),
                (_, Some(PermissionMode::Bypass)) => {
                    args.push("--dangerously-bypass-approvals-and-sandbox".into())
                }
                _ => args.extend([
                    "--ask-for-approval".into(),
                    "never".into(),
                    "--sandbox".into(),
                    if input.mode == "plan" {
                        "read-only".into()
                    } else {
                        "workspace-write".into()
                    },
                ]),
            }
            if transport != Transport::CodexServer {
                args.extend(["--cd".into(), input.project.to_string_lossy().into()]);
            }
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
            if transport == Transport::CodexServer {
                args.push("app-server".into());
            } else {
                args.push("exec".into());
                if input.session.is_some() {
                    args.push("resume".into());
                }
                args.push("--json".into());
            }
        }
        "cursor" => {
            args.push("--disable-auto-update".into());
            if transport != Transport::CursorAcp {
                args.extend(
                    ["--print", "--output-format", "stream-json", "--workspace"].map(String::from),
                );
                args.push(input.project.to_string_lossy().into());
                if permission == Some(PermissionMode::Bypass) {
                    args.extend(["--force", "--sandbox", "disabled"].map(String::from));
                } else {
                    args.extend(["--sandbox", "enabled"].map(String::from));
                }
                if input.mode == "plan" {
                    args.extend(["--mode", "plan"].map(String::from));
                }
                if let Some(session) = &input.session {
                    args.extend(["--resume".into(), session.clone()]);
                }
            }
        }
        _ => return Err(HarnessError::invalid("Unknown provider.")),
    }
    if let Some(usd) = input.usd {
        args.extend(["--max-budget-usd".into(), usd.to_string()]);
    }
    // Codex's app server takes the model with the thread.
    if let Some(model) = &input.model
        && transport != Transport::CodexServer
    {
        args.extend(["--model".into(), model.clone()]);
    }
    match (input.provider.as_str(), transport) {
        ("codex", Transport::Text) => {
            if let Some(session) = &input.session {
                args.push(session.clone());
            }
            args.push("-".into());
        }
        ("cursor", Transport::Text) => args.extend(["--".into(), input.prompt.clone()]),
        ("cursor", Transport::CursorAcp) => args.push("acp".into()),
        _ => {}
    }
    // A host that talks over its input is prompted by its driver.
    let stdin = if input.provider == "cursor" || transport != Transport::Text {
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
            prompt_prefix: String::new(),
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
    let mut reading = Reading::new(input, previous);
    let transport = transport(input);
    let output = if transport == Transport::Text {
        let observe = |line: &[u8]| -> Result<()> {
            let Ok(event) = crate::strict_json::parse(line, 2 * 1024 * 1024) else {
                return Ok(());
            };
            reading.read(input, &event, &events, &mut emit)
        };
        if let Some(deadline) = deadline {
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
        }
    } else {
        converse_with_host(
            input,
            transport,
            &plan,
            &remove,
            limits,
            cancel,
            runner_lock,
            observer,
            deadline,
            &mut reading,
            &events,
            &mut emit,
        )
        .await?
    };
    Ok(reading.finish(input, output))
}
/// What a run learns from its host's events.
struct Reading {
    accounting: RunCost,
    text: String,
    terminal: bool,
    failed: bool,
    session: Option<String>,
    cost: Option<f64>,
    limit: Option<String>,
}
impl Reading {
    fn new(input: &Launch, previous: Option<f64>) -> Self {
        Self {
            accounting: RunCost::new(&input.provider, input.session.clone(), previous),
            text: String::new(),
            terminal: false,
            failed: false,
            session: None,
            cost: None,
            limit: None,
        }
    }
    /// One event in the host's own format.
    fn read(
        &mut self,
        input: &Launch,
        event: &Value,
        events: &Option<Arc<dyn RunEvents>>,
        emit: &mut (impl FnMut(Value) -> Result<()> + Send),
    ) -> Result<()> {
        if let Some(events) = events {
            events.observe(Some(event), None)?;
        }
        self.accounting.observe(event);
        if input.provider == "claude" && event["subtype"] == "error_max_budget_usd" {
            self.limit = Some("USD".to_string());
        }
        for mut event in normalize(&input.provider, event) {
            if event["kind"] == "usage"
                && let Some(reported) = event["cost_usd"].as_f64()
            {
                event["cost_usd_reported"] = reported.into();
                event["cost_session_total"] = json!(self.accounting.total(reported));
                event["cost_usd"] = json!(self.accounting.own(reported));
            }
            match event["kind"].as_str() {
                Some("session") => {
                    self.session = event["native_session_id"].as_str().map(String::from);
                }
                Some("text") => {
                    self.text = event["text"]
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
                        self.cost = Some(value);
                    }
                }
                Some("error") => self.failed = true,
                Some("result") => {
                    self.terminal = true;
                    self.failed |= event["ok"] != true;
                    if self.text.is_empty() && event["ok"] == true {
                        self.text = event["text"]
                            .as_str()
                            .unwrap_or("")
                            .chars()
                            .take(32000)
                            .collect();
                    }
                }
                _ => {}
            }
            if let Some(events) = events {
                events.observe(None, Some(&event))?;
                event = events.enrich(&event)?;
            }
            emit(event)?;
        }
        Ok(())
    }
    fn finish(mut self, input: &Launch, output: process::Output) -> WorkerResult {
        let over = input
            .usd
            .zip(self.cost)
            .is_some_and(|(limit, cost)| cost > limit);
        if over {
            self.limit = Some("USD".into());
        }
        let ok = output.code == 0 && self.terminal && !self.failed && !over;
        WorkerResult {
            text: self.text,
            ok,
            cost_usd: self.cost,
            session_id: self.session,
            duration_seconds: output.seconds,
            error: if ok {
                None
            } else {
                Some(
                    if self.limit.is_some() {
                        "Native reviewer reached its USD budget; review incomplete."
                    } else {
                        "Native reviewer did not return a completed result."
                    }
                    .into(),
                )
            },
            limit_reached: self.limit,
        }
    }
}
/// Counts a request as waiting for a person until it is answered or dropped.
struct Waiting(Arc<AtomicUsize>);
impl Waiting {
    fn start(count: &Arc<AtomicUsize>) -> Self {
        count.fetch_add(1, Ordering::AcqRel);
        Self(count.clone())
    }
}
impl Drop for Waiting {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
fn line(value: &Value) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}
/// Runs a host that keeps its input open: replies to its protocol, routes its
/// permission requests and reads its events as a one-shot launch would.
#[expect(
    clippy::too_many_arguments,
    reason = "The conversation shares the one-shot launch's custody, observations and reading."
)]
async fn converse_with_host(
    input: &Launch,
    transport: Transport,
    plan: &PreparedCommand,
    remove: &[String],
    limits: Limits,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<std::fs::File>>,
    observer: Option<Arc<dyn process::RunObserver>>,
    deadline: Option<std::time::Instant>,
    reading: &mut Reading,
    events: &Option<Arc<dyn RunEvents>>,
    emit: &mut (impl FnMut(Value) -> Result<()> + Send),
) -> Result<process::Output> {
    let mode = input
        .permissions
        .ok_or_else(|| HarnessError::invalid("A conversation needs a permission mode."))?;
    if mode == PermissionMode::Manual && input.approvals.is_none() {
        return Err(HarnessError::invalid(
            "Manual permissions need someone to answer the requests.",
        ));
    }
    let setup = conversation::Setup {
        mode,
        prompt: format!("{}{}", plan.prompt_prefix, input.prompt),
        cwd: input.project.to_string_lossy().into_owned(),
        model: input.model.clone(),
        resume: input.session.clone(),
    };
    let mut driver = match transport {
        Transport::ClaudeControl => conversation::claude(setup),
        Transport::CodexServer => conversation::codex(setup),
        Transport::CursorAcp => conversation::cursor(setup),
        Transport::Text => {
            return Err(HarnessError::invalid(
                "A one-shot launch has no conversation.",
            ));
        }
    };
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let finished = Arc::new(AtomicBool::new(false));
    let waiting = Arc::new(AtomicUsize::new(0));
    let decisions: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>> = Arc::default();
    // Answers the driver has not heard yet, by ask id: allowed or not.
    let answers: Arc<Mutex<Vec<(String, bool)>>> = Arc::default();
    // The host hears the opening before it says anything: Codex and Cursor
    // answer only what they are asked.
    for reply in driver.opening().replies {
        let _ = sender.send(line(&reply)?);
    }
    let mut writer = Some(sender);
    let mut take = |step: Step,
                    writer: &mut Option<tokio::sync::mpsc::UnboundedSender<Vec<u8>>>,
                    reading: &mut Reading|
     -> Result<()> {
        // What the host said comes before what it asks, as it happened.
        for event in &step.events {
            reading.read(input, event, events, emit)?;
        }
        for reply in &step.replies {
            if let Some(writer) = writer.as_ref() {
                let _ = writer.send(line(reply)?);
            }
        }
        for notice in step.notices {
            emit(notice)?;
        }
        for id in &step.withdrawn {
            if let Some(approvals) = &input.approvals {
                approvals.withdraw(id)?;
            }
        }
        for ask in step.asks {
            let Some(approvals) = input.approvals.clone() else {
                continue;
            };
            let Some(sender) = writer.clone() else {
                continue;
            };
            let decided = match approvals.ask(&ask.public) {
                Ok(decided) => decided,
                Err(error) => {
                    // Nobody can see the request: the host hears no.
                    if let Ok(mut answers) = answers.lock() {
                        answers.push((ask.public.id.clone(), false));
                    }
                    let _ = sender.send(line(&ask.deny)?);
                    emit(
                        json!({"kind":"error","text":format!("A permission request could not be shown: {}", error.message)}),
                    )?;
                    continue;
                }
            };
            let pending = Waiting::start(&waiting);
            let answers = answers.clone();
            let task = tokio::spawn(async move {
                let _pending = pending;
                let (reply, allowed) = match decided.await {
                    Ok(Decision::Allow) => (ask.allow, true),
                    Ok(Decision::AllowSession) => (ask.allow_session.unwrap_or(ask.allow), true),
                    Ok(Decision::Deny) | Err(_) => (ask.deny, false),
                };
                // Recorded before the host hears it, so the driver knows the
                // answer before anything the host says about it.
                if let Ok(mut answers) = answers.lock() {
                    answers.push((ask.public.id.clone(), allowed));
                }
                if let Ok(bytes) = line(&reply) {
                    let _ = sender.send(bytes);
                }
            });
            if let Ok(mut tasks) = decisions.lock() {
                tasks.retain(|task| !task.is_finished());
                tasks.push(task);
            }
        }
        if step.close_input {
            writer.take();
        }
        if step.finished {
            finished.store(true, Ordering::Release);
        }
        Ok(())
    };
    let observe = |bytes: &[u8]| -> Result<()> {
        let Ok(message) = crate::strict_json::parse(bytes, 2 * 1024 * 1024) else {
            return Ok(());
        };
        if !message.is_object() {
            return Ok(());
        }
        let heard = answers
            .lock()
            .map(|mut answers| std::mem::take(&mut *answers))
            .unwrap_or_default();
        for (ask, allowed) in heard {
            driver.answered(&ask, allowed);
        }
        let step = driver.receive(message);
        take(step, &mut writer, reading)
    };
    let output = process::run_interactive(
        &plan.argv,
        &input.project,
        &plan.environment,
        remove,
        &[],
        limits,
        cancel,
        runner_lock,
        observer,
        deadline,
        process::Interactive {
            input: receiver,
            finished: finished.clone(),
            waiting: waiting.clone(),
        },
        observe,
    )
    .await;
    if let Ok(mut tasks) = decisions.lock() {
        for task in tasks.drain(..) {
            task.abort();
        }
    }
    output
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
            executable = Some(infra_core::fs::canonicalize(path)?);
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
