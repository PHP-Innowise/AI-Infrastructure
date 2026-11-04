//! Full native artifact and CLI regressions use Cargo's actual infra-engine binary.
//! Disposable roots only; no provider execution or client trust writes.
use infra_generator::{
    NativeGateServices, assets, common, generated, hook_policy, ownership, publication, seed,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};
static SERIAL: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "infra-generator-native-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(infra_core::fs::canonicalize(path).unwrap())
    }
    fn dir(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(&path).unwrap();
        path
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn write(root: &Path, name: &str, bytes: impl AsRef<[u8]>) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}
fn document(root: &Path, name: &str, value: &Value) {
    write(root, name, serde_json::to_vec_pretty(value).unwrap());
}
fn json_file(root: &Path, name: &str) -> Value {
    serde_json::from_slice(&fs::read(root.join(name)).unwrap()).unwrap()
}
fn selected_engine() -> PathBuf {
    let engine = std::env::var_os("INFRA_NATIVE_TEST_ENGINE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_infra-engine")));
    // Windows starts every child of this process under a guardian, which is
    // the current executable unless named, and a test binary cannot be one.
    infra_harness::process::use_guardian(engine.clone()).unwrap();
    engine
}
fn native_fixture() -> (Vec<u8>, Value) {
    let engine = selected_engine();
    assert!(
        engine.is_absolute(),
        "explicit absolute engine selection required"
    );
    let before = fs::read(&engine).unwrap();
    let output = Command::new(&engine)
        .args(["capabilities", "--json"])
        .output()
        .expect("real native capability admission");
    assert!(
        output.status.success(),
        "native capability admission failed"
    );
    assert_eq!(fs::read(&engine).unwrap(), before, "engine changed");
    let caps: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(caps["engine"], "infra-engine");
    assert_eq!(caps["capability_profile"], "infra-engine-v1");
    (before, caps)
}
#[test]
fn generate_edit_classify_keep_revalidate_lifecycle() {
    let temp = Temp::new();
    let source = temp.dir("standalone-assets");
    let staging = temp.dir("generation target with spaces");
    let delivery = temp.dir("final project with spaces");
    let rejected = temp.dir("rejected-stage");
    let (engine, caps) = native_fixture();
    let artifact = if cfg!(windows) {
        "bin/infra-engine.exe"
    } else {
        "bin/infra-engine"
    };
    let installed = format!("memory-bank/{artifact}");
    write(&source, artifact, &engine);
    let descriptor = json!({"schema_version":1,"engine":"infra-engine","capability_profile":"infra-engine-v1","version":caps["version"],"target":infra_tooling::artifacts::native_target().unwrap(),"path":artifact,"sha256":common::sha(&engine),"bytes":engine.len(),"capabilities":caps["capabilities"]});
    document(&source, "native-artifacts.json", &descriptor);
    document(
        &source,
        "runtime-contract.json",
        &assets::runtime_contract(),
    );
    document(&source, "config/mirror-rules.json", &assets::mirror_rules());
    write(&source, "MCP.md", "# Native memory MCP\n");
    write(&source, "templates/chunk.md", "---\n{}\n---\n");
    for name in [
        "dynamic-record",
        "handoff",
        "message",
        "promotion",
        "retrieval-manifest",
        "token-usage-event",
    ] {
        document(
            &source,
            &format!("project-brain/schemas/{name}.schema.json"),
            &json!({"type":"object"}),
        );
    }
    for name in [
        "task", "finding", "bug", "incident", "decision", "event", "handoff",
    ] {
        write(
            &source,
            &format!("project-brain/templates/{name}.md"),
            "---\n{}\n---\n",
        );
    }
    for name in ["retrieval-manifest", "promotion"] {
        document(
            &source,
            &format!("project-brain/templates/{name}.json"),
            &json!({}),
        );
    }
    for name in ["README.md", "PROTOCOL.md", ".gitignore"] {
        write(
            &source,
            &format!("project-brain/{name}"),
            "# Static Brain seed\n",
        );
    }
    for name in ["providers", "telemetry"] {
        document(
            &source,
            &format!("project-brain/config/{name}.json"),
            &json!({}),
        );
    }
    for name in ["active", "archive"] {
        document(
            &source,
            &format!("project-brain/indexes/{name}.json"),
            &json!([]),
        );
    }
    write(
        &source,
        "project-brain/config/runtime.json.template",
        r#"{"framework":"{{TARGET_FRAMEWORK}}","canonical_edition":"{{CANONICAL_EDITION}}"}"#,
    );
    for name in [
        "AGENTS.md",
        ".agents/skills/unreviewed/SKILL.md",
        ".claude/hooks/unreviewed.sh",
        "README.md",
        "INDEX.md",
        ".memory-counter",
        "chunks/MEM-unreviewed.md",
        "project-brain/dynamic/tasks/unreviewed.md",
    ] {
        write(&source, name, b"must not cross the scoped asset closure\n");
    }
    let mut bad = descriptor.clone();
    bad["sha256"] = json!("0".repeat(64));
    document(&source, "native-artifacts.json", &bad);
    assert!(seed::assets(&source, &rejected, "symfony", &["codex".into()], None).is_err());
    assert_eq!(fs::read_dir(&rejected).unwrap().count(), 0);
    document(&source, "native-artifacts.json", &descriptor);
    let emitted = seed::assets(
        &source,
        &staging,
        "symfony",
        &["claude".into(), "codex".into()],
        None,
    )
    .unwrap();
    assert_eq!(emitted["staging_only"], true);
    assert_eq!(emitted["canonical_edition"], ".agents");
    assert_eq!(fs::read(staging.join(&installed)).unwrap(), engine);
    // A generated project's root .gitignore need not name the engine.
    assert_eq!(
        fs::read_to_string(staging.join("memory-bank/bin/.gitignore")).unwrap(),
        infra_tooling::launch::ENGINE_IGNORE
    );
    assert_eq!(
        json_file(&staging, "project-brain/config/runtime.json"),
        json!({"framework":"symfony","canonical_edition":".agents"})
    );
    for name in [
        "AGENTS.md",
        ".agents",
        ".claude",
        "memory-bank/README.md",
        "memory-bank/INDEX.md",
        "memory-bank/.memory-counter",
        "memory-bank/chunks/MEM-unreviewed.md",
        "project-brain/dynamic/tasks/unreviewed.md",
        "runtime-rust",
        "install",
    ] {
        assert!(
            !staging.join(name).exists(),
            "unapproved asset escaped: {name}"
        );
    }
    let repeated = seed::assets(
        &source,
        &staging,
        "symfony",
        &["claude".into(), "codex".into()],
        None,
    )
    .unwrap();
    assert!(repeated["copied"].as_array().unwrap().is_empty());
    // Real registration writes only staging; emitted argv name the final root.
    infra_tooling::mcp_config::register_at(
        &staging,
        &delivery,
        &["claude".into(), "cursor".into(), "codex".into()],
        false,
    )
    .unwrap();
    for (host, path) in [("claude", ".mcp.json"), ("cursor", ".cursor/mcp.json")] {
        let value = json_file(&staging, path);
        assert_eq!(
            value["mcpServers"]["harness-memory"],
            infra_tooling::mcp_config::entry(host, &delivery).unwrap()
        );
        assert!(
            !String::from_utf8(fs::read(staging.join(path)).unwrap())
                .unwrap()
                .contains(staging.to_str().unwrap())
        );
    }
    assert_eq!(
        fs::read_to_string(staging.join(".codex/config.toml")).unwrap(),
        infra_tooling::mcp_config::codex_block(&delivery).unwrap()
    );
    assert_eq!(
        fs::read_dir(&delivery).unwrap().count(),
        0,
        "registration wrote final target"
    );

    // The whole-ready-edition compatibility overlay cannot silently accept a
    // different final destination: that would leave published hooks bound to stage.
    let selected_engine = selected_engine();
    let refused = Command::new(&selected_engine)
        .arg("creator")
        .arg("seed-runtime")
        .arg("--source")
        .arg(temp.0.join("intentionally-missing-ready-edition-source"))
        .arg("--staging")
        .arg(&staging)
        .arg("--edition")
        .arg("PHP Core")
        .arg("--delivery-root")
        .arg(&delivery)
        .output()
        .unwrap();
    assert!(!refused.status.success());
    let diagnostic = format!(
        "{}{}",
        String::from_utf8_lossy(&refused.stdout),
        String::from_utf8_lossy(&refused.stderr)
    );
    assert!(
        diagnostic.contains("seed-runtime binds ready-edition hooks and MCP to --staging"),
        "{diagnostic}"
    );
    assert_eq!(fs::read_dir(&delivery).unwrap().count(), 0);
    let handler = infra_tooling::attach::hook_handler(
        &delivery.join(&installed),
        &delivery,
        "claude",
        "session-init",
        cfg!(windows),
    )
    .unwrap();
    assert!(
        !serde_json::to_string(&handler)
            .unwrap()
            .contains(staging.to_str().unwrap())
    );
    exercise_generated_pipeline(&temp, &staging, &delivery, &engine, &installed);
    #[cfg(windows)]
    {
        assert_eq!(
            handler["command"],
            delivery.join(&installed).to_str().unwrap()
        );
        assert_eq!(
            handler["args"],
            json!(["hook", "claude", "session-init", "--root", delivery])
        );
    }
    #[cfg(not(windows))]
    {
        let command = handler["command"].as_str().unwrap();
        assert!(command.starts_with("exec '"));
        assert!(command.contains(delivery.join(&installed).to_str().unwrap()));
        assert!(command.contains("'--root'"));
        assert!(command.contains(delivery.to_str().unwrap()));
    }
}

fn creator_command(cwd: &Path, operation: &str) -> Command {
    let mut command = Command::new(selected_engine());
    command.current_dir(cwd).args(["creator", operation]);
    command
}
fn run_creator(command: &mut Command, label: &str) -> std::process::Output {
    let output = command.output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{label}: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
#[derive(Clone, Copy, Debug)]
enum TargetPathCase {
    Relative,
    Absolute,
}
fn caller_relative_target_generation_and_validation_use_the_callers_cwd(
    temp: &Temp,
    options: &generated::GeneratedOptions,
    owned: &[String],
) {
    exercise_public_target_paths(temp, options, owned, TargetPathCase::Relative);
}
fn absolute_target_generation_and_validation_are_independent_of_the_callers_cwd(
    temp: &Temp,
    options: &generated::GeneratedOptions,
    owned: &[String],
) {
    exercise_public_target_paths(temp, options, owned, TargetPathCase::Absolute);
}

fn generated_payload_image(root: &Path, owned: &[String]) -> BTreeMap<String, (String, u32)> {
    owned
        .iter()
        .map(|name| {
            let path = root.join(name);
            let metadata = fs::metadata(&path).unwrap();
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o777
            };
            #[cfg(not(unix))]
            let mode = u32::from(metadata.permissions().readonly());
            (name.clone(), (common::sha(&fs::read(path).unwrap()), mode))
        })
        .collect()
}
fn target_validation_command(
    cwd: &Path,
    target: &Path,
    options: &generated::GeneratedOptions,
    generator: &Path,
) -> Command {
    let mut command = Command::new(selected_engine());
    command
        .current_dir(cwd)
        .args(["creator", "validate-generated", "--target"])
        .arg(target)
        .arg("--delivery-root")
        .arg(options.delivery_root.as_ref().unwrap())
        .args(["--editions", "claude,cursor,codex", "--approved-profile"])
        .arg(options.approved_profile.as_ref().unwrap())
        .arg("--approved-profile-sha256")
        .arg(options.approved_profile_sha256.as_ref().unwrap())
        .arg("--profile-path")
        .arg(options.profile_path.as_ref().unwrap())
        .arg("--candidate-registry")
        .arg(&options.candidate_registry)
        .args(["--generator-version", "1.0.0", "--generator-root"])
        .arg(generator);
    command
}
fn exercise_public_target_paths(
    temp: &Temp,
    options: &generated::GeneratedOptions,
    owned: &[String],
    case: TargetPathCase,
) {
    let target = &options.target;
    let relative = PathBuf::from(target.file_name().unwrap());
    assert!(relative.to_str().unwrap().contains(' '));
    let other = temp.dir("unrelated caller with spaces");
    let generator = temp.dir("separate generator metadata");
    write(
        &generator.join(&relative),
        "AGENTS.md",
        b"decoy generator-root target must never be selected\n",
    );
    let decoy = fs::read(generator.join(&relative).join("AGENTS.md")).unwrap();
    let plan = temp.0.join("target path write plan.txt");
    fs::write(&plan, owned.join("\n") + "\n").unwrap();
    let original_manifest = fs::read(target.join(".infra-manifest.json")).unwrap();
    let original_payload = generated_payload_image(target, owned);
    let expected: Value = serde_json::from_slice(&original_manifest).unwrap();
    let (cwd, argument) = match case {
        TargetPathCase::Relative => (&temp.0, relative.as_path()),
        TargetPathCase::Absolute => (&other, target.as_path()),
    };
    for mode in ["full", "merge"] {
        // The actual public generation helper must resolve target just as the
        // public validator does. No process-global current directory is changed.
        let output = Command::new(selected_engine())
            .current_dir(cwd)
            .args(["creator", "ownership", "manifest", "--target"])
            .arg(argument)
            .arg("--write-plan")
            .arg(&plan)
            .args([
                "--generator",
                "NativeFixture",
                "--version",
                "1.0.0",
                "--task",
                "TASK-1",
                "--profile",
            ])
            .arg(options.profile_path.as_ref().unwrap())
            .args([
                "--editions",
                "claude,cursor,codex",
                "--mode",
                mode,
                "--generator-root",
            ])
            .arg(&generator)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "manifest generation ({case:?}, {mode}): stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let mut actual = json_file(target, ".infra-manifest.json");
        let mut baseline = expected.clone();
        baseline["mode"] = json!(mode);
        // This public helper intentionally stamps its own generation time.
        // Compare every other manifest field without inventing a test-only flag.
        assert!(actual["generated_at"].as_str().is_some());
        actual.as_object_mut().unwrap().remove("generated_at");
        baseline.as_object_mut().unwrap().remove("generated_at");
        assert_eq!(actual, baseline);
        let validation = target_validation_command(cwd, argument, options, &generator)
            .output()
            .unwrap();
        assert!(
            validation.status.success(),
            "validation ({case:?}, {mode}): stdout={} stderr={}",
            String::from_utf8_lossy(&validation.stdout),
            String::from_utf8_lossy(&validation.stderr)
        );
        assert_eq!(validation.stdout, b"generated accelerator OK\n");
        assert!(validation.stderr.is_empty());
        assert_eq!(generated_payload_image(target, owned), original_payload);
    }
    let wrong_target = other.join(&relative);
    assert!(!wrong_target.exists());
    let before_wrong_call = fs::read(target.join(".infra-manifest.json")).unwrap();
    let refused = target_validation_command(&other, &relative, options, &generator)
        .output()
        .unwrap();
    assert!(
        !refused.status.success(),
        "wrong-CWD relative target was accepted"
    );
    let error = format!(
        "{}{}",
        String::from_utf8_lossy(&refused.stdout),
        String::from_utf8_lossy(&refused.stderr)
    );
    assert!(error.contains("Target is not a directory"), "{error}");
    assert!(
        !wrong_target.exists(),
        "validator created its missing target"
    );
    assert_eq!(
        fs::read(target.join(".infra-manifest.json")).unwrap(),
        before_wrong_call
    );
    assert_eq!(generated_payload_image(target, owned), original_payload);
    assert_eq!(
        fs::read(generator.join(&relative).join("AGENTS.md")).unwrap(),
        decoy
    );
    write(target, ".infra-manifest.json", original_manifest);
}

fn exercise_generated_pipeline(
    temp: &Temp,
    stage: &Path,
    delivery: &Path,
    engine: &[u8],
    installed: &str,
) {
    let selected = infra_core::fs::canonicalize(selected_engine()).unwrap();
    let gate = infra_cli::generator_gate::Gate {
        binary: selected,
        digest: common::sha(engine),
        size: engine.len() as u64,
    };
    let reviewed = "# Reviewed native fixture\n<!-- infra-hook-policy-selection-v1 -->\n```json\n{\"schema_version\":1,\"framework\":\"symfony\",\"approved_tools\":[],\"project_command_prefixes\":[],\"deny_rules\":[]}\n```\n<!-- /infra-hook-policy-selection-v1 -->\n";
    write(&temp.0, "reviewed-profile.md", reviewed);
    let report = temp.0.join("reviewed-profile.md");
    let approved = common::sha(reviewed.as_bytes());
    let profile_ref = "tasks/infra-scan-TASK-1/infra-scan-project-profile.md";
    hook_policy::materialize(&report, &approved, profile_ref, stage).unwrap();
    write(
        stage,
        "AGENTS.md",
        "<!-- Generated by NativeFixture v1.0.0 | TASK-1 | reviewed native fixture -->\n# Fixture policy\nUse canonical sources and inspect evidence before changes.\n",
    );
    write(stage, ".claude/CLAUDE.md", "@../AGENTS.md\n");
    write(
        stage,
        "memory-bank/README.md",
        "# Memory Bank\nNo durable knowledge has been authored for this fixture.\n",
    );
    write(
        stage,
        "memory-bank/INDEX.md",
        "# Memory Bank index\nNo durable chunks.\n",
    );
    for host in ["claude", "cursor", "codex"] {
        let skills = if host == "codex" {
            ".agents/skills".to_string()
        } else {
            format!(".{host}/skills")
        };
        for guide in [
            "memory-bank",
            "project-brain",
            "checkpoint",
            "memory",
            "context-save",
            "context-load",
        ] {
            write(
                stage,
                &format!("{skills}/{guide}/SKILL.md"),
                format!(
                    "---\nname: {guide}\ndescription: Native {guide} runtime guide\n---\nInspect the native runtime contract and verify canonical sources before acting.\n"
                ),
            );
            if host != "codex" {
                write(
                    stage,
                    &format!(".{host}/agents/{guide}-agent.md"),
                    format!(
                        "---\nname: {guide}-agent\ninvokes: {guide}\n---\nExecute the named skill once and stop.\n"
                    ),
                );
                write(
                    stage,
                    &format!(".{host}/commands/{guide}.md"),
                    format!(
                        "---\nname: {guide}\nspawns: {guide}-agent\n---\nRoute the request to its skill.\n"
                    ),
                );
            }
        }
        for role in infra_cli::hooks::ROLES {
            let path = format!(".{host}/hooks/{role}.sh");
            write(
                stage,
                &path,
                infra_cli::hooks::shim(host, role, cfg!(windows)).unwrap(),
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(stage.join(&path), fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        let mappings: Vec<(&str, Vec<&str>)> = if host == "cursor" {
            vec![
                ("sessionStart", vec!["local-context", "context-continuity"]),
                (
                    "beforeSubmitPrompt",
                    vec!["working-memory-read", "context-continuity"],
                ),
                ("subagentStart", vec!["subagent-gate"]),
                ("subagentStop", vec!["subagent-dispatch"]),
                ("beforeShellExecution", vec!["bash-validator"]),
                (
                    "afterFileEdit",
                    vec!["file-naming-validator", "loop-detection"],
                ),
                ("afterAgentResponse", vec!["context-continuity"]),
                ("stop", vec!["working-memory-write"]),
            ]
        } else {
            let mut events = vec![
                ("SessionStart", vec!["local-context", "context-continuity"]),
                (
                    "UserPromptSubmit",
                    vec!["working-memory-read", "context-continuity"],
                ),
                ("Stop", vec!["working-memory-write", "context-continuity"]),
                (
                    "PreToolUse",
                    vec!["bash-validator", "file-naming-validator", "subagent-gate"],
                ),
                ("PostToolUse", vec!["loop-detection"]),
            ];
            if host == "claude" {
                events.last_mut().unwrap().1.push("subagent-dispatch");
                events.push(("SubagentStop", vec!["subagent-dispatch"]));
            }
            events
        };
        let mut hooks = serde_json::Map::new();
        for (event, roles) in mappings {
            let handlers = roles
                .iter()
                .map(|role| {
                    let mut value =
                        infra_cli::generator_gate::handler(delivery, host, role).unwrap();
                    if host != "cursor" {
                        value["type"] = json!("command");
                    }
                    value
                })
                .collect::<Vec<_>>();
            hooks.insert(event.into(), json!([{"hooks":handlers}]));
        }
        let path = if host == "claude" {
            ".claude/settings.json".to_string()
        } else {
            format!(".{host}/hooks.json")
        };
        document(stage, &path, &json!({"hooks":hooks}));
    }
    let codex = fs::read_to_string(stage.join(".codex/config.toml")).unwrap();
    write(
        stage,
        ".codex/config.toml",
        format!("[features]\nhooks = true\n\n{codex}"),
    );
    let inventory = ownership::staged_files(stage).unwrap();
    let owned_paths = inventory
        .keys()
        .filter(|path| ownership::ownable(path))
        .cloned()
        .collect::<Vec<_>>();
    let manifest = ownership::build_manifest(
        stage,
        &owned_paths,
        &ownership::ManifestRequest {
            generator: "NativeFixture".into(),
            version: "1.0.0".into(),
            task: "TASK-1".into(),
            profile: profile_ref.into(),
            editions: vec!["claude".into(), "cursor".into(), "codex".into()],
            mode: "full".into(),
            decisions: json!({}),
            generated_at: None,
        },
        None,
    )
    .unwrap();
    ownership::write_manifest(stage, &manifest).unwrap();
    let owned: BTreeMap<String, String> =
        serde_json::from_value(manifest["files"].clone()).unwrap();
    for host in ["claude", "cursor", "codex"] {
        let errors = gate.validate_host(stage, delivery, host, &owned).unwrap();
        assert!(errors.is_empty(), "actual {host} native Gate: {errors:?}");
    }
    // Producer uses underscore for the Codex table, hyphen for JSON clients.
    let valid_codex = fs::read_to_string(stage.join(".codex/config.toml")).unwrap();
    let wrong_key = valid_codex.replace(
        "[mcp_servers.harness_memory]",
        "[mcp_servers.harness-memory]",
    );
    assert_ne!(wrong_key, valid_codex);
    write(stage, ".codex/config.toml", &wrong_key);
    let errors = gate
        .validate_host(stage, delivery, "codex", &owned)
        .unwrap();
    assert!(
        errors
            .iter()
            .any(|error| error.contains("Memory MCP registration")),
        "{errors:?}"
    );
    for host in ["claude", "cursor"] {
        assert!(
            gate.validate_host(stage, delivery, host, &owned)
                .unwrap()
                .is_empty()
        );
    }
    // A registration bound to the absolute staging engine names discarded staging.
    let bound = format!(
        "{}\n[mcp_servers.harness_memory]\ncommand = {}\nargs = {}\nenabled = true\n{}\n",
        infra_tooling::mcp_config::BEGIN,
        serde_json::to_string(&stage.join(installed)).unwrap(),
        serde_json::to_string(&json!(["mcp", "--root", stage, "--host", "codex"])).unwrap(),
        infra_tooling::mcp_config::END
    );
    write(
        stage,
        ".codex/config.toml",
        format!("[features]\nhooks = true\n\n{bound}"),
    );
    let errors = gate
        .validate_host(stage, delivery, "codex", &owned)
        .unwrap();
    assert!(
        errors
            .iter()
            .any(|error| error.contains("Memory MCP registration")),
        "staging-bound MCP was accepted: {errors:?}"
    );
    write(stage, ".codex/config.toml", &valid_codex);
    let registry = temp.0.join("candidate-registry.json");
    document(&temp.0, "candidate-registry.json", &json!({}));
    let mut options = generated::GeneratedOptions {
        target: stage.to_path_buf(),
        delivery_root: Some(delivery.to_path_buf()),
        editions: vec!["claude".into(), "cursor".into(), "codex".into()],
        plan: None,
        evidence_target: None,
        approved_profile: Some(report),
        approved_profile_sha256: Some(approved),
        profile_path: Some(profile_ref.into()),
        candidate_registry: registry,
        generator_version: Some("1.0.0".into()),
        runtime_contract: assets::runtime_contract(),
    };
    // Structural/runtime validation is real. Skill-plan semantics have their own
    // contracts; this fixture deliberately does not fabricate a semantic plan.
    let errors = generated::validate(&options, &gate).unwrap();
    assert!(errors.is_empty(), "staged generated tree: {errors:?}");
    for poisoned in [false, true] {
        let mut command = Command::new(selected_engine());
        command
            .current_dir(&temp.0)
            .arg("creator")
            .arg("validate-generated")
            .arg("--target")
            .arg(stage)
            .arg("--delivery-root")
            .arg(delivery)
            .arg("--editions")
            .arg("claude,cursor,codex")
            .arg("--approved-profile")
            .arg(options.approved_profile.as_ref().unwrap())
            .arg("--approved-profile-sha256")
            .arg(options.approved_profile_sha256.as_ref().unwrap())
            .arg("--profile-path")
            .arg(profile_ref)
            .arg("--candidate-registry")
            .arg(&options.candidate_registry)
            .arg("--generator-version")
            .arg("1.0.0");
        if poisoned {
            // Installed generated trees are independent of the process that
            // launches the generator. Set only this child's environment.
            command
                .env("ACCELERATOR_HOME", stage)
                .env("ACCELERATOR_STATE_DIR", stage)
                .env("ACCELERATOR_PROJECT_DIR", stage)
                .env("ACCELERATOR_EDITION", "untrusted-ambient-host");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "public generated validation (poisoned={poisoned}): stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"generated accelerator OK\n");
    }
    caller_relative_target_generation_and_validation_use_the_callers_cwd(
        temp,
        &options,
        &owned_paths,
    );
    absolute_target_generation_and_validation_are_independent_of_the_callers_cwd(
        temp,
        &options,
        &owned_paths,
    );
    let mut paths = inventory.keys().cloned().collect::<Vec<_>>();
    paths.push(".infra-manifest.json".into());
    let baseline = publication::build_snapshot(delivery, &paths, &[]).unwrap();
    publication::publish(
        delivery,
        stage,
        &paths,
        &baseline,
        &temp.0.join("pipeline-journal"),
        &[],
        &[],
    )
    .unwrap();
    options.target = delivery.to_path_buf();
    let errors = generated::validate(&options, &gate).unwrap();
    assert!(errors.is_empty(), "published generated tree: {errors:?}");
    assert_eq!(fs::read(delivery.join(installed)).unwrap(), engine);
    for host in ["claude", "cursor", "codex"] {
        assert!(
            gate.validate_host(delivery, delivery, host, &owned)
                .unwrap()
                .is_empty()
        );
    }
    generated_update_keep_revalidate_lifecycle(temp, stage, delivery, &options, &owned_paths);
}

fn generated_update_keep_revalidate_lifecycle(
    temp: &Temp,
    stage: &Path,
    delivery: &Path,
    options: &generated::GeneratedOptions,
    owned_paths: &[String],
) {
    const KEPT: &str = ".agents/skills/memory/SKILL.md";
    const TEAM: &str = ".agents/team-notes.md";
    let previous_manifest = fs::read(delivery.join(ownership::MANIFEST)).unwrap();
    let original = fs::read_to_string(delivery.join(KEPT)).unwrap();
    let kept = format!("{original}\nTeam rule: review the query before changing its cache.\n");
    let proposal = format!(
        "{}\nGenerated update fixture rule.\n",
        fs::read_to_string(stage.join(KEPT)).unwrap()
    );
    write(delivery, KEPT, kept.as_bytes());
    write(
        delivery,
        TEAM,
        b"TODO team-owned notes must remain untouched.\n",
    );
    write(stage, KEPT, proposal.as_bytes());
    let agents = fs::read_to_string(stage.join("AGENTS.md")).unwrap();
    let next_agents = agents.replacen("| TASK-1 |", "| TASK-2 |", 1);
    assert_ne!(agents, next_agents);
    write(stage, "AGENTS.md", next_agents.as_bytes());
    let classify = || {
        let output = run_creator(
            creator_command(&temp.0, "ownership")
                .arg("classify")
                .arg("--target")
                .arg(delivery)
                .arg("--staging")
                .arg(stage),
            "native update classification",
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let rows = classify();
    let rows = rows.as_array().unwrap();
    assert!(rows.iter().any(|row| row["path"] == KEPT
        && row["classification"] == "requires-decision"
        && row["reason"] == "tracked-file-modified"));
    assert!(
        rows.iter()
            .any(|row| row["path"] == "AGENTS.md" && row["classification"] == "safe-update")
    );
    assert!(rows.iter().all(|row| row["path"] != TEAM));
    assert_eq!(fs::read(delivery.join(KEPT)).unwrap(), kept.as_bytes());
    assert_eq!(
        fs::read(delivery.join(ownership::MANIFEST)).unwrap(),
        previous_manifest
    );
    // Explicit fixture keep; never adopt the declined proposal as final bytes.
    let rejected = common::sha(proposal.as_bytes());
    document(
        &temp.0,
        "keep-decisions.json",
        &json!({
            (KEPT): {"decision":"kept","rejected_sha256":rejected,"task":"TASK-2"}
        }),
    );
    document(&temp.0, "keep-sources.json", &json!({(KEPT):"target"}));
    write(
        &temp.0,
        "keep-write-plan.txt",
        format!("{}\n", owned_paths.join("\n")),
    );
    run_creator(
        creator_command(&temp.0, "ownership")
            .arg("manifest")
            .arg("--target")
            .arg(stage)
            .arg("--write-plan")
            .arg(temp.0.join("keep-write-plan.txt"))
            .arg("--generator")
            .arg("NativeFixture")
            .arg("--version")
            .arg("1.0.0")
            .arg("--task")
            .arg("TASK-2")
            .arg("--profile")
            .arg(options.profile_path.as_ref().unwrap())
            .arg("--editions")
            .arg(options.editions.join(","))
            .arg("--mode")
            .arg("full")
            .arg("--decisions")
            .arg(temp.0.join("keep-decisions.json"))
            .arg("--source-target")
            .arg(delivery)
            .arg("--source-map")
            .arg(temp.0.join("keep-sources.json")),
        "native keep manifest",
    );
    let staged_manifest = json_file(stage, ownership::MANIFEST);
    assert_eq!(staged_manifest["files"][KEPT], common::sha(kept.as_bytes()));
    assert_eq!(
        staged_manifest["decisions"][KEPT]["rejected_sha256"],
        rejected
    );
    assert_eq!(fs::read(stage.join(KEPT)).unwrap(), proposal.as_bytes());
    assert_eq!(
        fs::read(delivery.join(ownership::MANIFEST)).unwrap(),
        previous_manifest
    );
    // Publish only accepted policy+manifest; keep is a baseline-only watch.
    write(
        &temp.0,
        "keep-publication-plan.txt",
        b"AGENTS.md\n.infra-manifest.json\n",
    );
    write(&temp.0, "keep-watch-plan.txt", format!("{KEPT}\n"));
    let baseline = temp.0.join("keep-baseline.json");
    run_creator(
        creator_command(&temp.0, "publication")
            .arg("snapshot")
            .arg("--target")
            .arg(delivery)
            .arg("--publication-plan")
            .arg(temp.0.join("keep-publication-plan.txt"))
            .arg("--watch-plan")
            .arg(temp.0.join("keep-watch-plan.txt"))
            .arg("--output")
            .arg(&baseline),
        "native keep baseline",
    );
    run_creator(
        creator_command(&temp.0, "publication")
            .arg("publish")
            .arg("--target")
            .arg(delivery)
            .arg("--staging")
            .arg(stage)
            .arg("--publication-plan")
            .arg(temp.0.join("keep-publication-plan.txt"))
            .arg("--watch-plan")
            .arg(temp.0.join("keep-watch-plan.txt"))
            .arg("--baseline")
            .arg(&baseline)
            .arg("--journal")
            .arg(temp.0.join("keep-publication-journal")),
        "native keep publication",
    );
    assert_eq!(fs::read(delivery.join(KEPT)).unwrap(), kept.as_bytes());
    assert_eq!(
        fs::read(delivery.join(TEAM)).unwrap(),
        b"TODO team-owned notes must remain untouched.\n"
    );
    assert_eq!(
        fs::read(delivery.join("AGENTS.md")).unwrap(),
        next_agents.as_bytes()
    );
    let manifest = json_file(delivery, ownership::MANIFEST);
    assert_eq!(manifest["task"], "TASK-2");
    assert_eq!(manifest["files"][KEPT], common::sha(kept.as_bytes()));
    assert!(manifest["files"].get(TEAM).is_none());
    assert_eq!(
        manifest["files"].as_object().unwrap().len(),
        owned_paths.len()
    );
    run_creator(
        creator_command(&temp.0, "validate-generated")
            .arg("--target")
            .arg(delivery)
            .arg("--delivery-root")
            .arg(delivery)
            .arg("--editions")
            .arg(options.editions.join(","))
            .arg("--approved-profile")
            .arg(options.approved_profile.as_ref().unwrap())
            .arg("--approved-profile-sha256")
            .arg(options.approved_profile_sha256.as_ref().unwrap())
            .arg("--profile-path")
            .arg(options.profile_path.as_ref().unwrap())
            .arg("--candidate-registry")
            .arg(&options.candidate_registry)
            .arg("--generator-version")
            .arg(options.generator_version.as_ref().unwrap()),
        "native kept target revalidation",
    );
    let rows = classify();
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["path"] == KEPT)
        .unwrap();
    assert_eq!(row["classification"], "standing-decision-honored");
    assert_eq!(row["reason"], "kept");
    write(
        stage,
        KEPT,
        format!("{proposal}\nA different later proposal.\n"),
    );
    let rows = classify();
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["path"] == KEPT)
        .unwrap();
    assert_eq!(row["classification"], "requires-decision");
    assert_eq!(row["reason"], "generator-output-changed");
    assert_eq!(fs::read(delivery.join(KEPT)).unwrap(), kept.as_bytes());
}

fn creator_source(temp: &Temp, name: &str) -> PathBuf {
    let source = temp.dir(name);
    for scope in [".agents", ".claude", ".cursor", ".codex"] {
        fs::create_dir_all(source.join(scope)).unwrap();
    }
    write(&source, "AGENTS.md", "# Canonical Creator fixture\n");
    write(&source, "VERSION", "1.0.0\n");
    write(&source, "README.md", "# Generator fixture\n");
    document(
        &source,
        "project-brain/config/runtime.json",
        &json!({"framework":"creator"}),
    );
    write(&source, ".agents/native-helper.sh", "#!/bin/sh\nexit 0\n");
    document(
        &source,
        ".agents/skills/memory-seed/assets/runtime-contract.json",
        &assets::runtime_contract(),
    );
    write(
        &source,
        "memory-bank/local/private.db",
        b"local user state must never be copied",
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            source.join(".agents/native-helper.sh"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        fs::set_permissions(source.join("README.md"), fs::Permissions::from_mode(0o640)).unwrap();
    }
    source
}
fn source_engine_binding(source: &Path, bytes: &[u8], caps: &Value) {
    let file = if cfg!(windows) {
        "infra-engine.exe"
    } else {
        "infra-engine"
    };
    for (binary, metadata, declared) in [
        (
            format!("memory-bank/bin/{file}"),
            "memory-bank/runtime-capabilities.json".to_string(),
            format!("memory-bank/bin/{file}"),
        ),
        (
            format!(".agents/skills/memory-seed/assets/bin/{file}"),
            ".agents/skills/memory-seed/assets/native-artifacts.json".to_string(),
            format!("bin/{file}"),
        ),
    ] {
        write(source, &binary, bytes);
        // Source images are validated as data and must not be executed.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(source.join(&binary), fs::Permissions::from_mode(0o600)).unwrap();
        }
        document(
            source,
            &metadata,
            &json!({
                "schema_version":1,"engine":"infra-engine","capability_profile":"infra-engine-v1",
                "version":caps["version"],"target":infra_tooling::artifacts::native_target().unwrap(),
                "path":declared,"sha256":common::sha(bytes),"bytes":bytes.len(),"capabilities":caps["capabilities"]
            }),
        );
    }
}
fn assert_creator_engine_copy(destination: &Path, expected: &[u8]) {
    let file = if cfg!(windows) {
        "infra-engine.exe"
    } else {
        "infra-engine"
    };
    for (binary, metadata) in [
        (
            format!("memory-bank/bin/{file}"),
            "memory-bank/runtime-capabilities.json",
        ),
        (
            format!(".agents/skills/memory-seed/assets/bin/{file}"),
            ".agents/skills/memory-seed/assets/native-artifacts.json",
        ),
    ] {
        assert_eq!(
            fs::read(destination.join(&binary)).unwrap().as_slice(),
            expected
        );
        let descriptor = json_file(destination, metadata);
        assert_eq!(descriptor["sha256"], common::sha(expected));
        assert_eq!(descriptor["bytes"].as_u64(), Some(expected.len() as u64));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(destination.join(binary))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o755
            );
        }
    }
    assert!(!destination.join("memory-bank/local").exists());
    assert_eq!(
        json_file(destination, "project-brain/config/runtime.json")["framework"],
        "creator"
    );
    assert!(destination.join("tasks/TASK-001").is_dir());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(destination.join(".agents/native-helper.sh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        assert_eq!(
            fs::metadata(destination.join("README.md"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
    }
}
#[test]
fn creator_copy_materializes_admitted_engine_with_or_without_release_artifacts() {
    use infra_harness::creator_files;
    let temp = Temp::new();
    let (engine, caps) = native_fixture();
    let mut admitted = creator_files::admit_engine(&selected_engine()).unwrap();
    let checkout = creator_source(&temp, "unassembled-creator");
    let destination = temp.0.join("checkout-copy");
    creator_files::copy_generator(&checkout, &destination, &mut admitted).unwrap();
    assert_creator_engine_copy(&destination, &engine);

    // A second native image can have different release/build metadata bytes
    // while retaining the same supported executable code and capability profile.
    let mut other_release = engine.clone();
    other_release.extend_from_slice(b"\0Creator alternate release build metadata\n");
    assert_ne!(common::sha(&other_release), common::sha(&engine));
    let assembled = creator_source(&temp, "assembled-creator");
    source_engine_binding(&assembled, &other_release, &caps);
    let destination = temp.0.join("assembled-copy");
    creator_files::copy_generator(&assembled, &destination, &mut admitted).unwrap();
    assert_creator_engine_copy(&destination, &engine);
    assert_eq!(
        json_file(&assembled, "memory-bank/runtime-capabilities.json")["sha256"],
        common::sha(&other_release)
    );
}
#[test]
fn creator_copy_rejects_bad_source_bindings_before_destination_writes() {
    use infra_harness::creator_files;
    let temp = Temp::new();
    let (engine, caps) = native_fixture();
    let mut admitted = creator_files::admit_engine(&selected_engine()).unwrap();
    let source = creator_source(&temp, "tampered-source");
    source_engine_binding(&source, &engine, &caps);
    let child = source.join("overlapping-copy");
    assert!(creator_files::copy_generator(&source, &child, &mut admitted).is_err());
    assert!(
        !child.exists(),
        "overlap created a destination inside the source"
    );
    let before = fs::read(source.join("AGENTS.md")).unwrap();
    assert!(creator_files::copy_generator(&source, &source, &mut admitted).is_err());
    assert_eq!(fs::read(source.join("AGENTS.md")).unwrap(), before);
    for (index, path) in [
        "memory-bank/runtime-capabilities.json",
        ".agents/skills/memory-seed/assets/native-artifacts.json",
    ]
    .iter()
    .enumerate()
    {
        let before = fs::read(source.join(path)).unwrap();
        let mut bad = json_file(&source, path);
        bad["sha256"] = json!("0".repeat(64));
        document(&source, path, &bad);
        let destination = temp.0.join(format!("refused-{index}"));
        assert!(creator_files::copy_generator(&source, &destination, &mut admitted).is_err());
        assert!(
            !destination.exists(),
            "bad binding caused partial working-copy writes"
        );
        write(&source, path, before);
    }
    let config = "project-brain/config/runtime.json";
    let approved_config = fs::read(source.join(config)).unwrap();
    for (index, invalid) in [
        b"{}".as_slice(),
        br#"{"framework":"generic"}"#.as_slice(),
        b"null".as_slice(),
        b"{".as_slice(),
    ]
    .iter()
    .enumerate()
    {
        write(&source, config, invalid);
        let destination = temp.0.join(format!("refused-framework-{index}"));
        assert!(creator_files::copy_generator(&source, &destination, &mut admitted).is_err());
        assert!(
            !destination.exists(),
            "bad Creator identity caused destination writes"
        );
    }
    fs::remove_file(source.join(config)).unwrap();
    let destination = temp.0.join("refused-missing-framework");
    assert!(creator_files::copy_generator(&source, &destination, &mut admitted).is_err());
    assert!(!destination.exists());
    write(&source, config, approved_config);

    fs::remove_file(source.join("memory-bank/runtime-capabilities.json")).unwrap();
    let destination = temp.0.join("refused-orphan-engine");
    assert!(creator_files::copy_generator(&source, &destination, &mut admitted).is_err());
    assert!(!destination.exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        source_engine_binding(&source, &engine, &caps);
        fs::remove_file(source.join(".agents/native-helper.sh")).unwrap();
        symlink(
            source.join("README.md"),
            source.join(".agents/native-helper.sh"),
        )
        .unwrap();
        let destination = temp.0.join("refused-payload-link");
        assert!(creator_files::copy_generator(&source, &destination, &mut admitted).is_err());
        assert!(!destination.exists());
    }
}

fn creator_hook_request(
    executable: &Path,
    arguments: &[std::ffi::OsString],
    root: &Path,
    command: &str,
) -> infra_tooling::process::ProcessOutput {
    let mut environment: BTreeMap<std::ffi::OsString, Option<std::ffi::OsString>> = [
        "ACCELERATOR_HOME",
        "ACCELERATOR_STATE_DIR",
        "ACCELERATOR_PROJECT_DIR",
        "ACCELERATOR_EDITION",
    ]
    .into_iter()
    .map(|name| (std::ffi::OsString::from(name), None))
    .collect();
    let counters = root.join(".native-hook-counters");
    infra_core::security::secure_private_dir(&counters).unwrap();
    environment.insert("TMPDIR".into(), Some(counters.into_os_string()));
    let payload = serde_json::to_vec(&json!({
        "tool_name":"Bash", "tool_input":{"command":command},
        "session_id":format!("creatorhook{}x{}", std::process::id(), SERIAL.fetch_add(1, Ordering::Relaxed)),
        "cwd":root
    })).unwrap();
    // Invoke only the native hook with a JSON tool description. The described
    // PHP/WordPress commands are never executed, and no provider is launched.
    infra_tooling::process::run_with_env(
        executable.as_os_str(),
        arguments,
        root,
        Some(payload),
        std::time::Duration::from_secs(10),
        256 * 1024,
        &environment,
    )
    .unwrap()
}
fn creator_native_hook(
    engine: &Path,
    root: &Path,
    host: &str,
    command: &str,
) -> infra_tooling::process::ProcessOutput {
    creator_hook_request(
        engine,
        &[
            "hook".into(),
            host.into(),
            "bash-validator".into(),
            "--root".into(),
            root.as_os_str().to_owned(),
        ],
        root,
        command,
    )
}

#[test]
fn copied_creator_hooks_preserve_framework_union_after_rename() {
    use infra_harness::creator_files;
    let temp = Temp::new();
    let source =
        infra_core::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.."))
            .unwrap()
            .join("Infrastructure-Creator");
    let agent = temp.0.join("agent");
    let mut engine = creator_files::admit_engine(&selected_engine()).unwrap();
    creator_files::copy_generator(&source, &agent, &mut engine).unwrap();
    assert_eq!(
        fs::read(agent.join(".agents/skills/memory-seed/assets/runtime-contract.json")).unwrap(),
        fs::read(source.join(".agents/skills/memory-seed/assets/runtime-contract.json")).unwrap()
    );
    let baseline = temp.dir("default-runtime");
    let baseline =
        infra_core::RuntimeConfig::load(&infra_core::Roots::installed(baseline).unwrap()).unwrap();
    let mut configured =
        infra_core::RuntimeConfig::load(&infra_core::Roots::installed(&agent).unwrap()).unwrap();
    assert_eq!(configured.value["framework"], "creator");
    configured.value["framework"] = json!("generic");
    assert_eq!(
        configured.value, baseline.value,
        "identity must preserve runtime policy/automatic defaults"
    );
    for path in [
        "project-brain/local",
        "project-brain/control",
        "project-brain/dynamic",
        "project-brain/indexes",
    ] {
        assert!(
            !agent.join(path).exists(),
            "copy admitted broader Brain state: {path}"
        );
    }
    let binary = agent.join(if cfg!(windows) {
        "memory-bank/bin/infra-engine.exe"
    } else {
        "memory-bank/bin/infra-engine"
    });
    for host in ["claude", "cursor", "codex"] {
        for command in [
            "php artisan db:seed --force",
            "php bin/console doctrine:database:drop --force",
            "wp db reset --yes",
        ] {
            let output = creator_native_hook(&binary, &agent, host, command);
            assert_eq!(
                output.code,
                2,
                "{host}/{command}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(String::from_utf8_lossy(&output.stderr).contains("rule category:"));
        }
        let output = creator_native_hook(&binary, &agent, host, "php -v");
        assert_eq!(
            output.code,
            0,
            "{host}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        #[cfg(unix)]
        {
            let wrapper = agent.join(format!(".{host}/hooks/bash-validator.sh"));
            let output = creator_hook_request(&wrapper, &[], &agent, "php artisan db:seed --force");
            assert_eq!(
                output.code,
                2,
                "copied {host} wrapper: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

#[test]
fn configured_framework_is_independent_of_creator_directory_name() {
    let temp = Temp::new();
    for (index, framework, seed_denied, doctrine_denied) in [
        (0, None, false, false),
        (1, Some("generic"), false, false),
        (2, Some("laravel"), true, false),
        (3, Some("symfony"), false, true),
    ] {
        let root = temp.dir(&format!("project-{index}/Infrastructure-Creator"));
        if let Some(framework) = framework {
            document(
                &root,
                "project-brain/config/runtime.json",
                &json!({"framework":framework}),
            );
        }
        for (command, denied) in [
            ("php artisan db:seed --force", seed_denied),
            (
                "php bin/console doctrine:database:drop --force",
                doctrine_denied,
            ),
        ] {
            let output = creator_native_hook(&selected_engine(), &root, "codex", command);
            assert_eq!(
                output.code,
                if denied { 2 } else { 0 },
                "{framework:?}/{command}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
