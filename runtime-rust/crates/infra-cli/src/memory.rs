//! Rendering and invocation of the canonical memory command surface.
use crate::{arguments::Arguments, output::Output};
use infra_core::{MemoryService, Result, Roots, RuntimeConfig, RuntimeContext, RuntimeError};
use serde_json::Value;
use std::path::{Path, PathBuf};
pub fn tooling(request: &Value) -> Result<PathBuf> {
    if let Some(path) = request["_tooling_root"].as_str() {
        let path = Path::new(path);
        std::fs::metadata(path)?;
        return infra_core::fs::python_realpath(path);
    }
    let executable = std::env::current_exe()?;
    for directory in executable.ancestors().skip(1) {
        if directory
            .join("project-brain/config/runtime.json")
            .is_file()
            && directory.join("memory-bank").is_dir()
        {
            return infra_core::fs::python_realpath(directory);
        }
    }
    if let Some(path) = std::env::var_os("ACCELERATOR_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        && path.join("project-brain/config/runtime.json").is_file()
    {
        return infra_core::fs::python_realpath(&path);
    }
    if let Some(root) = request["root"].as_str() {
        std::fs::metadata(root)?;
        return infra_core::fs::python_realpath(Path::new(root));
    }
    for directory in std::env::current_dir()?.ancestors() {
        if directory
            .join("project-brain/config/runtime.json")
            .is_file()
        {
            return infra_core::fs::python_realpath(directory);
        }
    }
    Err(RuntimeError::unavailable(
        "Memory installation root unavailable; pass --root",
    ))
}
pub fn context(request: &Value) -> Result<RuntimeContext> {
    let tooling = tooling(request)?;
    let repository = request["root"]
        .as_str()
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("ACCELERATOR_STATE_DIR")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
        })
        .unwrap_or_else(|| tooling.clone());
    let mut roots = Roots::discover(&repository, &tooling)?;
    if let Some(project) = request["project"].as_str() {
        let project = Path::new(project);
        if !project.is_absolute() {
            return Err(RuntimeError::policy("--project must be absolute"));
        }
        std::fs::metadata(project)?;
        let project = infra_core::fs::python_realpath(project)?;
        if project != roots.project {
            // The folder --project names is where this run is, as Claude
            // Code's project directory or the working directory would be: a
            // save for a session in a worktree runs from the installation's
            // root, which alone does not show that worktree.
            if !roots.shared && !roots.attached() && served_worktree(&roots.tooling, &project)? {
                roots.shared = true;
            }
            if !roots.shared {
                return Err(RuntimeError::policy(
                    "--project names neither this project nor a worktree this installation serves",
                ));
            }
            let old = infra_core::git::GitFrame::discover(&roots.project)?
                .ok_or_else(|| RuntimeError::unavailable("Git source context unavailable"))?;
            let new = infra_core::git::GitFrame::discover(&project)?
                .ok_or_else(|| RuntimeError::unavailable("Git source context unavailable"))?;
            if old.common_dir()? != new.common_dir()? {
                return Err(RuntimeError::policy(
                    "Served project belongs to another repository",
                ));
            }
            roots.project = project;
        }
    }
    let actor = request["owner"]
        .as_str()
        .map(String::from)
        .or_else(|| std::env::var("PROJECT_BRAIN_OWNER").ok())
        .unwrap_or_else(|| "local".into());
    let mode = RuntimeConfig::load(&roots)?.mode(request["mode"].as_str())?;
    RuntimeContext::new(roots, actor, mode)
}
/// Whether `project` is the place, in a linked worktree of this installation's
/// repository, where the installation lies in its main checkout, with no engine
/// of its own: the worktree these hooks serve when a session works there.
fn served_worktree(tooling: &Path, project: &Path) -> Result<bool> {
    let (Some(mine), Some(theirs)) = (
        infra_core::git::GitFrame::discover(tooling)?,
        infra_core::git::GitFrame::discover(project)?,
    ) else {
        return Ok(false);
    };
    if mine.top == theirs.top || mine.common_dir()? != theirs.common_dir()? {
        return Ok(false);
    }
    let mine_top = infra_core::fs::python_realpath_spelling(tooling, mine.top.clone())?;
    let Ok(suffix) = tooling.strip_prefix(&mine_top) else {
        return Ok(false);
    };
    let theirs_top = infra_core::fs::python_realpath_spelling(project, theirs.top.clone())?;
    let engine = if cfg!(windows) {
        "memory-bank/bin/infra-engine.exe"
    } else {
        "memory-bank/bin/infra-engine"
    };
    Ok(theirs_top.join(suffix) == project && !project.join(engine).is_file())
}
fn value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => "None".into(),
        Value::Bool(value) => {
            if *value {
                "True".into()
            } else {
                "False".into()
            }
        }
        _ => value.to_string(),
    }
}
fn v(result: &Value, key: &str) -> String {
    value(&result[key])
}
fn rows<'a>(result: &'a Value, key: &str) -> &'a [Value] {
    result[key].as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn count(result: &Value, key: &str) -> usize {
    rows(result, key).len()
}
fn text(lines: Vec<String>) -> Output {
    if lines.is_empty() {
        Output::default()
    } else {
        Output::text(lines.join("\n") + "\n")
    }
}
fn tally(value: &Value) -> String {
    value
        .as_object()
        .map(|map| {
            map.iter()
                .map(|(key, value)| format!("{key} {}", self::value(value)))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}
fn source_notice(value: &Value) -> String {
    let parts = [
        ("invalid-utf8", "invalid UTF-8 content"),
        ("invalid-utf8-path", "invalid UTF-8 paths"),
    ]
    .into_iter()
    .filter_map(|(key, label)| {
        value[key]
            .as_u64()
            .filter(|n| *n > 0)
            .map(|n| format!("{label} {n}"))
    })
    .collect::<Vec<_>>();
    if parts.is_empty() {
        String::new()
    } else {
        format!("sources omitted: {}", parts.join("; "))
    }
}
pub fn render(operation: &str, request: &Value, result: Value) -> Result<Output> {
    if result.get("_cli").is_some() {
        return Output::envelope(result);
    }
    let json_output = request["json"] == true
        || [
            "source-fingerprints",
            "memory-review",
            "memory-review-controller",
            "native-result",
            "prepare-result",
            "cursor-rule",
            "cursor-rule-retire",
        ]
        .contains(&operation);
    let code = if matches!(operation, "validate" | "capsule") && result["valid"] == false
        || operation == "parity" && (result["valid"] == false || result["parity"] == "drift")
    {
        1
    } else {
        0
    };
    if json_output {
        let mut output = Output::json(&result)?;
        output.code = code;
        if operation == "cursor-rule"
            && request["diagnostic"] == true
            && ["preserved", "incomplete"].contains(&result["state"].as_str().unwrap_or(""))
            && !(request["action"] == "prompt-held" && result["state"] == "preserved")
        {
            let reason = if result["state"] == "incomplete" {
                "maintenance_incomplete"
            } else {
                result["reason"]
                    .as_str()
                    .filter(|reason| {
                        [
                            "configuration_unavailable",
                            "unsupported_custody",
                            "migration_unapproved",
                            "custody_changed",
                            "ownership_unverified",
                            "receipt_unavailable",
                            "linked_directory",
                        ]
                        .contains(reason)
                    })
                    .unwrap_or("custom_or_unavailable")
            };
            output.stderr = format!("Cursor memory rule was not updated: {reason}.\n").into_bytes();
        }
        return Ok(output);
    }
    let mut lines = vec![];
    match operation {
        "index" => {
            lines.push(format!(
                "Context index: {} documents ({} removed, {} reused).",
                v(&result, "documents"),
                v(&result, "removed"),
                v(&result, "reused")
            ));
            if !rows(&result, "excluded").is_empty() {
                let mut counts = std::collections::BTreeMap::<String, usize>::new();
                for row in rows(&result, "excluded") {
                    *counts
                        .entry(row["reason"].as_str().unwrap_or("unknown").into())
                        .or_default() += 1;
                }
                lines.push(format!(
                    "Excluded: {} document(s) ({}); --json lists the paths.",
                    count(&result, "excluded"),
                    counts
                        .iter()
                        .map(|(key, value)| format!("{key}: {value}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            if !rows(&result, "redacted").is_empty() {
                lines.push(format!(
                    "Redacted: {} value(s) in {} document(s); --json lists the paths.",
                    rows(&result, "redacted")
                        .iter()
                        .filter_map(|row| row["values"].as_u64())
                        .sum::<u64>(),
                    count(&result, "redacted")
                ));
            }
        }
        "record" => lines.push(format!(
            "Context episode recorded: {}.",
            v(&result, "episode_id")
        )),
        "start" | "update" | "clear" => lines.push(format!(
            "Working task {}: {}.",
            match operation {
                "start" => "started",
                "update" => "updated",
                _ => "cleared",
            },
            v(&result, "task_id")
        )),
        "get" => lines.push(format!(
            "Working task: {} — {}",
            v(&result, "task_id"),
            v(&result, "goal")
        )),
        "complete" => lines.push(format!(
            "Working task completed as episode: {}.",
            v(&result, "episode_id")
        )),
        "rebind" => lines.push(format!(
            "Working task {}: {} -> {} (revision {}).",
            if result["already_bound"] == true {
                "already bound"
            } else {
                "rebound"
            },
            v(&result, "task_id"),
            v(&result, "task_uuid"),
            v(&result, "revision")
        )),
        "brain-create" => lines.push(format!(
            "Project Brain {} created: {}.",
            v(&result, "type"),
            v(&result, "id")
        )),
        "brain-update" => {
            lines.push(format!(
                "Project Brain {} updated: {}.",
                v(&result, "type"),
                v(&result, "id")
            ));
            for row in rows(&result["promotion"], "promoted") {
                lines.push(format!(
                    "Promoted to durable memory: {}.",
                    v(row, "memory_id")
                ));
            }
            for row in rows(&result["promotion"], "blocked") {
                lines.push(format!("warning: not promoted: {}", v(row, "reason")));
            }
        }
        "brain-get" => lines.push(format!(
            "Project Brain {}: {} — {}",
            v(&result, "type"),
            v(&result, "external_id"),
            v(&result, "title")
        )),
        "search" | "links" => {
            for row in rows(&result, "documents") {
                lines.push(format!(
                    "{} {}: {} — {}",
                    v(row, "layer"),
                    v(row, "kind"),
                    v(row, "path"),
                    v(row, "title")
                ));
                if operation == "search" {
                    lines.push(format!("  {}", v(row, "snippet")));
                } else if row["ref_kind"] == "file" {
                    lines.push(format!("  touched {}", v(row, "ref_path")));
                } else {
                    lines.push(format!(
                        "  cites {} ({})",
                        v(row, "ref_path"),
                        v(row, "ref_kind")
                    ));
                }
            }
            if operation == "search" {
                for row in rows(&result, "episodes") {
                    lines.push(format!("episode {}: {}", v(row, "id"), v(row, "summary")));
                    lines.push(format!("  {}", v(row, "outcome")));
                }
            } else {
                if !rows(&result, "documents").is_empty() {
                    for row in rows(&result, "excluded") {
                        lines.push(format!("withheld {}: {}", v(row, "path"), v(row, "reason")));
                    }
                }
                if rows(&result, "documents").is_empty() {
                    lines.push(format!(
                        "No eligible document cites or touched {}.{}",
                        v(&result, "path"),
                        if count(&result, "excluded") > 0 {
                            format!(
                                " {} withheld by policy or freshness.",
                                count(&result, "excluded")
                            )
                        } else {
                            String::new()
                        }
                    ));
                }
            }
        }
        "context" | "retrieve" | "refresh" | "hook-context" => {
            return Err(RuntimeError::unavailable(
                "Typed canonical memory output is unavailable",
            ));
        }
        "turn" => {
            if result["flushed"] == true {
                lines.push(format!(
                    "Working task checkpointed: {} ({} turn(s)).",
                    v(&result, "task_id"),
                    v(&result, "turns")
                ));
            } else if result["buffered"] == true || result["flushed"] == false {
                lines.push(format!("Working turn buffered: {}.", v(&result, "task_id")));
            }
        }
        "status" => {
            let counts = &result["consolidation"];
            let available = |key: &str| {
                if counts[key].is_null() {
                    "unavailable".into()
                } else {
                    v(counts, key)
                }
            };
            lines.push(format!("Consolidation: {} promotable candidate(s), {} blocked, {} promotion(s) applied, {} durable chunk(s).",available("promotable"),available("blocked"),available("applied"),available("chunks")));
            let layers = ["procedural", "semantic", "episodic"]
                .into_iter()
                .map(|layer| format!("{layer}: {}", v(&result["layers"], layer)))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!(
                "Context index: {} documents, {} episodes, {} working tasks ({}; {}).",
                v(&result, "documents"),
                v(&result, "episodes"),
                v(&result, "working"),
                layers,
                v(&result, "database")
            ));
            lines.push(format!(
                "Automatic memory: {} — {}",
                v(&result["automatic_memory"], "status"),
                v(&result["automatic_memory"], "reason")
            ));
            if let Some(remediation) = result["automatic_memory"]["remediation"]
                .as_str()
                .filter(|value| !value.is_empty())
            {
                lines.push(format!("Remediation: {remediation}"));
            }
        }
        "validate" => {
            for error in rows(&result, "errors") {
                lines.push(value(error));
            }
            if rows(&result, "errors").is_empty() {
                lines.push("Project Brain validation passed.".into());
            }
            for row in rows(&result, "stale_states") {
                let path = v(row, "path");
                let reading = match row["state"].as_str() {
                    Some("source-missing") => {
                        "a cited source no longer exists; retrieval leaves the record out"
                    }
                    Some("source-undigested") => {
                        "a cited source has no stored digest; retrieval leaves the record out"
                    }
                    _ => {
                        "a cited source changed after the record was written; retrieval serves it marked for checking"
                    }
                };
                let archived = if path.replace('\\', "/").contains("/project-brain/archive/") {
                    " (archived: never retrieved)"
                } else {
                    ""
                };
                lines.push(format!("Warning: {path}: {reading}{archived}"));
            }
        }
        "parity" => {
            if code == 0 {
                lines.push(if request["cross_edition"] == true {
                    "Cross-edition core parity passed.".into()
                } else {
                    format!(
                        "{} parity passed ({} canonical).",
                        if request["skills_only"] == true {
                            "Skill"
                        } else {
                            "Full mirror"
                        },
                        result["canonical"].as_str().unwrap_or(".agents")
                    )
                });
            } else {
                for row in rows(&result, "drift") {
                    lines.push(value(row));
                }
            }
        }
        "compact" => lines.push(format!(
            "Project Brain compacted: {} record(s) archived.",
            v(&result, "moved")
        )),
        "consolidate" => {
            if request["apply"] == true {
                lines.push(format!(
                    "Created {} finding(s), updated {}; {} event(s) superseded, {} blocked.",
                    v(&result, "created"),
                    v(&result, "updated"),
                    v(&result, "superseded"),
                    v(&result, "blocked")
                ));
            } else {
                let tickets = rows(&result, "tickets");
                lines.push(format!(
                    "Planned {} finding(s), {} update(s); {} event(s) to supersede.",
                    tickets
                        .iter()
                        .filter(|row| row["state"] == "planned")
                        .count(),
                    tickets
                        .iter()
                        .filter(|row| row["state"] == "planned-update")
                        .count(),
                    tickets
                        .iter()
                        .filter_map(|row| row["supersede"].as_u64())
                        .sum::<u64>()
                ));
            }
            for row in rows(&result, "tickets") {
                lines.push(format!(
                    "  {}: {} ({} event(s)){}",
                    v(row, "ticket"),
                    v(row, "state"),
                    v(row, "events"),
                    row["reason"]
                        .as_str()
                        .map(|reason| format!(" - {reason}"))
                        .unwrap_or_default()
                ));
            }
        }
        "bank-audit" => {
            lines.push(format!("Memory bank audit: {} chunk(s), {} with a changed source, {} overdue for review, {} without source digests, {} near-duplicate pair(s).",v(&result,"chunks"),count(&result,"source_changed"),count(&result,"overdue_review"),count(&result,"undigested"),count(&result,"duplicates")));
            for row in rows(&result, "duplicates") {
                lines.push(format!(
                    "  near-duplicate: {} ~ {} ({})",
                    v(row, "left"),
                    v(row, "right"),
                    v(row, "similarity")
                ));
            }
            for row in rows(&result, "source_changed") {
                lines.push(format!(
                    "  source-{}: {} cites {}",
                    v(row, "state"),
                    v(row, "chunk"),
                    v(row, "source")
                ));
            }
            for row in rows(&result, "overdue_review") {
                lines.push(format!(
                    "  overdue-review: {} (due {})",
                    v(row, "chunk"),
                    v(row, "review_after")
                ));
            }
            for row in rows(&result, "undigested") {
                lines.push(format!("  undigested: {}", value(row)));
            }
            if let Some(provenance) = result.get("provenance") {
                lines.push(format!("  provenance: {} auto-promoted, {} with external sources, {} instruction-like.",count(provenance,"auto_promoted"),count(provenance,"external_sources"),count(provenance,"instruction_like")));
            }
        }
        "bank-reverify" => lines.push(format!(
            "Re-verified {} against {} source(s) on {}; next review {}.",
            v(&result, "id"),
            count(&result, "source_digests"),
            v(&result, "last_verified"),
            v(&result, "review_after")
        )),
        "bank-retire" => lines.push(format!(
            "Retired {}: {} as of {}{}. Index rewritten with {} chunk row(s).",
            v(&result, "id"),
            v(&result, "status"),
            v(&result, "valid_to"),
            result["superseded_by"]
                .as_str()
                .map(|id| format!(", superseded by {id}"))
                .unwrap_or_default(),
            v(&result, "chunks")
        )),
        "bank-quarantine" => lines.push(format!(
            "Quarantined {}: {} as of {}; index rewritten with {} chunk row(s).",
            v(&result, "id"),
            v(&result, "status"),
            v(&result, "valid_to"),
            v(&result, "chunks")
        )),
        "reindex-bank" => lines.push(format!(
            "Memory bank index {}: {} chunk row(s).",
            if result["changed"] == true {
                "regenerated"
            } else {
                "already up to date"
            },
            v(&result, "chunks")
        )),
        "promote-propose" => lines.push(format!("Promotion proposed: {}.", v(&result, "id"))),
        "promote-review" => lines.push(format!(
            "Promotion review recorded: {}.",
            v(&result, "status")
        )),
        "promote-apply" => lines.push(format!(
            "Promotion applied: {}.",
            v(&result, "destination_memory_id")
        )),
        "promote-auto" => {
            if result["enabled"] == false {
                lines.push("Automatic promotion is disabled in runtime.json.".into());
            } else {
                lines.push(format!(
                    "Promoted {} record(s); {} blocked, {} failed, {} deferred.",
                    count(&result, "promoted"),
                    count(&result, "blocked"),
                    count(&result, "failed"),
                    v(&result, "skipped")
                ));
            }
        }
        "export" => {
            lines.push(format!(
                "Exported {} Brain record(s) and {} Memory Bank chunk(s) to {}.",
                v(&result, "brain_records"),
                v(&result, "memory_chunks"),
                v(&result, "destination")
            ));
            if result["excluded"].as_u64().unwrap_or(0) > 0 {
                lines.push(format!(
                    "  {} item(s) excluded; see MANIFEST.json for each reason.",
                    v(&result, "excluded")
                ));
            }
            if result["auto_promoted_chunks"].as_u64().unwrap_or(0) > 0 {
                lines.push(format!(
                    "  {} chunk(s) were promoted automatically and never human-reviewed.",
                    v(&result, "auto_promoted_chunks")
                ));
            }
        }
        "context-save" => lines.push(format!("Context handoff saved: {}.", v(&result, "output"))),
        "context-load" => {
            lines.push(format!("Context handoff loaded: {}.", v(&result, "input")));
            if count(&result, "drift") > 0 {
                lines.push(format!(
                    "  {} source drift item(s); review before continuing.",
                    count(&result, "drift")
                ));
                lines.push(serde_json::to_string_pretty(&result["drift"])?);
            }
            lines.push(infra_core::handoff::render_body(&result["context"])?);
            if let Some(transcript) = result["transcript"].as_str() {
                // Written as it was saved, as the retired command wrote it:
                // the line break that ends it is its own.
                lines.extend([
                    "## Visible transcript\n".into(),
                    transcript.strip_suffix('\n').unwrap_or(transcript).into(),
                ]);
            }
        }
        "msg-send" => lines.push(format!(
            "Message {} recorded: {} -> {}.",
            v(&result, "seq"),
            v(&result, "from_actor"),
            v(&result, "to_actor")
        )),
        "msg-dispatch" => lines.push(format!(
            "Dispatch {} recorded: {} {}.",
            v(&result, "seq"),
            v(request, "event"),
            v(request, "agent")
        )),
        "msg-read" => {
            let messages = result
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_else(|| rows(&result, "messages"));
            if messages.is_empty() {
                lines.push("No messages.".into());
            }
            for row in messages {
                let body = v(row, "body")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                let body = if body.chars().count() > 100 {
                    body.chars().take(97).collect::<String>() + "..."
                } else {
                    body
                };
                lines.push(format!(
                    "{:>4} {} [{}] {} -> {}: {body}",
                    v(row, "seq"),
                    v(row, "created_at"),
                    v(row, "type"),
                    v(row, "from_actor"),
                    v(row, "to_actor")
                ));
            }
        }
        "capsule" => {
            if result["valid"] == false {
                lines.push("Delegation capsule is under-specified:".into());
                for row in rows(&result, "problems") {
                    lines.push(format!("  - {}", value(row)));
                }
            } else {
                lines.push(format!(
                    "Delegation capsule is valid ({} characters).",
                    v(&result, "length")
                ));
            }
        }
        "record-result" | "replay-result" => {
            let states = rows(&result, "records")
                .iter()
                .map(|row| {
                    format!(
                        "{} {:?} {}",
                        v(row, "type"),
                        v(row, "title"),
                        v(row, "state")
                    )
                })
                .collect::<Vec<_>>();
            lines.push(format!(
                "{}: {}.",
                if result["replayed"] == true {
                    "Result already recorded"
                } else {
                    "Result recorded"
                },
                if states.is_empty() {
                    "no learnings".into()
                } else {
                    states.join(", ")
                }
            ));
        }
        "health" | "retrieval-report" => {
            let turns = result["turns"].as_u64().unwrap_or(0);
            if turns == 0 {
                lines.push(if operation == "health" {
                    "No refresh health records yet (memory-bank/local/refresh-health.ndjson)."
                        .into()
                } else {
                    format!(
                        "No retrieval manifests in scope {}. Lightweight mode writes none.",
                        request["scope"].as_str().unwrap_or("all")
                    )
                });
            } else if operation == "health" {
                lines.push(format!("Refresh health ({turns} turn(s)):"));
                if let Some(phases) = result["phase_seconds"].as_object() {
                    for (name, band) in pipeline_order(phases) {
                        lines.push(format!(
                            "  phase {name}: p50 {} p95 {} max {} ({} sample(s))",
                            v(band, "p50"),
                            v(band, "p95"),
                            v(band, "max"),
                            v(band, "samples")
                        ));
                    }
                }
                lines.push(format!(
                    "  timeouts: {}; lossy capsules: {}; incomplete layers: {}",
                    v(&result, "timeout_rate"),
                    v(&result, "lossy_capsule_rate"),
                    v(&result, "incomplete_layers_rate")
                ));
                let status = tally(&result["hook_status"]);
                if !status.is_empty() {
                    lines.push(format!("  hook status: {status}"));
                }
                if result["source_notices"].is_object() {
                    let notices = &result["source_notices"];
                    let notice = source_notice(&notices["totals"]);
                    lines.push(format!("  {notice} ({} record(s))", v(notices, "records")));
                }
            } else {
                lines.push(format!(
                    "Retrieval report ({turns} turn(s), scope {}): {} with an empty selection.",
                    v(&result, "scope"),
                    v(&result, "empty_selection")
                ));
                let query = tally(&result["query_source"]);
                lines.push(format!(
                    "  query source: {}",
                    if query.is_empty() {
                        "not recorded".into()
                    } else {
                        query
                    }
                ));
                let gate = &result["gate"];
                let rate = |key: &str| {
                    if gate[key].is_null() {
                        "n/a".into()
                    } else {
                        v(gate, key)
                    }
                };
                let reasons = tally(&gate["skip_reasons"]);
                lines.push(format!(
                    "  gate: {} decided; shadow would skip {}; enforce withheld {}{}",
                    v(gate, "decided"),
                    rate("would_skip_rate"),
                    rate("withheld_rate"),
                    if reasons.is_empty() {
                        String::new()
                    } else {
                        format!(" ({reasons})")
                    }
                ));
                for label in ["top_candidate_score", "top_delivered_score"] {
                    let band = &result[label];
                    lines.push(format!(
                        "  {}: p50 {} p95 {} max {} ({} sample(s))",
                        label.replace('_', " "),
                        v(band, "p50"),
                        v(band, "p95"),
                        v(band, "max"),
                        v(band, "samples")
                    ));
                }
                if let Some(phases) = result["phase_seconds"].as_object() {
                    for (name, band) in pipeline_order(phases) {
                        lines.push(format!(
                            "  phase {name}: p50 {} p95 {} ({} sample(s))",
                            v(band, "p50"),
                            v(band, "p95"),
                            v(band, "samples")
                        ));
                    }
                }
                for (key, label) in [
                    ("excluded_reasons", "excluded"),
                    ("match_strength", "match"),
                ] {
                    let counts = tally(&result[key]);
                    if !counts.is_empty() {
                        lines.push(format!("  {label}: {counts}"));
                    }
                }
                if let Some(paths) = result["top_paths"].as_object() {
                    for (path, count) in paths {
                        lines.push(format!("    {:>4}  {path}", value(count)));
                    }
                }
            }
        }
        "native-prompt" | "native-stop" => {
            return Err(RuntimeError::unavailable(
                "Native raw rendering envelope missing",
            ));
        }
        _ => {
            return Err(RuntimeError::invalid(format!(
                "Missing memory renderer: {operation}"
            )));
        }
    }
    let mut output = text(lines);
    output.code = code;
    Ok(output)
}
pub fn execute(arguments: Arguments, input: &[u8]) -> Result<Output> {
    execute_mode(arguments, input, false)
}
pub fn execute_in_eval(arguments: Arguments, input: &[u8]) -> Result<Output> {
    execute_mode(arguments, input, true)
}
fn execute_mode(arguments: Arguments, input: &[u8], controlled: bool) -> Result<Output> {
    let mut request = arguments.value;
    if let Some(path) = request["body_file"].as_str() {
        if request["body"].is_string() {
            return Err(RuntimeError::invalid(
                "Pass --body or --body-file, not both",
            ));
        }
        request["body"] = infra_core::fs::read_text(Path::new(path))?.into();
    }
    for field in ["limit", "flush_after", "max_files"] {
        if request[field].as_i64().is_some_and(|value| value < 1) {
            return Err(RuntimeError::invalid(format!(
                "--{} must be a positive integer",
                field.replace('_', "-")
            )));
        }
    }
    if request["provenance_response"] == true && request["json"] != true {
        return Err(RuntimeError::invalid(
            "Source provenance transport requires JSON output",
        ));
    }
    let context = context(&request)?;
    let database = request["db"].as_str().map(PathBuf::from);
    let service = MemoryService::new(context, database);
    let service = if controlled {
        service.with_eval_no_child()
    } else {
        service.with_queue_engine(std::env::current_exe()?)?
    };
    let command = service.execute_cli(&arguments.operation, &request, input)?;
    let output = if let Some(stdout) = command.stdout {
        Output {
            code: command.code,
            stdout: stdout.into_bytes(),
            stderr: command.stderr.into_bytes(),
            after_delivery: command.after_delivery,
        }
    } else {
        let mut output = render(&arguments.operation, &request, command.value)?;
        output.code = command.code.max(output.code);
        output.stderr.extend_from_slice(command.stderr.as_bytes());
        output.after_delivery = command.after_delivery;
        output
    };
    Ok(output)
}

/// Phases in the order a refresh runs them, as the retired reports listed
/// them; a JSON object holds its keys sorted, which put `stat` last.
fn pipeline_order(phases: &serde_json::Map<String, Value>) -> Vec<(&String, &Value)> {
    const PIPELINE: [&str; 3] = ["stat", "index", "retrieval"];
    let mut ordered = PIPELINE
        .iter()
        .filter_map(|name| phases.get_key_value(*name))
        .collect::<Vec<_>>();
    ordered.extend(
        phases
            .iter()
            .filter(|(name, _)| !PIPELINE.contains(&name.as_str())),
    );
    ordered
}
