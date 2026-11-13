//! Approved, bounded hook policy data. This module never evaluates a command.
use crate::{TailTrim, shell};
use infra_core::{Result, RuntimeError, bank::publication::PublicationFiles};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path};
pub const PATH: &str = "project-brain/config/hook-policy.json";
pub const MAX_BYTES: usize = 64 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub profile_path: String,
    pub profile_sha256: String,
    pub selection_sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandPrefix {
    pub tool: String,
    pub argv: Vec<String>,
    pub options_with_values: Vec<String>,
    pub short_options_with_values: String,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Matching {
    Exact,
    Console,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    DestructiveCommand,
    SecretExposure,
    VerificationBypass,
}
impl Reason {
    fn label(self) -> &'static str {
        match self {
            Self::DestructiveCommand => "destructive command",
            Self::SecretExposure => "secret exposure",
            Self::VerificationBypass => "verification bypass",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Condition {
    Always,
    AnyOption { patterns: Vec<String> },
    NoOption { patterns: Vec<String> },
    FirstArgument { patterns: Vec<String> },
    ArgumentContains { patterns: Vec<String> },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DenyRule {
    pub id: String,
    pub tool: String,
    pub command: Vec<String>,
    pub matching: Matching,
    pub condition: Condition,
    pub reason: Reason,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookPolicy {
    pub schema_version: u32,
    pub framework: String,
    pub evidence: Evidence,
    pub approved_tools: Vec<String>,
    pub project_command_prefixes: Vec<CommandPrefix>,
    pub deny_rules: Vec<DenyRule>,
}
fn invalid() -> RuntimeError {
    RuntimeError::policy("Native hook policy data or custody is invalid")
}
fn literal(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
fn slug(value: &str) -> bool {
    value.len() <= 64
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
}
fn hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn relative(value: &str) -> bool {
    literal(value, 1024)
        && !value.contains(['\\', ':'])
        && value
            .split('/')
            .all(|part| !part.is_empty() && ![".", ".."].contains(&part))
}
fn option(value: &str) -> bool {
    let value = value.strip_prefix("--").or_else(|| value.strip_prefix('-'));
    value.is_some_and(|value| {
        value.len() <= 63
            && value
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    })
}
fn entry(value: &str) -> String {
    let value = value
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(value)
        .tail_matches(".phar");
    if value == "wp-cli" {
        "wp".into()
    } else {
        value.into()
    }
}
fn pattern(value: &str) -> Result<regex::Regex> {
    if !literal(value, 128) {
        return Err(invalid());
    }
    let mut expression = String::from("^");
    let chars = value.chars().collect::<Vec<_>>();
    let mut at = 0;
    while at < chars.len() {
        match chars[at] {
            '*' => expression.push_str(".*"),
            '?' => expression.push('.'),
            '[' => {
                expression.push('[');
                at += 1;
                if chars.get(at) == Some(&'!') {
                    expression.push('^');
                    at += 1;
                }
                let begin = at;
                while at < chars.len() && chars[at] != ']' {
                    if matches!(chars[at], '\\' | '[' | '^') {
                        expression.push('\\');
                    }
                    expression.push(chars[at]);
                    at += 1;
                }
                if at == chars.len() || at == begin {
                    return Err(invalid());
                }
                expression.push(']');
            }
            ']' => return Err(invalid()),
            value => expression.push_str(&regex::escape(&value.to_string())),
        }
        at += 1;
    }
    expression.push('$');
    regex::Regex::new(&expression).map_err(|_| invalid())
}
fn patterns(condition: &Condition) -> Option<&[String]> {
    match condition {
        Condition::Always => None,
        Condition::AnyOption { patterns }
        | Condition::NoOption { patterns }
        | Condition::FirstArgument { patterns }
        | Condition::ArgumentContains { patterns } => Some(patterns),
    }
}
impl HookPolicy {
    pub fn selection_value(&self) -> Value {
        json!({"schema_version":self.schema_version,"framework":self.framework,"approved_tools":self.approved_tools,"project_command_prefixes":self.project_command_prefixes,"deny_rules":self.deny_rules})
    }
    pub fn selection_digest(&self) -> Result<String> {
        Ok(infra_core::query::sha256(
            infra_core::domain::json::canonical(&self.selection_value())?.as_bytes(),
        ))
    }
    pub fn validate_binding(&self, profile_sha256: &str, expected_selection: &Value) -> Result<()> {
        if !hex(profile_sha256)
            || self.evidence.profile_sha256 != profile_sha256
            || self.selection_value() != *expected_selection
            || self.selection_digest()? != self.evidence.selection_sha256
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn parse(raw: &[u8], expected_framework: Option<&str>) -> Result<Self> {
        let value = infra_core::strict_json::parse(raw, MAX_BYTES, 24, 20000)?;
        let policy: Self = serde_json::from_value(value).map_err(|_| invalid())?;
        policy.validate(expected_framework)?;
        Ok(policy)
    }
    pub fn validate(&self, expected_framework: Option<&str>) -> Result<()> {
        if self.schema_version != 1
            || !regex::Regex::new(r"^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$").is_ok_and(|pattern| {
                self.framework.len() <= 64 && pattern.is_match(&self.framework)
            })
            || expected_framework.is_some_and(|framework| framework != self.framework)
            || !relative(&self.evidence.profile_path)
            || !hex(&self.evidence.profile_sha256)
            || !hex(&self.evidence.selection_sha256)
            || self.approved_tools.len() > 64
            || self.project_command_prefixes.len() > 128
            || self.deny_rules.len() > 256
        {
            return Err(invalid());
        }
        let tools = self.approved_tools.iter().cloned().collect::<BTreeSet<_>>();
        if tools.len() != self.approved_tools.len() || tools.iter().any(|tool| !slug(tool)) {
            return Err(invalid());
        }
        let mut covered = BTreeSet::new();
        for prefix in &self.project_command_prefixes {
            if !tools.contains(&prefix.tool)
                || prefix.argv.is_empty()
                || prefix.argv.len() > 8
                || prefix.argv.iter().any(|word| !literal(word, 256))
                || prefix.options_with_values.len() > 32
                || prefix.options_with_values.iter().any(|word| !option(word))
                || prefix.short_options_with_values.len() > 26
                || !prefix
                    .short_options_with_values
                    .bytes()
                    .all(|byte| byte.is_ascii_alphabetic())
                || prefix
                    .short_options_with_values
                    .bytes()
                    .collect::<BTreeSet<_>>()
                    .len()
                    != prefix.short_options_with_values.len()
            {
                return Err(invalid());
            }
            covered.insert(prefix.tool.clone());
        }
        if covered != tools {
            return Err(invalid());
        }
        let mut ids = BTreeSet::new();
        for rule in &self.deny_rules {
            if !slug(&rule.id)
                || !ids.insert(&rule.id)
                || !tools.contains(&rule.tool)
                || rule.command.len() > 8
                || rule.command.iter().any(|word| !literal(word, 128))
            {
                return Err(invalid());
            }
            if let Some(patterns) = patterns(&rule.condition) {
                if patterns.is_empty() || patterns.len() > 16 {
                    return Err(invalid());
                }
                for value in patterns {
                    pattern(value)?;
                }
            }
        }
        if self.selection_digest()? != self.evidence.selection_sha256 {
            return Err(invalid());
        }
        Ok(())
    }
    fn effective(prefix: &CommandPrefix) -> Option<&[String]> {
        let words = &prefix.argv;
        let first = entry(words.first()?);
        if first == "php"
            || first
                .strip_prefix("php")
                .is_some_and(|value| value.starts_with(|c: char| c.is_ascii_digit()))
        {
            if let Some(at) = shell::php_script(words, 1) {
                Some(&words[at..])
            } else {
                Some(words)
            }
        } else {
            Some(words)
        }
    }
    pub fn knows_program(&self, program: &str) -> bool {
        self.project_command_prefixes
            .iter()
            .filter_map(Self::effective)
            .any(|words| words.first().is_some_and(|word| entry(word) == program))
    }
    fn next_option(prefix: &CommandPrefix, words: &[String], at: usize) -> usize {
        let word = &words[at];
        if !word.contains('=')
            && words.get(at + 1).is_some_and(|word| !word.starts_with('-'))
            && (prefix
                .options_with_values
                .iter()
                .any(|option| option == word)
                || shell::short_value(word, &prefix.short_options_with_values))
        {
            at + 2
        } else {
            at + 1
        }
    }
    fn condition(
        prefix: &CommandPrefix,
        rule: &DenyRule,
        words: &[String],
        first: usize,
        position: usize,
        console: Option<usize>,
    ) -> bool {
        let Some(patterns) = patterns(&rule.condition) else {
            return true;
        };
        let matches = |word: &str| {
            patterns
                .iter()
                .any(|value| pattern(value).is_ok_and(|pattern| pattern.is_match(word)))
        };
        match &rule.condition {
            Condition::Always => true,
            Condition::AnyOption { .. } | Condition::NoOption { .. } => {
                let found = words
                    .iter()
                    .enumerate()
                    .skip(first)
                    .take_while(|(_, word)| word.as_str() != "--")
                    .filter(|(at, _)| Some(*at) != console)
                    .any(|(_, word)| {
                        matches(word)
                            || console.is_some()
                                && patterns.iter().any(|value| {
                                    value.len() == 2
                                        && value.starts_with('-')
                                        && shell::short(
                                            word,
                                            value.chars().nth(1).unwrap_or(' '),
                                            &prefix.short_options_with_values,
                                        )
                                })
                    });
                if matches!(&rule.condition, Condition::NoOption { .. }) {
                    !found
                } else {
                    found
                }
            }
            Condition::FirstArgument { .. } => {
                let mut at = position;
                while at < words.len() && words[at].starts_with('-') {
                    at = Self::next_option(prefix, words, at);
                }
                words.get(at).is_some_and(|word| matches(word))
            }
            Condition::ArgumentContains { .. } => {
                let words = words
                    .iter()
                    .skip(position)
                    .filter(|word| !word.starts_with('-'))
                    .collect::<Vec<_>>();
                words.is_empty()
                    || words.iter().any(|word| {
                        patterns
                            .iter()
                            .any(|pattern| pattern.contains(word.as_str()))
                    })
            }
        }
    }
    pub fn classify(&self, words: &[String], program: &str, argument: usize) -> Option<String> {
        for prefix in &self.project_command_prefixes {
            let Some(expected) = Self::effective(prefix) else {
                continue;
            };
            if expected.first().is_none_or(|word| entry(word) != program) {
                continue;
            }
            let mut at = argument;
            let mut prefix_matches = true;
            for fixed in &expected[1..] {
                if !fixed.starts_with('-') {
                    while at < words.len()
                        && (words[at].starts_with('-') || words[at].starts_with('@'))
                    {
                        at = Self::next_option(prefix, words, at);
                    }
                }
                if words.get(at) != Some(fixed) {
                    prefix_matches = false;
                    break;
                }
                at += 1;
            }
            if !prefix_matches {
                continue;
            }
            let first = at;
            while at < words.len() && words[at].starts_with('-') {
                at = Self::next_option(prefix, words, at);
            }
            for rule in self
                .deny_rules
                .iter()
                .filter(|rule| rule.tool == prefix.tool)
            {
                let mut cursor = at;
                let mut matched = true;
                for expected in &rule.command {
                    if !expected.starts_with('-') {
                        while cursor < words.len()
                            && (words[cursor].starts_with('-') || words[cursor].starts_with('@'))
                        {
                            cursor = Self::next_option(prefix, words, cursor);
                        }
                    }
                    let Some(actual) = words.get(cursor) else {
                        matched = false;
                        break;
                    };
                    let accepted = match rule.matching {
                        Matching::Exact => actual == expected,
                        Matching::Console => {
                            let actual = actual.to_ascii_lowercase();
                            let expected = expected.to_ascii_lowercase();
                            let actual = actual.split(':').collect::<Vec<_>>();
                            let expected = expected.split(':').collect::<Vec<_>>();
                            actual.len() == expected.len()
                                && actual
                                    .iter()
                                    .zip(expected.iter())
                                    .all(|(actual, expected)| {
                                        !actual.is_empty() && expected.starts_with(actual)
                                    })
                        }
                    };
                    if !accepted {
                        matched = false;
                        break;
                    }
                    cursor += 1;
                }
                if matched
                    && Self::condition(
                        prefix,
                        rule,
                        words,
                        first,
                        cursor,
                        if matches!(rule.matching, Matching::Console) {
                            Some(at)
                        } else {
                            None
                        },
                    )
                {
                    return Some(format!(
                        "{} - approved project policy {}",
                        rule.reason.label(),
                        rule.id
                    ));
                }
            }
        }
        None
    }
}
pub fn load(root: &Path, expected_framework: Option<&str>) -> Result<Option<HookPolicy>> {
    let mut custody = PublicationFiles::new(root)?;
    let raw = custody.read(PATH, MAX_BYTES)?;
    let manifest = custody.read(".infra-manifest.json", 16 * 1024 * 1024)?;
    let Some(raw) = raw else {
        if manifest.is_some() {
            return Err(invalid());
        }
        return Ok(None);
    };
    let policy = HookPolicy::parse(&raw, expected_framework)?;
    if let Some(manifest) = manifest {
        let manifest = infra_core::strict_json::parse(&manifest, 8 * 1024 * 1024, 32, 300000)?;
        if manifest["files"][PATH].as_str() != Some(infra_core::query::sha256(&raw).as_str())
            || manifest["profile"].as_str() != Some(policy.evidence.profile_path.as_str())
        {
            return Err(invalid());
        }
    }
    Ok(Some(policy))
}
