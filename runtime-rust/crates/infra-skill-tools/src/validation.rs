use crate::{Result, common};
use serde_yaml::{Mapping, Value};
use std::path::Path;
const ALLOWED: [&str; 9] = [
    "allowed-tools",
    "compatibility",
    "description",
    "flow-alternatives",
    "flow-next",
    "license",
    "metadata",
    "name",
    "phase",
];
pub fn is_valid_skill_name(name: &str) -> bool {
    name.chars().count() <= 64
        && !name.is_empty()
        && name.split('-').all(|s| {
            !s.is_empty()
                && s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}
fn value<'a>(map: &'a Mapping, key: &str) -> Option<&'a Value> {
    map.get(Value::String(key.into()))
}
fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(n) if n.as_i64().is_some() || n.as_u64().is_some() => "int",
        Value::Number(_) => "float",
        Value::String(_) => "str",
        Value::Sequence(_) => "list",
        Value::Mapping(_) => "dict",
        Value::Tagged(t) => kind(&t.value),
    }
}
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Sequence(s) => !s.is_empty(),
        Value::Mapping(s) => !s.is_empty(),
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::Tagged(t) => truthy(&t.value),
    }
}
pub fn parse_frontmatter(text: &str) -> std::result::Result<Value, serde_yaml::Error> {
    serde_yaml::from_str(text)
}
pub fn validate_skill(path: &Path) -> Result<(bool, String)> {
    let md = path.join("SKILL.md");
    if !md.exists() {
        return Ok((false, "SKILL.md not found".into()));
    }
    let content = common::read(&md)?;
    let fail = |s: String| Ok((false, s));
    if !content.starts_with("---") {
        return fail("No YAML frontmatter found".into());
    }
    let Some(body) = content.strip_prefix("---\n") else {
        return fail("Invalid frontmatter format".into());
    };
    let Some(end) = body.find("\n---") else {
        return fail("Invalid frontmatter format".into());
    };
    let parsed = match parse_frontmatter(&body[..end]) {
        Ok(p) => p,
        Err(e) => return fail(format!("Invalid YAML in frontmatter: {e}")),
    };
    let Some(map) = parsed.as_mapping() else {
        return fail("Frontmatter must be a YAML dictionary".into());
    };
    let mut unexpected = map
        .keys()
        .filter_map(|k| match k.as_str() {
            Some(k) if ALLOWED.contains(&k) => None,
            Some(k) => Some(k.to_owned()),
            None => Some(format!("{k:?}")),
        })
        .collect::<Vec<_>>();
    unexpected.sort();
    if !unexpected.is_empty() {
        return fail(format!(
            "Unexpected key(s) in SKILL.md frontmatter: {}. Allowed properties are: {}",
            unexpected.join(", "),
            ALLOWED.join(", ")
        ));
    }
    for field in ["name", "description"] {
        if value(map, field).is_none() {
            return fail(format!("Missing '{field}' in frontmatter"));
        }
    }
    if let Some(p) = value(map, "phase")
        && !p
            .as_str()
            .is_some_and(|p| ["planning", "execution", "quality", "utility"].contains(&p))
    {
        return fail("'phase' must be planning, execution, quality, or utility".into());
    }
    if let Some(p) = value(map, "flow-next")
        && !p.is_null()
        && !p.as_str().is_some_and(is_valid_skill_name)
    {
        return fail("'flow-next' must be null or a kebab-case skill name".into());
    }
    if let Some(p) = value(map, "flow-alternatives")
        && !p.as_sequence().is_some_and(|list| {
            list.iter()
                .all(|p| p.as_str().is_some_and(is_valid_skill_name))
        })
    {
        return fail("'flow-alternatives' must be a list of kebab-case skill names".into());
    }
    let n = value(map, "name").unwrap();
    let Some(name) = n.as_str() else {
        return fail(format!("Name must be a string, got {}", kind(n)));
    };
    let name = name.trim();
    if !name.is_empty() {
        if name.chars().count() > 64 {
            return fail(format!(
                "Name is too long ({} characters). Maximum is 64 characters.",
                name.chars().count()
            ));
        }
        if !is_valid_skill_name(name) {
            return fail(format!("Name '{name}' should be kebab-case"));
        }
    }
    let d = value(map, "description").unwrap();
    let Some(desc) = d.as_str() else {
        return fail(format!("Description must be a string, got {}", kind(d)));
    };
    let desc = desc.trim();
    if !desc.is_empty() {
        if desc.contains(['<', '>']) {
            return fail("Description cannot contain angle brackets (< or >)".into());
        }
        if desc.chars().count() > 1024 {
            return fail(format!(
                "Description is too long ({} characters). Maximum is 1024 characters.",
                desc.chars().count()
            ));
        }
    }
    if let Some(c) = value(map, "compatibility").filter(|v| truthy(v)) {
        let Some(c) = c.as_str() else {
            return fail(format!("Compatibility must be a string, got {}", kind(c)));
        };
        if c.chars().count() > 500 {
            return fail(format!(
                "Compatibility is too long ({} characters). Maximum is 500 characters.",
                c.chars().count()
            ));
        }
    }
    Ok((true, "Skill is valid!".into()))
}
