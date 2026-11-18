use crate::{
    HarnessError, Result,
    process::Limits,
    provider::{self, Launch},
    store::SessionStore,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
const MAX: usize = 60;
fn clean(v: &Value, n: usize) -> String {
    v.as_str()
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(n)
        .collect()
}
fn entry(cycle: u64, round: u64, actor: &str, action: &str, note: &Value) -> Value {
    json!({"cycle":cycle,"round":round,"actor":actor,"action":action,"note":clean(note,600)})
}
fn items(state: &Value) -> Vec<Value> {
    state["items"].as_array().cloned().unwrap_or_default()
}
fn next(items: &[Value]) -> u64 {
    items
        .iter()
        .filter_map(|v| v["id"].as_u64())
        .max()
        .unwrap_or(0)
        + 1
}
fn new_item(raw: &Value, id: u64, actor: &str, kind: &str, cycle: u64, round: u64) -> Value {
    let claim = clean(&raw["claim"], 600);
    json!({"id":id,"kind":kind,"origin":actor,"severity":if ["high","medium","low"].contains(&raw["severity"].as_str().unwrap_or("")){raw["severity"].clone()}else{"unrated".into()},"file":clean(&raw["file"],300),"line":raw["line"].as_u64().filter(|n|*n<=1_000_000_000).unwrap_or(0),"claim":if claim.is_empty(){"No description was returned.".into()}else{claim},"evidence":clean(&raw["evidence"],600),"status":if kind=="objection"{"open"}else{"proposed"},"history":[entry(cycle,round,actor,"raised",&Value::Null)]})
}
fn history(item: &mut Value, record: Value) {
    if let Some(a) = item["history"].as_array_mut() {
        a.push(record);
    }
}
pub fn validate(value: &Value, workflow: &str, provider: &str) -> Result<Option<Value>> {
    if value.is_null() {
        return Ok(None);
    }
    if !["native", "review"].contains(&workflow)
        || !crate::store::exact(
            value,
            &["challenger"],
            &["rounds", "challenger_model", "challenger_thinking_effort"],
        )
    {
        return Err(HarnessError::invalid("Invalid Clash settings."));
    }
    let challenger = value["challenger"].as_str().unwrap_or("");
    let rounds = value["rounds"].as_u64().unwrap_or(2);
    if !["claude", "codex", "cursor"].contains(&challenger)
        || challenger == provider
        || !(1..=3).contains(&rounds)
        || value["challenger_thinking_effort"] == "ultracode"
    {
        return Err(HarnessError::invalid(
            "Choose a different challenger and one to three rounds.",
        ));
    }
    Ok(Some(
        json!({"challenger":challenger,"rounds":rounds,"challenger_model":value["challenger_model"],"challenger_thinking_effort":value["challenger_thinking_effort"]}),
    ))
}
pub fn new_state(session: &Value, stage: &str, previous: Option<&Value>) -> Value {
    let settings = &session["clash"];
    let same = previous.is_some_and(|p| p["stage"] == stage);
    let old_items = if same {
        previous
            .map(items)
            .unwrap_or_default()
            .into_iter()
            .take(MAX)
            .collect::<Vec<_>>()
    } else {
        vec![]
    };
    let old_turns = previous
        .and_then(|p| p["turns"].as_array())
        .map(|a| {
            a.iter()
                .rev()
                .take(MAX)
                .cloned()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let challenger = previous
        .filter(|p| p["challenger"]["provider"] == settings["challenger"])
        .map(|p| p["challenger_session_id"].clone())
        .unwrap_or(Value::Null);
    json!({"kind":"clash_state","stage":stage,"status":"running","outcome":null,"cycle":previous.and_then(|p|p["cycle"].as_u64()).unwrap_or(0)+1,"round":0,"rounds":settings["rounds"],"protagonist":{"provider":session["provider"],"role":if stage=="implement"{"implementer"}else{"reviewer"},"model":session["model"],"thinking_effort":session["thinking_effort"]},"challenger":{"provider":settings["challenger"],"role":"challenger","model":settings["challenger_model"],"thinking_effort":settings["challenger_thinking_effort"]},"challenger_session_id":challenger,"task":if same{previous.map(|p|p["task"].clone()).unwrap_or(json!(""))}else{json!("")},"followup":"","items":old_items,"turns":old_turns,"tokens":0,"cost_usd":0.0,"summary":"","verdict":null,"verdict_note":"","message":"","report":""})
}
pub fn unresolved(state: &Value) -> Vec<Value> {
    items(state)
        .into_iter()
        .filter(|i| {
            if state["stage"] == "implement" {
                i["status"] == "open" && i["severity"] != "low"
            } else {
                ["proposed", "disputed", "unverified", "defended"]
                    .contains(&i["status"].as_str().unwrap_or(""))
            }
        })
        .collect()
}
pub fn converged(state: &Value) -> bool {
    state["verdict"] == "accept" && unresolved(state).is_empty()
}
fn verdict(state: &mut Value, payload: &Value) {
    state["verdict"] = if ["accept", "reject"].contains(&payload["verdict"].as_str().unwrap_or(""))
    {
        payload["verdict"].clone()
    } else {
        Value::Null
    };
    state["verdict_note"]=if state["verdict"].is_null(){"The challenger returned no valid verdict; the cycle continues as rejected."}else if state["verdict"]=="accept"&&!unresolved(state).is_empty(){"The challenger said accept while unresolved ledger items remain; the ledger does not converge."}else{""}.into();
}
pub fn apply_challenge(state: &mut Value, payload: &Value, round: u64) {
    let cycle = state["cycle"].as_u64().unwrap_or(1);
    let mut ledger = items(state);
    let mut listed = BTreeMap::new();
    let mut fresh = Vec::new();
    let mut id = next(&ledger);
    let existing = ledger
        .iter()
        .filter(|i| i["kind"] == "objection")
        .filter_map(|i| i["id"].as_u64())
        .collect::<BTreeSet<_>>();
    let resolved = payload["resolved"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .collect::<BTreeSet<_>>();
    for raw in payload["objections"]
        .as_array()
        .into_iter()
        .flatten()
        .take(MAX)
        .filter(|v| v.is_object())
    {
        if let Some(id) = raw["id"]
            .as_u64()
            .filter(|id| existing.contains(id) && !listed.contains_key(id))
        {
            listed.insert(id, raw.clone());
        } else if ledger.len() + fresh.len() < MAX {
            fresh.push(new_item(raw, id, "challenger", "objection", cycle, round));
            id += 1;
        }
    }
    for item in ledger.iter_mut().filter(|i| i["kind"] == "objection") {
        let id = item["id"].as_u64().unwrap_or(0);
        let old = item["status"].as_str().unwrap_or("").to_string();
        if resolved.contains(&id) {
            if old != "resolved" {
                item["status"] = "resolved".into();
                history(
                    item,
                    entry(cycle, round, "challenger", "resolved", &Value::Null),
                );
            }
        } else if let Some(raw) = listed.get(&id) {
            for key in ["claim", "evidence"] {
                let text = clean(&raw[key], 600);
                if !text.is_empty() {
                    item[key] = text.into();
                }
            }
            if ["high", "medium", "low"].contains(&raw["severity"].as_str().unwrap_or("")) {
                item["severity"] = raw["severity"].clone();
            }
            let file = clean(&raw["file"], 300);
            if !file.is_empty() {
                item["file"] = file.into();
            }
            if let Some(line) = raw["line"].as_u64() {
                item["line"] = json!(line.min(1_000_000_000));
            }
            item["status"] = "open".into();
            history(
                item,
                entry(
                    cycle,
                    round,
                    "challenger",
                    if ["open", "fixed", "rebutted", "deferred"].contains(&old.as_str()) {
                        "maintained"
                    } else {
                        "reopened"
                    },
                    &Value::Null,
                ),
            );
        } else if ["open", "fixed", "rebutted", "deferred"].contains(&old.as_str()) {
            item["status"] = match old.as_str() {
                "fixed" => "resolved",
                "rebutted" => "withdrawn",
                "deferred" => "deferred",
                _ => "withdrawn",
            }
            .into();
            history(
                item,
                entry(cycle, round, "challenger", "not maintained", &Value::Null),
            );
        }
    }
    ledger.extend(fresh);
    state["items"] = json!(ledger);
    verdict(state, payload);
}
pub fn apply_response(state: &mut Value, payload: &Value, round: u64) {
    let cycle = state["cycle"].as_u64().unwrap_or(1);
    let mut ledger = items(state);
    let mut seen = BTreeSet::new();
    for raw in payload["responses"]
        .as_array()
        .into_iter()
        .flatten()
        .take(MAX * 2)
    {
        let Some(id) = raw["id"].as_u64() else {
            continue;
        };
        let action = raw["action"].as_str().unwrap_or("");
        if !["fixed", "rebutted", "deferred"].contains(&action) || seen.contains(&id) {
            continue;
        }
        if let Some(item) = ledger
            .iter_mut()
            .find(|i| i["id"] == id && i["kind"] == "objection" && i["status"] == "open")
        {
            seen.insert(id);
            item["status"] = action.into();
            history(
                item,
                entry(cycle, round, "implementer", action, &raw["note"]),
            );
        }
    }
    for item in ledger.iter_mut().filter(|i| {
        i["kind"] == "objection"
            && i["status"] == "open"
            && !seen.contains(&i["id"].as_u64().unwrap_or(0))
    }) {
        history(
            item,
            entry(cycle, round, "implementer", "no response", &Value::Null),
        );
    }
    state["items"] = json!(ledger);
}
pub fn apply_review(state: &mut Value, actor: &str, payload: &Value, round: u64) {
    let cycle = state["cycle"].as_u64().unwrap_or(1);
    let other = if actor == "reviewer" {
        "challenger"
    } else {
        "reviewer"
    };
    let mut ledger = items(state);
    let mut fresh = Vec::new();
    let mut id = next(&ledger);
    let mut assessed = BTreeSet::new();
    let mut answered = BTreeSet::new();
    for key in ["findings", "additional"] {
        for raw in payload[key]
            .as_array()
            .into_iter()
            .flatten()
            .take(MAX)
            .filter(|v| v.is_object())
        {
            if ledger.len() + fresh.len() < MAX {
                fresh.push(new_item(raw, id, actor, "finding", cycle, round));
                id += 1;
            }
        }
    }
    for raw in payload["assessments"]
        .as_array()
        .into_iter()
        .flatten()
        .take(MAX * 2)
    {
        let Some(id) = raw["id"].as_u64() else {
            continue;
        };
        let stance = raw["stance"].as_str().unwrap_or("");
        if assessed.contains(&id) || !["agree", "dispute", "unverified"].contains(&stance) {
            continue;
        }
        if let Some(item) = ledger.iter_mut().find(|i| {
            i["kind"] == "finding"
                && i["id"] == id
                && i["origin"] == other
                && ["proposed", "defended"].contains(&i["status"].as_str().unwrap_or(""))
        }) {
            assessed.insert(id);
            item["status"] = match stance {
                "agree" => "confirmed",
                "dispute" => "disputed",
                _ => "unverified",
            }
            .into();
            history(item, entry(cycle, round, actor, stance, &raw["evidence"]));
        }
    }
    for raw in payload["responses"]
        .as_array()
        .into_iter()
        .flatten()
        .take(MAX * 2)
    {
        let Some(id) = raw["id"].as_u64() else {
            continue;
        };
        let action = raw["action"].as_str().unwrap_or("");
        if answered.contains(&id) || !["defend", "withdraw"].contains(&action) {
            continue;
        }
        if let Some(item) = ledger.iter_mut().find(|i| {
            i["kind"] == "finding"
                && i["id"] == id
                && i["origin"] == actor
                && ["disputed", "unverified"].contains(&i["status"].as_str().unwrap_or(""))
        }) {
            answered.insert(id);
            item["status"] = if action == "defend" {
                "defended"
            } else {
                "withdrawn"
            }
            .into();
            history(item, entry(cycle, round, actor, action, &raw["note"]));
        }
    }
    for item in ledger.iter_mut().filter(|i| i["kind"] == "finding") {
        let id = item["id"].as_u64().unwrap_or(0);
        let status = item["status"].as_str().unwrap_or("");
        if item["origin"] == other
            && ["proposed", "defended"].contains(&status)
            && !assessed.contains(&id)
        {
            history(
                item,
                entry(cycle, round, actor, "not assessed", &Value::Null),
            );
        } else if item["origin"] == actor
            && ["disputed", "unverified"].contains(&status)
            && !answered.contains(&id)
        {
            history(
                item,
                entry(cycle, round, actor, "no response", &Value::Null),
            );
        }
    }
    ledger.extend(fresh);
    state["items"] = json!(ledger);
    if actor == "challenger" {
        verdict(state, payload);
    }
}
pub fn extract(text: &str, required: &[&str]) -> Option<Value> {
    for (start, _) in text
        .match_indices('{')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let mut stream = serde_json::Deserializer::from_str(&text[start..]).into_iter::<Value>();
        if let Some(Ok(v)) = stream.next()
            && v.is_object()
            && required.iter().any(|k| v.get(*k).is_some())
        {
            return Some(v);
        }
    }
    None
}
pub fn render(state: &Value) -> String {
    let mut out = format!(
        "# Clash — {} stage\n\nCycle {}, round {} of {}; outcome: {}; challenger verdict: {}.\n\n",
        state["stage"].as_str().unwrap_or(""),
        state["cycle"],
        state["round"],
        state["rounds"],
        state["outcome"].as_str().unwrap_or("running"),
        state["verdict"].as_str().unwrap_or("none")
    );
    for item in items(state) {
        out.push_str(&format!(
            "- #{} [{}] {} — {}:{} ({}): {}\n  evidence: {}\n",
            item["id"],
            item["severity"].as_str().unwrap_or(""),
            item["status"].as_str().unwrap_or(""),
            item["file"].as_str().unwrap_or("-"),
            item["line"],
            item["origin"].as_str().unwrap_or(""),
            item["claim"].as_str().unwrap_or(""),
            item["evidence"].as_str().unwrap_or("")
        ));
        for e in item["history"].as_array().into_iter().flatten().skip(1) {
            out.push_str(&format!(
                "  cycle {} round {} · {}: {} — {}\n",
                e["cycle"],
                e["round"],
                e["actor"].as_str().unwrap_or(""),
                e["action"].as_str().unwrap_or(""),
                e["note"].as_str().unwrap_or("")
            ));
        }
    }
    out.push_str("\n## Turns\n");
    for turn in state["turns"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "- cycle {} round {} · {} · {}: {}; cost {}; {}s — {}\n",
            turn["cycle"],
            turn["round"],
            turn["role"].as_str().unwrap_or(""),
            turn["provider"].as_str().unwrap_or(""),
            if turn["ok"] == true { "ok" } else { "failed" },
            turn["cost_usd"],
            turn["seconds"],
            turn["summary"].as_str().unwrap_or("")
        ));
    }
    if out.chars().count() > 32000 {
        out = out.chars().take(31970).collect();
        out.push_str("\n…[report truncated]\n");
    }
    out
}
fn round_usd(value: f64) -> f64 {
    ((value * 1_000_000_000.0).round()) / 1_000_000_000.0
}
struct CancelRelay(tokio::task::JoinHandle<()>);
impl Drop for CancelRelay {
    fn drop(&mut self) {
        self.0.abort();
    }
}
struct CycleBudget {
    max_seconds: Option<u64>,
    deadline: Option<Instant>,
    max_tokens: Option<u64>,
    tokens: Arc<AtomicU64>,
    max_usd: Option<f64>,
    usd: f64,
}
impl CycleBudget {
    fn new(session: &Value) -> Self {
        let max_seconds = session["budgets"]["seconds"].as_u64();
        let deadline = max_seconds.map(|s| Instant::now() + Duration::from_secs(s));
        Self {
            max_seconds,
            deadline,
            max_tokens: session["budgets"]["tokens"].as_u64(),
            tokens: Arc::new(AtomicU64::new(0)),
            max_usd: session["budgets"]["usd"].as_f64(),
            usd: 0.0,
        }
    }
    fn pre_turn_violation(&self, provider: &str) -> Option<&'static str> {
        if let Some(deadline) = self.deadline
            && deadline.saturating_duration_since(Instant::now()).is_zero()
        {
            return Some("Clash reached its time budget; cycle incomplete.");
        }
        if self
            .max_tokens
            .is_some_and(|limit| self.tokens.load(Ordering::Acquire) >= limit)
        {
            return Some("Clash reached its token budget; cycle incomplete.");
        }
        if let Some(limit) = self.max_usd
            && (self.usd >= limit || (provider == "claude" && round_usd(limit - self.usd) < 0.01))
        {
            return Some("Clash reached its USD budget; cycle incomplete.");
        }
        None
    }
    fn turn_limits(&self) -> Option<(Limits, Option<Instant>)> {
        let Some(deadline) = self.deadline else {
            return Some((
                Limits {
                    seconds: None,
                    ..Limits::default()
                },
                None,
            ));
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let max = self.max_seconds.unwrap_or(u64::MAX).max(1);
        let secs = (remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0)).clamp(1, max);
        Some((
            Limits {
                seconds: Some(secs),
                ..Limits::default()
            },
            Some(deadline),
        ))
    }
    fn remaining_usd_for(&self, provider: &str) -> Option<f64> {
        if provider != "claude" {
            return None;
        }
        self.max_usd
            .map(|limit| round_usd((limit - self.usd).max(0.0)))
    }
    fn record_cost(&mut self, cost: Option<f64>) {
        if let Some(cost) = cost.filter(|c| c.is_finite() && *c >= 0.0) {
            self.usd = round_usd(self.usd + cost);
        }
    }
    fn post_turn_violation(&self, output: &provider::WorkerResult) -> Option<&'static str> {
        match output.limit_reached.as_deref() {
            Some("time") => Some("Clash reached its time budget; cycle incomplete."),
            Some("token") => Some("Clash reached its token budget; cycle incomplete."),
            Some("USD") => Some("Clash reached its USD budget; cycle incomplete."),
            _ if self.max_usd.is_some_and(|limit| self.usd > limit) => {
                Some("Clash reached its USD budget; cycle incomplete.")
            }
            _ => None,
        }
    }
}
pub struct Cycle {
    pub store: SessionStore,
    pub session: Value,
    pub launch: Launch,
    pub executables: BTreeMap<String, std::path::PathBuf>,
    pub context: String,
    pub adapter: Arc<dyn provider::LaunchAdapter>,
}
impl Cycle {
    fn persist(&self, state: &Value) -> Result<()> {
        self.store.patch(
            self.session["id"].as_str().unwrap_or(""),
            &json!({"clash_result":state}),
        )?;
        self.store
            .event(self.session["id"].as_str().unwrap_or(""), state)?;
        Ok(())
    }
    fn turn_provider(&self, state: &Value, role: &str) -> Result<String> {
        let key = if role == "challenger" {
            "challenger"
        } else {
            "protagonist"
        };
        state[key]["provider"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| HarnessError::invalid("Missing Clash provider."))
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "Turn execution carries shared cycle budget state, cancellation, and event emitter."
    )]
    async fn turn(
        &mut self,
        state: &mut Value,
        role: &str,
        round: u64,
        prompt: String,
        budget: &mut CycleBudget,
        cancel: Arc<AtomicBool>,
        emit: Arc<dyn Fn(Value) -> Result<()> + Send + Sync>,
    ) -> Result<provider::WorkerResult> {
        let challenger = role == "challenger";
        let participant = state[if challenger {
            "challenger"
        } else {
            "protagonist"
        }]
        .clone();
        let provider = participant["provider"]
            .as_str()
            .ok_or_else(|| HarnessError::invalid("Missing Clash provider."))?;
        let Some((limits, deadline)) = budget.turn_limits() else {
            return Ok(provider::WorkerResult {
                text: String::new(),
                ok: false,
                cost_usd: None,
                session_id: None,
                duration_seconds: 0.0,
                error: Some("Clash reached its time budget; cycle incomplete.".into()),
                limit_reached: Some("time".into()),
            });
        };
        let mut launch = self.launch.clone();
        launch.provider = provider.into();
        launch.executable = self
            .executables
            .get(provider)
            .cloned()
            .ok_or_else(|| HarnessError::unavailable("The participant CLI is unavailable."))?;
        launch.agents = false;
        launch.count = 1;
        launch.mode = if !challenger && state["stage"] == "implement" {
            "edit"
        } else {
            "plan"
        }
        .into();
        if launch.mode != "edit" {
            launch.approvals = None;
        }
        launch.model = participant["model"].as_str().map(String::from);
        launch.effort = participant["thinking_effort"].as_str().map(String::from);
        launch.usd = budget.remaining_usd_for(provider);
        launch.session = if challenger {
            state["challenger_session_id"].as_str().map(String::from)
        } else {
            self.session["native_session_id"].as_str().map(String::from)
        };
        launch.prompt = format!("{prompt}\n\n{}", self.context);
        let previous = launch
            .session
            .as_deref()
            .and_then(|id| self.session["cost_totals"][id].as_f64());
        let turn_tokens = Arc::new(AtomicU64::new(0));
        let token_limit_hit = Arc::new(AtomicBool::new(false));
        let turn_cancel = Arc::new(AtomicBool::new(cancel.load(Ordering::Acquire)));
        let forwarding = tokio::runtime::Handle::try_current().ok().map(|handle| {
            let human = cancel.clone();
            let turn_control = turn_cancel.clone();
            CancelRelay(handle.spawn(async move {
                loop {
                    if human.load(Ordering::Acquire) {
                        turn_control.store(true, Ordering::Release);
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            }))
        });
        let session_id = self.session["id"].as_str().unwrap_or("").to_owned();
        let initial_totals = if self.session["cost_totals"].is_object() {
            self.session["cost_totals"].clone()
        } else {
            json!({})
        };
        let cost_totals = Arc::new(Mutex::new(initial_totals));
        let active_native = Arc::new(Mutex::new(launch.session.clone()));
        let last_turn_cost = Arc::new(Mutex::new(None::<f64>));
        let store = self.store.clone();
        let max_tokens = budget.max_tokens;
        let cycle_tokens = budget.tokens.clone();
        let stream_turn_tokens = turn_tokens.clone();
        let stream_token_hit = token_limit_hit.clone();
        let stream_cancel = turn_cancel.clone();
        let stream_totals = cost_totals.clone();
        let stream_native = active_native.clone();
        let stream_cost = last_turn_cost.clone();
        let stream_sid = session_id.clone();
        let turn_started = Instant::now();
        let outcome = provider::run_monitored_until(
            &launch,
            limits,
            turn_cancel,
            previous,
            Some(self.store.runner_lock()),
            Some(self.adapter.as_ref()),
            &BTreeMap::new(),
            None,
            deadline,
            move |event| {
                if event["kind"] == "session"
                    && let Some(native) = event["native_session_id"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                {
                    let mut current = stream_native
                        .lock()
                        .map_err(|_| HarnessError::unavailable("Clash session lock failed."))?;
                    if current.as_deref().is_some_and(|old| old != native) {
                        return Err(HarnessError::conflict(
                            "The participant changed native session identity.",
                        ));
                    }
                    *current = Some(native.to_owned());
                }
                if event["kind"] == "usage" {
                    if let Some(n) = provider::total_tokens(&event) {
                        stream_turn_tokens.fetch_add(n, Ordering::AcqRel);
                        let used = cycle_tokens
                            .fetch_add(n, Ordering::AcqRel)
                            .saturating_add(n);
                        if max_tokens.is_some_and(|limit| used >= limit) {
                            stream_token_hit.store(true, Ordering::Release);
                            stream_cancel.store(true, Ordering::Release);
                        }
                    }
                    if let Some(cost) = event["cost_usd"]
                        .as_f64()
                        .filter(|c| c.is_finite() && *c >= 0.0)
                        && let Ok(mut slot) = stream_cost.lock()
                    {
                        *slot = Some(cost);
                    }
                    if let Some(total) = event["cost_session_total"]
                        .as_f64()
                        .filter(|cost| cost.is_finite() && *cost >= 0.0)
                    {
                        let native_id = stream_native
                            .lock()
                            .ok()
                            .and_then(|guard| guard.clone())
                            .or_else(|| {
                                event["native_session_id"]
                                    .as_str()
                                    .filter(|id| !id.is_empty())
                                    .map(str::to_owned)
                            });
                        if let Some(native_id) = native_id
                            && let Ok(mut totals) = stream_totals.lock()
                        {
                            if !totals.is_object() {
                                *totals = json!({});
                            }
                            totals[&native_id] = json!(total);
                            if !stream_sid.is_empty() {
                                store
                                    .patch(&stream_sid, &json!({"cost_totals": totals.clone()}))?;
                            }
                        }
                    }
                }
                emit(event)
            },
        )
        .await;
        drop(forwarding);
        if let Ok(totals) = cost_totals.lock() {
            self.session["cost_totals"] = totals.clone();
        }
        let observed_sid = active_native.lock().ok().and_then(|guard| guard.clone());
        let recorded_cost = last_turn_cost.lock().ok().and_then(|guard| *guard);
        let mut output = match outcome {
            Ok(mut output) => {
                if token_limit_hit.load(Ordering::Acquire) && !cancel.load(Ordering::Acquire) {
                    output.ok = false;
                    output.limit_reached = Some("token".into());
                    output.error = Some("Clash reached its token budget; cycle incomplete.".into());
                }
                output
            }
            Err(error) if ["native-timeout", "native-time-budget"].contains(&error.code) => {
                provider::WorkerResult {
                    text: String::new(),
                    ok: false,
                    cost_usd: recorded_cost,
                    session_id: observed_sid,
                    duration_seconds: turn_started.elapsed().as_secs_f64(),
                    error: Some("Clash reached its time budget; cycle incomplete.".into()),
                    limit_reached: Some("time".into()),
                }
            }
            Err(error)
                if error.code == "native-cancelled"
                    && token_limit_hit.load(Ordering::Acquire)
                    && !cancel.load(Ordering::Acquire) =>
            {
                provider::WorkerResult {
                    text: String::new(),
                    ok: false,
                    cost_usd: recorded_cost,
                    session_id: observed_sid,
                    duration_seconds: turn_started.elapsed().as_secs_f64(),
                    error: Some("Clash reached its token budget; cycle incomplete.".into()),
                    limit_reached: Some("token".into()),
                }
            }
            Err(error) => return Err(error),
        };
        if output.cost_usd.is_none() {
            output.cost_usd = recorded_cost;
        }
        if let Some(sid) = &output.session_id {
            if launch.session.as_ref().is_some_and(|old| old != sid) {
                return Err(HarnessError::conflict(
                    "The participant changed native session identity.",
                ));
            }
            if challenger {
                state["challenger_session_id"] = Value::String(sid.clone());
            } else {
                self.session["native_session_id"] = Value::String(sid.clone());
                if !session_id.is_empty() {
                    self.store
                        .patch(&session_id, &json!({"native_session_id": sid}))?;
                }
            }
        }
        let this_turn_tokens = turn_tokens.load(Ordering::Acquire);
        budget.record_cost(output.cost_usd);
        state["tokens"] = json!(budget.tokens.load(Ordering::Acquire));
        state["cost_usd"] = json!(budget.usd);
        let turn = json!({
            "cycle": state["cycle"],
            "round": round,
            "role": role,
            "provider": provider,
            "ok": output.ok,
            "summary": clean(&json!(output.text), 600),
            "cost_usd": output.cost_usd,
            "tokens": this_turn_tokens,
            "seconds": output.duration_seconds,
            "error": output.error
        });
        state["turns"]
            .as_array_mut()
            .ok_or_else(|| HarnessError::invalid("Invalid Clash turns."))?
            .push(turn);
        let turns = state["turns"].as_array_mut().unwrap();
        if turns.len() > MAX {
            turns.drain(..turns.len() - MAX);
        }
        self.persist(state)?;
        Ok(output)
    }
    pub async fn drive(
        &mut self,
        prompt: &str,
        stage: &str,
        continued: bool,
        cancel: Arc<AtomicBool>,
        emit: Arc<dyn Fn(Value) -> Result<()> + Send + Sync>,
    ) -> Result<Value> {
        let previous = if continued {
            self.session.get("clash_result")
        } else {
            None
        };
        let mut state = new_state(&self.session, stage, previous);
        let mut budget = CycleBudget::new(&self.session);
        if state["task"].as_str().unwrap_or("").is_empty() {
            state["task"] = prompt.chars().take(8000).collect::<String>().into();
        } else {
            state["followup"] = prompt.chars().take(8000).collect::<String>().into();
        }
        self.persist(&state)?;
        let role = if stage == "implement" {
            "implementer"
        } else {
            "reviewer"
        };
        let protagonist_provider = self.turn_provider(&state, role)?;
        if let Some(message) = budget.pre_turn_violation(&protagonist_provider) {
            return self.incomplete(state, message);
        }
        let header = format!(
            "Two-provider Clash, cycle {}. No additional agents. One human-reviewed cycle only. Task: {}\nFollow-up: {}\n",
            state["cycle"],
            state["task"].as_str().unwrap_or(""),
            state["followup"].as_str().unwrap_or("")
        );
        let opening = format!(
            "{header}Act as {role}. {} Existing ledger is reference data: {}. Return verified file/line evidence. {}",
            if stage == "implement" {
                "Implement the requested work completely and verify it."
            } else {
                "Review without editing any files."
            },
            state["items"],
            if stage == "review" {
                "End with JSON {findings:[...]}, each item severity,file,line,claim,evidence."
            } else {
                "Summarize changed files, verification and limitations."
            }
        );
        let result = self
            .turn(
                &mut state,
                role,
                0,
                opening,
                &mut budget,
                cancel.clone(),
                emit.clone(),
            )
            .await?;
        if let Some(message) = budget.post_turn_violation(&result) {
            return self.incomplete(state, message);
        }
        if !result.ok {
            return self.incomplete(state, "The opening participant turn did not complete.");
        }
        state["summary"] = clean(&json!(result.text), 4000).into();
        if stage == "review" {
            let Some(payload) = extract(&result.text, &["findings"]) else {
                return self.incomplete(state, "Reviewer A did not return structured findings.");
            };
            apply_review(&mut state, "reviewer", &payload, 0);
        }
        let challenger_provider = self.turn_provider(&state, "challenger")?;
        for round in 1..=state["rounds"].as_u64().unwrap_or(2) {
            if let Some(message) = budget.pre_turn_violation(&challenger_provider) {
                return self.incomplete(state, message);
            }
            state["round"] = round.into();
            let challenge = format!(
                "{header}Act as challenger. Independently inspect the workspace without editing. Verify every objection; no style preferences. Maintain prior IDs. Ledger: {}. Latest summary: {}. {}",
                state["items"],
                state["summary"],
                if stage == "implement" {
                    "Return JSON {verdict:accept|reject,objections:[...],resolved:[ids]}; accept only when no high/medium/unrated objection remains."
                } else {
                    "Return JSON {verdict:accept|reject,assessments:[{id,stance:agree|dispute|unverified,evidence}],responses:[{id,action:defend|withdraw,note}],additional:[...]}. Assess every other-side item; defend or withdraw own disputed items."
                }
            );
            let result = self
                .turn(
                    &mut state,
                    "challenger",
                    round,
                    challenge,
                    &mut budget,
                    cancel.clone(),
                    emit.clone(),
                )
                .await?;
            if let Some(message) = budget.post_turn_violation(&result) {
                return self.incomplete(state, message);
            }
            if !result.ok {
                return self.incomplete(state, "The challenger turn did not complete.");
            }
            let Some(payload) = extract(&result.text, &["verdict", "objections", "assessments"])
            else {
                return self
                    .incomplete(state, "The challenger did not return a structured verdict.");
            };
            if stage == "implement" {
                apply_challenge(&mut state, &payload, round);
            } else {
                apply_review(&mut state, "challenger", &payload, round);
            }
            self.persist(&state)?;
            if converged(&state) || round == state["rounds"].as_u64().unwrap_or(2) {
                break;
            }
            if let Some(message) = budget.pre_turn_violation(&protagonist_provider) {
                return self.incomplete(state, message);
            }
            let response = format!(
                "{header}Act as {role}. Respond to every open or disputed item; do not concede a wrong claim or dismiss a valid defect. Ledger: {}. {}",
                state["items"],
                if stage == "implement" {
                    "Fix and verify, rebut with evidence, or defer only outside scope. Return JSON {responses:[{id,action:fixed|rebutted|deferred,note}]}."
                } else {
                    "Do not edit files. Return JSON {assessments:[{id,stance:agree|dispute|unverified,evidence}],responses:[{id,action:defend|withdraw,note}]}."
                }
            );
            let result = self
                .turn(
                    &mut state,
                    role,
                    round,
                    response,
                    &mut budget,
                    cancel.clone(),
                    emit.clone(),
                )
                .await?;
            if let Some(message) = budget.post_turn_violation(&result) {
                return self.incomplete(state, message);
            }
            if !result.ok {
                return self.incomplete(state, "The protagonist response turn did not complete.");
            }
            let payload = extract(&result.text, &["responses", "assessments"]).unwrap_or(json!({}));
            if stage == "implement" {
                apply_response(&mut state, &payload, round);
            } else {
                apply_review(&mut state, "reviewer", &payload, round);
            }
            state["summary"] = clean(&json!(result.text), 4000).into();
            self.persist(&state)?;
            if converged(&state) {
                break;
            }
        }
        state["status"] = "finished".into();
        state["outcome"] = if converged(&state) {
            "converged"
        } else {
            "unresolved"
        }
        .into();
        state["message"] = format!(
            "Cycle {} finished in round {}: {}.",
            state["cycle"],
            state["round"],
            state["outcome"].as_str().unwrap_or("")
        )
        .into();
        state["report"] = render(&state).into();
        self.persist(&state)?;
        Ok(state)
    }
    fn incomplete(&self, mut state: Value, message: &str) -> Result<Value> {
        state["status"] = "finished".into();
        state["outcome"] = "incomplete".into();
        state["message"] = message.into();
        state["report"] = render(&state).into();
        self.persist(&state)?;
        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_usd_and_cycle_budget_enforce_time_tokens_and_usd() {
        assert_eq!(round_usd(0.05 - 0.04), 0.01);
        assert!(round_usd(0.05 - 0.045) < 0.01);

        let mut budget = CycleBudget::new(&json!({
            "budgets": {"seconds": 30, "tokens": 100, "usd": 0.05}
        }));
        assert_eq!(budget.pre_turn_violation("claude"), None);
        assert_eq!(budget.remaining_usd_for("claude"), Some(0.05));
        assert_eq!(budget.remaining_usd_for("codex"), None);

        budget.record_cost(Some(0.04));
        assert_eq!(budget.remaining_usd_for("claude"), Some(0.01));
        assert_eq!(budget.pre_turn_violation("claude"), None);

        // Below $0.01 remaining blocks subsequent Claude turns, while non-Claude stops only at >= limit.
        budget.record_cost(Some(0.005));
        assert_eq!(
            budget.pre_turn_violation("claude"),
            Some("Clash reached its USD budget; cycle incomplete.")
        );
        assert_eq!(budget.pre_turn_violation("codex"), None);

        budget.record_cost(Some(0.005));
        assert_eq!(
            budget.pre_turn_violation("codex"),
            Some("Clash reached its USD budget; cycle incomplete.")
        );

        let token_budget = CycleBudget::new(&json!({"budgets": {"tokens": 50}}));
        token_budget.tokens.store(50, Ordering::Release);
        assert_eq!(
            token_budget.pre_turn_violation("codex"),
            Some("Clash reached its token budget; cycle incomplete.")
        );

        let zero_time = CycleBudget::new(&json!({"budgets": {"seconds": 0}}));
        assert_eq!(
            zero_time.pre_turn_violation("claude"),
            Some("Clash reached its time budget; cycle incomplete.")
        );
    }
}
