//! Reviewed profile → deterministic, data-only native hook policy.
use crate::{common::*, ownership};
use infra_core::bank::publication::PublicationFiles;
use infra_core::{Result, RuntimeError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json as object};
use std::{collections::BTreeSet, path::Path};
pub const PATH: &str = "project-brain/config/hook-policy.json";
pub const LIMIT: usize = 64 * 1024;
pub const SCHEMA_PATH: &str = "project-brain/schemas/hook-policy.schema.json";
const START: &str = "<!-- infra-hook-policy-selection-v1 -->";
const END: &str = "<!-- /infra-hook-policy-selection-v1 -->";
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Prefix {
    tool: String,
    argv: Vec<String>,
    options_with_values: Vec<String>,
    short_options_with_values: String,
}
fn present_patterns<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Vec<String>>, D::Error> {
    Vec::<String>::deserialize(deserializer).map(Some)
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Condition {
    kind: String,
    #[serde(
        default,
        deserialize_with = "present_patterns",
        skip_serializing_if = "Option::is_none"
    )]
    patterns: Option<Vec<String>>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rule {
    id: String,
    tool: String,
    command: Vec<String>,
    matching: String,
    condition: Condition,
    reason: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    schema_version: u32,
    framework: String,
    approved_tools: Vec<String>,
    project_command_prefixes: Vec<Prefix>,
    deny_rules: Vec<Rule>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    profile_path: String,
    profile_sha256: String,
    selection_sha256: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    schema_version: u32,
    framework: String,
    evidence: Evidence,
    approved_tools: Vec<String>,
    project_command_prefixes: Vec<Prefix>,
    deny_rules: Vec<Rule>,
}
impl Policy {
    fn selection(&self) -> Selection {
        Selection {
            schema_version: self.schema_version,
            framework: self.framework.clone(),
            approved_tools: self.approved_tools.clone(),
            project_command_prefixes: self.project_command_prefixes.clone(),
            deny_rules: self.deny_rules.clone(),
        }
    }
}
fn bad(message: impl Into<String>) -> RuntimeError {
    RuntimeError::invalid(message)
}
fn bounded_json(raw: &[u8]) -> Result<()> {
    if raw.len() > LIMIT {
        return Err(bad("Native hook policy exceeds 64 KiB"));
    }
    let mut quoted = false;
    let mut escape = false;
    let mut depth = 0usize;
    for byte in raw {
        if quoted {
            if escape {
                escape = false;
            } else if *byte == b'\\' {
                escape = true;
            } else if *byte == b'"' {
                quoted = false;
            }
            continue;
        }
        match byte {
            b'"' => quoted = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > 24 {
                    return Err(bad("Native hook policy JSON exceeds depth 24"));
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    let value: Value = serde_json::from_slice(raw)?;
    fn nodes(value: &Value) -> usize {
        1 + match value {
            Value::Array(values) => values.iter().map(nodes).sum::<usize>(),
            Value::Object(values) => values.values().map(nodes).sum::<usize>(),
            _ => 0,
        }
    }
    if nodes(&value) > 20000 {
        return Err(bad("Native hook policy exceeds node capacity"));
    }
    Ok(())
}
fn portable(value: &str) -> Result<()> {
    if !literal(value, 1024) || value.contains(['\\', ':']) {
        return Err(bad(
            "Hook policy profile reference must use a portable relative path",
        ));
    }
    ownership::normalize(value)?;
    Ok(())
}
fn literal(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && value.chars().all(|value| !value.is_control())
}
fn valid_glob(value: &str) -> bool {
    if !literal(value, 128) {
        return false;
    }
    let mut output = String::from("^");
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' => output.push_str(".*"),
            '?' => output.push('.'),
            '[' => {
                output.push('[');
                if chars.peek() == Some(&'!') {
                    chars.next();
                    output.push('^');
                }
                let mut count = 0;
                let mut closed = false;
                for c in chars.by_ref() {
                    if c == ']' {
                        closed = true;
                        break;
                    }
                    if matches!(c, '\\' | '[' | '^') {
                        output.push('\\');
                    }
                    output.push(c);
                    count += 1;
                }
                if !closed || count == 0 {
                    return false;
                }
                output.push(']');
            }
            ']' => return false,
            c => output.push_str(&regex::escape(&c.to_string())),
        }
    }
    output.push('$');
    regex::Regex::new(&output).is_ok()
}
fn validate_selection(selection: &Selection) -> Result<()> {
    if selection.schema_version != 1
        || selection.framework.len() > 64
        || !matching(r"^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$", &selection.framework)
    {
        return Err(bad("Invalid hook policy version or framework slug"));
    }
    if selection.approved_tools.len() > 64
        || selection.project_command_prefixes.len() > 128
        || selection.deny_rules.len() > 256
    {
        return Err(bad("Hook policy collection exceeds capacity"));
    }
    let mut tools = BTreeSet::new();
    for tool in &selection.approved_tools {
        if !matching(r"^[a-z][a-z0-9_-]{0,63}$", tool) || !tools.insert(tool.clone()) {
            return Err(bad(
                "Hook policy tool IDs must be unique bounded lower-case IDs",
            ));
        }
    }
    let mut covered = BTreeSet::new();
    for prefix in &selection.project_command_prefixes {
        if !tools.contains(&prefix.tool)
            || prefix.argv.is_empty()
            || prefix.argv.len() > 8
            || prefix.argv.iter().any(|value| !literal(value, 256))
            || prefix.options_with_values.len() > 32
            || prefix
                .options_with_values
                .iter()
                .any(|value| !matching(r"^-{1,2}[A-Za-z0-9][A-Za-z0-9_-]{0,62}$", value))
            || prefix.short_options_with_values.len() > 26
            || !prefix
                .short_options_with_values
                .bytes()
                .all(|value| value.is_ascii_alphabetic())
            || prefix
                .short_options_with_values
                .bytes()
                .collect::<BTreeSet<_>>()
                .len()
                != prefix.short_options_with_values.len()
        {
            return Err(bad("Invalid project command prefix or option arity"));
        }
        covered.insert(prefix.tool.clone());
    }
    if covered != tools {
        return Err(bad(
            "Every declared tool must have a reviewed literal command prefix",
        ));
    }
    let mut ids = BTreeSet::new();
    for rule in &selection.deny_rules {
        if !matching(r"^[a-z][a-z0-9_-]{0,63}$", &rule.id)
            || !ids.insert(rule.id.clone())
            || !tools.contains(&rule.tool)
            || rule.command.len() > 8
            || rule.command.iter().any(|value| !literal(value, 128))
            || !["exact", "console"].contains(&rule.matching.as_str())
            || ![
                "destructive_command",
                "secret_exposure",
                "verification_bypass",
            ]
            .contains(&rule.reason.as_str())
        {
            return Err(bad("Invalid native hook deny rule"));
        }
        match rule.condition.kind.as_str() {
            "always" => {
                if rule.condition.patterns.is_some() {
                    return Err(bad("Always condition must not contain patterns"));
                }
            }
            "any_option" | "no_option" | "first_argument" | "argument_contains" => {
                let patterns = rule
                    .condition
                    .patterns
                    .as_ref()
                    .filter(|patterns| !patterns.is_empty() && patterns.len() <= 16)
                    .ok_or_else(|| bad("Hook condition requires 1..16 bounded glob patterns"))?;
                if patterns.iter().any(|pattern| !valid_glob(pattern)) {
                    return Err(bad("Hook condition contains invalid glob data"));
                }
            }
            _ => return Err(bad("Unknown native hook condition")),
        }
    }
    Ok(())
}
fn selection_bytes(raw: &[u8]) -> Result<Selection> {
    bounded_json(raw)?;
    let selection: Selection = serde_json::from_slice(raw)?;
    validate_selection(&selection)?;
    Ok(selection)
}
fn from_policy(raw: &[u8]) -> Result<Policy> {
    bounded_json(raw)?;
    let policy: Policy = serde_json::from_slice(raw)?;
    validate_selection(&policy.selection())?;
    portable(&policy.evidence.profile_path)?;
    if !hex(&policy.evidence.profile_sha256, 64) || !hex(&policy.evidence.selection_sha256, 64) {
        return Err(bad("Invalid hook evidence digest"));
    }
    Ok(policy)
}
fn fence(line: &str) -> Option<(char, usize)> {
    let trimmed = line.trim_start();
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let c = trimmed.chars().next()?;
    if !['`', '~'].contains(&c) {
        return None;
    }
    let count = trimmed.chars().take_while(|value| *value == c).count();
    (count >= 3).then_some((c, count))
}
fn extract(raw: &[u8]) -> Result<Selection> {
    let text = std::str::from_utf8(raw)
        .map_err(|_| bad("Reviewed profile is not UTF-8"))?
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut outside_fence = None;
    let mut mode = 0u8;
    let mut blocks = 0usize;
    let mut payload = String::new();
    let mut selected = None;
    for line in text.split('\n') {
        match mode {
            0 => {
                if let Some((c, length)) = outside_fence {
                    if line.trim().chars().all(|value| value == c)
                        && line.trim().chars().count() >= length
                    {
                        outside_fence = None;
                    }
                    continue;
                }
                if line == START {
                    blocks += 1;
                    if blocks > 1 {
                        return Err(bad("Reviewed profile contains multiple hook selections"));
                    }
                    mode = 1;
                } else if line == END {
                    return Err(bad("Unexpected hook selection boundary"));
                } else if let Some(mark) = fence(line) {
                    outside_fence = Some(mark);
                }
            }
            1 => {
                if line.trim().is_empty() {
                    continue;
                }
                if line != "```json" {
                    return Err(bad("Hook selection must be one explicit JSON fence"));
                }
                mode = 2;
            }
            2 => {
                if line == "```" {
                    selected = Some(selection_bytes(payload.as_bytes())?);
                    mode = 3;
                } else {
                    payload.push_str(line);
                    payload.push('\n');
                    if payload.len() > LIMIT {
                        return Err(bad("Hook selection exceeds capacity"));
                    }
                }
            }
            3 => {
                if line.trim().is_empty() {
                    continue;
                }
                if line != END {
                    return Err(bad(
                        "Hook selection fence must close with its exact boundary",
                    ));
                }
                mode = 0;
            }
            _ => unreachable!(),
        }
    }
    if mode != 0 || blocks != 1 {
        return Err(bad(
            "Approved profile needs a complete pre-review hook-policy selection block; refresh and review legacy profiles",
        ));
    }
    selected.ok_or_else(|| bad("Reviewed hook policy selection missing"))
}
fn read_profile(path: &Path, approved: Option<&str>) -> Result<(Vec<u8>, String)> {
    let path = ownership::resolve_target(path)?;
    let parent = path.parent().ok_or_else(|| bad("Profile has no parent"))?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| bad("Profile filename is not UTF-8"))?;
    let mut files = PublicationFiles::new(parent)?;
    let before = files.image_identity(name)?;
    let raw = files
        .read(name, 8 * 1024 * 1024)?
        .ok_or_else(|| RuntimeError::not_found("Reviewed profile missing"))?;
    if files.image_identity(name)? != before {
        return Err(RuntimeError::conflict(
            "Reviewed profile changed during read",
        ));
    }
    let digest = sha(&raw);
    if approved.is_some_and(|value| !hex(value, 64) || value != digest) {
        return Err(RuntimeError::conflict(
            "Profile differs from its recorded human-review digest",
        ));
    }
    Ok((raw, digest))
}
pub fn inspect(profile: &Path, profile_path: &str) -> Result<Value> {
    portable(profile_path)?;
    let (raw, digest) = read_profile(profile, None)?;
    let selection = extract(&raw)?;
    let selection = serde_json::to_value(selection)?;
    Ok(
        object!({"profile_path":profile_path,"profile_sha256":digest,"selection_sha256":sha(infra_core::domain::json::canonical(&selection)?.as_bytes()),"selection":selection,"review_required":true}),
    )
}
fn expected(profile: &Path, approved: &str, profile_path: &str) -> Result<Policy> {
    portable(profile_path)?;
    if !hex(approved, 64) {
        return Err(bad("A recorded approved profile SHA256 is required"));
    }
    let (raw, digest) = read_profile(profile, Some(approved))?;
    let selection = extract(&raw)?;
    let value = serde_json::to_value(&selection)?;
    let selection_digest = sha(infra_core::domain::json::canonical(&value)?.as_bytes());
    Ok(Policy {
        schema_version: selection.schema_version,
        framework: selection.framework,
        evidence: Evidence {
            profile_path: profile_path.into(),
            profile_sha256: digest,
            selection_sha256: selection_digest,
        },
        approved_tools: selection.approved_tools,
        project_command_prefixes: selection.project_command_prefixes,
        deny_rules: selection.deny_rules,
    })
}
pub fn materialize(
    profile: &Path,
    approved: &str,
    profile_path: &str,
    staging: &Path,
) -> Result<Value> {
    let expected = expected(profile, approved, profile_path)?;
    let value = serde_json::to_value(expected)?;
    let bytes = (infra_core::domain::json::sorted_pretty(&value)? + "\n").into_bytes();
    if bytes.len() > LIMIT {
        return Err(bad("Materialized hook policy exceeds capacity"));
    }
    let mut files = PublicationFiles::new(&ownership::resolve_target(staging)?)?;
    let previous = files.read(PATH, LIMIT)?;
    let mode = files.read_mode(PATH)?.unwrap_or(0o644);
    files.write_expected_mode_checked(
        PATH,
        &bytes,
        previous.as_deref(),
        previous.as_ref().map(|_| mode),
        mode,
        LIMIT,
    )?;
    let schema = (infra_core::domain::json::sorted_pretty(&schema())? + "\n").into_bytes();
    let before = files.read(SCHEMA_PATH, LIMIT)?;
    let schema_mode = files.read_mode(SCHEMA_PATH)?.unwrap_or(0o644);
    files.write_expected_mode_checked(
        SCHEMA_PATH,
        &schema,
        before.as_deref(),
        before.as_ref().map(|_| schema_mode),
        schema_mode,
        LIMIT,
    )?;
    Ok(
        object!({"path":PATH,"sha256":sha(&bytes),"schema_path":SCHEMA_PATH,"schema_sha256":sha(&schema),"profile_sha256":approved,"selection_sha256":value["evidence"]["selection_sha256"],"framework":value["framework"],"data_only":true}),
    )
}
pub fn validate(
    target: &Path,
    profile: &Path,
    approved: &str,
    profile_path: &str,
) -> Result<Value> {
    let expected = expected(profile, approved, profile_path)?;
    let target = ownership::resolve_target(target)?;
    let mut files = PublicationFiles::new(&target)?;
    let raw = files
        .read(PATH, LIMIT)?
        .ok_or_else(|| RuntimeError::policy("Generated native hook policy is required"))?;
    let policy = from_policy(&raw)?;
    if serde_json::to_value(&policy)? != serde_json::to_value(&expected)? {
        return Err(RuntimeError::policy(
            "Generated hook policy differs from the reviewed profile selection or evidence binding",
        ));
    }
    let selection = serde_json::to_value(policy.selection())?;
    if sha(infra_core::domain::json::canonical(&selection)?.as_bytes())
        != policy.evidence.selection_sha256
    {
        return Err(RuntimeError::policy("Hook policy selection digest differs"));
    }
    let config = files
        .read("project-brain/config/runtime.json", 2 * 1024 * 1024)?
        .ok_or_else(|| RuntimeError::policy("Generated Brain runtime configuration is required"))?;
    let config: Value = serde_json::from_slice(&config)?;
    if config["framework"] != policy.framework {
        return Err(RuntimeError::policy(
            "Hook policy framework differs from native runtime configuration",
        ));
    }
    if let Some(raw_manifest) = files.read(".infra-manifest.json", 16 * 1024 * 1024)? {
        let manifest: Value = serde_json::from_slice(&raw_manifest)?;
        if manifest["profile"] != profile_path || manifest["files"][PATH] != sha(&raw) {
            return Err(RuntimeError::policy(
                "Hook policy profile reference or owned raw digest differs from manifest",
            ));
        }
    }
    let schema_bytes = files
        .read(SCHEMA_PATH, LIMIT)?
        .ok_or_else(|| RuntimeError::policy("Native hook policy schema resource is required"))?;
    let schema_value: Value = serde_json::from_slice(&schema_bytes)?;
    if schema_value != schema() {
        return Err(RuntimeError::policy(
            "Native hook policy schema resource differs from compiled contract",
        ));
    }
    files.check()?;
    Ok(
        object!({"valid":true,"path":PATH,"sha256":sha(&raw),"profile_sha256":approved,"selection_sha256":policy.evidence.selection_sha256,"framework":policy.framework,"data_only":true}),
    )
}
pub fn schema() -> Value {
    let id = object!({"type":"string","pattern":"^[a-z][a-z0-9_-]{0,63}$","maxLength":64});
    let literal = |limit| object!({"type":"string","minLength":1,"maxLength":limit,"x-maxBytes":limit,"pattern":r"^[^\u0000-\u001f\u007f-\u009f]+$"});
    let patterns = object!({"type":"array","minItems":1,"maxItems":16,"items":{"type":"string","minLength":1,"maxLength":128,"x-maxBytes":128,"x-syntax":"bounded glob (* ? balanced [class]/[!class]); no regex or evaluation"}});
    let condition = object!({"oneOf":[{"type":"object","additionalProperties":false,"required":["kind"],"properties":{"kind":{"const":"always"}}},{"type":"object","additionalProperties":false,"required":["kind","patterns"],"properties":{"kind":{"enum":["any_option","no_option","first_argument","argument_contains"]},"patterns":patterns}}]});
    let prefix = object!({"type":"object","additionalProperties":false,"required":["tool","argv","options_with_values","short_options_with_values"],"properties":{"tool":id,"argv":{"type":"array","minItems":1,"maxItems":8,"items":literal(256)},"options_with_values":{"type":"array","maxItems":32,"items":{"type":"string","pattern":"^-{1,2}[A-Za-z0-9][A-Za-z0-9_-]{0,62}$"}},"short_options_with_values":{"type":"string","maxLength":26,"pattern":"^[A-Za-z]*$","x-uniqueCharacters":true}}});
    let rule = object!({"type":"object","additionalProperties":false,"required":["id","tool","command","matching","condition","reason"],"properties":{"id":id,"tool":id,"command":{"type":"array","maxItems":8,"items":literal(128)},"matching":{"enum":["exact","console"]},"condition":condition,"reason":{"enum":["destructive_command","secret_exposure","verification_bypass"]}}});
    object!({"$schema":"https://json-schema.org/draft/2020-12/schema","title":"Native generated hook policy v1","type":"object","additionalProperties":false,"required":["schema_version","framework","evidence","approved_tools","project_command_prefixes","deny_rules"],"properties":{"schema_version":{"const":1},"framework":{"type":"string","pattern":"^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$","maxLength":64},"evidence":{"type":"object","additionalProperties":false,"required":["profile_path","profile_sha256","selection_sha256"],"properties":{"profile_path":{"type":"string","minLength":1,"maxLength":1024,"x-maxBytes":1024,"x-format":"normal portable relative POSIX path"},"profile_sha256":{"type":"string","pattern":"^[0-9a-f]{64}$"},"selection_sha256":{"type":"string","pattern":"^[0-9a-f]{64}$"}}},"approved_tools":{"type":"array","maxItems":64,"uniqueItems":true,"items":id},"project_command_prefixes":{"type":"array","maxItems":128,"items":prefix},"deny_rules":{"type":"array","maxItems":256,"items":rule}},"x-maxBytes":LIMIT,"x-maxDepth":24,"x-maxNodes":20000,"x-contract":"Every declared tool has a literal prefix; rule IDs unique and reference declared tools; selection canonical SHA excludes evidence. Actual independent reviewed-profile comparison is required; self-hash is not approval. argument_contains matches abbreviated credential selector against declared literal names, not arbitrary substring execution."})
}
