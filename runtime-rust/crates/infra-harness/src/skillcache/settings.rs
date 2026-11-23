//! The session's `skillcache` option.
use crate::{HarnessError, Result};
use serde_json::{Value, json};

/// `off`: nothing is recorded or used. `observe`: a cold turn's trace is kept.
/// `auto`: a qualified capsule is used when one matches, otherwise the turn is
/// cold and observed. `warm_only`: a matching capsule is used, nothing is
/// recorded.
pub const MODES: [&str; 4] = ["off", "observe", "auto", "warm_only"];
/// Evaluation-only ablation arms; rejected on normal sessions unless enabled.
pub const EVAL_ARMS: [&str; 6] = [
    "cold",
    "raw_trace",
    "written_procedure",
    "skillcache_agent",
    "skillcache_executor",
    "skillcache_no_replay",
];
pub const EVAL_SETTING: &str = "skillcache.eval_arms";
pub const WORKSPACE_ALLOWLIST: &str = "skillcache.workspace_allowlist";
/// Opt-in flag enabling skillless read-only `ToolChain` capsule capture/reuse.
pub const TOOLCHAIN_CAPSULES: &str = "skillcache.tool_chain_capsules";

/// Whether skillless read-only `ToolChain` capsules are enabled for `session`.
pub fn tool_chain_enabled(store: &crate::store::SessionStore, session: &Value) -> bool {
    session["skillcache"]["tool_chain"] == true
        || store
            .setting(TOOLCHAIN_CAPSULES)
            .ok()
            .flatten()
            .is_some_and(|value| value == true || value["enabled"] == true)
}

/// Optional project/store-wide safe directory allowlist overriding default
/// dependency ignores (`node_modules`, `vendor`, `.venv`, `dist`, `build`, `.next`).
pub fn workspace_allowlist(store: &crate::store::SessionStore) -> Vec<String> {
    store
        .setting(WORKSPACE_ALLOWLIST)
        .ok()
        .flatten()
        .and_then(|value| value.as_array().cloned())
        .into_iter()
        .flatten()
        .filter_map(|item| item.as_str().map(str::to_owned))
        .filter(|name| infra_skillcache::executor::is_safe_allowlist_name(name))
        .collect()
}

fn optional_id(value: Option<&Value>, label: &str) -> Result<Option<String>> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let text = crate::store::text(value, label, 128)?;
    if text.starts_with(['-', '.'])
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        return Err(HarnessError::invalid(format!("Invalid {label}.")));
    }
    Ok(Some(text))
}

fn optional_skill(value: Option<&Value>) -> Result<Option<String>> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let text = crate::store::text(value, "skill", 200)?;
    if text.starts_with('-')
        || text.contains('\\')
        || text.contains("..")
        || text.chars().any(char::is_control)
    {
        return Err(HarnessError::invalid("Invalid SkillCache skill hint."));
    }
    Ok(Some(text))
}

/// The stored form of the option, or null for `off`.
pub fn validate(value: &Value) -> Result<Value> {
    if value.is_null() {
        return Ok(Value::Null);
    }
    let invalid = || {
        HarnessError::invalid(
            "SkillCache settings are {\"mode\": \"off\"|\"observe\"|\"auto\"|\"warm_only\", \"compiler\": {\"provider\": \"claude\", \"model\": ...}, \"compile_after_observe\": false}.",
        )
    };
    let settings = value.as_object().ok_or_else(invalid)?;
    if settings.keys().any(|key| {
        ![
            "mode",
            "compiler",
            "compile_after_observe",
            "skill",
            "eval_arm",
            "trace_generation",
            "capsule_id",
            "tool_chain",
            "direct_dispatch",
            "edition",
        ]
        .contains(&key.as_str())
    }) {
        return Err(invalid());
    }
    let mode = settings
        .get("mode")
        .and_then(Value::as_str)
        .filter(|mode| MODES.contains(mode))
        .ok_or_else(invalid)?;
    let automatic = match settings.get("compile_after_observe") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(automatic)) => *automatic,
        Some(_) => return Err(invalid()),
    };
    let compiler = match settings.get("compiler") {
        None | Some(Value::Null) => json!({"provider": "claude", "model": null}),
        Some(compiler) => {
            let fields = compiler.as_object().ok_or_else(invalid)?;
            if fields
                .keys()
                .any(|key| !["provider", "model"].contains(&key.as_str()))
            {
                return Err(invalid());
            }
            // Only Claude can be started with no shell at all, which is what
            // "a compiler runs nothing" rests on.
            if fields
                .get("provider")
                .is_some_and(|provider| provider != "claude")
            {
                return Err(HarnessError::invalid(
                    "The SkillCache compiler is Claude: it is the only provider that can be started without a shell.",
                ));
            }
            let model = match fields.get("model") {
                None | Some(Value::Null) => Value::Null,
                Some(model) => {
                    let model = crate::store::text(model, "compiler model", 120)?;
                    if model.starts_with('-') || model.chars().any(char::is_control) {
                        return Err(HarnessError::invalid("Invalid compiler model."));
                    }
                    model.into()
                }
            };
            json!({"provider": "claude", "model": model})
        }
    };
    let skill = optional_skill(settings.get("skill"))?;
    let eval_arm = match settings.get("eval_arm") {
        None | Some(Value::Null) => None,
        Some(arm) => Some(
            arm.as_str()
                .filter(|arm| EVAL_ARMS.contains(arm))
                .ok_or_else(|| HarnessError::invalid("Invalid SkillCache evaluation arm."))?
                .to_string(),
        ),
    };
    let trace_generation = optional_id(settings.get("trace_generation"), "trace_generation")?;
    let capsule_id = optional_id(settings.get("capsule_id"), "capsule_id")?;
    let tool_chain = match settings.get("tool_chain") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(enabled)) => *enabled,
        Some(_) => return Err(invalid()),
    };
    let direct_dispatch = match settings.get("direct_dispatch") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(enabled)) => *enabled,
        Some(_) => return Err(invalid()),
    };
    let edition = match settings.get("edition") {
        None | Some(Value::Null) => None,
        Some(val) => {
            let text = crate::store::text(val, "edition", 64)?;
            if text.contains("..") || text.contains(['/', '\\']) {
                return Err(HarnessError::invalid("Invalid SkillCache edition."));
            }
            Some(text)
        }
    };
    if mode == "off" && eval_arm.is_none() && trace_generation.is_none() && capsule_id.is_none() {
        return Ok(Value::Null);
    }
    let mut stored =
        json!({"mode": mode, "compiler": compiler, "compile_after_observe": automatic});
    if let Some(skill) = skill {
        stored["skill"] = skill.into();
    }
    if let Some(arm) = eval_arm {
        stored["eval_arm"] = arm.into();
    }
    if let Some(generation) = trace_generation {
        stored["trace_generation"] = generation.into();
    }
    if let Some(capsule) = capsule_id {
        stored["capsule_id"] = capsule.into();
    }
    if tool_chain {
        stored["tool_chain"] = true.into();
    }
    if direct_dispatch {
        stored["direct_dispatch"] = true.into();
    }
    if let Some(edition) = edition {
        stored["edition"] = edition.into();
    }
    Ok(stored)
}

/// The session's mode; `off` when it never opted in.
pub fn mode(session: &Value) -> &str {
    session["skillcache"]["mode"]
        .as_str()
        .filter(|mode| MODES.contains(mode))
        .unwrap_or("off")
}
/// Whether a cold turn of this session keeps its trace.
pub fn records(session: &Value) -> bool {
    matches!(mode(session), "observe" | "auto") && eval_arm(session).is_none()
}
/// Whether a turn of this session may run a capsule.
pub fn warm(session: &Value) -> bool {
    matches!(mode(session), "auto" | "warm_only")
}
pub fn skill_hint(session: &Value) -> Option<&str> {
    session["skillcache"]["skill"]
        .as_str()
        .filter(|skill| !skill.trim().is_empty())
}
pub fn eval_arm(session: &Value) -> Option<&str> {
    session["skillcache"]["eval_arm"]
        .as_str()
        .filter(|arm| EVAL_ARMS.contains(arm))
}
pub fn trace_generation(session: &Value) -> Option<&str> {
    session["skillcache"]["trace_generation"]
        .as_str()
        .filter(|generation| !generation.is_empty())
}
pub fn capsule_pin(session: &Value) -> Option<&str> {
    session["skillcache"]["capsule_id"]
        .as_str()
        .filter(|id| !id.is_empty())
}
pub fn eval_enabled(store: &crate::store::SessionStore) -> bool {
    store
        .setting(EVAL_SETTING)
        .ok()
        .flatten()
        .is_some_and(|value| value == true)
}
pub fn has_eval_options(options: &Value) -> bool {
    let sc = &options["skillcache"];
    !sc.is_null()
        && (!sc["eval_arm"].is_null()
            || !sc["trace_generation"].is_null()
            || !sc["capsule_id"].is_null())
}

/// SkillCache applies to an ordinary session: one agent, one provider, the
/// native workflow. The specialized workflows run their own protocols.
pub fn admit(options: &Value) -> Result<()> {
    admit_with_eval(options, false)
}

/// As [`admit`], additionally gating evaluation-only ablation options behind
/// `eval_allowed`.
pub fn admit_with_eval(options: &Value, eval_allowed: bool) -> Result<()> {
    if options["skillcache"].is_null() {
        return Ok(());
    }
    if has_eval_options(options) && !eval_allowed {
        return Err(HarnessError::invalid(
            "SkillCache ablation arms are available only in evaluation runs.",
        ));
    }
    let special = [
        "fleet",
        "clash",
        "sdd",
        "system_explain",
        "creator",
        "system_run",
        "system_discovery",
    ]
    .iter()
    .any(|key| !options[*key].is_null());
    if special || options["workflow"].as_str().unwrap_or("native") != "native" {
        return Err(HarnessError::invalid(
            "SkillCache applies to ordinary sessions: not to Fleet, Clash, SDD, review, system or creator workflows.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_off_and_off_stores_nothing() {
        assert_eq!(validate(&Value::Null).unwrap(), Value::Null);
        assert_eq!(validate(&json!({"mode": "off"})).unwrap(), Value::Null);
        assert_eq!(mode(&json!({})), "off");
        assert!(!records(&json!({})) && !warm(&json!({})));
        assert_eq!(mode(&json!({"skillcache": {"mode": "nonsense"}})), "off");
    }

    #[test]
    fn each_mode_records_or_runs_as_named() {
        let session = |mode: &str| json!({"skillcache": validate(&json!({"mode": mode})).unwrap()});
        assert!(records(&session("observe")) && !warm(&session("observe")));
        assert!(records(&session("auto")) && warm(&session("auto")));
        assert!(!records(&session("warm_only")) && warm(&session("warm_only")));
        assert_eq!(
            validate(&json!({"mode": "auto"})).unwrap(),
            json!({"mode": "auto", "compiler": {"provider": "claude", "model": null}, "compile_after_observe": false})
        );
        assert_eq!(
            validate(&json!({"mode": "observe", "compiler": {"model": "claude-fixture"}, "compile_after_observe": true})).unwrap(),
            json!({"mode": "observe", "compiler": {"provider": "claude", "model": "claude-fixture"}, "compile_after_observe": true})
        );
    }

    #[test]
    fn unknown_settings_and_other_compilers_are_refused() {
        for value in [
            json!("auto"),
            json!({}),
            json!({"mode": "always"}),
            json!({"mode": "auto", "extra": 1}),
            json!({"mode": "auto", "compile_after_observe": "yes"}),
            json!({"mode": "auto", "compiler": {"provider": "codex"}}),
            json!({"mode": "auto", "compiler": {"provider": "claude", "effort": "high"}}),
            json!({"mode": "auto", "compiler": {"model": "--dangerous"}}),
        ] {
            assert!(validate(&value).is_err(), "{value}");
        }
    }

    #[test]
    fn specialized_workflows_do_not_take_skillcache() {
        let on = json!({"mode": "observe"});
        assert!(admit(&json!({"skillcache": on, "workflow": "native"})).is_ok());
        assert!(admit(&json!({"workflow": "fleet-review", "fleet": {}})).is_ok());
        for options in [
            json!({"skillcache": on, "workflow": "review"}),
            json!({"skillcache": on, "workflow": "native", "clash": {"challenger": "codex"}}),
            json!({"skillcache": on, "workflow": "sdd", "sdd": {}}),
            json!({"skillcache": on, "workflow": "native", "system_explain": {}}),
        ] {
            assert!(admit(&options).is_err(), "{options}");
        }
    }

    #[test]
    fn skill_hint_and_eval_options_validate_and_require_eval_gate() {
        let valid_skill = validate(&json!({"mode": "warm_only", "skill": "pdf"})).unwrap();
        assert_eq!(skill_hint(&json!({"skillcache": valid_skill})), Some("pdf"));
        let path_skill =
            validate(&json!({"mode": "warm_only", "skill": ".claude/skills/pdf/SKILL.md"}))
                .unwrap();
        assert_eq!(
            skill_hint(&json!({"skillcache": path_skill})),
            Some(".claude/skills/pdf/SKILL.md")
        );
        for invalid in ["../bad", "bad\\name", "-bad"] {
            assert!(
                validate(&json!({"mode": "warm_only", "skill": invalid})).is_err(),
                "{invalid}"
            );
        }

        for arm in EVAL_ARMS {
            let cfg = validate(&json!({
                "mode": "warm_only",
                "eval_arm": arm,
                "trace_generation": "gen-1",
                "capsule_id": "cap-1"
            }))
            .unwrap();
            let session = json!({"skillcache": cfg});
            assert_eq!(eval_arm(&session), Some(arm));
            assert_eq!(trace_generation(&session), Some("gen-1"));
            assert_eq!(capsule_pin(&session), Some("cap-1"));
            assert!(has_eval_options(&session));
            assert!(admit(&session).is_err());
            assert!(admit_with_eval(&session, false).is_err());
            assert!(admit_with_eval(&session, true).is_ok());
        }
        assert!(validate(&json!({"mode": "warm_only", "eval_arm": "bogus"})).is_err());
        assert!(validate(&json!({"mode": "warm_only", "trace_generation": "../bad"})).is_err());
        assert!(validate(&json!({"mode": "warm_only", "capsule_id": "-bad"})).is_err());
    }
}
