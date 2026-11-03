//! Disposable roots only; no provider execution or client trust writes.
use infra_core::bank::publication::PublicationFiles;
use infra_generator::{common, hook_policy, publication};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
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
        Self(fs::canonicalize(path).unwrap())
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
fn selection() -> Value {
    json!({"schema_version":1,"framework":"symfony","approved_tools":["console"],"project_command_prefixes":[{"tool":"console","argv":["php","bin/console"],"options_with_values":["--env","-e"],"short_options_with_values":"e"}],"deny_rules":[{"id":"drop-database","tool":"console","command":["doctrine:database:drop"],"matching":"console","condition":{"kind":"always"},"reason":"destructive_command"}]})
}
fn profile(selection: &Value) -> String {
    format!(
        "# Reviewed project profile\n\n<!-- infra-hook-policy-selection-v1 -->\n```json\n{}\n```\n<!-- /infra-hook-policy-selection-v1 -->\n",
        serde_json::to_string_pretty(selection).unwrap()
    )
}
const PROFILE_REF: &str = "tasks/infra-scan-TASK-1/infra-scan-project-profile.md";
fn bind_policy(root: &Path, raw: &[u8]) {
    let mut manifest = json!({"profile":PROFILE_REF,"files":{}});
    manifest["files"][hook_policy::PATH] = json!(common::sha(raw));
    document(root, ".infra-manifest.json", &manifest);
}

#[test]
fn reviewed_policy_binds_raw_profile_full_selection_and_manifest() {
    let temp = Temp::new();
    let staging = temp.dir("staging");
    let reviewed = profile(&selection());
    write(&temp.0, "profile.md", &reviewed);
    let report = temp.0.join("profile.md");
    let approved = common::sha(reviewed.as_bytes());
    let inspected = hook_policy::inspect(&report, PROFILE_REF).unwrap();
    assert_eq!(inspected["profile_sha256"], approved);
    assert_eq!(inspected["selection"], selection());
    assert_eq!(
        inspected["selection_sha256"],
        common::sha(
            infra_core::domain::json::canonical(&selection())
                .unwrap()
                .as_bytes()
        )
    );
    let emitted = hook_policy::materialize(&report, &approved, PROFILE_REF, &staging).unwrap();
    let raw = fs::read(staging.join(hook_policy::PATH)).unwrap();
    assert_eq!(emitted["sha256"], common::sha(&raw));
    assert_eq!(
        json_file(&staging, hook_policy::SCHEMA_PATH),
        hook_policy::schema()
    );
    document(
        &staging,
        "project-brain/config/runtime.json",
        &json!({"framework":"symfony"}),
    );
    bind_policy(&staging, &raw);
    assert_eq!(
        hook_policy::validate(&staging, &report, &approved, PROFILE_REF).unwrap()["valid"],
        true
    );
    // Rehashing a forged selection and manifest never substitutes for human review.
    let mut forged = json_file(&staging, hook_policy::PATH);
    forged["deny_rules"] = json!([]);
    let mut chosen = selection();
    chosen["deny_rules"] = json!([]);
    forged["evidence"]["selection_sha256"] = json!(common::sha(
        infra_core::domain::json::canonical(&chosen)
            .unwrap()
            .as_bytes()
    ));
    let forged_raw = serde_json::to_vec_pretty(&forged).unwrap();
    write(&staging, hook_policy::PATH, &forged_raw);
    bind_policy(&staging, &forged_raw);
    assert!(hook_policy::validate(&staging, &report, &approved, PROFILE_REF).is_err());
    write(&staging, hook_policy::PATH, &raw);
    bind_policy(&staging, &raw);
    let mut manifest = json_file(&staging, ".infra-manifest.json");
    manifest["files"][hook_policy::PATH] = json!("0".repeat(64));
    document(&staging, ".infra-manifest.json", &manifest);
    assert!(hook_policy::validate(&staging, &report, &approved, PROFILE_REF).is_err());
    bind_policy(&staging, &raw);
    // SHA covers prose outside the JSON block too.
    write(
        &temp.0,
        "profile.md",
        format!("{reviewed}\nUnreviewed profile change.\n"),
    );
    assert!(hook_policy::materialize(&report, &approved, PROFILE_REF, &staging).is_err());
    assert_eq!(fs::read(staging.join(hook_policy::PATH)).unwrap(), raw);
}
#[test]
fn malformed_policies_and_unreviewable_profiles_are_refused() {
    let temp = Temp::new();
    let staging = temp.dir("staging");
    let reviewed = profile(&selection());
    write(&temp.0, "profile.md", &reviewed);
    let report = temp.0.join("profile.md");
    let approved = common::sha(reviewed.as_bytes());
    hook_policy::materialize(&report, &approved, PROFILE_REF, &staging).unwrap();
    document(
        &staging,
        "project-brain/config/runtime.json",
        &json!({"framework":"symfony"}),
    );
    let raw = fs::read(staging.join(hook_policy::PATH)).unwrap();
    let valid: Value = serde_json::from_slice(&raw).unwrap();
    let mut unknown = valid.clone();
    unknown["execute"] = json!("unreviewed");
    let mut c1 = valid.clone();
    c1["project_command_prefixes"][0]["argv"][1] = json!("bin/\u{0085}console");
    let mut framework = valid.clone();
    framework["framework"] = json!("laravel");
    let mut condition = valid.clone();
    condition["deny_rules"][0]["condition"]["patterns"] = json!([]);
    let duplicate = String::from_utf8(raw.clone())
        .unwrap()
        .replacen(
            "\"schema_version\": 1",
            "\"schema_version\": 1, \"schema_version\": 1",
            1,
        )
        .into_bytes();
    assert_ne!(duplicate, raw, "duplicate-key fixture");
    let mut cases = vec![duplicate];
    for value in [unknown, c1, framework, condition] {
        cases.push(serde_json::to_vec(&value).unwrap());
    }
    let mut oversized = raw.clone();
    oversized.resize(hook_policy::LIMIT + 1, b' ');
    cases.push(oversized);
    for (index, invalid) in cases.iter().enumerate() {
        write(&staging, hook_policy::PATH, invalid);
        bind_policy(&staging, invalid);
        assert!(
            hook_policy::validate(&staging, &report, &approved, PROFILE_REF).is_err(),
            "invalid policy {index}"
        );
    }
    write(
        &temp.0,
        "profile.md",
        "# Legacy profile without selection\n",
    );
    assert!(hook_policy::inspect(&report, PROFILE_REF).is_err());
    write(&temp.0, "profile.md", format!("{reviewed}\n{reviewed}"));
    assert!(hook_policy::inspect(&report, PROFILE_REF).is_err());
    write(&temp.0, "profile.md", &reviewed);
    assert!(hook_policy::inspect(&report, "tasks/\u{0085}profile.md").is_err());
    let mut selected = selection();
    selected["project_command_prefixes"][0]["argv"][0] = json!("p\u{0085}hp");
    write(&temp.0, "profile.md", profile(&selected));
    assert!(hook_policy::inspect(&report, PROFILE_REF).is_err());
}
fn staged(temp: &Temp) -> (PathBuf, PathBuf, Vec<String>, Value) {
    let target = temp.dir("target");
    let staging = temp.dir("staging");
    write(&target, "AGENTS.md", b"team-approved old policy\n");
    write(&staging, "AGENTS.md", b"new generated policy\n");
    document(
        &target,
        ".infra-manifest.json",
        &json!({"files":{"AGENTS.md":common::sha(b"team-approved old policy\n")}}),
    );
    document(
        &staging,
        ".infra-manifest.json",
        &json!({"files":{"AGENTS.md":common::sha(b"new generated policy\n")}}),
    );
    let paths = vec!["AGENTS.md".into(), ".infra-manifest.json".into()];
    let baseline = publication::build_snapshot(&target, &paths, &[]).unwrap();
    (target, staging, paths, baseline)
}
#[test]
fn publication_rejects_replaced_target_identity_even_with_identical_bytes() {
    let temp = Temp::new();
    let (target, staging, paths, baseline) = staged(&temp);
    let original = fs::read(target.join("AGENTS.md")).unwrap();
    fs::rename(
        target.join("AGENTS.md"),
        temp.0.join("retained-original.md"),
    )
    .unwrap();
    write(&target, "AGENTS.md", &original);
    assert!(publication::verify_baseline(&target, &paths, &baseline, &[]).is_err());
    assert!(
        publication::publish(
            &target,
            &staging,
            &paths,
            &baseline,
            &temp.0.join("journal"),
            &[],
            &[]
        )
        .is_err()
    );
    assert_eq!(fs::read(target.join("AGENTS.md")).unwrap(), original);
    assert!(
        !temp.0.join("journal").exists(),
        "stale review fails before writes"
    );
}
#[test]
fn source_reader_refuses_a_named_source_swapped_after_open() {
    let temp = Temp::new();
    let source = temp.dir("source");
    write(&source, "AGENTS.md", b"selected staged bytes\n");
    let mut custody = PublicationFiles::new(&source).unwrap();
    let result = custody.read_checked("AGENTS.md", 1024, |_| {
        fs::rename(source.join("AGENTS.md"), temp.0.join("opened-original.md"))?;
        write(&source, "AGENTS.md", b"selected staged bytes\n");
        Ok(())
    });
    // A platform can deny the rename while held, or reject changed named identity.
    assert!(result.is_err());
}
#[test]
fn committed_publication_rolls_back_its_own_effects_idempotently() {
    let temp = Temp::new();
    let (target, staging, paths, baseline) = staged(&temp);
    let before = paths
        .iter()
        .map(|path| (path.clone(), fs::read(target.join(path)).unwrap()))
        .collect::<BTreeMap<_, _>>();
    let journal = temp.0.join("journal");
    let published =
        publication::publish(&target, &staging, &paths, &baseline, &journal, &[], &[]).unwrap();
    assert_eq!(published["schema_version"], 2);
    assert_eq!(published["status"], "published-pending-verification");
    for path in &paths {
        assert_eq!(
            fs::read(target.join(path)).unwrap(),
            fs::read(staging.join(path)).unwrap()
        );
        assert_eq!(published["effects"][path]["committed"], true);
        assert!(published["effects"][path]["after_identity"].is_array());
        assert!(journal.join("backups").join(path).is_file());
    }
    let restored = publication::restore(&target, &journal, None).unwrap();
    assert_eq!(restored["status"], "rolled-back");
    for (path, bytes) in &before {
        assert_eq!(fs::read(target.join(path)).unwrap(), *bytes);
    }
    assert_eq!(
        publication::restore(&target, &journal, None).unwrap(),
        restored
    );
}
#[test]
fn rollback_preserves_unrelated_same_bytes_after_image() {
    let temp = Temp::new();
    let (target, staging, paths, baseline) = staged(&temp);
    let journal = temp.0.join("journal");
    publication::publish(&target, &staging, &paths, &baseline, &journal, &[], &[]).unwrap();
    let committed = fs::read(target.join("AGENTS.md")).unwrap();
    let permissions = fs::metadata(target.join("AGENTS.md"))
        .unwrap()
        .permissions();
    fs::rename(
        target.join("AGENTS.md"),
        temp.0.join("retained-committed.md"),
    )
    .unwrap();
    write(&target, "AGENTS.md", &committed);
    fs::set_permissions(target.join("AGENTS.md"), permissions).unwrap();
    let substituted = PublicationFiles::new(&target)
        .unwrap()
        .image_identity("AGENTS.md")
        .unwrap();
    assert!(publication::restore(&target, &journal, None).is_err());
    assert_eq!(fs::read(target.join("AGENTS.md")).unwrap(), committed);
    assert_eq!(
        PublicationFiles::new(&target)
            .unwrap()
            .image_identity("AGENTS.md")
            .unwrap(),
        substituted,
        "rollback preserves the unrelated inode"
    );
    let durable = json_file(&journal, "journal.json");
    assert_eq!(durable["status"], "rolled-back-with-conflicts");
    assert!(
        durable["conflicts"]
            .as_array()
            .unwrap()
            .contains(&json!("AGENTS.md"))
    );
}

#[test]
fn ownership_decision_cross_fields_match_the_canonical_contract() {
    use infra_generator::{generated, ownership};
    let temp = Temp::new();
    let root = temp.dir("decisions");
    let body = b"<!-- Generated by NativeFixture v1.0.0 | TASK-1 | reviewed -->\n# Policy\n";
    write(&root, "AGENTS.md", body);
    let resolved = common::sha(body);
    let files = BTreeMap::from([("AGENTS.md".to_string(), resolved.clone())]);
    for (origin, strategy, decision, requirements, valid) in [
        ("generated", "keep", "kept", vec![], true),
        ("generated", "manual-merge", "merged", vec![], true),
        ("preexisting-team", "manual-merge", "merged", vec![], true),
        ("shared", "keep", "kept", vec!["preserve-team-rule"], true),
        (
            "shared",
            "append-requirements",
            "merged",
            vec!["preserve-team-rule"],
            true,
        ),
        ("generated", "append-requirements", "merged", vec![], false),
        (
            "preexisting-team",
            "append-requirements",
            "merged",
            vec!["rule"],
            false,
        ),
        ("shared", "append-requirements", "merged", vec![], false),
        ("shared", "keep", "kept", vec![], false),
        ("shared", "manual-merge", "merged", vec!["rule"], false),
        ("generated", "manual-merge", "merged", vec!["rule"], false),
        ("preexisting-team", "keep", "kept", vec!["rule"], false),
    ] {
        let entry = json!({
            "decision":decision,"task":"TASK-1","rejected_sha256":"a".repeat(64),
            "origin":origin,"strategy":strategy,"proposal_sha256":"a".repeat(64),
            "resolved_sha256":resolved,"requirements":requirements,
            "future_additive_metadata":{"retained":true}
        });
        let decisions = json!({"AGENTS.md":entry});
        assert_eq!(
            ownership::validate_decisions(&decisions, &files).is_ok(),
            valid,
            "{origin}/{strategy}/{decision}"
        );
        document(
            &root,
            ".infra-manifest.json",
            &json!({
                "manifest_version":1,"generator":"NativeFixture","generator_version":"1.0.0",
                "task":"TASK-1","profile":PROFILE_REF,"editions":["codex"],"mode":"merge",
                "files":files,"decisions":decisions
            }),
        );
        let (_, diagnostics) = generated::validate_manifest(&root, None).unwrap();
        assert_eq!(
            diagnostics.is_empty(),
            valid,
            "{origin}/{strategy}: {diagnostics:?}"
        );
        if !valid {
            assert!(
                diagnostics
                    .iter()
                    .any(|item| item.starts_with("manifest decisions:"))
            );
        }
    }
    let legacy = json!({"AGENTS.md":{
        "decision":"kept","task":"TASK-1","rejected_sha256":"a".repeat(64),
        "future_additive_metadata":true
    }});
    ownership::validate_decisions(&legacy, &files).unwrap();
}
#[test]
fn fnmatch_brackets_preserve_python_class_and_literal_rules() {
    for (pattern, value, expected) in [
        ("[^a]", "a", true),
        ("[^a]", "^", true),
        ("[^a]", "z", false),
        ("[!a]", "a", false),
        ("[!a]", "z", true),
        ("[[]", "[", true),
        ("[]]", "]", true),
        ("assets/[[]id].js", "assets/[id].js", true),
        ("[a-c]", "b", true),
        ("[a-c]", "z", false),
        ("[a-b-c]", "-", true),
        ("[a-b-c]", "c", true),
        ("[z-a]", "z", false),
        ("[!z-a]", "z", true),
        ("[a&&b]", "&", true),
        ("[a&&b]", "c", false),
        ("[", "[", true),
        ("[]", "[]", true),
        ("*[a-c]?", "prefix/b\n", true),
        ("**", "\n", true),
        (r"[\\]", r"\", true),
        ("[Z-a]", "a", true),
        ("[é-ê]", "ê", true),
        ("[ê-é]", "é", false),
        ("[!ê-é]", "\n", true),
        ("[!]]", "]", false),
        ("[!]]", "a", true),
        ("[a~~b]", "~", true),
        ("[a||b]", "|", true),
        ("[a--b]", "b", true),
        ("[a--b]", "a", false),
        ("a?b", "a\nb", true),
        ("a?b", "a💡b", true),
    ] {
        assert_eq!(
            common::glob_match_case(pattern, value),
            expected,
            "{pattern:?}/{value:?}"
        );
    }
    assert!(infra_generator::coverage::matches("assets/[^a].js", "file", "assets/a.js").unwrap());
    assert!(!infra_generator::coverage::matches("assets/[^a].js", "file", "assets/z.js").unwrap());
    assert!(
        infra_generator::coverage::matches("assets/[[]id].js", "file", "assets/[id].js").unwrap()
    );
}

#[test]
fn fnmatch_platform_normalization_keeps_case_contract_separate() {
    assert!(common::glob_match_case("[Z-a]", "a"));
    assert_eq!(common::glob_match("[Z-a]", "a").unwrap(), !cfg!(windows));
    for (pattern, value) in [
        (".env", ".ENV"),
        ("*.pem", "KEY.PEM"),
        ("[é-ê]", "Ê"),
        ("src/*.PHP", "src/Controller.php"),
        (r"src\*.PHP", "src/Controller.php"),
        ("src/*.php", r"src\Controller.php"),
        ("a\0b", "A\0B"),
    ] {
        assert_eq!(
            common::glob_match(pattern, value).unwrap(),
            cfg!(windows),
            "platform fnmatch {pattern:?}/{value:?}"
        );
        assert!(
            !common::glob_match_case(pattern, value),
            "case-sensitive fnmatch {pattern:?}/{value:?}"
        );
    }
    assert!(common::glob_match("", "").unwrap());
    assert!(!common::glob_match("", "a").unwrap());
    for (pattern, value) in [
        ("src/*.PHP", "src/Controller.php"),
        (r"src\*.PHP", "src/Controller.php"),
        ("src/*.php", r"src\Controller.php"),
    ] {
        assert_eq!(
            infra_generator::coverage::matches(pattern, "file", value).unwrap(),
            cfg!(windows),
            "coverage {pattern:?}/{value:?}"
        );
    }
}

#[test]
fn scanner_secret_dispositions_follow_platform_fnmatch() {
    let temp = Temp::new();
    for (index, surface, expected) in [
        (0, ".env", true),
        (1, ".ENV", cfg!(windows)),
        (2, "KEY.PEM", cfg!(windows)),
    ] {
        let target = temp.dir(&format!("secret-target-{index}"));
        let task = temp.dir(&format!("secret-task-{index}"));
        write(&target, surface, b"Synthetic non-secret fixture.\n");
        document(
            &task,
            "fixture-coverage.json",
            &json!({
                "scanner":"fixture","target_root":target,
                "surfaces":[{
                    "surface":surface,"kind":"file","disposition":"covered",
                    "reason":"Exercise the coverage admission contract.","evidence_ids":["E-1"]
                }]
            }),
        );
        document(
            &task,
            "fixture-evidence.json",
            &json!({
                "evidence":[{"id":"E-1","path":surface}]
            }),
        );
        document(
            &task,
            "project-claims.json",
            &json!({
                "target_root":target,"claims":[],"contradictions":[]
            }),
        );
        let diagnostics = infra_generator::coverage::validate(&target, &task, None);
        assert_eq!(
            diagnostics
                .iter()
                .filter(|item| item.code == "SCAN_SECRET_COVERED")
                .count(),
            usize::from(expected),
            "{surface}: {diagnostics:?}"
        );
        assert!(
            diagnostics
                .iter()
                .all(|item| item.code == "SCAN_SECRET_COVERED"),
            "fixture reaches only secret classification: {surface}: {diagnostics:?}"
        );
    }
}

#[test]
fn structured_decisions_require_a_resolved_digest_before_equality() {
    use infra_generator::ownership;
    let valid = "b".repeat(64);
    for (resolved, accepted) in [
        (json!(valid), true),
        (json!("not-a-digest"), false),
        (json!("B".repeat(64)), false),
        (json!("g".repeat(64)), false),
        (json!("b".repeat(63)), false),
        (json!(""), false),
        (Value::Null, false),
    ] {
        // Keep the map and the helper's text view equal, so equality cannot mask
        // the independently required digest-format check.
        let files = BTreeMap::from([(
            "AGENTS.md".to_string(),
            resolved.as_str().unwrap_or("").to_string(),
        )]);
        let decisions = json!({"AGENTS.md":{
            "decision":"kept","task":"TASK-1","rejected_sha256":"a".repeat(64),
            "origin":"generated","strategy":"keep","proposal_sha256":"a".repeat(64),
            "resolved_sha256":resolved,"requirements":[]
        }});
        assert_eq!(
            ownership::validate_decisions(&decisions, &files).is_ok(),
            accepted,
            "resolved value: {resolved:?}"
        );
    }
}
