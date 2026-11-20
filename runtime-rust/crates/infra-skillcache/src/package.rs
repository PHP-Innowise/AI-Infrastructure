//! A capsule's files on disk.
//!
//! The warm package is what an executor mounts: `capsule.json` (or
//! `capsule.md` for the textual form) with `programs/` and `assets/`, written
//! read-only. The observed values are written apart from it and are never
//! part of the warm tree: they exist only for qualification.
use crate::capsule::{Examples, Form, SkillCapsule, Violation};
use crate::fingerprint;
use infra_core::{Result, RuntimeError};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub const FILES: usize = 128;
pub const FILE_BYTES: usize = 512 * 1024;
pub const TOTAL_BYTES: usize = 4 * 1024 * 1024;
const DEPTH: usize = 8;
pub const CAPSULE_FILE: &str = "capsule.json";
pub const MARKDOWN_FILE: &str = "capsule.md";
pub const EXAMPLES_FILE: &str = "examples.json";

/// The files of a package beside its capsule, by `/`-separated relative path.
#[derive(Clone, Debug, Default)]
pub struct Files {
    files: BTreeMap<String, Vec<u8>>,
    violations: Vec<Violation>,
    bytes: usize,
}

fn plain_component(part: &str) -> bool {
    let mut bytes = part.bytes();
    part.len() <= 100
        && bytes
            .next()
            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && !part.ends_with('.')
}

/// Whether `path` may name a package file: below `programs/` or `assets/`,
/// plain components, a bounded depth.
pub fn admitted_path(path: &str) -> bool {
    let parts = path.split('/').collect::<Vec<_>>();
    (2..=DEPTH).contains(&parts.len())
        && ["programs", "assets"].contains(&parts[0])
        && parts.iter().all(|part| plain_component(part))
}

impl Files {
    pub fn new() -> Self {
        Self::default()
    }
    /// Adds one file. A refused file is recorded as a violation, so a gate
    /// reports it together with everything else.
    pub fn insert(&mut self, path: &str, content: Vec<u8>) {
        let at = format!("files.{path}");
        if !admitted_path(path) {
            self.violations.push(Violation::new(
                "invalid_file_path",
                at,
                "A package file lies below `programs/` or `assets/` and has plain path components.",
            ));
            return;
        }
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".cmd") || lower.ends_with(".bat") {
            self.violations.push(Violation::new(
                "argv_windows_script",
                at,
                "A package carries no `.cmd` or `.bat` launcher.",
            ));
            return;
        }
        if self
            .files
            .keys()
            .any(|known| known.to_ascii_lowercase() == lower)
        {
            self.violations.push(Violation::new(
                "file_path_collision",
                at,
                "Two package files differ only by letter case or are the same path.",
            ));
            return;
        }
        if content.len() > FILE_BYTES {
            self.violations.push(Violation::new(
                "size_limit",
                at,
                format!("A package file is at most {FILE_BYTES} bytes."),
            ));
            return;
        }
        self.bytes += content.len();
        if self.files.len() >= FILES || self.bytes > TOTAL_BYTES {
            self.violations.push(Violation::new(
                "size_limit",
                at,
                format!("A package has at most {FILES} files and {TOTAL_BYTES} bytes."),
            ));
            return;
        }
        self.files.insert(path.to_string(), content);
    }
    /// Reads the `files` list of a compiler's structured answer:
    /// `[{"path": "programs/x/main.py", "content": "..."}]`.
    pub fn from_response(value: &Value) -> Self {
        let mut files = Self::new();
        let Some(items) = value.as_array() else {
            if !value.is_null() {
                files.violations.push(Violation::new(
                    "invalid_type",
                    "files",
                    "Expected a list of files.",
                ));
            }
            return files;
        };
        for (index, item) in items.iter().enumerate() {
            let known = item
                .as_object()
                .is_some_and(|map| map.keys().all(|key| key == "path" || key == "content"));
            match (item["path"].as_str(), item["content"].as_str(), known) {
                (Some(path), Some(content), true) => {
                    files.insert(path, content.as_bytes().to_vec());
                }
                _ => files.violations.push(Violation::new(
                    "invalid_type",
                    format!("files[{index}]"),
                    "A file is `{\"path\": ..., \"content\": ...}` with text content.",
                )),
            }
        }
        files
    }
    pub fn contains(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }
    pub fn get(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }
    pub fn iter(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.files
            .iter()
            .map(|(path, content)| (path.as_str(), content.as_slice()))
    }
    pub fn len(&self) -> usize {
        self.files.len()
    }
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
    pub fn violations(&self) -> Vec<Violation> {
        self.violations.clone()
    }
}

fn set_mode(path: &Path, directory: bool, read_only: bool) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = match (directory, read_only) {
            (true, true) => 0o555,
            (true, false) => 0o700,
            (false, true) => 0o444,
            (false, false) => 0o600,
        };
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    {
        // A directory's read-only attribute does not protect its entries on
        // Windows; the files carry it.
        if !directory {
            let mut permissions = std::fs::metadata(path)?.permissions();
            permissions.set_readonly(read_only);
            std::fs::set_permissions(path, permissions)?;
        }
    }
    Ok(())
}

fn seal(root: &Path, read_only: bool) -> Result<()> {
    let metadata = std::fs::symlink_metadata(root)?;
    if infra_core::fs::linked(&metadata) {
        return Err(RuntimeError::policy("A package holds no links."));
    }
    if metadata.is_dir() {
        if !read_only {
            set_mode(root, true, false)?;
        }
        for entry in std::fs::read_dir(root)? {
            seal(&entry?.path(), read_only)?;
        }
        if read_only {
            set_mode(root, true, true)?;
        }
    } else {
        set_mode(root, false, read_only)?;
    }
    Ok(())
}

/// Removes a package tree, read-only or not. A missing tree is not an error.
pub fn remove(root: &Path) -> Result<()> {
    match std::fs::symlink_metadata(root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
        Ok(metadata) if infra_core::fs::linked(&metadata) || !metadata.is_dir() => {
            return Err(RuntimeError::policy("A package is an ordinary directory."));
        }
        Ok(_) => {}
    }
    seal(root, false)?;
    std::fs::remove_dir_all(root)?;
    Ok(())
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        infra_core::fs::ensure_directory(parent, true)?;
    }
    infra_core::fs::atomic_write(path, bytes)
}

/// Writes the warm package at `root` and returns its digest. The tree is
/// built beside `root` and renamed into place, then made read-only. It holds
/// the capsule, its programs and assets, and nothing that was observed.
pub fn write_warm(
    root: &Path,
    capsule: &SkillCapsule,
    markdown: Option<&[u8]>,
    files: &Files,
) -> Result<String> {
    if !files.violations.is_empty() {
        return Err(RuntimeError::invalid(
            "A package with refused files is not written.",
        ));
    }
    if (capsule.form == Form::Textual) != markdown.is_some()
        || capsule.form == Form::Textual && !files.is_empty()
    {
        return Err(RuntimeError::invalid(
            "A textual capsule is `capsule.md` alone; a hybrid one has no `capsule.md`.",
        ));
    }
    let parent = root
        .parent()
        .ok_or_else(|| RuntimeError::invalid("A package needs a parent directory."))?;
    infra_core::fs::ensure_directory(parent, true)?;
    if std::fs::symlink_metadata(root).is_ok() {
        return Err(RuntimeError::conflict(
            "This capsule revision is already written.",
        ));
    }
    let staging = parent.join(format!(
        ".{}.{}",
        root.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("warm"),
        infra_host::store::hexid()
    ));
    let built = (|| -> Result<()> {
        infra_core::fs::ensure_directory(&staging, true)?;
        write_file(&staging.join(CAPSULE_FILE), &capsule.to_bytes())?;
        if let Some(markdown) = markdown {
            write_file(&staging.join(MARKDOWN_FILE), markdown)?;
        }
        for (path, content) in files.iter() {
            write_file(&staging.join(path), content)?;
        }
        seal(&staging, true)?;
        std::fs::rename(&staging, root)?;
        Ok(())
    })();
    if built.is_err() {
        let _ = remove(&staging);
    }
    built?;
    digest(root)
}

/// Writes the observed values for qualification: a private file outside every
/// warm tree.
pub fn write_examples(path: &Path, examples: &Examples) -> Result<()> {
    if let Some(parent) = path.parent() {
        infra_core::fs::ensure_directory(parent, true)?;
    }
    infra_core::fs::atomic_write(path, &examples.to_bytes())?;
    infra_core::security::secure_private_file(path)
}

/// Digest of everything in a package: any change to it shows.
pub fn digest(root: &Path) -> Result<String> {
    Ok(fingerprint::tree_sha256(root)?.sha256)
}

/// A package read back from disk.
#[derive(Clone, Debug)]
pub struct Package {
    pub root: PathBuf,
    pub capsule: SkillCapsule,
    /// SHA-256 of `capsule.json` as stored.
    pub capsule_sha256: String,
    pub markdown: Option<String>,
    pub files: Files,
}

fn collect(root: &Path, folder: &Path, depth: usize, files: &mut Files) -> Result<()> {
    if depth > DEPTH {
        return Err(RuntimeError::invalid("The package is nested too deeply."));
    }
    let mut entries = std::fs::read_dir(folder)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        if infra_core::fs::linked(&metadata) {
            return Err(RuntimeError::policy("A package holds no links."));
        }
        if metadata.is_dir() {
            collect(root, &path, depth + 1, files)?;
        } else if metadata.is_file() {
            let name = path
                .strip_prefix(root)
                .ok()
                .and_then(|relative| {
                    relative
                        .components()
                        .map(|part| part.as_os_str().to_str())
                        .collect::<Option<Vec<_>>>()
                })
                .map(|parts| parts.join("/"))
                .ok_or_else(|| RuntimeError::invalid("A package file name is not UTF-8."))?;
            if metadata.len() > FILE_BYTES as u64 {
                return Err(RuntimeError::invalid("A package file exceeds its limit."));
            }
            files.insert(&name, infra_core::fs::read_regular(&path)?);
        } else {
            return Err(RuntimeError::policy(
                "A package holds only regular files and directories.",
            ));
        }
    }
    Ok(())
}

/// Reads the package at `root`. It refuses links, special files, an
/// `examples.json` and anything a capsule could not have been written with.
pub fn load(root: &Path) -> Result<Package> {
    let metadata = std::fs::symlink_metadata(root)?;
    if infra_core::fs::linked(&metadata) || !metadata.is_dir() {
        return Err(RuntimeError::policy("A package is an ordinary directory."));
    }
    let bytes = infra_core::fs::read_regular(&root.join(CAPSULE_FILE))?;
    let capsule = SkillCapsule::parse(&bytes).map_err(|violations| {
        RuntimeError::invalid(format!(
            "The stored capsule is not valid: {}",
            violations
                .first()
                .map_or("unreadable", |violation| violation.code)
        ))
    })?;
    let mut files = Files::new();
    let mut markdown = None;
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name();
        let metadata = std::fs::symlink_metadata(entry.path())?;
        if infra_core::fs::linked(&metadata) {
            return Err(RuntimeError::policy("A package holds no links."));
        }
        match name.to_str() {
            Some(CAPSULE_FILE) if metadata.is_file() => {}
            Some(MARKDOWN_FILE) if metadata.is_file() && capsule.form == Form::Textual => {
                markdown = Some(infra_core::fs::read_text(&entry.path())?);
            }
            Some("programs" | "assets") if metadata.is_dir() && capsule.form == Form::Hybrid => {
                collect(root, &entry.path(), 1, &mut files)?;
            }
            Some(EXAMPLES_FILE) => {
                return Err(RuntimeError::policy(
                    "Observed values are never part of a mounted package.",
                ));
            }
            _ => {
                return Err(RuntimeError::policy(
                    "The package holds a file a capsule does not define.",
                ));
            }
        }
    }
    if let Some(violation) = files.violations.first() {
        return Err(RuntimeError::invalid(format!(
            "The stored package is not valid: {}",
            violation.code
        )));
    }
    if capsule.form == Form::Textual && markdown.is_none() {
        return Err(RuntimeError::invalid(
            "A textual capsule has no `capsule.md`.",
        ));
    }
    Ok(Package {
        root: root.to_path_buf(),
        capsule_sha256: fingerprint::sha256(&bytes),
        capsule,
        markdown,
        files,
    })
}
