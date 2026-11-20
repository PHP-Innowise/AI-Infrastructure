//! The state of a workspace before an observed turn.
//!
//! Gate 3 replays a capsule on a fresh copy of the task, which is the
//! workspace as it was before the cold run changed it. For a Git checkout
//! that state is written as a tree into a private object folder, through a
//! private index: the project's own repository is read and never written.
//! Ignored files (installed dependencies, build output) are not part of the
//! tree; they are copied from the workspace when the replay copy is made.
//! A workspace that is not a Git checkout has no snapshot and so no replay.
use crate::{HarnessError, Result};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// A snapshot whose new objects exceed this is not kept.
const OBJECT_BYTES: u64 = 512 * 1024 * 1024;
const IGNORED_BYTES: u64 = 1024 * 1024 * 1024;
const IGNORED_FILES: usize = 100_000;
const GIT_SECONDS: u64 = 120;

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

/// Writes the workspace's present state as a private Git tree. Returns
/// `None` when the workspace is not a Git checkout or the snapshot would be
/// too large; an observed turn then has no replay copy.
pub fn take(state: &Path, workspace: &Path, generation: &str) -> Result<Option<Value>> {
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
        return Ok(None);
    };
    let text = String::from_utf8_lossy(&located).into_owned();
    let mut lines = text.lines();
    let (Some(top), Some(git_dir), Some(common)) = (lines.next(), lines.next(), lines.next())
    else {
        return Ok(None);
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
    if metadata.file_type().is_symlink() {
        #[cfg(unix)]
        std::os::unix::fs::symlink(std::fs::read_link(from)?, to)?;
    } else if metadata.is_dir() {
        std::fs::create_dir_all(to)?;
        let mut entries = std::fs::read_dir(from)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            copy_entry(&entry.path(), &to.join(entry.file_name()), copied)?;
        }
    } else if metadata.is_file() {
        if copied.files >= IGNORED_FILES || copied.bytes + metadata.len() > IGNORED_BYTES {
            copied.truncated = true;
            return Ok(());
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(from, to)?;
        copied.files += 1;
        copied.bytes += metadata.len();
    }
    Ok(())
}

/// Writes the snapshot of `generation` below `destination` and adds the
/// workspace's ignored files as they are now. Returns the copy's workspace
/// folder and what was copied.
pub fn materialize(state: &Path, generation: &str, destination: &Path) -> Result<Value> {
    let snapshot = load(state, generation)?.ok_or_else(|| {
        HarnessError::unavailable(
            "This turn has no snapshot of its starting state: its workspace is not a Git checkout, or was too large to snapshot.",
        )
    })?;
    let root = folder(state, generation)?;
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

    #[test]
    fn a_folder_that_is_not_a_checkout_has_no_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("plain");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("a.txt"), "x").unwrap();
        // Git must not find a repository above the temporary folder.
        if git(&project, &["rev-parse", "--show-toplevel"], &[]).is_ok() {
            eprintln!("skipped: the temporary folder lies inside a Git checkout");
            return;
        }
        assert!(
            take(&root.path().join("state"), &project, "gen-1")
                .unwrap()
                .is_none()
        );
        assert!(take(&root.path().join("state"), &project, "../escape").is_err());
    }
}
