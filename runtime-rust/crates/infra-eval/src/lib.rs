//! Maintainer evaluation controllers. Every process is a native executable.
//! Private labels and partial/unknown observations are never converted into success.
pub mod admission;
pub mod artifact;
pub mod bench;
pub mod cost;
pub mod delivery;
pub mod executable;
pub mod field;
pub mod intake;
pub mod isolation;
pub mod native;
pub mod oracle;
pub mod outcome;
pub mod process;
pub mod reconstruction;
pub mod response;
pub mod rng;
pub mod scoring;
pub mod skillcache;
pub mod transcript;
pub mod treatments;
pub use infra_core::{Result, RuntimeError};
pub trait ExcerptScorer {
    fn score(
        &mut self,
        query: &str,
        passages: &[String],
        remaining: std::time::Duration,
    ) -> std::result::Result<Vec<f64>, String>;
}
pub mod cli_contract;
pub mod control;
pub mod sampling;
pub use control::execute;
pub mod bench_driver;
pub mod cancel;
pub mod diagnostic;
pub mod facts;
pub mod freshness;
pub mod graph;
pub mod lifecycle;
pub mod onnx_adapter;
pub mod rehearsal;
pub mod reports;
pub mod throttle_oracle;

pub mod clock;

/// Shared sealed A/B evaluation protocol primitives reused across SkillCache and
/// memory/rerank evaluation treatments.
pub mod sealed_ab {
    use crate::{Result, artifact, process, rng};
    use serde_json::{Map, Value, json};
    use std::path::Path;

    /// Ordered normalized token components billed across Harness evaluations.
    pub const USAGE_COMPONENTS: [&str; 6] = [
        "input_tokens",
        "output_tokens",
        "cache_read_input_tokens",
        "cache_creation_input_tokens",
        "cache_write_5m_input_tokens",
        "cache_write_1h_input_tokens",
    ];

    /// Computes canonical SHA-256 digest of a protocol value excluding `"seal_sha256"`.
    pub fn seal_digest(protocol: &Value) -> Result<String> {
        let mut body = protocol.clone();
        if let Some(members) = body.as_object_mut() {
            members.remove("seal_sha256");
        }
        artifact::digest(&body)
    }

    /// Stamps `"sealed_at"` and `"seal_sha256"` onto a protocol document and writes it to disk.
    pub fn write_sealed_protocol(out: &Path, mut document: Value) -> Result<Value> {
        document["sealed_at"] = chrono::Utc::now()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
            .into();
        document["seal_sha256"] = seal_digest(&document)?.into();
        artifact::write_json(out, &document)?;
        Ok(document)
    }

    /// Verifies that `sealed["seal_sha256"]` matches the canonical body digest.
    pub fn verify_sealed_digest(sealed: &Value, mismatch_message: &str) -> Result<()> {
        if sealed["seal_sha256"].as_str() != Some(seal_digest(sealed)?.as_str()) {
            return Err(artifact::invalid(mismatch_message));
        }
        Ok(())
    }

    /// Fisher-Yates shuffle over experiment arms using a seeded deterministic PRNG.
    pub fn shuffled_arms<T: Clone>(items: &[T], order: &mut rng::Random) -> Vec<T> {
        let mut out = items.to_vec();
        for index in (1..out.len()).rev() {
            out.swap(index, order.below(index + 1));
        }
        out
    }

    /// Rounds fractional seconds to milliseconds.
    pub fn round_millis(seconds: f64) -> f64 {
        (seconds * 1000.0).round() / 1000.0
    }

    /// Empty 6-component usage object with `Null` counts.
    pub fn empty_usage() -> Value {
        USAGE_COMPONENTS
            .iter()
            .map(|component| ((*component).to_string(), Value::Null))
            .collect::<Map<_, _>>()
            .into()
    }

    /// Normalizes a provider usage object into the 6 canonical components, rejecting
    /// unnormalized `cached_input_tokens` so cache reads are never double-counted.
    pub fn normalize_usage(source: &Value) -> std::result::Result<Value, &'static str> {
        if source.is_null() {
            return Ok(empty_usage());
        }
        if !source.is_object() {
            return Err("driver-answer-invalid");
        }
        if source
            .get("cached_input_tokens")
            .is_some_and(|value| !value.is_null())
        {
            return Err("driver-answer-invalid");
        }
        let write_5m = process::token(&source["cache_write_5m_input_tokens"])
            .or_else(|| process::token(&source["cache_creation"]["ephemeral_5m_input_tokens"]));
        let write_1h = process::token(&source["cache_write_1h_input_tokens"])
            .or_else(|| process::token(&source["cache_creation"]["ephemeral_1h_input_tokens"]));
        let write_total =
            process::token(&source["cache_creation_input_tokens"]).or_else(|| {
                match (write_5m, write_1h) {
                    (Some(m5), Some(h1)) => m5.checked_add(h1),
                    _ => None,
                }
            });
        let mut map = Map::new();
        map.insert(
            "input_tokens".into(),
            json!(process::token(&source["input_tokens"])),
        );
        map.insert(
            "output_tokens".into(),
            json!(process::token(&source["output_tokens"])),
        );
        map.insert(
            "cache_read_input_tokens".into(),
            json!(process::token(&source["cache_read_input_tokens"])),
        );
        map.insert("cache_creation_input_tokens".into(), json!(write_total));
        map.insert("cache_write_5m_input_tokens".into(), json!(write_5m));
        map.insert("cache_write_1h_input_tokens".into(), json!(write_1h));
        Ok(Value::Object(map))
    }

    /// Wraps any evaluation output/treatment report with sealed provenance metadata
    /// (`schema`, `input_digest`, `seal_sha256`, `sealed_at`).
    pub fn seal_experiment_envelope(
        schema: &str,
        mut report: Value,
        input_digest: &str,
    ) -> Result<Value> {
        if let Some(obj) = report.as_object_mut() {
            obj.insert("schema".into(), json!(schema));
            obj.insert("input_digest".into(), json!(input_digest));
            obj.insert(
                "sealed_at".into(),
                json!(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            );
            let digest = seal_digest(&Value::Object(obj.clone()))?;
            obj.insert("seal_sha256".into(), json!(digest));
        }
        Ok(report)
    }
}
