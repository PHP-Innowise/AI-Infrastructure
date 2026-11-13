use crate::common::*;
use infra_core::fs;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
fn exact(value: &Value, names: &[&str]) -> bool {
    keys(value) == names.iter().map(|name| name.to_string()).collect()
}
fn answer(answer: &Value, fields: &[&str]) -> bool {
    exact(answer, fields)
        && matches!(text(&answer["verdict"]), "pass" | "fail")
        && nonempty(&answer["note"])
        && (!fields.contains(&"prompt") || nonempty(&answer["prompt"]))
}
const CONTENT_DIMENSIONS: &[&str] = &["uniqueness", "completeness", "accuracy", "coherence"];
const VERBATIM: &[&str] = &[
    "memory-bank/scripts/",
    "memory-bank/templates/",
    "project-brain/PROTOCOL.md",
    "project-brain/schemas/",
    "project-brain/templates/",
];
fn sanctioned(path: &str) -> bool {
    matches!(
        path,
        "memory-bank/bin/infra-engine"
            | "memory-bank/bin/infra-engine.exe"
            | "memory-bank/config/mirror-rules.json"
    ) || VERBATIM.iter().any(|prefix| {
        path == prefix.strip_suffix('/').unwrap_or(prefix) || path.starts_with(prefix)
    })
}
fn forge_owns(lane: &str, forge: &str) -> bool {
    match lane {
        "skills" => forge == "skill-forge",
        "wrappers" => ["agent-forge", "command-forge", "skill-flow-composer"].contains(&forge),
        "policy" => forge == "policy-forge",
        "hooks" => forge == "hook-forge",
        "memory" => forge == "memory-seed",
        "reports" => forge == "profile-synthesizer",
        _ => false,
    }
}
pub fn content_review_values(planned: &[String], review: &Value) -> Vec<Diagnostic> {
    let mut items = vec![];
    let rounds = review["rounds"].as_u64().unwrap_or_else(|| {
        diag(
            &mut items,
            "CONTENT_REVIEW_INVALID",
            "rounds must be a count >= 0",
        );
        0
    });
    let mut expected = [
        "task",
        "reviewer",
        "generation_root",
        "rounds",
        "files",
        "blockers",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    if rounds > 0 {
        expected.insert("post_repair_gates".into());
    }
    if keys(review) != expected {
        diag(
            &mut items,
            "CONTENT_REVIEW_INVALID",
            "record fields do not match required review shape",
        );
        return items;
    }
    if !nonempty(&review["task"]) || !nonempty(&review["generation_root"]) {
        diag(
            &mut items,
            "CONTENT_REVIEW_INVALID",
            "task and generation_root must name this run",
        );
    }
    if review["reviewer"] != "independent" {
        diag(
            &mut items,
            "CONTENT_REVIEW_NOT_INDEPENDENT",
            "reviewer did not declare independence",
        );
    }
    if rounds > 2 {
        diag(
            &mut items,
            "CONTENT_REVIEW_ROUNDS_EXCEEDED",
            "repair rounds exceed bound of 2",
        );
    }
    if rounds > 0 {
        let gates = texts(&review["post_repair_gates"]);
        if !["validate_skill_quality", "validate_generated"]
            .iter()
            .all(|gate| gates.iter().any(|value| value == gate))
        {
            diag(
                &mut items,
                "CONTENT_REVIEW_GATES_NOT_RERUN",
                "staged content repairs require both mechanical gates to be rerun",
            );
        }
    }
    if !string_list(&review["blockers"], true) {
        diag(
            &mut items,
            "CONTENT_REVIEW_INVALID",
            "blockers must be an array of statements",
        );
    }
    for blocker in texts(&review["blockers"]) {
        diag(
            &mut items,
            "CONTENT_REVIEW_BLOCKER_OPEN",
            format!("review left a blocker open: {blocker}"),
        );
    }
    let entries = review["files"].as_array().cloned().unwrap_or_else(|| {
        diag(
            &mut items,
            "CONTENT_REVIEW_INVALID",
            "files must be an array",
        );
        vec![]
    });
    let mut seen = BTreeSet::new();
    let planned = planned.iter().cloned().collect::<BTreeSet<_>>();
    let mut reports = BTreeSet::new();
    for entry in entries {
        let path = text(&entry["path"]).trim();
        let lane = text(&entry["lane"]);
        if path.is_empty() {
            diag(
                &mut items,
                "CONTENT_REVIEW_INVALID",
                "review entry must name its path",
            );
            continue;
        }
        if ![
            "skills",
            "wrappers",
            "policy",
            "hooks",
            "memory",
            "reports",
            "runtime-verbatim",
        ]
        .contains(&lane)
        {
            diag(
                &mut items,
                "CONTENT_REVIEW_LANE_UNKNOWN",
                format!("{path}: unknown lane {lane}"),
            );
            continue;
        }
        if lane == "reports" {
            reports.insert(path.rsplit('/').next().unwrap_or(path).to_string());
        } else if !planned.contains(path) {
            diag(
                &mut items,
                "CONTENT_REVIEW_FILE_UNKNOWN",
                format!("{path}: review covers an unplanned file"),
            );
            continue;
        }
        if !seen.insert(path.to_string()) {
            diag(
                &mut items,
                "CONTENT_REVIEW_FILE_DUPLICATE",
                format!("{path}: reviewed more than once"),
            );
            continue;
        }
        if lane == "runtime-verbatim" {
            if !sanctioned(path) {
                diag(
                    &mut items,
                    "CONTENT_REVIEW_VERBATIM_UNSANCTIONED",
                    format!("{path}: exemption outside sanctioned runtime surface"),
                );
            }
            if !nonempty(&entry["basis"]) {
                diag(
                    &mut items,
                    "CONTENT_REVIEW_INVALID",
                    format!("{path}: verbatim exemption needs basis"),
                );
            }
            continue;
        }
        if !exact(
            &entry,
            &[
                "path",
                "lane",
                "findings",
                "uniqueness",
                "completeness",
                "accuracy",
                "coherence",
            ],
        ) {
            diag(
                &mut items,
                "CONTENT_REVIEW_DIMENSION_MISSING",
                format!("{path}: review did not answer every dimension"),
            );
            continue;
        }
        for dimension in CONTENT_DIMENSIONS {
            let value = &entry[dimension];
            if !answer(value, &["verdict", "note"]) {
                diag(
                    &mut items,
                    "CONTENT_REVIEW_ANSWER_INVALID",
                    format!("{path}.{dimension}: expected verdict and substantive note"),
                );
            } else if value["verdict"] == "fail" {
                diag(
                    &mut items,
                    "CONTENT_REVIEW_VERDICT_FAILED",
                    format!("{path}.{dimension}: {}", text(&value["note"])),
                );
            }
        }
        let findings = entry["findings"].as_array().cloned().unwrap_or_else(|| {
            diag(
                &mut items,
                "CONTENT_REVIEW_INVALID",
                format!("{path}.findings must be array"),
            );
            vec![]
        });
        for (index, finding) in findings.iter().enumerate() {
            let label = format!("{path}.findings[{index}]");
            if !finding.is_object()
                || !CONTENT_DIMENSIONS.contains(&text(&finding["dimension"]))
                || !["blocking", "advisory"].contains(&text(&finding["severity"]))
                || !nonempty(&finding["statement"])
                || !finding["resolution"].is_object()
            {
                diag(
                    &mut items,
                    "CONTENT_REVIEW_FINDING_INVALID",
                    format!("{label}: invalid finding"),
                );
                continue;
            }
            let resolution = &finding["resolution"];
            let action = text(&resolution["action"]);
            if !["reforged", "escalated", "accepted"].contains(&action)
                || !nonempty(&resolution["note"])
            {
                diag(
                    &mut items,
                    "CONTENT_REVIEW_FINDING_INVALID",
                    format!("{label}: invalid resolution"),
                );
                continue;
            }
            if action == "accepted" && finding["severity"] == "blocking" {
                diag(
                    &mut items,
                    "CONTENT_REVIEW_BLOCKER_ACCEPTED",
                    format!("{label}: blocking finding cannot be accepted"),
                );
            }
            if action == "escalated" {
                diag(
                    &mut items,
                    "CONTENT_REVIEW_ESCALATION_OPEN",
                    format!("{label}: publication waits for human decision"),
                );
            }
            if action == "reforged" {
                if !forge_owns(lane, text(&resolution["forge"])) {
                    diag(
                        &mut items,
                        "CONTENT_REVIEW_FORGE_MISMATCH",
                        format!("{label}: repair forge does not own {lane}"),
                    );
                }
                if !resolution["round"]
                    .as_u64()
                    .is_some_and(|value| value > 0 && value <= rounds)
                {
                    diag(
                        &mut items,
                        "CONTENT_REVIEW_FINDING_INVALID",
                        format!("{label}: repair round outside 1..{rounds}"),
                    );
                }
            }
        }
    }
    for path in planned.difference(&seen) {
        diag(
            &mut items,
            "CONTENT_REVIEW_FILE_MISSING",
            format!("{path}: published but never reviewed"),
        );
    }
    for report in [
        "infra-scan-project-profile.md",
        "skill-generation-plan.json",
    ] {
        if !reports.contains(report) {
            diag(
                &mut items,
                "CONTENT_REVIEW_REPORTS_MISSING",
                format!("review never walked {report}"),
            );
        }
    }
    items
}
pub fn content_review(plan: &Path, review: &Path) -> Vec<Diagnostic> {
    let mut items = vec![];
    let paths = match fs::read_text(plan) {
        Ok(value) => value
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>(),
        Err(error) => {
            diag(
                &mut items,
                "CONTENT_REVIEW_PLAN_UNREADABLE",
                error.to_string(),
            );
            return items;
        }
    };
    if let Some(review) = load_diagnostic(review, "CONTENT_REVIEW", &mut items) {
        items.extend(content_review_values(&paths, &review));
    }
    items
}
const REQUESTS: &[&str] = &[
    "unseen_positive_request",
    "sibling_request",
    "ambiguous_request",
    "cross_domain_request",
];
const JUDGED: &[&str] = &[
    "catalog_role_coverage",
    "refusal_branch",
    "verification_realism",
    "identity_erasure",
];
fn tautological(prompt: &str, name: &str) -> bool {
    let spoken = name.replace(['-', '_'], " ").to_lowercase();
    let said = prompt.replace(['-', '_'], " ").to_lowercase();
    if said.contains(&spoken) {
        return true;
    }
    let words = word_tokens(&spoken)
        .into_iter()
        .filter(|word| word.chars().count() >= 3)
        .collect::<BTreeSet<_>>();
    !words.is_empty() && words.is_subset(&word_tokens(&said))
}
pub fn plan_review_values(
    plan: &Value,
    review: &Value,
    registry: Option<&Value>,
) -> Vec<Diagnostic> {
    let mut items = vec![];
    if !exact(
        review,
        &["plan", "reviewer", "skills", "rejected", "blockers"],
    ) {
        diag(
            &mut items,
            "REVIEW_INVALID",
            "review must define exactly plan, reviewer, skills, rejected and blockers",
        );
        return items;
    }
    if review["reviewer"] != "independent" {
        diag(
            &mut items,
            "REVIEW_NOT_INDEPENDENT",
            "review must be independent of plan author",
        );
    }
    if !string_list(&review["blockers"], true) {
        diag(
            &mut items,
            "REVIEW_INVALID",
            "blockers must be an array of statements",
        );
    }
    for blocker in texts(&review["blockers"]) {
        diag(
            &mut items,
            "REVIEW_BLOCKER_OPEN",
            format!("review left blocker open: {blocker}"),
        );
    }
    let planned = plan["skills"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|skill| nonempty(&skill["name"]))
        .map(|skill| (text(&skill["name"]).trim().to_string(), skill))
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    if !review["skills"].is_array() {
        diag(&mut items, "REVIEW_INVALID", "skills must be array");
    }
    for entry in review["skills"].as_array().into_iter().flatten() {
        let name = text(&entry["name"]).trim();
        if name.is_empty() {
            diag(
                &mut items,
                "REVIEW_INVALID",
                "reviewed skill must have name",
            );
            continue;
        }
        let Some(skill) = planned.get(name) else {
            diag(
                &mut items,
                "REVIEW_SKILL_UNKNOWN",
                format!("{name}: not selected by plan"),
            );
            continue;
        };
        if !seen.insert(name.to_string()) {
            diag(
                &mut items,
                "REVIEW_SKILL_DUPLICATE",
                format!("{name}: reviewed more than once"),
            );
            continue;
        }
        let expected = std::iter::once("name")
            .chain(REQUESTS.iter().copied())
            .chain(JUDGED.iter().copied())
            .map(str::to_string)
            .collect::<BTreeSet<_>>();
        if keys(entry) != expected {
            diag(
                &mut items,
                "REVIEW_DIMENSION_MISSING",
                format!("{name}: missing dimensions or unexpected fields"),
            );
            continue;
        }
        for dimension in REQUESTS.iter().chain(JUDGED) {
            let value = &entry[dimension];
            let fields: &[&str] = if REQUESTS.contains(dimension) {
                &["verdict", "prompt", "note"]
            } else if *dimension == "verification_realism" {
                &["verdict", "note", "executed"]
            } else {
                &["verdict", "note"]
            };
            if !answer(value, fields) {
                diag(
                    &mut items,
                    "REVIEW_ANSWER_INVALID",
                    format!("{name}.{dimension}: invalid answer"),
                );
                continue;
            }
            if value["verdict"] == "fail" {
                diag(
                    &mut items,
                    "REVIEW_VERDICT_FAILED",
                    format!("{name}.{dimension}: {}", text(&value["note"])),
                );
            }
            if REQUESTS.contains(dimension) && tautological(text(&value["prompt"]), name) {
                diag(
                    &mut items,
                    "REVIEW_PROMPT_TAUTOLOGICAL",
                    format!("{name}.{dimension}: fixture names its answer"),
                );
            }
        }
        let realism = &entry["verification_realism"];
        if exact(realism, &["verdict", "note", "executed"]) {
            if !string_list(&realism["executed"], true) {
                diag(
                    &mut items,
                    "REVIEW_ANSWER_INVALID",
                    format!("{name}: executed must be command list"),
                );
            } else {
                let ran = texts(&realism["executed"])
                    .into_iter()
                    .map(|value| value.trim().to_string())
                    .collect::<BTreeSet<_>>();
                let prescribed = skill["verification"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|check| check["command"].as_str())
                    .map(|value| value.trim().to_string())
                    .collect::<BTreeSet<_>>();
                for command in prescribed.difference(&ran) {
                    diag(
                        &mut items,
                        "REVIEW_VERIFICATION_NOT_EXECUTED",
                        format!("{name}: prescribed command not executed: {command}"),
                    );
                }
                for command in ran.difference(&prescribed) {
                    diag(
                        &mut items,
                        "REVIEW_VERIFICATION_UNPLANNED",
                        format!("{name}: review executed unplanned command: {command}"),
                    );
                }
            }
        }
    }
    for name in planned.keys().filter(|name| !seen.contains(*name)) {
        diag(
            &mut items,
            "REVIEW_SKILL_MISSING",
            format!("{name}: selected but not reviewed"),
        );
    }
    let rejected = plan["rejected_candidates"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| nonempty(&entry["name"]))
        .map(|entry| {
            (
                text(&entry["name"]).trim().to_string(),
                text(&entry["candidate_id"]).trim().to_string(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let flagged = registry
        .into_iter()
        .flat_map(|registry| registry["candidates"].as_array().into_iter().flatten())
        .filter(|entry| entry["escalates_on_rejection"] == true)
        .filter_map(|entry| entry["id"].as_str())
        .collect::<BTreeSet<_>>();
    let mut reviewed = BTreeSet::new();
    if !review["rejected"].is_array() {
        diag(&mut items, "REVIEW_INVALID", "rejected must be an array");
    }
    for entry in review["rejected"].as_array().into_iter().flatten() {
        let name = text(&entry["name"]).trim();
        if name.is_empty() {
            diag(
                &mut items,
                "REJECTION_ANSWER_INVALID",
                "rejected review must name candidate",
            );
            continue;
        }
        if !rejected.contains_key(name) {
            diag(
                &mut items,
                "REJECTION_REVIEW_UNKNOWN",
                format!("{name}: plan does not reject candidate"),
            );
            continue;
        }
        if !reviewed.insert(name.to_string()) {
            diag(
                &mut items,
                "REJECTION_REVIEW_DUPLICATE",
                format!("{name}: reviewed more than once"),
            );
            continue;
        }
        let class = text(&entry["classification"]);
        let extra: &[&str] = match class {
            "consolidated" => &["absorbed_by"],
            "insufficient-evidence" => &["narrow_scope", "interview_reference"],
            "unresolved-safety" => &["narrow_scope", "decision_reference"],
            "not-applicable" => &[],
            _ => {
                diag(
                    &mut items,
                    "REJECTION_ANSWER_INVALID",
                    format!("{name}: unknown classification"),
                );
                continue;
            }
        };
        let expected = ["name", "classification", "note"]
            .into_iter()
            .chain(extra.iter().copied())
            .map(str::to_string)
            .collect::<BTreeSet<_>>();
        if keys(entry) != expected || !nonempty(&entry["note"]) {
            diag(
                &mut items,
                "REJECTION_ANSWER_INVALID",
                format!("{name}: invalid rejection fields or note"),
            );
            continue;
        }
        if class == "consolidated" {
            if !string_list(&entry["absorbed_by"], false) {
                diag(
                    &mut items,
                    "REJECTION_ANSWER_INVALID",
                    format!("{name}: consolidated rejection needs absorbing skills"),
                );
            }
            for owner in texts(&entry["absorbed_by"]) {
                if !planned.contains_key(owner.trim()) {
                    diag(
                        &mut items,
                        "REJECTION_CONSOLIDATION_PHANTOM",
                        format!("{name}: absorber {owner} not selected"),
                    );
                }
            }
        }
        if ["insufficient-evidence", "unresolved-safety"].contains(&class) {
            if !nonempty(&entry["narrow_scope"]) {
                diag(
                    &mut items,
                    "REJECTION_ANSWER_INVALID",
                    format!("{name}: rejection needs narrow-scope justification"),
                );
            }
            let (field, code) = if class == "insufficient-evidence" {
                ("interview_reference", "REJECTION_NOT_ESCALATED")
            } else {
                ("decision_reference", "REJECTION_SAFETY_UNRESOLVED")
            };
            if !nonempty(&entry[field]) {
                diag(
                    &mut items,
                    code,
                    format!("{name}: rejection lacks recorded human reference"),
                );
            }
        }
    }
    for (name, candidate) in rejected {
        if flagged.contains(candidate.as_str()) && !reviewed.contains(&name) {
            diag(
                &mut items,
                "REJECTION_REVIEW_MISSING",
                format!("{name}: risk-flagged rejection was never reviewed"),
            );
        }
    }
    items
}
pub fn plan_review(plan: &Path, review: &Path, registry: Option<&Path>) -> Vec<Diagnostic> {
    let mut items = vec![];
    let Some(plan) = load_diagnostic(plan, "REVIEW", &mut items) else {
        return items;
    };
    let Some(review) = load_diagnostic(review, "REVIEW", &mut items) else {
        return items;
    };
    let registry = if let Some(path) = registry {
        match load_diagnostic(path, "REGISTRY", &mut items) {
            Some(value) if value["candidates"].is_array() => Some(value),
            Some(_) => {
                diag(
                    &mut items,
                    "REGISTRY_UNREADABLE",
                    "registry lists no candidates",
                );
                return items;
            }
            None => return items,
        }
    } else {
        None
    };
    items.extend(plan_review_values(&plan, &review, registry.as_ref()));
    items
}
