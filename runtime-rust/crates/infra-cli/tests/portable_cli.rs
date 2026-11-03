//! Public CLI contracts from an installed executable with no source checkout.
//! Every subprocess uses a private working directory and child-only environment
//! removal; the parent process environment and working directory are untouched.
#[path = "support/executable.rs"]
mod executable;
use infra_cli::{arguments, counters, shell};
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn counter_names_keep_standard_md5_digests_including_multiblock_input() {
    for (input, expected) in [
        ("", "d41d8cd98f00b204e9800998ecf8427e"),
        ("a", "0cc175b9c0f1b6a831c399e269772661"),
        ("abc", "900150983cd24fb0d6963f7d28e17f72"),
        ("message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
        (
            "abcdefghijklmnopqrstuvwxyz",
            "c3fcd3d76192e4007dfb496cca67e13b",
        ),
        (
            "12345678901234567890123456789012345678901234567890123456789012345678901234567890",
            "57edf4a22be3c955ac49da2e2107b67a",
        ),
    ] {
        assert_eq!(counters::md5(input.as_bytes()), expected, "{input:?}");
    }
}

struct InstalledCli {
    root: PathBuf,
    work: PathBuf,
    binary: PathBuf,
}

impl InstalledCli {
    fn new() -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system calendar")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "infra-cli-portable-{}-{nonce}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("create private test root");
        let root = fs::canonicalize(root).expect("resolve private test root");
        assert!(
            !root
                .ancestors()
                .any(|parent| parent.join("runtime-rust/Cargo.toml").is_file()),
            "portable CLI fixture must be outside the source checkout"
        );
        let bin = root.join("standalone Creator/memory-bank/bin");
        let work = root.join("project with spaces");
        fs::create_dir_all(&bin).expect("create installed executable directory");
        fs::create_dir(&work).expect("create independent project directory");
        let binary = bin.join(if cfg!(windows) {
            "infra-engine.exe"
        } else {
            "infra-engine"
        });
        executable::copy(env!("CARGO_BIN_EXE_infra-engine"), &binary);
        Self { root, work, binary }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(&self.binary)
            .args(args)
            .current_dir(&self.work)
            .env_remove("INFRA_SOURCE_ROOT")
            .env_remove("ACCELERATOR_HOME")
            .env_remove("ACCELERATOR_STATE_DIR")
            .env_remove("ACCELERATOR_PROJECT_DIR")
            .stdin(Stdio::null())
            .output()
            .unwrap_or_else(|error| panic!("could not invoke {args:?}: {error}"))
    }

    fn expect(&self, args: &[&str], code: i32) -> Output {
        let output = self.run(args);
        assert_eq!(
            output.status.code(),
            Some(code),
            "{args:?}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
}

impl Drop for InstalledCli {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn release_contract(cli: &InstalledCli) {
    fs::write(
        cli.work.join("commit list.txt"),
        "abc1234 feat(cli): make paths portable\nbcd2345 fix: preserve spaces\ncde3456 docs: explain setup\n",
    )
    .expect("write relative release input");
    let args = ["release", "categorize", "commit list.txt"];
    let markdown = cli.expect(&args, 0);
    let expected = "\n### Added\n\n- Make paths portable (abc1234)\n\n### Fixed\n\n- Preserve spaces (bcd2345)\n\n### Documentation\n\n- Explain setup (cde3456)\n";
    assert_eq!(String::from_utf8(markdown.stdout).unwrap(), expected);
    assert!(markdown.stderr.is_empty());

    let report = cli.expect(&["release", "categorize", "commit list.txt", "--json"], 0);
    let report = json(&report);
    assert_eq!(report["markdown"], expected);
    assert_eq!(report["commits"].as_array().unwrap().len(), 3);
    assert_eq!(report["commits"][0]["hash"], "abc1234");
    assert_eq!(
        report["categories"]["Fixed"][0]["message"],
        "fix: preserve spaces"
    );

    cli.expect(&["release", "categorize"], 2);
    cli.expect(&["release", "categorize", "--unknown-option"], 2);
    let missing = cli.expect(&["release", "categorize", "absent.txt"], 1);
    assert!(missing.stdout.is_empty());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("absent"));
    fs::write(cli.work.join("invalid.txt"), [0xff, 0xfe]).unwrap();
    cli.expect(&["release", "categorize", "invalid.txt"], 2);
}

fn skill_contract(cli: &InstalledCli) {
    cli.expect(
        &[
            "skill-tools",
            "init-skill",
            "portable-skill",
            "--path",
            "skills",
        ],
        0,
    );
    let skill = cli.work.join("skills/portable-skill");
    assert!(skill.join("SKILL.md").is_file());
    assert!(skill.join("scripts/example.rs").is_file());
    assert!(!skill.join("scripts/example.py").exists());

    // The initializer deliberately leaves a TODO description for the author.
    // Supplying a complete description makes this a real validation/package flow.
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: portable-skill\ndescription: Validate a standalone skill from a consuming project.\nphase: utility\n---\n\n# Portable skill\n",
    )
    .unwrap();
    let valid = cli.expect(
        &["skill-tools", "quick-validate", "skills/portable-skill"],
        0,
    );
    assert_eq!(valid.stdout, b"Skill is valid!\n");
    cli.expect(
        &[
            "skill-tools",
            "package-skill",
            "skills/portable-skill",
            "packages",
        ],
        0,
    );
    let archive = fs::read(cli.work.join("packages/portable-skill.skill")).unwrap();
    assert!(
        archive.starts_with(b"PK\x03\x04"),
        "package must be a ZIP image"
    );
    assert!(
        archive
            .windows(b"portable-skill/SKILL.md".len())
            .any(|window| window == b"portable-skill/SKILL.md"),
        "the archive must contain the selected skill"
    );
    assert!(
        archive.windows(4).any(|window| window == b"PK\x05\x06"),
        "the archive must contain its central-directory terminator"
    );
    cli.expect(
        &[
            "skill-tools",
            "init-skill",
            "portable-skill",
            "--path",
            "skills",
        ],
        1,
    );
    fs::write(skill.join("SKILL.md"), "# Missing frontmatter\n").unwrap();
    let invalid = cli.expect(
        &["skill-tools", "quick-validate", "skills/portable-skill"],
        1,
    );
    assert_eq!(invalid.stdout, b"No YAML frontmatter found\n");
    cli.expect(
        &[
            "skill-tools",
            "package-skill",
            "skills/portable-skill",
            "rejected packages",
        ],
        1,
    );
    assert!(
        !cli.work
            .join("rejected packages/portable-skill.skill")
            .exists()
    );
}

fn creator_contract(cli: &InstalledCli) {
    let schema = json(&cli.expect(&["creator", "hook-policy-schema"], 0));
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["schema_version"]["const"], 1);
    assert!(
        schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field == "evidence")
    );
    let contract = json(&cli.expect(&["creator", "runtime-contract"], 0));
    assert_eq!(contract["contract_id"], "context-brain-native-runtime");
    assert!(
        contract["authority"]["canonical_sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry == "memory-bank/bin/infra-engine")
    );

    // This is a standalone executable, not an approved source of a candidate registry.
    // Operations that require one must request it explicitly rather than trusting cwd.
    for args in [
        vec!["creator", "validate-generated", "--target", "."],
        vec![
            "creator",
            "validate-skill-quality",
            "--plan",
            "missing-plan.json",
            "--target",
            ".",
            "--plan-only",
        ],
        vec![
            "creator",
            "validate-plan-mutations",
            "--plan",
            "missing-plan.json",
            "--target",
            ".",
        ],
    ] {
        let output = cli.expect(&args, 2);
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("generator_root"), "{args:?}: {error}");
        assert!(!error.contains("Repository tooling root unavailable"));
    }
    assert!(!cli.work.join("project-brain").exists());
    assert!(!cli.work.join("memory-bank").exists());
}

#[test]
fn copied_engine_runs_portable_helpers_from_a_consuming_project() {
    let cli = InstalledCli::new();
    release_contract(&cli);
    skill_contract(&cli);
    creator_contract(&cli);
}

#[test]
fn public_helper_options_preserve_paths_repetition_and_boolean_flags() {
    let viewer = arguments::parse(
        &strings(&["run with spaces", "--ui-assets", "compiled UI with spaces"]),
        "skill-tools/generate-review",
    )
    .unwrap();
    assert_eq!(viewer.value["ui_assets"], "compiled UI with spaces");
    assert_eq!(
        viewer.value["_positionals"],
        serde_json::json!(["run with spaces"])
    );

    let args = arguments::parse(
        &strings(&[
            "--assets",
            "assets with spaces",
            "--staging",
            "output",
            "--framework",
            "laminas",
            "--editions",
            "claude,codex",
            "--engine",
            "native engine",
        ]),
        "creator/seed-assets",
    )
    .unwrap();
    assert_eq!(args.value["assets"], "assets with spaces");
    assert_eq!(args.value["framework"], "laminas");
    assert_eq!(args.value["engine"], "native engine");

    let tools = arguments::parse(
        &strings(&[
            "--source",
            "source",
            "--staging",
            "stage",
            "--edition",
            "PHP Core",
            "--tool",
            "claude",
            "--tool",
            "codex",
        ]),
        "creator/seed-runtime",
    )
    .unwrap();
    assert_eq!(tools.value["tool"], serde_json::json!(["claude", "codex"]));

    let release = arguments::parse(
        &strings(&["commit list.txt", "--json"]),
        "release/categorize",
    )
    .unwrap();
    assert_eq!(
        release.value["_positionals"],
        serde_json::json!(["commit list.txt"])
    );
    assert_eq!(release.value["json"], true);
    assert!(arguments::parse(&strings(&["--invented"]), "release/categorize").is_err());
}

#[test]
fn memory_parser_keeps_required_identity_and_repeated_source_paths() {
    let args = arguments::memory(&strings(&[
        "--root",
        "state",
        "retrieve",
        "current task",
        "--task-id",
        "TASK-1",
        "--path",
        "src/one.rs",
        "--path",
        "src/two.rs",
        "--trace-path",
        "project-brain/control/retrieval-manifests/one.json",
        "--json",
    ]))
    .unwrap();
    assert_eq!(args.operation, "retrieve");
    assert_eq!(args.value["root"], "state");
    assert_eq!(args.value["query"], "current task");
    assert_eq!(args.value["task_id"], "TASK-1");
    assert_eq!(
        args.value["paths"],
        serde_json::json!(["src/one.rs", "src/two.rs"])
    );
    assert_eq!(args.value["host"], "cli");
    assert_eq!(args.value["json"], true);
    assert!(arguments::memory(&strings(&["retrieve", "query"])).is_err());
    assert!(
        arguments::memory(&strings(&[
            "retrieve",
            "query",
            "--task-id",
            "TASK-1",
            "--host",
            "unknown",
        ]))
        .is_err()
    );
}

#[test]
fn native_shell_guard_distinguishes_executable_commands_from_search_text() {
    let policy = shell::Policy::for_framework("core");
    for command in [
        "git reset --hard",
        "git push origin --force-with-lease",
        "bash -c 'git reset --hard'",
        "ssh example.invalid sh -c 'git reset --hard'",
    ] {
        assert!(
            shell::validate(command, &policy).unwrap().is_some(),
            "unsafe command was admitted: {command}"
        );
    }
    for command in [
        "git status --porcelain",
        "git show HEAD:notes.txt",
        "git commit -m 'document --no-verify flag'",
        "rg -n 'DROP TABLE users|git reset --hard' README.md",
        "printf '%s' 'git reset --hard'",
    ] {
        assert_eq!(
            shell::validate(command, &policy).unwrap(),
            None,
            "read-only or literal command was rejected: {command}"
        );
    }
    let laravel = shell::Policy::for_framework("laravel");
    assert!(
        shell::validate("php artisan migrate:fresh", &laravel)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        shell::validate("php artisan migrate:status", &laravel).unwrap(),
        None
    );
    assert!(shell::polling("sleep 1 && git status --porcelain"));
    assert!(!shell::polling("git status && git reset --hard"));
}

#[test]
fn public_harness_help_is_pure_and_desktop_mutators_reject_unknown_options() {
    let cli = InstalledCli::new();
    let home = cli.root.join("private home");
    let missing_source = cli.root.join("absent source");
    let invoke = |args: &[&str], source: Option<&std::path::Path>| {
        let mut command = Command::new(&cli.binary);
        command
            .args(args)
            .current_dir(&cli.work)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("CODEX_HOME", home.join("codex"))
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("XDG_DATA_HOME", home.join("data"))
            .env("XDG_STATE_HOME", home.join("state"))
            .env("APPDATA", home.join("appdata"))
            .env("LOCALAPPDATA", home.join("localappdata"))
            .env_remove("INFRA_SOURCE_ROOT")
            .env_remove("ACCELERATOR_HOME")
            .env_remove("ACCELERATOR_STATE_DIR")
            .env_remove("ACCELERATOR_PROJECT_DIR")
            .stdin(Stdio::null());
        if let Some(source) = source {
            command.arg("--source").arg(source);
        }
        command.output().expect("invoke isolated public command")
    };
    let cases: &[(&[&str], bool)] = &[
        (&["harness", "--help"], false),
        (&["harness", "run", "--help"], false),
        (&["harness", "resume", "-h"], false),
        (&["serve", "--help"], false),
        (&["serve", "status", "--help"], true),
        (&["app", "--help"], false),
        (&["app", "install", "--help"], true),
        (&["app", "uninstall", "-h"], true),
    ];
    for (args, explicit_source) in cases {
        let output = invoke(args, explicit_source.then_some(missing_source.as_path()));
        assert_eq!(
            output.status.code(),
            Some(0),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .contains(&format!("Usage: infra-engine {}", args[0])),
            "missing public usage for {args:?}"
        );
        assert!(
            output.stderr.is_empty(),
            "help emitted an error for {args:?}"
        );
    }
    // A source-shaped fixture proves validation precedes source discovery.
    // On Windows, use only an absent source: HKCU is not isolated by HOME.
    #[cfg(not(windows))]
    let valid_source = {
        let source = cli.root.join("fixture source");
        fs::create_dir_all(source.join("harness/web")).unwrap();
        fs::create_dir_all(source.join("Infrastructure-Creator")).unwrap();
        fs::write(source.join("harness/web/index.html"), "fixture").unwrap();
        fs::write(source.join("Infrastructure-Creator/AGENTS.md"), "fixture").unwrap();
        source
    };
    #[cfg(not(windows))]
    let mutation_sources = [&missing_source, &valid_source];
    #[cfg(windows)]
    let mutation_sources = [&missing_source];
    for source in mutation_sources {
        for action in ["install", "uninstall"] {
            for flag in ["--unknown", "--dry-run"] {
                let output = invoke(&["app", action, flag], Some(source));
                assert_eq!(output.status.code(), Some(2), "{action} {flag}");
                assert!(
                    String::from_utf8_lossy(&output.stderr).contains("Unknown application option"),
                    "mutator reached discovery or an action before rejecting {flag}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
    for args in [
        &["internal-sandbox-child", "--help"][..],
        &["internal-discovery-probe", "--help"][..],
        &["harness", "run", "--", "--help"][..],
    ] {
        let output = invoke(args, None);
        assert_eq!(
            output.status.code(),
            Some(2),
            "nonpublic help was intercepted: {args:?}"
        );
    }
    assert!(
        !home.exists(),
        "help or rejected options created user configuration/state"
    );
    assert!(!missing_source.exists(), "a missing source was created");
    assert!(!cli.work.join("state").exists(), "Fleet state was created");
    assert!(
        !cli.work.join("reports").exists(),
        "Fleet reports were created"
    );
}

fn generate_report_with_stdin(cli: &InstalledCli, output_name: &str, oversized: bool) -> Output {
    use std::{
        io::{self, Read, Write},
        thread,
        time::{Duration, Instant},
    };
    const INPUT_LIMIT: usize = 32 * 1024 * 1024;
    const CAPTURE_LIMIT: u64 = 64 * 1024;
    let scratch = cli.root.join(format!("report-stdin-{output_name}"));
    fs::create_dir(&scratch).expect("create private report-input root");
    let home = scratch.join("home");
    let temporary = scratch.join("tmp");
    fs::create_dir(&home).unwrap();
    fs::create_dir(&temporary).unwrap();
    let mut child = Command::new(&cli.binary)
        .args([
            "skill-tools",
            "generate-report",
            "-",
            "--output",
            output_name,
        ])
        .current_dir(&cli.work)
        .env_remove("INFRA_SOURCE_ROOT")
        .env_remove("ACCELERATOR_HOME")
        .env_remove("ACCELERATOR_STATE_DIR")
        .env_remove("ACCELERATOR_PROJECT_DIR")
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("CODEX_HOME", home.join("codex"))
        .env("CLAUDE_CONFIG_DIR", home.join("claude"))
        .env("TMPDIR", &temporary)
        .env("TMP", &temporary)
        .env("TEMP", &temporary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start installed report command");
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let writer = thread::spawn(move || -> io::Result<()> {
        let prefix = br#"{"history":[]}"#;
        stdin.write_all(prefix)?;
        if oversized {
            // The first LIMIT bytes are a complete JSON value plus whitespace.
            // A capped reader must detect overflow, not accept this valid prefix.
            let padding = [b' '; 64 * 1024];
            let mut remaining = INPUT_LIMIT - prefix.len();
            while remaining > 0 {
                let count = remaining.min(padding.len());
                stdin.write_all(&padding[..count])?;
                remaining -= count;
            }
            stdin.write_all(b"trailing-json-garbage")?;
        }
        Ok(())
    });
    let capture = |reader: Box<dyn Read + Send>| {
        thread::spawn(move || -> io::Result<Vec<u8>> {
            let mut bytes = Vec::new();
            reader.take(CAPTURE_LIMIT + 1).read_to_end(&mut bytes)?;
            Ok(bytes)
        })
    };
    let stdout_reader = capture(Box::new(stdout));
    let stderr_reader = capture(Box::new(stderr));
    let deadline = Instant::now() + Duration::from_secs(30);
    let (status, timed_out) = loop {
        if let Some(status) = child.try_wait().expect("poll report command") {
            break (status, false);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            break (child.wait().expect("reap timed-out report command"), true);
        }
        thread::sleep(Duration::from_millis(10));
    };
    let written = writer.join().expect("report stdin writer panicked");
    if let Err(error) = written {
        assert!(
            oversized && error.kind() == io::ErrorKind::BrokenPipe,
            "unexpected report input failure: {error}"
        );
    }
    let stdout = stdout_reader
        .join()
        .unwrap()
        .expect("capture report stdout");
    let stderr = stderr_reader
        .join()
        .unwrap()
        .expect("capture report stderr");
    assert!(
        !timed_out,
        "report command exceeded its native test deadline"
    );
    assert!(
        stdout.len() <= CAPTURE_LIMIT as usize,
        "report stdout exceeded capture limit"
    );
    assert!(
        stderr.len() <= CAPTURE_LIMIT as usize,
        "report stderr exceeded capture limit"
    );
    Output {
        status,
        stdout,
        stderr,
    }
}

#[test]
fn generate_report_refuses_oversized_stdin_instead_of_accepting_a_json_prefix() {
    let cli = InstalledCli::new();
    let positive = generate_report_with_stdin(&cli, "small-report.html", false);
    assert!(
        positive.status.success(),
        "small JSON refused: {}",
        String::from_utf8_lossy(&positive.stderr)
    );
    assert!(positive.stdout.is_empty());
    let positive_path = cli.work.join("small-report.html");
    assert!(fs::metadata(&positive_path).unwrap().len() < 64 * 1024);
    assert!(
        fs::read_to_string(&positive_path)
            .unwrap()
            .starts_with("<!DOCTYPE html>")
    );

    let refused = generate_report_with_stdin(&cli, "oversized-report.html", true);
    assert_eq!(
        refused.status.code(),
        Some(2),
        "oversized JSON prefix was accepted; stdout: {}; stderr: {}",
        String::from_utf8_lossy(&refused.stdout),
        String::from_utf8_lossy(&refused.stderr)
    );
    assert!(refused.stdout.is_empty(), "refusal emitted a report");
    assert!(
        String::from_utf8_lossy(&refused.stderr)
            .contains("Standard input exceeds the 32 MiB JSON limit")
    );
    assert!(
        !cli.work.join("oversized-report.html").exists(),
        "oversized input created a partial or complete output report"
    );
}
