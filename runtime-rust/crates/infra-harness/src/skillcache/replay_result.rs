//! Host-side comparison of the regular files changed by observation and replay.
//! Only fingerprints of paths and final bytes are retained, beside the snapshot.
use super::snapshot;
use crate::{HarnessError, Result};
use infra_core::{bank::publication::PublicationFiles, fs as core_fs};
use infra_skillcache::fingerprint::sha256;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const ENTRIES: usize = 50_000;
const BYTES: usize = 256 * 1024 * 1024;
const DEPTH: usize = 32;
const EVIDENCE_BYTES: usize = 8 * 1024 * 1024;
const OBSERVED_TEXT_BYTES: usize = 16 * 1024;
pub(super) const OBSERVED_JSON_BYTES: usize = 1024 * 1024;
const FILE: &str = "observed-result.json";
const META_FILE: &str = "observed-meta.json";
pub(super) const MISSING: &str = "the observed workspace result was not captured within its file, size, depth and time limits; observe a new turn in a workspace the Harness can capture";

pub(super) type Manifest = BTreeMap<String, String>;
// A changed path maps to its final digest, or to None if the file was removed.
pub(super) type Changes = BTreeMap<String, Option<String>>;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Captured {
    pub(super) digests: Manifest,
    pub(super) structural: Manifest,
    pub(super) paths: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ObservedResult {
    pub(super) changes: Changes,
    pub(super) structural: Manifest,
    pub(super) paths: BTreeMap<String, String>,
    pub(super) observed_text: Option<String>,
    pub(super) observed_json: Option<Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct ObservedMeta {
    #[serde(default)]
    created: BTreeSet<String>,
    #[serde(default)]
    mutated: BTreeSet<String>,
    #[serde(default)]
    structural: Manifest,
    #[serde(default)]
    paths: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    observed_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    observed_json: Option<Value>,
}

struct Capture<'a> {
    workspace: &'a Path,
    excluded: Option<&'a Path>,
    allowlist: &'a [String],
    captured: Captured,
    entries_left: usize,
    bytes_left: usize,
    deadline: Instant,
}

impl Capture<'_> {
    fn walk(&mut self, folder: &Path, depth: usize) -> Result<()> {
        let limit = || HarnessError::unavailable("Workspace result capture exceeded its limits.");
        if depth > DEPTH || Instant::now() >= self.deadline {
            return Err(limit());
        }
        // Keep only this folder's custody handles live during a scan. Reads
        // open all path components below the workspace without following links.
        let mut files = PublicationFiles::new(self.workspace)?;
        for entry in fs::read_dir(folder)? {
            self.entries_left = self.entries_left.checked_sub(1).ok_or_else(limit)?;
            if Instant::now() >= self.deadline {
                return Err(limit());
            }
            let entry = entry?;
            let path = entry.path();
            let ignored = entry.file_name().to_str().is_some_and(|name| {
                infra_skillcache::executor::is_ignored_workspace_name_with(name, self.allowlist)
                    || (depth == 0 && snapshot::is_root_skipped_name(name))
            });
            if ignored || self.excluded == Some(path.as_path()) {
                continue;
            }
            let metadata = fs::symlink_metadata(&path)?;
            if core_fs::linked_at(&path, &metadata)? {
                continue;
            }
            if metadata.is_dir() {
                self.walk(&path, depth + 1)?;
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(self.workspace)
                    .ok()
                    .and_then(Path::to_str)
                    .ok_or_else(|| HarnessError::unavailable("A workspace path is not UTF-8."))?;
                let relative = if cfg!(windows) {
                    relative.replace('\\', "/")
                } else {
                    relative.to_owned()
                };
                let (bytes, _) = files
                    .capture_source(&relative, self.bytes_left)?
                    .ok_or_else(|| {
                        HarnessError::unavailable("A workspace file disappeared during capture.")
                    })?;
                self.bytes_left -= bytes.len();
                let path_hash = sha256(relative.as_bytes());
                self.captured
                    .digests
                    .insert(path_hash.clone(), sha256(&bytes));
                if let Some(digest) = structural_digest(&relative, &bytes) {
                    self.captured.structural.insert(path_hash.clone(), digest);
                }
                self.captured
                    .paths
                    .insert(path_hash, mask_relative_path(&relative));
            }
        }
        Ok(())
    }
}

fn mask_relative_path(relative: &str) -> String {
    match infra_core::domain::mask_secrets(relative) {
        Ok((masked, _)) => masked,
        Err(_) => "<masked-path>".into(),
    }
}

fn is_hex_run(bytes: &[u8], start: usize, len: usize) -> bool {
    bytes
        .get(start..start + len)
        .is_some_and(|slice| slice.iter().all(u8::is_ascii_hexdigit))
}

fn match_uuid_at(bytes: &[u8], pos: usize) -> Option<usize> {
    if pos + 36 > bytes.len() {
        return None;
    }
    if pos > 0 && (bytes[pos - 1].is_ascii_alphanumeric() || bytes[pos - 1] == b'-') {
        return None;
    }
    if bytes
        .get(pos + 36)
        .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'-')
    {
        return None;
    }
    if bytes[pos + 8] == b'-'
        && bytes[pos + 13] == b'-'
        && bytes[pos + 18] == b'-'
        && bytes[pos + 23] == b'-'
        && is_hex_run(bytes, pos, 8)
        && is_hex_run(bytes, pos + 9, 4)
        && is_hex_run(bytes, pos + 14, 4)
        && is_hex_run(bytes, pos + 19, 4)
        && is_hex_run(bytes, pos + 24, 12)
    {
        Some(36)
    } else {
        None
    }
}

fn match_iso_timestamp_at(bytes: &[u8], pos: usize) -> Option<usize> {
    // Expects YYYY-MM-DD[T|t| ]HH:MM:SS[(.digits)?(Z|z|[+-]HH:?MM)?]
    if pos + 19 > bytes.len() {
        return None;
    }
    if pos > 0 && bytes[pos - 1].is_ascii_alphanumeric() {
        return None;
    }
    let digits = |start: usize, len: usize| {
        bytes
            .get(start..start + len)
            .is_some_and(|s| s.iter().all(u8::is_ascii_digit))
    };
    if !digits(pos, 4)
        || bytes[pos + 4] != b'-'
        || !digits(pos + 5, 2)
        || bytes[pos + 7] != b'-'
        || !digits(pos + 8, 2)
        || !matches!(bytes[pos + 10], b'T' | b't' | b' ')
        || !digits(pos + 11, 2)
        || bytes[pos + 13] != b':'
        || !digits(pos + 14, 2)
        || bytes[pos + 16] != b':'
        || !digits(pos + 17, 2)
    {
        return None;
    }
    let mut end = pos + 19;
    if bytes.get(end) == Some(&b'.') && bytes.get(end + 1).is_some_and(u8::is_ascii_digit) {
        end += 1;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
    }
    if matches!(bytes.get(end), Some(b'Z' | b'z')) {
        end += 1;
    } else if matches!(bytes.get(end), Some(b'+' | b'-')) && digits(end + 1, 2) {
        let mut tz_end = end + 3;
        if bytes.get(tz_end) == Some(&b':') && digits(tz_end + 1, 2) {
            tz_end += 3;
        } else if digits(tz_end, 2) {
            tz_end += 2;
        }
        if tz_end > end + 3 {
            end = tz_end;
        }
    }
    if bytes.get(end).is_some_and(u8::is_ascii_alphanumeric) {
        return None;
    }
    Some(end - pos)
}

pub(super) fn normalize_volatile_tokens(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        if let Some(len) = match_iso_timestamp_at(bytes, i) {
            out.push_str("<TIMESTAMP>");
            i += len;
            continue;
        }
        if let Some(len) = match_uuid_at(bytes, i) {
            out.push_str("<UUID>");
            i += len;
            continue;
        }
        let ch = text[i..].chars().next().unwrap_or('\0');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn canonicalize_json_inner(value: &Value, sort_arrays: bool) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by_key(|(a, _)| *a);
            let sorted = entries
                .into_iter()
                .map(|(k, v)| (k.clone(), canonicalize_json_inner(v, sort_arrays)))
                .collect::<serde_json::Map<String, Value>>();
            Value::Object(sorted)
        }
        Value::Array(items) => {
            let mut normalized: Vec<(String, Value)> = items
                .iter()
                .map(|item| {
                    let canon = canonicalize_json_inner(item, sort_arrays);
                    let key = if sort_arrays {
                        serde_json::to_string(&canon).unwrap_or_default()
                    } else {
                        String::new()
                    };
                    (key, canon)
                })
                .collect();
            if sort_arrays {
                normalized.sort_by(|(a, _), (b, _)| a.cmp(b));
            }
            Value::Array(normalized.into_iter().map(|(_, v)| v).collect())
        }
        Value::String(s) => Value::String(normalize_volatile_tokens(s)),
        other => other.clone(),
    }
}

fn canonicalize_json(value: &Value) -> Value {
    canonicalize_json_inner(value, true)
}

fn normalize_text(text: &str) -> String {
    let normalized = normalize_volatile_tokens(&text.replace("\r\n", "\n"));
    normalized
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end_matches('\n')
        .to_owned()
}

pub(super) fn structural_digest(relative: &str, bytes: &[u8]) -> Option<String> {
    let lower = relative.to_ascii_lowercase();
    let text = std::str::from_utf8(bytes).ok()?;
    let trimmed = text.trim();
    if (lower.ends_with(".jsonl") || lower.ends_with(".ndjson")) && !trimmed.is_empty() {
        let mut lines = Vec::new();
        let mut all_json = true;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(value) =
                infra_core::strict_json::parse(line.as_bytes(), line.len(), 64, 100_000)
                && let Ok(canon) = serde_json::to_string(&canonicalize_json(&value))
            {
                lines.push(canon);
            } else {
                all_json = false;
                break;
            }
        }
        if all_json && !lines.is_empty() {
            lines.sort();
            return Some(format!(
                "canon-jsonl-v1:{}",
                sha256(lines.join("\n").as_bytes())
            ));
        }
    }
    if (lower.ends_with(".json") || trimmed.starts_with('{') || trimmed.starts_with('['))
        && let Ok(value) = infra_core::strict_json::parse(bytes, bytes.len().max(1), 64, 100_000)
        && let Ok(canon) = serde_json::to_vec(&canonicalize_json(&value))
    {
        return Some(format!("canon-json-v1:{}", sha256(&canon)));
    }
    Some(format!(
        "norm-text-v1:{}",
        sha256(normalize_text(text).as_bytes())
    ))
}

pub(super) fn values_structurally_equal(expected: &Value, actual: &Value) -> bool {
    if expected == actual {
        return true;
    }
    match (expected, actual) {
        (Value::String(a), Value::String(b)) => {
            match (
                structural_digest("", a.as_bytes()),
                structural_digest("", b.as_bytes()),
            ) {
                (Some(da), Some(db)) => da == db,
                _ => false,
            }
        }
        _ => canonicalize_json(expected) == canonicalize_json(actual),
    }
}

#[cfg(test)]
pub(crate) fn capture(workspace: &Path, excluded: Option<&Path>) -> Result<Captured> {
    capture_with(workspace, excluded, &[])
}

pub(crate) fn capture_with(
    workspace: &Path,
    excluded: Option<&Path>,
    allowlist: &[String],
) -> Result<Captured> {
    let mut capture = Capture {
        workspace,
        excluded,
        allowlist,
        captured: Captured::default(),
        entries_left: ENTRIES,
        bytes_left: BYTES,
        deadline: Instant::now() + Duration::from_secs(60),
    };
    capture.walk(workspace, 0)?;
    Ok(capture.captured)
}

pub(super) fn changes(before: &Manifest, after: &Manifest) -> Changes {
    let mut remaining = before.clone();
    let mut changed = Changes::new();
    for (path, digest) in after {
        if remaining.remove(path).as_ref() != Some(digest) {
            changed.insert(path.clone(), Some(digest.clone()));
        }
    }
    changed.extend(remaining.into_keys().map(|path| (path, None)));
    changed
}

pub(super) fn diff(before: &Captured, after: &Captured) -> ObservedResult {
    let changed = changes(&before.digests, &after.digests);
    let mut structural = Manifest::new();
    let mut paths = BTreeMap::new();
    for (path_hash, digest) in &changed {
        if digest.is_some()
            && let Some(struct_digest) = after.structural.get(path_hash)
        {
            structural.insert(path_hash.clone(), struct_digest.clone());
        }
        if let Some(masked) = after
            .paths
            .get(path_hash)
            .or_else(|| before.paths.get(path_hash))
        {
            paths.insert(path_hash.clone(), masked.clone());
        }
    }
    ObservedResult {
        changes: changed,
        structural,
        paths,
        observed_text: None,
        observed_json: None,
    }
}

fn path(state: &Path, generation: &str) -> Result<PathBuf> {
    Ok(snapshot::folder(state, generation)?.join(FILE))
}

fn meta_path(state: &Path, generation: &str) -> Result<PathBuf> {
    Ok(snapshot::folder(state, generation)?.join(META_FILE))
}

/// Returns path hashes created or mutated/deleted during the observed turn so
/// fallback dependency copying in Git materialization skips them.
pub(super) fn contaminated_hashes(snapshot_folder: &Path) -> BTreeSet<String> {
    let Ok(mut files) = PublicationFiles::new(snapshot_folder) else {
        return BTreeSet::new();
    };
    let Ok(Some((bytes, _))) = files.capture_source(META_FILE, EVIDENCE_BYTES) else {
        return BTreeSet::new();
    };
    serde_json::from_slice::<ObservedMeta>(&bytes)
        .map(|mut meta| {
            meta.created.extend(meta.mutated);
            meta.created
        })
        .unwrap_or_default()
}

fn extract_observed_output(text: Option<&str>) -> (Option<String>, Option<Value>) {
    let Some(raw) = text else {
        return (None, None);
    };
    let masked = match infra_core::domain::mask_secrets(raw) {
        Ok((masked, _)) => masked,
        Err(_) => return (None, None),
    };
    let trimmed = masked.trim();
    if trimmed.is_empty() {
        return (None, None);
    }
    // Parse structured JSON over the untruncated output up to OBSERVED_JSON_BYTES
    // so payloads beyond OBSERVED_TEXT_BYTES (16 KiB) are not silently lost.
    let parsed_json = parse_observed_json(trimmed);
    let bounded = if trimmed.len() <= OBSERVED_TEXT_BYTES {
        trimmed.to_owned()
    } else {
        let mut end = OBSERVED_TEXT_BYTES;
        while end > 0 && !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        trimmed[..end].to_owned()
    };
    (Some(bounded), parsed_json)
}

fn parse_observed_json(text: &str) -> Option<Value> {
    let parse_obj_or_arr = |candidate: &str| -> Option<Value> {
        let trimmed = candidate.trim();
        if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
            return None;
        }
        let val =
            infra_core::strict_json::parse(trimmed.as_bytes(), OBSERVED_JSON_BYTES, 64, 100_000)
                .ok()?;
        match &val {
            Value::Object(map) if !map.is_empty() => Some(val),
            Value::Array(arr) if !arr.is_empty() => Some(val),
            _ => None,
        }
    };
    if let Some(val) = parse_obj_or_arr(text) {
        return Some(val);
    }
    let mut candidate = None;
    let mut cursor = text;
    while let Some(start) = cursor.find("```") {
        let after = &cursor[start + 3..];
        let body_start = after.find('\n').map_or(0, |n| n + 1);
        let body_rest = &after[body_start..];
        let Some(end) = body_rest.find("```") else {
            break;
        };
        if let Some(val) = parse_obj_or_arr(&body_rest[..end]) {
            candidate = Some(val);
        }
        cursor = &body_rest[end + 3..];
    }
    if candidate.is_some() {
        return candidate;
    }
    // Fall back to trailing JSON objects/arrays in prose responses.
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if (trimmed.starts_with('{') || trimmed.starts_with('['))
            && let Some(val) = parse_obj_or_arr(&text[offset + (line.len() - trimmed.len())..])
        {
            return Some(val);
        }
        offset += line.len();
    }
    None
}

#[cfg(test)]
pub(crate) fn record(
    state: &Path,
    workspace: &Path,
    generation: &str,
    before: Captured,
) -> Result<()> {
    record_with_output(state, workspace, generation, before, None, &[])
}

fn write_observed_files(
    state: &Path,
    generation: &str,
    changes: &Changes,
    mut meta: ObservedMeta,
    observed_output: Option<&str>,
) -> Result<()> {
    let (observed_text, observed_json) = extract_observed_output(observed_output);
    meta.observed_text = observed_text;
    meta.observed_json = observed_json;
    let bytes = serde_json::to_vec(changes)?;
    if bytes.len() > EVIDENCE_BYTES {
        return Err(HarnessError::unavailable(
            "Too many workspace result changes.",
        ));
    }
    let meta_bytes = serde_json::to_vec(&meta)?;
    if meta_bytes.len() > EVIDENCE_BYTES {
        return Err(HarnessError::unavailable(
            "Too many workspace result changes.",
        ));
    }
    core_fs::atomic_write(&path(state, generation)?, &bytes)?;
    core_fs::atomic_write(&meta_path(state, generation)?, &meta_bytes)?;
    Ok(())
}

pub(crate) fn record_with_output(
    state: &Path,
    workspace: &Path,
    generation: &str,
    before: Captured,
    observed_output: Option<&str>,
    allowlist: &[String],
) -> Result<()> {
    if snapshot::load(state, generation)?.is_none() {
        return Err(HarnessError::unavailable(
            "The starting workspace snapshot is unavailable.",
        ));
    }
    let after = capture_with(workspace, Some(state), allowlist)?;
    let diff = diff(&before, &after);
    let created = diff
        .changes
        .iter()
        .filter(|(hash, digest)| digest.is_some() && !before.digests.contains_key(*hash))
        .map(|(hash, _)| hash.clone())
        .collect::<BTreeSet<_>>();
    let mutated = diff
        .changes
        .iter()
        .filter(|(hash, _)| before.digests.contains_key(*hash))
        .map(|(hash, _)| hash.clone())
        .collect::<BTreeSet<_>>();
    let meta = ObservedMeta {
        created,
        mutated,
        structural: diff.structural,
        paths: diff.paths,
        observed_text: None,
        observed_json: None,
    };
    write_observed_files(state, generation, &diff.changes, meta, observed_output)
}

/// Records empty workspace diffs + output text/JSON for an unrouted `ToolChain`
/// turn whose pre/post lightweight stat fingerprint verified `changes.is_empty()`.
pub(crate) fn record_zero_changes_with_output(
    state: &Path,
    generation: &str,
    observed_output: Option<&str>,
) -> Result<()> {
    if snapshot::load(state, generation)?.is_none() {
        return Err(HarnessError::unavailable(
            "The starting workspace snapshot is unavailable.",
        ));
    }
    write_observed_files(
        state,
        generation,
        &Changes::new(),
        ObservedMeta::default(),
        observed_output,
    )
}

pub(super) fn available(state: &Path, generation: &str) -> bool {
    path(state, generation).ok().is_some_and(|path| {
        fs::symlink_metadata(&path).is_ok_and(|metadata| {
            metadata.is_file()
                && !core_fs::linked(&metadata)
                && metadata.len() <= EVIDENCE_BYTES as u64
        })
    })
}

pub(super) fn load_expected(state: &Path, generation: &str) -> Result<ObservedResult> {
    let mut files = PublicationFiles::new(&snapshot::folder(state, generation)?)?;
    let (bytes, _) = files.capture_source(FILE, EVIDENCE_BYTES)?.ok_or_else(|| {
        HarnessError::unavailable("The observed workspace result is unavailable.")
    })?;
    let changes: Changes = serde_json::from_slice(&bytes)?;
    let meta = files
        .capture_source(META_FILE, EVIDENCE_BYTES)?
        .and_then(|(meta_bytes, _)| serde_json::from_slice::<ObservedMeta>(&meta_bytes).ok())
        .unwrap_or_default();
    Ok(ObservedResult {
        changes,
        structural: meta.structural,
        paths: meta.paths,
        observed_text: meta.observed_text,
        observed_json: meta.observed_json,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_workspace_result_records_additions_edits_and_deletions() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = core_fs::canonicalize(temp.path()).unwrap();
        let write = |path: &str, bytes: &[u8]| fs::write(workspace.join(path), bytes).unwrap();
        for path in ["changed", "deleted", "unchanged"] {
            write(path, b"before");
        }
        fs::create_dir(workspace.join(".harness-tmp")).unwrap();
        write(".harness-tmp/log", b"before");
        fs::create_dir(workspace.join("__pycache__")).unwrap();
        write("__pycache__/mod.pyc", b"before");
        let before = capture(&workspace, None).unwrap();

        write("changed", b"after");
        write("added", b"new");
        write(".harness-tmp/log", b"after");
        write("__pycache__/mod.pyc", b"after");
        fs::remove_file(workspace.join("deleted")).unwrap();
        let after = capture(&workspace, None).unwrap();
        let observed = diff(&before, &after);
        assert_eq!(
            observed.changes,
            Changes::from([
                (sha256(b"added"), Some(sha256(b"new"))),
                (sha256(b"changed"), Some(sha256(b"after"))),
                (sha256(b"deleted"), None),
            ])
        );
        assert_eq!(
            observed.paths.get(&sha256(b"added")).map(String::as_str),
            Some("added")
        );
    }

    #[test]
    fn structural_digests_accept_reformatted_json_and_text_whitespace_only() {
        let a = br#"{"z": [1, 2], "a": {"k": "v"}}"#;
        let b = b"{\n  \"a\": {\n    \"k\": \"v\"\n  },\n  \"z\": [1, 2]\n}\r\n";
        let c = br#"{"z": [1], "a": {"k": "v"}}"#;
        assert_eq!(
            structural_digest("config.json", a),
            structural_digest("config.json", b)
        );
        assert_ne!(
            structural_digest("config.json", a),
            structural_digest("config.json", c)
        );

        let text_a = b"header\r\nrow1   \r\nrow2\r\n";
        let text_b = b"header\nrow1\nrow2\n\n";
        let text_c = b"header\nrow1\n";
        assert_eq!(
            structural_digest("report.md", text_a),
            structural_digest("report.md", text_b)
        );
        assert_ne!(
            structural_digest("report.md", text_a),
            structural_digest("report.md", text_c)
        );
    }

    #[test]
    fn capture_skips_dependency_trees_unless_allowlisted_and_records_observed_output() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = core_fs::canonicalize(temp.path()).unwrap();
        let state = workspace.join(".state");
        fs::create_dir_all(workspace.join("node_modules/pkg")).unwrap();
        fs::write(workspace.join("node_modules/pkg/index.js"), b"v1").unwrap();
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::write(workspace.join("src/a.txt"), b"hello").unwrap();

        let snap = snapshot::take(&state, &workspace, "g1").unwrap().unwrap();
        assert_eq!(snap["kind"], "tree");
        let before = capture(&workspace, Some(&state)).unwrap();
        assert_eq!(before.digests.len(), 1);

        fs::write(workspace.join("node_modules/pkg/index.js"), b"v2").unwrap();
        record_with_output(
            &state,
            &workspace,
            "g1",
            before,
            Some("```json\n{\"total\": 42, \"currency\": \"USD\"}\n```"),
            &[],
        )
        .unwrap();
        let expected = load_expected(&state, "g1").unwrap();
        assert!(expected.changes.is_empty());
        assert_eq!(
            expected.observed_json,
            Some(serde_json::json!({"total": 42, "currency": "USD"}))
        );

        let allow = vec!["node_modules".into()];
        let with_allow = capture_with(&workspace, Some(&state), &allow).unwrap();
        assert_eq!(with_allow.digests.len(), 2);
    }

    #[test]
    fn extract_observed_output_parses_json_past_16kib_and_normalizes_volatile_digests() {
        let big_items: Vec<Value> = (0..500)
            .map(|idx| serde_json::json!({"id": idx, "label": format!("item-{idx:04}-payload-entry")}))
            .collect();
        let big_json = serde_json::to_string(&serde_json::json!({"items": big_items})).unwrap();
        assert!(big_json.len() > OBSERVED_TEXT_BYTES);
        let (text, parsed) = extract_observed_output(Some(&big_json));
        assert_eq!(text.as_ref().unwrap().len(), OBSERVED_TEXT_BYTES);
        assert_eq!(parsed.unwrap()["items"].as_array().unwrap().len(), 500);

        let a = br#"{"run_id": "550e8400-e29b-41d4-a716-446655440000", "generated_at": "2026-01-01T00:00:00Z", "tags": ["beta", "alpha"]}"#;
        let b = br#"{"tags": ["alpha", "beta"], "generated_at": "2026-11-23T14:30:00+02:00", "run_id": "123e4567-e89b-12d3-a456-426614174000"}"#;
        let c = br#"{"tags": ["alpha"], "generated_at": "2026-11-23T14:30:00+02:00", "run_id": "123e4567-e89b-12d3-a456-426614174000"}"#;
        assert_eq!(
            structural_digest("out.json", a),
            structural_digest("out.json", b)
        );
        assert_ne!(
            structural_digest("out.json", a),
            structural_digest("out.json", c)
        );

        let jl_a = b"{\"id\":2,\"ts\":\"2026-01-01T00:00:00Z\"}\n{\"id\":1}\n";
        let jl_b = b"{\"id\":1}\n{\"ts\":\"2026-11-23T12:00:00Z\",\"id\":2}\n";
        assert_eq!(
            structural_digest("events.jsonl", jl_a),
            structural_digest("events.jsonl", jl_b)
        );
    }
}
