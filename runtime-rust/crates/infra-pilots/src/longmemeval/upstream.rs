use super::{
    chat::{ChatController, ChatRow, Style},
    cost::Outcome,
    lifecycle::{CallbackReport, Construction, DispatchTicket},
};
use crate::strict;
use anyhow::{Result, bail};
use futures_util::FutureExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{future::Future, pin::Pin};
pub trait AsyncCreate: Send {
    fn create<'a>(
        &'a mut self,
        arguments: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>>;
}
pub trait SyncCreate: Send {
    fn create(&mut self, arguments: Value) -> Result<Value>;
}
pub trait Serializer: Send {
    fn serialize(&mut self, response: &Value) -> Result<Vec<u8>>;
}
impl<F: FnMut(&Value) -> Result<Vec<u8>> + Send> Serializer for F {
    fn serialize(&mut self, v: &Value) -> Result<Vec<u8>> {
        self(v)
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ReaderArgs {
    pub model: String,
    pub max_completion_tokens: u64,
    pub timeout_seconds: f64,
    pub base_url: Option<String>,
    pub reasoning_effort: Option<String>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub presence_penalty: Option<f64>,
    pub top_k: Option<u64>,
    pub repetition_penalty: Option<f64>,
    pub reader_enable_thinking: bool,
}
pub trait ReaderBuilder: Send {
    fn build(&mut self, args: &ReaderArgs, messages: &Value) -> Result<Value>;
}
pub type ReaderFuture<'a> = Pin<Box<dyn Future<Output = Result<(String, Value)>> + Send + 'a>>;
pub trait ReaderHelper: Send {
    fn run<'a>(
        &'a mut self,
        args: &'a ReaderArgs,
        messages: Value,
        create: &'a mut dyn AsyncCreate,
    ) -> ReaderFuture<'a>;
}
pub trait JudgeHelper: Send {
    fn run(&mut self, arguments: &Value, create: &mut dyn SyncCreate) -> Result<String>;
}
pub type JudgeFactory = Box<
    dyn FnOnce() -> Result<(
            Box<dyn SyncCreate>,
            Box<dyn Serializer>,
            Box<dyn FnMut(&str) -> Result<Option<bool>> + Send>,
        )> + Send,
>;
struct AsyncBoundary<'a> {
    expected: Value,
    transport: &'a mut dyn AsyncCreate,
    serializer: &'a mut dyn Serializer,
    row: &'a mut ChatRow,
    faults: &'a mut usize,
}
impl AsyncCreate for AsyncBoundary<'_> {
    fn create<'a>(
        &'a mut self,
        arguments: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
        Box::pin(async move {
            if self.row.boundaries != 0
                || strict::wire(&arguments)? != strict::wire(&self.expected)?
            {
                bail!("upstream_create_binding");
            }
            self.row.boundaries = 1;
            let response = self.transport.create(self.expected.clone()).await?;
            match self.serializer.serialize(&response) {
                Ok(raw) if raw.len() <= 524288 => {
                    self.row.response_bytes = Some(raw.len());
                    if self.row.capture.deliver(&raw, |_| Ok(())).is_err() {
                        *self.faults += 1;
                    }
                }
                _ => *self.faults += 1,
            }
            Ok(response)
        })
    }
}
struct SyncBoundary<'a> {
    expected: Value,
    transport: &'a mut dyn SyncCreate,
    serializer: &'a mut dyn Serializer,
    row: &'a mut ChatRow,
    faults: &'a mut usize,
}
impl SyncCreate for SyncBoundary<'_> {
    fn create(&mut self, arguments: Value) -> Result<Value> {
        if self.row.boundaries != 0 || strict::wire(&arguments)? != strict::wire(&self.expected)? {
            bail!("upstream_create_binding");
        }
        self.row.boundaries = 1;
        let response = self.transport.create(self.expected.clone())?;
        match self.serializer.serialize(&response) {
            Ok(raw) if raw.len() <= 524288 => {
                self.row.response_bytes = Some(raw.len());
                if self.row.capture.deliver(&raw, |_| Ok(())).is_err() {
                    *self.faults += 1;
                }
            }
            _ => *self.faults += 1,
        }
        Ok(response)
    }
}
pub struct UpstreamController {
    chat: ChatController,
    builder: Box<dyn ReaderBuilder>,
    reader: Box<dyn ReaderHelper>,
    judge: Box<dyn JudgeHelper>,
    reader_timeout: f64,
    judge_timeout: f64,
    faults: usize,
}
impl UpstreamController {
    pub fn new(
        mut chat: ChatController,
        builder: Box<dyn ReaderBuilder>,
        reader: Box<dyn ReaderHelper>,
        judge: Box<dyn JudgeHelper>,
        reader_timeout: f64,
        judge_timeout: f64,
    ) -> Result<Self> {
        if [reader_timeout, judge_timeout]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0 || *v > 3600.0)
        {
            bail!("upstream_timeout");
        }
        chat.style = Style::Upstream {
            reader_timeout,
            judge_timeout,
        };
        chat.core.set_profiles(
            "longmemeval/selected-upstream-mixed-controller-v1",
            "longmemeval/upstream-sdk-argument-construction-v1",
        );
        Ok(Self {
            chat,
            builder,
            reader,
            judge,
            reader_timeout,
            judge_timeout,
            faults: 0,
        })
    }
    fn reader_args(&self) -> ReaderArgs {
        ReaderArgs {
            model: self.chat.reader_model.clone(),
            max_completion_tokens: self.chat.reader_max,
            timeout_seconds: self.reader_timeout,
            base_url: None,
            reasoning_effort: None,
            temperature: None,
            top_p: None,
            presence_penalty: None,
            top_k: None,
            repetition_penalty: None,
            reader_enable_thinking: true,
        }
    }
    pub fn query(&mut self, i: usize) -> Result<Vec<Value>> {
        self.chat.query(i)
    }
    pub fn prepare_reader(&mut self, i: usize) -> Result<DispatchTicket> {
        let args = self.reader_args();
        let model = self.chat.reader_model.clone();
        let maximum = self.chat.reader_max;
        let style = self.chat.style.clone();
        let builder = &mut self.builder;
        let ticket = self.chat.core.prepare_reader_with(i, |local| {
            let body = ChatController::arguments(local, &model, maximum, &style, "reader")?;
            let expected: Value = serde_json::from_slice(&body)?;
            let compiled = builder.build(&args, &expected["messages"])?;
            if strict::wire(&compiled)? != body {
                bail!("upstream_request_profile");
            }
            Ok(body)
        })?;
        self.chat.record(&ticket);
        Ok(ticket)
    }
    pub fn prepare_judge(&mut self, i: usize, seal: &Construction) -> Result<DispatchTicket> {
        self.chat.prepare_judge(i, seal)
    }
    pub async fn dispatch_reader(
        &mut self,
        ticket: &DispatchTicket,
        transport: &mut dyn AsyncCreate,
        serializer: &mut dyn Serializer,
    ) -> Result<(String, Value)> {
        let entered = self.chat.core.enter(ticket, "reader")?;
        let expected: Value = serde_json::from_slice(&entered.body)?;
        let args = self.reader_args();
        let row = self
            .chat
            .rows
            .get_mut(&entered.id)
            .ok_or_else(|| anyhow::anyhow!("upstream_ticket"))?;
        let caught = {
            let mut boundary = AsyncBoundary {
                expected: expected.clone(),
                transport,
                serializer,
                row,
                faults: &mut self.faults,
            };
            std::panic::AssertUnwindSafe(self.reader.run(
                &args,
                expected["messages"].clone(),
                &mut boundary,
            ))
            .catch_unwind()
            .await
        };
        let result = match caught {
            Ok(result) => result,
            Err(payload) => {
                let row = self.chat.rows.get_mut(&entered.id).unwrap();
                row.outcome = "cancelled".into();
                let usage = row.capture.usage();
                self.chat.core.finish_unwind(entered, usage, payload)
            }
        };
        let row = self.chat.rows.get_mut(&entered.id).unwrap();
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
            value: result
                .as_ref()
                .map(|(answer, _)| json!(answer))
                .unwrap_or(Value::Null),
            usage: row.capture.usage(),
            usd_micros: None,
            billing: "unknown".into(),
        };
        let finish = self.chat.core.finish(entered, Ok(report));
        match result {
            Ok(v) => {
                finish?;
                Ok(v)
            }
            Err(e) => {
                let _ = finish;
                Err(e)
            }
        }
    }
    pub fn dispatch_judge(
        &mut self,
        ticket: &DispatchTicket,
        factory: JudgeFactory,
    ) -> Result<Option<bool>> {
        let entered = self.chat.core.enter(ticket, "judge")?;
        let expected: Value = serde_json::from_slice(&entered.body)?;
        let row = self
            .chat
            .rows
            .get_mut(&entered.id)
            .ok_or_else(|| anyhow::anyhow!("upstream_ticket"))?;
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            factory().and_then(|(mut transport, mut serializer, mut decode)| {
                let mut boundary = SyncBoundary {
                    expected: expected.clone(),
                    transport: transport.as_mut(),
                    serializer: serializer.as_mut(),
                    row,
                    faults: &mut self.faults,
                };
                let text = self.judge.run(&expected, &mut boundary)?;
                decode(&text)
            })
        }));
        let result = match caught {
            Ok(result) => result,
            Err(payload) => {
                let row = self.chat.rows.get_mut(&entered.id).unwrap();
                row.outcome = "cancelled".into();
                let usage = row.capture.usage();
                self.chat.core.finish_unwind(entered, usage, payload)
            }
        };
        let row = self.chat.rows.get_mut(&entered.id).unwrap();
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
            usage: row.capture.usage(),
            usd_micros: None,
            billing: "unknown".into(),
        };
        let finish = self.chat.core.finish(entered, Ok(report));
        match result {
            Ok(v) => {
                finish?;
                Ok(v)
            }
            Err(e) => {
                let _ = finish;
                Err(e)
            }
        }
    }
    pub fn seal_construction(&mut self) -> Result<Construction> {
        self.chat.seal_construction()
    }
    pub fn private_judge_journal(&self) -> Result<Vec<u8>> {
        let mut v: Value = serde_json::from_slice(&self.chat.private_judge_journal()?)?;
        v["profile"] = "longmemeval/upstream-sdk-argument-judge-journal-v1".into();
        strict::wire(&v)
    }
    pub fn snapshot(&self) -> Result<Value> {
        let mut v = self.chat.snapshot()?;
        let boundaries = v
            .as_object_mut()
            .unwrap()
            .remove("injected_transport_boundaries");
        v["sdk_create_boundaries"] = boundaries.unwrap_or(Value::Null);
        v["request_byte_scope"] = "canonical_sdk_argument_proxy".into();
        v["supplied_usage_observation_faults"] = json!(self.faults);
        v["reader_timeout"] = json!(self.reader_timeout);
        v["judge_timeout"] = json!(self.judge_timeout);
        Ok(v)
    }
    pub fn finalize(&mut self) -> Result<Value> {
        self.chat.finalize()?;
        self.snapshot()
    }
}
