use crate::common;
use infra_core::{Result, RuntimeError, domain::json as native_json, fs as safe};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};
const PATHS: &[&str] = &[
    "AGENTS.md",
    "CLAUDE.md",
    ".claude/CLAUDE.md",
    ".claude/DOD.md",
    ".claude/GOLDEN-PRINCIPLES.md",
    ".claude/STABILIZATION.md",
    ".agents/skills",
    ".claude/skills/skill-creator/SKILL.md",
    ".cursor/skills/skill-creator/SKILL.md",
    ".claude/skills/SKILL FLOW.md",
    ".agents/skills/SKILL FLOW.md",
    ".claude/agents/*-agent.md",
    ".claude/commands",
    ".claude/settings.json",
    ".cursor/rules",
    ".cursor/hooks.json",
    ".codex/hooks.json",
    ".codex/config.toml",
];
fn retired_policy_source(path: &str) -> bool {
    // The canonical skill subtree includes historical Python helper/oracle
    // names in an unstaged index after retirement. They are not native payload.
    if path.starts_with(".agents/skills/") && path.ends_with(".py") {
        return true;
    }
    path.strip_prefix(".agents/skills/memory-seed/assets/scripts/")
        .is_some_and(|name| !name.contains('/') && name.ends_with(".sh"))
}
pub fn lock(root: &Path, edition: &str, write: bool) -> Result<Value> {
    let edition = common::edition(edition)?;
    let base = root.join(edition);
    let specs = PATHS
        .iter()
        .map(|path| format!("{edition}/{path}"))
        .collect::<Vec<_>>();
    let prefix = format!("{edition}/");
    let names = common::tracked(root, &specs)?;
    if names.is_empty() {
        return Err(RuntimeError::not_found(
            "No model-facing files tracked for edition",
        ));
    }
    let mut files = BTreeMap::new();
    let mut models = BTreeMap::new();
    let mut findings = Vec::new();
    let model = regex::Regex::new(r"(?m)^model:\s*(\S+)\s*$")
        .map_err(|_| RuntimeError::invalid("Model expression is invalid"))?;
    for name in names {
        let relative = name
            .strip_prefix(&prefix)
            .ok_or_else(|| RuntimeError::policy("Policy path escaped edition"))?;
        if relative == ".accelerator-policy-lock.json" || retired_policy_source(relative) {
            continue;
        }
        let bytes = safe::read_regular(&common::under(root, &name)?)?;
        files.insert(relative.to_string(), common::sha256(&bytes));
        if relative.starts_with(".claude/agents/") && relative.ends_with("-agent.md") {
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| RuntimeError::invalid("Agent policy is not UTF-8"))?;
            if let Some(found) = model.captures(text) {
                let value = found[1].to_string();
                let stem = Path::new(relative)
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| RuntimeError::invalid("Agent name is not UTF-8"))?;
                let allowed = if edition == "Infrastructure-Creator" {
                    vec!["opus", "sonnet"]
                } else {
                    vec!["opus", "sonnet", "haiku"]
                };
                if !allowed.contains(&value.as_str()) {
                    findings.push(format!("{relative}: invalid model {value}"));
                }
                models.insert(stem.to_string(), value);
            }
        }
    }
    let file_value = serde_json::to_value(files)?;
    let manifest = native_json::serialize(&file_value, true, true, false, false)?;
    let release = if base.join("VERSION").is_file() {
        safe::read_text(&base.join("VERSION"))?.trim().into()
    } else {
        String::from("unknown")
    };
    let value = json!({"schema_version":1,"edition":edition,"release":release,"policy_digest":common::sha256(manifest.as_bytes()),"agent_models":models,"files":file_value});
    let relative = format!("{edition}/.accelerator-policy-lock.json");
    if write {
        if !findings.is_empty() {
            return Ok(
                json!({"edition":edition,"passed":false,"findings":findings,"written":false,"files":file_value.as_object().map_or(0,|files|files.len()),"policy_digest":value["policy_digest"]}),
            );
        }
        common::write(root, &relative, &common::json_bytes(&value)?)?;
    } else {
        match common::read_json(&root.join(&relative)) {
            Ok(stored) => {
                for key in [
                    "schema_version",
                    "edition",
                    "policy_digest",
                    "agent_models",
                    "files",
                ] {
                    if stored[key] != value[key] {
                        findings.push(format!("Policy lock field differs: {key}"));
                    }
                }
            }
            Err(error) => findings.push(format!("Policy lock cannot be read: {error}")),
        }
    }
    Ok(
        json!({"edition":edition,"passed":findings.is_empty(),"findings":findings,"written":write,"files":file_value.as_object().map_or(0,|files|files.len()),"policy_digest":value["policy_digest"]}),
    )
}
