//! Content identity of a skill's files and of the environment a capsule was
//! qualified in. A capsule is used only while both still match.
use infra_core::{Result, RuntimeError};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const FILES: usize = 4096;
const BYTES: u64 = 256 * 1024 * 1024;
const DEPTH: usize = 32;

/// What a quick look at one file shows without reading it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FileStat {
    /// `/`-separated path below the tree's root.
    pub path: String,
    pub size: u64,
    pub mtime_ns: i128,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tree {
    /// Lower-case hex SHA-256 over each entry's path and content digest.
    pub sha256: String,
    pub files: Vec<FileStat>,
    /// When the listing was taken, in nanoseconds since the Unix epoch.
    pub taken_ns: i128,
}

/// A file changed this close to the moment it was listed may change again
/// without its recorded time or size showing it, so it is read, not trusted.
const RACY_NS: i128 = 2_000_000_000;

fn now_ns() -> i128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |after| after.as_nanos() as i128)
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
pub fn sha256(bytes: &[u8]) -> String {
    hex(Sha256::digest(bytes))
}

fn mtime_ns(metadata: &std::fs::Metadata) -> i128 {
    metadata
        .modified()
        .ok()
        .map(|time| match time.duration_since(std::time::UNIX_EPOCH) {
            Ok(after) => after.as_nanos() as i128,
            Err(before) => -(before.duration().as_nanos() as i128),
        })
        .unwrap_or_default()
}

enum Entry {
    File(PathBuf),
    /// A link is identified by where it points and never followed.
    Link(String),
}

fn walk(
    root: &Path,
    folder: &Path,
    depth: usize,
    entries: &mut BTreeMap<String, (Entry, std::fs::Metadata)>,
) -> Result<()> {
    if depth > DEPTH {
        return Err(RuntimeError::invalid("The file tree is nested too deeply."));
    }
    let mut names = std::fs::read_dir(folder)?.collect::<std::io::Result<Vec<_>>>()?;
    names.sort_by_key(|entry| entry.file_name());
    for entry in names {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
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
            .ok_or_else(|| RuntimeError::invalid("A file name in the tree is not UTF-8."))?;
        if infra_core::fs::linked(&metadata) {
            let target = std::fs::read_link(&path)
                .map(|target| target.to_string_lossy().into_owned())
                .unwrap_or_default();
            entries.insert(name, (Entry::Link(target), metadata));
        } else if metadata.is_dir() {
            walk(root, &path, depth + 1, entries)?;
        } else if metadata.is_file() {
            entries.insert(name, (Entry::File(path), metadata));
        }
        if entries.len() > FILES {
            return Err(RuntimeError::invalid("The file tree has too many files."));
        }
    }
    Ok(())
}

fn listing(root: &Path) -> Result<BTreeMap<String, (Entry, std::fs::Metadata)>> {
    let metadata = std::fs::symlink_metadata(root)?;
    if infra_core::fs::linked(&metadata) || !metadata.is_dir() {
        return Err(RuntimeError::policy(
            "A fingerprinted tree is an ordinary directory.",
        ));
    }
    let mut entries = BTreeMap::new();
    walk(root, root, 0, &mut entries)?;
    Ok(entries)
}

/// The quick look alone: every file's size and modification time.
pub fn stat_tree(root: &Path) -> Result<Vec<FileStat>> {
    Ok(listing(root)?
        .into_iter()
        .map(|(path, (_, metadata))| FileStat {
            path,
            size: metadata.len(),
            mtime_ns: mtime_ns(&metadata),
        })
        .collect())
}

/// Digest of every regular file below `root`: its path, then the SHA-256 of
/// its bytes. A changed, added, removed or renamed file changes the digest.
pub fn tree_sha256(root: &Path) -> Result<Tree> {
    let mut hasher = Sha256::new();
    let mut files = Vec::new();
    let mut total = 0u64;
    let taken_ns = now_ns();
    for (name, (entry, metadata)) in listing(root)? {
        let digest = match &entry {
            Entry::File(path) => {
                total = total.saturating_add(metadata.len());
                if total > BYTES {
                    return Err(RuntimeError::invalid("The file tree is too large."));
                }
                Sha256::digest(infra_core::fs::read_regular(path)?)
            }
            Entry::Link(target) => Sha256::digest(format!("link:{target}").as_bytes()),
        };
        hasher.update((name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
        hasher.update(digest);
        files.push(FileStat {
            path: name,
            size: metadata.len(),
            mtime_ns: mtime_ns(&metadata),
        });
    }
    Ok(Tree {
        sha256: hex(hasher.finalize()),
        files,
        taken_ns,
    })
}

/// Whether the tree still has the content recorded in `known`. Files whose
/// size and time match the recorded listing are trusted without being read,
/// unless they changed too close to the moment that listing was taken; then,
/// and whenever the listing differs, everything is hashed again. Returns the
/// current tree when it was read.
pub fn verify_tree(root: &Path, known: &Tree) -> Result<(bool, Option<Tree>)> {
    let settled = known
        .files
        .iter()
        .all(|file| file.mtime_ns < known.taken_ns - RACY_NS);
    if settled && stat_tree(root)? == known.files {
        return Ok((true, None));
    }
    let tree = tree_sha256(root)?;
    Ok((tree.sha256 == known.sha256, Some(tree)))
}

/// The first line an interpreter prints for `--version`, or `None` when it is
/// not installed or does not answer within a few seconds.
pub fn runtime_version(name: &str) -> Option<String> {
    if !crate::capsule::is_runtime(name) {
        return None;
    }
    let executable = infra_host::provider::find_executable(name)?;
    let mut child = std::process::Command::new(executable)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let output = child.wait_with_output().ok()?;
    // Python 2 and Java print their version on stderr.
    [output.stdout, output.stderr]
        .iter()
        .filter_map(|stream| {
            String::from_utf8_lossy(stream)
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(|line| line.chars().take(200).collect::<String>())
        })
        .next()
}

/// The version of what the executor and the program tool promise a capsule's
/// programs: the environment contract, the command lines, the answers. It
/// changes only when that contract breaks, so an engine rebuilt for an
/// unrelated reason does not make every capsule stale.
pub const ENV_ABI_VERSION: u64 = 1;

/// Identity of what a capsule's programs run on: the SkillCache runtime
/// contract, the capsule schema and compiler rules, the operating system, the
/// processor and each declared runtime. A runtime that is not installed is
/// part of the identity as `missing`. The engine's own binary digest is not:
/// it is recorded beside the capsule for information only.
pub fn env_fingerprint(runtimes: &BTreeMap<String, Option<String>>) -> String {
    env_fingerprint_for(
        ENV_ABI_VERSION,
        crate::SCHEMA_VERSION,
        &crate::compile_rules::version(),
        runtimes,
    )
}

/// As [`env_fingerprint`], with the contract versions named by the caller.
pub fn env_fingerprint_for(
    abi_version: u64,
    schema_version: u64,
    compile_rules_version: &str,
    runtimes: &BTreeMap<String, Option<String>>,
) -> String {
    let mut hasher = Sha256::new();
    let abi = abi_version.to_string();
    let schema = schema_version.to_string();
    for field in [
        "skillcache-env-v2",
        abi.as_str(),
        schema.as_str(),
        compile_rules_version,
        std::env::consts::OS,
        std::env::consts::ARCH,
    ] {
        hasher.update((field.len() as u64).to_le_bytes());
        hasher.update(field.as_bytes());
    }
    for (name, version) in runtimes {
        let version = version.as_deref().unwrap_or("missing");
        for field in [name.as_str(), version] {
            hasher.update((field.len() as u64).to_le_bytes());
            hasher.update(field.as_bytes());
        }
    }
    hex(hasher.finalize())
}

/// The versions of `names` as they are installed now.
pub fn runtime_versions(names: &[String]) -> BTreeMap<String, Option<String>> {
    names
        .iter()
        .map(|name| (name.clone(), runtime_version(name)))
        .collect()
}
