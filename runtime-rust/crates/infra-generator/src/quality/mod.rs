//! Evidence-grounded skill planning, authored-body and routing validation.
mod authored;
mod baseline;
mod consts;
mod contracts;
mod evidence;
mod inventory;
mod paths;
mod plan;
mod prose;
pub mod regex_adapter;
mod registry;
mod rejections;
mod routing;
mod runtime;
mod search;
mod similarity;
mod util;
use crate::common::{Diagnostic, diag};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
use util::*;
pub fn validate_plan(plan_path: &Path, target: &Path, registry: &Path) -> Vec<Diagnostic> {
    let mut d = Vec::new();
    let plan_path = plan::absolute(plan_path);
    let target = plan::absolute(target);
    if !target.is_dir() {
        diag(
            &mut d,
            "TARGET_INVALID",
            format!("target is not a directory: {}", target.display()),
        );
        return finish(d);
    }
    let document = load(&plan_path, &mut d);
    if document.is_object() {
        plan::validate(
            &document,
            &plan_path,
            &target,
            &plan::absolute(registry),
            &mut d,
        );
    }
    finish(d)
}
pub fn validate_plan_value(
    document: &Value,
    plan_path: &Path,
    target: &Path,
    registry: &Path,
) -> Vec<Diagnostic> {
    let mut d = Vec::new();
    let target = plan::absolute(target);
    if !target.is_dir() {
        diag(
            &mut d,
            "TARGET_INVALID",
            format!("target is not a directory: {}", target.display()),
        );
        return finish(d);
    }
    if !document.is_object() {
        diag(&mut d, "PLAN_INVALID", "plan root must be an object")
    } else {
        plan::validate(
            document,
            &plan::absolute(plan_path),
            &target,
            &plan::absolute(registry),
            &mut d,
        );
    }
    finish(d)
}
pub fn validate_skills(
    skills_dir: &Path,
    plan_path: &Path,
    target: &Path,
    registry: &Path,
    partial: bool,
) -> Vec<Diagnostic> {
    let mut d = Vec::new();
    let target = plan::absolute(target);
    let skills_dir = plan::expand_user(skills_dir);
    let plan_path = plan::absolute(plan_path);
    if !target.is_dir() {
        diag(
            &mut d,
            "TARGET_INVALID",
            format!("target is not a directory: {}", target.display()),
        );
        return finish(d);
    }
    if !skills_dir.is_dir() {
        diag(
            &mut d,
            "SKILLS_DIR_INVALID",
            format!("skills directory invalid: {}", skills_dir.display()),
        );
        return finish(d);
    }
    if skills_dir.is_symlink() {
        diag(
            &mut d,
            "SKILLS_DIR_UNSAFE",
            format!(
                "skills directory must not be a symlink: {}",
                skills_dir.display()
            ),
        );
        return finish(d);
    }
    let skills_dir = plan::absolute(&skills_dir);
    let document = load(&plan_path, &mut d);
    let state = if document.is_object() {
        plan::validate(
            &document,
            &plan_path,
            &target,
            &plan::absolute(registry),
            &mut d,
        )
    } else {
        plan::PlanState {
            skills: SkillMap::new(),
            ledger: evidence::Ledger::default(),
        }
    };
    if consts::LEGACY_PLAN_SCHEMAS.contains(&s(&document["schema_version"])) {
        diag(
            &mut d,
            "LEGACY_PLAN_PUBLICATION_INELIGIBLE",
            format!(
                "schema {} is audit-only; authored output requires {}",
                s(&document["schema_version"]),
                consts::CURRENT_PLAN_SCHEMA
            ),
        )
    }
    let allowed = arr(&document["preexisting_team_skills"])
        .iter()
        .filter(|r| {
            ne(&r["name"])
                && s(&r["path"]) == format!(".claude/skills/{}", s(&r["name"]))
                && s(&r["kind"]) == "preexisting-team"
                && s(&r["mode"]) == "merge"
                && s(&r["ownership"]) == "team"
                && s(&r["publication"]) == "watch-only"
        })
        .map(|r| target.join(".claude/skills").join(s(&r["name"])))
        .collect::<BTreeSet<_>>();
    authored::inventory(
        &skills_dir,
        &state.skills,
        &state.ledger.locations,
        &target,
        partial,
        &allowed,
        &mut d,
    );
    finish(d)
}
pub fn validate_agent_routing(
    agents: &Path,
    plan: &Path,
    target: &Path,
    registry: &Path,
    require_invokes: bool,
) -> Vec<Diagnostic> {
    routing::agents(agents, plan, target, registry, require_invokes)
}
pub fn validate_flow_routing(
    commands: &Path,
    plan: &Path,
    target: &Path,
    registry: &Path,
) -> Vec<Diagnostic> {
    routing::flows(commands, plan, target, registry)
}
pub fn compare_coverage_baseline(current: &Path, baseline: &Path) -> (Vec<Diagnostic>, Value) {
    baseline::compare(current, baseline)
}
pub fn skill_class_split(plan: &Path) -> BTreeMap<String, Vec<String>> {
    baseline::classes(plan)
}
pub fn runtime_command_outcomes() -> BTreeMap<String, Vec<Value>> {
    runtime::outcomes()
}
