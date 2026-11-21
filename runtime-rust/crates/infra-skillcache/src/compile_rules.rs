//! What a compiler agent is given: its instructions, the shape of its answer
//! and the template of the textual form. They are engine resources, not an
//! edition's skill, so no mirror, inventory or installed project carries them.
use serde_json::Value;

pub const INSTRUCTIONS: &str = include_str!("../resources/compile/instructions.md");
pub const TEXTUAL_TEMPLATE: &str = include_str!("../resources/compile/textual.md");
const RESPONSE_SCHEMA: &str = include_str!("../resources/compile/response.schema.json");

/// The JSON Schema a compiler's structured answer follows.
pub fn response_schema() -> Value {
    serde_json::from_str(RESPONSE_SCHEMA).expect("the embedded answer schema is JSON")
}

/// Names the instructions a capsule was compiled under. Editing any of the
/// resources changes it, and a capsule compiled under other rules is stale.
pub fn version() -> String {
    let mut text = String::new();
    for resource in [INSTRUCTIONS, TEXTUAL_TEMPLATE, RESPONSE_SCHEMA] {
        text.push_str(&resource.len().to_string());
        text.push('\n');
        text.push_str(resource);
    }
    format!("1-{}", &crate::fingerprint::sha256(text.as_bytes())[..12])
}
