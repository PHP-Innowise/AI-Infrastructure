use super::{
    consts::*,
    evidence::{self, Ledger},
    paths, prose, runtime, search,
    util::*,
};
use crate::common::{Diagnostic, diag};
use serde_json::Value;
use std::{collections::BTreeSet, path::Path};
fn invalid(d: &mut Vec<Diagnostic>, code: &str, name: &str, field: &str, index: usize) {
    diag(
        d,
        code,
        format!("{name}.{field}[{index}] has invalid typed fields"),
    )
}
pub fn skill(
    schema: &str,
    name: &str,
    skill: &Value,
    ledger: &Ledger,
    target: &Path,
    d: &mut Vec<Diagnostic>,
) {
    let runtime_fixed = s(&skill["kind"]).to_lowercase() == RUNTIME_FIXED_KIND;
    let capability = &skill["capability"];
    let mode = s(&capability["mode"]);
    if !exact(capability, &["mode", "summary"])
        || !CAPABILITY_MODES.contains(&mode)
        || !ne(&capability["summary"])
    {
        diag(
            d,
            "SKILL_CAPABILITY_INVALID",
            format!("{name}.capability must define mode and summary"),
        )
    } else if mode == "read-only" {
        if !arr(&skill["writes"]).is_empty() {
            diag(
                d,
                "READ_ONLY_WRITE_CONFLICT",
                format!("{name} is read-only but declares writes"),
            )
        }
        let language = arr(&skill["procedure_steps"])
            .iter()
            .map(|v| s(&v["action"]))
            .collect::<Vec<_>>()
            .join(" ");
        if has(READ_ONLY_MUTATION_PATTERN, &language) {
            diag(
                d,
                "READ_ONLY_PROCEDURE_MUTATION",
                format!("{name} read-only procedure contains write language"),
            )
        }
    } else if mode == "workspace-write" && arr(&skill["writes"]).is_empty() {
        diag(
            d,
            "WRITE_CAPABILITY_WITHOUT_PATH",
            format!("{name} has workspace-write capability without paths"),
        )
    }
    let mut decisions = BTreeSet::new();
    for (i, v) in arr(&skill["decision_points"]).iter().enumerate() {
        if !exact(v, &["id", "question", "branches"])
            || !ne(&v["id"])
            || !full(OWNERSHIP_ID_PATTERN, s(&v["id"]))
            || !ne(&v["question"])
            || !sl(&v["branches"], false)
        {
            invalid(d, "DECISION_POINT_INVALID", name, "decision_points", i);
            continue;
        }
        if !decisions.insert(s(&v["id"]).to_owned()) {
            diag(
                d,
                "DECISION_POINT_DUPLICATE",
                format!("{name} repeats decision id: {}", s(&v["id"])),
            )
        }
    }
    let mut procedures = BTreeSet::new();
    let declared_evidence = set(&skill["evidence_ids"]);
    let declared_paths = arr(&skill["path_contracts"])
        .iter()
        .filter_map(|p| p["path"].as_str())
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if arr(&skill["procedure_steps"]).is_empty() {
        diag(
            d,
            "PROCEDURE_STEPS_INVALID",
            format!("{name}.procedure_steps must be a non-empty array"),
        )
    }
    for (i, v) in arr(&skill["procedure_steps"]).iter().enumerate() {
        if !exact(
            v,
            &[
                "id",
                "action",
                "evidence_ids",
                "path_refs",
                "decision_refs",
                "expected_outcome",
                "failure_branch",
            ],
        ) || !ne(&v["id"])
            || !full(OWNERSHIP_ID_PATTERN, s(&v["id"]))
            || ["action", "expected_outcome", "failure_branch"]
                .iter()
                .any(|k| !ne(&v[*k]))
            || ["evidence_ids", "path_refs", "decision_refs"]
                .iter()
                .any(|k| !sl(&v[*k], true))
        {
            invalid(d, "PROCEDURE_STEP_INVALID", name, "procedure_steps", i);
            continue;
        }
        if !procedures.insert(s(&v["id"]).to_owned()) {
            diag(
                d,
                "PROCEDURE_STEP_DUPLICATE",
                format!("{name} repeats procedure id: {}", s(&v["id"])),
            )
        }
        let action = s(&v["action"]);
        if prose::hedge(action).is_some() || !prose::commands_action(action) {
            diag(
                d,
                "PROCEDURE_STEP_NOT_OPERATIONAL",
                format!(
                    "{name}.{} commands no concrete action: {}",
                    s(&v["id"]),
                    prose::excerpt(action)
                ),
            )
        }
        if arr(&v["path_refs"]).is_empty()
            && arr(&v["evidence_ids"]).is_empty()
            && !prose::anchor(action)
        {
            diag(
                d,
                "PROCEDURE_STEP_UNANCHORED",
                format!("{name}.{} names no concrete anchor", s(&v["id"])),
            )
        }
        check_ref(
            d,
            "PROCEDURE_EVIDENCE_UNKNOWN",
            name,
            &v["evidence_ids"],
            &declared_evidence,
        );
        check_ref(
            d,
            "PROCEDURE_PATH_UNKNOWN",
            name,
            &v["path_refs"],
            &declared_paths,
        );
        check_ref(
            d,
            "PROCEDURE_DECISION_UNKNOWN",
            name,
            &v["decision_refs"],
            &decisions,
        );
    }
    if disposed(schema) {
        evidence::dispositions(name, skill, &ledger.locations, runtime_fixed, d)
    }
    if wired(schema) {
        let mut refs = Vec::new();
        for (i, v) in arr(&skill["required_procedure_roles"]).iter().enumerate() {
            if !exact(
                v,
                &["role", "requirements", "evidence_ids", "procedure_step_ids"],
            ) || !ne(&v["role"])
                || !sl(&v["requirements"], false)
                || !sl(&v["evidence_ids"], runtime_fixed)
                || !sl(&v["procedure_step_ids"], false)
            {
                invalid(
                    d,
                    "PROCEDURE_ROLE_CONTRACT_INVALID",
                    name,
                    "required_procedure_roles",
                    i,
                );
                continue;
            }
            check_ref(
                d,
                "PROCEDURE_ROLE_EVIDENCE_UNKNOWN",
                name,
                &v["evidence_ids"],
                &declared_evidence,
            );
            check_ref(
                d,
                "PROCEDURE_ROLE_STEP_UNKNOWN",
                name,
                &v["procedure_step_ids"],
                &procedures,
            );
            refs.push(set(&v["procedure_step_ids"]));
        }
        let union = refs.iter().flatten().cloned().collect::<BTreeSet<_>>();
        if refs.len() >= 3 && union.len() == 1 {
            diag(
                d,
                "PROCEDURE_ROLE_COLLAPSED",
                format!(
                    "{name} discharges {} catalog obligations with a single step",
                    refs.len()
                ),
            )
        }
    }
    let analyzer = match crate::commands::CommandAnalyzer::new(target) {
        Ok(v) => Some(v),
        Err(e) => {
            diag(d, "COMMAND_ANALYZER_UNAVAILABLE", format!("{name}: {e}"));
            None
        }
    };
    let mut verification_ids = BTreeSet::new();
    if arr(&skill["verification"]).is_empty() {
        diag(
            d,
            "VERIFICATION_CONTRACT_INVALID",
            format!("{name}.verification must be a structured non-empty array"),
        )
    }
    for (i, v) in arr(&skill["verification"]).iter().enumerate() {
        let mut fields = vec![
            "id",
            "mode",
            "instruction",
            "command",
            "prerequisites",
            "safe_scope",
            "mutation_class",
            "network_class",
            "expected_result",
            "failure_result",
            "skip_condition",
            "skip_reporting",
        ];
        if wired(schema) {
            fields.push("baseline")
        }
        let valid = exact(v, &fields)
            && ne(&v["id"])
            && full(OWNERSHIP_ID_PATTERN, s(&v["id"]))
            && VERIFICATION_MODES.contains(&s(&v["mode"]))
            && sl(&v["prerequisites"], true)
            && MUTATION_CLASSES.contains(&s(&v["mutation_class"]))
            && NETWORK_CLASSES.contains(&s(&v["network_class"]))
            && [
                "instruction",
                "safe_scope",
                "expected_result",
                "failure_result",
                "skip_condition",
                "skip_reporting",
            ]
            .iter()
            .all(|k| ne(&v[*k]))
            && ((s(&v["mode"]) == "command" && ne(&v["command"]))
                || (s(&v["mode"]) == "manual" && v["command"].is_null()));
        if !valid {
            invalid(d, "VERIFICATION_CONTRACT_INVALID", name, "verification", i);
            continue;
        }
        if !verification_ids.insert(s(&v["id"]).to_owned()) {
            diag(
                d,
                "VERIFICATION_ID_DUPLICATE",
                format!("{name} repeats verification id: {}", s(&v["id"])),
            )
        }
        if wired(schema) {
            runtime::baseline(name, v, target, mode, runtime_fixed, d)
        }
        let text = [
            "instruction",
            "command",
            "safe_scope",
            "expected_result",
            "failure_result",
        ]
        .iter()
        .map(|k| s(&v[*k]))
        .collect::<Vec<_>>()
        .join(" ");
        if has(GENERIC_VERIFICATION_PATTERN, &text) {
            diag(
                d,
                "GENERIC_VERIFICATION",
                format!("{name}.{} verification is generic", s(&v["id"])),
            )
        }
        if has(NON_FALSIFIABLE_VERIFICATION_PATTERN, &text) {
            diag(
                d,
                "VERIFICATION_NOT_FALSIFIABLE",
                format!("{name}.{} accepts an impression", s(&v["id"])),
            )
        }
        let command = s(&v["command"]);
        if s(&v["mutation_class"]) != "none" || has(r"(^|\s)--fix(?:\s|$)", command) {
            diag(
                d,
                "MUTATING_VERIFICATION_COMMAND",
                format!("{name}.{} mutates during verification", s(&v["id"])),
            )
        }
        if !command.is_empty() {
            if let Some(analyzer) = &analyzer {
                match analyzer.analyze(command, true, None) {
                    Err(e) => diag(
                        d,
                        "COMMAND_ANALYSIS_FAILED",
                        format!("{name}.{} cannot resolve command: {e}", s(&v["id"])),
                    ),
                    Ok(a) => {
                        if !a.verification_safe && !runtime::attested(command, &a) {
                            diag(
                                d,
                                "COMMAND_RISK_BLOCKED",
                                format!(
                                    "{name}.{} unsafe verification: {}",
                                    s(&v["id"]),
                                    a.findings
                                        .iter()
                                        .map(|f| f.code.clone())
                                        .collect::<BTreeSet<_>>()
                                        .into_iter()
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                ),
                            )
                        }
                        let cat = a
                            .categories
                            .iter()
                            .map(String::as_str)
                            .collect::<BTreeSet<_>>();
                        let network = if cat.contains("external_provider_network") {
                            "external-provider"
                        } else {
                            "none"
                        };
                        let mutation = if cat.contains("destructive_database_deploy") {
                            "destructive"
                        } else if cat.contains("workspace_mutation") {
                            "workspace-write"
                        } else {
                            "none"
                        };
                        if s(&v["network_class"]) != network {
                            diag(
                                d,
                                "VERIFICATION_NETWORK_CLASS_MISMATCH",
                                format!(
                                    "{name}.{} declares {} but analysis found {network}",
                                    s(&v["id"]),
                                    s(&v["network_class"])
                                ),
                            )
                        }
                        if s(&v["mutation_class"]) != mutation {
                            diag(
                                d,
                                "VERIFICATION_MUTATION_CLASS_MISMATCH",
                                format!(
                                    "{name}.{} declares {} but analysis found {mutation}",
                                    s(&v["id"]),
                                    s(&v["mutation_class"])
                                ),
                            )
                        }
                    }
                }
            }
            search::verification(name, v, command, target, d);
            if runtime_fixed {
                runtime::description(name, v, command, d)
            }
        }
        if s(&v["network_class"]) == "external-provider" && mode != "external-side-effect" {
            diag(
                d,
                "VERIFICATION_NETWORK_CAPABILITY",
                format!(
                    "{name}.{} declares provider access without capability",
                    s(&v["id"])
                ),
            )
        }
    }
    if !arr(&skill["writes"]).is_empty()
        && !arr(&skill["verification"]).is_empty()
        && arr(&skill["verification"])
            .iter()
            .all(|v| s(&v["mode"]) == "command" && has(SEARCH_HEAD, s(&v["command"])))
    {
        diag(
            d,
            "WRITE_VERIFICATION_SEARCH_ONLY",
            format!("{name} writes paths but verifies only by text search"),
        )
    }
    let safety = &skill["integration_safety"];
    let policy = s(&safety["network_policy"]);
    let environment = s(&safety["environment"]);
    if !exact(
        safety,
        &[
            "network_policy",
            "test_double_strategy",
            "environment",
            "authorization_required",
            "rollback",
            "sanitization",
        ],
    ) || !NETWORK_POLICIES.contains(&policy)
        || !INTEGRATION_ENVIRONMENTS.contains(&environment)
        || !safety["authorization_required"].is_boolean()
        || ["test_double_strategy", "rollback", "sanitization"]
            .iter()
            .any(|k| !ne(&safety[*k]))
    {
        diag(
            d,
            "INTEGRATION_SAFETY_INVALID",
            format!("{name}.integration_safety has invalid fields"),
        )
    } else if INTEGRATION_CATEGORIES.contains(&s(&skill["category"]).to_lowercase().as_str()) {
        let allowed = match policy {
            "forbidden" | "mock-only" => ["none", "local"].contains(&environment),
            "sandbox-with-approval" => environment == "sandbox",
            "approved-live" => environment == "approved-live",
            _ => false,
        };
        if !allowed {
            diag(
                d,
                "PROVIDER_ENVIRONMENT_MISMATCH",
                format!("{name} network policy and environment disagree"),
            )
        }
        if ["sandbox-with-approval", "approved-live"].contains(&policy)
            && safety["authorization_required"] != true
        {
            diag(
                d,
                "PROVIDER_AUTHORIZATION_REQUIRED",
                format!("{name} allows provider access without authorization"),
            )
        }
        if policy == "approved-live"
            && !has(
                r"(?i)\b(fake|fixture|mock|stub|local|sandbox)\b",
                s(&safety["test_double_strategy"]),
            )
        {
            diag(
                d,
                "PROVIDER_SAFE_DEFAULT_MISSING",
                format!("{name} lacks fake/fixture/local/sandbox default"),
            )
        }
    }
    if mode == "external-side-effect"
        && (!["sandbox-with-approval", "approved-live"].contains(&policy)
            || safety["authorization_required"] != true)
    {
        diag(
            d,
            "EXTERNAL_SIDE_EFFECT_UNAUTHORIZED",
            format!("{name} external capability requires authorized sandbox/live policy"),
        )
    }
    if arr(&skill["path_contracts"]).is_empty() {
        diag(
            d,
            "PATH_CONTRACTS_INVALID",
            format!("{name}.path_contracts must be non-empty"),
        )
    }
    let mut accepted_paths = BTreeSet::new();
    for (i, v) in arr(&skill["path_contracts"]).iter().enumerate() {
        let path = s(&v["path"]);
        if !exact(v, &["path", "access", "classification", "evidence_ids"])
            || !ne(&v["path"])
            || !safe(path)
            || paths::normalize_glob(path) != path
            || !PATH_ACCESS_MODES.contains(&s(&v["access"]))
            || !PATH_CLASSIFICATIONS.contains(&s(&v["classification"]))
            || !sl(&v["evidence_ids"], true)
        {
            invalid(d, "PATH_CONTRACT_INVALID", name, "path_contracts", i);
            continue;
        }
        accepted_paths.insert(path.to_owned());
        let class = s(&v["classification"]);
        if s(&v["access"]) == "write" && !set(&skill["writes"]).contains(path) {
            diag(
                d,
                "PATH_WRITE_UNDECLARED",
                format!("{name} path contract write absent from writes: {path}"),
            )
        }
        if class == "required-existing" {
            match paths::path_matches(target, path) {
                Ok(matches) if matches.is_empty() => diag(
                    d,
                    "PATH_EXISTING_MISSING",
                    format!("{name} existing path does not resolve: {path}"),
                ),
                Err(error) => diag(d, "QUALITY_GLOB_UNRESOLVED", error.to_string()),
                _ => {}
            }
        }
        if class == "generated-runtime" && !runtime_fixed {
            diag(
                d,
                "PATH_GENERATED_INVALID",
                format!("{name} only runtime-fixed output may declare generated-runtime: {path}"),
            )
        }
        if class == "creatable"
            && (s(&v["access"]) != "write" || (!runtime_fixed && !paths::creatable(target, path)))
        {
            diag(
                d,
                "PATH_CREATABLE_INVALID",
                format!("{name} creatable path requires write and existing parent: {path}"),
            )
        }
        check_ref(
            d,
            "PATH_EVIDENCE_UNKNOWN",
            name,
            &v["evidence_ids"],
            &declared_evidence,
        );
    }
    if runtime_fixed {
        let doc = crate::assets::runtime_contract();
        let forbidden = ss(&doc["path_contracts"]["forbidden_invented_paths"]);
        let required = ss(&doc["path_contracts"]["required_skeleton"])
            .into_iter()
            .map(|x| {
                if x.ends_with('/') {
                    format!("{}/**", rstrip(&x, "/"))
                } else {
                    x
                }
            })
            .collect::<Vec<_>>();
        let creatable = arr(&doc["path_contracts"]["creatable"])
            .iter()
            .map(|v| s(&v["path"]).to_owned())
            .collect::<Vec<_>>();
        for c in arr(&skill["path_contracts"]) {
            let path = s(&c["path"]);
            if forbidden.iter().any(|p| paths::intersects(path, p)) {
                diag(
                    d,
                    "RUNTIME_PATH_FORBIDDEN",
                    format!("{name} references invented runtime path: {path}"),
                )
            }
            let class = s(&c["classification"]);
            let allowed = if class == "generated-runtime" {
                &required
            } else {
                &creatable
            };
            if ["generated-runtime", "creatable"].contains(&class)
                && !allowed.iter().any(|p| paths::intersects(path, p))
            {
                diag(
                    d,
                    "RUNTIME_PATH_UNDECLARED",
                    format!("{name} path absent from trusted runtime contract: {path}"),
                )
            }
        }
    }
    for path in ss(&skill["source_paths"])
        .into_iter()
        .chain(ss(&skill["writes"]))
    {
        if !accepted_paths.contains(&path) {
            diag(
                d,
                "PATH_CONTRACT_MISSING",
                format!("{name} has no path contract: {path}"),
            )
        }
    }
    let anchors = &skill["evidence_anchors"];
    let anchorable = declared_evidence
        .iter()
        .filter(|id| {
            ledger
                .locations
                .get(*id)
                .is_none_or(|(kind, _)| kind != "absence")
        })
        .count();
    if !anchors.is_array() || (anchorable > 0 && arr(anchors).is_empty()) {
        diag(
            d,
            "EVIDENCE_ANCHORS_INVALID",
            format!("{name}.evidence_anchors must contain bounded anchors"),
        )
    }
    let mut anchored = BTreeSet::new();
    for (i, a) in arr(anchors).iter().enumerate() {
        let id = s(&a["evidence_id"]);
        if !exact(
            a,
            &[
                "evidence_id",
                "claim",
                "anchor",
                "procedure_step_ids",
                "verification_ids",
            ],
        ) || !declared_evidence.contains(id)
            || !ne(&a["claim"])
            || !ne(&a["anchor"])
            || !sl(&a["procedure_step_ids"], false)
            || !sl(&a["verification_ids"], false)
        {
            invalid(d, "EVIDENCE_ANCHOR_INVALID", name, "evidence_anchors", i);
            continue;
        }
        anchored.insert(id.to_owned());
        if !ledger
            .claims
            .get(id)
            .is_some_and(|claims| claims.contains(&s(&a["claim"]).to_owned()))
        {
            diag(
                d,
                "EVIDENCE_ANCHOR_CLAIM_UNKNOWN",
                format!("{name} anchor claim not declared by {id}"),
            )
        }
        if !set(&a["procedure_step_ids"]).is_subset(&procedures)
            || !set(&a["verification_ids"]).is_subset(&verification_ids)
        {
            diag(
                d,
                "EVIDENCE_ANCHOR_REFERENCE_UNKNOWN",
                format!("{name} anchor references unknown procedure/check"),
            )
        }
        if let Some((kind, path)) = ledger.locations.get(id)
            && kind == "path"
        {
            let anchor = s(&a["anchor"]);
            if !anchor.starts_with(&(path.clone() + ":")) {
                diag(
                    d,
                    "EVIDENCE_ANCHOR_LOCATION",
                    format!("{name} anchor must begin with canonical evidence path"),
                )
            } else if !full(
                &format!(
                    r"{}:(?:L[1-9][0-9]*(?:-L?[1-9][0-9]*)?|symbol:[A-Za-z_][A-Za-z0-9_.:\\-]*)",
                    super::regex_adapter::escape(path)
                ),
                anchor,
            ) {
                diag(
                    d,
                    "EVIDENCE_ANCHOR_FORMAT",
                    format!("{name} anchor needs bounded line range or symbol"),
                )
            } else {
                evidence::resolve_anchor(name, anchor, path, target, d)
            }
        }
        let step_ids = set(&a["procedure_step_ids"]);
        let check_ids = set(&a["verification_ids"]);
        let steps = arr(&skill["procedure_steps"])
            .iter()
            .filter(|v| step_ids.contains(s(&v["id"])))
            .flat_map(flat)
            .collect::<Vec<_>>()
            .join(" ");
        let checks = arr(&skill["verification"])
            .iter()
            .filter(|v| check_ids.contains(s(&v["id"])))
            .flat_map(flat)
            .collect::<Vec<_>>()
            .join(" ");
        if [
            &flat(&skill["owned_scope"]).join(" "),
            &steps,
            &checks,
            &flat(&skill["output_contract"]).join(" "),
        ]
        .iter()
        .any(|text| !prose::matches(&a["claim"], text))
        {
            diag(
                d,
                "CLAIM_TRACEABILITY_MISSING",
                format!(
                    "{name} claim not traceable through scope/procedure/verification/output: {}",
                    s(&a["claim"])
                ),
            )
        }
    }
    for id in &declared_evidence {
        if !anchored.contains(id)
            && ledger
                .locations
                .get(id)
                .is_none_or(|(kind, _)| kind != "absence")
        {
            diag(
                d,
                "EVIDENCE_ANCHOR_MISSING",
                format!("{name} lacks bounded anchor for {id}"),
            )
        }
    }
    routing(name, skill, runtime_fixed, d);
}
pub fn routing(name: &str, skill: &Value, runtime_fixed: bool, d: &mut Vec<Diagnostic>) {
    let cases = &skill["routing_cases"];
    if arr(cases).is_empty() {
        diag(
            d,
            "ROUTING_CASES_INVALID",
            format!("{name}.routing_cases must be non-empty"),
        )
    }
    let mut allowed = BTreeSet::from([name.to_owned()]);
    allowed.extend(
        arr(&skill["nearest_siblings"])
            .iter()
            .filter_map(|v| v["name"].as_str())
            .map(str::to_owned),
    );
    let mut destinations = BTreeSet::new();
    for (i, c) in arr(cases).iter().enumerate() {
        let primary = s(&c["expected_primary"]);
        if !exact(
            c,
            &[
                "prompt",
                "expected_primary",
                "permitted_secondary",
                "forbidden_skills",
                "rationale",
                "evidence_ids",
            ],
        ) || !ne(&c["prompt"])
            || !allowed.contains(primary)
            || !sl(&c["permitted_secondary"], true)
            || !sl(&c["forbidden_skills"], true)
            || !ne(&c["rationale"])
            || !sl(&c["evidence_ids"], runtime_fixed)
        {
            invalid(d, "ROUTING_CASE_INVALID", name, "routing_cases", i);
            continue;
        }
        let secondary = set(&c["permitted_secondary"]);
        let forbidden = set(&c["forbidden_skills"]);
        if !secondary.is_subset(&allowed) {
            diag(
                d,
                "ROUTING_CASE_SECONDARY_UNKNOWN",
                format!("{name} permits non-adjacent secondary"),
            )
        }
        if secondary.contains(primary)
            || forbidden.contains(primary)
            || !secondary.is_disjoint(&forbidden)
        {
            diag(
                d,
                "ROUTING_CASE_PRECEDENCE_CONFLICT",
                format!("{name} has contradictory owner precedence"),
            )
        }
        if !set(&c["evidence_ids"]).is_subset(&set(&skill["evidence_ids"])) {
            diag(
                d,
                "ROUTING_CASE_EVIDENCE_UNKNOWN",
                format!("{name} routing case cites undeclared evidence"),
            )
        }
        destinations.insert(primary.to_owned());
        let trigger = &skill["triggers"][if primary == name {
            "positive"
        } else {
            "negative"
        }];
        if !prose::matches(trigger, s(&c["prompt"])) {
            diag(
                d,
                "ROUTING_CASE_TRIGGER_MISMATCH",
                format!("{name} case does not match expected positive/negative trigger"),
            )
        }
    }
    if !allowed.is_subset(&destinations) {
        diag(
            d,
            "ROUTING_CASE_COVERAGE",
            format!("{name} routing cases must cover self and every adjacent owner"),
        )
    }
}
