use infra_core::{Result, RuntimeError};
use serde_json::Value;
pub const RUNTIME_CONTRACT: &str = include_str!("../resources/native-runtime-contract.json");
pub const MIRROR_RULES: &str = include_str!("../resources/mirror-rules.json");
pub fn runtime_contract() -> Value {
    serde_json::from_str(RUNTIME_CONTRACT)
        .expect("trusted native runtime contract compiled into generator")
}
pub fn mirror_rules() -> Value {
    serde_json::from_str(MIRROR_RULES)
        .expect("trusted generator mirror rules compiled into generator")
}
pub fn required_artifact() -> Result<String> {
    let contract = runtime_contract();
    let key = if cfg!(windows) {
        "windows_path"
    } else {
        "unix_path"
    };
    contract["native_engine"][key]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| {
            RuntimeError::invalid("Native artifact descriptor has no current-platform entry")
        })
}
