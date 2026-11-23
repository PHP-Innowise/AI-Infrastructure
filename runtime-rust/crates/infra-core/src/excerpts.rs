//! Quotes are selected from held indexed images, using SQLite's own marks.
use crate::{
    Result,
    capsule::Capsule,
    query::{self, Candidate},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const PREPARSED_SECTION_VERSION: u32 = 1;
const MARK_SEPARATOR: &str = "\n\u{1f}\n";

#[cfg(test)]
thread_local! {
    static LIVE_SECTION_PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static PREPARSED_CACHE_HITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
fn bump_live_section_parses() {
    LIVE_SECTION_PARSES.with(|cell| cell.set(cell.get() + 1));
}
#[cfg(test)]
fn bump_preparsed_cache_hits() {
    PREPARSED_CACHE_HITS.with(|cell| cell.set(cell.get() + 1));
}
pub const OPEN: char = '\u{2}';
pub const CLOSE: char = '\u{3}';
pub const MARKED_LIMIT: usize = 65536;
#[derive(Clone, Debug)]
pub struct IndexedImage {
    pub title: String,
    pub summary: String,
    pub content: String,
}
#[derive(Clone, Debug)]
pub struct Section {
    pub heading: String,
    pub slug: String,
    pub units: Vec<String>,
    pub hash: String,
    pub shown: String,
}
#[derive(Clone, Debug)]
pub struct Excerpt {
    pub text: String,
    pub heading: String,
    pub limit: usize,
    pub cut: bool,
}
#[derive(Clone, Debug, Default)]
pub struct PreparedRender {
    pub images: BTreeMap<String, IndexedImage>,
    pub excerpts: BTreeMap<String, Excerpt>,
}
/// A same-thread optional selector can choose only a held section of an
/// already admitted document. Index zero is always the lexical baseline.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreparedAlternative {
    pub text: String,
    pub heading: String,
    pub key: String,
    pub revision: String,
    pub limit: usize,
    pub cut: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreparationCandidate {
    pub path: String,
    pub alternatives: Vec<PreparedAlternative>,
}
pub trait ExcerptPreparer {
    fn select(
        &mut self,
        query: &str,
        candidates: &[PreparationCandidate],
    ) -> Result<BTreeMap<String, usize>>;
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RenderRecord {
    pub path: String,
    pub header_present: bool,
    pub body_bytes: usize,
    pub body_sha256: String,
    pub body_reason: String,
    pub pointer_only: bool,
    pub shrunk: bool,
    pub initial_excerpt_bytes: usize,
}
fn collapse(text: &str) -> String {
    crate::results::collapse(text)
}
pub fn unmarked(text: &str) -> String {
    text.chars()
        .filter(|character| *character != OPEN && *character != CLOSE)
        .collect()
}
fn marked_words(text: &str) -> Vec<String> {
    static MARKED: std::sync::OnceLock<std::result::Result<regex::Regex, regex::Error>> =
        std::sync::OnceLock::new();
    MARKED
        .get_or_init(|| regex::Regex::new("(?s)\u{2}(.*?)\u{3}"))
        .as_ref()
        .map(|pattern| {
            pattern
                .captures_iter(text)
                .map(|captures| captures[1].to_string())
                .collect()
        })
        .unwrap_or_default()
}
fn heading(line: &str) -> Option<(usize, String)> {
    let count = line.bytes().take_while(|byte| *byte == b'#').count();
    if (1..=6).contains(&count)
        && line[count..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
    {
        Some((count, line[count..].trim().into()))
    } else {
        None
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CitationLineRef {
    pub raw: String,
    pub base: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparsedSection {
    pub heading: String,
    pub folded_heading: String,
    pub slug: String,
    pub shown: String,
    pub units: Vec<String>,
    pub hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation_refs: Option<Vec<CitationLineRef>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparsedFirstBody {
    pub heading: String,
    pub slug: String,
    pub shown: String,
    pub units: Vec<String>,
    pub hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparsedLinkedTask {
    pub goal: String,
    pub files_text: String,
    pub sources_text: String,
    pub files: Vec<String>,
    pub sources: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparsedDocument {
    pub version: u32,
    pub title_folded: String,
    pub sections: Vec<PreparsedSection>,
    pub first_body: Option<PreparsedFirstBody>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_task: Option<PreparsedLinkedTask>,
    pub section_digest: String,
}

fn sections_raw(text: &str) -> Vec<(String, Vec<String>)> {
    let body = text
        .strip_prefix("---\n")
        .and_then(|tail| tail.split_once("\n---\n").map(|(_, body)| body))
        .unwrap_or(text);
    let mut output = vec![(String::new(), Vec::new())];
    for line in body.lines() {
        if let Some((_, heading)) = heading(line) {
            output.push((heading, vec![]));
        } else if let Some((_, lines)) = output.last_mut() {
            lines.push(line.into());
        }
    }
    output
}

pub fn sections(text: &str) -> Vec<(String, Vec<String>)> {
    #[cfg(test)]
    bump_live_section_parses();
    sections_raw(text)
}
pub fn units(lines: &[String]) -> Vec<String> {
    fn flush(block: &mut Vec<String>, output: &mut Vec<String>) {
        if block.is_empty() {
            return;
        }
        let text = collapse(&block.join(" "));
        block.clear();
        let characters = text.chars().collect::<Vec<_>>();
        let mut start = 0;
        for index in 1..characters.len() {
            if characters[index].is_whitespace() && ".!?".contains(characters[index - 1]) {
                let piece = characters[start..index].iter().collect::<String>();
                if !piece.is_empty() {
                    output.push(piece);
                }
                start = index + 1;
            }
        }
        let piece = characters[start..].iter().collect::<String>();
        if !piece.is_empty() {
            output.push(piece);
        }
    }
    static LIST: std::sync::OnceLock<std::result::Result<regex::Regex, regex::Error>> =
        std::sync::OnceLock::new();
    let list = LIST
        .get_or_init(|| regex::Regex::new(r"^(?:[-*+]|[0-9]+[.)])\s|^\|"))
        .as_ref()
        .ok();
    let mut output = Vec::new();
    let mut block = Vec::new();
    for line in lines {
        let stripped = line.trim();
        if stripped.is_empty() {
            flush(&mut block, &mut output);
        } else if list
            .as_ref()
            .is_some_and(|pattern| pattern.is_match(stripped))
        {
            flush(&mut block, &mut output);
            block.push(stripped.into());
            if stripped.starts_with('|') {
                flush(&mut block, &mut output);
            }
        } else {
            block.push(stripped.into());
        }
    }
    flush(&mut block, &mut output);
    output
}
pub fn weights(conn: &Connection, query: &str) -> BTreeMap<String, f64> {
    let operation = (|| -> Result<BTreeMap<String, f64>> {
        let tokens = query::evidence_tokens(&query::informative_tokens(
            conn,
            query::expand_query_tokens(conn, query::query_tokens(query)?)?,
        )?);
        let total = conn
            .query_row("SELECT COUNT(*) FROM documents", [], |row| {
                row.get::<_, usize>(0)
            })?
            .max(1);
        let frequencies = query::frequencies(conn, &tokens)?;
        let forms = query::term_forms(&tokens);
        let mut weights = BTreeMap::<String, f64>::new();
        for token in tokens {
            let frequency = frequencies
                .get(&token)
                .copied()
                .filter(|count| *count > 0)
                .unwrap_or(1);
            let weight = ((total as f64 + 1.0) / (frequency as f64 + 0.5))
                .ln()
                .max(0.05);
            for stem in forms
                .get(&token)
                .into_iter()
                .flat_map(|form| form.split_whitespace())
            {
                let previous = weights.entry(stem.into()).or_default();
                *previous = previous.max(weight);
            }
        }
        Ok(weights)
    })();
    operation.unwrap_or_default()
}
fn score_terms(found: &[String], weights: &BTreeMap<String, f64>) -> (i64, usize) {
    let terms = found.iter().collect::<BTreeSet<_>>();
    let floor = weights.values().copied().reduce(f64::min).unwrap_or(1.0);
    let value = terms
        .iter()
        .map(|term| weights.get(*term).copied().unwrap_or(floor))
        .sum::<f64>();
    ((value * 1_000_000.0).round() as i64, found.len())
}
pub fn score(units: &[String], weights: &BTreeMap<String, f64>) -> (i64, usize) {
    let found = units
        .iter()
        .flat_map(|unit| marked_words(unit))
        .collect::<Vec<_>>();
    let forms = query::term_forms(&found);
    let stems = found
        .iter()
        .flat_map(|word| {
            forms
                .get(word)
                .into_iter()
                .flat_map(|form| form.split_whitespace().map(String::from))
        })
        .collect::<Vec<_>>();
    score_terms(&stems, weights)
}
pub fn slug(heading: &str) -> String {
    static PLAIN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static GAPS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let folded = query::folded(heading);
    let plain = PLAIN
        .get_or_init(|| regex::Regex::new(r"[^\p{L}\p{N}_\s-]").expect("constant slug grammar"))
        .replace_all(&folded, "")
        .into_owned();
    let collapsed = GAPS
        .get_or_init(|| regex::Regex::new(r"[\s_]+").expect("constant slug grammar"))
        .replace_all(&plain, "-")
        .into_owned();
    query::chars(collapsed.trim_matches('-'), 60)
}
fn payload(heading: &str, lines: &[String], title: &str) -> Section {
    let units = units(lines);
    let plain = collapse(&unmarked(heading));
    let mut image = vec![plain.clone()];
    image.extend(units.iter().map(|unit| unmarked(unit)));
    Section {
        slug: slug(&plain),
        hash: query::sha256(image.join("\n").as_bytes())[..16].into(),
        shown: if query::folded(&plain) == query::folded(title) {
            String::new()
        } else {
            plain.clone()
        },
        heading: plain,
        units,
    }
}
fn first_body_raw(text: &str) -> Option<(String, Vec<String>)> {
    let body = text
        .strip_prefix("---\n")
        .and_then(|tail| tail.split_once("\n---\n").map(|(_, body)| body))
        .unwrap_or(text);
    let mut title = String::new();
    let mut preamble = Vec::new();
    let mut trail = Vec::<(usize, String)>::new();
    let mut lines = None::<Vec<String>>;
    for line in body.lines() {
        if let Some((level, name)) = heading(line) {
            if let Some(lines) = &lines
                && !units(lines).is_empty()
            {
                return Some((
                    trail
                        .iter()
                        .map(|(_, name)| name.as_str())
                        .collect::<Vec<_>>()
                        .join(" › "),
                    lines.clone(),
                ));
            }
            if lines.is_none()
                && level == 1
                && title.is_empty()
                && !query::contains_decimal_digit(&name)
            {
                title = name;
                continue;
            }
            while trail.last().is_some_and(|(previous, _)| *previous >= level) {
                trail.pop();
            }
            trail.push((level, name));
            lines = Some(vec![]);
        } else if let Some(lines) = &mut lines {
            lines.push(line.into());
        } else {
            preamble.push(line.into());
        }
    }
    if let Some(lines) = lines
        && !units(&lines).is_empty()
    {
        return Some((
            trail
                .iter()
                .map(|(_, name)| name.as_str())
                .collect::<Vec<_>>()
                .join(" › "),
            lines,
        ));
    }
    if !units(&preamble).is_empty() {
        Some((title, preamble))
    } else {
        None
    }
}
fn first_body(text: &str) -> Option<(String, Vec<String>)> {
    #[cfg(test)]
    bump_live_section_parses();
    first_body_raw(text)
}
fn extract_citation_refs(heading: &str, lines: &[String]) -> Option<Vec<CitationLineRef>> {
    if !["sources", "references"].contains(&query::folded(&unmarked(heading)).as_str()) {
        return None;
    }
    let mut refs = Vec::new();
    for line in lines.iter().filter(|line| !line.trim().is_empty()) {
        let plain = unmarked(line);
        let line = plain.trim();
        if !line.starts_with(['-', '*', '+'])
            || !line.chars().nth(1).is_some_and(char::is_whitespace)
        {
            return None;
        }
        let reference = line[1..].trim().trim_matches('`');
        let base = reference
            .split_once('#')
            .filter(|(_, fragment)| {
                !fragment.is_empty()
                    && !fragment.chars().any(char::is_whitespace)
                    && !fragment.contains('`')
            })
            .map(|(base, _)| base.to_string());
        refs.push(CitationLineRef {
            raw: reference.to_string(),
            base,
        });
    }
    (!refs.is_empty()).then_some(refs)
}
fn citation_refs_only(refs: Option<&[CitationLineRef]>, sources: &BTreeSet<String>) -> bool {
    let Some(refs) = refs else {
        return false;
    };
    !refs.is_empty()
        && refs.iter().all(|item| {
            sources.contains(&item.raw)
                || item
                    .base
                    .as_ref()
                    .is_some_and(|base| sources.contains(base))
        })
}
fn compute_section_digest(
    sections: &[PreparsedSection],
    first_body: &Option<PreparsedFirstBody>,
) -> String {
    serde_json::to_vec(&(PREPARSED_SECTION_VERSION, sections, first_body))
        .map(|bytes| query::sha256(&bytes))
        .unwrap_or_default()
}
fn linked_task_entries_pattern() -> Option<&'static regex::Regex> {
    static ENTRIES: std::sync::OnceLock<std::result::Result<regex::Regex, regex::Error>> =
        std::sync::OnceLock::new();
    ENTRIES
        .get_or_init(|| {
            let grave = char::from(96);
            regex::Regex::new(&format!("^- {grave}?([^{grave}\\n]+?){grave}?\\s*$"))
        })
        .as_ref()
        .ok()
}
fn preparse_linked_task_raw(content: &str) -> PreparsedLinkedTask {
    let mut sections = BTreeMap::<String, String>::new();
    let mut active = None::<String>;
    let mut lines = Vec::<String>::new();
    for line in content.lines() {
        if let Some(title) = line.strip_prefix("## ") {
            if let Some(previous) = active.take() {
                sections.insert(previous, lines.join("\n").trim().into());
                lines.clear();
            }
            active = Some(title.trim().into());
        } else if active.is_some() {
            lines.push(line.into());
        }
    }
    if let Some(previous) = active {
        sections.insert(previous, lines.join("\n").trim().into());
    }
    let goal = sections
        .get("Goal")
        .and_then(|goal| goal.split("\n\n").find(|part| !part.trim().is_empty()))
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    let files_text = sections.get("Files").cloned().unwrap_or_default();
    let sources_text = sections.get("Sources").cloned().unwrap_or_default();
    let pattern = linked_task_entries_pattern();
    let collect = |text: &str| {
        text.lines()
            .filter_map(|line| {
                pattern
                    .as_ref()
                    .and_then(|pattern| pattern.captures(line.trim()))
                    .map(|captures| captures[1].trim().to_string())
            })
            .collect::<Vec<_>>()
    };
    PreparsedLinkedTask {
        goal,
        files: collect(&files_text),
        sources: collect(&sources_text),
        files_text,
        sources_text,
    }
}
fn linked_task_excerpt_from_preparsed(
    parsed: &PreparsedLinkedTask,
    via: &str,
    limit: usize,
) -> (String, String) {
    let heading = if !via.is_empty()
        && !parsed.sources_text.is_empty()
        && parsed.sources_text.contains(via)
        && !parsed.files_text.contains(via)
    {
        "Sources"
    } else {
        "Files"
    };
    let entries = if heading == "Sources" {
        &parsed.sources
    } else {
        &parsed.files
    };
    let mut ordered = Vec::new();
    if entries.iter().any(|entry| entry == via) {
        ordered.push(via.to_string());
    }
    ordered.extend(entries.iter().rev().filter(|entry| *entry != via).cloned());
    let mut parts = Vec::new();
    if !parsed.goal.is_empty() {
        parts.push(format!("Goal: {}.", collapse(&parsed.goal)));
    }
    if !ordered.is_empty() {
        parts.push(format!(
            "{heading}: {}",
            ordered.into_iter().take(6).collect::<Vec<_>>().join(", ")
        ));
    }
    (
        crate::capsule::bounded(&parts.join(" "), limit),
        heading.into(),
    )
}
pub(crate) fn preparse_document(title: &str, kind: &str, content: &str) -> PreparsedDocument {
    let raw_sections = sections_raw(content);
    let mut sections = Vec::with_capacity(raw_sections.len());
    for (heading, lines) in raw_sections {
        let citation_refs = extract_citation_refs(&heading, &lines);
        let parsed = payload(&heading, &lines, title);
        let folded_heading = query::folded(&parsed.heading);
        sections.push(PreparsedSection {
            heading: parsed.heading,
            folded_heading,
            slug: parsed.slug,
            shown: parsed.shown,
            units: parsed.units,
            hash: parsed.hash,
            citation_refs,
        });
    }
    let first_body = first_body_raw(content).map(|(heading, lines)| {
        let parsed = payload(&heading, &lines, title);
        PreparsedFirstBody {
            heading: parsed.heading,
            slug: parsed.slug,
            shown: parsed.shown,
            units: parsed.units,
            hash: parsed.hash,
        }
    });
    let linked_task = (kind == "brain-task").then(|| preparse_linked_task_raw(content));
    let section_digest = compute_section_digest(&sections, &first_body);
    PreparsedDocument {
        version: PREPARSED_SECTION_VERSION,
        title_folded: query::folded(title),
        sections,
        first_body,
        linked_task,
        section_digest,
    }
}
fn load_preparsed(
    conn: &Connection,
    item: &Candidate,
    image: &IndexedImage,
) -> Option<PreparsedDocument> {
    if image.content.chars().count() > MARKED_LIMIT {
        return None;
    }
    let expected_source_hash = item
        .held_source_hash
        .as_deref()
        .unwrap_or(&item.document.source_hash);
    let row: Option<(String, String, String)> = conn
        .query_row(
            "SELECT source_hash, content_sha256, payload FROM document_sections WHERE path=?1",
            [&item.document.path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .ok()
        .flatten();
    let (stored_source_hash, stored_content_hash, raw) = row?;
    if stored_source_hash != expected_source_hash {
        return None;
    }
    if stored_content_hash != query::sha256(image.content.as_bytes()) {
        return None;
    }
    let parsed: PreparsedDocument = serde_json::from_str(&raw).ok()?;
    if parsed.version != PREPARSED_SECTION_VERSION
        || parsed.title_folded != query::folded(&image.title)
        || parsed.section_digest != compute_section_digest(&parsed.sections, &parsed.first_body)
    {
        return None;
    }
    #[cfg(test)]
    bump_preparsed_cache_hits();
    Some(parsed)
}
struct MarkedPreparsedSection<'a> {
    meta: &'a PreparsedSection,
    marked_heading: String,
    marked_units: Vec<String>,
}
fn mark_preparsed_sections<'a>(
    preparsed: &'a PreparsedDocument,
    tokens: &[String],
) -> Option<Vec<MarkedPreparsedSection<'a>>> {
    if tokens.is_empty() {
        return Some(
            preparsed
                .sections
                .iter()
                .map(|meta| MarkedPreparsedSection {
                    meta,
                    marked_heading: meta.heading.clone(),
                    marked_units: meta.units.clone(),
                })
                .collect(),
        );
    }
    let mut parts = Vec::new();
    for sec in &preparsed.sections {
        if sec.units.is_empty() {
            continue;
        }
        if sec.heading.contains('\u{1f}') || sec.units.iter().any(|u| u.contains('\u{1f}')) {
            return None;
        }
        parts.push(sec.heading.as_str());
        for unit in &sec.units {
            parts.push(unit.as_str());
        }
    }
    if parts.is_empty() {
        return Some(Vec::new());
    }
    let joined = parts.join(MARK_SEPARATOR);
    let highlighted = held_marked(
        &IndexedImage {
            title: String::new(),
            summary: String::new(),
            content: joined,
        },
        tokens,
    )
    .ok()?;
    let split: Vec<&str> = highlighted.split(MARK_SEPARATOR).collect();
    if split.len() != parts.len() {
        return None;
    }
    let mut cursor = 0usize;
    let mut out = Vec::with_capacity(preparsed.sections.len());
    for meta in &preparsed.sections {
        if meta.units.is_empty() {
            out.push(MarkedPreparsedSection {
                meta,
                marked_heading: meta.heading.clone(),
                marked_units: Vec::new(),
            });
            continue;
        }
        let marked_heading = split[cursor].to_string();
        cursor += 1;
        let end = cursor + meta.units.len();
        let marked_units = split[cursor..end]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        cursor = end;
        out.push(MarkedPreparsedSection {
            meta,
            marked_heading,
            marked_units,
        });
    }
    Some(out)
}
fn quoted_section_from_preparsed(
    preparsed: &PreparsedDocument,
    marked: &[MarkedPreparsedSection<'_>],
    weights: &BTreeMap<String, f64>,
    sources: Option<&BTreeSet<String>>,
    requested: &BTreeSet<String>,
    prefer_head: bool,
) -> Option<Section> {
    if prefer_head && sources.is_none() && requested.is_empty() {
        return preparsed.first_body.as_ref().map(|fb| Section {
            heading: fb.heading.clone(),
            slug: fb.slug.clone(),
            units: fb.units.clone(),
            hash: fb.hash.clone(),
            shown: fb.shown.clone(),
        });
    }
    let mut best = None;
    let mut requested_best = None;
    let mut first = None;
    for (index, item) in marked.iter().enumerate() {
        let is_requested = requested.contains(&item.meta.folded_heading);
        if !is_requested
            && sources.is_some_and(|sources| {
                citation_refs_only(item.meta.citation_refs.as_deref(), sources)
            })
        {
            continue;
        }
        if item.marked_units.is_empty() {
            continue;
        }
        if first.is_none() {
            first = Some(index);
        }
        let mut all = Vec::with_capacity(1 + item.marked_units.len());
        all.push(item.marked_heading.clone());
        all.extend(item.marked_units.iter().cloned());
        let key = (score(&all, weights), score(&item.marked_units, weights));
        if best.as_ref().is_none_or(|(previous, _)| key > *previous) {
            best = Some((key, index));
        }
        if is_requested
            && requested_best
                .as_ref()
                .is_none_or(|(previous, _)| key > *previous)
        {
            requested_best = Some((key, index));
        }
    }
    let index = if let Some((_, index)) = requested_best {
        index
    } else if prefer_head {
        return preparsed.first_body.as_ref().map(|fb| Section {
            heading: fb.heading.clone(),
            slug: fb.slug.clone(),
            units: fb.units.clone(),
            hash: fb.hash.clone(),
            shown: fb.shown.clone(),
        });
    } else {
        let (key, index) = best?;
        if key.0.0 > 0 { index } else { first? }
    };
    let chosen = &marked[index];
    Some(Section {
        heading: chosen.meta.heading.clone(),
        slug: chosen.meta.slug.clone(),
        units: chosen.marked_units.clone(),
        hash: chosen.meta.hash.clone(),
        shown: chosen.meta.shown.clone(),
    })
}
fn section_from_preparsed(
    conn: &Connection,
    item: &Candidate,
    preparsed: &PreparsedDocument,
    marked: &[MarkedPreparsedSection<'_>],
    weights: &BTreeMap<String, f64>,
    query: &str,
) -> Option<Section> {
    let linked = matches!(
        item.selection.as_deref(),
        Some("path-link" | "source-link" | "source-hint" | "prompt-link" | "touch-link")
    );
    let prefer_head = item.document.kind == "changelog";
    if linked && item.document.kind != "brain-task" || item.document.kind == "memory" {
        let words = query::word_tokens(query);
        let requested = ["sources", "references"]
            .into_iter()
            .filter(|heading| words.iter().any(|word| word == heading))
            .map(String::from)
            .collect::<BTreeSet<_>>();
        if !requested.is_empty() {
            return quoted_section_from_preparsed(
                preparsed,
                marked,
                weights,
                None,
                &requested,
                prefer_head,
            );
        }
        let sources = document_source_links(conn, &item.document.path);
        if !sources.is_empty()
            && sources.len() <= 32
            && let Some(found) = quoted_section_from_preparsed(
                preparsed,
                marked,
                weights,
                Some(&sources.into_iter().collect()),
                &BTreeSet::new(),
                false,
            )
        {
            return Some(found);
        }
    }
    quoted_section_from_preparsed(
        preparsed,
        marked,
        weights,
        None,
        &BTreeSet::new(),
        prefer_head,
    )
}
fn document_source_links(conn: &Connection, path: &str) -> Vec<String> {
    (|| -> rusqlite::Result<Vec<String>> {
        let mut statement = conn.prepare_cached(
            "SELECT ref_path FROM document_links WHERE path=?1 AND ref_kind='source' ORDER BY ref_path LIMIT 33",
        )?;
        let rows = statement.query_map([path], |row| row.get(0))?;
        rows.collect()
    })()
    .unwrap_or_default()
}
fn citation_only(heading: &str, lines: &[String], sources: &BTreeSet<String>) -> bool {
    if !["sources", "references"].contains(&query::folded(&unmarked(heading)).as_str()) {
        return false;
    }
    let mut count = 0;
    for line in lines.iter().filter(|line| !line.trim().is_empty()) {
        let plain = unmarked(line);
        let line = plain.trim();
        if !line.starts_with(['-', '*', '+'])
            || !line.chars().nth(1).is_some_and(char::is_whitespace)
        {
            return false;
        }
        let reference = line[1..].trim().trim_matches('`');
        if sources.contains(reference) {
            count += 1;
            continue;
        }
        let Some((base, fragment)) = reference.split_once('#') else {
            return false;
        };
        if !sources.contains(base)
            || fragment.is_empty()
            || fragment.chars().any(char::is_whitespace)
            || fragment.contains('`')
        {
            return false;
        }
        count += 1;
    }
    count > 0
}
pub fn quoted_section(
    text: &str,
    title: &str,
    weights: &BTreeMap<String, f64>,
    sources: Option<&BTreeSet<String>>,
    requested: &BTreeSet<String>,
    prefer_head: bool,
) -> Option<Section> {
    if prefer_head && sources.is_none() && requested.is_empty() {
        return first_body(text).map(|(heading, lines)| payload(&heading, &lines, title));
    }
    let sections = sections(text);
    let mut best = None;
    let mut requested_best = None;
    let mut first = None;
    for (index, (heading, lines)) in sections.iter().enumerate() {
        let requested = requested.contains(&query::folded(&unmarked(heading)));
        if !requested && sources.is_some_and(|sources| citation_only(heading, lines, sources)) {
            continue;
        }
        let units = units(lines);
        if units.is_empty() {
            continue;
        }
        if first.is_none() {
            first = Some(index);
        }
        let mut all = vec![heading.clone()];
        all.extend(units.clone());
        let key = (score(&all, weights), score(&units, weights));
        if best.as_ref().is_none_or(|(previous, _)| key > *previous) {
            best = Some((key, index));
        }
        if requested
            && requested_best
                .as_ref()
                .is_none_or(|(previous, _)| key > *previous)
        {
            requested_best = Some((key, index));
        }
    }
    let (index, _) = if let Some((_, index)) = requested_best {
        (index, true)
    } else if prefer_head {
        return first_body(text).map(|(heading, lines)| payload(&heading, &lines, title));
    } else {
        let (key, index) = best?;
        (if key.0.0 > 0 { index } else { first? }, false)
    };
    let (heading, lines) = &sections[index];
    Some(payload(heading, lines, title))
}
pub fn section(
    conn: &Connection,
    item: &Candidate,
    text: &str,
    weights: &BTreeMap<String, f64>,
    query: &str,
) -> Option<Section> {
    let linked = matches!(
        item.selection.as_deref(),
        Some("path-link" | "source-link" | "source-hint" | "prompt-link" | "touch-link")
    );
    let prefer_head = item.document.kind == "changelog";
    if linked && item.document.kind != "brain-task" || item.document.kind == "memory" {
        let words = query::word_tokens(query);
        let requested = ["sources", "references"]
            .into_iter()
            .filter(|heading| words.iter().any(|word| word == heading))
            .map(String::from)
            .collect::<BTreeSet<_>>();
        if !requested.is_empty() {
            return quoted_section(
                text,
                &item.document.title,
                weights,
                None,
                &requested,
                prefer_head,
            );
        }
        let sources=(||->rusqlite::Result<Vec<String>>{let mut statement=conn.prepare("SELECT ref_path FROM document_links WHERE path=?1 AND ref_kind='source' ORDER BY ref_path LIMIT 33")?;let rows=statement.query_map([&item.document.path],|row|row.get(0))?;rows.collect()})().unwrap_or_default();
        if !sources.is_empty()
            && sources.len() <= 32
            && let Some(found) = quoted_section(
                text,
                &item.document.title,
                weights,
                Some(&sources.into_iter().collect()),
                &BTreeSet::new(),
                false,
            )
        {
            return Some(found);
        }
    }
    quoted_section(
        text,
        &item.document.title,
        weights,
        None,
        &BTreeSet::new(),
        prefer_head,
    )
}
thread_local! {
    /// One in-memory FTS5 table per thread marks every held image of a
    /// retrieval; creating the table cost more than marking the text.
    static HELD_MARKER: std::cell::RefCell<Option<Connection>> =
        const { std::cell::RefCell::new(None) };
}
fn held_marked(image: &IndexedImage, tokens: &[String]) -> Result<String> {
    HELD_MARKER.with(|cell| {
        let mut slot = cell.borrow_mut();
        let conn = match slot.take() {
            Some(conn) => conn,
            None => {
                let conn = Connection::open_in_memory()?;
                conn.execute_batch("CREATE VIRTUAL TABLE held USING fts5(path UNINDEXED,layer UNINDEXED,kind UNINDEXED,title,summary,content,tokenize='porter unicode61')")?;
                conn
            }
        };
        let marked = mark_held(&conn, image, tokens);
        // A failed statement may leave the row behind: the next image gets a
        // new table rather than a second row.
        if marked.is_ok() {
            *slot = Some(conn);
        }
        marked
    })
}
fn mark_held(conn: &Connection, image: &IndexedImage, tokens: &[String]) -> Result<String> {
    use rusqlite::OptionalExtension;
    conn.prepare_cached("DELETE FROM held")?.execute([])?;
    conn.prepare_cached("INSERT INTO held VALUES('','','',?1,?2,?3)")?
        .execute(params![image.title, image.summary, image.content])?;
    let marked = conn
        .prepare_cached("SELECT highlight(held,5,?1,?2) FROM held WHERE held MATCH ?3")?
        .query_row(
            params![
                OPEN.to_string(),
                CLOSE.to_string(),
                query::fts_expression(tokens)
            ],
            |row| row.get(0),
        )
        .optional()?;
    conn.prepare_cached("DELETE FROM held")?.execute([])?;
    Ok(marked.unwrap_or_else(|| image.content.clone()))
}
fn answering_stretch(text: &str, tokens: &[String]) -> String {
    let characters = text.chars().collect::<Vec<_>>();
    if characters.len() <= MARKED_LIMIT {
        return text.into();
    }
    let forms = query::term_forms(tokens);
    let stems = forms
        .values()
        .flat_map(|form| form.split_whitespace().map(String::from))
        .chain(tokens.iter().map(|token| query::folded(token)))
        .collect::<BTreeSet<_>>();
    static WORDS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let words =
        WORDS.get_or_init(|| regex::Regex::new(r"[\p{L}\p{N}_]+").expect("constant word grammar"));
    let mut sections = Vec::<(usize, usize)>::new();
    let mut offset = 0;
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        if heading(line).is_some() && offset > start {
            sections.push((start, offset));
            start = offset;
        }
        offset += line.chars().count();
    }
    sections.push((start, characters.len()));
    let mut best = None;
    for (start, end) in sections {
        let body = characters[start..end].iter().collect::<String>();
        let found = words
            .find_iter(&body)
            .filter(|word| {
                stems
                    .iter()
                    .any(|stem| query::folded(word.as_str()).starts_with(stem))
            })
            .map(|word| {
                (
                    body[..word.start()].chars().count() + start,
                    query::folded(word.as_str()),
                )
            })
            .collect::<Vec<_>>();
        if found.is_empty() {
            continue;
        }
        let key = (
            found
                .iter()
                .map(|(_, word)| word.clone())
                .collect::<BTreeSet<_>>()
                .len(),
            found.len(),
            std::cmp::Reverse(start),
        );
        if best
            .as_ref()
            .is_none_or(|(previous, _, _, _)| key > *previous)
        {
            best = Some((key, start, end, found));
        }
    }
    let Some((_, start, end, found)) = best else {
        return characters[..MARKED_LIMIT].iter().collect();
    };
    if end - start <= MARKED_LIMIT {
        return characters[start..end].iter().collect();
    }
    let heading_end = if characters[start] == '#' {
        characters[start..end]
            .iter()
            .position(|character| *character == '\n')
            .map(|offset| start + offset + 1)
            .unwrap_or(start)
    } else {
        start
    };
    let heading = characters[start..heading_end].iter().collect::<String>();
    let span = MARKED_LIMIT.saturating_sub(heading.chars().count());
    let found = found
        .into_iter()
        .filter(|(position, _)| *position >= heading_end)
        .take(50000)
        .collect::<Vec<_>>();
    if found.is_empty() {
        return format!(
            "{heading}{}",
            characters[heading_end..(heading_end + span).min(end)]
                .iter()
                .collect::<String>()
        );
    }
    let mut counts = BTreeMap::<String, usize>::new();
    let mut tail = 0;
    let mut best_score = (0, 0);
    let mut begin = found[0].0;
    for (index, (position, word)) in found.iter().enumerate() {
        *counts.entry(word.clone()).or_default() += 1;
        while found[tail].0 < position.saturating_sub(span / 2) {
            if let Some(count) = counts.get_mut(&found[tail].1) {
                *count -= 1;
                if *count == 0 {
                    counts.remove(&found[tail].1);
                }
            }
            tail += 1;
        }
        let key = (counts.len(), index - tail + 1);
        if key > best_score {
            best_score = key;
            begin = found[tail].0;
        }
    }
    let begin = heading_end.max(begin.saturating_sub(span / 4));
    format!(
        "{heading}{}",
        characters[begin..(begin + span).min(end)]
            .iter()
            .collect::<String>()
    )
}
/// What quoting reads from the index for one query: its evidence words and
/// their excerpt weights. Both depend on the query and the index alone, so a
/// ranking reads them once for all its candidates, as the retired quoting read
/// its weights once per query.
pub struct QueryTerms {
    evidence: Vec<String>,
    weights: BTreeMap<String, f64>,
}
impl QueryTerms {
    pub fn read(conn: &Connection, query: &str) -> Self {
        Self {
            evidence: evidence_words(conn, query),
            weights: weights(conn, query),
        }
    }
}
fn evidence_words(conn: &Connection, query: &str) -> Vec<String> {
    query::query_tokens(query)
        .and_then(|tokens| query::expand_query_tokens(conn, tokens))
        .and_then(|tokens| query::informative_tokens(conn, tokens))
        .map(|tokens| query::evidence_tokens(&tokens))
        .unwrap_or_default()
}
pub fn marked(conn: &Connection, item: &Candidate, query: &str) -> String {
    marked_with(item, &evidence_words(conn, query))
}
fn marked_with(item: &Candidate, tokens: &[String]) -> String {
    let mut image = item.image();
    if image.content.chars().count() > MARKED_LIMIT {
        image.content = answering_stretch(&image.content, tokens);
    }
    if tokens.is_empty() {
        return image.content;
    }
    held_marked(&image, tokens).unwrap_or(image.content)
}
pub fn window(
    units: &[String],
    limit: usize,
    weights: &BTreeMap<String, f64>,
    prefer_head: bool,
) -> String {
    let plain = units.iter().map(|unit| unmarked(unit)).collect::<Vec<_>>();
    let whole = plain.join(" ");
    if whole.chars().count() <= limit {
        return whole;
    }
    let budget = limit.saturating_sub(4).max(1);
    if prefer_head {
        let mut text = String::new();
        for unit in &plain {
            let joined = if text.is_empty() {
                unit.clone()
            } else {
                format!("{text} {unit}")
            };
            if joined.chars().count() <= budget {
                text = joined;
                continue;
            }
            let room = budget.saturating_sub(text.chars().count() + usize::from(!text.is_empty()));
            if room > 40 {
                let mut piece = query::chars(unit, room);
                if let Some((position, _)) = piece
                    .char_indices()
                    .rev()
                    .find(|(_, character)| *character == ' ')
                    && piece[..position].chars().count() > room / 2
                {
                    piece.truncate(position);
                }
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(piece.trim());
            }
            return format!("{} …", text.trim());
        }
        return text;
    }
    let mut best = None::<((i64, usize), usize, usize, String, bool, bool)>;
    for start in 0..units.len() {
        let mut end = start;
        let mut length = 0;
        while end < units.len() {
            let cost = plain[end].chars().count() + usize::from(end > start);
            if length + cost > budget {
                break;
            }
            length += cost;
            end += 1;
        }
        let (key, text, left, right) = if end == start {
            let (key, text, left, right) = piece(&units[start], budget, weights);
            (key, text, left > 0, right < plain[start].chars().count())
        } else {
            (
                score(&units[start..end], weights),
                plain[start..end].join(" "),
                false,
                false,
            )
        };
        if best.as_ref().is_none_or(|(previous, _, _, _, _, _)| {
            key > *previous || key == *previous && key.1 > 0
        }) {
            best = Some((key, start, end, text, left, right));
        }
    }
    let Some((winning, start, mut end, mut text, left, right)) = best else {
        return String::new();
    };
    if end > start && start > 0 && plain[start - 1].ends_with(':') {
        let lead = plain[start - 1].chars().count() + 1;
        let mut size = text.chars().count();
        let mut last = end;
        while last > start && size + lead > budget {
            last -= 1;
            size = size.saturating_sub(plain[last].chars().count() + 1);
        }
        if last > start && size + lead <= budget && score(&units[start..last], weights) == winning {
            text = plain[start - 1..last].join(" ");
            end = last;
            return format!(
                "{}{}{}",
                if start > 1 { "… " } else { "" },
                text,
                if end < units.len() { " …" } else { "" }
            );
        }
    }
    query::chars(
        &format!(
            "{}{}{}",
            if start > 0 || left { "… " } else { "" },
            text,
            if right || end < units.len() {
                " …"
            } else {
                ""
            }
        ),
        limit,
    )
}
fn piece(
    unit: &str,
    budget: usize,
    weights: &BTreeMap<String, f64>,
) -> ((i64, usize), String, usize, usize) {
    let mut plain = Vec::new();
    let mut words = Vec::<(usize, usize, String)>::new();
    let mut marked = false;
    let mut begin = 0;
    for character in unit.chars() {
        if character == OPEN {
            marked = true;
            begin = plain.len();
        } else if character == CLOSE {
            if marked {
                let text = plain[begin..].iter().collect::<String>();
                let mut offset = 0;
                for word in text.split_whitespace() {
                    let start = text[offset..].find(word).unwrap_or(0) + offset;
                    words.push((
                        begin + text[..start].chars().count(),
                        begin + text[..start + word.len()].chars().count(),
                        word.into(),
                    ));
                    offset = start + word.len();
                }
            }
            marked = false;
        } else {
            plain.push(character);
        }
    }
    let forms = query::term_forms(
        &words
            .iter()
            .map(|(_, _, word)| word.clone())
            .collect::<Vec<_>>(),
    );
    let mut positions = BTreeSet::from([0usize]);
    for (start, end, _) in &words {
        positions.extend([
            start.saturating_sub(budget / 3),
            *start,
            end.saturating_sub(budget),
        ]);
    }
    let mut best = None;
    for position in positions {
        let mut left = position.min(plain.len());
        if left > 0
            && !plain[left - 1].is_whitespace()
            && let Some(space) = plain[..=left.min(plain.len().saturating_sub(1))]
                .iter()
                .rposition(|character| *character == ' ')
        {
            left = space + 1;
        }
        let mut right = (left + budget).min(plain.len());
        if right < plain.len()
            && !plain[right].is_whitespace()
            && let Some(space) = plain[left..right]
                .iter()
                .rposition(|character| *character == ' ')
            && space > 0
        {
            right = left + space;
        }
        let visible = words
            .iter()
            .filter(|(start, end, _)| *start >= left && *end <= right)
            .collect::<Vec<_>>();
        let stems = visible
            .iter()
            .flat_map(|(_, _, word)| {
                forms
                    .get(word)
                    .into_iter()
                    .flat_map(|form| form.split_whitespace().map(String::from))
            })
            .collect::<Vec<_>>();
        let score = score_terms(&stems, weights);
        let distance = visible
            .first()
            .map(|(start, _, _)| start.abs_diff(left + budget / 3))
            .unwrap_or(0);
        if best
            .as_ref()
            .is_none_or(|(previous, previous_distance, _, _)| {
                score > *previous
                    || score == *previous && score.1 > 0 && distance < *previous_distance
            })
        {
            best = Some((score, distance, left, right));
        }
    }
    let (score, _, left, right) = best.unwrap_or(((0, 0), 0, 0, budget.min(plain.len())));
    (
        score,
        plain[left..right].iter().collect::<String>().trim().into(),
        left,
        right,
    )
}
pub fn quote_candidate(
    conn: &Connection,
    item: &Candidate,
    query: &str,
    limit: usize,
) -> Result<String> {
    quote_candidate_with(conn, item, query, limit, &QueryTerms::read(conn, query))
}
pub fn quote_candidate_with(
    conn: &Connection,
    item: &Candidate,
    query: &str,
    limit: usize,
    terms: &QueryTerms,
) -> Result<String> {
    let image = item.image();
    if let Some(preparsed) = load_preparsed(conn, item, &image) {
        let tokens = if item.document.kind == "changelog" {
            &[][..]
        } else {
            terms.evidence.as_slice()
        };
        if let Some(marked_sections) = mark_preparsed_sections(&preparsed, tokens) {
            return Ok(section_from_preparsed(
                conn,
                item,
                &preparsed,
                &marked_sections,
                &terms.weights,
                query,
            )
            .map(|section| {
                window(
                    &section.units,
                    limit,
                    &terms.weights,
                    item.document.kind == "changelog",
                )
            })
            .unwrap_or_else(|| query::body_excerpt(&image.content, limit, &[], false)));
        }
    }
    let marked = marked_with(item, &terms.evidence);
    let text = if item.document.kind == "changelog" {
        &image.content
    } else {
        &marked
    };
    Ok(section(conn, item, text, &terms.weights, query)
        .map(|section| {
            window(
                &section.units,
                limit,
                &terms.weights,
                item.document.kind == "changelog",
            )
        })
        .unwrap_or_else(|| query::body_excerpt(&image.content, limit, &[], false)))
}
pub fn delivery_identity(conn: &Connection, item: &Candidate, query: &str) -> (String, String) {
    delivery_identity_with(conn, item, query, None)
}
/// `delivery_identity` with the query's terms when the caller holds them;
/// without them, they are read only when the item's identity depends on the
/// section its quote comes from.
pub fn delivery_identity_with(
    conn: &Connection,
    item: &Candidate,
    query: &str,
    terms: Option<&QueryTerms>,
) -> (String, String) {
    let mut key = item.document.path.clone();
    let mut revision = item.document.source_hash.clone();
    if item.document.layer != "procedural"
        && item.match_strength.as_deref() != Some("distinctive")
        && !(matches!(
            item.selection.as_deref(),
            Some("prompt-link" | "touch-link")
        ) && item.document.kind == "brain-task")
    {
        let read;
        let terms = match terms {
            Some(terms) => terms,
            None => {
                read = QueryTerms::read(conn, query);
                &read
            }
        };
        let image = item.image();
        let mut resolved_from_cache = false;
        if let Some(preparsed) = load_preparsed(conn, item, &image) {
            let tokens = if item.document.kind == "changelog" {
                &[][..]
            } else {
                terms.evidence.as_slice()
            };
            if let Some(marked_sections) = mark_preparsed_sections(&preparsed, tokens) {
                let ordinary_sections = if item.document.kind == "changelog"
                    && matches!(
                        item.selection.as_deref(),
                        Some("prompt-link" | "touch-link")
                    ) {
                    mark_preparsed_sections(&preparsed, &terms.evidence)
                } else {
                    None
                };
                if ordinary_sections.is_some()
                    || item.document.kind != "changelog"
                    || !matches!(
                        item.selection.as_deref(),
                        Some("prompt-link" | "touch-link")
                    )
                {
                    resolved_from_cache = true;
                    if let Some(selected) = section_from_preparsed(
                        conn,
                        item,
                        &preparsed,
                        &marked_sections,
                        &terms.weights,
                        query,
                    ) {
                        let ordinary = if matches!(
                            item.selection.as_deref(),
                            Some("prompt-link" | "touch-link")
                        ) {
                            let ord_slice =
                                ordinary_sections.as_deref().unwrap_or(&marked_sections);
                            quoted_section_from_preparsed(
                                &preparsed,
                                ord_slice,
                                &terms.weights,
                                None,
                                &BTreeSet::new(),
                                false,
                            )
                        } else {
                            None
                        };
                        if !(matches!(
                            item.selection.as_deref(),
                            Some("prompt-link" | "touch-link")
                        ) && ordinary.is_some_and(|ordinary| ordinary.hash == selected.hash))
                        {
                            key = format!("{key}#{}", selected.slug);
                            revision = selected.hash;
                        }
                    }
                }
            }
        }
        if !resolved_from_cache {
            let marked = marked_with(item, &terms.evidence);
            let text = if item.document.kind == "changelog" {
                &image.content
            } else {
                &marked
            };
            if let Some(selected) = section(conn, item, text, &terms.weights, query) {
                let ordinary = quoted_section(
                    &marked,
                    &item.document.title,
                    &terms.weights,
                    None,
                    &BTreeSet::new(),
                    false,
                );
                if !(matches!(
                    item.selection.as_deref(),
                    Some("prompt-link" | "touch-link")
                ) && ordinary.is_some_and(|ordinary| ordinary.hash == selected.hash))
                {
                    key = format!("{key}#{}", selected.slug);
                    revision = selected.hash;
                }
            }
        }
    }
    if !item.document.conflicts.is_empty() || item.match_strength.as_deref() == Some("conflict") {
        revision = query::sha256(format!("recorded-conflict\0{revision}").as_bytes());
    }
    (key, revision)
}
pub fn linked_task_excerpt(content: &str, via: &str, limit: usize) -> (String, String) {
    #[cfg(test)]
    bump_live_section_parses();
    let parsed = preparse_linked_task_raw(content);
    linked_task_excerpt_from_preparsed(&parsed, via, limit)
}
fn prepare_base_with_memo(
    conn: &Connection,
    capsule: &Capsule,
    selected: &[Candidate],
    limits: &BTreeMap<String, usize>,
    memo: &mut BTreeMap<String, Option<PreparsedDocument>>,
) -> Result<PreparedRender> {
    let mut prepared = PreparedRender::default();
    let query = capsule.0["query"].as_str().unwrap_or("");
    let term_weights = weights(conn, query);
    let evidence = if query.is_empty() {
        Vec::new()
    } else {
        evidence_words(conn, query)
    };
    let mut rank = 0;
    for candidate in selected {
        prepared
            .images
            .insert(candidate.document.path.clone(), candidate.image());
    }
    for layer in ["semantic", "episodic"] {
        for item in capsule.0[layer].as_array().into_iter().flatten() {
            let Some(path) = item["path"].as_str() else {
                if let Some(id) = item["id"].as_i64() {
                    let text = crate::capsule::bounded(item["outcome"].as_str().unwrap_or(""), 600);
                    if !text.is_empty() {
                        prepared.excerpts.insert(
                            format!("episode {id}"),
                            Excerpt {
                                text,
                                heading: String::new(),
                                limit: 600,
                                cut: false,
                            },
                        );
                    }
                }
                continue;
            };
            if item["match"] == "distinctive" {
                continue;
            }
            let Some(candidate) = selected
                .iter()
                .find(|candidate| candidate.document.path == path)
            else {
                continue;
            };
            let image = prepared
                .images
                .get(path)
                .cloned()
                .unwrap_or_else(|| candidate.image());
            let held_content = image.content.as_str();
            let linked = matches!(
                candidate.selection.as_deref(),
                Some("prompt-link" | "touch-link")
            ) && candidate.document.kind == "brain-task";
            let default = if layer == "semantic" || linked {
                [800, 600, 400][rank.min(2)]
            } else {
                600
            };
            let limit = limits.get(path).copied().unwrap_or(default);
            rank += 1;
            let preparsed = memo
                .entry(path.to_string())
                .or_insert_with(|| load_preparsed(conn, candidate, &image));
            if linked {
                let (text, heading) = if let Some(Some(parsed)) =
                    preparsed.as_ref().map(|p| p.linked_task.as_ref())
                {
                    linked_task_excerpt_from_preparsed(
                        parsed,
                        candidate.via.as_deref().unwrap_or(""),
                        limit,
                    )
                } else {
                    linked_task_excerpt(held_content, candidate.via.as_deref().unwrap_or(""), limit)
                };
                if !text.is_empty() {
                    let cut = text.chars().count() >= limit.saturating_sub(40);
                    prepared.excerpts.insert(
                        path.into(),
                        Excerpt {
                            text,
                            heading,
                            limit,
                            cut,
                        },
                    );
                }
                continue;
            }
            let section = if let Some(parsed) = preparsed.as_ref() {
                let tokens = if query.is_empty() || candidate.document.kind == "changelog" {
                    &[][..]
                } else {
                    evidence.as_slice()
                };
                if let Some(marked_sections) = mark_preparsed_sections(parsed, tokens) {
                    section_from_preparsed(
                        conn,
                        candidate,
                        parsed,
                        &marked_sections,
                        &term_weights,
                        query,
                    )
                } else {
                    let marked = if query.is_empty() {
                        held_content.to_string()
                    } else {
                        marked_with(candidate, &evidence)
                    };
                    let text = if candidate.document.kind == "changelog" {
                        held_content
                    } else {
                        &marked
                    };
                    section(conn, candidate, text, &term_weights, query)
                }
            } else {
                let marked = if query.is_empty() {
                    held_content.to_string()
                } else {
                    marked_with(candidate, &evidence)
                };
                let text = if candidate.document.kind == "changelog" {
                    held_content
                } else {
                    &marked
                };
                section(conn, candidate, text, &term_weights, query)
            };
            let mut body = section
                .as_ref()
                .map(|section| {
                    window(
                        &section.units,
                        limit,
                        &term_weights,
                        candidate.document.kind == "changelog",
                    )
                })
                .unwrap_or_default();
            if body.is_empty() {
                body = crate::capsule::bounded(&candidate.snippet.replace(['[', ']'], ""), limit);
            }
            if !body.is_empty() {
                let cut = section.as_ref().is_some_and(|section| {
                    section
                        .units
                        .iter()
                        .map(|unit| unmarked(unit).chars().count() + 1)
                        .sum::<usize>()
                        > limit
                });
                prepared.excerpts.insert(
                    path.into(),
                    Excerpt {
                        text: body,
                        heading: section.map(|section| section.shown).unwrap_or_default(),
                        limit,
                        cut,
                    },
                );
            }
        }
    }
    Ok(prepared)
}
fn prepare_with_memo(
    conn: &Connection,
    capsule: &Capsule,
    selected: &[Candidate],
    memo: &mut BTreeMap<String, Option<PreparsedDocument>>,
) -> Result<PreparedRender> {
    let initial = prepare_base_with_memo(conn, capsule, selected, &BTreeMap::new(), memo)?;
    let reserve =
        1200 + if capsule.0["recent_files"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
        {
            721
        } else {
            0
        } + if capsule.0["ticket_files"].is_object() {
            721
        } else {
            0
        };
    let used = initial
        .excerpts
        .values()
        .filter(|excerpt| !excerpt.text.is_empty())
        .map(|excerpt| excerpt.text.chars().count() + 3)
        .sum::<usize>();
    let mut room = 3600usize.saturating_sub(reserve + used);
    let mut grown = BTreeMap::new();
    let order = ["semantic", "episodic"]
        .into_iter()
        .flat_map(|layer| capsule.0[layer].as_array().into_iter().flatten())
        .filter_map(|item| item["path"].as_str())
        .filter(|path| {
            initial
                .excerpts
                .get(*path)
                .is_some_and(|excerpt| !excerpt.text.is_empty())
        });
    for (cap, path) in [1800, 1200].into_iter().zip(order) {
        let excerpt = &initial.excerpts[path];
        if room == 0 || excerpt.limit == 0 || !excerpt.cut || cap <= excerpt.limit {
            continue;
        }
        let limit = excerpt.limit + (cap - excerpt.limit).min(room);
        room -= limit - excerpt.limit;
        grown.insert(path.to_string(), limit);
    }
    if grown.is_empty() {
        Ok(initial)
    } else {
        prepare_base_with_memo(conn, capsule, selected, &grown, memo)
    }
}
pub fn prepare(
    conn: &Connection,
    capsule: &Capsule,
    selected: &[Candidate],
) -> Result<PreparedRender> {
    let mut memo = BTreeMap::new();
    prepare_with_memo(conn, capsule, selected, &mut memo)
}
fn packed_membership(capsule: &Capsule) -> Vec<(String, String)> {
    ["procedural", "semantic", "episodic"]
        .into_iter()
        .flat_map(|layer| {
            capsule.0[layer]
                .as_array()
                .into_iter()
                .flatten()
                .map(move |item| {
                    (
                        layer.to_string(),
                        item["path"]
                            .as_str()
                            .map(String::from)
                            .unwrap_or_else(|| format!("episode:{}", item["id"])),
                    )
                })
        })
        .collect()
}
pub fn prepare_with(
    conn: &Connection,
    capsule: &mut Capsule,
    selected: &mut [Candidate],
    profile: crate::capsule::DeliveryProfile,
    preparer: &mut dyn ExcerptPreparer,
) -> Result<bool> {
    let mut memo = BTreeMap::new();
    let lexical = prepare_with_memo(conn, capsule, selected, &mut memo)?;
    capsule.1 = Some(lexical.clone());
    let query = capsule.0["query"].as_str().unwrap_or("");
    let term_weights = weights(conn, query);
    let evidence = if query.is_empty() {
        Vec::new()
    } else {
        evidence_words(conn, query)
    };
    let mut offered = Vec::new();
    for item in selected.iter() {
        let Some(excerpt) = lexical.excerpts.get(&item.document.path) else {
            continue;
        };
        let identity = item
            .delivery_identity
            .clone()
            .unwrap_or_else(|| delivery_identity(conn, item, query));
        let mut alternatives = vec![PreparedAlternative {
            text: excerpt.text.clone(),
            heading: excerpt.heading.clone(),
            key: identity.0,
            revision: identity.1,
            limit: excerpt.limit,
            cut: excerpt.cut,
        }];
        if item.document.layer != "procedural"
            && item.match_strength.as_deref() != Some("distinctive")
            && !(item.document.kind == "brain-task"
                && matches!(
                    item.selection.as_deref(),
                    Some("prompt-link" | "touch-link")
                ))
        {
            let sources = document_source_links(conn, &item.document.path)
                .into_iter()
                .collect::<BTreeSet<_>>();
            let image = lexical
                .images
                .get(&item.document.path)
                .cloned()
                .unwrap_or_else(|| item.image());
            let preparsed = memo
                .entry(item.document.path.clone())
                .or_insert_with(|| load_preparsed(conn, item, &image));
            let mut handled_from_cache = false;
            if let Some(parsed) = preparsed.as_ref()
                && let Some(marked_sections) = mark_preparsed_sections(parsed, &evidence)
            {
                handled_from_cache = true;
                for sec in marked_sections.into_iter().take(64) {
                    if sources.len() <= 32
                        && citation_refs_only(sec.meta.citation_refs.as_deref(), &sources)
                    {
                        continue;
                    }
                    if sec.marked_units.is_empty() {
                        continue;
                    }
                    let text = window(
                        &sec.marked_units,
                        excerpt.limit,
                        &term_weights,
                        item.document.kind == "changelog",
                    );
                    if text.is_empty()
                        || alternatives.iter().any(|alternative| {
                            alternative.text == text && alternative.heading == sec.meta.shown
                        })
                    {
                        continue;
                    }
                    let mut revision = sec.meta.hash.clone();
                    if !item.document.conflicts.is_empty() {
                        revision =
                            query::sha256(format!("recorded-conflict\0{revision}").as_bytes());
                    }
                    alternatives.push(PreparedAlternative {
                        text,
                        heading: sec.meta.shown.clone(),
                        key: format!("{}#{}", item.document.path, sec.meta.slug),
                        revision,
                        limit: excerpt.limit,
                        cut: sec
                            .marked_units
                            .iter()
                            .map(|unit| unmarked(unit).chars().count() + 1)
                            .sum::<usize>()
                            > excerpt.limit,
                    });
                }
            }
            if !handled_from_cache {
                let held = marked_with(item, &evidence);
                for (heading, lines) in sections(&held).into_iter().take(64) {
                    if sources.len() <= 32 && citation_only(&heading, &lines, &sources) {
                        continue;
                    }
                    let section = payload(&heading, &lines, &item.document.title);
                    if section.units.is_empty() {
                        continue;
                    }
                    let text = window(
                        &section.units,
                        excerpt.limit,
                        &term_weights,
                        item.document.kind == "changelog",
                    );
                    if text.is_empty()
                        || alternatives.iter().any(|alternative| {
                            alternative.text == text && alternative.heading == section.shown
                        })
                    {
                        continue;
                    }
                    let mut revision = section.hash;
                    if !item.document.conflicts.is_empty() {
                        revision =
                            query::sha256(format!("recorded-conflict\0{revision}").as_bytes());
                    }
                    alternatives.push(PreparedAlternative {
                        text,
                        heading: section.shown,
                        key: format!("{}#{}", item.document.path, section.slug),
                        revision,
                        limit: excerpt.limit,
                        cut: section
                            .units
                            .iter()
                            .map(|unit| unmarked(unit).chars().count() + 1)
                            .sum::<usize>()
                            > excerpt.limit,
                    });
                }
            }
        }
        offered.push(PreparationCandidate {
            path: item.document.path.clone(),
            alternatives,
        });
    }
    let choice = match preparer.select(query, &offered) {
        Ok(choice) => choice,
        Err(_) => return Ok(false),
    };
    if choice.is_empty() {
        return Ok(false);
    }
    let mut proposed = lexical.clone();
    let mut identities = BTreeMap::new();
    let mut changed = false;
    for (path, index) in &choice {
        let Some(candidate) = offered.iter().find(|candidate| &candidate.path == path) else {
            return Ok(false);
        };
        let Some(alternative) = candidate.alternatives.get(*index) else {
            return Ok(false);
        };
        changed |= *index != 0;
        proposed.excerpts.insert(
            path.clone(),
            Excerpt {
                text: alternative.text.clone(),
                heading: alternative.heading.clone(),
                limit: alternative.limit,
                cut: alternative.cut,
            },
        );
        identities.insert(
            path.clone(),
            (alternative.key.clone(), alternative.revision.clone()),
        );
    }
    if !changed {
        return Ok(false);
    }
    // A window change may consume a different amount of header/body room. The
    // complete task, episode and transport profile must retain identical pack
    // membership and order, otherwise every proposed window and identity falls
    // back together to its lexical baseline.
    let baseline = match crate::capsule::pack(capsule.clone(), profile) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    let mut proposal = capsule.clone();
    proposal.1 = Some(proposed.clone());
    let proposal = match crate::capsule::pack(proposal, profile) {
        Ok(value) => value,
        Err(_) => return Ok(false),
    };
    if packed_membership(&baseline) != packed_membership(&proposal) {
        return Ok(false);
    }
    capsule.1 = Some(proposed);
    for item in selected {
        if let Some(identity) = identities.remove(&item.document.path) {
            item.delivery_identity = Some(identity);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod constant_pattern_tests {
    use super::*;

    #[test]
    fn marked_spans_keep_multiline_empty_and_unterminated_boundaries() {
        let text = "plain\u{2}Cache\u{3} \u{2}multi\nline\u{3} \u{2}\u{3} tail\u{2}unterminated";
        for _ in 0..2 {
            assert_eq!(marked_words(text), ["Cache", "multi\nline", ""]);
            assert!(marked_words("unmarked text").is_empty());
        }
    }

    #[test]
    fn units_keep_sentence_list_continuation_and_table_boundaries() {
        let lines = [
            "Paragraph one. Next sentence!",
            "",
            "- first item",
            "continues",
            "2) second item",
            "| A | B |",
            "| 1 | 2 |",
        ]
        .map(String::from);
        for _ in 0..2 {
            assert_eq!(
                units(&lines),
                [
                    "Paragraph one.",
                    "Next sentence!",
                    "- first item continues",
                    "2) second item",
                    "| A | B |",
                    "| 1 | 2 |",
                ]
            );
        }
    }

    #[test]
    fn linked_entries_keep_selected_source_and_reverse_file_order() {
        let content = "## Goal\nPreserve order\n\n## Files\n- `src/A.php`\n- src/B.php\n\n## Sources\n- `docs/C.md`\n";
        for _ in 0..2 {
            assert_eq!(
                linked_task_excerpt(content, "src/B.php", 400),
                (
                    "Goal: Preserve order. Files: src/B.php, src/A.php".into(),
                    "Files".into()
                )
            );
            assert_eq!(
                linked_task_excerpt(content, "docs/C.md", 400),
                (
                    "Goal: Preserve order. Sources: docs/C.md".into(),
                    "Sources".into()
                )
            );
        }
    }
}
#[cfg(test)]
mod held_marker_tests {
    use super::*;

    /// The marker as it was: a new table for every image.
    fn fresh(image: &IndexedImage, tokens: &[String]) -> Result<String> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("CREATE VIRTUAL TABLE held USING fts5(path UNINDEXED,layer UNINDEXED,kind UNINDEXED,title,summary,content,tokenize='porter unicode61')")?;
        mark_held(&conn, image, tokens)
    }

    #[test]
    fn held_marked_reuses_thread_local_connection_across_success_and_error() {
        let image = |title: &str, content: &str| IndexedImage {
            title: title.into(),
            summary: String::new(),
            content: content.into(),
        };
        let images = [
            image("Retry", "The retry queue backs off; queues retry twice."),
            image("Sessions", "Sessions expire after an hour of inactivity."),
            image("Straße", "Die Straße wird beim Wiederholen gesperrt."),
        ];
        let queries = [
            vec!["retry".to_string()],
            vec!["absent".to_string()],
            vec!["queue".to_string(), "sessions".to_string()],
            vec!["straße".to_string()],
        ];
        for _ in 0..2 {
            for image in &images {
                for tokens in &queries {
                    assert_eq!(
                        held_marked(image, tokens).unwrap(),
                        fresh(image, tokens).unwrap()
                    );
                }
            }
        }
        // The table is kept between images, and empty between them.
        HELD_MARKER.with(|cell| {
            let held = cell.borrow();
            let conn = held.as_ref().unwrap();
            let rows: i64 = conn
                .query_row("SELECT count(*) FROM held", [], |row| row.get(0))
                .unwrap();
            assert_eq!(rows, 0);
            conn.execute_batch("DROP TABLE held").unwrap();
        });
        // A failed statement gives the connection up, and the next image
        // gets a new table.
        assert!(held_marked(&images[0], &queries[0]).is_err());
        HELD_MARKER.with(|cell| assert!(cell.borrow().is_none()));
        assert_eq!(
            held_marked(&images[0], &queries[0]).unwrap(),
            fresh(&images[0], &queries[0]).unwrap()
        );
    }
}

#[cfg(test)]
mod preparsed_section_digest_tests {
    use super::*;
    use crate::{
        Roots, RuntimeContext,
        capsule::DeliveryProfile,
        domain::uuid4,
        index::refresh_index,
        retrieval::{RetrievalRequest, retrieve},
        storage::{OpenOptions, Store},
    };
    use std::path::PathBuf;

    struct OwnedDir(PathBuf);
    impl Drop for OwnedDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn reset_counters() {
        LIVE_SECTION_PARSES.with(|cell| cell.set(0));
        PREPARSED_CACHE_HITS.with(|cell| cell.set(0));
    }

    fn live_parses() -> usize {
        LIVE_SECTION_PARSES.with(|cell| cell.get())
    }

    fn cache_hits() -> usize {
        PREPARSED_CACHE_HITS.with(|cell| cell.get())
    }

    #[test]
    fn refresh_index_persists_preparsed_sections_and_retrieve_skips_live_parses() {
        let root = std::env::temp_dir().join(format!("infra-preparsed-{}", uuid4()));
        std::fs::create_dir_all(root.join("memory-bank/chunks")).unwrap();
        std::fs::create_dir_all(root.join("memory-bank/local")).unwrap();
        std::fs::create_dir_all(root.join("project-brain/tasks")).unwrap();
        std::fs::create_dir_all(root.join("specs")).unwrap();
        std::fs::create_dir_all(root.join("docs")).unwrap();
        let _dir = OwnedDir(root.clone());

        std::fs::write(
            root.join("docs/invoice.md"),
            "# Invoice Docs\n\n## Sequence\nYearly invoice sequence resets cleanly under lock.\n",
        )
        .unwrap();
        std::fs::write(
            root.join("specs/invoice.md"),
            "# Invoice Spec\n\n## Sequence\nYearly invoice sequence resets cleanly.\n",
        )
        .unwrap();
        std::fs::write(
            root.join("specs/MANIFEST.md"),
            "# Manifest\n\n- `specs/invoice.md` — Invoice spec\n",
        )
        .unwrap();
        let metadata = serde_json::json!({
            "id": "MEM-20260101-abcd1234",
            "title": "Invoice Rules",
            "type": "convention",
            "status": "active",
            "scope": ["application"],
            "tags": ["project-brain", "promoted"],
            "created": "2026-01-01",
            "last_verified": "2026-01-01",
            "review_after": "9999-12-31",
            "sources": ["specs/invoice.md", "docs/invoice.md"],
            "supersedes": [],
            "superseded_by": null
        });
        let body = "## Context\nYearly invoice numbering uses tenant locks.\n\n## Decision\nAlways allocate invoice sequence under advisory lock.\n";
        let chunk_bytes = crate::bank::codec::render_chunk(&metadata, body).unwrap();
        std::fs::write(
            root.join("memory-bank/chunks/MEM-20260101-abcd1234-invoice.md"),
            chunk_bytes,
        )
        .unwrap();
        std::fs::write(
            root.join("project-brain/tasks/TASK-001.md"),
            "# TASK-001\n\n## Goal\nMaintain invoice sequence integrity\n\n## Files\n- `specs/invoice.md`\n\n## Sources\n- `specs/invoice.md`\n",
        )
        .unwrap();

        let roots = Roots::installed(&root).unwrap();
        let context = RuntimeContext::new(roots, "tester", "governed").unwrap();
        let mut store = Store::open(
            root.join("memory-bank/local/context.db"),
            OpenOptions::refresh(None),
        )
        .unwrap();
        refresh_index(&mut store, &context, false).unwrap();

        let count: i64 = store
            .conn
            .query_row("SELECT count(*) FROM document_sections", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(
            count >= 3,
            "expected >= 3 document_sections rows, got {count}"
        );

        let request = RetrievalRequest {
            query: "invoice sequence lock".into(),
            task_id: None,
            limit: 5,
            refresh_index: false,
            delivery_profile: DeliveryProfile::Ordinary,
            ..RetrievalRequest::default()
        };

        reset_counters();
        let cached = retrieve(&mut store, &context, &request).unwrap();
        assert_eq!(live_parses(), 0, "cached retrieve must not re-parse files");
        assert!(cache_hits() > 0, "expected document_sections cache hits");
        assert!(
            !cached.0["semantic"]
                .as_array()
                .unwrap_or(&vec![])
                .is_empty(),
            "expected semantic retrieval hits: {}",
            cached.0
        );

        // Compare against uncached live parsing (document_sections cleared): output must match exactly.
        store
            .transaction(|conn| {
                conn.execute("DELETE FROM document_sections", [])?;
                Ok(())
            })
            .unwrap();
        reset_counters();
        let uncached = retrieve(&mut store, &context, &request).unwrap();
        assert!(live_parses() > 0);
        assert_eq!(cache_hits(), 0);
        assert_eq!(cached.0["semantic"], uncached.0["semantic"]);
        assert_eq!(cached.0["episodic"], uncached.0["episodic"]);

        // Re-refresh repopulates document_sections; corrupt payload falls back cleanly.
        refresh_index(&mut store, &context, false).unwrap();
        store
            .transaction(|conn| {
                conn.execute("UPDATE document_sections SET payload='{corrupt'", [])?;
                Ok(())
            })
            .unwrap();
        reset_counters();
        let recovered = retrieve(&mut store, &context, &request).unwrap();
        assert_eq!(cache_hits(), 0);
        assert!(live_parses() > 0);
        assert_eq!(recovered.0["semantic"], cached.0["semantic"]);
    }
}
