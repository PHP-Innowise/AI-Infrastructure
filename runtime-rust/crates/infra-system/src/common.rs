use infra_core::bank::publication::PublicationFiles;
use infra_core::{Result, RuntimeError, fs};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
pub const MAX_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_SOURCE_BYTES: usize = 256 * 1024;
pub const MAX_SERVICES: usize = 500;
pub const MAX_ITEMS: usize = 100;
pub fn fail(message: impl Into<String>) -> RuntimeError {
    RuntimeError::invalid(message)
}
pub fn host(error: infra_host::HostError) -> RuntimeError {
    let mut mapped = RuntimeError::unavailable(error.message);
    mapped.code = Some(error.code.to_string());
    mapped
}
pub fn encoded(value: &Value) -> Result<String> {
    infra_core::domain::json::canonical(value)
}
pub fn digest(bytes: &[u8]) -> String {
    infra_core::query::sha256(bytes)
}
pub fn parse_json(raw: &[u8], limit: usize) -> Result<Value> {
    if raw.len() > limit {
        return Err(fail("JSON input exceeds bound"));
    }
    let mut at = 0usize;
    let mut quoted = false;
    let mut escape = false;
    while at < raw.len() {
        let byte = raw[at];
        if quoted {
            if escape {
                escape = false;
            } else if byte == b'\\' {
                escape = true;
            } else if byte == b'"' {
                quoted = false;
            }
            at += 1;
            continue;
        }
        if byte == b'"' {
            quoted = true;
            at += 1;
            continue;
        }
        if byte == b'-' || byte.is_ascii_digit() {
            let start = at;
            at += 1;
            while at < raw.len()
                && (raw[at].is_ascii_digit() || matches!(raw[at], b'.' | b'e' | b'E' | b'+' | b'-'))
            {
                at += 1;
            }
            let token = &raw[start..at];
            if !token.contains(&b'.')
                && !token.contains(&b'e')
                && !token.contains(&b'E')
                && token.len() > 20
            {
                return Err(fail("JSON integer exceeds 20-character bound"));
            }
            continue;
        }
        at += 1;
    }
    infra_host::strict_json::parse(raw, limit).map_err(host)
}
pub fn string(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}
pub fn texts(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}
// Patterns are compiled once: compiled per call, they were most of what a
// 200-service catalog spent its second on. A pattern that cannot compile
// still refuses (sensitive) rather than admits.
static SENSITIVE: std::sync::LazyLock<Option<regex::Regex>> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r#"-----BEGIN [A-Z ]*PRIVATE KEY-----|\bAKIA[A-Z0-9]{16}\b|\bsk-[A-Za-z0-9_-]{20,}|(?i:\b(?:password|api[_-]?key|access[_-]?token)\s*[:=]\s*['"]?[A-Za-z0-9_+/=-]{16,})"#).ok()
});
static IDENTIFIER: std::sync::LazyLock<Option<regex::Regex>> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"^[a-z][a-z0-9._-]{0,79}$").ok());
pub fn sensitive(value: &str) -> bool {
    SENSITIVE
        .as_ref()
        .is_none_or(|pattern| pattern.is_match(value))
}
pub fn text(value: &Value, label: &str, limit: usize) -> Result<String> {
    let value = value
        .as_str()
        .ok_or_else(|| fail(format!("Invalid {label}")))?;
    if value.trim().is_empty()
        || value.chars().count() > limit
        || value
            .chars()
            .any(|character| character < ' ' || character == '\u{7f}')
        || sensitive(value)
    {
        return Err(fail(format!("Invalid or sensitive {label}")));
    }
    Ok(value.to_string())
}
pub fn identifier(value: &str) -> Result<String> {
    let pattern = IDENTIFIER
        .as_ref()
        .ok_or_else(|| fail("Invalid identifier expression"))?;
    if !pattern.is_match(value) {
        let shown = value.chars().take(80).collect::<String>();
        return Err(fail(format!(
            "Invalid identifier {shown:?}: use lowercase letters, digits, '.', '_' or '-', starting with a letter, at most 80 characters"
        )));
    }
    Ok(value.to_string())
}
pub fn hex(value: &str, size: usize) -> bool {
    value.len() == size
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
pub fn fields(value: &Value, required: &[&str], optional: &[&str]) -> Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| fail("Invalid object fields"))?;
    let missing = required
        .iter()
        .filter(|key| !object.contains_key(**key))
        .copied()
        .collect::<Vec<_>>();
    let unknown = object
        .keys()
        .filter(|key| !required.contains(&key.as_str()) && !optional.contains(&key.as_str()))
        .map(|key| key.chars().take(40).collect::<String>())
        .collect::<Vec<_>>();
    if !missing.is_empty() || !unknown.is_empty() {
        let mut parts = vec![];
        if !missing.is_empty() {
            parts.push(format!("missing {}", missing.join(", ")));
        }
        if !unknown.is_empty() {
            parts.push(format!("unknown {}", unknown.join(", ")));
        }
        return Err(fail(format!(
            "Invalid object fields (missing or unknown keys): {}",
            parts.join("; ")
        )));
    }
    Ok(())
}
pub fn items(value: &Value, limit: usize) -> Result<&Vec<Value>> {
    value
        .as_array()
        .filter(|items| items.len() <= limit)
        .ok_or_else(|| fail("Invalid or oversized list"))
}
pub fn choice(value: &Value, allowed: &[&str], label: &str) -> Result<String> {
    let value = string(value);
    if !allowed.contains(&value) {
        return Err(fail(format!(
            "Invalid {label} {:?}: choose {}",
            value.chars().take(40).collect::<String>(),
            allowed.join(", ")
        )));
    }
    Ok(value.to_string())
}
pub fn relative(value: &str) -> Result<String> {
    text(&Value::String(value.into()), "source path", 1024)?;
    let parts = value.split('/').collect::<Vec<_>>();
    let blocked = [
        ".git",
        ".ssh",
        ".aws",
        ".kube",
        "node_modules",
        "vendor",
        ".venv",
        "__pycache__",
        "secrets",
        ".secrets",
        "credentials",
    ];
    if value.starts_with('/')
        || value.contains('\\')
        || parts.iter().any(|part| {
            part.is_empty()
                || *part == "."
                || *part == ".."
                || blocked.contains(&part.to_lowercase().as_str())
                || part.to_lowercase().starts_with(".env")
                || part.contains([':', '<', '>', '"', '|', '?', '*'])
                || part.ends_with('.')
                || part.ends_with(' ')
        })
        || parts.windows(2).any(|parts| {
            ["memory-bank", "project-brain"].contains(&parts[0])
                && ["local", "archive", "dynamic", "control"].contains(&parts[1])
        })
    {
        return Err(fail(format!(
            "Refused source path {:?}: use a relative path to a non-secret file outside dependency, VCS and local runtime folders",
            value.chars().take(200).collect::<String>()
        )));
    }
    let name = parts.last().copied().unwrap_or("").to_lowercase();
    let suffix = Path::new(&name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if ["pem", "key", "p12", "pfx", "sqlite", "db"].contains(&suffix)
        || ["credentials", "credentials.json", "id_rsa", "id_ed25519"].contains(&name.as_str())
    {
        return Err(fail(format!(
            "Refused source path {:?}: keys, certificates, credentials and databases are never context",
            value.chars().take(200).collect::<String>()
        )));
    }
    Ok(value.into())
}
pub fn source(value: &Value) -> Result<()> {
    fields(value, &["path", "kind"], &[])?;
    let path = relative(string(&value["path"]))?;
    choice(
        &value["kind"],
        &["policy", "spec", "contract", "code", "test", "memory"],
        "source kind",
    )?;
    if is_memory(&path) && value["kind"] != "memory" {
        return Err(fail("Memory chunks must use the memory source kind"));
    }
    Ok(())
}
pub fn is_memory(path: &str) -> bool {
    let parts = path.split('/').collect::<Vec<_>>();
    parts.contains(&"memory-bank") && parts.contains(&"chunks")
}
/// A Brain record file, `project-brain/dynamic/<kind>/<uuid>.md`: a promoted
/// memory chunk cites the record it came from.
pub fn is_record_image(path: &str) -> bool {
    let parts = path.split('/').collect::<Vec<_>>();
    parts.len() == 4
        && parts[0] == "project-brain"
        && parts[1] == "dynamic"
        && [
            "findings",
            "decisions",
            "incidents",
            "bugs",
            "events",
            "tasks",
        ]
        .contains(&parts[2])
        && parts[3].strip_suffix(".md").is_some_and(|id| {
            id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
        })
}
/// The digest Memory Bank records for a cited source: SHA-256 of its bytes,
/// with CRLF read as LF unless the first 8192 bytes hold a NUL. A plain
/// digest made every chunk citing a CRLF file look stale.
pub fn memory_digest(raw: &[u8]) -> String {
    if raw[..raw.len().min(8192)].contains(&0) {
        return digest(raw);
    }
    let mut text = Vec::with_capacity(raw.len());
    let mut index = 0;
    while index < raw.len() {
        if raw[index] == b'\r' && raw.get(index + 1) == Some(&b'\n') {
            text.push(b'\n');
            index += 2;
        } else {
            text.push(raw[index]);
            index += 1;
        }
    }
    digest(&text)
}
/// The Memory Bank chunks a root keeps, `memory-bank/chunks/*.md`: the
/// candidates for the shared context of a plan's agents. A missing or linked
/// bank has none; the reader still checks each chunk it is offered.
pub fn memory_chunks(root: &Path) -> Result<Vec<String>> {
    for folder in ["memory-bank", "memory-bank/chunks"] {
        match std::fs::symlink_metadata(root.join(folder)) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return Ok(vec![]),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(error) => return Err(error.into()),
        }
    }
    let mut chunks = vec![];
    for entry in std::fs::read_dir(root.join("memory-bank/chunks"))? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if entry.file_type()?.is_file() && name.ends_with(".md") && !name.starts_with('.') {
            chunks.push(format!("memory-bank/chunks/{name}"));
        }
    }
    chunks.sort();
    Ok(chunks)
}
pub fn absolute(path: &Path) -> Result<PathBuf> {
    fs::absolute(path)
}
pub fn read_file(root: &Path, name: &str, limit: usize) -> Result<Vec<u8>> {
    read_checked(root, name, limit, |_| Ok(()))
}
pub fn read_checked(
    root: &Path,
    name: &str,
    limit: usize,
    before: impl FnOnce(u64) -> Result<()>,
) -> Result<Vec<u8>> {
    relative(name)?;
    let mut files = PublicationFiles::new(root)?;
    files
        .read_checked(name, limit, before)?
        .ok_or_else(|| RuntimeError::not_found("Source file is missing"))
}
pub fn read_json(root: &Path, name: &str, limit: usize) -> Result<Value> {
    parse_json(&read_file(root, name, limit)?, limit)
}
pub fn save(root: &Path, name: &str, value: &Value, new: bool) -> Result<()> {
    relative(name)?;
    if name.contains('/') || name.len() > 255 {
        return Err(fail(
            "Execution output must use a bounded ordinary filename",
        ));
    }
    let raw = (encoded(value)? + "\n").into_bytes();
    if raw.len() > 8 * MAX_BYTES {
        return Err(fail("Execution journal exceeds size limit"));
    }
    let mut files = PublicationFiles::new(root)?;
    let previous = files.read(name, 8 * MAX_BYTES)?;
    if new && previous.is_some() {
        return Err(RuntimeError::conflict("Refused existing execution output"));
    }
    files.write_expected_limit(name, &raw, previous.as_deref(), 8 * MAX_BYTES)
}
pub fn create_bytes_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let root = path.parent().ok_or_else(|| fail("Output has no parent"))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| fail("Output filename is not UTF-8"))?;
    let mut files = PublicationFiles::new(root)?;
    files.write_expected_limit(name, bytes, None, 8 * MAX_BYTES)
}
pub fn path_string(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| fail("Path is not UTF-8"))
}
pub fn id32() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
pub fn warning_key(value: &Value) -> String {
    encoded(value).unwrap_or_default()
}
pub fn unique(values: &[String]) -> bool {
    values.iter().collect::<BTreeSet<_>>().len() == values.len()
}
pub fn receipt_name(step: &Value) -> String {
    format!(
        "{}-a{}.json",
        string(&step["id"]),
        step["attempt"].as_u64().unwrap_or(0)
    )
}
/// The saved input of a dispatch attempt, next to its receipt: the exact
/// prompt its agent received, whose digest the receipt records.
pub fn input_name(step: &Value) -> String {
    format!(
        "{}-a{}.input.json",
        string(&step["id"]),
        step["attempt"].as_u64().unwrap_or(0)
    )
}
pub fn b64(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for block in bytes.chunks(3) {
        let a = block[0] as usize;
        let b = block.get(1).copied().unwrap_or(0) as usize;
        let c = block.get(2).copied().unwrap_or(0) as usize;
        result.push(TABLE[a >> 2] as char);
        result.push(TABLE[((a & 3) << 4) | (b >> 4)] as char);
        result.push(if block.len() > 1 {
            TABLE[((b & 15) << 2) | (c >> 6)] as char
        } else {
            '='
        });
        result.push(if block.len() > 2 {
            TABLE[c & 63] as char
        } else {
            '='
        });
    }
    result
}
pub fn unb64(value: &str) -> Result<Vec<u8>> {
    if !value.len().is_multiple_of(4) {
        return Err(fail("Invalid base64"));
    }
    let mut result = vec![];
    for (index, block) in value.as_bytes().chunks(4).enumerate() {
        let mut n = [0u8; 4];
        for i in 0..4 {
            n[i] = match block[i] {
                b'A'..=b'Z' => block[i] - b'A',
                b'a'..=b'z' => block[i] - b'a' + 26,
                b'0'..=b'9' => block[i] - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' if i >= 2 && index == value.len() / 4 - 1 => 0,
                _ => return Err(fail("Invalid base64")),
            };
        }
        if block[2] == b'=' && (block[3] != b'=' || n[1] & 15 != 0)
            || block[3] == b'=' && block[2] != b'=' && n[2] & 3 != 0
        {
            return Err(fail("Invalid base64"));
        }
        result.push((n[0] << 2) | (n[1] >> 4));
        if block[2] != b'=' {
            result.push((n[1] << 4) | (n[2] >> 2));
        }
        if block[3] != b'=' {
            result.push((n[2] << 6) | n[3]);
        }
    }
    Ok(result)
}

pub fn create_new(root: &Path, name: &str, value: &Value) -> Result<()> {
    save(root, name, value, true)
}
