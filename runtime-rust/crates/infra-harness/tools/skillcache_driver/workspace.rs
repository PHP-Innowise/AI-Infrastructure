//! Isolated workspace materialization for shared observations, preparations, and attempts.
use super::config::{DriverConfig, TaskConfig, check_outside};
use infra_core::fs as core_fs;
use infra_harness::{HarnessError, Result, projects};
use std::{
    fs,
    path::{Path, PathBuf},
};

const MAX_FILES: usize = 50_000;
const MAX_BYTES: u64 = 512 * 1024 * 1024;
const MAX_DEPTH: usize = 32;

struct CopyBudget {
    files: usize,
    bytes: u64,
}

fn copy_tree_no_links(
    src: &Path,
    dst: &Path,
    depth: usize,
    budget: &mut CopyBudget,
    skip_top_git: bool,
) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(HarnessError::invalid(
            "Task fixture directory exceeds maximum nesting depth.",
        ));
    }
    core_fs::ensure_directory(dst, true)?;
    let mut entries = fs::read_dir(src)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        if skip_top_git && depth == 0 && name == ".git" {
            continue;
        }
        // Build and scratch folders of the copied root only: a folder of the
        // same name deeper in the tree is the fixture's own.
        if depth == 0 && matches!(name.to_str(), Some("target" | ".harness-tmp")) {
            continue;
        }
        let src_path = entry.path();
        let meta = fs::symlink_metadata(&src_path)?;
        if core_fs::linked_at(&src_path, &meta)? {
            continue;
        }
        let dst_path = dst.join(&name);
        if meta.is_dir() {
            copy_tree_no_links(&src_path, &dst_path, depth + 1, budget, false)?;
        } else if meta.is_file() {
            budget.files += 1;
            budget.bytes = budget.bytes.saturating_add(meta.len());
            if budget.files > MAX_FILES || budget.bytes > MAX_BYTES {
                return Err(HarnessError::invalid(
                    "Task fixture exceeds bounded file or byte limit.",
                ));
            }
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

fn skill_dir_name(skill_src: &Path) -> Result<String> {
    if let Ok(text) = core_fs::read_text(&skill_src.join("SKILL.md"))
        && let Some(frontmatter) = text.strip_prefix("---\n")
        && let Some(end) = frontmatter.find("\n---")
    {
        for line in frontmatter[..end].lines() {
            if let Some(rest) = line.strip_prefix("name:") {
                let name = rest.trim().trim_matches('"').trim_matches('\'');
                if !name.is_empty()
                    && !name.contains('/')
                    && !name.contains('\\')
                    && !name.contains("..")
                {
                    return Ok(name.to_owned());
                }
            }
        }
    }
    skill_src
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| HarnessError::invalid("Invalid skill folder name."))
}

fn overlay_skills(task: &TaskConfig, dst: &Path) -> Result<bool> {
    if task.skills.is_empty() {
        return Ok(false);
    }
    let mut budget = CopyBudget { files: 0, bytes: 0 };
    for skill_src in &task.skills {
        let name = skill_dir_name(skill_src)?;
        for target_base in [".claude/skills", ".agents/skills"] {
            let target = dst.join(target_base).join(&name);
            core_fs::admit_descendant(dst, &target)?;
            copy_tree_no_links(skill_src, &target, 0, &mut budget, true)?;
        }
    }
    Ok(true)
}

fn run_git(dir: &Path, args: &[&str]) -> Result<()> {
    let owned: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
    let result = projects::git(dir, &owned, 60)?;
    if result.code != 0 {
        return Err(HarnessError::unavailable(format!(
            "git {:?} failed in {} (code {}): {}",
            args,
            dir.display(),
            result.code,
            String::from_utf8_lossy(&result.stdout).trim()
        )));
    }
    Ok(())
}

/// Materializes a fresh workspace for `task` at `dest`. Fails if `dest`
/// already exists or overlaps `state_root` or any `protected` path.
pub fn materialize(
    config: &DriverConfig,
    task: &TaskConfig,
    dest: &Path,
    protected: &[PathBuf],
) -> Result<PathBuf> {
    if dest.exists() {
        return Err(HarnessError::conflict(format!(
            "Workspace {} already exists.",
            dest.display()
        )));
    }
    let mut forbidden = protected.to_vec();
    forbidden.push(config.state_root.clone());
    check_outside(dest, &forbidden, "Attempt workspace")?;
    if let Some(parent) = dest.parent() {
        infra_host::util::private_directory(parent)?;
    }

    let is_git = task.fixture.join(".git").exists();
    if is_git {
        let parent = dest.parent().ok_or_else(|| {
            HarnessError::invalid("Workspace destination must have a parent folder.")
        })?;
        let name = dest
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| HarnessError::invalid("Workspace destination name is not UTF-8."))?;
        let fixture_str = task
            .fixture
            .to_str()
            .ok_or_else(|| HarnessError::invalid("Task fixture path is not UTF-8."))?;
        let null_hooks = format!(
            "core.hooksPath={}",
            if cfg!(windows) { "NUL" } else { "/dev/null" }
        );
        run_git(
            parent,
            &[
                "-c",
                &null_hooks,
                "clone",
                "--no-hardlinks",
                "--quiet",
                fixture_str,
                name,
            ],
        )?;
        // Never leak the sealed fixture's path via `.git/config`.
        let _ = run_git(dest, &["remote", "remove", "origin"]);
        if overlay_skills(task, dest)? {
            run_git(dest, &["add", "-A"])?;
            let _ = run_git(
                dest,
                &[
                    "-c",
                    "user.name=SkillCache",
                    "-c",
                    "user.email=skillcache@local.invalid",
                    "commit",
                    "-q",
                    "-m",
                    "skill overlay",
                ],
            );
        }
    } else {
        let mut budget = CopyBudget { files: 0, bytes: 0 };
        copy_tree_no_links(&task.fixture, dest, 0, &mut budget, true)?;
        let _ = overlay_skills(task, dest)?;
    }
    let canon = core_fs::canonicalize(dest)?;
    check_outside(&canon, &forbidden, "Attempt workspace")?;
    Ok(canon)
}
