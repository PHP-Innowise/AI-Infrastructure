use crate::{HarnessError, Result, store};
use serde_json::{Value, json};
use std::path::Path;
pub const PHASES: &[(&str, &str)] = &[
    ("initialize", "Initialize"),
    ("specify", "Specify"),
    ("plan", "Plan"),
    ("tasks", "Tasks"),
    ("implement", "Implement / resume"),
    ("review", "Review"),
];
pub fn validate(value: &Value, workflow: &str, previous: Option<&Value>) -> Result<Option<Value>> {
    if workflow != "sdd" {
        if !value.is_null() {
            return Err(HarnessError::invalid(
                "SDD settings require the SDD workflow.",
            ));
        }
        return Ok(None);
    }
    if !store::exact(value, &["phase", "feature"], &[]) {
        return Err(HarnessError::invalid("Choose an SDD phase and feature."));
    }
    let phase = value["phase"].as_str().unwrap_or("");
    let feature = value["feature"].as_str().unwrap_or("");
    if !PHASES.iter().any(|v| v.0 == phase)
        || feature.is_empty()
        || feature.len() > 80
        || feature == "memory"
        || feature.split('-').any(|s| {
            s.is_empty()
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
    {
        return Err(HarnessError::invalid(
            "Choose a feature slug of 1–80 lowercase letters, digits and single hyphens (not memory).",
        ));
    }
    if previous.is_some_and(|old| old["feature"] != value["feature"]) {
        return Err(HarnessError::invalid(
            "The SDD feature stays fixed for this session.",
        ));
    }
    Ok(Some(value.clone()))
}
pub fn mode(settings: &Value) -> &'static str {
    if settings["phase"] == "review" {
        "plan"
    } else {
        "edit"
    }
}
pub fn paths(root: &Path, settings: &Value) -> Vec<String> {
    let feature = settings["feature"].as_str().unwrap_or("");
    let mut names = vec!["specs/memory/constitution.md".to_owned()];
    for name in ["spec.md", "plan.md", "tasks.md", "progress.md"] {
        names.push(format!("specs/{feature}/{name}"));
    }
    names
        .into_iter()
        .map(|old| {
            let (folder, name) = old.rsplit_once('/').unwrap_or(("", &old));
            let new = format!("{folder}/sdd-{name}");
            if root.join(&old).exists() && !root.join(&new).exists() {
                old
            } else {
                new
            }
        })
        .collect()
}
pub fn artifacts(root: &Path, settings: &Value) -> Result<Vec<Value>> {
    paths(root,settings).into_iter().map(|name|{
  let path=root.join(&name);infra_core::fs::admit_descendant(root,&path)?;
  match infra_core::fs::read_regular(&path){
   Ok(bytes)=>{let truncated=bytes.len()>256*1024;let text=std::str::from_utf8(&bytes[..bytes.len().min(256*1024)]).ok().map(String::from);Ok(json!({"path":name,"available":text.is_some(),"text":text.unwrap_or_default(),"truncated":truncated}))}
   Err(e)if e.kind==infra_core::ErrorKind::NotFound=>Ok(json!({"path":name,"available":false,"text":"","truncated":false})),Err(e)=>Err(e.into())
  }
 }).collect()
}
pub fn require_documents(root: &Path, settings: &Value) -> Result<()> {
    let needed = match settings["phase"].as_str() {
        Some("plan") => 1,
        Some("tasks") => 2,
        Some("implement" | "review") => 3,
        _ => 0,
    };
    for file in artifacts(root, settings)?.into_iter().skip(1).take(needed) {
        if file["available"] != true
            || file["truncated"] == true
            || file["text"].as_str().is_none_or(|s| s.trim().is_empty())
        {
            return Err(HarnessError::invalid(format!(
                "SDD requires a readable, nonempty {} (at most 256 KiB).",
                file["path"].as_str().unwrap_or("")
            )));
        }
        if file["path"]
            .as_str()
            .is_some_and(|s| s.ends_with("spec.md"))
            && file["text"]
                .as_str()
                .is_some_and(|s| s.contains("[NEEDS CLARIFICATION"))
        {
            return Err(HarnessError::invalid(
                "Resolve NEEDS CLARIFICATION markers in Specify before continuing.",
            ));
        }
    }
    Ok(())
}
pub fn instructions(root: &Path, settings: &Value) -> Result<String> {
    let phase = settings["phase"]
        .as_str()
        .ok_or_else(|| HarnessError::invalid("Missing SDD phase."))?;
    let names = paths(root, settings);
    let action = match phase {
        "initialize" => {
            "Create only missing constitution.md with project principles. Append SDD guidance to AGENTS.md only if absent. Preserve all existing instructions and specifications. Do not reinitialize or replace documents."
        }
        "specify" => {
            "Write spec.md: Overview, User Stories, testable Acceptance Criteria, Non-Goals, Open Questions, Dependencies. Describe WHAT and WHY, not implementation details. Ask only unanswered questions; mark unresolved facts [NEEDS CLARIFICATION: question]. Do not write implementation code."
        }
        "plan" => {
            "Read spec.md and write plan.md: Technical Approach, Key Decisions with rationale, Data Model, Interface Contracts, Implementation Phases, Risks and Mitigations. Respect non-goals and principles. Do not write implementation code."
        }
        "tasks" => {
            "Read spec.md and plan.md, then write tasks.md with dependent tasks, test-before-implementation ordering and explicit verification. Use [ ] pending, [~] in progress, [x] verified complete, [P] independent groups and [C] checkpoints. Include final build/test verification from project commands. Do not implement tasks."
        }
        "implement" => {
            "Read spec.md, plan.md, tasks.md and progress.md. Resume the first unfinished task or independent group. Write and run failing tests before implementation, make them pass, then run project verification. At the next checkpoint record commands, outcomes, completed tasks, acceptance criteria and issues in progress.md, then STOP. Never mark unavailable checks passed."
        }
        "review" => {
            "Read spec.md, plan.md, tasks.md and progress.md. Review actual implementation against every acceptance criterion and task completion claim. Report met, unmet and unverified criteria with file references and test evidence. Do not modify files."
        }
        _ => return Err(HarnessError::invalid("Unknown SDD phase.")),
    };
    let mut out = format!(
        "Harness SDD phase: {phase}. Perform only this selected phase; do not automatically proceed. Read AGENTS.md and specs/memory/constitution.md. Read graphify-out/GRAPH_REPORT.md first if present for planning/tasks/implementation. Preserve authoritative documents and revise them in place. Stop for unresolved requirements or conflicting instructions. Follow session delegation limits.\n{action}\nReport changed document paths and decisions needed, then stop.\n",
    );
    for (name, path) in [
        "constitution.md",
        "spec.md",
        "plan.md",
        "tasks.md",
        "progress.md",
    ]
    .into_iter()
    .zip(names)
    {
        out = out.replace(name, path.rsplit('/').next().unwrap_or(name));
    }
    if phase != "review" {
        out.push_str("Register feature documents in specs/MANIFEST.md. ");
    }
    out.push_str("Follow existing naming hooks; do not disable them.\n\n");
    Ok(out)
}
