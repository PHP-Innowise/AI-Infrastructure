//! What a compiler agent is given: its instructions, the shape of its answer
//! and the template of the textual form. They are engine resources, not an
//! edition's skill, so no mirror, inventory or installed project carries them.
//!
//! This module also holds bounded deterministic binding extraction used at
//! compile time (Gate 1 validation) and at warm dispatch time so read-only
//! parameterized capsules can resolve inputs deterministically without an LLM.
use crate::capsule::{
    BINDING_BYTES, BINDING_PATTERN_CHARS, Binding, BindingOrigin, BindingType, SkillCapsule,
    is_admitted_value,
};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

pub const INSTRUCTIONS: &str = include_str!("../resources/compile/instructions.md");
pub const TEXTUAL_TEMPLATE: &str = include_str!("../resources/compile/textual.md");
const RESPONSE_SCHEMA: &str = include_str!("../resources/compile/response.schema.json");

pub const BINDING_REGEX_BYTES_LIMIT: usize = 64 * 1024;
pub const BINDING_TEXT_SCAN_LIMIT: usize = 16 * 1024;
pub const BINDING_WORKSPACE_SCAN_LIMIT: usize = 512;

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

/// Compiles a bounded linear-time regex for `workspace_path`, `route_arg`, or
/// `prompt_regex` bindings, rejecting empty-string-matching or over-budget patterns.
pub fn compile_bounded_binding_regex(pattern: &str) -> std::result::Result<regex::Regex, String> {
    if pattern.is_empty() || pattern.len() > BINDING_PATTERN_CHARS {
        return Err(format!(
            "`pattern` must be 1..={BINDING_PATTERN_CHARS} bytes."
        ));
    }
    let compiled = regex::RegexBuilder::new(pattern)
        .size_limit(BINDING_REGEX_BYTES_LIMIT)
        .dfa_size_limit(BINDING_REGEX_BYTES_LIMIT)
        .build()
        .map_err(|err| format!("invalid `pattern` regex: {err}"))?;
    if compiled.is_match("") {
        return Err("`pattern` regex must not match an empty string.".to_owned());
    }
    Ok(compiled)
}

/// Normalizes a candidate workspace-relative path and rejects escapes (`..`,
/// absolute paths, backslashes, or null bytes).
pub fn normalize_safe_relative_path(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.len() > 512
        || trimmed.contains('\0')
        || trimmed.contains('\\')
        || trimmed.starts_with('/')
    {
        return None;
    }
    let mut parts = Vec::new();
    for part in trimmed.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            segment => parts.push(segment),
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

/// Coerces an extracted text slice into the typed JSON value required by `kind`.
pub fn coerce_binding_scalar(kind: BindingType, raw: &str) -> Option<Value> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let value = match kind {
        BindingType::String => Value::String(trimmed.to_owned()),
        BindingType::Path => Value::String(normalize_safe_relative_path(trimmed)?),
        BindingType::Number => {
            if let Ok(int) = trimmed.parse::<i64>() {
                json!(int)
            } else if let Ok(float) = trimmed.parse::<f64>()
                && float.is_finite()
                && let Some(num) = serde_json::Number::from_f64(float)
            {
                Value::Number(num)
            } else {
                return None;
            }
        }
        BindingType::Boolean => match trimmed {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => return None,
        },
        BindingType::List => {
            if !trimmed.starts_with('[') {
                return None;
            }
            let parsed: Value = serde_json::from_str(trimmed).ok()?;
            if !kind.admits(&parsed) {
                return None;
            }
            parsed
        }
    };
    value_within_binding_limit(kind, &value).then_some(value)
}

/// Whether `value` obeys `kind`, scalar/flat-list rules, safe-path rules, and byte limits.
pub fn value_within_binding_limit(kind: BindingType, value: &Value) -> bool {
    if !is_admitted_value(value) || !kind.admits(value) {
        return false;
    }
    if kind == BindingType::Path
        && !value
            .as_str()
            .is_some_and(|path| normalize_safe_relative_path(path).as_deref() == Some(path))
    {
        return false;
    }
    serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= BINDING_BYTES)
}

/// Extracts bounded candidate path-like tokens from `route_args` and `message`.
pub fn extract_path_candidates(route_args: Option<&str>, message: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();
    for source in route_args.into_iter().chain(std::iter::once(message)) {
        if source.len() > BINDING_TEXT_SCAN_LIMIT {
            continue;
        }
        for raw in source.split_whitespace() {
            let token = raw.trim_matches(|c: char| {
                matches!(
                    c,
                    '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '<' | '>' | ',' | ';' | '.'
                )
            });
            if token.is_empty() || token.starts_with('-') || token.contains("://") {
                continue;
            }
            let clean = token.trim_end_matches(':');
            if let Some(norm) = normalize_safe_relative_path(clean)
                && seen.insert(norm.clone())
            {
                candidates.push(norm);
                if candidates.len() >= BINDING_WORKSPACE_SCAN_LIMIT {
                    return candidates;
                }
            }
        }
    }
    candidates
}

fn extract_regex_values(re: &regex::Regex, haystack: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for cap in re.captures_iter(haystack) {
        let matched = cap.get(1).or_else(|| cap.get(0)).map(|m| m.as_str().trim());
        if let Some(text) = matched
            && !text.is_empty()
            && seen.insert(text.to_owned())
        {
            out.push(text.to_owned());
        }
    }
    out
}

fn resolve_one_route_arg(
    binding: &Binding,
    route_args: Option<&str>,
    unpatterned_route_args: usize,
) -> std::result::Result<Option<Value>, ()> {
    let args = route_args.unwrap_or("").trim();
    if args.is_empty() {
        return Ok(None);
    }
    if args.len() > BINDING_TEXT_SCAN_LIMIT {
        return Err(());
    }
    if let Some(pattern) = &binding.pattern {
        let re = compile_bounded_binding_regex(pattern).map_err(|_| ())?;
        let matches = extract_regex_values(&re, args);
        return match matches.len() {
            0 => Ok(None),
            1 => coerce_binding_scalar(binding.kind, &matches[0])
                .map(Some)
                .ok_or(()),
            _ => Err(()),
        };
    }
    if unpatterned_route_args > 1 {
        return Err(());
    }
    match binding.kind {
        BindingType::String => coerce_binding_scalar(binding.kind, args)
            .map(Some)
            .ok_or(()),
        BindingType::Path => {
            if args.split_whitespace().count() > 1 {
                return Err(());
            }
            coerce_binding_scalar(binding.kind, args)
                .map(Some)
                .ok_or(())
        }
        BindingType::Number | BindingType::Boolean => {
            if args.split_whitespace().count() > 1 {
                return Err(());
            }
            coerce_binding_scalar(binding.kind, args)
                .map(Some)
                .ok_or(())
        }
        BindingType::List => coerce_binding_scalar(binding.kind, args)
            .map(Some)
            .ok_or(()),
    }
}

fn resolve_one_prompt_regex(
    binding: &Binding,
    message: &str,
) -> std::result::Result<Option<Value>, ()> {
    if message.len() > BINDING_TEXT_SCAN_LIMIT {
        return Err(());
    }
    let Some(pattern) = &binding.pattern else {
        return Err(());
    };
    let re = compile_bounded_binding_regex(pattern).map_err(|_| ())?;
    let matches = extract_regex_values(&re, message);
    match matches.len() {
        0 => Ok(None),
        1 => coerce_binding_scalar(binding.kind, &matches[0])
            .map(Some)
            .ok_or(()),
        _ => Err(()),
    }
}

/// Deterministically resolves all capsule bindings without calling an LLM.
///
/// Fails closed (`None`) if any binding has `origin: request`, if any extractor
/// finds multiple distinct candidate values (ambiguous), if a required pattern
/// is missing/invalid, or if a binding has no match and no valid default.
pub fn extract_deterministic_bindings<F>(
    capsule: &SkillCapsule,
    message: &str,
    route_args: Option<&str>,
    mut resolve_workspace_paths: F,
) -> Option<Map<String, Value>>
where
    F: FnMut(Option<&regex::Regex>, &[String]) -> std::result::Result<Vec<String>, ()>,
{
    let unpatterned_route_args = capsule
        .bindings
        .iter()
        .filter(|b| b.origin == BindingOrigin::RouteArg && b.pattern.is_none())
        .count();
    let mut path_candidates: Option<Vec<String>> = None;
    let mut resolved = Map::new();

    for binding in &capsule.bindings {
        let extracted: Option<Value> = match binding.origin {
            BindingOrigin::Request => return None,
            BindingOrigin::RouteArg => {
                resolve_one_route_arg(binding, route_args, unpatterned_route_args).ok()?
            }
            BindingOrigin::PromptRegex => resolve_one_prompt_regex(binding, message).ok()?,
            BindingOrigin::WorkspacePath => {
                if !matches!(binding.kind, BindingType::Path | BindingType::String) {
                    return None;
                }
                if route_args.is_some_and(|a| a.len() > BINDING_TEXT_SCAN_LIMIT)
                    || message.len() > BINDING_TEXT_SCAN_LIMIT
                {
                    return None;
                }
                let re = binding
                    .pattern
                    .as_deref()
                    .map(compile_bounded_binding_regex)
                    .transpose()
                    .ok()?;
                let tokens = path_candidates
                    .get_or_insert_with(|| extract_path_candidates(route_args, message));
                let matches = resolve_workspace_paths(re.as_ref(), tokens).ok()?;
                match matches.len() {
                    0 => None,
                    1 => coerce_binding_scalar(binding.kind, &matches[0]),
                    _ => return None,
                }
            }
        };

        let value = match extracted {
            Some(v) => v,
            None => match binding.default.clone() {
                Some(def) if value_within_binding_limit(binding.kind, &def) => def,
                _ => return None,
            },
        };
        if !value_within_binding_limit(binding.kind, &value) {
            return None;
        }
        resolved.insert(binding.name.clone(), value);
    }

    Some(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capsule::{Capabilities, CapsuleKind, Form, Mu, Network};

    fn capsule_with(bindings: Vec<Binding>) -> SkillCapsule {
        SkillCapsule {
            form: Form::Hybrid,
            kind: CapsuleKind::Skill,
            mu: Mu {
                name: "test".into(),
                description: "test".into(),
            },
            skills: vec![],
            source_edition: None,
            tool_chain_sha256: None,
            capabilities: Capabilities {
                network: Network::None,
                runtimes: vec![],
                allow_ignored: vec![],
            },
            bindings,
            programs: vec![],
            steps: vec![],
            verification: vec![],
        }
    }

    #[test]
    fn route_arg_and_prompt_regex_extract_unambiguously_and_fail_closed_on_ambiguity() {
        let cap = capsule_with(vec![
            Binding {
                name: "period".into(),
                kind: BindingType::Number,
                origin: BindingOrigin::PromptRegex,
                description: None,
                pattern: Some(r"\b(20\d{2})\b".into()),
                default: None,
            },
            Binding {
                name: "scope".into(),
                kind: BindingType::String,
                origin: BindingOrigin::RouteArg,
                description: None,
                pattern: Some(r"^([a-z0-9_-]+)".into()),
                default: Some(json!("all")),
            },
        ]);

        let ok =
            extract_deterministic_bindings(&cap, "Check 2026 taxes", Some("q1"), |_, _| Ok(vec![]))
                .expect("unique prompt regex + route arg extracts");
        assert_eq!(ok.get("period"), Some(&json!(2026)));
        assert_eq!(ok.get("scope"), Some(&json!("q1")));

        // Default used only on 0 matches for route_arg.
        let with_default =
            extract_deterministic_bindings(&cap, "Check 2026 taxes", None, |_, _| Ok(vec![]))
                .expect("default fills absent route_arg");
        assert_eq!(with_default.get("scope"), Some(&json!("all")));

        // Multiple distinct prompt_regex matches fail closed even when default exists.
        assert!(
            extract_deterministic_bindings(&cap, "Compare 2025 vs 2026", Some("q1"), |_, _| Ok(
                vec![]
            ))
            .is_none()
        );
    }

    #[test]
    fn workspace_path_and_request_binding_rules_fail_closed() {
        let cap_ws = capsule_with(vec![Binding {
            name: "target".into(),
            kind: BindingType::Path,
            origin: BindingOrigin::WorkspacePath,
            description: None,
            pattern: Some(r"^invoices/.*\.pdf$".into()),
            default: None,
        }]);

        // Unambiguous single workspace match succeeds.
        let single =
            extract_deterministic_bindings(&cap_ws, "audit invoices/jan.pdf", None, |_, tokens| {
                assert!(tokens.contains(&"invoices/jan.pdf".to_owned()));
                Ok(vec!["invoices/jan.pdf".to_owned()])
            })
            .expect("single path match");
        assert_eq!(single.get("target"), Some(&json!("invoices/jan.pdf")));

        // Multiple workspace matches fail closed.
        assert!(
            extract_deterministic_bindings(&cap_ws, "audit invoices", None, |_, _| {
                Ok(vec![
                    "invoices/jan.pdf".to_owned(),
                    "invoices/feb.pdf".to_owned(),
                ])
            })
            .is_none()
        );

        // Origin::request always fails closed in deterministic extraction.
        let cap_req = capsule_with(vec![Binding {
            name: "note".into(),
            kind: BindingType::String,
            origin: BindingOrigin::Request,
            description: None,
            pattern: None,
            default: Some(json!("fallback")),
        }]);
        assert!(
            extract_deterministic_bindings(&cap_req, "anything", None, |_, _| Ok(vec![])).is_none()
        );
    }

    #[test]
    fn bounded_regex_rejects_empty_matches_and_syntax_errors() {
        assert!(compile_bounded_binding_regex(".*").is_err());
        assert!(compile_bounded_binding_regex("").is_err());
        assert!(compile_bounded_binding_regex("[unclosed").is_err());
        assert!(compile_bounded_binding_regex(r"\b(20\d{2})\b").is_ok());
    }
}
