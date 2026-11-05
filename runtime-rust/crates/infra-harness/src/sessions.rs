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
    lifecycle: Arc<crate::lifecycle::RunnerLifecycle>,
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
        || value
            .as_object()
            .unwrap()
            .keys()
            .any(|k| !["lenses", "dry_run", "budget_usd", "worker_timeout"].contains(&k.as_str()))
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
    Ok(json!({"lenses":lenses,"dry_run":dry,"budget_usd":budget,"worker_timeout":seconds}))
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
            lifecycle,
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
        let default = json!({"seconds":self.default_seconds});
        value["budgets"] = budgets(data.get("budgets").unwrap_or(&default), provider, dry)?;
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
        let (root, branch, common) = projects::workspace(&self.store, &project, &value, &sid)?;
        value["_session_id"] = sid.into();
        value["_workspace_path"] = root.to_string_lossy().into_owned().into();
        value["branch"] = json!(branch);
        value["git_common_dir"] = json!(common);
        if !value["sdd"].is_null() {
            crate::sdd::require_documents(&root, &value["sdd"])?;
        }
        Ok(value)
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
    pub fn install_results(&self, value: Arc<crate::results::Results>) -> Result<()> {
        let mut slot = lock(&self.results)?;
        if slot.is_some() {
            return Err(HarnessError::conflict("Results were already installed."));
        }
        self.install_check_worker(value.clone())?;
        *slot = Some(value);
        Ok(())
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
            return Err(HarnessError::conflict("The run queue is full."));
        }
        Ok(generation)
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
    fn configure_launch(&self, session: &Value, prompt: &str) -> Result<Launch> {
        let provider = session["provider"]
            .as_str()
            .ok_or_else(|| HarnessError::invalid("Missing provider."))?;
        let executable = self
            .providers
            .get(provider)?
            .executable
            .ok_or_else(|| HarnessError::unavailable("The provider CLI is unavailable."))?;
        let root = PathBuf::from(
            session["project_path"]
                .as_str()
                .ok_or_else(|| HarnessError::invalid("Session workspace missing."))?,
        );
        if !session["sdd"].is_null() {
            crate::sdd::require_documents(&root, &session["sdd"])?;
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
        for name in ["creator", "system_run", "system_discovery"] {
            if !session[name].is_null() {
                return self
                    .special
                    .get(name)
                    .ok_or_else(|| HarnessError::unavailable("Native workflow is unavailable."))?
                    .run(
                        session,
                        job.prompt.clone(),
                        job.generation.clone(),
                        cancel,
                        emit,
                    )
                    .await;
            }
        }
        let mut graph_binding = None;
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
            fleet.initialize(
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
            )?;
            if let Some(action) = action
                .as_deref()
                .filter(|s| ["approve", "reject"].contains(s))
            {
                fleet.decide(&job.sid, action == "approve")?;
            }
            let result = fleet.drive(&job.sid, &config, cancel, emit).await;
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
            let mut executables = BTreeMap::new();
            for id in ["claude", "codex", "cursor"] {
                if let Some(path) = self.providers.get(id)?.executable {
                    executables.insert(id.into(), path);
                }
            }
            let launch = self.configure_launch(&session, &job.prompt)?;
            let stage = if session["mode"] == "edit" {
                "implement"
            } else {
                "review"
            };
            let mut cycle = clash::Cycle {
                store: self.store.clone(),
                session: session.clone(),
                launch,
                executables,
                context: session["brain"]["capsule_text"]
                    .as_str()
                    .unwrap_or("")
                    .into(),
                adapter: self.launch_adapter(&session)?,
            };
            let result = cycle
                .drive(
                    &job.prompt,
                    stage,
                    !session["clash_result"].is_null(),
                    cancel,
                    emit,
                )
                .await;
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
        let route = if session["workflow"] == "native" {
            self.commands()?.route(&session, &job.prompt).await?
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
        let configured = (|| -> Result<Launch> {
            let mut launch = self.configure_launch(&session, &route.text)?;
            let prepared = crate::prompt::build(
                &self.store,
                &session,
                &job.prompt,
                &route,
                &binding.instructions,
            )?;
            launch.prompt = prepared.text;
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
        let launch = match configured {
            Ok(launch) => launch,
            Err(error) => {
                let _ = self.lifecycle.abort(&binding, "failed");
                return Err(error);
            }
        };
        let adapter = self.launch_adapter(&session)?;
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
        let observed = Box::pin(provider::run_monitored_locked(
            &launch,
            Limits {
                seconds: session["budgets"]["seconds"].as_u64(),
                ..Limits::default()
            },
            dispatch_cancel,
            previous,
            Some(self.store.runner_lock()),
            Some(adapter.as_ref()),
            &binding.environment,
            Some(Arc::new(binding.clone())),
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
                emit_events(event)
            },
        ))
        .await;
        drop(forwarding);
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
                return Err(error);
            }
        };
        if inbox_owned
            && (expected_native.is_none()
                || result.session_id.as_deref() != expected_native.as_deref())
        {
            let _ = self.lifecycle.abort(&binding, "failed");
            return Err(HarnessError::conflict(
                "The assigned inbox turn has no matching native identity proof.",
            ));
        }
        let mut recovery = NativeRecovery {
            adapter,
            human_cancel: cancel.clone(),
        };
        let memory = self
            .lifecycle
            .finish(&binding, &result, &emit, &mut recovery)
            .await?;
        if !result.ok {
            return Err(HarnessError::unavailable(
                result
                    .error
                    .unwrap_or_else(|| "Native session did not complete.".into()),
            ));
        }
        *native_proof = result.session_id.clone();
        Ok(
            json!({"kind":"result","ok":true,"text":result.text,"observed_cost_usd":result.cost_usd,"provider_billing":"unknown","memory":memory["memory"],"full_cost":memory["full_cost"]}),
        )
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
        if let Some(brain) = session["brain"].as_object() {
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
                "sessions.create" => self.create_ready(self.create_options(body)?)?,
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
                    let _admission = lock(&self.admission)?;
                    let session = self.store.get(sid)?;
                    if active(&session) {
                        return Err(HarnessError::conflict("Wait for this session to finish."));
                    }
                    if !store::exact(body, &["query"], &[]) {
                        return Err(HarnessError::invalid("Invalid context request."));
                    }
                    let prepared = self.knowledge.prepare(
                        &session,
                        body["query"]
                            .as_str()
                            .ok_or_else(|| HarnessError::invalid("Missing context query."))?,
                    )?;
                    self.store.patch(sid, &json!({"brain":prepared}))?
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
                    let _admission = lock(&self.admission)?;
                    let session = self.store.get(sid)?;
                    if active(&session) {
                        return Err(HarnessError::conflict("Wait for this session to finish."));
                    }
                    return Ok(Payload::Json(self.knowledge.save_memory(&session, body)?));
                }
                _ => return Err(HarnessError::missing("Unknown session operation.")),
            };
            Ok(Payload::Json(json!({"session":value})))
        })
    }
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
                lifecycle: Arc::new(crate::lifecycle::RunnerLifecycle::new(
                    store, knowledge, source,
                )),
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
