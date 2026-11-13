//! Bounded native boundary occurrences. No raw error text or inferred cause persists.
use serde_json::{Value, json as object};
use std::collections::BTreeMap;
const LIMIT: u64 = 4096;
const SIGNALS: &[&str] = &[
    "launch_exception",
    "stdin_write_failed",
    "output_line_limit",
    "output_total_limit",
    "time_budget",
    "token_budget",
    "usd_budget",
    "cancelled",
    "shutdown",
    "generation_changed",
    "missing_terminal",
    "identity_changed",
    "wait_timeout",
    "cleanup_confirmed",
    "cleanup_unconfirmed",
    "empty_read_ended_or_error",
    "malformed_line",
    "non_object_line",
    "lifecycle_exception",
    "recovery_no_draft",
    "recovery_skipped",
    "shared_budget",
    "guard_exit_before_cleanup",
    "guard_exit_after_cleanup",
];
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
pub struct NativeDiagnostics {
    provider: String,
    final_value: Option<Value>,
    incomplete: bool,
    overflow: bool,
    events: u64,
    errors: u64,
    success: u64,
    failed: u64,
    unknown: u64,
    contradictory: u64,
    item_errors: u64,
    subtypes: BTreeMap<String, u64>,
    signals: BTreeMap<String, u64>,
    exits: BTreeMap<String, Value>,
    http_statuses: Vec<Value>,
    reasons: BTreeMap<String, u64>,
}
impl NativeDiagnostics {
    pub fn new(provider: &str) -> Self {
        Self {
            provider: if ["codex", "claude", "cursor"].contains(&provider) {
                provider
            } else {
                "unsupported"
            }
            .into(),
            final_value: None,
            incomplete: false,
            overflow: false,
            events: 0,
            errors: 0,
            success: 0,
            failed: 0,
            unknown: 0,
            contradictory: 0,
            item_errors: 0,
            subtypes: BTreeMap::new(),
            signals: BTreeMap::new(),
            exits: BTreeMap::new(),
            http_statuses: vec![],
            reasons: BTreeMap::new(),
        }
    }
    fn count(map: &mut BTreeMap<String, u64>, key: &str, overflow: &mut bool) {
        let value = map.entry(key.into()).or_default();
        if *value >= LIMIT {
            *overflow = true;
        } else {
            *value += 1;
        }
    }
    pub fn observe(&mut self, event: &Value) {
        if self.final_value.is_some() {
            return;
        }
        if self.events >= LIMIT {
            self.overflow = true;
            return;
        }
        self.events += 1;
        if !event.is_object() {
            self.incomplete = true;
            return;
        }
        let kind = event["type"].as_str().unwrap_or("");
        if kind == "error" {
            self.errors += 1;
        }
        if self.provider == "codex"
            && kind == "item.completed"
            && event["item"].is_object()
            && event["item"]["type"] == "error"
        {
            self.item_errors += 1;
        }
        if self.provider == "codex" && ["turn.completed", "turn.failed"].contains(&kind) {
            let flagged = truth(&event["error"]) || event["is_error"] == true;
            if kind == "turn.failed" || flagged {
                self.failed += 1;
            } else {
                self.success += 1;
            }
            self.contradictory += u64::from(kind == "turn.completed" && flagged);
        } else if self.provider == "claude" && kind == "result" {
            let bucket = match event["subtype"].as_str().unwrap_or("") {
                "success" => "success",
                "error_during_execution" => "execution_error",
                "error_max_turns" => "turn_limit",
                "error_max_budget_usd" => "usd_limit",
                "error_max_structured_output_retries" => "structured_output_retry_limit",
                _ => "unknown",
            };
            Self::count(&mut self.subtypes, bucket, &mut self.overflow);
            let flag = &event["is_error"];
            if bucket == "success" && flag == &Value::Bool(false) {
                self.success += 1;
            } else if flag == &Value::Bool(true) || !["success", "unknown"].contains(&bucket) {
                self.failed += 1;
            } else {
                self.unknown += 1;
            }
            self.contradictory +=
                u64::from(!["success", "unknown"].contains(&bucket) && flag == &Value::Bool(false));
            let reason = event["terminal_reason"].as_str();
            let reason_bucket = if let Some(reason) = reason.filter(|value| {
                [
                    "completed",
                    "max_turns",
                    "aborted_streaming",
                    "aborted_tools",
                ]
                .contains(value)
            }) {
                reason
            } else if event.get("terminal_reason").is_none() {
                "unreported"
            } else if event["terminal_reason"].is_null() || event["terminal_reason"].is_string() {
                "unknown"
            } else {
                "invalid"
            };
            Self::count(&mut self.reasons, reason_bucket, &mut self.overflow);
            let status = event["api_error_status"]
                .as_i64()
                .filter(|value| (100..=599).contains(value));
            if self.http_statuses.len() >= 8 {
                self.overflow = true;
            } else {
                self.http_statuses.push(object!({"status":status,"state":if status.is_some(){"observed"}else if event.get("api_error_status").is_none(){"unreported"}else if event["api_error_status"].is_null(){"unknown"}else{"invalid"}}));
            }
        }
    }
    pub fn boundary(&mut self, signal: &str, code: Option<i64>) {
        if self.final_value.is_some() {
            return;
        }
        if !SIGNALS.contains(&signal) {
            self.incomplete = true;
            return;
        }
        Self::count(&mut self.signals, signal, &mut self.overflow);
        if signal.starts_with("guard_exit_") {
            let code = code.filter(|value| *value >= i32::MIN as i64 && *value <= u32::MAX as i64);
            let new = object!({"code":code,"state":if code.is_some(){"observed"}else{"unknown"}});
            if let Some(old) = self.exits.get_mut(signal) {
                if old["code"] != new["code"] || old["state"] == "conflict" {
                    *old = object!({"code":null,"state":"conflict"});
                }
            } else {
                self.exits.insert(signal.into(), new);
            }
        }
    }
    pub fn unavailable(&mut self) {
        if self.final_value.is_none() {
            self.incomplete = true;
        }
    }
    pub fn snapshot(&self) -> Value {
        self.final_value.clone().unwrap_or_else(||object!({"schema":"native-boundary-diagnostics-v1","profile":if ["codex","claude"].contains(&self.provider.as_str()){&self.provider}else{"unsupported"},"events":self.events,"provider_error_events":self.errors,"nonfatal_item_error_events":self.item_errors,"successful_terminals":self.success,"failed_terminals":self.failed,"unknown_terminals":self.unknown,"contradictory_terminals":self.contradictory,"reported_subtypes":self.subtypes,"boundary_signals":self.signals,"reported_terminal_reasons":self.reasons,"reported_http_statuses":self.http_statuses,"guard_exits":self.exits,"coverage":if self.incomplete||self.overflow{"incomplete"}else{"observed_callbacks_only"},"overflow":self.overflow,"root_cause":"unmeasured","stderr":"not_captured","contract":"occurrences not incidents; success predicate not task correctness; reasons do not override flags; guard exits not native cause; missing events not zero errors"}))
    }
    pub fn finish(&mut self) -> Value {
        if self.final_value.is_none() {
            self.final_value = Some(self.snapshot());
        }
        self.snapshot()
    }
}
