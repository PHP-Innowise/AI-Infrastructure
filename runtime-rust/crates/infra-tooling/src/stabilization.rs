use crate::common;
use infra_core::{Result, RuntimeError, domain, fs as safe};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
pub fn check(root: &Path, editions: &[String]) -> Result<Value> {
    let heading = regex::Regex::new(r"(?m)^### Rule:\s*(.+?)\s*$")
        .map_err(|_| RuntimeError::invalid("Invalid rule heading expression"))?;
    let sections = regex::Regex::new(r"(?m)^## (.+?)\s*$")
        .map_err(|_| RuntimeError::invalid("Invalid section expression"))?;
    let field = regex::Regex::new(r"(?m)^\*\*([A-Za-z][A-Za-z -]*):\*\*[ \t]*(.*)$")
        .map_err(|_| RuntimeError::invalid("Invalid rule field expression"))?;
    let obligation = regex::Regex::new(r"\bMUST(?:\s+NOT)?\b")
        .map_err(|_| RuntimeError::invalid("Invalid obligation expression"))?;
    let selected = if editions.is_empty() {
        common::EDITIONS
            .iter()
            .map(|name| name.to_string())
            .collect()
    } else {
        editions.to_vec()
    };
    let mut findings = Vec::new();
    let mut count = 0;
    let mut skipped = Vec::new();
    for edition in selected {
        let edition = common::edition(&edition)?;
        let relative = format!("{edition}/.claude/STABILIZATION.md");
        let path = root.join(&relative);
        if !path.is_file() {
            findings.push(json!({"path":relative,"rule":"","message":"missing"}));
            continue;
        }
        let text = safe::read_text(&path)?;
        let marks = heading
            .captures_iter(&text)
            .map(|capture| {
                (
                    capture.get(0).unwrap().start(),
                    capture.get(0).unwrap().end(),
                    capture[1].to_string(),
                )
            })
            .collect::<Vec<_>>();
        let known = marks
            .iter()
            .filter(|(_, _, name)| name != "[Short Name]")
            .map(|(_, _, name)| name.clone())
            .collect::<BTreeSet<_>>();
        if known.is_empty() {
            skipped.push(relative);
            continue;
        }
        let section_marks = sections
            .captures_iter(&text)
            .map(|capture| (capture.get(0).unwrap().start(), capture[1].to_string()))
            .collect::<Vec<_>>();
        for (index, (start, end, name)) in marks.iter().enumerate() {
            if name == "[Short Name]" {
                continue;
            }
            count += 1;
            let finish = marks
                .get(index + 1)
                .map(|mark| mark.0)
                .unwrap_or(text.len());
            let body = &text[*end..finish];
            let fields = field
                .captures_iter(body)
                .map(|capture| (capture[1].trim().to_string(), capture[2].trim().to_string()))
                .collect::<BTreeMap<_, _>>();
            let mut problems = Vec::new();
            for required in ["Trigger", "Root cause", "Rule", "Example", "Enforcement"] {
                if !fields.contains_key(required) {
                    problems.push(format!("Missing required field {required}"));
                }
            }
            if fields
                .get("Rule")
                .is_some_and(|value| !value.is_empty() && !obligation.is_match(value))
            {
                problems.push("Rule must state MUST or MUST NOT".into());
            }
            if fields
                .get("Enforcement")
                .is_some_and(|value| value.is_empty())
            {
                problems.push("Enforcement must name its enforcement point".into());
            }
            for key in ["Added", "Retired"] {
                if let Some(value) = fields.get(key)
                    && domain::python_date(value).is_none()
                {
                    problems.push(format!("{key} must be an ISO date"));
                }
            }
            let section = section_marks
                .iter()
                .rev()
                .find(|(at, _)| at <= start)
                .map(|(_, name)| name.as_str())
                .unwrap_or("");
            if fields.contains_key("Retired") != (section == "Retired rules") {
                problems.push(
                    "Retired rules must be in the Retired rules section and carry a Retired date"
                        .into(),
                );
            }
            if fields
                .get("Superseded-by")
                .is_some_and(|replacement| !replacement.is_empty() && !known.contains(replacement))
            {
                problems.push("Superseded-by names no rule in this file".into());
            }
            if let Some(evidence) = fields.get("Evidence").filter(|value| !value.is_empty()) {
                if !domain::is_uuid4(evidence) {
                    problems.push("Evidence must be a record UUID".into());
                } else {
                    let brain = root.join(edition).join("project-brain");
                    let mut exists = false;
                    for directory in ["dynamic", "archive"] {
                        let base = brain.join(directory);
                        if base.is_dir()
                            && common::walk(&base)?.iter().any(|path| {
                                path.rsplit('/').next() == Some(&format!("{evidence}.md"))
                            })
                        {
                            exists = true;
                        }
                    }
                    if !exists {
                        problems.push("Evidence names no Project Brain record on disk".into());
                    }
                }
            }
            for message in problems {
                findings.push(json!({"path":relative,"rule":name,"message":message}));
            }
        }
    }
    Ok(json!({"passed":findings.is_empty(),"rules":count,"skipped":skipped,"findings":findings}))
}
