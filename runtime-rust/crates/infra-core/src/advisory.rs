//! Bounded local status and metadata-only operational accounting.
use crate::{
    Result, RuntimeContext, RuntimeError,
    bank::publication::PublicationFiles,
    capsule::Capsule,
    domain::{self, clock, json},
    query,
};
use serde_json::Value;
use std::collections::BTreeSet;
const REPORT: &str = "memory-bank/local/last-turn-report.json";
pub fn write_last_turn(context: &RuntimeContext, report: &Value) {
    let operation = (|| -> Result<()> {
        let mut bytes = json::sorted_pretty(report)?.into_bytes();
        bytes.push(b'\n');
        if bytes.len() > 1024 * 1024 {
            return Ok(());
        }
        let mut files = PublicationFiles::new(&context.roots.state)?;
        let previous = files.read(REPORT, 1024 * 1024)?;
        files.write_expected(REPORT, &bytes, previous.as_deref())
    })();
    let _ = operation;
}
pub fn load_last_turn(context: &RuntimeContext) -> Option<Value> {
    let operation = (|| -> Result<Value> {
        let mut files = PublicationFiles::new(&context.roots.state)?;
        let bytes = files
            .read(REPORT, 65536)?
            .ok_or_else(|| RuntimeError::unavailable("Prior turn unavailable"))?;
        let report = json::parse(
            std::str::from_utf8(&bytes)
                .map_err(|_| RuntimeError::invalid("Invalid advisory encoding"))?,
        )?;
        let at = chrono::DateTime::parse_from_rfc3339(
            report["timestamp"]
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("Invalid advisory timestamp"))?,
        )
        .map_err(|_| RuntimeError::invalid("Invalid advisory timestamp"))?
        .with_timezone(&chrono::Utc);
        let age = (clock::now() - at).num_seconds();
        if !(0..=86400).contains(&age) {
            return Err(RuntimeError::unavailable("Prior turn unavailable"));
        }
        Ok(report)
    })();
    operation.ok()
}
pub fn summarize(report: &Value, routine: bool) -> Option<String> {
    let mut parts = Vec::new();
    let strings = |key: &str| {
        report[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(String::from)
            .collect::<Vec<_>>()
    };
    let count = |key: &str| report[key].as_u64().unwrap_or(0);
    if let Some(error) = report["error"].as_str().filter(|value| !value.is_empty()) {
        parts.push(format!("turn failed: {error}"));
    }
    if report["native_capture_incomplete"] == true {
        parts.push("native memory result not saved completely; inspect the prior save before relying on it".into());
    }
    parts.extend(
        strings("queued_refused")
            .into_iter()
            .map(|value| format!("queued memory command not made: {value}")),
    );
    if report["promotion_retry_unavailable"] == true {
        parts.push("promotion retry unavailable: local scheduling state could not be saved".into());
    }
    if count("discarded_turns") > 0 {
        parts.push(format!("this branch's task is closed: {} turn(s) after its completion were not recorded; start a new task for further work",count("discarded_turns")));
    }
    for (label, key) in [
        ("promotion blocked", "promotion_blocked"),
        ("promotion failed", "promotion_failed"),
    ] {
        let mut seen = BTreeSet::new();
        for reason in report[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item["reason"].as_str())
            .filter(|value| !value.is_empty())
        {
            if seen.insert(reason) {
                parts.push(format!("{label}: {reason}"));
            }
        }
    }
    parts.extend(
        strings("compaction_errors")
            .into_iter()
            .map(|value| format!("compaction failed: {value}")),
    );
    let excluded = strings("excluded_paths");
    if !excluded.is_empty() {
        parts.push(format!(
            "excluded paths: {}{}",
            excluded
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
            if excluded.len() > 3 {
                format!(" (+{} more)", excluded.len() - 3)
            } else {
                String::new()
            }
        ));
    }
    let candidates = strings("completion_candidates");
    if !candidates.is_empty() {
        parts.push(format!(
            "completion candidate (explicit complete required): {}",
            candidates.join(", ")
        ));
    }
    if routine {
        let promoted = report["promoted"].as_array().map(Vec::len).unwrap_or(0);
        if promoted > 0 {
            parts.push(format!("promoted: {promoted} record(s)"));
        }
        if count("promotion_skipped") > 0 {
            parts.push(format!(
                "promotion deferred: {} record(s)",
                count("promotion_skipped")
            ));
        }
        if count("archived") > 0 {
            parts.push(format!("archived: {} record(s)", count("archived")));
        }
        if report["flushed"] == true {
            let mut flushed = "buffer flushed".to_string();
            if count("files") > 0 {
                flushed.push_str(&format!(": {} file(s) this turn", count("files")));
            }
            if count("files_omitted") > 0 {
                flushed.push_str(&format!(
                    " ({} beyond the per-flush limit)",
                    count("files_omitted")
                ));
            }
            if report["provisioned"] == true {
                flushed.push_str(", task auto-provisioned");
            }
            if !report["rebound"].is_null() {
                flushed.push_str(", binding restored to the existing task");
            }
            parts.push(flushed);
        }
    }
    if parts.is_empty() {
        return None;
    }
    let summary = parts.join("; ");
    if summary.chars().count() > 4096 {
        return Some("prior-turn diagnostics too large; inspect local memory status".into());
    }
    if domain::guard_shared_text("Last turn report", std::slice::from_ref(&summary)).is_err() {
        return Some(
            "prior-turn diagnostics withheld: sensitive content; inspect local memory status"
                .into(),
        );
    }
    Some(if summary.chars().count() > 600 {
        format!("{}…", query::chars(&summary, 599))
    } else {
        summary
    })
}
pub fn last_turn(context: &RuntimeContext, routine: bool) -> Option<String> {
    load_last_turn(context).and_then(|report| summarize(&report, routine))
}
pub fn append_refresh(context: &RuntimeContext, record: &Value) {
    crate::health::append_refresh(context, record)
}
pub fn health(context: &RuntimeContext, window: Option<usize>) -> Result<Value> {
    crate::health::health(context, window)
}
pub fn retrieval_report(
    context: &RuntimeContext,
    scope: &str,
    since: Option<usize>,
) -> Result<Value> {
    crate::health::retrieval_report(context, scope, since)
}
pub fn telemetry(context: &RuntimeContext, capsule: Option<&Capsule>, seconds: f64) {
    crate::health::telemetry(context, capsule, seconds)
}
