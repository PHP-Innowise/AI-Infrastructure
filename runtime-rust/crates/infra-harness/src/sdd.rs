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
fn needed_for_phase(phase: &str) -> usize {
    match phase {
        "plan" => 1,
        "tasks" => 2,
        "implement" | "review" => 3,
        _ => 0,
    }
}

pub fn require_documents(root: &Path, settings: &Value) -> Result<()> {
    require_documents_in(root, settings, None)
}

/// Validates required SDD phase artifacts and cross-references, optionally verifying
/// `[x]` workspace file/symbol claims against `(ir_state, project_id)` via `GraphIrStore`.
pub(crate) fn require_documents_in(
    root: &Path,
    settings: &Value,
    ir_context: Option<(&Path, &str)>,
) -> Result<()> {
    let phase = settings["phase"].as_str().unwrap_or("");
    let needed = needed_for_phase(phase);
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

#[derive(Clone)]
struct CachedSddEval {
    root_key: Option<String>,
    project_key: Option<String>,
    claimed_files: BTreeSet<String>,
    has_symbol_only_claims: bool,
    violations: Vec<Violation>,
}

fn dataflow_cache() -> &'static Mutex<BTreeMap<String, CachedSddEval>> {
    static CACHE: OnceLock<Mutex<BTreeMap<String, CachedSddEval>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
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

    let cache = dataflow_cache();
    let digest = dataflow_cache_digest(files, phase, needed, &context_stamp);
    if let Ok(guard) = cache.lock()
        && let Some(hit) = guard.get(&digest)
    {
        return hit.violations.clone();
    }

    #[cfg(test)]
    SDD_CACHE_EVALUATIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let mut result = compute_dataflow_diagnostics(files, phase);
    if (root.is_some() || ir_store.is_some()) && !claims.is_empty() {
        verify_checked_claims(root, ir_store.as_deref(), &claims, &mut result);
        result.truncate(16);
    }
    let mut claimed_files = BTreeSet::new();
    let mut has_symbol_only_claims = false;
    for claim in &claims {
        claimed_files.extend(claim.files.iter().cloned());
        if claim.files.is_empty() && !claim.symbols.is_empty() {
            has_symbol_only_claims = true;
        }
    }
    if let Ok(mut guard) = cache.lock() {
        if guard.len() >= 128 {
            guard.clear();
        }
        guard.insert(
            digest,
            CachedSddEval {
                root_key: root.map(|r| r.to_string_lossy().into_owned()),
                project_key: ir_context.map(|(_, p)| p.to_owned()),
                claimed_files,
                has_symbol_only_claims,
                violations: result.clone(),
            },
        );
    }
    result
}

/// Invalidates cached SDD claim diagnostics and emits `"sdd_staleness"` SSE notifications for
/// active SDD sessions in `project_id` whose checked claims overlap a Graph-IR invalidation delta.
pub(crate) fn on_ir_paths_updated(
    store: &crate::store::SessionStore,
    project_id: &str,
    root: &Path,
    delta: &crate::graph_ir::IrUpdateDelta,
) -> usize {
    let touched: BTreeSet<&str> = delta
        .changed_paths
        .iter()
        .chain(delta.edited_files.iter())
        .chain(delta.removed_files.iter())
        .chain(delta.incoming_caller_files.iter())
        .map(String::as_str)
        .collect();
    if touched.is_empty() && !delta.manifest_changed {
        return 0;
    }
    let root_str = root.to_string_lossy();
    if let Ok(mut guard) = dataflow_cache().lock() {
        guard.retain(|_, entry| {
            let same_scope = entry.project_key.as_deref() == Some(project_id)
                || entry.root_key.as_deref() == Some(root_str.as_ref());
            if !same_scope {
                return true;
            }
            !(delta.manifest_changed
                || entry.has_symbol_only_claims
                || entry
                    .claimed_files
                    .iter()
                    .any(|f| touched.contains(f.as_str())))
        });
    }

    let Ok(db) = store.connection() else {
        return 0;
    };
    let rows: Vec<(String, String, String)> = {
        let Ok(mut stmt) = db.prepare(
            "SELECT id, project_path, sdd FROM sessions
             WHERE project_id = ?1
               AND sdd IS NOT NULL
               AND sdd != ''
               AND status NOT IN ('completed', 'cancelled')
             ORDER BY updated_at DESC LIMIT 32",
        ) else {
            return 0;
        };
        let Ok(mapped) = stmt.query_map([project_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        }) else {
            return 0;
        };
        mapped.flatten().collect()
    };
    drop(db);

    let mut emitted = 0usize;
    for (sid, project_path, sdd_raw) in rows {
        let Ok(sdd_val) = serde_json::from_str::<Value>(&sdd_raw) else {
            continue;
        };
        let feature = sdd_val["feature"].as_str().unwrap_or("");
        let phase = sdd_val["phase"].as_str().unwrap_or("");
        let needed = needed_for_phase(phase);
        if feature.is_empty() || needed == 0 {
            continue;
        }
        let session_root = if !project_path.trim().is_empty() && Path::new(&project_path).is_dir() {
            Path::new(&project_path)
        } else {
            root
        };
        let Ok(arts) = artifacts(session_root, &sdd_val) else {
            continue;
        };
        let claims = extract_checked_claims(&arts);
        if claims.is_empty() {
            continue;
        }
        let overlaps = delta.manifest_changed
            || claims.iter().any(|claim| {
                (claim.files.is_empty() && !claim.symbols.is_empty())
                    || claim.files.iter().any(|f| touched.contains(f.as_str()))
            });
        if !overlaps {
            continue;
        }
        let violations = validate_dataflow_diagnostics_in(
            Some(session_root),
            Some((&store.state, project_id)),
            &arts,
            phase,
            needed,
        );
        store.publish_event(
            "sdd_staleness",
            Some(sid.clone()),
            Some(project_id.to_owned()),
            json!({
                "project_id": project_id,
                "session_id": sid,
                "feature": feature,
                "phase": phase,
                "ir_revision": delta.ir_revision,
                "stale": !violations.is_empty(),
                "violations": violations,
                "changed_paths": delta.changed_paths,
            }),
        );
        emitted += 1;
    }
    emitted
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
        ".rs", ".php", ".ts", ".tsx", ".mts", ".cjs", ".mjs", ".js", ".jsx", ".py", ".go", ".java",
        ".kt", ".kts", ".cs", ".rb", ".swift", ".scala", ".sc", ".phtml", ".inc", ".md", ".json",
        ".toml", ".yaml", ".yml", ".sql", ".sh",
    ];
    const EXACT_BASENAMES: &[&str] = &[
        "Dockerfile",
        "Containerfile",
        "Makefile",
        "Gemfile",
        "Rakefile",
        "Procfile",
    ];
    let trimmed = token.trim().trim_start_matches("./");
    if trimmed.is_empty()
        || trimmed.starts_with('/')
        || trimmed.ends_with('/')
        || trimmed.contains("://")
        || trimmed.contains('\\')
        || trimmed.contains("..")
        || trimmed.split('/').any(str::is_empty)
        || trimmed
            .chars()
            .any(|c| c.is_whitespace() || "*?[]{}<>\"'$()".contains(c))
    {
        return false;
    }
    let basename = trimmed.rsplit('/').next().unwrap_or(trimmed);
    EXACT_BASENAMES.contains(&basename)
        || EXTS
            .iter()
            .any(|ext| trimmed.ends_with(ext) && basename.len() > ext.len())
}

fn is_valid_claim_ident_segment(part: &str) -> bool {
    let mut bytes = part.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn is_ignored_bare_claim_word(token: &str) -> bool {
    if token.len() >= 2
        && token.starts_with(['T', 't'])
        && token[1..].bytes().all(|b| b.is_ascii_digit())
    {
        return true;
    }
    if matches!(
        token,
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
    ) {
        return true;
    }
    matches!(
        token.to_ascii_lowercase().as_str(),
        "true"
            | "false"
            | "null"
            | "none"
            | "nil"
            | "undefined"
            | "void"
            | "nan"
            | "ok"
            | "yes"
            | "no"
            | "todo"
            | "wip"
            | "fixme"
            | "id"
            | "any"
            | "unknown"
            | "never"
    )
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
    if trimmed.contains("::") {
        return trimmed.split("::").all(|part| {
            let p = part.trim_start_matches('\\');
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'\\'))
        });
    }
    // Dot-qualified member reference (`Owner.member`, `pkg.Type.method`) after ruling out filenames.
    if trimmed.contains('.') {
        let mut parts = trimmed.split('.');
        let first = parts.next();
        let second = parts.next();
        return first.is_some_and(is_valid_claim_ident_segment)
            && second.is_some_and(is_valid_claim_ident_segment)
            && parts.all(is_valid_claim_ident_segment);
    }
    trimmed.len() >= 2
        && is_valid_claim_ident_segment(trimmed)
        && !is_ignored_bare_claim_word(trimmed)
}

fn resolve_claimed_file_on_disk(
    root: &Path,
    doc_path: &str,
    rel: &str,
) -> Option<(String, std::path::PathBuf)> {
    let is_regular_non_symlink = |candidate: &Path| {
        infra_core::fs::admit_descendant(root, candidate).is_ok()
            && std::fs::symlink_metadata(candidate)
                .is_ok_and(|m| !m.file_type().is_symlink() && m.is_file())
    };
    let direct = root.join(rel);
    if is_regular_non_symlink(&direct) {
        return Some((rel.to_owned(), direct));
    }
    if !rel.contains('/')
        && let Some(parent) = Path::new(doc_path)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
    {
        let sibling_rel = format!("{}/{rel}", parent.to_string_lossy().replace('\\', "/"));
        let sibling = root.join(&sibling_rel);
        if is_regular_non_symlink(&sibling) {
            return Some((sibling_rel, sibling));
        }
    }
    None
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
        for rel in &line.files {
            unique_files.insert((line.doc_path.clone(), rel.clone()));
        }
    }
    let file_stamps: Vec<String> = unique_files
        .into_iter()
        .map(|(doc_path, rel)| {
            let meta = root
                .and_then(|r| resolve_claimed_file_on_disk(r, &doc_path, &rel))
                .and_then(|(_, p)| std::fs::symlink_metadata(&p).ok())
                .filter(|m| !m.file_type().is_symlink() && m.is_file());
            match meta {
                Some(m) => {
                    let mtime_ns = m
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0, |d| d.as_nanos());
                    format!("{doc_path}::{rel}:1:{}:{mtime_ns}", m.len())
                }
                None => format!("{doc_path}::{rel}:0"),
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

fn verify_checked_claims(
    root: Option<&Path>,
    ir: Option<&crate::graph_ir::GraphIrStore>,
    claims: &[CheckedClaimLine],
    violations: &mut Vec<Violation>,
) {
    let ir_has_symbols = ir
        .and_then(|store| store.summary_counts().ok().flatten())
        .is_some_and(|sum| sum.symbols > 0);
    let mut ir_file_cache: BTreeMap<String, bool> = BTreeMap::new();
    let mut check_file_in_ir = |rel: &str| -> bool {
        if let Some(&cached) = ir_file_cache.get(rel) {
            return cached;
        }
        let exists = ir.is_some_and(|store| {
            store
                .verify_path_or_symbol(Some(rel), None)
                .is_ok_and(|res| res.path_exists)
        });
        ir_file_cache.insert(rel.to_owned(), exists);
        exists
    };

    for claim in claims {
        let mut verified_files: Vec<(String, bool)> = Vec::new();
        for rel in &claim.files {
            let resolved_disk =
                root.and_then(|r| resolve_claimed_file_on_disk(r, &claim.doc_path, rel));
            let on_disk = resolved_disk.is_some();
            let resolved_rel = resolved_disk
                .as_ref()
                .map(|(norm_rel, _)| norm_rel.as_str())
                .unwrap_or(rel.as_str());
            let in_ir =
                check_file_in_ir(rel) || (resolved_rel != rel && check_file_in_ir(resolved_rel));
            if on_disk || in_ir {
                let effective_rel = if check_file_in_ir(rel) || resolved_disk.is_none() {
                    rel.clone()
                } else {
                    resolved_rel.to_owned()
                };
                verified_files.push((effective_rel, in_ir));
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
        let indexed_files: Vec<&str> = verified_files
            .iter()
            .filter_map(|(rel, in_ir)| (*in_ir).then_some(rel.as_str()))
            .collect();
        let unindexed_disk_files: Vec<&str> = verified_files
            .iter()
            .filter_map(|(rel, in_ir)| (!*in_ir).then_some(rel.as_str()))
            .collect();

        for sym in &claim.symbols {
            if let Some(store) = ir
                && ir_has_symbols
            {
                let ir_hit = if !indexed_files.is_empty() {
                    indexed_files.iter().any(|rel| {
                        store
                            .verify_path_or_symbol(Some(rel), Some(sym))
                            .is_ok_and(|res| res.symbol_exists)
                    })
                } else if verified_files.is_empty() {
                    store
                        .verify_path_or_symbol(None, Some(sym))
                        .is_ok_and(|res| res.symbol_exists)
                } else {
                    false
                };
                if ir_hit {
                    continue;
                }
                if !indexed_files.is_empty() || verified_files.is_empty() {
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
            }
            // No Graph-IR DB or file not yet indexed: fall back to checking verified claimed files on disk if present.
            let disk_targets: Vec<&str> = if ir.is_some() && ir_has_symbols {
                unindexed_disk_files.clone()
            } else {
                verified_files.iter().map(|(rel, _)| rel.as_str()).collect()
            };
            if let Some(r) = root
                && !disk_targets.is_empty()
            {
                let (owner, member) = sym
                    .rsplit_once("::")
                    .or_else(|| sym.rsplit_once('.'))
                    .map(|(o, m)| (o.rsplit(['\\', '.']).next().unwrap_or(o), m))
                    .unwrap_or(("", sym.as_str()));
                let found_on_disk = disk_targets.iter().any(|rel| {
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

    #[test]
    fn require_documents_in_enforces_graph_ir_and_validate_rejects_internal_keys() {
        assert!(
            validate(
                &json!({"feature": "001-ir", "phase": "review", "ir_state": "/tmp/x", "project_id": "p"}),
                "sdd",
                None
            )
            .is_err(),
            "external SDD validation must reject ir_state/project_id keys"
        );

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        let state = dir.path().join("state");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("specs/001-ir")).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(
            root.join("src/auth.rs"),
            "pub struct TokenService;\nimpl TokenService {\n    pub fn issue(&self) {}\n}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("specs/001-ir/spec.md"),
            "# Spec\n- FR-001: Auth tokens\n",
        )
        .unwrap();
        std::fs::write(
            root.join("specs/001-ir/plan.md"),
            "# Plan\nCovers FR-001.\n",
        )
        .unwrap();
        std::fs::write(
            root.join("specs/001-ir/tasks.md"),
            "- [x] T001 Implement `TokenService::issue` in `src/auth.rs`\n",
        )
        .unwrap();
        std::fs::write(root.join("specs/001-ir/progress.md"), "- [x] T001 Done\n").unwrap();

        let ir = crate::graph_ir::GraphIrStore::open(&state, "sdd-doc-ir").unwrap();
        ir.sync_workspace(&root).unwrap();
        drop(ir);

        let settings = json!({"feature": "001-ir", "phase": "review"});
        require_documents_in(&root, &settings, Some((&state, "sdd-doc-ir"))).unwrap();

        std::fs::write(
            root.join("specs/001-ir/tasks.md"),
            "- [x] T001 Implement `TokenService::missing_method` in `src/auth.rs`\n",
        )
        .unwrap();
        let err = require_documents_in(&root, &settings, Some((&state, "sdd-doc-ir"))).unwrap_err();
        assert!(
            err.message.contains("TokenService::missing_method"),
            "{}",
            err.message
        );
    }

    #[test]
    fn on_ir_paths_updated_invalidates_sdd_cache_and_emits_staleness_sse() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        let state = dir.path().join("state");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("specs/002-bus")).unwrap();
        std::fs::write(
            root.join("src/auth.rs"),
            "pub struct TokenService;\nimpl TokenService {\n    pub fn issue(&self) {}\n}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("specs/002-bus/spec.md"),
            "# Spec\n- FR-001: Auth tokens\n",
        )
        .unwrap();
        std::fs::write(
            root.join("specs/002-bus/plan.md"),
            "# Plan\nCovers FR-001.\n",
        )
        .unwrap();
        std::fs::write(
            root.join("specs/002-bus/tasks.md"),
            "- [x] T001 Implement `TokenService::issue` in `src/auth.rs`\n",
        )
        .unwrap();
        std::fs::write(root.join("specs/002-bus/progress.md"), "- [x] T001 Done\n").unwrap();

        let store = crate::store::SessionStore::open(&state).unwrap();
        let proj = store.register(&root).unwrap();
        let pid = proj["id"].as_str().unwrap().to_owned();
        let ir = crate::graph_ir::GraphIrStore::open(&state, &pid).unwrap();
        ir.sync_workspace(&root).unwrap();

        let sid = store::id();
        store
            .connection()
            .unwrap()
            .execute(
                "INSERT INTO sessions(id,title,project_id,project_path,provider,mode,workflow,project_context,status,created_at,updated_at,sdd)
                 VALUES(?1,'SDD Bus',?2,?3,'claude','plan','sdd',0,'running','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z',?4)",
                rusqlite::params![
                    sid,
                    pid,
                    root.to_string_lossy().as_ref(),
                    json!({"feature": "002-bus", "phase": "review"}).to_string(),
                ],
            )
            .unwrap();

        let crate::store::ReplayStart::Frames { mut rx, .. } = store.stream().subscribe(None)
        else {
            panic!("expected frames");
        };

        std::fs::write(root.join("src/auth.rs"), "pub struct TokenService;\n").unwrap();
        let mut delta = ir
            .update_paths_with_delta(&root, &["src/auth.rs".to_owned()])
            .unwrap();
        delta.ir_revision = 42;
        let emitted = on_ir_paths_updated(&store, &pid, &root, &delta);
        assert_eq!(emitted, 1);

        let mut saw_stale = false;
        while let Ok(frame) = rx.try_recv() {
            if frame.event == "sdd_staleness" && frame.project_id.as_deref() == Some(pid.as_str()) {
                assert_eq!(frame.data["stale"], true);
                assert_eq!(frame.data["feature"], "002-bus");
                assert_eq!(frame.data["ir_revision"], 42);
                assert!(
                    frame.data["violations"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|v| v["code"] == "SDD_CLAIMED_SYMBOL_MISSING"),
                    "{}",
                    frame.data
                );
                saw_stale = true;
            }
        }
        assert!(saw_stale, "expected sdd_staleness SSE event");
    }

    #[test]
    fn claim_token_parsers_support_root_files_multi_lang_exts_dot_and_bare_symbols() {
        for file_tok in [
            "Cargo.toml",
            "README.md",
            "Dockerfile",
            "Makefile",
            "src/App.java",
            "src/Main.kt",
            "Order.cs",
            "lib/worker.rb",
            "Sources/App.swift",
            "src/mod.mts",
            "config.cjs",
            "spec.md",
        ] {
            assert!(
                is_claim_file_token(file_tok),
                "expected file token: {file_tok}"
            );
            assert!(
                !is_claim_symbol_token(file_tok),
                "file token must not classify as symbol: {file_tok}"
            );
        }
        assert!(!is_claim_file_token(".rs"));
        assert!(!is_claim_file_token("src/.rs"));
        assert!(!is_claim_file_token("../escape.rs"));

        for sym_tok in [
            "TokenService::issue",
            r"App\Auth\Guard::check",
            "UserService.create",
            "pkg.AuthService.verify",
            "TokenService",
            "create_token",
            "handler",
        ] {
            assert!(
                is_claim_symbol_token(sym_tok),
                "expected symbol token: {sym_tok}"
            );
        }

        for non_sym in [
            "T001", "t42", "FR-001", "GET", "POST", "true", "null", "id", ".rs",
        ] {
            assert!(
                !is_claim_symbol_token(non_sym),
                "unexpected symbol match: {non_sym}"
            );
        }
    }

    #[test]
    fn verify_checked_claims_checks_root_filenames_new_exts_dot_and_bare_symbols_with_ir() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        let state = dir.path().join("state");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(root.join("Dockerfile"), "FROM rust:1.80\n").unwrap();
        std::fs::write(
            root.join("app.mts"),
            "export class UserService {\n  create() { return true; }\n}\nexport function create_token() { return 'ok'; }\n",
        )
        .unwrap();
        std::fs::write(root.join("src/App.java"), "class App { void run() {} }\n").unwrap();

        let ir = crate::graph_ir::GraphIrStore::open(&state, "sdd-claims-ext").unwrap();
        ir.sync_workspace(&root).unwrap();
        drop(ir);

        let files = vec![
            json!({"path": "specs/001-ext/spec.md", "available": true, "text": "# Spec\n- FR-001: Multi-lang\n"}),
            json!({"path": "specs/001-ext/plan.md", "available": true, "text": "# Plan\nCovers FR-001\n"}),
            json!({
                "path": "specs/001-ext/tasks.md",
                "available": true,
                "text": "- [x] T001 Containerized with `Dockerfile`, `app.mts`, `src/App.java`, `UserService.create`, and `create_token` (`T001`, `true`, `GET`)\n"
            }),
            json!({"path": "specs/001-ext/progress.md", "available": true, "text": "- [x] T001 Done\n"}),
        ];
        let valid = validate_dataflow_diagnostics_in(
            Some(&root),
            Some((&state, "sdd-claims-ext")),
            &files,
            "review",
            3,
        );
        assert!(
            valid.is_empty(),
            "expected valid multi-lang claims: {valid:?}"
        );

        let bogus_files = vec![
            json!({"path": "specs/001-ext/spec.md", "available": true, "text": "# Spec\n- FR-001: Multi-lang\n"}),
            json!({"path": "specs/001-ext/plan.md", "available": true, "text": "# Plan\nCovers FR-001\n"}),
            json!({
                "path": "specs/001-ext/tasks.md",
                "available": true,
                "text": "- [x] T001 Missing `Order.cs` and `UserService.nonexistent` and `GhostHelper` in `app.mts`\n"
            }),
            json!({"path": "specs/001-ext/progress.md", "available": true, "text": "- [x] T001 Done\n"}),
        ];
        let violations = validate_dataflow_diagnostics_in(
            Some(&root),
            Some((&state, "sdd-claims-ext")),
            &bogus_files,
            "review",
            3,
        );
        assert!(
            violations
                .iter()
                .any(|v| v.code == "SDD_CLAIMED_FILE_MISSING" && v.message.contains("Order.cs"))
        );
        assert!(violations.iter().any(|v| {
            v.code == "SDD_CLAIMED_SYMBOL_MISSING" && v.message.contains("UserService.nonexistent")
        }));
        assert!(violations.iter().any(|v| {
            v.code == "SDD_CLAIMED_SYMBOL_MISSING" && v.message.contains("GhostHelper")
        }));
    }

    #[test]
    fn on_ir_paths_updated_ignores_completed_and_cancelled_sessions_without_crowding_out_active() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        let state = dir.path().join("state");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("specs/003-active")).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(root.join("src/auth.rs"), "pub struct TokenService;\n").unwrap();
        std::fs::write(
            root.join("specs/003-active/spec.md"),
            "# Spec\n- FR-001: Active\n",
        )
        .unwrap();
        std::fs::write(root.join("specs/003-active/plan.md"), "# Plan\nFR-001\n").unwrap();
        std::fs::write(
            root.join("specs/003-active/tasks.md"),
            "- [x] T001 Claim `TokenService` in `src/auth.rs`\n",
        )
        .unwrap();
        std::fs::write(
            root.join("specs/003-active/progress.md"),
            "- [x] T001 Done\n",
        )
        .unwrap();

        let store = crate::store::SessionStore::open(&state).unwrap();
        let proj = store.register(&root).unwrap();
        let pid = proj["id"].as_str().unwrap().to_owned();
        let ir = crate::graph_ir::GraphIrStore::open(&state, &pid).unwrap();
        ir.sync_workspace(&root).unwrap();

        let conn = store.connection().unwrap();
        // Insert an older active session plus 40 newer completed/cancelled sessions so LIMIT 32
        // only preserves the active one if completed/cancelled rows are filtered out in SQL.
        conn.execute(
            "INSERT INTO sessions(id,title,project_id,project_path,provider,mode,workflow,project_context,status,created_at,updated_at,sdd)
             VALUES(?1,'Active SDD',?2,?3,'claude','plan','sdd',0,'waiting','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z',?4)",
            rusqlite::params![
                store::id(),
                pid,
                root.to_string_lossy().as_ref(),
                json!({"feature": "003-active", "phase": "review"}).to_string(),
            ],
        )
        .unwrap();
        for idx in 0..40 {
            let status = if idx % 2 == 0 {
                "completed"
            } else {
                "cancelled"
            };
            conn.execute(
                "INSERT INTO sessions(id,title,project_id,project_path,provider,mode,workflow,project_context,status,created_at,updated_at,sdd)
                 VALUES(?1,'Archived SDD',?2,?3,'claude','plan','sdd',0,?4,'2026-02-01T00:00:00Z',?5,?6)",
                rusqlite::params![
                    store::id(),
                    pid,
                    root.to_string_lossy().as_ref(),
                    status,
                    format!("2026-02-01T00:{:02}:00Z", idx % 60),
                    json!({"feature": "003-active", "phase": "review"}).to_string(),
                ],
            )
            .unwrap();
        }
        drop(conn);

        std::fs::write(root.join("src/auth.rs"), "pub struct ReplacedService;\n").unwrap();
        let delta = ir
            .update_paths_with_delta(&root, &["src/auth.rs".to_owned()])
            .unwrap();
        assert_eq!(
            on_ir_paths_updated(&store, &pid, &root, &delta),
            1,
            "only active/waiting session must be evaluated and emit"
        );
    }
}
