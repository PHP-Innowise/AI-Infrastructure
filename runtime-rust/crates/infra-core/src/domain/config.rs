use crate::{Result, Roots, RuntimeError, fs};
use serde_json::{Value, json};

#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    pub value: Value,
}

impl RuntimeConfig {
    pub fn load(roots: &Roots) -> Result<Self> {
        let mut value = json!({
            "mode":"governed","framework":"generic","canonical_edition":".agents",
            "allowed_privacy":["public","team"],"allowed_authority":["observed","verified"],
            "owners":["*"],"telemetry_enabled":false,"provider":"sqlite-fts5",
            "automatic_promotion":false,"automatic_completion":false,
            "automatic_consolidation":false,"automatic_consolidation_verification":false,
            "automatic_compaction":false,"compaction_threshold":5,"codebase_map_max_drift":25
        });
        let path = roots.brain_root().join("config/runtime.json");
        match std::fs::symlink_metadata(&path) {
            Ok(_) => {
                let incoming: Value = super::json::parse(&fs::read_text(&path)?)?;
                let object = incoming.as_object().ok_or_else(|| {
                    RuntimeError::invalid("Project Brain configuration must be an object")
                })?;
                for (key, incoming) in object {
                    if value.get(key).is_some_and(Value::is_boolean) && !incoming.is_boolean() {
                        return Err(RuntimeError::invalid(format!(
                            "Project Brain configuration {key} must be boolean"
                        )));
                    }
                    value[key] = incoming.clone();
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(Self { value })
    }

    pub fn mode(&self, requested: Option<&str>) -> Result<String> {
        let environment = std::env::var("PROJECT_BRAIN_MODE").ok();
        let mode = requested
            .or(environment.as_deref())
            .or(self.value["mode"].as_str())
            .unwrap_or("governed");
        if !["governed", "lightweight"].contains(&mode) {
            return Err(RuntimeError::invalid(
                "Mode must be governed or lightweight",
            ));
        }
        Ok(mode.into())
    }

    pub fn capsule_profile(&self, requested: Option<&str>) -> Result<String> {
        let environment = std::env::var("CONTEXT_CAPSULE_PROFILE").ok();
        let profile = requested
            .or(environment.as_deref())
            .or(self.value["capsule_profile"].as_str())
            .unwrap_or("full");
        if !["full", "navigation", "recall"].contains(&profile) {
            return Err(RuntimeError::invalid(
                "Capsule profile must be full, navigation or recall",
            ));
        }
        Ok(profile.into())
    }

    pub fn enabled(&self, key: &str) -> bool {
        self.value[key].as_bool() == Some(true)
    }
    pub fn strings(&self, key: &str) -> Result<Vec<String>> {
        let items = self.value[key]
            .as_array()
            .ok_or_else(|| RuntimeError::invalid(format!("Configuration {key} must be a list")))?;
        items
            .iter()
            .map(|item| {
                item.as_str().map(String::from).ok_or_else(|| {
                    RuntimeError::invalid(format!("Configuration {key} requires strings"))
                })
            })
            .collect()
    }
}
