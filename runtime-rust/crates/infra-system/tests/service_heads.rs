//! A catalog asks Git for each checkout's HEAD once, not once per service:
//! services of one monorepo share it, a nested repository keeps its own, and
//! a folder outside any checkout needs no Git process at all.
use infra_system::catalog::{git_head, git_heads};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

struct Root(PathBuf);
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn root() -> Root {
    let path = std::env::temp_dir().join(format!("infra-system-heads-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&path).unwrap();
    // Git cannot use the `\\?\` spelling std gives on Windows.
    Root(infra_core::fs::canonicalize(path).unwrap())
}
fn git(folder: &Path, arguments: &[&str]) {
    let status = Command::new("git")
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
        ])
        .args(["-c", "commit.gpgsign=false", "-C"])
        .arg(folder)
        .args(arguments)
        .status()
        .expect("run git");
    assert!(status.success(), "git {arguments:?}");
}
fn repository(folder: &Path, file: &str) {
    fs::create_dir_all(folder).unwrap();
    git(folder, &["init", "-q"]);
    fs::write(folder.join(file), file).unwrap();
    git(folder, &["add", "."]);
    git(folder, &["commit", "-q", "-m", "fixture"]);
}

#[test]
fn heads_are_asked_once_per_checkout_and_match_each_service_folder() {
    let root = root();
    let mono = root.0.join("mono");
    repository(&mono, "README.md");
    let nested = mono.join("services/legacy");
    repository(&nested, "legacy.txt");
    let single = root.0.join("single");
    repository(&single, "main.go");
    let plain = root.0.join("plain");
    let roots = [
        mono.join("services/orders"),
        mono.join("services/inventory"),
        nested.clone(),
        nested.join("src"),
        single.clone(),
        plain.clone(),
    ];
    for folder in &roots {
        fs::create_dir_all(folder).unwrap();
    }
    let heads = git_heads(&roots);
    let one_by_one = roots.iter().map(|root| git_head(root)).collect::<Vec<_>>();
    assert_eq!(heads, one_by_one);
    assert!(heads[0].is_some() && heads[0] == heads[1], "{heads:?}");
    assert!(heads[2].is_some() && heads[2] == heads[3], "{heads:?}");
    assert_ne!(heads[0], heads[2], "a nested repository keeps its own HEAD");
    assert!(heads[4].is_some() && heads[4] != heads[0]);
}
