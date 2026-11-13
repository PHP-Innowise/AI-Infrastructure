//! Explicit evaluation calendar. Ambient environment variables are never consulted.
//! Offset anchors advance with elapsed wall time; proof and deadline clocks remain real.
use crate::{Result, artifact};
use infra_core::domain::clock::{self, ClockScope, WallClock};
use serde_json::Value;
pub const PROFILE: &str = "explicit-offset-worker-v1";
pub fn enter(anchor: Option<&str>) -> Result<Option<ClockScope>> {
    let Some(anchor) = anchor else {
        return Ok(None);
    };
    if anchor.is_empty() || anchor.len() > 128 {
        return Err(artifact::invalid(
            "eval_clock must be an ISO datetime of at most 128 bytes",
        ));
    }
    let at = clock::parse_python_anchor(anchor)?;
    Ok(Some(clock::enter(WallClock::Offset(
        at - chrono::Utc::now(),
    ))))
}
pub fn anchor(request: &Value) -> Result<Option<&str>> {
    match request.get("eval_clock") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(anchor)) => {
            if anchor.is_empty() || anchor.len() > 128 {
                return Err(artifact::invalid(
                    "eval_clock must be an ISO datetime of at most 128 bytes",
                ));
            }
            clock::parse_python_anchor(anchor)?;
            Ok(Some(anchor))
        }
        _ => Err(artifact::invalid(
            "eval_clock must be an explicit ISO datetime string",
        )),
    }
}
pub fn request(request: &Value) -> Result<Option<ClockScope>> {
    enter(anchor(request)?)
}
