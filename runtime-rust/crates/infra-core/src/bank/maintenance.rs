use super::{
    BankService, codec, promotion,
    publication::{self, PublicationFiles},
    validation,
};
use crate::{
    Result, RuntimeError,
    domain::{self, json},
    fs,
};
use serde_json::{Value, json as object};
use std::collections::{BTreeMap, BTreeSet};

const STOPWORDS: &str = "a an and are as at be but by for from has have if in into is it its of on or that the their then there these this to was were will with when where which while";
// CPython 3.14 str.isdigit(), frozen from Unicode 16.0.0.
// Decimal and Digit numeric types only; Numeric fractions and Roman numerals
// are excluded. Keep this independent of the regex crate's Unicode release.
const PYTHON_DIGIT_RANGES: &[(u32, u32)] = &[
    (0x30, 0x39),
    (0xB2, 0xB3),
    (0xB9, 0xB9),
    (0x660, 0x669),
    (0x6F0, 0x6F9),
    (0x7C0, 0x7C9),
    (0x966, 0x96F),
    (0x9E6, 0x9EF),
    (0xA66, 0xA6F),
    (0xAE6, 0xAEF),
    (0xB66, 0xB6F),
    (0xBE6, 0xBEF),
    (0xC66, 0xC6F),
    (0xCE6, 0xCEF),
    (0xD66, 0xD6F),
    (0xDE6, 0xDEF),
    (0xE50, 0xE59),
    (0xED0, 0xED9),
    (0xF20, 0xF29),
    (0x1040, 0x1049),
    (0x1090, 0x1099),
    (0x1369, 0x1371),
    (0x17E0, 0x17E9),
    (0x1810, 0x1819),
    (0x1946, 0x194F),
    (0x19D0, 0x19DA),
    (0x1A80, 0x1A89),
    (0x1A90, 0x1A99),
    (0x1B50, 0x1B59),
    (0x1BB0, 0x1BB9),
    (0x1C40, 0x1C49),
    (0x1C50, 0x1C59),
    (0x2070, 0x2070),
    (0x2074, 0x2079),
    (0x2080, 0x2089),
    (0x2460, 0x2468),
    (0x2474, 0x247C),
    (0x2488, 0x2490),
    (0x24EA, 0x24EA),
    (0x24F5, 0x24FD),
    (0x24FF, 0x24FF),
    (0x2776, 0x277E),
    (0x2780, 0x2788),
    (0x278A, 0x2792),
    (0xA620, 0xA629),
    (0xA8D0, 0xA8D9),
    (0xA900, 0xA909),
    (0xA9D0, 0xA9D9),
    (0xA9F0, 0xA9F9),
    (0xAA50, 0xAA59),
    (0xABF0, 0xABF9),
    (0xFF10, 0xFF19),
    (0x104A0, 0x104A9),
    (0x10A40, 0x10A43),
    (0x10D30, 0x10D39),
    (0x10D40, 0x10D49),
    (0x10E60, 0x10E68),
    (0x11052, 0x1105A),
    (0x11066, 0x1106F),
    (0x110F0, 0x110F9),
    (0x11136, 0x1113F),
    (0x111D0, 0x111D9),
    (0x112F0, 0x112F9),
    (0x11450, 0x11459),
    (0x114D0, 0x114D9),
    (0x11650, 0x11659),
    (0x116C0, 0x116C9),
    (0x116D0, 0x116E3),
    (0x11730, 0x11739),
    (0x118E0, 0x118E9),
    (0x11950, 0x11959),
    (0x11BF0, 0x11BF9),
    (0x11C50, 0x11C59),
    (0x11D50, 0x11D59),
    (0x11DA0, 0x11DA9),
    (0x11F50, 0x11F59),
    (0x16130, 0x16139),
    (0x16A60, 0x16A69),
    (0x16AC0, 0x16AC9),
    (0x16B50, 0x16B59),
    (0x16D70, 0x16D79),
    (0x1CCF0, 0x1CCF9),
    (0x1D7CE, 0x1D7FF),
    (0x1E140, 0x1E149),
    (0x1E2F0, 0x1E2F9),
    (0x1E4F0, 0x1E4F9),
    (0x1E5F1, 0x1E5FA),
    (0x1E950, 0x1E959),
    (0x1F100, 0x1F10A),
    (0x1FBF0, 0x1FBF9),
];
fn python_digit(character: char) -> bool {
    let value = character as u32;
    let position = PYTHON_DIGIT_RANGES.partition_point(|(_, last)| *last < value);
    PYTHON_DIGIT_RANGES
        .get(position)
        .is_some_and(|(first, _)| *first <= value)
}
fn duplicate_tokens(text: &str) -> Vec<String> {
    static WORDS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = WORDS.get_or_init(|| {
        regex::Regex::new(r"[\p{L}\p{N}]+").expect("constant Python alphanumeric token grammar")
    });
    let text = crate::query::folded(text);
    pattern
        .find_iter(&text)
        .map(|token| token.as_str().to_string())
        .filter(|word| {
            (word.chars().count() >= 2 || word.chars().all(python_digit))
                && !STOPWORDS.split_whitespace().any(|stop| stop == word)
        })
        .collect()
}
fn shingles(text: &str) -> BTreeSet<Vec<String>> {
    let words = duplicate_tokens(text);
    if words.len() < 3 {
        if words.is_empty() {
            BTreeSet::new()
        } else {
            BTreeSet::from([words])
        }
    } else {
        words.windows(3).map(|window| window.to_vec()).collect()
    }
}

pub fn similarity(left: &str, right: &str) -> f64 {
    let a = shingles(left);
    let b = shingles(right);
    if a.is_empty() || b.is_empty() {
        0.0
    } else {
        a.intersection(&b).count() as f64 / a.union(&b).count() as f64
    }
}
pub fn duplicate_ratio(config: &Value) -> f64 {
    let value = config["bank_duplicate_ratio"]
        .as_f64()
        .or_else(|| {
            config["bank_duplicate_ratio"]
                .as_bool()
                .map(|value| if value { 1.0 } else { 0.0 })
        })
        .or_else(|| {
            config["bank_duplicate_ratio"]
                .as_str()
                .and_then(|text| text.trim().parse().ok())
        })
        .unwrap_or(0.6);
    if value > 0.0 && value <= 1.0 {
        value
    } else {
        0.6
    }
}
pub fn near_duplicate(
    content: &str,
    chunks: &[(String, String)],
    threshold: f64,
) -> Option<String> {
    let mut best = None;
    let mut score = threshold;
    for (id, text) in chunks {
        let current = similarity(content, text);
        if current >= score {
            score = current;
            best = Some(id.clone());
        }
    }
    best
}
pub fn active_texts(service: &BankService) -> Result<Vec<(String, String)>> {
    let mut files = PublicationFiles::new(&service.context.roots.state)?;
    let mut out = Vec::new();
    for path in codec::paths(service)? {
        let Ok(chunk) = codec::read(&mut files, &path) else {
            continue;
        };
        if chunk.metadata["status"] == "active"
            && let Some(id) = chunk.metadata["id"].as_str()
        {
            out.push((
                id.into(),
                format!(
                    "{}\n{}",
                    chunk.metadata["title"].as_str().unwrap_or(""),
                    chunk.body
                ),
            ));
        }
    }
    Ok(out)
}

pub fn audit(service: &BankService, provenance: bool) -> Result<Value> {
    let mut files = PublicationFiles::new(&service.context.roots.state)?;
    let mut total = 0;
    let mut changed = Vec::new();
    let mut overdue = Vec::new();
    let mut undigested = Vec::new();
    let mut automatic = Vec::new();
    let mut external = Vec::new();
    let mut instructions = Vec::new();
    for path in codec::paths(service)? {
        let Ok(chunk) = codec::read(&mut files, &path) else {
            continue;
        };
        total += 1;
        let m = &chunk.metadata;
        if m["status"] != "active" {
            continue;
        }
        let tags = codec::strings(m, "tags", true).unwrap_or_default();
        if provenance && tags.iter().any(|tag| tag == "auto-promoted") {
            automatic.push(object!({"chunk":path,"id":m["id"],"created":m["created"],"last_verified":m["last_verified"]}));
        }
        let sources = codec::strings(m, "sources", true).unwrap_or_default();
        let urls = sources
            .iter()
            .filter(|source| source.starts_with("http://") || source.starts_with("https://"))
            .cloned()
            .collect::<Vec<_>>();
        if provenance && !urls.is_empty() {
            external.push(object!({"chunk":path,"id":m["id"],"sources":urls}));
        }
        if provenance
            && promotion::instruction_like(m["title"].as_str().unwrap_or(""), &chunk.body)?
        {
            instructions.push(path.clone());
        }
        let digests = m["source_digests"].as_array();
        if digests.is_none_or(Vec::is_empty) {
            if !domain::digestible_sources(&m["sources"]).is_empty() {
                undigested.push(path.clone());
            }
        } else {
            for row in digests.into_iter().flatten() {
                let source = row["path"].as_str().unwrap_or("");
                let state =
                    match domain::source_fingerprints(&service.context.roots, &[source.into()]) {
                        Ok(rows)
                            if rows
                                .first()
                                .is_some_and(|current| current["sha256"] == row["sha256"]) =>
                        {
                            None
                        }
                        Ok(_) => Some("changed"),
                        Err(_) => Some("missing"),
                    };
                if let Some(state) = state {
                    changed.push(
                        object!({"chunk":path,"id":m["id"],"source":row["path"],"state":state}),
                    );
                }
            }
        }
        if m["review_after"]
            .as_str()
            .and_then(|text| codec::date(text).ok())
            .is_some_and(|date| date < codec::today())
        {
            overdue.push(object!({"chunk":path,"id":m["id"],"review_after":m["review_after"]}));
        }
    }
    let texts = active_texts(service)?;
    let threshold = duplicate_ratio(&service.config()?.value);
    let mut duplicates = Vec::new();
    for (index, (left, a)) in texts.iter().enumerate() {
        for (right, b) in &texts[index + 1..] {
            let score = similarity(a, b);
            if score >= threshold {
                duplicates.push(
                    object!({"left":left,"right":right,"similarity":(score*1000.0).round()/1000.0}),
                );
            }
        }
    }
    let mut result = object!({"chunks":total,"source_changed":changed,"overdue_review":overdue,"undigested":undigested,"duplicates":duplicates});
    if provenance {
        result["provenance"] = object!({"auto_promoted":automatic,"external_sources":external,"instruction_like":instructions});
    }
    Ok(result)
}

pub fn repository_errors(service: &BankService, freshness: bool) -> Result<Vec<String>> {
    super::repository::repository_errors(service, freshness)
}
fn evidence_entries(path: &std::path::Path) -> Vec<std::fs::DirEntry> {
    if !crate::fs::admit_directory(path).unwrap_or(false) {
        return Vec::new();
    }
    std::fs::read_dir(path)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(std::result::Result::ok)
        .collect()
}
fn remove_evidence(entry: &std::fs::DirEntry) -> usize {
    let Some(name) = entry.file_name().to_str().map(String::from) else {
        return 0;
    };
    if name.len() != 69
        || !name.ends_with(".json")
        || !name[..64]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return 0;
    }
    if entry.file_type().is_ok_and(|kind| kind.is_file())
        && std::fs::remove_file(entry.path()).is_ok()
    {
        1
    } else {
        0
    }
}
pub fn prune_result_evidence(service: &BankService) -> usize {
    let Ok(brain) = service.brain() else { return 0 };
    let Ok(records) = brain.records(false) else {
        return 0;
    };
    let active = records
        .iter()
        .filter_map(|image| image.record["id"].as_str())
        .collect::<BTreeSet<_>>();
    let mut removed = 0;
    for folder in evidence_entries(&brain.roots().brain_root().join("control/result-attempts")) {
        if !folder.file_type().is_ok_and(|kind| kind.is_dir()) {
            removed += remove_evidence(&folder);
            continue;
        }
        if folder
            .file_name()
            .to_str()
            .is_some_and(|id| active.contains(id))
        {
            continue;
        }
        for entry in evidence_entries(&folder.path()) {
            removed += remove_evidence(&entry);
        }
        let _ = std::fs::remove_dir(folder.path());
    }
    for entry in evidence_entries(
        &brain
            .roots()
            .brain_root()
            .join("control/result-learning-images"),
    ) {
        let Ok(Some(value)) = crate::result_store::read_json(&entry.path(), 4096) else {
            continue;
        };
        if value["record_id"]
            .as_str()
            .is_some_and(|id| !active.contains(id))
        {
            removed += remove_evidence(&entry);
        }
    }
    removed
}
fn relative(service: &BankService, path: &std::path::Path) -> Result<String> {
    path.strip_prefix(&service.context.roots.state)
        .ok()
        .and_then(|path| path.to_str())
        .map(|s| s.replace('\\', "/"))
        .ok_or_else(|| RuntimeError::policy("Authority path escapes state"))
}
pub fn compact(service: &BankService, only: Option<&str>) -> Result<Value> {
    let brain = service.brain()?;
    let _guard = brain.lock()?;
    let derived = super::repository::index_errors(service)?
        .into_iter()
        .collect::<BTreeSet<_>>();
    let errors = repository_errors(service, false)?
        .into_iter()
        .filter(|error| !derived.contains(error))
        .collect::<Vec<_>>();
    if !errors.is_empty() {
        return Err(RuntimeError::invalid(format!(
            "Compaction refused because active records are invalid ({} problem(s); see native memory validate)",
            errors.len()
        )));
    }
    let mut moves = Vec::new();
    let mut count = 0;
    for image in brain.records(false)? {
        let record = &image.record;
        let kind = record["type"].as_str().unwrap_or("");
        if !domain::is_terminal(kind, record["status"].as_str().unwrap_or(""))
            && record["superseded_by"].is_null()
        {
            continue;
        }
        if kind == "finding"
            && record["external_id"]
                .as_str()
                .is_some_and(|id| id.starts_with("CONSOLIDATED-"))
            && record["authority"] != "verified"
        {
            continue;
        }
        if only.is_some_and(|wanted| record["external_id"].as_str() != Some(wanted)) {
            continue;
        }
        let destination = brain.roots().brain_root().join("archive").join(kind).join(
            image
                .path
                .file_name()
                .ok_or_else(|| RuntimeError::invalid("Record filename unavailable"))?,
        );
        if std::fs::symlink_metadata(&destination).is_ok() {
            return Err(RuntimeError::conflict(format!(
                "Archive destination already exists: {}",
                destination.display()
            )));
        }
        moves.push((image.path, destination));
        count += 1;
        let handoff = brain.handoff_path(record["id"].as_str().unwrap_or(""))?;
        if std::fs::symlink_metadata(&handoff).is_ok() {
            let destination = brain.roots().brain_root().join("archive/handoffs").join(
                handoff
                    .file_name()
                    .ok_or_else(|| RuntimeError::invalid("Handoff filename unavailable"))?,
            );
            if std::fs::symlink_metadata(&destination).is_ok() {
                return Err(RuntimeError::conflict(format!(
                    "Archive destination already exists: {}",
                    destination.display()
                )));
            }
            moves.push((handoff, destination));
        }
    }
    let renames = moves
        .iter()
        .map(|(source, destination)| {
            Ok((relative(service, source)?, relative(service, destination)?))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let bank = brain.roots().state.join("memory-bank");
    let bank_exists = fs::admit_directory(&bank)?;
    let chunks = bank.join("chunks");
    let mut rewrites = Vec::new();
    if !renames.is_empty() && fs::admit_directory(&chunks)? {
        let mut paths = std::fs::read_dir(&chunks)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        paths.sort();
        for path in paths {
            if path.extension().is_none_or(|extension| extension != "md") {
                continue;
            }
            let Ok(raw) = fs::read_text(&path) else {
                continue;
            };
            let Ok((mut metadata, body)) = domain::parse_markdown_record(&raw) else {
                continue;
            };
            let Some(sources) = metadata["sources"].as_array_mut() else {
                continue;
            };
            let mut changed = false;
            for source in sources {
                let Some(value) = source.as_str() else {
                    continue;
                };
                let (head, fragment) = value
                    .split_once('#')
                    .map(|(head, tail)| (head, Some(tail)))
                    .unwrap_or((value, None));
                if let Some(destination) = renames.get(head) {
                    *source = match fragment {
                        Some(fragment) => format!("{destination}#{fragment}"),
                        None => destination.clone(),
                    }
                    .into();
                    changed = true;
                }
            }
            if changed {
                if let Some(rows) = metadata["source_digests"].as_array_mut() {
                    for row in rows {
                        if let Some(destination) =
                            row["path"].as_str().and_then(|path| renames.get(path))
                        {
                            row["path"] = destination.clone().into();
                        }
                    }
                }
                rewrites.push((
                    path,
                    domain::render_markdown_record(&metadata, &body)?.into_bytes(),
                ));
            }
        }
    }
    let before = if bank_exists {
        validation::validate_bank(service)?
    } else {
        Vec::new()
    };
    let snapshot = fs::snapshot(
        moves
            .iter()
            .flat_map(|(a, b)| [a.clone(), b.clone()])
            .chain(rewrites.iter().map(|(path, _)| path.clone()))
            .chain([
                brain.roots().brain_root().join("indexes/active.json"),
                brain.roots().brain_root().join("indexes/archive.json"),
            ]),
    )?;
    let effect = (|| -> Result<()> {
        for (source, destination) in &moves {
            fs::admit_descendant(&brain.roots().brain_root(), destination)?;
            if let Some(parent) = destination.parent() {
                fs::ensure_directory(parent, false)?;
            }
            std::fs::rename(source, destination)?;
        }
        for (path, raw) in &rewrites {
            fs::atomic_write(path, raw)?;
        }
        brain.rebuild_indexes()?;
        let errors = repository_errors(service, false)?;
        if !errors.is_empty() {
            return Err(RuntimeError::invalid(format!(
                "Archive validation failed after compaction: {}",
                errors.join("; ")
            )));
        }
        if bank_exists {
            let errors = validation::write_errors(service, &before, &[])?;
            if !errors.is_empty() {
                return Err(RuntimeError::invalid(format!(
                    "Memory Bank validation failed after compaction: {}",
                    errors.join("; ")
                )));
            }
        }
        Ok(())
    })();
    if let Err(error) = effect {
        fs::restore(&snapshot)?;
        return Err(error);
    }
    prune_result_evidence(service);
    Ok(object!({"moved":count}))
}
pub fn consolidate(service: &BankService, apply: bool, ticket: Option<&str>) -> Result<Value> {
    super::consolidation::consolidate(service, apply, ticket)
}

pub fn auto_consolidate(service: &BankService) -> Result<Value> {
    let empty = || object!({"enabled":false,"created":0,"updated":0,"superseded":0,"blocked":0,"tickets":0,"error":null});
    if !service.config()?.enabled("automatic_consolidation") {
        return Ok(empty());
    }
    match consolidate(service, true, None) {
        Ok(report) => Ok(
            object!({"enabled":true,"created":report["created"],"updated":report["updated"],"superseded":report["superseded"],"blocked":report["blocked"],"tickets":report["tickets"].as_array().map(Vec::len).unwrap_or(0),"error":null}),
        ),
        Err(error) => {
            let mut result = empty();
            result["enabled"] = true.into();
            result["error"] = error.to_string().into();
            Ok(result)
        }
    }
}
pub fn auto_compact(service: &BankService) -> Result<Value> {
    let mut result = object!({"enabled":false,"moved":0,"pending":0,"error":null});
    let config = service.config()?;
    if !config.enabled("automatic_compaction") {
        return Ok(result);
    }
    result["enabled"] = true.into();
    let Some(threshold) = config.value["compaction_threshold"]
        .as_u64()
        .filter(|n| *n > 0)
    else {
        result["error"] = "compaction_threshold must be a positive integer".into();
        return Ok(result);
    };
    let pending = service
        .brain()?
        .records(false)?
        .iter()
        .filter(|image| {
            let r = &image.record;
            (domain::is_terminal(
                r["type"].as_str().unwrap_or(""),
                r["status"].as_str().unwrap_or(""),
            ) || !r["superseded_by"].is_null())
                && !(r["type"] == "finding"
                    && r["external_id"]
                        .as_str()
                        .is_some_and(|id| id.starts_with("CONSOLIDATED-"))
                    && r["authority"] != "verified")
        })
        .count();
    result["pending"] = object!(pending);
    if pending < threshold as usize {
        return Ok(result);
    }
    match compact(service, None) {
        Ok(report) => {
            result["moved"] = report["moved"].clone();
            result["pending"] = 0.into();
        }
        Err(error) => result["error"] = error.to_string().into(),
    }
    Ok(result)
}
pub fn retry_pending(service: &BankService) -> Result<Value> {
    let empty = || object!({"enabled":false,"promoted":[],"failed":[],"blocked":[],"closed":[],"skipped":0});
    let marker = format!(
        "project-brain/local/promotion-retry-{}.json",
        publication::sha256(service.context.actor.as_bytes())
    );
    fn due(files: &mut PublicationFiles, path: &str) -> bool {
        files
            .read(path, 512)
            .ok()
            .flatten()
            .and_then(|raw| {
                std::str::from_utf8(&raw)
                    .ok()
                    .and_then(|text| json::parse(text).ok())
            })
            .is_some_and(|value| {
                let exact = value.as_object().is_some_and(|map| {
                    map.len() == 2
                        && map.contains_key("profile")
                        && map.contains_key("next_attempt")
                        || map.len() == 3
                            && map.contains_key("profile")
                            && map.contains_key("next_attempt")
                            && map.contains_key("after_record")
                });
                exact
                    && value["profile"] == "automatic-promotion-retry-v1"
                    && value["next_attempt"].as_u64().is_some_and(|at| {
                        at <= domain::clock::proof_now().timestamp().max(0) as u64
                    })
                    && (value["after_record"].is_null()
                        || value["after_record"].as_str().is_some_and(domain::is_uuid4))
            })
    }
    let mut files = PublicationFiles::new(&service.context.roots.state)?;
    if !due(&mut files, &marker) {
        return Ok(empty());
    }
    let _guard = service.brain()?.lock()?;
    if !due(&mut files, &marker) {
        return Ok(empty());
    }
    promotion::auto_promote(service, 1, Some(1), None)
}

#[cfg(test)]
mod duplicate_unicode_tests {
    use super::{duplicate_tokens, python_digit, similarity};

    #[test]
    fn combining_marks_and_join_controls_do_not_join_python_word_tokens() {
        assert_eq!(
            duplicate_tokens("cafe\u{0301} route"),
            vec!["cafe", "route"]
        );
        assert_eq!(
            duplicate_tokens("alpha\u{200d} beta"),
            vec!["alpha", "beta"]
        );
        assert_eq!(duplicate_tokens("a\u{0301} alpha"), vec!["alpha"]);
        assert_eq!(similarity("cafe\u{0301} route", "cafe route"), 1.0);
        assert_eq!(similarity("alpha\u{200d} beta", "alpha beta"), 1.0);
    }

    #[test]
    fn single_character_numeric_tokens_follow_digit_not_numeric_type() {
        for character in [
            '0',
            '\u{0660}',
            '\u{00b2}',
            '\u{1369}',
            '\u{2460}',
            '\u{1ccf0}',
        ] {
            assert!(
                python_digit(character),
                "expected Python digit U+{:04X}",
                character as u32
            );
        }
        for character in ['\u{00bd}', '\u{2163}', '\u{2469}'] {
            assert!(
                !python_digit(character),
                "unexpected Python digit U+{:04X}",
                character as u32
            );
            let text = character.to_string();
            assert!(duplicate_tokens(&text).is_empty());
            assert_eq!(similarity(&text, &text), 0.0);
        }
        assert_eq!(duplicate_tokens("\u{00b2}"), vec!["\u{00b2}"]);
        assert_eq!(duplicate_tokens("\u{1369}"), vec!["\u{1369}"]);
        assert_eq!(duplicate_tokens("1_2"), vec!["1", "2"]);
    }

    #[test]
    fn error_code_digits_remain_distinct_duplicate_evidence() {
        assert_eq!(similarity("timeout 502 retry", "timeout 503 retry"), 0.0);
        assert_eq!(similarity("timeout 502 retry", "timeout 502 retry"), 1.0);
    }
}
