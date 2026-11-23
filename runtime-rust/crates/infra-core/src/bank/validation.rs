use super::{
    BankService, codec,
    publication::{PLAN_LIMIT, PublicationFiles},
};
use crate::{
    Result, RuntimeError,
    domain::{self},
    fs,
};
use rusqlite::{Connection, OpenFlags, params};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub type SymbolLookupCallback<'a> = dyn Fn(&str, &str) -> Result<bool> + 'a;

pub enum SymbolVerifier<'a> {
    Callback(&'a SymbolLookupCallback<'a>),
    Sqlite(&'a Path),
}

enum ActiveSymbolVerifier<'a> {
    Callback(&'a SymbolLookupCallback<'a>),
    Sqlite(Connection),
}

impl<'a> ActiveSymbolVerifier<'a> {
    fn open(verifier: &'a SymbolVerifier<'a>) -> Result<Self> {
        match verifier {
            SymbolVerifier::Callback(cb) => Ok(Self::Callback(*cb)),
            SymbolVerifier::Sqlite(path) => {
                let meta = std::fs::symlink_metadata(path).map_err(|_| {
                    RuntimeError::unavailable(format!(
                        "Graph IR database unavailable: {}",
                        path.display()
                    ))
                })?;
                if fs::linked_at(path, &meta)? || !meta.is_file() {
                    return Err(RuntimeError::policy(format!(
                        "Graph IR database must be a regular non-linked file: {}",
                        path.display()
                    )));
                }
                let conn = Connection::open_with_flags(
                    path,
                    OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
                )?;
                Ok(Self::Sqlite(conn))
            }
        }
    }

    fn verify(&self, file_path: &str, symbol: &str) -> Result<bool> {
        match self {
            Self::Callback(cb) => cb(file_path, symbol),
            Self::Sqlite(conn) => sqlite_symbol_exists(conn, file_path, symbol),
        }
    }
}

fn is_line_anchor(fragment: &str) -> bool {
    let trimmed = fragment.strip_prefix(['L', 'l']).unwrap_or(fragment);
    if trimmed.is_empty() {
        return false;
    }
    if let Some((start, end)) = trimmed.split_once('-') {
        let end = end.strip_prefix(['L', 'l']).unwrap_or(end);
        !start.is_empty()
            && !end.is_empty()
            && start.bytes().all(|b| b.is_ascii_digit())
            && end.bytes().all(|b| b.is_ascii_digit())
    } else {
        trimmed.bytes().all(|b| b.is_ascii_digit())
    }
}

fn is_symbol_citation(base_path: &str, fragment: &str) -> bool {
    if fragment.is_empty() || is_line_anchor(fragment) {
        return false;
    }
    let ext = Path::new(base_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    !matches!(
        ext.as_str(),
        "md" | "markdown" | "txt" | "rst" | "adoc" | "json" | "toml" | "yaml" | "yml"
    )
}

fn split_symbol_query(query: &str) -> (Option<&str>, &str) {
    if let Some((qual, leaf)) = query.rsplit_once("::")
        && !qual.is_empty()
        && !leaf.is_empty()
    {
        return (Some(qual), leaf);
    }
    if query.contains('\\')
        && let Some((qual, leaf)) = query.rsplit_once('\\')
        && !leaf.is_empty()
    {
        return (Some(qual).filter(|q| !q.is_empty()), leaf);
    }
    if let Some((qual, leaf)) = query.rsplit_once('.')
        && !qual.is_empty()
        && !leaf.is_empty()
    {
        return (Some(qual), leaf);
    }
    (None, query)
}

fn symbol_matches_query(
    symbol_id: &str,
    name: &str,
    parent_name: Option<&str>,
    raw_query: &str,
) -> bool {
    if symbol_id == raw_query {
        return true;
    }
    let (qual, leaf) = split_symbol_query(raw_query);
    if let Some(q) = qual {
        let q_short = q.rsplit([':', '\\', '.']).next().unwrap_or(q);
        name == leaf
            && parent_name.is_some_and(|parent| {
                parent == q
                    || parent == q_short
                    || parent.ends_with(&format!("::{q}"))
                    || parent.ends_with(&format!("\\{q}"))
                    || parent.ends_with(&format!(".{q}"))
            })
    } else {
        name == raw_query || name == leaf
    }
}

fn sqlite_symbol_exists(conn: &Connection, file_path: &str, symbol: &str) -> Result<bool> {
    let (_, leaf) = split_symbol_query(symbol);
    let mut normalized = file_path.replace('\\', "/");
    while let Some(stripped) = normalized.strip_prefix("./") {
        normalized = stripped.to_string();
    }
    let mut stmt = conn.prepare_cached(
        "SELECT symbol_id, name, parent_name FROM ir_symbols WHERE file_path=?1 AND (symbol_id=?2 OR name=?3)",
    )?;
    let mut rows = stmt.query(params![normalized, symbol, leaf])?;
    while let Some(row) = rows.next()? {
        let symbol_id: String = row.get(0)?;
        let name: String = row.get(1)?;
        let parent_name: Option<String> = row.get(2)?;
        if symbol_matches_query(&symbol_id, &name, parent_name.as_deref(), symbol) {
            return Ok(true);
        }
    }
    Ok(false)
}

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
    metadata_with_symbol_verifier(service, path, value, warnings, None)
}

pub fn metadata_with_symbol_verifier(
    service: &BankService,
    path: &str,
    value: &Value,
    warnings: bool,
    symbol_verifier: Option<&SymbolVerifier<'_>>,
) -> Result<()> {
    let active = symbol_verifier
        .map(ActiveSymbolVerifier::open)
        .transpose()?;
    let mut ignored = Vec::new();
    checked(
        service,
        path,
        value,
        warnings.then_some(&mut ignored),
        active.as_ref(),
    )
}

fn checked(
    service: &BankService,
    path: &str,
    value: &Value,
    mut warnings: Option<&mut Vec<String>>,
    symbol_verifier: Option<&ActiveSymbolVerifier<'_>>,
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
    let local_refs = sources
        .iter()
        .filter(|source| !source.starts_with("https://") && !source.starts_with("http://"))
        .map(|source| {
            let (base, fragment) = source
                .split_once('#')
                .map_or((source.as_str(), None), |(b, f)| {
                    (b, (!f.is_empty()).then_some(f))
                });
            (source.as_str(), base, fragment)
        })
        .collect::<Vec<_>>();
    let local = local_refs
        .iter()
        .map(|(_, base, _)| *base)
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
    for (raw_source, source, fragment) in local_refs {
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
        if let Some(symbol) = fragment
            && let Some(verifier) = symbol_verifier
            && is_symbol_citation(source, symbol)
            && !verifier.verify(source, symbol)?
        {
            if terminal && let Some(warnings) = warnings.as_deref_mut() {
                warnings.push(format!(
                    "{path}: cited symbol no longer exists: {raw_source} (chunk is {status} and no longer retrievable)"
                ));
                continue;
            }
            return Err(invalid(format!(
                "cited symbol does not exist: {raw_source}"
            )));
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
    validate_bank_with_symbol_verifier(service, None)
}

pub fn validate_bank_with_symbol_verifier(
    service: &BankService,
    symbol_verifier: Option<&SymbolVerifier<'_>>,
) -> Result<Vec<String>> {
    Ok(report_with_symbol_verifier(service, symbol_verifier)?.0)
}

/// Errors and warnings, separately, in the retired validator's order: a
/// chunk that is wrong fails the bank, while one that correctly reached the
/// end of its life after a file it cites was deleted only warns.
pub fn report(service: &BankService) -> Result<(Vec<String>, Vec<String>)> {
    report_with_symbol_verifier(service, None)
}

pub fn report_with_symbol_verifier(
    service: &BankService,
    symbol_verifier: Option<&SymbolVerifier<'_>>,
) -> Result<(Vec<String>, Vec<String>)> {
    let active = symbol_verifier
        .map(ActiveSymbolVerifier::open)
        .transpose()?;
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let roots = &service.context.roots;
    let mut files = PublicationFiles::new(&roots.state)?;
    for required in ["memory-bank/README.md", INDEX] {
        if files.read(required, PLAN_LIMIT)?.is_some() {
            continue;
        }
        // An attached state holds only the project's memory; the README stays
        // with the accelerator it is attached from, which is never seeded into
        // the state. `bank validate` failed every attached project here.
        if required == "memory-bank/README.md"
            && roots.attached()
            && PublicationFiles::new(&roots.tooling)?
                .read(required, PLAN_LIMIT)?
                .is_some()
        {
            continue;
        }
        errors.push(format!("{required}: required file is missing"));
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
            checked(
                service,
                &path,
                &chunk.metadata,
                Some(&mut warnings),
                active.as_ref(),
            )?;
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
    write_errors_with_symbol_verifier(service, before, attested, None)
}

pub fn write_errors_with_symbol_verifier(
    service: &BankService,
    before: &[String],
    attested: &[String],
    symbol_verifier: Option<&SymbolVerifier<'_>>,
) -> Result<Vec<String>> {
    let active = symbol_verifier
        .map(ActiveSymbolVerifier::open)
        .transpose()?;
    let mut errors = validate_bank_with_symbol_verifier(service, symbol_verifier)?
        .into_iter()
        .filter(|error| !before.contains(error))
        .collect::<Vec<_>>();
    let mut files = PublicationFiles::new(&service.context.roots.state)?;
    for path in attested {
        let result = (|| -> Result<()> {
            let chunk = codec::read(&mut files, path)?;
            checked(service, path, &chunk.metadata, None, active.as_ref())?;
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

#[cfg(test)]
mod readme_tests {
    use super::*;
    use crate::{Roots, RuntimeContext};
    use std::path::PathBuf;

    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn readme_errors(roots: Roots) -> Vec<String> {
        let service = BankService::new(RuntimeContext::new(roots, "local", "governed").unwrap());
        validate_bank(&service)
            .unwrap()
            .into_iter()
            .filter(|error| error.starts_with("memory-bank/README.md"))
            .collect()
    }

    /// An attached state is seeded without the bank's README, which stays with
    /// the accelerator clone: `bank validate` refused every healthy attached
    /// project for it. An installed bank still needs its own.
    #[test]
    fn an_attached_bank_reads_its_readme_from_the_clone() {
        let root =
            std::env::temp_dir().join(format!("infra-bank-readme-{}", crate::domain::uuid4()));
        let _cleanup = Directory(root.clone());
        let (tooling, project, state) =
            (root.join("clone"), root.join("project"), root.join("state"));
        for folder in [&tooling, &state] {
            std::fs::create_dir_all(folder.join("memory-bank")).unwrap();
        }
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(state.join("memory-bank/INDEX.md"), "# Memory Bank Index\n").unwrap();
        let attached = |tooling: &PathBuf| Roots {
            tooling: tooling.clone(),
            project: project.clone(),
            state: state.clone(),
            shared: false,
        };
        assert_eq!(
            readme_errors(attached(&tooling)),
            ["memory-bank/README.md: required file is missing"],
            "a clone without the README must still fail"
        );
        std::fs::write(tooling.join("memory-bank/README.md"), "# Memory Bank\n").unwrap();
        assert!(readme_errors(attached(&tooling)).is_empty());
        assert_eq!(
            readme_errors(Roots::installed(&state).unwrap()),
            ["memory-bank/README.md: required file is missing"],
            "an installed bank must keep its own README"
        );
    }

    #[test]
    fn symbol_verifier_checks_code_symbols_against_callback_and_sqlite_ir() {
        let root = std::env::temp_dir().join(format!("infra-bank-sym-{}", crate::domain::uuid4()));
        let _cleanup = Directory(root.clone());
        std::fs::create_dir_all(root.join("memory-bank/chunks")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("specs")).unwrap();
        std::fs::write(root.join("memory-bank/README.md"), "# Memory Bank\n").unwrap();
        std::fs::write(
            root.join("memory-bank/INDEX.md"),
            "# Memory Bank Index\n\n| ID | Title | Type | Status | Updated | File |\n|---|---|---|---|---|---|\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/InvoiceService.php"),
            "<?php\nclass InvoiceService { public function issue(): void {} }\n",
        )
        .unwrap();
        std::fs::write(root.join("specs/invoice.md"), "# Invoice\n\n## Rules\n").unwrap();

        let service = BankService::new(
            RuntimeContext::new(Roots::installed(&root).unwrap(), "local", "governed").unwrap(),
        );

        let chunk_meta = |id: &str, status: &str, superseded_by: Option<&str>, sources: Value| {
            serde_json::json!({
                "id": id,
                "title": "Invoice rules",
                "type": "convention",
                "status": status,
                "scope": ["application"],
                "tags": ["project-brain", "promoted"],
                "created": "2026-01-01",
                "last_verified": "2026-01-01",
                "review_after": "9999-12-31",
                "sources": sources,
                "supersedes": [],
                "superseded_by": superseded_by
            })
        };

        // Line anchors & markdown section anchors are exempt from Graph IR symbol checks even when verifier rejects unknown symbols.
        let valid_meta = chunk_meta(
            "MEM-20260101-00000001",
            "active",
            None,
            serde_json::json!([
                "src/InvoiceService.php#InvoiceService::issue",
                "src/InvoiceService.php#L1-L2",
                "specs/invoice.md#rules"
            ]),
        );
        let cb = |path: &str, sym: &str| {
            Ok(path == "src/InvoiceService.php" && sym == "InvoiceService::issue")
        };
        metadata_with_symbol_verifier(
            &service,
            "memory-bank/chunks/MEM-20260101-00000001-invoice.md",
            &valid_meta,
            true,
            Some(&SymbolVerifier::Callback(&cb)),
        )
        .unwrap();

        let missing_cb_meta = chunk_meta(
            "MEM-20260101-00000002",
            "active",
            None,
            serde_json::json!(["src/InvoiceService.php#InvoiceService::missing"]),
        );
        let err = metadata_with_symbol_verifier(
            &service,
            "memory-bank/chunks/MEM-20260101-00000002-invoice.md",
            &missing_cb_meta,
            true,
            Some(&SymbolVerifier::Callback(&cb)),
        )
        .unwrap_err();
        assert!(err.message.contains("cited symbol does not exist"));

        // Verify against a real read-only SQLite ir_symbols table.
        let ir_db = root.join("ir.sqlite");
        {
            let conn = Connection::open(&ir_db).unwrap();
            conn.execute_batch(
                "CREATE TABLE ir_symbols(
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    file_path TEXT NOT NULL,
                    symbol_id TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    name TEXT NOT NULL,
                    parent_name TEXT,
                    signature TEXT,
                    visibility TEXT,
                    start_line INTEGER NOT NULL,
                    end_line INTEGER NOT NULL,
                    modifiers_json TEXT NOT NULL,
                    body_fingerprint TEXT NOT NULL,
                    is_test INTEGER NOT NULL DEFAULT 0
                );",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO ir_symbols(file_path,symbol_id,kind,name,parent_name,start_line,end_line,modifiers_json,body_fingerprint,is_test)
                 VALUES('src/InvoiceService.php','sym:method:App\\InvoiceService::issue','method','issue','App\\InvoiceService',2,2,'[]','fp',0)",
                [],
            )
            .unwrap();
        }
        let sqlite_verifier = SymbolVerifier::Sqlite(&ir_db);
        metadata_with_symbol_verifier(
            &service,
            "memory-bank/chunks/MEM-20260101-00000001-invoice.md",
            &valid_meta,
            true,
            Some(&sqlite_verifier),
        )
        .unwrap();

        let missing_err = metadata_with_symbol_verifier(
            &service,
            "memory-bank/chunks/MEM-20260101-00000002-invoice.md",
            &missing_cb_meta,
            true,
            Some(&sqlite_verifier),
        )
        .unwrap_err();
        assert!(missing_err.message.contains("cited symbol does not exist"));

        // Terminal chunks with a missing cited symbol emit a warning instead of failing report().
        let retired_bytes = codec::render_chunk(
            &chunk_meta(
                "MEM-20260101-00000003",
                "archived",
                None,
                serde_json::json!(["src/InvoiceService.php#InvoiceService::legacy"]),
            ),
            "Archived chunk body\n",
        )
        .unwrap();
        let retired_path = "memory-bank/chunks/MEM-20260101-00000003-retired.md";
        std::fs::write(root.join(retired_path), retired_bytes).unwrap();
        std::fs::write(
            root.join("memory-bank/INDEX.md"),
            "# Memory Bank Index\n\n| ID | Title | Type | Scope | Tags | Status | Last Verified | File |\n|---|---|---|---|---|---|---|---|\n| MEM-20260101-00000003 | Invoice rules | convention | application | project-brain, promoted | archived | 2026-01-01 | [chunks/MEM-20260101-00000003-retired.md](chunks/MEM-20260101-00000003-retired.md) |\n",
        )
        .unwrap();
        let (errors, warnings) =
            report_with_symbol_verifier(&service, Some(&sqlite_verifier)).unwrap();
        assert!(
            errors.is_empty(),
            "unexpected validation errors: {errors:?}"
        );
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("cited symbol no longer exists")
                    && w.contains("InvoiceService::legacy")),
            "expected symbol warning in {warnings:?}"
        );
    }
}
