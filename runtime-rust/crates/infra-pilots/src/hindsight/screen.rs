use super::recall::CorePolicy;
use crate::strict;
use anyhow::{Context, Result, bail};
use fancy_regex::Regex;
use serde_json::Value;
use std::collections::BTreeSet;
pub trait PublicPolicy {
    fn contains_private_or_raw(&self, text: &str) -> Result<bool>;
    fn source_path_problem(&self, path: &str) -> bool;
}
impl PublicPolicy for CorePolicy {
    fn contains_private_or_raw(&self, text: &str) -> Result<bool> {
        Ok(infra_core::domain::sensitive_label(text)?.is_some()||regex::Regex::new(r"(?im)^\s*(?:user|assistant|system|developer|tool|prompt|response|reasoning|stdout|stderr|log)\s*:")?.is_match(text))
    }
    fn source_path_problem(&self, path: &str) -> bool {
        infra_core::domain::source_path_problem(path).is_some()
    }
}
pub fn active(a: &mut dyn FnMut() -> bool) -> Result<()> {
    if !a() {
        bail!("cancelled");
    }
    Ok(())
}
const WORDS: &str = r"(?:password|passwd|passphrase|pass|pwd|secret(?:_access)?(?:_key|_token)?|api[_-]?(?:key|token)|access[_-]?(?:token|key)|auth[_-]?token|refresh[_-]?token|private[_-]?key|account[_-]?key|shared[_-]?access[_-]?key|client[_-]?secret|encryption[_-]?key|app[_-]?key|token|salt)";
const NOT_LITERAL: &str = r#"(?![$%{<\[(=]|(?:get)?env\(|config\(|secret\(|process\.env|os\.environ|vault:|\*{2,}|x{3,}|\.{2,}|\u{2026}|\d+(?![^\s'"\x60,;])|[A-Za-z_\\][\w\\.]*(?:::|->|\()|[a-z_]\w*\.[a-z_][\w.]*(?![^\s'"\x60,;])|(?:required|nullable|sometimes|confirmed|hashed|string|null|none|true|false|secret|password|passw(?:or)?d|pass|root|test|example|placeholder|redacted|!?change[-_]?me!?|your[-_ ][^\s]*)(?![A-Za-z0-9]))"#;
const COMPOSER: &str =
    r"(?:http-basic|github-oauth|gitlab-oauth|gitlab-token|bitbucket-oauth|bearer)";
struct Span {
    start: usize,
    end: usize,
    escaped: bool,
    opaque: bool,
}
fn spans(s: &str, lenient: bool, a: &mut dyn FnMut() -> bool) -> Result<Vec<Span>> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut out = vec![];
    while i < b.len() {
        if i % 2048 == 0 {
            active(a)?;
        }
        if b[i] != b'"' {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        let (mut escaped, mut opaque, mut known, mut high) = (false, false, false, false);
        loop {
            if i >= b.len() {
                if (known || high) && !lenient {
                    bail!("source_text");
                }
                return Ok(out);
            }
            let c = b[i];
            if c == b'\n' {
                break;
            }
            if high && c != b'\\' {
                if !lenient {
                    bail!("source_text");
                }
                opaque = true;
                high = false;
            }
            if c == b'"' {
                out.push(Span {
                    start,
                    end: i + 1,
                    escaped,
                    opaque,
                });
                i += 1;
                break;
            }
            if c != b'\\' {
                i += 1;
                continue;
            }
            escaped = true;
            if i + 1 >= b.len() {
                if lenient {
                    return Ok(out);
                }
                bail!("source_text");
            }
            let code = b[i + 1];
            if high && code != b'u' {
                if !lenient {
                    bail!("source_text");
                }
                opaque = true;
                high = false;
            }
            if code == b'u' && b.get(i + 2) == Some(&b'{') && !high {
                let end = (i + 3..(i + 10).min(b.len())).find(|j| b[*j] == b'}');
                let point = end
                    .and_then(|e| std::str::from_utf8(&b[i + 3..e]).ok())
                    .filter(|s| {
                        !s.is_empty() && s.len() <= 6 && s.bytes().all(|b| b.is_ascii_hexdigit())
                    })
                    .and_then(|s| u32::from_str_radix(s, 16).ok())
                    .and_then(char::from_u32);
                if point.is_some() {
                    known = true;
                    i = end.unwrap() + 1;
                } else {
                    if !lenient {
                        bail!("source_text");
                    }
                    opaque = true;
                    i += 2;
                }
            } else if code == b'u' {
                let unit = b
                    .get(i + 2..i + 6)
                    .and_then(|v| std::str::from_utf8(v).ok())
                    .filter(|v| v.bytes().all(|c| c.is_ascii_hexdigit()))
                    .and_then(|v| u16::from_str_radix(v, 16).ok());
                if unit.is_none()
                    || high && !unit.is_some_and(|u| (0xdc00..=0xdfff).contains(&u))
                    || !high && unit.is_some_and(|u| (0xdc00..=0xdfff).contains(&u))
                {
                    if !lenient {
                        bail!("source_text");
                    }
                    opaque = true;
                    high = false;
                    i += 2;
                } else {
                    known = true;
                    high = !high && unit.is_some_and(|u| (0xd800..=0xdbff).contains(&u));
                    i += 6;
                }
            } else {
                if b"\"\\/bfnrt".contains(&code) {
                    known = true;
                } else {
                    opaque = true;
                }
                i += 2;
            }
        }
    }
    Ok(out)
}
fn json_span(s: &str) -> Result<String> {
    let b = s.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && b.get(i + 1) == Some(&b'\\') {
            out.push_str("\\\\");
            i += 2;
        } else if b[i] == b'\\' && b.get(i + 1) == Some(&b'u') && b.get(i + 2) == Some(&b'{') {
            let end = (i + 3..b.len())
                .find(|j| b[*j] == b'}')
                .context("source_text")?;
            let p = u32::from_str_radix(&s[i + 3..end], 16)?;
            let c = char::from_u32(p).context("source_text")?;
            let encoded = serde_json::to_string(&c.to_string())?;
            out.push_str(&encoded[1..encoded.len() - 1]);
            i = end + 1;
        } else {
            let c = s[i..].chars().next().unwrap();
            out.push(c);
            i += c.len_utf8();
        }
    }
    Ok(out)
}
fn parse_at(s: &str, start: usize) -> Result<(Value, usize)> {
    let mut stream = serde_json::Deserializer::from_str(&s[start..]).into_iter::<Value>();
    let value = stream.next().context("source_text")??;
    Ok((value, start + stream.byte_offset()))
}
type JsonMember = (String, usize, usize, usize, Value);
fn members(
    s: &str,
    start: usize,
    end: usize,
    a: &mut dyn FnMut() -> bool,
) -> Result<Vec<JsonMember>> {
    let b = s.as_bytes();
    let mut i = start + 1;
    let mut out = vec![];
    while i < end {
        active(a)?;
        while i < end && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= end || b[i] == b'}' {
            break;
        }
        let ks = i;
        let (k, next) = parse_at(s, i)?;
        i = next;
        while i < end && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if b.get(i) != Some(&b':') {
            bail!("source_text");
        }
        i += 1;
        while i < end && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let vs = i;
        let (v, next) = parse_at(s, i)?;
        i = next;
        out.push((k.as_str().context("source_text")?.into(), ks, vs, i, v));
        while i < end && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if b.get(i) == Some(&b',') {
            i += 1;
        } else if i >= end || b.get(i) == Some(&b'}') {
            break;
        } else {
            bail!("source_text");
        }
    }
    Ok(out)
}
fn bearer_spans(s: &str, a: &mut dyn FnMut() -> bool) -> Result<BTreeSet<(usize, usize, usize)>> {
    let Ok(root) = strict::decode(s.as_bytes(), 131072, 16, 4096) else {
        return Ok(BTreeSet::new());
    };
    if root["openapi"] != "3.1.1" {
        return Ok(BTreeSet::new());
    }
    let start = s.len() - s.trim_start().len();
    let (_, end) = parse_at(s, start)?;
    let mut out = BTreeSet::new();
    for (k, _, start, end, v) in members(s, start, end, a)? {
        if k != "components" || !v.is_object() {
            continue;
        }
        for (k, _, start, end, v) in members(s, start, end, a)? {
            if k != "securitySchemes" || !v.is_object() {
                continue;
            }
            for (name, ks, vs, ve, v) in members(s, start, end, a)? {
                let Some(m) = v.as_object() else { continue };
                let keys = m.keys().map(String::as_str).collect::<BTreeSet<_>>();
                if name.eq_ignore_ascii_case("bearer")
                    && (keys == ["type", "scheme"].into_iter().collect()
                        || keys == ["type", "scheme", "bearerFormat"].into_iter().collect())
                    && v["type"] == "http"
                    && v["scheme"] == "bearer"
                    && (v.get("bearerFormat").is_none() || v["bearerFormat"] == "JWT")
                {
                    out.insert((ks, vs, ve));
                }
            }
        }
    }
    Ok(out)
}
fn credentials(view: &str, original: bool, a: &mut dyn FnMut() -> bool) -> Result<()> {
    let key = format!(r"(?:[A-Za-z0-9_-]*?{WORDS})");
    let patterns = [
        format!(r#"(?i)(?P<quote>['"\x60]){key}\k<quote>\s*(?:=>|:|\]\s*=)\s*['"\x60]"#),
        format!(r#"(?i)\bdefine\s*\(\s*(?P<quote>['"]){key}\k<quote>\s*,\s*['"]"#),
        format!(r#"(?i)(?<![A-Za-z0-9_]){key}\s*=\s*['"\x60]"#),
        format!(
            r#"(?i)(?<![A-Za-z0-9_$])(?:[A-Za-z0-9_-]*?(?!pwd\b){WORDS})[ \t]*[:=][ \t]*{NOT_LITERAL}(?=[^\s'"\x60,;]*[^A-Za-z\s'"\x60,;]|[^\s'"\x60,;]{{8}})[^\s'"\x60,;]{{4,}}"#
        ),
        format!(
            r#"(?i)(?:\benv|getenv|environ\.get)\s*\(\s*(?P<name>['"])[A-Za-z0-9_]*?{WORDS}[A-Za-z0-9_]*\k<name>\s*,\s*(?P<value>['"])(?!\k<value>)"#
        ),
        format!(
            r#"(?i)\b(?:IDENTIFIED\s+(?:WITH\s+\w+\s+)?BY(?:\s+PASSWORD)?|PASSWORD)\s+['"]{NOT_LITERAL}[^'"\s]"#
        ),
    ];
    for p in patterns {
        active(a)?;
        if Regex::new(&p)?.is_match(view)? {
            bail!("private_content");
        }
    }
    let allowed = if original {
        bearer_spans(view, a)?
    } else {
        BTreeSet::new()
    };
    let composer = Regex::new(&format!(r#"(?i)"{COMPOSER}"\s*:\s*(?P<block>\S)"#))?;
    for found in composer.captures_iter(view) {
        active(a)?;
        let found = found?;
        let all = found.get(0).unwrap();
        let b = found.name("block").unwrap();
        let (v, end) = parse_at(view, b.start()).map_err(|_| anyhow::anyhow!("private_content"))?;
        let Some(m) = v.as_object() else {
            bail!("private_content")
        };
        if !m.is_empty() && !allowed.contains(&(all.start(), b.start(), end)) {
            bail!("private_content");
        }
    }
    Ok(())
}
pub fn screen<P: PublicPolicy>(
    text: &str,
    path: &str,
    policy: &P,
    a: &mut dyn FnMut() -> bool,
) -> Result<()> {
    let lenient = ["php", "md", "txt", "sql"].contains(&path.rsplit('.').next().unwrap_or(""));
    let sensitive = Regex::new(&format!(r"(?i)^(?:[A-Za-z0-9_-]*?{WORDS}|{COMPOSER})$"))?;
    let mut pending = vec![(text.to_owned(), 0)];
    let mut views = 0;
    let mut total = 0;
    while let Some((view, depth)) = pending.pop() {
        active(a)?;
        let view = view.replace("\r\n", "\n").replace('\r', "\n");
        views += 1;
        total += view.len();
        if views > 4096 || depth > 16 || total > 8 * 131072 {
            bail!("source_text");
        }
        if policy.contains_private_or_raw(&view)? {
            bail!("private_content");
        }
        credentials(&view, depth == 0, a)?;
        let mut normalized = String::new();
        let mut end = 0;
        let mut changed = false;
        for span in spans(&view, lenient && depth == 0, a)? {
            if !span.escaped || span.opaque {
                continue;
            }
            let decoded: String = serde_json::from_str(&json_span(&view[span.start..span.end])?)?;
            if decoded != view[span.start + 1..span.end - 1] {
                if pending.len() + views >= 4096 {
                    bail!("source_text");
                }
                pending.push((decoded.clone(), depth + 1));
            }
            if sensitive.is_match(&decoded)? {
                normalized.push_str(&view[end..span.start]);
                normalized.push_str(&serde_json::to_string(&decoded)?);
                end = span.end;
                changed = true;
            }
        }
        if changed {
            normalized.push_str(&view[end..]);
            credentials(&normalized, depth == 0, a)?;
        } else {
            credentials(&view, depth == 0, a)?;
        }
    }
    active(a)
}
pub fn source_path<P: PublicPolicy>(
    path: &str,
    policy: &P,
    a: &mut dyn FnMut() -> bool,
) -> Result<()> {
    active(a)?;
    strict::path(path, 512)?;
    let denied = [
        "task",
        "tasks",
        "memory-bank",
        "project-brain",
        "private",
        "restricted",
        "local",
        "logs",
        "log",
        "artifacts",
        "archive",
        "tmp",
        "temp",
    ];
    if path.contains('#')
        || path
            .split('/')
            .any(|p| p.starts_with('.') || denied.contains(&p.to_lowercase().as_str()))
        || ![
            "md", "txt", "py", "php", "ts", "js", "json", "yaml", "yml", "toml", "sql",
        ]
        .contains(
            &path
                .rsplit('.')
                .next()
                .unwrap_or("")
                .to_lowercase()
                .as_str(),
        )
        || policy.source_path_problem(path)
    {
        bail!("source_path");
    }
    screen(path, path, policy, a)
}
