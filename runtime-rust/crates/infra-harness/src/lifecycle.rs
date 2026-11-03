//! Generation-owned native recall, bounded recovery and agent-attested saves.
use crate::{
    HarnessError, Result, knowledge::KnowledgeDomain, launch_accounting::LaunchAccounting,
    memory_attempts, memory_draft, sessions::Event, store::SessionStore,
};
use infra_core::{
    self as core, PrepareResult, RecordResult, ResultService, Revision, domain::json as wire,
};
use infra_host::{
    process::RunObserver,
    provider::{Launch, RunEvents, WorkerResult},
};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
#[derive(Clone, Copy, Debug)]
pub struct LifecycleOptions {
    pub native_command: bool,
    pub supported: bool,
}
impl Default for LifecycleOptions {
    fn default() -> Self {
        Self {
            native_command: false,
            supported: true,
        }
    }
}
#[derive(Clone)]
pub struct RunnerLifecycle {
    pub store: SessionStore,
    pub knowledge: Arc<KnowledgeDomain>,
    pub source: PathBuf,
    queue_engine: Option<PathBuf>,
}
#[derive(Clone)]
pub struct RunBinding {
    pub session: Value,
    pub ready: bool,
    pub instructions: String,
    pub environment: BTreeMap<String, Option<String>>,
    state: Arc<Mutex<RunState>>,
}
struct RunState {
    store: SessionStore,
    sid: String,
    generation: String,
    cancel: Arc<AtomicBool>,
    recovery_cancel: Option<Arc<AtomicBool>>,
    provider: String,
    session: Value,
    enabled: bool,
    native_command: bool,
    finalized: bool,
    persisted: bool,
    phase: String,
    started: Option<Instant>,
    started_at: Option<f64>,
    launch: Option<Launch>,
    native_id: Option<String>,
    tokens: Option<u64>,
    cost: Option<f64>,
    cost_unknown: bool,
    limit: Option<String>,
    accounting: LaunchAccounting,
    recovery_launched: bool,
    cleanup_confirmed: bool,
    recovery_replies: Vec<String>,
    recovery_terminal: bool,
    recovery_failed: bool,
    cursor_retirement: Option<Value>,
}
enum RecoveryPlan {
    Run(Box<RecoveryRequest>),
    Skip {
        state: &'static str,
        reason: &'static str,
    },
}
pub struct RecoveryRequest {
    pub launch: Launch,
    pub timeout: Duration,
    pub token_limit: Option<u64>,
    pub output_limit: usize,
    pub extra_env: BTreeMap<String, Option<String>>,
    pub cancel: Arc<AtomicBool>,
    pub previous_total: Option<f64>,
    pub runner_lock: Arc<std::fs::File>,
}
pub type RecoveryReply<'a> = Pin<Box<dyn Future<Output = Result<WorkerResult>> + Send + 'a>>;
pub trait RecoveryRunner: Send {
    fn run<'a>(
        &'a mut self,
        request: RecoveryRequest,
        events: Arc<dyn RunEvents>,
        emit: Event,
    ) -> RecoveryReply<'a>;
}
fn locked(binding: &RunBinding) -> Result<std::sync::MutexGuard<'_, RunState>> {
    binding
        .state
        .lock()
        .map_err(|_| HarnessError::unavailable("Launch memory observation is unavailable."))
}
fn current(
    store: &SessionStore,
    sid: &str,
    generation: &str,
    cancel: &AtomicBool,
) -> Result<Value> {
    if cancel.load(Ordering::Acquire) || store.generation(sid)? != generation {
        return Err(HarnessError::conflict(
            "The memory generation is no longer active.",
        ));
    }
    let session = store.get(sid)?;
    if session["status"] == "cancelled" {
        return Err(HarnessError::conflict(
            "The memory generation is no longer active.",
        ));
    }
    Ok(session)
}
fn patch(
    store: &SessionStore,
    sid: &str,
    generation: &str,
    cancel: &AtomicBool,
    fields: &Value,
) -> Result<Value> {
    store.patch_guarded(sid, generation, fields, cancel)
}
fn event(store: &SessionStore, sid: &str, generation: &str, value: Value) -> Result<()> {
    if !store.guarded_event(sid, generation, &value)? {
        return Err(HarnessError::conflict(
            "The memory generation is no longer active.",
        ));
    }
    Ok(())
}
fn string(value: &Value, key: &str) -> Result<String> {
    value[key]
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .map(String::from)
        .ok_or_else(|| HarnessError::invalid(format!("Missing {key}.")))
}
fn sources_from_brain(brain: &Value) -> Value {
    brain.get("capsule").cloned().unwrap_or(Value::Null)
}
impl RunnerLifecycle {
    pub fn new(store: SessionStore, knowledge: Arc<KnowledgeDomain>, source: PathBuf) -> Self {
        Self {
            store,
            knowledge,
            source,
            queue_engine: None,
        }
    }
    pub fn with_queue_engine(mut self, engine: PathBuf) -> Self {
        self.queue_engine = Some(engine);
        self
    }
    pub fn begin(
        &self,
        session: &Value,
        prompt: &str,
        generation: &str,
        cancel: Arc<AtomicBool>,
    ) -> Result<RunBinding> {
        self.begin_with_options(
            session,
            prompt,
            generation,
            cancel,
            LifecycleOptions::default(),
        )
    }
    pub fn begin_with_options(
        &self,
        session: &Value,
        prompt: &str,
        generation: &str,
        cancel: Arc<AtomicBool>,
        options: LifecycleOptions,
    ) -> Result<RunBinding> {
        let sid = string(session, "id")?;
        let mut session = current(&self.store, &sid, generation, &cancel)?;
        let provider = string(&session, "provider")?;
        let project = PathBuf::from(string(&session, "project_path")?);
        let supported = options.supported
            && [
                "fleet",
                "clash",
                "creator",
                "system_run",
                "system_discovery",
            ]
            .into_iter()
            .all(|field| session[field].is_null());
        let linked = session["brain"].is_object();
        let prepare_linked = linked
            && ["creator", "system_run", "system_discovery"]
                .into_iter()
                .all(|field| session[field].is_null())
            && !matches!(session["fleet_action"].as_str(), Some("approve" | "reject"));
        let reviewed = session["brain"]["review"] != false;
        let mut ready = true;
        let registered = PathBuf::from(string(
            &self
                .store
                .project(session["project_id"].as_str().unwrap_or(""))?,
            "path",
        )?);
        let accounting =
            LaunchAccounting::new(&provider, &session, generation, &project, &registered, None);
        let mut early = BeginAccountingGuard {
            store: self.store.clone(),
            sid: sid.clone(),
            generation: generation.into(),
            cancel: cancel.clone(),
            accounting: accounting.clone(),
            armed: true,
        };
        let recall_started = Instant::now();
        if prepare_linked && options.native_command && reviewed {
            return Err(HarnessError::conflict(
                "This session reviews project memory before each run, and a Claude Code command goes to the CLI without it. Run commands in a session with automatic memory.",
            ));
        }
        if prepare_linked && !options.native_command {
            if reviewed && session["brain"]["approved"] == true {
                match retry_busy(&self.store, &sid, generation, &cancel, || {
                    self.knowledge.verify_prepared(&session)
                }) {
                    Ok(()) => {
                        let mut brain = session["brain"].clone();
                        brain["approved"] = json!(false);
                        session = patch(
                            &self.store,
                            &sid,
                            generation,
                            &cancel,
                            &json!({"brain":brain}),
                        )?;
                    }
                    Err(error) => {
                        let mut brain = session["brain"].clone();
                        brain["approved"] = json!(false);
                        brain["context_id"] = Value::Null;
                        patch(
                            &self.store,
                            &sid,
                            generation,
                            &cancel,
                            &json!({"brain":brain,"status":"awaiting_context"}),
                        )?;
                        event(
                            &self.store,
                            &sid,
                            generation,
                            json!({"kind":"error","text":error.message}),
                        )?;
                        session = self.store.get(&sid)?;
                        ready = false;
                    }
                }
            } else {
                let result = self.prepare_context(
                    &session,
                    prompt,
                    generation,
                    &cancel,
                    !reviewed,
                    &accounting,
                );
                match result {
                    Ok(brain) => {
                        session = patch(
                            &self.store,
                            &sid,
                            generation,
                            &cancel,
                            &json!({"brain":brain}),
                        )?;
                        if reviewed {
                            session = patch(
                                &self.store,
                                &sid,
                                generation,
                                &cancel,
                                &json!({"status":"awaiting_context"}),
                            )?;
                            event(
                                &self.store,
                                &sid,
                                generation,
                                json!({"kind":"status","text":"Context prepared. Review it before starting the provider."}),
                            )?;
                            ready = false;
                        } else {
                            event(
                                &self.store,
                                &sid,
                                generation,
                                json!({"kind":"memory","ok":true,"text":memory_notice(&session["brain"])?}),
                            )?;
                        }
                    }
                    Err(error) => {
                        current(&self.store, &sid, generation, &cancel)?;
                        let mut brain = self.store.get(&sid)?["brain"].clone();
                        brain["context_id"] = Value::Null;
                        brain["capsule"] = Value::Null;
                        brain["capsule_text"] = Value::Null;
                        brain["source_hashes"] = json!({});
                        if reviewed {
                            brain["approved"] = json!(false);
                            session = patch(
                                &self.store,
                                &sid,
                                generation,
                                &cancel,
                                &json!({"brain":brain,"status":"awaiting_context"}),
                            )?;
                            event(
                                &self.store,
                                &sid,
                                generation,
                                json!({"kind":"error","text":error.message}),
                            )?;
                            ready = false;
                        } else {
                            session = patch(
                                &self.store,
                                &sid,
                                generation,
                                &cancel,
                                &json!({"brain":brain}),
                            )?;
                            event(
                                &self.store,
                                &sid,
                                generation,
                                json!({"kind":"memory","ok":false,"text":format!("Project memory was not retrieved for this turn: {}",error.message)}),
                            )?;
                        }
                    }
                }
            }
        }
        accounting.duration(
            if reviewed { "prepare" } else { "recall" },
            recall_started.elapsed().as_secs_f64(),
        );
        let instructions = if linked && supported && !options.native_command && !prompt.is_empty() {
            format!("\n\n{}", memory_draft::instruction())
        } else {
            String::new()
        };
        let mut environment = BTreeMap::from([
            ("CONTEXT_CAPSULE_DELIVERED".into(), None),
            ("CONTEXT_MEMORY_RECOVERY".into(), None),
            ("CONTEXT_TASK_ID".into(), None),
        ]);
        if linked && supported && !options.native_command {
            environment.insert(
                "CONTEXT_TASK_ID".into(),
                session["brain"]["task_id"].as_str().map(String::from),
            );
            if session["brain"]["capsule"].is_object() {
                environment.insert("CONTEXT_CAPSULE_DELIVERED".into(), Some("1".into()));
            }
        }
        let capsule = sources_from_brain(&session["brain"]);
        accounting.set_capsule(capsule.is_object().then_some(&capsule));
        accounting.payload("draft_instruction", &instructions);
        if let Ok(context) = self
            .knowledge
            .context(session["project_id"].as_str().unwrap_or(""), Some(&project))
        {
            accounting.set_outside_memory(context.roots.attached());
        }
        let native_id = session["native_session_id"].as_str().map(String::from);
        let state = RunState {
            store: self.store.clone(),
            sid,
            generation: generation.into(),
            cancel,
            recovery_cancel: None,
            provider,
            session: session.clone(),
            enabled: supported,
            native_command: options.native_command,
            finalized: false,
            persisted: false,
            phase: "main".into(),
            started: None,
            started_at: None,
            launch: None,
            native_id,
            tokens: None,
            cost: None,
            cost_unknown: false,
            limit: None,
            accounting,
            recovery_launched: false,
            cleanup_confirmed: false,
            recovery_replies: vec![],
            recovery_terminal: false,
            recovery_failed: false,
            cursor_retirement: None,
        };
        early.armed = false;
        Ok(RunBinding {
            session,
            ready,
            instructions,
            environment,
            state: Arc::new(Mutex::new(state)),
        })
    }
    fn prepare_context(
        &self,
        session: &Value,
        prompt: &str,
        generation: &str,
        cancel: &AtomicBool,
        automatic: bool,
        accounting: &LaunchAccounting,
    ) -> Result<Value> {
        let sid = string(session, "id")?;
        let query = if automatic {
            crate::brain_settings::query(prompt)?
        } else {
            session["brain"]["query"]
                .as_str()
                .unwrap_or(prompt)
                .to_string()
        };
        if query.trim().is_empty() {
            return Err(HarnessError::invalid(
                "The automatic query has no safe technical content.",
            ));
        }
        self.with_memory_operation(session,generation,cancel,|context|{
   current(&self.store,&sid,generation,cancel)?;let mut brain=session["brain"].clone();brain["query"]=json!(query);let task_alias=string(&brain,"task_id")?;let service=core::MemoryService::new(context.clone(),None);
   let binding=if brain["create"]==true{measured_core(accounting,"start",||service.execute("start",&json!({"task_id":task_alias,"goal":brain["goal"]}),&[]))?}else{measured_core(accounting,"rebind",||service.execute("rebind",&json!({"task_id":task_alias,"record_id":brain["record_id"]}),&[]))?};let id=binding["task_uuid"].as_str().filter(|id|core::domain::is_uuid4(id)).ok_or_else(||HarnessError::invalid("The Brain runtime did not return the task identity."))?.to_string();
   brain["record_id"]=json!(id);brain["create"]=json!(false);brain["approved"]=json!(false);brain["context_id"]=Value::Null;brain["capsule"]=Value::Null;patch(&self.store,&sid,generation,cancel,&json!({"brain":brain}))?;
   current(&self.store,&sid,generation,cancel)?;let task=ResultService::new(context.clone()).task(&id,false)?.record;let request=json!({"query":query,"task_id":task_alias,"host":if automatic{session["provider"].clone()}else{json!("cli")},"limit":3,"ephemeral":automatic,"sanitize":false,"provenance_response":automatic,"gate":"off","json":true});let action=if automatic{"refresh"}else{"retrieve"};let refreshed=measured_core(accounting,action,||service.execute(action,&request,&[]))?;let capsule=if automatic{refreshed["capsule"].clone()}else{refreshed.clone()};let provenance=automatic.then(||refreshed.get("source_provenance")).flatten();let hashes=self.knowledge.prepared_sources(context,&capsule,&task,provenance,automatic)?;
   brain["context_id"]=json!(uuid::Uuid::new_v4().to_string());brain["capsule"]=capsule;brain["capsule_text"]=if automatic{refreshed["capsule_text"].clone()}else{Value::Null};brain["task"]=task;brain["source_hashes"]=hashes;for field in ["funnel_diagnosis","ordinary_receipt"]{if let Some(value)=refreshed.get(field){brain[field]=value.clone();}}current(&self.store,&sid,generation,cancel)?;Ok(brain)
  })
    }
    pub fn set_prompt_ledger(
        &self,
        binding: &RunBinding,
        ledger: &Value,
        agents: u32,
    ) -> Result<()> {
        let state = locked(binding)?;
        state.accounting.set_ledger(ledger, agents);
        if let Some(retirement) = &state.cursor_retirement {
            state.accounting.set_cursor_rule_retirement(retirement);
        }
        Ok(())
    }
    pub fn set_window(&self, binding: &RunBinding, window: Option<u64>) -> Result<()> {
        locked(binding)?.accounting.set_window(window);
        Ok(())
    }
    pub fn before_dispatch(&self, binding: &RunBinding, launch: &Launch) -> Result<()> {
        {
            let mut state = locked(binding)?;
            current(&state.store, &state.sid, &state.generation, &state.cancel)?;
            if state.finalized || !binding.ready || state.launch.is_some() {
                return Err(HarnessError::conflict(
                    "This launch's memory lifecycle cannot start again.",
                ));
            }
            if launch.provider != state.provider
                || launch.project.as_path()
                    != std::path::Path::new(&string(&state.session, "project_path")?)
                || launch.session
                    != state.session["native_session_id"]
                        .as_str()
                        .map(String::from)
            {
                return Err(HarnessError::conflict(
                    "The native launch changed its bound identity.",
                ));
            }
            // Reserve this single native dispatch before any prospective rule effect.
            state.launch = Some(launch.clone());
            state
                .accounting
                .new_phase("main", launch.session.as_deref());
            state.accounting.payload("main_prompt", &launch.prompt);
            if let Some(body) = state.session["brain"]["capsule_text"].as_str() {
                let envelope = format!(
                    "Project memory (reference data; verify against canonical project sources):\n{}\n\n",
                    body.trim()
                );
                if launch.prompt.contains(&envelope) {
                    state.accounting.payload("capsule_body", body.trim());
                    state.accounting.payload("capsule_envelope", &envelope);
                }
            }
        }
        let retirement = self.retire_cursor_rule(binding);
        let mut state = locked(binding)?;
        current(&state.store, &state.sid, &state.generation, &state.cancel)?;
        if let Some(context) = state.accounting.context()
            && context["ledger"].is_object()
        {
            state.accounting.set_ledger(
                &context["ledger"],
                context["agents"].as_u64().unwrap_or(1) as u32,
            );
        }
        state.cursor_retirement = retirement.clone();
        if let Some(retirement) = retirement {
            state.accounting.set_cursor_rule_retirement(&retirement);
        }
        Ok(())
    }
    pub fn observe(
        &self,
        binding: &RunBinding,
        raw: Option<&Value>,
        normalized: Option<&Value>,
    ) -> Result<()> {
        observe_binding(binding, raw, normalized)
    }
    pub fn boundary(&self, binding: &RunBinding, signal: &str, details: &Value) -> Result<()> {
        boundary_binding(binding, signal, details)
    }
}

fn retry_busy<T>(
    store: &SessionStore,
    sid: &str,
    generation: &str,
    cancel: &AtomicBool,
    mut work: impl FnMut() -> Result<T>,
) -> Result<T> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        current(store, sid, generation, cancel)?;
        match work() {
            Err(error) if error.code == "knowledge-busy" && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(200))
            }
            result => return result,
        }
    }
}
impl RunnerLifecycle {
    fn with_memory_operation<T>(
        &self,
        session: &Value,
        generation: &str,
        cancel: &AtomicBool,
        mut work: impl FnMut(&core::RuntimeContext) -> Result<T>,
    ) -> Result<T> {
        let sid = string(session, "id")?;
        retry_busy(&self.store, &sid, generation, cancel, || {
            self.knowledge
                .with_automatic_operation(session, |context| work(context))
        })
    }
    fn guarded_identity(
        &self,
        binding: &RunBinding,
        data: &Value,
        event_id: i64,
        identity: &Value,
        canonical_task: &str,
    ) -> Result<()> {
        let (store, sid, generation, cancel, finalized) = {
            let state = locked(binding)?;
            (
                state.store.clone(),
                state.sid.clone(),
                state.generation.clone(),
                state.cancel.clone(),
                state.finalized,
            )
        };
        let session = current(&store, &sid, &generation, &cancel)?;
        let context = self.knowledge.bound_context(&session)?;
        let mut scope = self.knowledge.automatic_scope(&session, &context)?;
        if !scope["task"].as_str().is_some_and(core::domain::is_uuid4) {
            scope["task"] = json!(canonical_task);
            scope["task_alias"] = session["brain"]["task_id"].clone();
        }
        let db = store.connection()?;
        memory_attempts::guard_conn(&db, &sid, &generation, &cancel, finalized)?;
        let checked =
            memory_attempts::identity_conn(&db, &sid, &generation, event_id, data, &scope)?;
        if checked != *identity {
            return Err(HarnessError::conflict(memory_attempts::CONFLICT));
        }
        Ok(())
    }
    fn replay(
        &self,
        binding: &RunBinding,
        data: &Value,
        event_id: i64,
        identity: &Value,
        context: &core::RuntimeContext,
        retained: &Value,
    ) -> Result<Value> {
        let task = retained["identity"]["scope"]["task"]
            .as_str()
            .ok_or_else(|| HarnessError::conflict(memory_attempts::INCOMPLETE))?;
        let value = &retained["prepared"];
        let value = memory_attempts::prepared(
            value,
            task,
            value["result_id"].as_str().unwrap_or(""),
            value["revision"].as_u64().unwrap_or(0),
            retained["identity"]["scope"]["owner"]
                .as_str()
                .unwrap_or(""),
            memory_draft::AUTOMATIC_REASON,
        )?;
        let skipped = (0..retained["skipped_count"].as_u64().unwrap_or(0))
            .map(
                |_| json!({"reason":"An omission from the original automatic draft was retained."}),
            )
            .collect::<Vec<_>>();
        self.guarded_identity(binding, data, event_id, identity, task)?;
        let accounting = locked(binding)?.accounting.clone();
        let outcome = measured_core(&accounting, "replay-result", || {
            ResultService::new(context.clone()).record(RecordResult {
                task_id: task.into(),
                result_id: value["result_id"].as_str().unwrap_or("").into(),
                revision: Revision::Exact(value["revision"].as_u64().unwrap_or(0)),
                data: value["request"].clone(),
                reason: memory_draft::AUTOMATIC_REASON.into(),
                attestation: "agent".into(),
                prepared: value["scope"].as_str().map(String::from),
                replay_only: true,
            })
        });
        match outcome {
            Ok(outcome) => Ok(memory_outcome(&outcome, skipped)),
            Err(_) => Ok(
                json!({"ok":false,"saved":{"task":null,"records":[],"promotion":null},"skipped":skipped,"error":memory_attempts::INCOMPLETE}),
            ),
        }
    }
    pub fn save_automatic(
        &self,
        binding: &RunBinding,
        data: &Value,
        event_id: i64,
    ) -> Result<Value> {
        let (sid, generation, cancel, finalized) = {
            let state = locked(binding)?;
            (
                state.sid.clone(),
                state.generation.clone(),
                state.cancel.clone(),
                state.finalized,
            )
        };
        if finalized {
            return Err(HarnessError::conflict(memory_attempts::INCOMPLETE));
        }
        let session = current(&self.store, &sid, &generation, &cancel)?;
        self.with_memory_operation(&session,&generation,&cancel,|context|{
   let retained=memory_attempts::lookup(&self.store,&sid,&generation)?;let mut scope=self.knowledge.automatic_scope(&session,context)?;let service=ResultService::new(context.clone());let accounting=locked(binding)?.accounting.clone();let canonical=if let Some(task)=scope["task"].as_str().filter(|task|core::domain::is_uuid4(task)){task.to_string()}else if let Some(task)=retained.as_ref().and_then(|retained|retained["identity"]["scope"]["task"].as_str()).filter(|task|core::domain::is_uuid4(task)){task.to_string()}else{let alias=string(&session["brain"],"task_id")?;service.task(&alias,false)?.record["id"].as_str().filter(|task|core::domain::is_uuid4(task)).ok_or_else(||HarnessError::conflict(memory_attempts::INCOMPLETE))?.to_string()};if !scope["task"].as_str().is_some_and(core::domain::is_uuid4){scope["task"]=json!(canonical);scope["task_alias"]=session["brain"]["task_id"].clone();}
   let identity={let db=self.store.connection()?;memory_attempts::guard_conn(&db,&sid,&generation,&cancel,false)?;memory_attempts::identity_conn(&db,&sid,&generation,event_id,data,&scope)?};if let Some(retained)=retained{if retained["identity"]!=identity{return Err(HarnessError::conflict(memory_attempts::CONFLICT));}return self.replay(binding,data,event_id,&identity,context,&retained);}
   self.guarded_identity(binding,data,event_id,&identity,&canonical)?;if let Some(retained)=memory_attempts::lookup(&self.store,&sid,&generation)?{if retained["identity"]!=identity{return Err(HarnessError::conflict(memory_attempts::CONFLICT));}return self.replay(binding,data,event_id,&identity,context,&retained);}
   let(safe,skipped)=memory_draft::usable(&context.roots.project,data)?;if safe["progress"].as_str().unwrap_or("").is_empty()&&safe["next_steps"].as_array().is_none_or(Vec::is_empty)&&safe["learnings"].as_array().is_none_or(Vec::is_empty){self.guarded_identity(binding,data,event_id,&identity,&canonical)?;if let Some(retained)=memory_attempts::lookup(&self.store,&sid,&generation)?{return self.replay(binding,data,event_id,&identity,context,&retained);}return Ok(json!({"ok":true,"saved":{"task":null,"records":[],"promotion":null},"skipped":skipped,"error":null}));}
   let draft=memory_draft::submission(&safe)?;memory_draft::check_sources(&context.roots.project,&draft)?;let mut task=service.task(&canonical,false)?.record;if matches!(task["status"].as_str(),Some("completed"|"cancelled")){return Err(HarnessError::conflict("The linked task is finished. Link an active task to save more."));}
   let mut request=json!({"learnings":draft["learnings"],"verified":true});if let Some(capture)=draft.get("capture_decision"){request["capture_decision"]=capture.clone();}
   if !draft["progress"].as_str().unwrap_or("").is_empty()||draft["next_steps"].as_array().is_some_and(|steps|!steps.is_empty()){request["next_steps"]=draft["next_steps"].clone();if !draft["progress"].as_str().unwrap_or("").is_empty(){request["progress"]=draft["progress"].clone();}}
   let result_id=core::query::chars(&format!("harness-{sid}-{generation}"),64);let mut stale_retries=0usize;let mut prepared=None;
   for attempt in 0..2{self.guarded_identity(binding,data,event_id,&identity,&canonical)?;match measured_core(&accounting,"prepare-result",||service.prepare(PrepareResult{task_id:canonical.clone(),result_id:result_id.clone(),revision:task["revision"].as_u64().unwrap_or(0),data:request.clone(),reason:memory_draft::AUTOMATIC_REASON.into(),attestation:"agent".into()})){Ok(value)=>{prepared=Some(memory_attempts::prepared(&value,&canonical,&result_id,task["revision"].as_u64().unwrap_or(0),&context.actor,memory_draft::AUTOMATIC_REASON)?);break;},Err(error)if attempt==0&&error.code.as_deref()==Some("memory-result-v1:prepared_stale_no_write")=>{stale_retries+=1;locked(binding)?.accounting.note("stale_revision_retries");task=service.task(&canonical,false)?.record;},Err(error)=>return Err(error.into())}}
   let prepared=prepared.ok_or_else(||HarnessError::conflict(memory_attempts::INCOMPLETE))?;self.guarded_identity(binding,data,event_id,&identity,&canonical)?;let(retained,first)={let db=self.store.connection()?;memory_attempts::guard_conn(&db,&sid,&generation,&cancel,false)?;memory_attempts::insert_conn(&db,&identity,&prepared,skipped.len())?};if !first{return self.replay(binding,data,event_id,&identity,context,&retained);}
   let mut last=None;for attempt in 0..2{self.guarded_identity(binding,data,event_id,&identity,&canonical)?;match measured_core(&accounting,"record-result",||service.record(RecordResult{task_id:canonical.clone(),result_id:result_id.clone(),revision:Revision::Exact(task["revision"].as_u64().unwrap_or(0)),data:prepared["request"].clone(),reason:memory_draft::AUTOMATIC_REASON.into(),attestation:"agent".into(),prepared:prepared["scope"].as_str().map(String::from),replay_only:false})){Ok(value)=>return Ok(memory_outcome(&value,skipped)),Err(error)if attempt==0&&stale_retries==0&&error.code.as_deref()==Some("memory-result-v1:prepared_stale_no_write")=>{stale_retries+=1;locked(binding)?.accounting.note("stale_revision_retries");task=service.task(&canonical,false)?.record;},Err(error)=>{last=Some(error);break;}}}Ok(json!({"ok":false,"saved":{"task":null,"records":[],"promotion":null},"skipped":skipped,"error":last.map(|error|error.message).unwrap_or_else(||"The runtime did not finish.".into())}))
  })
    }
}
fn memory_outcome(outcome: &Value, skipped: Vec<Value>) -> Value {
    let allowed = [
        "archived",
        "kind-conflict",
        "revision-conflict",
        "not-authorized",
        "person-or-unknown-attestation",
        "original-image-unproven",
        "record-not-found",
        "reference-mismatch",
        "not-verified-terminal",
    ];
    let records=outcome["records"].as_array().into_iter().flatten().filter(|record|record.is_object()).map(|record|{let mut value=json!({"id":record["id"],"type":record["type"],"title":record["title"],"status":record["status"],"state":record["state"]});if let Some(revision)=record["revision"].as_u64().filter(|revision|*revision>0){value["revision"]=json!(revision);}
    if record["reason"].as_str().is_some_and(|reason|allowed.contains(&reason)){value["reason"]=record["reason"].clone();}value}).collect::<Vec<_>>();
    let mut saved = json!({"task":outcome.get("task").filter(|value|value.is_object()),"records":records,"promotion":outcome.get("promotion").filter(|value|value.is_object())});
    if outcome["capture_decision"]
        .as_str()
        .is_some_and(|decision| {
            [
                "selected",
                "no_reusable_knowledge",
                "insufficiently_checked",
            ]
            .contains(&decision)
        })
    {
        saved["capture_decision"] = outcome["capture_decision"].clone();
    }
    json!({"ok":true,"saved":saved,"skipped":skipped,"error":null})
}

impl RunObserver for RunBinding {
    fn boundary(&self, signal: &str, details: &Value) -> Result<()> {
        boundary_binding(self, signal, details)
    }
}
impl RunEvents for RunBinding {
    fn observe(&self, raw: Option<&Value>, normalized: Option<&Value>) -> Result<()> {
        observe_binding(self, raw, normalized)
    }
    fn enrich(&self, clean: &Value) -> Result<Value> {
        let state = locked(self)?;
        if !state.enabled {
            return Ok(clean.clone());
        }
        let mut enriched = clean.clone();
        if let Some(display) = state.accounting.clean(&state.phase, clean)
            && let (Some(target), Some(display)) = (enriched.as_object_mut(), display.as_object())
        {
            target.extend(display.clone());
        }
        for notice in state.accounting.take_events() {
            event(&state.store, &state.sid, &state.generation, notice)?;
        }
        Ok(enriched)
    }
}
fn observe_binding(
    binding: &RunBinding,
    raw: Option<&Value>,
    normalized: Option<&Value>,
) -> Result<()> {
    let mut state = locked(binding)?;
    if state.finalized {
        return Ok(());
    }
    current(&state.store, &state.sid, &state.generation, &state.cancel)?;
    let phase = state.phase.clone();
    let mut usage_changed = false;
    if let Some(raw) = raw {
        state.accounting.raw(&phase, raw);
    }
    if !state.enabled {
        return Ok(());
    }
    if let Some(clean) = normalized {
        if state.enabled
            && let Some(native) = clean["native_session_id"]
                .as_str()
                .filter(|native| !native.is_empty())
        {
            if state
                .native_id
                .as_deref()
                .is_some_and(|expected| expected != native)
            {
                state
                    .accounting
                    .boundary(&phase, "identity_changed", &Value::Null);
                return Err(HarnessError::conflict(
                    "Provider changed the original session identity.",
                ));
            }
            state.native_id = Some(native.into());
        }

        if clean["kind"] == "usage" {
            usage_changed = true;
            if let Some(tokens) = infra_host::provider::total_tokens(clean) {
                state.tokens = Some(state.tokens.unwrap_or(0).saturating_add(tokens));
            }
            let own = clean["cost_usd"]
                .as_f64()
                .filter(|cost| cost.is_finite() && *cost >= 0.0);
            let reported = clean["cost_usd_reported"]
                .as_f64()
                .filter(|cost| cost.is_finite() && *cost >= 0.0);
            state.accounting.usd(&phase, own, reported);
            if let Some(cost) = own {
                if !state.cost_unknown {
                    state.cost = Some(state.cost.unwrap_or(0.0) + cost);
                }
            } else if phase == "recovery" && reported.is_some() {
                state.cost_unknown = true;
                state.cost = None;
            }
            if let Some(total) = clean["cost_session_total"]
                .as_f64()
                .filter(|cost| cost.is_finite() && *cost >= 0.0)
                && let Some(native) = state.native_id.clone()
            {
                let mut session =
                    current(&state.store, &state.sid, &state.generation, &state.cancel)?;
                if !session["cost_totals"].is_object() {
                    session["cost_totals"] = json!({});
                }
                session["cost_totals"][native] = json!(total);
                patch(
                    &state.store,
                    &state.sid,
                    &state.generation,
                    &state.cancel,
                    &json!({"cost_totals":session["cost_totals"]}),
                )?;
            }
            if state.session["budgets"]["tokens"]
                .as_u64()
                .zip(state.tokens)
                .is_some_and(|(limit, used)| used >= limit)
            {
                state.limit = Some("token".into());
            }
            if state.session["budgets"]["usd"]
                .as_f64()
                .zip(state.cost)
                .is_some_and(|(limit, used)| {
                    if phase == "recovery" {
                        used >= limit
                    } else {
                        used > limit
                    }
                })
            {
                state.limit = Some("USD".into());
            }
            if state.limit.is_some() {
                if phase == "recovery" {
                    state
                        .accounting
                        .boundary("recovery", "shared_budget", &Value::Null);
                }
                if let Some(cancel) = &state.recovery_cancel {
                    cancel.store(true, Ordering::Release);
                }
            }
            if phase == "recovery" {
                let mut usage = clean.clone();
                usage["memory_recovery"] = json!(true);
                event(&state.store, &state.sid, &state.generation, usage)?;
            }
        }
        if phase == "recovery" {
            if clean["kind"] == "result" {
                state.recovery_terminal = true;
                state.recovery_failed |= clean["ok"] != true;
            }
            if matches!(clean["kind"].as_str(), Some("text" | "result"))
                && let Some(text) = clean["text"].as_str()
            {
                state
                    .recovery_replies
                    .push(core::query::chars(text, memory_draft::TEXT_LIMIT));
            }
        }
    }
    let context_changed = state.accounting.context_if_due().is_some();
    drop(state);
    if usage_changed || context_changed {
        persist_progress(binding, usage_changed, context_changed, false)
    } else {
        Ok(())
    }
}
fn boundary_binding(binding: &RunBinding, signal: &str, details: &Value) -> Result<()> {
    let mut state = locked(binding)?;
    if state.finalized {
        return Ok(());
    }
    let phase = state.phase.clone();
    match signal {
        "guardian_spawned" | "spawned" => {
            state.cleanup_confirmed = false;
            state.accounting.process(&phase, "started");
            if phase == "main" && state.started.is_none() {
                state.started = Some(Instant::now());
                state.started_at =
                    Some(core::domain::clock::proof_now().timestamp_micros() as f64 / 1_000_000.0);
            }
            if phase == "recovery" {
                state.recovery_launched = true;
            }
        }
        "input_written" => {
            let count = details["bytes"]
                .as_u64()
                .and_then(|count| usize::try_from(count).ok());
            if state.provider == "cursor" {
                state.accounting.transport(&phase, None, "not_used");
            } else {
                state.accounting.transport(&phase, count, "written");
            }
        }
        "input_closed" => {}
        "exited" => state
            .accounting
            .boundary(&phase, "guard_exit_before_cleanup", details),
        "cleanup_confirmed" => {
            state.cleanup_confirmed = true;
            state
                .accounting
                .boundary(&phase, "cleanup_confirmed", details);
        }
        "cleanup_unconfirmed" => {
            state.cleanup_confirmed = false;
            state
                .accounting
                .boundary(&phase, "cleanup_unconfirmed", details);
            return Err(HarnessError{status:503,code:"cleanup-unconfirmed",message:"Process cleanup could not be confirmed. The runner is stopping; check native processes before restarting.".into()});
        }
        _ => state.accounting.boundary(&phase, signal, details),
    }
    Ok(())
}
fn persist_progress(
    binding: &RunBinding,
    write_usage: bool,
    write_context: bool,
    write_receipt: bool,
) -> Result<()> {
    let (store, sid, generation, usage, context, receipt) = {
        let state = locked(binding)?;
        let seconds = state
            .started
            .map(|started| (started.elapsed().as_secs_f64() * 1000.0).round_ties_even() / 1000.0);
        (
            state.store.clone(),
            state.sid.clone(),
            state.generation.clone(),
            json!({"tokens":state.tokens,"cost_usd":state.cost,"seconds":seconds,"limit_reached":state.limit,"recovery_cost_unknown":state.cost_unknown}),
            state.accounting.context(),
            state.accounting.receipt(),
        )
    };
    let mut db = store.connection()?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let current = tx
        .query_row(
            "SELECT generation FROM harness_generations WHERE session_id=?1",
            [&sid],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if write_usage && current.as_deref() == Some(&generation) {
        tx.execute(
            "UPDATE sessions SET budget_usage=?1 WHERE id=?2",
            params![wire::python_default(&usage, false, true, true)?, sid],
        )?;
    }
    if write_usage {
        tx.execute(
            "UPDATE launches SET usage=?1 WHERE id=?2 AND session_id=?3",
            params![
                wire::python_default(&usage, false, true, true)?,
                generation,
                sid
            ],
        )?;
    }
    if write_context && let Some(context) = context {
        tx.execute(
            "UPDATE launches SET context=?1 WHERE id=?2 AND session_id=?3",
            params![
                wire::python_default(&context, false, true, true)?,
                generation,
                sid
            ],
        )?;
    }
    if write_receipt {
        let prior = tx
            .query_row(
                "SELECT receipt FROM launches WHERE id=?1 AND session_id=?2",
                params![generation, sid],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        let mut merged = prior
            .as_deref()
            .and_then(|value| wire::parse(value).ok())
            .filter(Value::is_object)
            .unwrap_or(json!({}));
        if let (Some(merged), Some(receipt)) = (merged.as_object_mut(), receipt.as_object()) {
            merged.extend(receipt.clone());
        }
        tx.execute(
            "UPDATE launches SET receipt=?1 WHERE id=?2 AND session_id=?3",
            params![
                wire::python_default(&merged, false, true, true)?,
                generation,
                sid
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}
fn finish_accounting(binding: &RunBinding, outcome: &str) -> Result<Value> {
    let (store, sid, generation, snapshot, already) = {
        let mut state = locked(binding)?;
        let snapshot = state.accounting.finish(outcome);
        let already = state.persisted;
        state.finalized = true;
        (
            state.store.clone(),
            state.sid.clone(),
            state.generation.clone(),
            snapshot,
            already,
        )
    };
    if already {
        return Ok(snapshot);
    }
    let persisted = (|| -> Result<()> {
        wire::serialize(&snapshot, false, false, false, false)?;
        let db = store.connection()?;
        let row = db
            .query_row(
                "SELECT session_id,receipt FROM launches WHERE id=?1",
                [&generation],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()?;
        if let Some((owner, raw)) = row {
            if owner != sid {
                return Err(HarnessError::conflict(
                    "Launch accounting belongs to another session.",
                ));
            }
            let mut receipt = raw
                .as_deref()
                .and_then(|value| wire::parse(value).ok())
                .filter(Value::is_object)
                .unwrap_or(json!({}));
            receipt["full_cost"] = snapshot.clone();
            db.execute(
                "UPDATE launches SET receipt=?1 WHERE id=?2 AND session_id=?3",
                params![
                    wire::python_default(&receipt, false, true, true)?,
                    generation,
                    sid
                ],
            )?;
        } else {
            let event = json!({"kind":"cost_observation","cost_generation":generation,"full_cost":snapshot,"at":crate::store::now()});
            db.execute(
                "INSERT INTO events(session_id,data) VALUES(?1,?2)",
                params![sid, wire::python_default(&event, false, true, true)?],
            )?;
        }
        Ok(())
    })();
    if persisted.is_err() {
        let fallback = json!({"kind":"cost_observation","cost_generation":generation,"full_cost":{"schema":"harness-full-cost-v1","state":"unavailable","observer_incomplete":true,"reason":"persistence_failure"},"at":crate::store::now()});
        if store.event(&sid, &fallback).is_ok() {
            locked(binding)?.persisted = true;
        }
    } else {
        locked(binding)?.persisted = true;
    }
    Ok(snapshot)
}
impl RunnerLifecycle {
    pub fn abort(&self, binding: &RunBinding, outcome: &str) -> Result<Value> {
        let _ = persist_progress(binding, true, true, true);
        finish_accounting(binding, outcome)
    }
    pub async fn finish(
        &self,
        binding: &RunBinding,
        result: &WorkerResult,
        emit: &Event,
        recovery: &mut dyn RecoveryRunner,
    ) -> Result<Value> {
        let _finalization = FinishGuard(binding.clone());
        let (sid, generation, cancel, enabled, native_command, session, cleanup) = {
            let state = locked(binding)?;
            (
                state.sid.clone(),
                state.generation.clone(),
                state.cancel.clone(),
                state.enabled,
                state.native_command,
                state.session.clone(),
                state.cleanup_confirmed,
            )
        };
        let mut outcome = if result.ok { "completed" } else { "failed" };
        if self.store.generation(&sid).ok().as_deref() != Some(&generation) {
            outcome = "superseded";
        } else if cancel.load(Ordering::Acquire) {
            outcome = "cancelled";
        }
        let mut memory = json!({"state":"not-requested","ok":null});
        if outcome == "completed"
            && enabled
            && !native_command
            && session["brain"].is_object()
            && cleanup
        {
            let mut latest = memory_attempts::latest(&self.store, &sid)?;
            if matches!(latest.state.as_str(), "missing" | "unreadable")
                && session["brain"]["review"] == false
            {
                event(
                    &self.store,
                    &sid,
                    &generation,
                    json!({"kind":"memory","ok":true,"recovery":"started","text":"The completed run left no usable memory draft. Trying one read-only follow-up in the same native session."}),
                )?;
                let before = Instant::now();
                {
                    let state = locked(binding)?;
                    state
                        .accounting
                        .new_phase("recovery", state.native_id.as_deref());
                }
                let request = self.recovery_request(binding, result)?;
                let recovered = match request {
                    RecoveryPlan::Run(request) => {
                        let observer: Arc<dyn RunEvents> = Arc::new(binding.clone());
                        let emit = emit.clone();
                        let response = recovery.run(*request, observer, emit).await;
                        self.recovery_result(binding, response)?
                    }
                    RecoveryPlan::Skip { state, reason } => {
                        let observed = locked(binding)?;
                        observed.accounting.process("recovery", "skipped");
                        observed
                            .accounting
                            .boundary("recovery", "recovery_skipped", &Value::Null);
                        json!({"state":state,"reason":reason,"launched":false})
                    }
                };
                locked(binding)?
                    .accounting
                    .duration("recovery", before.elapsed().as_secs_f64());
                current(&self.store, &sid, &generation, &cancel)?;
                let state = recovered["state"].as_str().unwrap_or("failed");
                event(
                    &self.store,
                    &sid,
                    &generation,
                    json!({"kind":"memory","ok":state=="recovered","recovery":state,"text":if state=="recovered"{"Memory draft recovered.".into()}else{recovered["reason"].as_str().unwrap_or("Memory recovery could not finish; the completed work is preserved.").to_string()}}),
                )?;
                if state == "recovered"
                    && let Some(reply) = recovered["reply"].as_str()
                {
                    event(
                        &self.store,
                        &sid,
                        &generation,
                        json!({"kind":"text","text":reply,"memory_recovery":true}),
                    )?;
                }
                persist_progress(binding, true, true, true)?;
                latest = memory_attempts::latest(&self.store, &sid)?;
            }
            if latest.state == "drafted" {
                if let (Some(draft), Some(origin)) = (latest.draft, latest.event_id) {
                    let receipt = locked(binding)?.accounting.receipt();
                    let provenance = memory_draft::usage(
                        &draft,
                        &sources_from_brain(&session["brain"]),
                        receipt.get("memory"),
                    );
                    let started = Instant::now();
                    let result = self.save_automatic(binding, &draft, origin).unwrap_or_else(
                        |error| json!({"ok":false,"saved":{},"skipped":[],"error":error.message}),
                    );
                    locked(binding)?
                        .accounting
                        .duration("save", started.elapsed().as_secs_f64());
                    current(&self.store, &sid, &generation, &cancel)?;
                    let mut notice = memory_draft::summary("drafted", Some(&result));
                    if provenance["delivered"].as_u64().unwrap_or(0) > 0 {
                        notice.push(' ');
                        notice.push_str(&memory_draft::use_notice(&provenance));
                    }
                    event(
                        &self.store,
                        &sid,
                        &generation,
                        json!({"kind":"memory","ok":result["ok"],"text":notice,"delivered_sources":provenance["delivered"],"opened_delivered_sources":provenance["opened"],"opened_observation":provenance["observation"],"reported_used_sources":provenance["reported_used"].as_array().map(Vec::len).unwrap_or(0),"attestation":provenance["attestation"]}),
                    )?;
                    memory = json!({"state":"drafted","ok":result["ok"],"records":result["saved"]["records"].as_array().map(Vec::len).unwrap_or(0),"skipped":result["skipped"].as_array().map(Vec::len).unwrap_or(0)});
                }
            } else if latest.state != "none" {
                event(
                    &self.store,
                    &sid,
                    &generation,
                    json!({"kind":"memory","ok":false,"text":memory_draft::summary(&latest.state,None)}),
                )?;
                memory = json!({"state":latest.state,"ok":false});
            }
            if session["provider"] == "cursor" {
                self.cursor_checkpoint(binding);
            }
        }
        if outcome == "completed"
            && enabled
            && !native_command
            && session["brain"].is_object()
            && !cleanup
        {
            event(
                &self.store,
                &sid,
                &generation,
                json!({"kind":"memory","ok":false,"text":"The native cleanup boundary was not confirmed; no automatic memory writes or recovery were started."}),
            )?;
            memory = json!({"state":"incomplete","ok":false});
        }
        if enabled {
            let started = Instant::now();
            let _ = self
                .knowledge
                .fold_project(session["project_id"].as_str().unwrap_or(""));
            locked(binding)?
                .accounting
                .duration("history", started.elapsed().as_secs_f64());
        }
        for notice in locked(binding)?.accounting.take_events() {
            let _ = event(&self.store, &sid, &generation, notice);
        }
        let _ = persist_progress(binding, true, true, true);
        let full_cost = finish_accounting(binding, outcome)?;
        Ok(
            json!({"memory":memory,"full_cost":full_cost,"provider_billing":"unknown","outcome":outcome}),
        )
    }
    fn recovery_request(
        &self,
        binding: &RunBinding,
        result: &WorkerResult,
    ) -> Result<RecoveryPlan> {
        let mut state = locked(binding)?;
        current(&state.store, &state.sid, &state.generation, &state.cancel)?;
        let native = state
            .native_id
            .clone()
            .or_else(|| result.session_id.clone());
        let Some(native) = native else {
            state
                .accounting
                .boundary("recovery", "recovery_skipped", &Value::Null);
            return Ok(RecoveryPlan::Skip {
                state: "skipped",
                reason: "The provider returned no resumable session identity.",
            });
        };
        if state.limit.is_some() {
            return Ok(RecoveryPlan::Skip {
                state: "skipped",
                reason: "The launch has no remaining budget.",
            });
        }
        for (field, known) in [
            ("tokens", state.tokens.is_some()),
            ("usd", state.cost.is_some()),
        ] {
            if !state.session["budgets"][field].is_null() && !known {
                return Ok(RecoveryPlan::Skip {
                    state: "skipped",
                    reason: "The remaining launch budget could not be measured.",
                });
            }
        }
        let remaining = state.session["budgets"]["seconds"].as_f64().map(|seconds| {
            seconds
                - state
                    .started
                    .map(|started| started.elapsed().as_secs_f64())
                    .unwrap_or(result.duration_seconds)
        });
        if remaining.is_some_and(|seconds| seconds <= 0.0) {
            return Ok(RecoveryPlan::Skip {
                state: "skipped",
                reason: "The launch has no remaining budget.",
            });
        }
        let timeout = Duration::from_secs_f64(remaining.unwrap_or(30.0).min(30.0));
        let usd = state.session["budgets"]["usd"]
            .as_f64()
            .map(|limit| limit - state.cost.unwrap_or(0.0));
        if usd.is_some_and(|budget| budget < 0.01) {
            return Ok(RecoveryPlan::Skip {
                state: "skipped",
                reason: "The remaining monetary budget is below the native minimum.",
            });
        }
        let Some(mut launch) = state.launch.clone() else {
            return Ok(RecoveryPlan::Skip {
                state: "failed",
                reason: "Memory recovery could not start; the completed work is preserved.",
            });
        };
        launch.session = Some(native.clone());
        launch.mode = "plan".into();
        launch.agents = false;
        launch.count = 3;
        launch.effort = None;
        launch.usd = usd;
        launch.hook_events = false;
        launch.prompt = format!(
            "The preceding work has finished. This internal request only repairs its missing memory draft. Do not repeat the user task, edit files, run tests, delegate, or write memory yourself. Use only verified facts from this native session; leave learnings empty when none are supported. Return only the fenced memory-draft block. {}",
            memory_draft::instruction()
        );
        let mut environment = binding.environment.clone();
        environment.insert("CONTEXT_MEMORY_RECOVERY".into(), Some("1".into()));
        environment.insert("CONTEXT_TASK_ID".into(), None);
        environment.insert("CONTEXT_CAPSULE_DELIVERED".into(), None);
        let cancel = Arc::new(AtomicBool::new(false));
        state.recovery_cancel = Some(cancel.clone());
        state.phase = "recovery".into();
        state.recovery_launched = false;
        state.recovery_terminal = false;
        state.recovery_failed = false;
        state.recovery_replies.clear();
        state.cleanup_confirmed = false;
        state.accounting.new_phase("recovery", Some(&native));
        state.accounting.payload("recovery_prompt", &launch.prompt);
        let previous = self.store.get(&state.sid)?["cost_totals"][native.as_str()].as_f64();
        let token_limit = state.session["budgets"]["tokens"]
            .as_u64()
            .map(|limit| limit.saturating_sub(state.tokens.unwrap_or(0)));
        Ok(RecoveryPlan::Run(Box::new(RecoveryRequest {
            launch,
            timeout,
            token_limit,
            output_limit: 512 * 1024,
            extra_env: environment,
            cancel,
            previous_total: previous,
            runner_lock: self.store.runner_lock(),
        })))
    }
    fn recovery_result(
        &self,
        binding: &RunBinding,
        response: Result<WorkerResult>,
    ) -> Result<Value> {
        let state = locked(binding)?;
        let launched = state.recovery_launched;
        if response
            .as_ref()
            .is_err_and(|error| error.code == "cleanup-unconfirmed")
            || launched && !state.cleanup_confirmed
        {
            state
                .accounting
                .boundary("recovery", "cleanup_unconfirmed", &Value::Null);
            return Err(HarnessError{status:503,code:"cleanup-unconfirmed",message:"Process cleanup could not be confirmed. The runner is stopping; check native processes before restarting.".into()});
        }
        state.accounting.process(
            "recovery",
            if !launched {
                "skipped"
            } else if response.as_ref().is_ok_and(|result| result.ok)
                && state.recovery_terminal
                && !state.recovery_failed
            {
                "finished"
            } else {
                "failed"
            },
        );
        let result = if current(&state.store, &state.sid, &state.generation, &state.cancel).is_err()
        {
            json!({"state":"cancelled","reason":"Memory recovery was cancelled; no draft was saved.","launched":launched})
        } else if state.limit.is_some() {
            json!({"state":"failed","reason":"Memory recovery reached the remaining launch budget.","launched":launched})
        } else if response
            .as_ref()
            .is_err_and(|error| error.code == "native-cancelled")
        {
            json!({"state":"cancelled","reason":"Memory recovery was cancelled; no draft was saved.","launched":launched})
        } else if response
            .as_ref()
            .is_err_and(|error| ["native-timeout", "native-time-budget"].contains(&error.code))
        {
            json!({"state":"failed","reason":"Memory recovery reached its bounded time limit.","launched":launched})
        } else if response.as_ref().is_err_and(|error| {
            ["output-limit", "output-line-limit", "native-output-limit"].contains(&error.code)
        }) {
            json!({"state":"failed","reason":"Memory recovery exceeded its output limit.","launched":launched})
        } else if response
            .as_ref()
            .is_err_and(|error| error.message == "Provider changed the original session identity.")
        {
            json!({"state":"failed","reason":"Memory recovery changed the native session identity.","launched":launched})
        } else if response.is_err() && !launched {
            json!({"state":"failed","reason":"Memory recovery could not start; the completed work is preserved.","launched":false})
        } else if response.is_err() {
            json!({"state":"failed","reason":"Memory recovery could not finish; the completed work is preserved.","launched":launched})
        } else if !response.as_ref().is_ok_and(|result| result.ok)
            || !state.recovery_terminal
            || state.recovery_failed
            || !state.cleanup_confirmed
        {
            json!({"state":"failed","reason":"The provider did not complete memory recovery successfully.","launched":launched})
        } else if let Some(reply) = state
            .recovery_replies
            .iter()
            .rev()
            .find(|reply| memory_draft::parse(reply).is_some())
        {
            json!({"state":"recovered","reply":reply,"launched":launched})
        } else {
            state
                .accounting
                .boundary("recovery", "recovery_no_draft", &Value::Null);
            json!({"state":"failed","reason":"The single recovery attempt returned no valid memory draft.","launched":launched})
        };
        if launched {
            let characters=format!("The preceding work has finished. This internal request only repairs its missing memory draft. Do not repeat the user task, edit files, run tests, delegate, or write memory yourself. Use only verified facts from this native session; leave learnings empty when none are supported. Return only the fenced memory-draft block. {}",memory_draft::instruction()).chars().count();
            state
                .accounting
                .set_recovery_context(characters, result["state"].as_str().unwrap_or("failed"));
        }
        Ok(result)
    }
    fn cursor_checkpoint(&self, binding: &RunBinding) {
        let operation = (|| -> Result<()> {
            let (sid, generation, cancel) = {
                let state = locked(binding)?;
                (
                    state.sid.clone(),
                    state.generation.clone(),
                    state.cancel.clone(),
                )
            };
            let session = current(&self.store, &sid, &generation, &cancel)?;
            self.with_memory_operation(&session, &generation, &cancel, |context| {
                current(&self.store, &sid, &generation, &cancel)?;
                let id = string(&session["brain"], "task_id")?;
                let since = locked(binding)?.started_at;
                let now = core::domain::clock::proof_now().timestamp_micros() as f64 / 1_000_000.0;
                if let Some(report) = core::advisory::load_last_turn(context) {
                    let written = report["timestamp"]
                        .as_str()
                        .and_then(|stamp| chrono::DateTime::parse_from_rfc3339(stamp).ok())
                        .map(|stamp| stamp.timestamp_micros() as f64 / 1_000_000.0);
                    if report["schema_version"] == 1
                        && report["task_id"] == id
                        && since.zip(written).is_some_and(|(since, written)| {
                            since.is_finite() && since <= written && written <= now
                        })
                    {
                        return Ok(());
                    }
                }
                let mut service = core::MemoryService::new(context.clone(), None);
                if let Some(engine) = &self.queue_engine {
                    service = service.with_queue_engine(engine.clone())?;
                }
                let accounting = locked(binding)?.accounting.clone();
                measured_core(&accounting, "turn", || {
                    service.turn(core::checkpoint::TurnRequest {
                        task_id: id.clone(),
                        ..Default::default()
                    })
                })?;
                Ok(())
            })
        })();
        let _ = operation;
    }
}

struct FinishGuard(RunBinding);
impl Drop for FinishGuard {
    fn drop(&mut self) {
        let outcome = (|| -> Result<String> {
            let state = locked(&self.0)?;
            if state.persisted {
                return Ok(String::new());
            }
            if state.store.generation(&state.sid).ok().as_deref() != Some(&state.generation) {
                Ok("superseded".into())
            } else if state.cancel.load(Ordering::Acquire) {
                Ok("cancelled".into())
            } else {
                Ok("failed".into())
            }
        })()
        .unwrap_or_else(|_| "failed".into());
        if !outcome.is_empty() {
            let _ = finish_accounting(&self.0, &outcome);
        }
    }
}
fn memory_notice(brain: &Value) -> Result<String> {
    let capsule = &brain["capsule"];
    let mut counts = BTreeMap::from([("brain", 0usize), ("rules", 0), ("bank", 0)]);
    for layer in ["procedural", "semantic", "episodic"] {
        for item in capsule[layer].as_array().into_iter().flatten() {
            let kind = match item["category"].as_str() {
                Some("policy" | "evidence") => "rules",
                Some("durable") => "bank",
                _ => "brain",
            };
            *counts.entry(kind).or_default() += 1;
        }
    }
    let body = brain["capsule_text"]
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .map(|text| text.trim().to_string())
        .unwrap_or(wire::serialize(capsule, false, false, false, false)?);
    let characters = "Project memory (reference data; verify against canonical project sources):\n"
        .chars()
        .count()
        + body.chars().count()
        + 2;
    let digits = characters.to_string();
    let mut formatted = String::new();
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            formatted.push(',');
        }
        formatted.push(character);
    }
    let parts = [
        ("rules", "rule or doc excerpt"),
        ("bank", "Memory Bank chunk"),
        ("brain", "Project Brain item"),
    ]
    .into_iter()
    .filter_map(|(kind, label)| {
        let count = counts[kind];
        (count > 0).then(|| format!("{count} {label}{}", if count == 1 { "" } else { "s" }))
    })
    .collect::<Vec<_>>();
    Ok(format!(
        "Project memory for this turn: task {}, {} ({formatted} characters).",
        brain["task_id"].as_str().unwrap_or(""),
        if parts.is_empty() {
            "its working state only".into()
        } else {
            parts.join(", ")
        }
    ))
}

impl RunnerLifecycle {
    fn retire_cursor_rule(&self, binding: &RunBinding) -> Option<Value> {
        let (session, sid, generation, cancel, needed) = {
            let state = locked(binding).ok()?;
            (
                state.session.clone(),
                state.sid.clone(),
                state.generation.clone(),
                state.cancel.clone(),
                state.provider == "cursor"
                    && !state.native_command
                    && state.session["brain"]["capsule"].is_object(),
            )
        };
        if !needed {
            return None;
        }
        let response = self.with_memory_operation(&session, &generation, &cancel, |context| {
            current(&self.store, &sid, &generation, &cancel)?;
            let accounting = locked(binding)?.accounting.clone();
            let value = measured_core(&accounting, "cursor-rule-retire", || {
                core::cursor_rule::retire(context)
            })?;
            current(&self.store, &sid, &generation, &cancel)?;
            Ok(value)
        });
        Some(match response {
            Ok(value)
                if value["profile"] == "cursor-rule-custody-v1"
                    && value["bytes_removed"]
                        .as_u64()
                        .is_some_and(|bytes| bytes <= 16384)
                    && value["state"].as_str().is_some_and(|state| {
                        ["removed", "preserved", "absent", "attached", "incomplete"]
                            .contains(&state)
                    })
                    && value["bytes_removed"].as_u64().is_some_and(|bytes| {
                        if value["state"] == "removed" {
                            bytes > 0
                        } else if matches!(
                            value["state"].as_str(),
                            Some("preserved" | "absent" | "attached")
                        ) {
                            bytes == 0
                        } else {
                            true
                        }
                    }) =>
            {
                json!({"state":value["state"],"bytes_removed":value["bytes_removed"]})
            }
            _ => json!({"state":"unavailable","bytes_removed":null}),
        })
    }
}

fn measured_core(
    accounting: &LaunchAccounting,
    action: &str,
    work: impl FnOnce() -> core::Result<Value>,
) -> core::Result<Value> {
    let mut measured = accounting.operation("run", Some(action));
    let result = work();
    if let Some(measured) = &mut measured {
        let outcome = result
            .as_ref()
            .ok()
            .map(|value| json!({"ok":true,"result":value}));
        measured.finish(
            outcome.as_ref(),
            if result.is_ok() { "returned" } else { "failed" },
        );
    }
    result
}

struct BeginAccountingGuard {
    store: SessionStore,
    sid: String,
    generation: String,
    cancel: Arc<AtomicBool>,
    accounting: LaunchAccounting,
    armed: bool,
}
impl Drop for BeginAccountingGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let outcome = if self.store.generation(&self.sid).ok().as_deref() != Some(&self.generation)
        {
            "superseded"
        } else if self.cancel.load(Ordering::Acquire) {
            "cancelled"
        } else {
            "failed"
        };
        let snapshot = self.accounting.finish(outcome);
        let persisted = (|| -> Result<()> {
            wire::serialize(&snapshot, false, false, false, false)?;
            let db = self.store.connection()?;
            let row = db
                .query_row(
                    "SELECT session_id,receipt FROM launches WHERE id=?1",
                    [&self.generation],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
                )
                .optional()?;
            if let Some((owner, raw)) = row {
                if owner == self.sid {
                    let mut receipt = raw
                        .as_deref()
                        .and_then(|raw| wire::parse(raw).ok())
                        .filter(Value::is_object)
                        .unwrap_or(json!({}));
                    receipt["full_cost"] = snapshot.clone();
                    db.execute(
                        "UPDATE launches SET receipt=?1 WHERE id=?2 AND session_id=?3",
                        params![
                            wire::python_default(&receipt, false, true, true)?,
                            self.generation,
                            self.sid
                        ],
                    )?;
                }
            } else {
                let event = json!({"kind":"cost_observation","cost_generation":self.generation,"full_cost":snapshot,"at":crate::store::now()});
                db.execute(
                    "INSERT INTO events(session_id,data) VALUES(?1,?2)",
                    params![self.sid, wire::python_default(&event, false, true, true)?],
                )?;
            }
            Ok(())
        })();
        if persisted.is_err() {
            let _=self.store.event(&self.sid,&json!({"kind":"cost_observation","cost_generation":self.generation,"full_cost":{"schema":"harness-full-cost-v1","state":"unavailable","observer_incomplete":true,"reason":"persistence_failure"},"at":crate::store::now()}));
        }
    }
}
