//! Persistent semantic keys follow the frozen Python 3.14 / Unicode 16 contract.
use infra_core::{
    BrainRepository, CreateRecord, Learning, RecordResult, ResultService, Revision, Roots,
    RuntimeContext, UpdateRecord, domain, query, result_store, results,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

const TASK: &str = "00000000-0000-4000-8000-000000000001";
const SOURCE: &str = "specs/cache.md";
const CONSEQUENCE: &str = "Cache lookups preserve their canonical identity.";

fn learning(title: &str) -> Learning {
    Learning {
        kind: "finding".into(),
        title: title.into(),
        consequence: CONSEQUENCE.into(),
        sources: vec![SOURCE.into()],
        expected_revision: None,
        record_id: None,
        source_fingerprints: None,
    }
}

#[test]
fn full_default_casefold_matches_the_pinned_unicode16_reference_for_every_scalar() {
    let raw = include_bytes!("../resources/unicode/CaseFolding-16.0.0.txt");
    assert_eq!(
        query::sha256(raw),
        "6f1f9c588eb4a5c718d9e8f93b782685e5c7fec872cf05e8e6878053599e09bb"
    );
    assert_eq!(
        query::sha256(include_bytes!("../resources/unicode/LICENSE.txt")),
        "e7a93b009565cfce55919a381437ac4db883e9da2126fa28b91d12732bc53d96"
    );
    let mut expected = BTreeMap::new();
    for line in std::str::from_utf8(raw).unwrap().lines() {
        let line = line.split('#').next().unwrap().trim();
        if line.is_empty() {
            continue;
        }
        let fields = line.split(';').map(str::trim).collect::<Vec<_>>();
        if !["C", "F"].contains(&fields[1]) {
            continue;
        }
        let scalar = u32::from_str_radix(fields[0], 16).unwrap();
        let mapped = fields[2]
            .split_whitespace()
            .map(|part| char::from_u32(u32::from_str_radix(part, 16).unwrap()).unwrap())
            .collect::<String>();
        assert!(expected.insert(scalar, mapped).is_none());
    }
    assert_eq!(expected.len(), 1557);
    let mut compared = 0;
    for scalar in 0..=0x10ffff {
        let Some(character) = char::from_u32(scalar) else {
            continue;
        };
        let input = character.to_string();
        let actual = query::folded(&input);
        assert_eq!(
            &actual,
            expected.get(&scalar).unwrap_or(&input),
            "U+{scalar:04X}"
        );
        compared += 1;
    }
    assert_eq!(compared, 1_112_064);
    assert_eq!(
        query::folded("Straße İ Σς \u{13a0}\u{ab70}"),
        "strasse i\u{307} σσ \u{13a0}\u{13a0}"
    );
    assert_eq!(query::folded(" \t\u{1c89}\n"), " \t\u{1c8a}\n");
    assert_eq!(query::folded("A\u{30a}"), "a\u{30a}");
    assert_ne!(
        query::folded("\u{c5}"),
        query::folded("A\u{30a}"),
        "folding adds no Unicode normalization"
    );
    assert_eq!(
        query::folded("\u{16ea0}"),
        "\u{16ea0}",
        "Unicode 17 additions must not inherit compiler case tables"
    );
}

#[test]
fn unicode_learning_ids_keep_the_frozen_canonical_hashes() {
    for (upper, lower, expected) in [
        (
            "Cache \u{1c89} identity",
            "Cache \u{1c8a} identity",
            "learning-c116ab76ca27a0e63439f0bd",
        ),
        (
            "Cache \u{1c90} identity",
            "Cache \u{10d0} identity",
            "learning-53f4b5f19b079099e35090b3",
        ),
    ] {
        assert_eq!(
            results::learning_id(TASK, &learning(upper)).unwrap(),
            expected
        );
        assert_eq!(
            results::learning_id(TASK, &learning(lower)).unwrap(),
            expected
        );
        let mut referenced = learning(upper);
        referenced.record_id = Some("00000000-0000-4000-8000-000000000002".into());
        let id = results::learning_id(TASK, &referenced).unwrap();
        referenced.title = lower.into();
        assert_eq!(
            results::learning_id(TASK, &referenced).unwrap(),
            id,
            "explicit record identity stays independent of title folding"
        );
    }
}

struct Fixture {
    root: PathBuf,
    context: RuntimeContext,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("infra-casefold-record-{}", domain::uuid4()));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("specs")).unwrap();
        fs::write(
            root.join(SOURCE),
            "# Cache identity\nThe canonical key is stable.\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("project-brain/schemas")).unwrap();
        let schema = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../PHP Core/project-brain/schemas/dynamic-record.schema.json");
        fs::copy(
            schema,
            root.join("project-brain/schemas/dynamic-record.schema.json"),
        )
        .unwrap();
        let context =
            RuntimeContext::new(Roots::installed(&root).unwrap(), "local", "governed").unwrap();
        Self { root, context }
    }
    fn brain(&self) -> BrainRepository {
        BrainRepository::new(self.context.clone()).unwrap()
    }
    fn service(&self) -> ResultService {
        ResultService::new(self.context.clone())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn payload(title: &str) -> Value {
    json!({
        "verified": true,
        "learnings": [{"type":"finding","title":title,"consequence":CONSEQUENCE,"sources":[SOURCE]}]
    })
}

#[test]
fn case_equivalent_learning_titles_conflict_before_any_record_write() {
    let fixture = Fixture::new();
    for (upper, lower) in [
        ("Cache \u{1c89} identity", "Cache \u{1c8a} identity"),
        ("Cache \u{1c90} identity", "Cache \u{10d0} identity"),
    ] {
        assert!(
            fixture.service().normalize(&payload(upper), false).is_ok(),
            "the uppercase title is accepted by the real guards"
        );
        assert!(
            fixture.service().normalize(&payload(lower), false).is_ok(),
            "the lowercase title is accepted by the real guards"
        );
        let mut value = payload(upper);
        value["learnings"]
            .as_array_mut()
            .unwrap()
            .push(payload(lower)["learnings"][0].clone());
        let error = fixture.service().normalize(&value, false).unwrap_err();
        assert_eq!(
            error.code.as_deref(),
            Some("memory-result-v1:conflicting_learnings")
        );
        assert!(fixture.brain().inventory(true).unwrap().records.is_empty());
    }
}

#[test]
fn a_migrated_unicode16_learning_is_reused_without_creating_a_duplicate() {
    let fixture = Fixture::new();
    let brain = fixture.brain();
    let task = brain
        .create(CreateRecord {
            external_id: "CASEFOLD-MIGRATION".into(),
            title: "Preserve canonical learning identities".into(),
            goal: Some("Reuse the checked learning rather than creating a second record.".into()),
            ..CreateRecord::default()
        })
        .unwrap();
    let task_id = task["id"].as_str().unwrap();
    for (index, (upper, lower, canonical_fold)) in [
        (
            "Cache \u{1c89} identity",
            "Cache \u{1c8a} identity",
            "cache \u{1c8a} identity",
        ),
        (
            "Cache \u{1c90} identity",
            "Cache \u{10d0} identity",
            "cache \u{10d0} identity",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        // Seed the exact legacy key from a literal Python/UCD16 fold. Do not
        // derive the migration oracle through the function under test.
        let legacy = format!(
            "learning-{}",
            &result_store::legacy_digest(
                &json!([task_id, "finding", canonical_fold, [SOURCE]]),
                false,
                false
            )
            .unwrap()[..24]
        );
        let original = brain
            .create(CreateRecord {
                record_type: "finding".into(),
                external_id: legacy.clone(),
                title: upper.into(),
                sources: vec![SOURCE.into()],
                goal: Some(CONSEQUENCE.into()),
                ..CreateRecord::default()
            })
            .unwrap();
        let original = brain
            .update(
                original["id"].as_str().unwrap(),
                UpdateRecord {
                    expected_revision: original["revision"].as_u64(),
                    progress: Some(CONSEQUENCE.into()),
                    authority: Some("verified".into()),
                    transition_to: Some("resolved".into()),
                    attestation: Some("person".into()),
                    actor: "local".into(),
                    reason: "Seed the canonical checked legacy finding".into(),
                    ..UpdateRecord::default()
                },
            )
            .unwrap();
        let original_id = original["id"].as_str().unwrap().to_owned();
        let path = brain.dynamic_path(&original).unwrap();
        let original_bytes = fs::read(&path).unwrap();
        for (variant, title) in [upper, lower].into_iter().enumerate() {
            let report = fixture
                .service()
                .record(RecordResult {
                    task_id: task_id.into(),
                    result_id: format!("later-{index}-{variant}"),
                    revision: Revision::Auto,
                    data: payload(title),
                    reason: "Restate checked canonical memory after migration".into(),
                    attestation: "person".into(),
                    prepared: None,
                    replay_only: false,
                })
                .unwrap();
            assert_eq!(report["records"].as_array().unwrap().len(), 1);
            assert_eq!(report["records"][0]["id"], original_id);
            assert_eq!(report["records"][0]["external_id"], legacy);
            assert_eq!(report["records"][0]["state"], "existing");
            assert_eq!(
                fs::read(&path).unwrap(),
                original_bytes,
                "a restatement does not rewrite the migrated record"
            );
            let inventory = brain.inventory(true).unwrap();
            assert!(inventory.problems.is_empty());
            assert_eq!(
                inventory
                    .records
                    .iter()
                    .filter(|image| image.record["type"] == "finding")
                    .count(),
                index + 1,
                "migration must not create a second title-derived record"
            );
        }
    }
}
