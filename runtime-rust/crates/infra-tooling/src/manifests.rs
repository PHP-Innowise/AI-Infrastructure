use crate::common;
use infra_core::{Result, RuntimeError, fs as safe};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreManifest {
    pub version: u32,
    pub canonical: String,
    pub editions: BTreeMap<String, String>,
    pub includes: Vec<String>,
    pub allowed_drift: BTreeMap<String, String>,
    #[serde(default)]
    pub retired: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MirrorManifest {
    pub version: u32,
    pub classes: Vec<MirrorClass>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MirrorClass {
    pub name: String,
    pub canonical: String,
    pub mirrors: BTreeMap<String, MirrorSpec>,
    #[serde(default)]
    pub only: Option<Vec<String>>,
    #[serde(default)]
    pub skip: Vec<String>,
    #[serde(default)]
    pub skip_by_framework: BTreeMap<String, Vec<String>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MirrorSpec {
    #[serde(default = "copy")]
    pub transform: String,
    #[serde(default)]
    pub replacements: Vec<(String, String)>,
    #[serde(default)]
    pub skip: Vec<String>,
    #[serde(default)]
    pub description_overrides: BTreeMap<String, String>,
    #[serde(default = "yes")]
    pub quote_description: bool,
}
fn copy() -> String {
    "copy".into()
}
fn yes() -> bool {
    true
}
impl CoreManifest {
    pub fn load(root: &Path) -> Result<Self> {
        let value: Self = serde_json::from_slice(&safe::read_regular(
            &root.join("install/core-manifest.json"),
        )?)?;
        if value.version != 1 || !value.editions.values().any(|name| name == &value.canonical) {
            return Err(RuntimeError::invalid("Invalid native core manifest"));
        }
        common::relative(&value.canonical)?;
        for name in value.editions.values() {
            common::relative(name)?;
        }
        if value.includes.is_empty() {
            return Err(RuntimeError::invalid("Empty core manifest"));
        }
        Ok(value)
    }
    pub fn retired(&self, path: &str) -> Result<bool> {
        for pattern in &self.retired {
            if common::matches(pattern, path, true)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub fn core(&self, path: &str) -> Result<bool> {
        if self.allowed_drift.keys().any(|entry| {
            if entry.ends_with('/') {
                path.starts_with(entry)
            } else {
                path == entry
            }
        }) {
            return Ok(false);
        }
        for pattern in &self.includes {
            if common::matches(pattern, path, true)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
impl MirrorManifest {
    pub fn load(edition: &Path) -> Result<Self> {
        let path = if edition.join("mirror-rules.json").is_file() {
            edition.join("mirror-rules.json")
        } else {
            edition.join("memory-bank/config/mirror-rules.json")
        };
        let result: Self = serde_json::from_slice(&safe::read_regular(&path)?)?;
        if result.version != 1 || result.classes.is_empty() {
            return Err(RuntimeError::invalid("Unsupported mirror manifest"));
        }
        for class in &result.classes {
            common::relative(&class.canonical)?;
            for (name, spec) in &class.mirrors {
                common::relative(name)?;
                if !["copy", "cursor-command", "cursor-agent"].contains(&spec.transform.as_str()) {
                    return Err(RuntimeError::invalid("Unknown mirror transformation"));
                }
            }
            if let Some(only) = &class.only {
                for path in only {
                    common::relative(path)?;
                }
            }
        }
        Ok(result)
    }
}
