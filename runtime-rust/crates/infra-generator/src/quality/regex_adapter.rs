//! Python-compatible regex matching for authored contracts and target evidence.
use fancy_regex::Regex;
use infra_core::{Result, RuntimeError};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone, Debug)]
pub struct Capture {
    pub start: usize,
    pub end: usize,
    pub groups: Vec<Option<String>>,
}
pub fn normalize(pattern: &str) -> String {
    // Python's word class is alphanumeric plus underscore; Rust's includes
    // combining marks/join controls. Keep the canonical evidence vocabulary.
    let pattern = pattern.replace(r"[^\W_]", r"[\p{L}\p{N}]");
    let mut out = String::new();
    let mut chars = pattern.chars().peekable();
    let mut class = false;
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('Z') => out.push_str(r"\z"),
                Some('w') => out.push_str(if class {
                    r"\p{L}\p{N}_"
                } else {
                    r"[\p{L}\p{N}_]"
                }),
                Some('W') => out.push_str(r"[^\p{L}\p{N}_]"),
                Some('s') => out.push_str(if class {
                    r"\s\x1c-\x1f"
                } else {
                    r"[\s\x1c-\x1f]"
                }),
                Some('S') => out.push_str(r"[^\s\x1c-\x1f]"),
                Some('b') if !class => out.push_str(
                    r"(?:(?<=[\p{L}\p{N}_])(?![\p{L}\p{N}_])|(?<![\p{L}\p{N}_])(?=[\p{L}\p{N}_]))",
                ),
                Some('B') if !class => out.push_str(
                    r"(?:(?<=[\p{L}\p{N}_])(?=[\p{L}\p{N}_])|(?<![\p{L}\p{N}_])(?![\p{L}\p{N}_]))",
                ),
                Some(next) => {
                    out.push('\\');
                    out.push(next);
                }
                None => out.push('\\'),
            }
        } else {
            if c == '[' {
                class = true;
            }
            if c == ']' {
                class = false;
            }
            out.push(c);
        }
    }
    out
}
pub fn compile(pattern: &str) -> Result<Arc<Regex>> {
    static CACHE: OnceLock<Mutex<BTreeMap<String, Arc<Regex>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(BTreeMap::new()));
    if let Some(regex) = cache
        .lock()
        .map_err(|_| RuntimeError::unavailable("Regex cache unavailable"))?
        .get(pattern)
        .cloned()
    {
        return Ok(regex);
    }
    let regex = Arc::new(
        Regex::new(&normalize(pattern))
            .map_err(|error| RuntimeError::invalid(format!("Invalid quality regex: {error}")))?,
    );
    let mut cache = cache
        .lock()
        .map_err(|_| RuntimeError::unavailable("Regex cache unavailable"))?;
    if cache.len() < 8192 {
        cache.insert(pattern.into(), regex.clone());
    }
    Ok(regex)
}
pub fn matching(pattern: &str, text: &str) -> Result<bool> {
    compile(pattern)?
        .is_match(text)
        .map_err(|error| RuntimeError::invalid(format!("Quality regex evaluation failed: {error}")))
}
pub fn full(pattern: &str, text: &str) -> Result<bool> {
    matching(&format!(r"\A(?:{pattern})\z"), text)
}
pub fn find(pattern: &str, text: &str) -> Result<Option<(usize, usize)>> {
    Ok(compile(pattern)?
        .find(text)
        .map_err(|error| {
            RuntimeError::invalid(format!("Quality regex evaluation failed: {error}"))
        })?
        .map(|m| (m.start(), m.end())))
}
pub fn captures(pattern: &str, text: &str) -> Result<Vec<Capture>> {
    let regex = compile(pattern)?;
    regex
        .captures_iter(text)
        .map(|capture| {
            let capture = capture.map_err(|error| {
                RuntimeError::invalid(format!("Quality regex evaluation failed: {error}"))
            })?;
            let whole = capture
                .get(0)
                .ok_or_else(|| RuntimeError::invalid("Regex omitted whole capture"))?;
            Ok(Capture {
                start: whole.start(),
                end: whole.end(),
                groups: (0..capture.len())
                    .map(|i| capture.get(i).map(|m| m.as_str().into()))
                    .collect(),
            })
        })
        .collect()
}
pub fn strings(pattern: &str, text: &str) -> Result<Vec<String>> {
    Ok(captures(pattern, text)?
        .into_iter()
        .filter_map(|capture| {
            capture
                .groups
                .into_iter()
                .nth(1)
                .flatten()
                .or_else(|| text.get(capture.start..capture.end).map(String::from))
        })
        .collect())
}
pub fn replace(pattern: &str, text: &str, replacement: &str) -> Result<String> {
    let found = captures(pattern, text)?;
    let mut output = String::new();
    let mut at = 0;
    for capture in found {
        output.push_str(&text[at..capture.start]);
        let mut chars = replacement.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '$' {
                let mut digits = String::new();
                while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                    digits.push(chars.next().unwrap_or('0'));
                }
                if !digits.is_empty() {
                    if let Ok(index) = digits.parse::<usize>()
                        && let Some(Some(value)) = capture.groups.get(index)
                    {
                        output.push_str(value);
                    }
                } else {
                    output.push(c);
                }
            } else {
                output.push(c);
            }
        }
        at = capture.end;
    }
    output.push_str(&text[at..]);
    Ok(output)
}
pub fn split(pattern: &str, text: &str) -> Result<Vec<String>> {
    let mut output = Vec::new();
    let mut at = 0;
    for capture in captures(pattern, text)? {
        output.push(text[at..capture.start].into());
        at = capture.end;
    }
    output.push(text[at..].into());
    Ok(output)
}
pub fn escape(text: &str) -> String {
    regex::escape(text)
}
