//! The state of a workspace before an observed turn.
//!
//! Gate 3 replays a capsule on a fresh copy of the task, which is the
//! workspace as it was before the cold run changed it. For a Git checkout
//! that state is written as a tree into a private object folder, through a
//! private index: the project's own repository is read and never written.
//! Ignored files (installed dependencies, build output) are not part of the
//! tree; they are copied from the workspace when the replay copy is made.
//! When the workspace is not a Git checkout, a bounded directory copy
//! captures its files into a private snapshot folder instead, skipping
//! `.git`, `target`, `.harness-tmp`, the Harness state folder, symlinks,
//! and special files; workspaces exceeding the snapshot limits have no
//! snapshot and so no replay.
use crate::{HarnessError, Result};
use serde_json::{Value, json};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// A snapshot whose new objects exceed this is not kept.
const OBJECT_BYTES: u64 = 512 * 1024 * 1024;
const IGNORED_BYTES: u64 = 1024 * 1024 * 1024;
const IGNORED_FILES: usize = 100_000;
const GIT_SECONDS: u64 = 120;
const TREE_FILES: usize = 50_000;
const TREE_BYTES: u64 = 256 * 1024 * 1024;
const TREE_DEPTH: usize = 32;
const TREE_SECONDS: u64 = 60;
const TREE_SKIPPED_NAMES: [&str; 3] = [".git", "target", ".harness-tmp"];

#[derive(Clone, Copy)]
struct Limits {
    files: usize,
    bytes: u64,
    depth: usize,
    seconds: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            files: TREE_FILES,
            bytes: TREE_BYTES,
            depth: TREE_DEPTH,
            seconds: TREE_SECONDS,
        }
    }
}

fn folder(state: &Path, generation: &str) -> Result<PathBuf> {
    if generation.is_empty()
        || !generation
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(HarnessError::invalid("Invalid launch identity."));
    }
    Ok(state.join("skillcache/snapshots").join(generation))
}

/// Runs Git in `top` with its own index and object folder. The inherited
/// `GIT_*` variables are dropped, hooks and the filesystem monitor are off.
fn git(top: &Path, arguments: &[&str], environment: &[(&str, &Path)]) -> Result<Vec<u8>> {
    let mut command = Command::new("git");
    command
        .args(["--no-optional-locks", "--no-pager", "-c"])
        .arg(format!(
            "core.hooksPath={}",
            if cfg!(windows) { "NUL" } else { "/dev/null" }
        ))
        .args(["-c", "core.fsmonitor=false", "-c", "gc.auto=0", "-C"])
        .arg(top)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (name, _) in std::env::vars_os() {
        if name
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("GIT_")
        {
            command.env_remove(name);
        }
    }
    command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0");
    for (name, value) in environment {
        command.env(name, value);
    }
    let mut child = command.spawn()?;
    let mut stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(stdout) = stdout.as_mut() {
            let _ = std::io::Read::read_to_end(stdout, &mut bytes);
        }
        bytes
    });
    let deadline = Instant::now() + Duration::from_secs(GIT_SECONDS);
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(HarnessError::unavailable(
                    "Git did not finish the workspace snapshot in time.",
                ));
            }
        }
    };
    let bytes = reader.join().unwrap_or_default();
    if !status.success() {
        return Err(HarnessError::unavailable(
            "Git could not snapshot the workspace.",
        ));
    }
    Ok(bytes)
}
fn line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_string()
}
fn tree_bytes(root: &Path) -> u64 {
    let mut total = 0;
    let mut folders = vec![root.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
            match entry.metadata() {
                Ok(metadata) if metadata.is_dir() => folders.push(entry.path()),
                Ok(metadata) => total += metadata.len(),
                Err(_) => {}
            }
        }
    }
    total
}

fn should_skip_name(name: &OsStr) -> bool {
    name.to_str()
        .is_some_and(|name| TREE_SKIPPED_NAMES.contains(&name))
}

struct TreeCopy<'a> {
    state_canon: &'a Path,
    deadline: Instant,
    limits: Limits,
    files: usize,
    bytes: u64,
    skipped_links: usize,
}
impl TreeCopy<'_> {
    fn walk(&mut self, source: &Path, destination: &Path, depth: usize) -> Result<bool> {
        if Instant::now() >= self.deadline {
            return Ok(false);
        }
        if source == self.state_canon || source.starts_with(self.state_canon) {
            return Ok(true);
        }
        let mut entries = std::fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if Instant::now() >= self.deadline {
                return Ok(false);
            }
            let name = entry.file_name();
            if should_skip_name(&name) {
                continue;
            }
            let from = entry.path();
            if from == self.state_canon || from.starts_with(self.state_canon) {
                continue;
            }
            let metadata = match std::fs::symlink_metadata(&from) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            if infra_core::fs::linked_at(&from, &metadata)? {
                self.skipped_links += 1;
                continue;
            }
            let to = destination.join(&name);
            if metadata.is_dir() {
                let next_depth = depth + 1;
                if next_depth > self.limits.depth {
                    return Ok(false);
                }
                infra_core::fs::ensure_directory(&to, true)?;
                if !self.walk(&from, &to, next_depth)? {
                    return Ok(false);
                }
            } else if metadata.is_file() {
                if self.files >= self.limits.files
                    || self.bytes.saturating_add(metadata.len()) > self.limits.bytes
                {
                    return Ok(false);
                }
                let copied = std::fs::copy(&from, &to)?;
                self.files += 1;
                self.bytes = self.bytes.saturating_add(copied);
                if self.bytes > self.limits.bytes {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
}

fn take_tree(
    state: &Path,
    workspace: &Path,
    generation: &str,
    root: &Path,
    limits: Limits,
) -> Result<Option<Value>> {
    if !infra_core::fs::admit_directory(workspace)? {
        return Ok(None);
    }
    let workspace_canon = infra_core::fs::canonicalize(workspace)?;
    remove(state, generation)?;
    let files_root = root.join("files");
    infra_core::fs::ensure_directory(&files_root, true)?;
    let state_canon = match infra_core::fs::canonicalize(state) {
        Ok(path) => path,
        Err(error) => {
            let _ = std::fs::remove_dir_all(root);
            return Err(error.into());
        }
    };
    let mut copy = TreeCopy {
        state_canon: &state_canon,
        deadline: Instant::now() + Duration::from_secs(limits.seconds),
        limits,
        files: 0,
        bytes: 0,
        skipped_links: 0,
    };
    match copy.walk(&workspace_canon, &files_root, 0) {
        Ok(true) => {}
        Ok(false) => {
            let _ = std::fs::remove_dir_all(root);
            return Ok(None);
        }
        Err(error) => {
            let _ = std::fs::remove_dir_all(root);
            return Err(error);
        }
    }
    let snapshot = json!({
        "kind": "tree",
        "workspace": workspace_canon,
        "files": copy.files,
        "bytes": copy.bytes,
        "skipped_links": copy.skipped_links,
    });
    if let Err(error) = infra_core::fs::atomic_json(&root.join("snapshot.json"), &snapshot) {
        let _ = std::fs::remove_dir_all(root);
        return Err(error.into());
    }
    Ok(Some(snapshot))
}

/// Writes the workspace's present state as a private Git tree or, when the
/// workspace is not a Git checkout, as a bounded directory copy. Returns
/// `None` when the workspace is missing or the snapshot would exceed its
/// limits; an observed turn then has no replay copy.
pub fn take(state: &Path, workspace: &Path, generation: &str) -> Result<Option<Value>> {
    take_with(state, workspace, generation, Limits::default())
}

fn take_with(
    state: &Path,
    workspace: &Path,
    generation: &str,
    limits: Limits,
) -> Result<Option<Value>> {
    let root = folder(state, generation)?;
    let Ok(located) = git(
        workspace,
        &[
            "rev-parse",
            "--show-toplevel",
            "--absolute-git-dir",
            "--git-common-dir",
            "--show-prefix",
        ],
        &[],
    ) else {
        return take_tree(state, workspace, generation, &root, limits);
    };
    let text = String::from_utf8_lossy(&located).into_owned();
    let mut lines = text.lines();
    let (Some(top), Some(git_dir), Some(common)) = (lines.next(), lines.next(), lines.next())
    else {
        return take_tree(state, workspace, generation, &root, limits);
    };
    let prefix = lines.next().unwrap_or("").trim_end_matches('/').to_string();
    let top = PathBuf::from(top);
    let git_dir = PathBuf::from(git_dir);
    let common = if Path::new(common).is_absolute() {
        PathBuf::from(common)
    } else {
        workspace.join(common)
    };
    let common = infra_core::fs::canonicalize(&common)?;
    remove(state, generation)?;
    infra_core::fs::ensure_directory(&root.join("objects"), true)?;
    let index = root.join("index");
    if git_dir.join("index").is_file() {
        std::fs::copy(git_dir.join("index"), &index)?;
    }
    let objects = common.join("objects");
    let environment: [(&str, &Path); 3] = [
        ("GIT_INDEX_FILE", &index),
        ("GIT_OBJECT_DIRECTORY", &root.join("objects")),
        ("GIT_ALTERNATE_OBJECT_DIRECTORIES", &objects),
    ];
    let written = git(&top, &["add", "-A"], &environment)
        .and_then(|_| git(&top, &["write-tree"], &environment));
    let tree = match written {
        Ok(tree) => line(&tree),
        Err(error) => {
            let _ = std::fs::remove_dir_all(&root);
            return Err(error);
        }
    };
    if tree.len() < 40 || tree_bytes(&root.join("objects")) > OBJECT_BYTES {
        let _ = std::fs::remove_dir_all(&root);
        return Ok(None);
    }
    let _ = std::fs::remove_file(&index);
    let snapshot = json!({
        "kind": "git",
        "tree": tree,
        "top": top,
        "prefix": prefix,
        "common_dir": common,
        "workspace": workspace,
    });
    infra_core::fs::atomic_json(&root.join("snapshot.json"), &snapshot)?;
    Ok(Some(snapshot))
}

/// The snapshot taken for a launch, if one was.
pub fn load(state: &Path, generation: &str) -> Result<Option<Value>> {
    let path = folder(state, generation)?.join("snapshot.json");
    match infra_core::fs::read_regular(&path) {
        Ok(bytes) => Ok(Some(crate::strict_json::parse(&bytes, 1024 * 1024)?)),
        Err(error) if error.kind == infra_core::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn remove(state: &Path, generation: &str) -> Result<()> {
    let root = folder(state, generation)?;
    match std::fs::remove_dir_all(&root) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Drops every snapshot whose launch is not one of `kept`.
pub fn collect(state: &Path, kept: &[String]) {
    let root = state.join("skillcache/snapshots");
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        if let Some(name) = entry.file_name().to_str()
            && !kept.iter().any(|generation| generation == name)
        {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

struct Copied {
    files: usize,
    bytes: u64,
    truncated: bool,
}
fn copy_entry(from: &Path, to: &Path, copied: &mut Copied) -> std::io::Result<()> {
    if copied.truncated {
        return Ok(());
    }
    let metadata = std::fs::symlink_metadata(from)?;
    if infra_core::fs::linked_at(from, &metadata).unwrap_or(true) {
        return Ok(());
    }
    if metadata.is_dir() {
        std::fs::create_dir_all(to)?;
        let mut entries = std::fs::read_dir(from)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            copy_entry(&entry.path(), &to.join(entry.file_name()), copied)?;
        }
    } else if metadata.is_file() {
        if copied.files >= IGNORED_FILES
            || copied.bytes.saturating_add(metadata.len()) > IGNORED_BYTES
        {
            copied.truncated = true;
            return Ok(());
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(from, to)?;
        copied.files += 1;
        copied.bytes = copied.bytes.saturating_add(metadata.len());
    }
    Ok(())
}

/// Writes the snapshot of `generation` below `destination` and adds any
/// workspace ignored files for Git checkouts. Returns the copy's workspace
/// folder and what was copied.
pub fn materialize(state: &Path, generation: &str, destination: &Path) -> Result<Value> {
    let snapshot = load(state, generation)?.ok_or_else(|| {
        HarnessError::unavailable(
            "This turn has no snapshot of its starting state: the workspace was too large to snapshot or unavailable.",
        )
    })?;
    let root = folder(state, generation)?;
    let kind = if snapshot["kind"].is_null() {
        "git"
    } else {
        snapshot["kind"]
            .as_str()
            .ok_or_else(|| HarnessError::invalid("The snapshot record is not readable."))?
    };
    match kind {
        "git" => materialize_git(&root, &snapshot, destination),
        "tree" => materialize_tree(&root, destination),
        _ => Err(HarnessError::invalid(
            "The snapshot record is not readable.",
        )),
    }
}

fn materialize_git(root: &Path, snapshot: &Value, destination: &Path) -> Result<Value> {
    let text = |name: &str| {
        snapshot[name]
            .as_str()
            .map(String::from)
            .ok_or_else(|| HarnessError::invalid("The snapshot record is not readable."))
    };
    let top = PathBuf::from(text("top")?);
    let common = PathBuf::from(text("common_dir")?);
    let prefix = text("prefix")?;
    let tree = text("tree")?;
    infra_core::fs::ensure_directory(destination, true)?;
    let index = root.join(format!("replay-index-{}", crate::store::hexid()));
    let objects = common.join("objects");
    let environment: [(&str, &Path); 3] = [
        ("GIT_INDEX_FILE", &index),
        ("GIT_OBJECT_DIRECTORY", &root.join("objects")),
        ("GIT_ALTERNATE_OBJECT_DIRECTORIES", &objects),
    ];
    let mut target = destination.display().to_string();
    if !target.ends_with(std::path::MAIN_SEPARATOR) {
        target.push('/');
    }
    let checked_out = git(&top, &["read-tree", &tree], &environment).and_then(|_| {
        git(
            &top,
            &["checkout-index", "-a", &format!("--prefix={target}")],
            &environment,
        )
    });
    let _ = std::fs::remove_file(&index);
    checked_out?;
    // What Git ignores is not in the tree: dependencies and build output are
    // taken from the workspace as they are now.
    let mut copied = Copied {
        files: 0,
        bytes: 0,
        truncated: false,
    };
    let scope = if prefix.is_empty() {
        vec![]
    } else {
        vec!["--", prefix.as_str()]
    };
    let mut arguments = vec![
        "ls-files",
        "--others",
        "--ignored",
        "--exclude-standard",
        "--directory",
        "-z",
    ];
    arguments.extend(scope);
    if let Ok(listed) = git(&top, &arguments, &[]) {
        for name in listed
            .split(|byte| *byte == 0)
            .filter(|name| !name.is_empty())
        {
            let name = String::from_utf8_lossy(name);
            let relative = name.trim_end_matches('/');
            if relative.is_empty()
                || relative
                    .split('/')
                    .any(|part| part == ".git" || part == "..")
            {
                continue;
            }
            let _ = copy_entry(
                &top.join(relative),
                &destination.join(relative),
                &mut copied,
            );
        }
    }
    let workspace = if prefix.is_empty() {
        destination.to_path_buf()
    } else {
        destination.join(&prefix)
    };
    std::fs::create_dir_all(&workspace)?;
    Ok(json!({
        "workspace": workspace,
        "ignored_files": copied.files,
        "ignored_bytes": copied.bytes,
        "ignored_truncated": copied.truncated,
    }))
}

fn copy_tree_materialize(
    source: &Path,
    destination: &Path,
    files: &mut usize,
    bytes: &mut u64,
) -> Result<()> {
    let mut entries = std::fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let from = entry.path();
        let metadata = std::fs::symlink_metadata(&from)?;
        if infra_core::fs::linked_at(&from, &metadata)? {
            return Err(HarnessError::forbidden(
                "The snapshot contains a symbolic link.",
            ));
        }
        let to = destination.join(entry.file_name());
        if metadata.is_dir() {
            infra_core::fs::ensure_directory(&to, true)?;
            copy_tree_materialize(&from, &to, files, bytes)?;
        } else if metadata.is_file() {
            if let Ok(existing) = std::fs::symlink_metadata(&to)
                && infra_core::fs::linked_at(&to, &existing)?
            {
                return Err(HarnessError::forbidden(
                    "The replay destination contains a symbolic link.",
                ));
            }
            let copied = std::fs::copy(&from, &to)?;
            *files += 1;
            *bytes = bytes.saturating_add(copied);
        } else {
            return Err(HarnessError::forbidden(
                "The snapshot contains an unsupported special file.",
            ));
        }
    }
    Ok(())
}

fn materialize_tree(root: &Path, destination: &Path) -> Result<Value> {
    let files_root = root.join("files");
    if !infra_core::fs::admit_directory(&files_root)? {
        return Err(HarnessError::invalid("The snapshot files are missing."));
    }
    infra_core::fs::ensure_directory(destination, true)?;
    let mut files = 0usize;
    let mut bytes = 0u64;
    copy_tree_materialize(&files_root, destination, &mut files, &mut bytes)?;
    Ok(json!({
        "workspace": destination,
        "kind": "tree",
        "ignored_files": 0,
        "ignored_bytes": 0,
        "ignored_truncated": false,
        "files": files,
        "bytes": bytes,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(folder: &Path, arguments: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(folder)
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
            ])
            .args(arguments)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {arguments:?}");
    }

    #[test]
    fn a_snapshot_restores_the_state_before_the_turn_without_touching_the_repository() {
        let root = tempfile::tempdir().unwrap();
        let base = infra_core::fs::canonicalize(root.path()).unwrap();
        let project = base.join("project");
        let state = base.join("state");
        std::fs::create_dir_all(project.join("src")).unwrap();
        std::fs::create_dir_all(project.join("vendor/lib")).unwrap();
        run(&project, &["init", "-q"]);
        std::fs::write(project.join(".gitignore"), "vendor/\n").unwrap();
        std::fs::write(project.join("src/a.txt"), "committed\n").unwrap();
        run(&project, &["add", "-A"]);
        run(&project, &["commit", "-q", "-m", "base"]);
        // The state the turn starts from: a modified tracked file, a new
        // untracked file, and an ignored dependency folder.
        std::fs::write(project.join("src/a.txt"), "edited before the turn\n").unwrap();
        std::fs::write(project.join("notes.txt"), "untracked before the turn\n").unwrap();
        std::fs::write(project.join("vendor/lib/dep.txt"), "dependency\n").unwrap();
        let objects_before = tree_bytes(&project.join(".git/objects"));
        let index_before = std::fs::read(project.join(".git/index")).unwrap();
        let snapshot = take(&state, &project, "gen-1").unwrap().unwrap();
        assert_eq!(snapshot["kind"], "git");
        assert_eq!(snapshot["prefix"], "");
        assert_eq!(tree_bytes(&project.join(".git/objects")), objects_before);
        assert_eq!(
            std::fs::read(project.join(".git/index")).unwrap(),
            index_before
        );
        // The turn changes, creates and deletes files.
        std::fs::write(project.join("src/a.txt"), "changed by the turn\n").unwrap();
        std::fs::write(project.join("out.csv"), "made by the turn\n").unwrap();
        std::fs::remove_file(project.join("notes.txt")).unwrap();
        let replay = base.join("replay");
        let report = materialize(&state, "gen-1", &replay).unwrap();
        assert_eq!(report["workspace"], json!(replay));
        assert_eq!(
            std::fs::read_to_string(replay.join("src/a.txt")).unwrap(),
            "edited before the turn\n"
        );
        assert_eq!(
            std::fs::read_to_string(replay.join("notes.txt")).unwrap(),
            "untracked before the turn\n"
        );
        assert!(
            !replay.join("out.csv").exists(),
            "the turn's output is not in a fresh copy"
        );
        assert_eq!(
            std::fs::read_to_string(replay.join("vendor/lib/dep.txt")).unwrap(),
            "dependency\n"
        );
        assert_eq!(report["ignored_files"], 1);
        assert_eq!(report["ignored_truncated"], false);
        assert!(!replay.join(".git").exists());
        assert_eq!(tree_bytes(&project.join(".git/objects")), objects_before);
        assert_eq!(
            std::fs::read(project.join(".git/index")).unwrap(),
            index_before
        );
        // Snapshots of launches that kept no trace are collected.
        take(&state, &project, "gen-2").unwrap().unwrap();
        collect(&state, &["gen-2".to_string()]);
        assert!(load(&state, "gen-1").unwrap().is_none());
        assert!(load(&state, "gen-2").unwrap().is_some());
        remove(&state, "gen-2").unwrap();
        assert!(materialize(&state, "gen-2", &base.join("replay-2")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn ignored_symlinks_are_not_copied_into_a_git_replay() {
        let root = tempfile::tempdir().unwrap();
        let base = infra_core::fs::canonicalize(root.path()).unwrap();
        let project = base.join("project");
        let state = base.join("state");
        std::fs::create_dir_all(project.join("vendor")).unwrap();
        run(&project, &["init", "-q"]);
        std::fs::write(project.join(".gitignore"), "vendor/\n").unwrap();
        std::fs::write(project.join("tracked.txt"), "ok\n").unwrap();
        run(&project, &["add", "-A"]);
        run(&project, &["commit", "-q", "-m", "base"]);
        let outside = base.join("outside.txt");
        std::fs::write(&outside, "secret\n").unwrap();
        std::fs::write(project.join("vendor/dep.txt"), "dependency\n").unwrap();
        std::os::unix::fs::symlink(&outside, project.join("vendor/escape-link")).unwrap();
        take(&state, &project, "gen-1").unwrap().unwrap();
        let replay = base.join("replay");
        let report = materialize(&state, "gen-1", &replay).unwrap();
        assert_eq!(report["ignored_files"], 1);
        assert_eq!(
            std::fs::read_to_string(replay.join("vendor/dep.txt")).unwrap(),
            "dependency\n"
        );
        assert!(std::fs::symlink_metadata(replay.join("vendor/escape-link")).is_err());
    }

    #[test]
    fn a_folder_that_is_not_a_checkout_is_snapshotted_as_a_tree() {
        let root = tempfile::tempdir().unwrap();
        let base = infra_core::fs::canonicalize(root.path()).unwrap();
        let project = base.join("plain");
        let state = base.join("state");
        std::fs::create_dir_all(project.join("src")).unwrap();
        // Git must not find a repository above the temporary folder.
        if git(&project, &["rev-parse", "--show-toplevel"], &[]).is_ok() {
            eprintln!("skipped: the temporary folder lies inside a Git checkout");
            return;
        }
        std::fs::create_dir_all(project.join("node_modules/pkg")).unwrap();
        std::fs::create_dir_all(project.join(".git")).unwrap();
        std::fs::create_dir_all(project.join("target/debug")).unwrap();
        std::fs::create_dir_all(project.join(".harness-tmp")).unwrap();
        std::fs::write(project.join("src/a.txt"), "before\n").unwrap();
        std::fs::write(project.join("notes.txt"), "untracked\n").unwrap();
        std::fs::write(
            project.join("node_modules/pkg/index.js"),
            "module.exports = 1;\n",
        )
        .unwrap();
        std::fs::write(project.join(".git/marker.txt"), "skip\n").unwrap();
        std::fs::write(project.join("target/debug/bin.dat"), "skip\n").unwrap();
        std::fs::write(project.join(".harness-tmp/scratch.txt"), "skip\n").unwrap();
        #[cfg(unix)]
        {
            let outside = base.join("outside.txt");
            std::fs::write(&outside, "secret\n").unwrap();
            std::os::unix::fs::symlink(&outside, project.join("escape-link")).unwrap();
            let linked_ws = base.join("linked-ws");
            std::os::unix::fs::symlink(&project, &linked_ws).unwrap();
            assert!(take(&state, &linked_ws, "gen-link").is_err());
        }
        assert!(take(&state, &project, "../escape").is_err());

        let snapshot = take(&state, &project, "gen-1").unwrap().unwrap();
        assert_eq!(snapshot["kind"], "tree");
        assert_eq!(snapshot["workspace"], json!(project));
        assert_eq!(snapshot["files"], 3);
        let expected_bytes = "before\n".len() + "untracked\n".len() + "module.exports = 1;\n".len();
        assert_eq!(snapshot["bytes"], expected_bytes as u64);
        #[cfg(unix)]
        assert_eq!(snapshot["skipped_links"], 1);

        // Mutate the workspace after taking the snapshot.
        std::fs::write(project.join("src/a.txt"), "changed by the turn\n").unwrap();
        std::fs::write(project.join("out.csv"), "made by the turn\n").unwrap();
        std::fs::remove_file(project.join("notes.txt")).unwrap();

        let replay = base.join("replay");
        let report = materialize(&state, "gen-1", &replay).unwrap();
        assert_eq!(report["workspace"], json!(replay));
        assert_eq!(report["kind"], "tree");
        assert_eq!(report["ignored_files"], 0);
        assert_eq!(report["ignored_bytes"], 0);
        assert_eq!(report["ignored_truncated"], false);
        assert_eq!(report["files"], 3);
        assert_eq!(report["bytes"], expected_bytes as u64);
        assert_eq!(
            std::fs::read_to_string(replay.join("src/a.txt")).unwrap(),
            "before\n"
        );
        assert_eq!(
            std::fs::read_to_string(replay.join("notes.txt")).unwrap(),
            "untracked\n"
        );
        assert_eq!(
            std::fs::read_to_string(replay.join("node_modules/pkg/index.js")).unwrap(),
            "module.exports = 1;\n"
        );
        assert!(!replay.join("out.csv").exists());
        assert!(!replay.join(".git").exists());
        assert!(!replay.join("target").exists());
        assert!(!replay.join(".harness-tmp").exists());
        #[cfg(unix)]
        assert!(std::fs::symlink_metadata(replay.join("escape-link")).is_err());

        remove(&state, "gen-1").unwrap();
        let error = materialize(&state, "gen-1", &base.join("replay-missing")).unwrap_err();
        assert!(error.to_string().contains("no snapshot"));
        assert!(!error.to_string().contains("Git checkout"));
    }

    #[test]
    fn a_tree_over_its_limits_keeps_no_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let base = infra_core::fs::canonicalize(root.path()).unwrap();
        let project = base.join("plain");
        let state = base.join("state");
        std::fs::create_dir_all(project.join("a/b/c")).unwrap();
        if git(&project, &["rev-parse", "--show-toplevel"], &[]).is_ok() {
            eprintln!("skipped: the temporary folder lies inside a Git checkout");
            return;
        }
        std::fs::write(project.join("one.txt"), "12345").unwrap();
        std::fs::write(project.join("two.txt"), "12345").unwrap();
        std::fs::write(project.join("a/b/c/leaf.txt"), "12345").unwrap();

        let over_files = Limits {
            files: 2,
            ..Limits::default()
        };
        assert!(
            take_with(&state, &project, "over-files", over_files)
                .unwrap()
                .is_none()
        );
        assert!(!state.join("skillcache/snapshots/over-files").exists());
        assert!(load(&state, "over-files").unwrap().is_none());

        let over_bytes = Limits {
            bytes: 10,
            ..Limits::default()
        };
        assert!(
            take_with(&state, &project, "over-bytes", over_bytes)
                .unwrap()
                .is_none()
        );
        assert!(!state.join("skillcache/snapshots/over-bytes").exists());

        let over_depth = Limits {
            depth: 2,
            ..Limits::default()
        };
        assert!(
            take_with(&state, &project, "over-depth", over_depth)
                .unwrap()
                .is_none()
        );
        assert!(!state.join("skillcache/snapshots/over-depth").exists());

        let over_time = Limits {
            seconds: 0,
            ..Limits::default()
        };
        assert!(
            take_with(&state, &project, "over-time", over_time)
                .unwrap()
                .is_none()
        );
        assert!(!state.join("skillcache/snapshots/over-time").exists());

        let within = Limits {
            files: 3,
            bytes: 15,
            depth: 3,
            seconds: 60,
        };
        assert!(
            take_with(&state, &project, "within", within)
                .unwrap()
                .is_some()
        );
        assert!(load(&state, "within").unwrap().is_some());
    }

    #[test]
    fn a_tree_snapshot_skips_state_inside_workspace() {
        let root = tempfile::tempdir().unwrap();
        let base = infra_core::fs::canonicalize(root.path()).unwrap();
        let project = base.join("plain");
        let state = project.join(".harness-state");
        std::fs::create_dir_all(&state).unwrap();
        if git(&project, &["rev-parse", "--show-toplevel"], &[]).is_ok() {
            eprintln!("skipped: the temporary folder lies inside a Git checkout");
            return;
        }
        std::fs::write(project.join("app.txt"), "hello\n").unwrap();
        std::fs::write(state.join("existing.txt"), "internal\n").unwrap();

        let snapshot = take(&state, &project, "gen-1").unwrap().unwrap();
        assert_eq!(snapshot["files"], 1);
        let replay = base.join("replay");
        materialize(&state, "gen-1", &replay).unwrap();
        assert_eq!(
            std::fs::read_to_string(replay.join("app.txt")).unwrap(),
            "hello\n"
        );
        assert!(!replay.join(".harness-state").exists());
    }

    #[cfg(unix)]
    #[test]
    fn materialize_refuses_symlinks_inside_tree_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let base = infra_core::fs::canonicalize(root.path()).unwrap();
        let project = base.join("plain");
        let state = base.join("state");
        std::fs::create_dir_all(&project).unwrap();
        if git(&project, &["rev-parse", "--show-toplevel"], &[]).is_ok() {
            eprintln!("skipped: the temporary folder lies inside a Git checkout");
            return;
        }
        std::fs::write(project.join("app.txt"), "hello\n").unwrap();
        take(&state, &project, "gen-1").unwrap().unwrap();
        let outside = base.join("outside.txt");
        std::fs::write(&outside, "secret\n").unwrap();
        std::os::unix::fs::symlink(
            &outside,
            state.join("skillcache/snapshots/gen-1/files/tampered-link"),
        )
        .unwrap();
        assert!(materialize(&state, "gen-1", &base.join("replay")).is_err());
    }
}
