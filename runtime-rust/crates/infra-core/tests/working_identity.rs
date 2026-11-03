//! The branch a checkout works on: the hooks name their task by it, and a
//! capsule is admitted only while it still holds. HEAD names no branch during
//! a rebase or a bisect, nor in a Codex thread's detached worktree, and the
//! work in hand is still that branch or that thread.
use infra_core::git::GitFrame;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const BRANCH: &str = "feature/rebased";
const THREAD: &str = "0123abcd-4567-89ab-cdef-0123456789ab";

struct Repository {
    root: PathBuf,
    checkout: PathBuf,
}

impl Repository {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "infra-working-identity-{}",
            infra_core::domain::uuid4()
        ));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let repository = Self {
            checkout: root.join("project"),
            root,
        };
        fs::create_dir(&repository.checkout).unwrap();
        repository.git(
            &repository.checkout,
            &["init", "--quiet", "--initial-branch", BRANCH],
        );
        repository.git(
            &repository.checkout,
            &["commit", "--quiet", "--allow-empty", "-m", "base"],
        );
        repository
    }

    fn git(&self, cwd: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args([
                "-c",
                "init.templateDir=",
                "-c",
                "user.name=Identity fixture",
                "-c",
                "user.email=identity@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(args)
            .current_dir(cwd)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CEILING_DIRECTORIES", &self.root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    fn admin(&self, checkout: &Path) -> PathBuf {
        PathBuf::from(self.git(checkout, &["rev-parse", "--absolute-git-dir"]))
    }
}

impl Drop for Repository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn identity(checkout: &Path) -> Option<String> {
    GitFrame::discover(checkout)
        .unwrap()
        .expect("a Git checkout")
        .working_identity()
        .unwrap()
}

#[test]
fn a_rebase_or_a_bisect_keeps_the_branch_it_works_on() {
    let repository = Repository::new();
    let checkout = &repository.checkout;
    assert_eq!(identity(checkout).as_deref(), Some(BRANCH));
    let head = repository.git(checkout, &["rev-parse", "HEAD"]);
    repository.git(checkout, &["checkout", "--quiet", "--detach"]);
    assert_eq!(
        identity(checkout),
        None,
        "another detached HEAD names nothing"
    );

    let admin = repository.admin(checkout);
    for state in ["rebase-merge", "rebase-apply"] {
        fs::create_dir(admin.join(state)).unwrap();
        fs::write(
            admin.join(state).join("head-name"),
            format!("refs/heads/{BRANCH}\n"),
        )
        .unwrap();
        assert_eq!(identity(checkout).as_deref(), Some(BRANCH), "{state}");
        fs::remove_dir_all(admin.join(state)).unwrap();
    }
    fs::write(admin.join("BISECT_START"), format!("{BRANCH}\n")).unwrap();
    assert_eq!(identity(checkout).as_deref(), Some(BRANCH));
    // A bisect started on a detached HEAD names a commit, not a branch.
    fs::write(admin.join("BISECT_START"), format!("{head}\n")).unwrap();
    assert_eq!(identity(checkout), None);
    fs::remove_file(admin.join("BISECT_START")).unwrap();

    // A checked-out branch comes first.
    fs::write(admin.join("BISECT_START"), "another/branch\n").unwrap();
    repository.git(checkout, &["checkout", "--quiet", BRANCH]);
    assert_eq!(identity(checkout).as_deref(), Some(BRANCH));
}

#[test]
fn a_codex_threads_detached_worktree_works_on_its_thread() {
    let repository = Repository::new();
    let worktree = repository.root.join("codex worktree");
    repository.git(
        &repository.checkout,
        &[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            worktree.to_str().unwrap(),
        ],
    );
    assert_eq!(identity(&worktree), None);
    let record = repository.admin(&worktree).join("codex-thread.json");
    fs::write(
        &record,
        format!("{{\"version\":1,\"ownerThreadId\":\"{THREAD}\"}}"),
    )
    .unwrap();
    assert_eq!(identity(&worktree), Some(format!("codex-thread/{THREAD}")));
    assert_eq!(
        identity(&repository.checkout).as_deref(),
        Some(BRANCH),
        "the main checkout keeps its branch"
    );
    for malformed in [
        format!("{{\"version\":2,\"ownerThreadId\":\"{THREAD}\"}}"),
        format!(
            "{{\"version\":1,\"ownerThreadId\":\"{}\"}}",
            THREAD.to_uppercase()
        ),
        "{\"version\":1,\"ownerThreadId\":\"0123abcd-4567-89ab-cdef\"}".into(),
        "not json".into(),
    ] {
        fs::write(&record, &malformed).unwrap();
        assert_eq!(identity(&worktree), None, "{malformed}");
    }
}
