use crate::{
    HarnessError, Result, attachments, clash, fleet,
    knowledge::KnowledgeDomain,
    process::Limits,
    projects,
    provider::{self, Launch, ProviderDomain},
    services::{Domain, Payload, Reply},
    session_scheduler::{self, Lease},
    store::{self, SessionStore},
};
use rusqlite::{TransactionBehavior, params};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
mod inbox_driver;
#[cfg(test)]
mod inbox_tests;
pub type Event = Arc<dyn Fn(Value) -> Result<()> + Send + Sync>;
pub type RunReply<'a> = Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>>;
pub trait SpecialWorkflow: Send + Sync {
    fn run<'a>(
        &'a self,
        session: Value,
        prompt: String,
        generation: String,
        cancel: Arc<AtomicBool>,
        emit: Event,
    ) -> RunReply<'a>;
}
pub trait BlackboardFactory: Send + Sync {
    fn for_session(&self, session: &Value) -> Result<Arc<dyn fleet::Blackboard>>;
}
#[derive(Clone)]
struct PreparedTrace {
    trace: infra_skillcache::trace::Trace,
    usage: Value,
    redactor: crate::skillcache::trace::SessionRedactor,
    admissible: std::result::Result<(Vec<Value>, usize), Value>,
}

enum PreTurnObservation {
    None,
    Full(Option<crate::skillcache::replay_result::Captured>),
    Toolchain(Option<crate::skillcache::trace::ToolchainPreTurnStamp>),
}

fn direct_formatter_payload(answer: &Value) -> Value {
    answer
        .get("result")
        .filter(|v| v.is_object())
        .or_else(|| answer.get("results").filter(|v| v.is_object()))
        .cloned()
        .unwrap_or_else(|| answer.clone())
}

fn should_run_thin_formatter(pref: Option<bool>, answer: &Value) -> bool {
    match pref {
        Some(enabled) => enabled,
        None => {
            let results = answer
                .get("result")
                .filter(|v| v.is_object())
                .or_else(|| answer.get("results").filter(|v| v.is_object()));
            let Some(obj) = results.and_then(Value::as_object) else {
                return false;
            };
            if obj.is_empty() {
                return false;
            }
            const PROSE_KEYS: [&str; 6] =
                ["markdown", "summary", "report", "output", "text", "message"];
            let has_prose = |map: &serde_json::Map<String, Value>| {
                PROSE_KEYS.iter().any(|k| {
                    map.get(*k)
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.trim().is_empty())
                })
            };
            if has_prose(obj) {
                return false;
            }
            if obj.len() == 1
                && obj
                    .values()
                    .next()
                    .and_then(Value::as_object)
                    .is_some_and(has_prose)
            {
                return false;
            }
            true
        }
    }
}
#[derive(Clone)]
enum JobKind {
    Run,
    Check {
        check_id: String,
        previous_status: String,
    },
}
#[derive(Clone)]
struct Job {
    sid: String,
    prompt: String,
    generation: String,
    kind: JobKind,
}
pub struct SessionLimits {
    pub seconds: u64,
    pub concurrency: usize,
}
#[derive(Clone)]
struct ProtectedSandbox {
    protected: Vec<PathBuf>,
    engine: Option<PathBuf>,
}
fn build_protected_spec(
    executable: &Path,
    protected: &ProtectedSandbox,
    workspace: &Path,
    codex: Option<PathBuf>,
    state_dir: Option<&Path>,
    extra_readable: &[PathBuf],
    evidence: &Path,
) -> Result<infra_host::sandbox::Spec> {
    let mut readable = extra_readable.to_vec();
    readable.push(evidence.to_path_buf());
    let mut spec = infra_system::workers::runtime_profile(
        executable,
        &protected.protected,
        workspace,
        codex,
        &readable,
    )?;
    spec.target = protected.protected[0].clone();
    spec.evidence = Some(evidence.to_path_buf());
    if let Some(engine_dir) = protected.engine.as_deref().and_then(Path::parent) {
        spec.runtime.push(engine_dir.to_path_buf());
    }
    if let Some(state_dir) = state_dir {
        spec.accounts.push(state_dir.to_path_buf());
    }
    Ok(spec)
}
struct ProtectedLaunchAdapter {
    inner: Arc<dyn provider::LaunchAdapter>,
    protected: ProtectedSandbox,
    workspace: PathBuf,
    codex: Option<PathBuf>,
    state_dir: Option<PathBuf>,
    evidence: PathBuf,
}
impl provider::LaunchAdapter for ProtectedLaunchAdapter {
    fn adapt(
        &self,
        input: &Launch,
        command: infra_host::provider::PreparedCommand,
    ) -> Result<infra_host::provider::PreparedCommand> {
        let mut command = self.inner.adapt(input, command)?;
        #[cfg(windows)]
        if input.provider == "codex"
            && let Some(position) = command.argv.iter().position(|value| value == "--sandbox")
        {
            command.argv.splice(
                position..position + 2,
                [
                    "-c".into(),
                    "default_permissions=\":danger-full-access\"".into(),
                ],
            );
        }
        let spec = build_protected_spec(
            &input.executable,
            &self.protected,
            &self.workspace,
            self.codex.clone(),
            self.state_dir.as_deref(),
            &command.readable,
            &self.evidence,
        )?;
        command.argv = infra_host::sandbox::discovery(&spec, &command.argv)?;
        Ok(command)
    }
}
pub struct Sessions {
    pub store: SessionStore,
    pub source: PathBuf,
    pub providers: Arc<ProviderDomain>,
    pub knowledge: Arc<KnowledgeDomain>,
    pub blackboards: Arc<dyn BlackboardFactory>,
    pub special: BTreeMap<String, Arc<dyn SpecialWorkflow>>,
    operation: Arc<tokio::sync::Mutex<()>>,
    checks: Mutex<Option<Arc<dyn crate::results::CheckWorkflow>>>,
    queue: session_scheduler::Queue<Job>,
    active: Mutex<BTreeMap<String, Arc<AtomicBool>>>,
    held: Mutex<Option<String>>,
    admission: Mutex<()>,
    stopping: Arc<AtomicBool>,
    queue_wake: Arc<tokio::sync::Notify>,
    queue_done: Arc<AtomicBool>,
    inbox_wake: Arc<tokio::sync::Notify>,
    inbox_done: Arc<AtomicBool>,
    ready: AtomicBool,
    accelerators: Mutex<Option<Arc<crate::accelerators::Accelerators>>>,
    commands: Mutex<Option<Arc<crate::commands::CommandCatalog>>>,
    results: Mutex<Option<Arc<crate::results::Results>>>,
    graphs: Mutex<Option<Arc<crate::graph::Graphs>>>,
    omnigraph: Mutex<Option<Arc<crate::omnigraph::OmnigraphManager>>>,
    skillcache: Mutex<Option<Arc<crate::skillcache::compile::Runtime>>>,
    protected: Mutex<Option<ProtectedSandbox>>,
    lifecycle: Arc<crate::lifecycle::RunnerLifecycle>,
    /// Permission requests running sessions wait on.
    permissions: Arc<crate::permissions::PermissionDesk>,
    pub default_seconds: u64,
}
fn lock<T>(value: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>> {
    value
        .lock()
        .map_err(|_| HarnessError::unavailable("Session state lock failed."))
}
pub fn validate_prompt(prompt: &str) -> Result<String> {
    if prompt.trim().is_empty() || prompt.len() > 32000 || prompt.contains('\0') {
        return Err(HarnessError::invalid(
            "Enter a task of 1–32000 UTF-8 bytes.",
        ));
    }
    Ok(prompt.trim().to_owned())
}
fn active(session: &Value) -> bool {
    ["queued", "running"].contains(&session["status"].as_str().unwrap_or(""))
}
pub fn budgets(value: &Value, provider: &str, dry: bool) -> Result<Value> {
    if !value.is_object()
        || value
            .as_object()
            .unwrap()
            .keys()
            .any(|k| !["usd", "tokens", "seconds"].contains(&k.as_str()))
    {
        return Err(HarnessError::invalid(
            "Budgets accept only usd, tokens and seconds.",
        ));
    }
    let usd = value["usd"].as_f64();
    if !value["usd"].is_null()
        && (usd.is_none_or(|v| !v.is_finite() || !(0.01..=1000.).contains(&v))
            || provider != "claude" && !dry)
    {
        return Err(HarnessError::invalid(
            "A finite USD cap is available only for Claude.",
        ));
    }
    for (key, max) in [("tokens", 1_000_000_000u64), ("seconds", 86400)] {
        if !value[key].is_null() && value[key].as_u64().is_none_or(|v| v == 0 || v > max) {
            return Err(HarnessError::invalid(format!("Invalid {key} budget.")));
        }
    }
    Ok(json!({"usd":value["usd"],"tokens":value["tokens"],"seconds":value["seconds"]}))
}
fn fleet_options(
    value: &Value,
    provider: &str,
    enabled: bool,
    effort: Option<&str>,
) -> Result<Value> {
    if !value.is_object()
        || value.as_object().unwrap().keys().any(|k| {
            ![
                "lenses",
                "dry_run",
                "budget_usd",
                "worker_timeout",
                "precheck",
            ]
            .contains(&k.as_str())
        })
    {
        return Err(HarnessError::invalid("Invalid Fleet review settings."));
    }
    let defaults = json!([
        "code-reviewer",
        "security-reviewer",
        "performance-optimization"
    ]);
    let lenses = value
        .get("lenses")
        .unwrap_or(&defaults)
        .as_array()
        .ok_or_else(|| HarnessError::invalid("Choose supported Fleet lenses."))?;
    let dry = value["dry_run"].as_bool().unwrap_or(false);
    let seconds = match value.get("worker_timeout") {
        Some(timeout) => timeout.as_u64().ok_or_else(|| {
            HarnessError::invalid("Use a reviewer timeout of 1–3600 whole seconds.")
        })?,
        None => 300,
    };
    let mut seen = std::collections::BTreeSet::new();
    if lenses.is_empty()
        || lenses.len() > 3
        || lenses.iter().any(|v| {
            ![
                "code-reviewer",
                "security-reviewer",
                "performance-optimization",
            ]
            .contains(&v.as_str().unwrap_or(""))
                || !seen.insert(v.to_string())
        })
        || !(1..=3600).contains(&seconds)
        || value.get("dry_run").is_some_and(|v| !v.is_boolean())
        || (!dry && !enabled)
        || effort == Some("ultracode")
    {
        return Err(HarnessError::invalid(
            "Invalid Fleet lenses, timeout or agent settings.",
        ));
    }
    let budget = value["budget_usd"].as_f64();
    if !value["budget_usd"].is_null()
        && (budget.is_none_or(|n| !n.is_finite() || !(0.01..=1000.).contains(&n))
            || provider != "claude" && !dry)
    {
        return Err(HarnessError::invalid(
            "Native Fleet USD limits require Claude.",
        ));
    }
    let precheck = crate::clash::validate_precheck(value.get("precheck"))?;
    let mut out =
        json!({"lenses":lenses,"dry_run":dry,"budget_usd":budget,"worker_timeout":seconds});
    if !precheck.is_null() {
        out["precheck"] = precheck;
    }
    Ok(out)
}
fn followup_settings(session: &Value, options: &Value, catalog: &Value) -> Result<Value> {
    let workflow = session["workflow"].as_str().unwrap_or("native");
    let sdd = crate::sdd::validate(
        options.get("sdd").unwrap_or(&session["sdd"]),
        workflow,
        Some(&session["sdd"]),
    )?;
    let mode = if let Some(settings) = sdd.as_ref() {
        crate::sdd::mode(settings)
    } else {
        options
            .get("mode")
            .unwrap_or(&session["mode"])
            .as_str()
            .ok_or_else(|| HarnessError::invalid("Invalid mode for this workflow."))?
    };
    if !["plan", "edit"].contains(&mode)
        || workflow != "native" && options.get("mode").is_some_and(|value| value != mode)
    {
        return Err(HarnessError::invalid("Invalid mode for this workflow."));
    }
    let mut fields = options.clone();
    fields["sdd"] = sdd.clone().unwrap_or(Value::Null);
    fields["mode"] = json!(mode);
    if let Some(permissions) = options.get("permissions") {
        fields["permissions"] = crate::permissions::validate(permissions)?;
    }
    let provider = session["provider"].as_str().unwrap_or("");
    let (model, effort, routing) = infra_host::model_catalog::routed_model_settings(
        catalog,
        provider,
        &fields,
        workflow,
        mode,
        sdd.as_ref(),
        Some(session),
    )?;
    if sdd.is_none()
        && mode != session["mode"].as_str().unwrap_or("plan")
        && (workflow != "native" || routing.is_null())
    {
        return Err(HarnessError::invalid(
            "A mode change requires native model routing.",
        ));
    }
    fields["model"] = model;
    fields["thinking_effort"] = effort;
    fields["model_routing"] = routing;
    Ok(fields)
}
fn updated_budget_settings(session: &Value, data: &Value) -> Result<(Value, Value)> {
    if !store::exact(data, &["budgets", "revision"], &["worker_timeout"]) {
        return Err(HarnessError::invalid(
            "Provide budgets and their current revision.",
        ));
    }
    if !session["system_run"].is_null() || !session["system_discovery"].is_null() {
        return Err(HarnessError::conflict(
            "Use System Orchestration to control this run.",
        ));
    }
    if !session["creator"].is_null() {
        return Err(HarnessError::conflict(
            "Change Creator budgets at its review checkpoint.",
        ));
    }
    if active(session) {
        return Err(HarnessError::conflict(
            "Wait for this launch to finish before changing its budgets.",
        ));
    }
    let revision = data["revision"]
        .as_i64()
        .filter(|revision| *revision >= 0)
        .ok_or_else(|| HarnessError::invalid("Invalid budget revision."))?;
    if session["budget_revision"].as_i64() != Some(revision) {
        return Err(HarnessError::conflict(
            "Budget settings changed; refresh the view.",
        ));
    }
    let provider = session["provider"].as_str().unwrap_or("");
    let value = budgets(
        &data["budgets"],
        provider,
        session["fleet"]["dry_run"] == true,
    )?;
    let mut fleet = session["fleet"].clone();
    if !fleet.is_null() && !fleet.is_object() {
        return Err(HarnessError::invalid("Invalid saved Fleet settings."));
    }
    if fleet.is_null() {
        if data.get("worker_timeout").is_some() {
            return Err(HarnessError::invalid(
                "Reviewer time budgets require Fleet review.",
            ));
        }
    } else {
        fleet["budget_usd"] = value["usd"].clone();
        if let Some(timeout) = data.get("worker_timeout") {
            if timeout
                .as_u64()
                .is_none_or(|seconds| !(1..=3600).contains(&seconds))
            {
                return Err(HarnessError::invalid(
                    "Use a reviewer timeout of 1–3600 whole seconds.",
                ));
            }
            fleet["worker_timeout"] = timeout.clone();
        }
        fleet = fleet_options(
            &fleet,
            provider,
            session["agents_enabled"] == true,
            session["thinking_effort"].as_str(),
        )?;
    }
    Ok((value, fleet))
}
fn save_budget_settings(store: &SessionStore, sid: &str, data: &Value) -> Result<Value> {
    let mut db = store.connection()?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let session = store::get_session(&tx, sid)?;
    let (budgets, fleet) = updated_budget_settings(&session, data)?;
    let at = store::now();
    let fleet_text = (!fleet.is_null()).then(|| fleet.to_string());
    if tx.execute("UPDATE sessions SET budgets=?1,fleet=?2,budget_revision=budget_revision+1,updated_at=?3 WHERE id=?4 AND budget_revision=?5",
        params![budgets.to_string(),fleet_text,at,sid,data["revision"].as_i64()])?!=1 {
        return Err(HarnessError::conflict("Budget settings changed; refresh the view."));
    }
    tx.execute("INSERT INTO events(session_id,data)VALUES(?1,?2)",params![sid,
        json!({"kind":"status","text":"Budgets saved for the next launch. Previous usage is retained.","at":at}).to_string()])?;
    let saved = store::get_session(&tx, sid)?;
    tx.commit()?;
    Ok(saved)
}
/// Observes a tool event in `LiveFocusTracker` and forwards completed workspace file mutations
/// to `Graphs::record_live_paths` when a code graph manager is installed.
fn observe_tool_event_with_graphs(
    tracker: &mut crate::graph_live::LiveFocusTracker,
    event: &Value,
    graphs: Option<&crate::graph::Graphs>,
    project_id: &str,
    workspace: &Path,
) -> Option<Value> {
    let delta = tracker.observe_tool_event(event);
    if event["kind"] == "tool"
        && event["state"].as_str().unwrap_or("") != "started"
        && event["ok"] != false
        && let Some(graphs) = graphs
        && !project_id.is_empty()
    {
        let tool = event["tool"].as_str().unwrap_or("").to_ascii_lowercase();
        let has_changes = event["changes"]
            .as_array()
            .is_some_and(|changes| !changes.is_empty());
        let is_mutation = matches!(
            tool.as_str(),
            "edit" | "write" | "multiedit" | "notebookedit" | "file_change" | "delete"
        ) || has_changes;
        if is_mutation {
            let mut mutated = std::collections::BTreeSet::new();
            if let Some(changes) = event["changes"].as_array().filter(|arr| !arr.is_empty()) {
                for change in changes {
                    if change["outside"] != true
                        && let Some(raw) = change["path"].as_str()
                        && let Some(rel) = crate::graph_ir::normalize_rel_path(workspace, raw)
                    {
                        mutated.insert(rel);
                    }
                }
            } else if event["outside"] != true
                && let Some(raw) = event["path"].as_str()
                && let Some(rel) = crate::graph_ir::normalize_rel_path(workspace, raw)
            {
                mutated.insert(rel);
            }
            if !mutated.is_empty() {
                let paths: Vec<String> = mutated.into_iter().collect();
                graphs.record_live_paths(project_id, &paths);
            }
        }
    }
    delta
}
/// Drains SkillCache executor journal + `run-log.changes.jsonl` mutations into session
/// events, live focus tracking, and incremental code-graph updates.
#[allow(clippy::too_many_arguments)]
fn ingest_warm_with_live_focus(
    warm: &crate::skillcache::warm::Warm,
    store: &SessionStore,
    sid: &str,
    project_id: &str,
    workspace: &Path,
    graphs: Option<&crate::graph::Graphs>,
    live_focus: &std::sync::Mutex<crate::graph_live::LiveFocusTracker>,
    emit: &dyn Fn(Value) -> Result<()>,
) {
    warm.ingest_with_changes(emit, &|change_event| {
        if let Ok(mut tracker) = live_focus.lock()
            && let Some(delta) = observe_tool_event_with_graphs(
                &mut tracker,
                &change_event,
                graphs,
                project_id,
                workspace,
            )
        {
            let _ = crate::graph_live::save_session_focus(store, sid, tracker.focus());
            let _ = emit(delta);
        }
    });
}
impl Sessions {
    pub fn new(
        store: SessionStore,
        source: PathBuf,
        providers: Arc<ProviderDomain>,
        knowledge: Arc<KnowledgeDomain>,
        blackboards: Arc<dyn BlackboardFactory>,
        special: BTreeMap<String, Arc<dyn SpecialWorkflow>>,
        limits: SessionLimits,
    ) -> Result<Arc<Self>> {
        if !(1..=session_scheduler::MAX_CONCURRENCY).contains(&limits.concurrency) {
            return Err(HarnessError::invalid("Session concurrency must be 1–4."));
        }
        for name in ["creator", "system_run", "system_discovery"] {
            if !special.contains_key(name) {
                return Err(HarnessError::invalid(
                    "Every native special workflow must be installed before starting the session queue.",
                ));
            }
        }
        let (queue, rx) = session_scheduler::channel();
        let lifecycle = Arc::new(
            crate::lifecycle::RunnerLifecycle::new(
                store.clone(),
                knowledge.clone(),
                source.clone(),
            )
            .with_queue_engine(std::env::current_exe()?),
        );
        let result = Arc::new(Self {
            store,
            source,
            providers,
            knowledge,
            blackboards,
            special,
            operation: Arc::new(tokio::sync::Mutex::new(())),
            checks: Mutex::new(None),
            queue,
            active: Mutex::new(BTreeMap::new()),
            held: Mutex::new(None),
            admission: Mutex::new(()),
            stopping: Arc::new(AtomicBool::new(false)),
            queue_wake: Arc::new(tokio::sync::Notify::new()),
            queue_done: Arc::new(AtomicBool::new(false)),
            inbox_wake: Arc::new(tokio::sync::Notify::new()),
            inbox_done: Arc::new(AtomicBool::new(false)),
            ready: AtomicBool::new(false),
            accelerators: Mutex::new(None),
            commands: Mutex::new(None),
            results: Mutex::new(None),
            graphs: Mutex::new(None),
            omnigraph: Mutex::new(None),
            skillcache: Mutex::new(None),
            protected: Mutex::new(None),
            lifecycle,
            permissions: Arc::default(),
            default_seconds: limits.seconds,
        });
        let runner = Arc::downgrade(&result);
        let classifier = runner.clone();
        let failures = runner.clone();
        let stopping = result.stopping.clone();
        let wake = result.queue_wake.clone();
        let done = result.queue_done.clone();
        let parallel = limits.concurrency > 1;
        tokio::spawn(async move {
            session_scheduler::run(
                rx,
                limits.concurrency,
                stopping,
                wake,
                move |job: &Job| {
                    let runner = classifier.upgrade();
                    let job = job.clone();
                    async move {
                        match runner {
                            Some(runner) if parallel => runner.job_lease(job).await,
                            _ => Lease::exclusive(&job.sid),
                        }
                    }
                },
                move |job, lease| {
                    let runner = runner.upgrade();
                    async move {
                        if let Some(runner) = runner {
                            Box::pin(runner.execute_job(job, Some(lease))).await;
                        }
                    }
                },
                move || {
                    if let Some(runner) = failures.upgrade() {
                        let _ = runner.close();
                    }
                },
            )
            .await;
            done.store(true, Ordering::Release);
        });
        let weak_desk = Arc::downgrade(&result.permissions);
        result.store.set_waiting_counter(Arc::new(move |sid| {
            weak_desk
                .upgrade()
                .map_or(0, |desk| desk.waiting(sid).len())
        }));
        let weak_desk_events = Arc::downgrade(&result.permissions);
        let store_for_events = result.store.clone();
        result.permissions.set_change_notifier(Arc::new(move |sid| {
            let counts = weak_desk_events
                .upgrade()
                .map(|desk| desk.waiting_counts())
                .unwrap_or_default();
            store_for_events.emit_waiting_change(sid, counts);
        }));
        result.start_inbox();
        Ok(result)
    }
    pub fn admit(&self) -> Result<()> {
        if !self.ready.load(Ordering::Acquire) {
            return Err(HarnessError::unavailable(
                "Native session services are still initializing.",
            ));
        }
        if self.stopping.load(Ordering::Acquire) {
            return Err(HarnessError::conflict("The server is stopping."));
        }
        if let Some(reason) = lock(&self.held)?.as_ref() {
            return Err(HarnessError::conflict(reason.clone()));
        }
        if self.queue.capacity() == 0 {
            return Err(HarnessError::conflict("The run queue is full."));
        }
        Ok(())
    }
    pub fn has_active(&self) -> Result<bool> {
        let db = self.store.connection()?;
        let pending: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE status IN ('queued','running'))",
            [],
            |r| r.get(0),
        )?;
        Ok(pending || !lock(&self.active)?.is_empty())
    }
    pub fn hold(&self, reason: &str) -> Result<()> {
        let _guard = lock(&self.admission)?;
        if self.has_active()? {
            return Err(HarnessError::conflict(
                "Wait for active runs and their cleanup before updating.",
            ));
        }
        let inbox_active: bool = self.store.connection()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM session_inbox WHERE status IN ('pending','assigned'))",
            [],
            |row| row.get(0),
        )?;
        if inbox_active {
            return Err(HarnessError::conflict(
                "Finish or cancel queued inbox messages before updating.",
            ));
        }
        *lock(&self.held)? = Some(reason.into());
        Ok(())
    }
    pub fn release(&self) -> Result<()> {
        *lock(&self.held)? = None;
        self.inbox_wake.notify_one();
        Ok(())
    }
    pub fn cancel(&self, sid: &str) -> Result<Value> {
        // Follow-up admission holds the same lock. Cancellation cannot select
        // one generation and then signal a token belonging to its replacement.
        let _admission = lock(&self.admission)?;
        let session = self.store.cancel_current(sid)?;
        if session["status"] == "cancelled"
            && let Some(token) = lock(&self.active)?.get(sid)
        {
            token.store(true, Ordering::Release);
        }
        Ok(session)
    }
    pub fn close(&self) -> Result<()> {
        self.stopping.store(true, Ordering::Release);
        self.queue_wake.notify_one();
        self.inbox_wake.notify_one();
        let _admission = lock(&self.admission)?;
        for token in lock(&self.active)?.values() {
            token.store(true, Ordering::Release);
        }
        self.queue_wake.notify_one();
        Ok(())
    }
    pub fn launch_internal(
        &self,
        data: &Value,
        creator: Option<&Value>,
        system: Option<&Value>,
        discovery: Option<&Value>,
    ) -> Result<Value> {
        let mut value = self.create_options(data)?;
        for (name, metadata) in [
            ("creator", creator),
            ("system_run", system),
            ("system_discovery", discovery),
        ] {
            if let Some(metadata) = metadata {
                value[name] = metadata.clone();
            }
        }
        self.create_ready(value)
    }
    fn create_options(&self, data: &Value) -> Result<Value> {
        let allowed = [
            "project_id",
            "provider",
            "prompt",
            "mode",
            "workflow",
            "model",
            "thinking_effort",
            "project_context",
            "agents_enabled",
            "agent_count",
            "workspace",
            "worktree_branch",
            "worktree_id",
            "fleet",
            "brain",
            "budgets",
            "sdd",
            "model_routing",
            "attachments",
            "clash",
            "system_explain",
            "omnigraph",
            "permissions",
            "skillcache",
            "skillcache_thin_formatter",
        ];
        if !data.is_object()
            || data
                .as_object()
                .unwrap()
                .keys()
                .any(|k| !allowed.contains(&k.as_str()))
        {
            return Err(HarnessError::invalid("Unknown session option."));
        }
        let project = self.store.project(
            data["project_id"]
                .as_str()
                .ok_or_else(|| HarnessError::invalid("Select a registered project."))?,
        )?;
        let provider = data["provider"]
            .as_str()
            .ok_or_else(|| HarnessError::invalid("Select a provider."))?;
        let discovered = self.providers.get(provider)?;
        let mode = data["mode"].as_str().unwrap_or("plan");
        let workflow = data["workflow"].as_str().unwrap_or("native");
        if !["plan", "edit"].contains(&mode)
            || !["native", "plan", "review", "sdd", "fleet-review"].contains(&workflow)
            || !["native", "sdd"].contains(&workflow) && mode != "plan"
        {
            return Err(HarnessError::invalid("Unknown mode or workflow."));
        }
        let enabled = data["agents_enabled"].as_bool().unwrap_or(false);
        let count = data["agent_count"].as_u64().unwrap_or(3);
        if data.get("agents_enabled").is_some_and(|v| !v.is_boolean())
            || data.get("agent_count").is_some_and(|v| !v.is_u64())
            || !(1..=40).contains(&count)
            || data.get("project_context").is_some_and(|v| !v.is_boolean())
        {
            return Err(HarnessError::invalid(
                "Invalid session boolean or agent count.",
            ));
        }
        for key in ["model", "thinking_effort"] {
            if !data[key].is_null() {
                let value = store::text(&data[key], key, 120)?;
                if value.starts_with('-') || value.chars().any(|c| c.is_control()) {
                    return Err(HarnessError::invalid("Invalid model or thinking effort."));
                }
            }
        }
        let mut value = data.clone();
        value["mode"] = mode.into();
        value["workflow"] = workflow.into();
        value["agents_enabled"] = enabled.into();
        value["agent_count"] = count.into();
        value["permissions"] = crate::permissions::validate(&data["permissions"])?;
        value["skillcache"] = crate::skillcache::settings::validate(&data["skillcache"])?;
        if let Some(tf) = data.get("skillcache_thin_formatter")
            && !tf.is_null()
            && !tf.is_boolean()
        {
            return Err(HarnessError::invalid(
                "`skillcache_thin_formatter` must be true, false, or null.",
            ));
        }
        crate::skillcache::settings::admit_with_eval(
            &value,
            crate::skillcache::settings::eval_enabled(&self.store),
        )?;
        let fleet = if workflow == "fleet-review" {
            fleet_options(
                data.get("fleet").unwrap_or(&json!({})),
                provider,
                enabled,
                data["thinking_effort"].as_str(),
            )?
        } else {
            if data.get("fleet").is_some() {
                return Err(HarnessError::invalid(
                    "Fleet settings require Fleet review.",
                ));
            }
            Value::Null
        };
        let dry = fleet["dry_run"] == true;
        let is_system_explain = data.get("system_explain").is_some_and(|v| !v.is_null());
        if is_system_explain {
            if mode != "plan"
                || data.get("project_context") != Some(&Value::Bool(false))
                || workflow != "native"
                || data["workspace"].as_str().unwrap_or("project") != "project"
                || enabled
            {
                return Err(HarnessError::invalid(
                    "System explanation requires a read-only plan session without project context.",
                ));
            }
            value["system_explain"] = crate::system_graph::validate_system_explain(
                &self.store,
                &project,
                &data["system_explain"],
            )?;
        } else {
            value["system_explain"] = Value::Null;
        }
        let default =
            json!({"seconds": if is_system_explain { 300 } else { self.default_seconds }});
        let mut parsed_budgets = budgets(data.get("budgets").unwrap_or(&default), provider, dry)?;
        if is_system_explain {
            match parsed_budgets["seconds"].as_u64() {
                None => {
                    parsed_budgets["seconds"] = json!(300);
                }
                Some(secs) if secs > 300 => {
                    return Err(HarnessError::invalid(
                        "System explanation sessions allow at most 300 seconds.",
                    ));
                }
                Some(_) => {}
            }
        }
        if !discovered.available && !dry {
            return Err(HarnessError::unavailable(
                "This provider CLI is unavailable.",
            ));
        }
        if dry
            && (!data["model"].is_null()
                || !data["thinking_effort"].is_null()
                || !data["brain"].is_null())
        {
            return Err(HarnessError::invalid(
                "Offline dry-run does not use model, effort or linked memory.",
            ));
        }
        value["fleet"] = fleet;
        value["budgets"] = parsed_budgets;
        if !value["fleet"].is_null() {
            value["budgets"]["usd"] = value["fleet"]["budget_usd"].clone();
        }
        value["sdd"] = crate::sdd::validate(&data["sdd"], workflow, None)?.unwrap_or(Value::Null);
        if !value["sdd"].is_null() {
            value["mode"] = crate::sdd::mode(&value["sdd"]).into();
        }
        let catalog = self.providers.catalog(provider)?;
        let (model, effort, routing) = infra_host::model_catalog::routed_model_settings(
            &catalog,
            provider,
            &value,
            workflow,
            value["mode"].as_str().unwrap_or(mode),
            (!value["sdd"].is_null()).then_some(&value["sdd"]),
            None,
        )?;
        value["model"] = model;
        value["thinking_effort"] = effort;
        value["model_routing"] = routing;
        value["clash"] =
            crate::clash::validate(&data["clash"], workflow, provider)?.unwrap_or(Value::Null);
        if !value["clash"].is_null() {
            let challenger = value["clash"]["challenger"].as_str().unwrap_or("");
            if !self.providers.get(challenger)?.available {
                return Err(HarnessError::unavailable(
                    "The challenger CLI is unavailable.",
                ));
            }
            infra_host::model_catalog::validate_model_effort(
                &self.providers.catalog(challenger)?,
                challenger,
                value["clash"]["challenger_model"].as_str(),
                value["clash"]["challenger_thinking_effort"].as_str(),
                false,
            )?;
        }
        if !value["clash"].is_null() && enabled {
            return Err(HarnessError::invalid(
                "Clash participants run without additional agents.",
            ));
        }
        let prompt = validate_prompt(
            data["prompt"]
                .as_str()
                .ok_or_else(|| HarnessError::invalid("Enter a task."))?,
        )?;
        self.commands()?.check(provider, &prompt, None)?;
        value["brain"] = crate::brain_settings::validate(&data["brain"], &prompt)?;
        value["prompt"] = prompt.into();
        let sid = store::id();
        value["omnigraph"] = validate_omnigraph_options(&data["omnigraph"], &sid)?;
        if value["omnigraph"].is_object() {
            enrich_linked_pull_branches(
                &self.store,
                project["id"].as_str().unwrap_or(""),
                &mut value["omnigraph"],
            )?;
        }
        let (root, branch, common) = projects::workspace(&self.store, &project, &value, &sid)?;
        value["_session_id"] = sid.into();
        value["_workspace_path"] = root.to_string_lossy().into_owned().into();
        value["branch"] = json!(branch);
        value["git_common_dir"] = json!(common);
        if !value["sdd"].is_null() {
            let ir_context = project["id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .map(|id| (self.store.state.as_path(), id));
            crate::sdd::require_documents_in(&root, &value["sdd"], ir_context)?;
        }
        Ok(value)
    }
    #[cfg(test)]
    pub(crate) fn create_options_for_test(&self, data: &Value) -> Result<Value> {
        self.create_options(data)
    }
    fn create_ready(&self, options: Value) -> Result<Value> {
        let _operation = self
            .operation
            .try_lock()
            .map_err(|_| HarnessError::conflict("A workspace operation is completing."))?;
        let _admission = lock(&self.admission)?;
        self.admit()?;
        let sid = options["_session_id"]
            .as_str()
            .ok_or_else(|| HarnessError::invalid("Missing session identity."))?;
        let uploads = attachments::validate(options.get("attachments").unwrap_or(&json!([])))?;
        let session = self.store.create_uploaded(
            &options,
            options["prompt"].as_str().unwrap_or(""),
            &uploads,
        )?;
        sync_reciprocal_session_links(&self.store, &session);
        let job = Job {
            sid: sid.into(),
            prompt: options["prompt"].as_str().unwrap_or("").into(),
            generation: self.store.generation(sid)?,
            kind: JobKind::Run,
        };
        if self.queue.try_send(job).is_err() {
            self.store.patch(sid, &json!({"status":"interrupted"}))?;
            return Err(HarnessError::conflict(
                "The run queue is full; this session was saved.",
            ));
        }
        Ok(session)
    }
    fn enqueue(&self, sid: &str, prompt: &str, fields: &Value) -> Result<Value> {
        self.enqueue_uploaded(sid, prompt, fields, &[])
    }
    fn enqueue_uploaded(
        &self,
        sid: &str,
        prompt: &str,
        fields: &Value,
        uploads: &[attachments::Upload],
    ) -> Result<Value> {
        let _operation = self
            .operation
            .try_lock()
            .map_err(|_| HarnessError::conflict("A workspace operation is completing."))?;
        let _admission = lock(&self.admission)?;
        self.admit()?;
        let session = self.store.requeue_uploaded(
            sid,
            (!prompt.is_empty()).then_some(prompt),
            fields,
            uploads,
        )?;
        let actual = if prompt.is_empty() && session["fleet"].is_null() {
            self.latest_user(sid, false)?
        } else {
            prompt.to_string()
        };
        let job = Job {
            sid: sid.into(),
            prompt: actual,
            generation: self.store.generation(sid)?,
            kind: JobKind::Run,
        };
        if self.queue.try_send(job).is_err() {
            self.store.patch(sid, &json!({"status":"interrupted"}))?;
            return Err(HarnessError::conflict(
                "The run queue is full; this session was saved.",
            ));
        }
        Ok(session)
    }
    pub fn activate(&self) -> Result<()> {
        if lock(&self.accelerators)?.is_none()
            || lock(&self.commands)?.is_none()
            || lock(&self.results)?.is_none()
            || lock(&self.checks)?.is_none()
        {
            return Err(HarnessError::unavailable(
                "Every native session service must be bound before admission.",
            ));
        }
        self.ready.store(true, Ordering::Release);
        self.inbox_wake.notify_one();
        Ok(())
    }
    pub fn install_accelerators(
        &self,
        value: Arc<crate::accelerators::Accelerators>,
    ) -> Result<()> {
        let mut slot = lock(&self.accelerators)?;
        if slot.is_some() {
            return Err(HarnessError::conflict(
                "Accelerators were already installed.",
            ));
        }
        *slot = Some(value);
        Ok(())
    }
    pub fn install_commands(&self, value: Arc<crate::commands::CommandCatalog>) -> Result<()> {
        let mut slot = lock(&self.commands)?;
        if slot.is_some() {
            return Err(HarnessError::conflict("Commands were already installed."));
        }
        *slot = Some(value);
        Ok(())
    }
    /// The code graph is optional: a session queue without one runs unchanged.
    pub fn install_graphs(&self, value: Arc<crate::graph::Graphs>) -> Result<()> {
        *lock(&self.graphs)? = Some(value);
        Ok(())
    }
    /// SkillCache is optional: without it a session that asks for warm turns
    /// still decides them, with this executable as the engine.
    pub fn install_skillcache(
        &self,
        value: Arc<crate::skillcache::compile::Runtime>,
    ) -> Result<()> {
        *lock(&self.skillcache)? = Some(value);
        Ok(())
    }
    /// Configures protected paths that every session run must not read
    /// (verified with `boundary_request_with` and wrapped via `sandbox::discovery`).
    pub fn install_protected(
        &self,
        protected: Vec<PathBuf>,
        engine: Option<PathBuf>,
    ) -> Result<()> {
        let mut canonical = Vec::with_capacity(protected.len());
        for path in protected {
            canonical.push(infra_core::fs::canonicalize(&path)?);
        }
        *lock(&self.protected)? = (!canonical.is_empty()).then_some(ProtectedSandbox {
            protected: canonical,
            engine,
        });
        Ok(())
    }
    #[inline(never)]
    async fn attach_protected(
        &self,
        inner: Arc<dyn provider::LaunchAdapter>,
        protected_cfg: ProtectedSandbox,
        executable: &Path,
        workspace: &Path,
        warm: Option<&crate::skillcache::warm::Warm>,
        cancel: &Arc<AtomicBool>,
    ) -> Result<Arc<ProtectedLaunchAdapter>> {
        let evidence_dir = self.store.state.join("skillcache/protected-probe");
        infra_host::util::private_directory(&self.store.state.join("skillcache"))?;
        infra_core::fs::ensure_directory(&evidence_dir, true)?;
        let probe_file = evidence_dir.join("probe.txt");
        if !probe_file.exists() {
            infra_core::fs::atomic_write(&probe_file, b"ok\n")?;
        }
        let codex = if cfg!(windows) {
            self.providers
                .get("codex")
                .ok()
                .and_then(|provider| provider.executable)
        } else {
            None
        };
        let warm_state_dir = warm.and_then(|w| w.state_dir.clone());
        let mut extra_readable = Vec::new();
        if let Some(w) = warm {
            if let Some(bin) = &w.bin {
                extra_readable.push(bin.clone());
                for name in ["skillcache-run", "skillcache-tool"] {
                    let link = bin.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
                    if let Ok(target) = std::fs::read_link(&link)
                        && let Some(parent) = target.parent()
                        && let Ok(canonical_parent) = infra_core::fs::canonicalize(parent)
                        && !protected_cfg.protected.iter().any(|p| {
                            canonical_parent.starts_with(p) || p.starts_with(&canonical_parent)
                        })
                    {
                        extra_readable.push(canonical_parent);
                    }
                }
            }
            if let Some(pkg) = &w.package {
                extra_readable.push(pkg.clone());
            }
        }
        let spec = build_protected_spec(
            executable,
            &protected_cfg,
            workspace,
            codex.clone(),
            warm_state_dir.as_deref(),
            &extra_readable,
            &evidence_dir,
        )?;
        let denied = crate::skillcache::compile::sample_paths(&protected_cfg.protected, 2000);
        let boundary = infra_host::sandbox::boundary_request_with(
            &spec,
            denied,
            protected_cfg.engine.as_deref(),
        )?;
        let probe_env = std::env::vars().collect::<BTreeMap<_, _>>();
        let probe = Box::pin(infra_system::native::run_raw_monitored(
            &boundary.command,
            workspace,
            &probe_env,
            &[],
            &[],
            60,
            cancel.clone(),
            Some(self.store.runner_lock()),
            None,
            |_| Ok(()),
        ))
        .await?;
        let attested = probe
            .bytes
            .split(|byte| *byte == b'\n')
            .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
            .next_back()
            .unwrap_or(Value::Null);
        let verified = probe.code == 0
            && attested["ok"] == true
            && attested["denied"].as_u64() == Some(boundary.denied as u64)
            && attested["evidence_read_only"] == true
            && attested["workspace_writable"] == true;
        let failure_msg =
            (!verified).then(|| infra_system::workers::probe_failure(&probe.bytes, probe.code));
        drop(boundary);
        if let Some(message) = failure_msg {
            return Err(HarnessError::forbidden(message));
        }
        Ok(Arc::new(ProtectedLaunchAdapter {
            inner,
            protected: protected_cfg,
            workspace: workspace.to_path_buf(),
            codex,
            state_dir: warm_state_dir,
            evidence: evidence_dir,
        }))
    }
    #[inline(never)]
    async fn verify_direct_protected(
        &self,
        protected_cfg: &ProtectedSandbox,
        workspace: &Path,
        warm: &crate::skillcache::warm::Warm,
        cancel: &Arc<AtomicBool>,
    ) -> Result<()> {
        struct ProbeCleanup(PathBuf, Vec<PathBuf>);
        impl Drop for ProbeCleanup {
            fn drop(&mut self) {
                for file in &self.1 {
                    let _ = std::fs::remove_file(file);
                }
                infra_skillcache::package::release(&self.0);
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let state_root = self.store.state.join("skillcache");
        infra_host::util::private_directory(&state_root)?;
        let probe_root =
            state_root.join(format!("direct-protected-probe-{}", crate::store::hexid()));
        let mut cleanup = ProbeCleanup(probe_root.clone(), Vec::new());
        let probe_ws = probe_root.join("work");
        let evidence_dir = probe_root.join("evidence");
        infra_core::fs::ensure_directory(&probe_ws, true)?;
        infra_core::fs::ensure_directory(&probe_ws.join(".harness-tmp"), true)?;
        infra_core::fs::ensure_directory(&evidence_dir, true)?;
        let probe_file = evidence_dir.join("probe.txt");
        infra_core::fs::atomic_write(&probe_file, b"ok\n")?;
        let canon_probe_ws = infra_core::fs::canonicalize(&probe_ws)?;
        let canon_evidence = infra_core::fs::canonicalize(&evidence_dir)?;
        let probe_engine = warm.engine.clone().or_else(|| protected_cfg.engine.clone());
        let helper_path = match &probe_engine {
            Some(path) => path.clone(),
            None => infra_host::sandbox::trusted_helper(&canon_probe_ws)?,
        };
        let mut readable = vec![
            infra_core::fs::canonicalize(workspace)?,
            canon_evidence.clone(),
        ];
        for extra in [
            warm.package.as_deref(),
            probe_engine.as_deref().and_then(Path::parent),
            warm.engine.as_deref().and_then(Path::parent),
        ]
        .into_iter()
        .flatten()
        {
            let canon = infra_core::fs::canonicalize(extra)?;
            if !readable.contains(&canon) {
                readable.push(canon);
            }
        }
        let mut accounts = Vec::new();
        if let Some(state_dir) = &warm.state_dir {
            infra_core::fs::ensure_directory(state_dir, true)?;
            accounts.push(infra_core::fs::canonicalize(state_dir)?);
        }
        let runtime = if let Some(pkg) = &warm.package
            && let Ok(loaded) = infra_skillcache::package::load(pkg)
        {
            crate::skillcache::gates::gate_runtime_dirs(&loaded.capsule.capabilities.runtimes)
        } else {
            Vec::new()
        };
        let spec = infra_host::sandbox::Spec {
            workspace: canon_probe_ws.clone(),
            target: canon_probe_ws.clone(),
            protected: protected_cfg.protected.clone(),
            readable,
            accounts,
            runtime,
            codex: warm.codex.clone(),
            evidence: Some(canon_evidence.clone()),
            network: false,
        };
        let mut denied_paths =
            crate::skillcache::compile::sample_paths(&protected_cfg.protected, 2000);
        let sentinel =
            std::env::temp_dir().join(format!("harness-direct-sentinel-{}", crate::store::hexid()));
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&sentinel)?;
        infra_host::util::private_file(&sentinel)?;
        cleanup.1.push(sentinel.clone());
        denied_paths.push(sentinel);
        let mut lines: Vec<String> = denied_paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let expected_denied = lines.len();
        lines.push("--readable".into());
        lines.push(
            infra_core::fs::canonicalize(&probe_file)?
                .to_string_lossy()
                .into_owned(),
        );
        let listing = probe_root.join("boundary-paths.txt");
        infra_core::fs::atomic_write(&listing, format!("{}\n", lines.join("\n")).as_bytes())?;
        let argv = vec![
            helper_path.to_string_lossy().into_owned(),
            "internal-discovery-probe".into(),
            listing.to_string_lossy().into_owned(),
            canon_probe_ws.to_string_lossy().into_owned(),
            canon_evidence.to_string_lossy().into_owned(),
        ];
        let command = infra_host::sandbox::gate(&spec, &argv)?;
        let probe_env = std::env::vars().collect::<BTreeMap<_, _>>();
        let probe = Box::pin(infra_system::native::run_raw_monitored(
            &command,
            workspace,
            &probe_env,
            &[],
            &[],
            60,
            cancel.clone(),
            Some(self.store.runner_lock()),
            None,
            |_| Ok(()),
        ))
        .await?;
        let attested = probe
            .bytes
            .split(|byte| *byte == b'\n')
            .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
            .next_back()
            .unwrap_or(Value::Null);
        let verified = probe.code == 0
            && attested["ok"] == true
            && attested["denied"].as_u64() == Some(expected_denied as u64)
            && attested["evidence_read_only"] == true
            && attested["workspace_writable"] == true;
        if !verified {
            return Err(HarnessError::forbidden(
                infra_system::workers::probe_failure(&probe.bytes, probe.code),
            ));
        }
        Ok(())
    }
    /// Resolves the shared SkillCache runtime.
    pub(crate) fn skillcache_runtime(&self) -> Arc<crate::skillcache::compile::Runtime> {
        lock(&self.skillcache)
            .ok()
            .and_then(|slot| slot.clone())
            .unwrap_or_else(|| {
                Arc::new(crate::skillcache::compile::Runtime {
                    store: self.store.clone(),
                    providers: self.providers.clone(),
                    engine: None,
                    cancel: self.stopping.clone(),
                    enqueuer: None,
                    requalifying: None,
                })
            })
    }
    /// Omnigraph cross-session context sharing is optional: a session queue without one runs unchanged.
    pub fn install_omnigraph(&self, value: Arc<crate::omnigraph::OmnigraphManager>) -> Result<()> {
        *lock(&self.omnigraph)? = Some(value);
        Ok(())
    }
    fn omnigraph(&self) -> Option<Arc<crate::omnigraph::OmnigraphManager>> {
        lock(&self.omnigraph).ok().and_then(|slot| slot.clone())
    }
    #[inline(never)]
    fn prepare_omnigraph_for_run<'a>(
        &'a self,
        session: &'a mut Value,
        job: &'a Job,
        workspace: &'a Path,
        cancel: &'a AtomicBool,
        emit: &'a Event,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            if session["omnigraph"]["enabled"] != true {
                return;
            }
            if let Some(raw_space) = session["omnigraph"]["shared_space"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                && let Ok(norm_space) = crate::omnigraph::normalize_shared_space_branch(raw_space)
            {
                let sess_branch = session["omnigraph"]["branch"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty() && *s != "auto")
                    .map(str::to_string)
                    .unwrap_or_else(|| crate::omnigraph::session_branch_name(&job.sid));
                if let Some(obj) = session["omnigraph"].as_object_mut() {
                    obj.insert("shared_space".into(), json!(norm_space));
                }
                if norm_space != sess_branch {
                    append_unique_str_list(
                        &mut session["omnigraph"]["pull_branches"],
                        &[norm_space.as_str()],
                        32,
                    );
                }
            }
            let Some(omnigraph) = self.omnigraph() else {
                return;
            };
            match omnigraph.prepare_session_context(session, workspace).await {
                Ok(prep) => {
                    let Some(updated_omni) =
                        apply_prepared_omnigraph_response(&session["omnigraph"], &job.sid, &prep)
                    else {
                        return;
                    };
                    let branch = updated_omni["branch"].as_str().unwrap_or("main").to_owned();
                    if let Ok(updated) = self.store.patch_guarded(
                        &job.sid,
                        &job.generation,
                        &json!({ "omnigraph": updated_omni }),
                        cancel,
                    ) {
                        *session = updated;
                    }
                    let atom_count = updated_omni["injected_atoms"]
                        .as_array()
                        .map(Vec::len)
                        .unwrap_or(0);
                    let _ = emit(json!({
                        "kind": "omnigraph",
                        "phase": "prepare",
                        "ok": true,
                        "branch": branch,
                        "atom_count": atom_count,
                        "text": format!("Omnigraph prepared branch `{branch}` ({atom_count} shared context atom(s))."),
                    }));
                }
                Err(err) => {
                    let mut omni = session["omnigraph"].clone();
                    if let Some(obj) = omni.as_object_mut() {
                        obj.insert("last_error".into(), json!(err.message));
                    }
                    if let Ok(updated) = self.store.patch_guarded(
                        &job.sid,
                        &job.generation,
                        &json!({ "omnigraph": omni }),
                        cancel,
                    ) {
                        *session = updated;
                    }
                    let _ = emit(json!({
                        "kind": "omnigraph",
                        "phase": "prepare",
                        "ok": false,
                        "warning": err.message,
                        "text": format!("Omnigraph context unavailable for this turn: {}", err.message),
                    }));
                }
            }
        })
    }
    /// A run starting or ending is when the project's graph is brought up to
    /// date; the build runs beside the session and never holds it.
    fn graph_moment(&self, sid: &str, trigger: crate::graph::Trigger) {
        let Some(graphs) = lock(&self.graphs).ok().and_then(|slot| slot.clone()) else {
            return;
        };
        if let Ok(session) = self.store.get(sid)
            && let Some(project) = session["project_id"].as_str()
        {
            let _ = graphs.refresh(project, trigger);
        }
    }
    fn live_graph_index(&self, project: &str) -> Option<Arc<crate::graph_map::Index>> {
        let graphs = lock(&self.graphs).ok().and_then(|slot| slot.clone())?;
        graphs.live_index(project)
    }
    fn emit_initial_focus(
        &self,
        sid: &str,
        session: &Value,
        workspace: &Path,
        prompt: &str,
        emit: &Event,
    ) -> Result<crate::graph_live::GraphFocus> {
        let project_id = session["project_id"].as_str().unwrap_or("");
        let live_index = self.live_graph_index(project_id);
        let initial_focus = crate::graph_live::predict_session_targets(
            &self.store,
            sid,
            session,
            workspace,
            live_index.as_deref(),
            prompt,
        );
        let _ = crate::graph_live::save_session_focus(&self.store, sid, &initial_focus);
        emit(initial_focus.to_event())?;
        Ok(initial_focus)
    }
    pub fn install_results(&self, value: Arc<crate::results::Results>) -> Result<()> {
        let mut slot = lock(&self.results)?;
        if slot.is_some() {
            return Err(HarnessError::conflict("Results were already installed."));
        }
        self.install_check_worker(value.clone())?;
        *slot = Some(value);
        Ok(())
    }
    /// After a run saved verified records or promoted chunks, carry them to
    /// the canonical Omnigraph branch in the background, as a manual save
    /// does: the turn's job does not wait for it, and a failure is the next
    /// sync's to repair.
    fn spawn_canonical_knowledge_sync(
        &self,
        sid: &str,
    ) -> Option<tokio::task::JoinHandle<Option<Value>>> {
        let session = self.store.get(sid).ok()?;
        if session["omnigraph"]["enabled"] != true {
            return None;
        }
        let project_id = session["project_id"].as_str()?.trim().to_string();
        if project_id.is_empty() {
            return None;
        }
        let omnigraph = self.results().ok()?.omnigraph()?;
        let knowledge = self.knowledge.clone();
        let mut body = json!({ "project_id": project_id, "session_id": sid });
        if let Some(bank) = session["brain"]["bank"]
            .as_str()
            .map(str::trim)
            .filter(|bank| !bank.is_empty())
        {
            body["bank"] = json!(bank);
        }
        Some(tokio::spawn(async move {
            let collected = crate::services::collect_sync_items(
                knowledge.as_ref(),
                &project_id,
                &json!({}),
                &body,
            )
            .await
            .ok()?;
            let items = collected["items"].as_array().cloned().unwrap_or_default();
            let prune_layers = collected["prune_layers"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect::<Vec<_>>();
            omnigraph
                .sync_canonical_knowledge_with_options(&project_id, &items, &prune_layers)
                .await
                .ok()
        }))
    }
    fn results(&self) -> Result<Arc<crate::results::Results>> {
        lock(&self.results)?
            .clone()
            .ok_or_else(|| HarnessError::unavailable("Native results are not initialized."))
    }
    fn commands(&self) -> Result<Arc<crate::commands::CommandCatalog>> {
        lock(&self.commands)?
            .clone()
            .ok_or_else(|| HarnessError::unavailable("Native commands are not initialized."))
    }
    pub fn launch_adapter(&self, session: &Value) -> Result<Arc<dyn provider::LaunchAdapter>> {
        let accelerators = lock(&self.accelerators)?.clone().ok_or_else(|| {
            HarnessError::unavailable("Native accelerator launch services are not initialized.")
        })?;
        let project = session["project_id"]
            .as_str()
            .ok_or_else(|| HarnessError::invalid("Session project missing."))?;
        self.store.project(project)?;
        Ok(Arc::new(crate::accelerators::ProjectLaunchAdapter {
            accelerators,
            project_id: project.into(),
            session_id: session["id"].as_str().map(String::from),
        }))
    }
    /// The adapter of a system run, which dispatches into several registered
    /// projects: each launch is adapted for the project its folder belongs to.
    pub fn system_launch_adapter(&self) -> Result<Arc<dyn provider::LaunchAdapter>> {
        let accelerators = lock(&self.accelerators)?.clone().ok_or_else(|| {
            HarnessError::unavailable("Native accelerator launch services are not initialized.")
        })?;
        Ok(Arc::new(crate::accelerators::SystemLaunchAdapter {
            accelerators,
        }))
    }
    pub fn shutdown_cancel(&self) -> Arc<AtomicBool> {
        self.stopping.clone()
    }
    pub async fn wait_closed(&self) -> Result<()> {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !self.queue_done.load(Ordering::Acquire)
            || !self.inbox_done.load(Ordering::Acquire)
            || !lock(&self.active)?.is_empty()
        {
            if std::time::Instant::now() >= until {
                return Err(HarnessError {
                    status: 503,
                    code: "cleanup-unconfirmed",
                    message:
                        "Native work has not confirmed cleanup; a new runner must retain exclusion."
                            .into(),
                });
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        Ok(())
    }
    pub fn operation_lock(&self) -> Arc<tokio::sync::Mutex<()>> {
        self.operation.clone()
    }
    pub fn idle(&self) -> Result<()> {
        let _admission = lock(&self.admission)?;
        if self.stopping.load(Ordering::Acquire)
            || !lock(&self.active)?.is_empty()
            || self.store.summaries()?.iter().any(active)
        {
            return Err(HarnessError::conflict(
                "Wait for all queued and running work to finish.",
            ));
        }
        Ok(())
    }
    pub fn install_check_worker(
        &self,
        worker: Arc<dyn crate::results::CheckWorkflow>,
    ) -> Result<()> {
        let mut slot = lock(&self.checks)?;
        if slot.is_some() {
            return Err(HarnessError::conflict(
                "The native check worker was already installed.",
            ));
        }
        *slot = Some(worker);
        Ok(())
    }
    pub fn enqueue_check(
        &self,
        sid: &str,
        check_id: &str,
        previous_status: &str,
    ) -> Result<String> {
        let _operation = self
            .operation
            .try_lock()
            .map_err(|_| HarnessError::conflict("A workspace operation is completing."))?;
        let _admission = lock(&self.admission)?;
        self.admit()?;
        if lock(&self.checks)?.is_none() {
            return Err(HarnessError::unavailable(
                "The native check worker is not installed.",
            ));
        }
        let current = self.store.get(sid)?;
        if active(&current) || current["status"] != previous_status {
            return Err(HarnessError::conflict(
                "The session changed before its check was queued.",
            ));
        }
        let old_generation = self.store.generation(sid)?;
        let generation = store::hexid();
        {
            let mut db = self.store.connection()?;
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if crate::store::get_session(&tx, sid)?["status"] != previous_status {
                return Err(HarnessError::conflict("The session status changed."));
            }
            tx.execute(
                "UPDATE sessions SET status='queued',updated_at=?1 WHERE id=?2",
                params![store::now(), sid],
            )?;
            tx.execute(
                "UPDATE harness_generations SET generation=?1 WHERE session_id=?2",
                params![generation, sid],
            )?;
            tx.commit()?;
        }
        let job = Job {
            sid: sid.into(),
            prompt: String::new(),
            generation: generation.clone(),
            kind: JobKind::Check {
                check_id: check_id.into(),
                previous_status: previous_status.into(),
            },
        };
        if self.queue.try_send(job).is_err() {
            let db = self.store.connection()?;
            db.execute(
                "UPDATE sessions SET status=?1 WHERE id=?2",
                params![previous_status, sid],
            )?;
            db.execute(
                "UPDATE harness_generations SET generation=?1 WHERE session_id=?2",
                params![old_generation, sid],
            )?;
            drop(db);
            self.store.emit_session_status(sid);
            self.emit_results_check(check_id);
            return Err(HarnessError::conflict("The run queue is full."));
        }
        self.store.emit_session_status(sid);
        self.emit_results_check(check_id);
        Ok(generation)
    }
    fn emit_results_check(&self, check_id: &str) {
        let Ok(Some(check)) = self.store.get_check(check_id) else {
            return;
        };
        let sid = check["session_id"].as_str().unwrap_or("").to_owned();
        let project_id = (!sid.is_empty())
            .then(|| self.store.session_project_id(&sid))
            .flatten();
        let status = check["status"].as_str().unwrap_or("").to_owned();
        self.store.publish_event(
            "results_check",
            (!sid.is_empty()).then_some(sid.clone()),
            project_id.clone(),
            json!({
                "id": check_id,
                "check_id": check_id,
                "session_id": sid,
                "project_id": project_id,
                "status": status,
                "check": check,
            }),
        );
    }
    pub fn history_session_ids(&self, session: &Value) -> Result<Vec<String>> {
        let sid = session["id"]
            .as_str()
            .ok_or_else(|| HarnessError::invalid("Missing session identity."))?;
        let Some(run) = session["creator"]["run_id"].as_str() else {
            return Ok(vec![sid.into()]);
        };
        let db = self.store.connection()?;
        let mut q = db.prepare(
            "SELECT id,creator FROM sessions WHERE creator IS NOT NULL ORDER BY created_at,id",
        )?;
        let mut ids = vec![];
        for row in q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (id, raw) = row?;
            let creator = crate::strict_json::parse(raw.as_bytes(), 65536)?;
            if creator["run_id"] == run {
                ids.push(id);
            }
        }
        Ok(ids)
    }
    pub fn latest_user(&self, sid: &str, first: bool) -> Result<String> {
        let db = self.store.connection()?;
        let mut q = db.prepare(if first {
            "SELECT data FROM events WHERE session_id=?1 ORDER BY id LIMIT 10000"
        } else {
            "SELECT data FROM events WHERE session_id=?1 ORDER BY id DESC LIMIT 10000"
        })?;
        for raw in q.query_map([sid], |r| r.get::<_, String>(0))? {
            let raw = raw?;
            let event = crate::strict_json::parse(raw.as_bytes(), 4 * 1024 * 1024)?;
            if event["kind"] == "user" {
                return store::text(&event["text"], "prompt", 32000);
            }
        }
        Err(HarnessError::invalid(
            "The session has no captured user instruction.",
        ))
    }
    pub fn merge(&self, data: &Value) -> Result<Value> {
        let _operation = self
            .operation
            .try_lock()
            .map_err(|_| HarnessError::conflict("A workspace operation is completing."))?;
        let _admission = lock(&self.admission)?;
        self.admit()?;
        let merger = crate::chat_merge::ChatMerge::new(self.store.clone());
        let mut created = false;
        let session = merger.merge(data, |destination, memory| {
            created = true;
            let mut options = self.create_options(destination)?;
            if !options["brain"].is_null() {
                options["brain"] = crate::brain_settings::validate(&destination["brain"], memory)?;
            }
            Ok(options)
        })?;
        if created {
            let sid = session["id"]
                .as_str()
                .ok_or_else(|| HarnessError::invalid("Merged session identity missing."))?;
            let job = Job {
                sid: sid.into(),
                prompt: self.latest_user(sid, true)?,
                generation: self.store.generation(sid)?,
                kind: JobKind::Run,
            };
            if self.queue.try_send(job).is_err() {
                self.store.patch(sid, &json!({"status":"interrupted"}))?;
                return Err(HarnessError::conflict(
                    "The merged task was saved but its queue is full. Restart it.",
                ));
            }
        }
        Ok(session)
    }
    pub fn restart_merge(&self, sid: &str) -> Result<Value> {
        let session = self.store.get(sid)?;
        if !["failed", "interrupted", "cancelled"]
            .contains(&session["status"].as_str().unwrap_or(""))
            || !session["native_session_id"].is_null()
        {
            return Err(HarnessError::conflict(
                "Only an interrupted merged task without a native session can restart.",
            ));
        }
        crate::chat_merge::ChatMerge::new(self.store.clone()).restore(sid)?;
        self.enqueue(sid, "", &json!({}))
    }
    /// Where a Manual session's launch puts the requests its person answers.
    fn approvals(
        &self,
        sid: &str,
        session: &Value,
        emit: &Event,
    ) -> Option<Arc<dyn infra_host::permission::Approvals>> {
        (crate::permissions::mode(session) == Some(infra_host::permission::PermissionMode::Manual))
            .then(|| self.permissions.approvals(sid, emit.clone()))
    }
    /// Enables Gate-1 read-only program auto-approval for `sid` inside registered `workspace`
    /// when `Accelerators` are installed (used for ordinary and Clash launches, keeping
    /// Creator and System manual permission workflows strictly explicit).
    fn register_read_only_auto_rules_for_session(&self, sid: &str, workspace: &Path) {
        if let Ok(slot) = lock(&self.accelerators)
            && let Some(accelerators) = slot.as_ref()
        {
            let _ = accelerators.register_read_only_auto_rules(&self.permissions, sid, workspace);
        }
    }
    /// Where a Creator launch puts permission requests when running in Manual mode.
    pub(crate) fn creator_approvals(
        &self,
        sid: &str,
        session: &Value,
        emit: &Event,
    ) -> Option<Arc<dyn infra_host::permission::Approvals>> {
        self.approvals(sid, session, emit)
    }
    /// Where a System Orchestration run puts requests that require human confirmation
    /// (Manual mode, or Auto mode when cross-service boundary escalation triggers).
    pub(crate) fn system_approvals(
        &self,
        sid: &str,
        session: &Value,
        emit: &Event,
    ) -> Option<Arc<dyn infra_host::permission::Approvals>> {
        matches!(
            crate::permissions::mode(session),
            Some(
                infra_host::permission::PermissionMode::Auto
                    | infra_host::permission::PermissionMode::Manual
            )
        )
        .then(|| self.permissions.approvals(sid, emit.clone()))
    }
    /// Expires any unanswered permission requests left waiting for `sid`.
    pub(crate) fn expire_permissions(&self, sid: &str) {
        self.permissions.expire(sid);
    }
    /// A person's answer to a request a running session waits on.
    pub fn decide_permission(&self, sid: &str, request: &str, body: &Value) -> Result<Value> {
        const MAX_PERMISSION_ANSWERS_BYTES: usize = 32 * 1024;
        if !store::exact(body, &["decision"], &["answers"]) {
            return Err(HarnessError::invalid(
                "Send only the decision and optional answers.",
            ));
        }
        let decision = body["decision"]
            .as_str()
            .and_then(infra_host::permission::Decision::parse)
            .ok_or_else(|| HarnessError::invalid("Choose allow, allow_session or deny."))?;
        let answers = match body.get("answers") {
            None | Some(Value::Null) => None,
            Some(Value::Object(_)) => {
                if body["answers"].to_string().len() > MAX_PERMISSION_ANSWERS_BYTES {
                    return Err(HarnessError::invalid(
                        "Question answers exceed the maximum payload size.",
                    ));
                }
                Some(body["answers"].clone())
            }
            Some(_) => {
                return Err(HarnessError::invalid("Question answers must be an object."));
            }
        };
        self.store.get(sid)?;
        self.permissions.decide_answer(
            sid,
            request,
            infra_host::permission::Answer { decision, answers },
        )
    }
    /// The requests a session's run waits on now.
    pub fn waiting_permissions(&self, sid: &str) -> Vec<Value> {
        self.permissions.waiting(sid)
    }
    fn configure_launch(&self, session: &Value, prompt: &str) -> Result<Launch> {
        let provider = session["provider"]
            .as_str()
            .ok_or_else(|| HarnessError::invalid("Missing provider."))?;
        let executable = self
            .providers
            .get(provider)?
            .executable
            .ok_or_else(|| HarnessError::unavailable("The provider CLI is unavailable."))?;
        let root = if let Some(primary) = session["system_explain"]["primary_root"].as_str() {
            PathBuf::from(primary)
        } else {
            PathBuf::from(
                session["project_path"]
                    .as_str()
                    .ok_or_else(|| HarnessError::invalid("Session workspace missing."))?,
            )
        };
        if !session["sdd"].is_null() {
            let ir_context = session["project_id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .map(|id| (self.store.state.as_path(), id));
            crate::sdd::require_documents_in(&root, &session["sdd"], ir_context)?;
        }
        Ok(Launch {
            provider: provider.into(),
            executable,
            project: root,
            prompt: prompt.into(),
            mode: session["mode"].as_str().unwrap_or("plan").into(),
            model: session["model"].as_str().map(String::from),
            session: session["native_session_id"].as_str().map(String::from),
            agents: session["agents_enabled"] == true,
            count: session["agent_count"].as_u64().unwrap_or(3) as u32,
            effort: session["thinking_effort"].as_str().map(String::from),
            usd: session["budgets"]["usd"].as_f64(),
            hook_events: true,
            permissions: crate::permissions::mode(session),
            approvals: None,
        })
    }
    #[cfg(test)]
    fn repository_lease(&self, job: &Job) -> Result<Lease> {
        Self::repository_lease_from(&self.store, job)
    }
    fn repository_lease_from(store: &SessionStore, job: &Job) -> Result<Lease> {
        let session = store.get(&job.sid)?;
        if !matches!(job.kind, JobKind::Run)
            || session["workflow"] != "native"
            || session["agents_enabled"] != false
            || [
                "brain",
                "sdd",
                "fleet",
                "clash",
                "creator",
                "system_run",
                "system_discovery",
                "system_explain",
            ]
            .iter()
            .any(|field| !session[*field].is_null())
        {
            return Ok(Lease::exclusive(&job.sid));
        }
        let recorded = Path::new(session["project_path"].as_str().unwrap_or(""));
        let workspace = infra_core::fs::canonicalize(recorded)?;
        if workspace != recorded || !workspace.is_dir() {
            return Err(HarnessError::conflict("The session workspace changed."));
        }
        let frame = infra_core::git::GitFrame::discover(&workspace)?.ok_or_else(|| {
            HarnessError::conflict("Parallel sessions require a verified Git repository.")
        })?;
        let top = infra_core::fs::canonicalize(&frame.top)?;
        let common = frame.common_dir()?;
        let saved = session["git_common_dir"]
            .as_str()
            .ok_or_else(|| HarnessError::conflict("The session Git identity is unavailable."))?;
        if top.parent().is_none() || infra_core::fs::canonicalize(saved)? != common {
            return Err(HarnessError::conflict("The session Git identity changed."));
        }
        let images = [&workspace, &top, &common, &frame.admin]
            .into_iter()
            .map(|path| {
                infra_core::fs::native_image(path)?
                    .map(|image| image.without_contents())
                    .ok_or_else(|| HarnessError::conflict("The session Git identity disappeared."))
            })
            .collect::<Result<Vec<_>>>()?;
        let mounts = crate::session_mounts::scopes(&[&workspace, &top, &common, &frame.admin])?;
        Ok(Lease::repository(&job.sid, &workspace, &top, &common)
            .with_images(images)
            .with_mounts(mounts))
    }
    async fn job_lease(&self, job: Job) -> Lease {
        let fallback = Lease::exclusive(&job.sid);
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || Self::repository_lease_from(&store, &job))
            .await
            .ok()
            .and_then(std::result::Result::ok)
            .unwrap_or(fallback)
    }
    fn remove_active(&self, sid: &str, cancel: &Arc<AtomicBool>) {
        if let Ok(mut active) = lock(&self.active)
            && active
                .get(sid)
                .is_some_and(|held| Arc::ptr_eq(held, cancel))
        {
            active.remove(sid);
        }
    }
    fn stop_failed(&self, current: &Arc<AtomicBool>) {
        self.stopping.store(true, Ordering::Release);
        let _admission = lock(&self.admission).ok();
        if let Ok(active) = lock(&self.active) {
            for token in active.values() {
                if !Arc::ptr_eq(token, current) {
                    token.store(true, Ordering::Release);
                }
            }
        }
        self.queue_wake.notify_one();
        self.inbox_wake.notify_one();
    }
    async fn execute_job(self: &Arc<Self>, job: Job, expected: Option<Lease>) {
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let Ok(_admission) = lock(&self.admission) else {
                self.stop_failed(&cancel);
                return;
            };
            if self.stopping.load(Ordering::Acquire) {
                return;
            }
            if let Ok(session) = self.store.get(&job.sid)
                && self.store.generation(&job.sid).ok().as_ref() == Some(&job.generation)
                && !active(&session)
                && session["status"] != "cancelled"
            {
                return;
            }
            let Ok(mut active) = lock(&self.active) else {
                self.stopping.store(true, Ordering::Release);
                self.queue_wake.notify_one();
                return;
            };
            if active.contains_key(&job.sid) {
                drop(active);
                drop(_admission);
                self.stop_failed(&cancel);
                return;
            }
            active.insert(job.sid.clone(), cancel.clone());
        }
        self.inbox_wake.notify_one();
        if let JobKind::Check { check_id, .. } = &job.kind {
            let result = async {
                let worker = lock(&self.checks)?.clone().ok_or_else(|| {
                    HarnessError::unavailable("Native check worker is unavailable.")
                })?;
                let mut session = self.store.get(&job.sid)?;
                if self.store.generation(&job.sid)? != job.generation
                    || session["status"] == "cancelled"
                {
                    cancel.store(true, Ordering::Release);
                } else {
                    match self.store.patch_guarded(
                        &job.sid,
                        &job.generation,
                        &json!({"status":"running"}),
                        &cancel,
                    ) {
                        Ok(updated) => session = updated,
                        Err(error) => {
                            session = self.store.get(&job.sid)?;
                            if cancel.load(Ordering::Acquire)
                                || session["status"] == "cancelled"
                                || self.store.generation(&job.sid)? != job.generation
                            {
                                cancel.store(true, Ordering::Release);
                            } else {
                                return Err(error);
                            }
                        }
                    }
                }
                // The concrete CheckWorkflow settles an already-cancelled or
                // obsolete queued receipt before performing any workspace I/O.
                self.emit_results_check(check_id);
                worker
                    .run_check(
                        session,
                        check_id.clone(),
                        job.generation.clone(),
                        cancel.clone(),
                    )
                    .await
            }
            .await
            .map(|()| Value::Null);
            if result
                .as_ref()
                .err()
                .is_some_and(|error| error.code == "cleanup-unconfirmed")
            {
                self.stop_failed(&cancel);
            }
            if finalize_job(&self.store, &job, &result, &cancel).is_err() {
                self.stop_failed(&cancel);
            }
            self.emit_results_check(check_id);
            if self
                .store
                .inbox_settle(&job.sid, &job.generation, false, None)
                .is_err()
            {
                self.stop_failed(&cancel);
            }
            self.remove_active(&job.sid, &cancel);
            self.inbox_wake.notify_one();
            return;
        }
        let mut native_proof = None;
        let result = if let Some(expected) = expected.filter(Lease::parallel)
            && self.job_lease(job.clone()).await != expected
        {
            Err(HarnessError::conflict(
                "The queued repository identity changed; retry this task.",
            ))
        } else {
            // Keep the large native workflow future out of the scheduler's
            // task frame, including unoptimized builds with a 2 MiB worker stack.
            Box::pin(self.run_job(&job, cancel.clone(), &mut native_proof)).await
        };
        // Physical cleanup authority is global and survives replacement of a
        // session generation. It is never filtered by the terminal row CAS.
        if result
            .as_ref()
            .err()
            .is_some_and(|error| error.code == "cleanup-unconfirmed")
        {
            self.stop_failed(&cancel);
        }
        if let Ok(ref val) = result
            && !cancel.load(Ordering::Acquire)
            && val["kind"] != "awaiting_context"
            && !matches!(
                val["status"].as_str(),
                Some("awaiting_approval" | "rejected")
            )
            && val["outcome"] != "incomplete"
            && val["ok"] != false
            && let Ok(results) = self.results()
        {
            let _ = spawn_omnigraph_publish(results, &job.sid, &job.generation, val, &cancel).await;
            if saved_canonical_knowledge(&val["memory"]) {
                let _ = self.spawn_canonical_knowledge_sync(&job.sid);
            }
        }
        match finalize_job(&self.store, &job, &result, &cancel) {
            Ok(status) => {
                if let Ok(results) = self.results() {
                    let _ = results.finish_launch(&job.generation, &status);
                }
                if self
                    .store
                    .inbox_settle(
                        &job.sid,
                        &job.generation,
                        status == "completed",
                        native_proof.as_deref(),
                    )
                    .is_err()
                {
                    self.stop_failed(&cancel);
                }
            }
            Err(_) => {
                self.stop_failed(&cancel);
            }
        }
        let _ = crate::graph_live::complete_stored_session_focus(&self.store, &job.sid);
        self.graph_moment(&job.sid, crate::graph::Trigger::Finish);
        self.remove_active(&job.sid, &cancel);
        self.inbox_wake.notify_one();
    }
    async fn run_job(
        &self,
        job: &Job,
        cancel: Arc<AtomicBool>,
        native_proof: &mut Option<String>,
    ) -> Result<Value> {
        let mut session = self.store.get(&job.sid)?;
        if session["status"] == "cancelled" || self.store.generation(&job.sid)? != job.generation {
            return Err(HarnessError::conflict(
                "The queued run is no longer current.",
            ));
        }
        let inbox_owned = self.store.inbox_owned(&job.sid, &job.generation)?;
        let expected_native = if inbox_owned {
            self.store.inbox_native_guard(&job.sid, &job.generation)?
        } else {
            None
        };
        if inbox_owned {
            let lease = self.job_lease(job.clone()).await;
            self.store.inbox_verify_execution(
                &job.sid,
                &job.generation,
                &inbox_driver::stamp(&lease),
            )?;
            if expected_native.is_none()
                || expected_native.as_deref() != session["native_session_id"].as_str()
            {
                return Err(HarnessError::conflict(
                    "The assigned inbox native identity changed.",
                ));
            }
        }
        let requested_workspace = Path::new(session["project_path"].as_str().unwrap_or(""));
        std::fs::metadata(requested_workspace)?;
        let workspace = infra_core::fs::python_realpath(requested_workspace)?;
        if workspace.as_path() != Path::new(session["project_path"].as_str().unwrap_or("")) {
            return Err(HarnessError::forbidden("The session workspace changed."));
        }
        let results = self.results()?;
        Box::pin(results.baseline(&session)).await?;
        self.store.patch_guarded(
            &job.sid,
            &job.generation,
            &json!({"status":"running"}),
            &cancel,
        )?;
        self.inbox_wake.notify_one();
        results.start_launch(&job.sid, &job.generation, None)?;
        let store = self.store.clone();
        let sid = job.sid.clone();
        let generation = job.generation.clone();
        let emit: Event = Arc::new(move |event| {
            if !store.guarded_event(&sid, &generation, &event)? {
                return Err(HarnessError::conflict("The session generation changed."));
            }
            Ok(())
        });
        // Protected paths are kept from a run by wrapping an ordinary
        // session's launches. The specialized workflows start their own, so
        // with protected paths installed they are refused rather than run
        // outside the boundary.
        if lock(&self.protected)?.is_some()
            && [
                "creator",
                "system_run",
                "system_discovery",
                "fleet",
                "clash",
            ]
            .iter()
            .any(|name| !session[*name].is_null())
        {
            return Err(HarnessError::forbidden(
                "Protected paths are installed: only an ordinary session runs here.",
            ));
        }
        for name in ["creator", "system_run", "system_discovery"] {
            if !session[name].is_null() {
                let result = Box::pin(
                    self.special
                        .get(name)
                        .ok_or_else(|| {
                            HarnessError::unavailable("Native workflow is unavailable.")
                        })?
                        .run(
                            session,
                            job.prompt.clone(),
                            job.generation.clone(),
                            cancel,
                            emit,
                        ),
                )
                .await;
                self.permissions.expire(&job.sid);
                return result;
            }
        }
        let mut graph_binding = None;
        let mut omnigraph_prepared = false;
        let omnigraph_already_reviewed = session["brain"]["approved"] == true
            && (!session["omnigraph"]["capsule_text"]
                .as_str()
                .unwrap_or("")
                .trim()
                .is_empty()
                || session["omnigraph"]["query"].is_string());
        if !session["fleet"].is_null() || !session["clash"].is_null() {
            let saved: Option<String> = self.store.connection()?.query_row(
                "SELECT fleet_action FROM sessions WHERE id=?1",
                [&job.sid],
                |r| r.get(0),
            )?;
            if !saved
                .as_deref()
                .is_some_and(|value| ["approve", "reject"].contains(&value))
            {
                if session["omnigraph"]["enabled"] == true && !omnigraph_already_reviewed {
                    Box::pin(self.prepare_omnigraph_for_run(
                        &mut session,
                        job,
                        &workspace,
                        &cancel,
                        &emit,
                    ))
                    .await;
                    omnigraph_prepared = true;
                }
                let binding = self.lifecycle.begin_with_options(
                    &session,
                    &job.prompt,
                    &job.generation,
                    cancel.clone(),
                    crate::lifecycle::LifecycleOptions {
                        native_command: false,
                        supported: false,
                    },
                )?;
                if !binding.ready {
                    self.lifecycle.abort(&binding, "awaiting_context")?;
                    return Ok(json!({"kind":"awaiting_context"}));
                }
                session = binding.session.clone();
                graph_binding = Some(binding);
            }
        }
        if !session["fleet"].is_null() {
            let settings = &session["fleet"];
            let provider = session["provider"].as_str().unwrap_or("codex");
            let config = fleet::Config {
                project: workspace,
                provider: provider.into(),
                executable: self
                    .providers
                    .get(provider)?
                    .executable
                    .unwrap_or_else(|| PathBuf::from(provider)),
                lenses: settings["lenses"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(String::from)
                    .collect(),
                model: session["model"].as_str().map(String::from),
                effort: session["thinking_effort"].as_str().map(String::from),
                budget: settings["budget_usd"].as_f64(),
                seconds: settings["worker_timeout"].as_u64().unwrap_or(300),
                concurrency: session["agent_count"].as_u64().unwrap_or(3) as usize,
                dry_run: settings["dry_run"] == true,
                reports: self
                    .store
                    .state
                    .join("fleet")
                    .join(&job.sid)
                    .join("reports"),
                adapter: Some(self.launch_adapter(&session)?),
                runner_lock: Some(self.store.runner_lock()),
            };
            let fleet = fleet::Fleet {
                store: self.store.clone(),
                blackboard: self.blackboards.for_session(&session)?,
            };
            let action: Option<String> = self.store.connection()?.query_row(
                "SELECT fleet_action FROM sessions WHERE id=?1",
                [&job.sid],
                |r| r.get(0),
            )?;
            fleet.initialize_with_precheck(
                &job.sid,
                &job.prompt,
                &format!("harness/{}", job.sid),
                &config,
                Some(
                    &self
                        .store
                        .state
                        .join("fleet")
                        .join(&job.sid)
                        .join("checkpoints.sqlite"),
                ),
                action.as_deref(),
                settings.get("precheck").filter(|v| !v.is_null()),
            )?;
            if let Some(action) = action
                .as_deref()
                .filter(|s| ["approve", "reject"].contains(s))
            {
                fleet.decide(&job.sid, action == "approve")?;
            } else if !omnigraph_prepared {
                Box::pin(self.prepare_omnigraph_for_run(
                    &mut session,
                    job,
                    &config.project,
                    &cancel,
                    &emit,
                ))
                .await;
            }
            let _ =
                self.emit_initial_focus(&job.sid, &session, &config.project, &job.prompt, &emit);
            let result = Box::pin(fleet.drive(&job.sid, &config, cancel, emit)).await;
            if let Some(binding) = &graph_binding {
                let _ = self.lifecycle.abort(
                    binding,
                    if result.is_ok() {
                        "fleet-finished"
                    } else {
                        "failed"
                    },
                );
            }
            return result;
        }
        if !session["clash"].is_null() {
            if !omnigraph_prepared {
                Box::pin(self.prepare_omnigraph_for_run(
                    &mut session,
                    job,
                    &workspace,
                    &cancel,
                    &emit,
                ))
                .await;
            }
            let mut executables = BTreeMap::new();
            for id in ["claude", "codex", "cursor"] {
                if let Some(path) = self.providers.get(id)?.executable {
                    executables.insert(id.into(), path);
                }
            }
            let mut launch = self.configure_launch(&session, &job.prompt)?;
            launch.approvals = self.approvals(&job.sid, &session, &emit);
            if launch.approvals.is_some() {
                self.register_read_only_auto_rules_for_session(&job.sid, &workspace);
            }
            let stage = if session["mode"] == "edit" {
                "implement"
            } else {
                "review"
            };
            let brain_ctx = session["brain"]["capsule_text"].as_str().unwrap_or("");
            let omni_ctx = session["omnigraph"]["capsule_text"]
                .as_str()
                .filter(|_| session["omnigraph"]["enabled"] == true)
                .unwrap_or("");
            let clash_context = match (brain_ctx.trim().is_empty(), omni_ctx.trim().is_empty()) {
                (true, true) => String::new(),
                (false, true) => brain_ctx.to_owned(),
                (true, false) => format!("Shared Omnigraph Context:\n{omni_ctx}"),
                (false, false) => format!("{brain_ctx}\n\nShared Omnigraph Context:\n{omni_ctx}"),
            };
            let mut cycle = clash::Cycle {
                store: self.store.clone(),
                session: session.clone(),
                launch,
                executables,
                context: clash_context,
                adapter: self.launch_adapter(&session)?,
            };
            let _ = self.emit_initial_focus(&job.sid, &session, &workspace, &job.prompt, &emit);
            let result = Box::pin(cycle.drive(
                &job.prompt,
                stage,
                !session["clash_result"].is_null(),
                cancel,
                emit,
            ))
            .await;
            self.permissions.expire(&job.sid);
            if let Some(binding) = &graph_binding {
                let _ = self.lifecycle.abort(
                    binding,
                    if result.is_ok() {
                        "clash-finished"
                    } else {
                        "failed"
                    },
                );
            }
            return result;
        }
        let mut route = if session["workflow"] == "native" {
            let commands = self.commands()?;
            let mut route = Box::pin(commands.route(&session, &job.prompt)).await?;
            Box::pin(commands.apply_skill_hint(&session, &mut route)).await?;
            route
        } else {
            crate::commands::ResolvedCommand {
                mode: "text".into(),
                text: job.prompt.clone(),
                requests: vec![],
                notice: None,
            }
        };
        if let Some(notice) = &route.notice {
            emit(json!({"kind":"status","text":notice}))?;
        }
        if session["omnigraph"]["enabled"] == true
            && !omnigraph_prepared
            && !omnigraph_already_reviewed
        {
            Box::pin(self.prepare_omnigraph_for_run(&mut session, job, &workspace, &cancel, &emit))
                .await;
        }
        let binding = self.lifecycle.begin_with_options(
            &session,
            &route.text,
            &job.generation,
            cancel.clone(),
            crate::lifecycle::LifecycleOptions {
                native_command: route.mode == "native",
                supported: true,
            },
        )?;
        if !binding.ready {
            self.lifecycle.abort(&binding, "awaiting_context")?;
            return Ok(json!({"kind":"awaiting_context"}));
        }
        session = binding.session.clone();
        if session["workflow"] == "native" {
            let commands = self.commands()?;
            Box::pin(commands.apply_skill_hint(&session, &mut route)).await?;
        }
        // A turn that may run a SkillCache capsule decides it only once the
        // turn is ready to dispatch, so `awaiting_context` never leaves stage files.
        let mut decision = crate::skillcache::warm::decide(
            &self.skillcache_runtime(),
            &session,
            &route,
            &job.prompt,
            &workspace,
            &job.generation,
        );
        if matches!(
            &decision,
            crate::skillcache::warm::Decision::Miss {
                reason: "no_capsule",
                ..
            }
        ) && crate::skillcache::settings::eval_arm(&session).is_none()
            && crate::skillcache::compile::try_admit_template_on_miss(
                &self.skillcache_runtime(),
                &session,
                &job.generation,
                &route,
                &workspace,
            )
            .await
            .ok()
            .flatten()
            .is_some()
        {
            decision = crate::skillcache::warm::decide(
                &self.skillcache_runtime(),
                &session,
                &route,
                &job.prompt,
                &workspace,
                &job.generation,
            );
        }
        let protected_cfg = lock(&self.protected)?.clone();
        let protected_paths = protected_cfg
            .as_ref()
            .map(|cfg| cfg.protected.clone())
            .unwrap_or_default();
        let mut direct_answer: Option<(serde_json::Value, String)> = None;
        let mut partial_fallback: Option<crate::skillcache::warm::PartialDirectFailure> = None;
        let warm = match decision {
            crate::skillcache::warm::Decision::Hit(warm) if warm.direct_dispatch => {
                if let Some(ref cfg) = protected_cfg
                    && let Err(error) =
                        Box::pin(self.verify_direct_protected(cfg, &workspace, &warm, &cancel))
                            .await
                {
                    warm.cleanup_abandoned();
                    let _ = self.lifecycle.abort(&binding, "failed");
                    return Err(error);
                }
                let warm_run =
                    warm.with_engine(protected_cfg.as_ref().and_then(|cfg| cfg.engine.clone()));
                let ws_run = workspace.clone();
                let gen_run = job.generation.clone();
                let cancel_run = cancel.clone();
                let direct = tokio::task::spawn_blocking(move || {
                    crate::skillcache::warm::try_direct_execute_with(
                        &warm_run,
                        &ws_run,
                        &gen_run,
                        None,
                        &protected_paths,
                        Some(&cancel_run),
                    )
                })
                .await
                .unwrap_or_else(|_| {
                    crate::skillcache::warm::DirectDispatchOutcome::Fallback {
                        reason: "join_error".into(),
                        program_failed: false,
                        partial: None,
                    }
                });
                match direct {
                    crate::skillcache::warm::DirectDispatchOutcome::Completed {
                        answer,
                        formatted,
                    } => {
                        let hit_decision = crate::skillcache::warm::Decision::Hit(warm.clone());
                        if let Some(mut event) = hit_decision.event() {
                            event["direct_dispatch"] = json!(true);
                            emit(event)?;
                        }
                        if let Some(mut record) = hit_decision.record() {
                            record["direct_dispatch"] = json!(true);
                            let _ = results.note_skillcache(&job.generation, &record);
                        }
                        direct_answer = Some((answer, formatted));
                        Some(warm)
                    }
                    crate::skillcache::warm::DirectDispatchOutcome::Fallback {
                        reason,
                        program_failed,
                        partial,
                    } => {
                        if let Some(partial_fail) = partial {
                            let hit_decision = crate::skillcache::warm::Decision::Hit(warm.clone());
                            if let Some(mut event) = hit_decision.event() {
                                event["partial_direct_fallback"] = json!(true);
                                event["failed_step_index"] = json!(partial_fail.step_index);
                                emit(event)?;
                            }
                            if let Some(mut record) = hit_decision.record() {
                                record["partial_direct_fallback"] = json!(true);
                                record["failed_step_index"] = json!(partial_fail.step_index);
                                let _ = results.note_skillcache(&job.generation, &record);
                            }
                            partial_fallback = Some(partial_fail);
                            Some(warm)
                        } else if reason == "needs_bindings" && !program_failed {
                            let hit_decision = crate::skillcache::warm::Decision::Hit(warm.clone());
                            if let Some(event) = hit_decision.event() {
                                emit(event)?;
                            }
                            if let Some(record) = hit_decision.record() {
                                let _ = results.note_skillcache(&job.generation, &record);
                            }
                            Some(warm)
                        } else {
                            if program_failed {
                                warm.finish(&self.store, &session, &job.generation, &|event| {
                                    emit(event)
                                });
                                crate::skillcache::maybe_enqueue_recompile_on_finish(
                                    &self.skillcache_runtime(),
                                    &warm.capsule_id,
                                    &|event| {
                                        let _ = emit(event);
                                    },
                                );
                            } else {
                                warm.cleanup_abandoned();
                            }
                            let fallback_decision = crate::skillcache::warm::Decision::Miss {
                                reason: "direct_fallback",
                                text: format!("direct execution fell back to cold run ({reason})"),
                            };
                            if let Some(event) = fallback_decision.event() {
                                emit(event)?;
                            }
                            if let Some(record) = fallback_decision.record() {
                                let _ = results.note_skillcache(&job.generation, &record);
                            }
                            None
                        }
                    }
                }
            }
            other => {
                if let Some(event) = other.event() {
                    emit(event)?;
                }
                if let Some(record) = other.record() {
                    let _ = results.note_skillcache(&job.generation, &record);
                }
                match other {
                    crate::skillcache::warm::Decision::Hit(warm) => Some(warm),
                    _ => None,
                }
            }
        };
        let thin_formatter_results = if let Some((answer, formatted)) = direct_answer {
            let warm_ref = warm.as_ref().expect("direct dispatch requires warm");
            let thin_pref = crate::skillcache::settings::thin_formatter(&session);
            if should_run_thin_formatter(thin_pref, &answer) {
                Some(direct_formatter_payload(&answer))
            } else {
                let started = std::time::Instant::now();
                let initial_focus =
                    self.emit_initial_focus(&job.sid, &session, &workspace, &route.text, &emit)?;
                let live_focus = Arc::new(Mutex::new(crate::graph_live::LiveFocusTracker::new(
                    workspace.clone(),
                )));
                if let Ok(mut tracker) = live_focus.lock() {
                    tracker.seed_focus(initial_focus);
                }
                let graphs = lock(&self.graphs).ok().and_then(|slot| slot.clone());
                let direct_ledger = json!({
                    "total": 0,
                    "direct_dispatch": true,
                    "message": 0,
                    "instructions": 0,
                    "skillcache": 0,
                    "prefix_cache_hit": true,
                });
                let direct_setup = (|| -> Result<Launch> {
                    let mut launch = self.configure_launch(&session, &route.text)?;
                    launch.prompt = formatted.clone();
                    self.lifecycle
                        .set_prompt_ledger(&binding, &direct_ledger, 1)?;
                    if let Some(model) = launch.model.as_deref() {
                        self.lifecycle
                            .set_window(&binding, results.last_window(&job.sid, model)?)?;
                    }
                    self.lifecycle.before_dispatch(&binding, &launch)?;
                    Ok(launch)
                })();
                if let Err(err) = direct_setup {
                    warm_ref.cleanup_abandoned();
                    let _ = self.lifecycle.abort(&binding, "failed");
                    return Err(err);
                }
                let usage_ev = json!({
                    "kind": "usage",
                    "input_tokens": 0,
                    "output_tokens": 0,
                    "cache_read_input_tokens": 0,
                    "cache_creation_input_tokens": 0,
                    "cost_usd": 0.0,
                    "model_calls": 0,
                    "direct_dispatch": true,
                });
                provider::RunEvents::observe(&binding, None, Some(&usage_ev))?;
                emit(usage_ev)?;
                emit(json!({"kind": "text", "text": formatted.clone()}))?;
                let result_ev = json!({
                    "kind": "result",
                    "ok": true,
                    "text": formatted.clone(),
                    "cost_usd": 0.0,
                    "direct_dispatch": true,
                });
                provider::RunEvents::observe(&binding, None, Some(&result_ev))?;
                emit(result_ev)?;
                ingest_warm_with_live_focus(
                    warm_ref,
                    &self.store,
                    &job.sid,
                    session["project_id"].as_str().unwrap_or(""),
                    &workspace,
                    graphs.as_deref(),
                    &live_focus,
                    &|event| emit(event),
                );
                warm_ref.finish(&self.store, &session, &job.generation, &|event| emit(event));
                crate::skillcache::maybe_enqueue_recompile_on_finish(
                    &self.skillcache_runtime(),
                    &warm_ref.capsule_id,
                    &|event| {
                        let _ = emit(event);
                    },
                );
                let mut recovery = NativeRecovery {
                    adapter: self.launch_adapter(&session)?,
                    human_cancel: cancel.clone(),
                };
                let worker = infra_host::provider::WorkerResult {
                    text: formatted,
                    ok: true,
                    cost_usd: Some(0.0),
                    session_id: session["native_session_id"].as_str().map(str::to_owned),
                    duration_seconds: started.elapsed().as_secs_f64(),
                    error: None,
                    limit_reached: None,
                };
                let memory =
                    Box::pin(
                        self.lifecycle
                            .finish(&binding, &worker, &emit, &mut recovery),
                    )
                    .await?;
                if let Ok(mut tracker) = live_focus.lock()
                    && tracker.complete()
                {
                    let _ = crate::graph_live::save_session_focus(
                        &self.store,
                        &job.sid,
                        tracker.focus(),
                    );
                    if let Some(delta) = tracker.take_delta_event() {
                        let _ = emit(delta);
                    }
                }
                *native_proof = worker.session_id.clone();
                return Ok(json!({
                    "kind": "result",
                    "ok": true,
                    "text": worker.text,
                    "observed_cost_usd": worker.cost_usd,
                    "provider_billing": "direct_dispatch",
                    "memory": memory["memory"],
                    "full_cost": memory["full_cost"],
                }));
            }
        } else {
            None
        };
        self.graph_moment(&job.sid, crate::graph::Trigger::Launch);
        let initial_focus =
            self.emit_initial_focus(&job.sid, &session, &workspace, &route.text, &emit)?;
        let live_focus = Arc::new(Mutex::new(crate::graph_live::LiveFocusTracker::new(
            workspace.clone(),
        )));
        if let Ok(mut tracker) = live_focus.lock() {
            tracker.seed_focus(initial_focus);
        }
        let configured = (|| -> Result<Launch> {
            let mut launch = self.configure_launch(&session, &route.text)?;
            let prepared = if let Some(results_json) = &thin_formatter_results {
                let warm_name = warm.as_ref().map_or("capsule", |w| w.name.as_str());
                crate::prompt::build_thin_formatter_prompt(&job.prompt, warm_name, results_json)?
            } else {
                let partial_block = partial_fallback.as_ref().map(|pf| {
                    let prefix_json = serde_json::to_string_pretty(&pf.prefix_results)
                        .unwrap_or_else(|_| "{}".into());
                    let diag_json = serde_json::to_string_pretty(&pf.diagnostics)
                        .unwrap_or_else(|_| "{}".into());
                    let req = if pf.required_outputs.is_empty() {
                        "then run `skillcache-run continue`".to_owned()
                    } else {
                        format!(
                            "then run `skillcache-run continue '{{...}}'` supplying: {}",
                            pf.required_outputs.join(", ")
                        )
                    };
                    format!(
                        "{}\n\nPartial direct execution completed steps 0..{} and failed at step {} (`{}`):\n- Instruction: {}\n- Failure diagnostics: {}\n- Verified prefix outputs (0..{}): {}\nContinue from the saved paused run state ({}).",
                        warm.as_ref().map_or("", |w| w.block.as_str()),
                        pf.step_index.saturating_sub(1),
                        pf.step_index,
                        pf.program,
                        pf.instruction,
                        diag_json,
                        pf.step_index.saturating_sub(1),
                        prefix_json,
                        req,
                    )
                });
                let skillcache_block = partial_block
                    .as_deref()
                    .or_else(|| warm.as_ref().map(|w| w.block.as_str()));
                crate::prompt::build_with_skillcache(
                    &self.store,
                    &session,
                    &job.prompt,
                    &route,
                    &binding.instructions,
                    skillcache_block,
                )?
            };
            launch.prompt = if session["system_explain"].is_object() {
                format!(
                    "{}{}",
                    crate::system_graph::system_explain_prompt_prefix(
                        &self.store,
                        &session["system_explain"],
                    ),
                    prepared.text
                )
            } else {
                prepared.text
            };
            self.lifecycle.set_prompt_ledger(
                &binding,
                &prepared.ledger,
                if launch.agents { launch.count + 1 } else { 1 },
            )?;
            if let Some(model) = launch.model.as_deref() {
                self.lifecycle
                    .set_window(&binding, results.last_window(&job.sid, model)?)?;
            }
            self.lifecycle.before_dispatch(&binding, &launch)?;
            Ok(launch)
        })();
        let mut launch = match configured {
            Ok(launch) => launch,
            Err(error) => {
                if let Some(warm) = &warm {
                    warm.cleanup_abandoned();
                }
                let _ = self.lifecycle.abort(&binding, "failed");
                return Err(error);
            }
        };
        launch.approvals = self.approvals(&job.sid, &session, &emit);
        if launch.approvals.is_some() {
            self.register_read_only_auto_rules_for_session(&job.sid, &workspace);
        }
        let adapter = self.launch_adapter(&session)?;
        // The turn's own launch gets the executor when it runs a capsule; a
        // recovery turn after it does not.
        let mut dispatch_adapter: Arc<dyn provider::LaunchAdapter> = match &warm {
            Some(warm) => Arc::new(crate::skillcache::warm::WarmLaunchAdapter {
                inner: adapter.clone(),
                warm: warm.clone(),
            }),
            None => adapter.clone(),
        };
        let mut recovery_adapter = adapter.clone();
        let protected_cfg = lock(&self.protected)?.clone();
        if let Some(protected_cfg) = protected_cfg {
            dispatch_adapter = match Box::pin(self.attach_protected(
                dispatch_adapter,
                protected_cfg,
                &launch.executable,
                &workspace,
                warm.as_deref(),
                &cancel,
            ))
            .await
            {
                Ok(adapted) => {
                    // The recovery launch of the turn runs inside the same
                    // boundary. It gets no capsule, so nothing of the warm
                    // run is opened to it.
                    recovery_adapter = Arc::new(ProtectedLaunchAdapter {
                        inner: adapter.clone(),
                        protected: adapted.protected.clone(),
                        workspace: adapted.workspace.clone(),
                        codex: adapted.codex.clone(),
                        state_dir: None,
                        evidence: adapted.evidence.clone(),
                    });
                    adapted
                }
                Err(error) => {
                    if let Some(state_dir) = warm.as_ref().and_then(|w| w.state_dir.as_deref()) {
                        let _ = std::fs::remove_dir_all(state_dir);
                    }
                    let _ = self.lifecycle.abort(&binding, "failed");
                    return Err(error);
                }
            };
        }
        let warm_for_stream = warm.clone();
        let previous = launch
            .session
            .as_deref()
            .and_then(|id| session["cost_totals"][id].as_f64());
        let max_tokens = session["budgets"]["tokens"].as_u64();
        let mut tokens = 0u64;
        let budget_stop = Arc::new(AtomicBool::new(false));
        let budget_control = budget_stop.clone();
        let dispatch_cancel = Arc::new(AtomicBool::new(cancel.load(Ordering::Acquire)));
        let dispatch_control = dispatch_cancel.clone();
        let human = cancel.clone();
        let forwarding = CancelRelay(tokio::spawn(async move {
            loop {
                if human.load(Ordering::Acquire) {
                    dispatch_control.store(true, Ordering::Release);
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        }));
        let control = cancel.clone();
        let process_control = dispatch_cancel.clone();
        let store = self.store.clone();
        let sid = job.sid.clone();
        let generation = job.generation.clone();
        let emit_events = emit.clone();
        let focus_for_stream = live_focus.clone();
        // A session that opted in keeps the trace of this cold turn (evaluation
        // ablation runs never record traces). The recorder sees the same raw events
        // as the binding and nothing else.
        let recorder = (crate::skillcache::settings::records(&session)
            && crate::skillcache::settings::eval_arm(&session).is_none()
            && warm.is_none())
        .then(|| crate::skillcache::trace::TraceRecorder::new(&launch.provider));
        let run_events: Arc<dyn provider::RunEvents> = match &recorder {
            Some(recorder) => Arc::new(crate::skillcache::trace::Tee {
                primary: binding.clone(),
                recorder: recorder.clone(),
            }),
            None => Arc::new(binding.clone()),
        };
        let allowlist = crate::skillcache::settings::workspace_allowlist(&self.store);
        let obs_strategy = if recorder.is_some() {
            crate::skillcache::trace::observation_strategy(
                &self.store,
                &session,
                &route,
                &workspace,
            )
        } else {
            crate::skillcache::trace::ObservationStrategy::None
        };
        let pre_turn_obs = match obs_strategy {
            crate::skillcache::trace::ObservationStrategy::Full => {
                // A capsule compiled from this turn is replayed on the workspace
                // as it is now, before the turn changes it.
                let (state, root, generation, allowlist) = (
                    self.store.state.clone(),
                    workspace.clone(),
                    job.generation.clone(),
                    allowlist.clone(),
                );
                let captured = tokio::task::spawn_blocking(move || {
                    crate::skillcache::snapshot::take_with_allowlist(
                        &state,
                        &root,
                        &generation,
                        &allowlist,
                    )
                    .ok()??;
                    crate::skillcache::replay_result::capture_with(&root, Some(&state), &allowlist)
                        .ok()
                })
                .await
                .ok()
                .flatten();
                PreTurnObservation::Full(captured)
            }
            crate::skillcache::trace::ObservationStrategy::LightweightToolchain => {
                let (root, allowlist) = (workspace.clone(), allowlist.clone());
                let stamp = tokio::task::spawn_blocking(move || {
                    crate::skillcache::trace::capture_toolchain_pre_turn_stamp(&root, &allowlist)
                })
                .await
                .ok()
                .flatten();
                PreTurnObservation::Toolchain(stamp)
            }
            crate::skillcache::trace::ObservationStrategy::None => PreTurnObservation::None,
        };
        let graphs = lock(&self.graphs).ok().and_then(|slot| slot.clone());
        let graphs_for_stream = graphs.clone();
        let project_id = session["project_id"].as_str().unwrap_or("").to_owned();
        let project_id_for_stream = project_id.clone();
        let workspace_for_stream = workspace.clone();
        let observed = Box::pin(provider::run_monitored_locked(
            &launch,
            Limits {
                seconds: session["budgets"]["seconds"].as_u64(),
                ..Limits::default()
            },
            dispatch_cancel,
            previous,
            Some(self.store.runner_lock()),
            Some(dispatch_adapter.as_ref()),
            &binding.environment,
            Some(run_events),
            move |event| {
                if event["kind"] == "usage"
                    && let Some(n) = provider::total_tokens(&event)
                {
                    tokens = tokens.saturating_add(n);
                    if max_tokens.is_some_and(|limit| tokens >= limit) {
                        budget_control.store(true, Ordering::Release);
                        process_control.store(true, Ordering::Release);
                    }
                }
                check_inbox_identity(&store, &sid, &generation, &event)?;
                if event["kind"] == "session"
                    && let Some(native) = event["native_session_id"].as_str()
                {
                    store.patch_guarded(
                        &sid,
                        &generation,
                        &json!({"native_session_id":native}),
                        &control,
                    )?;
                }
                let focus_delta = if event["kind"] == "tool"
                    && let Ok(mut tracker) = focus_for_stream.lock()
                {
                    let delta = observe_tool_event_with_graphs(
                        &mut tracker,
                        &event,
                        graphs_for_stream.as_deref(),
                        &project_id_for_stream,
                        &workspace_for_stream,
                    );
                    if delta.is_some() {
                        let _ =
                            crate::graph_live::save_session_focus(&store, &sid, tracker.focus());
                    }
                    delta
                } else {
                    None
                };
                let tool = event["kind"] == "tool";
                emit_events(event)?;
                if let Some(delta) = focus_delta {
                    emit_events(delta)?;
                }
                // The executor journals each phase beside its state; a tool
                // event is when a new line can have appeared.
                if tool && let Some(warm) = &warm_for_stream {
                    ingest_warm_with_live_focus(
                        warm,
                        &store,
                        &sid,
                        &project_id_for_stream,
                        &workspace_for_stream,
                        graphs_for_stream.as_deref(),
                        &focus_for_stream,
                        &*emit_events,
                    );
                }
                Ok(())
            },
        ))
        .await;
        drop(forwarding);
        // Whatever the run left waiting is answered no.
        self.permissions.expire(&job.sid);
        let observed = observed.map_err(|error| {
            if error.code == "native-cancelled"
                && budget_stop.load(Ordering::Acquire)
                && !cancel.load(Ordering::Acquire)
            {
                HarnessError {
                    status: 409,
                    code: "native-token-budget",
                    message: "The native token budget was reached.".into(),
                }
            } else {
                error
            }
        });
        let result = match observed {
            Ok(result) => result,
            Err(error) => {
                if let Some(warm) = &warm {
                    warm.cleanup_abandoned();
                }
                if error.code == "cleanup-unconfirmed" {
                    self.stopping.store(true, Ordering::Release);
                    let _ = self
                        .lifecycle
                        .boundary(&binding, "cleanup_unconfirmed", &Value::Null);
                }
                let _ = self.lifecycle.abort(
                    &binding,
                    if cancel.load(Ordering::Acquire) {
                        "cancelled"
                    } else {
                        "failed"
                    },
                );
                if let Ok(mut tracker) = live_focus.lock()
                    && tracker.complete()
                {
                    let _ = crate::graph_live::save_session_focus(
                        &self.store,
                        &job.sid,
                        tracker.focus(),
                    );
                    if let Some(delta) = tracker.take_delta_event() {
                        let _ = emit(delta);
                    }
                }
                return Err(error);
            }
        };
        if inbox_owned
            && (expected_native.is_none()
                || result.session_id.as_deref() != expected_native.as_deref())
        {
            if let Some(warm) = &warm {
                warm.cleanup_abandoned();
            }
            let _ = self.lifecycle.abort(&binding, "failed");
            return Err(HarnessError::conflict(
                "The assigned inbox turn has no matching native identity proof.",
            ));
        }
        if let Some(warm) = &warm {
            ingest_warm_with_live_focus(
                warm,
                &self.store,
                &job.sid,
                &project_id,
                &workspace,
                graphs.as_deref(),
                &live_focus,
                &|event| emit(event),
            );
            warm.finish(&self.store, &session, &job.generation, &|event| emit(event));
            crate::skillcache::maybe_enqueue_recompile_on_finish(
                &self.skillcache_runtime(),
                &warm.capsule_id,
                &|event| {
                    let _ = emit(event);
                },
            );
        }
        let mut prepared_trace = if result.ok {
            recorder.as_ref().and_then(|recorder| {
                self.finish_recorded_trace(
                    recorder,
                    &session,
                    &job.prompt,
                    &route,
                    &workspace,
                    &binding.environment,
                )
            })
        } else {
            None
        };
        let trace_eligible = prepared_trace
            .as_ref()
            .is_none_or(|prepared| prepared.admissible.is_ok());
        match pre_turn_obs {
            PreTurnObservation::Full(Some(before)) if result.ok && trace_eligible => {
                // Capture the primary turn's effects before lifecycle recovery
                // or publication can change the project.
                let (state, root, generation, allowlist) = (
                    self.store.state.clone(),
                    workspace.clone(),
                    job.generation.clone(),
                    allowlist,
                );
                let result_text = result.text.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    crate::skillcache::replay_result::record_with_output(
                        &state,
                        &root,
                        &generation,
                        before,
                        Some(&result_text),
                        &allowlist,
                    )
                })
                .await;
            }
            PreTurnObservation::Full(Some(_)) => {
                let _ = crate::skillcache::snapshot::remove(&self.store.state, &job.generation);
            }
            PreTurnObservation::Toolchain(Some(before_stamp)) if result.ok && trace_eligible => {
                let (state, root, generation, allowlist) = (
                    self.store.state.clone(),
                    workspace.clone(),
                    job.generation.clone(),
                    allowlist,
                );
                let result_text = result.text.clone();
                let unchanged = tokio::task::spawn_blocking(move || {
                    let after_stamp = crate::skillcache::trace::capture_toolchain_pre_turn_stamp(
                        &root, &allowlist,
                    );
                    if after_stamp.as_ref() != Some(&before_stamp) {
                        return false;
                    }
                    let taken = crate::skillcache::snapshot::take_with_allowlist(
                        &state,
                        &root,
                        &generation,
                        &allowlist,
                    )
                    .ok()
                    .flatten()
                    .is_some();
                    if taken {
                        let _ = crate::skillcache::replay_result::record_zero_changes_with_output(
                            &state,
                            &generation,
                            Some(&result_text),
                        );
                    }
                    true
                })
                .await
                .unwrap_or(false);
                if !unchanged {
                    // Reject ToolChain admission if workspace files changed post-turn.
                    if let Some(prepared) = prepared_trace.as_mut() {
                        prepared.admissible = Err(json!({
                            "kind": "skillcache",
                            "phase": "not_observed",
                            "reason": "workspace_mutated",
                            "text": "SkillCache: workspace files changed during an unrouted turn, so it was not kept as a read-only tool-chain trace.",
                        }));
                    }
                    let _ = crate::skillcache::snapshot::remove(&self.store.state, &job.generation);
                }
            }
            PreTurnObservation::Toolchain(None) => {
                if let Some(prepared) = prepared_trace.as_mut()
                    && prepared.admissible.is_ok()
                {
                    prepared.admissible = Err(json!({
                        "kind": "skillcache",
                        "phase": "not_observed",
                        "reason": "snapshot_unavailable",
                        "text": "SkillCache: pre-turn workspace fingerprint was unavailable for this turn.",
                    }));
                }
            }
            _ => {}
        }
        let mut recovery = NativeRecovery {
            adapter: recovery_adapter,
            human_cancel: cancel.clone(),
        };
        let memory = Box::pin(
            self.lifecycle
                .finish(&binding, &result, &emit, &mut recovery),
        )
        .await?;
        if let Ok(mut tracker) = live_focus.lock()
            && tracker.complete()
        {
            let _ = crate::graph_live::save_session_focus(&self.store, &job.sid, tracker.focus());
            if let Some(delta) = tracker.take_delta_event() {
                let _ = emit(delta);
            }
        }
        if !result.ok {
            return Err(HarnessError::unavailable(
                result
                    .error
                    .unwrap_or_else(|| "Native session did not complete.".into()),
            ));
        }
        if let Some(prepared) = prepared_trace {
            self.keep_prepared_trace(
                prepared,
                &crate::skillcache::trace::ObservedTurn {
                    session: &session,
                    generation: &job.generation,
                    message: &job.prompt,
                    route: &route,
                    workspace: &workspace,
                    usage: &Value::Null,
                },
                &emit,
            );
        }
        *native_proof = result.session_id.clone();
        Ok(
            json!({"kind":"result","ok":true,"text":result.text,"observed_cost_usd":result.cost_usd,"provider_billing":"unknown","memory":memory["memory"],"full_cost":memory["full_cost"]}),
        )
    }
    /// What SkillCache would decide for a message of the composer, before it
    /// is sent. `body["skillcache"]` carries what the composer shows and a
    /// stored session does not yet have: the mode, and for a new session the
    /// provider, the session's mode and its permissions.
    async fn skillcache_preview(&self, project_id: &str, body: &Value) -> Result<Value> {
        let composer = &body["skillcache"];
        let prompt = body["prompt"].as_str().unwrap_or("").trim().to_string();
        let mut session = match body["session_id"].as_str().filter(|sid| !sid.is_empty()) {
            Some(sid) => self.store.get(sid)?,
            None => {
                let project = self.store.project(project_id)?;
                json!({
                    "id": "",
                    "project_id": project_id,
                    "project_path": project["path"],
                    "workflow": "native",
                    "agents_enabled": false,
                    "agent_count": 3,
                })
            }
        };
        if session["project_id"] != project_id {
            return Err(HarnessError::invalid(
                "Session does not belong to the requested project.",
            ));
        }
        for (field, key) in [
            ("provider", "provider"),
            ("session_mode", "mode"),
            ("permissions", "permissions"),
        ] {
            if let Some(value) = composer[field].as_str() {
                session[key] = value.into();
            }
        }
        if composer.is_object() {
            let mut merged = session["skillcache"]
                .as_object()
                .cloned()
                .unwrap_or_default();
            let mut has_skillcache_field = false;
            for key in [
                "mode",
                "compiler",
                "compile_after_observe",
                "skill",
                "eval_arm",
                "trace_generation",
                "capsule_id",
                "tool_chain",
                "direct_dispatch",
                "thin_formatter",
                "edition",
            ] {
                if let Some(value) = composer.get(key)
                    && !value.is_null()
                {
                    merged.insert(key.into(), value.clone());
                    has_skillcache_field = true;
                }
            }
            if has_skillcache_field {
                if !merged.contains_key("mode") {
                    merged.insert(
                        "mode".into(),
                        crate::skillcache::settings::mode(&session).into(),
                    );
                }
                session["skillcache"] =
                    crate::skillcache::settings::validate(&Value::Object(merged))?;
                crate::skillcache::settings::admit_with_eval(
                    &session,
                    crate::skillcache::settings::eval_enabled(&self.store),
                )?;
            }
        }
        if !crate::skillcache::settings::warm(&session) || prompt.is_empty() {
            return Ok(json!({
                "mode": crate::skillcache::settings::mode(&session),
                "decision": "off",
            }));
        }
        let workspace = PathBuf::from(session["project_path"].as_str().unwrap_or(""));
        let mut route = self.commands()?.route(&session, &prompt).await?;
        self.commands()?
            .apply_skill_hint(&session, &mut route)
            .await?;
        Ok(crate::skillcache::warm::preview(
            &self.skillcache_runtime(),
            &session,
            &route,
            &prompt,
            &workspace,
        ))
    }
    /// Finalizes the trace recorder before post-turn workspace capture so
    /// ineligible turns skip hashing the workspace.
    fn finish_recorded_trace(
        &self,
        recorder: &crate::skillcache::trace::TraceRecorder,
        session: &Value,
        message: &str,
        route: &crate::commands::ResolvedCommand,
        workspace: &Path,
        environment: &BTreeMap<String, Option<String>>,
    ) -> Option<PreparedTrace> {
        let launched = std::env::vars().chain(
            environment
                .iter()
                .filter_map(|(name, value)| Some((name.clone(), value.clone()?))),
        );
        let redactor = crate::skillcache::trace::SessionRedactor::new(workspace, launched);
        let (trace, usage) = recorder.finish_with_usage(&redactor)?;
        let allow_tool_chain =
            crate::skillcache::settings::tool_chain_enabled(&self.store, session);
        let admissible = crate::skillcache::trace::check_admissible_with_toolchain(
            &self.store,
            allow_tool_chain,
            workspace,
            route,
            message,
            &trace,
            &usage,
        )
        .map(|(skills, tools, _)| (skills, tools));
        Some(PreparedTrace {
            trace,
            usage,
            redactor,
            admissible,
        })
    }
    /// Stores the pre-finalized observed trace of a completed turn.
    fn keep_prepared_trace(
        &self,
        prepared: PreparedTrace,
        turn: &crate::skillcache::trace::ObservedTurn<'_>,
        emit: &Event,
    ) {
        let event = match prepared.admissible {
            Err(skipped) => skipped,
            Ok(_) => {
                let usage_turn = crate::skillcache::trace::ObservedTurn {
                    session: turn.session,
                    generation: turn.generation,
                    message: turn.message,
                    route: turn.route,
                    workspace: turn.workspace,
                    usage: &prepared.usage,
                };
                crate::skillcache::trace::save(
                    &self.store,
                    &usage_turn,
                    prepared.trace,
                    &prepared.redactor,
                )
                .unwrap_or_else(|error| {
                    json!({
                        "kind": "skillcache",
                        "phase": "not_observed",
                        "reason": "storage",
                        "text": format!("SkillCache could not keep this turn's trace: {error}"),
                    })
                })
            }
        };
        // A snapshot lives exactly as long as the trace it belongs to.
        let mut auto_queued = None;
        if event["phase"] == "observed" {
            let kept = self
                .store
                .connection()
                .and_then(|db| crate::skillcache::db::trace_generations(&db));
            if let Ok(kept) = kept {
                crate::skillcache::snapshot::collect(&self.store.state, &kept);
            }
            auto_queued = crate::skillcache::maybe_enqueue_after_observe(
                &self.skillcache_runtime(),
                turn.session,
                turn.generation,
            );
        } else {
            let _ = crate::skillcache::snapshot::remove(&self.store.state, turn.generation);
        }
        let _ = emit(event);
        if let Some(queued) = auto_queued {
            let _ = emit(queued);
        }
    }
    pub fn send(&self, sid: &str, prompt: &str, options: &Value) -> Result<Value> {
        let session = self.store.get(sid)?;
        self.commands()?
            .check(session["provider"].as_str().unwrap_or(""), prompt, None)?;
        if active(&session)
            || !session["fleet"].is_null()
            || !session["creator"].is_null()
            || !session["system_run"].is_null()
            || !session["system_discovery"].is_null()
            || !session["system_explain"].is_null()
        {
            return Err(HarnessError::conflict(
                "This session cannot accept a chat follow-up.",
            ));
        }
        let allowed = [
            "model",
            "thinking_effort",
            "sdd",
            "model_routing",
            "mode",
            "attachments",
            "agents_enabled",
            "agent_count",
            "clash",
            "omnigraph",
            "brain",
            "permissions",
            "skillcache",
            "skillcache_thin_formatter",
        ];
        if !options.is_object()
            || options
                .as_object()
                .unwrap()
                .keys()
                .any(|k| !allowed.contains(&k.as_str()))
        {
            return Err(HarnessError::invalid("Unknown follow-up option."));
        }
        let provider = session["provider"].as_str().unwrap_or("");
        let mut fields = followup_settings(&session, options, &self.providers.catalog(provider)?)?;
        if let Some(raw_omni) = options.get("omnigraph") {
            fields["omnigraph"] = merge_omnigraph_followup(&session["omnigraph"], raw_omni, sid)?;
        }
        if let Some(raw) = options.get("skillcache") {
            fields["skillcache"] = crate::skillcache::settings::validate(raw)?;
        }
        if let Some(tf) = options.get("skillcache_thin_formatter") {
            if !tf.is_null() && !tf.is_boolean() {
                return Err(HarnessError::invalid(
                    "`skillcache_thin_formatter` must be true, false, or null.",
                ));
            }
            fields["skillcache_thin_formatter"] = tf.clone();
        }
        {
            // The turn as it would run: SkillCache stays with ordinary turns.
            let mut next = session.clone();
            for key in ["skillcache", "clash", "sdd"] {
                if let Some(value) = fields.get(key) {
                    next[key] = value.clone();
                }
            }
            crate::skillcache::settings::admit_with_eval(
                &next,
                crate::skillcache::settings::eval_enabled(&self.store),
            )?;
        }
        if let Some(update) = options.get("brain") {
            fields["brain"] = crate::brain_settings::followup(&session["brain"], update)?;
        } else if let Some(brain) = session["brain"].as_object() {
            let mut brain = brain.clone();
            brain.insert("approved".into(), false.into());
            brain.insert("context_id".into(), Value::Null);
            fields["brain"] = brain.into();
        }
        let uploads = attachments::validate(fields.get("attachments").unwrap_or(&json!([])))?;
        fields
            .as_object_mut()
            .ok_or_else(|| HarnessError::invalid("Invalid message settings."))?
            .remove("attachments");
        let enabled = fields
            .get("agents_enabled")
            .unwrap_or(&session["agents_enabled"]);
        let count = fields.get("agent_count").unwrap_or(&session["agent_count"]);
        if !enabled.is_boolean() || count.as_u64().is_none_or(|n| !(1..=40).contains(&n)) {
            return Err(HarnessError::invalid("Invalid additional-agent settings."));
        }
        self.enqueue_uploaded(sid, &validate_prompt(prompt)?, &fields, &uploads)
    }
    pub fn continue_fleet(&self, sid: &str, action: &str) -> Result<Value> {
        let session = self.store.get(sid)?;
        if session["fleet"].is_null() {
            return Err(HarnessError::invalid("This is not a Fleet review session."));
        }
        let decision = ["approve", "reject"].contains(&action);
        if decision && session["status"] != "awaiting_approval"
            || !decision
                && !["interrupted", "failed", "cancelled"]
                    .contains(&session["status"].as_str().unwrap_or(""))
        {
            return Err(HarnessError::conflict(
                "This Fleet review is not ready for that action.",
            ));
        }
        let saved: Option<String> = self.store.connection()?.query_row(
            "SELECT fleet_action FROM sessions WHERE id=?1",
            [sid],
            |r| r.get(0),
        )?;
        let action = if action == "resume" {
            saved
                .filter(|s| ["approve", "reject"].contains(&s.as_str()))
                .unwrap_or("resume".into())
        } else {
            action.into()
        };
        self.enqueue(sid, "", &json!({"fleet_action":action}))
    }
    pub fn set_budgets(&self, sid: &str, data: &Value) -> Result<Value> {
        let _admission = lock(&self.admission)?;
        save_budget_settings(&self.store, sid, data)
    }
}
impl Domain for Sessions {
    fn execute<'a>(&'a self, operation: &'a str, request: Value) -> Reply<'a> {
        Box::pin(async move {
            let sid = request["params"]["session_id"].as_str().unwrap_or("");
            let body = &request["body"];
            let value = match operation {
                "sessions.skillcache_preview" => {
                    return Ok(Payload::Json(
                        self.skillcache_preview(
                            request["params"]["project_id"].as_str().unwrap_or(""),
                            body,
                        )
                        .await?,
                    ));
                }
                "sessions.inbox_accept" => {
                    if !store::exact(body, &["request_id", "prompt"], &[]) {
                        return Err(HarnessError::invalid(
                            "Inbox requires only request_id and prompt.",
                        ));
                    }
                    let id = body["request_id"]
                        .as_str()
                        .ok_or_else(|| HarnessError::invalid("Missing request ID."))?;
                    let prompt = body["prompt"]
                        .as_str()
                        .ok_or_else(|| HarnessError::invalid("Missing prompt."))?;
                    return Ok(Payload::Json(self.accept_inbox(sid, id, prompt).await?));
                }
                "sessions.inbox_list" => {
                    let before = request["query"]
                        .get("before")
                        .map(|value| {
                            value
                                .as_str()
                                .and_then(|text| text.parse::<i64>().ok())
                                .filter(|cursor| *cursor > 0)
                                .ok_or_else(|| HarnessError::invalid("Invalid inbox cursor."))
                        })
                        .transpose()?;
                    return Ok(Payload::Json(self.store.inbox_list(sid, before)?));
                }
                "sessions.inbox_cancel" => {
                    if !store::exact(body, &[], &[]) {
                        return Err(HarnessError::invalid(
                            "Inbox cancellation requires an empty object.",
                        ));
                    }
                    let id = request["params"]["request_id"].as_str().unwrap_or("");
                    let _admission = lock(&self.admission)?;
                    return Ok(Payload::Json(self.store.inbox_cancel(sid, id)?));
                }
                "sessions.create" => {
                    let created = self.create_ready(self.create_options(body)?)?;
                    if created["omnigraph"]["linked_session_ids"]
                        .as_array()
                        .is_some_and(|a| !a.is_empty())
                        && let Some(omnigraph) = self.omnigraph()
                    {
                        let _ = omnigraph
                            .sync_new_session_links(&self.store, &created)
                            .await;
                    }
                    created
                }
                "sessions.send" => {
                    let prompt = body["prompt"]
                        .as_str()
                        .ok_or_else(|| HarnessError::invalid("Missing prompt."))?;
                    let mut options = body
                        .as_object()
                        .cloned()
                        .ok_or_else(|| HarnessError::invalid("Invalid message options."))?;
                    options.remove("prompt");
                    self.send(sid, prompt, &options.into())?
                }
                "sessions.permission" => {
                    let request = request["params"]["request_id"].as_str().unwrap_or("");
                    let answer = self.decide_permission(sid, request, body)?;
                    return Ok(Payload::Json(
                        json!({"permission": answer, "session": self.store.get(sid)?}),
                    ));
                }
                "sessions.waiting_permissions" => {
                    return Ok(Payload::Json(
                        json!({"items": self.waiting_permissions(sid)}),
                    ));
                }
                "sessions.waiting_counts" => {
                    return Ok(Payload::Json(
                        json!({"counts": self.permissions.waiting_counts()}),
                    ));
                }
                "sessions.permission_rules_list" => {
                    self.store.get(sid)?;
                    let rules = self.permissions.list_session_rules(sid)?;
                    return Ok(Payload::Json(json!({
                        "session_id": sid,
                        "rules": rules,
                    })));
                }
                "sessions.permission_rules_delete" => {
                    self.store.get(sid)?;
                    if !store::exact(body, &[], &["rule_key", "id", "all"]) {
                        return Err(HarnessError::invalid(
                            "Invalid session permission rule revocation request.",
                        ));
                    }
                    let req_query = &request["query"];
                    let all_from_query =
                        matches!(req_query["all"].as_str(), Some("1" | "true" | "yes"));
                    let all = match body.get("all") {
                        None | Some(Value::Null) => all_from_query,
                        Some(Value::Bool(b)) => *b,
                        Some(_) => {
                            return Err(HarnessError::invalid(
                                "Permission rule 'all' flag must be boolean.",
                            ));
                        }
                    };
                    let rule_key = body["rule_key"]
                        .as_str()
                        .or_else(|| req_query["rule_key"].as_str());
                    let rule_id = request["params"]["rule_id"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .or_else(|| body["id"].as_str())
                        .or_else(|| req_query["id"].as_str());
                    let (removed, rules) = self
                        .permissions
                        .revoke_session_rule(sid, rule_key, rule_id, all)?;
                    return Ok(Payload::Json(json!({
                        "ok": true,
                        "session_id": sid,
                        "removed": removed,
                        "rules": rules,
                    })));
                }
                "sessions.cancel" => {
                    if !store::exact(body, &[], &[]) {
                        return Err(HarnessError::invalid(
                            "Cancellation requires an empty object.",
                        ));
                    }
                    self.cancel(sid)?
                }
                "sessions.decide" => {
                    if !store::exact(body, &["approve"], &[]) {
                        return Err(HarnessError::invalid("Invalid report decision."));
                    }
                    let approve = body["approve"].as_bool().ok_or_else(|| {
                        HarnessError::invalid("The report decision must be a boolean.")
                    })?;
                    self.continue_fleet(sid, if approve { "approve" } else { "reject" })?
                }
                "sessions.resume" => {
                    if !store::exact(body, &[], &[]) {
                        return Err(HarnessError::invalid("Resume requires an empty object."));
                    }
                    self.continue_fleet(sid, "resume")?
                }
                "sessions.budgets" => self.set_budgets(sid, body)?,
                "sessions.merge" => self.merge(body)?,
                "sessions.restart_merge" => self.restart_merge(sid)?,
                "context.prepare" => {
                    let (mut session, query_text) = {
                        let _admission = lock(&self.admission)?;
                        let session = self.store.get(sid)?;
                        if active(&session) {
                            return Err(HarnessError::conflict("Wait for this session to finish."));
                        }
                        if !store::exact(body, &["query"], &[]) {
                            return Err(HarnessError::invalid("Invalid context request."));
                        }
                        let query_text = body["query"]
                            .as_str()
                            .ok_or_else(|| HarnessError::invalid("Missing context query."))?
                            .to_owned();
                        (session, query_text)
                    };
                    let mut updated_omni = None;
                    if session["omnigraph"]["enabled"] == true
                        && let Some(omnigraph) = self.omnigraph()
                    {
                        let mut prep_session = session.clone();
                        prep_session["omnigraph"]["query"] = json!(query_text);
                        let workspace =
                            PathBuf::from(session["project_path"].as_str().unwrap_or(""));
                        match omnigraph
                            .prepare_session_context(&prep_session, &workspace)
                            .await
                        {
                            Ok(prep) => {
                                if let Some(mut merged) = apply_prepared_omnigraph_response(
                                    &session["omnigraph"],
                                    sid,
                                    &prep,
                                ) {
                                    merged["query"] = json!(query_text);
                                    session["omnigraph"] = merged.clone();
                                    updated_omni = Some(merged);
                                }
                            }
                            Err(err) => {
                                let mut omni = session["omnigraph"].clone();
                                if let Some(obj) = omni.as_object_mut() {
                                    obj.insert("last_error".into(), json!(err.message));
                                }
                                session["omnigraph"] = omni.clone();
                                updated_omni = Some(omni);
                            }
                        }
                    }
                    let _admission = lock(&self.admission)?;
                    let latest = self.store.get(sid)?;
                    if active(&latest) {
                        return Err(HarnessError::conflict("Wait for this session to finish."));
                    }
                    let mut prep_input = latest;
                    if let Some(ref omni) = updated_omni {
                        prep_input["omnigraph"] = omni.clone();
                    }
                    let prepared = self.knowledge.prepare(&prep_input, &query_text)?;
                    let patch = if let Some(omni) = updated_omni {
                        json!({"brain": prepared, "omnigraph": omni})
                    } else {
                        json!({"brain": prepared})
                    };
                    self.store.patch(sid, &patch)?
                }
                "context.run" => {
                    let mut session = self.store.get(sid)?;
                    if !store::exact(body, &["context_id"], &[])
                        || session["brain"]["context_id"] != body["context_id"]
                    {
                        return Err(HarnessError::conflict(
                            "Prepared context changed; review it again.",
                        ));
                    }
                    self.knowledge.verify_prepared(&session)?;
                    session["brain"]["approved"] = true.into();
                    self.enqueue(sid, "", &json!({"brain":session["brain"]}))?
                }
                "brain.info" => {
                    let session = self.store.get(sid)?;
                    let brain = &session["brain"];
                    if !brain.is_object() {
                        return Err(HarnessError::invalid(
                            "This session has no linked Brain task.",
                        ));
                    }
                    let mut info = self.knowledge.inspect(
                        session["project_id"].as_str().unwrap_or(""),
                        brain["bank"].as_str(),
                        Some(Path::new(session["project_path"].as_str().unwrap_or(""))),
                        brain["record_id"]
                            .as_str()
                            .or_else(|| brain["task_id"].as_str()),
                        false,
                    )?;
                    let latest = crate::memory_attempts::latest(&self.store, sid)?;
                    info["context_id"] = brain["context_id"].clone();
                    info["approved"] = (brain["approved"] == true).into();
                    info["memory_draft"] = json!({"state":latest.state,"draft":latest.draft,"event_id":latest.event_id});
                    return Ok(Payload::Json(info));
                }
                "brain.act" => {
                    let _admission = lock(&self.admission)?;
                    let session = self.store.get(sid)?;
                    if active(&session) {
                        return Err(HarnessError::conflict("Wait for this session to finish."));
                    }
                    if body["action"] == "complete" && session["status"] != "completed" {
                        return Err(HarnessError::conflict(
                            "Finish the session before recording its task outcome.",
                        ));
                    }
                    return Ok(Payload::Json(self.knowledge.brain_action(&session, body)?));
                }
                "memory.save" => {
                    let saved = {
                        let _admission = lock(&self.admission)?;
                        let session = self.store.get(sid)?;
                        if active(&session) {
                            return Err(HarnessError::conflict("Wait for this session to finish."));
                        }
                        let saved = self.knowledge.save_memory(&session, body)?;
                        (session, saved)
                    };
                    if saved.1["ok"] == true
                        && let Ok(results) = self.results()
                    {
                        let _ = results
                            .publish_manual_memory_to_omnigraph(
                                &saved.0,
                                body,
                                Some(&saved.1["saved"]),
                            )
                            .await;
                    }
                    return Ok(Payload::Json(saved.1));
                }
                _ => return Err(HarnessError::missing("Unknown session operation.")),
            };
            Ok(Payload::Json(json!({"session":value})))
        })
    }
}

#[inline(never)]
/// Whether a finished run's memory save wrote what the canonical branch
/// carries: verified records, or chunks promoted into the bank.
fn saved_canonical_knowledge(memory: &Value) -> bool {
    memory["records"].as_u64().unwrap_or(0) > 0 || memory["bank_promoted"] == true
}

fn spawn_omnigraph_publish(
    results: Arc<crate::results::Results>,
    sid: &str,
    generation: &str,
    result_val: &Value,
    cancel: &Arc<AtomicBool>,
) -> tokio::task::JoinHandle<()> {
    let pub_sid = sid.to_owned();
    let pub_gen = generation.to_owned();
    let pub_val = result_val.clone();
    let pub_cancel = cancel.clone();
    tokio::spawn(async move {
        let _ = results
            .publish_omnigraph_for_run(&pub_sid, &pub_gen, &pub_val, &pub_cancel)
            .await;
    })
}

fn check_inbox_identity(
    store: &SessionStore,
    sid: &str,
    generation: &str,
    event: &Value,
) -> Result<()> {
    if let Some(native) = event["native_session_id"].as_str()
        && store
            .inbox_native_guard(sid, generation)?
            .as_deref()
            .is_some_and(|expected| expected != native)
    {
        return Err(HarnessError::conflict(
            "The inbox native session identity changed in the provider stream.",
        ));
    }
    Ok(())
}

fn finalize_job(
    store: &SessionStore,
    job: &Job,
    result: &Result<Value>,
    cancel: &AtomicBool,
) -> Result<String> {
    let cancelled = cancel.load(Ordering::Acquire);
    let mut fields = json!({});
    let mut events = Vec::new();
    let mut status = if cancelled {
        "cancelled"
    } else if result.is_ok() {
        "completed"
    } else {
        "failed"
    }
    .to_owned();
    match &job.kind {
        JobKind::Check {
            previous_status, ..
        } => {
            if !cancelled {
                status = previous_status.clone();
            }
            fields["status"] = status.clone().into();
            if let Err(error) = result {
                events.push(json!({"kind":"error","text":error.message}));
            }
            events.push(json!({"kind":"status","text":"Check finished.","status":status}));
        }
        JobKind::Run => match result {
            Ok(value) if value["kind"] == "awaiting_context" => {
                if !cancelled {
                    status = "awaiting_context".into();
                }
            }
            Ok(value) if value["kind"] == "fleet_state" => {
                fields["fleet_result"] = value.clone();
                if !cancelled {
                    status = value["status"]
                        .as_str()
                        .ok_or_else(|| HarnessError::invalid("Fleet terminal status is missing."))?
                        .into();
                }
                fields["status"] = status.clone().into();
            }
            Ok(value) => {
                if !cancelled && (value["outcome"] == "incomplete" || value["ok"] == false) {
                    status = "failed".into();
                }
                fields["status"] = status.clone().into();
            }
            Err(error) => {
                events.push(json!({"kind":"error","text":error.message}));
                fields["status"] = status.clone().into();
            }
        },
    }
    if let Some(session) =
        store.finish_generation(&job.sid, &job.generation, &fields, &events, cancel)?
    {
        status = session["status"]
            .as_str()
            .ok_or_else(|| HarnessError::invalid("Terminal session status is missing."))?
            .into();
    }
    // This is the old job's result, never a status fetched from a newer row.
    Ok(status)
}

fn validate_branch_token(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.len() > 128
        || trimmed.chars().any(|c| c.is_control() || c.is_whitespace())
        || trimmed.starts_with('/')
        || trimmed.ends_with('/')
        || trimmed.contains("..")
        || trimmed.contains("//")
    {
        return Err(HarnessError::invalid("Invalid Omnigraph branch name."));
    }
    Ok(trimmed.to_owned())
}

fn validate_omnigraph_fields(raw: &Value, sid: &str, base: Option<&Value>) -> Result<Value> {
    let Some(obj) = raw.as_object() else {
        return Err(HarnessError::invalid("Invalid Omnigraph session settings."));
    };
    let allowed = [
        "enabled",
        "branch",
        "pull_branches",
        "query",
        "auto_publish",
        "auto_merge",
        "shared_space",
        "linked_session_ids",
    ];
    if obj.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(HarnessError::invalid("Unknown Omnigraph session setting."));
    }

    let mut out = match base.filter(|b| b.is_object()) {
        Some(existing) => existing.clone(),
        None => json!({
            "enabled": true,
            "branch": crate::omnigraph::session_branch_name(sid),
            "pull_branches": ["main"],
            "query": Value::Null,
            "auto_publish": true,
            "auto_merge": false,
            "shared_space": Value::Null,
            "linked_session_ids": [],
            "capsule_text": "",
            "injected_atoms": [],
            "published_atoms": [],
            "merge_status": Value::Null,
            "last_error": Value::Null,
        }),
    };
    if out.get("shared_space").is_none() {
        out["shared_space"] = Value::Null;
    }
    if !out["linked_session_ids"].is_array() {
        out["linked_session_ids"] = json!([]);
    }

    if let Some(v) = obj.get("enabled") {
        let b = v
            .as_bool()
            .ok_or_else(|| HarnessError::invalid("Omnigraph `enabled` must be a boolean."))?;
        out["enabled"] = json!(b);
    }
    if let Some(v) = obj.get("branch") {
        let resolved = if v.is_null() {
            crate::omnigraph::session_branch_name(sid)
        } else {
            let s = v
                .as_str()
                .ok_or_else(|| HarnessError::invalid("Omnigraph `branch` must be text."))?;
            let trimmed = s.trim();
            if trimmed.is_empty() || trimmed == "auto" {
                crate::omnigraph::session_branch_name(sid)
            } else {
                validate_branch_token(trimmed)?
            }
        };
        out["branch"] = json!(resolved);
    }
    if let Some(v) = obj.get("pull_branches") {
        let arr = v
            .as_array()
            .ok_or_else(|| HarnessError::invalid("Omnigraph `pull_branches` must be an array."))?;
        if arr.len() > 16 {
            return Err(HarnessError::invalid(
                "Omnigraph `pull_branches` allows at most 16 branches.",
            ));
        }
        let mut parsed = Vec::with_capacity(arr.len());
        for item in arr {
            let s = item
                .as_str()
                .ok_or_else(|| HarnessError::invalid("Omnigraph branch names must be text."))?;
            parsed.push(Value::String(validate_branch_token(s)?));
        }
        out["pull_branches"] = Value::Array(parsed);
    }
    if let Some(v) = obj.get("query") {
        if v.is_null() {
            out["query"] = Value::Null;
        } else {
            let s = v
                .as_str()
                .ok_or_else(|| HarnessError::invalid("Omnigraph `query` must be text."))?;
            if s.chars().count() > 4000
                || s.chars().any(|c| c.is_control() && c != '\n' && c != '\t')
            {
                return Err(HarnessError::invalid("Invalid Omnigraph `query`."));
            }
            let trimmed = s.trim();
            out["query"] = if trimmed.is_empty() {
                Value::Null
            } else {
                json!(trimmed)
            };
        }
    }
    for flag in ["auto_publish", "auto_merge"] {
        if let Some(v) = obj.get(flag) {
            let b = v.as_bool().ok_or_else(|| {
                HarnessError::invalid(format!("Omnigraph `{flag}` must be a boolean."))
            })?;
            out[flag] = json!(b);
        }
    }
    if let Some(v) = obj.get("shared_space") {
        if v.is_null() {
            out["shared_space"] = Value::Null;
        } else {
            let s = v
                .as_str()
                .ok_or_else(|| HarnessError::invalid("Omnigraph `shared_space` must be text."))?;
            let trimmed = s.trim();
            if trimmed.is_empty() {
                out["shared_space"] = Value::Null;
            } else {
                let normalized = crate::omnigraph::normalize_shared_space_branch(trimmed)?;
                out["shared_space"] = json!(normalized);
            }
        }
    }
    if let Some(v) = obj.get("linked_session_ids") {
        let arr = v.as_array().ok_or_else(|| {
            HarnessError::invalid("Omnigraph `linked_session_ids` must be an array.")
        })?;
        if arr.len() > 32 {
            return Err(HarnessError::invalid(
                "Omnigraph `linked_session_ids` allows at most 32 sessions.",
            ));
        }
        let mut parsed = Vec::with_capacity(arr.len());
        let mut seen = std::collections::BTreeSet::new();
        for item in arr {
            let s = item.as_str().ok_or_else(|| {
                HarnessError::invalid("Omnigraph `linked_session_ids` items must be text.")
            })?;
            let trimmed = s.trim();
            if trimmed.is_empty() || trimmed.len() > 128 || trimmed.chars().any(char::is_control) {
                return Err(HarnessError::invalid(
                    "Invalid session id in Omnigraph `linked_session_ids`.",
                ));
            }
            if trimmed != sid && seen.insert(trimmed.to_string()) {
                parsed.push(Value::String(trimmed.to_string()));
            }
        }
        out["linked_session_ids"] = Value::Array(parsed);
    }
    Ok(out)
}

pub(crate) fn validate_omnigraph_options(raw: &Value, sid: &str) -> Result<Value> {
    if raw.is_null() {
        return Ok(Value::Null);
    }
    validate_omnigraph_fields(raw, sid, None)
}

fn merge_omnigraph_followup(existing: &Value, raw: &Value, sid: &str) -> Result<Value> {
    if raw.is_null() {
        return Ok(Value::Null);
    }
    validate_omnigraph_fields(raw, sid, Some(existing))
}

fn atom_id_of(atom: &Value) -> Option<String> {
    atom["atom_id"]
        .as_str()
        .or_else(|| atom["id"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn apply_prepared_omnigraph_response(existing: &Value, sid: &str, prep: &Value) -> Option<Value> {
    if prep["enabled"] != true {
        return None;
    }
    let branch = prep["branch"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| crate::omnigraph::session_branch_name(sid));
    let atoms = prep["atoms"].as_array().cloned().unwrap_or_default();
    let capsule_override = prep["capsule_text"].as_str();
    let mut updated_omni =
        merge_omnigraph_context(existing, &atoms, &branch, capsule_override, None);
    if prep.get("head_commit").is_some_and(|c| !c.is_null()) {
        updated_omni["head_commit"] = prep["head_commit"].clone();
    }
    // What this turn's capsule cautions about, for a person to review.
    updated_omni["contradiction_warnings"] = match prep["contradiction_warnings"].as_array() {
        Some(warnings) => Value::Array(warnings.iter().take(24).cloned().collect()),
        None => json!([]),
    };
    updated_omni["last_error"] = Value::Null;
    Some(updated_omni)
}

fn compact_atom_summary(atom: &Value) -> Value {
    let aid = atom_id_of(atom).unwrap_or_default();
    let mut out = json!({
        "atom_id": aid,
        "kind": atom["kind"].as_str().unwrap_or("insight"),
        "title": atom["title"].as_str().unwrap_or("Shared context"),
        "summary": atom["summary"].as_str().unwrap_or(""),
        "source_branch": atom["source_branch"].as_str().unwrap_or("main"),
        "confidence": atom["confidence"].as_f64().unwrap_or(0.85),
        "pin_priority": atom["pin_priority"].as_i64().unwrap_or(10),
        "verified": atom["verified"].as_bool().unwrap_or(false),
    });
    for key in [
        "origin_layer",
        "authority",
        "privacy",
        "brain_record_id",
        "memory_id",
    ] {
        if let Some(val) = atom[key].as_str().map(str::trim).filter(|s| !s.is_empty()) {
            out[key] = json!(val);
        }
    }
    if let Some(rev) = atom["record_revision"].as_i64().or_else(|| {
        atom["record_revision"]
            .as_u64()
            .and_then(|n| i64::try_from(n).ok())
    }) {
        out["record_revision"] = json!(rev);
    }
    if let Some(files) = atom["files"].as_array() {
        let cleaned: Vec<Value> = files
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .take(24)
            .map(|s| json!(s))
            .collect();
        if !cleaned.is_empty() {
            out["files"] = Value::Array(cleaned);
        }
    }
    out
}

/// A carried atom keeps the authority tag it was first delivered with: an
/// unverified one stays marked as a reference to check.
fn format_atom_line(atom: &Value) -> String {
    crate::omnigraph::sidecar::format_atom_line_with_authority(atom)
}

/// Merge newly prepared or explicitly shared Omnigraph atoms into `session["omnigraph"]` without
/// losing prior shared items, updating `injected_atoms` and bounded `capsule_text`.
pub(crate) fn merge_omnigraph_context(
    existing: &Value,
    new_atoms: &[Value],
    branch: &str,
    capsule_override: Option<&str>,
    source_branch_fallback: Option<&str>,
) -> Value {
    let mut omni = if existing.is_object() {
        existing.clone()
    } else {
        json!({
            "enabled": true,
            "branch": branch,
            "pull_branches": ["main"],
            "query": Value::Null,
            "auto_publish": true,
            "auto_merge": false,
            "shared_space": Value::Null,
            "linked_session_ids": [],
            "capsule_text": "",
            "injected_atoms": [],
            "published_atoms": [],
            "merge_status": Value::Null,
            "last_error": Value::Null,
        })
    };
    if omni["enabled"].is_null() {
        omni["enabled"] = json!(true);
    }
    if omni.get("shared_space").is_none() {
        omni["shared_space"] = Value::Null;
    }
    if !omni["linked_session_ids"].is_array() {
        omni["linked_session_ids"] = json!([]);
    }

    let mut merged = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for atom in new_atoms {
        let mut entry = compact_atom_summary(atom);
        if entry["source_branch"] == "main"
            && let Some(fb) = source_branch_fallback.filter(|s| !s.trim().is_empty())
            && atom["source_branch"].as_str().is_none()
        {
            entry["source_branch"] = json!(fb);
        }
        let aid = atom_id_of(&entry).unwrap_or_default();
        if !aid.is_empty() && seen.insert(aid) {
            merged.push(entry);
        }
    }
    let mut carried = Vec::new();
    for old in omni["injected_atoms"].as_array().into_iter().flatten() {
        let entry = compact_atom_summary(old);
        let aid = atom_id_of(&entry).unwrap_or_default();
        if !aid.is_empty() && seen.insert(aid) && merged.len() < 24 {
            carried.push(entry.clone());
            merged.push(entry);
        }
    }
    merged.truncate(24);

    let mut capsule = if let Some(raw) = capsule_override.map(str::trim).filter(|s| !s.is_empty()) {
        let mut out = format!("{raw}\n");
        for item in &carried {
            out.push_str(&format_atom_line(item));
        }
        out
    } else if !merged.is_empty() {
        let mut out = format!("Omnigraph Shared Context (branch `{branch}`):\n");
        for item in &merged {
            out.push_str(&format_atom_line(item));
        }
        out
    } else {
        omni["capsule_text"].as_str().unwrap_or("").to_owned()
    };

    if capsule.chars().count() > 8000 {
        let truncated: String = capsule.chars().take(7980).collect();
        capsule = format!("{}\n…[truncated]\n", truncated.trim_end());
    }

    omni["branch"] = json!(branch);
    omni["injected_atoms"] = Value::Array(merged);
    omni["capsule_text"] = json!(capsule);
    omni["last_error"] = Value::Null;
    omni
}

/// Update `session["omnigraph"]` (`injected_atoms` and `capsule_text`) and emit a session event
/// after `omnigraph.share` succeeds for an existing Harness session.
pub(crate) async fn sync_omnigraph_share_to_session(
    store: &SessionStore,
    omnigraph_domain: &dyn Domain,
    body: &Value,
    share_result: &Value,
) -> Option<Value> {
    if share_result["ok"] != true {
        return None;
    }
    let target_session_id = share_result["target_session_id"]
        .as_str()
        .or_else(|| body["target_session_id"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let Ok(session) = store.get(target_session_id) else {
        return None;
    };
    let atom_ids: Vec<String> = share_result["atom_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if atom_ids.is_empty() {
        return None;
    }
    let source_branch = body["source_branch"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("main");
    let target_branch = share_result["target_branch"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| crate::omnigraph::session_branch_name(target_session_id));
    let project_id = session["project_id"].as_str().unwrap_or("");
    let mode = share_result["mode"]
        .as_str()
        .or_else(|| body["mode"].as_str())
        .unwrap_or("reference");

    let mut fetched = Vec::new();
    let mut found_ids = std::collections::BTreeSet::new();
    for branch in [source_branch, target_branch.as_str()] {
        if found_ids.len() >= atom_ids.len() {
            break;
        }
        if let Ok(Payload::Json(search)) = omnigraph_domain
            .execute(
                "omnigraph.search",
                json!({
                    "params": {},
                    "query": { "project_id": project_id, "q": "", "branch": branch, "limit": "100" },
                    "body": {},
                }),
            )
            .await
        {
            for hit in search["hits"].as_array().into_iter().flatten() {
                let atom = &hit["atom"];
                if let Some(aid) = atom_id_of(atom)
                    && atom_ids.contains(&aid)
                    && found_ids.insert(aid)
                {
                    fetched.push(atom.clone());
                }
            }
        }
    }
    for aid in &atom_ids {
        if !found_ids.contains(aid) {
            fetched.push(json!({
                "atom_id": aid,
                "kind": "insight",
                "title": aid,
                "summary": "",
                "source_branch": source_branch,
            }));
        }
    }

    let updated_omni = merge_omnigraph_context(
        &session["omnigraph"],
        &fetched,
        &target_branch,
        None,
        Some(source_branch),
    );
    let patched = store
        .patch(target_session_id, &json!({ "omnigraph": updated_omni }))
        .ok()?;
    let status = session["status"].as_str().unwrap_or("");
    let qualifier = if matches!(status, "queued" | "running") {
        "active session"
    } else {
        "session"
    };
    let _ = store.event(
        target_session_id,
        &json!({
            "kind": "omnigraph",
            "phase": "share",
            "ok": true,
            "mode": mode,
            "source_branch": source_branch,
            "target_branch": target_branch,
            "atom_ids": atom_ids,
            "session_status": status,
            "text": format!(
                "Shared {} Omnigraph context atom(s) from `{source_branch}` into {qualifier} `{target_session_id}` ({mode}).",
                atom_ids.len()
            ),
        }),
    );
    Some(patched)
}

/// Update `session["omnigraph"]["merge_status"]` and emit a session event when
/// `omnigraph.branch_act` (`action == "merge"`) successfully merges a session branch.
pub(crate) fn sync_omnigraph_merge_to_session(
    store: &SessionStore,
    body: &Value,
    merge_result: &Value,
) -> Option<Value> {
    if merge_result["ok"] != true
        || merge_result["action"] != "merge"
        || merge_result["dry_run"] == true
    {
        return None;
    }
    let outcome = merge_result["outcome"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "conflict")?;
    let source = merge_result["source"]
        .as_str()
        .or_else(|| body["source"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let explicit_sid = body["session_id"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let session = if let Some(sid) = explicit_sid {
        store.get(sid).ok()?
    } else {
        let src = source?;
        let matched = store.summaries().ok()?.into_iter().find(|s| {
            s["omnigraph"]["branch"].as_str() == Some(src)
                || s["id"]
                    .as_str()
                    .is_some_and(|id| crate::omnigraph::session_branch_name(id) == src)
        })?;
        let sid = matched["id"].as_str()?;
        store.get(sid).ok()?
    };

    let sid = session["id"].as_str()?.to_owned();
    let mut omni = if session["omnigraph"].is_object() {
        session["omnigraph"].clone()
    } else {
        json!({ "enabled": true })
    };
    omni["merge_status"] = json!(outcome);
    omni["last_error"] = Value::Null;
    let patched = store.patch(&sid, &json!({ "omnigraph": omni })).ok()?;
    let branch = source.unwrap_or("main");
    let target = merge_result["target"].as_str().unwrap_or("main");
    let _ = store.event(
        &sid,
        &json!({
            "kind": "omnigraph",
            "phase": "merge",
            "ok": true,
            "branch": branch,
            "target": target,
            "merge_status": outcome,
            "text": format!("Merged Omnigraph branch `{branch}` into `{target}` ({outcome})."),
        }),
    );
    Some(patched)
}

fn append_unique_str_list(array_val: &mut Value, additions: &[&str], limit: usize) {
    let mut items: Vec<String> = array_val
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    for add in additions {
        let trimmed = add.trim();
        if !trimmed.is_empty() && !items.iter().any(|existing| existing == trimmed) {
            if items.len() >= limit {
                break;
            }
            items.push(trimmed.to_string());
        }
    }
    *array_val = Value::Array(items.into_iter().map(Value::String).collect());
}

fn default_omnigraph_object(branch: &str) -> Value {
    json!({
        "enabled": true,
        "branch": branch,
        "pull_branches": ["main"],
        "query": Value::Null,
        "auto_publish": true,
        "auto_merge": false,
        "shared_space": Value::Null,
        "linked_session_ids": [],
        "capsule_text": "",
        "injected_atoms": [],
        "published_atoms": [],
        "merge_status": Value::Null,
        "last_error": Value::Null,
    })
}

fn enrich_linked_pull_branches(
    store: &SessionStore,
    project_id: &str,
    omni: &mut Value,
) -> Result<()> {
    let mut extra_branches = Vec::new();
    if let Some(raw_space) = omni["shared_space"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        && let Ok(space_branch) = crate::omnigraph::normalize_shared_space_branch(raw_space)
    {
        omni["shared_space"] = json!(space_branch);
        extra_branches.push(space_branch);
    }

    let linked: Vec<String> = omni["linked_session_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    for peer_id in &linked {
        let peer = store.get(peer_id)?;
        if peer["project_id"].as_str().unwrap_or("") != project_id {
            return Err(HarnessError::invalid(format!(
                "Linked session `{peer_id}` belongs to a different project."
            )));
        }
        let peer_branch = peer["omnigraph"]["branch"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "auto")
            .map(str::to_string)
            .unwrap_or_else(|| crate::omnigraph::session_branch_name(peer_id));
        extra_branches.push(peer_branch);
        if let Some(raw_space) = peer["omnigraph"]["shared_space"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            && let Ok(space_branch) = crate::omnigraph::normalize_shared_space_branch(raw_space)
        {
            extra_branches.push(space_branch);
        }
    }
    if !extra_branches.is_empty() {
        let refs: Vec<&str> = extra_branches.iter().map(String::as_str).collect();
        append_unique_str_list(&mut omni["pull_branches"], &refs, 32);
    }
    Ok(())
}

fn sync_reciprocal_session_links(store: &SessionStore, session: &Value) {
    let Some(sid) = session["id"].as_str().filter(|s| !s.is_empty()) else {
        return;
    };
    let linked: Vec<String> = session["omnigraph"]["linked_session_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != sid)
        .map(str::to_string)
        .collect();
    if linked.is_empty() {
        return;
    }
    let new_branch = session["omnigraph"]["branch"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "auto")
        .map(str::to_string)
        .unwrap_or_else(|| crate::omnigraph::session_branch_name(sid));
    let shared_space = session["omnigraph"]["shared_space"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| crate::omnigraph::normalize_shared_space_branch(s).ok());

    for peer_id in &linked {
        let Ok(peer) = store.get(peer_id) else {
            continue;
        };
        let peer_branch = peer["omnigraph"]["branch"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "auto")
            .map(str::to_string)
            .unwrap_or_else(|| crate::omnigraph::session_branch_name(peer_id));
        let mut peer_omni = if peer["omnigraph"].is_object() {
            peer["omnigraph"].clone()
        } else {
            default_omnigraph_object(&peer_branch)
        };
        peer_omni["enabled"] = json!(true);
        append_unique_str_list(&mut peer_omni["linked_session_ids"], &[sid], 32);
        let mut pulls = vec![new_branch.as_str()];
        if let Some(ref sp) = shared_space {
            pulls.push(sp.as_str());
        }
        append_unique_str_list(&mut peer_omni["pull_branches"], &pulls, 32);
        if peer_omni["shared_space"].is_null()
            && let Some(ref sp) = shared_space
        {
            peer_omni["shared_space"] = json!(sp);
        }
        if store
            .patch(peer_id, &json!({ "omnigraph": peer_omni }))
            .is_ok()
        {
            let _ = store.event(
                peer_id,
                &json!({
                    "kind": "omnigraph",
                    "phase": "link",
                    "ok": true,
                    "mode": "linked_create",
                    "linked_session_ids": [sid],
                    "branch": new_branch,
                    "shared_space": shared_space,
                    "text": format!("Linked Omnigraph context with new session `{sid}` (`{new_branch}`)."),
                }),
            );
        }
    }
}

/// Tell a running native session about a context update through its inbox,
/// and say what happened: `notified`, or the `reason` it was not. A session
/// that is not running has nothing to tell; a running one that cannot take
/// the follow-up, or whose inbox refuses it, records an `inbox_notify` event.
async fn notify_live_native_session_inbox(
    store: &SessionStore,
    sessions_domain: Option<&dyn Domain>,
    session: &Value,
    request_prefix: &str,
    prompt: &str,
) -> Value {
    let sid = session["id"].as_str().unwrap_or("");
    let mut outcome = json!({ "session_id": sid, "notified": false });
    let refused = if let Err(reason) = crate::inbox::live_eligibility_reason(session) {
        outcome["reason"] = json!(reason);
        reason != "not_running"
    } else if !sid.is_empty()
        && store
            .get(sid)
            .ok()
            .is_some_and(|fresh| fresh["permissions"] == "manual")
    {
        outcome["reason"] = json!("manual_permissions");
        true
    } else if let Some(domain) = sessions_domain {
        let request_id = format!("{request_prefix}-{}", &store::hexid()[..12]);
        match domain
            .execute(
                "sessions.inbox_accept",
                json!({
                    "params": { "session_id": sid },
                    "query": {},
                    "body": {
                        "request_id": request_id,
                        "prompt": prompt,
                    },
                }),
            )
            .await
        {
            Ok(_) => {
                outcome["notified"] = json!(true);
                outcome["request_id"] = json!(request_id);
                false
            }
            Err(error) => {
                outcome["reason"] = json!("inbox_refused");
                outcome["error"] = json!(error.message);
                true
            }
        }
    } else {
        outcome["reason"] = json!("inbox_unavailable");
        true
    };
    if refused && !sid.is_empty() {
        let reason = outcome["reason"].as_str().unwrap_or("");
        let detail = outcome["error"]
            .as_str()
            .map(|error| format!(": {error}"))
            .unwrap_or_default();
        let _ = store.event(
            sid,
            &json!({
                "kind": "omnigraph",
                "phase": "inbox_notify",
                "ok": false,
                "reason": reason,
                "text": format!(
                    "The running turn was not told about the Omnigraph update ({reason}{detail}); the next turn reads it from the session's context."
                ),
            }),
        );
    }
    outcome
}

/// Sessions an Omnigraph update changed, and what each running one was told.
pub(crate) struct SessionContextSync {
    pub sessions: Vec<Value>,
    pub inbox_notifications: Vec<Value>,
}

/// Update `session["omnigraph"]`, emit events, and notify active native sessions after `omnigraph.import`.
pub(crate) async fn sync_omnigraph_import_to_sessions(
    store: &SessionStore,
    sessions_domain: Option<&dyn Domain>,
    body: &Value,
    import_result: &Value,
) -> SessionContextSync {
    let mut synced = SessionContextSync {
        sessions: Vec::new(),
        inbox_notifications: Vec::new(),
    };
    if import_result["ok"] != true {
        return synced;
    }
    let shared_sids: Vec<String> = import_result["shared_session_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if shared_sids.is_empty() {
        return synced;
    }
    let target_branch = import_result["target_branch"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("main");
    let atoms = import_result["atoms"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let atom_ids = import_result["atom_ids"].clone();
    let inject_live = body.get("inject_live").and_then(Value::as_bool) != Some(false);

    for sid in &shared_sids {
        let Ok(session) = store.get(sid) else {
            continue;
        };
        let sess_branch = session["omnigraph"]["branch"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "auto")
            .map(str::to_owned)
            .unwrap_or_else(|| crate::omnigraph::session_branch_name(sid));
        let mut updated_omni = merge_omnigraph_context(
            &session["omnigraph"],
            &atoms,
            &sess_branch,
            None,
            Some(target_branch),
        );
        if target_branch != sess_branch {
            append_unique_str_list(&mut updated_omni["pull_branches"], &[target_branch], 32);
        }
        let Ok(patched) = store.patch(sid, &json!({ "omnigraph": updated_omni })) else {
            continue;
        };
        let status = session["status"].as_str().unwrap_or("");
        let _ = store.event(
            sid,
            &json!({
                "kind": "omnigraph",
                "phase": "import",
                "ok": true,
                "target_branch": target_branch,
                "atom_ids": atom_ids,
                "imported_atoms": atoms.len(),
                "session_status": status,
                "text": format!(
                    "Imported {} Omnigraph context atom(s) onto `{target_branch}` into session `{sid}`.",
                    atoms.len()
                ),
            }),
        );
        if inject_live {
            let notice = format!(
                "Omnigraph context update: {} imported atom(s) added on branch `{target_branch}` and injected into this session.",
                atoms.len()
            );
            synced.inbox_notifications.push(
                notify_live_native_session_inbox(
                    store,
                    sessions_domain,
                    &session,
                    "omni-import",
                    &notice,
                )
                .await,
            );
        }
        synced.sessions.push(patched);
    }
    synced
}

/// Update `session["omnigraph"]` (`shared_space`, `linked_session_ids`, `pull_branches`, `injected_atoms`,
/// `capsule_text`), emit events, and notify active native sessions after `omnigraph.link_sessions`.
pub(crate) async fn sync_omnigraph_link_to_sessions(
    store: &SessionStore,
    sessions_domain: Option<&dyn Domain>,
    body: &Value,
    link_result: &Value,
) -> SessionContextSync {
    let mut synced = SessionContextSync {
        sessions: Vec::new(),
        inbox_notifications: Vec::new(),
    };
    if link_result["ok"] != true {
        return synced;
    }
    let linked_sids: Vec<String> = link_result["linked_session_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if linked_sids.is_empty() {
        return synced;
    }
    let mode = link_result["mode"]
        .as_str()
        .or_else(|| body["mode"].as_str())
        .unwrap_or("shared_branch");
    let shared_branch = link_result["shared_branch"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("main");
    let shared_space = link_result["shared_space"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(shared_branch);
    let source_sid = link_result["source_session_id"]
        .as_str()
        .or_else(|| body["source_session_id"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let atoms = link_result["atoms"].as_array().cloned().unwrap_or_default();
    let synced_count = link_result["synced_atoms"]
        .as_u64()
        .unwrap_or(atoms.len() as u64);
    let inject_live = body.get("inject_live").and_then(Value::as_bool) != Some(false);

    let mut session_branches = std::collections::BTreeMap::new();
    for sid in &linked_sids {
        if let Ok(sess) = store.get(sid) {
            let br = sess["omnigraph"]["branch"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty() && *s != "auto")
                .map(str::to_owned)
                .unwrap_or_else(|| crate::omnigraph::session_branch_name(sid));
            session_branches.insert(sid.clone(), br);
        }
    }

    for sid in &linked_sids {
        let Ok(session) = store.get(sid) else {
            continue;
        };
        let sess_branch = session_branches
            .get(sid)
            .cloned()
            .unwrap_or_else(|| crate::omnigraph::session_branch_name(sid));
        let peer_ids: Vec<String> = linked_sids
            .iter()
            .filter(|id| id.as_str() != sid.as_str())
            .cloned()
            .collect();
        let peer_refs: Vec<&str> = peer_ids.iter().map(String::as_str).collect();

        let is_broadcast_source = mode == "broadcast" && source_sid == Some(sid.as_str());
        let mut updated_omni = if is_broadcast_source {
            if session["omnigraph"].is_object() {
                session["omnigraph"].clone()
            } else {
                default_omnigraph_object(&sess_branch)
            }
        } else {
            merge_omnigraph_context(
                &session["omnigraph"],
                &atoms,
                &sess_branch,
                None,
                Some(shared_branch),
            )
        };
        updated_omni["enabled"] = json!(true);
        updated_omni["shared_space"] = json!(shared_space);
        updated_omni["last_error"] = Value::Null;
        if !updated_omni["linked_session_ids"].is_array() {
            updated_omni["linked_session_ids"] = json!([]);
        }
        append_unique_str_list(&mut updated_omni["linked_session_ids"], &peer_refs, 32);

        if !is_broadcast_source {
            let mut pull_adds: Vec<String> = Vec::new();
            if shared_branch != sess_branch {
                pull_adds.push(shared_branch.to_string());
            }
            if mode == "shared_branch" {
                for pid in &peer_ids {
                    if let Some(pbr) = session_branches.get(pid)
                        && pbr != &sess_branch
                    {
                        pull_adds.push(pbr.clone());
                    }
                }
            } else if let Some(src_id) = source_sid
                && let Some(sbr) = session_branches.get(src_id)
                && sbr != &sess_branch
            {
                pull_adds.push(sbr.clone());
            }
            let pull_refs: Vec<&str> = pull_adds.iter().map(String::as_str).collect();
            append_unique_str_list(&mut updated_omni["pull_branches"], &pull_refs, 32);
        }

        let Ok(patched) = store.patch(sid, &json!({ "omnigraph": updated_omni })) else {
            continue;
        };
        let status = session["status"].as_str().unwrap_or("");
        let _ = store.event(
            sid,
            &json!({
                "kind": "omnigraph",
                "phase": "link",
                "ok": true,
                "mode": mode,
                "shared_branch": shared_branch,
                "shared_space": shared_space,
                "linked_session_ids": peer_ids,
                "synced_atoms": synced_count,
                "session_status": status,
                "text": format!(
                    "Linked {} session(s) via `{mode}` on `{shared_branch}` ({synced_count} context atom(s)).",
                    linked_sids.len()
                ),
            }),
        );
        if inject_live && !is_broadcast_source {
            let notice = format!(
                "Shared Omnigraph context update ({mode}): {synced_count} atom(s) synced on `{shared_branch}` across linked tasks."
            );
            synced.inbox_notifications.push(
                notify_live_native_session_inbox(
                    store,
                    sessions_domain,
                    &session,
                    "omni-link",
                    &notice,
                )
                .await,
            );
        }
        synced.sessions.push(patched);
    }
    synced
}

struct CancelRelay(tokio::task::JoinHandle<()>);
impl Drop for CancelRelay {
    fn drop(&mut self) {
        self.0.abort();
    }
}
struct NativeRecovery {
    adapter: Arc<dyn provider::LaunchAdapter>,
    human_cancel: Arc<AtomicBool>,
}
impl crate::lifecycle::RecoveryRunner for NativeRecovery {
    fn run<'a>(
        &'a mut self,
        request: crate::lifecycle::RecoveryRequest,
        events: Arc<dyn provider::RunEvents>,
        _emit: Event,
    ) -> crate::lifecycle::RecoveryReply<'a> {
        Box::pin(async move {
            let deadline = std::time::Instant::now() + request.timeout;
            let human = self.human_cancel.clone();
            let cancellation = request.cancel.clone();
            let forwarding = CancelRelay(tokio::spawn(async move {
                loop {
                    if human.load(Ordering::Acquire) {
                        cancellation.store(true, Ordering::Release);
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                }
            }));
            // The observer owns recovery usage/history; duplicate display emission is suppressed.
            let result = provider::run_monitored_until(
                &request.launch,
                Limits {
                    seconds: None,
                    output: request.output_limit,
                    line: request.output_limit,
                },
                request.cancel,
                request.previous_total,
                Some(request.runner_lock),
                Some(self.adapter.as_ref()),
                &request.extra_env,
                Some(events),
                Some(deadline),
                |_| Ok(()),
            )
            .await;
            drop(forwarding);
            result
        })
    }
}

#[cfg(test)]
pub(crate) mod generation_completion_tests {
    use super::*;
    use crate::results::{Results, ResultsBackend, TargetCheckWorkflow, TargetReply};
    use infra_core::RuntimeContext;

    struct NoNativeBackend {
        operation: Arc<tokio::sync::Mutex<()>>,
        stop: Arc<AtomicBool>,
    }
    impl ResultsBackend for NoNativeBackend {
        fn workspace(&self, _: &Value) -> Result<PathBuf> {
            panic!("test must not inspect a native workspace")
        }
        fn idle(&self) -> Result<()> {
            panic!("test must not admit native work")
        }
        fn enqueue_check(&self, _: &str, _: &str, _: &str) -> Result<String> {
            panic!("test drives the actual private Job channel")
        }
        fn operation_lock(&self) -> Arc<tokio::sync::Mutex<()>> {
            self.operation.clone()
        }
        fn history_session_ids(&self, _: &Value) -> Result<Vec<String>> {
            panic!("test must not inspect native history")
        }
        fn shutdown_cancel(&self) -> Arc<AtomicBool> {
            self.stop.clone()
        }
        fn result_context(&self, _: &Value) -> Result<RuntimeContext> {
            panic!("test must not open native memory")
        }
    }
    struct NoBlackboard;
    impl BlackboardFactory for NoBlackboard {
        fn for_session(&self, _: &Value) -> Result<Arc<dyn fleet::Blackboard>> {
            panic!("test must not open a provider or blackboard")
        }
    }
    pub(crate) struct Fixture {
        pub(super) sessions: Arc<Sessions>,
        pub(super) results: Arc<Results>,
        pub(super) queue: tokio::sync::mpsc::Receiver<session_scheduler::Waiting<Job>>,
        pub(super) temporary: tempfile::TempDir,
    }
    impl Fixture {
        pub(crate) fn new() -> Self {
            let temporary = tempfile::tempdir().unwrap();
            let project = temporary.path().join("project");
            let source = temporary.path().join("source");
            std::fs::create_dir(&project).unwrap();
            std::fs::create_dir(&source).unwrap();
            let store = SessionStore::open(&temporary.path().join("state")).unwrap();
            let knowledge = Arc::new(KnowledgeDomain::new(store.clone(), source.clone()));
            let operation = Arc::new(tokio::sync::Mutex::new(()));
            let stopping = Arc::new(AtomicBool::new(false));
            let results = Arc::new(
                Results::new(
                    store.clone(),
                    Arc::new(NoNativeBackend {
                        operation: operation.clone(),
                        stop: stopping.clone(),
                    }),
                )
                .unwrap(),
            );
            let (queue, receiver) = session_scheduler::channel();
            let sessions = Arc::new(Sessions {
                store: store.clone(),
                source: source.clone(),
                providers: Arc::new(ProviderDomain {
                    providers: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
                }),
                knowledge: knowledge.clone(),
                blackboards: Arc::new(NoBlackboard),
                special: BTreeMap::new(),
                operation,
                checks: Mutex::new(Some(results.clone())),
                queue,
                active: Mutex::new(BTreeMap::new()),
                held: Mutex::new(None),
                admission: Mutex::new(()),
                stopping,
                queue_wake: Arc::new(tokio::sync::Notify::new()),
                queue_done: Arc::new(AtomicBool::new(true)),
                inbox_wake: Arc::new(tokio::sync::Notify::new()),
                inbox_done: Arc::new(AtomicBool::new(true)),
                ready: AtomicBool::new(true),
                accelerators: Mutex::new(None),
                commands: Mutex::new(None),
                results: Mutex::new(Some(results.clone())),
                graphs: Mutex::new(None),
                omnigraph: Mutex::new(None),
                skillcache: Mutex::new(None),
                protected: Mutex::new(None),
                lifecycle: Arc::new(crate::lifecycle::RunnerLifecycle::new(
                    store, knowledge, source,
                )),
                permissions: Arc::default(),
                default_seconds: 900,
            });
            Self {
                sessions,
                results,
                queue: receiver,
                temporary,
            }
        }
        pub(crate) fn delivery(&self) -> crate::delivery::Delivery {
            crate::delivery::Delivery::new(self.sessions.store.clone(), self.results.clone())
                .unwrap()
        }
        fn session(&self, status: &str) -> String {
            let project = self
                .sessions
                .store
                .register(&self.temporary.path().join("project"))
                .unwrap();
            let row = self
                .sessions
                .store
                .create(
                    &json!({
                        "project_id":project["id"],"provider":"claude","mode":"plan",
                        "workflow":"native","project_context":false
                    }),
                    "isolated queued task",
                    &[],
                )
                .unwrap();
            let sid = row["id"].as_str().unwrap().to_owned();
            if status != "queued" {
                self.sessions
                    .store
                    .patch(&sid, &json!({"status":status}))
                    .unwrap();
            }
            sid
        }
        fn job(&self, sid: &str, kind: JobKind) -> Job {
            Job {
                sid: sid.into(),
                prompt: "isolated queued task".into(),
                generation: self.sessions.store.generation(sid).unwrap(),
                kind,
            }
        }
        fn queued_check(&self, sid: &str, id: &str) {
            self.results
                .save_check(&json!({
                    "id":id,"session_id":sid,"status":"queued","kind":"target",
                    "command":"fixture-only","previous_status":"completed",
                    "output":"","started_at":null,"finished_at":null
                }))
                .unwrap();
        }
    }

    #[tokio::test]
    async fn queued_run_cancel_stays_cancelled_after_dequeue_without_native_work() {
        let mut fixture = Fixture::new();
        let sid = fixture.session("queued");
        let job = fixture.job(&sid, JobKind::Run);
        assert!(fixture.sessions.queue.try_send(job).is_ok());
        assert_eq!(
            fixture.sessions.cancel(&sid).unwrap()["status"],
            "cancelled"
        );
        let job = fixture.queue.recv().await.unwrap().into_job();
        fixture.sessions.execute_job(job, None).await;
        assert_eq!(
            fixture.sessions.store.get(&sid).unwrap()["status"],
            "cancelled"
        );
        let events = fixture.sessions.store.events(&sid, 0).unwrap();
        assert!(!events.iter().any(|event| event["kind"] == "error"));
        assert!(fixture.sessions.active.lock().unwrap().is_empty());
        assert!(!fixture.sessions.stopping.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn decide_permission_validates_optional_answers_object_and_size() {
        let fixture = Fixture::new();
        let sid = fixture.session("running");
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let approvals = fixture.sessions.permissions.approvals(
            &sid,
            Arc::new(move |event| {
                sink.lock().unwrap().push(event);
                Ok(())
            }),
        );
        let ask = infra_host::permission::PermissionAsk {
            id: "q-1".into(),
            provider: "claude".into(),
            tool: "AskUserQuestion".into(),
            summary: "Pick".into(),
            kind: "question".into(),
            ..Default::default()
        };
        let rx = approvals.ask_answer(&ask).unwrap();
        let req_id = events.lock().unwrap()[0]["request_id"]
            .as_str()
            .unwrap()
            .to_owned();

        // Non-object, oversized, and extra keys are rejected with 400.
        assert_eq!(
            fixture
                .sessions
                .decide_permission(&sid, &req_id, &json!({"decision":"allow","answers":"text"}))
                .unwrap_err()
                .status,
            400
        );
        assert_eq!(
            fixture
                .sessions
                .decide_permission(
                    &sid,
                    &req_id,
                    &json!({"decision":"allow","answers":{"k":"x".repeat(40_000)}})
                )
                .unwrap_err()
                .status,
            400
        );
        assert_eq!(
            fixture
                .sessions
                .decide_permission(
                    &sid,
                    &req_id,
                    &json!({"decision":"allow","answers":{},"extra":1})
                )
                .unwrap_err()
                .status,
            400
        );

        // Null answers behaves identically to omitted answers (backwards-compatible).
        let rx_null = approvals
            .ask_answer(&infra_host::permission::PermissionAsk {
                id: "q-null".into(),
                ..ask.clone()
            })
            .unwrap();
        let req_null = events.lock().unwrap()[1]["request_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let ok_null = fixture
            .sessions
            .decide_permission(&sid, &req_null, &json!({"decision":"allow","answers":null}))
            .unwrap();
        assert_eq!(ok_null["state"], "allowed");
        assert_eq!(
            rx_null.await.unwrap(),
            infra_host::permission::Answer::from_decision(infra_host::permission::Decision::Allow)
        );

        // Object answers flows through to Answer.
        let ok_obj = fixture
            .sessions
            .decide_permission(
                &sid,
                &req_id,
                &json!({"decision":"allow","answers":{"q1":"PostgreSQL"}}),
            )
            .unwrap();
        assert_eq!(ok_obj["state"], "allowed");
        assert_eq!(
            rx.await.unwrap(),
            infra_host::permission::Answer {
                decision: infra_host::permission::Decision::Allow,
                answers: Some(json!({"q1": "PostgreSQL"})),
            }
        );
    }

    pub(super) fn git_fixture(fixture: &Fixture) -> (String, Job, Lease) {
        let root = fixture.temporary.path().join("project");
        assert_eq!(
            projects::git(&root, &["init".into(), "--quiet".into()], 5)
                .unwrap()
                .code,
            0
        );
        let sid = fixture.session("queued");
        let common = infra_core::git::GitFrame::discover(&root)
            .unwrap()
            .unwrap()
            .common_dir()
            .unwrap();
        fixture
            .sessions
            .store
            .patch(&sid, &json!({"git_common_dir":common}))
            .unwrap();
        let job = fixture.job(&sid, JobKind::Run);
        let lease = fixture.sessions.repository_lease(&job).unwrap();
        assert!(lease.parallel());
        (sid, job, lease)
    }
    #[test]
    fn parallel_eligibility_requires_plain_native_work_and_fresh_git_identity() {
        let fixture = Fixture::new();
        let sid = fixture.session("queued");
        let unknown = fixture.job(&sid, JobKind::Run);
        assert!(fixture.sessions.repository_lease(&unknown).is_err());
        let (sid, job, _) = git_fixture(&fixture);
        for (field, value, reset) in [
            ("agents_enabled", json!(true), json!(false)),
            ("brain", json!({"approved":true}), Value::Null),
            ("sdd", json!({}), Value::Null),
            ("fleet", json!({}), Value::Null),
            ("clash", json!({}), Value::Null),
            ("creator", json!({}), Value::Null),
            ("system_run", json!({}), Value::Null),
            ("system_discovery", json!({}), Value::Null),
        ] {
            fixture
                .sessions
                .store
                .patch(&sid, &json!({field:value}))
                .unwrap();
            assert!(
                !fixture.sessions.repository_lease(&job).unwrap().parallel(),
                "{field}"
            );
            fixture
                .sessions
                .store
                .patch(&sid, &json!({field:reset}))
                .unwrap();
        }
        let check = fixture.job(
            &sid,
            JobKind::Check {
                check_id: "check".into(),
                previous_status: "completed".into(),
            },
        );
        assert!(
            !fixture
                .sessions
                .repository_lease(&check)
                .unwrap()
                .parallel()
        );
        fixture
            .sessions
            .store
            .connection()
            .unwrap()
            .execute("UPDATE sessions SET workflow='review' WHERE id=?1", [&sid])
            .unwrap();
        assert!(!fixture.sessions.repository_lease(&job).unwrap().parallel());
        fixture
            .sessions
            .store
            .connection()
            .unwrap()
            .execute("UPDATE sessions SET workflow='native' WHERE id=?1", [&sid])
            .unwrap();
        fixture
            .sessions
            .store
            .patch(
                &sid,
                &json!({"git_common_dir":fixture.temporary.path().join("source")}),
            )
            .unwrap();
        assert!(fixture.sessions.repository_lease(&job).is_err());
    }
    #[tokio::test]
    async fn changed_queued_identity_fails_before_native_work_and_completed_duplicate_is_ignored() {
        let fixture = Fixture::new();
        let (sid, job, lease) = git_fixture(&fixture);
        fixture
            .sessions
            .store
            .patch(&sid, &json!({"git_common_dir":null}))
            .unwrap();
        fixture.sessions.execute_job(job, Some(lease)).await;
        assert_eq!(
            fixture.sessions.store.get(&sid).unwrap()["status"],
            "failed"
        );
        assert!(fixture.sessions.active.lock().unwrap().is_empty());
        let completed = fixture.session("completed");
        fixture
            .sessions
            .execute_job(fixture.job(&completed, JobKind::Run), None)
            .await;
        assert_eq!(
            fixture.sessions.store.get(&completed).unwrap()["status"],
            "completed"
        );
    }
    #[tokio::test]
    async fn replaced_git_metadata_at_the_same_path_invalidates_the_queued_lease() {
        let fixture = Fixture::new();
        let (sid, job, lease) = git_fixture(&fixture);
        let root = fixture.temporary.path().join("project");
        std::fs::rename(root.join(".git"), root.join("previous-git")).unwrap();
        assert_eq!(
            projects::git(&root, &["init".into(), "--quiet".into()], 5)
                .unwrap()
                .code,
            0
        );
        assert_ne!(fixture.sessions.repository_lease(&job).unwrap(), lease);
        fixture.sessions.execute_job(job, Some(lease)).await;
        assert_eq!(
            fixture.sessions.store.get(&sid).unwrap()["status"],
            "failed"
        );
        assert!(fixture.sessions.active.lock().unwrap().is_empty());
    }
    #[test]
    fn actual_linked_worktrees_and_nested_repositories_conflict() {
        let fixture = Fixture::new();
        let (_, _, main) = git_fixture(&fixture);
        let root = fixture.temporary.path().join("project");
        let hooks = fixture.temporary.path().join("empty-hooks");
        std::fs::create_dir(&hooks).unwrap();
        let config = format!("core.hooksPath={}", hooks.display());
        let commit = [
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            &config,
            "commit",
            "--allow-empty",
            "--quiet",
            "-m",
            "fixture",
        ];
        assert_eq!(
            projects::git(&root, &commit.map(String::from), 5)
                .unwrap()
                .code,
            0
        );
        let linked = fixture.temporary.path().join("linked");
        let add = vec![
            "-c".into(),
            config,
            "worktree".into(),
            "add".into(),
            "--detach".into(),
            linked.to_string_lossy().into_owned(),
        ];
        assert_eq!(projects::git(&root, &add, 5).unwrap().code, 0);
        for directory in [linked, root.join("nested")] {
            if !directory.exists() {
                std::fs::create_dir(&directory).unwrap();
                assert_eq!(
                    projects::git(&directory, &["init".into(), "--quiet".into()], 5)
                        .unwrap()
                        .code,
                    0
                );
            }
            let registered = fixture.sessions.store.register(&directory).unwrap();
            let row = fixture.sessions.store.create(&json!({"project_id":registered["id"],"provider":"claude","workflow":"native","mode":"plan","agents_enabled":false}), "fixture", &[]).unwrap();
            let sid = row["id"].as_str().unwrap();
            let common = infra_core::git::GitFrame::discover(&directory)
                .unwrap()
                .unwrap()
                .common_dir()
                .unwrap();
            fixture
                .sessions
                .store
                .patch(sid, &json!({"git_common_dir":common}))
                .unwrap();
            let lease = fixture
                .sessions
                .repository_lease(&fixture.job(sid, JobKind::Run))
                .unwrap();
            assert!(lease.parallel());
            assert!(main.conflicts(&lease));
        }
    }
    #[tokio::test]
    async fn close_before_dispatch_and_old_token_removal_cannot_lose_new_ownership() {
        let fixture = Fixture::new();
        let sid = fixture.session("queued");
        let old = Arc::new(AtomicBool::new(false));
        let replacement = Arc::new(AtomicBool::new(false));
        fixture
            .sessions
            .active
            .lock()
            .unwrap()
            .insert(sid.clone(), replacement.clone());
        fixture.sessions.remove_active(&sid, &old);
        assert!(Arc::ptr_eq(
            fixture.sessions.active.lock().unwrap().get(&sid).unwrap(),
            &replacement
        ));
        fixture.sessions.close().unwrap();
        assert!(replacement.load(Ordering::Acquire));
        fixture
            .sessions
            .execute_job(fixture.job(&sid, JobKind::Run), None)
            .await;
        assert_eq!(
            fixture.sessions.store.get(&sid).unwrap()["status"],
            "queued"
        );
        fixture.sessions.remove_active(&sid, &replacement);
        fixture.sessions.wait_closed().await.unwrap();
    }
    #[test]
    fn fatal_cleanup_stops_admission_and_cancels_other_active_work() {
        let fixture = Fixture::new();
        let failed = Arc::new(AtomicBool::new(false));
        let sibling = Arc::new(AtomicBool::new(false));
        fixture
            .sessions
            .active
            .lock()
            .unwrap()
            .insert("failed".into(), failed.clone());
        fixture
            .sessions
            .active
            .lock()
            .unwrap()
            .insert("sibling".into(), sibling.clone());
        fixture.sessions.stop_failed(&failed);
        assert!(!failed.load(Ordering::Acquire));
        assert!(sibling.load(Ordering::Acquire));
        assert!(fixture.sessions.admit().is_err());
    }

    #[tokio::test]
    async fn queued_check_cancel_settles_its_receipt_without_restoring_completed() {
        let mut fixture = Fixture::new();
        let sid = fixture.session("queued");
        let check = store::id();
        fixture.queued_check(&sid, &check);
        assert!(
            fixture
                .sessions
                .queue
                .try_send(fixture.job(
                    &sid,
                    JobKind::Check {
                        check_id: check.clone(),
                        previous_status: "completed".into(),
                    }
                ))
                .is_ok()
        );
        fixture.sessions.cancel(&sid).unwrap();
        let job = fixture.queue.recv().await.unwrap().into_job();
        fixture.sessions.execute_job(job, None).await;
        assert_eq!(
            fixture.sessions.store.get(&sid).unwrap()["status"],
            "cancelled"
        );
        let receipt = fixture.results.check(&check, Some(&sid)).unwrap().unwrap();
        assert_eq!(receipt["status"], "cancelled");
        assert!(receipt["finished_at"].is_string());
        assert!(
            !fixture
                .sessions
                .store
                .events(&sid, 0)
                .unwrap()
                .iter()
                .any(|event| event["text"] == "Check finished.")
        );
    }

    #[test]
    fn cached_old_completion_cannot_replace_a_committed_followup_or_its_history() {
        let fixture = Fixture::new();
        let sid = fixture.session("running");
        let job = fixture.job(&sid, JobKind::Run);
        fixture
            .results
            .start_launch(&sid, &job.generation, None)
            .unwrap();
        let old_generation = job.generation.clone();
        let (ready, entered) = std::sync::mpsc::channel();
        let (resume, release) = std::sync::mpsc::channel();
        let store = fixture.sessions.store.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        fixture
            .sessions
            .active
            .lock()
            .unwrap()
            .insert(sid.clone(), cancel.clone());
        let finishing = std::thread::spawn(move || {
            ready.send(()).unwrap();
            release.recv().unwrap();
            finalize_job(
                &store,
                &job,
                &Ok(json!({"kind":"result","ok":true})),
                &cancel,
            )
            .unwrap()
        });
        entered.recv().unwrap();
        fixture.sessions.cancel(&sid).unwrap();
        fixture
            .sessions
            .store
            .requeue(&sid, Some("accepted replacement"), &json!({}))
            .unwrap();
        let replacement = fixture.sessions.store.generation(&sid).unwrap();
        resume.send(()).unwrap();
        let old_status = finishing.join().unwrap();
        assert_eq!(old_status, "cancelled");
        fixture.sessions.active.lock().unwrap().remove(&sid);
        fixture
            .results
            .finish_launch(&old_generation, &old_status)
            .unwrap();
        assert_ne!(replacement, old_generation);
        assert_eq!(
            fixture.sessions.store.get(&sid).unwrap()["status"],
            "queued"
        );
        let db = fixture.sessions.store.connection().unwrap();
        let status: String = db
            .query_row(
                "SELECT status FROM launches WHERE id=?1",
                [old_generation],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "cancelled");
        drop(db);
        assert!(
            !fixture
                .sessions
                .store
                .events(&sid, 0)
                .unwrap()
                .iter()
                .any(|event| event["kind"] == "error")
        );
    }

    #[test]
    fn terminal_fields_and_generation_events_roll_back_as_one_transaction() {
        let fixture = Fixture::new();
        let sid = fixture.session("running");
        let generation = fixture.sessions.store.generation(&sid).unwrap();
        let before = fixture.sessions.store.events(&sid, 0).unwrap().len();
        let result = fixture.sessions.store.finish_generation(
            &sid,
            &generation,
            &json!({"status":"completed","fleet_result":{"report":"owned"}}),
            &[
                json!({"kind":"status","text":"done"}),
                json!({"kind":"error","launch_id":"different-generation"}),
            ],
            &AtomicBool::new(false),
        );
        assert!(result.is_err());
        let row = fixture.sessions.store.get(&sid).unwrap();
        assert_eq!(row["status"], "running");
        assert!(row["fleet_result"].is_null());
        assert_eq!(
            fixture.sessions.store.events(&sid, 0).unwrap().len(),
            before
        );
    }

    #[test]
    fn cancellation_after_terminal_commit_does_not_rewrite_completion() {
        let fixture = Fixture::new();
        let sid = fixture.session("running");
        let job = fixture.job(&sid, JobKind::Run);
        finalize_job(
            &fixture.sessions.store,
            &job,
            &Ok(json!({"ok":true})),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            fixture.sessions.cancel(&sid).unwrap()["status"],
            "completed"
        );
        assert!(
            !fixture
                .sessions
                .store
                .events(&sid, 0)
                .unwrap()
                .iter()
                .any(|event| event["text"] == "Cancellation requested.")
        );
    }

    struct UnknownCleanup {
        entered: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        release: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
        failure: Mutex<Option<HarnessError>>,
    }
    impl TargetCheckWorkflow for UnknownCleanup {
        fn run_target<'a>(&'a self, _: &'a mut Value, cancel: Arc<AtomicBool>) -> TargetReply<'a> {
            Box::pin(async move {
                self.entered
                    .lock()
                    .unwrap()
                    .take()
                    .unwrap()
                    .send(())
                    .unwrap();
                let release = self.release.lock().unwrap().take().unwrap();
                release.await.unwrap();
                assert!(cancel.load(Ordering::Acquire));
                Err(self.failure.lock().unwrap().take().unwrap())
            })
        }
    }

    #[tokio::test]
    async fn cancelled_stale_check_cleanup_failure_still_stops_global_admission() {
        assert_cleanup_failure_stops(HarnessError {
            status: 503,
            code: "cleanup-unconfirmed",
            message: "Fixture reports unknown cleanup; no OS process was created.".into(),
        })
        .await;
    }

    pub(crate) async fn assert_cleanup_failure_stops(failure: HarnessError) {
        assert_eq!(failure.code, "cleanup-unconfirmed");
        let message = failure.message.clone();
        let mut fixture = Fixture::new();
        let sid = fixture.session("queued");
        let check = store::id();
        fixture.queued_check(&sid, &check);
        let old_generation = fixture.sessions.store.generation(&sid).unwrap();
        let (entered, ready) = tokio::sync::oneshot::channel();
        let (release, resume) = tokio::sync::oneshot::channel();
        let target: Arc<dyn TargetCheckWorkflow> = Arc::new(UnknownCleanup {
            entered: Mutex::new(Some(entered)),
            release: Mutex::new(Some(resume)),
            failure: Mutex::new(Some(failure)),
        });
        fixture
            .results
            .bind_target(Arc::downgrade(&target))
            .unwrap();
        assert!(
            fixture
                .sessions
                .queue
                .try_send(fixture.job(
                    &sid,
                    JobKind::Check {
                        check_id: check.clone(),
                        previous_status: "completed".into(),
                    }
                ))
                .is_ok()
        );
        let job = fixture.queue.recv().await.unwrap().into_job();
        let runner = fixture.sessions.clone();
        let executing = tokio::spawn(async move { runner.execute_job(job, None).await });
        ready.await.unwrap();
        fixture.sessions.cancel(&sid).unwrap();
        fixture
            .sessions
            .store
            .requeue(&sid, Some("accepted replacement"), &json!({}))
            .unwrap();
        let replacement = fixture.sessions.store.generation(&sid).unwrap();
        assert_ne!(replacement, old_generation);
        release.send(()).unwrap();
        executing.await.unwrap();
        assert!(fixture.sessions.stopping.load(Ordering::Acquire));
        assert_eq!(
            fixture.sessions.store.get(&sid).unwrap()["status"],
            "queued"
        );
        assert_eq!(
            fixture.sessions.store.generation(&sid).unwrap(),
            replacement
        );
        assert!(fixture.sessions.admit().is_err());
        let receipt = fixture.results.check(&check, Some(&sid)).unwrap().unwrap();
        assert_eq!(receipt["status"], "failed");
        assert!(
            receipt["output"]
                .as_str()
                .unwrap()
                .contains(message.as_str())
        );
        assert!(
            !fixture
                .sessions
                .store
                .events(&sid, 0)
                .unwrap()
                .iter()
                .any(|event| { event["launch_id"] == old_generation && event["kind"] == "error" })
        );
    }
}
#[cfg(test)]
mod followup_budget_contract_tests {
    use super::*;
    fn catalog() -> Value {
        json!({"models":[{"id":"planner","efforts":["high"]},{"id":"editor","efforts":["max"]}],"efforts":["high","max"]})
    }
    fn routing() -> Value {
        json!({"plan":{"model":"planner","thinking_effort":"high"},"edit":{"model":"editor","thinking_effort":"max"}})
    }
    fn ordinary() -> Value {
        json!({"workflow":"native","provider":"claude","model":"planner","thinking_effort":"high","mode":"plan","agents_enabled":false,"agent_count":3,"model_routing":null,"sdd":null})
    }
    #[test]
    fn sdd_followups_change_permission_mode_without_enabling_model_routing() {
        let mut session = ordinary();
        session["workflow"] = json!("sdd");
        session["sdd"] = json!({"feature":"billing","phase":"review"});
        let implement = followup_settings(
            &session,
            &json!({"sdd":{"feature":"billing","phase":"implement"}}),
            &catalog(),
        )
        .unwrap();
        assert_eq!(implement["mode"], "edit");
        assert!(implement["model_routing"].is_null());
        session["mode"] = implement["mode"].clone();
        session["sdd"] = implement["sdd"].clone();
        let review = followup_settings(
            &session,
            &json!({"sdd":{"feature":"billing","phase":"review"}}),
            &catalog(),
        )
        .unwrap();
        assert_eq!(review["mode"], "plan");
        assert!(
            followup_settings(
                &session,
                &json!({"sdd":{"feature":"another","phase":"review"}}),
                &catalog()
            )
            .is_err()
        );
        assert!(
            followup_settings(
                &session,
                &json!({"sdd":{"feature":"billing","phase":"review"},"mode":"edit"}),
                &catalog()
            )
            .is_err()
        );
    }
    #[test]
    fn a_native_mode_change_uses_the_routing_requested_for_this_turn() {
        let mut session = ordinary();
        let enabled = followup_settings(
            &session,
            &json!({"mode":"edit","model_routing":routing()}),
            &catalog(),
        )
        .unwrap();
        assert_eq!(enabled["model"], "editor");
        assert_eq!(enabled["thinking_effort"], "max");
        assert_eq!(enabled["mode"], "edit");
        session["model_routing"] = enabled["model_routing"].clone();
        assert!(
            followup_settings(
                &session,
                &json!({"mode":"edit","model_routing":null}),
                &catalog()
            )
            .is_err()
        );
        session["mode"] = json!("edit");
        assert!(
            followup_settings(
                &session,
                &json!({"mode":"edit","model_routing":null}),
                &catalog()
            )
            .is_ok()
        );
        assert!(followup_settings(&session, &json!({"mode":"unexpected"}), &catalog()).is_err());
        assert!(followup_settings(&session, &json!({"mode":false}), &catalog()).is_err());
    }
    fn budget_fixture(fleet: bool) -> (tempfile::TempDir, SessionStore, String) {
        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&directory.path().join("state")).unwrap();
        let project = store.register(directory.path()).unwrap();
        let sid = uuid::Uuid::new_v4().to_string();
        let options = json!({"provider":"claude","mode":"plan","workflow":if fleet{"fleet-review"}else{"native"},
            "agents_enabled":true,"agent_count":2,"model":"planner","thinking_effort":"high",
            "fleet":if fleet{json!({"lenses":["code-reviewer","security-reviewer"],"dry_run":false,"budget_usd":8.0,"worker_timeout":300})}else{Value::Null},
            "budgets":{"usd":8.0,"tokens":1000,"seconds":900}});
        store::insert_session(
            &store.connection().unwrap(),
            &options,
            &project,
            "Review",
            &[],
            &sid,
            &store::now(),
            &"0".repeat(32),
        )
        .unwrap();
        store
            .patch(
                &sid,
                &json!({"status":"completed","budget_usage":{"tokens":17,"usd":1.25}}),
            )
            .unwrap();
        store
            .connection()
            .unwrap()
            .execute(
                "UPDATE sessions SET updated_at='before' WHERE id=?1",
                [&sid],
            )
            .unwrap();
        (directory, store, sid)
    }
    #[test]
    fn fleet_budget_api_persists_next_launch_caps_timeout_revision_and_event() {
        let (_directory, store, sid) = budget_fixture(true);
        let saved=save_budget_settings(&store,&sid,&json!({"budgets":{"usd":4.0,"tokens":200,"seconds":70},"revision":0,"worker_timeout":19})).unwrap();
        assert_eq!(saved["fleet"]["worker_timeout"], 19);
        assert_eq!(saved["fleet"]["budget_usd"], 4.0);
        assert_eq!(
            saved["budgets"],
            json!({"usd":4.0,"tokens":200,"seconds":70})
        );
        assert_eq!(saved["budget_revision"], 1);
        assert_eq!(saved["budget_usage"], json!({"tokens":17,"usd":1.25}));
        assert_ne!(saved["updated_at"], "before");
        assert_eq!(saved["agent_budget_plan"]["seconds"], 19);
        assert_eq!(saved["agent_budget_plan"]["usd"], 2.0);
        let events = store.events(&sid, 0).unwrap();
        assert_eq!(events.last().unwrap()["kind"], "status");
        assert_eq!(
            events.last().unwrap()["text"],
            "Budgets saved for the next launch. Previous usage is retained."
        );
        let before = store.get(&sid).unwrap();
        let count = events.len();
        assert!(
            save_budget_settings(
                &store,
                &sid,
                &json!({"budgets":{"usd":3.0},"revision":0,"worker_timeout":11})
            )
            .is_err()
        );
        assert_eq!(store.get(&sid).unwrap(), before);
        assert_eq!(store.events(&sid, 0).unwrap().len(), count);
    }
    #[test]
    fn budget_api_refuses_running_specialized_and_non_fleet_timeout_updates() {
        let (_directory, store, sid) = budget_fixture(false);
        let request = json!({"budgets":{"tokens":200},"revision":0,"worker_timeout":19});
        let before = store.get(&sid).unwrap();
        assert!(save_budget_settings(&store, &sid, &request).is_err());
        assert_eq!(store.get(&sid).unwrap(), before);
        let request = json!({"budgets":{"tokens":200},"revision":0});
        for status in ["queued", "running"] {
            store.patch(&sid, &json!({"status":status})).unwrap();
            assert!(save_budget_settings(&store, &sid, &request).is_err());
        }
        store.patch(&sid, &json!({"status":"completed"})).unwrap();
        for field in ["creator", "system_run", "system_discovery"] {
            store.patch(&sid, &json!({(field):{"id":"owned"}})).unwrap();
            assert!(save_budget_settings(&store, &sid, &request).is_err());
            store.patch(&sid, &json!({(field):null})).unwrap();
        }
        assert_eq!(store.get(&sid).unwrap()["budget_revision"], 0);
        assert_eq!(store.events(&sid, 0).unwrap().len(), 1);
    }
    #[test]
    fn fleet_timeout_rejects_fractional_boolean_null_and_out_of_range_values_without_writes() {
        let (_directory, store, sid) = budget_fixture(true);
        let before = store.get(&sid).unwrap();
        for timeout in [
            json!(0),
            json!(3601),
            json!(1.5),
            json!(true),
            Value::Null,
            json!("10"),
        ] {
            assert!(
                save_budget_settings(
                    &store,
                    &sid,
                    &json!({"budgets":{"usd":4.0},"revision":0,"worker_timeout":timeout})
                )
                .is_err()
            );
            assert_eq!(store.get(&sid).unwrap(), before);
        }
        let saved =
            save_budget_settings(&store, &sid, &json!({"budgets":{"usd":4.0},"revision":0}))
                .unwrap();
        assert_eq!(saved["fleet"]["worker_timeout"], 300);
    }
}

#[cfg(test)]
mod graph_focus_tests {
    use super::generation_completion_tests::Fixture;
    use super::*;

    #[tokio::test]
    async fn run_job_emits_initial_graph_focus_before_provider_launch_and_completes_on_finish() {
        let fixture = Fixture::new();
        let project = fixture.temporary.path().join("project");
        std::fs::create_dir_all(project.join("src/Auth")).unwrap();
        std::fs::write(
            project.join("src/Auth/TokenService.php"),
            "<?php\nclass TokenService {\n    public function decode(): void {}\n}\n",
        )
        .unwrap();

        let store = fixture.sessions.store.clone();
        let registered = store.register(&project).unwrap();
        let pid = registered["id"].as_str().unwrap().to_owned();

        let graph_path = crate::graph::live(&store.state, &pid);
        std::fs::create_dir_all(graph_path.parent().unwrap()).unwrap();
        std::fs::write(
            &graph_path,
            json!({
                "nodes": [
                    {
                        "id": "token",
                        "label": "TokenService",
                        "source_file": "src/Auth/TokenService.php",
                        "node_kind": "class",
                        "source_location": "line 2"
                    }
                ],
                "links": []
            })
            .to_string(),
        )
        .unwrap();

        let graphs = crate::graph::Graphs::new(store.clone());
        fixture.sessions.install_graphs(graphs).unwrap();

        let created = store
            .create(
                &json!({
                    "project_id": pid,
                    "provider": "claude",
                    "mode": "edit",
                    "workflow": "text",
                    "project_context": false
                }),
                "Update TokenService validation logic",
                &[],
            )
            .unwrap();
        let sid = created["id"].as_str().unwrap().to_owned();
        store
            .patch(&sid, &json!({"result_base": {"is_git": false}}))
            .unwrap();
        let generation = store.generation(&sid).unwrap();

        let job = Job {
            sid: sid.clone(),
            generation,
            prompt: "Update TokenService validation logic".to_owned(),
            kind: JobKind::Run,
        };
        fixture.sessions.execute_job(job, None).await;

        let events = store.events(&sid, 0).unwrap();
        let focus_events: Vec<&Value> = events
            .iter()
            .filter(|e| e["kind"] == "graph_focus")
            .collect();
        assert!(
            !focus_events.is_empty(),
            "expected prompt-start graph_focus event before provider launch, events={events:?}"
        );
        assert_eq!(focus_events[0]["phase"], "predicting");
        assert!(
            focus_events[0]["target_folders"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "src/Auth")
        );
        assert!(
            focus_events[0]["target_paths"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "src/Auth/TokenService.php")
        );

        let stored_focus = crate::graph_live::session_focus_value(&store, &sid).unwrap();
        assert_eq!(stored_focus["phase"], "completed");
        assert!(
            stored_focus["target_paths"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "src/Auth/TokenService.php")
        );
    }

    #[test]
    fn live_focus_tracker_handles_tool_stream_and_persists_deltas() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src/Auth")).unwrap();
        let file = root.join("src/Auth/JwtGuard.php");
        std::fs::write(
            &file,
            "<?php\nclass JwtGuard {\n    public function oldCheck(): bool { return true; }\n}\n",
        )
        .unwrap();

        let mut tracker = crate::graph_live::LiveFocusTracker::new(root.to_path_buf());
        assert!(
            tracker
                .observe_tool_event(&json!({
                    "kind": "tool",
                    "state": "started",
                    "tool": "Edit",
                    "path": "src/Auth/JwtGuard.php"
                }))
                .is_some()
        );
        std::fs::write(
            &file,
            "<?php\nclass JwtGuard {\n    public function oldCheck(): bool { return true; }\n    public function verifyToken(): bool { return false; }\n}\n",
        )
        .unwrap();
        let delta = tracker
            .observe_tool_event(&json!({
                "kind": "tool",
                "state": "completed",
                "ok": true,
                "tool": "Edit",
                "path": "src/Auth/JwtGuard.php"
            }))
            .expect("delta");
        assert_eq!(delta["kind"], "graph_focus");
        assert_eq!(delta["phase"], "active");
        assert_eq!(delta["created_entities"].as_array().map(Vec::len), Some(1));
        assert_eq!(delta["created_entities"][0]["name"], "verifyToken");
        assert_eq!(delta["created_entities"][0]["parent"], "JwtGuard");
    }

    #[test]
    fn observe_tool_event_with_graphs_updates_native_ir_and_live_graph() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/service.rs"),
            "pub fn initial_fn() -> i32 {\n    1\n}\n",
        )
        .unwrap();

        let store = SessionStore::open(&state).unwrap();
        let pid = store.register(&root).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let graphs =
            crate::graph::Graphs::with_native_backend(store.clone(), std::time::Duration::ZERO);
        graphs.act(&pid, "enable").unwrap();
        let started = std::time::Instant::now();
        while graphs.status(&pid, false).unwrap()["state"] == "building" {
            assert!(started.elapsed() < std::time::Duration::from_secs(10));
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        assert_eq!(graphs.status(&pid, false).unwrap()["state"], "ready");

        let mut tracker = crate::graph_live::LiveFocusTracker::new(root.clone());
        let _ = observe_tool_event_with_graphs(
            &mut tracker,
            &json!({
                "kind": "tool",
                "state": "started",
                "tool": "Edit",
                "path": "src/service.rs"
            }),
            Some(&graphs),
            &pid,
            &root,
        );
        std::fs::write(
            root.join("src/service.rs"),
            "pub fn initial_fn() -> i32 {\n    1\n}\npub fn newly_added_fn() -> i32 {\n    2\n}\n",
        )
        .unwrap();
        let delta = observe_tool_event_with_graphs(
            &mut tracker,
            &json!({
                "kind": "tool",
                "state": "completed",
                "ok": true,
                "tool": "Edit",
                "path": "src/service.rs"
            }),
            Some(&graphs),
            &pid,
            &root,
        )
        .expect("live focus delta");
        assert_eq!(delta["created_entities"][0]["name"], "newly_added_fn");

        let found = graphs.find(&pid, None, "newly_added_fn", false).unwrap();
        assert_eq!(found["rows"][0]["file"], "src/service.rs", "{found}");
        graphs.close();
    }

    #[test]
    fn ingest_warm_with_live_focus_forwards_changes_to_graph_and_focus() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let root = dir.path().join("project");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/service.rs"),
            "pub fn initial_fn() -> i32 {\n    1\n}\n",
        )
        .unwrap();

        let store = SessionStore::open(&state).unwrap();
        let pid = store.register(&root).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let session = store
            .create(
                &json!({
                    "project_id": pid,
                    "provider": "claude",
                    "mode": "edit",
                    "workflow": "native",
                }),
                "Run compiled skill",
                &[],
            )
            .unwrap();
        let sid = session["id"].as_str().unwrap().to_owned();
        let graphs =
            crate::graph::Graphs::with_native_backend(store.clone(), std::time::Duration::ZERO);
        graphs.act(&pid, "enable").unwrap();
        let started = std::time::Instant::now();
        while graphs.status(&pid, false).unwrap()["state"] == "building" {
            assert!(started.elapsed() < std::time::Duration::from_secs(10));
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        assert_eq!(graphs.status(&pid, false).unwrap()["state"], "ready");

        std::fs::write(
            root.join("src/service.rs"),
            "pub fn initial_fn() -> i32 {\n    1\n}\npub fn warm_script_fn() -> i32 {\n    2\n}\n",
        )
        .unwrap();
        let warm_dir = dir.path().join("warm-state");
        std::fs::create_dir_all(&warm_dir).unwrap();
        let state_file = warm_dir.join("plan-state.json");
        let mut run_log = infra_skillcache::state::RunLog::open(&state_file, "g1").unwrap();
        run_log
            .append(
                "program_completed",
                json!({"step_index": 0, "program": "mutate"}),
            )
            .unwrap();
        infra_skillcache::state::append_changes(
            &infra_skillcache::state::log_path(&state_file),
            "g1",
            &[json!({"path": "src/service.rs", "kind": "edit"})],
        )
        .unwrap();

        let warm = crate::skillcache::warm::Warm::for_changes_test(warm_dir);
        let live_focus =
            std::sync::Mutex::new(crate::graph_live::LiveFocusTracker::new(root.clone()));
        let emitted = std::sync::Mutex::new(Vec::<Value>::new());
        ingest_warm_with_live_focus(
            &warm,
            &store,
            &sid,
            &pid,
            &root,
            Some(&graphs),
            &live_focus,
            &|event| {
                emitted.lock().unwrap().push(event);
                Ok(())
            },
        );
        let events = emitted.lock().unwrap();
        assert!(events.iter().any(|e| e["kind"] == "skillcache"));
        assert!(events.iter().any(|e| {
            e["kind"] == "graph_focus"
                && e["target_paths"]
                    .as_array()
                    .is_some_and(|arr| arr.iter().any(|p| p == "src/service.rs"))
        }));
        let found = graphs.find(&pid, None, "warm_script_fn", false).unwrap();
        assert_eq!(found["rows"][0]["file"], "src/service.rs", "{found}");
        graphs.close();
    }

    #[tokio::test]
    async fn saved_knowledge_syncs_the_canonical_branch_in_the_background() {
        assert!(saved_canonical_knowledge(
            &json!({"records": 2, "bank_promoted": false})
        ));
        assert!(saved_canonical_knowledge(
            &json!({"records": 0, "bank_promoted": true})
        ));
        assert!(!saved_canonical_knowledge(
            &json!({"records": 0, "bank_promoted": false})
        ));
        assert!(!saved_canonical_knowledge(&json!({"state": "none"})));

        let fixture = Fixture::new();
        let (base_url, _server) = crate::omnigraph::tests::spawn_mock_omnigraph().await;
        let omni_state = fixture.temporary.path().join("omni-state");
        std::fs::create_dir_all(&omni_state).unwrap();
        let manager = Arc::new(crate::omnigraph::OmnigraphManager::new(omni_state, None).unwrap());
        manager
            .setup_json(&json!({
                "action": "connect",
                "endpoint": base_url,
                "token": "tok-harness",
                "graph_id": crate::omnigraph::schema::HARNESS_GRAPH_ID
            }))
            .await
            .unwrap();
        fixture
            .results
            .install_omnigraph(Arc::clone(&manager))
            .unwrap();
        // A governed installation with a Brain and an empty bank.
        let proj = fixture.temporary.path().join("project");
        std::fs::create_dir_all(proj.join("memory-bank/chunks")).unwrap();
        for (name, text) in [
            ("INDEX.md", "# Index\n"),
            ("README.md", "# Memory Bank\n"),
            ("runtime-contract.json", "{}"),
        ] {
            std::fs::write(proj.join("memory-bank").join(name), text).unwrap();
        }
        std::fs::create_dir_all(proj.join("project-brain/config")).unwrap();
        std::fs::write(
            proj.join("project-brain/config/runtime.json"),
            r#"{"mode":"governed","allowed_privacy":["public","team"],"allowed_authority":["observed","verified"],"owners":["*"]}"#,
        )
        .unwrap();
        let schemas = proj.join("project-brain/schemas");
        std::fs::create_dir_all(&schemas).unwrap();
        for entry in std::fs::read_dir(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../PHP Core/project-brain/schemas"),
        )
        .unwrap()
        {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), schemas.join(entry.file_name())).unwrap();
        }
        let reg = fixture.sessions.store.register(&proj).unwrap();
        let project_id = reg["id"].as_str().unwrap().to_string();
        // An atom of a record the Brain no longer holds.
        manager
            .sync_canonical_knowledge(
                &project_id,
                &[json!({"atom_id": "brain-gone", "kind": "finding", "title": "Gone", "origin_layer": "project-brain", "authority": "verified", "privacy": "public"})],
            )
            .await
            .unwrap();
        let sess = fixture
            .sessions
            .store
            .create(
                &json!({"project_id": reg["id"], "provider": "claude", "workflow": "native", "mode": "plan", "agents_enabled": false}),
                "Test session",
                &[],
            )
            .unwrap();
        let sid = sess["id"].as_str().unwrap();
        // A session without Omnigraph starts nothing.
        assert!(
            fixture
                .sessions
                .spawn_canonical_knowledge_sync(sid)
                .is_none()
        );
        fixture
            .sessions
            .store
            .patch(sid, &json!({"omnigraph": {"enabled": true}}))
            .unwrap();
        let synced = fixture
            .sessions
            .spawn_canonical_knowledge_sync(sid)
            .expect("a background sync")
            .await
            .unwrap()
            .expect("the sync answered");
        assert_eq!(synced["ok"], true);
        assert_eq!(synced["branch"], "knowledge/canonical");
        // The Brain was read whole, so its gone record left the branch.
        assert_eq!(synced["deleted_atom_ids"], json!(["brain-gone"]));
    }

    #[tokio::test]
    async fn memory_save_does_not_publish_to_omnigraph_unless_ok_is_true() {
        let fixture = Fixture::new();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let hit = Arc::new(AtomicBool::new(false));
        let hit_bg = hit.clone();
        listener.set_nonblocking(true).unwrap();
        let _guard = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while std::time::Instant::now() < deadline {
                if listener.accept().is_ok() {
                    hit_bg.store(true, Ordering::Release);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        });

        let omni_state = fixture.temporary.path().join("omni-state");
        std::fs::create_dir_all(&omni_state).unwrap();
        let manager = Arc::new(crate::omnigraph::OmnigraphManager::new(omni_state, None).unwrap());
        let cfg = crate::omnigraph::OmnigraphConfig {
            mode: crate::omnigraph::OmnigraphMode::External,
            endpoint: Some(format!("http://{addr}")),
            ..Default::default()
        };
        manager.save_config(cfg).await.unwrap();
        fixture
            .results
            .install_omnigraph(Arc::clone(&manager))
            .unwrap();
        fixture
            .sessions
            .install_omnigraph(Arc::clone(&manager))
            .unwrap();

        let proj = fixture.temporary.path().join("project");
        let reg = fixture.sessions.store.register(&proj).unwrap();
        let sess = fixture
            .sessions
            .store
            .create(
                &json!({
                    "project_id": reg["id"],
                    "provider": "claude",
                    "workflow": "native",
                    "mode": "plan",
                    "agents_enabled": false,
                }),
                "Test session",
                &[],
            )
            .unwrap();
        let sid = sess["id"].as_str().unwrap();
        fixture
            .sessions
            .store
            .patch(
                sid,
                &json!({
                    "omnigraph": { "enabled": true, "auto_publish": true },
                }),
            )
            .unwrap();

        let save_res = fixture
            .sessions
            .execute(
                "memory.save",
                json!({
                    "params": { "session_id": sid },
                    "query": {},
                    "body": { "learnings": [] },
                }),
            )
            .await;
        assert!(match &save_res {
            Err(_) => true,
            Ok(Payload::Json(v)) => v["ok"] != true,
            Ok(_) => false,
        });
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        assert!(
            !hit.load(Ordering::Acquire),
            "memory.save must not publish to Omnigraph unless ok == true"
        );
    }
}

#[cfg(test)]
mod inbox_notify_tests {
    use super::*;

    struct Refusing;
    impl Domain for Refusing {
        fn execute<'a>(&'a self, operation: &'a str, _: Value) -> Reply<'a> {
            Box::pin(async move {
                assert_eq!(operation, "sessions.inbox_accept");
                Err(HarnessError::conflict("The inbox is full."))
            })
        }
    }

    #[tokio::test]
    async fn notify_live_native_session_inbox_surfaces_brain_skip_and_domain_errors() {
        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&directory.path().join("state")).unwrap();
        {
            let db = store.connection().unwrap();
            crate::inbox::initialize(&db).unwrap();
            for (id, status) in [
                ("live", "running"),
                ("idle", "completed"),
                ("linked", "running"),
            ] {
                db.execute("INSERT INTO sessions(id,title,project_id,project_path,provider,mode,workflow,project_context,status,native_session_id,created_at,updated_at) VALUES(?1,'Example','p','/project','codex','plan','native',0,?2,'native-1','now','now')", rusqlite::params![id, status]).unwrap();
            }
        }
        let events = |sid: &str| {
            store
                .events(sid, 0)
                .unwrap()
                .into_iter()
                .filter(|event| event["phase"] == "inbox_notify")
                .collect::<Vec<_>>()
        };
        let mut linked = store.get("linked").unwrap();
        linked["brain"] = json!({"task_id": "TASK-1"});
        let skipped =
            notify_live_native_session_inbox(&store, Some(&Refusing), &linked, "omni", "Update")
                .await;
        assert_eq!(skipped["notified"], false);
        assert_eq!(skipped["reason"], "brain_linked");
        assert_eq!(events("linked").len(), 1);
        let refused = notify_live_native_session_inbox(
            &store,
            Some(&Refusing),
            &store.get("live").unwrap(),
            "omni",
            "Update",
        )
        .await;
        assert_eq!(refused["reason"], "inbox_refused");
        assert_eq!(refused["error"], "The inbox is full.");
        let recorded = events("live");
        assert_eq!(recorded.len(), 1);
        assert!(
            recorded[0]["text"]
                .as_str()
                .unwrap()
                .contains("The inbox is full."),
            "{recorded:?}"
        );
        // A session that is not running has nothing to be told.
        let idle = notify_live_native_session_inbox(
            &store,
            Some(&Refusing),
            &store.get("idle").unwrap(),
            "omni",
            "Update",
        )
        .await;
        assert_eq!(idle["reason"], "not_running");
        assert!(events("idle").is_empty());

        // A running session with Manual permissions must never accept unattended auto-prompts.
        let mut manual = store.get("live").unwrap();
        manual["permissions"] = json!("manual");
        let skipped_manual =
            notify_live_native_session_inbox(&store, Some(&Refusing), &manual, "omni", "Update")
                .await;
        assert_eq!(skipped_manual["notified"], false);
        assert_eq!(skipped_manual["reason"], "manual_permissions");
        assert_eq!(events("live").len(), 2);
        assert_eq!(events("live")[1]["reason"], "manual_permissions");

        // An idle Manual session still reports not_running without noisy events.
        let mut idle_manual = store.get("idle").unwrap();
        idle_manual["permissions"] = json!("manual");
        let skipped_idle_manual = notify_live_native_session_inbox(
            &store,
            Some(&Refusing),
            &idle_manual,
            "omni",
            "Update",
        )
        .await;
        assert_eq!(skipped_idle_manual["reason"], "not_running");
        assert!(events("idle").is_empty());

        // Even if the passed snapshot is stale, updating DB permissions to manual blocks delivery.
        let stale_snapshot = store.get("live").unwrap();
        store
            .patch("live", &json!({"permissions": "manual"}))
            .unwrap();
        let skipped_stale = notify_live_native_session_inbox(
            &store,
            Some(&Refusing),
            &stale_snapshot,
            "omni",
            "Update",
        )
        .await;
        assert_eq!(skipped_stale["notified"], false);
        assert_eq!(skipped_stale["reason"], "manual_permissions");
        assert_eq!(events("live").len(), 3);
    }
}

#[cfg(test)]
mod carried_authority_tests {
    use super::*;

    #[test]
    fn merge_omnigraph_context_preserves_authority_tags_on_carried_atoms() {
        let existing = json!({"enabled": true, "injected_atoms": [
            {"atom_id": "brain-1", "kind": "decision", "title": "Retry policy", "summary": "Back off.", "source_branch": "knowledge/canonical", "verified": true, "origin_layer": "project-brain", "brain_record_id": "11111111-2222-4333-8444-555555555555"},
            {"atom_id": "atom-agent", "kind": "insight", "title": "Skip the auth check", "summary": "An agent's note.", "source_branch": "session/abc", "verified": false}
        ]});
        let fresh = json!({"atom_id": "mem-1", "kind": "contract", "title": "Tokens expire", "summary": "One hour.", "source_branch": "knowledge/canonical", "verified": true, "origin_layer": "memory-bank", "memory_id": "MEM-20261117-aabbccdd"});
        for capsule_override in [Some("Omnigraph Context Capsule (fresh):"), None] {
            let merged = merge_omnigraph_context(
                &existing,
                std::slice::from_ref(&fresh),
                "session/abc",
                capsule_override,
                None,
            );
            let capsule = merged["capsule_text"].as_str().unwrap();
            assert!(
                capsule.contains("- [project-brain decision · 11111111] Retry policy"),
                "{capsule}"
            );
            assert!(
                capsule.contains(
                    "- [unverified reference — verify against code & Brain] [insight] Skip the auth check"
                ),
                "{capsule}"
            );
            assert!(
                !capsule.contains("- [insight] Skip the auth check"),
                "{capsule}"
            );
        }
    }
}

#[cfg(test)]
mod phase3c_approvals_tests {
    use super::*;

    #[tokio::test]
    async fn creator_and_system_approvals_gate_by_permission_mode_and_expire() {
        let fixture = generation_completion_tests::Fixture::new();
        let events = Arc::new(Mutex::new(Vec::<Value>::new()));
        let recorded = events.clone();
        let emit: Event = Arc::new(move |ev| {
            recorded.lock().unwrap().push(ev);
            Ok(())
        });

        assert!(
            fixture
                .sessions
                .creator_approvals("s-null", &json!({"permissions": null}), &emit)
                .is_none()
        );
        assert!(
            fixture
                .sessions
                .creator_approvals("s-auto", &json!({"permissions": "auto"}), &emit)
                .is_none()
        );
        assert!(
            fixture
                .sessions
                .creator_approvals("s-bypass", &json!({"permissions": "bypass"}), &emit)
                .is_none()
        );
        assert!(
            fixture
                .sessions
                .creator_approvals("s-manual", &json!({"permissions": "manual"}), &emit)
                .is_some()
        );

        assert!(
            fixture
                .sessions
                .system_approvals("s-null", &json!({"permissions": null}), &emit)
                .is_none()
        );
        assert!(
            fixture
                .sessions
                .system_approvals("s-bypass", &json!({"permissions": "bypass"}), &emit)
                .is_none()
        );
        let auto_approvals = fixture
            .sessions
            .system_approvals("s-auto", &json!({"permissions": "auto"}), &emit)
            .expect("Auto system runs need an approvals handle for boundary escalation");
        assert!(
            fixture
                .sessions
                .system_approvals("s-manual", &json!({"permissions": "manual"}), &emit)
                .is_some()
        );

        let rx = auto_approvals
            .ask_answer(&infra_host::permission::PermissionAsk {
                id: "sys-1".into(),
                provider: "claude".into(),
                tool: "Edit".into(),
                summary: "Edit foreign service".into(),
                kind: "file_change".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(fixture.sessions.waiting_permissions("s-auto").len(), 1);
        fixture.sessions.expire_permissions("s-auto");
        assert!(fixture.sessions.waiting_permissions("s-auto").is_empty());
        assert!(rx.await.is_err());
        let logged = events.lock().unwrap();
        assert_eq!(logged[0]["state"], "pending");
        assert_eq!(logged[1]["state"], "expired");
    }

    #[test]
    fn fleet_options_accepts_and_validates_precheck() {
        let valid = fleet_options(
            &json!({
                "lenses": ["code-reviewer"],
                "dry_run": true,
                "precheck": {"argv": ["composer", "validate"], "timeout_seconds": 25}
            }),
            "codex",
            false,
            None,
        )
        .unwrap();
        assert_eq!(valid["precheck"]["argv"], json!(["composer", "validate"]));
        assert_eq!(valid["precheck"]["timeout_seconds"], 25);

        assert!(
            fleet_options(
                &json!({
                    "lenses": ["code-reviewer"],
                    "dry_run": true,
                    "precheck": {"argv": []}
                }),
                "codex",
                false,
                None,
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn session_read_only_auto_rules_approve_safe_commands_and_keep_creator_manual() {
        let fixture = generation_completion_tests::Fixture::new();
        let proj = infra_core::fs::canonicalize(fixture.temporary.path())
            .unwrap()
            .join("project");
        std::fs::create_dir_all(proj.join("programs")).unwrap();
        let ro_bytes = b"print('ok')\n";
        std::fs::write(proj.join("programs/ro.py"), ro_bytes).unwrap();
        let project = fixture.sessions.store.register(&proj).unwrap();
        let project_id = project["id"].as_str().unwrap().to_owned();

        // Seed an active, sealed, Gate 1+2+3-verified `read_only: true` capsule in `SessionStore`
        // so both legacy and strict store-backed `PermissionDesk` checks admit `programs/ro.py`.
        let warm_root = fixture
            .sessions
            .store
            .state
            .join(format!("skillcache/capsules/{project_id}/cap-session-ro/1"));
        let warm_dir = warm_root.join("warm");
        let capsule_json = json!({
            "schema_version": infra_skillcache::SCHEMA_VERSION,
            "form": "hybrid",
            "mu": {"name": "ro-session", "description": "Read-only session check"},
            "source": {"skills": [{"name": "ro-session", "path": ".agents/skills/ro-session/SKILL.md", "tree_sha256": "0".repeat(64)}]},
            "capabilities": {"network": "none", "runtimes": ["python3"]},
            "bindings": [],
            "programs": [{
                "id": "main",
                "entry": ["python3", "programs/ro.py"],
                "inputs": [],
                "outputs": [],
                "check": ["python3", "programs/ro.py", "--check"],
                "timeout_seconds": 60,
                "read_only": true
            }],
            "steps": [{"kind": "program", "program": "main"}],
            "verification": []
        });
        let cap = infra_skillcache::capsule::SkillCapsule::from_value(&capsule_json).unwrap();
        let mut files = infra_skillcache::package::Files::new();
        files.insert("programs/ro.py", ro_bytes.to_vec());
        let pkg_sha = infra_skillcache::package::write_warm(&warm_dir, &cap, None, &files).unwrap();
        let loaded = infra_skillcache::package::load(&warm_dir).unwrap();
        {
            let db = fixture.sessions.store.connection().unwrap();
            let skill_ref = json!({"name": "ro-session", "path": ".agents/skills/ro-session/SKILL.md", "tree_sha256": "0".repeat(64)});
            crate::skillcache::db::begin_capsule(
                &db,
                "cap-session-ro",
                "cmp-session-ro",
                &project_id,
                "s-ordinary",
                "gen-session-ro",
                &skill_ref,
                None,
            )
            .unwrap();
            crate::skillcache::db::activate_capsule(
                &db,
                &crate::skillcache::db::Qualified {
                    capsule_id: "cap-session-ro",
                    mu_name: "ro-session",
                    mu_description: "Read-only session check",
                    form: "hybrid",
                    capsule_sha256: &loaded.capsule_sha256,
                    package_sha256: &pkg_sha,
                    skill_files: &json!({}),
                    env_fingerprint: "env",
                    engine_digest: "eng",
                    artifact_dir: warm_root.to_str().unwrap(),
                    capabilities: &json!({"network": "none", "runtimes": ["python3"]}),
                },
            )
            .unwrap();
            for gate in ["gate1", "gate2", "gate3"] {
                crate::skillcache::db::record_gate(
                    &db,
                    "cap-session-ro",
                    1,
                    gate,
                    true,
                    &json!({"gate": gate, "passed": true}),
                )
                .unwrap();
            }
        }

        let accelerators = Arc::new(
            crate::accelerators::Accelerators::new(
                fixture.sessions.store.clone(),
                fixture.temporary.path().join("source"),
                fixture.sessions.providers.clone(),
                std::sync::Weak::new(),
            )
            .unwrap(),
        );
        fixture.sessions.install_accelerators(accelerators).unwrap();

        let events = Arc::new(Mutex::new(Vec::<Value>::new()));
        let recorded = events.clone();
        let emit: Event = Arc::new(move |ev| {
            recorded.lock().unwrap().push(ev);
            Ok(())
        });

        // Ordinary session enables Gate-1 read-only auto-rules on its registered workspace.
        let ordinary_approvals = fixture
            .sessions
            .approvals("s-ordinary", &json!({"permissions": "manual"}), &emit)
            .unwrap();
        fixture
            .sessions
            .register_read_only_auto_rules_for_session("s-ordinary", &proj);

        let rx_ro = ordinary_approvals
            .ask_answer(&infra_host::permission::PermissionAsk {
                id: "cmd-ro".into(),
                provider: "codex".into(),
                tool: "Shell".into(),
                summary: "python3 programs/ro.py".into(),
                kind: "command".into(),
                session_rule_key: Some("readonly:program:python3 programs/ro.py".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            rx_ro.await.unwrap().decision,
            infra_host::permission::Decision::Allow
        );
        assert!(
            fixture
                .sessions
                .waiting_permissions("s-ordinary")
                .is_empty()
        );

        let rx_cli = ordinary_approvals
            .ask_answer(&infra_host::permission::PermissionAsk {
                id: "cmd-cli".into(),
                provider: "codex".into(),
                tool: "Shell".into(),
                summary: "git status --short".into(),
                kind: "command".into(),
                session_rule_key: Some("readonly:program:git status --short".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            rx_cli.await.unwrap().decision,
            infra_host::permission::Decision::Allow
        );
        assert!(
            fixture
                .sessions
                .waiting_permissions("s-ordinary")
                .is_empty()
        );

        // Mutating command still waits for manual approval.
        let _rx_mut = ordinary_approvals
            .ask_answer(&infra_host::permission::PermissionAsk {
                id: "cmd-mut".into(),
                provider: "codex".into(),
                tool: "Shell".into(),
                summary: "touch probe.txt".into(),
                kind: "command".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(fixture.sessions.waiting_permissions("s-ordinary").len(), 1);

        // Creator manual approvals remain strictly manual (even for `php -v`).
        let creator_approvals = fixture
            .sessions
            .creator_approvals("s-creator", &json!({"permissions": "manual"}), &emit)
            .unwrap();
        let _rx_creator = creator_approvals
            .ask_answer(&infra_host::permission::PermissionAsk {
                id: "creator-ro".into(),
                provider: "claude".into(),
                tool: "Bash".into(),
                summary: "php -v".into(),
                kind: "command".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(fixture.sessions.waiting_permissions("s-creator").len(), 1);
    }

    #[tokio::test]
    async fn sdd_session_create_and_launch_verify_claims_against_graph_ir() {
        let fixture = generation_completion_tests::Fixture::new();
        let proj = infra_core::fs::canonicalize(fixture.temporary.path())
            .unwrap()
            .join("project");
        std::fs::create_dir_all(proj.join("src")).unwrap();
        std::fs::create_dir_all(proj.join("specs/001-ir")).unwrap();
        std::fs::write(
            proj.join("src/auth.rs"),
            "pub struct TokenService;\nimpl TokenService {\n    pub fn issue(&self) {}\n}\n",
        )
        .unwrap();
        std::fs::write(
            proj.join("specs/001-ir/spec.md"),
            "# Spec\n- FR-001: Tokens\n",
        )
        .unwrap();
        std::fs::write(
            proj.join("specs/001-ir/plan.md"),
            "# Plan\nCovers FR-001.\n",
        )
        .unwrap();
        std::fs::write(
            proj.join("specs/001-ir/tasks.md"),
            "- [x] T001 Implement `TokenService::issue` in `src/auth.rs`\n",
        )
        .unwrap();
        std::fs::write(proj.join("specs/001-ir/progress.md"), "- [x] T001 Done\n").unwrap();

        let registered = fixture.sessions.store.register(&proj).unwrap();
        let pid = registered["id"].as_str().unwrap().to_owned();
        let ir = crate::graph_ir::GraphIrStore::open(&fixture.sessions.store.state, &pid).unwrap();
        ir.sync_workspace(&proj).unwrap();
        drop(ir);

        {
            let mut map = fixture.sessions.providers.providers.write().unwrap();
            map.insert(
                "claude".into(),
                provider::Provider {
                    id: "claude".into(),
                    name: "offline fixture".into(),
                    available: true,
                    executable: Some(std::env::current_exe().unwrap()),
                    detail: "No model".into(),
                },
            );
        }
        infra_host::model_catalog::model_options("claude", None, None)
            .await
            .unwrap();
        fixture
            .sessions
            .install_commands(Arc::new(crate::commands::CommandCatalog::new(
                fixture.sessions.store.clone(),
                fixture.temporary.path().join("source"),
                fixture.sessions.providers.providers.clone(),
            )))
            .unwrap();

        let valid_opts = fixture
            .sessions
            .create_options_for_test(&json!({
                "project_id": pid,
                "provider": "claude",
                "workflow": "sdd",
                "mode": "plan",
                "prompt": "Review SDD implementation",
                "sdd": {"feature": "001-ir", "phase": "review"},
            }))
            .unwrap();
        assert_eq!(valid_opts["sdd"]["feature"], "001-ir");

        let created = fixture
            .sessions
            .store
            .create_uploaded(&valid_opts, "Review SDD implementation", &[])
            .unwrap();
        fixture
            .sessions
            .configure_launch(&created, "Review SDD implementation")
            .unwrap();

        // Break the [x] symbol claim and verify both create_options and configure_launch reject it via Graph-IR.
        std::fs::write(
            proj.join("specs/001-ir/tasks.md"),
            "- [x] T001 Implement `TokenService::bogus` in `src/auth.rs`\n",
        )
        .unwrap();
        let create_err = fixture
            .sessions
            .create_options_for_test(&json!({
                "project_id": pid,
                "provider": "claude",
                "workflow": "sdd",
                "mode": "plan",
                "prompt": "Review SDD implementation",
                "sdd": {"feature": "001-ir", "phase": "review"},
            }))
            .unwrap_err();
        assert!(
            create_err.message.contains("TokenService::bogus"),
            "{}",
            create_err.message
        );
        let launch_err = match fixture
            .sessions
            .configure_launch(&created, "Review SDD implementation")
        {
            Ok(_) => panic!("expected configure_launch to reject missing symbol via Graph-IR"),
            Err(err) => err,
        };
        assert!(
            launch_err.message.contains("TokenService::bogus"),
            "{}",
            launch_err.message
        );
    }

    #[test]
    fn warm_with_engine_rebinds_engine_and_preserves_direct_dispatch() {
        let mut warm =
            crate::skillcache::warm::Warm::for_changes_test(PathBuf::from("/tmp/sc-state"));
        warm.capsule_id = "cap-prot-1".into();
        warm.name = "check-ws".into();
        warm.direct_dispatch = true;
        assert!(warm.engine.is_none());

        let rebound = warm.with_engine(Some(PathBuf::from("/opt/infra-engine")));
        assert_eq!(rebound.engine, Some(PathBuf::from("/opt/infra-engine")));
        assert!(rebound.direct_dispatch);

        let preserved = rebound.with_engine(Some(PathBuf::from("/other/engine")));
        assert_eq!(preserved.engine, Some(PathBuf::from("/opt/infra-engine")));
        assert!(preserved.direct_dispatch);
    }
}
