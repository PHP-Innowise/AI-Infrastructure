use super::*;
use crate::{evidence, manifest, validate};
use serde_json::json;
use std::{collections::BTreeSet, process::Command};

const LEGACY: &str = include_str!("../../../../../qa/tc-ai/cases.json");
const NATIVE: &str = include_str!("../../../../../qa/tc-ai/cases.native.json");
const RUN: &str = "RUN-20260803-001";

struct Fixture(tempfile::TempDir);
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let fixture = Self(directory);
        for name in ["qa/tc-ai", "schemas/qa", "empty-hooks"] {
            fs::create_dir_all(fixture.root().join(name)).unwrap();
        }
        fs::write(fixture.root().join("empty-git-config"), "").unwrap();
        fs::write(fixture.root().join(LEGACY_PATH), LEGACY).unwrap();
        let mut native: Value = serde_json::from_str(NATIVE).unwrap();
        native["cases"][0]["commands"] = json!([{
            "id":"fixture-native-version", "argv":["git","--version"], "cwd":"."
        }]);
        write_json(&fixture.root().join(NATIVE_PATH), &native).unwrap();
        for (name, raw) in [
            (
                "tc-ai-case.schema.json",
                include_str!("../../../../../schemas/qa/tc-ai-case.schema.json"),
            ),
            (
                "run-evidence.schema.json",
                include_str!("../../../../../schemas/qa/run-evidence.schema.json"),
            ),
            (
                "disposition-ledger.schema.json",
                include_str!("../../../../../schemas/qa/disposition-ledger.schema.json"),
            ),
        ] {
            fs::write(fixture.root().join("schemas/qa").join(name), raw).unwrap();
        }
        fixture.git(&["init", "--quiet"]);
        fixture.git(&["add", "."]);
        fixture.git(&[
            "-c",
            "user.name=Native QA fixture",
            "-c",
            "user.email=qa@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "Synthetic native QA fixture",
        ]);
        fixture
    }
    fn root(&self) -> &Path {
        self.0.path()
    }
    fn git(&self, arguments: &[&str]) {
        let reply = Command::new("git")
            .arg("-c")
            .arg(format!(
                "core.hooksPath={}",
                self.root().join("empty-hooks").display()
            ))
            .args(arguments)
            .current_dir(self.root())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.root().join("empty-git-config"))
            .output()
            .unwrap();
        assert!(
            reply.status.success(),
            "{}",
            String::from_utf8_lossy(&reply.stderr)
        );
    }
    fn run(&self, arguments: &[&str]) -> Result<i32> {
        evidence::run(
            self.root(),
            &arguments
                .iter()
                .map(|arg| (*arg).into())
                .collect::<Vec<_>>(),
        )
    }
    fn source(&self) -> Snapshot {
        Snapshot::read(self.root(), &self.root().join(NATIVE_PATH)).unwrap()
    }
    fn destination(&self) -> PathBuf {
        self.root().join(NATIVE_EVIDENCE).join(RUN)
    }
}
fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for item in fs::read_dir(source).unwrap() {
        let item = item.unwrap();
        let to = destination.join(item.file_name());
        if item.file_type().unwrap().is_dir() {
            copy_tree(&item.path(), &to);
        } else {
            fs::copy(item.path(), to).unwrap();
        }
    }
}
#[test]
fn native_catalog_keeps_identity_hosts_and_explicit_missing_regression_guards() {
    let legacy: Value = serde_json::from_str(LEGACY).unwrap();
    let native: Value = serde_json::from_str(NATIVE).unwrap();
    let old = evidence::catalog(&legacy).unwrap();
    let new = evidence::catalog(&native).unwrap();
    assert_eq!(schema_version(&legacy).unwrap(), LEGACY_VERSION);
    assert_eq!(schema_version(&native).unwrap(), NATIVE_VERSION);
    assert_eq!(new.len(), 18);
    for (before, after) in old.iter().zip(&new) {
        for field in [
            "run_id",
            "test_case_id",
            "classification",
            "native_requirement",
        ] {
            assert_eq!(before[field], after[field], "{field}");
        }
        for command in after["commands"].as_array().unwrap() {
            for argument in command["argv"].as_array().unwrap() {
                let value = argument.as_str().unwrap();
                assert!(!value.contains("{python}"));
                assert!(!value.contains("{python_shell}"));
                assert!(!value.contains("python3 "));
            }
        }
    }
    {
        let number = 16;
        let case = &new[number - 1];
        assert!(
            case["blocker"]
                .as_str()
                .is_some_and(|reason| !reason.is_empty())
        );
        let guards = case["required_test_patterns"].as_array().unwrap();
        assert!(!guards.is_empty());
        assert!(
            guards
                .iter()
                .all(|guard| !guard["glob"].as_str().unwrap().ends_with(".py"))
        );
    }
    let source = include_str!("../../../infra-cli/tests/memory_public_contracts.rs");
    for (number, names) in [
        (
            11,
            &[
                "resolving_verified_record_promotes_only_that_record_before_a_turn_boundary",
                "promotion_remains_optional_and_its_failure_does_not_undo_resolution",
                "reviewed_manual_promotion_requires_independent_approval_and_applies_once",
            ][..],
        ),
        (
            12,
            &[
                "turn_reports_merge_candidate_without_completing_authority_or_losing_pending",
                "governed_completion_requires_an_explicit_numeric_revision_and_preserves_refused_state",
                "concurrent_completion_cas_creates_one_episode_and_preserves_pending_and_other_authority",
                "completion_candidate_ignores_a_never_diverged_branch_even_after_checkout_to_main",
            ][..],
        ),
    ] {
        let case = &new[number - 1];
        assert!(case["blocker"].is_null());
        assert_eq!(case["classification"], "hybrid");
        assert_eq!(case["native_requirement"], "codex");
        let guards = case["required_test_patterns"].as_array().unwrap();
        assert_eq!(guards.len(), names.len());
        for (guard, name) in guards.iter().zip(names) {
            assert_eq!(
                guard["glob"],
                "runtime-rust/crates/infra-cli/tests/memory_public_contracts.rs"
            );
            let pattern = format!("(?m)^fn {name}\\(");
            assert_eq!(guard["regex"], pattern);
            assert!(
                regex::Regex::new(&pattern).unwrap().is_match(source),
                "{name}"
            );
        }
    }
    let generator = include_str!("../../../infra-cli/tests/native_generator.rs");
    let health = include_str!("../../../infra-core/src/health.rs");
    let telemetry_fixture = include_str!("../../../../../qa/tc-ai/fixtures/telemetry-events.json");
    let sources = [
        (
            "runtime-rust/crates/infra-cli/tests/native_generator.rs",
            generator,
        ),
        ("runtime-rust/crates/infra-core/src/health.rs", health),
        ("qa/tc-ai/fixtures/telemetry-events.json", telemetry_fixture),
    ];
    for (number, count) in [(13, 5), (14, 3), (17, 4)] {
        let case = &new[number - 1];
        assert!(case["blocker"].is_null());
        let guards = case["required_test_patterns"].as_array().unwrap();
        assert_eq!(guards.len(), count);
        for guard in guards {
            let path = guard["glob"].as_str().unwrap();
            let raw = sources
                .iter()
                .find(|(candidate, _)| *candidate == path)
                .unwrap()
                .1;
            let pattern = guard["regex"].as_str().unwrap();
            assert!(
                regex::Regex::new(pattern).unwrap().is_match(raw),
                "case{number}: {pattern}"
            );
        }
    }
    for number in [13, 14] {
        let args = new[number - 1]["commands"][0]["argv"].as_array().unwrap();
        assert_eq!(
            &args[args.len() - 3..],
            &[
                json!("generate_edit_classify_keep_revalidate_lifecycle"),
                json!("--"),
                json!("--exact"),
            ]
        );
    }
    assert!(new[12]["native_requirement"].is_null());
    assert_eq!(new[13]["classification"], "hybrid");
    assert_eq!(new[13]["native_requirement"], "claude-code");
    assert!(new[16]["native_requirement"].is_null());
    let fixture: Value = serde_json::from_str(telemetry_fixture).unwrap();
    let sanitized = fixture["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["id"] == "sanitized-adapter")
        .unwrap();
    assert_eq!(sanitized["input_tokens"], 120);
    assert_eq!(sanitized["output_tokens"], 30);
    assert!(sanitized["cost"].is_null());
    // Preserve the generic label; it is not asserted as native event availability.
    assert_eq!(sanitized["expected_status"], "measured");
    assert!(new[17]["commands"].as_array().unwrap().is_empty());
}
#[test]
fn default_run_publishes_native_snapshot_without_rewriting_frozen_inputs() {
    let fixture = Fixture::new();
    let old = fs::read(fixture.root().join(LEGACY_PATH)).unwrap();
    let historical = fixture.root().join("docs/qa/evidence").join(RUN);
    fs::create_dir_all(&historical).unwrap();
    fs::write(
        historical.join("frozen-observation.txt"),
        b"supplied historical bytes\n",
    )
    .unwrap();
    assert_eq!(fixture.run(&["--run-id", RUN]).unwrap(), 0);
    let destination = fixture.destination();
    let source = fixture.source();
    let provenance = read_json(&destination.join("provenance.json")).unwrap();
    assert_eq!(provenance["catalog"], NATIVE_PATH);
    assert_eq!(provenance["catalog_sha256"], source.sha256);
    assert_eq!(
        read_json(&destination.join("case.json")).unwrap(),
        *source.case(RUN).unwrap()
    );
    let validation_errors =
        validate::run_evidence_with_catalog(fixture.root(), &destination, RUN, &source).unwrap();
    assert!(validation_errors.is_empty(), "{validation_errors:#?}");
    assert_eq!(fs::read(fixture.root().join(LEGACY_PATH)).unwrap(), old);
    assert_eq!(
        fs::read(historical.join("frozen-observation.txt")).unwrap(),
        b"supplied historical bytes\n"
    );
    let before = fs::read(destination.join("run.json")).unwrap();
    assert!(fixture.run(&["--run-id", RUN]).is_err());
    assert_eq!(fs::read(destination.join("run.json")).unwrap(), before);
}
#[test]
fn every_selected_output_is_admitted_before_the_first_case_runs() {
    let fixture = Fixture::new();
    let existing = fixture
        .root()
        .join(NATIVE_EVIDENCE)
        .join("RUN-20260803-002");
    fs::create_dir_all(&existing).unwrap();
    fs::write(existing.join("keep.txt"), b"existing evidence").unwrap();
    assert!(
        fixture
            .run(&["--run-id", RUN, "--run-id", "RUN-20260803-002"])
            .is_err()
    );
    assert!(!fixture.destination().exists());
    assert_eq!(
        fs::read(existing.join("keep.txt")).unwrap(),
        b"existing evidence"
    );
}
#[test]
fn a_custom_selected_catalog_records_and_validates_its_raw_image() {
    let fixture = Fixture::new();
    let custom = fixture.root().join("qa/tc-ai/selected.native.json");
    fs::copy(fixture.root().join(NATIVE_PATH), &custom).unwrap();
    let source = Snapshot::read(fixture.root(), &custom).unwrap();
    let destination = evidence::run_case_with_catalog(
        fixture.root(),
        &fixture.root().join(NATIVE_EVIDENCE),
        source.case(RUN).unwrap(),
        &BTreeSet::new(),
        &source,
    )
    .unwrap();
    let provenance = read_json(&destination.join("provenance.json")).unwrap();
    assert_eq!(provenance["catalog"], "qa/tc-ai/selected.native.json");
    assert_eq!(provenance["catalog_sha256"], source.sha256);
    let validation_errors =
        validate::run_evidence_with_catalog(fixture.root(), &destination, RUN, &source).unwrap();
    assert!(validation_errors.is_empty(), "{validation_errors:#?}");
    let mut changed = source.document.clone();
    changed["cases"][1]["title"] = json!("Changed unrelated catalog title");
    write_json(&custom, &changed).unwrap();
    let changed = Snapshot::read(fixture.root(), &custom).unwrap();
    let errors =
        validate::run_evidence_with_catalog(fixture.root(), &destination, RUN, &changed).unwrap();
    assert!(
        errors
            .iter()
            .any(|error| error == "catalog source checksum differs")
    );
}
#[test]
fn synthetic_legacy_bundle_is_validated_against_its_frozen_case_not_native_commands() {
    let fixture = Fixture::new();
    // This disposable v1 protocol fixture records the real Git command below;
    // it never claims that a retired Python suite or historical run executed.
    let mut legacy_document: Value = serde_json::from_str(LEGACY).unwrap();
    legacy_document["cases"][0]["commands"] =
        fixture.source().case(RUN).unwrap()["commands"].clone();
    legacy_document["cases"][0]["title"] = json!("Retained synthetic schema1 reader fixture");
    write_json(&fixture.root().join(LEGACY_PATH), &legacy_document).unwrap();
    let frozen = fs::read(fixture.root().join(LEGACY_PATH)).unwrap();
    assert_eq!(fixture.run(&["--run-id", RUN]).unwrap(), 0);
    let destination = fixture.root().join("docs/qa/evidence").join(RUN);
    copy_tree(&fixture.destination(), &destination);
    let legacy = for_version(fixture.root(), LEGACY_VERSION).unwrap();
    let mut run = read_json(&destination.join("run.json")).unwrap();
    run["schema_version"] = json!(LEGACY_VERSION);
    write_json(&destination.join("run.json"), &run).unwrap();
    write_json(&destination.join("case.json"), legacy.case(RUN).unwrap()).unwrap();
    let mut environment = read_json(&destination.join("environment.json")).unwrap();
    let object = environment.as_object_mut().unwrap();
    for field in ["runtime", "runtime_version", "artifact_contract"] {
        object.remove(field);
    }
    object.insert(
        "python".into(),
        json!("synthetic retained-v1-reader fixture; no legacy runtime executed"),
    );
    write_json(&destination.join("environment.json"), &environment).unwrap();
    let mut provenance = read_json(&destination.join("provenance.json")).unwrap();
    provenance["catalog"] = json!(LEGACY_PATH);
    provenance["runner"] = json!("scripts/qa/run_tc_ai.py");
    provenance.as_object_mut().unwrap().remove("catalog_sha256");
    provenance
        .as_object_mut()
        .unwrap()
        .remove("artifact_contract");
    write_json(&destination.join("provenance.json"), &provenance).unwrap();
    let checksums = destination.join("SHA256SUMS.txt");
    atomic(
        &checksums,
        manifest::render(&destination, &checksums)
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    // Even a caller selecting the native catalog cannot rebind a v1 case snapshot.
    let validation_errors =
        validate::run_evidence_with_catalog(fixture.root(), &destination, RUN, &fixture.source())
            .unwrap();
    assert!(validation_errors.is_empty(), "{validation_errors:#?}");
    assert_eq!(fs::read(fixture.root().join(LEGACY_PATH)).unwrap(), frozen);
}
#[test]
fn declaring_all_native_hosts_never_manufactures_provider_observations() {
    let fixture = Fixture::new();
    let run = "RUN-20260803-018";
    assert_eq!(
        fixture
            .run(&[
                "--run-id",
                run,
                "--native-host",
                "codex",
                "--native-host",
                "cursor",
                "--native-host",
                "claude-code"
            ])
            .unwrap(),
        0
    );
    let result = read_json(
        &fixture
            .root()
            .join(NATIVE_EVIDENCE)
            .join(run)
            .join("run.json"),
    )
    .unwrap();
    assert_eq!(result["status"], "skipped");
    assert_eq!(result["native"]["status"], "skipped");
    assert!(result["commands"].as_array().unwrap().is_empty());
    assert_eq!(result["assertions"]["passed"], 0);
}
#[test]
fn legacy_execution_stays_disabled_but_legacy_listing_remains_available() {
    let fixture = Fixture::new();
    let path = fixture
        .root()
        .join(LEGACY_PATH)
        .to_string_lossy()
        .into_owned();
    assert_eq!(fixture.run(&["--cases", &path, "--list"]).unwrap(), 0);
    assert!(fixture.run(&["--cases", &path, "--run-id", RUN]).is_err());
    assert!(!fixture.root().join(NATIVE_EVIDENCE).exists());
}
