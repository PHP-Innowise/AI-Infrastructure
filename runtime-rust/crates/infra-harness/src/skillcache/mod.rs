//! SkillCache in the Harness.
//!
//! A session opts in with `skillcache: {"mode": ...}`; the default is `off`
//! and then nothing here runs. `observe` and `auto` keep the observed trace of
//! a cold turn, which stores tool results the Harness otherwise never stores:
//! that is why it is an explicit choice per session. The capsule's own
//! contracts live in the `infra-skillcache` crate.
pub(crate) mod db;
pub mod settings;
pub mod trace;

use crate::{
    HarnessError,
    services::{Domain, Payload, Reply},
    store::SessionStore,
};
use serde_json::{Value, json};

/// The HTTP operations of SkillCache.
pub struct SkillCacheDomain {
    store: SessionStore,
}
impl SkillCacheDomain {
    pub fn new(store: SessionStore) -> Self {
        Self { store }
    }
    /// What a session's SkillCache state is: its mode and the traces it kept.
    /// A trace is listed without its steps; they stay in the private state.
    fn session(&self, session_id: &str) -> crate::Result<Value> {
        let session = self.store.get(session_id)?;
        let traces = db::session_traces(&*self.store.connection()?, session_id)?;
        Ok(json!({
            "session_id": session_id,
            "mode": settings::mode(&session),
            "settings": session["skillcache"],
            "traces": traces,
            "retention": {"days": db::TRACE_DAYS, "per_session": db::TRACES_PER_SESSION},
        }))
    }
}
impl Domain for SkillCacheDomain {
    fn execute<'a>(&'a self, operation: &'a str, request: Value) -> Reply<'a> {
        Box::pin(async move {
            let parameter = |name: &str| {
                request["params"][name]
                    .as_str()
                    .ok_or_else(|| HarnessError::invalid(format!("Missing {name}.")))
            };
            match operation {
                "skillcache.session" => Ok(Payload::Json(self.session(parameter("session_id")?)?)),
                _ => Err(HarnessError::missing("Unknown SkillCache operation.")),
            }
        })
    }
}
