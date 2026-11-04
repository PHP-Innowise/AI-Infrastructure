//! File-mode authority comes from an admitted Git frame or an ordinary plain tree.
//! Every marker, index and project below belongs to this fixture.
use infra_core::ErrorKind;
use infra_tooling::file_modes::FileModes;
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

struct Fixture {
    owned: PathBuf,
    repository: PathBuf,
    edition: PathBuf,
    home: PathBuf,
    templates: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let owned = std::env::temp_dir().join(format!("infra-file-modes-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&owned).unwrap();
        // This is deliberately inert, and never the host's /tmp/.git.
        fs::create_dir(owned.join(".git")).unwrap();
        let repository = owned.join("private source");
        let edition = repository.join("Synthetic Edition");
        fs::create_dir_all(edition.join("hooks")).unwrap();
        let home = owned.join("private home");
        let templates = owned.join("empty templates");
        fs::create_dir(&home).unwrap();
        fs::create_dir(&templates).unwrap();
        Self {
            owned,
            repository,
            edition,
            home,
            templates,
        }
    }
    fn git(&self, arguments: &[&str]) -> String {
        self.git_at(&self.repository, arguments)
    }
    fn git_at(&self, directory: &Path, arguments: &[&str]) -> String {
        let mut command = Command::new("git");
        for (key, _) in std::env::vars_os() {
            if key
                .to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("GIT_")
            {
                command.env_remove(key);
            }
        }
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let call = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let stdout = self.owned.join(format!("git-{call}.stdout"));
        let stderr = self.owned.join(format!("git-{call}.stderr"));
        let mut child = command
            .args(["--no-optional-locks", "--no-pager", "-c"])
            .arg(format!("core.hooksPath={}", self.templates.display()))
            .arg("-c")
            .arg(format!("init.templateDir={}", self.templates.display()))
            .args(arguments)
            .current_dir(directory)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.home.join("gitconfig"))
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .stdout(File::create(&stdout).unwrap())
            .stderr(File::create(&stderr).unwrap())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        let status = loop {
            match child.try_wait().unwrap() {
                Some(status) => break status,
                None if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                None => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("fixture Git exceeded 30 seconds: {arguments:?}");
                }
            }
        };
        fn bounded(path: &Path) -> Vec<u8> {
            let mut bytes = Vec::new();
            File::open(path)
                .unwrap()
                .take(64 * 1024 + 1)
                .read_to_end(&mut bytes)
                .unwrap();
            assert!(
                bytes.len() <= 64 * 1024,
                "fixture Git output exceeded 64 KiB"
            );
            bytes
        }
        let stdout = bounded(&stdout);
        let stderr = bounded(&stderr);
        assert!(
            status.success(),
            "Git {:?}: {}",
            arguments,
            String::from_utf8_lossy(&stderr)
        );
        String::from_utf8(stdout).unwrap()
    }
    fn init(&self) {
        self.git(&["init", "--quiet"]);
    }
    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.edition.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }
    fn assert_inert_parent_preserved(&self) {
        assert!(self.owned.join(".git").is_dir());
        assert!(
            fs::read_dir(self.owned.join(".git"))
                .unwrap()
                .next()
                .is_none()
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.owned);
    }
}
#[cfg(unix)]
fn chmod(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}
fn source_git_refusal(root: &Path) {
    let error = FileModes::read(root)
        .err()
        .expect("broken or unsafe Git metadata must not fall back to filesystem authority");
    assert_eq!(error.code.as_deref(), Some("source-git"));
    assert!(matches!(
        error.kind,
        ErrorKind::Unavailable | ErrorKind::Policy | ErrorKind::Io | ErrorKind::Conflict
    ));
}

#[test]
fn empty_ordinary_git_ancestor_is_plain_tree_and_empty_flush_is_safe() {
    let fixture = Fixture::new();
    let path = fixture.file("hooks/ordinary.sh", b"ordinary source\n");
    #[cfg(unix)]
    chmod(&path, 0o644);
    let mut modes = FileModes::read(&fixture.edition)
        .expect("an empty ordinary ancestor .git is not a repository");
    #[cfg(unix)]
    {
        assert_eq!(modes.executable("hooks/ordinary.sh").unwrap(), Some(false));
        assert!(modes.set("hooks/ordinary.sh", true).unwrap());
        assert_eq!(modes.executable("hooks/ordinary.sh").unwrap(), Some(true));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        assert_eq!(modes.executable("hooks/ordinary.sh").unwrap(), None);
        assert!(!modes.set("hooks/ordinary.sh", true).unwrap());
    }
    modes
        .flush()
        .expect("no index updates means no Git query or mutation");
    fixture.assert_inert_parent_preserved();
}

#[test]
fn nonempty_broken_git_directory_and_broken_git_file_remain_refusals() {
    for directory in [true, false] {
        let fixture = Fixture::new();
        fixture.file("hooks/ordinary.sh", b"ordinary source\n");
        let marker = fixture.repository.join(".git");
        if directory {
            fs::create_dir(&marker).unwrap();
            fs::write(
                marker.join("unrecognized-metadata"),
                b"retain this evidence\n",
            )
            .unwrap();
        } else {
            fs::write(&marker, b"gitdir: missing-administration\n").unwrap();
        }
        source_git_refusal(&fixture.edition);
        if directory {
            assert_eq!(
                fs::read(marker.join("unrecognized-metadata")).unwrap(),
                b"retain this evidence\n"
            );
        } else {
            assert_eq!(
                fs::read(&marker).unwrap(),
                b"gitdir: missing-administration\n"
            );
        }
        fixture.assert_inert_parent_preserved();
    }
}

#[cfg(unix)]
#[test]
fn linked_empty_git_marker_is_not_ordinary_empty_metadata() {
    let fixture = Fixture::new();
    let other = fixture.owned.join("unrelated empty directory");
    fs::create_dir(&other).unwrap();
    std::os::unix::fs::symlink(&other, fixture.repository.join(".git")).unwrap();
    source_git_refusal(&fixture.edition);
    assert!(
        fs::symlink_metadata(fixture.repository.join(".git"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(fs::read_dir(&other).unwrap().next().is_none());
    fixture.assert_inert_parent_preserved();
}

#[test]
fn real_git_index_stays_authoritative_with_filemode_false_and_preserves_blob() {
    let fixture = Fixture::new();
    fixture.init();
    let path = fixture.file("hooks/tracked.sh", b"indexed content\n");
    fixture.git(&["add", "--", "Synthetic Edition/hooks/tracked.sh"]);
    fixture.git(&[
        "update-index",
        "--chmod=+x",
        "--",
        "Synthetic Edition/hooks/tracked.sh",
    ]);
    let before = fixture.git(&[
        "ls-files",
        "--stage",
        "--",
        "Synthetic Edition/hooks/tracked.sh",
    ]);
    let before = before
        .split_whitespace()
        .take(3)
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(before[0], "100755");
    fs::write(&path, b"unstaged replacement content\n").unwrap();
    #[cfg(unix)]
    chmod(&path, 0o644);
    fixture.file("hooks/untracked.sh", b"untracked content\n");
    fixture.git(&["config", "core.fileMode", "false"]);
    let mut modes = FileModes::read(&fixture.edition).unwrap();
    assert_eq!(modes.executable("hooks/tracked.sh").unwrap(), Some(true));
    assert_eq!(modes.executable("hooks/untracked.sh").unwrap(), None);
    assert!(modes.set("hooks/tracked.sh", false).unwrap());
    assert!(!modes.set("hooks/untracked.sh", true).unwrap());
    modes.flush().unwrap();
    let after = fixture.git(&[
        "ls-files",
        "--stage",
        "--",
        "Synthetic Edition/hooks/tracked.sh",
    ]);
    let after = after
        .split_whitespace()
        .take(3)
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(after[0], "100644");
    assert_eq!(
        &after[1..],
        &before[1..],
        "a mode repair must keep Git's original blob/stage"
    );
    assert_eq!(fs::read(&path).unwrap(), b"unstaged replacement content\n");
    assert!(
        fixture
            .git(&[
                "diff",
                "--name-only",
                "--",
                "Synthetic Edition/hooks/tracked.sh"
            ])
            .contains("Synthetic Edition/hooks/tracked.sh")
    );
    fixture.assert_inert_parent_preserved();
}

#[test]
fn actual_git_query_failures_are_not_plain_tree_fallbacks() {
    for corrupt_index in [false, true] {
        let fixture = Fixture::new();
        fixture.init();
        fixture.file("hooks/tracked.sh", b"indexed content\n");
        fixture.git(&["add", "--", "Synthetic Edition/hooks/tracked.sh"]);
        if corrupt_index {
            fs::write(fixture.repository.join(".git/index"), b"invalid index\n").unwrap();
        } else {
            fixture.git(&["config", "core.fileMode", "not-a-boolean"]);
        }
        source_git_refusal(&fixture.edition);
        fixture.assert_inert_parent_preserved();
    }
}

#[test]
fn intent_to_add_is_not_staged_by_mode_repair() {
    for trust_disk in [true, false] {
        let fixture = Fixture::new();
        fixture.init();
        let path = fixture.file("hooks/intent.sh", b"intent-to-add content\n");
        #[cfg(unix)]
        chmod(&path, 0o644);
        fixture.git(&["add", "-N", "--", "Synthetic Edition/hooks/intent.sh"]);
        if !trust_disk {
            fixture.git(&["config", "core.fileMode", "false"]);
        }
        let expected = " A Synthetic Edition/hooks/intent.sh\0";
        assert_eq!(
            fixture.git(&["status", "--porcelain=v1", "-z", "--", "Synthetic Edition"]),
            expected
        );
        let mut modes = FileModes::read(&fixture.edition).unwrap();
        #[cfg(unix)]
        if trust_disk {
            assert_eq!(modes.executable("hooks/intent.sh").unwrap(), Some(false));
            assert!(modes.set("hooks/intent.sh", true).unwrap());
        }
        if !trust_disk || !cfg!(unix) {
            assert_eq!(modes.executable("hooks/intent.sh").unwrap(), None);
            assert!(!modes.set("hooks/intent.sh", true).unwrap());
        }
        modes.flush().unwrap();
        assert_eq!(
            fixture.git(&["status", "--porcelain=v1", "-z", "--", "Synthetic Edition"]),
            expected
        );
        assert_eq!(fs::read(&path).unwrap(), b"intent-to-add content\n");
        fixture.assert_inert_parent_preserved();
    }
}

#[test]
fn linked_worktree_git_file_uses_its_own_index_and_canonical_prefix() {
    let fixture = Fixture::new();
    fixture.init();
    fixture.file("hooks/tracked.sh", b"indexed worktree content\n");
    fixture.git(&["add", "--", "Synthetic Edition/hooks/tracked.sh"]);
    fixture.git(&[
        "update-index",
        "--chmod=+x",
        "--",
        "Synthetic Edition/hooks/tracked.sh",
    ]);
    fixture.git(&[
        "-c",
        "user.name=Mode Fixture",
        "-c",
        "user.email=mode-fixture@example.invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "--quiet",
        "--no-verify",
        "-m",
        "fixture",
    ]);
    let worktree = fixture.owned.join("separate linked worktree");
    fixture.git(&[
        "worktree",
        "add",
        "--quiet",
        "--detach",
        worktree.to_str().unwrap(),
        "HEAD",
    ]);
    assert!(
        fs::symlink_metadata(worktree.join(".git"))
            .unwrap()
            .is_file()
    );
    let marker = fs::read(worktree.join(".git")).unwrap();
    assert!(marker.starts_with(b"gitdir: "));
    let main_index = fs::read(fixture.repository.join(".git/index")).unwrap();
    fixture.git(&["config", "core.fileMode", "false"]);
    let mut modes = FileModes::read(&worktree.join("Synthetic Edition")).unwrap();
    assert_eq!(modes.executable("hooks/tracked.sh").unwrap(), Some(true));
    assert!(modes.set("hooks/tracked.sh", false).unwrap());
    modes.flush().unwrap();
    let indexed = fixture.git_at(
        &worktree,
        &[
            "ls-files",
            "--stage",
            "--",
            "Synthetic Edition/hooks/tracked.sh",
        ],
    );
    assert_eq!(indexed.split_whitespace().next(), Some("100644"));
    assert_eq!(
        fs::read(fixture.repository.join(".git/index")).unwrap(),
        main_index
    );
    assert_eq!(fs::read(worktree.join(".git")).unwrap(), marker);
    fixture.assert_inert_parent_preserved();
}
