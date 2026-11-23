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
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
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

#[derive(Clone)]
struct ReadOnlyAutoScope {
    workspace: std::path::PathBuf,
    store: Option<crate::store::SessionStore>,
}

/// Every request waiting for a person, by session and request.
pub struct PermissionDesk {
    pending: Mutex<BTreeMap<(String, String), Pending>>,
    session_rules: Mutex<BTreeMap<String, BTreeSet<String>>>,
    session_rule_meta: Mutex<BTreeMap<String, BTreeMap<String, SessionRuleInfo>>>,
    read_only_auto_workspaces: Mutex<BTreeMap<String, ReadOnlyAutoScope>>,
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
    /// Enables automatic scoped `session_rule_key` approval for verified
    /// read-only CLI commands inside `workspace`.
    pub fn enable_read_only_auto_rules(&self, session: &str, workspace: std::path::PathBuf) {
        if let Ok(mut slot) = self.read_only_auto_workspaces.lock() {
            slot.insert(
                session.to_owned(),
                ReadOnlyAutoScope {
                    workspace,
                    store: None,
                },
            );
        }
    }
    /// Enables automatic scoped `session_rule_key` approval inside `workspace`
    /// with `store` access so active, gate-verified sealed `read_only` SkillCache
    /// capsules can also be verified.
    pub fn enable_read_only_auto_rules_with_store(
        &self,
        session: &str,
        workspace: std::path::PathBuf,
        store: crate::store::SessionStore,
    ) {
        if let Ok(mut slot) = self.read_only_auto_workspaces.lock() {
            slot.insert(
                session.to_owned(),
                ReadOnlyAutoScope {
                    workspace,
                    store: Some(store),
                },
            );
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

const READ_ONLY_INLINE_FLAGS: &[&str] = &[
    "-c", "-e", "-E", "--eval", "-p", "--print", "-Command", "/c", "/C", "/k", "/K",
];

fn tokenize_program_command(command: &str) -> std::result::Result<Vec<String>, String> {
    if command.is_empty() || command.len() > 2048 {
        return Err("command length out of bounds".into());
    }
    if command
        .chars()
        .any(|c| matches!(c, '>' | '<' | '`' | '$' | '\n' | '\r' | '\0'))
    {
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
            None if matches!(c, '|' | ';' | '&') => {
                return Err(
                    "shell operators are not permitted in deterministic read-only commands".into(),
                );
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

type FileStamp = Option<(i64, u64)>;
type VerifiedCapsuleCacheKey = (String, String, String, i64);

const MAX_CACHED_WORKSPACES: usize = 64;
const MAX_CACHED_CAPSULES_PER_WS: usize = 128;

#[derive(Clone)]
struct VerifiedCapsuleSnapshot {
    package: Arc<infra_skillcache::package::Package>,
}

struct WorkspaceReadOnlyCacheEntry {
    analyzer_stamps: (FileStamp, FileStamp),
    analyzer: Option<Arc<infra_generator::commands::CommandAnalyzer>>,
    verified_capsules: BTreeMap<VerifiedCapsuleCacheKey, VerifiedCapsuleSnapshot>,
    #[cfg(test)]
    analyzer_builds: usize,
    #[cfg(test)]
    program_reads: usize,
    #[cfg(test)]
    gate1_evals: usize,
}

#[derive(Default)]
struct ReadOnlyWorkspaceCache {
    workspaces: BTreeMap<PathBuf, WorkspaceReadOnlyCacheEntry>,
}

fn read_only_workspace_cache() -> &'static Mutex<ReadOnlyWorkspaceCache> {
    static CACHE: OnceLock<Mutex<ReadOnlyWorkspaceCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(ReadOnlyWorkspaceCache::default()))
}

fn path_stamp(path: &Path) -> FileStamp {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if infra_core::fs::linked(&meta) {
        return None;
    }
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_nanos()).ok())
        .unwrap_or(0);
    Some((mtime_ns, meta.len()))
}

fn workspace_analyzer_stamps(ws: &Path) -> (FileStamp, FileStamp) {
    (
        path_stamp(&ws.join("composer.json")),
        path_stamp(&ws.join("package.json")),
    )
}

fn ensure_workspace_entry<'a>(
    cache: &'a mut ReadOnlyWorkspaceCache,
    ws: &Path,
) -> &'a mut WorkspaceReadOnlyCacheEntry {
    if !cache.workspaces.contains_key(ws) && cache.workspaces.len() >= MAX_CACHED_WORKSPACES {
        cache.workspaces.clear();
    }
    cache
        .workspaces
        .entry(ws.to_path_buf())
        .or_insert_with(|| WorkspaceReadOnlyCacheEntry {
            analyzer_stamps: (None, None),
            analyzer: None,
            verified_capsules: BTreeMap::new(),
            #[cfg(test)]
            analyzer_builds: 0,
            #[cfg(test)]
            program_reads: 0,
            #[cfg(test)]
            gate1_evals: 0,
        })
}

fn cached_command_analyzer(ws: &Path) -> Option<Arc<infra_generator::commands::CommandAnalyzer>> {
    let ws_canon = infra_core::fs::canonicalize(ws).ok()?;
    let stamps = workspace_analyzer_stamps(&ws_canon);
    if let Ok(guard) = read_only_workspace_cache().lock()
        && let Some(entry) = guard.workspaces.get(&ws_canon)
        && entry.analyzer.is_some()
        && entry.analyzer_stamps == stamps
    {
        return entry.analyzer.clone();
    }
    let built = infra_generator::commands::CommandAnalyzer::new(&ws_canon)
        .ok()
        .map(Arc::new);
    if let Ok(mut guard) = read_only_workspace_cache().lock() {
        let entry = ensure_workspace_entry(&mut guard, &ws_canon);
        #[cfg(test)]
        {
            entry.analyzer_builds += 1;
        }
        entry.analyzer_stamps = stamps;
        entry.analyzer = built.clone();
    }
    built
}

/// Pre-warms cached `CommandAnalyzer` entries for `workspace`.
pub(crate) fn warm_read_only_cache(workspace: &Path) {
    let Ok(ws_canon) = infra_core::fs::canonicalize(workspace) else {
        return;
    };
    let _ = cached_command_analyzer(&ws_canon);
}

#[cfg(test)]
fn read_only_cache_counters(ws: &Path) -> (usize, usize, usize) {
    let Ok(canon) = infra_core::fs::canonicalize(ws) else {
        return (0, 0, 0);
    };
    let guard = read_only_workspace_cache().lock().unwrap();
    guard.workspaces.get(&canon).map_or((0, 0, 0), |entry| {
        (
            entry.analyzer_builds,
            entry.program_reads,
            entry.gate1_evals,
        )
    })
}

fn package_relative_arg(arg: &str) -> Option<String> {
    let mut rest = arg.split_once('=').map_or(arg, |(_, v)| v);
    while let Some(t) = rest.strip_prefix("./").or_else(|| rest.strip_prefix(".\\")) {
        rest = t;
    }
    (rest.starts_with("programs/") || rest.starts_with("assets/")).then(|| rest.to_owned())
}

fn latest_round_gates_passed(db: &rusqlite::Connection, capsule_id: &str) -> Option<i64> {
    let gates = crate::skillcache::db::gates(db, capsule_id).ok()?;
    let max_round = gates.iter().filter_map(|g| g["round"].as_i64()).max()?;
    let mut g1 = false;
    let mut g2 = false;
    let mut g3 = false;
    for row in gates {
        if row["round"].as_i64() != Some(max_round) {
            continue;
        }
        if row["passed"] != true {
            return None;
        }
        match row["gate"].as_str() {
            Some("gate1") => g1 = true,
            Some("gate2") => g2 = true,
            Some("gate3") => g3 = true,
            _ => {}
        }
    }
    (g1 && g2 && g3).then_some(max_round)
}

fn workspace_file_matches_sealed(
    ws_canon: &Path,
    rel: &str,
    sealed_bytes: &[u8],
) -> std::result::Result<(), String> {
    let ws_path = ws_canon.join(rel);
    match std::fs::symlink_metadata(&ws_path) {
        Ok(meta) => {
            if infra_core::fs::linked(&meta) || !meta.is_file() {
                return Err(format!(
                    "program_symlink_refused: linked or non-regular `{rel}` is not admitted"
                ));
            }
            let Ok(disk_bytes) = std::fs::read(&ws_path) else {
                return Err(format!("program_unreadable: `{rel}` is not readable"));
            };
            if infra_skillcache::fingerprint::sha256(&disk_bytes)
                != infra_skillcache::fingerprint::sha256(sealed_bytes)
            {
                return Err(format!(
                    "program_digest_mismatch: workspace `{rel}` does not match sealed capsule"
                ));
            }
            Ok(())
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(format!("program_unreadable: `{rel}` is not readable")),
    }
}

fn load_verified_sealed_capsule(
    ws_canon: &Path,
    db: &rusqlite::Connection,
    capsule_row: &Value,
) -> Option<Arc<infra_skillcache::package::Package>> {
    let capsule_id = capsule_row["capsule_id"].as_str()?;
    let expected_cap_sha = capsule_row["capsule_sha256"]
        .as_str()
        .filter(|s| !s.is_empty())?;
    let expected_pkg_sha = capsule_row["package_sha256"]
        .as_str()
        .filter(|s| !s.is_empty())?;
    let artifact_dir = capsule_row["artifact_dir"]
        .as_str()
        .filter(|s| !s.is_empty())?;
    let max_round = latest_round_gates_passed(db, capsule_id)?;
    let warm_dir = PathBuf::from(artifact_dir).join("warm");
    let cache_key = (
        capsule_id.to_owned(),
        expected_cap_sha.to_owned(),
        expected_pkg_sha.to_owned(),
        max_round,
    );

    if let Ok(guard) = read_only_workspace_cache().lock()
        && let Some(entry) = guard.workspaces.get(ws_canon)
        && let Some(hit) = entry.verified_capsules.get(&cache_key)
        && infra_skillcache::package::digest(&warm_dir).ok().as_deref() == Some(expected_pkg_sha)
    {
        return Some(hit.package.clone());
    }

    let loaded = infra_skillcache::package::load(&warm_dir).ok()?;
    if loaded.capsule.form != infra_skillcache::Form::Hybrid
        || loaded.capsule_sha256 != expected_cap_sha
        || loaded.package_sha256 != expected_pkg_sha
        || !infra_skillcache::gate1::structure(&loaded.capsule, &loaded.files).is_empty()
    {
        return None;
    }
    let pkg = Arc::new(loaded);
    if let Ok(mut guard) = read_only_workspace_cache().lock() {
        let entry = ensure_workspace_entry(&mut guard, ws_canon);
        #[cfg(test)]
        {
            entry.program_reads += 1;
            entry.gate1_evals += 1;
        }
        if entry.verified_capsules.len() >= MAX_CACHED_CAPSULES_PER_WS {
            entry.verified_capsules.clear();
        }
        entry.verified_capsules.insert(
            cache_key,
            VerifiedCapsuleSnapshot {
                package: pkg.clone(),
            },
        );
    }
    Some(pkg)
}

fn check_read_only_capsule_program(
    workspace: &std::path::Path,
    store: Option<&crate::store::SessionStore>,
    tokens: &[String],
) -> std::result::Result<bool, String> {
    let rel_refs: Vec<String> = tokens
        .iter()
        .skip(1)
        .filter_map(|arg| package_relative_arg(arg))
        .collect();
    if !rel_refs.iter().any(|rel| rel.starts_with("programs/")) {
        return Ok(false);
    }
    let ws_canon = infra_core::fs::canonicalize(workspace)
        .map_err(|_| "workspace path does not exist".to_owned())?;
    for rel in &rel_refs {
        let ws_path = ws_canon.join(rel);
        if let Ok(meta) = std::fs::symlink_metadata(&ws_path)
            && (infra_core::fs::linked(&meta) || !meta.is_file())
        {
            return Err(format!(
                "program_symlink_refused: linked or non-regular `{rel}` is not admitted"
            ));
        }
    }
    let Some(store) = store else {
        return Ok(false);
    };
    let Ok(projects) = store.projects() else {
        return Ok(false);
    };
    let mut matching_projects: Vec<(usize, String)> = projects
        .into_iter()
        .filter_map(|p| {
            let id = p["id"].as_str()?.to_owned();
            let proj_path = p["path"]
                .as_str()
                .and_then(|s| infra_core::fs::canonicalize(s).ok())?;
            if ws_canon == proj_path || ws_canon.starts_with(&proj_path) {
                Some((proj_path.components().count(), id))
            } else {
                None
            }
        })
        .collect();
    matching_projects.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let Ok(db) = store.connection() else {
        return Ok(false);
    };
    for (_, project_id) in matching_projects {
        let Ok(capsules) = crate::skillcache::db::capsules(
            &db,
            &project_id,
            None,
            Some("active"),
            Some("hybrid"),
            false,
            false,
        ) else {
            continue;
        };
        for row in capsules {
            let Some(pkg) = load_verified_sealed_capsule(&ws_canon, &db, &row) else {
                continue;
            };
            let matches_program = pkg.capsule.programs.iter().any(|program| {
                program.read_only && (tokens == program.entry || tokens == program.check)
            });
            if !matches_program {
                continue;
            }
            let mut all_files_verified = true;
            for rel in &rel_refs {
                let Some(sealed_bytes) = pkg.files.get(rel) else {
                    all_files_verified = false;
                    break;
                };
                if workspace_file_matches_sealed(&ws_canon, rel, sealed_bytes).is_err() {
                    all_files_verified = false;
                    break;
                }
            }
            if all_files_verified {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

const ALIGNED_READONLY_CLIS: &[&str] = &[
    "ava",
    "behat",
    "cat",
    "date",
    "deptrac",
    "diff",
    "echo",
    "eslint",
    "false",
    "find",
    "flake8",
    "gofmt",
    "grep",
    "head",
    "jest",
    "ls",
    "mocha",
    "mypy",
    "parallel-lint",
    "paratest",
    "pest",
    "phpcs",
    "phploc",
    "phpmd",
    "phpspec",
    "phpstan",
    "phpunit",
    "printf",
    "psalm",
    "pwd",
    "pylint",
    "pytest",
    "rg",
    "sort",
    "stylelint",
    "tail",
    "test",
    "true",
    "tsc",
    "uniq",
    "unittest",
    "vitest",
    "wc",
    "which",
];

const GENERATOR_MUTATE_FLAGS: &[&str] = &[
    "--fix",
    "--fix-dry-run=false",
    "--write",
    "-w",
    "--apply",
    "--update",
    "--alter",
    "--generate-baseline",
    "--set-baseline",
    "--update-baseline",
    "--migrate-configuration",
];

fn has_mutating_flag(tokens: &[String]) -> bool {
    tokens.iter().any(|arg| {
        let base = arg.split('=').next().unwrap_or(arg.as_str());
        GENERATOR_MUTATE_FLAGS.contains(&arg.as_str())
            || GENERATOR_MUTATE_FLAGS.contains(&base)
            || arg == "--fix-dry-run=false"
    })
}

fn has_cli_output_or_exec_override(tokens: &[String]) -> bool {
    let Some(cmd) = tokens.first().map(|s| {
        std::path::Path::new(s)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(s.as_str())
    }) else {
        return false;
    };
    let lower = cmd.to_ascii_lowercase();
    let args = &tokens[1..];
    match lower.as_str() {
        "sort" => args.iter().any(|arg| {
            arg == "-o"
                || arg == "--output"
                || arg.starts_with("--output=")
                || (!arg.starts_with("--") && arg.starts_with('-') && arg[1..].contains('o'))
        }),
        "rg" => args
            .iter()
            .any(|arg| arg == "--pre" || arg.starts_with("--pre=")),
        "git" => args.iter().any(|arg| {
            arg == "--output"
                || arg.starts_with("--output=")
                || arg == "--ext-diff"
                || arg == "-c"
                || arg.starts_with("-c=")
        }),
        _ => false,
    }
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
    if has_mutating_flag(tokens) || has_cli_output_or_exec_override(tokens) {
        return false;
    }
    let lower = cmd.to_ascii_lowercase();
    if ALIGNED_READONLY_CLIS.contains(&lower.as_str()) {
        if lower == "find" {
            return !tokens.iter().any(|a| {
                matches!(
                    a.as_str(),
                    "-delete"
                        | "-exec"
                        | "-execdir"
                        | "-ok"
                        | "-okdir"
                        | "-fprint"
                        | "-fprint0"
                        | "-fprintf"
                        | "-fls"
                )
            });
        }
        return true;
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

/// Verifies whether `command` is a verified read-only CLI or an active,
/// gate-verified sealed `read_only` SkillCache capsule program command, and returns
/// its deterministic scoped `session_rule_key` (`readonly:program:...`).
pub fn classify_read_only_program_command(
    workspace: Option<&std::path::Path>,
    command: &str,
) -> std::result::Result<Option<String>, String> {
    classify_read_only_program_command_in_store(workspace, None, command)
}

/// Like [`classify_read_only_program_command`], additionally checking `store` for
/// active, gate-verified sealed `read_only` SkillCache capsules matching `command`.
pub fn classify_read_only_program_command_in_store(
    workspace: Option<&std::path::Path>,
    store: Option<&crate::store::SessionStore>,
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
        && check_read_only_capsule_program(ws, store, &tokens)?
    {
        return Ok(Some(format!("readonly:program:{}", tokens.join(" "))));
    }
    if has_mutating_flag(&tokens) || has_cli_output_or_exec_override(&tokens) {
        return Ok(None);
    }
    let generator_safe = workspace
        .and_then(cached_command_analyzer)
        .and_then(|analyzer| analyzer.analyze(command, true, None).ok())
        .is_some_and(|analysis| analysis.verification_safe);
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
        let scope = self
            .read_only_auto_workspaces
            .lock()
            .ok()
            .and_then(|m| m.get(session).cloned());
        let Some(scope) = scope else {
            return Ok(None);
        };
        let Ok(Some(scoped_key)) = classify_read_only_program_command_in_store(
            Some(&scope.workspace),
            scope.store.as_ref(),
            &ask.summary,
        ) else {
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

    fn seed_sealed_capsule(
        store: &crate::store::SessionStore,
        project_id: &str,
        capsule_id: &str,
        program_rel: &str,
        program_body: &[u8],
        read_only: bool,
        passed_gates: &[&str],
    ) -> (PathBuf, String) {
        let warm_root = store
            .state
            .join(format!("skillcache/capsules/{project_id}/{capsule_id}/1"));
        let warm_dir = warm_root.join("warm");
        let _ = infra_skillcache::package::remove(&warm_dir);
        let cmd = vec!["python3".to_string(), program_rel.to_string()];
        let check_cmd = vec![
            "python3".to_string(),
            program_rel.to_string(),
            "--check".to_string(),
        ];
        let capsule_json = json!({
            "schema_version": infra_skillcache::SCHEMA_VERSION,
            "form": "hybrid",
            "mu": {"name": "ro-check", "description": "Read-only capsule check"},
            "source": {"skills": [{"name": "ro-check", "path": ".agents/skills/ro-check/SKILL.md", "tree_sha256": "0".repeat(64)}]},
            "capabilities": {"network": "none", "runtimes": ["python3"]},
            "bindings": [],
            "programs": [{
                "id": "main",
                "entry": cmd,
                "inputs": [],
                "outputs": [],
                "check": check_cmd,
                "timeout_seconds": 60,
                "read_only": read_only
            }],
            "steps": [{"kind": "program", "program": "main"}],
            "verification": []
        });
        let cap =
            infra_skillcache::capsule::SkillCapsule::from_value(&capsule_json).expect("capsule");
        let mut files = infra_skillcache::package::Files::new();
        files.insert(program_rel, program_body.to_vec());
        let pkg_sha =
            infra_skillcache::package::write_warm(&warm_dir, &cap, None, &files).expect("warm");
        let loaded = infra_skillcache::package::load(&warm_dir).expect("load");
        let db = store.connection().unwrap();
        let skill_ref = json!({
            "name": "ro-check",
            "path": ".agents/skills/ro-check/SKILL.md",
            "tree_sha256": "0".repeat(64)
        });
        crate::skillcache::db::begin_capsule(
            &db,
            capsule_id,
            &format!("cmp-{capsule_id}"),
            project_id,
            "s-seed",
            &format!("gen-{capsule_id}"),
            &skill_ref,
            None,
        )
        .unwrap();
        crate::skillcache::db::activate_capsule(
            &db,
            &crate::skillcache::db::Qualified {
                capsule_id,
                mu_name: "ro-check",
                mu_description: "Read-only capsule check",
                form: "hybrid",
                capsule_sha256: &loaded.capsule_sha256,
                package_sha256: &pkg_sha,
                skill_files: &json!({}),
                env_fingerprint: "env",
                engine_digest: "eng",
                artifact_dir: warm_root.to_str().unwrap(),
                capabilities: &json!({"network": "none", "runtimes": ["python3"]}),
            },
        )
        .unwrap();
        for gate in passed_gates {
            crate::skillcache::db::record_gate(
                &db,
                capsule_id,
                1,
                gate,
                true,
                &json!({"gate": gate, "passed": true}),
            )
            .unwrap();
        }
        (warm_dir, pkg_sha)
    }

    #[tokio::test]
    async fn gate1_read_only_program_rules_auto_approve_safe_commands_and_block_unsafe() {
        let dir = tempfile::tempdir().unwrap();
        let root = infra_core::fs::canonicalize(dir.path()).unwrap();
        let ws = root.join("ws");
        std::fs::create_dir_all(ws.join("programs")).unwrap();
        let prog_bytes = b"print('ok')\n";
        std::fs::write(ws.join("programs/check.py"), prog_bytes).unwrap();

        let safe_key = classify_read_only_program_command(Some(&ws), "git status --short")
            .unwrap()
            .unwrap();
        assert_eq!(safe_key, "readonly:program:git status --short");

        // Arbitrary unsealed workspace `programs/...` without an active sealed capsule MUST NOT auto-approve.
        assert_eq!(
            classify_read_only_program_command(Some(&ws), "python3 programs/check.py").unwrap(),
            None
        );

        assert!(classify_read_only_program_command(Some(&ws), "python3 -c 'print(1)'").is_err());
        assert!(classify_read_only_program_command(Some(&ws), "python3 ../escape.py").is_err());
        assert!(classify_read_only_program_command(Some(&ws), "rm -rf /tmp/x").is_err());
        assert_eq!(
            classify_read_only_program_command(Some(&ws), "rm -rf file.txt").unwrap(),
            None
        );

        let store = crate::store::SessionStore::open(&root.join("state")).unwrap();
        let project_id = store.register(&ws).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();

        // Still denied when no capsule is in store.
        assert_eq!(
            classify_read_only_program_command_in_store(
                Some(&ws),
                Some(&store),
                "python3 programs/check.py",
            )
            .unwrap(),
            None
        );

        // Capsule missing Gate 3 replay must NOT auto-approve.
        seed_sealed_capsule(
            &store,
            &project_id,
            "cap-no-g3",
            "programs/check.py",
            prog_bytes,
            true,
            &["gate1", "gate2"],
        );
        assert_eq!(
            classify_read_only_program_command_in_store(
                Some(&ws),
                Some(&store),
                "python3 programs/check.py",
            )
            .unwrap(),
            None
        );

        // Capsule with read_only=false must NOT auto-approve even with all 3 gates.
        seed_sealed_capsule(
            &store,
            &project_id,
            "cap-rw",
            "programs/check.py",
            prog_bytes,
            false,
            &["gate1", "gate2", "gate3"],
        );
        assert_eq!(
            classify_read_only_program_command_in_store(
                Some(&ws),
                Some(&store),
                "python3 programs/check.py",
            )
            .unwrap(),
            None
        );

        // Active, sealed, Gate 1+2+3 verified read_only=true capsule IS approved.
        let (warm_dir, _) = seed_sealed_capsule(
            &store,
            &project_id,
            "cap-ro",
            "programs/check.py",
            prog_bytes,
            true,
            &["gate1", "gate2", "gate3"],
        );
        let capsule_key = classify_read_only_program_command_in_store(
            Some(&ws),
            Some(&store),
            "python3 programs/check.py",
        )
        .unwrap()
        .unwrap();
        assert_eq!(capsule_key, "readonly:program:python3 programs/check.py");

        // Extra arguments outside verified entry/check are rejected.
        assert_eq!(
            classify_read_only_program_command_in_store(
                Some(&ws),
                Some(&store),
                "python3 programs/check.py --extra",
            )
            .unwrap(),
            None
        );

        let desk = Arc::new(PermissionDesk::default());
        desk.enable_read_only_auto_rules_with_store("s-ro", ws.clone(), store.clone());
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

        // Mutating workspace file to mismatch sealed SHA-256 revokes auto-approval.
        std::fs::write(
            ws.join("programs/check.py"),
            b"import os; os.system('id')\n",
        )
        .unwrap();
        assert_eq!(
            classify_read_only_program_command_in_store(
                Some(&ws),
                Some(&store),
                "python3 programs/check.py",
            )
            .unwrap(),
            None
        );

        // Restoring workspace bytes re-enables, and invalidating or tampering sealed package revokes.
        std::fs::write(ws.join("programs/check.py"), prog_bytes).unwrap();
        assert!(
            classify_read_only_program_command_in_store(
                Some(&ws),
                Some(&store),
                "python3 programs/check.py",
            )
            .unwrap()
            .is_some()
        );
        infra_skillcache::package::release(&warm_dir);
        let tampered_prog = warm_dir.join("programs/check.py");
        let mut perms = std::fs::metadata(&tampered_prog).unwrap().permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            perms.set_mode(0o644);
        }
        #[cfg(not(unix))]
        {
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
        }
        std::fs::set_permissions(&tampered_prog, perms).unwrap();
        std::fs::write(&tampered_prog, b"print('tampered')\n").unwrap();
        assert_eq!(
            classify_read_only_program_command_in_store(
                Some(&ws),
                Some(&store),
                "python3 programs/check.py",
            )
            .unwrap(),
            None
        );
        let _ = infra_skillcache::package::remove(&warm_dir);
    }

    #[test]
    fn readonly_cli_alignment_and_workspace_cache_reuse_and_invalidate() {
        let dir = tempfile::tempdir().unwrap();
        let root = infra_core::fs::canonicalize(dir.path()).unwrap();
        let ws = root.join("ws");
        std::fs::create_dir_all(ws.join("programs")).unwrap();
        let lint_bytes = b"print('ok')\n";
        std::fs::write(ws.join("programs/lint.py"), lint_bytes).unwrap();
        std::fs::write(
            ws.join("composer.json"),
            r#"{"scripts":{"test":"phpunit"}}"#,
        )
        .unwrap();

        // Aligned READONLY CLIs and rejection of mutating flags.
        assert_eq!(
            classify_read_only_program_command(Some(&ws), "phpstan analyse src")
                .unwrap()
                .as_deref(),
            Some("readonly:program:phpstan analyse src")
        );
        assert_eq!(
            classify_read_only_program_command(Some(&ws), "pytest -q")
                .unwrap()
                .as_deref(),
            Some("readonly:program:pytest -q")
        );
        assert_eq!(
            classify_read_only_program_command(Some(&ws), "eslint --fix src").unwrap(),
            None
        );
        assert_eq!(
            classify_read_only_program_command(Some(&ws), "gofmt -w main.go").unwrap(),
            None
        );
        assert_eq!(
            classify_read_only_program_command(Some(&ws), "find . -exec rm {} +").unwrap(),
            None
        );
        assert!(classify_read_only_program_command(Some(&ws), "find . -exec rm {} ;").is_err());
        assert_eq!(
            classify_read_only_program_command(Some(&ws), "sort -o out.txt composer.json").unwrap(),
            None
        );
        assert_eq!(
            classify_read_only_program_command(Some(&ws), "rg --pre=./evil.sh echo src").unwrap(),
            None
        );
        assert_eq!(
            classify_read_only_program_command(Some(&ws), "git rm -r src").unwrap(),
            None
        );
        assert_eq!(
            classify_read_only_program_command(Some(&ws), "git diff --output=out.txt").unwrap(),
            None
        );
        assert_eq!(
            classify_read_only_program_command(Some(&ws), "rg 'a|b' src")
                .unwrap()
                .as_deref(),
            Some("readonly:program:rg a|b src")
        );

        let store = crate::store::SessionStore::open(&root.join("state")).unwrap();
        let project_id = store.register(&ws).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let (warm_dir, _) = seed_sealed_capsule(
            &store,
            &project_id,
            "cap-lint",
            "programs/lint.py",
            lint_bytes,
            true,
            &["gate1", "gate2", "gate3"],
        );

        // Cache hit on repeated sealed capsule + CommandAnalyzer lookups without changes.
        assert_eq!(
            classify_read_only_program_command_in_store(
                Some(&ws),
                Some(&store),
                "python3 programs/lint.py",
            )
            .unwrap()
            .as_deref(),
            Some("readonly:program:python3 programs/lint.py")
        );
        assert_eq!(
            classify_read_only_program_command_in_store(Some(&ws), Some(&store), "composer test",)
                .unwrap()
                .as_deref(),
            Some("readonly:program:composer test")
        );
        let (builds_1, reads_1, evals_1) = read_only_cache_counters(&ws);
        assert_eq!(
            classify_read_only_program_command_in_store(
                Some(&ws),
                Some(&store),
                "python3 programs/lint.py",
            )
            .unwrap()
            .as_deref(),
            Some("readonly:program:python3 programs/lint.py")
        );
        assert_eq!(
            classify_read_only_program_command_in_store(Some(&ws), Some(&store), "composer test",)
                .unwrap()
                .as_deref(),
            Some("readonly:program:composer test")
        );
        let (builds_2, reads_2, evals_2) = read_only_cache_counters(&ws);
        assert_eq!(builds_2, builds_1, "CommandAnalyzer should be cached");
        assert_eq!(reads_2, reads_1, "sealed package load should be cached");
        assert_eq!(
            evals_2, evals_1,
            "sealed capsule verification should be cached"
        );

        // Marking capsule stale immediately revokes approval.
        {
            let db = store.connection().unwrap();
            crate::skillcache::db::set_capsule_status(&db, "cap-lint", "stale", Some("stale"))
                .unwrap();
        }
        assert_eq!(
            classify_read_only_program_command_in_store(
                Some(&ws),
                Some(&store),
                "python3 programs/lint.py",
            )
            .unwrap(),
            None
        );

        // Changing composer.json invalidates CommandAnalyzer cache cleanly.
        std::fs::write(
            ws.join("composer.json"),
            r#"{"scripts":{"test":"rm -rf build"}}"#,
        )
        .unwrap();
        assert_eq!(
            classify_read_only_program_command(Some(&ws), "composer test").unwrap(),
            None
        );
        let (builds_4, _, _) = read_only_cache_counters(&ws);
        assert!(builds_4 > builds_2);
        let _ = infra_skillcache::package::remove(&warm_dir);
    }
}
