use super::{consts::*, paths, prose, runtime, similarity, util::*};
use crate::common::{Diagnostic, diag, warn};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
fn evidence_rows(
    name: &str,
    skill: &Value,
    body: &str,
    evidence: &EvidenceMap,
    d: &mut Vec<Diagnostic>,
) {
    let declared = set(&skill["evidence_ids"]);
    for line in body.lines() {
        let cells = similarity::table_cells(line);
        if cells.len() < 2 {
            continue;
        }
        let mut id = None;
        let mut candidates = Vec::new();
        for cell in cells {
            let bare = cell.trim_matches(['\u{60}', '*', '_', ' ']);
            if id.is_none() && (full(EVIDENCE_ID_CELL, bare) || evidence.contains_key(bare)) {
                id = Some(bare.to_owned());
                continue;
            }
            for span in words(CODE_SPAN_CONTENT, &cell) {
                if let Some(path) = paths::named(&span) {
                    candidates.push(path)
                }
            }
        }
        let Some(id) = id else { continue };
        if candidates.is_empty() {
            continue;
        }
        if !declared.contains(&id) {
            diag(
                d,
                "SKILL_EVIDENCE_ROW_UNDECLARED",
                format!("{name}: evidence table cites undeclared {id}"),
            );
            continue;
        }
        if let Some((kind, path)) = evidence.get(&id)
            && kind == "path"
            && !candidates.contains(path)
        {
            diag(
                d,
                "SKILL_EVIDENCE_ROW_ANCHOR",
                format!(
                    "{name}: {id} row cites wrong paths: {}",
                    candidates.join(", ")
                ),
            )
        }
    }
}
pub fn skill_file(
    name: &str,
    skill: &Value,
    path: &Path,
    evidence: &EvidenceMap,
    target: &Path,
    d: &mut Vec<Diagnostic>,
) -> (String, Vec<String>, Vec<String>) {
    let text = match read_text(path) {
        Ok(v) => v,
        Err(e) => {
            diag(
                d,
                "SKILL_UNREADABLE",
                format!("{name}: cannot read SKILL.md: {e}"),
            );
            return (String::new(), Vec::new(), Vec::new());
        }
    };
    let (frontmatter, body) = prose::frontmatter(&text);
    runtime::body_commands(name, &body, target, d);
    if frontmatter.get("name").map(String::as_str) != Some(name) {
        diag(
            d,
            "SKILL_FRONTMATTER_NAME",
            format!("{name}: directory and frontmatter name differ"),
        )
    }
    let description = frontmatter
        .get("description")
        .map(String::as_str)
        .unwrap_or("");
    if description.trim().is_empty() || !has(r"(?i)\b(when|use|trigger|for)\b", description) {
        diag(
            d,
            "SKILL_DESCRIPTION",
            format!("{name}: description must identify selection conditions"),
        )
    }
    let sections = prose::sections(&body);
    let mut resolved = BTreeMap::new();
    let mut missing = 0;
    for (role, aliases) in SECTION_ALIASES {
        let content = prose::section(&sections, aliases);
        if prose::meaningful(&content).len() < 3 {
            missing += 1;
            diag(
                d,
                "SKILL_SECTION_MISSING",
                format!("{name}: missing or insubstantial {role} section"),
            )
        }
        resolved.insert(*role, content);
    }
    let lower = body.to_lowercase();
    if !prose::matches(
        &skill["triggers"]["positive"],
        &format!("{description}\n{body}"),
    ) {
        diag(
            d,
            "SKILL_POSITIVE_TRIGGER_TRACE",
            format!("{name}: positive trigger is not traceable"),
        )
    }
    if !prose::matches(&skill["triggers"]["negative"], &body) {
        diag(
            d,
            "SKILL_NEGATIVE_TRIGGER_TRACE",
            format!("{name}: negative trigger/deferral is not traceable"),
        )
    }
    for phrase in GENERIC_PHRASES {
        if lower.contains(phrase) {
            diag(
                d,
                "SKILL_GENERIC_PHRASE",
                format!("{name}: nonoperational generic phrase: {phrase:?}"),
            )
        }
    }
    let steps = prose::steps(&resolved["procedure"]);
    for (i, step) in steps.iter().enumerate() {
        if let Some(hedge) = prose::hedge(step) {
            diag(
                d,
                "SKILL_STEP_HEDGED",
                format!(
                    "{name}: step {} defers to reflection ({hedge:?}): {}",
                    i + 1,
                    prose::excerpt(step)
                ),
            )
        } else if !prose::commands_action(step) {
            diag(
                d,
                "SKILL_STEP_NOT_OPERATIONAL",
                format!(
                    "{name}: step {} commands no action: {}",
                    i + 1,
                    prose::excerpt(step)
                ),
            )
        }
    }
    if !steps.is_empty() && !prose::anchor(&steps.join(" ")) {
        diag(
            d,
            "SKILL_PROCEDURE_UNANCHORED",
            format!("{name}: procedure names no concrete anchor"),
        )
    }
    let verification = &resolved["verification"];
    if has(NON_FALSIFIABLE_VERIFICATION_PATTERN, verification) {
        diag(
            d,
            "SKILL_VERIFICATION_NOT_FALSIFIABLE",
            format!("{name}: verification accepts an impression"),
        )
    } else if !verification.is_empty()
        && !prose::action(verification, VERIFICATION_ACTION_VERBS)
        && !prose::tooling(verification)
    {
        diag(
            d,
            "SKILL_VERIFICATION_NOT_OPERATIONAL",
            format!(
                "{name}: verification names no check: {}",
                prose::excerpt(verification)
            ),
        )
    }
    if prose::circular(name, &resolved["purpose"]) {
        diag(
            d,
            "SKILL_CIRCULAR_PURPOSE",
            format!("{name}: circular purpose"),
        )
    }
    for (field, haystack, code) in [
        ("necessity_rationale", &body, "SKILL_RATIONALE_TRACE"),
        (
            "required_procedure_roles",
            &resolved["procedure"],
            "SKILL_PROCEDURE_TRACE",
        ),
        (
            "decision_points",
            &resolved["procedure"],
            "SKILL_DECISION_TRACE",
        ),
        (
            "verification",
            &resolved["verification"],
            "SKILL_VERIFICATION_TRACE",
        ),
        (
            "output_contract",
            &resolved["outputs"],
            "SKILL_OUTPUT_TRACE",
        ),
        (
            "failure_handling",
            &resolved["failure"],
            "SKILL_FAILURE_TRACE",
        ),
        ("owned_scope", &body, "SKILL_OWNED_SCOPE_TRACE"),
        ("excluded_scope", &body, "SKILL_EXCLUDED_SCOPE_TRACE"),
    ] {
        if !prose::matches(&skill[field], haystack) {
            diag(
                d,
                code,
                format!("{name}: {field} is not traceable to skill content"),
            )
        }
    }
    for step in arr(&skill["procedure_steps"]) {
        if ne(&step["action"]) && !prose::matches(&step["action"], &resolved["procedure"]) {
            diag(
                d,
                "SKILL_STEP_NOT_RENDERED",
                format!(
                    "{name}: approved step {} never reaches procedure: {}",
                    s(&step["id"]),
                    prose::excerpt(s(&step["action"]))
                ),
            )
        }
    }
    for role in arr(&skill["required_procedure_roles"]) {
        if ne(&role["role"]) {
            let stated = format!(
                "{} {}",
                s(&role["role"]),
                ss(&role["requirements"]).join(" ")
            );
            if !prose::matches(&Value::String(stated), &resolved["procedure"]) {
                diag(
                    d,
                    "SKILL_ROLE_NOT_RENDERED",
                    format!(
                        "{name}: catalog obligation {} never reaches procedure",
                        s(&role["role"])
                    ),
                )
            }
        }
    }
    let mut refs = ss(&skill["source_paths"]);
    for id in ss(&skill["evidence_ids"]) {
        if let Some((kind, path)) = evidence.get(&id)
            && kind == "path"
        {
            refs.push(path.clone())
        }
    }
    let kind = s(&skill["kind"]).to_lowercase();
    if kind != RUNTIME_FIXED_KIND
        && (TARGET_DERIVED_KINDS.contains(&kind.as_str()) || !refs.is_empty())
        && !refs.iter().any(|p| lower.contains(&p.to_lowercase()))
    {
        diag(
            d,
            "SKILL_TARGET_REFERENCE",
            format!("{name}: no concrete target evidence path appears"),
        )
    }
    if kind == RUNTIME_FIXED_KIND {
        if !has(r"\b(memory-bank|project-brain)/", &body) {
            diag(
                d,
                "SKILL_RUNTIME_REFERENCE",
                format!("{name}: runtime-fixed guide must cite installed runtime"),
            )
        }
        runtime::body(name, skill, &body, evidence, d)
    } else {
        paths::body_paths(name, skill, &body, evidence, target, d)
    }
    evidence_rows(name, skill, &body, evidence, d);
    let category = s(&skill["category"]).to_lowercase();
    let mut evidence_text = String::new();
    if INTEGRATION_CATEGORIES.contains(&category.as_str())
        || DOMAIN_CATEGORIES.contains(&category.as_str())
    {
        for id in ss(&skill["evidence_ids"]) {
            if let Some((kind, value)) = evidence.get(&id) {
                evidence_text.push_str(value);
                evidence_text.push(' ');
                if kind == "path"
                    && let Some(path) = confined(target, value)
                    && let Ok(text) = read_text(path)
                {
                    evidence_text.extend(text.chars().take(200_000));
                }
            }
        }
        let identifying = prose::meaningful(&name.replace('-', " "))
            .difference(&IDENTITY_STOPWORDS.iter().map(|s| s.to_string()).collect())
            .cloned()
            .collect::<BTreeSet<_>>();
        if !identifying.is_empty() && identifying.is_disjoint(&prose::meaningful(&evidence_text)) {
            diag(
                d,
                "SKILL_EVIDENCE_IRRELEVANT",
                format!("{name}: evidence does not identify planned concern"),
            )
        }
    }
    if INTEGRATION_CATEGORIES.contains(&category.as_str()) {
        let identity = prose::meaningful(&format!("{} {evidence_text}", name.replace('-', " ")));
        if identity.is_disjoint(&prose::meaningful(&body)) {
            diag(
                d,
                "INTEGRATION_IDENTITY",
                format!("{name}: integration identity is not concrete"),
            )
        }
        if !has(
            r"(?i)\b(runtime|boundary|client|adapter|service|transport|sdk|api)\b",
            &body,
        ) {
            diag(
                d,
                "INTEGRATION_BOUNDARY",
                format!("{name}: integration runtime boundary missing"),
            )
        }
    }
    if DOMAIN_CATEGORIES.contains(&category.as_str())
        && !has(
            r"(?i)\b(invariant|transition|permission|role|failure|forbidden|state)\b",
            &body,
        )
    {
        diag(
            d,
            "DOMAIN_CONTRACT",
            format!("{name}: domain invariants/transitions/permissions/failures missing"),
        )
    }
    for sibling in ss(&skill["related_skills"]).into_iter().chain(
        arr(&skill["nearest_siblings"])
            .iter()
            .map(|v| s(&v["name"]).to_owned()),
    ) {
        if !lower.contains(&sibling.to_lowercase()) {
            diag(
                d,
                "SKILL_SIBLING_TRACE",
                format!("{name}: sibling {sibling} absent from body"),
            )
        }
    }
    for sibling in arr(&skill["nearest_siblings"]) {
        if sibling.is_object() && !prose::matches(&sibling["boundary"], &body) {
            diag(
                d,
                "SKILL_SIBLING_BOUNDARY_TRACE",
                format!("{name}: nearest-sibling boundary not traceable"),
            )
        }
    }
    let fixed = fixed(&skill["fixed_blocks"]);
    for block in &fixed {
        if !body.contains(block) {
            diag(
                d,
                "FIXED_BLOCK_MISSING",
                format!("{name}: approved fixed block missing byte-for-byte"),
            )
        } else if !resolved["failure"].contains(block) {
            diag(
                d,
                "FIXED_BLOCK_SCOPE",
                format!("{name}: fixed block must be in failure handling"),
            )
        }
        if prose::tokens(block).len() > 120 {
            diag(
                d,
                "FIXED_BLOCK_OVERSIZED",
                format!("{name}: fixed block exceeds shared exemption"),
            )
        }
    }
    let (lines, tokens) = similarity::body(&body, &fixed);
    if missing >= 3
        || lines.iter().collect::<BTreeSet<_>>().len() < 5
        || tokens.iter().collect::<BTreeSet<_>>().len() < 18
    {
        diag(
            d,
            "SKILL_CONTENT_EMPTY",
            format!("{name}: substantive content empty or filler"),
        )
    }
    (body, lines, tokens)
}
pub fn inventory(
    skills_dir: &Path,
    skills: &SkillMap,
    evidence: &EvidenceMap,
    target: &Path,
    partial: bool,
    allowed: &BTreeSet<std::path::PathBuf>,
    d: &mut Vec<Diagnostic>,
) {
    let mut actual = BTreeMap::new();
    let entries = match std::fs::read_dir(skills_dir) {
        Ok(v) => v,
        Err(e) => {
            diag(
                d,
                "SKILLS_DIR_INVALID",
                format!("cannot read skills directory: {e}"),
            );
            return;
        }
    };
    let mut entries = entries
        .filter_map(std::result::Result::ok)
        .collect::<Vec<_>>();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_symlink() {
            let lexical = path
                .parent()
                .and_then(|p| infra_core::fs::canonicalize(p).ok())
                .unwrap_or_else(|| skills_dir.to_owned())
                .join(&name);
            if !allowed.contains(&lexical) {
                diag(
                    d,
                    "SKILL_PATH_UNSAFE",
                    format!("skill path must not be symlink: {name}"),
                )
            }
        } else if path.is_dir() && path.join("SKILL.md").is_file() {
            actual.insert(name, path.join("SKILL.md"));
        }
    }
    if !partial {
        for name in skills.keys().filter(|n| !actual.contains_key(*n)) {
            diag(
                d,
                "SKILL_FILE_MISSING",
                format!("planned skill lacks SKILL.md: {name}"),
            )
        }
    }
    for name in actual.keys().filter(|n| !skills.contains_key(*n)) {
        diag(
            d,
            "SKILL_UNPLANNED",
            format!("skill has no plan entry: {name}"),
        )
    }
    let entities = skills
        .keys()
        .chain(actual.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut normalized = BTreeMap::new();
    let mut skeletons = BTreeMap::new();
    for (name, path) in &actual {
        if let Some(skill) = skills.get(name) {
            let (body, lines, tokens) = skill_file(name, skill, path, evidence, target, d);
            skeletons.insert(
                name.clone(),
                similarity::skeleton(&body, &fixed(&skill["fixed_blocks"]), &entities),
            );
            normalized.insert(name.clone(), (lines, tokens));
        }
    }
    let names = normalized.keys().cloned().collect::<Vec<_>>();
    for (i, a) in names.iter().enumerate() {
        for b in names.iter().skip(i + 1) {
            let line = similarity::line_score(&normalized[a].0, &normalized[b].0);
            let token = similarity::token_score(&normalized[a].1, &normalized[b].1);
            if line >= LINE_FAIL_THRESHOLD || token >= TOKEN_FAIL_THRESHOLD {
                let repeated = normalized[a]
                    .0
                    .iter()
                    .filter(|l| normalized[b].0.contains(l))
                    .cloned()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .take(3)
                    .collect::<Vec<_>>();
                diag(
                    d,
                    "SKILL_SIMILARITY",
                    format!(
                        "{a} and {b} duplicate substantive content (line={line:.3}, token={token:.3}); repeated: {}",
                        repeated.join(" | ")
                    ),
                )
            } else if token >= TOKEN_WARN_THRESHOLD {
                warn(
                    d,
                    "SKILL_SIMILARITY_WARN",
                    format!("{a} and {b} high token similarity (line={line:.3}, token={token:.3})"),
                )
            }
            similarity::compare_skeletons(a, &skeletons[a], b, &skeletons[b], d)
        }
    }
    similarity::repeated(
        &normalized
            .into_iter()
            .map(|(name, (lines, _))| (name, lines))
            .collect(),
        false,
        d,
    );
    similarity::repeated(&skeletons, true, d)
}
