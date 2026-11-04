use super::{consts::*, prose, util::*};
use crate::common::{Diagnostic, warn};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
fn coverage(plan: &Value) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut names = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for skill in arr(&plan["skills"]) {
        if ne(&skill["name"]) {
            names.insert(s(&skill["name"]).trim().to_owned());
        }
        for path in ss(&skill["source_paths"]) {
            paths.insert(path.trim().to_owned());
        }
    }
    for e in arr(&plan["evidence"]) {
        if ne(&e["path"]) {
            paths.insert(s(&e["path"]).trim().to_owned());
        }
    }
    (names, paths)
}
fn dimensions(plan: &Value) -> BTreeMap<String, BTreeSet<String>> {
    let mut out = BTreeMap::from([
        ("lost_invariants".to_owned(), BTreeSet::new()),
        ("lost_owned_paths".to_owned(), BTreeSet::new()),
        ("lost_modules".to_owned(), BTreeSet::new()),
    ]);
    for invariant in arr(&plan["critical_invariants"]) {
        if ne(&invariant["statement"]) {
            out.get_mut("lost_invariants")
                .unwrap()
                .insert(s(&invariant["statement"]).trim().to_owned());
        }
    }
    for skill in arr(&plan["skills"]) {
        for ownership in arr(&skill["ownership"]) {
            for path in ss(&ownership["paths"]) {
                out.get_mut("lost_owned_paths")
                    .unwrap()
                    .insert(path.trim().to_owned());
            }
        }
    }
    for path in coverage(plan).1 {
        if let Some((head, _)) = path.split_once('/')
            && !head.is_empty()
            && !head.starts_with('.')
        {
            out.get_mut("lost_modules").unwrap().insert(head.to_owned());
        }
    }
    out
}
pub fn compare(current: &Path, baseline: &Path) -> (Vec<Diagnostic>, Value) {
    let mut d = Vec::new();
    let mut report = json!({"compared":false,"skipped_reason":"","lost_skills":[],"lost_source_paths":[],"lost_invariants":[],"lost_owned_paths":[],"lost_modules":[]});
    let old = match crate::common::read_object(&super::plan::absolute(baseline)) {
        Ok(v) => v,
        Err(e) => {
            let reason = e.to_string();
            warn(
                &mut d,
                "BASELINE_PLAN_UNREADABLE",
                format!("baseline plan is not readable JSON: {reason}"),
            );
            report["skipped_reason"] = json!(reason);
            return (d, report);
        }
    };
    let new = match crate::common::read_object(&super::plan::absolute(current)) {
        Ok(v) => v,
        Err(e) => {
            let reason = e.to_string();
            warn(
                &mut d,
                "BASELINE_COMPARE_SKIPPED",
                format!("coverage baseline not compared, current plan unreadable: {reason}"),
            );
            report["skipped_reason"] = json!(reason);
            return (d, report);
        }
    };
    let (old_names, old_paths) = coverage(&old);
    let (new_names, new_paths) = coverage(&new);
    let lost_names = old_names
        .difference(&new_names)
        .cloned()
        .collect::<Vec<_>>();
    let lost_paths = old_paths
        .difference(&new_paths)
        .cloned()
        .collect::<Vec<_>>();
    report["compared"] = json!(true);
    report["lost_skills"] = json!(lost_names);
    report["lost_source_paths"] = json!(lost_paths);
    let old_dims = dimensions(&old);
    let new_dims = dimensions(&new);
    for &(key, code, label) in BASELINE_DIMENSIONS {
        let lost = if key == "lost_invariants" {
            old_dims[key]
                .iter()
                .filter(|statement| {
                    !new_dims[key].iter().any(|current| {
                        prose::meaningful(statement)
                            .intersection(&prose::meaningful(current))
                            .count()
                            >= INVARIANT_MATCH_TOKENS
                    })
                })
                .cloned()
                .collect::<Vec<_>>()
        } else {
            old_dims[key]
                .difference(&new_dims[key])
                .cloned()
                .collect::<Vec<_>>()
        };
        report[key] = json!(lost);
        for member in lost {
            warn(
                &mut d,
                code,
                format!("{member}: {label} in baseline plan, absent from this plan"),
            )
        }
    }
    for name in lost_names {
        let cited = arr(&old["skills"])
            .iter()
            .filter(|v| s(&v["name"]).trim() == name)
            .flat_map(|v| ss(&v["source_paths"]))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(", ");
        warn(
            &mut d,
            "BASELINE_SKILL_DROPPED",
            format!(
                "{name}: in baseline inventory, absent from this plan (baseline source paths: {})",
                if cited.is_empty() { "none" } else { &cited }
            ),
        )
    }
    for path in lost_paths {
        warn(
            &mut d,
            "BASELINE_COVERAGE_DROPPED",
            format!("{path}: covered by baseline, uncovered by this plan"),
        )
    }
    (finish(d), report)
}
pub fn classes(plan: &Path) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::from([
        ("project".to_owned(), Vec::new()),
        ("runtime_fixed".to_owned(), Vec::new()),
    ]);
    if let Ok(plan) = crate::common::read_object(plan) {
        for v in arr(&plan["skills"]) {
            if ne(&v["name"]) {
                out.get_mut(if s(&v["kind"]).to_lowercase() == RUNTIME_FIXED_KIND {
                    "runtime_fixed"
                } else {
                    "project"
                })
                .unwrap()
                .push(s(&v["name"]).trim().to_owned());
            }
        }
    }
    for v in out.values_mut() {
        v.sort()
    }
    out
}
