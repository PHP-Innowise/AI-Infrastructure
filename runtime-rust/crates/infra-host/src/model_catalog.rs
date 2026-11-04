//! Public native model metadata. Reads no credentials and makes no account/network requests.
use crate::{
    HostError, Result,
    process::{self, Limits},
};
use infra_core::bank::publication::PublicationFiles;
use serde_json::{Value as V, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock, atomic::AtomicBool},
};
const LIMIT: usize = 4 * 1024 * 1024;
static CACHE: OnceLock<tokio::sync::Mutex<BTreeMap<String, V>>> = OnceLock::new();
pub fn codex_home() -> PathBuf {
    let value = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".codex"));
    if let Ok(relative) = value.strip_prefix("~") {
        home().join(relative)
    } else {
        value
    }
}
pub fn home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}
pub fn efforts(provider: &str) -> Result<Vec<String>> {
    let values: &[&str] = match provider {
        "claude" => &["low", "medium", "high", "xhigh", "max", "ultracode"],
        "codex" => &[
            "none",
            "minimal",
            "low",
            "medium",
            "high",
            "xhigh",
            "max",
            "ultra",
            "persistent",
        ],
        "cursor" => &[],
        _ => return Err(HostError::invalid("Unknown provider.")),
    };
    Ok(values.iter().map(|s| (*s).into()).collect())
}
pub fn value(value: &str, label: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.starts_with('-')
        || value.chars().any(|c| c < ' ' || c == '\u{7f}')
    {
        Err(HostError::invalid(format!(
            "{label} must be a nonempty value, not an option."
        )))
    } else {
        Ok(())
    }
}
fn catalog_rows(data: &V) -> Vec<V> {
    let mut rows = vec![];
    let mut seen = BTreeSet::new();
    for entry in data["models"].as_array().into_iter().flatten().take(1000) {
        let Some(id) = entry["slug"]
            .as_str()
            .filter(|s| s.chars().count() <= 120 && value(s, "model").is_ok())
        else {
            continue;
        };
        if entry["visibility"] != "list" || !seen.insert(id.to_owned()) {
            continue;
        }
        let Some(levels) = entry["supported_reasoning_levels"].as_array() else {
            continue;
        };
        let label = entry["display_name"]
            .as_str()
            .filter(|s| {
                !s.trim().is_empty()
                    && s.chars().count() <= 120
                    && !s.chars().any(|c| c < ' ' || c == '\u{7f}')
            })
            .unwrap_or(id);
        let mut efforts = vec![];
        for row in levels {
            if let Some(e) = row["effort"].as_str().filter(|s| {
                s.len() <= 32
                    && s.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
                    && s.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'
                    })
            }) && !efforts.contains(&e.to_owned())
            {
                efforts.push(e.to_owned());
            }
        }
        rows.push((
            entry["priority"].as_i64().unwrap_or(1000),
            json!({"id":id,"label":label,"efforts":efforts}),
        ));
    }
    rows.sort_by_key(|(p, _)| *p);
    rows.into_iter().take(100).map(|(_, v)| v).collect()
}
pub async fn model_options(
    provider: &str,
    executable: Option<&Path>,
    runner_lock: Option<Arc<std::fs::File>>,
) -> Result<V> {
    let _ = efforts(provider)?;
    let cache = CACHE.get_or_init(|| tokio::sync::Mutex::new(BTreeMap::new()));
    let mut cache = cache.lock().await;
    if let Some(v) = cache.get(provider) {
        return Ok(v.clone());
    }
    let mut all = efforts(provider)?;
    let result = match provider {
        "claude" => {
            let mut models = vec![];
            for (id, label, kind) in [
                ("claude-fable-5-1", "Fable 5.1", 2),
                ("claude-fable-5", "Fable 5", 2),
                ("fable", "Fable (auto)", 2),
                ("claude-opus-5", "Opus 5", 2),
                ("claude-opus-4-8", "Opus 4.8", 2),
                ("claude-opus-4-7", "Opus 4.7", 2),
                ("claude-opus-4-6", "Opus 4.6", 1),
                ("opus", "Opus (auto)", 2),
                ("claude-sonnet-5", "Sonnet 5", 2),
                ("claude-sonnet-4-6", "Sonnet 4.6", 1),
                ("sonnet", "Sonnet (auto)", 2),
                ("claude-haiku-4-5-20251001", "Haiku 4.5", 0),
                ("haiku", "Haiku (auto)", 0),
            ] {
                let levels = if kind == 2 {
                    all.clone()
                } else if kind == 1 {
                    vec!["low".into(), "medium".into(), "high".into(), "max".into()]
                } else {
                    vec![]
                };
                models.push(json!({"id":id,"label":label,"efforts":levels}));
            }
            json!({"models":models,"efforts":all,"detail":"Versioned entries request an explicit model ID; auto aliases follow your Claude configuration. Account availability and effective effort are checked by Claude; custom model support may differ."})
        }
        "cursor" => {
            json!({"models":[],"efforts":[],"detail":"Use the native default or a custom model/variant. Cursor has no separate effort flag; its account model list requires a native account request."})
        }
        _ => {
            let folder = codex_home();
            let mut models = PublicationFiles::new(&folder)
                .ok()
                .and_then(|mut files| files.read("models_cache.json", LIMIT).ok().flatten())
                .and_then(|raw| crate::strict_json::parse(&raw, LIMIT).ok())
                .map(|v| catalog_rows(&v))
                .unwrap_or_default();
            let mut origin = "Local cached Codex catalog";
            if models.is_empty() {
                if let Some(executable) = executable {
                    let args = crate::provider::command_argv(
                        &[
                            executable.to_string_lossy().into_owned(),
                            "debug".into(),
                            "models".into(),
                            "--bundled".into(),
                        ],
                        Some("codex"),
                    )?;
                    if let Ok(output) = process::run_with_lock(
                        &args,
                        &std::env::current_dir().unwrap_or_else(|_| home()),
                        &BTreeMap::new(),
                        &[],
                        Limits {
                            seconds: Some(3),
                            output: LIMIT,
                            line: LIMIT,
                        },
                        Arc::new(AtomicBool::new(false)),
                        runner_lock,
                        |_| Ok(()),
                    )
                    .await
                        && output.code == 0
                    {
                        models = crate::strict_json::parse(&output.bytes, LIMIT)
                            .map(|v| catalog_rows(&v))
                            .unwrap_or_default();
                    }
                }
                origin = if models.is_empty() {
                    "No local Codex model catalog is available"
                } else {
                    "Bundled offline Codex catalog"
                };
            }
            for model in &models {
                for effort in model["efforts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(V::as_str)
                {
                    if !all.iter().any(|s| s == effort) {
                        all.push(effort.into());
                    }
                }
            }
            json!({"models":models,"efforts":all,"detail":format!("{origin}. Availability is checked by the native CLI; custom models may support different effort levels.")})
        }
    };
    cache.insert(provider.into(), result.clone());
    Ok(result)
}
pub fn validate_model_effort(
    options: &V,
    provider: &str,
    model: Option<&str>,
    effort: Option<&str>,
    agents: bool,
) -> Result<()> {
    let _ = efforts(provider)?;
    if let Some(model) = model {
        value(model, "model")?;
        if model.chars().count() > 120 {
            return Err(HostError::invalid("Invalid model name."));
        }
    }
    let Some(effort) = effort else { return Ok(()) };
    let mut levels = &options["efforts"];
    if let Some(row) = options["models"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|v| v["id"].as_str() == model)
    {
        levels = &row["efforts"];
    }
    if !levels.as_array().into_iter().flatten().any(|v| v == effort) {
        return Err(HostError::invalid(
            "This provider/model does not support the selected thinking effort.",
        ));
    }
    if effort == "ultracode" && !agents {
        return Err(HostError::invalid(
            "Ultracode requires additional agents. Enable them in a new session or choose another effort.",
        ));
    }
    Ok(())
}
fn selected<'a>(options: &'a V, previous: &'a V, key: &str) -> &'a V {
    options.get(key).unwrap_or(&previous[key])
}
fn model_settings(catalog: &V, provider: &str, options: &V, previous: &V) -> Result<(V, V)> {
    let normalize = |value: &V| -> Result<V> {
        if value.is_null() || value == "" {
            Ok(V::Null)
        } else if value.is_string() {
            Ok(value.clone())
        } else {
            Err(HostError::invalid(
                "Model and thinking effort must be text or null.",
            ))
        }
    };
    let model = normalize(selected(options, previous, "model"))?;
    let effort = normalize(selected(options, previous, "thinking_effort"))?;
    let agents = selected(options, previous, "agents_enabled")
        .as_bool()
        .unwrap_or(false);
    validate_model_effort(catalog, provider, model.as_str(), effort.as_str(), agents)?;
    Ok((model, effort))
}
pub fn routed_model_settings(
    catalog: &V,
    provider: &str,
    options: &V,
    workflow: &str,
    mode: &str,
    sdd: Option<&V>,
    previous: Option<&V>,
) -> Result<(V, V, V)> {
    let empty = V::Null;
    let previous = previous.unwrap_or(&empty);
    let routing = selected(options, previous, "model_routing");
    if routing.is_null() {
        let (model, effort) = model_settings(catalog, provider, options, previous)?;
        return Ok((model, effort, V::Null));
    }
    if workflow == "fleet-review" || previous["creator"].is_object() {
        return Err(HostError::invalid(
            "Separate planning/editing models are available for ordinary and SDD sessions.",
        ));
    }
    if routing
        .as_object()
        .is_none_or(|o| o.len() != 2 || !o.contains_key("plan") || !o.contains_key("edit"))
    {
        return Err(HostError::invalid(
            "Model routing requires plan and edit model settings, or null to disable it.",
        ));
    }
    let mut normalized = json!({});
    for role in ["plan", "edit"] {
        let mut item = routing[role].clone();
        if item.as_object().is_none_or(|o| {
            o.len() != 2 || !o.contains_key("model") || !o.contains_key("thinking_effort")
        }) {
            return Err(HostError::invalid(
                "Each model role requires model and thinking_effort.",
            ));
        }
        item["agents_enabled"] = selected(options, previous, "agents_enabled").clone();
        let (model, effort) = model_settings(catalog, provider, &item, &empty)?;
        normalized[role] = json!({"model":model,"thinking_effort":effort});
    }
    let role = if workflow == "sdd" {
        if sdd.is_some_and(|s| s["phase"] == "implement") {
            "edit"
        } else {
            "plan"
        }
    } else {
        mode
    };
    if !["plan", "edit"].contains(&role) {
        return Err(HostError::invalid("Unknown model routing role."));
    }
    Ok((
        normalized[role]["model"].clone(),
        normalized[role]["thinking_effort"].clone(),
        normalized,
    ))
}

pub fn cached_options(provider: &str) -> Option<V> {
    CACHE.get()?.try_lock().ok()?.get(provider).cloned()
}

#[cfg(test)]
mod contract_tests {
    use super::*;
    fn catalog() -> V {
        json!({"models":[
            {"id":"planner","efforts":["low","high"]},
            {"id":"editor","efforts":["medium","max"]},
            {"id":"small","efforts":[]}
        ],"efforts":["low","medium","high","max","ultracode"]})
    }
    fn routed() -> V {
        json!({"model_routing":{
            "plan":{"model":"planner","thinking_effort":"high"},
            "edit":{"model":"editor","thinking_effort":"max"}
        },"agents_enabled":false})
    }
    #[test]
    fn native_turn_selects_role_and_preserves_both_reviewed_settings() {
        let (model, effort, routing) = routed_model_settings(
            &catalog(),
            "claude",
            &routed(),
            "native",
            "edit",
            None,
            None,
        )
        .unwrap();
        assert_eq!(model, "editor");
        assert_eq!(effort, "max");
        assert_eq!(routing["plan"]["model"], "planner");
        assert_eq!(routing["edit"]["model"], "editor");
    }
    #[test]
    fn sdd_only_implement_uses_the_editing_role() {
        for phase in [
            "initialize",
            "specify",
            "plan",
            "tasks",
            "implement",
            "review",
        ] {
            let (model, _, _) = routed_model_settings(
                &catalog(),
                "claude",
                &routed(),
                "sdd",
                "edit",
                Some(&json!({"phase":phase})),
                None,
            )
            .unwrap();
            assert_eq!(
                model,
                if phase == "implement" {
                    "editor"
                } else {
                    "planner"
                },
                "{phase}"
            );
        }
    }
    #[test]
    fn omitted_followup_settings_preserve_routing_and_explicit_null_disables_it() {
        let previous = routed();
        let (model, _, routing) = routed_model_settings(
            &catalog(),
            "claude",
            &json!({}),
            "native",
            "plan",
            None,
            Some(&previous),
        )
        .unwrap();
        assert_eq!(model, "planner");
        assert_eq!(routing["edit"]["model"], "editor");
        let (_, _, disabled) = routed_model_settings(
            &catalog(),
            "claude",
            &json!({"model_routing":null}),
            "native",
            "plan",
            None,
            Some(&previous),
        )
        .unwrap();
        assert!(disabled.is_null());
    }
    #[test]
    fn unsupported_effort_and_unreviewed_routing_shapes_are_rejected() {
        assert!(
            validate_model_effort(&catalog(), "claude", Some("small"), Some("high"), false)
                .is_err()
        );
        assert!(
            validate_model_effort(&catalog(), "claude", None, Some("ultracode"), false).is_err()
        );
        assert!(validate_model_effort(&catalog(), "claude", None, Some("ultracode"), true).is_ok());
        let mut options = routed();
        options["model_routing"]["edit"]["extra"] = json!("ignored?");
        assert!(
            routed_model_settings(&catalog(), "claude", &options, "native", "edit", None, None)
                .is_err()
        );
        assert!(
            routed_model_settings(
                &catalog(),
                "claude",
                &routed(),
                "fleet-review",
                "plan",
                None,
                None
            )
            .is_err()
        );
    }
}
