//! Real disposable SQLite databases exercise the public native queue selector.
//! Child-only TMPDIR/CODEX_HOME keep queue and client state inside the fixture.
#![cfg(unix)]
use serde_json::Value;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};
unsafe extern "C" {
    fn geteuid() -> u32;
}

struct Fixture {
    root: PathBuf,
    project: PathBuf,
    custom: PathBuf,
    spool: PathBuf,
    binary: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "infra-memory-db-queue-{}",
            infra_core::domain::uuid4()
        ));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let project = root.join("project with spaces");
        let custom = root.join("alternate database").join("context.db");
        let spool = root.join("spool");
        fs::create_dir_all(project.join("project-brain/config")).unwrap();
        fs::create_dir_all(project.join("memory-bank/bin")).unwrap();
        fs::create_dir_all(custom.parent().unwrap()).unwrap();
        fs::create_dir(&spool).unwrap();
        fs::write(
            project.join("project-brain/config/runtime.json"),
            br#"{"mode":"lightweight"}"#,
        )
        .unwrap();
        let binary = project.join("memory-bank/bin/infra-engine");
        fs::copy(env!("CARGO_BIN_EXE_infra-engine"), &binary).unwrap();
        let git = Command::new("git")
            .args(["-c", "init.templateDir=", "init", "-q"])
            .arg(&project)
            .env_remove("GIT_DIR")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_CONFIG")
            .env_remove("GIT_CONFIG_PARAMETERS")
            .env_remove("GIT_CONFIG_COUNT")
            .output()
            .unwrap();
        assert!(
            git.status.success(),
            "{}",
            String::from_utf8_lossy(&git.stderr)
        );
        Self {
            root,
            project,
            custom,
            spool,
            binary,
        }
    }
    fn run(&self, database: Option<&Path>, args: &[&str]) -> Output {
        let mut command = Command::new(&self.binary);
        command
            .arg("memory")
            .arg("--root")
            .arg(&self.project)
            .args(["--mode", "lightweight"]);
        if let Some(database) = database {
            command.arg("--db").arg(database);
        }
        command
            .args(args)
            .current_dir(&self.project)
            .env("TMPDIR", &self.spool)
            .env("CODEX_HOME", self.root.join("codex-home"))
            .env("HOME", self.root.join("home"))
            .env_remove("CLAUDE_CODE_TMPDIR")
            .env_remove("INFRA_SOURCE_ROOT")
            .env_remove("ACCELERATOR_HOME")
            .env_remove("ACCELERATOR_STATE_DIR")
            .env_remove("ACCELERATOR_PROJECT_DIR")
            .env_remove("PROJECT_BRAIN_OWNER")
            .env_remove("PROJECT_BRAIN_MODE")
            .env_remove("CONTEXT_QUEUE_DRAIN")
            .env_remove("GIT_DIR")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_CONFIG")
            .env_remove("GIT_CONFIG_PARAMETERS")
            .env_remove("GIT_CONFIG_COUNT")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
    fn success(&self, database: Option<&Path>, args: &[&str]) -> Value {
        let output = self.run(database, args);
        assert!(
            output.status.success(),
            "{args:?}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn queues(&self) -> Vec<PathBuf> {
        fn visit(root: &Path, out: &mut Vec<PathBuf>) {
            for entry in fs::read_dir(root).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let kind = entry.file_type().unwrap();
                if kind.is_dir() {
                    visit(&path, out);
                } else if entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with("command-") && name.ends_with(".json"))
                {
                    out.push(path);
                }
            }
        }
        let mut out = vec![];
        visit(&self.spool, &mut out);
        out
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct ReadOnlyDirectory {
    path: PathBuf,
    mode: u32,
}
impl ReadOnlyDirectory {
    fn new(path: &Path) -> Self {
        let mode = fs::metadata(path).unwrap().permissions().mode();
        fs::set_permissions(path, fs::Permissions::from_mode(mode & !0o222)).unwrap();
        Self {
            path: path.into(),
            mode,
        }
    }
}
impl Drop for ReadOnlyDirectory {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(self.mode));
    }
}

#[test]
fn a_custom_database_update_is_refused_instead_of_queued_for_the_default_database() {
    if unsafe { geteuid() } == 0 {
        eprintln!("permission fixture requires an unprivileged POSIX user");
        return;
    }
    let fixture = Fixture::new();
    fixture.success(
        None,
        &[
            "start",
            "--task-id",
            "DB-QUEUE",
            "--goal",
            "Default database task",
            "--json",
        ],
    );
    fixture.success(
        Some(&fixture.custom),
        &[
            "start",
            "--task-id",
            "DB-QUEUE",
            "--goal",
            "Custom database task",
            "--json",
        ],
    );
    let before_default = fixture.success(None, &["get", "--task-id", "DB-QUEUE", "--json"]);
    let before_custom = fixture.success(
        Some(&fixture.custom),
        &["get", "--task-id", "DB-QUEUE", "--json"],
    );
    let output = {
        let _read_only = ReadOnlyDirectory::new(fixture.custom.parent().unwrap());
        fixture.run(
            Some(&fixture.custom),
            &[
                "update",
                "--task-id",
                "DB-QUEUE",
                "--progress",
                "Must stay in custom database",
                "--json",
            ],
        )
    };
    assert!(
        !output.status.success(),
        "custom database must never be silently queued: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("custom database"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        fixture.queues().is_empty(),
        "refusal must not create deferred commands"
    );
    assert_eq!(
        fixture.success(None, &["get", "--task-id", "DB-QUEUE", "--json"]),
        before_default
    );
    assert_eq!(
        fixture.success(
            Some(&fixture.custom),
            &["get", "--task-id", "DB-QUEUE", "--json"]
        ),
        before_custom
    );
}

#[test]
fn implicit_and_explicit_normalized_default_database_queues_still_drain() {
    if unsafe { geteuid() } == 0 {
        eprintln!("permission fixture requires an unprivileged POSIX user");
        return;
    }
    let fixture = Fixture::new();
    fixture.success(
        None,
        &[
            "start",
            "--task-id",
            "DB-QUEUE",
            "--goal",
            "Default queue remains usable",
            "--json",
        ],
    );
    let local = fixture.project.join("memory-bank/local");
    let explicit_default = local.join(".").join("context.db");
    for (database, progress) in [
        (None, "Implicit default queue"),
        (Some(explicit_default.as_path()), "Explicit default queue"),
    ] {
        let queued = {
            let _read_only = ReadOnlyDirectory::new(&local);
            fixture.success(
                database,
                &[
                    "update",
                    "--task-id",
                    "DB-QUEUE",
                    "--progress",
                    progress,
                    "--json",
                ],
            )
        };
        assert_eq!(queued["queued"], true);
        assert_eq!(fixture.queues().len(), 1);
        let turn = fixture.success(
            None,
            &["turn", "--task-id", "DB-QUEUE", "--flush", "--json"],
        );
        assert!(
            turn["queued_commands"]
                .as_array()
                .is_some_and(|rows| rows.iter().any(|row| row["made"] == true)),
            "{turn}"
        );
        assert!(
            fixture.queues().is_empty(),
            "successful drain consumes its command"
        );
        let task = fixture.success(None, &["get", "--task-id", "DB-QUEUE", "--json"]);
        assert_eq!(task["progress"], progress);
    }
}
