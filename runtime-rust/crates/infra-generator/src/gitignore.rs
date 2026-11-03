use crate::common::*;
use infra_core::{Result, RuntimeError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json as object};
use std::collections::BTreeSet;

pub const BEGIN: &str = "# BEGIN Infrastructure-Creator .gitignore v1";
pub const END: &str = "# END Infrastructure-Creator .gitignore v1";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeResult {
    pub staged_bytes: Vec<u8>,
    pub metadata: Value,
}
pub fn requirements(value: &Value) -> Result<Vec<String>> {
    let value = if value.is_object() {
        let keys = keys(value);
        if keys != BTreeSet::from(["requirements".into()]) {
            return Err(RuntimeError::invalid(
                "requirement JSON object must contain only 'requirements'",
            ));
        }
        &value["requirements"]
    } else {
        value
    };
    let items = value.as_array().ok_or_else(|| {
        RuntimeError::invalid("requirement JSON must be an array or {'requirements': [...]}")
    })?;
    let controls = expression(r"\p{C}")?;
    let mut output = BTreeSet::new();
    for (index, item) in items.iter().enumerate() {
        let value = item.as_str().ok_or_else(|| {
            RuntimeError::invalid(format!("requirement {index} must be a string"))
        })?;
        if value.is_empty() || value.starts_with('#') || controls.is_match(value) {
            return Err(RuntimeError::invalid(format!(
                "requirement {index} is empty, a comment, or contains a control character"
            )));
        }
        output.insert(value.to_string());
    }
    Ok(output.into_iter().collect())
}
pub fn load_requirements(raw: &[u8]) -> Result<Vec<String>> {
    if raw.contains(&0) {
        return Err(RuntimeError::invalid("requirement JSON contains NUL"));
    }
    let text = std::str::from_utf8(raw.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(raw))
        .map_err(|_| RuntimeError::invalid("requirement JSON must be valid UTF-8"))?;
    requirements(&infra_core::domain::json::parse(text)?)
}
pub fn combine(documents: &[Vec<u8>]) -> Result<Vec<String>> {
    if documents.is_empty() {
        return Err(RuntimeError::invalid(
            "at least one requirement document is required",
        ));
    }
    let mut values = BTreeSet::new();
    for document in documents {
        values.extend(load_requirements(document)?);
    }
    Ok(values.into_iter().collect())
}
fn lines_keep(text: &str) -> Vec<&str> {
    let mut output = vec![];
    let mut start = 0;
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'\r' {
            at += 1;
            if bytes.get(at) == Some(&b'\n') {
                at += 1;
            }
            output.push(&text[start..at]);
            start = at;
        } else if bytes[at] == b'\n' {
            at += 1;
            output.push(&text[start..at]);
            start = at;
        } else {
            at += 1;
        }
    }
    if start < text.len() {
        output.push(&text[start..]);
    }
    output
}
fn line_value(text: &str) -> &str {
    trailing(text, &['\r', '\n'])
}
fn newline_policy(text: &str) -> &'static str {
    let mut found = false;
    let mut at = 0;
    let bytes = text.as_bytes();
    while at < bytes.len() {
        if bytes[at] == b'\r' {
            if bytes.get(at + 1) != Some(&b'\n') {
                return "\n";
            }
            found = true;
            at += 2;
        } else if bytes[at] == b'\n' {
            return "\n";
        } else {
            at += 1;
        }
    }
    if found { "\r\n" } else { "\n" }
}
pub fn merge(existing: Option<&[u8]>, required: &[String]) -> Result<MergeResult> {
    let original = existing.unwrap_or(&[]);
    if original.contains(&0) {
        return Err(RuntimeError::invalid("existing .gitignore contains NUL"));
    }
    let bom = original.starts_with(&[0xef, 0xbb, 0xbf]);
    let payload = if bom { &original[3..] } else { original };
    let text = std::str::from_utf8(payload)
        .map_err(|_| RuntimeError::invalid("existing .gitignore must be valid UTF-8"))?;
    let required = requirements(&object!(required))?;
    let newline = newline_policy(text);
    let lines = lines_keep(text);
    let marker = expression(r"^# (?:BEGIN|END) Infrastructure-Creator \.gitignore v\d+$")?;
    let markers = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| marker.is_match(line_value(line)))
        .map(|(index, line)| (index, line_value(line)))
        .collect::<Vec<_>>();
    let span = if markers.is_empty() {
        None
    } else {
        if markers.len() != 2
            || markers[0].1 != BEGIN
            || markers[1].1 != END
            || markers[1].0 <= markers[0].0
        {
            return Err(RuntimeError::conflict(
                "existing .gitignore has malformed, duplicate, or unsupported managed markers",
            ));
        }
        Some((markers[0].0, markers[1].0))
    };
    let outside = lines
        .iter()
        .enumerate()
        .filter(|(index, _)| !span.is_some_and(|(start, end)| start <= *index && *index <= end))
        .map(|(_, line)| line_value(line))
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<BTreeSet<_>>();
    let mut managed = required
        .iter()
        .filter(|value| !outside.contains(value.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    for value in &managed {
        let opposite = value
            .strip_prefix('!')
            .map(str::to_owned)
            .unwrap_or_else(|| format!("!{value}"));
        if outside.contains(opposite.as_str()) {
            return Err(RuntimeError::conflict(format!(
                "requirements contradict team-owned entries; an explicit decision is required: {value}"
            )));
        }
    }
    managed
        .sort_by(|left, right| (left.starts_with('!'), left).cmp(&(right.starts_with('!'), right)));
    let block = [vec![BEGIN.to_string()], managed, vec![END.to_string()]]
        .concat()
        .join(newline)
        + newline;
    let (proposal, strategy) = match span {
        None if block == format!("{BEGIN}{newline}{END}{newline}") => {
            (text.to_string(), "unchanged")
        }
        None => {
            let separator = if text.is_empty() || text.ends_with(['\r', '\n']) {
                ""
            } else {
                newline
            };
            let blank = if text.is_empty()
                || text.ends_with(&format!("{newline}{newline}"))
                || text.ends_with("\r\r")
            {
                ""
            } else {
                newline
            };
            (
                format!("{text}{separator}{blank}{block}"),
                "append-managed-block",
            )
        }
        Some((start, end)) => {
            let output = lines[..start].concat() + &block + &lines[end + 1..].concat();
            let strategy = if output == text {
                "unchanged"
            } else {
                "replace-managed-block"
            };
            (output, strategy)
        }
    };
    let mut staged = if bom { vec![0xef, 0xbb, 0xbf] } else { vec![] };
    staged.extend_from_slice(proposal.as_bytes());
    let digest = sha(&staged);
    let metadata = object!({"schema_version":1,"path":".gitignore","origin":"Infrastructure-Creator","strategy":strategy,"block_version":1,"newline":if newline=="\r\n"{"crlf"}else{"lf"},"changed":staged!=original,"original_sha256":sha(original),"proposal_sha256":digest,"resolved_sha256":digest,"requirements":required});
    Ok(MergeResult {
        staged_bytes: staged,
        metadata,
    })
}
