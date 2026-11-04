//! Public application boundary shared by the CLI, hooks and connected clients.
use crate::{
    BankService, BrainRepository, ErrorKind, HandoffService, NativeService, OpenOptions, Result,
    ResultService, ReviewService, RuntimeConfig, RuntimeContext, RuntimeError, Store, TaskService,
    advisory,
    capsule::{self, Capsule, DeliveryProfile},
    checkpoint::{self, TurnRequest},
    continuity,
    domain::{self, clock, json},
    index, query,
    queue::{self, QueueDispatcher, QueuedCommand, QueuedOperation},
    retrieval::{self, RetrievalRequest},
    storage,
};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json as object};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RefreshRequest {
    pub capsule: Option<RetrievalRequest>,
    pub validate: bool,
    pub read_only: bool,
    pub read_only_reason: Option<String>,
    pub host: Option<String>,
    pub task_id: Option<String>,
    pub query_withheld: bool,
    pub provenance_response: bool,
    pub capsule_profile: Option<String>,
    pub delivery_profile: DeliveryProfile,
    pub cursor_continuation: bool,
    pub cursor_topic_eligible: bool,
    /// The caller prints the capsule and records the printed render once its
    /// write succeeds. Otherwise the JSON's `capsule_text` is what reaches
    /// the model, and its render is recorded before the answer is built.
    pub printed: bool,
}
#[derive(Clone, Debug)]
pub struct MemoryService {
    pub context: RuntimeContext,
    pub database: Option<PathBuf>,
    /// The executable that runs queued commands. Admission reads its whole
    /// image twice and starts it once, so it happens only when a turn finds
    /// queued commands, not for every memory command.
    queue_engine: Option<PathBuf>,
    admitted: std::sync::OnceLock<queue::NativeQueueWorker>,
    eval_no_child: bool,
}
impl MemoryService {
    pub fn new(context: RuntimeContext, database: Option<PathBuf>) -> Self {
        Self {
            context,
            database,
            queue_engine: None,
            admitted: std::sync::OnceLock::new(),
            eval_no_child: false,
        }
    }
    pub fn with_queue_engine(mut self, engine: PathBuf) -> Result<Self> {
        self.queue_engine = Some(engine);
        Ok(self)
    }
    fn queue_worker(&self) -> Result<Option<&queue::NativeQueueWorker>> {
        let Some(engine) = &self.queue_engine else {
            return Ok(None);
        };
        if let Some(worker) = self.admitted.get() {
            return Ok(Some(worker));
        }
        let worker = queue::NativeQueueWorker::admit(engine.clone(), &self.context)?;
        Ok(Some(self.admitted.get_or_init(|| worker)))
    }
    pub fn with_eval_no_child(mut self) -> Self {
        self.eval_no_child = true;
        self
    }
    fn no_clockless_child(&self) -> bool {
        self.eval_no_child || domain::clock::is_controlled()
    }
    fn enqueue(&self, command: &QueuedCommand) -> Result<Value> {
        if self.no_clockless_child() {
            return Err(RuntimeError::coded(
                ErrorKind::Unavailable,
                "eval-clock-child-unsupported",
                "The controlled memory clock cannot be handed to a queued worker; no command was queued",
            ));
        }
        // A queued worker is scoped to the default database family. Binding only
        // the file inode would be insufficient: SQLite's journal/WAL names and
        // lightweight task identities belong to the selected database pathname.
        if let Some(selected) = self.database.as_deref() {
            let selected = crate::fs::python_realpath(selected)?;
            let default =
                crate::fs::python_realpath(&storage::default_database(&self.context.roots)?)?;
            if selected != default {
                return Err(RuntimeError::coded(
                    ErrorKind::Unavailable,
                    "queue-database-unsupported",
                    "Deferred memory commands cannot use a custom database; no command was queued",
                ));
            }
        }
        queue::enqueue(&self.context, command)
    }
    pub fn database(&self) -> Result<PathBuf> {
        self.database
            .clone()
            .map(Ok)
            .unwrap_or_else(|| storage::default_database(&self.context.roots))
    }
    pub fn open(&self) -> Result<Store> {
        Store::open(self.database()?, OpenOptions::default())
    }
    pub fn retrieve(&self, request: RetrievalRequest) -> Result<Capsule> {
        let mut store = self.open_recover(true)?;
        retrieval::retrieve(&mut store, &self.context, &request)
    }
    pub fn parity(&self, request: crate::parity::ParityRequest) -> Result<Value> {
        crate::parity::ParityService::new(self.context.clone()).run(request)
    }
    pub fn health(&self, window: Option<usize>) -> Result<Value> {
        advisory::health(&self.context, window)
    }
    pub fn retrieval_report(&self, scope: &str, since: Option<usize>) -> Result<Value> {
        advisory::retrieval_report(&self.context, scope, since)
    }
    fn open_recover(&self, recover: bool) -> Result<Store> {
        self.open_recover_with_diagnosis(recover)
            .map(|(store, _)| store)
    }
    fn open_recover_with_diagnosis(
        &self,
        recover: bool,
    ) -> Result<(Store, Option<(String, String)>)> {
        match self.open() {
            Ok(store) => Ok((store, None)),
            Err(error)
                if recover
                    && error.code.as_deref().is_some_and(|code| {
                        ["sqlite-corrupt", "sqlite-not-a-database"].contains(&code)
                    }) =>
            {
                let database = self.database()?;
                let parent = database
                    .parent()
                    .ok_or_else(|| RuntimeError::invalid("Database has no parent"))?;
                let name = format!(
                    "context-damaged-{}-{}.db",
                    clock::proof_now().format("%Y%m%dT%H%M%SZ"),
                    &domain::uuid4()[..8]
                );
                let target = parent.join(&name);
                for suffix in ["", "-journal", "-wal", "-shm"] {
                    let source = PathBuf::from(format!("{}{suffix}", database.display()));
                    if source.exists() {
                        std::fs::rename(
                            source,
                            PathBuf::from(format!("{}{suffix}", target.display())),
                        )?;
                    }
                }
                self.open()
                    .map(|store| (store, Some((name, error.message))))
            }
            Err(error) => Err(error),
        }
    }
    pub fn status(&self, task_id: Option<&str>) -> Result<Value> {
        let store = self.open()?;
        crate::health::status(&self.context, &store, &self.database()?, task_id)
    }
    pub fn refresh(&self, request: RefreshRequest) -> Result<Value> {
        self.refresh_inner(request, None)
            .map(|outcome| outcome.value)
    }
    pub fn refresh_output(&self, request: RefreshRequest) -> Result<RefreshOutcome> {
        self.refresh_inner(request, None)
    }
    pub fn refresh_with_preparer(
        &self,
        request: RefreshRequest,
        preparer: &mut dyn crate::excerpts::ExcerptPreparer,
    ) -> Result<Value> {
        self.refresh_inner(request, Some(preparer))
            .map(|outcome| outcome.value)
    }
    fn refresh_inner(
        &self,
        mut request: RefreshRequest,
        mut preparer: Option<&mut dyn crate::excerpts::ExcerptPreparer>,
    ) -> Result<RefreshOutcome> {
        let host = request
            .host
            .clone()
            .or_else(|| request.capsule.as_ref().map(|r| r.host.clone()))
            .unwrap_or_else(|| "cli".into());
        let task_id = request
            .task_id
            .clone()
            .or_else(|| request.capsule.as_ref().and_then(|r| r.task_id.clone()));
        let profile = request
            .capsule
            .as_ref()
            .map(|r| r.delivery_profile)
            .unwrap_or(request.delivery_profile);
        if let Some(r) = &request.capsule {
            if r.limit < 1 {
                return Err(RuntimeError::invalid("--limit must be a positive integer"));
            }
            if r.task_id.is_none() {
                return Err(RuntimeError::invalid("--query requires --task-id"));
            }
            if refresh_native_continuation(&host, &r.query) {
                return Ok(RefreshOutcome {
                    value: object!({"native_continuation":true}),
                    capsule: None,
                    profile,
                });
            }
        }
        let started = Instant::now();
        let budget = std::env::var("CONTEXT_HOOK_BUDGET")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|n| n.is_finite() && *n >= 0.0);
        let database = self.database()?;
        let mut degraded = request.read_only;
        let mut quarantine = None;
        let mut store = if request.read_only {
            Store::open_read_only(&database, Duration::from_secs(2)).ok()
        } else {
            match Store::open(&database, OpenOptions::refresh(budget)) {
                Ok(s) => Some(s),
                Err(e)
                    if e.code.as_deref().is_some_and(|c| {
                        ["sqlite-corrupt", "sqlite-not-a-database"].contains(&c)
                    }) =>
                {
                    let (opened, diagnosis) = self.open_recover_with_diagnosis(true)?;
                    quarantine = diagnosis;
                    Some(opened)
                }
                Err(_) => {
                    degraded = true;
                    Store::open_read_only(&database, Duration::from_secs(2)).ok()
                }
            }
        };
        if degraded {
            let warning = if request.read_only_reason.as_deref() == Some("budget") {
                "Memory refresh incomplete: refresh budget exhausted; retrieval was not consulted."
            } else if store.is_some() {
                "Memory refresh incomplete: SQLite writes unavailable; retrieval was not consulted."
            } else {
                "Memory refresh incomplete: local state unavailable; retrieval was not consulted."
            };
            let mut warnings = vec![warning.to_owned()];
            let mut capsule = None;
            if let (Some(store), Some(r)) = (&mut store, &request.capsule) {
                let mut r = r.clone();
                r.refresh_index = false;
                r.capsule_profile = Some("navigation".into());
                r.index_warnings = warnings.clone();
                match retrieval::retrieve(store,&self.context,&r){Ok(c)=>capsule=Some(c),Err(e)if e.code.as_deref()==Some("source-git")=>return Err(e),Err(_)=>warnings.push("Working state unavailable: current identity or eligibility could not be verified.".into())}
            }
            let mut value = object!({"mode":self.context.mode,"procedural":"failed","semantic":"failed","episodic":"failed","startup_profile":"read-only-working-v1","documents":null,"reused":null,"counts":{},"warnings":warnings,"capsule":capsule.as_ref().map(|c|c.0.clone()),"persisted":false});
            if let Some(c) = &capsule {
                value["capsule_text"] = object!(capsule::render(c, profile)?);
            } else if profile.persistent() {
                value["capsule_text"] =
                    object!(format!("working: unavailable\nwarning: {warning}"));
            }
            return Ok(RefreshOutcome {
                value,
                capsule,
                profile,
            });
        }
        let store = store
            .as_mut()
            .ok_or_else(|| RuntimeError::unavailable("Local state unavailable"))?;
        // A failed index does not imply that its prior SQLite rows are current.
        // Retrieval receives the warning before its identity is assembled.
        let mut warnings = Vec::<String>::new();
        let indexed = match index::refresh_index(store, &self.context, true) {
            Ok(v) => Some(v),
            Err(e) if e.code.as_deref() == Some("source-git") => return Err(e),
            Err(e) => {
                warnings.push(format!("Index refresh failed: {e}"));
                None
            }
        };
        let updated = indexed.is_some();
        let mut phases = BTreeMap::from([("stat".to_owned(), 0.0), ("index".to_owned(), 0.0)]);
        if let Some(indexed) = &indexed {
            for (k, v) in &indexed.phase_seconds {
                if v.is_finite() && *v >= 0.0 {
                    phases.insert(k.clone(), *v);
                }
            }
        }
        let source_notices = indexed
            .as_ref()
            .map(|r| r.source_notices.clone())
            .unwrap_or_default();
        let mut capsule = None;
        let mut retrieval_seconds = 0.0;
        let mut withheld = request.query_withheld;
        let mut topic = None;
        if profile.persistent()
            && let Some(id) = &task_id
        {
            topic = Some(continuity::topic_begin(store, &self.context, id));
        }
        if let Some(r) = &mut request.capsule {
            let retrieval_started = Instant::now();
            let raw = r.query.clone();
            // Whether a Cursor prompt only continues the conversation's topic,
            // decided on the raw prompt before it is cleaned, as the retired
            // runtime decided it: a redacted prompt is never a continuation,
            // so redaction cannot turn a sensitive request into a topic lookup.
            // Without this a bare "yes" or "давай" searched by itself and the
            // re-sent rule lost the knowledge of the turn it answers.
            let unchanged = r.sanitize
                && domain::sanitize_automatic_query(&raw).is_ok_and(|cleaned| {
                    cleaned == raw.trim_matches(|c: char| " \t\r\n,;".contains(c))
                });
            let cursor_continuation = request.cursor_continuation
                || (unchanged
                    && host == "cursor"
                    && continuity::continuation(&raw, profile.persistent()));
            let cursor_topic_eligible = request.cursor_topic_eligible
                || (unchanged
                    && profile.persistent()
                    && raw.chars().count() <= 1024
                    && !["do not continue", "не продолжай", "стоп", "stop"].contains(
                        &query::folded(&raw).trim_matches(|c: char| " \t\r\n,.!?;".contains(c)),
                    ));
            let mut query_source = "prompt".to_owned();
            r.refresh_index = false;
            r.entry_point = "refresh".into();
            r.host = host.clone();
            r.allow_unprovisioned = true;
            r.automatic_links = true;
            r.index_warnings = warnings.clone();
            if self.context.mode == "governed"
                && let Some((name, damage)) = &quarantine
            {
                r.index_warnings.push(format!("The memory database was damaged ({damage}) and was rebuilt; the old one is memory-bank/local/{name}."));
            }
            r.source_notices = source_notices.clone();
            r.phase_seconds = phases.clone();
            r.history_text = Some(raw.clone());
            let attempt = (|| -> Result<Option<Capsule>> {
                domain::guard_shared_text("Task Capsule", std::slice::from_ref(&raw))?;
                if r.sanitize {
                    r.query = domain::sanitize_automatic_query(&raw)?;
                    r.sanitize = false;
                }
                if r.query.trim().is_empty() {
                    withheld = true;
                    return Ok(None);
                }
                // The prompt is distilled to its informative terms; a
                // continuation then asks with its topic or its task instead,
                // undistilled, as the retired runtime did.
                if warnings.is_empty() {
                    r.query = query::distill_query(&store.conn, &r.query)?;
                }
                if cursor_continuation {
                    if profile.persistent() {
                        if let (Some(plan), Some(id)) = (&topic, &task_id)
                            && let Some((query, source)) =
                                continuity::topic_query(store, &self.context, id, plan)
                        {
                            r.query = query;
                            query_source = source;
                        }
                    } else if self.context.mode == "governed"
                        && let Some(id) = &task_id
                        && let Some(uuid) = checkpoint::served_uuid(Some(store), &self.context, id)?
                        && let Some((query, source)) = continuation_query(&self.context, id, &uuid)?
                    {
                        r.query = query;
                        query_source = source;
                    }
                }
                r.query_source = query_source.clone();
                let c = if let Some(preparer) = preparer.take() {
                    retrieval::retrieve_with_preparer(store, &self.context, r, preparer)?
                } else {
                    retrieval::retrieve(store, &self.context, r)?
                };
                Ok(Some(c))
            })();
            match attempt{Ok(c)=>capsule=c,Err(e)if e.code.as_deref()==Some("source-git")=>return Err(e),Err(e)=>warnings.push(format!("Capsule unavailable: {e}. No task context was assembled this turn; read the canonical sources directly."))}
            if self.context.mode == "governed"
                && let Some(c) = &capsule
            {
                warnings.extend(
                    c.0["warnings"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(String::from),
                );
            }
            warnings = domain::unique_strings(warnings);
            if let (Some(plan), Some(id)) = (&topic, &task_id) {
                let selected = capsule.as_ref().is_some_and(|c| {
                    ["procedural", "semantic", "episodic"]
                        .iter()
                        .any(|l| c.0[*l].as_array().is_some_and(|v| !v.is_empty()))
                });
                continuity::topic_finish(
                    store,
                    &self.context,
                    id,
                    plan,
                    &r.query,
                    cursor_topic_eligible && capsule.is_some(),
                    &warnings,
                    query_source == "continuation",
                    cursor_continuation,
                    selected,
                );
            }
            retrieval_seconds =
                (retrieval_started.elapsed().as_secs_f64() * 1_000_000.0).round() / 1_000_000.0;
        } else if let Some(plan) = &topic {
            continuity::topic_write(store, plan.expected.as_deref(), None);
        }
        phases.insert("retrieval".into(), retrieval_seconds);
        let documents = match &indexed {
            Some(r) => r.documents,
            None => store
                .conn
                .query_row("SELECT COUNT(*) FROM documents", [], |r| {
                    r.get::<_, usize>(0)
                })?,
        };
        let overdue = indexed.as_ref().map_or(0, |r| {
            r.excluded
                .iter()
                .filter(|e| e.reason == "overdue-review")
                .count()
        });
        let mut result = object!({"mode":self.context.mode,"procedural":if updated{"updated"}else{"failed"},"semantic":if updated{"updated"}else{"failed"},"episodic":if updated{"updated"}else{"failed"},"codebase":codebase_status(store,&self.context)?,"documents":documents,"reused":indexed.as_ref().map_or(0,|r|r.reused),"counts":indexed.as_ref().map(|r|r.layers.clone()).unwrap_or_default(),"overdue_review":overdue,"parity_drift":indexed.as_ref().map(|r|r.parity_drift.clone()).unwrap_or_default(),"phases":phases,"warnings":warnings,"capsule":capsule.as_ref().map(|c|c.0.clone())});
        if !source_notices.is_empty() {
            result["source_notices"] = object!(source_notices);
        }
        if withheld {
            result["query_withheld"] = object!(true);
        }
        if request.provenance_response
            || request
                .capsule
                .as_ref()
                .is_some_and(|r| r.provenance_response)
        {
            result["response_profile"] = object!("local-source-provenance-v1");
            result["source_provenance"] = capsule
                .as_ref()
                .and_then(|c| c.2.clone())
                .unwrap_or(Value::Null);
        }
        let mut review_stage = None;
        if self.context.mode == "governed"
            && !withheld
            && let (Some(c), Some(id)) = (&capsule, &task_id)
        {
            let service = ReviewService::new(self.context.clone());
            let images =
                c.1.as_ref()
                    .map(|p| {
                        p.images
                            .iter()
                            .map(|(path, image)| (path.clone(), image.content.clone()))
                            .collect::<BTreeMap<_, _>>()
                    })
                    .unwrap_or_default();
            let _ = service.observe_encounters(&c.0, id, &images);
            review_stage = service.notice(id).ok().flatten();
        }
        if let Some(review) = &review_stage {
            result["memory_review"] = object!(review);
        }
        // The capsule as its render plan writes it, without the review notice:
        // the render a trace records and the funnel diagnosis checks.
        let mut delivered = None;
        if let Some(c) = &capsule {
            let plain = capsule::render(c, profile)?;
            let mut rendered = plain.clone();
            if let Some(review) = &review_stage {
                rendered.push('\n');
                rendered.push_str(review);
            }
            result["capsule_text"] = object!(rendered);
            if !request.printed {
                // The answer carries this text to the model (a Cursor rule,
                // Memory MCP, the Harness), so it is the emitter's render, as
                // the retired runtime recorded it before answering.
                crate::observability::record_render(&self.context, c, &plain, "capsule_text");
            }
            delivered = Some(plain);
        } else if profile.persistent() {
            result["capsule_text"] = object!(
                "working: unavailable\nwarning: memory refresh incomplete; verify current canonical sources"
            );
        }
        if request.validate {
            let validation = self.validate()?;
            let valid = validation["valid"] == true;
            result["brain_validation"] = object!(if valid { "valid" } else { "invalid" });
            warnings.extend(
                validation["errors"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(String::from),
            );
            result["warnings"] = object!(warnings);
        }
        advisory::telemetry(&self.context, capsule.as_ref(), retrieval_seconds);
        let mut health = object!({"at":domain::utc_now(),"mode":self.context.mode,"phase_seconds":phases,"omitted":capsule.as_ref().map(|c|c.0["omitted"].clone()),"layers_updated":updated,"warnings":warnings.len(),"source":"refresh","host":host});
        if let Some(c) = &capsule {
            let rendered = result["capsule_text"].as_str();
            health["capsule"] = refresh_capsule_shape(
                &self.context,
                c,
                request.capsule_profile.as_deref().or_else(|| {
                    request
                        .capsule
                        .as_ref()
                        .and_then(|r| r.capsule_profile.as_deref())
                }),
                rendered,
            );
            if let Some(rendered) = &delivered {
                let expected = object!({"sha256":query::sha256(rendered.as_bytes()),"bytes":rendered.len(),"form":"capsule_text"});
                let (diagnosis, receipt) =
                    crate::observability::diagnose_with_receipt(&self.context, c, &expected);
                if crate::observability::valid_funnel_diagnosis(&diagnosis) {
                    health["funnel_diagnosis"] = diagnosis.clone();
                    result["funnel_diagnosis"] = diagnosis;
                }
                if !receipt.is_null() {
                    health["ordinary_receipt"] = receipt.clone();
                    result["ordinary_receipt"] = receipt;
                }
            }
        } else {
            if request.capsule.is_some() {
                health["capsule"] = Value::Null;
            }
            // No capsule, nothing to observe: the record says so, as the
            // retired one did, instead of reading as one whose diagnosis is
            // missing.
            let diagnosis = crate::observability::unknown_funnel_diagnosis();
            health["funnel_diagnosis"] = diagnosis.clone();
            result["funnel_diagnosis"] = diagnosis;
            let receipt = crate::observability::unknown_ordinary_receipt();
            health["ordinary_receipt"] = receipt.clone();
            result["ordinary_receipt"] = receipt;
        }
        if !source_notices.is_empty() {
            health["source_notices"] = object!(source_notices);
        }
        advisory::append_refresh(&self.context, &health);
        let _ = started;
        Ok(RefreshOutcome {
            value: result,
            capsule,
            profile,
        })
    }
    pub fn turn(&self, request: TurnRequest) -> Result<Value> {
        if self.no_clockless_child() {
            if queue::has_waiting(&self.context)? {
                return Err(RuntimeError::coded(
                    ErrorKind::Unavailable,
                    "eval-clock-child-unsupported",
                    "Queued memory commands require a native child; the controlled-clock turn was not started",
                ));
            }
            return self.turn_direct(request);
        }
        // Commands already queued admit the worker before the turn, so an
        // unavailable worker still refuses before any work. Ones queued by
        // another process during the turn are drained when the worker admits,
        // and otherwise stay queued as without a worker.
        let worker = if queue::has_waiting(&self.context).unwrap_or(true) {
            self.queue_worker()?
        } else {
            None
        };
        let mut result = self.turn_direct(request)?;
        let worker = match worker {
            Some(worker) => Some(worker),
            None if queue::has_waiting(&self.context).unwrap_or(true) => {
                self.queue_worker().ok().flatten()
            }
            None => None,
        };
        let queued = queue::drain_configured(&self.context, worker)?;
        if !queued.is_empty() {
            result["queued_commands"] = object!(queued);
        }
        Ok(result)
    }
    /// The capsule a hook asks for without a prompt, chosen as the retired
    /// runtime chose it. Cursor's rule follows the conversation's current
    /// topic first; then a lightweight task or a governed one (bound, or held
    /// by the Brain before its first checkpoint) asks with its own state. A
    /// branch with no task yet is never searched by its name: without a
    /// change it has no context, and Cursor retires a stale rule on that
    /// outcome; with one, it gets the warming notice alone.
    fn hook_context(&self, request: &Value) -> Result<(HookContext, DeliveryProfile)> {
        let mut store = self.open_recover(true)?;
        let mut retrieval = request_retrieval(request)?;
        let id = required(request, "task_id")?.to_string();
        let profile = retrieval.delivery_profile;
        retrieval.entry_point = "hook-context".into();
        retrieval.limit = 3;
        retrieval.ephemeral = true;
        let mut chosen = None;
        if profile.persistent() {
            let plan = continuity::topic_begin(&store, &self.context, &id);
            if let Some(found) = continuity::topic_query(&store, &self.context, &id, &plan) {
                retrieval.allow_unprovisioned = true;
                retrieval.automatic_links = true;
                chosen = Some(found);
            }
        }
        if chosen.is_none() {
            let provisioned = if self.context.mode == "lightweight" {
                store.working(&id)?.is_some()
            } else {
                checkpoint::served_uuid(Some(&store), &self.context, &id)?.is_some()
            };
            if provisioned {
                if self.context.mode != "lightweight" {
                    retrieval.allow_unprovisioned = store.binding(&id)?.is_none();
                    retrieval.automatic_links = true;
                }
                chosen = Some(hook_query(&store, &self.context, &id)?);
            }
        }
        let Some((query, source)) = chosen else {
            let (files, excluded) = checkpoint::changed_paths(&self.context)?;
            if files.is_empty() {
                return Ok((HookContext::Nothing, profile));
            }
            let pending = store.pending_turns(&id)?.len();
            return Ok((
                HookContext::Capsule(Capsule::new(object!({
                    "kind":"warming","task_id":id,"working":null,
                    "procedural":[],"semantic":[],"episodic":[],
                    "changed_files":files.len(),"excluded_files":excluded.len(),"pending_turns":pending,
                    "warnings":["Governed task not provisioned yet; authoritative persistence starts at the 5-turn boundary."]
                }))),
                profile,
            ));
        };
        retrieval.query = query;
        retrieval.query_source = source;
        let capsule = retrieval::retrieve(&mut store, &self.context, &retrieval)?;
        let gate = &capsule.0["gate"];
        if gate["mode"] == "enforce"
            && gate["decision"] == "skip"
            && !(profile.persistent()
                && ["no-relevant-match", "empty-after-filter"]
                    .contains(&gate["reason"].as_str().unwrap_or("")))
        {
            return Ok((HookContext::Withheld, profile));
        }
        Ok((HookContext::Capsule(capsule), profile))
    }
    fn turn_direct(&self, request: TurnRequest) -> Result<Value> {
        let mut store = self.open_recover(true)?;
        checkpoint::run_turn(&mut store, &self.context, &request)
    }
    pub fn validate(&self) -> Result<Value> {
        let brain = BrainRepository::new(self.context.clone())?;
        let inventory = brain.inventory(true)?;
        let mut errors = inventory
            .problems
            .iter()
            .map(|problem| format!("{}: {}", problem.path.display(), problem.error))
            .collect::<Vec<_>>();
        errors.extend(BankService::new(self.context.clone()).validate()?);
        let ids = inventory
            .records
            .iter()
            .filter_map(|image| image.record["id"].as_str())
            .collect::<std::collections::BTreeSet<_>>();
        for image in &inventory.records {
            for field in ["conflicts", "supersedes"] {
                for id in image.record[field]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    if !ids.contains(id) {
                        errors.push(format!(
                            "{}: unknown {field} record {id}",
                            image.path.display()
                        ));
                    }
                }
            }
        }
        let handoffs = self.context.roots.brain_root().join("control/handoffs");
        if handoffs.is_dir() {
            for entry in std::fs::read_dir(handoffs)? {
                let path = entry?.path();
                if path.extension().is_none_or(|extension| extension != "md") {
                    continue;
                }
                let checked = (|| -> Result<()> {
                    let (handoff, _) =
                        domain::parse_markdown_record(&crate::fs::read_text(&path)?)?;
                    let task = brain.get_task(handoff["task_id"].as_str().unwrap_or(""))?;
                    domain::validate_handoff(&handoff, Some(&task))
                })();
                if let Err(error) = checked {
                    errors.push(format!("{}: {error}", path.display()));
                }
            }
        }
        // A record whose cited file changed is intact: retrieval serves it
        // marked for checking. It is listed, archived ones too, and does not
        // fail validation the way a broken record does.
        let mut stale = Vec::new();
        let mut states = Vec::new();
        for image in &inventory.records {
            let mut record = image.record.clone();
            if record.get("source_fingerprints").is_none() {
                record["source_fingerprints"] = object!([]);
            }
            // A record whose freshness cannot be read is not listed.
            if let Ok(false) = domain::sources_are_fresh(&self.context.roots, &record) {
                let state = domain::citation_problem(&self.context.roots, &record)
                    .ok()
                    .flatten()
                    .unwrap_or("changed");
                let path = image.path.display().to_string();
                states.push(object!({"path":path,"state":state}));
                stale.push(path);
            }
        }
        Ok(
            object!({"valid":errors.is_empty(),"records":inventory.records.len(),"errors":errors,"stale":stale,"stale_states":states}),
        )
    }
    pub fn execute(&self, operation: &str, request: &Value, input: &[u8]) -> Result<Value> {
        let operation = operation.replace('_', "-");
        let brain = BrainRepository::new(self.context.clone())?;
        let bank = BankService::new(self.context.clone());
        let results = ResultService::new(self.context.clone());
        let task = || required(request, "task_id");
        let id = || required(request, "record_id");
        let request = request.clone();
        match operation.as_str() {
            "status" => self.status(request["task_id"].as_str()),
            "health" => self.health(number(&request, "window")),
            "retrieval-report" => self.retrieval_report(
                text(&request, "scope").unwrap_or("all"),
                number(&request, "since"),
            ),
            "index" => {
                let mut store = self.open_recover(true)?;
                Ok(serde_json::to_value(index::refresh_index(
                    &mut store,
                    &self.context,
                    flag(&request, "incremental"),
                )?)?)
            }
            "search" => {
                let mut store = self.open()?;
                index::refresh_index(&mut store, &self.context, true)?;
                let query = required(&request, "query")?;
                let documents = query::search(
                    &store,
                    &self.context,
                    &query::SearchRequest {
                        query: query.into(),
                        limit: number(&request, "limit").unwrap_or(8),
                        layer: text(&request, "layer").map(String::from),
                    },
                )?;
                let episodes = if text(&request, "layer").is_none_or(|layer| layer == "episodic") {
                    query::search_episodes(
                        &store.conn,
                        query,
                        number(&request, "limit").unwrap_or(8),
                    )?
                } else {
                    vec![]
                };
                Ok(object!({"query":query,"documents":documents,"episodes":episodes}))
            }
            "context" | "retrieve" => {
                let mut retrieval = request_retrieval(&request)?;
                retrieval.entry_point = operation;
                retrieval.capsule_profile = Some("full".into());
                retrieval.allow_unprovisioned = true;
                let capsule = self.retrieve(retrieval)?;
                if flag(&request, "provenance_response") {
                    Ok(
                        object!({"response_profile":"local-source-provenance-v1","capsule":capsule.0,"source_provenance":capsule.2}),
                    )
                } else {
                    Ok(capsule.0)
                }
            }
            "refresh" => self.refresh(request_refresh(&request)?),
            "hook-context" => match self.hook_context(&request)? {
                (HookContext::Capsule(capsule), profile) => {
                    if flag(&request, "json") {
                        Ok(capsule.0)
                    } else {
                        Ok(envelope(
                            capsule::render(&capsule, profile)? + "\n",
                            capsule.0,
                            0,
                        ))
                    }
                }
                (HookContext::Nothing, _) => Ok(envelope(String::new(), Value::Null, 3)),
                (HookContext::Withheld, _) => Ok(envelope(String::new(), Value::Null, 4)),
            },
            "get" => {
                let store = self.open()?;
                let id = task()?;
                if self.context.mode == "lightweight" {
                    Ok(serde_json::to_value(store.working(id)?.ok_or_else(
                        || RuntimeError::not_found(format!("Working task not found: {id}")),
                    )?)?)
                } else {
                    let binding = store.binding(id)?.ok_or_else(|| unbound(id))?;
                    Ok(task_view(&brain.get_task(&binding.task_uuid)?))
                }
            }
            "start" => {
                let mut store = self.open()?;
                let id = required(&request, "task_id")?;
                let goal = required(&request, "goal")?;
                if self.context.mode == "lightweight" {
                    Ok(serde_json::to_value(store.start(storage::StartTask {
                        task_id: id.into(),
                        goal: goal.into(),
                        files: list(&request, "files", "file")?,
                        sources: list(&request, "sources", "source")?,
                    })?)?)
                } else {
                    let _guard = brain.lock()?;
                    let record = brain.create_task(
                        id,
                        goal,
                        list(&request, "files", "file")?,
                        list(&request, "sources", "source")?,
                        "team",
                    )?;
                    if let Err(error) = store.bind(
                        id,
                        record["id"].as_str().unwrap_or(""),
                        record["revision"].as_u64().unwrap_or(1),
                    ) {
                        brain.rollback_created(record["id"].as_str().unwrap_or(""))?;
                        return Err(error);
                    }
                    Ok(task_view(&record))
                }
            }
            "rebind" => {
                let mut store = self.open()?;
                checkpoint::rebind(
                    &mut store,
                    &self.context,
                    task()?,
                    text(&request, "record_id").or(text(&request, "task_uuid")),
                )
            }
            "update" => self.update(&request),
            "clear" => {
                let mut store = self.open()?;
                let id = task()?;
                if self.context.mode == "lightweight" {
                    store.clear(id)?;
                } else {
                    let binding = store.binding(id)?.ok_or_else(|| unbound(id))?;
                    let snapshot = brain.snapshot_record(&binding.task_uuid)?;
                    brain.cancel_task(&binding.task_uuid)?;
                    if let Err(error) = store.remove_binding(&binding.external_id) {
                        brain.restore_record(&snapshot)?;
                        return Err(error);
                    }
                }
                Ok(object!({"task_id":id}))
            }
            "complete" => self.complete(&request),
            // A standalone completed-work episode in local SQLite, in either
            // mode: no Brain record, handoff or promotion (docs/OPERATIONS.md).
            "record" => {
                let mut store = self.open()?;
                let episode = store.record_episode(storage::EpisodeInput {
                    summary: required(&request, "summary")?.into(),
                    outcome: required(&request, "outcome")?.into(),
                    files: list(&request, "files", "file")?,
                    verification: list(&request, "verification", "verification")?,
                    sources: list(&request, "sources", "source")?,
                })?;
                Ok(object!({"episode_id":episode}))
            }
            "turn" => {
                let request = TurnRequest {
                    task_id: task()?.into(),
                    flush: flag(&request, "flush"),
                    flush_after: number(&request, "flush_after").unwrap_or(5),
                    max_files: number(&request, "max_files").unwrap_or(20),
                    transcript: text(&request, "transcript").map(PathBuf::from),
                };
                if request.flush
                    && std::env::var("CONTEXT_QUEUE_DRAIN").as_deref() != Ok("1")
                    && !queue::writable(&self.context, &self.database()?)?
                {
                    return self.enqueue(&QueuedCommand {
                        owner: self.context.actor.clone(),
                        mode: Some(self.context.mode.clone()),
                        project: Some(self.context.roots.project.clone()),
                        operation: QueuedOperation::Flush {
                            task_id: request.task_id,
                            max_files: request.max_files,
                        },
                    });
                }
                self.turn(request)
            }
            "source-fingerprints" => Ok(
                object!({"source_fingerprints":results.checked_fingerprints(&list(&request,"sources","source")?)?}),
            ),
            "record-result" | "prepare-result" | "replay-result" => {
                let data = payload(&request, input)?;
                let revision = revision(&request)?;
                let reason = text(&request, "reason").map(String::from).unwrap_or_else(|| {
                    if text(&request, "attestation") == Some("person") {
                        "Saved with infra-engine memory record-result; reviewed by a person".into()
                    } else {
                        "Saved with infra-engine memory record-result; agent-attested, not reviewed by a person".into()
                    }
                });
                let attestation = text(&request, "attestation").unwrap_or("agent").to_string();
                if operation == "prepare-result" {
                    let crate::Revision::Exact(revision) = revision else {
                        return Err(RuntimeError::invalid(
                            "Preparation requires a numeric revision",
                        ));
                    };
                    results.prepare(crate::PrepareResult {
                        task_id: task()?.into(),
                        result_id: required(&request, "result_id")?.into(),
                        revision,
                        data,
                        reason,
                        attestation,
                    })
                } else {
                    results.record(crate::RecordResult {
                        task_id: task()?.into(),
                        result_id: required(&request, "result_id")?.into(),
                        revision,
                        data,
                        reason,
                        attestation,
                        prepared: text(&request, "prepared_scope").map(String::from),
                        replay_only: operation == "replay-result",
                    })
                }
            }
            "native-prompt" | "native-stop" => {
                let native = NativeService::new(self.context.clone());
                let host = host(required(&request, "host")?)?;
                let payload = crate::native::parse_input(input)?;
                let mut store = self.open_recover(true)?;
                let mut tasks = TaskService::new(self.context.clone(), Some(&mut store));
                native.drain(
                    crate::NativeDrain {
                        task_id: task()?.into(),
                        host,
                        payload: payload.clone(),
                    },
                    &mut tasks,
                )?;
                let value = if operation == "native-prompt" {
                    native.begin(task()?, host, &payload)?
                } else {
                    native.stop(
                        crate::NativeStop {
                            task_id: task()?.into(),
                            host,
                            payload,
                            entered: flag(&request, "entered"),
                        },
                        &mut tasks,
                    )?
                };
                let cursor = matches!(host, crate::Host::Cursor);
                if operation == "native-prompt" && cursor && value["warning"].is_string() {
                    // Cursor's hook answers in JSON only, so its rule is where
                    // the notice goes: the next capsule's last-turn line says
                    // the previous result was not saved completely, as the
                    // retired runtime wrote it. Claude Code's and Codex's hooks
                    // print the warning line themselves.
                    let mut report = crate::advisory::load_last_turn(&self.context)
                        .unwrap_or_else(|| object!({"schema_version":1}));
                    report["timestamp"] = object!(domain::utc_now());
                    report["task_id"] = object!(task()?);
                    report["native_capture_incomplete"] = object!(true);
                    crate::advisory::write_last_turn(&self.context, &report);
                }
                if flag(&request, "json") {
                    Ok(value)
                } else {
                    let stdout = if operation == "native-prompt" {
                        value["warning"]
                            .as_str()
                            .filter(|_| !cursor)
                            .map(|warning| format!("{warning}\n"))
                            .unwrap_or_default()
                    } else if value.as_object().is_some_and(|object| !object.is_empty()) {
                        json::python_default(&value, false, false, true)? + "\n"
                    } else {
                        String::new()
                    };
                    Ok(envelope(stdout, value, 0))
                }
            }
            "native-result" => {
                let native = NativeService::new(self.context.clone());
                let mut store = self.open()?;
                let mut tasks = TaskService::new(self.context.clone(), Some(&mut store));
                native.save(
                    crate::NativeSave {
                        key: required(&request, "request")?.into(),
                        token: required(&request, "token")?.into(),
                        data: if flag(&request, "skip") {
                            Value::Null
                        } else {
                            payload(&request, input)?
                        },
                        skip: flag(&request, "skip"),
                        spooled: false,
                    },
                    &mut tasks,
                )
            }
            "brain-get" => brain.get(id()?),
            "brain-create" => {
                let kind = required(&request, "type")?;
                let authority = text(&request, "authority").unwrap_or("observed");
                brain.create(crate::CreateRecord {
                    record_type: kind.into(),
                    external_id: required(&request, "external_id")?.into(),
                    title: required(&request, "title")?.trim().into(),
                    goal: text(&request, "goal").map(String::from),
                    files: list(&request, "files", "file")?,
                    sources: list(&request, "sources", "source")?,
                    conflicts: list(&request, "conflicts", "conflict")?,
                    privacy: text(&request, "privacy").unwrap_or("team").into(),
                    authority: authority.into(),
                    confidence: request["confidence"].as_f64().unwrap_or(1.0),
                    attestation: if authority == "verified" {
                        Some(text(&request, "attestation").unwrap_or("agent").into())
                    } else {
                        None
                    },
                    ..Default::default()
                })
            }
            "brain-update" => {
                let record = brain.get(id()?)?;
                let verifying =
                    text(&request, "authority") == Some("verified") || flag(&request, "reverify");
                let mut updated = brain.update(
                    id()?,
                    crate::UpdateRecord {
                        expected_revision: expected(&request, &record)?,
                        progress: text(&request, "progress").map(|value| value.trim().into()),
                        next_steps: list(&request, "next_steps", "next_step")?,
                        replace_next_steps: flag(&request, "replace_next_steps"),
                        files: list(&request, "files", "file")?,
                        sources: list(&request, "sources", "source")?,
                        conflicts: if field_supplied(&request, &["conflict", "conflicts"]) {
                            Some(list(&request, "conflicts", "conflict")?)
                        } else {
                            None
                        },
                        transition_to: text(&request, "transition").map(String::from),
                        phase: text(&request, "phase").map(String::from),
                        authority: text(&request, "authority").map(String::from),
                        reverify: flag(&request, "reverify"),
                        attestation: text(&request, "attestation")
                            .map(String::from)
                            .or_else(|| verifying.then(|| "agent".into())),
                        reason: text(&request, "reason").unwrap_or("Record updated").into(),
                        ..Default::default()
                    },
                )?;
                if let Some(promotion) = promote_on_resolution(&bank, &updated) {
                    updated["promotion"] = promotion;
                }
                Ok(updated)
            }
            "memory-review" => {
                let review = ReviewService::new(self.context.clone());
                if text(&request, "input").is_some()
                    || !input.is_empty()
                    || request.get("data").is_some_and(Value::is_object)
                    || request.get("request").is_some_and(Value::is_object)
                {
                    review.apply(task()?, &payload(&request, input)?)
                } else {
                    review.plan(task()?)
                }
            }
            "memory-review-controller" => {
                let review = ReviewService::new(self.context.clone());
                let proposal = required(&request, "proposal_id")?;
                let digest = required(&request, "proposal_sha")?;
                if required(&request, "action")? == "export" {
                    review.semantic_export(task()?, proposal, digest)
                } else {
                    let packet =
                        review.controller_input(Path::new(required(&request, "packet")?))?;
                    let authorization =
                        review.controller_input(Path::new(required(&request, "authorization")?))?;
                    review.semantic_apply(task()?, proposal, digest, &packet, &authorization)
                }
            }
            "validate" | "brain-validate" => self.validate(),
            "compact" => bank.compact(None),
            "consolidate" => bank.consolidate(flag(&request, "apply"), text(&request, "ticket")),
            "bank-audit" => bank.audit(flag(&request, "provenance")),
            "bank-reverify" => bank.reverify(
                required(&request, "memory_id")?,
                text(&request, "review_after"),
            ),
            "bank-retire" => bank.retire(
                required(&request, "memory_id")?,
                required(&request, "valid_to")?,
                text(&request, "superseded_by"),
                text(&request, "reason"),
            ),
            "bank-quarantine" => {
                bank.quarantine(required(&request, "memory_id")?, text(&request, "reason"))
            }
            "reindex-bank" => bank.index(),
            "promote-propose" => bank.propose(
                &list(&request, "source_ids", "source_id")?,
                required(&request, "title")?,
                required(&request, "content")?,
                false,
            ),
            "promote-review" => bank.review(
                required(&request, "promotion_id")?,
                text(&request, "reviewer"),
                !flag(&request, "reject"),
            ),
            "promote-apply" => bank.apply(required(&request, "promotion_id")?),
            "promote-auto" => bank.auto_promote(
                crate::bank::promotion::AUTOMATIC_PROMOTION_LIMIT,
                None,
                None,
            ),
            "export" => bank.export(
                Path::new(required(&request, "destination")?),
                flag(&request, "include_archive"),
                flag(&request, "include_superseded"),
                flag(&request, "force"),
            ),
            "msg-send" => {
                let task = TaskService::new(self.context.clone(), None).task(task()?, true)?;
                brain.append_message(
                    task["id"].as_str().unwrap_or(""),
                    crate::MessageInput {
                        from_actor: required(&request, "from_actor")?.into(),
                        to_actor: required(&request, "to_actor")?.into(),
                        message_type: required(&request, "type")?.into(),
                        body: required(&request, "body")?.into(),
                        refs: list(&request, "refs", "ref")?,
                        capsule_digest: text(&request, "capsule_digest").map(String::from),
                    },
                )
            }
            "msg-read" => Ok(object!(brain.read_messages(
                task()?,
                text(&request, "for_actor"),
                request["since_seq"].as_u64().unwrap_or(0),
                text(&request, "type")
            )?)),
            "context-save" => HandoffService::new(self.context.clone()).save(crate::SaveHandoff {
                input: payload(&request, input)?,
                output: PathBuf::from(required(&request, "output")?),
                detail: required(&request, "detail")?.into(),
                topic: text(&request, "topic").map(String::from),
                task_id: text(&request, "task_id").map(String::from),
                source_client: required(&request, "source_client")?.into(),
                transcript: text(&request, "transcript").map(PathBuf::from),
            }),
            "context-load" => HandoffService::new(self.context.clone()).load(
                Path::new(required(&request, "input")?),
                flag(&request, "include_transcript"),
            ),
            "links" => self.links(&request),
            "parity" => self.parity(crate::parity::ParityRequest {
                skills_only: flag(&request, "skills_only"),
                cross_edition: flag(&request, "cross_edition"),
            }),
            "cursor-rule" => crate::cursor_rule::maintain_input(
                &self.context,
                required(&request, "action")?,
                text(&request, "task_id").unwrap_or(""),
                text(&request, "phase").unwrap_or("prompt"),
                text(&request, "approved_current_sha256"),
                flag(&request, "report_json"),
                input,
            ),
            "cursor-rule-retire" => crate::cursor_rule::retire(&self.context),
            "capsule" | "msg-dispatch" => {
                crate::dispatch::DispatchService::new(self.context.clone())
                    .execute(&operation, &request, input)
            }
            _ => Err(RuntimeError::invalid(format!(
                "Unknown memory operation: {operation}"
            ))),
        }
    }
    fn update(&self, request: &Value) -> Result<Value> {
        let id = required(request, "task_id")?;
        let update = storage::UpdateTask {
            task_id: id.into(),
            progress: text(request, "progress").map(String::from),
            next_steps: list(request, "next_steps", "next_step")?,
            files: list(request, "files", "file")?,
            sources: list(request, "sources", "source")?,
            replace_next_steps: flag(request, "replace_next_steps"),
            ..Default::default()
        };
        let command = QueuedCommand {
            owner: self.context.actor.clone(),
            mode: Some(self.context.mode.clone()),
            project: Some(self.context.roots.project.clone()),
            operation: QueuedOperation::Update {
                update: update.clone(),
                revision: request["revision"].as_u64(),
                actor: text(request, "actor").map(String::from),
                phase: text(request, "phase").map(String::from),
                allow_phase_regression: flag(request, "allow_phase_regression"),
            },
        };
        if std::env::var("CONTEXT_QUEUE_DRAIN").as_deref() != Ok("1")
            && !queue::writable(&self.context, &self.database()?)?
        {
            return self.enqueue(&command);
        }
        let mut store = self.open()?;
        if self.context.mode == "lightweight" {
            return Ok(serde_json::to_value(store.update(update)?)?);
        }
        let binding = store.binding(id)?.ok_or_else(|| unbound(id))?;
        let brain = BrainRepository::new(self.context.clone())?;
        let _guard = brain.lock()?;
        let record = brain.get_task(&binding.task_uuid)?;
        let snapshot = brain.snapshot_record(&binding.task_uuid)?;
        let mut progress = update.progress;
        if let (Some(actor), Some(value)) = (text(request, "actor"), progress.as_mut()) {
            crate::brain::actor(actor, false)?;
            *value = format!("[{actor}] {value}");
        }
        let updated = brain.update(
            &binding.task_uuid,
            crate::UpdateRecord {
                expected_revision: update_expected(request, &record)?,
                progress,
                next_steps: update.next_steps,
                files: update.files,
                sources: update.sources,
                replace_next_steps: update.replace_next_steps,
                phase: text(request, "phase").map(String::from),
                allow_phase_regression: flag(request, "allow_phase_regression"),
                ..Default::default()
            },
        )?;
        if let Err(error) = store.refresh_binding(
            &binding.external_id,
            updated["revision"].as_u64().unwrap_or(1),
        ) {
            brain.restore_record(&snapshot)?;
            return Err(error);
        }
        Ok(task_view(&updated))
    }
    fn complete(&self, request: &Value) -> Result<Value> {
        let mut store = self.open()?;
        let id = required(request, "task_id")?;
        let outcome = required(request, "outcome")?;
        if self.context.mode == "lightweight" {
            return Ok(
                object!({"episode_id":store.complete(storage::CompleteTask{task_id:id.into(),summary:text(request,"summary").map(String::from),outcome:outcome.into(),files:list(request,"files","file")?,verification:list(request,"verification","verification")?,sources:list(request,"sources","source")?})?}),
            );
        }
        let revision = request["revision"]
            .as_u64()
            .filter(|revision| *revision > 0)
            .ok_or_else(|| {
                RuntimeError::invalid("Governed completion requires the current numeric --revision")
            })?;
        let binding = store.binding(id)?.ok_or_else(|| unbound(id))?;
        let brain = BrainRepository::new(self.context.clone())?;
        let _guard = brain.lock()?;
        let before = brain.get_task(&binding.task_uuid)?;
        let snapshot = brain.snapshot_record(&binding.task_uuid)?;
        let verification = list(request, "verification", "verification")?;
        let task = brain.close_task(&binding.task_uuid, outcome, &verification, Some(revision))?;
        let episode = storage::EpisodeInput {
            summary: text(request, "summary")
                .unwrap_or(before["goal"].as_str().unwrap_or(""))
                .into(),
            outcome: outcome.into(),
            files: domain::unique_strings(
                strings(&before["files"])
                    .into_iter()
                    .chain(list(request, "files", "file")?),
            ),
            verification: verification.clone(),
            sources: domain::unique_strings(
                strings(&before["sources"])
                    .into_iter()
                    .chain(list(request, "sources", "source")?),
            ),
        };
        let saved = store.transaction(|conn| {
            let episode_id = storage::insert_episode(conn, &episode)?;
            conn.execute(
                "DELETE FROM task_bindings WHERE external_id=?1",
                [&binding.external_id],
            )?;
            conn.execute(
                "DELETE FROM working_tasks WHERE task_id=?1",
                [&binding.external_id],
            )?;
            Ok(episode_id)
        });
        let episode_id = match saved {
            Ok(id) => id,
            Err(error) => {
                brain.restore_record(&snapshot)?;
                return Err(error);
            }
        };
        let event = brain
            .create(crate::CreateRecord {
                record_type: "event".into(),
                external_id: format!("EVENT-{}", &binding.task_uuid.replace('-', "")[..12]),
                title: format!(
                    "Completed {}: {}",
                    task["external_id"].as_str().unwrap_or(""),
                    query::chars(task["title"].as_str().unwrap_or(""), 80)
                ),
                goal: Some(format!(
                    "{}{}",
                    outcome.trim(),
                    if verification.is_empty() {
                        String::new()
                    } else {
                        format!(" Verification: {}", verification.join("; "))
                    }
                )),
                sources: strings(&task["sources"])
                    .into_iter()
                    .filter(|source| self.context.roots.resolve(source).is_file())
                    .collect(),
                ..Default::default()
            })
            .ok();
        Ok(
            object!({"episode_id":episode_id,"task_id":id,"task_uuid":task["id"],"revision":task["revision"],"event_id":event.and_then(|record|record["id"].as_str().map(String::from))}),
        )
    }
    fn links(&self, request: &Value) -> Result<Value> {
        let mut store = self.open()?;
        index::refresh_index(&mut store, &self.context, true)?;
        let paths = list(request, "paths", "path")?;
        let path = paths
            .first()
            .ok_or_else(|| RuntimeError::invalid("path is required"))?;
        let prefix = flag(request, "prefix");
        let pattern = format!(
            "{}/%",
            path.trim_matches('/')
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let sql = format!(
            "SELECT DISTINCT {},l.ref_path,l.ref_kind FROM document_links l JOIN documents d ON d.path=l.path JOIN document_metadata m ON m.path=d.path WHERE l.ref_path=?1 OR (?2 AND l.ref_path LIKE ?3 ESCAPE '\\') ORDER BY d.path,l.ref_path,l.ref_kind",
            index::DOCUMENT_SELECT
        );
        let mut statement = store.conn.prepare(&sql)?;
        let config = RuntimeConfig::load(&self.context.roots)?;
        let mut documents = Vec::new();
        let mut excluded = Vec::new();
        for row in statement.query_map(params![path, prefix, pattern], |row| {
            Ok((
                index::document_from_row(row)?,
                row.get::<_, String>("ref_path")?,
                row.get::<_, String>("ref_kind")?,
            ))
        })? {
            let (document, reference, kind) = row?;
            if let Some(reason) = retrieval::admit_document(&self.context, &config, &document)? {
                excluded.push(object!({"path":document.path,"reason":reason}));
            } else {
                documents.push(object!({"path":document.path,"layer":document.layer,"kind":document.kind,"title":document.title,"ref_path":reference,"ref_kind":kind,"authority":document.authority,"lifecycle":document.lifecycle}));
            }
        }
        Ok(object!({"path":path,"prefix":prefix,"documents":documents,"excluded":excluded}))
    }
}
impl QueueDispatcher for MemoryService {
    fn dispatch(&mut self, command: &QueuedCommand) -> Result<Value> {
        let mut service = self.clone();
        if let Some(project) = &command.project
            && crate::fs::canonicalize(project)?
                != crate::fs::canonicalize(&service.context.roots.project)?
        {
            return Err(RuntimeError::policy(
                "Queued project does not match this checkout",
            ));
        }
        service.context.actor = command.owner.clone();
        if let Some(mode) = &command.mode {
            service.context.mode = mode.clone();
        }
        match &command.operation {
            QueuedOperation::Update {
                update,
                revision,
                actor,
                phase,
                allow_phase_regression,
            } => {
                let mut request = serde_json::to_value(update)?;
                request["revision"] = revision.map(Value::from).unwrap_or(object!("auto"));
                request["actor"] = object!(actor);
                request["phase"] = object!(phase);
                request["allow_phase_regression"] = object!(allow_phase_regression);
                service.update_direct(&request)
            }
            QueuedOperation::Flush { task_id, max_files } => service.turn_direct(TurnRequest {
                task_id: task_id.clone(),
                flush: true,
                max_files: *max_files,
                ..Default::default()
            }),
        }
    }
}
impl MemoryService {
    fn update_direct(&self, request: &Value) -> Result<Value> {
        let mut store = self.open()?;
        if self.context.mode == "lightweight" {
            return Ok(serde_json::to_value(
                store.update(serde_json::from_value(request.clone())?)?,
            )?);
        }
        let id = required(request, "task_id")?;
        let binding = store.binding(id)?.ok_or_else(|| unbound(id))?;
        let brain = BrainRepository::new(self.context.clone())?;
        let _guard = brain.lock()?;
        let record = brain.get_task(&binding.task_uuid)?;
        let snapshot = brain.snapshot_record(&binding.task_uuid)?;
        let mut progress = text(request, "progress").map(String::from);
        if let (Some(actor), Some(value)) = (text(request, "actor"), progress.as_mut()) {
            crate::brain::actor(actor, false)?;
            *value = format!("[{actor}] {value}");
        }
        let updated = brain.update(
            &binding.task_uuid,
            crate::UpdateRecord {
                expected_revision: update_expected(request, &record)?,
                progress,
                next_steps: list(request, "next_steps", "next_step")?,
                files: list(request, "files", "file")?,
                sources: list(request, "sources", "source")?,
                replace_next_steps: flag(request, "replace_next_steps"),
                phase: text(request, "phase").map(String::from),
                allow_phase_regression: flag(request, "allow_phase_regression"),
                ..Default::default()
            },
        )?;
        if let Err(error) = store.refresh_binding(id, updated["revision"].as_u64().unwrap_or(1)) {
            brain.restore_record(&snapshot)?;
            return Err(error);
        }
        Ok(task_view(&updated))
    }
}
/// A governed task as `get`, `start`, `update` and `rebind` show it: its
/// status and phase (null when undeclared), and Project Brain as its authority.
pub fn task_view(record: &Value) -> Value {
    object!({"task_id":record["external_id"],"phase":record.get("phase").cloned().unwrap_or(Value::Null),"task_uuid":record["id"],"revision":record["revision"],"status":record["status"],"goal":record["goal"],"progress":record["progress"],"auto_checkpoint":record.get("auto_checkpoint").cloned().unwrap_or(Value::Null),"next_steps":record["next_steps"],"files":record["files"],"sources":record["sources"],"created_at":record["created_at"],"updated_at":record["updated_at"],"authority":"project-brain"})
}
pub fn text<'a>(request: &'a Value, key: &str) -> Option<&'a str> {
    request.get(key).and_then(Value::as_str)
}
pub fn required<'a>(request: &'a Value, key: &str) -> Result<&'a str> {
    text(request, key)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| RuntimeError::invalid(format!("{key} is required")))
}
pub fn flag(request: &Value, key: &str) -> bool {
    request[key].as_bool() == Some(true)
}
pub fn number(request: &Value, key: &str) -> Option<usize> {
    request[key]
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
}
pub fn list(request: &Value, plural: &str, singular: &str) -> Result<Vec<String>> {
    let value = request.get(plural).or_else(|| request.get(singular));
    match value {
        None | Some(Value::Null) => Ok(vec![]),
        Some(Value::String(value)) => Ok(vec![value.clone()]),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(String::from)
                    .ok_or_else(|| RuntimeError::invalid(format!("{plural} requires strings")))
            })
            .collect(),
        _ => Err(RuntimeError::invalid(format!("{plural} requires a list"))),
    }
}
fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect()
}
fn revision(request: &Value) -> Result<crate::Revision> {
    if request["revision"] == "auto" {
        Ok(crate::Revision::Auto)
    } else {
        request["revision"]
            .as_u64()
            .filter(|revision| *revision > 0)
            .map(crate::Revision::Exact)
            .ok_or_else(|| RuntimeError::invalid("Supply the current task revision"))
    }
}
fn expected(request: &Value, record: &Value) -> Result<Option<u64>> {
    match revision(request)? {
        crate::Revision::Auto => Ok(record["revision"].as_u64()),
        crate::Revision::Exact(revision) => Ok(Some(revision)),
    }
}

fn update_expected(request: &Value, record: &Value) -> Result<Option<u64>> {
    // The legacy working-task update accepts an omitted revision. Explicit
    // revisions still compare-and-swap, and auto is resolved under its caller's lock.
    if request.get("revision").is_none_or(Value::is_null) {
        Ok(None)
    } else {
        expected(request, record)
    }
}
fn promote_on_resolution(bank: &BankService, record: &Value) -> Option<Value> {
    if record["authority"] != "verified"
        || !matches!(
            (record["type"].as_str(), record["status"].as_str()),
            (Some("finding" | "bug"), Some("resolved"))
                | (Some("incident"), Some("closed"))
                | (Some("decision"), Some("accepted"))
        )
    {
        return None;
    }
    let id = record["id"].as_str()?.to_string();
    let only = [id.clone()];
    Some(
        bank.auto_promote(
            crate::bank::promotion::AUTOMATIC_PROMOTION_LIMIT,
            None,
            Some(&only),
        )
        .unwrap_or_else(|error| {
            object!({"enabled":true,"promoted":[],"blocked":[],"skipped":0,
            "failed":[{"record_id":id,"reason":error.to_string()}]})
        }),
    )
}
fn payload(request: &Value, input: &[u8]) -> Result<Value> {
    if let Some(value) = request
        .get("request")
        .or_else(|| request.get("data"))
        .filter(|value| value.is_object())
    {
        return Ok(value.clone());
    }
    if let Some(path) = text(request, "input").filter(|path| *path != "-") {
        let bytes = crate::result_store::read_bounded(Path::new(path), 1024 * 1024)?
            .ok_or_else(|| RuntimeError::not_found("Input is unavailable"))?;
        crate::results::strict_input(&bytes)
    } else {
        crate::results::strict_input(input)
    }
}
/// The refusal agents met most often in the retired runtime, which says how a
/// task comes to be bound in this checkout.
fn unbound(id: &str) -> RuntimeError {
    RuntimeError::not_found(format!(
        "Working task not found: {id}; start records it (so does the first checkpoint), and rebind --task-id {id} binds one recorded on another machine"
    ))
}
pub fn envelope(stdout: String, result: Value, code: i32) -> Value {
    object!({"_cli":{"stdout":stdout,"stderr":"","code":code},"result":result})
}
fn host(value: &str) -> Result<crate::Host> {
    match value {
        "claude" => Ok(crate::Host::Claude),
        "codex" => Ok(crate::Host::Codex),
        "cursor" => Ok(crate::Host::Cursor),
        _ => Err(RuntimeError::invalid("Invalid native host")),
    }
}
pub fn request_retrieval(request: &Value) -> Result<RetrievalRequest> {
    let mut value = request.clone();
    if let Some(object) = value.as_object_mut() {
        if let Some(paths) = object
            .get("paths")
            .cloned()
            .or_else(|| object.get("path").cloned())
        {
            object.insert(
                "paths".into(),
                if paths.is_array() {
                    paths
                } else {
                    object!([paths])
                },
            );
        }
        if let Some(hints) = object.get("source_hints_json").and_then(Value::as_str) {
            // The optional hint boundary owns strict decoding, including
            // duplicate decoded keys and its byte/depth/node limits.
            object.insert("source_hints".into(), Value::String(hints.to_string()));
        }
    }
    Ok(serde_json::from_value(value)?)
}
fn hook_query(store: &Store, context: &RuntimeContext, id: &str) -> Result<(String, String)> {
    if context.mode == "lightweight" {
        if let Some(task) = store.working(id)? {
            return Ok((
                query::build_capsule_query("", Some(&serde_json::to_value(task)?))?,
                "task".into(),
            ));
        }
    } else if let Some(uuid) = checkpoint::served_uuid(Some(store), context, id)?
        && let Some(found) = governed_hook_query(context, id, &uuid)?
    {
        return Ok(found);
    }
    Ok((id.into(), "task-id".into()))
}
/// A governed task's own state, as the retired hook asked with it: the goal
/// unless it is the auto-provisioned one (the branch slug re-cased, noise
/// under another name), the manual progress note, next steps and files. With
/// nothing substantive left, or a task that cannot be read, the bare
/// identifier asks instead.
fn governed_hook_query(
    context: &RuntimeContext,
    id: &str,
    uuid: &str,
) -> Result<Option<(String, String)>> {
    match readable_task(context, uuid)? {
        Some(task) => task_hook_query(context, id, &task),
        None => Ok(None),
    }
}
fn readable_task(context: &RuntimeContext, uuid: &str) -> Result<Option<Value>> {
    match BrainRepository::new(context.clone())?.get_task(uuid) {
        Ok(task) => Ok(Some(task)),
        Err(error) if error.code.as_deref() == Some("source-git") => Err(error),
        Err(_) => Ok(None),
    }
}
/// A governed continuation of a Cursor prompt asks with the current task, as
/// the retired runtime did: an explicit goal alone (incidental progress and
/// next-step words would dilute it below relevance), otherwise the task's
/// state. A terminal or empty task leaves the prompt's own query.
fn continuation_query(
    context: &RuntimeContext,
    id: &str,
    uuid: &str,
) -> Result<Option<(String, String)>> {
    let Some(mut task) = readable_task(context, uuid)? else {
        return Ok(None);
    };
    if domain::is_terminal("task", task["status"].as_str().unwrap_or("")) {
        return Ok(None);
    }
    if task["goal"]
        .as_str()
        .is_some_and(|goal| !goal.trim().is_empty() && goal != storage::derive_goal(id))
    {
        task["progress"] = object!("");
        task["next_steps"] = object!([]);
        task["files"] = object!([]);
    }
    task_hook_query(context, id, &task)
}
fn task_hook_query(
    context: &RuntimeContext,
    id: &str,
    task: &Value,
) -> Result<Option<(String, String)>> {
    let config = RuntimeConfig::load(&context.roots)?;
    if index::record_eligibility(context, &config, task)?.is_some() {
        return Ok(None);
    }
    let goal = task["goal"]
        .as_str()
        .filter(|goal| *goal != storage::derive_goal(id))
        .unwrap_or("");
    let listed = |field: &str| task[field].as_array().cloned().unwrap_or_default();
    let working = object!({"goal":goal,"progress":task["progress"].as_str().unwrap_or(""),"next_steps":listed("next_steps"),"files":listed("files")});
    let substantive = !goal.is_empty()
        || !working["progress"].as_str().unwrap_or("").is_empty()
        || !listed("next_steps").is_empty()
        || !listed("files").is_empty();
    if !substantive {
        return Ok(None);
    }
    Ok(query::build_capsule_query(id, Some(&working))
        .ok()
        .map(|query| (query, "task".into())))
}
fn codebase_status(store: &Store, context: &RuntimeContext) -> Result<Value> {
    let rows = (|| -> std::result::Result<Vec<String>, rusqlite::Error> {
        let mut statement = store
            .conn
            .prepare("SELECT path FROM documents WHERE kind='codebase' ORDER BY path")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect()
    })()
    .unwrap_or_default();
    let mut maps = Vec::new();
    for path in rows {
        let content = match crate::fs::read_text(&context.roots.resolve(&path)) {
            Ok(content) => content,
            Err(_) => continue,
        };
        let drift = match retrieval::map_drift(context, &content) {
            Ok(drift) => drift,
            Err(error) if error.code.as_deref() == Some("source-git") => return Err(error),
            Err(_) => None,
        };
        maps.push(object!({"path":path,"commits_behind":drift}));
    }
    Ok(object!(maps))
}
fn refresh_native_continuation(host: &str, query: &str) -> bool {
    if !["claude", "codex"].contains(&host) {
        return false;
    }
    let query = query.trim_start();
    let Some(tail) = query.strip_prefix("INTERNAL_NATIVE_MEMORY_RESULT ") else {
        return false;
    };
    let token = tail.as_bytes();
    token.len() >= 32
        && token[..32]
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
        && (token.len() == 32 || tail[32..].chars().next().is_some_and(char::is_whitespace))
}
pub fn request_refresh(request: &Value) -> Result<RefreshRequest> {
    let capsule = request
        .get("query")
        .and_then(Value::as_str)
        .map(|_| request_retrieval(request))
        .transpose()?;
    if request.get("limit").is_some() && request["limit"].as_u64().is_none_or(|n| n < 1) {
        return Err(RuntimeError::invalid("--limit must be a positive integer"));
    }
    Ok(RefreshRequest {
        capsule,
        validate: flag(request, "validate"),
        read_only: flag(request, "read_only"),
        read_only_reason: text(request, "read_only_reason").map(String::from),
        host: text(request, "host").map(String::from),
        task_id: text(request, "task_id").map(String::from),
        query_withheld: flag(request, "query_withheld"),
        provenance_response: flag(request, "provenance_response"),
        capsule_profile: text(request, "capsule_profile").map(String::from),
        delivery_profile: request
            .get("delivery_profile")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()?
            .unwrap_or_default(),
        cursor_continuation: flag(request, "cursor_continuation"),
        cursor_topic_eligible: flag(request, "cursor_topic_eligible"),
        printed: false,
    })
}
fn refresh_capsule_shape(
    context: &RuntimeContext,
    capsule: &Capsule,
    requested: Option<&str>,
    text: Option<&str>,
) -> Value {
    let profile = RuntimeConfig::load(&context.roots)
        .and_then(|c| c.capsule_profile(requested))
        .ok();
    let count = |key: &str| capsule.0[key].as_array().map_or(0, Vec::len);
    let tokens = capsule.0["token_estimates"]["total"].as_u64();
    object!({"profile":profile,"working":capsule.0["working"].as_object().is_some_and(|v|!v.is_empty()),"recent_files":count("recent_files"),"ticket_files":capsule.0["ticket_files"]["paths"].as_array().map_or(0,Vec::len),"documents":count("selected"),"tokens":tokens,"bytes":text.map(|t|t.len()+1)})
}
#[derive(Clone, Debug)]
pub struct RefreshOutcome {
    pub value: Value,
    pub capsule: Option<Capsule>,
    pub profile: DeliveryProfile,
}

/// What a hook-context request delivers.
enum HookContext {
    Capsule(Capsule),
    /// A valid branch with no task and no change has no context (exit 3).
    Nothing,
    /// An enforced gate skipped the turn; the host keeps what it has (exit 4).
    Withheld,
}
/// stdout=None delegates ordinary value formatting to the CLI. A captured
/// raw protocol may intentionally supply Some("") and still succeed.
pub struct CommandOutput {
    pub value: Value,
    pub stdout: Option<String>,
    pub stderr: String,
    pub code: i32,
    pub after_delivery: Option<crate::observability::RenderFinalizer>,
}
impl CommandOutput {
    fn value(value: Value) -> Self {
        Self {
            value,
            stdout: None,
            stderr: String::new(),
            code: 0,
            after_delivery: None,
        }
    }
    fn capsule(
        context: &RuntimeContext,
        capsule: Capsule,
        profile: DeliveryProfile,
    ) -> Result<Self> {
        let text = capsule::render(&capsule, profile)? + "\n";
        let finalizer = crate::observability::RenderFinalizer::from_projection(
            context,
            &capsule,
            &text,
            "printed_capsule",
        );
        Ok(Self {
            value: capsule.0,
            stdout: Some(text),
            stderr: String::new(),
            code: 0,
            after_delivery: finalizer,
        })
    }
}
impl MemoryService {
    pub fn execute_cli(
        &self,
        operation: &str,
        request: &Value,
        input: &[u8],
    ) -> Result<CommandOutput> {
        let operation = operation.replace('_', "-");
        if operation == "parity" {
            let scope = crate::parity::ParityRequest {
                skills_only: flag(request, "skills_only"),
                cross_edition: flag(request, "cross_edition"),
            };
            let value = self.parity(scope.clone())?;
            let code = if value["valid"] == true { 0 } else { 1 };
            if flag(request, "json") {
                return Ok(CommandOutput {
                    value,
                    stdout: None,
                    stderr: String::new(),
                    code,
                    after_delivery: None,
                });
            }
            let (stdout, stderr) = crate::parity::render_report(&value, &scope);
            return Ok(CommandOutput {
                value,
                stdout: Some(stdout),
                stderr,
                code,
                after_delivery: None,
            });
        }
        if operation == "refresh" {
            let mut refresh = request_refresh(request)?;
            refresh.printed = !flag(request, "json");
            let outcome = self.refresh_output(refresh)?;
            if outcome.value["native_continuation"] == true {
                return Ok(CommandOutput {
                    value: Value::Null,
                    stdout: Some(String::new()),
                    stderr: String::new(),
                    code: 0,
                    after_delivery: None,
                });
            }
            if !flag(request, "json") {
                return refresh_command_output(&self.context, outcome, request);
            }
            let healthy = refresh_succeeded(&outcome.value);
            return Ok(CommandOutput {
                value: outcome.value,
                stdout: None,
                stderr: String::new(),
                code: if healthy { 0 } else { 1 },
                after_delivery: None,
            });
        }
        if !flag(request, "json") {
            match operation.as_str() {
                "context" | "retrieve" => {
                    let mut retrieval = request_retrieval(request)?;
                    retrieval.entry_point = operation;
                    retrieval.capsule_profile = Some("full".into());
                    retrieval.allow_unprovisioned = true;
                    let profile = retrieval.delivery_profile;
                    return CommandOutput::capsule(
                        &self.context,
                        self.retrieve(retrieval)?,
                        profile,
                    );
                }
                "hook-context" => {
                    return match self.hook_context(request)? {
                        (HookContext::Capsule(capsule), profile) => {
                            CommandOutput::capsule(&self.context, capsule, profile)
                        }
                        (outcome, _) => Ok(CommandOutput {
                            value: Value::Null,
                            stdout: Some(String::new()),
                            stderr: String::new(),
                            code: if matches!(outcome, HookContext::Nothing) {
                                3
                            } else {
                                4
                            },
                            after_delivery: None,
                        }),
                    };
                }
                _ => {}
            }
        }
        let value = self.execute(&operation, request, input)?;
        if let Some(protocol) = value.get("_cli") {
            return Ok(CommandOutput {
                value: value.get("result").cloned().unwrap_or(Value::Null),
                stdout: Some(protocol["stdout"].as_str().unwrap_or("").into()),
                stderr: protocol["stderr"].as_str().unwrap_or("").into(),
                code: protocol["code"].as_i64().unwrap_or(0) as i32,
                after_delivery: None,
            });
        }
        Ok(CommandOutput::value(value))
    }
}

pub(crate) fn refresh_succeeded(result: &Value) -> bool {
    ["procedural", "semantic", "episodic"]
        .into_iter()
        .all(|layer| result[layer] == "updated")
}

fn refresh_command_output(
    context: &RuntimeContext,
    outcome: RefreshOutcome,
    request: &Value,
) -> Result<CommandOutput> {
    let result = &outcome.value;
    if result["startup_profile"] == "read-only-working-v1" {
        let stdout = if let Some(text) = result["capsule_text"].as_str() {
            format!("{text}\n")
        } else {
            let warnings = strings(&result["warnings"])
                .into_iter()
                .map(|warning| format!("warning: {warning}"))
                .collect::<Vec<_>>();
            if warnings.is_empty() {
                String::new()
            } else {
                warnings.join("\n") + "\n"
            }
        };
        return Ok(CommandOutput {
            value: outcome.value,
            stdout: Some(stdout),
            stderr: String::new(),
            code: 1,
            after_delivery: None,
        });
    }
    let healthy = refresh_succeeded(result);
    let per_turn = outcome.capsule.is_some() || result["query_withheld"] == true;
    let host = text(request, "host").unwrap_or("cli");
    if healthy
        && host != "cli"
        && request.get("query").is_none_or(Value::is_null)
        && request.get("task_id").is_none_or(Value::is_null)
        && result["query_withheld"] != true
    {
        return Ok(CommandOutput {
            value: outcome.value,
            stdout: Some(String::new()),
            stderr: String::new(),
            code: 0,
            after_delivery: None,
        });
    }
    let mut lines = Vec::new();
    for layer in ["procedural", "semantic", "episodic"] {
        if per_turn && healthy {
            break;
        }
        let suffix = result["counts"][layer]
            .as_u64()
            .map(|count| format!(" ({count} documents)"))
            .unwrap_or_default();
        lines.push(format!(
            "{layer}: {}{suffix}",
            result[layer].as_str().unwrap_or("failed")
        ));
    }
    if let Some(overdue) = result["overdue_review"].as_u64().filter(|count| *count > 0) {
        lines.push(format!(
            "memory review: {overdue} chunk(s) overdue, not served until re-verified (bank-audit)"
        ));
    }
    if outcome.capsule.is_none() {
        let notices = [
            ("invalid-utf8", "invalid UTF-8 content"),
            ("invalid-utf8-path", "invalid UTF-8 paths"),
        ]
        .into_iter()
        .filter_map(|(key, label)| {
            result["source_notices"][key]
                .as_u64()
                .filter(|count| *count > 0)
                .map(|count| format!("{label} {count}"))
        })
        .collect::<Vec<_>>();
        if !notices.is_empty() {
            lines.push(format!("sources omitted: {}", notices.join("; ")));
        }
    }
    for item in result["codebase"].as_array().into_iter().flatten() {
        let behind = item["commits_behind"]
            .as_u64()
            .map(|count| format!("{count} commit(s) behind"))
            .unwrap_or_else(|| "unverifiable".into());
        lines.push(format!(
            "codebase map: {} — {behind}",
            item["path"].as_str().unwrap_or("")
        ));
    }
    let elapsed = ["stat", "index", "retrieval"]
        .iter()
        .filter_map(|name| result["phases"][*name].as_f64())
        .sum::<f64>();
    if !per_turn || elapsed >= 2.0 {
        let values = ["stat", "index", "retrieval"]
            .into_iter()
            .filter_map(|name| {
                result["phases"][name].as_f64().map(|seconds| {
                    format!("{name} {}ms", (seconds * 1000.0).round_ties_even() as i64)
                })
            })
            .collect::<Vec<_>>();
        lines.push(format!("phases: {}", values.join(" ")));
    }
    if let Some(validation) = result["brain_validation"].as_str() {
        lines.push(format!("brain-validation: {validation}"));
    }
    let mut after_delivery = None;
    if let Some(capsule) = &outcome.capsule {
        let rendered = capsule::render(capsule, outcome.profile)?;
        after_delivery = crate::observability::RenderFinalizer::from_projection(
            context,
            capsule,
            &(rendered.clone() + "\n"),
            "printed_capsule",
        );
        lines.push(rendered);
        if let Some(review) = result["memory_review"]
            .as_str()
            .filter(|value| !value.is_empty())
        {
            lines.push(review.into());
        }
    } else {
        for warning in strings(&result["warnings"]) {
            lines.push(format!("warning: {warning}"));
        }
    }
    let stdout = if lines.is_empty() {
        String::new()
    } else {
        lines.join("\n") + "\n"
    };
    Ok(CommandOutput {
        value: outcome.value,
        stdout: Some(stdout),
        stderr: String::new(),
        code: if healthy { 0 } else { 1 },
        after_delivery,
    })
}

fn field_supplied(request: &Value, names: &[&str]) -> bool {
    if let Some(arguments) = request["_arguments"].as_array() {
        return arguments.iter().filter_map(Value::as_str).any(|argument| {
            names.iter().any(|name| {
                let flag = format!("--{}", name.replace('_', "-"));
                argument == flag
                    || argument
                        .strip_prefix(&flag)
                        .is_some_and(|tail| tail.starts_with('='))
            })
        });
    }
    names
        .iter()
        .any(|name| request.get(*name).is_some_and(|value| !value.is_null()))
}
