use crate::common;
use infra_core::{Result, RuntimeError, fs as safe};
use serde_json::{Value, json};
use std::path::Path;
fn percent_decode(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let nibble = |byte: u8| match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                b'A'..=b'F' => Some(byte - b'A' + 10),
                _ => None,
            };
            if let (Some(high), Some(low)) = (nibble(bytes[i + 1]), nibble(bytes[i + 2])) {
                decoded.push(high * 16 + low);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(decoded).map_err(|_| RuntimeError::invalid("Link target is not UTF-8"))
}
pub fn check(root: &Path, standalone: bool) -> Result<Value> {
    let root = std::fs::canonicalize(root)?;
    let files = if standalone {
        common::walk(&root)?
    } else {
        common::tracked(&root, &["*.md".into()])?
    };
    let pattern_path = root.join("scripts/check_links_ignore.txt");
    let ignores = if !standalone && pattern_path.is_file() {
        safe::read_text(&pattern_path)?
            .lines()
            .map(|line| line.split('#').next().unwrap_or("").trim().to_string())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let link = regex::Regex::new(r#"\[[^\]]*\]\(\s*(?:<([^>]*)>|([^()\s]+))(?:\s+"[^"]*")?\s*\)"#)
        .map_err(|_| RuntimeError::invalid("Link expression is invalid"))?;
    let inline = regex::Regex::new(r"`[^`]*`")
        .map_err(|_| RuntimeError::invalid("Inline expression is invalid"))?;
    let scheme = regex::Regex::new(r"^[a-zA-Z][a-zA-Z0-9+.-]*:")
        .map_err(|_| RuntimeError::invalid("Scheme expression is invalid"))?;
    let mut findings = Vec::new();
    let mut examined = 0usize;
    for name in files.into_iter().filter(|name| name.ends_with(".md")) {
        let path = common::under(&root, &name)?;
        if !path.is_file() {
            continue;
        }
        examined += 1;
        let bytes = safe::read_regular(&path)?;
        let text = String::from_utf8_lossy(&bytes);
        let mut fence = None::<&str>;
        for (number, line) in text.lines().enumerate() {
            let left = line.trim_start();
            if left.starts_with("```") || left.starts_with("~~~") {
                let marker = &left[..3];
                if fence == Some(marker) {
                    fence = None;
                } else if fence.is_none() {
                    fence = Some(marker);
                }
                continue;
            }
            if fence.is_some() {
                continue;
            }
            let visible = inline.replace_all(line, "");
            for capture in link.captures_iter(&visible) {
                let raw = capture
                    .get(1)
                    .or_else(|| capture.get(2))
                    .map(|value| value.as_str())
                    .unwrap_or("");
                if scheme.is_match(raw) {
                    continue;
                }
                let target = raw.split('#').next().unwrap_or("");
                if target.is_empty() {
                    continue;
                }
                let decoded = percent_decode(target)?;
                let candidate = if decoded.starts_with('/') {
                    root.join(decoded.strip_prefix('/').unwrap_or(&decoded))
                } else {
                    path.parent().unwrap_or(&root).join(decoded)
                };
                let resolved = safe::canonical_or_missing(&candidate)?;
                let inside = resolved.starts_with(&root);
                if inside && resolved.exists() {
                    continue;
                }
                let shown = resolved
                    .strip_prefix(&root)
                    .unwrap_or(&resolved)
                    .to_string_lossy();
                let mut ignored = false;
                for pattern in &ignores {
                    if common::matches(pattern, raw, false)?
                        || common::matches(pattern, &shown, false)?
                    {
                        ignored = true;
                        break;
                    }
                }
                if !ignored {
                    findings.push(json!({"path":name,"line":number+1,"target":raw,"resolved":shown,"reason":if inside{"missing"}else{"outside-root"}}));
                }
            }
        }
    }
    Ok(json!({"passed":findings.is_empty(),"files":examined,"findings":findings}))
}
