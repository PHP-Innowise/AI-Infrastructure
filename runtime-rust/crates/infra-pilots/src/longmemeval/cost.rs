use super::usage::{LIMIT, Usage};
use crate::strict;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;
const PHASES: [&str; 4] = ["build", "query", "reader", "judge"];
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Succeeded,
    Failed,
    Cancelled,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub outcome: Outcome,
    pub usage: Usage,
    pub usd_micros: Option<u64>,
    pub billing: String,
    pub component_ns: Option<u64>,
}
#[derive(Clone, PartialEq, Eq)]
pub struct Ticket {
    owner: Uuid,
    key: String,
}
impl std::fmt::Debug for Ticket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<opaque memory cost ticket>")
    }
}
#[derive(Clone, PartialEq, Eq)]
pub struct Close {
    pub coverage: String,
    pub model_work: String,
    pub billing_complete: bool,
    pub component_ns: Option<u64>,
    pub constructed_bytes: Option<u64>,
}
#[derive(Default)]
struct Unit {
    keys: Vec<String>,
    close: Option<Close>,
    conflict: bool,
    close_conflict: bool,
}
struct Record {
    phase: String,
    ordinal: Option<usize>,
    report: Option<Report>,
    usage_valid: bool,
    conflict: bool,
}
pub struct Ledger {
    horizon: usize,
    owner: Uuid,
    secret: [u8; 32],
    records: BTreeMap<String, Record>,
    units: BTreeMap<(String, Option<usize>), Unit>,
    grades: BTreeMap<usize, Option<bool>>,
    grade_conflicts: Vec<usize>,
    events: usize,
    duplicates: usize,
    errors: usize,
    overflow: bool,
    invalid: bool,
    frozen: bool,
    attempt_limit: usize,
    event_limit: usize,
}
pub fn hmac(key: &[u8], body: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    let mut outer = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(body);
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    infra_core::domain::hex_lower(outer.finalize())
}
impl Ledger {
    pub fn new(horizon: usize) -> Result<Self> {
        Self::with_limits(horizon, 4096, 65536)
    }
    pub fn with_limits(horizon: usize, attempt_limit: usize, event_limit: usize) -> Result<Self> {
        if !(1..=512).contains(&horizon)
            || !(1..=4096).contains(&attempt_limit)
            || !(1..=65536).contains(&event_limit)
        {
            bail!("ledger_bounds");
        }
        let mut secret = [0; 32];
        secret[..16].copy_from_slice(Uuid::new_v4().as_bytes());
        secret[16..].copy_from_slice(Uuid::new_v4().as_bytes());
        let mut units = BTreeMap::new();
        for phase in PHASES {
            if phase == "build" {
                units.insert((phase.into(), None), Unit::default());
            } else {
                for ordinal in 0..horizon {
                    units.insert((phase.into(), Some(ordinal)), Unit::default());
                }
            }
        }
        Ok(Self {
            horizon,
            owner: Uuid::new_v4(),
            secret,
            records: BTreeMap::new(),
            units,
            grades: BTreeMap::new(),
            grade_conflicts: vec![],
            events: 0,
            duplicates: 0,
            errors: 0,
            overflow: false,
            invalid: false,
            frozen: false,
            attempt_limit,
            event_limit,
        })
    }
    fn fail<T>(&mut self, code: &str) -> Result<T> {
        self.errors = (self.errors + 1).min(65536);
        self.invalid = true;
        bail!("{code}")
    }
    fn tick(&mut self) -> Result<()> {
        if self.frozen {
            bail!("ledger_frozen");
        }
        if self.events >= self.event_limit {
            self.overflow = true;
            return self.fail("event_capacity");
        }
        self.events += 1;
        Ok(())
    }
    pub fn begin_call(
        &mut self,
        phase: &str,
        ordinal: Option<usize>,
        call_id: &str,
        attempt_id: &str,
    ) -> Result<Ticket> {
        self.tick()?;
        let unit = (phase.to_owned(), ordinal);
        if !self.units.contains_key(&unit)
            || [call_id, attempt_id]
                .iter()
                .any(|s| s.is_empty() || s.len() > 1024)
        {
            return self.fail("ledger_scope");
        }
        let key = hmac(&self.secret, &strict::wire(&json!([call_id, attempt_id]))?);
        if let Some(old) = self.records.get_mut(&key) {
            if old.phase != phase || old.ordinal != ordinal {
                old.conflict = true;
                return self.fail("attempt_binding");
            }
            self.duplicates += 1;
            return Ok(Ticket {
                owner: self.owner,
                key,
            });
        }
        if self.records.len() >= self.attempt_limit {
            self.overflow = true;
            return self.fail("attempt_capacity");
        }
        let u = self.units.get_mut(&unit).unwrap();
        if u.close.is_some() {
            u.conflict = true;
            return self.fail("unit_closed");
        }
        u.keys.push(key.clone());
        self.records.insert(
            key.clone(),
            Record {
                phase: phase.into(),
                ordinal,
                report: None,
                usage_valid: true,
                conflict: false,
            },
        );
        Ok(Ticket {
            owner: self.owner,
            key,
        })
    }
    pub fn finish_call(&mut self, ticket: &Ticket, mut report: Report) -> Result<()> {
        self.tick()?;
        if ticket.owner != self.owner || !self.records.contains_key(&ticket.key) {
            return self.fail("ticket_binding");
        }
        let usage_valid = report.usage.valid();
        if !usage_valid {
            report.usage = Usage::default();
            self.errors = (self.errors + 1).min(65536);
        }
        if report.usd_micros.is_some_and(|v| v > LIMIT) {
            report.usd_micros = None;
            self.errors = (self.errors + 1).min(65536);
        }
        if report.component_ns.is_some_and(|v| v > LIMIT) {
            report.component_ns = None;
            self.errors = (self.errors + 1).min(65536);
        }
        if !["unknown", "provider_reported", "invoice_allocated"].contains(&report.billing.as_str())
        {
            report.billing = "unknown".into();
            self.errors = (self.errors + 1).min(65536);
        }
        let scope = {
            let r = &self.records[&ticket.key];
            (r.phase.clone(), r.ordinal)
        };
        let closed = self.units[&scope].close.is_some();
        let r = self.records.get_mut(&ticket.key).unwrap();
        if let Some(old) = &r.report {
            if old == &report && r.usage_valid == usage_valid && !r.conflict {
                self.duplicates += 1;
                return Ok(());
            }
            r.conflict = true;
            if closed {
                self.units.get_mut(&scope).unwrap().conflict = true;
                return self.fail("late_report");
            }
            return Ok(());
        }
        if closed {
            r.conflict = true;
            self.units.get_mut(&scope).unwrap().conflict = true;
            return self.fail("late_report");
        }
        r.report = Some(report);
        r.usage_valid = usage_valid;
        Ok(())
    }
    pub fn close_unit(&mut self, phase: &str, ordinal: Option<usize>, close: Close) -> Result<()> {
        self.tick()?;
        if !["complete", "incomplete"].contains(&close.coverage.as_str())
            || !["none_observed_local", "reported_calls", "unknown"]
                .contains(&close.model_work.as_str())
            || close.component_ns.is_some_and(|v| v > LIMIT)
            || close.constructed_bytes.is_some_and(|v| v > LIMIT)
        {
            return self.fail("close");
        }
        let Some(u) = self.units.get_mut(&(phase.into(), ordinal)) else {
            return self.fail("ledger_scope");
        };
        if let Some(old) = &u.close {
            if old == &close {
                self.duplicates += 1;
                return Ok(());
            }
            u.close_conflict = true;
            u.conflict = true;
            return Ok(());
        }
        if close.model_work == "none_observed_local"
            && (!["build", "query"].contains(&phase) || !u.keys.is_empty())
        {
            u.conflict = true;
            return self.fail("no_model_conflict");
        }
        if close.model_work == "reported_calls" && u.keys.is_empty() {
            u.conflict = true;
            return self.fail("missing_reported_calls");
        }
        u.close = Some(close);
        Ok(())
    }
    pub fn grade(&mut self, ordinal: usize, correct: Option<bool>) -> Result<()> {
        self.tick()?;
        if ordinal >= self.horizon {
            return self.fail("grade_scope");
        }
        if let Some(old) = self.grades.get(&ordinal) {
            if old == &correct && !self.grade_conflicts.contains(&ordinal) {
                self.duplicates += 1;
                return Ok(());
            }
            if !self.grade_conflicts.contains(&ordinal) {
                self.grade_conflicts.push(ordinal);
            }
            return Ok(());
        }
        self.grades.insert(ordinal, correct);
        Ok(())
    }
    fn metrics(&self, phases: &[&str]) -> Value {
        let units = self
            .units
            .iter()
            .filter(|((p, _), _)| phases.contains(&p.as_str()))
            .map(|(_, u)| u)
            .collect::<Vec<_>>();
        let records = units
            .iter()
            .flat_map(|u| u.keys.iter().map(|k| &self.records[k]))
            .collect::<Vec<_>>();
        let reports = records
            .iter()
            .filter(|r| !r.conflict)
            .filter_map(|r| r.report.as_ref())
            .collect::<Vec<_>>();
        let usable = records
            .iter()
            .filter(|r| !r.conflict && r.usage_valid)
            .filter_map(|r| r.report.as_ref())
            .collect::<Vec<_>>();
        let paired = usable
            .iter()
            .filter(|r| r.usage.input_tokens.is_some() && r.usage.output_tokens.is_some())
            .collect::<Vec<_>>();
        let coverage = !self.overflow
            && !self.invalid
            && units.iter().all(|u| {
                !u.conflict
                    && u.close
                        .as_ref()
                        .is_some_and(|c| c.coverage == "complete" && c.model_work != "unknown")
            })
            && records.iter().all(|r| r.report.is_some() && !r.conflict);
        let sum = |a: Vec<Option<u64>>| -> Option<u64> {
            let n = a.into_iter().flatten().map(u128::from).sum::<u128>();
            (n <= LIMIT as u128).then_some(n as u64)
        };
        let input = sum(usable.iter().map(|r| r.usage.input_tokens).collect());
        let output = sum(usable.iter().map(|r| r.usage.output_tokens).collect());
        let tokens = sum(paired
            .iter()
            .map(|r| Some(r.usage.input_tokens.unwrap() + r.usage.output_tokens.unwrap()))
            .collect());
        let usd = sum(reports.iter().map(|r| r.usd_micros).collect());
        let timing = sum(reports
            .iter()
            .map(|r| r.component_ns)
            .chain(
                units
                    .iter()
                    .filter(|u| !u.close_conflict)
                    .filter_map(|u| u.close.as_ref().map(|c| c.component_ns)),
            )
            .collect());
        let bytes = sum(units
            .iter()
            .filter(|u| !u.close_conflict)
            .filter_map(|u| u.close.as_ref().map(|c| c.constructed_bytes))
            .collect());
        let cached = sum(paired.iter().map(|r| r.usage.cached_input_tokens).collect());
        let reasoning = sum(paired
            .iter()
            .map(|r| r.usage.reasoning_output_tokens)
            .collect());
        let overflow = [input, output, tokens, usd, timing, bytes, cached, reasoning]
            .iter()
            .any(Option::is_none);
        let tokens_complete = coverage && paired.len() == records.len() && !overflow;
        let cache_complete =
            tokens_complete && paired.iter().all(|r| r.usage.cached_input_tokens.is_some());
        let reasoning_complete = tokens_complete
            && paired
                .iter()
                .all(|r| r.usage.reasoning_output_tokens.is_some());
        let billing = coverage
            && !overflow
            && units
                .iter()
                .all(|u| u.close.as_ref().is_some_and(|c| c.billing_complete))
            && reports
                .iter()
                .all(|r| r.usd_micros.is_some() && r.billing != "unknown");
        let time = coverage
            && !overflow
            && units
                .iter()
                .all(|u| u.close.as_ref().is_some_and(|c| c.component_ns.is_some()))
            && reports.iter().all(|r| r.component_ns.is_some());
        let bytes_complete = coverage
            && !overflow
            && units.iter().all(|u| {
                u.close
                    .as_ref()
                    .is_some_and(|c| c.constructed_bytes.is_some())
            });
        json!({"known_input_tokens":input,"known_output_tokens":output,"known_tokens":tokens,"known_usd_micros":usd,"known_component_ns":timing,"known_constructed_bytes":bytes,"input_tokens":if tokens_complete{input}else{None},"output_tokens":if tokens_complete{output}else{None},"tokens":if tokens_complete{tokens}else{None},"cached_input_tokens":if cache_complete{cached}else{None},"reasoning_output_tokens":if reasoning_complete{reasoning}else{None},"uncached_input_tokens":if cache_complete{input.zip(cached).map(|(i,c)|i-c)}else{None},"usd_micros":if billing{usd}else{None},"component_ns":if time{timing}else{None},"constructed_bytes":if bytes_complete{bytes}else{None},"complete":{"coverage":coverage&&!overflow,"tokens":tokens_complete,"cache":cache_complete,"reasoning":reasoning_complete,"billing":billing,"timing":time,"bytes":bytes_complete},"overflow":overflow})
    }
    pub fn snapshot(&self) -> Result<Value> {
        let mut phases = serde_json::Map::new();
        for p in PHASES {
            let units = self
                .units
                .iter()
                .filter(|((name, _), _)| name == p)
                .map(|(_, u)| u)
                .collect::<Vec<_>>();
            let records = units
                .iter()
                .flat_map(|u| u.keys.iter().map(|k| &self.records[k]))
                .collect::<Vec<_>>();
            let mut outcomes =
                json!({"succeeded":0,"failed":0,"cancelled":0,"pending":0,"conflict":0});
            for r in &records {
                let k = if r.conflict {
                    "conflict"
                } else {
                    match r.report.as_ref().map(|r| r.outcome) {
                        None => "pending",
                        Some(Outcome::Succeeded) => "succeeded",
                        Some(Outcome::Failed) => "failed",
                        Some(Outcome::Cancelled) => "cancelled",
                    }
                };
                outcomes[k] = json!(outcomes[k].as_u64().unwrap() + 1);
            }
            let mut v = self.metrics(&[p]);
            v["expected_units"] = json!(units.len());
            v["closed_units"] = json!(units.iter().filter(|u| u.close.is_some()).count());
            v["attempts"] = json!(records.len());
            v["finished_attempts"] = json!(records.iter().filter(|r| r.report.is_some()).count());
            v["conflicting_attempts"] = json!(records.iter().filter(|r| r.conflict).count());
            v["outcomes"] = outcomes;
            phases.insert(p.into(), v);
        }
        let grades = self
            .grades
            .iter()
            .filter(|(o, _)| !self.grade_conflicts.contains(o))
            .filter_map(|(_, v)| *v)
            .collect::<Vec<_>>();
        let complete = grades.len() == self.horizon;
        let correct = complete.then(|| grades.iter().filter(|v| **v).count());
        let mut scopes = BTreeMap::new();
        for (name, included) in [("product", &PHASES[..3]), ("experiment", &PHASES[..])] {
            let mut v = self.metrics(included);
            for (metric, ratio) in [
                ("tokens", "tokens_per_planned_query"),
                ("usd_micros", "usd_micros_per_planned_query"),
            ] {
                v[ratio] = json!(v[metric].as_f64().map(|n| n / self.horizon as f64));
            }
            for (metric, ratio) in [
                ("tokens", "tokens_per_correct"),
                ("usd_micros", "usd_micros_per_correct"),
            ] {
                v[ratio] = json!(
                    correct
                        .filter(|n| *n > 0)
                        .and_then(|n| v[metric].as_f64().map(|x| x / n as f64))
                );
            }
            scopes.insert(name, v);
        }
        let v = json!({"profile":"memory-benchmark-ledger-v1","usage_profile":"inclusive-input-output-v1","horizon":self.horizon,"attempt_limit":self.attempt_limit,"attempts":self.records.len(),"duplicates":self.duplicates,"observation_errors":self.errors,"overflow":self.overflow||phases.values().any(|v|v["overflow"]==true)||scopes.values().any(|v|v["overflow"]==true),"frozen":self.frozen,"phases":phases,"grading":{"observed":grades.len(),"known_correct":grades.iter().filter(|v|**v).count(),"correct":correct,"complete":complete,"conflicting":self.grade_conflicts.len()},"product":scopes["product"],"experiment":scopes["experiment"]});
        if strict::wire(&v)?.len() > 262144 {
            bail!("snapshot_capacity");
        }
        Ok(v)
    }
    pub fn finalize(&mut self) -> Result<Value> {
        self.frozen = true;
        self.snapshot()
    }
}

pub use super::comparison::compare_incremental;
