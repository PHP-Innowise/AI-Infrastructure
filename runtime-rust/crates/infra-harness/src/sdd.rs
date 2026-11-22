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
    let phase = settings["phase"].as_str().unwrap_or("");
    let needed = match phase {
        "plan" => 1,
        "tasks" => 2,
        "implement" | "review" => 3,
        _ => 0,
    };
    let all_artifacts = artifacts(root, settings)?;
    for file in all_artifacts.iter().skip(1).take(needed) {
        if file["available"] != true
            || file["truncated"] == true
            || file["text"].as_str().is_none_or(|s| s.trim().is_empty())
        {
            return Err(HarnessError::invalid(format!(
                "SDD requires a readable, nonempty {} (at most 256 KiB).",
                file["path"].as_str().unwrap_or("")
            )));
        }
        let path = file["path"].as_str().unwrap_or("");
        let body = file["text"].as_str().unwrap_or("");
        if (path.ends_with("spec.md") || path.ends_with("plan.md") || path.ends_with("tasks.md"))
            && body.contains("[NEEDS CLARIFICATION")
        {
            return Err(HarnessError::invalid(format!(
                "Resolve NEEDS CLARIFICATION markers in {} before continuing.",
                path
            )));
        }
    }
    let errors = validate_dataflow(&all_artifacts, phase, needed);
    if !errors.is_empty() {
        return Err(HarnessError::invalid(format!(
            "SDD static cross-reference check failed: {}",
            errors.join("; ")
        )));
    }
    Ok(())
}

/// Gate-1-style static dataflow validator across `spec.md -> plan.md -> tasks.md -> progress.md`.
/// Detects dangling requirement/task IDs, duplicate task definitions, and unverified task claims.
pub fn validate_dataflow(files: &[Value], phase: &str, needed: usize) -> Vec<String> {
    use std::collections::{BTreeMap, BTreeSet};

    if needed == 0 {
        return vec![];
    }
    let find_doc = |suffix: &str| -> Option<&str> {
        files.iter().find_map(|f| {
            let p = f["path"].as_str()?;
            (p.ends_with(suffix) && f["available"] == true)
                .then(|| f["text"].as_str())
                .flatten()
        })
    };
    let spec = find_doc("spec.md");
    let plan = find_doc("plan.md");
    let tasks = find_doc("tasks.md");
    let progress = find_doc("progress.md");

    let mut errors = Vec::new();
    let req_re = regex::Regex::new(r"\b(?:FR|NFR|SC|AC)-\d+\b").expect("valid req regex");
    let task_id_re = regex::Regex::new(r"\bT-?\d{2,}\b").expect("valid task regex");

    // 1. Check requirement cross-references against `spec.md`.
    if let Some(spec_text) = spec {
        let declared_reqs: BTreeSet<String> = req_re
            .find_iter(spec_text)
            .map(|m| m.as_str().to_owned())
            .collect();
        if !declared_reqs.is_empty() {
            for (name, doc_opt) in [
                ("plan.md", plan),
                ("tasks.md", tasks),
                ("progress.md", progress),
            ] {
                let Some(text) = doc_opt else { continue };
                for m in req_re.find_iter(text) {
                    let id = m.as_str();
                    if !declared_reqs.contains(id) {
                        errors.push(format!(
                            "dangling requirement reference `{id}` in {name} (not in spec.md)"
                        ));
                    }
                }
            }
        }
    }

    // 2. Check task definitions, duplicates, dangling task references, and progress sync.
    if let Some(tasks_text) = tasks {
        let mut defined_tasks: BTreeMap<String, char> = BTreeMap::new();
        let mut pending_tasks = Vec::new();
        for line in tasks_text.lines() {
            let trimmed = line.trim_start();
            let checkbox = if trimmed.starts_with("- [ ]") || trimmed.starts_with("* [ ]") {
                Some(' ')
            } else if trimmed.starts_with("- [~]") || trimmed.starts_with("* [~]") {
                Some('~')
            } else if trimmed.starts_with("- [x]")
                || trimmed.starts_with("- [X]")
                || trimmed.starts_with("* [x]")
                || trimmed.starts_with("* [X]")
            {
                Some('x')
            } else {
                None
            };
            if let Some(state) = checkbox {
                // First task ID on a checkbox line is the defined task ID (others on line are deps).
                let mut ids = task_id_re.find_iter(trimmed).map(|m| m.as_str().to_owned());
                if let Some(first_id) = ids.next() {
                    if defined_tasks.contains_key(&first_id) {
                        errors.push(format!(
                            "duplicate task definition `{first_id}` in tasks.md"
                        ));
                    } else {
                        defined_tasks.insert(first_id.clone(), state);
                        if state != 'x' {
                            pending_tasks.push(first_id);
                        }
                    }
                } else if state != 'x' {
                    pending_tasks.push(trimmed.chars().take(60).collect());
                }
            }
        }

        if !defined_tasks.is_empty() {
            for m in task_id_re.find_iter(tasks_text) {
                let id = m.as_str();
                if !defined_tasks.contains_key(id) {
                    errors.push(format!(
                        "dangling task dependency `{id}` in tasks.md (no matching task definition)"
                    ));
                }
            }
            if let Some(progress_text) = progress {
                for line in progress_text.lines() {
                    let trimmed = line.trim_start();
                    let marked_done = trimmed.starts_with("- [x]")
                        || trimmed.starts_with("- [X]")
                        || trimmed.starts_with("* [x]")
                        || trimmed.starts_with("* [X]");
                    for m in task_id_re.find_iter(trimmed) {
                        let id = m.as_str();
                        match defined_tasks.get(id) {
                            None => errors.push(format!(
                                "dangling task reference `{id}` in progress.md (not in tasks.md)"
                            )),
                            Some(&task_state) if marked_done && task_state != 'x' => {
                                errors.push(format!(
                                    "task `{id}` marked complete in progress.md while unchecked in tasks.md"
                                ));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        if phase == "review" && !pending_tasks.is_empty() {
            let preview: Vec<String> = pending_tasks.into_iter().take(5).collect();
            errors.push(format!(
                "review stage requires all tasks verified complete in tasks.md (unchecked: {})",
                preview.join(", ")
            ));
        }
    }

    errors.into_iter().take(16).collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn gate1_dataflow_validates_cross_references_and_catches_dangling_and_unchecked_claims() {
        let dir = tempfile::tempdir().unwrap();
        let feature_dir = dir.path().join("specs/001-demo");
        std::fs::create_dir_all(&feature_dir).unwrap();
        std::fs::write(feature_dir.join("feature.md"), "# Demo Feature\n").unwrap();
        std::fs::write(
            feature_dir.join("spec.md"),
            "# Spec\n- FR-001: Support deterministic verify.\n- SC-001: Zero model calls on pass.\n",
        )
        .unwrap();
        std::fs::write(
            feature_dir.join("plan.md"),
            "# Plan\nCovers FR-001 cleanly.\n",
        )
        .unwrap();
        std::fs::write(
            feature_dir.join("tasks.md"),
            "- [x] T001 Implement FR-001\n- [x] T002 Verify SC-001 (depends on T001)\n",
        )
        .unwrap();
        std::fs::write(
            feature_dir.join("progress.md"),
            "- [x] T001 done\n- [x] T002 verified for SC-001\n",
        )
        .unwrap();

        let review_settings = json!({"feature": "001-demo", "phase": "review"});
        require_documents(dir.path(), &review_settings).unwrap();

        // Dangling FR in plan.md fails implement stage.
        std::fs::write(
            feature_dir.join("plan.md"),
            "# Plan\nCovers FR-001 and dangling FR-999.\n",
        )
        .unwrap();
        let err = require_documents(
            dir.path(),
            &json!({"feature": "001-demo", "phase": "tasks"}),
        )
        .unwrap_err();
        assert!(err.message.contains("FR-999"), "{}", err.message);

        // Restore valid plan.md, make T002 unchecked while progress.md claims complete -> rejected in review.
        std::fs::write(feature_dir.join("plan.md"), "# Plan\nCovers FR-001.\n").unwrap();
        std::fs::write(
            feature_dir.join("tasks.md"),
            "- [x] T001 Implement FR-001\n- [ ] T002 Verify SC-001 (depends on T001)\n",
        )
        .unwrap();
        let err = require_documents(dir.path(), &review_settings).unwrap_err();
        assert!(
            err.message.contains("T002") || err.message.contains("unchecked"),
            "{}",
            err.message
        );
    }
}
