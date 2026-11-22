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
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct ObservedMeta {
    #[serde(default)]
    created: BTreeSet<String>,
    #[serde(default)]
    structural: Manifest,
    #[serde(default)]
    paths: BTreeMap<String, String>,
}

struct Capture<'a> {
    workspace: &'a Path,
    excluded: Option<&'a Path>,
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
            if snapshot::should_skip_name(&entry.file_name(), depth)
                || self.excluded == Some(path.as_path())
            {
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

fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by_key(|(a, _)| *a);
            let sorted = entries
                .into_iter()
                .map(|(k, v)| (k.clone(), canonicalize_json(v)))
                .collect::<serde_json::Map<String, Value>>();
            Value::Object(sorted)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_json).collect()),
        other => other.clone(),
    }
}

fn normalize_text(text: &str) -> String {
    text.replace("\r\n", "\n")
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

pub(crate) fn capture(workspace: &Path, excluded: Option<&Path>) -> Result<Captured> {
    let mut capture = Capture {
        workspace,
        excluded,
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
    }
}

fn path(state: &Path, generation: &str) -> Result<PathBuf> {
    Ok(snapshot::folder(state, generation)?.join(FILE))
}

fn meta_path(state: &Path, generation: &str) -> Result<PathBuf> {
    Ok(snapshot::folder(state, generation)?.join(META_FILE))
}

pub(super) fn created_hashes(snapshot_folder: &Path) -> BTreeSet<String> {
    let Ok(mut files) = PublicationFiles::new(snapshot_folder) else {
        return BTreeSet::new();
    };
    let Ok(Some((bytes, _))) = files.capture_source(META_FILE, EVIDENCE_BYTES) else {
        return BTreeSet::new();
    };
    serde_json::from_slice::<ObservedMeta>(&bytes)
        .map(|meta| meta.created)
        .unwrap_or_default()
}

pub(crate) fn record(
    state: &Path,
    workspace: &Path,
    generation: &str,
    before: Captured,
) -> Result<()> {
    if snapshot::load(state, generation)?.is_none() {
        return Err(HarnessError::unavailable(
            "The starting workspace snapshot is unavailable.",
        ));
    }
    let after = capture(workspace, Some(state))?;
    let diff = diff(&before, &after);
    let created = diff
        .changes
        .iter()
        .filter(|(hash, digest)| digest.is_some() && !before.digests.contains_key(*hash))
        .map(|(hash, _)| hash.clone())
        .collect::<BTreeSet<_>>();
    let bytes = serde_json::to_vec(&diff.changes)?;
    if bytes.len() > EVIDENCE_BYTES {
        return Err(HarnessError::unavailable(
            "Too many workspace result changes.",
        ));
    }
    let meta = ObservedMeta {
        created,
        structural: diff.structural,
        paths: diff.paths,
    };
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
}
