//! Concrete binding to the production Rust runtime; never imports project Python.
use super::projection::{NativeRequest, NativeResponse, NativeRuntime};
use anyhow::Result;
use std::path::Path;
pub struct CoreRuntime {
    pub actor: String,
    clock: Option<infra_core::domain::clock::WallClock>,
}
impl Default for CoreRuntime {
    fn default() -> Self {
        Self::new("agent")
    }
}
impl CoreRuntime {
    pub fn new(actor: impl Into<String>) -> Self {
        Self {
            actor: actor.into(),
            clock: None,
        }
    }
    /// Explicit trusted evaluation input. Ordinary instances use the system
    /// knowledge clock and never inspect MEMORY_EVAL_CLOCK.
    pub fn with_eval_clock(actor: impl Into<String>, anchor: &str) -> Result<Self> {
        let anchor = infra_core::domain::clock::parse_python_anchor(anchor)?;
        Ok(Self {
            actor: actor.into(),
            clock: Some(infra_core::domain::clock::WallClock::Offset(
                anchor - chrono::Utc::now(),
            )),
        })
    }
    pub fn with_clock(
        actor: impl Into<String>,
        clock: infra_core::domain::clock::WallClock,
    ) -> Self {
        Self {
            actor: actor.into(),
            clock: Some(clock),
        }
    }
}
#[derive(Debug)]
pub struct CleanupFailure;
impl std::fmt::Display for CleanupFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("native connection cleanup failed")
    }
}
impl std::error::Error for CleanupFailure {}
impl NativeRuntime for CoreRuntime {
    fn refresh_and_render(
        &mut self,
        repository: &Path,
        request: NativeRequest,
    ) -> Result<NativeResponse> {
        let _clock = self.clock.clone().map(infra_core::domain::clock::enter);
        let prompt = infra_core::domain::sanitize_automatic_query(&request.query)?;
        if prompt.is_empty() {
            return Ok(NativeResponse {
                text: String::new(),
                has_evidence: false,
                warnings: false,
                sanitized_empty: true,
            });
        }
        infra_core::domain::guard_shared_text(
            "Task Capsule request",
            std::slice::from_ref(&prompt),
        )?;
        let roots = infra_core::Roots::installed(repository)?;
        let config = infra_core::RuntimeConfig::load(&roots)?;
        let context =
            infra_core::RuntimeContext::new(roots, self.actor.clone(), config.mode(None)?)?;
        let mut store = infra_core::Store::open(
            repository.join("memory-bank/local/context.db"),
            infra_core::OpenOptions::refresh(None),
        )?;
        let operation = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || -> Result<NativeResponse> {
                let capsule = infra_core::retrieve(
                    &mut store,
                    &context,
                    &infra_core::RetrievalRequest {
                        query: prompt,
                        task_id: Some(request.task_id),
                        limit: request.limit,
                        host: request.host.into(),
                        entry_point: request.entry_point.into(),
                        ephemeral: request.ephemeral,
                        capsule_profile: Some(request.capsule_profile.into()),
                        query_source: "prompt".into(),
                        allow_unprovisioned: request.allow_unprovisioned,
                        automatic_links: true,
                        refresh_index: true,
                        ..Default::default()
                    },
                )?;
                let text =
                    infra_core::capsule::render(&capsule, infra_core::DeliveryProfile::Ordinary)?;
                let evidence = ["semantic", "episodic"]
                    .iter()
                    .any(|k| capsule.0[*k].as_array().is_some_and(|a| !a.is_empty()));
                let warnings = capsule.0["warnings"]
                    .as_array()
                    .is_some_and(|a| !a.is_empty());
                Ok(NativeResponse {
                    text,
                    has_evidence: evidence,
                    warnings,
                    sanitized_empty: false,
                })
            },
        ));
        let cleanup = store.conn.close();
        if cleanup.is_err() {
            return Err(CleanupFailure.into());
        }
        match operation {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }
}
