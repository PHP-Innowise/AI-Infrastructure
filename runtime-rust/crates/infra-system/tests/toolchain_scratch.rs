//! Read-only Codex dispatches get a private scratch folder for toolchains when
//! the CLI supports permission profiles, and keep the plain read-only sandbox
//! otherwise. Codex is faked: the probe only runs `codex sandbox … -- true`.
#![cfg(unix)]
use infra_system::native::{prepare_launch, read_only_scratch, use_scratch};
use serde_json::json;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

struct Root(PathBuf);
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn root() -> Root {
    let path = std::env::temp_dir().join(format!("infra-system-scratch-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&path).unwrap();
    Root(fs::canonicalize(path).unwrap())
}
/// A stand-in Codex whose `sandbox` subcommand exits with `code`.
fn codex(root: &Path, name: &str, code: i32) -> PathBuf {
    let path = root.join(name);
    fs::write(
        &path,
        format!("#!/bin/sh\nif [ \"$1\" = sandbox ]; then exit {code}; fi\nexit 0\n"),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}
fn read_only_launch(executable: &Path, project: &Path) -> infra_host::provider::PreparedCommand {
    prepare_launch(
        "codex",
        executable,
        project,
        "inspect",
        &json!({"type":"object"}),
        &project.join("schema.json"),
        "read-only",
        &[],
        None,
    )
    .unwrap()
}

#[test]
fn a_profile_capable_codex_reads_everything_and_writes_only_its_scratch() {
    let root = root();
    let executable = codex(&root.0, "codex-with-profiles", 0);
    let run = root.0.join("run");
    fs::create_dir(&run).unwrap();
    let scratch = read_only_scratch("codex", &executable, "read-only", &run)
        .unwrap()
        .expect("a read-only Codex dispatch gets a scratch folder");
    assert_eq!(scratch, run.join("scratch"));
    assert!(scratch.join("tmp").is_dir() && scratch.join("cache").is_dir());

    let prepared = read_only_launch(&executable, &root.0);
    assert!(
        prepared
            .argv
            .windows(2)
            .any(|pair| pair == ["--sandbox", "read-only"])
    );
    let prepared = use_scratch(prepared, &scratch).unwrap();
    assert!(
        !prepared.argv.iter().any(|argument| argument == "--sandbox"),
        "{:?}",
        prepared.argv
    );
    let profile = prepared
        .argv
        .iter()
        .find(|argument| argument.starts_with("permissions={infrasystemreadonly="))
        .expect("permission profile configured");
    assert!(profile.contains("\":root\"=\"read\""), "{profile}");
    assert!(
        profile.contains(&format!("\"{}\"=\"write\"", scratch.display())),
        "{profile}"
    );
    assert!(profile.contains("network={enabled=false}"), "{profile}");
    assert!(
        prepared
            .argv
            .iter()
            .any(|argument| argument == "default_permissions=\"infrasystemreadonly\"")
    );
    assert_eq!(
        prepared.environment["TMPDIR"],
        scratch.join("tmp").display().to_string()
    );
    assert_eq!(
        prepared.environment["GOCACHE"],
        scratch.join("cache/go-build").display().to_string()
    );
    assert_eq!(
        prepared.environment["CARGO_TARGET_DIR"],
        scratch.join("cargo-target").display().to_string()
    );
}

#[test]
fn edits_other_providers_and_older_codex_keep_their_sandbox() {
    let root = root();
    let run = root.0.join("run");
    fs::create_dir(&run).unwrap();
    let capable = codex(&root.0, "codex-capable", 0);
    let older = codex(&root.0, "codex-older", 2);
    assert!(
        read_only_scratch("codex", &capable, "edit", &run)
            .unwrap()
            .is_none()
    );
    assert!(
        read_only_scratch("claude", &capable, "read-only", &run)
            .unwrap()
            .is_none()
    );
    assert!(
        read_only_scratch("codex", &older, "read-only", &run)
            .unwrap()
            .is_none(),
        "a Codex without permission profiles keeps --sandbox read-only"
    );
    assert!(!run.join("scratch").exists());
}
