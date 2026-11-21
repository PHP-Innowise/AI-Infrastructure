//! A warm package holds the capsule and its programs, read-only, and nothing
//! that was observed; a skill's identity changes with any of its files.
#[path = "support/scratch.rs"]
mod scratch;
mod support;
use infra_skillcache::{Examples, SkillCapsule, fingerprint, package};
use std::path::Path;
use support::{bytes, capsule, examples, files};

fn names(root: &Path) -> Vec<String> {
    fingerprint::stat_tree(root)
        .unwrap()
        .into_iter()
        .map(|file| file.path)
        .collect()
}

#[test]
fn package_warm_has_no_examples() {
    let state = scratch::scratch();
    let warm = state.path().join("capsules/project/capsule/1/warm");
    let parsed = SkillCapsule::parse(&bytes(&capsule())).unwrap();
    let observed = Examples::from_value(&examples()).unwrap();
    let digest = package::write_warm(&warm, &parsed, None, &files()).unwrap();
    let qualification = state.path().join("qualification/capsule/1/examples.json");
    package::write_examples(&qualification, &observed).unwrap();
    assert_eq!(
        names(&warm),
        [
            "assets/columns.json",
            "capsule.json",
            "programs/extract/main.py",
            "programs/summarize/main.py",
            "programs/verify_csv/main.py",
        ]
    );
    // No file of the warm tree carries an observed value, under any name.
    for file in names(&warm) {
        let text = std::fs::read_to_string(warm.join(&file)).unwrap();
        for value in [
            "invoices/2026-q3/",
            "q3-01.pdf",
            "out/rows.json",
            "18240.00",
        ] {
            assert!(!text.contains(value), "{file} repeats observed `{value}`");
        }
    }
    assert!(!qualification.starts_with(&warm));
    assert_eq!(package::digest(&warm).unwrap(), digest);
    let loaded = package::load(&warm).unwrap();
    assert_eq!(loaded.capsule, parsed);
    assert_eq!(loaded.files.len(), 4);
    assert_eq!(
        loaded.capsule_sha256,
        fingerprint::sha256(&std::fs::read(warm.join("capsule.json")).unwrap())
    );
    // A revision is written once.
    assert!(package::write_warm(&warm, &parsed, None, &files()).is_err());
    package::remove(&warm).unwrap();
    assert!(!warm.exists());
    package::remove(&warm).unwrap();
}

#[cfg(unix)]
#[test]
fn package_warm_is_read_only_and_examples_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let state = scratch::scratch();
    let warm = state.path().join("warm");
    let parsed = SkillCapsule::parse(&bytes(&capsule())).unwrap();
    package::write_warm(&warm, &parsed, None, &files()).unwrap();
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&warm), 0o555);
    assert_eq!(mode(&warm.join("programs/extract")), 0o555);
    assert_eq!(mode(&warm.join("programs/extract/main.py")), 0o444);
    assert_eq!(mode(&warm.join("capsule.json")), 0o444);
    let observed = state.path().join("qualification/examples.json");
    package::write_examples(&observed, &Examples::from_value(&examples()).unwrap()).unwrap();
    assert_eq!(mode(&observed), 0o600);
    package::remove(&warm).unwrap();
}

#[test]
fn package_load_refuses_what_a_capsule_does_not_define() {
    let state = scratch::scratch();
    let parsed = SkillCapsule::parse(&bytes(&capsule())).unwrap();
    let build = |name: &str| {
        let root = state.path().join(name);
        std::fs::create_dir_all(root.join("programs/extract")).unwrap();
        std::fs::write(root.join("capsule.json"), parsed.to_bytes()).unwrap();
        std::fs::write(root.join("programs/extract/main.py"), b"pass\n").unwrap();
        root
    };
    assert!(package::load(&build("plain")).is_ok());
    let with_examples = build("examples");
    std::fs::write(with_examples.join("examples.json"), b"{}").unwrap();
    let error = package::load(&with_examples).unwrap_err();
    assert!(error.message.contains("Observed values"), "{error}");
    let stray = build("stray");
    std::fs::write(stray.join("notes.txt"), b"x").unwrap();
    assert!(package::load(&stray).is_err());
    #[cfg(unix)]
    {
        let linked = build("linked");
        std::os::unix::fs::symlink("/etc/hosts", linked.join("programs/extract/hosts")).unwrap();
        let error = package::load(&linked).unwrap_err();
        assert!(error.message.contains("links"), "{error}");
    }
    // A textual capsule is its Markdown alone.
    let (manifest, markdown) = support::textual();
    let textual = SkillCapsule::parse(&bytes(&manifest)).unwrap();
    let root = state.path().join("textual");
    assert!(package::write_warm(&root, &textual, None, &package::Files::new()).is_err());
    assert!(package::write_warm(&root, &textual, Some(markdown.as_bytes()), &files()).is_err());
    package::write_warm(
        &root,
        &textual,
        Some(markdown.as_bytes()),
        &package::Files::new(),
    )
    .unwrap();
    assert_eq!(names(&root), ["capsule.json", "capsule.md"]);
    assert_eq!(package::load(&root).unwrap().markdown.unwrap(), markdown);
    package::remove(&root).unwrap();
}

#[test]
fn fingerprint_detects_script_change() {
    let project = scratch::scratch();
    let skill = project.path().join(".claude/skills/pdf");
    std::fs::create_dir_all(skill.join("scripts")).unwrap();
    std::fs::create_dir_all(skill.join("references")).unwrap();
    std::fs::write(skill.join("SKILL.md"), "# PDF\n").unwrap();
    std::fs::write(skill.join("scripts/extract.py"), "print(1)\n").unwrap();
    std::fs::write(skill.join("references/forms.md"), "Forms\n").unwrap();
    let fresh = fingerprint::tree_sha256(&skill).unwrap();
    assert_eq!(fresh.files.len(), 3);
    // Files written just before the listing are read again, not trusted.
    let (same, read) = fingerprint::verify_tree(&skill, &fresh).unwrap();
    assert!(same && read.is_some());
    // Settled files are trusted by size and time, without reading them.
    let before = fingerprint::Tree {
        taken_ns: fresh.taken_ns + 10_000_000_000,
        ..fresh
    };
    let (same, read) = fingerprint::verify_tree(&skill, &before).unwrap();
    assert!(same && read.is_none());
    // The entry document is untouched; a script the skill runs is not.
    std::fs::write(skill.join("scripts/extract.py"), "print(2)\n").unwrap();
    let (same, read) = fingerprint::verify_tree(&skill, &before).unwrap();
    assert!(!same);
    assert_ne!(read.unwrap().sha256, before.sha256);
    // The same bytes with a new time are read again and found unchanged.
    std::fs::write(skill.join("scripts/extract.py"), "print(1)\n").unwrap();
    let (same, read) = fingerprint::verify_tree(&skill, &before).unwrap();
    assert!(same);
    assert_eq!(read.unwrap().sha256, before.sha256);
    // A new file, a removed one and a rename each change the identity.
    std::fs::write(skill.join("references/extra.md"), "x").unwrap();
    assert_ne!(
        fingerprint::tree_sha256(&skill).unwrap().sha256,
        before.sha256
    );
    std::fs::remove_file(skill.join("references/extra.md")).unwrap();
    assert_eq!(
        fingerprint::tree_sha256(&skill).unwrap().sha256,
        before.sha256
    );
    std::fs::rename(
        skill.join("references/forms.md"),
        skill.join("references/form.md"),
    )
    .unwrap();
    assert_ne!(
        fingerprint::tree_sha256(&skill).unwrap().sha256,
        before.sha256
    );
}

#[test]
fn the_environment_identity_names_the_contract_and_each_runtime() {
    use std::collections::BTreeMap;
    let runtimes = |version: Option<&str>| {
        BTreeMap::from([("python3".to_string(), version.map(String::from))])
    };
    let base = fingerprint::env_fingerprint(&runtimes(Some("Python 3.13.1")));
    assert_eq!(base.len(), 64);
    assert_eq!(
        base,
        fingerprint::env_fingerprint_for(
            fingerprint::ENV_ABI_VERSION,
            infra_skillcache::SCHEMA_VERSION,
            &infra_skillcache::compile_rules::version(),
            &runtimes(Some("Python 3.13.1")),
        )
    );
    // Unrelated engine rebuilds do not change the environment identity; changes
    // to the runtime contract, schema, compiler rules or interpreters do.
    assert_ne!(
        base,
        fingerprint::env_fingerprint_for(
            fingerprint::ENV_ABI_VERSION + 1,
            infra_skillcache::SCHEMA_VERSION,
            &infra_skillcache::compile_rules::version(),
            &runtimes(Some("Python 3.13.1")),
        )
    );
    assert_ne!(
        base,
        fingerprint::env_fingerprint_for(
            fingerprint::ENV_ABI_VERSION,
            infra_skillcache::SCHEMA_VERSION + 1,
            &infra_skillcache::compile_rules::version(),
            &runtimes(Some("Python 3.13.1")),
        )
    );
    assert_ne!(
        base,
        fingerprint::env_fingerprint_for(
            fingerprint::ENV_ABI_VERSION,
            infra_skillcache::SCHEMA_VERSION,
            "1-other",
            &runtimes(Some("Python 3.13.1")),
        )
    );
    assert_ne!(
        base,
        fingerprint::env_fingerprint(&runtimes(Some("Python 3.14.0")))
    );
    assert_ne!(base, fingerprint::env_fingerprint(&runtimes(None)));
    assert_eq!(fingerprint::runtime_version("no-such-runtime-here"), None);
    assert_eq!(fingerprint::runtime_version("../bin/sh"), None);
}

#[test]
fn the_compile_rules_carry_a_version_and_a_schema() {
    use infra_skillcache::compile_rules;
    let version = compile_rules::version();
    assert!(
        version.starts_with("1-") && version.len() == 14,
        "{version}"
    );
    let schema = compile_rules::response_schema();
    assert_eq!(schema["required"], serde_json::json!(["form", "capsule"]));
    for rule in [
        "Path pruning",
        "Binding lifting",
        "Region fusion",
        "Grounded postconditions",
    ] {
        assert!(compile_rules::INSTRUCTIONS.contains(rule), "{rule}");
    }
    assert!(compile_rules::TEXTUAL_TEMPLATE.contains("## Steps"));
}

#[test]
fn a_folder_that_held_a_sealed_package_can_be_given_back() {
    // A plain temporary folder on purpose: this is the case `release` is for.
    let state = tempfile::tempdir().unwrap();
    let held = state.path().join("capsules");
    let warm = held.join("project/capsule/1/warm");
    let parsed = SkillCapsule::parse(&bytes(&capsule())).unwrap();
    package::write_warm(&warm, &parsed, None, &files()).unwrap();
    // Sealed, the tree cannot be removed as it is. An administrator is not
    // held back by a mode, so the refusal is checked only where it applies.
    let probe = warm.join("probe");
    if std::fs::write(&probe, b"x").is_err() {
        assert!(std::fs::remove_dir_all(&held).is_err());
        assert!(warm.join("capsule.json").is_file(), "nothing was removed");
    } else {
        std::fs::remove_file(&probe).unwrap();
    }
    package::release(state.path());
    std::fs::remove_dir_all(&held).unwrap();
    assert!(!held.exists());
    // Releasing what is not there, or what is already free, changes nothing.
    package::release(&held);
    package::release(state.path());
}
