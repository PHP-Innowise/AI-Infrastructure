//! Public extraction contract retained separately from full YAML validation.
use crate::{Result, RuntimeError, common};
use std::path::Path;
#[derive(Clone, Debug)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub content: String,
}
fn unquote(s: &str) -> String {
    s.trim().trim_matches('"').trim_matches('\'').to_owned()
}
pub fn parse_skill_md(path: &Path) -> Result<Skill> {
    let content = common::read(&path.join("SKILL.md"))?;
    let lines: Vec<_> = content.split('\n').collect();
    if lines.first().map(|s| s.trim()) != Some("---") {
        return Err(RuntimeError::invalid(
            "SKILL.md missing frontmatter (no opening ---)",
        ));
    }
    let end = lines
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, s)| s.trim() == "---")
        .map(|(i, _)| i)
        .ok_or_else(|| RuntimeError::invalid("SKILL.md missing frontmatter (no closing ---)"))?;
    let (mut name, mut description) = (String::new(), String::new());
    let mut i = 1;
    while i < end {
        let line = lines[i];
        if let Some(v) = line.strip_prefix("name:") {
            name = unquote(v);
        } else if let Some(v) = line.strip_prefix("description:") {
            let raw = v.trim();
            if [">", "|", ">-", "|-"].contains(&raw) {
                let mut parts = vec![];
                i += 1;
                while i < end && (lines[i].starts_with("  ") || lines[i].starts_with('\t')) {
                    parts.push(lines[i].trim());
                    i += 1;
                }
                description = parts.join(" ");
                continue;
            }
            description = unquote(raw);
        }
        i += 1;
    }
    Ok(Skill {
        name,
        description,
        content,
    })
}
