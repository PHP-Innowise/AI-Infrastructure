use super::{
    cost::{self, Close, Ledger},
    projection::{CONTEXT_BYTES, NativeProjectionMemory, NativeRuntime},
};
use crate::strict;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::ThreadId,
    time::Instant,
};
use uuid::Uuid;
pub const PROFILE: &str = "longmemeval/native-fts5-registration-v1";
pub const MEMORY_TYPE: &str = "native_fts5_states_v1";
impl<T: NativeRuntime + ?Sized> NativeRuntime for Box<T> {
    fn refresh_and_render(
        &mut self,
        r: &std::path::Path,
        q: super::projection::NativeRequest,
    ) -> Result<super::projection::NativeResponse> {
        (**self).refresh_and_render(r, q)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Params {
    pub horizon: usize,
    #[serde(default = "unknown")]
    pub local_token_profile: String,
}
fn unknown() -> String {
    "unknown".into()
}
impl Params {
    pub fn validate(&self) -> Result<()> {
        if !(1..=512).contains(&self.horizon)
            || !["unknown", "pinned_local_no_model"].contains(&self.local_token_profile.as_str())
        {
            bail!("bridge_parameters");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryReceipt {
    pub ordinal: usize,
    pub outcome: String,
    pub state: String,
    pub native_metadata_bound: bool,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub component_ns: Option<u64>,
    pub query_image_ignored: bool,
}
pub trait Memory: Send {
    fn memory_config(&self) -> Value;
    fn insert(&mut self, trajectory: &Value) -> Result<()>;
    fn set_query_context(&mut self, id: &str) -> Result<()>;
    fn query(&mut self, query: &str, image: Option<&str>) -> Result<Vec<Value>>;
    fn post_query_hook(
        &mut self,
        query: &str,
        image: Option<&str>,
        context: &[Value],
    ) -> Result<Value>;
    fn clear_query_context(&mut self) -> Result<()>;
    fn snapshot(&self) -> Result<Value>;
    fn close(&mut self) -> Result<Value>;
    fn failure_observation(&self, phase: &str) -> Value;
    fn custody_failure_observation(&self) -> Value;
    fn save_memory(&self, _: &std::path::Path) -> Result<()> {
        bail!("bridge_persistence_unsupported")
    }
}
pub struct ObservedProjectionMemory {
    backend: NativeProjectionMemory<Box<dyn NativeRuntime>>,
    params: Params,
    ledger: Ledger,
    state: &'static str,
    insert_calls: usize,
    inserted: usize,
    replayed: usize,
    insert_failed: usize,
    insert_cancelled: usize,
    projection_bytes: usize,
    insert_bytes: u64,
    build_ns: Option<u64>,
    clock: Option<Box<dyn super::lifecycle::Clock>>,
    clock_abort: Option<Box<dyn std::any::Any + Send>>,
    timing_incomplete: bool,
    images: BTreeMap<String, String>,
    receipts: Vec<QueryReceipt>,
    native_queries: usize,
    closed: bool,
    final_view: Option<Value>,
    failures: BTreeMap<String, Value>,
}
impl ObservedProjectionMemory {
    pub fn new(root: PathBuf, runtime: Box<dyn NativeRuntime>, params: Params) -> Result<Self> {
        Self::new_with_clock(root, runtime, params, None)
    }
    pub fn new_with_clock(
        root: PathBuf,
        runtime: Box<dyn NativeRuntime>,
        params: Params,
        mut clock: Option<Box<dyn super::lifecycle::Clock>>,
    ) -> Result<Self> {
        params.validate()?;
        let external_start = clock
            .as_mut()
            .and_then(|c| c.now_ns().ok())
            .filter(|n| *n <= super::usage::LIMIT);
        let start = Instant::now();
        let backend = NativeProjectionMemory::open(&root, runtime)?;
        let build_ns = if let Some(clock) = clock.as_mut() {
            external_start
                .zip(clock.now_ns().ok())
                .and_then(|(s, e)| e.checked_sub(s))
                .filter(|n| *n <= super::usage::LIMIT)
        } else {
            start.elapsed().as_nanos().try_into().ok()
        };
        let timing_incomplete = build_ns.is_none();
        Ok(Self {
            backend,
            ledger: Ledger::new(params.horizon)?,
            params,
            state: "building",
            insert_calls: 0,
            inserted: 0,
            replayed: 0,
            insert_failed: 0,
            insert_cancelled: 0,
            projection_bytes: 0,
            insert_bytes: 0,
            build_ns,
            clock,
            clock_abort: None,
            timing_incomplete,
            images: BTreeMap::new(),
            receipts: vec![],
            native_queries: 0,
            closed: false,
            final_view: None,
            failures: BTreeMap::new(),
        })
    }
    fn timing_start(&mut self) -> (Instant, Option<u64>) {
        let value = if let Some(clock) = self.clock.as_mut() {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| clock.now_ns())) {
                Ok(Ok(n)) if n <= super::usage::LIMIT => Some(n),
                Ok(_) => {
                    self.timing_incomplete = true;
                    None
                }
                Err(payload) => {
                    self.timing_incomplete = true;
                    if self.clock_abort.is_none() {
                        self.clock_abort = Some(payload);
                    }
                    None
                }
            }
        } else {
            None
        };
        (Instant::now(), value)
    }
    fn timing_end(&mut self, start: (Instant, Option<u64>)) -> Option<u64> {
        let result = if self.clock.is_some() {
            let (_, end) = self.timing_start();
            start
                .1
                .zip(end)
                .and_then(|(s, e)| e.checked_sub(s))
                .filter(|n| *n <= super::usage::LIMIT)
        } else {
            start.0.elapsed().as_nanos().try_into().ok()
        };
        if result.is_none() {
            self.timing_incomplete = true;
        }
        result
    }
    fn add_build(&mut self, amount: Option<u64>) {
        self.build_ns = self
            .build_ns
            .zip(amount)
            .and_then(|(a, b)| a.checked_add(b))
            .filter(|n| *n <= super::usage::LIMIT);
        if self.build_ns.is_none() {
            self.timing_incomplete = true;
        }
    }
    fn return_or_clock<T>(&mut self, result: Result<T>) -> Result<T> {
        let abort = self.clock_abort.take();
        if result.is_ok()
            && let Some(payload) = abort
        {
            std::panic::resume_unwind(payload);
        }
        result
    }
    fn note(&mut self, phase: &str, error: &anyhow::Error) {
        let guard = error
            .downcast_ref::<super::projection::ProjectionError>()
            .map(|e| e.0)
            .filter(|g| {
                [
                    "ambiguous_alias",
                    "conflicting_identity",
                    "context_bound",
                    "corpus_bound",
                    "duplicate_key",
                    "input_bound",
                    "input_encoding",
                    "input_schema",
                    "instance_unavailable",
                    "native_configuration",
                    "native_profile",
                    "query_bound",
                    "screenshot_reference",
                    "source_changed",
                ]
                .contains(g)
            });
        self.failures.insert(phase.into(),json!({"profile":"longmemeval/projection-body-failure-v1","scope":"last_operation_body_exception_only","phase":phase,"reported_exception":if error.is::<super::projection::ProjectionError>(){"ProjectionError"}else{"unclassified"},"reported_guard":guard,"observation_state":if guard.is_some(){"fixed_guard_reported"}else{"unmeasured"},"root_cause":null,"provider_tokens":null,"usd_micros":null}));
    }
    pub fn insert(&mut self, v: &Value) -> Result<()> {
        if self.closed || self.state != "building" {
            bail!("observation_lifecycle");
        }
        if self.insert_calls >= 4096 {
            bail!("insert_capacity");
        }
        self.insert_calls += 1;
        let start = self.timing_start();
        if let Some(payload) = self.clock_abort.take() {
            self.insert_cancelled += 1;
            self.add_build(None);
            std::panic::resume_unwind(payload);
        }
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
            let raw = strict::wire(v)?;
            self.insert_bytes = self
                .insert_bytes
                .checked_add(raw.len() as u64)
                .context("counter_overflow")?;
            let p = super::projection::project_trajectory(v)?;
            self.backend.insert(v)?;
            let id = strict::sha(p.trajectory_id.as_bytes());
            if let Some(prior) = self.images.get(&id) {
                if prior == &p.input_sha256 {
                    self.replayed += 1;
                } else {
                    self.state = "backend_failed";
                    bail!("backend_identity");
                }
            } else {
                self.images.insert(id, p.input_sha256);
                self.inserted += 1;
                self.projection_bytes += p.body.len();
            }
            Ok(())
        }));
        let mut panic = None;
        let result = match caught {
            Ok(result) => result,
            Err(payload) => {
                panic = Some(payload);
                Err(anyhow::anyhow!("insert_cancelled"))
            }
        };
        let duration = self.timing_end(start);
        self.add_build(duration);
        if let Err(e) = &result {
            if panic.is_some() {
                self.insert_cancelled += 1;
            } else {
                self.insert_failed += 1;
            }
            self.note("insert", e);
        }
        if let Some(payload) = panic {
            self.clock_abort.take();
            std::panic::resume_unwind(payload);
        }
        self.return_or_clock(result)
    }
    fn close_build(&mut self) -> Result<()> {
        if self.state == "building" {
            self.state = "querying";
        }
        Ok(())
    }
    pub fn query(&mut self, q: &str, image: Option<&str>) -> Result<Vec<Value>> {
        if self.closed || !["building", "querying"].contains(&self.state) {
            bail!("observation_lifecycle");
        }
        if self.receipts.len() >= self.params.horizon {
            bail!("query_capacity");
        }
        self.close_build()?;
        let ordinal = self.receipts.len();
        let start = self.timing_start();
        if let Some(payload) = self.clock_abort.take() {
            self.receipts.push(QueryReceipt {
                ordinal,
                outcome: "cancelled".into(),
                state: "cancelled".into(),
                native_metadata_bound: false,
                input_bytes: 0,
                output_bytes: 0,
                component_ns: None,
                query_image_ignored: image.is_some(),
            });
            self.ledger.close_unit(
                "query",
                Some(ordinal),
                Close {
                    coverage: "incomplete".into(),
                    model_work: "unknown".into(),
                    billing_complete: false,
                    component_ns: None,
                    constructed_bytes: Some(0),
                },
            )?;
            std::panic::resume_unwind(payload);
        }
        let input = strict::wire(&json!({"query":q,"query_image":image}))?.len() as u64;
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.backend.query(q, image)
        }));
        let mut panic = None;
        let result = match caught {
            Ok(result) => result,
            Err(payload) => {
                panic = Some(payload);
                Err(anyhow::anyhow!("query_cancelled"))
            }
        };
        let meta = self.backend.post_query_hook();
        let bound = meta["queries"].as_u64() == Some((self.native_queries + 1) as u64)
            && meta["query_image_ignored"] == json!(image.is_some());
        if bound {
            self.native_queries += 1;
        }
        let state = if bound {
            meta["state"].as_str().unwrap_or("unmeasured").to_owned()
        } else {
            "unmeasured".into()
        };
        let bytes = result
            .as_ref()
            .ok()
            .and_then(|v| strict::wire(&json!(v)).ok())
            .map(|b| b.len() as u64)
            .unwrap_or(0);
        let duration = self.timing_end(start);
        let receipt = QueryReceipt {
            ordinal,
            outcome: if panic.is_some() {
                "cancelled"
            } else if result.is_ok() {
                "succeeded"
            } else {
                "failed"
            }
            .into(),
            state,
            native_metadata_bound: bound,
            input_bytes: input,
            output_bytes: bytes,
            component_ns: duration,
            query_image_ignored: image.is_some(),
        };
        self.receipts.push(receipt);
        self.ledger.close_unit(
            "query",
            Some(ordinal),
            Close {
                coverage: if bound { "complete" } else { "incomplete" }.into(),
                model_work: if self.params.local_token_profile == "pinned_local_no_model" {
                    "none_observed_local"
                } else {
                    "unknown"
                }
                .into(),
                billing_complete: false,
                component_ns: duration,
                constructed_bytes: input.checked_add(bytes),
            },
        )?;
        if let Err(e) = &result {
            self.note("query", e);
        }
        if let Some(payload) = panic {
            self.clock_abort.take();
            std::panic::resume_unwind(payload);
        }
        self.return_or_clock(result)
    }
    pub fn snapshot(&self) -> Result<Value> {
        if let Some(v) = &self.final_view {
            return Ok(v.clone());
        }
        let query_ns = self
            .receipts
            .iter()
            .map(|r| r.component_ns.unwrap_or(0) as u128)
            .sum::<u128>();
        let query_bytes = self
            .receipts
            .iter()
            .map(|r| r.input_bytes as u128)
            .sum::<u128>();
        let output_bytes = self
            .receipts
            .iter()
            .map(|r| r.output_bytes as u128)
            .sum::<u128>();
        let bounded = |v: u128| {
            if v <= super::usage::LIMIT as u128 {
                Some(v as u64)
            } else {
                None
            }
        };
        Ok(
            json!({"profile":"native-fts5/observed-local-operations-v1","observed":{"status":self.state,"local_token_profile":self.params.local_token_profile,"initialization_attempted":true,"initialization_outcome":"succeeded","cleanup_outcome":if !self.closed{"not_attempted"}else if self.state=="cleanup_failed"{"failed"}else{"succeeded"},"insert_calls":self.insert_calls,"inserted":self.inserted,"replayed":self.replayed,"insert_failed":self.insert_failed,"insert_cancelled":self.insert_cancelled,"query_calls":self.receipts.len(),"query_failed":self.receipts.iter().filter(|r|r.outcome=="failed").count(),"query_cancelled":self.receipts.iter().filter(|r|r.outcome=="cancelled").count(),"unlaunched_queries":self.params.horizon-self.receipts.len(),"unique_projection_bytes":self.projection_bytes,"insert_ingress_bytes":self.insert_bytes,"query_ingress_bytes":bounded(query_bytes),"returned_context_bytes":bounded(output_bytes),"images_ingested":0,"observation_incomplete":self.timing_incomplete||self.receipts.iter().any(|r|!r.native_metadata_bound)||query_ns>super::usage::LIMIT as u128,"build_component_ns":self.build_ns,"query_component_ns":if self.receipts.iter().all(|r|r.component_ns.is_some()){bounded(query_ns)}else{None},"query_receipts":self.receipts},"ledger":self.ledger.snapshot()?}),
        )
    }
    pub fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        let start = self.timing_start();
        let close = self.backend.close();
        let duration = self.timing_end(start);
        self.add_build(duration);
        self.closed = true;
        self.state = if close.is_ok() {
            "finalized"
        } else {
            "cleanup_failed"
        };
        self.ledger.close_unit(
            "build",
            None,
            Close {
                coverage: "complete".into(),
                model_work: if self.params.local_token_profile == "pinned_local_no_model" {
                    "none_observed_local"
                } else {
                    "unknown"
                }
                .into(),
                billing_complete: false,
                component_ns: self.build_ns,
                constructed_bytes: Some(self.insert_bytes.min(super::usage::LIMIT)),
            },
        )?;
        self.ledger.finalize()?;
        self.final_view = Some(self.snapshot()?);
        self.return_or_clock(close)
    }
}
struct Invocation {
    started: bool,
    cleared: bool,
    hooks: usize,
    request: Option<String>,
    result: Option<String>,
    payload: Option<Value>,
}
impl ObservedProjectionMemory {
    pub fn query_receipt(&self, ordinal: usize) -> Result<QueryReceipt> {
        self.receipts
            .get(ordinal)
            .cloned()
            .context("receipt_ordinal")
    }
    pub fn failure_observation(&self, phase: &str) -> Result<Value> {
        if !["initialization", "insert", "query", "cleanup"].contains(&phase) {
            bail!("failure_phase_profile");
        }
        Ok(self.failures.get(phase).cloned().unwrap_or_else(||json!({"profile":"longmemeval/projection-body-failure-v1","scope":"last_operation_body_exception_only","phase":null,"reported_exception":null,"reported_guard":null,"observation_state":"unobserved","root_cause":null,"provider_tokens":null,"usd_micros":null})))
    }
    pub fn custody_failure_observation(&self) -> Value {
        self.backend.custody_failure_observation()
    }
}
pub struct Bridge {
    params: Params,
    observer: ObservedProjectionMemory,
    secret: [u8; 32],
    contexts: BTreeMap<String, Invocation>,
    current: HashMap<ThreadId, String>,
    closed: bool,
    started: bool,
    queries: usize,
    hooks: usize,
    replays: usize,
    cleared: usize,
    cancelled: usize,
    cancel: Option<Arc<AtomicBool>>,
    runtime_ignored: bool,
    final_view: Option<Value>,
}
fn context_image(c: &[Value]) -> Result<Vec<u8>> {
    if c.len() > 1 {
        bail!("bridge_hook_context");
    }
    for v in c {
        strict::keys(v, &["type", "value"], &["type", "value"])?;
        if v["type"] != "text" {
            bail!("bridge_hook_context");
        }
        strict::text(&v["value"], CONTEXT_BYTES, false, true)?;
    }
    let raw = strict::wire(&json!(c))?;
    if raw.len() > CONTEXT_BYTES {
        bail!("bridge_hook_context");
    }
    Ok(raw)
}
fn eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).fold(0u8, |d, (a, b)| d | (a ^ b)) == 0
}
impl Bridge {
    pub fn new(root: PathBuf, runtime: Box<dyn NativeRuntime>, params: Params) -> Result<Self> {
        params.validate()?;
        let mut secret = [0; 32];
        secret[..16].copy_from_slice(Uuid::new_v4().as_bytes());
        secret[16..].copy_from_slice(Uuid::new_v4().as_bytes());
        Ok(Self {
            observer: ObservedProjectionMemory::new(root, runtime, params.clone())?,
            params,
            secret,
            contexts: BTreeMap::new(),
            current: HashMap::new(),
            closed: false,
            started: false,
            queries: 0,
            hooks: 0,
            replays: 0,
            cleared: 0,
            cancelled: 0,
            cancel: None,
            runtime_ignored: false,
            final_view: None,
        })
    }
    fn ready(&self) -> Result<()> {
        if self.closed {
            bail!("bridge_lifecycle");
        }
        Ok(())
    }
    fn check_cancel(&mut self) -> Result<()> {
        if self
            .cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Acquire))
        {
            self.cancelled += 1;
            bail!("bridge_cancelled_before_work");
        }
        Ok(())
    }
    fn request(&self, q: &str, image: Option<&str>) -> Result<String> {
        strict::text(&json!(q), 8192, true, true)?;
        if let Some(i) = image {
            strict::text(&json!(i), 4096, true, false)?;
        }
        Ok(cost::hmac(
            &self.secret,
            &strict::wire(&json!({"query":q,"query_image":image}))?,
        ))
    }
    pub fn configure_runtime(
        &mut self,
        cancel: Option<Arc<AtomicBool>>,
        query_trace_dir: Option<&std::path::Path>,
        temperature: Option<f64>,
        top_p: Option<f64>,
    ) -> Result<()> {
        self.ready()?;
        if self.started
            || [temperature, top_p]
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || !(0.0..=1000.0).contains(v))
            || query_trace_dir.is_some_and(|p| p.to_string_lossy().len() > 4096)
        {
            bail!("bridge_runtime_parameters");
        }
        self.cancel = cancel;
        self.runtime_ignored =
            query_trace_dir.is_some() || temperature.is_some() || top_p.is_some();
        Ok(())
    }
}
impl Memory for Bridge {
    fn memory_config(&self) -> Value {
        json!({"memory_type":MEMORY_TYPE,"memory_params":self.params})
    }
    fn insert(&mut self, t: &Value) -> Result<()> {
        self.ready()?;
        self.started = true;
        self.check_cancel()?;
        self.observer.insert(t)
    }
    fn set_query_context(&mut self, id: &str) -> Result<()> {
        self.ready()?;
        strict::text(&json!(id), 256, true, false)?;
        if id.trim() != id {
            bail!("bridge_invocation_alias");
        }
        let thread = std::thread::current().id();
        if self.current.contains_key(&thread) {
            bail!("bridge_invocation_uncleared");
        }
        let key = cost::hmac(&self.secret, id.as_bytes());
        if self.contexts.contains_key(&key) {
            bail!("bridge_invocation_reused");
        }
        if self.contexts.len() >= self.params.horizon {
            bail!("bridge_invocation_capacity");
        }
        self.contexts.insert(
            key.clone(),
            Invocation {
                started: false,
                cleared: false,
                hooks: 0,
                request: None,
                result: None,
                payload: None,
            },
        );
        self.current.insert(thread, key);
        Ok(())
    }
    fn clear_query_context(&mut self) -> Result<()> {
        if let Some(key) = self.current.remove(&std::thread::current().id()) {
            if let Some(c) = self.contexts.get_mut(&key) {
                c.cleared = true;
            }
            if !self.closed {
                self.cleared += 1;
            }
        }
        Ok(())
    }
    fn query(&mut self, q: &str, image: Option<&str>) -> Result<Vec<Value>> {
        self.ready()?;
        let key = self
            .current
            .get(&std::thread::current().id())
            .cloned()
            .context("bridge_invocation_missing")?;
        if self.contexts[&key].started {
            bail!("bridge_invocation_reused");
        }
        self.contexts.get_mut(&key).unwrap().started = true;
        self.started = true;
        self.queries += 1;
        self.check_cancel()?;
        let request = self.request(q, image)?;
        let ordinal = self.observer.receipts.len();
        let result = self.observer.query(q, image)?;
        let digest = cost::hmac(&self.secret, &context_image(&result)?);
        let c = self.contexts.get_mut(&key).unwrap();
        c.request = Some(request);
        c.result = Some(digest);
        c.payload = Some(
            json!({"profile":PROFILE,"ordinal":ordinal,"stage":"constructed_before_reader_truncation","query_receipt":self.observer.receipts[ordinal],"reader_usage":null,"reader_usage_complete":false}),
        );
        Ok(result)
    }
    fn post_query_hook(
        &mut self,
        q: &str,
        image: Option<&str>,
        context: &[Value],
    ) -> Result<Value> {
        self.ready()?;
        let key = self
            .current
            .get(&std::thread::current().id())
            .cloned()
            .context("bridge_invocation_missing")?;
        let request = self.request(q, image)?;
        let digest = cost::hmac(&self.secret, &context_image(context)?);
        let c = self.contexts.get_mut(&key).unwrap();
        if c.payload.is_none() || c.hooks >= 16 {
            bail!("bridge_hook_state");
        }
        if !c.request.as_ref().is_some_and(|r| eq(r, &request))
            || !c.result.as_ref().is_some_and(|r| eq(r, &digest))
        {
            bail!("bridge_hook_binding");
        }
        self.replays += usize::from(c.hooks > 0);
        c.hooks += 1;
        self.hooks += 1;
        Ok(c.payload.clone().unwrap())
    }
    fn snapshot(&self) -> Result<Value> {
        if let Some(v) = &self.final_view {
            return Ok(v.clone());
        }
        Ok(
            json!({"profile":PROFILE,"closed":self.closed,"contexts_admitted":self.contexts.len(),"queries_attempted":self.queries,"hooks_verified":self.hooks,"hook_replays":self.replays,"contexts_cleared":self.cleared,"prework_cancelled":self.cancelled,"pending_contexts":self.contexts.values().filter(|c|!c.cleared).count(),"runtime_overrides_ignored":self.runtime_ignored,"local_observation":self.observer.snapshot()?}),
        )
    }
    fn failure_observation(&self, phase: &str) -> Value {
        self.observer.failures.get(phase).cloned().unwrap_or_else(||json!({"profile":"longmemeval/projection-body-failure-v1","scope":"last_operation_body_exception_only","phase":null,"reported_exception":null,"reported_guard":null,"observation_state":"unobserved","root_cause":null,"provider_tokens":null,"usd_micros":null}))
    }
    fn custody_failure_observation(&self) -> Value {
        self.observer.backend.custody_failure_observation()
    }
    fn close(&mut self) -> Result<Value> {
        if self.closed {
            return self.snapshot();
        }
        let result = self.observer.close();
        self.closed = true;
        let view = self.snapshot()?;
        self.final_view = Some(view.clone());
        result?;
        Ok(view)
    }
}
pub type ProjectFactory = Arc<dyn Fn() -> Result<(PathBuf, Box<dyn NativeRuntime>)> + Send + Sync>;
pub type MemoryFactory = Arc<dyn Fn(Params) -> Result<Box<dyn Memory>> + Send + Sync>;
#[derive(Default)]
pub struct MemoryRegistry {
    factories: BTreeMap<String, MemoryFactory>,
}
impl MemoryRegistry {
    pub fn create(&self, name: &str, params: Params) -> Result<Box<dyn Memory>> {
        self.factories.get(name).context("bridge_api_profile")?(params)
    }
}
pub fn register_bridge(
    registry: &mut MemoryRegistry,
    project_factory: ProjectFactory,
) -> Result<()> {
    if registry.factories.contains_key(MEMORY_TYPE) {
        bail!("bridge_registry_collision");
    }
    registry.factories.insert(
        MEMORY_TYPE.into(),
        Arc::new(move |params| {
            let (root, runtime) = project_factory()?;
            Ok(Box::new(Bridge::new(root, runtime, params)?))
        }),
    );
    Ok(())
}
