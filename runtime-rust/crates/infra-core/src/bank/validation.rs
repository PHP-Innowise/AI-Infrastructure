use super::{
    BankService, codec,
    publication::{PLAN_LIMIT, PublicationFiles},
};
use crate::{
    Result, RuntimeError,
    domain::{self},
    fs,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

const REQUIRED: [&str; 12] = [
    "id",
    "title",
    "type",
    "status",
    "scope",
    "tags",
    "created",
    "last_verified",
    "review_after",
    "sources",
    "supersedes",
    "superseded_by",
];
const OPTIONAL: [&str; 3] = ["valid_from", "valid_to", "source_digests"];
const TYPES: [&str; 7] = [
    "architecture",
    "constraint",
    "convention",
    "decision",
    "domain",
    "integration",
    "operations",
];
const STATUSES: [&str; 4] = ["active", "archived", "needs-review", "superseded"];
const INDEX: &str = "memory-bank/INDEX.md";

fn invalid(message: impl Into<String>) -> RuntimeError {
    RuntimeError::invalid(message)
}

/// A list of non-empty strings, refused in the retired validator's words.
/// An item of blanks stays refused, as this runtime has refused it.
fn string_list(value: &Value, key: &str, allow_empty: bool) -> Result<Vec<String>> {
    let items = value[key]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .map(|item| {
                    item.as_str()
                        .filter(|text| !text.trim().is_empty())
                        .map(String::from)
                })
                .collect::<Option<Vec<_>>>()
        })
        .ok_or_else(|| invalid(format!("{key} must be a list of non-empty strings")))?;
    if !allow_empty && items.is_empty() {
        return Err(invalid(format!("{key} must not be empty")));
    }
    Ok(items)
}

fn date_of(value: &Value, key: &str) -> Result<chrono::NaiveDate> {
    let text = value[key]
        .as_str()
        .ok_or_else(|| invalid(format!("{key} must be an ISO date string")))?;
    domain::python_date(text).ok_or_else(|| invalid(format!("{key} must use YYYY-MM-DD")))
}

/// Whether a chunk can be used: the first problem that makes it unusable,
/// worded as the retired validator worded it, since that sentence is all a
/// person repairing the chunk is told. `warnings` lets a terminal chunk whose
/// cited file is gone pass, as the full report does; without it the chunk is
/// refused, which keeps retrieval's gate strict.
pub fn metadata(service: &BankService, path: &str, value: &Value, warnings: bool) -> Result<()> {
    let mut ignored = Vec::new();
    checked(service, path, value, warnings.then_some(&mut ignored))
}

fn checked(
    service: &BankService,
    path: &str,
    value: &Value,
    mut warnings: Option<&mut Vec<String>>,
) -> Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("frontmatter must be an object"))?;
    let missing = REQUIRED
        .into_iter()
        .filter(|key| !object.contains_key(*key))
        .collect::<BTreeSet<_>>();
    if !missing.is_empty() {
        return Err(invalid(format!(
            "missing metadata keys: {}",
            missing.into_iter().collect::<Vec<_>>().join(", ")
        )));
    }
    let extra = object
        .keys()
        .map(String::as_str)
        .filter(|key| !REQUIRED.contains(key) && !OPTIONAL.contains(key))
        .collect::<BTreeSet<_>>();
    if !extra.is_empty() {
        return Err(invalid(format!(
            "unexpected metadata keys: {}",
            extra.into_iter().collect::<Vec<_>>().join(", ")
        )));
    }
    let name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let Some(bound) = codec::filename_id(name)? else {
        return Err(invalid(
            "filename must be MEM-YYYYMMDD-xxxxxxxx-short-slug.md (or legacy MEM-0001-short-slug.md)",
        ));
    };
    let pattern = codec::id_pattern()?;
    let id = value["id"]
        .as_str()
        .filter(|id| pattern.is_match(id))
        .ok_or_else(|| invalid("id must use MEM-YYYYMMDD-xxxxxxxx (or legacy MEM-0001) format"))?;
    if bound != id {
        return Err(invalid("frontmatter id does not match filename id"));
    }
    if value["title"].as_str().is_none_or(|s| s.trim().is_empty()) {
        return Err(invalid("title must be a non-empty string"));
    }
    if !TYPES.contains(&value["type"].as_str().unwrap_or("")) {
        return Err(invalid(format!(
            "type must be one of: {}",
            TYPES.join(", ")
        )));
    }
    let status = value["status"].as_str().unwrap_or("");
    if !STATUSES.contains(&status) {
        return Err(invalid(format!(
            "status must be one of: {}",
            STATUSES.join(", ")
        )));
    }
    string_list(value, "scope", false)?;
    string_list(value, "tags", false)?;
    let sources = string_list(value, "sources", false)?;
    string_list(value, "supersedes", true)?;
    let created = date_of(value, "created")?;
    let verified = date_of(value, "last_verified")?;
    let review = date_of(value, "review_after")?;
    let today = domain::clock::local_today();
    if created > verified {
        return Err(invalid("created must not be later than last_verified"));
    }
    if verified > domain::clock::latest_today() {
        return Err(invalid("last_verified must not be in the future"));
    }
    if review < verified {
        return Err(invalid(
            "review_after must not be earlier than last_verified",
        ));
    }
    if status == "active" && review < today {
        return Err(invalid("active chunk is overdue for review"));
    }
    let start = object
        .contains_key("valid_from")
        .then(|| date_of(value, "valid_from"))
        .transpose()?;
    let end = (!value["valid_to"].is_null())
        .then(|| date_of(value, "valid_to"))
        .transpose()?;
    if let Some(end) = end {
        if start.is_some_and(|start| end < start) {
            return Err(invalid("valid_to must not be earlier than valid_from"));
        }
        if status == "active" && end < today {
            return Err(invalid("active chunk is past its valid_to date"));
        }
    }
    let local = sources
        .iter()
        .filter(|source| !source.starts_with("https://") && !source.starts_with("http://"))
        .map(|source| source.split('#').next().unwrap_or(source))
        .collect::<Vec<_>>();
    if let Some(digests) = value.get("source_digests").filter(|value| !value.is_null()) {
        let rows = digests
            .as_array()
            .ok_or_else(|| invalid("source_digests must be a list"))?;
        for row in rows {
            if row.as_object().is_none_or(|entry| {
                entry.len() != 2 || !entry.contains_key("path") || !entry.contains_key("sha256")
            }) {
                return Err(invalid("each source_digests entry must be {path, sha256}"));
            }
            if row["path"].as_str().is_none_or(str::is_empty) {
                return Err(invalid("source_digests path must be a non-empty string"));
            }
            if !row["sha256"].as_str().is_some_and(|hash| {
                hash.len() == 64
                    && hash
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            }) {
                return Err(invalid("source_digests sha256 must be a hex digest"));
            }
        }
        if rows
            .iter()
            .any(|row| !local.contains(&row["path"].as_str().unwrap_or("")))
        {
            return Err(invalid(
                "source_digests may only cover paths listed in sources",
            ));
        }
    }
    let replacement = &value["superseded_by"];
    if !replacement.is_null() && replacement.as_str().is_none_or(|id| !pattern.is_match(id)) {
        return Err(invalid("superseded_by must be null or a memory ID"));
    }
    if status == "superseded" && replacement.is_null() {
        return Err(invalid("superseded chunks require superseded_by"));
    }
    if status != "superseded" && !replacement.is_null() {
        return Err(invalid("only superseded chunks may set superseded_by"));
    }
    let terminal =
        ["superseded", "archived"].contains(&status) || end.is_some_and(|end| end < today);
    for source in local {
        let resolved = fs::canonical_or_missing(&service.context.roots.resolve(source))?;
        let inside = [
            &service.context.roots.project,
            &service.context.roots.state,
            &service.context.roots.tooling,
        ]
        .iter()
        .any(|base| fs::canonical_or_missing(base).is_ok_and(|base| resolved.starts_with(base)));
        if !inside {
            return Err(RuntimeError::policy(format!(
                "source path escapes the repository: {source}"
            )));
        }
        if !resolved.exists() {
            // The containment check above stays fatal in every status: it
            // is a boundary guarantee, not a freshness one.
            if terminal && let Some(warnings) = warnings.as_deref_mut() {
                warnings.push(format!(
                    "{path}: cited source no longer exists: {source} (chunk is {status} and no longer retrievable)"
                ));
                continue;
            }
            return Err(invalid(format!("source path does not exist: {source}")));
        }
    }
    Ok(())
}

/// The secret refusal of a chunk's text, as the retired validator gave it.
fn secret(raw: &[u8]) -> Result<()> {
    let text = std::str::from_utf8(raw).map_err(|_| invalid("invalid UTF-8 chunk"))?;
    match domain::secret_label(text)? {
        Some(label) => Err(RuntimeError::policy(format!(
            "possible {label} detected; value intentionally not printed"
        ))),
        None => Ok(()),
    }
}

/// An index row: title, type, scope, tags, status, last verified, file.
type Row = Vec<String>;

/// The index's rows in file order.
fn parse_index(text: &str) -> Result<Vec<(String, Row)>> {
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    for line in text.lines().filter(|line| line.starts_with("| MEM-")) {
        let mut cells = line
            .trim()
            .trim_matches('|')
            .split('|')
            .map(|s| s.trim().to_string())
            .collect::<Vec<_>>();
        if cells.len() != 8 {
            return Err(invalid(format!("invalid index row: {line}")));
        }
        // A file cell written as a Markdown link names its target.
        let file = cells[7].trim_matches(|c| c == '[' || c == ']');
        cells[7] = file
            .split_once("](")
            .map_or(file, |(_, target)| target)
            .trim_end_matches(')')
            .to_string();
        let id = cells.remove(0);
        if !seen.insert(id.clone()) {
            return Err(invalid(format!("duplicate index ID: {id}")));
        }
        rows.push((id, cells));
    }
    Ok(rows)
}

pub fn validate_bank(service: &BankService) -> Result<Vec<String>> {
    Ok(report(service)?.0)
}

/// Errors and warnings, separately, in the retired validator's order: a
/// chunk that is wrong fails the bank, while one that correctly reached the
/// end of its life after a file it cites was deleted only warns.
pub fn report(service: &BankService) -> Result<(Vec<String>, Vec<String>)> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut files = PublicationFiles::new(&service.context.roots.state)?;
    for required in ["memory-bank/README.md", INDEX] {
        if files.read(required, PLAN_LIMIT)?.is_none() {
            errors.push(format!("{required}: required file is missing"));
        }
    }
    if !errors.is_empty() {
        return Ok((errors, warnings));
    }
    let raw = files.read(INDEX, PLAN_LIMIT)?.unwrap_or_default();
    // A pull of two clones that both promoted leaves the index in a merge
    // conflict. The index is generated, and any bank write writes it again.
    let index = match String::from_utf8(raw) {
        Err(_) => {
            errors.push(format!("{INDEX}: invalid UTF-8 index"));
            Some(Vec::new())
        }
        Ok(text)
            if text
                .lines()
                .any(|line| line.starts_with("<<<<<<< ") || line.starts_with(">>>>>>> ")) =>
        {
            errors.push(format!(
                "{INDEX}: unresolved Git merge conflict; the index is generated from the chunks - `memory-bank/bin/infra-engine memory reindex-bank` writes it again"
            ));
            None
        }
        Ok(text) => match parse_index(&text) {
            Ok(index) => Some(index),
            Err(error) => {
                errors.push(format!("{INDEX}: {error}"));
                Some(Vec::new())
            }
        },
    };
    let directory = service.context.roots.state.join("memory-bank/chunks");
    if fs::admit_directory(&directory)? {
        let mut entries = std::fs::read_dir(&directory)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let m = std::fs::symlink_metadata(entry.path())?;
            // Native seeding carries this one empty directory placeholder.
            // Strict rooted admission still rejects links, reparse points,
            // multiple links, non-regular entries and nonempty placeholders.
            if entry.file_name() == ".gitkeep"
                && files
                    .read("memory-bank/chunks/.gitkeep", 0)
                    .is_ok_and(|bytes| bytes.is_some_and(|bytes| bytes.is_empty()))
            {
                continue;
            }
            if fs::linked_at(&entry.path(), &m)?
                || !m.is_file()
                || codec::filename_id(&entry.file_name().to_string_lossy())?.is_none()
            {
                errors.push(format!(
                    "memory-bank/chunks/{}: unexpected chunk entry; expected a direct MEM-0001-short-slug.md file",
                    entry.file_name().to_string_lossy()
                ));
            }
        }
    }
    let mut chunks = BTreeMap::<String, codec::Chunk>::new();
    let mut order = Vec::<String>::new();
    // Every id on disk, the broken chunks' too: an index row for a chunk
    // that is here and broken is not a row for a missing file.
    let mut on_disk = BTreeSet::new();
    for path in codec::paths(service)? {
        if let Some(id) = codec::filename_id(
            Path::new(&path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(""),
        )? {
            on_disk.insert(id);
        }
        let result = (|| -> Result<codec::Chunk> {
            let chunk = codec::read(&mut files, &path)?;
            checked(service, &path, &chunk.metadata, Some(&mut warnings))?;
            secret(&chunk.raw)?;
            let id = chunk.metadata["id"].as_str().unwrap_or("").to_string();
            on_disk.insert(id.clone());
            if chunks.contains_key(&id) {
                return Err(invalid(format!("duplicate chunk ID: {id}")));
            }
            Ok(chunk)
        })();
        match result {
            Ok(chunk) => {
                let id = chunk.metadata["id"].as_str().unwrap_or("").to_string();
                order.push(id.clone());
                chunks.insert(id, chunk);
            }
            Err(error) => errors.push(format!("{path}: {error}")),
        }
    }
    if let Some(index) = &index {
        let rows = index
            .iter()
            .map(|(id, row)| (id.as_str(), row))
            .collect::<BTreeMap<_, _>>();
        for id in &order {
            let chunk = &chunks[id];
            let Some(row) = rows.get(id.as_str()) else {
                errors.push(format!("{}: chunk is missing from INDEX.md", chunk.path));
                continue;
            };
            let m = &chunk.metadata;
            let expected_file = chunk
                .path
                .strip_prefix("memory-bank/")
                .unwrap_or(&chunk.path);
            if row[6] != expected_file {
                errors.push(format!("{INDEX}: {id} file must be {expected_file}"));
            }
            for (column, key) in [
                (0, "title"),
                (1, "type"),
                (4, "status"),
                (5, "last_verified"),
                (2, "scope"),
                (3, "tags"),
            ] {
                let expected = match key {
                    "scope" | "tags" => string_list(m, key, true)?.join(", "),
                    _ => m[key].as_str().unwrap_or("").to_string(),
                };
                if row[column] != expected {
                    errors.push(format!("{INDEX}: {id} {key} differs from chunk metadata"));
                }
            }
        }
        for (id, row) in index {
            if chunks.contains_key(id) {
                continue;
            }
            if on_disk.contains(id) {
                // The chunk is on disk and already reported its own error.
                // Saying it is missing sends the reader looking for a deleted
                // file that is sitting right there.
                warnings.push(format!(
                    "{INDEX}: {id} index row retained for invalid chunk ({})",
                    row[6]
                ));
            } else {
                errors.push(format!(
                    "{INDEX}: {id} points to a missing chunk ({})",
                    row[6]
                ));
            }
        }
    }
    for id in &order {
        let m = &chunks[id].metadata;
        let predecessors = string_list(m, "supersedes", true)?;
        let successor = m["superseded_by"].as_str();
        if predecessors.contains(id) || successor == Some(id.as_str()) {
            errors.push(format!("{id}: chunk cannot supersede itself"));
        }
        for referenced in predecessors.iter().map(String::as_str).chain(successor) {
            if !chunks.contains_key(referenced) {
                errors.push(format!(
                    "{id}: replacement link points to missing {referenced}"
                ));
            }
        }
        for previous in &predecessors {
            let Some(previous_chunk) = chunks.get(previous) else {
                continue;
            };
            if previous_chunk.metadata["status"] != "superseded" {
                errors.push(format!("{id}: {previous} must have superseded status"));
            }
            if previous_chunk.metadata["superseded_by"].as_str() != Some(id.as_str()) {
                errors.push(format!(
                    "{id}: {previous} must point back with superseded_by"
                ));
            }
        }
        if let Some(successor) = successor
            && let Some(next) = chunks.get(successor)
            && !string_list(&next.metadata, "supersedes", true)?.contains(id)
        {
            errors.push(format!(
                "{id}: {successor} must include this ID in supersedes"
            ));
        }
    }
    for id in &order {
        let mut seen = BTreeSet::new();
        let mut current = Some(id.as_str());
        while let Some(next) = current {
            let Some(chunk) = chunks.get(next) else {
                break;
            };
            if !seen.insert(next) {
                errors.push(format!("{id}: supersession cycle detected"));
                break;
            }
            current = chunk.metadata["superseded_by"].as_str();
        }
    }
    Ok((errors, warnings))
}

pub fn write_errors(
    service: &BankService,
    before: &[String],
    attested: &[String],
) -> Result<Vec<String>> {
    let mut errors = validate_bank(service)?
        .into_iter()
        .filter(|error| !before.contains(error))
        .collect::<Vec<_>>();
    let mut files = PublicationFiles::new(&service.context.roots.state)?;
    for path in attested {
        let result = (|| -> Result<()> {
            let chunk = codec::read(&mut files, path)?;
            metadata(service, path, &chunk.metadata, true)?;
            secret(&chunk.raw)
        })();
        if let Err(error) = result {
            let message = format!("{path}: {error}");
            if !errors.contains(&message) {
                errors.push(message);
            }
        }
    }
    Ok(errors)
}
