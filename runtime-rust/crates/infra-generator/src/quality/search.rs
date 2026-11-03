use super::{consts::*, prose, util::*};
use crate::common::{Diagnostic, diag, warn};
use infra_core::Result;
use serde_json::Value;
use std::{collections::BTreeSet, path::Path};
#[derive(Clone, Debug)]
pub struct SearchQuery {
    pub pattern: String,
    pub paths: Vec<String>,
    pub ignore_case: bool,
    pub word: bool,
    pub include: Vec<String>,
}
pub fn parse(command: &str, skip: &str, target: &Path) -> Option<SearchQuery> {
    if SEARCH_SHELL_METACHARACTERS
        .iter()
        .any(|c| command.contains(c))
    {
        return None;
    }
    let tokens = crate::commands::tokenize(command).ok()?;
    if tokens.len() < 3 || !SEARCH_EXECUTABLES.contains(&tokens[0].as_str()) {
        return None;
    }
    let (mut recursive, mut ignore_case, mut word, mut literal) =
        (false, false, false, tokens[0] == "fgrep");
    let mut include = Vec::new();
    let mut operands = Vec::new();
    let mut flags = true;
    for token in &tokens[1..] {
        if flags && token == "--" {
            flags = false;
            continue;
        }
        if flags && token.starts_with("--") {
            let pattern = token.strip_prefix("--include=")?;
            if pattern.is_empty() {
                return None;
            }
            include.push(pattern.to_owned());
            continue;
        }
        if flags && token.starts_with('-') && token.len() > 1 {
            for flag in token[1..].chars() {
                if !SEARCH_SHORT_FLAGS.contains(&flag.to_string().as_str()) {
                    return None;
                }
                match flag {
                    'r' | 'R' => recursive = true,
                    'i' => ignore_case = true,
                    'w' => word = true,
                    'F' => literal = true,
                    _ => {}
                }
            }
            continue;
        }
        operands.push(token.clone());
    }
    if operands.len() < 2 || operands[0].is_empty() {
        return None;
    }
    let pattern = operands.remove(0);
    if !literal
        && pattern
            .chars()
            .any(|c| SEARCH_REGEX_METACHARACTERS.contains(&c.to_string().as_str()))
    {
        return None;
    }
    let tolerant = has(SEARCH_ABSENCE_TOLERANT, skip);
    let mut out = Vec::new();
    for raw in operands {
        let value = raw.strip_prefix("./").unwrap_or(&raw);
        let path = confined(target, value)?;
        if !path.exists() {
            if tolerant { continue } else { return None }
        }
        if path.is_dir() {
            if !recursive {
                return None;
            }
        } else if !path.is_file() {
            return None;
        }
        out.push(value.to_owned());
    }
    if out.is_empty() {
        None
    } else {
        Some(SearchQuery {
            pattern,
            paths: out,
            ignore_case,
            word,
            include,
        })
    }
}
fn boundary(c: Option<char>) -> bool {
    c.is_some_and(|c| c == '_' || c.is_alphanumeric())
}
pub fn literal_hit(haystack: &str, needle: &str, word: bool) -> bool {
    if !word {
        return haystack.contains(needle);
    }
    let mut start = 0;
    while start <= haystack.len() {
        let Some(relative) = haystack[start..].find(needle) else {
            return false;
        };
        let at = start + relative;
        if !boundary(haystack[..at].chars().next_back())
            && !boundary(haystack[at + needle.len()..].chars().next())
        {
            return true;
        }
        start = at
            + haystack[at..]
                .chars()
                .next()
                .map(char::len_utf8)
                .unwrap_or(1);
    }
    false
}
pub fn resolve(target: &Path, query: &SearchQuery) -> Result<Option<(Vec<String>, Vec<String>)>> {
    fn collect_files(target: &Path, query: &SearchQuery) -> Option<BTreeSet<std::path::PathBuf>> {
        let mut files = BTreeSet::new();
        for value in &query.paths {
            let root = target.join(value);
            if !root.is_dir() {
                files.insert(root);
                continue;
            }
            let mut stack = vec![root];
            while let Some(dir) = stack.pop() {
                for entry in std::fs::read_dir(dir).ok()? {
                    let entry = entry.ok()?;
                    let ft = entry.file_type().ok()?;
                    if ft.is_symlink() {
                        continue;
                    }
                    if ft.is_dir() {
                        stack.push(entry.path())
                    } else if ft.is_file() {
                        files.insert(entry.path());
                    }
                }
                if files.len() > SEARCH_FILE_BUDGET {
                    return None;
                }
            }
        }
        Some(files)
    }
    let Some(files) = collect_files(target, query) else {
        return Ok(None);
    };
    let needle = if query.ignore_case {
        query.pattern.to_lowercase()
    } else {
        query.pattern.clone()
    };
    let mut scanned = BTreeSet::new();
    let mut matched = BTreeSet::new();
    let mut total = 0u64;
    for path in files {
        let Some(name) = path.file_name() else {
            return Ok(None);
        };
        let name = name.to_string_lossy();
        if !query.include.is_empty()
            && !query
                .include
                .iter()
                .try_fold(false, |found, pattern| -> Result<bool> {
                    Ok(found || crate::common::glob_match(pattern, &name)?)
                })?
        {
            continue;
        }
        let Ok(metadata) = path.metadata() else {
            return Ok(None);
        };
        let size = metadata.len();
        let Some(next_total) = total.checked_add(size) else {
            return Ok(None);
        };
        total = next_total;
        if size > SEARCH_FILE_BYTE_BUDGET as u64 || total > SEARCH_TOTAL_BYTE_BUDGET as u64 {
            return Ok(None);
        }
        let Ok(bytes) = std::fs::read(&path) else {
            return Ok(None);
        };
        if bytes.contains(&0) {
            continue;
        }
        let rel = relative(target, &path);
        scanned.insert(rel.clone());
        let text = String::from_utf8_lossy(&bytes);
        let text = if query.ignore_case {
            text.to_lowercase()
        } else {
            text.into_owned()
        };
        if literal_hit(&text, &needle, query.word) {
            matched.insert(rel);
        }
    }
    Ok(Some((
        scanned.into_iter().collect(),
        matched.into_iter().collect(),
    )))
}
pub fn absence(id: &str, value: &Value, target: &Path, d: &mut Vec<Diagnostic>) -> bool {
    if !exact(value, &["subject", "search", "accounted_matches"])
        || !ne(&value["subject"])
        || !ne(&value["search"])
        || !sl(&value["accounted_matches"], true)
    {
        diag(
            d,
            "EVIDENCE_ABSENCE_INVALID",
            format!("evidence {id} must state subject, search and accounted_matches"),
        );
        return false;
    }
    let Some(query) = parse(s(&value["search"]), "", target) else {
        diag(
            d,
            "EVIDENCE_ABSENCE_UNRESOLVABLE",
            format!("evidence {id} rests on an unresolvable search"),
        );
        return false;
    };
    let resolved = match resolve(target, &query) {
        Ok(value) => value,
        Err(error) => {
            diag(d, "QUALITY_GLOB_UNRESOLVED", error.to_string());
            return false;
        }
    };
    let Some((_, matched)) = resolved else {
        diag(
            d,
            "EVIDENCE_ABSENCE_UNRESOLVABLE",
            format!("evidence {id} search exceeds the scan budget or reads an unreadable file"),
        );
        return false;
    };
    let actual = matched.into_iter().collect::<BTreeSet<_>>();
    let accounted = ss(&value["accounted_matches"])
        .into_iter()
        .map(|x| x.trim().to_owned())
        .collect::<BTreeSet<_>>();
    let contradictory = actual
        .difference(&accounted)
        .take(5)
        .cloned()
        .collect::<Vec<_>>();
    if !contradictory.is_empty() {
        diag(
            d,
            "EVIDENCE_ABSENCE_CONTRADICTED",
            format!(
                "evidence {id} target contradicts absence at: {}",
                contradictory.join(", ")
            ),
        );
        return false;
    }
    let stale = accounted
        .difference(&actual)
        .take(5)
        .cloned()
        .collect::<Vec<_>>();
    if !stale.is_empty() {
        diag(
            d,
            "EVIDENCE_ABSENCE_STALE",
            format!(
                "evidence {id} accounts for vanished matches: {}",
                stale.join(", ")
            ),
        );
        return false;
    }
    true
}
fn references(expected: &str, relative: &str) -> bool {
    let basename = relative.rsplit('/').next().unwrap_or(relative);
    let stem = basename.split('.').next().unwrap_or(basename);
    if expected.contains(&relative.to_lowercase()) || expected.contains(&basename.to_lowercase()) {
        return true;
    }
    let present = |word: &str| {
        has(
            &format!(
                r"(?<![0-9A-Za-z_]){}(?![0-9A-Za-z_])",
                super::regex_adapter::escape(&word.to_lowercase())
            ),
            expected,
        )
    };
    present(stem)
        || words(r"[A-Z]+(?![a-z])|[A-Z][a-z0-9]*|[a-z0-9]+", stem)
            .iter()
            .any(|p| p.len() >= SEARCH_NAME_WORD_MIN && present(p))
}
pub fn verification(
    name: &str,
    check: &Value,
    command: &str,
    target: &Path,
    d: &mut Vec<Diagnostic>,
) {
    let Some(query) = parse(command, s(&check["skip_condition"]), target) else {
        return;
    };
    let label = format!("{name}.{}", s(&check["id"]));
    let resolved = match resolve(target, &query) {
        Ok(value) => value,
        Err(error) => {
            diag(d, "QUALITY_GLOB_UNRESOLVED", error.to_string());
            return;
        }
    };
    let Some((scanned, matched)) = resolved else {
        warn(
            d,
            "VERIFICATION_SEARCH_UNRESOLVED",
            format!("{label} search was not graded: unreadable file or offline scan budget"),
        );
        return;
    };
    let expected = s(&check["expected_result"]);
    if matched.is_empty() {
        if !has(SEARCH_EXPECTS_ABSENCE, expected) {
            diag(
                d,
                "VERIFICATION_SEARCH_DEAD",
                format!(
                    "{label} searches {} for {:?} and unchanged target answers nothing",
                    query.paths.join(", "),
                    query.pattern
                ),
            );
        }
        return;
    }
    let mut named = BTreeSet::new();
    for c in finds(r"[A-Za-z0-9_][A-Za-z0-9_./+-]*", expected) {
        let raw = &expected[c.start..c.end];
        let path = rstrip(raw, "./-");
        let ext = path
            .rsplit_once('.')
            .map(|(_, x)| x.to_lowercase())
            .unwrap_or_default();
        if !safe(&path)
            || (!path.contains('/') && !SEARCH_PATH_EXTENSIONS.contains(&ext.as_str()))
            || !scanned.contains(&path)
            || prose::forbids(expected, c.start, c.end - c.start, true)
        {
            continue;
        }
        named.insert(path);
    }
    let unreached = named
        .iter()
        .filter(|p| !matched.contains(p))
        .cloned()
        .collect::<Vec<_>>();
    if !unreached.is_empty() {
        diag(
            d,
            "VERIFICATION_SEARCH_EXPECTATION",
            format!(
                "{label} expects {} but search does not match there",
                unreached.join(", ")
            ),
        )
    }
    if matched.len() > 1 && has(SEARCH_EXCLUSIVITY_PATTERN, expected) {
        let text = expected.to_lowercase();
        let known = matched
            .iter()
            .filter(|p| references(&text, p))
            .collect::<BTreeSet<_>>();
        if !known.is_empty() {
            let surplus = matched
                .iter()
                .filter(|p| !known.contains(p))
                .cloned()
                .collect::<Vec<_>>();
            if !surplus.is_empty() {
                diag(
                    d,
                    "VERIFICATION_SEARCH_EXCLUSIVITY",
                    format!(
                        "{label} claims an exclusive result but target also answers: {}",
                        surplus.join(", ")
                    ),
                )
            }
        }
    }
}
