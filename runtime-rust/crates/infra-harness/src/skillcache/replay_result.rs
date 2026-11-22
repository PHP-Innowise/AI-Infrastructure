//! Host-side comparison of the regular files changed by observation and replay.
//! Only fingerprints of paths and final bytes are retained, beside the snapshot.
use super::snapshot;
use crate::{HarnessError, Result};
use infra_core::{bank::publication::PublicationFiles, fs as core_fs};
use infra_skillcache::fingerprint::sha256;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const ENTRIES: usize = 50_000;
const BYTES: usize = 256 * 1024 * 1024;
const DEPTH: usize = 32;
const EVIDENCE_BYTES: usize = 8 * 1024 * 1024;
const FILE: &str = "observed-result.json";
pub(super) const MISSING: &str = "the observed workspace result was not captured within its file, size, depth and time limits; observe a new turn in a workspace the Harness can capture";

type Manifest = BTreeMap<String, String>;
// A changed path maps to its final digest, or to None if the file was removed.
pub(super) type Changes = BTreeMap<String, Option<String>>;

struct Capture<'a> {
    workspace: &'a Path,
    excluded: Option<&'a Path>,
    manifest: Manifest,
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
                self.manifest
                    .insert(sha256(relative.as_bytes()), sha256(&bytes));
            }
        }
        Ok(())
    }
}

pub(crate) fn capture(workspace: &Path, excluded: Option<&Path>) -> Result<Manifest> {
    let mut capture = Capture {
        workspace,
        excluded,
        manifest: Manifest::new(),
        entries_left: ENTRIES,
        bytes_left: BYTES,
        deadline: Instant::now() + Duration::from_secs(60),
    };
    capture.walk(workspace, 0)?;
    Ok(capture.manifest)
}

pub(super) fn changes(mut before: Manifest, after: Manifest) -> Changes {
    let mut changed = Changes::new();
    for (path, digest) in after {
        if before.remove(&path).as_ref() != Some(&digest) {
            changed.insert(path, Some(digest));
        }
    }
    changed.extend(before.into_keys().map(|path| (path, None)));
    changed
}

fn path(state: &Path, generation: &str) -> Result<PathBuf> {
    Ok(snapshot::folder(state, generation)?.join(FILE))
}

pub(crate) fn record(
    state: &Path,
    workspace: &Path,
    generation: &str,
    before: Manifest,
) -> Result<()> {
    if snapshot::load(state, generation)?.is_none() {
        return Err(HarnessError::unavailable(
            "The starting workspace snapshot is unavailable.",
        ));
    }
    let after = capture(workspace, Some(state))?;
    let bytes = serde_json::to_vec(&changes(before, after))?;
    if bytes.len() > EVIDENCE_BYTES {
        return Err(HarnessError::unavailable(
            "Too many workspace result changes.",
        ));
    }
    core_fs::atomic_write(&path(state, generation)?, &bytes)?;
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

pub(super) fn load(state: &Path, generation: &str) -> Result<Changes> {
    let mut files = PublicationFiles::new(&snapshot::folder(state, generation)?)?;
    let (bytes, _) = files.capture_source(FILE, EVIDENCE_BYTES)?.ok_or_else(|| {
        HarnessError::unavailable("The observed workspace result is unavailable.")
    })?;
    Ok(serde_json::from_slice(&bytes)?)
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
        let before = capture(&workspace, None).unwrap();

        write("changed", b"after");
        write("added", b"new");
        write(".harness-tmp/log", b"after");
        fs::remove_file(workspace.join("deleted")).unwrap();
        let after = capture(&workspace, None).unwrap();
        assert_eq!(
            changes(before, after),
            Changes::from([
                (sha256(b"added"), Some(sha256(b"new"))),
                (sha256(b"changed"), Some(sha256(b"after"))),
                (sha256(b"deleted"), None),
            ])
        );
    }
}
