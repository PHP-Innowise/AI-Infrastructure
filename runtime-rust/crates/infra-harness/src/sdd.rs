use crate::{HarnessError, Result, store};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{Mutex, OnceLock},
};
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
    let ir_context = settings
        .get("ir_state")
        .and_then(Value::as_str)
        .zip(settings.get("project_id").and_then(Value::as_str))
        .map(|(state, project)| (Path::new(state), project));
    let violations =
        validate_dataflow_diagnostics_in(Some(root), ir_context, &all_artifacts, phase, needed);
    if !violations.is_empty() {
        return Err(HarnessError::invalid(format!(
            "SDD static cross-reference check failed: {}",
            format_bounded_violations(&violations)
        )));
    }
    Ok(())
}

/// Structured bounded-repair diagnostic emitted by [`validate_dataflow_diagnostics`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Violation {
    pub code: String,
    pub path: String,
    pub line: Option<usize>,
    pub message: String,
    pub hint: String,
}

fn format_bounded_violations(violations: &[Violation]) -> String {
    violations
        .iter()
        .take(16)
        .map(|v| match v.line {
            Some(line) => format!(
                "[{}:{}] {}: {} ({})",
                v.path, line, v.code, v.message, v.hint
            ),
            None => format!("[{}] {}: {} ({})", v.path, v.code, v.message, v.hint),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn dataflow_cache_digest(
    files: &[Value],
    phase: &str,
    needed: usize,
    context_stamp: &str,
) -> String {
    let entries: Vec<Value> = files
        .iter()
        .map(|f| {
            let text = f["text"].as_str().unwrap_or("");
            json!({
                "path": f["path"].as_str().unwrap_or(""),
                "available": f["available"].as_bool() == Some(true),
                "digest": store::sha(text.as_bytes()),
            })
        })
        .collect();
    store::sha(
        json!({
            "phase": phase,
            "needed": needed,
            "context_stamp": context_stamp,
            "files": entries
        })
        .to_string()
        .as_bytes(),
    )
}

#[cfg(test)]
static SDD_CACHE_EVALUATIONS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Gate-1-style static dataflow validator across `spec.md -> plan.md -> tasks.md -> progress.md`.
/// Detects dangling requirement/task IDs, duplicate task definitions, and unverified task claims.
pub fn validate_dataflow(files: &[Value], phase: &str, needed: usize) -> Vec<String> {
    validate_dataflow_diagnostics(files, phase, needed)
        .into_iter()
        .map(|v| v.message)
        .collect()
}

/// Returns cached structured [`Violation`] diagnostics across SDD artifacts for bounded repair.
pub fn validate_dataflow_diagnostics(
    files: &[Value],
    phase: &str,
    needed: usize,
) -> Vec<Violation> {
    validate_dataflow_diagnostics_in(None, None, files, phase, needed)
}

/// Returns cached structured [`Violation`] diagnostics across SDD artifacts plus optional
/// workspace file and `GraphIrStore::open_readonly_cached` symbol claim cross-checks.
pub fn validate_dataflow_diagnostics_in(
    root: Option<&Path>,
    ir_context: Option<(&Path, &str)>,
    files: &[Value],
    phase: &str,
    needed: usize,
) -> Vec<Violation> {
    if needed == 0 {
        return vec![];
    }
    let ir_store = ir_context.and_then(|(state, project)| {
        crate::graph_ir::GraphIrStore::open_readonly_cached(state, project)
            .ok()
            .flatten()
    });
    let claims = extract_checked_claims(files);
    let context_stamp = workspace_ir_claim_stamp(root, ir_store.as_deref(), &claims);

    static CACHE: OnceLock<Mutex<BTreeMap<String, Vec<Violation>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(BTreeMap::new()));
    let digest = dataflow_cache_digest(files, phase, needed, &context_stamp);
    if let Ok(guard) = cache.lock()
        && let Some(hit) = guard.get(&digest)
    {
        return hit.clone();
    }

    #[cfg(test)]
    SDD_CACHE_EVALUATIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let mut result = compute_dataflow_diagnostics(files, phase);
    if (root.is_some() || ir_store.is_some()) && !claims.is_empty() {
        verify_checked_claims(root, ir_store.as_deref(), &claims, &mut result);
        result.truncate(16);
    }
    if let Ok(mut guard) = cache.lock() {
        if guard.len() >= 128 {
            guard.clear();
        }
        guard.insert(digest, result.clone());
    }
    result
}

#[derive(Clone, Debug)]
struct CheckedClaimLine {
    doc_path: String,
    line_no: usize,
    files: Vec<String>,
    symbols: Vec<String>,
}

fn is_claim_file_token(token: &str) -> bool {
    const EXTS: &[&str] = &[
        ".rs", ".php", ".ts", ".tsx", ".js", ".jsx", ".py", ".go", ".md", ".json", ".toml",
        ".yaml", ".yml", ".sql", ".sh",
    ];
    let trimmed = token.trim().trim_start_matches("./");
    !trimmed.is_empty()
        && trimmed.contains('/')
        && !trimmed.starts_with('/')
        && !trimmed.contains("://")
        && !trimmed.contains('\\')
        && !trimmed.contains("..")
        && !trimmed
            .chars()
            .any(|c| c.is_whitespace() || "*?[]{}<>\"'$()".contains(c))
        && EXTS.iter().any(|ext| trimmed.ends_with(ext))
}

fn is_claim_symbol_token(token: &str) -> bool {
    let trimmed = token.trim();
    if trimmed.is_empty()
        || trimmed.contains(['/', ' ', '\t', '\n', '(', ')', '{', '}', '*', '$'])
        || is_claim_file_token(trimmed)
    {
        return false;
    }
    // Qualified symbol reference (`Owner::member`, `ns\Class::method`) with valid identifier segments.
    trimmed.contains("::")
        && trimmed.split("::").all(|part| {
            let p = part.trim_start_matches('\\');
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'\\'))
        })
}

fn extract_checked_claims(files: &[Value]) -> Vec<CheckedClaimLine> {
    static BACKTICK_RE: OnceLock<regex::Regex> = OnceLock::new();
    let backtick_re = BACKTICK_RE
        .get_or_init(|| regex::Regex::new(r"`([^`\n]{2,180})`").expect("valid backtick regex"));

    let mut out = Vec::new();
    for suffix in ["tasks.md", "progress.md"] {
        let Some((doc_path, text)) = files.iter().find_map(|f| {
            let p = f["path"].as_str()?;
            (p.ends_with(suffix) && f["available"] == true)
                .then(|| f["text"].as_str().map(|t| (p, t)))
                .flatten()
        }) else {
            continue;
        };
        for (idx, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            let is_done = trimmed.starts_with("- [x]")
                || trimmed.starts_with("- [X]")
                || trimmed.starts_with("* [x]")
                || trimmed.starts_with("* [X]");
            if !is_done {
                continue;
            }
            let mut claimed_files = Vec::new();
            let mut claimed_symbols = Vec::new();
            for cap in backtick_re.captures_iter(trimmed) {
                let raw = cap.get(1).map(|m| m.as_str()).unwrap_or("");
                let token = raw.trim();
                if is_claim_file_token(token) {
                    let norm = token.trim_start_matches("./").to_owned();
                    if !claimed_files.contains(&norm) {
                        claimed_files.push(norm);
                    }
                } else if is_claim_symbol_token(token) {
                    let sym = token.to_owned();
                    if !claimed_symbols.contains(&sym) {
                        claimed_symbols.push(sym);
                    }
                }
            }
            if !claimed_files.is_empty() || !claimed_symbols.is_empty() {
                out.push(CheckedClaimLine {
                    doc_path: doc_path.to_owned(),
                    line_no: idx + 1,
                    files: claimed_files,
                    symbols: claimed_symbols,
                });
            }
        }
    }
    out
}

fn workspace_ir_claim_stamp(
    root: Option<&Path>,
    ir: Option<&crate::graph_ir::GraphIrStore>,
    claims: &[CheckedClaimLine],
) -> String {
    if root.is_none() && ir.is_none() {
        return String::new();
    }
    let mut unique_files = BTreeSet::new();
    for line in claims {
        unique_files.extend(line.files.iter().cloned());
    }
    let file_stamps: Vec<String> = unique_files
        .into_iter()
        .map(|rel| {
            let meta = root
                .and_then(|r| {
                    let p = r.join(&rel);
                    infra_core::fs::admit_descendant(r, &p).ok()?;
                    std::fs::symlink_metadata(&p).ok()
                })
                .filter(|m| !m.file_type().is_symlink() && m.is_file());
            match meta {
                Some(m) => {
                    let mtime_ns = m
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0, |d| d.as_nanos());
                    format!("{rel}:1:{}:{mtime_ns}", m.len())
                }
                None => format!("{rel}:0"),
            }
        })
        .collect();
    let ir_epoch = ir
        .map(|store| format!("ir:{}", store.current_write_epoch()))
        .unwrap_or_else(|| "ir:none".into());
    let root_label = root
        .map(|r| r.to_string_lossy().into_owned())
        .unwrap_or_default();
    store::sha(
        json!({
            "root": root_label,
            "ir": ir_epoch,
            "files": file_stamps,
        })
        .to_string()
        .as_bytes(),
    )
}

fn symbol_matches_ir_keys(symbol: &str, keys: &BTreeMap<String, String>) -> bool {
    let normalized = symbol.rsplit('\\').next().unwrap_or(symbol);
    let target = format!("::{normalized}");
    keys.keys().any(|k| k.ends_with(&target))
}

fn verify_checked_claims(
    root: Option<&Path>,
    ir: Option<&crate::graph_ir::GraphIrStore>,
    claims: &[CheckedClaimLine],
    violations: &mut Vec<Violation>,
) {
    let known_ir_files = ir
        .and_then(|store| store.known_files().ok())
        .unwrap_or_default();
    let ir_symbol_cache =
        if !known_ir_files.is_empty() && claims.iter().any(|c| !c.symbols.is_empty()) {
            ir.and_then(|store| store.canonical_symbol_ids_for_files(&known_ir_files).ok())
                .unwrap_or_default()
        } else {
            BTreeMap::new()
        };

    for claim in claims {
        let mut verified_files = BTreeSet::new();
        for rel in &claim.files {
            let on_disk = root.is_some_and(|r| {
                let candidate = r.join(rel);
                infra_core::fs::admit_descendant(r, &candidate).is_ok()
                    && std::fs::symlink_metadata(&candidate)
                        .is_ok_and(|m| !m.file_type().is_symlink() && m.is_file())
            });
            let in_ir = known_ir_files.contains(rel);
            if on_disk || in_ir {
                verified_files.insert(rel.clone());
            } else {
                violations.push(Violation {
                    code: "SDD_CLAIMED_FILE_MISSING".into(),
                    path: claim.doc_path.clone(),
                    line: Some(claim.line_no),
                    message: format!(
                        "checked task claims missing file `{rel}` not found in workspace or Graph-IR"
                    ),
                    hint: format!(
                        "Create `{rel}` or update/uncheck the task claim in `{}`.",
                        claim.doc_path
                    ),
                });
            }
        }

        if claim.symbols.is_empty() {
            continue;
        }
        let scoped_keys: BTreeMap<String, String> =
            if !verified_files.is_empty() && !ir_symbol_cache.is_empty() {
                ir_symbol_cache
                    .iter()
                    .filter(|(k, _)| {
                        verified_files
                            .iter()
                            .any(|rel| k.starts_with(&format!("{rel}::")))
                    })
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            } else {
                ir_symbol_cache.clone()
            };

        for sym in &claim.symbols {
            let ir_hit = !ir_symbol_cache.is_empty() && symbol_matches_ir_keys(sym, &scoped_keys);
            if ir_hit {
                continue;
            }
            if !ir_symbol_cache.is_empty() {
                violations.push(Violation {
                    code: "SDD_CLAIMED_SYMBOL_MISSING".into(),
                    path: claim.doc_path.clone(),
                    line: Some(claim.line_no),
                    message: format!(
                        "checked task claims symbol `{sym}` not found in Graph-IR index"
                    ),
                    hint: format!(
                        "Verify `{sym}` exists and re-sync Graph-IR, or uncheck the claim in `{}`.",
                        claim.doc_path
                    ),
                });
                continue;
            }
            // No Graph-IR DB yet: fall back to checking verified claimed files on disk if present.
            if let Some(r) = root
                && !verified_files.is_empty()
            {
                let (owner, member) = sym
                    .rsplit_once("::")
                    .map(|(o, m)| (o.rsplit('\\').next().unwrap_or(o), m))
                    .unwrap_or(("", sym.as_str()));
                let found_on_disk = verified_files.iter().any(|rel| {
                    infra_core::fs::read_regular(&r.join(rel))
                        .ok()
                        .and_then(|b| String::from_utf8(b).ok())
                        .is_some_and(|text| {
                            text.contains(member) && (owner.is_empty() || text.contains(owner))
                        })
                });
                if !found_on_disk {
                    violations.push(Violation {
                        code: "SDD_CLAIMED_SYMBOL_MISSING".into(),
                        path: claim.doc_path.clone(),
                        line: Some(claim.line_no),
                        message: format!(
                            "checked task claims symbol `{sym}` not found in claimed workspace file(s)"
                        ),
                        hint: format!(
                            "Implement `{sym}` in the claimed file or update `{}`.",
                            claim.doc_path
                        ),
                    });
                }
            }
        }
    }
}

fn compute_dataflow_diagnostics(files: &[Value], phase: &str) -> Vec<Violation> {
    static REQ_RE: OnceLock<regex::Regex> = OnceLock::new();
    static TASK_ID_RE: OnceLock<regex::Regex> = OnceLock::new();

    let find_doc = |suffix: &str| -> Option<(&str, &str)> {
        files.iter().find_map(|f| {
            let p = f["path"].as_str()?;
            (p.ends_with(suffix) && f["available"] == true)
                .then(|| f["text"].as_str().map(|t| (p, t)))
                .flatten()
        })
    };
    let spec = find_doc("spec.md");
    let plan = find_doc("plan.md");
    let tasks = find_doc("tasks.md");
    let progress = find_doc("progress.md");

    let mut violations = Vec::new();
    let req_re = REQ_RE
        .get_or_init(|| regex::Regex::new(r"\b(?:FR|NFR|SC|AC)-\d+\b").expect("valid req regex"));
    let task_id_re =
        TASK_ID_RE.get_or_init(|| regex::Regex::new(r"\bT-?\d{2,}\b").expect("valid task regex"));

    // 1. Check requirement cross-references against `spec.md`.
    if let Some((_, spec_text)) = spec {
        let declared_reqs: BTreeSet<String> = req_re
            .find_iter(spec_text)
            .map(|m| m.as_str().to_owned())
            .collect();
        if !declared_reqs.is_empty() {
            for (fallback_name, doc_opt) in [
                ("plan.md", plan),
                ("tasks.md", tasks),
                ("progress.md", progress),
            ] {
                let Some((doc_path, text)) = doc_opt else {
                    continue;
                };
                for (idx, line) in text.lines().enumerate() {
                    let line_no = idx + 1;
                    for m in req_re.find_iter(line) {
                        let id = m.as_str();
                        if !declared_reqs.contains(id) {
                            violations.push(Violation {
                                code: "SDD_DANGLING_REQUIREMENT".into(),
                                path: doc_path.to_owned(),
                                line: Some(line_no),
                                message: format!(
                                    "dangling requirement reference `{id}` in {fallback_name} (not in spec.md)"
                                ),
                                hint: format!(
                                    "Define `{id}` in spec.md or update the reference in {fallback_name}."
                                ),
                            });
                        }
                    }
                }
            }
        }
    }

    // 2. Check task definitions, duplicates, dangling task references, and progress sync.
    if let Some((tasks_path, tasks_text)) = tasks {
        let mut defined_tasks: BTreeMap<String, char> = BTreeMap::new();
        let mut pending_tasks = Vec::new();
        for (idx, line) in tasks_text.lines().enumerate() {
            let line_no = idx + 1;
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
                        violations.push(Violation {
                            code: "SDD_DUPLICATE_TASK".into(),
                            path: tasks_path.to_owned(),
                            line: Some(line_no),
                            message: format!("duplicate task definition `{first_id}` in tasks.md"),
                            hint: format!(
                                "Rename or remove the duplicate `{first_id}` definition in tasks.md."
                            ),
                        });
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
            for (idx, line) in tasks_text.lines().enumerate() {
                let line_no = idx + 1;
                for m in task_id_re.find_iter(line) {
                    let id = m.as_str();
                    if !defined_tasks.contains_key(id) {
                        violations.push(Violation {
                            code: "SDD_DANGLING_TASK_DEP".into(),
                            path: tasks_path.to_owned(),
                            line: Some(line_no),
                            message: format!(
                                "dangling task dependency `{id}` in tasks.md (no matching task definition)"
                            ),
                            hint: format!(
                                "Add a task checkbox defining `{id}` in tasks.md or fix the dependency ID."
                            ),
                        });
                    }
                }
            }
            if let Some((progress_path, progress_text)) = progress {
                for (idx, line) in progress_text.lines().enumerate() {
                    let line_no = idx + 1;
                    let trimmed = line.trim_start();
                    let marked_done = trimmed.starts_with("- [x]")
                        || trimmed.starts_with("- [X]")
                        || trimmed.starts_with("* [x]")
                        || trimmed.starts_with("* [X]");
                    for m in task_id_re.find_iter(trimmed) {
                        let id = m.as_str();
                        match defined_tasks.get(id) {
                            None => violations.push(Violation {
                                code: "SDD_DANGLING_PROGRESS_TASK".into(),
                                path: progress_path.to_owned(),
                                line: Some(line_no),
                                message: format!(
                                    "dangling task reference `{id}` in progress.md (not in tasks.md)"
                                ),
                                hint: format!(
                                    "Define `{id}` in tasks.md or remove the stale reference in progress.md."
                                ),
                            }),
                            Some(&task_state) if marked_done && task_state != 'x' => {
                                violations.push(Violation {
                                    code: "SDD_PROGRESS_STATUS_MISMATCH".into(),
                                    path: progress_path.to_owned(),
                                    line: Some(line_no),
                                    message: format!(
                                        "task `{id}` marked complete in progress.md while unchecked in tasks.md"
                                    ),
                                    hint: format!(
                                        "Mark `{id}` as `[x]` in tasks.md after verifying completion, or uncheck it in progress.md."
                                    ),
                                });
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        if phase == "review" && !pending_tasks.is_empty() {
            let preview: Vec<String> = pending_tasks.into_iter().take(5).collect();
            violations.push(Violation {
                code: "SDD_REVIEW_UNCHECKED_TASKS".into(),
                path: tasks_path.to_owned(),
                line: None,
                message: format!(
                    "review stage requires all tasks verified complete in tasks.md (unchecked: {})",
                    preview.join(", ")
                ),
                hint: "Complete and mark `[x]` for all unchecked tasks in tasks.md before review."
                    .into(),
            });
        }
    }

    violations.into_iter().take(16).collect()
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

    #[test]
    fn validate_dataflow_diagnostics_caches_by_digest_and_reports_structured_violations() {
        let files = vec![
            json!({"path": "specs/001-cache/spec.md", "available": true, "text": "# Spec\n- FR-001: Auth\n"}),
            json!({"path": "specs/001-cache/plan.md", "available": true, "text": "# Plan\nCovers FR-001\nRef FR-404\n"}),
            json!({"path": "specs/001-cache/tasks.md", "available": true, "text": "- [ ] T001 Build\n- [ ] T001 Dup\n"}),
            json!({"path": "specs/001-cache/progress.md", "available": true, "text": "- [x] T001 Done early\n"}),
        ];

        let before = SDD_CACHE_EVALUATIONS.load(std::sync::atomic::Ordering::Relaxed);
        let first = validate_dataflow_diagnostics(&files, "implement", 3);
        let after_first = SDD_CACHE_EVALUATIONS.load(std::sync::atomic::Ordering::Relaxed);
        assert!(after_first > before);
        let second = validate_dataflow_diagnostics(&files, "implement", 3);
        let after_second = SDD_CACHE_EVALUATIONS.load(std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            after_second, after_first,
            "identical SDD artifacts must hit digest cache"
        );
        assert_eq!(first, second);

        assert!(first.iter().any(|v| {
            v.code == "SDD_DANGLING_REQUIREMENT"
                && v.path == "specs/001-cache/plan.md"
                && v.line == Some(3)
                && v.message.contains("FR-404")
                && !v.hint.is_empty()
        }));
        assert!(first.iter().any(|v| {
            v.code == "SDD_DUPLICATE_TASK"
                && v.path == "specs/001-cache/tasks.md"
                && v.line == Some(2)
        }));
        assert!(first.iter().any(|v| {
            v.code == "SDD_PROGRESS_STATUS_MISMATCH"
                && v.path == "specs/001-cache/progress.md"
                && v.line == Some(1)
        }));
    }

    #[test]
    fn sdd_validates_checked_file_and_symbol_claims_against_workspace_and_graph_ir() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        let state = dir.path().join("state");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(
            root.join("src/auth.rs"),
            "pub struct TokenService;\nimpl TokenService {\n    pub fn issue(&self) {}\n}\n",
        )
        .unwrap();

        let ir = crate::graph_ir::GraphIrStore::open(&state, "sdd-ir-proj").unwrap();
        ir.sync_workspace(&root).unwrap();
        drop(ir);

        let valid_files = vec![
            json!({"path": "specs/001-ir/spec.md", "available": true, "text": "# Spec\n- FR-001: Auth tokens\n"}),
            json!({"path": "specs/001-ir/plan.md", "available": true, "text": "# Plan\nCovers FR-001\n"}),
            json!({"path": "specs/001-ir/tasks.md", "available": true, "text": "- [x] T001 Implement `TokenService::issue` in `src/auth.rs`\n"}),
            json!({"path": "specs/001-ir/progress.md", "available": true, "text": "- [x] T001 Verified `src/auth.rs` (`TokenService::issue`)\n"}),
        ];
        let valid_violations = validate_dataflow_diagnostics_in(
            Some(&root),
            Some((&state, "sdd-ir-proj")),
            &valid_files,
            "review",
            3,
        );
        assert!(
            valid_violations.is_empty(),
            "expected valid [x] claims to pass, got: {valid_violations:?}"
        );

        // Claiming a non-existent file and non-existent symbol on [x] rows emits structured Violations.
        let invalid_files = vec![
            json!({"path": "specs/001-ir/spec.md", "available": true, "text": "# Spec\n- FR-001: Auth tokens\n"}),
            json!({"path": "specs/001-ir/plan.md", "available": true, "text": "# Plan\nCovers FR-001\n"}),
            json!({"path": "specs/001-ir/tasks.md", "available": true, "text": "- [x] T001 Implemented `src/missing.rs` and `TokenService::bogus` in `src/auth.rs`\n"}),
            json!({"path": "specs/001-ir/progress.md", "available": true, "text": "- [x] T001 Completed\n"}),
        ];
        let violations = validate_dataflow_diagnostics_in(
            Some(&root),
            Some((&state, "sdd-ir-proj")),
            &invalid_files,
            "review",
            3,
        );
        assert!(violations.iter().any(|v| {
            v.code == "SDD_CLAIMED_FILE_MISSING"
                && v.path == "specs/001-ir/tasks.md"
                && v.line == Some(1)
                && v.message.contains("src/missing.rs")
        }));
        assert!(violations.iter().any(|v| {
            v.code == "SDD_CLAIMED_SYMBOL_MISSING"
                && v.path == "specs/001-ir/tasks.md"
                && v.line == Some(1)
                && v.message.contains("TokenService::bogus")
        }));
    }
}
