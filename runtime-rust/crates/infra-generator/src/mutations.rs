use crate::{common::*, quality};
use serde_json::{Value, json as object};
use std::{collections::BTreeSet, path::Path};
const MUTATIONS: &[(&str, &str)] = &[
    ("a catalog obligation is dropped", "CATALOG_ROLE_UNCOVERED"),
    (
        "every obligation is discharged by one step",
        "PROCEDURE_ROLE_COLLAPSED",
    ),
    (
        "an executable check loses its baseline",
        "VERIFICATION_BASELINE_MISSING",
    ),
    (
        "a check promises absolute success against a failing baseline",
        "VERIFICATION_BASELINE_CONTRADICTED",
    ),
    (
        "a runtime check promises zero despite nonzero contract exits",
        "RUNTIME_EXPECTATION_CONTRADICTED",
    ),
    (
        "a routing fixture names its winning skill",
        "ROUTING_CASE_TAUTOLOGICAL",
    ),
    (
        "a skill stops naming reconciled claims",
        "SKILL_CLAIM_IDS_INVALID",
    ),
    (
        "a skill stops disposing evidence in its paths",
        "SKILL_EVIDENCE_UNDISPOSED",
    ),
    (
        "a preexisting team skill fingerprint changes",
        "PREEXISTING_TEAM_SKILL_DRIFT",
    ),
];
fn normalize_command(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
pub fn damage(plan: &mut Value, index: usize, registry: &Value, runtime: &Value) -> Option<String> {
    if index == 8 {
        for record in plan["preexisting_team_skills"]
            .as_array_mut()
            .into_iter()
            .flatten()
        {
            let name = text(&record["name"]).to_string();
            if let Some(first) = record["files"]
                .as_array_mut()
                .and_then(|files| files.first_mut())
                && nonempty(&first["sha256"])
            {
                first["sha256"] = "0".repeat(64).into();
                return Some(if name.is_empty() {
                    "preexisting-team".into()
                } else {
                    name
                });
            }
        }
        return None;
    }
    let skills = plan["skills"].as_array_mut()?;
    for skill in skills {
        let name = text(&skill["name"]).to_string();
        let fixed = skill["kind"] == "runtime-fixed";
        if index != 4 && fixed {
            continue;
        }
        if index == 4 && !fixed {
            continue;
        }
        match index {
            0 => {
                let id = text(&skill["selection_gate"]["candidate_id"]);
                let roles = registry["candidates"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|entry| text(&entry["id"]) == id)
                    .and_then(|entry| entry["roles"].as_array());
                if roles.is_some_and(|roles| !roles.is_empty())
                    && let Some(entries) = skill["required_procedure_roles"]
                        .as_array_mut()
                        .filter(|entries| entries.len() > 1)
                {
                    entries.pop();
                    return Some(name);
                }
            }
            1 => {
                let steps = skill["procedure_steps"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                if steps.len() >= 2
                    && let Some(entries) = skill["required_procedure_roles"]
                        .as_array_mut()
                        .filter(|entries| entries.len() >= 3)
                {
                    for entry in entries {
                        if entry.is_object() {
                            entry["procedure_step_ids"] = object!([steps[0]["id"]]);
                        }
                    }
                    return Some(name);
                }
            }
            2 => {
                for check in skill["verification"].as_array_mut().into_iter().flatten() {
                    if check["baseline"]
                        .as_object()
                        .is_some_and(|value| !value.is_empty())
                    {
                        check["baseline"] = Value::Null;
                        return Some(name);
                    }
                }
            }
            3 => {
                for check in skill["verification"].as_array_mut().into_iter().flatten() {
                    if check["baseline"]["outcome"] == "failing" {
                        check["expected_result"] =
                            "The command exits zero and reports no errors".into();
                        return Some(name);
                    }
                }
            }
            4 => {
                for check in skill["verification"].as_array_mut().into_iter().flatten() {
                    let command = normalize_command(text(&check["command"]));
                    if runtime["commands"]["outcomes"][&command]
                        .as_array()
                        .is_some_and(|outcomes| {
                            outcomes.iter().any(|outcome| {
                                outcome["exit"].as_i64().is_some_and(|code| code != 0)
                            })
                        })
                    {
                        check["expected_result"] =
                            "The command exits zero and reports no error".into();
                        return Some(name);
                    }
                }
            }
            5 => {
                if let Some(case) = skill["routing_cases"]
                    .as_array_mut()
                    .and_then(|cases| cases.first_mut())
                    .filter(|case| case.is_object())
                {
                    let spoken = name.replace('-', " ");
                    case["prompt"] = format!("use {spoken} to handle this {spoken} request").into();
                    return Some(name);
                }
            }
            6 => {
                if skill["claim_ids"]
                    .as_array()
                    .is_some_and(|items| !items.is_empty())
                {
                    skill["claim_ids"] = object!([]);
                    return Some(name);
                }
            }
            7 if skill["evidence_dispositions"]
                .as_array()
                .is_some_and(|items| !items.is_empty()) =>
            {
                skill["evidence_dispositions"] = object!([]);
                return Some(name);
            }
            _ => {}
        }
    }
    None
}
pub fn validate(
    plan_path: &Path,
    target: &Path,
    registry_path: &Path,
    runtime: &Value,
) -> Vec<Diagnostic> {
    let mut items = vec![];
    let original = match read_object(plan_path) {
        Ok(value) => value,
        Err(error) => {
            diag(&mut items, "MUTATION_PLAN_UNREADABLE", error.to_string());
            return items;
        }
    };
    let registry = match read_object(registry_path) {
        Ok(value) => value,
        Err(error) => {
            diag(
                &mut items,
                "MUTATION_PLAN_UNREADABLE",
                format!("registry: {error}"),
            );
            return items;
        }
    };
    let errors_for = |plan: &Value| -> BTreeSet<String> {
        quality::validate_plan_value(plan, plan_path, target, registry_path)
            .into_iter()
            .filter(|diagnostic| diagnostic.severity == "error")
            .map(|diagnostic| diagnostic.code)
            .collect()
    };
    let control = errors_for(&original);
    if !control.is_empty() {
        diag(
            &mut items,
            "MUTATION_CONTROL_DIRTY",
            format!(
                "unmutated plan does not validate clean: {}",
                control.into_iter().collect::<Vec<_>>().join(", ")
            ),
        );
        return items;
    }
    for (index, (description, expected)) in MUTATIONS.iter().enumerate() {
        let mut damaged = original.clone();
        let Some(touched) = damage(&mut damaged, index, &registry, runtime) else {
            warn(
                &mut items,
                "MUTATION_NOT_APPLICABLE",
                format!("plan has no shape where {description}; guard remains unexercised"),
            );
            continue;
        };
        let errors = errors_for(&damaged);
        if errors.is_empty() {
            diag(
                &mut items,
                "MUTATION_UNCAUGHT",
                format!("{touched}: gate accepted plan where {description}"),
            );
        } else if !errors.contains(*expected) {
            diag(
                &mut items,
                "MUTATION_CODE_MISSING",
                format!("{touched}: rejected without required guard {expected}: {errors:?}"),
            );
        }
    }
    items
}
