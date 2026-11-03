//! Quotes are selected from held indexed images, using SQLite's own marks.
use crate::{
    Result,
    capsule::Capsule,
    query::{self, Candidate},
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
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
    regex::Regex::new("(?s)\u{2}(.*?)\u{3}")
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
pub fn sections(text: &str) -> Vec<(String, Vec<String>)> {
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
    let list = regex::Regex::new(r"^(?:[-*+]|[0-9]+[.)])\s|^\|").ok();
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
fn first_body(text: &str) -> Option<(String, Vec<String>)> {
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
fn held_marked(image: &IndexedImage, tokens: &[String]) -> Result<String> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("CREATE VIRTUAL TABLE held USING fts5(path UNINDEXED,layer UNINDEXED,kind UNINDEXED,title,summary,content,tokenize='porter unicode61')")?;
    conn.execute(
        "INSERT INTO held VALUES('','','',?1,?2,?3)",
        params![image.title, image.summary, image.content],
    )?;
    use rusqlite::OptionalExtension;
    Ok(conn
        .query_row(
            "SELECT highlight(held,5,?1,?2) FROM held WHERE held MATCH ?3",
            params![
                OPEN.to_string(),
                CLOSE.to_string(),
                query::fts_expression(tokens)
            ],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or_else(|| image.content.clone()))
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
pub fn marked(conn: &Connection, item: &Candidate, query: &str) -> String {
    let tokens = query::query_tokens(query)
        .and_then(|tokens| query::expand_query_tokens(conn, tokens))
        .and_then(|tokens| query::informative_tokens(conn, tokens))
        .map(|tokens| query::evidence_tokens(&tokens))
        .unwrap_or_default();
    let mut image = item.image();
    if image.content.chars().count() > MARKED_LIMIT {
        image.content = answering_stretch(&image.content, &tokens);
    }
    if tokens.is_empty() {
        return image.content;
    }
    held_marked(&image, &tokens).unwrap_or(image.content)
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
    let image = item.image();
    let marked = marked(conn, item, query);
    let text = if item.document.kind == "changelog" {
        &image.content
    } else {
        &marked
    };
    Ok(section(conn, item, text, &weights(conn, query), query)
        .map(|section| {
            window(
                &section.units,
                limit,
                &weights(conn, query),
                item.document.kind == "changelog",
            )
        })
        .unwrap_or_else(|| query::body_excerpt(&image.content, limit, &[], false)))
}
pub fn delivery_identity(conn: &Connection, item: &Candidate, query: &str) -> (String, String) {
    let mut key = item.document.path.clone();
    let mut revision = item.document.source_hash.clone();
    if item.document.layer != "procedural"
        && item.match_strength.as_deref() != Some("distinctive")
        && !(matches!(
            item.selection.as_deref(),
            Some("prompt-link" | "touch-link")
        ) && item.document.kind == "brain-task")
    {
        let image = item.image();
        let marked = marked(conn, item, query);
        let text = if item.document.kind == "changelog" {
            &image.content
        } else {
            &marked
        };
        if let Some(selected) = section(conn, item, text, &weights(conn, query), query) {
            let ordinary = quoted_section(
                &marked,
                &item.document.title,
                &weights(conn, query),
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
    if !item.document.conflicts.is_empty() || item.match_strength.as_deref() == Some("conflict") {
        revision = query::sha256(format!("recorded-conflict\0{revision}").as_bytes());
    }
    (key, revision)
}
pub fn linked_task_excerpt(content: &str, via: &str, limit: usize) -> (String, String) {
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
        .unwrap_or("");
    let heading = if !via.is_empty()
        && sections
            .get("Sources")
            .is_some_and(|text| text.contains(via))
        && !sections.get("Files").is_some_and(|text| text.contains(via))
    {
        "Sources"
    } else {
        "Files"
    };
    let grave = char::from(96);
    let pattern = regex::Regex::new(&format!("^- {grave}?([^{grave}\\n]+?){grave}?\\s*$")).ok();
    let entries = sections
        .get(heading)
        .into_iter()
        .flat_map(|text| text.lines())
        .filter_map(|line| {
            pattern
                .as_ref()
                .and_then(|pattern| pattern.captures(line.trim()))
                .map(|captures| captures[1].trim().to_string())
        })
        .collect::<Vec<_>>();
    let mut ordered = Vec::new();
    if entries.iter().any(|entry| entry == via) {
        ordered.push(via.to_string());
    }
    ordered.extend(entries.into_iter().rev().filter(|entry| entry != via));
    let mut parts = Vec::new();
    if !goal.is_empty() {
        parts.push(format!("Goal: {}.", collapse(goal)));
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
fn prepare_base(
    conn: &Connection,
    capsule: &Capsule,
    selected: &[Candidate],
    limits: &BTreeMap<String, usize>,
) -> Result<PreparedRender> {
    let mut prepared = PreparedRender::default();
    let query = capsule.0["query"].as_str().unwrap_or("");
    let term_weights = weights(conn, query);
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
            let held_content = prepared
                .images
                .get(path)
                .map(|image| image.content.as_str())
                .unwrap_or("");
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
            if linked {
                let (text, heading) = linked_task_excerpt(
                    held_content,
                    candidate.via.as_deref().unwrap_or(""),
                    limit,
                );
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
            let marked = if query.is_empty() {
                held_content.to_string()
            } else {
                marked(conn, candidate, query)
            };
            let text = if candidate.document.kind == "changelog" {
                held_content
            } else {
                &marked
            };
            let section = section(conn, candidate, text, &term_weights, query);
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
pub fn prepare(
    conn: &Connection,
    capsule: &Capsule,
    selected: &[Candidate],
) -> Result<PreparedRender> {
    let initial = prepare_base(conn, capsule, selected, &BTreeMap::new())?;
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
        prepare_base(conn, capsule, selected, &grown)
    }
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
    let lexical = prepare(conn, capsule, selected)?;
    capsule.1 = Some(lexical.clone());
    let query = capsule.0["query"].as_str().unwrap_or("");
    let term_weights = weights(conn, query);
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
            let held = marked(conn, item, query);
            let sources = {
                let mut statement=conn.prepare("SELECT ref_path FROM document_links WHERE path=?1 AND ref_kind='source' ORDER BY ref_path LIMIT 33")?;
                let rows =
                    statement.query_map([&item.document.path], |row| row.get::<_, String>(0))?;
                rows.collect::<std::result::Result<BTreeSet<_>, _>>()?
            };
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
                    revision = query::sha256(format!("recorded-conflict\0{revision}").as_bytes());
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
