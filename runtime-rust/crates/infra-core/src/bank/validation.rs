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

pub fn metadata(service: &BankService, path: &str, value: &Value, warnings: bool) -> Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| RuntimeError::invalid("Memory metadata must be an object"))?;
    let required = [
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
    let optional = ["valid_from", "valid_to", "source_digests"];
    if required.iter().any(|key| !object.contains_key(*key))
        || object
            .keys()
            .any(|key| !required.contains(&key.as_str()) && !optional.contains(&key.as_str()))
    {
        return Err(RuntimeError::invalid(
            "Missing or unexpected memory metadata keys",
        ));
    }
    let id = value["id"]
        .as_str()
        .ok_or_else(|| RuntimeError::invalid("Memory ID must be text"))?;
    if !codec::id_pattern()?.is_match(id)
        || codec::filename_id(
            Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(""),
        )?
        .as_deref()
            != Some(id)
    {
        return Err(RuntimeError::invalid(
            "Memory ID or filename binding is invalid",
        ));
    }
    if value["title"].as_str().is_none_or(|s| s.trim().is_empty()) {
        return Err(RuntimeError::invalid("Memory title must not be empty"));
    }
    if ![
        "architecture",
        "constraint",
        "convention",
        "decision",
        "domain",
        "integration",
        "operations",
    ]
    .contains(&value["type"].as_str().unwrap_or(""))
    {
        return Err(RuntimeError::invalid("Invalid memory type"));
    }
    let status = value["status"].as_str().unwrap_or("");
    if !["active", "needs-review", "superseded", "archived"].contains(&status) {
        return Err(RuntimeError::invalid("Invalid memory status"));
    }
    for key in ["scope", "tags", "sources"] {
        codec::strings(value, key, false)?;
    }
    let supersedes = codec::strings(value, "supersedes", true)?;
    let created = codec::date(value["created"].as_str().unwrap_or(""))?;
    let verified = codec::date(value["last_verified"].as_str().unwrap_or(""))?;
    let review = codec::date(value["review_after"].as_str().unwrap_or(""))?;
    let today = domain::clock::local_today();
    let latest = domain::clock::latest_today();
    if created > verified
        || verified > latest
        || review < verified
        || status == "active" && review < today
    {
        return Err(RuntimeError::invalid(
            "Memory verification dates are invalid or overdue",
        ));
    }
    let start = value
        .get("valid_from")
        .map(|v| codec::date(v.as_str().unwrap_or("")))
        .transpose()?;
    let end = value
        .get("valid_to")
        .filter(|v| !v.is_null())
        .map(|v| codec::date(v.as_str().unwrap_or("")))
        .transpose()?;
    if start.zip(end).is_some_and(|(a, b)| b < a)
        || status == "active" && end.is_some_and(|end| end < today)
    {
        return Err(RuntimeError::invalid(
            "Memory validity interval is invalid or retired",
        ));
    }
    let sources = codec::strings(value, "sources", false)?;
    let local = sources
        .iter()
        .filter(|source| !source.starts_with("https://") && !source.starts_with("http://"))
        .map(|source| source.split('#').next().unwrap_or(source))
        .collect::<BTreeSet<_>>();
    if let Some(digests) = value.get("source_digests").filter(|value| !value.is_null()) {
        let rows = digests
            .as_array()
            .ok_or_else(|| RuntimeError::invalid("source_digests must be a list"))?;
        for row in rows {
            let path = row["path"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| RuntimeError::invalid("Digest path must be text"))?;
            let hash = row["sha256"].as_str().unwrap_or("");
            if row.as_object().is_none_or(|o| o.len() != 2)
                || hash.len() != 64
                || !hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                || !local.contains(path)
            {
                return Err(RuntimeError::invalid(
                    "Invalid source digest or uncited path",
                ));
            }
        }
    }
    let successor = value["superseded_by"].as_str();
    if !value["superseded_by"].is_null()
        && successor.is_none_or(|id| !codec::id_pattern().is_ok_and(|p| p.is_match(id)))
    {
        return Err(RuntimeError::invalid("Invalid superseded_by"));
    }
    if (status == "superseded") != successor.is_some() {
        return Err(RuntimeError::invalid(
            "Superseded chunks require a successor and only those chunks may name one",
        ));
    }
    for id in supersedes {
        if !codec::id_pattern()?.is_match(&id) {
            return Err(RuntimeError::invalid("Invalid predecessor ID"));
        }
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
            return Err(RuntimeError::policy(
                "Memory source path escapes repository roots",
            ));
        }
        if !resolved.exists() && !(warnings && terminal) {
            return Err(RuntimeError::invalid(format!(
                "Source path does not exist: {source}"
            )));
        }
    }
    Ok(())
}

fn parse_index(text: &str) -> Result<BTreeMap<String, Vec<String>>> {
    let mut index = BTreeMap::new();
    for line in text.lines().filter(|line| line.starts_with("| MEM-")) {
        let cells = line
            .trim()
            .trim_matches('|')
            .split('|')
            .map(|s| s.trim().to_string())
            .collect::<Vec<_>>();
        if cells.len() != 8 {
            return Err(RuntimeError::invalid("Invalid memory index row"));
        }
        if index
            .insert(cells[0].clone(), cells[1..].to_vec())
            .is_some()
        {
            return Err(RuntimeError::invalid("Duplicate memory index ID"));
        }
    }
    Ok(index)
}

pub fn validate_bank(service: &BankService) -> Result<Vec<String>> {
    let mut errors = Vec::new();
    let mut files = PublicationFiles::new(&service.context.roots.state)?;
    for required in ["memory-bank/README.md", "memory-bank/INDEX.md"] {
        if files.read(required, PLAN_LIMIT)?.is_none() {
            errors.push(format!("{required}: required file is missing"));
        }
    }
    if !errors.is_empty() {
        return Ok(errors);
    }
    let raw = files
        .read("memory-bank/INDEX.md", PLAN_LIMIT)?
        .unwrap_or_default();
    let text =
        String::from_utf8(raw).map_err(|_| RuntimeError::invalid("Memory index is not UTF-8"))?;
    let conflicted = text
        .lines()
        .any(|line| line.starts_with("<<<<<<< ") || line.starts_with(">>>>>>> "));
    let index = if conflicted {
        errors.push(
            "memory-bank/INDEX.md: unresolved Git merge conflict; regenerate the index".into(),
        );
        None
    } else {
        match parse_index(&text) {
            Ok(index) => Some(index),
            Err(error) => {
                errors.push(format!("memory-bank/INDEX.md: {error}"));
                Some(BTreeMap::new())
            }
        }
    };
    let directory = service.context.roots.state.join("memory-bank/chunks");
    if fs::admit_directory(&directory)? {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
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
                    "memory-bank/chunks/{}: unexpected chunk entry",
                    entry.file_name().to_string_lossy()
                ));
            }
        }
    }
    let mut chunks = BTreeMap::<String, codec::Chunk>::new();
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
            metadata(service, &path, &chunk.metadata, true)?;
            if let Some(label) = domain::secret_label(
                std::str::from_utf8(&chunk.raw)
                    .map_err(|_| RuntimeError::invalid("Memory chunk is not UTF-8"))?,
            )? {
                return Err(RuntimeError::policy(format!("Possible {label}")));
            }
            Ok(chunk)
        })();
        match result {
            Ok(chunk) => {
                let id = chunk.metadata["id"].as_str().unwrap_or("").to_string();
                if let std::collections::btree_map::Entry::Vacant(e) = chunks.entry(id) {
                    e.insert(chunk);
                } else {
                    errors.push(format!("{path}: duplicate chunk ID"));
                }
            }
            Err(error) => errors.push(format!("{path}: {error}")),
        }
    }
    if let Some(index) = index {
        for (id, chunk) in &chunks {
            let Some(row) = index.get(id) else {
                errors.push(format!("{}: chunk is missing from INDEX.md", chunk.path));
                continue;
            };
            let m = &chunk.metadata;
            let expected = [
                m["title"].as_str().unwrap_or("").to_string(),
                m["type"].as_str().unwrap_or("").into(),
                codec::strings(m, "scope", true)?.join(", "),
                codec::strings(m, "tags", true)?.join(", "),
                m["status"].as_str().unwrap_or("").into(),
                m["last_verified"].as_str().unwrap_or("").into(),
                chunk
                    .path
                    .strip_prefix("memory-bank/")
                    .unwrap_or(&chunk.path)
                    .into(),
            ];
            if row != &expected {
                errors.push(format!(
                    "memory-bank/INDEX.md: {id} row differs from chunk metadata"
                ));
            }
        }
        for id in index.keys() {
            if !chunks.contains_key(id) && !on_disk.contains(id) {
                errors.push(format!(
                    "memory-bank/INDEX.md: {id} points to a missing chunk"
                ));
            }
        }
    }
    for (id, chunk) in &chunks {
        let m = &chunk.metadata;
        let predecessors = codec::strings(m, "supersedes", true)?;
        let successor = m["superseded_by"].as_str();
        if predecessors.iter().any(|p| p == id) || successor == Some(id) {
            errors.push(format!("{id}: chunk cannot supersede itself"));
        }
        for previous in &predecessors {
            match chunks.get(previous) {
                Some(previous_chunk) => {
                    if previous_chunk.metadata["status"] != "superseded"
                        || previous_chunk.metadata["superseded_by"].as_str() != Some(id)
                    {
                        errors.push(format!(
                            "{id}: predecessor {previous} has inconsistent inverse lineage"
                        ));
                    }
                }
                None => errors.push(format!(
                    "{id}: replacement link points to missing {previous}"
                )),
            }
        }
        if let Some(successor) = successor {
            match chunks.get(successor) {
                Some(next) => {
                    if !codec::strings(&next.metadata, "supersedes", true)?
                        .iter()
                        .any(|previous| previous == id)
                    {
                        errors.push(format!(
                            "{id}: successor must include this ID in supersedes"
                        ));
                    }
                }
                None => errors.push(format!(
                    "{id}: replacement link points to missing {successor}"
                )),
            }
        }
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
    Ok(errors)
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
            if let Some(label) = domain::secret_label(
                std::str::from_utf8(&chunk.raw)
                    .map_err(|_| RuntimeError::invalid("Memory chunk is not UTF-8"))?,
            )? {
                return Err(RuntimeError::policy(format!("Possible {label}")));
            }
            Ok(())
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
