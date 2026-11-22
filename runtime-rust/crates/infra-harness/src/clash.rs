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
        let mut header = format!(
            "Two-provider Clash, cycle {}. No additional agents. One human-reviewed cycle only. Task: {}\nFollow-up: {}\n",
            state["cycle"],
            state["task"].as_str().unwrap_or(""),
            state["followup"].as_str().unwrap_or("")
        );
        if let Some((pre_summary, pre_record)) = run_clash_precheck(&self.session).await {
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
                    self.persist(&state)?;
                    return self.incomplete(
                        state,
                        &format!("Reviewer A did not return structured findings: {reason}"),
                    );
                }
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

async fn run_clash_precheck(session: &Value) -> Option<(String, Value)> {
    let precheck = session
        .get("clash")
        .and_then(|c| c.get("precheck"))
        .filter(|p| !p.is_null())?;
    let cwd = precheck_workspace(session);
    let record = run_precheck_sandboxed(cwd, precheck).await?;
    let summary = record["summary"].as_str().unwrap_or("").to_owned();
    Some((summary, record))
}

/// Runs a configured deterministic precheck command inside a network-off `Profile::Gate`
/// sandbox with scrubbed env and zero-token attribution.
pub(crate) async fn run_precheck_sandboxed(
    workspace: &std::path::Path,
    precheck: &Value,
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
    let out = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let sink = out.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let timeout = precheck["timeout_seconds"]
        .as_u64()
        .unwrap_or(30)
        .clamp(1, 120);
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
    let diagnostic_input = if raw.trim().is_empty() {
        match &res {
            Err(err) => err.message.as_str(),
            Ok(_) => "",
        }
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
    let ok = res.as_ref().is_ok_and(|o| o.code == 0);
    let code = res.as_ref().ok().map(|o| o.code);
    Some(json!({
        "ok": ok,
        "exit_code": code,
        "summary": summary,
        "deterministic": true,
        "usage_components": {
            "input_tokens": 0u64,
            "output_tokens": 0u64,
            "cache_read_input_tokens": 0u64,
            "cache_write_5m_input_tokens": 0u64,
            "cache_write_1h_input_tokens": 0u64,
            "model_calls": 0u64,
            "source": "deterministic"
        }
    }))
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

    #[tokio::test]
    async fn run_clash_precheck_reads_project_path_and_falls_back_to_workspace_path() {
        let dir = tempfile::tempdir().unwrap();
        let ws = infra_core::fs::canonicalize(dir.path()).unwrap();
        let missing = ws.join("does-not-exist");

        // Persisted `SessionStore` rows carry `project_path` (no `_workspace_path`).
        let persisted_session = json!({
            "project_path": ws,
            "_workspace_path": missing,
            "clash": {
                "challenger": "codex",
                "precheck": {"argv": ["/bin/sh", "-c", "echo clash-precheck-project-path"], "timeout_seconds": 10}
            }
        });
        assert_eq!(precheck_workspace(&persisted_session), ws.as_path());
        let (_summary, record) = run_clash_precheck(&persisted_session).await.expect(
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
                "precheck": {"argv": ["/bin/sh", "-c", "echo clash-precheck-fallback"], "timeout_seconds": 10}
            }
        });
        assert_eq!(precheck_workspace(&transient_session), ws.as_path());
        let (_fallback_summary, fallback_record) = run_clash_precheck(&transient_session)
            .await
            .expect("precheck should fall back to _workspace_path");
        assert_eq!(fallback_record["deterministic"], true);

        // Missing or empty paths return None cleanly.
        let missing_session = json!({
            "project_path": missing,
            "clash": {
                "challenger": "codex",
                "precheck": {"argv": ["/bin/sh", "-c", "echo unreachable"], "timeout_seconds": 10}
            }
        });
        assert!(run_clash_precheck(&missing_session).await.is_none());
    }
}
