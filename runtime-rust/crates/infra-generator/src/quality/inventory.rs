use super::{consts::*, paths, prose, similarity, util::*};
use crate::common::{Diagnostic, diag, warn};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
pub fn preexisting(value: &Value, target: &Path, d: &mut Vec<Diagnostic>) -> SkillMap {
    let mut records = SkillMap::new();
    if !value.is_array() {
        diag(
            d,
            "PREEXISTING_TEAM_SKILLS_INVALID",
            "preexisting_team_skills must be an array",
        );
        return records;
    }
    let fields = [
        "name",
        "candidate_id",
        "edition",
        "path",
        "kind",
        "mode",
        "ownership",
        "publication",
        "generated_alias",
        "symlink_target",
        "symlink_sha256",
        "files",
    ];
    let root = target.canonicalize().unwrap_or_else(|_| target.to_owned());
    for (i, v) in arr(value).iter().enumerate() {
        let label = format!("preexisting_team_skills[{i}]");
        if !exact(v, &fields)
            || fields
                .iter()
                .filter(|k| **k != "files")
                .any(|k| !ne(&v[*k]))
        {
            diag(
                d,
                "PREEXISTING_TEAM_SKILL_INVALID",
                format!("{label} must contain exact nonempty scalar fields and files"),
            );
            continue;
        }
        let name = s(&v["name"]).trim();
        if records.contains_key(name) {
            diag(
                d,
                "PREEXISTING_TEAM_SKILL_DUPLICATE",
                format!("duplicate team skill: {name}"),
            );
            continue;
        }
        if s(&v["kind"]) != "preexisting-team"
            || s(&v["mode"]) != "merge"
            || s(&v["ownership"]) != "team"
            || s(&v["publication"]) != "watch-only"
            || !["claude", "cursor", "codex"].contains(&s(&v["edition"]))
            || !safe(s(&v["path"]))
            || !safe(s(&v["generated_alias"]))
            || s(&v["generated_alias"]).contains('/')
        {
            diag(
                d,
                "PREEXISTING_TEAM_SKILL_INVALID",
                format!("{label} must be merge-only, team-owned, watch-only and target-relative"),
            );
            continue;
        }
        let candidate = root.join(s(&v["path"]));
        if !candidate
            .parent()
            .and_then(|p| p.canonicalize().ok())
            .is_some_and(|p| p.starts_with(&root))
        {
            diag(
                d,
                "PREEXISTING_TEAM_SKILL_PATH",
                format!("{name} protected path escapes target"),
            );
            continue;
        }
        if !candidate.is_symlink() {
            diag(
                d,
                "PREEXISTING_TEAM_SKILL_DRIFT",
                format!("{name} protected path is no longer approved symlink"),
            );
            continue;
        }
        match std::fs::read_link(&candidate)
            .ok()
            .and_then(|p| p.to_str().map(str::to_owned))
        {
            Some(link) => {
                if link != s(&v["symlink_target"])
                    || crate::common::sha(link.as_bytes()) != s(&v["symlink_sha256"])
                {
                    diag(
                        d,
                        "PREEXISTING_TEAM_SKILL_DRIFT",
                        format!("{name} symlink identity drifted"),
                    )
                }
            }
            None => diag(
                d,
                "PREEXISTING_TEAM_SKILL_DRIFT",
                format!("{name} symlink target is unreadable or not UTF-8"),
            ),
        }
        if arr(&v["files"]).is_empty() {
            diag(
                d,
                "PREEXISTING_TEAM_SKILL_INVALID",
                format!("{name}.files must be non-empty"),
            );
            continue;
        }
        let mut seen = BTreeSet::new();
        for (index, file) in arr(&v["files"]).iter().enumerate() {
            let path = s(&file["path"]);
            if !exact(file, &["path", "sha256"])
                || !safe(path)
                || !full(r"[0-9a-f]{64}", s(&file["sha256"]))
            {
                diag(
                    d,
                    "PREEXISTING_TEAM_SKILL_INVALID",
                    format!("{name}.files[{index}] needs safe path and SHA-256"),
                );
                continue;
            }
            if !seen.insert(path) {
                diag(
                    d,
                    "PREEXISTING_TEAM_SKILL_INVALID",
                    format!("{name} repeats protected file {path}"),
                );
                continue;
            }
            let actual = confined(target, path)
                .filter(|p| p.is_file() && !p.is_symlink())
                .and_then(|p| std::fs::read(p).ok());
            if actual.as_ref().map(|b| crate::common::sha(b)).as_deref() != Some(s(&file["sha256"]))
            {
                diag(
                    d,
                    "PREEXISTING_TEAM_SKILL_DRIFT",
                    format!("{name} protected file missing, unsafe or fingerprint drifted: {path}"),
                );
            }
        }
        records.insert(name.to_owned(), v.clone());
    }
    records
}
pub fn validate(skills: &mut SkillMap, schema: &str, d: &mut Vec<Diagnostic>) {
    if schema == "1.0" {
        paths::legacy(skills)
    }
    let names = skills.keys().cloned().collect::<Vec<_>>();
    let mut entries: BTreeMap<String, Vec<(String, Value)>> = BTreeMap::new();
    if schema == "1.1" || typed(schema) {
        for (name, skill) in skills.iter_mut() {
            if arr(&skill["ownership"]).is_empty() {
                diag(
                    d,
                    "SKILL_OWNERSHIP_INVALID",
                    format!("{name}.ownership must be non-empty"),
                );
                continue;
            }
            let mut local = BTreeSet::new();
            for (i, item) in arr(&skill["ownership"]).iter().enumerate() {
                if !exact(item, &["id", "mode", "description", "paths"])
                    || !ne(&item["id"])
                    || !full(OWNERSHIP_ID_PATTERN, s(&item["id"]).trim())
                    || !OWNERSHIP_MODES.contains(&s(&item["mode"]))
                    || !ne(&item["description"])
                    || !sl(&item["paths"], false)
                {
                    diag(
                        d,
                        "SKILL_OWNERSHIP_INVALID",
                        format!("{name}.ownership[{i}] needs id/mode/description/paths"),
                    );
                    continue;
                }
                let id = s(&item["id"]).trim().to_owned();
                if !local.insert(id.clone()) {
                    diag(
                        d,
                        "SKILL_OWNERSHIP_DUPLICATE",
                        format!("{name} repeats ownership id {id}"),
                    )
                }
                entries
                    .entry(id.clone())
                    .or_default()
                    .push((name.clone(), item.clone()));
                for path in ss(&item["paths"]) {
                    if !safe(&path) || paths::normalize_glob(&path) != path {
                        diag(
                            d,
                            "SKILL_OWNERSHIP_PATH_INVALID",
                            format!("{name}.{id} path is not normalized: {path}"),
                        )
                    }
                }
            }
            skill["_normalized_ownership"] = skill["ownership"].clone();
            let mut siblings = Vec::new();
            for (i, item) in arr(&skill["nearest_siblings"]).iter().enumerate() {
                if !exact(item, &["name", "role", "ownership_ids", "boundary"])
                    || !ROUTING_ROLES.contains(&s(&item["role"]))
                    || !sl(&item["ownership_ids"], false)
                    || !ne(&item["boundary"])
                {
                    diag(
                        d,
                        "SKILL_SIBLING_CONTRACT_INVALID",
                        format!(
                            "{name}.nearest_siblings[{i}] needs name/role/ownership_ids/boundary"
                        ),
                    );
                    continue;
                }
                siblings.push(item.clone());
            }
            skill["_normalized_siblings"] = json!(siblings);
        }
        let owner_map = entries
            .iter()
            .map(|(id, e)| {
                (
                    id.clone(),
                    e.iter().map(|(n, _)| n.clone()).collect::<BTreeSet<_>>(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        for (id, items) in &entries {
            let modes = items
                .iter()
                .map(|(_, v)| s(&v["mode"]))
                .collect::<BTreeSet<_>>();
            let descriptions = items
                .iter()
                .map(|(_, v)| similarity::normalize_line(s(&v["description"])))
                .collect::<BTreeSet<_>>();
            if modes.len() != 1 || descriptions.len() != 1 {
                diag(
                    d,
                    "OWNERSHIP_ID_CONFLICT",
                    format!("{id} has inconsistent mode or description"),
                );
                continue;
            }
            let mode = *modes.first().unwrap();
            let owners = &owner_map[id];
            let paths = items
                .iter()
                .flat_map(|(_, v)| ss(&v["paths"]))
                .collect::<BTreeSet<_>>();
            if mode == "exclusive" {
                if owners.len() != 1 {
                    diag(
                        d,
                        "OWNERSHIP_ID_CONFLICT",
                        format!("exclusive ownership {id} has multiple owners: {owners:?}"),
                    )
                }
                for writer in &names {
                    if owners.contains(writer) {
                        continue;
                    }
                    let intrusions = paths
                        .iter()
                        .filter(|path| {
                            ss(&skills[writer]["writes"])
                                .iter()
                                .any(|write| paths::intersects(path, write))
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    if !intrusions.is_empty() {
                        diag(
                            d,
                            "OWNERSHIP_EXCLUSIVE_WRITE_CONFLICT",
                            format!(
                                "{writer} writes into exclusive ownership {id}: {}",
                                intrusions.join(", ")
                            ),
                        )
                    }
                }
            }
            if mode == "shared" {
                let owners = owners.iter().collect::<Vec<_>>();
                for (i, a) in owners.iter().enumerate() {
                    for b in owners.iter().skip(i + 1) {
                        let c = paths::collisions(&skills[*a]["writes"], &skills[*b]["writes"]);
                        if !c.is_empty() {
                            diag(
                                d,
                                "OWNERSHIP_SHARED_WRITE_CONFLICT",
                                format!(
                                    "shared {id} cannot authorize overlapping writes by {a} and {b}: {c:?}"
                                ),
                            )
                        }
                    }
                }
            }
            if mode == "composed" {
                for path in paths {
                    let writers = owners
                        .iter()
                        .filter(|n| {
                            ss(&skills[*n]["writes"])
                                .iter()
                                .any(|w| paths::intersects(&path, w))
                        })
                        .collect::<Vec<_>>();
                    if writers.len() != 1 {
                        diag(
                            d,
                            "OWNERSHIP_COMPOSER_CONFLICT",
                            format!(
                                "composed {id} path {path} requires exactly one writer; found {writers:?}"
                            ),
                        )
                    }
                }
            }
        }
        for (name, skill) in skills.iter() {
            for sibling in arr(&skill["_normalized_siblings"]) {
                let other = s(&sibling["name"]);
                let Some(other_skill) = skills.get(other) else {
                    continue;
                };
                let ids = set(&sibling["ownership_ids"]);
                for id in &ids {
                    match owner_map.get(id) {
                        None => diag(
                            d,
                            "SKILL_SIBLING_OWNERSHIP_UNKNOWN",
                            format!("{name} -> {other} unknown ownership {id}"),
                        ),
                        Some(owners) if !owners.contains(name) && !owners.contains(other) => diag(
                            d,
                            "SKILL_SIBLING_OWNERSHIP_MISMATCH",
                            format!("{name} -> {other} unrelated ownership {id}"),
                        ),
                        _ => {}
                    }
                }
                let Some(reciprocal) = paths::relation(other_skill, name) else {
                    diag(
                        d,
                        "SKILL_SIBLING_RECIPROCAL_MISSING",
                        format!("{name} -> {other} has no reciprocal relation"),
                    );
                    continue;
                };
                if set(&reciprocal["ownership_ids"]) != ids {
                    diag(
                        d,
                        "SKILL_SIBLING_OWNERSHIP_MISMATCH",
                        format!("{name} and {other} have different reciprocal ownership ids"),
                    )
                }
                if !paths::reciprocal(s(&sibling["role"]), s(&reciprocal["role"])) {
                    diag(
                        d,
                        "SKILL_SIBLING_ROLE_CONTRADICTION",
                        format!(
                            "{name} and {other} require primary-defer or primary-fallback reciprocity"
                        ),
                    )
                }
            }
        }
    }
    let projections = skills
        .iter()
        .map(|(n, s)| (n.clone(), similarity::projection(s)))
        .collect::<BTreeMap<_, _>>();
    let mut repeated: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for (i, an) in names.iter().enumerate() {
        for bn in names.iter().skip(i + 1) {
            let a = &skills[an];
            let b = &skills[bn];
            for (field, av, bv) in [
                (
                    "owned_scope",
                    a["owned_scope"].clone(),
                    b["owned_scope"].clone(),
                ),
                (
                    "ownership.description",
                    json!(
                        arr(&a["ownership"])
                            .iter()
                            .map(|v| v["description"].clone())
                            .collect::<Vec<_>>()
                    ),
                    json!(
                        arr(&b["ownership"])
                            .iter()
                            .map(|v| v["description"].clone())
                            .collect::<Vec<_>>()
                    ),
                ),
                (
                    "triggers.positive",
                    a["triggers"]["positive"].clone(),
                    b["triggers"]["positive"].clone(),
                ),
            ] {
                let av = flat(&av)
                    .iter()
                    .map(|s| similarity::normalize_line(s))
                    .filter(|s| !s.is_empty())
                    .collect::<BTreeSet<_>>();
                let bv = flat(&bv)
                    .iter()
                    .map(|s| similarity::normalize_line(s))
                    .collect::<BTreeSet<_>>();
                for line in av.intersection(&bv) {
                    diag(
                        d,
                        "BOUNDARY_NOT_SEPARATING",
                        format!("{an} and {bn} declare the same {field}: {line}"),
                    )
                }
            }
            let scope = paths::scope_collisions(
                &paths::scope(&a["owned_scope"]),
                &paths::scope(&b["owned_scope"]),
            );
            if !scope.is_empty() {
                let resolved = (schema == "1.1" || typed(schema))
                    && paths::relation(a, bn)
                        .zip(paths::relation(b, an))
                        .is_some_and(|(x, y)| {
                            paths::reciprocal(s(&x["role"]), s(&y["role"]))
                                && set(&x["ownership_ids"]) == set(&y["ownership_ids"])
                        });
                if !resolved {
                    diag(
                        d,
                        "SCOPE_COLLISION",
                        format!("{an} and {bn} overlap ownership: {scope:?}"),
                    )
                }
            }
            let collisions = paths::collisions(&a["writes"], &b["writes"]);
            if !collisions.is_empty() {
                diag(
                    d,
                    "WRITE_SURFACE_COLLISION",
                    format!("{an} and {bn} intersect writes: {collisions:?}"),
                )
            }
            let precedence = paths::precedence(a, bn) || paths::precedence(b, an);
            let a_tokens = prose::meaningful(&ss(&a["triggers"]["positive"]).join(" "));
            let b_tokens = prose::meaningful(&ss(&b["triggers"]["positive"]).join(" "));
            let union = a_tokens.union(&b_tokens).count();
            let score = if union == 0 {
                0.0
            } else {
                a_tokens.intersection(&b_tokens).count() as f64 / union as f64
            };
            if score >= 0.75 && !precedence {
                diag(
                    d,
                    "ROUTING_AMBIGUITY",
                    format!(
                        "{an} and {bn} overlap positive routing ({score:.3}) without precedence"
                    ),
                )
            }
            if !precedence {
                for at in ss(&a["triggers"]["positive"]) {
                    let atokens = prose::meaningful(&at);
                    if atokens.is_empty() {
                        continue;
                    }
                    for bt in ss(&b["triggers"]["positive"]) {
                        let btokens = prose::meaningful(&bt);
                        let union = atokens.union(&btokens).count();
                        if union > 0
                            && atokens.intersection(&btokens).count() as f64 / union as f64 >= 0.75
                        {
                            diag(
                                d,
                                "ROUTING_AMBIGUITY",
                                format!(
                                    "{an} and {bn} share ambiguous positive triggers: {at:?} vs {bt:?}"
                                ),
                            )
                        }
                    }
                }
            }
            for (field, (al, at)) in &projections[an] {
                let (bl, bt) = &projections[bn][field];
                let line = similarity::line_score(al, bl);
                let token = similarity::token_score(at, bt);
                if at.len() >= 8
                    && bt.len() >= 8
                    && (line >= LINE_FAIL_THRESHOLD || token >= TOKEN_WARN_THRESHOLD)
                {
                    warn(
                        d,
                        "CONTRACT_SIMILARITY_WARN",
                        format!(
                            "{an} and {bn} similar {field} contracts (line={line:.3}, token={token:.3})"
                        ),
                    )
                }
                for line in al.iter().filter(|l| bl.contains(l)) {
                    if prose::tokens(line).len() >= 8 {
                        repeated
                            .entry((field.clone(), line.clone()))
                            .or_default()
                            .extend([an.clone(), bn.clone()]);
                    }
                }
            }
        }
    }
    for ((field, line), owners) in repeated {
        warn(
            d,
            "CONTRACT_REPEATED_BLOCK",
            format!(
                "repeated {field} across {}: {line}",
                owners.into_iter().collect::<Vec<_>>().join(", ")
            ),
        )
    }
}
pub fn invariants(
    plan: &Value,
    skills: &SkillMap,
    ledger: &super::evidence::Ledger,
    d: &mut Vec<Diagnostic>,
) {
    let mut seen = BTreeSet::new();
    if !plan["critical_invariants"].is_array() {
        diag(
            d,
            "CRITICAL_INVARIANTS_INVALID",
            "critical_invariants must be an array",
        )
    }
    for (i, v) in arr(&plan["critical_invariants"]).iter().enumerate() {
        if !exact(
            v,
            &[
                "id",
                "statement",
                "evidence_ids",
                "skill_names",
                "assertions",
            ],
        ) || !ne(&v["id"])
            || !full(OWNERSHIP_ID_PATTERN, s(&v["id"]))
            || !ne(&v["statement"])
            || !sl(&v["evidence_ids"], false)
            || !sl(&v["skill_names"], false)
            || arr(&v["assertions"]).is_empty()
        {
            diag(
                d,
                "CRITICAL_INVARIANT_INVALID",
                format!("critical_invariants[{i}] invalid"),
            );
            continue;
        }
        let id = s(&v["id"]);
        if !seen.insert(id) {
            diag(
                d,
                "CRITICAL_INVARIANT_DUPLICATE",
                format!("duplicate invariant {id}"),
            )
        }
        check_ref(
            d,
            "CRITICAL_INVARIANT_EVIDENCE_UNKNOWN",
            id,
            &v["evidence_ids"],
            &ledger.locations.keys().cloned().collect(),
        );
        let evidence_ids = set(&v["evidence_ids"]);
        let claims = skills
            .values()
            .flat_map(|skill| arr(&skill["evidence_anchors"]))
            .filter(|anchor| evidence_ids.contains(s(&anchor["evidence_id"])))
            .map(|anchor| s(&anchor["claim"]))
            .collect::<Vec<_>>()
            .join(" ");
        if !prose::matches(&v["statement"], &claims) {
            diag(
                d,
                "CRITICAL_INVARIANT_CLAIM_TRACE",
                format!("{id} statement not supported by cited claims"),
            )
        }
        for assertion in arr(&v["assertions"]) {
            if !exact(assertion, &["skill_name", "verification_id"])
                || !ne(&assertion["skill_name"])
                || !ne(&assertion["verification_id"])
            {
                diag(
                    d,
                    "CRITICAL_INVARIANT_ASSERTION_INVALID",
                    format!("{id} assertion must name skill and verification"),
                )
            }
        }
        for name in ss(&v["skill_names"]) {
            let Some(skill) = skills.get(&name) else {
                diag(
                    d,
                    "CRITICAL_INVARIANT_SKILL_UNKNOWN",
                    format!("{id} unknown skill {name}"),
                );
                continue;
            };
            if !set(&v["evidence_ids"]).is_subset(&set(&skill["evidence_ids"])) {
                diag(
                    d,
                    "CRITICAL_INVARIANT_SKILL_EVIDENCE",
                    format!("{id} evidence not declared by {name}"),
                )
            }
            let assertions = arr(&v["assertions"])
                .iter()
                .filter(|a| s(&a["skill_name"]) == name)
                .collect::<Vec<_>>();
            if assertions.is_empty() {
                diag(
                    d,
                    "CRITICAL_INVARIANT_ASSERTION_MISSING",
                    format!("{id} has no assertion in {name}"),
                )
            }
            let ids = ids(&skill["verification"]);
            for a in assertions {
                if !ids.contains(s(&a["verification_id"])) {
                    diag(
                        d,
                        "CRITICAL_INVARIANT_ASSERTION_MISSING",
                        format!("{id} unknown check {} in {name}", s(&a["verification_id"])),
                    )
                }
            }
            if !prose::matches(&v["statement"], &flat(&skill["procedure_steps"]).join(" "))
                || !prose::matches(&v["statement"], &flat(&skill["verification"]).join(" "))
            {
                diag(
                    d,
                    "CRITICAL_INVARIANT_COVERAGE",
                    format!("{id} does not trace through {name} procedure and verification"),
                )
            }
        }
    }
    for (name, skill) in skills {
        for case in arr(&skill["routing_cases"]) {
            let primary = s(&case["expected_primary"]);
            if skills
                .get(primary)
                .is_some_and(|s| s["kind"] == RUNTIME_FIXED_KIND)
            {
                continue;
            }
            let spoken = primary.replace(['-', '_'], " ").to_lowercase();
            let prompt = s(&case["prompt"]).replace(['-', '_'], " ").to_lowercase();
            let tokens = prose::meaningful(&spoken);
            if !spoken.is_empty()
                && (prompt.contains(&spoken)
                    || (!tokens.is_empty() && tokens.is_subset(&prose::meaningful(&prompt))))
            {
                diag(
                    d,
                    "ROUTING_CASE_TAUTOLOGICAL",
                    format!("{name} routing prompt gives away expected owner {primary}"),
                )
            }
        }
    }
    for error in crate::flows::validate_plan_graph(plan) {
        diag(d, "FLOW_CONTRACT_INVALID", error)
    }
}
