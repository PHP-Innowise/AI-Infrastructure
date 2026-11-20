//! Usage component aggregation and SkillCache run-step extraction.
use serde_json::{Value, json};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Components {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
    pub cache_write_5m_input_tokens: Option<u64>,
    pub cache_write_1h_input_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
    pub model_calls: Option<u64>,
}

impl Components {
    pub fn to_usage_json(&self) -> Value {
        json!({
            "input_tokens": self.input_tokens,
            "output_tokens": self.output_tokens,
            "cache_read_input_tokens": self.cache_read_input_tokens,
            "cache_write_5m_input_tokens": self.cache_write_5m_input_tokens,
            "cache_write_1h_input_tokens": self.cache_write_1h_input_tokens,
            "cache_creation_input_tokens": self.cache_creation_input_tokens,
        })
    }
}

pub fn zero_usage_json() -> Value {
    json!({
        "input_tokens": 0,
        "output_tokens": 0,
        "cache_read_input_tokens": 0,
        "cache_write_5m_input_tokens": 0,
        "cache_write_1h_input_tokens": 0,
        "cache_creation_input_tokens": 0,
    })
}

fn token(value: &Value) -> Option<u64> {
    value.as_u64().filter(|n| *n <= 9_007_199_254_740_991)
}

fn sum_required<F>(items: &[Value], get: F) -> Option<u64>
where
    F: Fn(&Value) -> Option<u64>,
{
    if items.is_empty() {
        return None;
    }
    let mut total: u64 = 0;
    for item in items {
        let value = get(item)?;
        total = total.checked_add(value)?;
    }
    Some(total)
}

fn cache_creation_total(m5: Option<u64>, h1: Option<u64>, scalar: Option<u64>) -> Option<u64> {
    match (m5, h1) {
        (Some(a), Some(b)) => a.checked_add(b),
        _ => scalar,
    }
}

/// Sums `launches[*].usage.components` across all launches of a turn.
/// A component stays `null` unless every launch in the slice reported it.
pub fn sum_launch_components(launches: &[Value]) -> Components {
    let entries: Vec<Value> = launches
        .iter()
        .map(|launch| launch["usage"]["components"].clone())
        .collect();
    let input_tokens = sum_required(&entries, |c| token(&c["input_tokens"]));
    let output_tokens = sum_required(&entries, |c| token(&c["output_tokens"]));
    let cache_read_input_tokens = sum_required(&entries, |c| token(&c["cache_read_input_tokens"]));
    let cache_write_5m_input_tokens =
        sum_required(&entries, |c| token(&c["cache_write_5m_input_tokens"]));
    let cache_write_1h_input_tokens =
        sum_required(&entries, |c| token(&c["cache_write_1h_input_tokens"]));
    let model_calls = sum_required(&entries, |c| token(&c["model_calls"]));
    let cache_creation_input_tokens = cache_creation_total(
        cache_write_5m_input_tokens,
        cache_write_1h_input_tokens,
        sum_required(&entries, |c| token(&c["cache_creation_input_tokens"])),
    );
    Components {
        input_tokens,
        output_tokens,
        cache_read_input_tokens,
        cache_write_5m_input_tokens,
        cache_write_1h_input_tokens,
        cache_creation_input_tokens,
        model_calls,
    }
}

/// Extracts compile token components from a `skillcache_compiles` row,
/// summing across `receipt.rounds[*]` when every round reported a field.
pub fn from_compile_row(compile: &Value) -> Components {
    let rounds = compile["receipt"]["rounds"]
        .as_array()
        .filter(|arr| !arr.is_empty())
        .cloned()
        .unwrap_or_else(|| {
            if compile["usage"].is_object() {
                vec![compile["usage"].clone()]
            } else {
                Vec::new()
            }
        });
    if rounds.is_empty() {
        return Components::default();
    }
    let normalized: Vec<Value> = rounds
        .into_iter()
        .map(|round| {
            if round["components"].is_object() {
                round["components"].clone()
            } else {
                round
            }
        })
        .collect();
    let input_tokens = sum_required(&normalized, |c| token(&c["input_tokens"]));
    let output_tokens = sum_required(&normalized, |c| token(&c["output_tokens"]));
    let cache_read_input_tokens =
        sum_required(&normalized, |c| token(&c["cache_read_input_tokens"]));
    let cache_write_5m_input_tokens =
        sum_required(&normalized, |c| token(&c["cache_write_5m_input_tokens"]));
    let cache_write_1h_input_tokens =
        sum_required(&normalized, |c| token(&c["cache_write_1h_input_tokens"]));
    let cache_creation_scalar =
        sum_required(&normalized, |c| token(&c["cache_creation_input_tokens"]));
    let cache_creation_input_tokens = cache_creation_total(
        cache_write_5m_input_tokens,
        cache_write_1h_input_tokens,
        cache_creation_scalar,
    );
    let model_calls = sum_required(&normalized, |c| token(&c["model_calls"]));
    Components {
        input_tokens,
        output_tokens,
        cache_read_input_tokens,
        cache_write_5m_input_tokens,
        cache_write_1h_input_tokens,
        cache_creation_input_tokens,
        model_calls,
    }
}

/// Extracts `(program_steps, failed_steps)` from a `skillcache_runs` row.
pub fn steps_from_run(run_row: Option<&Value>) -> (Option<u64>, Option<u64>) {
    let Some(row) = run_row else {
        return (None, None);
    };
    if row["outcome"].as_str() == Some("read") {
        return (None, None);
    }
    let steps = row["steps"].as_array();
    let completed = steps
        .into_iter()
        .flatten()
        .filter(|step| step["event"] == "program_completed")
        .count() as u64;
    let failed_events = steps
        .into_iter()
        .flatten()
        .filter(|step| step["event"] == "program_failed")
        .count() as u64;
    let failed = row["deopts"].as_u64().unwrap_or(failed_events);
    (Some(completed.saturating_add(failed)), Some(failed))
}
