use crate::{
    HarnessError, Result,
    process::Limits,
    provider::{self, Launch},
    store::SessionStore,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
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
/// Validates an optional deterministic `precheck` specification (`{argv, timeout_seconds?}`).
pub(crate) fn validate_precheck(value: Option<&Value>) -> Result<Value> {
    let Some(value) = value else {
        return Ok(Value::Null);
    };
    if value.is_null() {
        return Ok(Value::Null);
    }
    if !crate::store::exact(value, &["argv"], &["timeout_seconds"]) {
        return Err(HarnessError::invalid("Invalid precheck settings."));
    }
    let argv_raw = value["argv"]
        .as_array()
        .ok_or_else(|| HarnessError::invalid("Precheck argv must be a non-empty array."))?;
    if argv_raw.is_empty() || argv_raw.len() > 64 {
        return Err(HarnessError::invalid(
            "Precheck argv must contain 1–64 arguments.",
        ));
    }
    let mut argv = Vec::with_capacity(argv_raw.len());
    for arg in argv_raw {
        let s = arg
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 4096 && !s.contains('\0'))
            .ok_or_else(|| {
                HarnessError::invalid("Precheck argv entries must be non-empty strings.")
            })?;
        argv.push(s.to_owned());
    }
    let timeout = match value.get("timeout_seconds") {
        None | Some(Value::Null) => 30,
        Some(v) => v
            .as_u64()
            .filter(|n| (1..=120).contains(n))
            .ok_or_else(|| {
                HarnessError::invalid("Precheck timeout_seconds must be an integer from 1 to 120.")
            })?,
    };
    Ok(json!({"argv": argv, "timeout_seconds": timeout}))
}
pub fn validate(value: &Value, workflow: &str, provider: &str) -> Result<Option<Value>> {
    if value.is_null() {
        return Ok(None);
    }
    if !["native", "review"].contains(&workflow)
        || !crate::store::exact(
            value,
            &["challenger"],
            &[
                "rounds",
                "challenger_model",
                "challenger_thinking_effort",
                "precheck",
            ],
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
    let precheck = validate_precheck(value.get("precheck"))?;
    let mut out = json!({
        "challenger": challenger,
        "rounds": rounds,
        "challenger_model": value["challenger_model"],
        "challenger_thinking_effort": value["challenger_thinking_effort"],
    });
    if !precheck.is_null() {
        out["precheck"] = precheck;
    }
    Ok(Some(out))
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
    json!({"kind":"clash_state","stage":stage,"status":"running","outcome":null,"cycle":previous.and_then(|p|p["cycle"].as_u64()).unwrap_or(0)+1,"round":0,"rounds":settings["rounds"],"protagonist":{"provider":session["provider"],"role":if stage=="implement"{"implementer"}else{"reviewer"},"model":session["model"],"thinking_effort":session["thinking_effort"]},"challenger":{"provider":settings["challenger"],"role":"challenger","model":settings["challenger_model"],"thinking_effort":settings["challenger_thinking_effort"]},"challenger_session_id":challenger,"task":if same{previous.map(|p|p["task"].clone()).unwrap_or(json!(""))}else{json!("")},"followup":"","items":old_items,"turns":old_turns,"tokens":0,"cost_usd":0.0,"usage_components":{"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_write_5m_input_tokens":0,"cache_write_1h_input_tokens":0,"model_calls":0},"summary":"","verdict":null,"verdict_note":"","message":"","report":""})
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
/// Strictly parses structured Clash JSON from a whole object or fenced ```json block.
pub fn extract(text: &str, required: &[&str]) -> Option<Value> {
    parse_strict_payload(text, required).ok()
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
        let turn_usage = Arc::new(Mutex::new(json!({
            "input_tokens": 0u64,
            "output_tokens": 0u64,
            "cache_read_input_tokens": 0u64,
            "cache_write_5m_input_tokens": 0u64,
            "cache_write_1h_input_tokens": 0u64,
            "model_calls": 0u64,
            "source": provider,
        })));
        let stream_usage = turn_usage.clone();
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
                    if let Ok(mut slot) = stream_usage.lock() {
                        observe_usage_event(&mut slot, &event);
                    }
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
        let usage_snapshot = turn_usage
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_else(|_| json!({}));
        budget.record_cost(output.cost_usd);
        state["tokens"] = json!(budget.tokens.load(Ordering::Acquire));
        state["cost_usd"] = json!(budget.usd);
        merge_usage_components(&mut state["usage_components"], &usage_snapshot);
        let turn = json!({
            "cycle": state["cycle"],
            "round": round,
            "role": role,
            "provider": provider,
            "ok": output.ok,
            "summary": clean(&json!(output.text), 600),
            "cost_usd": output.cost_usd,
            "tokens": this_turn_tokens,
            "usage_components": usage_snapshot,
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
        let model = self.session["model"].as_str();
        record_workflow_health(
            &self.store,
            "clash",
            provider,
            model,
            &session_id,
            if output.ok { "completed" } else { "failed" },
            false,
            !output.ok,
            output.cost_usd,
            output.duration_seconds,
        );
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
        let challenger_provider = self.turn_provider(&state, "challenger")?;
        let pre_warnings = apply_prelaunch_health_policy(
            &self.store,
            &mut state,
            &protagonist_provider,
            &challenger_provider,
        );
        if !pre_warnings.is_empty() {
            let _ = emit(json!({
                "kind": "clash_health",
                "rounds": state["rounds"],
                "warnings": pre_warnings,
            }));
        }
        let mut header = format!(
            "Two-provider Clash, cycle {}. No additional agents. One human-reviewed cycle only. Task: {}\nFollow-up: {}\n",
            state["cycle"],
            state["task"].as_str().unwrap_or(""),
            state["followup"].as_str().unwrap_or("")
        );
        if let Some((pre_summary, pre_record)) =
            run_clash_precheck(&self.store, &self.session).await
        {
            state["deterministic_precheck"] = pre_record;
            self.persist(&state)?;
            if !pre_summary.is_empty() {
                header.push_str(&format!(
                    "Deterministic pre-stage check output:\n{pre_summary}\n"
                ));
            }
        }
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
        let (result, opening_payload) = if stage == "review" {
            self.complete_structured_turn(
                &mut state,
                role,
                0,
                opening,
                &["findings"],
                "{\"findings\":[{\"severity\":\"high|medium|low\",\"file\":\"...\",\"line\":1,\"claim\":\"...\",\"evidence\":\"...\"}]}",
                &mut budget,
                cancel.clone(),
                emit.clone(),
            )
            .await?
        } else {
            let res = self
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
            (res, Ok(json!({})))
        };
        if let Some(message) = budget.post_turn_violation(&result) {
            return self.incomplete(state, message);
        }
        if !result.ok {
            return self.incomplete(state, "The opening participant turn did not complete.");
        }
        state["summary"] = clean(&json!(result.text), 4000).into();
        if stage == "review" {
            let payload = match opening_payload {
                Ok(valid) => valid,
                Err(reason) => {
                    record_schema_error(&mut state, role, 0, &reason);
                    record_workflow_health(
                        &self.store,
                        "clash",
                        &protagonist_provider,
                        self.session["model"].as_str(),
                        self.session["id"].as_str().unwrap_or(""),
                        "schema_error",
                        true,
                        true,
                        None,
                        0.0,
                    );
                    let warnings = workflow_health_warnings(
                        &self.store,
                        "clash",
                        &protagonist_provider,
                        self.session["model"].as_str(),
                    );
                    merge_health_warnings(&mut state, &warnings);
                    self.persist(&state)?;
                    return self.incomplete(
                        state,
                        &format!("Reviewer A did not return structured findings: {reason}"),
                    );
                }
            };
            apply_review(&mut state, "reviewer", &payload, 0);
        }
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
            let chal_schema = if stage == "implement" {
                "{\"verdict\":\"accept|reject\",\"objections\":[],\"resolved\":[]}"
            } else {
                "{\"verdict\":\"accept|reject\",\"assessments\":[],\"responses\":[],\"additional\":[]}"
            };
            let chal_required: &[&str] = if stage == "implement" {
                &["verdict", "objections"]
            } else {
                &["verdict", "assessments"]
            };
            let (result, chal_payload) = self
                .complete_structured_turn(
                    &mut state,
                    "challenger",
                    round,
                    challenge,
                    chal_required,
                    chal_schema,
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
            let payload = match chal_payload {
                Ok(valid) => valid,
                Err(reason) => {
                    record_schema_error(&mut state, "challenger", round, &reason);
                    record_workflow_health(
                        &self.store,
                        "clash",
                        &challenger_provider,
                        self.session["model"].as_str(),
                        self.session["id"].as_str().unwrap_or(""),
                        "schema_error",
                        true,
                        true,
                        None,
                        0.0,
                    );
                    let warnings = workflow_health_warnings(
                        &self.store,
                        "clash",
                        &challenger_provider,
                        self.session["model"].as_str(),
                    );
                    merge_health_warnings(&mut state, &warnings);
                    self.persist(&state)?;
                    return self.incomplete(
                        state,
                        &format!("The challenger did not return a structured verdict: {reason}"),
                    );
                }
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
            let schema_spec = if stage == "implement" {
                "{\"responses\":[{\"id\":1,\"action\":\"fixed|rebutted|deferred\",\"note\":\"...\"}]}"
            } else {
                "{\"assessments\":[{\"id\":1,\"stance\":\"agree|dispute|unverified\",\"evidence\":\"...\"}],\"responses\":[{\"id\":1,\"action\":\"defend|withdraw\",\"note\":\"...\"}]}"
            };
            let resp_required: &[&str] = if stage == "implement" {
                &["responses"]
            } else {
                &["responses", "assessments"]
            };
            let (result, payload) = self
                .complete_structured_turn(
                    &mut state,
                    role,
                    round,
                    response,
                    resp_required,
                    schema_spec,
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
            let payload = match payload {
                Ok(valid) => valid,
                Err(reason) => {
                    record_schema_error(&mut state, role, round, &reason);
                    record_workflow_health(
                        &self.store,
                        "clash",
                        &protagonist_provider,
                        self.session["model"].as_str(),
                        self.session["id"].as_str().unwrap_or(""),
                        "schema_error",
                        true,
                        true,
                        None,
                        0.0,
                    );
                    let warnings = workflow_health_warnings(
                        &self.store,
                        "clash",
                        &protagonist_provider,
                        self.session["model"].as_str(),
                    );
                    merge_health_warnings(&mut state, &warnings);
                    self.persist(&state)?;
                    return self.incomplete(
                        state,
                        &format!("Clash {role} output failed JSON schema validation: {reason}"),
                    );
                }
            };
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

    #[expect(
        clippy::too_many_arguments,
        reason = "Structured turn runner carries shared cycle budget, schema requirements, and bounded repair state."
    )]
    async fn complete_structured_turn(
        &mut self,
        state: &mut Value,
        role: &str,
        round: u64,
        prompt: String,
        required: &[&str],
        schema_spec: &str,
        budget: &mut CycleBudget,
        cancel: Arc<AtomicBool>,
        emit: Arc<dyn Fn(Value) -> Result<()> + Send + Sync>,
    ) -> Result<(provider::WorkerResult, std::result::Result<Value, String>)> {
        let result = self
            .turn(
                state,
                role,
                round,
                prompt,
                budget,
                cancel.clone(),
                emit.clone(),
            )
            .await?;
        if budget.post_turn_violation(&result).is_some() {
            return Ok((result, Err("budget exceeded during turn".into())));
        }
        if !result.ok {
            return Ok((
                result,
                Err("turn failed before returning structured output".into()),
            ));
        }
        match parse_strict_payload(&result.text, required) {
            Ok(valid) => Ok((result, Ok(valid))),
            Err(err) => {
                let provider = self.turn_provider(state, role)?;
                if cancel.load(Ordering::Acquire) || budget.pre_turn_violation(&provider).is_some()
                {
                    return Ok((result, Err(err)));
                }
                let snippet: String = result.text.chars().take(1024).collect();
                let repair_prompt = format!(
                    "Your previous response did not match the required JSON schema ({err}).\nExpected schema: {schema_spec}\nReturn ONLY a single valid JSON object matching the schema with no surrounding commentary.\nPrevious invalid reply:\n{snippet}"
                );
                let mut repaired = self
                    .turn(state, role, round, repair_prompt, budget, cancel, emit)
                    .await?;
                repaired.cost_usd = match (result.cost_usd, repaired.cost_usd) {
                    (Some(a), Some(b)) => Some(a + b),
                    (a, b) => a.or(b),
                };
                if budget.post_turn_violation(&repaired).is_some() || !repaired.ok {
                    return Ok((repaired, Err(err)));
                }
                let validated = parse_strict_payload(&repaired.text, required);
                Ok((repaired, validated))
            }
        }
    }
}

/// Strictly parses a Clash structured turn response: accepts a whole JSON object
/// or a single fenced ```json``` block without heuristic backward brace scanning.
pub fn parse_strict_payload(text: &str, required: &[&str]) -> std::result::Result<Value, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("empty response".into());
    }
    let parsed = if let Ok(v) = serde_json::from_str::<Value>(trimmed)
        && v.is_object()
    {
        v
    } else if let Some(fenced) = extract_json_fence(trimmed)
        && let Ok(v) = serde_json::from_str::<Value>(fenced)
        && v.is_object()
    {
        v
    } else {
        return Err("response is not a valid JSON object or fenced ```json block".into());
    };
    if !required.is_empty() && !required.iter().any(|k| parsed.get(*k).is_some()) {
        return Err(format!(
            "missing one of required keys: {}",
            required.join(", ")
        ));
    }
    validate_clash_schema(&parsed)?;
    Ok(parsed)
}

/// Extracts the payload inside a Markdown fenced ```json or ``` code block.
pub(crate) fn extract_json_fence(text: &str) -> Option<&str> {
    let start = text.find("```json").or_else(|| text.find("```"))?;
    let after = &text[start + 3..];
    let body_start = after.find('\n')? + 1;
    let rest = &after[body_start..];
    let end = rest.rfind("\n```").or_else(|| rest.rfind("```"))?;
    Some(rest[..end].trim())
}

fn check_optional_string(
    obj: &serde_json::Map<String, Value>,
    ctx: &str,
    key: &str,
) -> std::result::Result<(), String> {
    if let Some(v) = obj.get(key)
        && !v.is_null()
        && !v.is_string()
    {
        return Err(format!("`{ctx}.{key}` must be a string"));
    }
    Ok(())
}

fn check_optional_u64(
    obj: &serde_json::Map<String, Value>,
    ctx: &str,
    key: &str,
) -> std::result::Result<(), String> {
    if let Some(v) = obj.get(key)
        && !v.is_null()
        && !v.is_u64()
    {
        return Err(format!("`{ctx}.{key}` must be a non-negative integer"));
    }
    Ok(())
}

fn check_optional_severity(
    obj: &serde_json::Map<String, Value>,
    ctx: &str,
) -> std::result::Result<(), String> {
    if let Some(v) = obj.get("severity")
        && !v.is_null()
    {
        let sev = v
            .as_str()
            .map(|s| s.trim().to_ascii_lowercase())
            .unwrap_or_default();
        if !["critical", "high", "medium", "low", "info", "unrated"].contains(&sev.as_str()) {
            return Err(format!("`{ctx}.severity` has invalid value `{sev}`"));
        }
    }
    Ok(())
}

fn validate_clash_schema(value: &Value) -> std::result::Result<(), String> {
    if let Some(verdict) = value.get("verdict") {
        let Some(v) = verdict.as_str() else {
            return Err("`verdict` must be a string".into());
        };
        if !["accept", "reject", "continue"].contains(&v) {
            return Err(format!("invalid verdict `{v}`"));
        }
    }
    if let Some(resolved) = value.get("resolved")
        && !resolved.is_null()
    {
        let Some(arr) = resolved.as_array() else {
            return Err("`resolved` must be an array".into());
        };
        for (idx, item) in arr.iter().enumerate() {
            if !item.is_u64() {
                return Err(format!("`resolved[{idx}]` must be a non-negative integer"));
            }
        }
    }
    for array_key in [
        "objections",
        "findings",
        "additional",
        "responses",
        "assessments",
    ] {
        let Some(val) = value.get(array_key) else {
            continue;
        };
        if val.is_null() {
            continue;
        }
        let Some(arr) = val.as_array() else {
            return Err(format!("`{array_key}` must be an array"));
        };
        for (idx, item) in arr.iter().enumerate() {
            let ctx = format!("{array_key}[{idx}]");
            let Some(obj) = item.as_object() else {
                return Err(format!("`{ctx}` must be an object"));
            };
            match array_key {
                "objections" => {
                    check_optional_u64(obj, &ctx, "id")?;
                    let has_id = obj.get("id").is_some_and(Value::is_u64);
                    if !has_id || obj.get("claim").is_some_and(|v| !v.is_null()) {
                        let valid_claim = obj
                            .get("claim")
                            .and_then(Value::as_str)
                            .is_some_and(|s| !s.trim().is_empty());
                        if !valid_claim {
                            return Err(format!("`{ctx}.claim` must be a non-empty string"));
                        }
                    }
                    check_optional_severity(obj, &ctx)?;
                    check_optional_u64(obj, &ctx, "line")?;
                    check_optional_string(obj, &ctx, "file")?;
                    check_optional_string(obj, &ctx, "evidence")?;
                }
                "findings" | "additional" => {
                    let valid_claim = obj
                        .get("claim")
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.trim().is_empty());
                    if !valid_claim {
                        return Err(format!("`{ctx}.claim` must be a non-empty string"));
                    }
                    check_optional_severity(obj, &ctx)?;
                    check_optional_u64(obj, &ctx, "line")?;
                    check_optional_string(obj, &ctx, "file")?;
                    check_optional_string(obj, &ctx, "evidence")?;
                }
                "responses" => {
                    if !obj.get("id").is_some_and(Value::is_u64) {
                        return Err(format!("`{ctx}.id` must be a non-negative integer"));
                    }
                    let action = obj.get("action").and_then(Value::as_str).unwrap_or("");
                    if !["fixed", "rebutted", "deferred", "defend", "withdraw"].contains(&action) {
                        return Err(format!("`{ctx}.action` has invalid value `{action}`"));
                    }
                    check_optional_string(obj, &ctx, "note")?;
                }
                "assessments" => {
                    if !obj.get("id").is_some_and(Value::is_u64) {
                        return Err(format!("`{ctx}.id` must be a non-negative integer"));
                    }
                    let stance = obj.get("stance").and_then(Value::as_str).unwrap_or("");
                    if !["agree", "dispute", "unverified"].contains(&stance) {
                        return Err(format!("`{ctx}.stance` has invalid value `{stance}`"));
                    }
                    check_optional_string(obj, &ctx, "evidence")?;
                }
                _ => {}
            }
        }
    }
    Ok(())
}

pub(crate) fn observe_usage_event(slot: &mut Value, event: &Value) {
    for key in [
        "input_tokens",
        "output_tokens",
        "cache_read_input_tokens",
        "cache_write_5m_input_tokens",
        "cache_write_1h_input_tokens",
    ] {
        if let Some(v) = event.get(key).and_then(Value::as_u64) {
            let cur = slot.get(key).and_then(Value::as_u64).unwrap_or(0);
            slot[key] = json!(cur.saturating_add(v));
        }
    }
    if event["cache_write_5m_input_tokens"].is_null()
        && let Some(creation) = event
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64)
    {
        let cur = slot
            .get("cache_write_5m_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        slot["cache_write_5m_input_tokens"] = json!(cur.saturating_add(creation));
    }
    let calls = slot.get("model_calls").and_then(Value::as_u64).unwrap_or(0);
    slot["model_calls"] = json!(calls.saturating_add(1));
}

fn record_schema_error(state: &mut Value, role: &str, round: u64, reason: &str) {
    let mut errs = state["schema_errors"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if errs.len() < 20 {
        errs.push(json!({"role": role, "round": round, "error": reason}));
    }
    state["schema_errors"] = json!(errs);
}

pub(crate) fn merge_usage_components(acc: &mut Value, turn: &Value) {
    if !acc.is_object() {
        *acc = json!({
            "input_tokens": 0u64,
            "output_tokens": 0u64,
            "cache_read_input_tokens": 0u64,
            "cache_write_5m_input_tokens": 0u64,
            "cache_write_1h_input_tokens": 0u64,
            "model_calls": 0u64,
        });
    }
    for key in [
        "input_tokens",
        "output_tokens",
        "cache_read_input_tokens",
        "cache_write_5m_input_tokens",
        "cache_write_1h_input_tokens",
        "model_calls",
    ] {
        if let Some(add) = turn.get(key).and_then(Value::as_u64) {
            let cur = acc.get(key).and_then(Value::as_u64).unwrap_or(0);
            acc[key] = json!(cur.saturating_add(add));
        }
    }
}

fn precheck_workspace(session: &Value) -> &std::path::Path {
    let workspace = session["project_path"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            session["_workspace_path"]
                .as_str()
                .filter(|s| !s.is_empty())
        })
        .unwrap_or("");
    std::path::Path::new(workspace)
}

async fn run_clash_precheck(store: &SessionStore, session: &Value) -> Option<(String, Value)> {
    let precheck = session
        .get("clash")
        .and_then(|c| c.get("precheck"))
        .filter(|p| !p.is_null())?;
    let cwd = precheck_workspace(session);
    let record = run_precheck_sandboxed_in_store(store, cwd, precheck).await?;
    let summary = record["summary"].as_str().unwrap_or("").to_owned();
    Some((summary, record))
}

/// Nanosecond stat racy-window threshold matching `infra_skillcache::fingerprint::RACY_NS`.
pub(crate) const PRECHECK_RACY_NS: i128 = 2_000_000_000;
const HEALTH_WINDOW_LIMIT: usize = 50;
const HEALTH_DECAY: f64 = 0.82;
const HEALTH_PRIOR_ALPHA: f64 = 0.25;
const HEALTH_PRIOR_WEIGHT: f64 = 1.5;
const MAX_PRECHECK_L1_ENTRIES: usize = 128;
const MAX_PRECHECK_PERSISTED_ROWS: usize = 2048;

static PRECHECK_SEQ: AtomicU64 = AtomicU64::new(0);

fn next_precheck_seq() -> i64 {
    let nanos = u64::try_from(now_unix_ns().max(0)).unwrap_or(u64::MAX / 2);
    let mut cur = PRECHECK_SEQ.load(Ordering::Relaxed);
    loop {
        let next = cur.max(nanos).saturating_add(1);
        match PRECHECK_SEQ.compare_exchange_weak(cur, next, Ordering::SeqCst, Ordering::Relaxed) {
            Ok(_) => return i64::try_from(next & (i64::MAX as u64)).unwrap_or(i64::MAX),
            Err(observed) => cur = observed,
        }
    }
}

pub(crate) fn ensure_health_table(db: &rusqlite::Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS workflow_health_runs(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            workflow TEXT NOT NULL,
            provider TEXT NOT NULL,
            model TEXT NOT NULL,
            run_key TEXT NOT NULL,
            outcome TEXT NOT NULL,
            schema_error INTEGER NOT NULL DEFAULT 0,
            deopt INTEGER NOT NULL DEFAULT 0,
            cost_usd REAL NOT NULL DEFAULT 0.0,
            duration_seconds REAL NOT NULL DEFAULT 0.0,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS workflow_health_lookup
            ON workflow_health_runs(workflow, provider, model, id DESC);",
    )?;
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "Health ledger row columns mirror skillcache_runs explicitly."
)]
pub(crate) fn record_workflow_health(
    store: &SessionStore,
    workflow: &str,
    provider: &str,
    model: Option<&str>,
    run_key: &str,
    outcome: &str,
    schema_error: bool,
    deopt: bool,
    cost_usd: Option<f64>,
    duration_seconds: f64,
) {
    let Ok(db) = store.connection() else {
        return;
    };
    if ensure_health_table(&db).is_err() {
        return;
    }
    let _ = db.execute(
        "INSERT INTO workflow_health_runs(workflow,provider,model,run_key,outcome,schema_error,deopt,cost_usd,duration_seconds,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        rusqlite::params![
            workflow,
            provider,
            model.unwrap_or("default"),
            run_key,
            outcome,
            i64::from(schema_error),
            i64::from(deopt),
            cost_usd.unwrap_or(0.0),
            duration_seconds,
            crate::store::now(),
        ],
    );
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct WorkflowHealthSummary {
    pub total_runs: usize,
    pub schema_error_runs: usize,
    pub deopt_runs: usize,
    pub total_cost_usd: f64,
    pub schema_error_rate: f64,
    pub deopt_rate: f64,
    pub warnings: Vec<String>,
}

impl WorkflowHealthSummary {
    /// Pre-launch Clash round budget recommendation based on smoothed health.
    pub fn recommended_clash_rounds(&self, requested_rounds: u64) -> u64 {
        let requested = requested_rounds.clamp(1, 3);
        if self.total_runs < 3 {
            return requested;
        }
        if self.schema_error_rate > 0.45 || self.deopt_rate > 0.50 {
            requested.min(1)
        } else if self.schema_error_rate > 0.25 || self.deopt_rate > 0.30 {
            requested.min(2)
        } else {
            requested
        }
    }

    /// Pre-launch Fleet worker concurrency recommendation based on smoothed health.
    pub fn recommended_fleet_concurrency(&self, requested: usize) -> usize {
        let req = requested.max(1);
        if self.total_runs < 3 || req <= 1 {
            return req;
        }
        if self.schema_error_rate > 0.45 || self.deopt_rate > 0.50 {
            1
        } else if self.schema_error_rate > 0.25 || self.deopt_rate > 0.30 {
            (req / 2).max(1)
        } else {
            req
        }
    }
}

pub(crate) fn merge_health_warnings(state: &mut Value, extra: &[String]) {
    if extra.is_empty() {
        return;
    }
    let mut existing: Vec<String> = state["health_warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    for w in extra {
        if !existing.contains(w) {
            existing.push(w.clone());
        }
    }
    state["health_warnings"] = json!(existing);
}

fn apply_prelaunch_health_policy(
    store: &SessionStore,
    state: &mut Value,
    protagonist_provider: &str,
    challenger_provider: &str,
) -> Vec<String> {
    let pro_health = summarize_workflow_health(
        store,
        "clash",
        protagonist_provider,
        state["protagonist"]["model"].as_str(),
    );
    let chal_health = summarize_workflow_health(
        store,
        "clash",
        challenger_provider,
        state["challenger"]["model"].as_str(),
    );
    let requested = state["rounds"].as_u64().unwrap_or(2).clamp(1, 3);
    let clamped = pro_health
        .recommended_clash_rounds(requested)
        .min(chal_health.recommended_clash_rounds(requested));
    let mut warnings = Vec::new();
    for w in pro_health
        .warnings
        .iter()
        .chain(chal_health.warnings.iter())
    {
        if !warnings.contains(w) {
            warnings.push(w.clone());
        }
    }
    if clamped < requested {
        state["rounds"] = json!(clamped);
        warnings.push(format!(
            "Clamped Clash rounds pre-launch from {requested} to {clamped} based on smoothed workflow health."
        ));
    }
    merge_health_warnings(state, &warnings);
    warnings
}

pub(crate) fn summarize_workflow_health(
    store: &SessionStore,
    workflow: &str,
    provider: &str,
    model: Option<&str>,
) -> WorkflowHealthSummary {
    let mut summary = WorkflowHealthSummary::default();
    let Ok(db) = store.connection() else {
        return summary;
    };
    if ensure_health_table(&db).is_err() {
        return summary;
    }
    let model_key = model.unwrap_or("default");
    let Ok(mut stmt) = db.prepare(
        "SELECT schema_error, deopt, cost_usd
         FROM workflow_health_runs
         WHERE workflow=?1 AND provider=?2 AND model=?3
         ORDER BY id DESC LIMIT ?4",
    ) else {
        return summary;
    };
    let Ok(rows) = stmt.query_map(
        rusqlite::params![
            workflow,
            provider,
            model_key,
            i64::try_from(HEALTH_WINDOW_LIMIT).unwrap_or(50)
        ],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, f64>(2)?,
            ))
        },
    ) else {
        return summary;
    };
    let mut weight = 1.0_f64;
    let mut weight_sum = 0.0_f64;
    let mut weighted_schema = 0.0_f64;
    let mut weighted_deopt = 0.0_f64;
    for row in rows.flatten() {
        summary.total_runs += 1;
        if row.0 != 0 {
            summary.schema_error_runs += 1;
            weighted_schema += weight;
        }
        if row.1 != 0 {
            summary.deopt_runs += 1;
            weighted_deopt += weight;
        }
        if row.2.is_finite() && row.2 > 0.0 {
            summary.total_cost_usd += row.2;
        }
        weight_sum += weight;
        weight *= HEALTH_DECAY;
    }
    if summary.total_runs > 0 && weight_sum > 0.0 {
        summary.schema_error_rate =
            (weighted_schema + HEALTH_PRIOR_ALPHA) / (weight_sum + HEALTH_PRIOR_WEIGHT);
        summary.deopt_rate =
            (weighted_deopt + HEALTH_PRIOR_ALPHA) / (weight_sum + HEALTH_PRIOR_WEIGHT);
    }
    if summary.total_runs >= 3 {
        if summary.schema_error_rate > 0.25 {
            summary.warnings.push(format!(
                "High smoothed schema error rate for {workflow}/{provider}/{model_key}: {:.0}% ({}/{})",
                summary.schema_error_rate * 100.0,
                summary.schema_error_runs,
                summary.total_runs
            ));
        }
        if summary.deopt_rate > 0.30 {
            summary.warnings.push(format!(
                "Degraded smoothed ROI/deopt warning for {workflow}/{provider}/{model_key}: {:.0}% ({}/{})",
                summary.deopt_rate * 100.0,
                summary.deopt_runs,
                summary.total_runs
            ));
        }
    }
    summary
}

pub(crate) fn workflow_health_warnings(
    store: &SessionStore,
    workflow: &str,
    provider: &str,
    model: Option<&str>,
) -> Vec<String> {
    summarize_workflow_health(store, workflow, provider, model).warnings
}

fn time_to_unix_ns(time: std::time::SystemTime) -> Option<i128> {
    match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(delta) => i128::try_from(delta.as_nanos()).ok(),
        Err(err) => i128::try_from(err.duration().as_nanos())
            .ok()
            .and_then(|ns| ns.checked_neg()),
    }
}

fn now_unix_ns() -> i128 {
    time_to_unix_ns(std::time::SystemTime::now()).unwrap_or(0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrecheckStampStrategy {
    ScopedFiles,
    IrAndGit,
    IrIndexed,
    BoundedWalk,
}

fn is_single_file_linter_argv(argv: &[String]) -> Option<Vec<String>> {
    if argv.len() < 2 {
        return None;
    }
    let bin = std::path::Path::new(&argv[0])
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_single_target_cmd = (bin == "php" && argv.iter().any(|a| a == "-l"))
        || (matches!(bin.as_str(), "node" | "nodejs")
            && argv.iter().any(|a| a == "-c" || a == "--check"))
        || (matches!(bin.as_str(), "eslint" | "oxlint" | "prettier"));
    if !is_single_target_cmd {
        return None;
    }
    let files: Vec<String> = argv
        .iter()
        .skip(1)
        .filter(|arg| {
            let trimmed = arg.trim();
            !trimmed.is_empty()
                && !trimmed.starts_with('-')
                && !trimmed.contains('\0')
                && (trimmed.contains('/')
                    || trimmed.contains('.')
                    || trimmed == "src"
                    || trimmed == "tests")
        })
        .cloned()
        .collect();
    if files.is_empty() { None } else { Some(files) }
}

fn stat_rel_path(
    root: &Path,
    rel: &str,
    stamps: &mut Vec<String>,
    max_mtime_ns: &mut i128,
) -> bool {
    let clean = rel.replace('\\', "/");
    if clean.starts_with('/')
        || clean
            .split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return false;
    }
    let path = root.join(&clean);
    let Ok(meta) = std::fs::symlink_metadata(&path) else {
        stamps.push(format!("missing:{clean}"));
        return true;
    };
    if meta.file_type().is_symlink() {
        return false;
    }
    if meta.is_file() {
        let Some(mtime) = meta.modified().ok().and_then(time_to_unix_ns) else {
            return false;
        };
        *max_mtime_ns = (*max_mtime_ns).max(mtime);
        stamps.push(format!("file:{clean}:{}:{mtime}", meta.len()));
    } else if meta.is_dir() {
        let Some(mtime) = meta.modified().ok().and_then(time_to_unix_ns) else {
            return false;
        };
        *max_mtime_ns = (*max_mtime_ns).max(mtime);
        stamps.push(format!("dir:{clean}:{mtime}"));
    }
    true
}

fn collect_root_manifests(root: &Path, stamps: &mut Vec<String>, max_mtime_ns: &mut i128) {
    const MANIFESTS: &[&str] = &[
        "composer.json",
        "composer.lock",
        "package.json",
        "package-lock.json",
        "pnpm-lock.yaml",
        "yarn.lock",
        "Cargo.toml",
        "Cargo.lock",
        "phpunit.xml",
        "phpunit.xml.dist",
        "phpstan.neon",
        "phpstan.neon.dist",
        "eslint.config.js",
        "eslint.config.mjs",
        "oxlint.json",
        "biome.json",
    ];
    for name in MANIFESTS {
        let p = root.join(name);
        if let Ok(meta) = std::fs::symlink_metadata(&p)
            && !meta.file_type().is_symlink()
            && meta.is_file()
            && let Some(mtime) = meta.modified().ok().and_then(time_to_unix_ns)
        {
            *max_mtime_ns = (*max_mtime_ns).max(mtime);
            stamps.push(format!("manifest:{name}:{}:{mtime}", meta.len()));
        }
    }
}

fn resolve_graph_ir_for_workspace(
    store: Option<&SessionStore>,
    root_canon: &Path,
) -> Option<Arc<crate::graph_ir::GraphIrStore>> {
    let store = store?;
    let root_str = root_canon.to_str()?;
    let identity = if cfg!(windows) {
        root_str.replace('/', "\\").to_lowercase()
    } else {
        root_str.to_owned()
    };
    let candidate_id = crate::store::sha(identity.as_bytes())[..16].to_owned();
    if let Ok(Some(ir)) =
        crate::graph_ir::GraphIrStore::open_readonly_cached(&store.state, &candidate_id)
    {
        return Some(ir);
    }
    if let Ok(projects) = store.projects() {
        for proj in projects {
            if let (Some(pid), Some(path)) = (proj["id"].as_str(), proj["path"].as_str())
                && infra_core::fs::canonicalize(Path::new(path))
                    .ok()
                    .as_deref()
                    == Some(root_canon)
                && let Ok(Some(ir)) =
                    crate::graph_ir::GraphIrStore::open_readonly_cached(&store.state, pid)
            {
                return Some(ir);
            }
        }
    }
    None
}

fn precheck_git_overlay_stamp(
    root: &Path,
    stamps: &mut Vec<String>,
    max_mtime_ns: &mut i128,
) -> bool {
    if !root.join(".git").exists() {
        return false;
    }
    let head = crate::graph_ir::current_git_head(root).unwrap_or_default();
    stamps.push(format!("git_head:{head}"));
    let Ok(out) = std::process::Command::new("git")
        .args(["status", "--porcelain=v1", "--untracked-files=all", "-z"])
        .current_dir(root)
        .output()
    else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    let mut entries = out.stdout.split(|b| *b == 0);
    while let Some(raw_entry) = entries.next() {
        if raw_entry.len() < 4 {
            continue;
        }
        let status = String::from_utf8_lossy(&raw_entry[..2]).into_owned();
        let path_part = String::from_utf8_lossy(&raw_entry[3..]).into_owned();
        if status.starts_with('R') || status.starts_with('C') {
            let _ = entries.next();
        }
        stamps.push(format!("git_dirty:{status}:{path_part}"));
        if !stat_rel_path(root, &path_part, stamps, max_mtime_ns) {
            return false;
        }
    }
    true
}

fn precheck_workspace_stamp_scoped(
    root: &std::path::Path,
    argv: &[String],
    store: Option<&SessionStore>,
) -> Option<(String, i128, PrecheckStampStrategy)> {
    const MAX_ENTRIES: usize = 2000;
    let mut max_mtime_ns = root
        .metadata()
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(time_to_unix_ns)
        .unwrap_or(0);

    // Tier 1: Single-file syntax/linter commands stamp only their target path(s) + manifests.
    if let Some(target_args) = is_single_file_linter_argv(argv) {
        let mut stamps = Vec::new();
        collect_root_manifests(root, &mut stamps, &mut max_mtime_ns);
        let mut valid = true;
        for arg in &target_args {
            let candidate = root.join(arg);
            if candidate.is_dir() {
                let mut sub_queue = std::collections::VecDeque::from([candidate]);
                while let Some(dir) = sub_queue.pop_front() {
                    let Ok(entries) = std::fs::read_dir(&dir) else {
                        valid = false;
                        break;
                    };
                    let mut children: Vec<_> = entries.filter_map(|e| e.ok()).collect();
                    children.sort_by_key(|e| e.file_name());
                    for entry in children {
                        let p = entry.path();
                        let Ok(meta) = std::fs::symlink_metadata(&p) else {
                            valid = false;
                            break;
                        };
                        if meta.file_type().is_symlink() {
                            valid = false;
                            break;
                        }
                        if meta.is_dir() {
                            sub_queue.push_back(p);
                        } else if meta.is_file() {
                            let Some(rel) = p.strip_prefix(root).ok().and_then(|r| r.to_str())
                            else {
                                valid = false;
                                break;
                            };
                            if !stat_rel_path(root, rel, &mut stamps, &mut max_mtime_ns) {
                                valid = false;
                                break;
                            }
                        }
                        if stamps.len() >= MAX_ENTRIES {
                            valid = false;
                            break;
                        }
                    }
                    if !valid {
                        break;
                    }
                }
            } else if !stat_rel_path(root, arg, &mut stamps, &mut max_mtime_ns) {
                valid = false;
                break;
            }
        }
        if valid {
            let digest = crate::store::sha(stamps.join("\n").as_bytes());
            return Some((digest, max_mtime_ns, PrecheckStampStrategy::ScopedFiles));
        }
    }

    // Tier 2: GraphIrStore revision + Git dirty/untracked overlay (or indexed file/dir stats).
    if let Some(ir) = resolve_graph_ir_for_workspace(store, root)
        && let Ok((indexed_stamps, _, _)) = ir.file_stamps_and_meta()
    {
        let mut stamps = vec![
            format!("ir_epoch:{}", ir.current_write_epoch()),
            format!(
                "root_manifest:{}",
                crate::graph_ir::compute_root_manifest_stamp(root)
            ),
        ];
        collect_root_manifests(root, &mut stamps, &mut max_mtime_ns);
        if precheck_git_overlay_stamp(root, &mut stamps, &mut max_mtime_ns) {
            for (mtime, _len) in indexed_stamps.values() {
                max_mtime_ns = max_mtime_ns.max(i128::from(*mtime));
            }
            let digest = crate::store::sha(stamps.join("\n").as_bytes());
            return Some((digest, max_mtime_ns, PrecheckStampStrategy::IrAndGit));
        }
        let mut parent_dirs = BTreeSet::new();
        parent_dirs.insert(root.to_path_buf());
        let mut ok = !indexed_stamps.is_empty();
        for rel in indexed_stamps.keys() {
            if !stat_rel_path(root, rel, &mut stamps, &mut max_mtime_ns) {
                ok = false;
                break;
            }
            if let Some(parent) = root.join(rel).parent() {
                parent_dirs.insert(parent.to_path_buf());
            }
        }
        if ok {
            for dir in parent_dirs {
                if let Ok(meta) = std::fs::symlink_metadata(&dir)
                    && let Some(mtime) = meta.modified().ok().and_then(time_to_unix_ns)
                {
                    max_mtime_ns = max_mtime_ns.max(mtime);
                    let rel_dir = dir
                        .strip_prefix(root)
                        .ok()
                        .map(|r| r.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_default();
                    stamps.push(format!("pdir:{rel_dir}:{mtime}"));
                }
            }
            let digest = crate::store::sha(stamps.join("\n").as_bytes());
            return Some((digest, max_mtime_ns, PrecheckStampStrategy::IrIndexed));
        }
    }

    // Tier 3: Bounded filesystem walk fallback.
    let mut queue = std::collections::VecDeque::from([root.to_path_buf()]);
    let mut stamps = Vec::new();
    while let Some(dir) = queue.pop_front() {
        let entries = std::fs::read_dir(&dir).ok()?;
        let mut children: Vec<_> = entries.filter_map(|e| e.ok()).collect();
        children.sort_by_key(|e| e.file_name());
        for entry in children {
            let name = entry.file_name();
            let name_s = name.to_string_lossy();
            if matches!(
                name_s.as_ref(),
                ".git" | "target" | "vendor" | "node_modules" | ".phpunit.cache" | ".infra-work"
            ) {
                continue;
            }
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path).ok()?;
            if meta.file_type().is_symlink() {
                return None;
            }
            if meta.is_dir() {
                queue.push_back(path);
                continue;
            }
            if meta.is_file() {
                let mtime = meta.modified().ok().and_then(time_to_unix_ns)?;
                max_mtime_ns = max_mtime_ns.max(mtime);
                let rel = path
                    .strip_prefix(root)
                    .ok()?
                    .to_string_lossy()
                    .replace('\\', "/");
                stamps.push(format!("{rel}:{}:{mtime}", meta.len()));
                if stamps.len() >= MAX_ENTRIES {
                    return None;
                }
            }
        }
    }
    let digest = crate::store::sha(stamps.join("\n").as_bytes());
    Some((digest, max_mtime_ns, PrecheckStampStrategy::BoundedWalk))
}

#[cfg_attr(not(test), expect(dead_code))]
fn precheck_workspace_stamp(root: &std::path::Path) -> Option<(String, i128)> {
    precheck_workspace_stamp_scoped(root, &[], None).map(|(digest, mtime, _)| (digest, mtime))
}

fn precheck_cache_settled(max_mtime_ns: i128, now_ns: i128) -> bool {
    max_mtime_ns > 0 && now_ns > PRECHECK_RACY_NS && max_mtime_ns <= now_ns - PRECHECK_RACY_NS
}

#[derive(Default)]
struct PrecheckL1Cache {
    seq: u64,
    entries: BTreeMap<String, (Value, u64)>,
}

impl PrecheckL1Cache {
    fn get(&mut self, key: &str) -> Option<Value> {
        self.seq = self.seq.saturating_add(1);
        let use_seq = self.seq;
        let entry = self.entries.get_mut(key)?;
        entry.1 = use_seq;
        Some(entry.0.clone())
    }

    fn insert(&mut self, key: String, record: Value) {
        self.seq = self.seq.saturating_add(1);
        let use_seq = self.seq;
        if self.entries.len() >= MAX_PRECHECK_L1_ENTRIES
            && !self.entries.contains_key(&key)
            && let Some(evict_key) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, last_used))| *last_used)
                .map(|(k, _)| k.clone())
        {
            self.entries.remove(&evict_key);
        }
        self.entries.insert(key, (record, use_seq));
    }
}

fn ensure_precheck_cache_table(db: &rusqlite::Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS precheck_receipt_cache(
            cache_key TEXT PRIMARY KEY,
            record_json TEXT NOT NULL,
            updated_at INTEGER NOT NULL,
            last_used_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS precheck_receipt_cache_lru
            ON precheck_receipt_cache(last_used_at ASC);",
    )?;
    Ok(())
}

fn load_persisted_precheck(store: &SessionStore, key: &str) -> Option<Value> {
    let db = store.connection().ok()?;
    ensure_precheck_cache_table(&db).ok()?;
    let raw: String = db
        .query_row(
            "SELECT record_json FROM precheck_receipt_cache WHERE cache_key = ?1",
            [key],
            |r| r.get(0),
        )
        .ok()?;
    let parsed: Value = serde_json::from_str(&raw).ok()?;
    let _ = db.execute(
        "UPDATE precheck_receipt_cache SET last_used_at = ?2 WHERE cache_key = ?1",
        rusqlite::params![key, next_precheck_seq()],
    );
    Some(parsed)
}

fn save_persisted_precheck(store: &SessionStore, key: &str, record: &Value) {
    let Ok(db) = store.connection() else {
        return;
    };
    if ensure_precheck_cache_table(&db).is_err() {
        return;
    }
    let seq = next_precheck_seq();
    let _ = db.execute(
        "INSERT INTO precheck_receipt_cache(cache_key, record_json, updated_at, last_used_at)
         VALUES (?1, ?2, ?3, ?3)
         ON CONFLICT(cache_key) DO UPDATE SET
             record_json = excluded.record_json,
             updated_at = excluded.updated_at,
             last_used_at = excluded.last_used_at",
        rusqlite::params![key, record.to_string(), seq],
    );
    let _ = db.execute(
        "DELETE FROM precheck_receipt_cache
         WHERE cache_key IN (
             SELECT cache_key FROM precheck_receipt_cache
             ORDER BY last_used_at DESC
             LIMIT -1 OFFSET ?1
         )",
        rusqlite::params![i64::try_from(MAX_PRECHECK_PERSISTED_ROWS).unwrap_or(2048)],
    );
}

type PrecheckExec<'a> = &'a (
        dyn Fn(
    &[String],
    &std::path::Path,
    &BTreeMap<String, String>,
) -> std::result::Result<(i32, String), String>
            + Sync
    );

/// Runs a deterministic precheck with durable SQLite receipt persistence in `store`.
pub(crate) async fn run_precheck_sandboxed_in_store(
    store: &SessionStore,
    workspace: &std::path::Path,
    precheck: &Value,
) -> Option<Value> {
    run_precheck_sandboxed_at(Some(store), workspace, precheck, now_unix_ns(), None).await
}

async fn run_precheck_sandboxed_at(
    store: Option<&SessionStore>,
    workspace: &std::path::Path,
    precheck: &Value,
    now_ns: i128,
    exec: Option<PrecheckExec<'_>>,
) -> Option<Value> {
    let argv: Vec<String> = precheck
        .get("argv")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if argv.is_empty() || !workspace.is_dir() {
        return None;
    }
    let cwd_canon = infra_core::fs::canonicalize(workspace).ok()?;
    let env = infra_host::sandbox::scrub_env(std::env::vars(), &BTreeMap::new());
    let timeout = precheck["timeout_seconds"]
        .as_u64()
        .unwrap_or(30)
        .clamp(1, 120);
    let cache_enabled = precheck.get("no_cache").and_then(Value::as_bool) != Some(true)
        && std::env::var("HARNESS_DISABLE_CHECK_CACHE").as_deref() != Ok("1");
    static PRECHECK_CACHE: std::sync::OnceLock<Mutex<PrecheckL1Cache>> = std::sync::OnceLock::new();
    let cache = PRECHECK_CACHE.get_or_init(|| Mutex::new(PrecheckL1Cache::default()));
    let stamp = if cache_enabled {
        precheck_workspace_stamp_scoped(&cwd_canon, &argv, store)
    } else {
        None
    };
    let cache_key = stamp.as_ref().and_then(|(tree_digest, max_mtime, _)| {
        precheck_cache_settled(*max_mtime, now_ns).then(|| {
            let env_digest =
                crate::store::sha(serde_json::to_vec(&env).unwrap_or_default().as_slice());
            crate::store::sha(
                json!({
                    "v": 2,
                    "cwd": cwd_canon.to_string_lossy(),
                    "argv": argv,
                    "timeout": timeout,
                    "env": env_digest,
                    "tree": tree_digest,
                })
                .to_string()
                .as_bytes(),
            )
        })
    });
    if let Some(key) = &cache_key {
        let mut hit = cache.lock().ok().and_then(|mut guard| guard.get(key));
        if hit.is_none()
            && let Some(s) = store
            && let Some(persisted) = load_persisted_precheck(s, key)
        {
            if let Ok(mut guard) = cache.lock() {
                guard.insert(key.clone(), persisted.clone());
            }
            hit = Some(persisted);
        }
        if let Some(mut hit) = hit {
            hit["cached"] = json!(true);
            return Some(hit);
        }
    }

    let path_env = std::env::var("PATH").ok();
    let wrapped = if infra_host::sandbox::available() {
        let spec = infra_host::sandbox::Spec {
            workspace: cwd_canon.clone(),
            target: cwd_canon.clone(),
            protected: vec![],
            readable: vec![],
            accounts: vec![],
            runtime: infra_host::sandbox::runtime_dirs_for_argv(&argv, path_env.as_deref()),
            codex: None,
            evidence: None,
            network: false,
        };
        infra_host::sandbox::gate(&spec, &argv).ok()?
    } else {
        argv.clone()
    };
    let cap = infra_skillcache::tool::DIAGNOSTIC_BYTES;
    let (ok, code, raw, err_msg): (bool, Option<i32>, String, Option<String>) =
        if let Some(runner) = exec {
            match runner(&wrapped, &cwd_canon, &env) {
                Ok((exit_code, output)) => (exit_code == 0, Some(exit_code), output, None),
                Err(err) => (false, None, String::new(), Some(err)),
            }
        } else {
            let out = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
            let sink = out.clone();
            let cancel = Arc::new(AtomicBool::new(false));
            let res = infra_host::process::run_raw_with_lock(
                &wrapped,
                &cwd_canon,
                &env,
                &[],
                infra_host::process::Limits {
                    seconds: Some(timeout),
                    output: infra_skillcache::tool::STDOUT_BYTES,
                    line: infra_skillcache::tool::STDOUT_BYTES,
                },
                cancel,
                None,
                move |chunk| {
                    let mut buf = sink.lock().map_err(|_| {
                        infra_host::HarnessError::unavailable("Precheck buffer lock failed")
                    })?;
                    let left = infra_skillcache::tool::STDOUT_BYTES.saturating_sub(buf.len());
                    buf.extend_from_slice(&chunk[..chunk.len().min(left)]);
                    Ok(())
                },
            )
            .await;
            let raw = out
                .lock()
                .map(|g| String::from_utf8_lossy(&g).into_owned())
                .unwrap_or_default();
            let ok = res.as_ref().is_ok_and(|o| o.code == 0);
            let code = res.as_ref().ok().map(|o| o.code);
            let err_msg = res.err().map(|e| e.message);
            (ok, code, raw, err_msg)
        };
    let diagnostic_input = if raw.trim().is_empty() {
        err_msg.as_deref().unwrap_or("")
    } else {
        raw.trim()
    };
    let masked = infra_core::domain::mask_secrets(diagnostic_input)
        .map(|(t, _)| t)
        .unwrap_or_else(|_| "[redacted]".into());
    let mut end = masked.len().min(cap);
    while end > 0 && !masked.is_char_boundary(end) {
        end -= 1;
    }
    let summary = masked[..end].to_owned();
    let record = json!({
        "ok": ok,
        "exit_code": code,
        "summary": summary,
        "deterministic": true,
        "cached": false,
        "usage_components": {
            "input_tokens": 0u64,
            "output_tokens": 0u64,
            "cache_read_input_tokens": 0u64,
            "cache_write_5m_input_tokens": 0u64,
            "cache_write_1h_input_tokens": 0u64,
            "model_calls": 0u64,
            "source": "deterministic"
        }
    });
    // Cache deterministic passes AND deterministic non-zero exits (`code.is_some()`, excluding
    // transient infrastructure/timeout/spawn errors where `code` is `None`).
    let deterministic = ok || (code.is_some() && err_msg.is_none());
    if deterministic && let Some(key) = cache_key {
        if let Ok(mut guard) = cache.lock() {
            guard.insert(key.clone(), record.clone());
        }
        if let Some(s) = store {
            save_persisted_precheck(s, &key, &record);
        }
    }
    Some(record)
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

    #[test]
    fn strict_payload_rejects_unstructured_prose_with_trailing_brace_and_accepts_fenced_json() {
        // Heuristic backward brace scanning would have accepted trailing JSON inside prose;
        // strict parser rejects mixed prose unless enclosed in a dedicated JSON block/fenced object.
        let mixed_prose =
            "I inspected things. Here is an informal note {\"verdict\":\"accept\"} in prose.";
        assert!(parse_strict_payload(mixed_prose, &["verdict"]).is_err());
        assert_eq!(extract(mixed_prose, &["verdict"]), None);

        let fenced = "```json\n{\"verdict\":\"accept\",\"objections\":[],\"resolved\":[1]}\n```";
        let ok = parse_strict_payload(fenced, &["verdict"]).unwrap();
        assert_eq!(ok["verdict"], "accept");

        let bad_verdict = r#"{"verdict":"maybe","objections":[]}"#;
        assert!(parse_strict_payload(bad_verdict, &["verdict"]).is_err());

        let mut acc = json!({});
        let t1 = json!({"input_tokens": 100, "output_tokens": 20, "cache_read_input_tokens": 40, "model_calls": 1});
        let t2 = json!({"input_tokens": 50, "output_tokens": 10, "cache_write_5m_input_tokens": 30, "model_calls": 1});
        merge_usage_components(&mut acc, &t1);
        merge_usage_components(&mut acc, &t2);
        assert_eq!(acc["input_tokens"], 150);
        assert_eq!(acc["output_tokens"], 30);
        assert_eq!(acc["cache_read_input_tokens"], 40);
        assert_eq!(acc["cache_write_5m_input_tokens"], 30);
        assert_eq!(acc["model_calls"], 2);
    }

    #[test]
    fn clash_validate_accepts_precheck_and_rejects_invalid_precheck() {
        let valid = validate(
            &json!({
                "challenger": "codex",
                "rounds": 2,
                "precheck": {"argv": ["php", "-l", "src/App.php"], "timeout_seconds": 15}
            }),
            "native",
            "claude",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            valid["precheck"]["argv"],
            json!(["php", "-l", "src/App.php"])
        );
        assert_eq!(valid["precheck"]["timeout_seconds"], 15);

        let defaulted = validate(
            &json!({
                "challenger": "codex",
                "precheck": {"argv": ["cargo", "check"]}
            }),
            "review",
            "claude",
        )
        .unwrap()
        .unwrap();
        assert_eq!(defaulted["precheck"]["timeout_seconds"], 30);

        for bad in [
            json!({"challenger": "codex", "precheck": {"argv": []}}),
            json!({"challenger": "codex", "precheck": {"argv": [""]}}),
            json!({"challenger": "codex", "precheck": {"argv": ["ok"], "timeout_seconds": 0}}),
            json!({"challenger": "codex", "precheck": {"argv": ["ok"], "timeout_seconds": 121}}),
            json!({"challenger": "codex", "precheck": {"argv": ["ok"], "extra": true}}),
        ] {
            assert!(validate(&bad, "native", "claude").is_err(), "{bad}");
        }
    }

    #[test]
    fn validate_clash_schema_enforces_item_level_contracts_without_breaking_valid_payloads() {
        // Valid implement-stage challenge (including id-only maintenance + resolved IDs).
        assert!(
            parse_strict_payload(
                r#"{"verdict":"reject","objections":[{"id":1},{"severity":"high","file":"a.php","line":10,"claim":"Null deref","evidence":"line 10"}],"resolved":[2]}"#,
                &["verdict"],
            )
            .is_ok()
        );
        // Valid review-stage payload.
        assert!(
            parse_strict_payload(
                r#"{"verdict":"accept","assessments":[{"id":1,"stance":"agree","evidence":"verified"}],"responses":[{"id":2,"action":"defend","note":"valid"}],"additional":[{"severity":"low","file":"b.php","line":3,"claim":"Check","evidence":"e"}]}"#,
                &["verdict"],
            )
            .is_ok()
        );
        // Invalid item-level shapes rejected cleanly.
        for bad in [
            r#"{"verdict":null,"objections":[]}"#,
            r#"{"verdict":"accept","resolved":["bad"]}"#,
            r#"{"verdict":"reject","objections":[{"severity":"bogus","claim":"x"}]}"#,
            r#"{"verdict":"reject","objections":[{"file":"a.php"}]}"#,
            r#"{"findings":[{"severity":"high","file":"a.php"}]}"#,
            r#"{"responses":[{"id":1,"action":"ignored"}]}"#,
            r#"{"assessments":[{"id":1,"stance":"unsure"}]}"#,
        ] {
            assert!(
                validate_clash_schema(&serde_json::from_str(bad).unwrap()).is_err(),
                "{bad}"
            );
        }
    }

    fn echo_argv(message: &str) -> Value {
        if cfg!(windows) {
            json!(["cmd", "/C", format!("echo {message}")])
        } else {
            json!(["/bin/sh", "-c", format!("echo {message}")])
        }
    }

    #[tokio::test]
    async fn run_clash_precheck_reads_project_path_and_falls_back_to_workspace_path() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&dir.path().join("state")).unwrap();
        let ws = infra_core::fs::canonicalize(dir.path()).unwrap();
        let missing = ws.join("does-not-exist");

        // Persisted `SessionStore` rows carry `project_path` (no `_workspace_path`).
        let persisted_session = json!({
            "project_path": ws,
            "_workspace_path": missing,
            "clash": {
                "challenger": "codex",
                "precheck": {"argv": echo_argv("clash-precheck-project-path"), "timeout_seconds": 10}
            }
        });
        assert_eq!(precheck_workspace(&persisted_session), ws.as_path());
        let (_summary, record) = run_clash_precheck(&store, &persisted_session).await.expect(
            "precheck should resolve persisted project_path even when _workspace_path is missing",
        );
        assert_eq!(record["deterministic"], true);
        assert_eq!(record["usage_components"]["model_calls"], 0);
        assert_eq!(record["usage_components"]["source"], "deterministic");

        // Fallback to transient `_workspace_path` when `project_path` is absent/empty.
        let transient_session = json!({
            "project_path": "",
            "_workspace_path": ws,
            "clash": {
                "challenger": "codex",
                "precheck": {"argv": echo_argv("clash-precheck-fallback"), "timeout_seconds": 10}
            }
        });
        assert_eq!(precheck_workspace(&transient_session), ws.as_path());
        let (_fallback_summary, fallback_record) = run_clash_precheck(&store, &transient_session)
            .await
            .expect("precheck should fall back to _workspace_path");
        assert_eq!(fallback_record["deterministic"], true);

        // Missing or empty paths return None cleanly.
        let missing_session = json!({
            "project_path": missing,
            "clash": {
                "challenger": "codex",
                "precheck": {"argv": echo_argv("unreachable"), "timeout_seconds": 10}
            }
        });
        assert!(run_clash_precheck(&store, &missing_session).await.is_none());
    }

    #[test]
    fn workflow_health_ledger_tracks_schema_errors_and_emits_roi_warnings() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&dir.path().join("state")).unwrap();
        for i in 0..4 {
            record_workflow_health(
                &store,
                "fleet",
                "codex",
                Some("gpt-5"),
                &format!("run-{i}"),
                if i < 2 { "schema_error" } else { "completed" },
                i < 2,
                i < 2,
                Some(0.05),
                1.2,
            );
        }
        let summary = summarize_workflow_health(&store, "fleet", "codex", Some("gpt-5"));
        assert_eq!(summary.total_runs, 4);
        assert_eq!(summary.schema_error_runs, 2);
        assert_eq!(summary.deopt_runs, 2);
        assert!(
            summary.schema_error_rate > 0.25 && summary.schema_error_rate < 0.45,
            "smoothed recency rate after 2 old failures + 2 passes should be moderated: {}",
            summary.schema_error_rate
        );
        assert!(
            summary
                .warnings
                .iter()
                .any(|w| w.contains("schema error rate"))
        );
        assert!(summary.warnings.iter().any(|w| w.contains("ROI/deopt")));

        // Compare against recent failures (2 old passes then 2 recent failures) -> strictly higher!
        for i in 0..4 {
            record_workflow_health(
                &store,
                "fleet",
                "claude",
                Some("sonnet"),
                &format!("run-recent-{i}"),
                if i >= 2 { "schema_error" } else { "completed" },
                i >= 2,
                i >= 2,
                Some(0.05),
                1.2,
            );
        }
        let recent_fail = summarize_workflow_health(&store, "fleet", "claude", Some("sonnet"));
        assert!(
            recent_fail.schema_error_rate > summary.schema_error_rate + 0.08,
            "recent failures ({}) must weigh more heavily than stale failures ({})",
            recent_fail.schema_error_rate,
            summary.schema_error_rate
        );
        assert_eq!(recent_fail.recommended_clash_rounds(3), 1);
        assert_eq!(recent_fail.recommended_fleet_concurrency(4), 1);

        // Recovery streak clears warnings and restores full rounds & concurrency.
        for i in 0..10 {
            record_workflow_health(
                &store,
                "fleet",
                "claude",
                Some("sonnet"),
                &format!("run-recover-{i}"),
                "completed",
                false,
                false,
                Some(0.02),
                0.9,
            );
        }
        let recovered = summarize_workflow_health(&store, "fleet", "claude", Some("sonnet"));
        assert!(recovered.warnings.is_empty(), "{:?}", recovered.warnings);
        assert_eq!(recovered.recommended_clash_rounds(3), 3);
        assert_eq!(recovered.recommended_fleet_concurrency(4), 4);
    }

    #[tokio::test]
    async fn run_precheck_sandboxed_reuses_racy_ns_settled_receipt_and_invalidates_on_change() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let file = ws.join("app.php");
        std::fs::write(&file, "<?php\n").unwrap();

        let precheck = json!({
            "argv": echo_argv("precheck-ok"),
            "timeout_seconds": 10
        });
        let (_digest, max_mtime) =
            precheck_workspace_stamp(&infra_core::fs::canonicalize(&ws).unwrap()).unwrap();

        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_ref = calls.clone();
        let run_local =
            move |cmd: &[String],
                  cwd: &std::path::Path,
                  env: &std::collections::BTreeMap<String, String>| {
                calls_ref.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let out = std::process::Command::new(&cmd[0])
                    .args(&cmd[1..])
                    .current_dir(cwd)
                    .env_clear()
                    .envs(env)
                    .output()
                    .map_err(|e| e.to_string())?;
                let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
                if !out.stderr.is_empty() {
                    text.push_str(&String::from_utf8_lossy(&out.stderr));
                }
                Ok((out.status.code().unwrap_or(-1), text))
            };

        // Inside 2s RACY_NS window: never hits or populates cache.
        let racy_rec = run_precheck_sandboxed_at(
            None,
            &ws,
            &precheck,
            max_mtime + PRECHECK_RACY_NS - 1,
            Some(&run_local),
        )
        .await
        .unwrap();
        assert_eq!(racy_rec["ok"], true);
        assert_eq!(racy_rec["cached"], false);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

        // Outside RACY_NS window: first run computes and caches, second hits cache.
        let settled_now = max_mtime + PRECHECK_RACY_NS + 100;
        let first = run_precheck_sandboxed_at(None, &ws, &precheck, settled_now, Some(&run_local))
            .await
            .unwrap();
        assert_eq!(first["ok"], true);
        assert_eq!(first["cached"], false);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        let second =
            run_precheck_sandboxed_at(None, &ws, &precheck, settled_now + 50, Some(&run_local))
                .await
                .unwrap();
        assert_eq!(second["cached"], true);
        assert_eq!(second["summary"], first["summary"]);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);

        // Updating workspace file invalidates cached precheck cleanly.
        std::fs::write(&file, "<?php\necho 1;\n").unwrap();
        let (_, new_mtime) =
            precheck_workspace_stamp(&infra_core::fs::canonicalize(&ws).unwrap()).unwrap();
        let after_edit = run_precheck_sandboxed_at(
            None,
            &ws,
            &precheck,
            new_mtime + PRECHECK_RACY_NS + 100,
            Some(&run_local),
        )
        .await
        .unwrap();
        assert_eq!(after_edit["ok"], true);
        assert_eq!(after_edit["cached"], false);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn precheck_persists_deterministic_failures_in_sqlite_and_scopes_stamps() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&dir.path().join("state")).unwrap();
        let ws = dir.path().join("ws-scoped");
        std::fs::create_dir_all(ws.join("src")).unwrap();
        let target = ws.join("src/target.php");
        let unrelated = ws.join("src/unrelated.php");
        std::fs::write(&target, "<?php broken(").unwrap();
        std::fs::write(&unrelated, "<?php echo 1;").unwrap();

        let argv = vec!["php".into(), "-l".into(), "src/target.php".into()];
        let ws_canon = infra_core::fs::canonicalize(&ws).unwrap();
        let (digest_before, max_mtime, strategy) =
            precheck_workspace_stamp_scoped(&ws_canon, &argv, Some(&store)).unwrap();
        assert_eq!(strategy, PrecheckStampStrategy::ScopedFiles);

        // Editing an unrelated sibling file must NOT perturb the scoped stamp for `php -l src/target.php`.
        std::fs::write(&unrelated, "<?php echo 2;").unwrap();
        let (digest_after_unrelated, _, _) =
            precheck_workspace_stamp_scoped(&ws_canon, &argv, Some(&store)).unwrap();
        assert_eq!(digest_before, digest_after_unrelated);

        // Editing the target file DOES invalidate the scoped stamp.
        std::fs::write(&target, "<?php still broken(").unwrap();
        let (digest_after_target, max_mtime2, _) =
            precheck_workspace_stamp_scoped(&ws_canon, &argv, Some(&store)).unwrap();
        assert_ne!(digest_before, digest_after_target);

        let fail_argv = json!({
            "argv": ["php", "-l", "src/target.php"],
            "timeout_seconds": 10
        });
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_ref = calls.clone();
        let fail_det =
            move |_cmd: &[String],
                  _cwd: &std::path::Path,
                  _env: &std::collections::BTreeMap<String, String>| {
                calls_ref.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok((255, "Parse error: syntax error in src/target.php".into()))
            };
        let settled = max_mtime.max(max_mtime2) + PRECHECK_RACY_NS + 500;
        let first_fail =
            run_precheck_sandboxed_at(Some(&store), &ws, &fail_argv, settled, Some(&fail_det))
                .await
                .unwrap();
        assert_eq!(first_fail["ok"], false);
        assert_eq!(first_fail["exit_code"], 255);
        assert_eq!(first_fail["cached"], false);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

        // Second call (including after L1 eviction) hits deterministic failure cache in SQLite!
        let second_fail = run_precheck_sandboxed_at(
            Some(&store),
            &ws,
            &fail_argv,
            settled + 100,
            Some(&fail_det),
        )
        .await
        .unwrap();
        assert_eq!(second_fail["ok"], false);
        assert_eq!(second_fail["exit_code"], 255);
        assert_eq!(second_fail["cached"], true);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

        // Transient infrastructure error (`Err(...)`, `code: None`) is NEVER cached.
        let calls_err = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_err_ref = calls_err.clone();
        let transient_err =
            move |_cmd: &[String],
                  _cwd: &std::path::Path,
                  _env: &std::collections::BTreeMap<String, String>| {
                calls_err_ref.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err("sandbox timed out".to_owned())
            };
        let transient_pre = json!({
            "argv": ["php", "-l", "src/unrelated.php"],
            "timeout_seconds": 10
        });
        let r1 = run_precheck_sandboxed_at(
            Some(&store),
            &ws,
            &transient_pre,
            settled,
            Some(&transient_err),
        )
        .await
        .unwrap();
        let r2 = run_precheck_sandboxed_at(
            Some(&store),
            &ws,
            &transient_pre,
            settled + 10,
            Some(&transient_err),
        )
        .await
        .unwrap();
        assert_eq!(r1["ok"], false);
        assert!(r1["exit_code"].is_null());
        assert_eq!(r2["cached"], false);
        assert_eq!(calls_err.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn clash_prelaunch_health_clamps_excessive_rounds_and_emits_warnings() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&dir.path().join("state")).unwrap();
        for i in 0..4 {
            record_workflow_health(
                &store,
                "clash",
                "codex",
                None,
                &format!("c-{i}"),
                "schema_error",
                true,
                true,
                None,
                1.0,
            );
        }
        let session = json!({
            "id": "s-1",
            "provider": "codex",
            "model": null,
            "thinking_effort": null,
            "clash": {
                "challenger": "claude",
                "rounds": 3,
                "challenger_model": null,
                "challenger_thinking_effort": null
            }
        });
        let mut state = new_state(&session, "review", None);
        assert_eq!(state["rounds"], 3);
        let warnings = apply_prelaunch_health_policy(&store, &mut state, "codex", "claude");
        assert_eq!(state["rounds"], 1);
        assert!(
            warnings.iter().any(|w| w.contains("Clamped Clash rounds")),
            "{warnings:?}"
        );
        assert!(
            warnings.iter().any(|w| w.contains("schema error rate")),
            "{warnings:?}"
        );
    }
}
