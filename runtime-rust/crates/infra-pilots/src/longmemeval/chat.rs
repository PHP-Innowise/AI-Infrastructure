use super::{
    cost::Outcome,
    lifecycle::{CallbackReport, Construction, Controller, DispatchTicket},
    usage::CompletionCapture,
};
use crate::strict;
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;
pub type Transport = Box<dyn FnMut(&[u8]) -> Result<Vec<u8>> + Send>;
pub type GradeDecoder = Box<dyn FnMut(&[u8]) -> Result<Option<bool>> + Send>;
pub(crate) struct ChatRow {
    pub phase: String,
    pub ordinal: usize,
    pub body: Vec<u8>,
    pub boundaries: u64,
    pub response_bytes: Option<usize>,
    pub capture: CompletionCapture,
    pub outcome: String,
}
#[derive(Clone)]
pub(crate) enum Style {
    Injected,
    Upstream {
        reader_timeout: f64,
        judge_timeout: f64,
    },
}
pub struct ChatController {
    pub(crate) core: Controller,
    pub(crate) reader_model: String,
    pub(crate) judge_model: String,
    pub(crate) reader_max: u64,
    pub(crate) judge_max: u64,
    pub(crate) style: Style,
    pub(crate) rows: BTreeMap<Uuid, ChatRow>,
}
impl ChatController {
    pub fn new(
        mut core: Controller,
        reader_model: String,
        judge_model: String,
        reader_max: u64,
        judge_max: u64,
    ) -> Result<Self> {
        for m in [&reader_model, &judge_model] {
            strict::text(&json!(m), 128, true, false)?;
        }
        if !(1..=65536).contains(&reader_max) || !(1..=65536).contains(&judge_max) {
            bail!("chat_parameters");
        }
        core.set_profiles(
            "longmemeval/injected-chat-controller-v1",
            "longmemeval/injected-chat-reader-construction-v1",
        );
        Ok(Self {
            core,
            reader_model,
            judge_model,
            reader_max,
            judge_max,
            style: Style::Injected,
            rows: BTreeMap::new(),
        })
    }
    pub(crate) fn arguments(
        local: &Value,
        model: &str,
        maximum: u64,
        style: &Style,
        phase: &str,
    ) -> Result<Vec<u8>> {
        let mut local = local.clone();
        local
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("chat_local_request"))?
            .remove("system");
        let mut body = json!({"model":model,"messages":[{"role":"system","content":super::lifecycle::SYSTEM},{"role":"user","content":String::from_utf8(strict::wire(&local)?)?}],"max_completion_tokens":maximum});
        match style {
            Style::Injected => {
                body["n"] = json!(1);
                body["stream"] = json!(false);
            }
            Style::Upstream {
                reader_timeout,
                judge_timeout,
            } => {
                body["timeout"] = json!(if phase == "reader" {
                    reader_timeout
                } else {
                    judge_timeout
                })
            }
        }
        strict::wire(&body)
    }
    pub fn query(&mut self, ordinal: usize) -> Result<Vec<Value>> {
        self.core.query(ordinal)
    }
    pub fn prepare_reader(&mut self, i: usize) -> Result<DispatchTicket> {
        let model = self.reader_model.clone();
        let maximum = self.reader_max;
        let style = self.style.clone();
        let ticket = self.core.prepare_reader_with(i, move |v| {
            Self::arguments(v, &model, maximum, &style, "reader")
        })?;
        self.record(&ticket);
        Ok(ticket)
    }
    pub fn prepare_judge(&mut self, i: usize, seal: &Construction) -> Result<DispatchTicket> {
        let model = self.judge_model.clone();
        let maximum = self.judge_max;
        let style = self.style.clone();
        let ticket = self.core.prepare_judge_with(i, seal, move |v| {
            Self::arguments(v, &model, maximum, &style, "judge")
        })?;
        self.record(&ticket);
        Ok(ticket)
    }
    pub(crate) fn record(&mut self, t: &DispatchTicket) {
        let d = &self.core.tickets[&t.id];
        self.rows.insert(
            t.id,
            ChatRow {
                phase: d.phase.clone(),
                ordinal: d.ordinal,
                body: d.body.clone(),
                boundaries: 0,
                response_bytes: None,
                capture: CompletionCapture::default(),
                outcome: "unlaunched".into(),
            },
        );
    }
    pub fn dispatch_reader<T, D>(
        &mut self,
        ticket: &DispatchTicket,
        mut transport: T,
        decoder: D,
    ) -> Result<String>
    where
        T: FnMut(&[u8]) -> Result<Vec<u8>>,
        D: FnOnce(&[u8]) -> Result<String>,
    {
        let entered = self.core.enter(ticket, "reader")?;
        let row = self
            .rows
            .get_mut(&entered.id)
            .ok_or_else(|| anyhow::anyhow!("chat_ticket_binding"))?;
        row.boundaries = 1;
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            transport(&entered.body).and_then(|raw| {
                row.response_bytes = Some(raw.len());
                row.capture.deliver(&raw, decoder)
            })
        }));
        let result = match caught {
            Ok(result) => result,
            Err(payload) => {
                let usage = row.capture.usage();
                row.outcome = "cancelled".into();
                self.core.finish_unwind(entered, usage, payload)
            }
        };
        let usage = row.capture.usage();
        row.outcome = if result.is_ok() {
            "succeeded"
        } else {
            "failed"
        }
        .into();
        let report = CallbackReport {
            outcome: if result.is_ok() {
                Outcome::Succeeded
            } else {
                Outcome::Failed
            },
            value: result.as_ref().map(|s| json!(s)).unwrap_or(Value::Null),
            usage,
            usd_micros: None,
            billing: "unknown".into(),
        };
        let finish = self.core.finish(entered, Ok(report));
        match result {
            Ok(answer) => {
                finish?;
                Ok(answer)
            }
            Err(error) => {
                let _ = finish;
                Err(error)
            }
        }
    }
    pub fn dispatch_judge<F>(&mut self, ticket: &DispatchTicket, factory: F) -> Result<Option<bool>>
    where
        F: FnOnce() -> Result<(Transport, GradeDecoder)>,
    {
        let entered = self.core.enter(ticket, "judge")?;
        let row = self
            .rows
            .get_mut(&entered.id)
            .ok_or_else(|| anyhow::anyhow!("chat_ticket_binding"))?;
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            factory().and_then(|(mut transport, mut decode)| {
                row.boundaries = 1;
                let raw = transport(&entered.body)?;
                row.response_bytes = Some(raw.len());
                row.capture.deliver(&raw, |raw| decode(raw))
            })
        }));
        let result = match caught {
            Ok(result) => result,
            Err(payload) => {
                let usage = row.capture.usage();
                row.outcome = "cancelled".into();
                self.core.finish_unwind(entered, usage, payload)
            }
        };
        let usage = row.capture.usage();
        row.outcome = if result.is_ok() {
            "succeeded"
        } else {
            "failed"
        }
        .into();
        let report = CallbackReport {
            outcome: if result.is_ok() {
                Outcome::Succeeded
            } else {
                Outcome::Failed
            },
            value: result.as_ref().map(|v| json!(v)).unwrap_or(Value::Null),
            usage,
            usd_micros: None,
            billing: "unknown".into(),
        };
        let finish = self.core.finish(entered, Ok(report));
        match result {
            Ok(grade) => {
                finish?;
                Ok(grade)
            }
            Err(error) => {
                let _ = finish;
                Err(error)
            }
        }
    }
    pub fn seal_construction(&mut self) -> Result<Construction> {
        self.core.seal_construction()
    }
    pub fn private_judge_journal(&self) -> Result<Vec<u8>> {
        strict::wire(
            &json!({"profile":"longmemeval/injected-chat-judge-journal-v1","requests":self.rows.values().filter(|r|r.phase=="judge").map(|r|json!({"ordinal":r.ordinal,"request":serde_json::from_slice::<Value>(&r.body).ok(),"outcome":r.outcome})).collect::<Vec<_>>()}),
        )
    }
    pub fn snapshot(&self) -> Result<Value> {
        let mut view = self.core.snapshot()?;
        view["injected_transport_boundaries"] =
            json!(self.rows.values().map(|r| r.boundaries).sum::<u64>());
        view["request_byte_scope"] = "canonical_injected_chat_arguments".into();
        view["chat_receipts"]=json!(self.rows.values().map(|r|json!({"phase":r.phase,"ordinal":r.ordinal,"request_bytes":r.body.len(),"response_bytes":r.response_bytes,"injected_transport_boundaries":r.boundaries,"usage_observation":r.capture.snapshot(),"outcome":r.outcome})).collect::<Vec<_>>());
        Ok(view)
    }
    pub fn finalize(&mut self) -> Result<Value> {
        self.core.finalize()?;
        self.snapshot()
    }
}
