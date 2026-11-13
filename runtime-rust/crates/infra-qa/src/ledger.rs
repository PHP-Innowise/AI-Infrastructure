use crate::{common::*, manifest, ooxml::Package};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path};
fn normalize(r: &Value) -> Value {
    json!({"run_id":pick(r,&["run_id","run id"],""),"test_case_id":pick(r,&["test_case_id","test case id"],""),"historical_result":pick(r,&["result","historical_result"],""),"framework":pick(r,&["framework"],"Unknown"),"tool":pick(r,&["ai_tool","tool"],"Unknown"),"tested_commit":pick(r,&["git_commit","tested_commit"],"Unknown"),"original_defect_id":nullable(pick(r,&["defect_id","defect id"],"")),"evidence_path":nullable(pick(r,&["evidence_link","evidence path"],""))})
}
fn defects(rows: &[Value]) -> Result<BTreeSet<(String, String, String)>> {
    let mut out = BTreeSet::new();
    for r in rows {
        let k = (
            pick(r, &["run_id", "run id"], ""),
            pick(r, &["test_case_id", "test case id"], ""),
            pick(r, &["defect_id", "defect id"], ""),
        );
        if !k.0.is_empty() && !k.1.is_empty() && !k.2.is_empty() && !out.insert(k) {
            bail!("duplicate composite defect");
        }
    }
    Ok(out)
}
fn requirement(r: &Value) -> &'static str {
    if r["run_id"] == "RUN-20260803-018" {
        "multiple"
    } else {
        let tool = text(&r["tool"]).to_lowercase();
        if tool.contains("cursor") {
            "cursor"
        } else if tool.contains("codex") {
            "codex"
        } else if tool.contains("claude") {
            "claude-code"
        } else {
            "none"
        }
    }
}
pub fn release_errors(rows: &[Value]) -> Vec<String> {
    let mut out = vec![];
    for r in rows {
        for field in ["final_evidence_path", "owner"] {
            if r[field].as_str().unwrap_or("").is_empty() {
                out.push(format!("{}: {field} required", text(&r["identity"])));
            }
        }
        if !FINAL.contains(&r["final_disposition"].as_str().unwrap_or("")) {
            out.push(format!("{}: disposition incomplete", text(&r["identity"])));
        }
        if !["approved", "rejected", "deferred"]
            .contains(&r["review_status"].as_str().unwrap_or(""))
        {
            out.push(format!("{}: review incomplete", text(&r["identity"])));
        }
    }
    out
}
pub fn build(root: &Path, evidence: &Path, workbook: &Path, release: bool) -> Result<Value> {
    let paths = primary(evidence)?;
    let errors = manifest::verify(evidence, &paths["SHA256SUMS.txt"], false)?;
    if !errors.is_empty() {
        bail!("{}", errors.join("; "));
    }
    let historical = records(
        &read_json(&paths["manifests/historical-runs.json"])?,
        &["runs", "records", "rows", "data"],
    )?
    .iter()
    .map(normalize)
    .collect::<Vec<_>>();
    if historical.len() != 57
        || historical
            .iter()
            .map(|r| text(&r["run_id"]))
            .collect::<BTreeSet<_>>()
            != crate::common::historical().into_iter().collect()
    {
        bail!("historical run identities differ");
    }
    assert_unique(&historical, &["run_id"])?;
    let book = Package::read(workbook)?;
    let native = book
        .records("Test Runs")?
        .iter()
        .filter(|r| text(&r["Run ID"]).starts_with("RUN-20260803-"))
        .map(normalize)
        .collect::<Vec<_>>();
    if native.len() != 18
        || native
            .iter()
            .map(|r| text(&r["run_id"]))
            .collect::<BTreeSet<_>>()
            != codex().into_iter().collect()
    {
        bail!("current workbook must contain eighteen Codex runs");
    }
    let cases = book
        .records("Test Cases")?
        .iter()
        .filter(|r| text(&r["Test Case ID"]).starts_with("TC-AI-"))
        .map(|r| text(&r["Test Case ID"]))
        .collect::<BTreeSet<_>>();
    if cases != (1..=18).map(|n| format!("TC-AI-{n:03}")).collect() {
        bail!("current workbook case identities differ");
    }
    let other = read_json(&paths["manifests/historical-othersheets.json"])?;
    let matrix = other["Defects"]
        .as_array()
        .context("historical defects matrix")?;
    if matrix.len() != 10 {
        bail!("historical Defects requires one header and nine rows");
    }
    let h = matrix[0].as_array().context("defect headers")?;
    let hd = matrix
        .iter()
        .skip(1)
        .map(|r| {
            let mut m = serde_json::Map::new();
            for (i, k) in h.iter().enumerate() {
                m.insert(text(k), r.get(i).cloned().unwrap_or(Value::Null));
            }
            Value::Object(m)
        })
        .collect::<Vec<_>>();
    let historical_defects = defects(&hd)?;
    let current_defects = defects(&book.records("Defects")?)?;
    let checksums = manifest::parse(&paths["SHA256SUMS.txt"])?;
    let mut output = vec![];
    for (group, runs) in [("historical-57", historical), ("codex-18", native)] {
        for r in runs {
            let run = text(&r["run_id"]);
            let case = text(&r["test_case_id"]);
            let mut original = text(&r["original_defect_id"]);
            let mut namespace = original.clone();
            let mut conflicts = vec![];
            if group == "codex-18" {
                let mut matching = current_defects
                    .iter()
                    .filter(|k| k.0 == run && k.1 == case)
                    .cloned()
                    .collect::<Vec<_>>();
                if run == "RUN-20260803-013" {
                    matching = vec![(run.clone(), case.clone(), "DEF-003".into())];
                    conflicts.push("Current Test Runs defect cell is blank; the rejected Defects row supplies original DEF-003.");
                }
                if !original.is_empty() {
                    let expected = (run.clone(), case.clone(), original.clone());
                    if !current_defects.contains(&expected) {
                        bail!("Codex defect lacks composite match");
                    }
                    matching = vec![expected];
                }
                if matching.len() > 1 {
                    bail!("ambiguous Codex defect");
                }
                if let Some(k) = matching.first() {
                    original = k.2.clone();
                    namespace = defect(&k.0, &k.1, &k.2)
                        .context("unexpected Codex defect composite")?
                        .into();
                }
            } else if !original.is_empty()
                && !historical_defects.contains(&(run.clone(), case.clone(), original.clone()))
            {
                bail!("historical defect lacks composite match");
            }
            let mut evidence_path = text(&r["evidence_path"]).replace('\\', "/");
            if evidence_path.starts_with("Accelerator-TestEvidence/") {
                evidence_path = format!(
                    "logs/original/{}",
                    evidence_path.rsplit('/').next().unwrap_or("")
                );
            }
            if evidence_path.ends_with("TC-074.log") {
                evidence_path = "transcripts/RUN-057-TC-074-sanitized.md".into();
            }
            let hash = checksums.get(&evidence_path).cloned();
            let status = if run == "RUN-057" {
                "withheld-sensitive"
            } else if !evidence_path.is_empty() && hash.is_some() {
                "available-original"
            } else if group == "codex-18" {
                "workbook-recorded"
            } else {
                "missing"
            };
            output.push(json!({"identity":format!("{run}|{case}|{}",if namespace.is_empty(){"NONE"}else{&namespace}),"run_id":run,"test_case_id":case,"historical_result":r["historical_result"],"framework":r["framework"],"tool":r["tool"],"tested_commit":r["tested_commit"],"source_group":group,"source_evidence_status":if group=="historical-57"{"package-verified"}else{"workbook-recorded"},"original_defect_id":nullable(original),"namespaced_defect_id":nullable(namespace),"evidence_status":status,"evidence_path":nullable(evidence_path),"evidence_sha256":hash,"classification":null,"remediation_workstream":null,"required_regressions":[],"native_client_requirement":requirement(&r),"final_disposition":null,"final_evidence_path":null,"owner":null,"review_status":"pending","provenance":{"sources":[if group=="historical-57"{"manifests/historical-runs.json".into()}else{relative(workbook,root)?}],"conflicts":conflicts}}));
        }
    }
    assert_unique(&output, &["run_id"])?;
    assert_unique(&output, &["identity"])?;
    if output.len() != 75 {
        bail!("ledger count differs");
    }
    if release {
        let e = release_errors(&output);
        if !e.is_empty() {
            bail!("{}", e.join("; "));
        }
    }
    output.sort_by_key(|r| text(&r["run_id"]));
    let mut hashes = serde_json::Map::new();
    for (n, p) in paths {
        hashes.insert(n, hash_file(&p)?.into());
    }
    hashes.insert(relative(workbook, root)?, hash_file(workbook)?.into());
    Ok(
        json!({"schema_version":NATIVE_VERSION,"generated_at":now(),"repository_commit":git(root,&["rev-parse","HEAD"])?.trim(),"release_ready":release,"source_checksums":hashes,"records":output}),
    )
}
pub fn apply(root: &Path, ledger: &Value, policy: &Value) -> Result<Value> {
    let rows = ledger["records"].as_array().context("ledger records")?;
    if rows.len() != 75 {
        bail!("ledger requires75 records");
    }
    let owner = policy["accelerator_owner"].as_str().unwrap_or("").trim();
    let external = policy["external_owner"].as_str().unwrap_or("").trim();
    let expected = ["RUN-053", "RUN-056", "RUN-057"]
        .into_iter()
        .collect::<BTreeSet<_>>();
    let supplied = policy["external_runs"]
        .as_array()
        .context("external runs")?
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    if owner.is_empty()
        || external.is_empty()
        || supplied != expected
        || policy["approval_policy"]
            != json!({"id":"evidence-backed-local-v1","eligible_review_status":"approved","ineligible_review_status":"pending"})
    {
        bail!("unsupported owner/approval policy");
    }
    let regressions = policy["historical_regressions"]
        .as_object()
        .context("historical regression policy")?;
    if regressions.values().any(|v| {
        v.as_array()
            .is_none_or(|a| a.is_empty() || a.iter().any(|v| v.as_str().is_none_or(str::is_empty)))
    }) {
        bail!("invalid historical regressions");
    }
    let evidence = root.join(
        policy["codex_evidence_root"]
            .as_str()
            .context("evidence root")?,
    );
    let mut updated = ledger.clone();
    let mut result = vec![];
    for source in rows {
        let mut r = source.clone();
        let run = text(&r["run_id"]);
        if expected.contains(run.as_str()) {
            r["classification"] = "external-client-finding".into();
            r["remediation_workstream"] = "external-client".into();
            r["owner"] = external.into();
            r["review_status"] = "pending".into();
            r["final_disposition"] = Value::Null;
            r["final_evidence_path"] = Value::Null;
        } else if r["source_group"] == "historical-57" {
            r["owner"] = owner.into();
            let defect = text(&r["namespaced_defect_id"]);
            if let Some(patterns) = regressions.get(&defect) {
                r["classification"] = "accelerator-finding".into();
                r["remediation_workstream"] = "post-remediation-verification".into();
                r["required_regressions"] = patterns.clone();
            }
            if r["historical_result"] == "Pass"
                && defect.is_empty()
                && r["native_client_requirement"] == "none"
                && policy["historical_pass_policy"] == "propose-accepted-historical-pass"
            {
                r["classification"] = "historical-evidence-pass".into();
                r["remediation_workstream"] = "historical-evidence-acceptance".into();
                r["final_disposition"] = "accepted-historical-pass".into();
                r["final_evidence_path"] = r["evidence_path"]
                    .as_str()
                    .map(|p| format!("docs/Accelerator-TestEvidence-Submission/{p}").into())
                    .unwrap_or(Value::Null);
                r["review_status"] = "approved".into();
            } else if r["historical_result"] == "Pass" {
                r["classification"] = if !defect.is_empty() {
                    "accelerator-finding"
                } else {
                    "native-client-verification"
                }
                .into();
                r["remediation_workstream"] = if !defect.is_empty() {
                    "post-remediation-verification"
                } else {
                    "native-client-closure"
                }
                .into();
                r["final_disposition"] = Value::Null;
                r["final_evidence_path"] = Value::Null;
                r["review_status"] = "pending".into();
            }
        } else if r["source_group"] == "codex-18" {
            let dir = evidence.join(&run);
            let reproduced = read_json(&dir.join("run.json"))?;
            let selected_catalog = crate::catalog::for_version(root, schema_version(&reproduced)?)?;
            let case = selected_catalog.case(&run)?;
            let errors =
                crate::validate::run_evidence_with_catalog(root, &dir, &run, &selected_catalog)?;
            if !errors.is_empty() {
                bail!("{}", errors.join("; "));
            }
            r["owner"] = owner.into();
            for k in ["source_evidence_status", "evidence_status"] {
                r[k] = "newly-reproduced".into();
            }
            r["evidence_path"] = relative(&dir.join("run.json"), root)?.into();
            r["evidence_sha256"] = hash_file(&dir.join("run.json"))?.into();
            r["classification"] = case["classification"].clone();
            r["remediation_workstream"] = "codex-18-remediation".into();
            r["required_regressions"] = json!(
                case["required_test_patterns"]
                    .as_array()
                    .context("test patterns")?
                    .iter()
                    .map(|p| p["regex"].clone())
                    .collect::<Vec<_>>()
            );
            r["final_evidence_path"] = relative(&dir, root)?.into();
            if reproduced["status"] == "passed" {
                r["final_disposition"] = if run == "RUN-20260803-013" {
                    "closed-rejected-finding"
                } else if r["historical_result"] == "Pass" {
                    "verified-unmodified-pass"
                } else {
                    "verified-after-remediation"
                }
                .into();
                r["review_status"] = "approved".into();
            } else {
                r["final_disposition"] = Value::Null;
                r["review_status"] = "pending".into();
            }
        }
        result.push(r);
    }
    updated["records"] = json!(result);
    Ok(updated)
}
