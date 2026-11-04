use anyhow::{Result, bail};
use serde_json::Value;
use std::collections::BTreeSet;
pub fn decode(raw: &[u8], bytes: usize, depth: usize, nodes: usize) -> Result<Value> {
    infra_core::strict_json::parse(raw, bytes, depth, nodes).map_err(|error| {
        anyhow::anyhow!(match error.code.as_deref() {
            Some(
                "strict-json:byte-limit" | "strict-json:depth-limit" | "strict-json:node-limit",
            ) => "input_bound",
            Some("strict-json:duplicate-key") => "duplicate_key",
            Some("strict-json:nonfinite") => "nonfinite",
            Some("strict-json:encoding") => "input_encoding",
            _ => "input_schema",
        })
    })
}
pub fn keys(v: &Value, allowed: &[&str], required: &[&str]) -> Result<()> {
    let m = v
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("input_schema"))?;
    if m.keys().any(|k| !allowed.contains(&k.as_str()))
        || required.iter().any(|k| !m.contains_key(*k))
    {
        bail!("input_schema");
    }
    Ok(())
}
pub fn wire(v: &Value) -> Result<Vec<u8>> {
    Ok(infra_core::domain::json::canonical(v)?.into_bytes())
}
pub fn sha(b: &[u8]) -> String {
    infra_core::query::sha256(b)
}
pub fn text(v: &Value, max: usize, nonempty: bool, multiline: bool) -> Result<&str> {
    let s = v.as_str().ok_or_else(|| anyhow::anyhow!("input_schema"))?;
    if s.chars().count() > max || nonempty && s.trim().is_empty() {
        bail!("input_schema");
    }
    if s.chars()
        .any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\r' | '\t')))
    {
        bail!("input_encoding");
    }
    Ok(s)
}
pub fn uuid(s: &str) -> Result<uuid::Uuid> {
    let u = uuid::Uuid::parse_str(s)?;
    if u.to_string() != s {
        bail!("identity");
    }
    Ok(u)
}
pub fn digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
pub fn path(s: &str, max: usize) -> Result<()> {
    if s.is_empty()
        || s.len() > max
        || s.starts_with('/')
        || s.contains('\\')
        || s.contains(':')
        || s.split('/').any(|p| p.is_empty() || p == "." || p == "..")
        || s.chars().any(char::is_control)
    {
        bail!("source_path");
    }
    Ok(())
}
pub fn unique_strings(a: &[String]) -> bool {
    a.iter().collect::<BTreeSet<_>>().len() == a.len()
}
