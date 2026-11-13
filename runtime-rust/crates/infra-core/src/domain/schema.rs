use crate::{Result, Roots, RuntimeError, fs};
use serde_json::Value;
use std::collections::BTreeSet;

pub fn is_uuid4(text: &str) -> bool {
    if text.len() != 36
        || ![8, 13, 18, 23]
            .iter()
            .all(|index| text.as_bytes()[*index] == b'-')
    {
        return false;
    }
    uuid::Uuid::parse_str(text)
        .is_ok_and(|id| id.get_version_num() == 4 && id.get_variant() == uuid::Variant::RFC4122)
}

pub fn valid_datetime(text: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(text).is_ok()
        || chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok()
        || ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"]
            .iter()
            .any(|format| chrono::NaiveDateTime::parse_from_str(text, format).is_ok())
        || python_date(text).is_some()
        || text.char_indices().any(|(end, separator)| {
            python_date(&text[..end]).is_some() && iso_time(&text[end + separator.len_utf8()..])
        })
}

pub fn python_date(text: &str) -> Option<chrono::NaiveDate> {
    for format in ["%Y-%m-%d", "%Y%m%d", "%G-W%V-%u", "%GW%V%u"] {
        if let Ok(date) = chrono::NaiveDate::parse_from_str(text, format) {
            return Some(date);
        }
    }
    if text.len() == 8
        && text.as_bytes().get(4) == Some(&b'-')
        && text.as_bytes().get(5) == Some(&b'W')
    {
        return chrono::NaiveDate::parse_from_str(&format!("{text}-1"), "%G-W%V-%u").ok();
    }
    if text.len() == 7 && text.as_bytes().get(4) == Some(&b'W') {
        return chrono::NaiveDate::parse_from_str(&format!("{text}1"), "%GW%V%u").ok();
    }
    None
}
fn iso_time(text: &str) -> bool {
    let (time, zone) = if let Some(time) = text.strip_suffix('Z') {
        (time, None)
    } else if let Some(index) = text.char_indices().find_map(|(index, c)| {
        if index > 0 && (c == '+' || c == '-') {
            Some(index)
        } else {
            None
        }
    }) {
        (&text[..index], Some(&text[index + 1..]))
    } else {
        (text, None)
    };
    fn components(text: &str, zone: bool) -> bool {
        let (head, fraction) = text
            .split_once(['.', ','])
            .map(|(head, fraction)| (head, Some(fraction)))
            .unwrap_or((text, None));
        if fraction.is_some_and(|fraction| {
            fraction.is_empty() || !fraction.bytes().all(|b| b.is_ascii_digit())
        }) {
            return false;
        }
        let parts = if head.contains(':') {
            head.split(':').collect::<Vec<_>>()
        } else {
            if ![2, 4, 6].contains(&head.len()) {
                return false;
            }
            head.as_bytes()
                .chunks(2)
                .filter_map(|part| std::str::from_utf8(part).ok())
                .collect::<Vec<_>>()
        };
        if parts.is_empty()
            || parts.len() > 3
            || parts
                .iter()
                .any(|part| part.len() != 2 || !part.bytes().all(|b| b.is_ascii_digit()))
        {
            return false;
        }
        let values = parts
            .iter()
            .filter_map(|part| part.parse::<u32>().ok())
            .collect::<Vec<_>>();
        values[0] < 24
            && values.get(1).is_none_or(|value| *value < 60)
            && values.get(2).is_none_or(|value| *value < 60)
            && (!zone
                || values.iter().any(|value| *value != 0)
                || fraction.is_none_or(|digits| digits.bytes().all(|b| b == b'0')))
    }
    components(time, false) && zone.is_none_or(|zone| components(zone, true))
}

pub fn validate_schema_value(value: &Value, schema: &Value) -> Result<()> {
    validate_at(value, schema, schema, "$", 0)
}

/// A schema value as Python's `repr` shows it in "value must equal".
fn shown(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::String(text) => format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'")),
        other => other.to_string(),
    }
}

/// The retired validator's checks in its order, each refusal naming the
/// field and the rule it broke; `maxItems` and `maxLength`, which this
/// runtime also enforces, come after the rules it shared.
fn validate_at(
    value: &Value,
    schema: &Value,
    root: &Value,
    location: &str,
    depth: usize,
) -> Result<()> {
    if depth > 128 {
        return Err(RuntimeError::invalid("Schema nesting limit exceeded"));
    }
    if let Some(reference) = schema["$ref"].as_str() {
        let target = reference
            .strip_prefix('#')
            .and_then(|pointer| root.pointer(pointer))
            .filter(|target| target.is_object())
            .ok_or_else(|| {
                RuntimeError::invalid(format!("invalid schema reference: {reference}"))
            })?;
        return validate_at(value, target, root, location, depth + 1);
    }
    let fail = |rule: &str| RuntimeError::invalid(format!("{location}: {rule}"));
    if let Some(constant) = schema.get("const")
        && constant != value
    {
        return Err(fail(&format!("value must equal {}", shown(constant))));
    }
    if let Some(items) = schema["enum"].as_array()
        && !items.contains(value)
    {
        return Err(fail("value is outside schema enum"));
    }
    let accepts = |kind: &str| match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    };
    if let Some(declared) = schema.get("type") {
        let accepted = match declared {
            Value::String(kind) => accepts(kind),
            Value::Array(kinds) => kinds.iter().filter_map(Value::as_str).any(accepts),
            _ => false,
        };
        if !accepted {
            return Err(fail("value has invalid schema type"));
        }
    }
    if let Some(object) = value.as_object() {
        let missing = schema["required"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|key| !object.contains_key(*key))
            .collect::<BTreeSet<_>>();
        if !missing.is_empty() {
            return Err(fail(&format!(
                "missing keys {}",
                missing.into_iter().collect::<Vec<_>>().join(", ")
            )));
        }
        let properties = schema["properties"].as_object();
        if schema["additionalProperties"] == false {
            let extra = object
                .keys()
                .map(String::as_str)
                .filter(|key| !properties.is_some_and(|properties| properties.contains_key(*key)))
                .collect::<BTreeSet<_>>();
            if !extra.is_empty() {
                return Err(fail(&format!(
                    "unexpected keys {}",
                    extra.into_iter().collect::<Vec<_>>().join(", ")
                )));
            }
        }
        if let Some(properties) = properties {
            for (key, child) in properties {
                if let Some(item) = object.get(key) {
                    validate_at(item, child, root, &format!("{location}.{key}"), depth + 1)?;
                }
            }
        }
    }
    if let Some(array) = value.as_array() {
        if array.len() < schema["minItems"].as_u64().unwrap_or(0) as usize {
            return Err(fail("too few items"));
        }
        if schema["uniqueItems"] == true {
            let serialized: BTreeSet<_> = array.iter().map(Value::to_string).collect();
            if serialized.len() != array.len() {
                return Err(fail("items must be unique"));
            }
        }
        if schema["items"].is_object() {
            for (index, item) in array.iter().enumerate() {
                validate_at(
                    item,
                    &schema["items"],
                    root,
                    &format!("{location}[{index}]"),
                    depth + 1,
                )?;
            }
        }
        if schema["maxItems"]
            .as_u64()
            .is_some_and(|limit| array.len() as u64 > limit)
        {
            return Err(fail("too many items"));
        }
    }
    if let Some(text) = value.as_str() {
        if text.chars().count() < schema["minLength"].as_u64().unwrap_or(0) as usize {
            return Err(fail("string is too short"));
        }
        if let Some(pattern) = schema["pattern"].as_str() {
            let pattern = regex::Regex::new(&format!("^(?:{pattern})$"))
                .map_err(|_| RuntimeError::invalid("Unsupported schema pattern"))?;
            if !pattern.is_match(text) {
                return Err(fail("string does not match schema pattern"));
            }
        }
        if schema["format"] == "uuid" && !is_uuid4(text) {
            return Err(fail("value must be UUIDv4"));
        }
        if schema["format"] == "date-time" && !valid_datetime(text) {
            return Err(fail("invalid date-time"));
        }
        if schema["maxLength"]
            .as_u64()
            .is_some_and(|limit| text.chars().count() as u64 > limit)
        {
            return Err(fail("string is too long"));
        }
    }
    if let Some(number) = value.as_f64() {
        if schema["minimum"]
            .as_f64()
            .is_some_and(|minimum| number < minimum)
        {
            return Err(fail("value is below minimum"));
        }
        if schema["maximum"]
            .as_f64()
            .is_some_and(|maximum| number > maximum)
        {
            return Err(fail("value is above maximum"));
        }
    }
    Ok(())
}

pub fn validate_schema_file(roots: &Roots, name: &str, value: &Value) -> Result<()> {
    if name.contains('/') || name.contains('\\') || name == ".." {
        return Err(RuntimeError::invalid("Invalid schema filename"));
    }
    for base in [&roots.state, &roots.tooling] {
        let path = base.join("project-brain/schemas").join(name);
        match std::fs::symlink_metadata(&path) {
            Ok(_) => {
                let schema = serde_json::from_str(&fs::read_text(&path)?)?;
                return validate_schema_value(value, &schema);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod maximum_contract_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn maximum_lengths_use_unicode_characters() {
        assert!(
            validate_schema_value(&json!("éβ"), &json!({"type":"string","maxLength":2})).is_ok()
        );
        assert!(
            validate_schema_value(&json!("éβ"), &json!({"type":"string","maxLength":1})).is_err()
        );
        assert!(validate_schema_value(&json!("éβ"), &json!({"type":"string"})).is_ok());
    }
    #[test]
    fn maximum_item_count_is_inclusive() {
        assert!(
            validate_schema_value(&json!([1, 2]), &json!({"type":"array","maxItems":2})).is_ok()
        );
        assert!(
            validate_schema_value(&json!([1, 2]), &json!({"type":"array","maxItems":1})).is_err()
        );
        assert!(validate_schema_value(&json!([1, 2]), &json!({"type":"array"})).is_ok());
    }
}
