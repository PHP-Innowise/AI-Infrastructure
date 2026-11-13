use super::{
    BankService,
    publication::{PLAN_LIMIT, PublicationFiles},
};
use crate::{
    Result, RuntimeError,
    domain::{self, json},
    fs,
};
use serde_json::{Value, json as object};
use std::{collections::BTreeMap, path::Path};

pub const HEADER: &str = "| ID | Title | Type | Scope | Tags | Status | Last Verified | File |\n| --- | --- | --- | --- | --- | --- | --- | --- |\n";
const PREAMBLE: &str = "# Memory Index\n\nRead [README.md](README.md) before using this index. Load only active chunks relevant to the current task, then verify them against their cited sources.\n\n";
#[derive(Clone, Debug)]
pub struct Chunk {
    pub path: String,
    pub metadata: Value,
    pub body: String,
    pub raw: Vec<u8>,
}

pub fn id_pattern() -> Result<regex::Regex> {
    static ID: std::sync::OnceLock<std::result::Result<regex::Regex, regex::Error>> =
        std::sync::OnceLock::new();
    ID.get_or_init(|| regex::Regex::new(r"^MEM-(?:\d{8}-[0-9a-f]{8}|\d{4,})$"))
        .as_ref()
        .cloned()
        .map_err(|_| RuntimeError::invalid("Invalid internal memory ID expression"))
}
pub fn filename_id(name: &str) -> Result<Option<String>> {
    static FILENAME: std::sync::OnceLock<std::result::Result<regex::Regex, regex::Error>> =
        std::sync::OnceLock::new();
    let pattern = FILENAME
        .get_or_init(|| {
            regex::Regex::new(r"^(MEM-(?:\d{8}-[0-9a-f]{8}|\d{4,}))-[a-z0-9]+(?:-[a-z0-9]+)*\.md$")
        })
        .as_ref()
        .map_err(|_| RuntimeError::invalid("Invalid internal memory filename expression"))?;
    Ok(pattern.captures(name).map(|capture| capture[1].into()))
}
pub fn parse_raw(bytes: &[u8]) -> Result<(Value, String)> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| RuntimeError::invalid("Memory chunk is not UTF-8"))?;
    let rest = text
        .strip_prefix("---\r\n")
        .or_else(|| text.strip_prefix("---\n"))
        .ok_or_else(|| RuntimeError::invalid("Memory frontmatter opening delimiter is missing"))?;
    static DELIMITER: std::sync::OnceLock<std::result::Result<regex::Regex, regex::Error>> =
        std::sync::OnceLock::new();
    let delimiter = DELIMITER
        .get_or_init(|| regex::Regex::new(r"\r?\n---(?:\r?\n|$)"))
        .as_ref()
        .map_err(|_| RuntimeError::invalid("Invalid delimiter expression"))?;
    let found = delimiter
        .find(rest)
        .ok_or_else(|| RuntimeError::invalid("Memory frontmatter is unterminated"))?;
    let metadata = json::parse(&rest[..found.start()])?;
    if !metadata.is_object() {
        return Err(RuntimeError::invalid(
            "Memory frontmatter must be an object",
        ));
    }
    Ok((metadata, rest[found.end()..].into()))
}
pub fn render_chunk(metadata: &Value, body: &str) -> Result<Vec<u8>> {
    Ok(format!(
        "---\n{}\n---\n{body}",
        json::serialize(metadata, false, true, true, true)?
    )
    .into_bytes())
}
pub fn read(files: &mut PublicationFiles, path: &str) -> Result<Chunk> {
    let raw = files
        .read(path, usize::MAX.saturating_sub(1))?
        .ok_or_else(|| RuntimeError::not_found("Memory chunk not found"))?;
    let (metadata, body) = parse_raw(&raw)?;
    Ok(Chunk {
        path: path.into(),
        metadata,
        body,
        raw,
    })
}
pub fn paths(service: &BankService) -> Result<Vec<String>> {
    let base = service.context.roots.state.join("memory-bank/chunks");
    if !fs::admit_directory(&base)? {
        return Ok(vec![]);
    }
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(base)? {
        let entry = entry?;
        let metadata = std::fs::symlink_metadata(entry.path())?;
        if fs::linked_at(&entry.path(), &metadata)? || !metadata.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| RuntimeError::invalid("Memory filename is not UTF-8"))?;
        if filename_id(name)?.is_some() {
            paths.push(format!("memory-bank/chunks/{name}"));
        }
    }
    paths.sort();
    Ok(paths)
}
pub fn find(service: &BankService, id: &str, files: &mut PublicationFiles) -> Result<Chunk> {
    if id.len() > 128 || !id_pattern()?.is_match(id) {
        return Err(RuntimeError::invalid("Invalid memory ID"));
    }
    let selected = paths(service)?
        .into_iter()
        .filter(|path| {
            Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| filename_id(name).ok().flatten())
                .as_deref()
                == Some(id)
        })
        .collect::<Vec<_>>();
    if selected.len() > 1 {
        return Err(RuntimeError::conflict(
            "Memory chunk identifier is ambiguous",
        ));
    }
    let path = selected
        .first()
        .ok_or_else(|| RuntimeError::not_found("Memory chunk not found"))?;
    let chunk = read(files, path)?;
    if chunk.metadata["id"].as_str() != Some(id) {
        return Err(RuntimeError::invalid(
            "Memory identity differs from filename",
        ));
    }
    Ok(chunk)
}
pub fn render_index(service: &BankService) -> Result<(String, usize)> {
    let mut files = PublicationFiles::new(&service.context.roots.state)?;
    render_index_images(&mut files, paths(service)?)
}
/// Render the same index through the caller's retained state custody.
pub fn render_index_in(files: &mut PublicationFiles) -> Result<(String, usize)> {
    let mut paths = Vec::new();
    if files.archive_directory_exists("memory-bank/chunks")? {
        for name in files.archive_entries("memory-bank/chunks", 1_000_000)? {
            if filename_id(&name)?.is_some() {
                let path = format!("memory-bank/chunks/{name}");
                if files.image_identity(&path)?.is_some() {
                    paths.push(path);
                }
            }
        }
    }
    paths.sort();
    render_index_images(files, paths)
}
fn render_index_images(
    files: &mut PublicationFiles,
    paths: Vec<String>,
) -> Result<(String, usize)> {
    let mut rows = BTreeMap::new();
    for path in paths {
        let chunk = read(files, &path)?;
        let m = &chunk.metadata;
        let id = m["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| RuntimeError::invalid("Memory index row has no ID"))?;
        if rows.contains_key(id) {
            return Err(RuntimeError::conflict("Duplicate Memory Bank chunk ID"));
        }
        let field = |name: &str| {
            m[name]
                .as_str()
                .ok_or_else(|| RuntimeError::invalid("Memory index metadata is incomplete"))
        };
        let row = format!(
            "| {id} | {} | {} | {} | {} | {} | {} | chunks/{} |\n",
            field("title")?,
            field("type")?,
            strings(m, "scope", true)?.join(", "),
            strings(m, "tags", true)?.join(", "),
            field("status")?,
            field("last_verified")?,
            Path::new(&path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("")
        );
        rows.insert(id.to_string(), row);
    }
    let mut ids = rows.keys().collect::<Vec<_>>();
    ids.sort_by_key(|id| {
        let tail = &id[4..];
        if tail.bytes().all(|b| b.is_ascii_digit()) {
            let normalized = tail.trim_start_matches('0');
            let normalized = if normalized.is_empty() {
                "0"
            } else {
                normalized
            };
            let padded = format!(
                "{}{normalized}",
                "0".repeat(20usize.saturating_sub(normalized.len()))
            );
            (0, padded, id.to_string())
        } else {
            (1, id.to_string(), id.to_string())
        }
    });
    let mut preamble = PREAMBLE.to_string();
    if let Some(raw) = files.read("memory-bank/INDEX.md", PLAN_LIMIT)? {
        let text = String::from_utf8(raw)
            .map_err(|_| RuntimeError::invalid("Memory index is not UTF-8"))?
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        if let Some((head, _)) = text.split_once("| ID | Title |") {
            preamble = head.into();
        }
    }
    let body = ids
        .into_iter()
        .map(|id| rows[id].as_str())
        .collect::<String>();
    Ok((format!("{preamble}{HEADER}{body}"), rows.len()))
}
pub fn reindex(service: &BankService) -> Result<Value> {
    let _guard = service.brain()?.lock()?;
    let (rendered, chunks) = render_index(service)?;
    let mut files = PublicationFiles::new(&service.context.roots.state)?;
    let previous = files.read("memory-bank/INDEX.md", PLAN_LIMIT)?;
    let changed = previous.as_deref() != Some(rendered.as_bytes());
    if changed {
        files.write_expected(
            "memory-bank/INDEX.md",
            rendered.as_bytes(),
            previous.as_deref(),
        )?;
    }
    Ok(object!({"index":"memory-bank/INDEX.md","chunks":chunks,"changed":changed}))
}
pub fn strings(value: &Value, key: &str, allow_empty: bool) -> Result<Vec<String>> {
    let array = value[key]
        .as_array()
        .ok_or_else(|| RuntimeError::invalid(format!("{key} must be a list")))?;
    if !allow_empty && array.is_empty() {
        return Err(RuntimeError::invalid(format!("{key} must not be empty")));
    }
    array
        .iter()
        .map(|item| {
            item.as_str()
                .filter(|s| !s.trim().is_empty())
                .map(String::from)
                .ok_or_else(|| {
                    RuntimeError::invalid(format!("{key} must contain non-empty strings"))
                })
        })
        .collect()
}
pub fn today() -> chrono::NaiveDate {
    domain::clock::today()
}
pub fn date(text: &str) -> Result<chrono::NaiveDate> {
    domain::python_date(text).ok_or_else(|| RuntimeError::invalid("Date must be YYYY-MM-DD"))
}
pub fn slug(title: &str) -> Result<String> {
    let expression = regex::Regex::new("[^a-z0-9]+")
        .map_err(|_| RuntimeError::invalid("Invalid slug expression"))?;
    let slug = expression
        .replace_all(&title.to_lowercase(), "-")
        .trim_matches('-')
        .to_string();
    Ok(if slug.is_empty() {
        "promoted".into()
    } else {
        slug
    })
}

#[cfg(test)]
mod constant_pattern_tests {
    use super::*;

    #[test]
    fn memory_names_keep_modern_legacy_unicode_and_slug_boundaries() {
        for _ in 0..2 {
            for (name, id) in [
                (
                    "MEM-20261103-1234abcd-cache-policy.md",
                    "MEM-20261103-1234abcd",
                ),
                ("MEM-0001-cache.md", "MEM-0001"),
                ("MEM-٠٠٠١-cache.md", "MEM-٠٠٠١"),
            ] {
                assert_eq!(filename_id(name).unwrap().as_deref(), Some(id));
                assert!(id_pattern().unwrap().is_match(id));
            }
            for name in [
                "MEM-001-cache.md",
                "MEM-0001-Cache.md",
                "MEM-0001-cache.md.bak",
            ] {
                assert!(filename_id(name).unwrap().is_none());
            }
        }
    }

    #[test]
    fn chunk_delimiters_keep_lf_crlf_and_terminal_body_bytes() {
        for _ in 0..2 {
            for (raw, body) in [
                ("---\n{\"id\":\"MEM-0001\"}\n---\nbody", "body"),
                (
                    "---\r\n{\"id\":\"MEM-0001\"}\r\n---\r\nbody\r\n",
                    "body\r\n",
                ),
                ("---\n{\"id\":\"MEM-0001\"}\n---", ""),
            ] {
                let (metadata, actual_body) = parse_raw(raw.as_bytes()).unwrap();
                assert_eq!(metadata, object!({"id":"MEM-0001"}));
                assert_eq!(actual_body, body);
            }
        }
    }

    #[test]
    fn malformed_chunks_keep_their_specific_errors_on_repeated_calls() {
        for _ in 0..2 {
            for (raw, message) in [
                (&b"\xff"[..], "Memory chunk is not UTF-8"),
                (
                    &b"{}"[..],
                    "Memory frontmatter opening delimiter is missing",
                ),
                (&b"---\n{}\nbody"[..], "Memory frontmatter is unterminated"),
                (
                    &b"---\n[]\n---\n"[..],
                    "Memory frontmatter must be an object",
                ),
            ] {
                let error = parse_raw(raw).unwrap_err();
                assert_eq!(error.kind, crate::ErrorKind::InvalidInput);
                assert_eq!(error.message, message);
            }
        }
    }
}
