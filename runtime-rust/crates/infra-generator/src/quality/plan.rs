use super::{
    consts::*,
    contracts,
    evidence::{self, Ledger},
    inventory, paths, prose, registry, rejections,
    util::*,
};
use crate::common::{Diagnostic, diag, warn};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
pub struct PlanState {
    pub skills: SkillMap,
    pub ledger: Ledger,
}
pub fn expand_user(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if (text == "~" || text.starts_with("~/"))
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(text.strip_prefix("~/").unwrap_or(""));
    }
    path.to_owned()
}
pub fn absolute(path: &Path) -> PathBuf {
    let p = expand_user(path);
    if let Ok(p) = infra_core::fs::canonicalize(&p) {
        p
    } else if p.is_absolute() {
        p
    } else {
        std::env::current_dir().unwrap_or_default().join(p)
    }
}
pub fn validate(
    plan: &Value,
    plan_path: &Path,
    target: &Path,
    registry_path: &Path,
    d: &mut Vec<Diagnostic>,
) -> PlanState {
    let empty = || PlanState {
        skills: SkillMap::new(),
        ledger: Ledger::default(),
    };
    let schema = s(&plan["schema_version"]);
    let required = if schema == "1.6" {
        SCHEMA_1_6_PLAN_FIELDS
    } else if typed(schema) {
        SCHEMA_1_2_PLAN_FIELDS
    } else {
        LEGACY_PLAN_FIELDS
    };
    for field in required {
        if plan.get(*field).is_none() {
            diag(
                d,
                "PLAN_FIELD_MISSING",
                format!("plan missing top-level field: {field}"),
            )
        }
    }
    if !d.is_empty() {
        return empty();
    }
    if let Some(map) = plan.as_object() {
        for field in map.keys() {
            if !required.contains(&field.as_str()) {
                diag(
                    d,
                    "PLAN_FIELD_UNKNOWN",
                    format!("unknown top-level plan field: {field}"),
                )
            }
        }
    }
    if !SUPPORTED_PLAN_SCHEMAS.contains(&schema) {
        diag(
            d,
            "PLAN_SCHEMA_VERSION",
            format!(
                "schema_version must be one of {}",
                SUPPORTED_PLAN_SCHEMAS.join(", ")
            ),
        )
    } else if LEGACY_PLAN_SCHEMAS.contains(&schema) {
        warn(
            d,
            "PLAN_SCHEMA_MIGRATION",
            format!("schema {schema} is audit-only; new plans require {CURRENT_PLAN_SCHEMA}"),
        )
    }
    if !ne(&plan["catalog_version"]) {
        diag(
            d,
            "PLAN_CATALOG_VERSION",
            "catalog_version must be a nonempty reference-corpus version",
        )
    }
    let candidates = registry::load(registry_path, &plan["catalog_version"], d);
    let target = absolute(target);
    if !ne(&plan["target_root"]) {
        diag(d, "PLAN_TARGET_ROOT", "target_root must be nonempty")
    } else {
        let value = expand_user(Path::new(s(&plan["target_root"])));
        let declared = if value.is_absolute() {
            absolute(&value)
        } else {
            absolute(&target.join(value))
        };
        if declared != target {
            diag(
                d,
                "PLAN_TARGET_MISMATCH",
                format!(
                    "target_root differs from --target: {}",
                    s(&plan["target_root"])
                ),
            )
        }
    }
    let profile = s(&plan["profile"]);
    let parts = Path::new(profile).components().collect::<Vec<_>>();
    let parent = plan_path.parent().unwrap_or(Path::new("."));
    let expected_task = parent.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if !safe(profile)
        || parts.len() != 3
        || profile != format!("tasks/{expected_task}/infra-scan-project-profile.md")
        || !parent.join("infra-scan-project-profile.md").is_file()
    {
        diag(
            d,
            "PROFILE_INVALID",
            format!("profile must resolve beside generation plan: {profile}"),
        )
    }
    let ledger = evidence::validate(plan, &target, schema, d);
    let (rejected_names, rejected_candidates) =
        rejections::validate(plan, &candidates, &target, schema, d);
    let teams = if schema == "1.6" {
        inventory::preexisting(&plan["preexisting_team_skills"], &target, d)
    } else {
        SkillMap::new()
    };
    let aliases = teams
        .values()
        .map(|v| (s(&v["generated_alias"]).to_owned(), v))
        .collect::<BTreeMap<_, _>>();
    let mut skills = SkillMap::new();
    if arr(&plan["skills"]).is_empty() {
        diag(d, "SKILL_PLAN_EMPTY", "skills must be a non-empty array");
        return PlanState { skills, ledger };
    }
    for (index, skill) in arr(&plan["skills"]).iter().enumerate() {
        if !skill.is_object() {
            diag(
                d,
                "SKILL_PLAN_INVALID",
                format!("skills[{index}] must be an object"),
            );
            continue;
        }
        let fields = if disposed(schema) {
            SCHEMA_1_4_SKILL_FIELDS
        } else if typed(schema) {
            SCHEMA_1_2_SKILL_FIELDS
        } else {
            REQUIRED_SKILL_FIELDS
        };
        let missing = fields
            .iter()
            .filter(|f| skill.get(**f).is_none())
            .copied()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            diag(
                d,
                "SKILL_PLAN_INCOMPLETE",
                format!("skills[{index}] missing: {}", missing.join(", ")),
            );
            continue;
        }
        if !ne(&skill["name"]) {
            diag(
                d,
                "SKILL_NAME_INVALID",
                format!("skills[{index}] has invalid name"),
            );
            continue;
        }
        let name = s(&skill["name"]).trim();
        if skills.contains_key(name) {
            diag(
                d,
                "SKILL_NAME_DUPLICATE",
                format!("duplicate skill name: {name}"),
            );
            continue;
        }
        let mut structural = true;
        for field in ["category", "kind", "phase", "necessity_rationale"] {
            if !ne(&skill[field]) {
                diag(
                    d,
                    "SKILL_PLAN_VALUE",
                    format!("{name}.{field} must be substantive"),
                )
            }
        }
        let triggers = &skill["triggers"];
        if !triggers.is_object()
            || !sl(&triggers["positive"], false)
            || !sl(&triggers["negative"], false)
        {
            diag(
                d,
                "SKILL_TRIGGERS_INVALID",
                format!("{name}.triggers requires positive and negative arrays"),
            );
            if !triggers.is_object()
                || !triggers["positive"].is_array()
                || !triggers["negative"].is_array()
            {
                structural = false
            }
        }
        let runtime = s(&skill["kind"]).to_lowercase() == RUNTIME_FIXED_KIND;
        for field in ["evidence_ids", "source_paths", "writes", "related_skills"] {
            let allow =
                field == "writes" || (runtime && ["evidence_ids", "source_paths"].contains(&field));
            if !sl(&skill[field], allow) {
                diag(
                    d,
                    "SKILL_PLAN_VALUE",
                    format!("{name}.{field} must be a string array"),
                );
                if !sl(&skill[field], true) {
                    structural = false
                }
            }
        }
        for field in [
            "owned_scope",
            "excluded_scope",
            "required_procedure_roles",
            "decision_points",
            "verification",
            "output_contract",
            "failure_handling",
            "nearest_siblings",
        ] {
            if !substantive(&skill[field]) {
                diag(
                    d,
                    "SKILL_PLAN_VALUE",
                    format!("{name}.{field} must be substantive"),
                );
                if !skill[field].is_array() && !skill[field].is_string() {
                    structural = false
                }
            }
        }
        if let Some(blocks) = skill.get("fixed_blocks")
            && (!blocks.is_array() || fixed(blocks).len() != arr(blocks).len())
        {
            diag(
                d,
                "SKILL_FIXED_BLOCKS_INVALID",
                format!("{name}.fixed_blocks must contain versioned id/version/content objects"),
            )
        }
        let gate = &skill["selection_gate"];
        if !exact(
            gate,
            &[
                "catalog",
                "candidate_id",
                "candidate",
                "conditions",
                "distinct_value_from",
            ],
        ) {
            diag(
                d,
                "SKILL_SELECTION_GATE",
                format!("{name}.selection_gate has invalid fields"),
            );
            if !gate.is_object() {
                structural = false
            }
        } else if structural {
            let id = s(&gate["candidate_id"]);
            let candidate = candidates.get(id);
            if let Some(candidate) = candidate {
                let mode = s(&candidate["mode"]);
                let alias_ok = schema == "1.6"
                    && aliases.get(name).is_some_and(|r| {
                        s(&r["candidate_id"]) == id
                            && s(&r["name"]) == id
                            && skill["preexisting_team_replacement"] == r["name"]
                    });
                if gate["catalog"] != candidate["catalog"]
                    || s(&skill["category"]).to_lowercase()
                        != s(&candidate["category"]).to_lowercase()
                    || (["static", "runtime-fixed"].contains(&mode) && id != name && !alias_ok)
                    || (mode == "runtime-fixed" && !runtime)
                {
                    diag(
                        d,
                        "SKILL_SELECTION_REGISTRY_MISMATCH",
                        format!("{name} selection gate differs from registry"),
                    )
                }
                registry::roles(name, skill, candidate, d);
            } else {
                diag(
                    d,
                    "SKILL_SELECTION_CANDIDATE_UNKNOWN",
                    format!("{name} references unknown registry candidate"),
                )
            }
            if !ne(&gate["catalog"])
                || !ne(&gate["candidate_id"])
                || s(&gate["candidate"]) != name
                || !sl(&gate["distinct_value_from"], true)
                || arr(&gate["conditions"]).is_empty()
            {
                diag(
                    d,
                    "SKILL_SELECTION_GATE",
                    format!("{name}.selection_gate is incomplete"),
                )
            } else {
                rejections::absent_golden(name, skill, &candidates, &ledger.locations, d);
                for condition in arr(&gate["conditions"]) {
                    if !exact(
                        condition,
                        &["requirement", "evidence_ids", "status", "explanation"],
                    ) || !ne(&condition["requirement"])
                        || !SELECTION_CONDITION_STATUSES.contains(&s(&condition["status"]))
                        || !ne(&condition["explanation"])
                        || !sl(&condition["evidence_ids"], runtime)
                    {
                        diag(
                            d,
                            "SKILL_SELECTION_CONDITION",
                            format!("{name} has incomplete selection condition"),
                        );
                        continue;
                    }
                    check_ref(
                        d,
                        "SKILL_SELECTION_EVIDENCE",
                        name,
                        &condition["evidence_ids"],
                        &set(&skill["evidence_ids"]),
                    );
                    let claims = ss(&condition["evidence_ids"])
                        .iter()
                        .filter_map(|id| ledger.claims.get(id))
                        .flatten()
                        .cloned()
                        .collect::<Vec<_>>();
                    if s(&condition["status"]) != "absent-golden"
                        && !claims.is_empty()
                        && !prose::matches(
                            &serde_json::json!(claims),
                            &format!(
                                "{} {} {}",
                                s(&condition["requirement"]),
                                s(&condition["explanation"]),
                                s(&skill["necessity_rationale"])
                            ),
                        )
                    {
                        diag(
                            d,
                            "SKILL_SELECTION_CLAIM_TRACE",
                            format!("{name} selection condition not supported by cited claims"),
                        )
                    }
                }
            }
        }
        if !structural {
            continue;
        }
        check_ref(
            d,
            "SKILL_EVIDENCE_UNKNOWN",
            name,
            &skill["evidence_ids"],
            &ledger.locations.keys().cloned().collect(),
        );
        for source in ss(&skill["source_paths"]) {
            if !confined(&target, &source).is_some_and(|p| p.exists()) {
                diag(
                    d,
                    "SKILL_SOURCE_INVALID",
                    format!("{name} source unsafe or missing: {source}"),
                )
            }
            if has(r"(?i)(^|/)tasks/TASK-[^/]+/", &source) {
                diag(
                    d,
                    "SKILL_SOURCE_TASK_PATH",
                    format!("{name} source points to generator runtime: {source}"),
                )
            }
        }
        for write in ss(&skill["writes"]) {
            if !safe(&write) || paths::normalize_glob(&write) != write {
                diag(
                    d,
                    "SKILL_WRITE_INVALID",
                    format!("{name} write must be normalized and target-relative: {write}"),
                )
            }
        }
        if typed(schema) {
            contracts::skill(schema, name, skill, &ledger, &target, d);
            if [
                "procedure_steps",
                "path_contracts",
                "evidence_anchors",
                "routing_cases",
            ]
            .iter()
            .any(|k| !skill[*k].is_array())
            {
                continue;
            }
        }
        skills.insert(name.to_owned(), skill.clone());
    }
    if schema == "1.6" {
        for (original, record) in &teams {
            let alias = s(&record["generated_alias"]);
            if skills.contains_key(original) {
                diag(
                    d,
                    "PREEXISTING_TEAM_SKILL_COLLISION",
                    format!("{original} remains selected at protected team name"),
                )
            }
            match skills.get(alias) {
                None => diag(
                    d,
                    "PREEXISTING_TEAM_SKILL_REPLACEMENT_MISSING",
                    format!("{original} has no generated replacement contract {alias}"),
                ),
                Some(replacement)
                    if s(&replacement["preexisting_team_replacement"]) != original
                        || replacement["selection_gate"]["candidate_id"]
                            != record["candidate_id"] =>
                {
                    diag(
                        d,
                        "PREEXISTING_TEAM_SKILL_REPLACEMENT_INVALID",
                        format!("{alias} does not explicitly replace {original}"),
                    )
                }
                _ => {}
            }
        }
        for (name, skill) in &skills {
            if let Some(replacement) = skill.get("preexisting_team_replacement")
                && (!ne(replacement)
                    || !teams
                        .get(s(replacement))
                        .is_some_and(|r| s(&r["generated_alias"]) == name))
            {
                diag(
                    d,
                    "PREEXISTING_TEAM_SKILL_REPLACEMENT_INVALID",
                    format!("{name} names unknown or mismatched team replacement"),
                )
            }
        }
    }
    let selected = skills
        .values()
        .filter_map(|v| v["selection_gate"]["candidate_id"].as_str())
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut counts = BTreeMap::new();
    for id in selected.iter().chain(rejected_candidates.iter()) {
        *counts.entry(id.clone()).or_insert(0usize) += 1
    }
    let selected_set = selected.into_iter().collect::<BTreeSet<_>>();
    for id in selected_set.intersection(&rejected_candidates.into_iter().collect()) {
        diag(
            d,
            "INVENTORY_CANDIDATE_DUAL_DISPOSITION",
            format!("candidate selected and rejected: {id}"),
        )
    }
    for (id, candidate) in &candidates {
        let count = counts.get(id).copied().unwrap_or(0);
        if count == 0 {
            diag(
                d,
                "INVENTORY_CANDIDATE_UNACCOUNTED",
                format!("registry candidate neither selected nor rejected: {id}"),
            )
        } else if s(&candidate["mode"]) != "family" && count != 1 {
            diag(
                d,
                "INVENTORY_CANDIDATE_DUPLICATE",
                format!("non-family {id} appears {count} times"),
            )
        }
    }
    for id in counts.keys().filter(|id| !candidates.contains_key(*id)) {
        diag(
            d,
            "INVENTORY_CANDIDATE_UNKNOWN",
            format!("plan accounts for unknown candidate {id}"),
        )
    }
    for name in skills.keys().filter(|n| rejected_names.contains(*n)) {
        diag(
            d,
            "INVENTORY_SELECTED_AND_REJECTED",
            format!("candidate selected and rejected: {name}"),
        )
    }
    for (name, skill) in &skills {
        let siblings = ss(&skill["related_skills"])
            .into_iter()
            .chain(
                arr(&skill["nearest_siblings"])
                    .iter()
                    .map(|v| s(&v["name"]).to_owned()),
            )
            .collect::<Vec<_>>();
        for sibling in siblings {
            if sibling == *name || !skills.contains_key(&sibling) {
                diag(
                    d,
                    "SKILL_SIBLING_UNKNOWN",
                    format!("{name} unresolved sibling: {sibling}"),
                )
            }
        }
        for other in ss(&skill["selection_gate"]["distinct_value_from"]) {
            if !skills.contains_key(&other) && !rejected_names.contains(&other) {
                diag(
                    d,
                    "SKILL_SELECTION_BOUNDARY_UNKNOWN",
                    format!("{name} distinguishes unknown candidate: {other}"),
                )
            }
        }
    }
    if typed(schema) {
        inventory::invariants(plan, &skills, &ledger, d)
    }
    if SUPPORTED_PLAN_SCHEMAS.contains(&schema) {
        inventory::validate(&mut skills, schema, d)
    }
    PlanState { skills, ledger }
}
