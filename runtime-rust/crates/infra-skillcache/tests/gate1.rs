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

#[test]
fn gate1_rejects_extended_inline_flags_and_package_file_example_leaks() {
    for args in [
        json!(["powershell", "-EncodedCommand", "ZABpAHIA"]),
        json!(["powershell", "-enc", "ZABpAHIA"]),
        json!(["node", "--input-type=module", "-e", "console.log(1)"]),
        json!(["perl", "-pe", "s/a/b/", "programs/extract/main.py"]),
        json!(["perl", "-n", "programs/extract/main.py"]),
    ] {
        refused("argv_inline_code", |value| {
            value["programs"][0]["entry"] = args.clone();
        });
    }

    let mut pkg = Files::new();
    for (path, content) in files().iter() {
        let replacement = if path == "programs/extract/main.py" {
            b"import sys\nprint('invoices/2026-q3/')\n".to_vec()
        } else {
            content.to_vec()
        };
        pkg.insert(path, replacement);
    }
    let report = gate(&capsule(), &pkg, Some(&examples()));
    assert!(
        report.has("example_value_in_capsule"),
        "{:?}",
        report.violations
    );
}

#[test]
fn gate1_catches_short_string_and_numeric_example_smuggling() {
    // Short (<8 chars) example binding like "EUR" in capsule description or script literal is refused.
    refused("example_value_in_capsule", |value| {
        value["mu"]["description"] = "Convert EUR invoices into a summary".into();
    });

    let mut pkg = Files::new();
    for (path, content) in files().iter() {
        let replacement = if path == "programs/summarize/main.py" {
            b"RATE = {'EUR': 1.0}\nprint('{\"status\": \"complete\", \"result\": {}}')\n".to_vec()
        } else {
            content.to_vec()
        };
        pkg.insert(path, replacement);
    }
    assert!(gate(&capsule(), &pkg, Some(&examples())).has("example_value_in_capsule"));

    // Numeric constant smuggling in observed step outputs and numeric bindings is refused.
    let mut numeric_obs = examples();
    numeric_obs["bindings"]["tax_rate"] = json!(42);
    let mut cap_with_num = capsule();
    cap_with_num["bindings"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name": "tax_rate", "type": "number", "origin": "request"}));
    cap_with_num["programs"][0]["inputs"] = json!(["input_dir", "quarter_files", "tax_rate"]);
    cap_with_num["steps"][0]["text"] = "Apply rate 42 for the quarter.".into();
    let num_report = gate(&cap_with_num, &files(), Some(&numeric_obs));
    assert!(
        num_report.has("example_value_in_capsule"),
        "{:?}",
        num_report.violations
    );

    // Numeric smuggling inside package script from examples.steps result.
    let mut step_num_obs = examples();
    step_num_obs["steps"]["1"]["result"]["invoices"] = json!(777);
    let mut num_pkg = Files::new();
    for (path, content) in files().iter() {
        let replacement = if path == "programs/extract/main.py" {
            b"COUNT = 777\nprint('{\"status\": \"complete\", \"result\": {}}')\n".to_vec()
        } else {
            content.to_vec()
        };
        num_pkg.insert(path, replacement);
    }
    let step_num_report = gate(&capsule(), &num_pkg, Some(&step_num_obs));
    assert!(
        step_num_report.has("example_value_in_capsule"),
        "{:?}",
        step_num_report.violations
    );
}

#[test]
fn gate1_tightens_shell_env_and_module_flags() {
    refused("invalid_value", |value| {
        value["capabilities"]["runtimes"] = json!(["env"]);
    });

    for (runtimes, args) in [
        (
            json!(["sh", "python3"]),
            json!(["sh", "-s", "programs/extract/main.py"]),
        ),
        (
            json!(["sh", "python3"]),
            json!(["sh", "-m", "programs/extract/main.py"]),
        ),
        (
            json!(["bash", "python3"]),
            json!(["bash", "-lc", "echo hi", "programs/extract/main.py"]),
        ),
        (
            json!(["bash", "python3"]),
            json!([
                "bash",
                "--rcfile=programs/extract/main.py",
                "programs/extract/main.py"
            ]),
        ),
        (
            json!(["python3"]),
            json!(["python3", "-m", "runpy", "programs/extract/main.py"]),
        ),
        (
            json!(["sh", "python3"]),
            json!(["sh", "FOO=bar", "programs/extract/main.py"]),
        ),
        (
            json!(["sh", "python3"]),
            json!(["sh", "env", "programs/extract/main.py"]),
        ),
    ] {
        refused("argv_inline_code", |value| {
            value["capabilities"]["runtimes"] = runtimes.clone();
            value["programs"][0]["entry"] = args.clone();
        });
    }

    // Direct sh execution of a package script with flags AFTER the script remains allowed.
    let mut ok = capsule();
    ok["capabilities"]["runtimes"] = json!(["sh", "python3"]);
    ok["programs"][0]["entry"] = json!(["sh", "programs/extract/main.py", "-s", "--mode"]);
    let report = gate(&ok, &files(), Some(&examples()));
    assert!(report.passed, "{:?}", report.violations);
}

#[test]
fn gate1_allows_parameterized_multi_call_helper_reuse() {
    let cap = json!({
        "schema_version": 1,
        "form": "hybrid",
        "kind": "tool_chain",
        "mu": {"name": "hash-pair", "description": "Read and compare two source paths"},
        "source": {"skills": []},
        "capabilities": {"network": "none", "runtimes": ["python3"]},
        "bindings": [
            {"name": "first_path", "type": "path", "origin": "request"},
            {"name": "second_path", "type": "path", "origin": "request"}
        ],
        "programs": [
            {
                "id": "hash_file",
                "entry": ["python3", "programs/hash_file/main.py"],
                "inputs": ["target_path"],
                "outputs": ["digest"],
                "check": ["python3", "programs/hash_file/main.py", "--check"],
                "timeout_seconds": 30,
                "read_only": true
            },
            {
                "id": "compare",
                "entry": ["python3", "programs/compare/main.py"],
                "inputs": ["first_digest", "second_digest"],
                "outputs": [],
                "check": ["python3", "programs/compare/main.py", "--check"],
                "timeout_seconds": 30,
                "read_only": true
            }
        ],
        "steps": [
            {
                "kind": "program",
                "program": "hash_file",
                "inputs": {"target_path": "first_path"},
                "outputs": {"digest": "first_digest"}
            },
            {
                "kind": "program",
                "program": "hash_file",
                "inputs": {"target_path": "second_path"},
                "outputs": {"digest": "second_digest"}
            },
            {"kind": "program", "program": "compare"}
        ],
        "verification": []
    });
    let mut pkg = Files::new();
    pkg.insert("programs/hash_file/main.py", b"print('ok')\n".to_vec());
    pkg.insert("programs/compare/main.py", b"print('ok')\n".to_vec());
    let report = gate(&cap, &pkg, None);
    assert!(report.passed, "{:?}", report.violations);
}
