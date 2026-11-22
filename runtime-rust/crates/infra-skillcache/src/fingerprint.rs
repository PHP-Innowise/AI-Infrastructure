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

/// Computes the tree SHA-256 over explicit `(relative_path, content)` pairs
/// using the exact canonical framing as [`tree_sha256`].
pub fn digest_entries<'a>(entries: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> String {
    let sorted: BTreeMap<&'a str, &'a [u8]> = entries.into_iter().collect();
    let mut hasher = Sha256::new();
    for (name, bytes) in sorted {
        hasher.update((name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
        hasher.update(Sha256::digest(bytes));
    }
    hex(hasher.finalize())
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

fn sandbox_visible_executable(name: &str) -> Option<PathBuf> {
    let path_env = std::env::var_os("PATH")?;
    let home =
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    sandbox_visible_executable_in(name, &path_env, home.as_deref(), &default_temp_roots())
}

fn default_temp_roots() -> Vec<PathBuf> {
    let mut roots = vec![std::env::temp_dir()];
    if let Ok(canon) = infra_core::fs::canonicalize(std::env::temp_dir()) {
        roots.push(canon);
    }
    for fixed in ["/tmp", "/var/tmp", "/private/tmp", "/private/var/folders"] {
        let path = PathBuf::from(fixed);
        roots.push(path.clone());
        if let Ok(canon) = infra_core::fs::canonicalize(&path) {
            roots.push(canon);
        }
    }
    roots
}

fn sandbox_visible_executable_in(
    name: &str,
    path_env: &std::ffi::OsStr,
    home: Option<&Path>,
    temp_roots: &[PathBuf],
) -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    const VISIBLE_PREFIXES: &[&str] = &["/usr", "/bin", "/sbin", "/lib", "/lib64"];
    #[cfg(target_os = "macos")]
    const VISIBLE_PREFIXES: &[&str] = &["/usr", "/bin", "/sbin", "/System", "/Library/Apple"];
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let is_os_visible = |p: &Path| VISIBLE_PREFIXES.iter().any(|prefix| p.starts_with(prefix));
        let is_temp = |p: &Path| temp_roots.iter().any(|tmp| p.starts_with(tmp));
        let canon_home = home.and_then(|h| infra_core::fs::canonicalize(h).ok());
        let is_home_root = |dir: &Path| {
            home.is_some_and(|h| dir == h)
                || canon_home
                    .as_deref()
                    .is_some_and(|h| infra_core::fs::canonicalize(dir).ok().as_deref() == Some(h))
        };
        let path_str = path_env.to_str();
        for dir in std::env::split_paths(path_env) {
            if !dir.is_absolute() {
                continue;
            }
            let candidate = dir.join(name);
            if !candidate.is_file() {
                continue;
            }
            let Ok(canonical) = std::fs::canonicalize(&candidate) else {
                continue;
            };
            if is_os_visible(&dir) && is_os_visible(&canonical) {
                return Some(candidate);
            }
            if is_temp(&dir) || is_temp(&canonical) || is_home_root(&dir) {
                continue;
            }
            let Some(canon_parent) = canonical.parent() else {
                continue;
            };
            if is_home_root(canon_parent) {
                continue;
            }
            let dir_ok = std::fs::symlink_metadata(&dir).is_ok_and(|m| !infra_core::fs::linked(&m));
            if !dir_ok {
                continue;
            }
            let runtime_dirs = infra_host::sandbox::runtime_dirs_for_argv(
                &[candidate.to_string_lossy().into_owned()],
                path_str,
            );
            let canonical_ok = is_os_visible(&canonical)
                || !infra_host::sandbox::runtime_dirs_for_argv(
                    &[canonical.to_string_lossy().into_owned()],
                    path_str,
                )
                .is_empty();
            if !runtime_dirs.is_empty() && canonical_ok {
                return Some(candidate);
            }
        }
        None
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (path_env, home, temp_roots);
        infra_host::provider::find_executable(name)
    }
}

/// The first line an interpreter prints for `--version`, or `None` when it is
/// not installed in a gate-visible location or does not answer within a few seconds.
pub fn runtime_version(name: &str) -> Option<String> {
    if !crate::capsule::is_runtime(name) {
        return None;
    }
    let executable = sandbox_visible_executable(name)?;
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

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn sandbox_visible_executable_accepts_userspace_runtime_dirs_and_rejects_home_or_tmp() {
        let base = std::env::temp_dir().join(format!(
            "sc-fp-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let home = base.join("home");
        let bin_dir = home.join(".local/bin");
        let tmp_dir = base.join("tmp");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::create_dir_all(&tmp_dir).unwrap();

        let make_exec = |p: &Path| {
            std::fs::write(p, b"#!/bin/sh\necho 1.0\n").unwrap();
            let mut perms = std::fs::metadata(p).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(p, perms).unwrap();
        };

        let user_bin = bin_dir.join("node-custom");
        let home_bin = home.join("direct-home-bin");
        let tmp_bin = tmp_dir.join("tmp-bin");
        make_exec(&user_bin);
        make_exec(&home_bin);
        make_exec(&tmp_bin);

        let temp_roots = vec![tmp_dir.clone()];
        let joined = std::env::join_paths([&home, &bin_dir, &tmp_dir]).unwrap();

        assert_eq!(
            sandbox_visible_executable_in("node-custom", &joined, Some(&home), &temp_roots),
            Some(user_bin)
        );
        assert_eq!(
            sandbox_visible_executable_in("direct-home-bin", &joined, Some(&home), &temp_roots),
            None
        );
        assert_eq!(
            sandbox_visible_executable_in("tmp-bin", &joined, Some(&home), &temp_roots),
            None
        );

        let _ = std::fs::remove_dir_all(&base);
    }
}
