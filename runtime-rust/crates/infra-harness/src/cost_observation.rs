//! Worker-local cost observations retain numeric/categorical metadata only.
use crate::native_diagnostics::NativeDiagnostics;
use serde_json::{Value, json as object};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Instant,
};
pub const COUNTERS: &[&str] = &[
    "input_tokens",
    "output_tokens",
    "cached_input_tokens",
    "cache_read_input_tokens",
    "cache_creation_input_tokens",
];
pub fn counter(value: &Value) -> Option<u64> {
    value.as_u64().filter(|value| *value <= i64::MAX as u64)
}
pub fn amount(value: &Value) -> Option<f64> {
    if value.is_i64() || value.is_u64() {
        return counter(value).map(|value| value as f64);
    }
    value
        .as_f64()
        .filter(|value| value.is_finite() && *value >= 0. && *value < 9223372036854775808.)
}
fn money(value: &Value) -> Value {
    if value.is_i64() || value.is_u64() {
        return counter(value).map(|_| value.clone()).unwrap_or(Value::Null);
    }
    amount(value).map(|_| value.clone()).unwrap_or(Value::Null)
}
fn numeric(value: u128) -> Value {
    Value::Number(serde_json::Number::from_string_unchecked(value.to_string()))
}
fn truth(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.),
    }
}
#[derive(Clone)]
pub struct Limits {
    pub events: usize,
    pub calls: usize,
    pub characters_per_payload: usize,
    pub content_blocks: usize,
    pub terminals: usize,
    pub identifier_characters: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            events: 4096,
            calls: 512,
            characters_per_payload: 200000,
            content_blocks: 64,
            terminals: 8,
            identifier_characters: 1024,
        }
    }
}
impl Limits {
    pub fn bounded(value: &Value) -> Option<Self> {
        let mut limits = Self::default();
        for (key, value) in value.as_object()? {
            let limit = match key.as_str() {
                "events" => &mut limits.events,
                "calls" => &mut limits.calls,
                "characters_per_payload" => &mut limits.characters_per_payload,
                "content_blocks" => &mut limits.content_blocks,
                "terminals" => &mut limits.terminals,
                "identifier_characters" => &mut limits.identifier_characters,
                _ => return None,
            };
            let count = usize::try_from(value.as_u64()?).ok()?;
            if count < 1 || count > *limit {
                return None;
            }
            *limit = count;
        }
        Some(limits)
    }
}
#[derive(Clone, PartialEq, Eq)]
struct Signature {
    tool: String,
    scope: String,
    outcome: String,
    coverage: String,
    digest: Option<Vec<u8>>,
}
type ObservationKey = (Vec<u8>, Option<Vec<u8>>);
type SeenObservations = BTreeMap<ObservationKey, (usize, Signature)>;
#[derive(Clone)]
pub struct Phase {
    provider: String,
    id: u32,
    label: String,
    limits: Limits,
    key: [u8; 32],
    expected: Option<Vec<u8>>,
    native: Option<Vec<u8>>,
    native_conflict: bool,
    native_unknown: bool,
    events: usize,
    terminal_count: usize,
    occurrences: u64,
    reports: Vec<Value>,
    calls: Vec<Value>,
    seen: SeenObservations,
    overflow: bool,
    conflict: bool,
    payload_overflow: bool,
    unidentified: u64,
    unverifiable_repeats: u64,
    duplicates: u64,
}
impl Phase {
    pub fn new(
        provider: &str,
        id: u32,
        label: &str,
        expected: Option<&str>,
        limits: Limits,
    ) -> Option<Self> {
        if !["codex", "claude", "cursor"].contains(&provider)
            || !["main", "recovery"].contains(&label)
            || id > 4096
            || limits.events == 0
            || limits.events > 4096
            || limits.calls == 0
            || limits.calls > 512
            || limits.characters_per_payload == 0
            || limits.characters_per_payload > 200000
            || limits.content_blocks == 0
            || limits.content_blocks > 64
            || limits.terminals == 0
            || limits.terminals > 8
            || limits.identifier_characters == 0
            || limits.identifier_characters > 1024
        {
            return None;
        }
        let key = std::panic::catch_unwind(|| {
            let mut key = [0u8; 32];
            key[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
            key[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
            key
        })
        .ok()?;
        let mut phase = Self {
            provider: provider.into(),
            id,
            label: label.into(),
            limits,
            key,
            expected: None,
            native: None,
            native_conflict: false,
            native_unknown: false,
            events: 0,
            terminal_count: 0,
            occurrences: 0,
            reports: vec![],
            calls: vec![],
            seen: BTreeMap::new(),
            overflow: false,
            conflict: false,
            payload_overflow: false,
            unidentified: 0,
            unverifiable_repeats: 0,
            duplicates: 0,
        };
        phase.expected = expected.and_then(|value| phase.opaque(&object!(value)));
        phase.native_unknown = expected.is_some() && phase.expected.is_none();
        Some(phase)
    }
    fn hmac(&self, pieces: &[&[u8]]) -> Vec<u8> {
        let mut inner_pad = [0x36u8; 64];
        let mut outer_pad = [0x5cu8; 64];
        for (index, byte) in self.key.iter().enumerate() {
            inner_pad[index] ^= *byte;
            outer_pad[index] ^= *byte;
        }
        let mut inner = Sha256::new();
        inner.update(inner_pad);
        for piece in pieces {
            inner.update(piece);
        }
        let mut outer = Sha256::new();
        outer.update(outer_pad);
        outer.update(inner.finalize());
        outer.finalize().to_vec()
    }
    fn opaque(&self, value: &Value) -> Option<Vec<u8>> {
        let value = value.as_str().filter(|value| {
            !value.is_empty() && value.chars().count() <= self.limits.identifier_characters
        })?;
        Some(self.hmac(&[value.as_bytes()]))
    }
    fn measure(&mut self, content: &Value) -> (Value, Option<Vec<u8>>) {
        let mut complete = true;
        let mut overflow = false;
        let mut pieces = vec![];
        let prefix: &[u8];
        if let Some(value) = content.as_str() {
            prefix = b"string\0";
            pieces.push(value);
        } else if let Some(blocks) = content.as_array() {
            prefix = b"blocks\0";
            if blocks.len() > self.limits.content_blocks {
                complete = false;
                overflow = true;
            }
            for block in blocks.iter().take(self.limits.content_blocks) {
                if block["type"] == "text" {
                    if let Some(text) = block["text"].as_str() {
                        pieces.push(text);
                    } else {
                        complete = false;
                    }
                } else {
                    complete = false;
                }
            }
        } else {
            return (
                object!({"characters":null,"bytes":null,"known_characters":0,"known_bytes":0,"coverage":"unknown"}),
                None,
            );
        }
        let mut characters = 0usize;
        let mut bytes = 0usize;
        let mut inspected = 0usize;
        let mut chunks = vec![prefix.to_vec()];
        for text in pieces {
            let remaining = self.limits.characters_per_payload.saturating_sub(inspected);
            let count = text.chars().count();
            if count > remaining {
                complete = false;
                overflow = true;
            }
            let text = text.chars().take(remaining).collect::<String>();
            let count = text.chars().count();
            inspected += count;
            characters += count;
            bytes += text.len();
            chunks.push((count as u64).to_be_bytes().to_vec());
            chunks.push(text.into_bytes());
        }
        self.payload_overflow |= overflow;
        let digest = if complete {
            Some(self.hmac(&chunks.iter().map(Vec::as_slice).collect::<Vec<_>>()))
        } else {
            None
        };
        (
            object!({"characters":if complete{Some(characters)}else{None},"bytes":if complete{Some(bytes)}else{None},"known_characters":characters,"known_bytes":bytes,"coverage":if complete{"complete"}else{"partial"}}),
            digest,
        )
    }
    fn tool(
        &mut self,
        identifier: &Value,
        tool: &str,
        content: &Value,
        outcome: &str,
        scope: &str,
        discriminator: Option<&str>,
    ) {
        self.occurrences = self.occurrences.saturating_add(1);
        let mut token = self.opaque(identifier);
        let entry = discriminator.and_then(|value| self.opaque(&object!(value)));
        if discriminator.is_some() && entry.is_none() {
            token = None;
        }
        let key = token.clone().map(|token| (token, entry));
        if key.as_ref().is_none_or(|key| !self.seen.contains_key(key))
            && self.calls.len() >= self.limits.calls
        {
            self.overflow = true;
            return;
        }
        let (metrics, digest) = self.measure(content);
        let signature = Signature {
            tool: tool.into(),
            scope: scope.into(),
            outcome: outcome.into(),
            coverage: metrics["coverage"].as_str().unwrap_or("unknown").into(),
            digest: digest.clone(),
        };
        if let Some(key) = &key
            && let Some((index, old)) = self.seen.get(key)
        {
            if digest.is_some() && old == &signature {
                self.duplicates += 1;
            } else {
                self.conflict = true;
                self.unverifiable_repeats += u64::from(digest.is_none());
                self.calls[*index]["identity_state"] = "conflict".into();
            }
            return;
        }
        let mut record = object!({"tool":tool,"outcome":outcome,"scope":scope,"identified":token.is_some(),"identity_state":if token.is_some(){"single"}else{"unidentified"}});
        if let Some(map) = metrics.as_object() {
            for (key, value) in map {
                record[key] = value.clone();
            }
        }
        if let Some(key) = key {
            self.seen.insert(key, (self.calls.len(), signature));
        } else {
            self.unidentified += 1;
        }
        self.calls.push(record)
    }
    pub fn observe(&mut self, event: &Value) {
        if self.events >= self.limits.events {
            self.overflow = true;
            return;
        }
        self.events += 1;
        if !event.is_object() {
            return;
        }
        let kind = event["type"].as_str().unwrap_or("");
        let field = if self.provider == "codex" {
            "thread_id"
        } else {
            "session_id"
        };
        if let Some(value) = event.get(field) {
            if let Some(identity) = self.opaque(value) {
                if self.native.as_ref().is_some_and(|prior| prior != &identity)
                    || self
                        .expected
                        .as_ref()
                        .is_some_and(|prior| prior != &identity)
                {
                    self.native_conflict = true;
                } else {
                    self.native = Some(identity);
                }
            } else {
                self.native_unknown = true;
            }
        }
        let terminal = if self.provider == "codex" {
            ["turn.completed", "turn.failed"].contains(&kind)
        } else {
            kind == "result"
        };
        if terminal {
            self.terminal_count += 1;
            if self.reports.len() >= self.limits.terminals {
                self.overflow = true;
                return;
            }
            let ok = if self.provider == "codex" {
                kind == "turn.completed" && !truth(&event["error"]) && event["is_error"] != true
            } else {
                event["subtype"] == "success" && event["is_error"] == false
            };
            self.reports.push(object!({"counters":COUNTERS.iter().map(|key|((*key).to_string(),object!(counter(&event["usage"][*key])))).collect::<BTreeMap<_,_>>(),"reported_usd":money(&event["total_cost_usd"]),"outcome":if ok{"success"}else{"failed"}}));
            return;
        }
        if self.provider == "claude" && kind == "user" {
            if let Some(blocks) = event["message"]["content"].as_array() {
                self.payload_overflow |= blocks.len() > self.limits.content_blocks;
                let scope = if event.get("parent_tool_use_id").is_none() {
                    "unknown"
                } else if event["parent_tool_use_id"].is_null() {
                    "main_stream"
                } else {
                    "helper_stream"
                };
                for block in blocks.iter().take(self.limits.content_blocks) {
                    if block.is_object() && block["type"] == "tool_result" {
                        let outcome = if block["is_error"] == true {
                            "failed"
                        } else if block["is_error"] == false {
                            "success"
                        } else {
                            "unknown"
                        };
                        self.tool(
                            &block["tool_use_id"],
                            "tool_result",
                            &block["content"],
                            outcome,
                            scope,
                            None,
                        );
                    }
                }
            }
        } else if self.provider == "codex" && kind == "item.completed" {
            let item = &event["item"];
            let name = item["type"].as_str().unwrap_or("");
            let tool = match name {
                "command_execution" => "shell",
                "mcp_tool_call" => "mcp",
                "file_change" => "edit",
                "web_search" => "search",
                "collab_tool_call" => "helper",
                _ => return,
            };
            let body = if name == "command_execution" {
                &item["aggregated_output"]
            } else if name == "mcp_tool_call" {
                &item["result"]
            } else {
                &Value::Null
            };
            let body = if body.is_string() { body } else { &Value::Null };
            let status = item["status"].as_str().unwrap_or("");
            let failed = item["exit_code"].as_i64().is_some_and(|value| value != 0);
            let outcome = if status == "declined" {
                "not_run"
            } else if status == "failed" || failed {
                "failed"
            } else if status == "completed" {
                "success"
            } else {
                "unknown"
            };
            self.tool(&item["id"], tool, body, outcome, "main_stream", None);
        } else if self.provider == "cursor"
            && kind == "tool_call"
            && event["subtype"] == "completed"
            && let Some(calls) = event["tool_call"].as_object()
        {
            self.payload_overflow |= calls.len() > self.limits.content_blocks;
            for (name, call) in calls.iter().take(self.limits.content_blocks) {
                if !call.is_object() {
                    continue;
                }
                let result = &call["result"];
                let states = ["success", "error", "rejected"]
                    .into_iter()
                    .filter(|key| result.get(*key).is_some())
                    .collect::<Vec<_>>();
                let outcome = if states.len() == 1 {
                    match states[0] {
                        "success" => "success",
                        "error" => "failed",
                        _ => "not_run",
                    }
                } else {
                    "unknown"
                };
                let body = if name == "readToolCall" && result["success"]["content"].is_string() {
                    &result["success"]["content"]
                } else {
                    &Value::Null
                };
                let tool = match name.as_str() {
                    "readToolCall" => "read",
                    "shellToolCall" => "shell",
                    "mcpToolCall" => "mcp",
                    "editToolCall" => "edit",
                    "writeToolCall" => "write",
                    "grepToolCall" => "search",
                    _ => "other",
                };
                self.tool(
                    &event["call_id"],
                    tool,
                    body,
                    outcome,
                    "main_stream",
                    Some(name),
                );
            }
        }
    }
    pub fn snapshot(&self) -> Value {
        let report = if self.terminal_count == 1 && self.reports.len() == 1 {
            self.reports.first()
        } else {
            None
        };
        let fields = if self.provider == "codex" {
            vec!["input_tokens", "output_tokens"]
        } else {
            vec![
                "input_tokens",
                "output_tokens",
                "cache_read_input_tokens",
                "cache_creation_input_tokens",
            ]
        };
        let total = report
            .filter(|_| {
                self.provider != "cursor"
                    && !self.overflow
                    && !self.native_conflict
                    && !self.native_unknown
            })
            .and_then(|report| {
                fields
                    .iter()
                    .map(|key| counter(&report["counters"][*key]).map(u128::from))
                    .collect::<Option<Vec<_>>>()
            })
            .map(|values| values.into_iter().sum::<u128>());
        let complete_text =
            !(self.overflow || self.payload_overflow || self.conflict || self.unidentified > 0)
                && self.calls.iter().all(|call| call["coverage"] == "complete");
        let cache_state = if let Some(report) = report {
            if self.provider == "codex" {
                match (
                    counter(&report["counters"]["input_tokens"]),
                    counter(&report["counters"]["cached_input_tokens"]),
                ) {
                    (Some(input), Some(cached)) => {
                        if cached <= input {
                            "complete"
                        } else {
                            "invalid"
                        }
                    }
                    _ => "unknown",
                }
            } else if self.provider == "claude"
                && counter(&report["counters"]["cache_read_input_tokens"]).is_some()
                && counter(&report["counters"]["cache_creation_input_tokens"]).is_some()
            {
                "complete"
            } else {
                "unknown"
            }
        } else {
            "unknown"
        };
        object!({"schema":"private-cost-core-v1","provider":self.provider,"execution_id":self.id,"phase":self.label,"events":self.events,"overflow":self.overflow,"payload_overflow":self.payload_overflow,"terminal_count":self.terminal_count,"valid_usage_report_count":self.reports.iter().filter(|report|COUNTERS.iter().any(|key|!report["counters"][*key].is_null())).count(),"terminal_reports":self.reports,"provider_tokens":total.map(numeric),"token_state":if total.is_some(){"complete"}else{"incomplete"},"cache_breakdown_state":cache_state,"own_usd":null,"own_usd_state":"unattributed","native_identity_state":if self.native_conflict{"conflict"}else if self.native_unknown{"unknown"}else if self.native.is_some(){"observed"}else{"unobserved"},"tool_occurrences":self.occurrences,"duplicate_completions":self.duplicates,"unidentified_completions":self.unidentified,"unverifiable_repeats":self.unverifiable_repeats,"tool_observations":self.calls,"tool_component_known_bytes":self.calls.iter().filter(|row|row["identity_state"]=="single").map(|row|row["known_bytes"].as_u64().unwrap_or(0)).sum::<u64>(),"tool_component_bytes":if complete_text{Some(self.calls.iter().map(|row|row["bytes"].as_u64().unwrap_or(0)).sum::<u64>())}else{None},"tool_scope":"supported raw text components; excludes wrappers/nontext/unobserved tools; not model tokens"})
    }
}
fn tokens(value: &Value) -> Option<u128> {
    match value {
        Value::Number(value) => value.to_string().parse().ok(),
        _ => None,
    }
}
pub fn episode(phases: &[Phase]) -> Value {
    if phases.len() > 16 {
        return object!({"executions":[],"known_token_subtotal":null,"provider_tokens":null,"token_state":"incomplete","capacity_exceeded":true});
    }
    let snapshots = phases.iter().map(Phase::snapshot).collect::<Vec<_>>();
    let unique = snapshots
        .iter()
        .map(|value| value["execution_id"].as_u64())
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        == snapshots.len();
    let known = if unique {
        Some(
            snapshots
                .iter()
                .filter_map(|value| tokens(&value["provider_tokens"]))
                .sum::<u128>(),
        )
    } else {
        None
    };
    let complete = !snapshots.is_empty()
        && unique
        && snapshots
            .iter()
            .all(|value| !value["provider_tokens"].is_null());
    object!({"executions":snapshots,"known_token_subtotal":known.map(numeric),"provider_tokens":if complete{known.map(numeric)}else{None},"token_state":if complete{"complete"}else{"incomplete"},"capacity_exceeded":false})
}
pub fn text_size(value: &str) -> Value {
    let characters = value.chars().count();
    object!({"characters":characters,"bytes":if characters<=2_000_000{Some(value.len())}else{None},"state":if characters<=2_000_000{"observed"}else{"capacity"}})
}
pub struct LaunchObservation {
    pub provider: String,
    pub supported: bool,
    pub incomplete: bool,
    pub persisted: bool,
    pub stage: String,
    started: Instant,
    phases: BTreeMap<String, Phase>,
    processes: BTreeMap<String, Value>,
    payloads: BTreeMap<String, Value>,
    usd_reports: BTreeMap<String, Vec<Value>>,
    diagnostics: BTreeMap<String, NativeDiagnostics>,
    durations: BTreeMap<String, f64>,
    operations: Vec<Value>,
    notes: BTreeMap<String, u64>,
    remote_operations: Vec<Value>,
    remote_overflow: bool,
    final_value: Option<Value>,
    outcome: String,
}
impl LaunchObservation {
    pub fn new(provider: &str, supported: bool) -> Self {
        Self {
            provider: provider.into(),
            supported,
            incomplete: false,
            persisted: false,
            stage: "prepare".into(),
            started: Instant::now(),
            phases: BTreeMap::new(),
            processes: BTreeMap::new(),
            payloads: BTreeMap::new(),
            usd_reports: BTreeMap::new(),
            diagnostics: BTreeMap::new(),
            durations: BTreeMap::new(),
            operations: vec![],
            notes: BTreeMap::new(),
            remote_operations: vec![],
            remote_overflow: false,
            final_value: None,
            outcome: "unknown".into(),
        }
    }
    pub fn new_phase(&mut self, label: &str, native: Option<&str>) {
        if !self.supported
            || self.final_value.is_some()
            || !["main", "recovery"].contains(&label)
            || self.processes.contains_key(label)
        {
            return;
        }
        self.processes.insert(
            label.into(),
            object!({"attempted":true,"started":false,"state":"attempted"}),
        );
        self.diagnostics
            .insert(label.into(), NativeDiagnostics::new(&self.provider));
        if let Some(phase) = Phase::new(
            &self.provider,
            if label == "main" { 0 } else { 1 },
            label,
            native,
            Limits::default(),
        ) {
            self.phases.insert(label.into(), phase);
        } else {
            self.incomplete = true;
        }
    }
    pub fn diagnostic(&mut self, label: &str, signal: &str, code: Option<i64>) {
        if self.final_value.is_none()
            && let Some(diagnostics) = self.diagnostics.get_mut(label)
        {
            diagnostics.boundary(signal, code);
        }
    }
    pub fn process(&mut self, label: &str, state: &str) {
        if self.final_value.is_none()
            && ["started", "finished", "failed", "cancelled", "skipped"].contains(&state)
            && let Some(process) = self.processes.get_mut(label)
        {
            process["started"] = (process["started"] == true || state == "started").into();
            process["state"] = state.into();
        }
    }
    pub fn raw(&mut self, label: &str, event: &Value) {
        if self.final_value.is_none() {
            if let Some(diagnostics) = self.diagnostics.get_mut(label) {
                diagnostics.observe(event);
            }
            if let Some(phase) = self.phases.get_mut(label) {
                phase.observe(event);
            }
        }
    }
    pub fn payload(&mut self, name: &str, value: &str) {
        if self.final_value.is_none()
            && [
                "main_prompt",
                "capsule_body",
                "capsule_envelope",
                "draft_instruction",
                "recovery_prompt",
            ]
            .contains(&name)
        {
            self.payloads.insert(name.into(), text_size(value));
        }
    }
    pub fn transport(&mut self, label: &str, count: Option<u64>, state: &str) {
        if self.final_value.is_none()
            && ["written", "failed", "not_used"].contains(&state)
            && let Some(process) = self.processes.get_mut(label)
        {
            process["stdin_written_bytes"] =
                object!(count.filter(|value| *value <= i64::MAX as u64));
            process["stdin_state"] = state.into();
        }
    }
    pub fn usd(
        &mut self,
        label: &str,
        own: Option<f64>,
        reported: Option<f64>,
        cost: Option<&infra_host::provider::RunCost>,
    ) {
        if self.final_value.is_some() || !self.phases.contains_key(label) {
            return;
        }
        let attribution = match cost {
            Some(cost) if cost.provider == "claude" && cost.resumed.is_some() && cost.restores => {
                if cost.previous.is_none() {
                    "missing_previous"
                } else if reported.is_some_and(|value| value < cost.previous.unwrap_or(0.)) {
                    "decreasing_total"
                } else {
                    "resumed_delta"
                }
            }
            Some(cost) if cost.provider == "claude" => {
                if cost.resumed.is_some() {
                    "legacy_or_unrecognized_version_assumption"
                } else {
                    "fresh_reported"
                }
            }
            _ => "reported_direct",
        };
        let records = self.usd_reports.entry(label.into()).or_default();
        if records.len() >= 8 {
            self.incomplete = true;
        } else {
            records.push(object!({"own_usd":own.filter(|value|value.is_finite()&&*value>=0.&&*value<9223372036854775808.),"attribution":attribution}));
        }
    }
    pub fn duration(&mut self, name: &str, seconds: f64) {
        if self.final_value.is_none()
            && [
                "recall",
                "prepare",
                "provider",
                "recovery",
                "save",
                "checkpoint",
                "history",
            ]
            .contains(&name)
            && seconds.is_finite()
            && (0. ..9223372036854775808.).contains(&seconds)
        {
            *self.durations.entry(name.into()).or_default() += seconds;
        }
    }
    pub fn local(&mut self, record: Value) {
        if self.final_value.is_some() {
            return;
        }
        if self.operations.len() >= 128 {
            self.incomplete = true;
        } else {
            self.operations.push(record);
        }
    }
    pub fn note(&mut self, name: &str) {
        if self.final_value.is_none()
            && ["stale_revision_retries", "observation_errors"].contains(&name)
        {
            let value = self.notes.entry(name.into()).or_default();
            *value = 4096.min(value.saturating_add(1));
        }
    }
    pub fn source_hints(
        &mut self,
        calls: Option<u64>,
        outcome: &str,
        seconds: Option<f64>,
        references: Option<u64>,
        transport: Option<&Value>,
    ) {
        if self.final_value.is_some() {
            return;
        }
        if self.remote_operations.len() >= 16 {
            self.remote_overflow = true;
            return;
        }
        let empty = Value::Null;
        let transport = transport.unwrap_or(&empty);
        let bound = |key: &str, max: u64| counter(&transport[key]).filter(|value| *value <= max);
        let mut record = object!({"schema":"source-hint-observation-v1","operation":"recall-references","provider_calls":calls.filter(|value|*value<=1),"outcome":if ["resolved","empty","invalid","unavailable","cancelled","unsupported"].contains(&outcome){outcome}else{"unavailable"},"seconds":seconds.filter(|value|value.is_finite()&&*value>=0.&&*value<9223372036854775808.),"supplied_references":references.filter(|value|*value<=8),"transport_method_calls":bound("attempts",1),"request_body_bytes":bound("request_body_bytes",8192),"response_body_bytes":bound("response_body_bytes",65537),"response_eof":transport["response_eof"].as_bool(),"remote_tokens":null,"remote_usd":null,"provider_tokens":null,"total_cost_complete":false});
        record["metadata_state"] = if [
            "provider_calls",
            "seconds",
            "supplied_references",
            "transport_method_calls",
            "request_body_bytes",
            "response_body_bytes",
            "response_eof",
        ]
        .iter()
        .all(|key| !record[*key].is_null())
        {
            "observed_schema_only"
        } else {
            "incomplete"
        }
        .into();
        self.remote_operations.push(record)
    }
    pub fn set_outcome(&mut self, outcome: &str) {
        if self.final_value.is_none()
            && [
                "completed",
                "failed",
                "cancelled",
                "interrupted",
                "awaiting_context",
                "superseded",
                "not_executed",
            ]
            .contains(&outcome)
        {
            self.outcome = outcome.into();
        }
    }
    pub fn snapshot(&self) -> Value {
        if let Some(final_value) = &self.final_value {
            return final_value.clone();
        }
        let phases = self
            .phases
            .iter()
            .filter(|(label, _)| self.processes[*label]["started"] == true)
            .map(|(_, phase)| phase.clone())
            .collect::<Vec<_>>();
        let mut aggregate = episode(&phases);
        if self.incomplete {
            aggregate["provider_tokens"] = Value::Null;
            aggregate["token_state"] = "incomplete".into();
        }
        if let Some(rows) = aggregate["executions"].as_array_mut() {
            for row in rows {
                let label = row["phase"].as_str().unwrap_or("");
                let attributed = self.usd_reports.get(label).cloned().unwrap_or_default();
                row["usd_attributions"] = object!(attributed);
                if row["terminal_count"] == 1
                    && attributed.len() == 1
                    && !attributed[0]["own_usd"].is_null()
                    && !["conflict", "unknown"]
                        .contains(&row["native_identity_state"].as_str().unwrap_or(""))
                    && row["overflow"] == false
                {
                    row["own_usd"] = attributed[0]["own_usd"].clone();
                    row["own_usd_state"] = "attributed".into();
                }
            }
        }
        let rows = aggregate["executions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let known = rows
            .iter()
            .filter_map(|row| amount(&row["own_usd"]))
            .sum::<f64>();
        let complete = !rows.is_empty()
            && !self.incomplete
            && rows.iter().all(|row| !row["own_usd"].is_null());
        aggregate["known_usd_subtotal"] = object!(known);
        aggregate["own_usd"] = object!(if complete { Some(known) } else { None });
        aggregate["usd_state"] = if complete { "complete" } else { "incomplete" }.into();
        object!({"schema":"harness-full-cost-v1","state":"collecting","outcome":self.outcome,"supported_workflow":self.supported,"observer_incomplete":self.incomplete,"lifecycle_seconds":self.started.elapsed().as_secs_f64(),"stage_seconds":self.durations,"processes":self.processes,"provider":aggregate,"native_diagnostics":self.diagnostics.iter().map(|(label,value)|(label.clone(),value.snapshot())).collect::<BTreeMap<_,_>>(),"constructed_payloads":self.payloads,"payload_contract":"prompt parts are subsets, not additive; construction/write does not prove model use","local_operations":self.operations,"notes":self.notes,"remote_operations":self.remote_operations,"remote_metadata_overflow":self.remote_overflow,"remote_cost_contract":"provider totals exclude unknown remote billing; method calls/body bytes do not prove server receipt or model tokens","local_cost_contract":"local I/O/write bytes are not model tokens; filesystem I/O unmeasured"})
    }
    pub fn finish(&mut self, outcome: Option<&str>) -> Value {
        if self.final_value.is_none() {
            for diagnostic in self.diagnostics.values_mut() {
                diagnostic.finish();
            }
            if let Some(outcome) = outcome {
                self.set_outcome(outcome);
            }
            let mut result = self.snapshot();
            result["state"] = "final".into();
            self.final_value = Some(result);
        }
        self.snapshot()
    }
}
pub struct LocalOperation {
    owner: Arc<Mutex<LaunchObservation>>,
    name: String,
    started: Instant,
    retries: u32,
    io: Vec<Value>,
    overflow: bool,
    incomplete: bool,
    finished: bool,
}
impl LocalOperation {
    pub fn new(
        owner: Arc<Mutex<LaunchObservation>>,
        operation: &str,
        action: Option<&str>,
    ) -> Self {
        let action = if operation == "run" && action == Some("turn") {
            Some("checkpoint")
        } else {
            action
        };
        let allowed = [
            "refresh",
            "retrieve",
            "start",
            "rebind",
            "record-result",
            "prepare-result",
            "replay-result",
            "brain-create",
            "brain-update",
            "promote-auto",
            "checkpoint",
            "cursor-rule-retire",
        ];
        let name = if operation == "run" && action.is_some_and(|action| allowed.contains(&action)) {
            action.unwrap_or("other")
        } else if ["inspect", "info"].contains(&operation) {
            operation
        } else {
            "other"
        };
        Self {
            owner,
            name: name.into(),
            started: Instant::now(),
            retries: 0,
            io: vec![],
            overflow: false,
            incomplete: false,
            finished: false,
        }
    }
    pub fn retry(&mut self) {
        self.retries = 4096.min(self.retries.saturating_add(1));
    }
    pub fn io(&mut self, values: &Value) {
        if self.io.len() >= 64 {
            self.overflow = true;
            return;
        }
        let mut record = object!({});
        for name in [
            "argv_argument_bytes",
            "request_file_bytes",
            "stdout_known_bytes",
            "stderr_known_bytes",
            "returned_stdout_bytes",
            "returned_stderr_bytes",
        ] {
            record[name] = object!(counter(&values[name]));
        }
        for name in ["started", "returned", "stdout_eof", "stderr_eof"] {
            record[name] = object!(values[name].as_bool());
        }
        record["seconds"] = money(&values["seconds"]);
        self.io.push(record);
    }
    pub fn finish(&mut self, result: Option<&Value>, outcome: &str) {
        if self.finished {
            return;
        }
        self.finished = true;
        let outcome = if let Some(result) = result.filter(|value| value.is_object()) {
            if result["ok"] == false {
                "failed"
            } else {
                "returned"
            }
        } else {
            if ["failed", "returned", "cancelled", "interrupted", "not_run"].contains(&outcome) {
                outcome
            } else {
                "failed"
            }
        };
        let mut states = BTreeMap::<String, u32>::new();
        let mut state_status = "not_applicable";
        if ["record-result", "replay-result"].contains(&self.name.as_str()) && outcome == "returned"
        {
            state_status = "observed";
            let records = result.and_then(|value| value["result"]["records"].as_array());
            if let Some(records) = records.filter(|records| records.len() <= 128) {
                for record in records {
                    if let Some(state) = record["state"].as_str().filter(|value| {
                        [
                            "created",
                            "updated",
                            "existing",
                            "archived",
                            "differs",
                            "archived-differs",
                            "missing",
                            "completed",
                        ]
                        .contains(value)
                    }) {
                        *states.entry(state.into()).or_default() += 1;
                    } else {
                        self.incomplete = true;
                        state_status = "unknown";
                    }
                }
            } else {
                self.incomplete = true;
                state_status = "unknown";
            }
        }
        if let Ok(mut owner) = self.owner.lock() {
            owner.incomplete |= self.incomplete;
            owner.local(object!({"operation":self.name,"outcome":outcome,"seconds":self.started.elapsed().as_secs_f64(),"lock_retries":self.retries,"runtime_io":self.io,"io_state":if !self.io.is_empty()&&!self.overflow&&!self.incomplete{"observed_subprocesses"}else{"unknown"},"filesystem_io":"unmeasured","record_states":states,"record_states_state":state_status}));
        }
    }
}
impl Drop for LocalOperation {
    fn drop(&mut self) {
        if !self.finished {
            self.finish(None, "failed");
        }
    }
}
