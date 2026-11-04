//! Canonical Eval/Bench argument contracts, extracted from syntax without importing retired code.
use crate::{Result, RuntimeError};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::OnceLock,
};

pub type Schema = BTreeMap<String, Value>;
type Registry = BTreeMap<String, Schema>;
static REGISTRY: OnceLock<std::result::Result<Registry, &'static str>> = OnceLock::new();

fn registry() -> Result<&'static Registry> {
    REGISTRY
        .get_or_init(|| {
            let value: Registry =
                serde_json::from_str(include_str!("../resources/cli-contract-v2.json"))
                    .map_err(|_| "Native evaluation CLI contract is invalid")?;
            if value.is_empty()
                || value.values().any(|schema| {
                    schema
                        .get("$contract")
                        .is_none_or(|contract| contract["profile"] != "eval-cli-v2")
                })
            {
                return Err("Native evaluation CLI contract is invalid");
            }
            Ok(value)
        })
        .as_ref()
        .map_err(|message| RuntimeError::unavailable(*message))
}
pub fn schema(operation: &str) -> Result<Option<Schema>> {
    Ok(registry()?.get(operation).cloned())
}
pub fn known_family(family: &str) -> Result<bool> {
    Ok(registry()?
        .keys()
        .any(|operation| operation.split('/').next() == Some(family)))
}
pub fn key(spec: &Value) -> Option<String> {
    spec["dest"]
        .as_str()
        .or_else(|| spec["flags"].as_array()?.first()?.as_str())
        .map(|name| name.trim_start_matches('-').replace('-', "_"))
}
fn is_append(spec: &Value) -> bool {
    spec["actions"]
        .as_array()
        .is_some_and(|actions| actions.iter().all(|action| action == "append"))
}
fn check_choice(spec: &Value, key: &str, value: &Value) -> Result<()> {
    let Some(choices) = spec["choices"].as_array() else {
        return Ok(());
    };
    let admitted = if is_append(spec) {
        value
            .as_array()
            .is_some_and(|values| values.iter().all(|value| choices.contains(value)))
    } else {
        choices.contains(value)
    };
    if !admitted {
        return Err(RuntimeError::invalid(format!(
            "Invalid value for --{}",
            key.replace('_', "-")
        )));
    }
    Ok(())
}
pub fn validate(operation: &str, request: &Value) -> Result<()> {
    let Some(schema) = schema(operation)? else {
        return Ok(());
    };
    // Validate supplied choices before requiredness; no cache or corpus is created here.
    for spec in schema.values() {
        let Some(key) = key(spec) else {
            continue;
        };
        if let Some(value) = request.get(&key).filter(|value| !value.is_null()) {
            check_choice(spec, &key, value)?;
        }
    }
    let mut seen = BTreeSet::new();
    for spec in schema.values() {
        let Some(key) = key(spec) else {
            continue;
        };
        if !seen.insert(key.clone()) || spec["required"] != true {
            continue;
        }
        if request.get(&key).is_none_or(|value| {
            value.is_null() || is_append(spec) && value.as_array().is_some_and(Vec::is_empty)
        }) {
            let label = if spec["positional"] == true {
                key
            } else {
                format!("--{}", key.replace('_', "-"))
            };
            return Err(RuntimeError::invalid(format!(
                "Required argument missing: {label}"
            )));
        }
    }
    Ok(())
}
fn executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|image| image.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        true
    }
}
fn which(program: &str) -> Option<PathBuf> {
    for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let candidate = directory.join(program);
        if executable(&candidate) {
            return Some(candidate);
        }
        #[cfg(windows)]
        for extension in std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
        {
            let candidate = directory.join(format!("{program}{extension}"));
            if executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}
pub fn default_value(spec: &Value) -> Result<Option<Value>> {
    let Some(default) = spec.get("default") else {
        return Ok(None);
    };
    if default.is_null() || default == false {
        // JSON absence represents canonical None/False for the existing typed controllers.
        // Preserving absence also keeps supplied-option provenance independent of defaults.
        return Ok(None);
    }
    match default["provider"].as_str() {
        Some("home") => Ok(Some(
            crate::control::home()
                .join(
                    default["relative"]
                        .as_str()
                        .ok_or_else(|| RuntimeError::unavailable("Invalid home default"))?,
                )
                .display()
                .to_string()
                .into(),
        )),
        Some("which") => {
            let program = default["program"]
                .as_str()
                .ok_or_else(|| RuntimeError::unavailable("Invalid executable default"))?;
            let fallback = default["fallback"]
                .as_str()
                .ok_or_else(|| RuntimeError::unavailable("Invalid executable default"))?;
            Ok(Some(
                which(program)
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| fallback.into())
                    .into(),
            ))
        }
        Some("controller") => Ok(None),
        Some(_) => Err(RuntimeError::unavailable(
            "Unknown evaluation default provider",
        )),
        None => Ok(Some(default.clone())),
    }
}
