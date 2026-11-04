use crate::{common::*, ownership};
use infra_core::{Result, RuntimeError, fs};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
const CONCEPTS: &[(&str, &str)] = &[
    (
        "selection",
        r"\b(select|selection|trigger|evidence-gated)\b",
    ),
    ("evidence", r"\bevidence\b"),
    ("scope", r"\bscope\b"),
    ("procedure", r"\bprocedure\b"),
    ("decisions", r"\bdecision\b"),
    ("verification", r"\b(verif(?:y|ication)|tests?)\b"),
    ("outputs", r"\boutput\b"),
    ("failure", r"\bfail(?:ure|ing|s)?\b"),
    ("boundaries", r"\b(boundar(?:y|ies)|sibling)\b"),
    ("negative-example", r"\b(bad|negative|invalid)\b"),
];
pub fn heading_slug(value: &str) -> String {
    let lowered = value.trim().to_ascii_lowercase();
    let reduced = lowered
        .chars()
        .filter(|value| {
            value.is_ascii_lowercase() || value.is_ascii_digit() || matches!(value, ' ' | '-')
        })
        .collect::<String>();
    expression(r"-+")
        .map(|regex| {
            regex
                .replace_all(
                    &reduced.split_whitespace().collect::<Vec<_>>().join("-"),
                    "-",
                )
                .trim_matches('-')
                .to_string()
        })
        .unwrap_or_default()
}
pub fn valid_falsifier(candidate: &Value) -> bool {
    let Some(falsifier) = candidate.get("falsifier") else {
        return true;
    };
    if falsifier.is_null() {
        return nonempty(&candidate["falsifier_absent"]);
    }
    if candidate.get("falsifier_absent").is_some()
        || !falsifier.is_object()
        || keys(falsifier)
            .iter()
            .any(|key| !["surface", "requires", "probes", "at_least"].contains(&key.as_str()))
        || !nonempty(&falsifier["surface"])
        || !["any", "all"].contains(&text(&falsifier["requires"]))
        || !falsifier
            .get("at_least")
            .map(|value| value.as_u64().is_some_and(|value| value >= 1))
            .unwrap_or(true)
    {
        return false;
    }
    falsifier["probes"].as_array().is_some_and(|probes| {
        !probes.is_empty()
            && probes.iter().all(|probe| {
                probe.is_object()
                    && keys(probe)
                        .iter()
                        .all(|key| ["paths", "pattern"].contains(&key.as_str()))
                    && string_list(&probe["paths"], false)
                    && probe
                        .get("pattern")
                        .map(|pattern| nonempty(pattern) && expression(text(pattern)).is_ok())
                        .unwrap_or(true)
            })
    })
}
pub fn references(directory: &Path, forbidden: &[String]) -> Vec<String> {
    let mut errors = vec![];
    let files = match std::fs::read_dir(directory) {
        Ok(entries) => entries
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|value| value == "md"))
            .map(|entry| entry.path())
            .collect::<Vec<_>>(),
        Err(_) => vec![],
    };
    if files.len() < 6 {
        errors.push(format!(
            "expected at least six reference catalogs, found {}",
            files.len()
        ));
    }
    let mut headings = BTreeMap::<String, BTreeSet<String>>::new();
    for path in files {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_string();
        let content = match fs::read_text(&path) {
            Ok(content) => content,
            Err(error) => {
                errors.push(format!("{name}: cannot read catalog: {error}"));
                continue;
            }
        };
        let mut anchors = BTreeSet::new();
        for line in content.lines() {
            let hashes = line.chars().take_while(|value| *value == '#').count();
            if (1..=6).contains(&hashes)
                && line
                    .as_bytes()
                    .get(hashes)
                    .is_some_and(u8::is_ascii_whitespace)
            {
                anchors.insert(heading_slug(line[hashes..].trim()));
            }
        }
        headings.insert(name.clone(), anchors);
        if content.split_whitespace().count() < 180 {
            errors.push(format!("{name}: reference catalog is a stub"));
        }
        for (concept, pattern) in CONCEPTS {
            if !matching(&format!("(?i){pattern}"), &content) {
                errors.push(format!("{name}: missing contract concept {concept}"));
            }
        }
        if matching(r"(?i)\{\{[^}]+\}\}|\[Stack\]", &content) {
            errors.push(format!("{name}: unresolved placeholder"));
        }
        for term in forbidden {
            if !term.trim().is_empty()
                && matching(
                    &format!(r"(?i)\b{}\b", regex::escape(term.trim())),
                    &content,
                )
            {
                errors.push(format!(
                    "{name}: forbidden source-stack term {:?}",
                    term.trim()
                ));
            }
        }
    }
    let registry = match read_object(&directory.join("candidate-registry.json")) {
        Ok(registry) => registry,
        Err(error) => {
            errors.push(format!(
                "candidate-registry.json: missing or invalid: {error}"
            ));
            Value::Null
        }
    };
    if registry["schema_version"] != "1.1" || !registry["candidates"].is_array() {
        errors.push("candidate-registry.json: invalid schema".into());
    } else {
        let candidates = registry["candidates"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if candidates.is_empty() {
            errors.push("candidate-registry.json: candidates must list at least one entry".into());
        }
        let mut seen = BTreeSet::new();
        for (index, candidate) in candidates.iter().enumerate() {
            let allowed = [
                "id",
                "catalog",
                "category",
                "mode",
                "roles",
                "escalates_on_rejection",
                "falsifier",
                "falsifier_absent",
                "golden",
            ];
            let roles = candidate
                .get("roles")
                .map(|value| value.is_null() || string_list(value, false))
                .unwrap_or(true);
            if !candidate.is_object()
                || !["id", "catalog", "category", "mode"]
                    .iter()
                    .all(|key| candidate.get(key).is_some())
                || keys(candidate)
                    .iter()
                    .any(|key| !allowed.contains(&key.as_str()))
                || !roles
                || !candidate
                    .get("escalates_on_rejection")
                    .map(Value::is_boolean)
                    .unwrap_or(true)
                || !candidate
                    .get("golden")
                    .map(Value::is_boolean)
                    .unwrap_or(true)
                || !valid_falsifier(candidate)
                || !["static", "family", "runtime-fixed"].contains(&text(&candidate["mode"]))
                || !["id", "catalog", "category"]
                    .iter()
                    .all(|key| nonempty(&candidate[key]))
            {
                errors.push(format!(
                    "candidate-registry.json: candidates[{index}] is invalid"
                ));
                continue;
            }
            let id = text(&candidate["id"]);
            if !seen.insert(id.to_string()) {
                errors.push(format!("candidate-registry.json: duplicate candidate {id}"));
            }
            match text(&candidate["catalog"]).split_once('#') {
                Some((file, anchor))
                    if headings
                        .get(file)
                        .is_some_and(|anchors| anchors.contains(anchor)) => {}
                _ => errors.push(format!(
                    "candidate-registry.json: catalog anchor does not resolve for {id}"
                )),
            }
        }
    }
    errors
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
struct Chunk {
    name: String,
    meta: Value,
    sections: BTreeMap<String, String>,
    title_words: BTreeSet<String>,
}
fn section_map(body: &str) -> BTreeMap<String, String> {
    let mut sections = BTreeMap::new();
    let mut name = None;
    let mut current = String::new();
    for line in body.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            if let Some(name) = name.take() {
                sections.insert(name, current.trim().to_string());
            }
            name = Some(heading.trim().to_string());
            current.clear();
        } else {
            current.push_str(line);
            current.push('\n');
        }
    }
    if let Some(name) = name {
        sections.insert(name, current.trim().to_string());
    }
    sections
}
fn parse_chunk(path: &Path) -> Result<Chunk> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_string();
    let content = fs::read_text(path)?;
    let content = content
        .strip_prefix("---\n")
        .ok_or_else(|| RuntimeError::invalid("no frontmatter block"))?;
    let (raw, body) = content
        .split_once("\n---\n")
        .ok_or_else(|| RuntimeError::invalid("no closing frontmatter block"))?;
    let meta = infra_core::domain::json::parse(raw)?;
    if !meta.is_object() {
        return Err(RuntimeError::invalid("chunk metadata must be object"));
    }
    let sections = section_map(body);
    for required in ["Durable Context", "Consequences", "Verification"] {
        if !sections
            .get(required)
            .is_some_and(|value| !value.is_empty())
        {
            return Err(RuntimeError::invalid(format!(
                "missing or empty section: {required}"
            )));
        }
    }
    let title_words = expression(r"[a-zA-Z]{3,}")?
        .find_iter(text(&meta["title"]))
        .map(|word| word.as_str().to_ascii_lowercase())
        .collect();
    Ok(Chunk {
        name,
        meta,
        sections,
        title_words,
    })
}
fn skeleton(content: &str, title: &BTreeSet<String>) -> String {
    let content = content.to_lowercase();
    let content = expression(r"`[^`]+`")
        .map(|regex| regex.replace_all(&content, " x ").to_string())
        .unwrap_or(content);
    let content = expression(r"\S*[/\\]\S*")
        .map(|regex| regex.replace_all(&content, " p ").to_string())
        .unwrap_or(content);
    let content = expression(r"\bl?\d+(?:-l?\d+)?\b")
        .map(|regex| regex.replace_all(&content, " n ").to_string())
        .unwrap_or(content);
    expression(r"[a-z]+")
        .map(|regex| {
            regex
                .find_iter(&content)
                .map(|hit| hit.as_str())
                .filter(|word| !title.contains(*word))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}
pub fn memory_content(bank: &Path, target: Option<&Path>) -> Vec<Diagnostic> {
    let mut items = vec![];
    let directory = bank.join("chunks");
    if !directory.is_dir() {
        diag(
            &mut items,
            "MEMORY_BANK_UNREADABLE",
            "chunks is not a directory",
        );
        return items;
    }
    let mut files = std::fs::read_dir(&directory)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("MEM-") && name.ends_with(".md"))
        })
        .collect::<Vec<_>>();
    files.sort();
    let mut chunks = vec![];
    for path in files {
        match parse_chunk(&path) {
            Ok(chunk) => chunks.push(chunk),
            Err(error) => diag(
                &mut items,
                "MEMORY_CHUNK_UNREADABLE",
                format!("{}: {error}", path.display()),
            ),
        }
    }
    if chunks.is_empty() {
        diag(
            &mut items,
            "MEMORY_BANK_EMPTY",
            "bank seeds no readable chunks",
        );
        return items;
    }
    let range = match expression(r"([\w./\\-]+\.\w{1,12}):L(\d+)(?:\s*-\s*L?(\d+))?") {
        Ok(value) => value,
        Err(error) => {
            diag(&mut items, "MEMORY_CHUNK_UNREADABLE", error.to_string());
            return items;
        }
    };
    for chunk in &chunks {
        let durable = &chunk.sections["Durable Context"];
        let body = chunk
            .sections
            .values()
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
        let label = &chunk.name;
        if !matching(
            r"(?i)\b(must|must not|never|only|cannot|forbidden|required|requires|refuse[sd]?|reject[sed]*|block[sed]*|remain[s]?|allowed|until|unless|when|no\b)",
            durable,
        ) || !matching(
            r#"`[^`]+`|\b[A-Z][A-Z0-9_]{2,}\b|\b[a-z]+[A-Z]\w+\b|\b[A-Z]\w+[A-Z]\w*\b|\b\d+\b|"[^"]+"|'[^']+'|\S+:L\d+"#,
            durable,
        ) {
            diag(
                &mut items,
                "MEMORY_RULE_MISSING",
                format!("{label}: Durable Context needs behavioral modality and concrete anchor"),
            );
        }
        for phrase in [
            "re-open the cited sources",
            "apply the matching generated specialist",
            "authority boundary recorded by",
            "do not infer missing behavior",
            "instead of promoting an assumption",
            "recompute authority from:",
        ] {
            if body.to_lowercase().contains(phrase) {
                diag(
                    &mut items,
                    "MEMORY_GENERIC_PHRASE",
                    format!("{label}: known template phrase {phrase:?}"),
                );
            }
        }
        let mut seen = BTreeSet::new();
        for source in texts(&chunk.meta["sources"]) {
            if !seen.insert(source.clone()) {
                diag(
                    &mut items,
                    "MEMORY_SOURCE_DUPLICATE",
                    format!("{label}: duplicate source {source}"),
                );
                continue;
            }
            if source.starts_with("https://") || source.starts_with("http://") {
                continue;
            }
            if let Some(target) = target
                && safe_source(target, &source)
                    .map(|path| !path.exists())
                    .unwrap_or(true)
            {
                diag(
                    &mut items,
                    "MEMORY_SOURCE_MISSING",
                    format!("{label}: source does not exist: {source}"),
                );
            }
            let explanations = chunk.sections["Verification"]
                .lines()
                .filter(|line| line.contains(&source))
                .collect::<Vec<_>>();
            let mut residues = vec![];
            for line in explanations {
                let line = range.replace_all(line, " ");
                let line = expression(r"\S*[/\\]\S*")
                    .map(|regex| regex.replace_all(&line, " ").to_string())
                    .unwrap_or(line.to_string());
                if let Ok(words) = expression(r"[a-zA-Z]{3,}") {
                    residues.extend(words.find_iter(&line).map(|hit| hit.as_str().to_string()));
                }
            }
            if residues.len() < 4 {
                diag(
                    &mut items,
                    "MEMORY_SOURCE_UNEXPLAINED",
                    format!("{label}: Verification never explains what {source} proves"),
                );
            }
        }
        if let Some(target) = target {
            for matched in range.captures_iter(&body) {
                let source = match ownership::confined(target, &matched[1]) {
                    Ok(source) => source,
                    Err(_) => continue,
                };
                if !source.is_file() {
                    continue;
                }
                if let Ok(raw) = fs::read_regular(&source) {
                    let total = String::from_utf8_lossy(&raw).lines().count();
                    let start = matched[2].parse::<usize>().unwrap_or(0);
                    let end = matched
                        .get(3)
                        .and_then(|value| value.as_str().parse::<usize>().ok())
                        .unwrap_or(start);
                    if start < 1 || end < start || end > total {
                        diag(
                            &mut items,
                            "MEMORY_VERIFICATION_RANGE",
                            format!(
                                "{label}: {}:L{start}-L{end} outside {total} lines",
                                &matched[1]
                            ),
                        );
                    }
                }
            }
        }
        let identity = format!(
            "{} {}",
            text(&chunk.meta["title"]),
            texts(&chunk.meta["tags"]).join(" ")
        );
        if matching(r"(?i)contradict|conflict|versus|disagree", &identity) {
            if chunk.meta["status"] != "needs-review" {
                diag(
                    &mut items,
                    "MEMORY_CONTRADICTION_SETTLED",
                    format!("{label}: unresolved contradiction must remain needs-review"),
                );
            }
            if range.captures_iter(durable).count() < 2
                || !matching(r"(?i)contradict|conflict|versus|disagree", durable)
                || !matching(
                    r"(?i)current (?:safe )?(?:operating |operational )?rule|operate as|treat as|until (?:it is )?resolved|owner|resolution|needs review",
                    durable,
                )
            {
                diag(
                    &mut items,
                    "MEMORY_CONTRADICTION_IMPLICIT",
                    format!(
                        "{label}: contradiction needs both claims, cited ranges, operating rule and resolution owner"
                    ),
                );
            }
        }
    }
    for section in ["Durable Context", "Consequences"] {
        let mut seen = BTreeMap::new();
        for chunk in &chunks {
            let skeleton = skeleton(&chunk.sections[section], &chunk.title_words);
            if skeleton.len() < 40 {
                continue;
            }
            if let Some(previous) = seen.get(&skeleton) {
                diag(
                    &mut items,
                    "MEMORY_BODY_TEMPLATED",
                    format!(
                        "{}: {section} shares erased skeleton with {previous}",
                        chunk.name
                    ),
                );
            } else {
                seen.insert(skeleton, chunk.name.clone());
            }
        }
    }
    items
}
