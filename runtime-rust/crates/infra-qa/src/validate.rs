use crate::{common::*, evidence, ledger, manifest, ooxml};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
pub fn ledger(root: &Path, evidence_root: &Path, v: &Value, release: bool) -> Result<Vec<String>> {
    let mut errors = schema_errors(
        v,
        &root.join("schemas/qa/disposition-ledger.schema.json"),
        "disposition ledger",
    )?;
    let rows = v["records"].as_array().context("ledger records")?;
    let runs = rows
        .iter()
        .map(|r| text(&r["run_id"]))
        .collect::<BTreeSet<_>>();
    if rows.len() != 75 || runs != all_runs() {
        errors.push("ledger requires exactly authoritative75 runs".into());
    }
    for fields in [vec!["run_id"], vec!["identity"]] {
        if assert_unique(rows, &fields).is_err() {
            errors.push(format!("duplicate {}", fields.join("+")));
        }
    }
    let mut counts = BTreeMap::<String, usize>::new();
    for r in rows {
        *counts.entry(text(&r["historical_result"])).or_default() += 1;
    }
    let expected = [
        ("Pass".into(), 52),
        ("Partial".into(), 15),
        ("Fail".into(), 2),
        ("Blocked".into(), 6),
    ]
    .into_iter()
    .collect::<BTreeMap<_, _>>();
    if counts != expected {
        errors.push("historical result arithmetic differs".into());
    }
    for r in rows {
        let run = text(&r["run_id"]);
        let original = text(&r["original_defect_id"]);
        let namespace = text(&r["namespaced_defect_id"]);
        if run.starts_with("RUN-20260803-")
            && !original.is_empty()
            && defect(&run, &text(&r["test_case_id"]), &original) != Some(namespace.as_str())
        {
            errors.push(format!("{run}: incorrect defect namespace"));
        }
        if let Some(path) = r["evidence_path"].as_str()
            && let Some(expected) = r["evidence_sha256"].as_str()
        {
            let base = if r["source_group"] == "historical-57" {
                evidence_root
            } else {
                root
            };
            match confined(base, path) {
                Ok(p) if regular(&p) => {
                    if hash_file(&p)? != expected {
                        errors.push(format!("{run}: evidence hash differs"));
                    }
                }
                _ => {
                    if r["evidence_status"] != "withheld-sensitive" {
                        errors.push(format!("{run}: evidence missing or unsafe"));
                    }
                }
            }
        }
    }
    if release {
        errors.extend(ledger::release_errors(rows));
        if v["release_ready"] != true {
            errors.push("ledger release_ready must be true".into());
        }
    }
    Ok(errors)
}
pub fn run_evidence(
    root: &Path,
    path: &Path,
    expected_run: &str,
    case: &Value,
) -> Result<Vec<String>> {
    run_evidence_selected(root, path, expected_run, None, Some(case))
}
pub fn run_evidence_with_catalog(
    root: &Path,
    path: &Path,
    expected_run: &str,
    selected: &crate::catalog::Snapshot,
) -> Result<Vec<String>> {
    run_evidence_selected(root, path, expected_run, Some(selected), None)
}
fn run_evidence_selected(
    root: &Path,
    path: &Path,
    expected_run: &str,
    selected: Option<&crate::catalog::Snapshot>,
    supplied_case: Option<&Value>,
) -> Result<Vec<String>> {
    let required = [
        "run.json",
        "case.json",
        "environment.json",
        "commands.json",
        "exit-codes.json",
        "assertions.json",
        "git-before.txt",
        "git-after.txt",
        "inventory.json",
        "provenance.json",
        "SHA256SUMS.txt",
    ];
    let mut errors = vec![];
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Ok(vec![format!("{expected_run}: evidence directory symlink")]);
    }
    for name in required {
        if !regular(&path.join(name)) {
            errors.push(format!("{expected_run}: missing/nonregular/symlink {name}"));
        }
    }
    if !errors.is_empty() {
        return Ok(errors);
    }
    let run = read_json(&path.join("run.json"))?;
    let version = schema_version(&run)?;
    let automatic;
    let selected = if let Some(selected) = selected.filter(|selected| selected.version == version) {
        selected
    } else {
        automatic = crate::catalog::for_version(root, version)?;
        &automatic
    };
    let case = if version == LEGACY_VERSION && selected.source == "qa/tc-ai/cases.v1.json" {
        selected.case(expected_run)?
    } else {
        supplied_case.unwrap_or(selected.case(expected_run)?)
    };
    errors.extend(schema_errors(
        &run,
        &root.join("schemas/qa/run-evidence.schema.json"),
        expected_run,
    )?);
    for (field, want) in [
        ("run_id", json!(expected_run)),
        ("test_case_id", case["test_case_id"].clone()),
        ("classification", case["classification"].clone()),
    ] {
        if run[field] != want {
            errors.push(format!("{expected_run}: {field} mismatch"));
        }
    }
    if read_json(&path.join("case.json"))? != *case {
        errors.push(format!("{expected_run}: case differs from catalog"));
    }
    let items = run["assertions"]["items"]
        .as_array()
        .context("assertion items")?;
    let summary = evidence::summary(items);
    if summary != run["assertions"]
        || evidence::status(&summary) != run["status"].as_str().unwrap_or("")
    {
        errors.push(format!(
            "{expected_run}: assertion/status arithmetic mismatch"
        ));
    }
    let commands = run["commands"].as_array().context("run commands")?;
    if read_json(&path.join("commands.json"))? != run["commands"] {
        errors.push("commands document mismatch".into());
    }
    if read_json(&path.join("assertions.json"))? != run["assertions"] {
        errors.push("assertions document mismatch".into());
    }
    let codes = commands
        .iter()
        .map(|c| (text(&c["id"]), c["exit_code"].clone()))
        .collect::<serde_json::Map<_, _>>();
    if read_json(&path.join("exit-codes.json"))? != Value::Object(codes) {
        errors.push("exit-codes document mismatch".into());
    }
    if assert_unique(items, &["id"]).is_err() || assert_unique(commands, &["id"]).is_err() {
        errors.push("duplicate command/assertion IDs".into());
    }
    let assertions = items
        .iter()
        .map(|v| (text(&v["id"]), v))
        .collect::<BTreeMap<_, _>>();
    let mut expected_files = [
        "run.json",
        "case.json",
        "environment.json",
        "commands.json",
        "exit-codes.json",
        "assertions.json",
        "git-before.txt",
        "git-after.txt",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    for c in commands {
        let id = text(&c["id"]);
        let code = c["exit_code"].as_i64();
        if code.is_none() {
            errors.push(format!("command {id}: missing exit code"));
        }
        if !assertions
            .get(&id)
            .is_some_and(|a| a["status"] == if code == Some(0) { "passed" } else { "failed" })
        {
            errors.push(format!("command {id}: assertion disagrees with exit code"));
        }
        for field in ["stdout_path", "stderr_path"] {
            let rel = c[field].as_str().unwrap_or("");
            expected_files.insert(rel.into());
            if !confined(path, rel).is_ok_and(|p| regular(&p)) {
                errors.push(format!("command {id}: unsafe or missing stream"));
            }
        }
    }
    let inventory = read_json(&path.join("inventory.json"))?;
    let files = inventory["files"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    if !expected_files.is_subset(&files) || !inventory["repository_status_changed"].is_boolean() {
        errors.push("inventory incomplete".into());
    }
    let before = fs::read(path.join("git-before.txt"))?;
    let after = fs::read(path.join("git-after.txt"))?;
    if inventory["repository_status_changed"] != json!(before != after) {
        errors.push("repository status change flag mismatch".into());
    }
    let environment = read_json(&path.join("environment.json"))?;
    let fields = if version == LEGACY_VERSION {
        vec![
            "branch",
            "captured_at",
            "executable",
            "platform",
            "python",
            "repository_commit",
            "repository_root",
        ]
    } else {
        vec![
            "branch",
            "captured_at",
            "executable",
            "platform",
            "runtime",
            "runtime_version",
            "artifact_contract",
            "repository_commit",
            "repository_root",
        ]
    };
    if fields
        .iter()
        .any(|k| environment[*k].as_str().is_none_or(str::is_empty))
    {
        errors.push("environment incomplete".into());
    }
    if version == NATIVE_VERSION
        && (environment["runtime"] != "infra-qa-rust"
            || environment["artifact_contract"] != NATIVE_VERSION
            || environment["executable"] != "{infra}")
    {
        errors.push("native runtime metadata mismatch".into());
    }
    let provenance = read_json(&path.join("provenance.json"))?;
    let runner = if version == LEGACY_VERSION {
        "scripts/qa/run_tc_ai.py"
    } else {
        "infra qa run"
    };
    let catalog_source = if version == LEGACY_VERSION {
        crate::catalog::LEGACY_PATH
    } else {
        selected.source.as_str()
    };
    if provenance["catalog"] != catalog_source
        || provenance["runner"] != runner
        || provenance["historical_claim_used_as_pass_evidence"] != false
        || provenance["repository_commit"]
            .as_str()
            .is_none_or(str::is_empty)
        || provenance["generated_at"]
            .as_str()
            .is_none_or(str::is_empty)
    {
        errors.push("provenance incomplete".into());
    }
    // New native evidence binds the exact selected raw catalog. Earlier v2
    // artifacts without this additive field remain readable when the case matches.
    if version == NATIVE_VERSION
        && !provenance["catalog_sha256"].is_null()
        && provenance["catalog_sha256"] != selected.sha256
    {
        errors.push("catalog source checksum differs".into());
    }
    if run["native"]["required"] == true && run["native"]["status"] == "passed" {
        errors.push("deterministic runner may not mark native evidence passed".into());
    }
    errors.extend(manifest::verify(path, &path.join("SHA256SUMS.txt"), true)?);
    Ok(errors)
}
pub fn run(root: &Path, args: &[String]) -> Result<i32> {
    let (o, p) = parse_args(
        args,
        &[
            "--ledger",
            "--workbook",
            "--run-evidence",
            "--evidence-root",
            "--cases",
        ],
        &[
            "--release",
            "--skip-ledger",
            "--skip-workbook",
            "--skip-run-evidence",
        ],
    )?;
    if !p.is_empty() {
        anyhow::bail!("unexpected positional argument");
    }
    let mut errors = vec![];
    for n in [
        "disposition-ledger.schema.json",
        "run-evidence.schema.json",
        "tc-ai-case.schema.json",
    ] {
        let s = read_json(&root.join("schemas/qa").join(n))?;
        if s["$schema"] != "https://json-schema.org/draft/2020-12/schema"
            || s["additionalProperties"] != false
            || s["required"].as_array().is_none_or(Vec::is_empty)
        {
            errors.push(format!("{n}: schema strictness differs"));
        }
        if jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&s)
            .is_err()
        {
            errors.push(format!("{n}: schema invalid"));
        }
    }
    let catalog_path = PathBuf::from(option(
        &o,
        "--cases",
        &crate::catalog::native_path(root).to_string_lossy(),
    ));
    let selected_catalog = crate::catalog::Snapshot::read(root, &catalog_path)?;
    errors.extend(schema_errors(
        &selected_catalog.document,
        &root.join("schemas/qa/tc-ai-case.schema.json"),
        "TC-AI catalog",
    )?);
    let cases = &selected_catalog.cases;
    let release = o.contains_key("--release");
    let evidence_root = PathBuf::from(option(
        &o,
        "--evidence-root",
        &root
            .join("docs/Accelerator-TestEvidence-Submission")
            .to_string_lossy(),
    ));
    if !o.contains_key("--skip-ledger") {
        let path = PathBuf::from(option(
            &o,
            "--ledger",
            &root
                .join("docs/qa/75-run-disposition.json")
                .to_string_lossy(),
        ));
        errors.extend(ledger(root, &evidence_root, &read_json(&path)?, release)?);
    }
    if !o.contains_key("--skip-workbook") {
        let path = PathBuf::from(option(
            &o,
            "--workbook",
            &root
                .join("Accelerator TestCases.reconstructed.xlsx")
                .to_string_lossy(),
        ));
        errors.extend(ooxml::validate(&ooxml::Package::read(&path)?)?);
    }
    if !o.contains_key("--skip-run-evidence") {
        let path = PathBuf::from(option(
            &o,
            "--run-evidence",
            &root.join(crate::catalog::NATIVE_EVIDENCE).to_string_lossy(),
        ));
        let mut existing = BTreeSet::new();
        if path.is_dir() {
            for e in fs::read_dir(&path)? {
                let e = e?;
                if e.file_type()?.is_dir() {
                    existing.insert(e.file_name().to_string_lossy().into_owned());
                }
            }
        }
        if release && existing != codex().into_iter().collect() {
            errors.push("release evidence requires exactly18 run directories".into());
        }
        for run in &existing {
            if cases.iter().any(|case| case["run_id"] == run.as_str()) {
                errors.extend(run_evidence_with_catalog(
                    root,
                    &path.join(run),
                    run,
                    &selected_catalog,
                )?);
            } else {
                errors.push(format!("unexpected evidence directory {run}"));
            }
        }
    }
    if errors.is_empty() {
        println!(
            "QA artifacts validated{}",
            if release { " for release" } else { "" }
        );
        Ok(0)
    } else {
        for error in errors {
            eprintln!("ERROR: {error}");
        }
        Ok(2)
    }
}
