//! Shared automatic-query, citation-path and secret-projection policy.
use crate::{Result, RuntimeError};
use fancy_regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, sync::OnceLock};

const LABELS: &[&str] = &[
    "private key",
    "GitHub token",
    "GitLab token",
    "Slack token",
    "Slack webhook",
    "Discord webhook",
    "Telegram bot token",
    "AWS access key",
    "Google API key",
    "Google OAuth token",
    "OpenAI-style token",
    "Stripe secret key",
    "npm token",
    "PyPI token",
    "Hugging Face token",
    "SendGrid key",
    "Shopify token",
    "DigitalOcean token",
    "JWT",
    "Laravel application key",
    "credential in URL",
    "authorization header",
    "basic credentials",
    "bearer token",
    "assigned credential",
    "email address",
    "phone number",
    "customer identifier",
    "customer number",
];
const WORD: &str = r"[\p{L}\p{N}_]";
const BOUNDARY: &str =
    r"(?:(?<![\p{L}\p{N}_])(?=[\p{L}\p{N}_])|(?<=[\p{L}\p{N}_])(?![\p{L}\p{N}_]))";
struct Rule {
    label: &'static str,
    expression: Regex,
    original: String,
    flags: u64,
}
struct Policy {
    secrets: Vec<Rule>,
    private: Vec<Rule>,
    raw: Regex,
    quoted: Regex,
    block: Regex,
    assigned_key: Regex,
    assigned_at: Regex,
    contacts: BTreeSet<String>,
    fingerprint: String,
}
fn failure() -> RuntimeError {
    RuntimeError::policy("Shared-text policy could not safely inspect this image")
}
fn translate(pattern: &str, flags: u64) -> String {
    let insensitive = flags & 2 != 0;
    let mut out = String::new();
    let chars = pattern.chars().collect::<Vec<_>>();
    let mut at = 0;
    let mut in_class = false;
    while at < chars.len() {
        let c = chars[at];
        if c == '\\' && at + 1 < chars.len() {
            let next = chars[at + 1];
            match next {
                'w' => out.push_str(if in_class { r"\p{L}\p{N}_" } else { WORD }),
                'W' => out.push_str(r"[^\p{L}\p{N}_]"),
                'b' if !in_class => out.push_str(BOUNDARY),
                's' => out.push_str(if in_class {
                    r"\s\u{1c}-\u{1f}"
                } else {
                    r"[\s\u{1c}-\u{1f}]"
                }),
                'Z' => out.push_str(r"\z"),
                _ => {
                    out.push('\\');
                    out.push(next);
                }
            }
            at += 2;
            continue;
        }
        if c == '[' {
            in_class = true;
            out.push(c);
            at += 1;
            continue;
        }
        if c == ']' {
            in_class = false;
            out.push(c);
            at += 1;
            continue;
        }
        if insensitive
            && in_class
            && at + 2 < chars.len()
            && chars[at + 1] == '-'
            && ((c == 'A' && chars[at + 2] == 'Z') || (c == 'a' && chars[at + 2] == 'z'))
        {
            out.push(c);
            out.push('-');
            out.push(chars[at + 2]);
            out.push_str("İıſK");
            at += 3;
            continue;
        }
        // Python IGNORECASE additionally folds the dotted/dotless I; spans remain in the original UTF-8 image.
        if insensitive && !in_class && (c == 'i' || c == 'I') {
            out.push_str("[iIİı]");
        } else {
            out.push(c);
        }
        at += 1;
    }
    let mut modes = String::new();
    if flags & 2 != 0 {
        modes.push('i');
    }
    if flags & 8 != 0 {
        modes.push('m');
    }
    if flags & 16 != 0 {
        modes.push('s');
    }
    if modes.is_empty() {
        out
    } else {
        format!("(?{modes}){out}")
    }
}
fn compile(pattern: &str, flags: u64) -> std::result::Result<Regex, String> {
    Regex::new(&translate(pattern, flags))
        .map_err(|_| "Shared-text policy expression is invalid".into())
}
fn rules(value: &Value) -> std::result::Result<Vec<Rule>, String> {
    let mut out = Vec::new();
    for (label, row) in value.as_object().ok_or("Policy rules must be an object")? {
        let label = LABELS
            .iter()
            .copied()
            .find(|known| *known == label)
            .ok_or("Unknown policy label")?;
        let original = row["pattern"]
            .as_str()
            .ok_or("Policy pattern missing")?
            .to_string();
        let flags = row["flags"].as_u64().ok_or("Policy flags missing")?;
        out.push(Rule {
            label,
            expression: compile(&original, flags)?,
            original,
            flags,
        });
    }
    Ok(out)
}
fn initialize() -> std::result::Result<Policy, String> {
    let source: Value = serde_json::from_str(include_str!("privacy-policy.json"))
        .map_err(|_| "Invalid policy definition")?;
    let secrets = rules(&source["SECRET_PATTERNS"])?;
    let private = rules(&source["PRIVATE_PATTERNS"])?;
    let make = |key: &str| {
        compile(
            source[key]["pattern"]
                .as_str()
                .ok_or("Missing policy pattern")?,
            source[key]["flags"].as_u64().ok_or("Missing flags")?,
        )
    };
    let assigned = secrets
        .iter()
        .find(|rule| rule.label == "assigned credential")
        .ok_or("Missing assignment policy")?;
    let assigned_at = Regex::new(&format!(
        r"\A(?:{})",
        translate(&assigned.original, assigned.flags)
    ))
    .map_err(|_| "Assignment anchor invalid")?;
    let assigned_key = compile(
        r"(?<![A-Za-z0-9])(?:password|passwd|secret(?:_access)?(?:_key|_token)?|api[_-]?key|access[_-]?(?:token|key)|auth[_-]?token|refresh[_-]?token|private[_-]?key|account[_-]?key|shared[_-]?access[_-]?key)[ \t]*[:=]",
        34,
    )?;
    let mut parts = vec![
        "1".to_string(),
        source["PRIVATE_KEY_BLOCK"]["pattern"]
            .as_str()
            .ok_or("Policy block missing")?
            .into(),
        source["QUOTED_CREDENTIAL"]["pattern"]
            .as_str()
            .ok_or("Quoted policy missing")?
            .into(),
    ];
    let mut keyed = vec![
        "assigned credential",
        "Laravel application key",
        "authorization header",
        "basic credentials",
    ];
    keyed.sort();
    parts.extend(keyed.into_iter().map(String::from));
    for rule in &secrets {
        parts.extend([
            rule.label.to_string(),
            rule.original.clone(),
            rule.flags.to_string(),
        ]);
    }
    let fingerprint = crate::domain::hex_lower(Sha256::digest(parts.join("\0").as_bytes()));
    let contacts = source["CONTACT_WORDS"]
        .as_array()
        .ok_or("Contact words missing")?
        .iter()
        .map(|word| {
            word.as_str()
                .map(String::from)
                .ok_or("Contact word invalid")
        })
        .collect::<std::result::Result<BTreeSet<_>, _>>()?;
    Ok(Policy {
        secrets,
        private,
        raw: make("RAW_TEXT_PATTERN")?,
        quoted: make("QUOTED_CREDENTIAL")?,
        block: make("PRIVATE_KEY_BLOCK")?,
        assigned_key,
        assigned_at,
        contacts,
        fingerprint,
    })
}
fn policy() -> Result<&'static Policy> {
    static POLICY: OnceLock<std::result::Result<Policy, String>> = OnceLock::new();
    POLICY
        .get_or_init(initialize)
        .as_ref()
        .map_err(|_| failure())
}
/// Apply a static shared policy expression with Python Unicode boundaries.
pub fn policy_pattern_matches(pattern: &str, flags: u64, text: &str) -> Result<bool> {
    compile(pattern, flags)
        .map_err(|_| failure())?
        .is_match(text)
        .map_err(|_| failure())
}
fn match_ranges(rule: &Rule, text: &str, p: &Policy) -> Result<Vec<(usize, usize)>> {
    if rule.label != "assigned credential" {
        return rule
            .expression
            .find_iter(text)
            .map(|found| found.map(|m| (m.start(), m.end())).map_err(|_| failure()))
            .collect();
    }
    let mut result = Vec::new();
    let mut resume = 0;
    for candidate in p.assigned_key.find_iter(text) {
        let candidate = candidate.map_err(|_| failure())?;
        let mut start = candidate.start();
        if start < resume {
            continue;
        }
        while start > resume && text[..start].ends_with('_') {
            let segment_end = start - 1;
            let mut segment_start = segment_end;
            while segment_start > resume {
                let Some((position, c)) = text[..segment_start].char_indices().next_back() else {
                    break;
                };
                if !assigned_prefix(c) {
                    break;
                }
                segment_start = position;
            }
            if segment_start == segment_end {
                break;
            }
            if segment_start == resume
                && resume > 0
                && text[..resume].chars().last().is_some_and(assigned_prefix)
            {
                break;
            }
            start = segment_start;
        }
        if let Some(found) = p.assigned_at.find(&text[start..]).map_err(|_| failure())? {
            let span = (start + found.start(), start + found.end());
            resume = span.1;
            result.push(span);
        }
    }
    Ok(result)
}
fn assigned_prefix(c: char) -> bool {
    c.is_ascii_alphanumeric() || ['İ', 'ı', 'ſ', 'K'].contains(&c)
}
pub fn secret_label(text: &str) -> Result<Option<&'static str>> {
    let p = policy()?;
    for rule in &p.secrets {
        if !match_ranges(rule, text, p)?.is_empty() {
            return Ok(Some(rule.label));
        }
    }
    Ok(None)
}
pub fn sensitive_label(text: &str) -> Result<Option<&'static str>> {
    if let Some(label) = secret_label(text)? {
        return Ok(Some(label));
    }
    for rule in &policy()?.private {
        if rule.expression.is_match(text).map_err(|_| failure())? {
            return Ok(Some(rule.label));
        }
    }
    Ok(None)
}
pub fn secret_policy_fingerprint() -> Result<String> {
    Ok(policy()?.fingerprint.clone())
}
pub fn guard_shared_text(label: &str, values: &[String]) -> Result<()> {
    if let Some(kind) = sensitive_label(&values.join("\n"))? {
        return Err(RuntimeError::policy(format!(
            "{label} contains {kind}; replace it with a sanitized summary - nothing was stored"
        )));
    }
    Ok(())
}
fn replace_ranges(text: &str, ranges: &[(usize, usize, String)]) -> String {
    let mut out = String::new();
    let mut end = 0;
    for (start, stop, replacement) in ranges {
        if *start < end {
            end = end.max(*stop);
            continue;
        }
        out.push_str(&text[end..*start]);
        out.push_str(replacement);
        end = *stop;
    }
    out.push_str(&text[end..]);
    out
}
fn masked_span(
    p: &Policy,
    text: &str,
    label: &str,
    start: usize,
    end: usize,
) -> Result<(usize, usize)> {
    if label == "private key" {
        return Ok((
            start,
            p.block
                .find(&text[start..])
                .map_err(|_| failure())?
                .filter(|m| m.start() == 0)
                .map(|m| start + m.end())
                .unwrap_or(end),
        ));
    }
    if ![
        "assigned credential",
        "Laravel application key",
        "authorization header",
        "basic credentials",
    ]
    .contains(&label)
    {
        return Ok((start, end));
    }
    let Some((position, c)) = text[start..end]
        .char_indices()
        .find(|(_, c)| *c == ':' || *c == '=')
    else {
        return Ok((start, end));
    };
    let mut value = start + position + c.len_utf8();
    while value < end && matches!(text.as_bytes()[value], b' ' | b'\t') {
        value += 1;
    }
    if label == "assigned credential"
        && let Some(capture) = p.quoted.captures(&text[start..]).map_err(|_| failure())?
        && capture.get(0).is_some_and(|m| m.start() == 0)
        && let (Some(quote), Some(whole)) = (capture.get(1), capture.get(0))
    {
        return Ok((start + quote.start(), start + whole.end()));
    }
    if text[end..]
        .chars()
        .next()
        .is_some_and(|c| ['\'', '"', '\u{60}'].contains(&c))
    {
        return Ok((value, end));
    }
    Ok((
        value,
        text[end..]
            .find('\n')
            .map(|offset| end + offset)
            .unwrap_or(text.len()),
    ))
}
pub fn mask_secrets(original: &str) -> Result<(String, usize)> {
    let p = policy()?;
    let mut text = original.to_string();
    let mut count = 0;
    for _ in 0..3 {
        let mut spans = Vec::new();
        for rule in &p.secrets {
            for (start, end) in match_ranges(rule, &text, p)? {
                let (a, b) = masked_span(p, &text, rule.label, start, end)?;
                spans.push((a, b, rule.label));
            }
        }
        spans.sort_by(|a, b| (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2)));
        if spans.is_empty() {
            return Ok((text, count));
        }
        let mut ranges = Vec::new();
        let mut end = 0;
        for (start, stop, label) in spans {
            if start < end {
                if let Some(last) = ranges.last_mut() {
                    let row: &mut (usize, usize, String) = last;
                    row.1 = row.1.max(stop);
                }
                end = end.max(stop);
                continue;
            }
            ranges.push((start, stop, format!("[redacted: {label}]")));
            end = stop;
            count += 1;
        }
        text = replace_ranges(&text, &ranges);
    }
    for rule in &p.secrets {
        if !match_ranges(rule, &text, p)?.is_empty() {
            return Err(RuntimeError::policy(
                "possible secret survives masking; value intentionally not printed",
            ));
        }
    }
    Ok((text, count))
}
pub fn indexed_secret_policy_reason(texts: &[&str]) -> Option<&'static str> {
    for text in texts {
        if match mask_secrets(text) {
            Ok((masked, _)) => masked != *text,
            Err(_) => true,
        } {
            return Some("secret-policy");
        }
    }
    None
}
fn substitute(regex: &Regex, text: &str, keep_label: bool) -> Result<String> {
    let mut ranges = Vec::new();
    for found in regex.captures_iter(text) {
        let found = found.map_err(|_| failure())?;
        let whole = found.get(0).ok_or_else(failure)?;
        let replacement = if keep_label {
            found
                .name("value")
                .map(|value| format!("{} ", &text[whole.start()..value.start()]))
                .unwrap_or(" ".into())
        } else {
            " ".into()
        };
        ranges.push((whole.start(), whole.end(), replacement));
    }
    Ok(replace_ranges(text, &ranges))
}
pub fn sanitize_automatic_query(original: &str) -> Result<String> {
    let p = policy()?;
    let mut text = substitute(&p.block, original, false)?;
    text = substitute(&p.quoted, &text, false)?;
    let identifier = p
        .private
        .iter()
        .find(|rule| rule.label == "customer identifier")
        .ok_or_else(failure)?;
    if let Some(found) = identifier.expression.find(&text).map_err(|_| failure())? {
        text.truncate(found.start());
    }
    for _ in 0..3 {
        let previous = text.clone();
        for rule in &p.secrets {
            let ranges = match_ranges(rule, &text, p)?
                .into_iter()
                .map(|(a, b)| (a, b, " ".into()))
                .collect::<Vec<_>>();
            text = replace_ranges(&text, &ranges);
        }
        for rule in &p.private {
            if rule.label != "customer identifier" {
                text = substitute(&rule.expression, &text, true)?;
            }
        }
        text = substitute(&p.raw, &text, false)?;
        if text == previous {
            break;
        }
    }
    if secret_label(&text)?.is_some() {
        return Err(RuntimeError::policy(
            "Automatic memory query still carries a secret after redaction",
        ));
    }
    if text != original {
        let folded = crate::query::folded(&text);
        let residue = folded
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .filter(|word| !word.is_empty())
            .collect::<Vec<_>>();
        if residue.is_empty() || residue.iter().all(|word| p.contacts.contains(*word)) {
            return Ok(String::new());
        }
    }
    Ok(text.trim_matches([' ', '\t', '\r', '\n', ',', ';']).into())
}
pub fn source_path_problem(value: &str) -> Option<&'static str> {
    let head = value.split('#').next().unwrap_or(value);
    let parts = head
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let name = crate::query::folded(parts.last().copied().unwrap_or(""));
    let suffix = name
        .rsplit_once('.')
        .map(|(_, suffix)| suffix)
        .unwrap_or("");
    if name == ".env"
        || (name.starts_with(".env.") && name.len() > 5)
        || ["secret", "secrets"].contains(&name.as_str())
        || (name.starts_with("id_")
            && name.len() > 3
            && name[3..].chars().all(|c| c.is_ascii_alphanumeric()))
        || ["pem", "key", "p12", "pfx", "jks", "keystore"].contains(&suffix)
    {
        return Some("Sensitive source paths are refused");
    }
    if [
        "memory-bank/local/",
        "memory-bank/chunks/",
        "project-brain/local/",
        "project-brain/dynamic/",
        "project-brain/archive/",
        "project-brain/control/",
    ]
    .iter()
    .any(|prefix| head.starts_with(prefix))
    {
        return Some("Cite canonical project sources, not private runtime state or derived memory");
    }
    if parts.iter().any(|part| {
        let value = crate::query::folded(part);
        [
            ".git",
            ".ssh",
            ".aws",
            ".kube",
            "node_modules",
            "vendor",
            ".venv",
            "__pycache__",
            "secrets",
            ".secrets",
            "credentials",
        ]
        .contains(&value.as_str())
            || value.starts_with(".env")
    }) || ["pem", "key", "p12", "pfx", "sqlite", "db"].contains(&suffix)
        || name == "credentials.json"
    {
        return Some("Private or dependency source paths are refused");
    }
    if head.starts_with('/') || parts.contains(&"..") || head.contains('\\') || head.contains(':') {
        return Some("Use project-relative source paths");
    }
    None
}
