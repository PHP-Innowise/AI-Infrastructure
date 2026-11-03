//! Public provider-free evaluation must admit an explicitly selected Cargo image.
//! All Git repositories, engine hardlinks, homes and result files are private.
use infra_eval::process::{Budget, CommandSpec};

#[path = "support/eval_process_driver.rs"]
mod eval_process_driver;
#[path = "support/executable.rs"]
mod executable;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const EXECUTABLE_LIMIT: usize = 512 * 1024 * 1024;
const OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
const ENGINE_NAME: &str = if cfg!(windows) {
    "infra-engine.exe"
} else {
    "infra-engine"
};

struct Fixture {
    root: PathBuf,
    env: BTreeMap<String, String>,
}
impl Fixture {
    fn new() -> Self {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "infra-eval-native-runtime-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&root).unwrap();
        private_mode(&root, 0o700);
        let root = fs::canonicalize(root).unwrap();
        let mut env = std::env::vars()
            .filter(|(name, _)| {
                [
                    "PATH",
                    "LANG",
                    "LC_ALL",
                    "LC_CTYPE",
                    "SYSTEMROOT",
                    "SYSTEMDRIVE",
                    "COMSPEC",
                    "PATHEXT",
                    "WINDIR",
                ]
                .contains(&name.to_ascii_uppercase().as_str())
            })
            .collect::<BTreeMap<_, _>>();
        for folder in [
            "home",
            "tmp",
            "codex-home",
            "claude-home",
            "templates",
            "selected",
        ] {
            fs::create_dir(root.join(folder)).unwrap();
            private_mode(&root.join(folder), 0o700);
        }
        for (name, relative) in [
            ("HOME", "home"),
            ("USERPROFILE", "home"),
            ("TMPDIR", "tmp"),
            ("TMP", "tmp"),
            ("TEMP", "tmp"),
            ("CODEX_HOME", "codex-home"),
            ("CLAUDE_CONFIG_DIR", "claude-home"),
            ("XDG_CONFIG_HOME", "home/config"),
            ("XDG_DATA_HOME", "home/data"),
            ("XDG_STATE_HOME", "home/state"),
            ("APPDATA", "home/appdata"),
            ("LOCALAPPDATA", "home/localappdata"),
        ] {
            let path = root.join(relative);
            fs::create_dir_all(&path).unwrap();
            env.insert(name.into(), path.display().to_string());
        }
        fs::write(root.join("home/gitconfig"), b"").unwrap();
        env.insert(
            "GIT_CONFIG_GLOBAL".into(),
            root.join("home/gitconfig").display().to_string(),
        );
        env.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
        env.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
        env.insert("GIT_NO_LAZY_FETCH".into(), "1".into());
        env.insert("GIT_AUTHOR_DATE".into(), "2026-08-01T12:00:00Z".into());
        env.insert("GIT_COMMITTER_DATE".into(), "2026-08-01T12:00:00Z".into());
        env.insert("TZ".into(), "UTC".into());
        Self { root, env }
    }
    fn run(&self, cwd: &Path, argv: Vec<String>, seconds: u64) -> (i32, Vec<u8>, Vec<u8>) {
        self.run_with_env(cwd, argv, seconds, &self.env)
    }
    fn run_with_env(
        &self,
        cwd: &Path,
        argv: Vec<String>,
        seconds: u64,
        env: &BTreeMap<String, String>,
    ) -> (i32, Vec<u8>, Vec<u8>) {
        static CALL: AtomicU64 = AtomicU64::new(0);
        let call = CALL.fetch_add(1, Ordering::Relaxed);
        let stdout = self.root.join(format!("command-{call}.stdout"));
        let stderr = self.root.join(format!("command-{call}.stderr"));
        let request = self.root.join(format!("command-{call}.request.json"));
        let result = eval_process_driver::execute(
            Path::new(env!("CARGO_BIN_EXE_infra-cli-eval-process-fixture")),
            &CommandSpec {
                argv: argv.clone(),
                cwd: cwd.into(),
                env: env.clone(),
                input: vec![],
            },
            &Budget {
                timeout: Duration::from_secs(seconds),
                cap: usize::MAX,
                max_bytes: OUTPUT_LIMIT,
                native: false,
                strict_events: false,
                error_diagnostics_terminal: false,
            },
            &request,
            &stdout,
            &stderr,
        );
        assert_eq!(
            result["stopped"], "finished",
            "private command exceeded its {seconds}s/output bound: {argv:?}"
        );
        (
            i32::try_from(result["exit"].as_i64().unwrap_or(-1)).unwrap(),
            bounded(&stdout, OUTPUT_LIMIT),
            bounded(&stderr, OUTPUT_LIMIT),
        )
    }
    fn git(&self, cwd: &Path, args: &[&str]) {
        let mut argv = vec![
            "git".into(),
            "--no-optional-locks".into(),
            "--no-pager".into(),
            "-c".into(),
            format!("core.hooksPath={}", self.root.join("templates").display()),
            "-c".into(),
            format!("init.templateDir={}", self.root.join("templates").display()),
            "-c".into(),
            "commit.gpgsign=false".into(),
            "-c".into(),
            "user.name=Native Runtime Fixture".into(),
            "-c".into(),
            "user.email=native-runtime@example.invalid".into(),
        ];
        argv.extend(args.iter().map(|argument| argument.to_string()));
        let (code, stdout, stderr) = self.run(cwd, argv, 30);
        assert_eq!(
            code,
            0,
            "private Git {args:?}: {}\n{}",
            String::from_utf8_lossy(&stdout),
            String::from_utf8_lossy(&stderr)
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn private_mode(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
}
fn bounded(path: &Path, limit: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    File::open(path)
        .unwrap()
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .unwrap();
    assert!(
        bytes.len() <= limit,
        "fixture input exceeded its explicit bound"
    );
    bytes
}
fn digest(path: &Path) -> String {
    infra_core::query::sha256(&bounded(path, EXECUTABLE_LIMIT))
}
fn source_image(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, at: &Path, total: &mut usize, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(at).unwrap() {
            let path = entry.unwrap().path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            assert!(!infra_core::fs::linked(&metadata));
            if metadata.is_dir() {
                walk(root, &path, total, out);
            } else {
                assert!(metadata.is_file());
                let bytes = bounded(&path, 4 * 1024 * 1024);
                *total += bytes.len();
                assert!(*total <= 16 * 1024 * 1024);
                out.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .replace('\\', "/"),
                    bytes,
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    let mut total = 0;
    walk(root, root, &mut total, &mut out);
    out
}

#[test]
fn public_memory_eval_accepts_hardlinked_selected_native_runtime_without_a_provider() {
    let fixture = Fixture::new();
    let original_engine = Path::new(env!("CARGO_BIN_EXE_infra-engine"));
    let repository =
        fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")).unwrap();
    let selected = fixture.root.join("selected").join(ENGINE_NAME);
    let second = fixture.root.join("selected/second-engine-image");
    executable::copy(original_engine, &selected);
    private_mode(&selected, 0o700);
    fs::hard_link(&selected, &second).unwrap();
    let selected_sha = digest(&selected);
    let original_sha = digest(original_engine);
    assert_eq!(selected_sha, original_sha);
    let mut selected_parent =
        infra_core::bank::publication::PublicationFiles::new(selected.parent().unwrap()).unwrap();
    let (_, selected_identity) = selected_parent
        .capture_source(
            selected.file_name().unwrap().to_str().unwrap(),
            EXECUTABLE_LIMIT,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        selected_identity.3, 2,
        "fixture must contain the actual hardlinked image"
    );
    #[cfg(unix)]
    assert!(
        infra_eval::artifact::read_file(&selected, EXECUTABLE_LIMIT).is_err(),
        "ordinary data-file admission must keep its one-link requirement"
    );

    let project = fixture.root.join("projects/fixture");
    fs::create_dir_all(project.join("specs")).unwrap();
    fs::write(
        project.join("specs/invoices.md"),
        b"# Invoice numbers\nInvoices use monotonic invoice numbers.\n",
    )
    .unwrap();
    fixture.git(&project, &["init", "--quiet"]);
    fixture.git(&project, &["add", "--", "specs/invoices.md"]);
    fixture.git(
        &project,
        &["commit", "--quiet", "--no-verify", "-m", "fixture source"],
    );
    let original_project = source_image(&project);
    let set = fixture.root.join("set.json");
    let grades = fixture.root.join("judgments.json");
    fs::write(
        &set,
        serde_json::to_vec(&json!([{
            "id":"hardlinked", "project":"fixture", "ts":"2026-08-05T12:00:00Z",
            "prompt":"How are invoices numbered?"
        }]))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        &grades,
        serde_json::to_vec(&json!({"hardlinked":{"specs/invoices.md":2}})).unwrap(),
    )
    .unwrap();
    private_mode(&set, 0o600);
    private_mode(&grades, 0o600);
    let result_path = fixture.root.join("result.json");
    let cache = fixture.root.join("cache");
    let argv = vec![
        selected.display().to_string(),
        "eval".into(),
        "memory-eval".into(),
        "run".into(),
        "--tooling-root".into(),
        repository.display().to_string(),
        "--edition".into(),
        "PHP Core".into(),
        "--host".into(),
        "codex".into(),
        "--projects-root".into(),
        fixture.root.join("projects").display().to_string(),
        "--set".into(),
        set.display().to_string(),
        "--judgments".into(),
        grades.display().to_string(),
        "--cache".into(),
        cache.display().to_string(),
        "--out".into(),
        result_path.display().to_string(),
        "--as-of".into(),
        "now".into(),
        "--clock".into(),
        "real".into(),
        "--timeout".into(),
        "30".into(),
        "--keep".into(),
    ];
    // No runtime-binary override: the public CLI's default current_exe is our
    // two-link native engine. This controller invokes only local index/refresh.
    let (code, stdout, stderr) = fixture.run(&fixture.root, argv, 120);
    let result: Value = serde_json::from_slice(&bounded(&result_path, 64 * 1024 * 1024)).unwrap();
    if code != 0 {
        eprintln!(
            "synthetic evaluation decision: {}",
            json!({
                "status": result["items"]["hardlinked"]["status"],
                "reason": result["items"]["hardlinked"]["reason"],
                "refresh": result["items"]["hardlinked"]["refresh"],
                "admitted": result["planned"]["units"]["hardlinked"]["runtime_admitted"],
                "evaluated": result["summary"]["evaluated"],
                "runtime_sha256": result["meta"]["editions"]["PHP Core"]["runtime_sha256"],
            })
        );
        let corpus = PathBuf::from(
            result["items"]["hardlinked"]["corpus"]
                .as_str()
                .expect("--keep must retain the failed synthetic corpus"),
        );
        assert!(
            corpus.starts_with(&cache),
            "diagnostic corpus must belong to the owned cache"
        );
        let stamp = chrono::DateTime::parse_from_rfc3339("2026-08-05T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let env = infra_eval::reconstruction::runtime_env(&cache, stamp, false).unwrap();
        let (branch_code, branch, branch_error) = fixture.run_with_env(
            &corpus,
            vec![
                "git".into(),
                "-c".into(),
                "core.fsmonitor=false".into(),
                "-c".into(),
                format!(
                    "core.hooksPath={}",
                    fixture.root.join("templates").display()
                ),
                "branch".into(),
                "--show-current".into(),
            ],
            30,
            &env,
        );
        assert_eq!(
            branch_code,
            0,
            "synthetic branch lookup: {}",
            String::from_utf8_lossy(&branch_error)
        );
        let branch = String::from_utf8(branch).unwrap().trim().to_string();
        for (label, arguments) in [
            ("index", vec!["index".to_string(), "--json".into()]),
            (
                "refresh",
                vec![
                    "refresh".into(),
                    "--host".into(),
                    "codex".into(),
                    "--query".into(),
                    "How are invoices numbered?".into(),
                    "--task-id".into(),
                    branch,
                    "--limit".into(),
                    "3".into(),
                    "--ephemeral".into(),
                    "--sanitize".into(),
                    "--json".into(),
                ],
            ),
        ] {
            let mut argv = vec![
                selected.display().to_string(),
                "memory".into(),
                "--root".into(),
                corpus.display().to_string(),
            ];
            argv.extend(arguments);
            let (exit, stdout, stderr) = fixture.run_with_env(&corpus, argv, 30, &env);
            let tail = |bytes: &[u8]| {
                String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(16 * 1024)..])
                    .into_owned()
            };
            eprintln!(
                "synthetic {label} replay: exit={exit}\nstdout={}\nstderr={}",
                tail(&stdout),
                tail(&stderr)
            );
        }
    }
    assert_eq!(
        code,
        0,
        "item={}\nstdout={}\nstderr={}",
        result["items"]["hardlinked"],
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    );
    assert_eq!(result["items"]["hardlinked"]["status"], "ok");
    assert_eq!(
        result["planned"]["units"]["hardlinked"]["runtime_admitted"],
        true
    );
    assert_eq!(result["summary"]["evaluated"], 1);
    assert_eq!(
        result["meta"]["editions"]["PHP Core"]["runtime_sha256"],
        selected_sha
    );
    assert_eq!(
        source_image(&project),
        original_project,
        "the source Git project changed"
    );
    assert_eq!(digest(&selected), selected_sha);
    assert_eq!(digest(&second), selected_sha);
    assert_eq!(
        digest(original_engine),
        original_sha,
        "the Cargo engine was mutated"
    );
    let (_, after) = selected_parent
        .capture_source(
            selected.file_name().unwrap().to_str().unwrap(),
            EXECUTABLE_LIMIT,
        )
        .unwrap()
        .unwrap();
    assert_eq!(after, selected_identity);
    assert!(
        fs::read_dir(fixture.root.join("codex-home"))
            .unwrap()
            .next()
            .is_none()
    );
    assert!(
        fs::read_dir(fixture.root.join("claude-home"))
            .unwrap()
            .next()
            .is_none()
    );
}

#[test]
fn public_native_ab_plan_admits_only_the_canonical_engine_outside_data_budgets() {
    let fixture = Fixture::new();
    let cargo_engine = Path::new(env!("CARGO_BIN_EXE_infra-engine"));
    let repository =
        fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")).unwrap();
    let selected = fixture.root.join("selected").join(ENGINE_NAME);
    executable::copy(cargo_engine, &selected);
    private_mode(&selected, 0o700);
    let original_sha = digest(cargo_engine);
    assert_eq!(digest(&selected), original_sha);
    let snapshot = fixture.root.join("native-snapshot");
    let installed = snapshot.join("memory-bank/bin").join(ENGINE_NAME);
    fs::create_dir_all(installed.parent().unwrap()).unwrap();
    executable::copy(cargo_engine, &installed);
    private_mode(&installed, 0o755);
    let engine_bytes = fs::metadata(&installed).unwrap().len();
    assert!(
        engine_bytes > 4 * 1024 * 1024,
        "this public regression must exercise a real engine larger than the ordinary file budget"
    );
    fs::write(
        snapshot.join("spec.md"),
        b"# Public fixture\nInvoice numbers are monotonic.\n",
    )
    .unwrap();
    let snapshot_sha = digest(&installed);
    let spec_before = fs::read(snapshot.join("spec.md")).unwrap();
    let config = fixture.root.join("native-config.json");
    fs::write(&config, serde_json::to_vec(&json!({
        "schema":"memory-native-ab-v1",
        "project_snapshot":snapshot,
        "source":{"task_alias":"source-task","prompt":"Read the public invoice specification."},
        "served":{"task_alias":"served-task","prompt":"Explain the public invoice numbering rule."},
        "model":"fixture-no-provider",
        "tool_cap":1,
        "deadline_seconds":30,
    })).unwrap()).unwrap();
    private_mode(&config, 0o600);
    let run = fixture.root.join("native-run");
    let args = |out: &Path| {
        vec![
            selected.display().to_string(),
            "eval".into(),
            "memory-native-ab".into(),
            "plan".into(),
            "--tooling-root".into(),
            repository.display().to_string(),
            "--config".into(),
            config.display().to_string(),
            "--out".into(),
            out.display().to_string(),
            // The canonical plan grammar has no --cache flag. Its default cache
            // remains inside this fixture through the isolated TMPDIR/TEMP values.
        ]
    };
    let (code, stdout, stderr) = fixture.run(&fixture.root, args(&run), 120);
    assert_eq!(
        code,
        0,
        "public native plan failed for real {engine_bytes}-byte engine:\n{}\n{}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    );
    let summary: Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(summary["planned"], 2);
    assert_eq!(summary["attempted"], 0);
    assert_eq!(summary["stages"]["source"], "planned");
    assert_eq!(summary["stages"]["served"], "planned");
    assert!(summary["causal_benefit"].is_null());
    assert!(summary["provider_cost_savings"].is_null());
    let seal: Value =
        serde_json::from_slice(&bounded(&run.join("control/plan.json"), 1024 * 1024)).unwrap();
    let admission = &seal["source_admission"];
    assert_eq!(admission["budget_profile"], "canonical-native-engine-v1");
    assert_eq!(admission["native_engine_bytes"], engine_bytes);
    assert_eq!(admission["data_bytes"], spec_before.len());
    assert_eq!(admission["bytes"], engine_bytes + spec_before.len() as u64);
    assert_eq!(admission["exact_canonical_files"], 1);
    assert_eq!(seal["source_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(digest(&installed), snapshot_sha);
    assert_eq!(fs::read(snapshot.join("spec.md")).unwrap(), spec_before);

    // Same pathname, same mode and a native header do not grant engine authority.
    let mut modified = bounded(&installed, EXECUTABLE_LIMIT);
    *modified.last_mut().unwrap() ^= 1;
    fs::write(&installed, &modified).unwrap();
    private_mode(&installed, 0o755);
    let refused_run = fixture.root.join("native-refused-run");
    let (code, stdout, stderr) = fixture.run(&fixture.root, args(&refused_run), 120);
    assert_ne!(
        code,
        0,
        "a modified installed image was accepted:\n{}\n{}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    );
    assert!(
        !refused_run.exists(),
        "refusal must occur before publishing a plan"
    );
    assert_eq!(
        bounded(&installed, EXECUTABLE_LIMIT),
        modified,
        "admission must not repair or replace untrusted source bytes"
    );
    assert_eq!(digest(&selected), original_sha);
    assert_eq!(digest(cargo_engine), original_sha);
    for home in ["codex-home", "claude-home"] {
        assert!(
            fs::read_dir(fixture.root.join(home))
                .unwrap()
                .next()
                .is_none(),
            "provider-free plan touched a provider home"
        );
    }
}

#[test]
fn public_outcome_seal_consolidation_retains_the_selected_native_engine_without_a_provider() {
    let fixture = Fixture::new();
    let cargo_engine = Path::new(env!("CARGO_BIN_EXE_infra-engine"));
    let repository =
        fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")).unwrap();
    let selected = fixture.root.join("selected").join(ENGINE_NAME);
    executable::copy(cargo_engine, &selected);
    private_mode(&selected, 0o700);
    let engine_sha = digest(&selected);
    assert_eq!(engine_sha, digest(cargo_engine));
    let engine_bytes = fs::metadata(&selected).unwrap().len();
    assert!(engine_bytes > 4 * 1024 * 1024);
    let project = fixture.root.join("projects/fixture");
    fs::create_dir_all(project.join("specs")).unwrap();
    fs::create_dir_all(project.join("project-brain/config")).unwrap();
    fs::write(
        project.join("specs/invoices.md"),
        b"# Invoice numbers\nInvoice numbers are monotonic.\n",
    )
    .unwrap();
    fs::copy(
        repository.join("PHP Core/project-brain/config/runtime.json"),
        project.join("project-brain/config/runtime.json"),
    )
    .unwrap();
    fixture.git(&project, &["init", "--quiet"]);
    fixture.git(&project, &["add", "--", "."]);
    fixture.git(
        &project,
        &["commit", "--quiet", "--no-verify", "-m", "fixture source"],
    );
    let project_before = source_image(&project);
    let tasks = fixture.root.join("outcome-set.json");
    let verifiers = fixture.root.join("outcome-verifiers.json");
    let design = fixture.root.join("outcome-design.json");
    for (path, value) in [
        (
            &tasks,
            json!({"tasks":[{
                "id":"native-engine", "project":"fixture", "ts":"2026-08-05T12:00:00Z",
                "prompt":"State how invoices are numbered.", "task_mode":"answer", "verifier_id":"invoice"
            }]}),
        ),
        (
            &verifiers,
            json!({"cases":{"invoice":{"mode":"answer","equals":"Monotonic invoice numbers."}}}),
        ),
        (
            &design,
            json!({
                "primary":"correctness", "quality_margin":0.1, "minimum_tasks":1,
                "power_rationale":"Provider-free preparation boundary fixture.",
                "selection":"One fixed synthetic public task.",
                "exclusion":"No provider execution or task-outcome inference.",
                "secondary_benefit":"Not measured.",
                "stopping_rule":"fixed-horizon", "unknown_policy":"insufficient"
            }),
        ),
    ] {
        fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
        private_mode(path, 0o600);
    }
    let parsed_design = infra_eval::artifact::read_json(&design, 1024 * 1024).unwrap();
    infra_eval::outcome::validate_design(&parsed_design)
        .expect("the serialized fixture design must pass the actual public design reader");
    let protocol = fixture.root.join("outcome-protocol.json");
    let cache = fixture.root.join("outcome-cache");
    // This local native CLI is used only for Settings' --version probe.
    // The seal endpoint never executes a model/provider command.
    let argv = vec![
        selected.display().to_string(),
        "eval".into(),
        "memory-outcome-ab".into(),
        "seal".into(),
        "--tooling-root".into(),
        repository.display().to_string(),
        "--runtime-binary".into(),
        selected.display().to_string(),
        "--projects-root".into(),
        fixture.root.join("projects").display().to_string(),
        "--transcripts".into(),
        fixture.root.join("claude-home").display().to_string(),
        "--codex-root".into(),
        fixture.root.join("codex-home").display().to_string(),
        "--set".into(),
        tasks.display().to_string(),
        "--verifiers".into(),
        verifiers.display().to_string(),
        "--design".into(),
        design.display().to_string(),
        "--out".into(),
        protocol.display().to_string(),
        "--cache".into(),
        cache.display().to_string(),
        "--arms".into(),
        "off,consolidate".into(),
        "--repeat".into(),
        "1".into(),
        "--cap".into(),
        "1".into(),
        "--timeout".into(),
        "30".into(),
        "--model".into(),
        "fixture-no-provider".into(),
        "--codex".into(),
        selected.display().to_string(),
    ];
    let (code, stdout, stderr) = fixture.run(&fixture.root, argv, 120);
    assert_eq!(
        code,
        0,
        "provider-free seal failed for selected {engine_bytes}-byte engine:\n{}\n{}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    );
    assert!(String::from_utf8_lossy(&stdout).contains("no model was called"));
    let sealed: Value = serde_json::from_slice(&bounded(&protocol, 4 * 1024 * 1024)).unwrap();
    assert_eq!(sealed["schema"], "task-outcome-protocol-v1");
    assert_eq!(sealed["seal_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(sealed["settings"]["arms"], json!(["off", "consolidate"]));
    let base = &sealed["bases"]["native-engine"];
    for image in [base, &base["variants"]["consolidate"]] {
        let root = PathBuf::from(image["path"].as_str().unwrap());
        assert!(
            root.starts_with(&cache),
            "sealed base must stay in the owned cache"
        );
        let installed = root.join("memory-bank/bin").join(ENGINE_NAME);
        assert_eq!(digest(&installed), engine_sha);
        assert_eq!(fs::metadata(&installed).unwrap().len(), engine_bytes);
        let (git_code, staged, git_stderr) = fixture.run(
            &root,
            vec![
                "git".into(),
                "--no-optional-locks".into(),
                "-c".into(),
                "core.fsmonitor=false".into(),
                "ls-files".into(),
                "--".into(),
                format!("memory-bank/bin/{ENGINE_NAME}"),
            ],
            30,
        );
        assert_eq!(git_code, 0, "{}", String::from_utf8_lossy(&git_stderr));
        assert!(
            staged.is_empty(),
            "the native engine must remain outside fresh baseline Git blobs"
        );
    }
    assert_eq!(source_image(&project), project_before);
    assert_eq!(digest(&selected), engine_sha);
    assert_eq!(digest(cargo_engine), engine_sha);
    for home in ["codex-home", "claude-home"] {
        assert!(
            fs::read_dir(fixture.root.join(home))
                .unwrap()
                .next()
                .is_none(),
            "provider-free seal touched a provider home"
        );
    }
}
