use crate::{HarnessError, Result};
use serde_json::Value;
pub fn parse(bytes: &[u8], limit: usize) -> Result<Value> {
    if bytes.len() > limit {
        return Err(HarnessError::invalid("JSON body exceeds its byte limit."));
    }
    infra_core::strict_json::parse(bytes, limit, 128, 1_000_000)
        .map_err(|_| HarnessError::invalid("Invalid JSON request."))
}
