//! Native conventional-commit grouping used by each edition's release skill.
use infra_core::{Result, RuntimeError, bank::publication::PublicationFiles, fs as safe};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, sync::OnceLock};
const INPUT_LIMIT: usize = 64 * 1024 * 1024;
const ORDER: &[&str] = &[
    "Added",
    "Changed",
    "Deprecated",
    "Removed",
    "Fixed",
    "Security",
];
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Commit {
    pub hash: String,
    pub message: String,
    pub category: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Categorization {
    pub commits: Vec<Commit>,
    pub categories: BTreeMap<String, Vec<Commit>>,
    pub markdown: String,
}
fn patterns() -> Result<&'static (Regex, Regex)> {
    // Python's word class is letters/numbers/underscore, not the additional
    // combining marks accepted by the Unicode Regex crate's general word class.
    // Its whitespace class also includes the four information separators.
    static PATTERNS: OnceLock<std::result::Result<(Regex, Regex), String>> = OnceLock::new();
    match PATTERNS.get_or_init(|| {
        let category = Regex::new(r"^([\p{L}\p{N}_]+)(?:\([\p{L}\p{N}_-]+\))?:[\s\x1c-\x1f]*(.+)")
            .map_err(|error| error.to_string())?;
        let strip = Regex::new(r"^([\p{L}\p{N}_]+)(?:\([\p{L}\p{N}_-]+\))?:[\s\x1c-\x1f]*")
            .map_err(|error| error.to_string())?;
        Ok((category, strip))
    }) {
        Ok(patterns) => Ok(patterns),
        Err(error) => Err(RuntimeError::invalid(format!(
            "Native release prefix grammar is invalid: {error}"
        ))),
    }
}
fn python_strip(value: &str) -> &str {
    value.trim_matches(|character: char| {
        character.is_whitespace() || ('\u{001c}'..='\u{001f}').contains(&character)
    })
}
pub fn parse_commit(line: &str) -> (String, String) {
    let line = python_strip(line);
    match line.split_once(' ') {
        Some((hash, message)) => (hash.into(), message.into()),
        None => (line.into(), String::new()),
    }
}
pub fn categorize_commit(message: &str) -> Result<&'static str> {
    let Some(found) = patterns()?.0.captures(message) else {
        return Ok("Changed");
    };
    let prefix = found[1].to_lowercase();
    Ok(match prefix.as_str() {
        "feat" | "feature" => "Added",
        "fix" | "bugfix" => "Fixed",
        "refactor" | "perf" | "improvement" | "update" => "Changed",
        "deprecate" | "deprecated" => "Deprecated",
        "remove" | "delete" => "Removed",
        "security" | "sec" => "Security",
        "docs" | "doc" => "Documentation",
        "build" | "ci" | "chore" => "Build",
        _ => "Changed",
    })
}
pub fn format_commit(commit: &Commit) -> Result<String> {
    let message = patterns()?.1.replace(&commit.message, "");
    let mut characters = message.chars();
    let formatted = if let Some(first) = characters.next() {
        let mut value = first.to_uppercase().collect::<String>();
        value.push_str(characters.as_str());
        value
    } else {
        String::new()
    };
    Ok(format!("- {formatted} ({})", commit.hash))
}
pub fn categorize_text(text: &str) -> Result<Categorization> {
    if text.len() > INPUT_LIMIT {
        return Err(RuntimeError::invalid(
            "Commit list exceeds the native release input bound",
        ));
    }
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut commits = Vec::new();
    let mut categories = BTreeMap::<String, Vec<Commit>>::new();
    for line in text.split('\n') {
        if python_strip(line).is_empty() {
            continue;
        }
        let (hash, message) = parse_commit(line);
        let category = categorize_commit(&message)?.to_string();
        let commit = Commit {
            hash,
            message,
            category: category.clone(),
        };
        categories.entry(category).or_default().push(commit.clone());
        commits.push(commit);
    }
    let mut order = ORDER
        .iter()
        .map(|value| (*value).to_string())
        .collect::<Vec<_>>();
    order.extend(
        categories
            .keys()
            .filter(|category| !ORDER.contains(&category.as_str()))
            .cloned(),
    );
    let mut markdown = String::new();
    for category in order {
        if let Some(commits) = categories.get(&category) {
            markdown.push_str(&format!("\n### {category}\n\n"));
            for commit in commits {
                markdown.push_str(&format_commit(commit)?);
                markdown.push('\n');
            }
        }
    }
    Ok(Categorization {
        commits,
        categories,
        markdown,
    })
}
pub fn execute(operation: &str, request: &Value) -> Result<Value> {
    if operation != "categorize" {
        return Err(RuntimeError::invalid(format!(
            "Unknown native release operation: {operation}"
        )));
    }
    let text = if let Some(text) = request["text"].as_str() {
        if request.get("input").is_some() || request.get("file").is_some() {
            return Err(RuntimeError::invalid(
                "Choose a commit-list file or explicit text, not both",
            ));
        }
        text.to_string()
    } else {
        let input = request["input"]
            .as_str()
            .or_else(|| request["file"].as_str())
            .or_else(|| {
                request["_positionals"]
                    .as_array()
                    .and_then(|values| values.first())
                    .and_then(Value::as_str)
            })
            .ok_or_else(|| {
                RuntimeError::invalid("release categorize requires a commit-list file")
            })?;
        let root = Path::new(request["root"].as_str().unwrap_or("."));
        let path = safe::absolute(&root.join(input))?;
        let parent = path
            .parent()
            .ok_or_else(|| RuntimeError::invalid("Commit list has no parent"))?;
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| RuntimeError::invalid("Commit-list name is not UTF-8"))?;
        let mut files = PublicationFiles::new(parent)?;
        let (bytes, _) = files
            .capture_source(name, INPUT_LIMIT)?
            .ok_or_else(|| RuntimeError::not_found("Commit-list file is absent"))?;
        String::from_utf8(bytes)
            .map_err(|_| RuntimeError::invalid("Commit-list file is not UTF-8"))?
    };
    let result = categorize_text(&text)?;
    if request["json"].as_bool().unwrap_or(false) {
        return Ok(serde_json::to_value(result)?);
    }
    Ok(json!({"_cli":{"code":0,"stdout":result.markdown,"stderr":""},"result":result}))
}
