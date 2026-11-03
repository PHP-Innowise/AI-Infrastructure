use super::{consts::*, util::*};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
pub fn tokens(text: &str) -> Vec<String> {
    words(r"[a-z0-9][a-z0-9_.:/-]*", &text.to_lowercase())
        .into_iter()
        .map(|x| rstrip(&x, ".:/"))
        .filter(|x| x.len() > 1 && !STOPWORDS.contains(&x.as_str()))
        .collect()
}
pub fn meaningful(text: &str) -> BTreeSet<String> {
    tokens(text)
        .into_iter()
        .filter(|x| x.chars().count() >= 3 || x.chars().any(|c| c.is_ascii_digit()))
        .collect()
}
pub fn vocabulary(text: &str) -> BTreeSet<String> {
    let mut out = meaningful(text);
    for id in words(IDENTIFIER_RE, text) {
        let parts = words(CAMEL_RE, &id)
            .into_iter()
            .filter(|p| p.len() >= 3)
            .map(|p| p.to_lowercase())
            .collect::<Vec<_>>();
        if parts.len() > 1 {
            out.extend(parts);
        }
    }
    for id in words(DOTTED_RE, &text.to_lowercase()) {
        out.extend(
            split(DOTTED_SPLIT_RE, &id)
                .into_iter()
                .map(|p| p.to_lowercase())
                .filter(|p| p.len() >= 3),
        );
    }
    out
}
pub fn matches(value: &Value, haystack: &str) -> bool {
    let expected = meaningful(&flat(value).join(" "));
    let found = meaningful(haystack);
    !expected.is_empty()
        && expected.intersection(&found).count() >= if expected.len() < 4 { 1 } else { 2 }
}
pub fn forms(verbs: &[&str]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for &v in verbs {
        for tail in ["", "s", "ing", "ed"] {
            out.insert(format!("{v}{tail}"));
        }
        if let Some(root) = v.strip_suffix('e') {
            out.insert(format!("{root}ing"));
            out.insert(format!("{v}d"));
        }
        if ["s", "x", "z", "ch", "sh"].iter().any(|x| v.ends_with(x)) {
            out.insert(format!("{v}es"));
        }
        if v.ends_with('y')
            && v.len() > 1
            && !v.chars().rev().nth(1).is_some_and(|c| "aeiou".contains(c))
        {
            let root = &v[..v.len() - 1];
            out.insert(format!("{root}ies"));
            out.insert(format!("{root}ied"));
        }
    }
    out
}
pub fn action(text: &str, verbs: &[&str]) -> bool {
    let forms = forms(verbs);
    words(WORD_PATTERN, &text.to_lowercase())
        .iter()
        .any(|word| forms.contains(word))
}
pub fn tooling(text: &str) -> bool {
    let text = normalize_command(text);
    super::runtime::command_segments(&text)
        .into_iter()
        .any(|segment| {
            let t = segment
                .split_whitespace()
                .skip_while(|s| has(ENV_ASSIGNMENT_TOKEN, s))
                .collect::<Vec<_>>();
            let Some(head) = t.first() else { return false };
            if head.chars().next().is_some_and(char::is_uppercase) {
                return false;
            }
            let executable = head.rsplit('/').next().unwrap_or(head).to_lowercase();
            t.len() >= 2
                && crate::commands::known_executable(&executable)
                && (!AMBIGUOUS_BARE_EXECUTABLES.contains(&executable.as_str())
                    || head.contains('/'))
        })
}
pub fn commands_action(text: &str) -> bool {
    action(text, ACTION_VERBS) || tooling(text)
}
pub fn hedge(text: &str) -> Option<String> {
    let normalized = normalize_command(text).to_lowercase();
    if let Some(p) = HEDGE_PHRASES.iter().find(|p| normalized.contains(**p)) {
        return Some((*p).to_owned());
    }
    let mut openers = HEDGE_OPENERS.to_vec();
    openers.sort_by_key(|p| std::cmp::Reverse(p.len()));
    for sentence in split(SENTENCE_SPLIT_PATTERN, &normalized) {
        let opening = replace(
            LEADING_CONNECTIVE_PATTERN,
            sentence.trim_matches([' ', '\t', '"', '\'', '\u{60}', '*', '_', '-', '(', '[']),
            "",
        )
        .trim()
        .to_owned();
        let remainder = replace(SUBORDINATE_CLAUSE_PATTERN, &opening, "")
            .trim()
            .to_owned();
        let mut clauses = vec![opening.clone()];
        if !remainder.is_empty() && remainder != opening {
            clauses.push(
                replace(LEADING_CONNECTIVE_PATTERN, &remainder, "")
                    .trim()
                    .to_owned(),
            )
        }
        for clause in clauses {
            for opener in &openers {
                if has(
                    &format!(r"^{}(?:\b|$)", super::regex_adapter::escape(opener)),
                    &clause,
                ) {
                    return Some((*opener).to_owned());
                }
            }
        }
    }
    None
}
pub fn anchor(text: &str) -> bool {
    ANCHOR_PATTERNS.iter().any(|p| has(p, text))
}
pub fn excerpt(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars = collapsed.chars().collect::<Vec<_>>();
    if chars.len() <= MAX_DIAGNOSTIC_EXCERPT {
        collapsed
    } else {
        format!(
            "{}…",
            chars[..MAX_DIAGNOSTIC_EXCERPT - 1]
                .iter()
                .collect::<String>()
        )
    }
}
pub fn normalize_command(text: &str) -> String {
    replace(
        UNICODE_SPACE_PATTERN,
        &replace(ZERO_WIDTH_PATTERN, text, ""),
        " ",
    )
}
pub fn frontmatter(text: &str) -> (BTreeMap<String, String>, String) {
    let lines = py_lines(text);
    let mut m = BTreeMap::new();
    if lines.first().is_none_or(|l| l.trim() != "---") {
        return (m, text.to_owned());
    }
    let Some(end) = lines
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, l)| l.trim() == "---")
        .map(|(i, _)| i)
    else {
        return (m, text.to_owned());
    };
    let mut i = 1;
    while i < end {
        let line = lines[i];
        let cap = finds(r"^([A-Za-z][A-Za-z0-9_-]*):\s*(.*)$", line);
        if let Some(c) = cap.first() {
            let key = c.groups[1].clone().unwrap_or_default();
            let mut value = c.groups[2].clone().unwrap_or_default().trim().to_owned();
            if full(r"[>|][+-]?", &value) {
                let mut parts = Vec::new();
                while i + 1 < end
                    && (lines[i + 1].starts_with([' ', '\t']) || lines[i + 1].trim().is_empty())
                {
                    i += 1;
                    parts.push(lines[i].trim())
                }
                value = parts.join(" ");
            } else {
                value = value.trim_matches(['\'', '"']).to_owned()
            }
            m.insert(key, value);
        }
        i += 1;
    }
    (m, lines[end + 1..].join("\n"))
}
pub fn sections(body: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String, usize)> = Vec::new();
    let mut active: Vec<usize> = Vec::new();
    for line in py_lines(body).into_iter() {
        let heading = finds(r"^(#{2,6})\s+(.+?)\s*$", line);
        if let Some(c) = heading.first() {
            let level = c.groups[1].as_ref().unwrap().len();
            active.retain(|i| out[*i].2 < level);
            for i in &active {
                out[*i].1.push_str(line);
                out[*i].1.push('\n');
            }
            let title = replace(r"[*_\x60]", c.groups[2].as_ref().unwrap(), "").to_lowercase();
            let idx = if let Some(i) = out.iter().position(|x| x.0 == title) {
                i
            } else {
                out.push((title, String::new(), level));
                out.len() - 1
            };
            active.push(idx);
        } else {
            for i in &active {
                out[*i].1.push_str(line);
                out[*i].1.push('\n');
            }
        }
    }
    out.into_iter()
        .map(|(a, b, _)| (a, b.trim().to_owned()))
        .collect()
}
pub fn section(sections: &[(String, String)], aliases: &[&str]) -> String {
    for alias in aliases {
        if let Some((_, value)) = sections.iter().find(|(k, _)| k == alias) {
            return value.clone();
        }
    }
    for alias in aliases {
        let pattern = format!(r"(?:^|\W){}(?:\W|$)", super::regex_adapter::escape(alias));
        for (title, value) in sections {
            if has(&pattern, title) {
                return value.clone();
            }
        }
    }
    String::new()
}
pub fn steps(text: &str) -> Vec<String> {
    let lines = py_lines(text);
    if lines.iter().any(|l| has(HEADING_LINE_PATTERN, l)) {
        let mut steps = Vec::<Vec<String>>::new();
        for raw in lines {
            let heading = finds(HEADING_LINE_PATTERN, raw);
            if let Some(c) = heading.first() {
                steps.push(vec![
                    replace(r"[*_\x60]", c.groups[1].as_deref().unwrap_or(""), "")
                        .trim()
                        .to_owned(),
                ]);
            } else if !raw.trim().is_empty()
                && let Some(last) = steps.last_mut()
            {
                last.push(raw.trim().to_owned())
            }
        }
        return steps
            .into_iter()
            .map(|v| v.join(" ").trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect();
    }
    let mut listed = Vec::<Vec<String>>::new();
    let mut paragraphs = Vec::<Vec<String>>::new();
    let mut current = None::<(bool, usize)>;
    let mut fence = None::<(char, usize)>;
    for raw in lines {
        let stripped = raw.trim();
        if let Some((ch, len)) = fence {
            if stripped.chars().count() >= len && stripped.chars().all(|c| c == ch) {
                fence = None
            } else if let Some((is_list, index)) = current {
                if is_list {
                    listed[index].push(stripped.into())
                } else {
                    paragraphs[index].push(stripped.into())
                }
            }
            continue;
        }
        if let Some(c) = finds(FENCE_LINE_PATTERN, stripped).first() {
            let marker = c.groups[1].as_deref().unwrap_or("");
            fence = Some((
                marker.chars().next().unwrap_or('\0'),
                marker.chars().count(),
            ));
            continue;
        }
        if has(STEP_MARKER_PATTERN, raw) {
            listed.push(vec![
                replace(STEP_MARKER_PATTERN, raw, "").trim().to_owned(),
            ]);
            current = Some((true, listed.len() - 1));
            continue;
        }
        if stripped.is_empty() {
            if listed.is_empty() {
                current = None
            }
            continue;
        }
        if current.is_none() {
            paragraphs.push(Vec::new());
            current = Some((false, paragraphs.len() - 1));
        }
        let (is_list, index) = current.unwrap();
        if is_list {
            listed[index].push(stripped.into())
        } else {
            paragraphs[index].push(stripped.into())
        }
    }
    let buckets = if listed.is_empty() {
        paragraphs
    } else {
        listed
    };
    buckets
        .into_iter()
        .map(|v| v.join(" ").trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}
pub fn shell_blocks(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = None::<(char, usize, bool, Vec<String>)>;
    for line in body.lines() {
        let t = line.trim_start();
        if let Some((ch, len, keep, lines)) = &mut current {
            let count = t.chars().take_while(|c| c == ch).count();
            if count >= *len && t.chars().all(|c| c == *ch || c.is_whitespace()) {
                let (_, _, keep, lines) = current.take().unwrap();
                if keep {
                    out.push(lines.join("\n"))
                }
            } else if *keep {
                lines.push(line.to_owned())
            }
        } else {
            let c = t.chars().next().unwrap_or('\0');
            let n = t.chars().take_while(|x| *x == c).count();
            if (c == '\u{60}' || c == '~') && n >= 3 {
                let info = t[n..]
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_lowercase();
                current = Some((
                    c,
                    n,
                    SHELL_FENCE_LANGUAGES.contains(&info.as_str()),
                    Vec::new(),
                ));
            }
        }
    }
    if let Some((_, _, true, lines)) = current {
        out.push(lines.join("\n"))
    }
    out
}
pub fn shell_commands(block: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for line in py_lines(block).into_iter() {
        let l = normalize_command(line);
        let l = replace(SHELL_PROMPT_PATTERN, l.trim(), "");
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let continuation = l.ends_with('\\');
        let l = if continuation { &l[..l.len() - 1] } else { &l };
        if !current.is_empty() {
            current.push(' ')
        }
        current.push_str(l.trim());
        if !continuation {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current)
    }
    out
}
pub fn blank_code(text: &str) -> String {
    let mut out = text.to_owned();
    for c in finds(INLINE_CODE_PATTERN, text).into_iter().rev() {
        let original = &text[c.start..c.end];
        let replacement = original
            .chars()
            .map(|c| if c == '\u{60}' { c } else { ' ' })
            .collect::<String>();
        out.replace_range(c.start..c.end, &replacement)
    }
    out
}
pub fn forbids(text: &str, start: usize, len: usize, both: bool) -> bool {
    let blank = blank_code(text);
    let mut lo = 0;
    let mut hi = text.len();
    for c in finds(CLAUSE_BOUNDARY_PATTERN, &blank) {
        if c.end <= start {
            lo = c.end
        } else if both && c.start >= start + len {
            hi = c.start;
            break;
        }
    }
    let clause = blank[lo..hi.min(if both { text.len() } else { start })].to_lowercase();
    PROHIBITION_MARKERS.iter().any(|m| clause.contains(m))
}
pub fn circular(name: &str, text: &str) -> bool {
    let namepat = name
        .split('-')
        .map(super::regex_adapter::escape)
        .collect::<Vec<_>>()
        .join(r"[- ]");
    py_lines(text).into_iter().any(|line| {
        let blank = blank_code(line);
        finds(&format!(r"(?i)(?<![/\w]){namepat}(?![/\w])"), &blank).len() > 1
    })
}
