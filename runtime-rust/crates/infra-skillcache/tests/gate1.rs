//! Gate 1: every structural and dataflow rule of a capsule refuses its own
//! violation with its own code, and the reference capsule passes all of them.
mod support;
use infra_skillcache::gate1::{self, Context, Report};
use infra_skillcache::package::Files;
use serde_json::{Value, json};
use support::{bytes, capsule, examples, files};

fn gate(capsule: &Value, files: &Files, examples: Option<&Value>) -> Report {
    let examples = examples.map(bytes);
    gate1::check(
        &bytes(capsule),
        files,
        examples.as_deref(),
        &Context::default(),
    )
}
/// The codes raised when `change` is applied to the reference capsule.
fn codes(change: impl FnOnce(&mut Value)) -> Vec<&'static str> {
    let mut value = capsule();
    change(&mut value);
    gate(&value, &files(), Some(&examples()))
        .violations
        .into_iter()
        .map(|violation| violation.code)
        .collect()
}
#[track_caller]
fn refused(code: &str, change: impl FnOnce(&mut Value)) {
    let found = codes(change);
    assert!(
        found.contains(&code),
        "expected `{code}`, the gate reported {found:?}"
    );
}

#[test]
fn gate1_accepts_the_reference_capsule() {
    let report = gate(&capsule(), &files(), Some(&examples()));
    assert!(report.passed, "{:?}", report.violations);
    assert_eq!(report.gate, "gate1");
    // The examples are optional for the structural rules alone.
    assert!(gate(&capsule(), &files(), None).passed);
}

#[test]
fn gate1_rejects_json_that_is_not_strict() {
    let text = r#"{"schema_version":1,"schema_version":1,"form":"hybrid"}"#;
    let report = gate1::check(text.as_bytes(), &files(), None, &Context::default());
    assert!(report.has("invalid_json"), "{:?}", report.violations);
    let huge = format!("{{\"pad\":\"{}\"}}", "x".repeat(300 * 1024));
    let report = gate1::check(huge.as_bytes(), &files(), None, &Context::default());
    assert!(report.has("size_limit"), "{:?}", report.violations);
}

#[test]
fn gate1_rejects_unknown_and_missing_fields() {
    refused("unknown_field", |value| value["owner"] = "me".into());
    refused("unknown_field", |value| {
        value["programs"][0]["shell"] = true.into()
    });
    refused("unknown_field", |value| {
        value["steps"][1]["note"] = "x".into()
    });
    refused("missing_field", |value| {
        value.as_object_mut().unwrap().remove("mu");
    });
    refused("missing_field", |value| {
        value["programs"][0]
            .as_object_mut()
            .unwrap()
            .remove("check");
    });
    refused("missing_field", |value| {
        value["programs"][0]
            .as_object_mut()
            .unwrap()
            .remove("read_only");
    });
    refused("schema_version_unsupported", |value| {
        value["schema_version"] = 2.into()
    });
}

#[test]
fn gate1_rejects_values_outside_their_grammar() {
    refused("invalid_value", |value| {
        value["mu"]["name"] = "PDF Totals".into()
    });
    refused("invalid_value", |value| {
        value["bindings"][0]["name"] = "Input-Dir".into()
    });
    refused("invalid_value", |value| {
        value["bindings"][0]["type"] = "file".into()
    });
    refused("invalid_value", |value| {
        value["bindings"][0]["origin"] = "guess".into()
    });
    refused("invalid_value", |value| {
        value["capabilities"]["network"] = "maybe".into()
    });
    refused("invalid_value", |value| {
        value["capabilities"]["runtimes"] = json!(["/usr/bin/python3"])
    });
    refused("invalid_value", |value| {
        value["programs"][0]["timeout_seconds"] = 0.into()
    });
    refused("invalid_value", |value| {
        value["source"]["skills"][0]["path"] = "../outside/SKILL.md".into()
    });
    refused("invalid_value", |value| {
        value["source"]["skills"][0]["tree_sha256"] = "abc".into()
    });
    refused("size_limit", |value| {
        value["steps"][0]["text"] = "x".repeat(5000).into()
    });
}

#[test]
fn gate1_rejects_examples_inside_the_capsule() {
    refused("examples_in_capsule", |value| {
        value["examples"] = examples()
    });
    // The same value repeated in a description is the instance, not the parameter.
    refused("example_value_in_capsule", |value| {
        value["mu"]["description"] = "Summarise invoices/2026-q3/ into a CSV".into()
    });
    // A multiline observed value matches even when JSON-escaped in the capsule.
    let mut multiline = examples();
    multiline["bindings"]["currency"] = "first line\nsecond line".into();
    let mut with_nl = capsule();
    with_nl["mu"]["description"] = "Handle first line\nsecond line".into();
    let report = gate(&with_nl, &files(), Some(&multiline));
    assert!(
        report.has("example_value_in_capsule"),
        "{:?}",
        report.violations
    );
}

#[test]
fn gate1_rejects_duplicate_and_unknown_programs() {
    refused("duplicate_id", |value| {
        let copy = value["programs"][0].clone();
        value["programs"].as_array_mut().unwrap().push(copy);
    });
    refused("unknown_program", |value| {
        value["steps"][1]["program"] = "missing".into()
    });
    refused("unknown_program", |value| {
        value["verification"][0]["program"] = "missing".into()
    });
    refused("program_referenced_twice", |value| {
        value["steps"]
            .as_array_mut()
            .unwrap()
            .push(json!({"kind": "program", "program": "extract"}));
    });
    refused("program_unreferenced", |value| {
        value["steps"].as_array_mut().unwrap().remove(2);
    });
}

#[test]
fn gate1_rejects_a_dataflow_that_is_not_a_dag() {
    // An input nothing defines.
    refused("dangling_input", |value| {
        value["programs"][0]["inputs"] = json!(["input_dir", "nowhere"])
    });
    // An input only a later step defines: the order is the plan's.
    refused("dangling_input", |value| {
        let steps = value["steps"].as_array_mut().unwrap();
        steps.swap(1, 2);
    });
    // A value produced after the program that needs it.
    refused("dangling_input", |value| {
        let steps = value["steps"].as_array_mut().unwrap();
        steps.swap(0, 1);
    });
    refused("name_redefined", |value| {
        value["programs"][1]["outputs"] = json!(["summary_file", "quarter_files"])
    });
    refused("name_redefined", |value| {
        value["steps"][0]["produces"] = json!(["quarter_files", "currency"])
    });
}

#[test]
fn gate1_rejects_outputs_that_are_not_for_later_programs() {
    refused("output_overlaps_input", |value| {
        value["programs"][0]["outputs"] = json!(["rows_file", "input_dir"])
    });
    // An answer for the agent declared as an output: nothing reads it.
    refused("output_unread", |value| {
        value["programs"][1]["outputs"] = json!(["summary_file", "total"])
    });
    refused("binding_unused", |value| {
        value["bindings"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "locale", "type": "string", "origin": "request"}));
    });
    refused("produces_unread", |value| {
        value["steps"][0]["produces"] = json!(["quarter_files", "notes"])
    });
}

#[test]
fn gate1_rejects_verification_that_is_not_read_only_or_misplaced() {
    refused("verification_not_read_only", |value| {
        value["programs"][2]["read_only"] = false.into()
    });
    refused("invalid_step_kind", |value| {
        value["verification"][1]["kind"] = "instruction".into()
    });
    refused("invalid_step_kind", |value| {
        value["steps"][0] = json!({"kind": "inspection", "text": "Look at it."})
    });
    refused("empty_plan", |value| {
        value["steps"] = json!([]);
        value["verification"] = json!([]);
    });
    refused("hybrid_without_programs", |value| {
        value["programs"] = json!([]);
        value["steps"] = json!([{"kind": "instruction", "text": "Do it by hand."}]);
        value["verification"] = json!([]);
        value["bindings"] = json!([]);
    });
}

#[test]
fn gate1_rejects_commands_that_are_not_package_files() {
    refused("argv_empty", |value| {
        value["programs"][0]["entry"] = json!([])
    });
    refused("runtime_undeclared", |value| {
        value["programs"][0]["entry"] = json!(["node", "programs/extract/main.py"])
    });
    refused("argv_inline_code", |value| {
        value["programs"][0]["entry"] = json!(["python3", "-c", "print(1)"])
    });
    refused("argv_absolute_path", |value| {
        value["programs"][0]["entry"] = json!(["python3", "/tmp/main.py"])
    });
    refused("argv_absolute_path", |value| {
        value["programs"][0]["entry"] =
            json!(["python3", "programs/extract/main.py", "--out=C:\\temp\\x"])
    });
    refused("argv_parent_traversal", |value| {
        value["programs"][0]["entry"] = json!(["python3", "programs/../../main.py"])
    });
    refused("argv_parent_traversal", |value| {
        value["programs"][0]["entry"] =
            json!(["python3", "programs/extract/main.py", "-I../secret"])
    });
    refused("argv_missing_file", |value| {
        value["programs"][0]["entry"] = json!(["python3", "./programs/extract/main.py"])
    });
    refused("argv_missing_file", |value| {
        value["programs"][0]["check"] = json!(["python3", "programs/extract/absent.py"])
    });
    refused("argv_windows_script", |value| {
        value["programs"][0]["entry"] = json!(["python3", "programs/extract/run.cmd"])
    });
    refused("argv_no_program_file", |value| {
        value["programs"][0]["entry"] = json!(["python3", "assets/columns.json"])
    });
    refused("argv_no_program_file", |value| {
        value["programs"][0]["entry"] = json!(["python3"])
    });
    refused("source_outside_skill", |value| {
        value["steps"][0]["source"]["path"] = "docs/OTHER.md".into()
    });
}

#[test]
fn gate1_rejects_package_files_outside_their_folders() {
    let mut package = files();
    package.insert("capsule.json", b"{}".to_vec());
    package.insert("programs/../escape.py", b"x".to_vec());
    package.insert("programs/extract/.hidden", b"x".to_vec());
    package.insert("programs/Extract/main.py", b"x".to_vec());
    package.insert("programs/big.py", vec![b'x'; 600 * 1024]);
    package.insert("programs/run.bat", b"x".to_vec());
    let report = gate(&capsule(), &package, None);
    let found = report
        .violations
        .iter()
        .map(|violation| (violation.code, violation.at.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        found,
        [
            ("invalid_file_path", "files.capsule.json"),
            ("invalid_file_path", "files.programs/../escape.py"),
            ("invalid_file_path", "files.programs/extract/.hidden"),
            ("file_path_collision", "files.programs/Extract/main.py"),
            ("size_limit", "files.programs/big.py"),
            ("argv_windows_script", "files.programs/run.bat"),
        ]
    );
}

#[test]
fn gate1_rejects_bindings_that_are_file_content() {
    let invoice = "Invoice 17\nCustomer: Example GmbH\nTotal: 120.00\nDue: 30 days\n";
    let mut observed = examples();
    observed["bindings"]["currency"] = invoice.into();
    let report = gate1::check(
        &bytes(&capsule()),
        &files(),
        Some(&bytes(&observed)),
        &Context {
            observed_files: &[format!("{invoice}\n")],
        },
    );
    assert!(
        report.has("file_content_binding"),
        "{:?}",
        report.violations
    );
    // A few lines out of a file the run read are file content as well.
    let mut observed = examples();
    observed["bindings"]["currency"] = "Customer: Example GmbH\nTotal: 120.00\nDue: 30 days".into();
    let report = gate1::check(
        &bytes(&capsule()),
        &files(),
        Some(&bytes(&observed)),
        &Context {
            observed_files: &[invoice.to_string()],
        },
    );
    assert!(
        report.has("file_content_binding"),
        "{:?}",
        report.violations
    );
    // A path that merely occurs inside a file the run read stays a binding.
    let report = gate1::check(
        &bytes(&capsule()),
        &files(),
        Some(&bytes(&examples())),
        &Context {
            observed_files: &["see invoices/2026-q3/ and EUR".to_string()],
        },
    );
    assert!(report.passed, "{:?}", report.violations);
}

#[test]
fn gate1_rejects_examples_that_do_not_fit_the_capsule() {
    let with = |change: &dyn Fn(&mut Value)| {
        let mut observed = examples();
        change(&mut observed);
        gate(&capsule(), &files(), Some(&observed))
    };
    let report = with(&|observed| observed["bindings"]["currency"] = "x".repeat(5000).into());
    assert!(report.has("binding_too_large"), "{:?}", report.violations);
    let report = with(&|observed| observed["bindings"]["currency"] = 7.into());
    assert!(
        report.has("binding_type_mismatch"),
        "{:?}",
        report.violations
    );
    let report = with(&|observed| {
        observed["bindings"]
            .as_object_mut()
            .unwrap()
            .remove("input_dir");
    });
    assert!(report.has("example_missing"), "{:?}", report.violations);
    let report = with(&|observed| {
        observed["bindings"]
            .as_object_mut()
            .unwrap()
            .remove("quarter_files");
    });
    assert!(report.has("example_missing"), "{:?}", report.violations);
    let report = with(&|observed| observed["bindings"]["stray"] = "x".into());
    assert!(
        report.has("example_unknown_name"),
        "{:?}",
        report.violations
    );
    let report = with(&|observed| observed["steps"]["0"] = json!({"result": {}}));
    assert!(
        report.has("example_unknown_step"),
        "{:?}",
        report.violations
    );
    let report = with(&|observed| observed["steps"]["1"] = json!({"result": {"invoices": 2}}));
    assert!(
        report.has("example_missing_output"),
        "{:?}",
        report.violations
    );
    let report = with(&|observed| observed["steps"]["1"] = json!({"result": {"rows_file": null}}));
    assert!(
        report.has("example_missing_output"),
        "{:?}",
        report.violations
    );
    let report =
        with(&|observed| observed["steps"]["1"] = json!({"result": {"rows_file": {"a": 1}}}));
    assert!(report.has("invalid_type"), "{:?}", report.violations);
    let report = with(&|observed| {
        observed["steps"]["1"] = json!({"result": {"rows_file": "x".repeat(5000)}});
    });
    assert!(report.has("binding_too_large"), "{:?}", report.violations);
    let report = with(&|observed| observed["bindings"]["quarter_files"] = json!([["nested"]]));
    assert!(report.has("invalid_type"), "{:?}", report.violations);
    let report = with(&|observed| {
        observed["steps"].as_object_mut().unwrap().remove("2");
    });
    assert!(report.has("example_missing"), "{:?}", report.violations);
    let report = with(&|observed| observed["extra"] = 1.into());
    assert!(report.has("unknown_field"), "{:?}", report.violations);
}

#[test]
fn gate1_rejects_secrets_in_the_capsule_the_examples_and_the_programs() {
    // Assembled here so that no credential-shaped literal is committed.
    let secret = format!(
        "api_key = \"{}\"",
        ["sk", "live", &"9fQ2xLm7".repeat(4)].join("_")
    );
    assert!(
        infra_core::domain::secret_label(&secret).unwrap().is_some(),
        "the fixture must look like a credential to the shared policy"
    );
    let found = codes(|value| value["steps"][0]["text"] = secret.clone().into());
    assert!(found.contains(&"secret_detected"), "{found:?}");
    let mut observed = examples();
    observed["bindings"]["currency"] = secret.clone().into();
    let report = gate(&capsule(), &files(), Some(&observed));
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.code == "secret_detected" && violation.at == "examples"),
        "{:?}",
        report.violations
    );
    let mut package = files();
    package.insert("programs/leak.py", secret.clone().into_bytes());
    let report = gate(&capsule(), &package, None);
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.code == "secret_detected"
                && violation.at == "programs/leak.py"),
        "{:?}",
        report.violations
    );
    // A report names the kind of secret and never repeats the value.
    assert!(
        !serde_json::to_string(&report.violations)
            .unwrap()
            .contains("9fQ2xLm7")
    );
}

#[test]
fn gate1_reports_every_violation_in_one_pass() {
    let found = codes(|value| {
        value["programs"][2]["read_only"] = false.into();
        value["programs"][0]["inputs"] = json!(["input_dir", "nowhere"]);
        value["programs"][1]["entry"] = json!(["python3", "-c", "print(1)"]);
        value["steps"][0]["source"]["path"] = "docs/OTHER.md".into();
    });
    for code in [
        "verification_not_read_only",
        "dangling_input",
        "argv_inline_code",
        "source_outside_skill",
        "argv_no_program_file",
    ] {
        assert!(found.contains(&code), "missing `{code}` in {found:?}");
    }
}

#[test]
fn gate1t_checks_the_textual_form() {
    let (manifest, markdown) = support::textual();
    let report = gate1::check_textual(&bytes(&manifest), markdown.as_bytes(), None);
    assert!(report.passed, "{:?}", report.violations);
    assert_eq!(report.gate, "gate1t");
    let codes = |manifest: &Value, markdown: &str| {
        gate1::check_textual(&bytes(manifest), markdown.as_bytes(), None)
            .violations
            .into_iter()
            .map(|violation| violation.code)
            .collect::<Vec<_>>()
    };
    // The textual form has no programs: their fields are unknown to it.
    let mut with_programs = manifest.clone();
    with_programs["programs"] = json!([]);
    assert!(codes(&with_programs, &markdown).contains(&"unknown_field"));
    assert!(codes(&capsule(), &markdown).contains(&"invalid_value"));
    assert!(codes(&manifest, "Just do it.\n").contains(&"invalid_markdown"));
    assert!(codes(&manifest, "# Title\n\nNo steps here.\n").contains(&"invalid_markdown"));
    assert!(codes(&manifest, "# Title\n\n1. Run\n\n```sh\nls\n").contains(&"invalid_markdown"));
    assert!(
        codes(&manifest, &format!("# T\n\n1. x\n\n{}", "y".repeat(30_000))).contains(&"size_limit")
    );
    let secret = format!(
        "api_key = \"{}\"",
        ["sk", "live", &"9fQ2xLm7".repeat(4)].join("_")
    );
    assert!(codes(&manifest, &format!("# T\n\n1. Use {secret}\n")).contains(&"secret_detected"));
    // Observed values of the run it was compiled from must not be repeated.
    let observed = infra_skillcache::Examples::from_value(&examples()).unwrap();
    let report = gate1::check_textual(
        &bytes(&manifest),
        b"# T\n\n1. Read invoices/2026-q3/ and sum.\n",
        Some(&observed),
    );
    assert!(
        report.has("example_value_in_capsule"),
        "{:?}",
        report.violations
    );
    // An element of an array binding is checked in Gate 1T as well.
    let mut array_observed = observed.clone();
    array_observed.bindings.insert(
        "quarter_files".into(),
        json!(["q3-invoice-01.pdf", "q3-invoice-02.pdf"]),
    );
    let report = gate1::check_textual(
        &bytes(&manifest),
        b"# T\n\n1. Open q3-invoice-01.pdf.\n",
        Some(&array_observed),
    );
    assert!(
        report.has("example_value_in_capsule"),
        "{:?}",
        report.violations
    );
}

#[test]
fn a_capsule_is_stored_in_one_spelling() {
    let parsed = infra_skillcache::SkillCapsule::parse(&bytes(&capsule())).unwrap();
    let stored = parsed.to_bytes();
    assert_eq!(
        infra_skillcache::SkillCapsule::parse(&stored).unwrap(),
        parsed
    );
    assert_eq!(parsed.plan_len(), 5);
    assert!(!parsed.is_verification(2) && parsed.is_verification(3));
    assert_eq!(parsed.outputs_read_later(1), ["rows_file"]);
    assert_eq!(parsed.outputs_read_later(3), Vec::<String>::new());
    let observed = infra_skillcache::Examples::from_value(&examples()).unwrap();
    assert_eq!(
        infra_skillcache::Examples::parse(&observed.to_bytes()).unwrap(),
        observed
    );
}
