//! Per-generation accounting joins raw observations with bounded display metadata.
use crate::{
    context_usage::{self, CodexLive, ContextTracker},
    cost_observation::{self, LaunchObservation, LocalOperation},
    run_activity::Enricher,
};
use serde_json::{Value, json as object};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
struct State {
    provider: String,
    session: Value,
    generation: String,
    project: PathBuf,
    _source: PathBuf,
    supported: bool,
    observation: Arc<Mutex<LaunchObservation>>,
    tracker: Option<ContextTracker>,
    live: Option<CodexLive>,
    live_polled: Instant,
    native: Option<String>,
    costs: BTreeMap<String, infra_host::provider::RunCost>,
    enricher: Enricher,
    context: Option<Value>,
    count: usize,
    output_bytes: usize,
    extra: Vec<Value>,
    finished: bool,
    last_exit: BTreeMap<String, i64>,
    context_extras: BTreeMap<String, Value>,
    recovery_characters: Option<usize>,
}
#[derive(Clone)]
pub struct LaunchAccounting {
    inner: Arc<Mutex<State>>,
}
fn count(value: &Value) -> Option<u64> {
    value.as_u64()
}
fn native_workflow(session: &Value) -> bool {
    ["native", "workspace", "sdd", "plan", "review"]
        .contains(&session["workflow"].as_str().unwrap_or("native"))
        && session["fleet"].is_null()
        && session["clash"].is_null()
        && session["creator"].is_null()
        && session["creator_run"].is_null()
        && session["system_run"].is_null()
        && session["system_discovery"].is_null()
}
fn safe_name(value: &Value) -> Value {
    value
        .as_str()
        .filter(|value| {
            value.len() <= 1024
                && !Path::new(value).is_absolute()
                && !value.contains('\\')
                && !value.split('/').any(|part| part == "..")
        })
        .map(|value| object!(value))
        .unwrap_or(Value::Null)
}
fn ledger(value: &Value) -> Value {
    let mut result = object!({"message":count(&value["message"]),"capsule":null,"excerpts":null,"attachments":null,"total":count(&value["total"]),"instructions":count(&value["instructions"])});
    if value["capsule"].is_object() {
        let capsule = &value["capsule"];
        result["capsule"] = object!({"characters":count(&capsule["characters"]),"inserted":count(&capsule["inserted"]),"repeats":count(&capsule["repeats"]),"dropped":count(&capsule["dropped"]),"kinds":null,"items":null});
        for name in ["kinds", "items"] {
            if capsule[name].is_object() {
                result["capsule"][name] = object!({"brain":count(&capsule[name]["brain"]),"bank":count(&capsule[name]["bank"]),"rules":count(&capsule[name]["rules"])});
            }
        }
    }
    if let Some(excerpts) = value["excerpts"].as_array() {
        result["excerpts"]=object!(excerpts.iter().take(128).map(|entry|object!({"name":safe_name(&entry["name"]),"sent":count(&entry["sent"]),"full":count(&entry["full"]),"characters":count(&entry["characters"])})).collect::<Vec<_>>());
    }
    if value["attachments"].is_object() {
        result["attachments"] = object!({"characters":count(&value["attachments"]["characters"]),"count":count(&value["attachments"]["count"])});
    }
    result
}
impl State {
    fn poll(&mut self, settle: bool) {
        if self.provider != "codex" || self.tracker.is_none() || self.live.is_none() {
            return;
        }
        if !settle && self.live_polled.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.live_polled = Instant::now();
        let changed = if settle {
            self.live
                .as_mut()
                .is_some_and(|live| live.settle(self.native.as_deref(), context_usage::unix_now()))
        } else {
            self.live
                .as_mut()
                .is_some_and(|live| live.poll(self.native.as_deref()))
        };
        if changed {
            let fill = self.live.as_ref().and_then(|live| live.fill.snapshot());
            if let Some(tracker) = &mut self.tracker {
                tracker.update(fill.as_ref());
            }
        }
    }
    fn compactions(&mut self) {
        if let Some(tracker) = &self.tracker {
            self.extra
                .extend(self.enricher.compaction(&tracker.compactions));
        }
    }
    fn context(&mut self) -> Option<Value> {
        self.poll(false);
        if let (Some(context), Some(tracker)) = (&mut self.context, &self.tracker) {
            context["fill"] = tracker.snapshot();
        }
        self.context.clone()
    }
}
impl LaunchAccounting {
    pub fn new(
        provider: &str,
        session: &Value,
        generation: &str,
        project: &Path,
        source: &Path,
        capsule: Option<&Value>,
    ) -> Self {
        let supported =
            native_workflow(session) && ["codex", "claude", "cursor"].contains(&provider);
        let watch = capsule
            .filter(|capsule| capsule.is_object() || capsule.is_array())
            .map(crate::memory_draft::watch);
        let root = project.to_string_lossy().to_string();
        let alias = session["project_path"].as_str();
        let home = crate::run_activity::user_home();
        let strip = vec![source.to_string_lossy().to_string()];
        let outside = session["accelerator"].is_object() || session["accelerator_attached"] == true;
        let enricher = Enricher::new(
            provider,
            &root,
            alias,
            &strip,
            home.as_deref(),
            watch,
            outside,
        );
        let observation = Arc::new(Mutex::new(LaunchObservation::new(provider, supported)));
        let mut tracker = if supported {
            Some(ContextTracker::new(provider))
        } else {
            None
        };
        if let Some(tracker) = &mut tracker {
            tracker.window = count(&session["last_context_window"])
                .or_else(|| count(&session["context"]["fill"]["window"]));
        }
        Self {
            inner: Arc::new(Mutex::new(State {
                provider: provider.into(),
                session: session.clone(),
                generation: generation.into(),
                project: project.into(),
                _source: source.into(),
                supported,
                observation,
                tracker,
                live: None,
                live_polled: Instant::now(),
                native: session["native_session_id"]
                    .as_str()
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                costs: BTreeMap::new(),
                enricher,
                context: None,
                count: 0,
                output_bytes: 0,
                extra: vec![],
                finished: false,
                last_exit: BTreeMap::new(),
                context_extras: BTreeMap::new(),
                recovery_characters: None,
            })),
        }
    }
    pub fn set_capsule(&self, capsule: Option<&Value>) {
        if let Ok(mut state) = self.inner.lock() {
            if state.finished {
                return;
            }
            let watch = capsule
                .filter(|capsule| capsule.is_object() || capsule.is_array())
                .map(crate::memory_draft::watch);
            if !state.enricher.ledger.set_watch(watch)
                && let Ok(mut observed) = state.observation.lock()
            {
                observed.incomplete = true;
            }
        }
    }
    pub fn set_ledger(&self, value: &Value, agents: u32) {
        if let Ok(mut state) = self.inner.lock() {
            if state.finished || !state.supported {
                return;
            }
            let metadata = ledger(value);
            let mut context = object!({"version":1,"provider":state.provider,"agents":agents,"ledger":metadata,"fill":state.tracker.as_ref().map(ContextTracker::snapshot)});
            if let Some(files) =
                context_usage::launch_files(&state.project, &state.provider).as_object()
            {
                for (key, value) in files {
                    context[key] = value.clone();
                }
            }
            for (key, value) in &state.context_extras {
                context[key] = value.clone();
            }
            if let Some(characters) = state.recovery_characters {
                for key in ["total", "instructions"] {
                    context["ledger"][key] = context["ledger"][key]
                        .as_u64()
                        .map(|value| value.saturating_add(characters as u64))
                        .map(Value::from)
                        .unwrap_or(Value::Null);
                }
            }
            state.context = Some(context);
        }
    }
    pub fn set_cursor_rule_retirement(&self, value: &Value) {
        if let Ok(mut state) = self.inner.lock() {
            if state.finished {
                return;
            }
            let status = value["state"]
                .as_str()
                .filter(|value| {
                    [
                        "removed",
                        "preserved",
                        "absent",
                        "attached",
                        "incomplete",
                        "unavailable",
                    ]
                    .contains(value)
                })
                .unwrap_or("unavailable");
            let record = object!({"state":status,"bytes_removed":count(&value["bytes_removed"])});
            state
                .context_extras
                .insert("cursor_rule_retirement".into(), record.clone());
            if let Some(context) = &mut state.context {
                context["cursor_rule_retirement"] = record;
            }
        }
    }
    pub fn set_recovery_context(&self, characters: usize, status: &str) {
        if let Ok(mut state) = self.inner.lock() {
            if state.finished {
                return;
            }
            let status = if !status.is_empty()
                && status.len() <= 64
                && status
                    .bytes()
                    .all(|value| value.is_ascii_lowercase() || b"_-".contains(&value))
            {
                status
            } else {
                "unavailable"
            };
            let first = state.recovery_characters.is_none();
            if state
                .recovery_characters
                .is_some_and(|old| old != characters)
                && let Ok(mut observed) = state.observation.lock()
            {
                observed.incomplete = true;
            }
            let characters = *state.recovery_characters.get_or_insert(characters);
            let record = object!({"characters":characters,"state":status});
            state
                .context_extras
                .insert("recovery".into(), record.clone());
            if let Some(context) = &mut state.context {
                context["recovery"] = record;
                if first {
                    for key in ["total", "instructions"] {
                        context["ledger"][key] = context["ledger"][key]
                            .as_u64()
                            .map(|value| value.saturating_add(characters as u64))
                            .map(Value::from)
                            .unwrap_or(Value::Null);
                    }
                }
            }
        }
    }
    pub fn set_window(&self, window: Option<u64>) {
        if let Ok(mut state) = self.inner.lock()
            && let Some(tracker) = &mut state.tracker
        {
            tracker.window = window.filter(|value| *value > 0).or(tracker.window);
        }
    }
    pub fn set_outside_memory(&self, outside: bool) {
        if let Ok(mut state) = self.inner.lock() {
            state.enricher.ledger.set_watch_outside(outside);
        }
    }
    pub fn new_phase(&self, label: &str, native: Option<&str>) {
        if let Ok(mut state) = self.inner.lock() {
            if state.finished || !["main", "recovery"].contains(&label) {
                return;
            }
            if let Ok(mut observed) = state.observation.lock() {
                observed.new_phase(label, native);
            }
            if !state.costs.contains_key(label) {
                let previous = native
                    .and_then(|id| cost_observation::amount(&state.session["cost_totals"][id]));
                let cost = infra_host::provider::RunCost::new(
                    &state.provider,
                    native.map(str::to_string),
                    previous,
                );
                state.costs.insert(label.into(), cost);
            }
        }
    }
    pub fn raw(&self, label: &str, event: &Value) {
        if let Ok(mut state) = self.inner.lock() {
            if state.finished {
                return;
            }
            if let Ok(mut observed) = state.observation.lock() {
                observed.raw(label, event);
            }
            if let Some(cost) = state.costs.get_mut(label) {
                cost.observe(event);
            }
            if let Some(tracker) = &mut state.tracker {
                tracker.observe(event);
            }
            state.poll(false);
            state.compactions();
        }
    }
    /// Normalized tool events have already passed provider normalization. This only
    /// enriches display fields; it never attributes USD or changes the native budget.
    pub fn clean(&self, _label: &str, event: &Value) -> Option<Value> {
        let Ok(mut state) = self.inner.lock() else {
            return Some(event.clone());
        };
        if state.finished {
            return Some(event.clone());
        }
        if event["kind"] == "plan" {
            return state.enricher.plan(event);
        }
        let mut clean = event.clone();
        if let Some(text) = clean["text"].as_str() {
            clean["text"] = text.chars().take(32000).collect::<String>().into();
        }
        let count = state.count;
        let bytes = state.output_bytes;
        let display = if event["kind"] == "tool" {
            state.enricher.take(&mut clean, count, bytes)
        } else {
            None
        };
        state.count = state.count.saturating_add(1);
        state.output_bytes = state.output_bytes.saturating_add(
            infra_core::domain::json::python_default(&clean, false, true, true)
                .map(|value| value.len())
                .unwrap_or(0),
        );
        if let Some(display) = display {
            if let Some(fields) = display.as_object() {
                for (key, value) in fields {
                    clean[key] = value.clone();
                }
            }
        } else if event["kind"] == "tool"
            && let Some(notice) = state.enricher.notice()
        {
            state.extra.push(notice);
        }
        if let Some(native) = clean["native_session_id"]
            .as_str()
            .filter(|value| !value.is_empty() && value.chars().count() <= 1024)
        {
            if state.native.as_deref().is_none_or(|old| old == native) {
                state.native = Some(native.into());
            } else {
                if let Ok(mut observed) = state.observation.lock() {
                    observed.diagnostic("main", "identity_changed", None);
                }
            }
        }
        Some(clean)
    }
    pub fn normalized(&self, label: &str, event: &Value) -> Option<Value> {
        self.clean(label, event)
    }
    pub fn boundary(&self, label: &str, signal: &str, details: &Value) {
        if let Ok(mut state) = self.inner.lock() {
            if state.finished {
                return;
            }
            let code = details["code"].as_i64();
            if ["guard_exit_before_cleanup", "guard_exit_after_cleanup"].contains(&signal)
                && let Some(code) = code
            {
                state.last_exit.insert(label.into(), code);
            }
            if signal == "guardian_spawned" {
                if state.provider == "codex" && state.live.is_none() {
                    state.live = Some(CodexLive::new(&state.project, context_usage::unix_now()));
                    state.live_polled = Instant::now();
                }
                if let Some(live) = &mut state.live {
                    live.fill.until = None;
                }
                if let Ok(mut observed) = state.observation.lock() {
                    observed.process(label, "started");
                }
                return;
            }
            if signal == "input_written" {
                if let Ok(mut observed) = state.observation.lock() {
                    observed.transport(label, count(&details["bytes"]), "written");
                }
                return;
            }
            if signal == "input_closed" {
                return;
            }
            if signal == "exited" {
                if let Some(code) = code {
                    state.last_exit.insert(label.into(), code);
                }
                if let Ok(mut observed) = state.observation.lock() {
                    observed.diagnostic(label, "guard_exit_before_cleanup", code);
                }
                return;
            }
            if let Ok(mut observed) = state.observation.lock() {
                observed.diagnostic(label, signal, code);
                if signal == "cleanup_confirmed" {
                    if let Some(code) = state.last_exit.get(label) {
                        observed.diagnostic(label, "guard_exit_after_cleanup", Some(*code));
                    }
                    observed.process(
                        label,
                        if code
                            .or_else(|| state.last_exit.get(label).copied())
                            .is_some_and(|code| code == 0)
                        {
                            "finished"
                        } else {
                            "failed"
                        },
                    );
                }
            }
            if signal == "cleanup_confirmed" {
                state.poll(true);
                state.compactions();
            }
        }
    }
    pub fn process(&self, label: &str, status: &str) {
        if let Ok(mut state) = self.inner.lock() {
            if state.finished {
                return;
            }
            if status == "started" && state.provider == "codex" {
                if state.live.is_none() {
                    state.live = Some(CodexLive::new(&state.project, context_usage::unix_now()));
                }
                if let Some(live) = &mut state.live {
                    live.fill.until = None;
                }
            }
            if let Ok(mut observed) = state.observation.lock() {
                observed.process(label, status);
            }
        }
    }
    pub fn payload(&self, name: &str, text: &str) {
        if let Ok(state) = self.inner.lock()
            && let Ok(mut observed) = state.observation.lock()
        {
            observed.payload(name, text);
        }
    }
    pub fn transport(&self, label: &str, bytes: Option<usize>, status: &str) {
        if let Ok(state) = self.inner.lock()
            && let Ok(mut observed) = state.observation.lock()
        {
            observed.transport(label, bytes.map(|value| value as u64), status);
        }
    }
    pub fn usd(&self, label: &str, own: Option<f64>, reported: Option<f64>) {
        if let Ok(state) = self.inner.lock()
            && let Ok(mut observed) = state.observation.lock()
        {
            observed.usd(label, own, reported, state.costs.get(label));
        }
    }
    pub fn usd_with_cost(
        &self,
        label: &str,
        own: Option<f64>,
        reported: Option<f64>,
        cost: &infra_host::provider::RunCost,
    ) {
        if let Ok(state) = self.inner.lock()
            && let Ok(mut observed) = state.observation.lock()
        {
            observed.usd(label, own, reported, Some(cost));
        }
    }
    pub fn duration(&self, name: &str, seconds: f64) {
        if let Ok(state) = self.inner.lock()
            && let Ok(mut observed) = state.observation.lock()
        {
            observed.duration(name, seconds);
        }
    }
    pub fn note(&self, name: &str) {
        if let Ok(state) = self.inner.lock()
            && let Ok(mut observed) = state.observation.lock()
        {
            observed.note(name);
        }
    }
    pub fn operation(&self, operation: &str, action: Option<&str>) -> Option<LocalOperation> {
        self.inner
            .lock()
            .ok()
            .map(|state| LocalOperation::new(state.observation.clone(), operation, action))
    }
    pub fn source_hints(
        &self,
        calls: Option<u64>,
        outcome: &str,
        seconds: Option<f64>,
        references: Option<u64>,
        transport: Option<&Value>,
    ) {
        if let Ok(state) = self.inner.lock()
            && let Ok(mut observed) = state.observation.lock()
        {
            observed.source_hints(calls, outcome, seconds, references, transport);
        }
    }
    pub fn context(&self) -> Option<Value> {
        self.inner.lock().ok().and_then(|mut state| state.context())
    }
    pub fn context_if_due(&self) -> Option<Value> {
        let mut state = self.inner.lock().ok()?;
        state.poll(false);
        if !state.tracker.as_mut().is_some_and(ContextTracker::due) {
            return None;
        }
        state.context()
    }
    pub fn receipt(&self) -> Value {
        self.inner
            .lock()
            .map(|state| state.enricher.ledger.receipt())
            .unwrap_or_else(|_| object!({"limited":true,"memory":null}))
    }
    pub fn take_events(&self) -> Vec<Value> {
        self.inner
            .lock()
            .map(|mut state| std::mem::take(&mut state.extra))
            .unwrap_or_default()
    }
    pub fn snapshot(&self) -> Value {
        self.inner.lock().ok().and_then(|state|state.observation.lock().ok().map(|observed|observed.snapshot())).unwrap_or_else(||object!({"schema":"harness-full-cost-v1","state":"unavailable","observer_incomplete":true}))
    }
    pub fn finish(&self, outcome: &str) -> Value {
        let Ok(mut state) = self.inner.lock() else {
            return object!({"schema":"harness-full-cost-v1","state":"unavailable","observer_incomplete":true});
        };
        if !state.finished {
            state.poll(true);
            state.compactions();
            state.finished = true;
        }
        let observation = state.observation.clone();
        drop(state);
        observation.lock().map(|mut observed|observed.finish(Some(outcome))).unwrap_or_else(|_|object!({"schema":"harness-full-cost-v1","state":"unavailable","observer_incomplete":true}))
    }
    pub fn generation(&self) -> Option<String> {
        self.inner.lock().ok().map(|state| state.generation.clone())
    }
}
