use crate::{Result, Roots, RuntimeError, fs};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub fn source_fingerprints(roots: &Roots, sources: &[String]) -> Result<Vec<Value>> {
    let mut output = Vec::new();
    for source in sources.iter().collect::<BTreeSet<_>>() {
        let relative = source.split('#').next().unwrap_or(source);
        let path = std::fs::canonicalize(roots.resolve(relative))?;
        if !roots.contains(&path)? {
            return Err(RuntimeError::policy("Source escapes repository roots"));
        }
        let bytes = fs::read_regular(&path)?;
        output
            .push(json!({"path":relative,"sha256":memory_kernel_probe::fingerprint_bytes(&bytes)}));
    }
    Ok(output)
}

pub fn sources_are_fresh(roots: &Roots, record: &Value) -> Result<bool> {
    let Some(rows) = record["source_fingerprints"].as_array() else {
        return Ok(false);
    };
    let expected = record["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|source| source.split('#').next().unwrap_or(source))
        .collect::<BTreeSet<_>>();
    let actual = rows
        .iter()
        .filter_map(|row| row["path"].as_str())
        .collect::<BTreeSet<_>>();
    if expected != actual
        || rows
            .iter()
            .any(|row| !row["path"].is_string() || !row["sha256"].is_string())
    {
        return Ok(false);
    }
    let (changed, missing) = source_changes(roots, &record["source_fingerprints"])?;
    Ok(changed.is_empty() && missing.is_empty() && citation_problem(roots, record)?.is_none())
}

pub fn source_changes(roots: &Roots, fingerprints: &Value) -> Result<(Vec<String>, Vec<String>)> {
    let mut changed = Vec::new();
    let mut missing = Vec::new();
    let Some(fingerprints) = fingerprints.as_array() else {
        return Ok((changed, missing));
    };
    for previous in fingerprints {
        let key = previous["path"].as_str();
        let expected = previous["sha256"].as_str();
        let (Some(key), Some(expected)) = (key, expected) else {
            missing.push(previous["path"].to_string());
            continue;
        };
        match source_fingerprints(roots, &[key.into()]) {
            Ok(current)
                if current
                    .first()
                    .is_some_and(|current| current["sha256"].as_str() == Some(expected)) => {}
            Ok(_) => changed.push(key.into()),
            Err(_) => missing.push(key.into()),
        }
    }
    Ok((changed, missing))
}

pub fn digestible_sources(sources: &Value) -> Vec<String> {
    sources
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|source| {
            !source.is_empty() && !source.starts_with("http://") && !source.starts_with("https://")
        })
        .map(String::from)
        .collect()
}

pub fn citation_problem(roots: &Roots, record: &Value) -> Result<Option<&'static str>> {
    let Some(rows) = record["source_fingerprints"].as_array() else {
        return Ok(Some("source-undigested"));
    };
    let mut paths = BTreeSet::new();
    for row in rows {
        let Some(path) = row["path"].as_str() else {
            return Ok(Some("source-undigested"));
        };
        if !row["sha256"].is_string() {
            return Ok(Some("source-undigested"));
        }
        paths.insert(path);
    }
    for source in digestible_sources(&record["sources"]) {
        if !paths.contains(source.split('#').next().unwrap_or(&source)) {
            return Ok(Some("source-undigested"));
        }
    }
    for path in paths {
        let resolved = match std::fs::canonicalize(roots.resolve(path)) {
            Ok(path) => path,
            Err(_) => return Ok(Some("source-missing")),
        };
        if !roots.contains(&resolved)? || !resolved.is_file() {
            return Ok(Some("source-missing"));
        }
    }
    Ok(None)
}
