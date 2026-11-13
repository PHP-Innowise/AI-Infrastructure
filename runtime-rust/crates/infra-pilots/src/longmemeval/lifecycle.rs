use super::{
    cost::{Close, Ledger, Outcome, Report, Ticket as CostTicket},
    registry::{Memory, PROFILE as BRIDGE_PROFILE, QueryReceipt},
    usage::{LIMIT, Usage},
};
use crate::strict;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc, time::Instant};
use uuid::Uuid;
pub const SYSTEM: &str = "Use reference memory; verify canonical sources.";
pub trait Clock: Send {
    fn now_ns(&mut self) -> Result<u64>;
}
pub struct MonotonicClock(Instant);
impl Default for MonotonicClock {
    fn default() -> Self {
        Self(Instant::now())
    }
}
impl Clock for MonotonicClock {
    fn now_ns(&mut self) -> Result<u64> {
        self.0
            .elapsed()
            .as_nanos()
            .try_into()
            .context("clock_bound")
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Question {
    pub question: String,
    pub query_image: Option<String>,
}
#[derive(Clone)]
pub struct Construction {
    owner: Uuid,
    body: Arc<Vec<u8>>,
}
impl Construction {
    pub fn body(&self) -> &[u8] {
        &self.body
    }
    pub fn sha256(&self) -> String {
        strict::sha(&self.body)
    }
}
#[derive(Clone, PartialEq, Eq)]
pub struct DispatchTicket {
    pub(crate) owner: Uuid,
    pub(crate) id: Uuid,
}
impl std::fmt::Debug for DispatchTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<opaque local callback ticket>")
    }
}
#[derive(Debug, Clone)]
pub struct CallbackReport {
    pub outcome: Outcome,
    pub value: Value,
    pub usage: Usage,
    pub usd_micros: Option<u64>,
    pub billing: String,
}
impl CallbackReport {
    pub fn validate(&self, phase: &str) -> Result<()> {
        if self.outcome == Outcome::Succeeded {
            if phase == "reader" {
                strict::text(&self.value, 32768, false, true)?;
            } else if !self.value.is_boolean() && !self.value.is_null() {
                bail!("controller_grade");
            }
        } else if !self.value.is_null() {
            bail!("controller_report");
        }
        if self.usage.fields().iter().flatten().any(|v| *v > LIMIT)
            || self.usd_micros.is_some_and(|v| v > LIMIT)
            || !["unknown", "provider_reported", "invoice_allocated"]
                .contains(&self.billing.as_str())
        {
            bail!("controller_report");
        }
        Ok(())
    }
}
pub trait Callback {
    fn invoke(&mut self, body: &[u8]) -> Result<CallbackReport>;
}
impl<F: FnMut(&[u8]) -> Result<CallbackReport>> Callback for F {
    fn invoke(&mut self, b: &[u8]) -> Result<CallbackReport> {
        self(b)
    }
}
#[derive(Default)]
struct Slot {
    query_outcome: String,
    local_ordinal: Option<usize>,
    receipt: Option<QueryReceipt>,
    context: Option<Vec<Value>>,
    reader_body: Option<Vec<u8>>,
}
#[derive(Default)]
struct Unit {
    issued: usize,
    entries: usize,
    bytes: u64,
    outcome: String,
    value: Value,
    closed: bool,
}
pub(crate) struct Dispatch {
    pub phase: String,
    pub ordinal: usize,
    pub body: Vec<u8>,
    pub consumed: bool,
}
pub(crate) struct Entered {
    pub id: Uuid,
    pub phase: String,
    pub ordinal: usize,
    pub body: Vec<u8>,
    pub observed: CostTicket,
    pub started: Option<u64>,
}
pub struct Controller {
    memory: Box<dyn Memory>,
    questions: Vec<Question>,
    budget: usize,
    clock: Box<dyn Clock>,
    ledger: Ledger,
    owner: Uuid,
    slots: Vec<Slot>,
    units: BTreeMap<(String, usize), Unit>,
    pub(crate) tickets: BTreeMap<Uuid, Dispatch>,
    held: usize,
    busy: bool,
    closed: bool,
    joined: bool,
    blocked: bool,
    native_teardown: bool,
    seal: Option<Construction>,
    final_view: Option<Value>,
    initial_inserts: u64,
    next_local: usize,
    entries: usize,
    faults: usize,
    controller_profile: &'static str,
    construction_profile: &'static str,
    clock_abort: Option<Box<dyn std::any::Any + Send>>,
}
impl Controller {
    pub fn new(
        memory: Box<dyn Memory>,
        questions: Vec<Question>,
        reader_budget: usize,
        clock: Option<Box<dyn Clock>>,
    ) -> Result<Self> {
        if !(1..=32).contains(&questions.len()) || !(32..=65536).contains(&reader_budget) {
            bail!("controller_parameters");
        }
        for q in &questions {
            strict::text(&json!(q.question), 8192, true, true)?;
            if q.query_image.is_some() {
                bail!("controller_images_unsupported");
            }
        }
        let view = memory.snapshot()?;
        if view["profile"] != BRIDGE_PROFILE
            || view["closed"] != false
            || view["local_observation"]["ledger"]["horizon"].as_u64()
                != Some(questions.len() as u64)
            || view["local_observation"]["observed"]["query_calls"] != 0
        {
            bail!("controller_fresh_memory");
        }
        let initial = view["local_observation"]["observed"]["insert_calls"]
            .as_u64()
            .context("controller_fresh_memory")?;
        let count = questions.len();
        let held = strict::wire(&json!(questions))?.len();
        let mut units = BTreeMap::new();
        for phase in ["reader", "judge"] {
            for i in 0..count {
                units.insert(
                    (phase.into(), i),
                    Unit {
                        outcome: "unlaunched".into(),
                        ..Unit::default()
                    },
                );
            }
        }
        Ok(Self {
            memory,
            questions,
            budget: reader_budget,
            clock: clock.unwrap_or_else(|| Box::new(MonotonicClock::default())),
            ledger: Ledger::new(count)?,
            owner: Uuid::new_v4(),
            slots: (0..count)
                .map(|_| Slot {
                    query_outcome: "unlaunched".into(),
                    ..Slot::default()
                })
                .collect(),
            units,
            tickets: BTreeMap::new(),
            held,
            busy: false,
            closed: false,
            joined: false,
            blocked: false,
            native_teardown: false,
            seal: None,
            final_view: None,
            initial_inserts: initial,
            next_local: 0,
            entries: 0,
            faults: 0,
            controller_profile: "longmemeval/local-text-callback-controller-v1",
            construction_profile: "longmemeval/local-text-callback-construction-v1",
            clock_abort: None,
        })
    }
    pub(crate) fn set_profiles(&mut self, controller: &'static str, construction: &'static str) {
        self.controller_profile = controller;
        self.construction_profile = construction;
    }
    fn ready(&self, finalizing: bool) -> Result<()> {
        if self.closed || self.busy || self.blocked && !finalizing {
            bail!("controller_lifecycle");
        }
        Ok(())
    }
    fn ordinal(&self, i: usize) -> Result<()> {
        if i >= self.questions.len() {
            bail!("controller_ordinal");
        }
        Ok(())
    }
    fn hold(&mut self, n: usize) -> Result<()> {
        if self.held.checked_add(n).is_none_or(|n| n > 8388608) {
            bail!("controller_held_bound");
        }
        self.held += n;
        Ok(())
    }
    fn now(&mut self) -> Option<u64> {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.clock.now_ns())) {
            Ok(Ok(n)) if n <= LIMIT => Some(n),
            Ok(_) => {
                self.faults += 1;
                None
            }
            Err(payload) => {
                self.faults += 1;
                if self.clock_abort.is_none() {
                    self.clock_abort = Some(payload);
                }
                None
            }
        }
    }
    pub fn query(&mut self, i: usize) -> Result<Vec<Value>> {
        self.ready(false)?;
        self.ordinal(i)?;
        if self.seal.is_some() || self.slots[i].query_outcome != "unlaunched" {
            bail!("controller_query_reused");
        }
        let before = self.memory.snapshot()?;
        let observed = &before["local_observation"]["observed"];
        if observed["insert_calls"].as_u64() != Some(self.initial_inserts) {
            self.blocked = true;
            self.faults += 1;
            bail!("controller_external_insert");
        }
        if observed["query_calls"].as_u64() != Some(self.next_local as u64) {
            self.faults += 1;
            bail!("controller_external_query");
        }
        let ordinal = self.next_local;
        self.slots[i].query_outcome = "pending".into();
        self.busy = true;
        let q = self.questions[i].question.clone();
        let caught =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Vec<Value>> {
                self.memory
                    .set_query_context(&format!("controller-{}", Uuid::new_v4().simple()))?;
                let context = self.memory.query(&q, None)?;
                validate_context(&context)?;
                let hook = self.memory.post_query_hook(&q, None, &context)?;
                let receipt: QueryReceipt = serde_json::from_value(hook["query_receipt"].clone())?;
                validate_receipt(&receipt, ordinal)?;
                if receipt.outcome != "succeeded"
                    || !receipt.native_metadata_bound
                    || !["delivered", "no_evidence", "sanitized_empty"]
                        .contains(&receipt.state.as_str())
                {
                    bail!("controller_query_not_succeeded");
                }
                self.hold(strict::wire(&json!(context))?.len())?;
                self.slots[i].context = Some(context.clone());
                self.slots[i].query_outcome = "succeeded".into();
                Ok(context)
            }));
        let mut panic = None;
        let result = match caught {
            Ok(result) => result,
            Err(payload) => {
                panic = Some(payload);
                Err(anyhow::anyhow!("controller_cancelled"))
            }
        };
        if result.is_err() && self.slots[i].query_outcome == "pending" {
            self.slots[i].query_outcome = if panic.is_some() {
                "cancelled"
            } else {
                "failed"
            }
            .into();
        }
        let after = self.memory.snapshot();
        let clear = self.memory.clear_query_context();
        self.busy = false;
        if let Ok(after) = after {
            let observed = &after["local_observation"]["observed"];
            let count = observed["query_calls"]
                .as_u64()
                .context("controller_external_query")? as usize;
            if count == ordinal + 1 {
                let receipt: QueryReceipt =
                    serde_json::from_value(observed["query_receipts"][ordinal].clone())?;
                validate_receipt(&receipt, ordinal)?;
                self.slots[i].local_ordinal = Some(ordinal);
                self.slots[i].receipt = Some(receipt);
                self.next_local += 1;
            } else if count != ordinal {
                self.faults += 1;
                bail!("controller_external_query");
            }
        }
        clear?;
        if let Some(payload) = panic {
            std::panic::resume_unwind(payload);
        }
        result
    }
    fn issue(&mut self, phase: &str, i: usize, body: Vec<u8>) -> Result<DispatchTicket> {
        let unit = self
            .units
            .get_mut(&(phase.into(), i))
            .context("controller_ordinal")?;
        if unit.issued >= 2
            || ["pending", "succeeded", "cancelled"].contains(&unit.outcome.as_str())
        {
            bail!("controller_attempt_budget");
        }
        unit.issued += 1;
        unit.outcome = "pending".into();
        let id = Uuid::new_v4();
        self.tickets.insert(
            id,
            Dispatch {
                phase: phase.into(),
                ordinal: i,
                body,
                consumed: false,
            },
        );
        Ok(DispatchTicket {
            owner: self.owner,
            id,
        })
    }
    pub fn prepare_reader(&mut self, i: usize) -> Result<DispatchTicket> {
        self.prepare_reader_with(i, strict::wire)
    }
    pub(crate) fn prepare_reader_with<F>(
        &mut self,
        i: usize,
        mut compile: F,
    ) -> Result<DispatchTicket>
    where
        F: FnMut(&Value) -> Result<Vec<u8>>,
    {
        self.ready(false)?;
        self.ordinal(i)?;
        if self.seal.is_some() || self.slots[i].query_outcome != "succeeded" {
            bail!("controller_reader_state");
        }
        if self.slots[i].reader_body.is_none() {
            let mut v = json!({"system":SYSTEM,"question":self.questions[i].question,"memory_context":self.slots[i].context});
            let mut b = compile(&v)?;
            if b.len() > self.budget {
                v["memory_context"] = json!([]);
                b = compile(&v)?;
            }
            if b.len() > self.budget {
                bail!("controller_request_bound");
            }
            self.hold(b.len())?;
            self.slots[i].reader_body = Some(b);
        }
        self.issue("reader", i, self.slots[i].reader_body.clone().unwrap())
    }
    pub fn prepare_judge(
        &mut self,
        i: usize,
        construction: &Construction,
    ) -> Result<DispatchTicket> {
        self.prepare_judge_with(i, construction, strict::wire)
    }
    pub(crate) fn prepare_judge_with<F>(
        &mut self,
        i: usize,
        construction: &Construction,
        mut compile: F,
    ) -> Result<DispatchTicket>
    where
        F: FnMut(&Value) -> Result<Vec<u8>>,
    {
        self.ready(false)?;
        self.ordinal(i)?;
        let seal = self
            .seal
            .as_ref()
            .context("controller_construction_binding")?;
        if construction.owner != self.owner || !Arc::ptr_eq(&construction.body, &seal.body) {
            bail!("controller_construction_binding");
        }
        let reader = &self.units[&("reader".into(), i)];
        if reader.outcome != "succeeded" {
            bail!("controller_judge_state");
        }
        let body = compile(
            &json!({"question":self.questions[i].question,"answer":reader.value,"construction_sha256":seal.sha256()}),
        )?;
        if body.len() > 65536 {
            bail!("controller_request_bound");
        }
        self.hold(body.len())?;
        self.issue("judge", i, body)
    }
    pub(crate) fn enter(&mut self, ticket: &DispatchTicket, phase: &str) -> Result<Entered> {
        self.ready(false)?;
        if ticket.owner != self.owner {
            bail!("controller_ticket_binding");
        }
        let d = self
            .tickets
            .get(&ticket.id)
            .context("controller_ticket_binding")?;
        if d.phase != phase || d.consumed || phase == "reader" && self.seal.is_some() {
            bail!("controller_ticket_reused");
        }
        let ordinal = d.ordinal;
        let body = d.body.clone();
        let observed = self.ledger.begin_call(
            phase,
            Some(ordinal),
            &Uuid::new_v4().to_string(),
            &Uuid::new_v4().to_string(),
        )?;
        self.tickets.get_mut(&ticket.id).unwrap().consumed = true;
        self.busy = true;
        let started = self.now();
        if let Some(payload) = self.clock_abort.take() {
            let _ = self.ledger.finish_call(
                &observed,
                Report {
                    outcome: Outcome::Cancelled,
                    usage: Usage::default(),
                    usd_micros: None,
                    billing: "unknown".into(),
                    component_ns: None,
                },
            );
            self.units
                .get_mut(&(phase.into(), ordinal))
                .unwrap()
                .outcome = "cancelled".into();
            self.busy = false;
            std::panic::resume_unwind(payload);
        }
        let unit = self.units.get_mut(&(phase.into(), ordinal)).unwrap();
        unit.entries += 1;
        unit.bytes = unit.bytes.saturating_add(body.len() as u64);
        self.entries += 1;
        Ok(Entered {
            id: ticket.id,
            phase: phase.into(),
            ordinal,
            body,
            observed,
            started,
        })
    }
    pub(crate) fn finish(
        &mut self,
        entered: Entered,
        report: Result<CallbackReport>,
    ) -> Result<Value> {
        let end = self.now();
        let clock_abort = self.clock_abort.take();
        let duration = entered.started.zip(end).and_then(|(s, e)| e.checked_sub(s));
        let mut report = match report {
            Ok(r) => r,
            Err(error) => {
                let u = self
                    .units
                    .get_mut(&(entered.phase.clone(), entered.ordinal))
                    .unwrap();
                u.value = Value::Null;
                u.outcome = "failed".into();
                self.ledger.finish_call(
                    &entered.observed,
                    Report {
                        outcome: Outcome::Failed,
                        usage: Usage::default(),
                        usd_micros: None,
                        billing: "unknown".into(),
                        component_ns: duration,
                    },
                )?;
                self.busy = false;
                return Err(error);
            }
        };
        if let Err(error) = report.validate(&entered.phase) {
            report = CallbackReport {
                outcome: Outcome::Failed,
                value: Value::Null,
                usage: Usage::default(),
                usd_micros: None,
                billing: "unknown".into(),
            };
            self.ledger.finish_call(
                &entered.observed,
                Report {
                    outcome: report.outcome,
                    usage: report.usage,
                    usd_micros: None,
                    billing: "unknown".into(),
                    component_ns: duration,
                },
            )?;
            self.units
                .get_mut(&(entered.phase.clone(), entered.ordinal))
                .unwrap()
                .outcome = "failed".into();
            self.busy = false;
            return Err(error);
        }
        let hold_error = if entered.phase == "reader" && report.outcome == Outcome::Succeeded {
            self.hold(report.value.as_str().unwrap().len()).err()
        } else {
            None
        };
        if hold_error.is_some() {
            report.outcome = Outcome::Failed;
            report.value = Value::Null;
            self.blocked = true;
        }
        let u = self
            .units
            .get_mut(&(entered.phase.clone(), entered.ordinal))
            .unwrap();
        u.outcome = match report.outcome {
            Outcome::Succeeded => "succeeded",
            Outcome::Failed => "failed",
            Outcome::Cancelled => "cancelled",
        }
        .into();
        u.value = report.value.clone();
        let finish = self.ledger.finish_call(
            &entered.observed,
            Report {
                outcome: report.outcome,
                usage: report.usage,
                usd_micros: report.usd_micros,
                billing: report.billing,
                component_ns: duration,
            },
        );
        self.busy = false;
        finish?;
        if let Some(error) = hold_error {
            return Err(error);
        }
        if let Some(payload) = clock_abort {
            std::panic::resume_unwind(payload);
        }
        Ok(report.value)
    }
    pub(crate) fn finish_unwind(
        &mut self,
        e: Entered,
        usage: Usage,
        payload: Box<dyn std::any::Any + Send>,
    ) -> ! {
        let report = CallbackReport {
            outcome: Outcome::Cancelled,
            value: Value::Null,
            usage,
            usd_micros: None,
            billing: "unknown".into(),
        };
        let _ =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.finish(e, Ok(report))));
        std::panic::resume_unwind(payload)
    }
    pub fn dispatch_reader(
        &mut self,
        ticket: &DispatchTicket,
        callback: &mut dyn Callback,
    ) -> Result<Value> {
        let e = self.enter(ticket, "reader")?;
        let body = e.body.clone();
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback.invoke(&body))) {
            Ok(reply) => self.finish(e, reply),
            Err(payload) => self.finish_unwind(e, Usage::default(), payload),
        }
    }
    pub fn dispatch_judge<F>(&mut self, ticket: &DispatchTicket, factory: F) -> Result<Value>
    where
        F: FnOnce() -> Result<Box<dyn Callback>>,
    {
        let e = self.enter(ticket, "judge")?;
        let body = e.body.clone();
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            factory().and_then(|mut c| c.invoke(&body))
        })) {
            Ok(reply) => self.finish(e, reply),
            Err(payload) => self.finish_unwind(e, Usage::default(), payload),
        }
    }
    fn join_local(&mut self, final_view: &Value) -> Result<()> {
        if self.joined {
            return Ok(());
        }
        let local = &final_view["local_observation"];
        let observed = &local["observed"];
        if final_view["profile"] != BRIDGE_PROFILE
            || final_view["closed"] != true
            || local["profile"] != "native-fts5/observed-local-operations-v1"
            || local["ledger"]["profile"] != "memory-benchmark-ledger-v1"
            || local["ledger"]["frozen"] != true
            || local["ledger"]["horizon"].as_u64() != Some(self.questions.len() as u64)
            || observed["query_calls"].as_u64() != Some(self.next_local as u64)
            || observed["insert_calls"].as_u64() != Some(self.initial_inserts)
        {
            bail!("controller_local_final");
        }
        let receipts = observed["query_receipts"]
            .as_array()
            .context("controller_local_final")?;
        if receipts.len() != self.next_local {
            bail!("controller_local_mapping");
        }
        let mut mapped = vec![];
        for s in &self.slots {
            if let Some(i) = s.local_ordinal {
                mapped.push(i);
                if json!(s.receipt) != receipts[i] {
                    bail!("controller_local_changed");
                }
            }
        }
        mapped.sort();
        if mapped != (0..self.next_local).collect::<Vec<_>>() {
            bail!("controller_local_mapping");
        }
        let model = if observed["local_token_profile"] == "pinned_local_no_model" {
            "none_observed_local"
        } else {
            "unknown"
        };
        let coverage = if observed["observation_incomplete"] == true
            || observed["initialization_outcome"] != "succeeded"
        {
            "incomplete"
        } else {
            "complete"
        };
        self.ledger.close_unit(
            "build",
            None,
            Close {
                coverage: coverage.into(),
                model_work: model.into(),
                billing_complete: false,
                component_ns: observed["build_component_ns"].as_u64(),
                constructed_bytes: observed["insert_ingress_bytes"].as_u64(),
            },
        )?;
        for (i, s) in self.slots.iter().enumerate() {
            self.ledger.close_unit(
                "query",
                Some(i),
                Close {
                    coverage: if s.receipt.is_some() {
                        coverage
                    } else {
                        "incomplete"
                    }
                    .into(),
                    model_work: if s.receipt.is_some() {
                        model
                    } else {
                        "unknown"
                    }
                    .into(),
                    billing_complete: false,
                    component_ns: s.receipt.as_ref().and_then(|r| r.component_ns),
                    constructed_bytes: s
                        .receipt
                        .as_ref()
                        .and_then(|r| r.input_bytes.checked_add(r.output_bytes)),
                },
            )?;
        }
        self.joined = true;
        Ok(())
    }
    fn close_units(&mut self, phase: &str) -> Result<()> {
        for i in 0..self.questions.len() {
            let u = self.units.get_mut(&(phase.into(), i)).unwrap();
            if !u.closed {
                self.ledger.close_unit(
                    phase,
                    Some(i),
                    Close {
                        coverage: "incomplete".into(),
                        model_work: if u.entries > 0 {
                            "reported_calls"
                        } else {
                            "unknown"
                        }
                        .into(),
                        billing_complete: false,
                        component_ns: None,
                        constructed_bytes: (u.entries > 0).then_some(u.bytes),
                    },
                )?;
                u.closed = true;
                if phase == "judge" {
                    self.ledger.grade(
                        i,
                        if u.outcome == "succeeded" {
                            u.value.as_bool()
                        } else {
                            None
                        },
                    )?;
                }
            }
        }
        Ok(())
    }
    pub fn seal_construction(&mut self) -> Result<Construction> {
        self.ready(false)?;
        if let Some(s) = &self.seal {
            return Ok(s.clone());
        }
        let final_view = if !self.native_teardown {
            self.native_teardown = true;
            match self.memory.close() {
                Ok(v) => v,
                Err(_) => {
                    self.faults += 1;
                    self.memory.snapshot()?
                }
            }
        } else {
            self.memory.snapshot()?
        };
        if let Err(e) = self.join_local(&final_view) {
            self.blocked = true;
            return Err(e);
        }
        self.close_units("reader")?;
        let slots=self.slots.iter().enumerate().map(|(i,s)|{let reader=&self.units[&("reader".into(),i)];json!({"ordinal":i,"question":self.questions[i].question,"query_outcome":s.query_outcome,"local_ordinal":s.local_ordinal,"reader_request":s.reader_body.as_ref().and_then(|b|serde_json::from_slice::<Value>(b).ok()),"reader_outcome":reader.outcome,"reader_answer":reader.value})}).collect::<Vec<_>>();
        let body = strict::wire(
            &json!({"profile":self.construction_profile,"horizon":self.questions.len(),"slots":slots}),
        )?;
        self.hold(body.len())?;
        let seal = Construction {
            owner: self.owner,
            body: Arc::new(body),
        };
        self.seal = Some(seal.clone());
        Ok(seal)
    }
    pub fn snapshot(&self) -> Result<Value> {
        if let Some(v) = &self.final_view {
            return Ok(v.clone());
        }
        Ok(
            json!({"profile":self.controller_profile,"closed":self.closed,"local_imports":usize::from(self.joined),"planned_to_local":self.slots.iter().map(|s|s.local_ordinal).collect::<Vec<_>>(),"callback_entries":self.entries,"pending_tickets":self.tickets.values().filter(|t|!t.consumed).count(),"observation_faults":self.faults,"provider_coverage":"UNKNOWN","construction_blocked":self.blocked,"native_teardown_attempted":self.native_teardown,"provider_usage_verified":false,"construction_sha256":self.seal.as_ref().map(Construction::sha256),"ledger":self.ledger.snapshot()?}),
        )
    }
    pub fn finalize(&mut self) -> Result<Value> {
        if self.closed {
            return self.snapshot();
        }
        self.ready(true)?;
        if !self.blocked && self.seal_construction().is_err() {
            self.faults += 1;
        }
        if !self.native_teardown {
            self.native_teardown = true;
            if self.memory.close().is_err() {
                self.faults += 1;
            }
        }
        self.close_units("reader")?;
        self.close_units("judge")?;
        self.closed = true;
        self.ledger.finalize()?;
        let v = self.snapshot()?;
        self.final_view = Some(v.clone());
        Ok(v)
    }
}
pub fn validate_context(c: &[Value]) -> Result<()> {
    if c.len() > 1 {
        bail!("controller_text_profile");
    }
    for v in c {
        strict::keys(v, &["type", "value"], &["type", "value"])?;
        if v["type"] != "text" {
            bail!("controller_text_profile");
        }
        strict::text(&v["value"], 262144, false, true)?;
    }
    if strict::wire(&json!(c))?.len() > 262144 {
        bail!("controller_context_bound");
    }
    Ok(())
}
pub fn validate_receipt(r: &QueryReceipt, i: usize) -> Result<()> {
    if r.ordinal != i
        || !["succeeded", "failed", "cancelled", "unknown"].contains(&r.outcome.as_str())
        || ![
            "delivered",
            "no_evidence",
            "sanitized_empty",
            "refresh_failed",
            "cleanup_failed",
            "failed",
            "cancelled",
            "input_refused",
            "unmeasured",
        ]
        .contains(&r.state.as_str())
        || r.input_bytes
            .checked_add(r.output_bytes)
            .is_none_or(|v| v > LIMIT)
        || r.component_ns.is_some_and(|v| v > LIMIT)
    {
        bail!("controller_local_receipt");
    }
    Ok(())
}
