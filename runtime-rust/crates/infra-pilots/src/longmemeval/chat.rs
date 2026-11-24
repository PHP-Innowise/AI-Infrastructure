use super::{
    cost::Outcome,
    lifecycle::{CallbackReport, Construction, Controller, DispatchTicket},
    usage::CompletionCapture,
};
use crate::strict;
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};
use uuid::Uuid;

const REPLAY_MEMO_CAP: usize = 2048;

#[derive(Clone)]
struct ReplayMemoEntry {
    answer: Option<String>,
    grade: Option<Option<bool>>,
    response_bytes: usize,
    capture: CompletionCapture,
}

/// Strictly opt-in replay memoization store for `ChatController` dispatches.
/// Disabled (`None`) by default so live benchmark runs never skip or bypass transport/seals.
#[derive(Default)]
pub struct ReplayMemoStore {
    entries: BTreeMap<String, ReplayMemoEntry>,
    order: VecDeque<String>,
    hits: u64,
    misses: u64,
}

impl ReplayMemoStore {
    pub fn new() -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self::default()))
    }

    pub fn stats(&self) -> (usize, u64, u64) {
        (self.entries.len(), self.hits, self.misses)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.hits = 0;
        self.misses = 0;
    }

    fn key(phase: &str, body: &[u8]) -> String {
        format!("{phase}:{}", strict::sha(body))
    }

    fn get(&mut self, phase: &str, body: &[u8]) -> Option<ReplayMemoEntry> {
        let k = Self::key(phase, body);
        match self.entries.get(&k).cloned() {
            Some(entry) => {
                self.hits += 1;
                Some(entry)
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    fn insert(&mut self, phase: &str, body: &[u8], entry: ReplayMemoEntry) {
        let k = Self::key(phase, body);
        if !self.entries.contains_key(&k) {
            if self.entries.len() >= REPLAY_MEMO_CAP
                && let Some(evict) = self.order.pop_front()
            {
                self.entries.remove(&evict);
            }
            self.order.push_back(k.clone());
        }
        self.entries.insert(k, entry);
    }
}
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
    replay_memo: Option<Arc<Mutex<ReplayMemoStore>>>,
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
            replay_memo: None,
        })
    }

    /// Explicitly opts into deterministic replay memoization (`enabled = true`).
    /// Never active by default so live benchmark seals and transport boundaries are preserved.
    pub fn with_replay_memo(mut self, enabled: bool) -> Self {
        self.set_replay_memo(enabled);
        self
    }

    /// Attaches a caller-managed [`ReplayMemoStore`] for explicit replay runs across iterations.
    pub fn with_replay_memo_store(mut self, store: Arc<Mutex<ReplayMemoStore>>) -> Self {
        self.replay_memo = Some(store);
        self
    }

    pub fn set_replay_memo(&mut self, enabled: bool) {
        self.replay_memo = if enabled {
            Some(self.replay_memo.take().unwrap_or_default())
        } else {
            None
        };
    }

    pub fn replay_memo_enabled(&self) -> bool {
        self.replay_memo.is_some()
    }

    pub fn replay_memo_stats(&self) -> (usize, u64, u64) {
        self.replay_memo
            .as_ref()
            .and_then(|s| s.lock().ok().map(|guard| guard.stats()))
            .unwrap_or((0, 0, 0))
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
        if let Some(memo) = self.replay_memo.as_ref()
            && let Some(hit) = memo
                .lock()
                .ok()
                .and_then(|mut guard| guard.get("reader", &entered.body))
            && let Some(answer) = hit.answer
        {
            let usage = {
                let row = self
                    .rows
                    .get_mut(&entered.id)
                    .ok_or_else(|| anyhow::anyhow!("chat_ticket_binding"))?;
                row.boundaries = 0;
                row.response_bytes = Some(hit.response_bytes);
                row.capture = hit.capture;
                row.outcome = "succeeded".into();
                row.capture.usage()
            };
            let report = CallbackReport {
                outcome: Outcome::Succeeded,
                value: json!(answer),
                usage,
                usd_micros: None,
                billing: "unknown".into(),
            };
            self.core.finish(entered, Ok(report))?;
            return Ok(answer);
        }
        let body_key = entered.body.clone();
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
        let memo_snapshot = if self.replay_memo.is_some() && result.is_ok() {
            Some((row.response_bytes.unwrap_or(0), row.capture.clone()))
        } else {
            None
        };
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
                if let (Some(memo), Some((response_bytes, capture))) =
                    (self.replay_memo.as_ref(), memo_snapshot)
                    && let Ok(mut guard) = memo.lock()
                {
                    guard.insert(
                        "reader",
                        &body_key,
                        ReplayMemoEntry {
                            answer: Some(answer.clone()),
                            grade: None,
                            response_bytes,
                            capture,
                        },
                    );
                }
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
        if let Some(memo) = self.replay_memo.as_ref()
            && let Some(hit) = memo
                .lock()
                .ok()
                .and_then(|mut guard| guard.get("judge", &entered.body))
            && let Some(grade) = hit.grade
        {
            let usage = {
                let row = self
                    .rows
                    .get_mut(&entered.id)
                    .ok_or_else(|| anyhow::anyhow!("chat_ticket_binding"))?;
                row.boundaries = 0;
                row.response_bytes = Some(hit.response_bytes);
                row.capture = hit.capture;
                row.outcome = "succeeded".into();
                row.capture.usage()
            };
            let report = CallbackReport {
                outcome: Outcome::Succeeded,
                value: json!(grade),
                usage,
                usd_micros: None,
                billing: "unknown".into(),
            };
            self.core.finish(entered, Ok(report))?;
            return Ok(grade);
        }
        let body_key = entered.body.clone();
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
        let memo_snapshot = if self.replay_memo.is_some() && result.is_ok() {
            Some((row.response_bytes.unwrap_or(0), row.capture.clone()))
        } else {
            None
        };
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
                if let (Some(memo), Some((response_bytes, capture))) =
                    (self.replay_memo.as_ref(), memo_snapshot)
                    && let Ok(mut guard) = memo.lock()
                {
                    guard.insert(
                        "judge",
                        &body_key,
                        ReplayMemoEntry {
                            answer: None,
                            grade: Some(grade),
                            response_bytes,
                            capture,
                        },
                    );
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::longmemeval::{
        lifecycle::Question,
        projection::{NativeRequest, NativeResponse, NativeRuntime},
        registry::{Bridge, Memory, Params},
    };
    #[cfg(unix)]
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[cfg(unix)]
    struct DummyNativeRuntime;
    #[cfg(unix)]
    impl NativeRuntime for DummyNativeRuntime {
        fn refresh_and_render(
            &mut self,
            _repository: &std::path::Path,
            _request: NativeRequest,
        ) -> Result<NativeResponse> {
            Ok(NativeResponse {
                text: "evidence-text".into(),
                has_evidence: true,
                warnings: false,
                sanitized_empty: false,
            })
        }
    }

    #[cfg(unix)]
    fn make_bridge(dir: &std::path::Path, horizon: usize) -> Box<dyn Memory> {
        std::fs::create_dir_all(dir.join("project-brain/config")).unwrap();
        std::fs::create_dir_all(dir.join("project-brain/schemas")).unwrap();
        std::fs::create_dir_all(dir.join("specs")).unwrap();
        std::fs::write(dir.join("project-brain/config/runtime.json"), "{}").unwrap();
        let bridge = Bridge::new(
            dir.to_path_buf(),
            Box::new(DummyNativeRuntime),
            Params {
                horizon,
                local_token_profile: "unknown".into(),
            },
        )
        .unwrap();
        Box::new(bridge)
    }

    #[cfg(unix)]
    fn make_questions(count: usize) -> Vec<Question> {
        (0..count)
            .map(|_| Question {
                question: "Where is auth configured?".into(),
                query_image: None,
            })
            .collect()
    }

    #[test]
    fn replay_memo_store_tracks_hits_misses_and_bounds_capacity() {
        let store = ReplayMemoStore::new();
        let mut guard = store.lock().unwrap();
        assert_eq!(guard.stats(), (0, 0, 0));
        assert!(guard.get("reader", b"req-0").is_none());
        assert_eq!(guard.stats(), (0, 0, 1));

        for i in 0..=REPLAY_MEMO_CAP {
            guard.insert(
                "reader",
                format!("req-{i}").as_bytes(),
                ReplayMemoEntry {
                    answer: Some(format!("ans-{i}")),
                    grade: None,
                    response_bytes: 12,
                    capture: CompletionCapture::default(),
                },
            );
        }
        assert_eq!(guard.stats().0, REPLAY_MEMO_CAP);
        assert!(guard.get("reader", b"req-0").is_none());
        let hit = guard.get("reader", format!("req-{REPLAY_MEMO_CAP}").as_bytes());
        assert_eq!(
            hit.and_then(|e| e.answer).as_deref(),
            Some(format!("ans-{REPLAY_MEMO_CAP}").as_str())
        );
        guard.clear();
        assert_eq!(guard.stats(), (0, 0, 0));
    }

    #[cfg(unix)]
    #[test]
    fn replay_memo_is_disabled_by_default_and_opts_in_preserving_seals() {
        let base =
            std::env::temp_dir().join(format!("infra-pilots-chat-replay-{}", Uuid::new_v4()));
        let d1 = base.join("live");
        let d2 = base.join("optin");
        let d3 = base.join("other_seal");
        std::fs::create_dir_all(&d1).unwrap();
        std::fs::create_dir_all(&d2).unwrap();
        std::fs::create_dir_all(&d3).unwrap();

        // 1. Default ChatController NEVER memoizes live reader or judge runs.
        let core_live =
            Controller::new(make_bridge(&d1, 2), make_questions(2), 4096, None).unwrap();
        let mut live =
            ChatController::new(core_live, "gpt-test".into(), "gpt-judge".into(), 128, 128)
                .unwrap();
        assert!(!live.replay_memo_enabled());

        let reader_calls = AtomicUsize::new(0);
        let judge_calls = AtomicUsize::new(0);
        for i in 0..2 {
            live.query(i).unwrap();
            let t = live.prepare_reader(i).unwrap();
            let ans = live
                .dispatch_reader(
                    &t,
                    |_| {
                        reader_calls.fetch_add(1, Ordering::SeqCst);
                        Ok(
                            br#"{"usage":{"input_tokens":5,"output_tokens":3,"total_tokens":8}}"#
                                .to_vec(),
                        )
                    },
                    |_| Ok("answer-a".into()),
                )
                .unwrap();
            assert_eq!(ans, "answer-a");
        }
        let seal_live = live.seal_construction().unwrap();
        for i in 0..2 {
            let t = live.prepare_judge(i, &seal_live).unwrap();
            let grade = live
                .dispatch_judge(&t, || {
                    judge_calls.fetch_add(1, Ordering::SeqCst);
                    Ok((
                        Box::new(|_| Ok(b"{}".to_vec())),
                        Box::new(|_| Ok(Some(true))),
                    ))
                })
                .unwrap();
            assert_eq!(grade, Some(true));
        }
        assert_eq!(reader_calls.load(Ordering::SeqCst), 2);
        assert_eq!(judge_calls.load(Ordering::SeqCst), 2);
        assert_eq!(live.replay_memo_stats(), (0, 0, 0));
        live.finalize().unwrap();

        // 2. Explicit opt-in ReplayMemoStore caches identical reader/judge requests while still
        // enforcing ticket & construction seals.
        let shared_store = ReplayMemoStore::new();
        let core_opt = Controller::new(make_bridge(&d2, 2), make_questions(2), 4096, None).unwrap();
        let mut opt =
            ChatController::new(core_opt, "gpt-test".into(), "gpt-judge".into(), 128, 128)
                .unwrap()
                .with_replay_memo_store(shared_store.clone());
        assert!(opt.replay_memo_enabled());

        let opt_reader_calls = AtomicUsize::new(0);
        let opt_judge_calls = AtomicUsize::new(0);
        let mut reader_tickets = Vec::new();
        for i in 0..2 {
            opt.query(i).unwrap();
            let t = opt.prepare_reader(i).unwrap();
            let ans = opt
                .dispatch_reader(
                    &t,
                    |_| {
                        opt_reader_calls.fetch_add(1, Ordering::SeqCst);
                        Ok(
                            br#"{"usage":{"input_tokens":5,"output_tokens":3,"total_tokens":8}}"#
                                .to_vec(),
                        )
                    },
                    |_| Ok("answer-a".into()),
                )
                .unwrap();
            assert_eq!(ans, "answer-a");
            reader_tickets.push(t);
        }
        // Reusing a consumed ticket is still rejected even when replay memo is active.
        assert!(
            opt.dispatch_reader(&reader_tickets[0], |_| unreachable!(), |_| unreachable!())
                .is_err()
        );

        let seal_opt = opt.seal_construction().unwrap();
        for i in 0..2 {
            let t = opt.prepare_judge(i, &seal_opt).unwrap();
            let grade = opt
                .dispatch_judge(&t, || {
                    opt_judge_calls.fetch_add(1, Ordering::SeqCst);
                    Ok((
                        Box::new(|_| Ok(b"{}".to_vec())),
                        Box::new(|_| Ok(Some(true))),
                    ))
                })
                .unwrap();
            assert_eq!(grade, Some(true));
        }
        assert_eq!(opt_reader_calls.load(Ordering::SeqCst), 1);
        assert_eq!(opt_judge_calls.load(Ordering::SeqCst), 1);
        assert_eq!(opt.replay_memo_stats(), (2, 2, 2));
        // On a replay hit, boundaries == 0 while the first invocation recorded boundaries == 1.
        let snap = opt.finalize().unwrap();
        assert_eq!(snap["injected_transport_boundaries"], json!(2));

        // 3. A distinct controller construction seal cannot reuse judge cache entries from seal_opt!
        let mut qs3 = make_questions(1);
        let _ = &mut qs3;
        let core_other =
            Controller::new(make_bridge(&d3, 1), make_questions(1), 4096, None).unwrap();
        let mut other =
            ChatController::new(core_other, "gpt-test".into(), "gpt-judge".into(), 128, 128)
                .unwrap()
                .with_replay_memo_store(shared_store);
        other.query(0).unwrap();
        let tr = other.prepare_reader(0).unwrap();
        // Reader hits existing entry because question/model/body matched.
        other
            .dispatch_reader(&tr, |_| unreachable!("reader hit"), |_| unreachable!())
            .unwrap();
        let seal_other = other.seal_construction().unwrap();
        assert_ne!(seal_opt.sha256(), seal_other.sha256());
        let tj = other.prepare_judge(0, &seal_other).unwrap();
        let other_judge_calls = AtomicUsize::new(0);
        let g = other
            .dispatch_judge(&tj, || {
                other_judge_calls.fetch_add(1, Ordering::SeqCst);
                Ok((
                    Box::new(|_| Ok(b"{}".to_vec())),
                    Box::new(|_| Ok(Some(false))),
                ))
            })
            .unwrap();
        assert_eq!(g, Some(false));
        assert_eq!(other_judge_calls.load(Ordering::SeqCst), 1);

        let _ = std::fs::remove_dir_all(&base);
    }
}
