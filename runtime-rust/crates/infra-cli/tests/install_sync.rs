//! `install --sync` of an existing installation through the actual engine.
//! Each case builds a release of its own under Git, from real PHP Core payload
//! files, and a client project under Git; HOME, CODEX_HOME and the Git
//! configuration stay inside a private temporary root. POSIX only until a
//! native Windows runner has confirmed the same contracts.
#![cfg(unix)]
use serde_json::Value;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

const ENGINE: &str = if cfg!(windows) {
    "memory-bank/bin/infra-engine.exe"
} else {
    "memory-bank/bin/infra-engine"
};
const RECORD: &str = "memory-bank/local/accelerator-install.json";
/// Real payload paths of the PHP Core edition this release carries.
const PAYLOAD: &[&str] = &[
    ".gitattributes",
    ".gitignore",
    "AGENTS.md",
    "README.md",
    "VERSION",
    "memory-bank/INDEX.md",
    "project-brain/config/runtime.json",
    "tasks/.task-counter",
    ".codex/config.toml",
    ".codex/hooks.json",
    ".codex/hooks/working-memory-read.sh",
    ".agents/skills/memory/SKILL.md",
    ".agents/skills/checkpoint/SKILL.md",
];
const MEMORY_SKILL: &str = ".agents/skills/memory/SKILL.md";
const CHECKPOINT_SKILL: &str = ".agents/skills/checkpoint/SKILL.md";
const WIRING: &str = ".codex/hooks.json";
fn repository() -> PathBuf {
    infra_core::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")).unwrap()
}

fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// Copies a release without its Git metadata, as a packaged release ships.
fn package(from: &Path, to: &Path) {
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() == ".git" {
            continue;
        }
        let destination = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            package(&entry.path(), &destination);
        } else {
            write(&destination, fs::read(entry.path()).unwrap());
        }
    }
}

/// Every regular file below `root`, by relative path; links are not followed.
fn files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn visit(root: &Path, here: &Path, result: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(entries) = fs::read_dir(here) else {
            return;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            if metadata.is_dir() {
                visit(root, &path, result);
            } else if metadata.is_file() {
                let name = path.strip_prefix(root).unwrap().to_string_lossy();
                result.insert(name.replace('\\', "/"), fs::read(&path).unwrap());
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

struct Reply {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Reply {
    fn expect(self, code: i32) -> Self {
        assert_eq!(
            self.code, code,
            "stdout: {}\nstderr: {}",
            self.stdout, self.stderr
        );
        self
    }
    fn json(&self) -> Value {
        serde_json::from_str(&self.stdout).unwrap_or_else(|error| {
            panic!(
                "invalid JSON ({error})\nstdout: {}\nstderr: {}",
                self.stdout, self.stderr
            )
        })
    }
}

/// What a sync report says about each path.
fn changed(report: &Value) -> BTreeMap<String, String> {
    report["changed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["path"].as_str().unwrap().to_string(),
                item["action"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn kept(report: &Value) -> BTreeMap<String, String> {
    report["kept"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["path"].as_str().unwrap().to_string(),
                item["reason"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

struct Fixture {
    root: PathBuf,
    home: PathBuf,
    source: PathBuf,
    target: PathBuf,
}

impl Fixture {
    /// A committed release and a client project with one commit of its own.
    fn new() -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "infra-install-sync-{}-{nonce}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let root = infra_core::fs::canonicalize(root).unwrap();
        let home = root.join("private home");
        fs::create_dir(&home).unwrap();
        let fixture = Self {
            source: root.join("accelerator clone"),
            target: root.join("client project"),
            root,
            home,
        };
        let repo = repository();
        let mut inventory: Value = serde_json::from_slice(
            &fs::read(repo.join("install/inventories/php-core.json")).unwrap(),
        )
        .unwrap();
        let mut selected = Vec::new();
        for paths in inventory["installed"].as_object_mut().unwrap().values_mut() {
            paths.as_array_mut().unwrap().retain(|value| {
                let keep = PAYLOAD.contains(&value.as_str().unwrap());
                if keep {
                    selected.push(value.as_str().unwrap().to_string());
                }
                keep
            });
        }
        selected.sort();
        let mut wanted = PAYLOAD
            .iter()
            .map(|path| path.to_string())
            .collect::<Vec<_>>();
        wanted.sort();
        assert_eq!(selected, wanted, "the release must carry declared payload");
        inventory["source_overrides"]
            .as_object_mut()
            .unwrap()
            .retain(|name, _| PAYLOAD.contains(&name.as_str()));
        for name in PAYLOAD {
            let origin = inventory["source_overrides"][*name]
                .as_str()
                .unwrap_or(name)
                .to_string();
            write(
                &fixture.source.join("PHP Core").join(&origin),
                fs::read(repo.join("PHP Core").join(&origin)).unwrap(),
            );
        }
        write(
            &fixture.source.join("install/inventories/php-core.json"),
            serde_json::to_vec_pretty(&inventory).unwrap(),
        );
        write(
            &fixture.source.join("install/native-payload.json"),
            fs::read(repo.join("install/native-payload.json")).unwrap(),
        );
        fixture.git(
            &fixture.source,
            &["init", "--quiet", "--initial-branch=main"],
        );
        fixture.git(&fixture.source, &["add", "--all"]);
        fixture.git(&fixture.source, &["commit", "--quiet", "-m", "release one"]);

        write(
            &fixture.target.join("composer.json"),
            "{\"name\":\"acme/shop\"}\n",
        );
        write(&fixture.target.join("src/Invoice.php"), "<?php\n");
        fixture.git(
            &fixture.target,
            &["init", "--quiet", "--initial-branch=main"],
        );
        fixture.git(&fixture.target, &["add", "--all"]);
        fixture.git(
            &fixture.target,
            &["commit", "--quiet", "-m", "client baseline"],
        );
        fixture
    }

    fn isolated(&self, command: &mut Command) {
        command
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CODEX_HOME", self.home.join("codex"))
            .env("XDG_CONFIG_HOME", self.home.join("config"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.home.join("gitconfig"))
            .env("GIT_TERMINAL_PROMPT", "0")
            // No repository above the fixture (a stray /tmp/.git) may stand
            // in for a release or a project that has none.
            .env("GIT_CEILING_DIRECTORIES", &self.root)
            .stdin(Stdio::null());
        for name in [
            "INFRA_SOURCE_ROOT",
            "ACCELERATOR_HOME",
            "ACCELERATOR_STATE_DIR",
            "ACCELERATOR_PROJECT_DIR",
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_COMMON_DIR",
        ] {
            command.env_remove(name);
        }
    }

    fn git_status(&self, cwd: &Path, args: &[&str]) -> Reply {
        let mut command = Command::new("git");
        self.isolated(&mut command);
        let output = command
            .args([
                "-c",
                "init.templateDir=",
                "-c",
                "user.name=Sync Fixture",
                "-c",
                "user.email=sync-fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "core.autocrlf=false",
            ])
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        Reply {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    fn git(&self, cwd: &Path, args: &[&str]) -> String {
        let reply = self.git_status(cwd, args);
        assert_eq!(reply.code, 0, "git {args:?}: {}", reply.stderr);
        reply.stdout.trim().to_string()
    }

    fn engine(&self, args: &[OsString]) -> Reply {
        let mut command = Command::new(env!("CARGO_BIN_EXE_infra-engine"));
        self.isolated(&mut command);
        let output = command.args(args).current_dir(&self.root).output().unwrap();
        Reply {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    fn install(&self, extra: &[&str]) -> Value {
        let mut args: Vec<OsString> = vec![
            "install".into(),
            "--source-root".into(),
            self.source.clone().into(),
            "--target".into(),
            self.target.clone().into(),
            "--edition".into(),
            "PHP Core".into(),
            "--tool".into(),
            "codex".into(),
            "--json".into(),
        ];
        args.extend(extra.iter().map(OsString::from));
        let reply = self.engine(&args).expect(0);
        assert!(
            self.target.join(ENGINE).is_file(),
            "the install has an engine"
        );
        assert!(
            self.target.join(RECORD).is_file(),
            "the install records itself"
        );
        reply.json()
    }

    fn sync_reply(&self, target: &Path, extra: &[&str]) -> Reply {
        self.sync_from(&self.source, target, extra)
    }

    fn sync_from(&self, source: &Path, target: &Path, extra: &[&str]) -> Reply {
        let mut args: Vec<OsString> = vec![
            "install".into(),
            "--sync".into(),
            "--source-root".into(),
            source.into(),
            "--target".into(),
            target.into(),
            "--json".into(),
        ];
        args.extend(extra.iter().map(OsString::from));
        self.engine(&args)
    }

    fn sync(&self, extra: &[&str]) -> Value {
        self.sync_reply(&self.target, extra).expect(0).json()
    }

    /// Publishes a new release commit with `change` applied to one file.
    fn release(&self, path: &str, change: impl FnOnce(String) -> String) -> String {
        let file = self.source.join("PHP Core").join(path);
        fs::write(&file, change(text(&file))).unwrap();
        self.git(&self.source, &["commit", "--quiet", "--all", "-m", path]);
        self.git(&self.source, &["rev-parse", "HEAD"])
    }

    fn project(&self, path: &str) -> PathBuf {
        self.target.join(path)
    }

    fn record(&self) -> Value {
        serde_json::from_slice(&fs::read(self.project(RECORD)).unwrap()).unwrap()
    }

    /// Whether the project's Git ignores `path` (`git check-ignore`).
    fn ignored(&self, path: &str) -> bool {
        match self
            .git_status(&self.target, &["check-ignore", "--quiet", "--", path])
            .code
        {
            0 => true,
            1 => false,
            code => panic!("git check-ignore {path} exited {code}"),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn a_fresh_install_is_already_current() {
    let fixture = Fixture::new();
    fixture.install(&[]);
    let before = files(&fixture.target);
    let report = fixture.sync(&[]);
    assert_eq!(report["edition"], "PHP Core", "{report}");
    assert_eq!(report["status"], "complete", "{report}");
    assert!(changed(&report).is_empty(), "{report}");
    assert_eq!(files(&fixture.target), before, "a current install changed");
    assert!(!fixture.project("ACCELERATOR.md").exists());

    // Without a README of its own the project got the edition's as
    // README.md: no sync writes it, and none adds ACCELERATOR.md.
    let readme = fs::read(fixture.project("README.md")).unwrap();
    fixture.release("README.md", |text| text + "\nRelease note.\n");
    let report = fixture.sync(&[]);
    assert_eq!(fs::read(fixture.project("README.md")).unwrap(), readme);
    assert!(!fixture.project("ACCELERATOR.md").exists());
    let reported = changed(&report)
        .into_keys()
        .chain(kept(&report).into_keys())
        .collect::<Vec<_>>();
    assert!(
        !reported
            .iter()
            .any(|path| path == "README.md" || path == "ACCELERATOR.md"),
        "{report}"
    );
}

#[test]
fn the_accelerator_readme_beside_the_projects_follows_the_release_and_is_never_added_back() {
    let fixture = Fixture::new();
    let readme = fixture.project("README.md");
    write(&readme, "# Client app\n");
    fixture.install(&["--merge-existing"]);
    let accelerator = fixture.project("ACCELERATOR.md");
    assert_eq!(
        fs::read(&accelerator).unwrap(),
        fs::read(fixture.source.join("PHP Core/README.md")).unwrap()
    );
    assert!(changed(&fixture.sync(&[])).is_empty());

    fixture.release("README.md", |text| text + "\nRelease note.\n");
    let report = fixture.sync(&[]);
    assert_eq!(
        changed(&report).get("ACCELERATOR.md").map(String::as_str),
        Some("updated"),
        "{report}"
    );
    assert_eq!(
        fs::read(&accelerator).unwrap(),
        fs::read(fixture.source.join("PHP Core/README.md")).unwrap()
    );
    assert_eq!(
        text(&readme),
        "# Client app\n",
        "the project's README was written"
    );
    assert!(!changed(&report).contains_key("README.md"), "{report}");

    // A team's edit stays.
    let edited = text(&accelerator) + "\nTeam note: deploys go through the release board.\n";
    fs::write(&accelerator, &edited).unwrap();
    fixture.release("README.md", |text| text + "\nAnother release note.\n");
    let report = fixture.sync(&[]);
    assert_eq!(
        kept(&report).get("ACCELERATOR.md").map(String::as_str),
        Some("edited in project"),
        "{report}"
    );
    assert_eq!(text(&accelerator), edited);

    // A removed copy is not added back.
    fs::remove_file(&accelerator).unwrap();
    fixture.release("README.md", |text| text + "\nA third release note.\n");
    let report = fixture.sync(&[]);
    assert!(!accelerator.exists());
    assert!(!changed(&report).contains_key("ACCELERATOR.md"), "{report}");
    assert!(!kept(&report).contains_key("ACCELERATOR.md"), "{report}");
    assert_eq!(text(&readme), "# Client app\n");
}

#[test]
fn a_folder_without_an_install_is_refused() {
    let fixture = Fixture::new();
    let empty = fixture.root.join("no install here");
    fs::create_dir(&empty).unwrap();
    let refused = fixture.sync_reply(&empty, &[]);
    assert_ne!(refused.code, 0, "{}", refused.stdout);
    assert!(
        fs::read_dir(&empty).unwrap().next().is_none(),
        "a refused sync wrote into the folder"
    );
}

#[test]
fn a_new_release_reaches_untouched_files_and_keeps_project_edits() {
    let fixture = Fixture::new();
    fixture.install(&[]);
    fixture.release(MEMORY_SKILL, |text| text + "\nNew release note.\n");
    fixture.release(CHECKPOINT_SKILL, |text| text + "\nAnother release note.\n");
    let edited = fixture.project(CHECKPOINT_SKILL);
    fs::write(&edited, text(&edited) + "\nTeam rule.\n").unwrap();

    let before = files(&fixture.target);
    let planned = fixture.sync(&["--dry-run"]);
    assert_eq!(planned["status"], "planned", "{planned}");
    assert_eq!(files(&fixture.target), before, "a dry run wrote");

    let report = fixture.sync(&[]);
    assert_eq!(report["status"], "complete", "{report}");
    assert_eq!(
        changed(&report),
        changed(&planned),
        "the dry run said otherwise"
    );
    assert_eq!(
        changed(&report).get(MEMORY_SKILL).map(String::as_str),
        Some("updated"),
        "{report}"
    );
    assert!(text(&fixture.project(MEMORY_SKILL)).contains("New release note."));
    assert_eq!(
        kept(&report).get(CHECKPOINT_SKILL).map(String::as_str),
        Some("edited in project"),
        "{report}"
    );
    let local = text(&edited);
    assert!(local.contains("Team rule.") && !local.contains("Another release note."));
    assert_eq!(
        fixture.record()["files"][MEMORY_SKILL],
        infra_core::query::sha256(&fs::read(fixture.project(MEMORY_SKILL)).unwrap()),
        "the record follows what the sync wrote"
    );
    let again = fixture.sync(&[]);
    assert!(changed(&again).is_empty(), "a second sync changed: {again}");
}

#[test]
fn tracked_files_wait_for_update_tracked_and_seeded_state_stays_the_projects() {
    let fixture = Fixture::new();
    fixture.install(&[]);
    fixture.git(&fixture.target, &["add", "--", MEMORY_SKILL]);
    let config = fixture.project("project-brain/config/runtime.json");
    fs::write(&config, "{\"mode\": \"governed\"}\n").unwrap();
    fixture.release(MEMORY_SKILL, |text| {
        text + "\nRelease edit of a tracked file.\n"
    });
    fixture.release("project-brain/config/runtime.json", |text| text + "\n");
    let tracked = text(&fixture.project(MEMORY_SKILL));

    let report = fixture.sync(&[]);
    let reasons = kept(&report);
    assert!(
        reasons
            .get(MEMORY_SKILL)
            .is_some_and(|reason| reason.contains("tracked by project")),
        "{report}"
    );
    assert_eq!(text(&fixture.project(MEMORY_SKILL)), tracked);
    assert_eq!(
        reasons
            .get("project-brain/config/runtime.json")
            .map(String::as_str),
        Some("seeded project-owned state"),
        "{report}"
    );

    let report = fixture.sync(&["--update-tracked"]);
    assert_eq!(
        changed(&report).get(MEMORY_SKILL).map(String::as_str),
        Some("updated"),
        "{report}"
    );
    assert!(text(&fixture.project(MEMORY_SKILL)).contains("Release edit of a tracked file."));
    assert_eq!(
        text(&config),
        "{\"mode\": \"governed\"}\n",
        "seeded state was written"
    );
}

#[test]
fn codex_hook_wiring_follows_the_release_only_with_rewire_codex() {
    let fixture = Fixture::new();
    fixture.install(&[]);
    let limit = |from: &str, to: &str| {
        let (from, to) = (
            format!("\"additionalContextLimit\": {from}"),
            format!("\"additionalContextLimit\": {to}"),
        );
        move |text: String| {
            assert!(text.contains(&from), "the release wiring changed shape");
            text.replacen(&from, &to, 1)
        }
    };
    fixture.release(WIRING, limit("6000", "7000"));
    let wiring = fixture.project(WIRING);
    let installed = fs::read(&wiring).unwrap();

    let report = fixture.sync(&[]);
    assert!(
        kept(&report)
            .get(WIRING)
            .is_some_and(|reason| reason.contains("rewire_codex")),
        "{report}"
    );
    assert_eq!(report["status"], "partial", "{report}");
    assert_eq!(report["requires_review"], true, "{report}");
    assert_eq!(
        fs::read(&wiring).unwrap(),
        installed,
        "trust-bound wiring changed"
    );

    let report = fixture.sync(&["--rewire-codex"]);
    assert_eq!(
        changed(&report).get(WIRING).map(String::as_str),
        Some("updated"),
        "{report}"
    );
    assert!(text(&wiring).contains("7000"));

    // A team's own edit stays, approval or not.
    let edited = text(&wiring).replacen("7000", "7500", 1);
    fs::write(&wiring, &edited).unwrap();
    fixture.release(WIRING, limit("7000", "8000"));
    let report = fixture.sync(&["--rewire-codex"]);
    assert_eq!(
        kept(&report).get(WIRING).map(String::as_str),
        Some("edited in project"),
        "{report}"
    );
    assert_eq!(text(&wiring), edited);
}

/// The downgrade check needs a commit to compare: a shallow clone lacks the
/// history, a packaged release and a record naming no commit have none.
#[test]
fn a_shallow_or_packaged_source_and_an_unnamed_commit_never_stop_the_sync() {
    let fixture = Fixture::new();
    fixture.install(&[]);
    fixture.release(MEMORY_SKILL, |text| text + "\nA fix from the clone.\n");
    fixture.sync(&[]);
    let recorded = fixture.record()["source_commit"]
        .as_str()
        .unwrap()
        .to_string();
    let skill = fixture.project(MEMORY_SKILL);

    let head = fixture.release(MEMORY_SKILL, |text| text + "\nA shallow note.\n");
    let shallow = fixture.root.join("shallow clone");
    let url = format!("file://{}", fixture.source.display());
    fixture.git(
        &fixture.root,
        &["clone", "--quiet", "--depth", "1", &url, "shallow clone"],
    );
    let missing = fixture.git_status(
        &shallow,
        &["cat-file", "-e", &format!("{recorded}^{{commit}}")],
    );
    assert_ne!(
        missing.code, 0,
        "the shallow clone must lack the recorded commit"
    );
    let report = fixture
        .sync_from(&shallow, &fixture.target, &[])
        .expect(0)
        .json();
    assert_eq!(
        changed(&report).get(MEMORY_SKILL).map(String::as_str),
        Some("updated"),
        "{report}"
    );
    assert!(text(&skill).contains("A shallow note."));
    assert_eq!(fixture.record()["source_commit"], head.as_str());

    let packaged = fixture.root.join("packaged release");
    package(&fixture.source, &packaged);
    let released = packaged.join("PHP Core").join(MEMORY_SKILL);
    fs::write(&released, text(&released) + "\nA packaged note.\n").unwrap();
    let report = fixture
        .sync_from(&packaged, &fixture.target, &[])
        .expect(0)
        .json();
    assert_eq!(
        changed(&report).get(MEMORY_SKILL).map(String::as_str),
        Some("updated"),
        "{report}"
    );
    assert!(text(&skill).contains("A packaged note."));
    assert_eq!(fixture.record()["source_commit"], Value::Null);

    fixture.release(MEMORY_SKILL, |text| {
        text + "\nA note after a damaged record.\n"
    });
    let mut record = fixture.record();
    record["source_commit"] = "not a commit".into();
    write(
        &fixture.project(RECORD),
        serde_json::to_vec_pretty(&record).unwrap(),
    );
    fixture.sync(&[]);
    assert!(text(&skill).contains("A note after a damaged record."));
}

#[test]
fn a_sync_never_takes_a_project_back_to_an_older_clone() {
    let fixture = Fixture::new();
    let older = fixture.git(&fixture.source, &["rev-parse", "HEAD"]);
    fixture.install(&[]);
    let newer = fixture.release(MEMORY_SKILL, |text| text + "\nA newer fix.\n");
    fixture.sync(&[]);
    assert_eq!(fixture.record()["source_commit"], newer.as_str());
    let skill = fixture.project(MEMORY_SKILL);
    assert!(text(&skill).contains("A newer fix."));

    fixture.git(&fixture.source, &["checkout", "--quiet", &older]);
    let before = files(&fixture.target);
    for extra in [&["--dry-run"][..], &[]] {
        let refused = fixture.sync_reply(&fixture.target, extra);
        assert_ne!(refused.code, 0, "{extra:?}: {}", refused.stdout);
        assert!(
            format!("{}{}", refused.stdout, refused.stderr).contains("downgrade"),
            "the refusal names its reason: {}{}",
            refused.stdout,
            refused.stderr
        );
        assert_eq!(files(&fixture.target), before, "a refused sync wrote");
    }
    // A person going back on purpose.
    let report = fixture.sync(&["--allow-downgrade"]);
    assert_eq!(
        changed(&report).get(MEMORY_SKILL).map(String::as_str),
        Some("updated"),
        "{report}"
    );
    assert!(!text(&skill).contains("A newer fix."));

    // A commit this clone has never seen stops it as well.
    fixture.git(&fixture.source, &["checkout", "--quiet", "main"]);
    let mut record = fixture.record();
    record["source_commit"] = "f".repeat(40).into();
    write(
        &fixture.project(RECORD),
        serde_json::to_vec_pretty(&record).unwrap(),
    );
    let before = files(&fixture.target);
    let refused = fixture.sync_reply(&fixture.target, &[]);
    assert_ne!(refused.code, 0, "{}", refused.stdout);
    assert_eq!(files(&fixture.target), before, "a refused sync wrote");
}

/// Installs a release before the native engine: its .gitignore names none.
/// Returns the current .gitignore, to publish as the next release.
fn install_before_the_engine(fixture: &Fixture) -> String {
    let current = text(&fixture.source.join("PHP Core/.gitignore"));
    fixture.release(".gitignore", |text| {
        text.lines()
            .filter(|line| !line.contains("infra-engine") && !line.contains("Platform-specific"))
            .map(|line| format!("{line}\n"))
            .collect()
    });
    fixture.install(&[]);
    assert!(
        !fixture.ignored(ENGINE),
        "the older release must not ignore the engine"
    );
    current
}

#[test]
fn the_engine_is_ignored_after_a_sync_of_an_install_with_an_older_gitignore() {
    let fixture = Fixture::new();
    let current = install_before_the_engine(&fixture);
    fixture.release(".gitignore", |_| current.clone());

    let report = fixture.sync(&[]);
    assert_eq!(
        changed(&report).get(".gitignore").map(String::as_str),
        Some("updated"),
        "an untouched copy follows the release: {report}"
    );
    assert_eq!(text(&fixture.project(".gitignore")), current);
    assert!(
        fixture.ignored(ENGINE),
        "after the sync the project's Git still offers the engine for commit: {report}"
    );
    let status = fixture.git(
        &fixture.target,
        &["status", "--porcelain", "--untracked-files=all"],
    );
    assert!(!status.contains(ENGINE), "{status}");
}

#[test]
fn an_edited_older_gitignore_receives_the_missing_entries_in_the_managed_block() {
    let fixture = Fixture::new();
    let current = install_before_the_engine(&fixture);
    let ignore = fixture.project(".gitignore");
    let edited = text(&ignore) + "/team-cache/\n";
    fs::write(&ignore, &edited).unwrap();
    fixture.release(".gitignore", |_| current);

    let report = fixture.sync(&[]);
    assert_eq!(
        changed(&report).get(".gitignore").map(String::as_str),
        Some("managed entries added"),
        "{report}"
    );
    let synced = text(&ignore);
    assert!(
        synced.starts_with(&edited),
        "every line of the project's file stays first: {synced}"
    );
    let block = synced
        .split_once("# BEGIN ACCELERATOR MANAGED ENTRIES\n")
        .and_then(|(_, rest)| rest.split_once("# END ACCELERATOR MANAGED ENTRIES\n"))
        .map(|(block, _)| block)
        .expect("one managed block");
    assert!(block.contains("memory-bank/bin/infra-engine\n"), "{synced}");
    assert!(!block.contains("memory-bank/local/"), "{synced}");
    assert!(fixture.ignored(ENGINE), "{synced}");
    assert!(fixture.ignored("team-cache/output.log"), "{synced}");
    let again = fixture.sync(&[]);
    assert!(!changed(&again).contains_key(".gitignore"), "{again}");
}

#[test]
fn the_managed_ignore_block_follows_the_release_and_keeps_project_lines() {
    let fixture = Fixture::new();
    let ignore = fixture.project(".gitignore");
    write(&ignore, "/client-build/\n");
    fixture.install(&["--merge-existing"]);
    let merged = text(&ignore);
    assert!(merged.starts_with("/client-build/\n"), "{merged}");
    assert!(
        merged.contains("# BEGIN ACCELERATOR MANAGED ENTRIES\n"),
        "{merged}"
    );
    assert!(fixture.ignored(ENGINE));
    // The team adds its own line after the block.
    fs::write(&ignore, format!("{merged}/team-cache/\n")).unwrap();
    fixture.release(".gitignore", |text| text + "/release-two-cache/\n");

    let report = fixture.sync(&[]);
    assert_eq!(
        changed(&report).get(".gitignore").map(String::as_str),
        Some("managed entries updated"),
        "{report}"
    );
    let synced = text(&ignore);
    assert!(synced.starts_with("/client-build/\n"), "{synced}");
    assert!(synced.contains("/team-cache/\n"), "{synced}");
    let block = synced
        .split_once("# BEGIN ACCELERATOR MANAGED ENTRIES\n")
        .and_then(|(_, rest)| rest.split_once("# END ACCELERATOR MANAGED ENTRIES\n"))
        .map(|(block, _)| block)
        .expect("one managed block");
    assert!(block.contains("/release-two-cache/\n"), "{synced}");
    assert_eq!(
        synced
            .matches("# BEGIN ACCELERATOR MANAGED ENTRIES")
            .count(),
        1
    );
    assert!(fixture.ignored(ENGINE));
    assert!(fixture.ignored("release-two-cache/output.log"));
    assert!(fixture.ignored("team-cache/output.log"));
    let again = fixture.sync(&[]);
    assert!(!changed(&again).contains_key(".gitignore"), "{again}");
}

#[test]
fn a_sync_writes_nothing_through_a_linked_folder() {
    let fixture = Fixture::new();
    fixture.install(&[]);
    fixture.release(MEMORY_SKILL, |text| text + "\nRelease through a link.\n");
    for linked in [".agents", "memory-bank/local"] {
        let folder = fixture.project(linked);
        let outside = fixture
            .root
            .join(format!("outside {}", linked.replace('/', " ")));
        fs::rename(&folder, &outside).unwrap();
        std::os::unix::fs::symlink(&outside, &folder).unwrap();
        let before = files(&outside);
        for extra in [&["--dry-run"][..], &[]] {
            let reply = fixture.sync_reply(&fixture.target, extra);
            assert_eq!(
                files(&outside),
                before,
                "{linked} {extra:?} wrote outside the project: {}{}",
                reply.stdout,
                reply.stderr
            );
            if reply.code == 0 {
                let report = reply.json();
                assert!(
                    changed(&report)
                        .keys()
                        .all(|path| !path.starts_with(linked)),
                    "{report}"
                );
            }
        }
        fs::remove_file(&folder).unwrap();
        fs::rename(&outside, &folder).unwrap();
    }
}
