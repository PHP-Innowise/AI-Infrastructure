use crate::common;
use infra_core::{Result, RuntimeError, fs as safe};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};
pub fn split<'a>(text: &'a str, path: &str) -> Result<(&'a str, &'a str)> {
    let first = text.find('\n').unwrap_or(text.len());
    if text[..first].trim() != "---" || first == text.len() {
        return Err(RuntimeError::invalid(format!(
            "{path}: missing frontmatter"
        )));
    }
    let start = first + 1;
    let mut position = start;
    while position <= text.len() {
        let end = text[position..]
            .find('\n')
            .map(|offset| position + offset)
            .unwrap_or(text.len());
        if text[position..end].trim() == "---" {
            let front_end = if position > start && text.as_bytes()[position - 1] == b'\n' {
                position - 1
            } else {
                position
            };
            let body_start = if end < text.len() { end + 1 } else { end };
            return Ok((&text[start..front_end], &text[body_start..]));
        }
        if end == text.len() {
            break;
        }
        position = end + 1;
    }
    Err(RuntimeError::invalid(format!(
        "{path}: missing closing frontmatter"
    )))
}
pub fn field(front: &str, key: &str) -> Option<String> {
    let mut lines = front.lines();
    let prefix = format!("{key}:");
    while let Some(line) = lines.next() {
        let Some(first) = line.strip_prefix(&prefix) else {
            continue;
        };
        let mut parts = vec![first.trim().to_string()];
        for line in lines.by_ref() {
            if !line.is_empty() && !line.starts_with(char::is_whitespace) {
                break;
            }
            parts.push(line.trim().into());
        }
        let mut value = parts.join(" ").trim().to_string();
        if value.len() >= 2 {
            let quote = value.as_bytes()[0];
            if (quote == b'\'' || quote == b'"') && value.as_bytes().last() == Some(&quote) {
                value = value[1..value.len() - 1].to_string();
                if quote == b'"' {
                    for (from, to) in [("\\n", "\n"), ("\\t", "\t"), ("\\\"", "\""), ("\\\\", "\\")]
                    {
                        value = value.replace(from, to);
                    }
                }
            }
        }
        return Some(value);
    }
    None
}
fn paragraph(body: &str) -> String {
    let mut lines = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            if !lines.is_empty() {
                break;
            }
            continue;
        }
        lines.push(line);
    }
    lines.join(" ")
}
fn listing(base: &Path, pattern: &str) -> Result<usize> {
    if !base.is_dir() {
        return Ok(0);
    }
    let mut count = 0;
    for relative in common::walk(base)? {
        if relative.contains('/') || !common::matches(pattern, &relative, true)? {
            continue;
        }
        let text = safe::read_text(&common::under(base, &relative)?)?
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        let (front, body) = split(&text, &relative).unwrap_or(("", &text));
        let description = field(front, "description").unwrap_or_else(|| paragraph(body));
        let stem = Path::new(&relative)
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| RuntimeError::invalid("Listing path is not UTF-8"))?;
        count += stem.len() + 1 + description.len();
    }
    Ok(count)
}
pub fn measure(root: &Path, edition: &str) -> Result<Value> {
    let edition = common::edition(edition)?;
    let base = root.join(edition);
    let policy = safe::read_regular(&base.join("AGENTS.md"))?;
    let skills = base.join(".agents/skills");
    let mut values = BTreeMap::from([
        ("agents_md_bytes", policy.len()),
        ("descriptor_bytes", 0),
        ("frontmatter_bytes", 0),
        ("body_bytes", 0),
        ("skills", 0),
    ]);
    for relative in common::walk(&skills)? {
        if relative.split('/').count() != 2 || !relative.ends_with("/SKILL.md") {
            continue;
        }
        let text = safe::read_text(&common::under(&skills, &relative)?)?
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        let (front, body) = split(&text, &relative)?;
        *values.get_mut("frontmatter_bytes").unwrap() += front.len();
        *values.get_mut("body_bytes").unwrap() += body.len();
        *values.get_mut("skills").unwrap() += 1;
        let name = field(front, "name")
            .ok_or_else(|| RuntimeError::invalid(format!("{relative}: no skill name")))?;
        let description = field(front, "description")
            .ok_or_else(|| RuntimeError::invalid(format!("{relative}: no skill description")))?;
        if !front.contains("disable-model-invocation: true") {
            *values.get_mut("descriptor_bytes").unwrap() += name.len() + 1 + description.len();
        }
    }
    if values["skills"] == 0 {
        return Err(RuntimeError::invalid(
            "Edition contains no canonical skills",
        ));
    }
    values.insert(
        "command_bytes",
        listing(&base.join(".claude/commands"), "*.md")?,
    );
    values.insert(
        "agent_bytes",
        listing(&base.join(".claude/agents"), "*-agent.md")?,
    );
    Ok(serde_json::to_value(values)?)
}
pub fn check(root: &Path, enforce: bool) -> Result<Value> {
    report(root, enforce, false)
}
pub fn report(root: &Path, enforce: bool, require_headroom: bool) -> Result<Value> {
    let config = match common::read_json(&root.join("scripts/token_budget.json")) {
        Ok(value) if value["editions"].is_object() => value,
        Ok(_) if !enforce && !require_headroom => json!({"editions":{}}),
        Ok(_) => return Err(RuntimeError::invalid("Context-budget editions are absent")),
        Err(_) if !enforce && !require_headroom => json!({"editions":{}}),
        Err(error) => return Err(error),
    };
    let ceilings = config["editions"]
        .as_object()
        .ok_or_else(|| RuntimeError::invalid("Context-budget editions are absent"))?;
    let mut measured = serde_json::Map::new();
    let mut findings = Vec::new();
    for edition in common::EDITIONS {
        let label = if *edition == "Cms/wordpress" {
            "WordPress"
        } else {
            edition
        };
        if !root.join(edition).is_dir() {
            return Err(RuntimeError::not_found(format!(
                "Budget edition is missing: {edition}"
            )));
        }
        let actual = measure(root, edition)?;
        let budget = ceilings.get(label).and_then(Value::as_object);
        if enforce {
            let budget = budget.ok_or_else(|| {
                RuntimeError::invalid(format!("Budget ceilings are missing: {edition}"))
            })?;
            for key in [
                "agents_md_bytes",
                "descriptor_bytes",
                "command_bytes",
                "agent_bytes",
                "frontmatter_bytes",
                "body_bytes",
                "skills",
            ] {
                let maximum = budget.get(key).and_then(Value::as_u64).ok_or_else(|| {
                    RuntimeError::invalid(format!("Invalid budget ceiling {label}.{key}"))
                })?;
                if actual[key].as_u64().unwrap_or(0) > maximum {
                    findings.push(format!("{label}.{key}: {} exceeds {maximum}", actual[key]));
                }
            }
        }
        measured.insert(label.to_string(), actual);
    }
    if enforce {
        for label in ceilings.keys() {
            if common::edition(label).is_err() {
                return Err(RuntimeError::invalid(format!(
                    "Unknown budget edition: {label}"
                )));
            }
        }
    }
    let ratios = BTreeMap::from([
        ("agents_md_bytes", 4.76_f64),
        ("descriptor_bytes", 4.91),
        ("command_bytes", 5.10),
        ("agent_bytes", 5.14),
        ("frontmatter_bytes", 4.59),
        ("body_bytes", 4.23),
    ]);
    let mut headroom = Vec::<(i128, String, String, u64, u64)>::new();
    let mut tokens = serde_json::Map::new();
    let mut totals = BTreeMap::<String, u64>::new();
    let mut startup = serde_json::Map::new();
    for (label, actual) in &measured {
        let mut estimates = serde_json::Map::new();
        let mut startup_tokens = 0_u64;
        for (key, ratio) in &ratios {
            let count = actual[*key].as_u64().unwrap_or(0);
            let estimate = (count as f64 / ratio).round_ties_even() as u64;
            estimates.insert((*key).into(), estimate.into());
            if [
                "agents_md_bytes",
                "descriptor_bytes",
                "command_bytes",
                "agent_bytes",
            ]
            .contains(key)
            {
                startup_tokens += estimate;
            }
        }
        for (key, count) in actual.as_object().into_iter().flatten() {
            if let Some(count) = count.as_u64() {
                *totals.entry(key.clone()).or_default() += count;
            }
        }
        tokens.insert(label.clone(), Value::Object(estimates));
        startup.insert(label.clone(), startup_tokens.into());
        if let Some(budget) = ceilings.get(label).and_then(Value::as_object) {
            for (key, ceiling) in budget {
                if let (Some(ceiling), Some(value)) = (ceiling.as_u64(), actual[key].as_u64()) {
                    headroom.push((
                        i128::from(ceiling) - i128::from(value),
                        label.clone(),
                        key.clone(),
                        value,
                        ceiling,
                    ));
                }
            }
        }
    }
    headroom.sort();
    let rows=headroom.into_iter().map(|(remaining,edition,category,measured,ceiling)|json!({"remaining":remaining,"edition":edition,"category":category,"measured":measured,"ceiling":ceiling})).collect::<Vec<_>>();
    let total_startup = ratios
        .iter()
        .filter(|(key, _)| {
            [
                "agents_md_bytes",
                "descriptor_bytes",
                "command_bytes",
                "agent_bytes",
            ]
            .contains(key)
        })
        .map(|(key, ratio)| {
            (totals.get(*key).copied().unwrap_or(0) as f64 / ratio).round_ties_even() as u64
        })
        .sum::<u64>();
    Ok(
        json!({"passed":findings.is_empty(),"editions":measured,"findings":findings,"enforced":enforce,"headroom":rows,"tokens":tokens,"startup_tokens":startup,"totals":totals,"total_startup_tokens":total_startup,"bytes_per_token":ratios,"ceilings":ceilings}),
    )
}
