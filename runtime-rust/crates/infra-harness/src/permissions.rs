//! Permission requests a running session waits on, and the person's answers.
//!
//! A session's Permissions setting says what an editing launch may do without
//! asking (`infra_host::permission`). With Manual, each request the host asks
//! becomes a `permission` event the page shows with its answers, and the
//! launch waits here until the person answers, the host withdraws the request
//! or the run ends.
use crate::{HarnessError, Result, sessions::Event};
use infra_host::permission::{Answer, Approvals, Decision, PermissionAsk, PermissionMode};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

const PERMISSION_TIMEOUT_ENV: &str = "HARNESS_PERMISSION_TIMEOUT_SECONDS";
const MAX_TIMEOUT_SECONDS: u64 = 365 * 86_400;
const MAX_SESSION_RULES_PER_SESSION: usize = 256;
const MAX_RULE_SESSIONS: usize = 1024;
const MAX_SESSION_RULE_KEY_BYTES: usize = 4096;

fn parse_timeout_seconds(raw: Option<&str>) -> Option<Duration> {
    let secs = raw?.trim().parse::<u64>().ok()?;
    (secs > 0).then(|| Duration::from_secs(secs.min(MAX_TIMEOUT_SECONDS)))
}

fn timeout_from_env() -> Option<Duration> {
    let raw = std::env::var(PERMISSION_TIMEOUT_ENV).ok();
    parse_timeout_seconds(raw.as_deref())
}

/// A session's Permissions value: a mode's name, or null for a session that
/// names none and keeps the behaviour from before the setting existed
/// (workspace edits without a person, everything else declined).
pub fn validate(value: &Value) -> Result<Value> {
    match value {
        Value::Null => Ok(Value::Null),
        Value::String(name) => PermissionMode::parse(name)
            .map(|mode| json!(mode.as_str()))
            .ok_or_else(|| HarnessError::invalid("Choose Auto, Manual or Bypass permissions.")),
        _ => Err(HarnessError::invalid(
            "Choose Auto, Manual or Bypass permissions.",
        )),
    }
}
/// The mode a session's editing launch runs with, if it names one.
pub fn mode(session: &Value) -> Option<PermissionMode> {
    session["permissions"]
        .as_str()
        .and_then(PermissionMode::parse)
}

struct Pending {
    nonce: u64,
    sender: tokio::sync::oneshot::Sender<Answer>,
    emit: Event,
    ask: PermissionAsk,
    timeout_task: Option<tokio::task::AbortHandle>,
}
impl Pending {
    fn cancel_timeout(&mut self) {
        if let Some(handle) = self.timeout_task.take() {
            handle.abort();
        }
    }
}

/// Every request waiting for a person, by session and request.
pub struct PermissionDesk {
    pending: Mutex<BTreeMap<(String, String), Pending>>,
    session_rules: Mutex<BTreeMap<String, BTreeSet<String>>>,
    timeout: Option<Duration>,
    next_nonce: AtomicU64,
}
impl Default for PermissionDesk {
    fn default() -> Self {
        Self::new_with_timeout(timeout_from_env())
    }
}
fn lock<T>(value: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
    value
        .lock()
        .map_err(|_| HarnessError::unavailable("Permission requests lock failed."))
}
impl PermissionDesk {
    /// Creates a permission desk with an optional per-ask expiration timeout.
    pub fn new_with_timeout(timeout: Option<Duration>) -> Self {
        Self {
            pending: Mutex::new(BTreeMap::new()),
            session_rules: Mutex::new(BTreeMap::new()),
            timeout,
            next_nonce: AtomicU64::new(1),
        }
    }
    /// Where one run of a session puts its requests; its events go to `emit`.
    pub fn approvals(self: &Arc<Self>, session: &str, emit: Event) -> Arc<dyn Approvals> {
        Arc::new(SessionApprovals {
            desk: self.clone(),
            session: session.into(),
            run: crate::store::hexid()[..8].to_owned(),
            emit,
        })
    }
    /// The person's decision for one request. The answer is recorded before the
    /// host hears it.
    pub fn decide(&self, session: &str, id: &str, decision: Decision) -> Result<Value> {
        self.decide_answer(session, id, Answer::from_decision(decision))
    }
    /// The person's decision and optional structured question answers for one
    /// request. Recorded before the host hears it.
    pub fn decide_answer(&self, session: &str, id: &str, answer: Answer) -> Result<Value> {
        let key = (session.to_owned(), id.to_owned());
        let mut pending = lock(&self.pending)?;
        let session_option = pending
            .get(&key)
            .ok_or_else(|| {
                HarnessError::conflict("This request is no longer waiting for an answer.")
            })?
            .ask
            .session_option;
        if answer.decision == Decision::AllowSession && !session_option {
            return Err(HarnessError::invalid(
                "This host cannot remember an approval for the session; allow it once.",
            ));
        }
        let Some(mut request) = pending.remove(&key) else {
            return Err(HarnessError::conflict(
                "This request is no longer waiting for an answer.",
            ));
        };
        drop(pending);
        request.cancel_timeout();
        if answer.decision == Decision::AllowSession
            && let Some(rule_key) = request.ask.session_rule_key.as_deref()
        {
            self.remember_session_rule(session, rule_key)?;
        }
        let state = answer.decision.state();
        let mut event = request.ask.resolved_event(state, "person");
        if answer.decision != Decision::Deny
            && let Some(ref answers) = answer.answers
        {
            event["answers"] = answers.clone();
        }
        (request.emit)(event)?;
        // The host may have stopped while the person decided.
        let delivered = request.sender.send(answer).is_ok();
        Ok(json!({"id": id, "state": state, "delivered": delivered}))
    }
    fn has_session_rule(&self, session: &str, rule_key: &str) -> Result<bool> {
        let rules = lock(&self.session_rules)?;
        Ok(rules
            .get(session)
            .is_some_and(|session_set| session_set.contains(rule_key)))
    }
    fn remember_session_rule(&self, session: &str, rule_key: &str) -> Result<()> {
        if rule_key.is_empty() || rule_key.len() > MAX_SESSION_RULE_KEY_BYTES {
            return Ok(());
        }
        let mut all_rules = lock(&self.session_rules)?;
        if !all_rules.contains_key(session) && all_rules.len() >= MAX_RULE_SESSIONS {
            all_rules.pop_first();
        }
        let rules = all_rules.entry(session.to_owned()).or_default();
        if !rules.contains(rule_key) && rules.len() >= MAX_SESSION_RULES_PER_SESSION {
            rules.pop_first();
        }
        rules.insert(rule_key.to_owned());
        Ok(())
    }
    fn spawn_timeout(
        self: &Arc<Self>,
        session: &str,
        public_id: &str,
        nonce: u64,
    ) -> Option<tokio::task::AbortHandle> {
        let ttl = self.timeout?;
        let handle = tokio::runtime::Handle::try_current().ok()?;
        let desk = self.clone();
        let session = session.to_owned();
        let public_id = public_id.to_owned();
        let join = handle.spawn(async move {
            tokio::time::sleep(ttl).await;
            desk.expire_request(&session, &public_id, nonce);
        });
        Some(join.abort_handle())
    }
    fn expire_request(&self, session: &str, public_id: &str, nonce: u64) {
        let removed = match lock(&self.pending) {
            Ok(mut pending) => {
                let key = (session.to_owned(), public_id.to_owned());
                if pending.get(&key).is_some_and(|entry| entry.nonce == nonce) {
                    pending.remove(&key)
                } else {
                    None
                }
            }
            Err(_) => None,
        };
        let Some(request) = removed else {
            return;
        };
        let _ = (request.emit)(request.ask.resolved_event("expired", "timeout"));
        let _ = request.sender.send(Answer::from_decision(Decision::Deny));
    }
    /// Answers no to whatever a session's finished or stopped run left
    /// waiting, so its page stops offering answers.
    pub fn expire(&self, session: &str) {
        let left: Vec<Pending> = match lock(&self.pending) {
            Ok(mut pending) => {
                let keys: Vec<_> = pending
                    .keys()
                    .filter(|(owner, _)| owner == session)
                    .cloned()
                    .collect();
                keys.into_iter()
                    .filter_map(|key| pending.remove(&key))
                    .collect()
            }
            Err(_) => vec![],
        };
        for mut request in left {
            request.cancel_timeout();
            let _ = (request.emit)(request.ask.resolved_event("expired", "harness"));
        }
    }
    /// The requests a session's run waits on now.
    pub fn waiting(&self, session: &str) -> Vec<Value> {
        lock(&self.pending)
            .map(|pending| {
                pending
                    .iter()
                    .filter(|((owner, _), _)| owner == session)
                    .map(|(_, request)| request.ask.pending_event())
                    .collect()
            })
            .unwrap_or_default()
    }
    /// Number of waiting requests per session.
    pub fn waiting_counts(&self) -> BTreeMap<String, usize> {
        lock(&self.pending)
            .map(|pending| {
                let mut counts = BTreeMap::new();
                for (owner, _) in pending.keys() {
                    *counts.entry(owner.clone()).or_insert(0) += 1;
                }
                counts
            })
            .unwrap_or_default()
    }
}

struct SessionApprovals {
    desk: Arc<PermissionDesk>,
    session: String,
    /// Codex and Cursor number their requests afresh in every process, so a
    /// session's next run would reuse the ids of the last; the page keeps one
    /// block per id. Each run tags its requests.
    run: String,
    emit: Event,
}
impl SessionApprovals {
    fn public(&self, id: &str) -> String {
        format!("{id}-{}", self.run)
    }
}
impl Approvals for SessionApprovals {
    fn ask(&self, ask: &PermissionAsk) -> Result<tokio::sync::oneshot::Receiver<Decision>> {
        let mut rx = self.ask_answer(ask)?;
        let (mut tx, decision_rx) = tokio::sync::oneshot::channel();
        match rx.try_recv() {
            Ok(answer) => {
                let _ = tx.send(answer.decision);
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {}
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    handle.spawn(async move {
                        tokio::select! {
                            res = &mut rx => {
                                if let Ok(answer) = res {
                                    let _ = tx.send(answer.decision);
                                }
                            }
                            _ = tx.closed() => {}
                        }
                    });
                } else {
                    std::thread::spawn(move || {
                        if let Ok(answer) = rx.blocking_recv() {
                            let _ = tx.send(answer.decision);
                        }
                    });
                }
            }
        }
        Ok(decision_rx)
    }
    fn ask_answer(&self, ask: &PermissionAsk) -> Result<tokio::sync::oneshot::Receiver<Answer>> {
        let ask = PermissionAsk {
            id: self.public(&ask.id),
            ..ask.clone()
        };
        if let Some(rule_key) = ask.session_rule_key.as_deref()
            && self.desk.has_session_rule(&self.session, rule_key)?
        {
            (self.emit)(ask.resolved_event("allowed_session", "session_rule"))?;
            let (tx, rx) = tokio::sync::oneshot::channel();
            let _ = tx.send(Answer::from_decision(Decision::Allow));
            return Ok(rx);
        }
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let mut pending = lock(&self.desk.pending)?;
        let key = (self.session.clone(), ask.id.clone());
        if pending.contains_key(&key) {
            return Err(HarnessError::conflict(
                "The host asked the same request twice.",
            ));
        }
        // The page sees the request before anyone can answer it.
        (self.emit)(ask.pending_event())?;
        let nonce = self.desk.next_nonce.fetch_add(1, Ordering::Relaxed);
        let timeout_task = self.desk.spawn_timeout(&self.session, &ask.id, nonce);
        pending.insert(
            key,
            Pending {
                nonce,
                sender,
                emit: self.emit.clone(),
                ask,
                timeout_task,
            },
        );
        Ok(receiver)
    }
    fn withdraw(&self, id: &str) -> Result<()> {
        let request = lock(&self.desk.pending)?.remove(&(self.session.clone(), self.public(id)));
        if let Some(mut request) = request {
            request.cancel_timeout();
            (request.emit)(request.ask.resolved_event("withdrawn", "host"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recorder() -> (Event, Arc<Mutex<Vec<Value>>>) {
        let events = Arc::new(Mutex::new(vec![]));
        let sink = events.clone();
        (
            Arc::new(move |event| {
                sink.lock().unwrap().push(event);
                Ok(())
            }),
            events,
        )
    }
    /// The id the page saw for the `at`-th recorded event's request.
    fn seen(events: &Arc<Mutex<Vec<Value>>>, at: usize) -> String {
        events.lock().unwrap()[at]["request_id"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    fn ask(id: &str, session_option: bool) -> PermissionAsk {
        PermissionAsk {
            id: id.into(),
            provider: "codex".into(),
            tool: "Shell".into(),
            summary: "cargo test".into(),
            detail: None,
            session_option,
            ..Default::default()
        }
    }

    #[test]
    fn values_name_a_mode_or_nothing_and_refuse_anything_else() {
        assert_eq!(validate(&Value::Null).unwrap(), Value::Null);
        for name in ["auto", "manual", "bypass"] {
            assert_eq!(validate(&json!(name)).unwrap(), name);
        }
        for bad in [
            json!("Manual"),
            json!("bypassPermissions"),
            json!(true),
            json!(1),
        ] {
            assert!(validate(&bad).is_err(), "{bad}");
        }
        assert_eq!(mode(&json!({"permissions": null})), None);
        assert_eq!(
            mode(&json!({"permissions": "manual"})),
            Some(PermissionMode::Manual)
        );
    }

    #[tokio::test]
    async fn an_answer_reaches_the_waiting_launch_once_and_is_recorded_first() {
        let desk = Arc::new(PermissionDesk::default());
        let (emit, events) = recorder();
        let approvals = desk.approvals("session-1", emit);
        let receiver = approvals.ask(&ask("codex-7", true)).unwrap();
        assert_eq!(events.lock().unwrap()[0]["state"], "pending");
        let id = seen(&events, 0);
        assert!(id.starts_with("codex-7-"), "{id}");
        assert_eq!(desk.waiting("session-1")[0]["request_id"], id.as_str());
        assert!(desk.waiting("session-2").is_empty());
        // Another session cannot answer it.
        assert_eq!(
            desk.decide("session-2", &id, Decision::Allow)
                .unwrap_err()
                .status,
            409
        );
        let answer = desk
            .decide("session-1", &id, Decision::AllowSession)
            .unwrap();
        assert_eq!(answer["state"], "allowed_session");
        assert_eq!(answer["delivered"], true);
        assert_eq!(receiver.await.unwrap(), Decision::AllowSession);
        let recorded = events.lock().unwrap().clone();
        assert_eq!(recorded[1]["state"], "allowed_session");
        assert_eq!(recorded[1]["by"], "person");
        // A second answer finds nothing waiting.
        assert_eq!(
            desk.decide("session-1", &id, Decision::Deny)
                .unwrap_err()
                .status,
            409
        );
    }

    #[tokio::test]
    async fn a_session_approval_needs_a_host_that_remembers_it() {
        let desk = Arc::new(PermissionDesk::default());
        let (emit, events) = recorder();
        let approvals = desk.approvals("s", emit);
        let receiver = approvals.ask(&ask("cursor-1", false)).unwrap();
        let id = seen(&events, 0);
        assert_eq!(
            desk.decide("s", &id, Decision::AllowSession)
                .unwrap_err()
                .status,
            400
        );
        // The refused answer left the request waiting.
        desk.decide("s", &id, Decision::Deny).unwrap();
        assert_eq!(receiver.await.unwrap(), Decision::Deny);
    }

    #[tokio::test]
    async fn withdrawn_and_expired_requests_stop_waiting() {
        let desk = Arc::new(PermissionDesk::default());
        let (emit, events) = recorder();
        let approvals = desk.approvals("s", emit);
        let withdrawn = approvals.ask(&ask("claude-1", true)).unwrap();
        let expired = approvals.ask(&ask("claude-2", true)).unwrap();
        assert!(approvals.ask(&ask("claude-2", true)).is_err());
        approvals.withdraw("claude-1").unwrap();
        assert!(withdrawn.await.is_err());
        desk.expire("s");
        assert!(expired.await.is_err());
        assert!(desk.waiting("s").is_empty());
        let states: Vec<_> = events
            .lock()
            .unwrap()
            .iter()
            .map(|event| event["state"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(states, ["pending", "pending", "withdrawn", "expired"]);
    }

    #[tokio::test]
    async fn each_run_tags_its_requests_so_reused_host_ids_stay_apart() {
        let desk = Arc::new(PermissionDesk::default());
        let (emit, events) = recorder();
        // Codex and Cursor number their requests from 0 in every process.
        let first = desk
            .approvals("s", emit.clone())
            .ask(&ask("cursor-0", false))
            .unwrap();
        let earlier = seen(&events, 0);
        desk.decide("s", &earlier, Decision::Allow).unwrap();
        assert_eq!(first.await.unwrap(), Decision::Allow);
        let second = desk
            .approvals("s", emit)
            .ask(&ask("cursor-0", false))
            .unwrap();
        let later = seen(&events, 2);
        assert!(earlier.starts_with("cursor-0-") && later.starts_with("cursor-0-"));
        assert_ne!(earlier, later);
        assert_eq!(
            desk.decide("s", &earlier, Decision::Deny)
                .unwrap_err()
                .status,
            409
        );
        desk.decide("s", &later, Decision::Deny).unwrap();
        assert_eq!(second.await.unwrap(), Decision::Deny);
    }

    #[tokio::test]
    async fn waiting_counts_tracks_pending_requests_per_session() {
        let desk = Arc::new(PermissionDesk::default());
        assert!(desk.waiting_counts().is_empty());
        let (emit_a, events_a) = recorder();
        let (emit_b, _) = recorder();
        let a = desk.approvals("session-a", emit_a);
        let b = desk.approvals("session-b", emit_b);
        let _r1 = a.ask(&ask("claude-1", true)).unwrap();
        let _r2 = a.ask(&ask("claude-2", true)).unwrap();
        let _r3 = b.ask(&ask("codex-1", true)).unwrap();
        assert_eq!(
            desk.waiting_counts(),
            BTreeMap::from([("session-a".into(), 2), ("session-b".into(), 1)])
        );
        let id1 = seen(&events_a, 0);
        desk.decide("session-a", &id1, Decision::Allow).unwrap();
        assert_eq!(
            desk.waiting_counts(),
            BTreeMap::from([("session-a".into(), 1), ("session-b".into(), 1)])
        );
        a.withdraw("claude-2").unwrap();
        assert_eq!(
            desk.waiting_counts(),
            BTreeMap::from([("session-b".into(), 1)])
        );
        desk.expire("session-b");
        assert!(desk.waiting_counts().is_empty());
    }

    #[tokio::test]
    async fn session_rules_cache_allow_session_per_session_and_bound_entries() {
        let desk = Arc::new(PermissionDesk::new_with_timeout(None));
        let (emit_a, events_a) = recorder();
        let (emit_b, events_b) = recorder();
        let cursor_ask = |id: &str, key: &str| PermissionAsk {
            id: id.into(),
            provider: "cursor".into(),
            tool: "Shell".into(),
            summary: key.into(),
            detail: None,
            session_option: true,
            kind: "command".into(),
            diff_preview: None,
            rule_summary: Some("Remember exact command/path in Harness".into()),
            session_rule_key: Some(key.into()),
            questions: None,
        };

        // Allow (once) must NOT cache the rule.
        let a1 = desk.approvals("s-a", emit_a.clone());
        let rx1 = a1
            .ask_answer(&cursor_ask("cursor-0", "cursor:shell:touch one.txt"))
            .unwrap();
        let id1 = seen(&events_a, 0);
        desk.decide("s-a", &id1, Decision::Allow).unwrap();
        assert_eq!(rx1.await.unwrap(), Answer::from_decision(Decision::Allow));

        // Asking again still prompts because Allow did not cache.
        let rx2 = a1
            .ask_answer(&cursor_ask("cursor-1", "cursor:shell:touch one.txt"))
            .unwrap();
        let id2 = seen(&events_a, 2);
        desk.decide("s-a", &id2, Decision::AllowSession).unwrap();
        assert_eq!(
            rx2.await.unwrap(),
            Answer::from_decision(Decision::AllowSession)
        );

        // Now cached in s-a: ask_answer AND legacy ask immediately emit
        // allowed_session by session_rule and return Decision::Allow without waiting.
        let a2 = desk.approvals("s-a", emit_a);
        let rx3 = a2
            .ask_answer(&cursor_ask("cursor-0", "cursor:shell:touch one.txt"))
            .unwrap();
        assert_eq!(rx3.await.unwrap(), Answer::from_decision(Decision::Allow));
        let rx4 = a2
            .ask(&cursor_ask("cursor-1", "cursor:shell:touch one.txt"))
            .unwrap();
        assert_eq!(rx4.await.unwrap(), Decision::Allow);
        assert!(desk.waiting("s-a").is_empty());
        let recorded = events_a.lock().unwrap().clone();
        assert_eq!(recorded[4]["state"], "allowed_session");
        assert_eq!(recorded[4]["by"], "session_rule");
        assert_eq!(recorded[5]["state"], "allowed_session");
        assert_eq!(recorded[5]["by"], "session_rule");

        // Another session does NOT see s-a's cached rules.
        let b1 = desk.approvals("s-b", emit_b);
        let rx_b = b1
            .ask_answer(&cursor_ask("cursor-0", "cursor:shell:touch one.txt"))
            .unwrap();
        assert_eq!(desk.waiting("s-b").len(), 1);
        let id_b = seen(&events_b, 0);
        desk.decide("s-b", &id_b, Decision::Deny).unwrap();
        assert_eq!(rx_b.await.unwrap(), Answer::from_decision(Decision::Deny));

        // Bounding keeps at most MAX_SESSION_RULES_PER_SESSION rules per session.
        for i in 0..(MAX_SESSION_RULES_PER_SESSION + 20) {
            desk.remember_session_rule("s-bounded", &format!("cursor:shell:cmd-{i:04}"))
                .unwrap();
        }
        assert_eq!(
            desk.session_rules.lock().unwrap()["s-bounded"].len(),
            MAX_SESSION_RULES_PER_SESSION
        );
    }

    #[tokio::test]
    async fn decide_answer_delivers_structured_answers_and_records_on_event() {
        let desk = Arc::new(PermissionDesk::new_with_timeout(None));
        let (emit, events) = recorder();
        let approvals = desk.approvals("s-q", emit);
        let ask = PermissionAsk {
            id: "claude-q".into(),
            provider: "claude".into(),
            tool: "AskUserQuestion".into(),
            summary: "Pick one".into(),
            kind: "question".into(),
            session_option: false,
            questions: Some(vec![json!({"id": "q1", "question": "Pick one"})]),
            ..Default::default()
        };
        let rx = approvals.ask_answer(&ask).unwrap();
        let id = seen(&events, 0);
        assert_eq!(
            desk.decide_answer(
                "s-q",
                &id,
                Answer {
                    decision: Decision::AllowSession,
                    answers: Some(json!({"q1": "A"})),
                }
            )
            .unwrap_err()
            .status,
            400
        );
        let reply = desk
            .decide_answer(
                "s-q",
                &id,
                Answer {
                    decision: Decision::Allow,
                    answers: Some(json!({"q1": "A"})),
                },
            )
            .unwrap();
        assert_eq!(reply["state"], "allowed");
        assert_eq!(reply["delivered"], true);
        assert_eq!(
            rx.await.unwrap(),
            Answer {
                decision: Decision::Allow,
                answers: Some(json!({"q1": "A"})),
            }
        );
        let recorded = events.lock().unwrap().clone();
        assert_eq!(recorded[1]["state"], "allowed");
        assert_eq!(recorded[1]["by"], "person");
        assert_eq!(recorded[1]["answers"], json!({"q1": "A"}));
    }

    #[tokio::test]
    async fn configured_timeout_expires_unanswered_asks_and_denies_host() {
        let desk = Arc::new(PermissionDesk::new_with_timeout(Some(
            Duration::from_millis(35),
        )));
        let (emit, events) = recorder();
        let approvals = desk.approvals("s-ttl", emit);
        let rx = approvals.ask_answer(&ask("codex-ttl", true)).unwrap();
        let id = seen(&events, 0);
        assert_eq!(desk.waiting("s-ttl").len(), 1);
        assert_eq!(rx.await.unwrap(), Answer::from_decision(Decision::Deny));
        assert!(desk.waiting("s-ttl").is_empty());
        assert!(desk.waiting_counts().is_empty());
        assert_eq!(
            desk.decide("s-ttl", &id, Decision::Allow)
                .unwrap_err()
                .status,
            409
        );
        let recorded = events.lock().unwrap().clone();
        assert_eq!(recorded.len(), 2);
        assert_eq!(recorded[1]["state"], "expired");
        assert_eq!(recorded[1]["by"], "timeout");

        // Answering before timeout cancels the timer and emits only once.
        let rx2 = approvals.ask_answer(&ask("codex-fast", true)).unwrap();
        let id2 = seen(&events, 2);
        desk.decide("s-ttl", &id2, Decision::Allow).unwrap();
        assert_eq!(rx2.await.unwrap(), Answer::from_decision(Decision::Allow));
        tokio::time::sleep(Duration::from_millis(70)).await;
        let recorded = events.lock().unwrap().clone();
        assert_eq!(recorded.len(), 4);
        assert_eq!(recorded[3]["state"], "allowed");
        assert_eq!(recorded[3]["by"], "person");
    }

    #[test]
    fn timeout_env_parser_accepts_only_positive_integers() {
        assert_eq!(parse_timeout_seconds(None), None);
        assert_eq!(parse_timeout_seconds(Some("")), None);
        assert_eq!(parse_timeout_seconds(Some("0")), None);
        assert_eq!(parse_timeout_seconds(Some("-5")), None);
        assert_eq!(parse_timeout_seconds(Some("abc")), None);
        assert_eq!(
            parse_timeout_seconds(Some(" 45 ")),
            Some(Duration::from_secs(45))
        );
        assert_eq!(
            parse_timeout_seconds(Some("999999999999")),
            Some(Duration::from_secs(MAX_TIMEOUT_SECONDS))
        );
    }
}
