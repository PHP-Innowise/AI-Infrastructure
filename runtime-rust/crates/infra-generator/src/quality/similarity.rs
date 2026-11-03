use super::{consts::*, prose, util::*};
use crate::common::{Diagnostic, diag, warn};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
pub fn normalize_line(line: &str) -> String {
    let x = replace(r"(?i)TASK-\d+", line, "task-id");
    let x = replace(r"(?i)\bprofile[-_][a-z0-9_-]+\b", &x, "profile-id");
    x.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn body(body: &str, fixed: &[String]) -> (Vec<String>, Vec<String>) {
    let mut body = body.to_owned();
    let mut blocks = fixed.to_vec();
    blocks.sort_by(|a, b| (b.len(), b).cmp(&(a.len(), a)));
    for block in blocks {
        body = body.replace(&block, "")
    }
    let lines = body
        .lines()
        .filter(|l| !l.trim().is_empty() && !has(r"^#{1,6}\s+", l))
        .map(normalize_line)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>();
    let words = prose::tokens(&lines.join("\n"));
    (lines, words)
}
pub fn table_cells(line: &str) -> Vec<String> {
    let x = line.trim();
    if !x.starts_with('|') || x.matches('|').count() < 2 {
        return Vec::new();
    }
    x.trim_matches('|')
        .split('|')
        .map(|x| x.trim().to_owned())
        .collect()
}
pub fn evidence_row(line: &str) -> bool {
    let cells = table_cells(line);
    let id = cells
        .iter()
        .any(|s| full(EVIDENCE_ID_CELL, s.trim_matches(['\u{60}', '*', '_', ' '])));
    id && cells.iter().any(|s| has(PATH_PATTERN, s))
}
pub fn table_rule(line: &str) -> bool {
    let c = table_cells(line);
    !c.is_empty() && c.iter().all(|x| full(TABLE_RULE_CELL, x))
}
pub fn repeatable(lines: &[String]) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut current = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let header =
            !table_cells(line).is_empty() && lines.get(index + 1).is_some_and(|l| table_rule(l));
        if header || table_rule(line) || evidence_row(line) {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current))
            }
        } else {
            current.push(line.clone())
        }
    }
    if !current.is_empty() {
        out.push(current)
    }
    out
}
pub fn line_score(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut counts = BTreeMap::new();
    for x in a {
        *counts.entry(x).or_insert(0usize) += 1
    }
    let mut overlap = 0;
    for x in b {
        if let Some(n) = counts.get_mut(x)
            && *n > 0
        {
            overlap += 1;
            *n -= 1
        }
    }
    overlap as f64 / a.len().min(b.len()) as f64
}
pub fn ngrams(v: &[String]) -> BTreeSet<Vec<String>> {
    if v.len() < 3 {
        v.iter().map(|x| vec![x.clone()]).collect()
    } else {
        v.windows(3).map(<[String]>::to_vec).collect()
    }
}
pub fn token_score(a: &[String], b: &[String]) -> f64 {
    let a = ngrams(a);
    let b = ngrams(b);
    let union = a.union(&b).count();
    if union == 0 {
        0.0
    } else {
        a.intersection(&b).count() as f64 / union as f64
    }
}
pub fn skeleton_line(line: &str, entities: &BTreeSet<String>) -> String {
    let mut x = line.to_owned();
    for pattern in [
        CODE_SPAN_PATTERN,
        PHP_VARIABLE_PATTERN,
        CALL_PATTERN,
        PATH_PATTERN,
        UPPER_SNAKE_PATTERN,
        CAMEL_CASE_PATTERN,
        DOTTED_ID_PATTERN,
        NUMBER_PATTERN,
    ] {
        x = replace(pattern, &x, SKELETON_SUB)
    }
    let mut names = entities.iter().filter(|x| x.len() >= 3).collect::<Vec<_>>();
    names.sort_by(|a, b| (b.len(), b).cmp(&(a.len(), a)));
    for name in names {
        x = replace(
            &format!(r"(?i)\b{}\b", super::regex_adapter::escape(name)),
            &x,
            SKELETON_SUB,
        )
    }
    x = replace(NON_WORD_PATTERN, &x.to_lowercase(), " ");
    let mut words = x.split_whitespace().map(str::to_owned).collect::<Vec<_>>();
    for word in &mut words {
        if let Some((_, replacement)) = SKELETON_CONNECTIVES
            .iter()
            .find(|(key, _)| *key == word.as_str())
        {
            *word = (*replacement).into()
        }
    }
    words.join(" ")
}
pub fn skeleton(body: &str, fixed: &[String], entities: &BTreeSet<String>) -> Vec<String> {
    let mut text = body.to_owned();
    let mut blocks = fixed.to_vec();
    blocks.sort_by(|a, b| (b.len(), b).cmp(&(a.len(), a)));
    for block in blocks {
        text = text.replace(&block, "")
    }
    let mut fenced = false;
    let mut out = Vec::new();
    for line in py_lines(&text).into_iter() {
        if line.trim_start().starts_with("\u{60}\u{60}\u{60}")
            || line.trim_start().starts_with("~~~")
        {
            fenced = !fenced;
            continue;
        }
        if fenced || line.trim().is_empty() || has(r"^#{1,6}\s+", line) {
            continue;
        }
        let x = skeleton_line(line, entities);
        let count = prose::tokens(&x)
            .into_iter()
            .filter(|w| w != SKELETON_PLACEHOLDER && !STOPWORDS.contains(&w.as_str()))
            .count();
        if count >= SKELETON_MIN_RESIDUAL {
            out.push(x)
        }
    }
    out
}
fn bag(line: &str) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for word in line.split_whitespace() {
        *m.entry(word.to_owned()).or_insert(0) += 1
    }
    m
}
fn dice(a: &BTreeMap<String, usize>, b: &BTreeMap<String, usize>) -> f64 {
    let total = a.values().sum::<usize>() + b.values().sum::<usize>();
    if total == 0 {
        0.0
    } else {
        2.0 * a
            .iter()
            .map(|(w, n)| n.min(b.get(w).unwrap_or(&0)))
            .sum::<usize>() as f64
            / total as f64
    }
}
pub fn skeleton_score(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let bags = b.iter().map(|x| bag(x)).collect::<Vec<_>>();
    let mut index: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    for (i, line) in bags.iter().enumerate() {
        for word in line.keys() {
            if word != SKELETON_PLACEHOLDER && !STOPWORDS.contains(&word.as_str()) && word.len() > 1
            {
                index.entry(word.clone()).or_default().insert(i);
            }
        }
    }
    let mut used = BTreeSet::new();
    let mut found = 0;
    for line in a {
        let ab = bag(line);
        let candidates = ab
            .keys()
            .filter_map(|w| index.get(w))
            .flatten()
            .copied()
            .collect::<BTreeSet<_>>();
        let mut best = None;
        let mut score = 0.0;
        for i in candidates {
            if used.contains(&i) {
                continue;
            }
            let value = dice(&ab, &bags[i]);
            if value > score {
                score = value;
                best = Some(i)
            }
        }
        if score >= SKELETON_LINE_MATCH
            && let Some(i) = best
        {
            used.insert(i);
            found += 1
        }
    }
    found as f64 / a.len().min(b.len()) as f64
}
pub fn compare_skeletons(an: &str, a: &[String], bn: &str, b: &[String], d: &mut Vec<Diagnostic>) {
    if a.len() < SKELETON_MIN_LINES || b.len() < SKELETON_MIN_LINES {
        return;
    }
    let line = skeleton_score(a, b);
    let token = token_score(&prose::tokens(&a.join("\n")), &prose::tokens(&b.join("\n")));
    let message =
        format!("{an} and {bn} share a prose skeleton (line={line:.3}, token={token:.3})");
    if line >= SKELETON_LINE_FAIL || token >= SKELETON_TOKEN_FAIL {
        let shared = a
            .iter()
            .filter(|x| b.contains(x))
            .take(2)
            .cloned()
            .collect::<Vec<_>>();
        diag(
            d,
            "SKILL_TEMPLATE_REUSE",
            if shared.is_empty() {
                message
            } else {
                format!("{message}; repeated: {}", shared.join(" | "))
            },
        )
    } else if line >= SKELETON_LINE_WARN || token >= SKELETON_TOKEN_WARN {
        warn(d, "SKILL_TEMPLATE_REUSE_WARN", message)
    }
}
pub fn repeated(groups: &BTreeMap<String, Vec<String>>, skeleton: bool, d: &mut Vec<Diagnostic>) {
    let mut owners: BTreeMap<Vec<String>, BTreeSet<String>> = BTreeMap::new();
    for (name, lines) in groups {
        let segments = if skeleton {
            vec![lines.clone()]
        } else {
            repeatable(lines)
        };
        for segment in segments {
            for size in if skeleton { SKELETON_BLOCK_SIZE } else { 3 }..7.min(segment.len() + 1) {
                for block in segment.windows(size) {
                    if block.iter().map(|s| prose::tokens(s).len()).sum::<usize>() >= 20 {
                        owners
                            .entry(block.to_vec())
                            .or_default()
                            .insert(name.clone());
                    }
                }
            }
        }
    }
    for (block, names) in &owners {
        if names.len() > 1
            && block.len()
                == owners
                    .iter()
                    .filter(|(_, n)| *n == names)
                    .map(|(b, _)| b.len())
                    .max()
                    .unwrap_or(0)
        {
            let message = format!(
                "repeated {} across {}: {}",
                if skeleton {
                    "prose skeleton"
                } else {
                    "substantive block"
                },
                names.iter().cloned().collect::<Vec<_>>().join(", "),
                block
                    .iter()
                    .take(2)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" | ")
            );
            if skeleton {
                warn(d, "SKILL_TEMPLATE_BLOCK", message)
            } else {
                diag(d, "REPEATED_BLOCK", message)
            }
        }
    }
}
pub fn projection(skill: &Value) -> BTreeMap<String, (Vec<String>, Vec<String>)> {
    let mut out = BTreeMap::new();
    for &field in CONTRACT_PROJECTION_FIELDS {
        let strings = if field == "triggers" {
            [
                flat(&skill["triggers"]["positive"]),
                flat(&skill["triggers"]["negative"]),
            ]
            .concat()
        } else {
            flat(&skill[field])
        };
        let lines = strings
            .iter()
            .map(|l| normalize_line(l))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        let tokens = prose::tokens(&lines.join("\n"));
        out.insert(field.to_owned(), (lines, tokens));
    }
    for (field, key, member) in [
        ("ownership", "_normalized_ownership", "description"),
        (
            "nearest_siblings.boundary",
            "_normalized_siblings",
            "boundary",
        ),
    ] {
        let source = skill.get(key).unwrap_or(
            &skill[if field == "ownership" {
                "ownership"
            } else {
                "nearest_siblings"
            }],
        );
        let lines = arr(source)
            .iter()
            .flat_map(|i| flat(&i[member]))
            .map(|l| normalize_line(&l))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        let tokens = prose::tokens(&lines.join("\n"));
        out.insert(field.to_owned(), (lines, tokens));
    }
    out
}
