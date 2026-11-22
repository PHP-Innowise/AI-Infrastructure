//! Permission requests a running session waits on, and the person's answers.
//!
//! A session's Permissions setting says what an editing launch may do without
//! asking (`infra_host::permission`). With Manual, each request the host asks
//! becomes a `permission` event the page shows with its answers, and the
//! launch waits here until the person answers, the host withdraws the request
//! or the run ends.
use crate::{HarnessError, Result, sessions::Event};
use infra_host::permission::{Answer, Approvals, Decision, PermissionAsk, PermissionMode};
use serde::Serialize;
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

/// Callback notified after pending permission state changes for a session.
pub type ChangeNotifier = Arc<dyn Fn(&str) + Send + Sync>;

/// Inspectable metadata for one remembered session permission rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SessionRuleInfo {
    pub id: String,
    pub rule_key: String,
    pub rule_summary: Option<String>,
    pub tool: String,
    pub provider: String,
    pub created_at: String,
}

/// Stable URL-safe identifier for one session rule key.
pub fn rule_id_for_key(rule_key: &str) -> String {
    crate::store::sha(rule_key.as_bytes())[..16].to_owned()
}

fn timeout_metadata(timeout: Option<Duration>) -> (Option<u64>, Option<String>) {
    let Some(ttl) = timeout else {
        return (None, None);
    };
    let timeout_seconds = ttl
        .as_secs()
        .saturating_add(u64::from(ttl.subsec_nanos() > 0));
    let delta = chrono::TimeDelta::from_std(ttl).unwrap_or_else(|_| {
        chrono::TimeDelta::seconds(i64::try_from(timeout_seconds).unwrap_or(86_400))
    });
    let expires_at =
        (chrono::Utc::now() + delta).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    (Some(timeout_seconds), Some(expires_at))
}

struct Pending {
    nonce: u64,
    sender: tokio::sync::oneshot::Sender<Answer>,
    emit: Event,
    ask: PermissionAsk,
    timeout_task: Option<tokio::task::AbortHandle>,
    timeout_seconds: Option<u64>,
    expires_at: Option<String>,
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
    session_rule_meta: Mutex<BTreeMap<String, BTreeMap<String, SessionRuleInfo>>>,
    read_only_auto_workspaces: Mutex<BTreeMap<String, std::path::PathBuf>>,
    change_notifier: Mutex<Option<ChangeNotifier>>,
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
            session_rule_meta: Mutex::new(BTreeMap::new()),
            read_only_auto_workspaces: Mutex::new(BTreeMap::new()),
            change_notifier: Mutex::new(None),
            timeout,
            next_nonce: AtomicU64::new(1),
        }
    }
    /// Enables automatic scoped `session_rule_key` approval for Gate-1-verified
    /// read-only program commands inside `workspace`.
    pub fn enable_read_only_auto_rules(&self, session: &str, workspace: std::path::PathBuf) {
        if let Ok(mut slot) = self.read_only_auto_workspaces.lock() {
            slot.insert(session.to_owned(), workspace);
        }
    }
    /// Registers a callback invoked after releasing locks whenever a session's
    /// pending permissions change.
    pub fn set_change_notifier(&self, notifier: ChangeNotifier) {
        if let Ok(mut slot) = self.change_notifier.lock() {
            *slot = Some(notifier);
        }
    }
    fn notify_changed(&self, session: &str) {
        let notifier = self
            .change_notifier
            .lock()
            .ok()
            .and_then(|slot| slot.clone());
        if let Some(cb) = notifier {
            cb(session);
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
            self.remember_session_rule_with_ask(session, rule_key, Some(&request.ask))?;
        }
        let state = answer.decision.state();
        let mut event = request.ask.resolved_event(state, "person");
        if answer.decision != Decision::Deny
            && let Some(ref answers) = answer.answers
        {
            event["answers"] = answers.clone();
        }
        (request.emit)(event)?;
        self.notify_changed(session);
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
    #[cfg(test)]
    fn remember_session_rule(&self, session: &str, rule_key: &str) -> Result<()> {
        self.remember_session_rule_with_ask(session, rule_key, None)
    }
    fn remember_session_rule_with_ask(
        &self,
        session: &str,
        rule_key: &str,
        ask: Option<&PermissionAsk>,
    ) -> Result<()> {
        if rule_key.is_empty() || rule_key.len() > MAX_SESSION_RULE_KEY_BYTES {
            return Ok(());
        }
        let mut all_rules = lock(&self.session_rules)?;
        let mut all_meta = lock(&self.session_rule_meta)?;
        if !all_rules.contains_key(session)
            && all_rules.len() >= MAX_RULE_SESSIONS
            && let Some((evicted_session, _)) = all_rules.pop_first()
        {
            all_meta.remove(&evicted_session);
        }
        let rules = all_rules.entry(session.to_owned()).or_default();
        let meta = all_meta.entry(session.to_owned()).or_default();
        if !rules.contains(rule_key)
            && rules.len() >= MAX_SESSION_RULES_PER_SESSION
            && let Some(evicted_key) = rules.pop_first()
        {
            meta.remove(&evicted_key);
        }
        rules.insert(rule_key.to_owned());
        meta.insert(
            rule_key.to_owned(),
            SessionRuleInfo {
                id: rule_id_for_key(rule_key),
                rule_key: rule_key.to_owned(),
                rule_summary: ask.and_then(|item| item.rule_summary.clone()),
                tool: ask.map(|item| item.tool.clone()).unwrap_or_default(),
                provider: ask.map(|item| item.provider.clone()).unwrap_or_default(),
                created_at: crate::store::now(),
            },
        );
        Ok(())
    }
    /// Lists all active remembered session permission rules for `session`.
    pub fn list_session_rules(&self, session: &str) -> Result<Vec<SessionRuleInfo>> {
        let all_rules = lock(&self.session_rules)?;
        let all_meta = lock(&self.session_rule_meta)?;
        let Some(keys) = all_rules.get(session) else {
            return Ok(vec![]);
        };
        let meta = all_meta.get(session);
        Ok(keys
            .iter()
            .map(|rule_key| {
                meta.and_then(|m| m.get(rule_key).cloned())
                    .unwrap_or_else(|| SessionRuleInfo {
                        id: rule_id_for_key(rule_key),
                        rule_key: rule_key.clone(),
                        rule_summary: None,
                        tool: String::new(),
                        provider: String::new(),
                        created_at: String::new(),
                    })
            })
            .collect())
    }
    /// Revokes one remembered session rule (matching `rule_key` or `rule_id`),
    /// or all remembered rules for `session` when `all` is true.
    pub fn revoke_session_rule(
        &self,
        session: &str,
        rule_key: Option<&str>,
        rule_id: Option<&str>,
        all: bool,
    ) -> Result<(usize, Vec<SessionRuleInfo>)> {
        let removed = {
            let mut all_rules = lock(&self.session_rules)?;
            let mut all_meta = lock(&self.session_rule_meta)?;
            if all {
                let count = all_rules.remove(session).map_or(0, |set| set.len());
                all_meta.remove(session);
                count
            } else {
                let target_key =
                    if let Some(key) = rule_key.map(str::trim).filter(|s| !s.is_empty()) {
                        Some(key.to_owned())
                    } else if let Some(id) = rule_id.map(str::trim).filter(|s| !s.is_empty()) {
                        all_rules.get(session).and_then(|keys| {
                            keys.iter()
                                .find(|k| *k == id || rule_id_for_key(k) == id)
                                .cloned()
                        })
                    } else {
                        return Err(HarnessError::invalid(
                            "Provide a rule key, rule id, or all=true to revoke session rules.",
                        ));
                    };
                if let Some(key) = target_key
                    && let Some(keys) = all_rules.get_mut(session)
                    && keys.remove(&key)
                {
                    if let Some(meta) = all_meta.get_mut(session) {
                        meta.remove(&key);
                    }
                    if keys.is_empty() {
                        all_rules.remove(session);
                        all_meta.remove(session);
                    }
                    1
                } else {
                    0
                }
            }
        };
        Ok((removed, self.list_session_rules(session)?))
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
        self.notify_changed(session);
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
        let had_left = !left.is_empty();
        for mut request in left {
            request.cancel_timeout();
            let _ = (request.emit)(request.ask.resolved_event("expired", "harness"));
        }
        if had_left {
            self.notify_changed(session);
        }
    }
    /// The requests a session's run waits on now.
    pub fn waiting(&self, session: &str) -> Vec<Value> {
        lock(&self.pending)
            .map(|pending| {
                pending
                    .iter()
                    .filter(|((owner, _), _)| owner == session)
                    .map(|(_, request)| {
                        request.ask.pending_event_with_timeout(
                            request.timeout_seconds,
                            request.expires_at.as_deref(),
                        )
                    })
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
        if let Some(scoped_key) = self.desk.read_only_auto_rule_for_ask(&self.session, &ask)? {
            let mut ev = ask.resolved_event("allowed_session", "gate1_read_only_rule");
            ev["session_rule_key"] = json!(scoped_key);
            (self.emit)(ev)?;
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
        let (timeout_seconds, expires_at) = timeout_metadata(self.desk.timeout);
        // The page sees the request before anyone can answer it.
        (self.emit)(ask.pending_event_with_timeout(timeout_seconds, expires_at.as_deref()))?;
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
                timeout_seconds,
                expires_at,
            },
        );
        drop(pending);
        self.desk.notify_changed(&self.session);
        Ok(receiver)
    }
    fn withdraw(&self, id: &str) -> Result<()> {
        let request = lock(&self.desk.pending)?.remove(&(self.session.clone(), self.public(id)));
        if let Some(mut request) = request {
            request.cancel_timeout();
            (request.emit)(request.ask.resolved_event("withdrawn", "host"))?;
            self.desk.notify_changed(&self.session);
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

    #[tokio::test]
    async fn timeout_metadata_session_rule_listing_revocation_and_notifier_work() {
        let desk = Arc::new(PermissionDesk::new_with_timeout(Some(Duration::from_secs(
            30,
        ))));
        let changed = Arc::new(Mutex::new(Vec::<String>::new()));
        let changed_hook = changed.clone();
        desk.set_change_notifier(Arc::new(move |sid| {
            changed_hook.lock().unwrap().push(sid.to_owned());
        }));
        let (emit, events) = recorder();
        let approvals = desk.approvals("s-rules", emit);

        let mut req = ask("codex-rule", true);
        req.rule_summary = Some("Allow running cargo test in this session".into());
        req.session_rule_key = Some("codex:cmd:cargo-test".into());
        let rx = approvals.ask_answer(&req).unwrap();
        let waiting = desk.waiting("s-rules");
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0]["timeout_seconds"], 30);
        let exp = waiting[0]["expires_at"].as_str().expect("expires_at");
        chrono::DateTime::parse_from_rfc3339(exp).expect("valid RFC3339 expires_at");

        let recorded = events.lock().unwrap().clone();
        assert_eq!(recorded[0]["timeout_seconds"], 30);
        assert_eq!(recorded[0]["expires_at"], exp);

        let id = seen(&events, 0);
        desk.decide("s-rules", &id, Decision::AllowSession).unwrap();
        assert_eq!(rx.await.unwrap().decision, Decision::AllowSession);

        let rules = desk.list_session_rules("s-rules").unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].rule_key, "codex:cmd:cargo-test");
        assert_eq!(
            rules[0].rule_summary.as_deref(),
            Some("Allow running cargo test in this session")
        );
        assert_eq!(rules[0].id, rule_id_for_key("codex:cmd:cargo-test"));

        let (removed, left) = desk
            .revoke_session_rule("s-rules", None, Some(&rules[0].id), false)
            .unwrap();
        assert_eq!(removed, 1);
        assert!(left.is_empty());
        assert_eq!(changed.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn gate1_read_only_program_rules_auto_approve_safe_commands_and_block_unsafe() {
        let dir = tempfile::tempdir().unwrap();
        let ws = infra_core::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir_all(ws.join("programs")).unwrap();
        std::fs::write(ws.join("programs/check.py"), "print('ok')\n").unwrap();

        let safe_key = classify_read_only_program_command(Some(&ws), "git status --short")
            .unwrap()
            .unwrap();
        assert_eq!(safe_key, "readonly:program:git status --short");

        let capsule_key =
            classify_read_only_program_command(Some(&ws), "python3 programs/check.py")
                .unwrap()
                .unwrap();
        assert_eq!(capsule_key, "readonly:program:python3 programs/check.py");

        assert!(classify_read_only_program_command(Some(&ws), "python3 -c 'print(1)'").is_err());
        assert!(classify_read_only_program_command(Some(&ws), "python3 ../escape.py").is_err());
        assert!(classify_read_only_program_command(Some(&ws), "rm -rf /tmp/x").is_err());
        assert_eq!(
            classify_read_only_program_command(Some(&ws), "rm -rf file.txt").unwrap(),
            None
        );

        let desk = Arc::new(PermissionDesk::default());
        desk.enable_read_only_auto_rules("s-ro", ws.clone());
        let (emit, events) = recorder();
        let approvals = desk.approvals("s-ro", emit);
        let mut req = ask("codex-ro-1", true);
        req.summary = "python3 programs/check.py".into();
        req.kind = "command".into();
        req.session_rule_key = Some(capsule_key.clone());

        let rx = approvals.ask_answer(&req).unwrap();
        assert_eq!(rx.await.unwrap().decision, Decision::Allow);
        assert!(desk.waiting("s-ro").is_empty());
        let evs = events.lock().unwrap().clone();
        assert_eq!(evs[0]["state"], "allowed_session");
        assert_eq!(evs[0]["by"], "gate1_read_only_rule");
        assert!(desk.has_session_rule("s-ro", &capsule_key).unwrap());
    }
}

const READ_ONLY_INLINE_FLAGS: &[&str] = &[
    "-c", "-e", "-E", "--eval", "-p", "--print", "-Command", "/c", "/C", "/k", "/K",
];

fn tokenize_program_command(command: &str) -> std::result::Result<Vec<String>, String> {
    if command.is_empty() || command.len() > 2048 {
        return Err("command length out of bounds".into());
    }
    if command.chars().any(|c| {
        matches!(
            c,
            '|' | ';' | '&' | '>' | '<' | '`' | '$' | '\n' | '\r' | '\0'
        )
    }) {
        return Err("shell operators are not permitted in deterministic read-only commands".into());
    }
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut in_token = false;
    let mut quote: Option<char> = None;
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) if c == q => {
                quote = None;
                in_token = true;
            }
            Some('"') if c == '\\' => {
                let next = chars
                    .next()
                    .ok_or_else(|| "unterminated escape in command".to_owned())?;
                cur.push(next);
                in_token = true;
            }
            Some(_) => {
                cur.push(c);
                in_token = true;
            }
            None if c.is_whitespace() => {
                if in_token {
                    tokens.push(std::mem::take(&mut cur));
                    in_token = false;
                }
            }
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                in_token = true;
            }
            None if c == '\\' => {
                let next = chars
                    .next()
                    .ok_or_else(|| "unterminated escape in command".to_owned())?;
                cur.push(next);
                in_token = true;
            }
            None => {
                cur.push(c);
                in_token = true;
            }
        }
    }
    if quote.is_some() {
        return Err("unclosed quote in command".into());
    }
    if in_token {
        tokens.push(cur);
    }
    Ok(tokens)
}

fn check_read_only_capsule_program(
    workspace: &std::path::Path,
    tokens: &[String],
) -> std::result::Result<bool, String> {
    let Some(rel_prog) = tokens.iter().skip(1).find_map(|arg| {
        let mut rest = arg.split_once('=').map_or(arg.as_str(), |(_, v)| v);
        while let Some(t) = rest.strip_prefix("./").or_else(|| rest.strip_prefix(".\\")) {
            rest = t;
        }
        rest.starts_with("programs/").then(|| rest.to_owned())
    }) else {
        return Ok(false);
    };
    let prog_path = workspace.join(&rel_prog);
    let body = std::fs::read(&prog_path)
        .map_err(|_| format!("program file `{rel_prog}` does not exist inside workspace"))?;
    let mut files = infra_skillcache::package::Files::new();
    files.insert(&rel_prog, body);
    let runtime = tokens.first().cloned().unwrap_or_default();
    let capsule_json = json!({
        "schema_version": infra_skillcache::SCHEMA_VERSION,
        "form": "hybrid",
        "mu": {
            "name": "readonly-check",
            "description": "Read-only permission rule program check"
        },
        "source": {
            "skills": [{
                "name": "readonly-check",
                "path": ".agents/skills/readonly-check/SKILL.md",
                "tree_sha256": "0000000000000000000000000000000000000000000000000000000000000000"
            }]
        },
        "capabilities": {
            "network": "none",
            "runtimes": [runtime]
        },
        "bindings": [],
        "programs": [{
            "id": "main",
            "entry": tokens,
            "inputs": [],
            "outputs": [],
            "check": tokens,
            "timeout_seconds": 60,
            "read_only": true
        }],
        "steps": [{
            "kind": "program",
            "program": "main"
        }],
        "verification": []
    });
    let capsule =
        infra_skillcache::capsule::SkillCapsule::from_value(&capsule_json).map_err(|v| {
            v.into_iter()
                .map(|e| format!("{}: {}", e.code, e.message))
                .collect::<Vec<_>>()
                .join("; ")
        })?;
    let violations = infra_skillcache::gate1::structure(&capsule, &files);
    if !violations.is_empty() {
        return Err(violations
            .into_iter()
            .map(|v| format!("{}: {}", v.code, v.message))
            .collect::<Vec<_>>()
            .join("; "));
    }
    Ok(true)
}

fn is_known_read_only_cli(tokens: &[String]) -> bool {
    let Some(cmd) = tokens.first().map(|s| {
        std::path::Path::new(s)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(s.as_str())
    }) else {
        return false;
    };
    let lower = cmd.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "rg" | "grep" | "cat" | "head" | "tail" | "ls" | "stat" | "wc" | "pwd" | "find"
    ) {
        return !tokens.iter().any(|a| {
            matches!(
                a.as_str(),
                "-delete" | "-exec" | "-execdir" | "-ok" | "-fprint"
            )
        });
    }
    if lower == "git" {
        return tokens.get(1).is_some_and(|sub| {
            matches!(
                sub.as_str(),
                "status" | "diff" | "log" | "show" | "rev-parse"
            )
        });
    }
    if lower == "php" {
        return tokens.get(1).is_some_and(|f| f == "-l");
    }
    if lower == "composer" {
        return tokens
            .get(1)
            .is_some_and(|s| matches!(s.as_str(), "validate" | "show"));
    }
    if lower == "cargo" {
        return tokens
            .get(1)
            .is_some_and(|s| matches!(s.as_str(), "check" | "metadata"));
    }
    false
}

/// Verifies whether `command` is a Gate-1 / verification-safe read-only program command
/// and returns its deterministic scoped `session_rule_key` (`readonly:program:...`).
pub fn classify_read_only_program_command(
    workspace: Option<&std::path::Path>,
    command: &str,
) -> std::result::Result<Option<String>, String> {
    let tokens = tokenize_program_command(command)?;
    let Some(runtime) = tokens.first() else {
        return Ok(None);
    };
    if runtime.is_empty() {
        return Ok(None);
    }
    for (idx, arg) in tokens.iter().enumerate() {
        let lower = arg.to_ascii_lowercase();
        if lower.ends_with(".cmd") || lower.ends_with(".bat") {
            return Err("argv_windows_script: Windows .cmd/.bat launchers are not admitted".into());
        }
        if idx > 0 && READ_ONLY_INLINE_FLAGS.contains(&arg.as_str()) {
            return Err("argv_inline_code: inline code flags are not admitted".into());
        }
        let path_part = arg.split_once('=').map_or(arg.as_str(), |(_, v)| v);
        if path_part.starts_with('~')
            || path_part
                .split(['/', '\\'])
                .any(|part| part == ".." || (part.starts_with('-') && part.ends_with("..")))
        {
            return Err("argv_parent_traversal: parent or home traversal is not admitted".into());
        }
        if path_part.starts_with('/') || path_part.starts_with('\\') {
            return Err("argv_absolute_path: absolute paths are not admitted in auto rules".into());
        }
    }

    if let Some(ws) = workspace
        && check_read_only_capsule_program(ws, &tokens)?
    {
        return Ok(Some(format!("readonly:program:{}", tokens.join(" "))));
    }
    let generator_safe = workspace
        .and_then(|ws| infra_generator::commands::is_verification_safe(ws, command).ok())
        .unwrap_or(false);
    if is_known_read_only_cli(&tokens) || generator_safe {
        return Ok(Some(format!("readonly:program:{}", tokens.join(" "))));
    }
    Ok(None)
}

impl PermissionDesk {
    fn read_only_auto_rule_for_ask(
        &self,
        session: &str,
        ask: &PermissionAsk,
    ) -> Result<Option<String>> {
        if ask.kind != "command" || ask.summary.trim().is_empty() {
            return Ok(None);
        }
        let workspace = self
            .read_only_auto_workspaces
            .lock()
            .ok()
            .and_then(|m| m.get(session).cloned());
        let Some(ws) = workspace else {
            return Ok(None);
        };
        let Ok(Some(scoped_key)) = classify_read_only_program_command(Some(&ws), &ask.summary)
        else {
            return Ok(None);
        };
        if let Some(requested_key) = ask.session_rule_key.as_deref()
            && !requested_key.is_empty()
            && requested_key != scoped_key
        {
            return Ok(None);
        }
        self.remember_session_rule_with_ask(session, &scoped_key, Some(ask))?;
        Ok(Some(scoped_key))
    }
}
