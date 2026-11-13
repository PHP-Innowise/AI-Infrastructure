use super::{consts::*, paths, registry, util::*};
use crate::common::{Diagnostic, diag};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
pub fn path_tokens(text: &str) -> Vec<String> {
    split(r#"[\s,;()\[\]\x60"']+"#, text)
        .into_iter()
        .filter_map(|raw| {
            let v = rstrip(raw.trim(), ".,;:!?");
            if v.len() < 3 || has(REMOTE_LOCATION_PATTERN, &v) {
                return None;
            }
            let extension = v
                .rsplit_once('.')
                .map(|(_, x)| x.to_lowercase())
                .unwrap_or_default();
            (v.contains('/')
                || full(DOTFILE_SPAN_PATTERN, &v)
                || REJECTION_EXTENSIONS.contains(&extension.as_str()))
            .then_some(v)
        })
        .collect()
}
fn reaches(candidate: &Value, owner: &Value) -> Option<bool> {
    let surfaces = arr(&candidate["falsifier"]["probes"])
        .iter()
        .flat_map(|p| ss(&p["paths"]))
        .collect::<Vec<_>>();
    if surfaces.is_empty() {
        return None;
    }
    let mut declared = ss(&owner["writes"]);
    declared.extend(paths::territory(owner).into_iter().map(|(p, _)| p));
    Some(
        surfaces
            .iter()
            .any(|p| declared.iter().any(|q| paths::intersects(p, q))),
    )
}
pub fn fields(item: &Value, disposed: bool) -> Vec<&'static str> {
    if !disposed {
        return vec![
            "candidate_id",
            "name",
            "category",
            "reason",
            "missing_evidence",
        ];
    }
    let mut out = vec!["candidate_id", "name", "category", "reason", "disposition"];
    out.push(if s(&item["disposition"]) == "consolidated" {
        "absorbed_by"
    } else {
        "missing_evidence"
    });
    out
}
pub fn validate(
    plan: &Value,
    candidates: &SkillMap,
    target: &Path,
    schema: &str,
    d: &mut Vec<Diagnostic>,
) -> (BTreeSet<String>, Vec<String>) {
    let mut names = BTreeSet::new();
    let mut candidate_ids = Vec::new();
    if !plan["rejected_candidates"].is_array() {
        diag(
            d,
            "REJECTED_CANDIDATES_INVALID",
            "rejected_candidates must be an array",
        );
        return (names, candidate_ids);
    }
    let selected = arr(&plan["skills"])
        .iter()
        .filter(|v| ne(&v["name"]))
        .map(|v| (s(&v["name"]).to_owned(), v.clone()))
        .collect::<SkillMap>();
    let mut valid = Vec::new();
    for item in arr(&plan["rejected_candidates"]) {
        if !exact(item, &fields(item, disposed(schema)))
            || ["candidate_id", "name", "category", "reason"]
                .iter()
                .any(|k| !ne(&item[*k]))
            || (item.get("missing_evidence").is_some() && !sl(&item["missing_evidence"], true))
            || (item.get("absorbed_by").is_some() && !ne(&item["absorbed_by"]))
        {
            diag(
                d,
                "REJECTED_CANDIDATE_INVALID",
                format!("invalid rejection: {item}"),
            );
            continue;
        }
        let name = s(&item["name"]).to_owned();
        if !names.insert(name.clone()) {
            diag(
                d,
                "REJECTED_CANDIDATE_DUPLICATE",
                format!("duplicate rejected candidate: {name}"),
            )
        }
        let id = s(&item["candidate_id"]).to_owned();
        candidate_ids.push(id.clone());
        if let Some(candidate) = candidates.get(&id) {
            if candidate["golden"] == true {
                diag(
                    d,
                    "GOLDEN_CANDIDATE_REJECTED",
                    format!("golden candidate cannot be rejected: {id}"),
                )
            }
            if s(&candidate["category"]) != s(&item["category"]) {
                diag(
                    d,
                    "REJECTED_CANDIDATE_CONTRACT",
                    format!("{id} category differs from registry"),
                )
            }
            if s(&candidate["mode"]) == "static" && name != id {
                diag(
                    d,
                    "REJECTED_CANDIDATE_CONTRACT",
                    format!("{id} static rejection name must equal id"),
                )
            }
            if s(&candidate["mode"]) == "runtime-fixed" {
                diag(
                    d,
                    "REJECTED_CANDIDATE_CONTRACT",
                    format!("runtime-fixed candidate cannot be rejected: {id}"),
                )
            }
            if item.get("missing_evidence").is_some() && arr(&item["missing_evidence"]).is_empty() {
                diag(
                    d,
                    "REJECTED_CANDIDATE_CONTRACT",
                    format!("{id} rejection must name missing evidence"),
                )
            }
        } else {
            diag(
                d,
                "REJECTED_CANDIDATE_UNKNOWN",
                format!("unknown rejected registry candidate: {id}"),
            )
        }
        valid.push(item);
    }
    let mut shared: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    for item in &valid {
        let id = s(&item["candidate_id"]);
        let reason = s(&item["reason"]);
        let evidence = ss(&item["missing_evidence"]).join(" ");
        if path_tokens(&format!("{reason} {evidence}")).is_empty() {
            diag(
                d,
                "REJECTION_UNANCHORED",
                format!("rejected {id} names no target path where surface was looked for"),
            )
        }
        let key = reason
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        if !key.is_empty() {
            shared.entry(key).or_default().push(item)
        }
        if disposed(schema) {
            let disposition = s(&item["disposition"]);
            if !["absent", "consolidated"].contains(&disposition) {
                diag(
                    d,
                    "REJECTION_DISPOSITION_INVALID",
                    format!("{id} must declare absent or consolidated"),
                )
            } else if disposition == "consolidated" {
                let owner = s(&item["absorbed_by"]);
                if !selected.contains_key(owner) || owner == s(&item["name"]) {
                    diag(
                        d,
                        "REJECTION_ABSORBER_UNKNOWN",
                        format!("{id} cannot consolidate into unselected/self owner {owner}"),
                    )
                } else if candidates
                    .get(id)
                    .and_then(|c| reaches(c, &selected[owner]))
                    == Some(false)
                {
                    diag(
                        d,
                        "REJECTION_ABSORBER_OUT_OF_REACH",
                        format!("{id} absorbing skill {owner} paths do not reach concern"),
                    )
                }
            }
        }
    }
    for items in shared.values() {
        let categories = items
            .iter()
            .map(|i| s(&i["category"]).to_lowercase())
            .collect::<BTreeSet<_>>();
        if items.len() > SHARED_REJECTION_LIMIT && categories.len() > 1 {
            diag(
                d,
                "REJECTION_TEMPLATED",
                format!(
                    "one rejection sentence covers {} candidates across {} categories: {}",
                    items.len(),
                    categories.len(),
                    items
                        .iter()
                        .map(|i| s(&i["candidate_id"]))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
        }
    }
    let checkable = valid
        .iter()
        .filter(|v| {
            candidates
                .get(s(&v["candidate_id"]))
                .is_some_and(|c| !c["falsifier"].is_null())
        })
        .collect::<Vec<_>>();
    if !checkable.is_empty() {
        let index = registry::index(target);
        for item in checkable {
            let id = s(&item["candidate_id"]);
            let falsifier = &candidates[id]["falsifier"];
            match registry::fires(falsifier, target, &index) {
                Err(error) => diag(
                    d,
                    "REJECTION_FALSIFIER_UNRESOLVED",
                    format!("{id} cannot evaluate falsifier: {error}"),
                ),
                Ok(fires) => {
                    let disposition = item
                        .get("disposition")
                        .and_then(Value::as_str)
                        .unwrap_or("absent");
                    if disposition == "absent" && fires {
                        diag(
                            d,
                            "REJECTION_CONTRADICTED",
                            format!(
                                "{id} rejected as absent but target holds {}",
                                s(&falsifier["surface"])
                            ),
                        )
                    } else if disposition == "consolidated" && !fires {
                        diag(
                            d,
                            "REJECTION_CONSOLIDATION_WITHOUT_SURFACE",
                            format!(
                                "{id} consolidated without target surface {}",
                                s(&falsifier["surface"])
                            ),
                        )
                    }
                }
            }
        }
    }
    (names, candidate_ids)
}
pub fn absent_golden(
    name: &str,
    skill: &Value,
    candidates: &SkillMap,
    evidence: &EvidenceMap,
    d: &mut Vec<Diagnostic>,
) {
    let conditions = arr(&skill["selection_gate"]["conditions"]);
    let absent = conditions
        .iter()
        .filter(|c| s(&c["status"]) == "absent-golden")
        .collect::<Vec<_>>();
    if absent.is_empty() {
        if ne(&skill["narrow_scope"]) {
            diag(
                d,
                "NARROW_SCOPE_UNEXPECTED",
                format!("{name} narrows scope while every selection condition says surface exists"),
            )
        }
        return;
    }
    let id = s(&skill["selection_gate"]["candidate_id"]);
    if !candidates.get(id).is_some_and(|c| c["golden"] == true) {
        diag(
            d,
            "ABSENT_GOLDEN_NOT_PERMITTED",
            format!("{name}: only a golden candidate may generate with absent surface"),
        )
    }
    if !ne(&skill["narrow_scope"]) {
        diag(
            d,
            "NARROW_SCOPE_MISSING",
            format!("{name} must state useful scope and limits until missing surface exists"),
        )
    }
    for c in absent {
        if arr(&c["evidence_ids"]).is_empty() {
            diag(
                d,
                "ABSENT_GOLDEN_UNPROVEN",
                format!("{name} lacks absence evidence"),
            )
        }
        for id in ss(&c["evidence_ids"]) {
            if !evidence.get(&id).is_some_and(|(kind, _)| kind == "absence") {
                diag(
                    d,
                    "ABSENT_GOLDEN_UNPROVEN",
                    format!("{name} rests absent surface on present evidence {id}"),
                )
            }
        }
    }
}
