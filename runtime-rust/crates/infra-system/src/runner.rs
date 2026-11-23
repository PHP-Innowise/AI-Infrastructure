use crate::{
    boundary::{BoundaryScope, SystemBoundaryApprovals, check_step_boundary_writes_with_extra},
    catalog::{System, git_head},
    common::*,
    execution::{
        self, Brain, access_scope, changed_path, dispatch_access, dispatch_receipt, reports,
    },
    native,
    system_graph::{
        CodeGraphBridge, DispatchEvidence, DispatchGraphContext, EvidenceFileChange,
        git_changes_since, git_worktree_changes,
    },
};
use infra_core::{ErrorKind, Result, RuntimeError};
use infra_host::permission::{Approvals, PermissionMode};
use serde_json::{Value, json as object};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

const SYSTEM_CACHE_RACY_NS: i128 = 2_000_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
struct WatchedFileStamp {
    exists: bool,
    is_file: bool,
    len: u64,
    mtime_ns: i128,
    racy_sha256: Option<String>,
}

impl WatchedFileStamp {
    fn capture(path: &Path, now_ns: i128) -> Self {
        let Ok(meta) = std::fs::symlink_metadata(path) else {
            return Self {
                exists: false,
                is_file: false,
                len: 0,
                mtime_ns: 0,
                racy_sha256: None,
            };
        };
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Self {
                exists: true,
                is_file: false,
                len: 0,
                mtime_ns: 0,
                racy_sha256: None,
            };
        }
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|d| i128::try_from(d.as_nanos()).ok())
            .unwrap_or(0);
        let is_racy = mtime_ns <= 0 || now_ns <= 0 || mtime_ns > now_ns - SYSTEM_CACHE_RACY_NS;
        let racy_sha256 = if is_racy {
            std::fs::read(path).ok().map(|bytes| digest(&bytes))
        } else {
            None
        };
        Self {
            exists: true,
            is_file: true,
            len: meta.len(),
            mtime_ns,
            racy_sha256,
        }
    }

    fn matches(&self, path: &Path, now_ns: i128) -> bool {
        let Ok(meta) = std::fs::symlink_metadata(path) else {
            return !self.exists;
        };
        if !self.exists {
            return false;
        }
        if meta.file_type().is_symlink() || !meta.is_file() {
            return !self.is_file;
        }
        if !self.is_file {
            return false;
        }
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|d| i128::try_from(d.as_nanos()).ok())
            .unwrap_or(0);
        if meta.len() != self.len || mtime_ns != self.mtime_ns {
            return false;
        }
        let is_racy = mtime_ns <= 0 || now_ns <= 0 || mtime_ns > now_ns - SYSTEM_CACHE_RACY_NS;
        if is_racy || self.racy_sha256.is_some() {
            let Some(expected) = &self.racy_sha256 else {
                return false;
            };
            return std::fs::read(path)
                .ok()
                .is_some_and(|bytes| digest(&bytes) == *expected);
        }
        true
    }
}

fn resolve_git_dir(root: &Path) -> Option<PathBuf> {
    let top = root
        .ancestors()
        .find(|folder| std::fs::symlink_metadata(folder.join(".git")).is_ok())?;
    let dot_git = top.join(".git");
    let meta = std::fs::symlink_metadata(&dot_git).ok()?;
    if meta.is_dir() {
        return Some(dot_git);
    }
    if meta.is_file()
        && let Ok(text) = std::fs::read_to_string(&dot_git)
        && let Some(raw) = text.trim().strip_prefix("gitdir:")
    {
        let p = Path::new(raw.trim());
        return Some(if p.is_absolute() {
            p.to_path_buf()
        } else {
            top.join(p)
        });
    }
    None
}

fn collect_watched_system_paths(system: &System) -> Vec<PathBuf> {
    let mut paths = BTreeSet::new();
    paths.insert(system.config.clone());
    paths.insert(system.root.join("project-brain/config/runtime.json"));
    let mut git_dirs = BTreeSet::new();
    if let Some(gd) = resolve_git_dir(&system.root) {
        git_dirs.insert(gd);
    }
    for svc in system.services.values() {
        if !svc.manifest_path.is_empty() {
            paths.insert(svc.root.join(&svc.manifest_path));
        }
        if let Some(gd) = resolve_git_dir(&svc.root) {
            git_dirs.insert(gd);
        }
    }
    for gd in git_dirs {
        let head = gd.join("HEAD");
        if let Ok(head_text) = std::fs::read_to_string(&head)
            && let Some(ref_rel) = head_text.trim().strip_prefix("ref:")
        {
            paths.insert(gd.join(ref_rel.trim()));
        }
        paths.insert(head);
        paths.insert(gd.join("packed-refs"));
    }
    paths.into_iter().collect()
}

struct CachedSystem {
    system: System,
    watched: Vec<(PathBuf, WatchedFileStamp)>,
    reload_count: usize,
}

impl CachedSystem {
    fn from_system(system: &System) -> Self {
        let now_ns = current_unix_ns();
        let watched = collect_watched_system_paths(system)
            .into_iter()
            .map(|p| {
                let stamp = WatchedFileStamp::capture(&p, now_ns);
                (p, stamp)
            })
            .collect();
        Self {
            system: system.clone(),
            watched,
            reload_count: 0,
        }
    }

    fn is_drifted(&self) -> bool {
        let now_ns = current_unix_ns();
        self.watched
            .iter()
            .any(|(path, stamp)| !stamp.matches(path, now_ns))
    }

    fn get_or_reload(&mut self, allow_roots: &[PathBuf]) -> Result<&mut System> {
        if self.is_drifted() {
            let reloaded = System::new(&self.system.config, allow_roots)?;
            let now_ns = current_unix_ns();
            self.watched = collect_watched_system_paths(&reloaded)
                .into_iter()
                .map(|p| {
                    let stamp = WatchedFileStamp::capture(&p, now_ns);
                    (p, stamp)
                })
                .collect();
            self.system = reloaded;
            self.reload_count += 1;
        }
        Ok(&mut self.system)
    }
}

fn current_unix_ns() -> i128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i128::try_from(d.as_nanos()).ok())
        .unwrap_or(0)
}

struct PrelaunchHealthDecision {
    active_model: Option<String>,
    max_attempts: u64,
}

fn evaluate_prelaunch_health(
    health_db_path: &Path,
    provider: &str,
    step: &Value,
    state: &mut Value,
    event: &mut (dyn FnMut(Value) -> Result<()> + Send),
) -> PrelaunchHealthDecision {
    let requested_model = state
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let pre_health = execution::summarize_system_health(
        health_db_path,
        "system",
        provider,
        requested_model.as_deref(),
    );
    let healthier_model = execution::select_healthier_model(
        health_db_path,
        "system",
        provider,
        requested_model.as_deref(),
    );
    let active_model = healthier_model.clone().or_else(|| requested_model.clone());
    let mut pre_warnings = pre_health.warnings.clone();
    if let Some(alt) = &healthier_model {
        pre_warnings.push(format!(
            "Selected healthier model `{alt}` over degraded `{}` for system/{provider}",
            requested_model.as_deref().unwrap_or("default")
        ));
    }
    merge_health_warnings(state, &pre_warnings);
    let max_attempts = pre_health.recommended_max_attempts(100);
    if pre_health.is_degraded() || healthier_model.is_some() {
        notify(
            event,
            object!({
                "type": "system_health",
                "phase": step["id"],
                "provider": provider,
                "requested_model": requested_model,
                "selected_model": &active_model,
                "recommended_max_attempts": max_attempts,
                "recommended_concurrency": pre_health.recommended_concurrency(1),
                "warnings": pre_warnings,
            }),
        );
    }
    PrelaunchHealthDecision {
        active_model,
        max_attempts,
    }
}

fn should_require_verify_llm_review(
    verify_plan_step: Option<&Value>,
    state: &Value,
    prior: &[Value],
) -> bool {
    let deterministic_only = verify_plan_step.is_some_and(|s| s["deterministic_only"] == true);
    let prior_step_mutated = prior.iter().any(|rep| {
        rep.get("changed_files")
            .and_then(Value::as_array)
            .is_some_and(|arr| !arr.is_empty())
    });
    let has_explicit_mutating_step = state["steps"].as_array().is_some_and(|all_steps| {
        all_steps.iter().any(|s| {
            s["kind"].as_str() == Some("mutate")
                || s["mutating"] == true
                || s["mode"].as_str() == Some("edit")
        })
    });
    let mode_forbids_zero_token = state["mode"].as_str() != Some("read-only")
        || has_explicit_mutating_step
        || prior_step_mutated;
    verify_plan_step.is_some_and(|s| s["require_llm_review"] == true)
        || (!deterministic_only && mode_forbids_zero_token)
}

fn merge_health_warnings(state: &mut Value, warnings: &[String]) {
    if warnings.is_empty() {
        return;
    }
    let mut existing: Vec<String> = state
        .get("health_warnings")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    for w in warnings {
        if !existing.contains(w) {
            existing.push(w.clone());
        }
    }
    state["health_warnings"] = object!(existing);
}

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Optional code-graph bridge, launch identity, and approval hooks threaded through a system run.
#[derive(Clone, Default)]
pub struct RunnerGraphHooks {
    pub bridge: Option<Arc<dyn CodeGraphBridge>>,
    pub system_run_id: Option<String>,
    pub launch_id: Option<String>,
    pub approvals: Option<Arc<dyn Approvals>>,
}

/// Opt-in dispatch context policy. The default runner has no extension.
/// Prompts are finalized before native input custody; completed receipts remain native.
pub trait DispatchContext: Send {
    fn prompt(
        &mut self,
        state: &Value,
        plan: &Value,
        step: &Value,
        prior: &[Value],
        scratch: Option<&Path>,
    ) -> Result<String>;
    fn completed(&mut self, state: &Value, step: &Value, receipt: &Value) -> Result<()>;
}

pub type SharedDispatchContext = Arc<Mutex<dyn DispatchContext>>;
fn context_completed(
    context: &Option<SharedDispatchContext>,
    state: &Value,
    step: &Value,
    receipt: &Value,
) -> Result<()> {
    if let Some(context) = context {
        context
            .lock()
            .map_err(|_| fail("Dispatch context lock poisoned"))?
            .completed(state, step, receipt)?;
    }
    Ok(())
}

/// A caller changing entry points on resume must not promote earlier pilot claims.
fn experimental_context(directory: &Path, state: &Value, enabled: bool) -> Result<bool> {
    let name = "experimental-context.json";
    let expected =
        object!({"schema_version":1,"kind":"ai-system-untrusted-context","run_id":state["run_id"]});
    match read_json(directory, name, 4096) {
        Ok(marker) if marker == expected => Ok(true),
        Ok(_) => Err(fail("Experimental context marker does not match this run")),
        Err(error) if error.kind == ErrorKind::NotFound => {
            if enabled {
                save(directory, name, &expected, true)?;
            }
            Ok(enabled)
        }
        Err(_) => Err(fail("Experimental context marker is invalid")),
    }
}

fn notify(event: &mut (impl FnMut(Value) -> Result<()> + ?Sized), value: Value) {
    let _ = event(value);
}
fn reference_mut<'a>(state: &'a mut Value, sid: &str) -> Result<&'a mut Value> {
    state["tasks"]
        .get_mut(sid)
        .ok_or_else(|| fail("Missing native task reference"))
}
fn step_index(state: &Value, id: &str) -> Result<usize> {
    state["steps"]
        .as_array()
        .and_then(|steps| steps.iter().position(|step| step["id"] == id))
        .ok_or_else(|| fail("Missing dispatch step"))
}
/// `person_accepted` is true only on a resume that accepts source changes:
/// then a person's content edit to a source the plan fingerprints in the
/// system folder, which no worker may write, is adopted too (the system file
/// itself still needs a new plan).
pub fn check_changes(
    system: &mut System,
    plan: &Value,
    step: Option<&Value>,
    report: Option<&Value>,
    state: &Value,
    person_accepted: bool,
) -> Result<Value> {
    let verification = system.verify(plan)?;
    if verification["fresh"] == true {
        return Ok(plan.clone());
    }
    let writable = step
        .filter(|step| step["mode"] == "edit")
        .map(|step| texts(&dispatch_access(state, step)["writable"]))
        .unwrap_or_default();
    let reported = if let (Some(step), Some(report)) = (step, report) {
        Some(
            texts(&report["changed_files"])
                .into_iter()
                .map(|path| changed_path(state, step, &path))
                .collect::<Result<BTreeSet<_>>>()?,
        )
    } else {
        None
    };
    let mut accepted = BTreeSet::new();
    for change in items(
        &verification["changed"],
        MAX_SERVICES * MAX_ITEMS * 3 + MAX_ITEMS,
    )? {
        // Shared memory that stopped holding only leaves the context, which
        // the refresh below does; it is no change to a source.
        if change["reason"] == "memory_ineligible" {
            continue;
        }
        let sid = string(&change["service"]);
        let path = string(&change["path"]);
        let place = if sid == "__system__" {
            format!(
                "System folder {}",
                if path.is_empty() { "head" } else { path }
            )
        } else if path.is_empty() {
            format!("{sid} head")
        } else {
            format!("{sid}/{path}")
        };
        let reason = string(&change["reason"]).replace('_', " ");
        if sid == "__system__" {
            // The system folder is the system file's parent.
            let config = system.config.file_name().and_then(|name| name.to_str());
            if config == Some(path) || change["reason"] != "content_changed" {
                return Err(fail(format!(
                    "{place} changed ({reason}); the system file and the folder's sources define the run; prepare a new plan"
                )));
            }
            if !person_accepted {
                return Err(fail(format!(
                    "{place} changed ({reason}), and no worker may write the system folder; inspect the edit, then resume accepting source changes (CLI: --accept-source-changes) or prepare a new plan"
                )));
            }
            accepted.insert((sid.to_string(), path.to_string()));
            continue;
        }
        let Some(service) = system.services.get(sid) else {
            return Err(fail(format!(
                "{place} changed ({reason}) outside the worker's scope; prepare a new plan"
            )));
        };
        let why = if step.is_none() {
            Some("while no worker was running")
        } else if !writable.iter().any(|value| value == sid) {
            Some("outside current worker scope")
        } else if change["reason"] != "content_changed" {
            Some("in a way a worker may not change it")
        } else if path == service.manifest_path {
            Some("as the service passport, which only Save system writes")
        } else if reported
            .as_ref()
            .is_some_and(|reported| !reported.contains(&service.root.join(path)))
        {
            Some("without being reported in changed_files")
        } else {
            None
        };
        if let Some(why) = why {
            return Err(fail(format!(
                "{place} changed ({reason}) {why}; prepare a new plan"
            )));
        }
        accepted.insert((sid.to_string(), path.to_string()));
    }
    system.refresh_checkpoint(plan, &accepted)
}
/// What a person accepted when resuming a stopped run.
#[derive(Clone, Copy, Debug, Default)]
pub struct Acceptance {
    /// Source edits since the checkpoint: a worker's partial edits, or a
    /// person's own edit of a system-folder source.
    pub source_changes: bool,
    /// The checks a completed verification could not run here.
    pub unrun_checks: bool,
}
impl Acceptance {
    pub fn sources(source_changes: bool) -> Self {
        Self {
            source_changes,
            unrun_checks: false,
        }
    }
}
fn finish(
    directory: &Path,
    state: &mut Value,
    step: &Value,
    receipt: &Value,
    brains: &BTreeMap<String, Brain>,
) -> Result<()> {
    let sid = string(&step["service"]);
    let brain = brains
        .get(sid)
        .ok_or_else(|| fail("Missing dispatch Brain"))?;
    let index = step_index(state, string(&step["id"]))?;
    let accepted = texts(&state["steps"][index]["accepted_unrun_checks"]);
    brain.progress(
        reference_mut(state, sid)?,
        &format!(
            "Dispatch {} reported complete: {}{}",
            string(&step["dispatch_id"]),
            string(&step["id"]),
            if accepted.is_empty() {
                String::new()
            } else {
                format!(
                    "; a person accepted the checks it could not run: {}",
                    accepted.join(", ")
                )
            }
        ),
    )?;
    state["steps"][index]["status"] = "completed".into();
    if step["id"] == "contracts" {
        let mut by_id = state["steps"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|step| (string(&step["id"]).to_string(), step.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut ordered = vec![
            by_id
                .remove("contracts")
                .ok_or_else(|| fail("Missing contract step"))?,
        ];
        for sid in texts(&receipt["report"]["service_order"]) {
            ordered.push(
                by_id
                    .remove(&format!("service-{sid}"))
                    .ok_or_else(|| fail("Contract order refers to unknown service"))?,
            );
        }
        ordered.push(
            by_id
                .remove("verify")
                .ok_or_else(|| fail("Missing verification step"))?,
        );
        if !by_id.is_empty() {
            return Err(fail("Contract order omitted dispatch"));
        }
        state["steps"] = object!(ordered);
    }
    save(directory, "run.json", state, false)
}
#[derive(Default)]
struct NativeEvidenceCollector {
    opened: BTreeSet<String>,
    edited: BTreeMap<String, String>,
    shell_reads: BTreeMap<String, i64>,
    shell_events: Vec<Value>,
    graph_asked: bool,
    steps: usize,
    searched: usize,
    commands: usize,
    helpers: usize,
    file_touches: Vec<Value>,
    plan_items: Vec<Value>,
    command_runs: Vec<Value>,
    seq: i64,
}

impl NativeEvidenceCollector {
    fn observe_line(&mut self, raw: &[u8], provider: &str, root: &Path) {
        if !["codex", "claude", "cursor"].contains(&provider) {
            return;
        }
        let Ok(event) = parse_json(raw, 2 * MAX_BYTES) else {
            return;
        };
        if !event.is_object() {
            return;
        }
        self.seq += 1;
        // Codex reports the full shell command on the raw item; normalized
        // targets only keep a bounded command head.
        let raw_command = if provider == "codex" && event["item"]["type"] == "command_execution" {
            match &event["item"]["command"] {
                Value::String(text) => text.clone(),
                Value::Array(parts) => {
                    let parts = parts.iter().filter_map(Value::as_str).collect::<Vec<_>>();
                    if parts.len() == 3 && ["-lc", "-c"].contains(&parts[1]) {
                        parts[2].to_string()
                    } else {
                        parts.join(" ")
                    }
                }
                _ => String::new(),
            }
        } else {
            String::new()
        };
        for norm in infra_host::provider::normalize(provider, &event) {
            if norm["kind"] != "tool" {
                continue;
            }
            let target = &norm["targets"];
            // Codex reports each item as started and completed; count it once.
            if provider == "codex" && target["state"] == "started" {
                continue;
            }
            self.steps += 1;
            let name = [&target["tool"], &norm["name"]]
                .into_iter()
                .find_map(|value| value.as_str().filter(|s| !s.is_empty()))
                .unwrap_or_default()
                .to_lowercase();
            let input_owned = if !raw_command.is_empty() && name == "shell" {
                raw_command.clone()
            } else {
                [&target["path"], &target["detail"], &norm["input"]]
                    .into_iter()
                    .find_map(|value| value.as_str().filter(|s| !s.is_empty()))
                    .unwrap_or_default()
                    .to_string()
            };
            let input_text = input_owned.as_str();
            if matches!(
                name.as_str(),
                "grep" | "glob" | "search" | "ripgrep" | "find_by_name" | "grep_search"
            ) {
                self.searched += 1;
            } else if matches!(name.as_str(), "task" | "agent" | "subagent") {
                self.helpers += 1;
            } else if matches!(
                name.as_str(),
                "bash" | "shell" | "exec_command" | "command" | "run_terminal_cmd"
            ) {
                self.commands += 1;
                if input_text.contains("graphify ")
                    && input_text.contains("--graph")
                    && (input_text.contains(" query ")
                        || input_text.contains(" explain ")
                        || input_text.contains(" path "))
                {
                    self.graph_asked = true;
                }
                for shell_path in extract_shell_read_paths(input_text, root) {
                    self.opened.insert(shell_path.clone());
                    self.shell_reads
                        .entry(shell_path.clone())
                        .or_insert(self.seq);
                    if self.shell_events.len() < 100 {
                        self.shell_events.push(object!({
                            "id": self.seq,
                            "path": shell_path,
                            "command": clipped_str(input_text, 240),
                        }));
                    }
                }
                if self.command_runs.len() < 100 && !input_text.is_empty() {
                    self.command_runs.push(object!({
                        "id": self.seq,
                        "command": clipped_str(input_text, 240),
                        "status": norm.get("status").cloned().unwrap_or_else(|| object!("completed")),
                    }));
                }
            } else if matches!(name.as_str(), "read" | "read_file" | "view") {
                if let Some(rel) = normalize_tool_path(input_text, root) {
                    self.opened.insert(rel.clone());
                    if self.file_touches.len() < 200 {
                        self.file_touches
                            .push(object!({ "path": rel, "action": "read" }));
                    }
                }
            } else if matches!(
                name.as_str(),
                "edit" | "write" | "multiedit" | "apply_patch" | "file_change" | "write_file"
            ) && let Some(rel) = normalize_tool_path(input_text, root)
            {
                self.opened.insert(rel.clone());
                self.edited.insert(rel.clone(), "modified".into());
                if self.file_touches.len() < 200 {
                    self.file_touches
                        .push(object!({ "path": rel, "action": "edit" }));
                }
            } else if matches!(name.as_str(), "todowrite" | "update_plan")
                && self.plan_items.len() < 50
            {
                self.plan_items
                    .push(object!({ "step": self.seq, "summary": clipped_str(input_text, 240) }));
            }
        }
    }
}

fn clipped_str(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        let mut out = value
            .chars()
            .take(max_chars.saturating_sub(1))
            .collect::<String>();
        out.push('…');
        out
    }
}

fn normalize_tool_path(raw: &str, root: &Path) -> Option<String> {
    let candidate = raw.trim().trim_matches('"').trim_matches('\'');
    if candidate.is_empty() {
        return None;
    }
    let path = Path::new(candidate);
    let rel = if path.is_absolute() {
        path.strip_prefix(root).ok()?.to_str()?
    } else {
        candidate.strip_prefix("./").unwrap_or(candidate)
    };
    let clean = rel.replace('\\', "/");
    relative(&clean).ok()
}

fn extract_shell_read_paths(command: &str, root: &Path) -> Vec<String> {
    let trimmed = command.trim();
    let is_reader = trimmed.starts_with("sed ")
        || trimmed.contains(" sed ")
        || trimmed.starts_with("cat ")
        || trimmed.starts_with("head ")
        || trimmed.starts_with("tail ")
        || trimmed.starts_with("nl ")
        || trimmed.starts_with("bat ")
        || trimmed.starts_with("awk ");
    if !is_reader {
        return vec![];
    }
    let mut found = Vec::new();
    for token in trimmed.split_whitespace() {
        let clean_tok = token.trim_matches(|c| matches!(c, '"' | '\'' | ';' | '|' | '&' | ')'));
        if clean_tok.is_empty() || clean_tok.starts_with('-') || clean_tok.contains('=') {
            continue;
        }
        if let Some(rel) = normalize_tool_path(clean_tok, root)
            && root.join(&rel).is_file()
            && !found.contains(&rel)
        {
            found.push(rel);
        }
    }
    found
}

fn collect_step_git_changes(
    state: &Value,
    step: &Value,
    root: &Path,
    base_head: Option<&str>,
    pre_existing_dirty: &BTreeSet<String>,
    report: Option<&Value>,
) -> Vec<EvidenceFileChange> {
    let mut out = BTreeMap::<String, EvidenceFileChange>::new();
    let reported_set: BTreeSet<String> = report
        .map(|r| texts(&r["changed_files"]).into_iter().collect())
        .unwrap_or_default();

    if access_scope(state) == "all" && step["mode"] == "edit" {
        for sid in texts(&state["selected"]) {
            let svc_root = Path::new(string(&state["roots"][&sid]));
            for ch in git_changes_since(svc_root, if svc_root == root { base_head } else { None }) {
                let key = format!("{sid}/{}", ch.path);
                if !pre_existing_dirty.contains(&key) || reported_set.contains(&key) {
                    out.insert(
                        key.clone(),
                        EvidenceFileChange {
                            path: key,
                            status: ch.status,
                            from: ch.from.map(|f| format!("{sid}/{f}")),
                        },
                    );
                }
            }
        }
    } else if step["mode"] == "edit" {
        for ch in git_changes_since(root, base_head) {
            if !pre_existing_dirty.contains(&ch.path) || reported_set.contains(&ch.path) {
                out.insert(ch.path.clone(), ch);
            }
        }
    }

    for entry in &reported_set {
        out.entry(entry.clone())
            .or_insert_with(|| EvidenceFileChange {
                path: entry.clone(),
                status: "modified".into(),
                from: None,
            });
    }
    out.into_values().collect()
}

struct SystemLaunchAdapter {
    inner: Option<Arc<dyn infra_host::provider::LaunchAdapter>>,
    schema: Value,
    schema_path: PathBuf,
    extra: Vec<PathBuf>,
    read: Vec<PathBuf>,
    offers: Vec<crate::system_graph::ServiceGraphOffer>,
}

impl infra_host::provider::LaunchAdapter for SystemLaunchAdapter {
    fn adapt(
        &self,
        input: &infra_host::provider::Launch,
        mut command: infra_host::provider::PreparedCommand,
    ) -> infra_host::Result<infra_host::provider::PreparedCommand> {
        let active_transport = infra_host::provider::transport(input);
        if input.provider == "cursor"
            && active_transport == infra_host::provider::Transport::Text
            && input.prompt.len() > native::CURSOR_PROMPT_BYTES
        {
            return Err(infra_host::HarnessError::invalid(
                "Cursor prompt exceeds argument byte limit; narrow plan",
            ));
        }
        native::apply_system_launch_flags(
            &input.provider,
            active_transport,
            &input.mode,
            &self.schema,
            &self.schema_path,
            &self.extra,
            &self.read,
            &mut command.argv,
        )?;
        if let Some(inner) = &self.inner {
            command = inner.adapt(input, command)?;
        }
        Ok(crate::workers::prepare_worker_for_transport(
            &input.provider,
            &input.mode,
            active_transport,
            command,
            &self.offers,
        )?)
    }
}

struct SystemRunEvents {
    observer: Option<Arc<dyn infra_host::process::RunObserver>>,
    collector: Mutex<NativeEvidenceCollector>,
    raw_lines: Mutex<Vec<u8>>,
    bridge: Option<Arc<dyn CodeGraphBridge>>,
    graph_ctx: DispatchGraphContext,
    provider: String,
    root: PathBuf,
    phase: String,
    attempt: u64,
    tx: Mutex<Option<tokio::sync::mpsc::UnboundedSender<Value>>>,
}

impl SystemRunEvents {
    fn close(&self) {
        if let Ok(mut tx) = self.tx.lock() {
            tx.take();
        }
    }
    fn take_collector(&self) -> NativeEvidenceCollector {
        self.collector
            .lock()
            .map(|mut c| std::mem::take(&mut *c))
            .unwrap_or_default()
    }
    fn take_raw_bytes(&self) -> Vec<u8> {
        self.raw_lines
            .lock()
            .map(|mut b| std::mem::take(&mut *b))
            .unwrap_or_default()
    }
}

impl infra_host::process::RunObserver for SystemRunEvents {
    fn boundary(&self, signal: &str, details: &Value) -> infra_host::Result<()> {
        if let Some(observer) = &self.observer {
            observer.boundary(signal, details)?;
        }
        Ok(())
    }
}

impl infra_host::provider::RunEvents for SystemRunEvents {
    fn observe(&self, raw: Option<&Value>, _normalized: Option<&Value>) -> infra_host::Result<()> {
        if let Some(raw) = raw
            && let Ok(bytes) = serde_json::to_vec(raw)
        {
            if let Ok(mut collector) = self.collector.lock() {
                collector.observe_line(&bytes, &self.provider, &self.root);
            }
            if let Some(bridge) = &self.bridge {
                let _ = bridge.on_dispatch_signal(&self.graph_ctx, &bytes);
            }
            if let Ok(mut buf) = self.raw_lines.lock()
                && buf.len().saturating_add(bytes.len()).saturating_add(1) <= 8 * MAX_BYTES
            {
                buf.extend_from_slice(&bytes);
                buf.push(b'\n');
            }
            let line = String::from_utf8_lossy(&bytes).to_string();
            if let Ok(tx) = self.tx.lock()
                && let Some(tx) = tx.as_ref()
            {
                let _ = tx.send(object!({
                    "type": "output",
                    "phase": self.phase,
                    "attempt": self.attempt,
                    "line": line,
                }));
            }
        }
        Ok(())
    }
}

fn finalize_worker_report(
    raw_bytes: &[u8],
    provider: &str,
    preview: &Value,
    state: &Value,
    untrusted_context: bool,
    receipt: &mut Value,
) {
    receipt["usage_components"] = execution::usage_components_from_raw(raw_bytes, provider);
    receipt["deterministic"] = object!(false);
    match native::worker_result(raw_bytes, provider).and_then(|report| {
        if untrusted_context && infra_core::domain::secret_label(&encoded(&report)?)?.is_some() {
            return Err(fail("Sensitive experimental worker result"));
        }
        execution::validate_report(
            &report,
            preview,
            &texts(&state["selected"]),
            access_scope(state),
        )?;
        Ok(report)
    }) {
        Ok(report) => {
            receipt["report"] = report;
            receipt["ok"] = object!(execution::success(
                receipt,
                preview,
                state["mode"] == "read-only"
            ));
            if receipt["ok"] != true {
                receipt["error"] = if preview["mode"] == "read-only"
                    && !texts(&receipt["report"]["changed_files"]).is_empty()
                {
                    "read_only_worker_reported_writes"
                } else {
                    "worker_blocked_or_verification_incomplete"
                }
                .into();
            }
        }
        Err(error) => {
            receipt["error"] = "invalid_or_sensitive_worker_result".into();
            receipt["rejection"] = if untrusted_context {
                "Invalid experimental worker report".into()
            } else {
                error.message.chars().take(500).collect::<String>().into()
            };
        }
    }
}

struct HostDispatchArgs {
    launch: infra_host::provider::Launch,
    launch_adapter: SystemLaunchAdapter,
    limits: infra_host::process::Limits,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<std::fs::File>>,
    observer: Option<Arc<dyn infra_host::process::RunObserver>>,
    bridge: Option<Arc<dyn CodeGraphBridge>>,
    graph_ctx: DispatchGraphContext,
    provider: String,
    root: PathBuf,
    phase: String,
    attempt: u64,
    preview: Value,
    state: Value,
    untrusted_context: bool,
    started: Instant,
    receipt: Value,
}

struct RawDispatchArgs {
    prepared: infra_host::provider::PreparedCommand,
    root: PathBuf,
    timeout: u64,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<std::fs::File>>,
    observer: Option<Arc<dyn infra_host::process::RunObserver>>,
    bridge: Option<Arc<dyn CodeGraphBridge>>,
    graph_ctx: DispatchGraphContext,
    provider: String,
    phase: String,
    attempt: u64,
    preview: Value,
    state: Value,
    untrusted_context: bool,
    started: Instant,
    receipt: Value,
}

struct RunStepsArgs<'a> {
    system: &'a mut System,
    directory: &'a Path,
    retry: Option<&'a str>,
    accept: Acceptance,
    cancel: &'a Arc<AtomicBool>,
    runner_lock: Option<Arc<std::fs::File>>,
    adapter: Option<Arc<dyn infra_host::provider::LaunchAdapter>>,
    observer: Option<Arc<dyn infra_host::process::RunObserver>>,
    dispatch_context: &'a Option<SharedDispatchContext>,
    hooks: &'a RunnerGraphHooks,
    untrusted_context: bool,
    brains: &'a BTreeMap<String, Brain>,
    state: &'a mut Value,
    plan: Value,
    event: &'a mut (dyn FnMut(Value) -> Result<()> + Send),
}

#[inline(never)]
fn validate_state_boxed<'a>(
    system: &'a mut System,
    directory: &'a Path,
    verify_cli: bool,
) -> BoxFuture<'a, Result<Value>> {
    Box::pin(native::validate_state(system, directory, verify_cli))
}

#[inline(never)]
#[expect(
    clippy::too_many_arguments,
    reason = "Isolates infra_host::provider::run_monitored_locked into its own heap-allocated future frame."
)]
fn run_monitored_locked_boxed(
    launch: infra_host::provider::Launch,
    limits: infra_host::process::Limits,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<std::fs::File>>,
    launch_adapter: SystemLaunchAdapter,
    run_events: Arc<SystemRunEvents>,
    tx: tokio::sync::mpsc::UnboundedSender<Value>,
    phase: String,
    attempt: u64,
) -> BoxFuture<'static, infra_host::Result<infra_host::provider::WorkerResult>> {
    Box::pin(async move {
        let empty_env = BTreeMap::new();
        let res = infra_host::provider::run_monitored_locked(
            &launch,
            limits,
            cancel,
            None,
            runner_lock,
            Some(&launch_adapter),
            &empty_env,
            Some(run_events.clone()),
            move |norm| {
                if matches!(norm["kind"].as_str(), Some("permission" | "status")) {
                    let _ = tx.send(object!({
                        "type": norm["kind"],
                        "phase": phase,
                        "attempt": attempt,
                        "event": norm,
                    }));
                }
                Ok(())
            },
        )
        .await;
        run_events.close();
        res
    })
}

#[inline(never)]
fn run_host_dispatch_boxed<'a>(
    args: HostDispatchArgs,
    event: &'a mut (dyn FnMut(Value) -> Result<()> + Send),
) -> BoxFuture<'a, (NativeEvidenceCollector, Value)> {
    Box::pin(async move {
        let HostDispatchArgs {
            launch,
            launch_adapter,
            limits,
            cancel,
            runner_lock,
            observer,
            bridge,
            graph_ctx,
            provider,
            root,
            phase,
            attempt,
            preview,
            state,
            untrusted_context,
            started,
            mut receipt,
        } = args;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
        let run_events = Arc::new(SystemRunEvents {
            observer,
            collector: Mutex::new(NativeEvidenceCollector::default()),
            raw_lines: Mutex::new(Vec::new()),
            bridge,
            graph_ctx,
            provider: provider.clone(),
            root,
            phase: phase.clone(),
            attempt,
            tx: Mutex::new(Some(tx.clone())),
        });
        let run_fut = run_monitored_locked_boxed(
            launch,
            limits,
            cancel.clone(),
            runner_lock,
            launch_adapter,
            run_events.clone(),
            tx,
            phase,
            attempt,
        );
        let drain_fut = async {
            while let Some(ev) = rx.recv().await {
                notify(event, ev);
            }
        };
        let (output, ()) = tokio::join!(run_fut, drain_fut);
        let elapsed = started.elapsed().as_secs_f64();
        receipt["duration_seconds"] = object!(elapsed);
        let native_collector = run_events.take_collector();
        let raw_bytes = run_events.take_raw_bytes();
        match output {
            Ok(worker_res) => {
                if worker_res.ok {
                    receipt["returncode"] = object!(0);
                    finalize_worker_report(
                        &raw_bytes,
                        &provider,
                        &preview,
                        &state,
                        untrusted_context,
                        &mut receipt,
                    );
                } else {
                    receipt["error"] = "worker_exit_failure".into();
                    if let Some(failure) = native::worker_failure(&raw_bytes, &provider)
                        .or_else(|| worker_res.error.map(|e| native::shown_failure(&e)))
                    {
                        receipt["failure"] = failure.into();
                    }
                }
            }
            Err(error) => {
                let interrupted = cancel.load(Ordering::SeqCst) || error.code == "native-cancelled";
                receipt["error"] = if interrupted {
                    "interrupted"
                } else {
                    error.code
                }
                .into();
                if !interrupted && !error.message.trim().is_empty() {
                    receipt["failure"] = native::shown_failure(&error.message).into();
                }
            }
        }
        (native_collector, receipt)
    })
}

#[inline(never)]
fn run_raw_dispatch_boxed<'a>(
    args: RawDispatchArgs,
    event: &'a mut (dyn FnMut(Value) -> Result<()> + Send),
) -> BoxFuture<'a, (NativeEvidenceCollector, Value)> {
    Box::pin(async move {
        let RawDispatchArgs {
            prepared,
            root,
            timeout,
            cancel,
            runner_lock,
            observer,
            bridge,
            graph_ctx,
            provider,
            phase,
            attempt,
            preview,
            state,
            untrusted_context,
            started,
            mut receipt,
        } = args;
        let mut native_collector = NativeEvidenceCollector::default();
        let output = Box::pin(native::run_raw_monitored(
            &prepared.argv,
            &root,
            &prepared.environment,
            &prepared.remove,
            &prepared.input,
            timeout,
            cancel.clone(),
            runner_lock,
            observer,
            |line| {
                native_collector.observe_line(line, &provider, &root);
                if let Some(bridge) = &bridge {
                    let _ = bridge.on_dispatch_signal(&graph_ctx, line);
                }
                let line = String::from_utf8_lossy(line).to_string();
                notify(
                    event,
                    object!({"type":"output","phase":phase,"attempt":attempt,"line":line}),
                );
                Ok(())
            },
        ))
        .await;
        let elapsed = started.elapsed().as_secs_f64();
        receipt["duration_seconds"] = object!(elapsed);
        match output {
            Ok(output) => {
                receipt["returncode"] = object!(output.code);
                if output.code == 0 {
                    finalize_worker_report(
                        &output.bytes,
                        &provider,
                        &preview,
                        &state,
                        untrusted_context,
                        &mut receipt,
                    );
                } else {
                    receipt["error"] = "worker_exit_failure".into();
                    if let Some(failure) = native::worker_failure(&output.bytes, &provider) {
                        receipt["failure"] = failure.into();
                    }
                }
            }
            // A CLI that could not start or be supervised says why: a real verification stopped as only "unavailable" after the engine binary was replaced under a running Harness.
            Err(error) => {
                let interrupted = cancel.load(Ordering::SeqCst);
                receipt["error"] = if interrupted {
                    "interrupted"
                } else {
                    error.code.as_deref().unwrap_or("worker_process_failure")
                }
                .into();
                if !interrupted && !error.message.trim().is_empty() {
                    receipt["failure"] = native::shown_failure(&error.message).into();
                }
            }
        }
        (native_collector, receipt)
    })
}

#[inline(never)]
fn execute_run_steps_boxed<'a>(args: RunStepsArgs<'a>) -> BoxFuture<'a, Result<Value>> {
    Box::pin(async move {
        let RunStepsArgs {
            system,
            directory,
            retry,
            accept,
            cancel,
            runner_lock,
            adapter,
            observer,
            dispatch_context,
            hooks,
            untrusted_context,
            brains,
            state,
            mut plan,
            event,
        } = args;
        let retry = retry.map(str::to_string);
        let blocked = state["steps"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|step| ["running", "blocked", "interrupted"].contains(&string(&step["status"])))
            .cloned();
        if let Some(step) = blocked {
            let receipt = match dispatch_receipt(directory, state, &step) {
                Ok(value) => Some(value),
                Err(error) if error.kind == ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            };
            let unrun = receipt
                .as_ref()
                .and_then(|receipt| execution::unrun_checks(receipt, &step));
            if accept.unrun_checks && unrun.is_none() {
                return Err(fail(format!(
                    "Only a completed verification whose only gaps are checks it could not run can be accepted; retry {id} instead (CLI: --retry-step {id})",
                    id = string(&step["id"])
                )));
            }
            if accept.unrun_checks && retry.is_some() {
                return Err(fail(
                    "Accept the checks verification could not run, or retry verification, not both",
                ));
            }
            if receipt
                .as_ref()
                .is_some_and(|receipt| receipt["ok"] == true)
                || accept.unrun_checks
            {
                let receipt = receipt.ok_or_else(|| fail("Receipt disappeared"))?;
                if system.verify(&plan)?["fresh"] != true {
                    if !accept.source_changes {
                        return Err(fail(
                            "Successful receipt has uncheckpointed edits; inspect and explicitly accept source changes",
                        ));
                    }
                    plan = check_changes(
                        system,
                        &plan,
                        Some(&step),
                        Some(&receipt["report"]),
                        state,
                        true,
                    )?;
                    save(directory, "checkpoint.json", &plan, false)?;
                }
                if let Some(unrun) = unrun.filter(|_| accept.unrun_checks) {
                    let index = step_index(state, string(&step["id"]))?;
                    state["steps"][index]["accepted_unrun_checks"] = object!(unrun);
                }
                finish(directory, state, &step, &receipt, brains)?;
                context_completed(dispatch_context, state, &step, &receipt)?;
            } else {
                if retry.as_deref() != Some(string(&step["id"])) {
                    return Err(fail(format!(
                        "Dispatch {id} stopped without a successful receipt and may have changed files; inspect them, then retry {id} explicitly (CLI: --retry-step {id})",
                        id = string(&step["id"])
                    )));
                }
                if system.verify(&plan)?["fresh"] != true {
                    if !accept.source_changes {
                        return Err(fail(
                            "Partial edits detected; inspect and explicitly accept source changes",
                        ));
                    }
                    plan = check_changes(system, &plan, Some(&step), None, state, true)?;
                    save(directory, "checkpoint.json", &plan, false)?;
                }
                let index = step_index(state, string(&step["id"]))?;
                state["steps"][index]["status"] = "pending".into();
            }
        } else if retry.is_some() {
            return Err(fail(
                "No unfinished dispatch matches the dispatch chosen for retry",
            ));
        } else if accept.unrun_checks {
            return Err(fail("No stopped verification has checks to accept"));
        }
        // Between dispatches only a person's accepted system-folder edit is adopted;
        // any other change is named and needs a new plan.
        if system.verify(&plan)?["fresh"] != true {
            plan = check_changes(system, &plan, None, None, state, accept.source_changes)?;
            save(directory, "checkpoint.json", &plan, false)?;
        }
        plan = system.refresh_checkpoint(&plan, &BTreeSet::new())?;
        let ids = brains.keys().cloned().collect::<Vec<_>>();
        for sid in &ids {
            let brain = brains.get(sid).ok_or_else(|| fail("Missing Brain"))?;
            let record = brain.ensure(reference_mut(state, sid)?)?;
            if record["status"] == "completed" && !texts(&state["closed_tasks"]).contains(sid) {
                if state["steps"]
                    .as_array()
                    .is_some_and(|steps| steps.iter().any(|step| step["status"] != "completed"))
                {
                    return Err(fail("Native task terminal before worker verification"));
                }
                let run = string(&state["run_id"]).to_string();
                brain.complete(reference_mut(state, sid)?, &run)?;
                state["closed_tasks"]
                    .as_array_mut()
                    .ok_or_else(|| fail("Closed tasks missing"))?
                    .push(sid.clone().into());
            }
            save(directory, "run.json", state, false)?;
        }
        state["status"] = "running".into();
        state["error"] = Value::Null;
        save(directory, "run.json", state, false)?;
        let steps = state["steps"]
            .as_array()
            .cloned()
            .ok_or_else(|| fail("Missing steps"))?;
        let mut cached_system = CachedSystem::from_system(system);
        for step in steps {
            if step["status"] == "completed" {
                continue;
            }
            if cancel.load(Ordering::SeqCst) {
                return Err(RuntimeError::coded(
                    ErrorKind::Unavailable,
                    "interrupted",
                    "interrupted",
                ));
            }
            let current_ref = cached_system.get_or_reload(&system.allow_roots)?;
            let verdict = current_ref.verify(&plan)?;
            if verdict["fresh"] != true {
                // Memory that stopped holding between dispatches leaves the shared context; any other change needs a new plan.
                if verdict["changed"].as_array().is_some_and(|changed| {
                    changed
                        .iter()
                        .all(|change| change["reason"] == "memory_ineligible")
                }) {
                    plan = current_ref.refresh_checkpoint(&plan, &BTreeSet::new())?;
                    save(directory, "checkpoint.json", &plan, false)?;
                } else {
                    check_changes(current_ref, &plan, None, None, state, false)?;
                    return Err(fail("Sources changed between dispatches; prepare new plan"));
                }
            }
            let current = current_ref.clone();
            let prior = reports(directory, state)?;
            let sid = string(&step["service"]).to_string();
            let provider = string(&state["provider"]).to_string();
            let health_db_path = directory
                .parent()
                .unwrap_or(directory)
                .join("system_health.sqlite3");
            let PrelaunchHealthDecision {
                active_model,
                max_attempts,
            } = evaluate_prelaunch_health(&health_db_path, &provider, &step, state, event);
            let attempt = step["attempt"]
                .as_u64()
                .filter(|attempt| *attempt < max_attempts)
                .ok_or_else(|| fail("Dispatch retry limit reached"))?
                + 1;
            let mut preview = step.clone();
            preview["dispatch_id"] = id32().into();
            preview["status"] = "running".into();
            preview["attempt"] = object!(attempt);
            let executable = PathBuf::from(string(&state["executable"]));
            let scratch = native::read_only_scratch(
                &provider,
                &executable,
                string(&step["mode"]),
                directory,
            )?;
            let access = dispatch_access(state, &step);
            let root = PathBuf::from(string(&state["roots"][&sid]));
            let schema = execution::result_schema(&texts(&state["selected"]));
            let roots_map = state["roots"]
                .as_object()
                .ok_or_else(|| fail("Missing run roots"))?
                .iter()
                .map(|(k, v)| (k.clone(), PathBuf::from(string(v))))
                .collect::<BTreeMap<_, _>>();
            let base_head = git_head(&root);
            let pre_existing_dirty: BTreeSet<String> =
                if access_scope(state) == "all" && step["mode"] == "edit" {
                    texts(&state["selected"])
                        .into_iter()
                        .flat_map(|s| {
                            let sr = Path::new(string(&state["roots"][&s]));
                            git_worktree_changes(sr)
                                .into_iter()
                                .map(move |c| format!("{s}/{}", c.path))
                        })
                        .collect()
                } else if step["mode"] == "edit" {
                    git_worktree_changes(&root)
                        .into_iter()
                        .map(|c| c.path)
                        .collect()
                } else {
                    BTreeSet::new()
                };
            let journal_run_id = string(&state["run_id"]).to_string();
            let outer_run_id = hooks
                .system_run_id
                .clone()
                .unwrap_or_else(|| journal_run_id.clone());
            let mut graph_ctx = DispatchGraphContext {
                run_id: outer_run_id,
                journal_run_id,
                launch_id: hooks.launch_id.clone(),
                execution_dir: directory.to_path_buf(),
                system_file: system.config.clone(),
                provider: provider.clone(),
                mode: string(&preview["mode"]).to_string(),
                access: access_scope(state).to_string(),
                tell_agents_graph: state.get("tell_agents_graph").and_then(Value::as_bool),
                phase: string(&preview["id"]).to_string(),
                dispatch_id: string(&preview["dispatch_id"]).to_string(),
                attempt,
                service: sid.clone(),
                root: root.clone(),
                roots: roots_map,
                selected: texts(&state["selected"]),
                readable: texts(&access["readable"]),
                writable: texts(&access["writable"]),
                base_head: base_head.clone(),
                head_after: None,
                plan: plan.clone(),
                state: state.clone(),
            };
            let raw_offers = match &hooks.bridge {
                Some(bridge) => bridge.offers_for_dispatch(&graph_ctx)?,
                None => vec![],
            };
            let step_offers = crate::workers::offers_for_step(state, &preview, &raw_offers);
            let verify_plan_step = (preview["id"] == "verify")
                .then(|| {
                    plan["steps"]
                        .as_array()
                        .and_then(|steps| steps.iter().find(|s| s["id"] == "verify"))
                })
                .flatten();
            let verify_programs = verify_plan_step
                .and_then(|s| s["program_checks"].as_array())
                .cloned()
                .unwrap_or_default();
            let deterministic_only =
                verify_plan_step.is_some_and(|s| s["deterministic_only"] == true);
            let require_llm_review =
                should_require_verify_llm_review(verify_plan_step, state, &prior);
            let mut deterministic_short_circuit = None;
            let mut det_prompt_note: Option<String> = None;
            if !verify_programs.is_empty() {
                let det = execution::run_deterministic_verify_checks_cached(
                    &verify_programs,
                    state,
                    cancel.clone(),
                    runner_lock.clone(),
                    Some(&health_db_path),
                )
                .await?;
                if (det.all_passed && !require_llm_review)
                    || (!det.all_passed && deterministic_only)
                {
                    deterministic_short_circuit = Some(det);
                } else if det.all_passed {
                    det_prompt_note = Some(format!(
                        "\n\nDeterministic verification pre-checks passed ({} check(s)).",
                        det.checks.len()
                    ));
                } else {
                    det_prompt_note = Some(format!(
                        "\n\nDeterministic verification pre-checks failed (waking verification worker with bounded diagnostics):\n{}",
                        det.diagnostics_summary
                    ));
                }
            }
            let mut prompt = match dispatch_context {
                Some(context) => context
                    .lock()
                    .map_err(|_| fail("Dispatch context lock poisoned"))?
                    .prompt(state, &plan, &preview, &prior, scratch.as_deref())?,
                None => native::prompt_for_with_graphs(
                    state,
                    &plan,
                    &preview,
                    &prior,
                    scratch.as_deref(),
                    &step_offers,
                )?,
            };
            if let Some(note) = &det_prompt_note {
                prompt.push_str(note);
            }
            let step_permissions = if step["mode"] == "edit" {
                state
                    .get("permissions")
                    .and_then(Value::as_str)
                    .and_then(PermissionMode::parse)
            } else {
                None
            };
            let use_host_permissions = provider != "command" && step_permissions.is_some();
            let extra_dirs = execution::extra_directories(state, &step, directory);
            let read_dirs = execution::read_directories(state, &step, directory);
            let schema_path = directory.join("result-schema.json");
            let boundary_scope = if step["mode"] == "edit" {
                Some(BoundaryScope::from_run(&current, state, &plan, &preview)?)
            } else {
                None
            };
            let mut prepared_opt = None;
            let mut host_launch_opt = None;
            let mut active_transport = infra_host::provider::Transport::Text;
            if deterministic_short_circuit.is_some() {
                // Resolved deterministically without spawning a worker LLM.
            } else if use_host_permissions {
                let perm_mode = step_permissions.unwrap();
                let mut launch = infra_host::provider::Launch {
                    provider: provider.clone(),
                    executable: executable.clone(),
                    project: root.clone(),
                    prompt: prompt.clone(),
                    mode: "edit".into(),
                    model: active_model.clone(),
                    session: None,
                    agents: false,
                    count: 1,
                    effort: None,
                    usd: None,
                    hook_events: false,
                    permissions: Some(perm_mode),
                    approvals: None,
                };
                active_transport = infra_host::provider::transport(&launch);
                if let Some(scope) = boundary_scope.as_ref() {
                    let wrap = (perm_mode == PermissionMode::Manual && hooks.approvals.is_some())
                        || (perm_mode == PermissionMode::Auto && provider == "claude");
                    if wrap {
                        launch.approvals = Some(Arc::new(SystemBoundaryApprovals::new(
                            hooks.approvals.clone(),
                            Some(perm_mode),
                            scope.clone(),
                        )));
                    }
                }
                host_launch_opt = Some(launch);
            } else {
                let prepared = native::prepare_launch(
                    &provider,
                    &executable,
                    &root,
                    &prompt,
                    &schema,
                    &schema_path,
                    string(&step["mode"]),
                    &extra_dirs,
                    &read_dirs,
                    adapter.as_deref(),
                )?;
                let prepared = match &scratch {
                    Some(scratch) => native::use_scratch(prepared, scratch)?,
                    None => prepared,
                };
                let prepared = crate::workers::prepare_worker(
                    &provider,
                    string(&step["mode"]),
                    prepared,
                    &step_offers,
                )?;
                prepared_opt = Some(prepared);
            }
            let check_non_interactive_boundary = step["mode"] == "edit"
                && step_permissions.is_some()
                && active_transport == infra_host::provider::Transport::Text;
            let (system_base_head, pre_existing_system_dirty, foreign_pre_existing) =
                if check_non_interactive_boundary {
                    let sys_head = git_head(&current.root);
                    let sys_dirty: BTreeSet<String> = git_worktree_changes(&current.root)
                        .into_iter()
                        .map(|c| c.path)
                        .collect();
                    let mut foreign = BTreeMap::new();
                    if access_scope(state) == "service" {
                        for s in texts(&state["selected"]) {
                            if s == sid {
                                continue;
                            }
                            let sr = PathBuf::from(string(&state["roots"][&s]));
                            let f_head = git_head(&sr);
                            let f_dirty: BTreeSet<String> = git_worktree_changes(&sr)
                                .into_iter()
                                .map(|c| c.path)
                                .collect();
                            foreign.insert(s, (sr, f_head, f_dirty));
                        }
                    }
                    (sys_head, sys_dirty, foreign)
                } else {
                    (None, BTreeSet::new(), BTreeMap::new())
                };
            preview["input_sha256"] = digest(prompt.as_bytes()).into();
            let index = step_index(state, string(&step["id"]))?;
            state["steps"][index] = preview.clone();
            graph_ctx.state = state.clone();
            save(directory, "run.json", state, false)?;
            // The exact input this agent receives, next to its receipt: what every agent of the run shared can be read back.
            save(
                directory,
                &input_name(&preview),
                &object!({
                    "schema_version": 1,
                    "kind": "ai-system-dispatch-input",
                    "dispatch_id": preview["dispatch_id"],
                    "phase": preview["id"],
                    "attempt": attempt,
                    "input_sha256": preview["input_sha256"],
                    "prompt": prompt
                }),
                false,
            )?;
            brains
                .get(&sid)
                .ok_or_else(|| fail("Missing dispatch Brain"))?
                .progress(
                    reference_mut(state, &sid)?,
                    &format!(
                        "Dispatch {} started: {}",
                        string(&preview["dispatch_id"]),
                        string(&preview["id"])
                    ),
                )?;
            if let Some(bridge) = &hooks.bridge {
                bridge.on_dispatch_start(&graph_ctx)?;
            }
            notify(
                event,
                object!({
                    "type": "dispatch_started",
                    "phase": preview["id"],
                    "service": sid,
                    "attempt": attempt,
                    "dispatch_id": preview["dispatch_id"],
                    "mode": preview["mode"],
                    "provider": provider,
                    "root": root,
                    "readable": access["readable"],
                    "writable": access["writable"]
                }),
            );
            let started = Instant::now();
            let timeout = state["timeout"]
                .as_u64()
                .ok_or_else(|| fail("Worker timeout missing"))?;
            let phase = string(&preview["id"]).to_string();
            let receipt = object!({
                "dispatch_id": preview["dispatch_id"],
                "phase": preview["id"],
                "input_sha256": preview["input_sha256"],
                "mode": preview["mode"],
                "returncode": null,
                "error": null,
                "duration_seconds": 0.0,
                "ok": false,
                "report": null
            });
            let is_deterministic_short_circuit = deterministic_short_circuit.is_some();
            let (native_collector, mut receipt) = if let Some(det) = deterministic_short_circuit {
                let mut det_receipt = receipt;
                det_receipt["duration_seconds"] = object!(det.duration_seconds);
                det_receipt["deterministic"] = object!(true);
                det_receipt["usage_components"] =
                    execution::empty_usage_components(Some("deterministic"), 0);
                if det.all_passed {
                    let report = object!({
                        "status": "completed",
                        "summary": format!(
                            "Deterministic verification passed ({} check(s), 0 model calls).",
                            det.checks.len()
                        ),
                        "checks": det.checks,
                        "changed_files": [],
                        "service_order": []
                    });
                    execution::validate_report(
                        &report,
                        &preview,
                        &texts(&state["selected"]),
                        access_scope(state),
                    )?;
                    det_receipt["returncode"] = object!(0);
                    det_receipt["report"] = report;
                    det_receipt["ok"] = object!(true);
                } else {
                    let summary = if det.diagnostics_summary.is_empty() {
                        "Deterministic verification checks failed.".to_owned()
                    } else {
                        format!(
                            "Deterministic verification failed: {}",
                            det.diagnostics_summary
                        )
                    };
                    let report = object!({
                        "status": "blocked",
                        "summary": summary,
                        "checks": det.checks,
                        "changed_files": [],
                        "service_order": []
                    });
                    det_receipt["returncode"] = object!(1);
                    det_receipt["report"] = report;
                    det_receipt["ok"] = object!(false);
                    det_receipt["error"] = "worker_blocked_or_verification_incomplete".into();
                }
                (NativeEvidenceCollector::default(), det_receipt)
            } else if let Some(launch) = host_launch_opt {
                let launch_adapter = SystemLaunchAdapter {
                    inner: adapter.clone(),
                    schema: schema.clone(),
                    schema_path: schema_path.clone(),
                    extra: extra_dirs,
                    read: read_dirs,
                    offers: step_offers.clone(),
                };
                let limits = infra_host::process::Limits {
                    seconds: Some(timeout),
                    output: 2 * MAX_BYTES,
                    line: 2 * MAX_BYTES,
                };
                run_host_dispatch_boxed(
                    HostDispatchArgs {
                        launch,
                        launch_adapter,
                        limits,
                        cancel: cancel.clone(),
                        runner_lock: runner_lock.clone(),
                        observer: observer.clone(),
                        bridge: hooks.bridge.clone(),
                        graph_ctx: graph_ctx.clone(),
                        provider: provider.clone(),
                        root: root.clone(),
                        phase,
                        attempt,
                        preview: preview.clone(),
                        state: state.clone(),
                        untrusted_context,
                        started,
                        receipt,
                    },
                    event,
                )
                .await
            } else {
                let prepared = prepared_opt.expect("raw launch must prepare command");
                run_raw_dispatch_boxed(
                    RawDispatchArgs {
                        prepared,
                        root: root.clone(),
                        timeout,
                        cancel: cancel.clone(),
                        runner_lock: runner_lock.clone(),
                        observer: observer.clone(),
                        bridge: hooks.bridge.clone(),
                        graph_ctx: graph_ctx.clone(),
                        provider: provider.clone(),
                        phase,
                        attempt,
                        preview: preview.clone(),
                        state: state.clone(),
                        untrusted_context,
                        started,
                        receipt,
                    },
                    event,
                )
                .await
            };
            let elapsed = receipt["duration_seconds"].as_f64().unwrap_or(0.0);
            let head_after = git_head(&root);
            graph_ctx.head_after = head_after.clone();
            let git_changed = collect_step_git_changes(
                state,
                &preview,
                &root,
                base_head.as_deref(),
                &pre_existing_dirty,
                receipt.get("report").filter(|v| v.is_object()),
            );
            if check_non_interactive_boundary
                && receipt["ok"] == true
                && let Some(scope) = boundary_scope.as_ref()
            {
                let mut extra_written = Vec::new();
                for ch in git_changes_since(&current.root, system_base_head.as_deref()) {
                    if !pre_existing_system_dirty.contains(&ch.path) {
                        extra_written.push(current.root.join(&ch.path));
                    }
                }
                for (_foreign_sid, (sr, f_head, f_dirty)) in foreign_pre_existing {
                    for ch in git_changes_since(&sr, f_head.as_deref()) {
                        if !f_dirty.contains(&ch.path) {
                            extra_written.push(sr.join(&ch.path));
                        }
                    }
                }
                let reported_files = texts(&receipt["report"]["changed_files"]);
                if let Err(boundary_err) = check_step_boundary_writes_with_extra(
                    scope,
                    &git_changed,
                    &reported_files,
                    &extra_written,
                ) {
                    receipt["ok"] = object!(false);
                    receipt["error"] = "boundary_violation".into();
                    receipt["rejection"] = boundary_err
                        .message
                        .chars()
                        .take(500)
                        .collect::<String>()
                        .into();
                }
            }
            save(directory, &receipt_name(&preview), &receipt, true)?;
            let bridge_evidence = match &hooks.bridge {
                Some(bridge) => bridge.on_dispatch_finish(&graph_ctx, &receipt)?,
                None => None,
            };
            let evidence = match bridge_evidence {
                Some(mut ev) => {
                    ev.phase = string(&preview["id"]).to_string();
                    ev.attempt = attempt;
                    if ev.launch_id.is_none() {
                        ev.launch_id = hooks.launch_id.clone();
                    }
                    ev.service = sid.clone();
                    ev.root = path_string(&root)?;
                    if ev.base_head.is_none() {
                        ev.base_head = base_head;
                    }
                    if ev.head_after.is_none() {
                        ev.head_after = head_after;
                    }
                    ev.git_changed = git_changed;
                    ev
                }
                None => {
                    let mut ledger_edited: Vec<EvidenceFileChange> = native_collector
                        .edited
                        .into_iter()
                        .map(|(path, status)| EvidenceFileChange {
                            path,
                            status,
                            from: None,
                        })
                        .collect();
                    if ledger_edited.is_empty() {
                        for entry in texts(&receipt["report"]["changed_files"]) {
                            ledger_edited.push(EvidenceFileChange {
                                path: entry,
                                status: "modified".into(),
                                from: None,
                            });
                        }
                    }
                    DispatchEvidence {
                        phase: string(&preview["id"]).to_string(),
                        attempt,
                        launch_id: hooks.launch_id.clone(),
                        service: sid.clone(),
                        root: path_string(&root)?,
                        base_head,
                        head_after,
                        git_changed,
                        ledger_edited,
                        opened: native_collector.opened.into_iter().collect(),
                        shell_reads: native_collector.shell_reads,
                        shell_events: native_collector.shell_events,
                        graph_asked: native_collector.graph_asked,
                        steps: native_collector.steps,
                        searched: native_collector.searched,
                        commands: native_collector.commands,
                        helpers: native_collector.helpers,
                        limited: false,
                        created_entities: vec![],
                        target_folders: vec![],
                        target_paths: vec![],
                        target_nodes: vec![],
                        file_touches: native_collector.file_touches,
                        plan_items: native_collector.plan_items,
                        command_runs: native_collector.command_runs,
                    }
                }
            };
            save(
                directory,
                &evidence_name(&preview),
                &serde_json::to_value(&evidence)?,
                false,
            )?;
            let err_label = receipt["error"].as_str().unwrap_or("");
            let is_schema_err = err_label.contains("schema") || err_label.contains("report");
            let is_deopt = receipt["ok"] != true || attempt > 1;
            let step_provider = if receipt["deterministic"] == true {
                "deterministic"
            } else {
                provider.as_str()
            };
            let run_key = format!(
                "{}:{}:{}",
                string(&state["run_id"]),
                string(&preview["id"]),
                attempt
            );
            let step_model = if is_deterministic_short_circuit {
                None
            } else {
                active_model.as_deref()
            };
            execution::record_system_health(
                &health_db_path,
                "system",
                step_provider,
                step_model,
                &run_key,
                string(&preview["id"]),
                if receipt["ok"] == true {
                    "completed"
                } else if is_schema_err {
                    "schema_error"
                } else {
                    "blocked"
                },
                is_schema_err,
                is_deopt,
                elapsed,
            );
            let health = execution::summarize_system_health(
                &health_db_path,
                "system",
                step_provider,
                step_model,
            );
            merge_health_warnings(state, &health.warnings);
            let finished = object!({
                "type": "dispatch_finished",
                "phase": preview["id"],
                "service": sid,
                "attempt": attempt,
                "ok": receipt["ok"],
                "error": if receipt["rejection"].is_string() {
                    object!(format!("{}: {}", string(&receipt["error"]), string(&receipt["rejection"])))
                } else if receipt["failure"].is_string() {
                    object!(format!("{}: {}", string(&receipt["error"]), string(&receipt["failure"])))
                } else {
                    receipt["error"].clone()
                },
                "duration_seconds": elapsed,
                "report": receipt["report"]
            });
            if receipt["ok"] != true {
                let interrupted =
                    cancel.load(Ordering::SeqCst) || receipt["error"] == "interrupted";
                state["steps"][index]["status"] = if interrupted {
                    "interrupted"
                } else {
                    "blocked"
                }
                .into();
                state["status"] = if interrupted {
                    "interrupted"
                } else {
                    "blocked"
                }
                .into();
                state["error"] = if receipt["failure"].is_string() {
                    object!(match receipt["returncode"].as_i64() {
                        Some(code) => format!(
                            "The {provider} CLI exited with code {code}: {}",
                            string(&receipt["failure"])
                        ),
                        None => format!(
                            "The {provider} CLI could not run: {}",
                            string(&receipt["failure"])
                        ),
                    })
                } else if receipt["error"] == "boundary_violation"
                    && receipt["rejection"].is_string()
                {
                    receipt["rejection"].clone()
                } else {
                    receipt["error"].clone()
                };
                save(directory, "run.json", state, false)?;
                notify(event, finished);
                return Ok(state.clone());
            }
            let refreshed = cached_system.get_or_reload(&system.allow_roots)?;
            plan = check_changes(
                refreshed,
                &plan,
                Some(&preview),
                Some(&receipt["report"]),
                state,
                false,
            )?;
            save(directory, "checkpoint.json", &plan, false)?;
            finish(directory, state, &preview, &receipt, brains)?;
            context_completed(dispatch_context, state, &preview, &receipt)?;
            notify(event, finished);
            if preview["id"] == "contracts" {
                break;
            }
        }
        if state["steps"]
            .as_array()
            .is_some_and(|steps| steps.iter().any(|step| step["status"] != "completed"))
        {
            return Ok(state.clone());
        }
        let reports = reports(directory, state)?;
        // The run's outcome becomes the system's memory by itself; failing to share it never fails a completed run.
        let memory = if untrusted_context {
            object!({
                "status": "skipped",
                "reason": "experimental worker claims stay in receipts and the private journal; no verified Brain outcome or promotion"
            })
        } else {
            match brains.get("__system__") {
                Some(brain) => brain
                    .share_outcome(state, &plan, &reports)
                    .unwrap_or_else(|error| object!({"error": error.message})),
                None => Value::Null,
            }
        };
        let shared = if memory["status"] == "applied" {
            " Its outcome is now the system's memory: later plans share it with every agent."
        } else if memory["review_mode"] == "human" {
            " Its outcome waits as a proposal for the system's memory, since the system folder turns automatic promotion off: approve it with memory promote-review, then promote-apply."
        } else {
            ""
        };
        save(
            directory,
            "handoff.json",
            &object!({
                "schema_version": 1,
                "kind": "ai-system-handoff",
                "run_id": state["run_id"],
                "change_id": state["change_id"],
                "tasks": state["tasks"],
                "reports": reports,
                "memory": memory,
                "receipts": state["steps"].as_array().into_iter().flatten().map(receipt_name).collect::<Vec<_>>(),
                "knowledge": format!(
                    "Review contract decisions and reusable lessons in receipts; publish through each owning runtime's promotion workflow. Raw context is not durable knowledge.{shared} If the change added, versioned or removed a contract or a dependency, update the service passports (Fill with AI or the system editor) before planning the next change: plans route and trace impact only through declared contracts."
                ),
                "meaning": "Worker-reported completion and checks; no commit, merge or deployment is implied."
            }),
            false,
        )?;
        for sid in &ids {
            if !texts(&state["closed_tasks"]).contains(sid) {
                let run = string(&state["run_id"]).to_string();
                brains
                    .get(sid)
                    .ok_or_else(|| fail("Missing native Brain"))?
                    .complete(reference_mut(state, sid)?, &run)?;
                state["closed_tasks"]
                    .as_array_mut()
                    .ok_or_else(|| fail("Closed tasks missing"))?
                    .push(sid.clone().into());
                save(directory, "run.json", state, false)?;
            }
        }
        state["status"] = "completed".into();
        state["finished_at"] = now().into();
        save(directory, "run.json", state, false)?;
        Ok(state.clone())
    })
}

#[inline(never)]
#[expect(
    clippy::too_many_arguments,
    reason = "Compatibility API keeps explicit per-call custody and process controls."
)]
fn execute_run_context_boxed<'a>(
    system: &'a mut System,
    directory: &'a Path,
    retry: Option<&'a str>,
    accept: Acceptance,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<std::fs::File>>,
    adapter: Option<Arc<dyn infra_host::provider::LaunchAdapter>>,
    observer: Option<Arc<dyn infra_host::process::RunObserver>>,
    dispatch_context: Option<SharedDispatchContext>,
    hooks: &'a RunnerGraphHooks,
    event: &'a mut (dyn FnMut(Value) -> Result<()> + Send),
) -> BoxFuture<'a, Result<Value>> {
    Box::pin(execute_run_context(
        system,
        directory,
        retry,
        accept,
        cancel,
        runner_lock,
        adapter,
        observer,
        dispatch_context,
        hooks,
        event,
    ))
}

#[expect(
    clippy::too_many_arguments,
    reason = "Compatibility API keeps explicit per-call custody and process controls."
)]
async fn execute_run_context(
    system: &mut System,
    directory: &Path,
    retry: Option<&str>,
    accept: Acceptance,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<std::fs::File>>,
    adapter: Option<Arc<dyn infra_host::provider::LaunchAdapter>>,
    observer: Option<Arc<dyn infra_host::process::RunObserver>>,
    dispatch_context: Option<SharedDispatchContext>,
    hooks: &RunnerGraphHooks,
    event: &mut (dyn FnMut(Value) -> Result<()> + Send),
) -> Result<Value> {
    let directory = absolute(directory)?;
    let _run_lock = execution::lock(&directory, ".run.lock")?;
    let initial = match validate_state_boxed(system, &directory, true).await {
        Ok(state) => state,
        Err(error) => {
            // A CLI that went missing or changed (a self-updating CLI) stops
            // the run in its journal too, not only on the terminal that
            // resumed it: the run view showed such a run as interrupted with
            // no cause.
            if let Ok(mut state) = validate_state_boxed(system, &directory, false).await
                && state["status"] != "completed"
            {
                state["status"] = "blocked".into();
                state["error"] = error.message.clone().into();
                let _ = save(&directory, "run.json", &state, false);
            }
            return Err(error);
        }
    };
    let _workspaces = execution::workspace_locks(system, &texts(&initial["selected"]))?;
    let mut state = validate_state_boxed(system, &directory, true).await?;
    if state["status"] == "completed" {
        return Ok(state);
    }
    let untrusted_context = experimental_context(&directory, &state, dispatch_context.is_some())?;
    let owner = format!("ai-system-{}", string(&state["run_id"]));
    let mut brains = BTreeMap::new();
    for (sid, root) in state["roots"]
        .as_object()
        .ok_or_else(|| fail("Missing run roots"))?
    {
        brains.insert(sid.clone(), Brain::new(Path::new(string(root)), &owner)?);
    }
    let plan = read_json(&directory, "checkpoint.json", execution::MAX_STATE)?;
    system.selected_plan(&plan, true)?;
    let result = execute_run_steps_boxed(RunStepsArgs {
        system,
        directory: &directory,
        retry,
        accept,
        cancel: &cancel,
        runner_lock,
        adapter,
        observer,
        dispatch_context: &dispatch_context,
        hooks,
        untrusted_context,
        brains: &brains,
        state: &mut state,
        plan,
        event,
    })
    .await;
    match result {
        Ok(state) => Ok(state),
        Err(error) => {
            let interrupted =
                cancel.load(Ordering::SeqCst) || error.code.as_deref() == Some("interrupted");
            state["status"] = if interrupted {
                "interrupted"
            } else {
                "blocked"
            }
            .into();
            state["error"] = error.to_string().into();
            if interrupted {
                for step in state["steps"].as_array_mut().into_iter().flatten() {
                    if step["status"] == "running" {
                        step["status"] = "interrupted".into();
                        notify(
                            event,
                            object!({"type":"dispatch_finished","phase":step["id"],"service":step["service"],"attempt":step["attempt"],"ok":false,"error":"interrupted","interrupted":true}),
                        );
                    }
                }
            }
            save(&directory, "run.json", &state, false)?;
            if interrupted { Ok(state) } else { Err(error) }
        }
    }
}

pub async fn execute_run_locked(
    system: &mut System,
    directory: &Path,
    retry: Option<&str>,
    accept: bool,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<std::fs::File>>,
    event: &mut (impl FnMut(Value) -> Result<()> + Send),
) -> Result<Value> {
    let hooks = RunnerGraphHooks::default();
    execute_run_context_boxed(
        system,
        directory,
        retry,
        Acceptance::sources(accept),
        cancel,
        runner_lock,
        None,
        None,
        None,
        &hooks,
        event,
    )
    .await
}
pub async fn execute_run(
    system: &mut System,
    directory: &Path,
    retry: Option<&str>,
    accept: bool,
    cancel: Arc<AtomicBool>,
    event: &mut (impl FnMut(Value) -> Result<()> + Send),
) -> Result<Value> {
    execute_run_locked(system, directory, retry, accept, cancel, None, event).await
}
#[expect(
    clippy::too_many_arguments,
    reason = "Compatibility API keeps explicit per-call custody and process controls."
)]
pub(crate) async fn drive_run_context(
    system: &mut System,
    directory: &Path,
    retry: Option<&str>,
    accept: Acceptance,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<std::fs::File>>,
    adapter: Option<Arc<dyn infra_host::provider::LaunchAdapter>>,
    observer: Option<Arc<dyn infra_host::process::RunObserver>>,
    dispatch_context: Option<SharedDispatchContext>,
    event: &mut (impl FnMut(Value) -> Result<()> + Send),
) -> Result<Value> {
    drive_run_context_with_hooks(
        system,
        directory,
        retry,
        accept,
        cancel,
        runner_lock,
        adapter,
        observer,
        dispatch_context,
        RunnerGraphHooks::default(),
        event,
    )
    .await
}

#[expect(
    clippy::too_many_arguments,
    reason = "Internal driver coordinates both contract and service passes with optional graph hooks."
)]
pub(crate) async fn drive_run_context_with_hooks(
    system: &mut System,
    directory: &Path,
    retry: Option<&str>,
    accept: Acceptance,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<std::fs::File>>,
    adapter: Option<Arc<dyn infra_host::provider::LaunchAdapter>>,
    observer: Option<Arc<dyn infra_host::process::RunObserver>>,
    dispatch_context: Option<SharedDispatchContext>,
    hooks: RunnerGraphHooks,
    event: &mut (impl FnMut(Value) -> Result<()> + Send),
) -> Result<Value> {
    if let Some(bridge) = &hooks.bridge
        && let Ok(abs_dir) = absolute(directory)
        && let Ok(initial_state) = read_json(&abs_dir, "run.json", execution::MAX_STATE)
    {
        let outer_id = hooks
            .system_run_id
            .clone()
            .unwrap_or_else(|| string(&initial_state["run_id"]).to_string());
        let _ = bridge.on_run_start(&outer_id, &initial_state, system);
    }

    let first = execute_run_context_boxed(
        system,
        directory,
        retry,
        accept,
        cancel.clone(),
        runner_lock.clone(),
        adapter.clone(),
        observer.clone(),
        dispatch_context.clone(),
        &hooks,
        event,
    )
    .await;

    let final_result = match first {
        Ok(state)
            if state["status"] == "running"
                && state["steps"]
                    .as_array()
                    .is_some_and(|steps| steps.iter().all(|step| step["status"] != "running")) =>
        {
            execute_run_context_boxed(
                system,
                directory,
                None,
                Acceptance::default(),
                cancel,
                runner_lock,
                adapter,
                observer,
                dispatch_context,
                &hooks,
                event,
            )
            .await
        }
        other => other,
    };

    if let Some(bridge) = &hooks.bridge {
        let state_snapshot = match &final_result {
            Ok(state) => Some(state.clone()),
            Err(_) => absolute(directory)
                .ok()
                .and_then(|d| read_json(&d, "run.json", execution::MAX_STATE).ok()),
        };
        if let Some(state_val) = state_snapshot {
            let outer_id = hooks
                .system_run_id
                .clone()
                .unwrap_or_else(|| string(&state_val["run_id"]).to_string());
            let _ = bridge.on_run_finish(&outer_id, &state_val, system);
        }
    }

    final_result
}

#[expect(
    clippy::too_many_arguments,
    reason = "Public bridge entry point keeps explicit custody and graph hooks."
)]
pub async fn drive_run_with_bridge(
    system: &mut System,
    directory: &Path,
    retry: Option<&str>,
    accept: Acceptance,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<std::fs::File>>,
    adapter: Option<Arc<dyn infra_host::provider::LaunchAdapter>>,
    observer: Option<Arc<dyn infra_host::process::RunObserver>>,
    bridge: Arc<dyn CodeGraphBridge>,
    launch_id: Option<String>,
    event: &mut (impl FnMut(Value) -> Result<()> + Send),
) -> Result<Value> {
    drive_run_context_with_hooks(
        system,
        directory,
        retry,
        accept,
        cancel,
        runner_lock,
        adapter,
        observer,
        None,
        RunnerGraphHooks {
            bridge: Some(bridge),
            system_run_id: None,
            launch_id,
            approvals: None,
        },
        event,
    )
    .await
}
pub async fn drive_run_locked(
    system: &mut System,
    directory: &Path,
    retry: Option<&str>,
    accept: bool,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<std::fs::File>>,
    event: &mut (impl FnMut(Value) -> Result<()> + Send),
) -> Result<Value> {
    drive_run_context(
        system,
        directory,
        retry,
        Acceptance::sources(accept),
        cancel,
        runner_lock,
        None,
        None,
        None,
        event,
    )
    .await
}
pub async fn drive_run(
    system: &mut System,
    directory: &Path,
    retry: Option<&str>,
    accept: Acceptance,
    cancel: Arc<AtomicBool>,
    event: &mut (impl FnMut(Value) -> Result<()> + Send),
) -> Result<Value> {
    drive_run_context(
        system, directory, retry, accept, cancel, None, None, None, None, event,
    )
    .await
}
#[expect(
    clippy::too_many_arguments,
    reason = "Compatibility API keeps explicit per-call custody and process controls."
)]
pub async fn drive_run_with_adapter(
    system: &mut System,
    directory: &Path,
    retry: Option<&str>,
    accept: bool,
    cancel: Arc<AtomicBool>,
    runner_lock: Option<Arc<std::fs::File>>,
    adapter: Arc<dyn infra_host::provider::LaunchAdapter>,
    observer: Arc<dyn infra_host::process::RunObserver>,
    event: &mut (impl FnMut(Value) -> Result<()> + Send),
) -> Result<Value> {
    drive_run_context(
        system,
        directory,
        retry,
        Acceptance::sources(accept),
        cancel,
        runner_lock,
        Some(adapter),
        Some(observer),
        None,
        event,
    )
    .await
}

/// Maintainer experiment entry point; all ordinary wrappers pass no context policy.
pub async fn drive_run_with_context(
    system: &mut System,
    directory: &Path,
    cancel: Arc<AtomicBool>,
    context: SharedDispatchContext,
    adapter: Option<Arc<dyn infra_host::provider::LaunchAdapter>>,
    event: &mut (impl FnMut(Value) -> Result<()> + Send),
) -> Result<Value> {
    drive_run_context(
        system,
        directory,
        None,
        Acceptance::default(),
        cancel,
        None,
        adapter,
        None,
        Some(context),
        event,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system_graph::ServiceGraphOffer;
    use infra_host::{
        process::RunObserver,
        provider::{Launch, LaunchAdapter, PreparedCommand, RunEvents, command_with_adapter},
    };

    #[test]
    fn system_launch_adapter_applies_transport_appropriate_flags_and_worker_offers() {
        let project = std::env::temp_dir();
        let schema_path = project.join("result-schema.json");
        let schema = execution::result_schema(&["orders".into()]);
        let offer = ServiceGraphOffer {
            service: "orders".into(),
            command: "graphify".into(),
            graph_path: project.join("orders-graph/graph.json"),
            nodes: 3,
            edges: 2,
            commit: None,
            readable_paths: vec![project.join("orders-graph/graph.json")],
            claude_allow_rules: vec![],
            env: BTreeMap::from([("EXTRA_GRAPH_ENV".into(), "ok".into())]),
            prompt_instructions: None,
        };

        struct RecordingInner;
        impl LaunchAdapter for RecordingInner {
            fn adapt(
                &self,
                _input: &Launch,
                mut command: PreparedCommand,
            ) -> infra_host::Result<PreparedCommand> {
                command
                    .environment
                    .insert("INNER_ADAPTED".into(), "yes".into());
                Ok(command)
            }
        }

        let adapter = SystemLaunchAdapter {
            inner: Some(Arc::new(RecordingInner)),
            schema,
            schema_path: schema_path.clone(),
            extra: vec![project.join("billing")],
            read: vec![project.join("sys")],
            offers: vec![offer],
        };

        // Codex Manual (`CodexServer`): skips `exec`-only flags (`--skip-git-repo-check`, `--add-dir`, `--output-schema`, `-`).
        let codex_manual = Launch {
            provider: "codex".into(),
            executable: PathBuf::from("codex"),
            project: project.clone(),
            prompt: "Run service step".into(),
            mode: "edit".into(),
            model: None,
            session: None,
            agents: false,
            count: 1,
            effort: None,
            usd: None,
            hook_events: false,
            permissions: Some(PermissionMode::Manual),
            approvals: None,
        };
        let prepared = command_with_adapter(&codex_manual, &adapter).unwrap();
        assert_eq!(prepared.argv.last().map(String::as_str), Some("app-server"));
        assert!(!prepared.argv.iter().any(|a| a == "--skip-git-repo-check"));
        assert!(!prepared.argv.iter().any(|a| a == "--add-dir"));
        assert!(!prepared.argv.iter().any(|a| a == "--output-schema"));
        assert!(!prepared.argv.iter().any(|a| a == "-"));
        assert_eq!(
            prepared.environment.get("GRAPHIFY_NO_AUTO_REFRESH"),
            Some(&"1".to_string())
        );
        assert_eq!(
            prepared.environment.get("INNER_ADAPTED"),
            Some(&"yes".to_string())
        );
        assert!(
            prepared
                .readable
                .contains(&project.join("orders-graph/graph.json"))
        );

        // Codex Auto (`Text`): keeps `--skip-git-repo-check`, `--add-dir`, `--output-schema`, `-`.
        let codex_auto = Launch {
            permissions: Some(PermissionMode::Auto),
            ..codex_manual
        };
        let prepared_auto = command_with_adapter(&codex_auto, &adapter).unwrap();
        assert!(
            prepared_auto
                .argv
                .iter()
                .any(|a| a == "--skip-git-repo-check")
        );
        assert!(prepared_auto.argv.iter().any(|a| a == "--add-dir"));
        assert!(prepared_auto.argv.iter().any(|a| a == "--output-schema"));
        assert_eq!(prepared_auto.argv.last().map(String::as_str), Some("-"));

        // Cursor Manual (`CursorAcp`): skips Cursor CLI prompt byte limit; Cursor Auto (`Text`) enforces it.
        let oversized_prompt = "x".repeat(native::CURSOR_PROMPT_BYTES + 200);
        let cursor_adapter = SystemLaunchAdapter {
            extra: vec![],
            ..adapter
        };
        let cursor_manual = Launch {
            provider: "cursor".into(),
            executable: PathBuf::from("cursor-agent"),
            project: project.clone(),
            prompt: oversized_prompt.clone(),
            mode: "edit".into(),
            model: None,
            session: None,
            agents: false,
            count: 1,
            effort: None,
            usd: None,
            hook_events: false,
            permissions: Some(PermissionMode::Manual),
            approvals: None,
        };
        let cursor_prepared = command_with_adapter(&cursor_manual, &cursor_adapter).unwrap();
        assert_eq!(cursor_prepared.argv.last().map(String::as_str), Some("acp"));

        let cursor_auto = Launch {
            permissions: Some(PermissionMode::Auto),
            ..cursor_manual
        };
        assert!(command_with_adapter(&cursor_auto, &cursor_adapter).is_err());
    }

    #[test]
    fn system_run_events_collects_raw_lines_for_worker_result_evidence_and_bridge() {
        #[derive(Default)]
        struct SignalBridge {
            signals: Mutex<Vec<String>>,
        }
        impl CodeGraphBridge for SignalBridge {
            fn on_dispatch_signal(
                &self,
                _ctx: &DispatchGraphContext,
                raw_line: &[u8],
            ) -> Result<()> {
                self.signals
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(raw_line).to_string());
                Ok(())
            }
        }
        struct RecordingObserver(Mutex<Vec<String>>);
        impl RunObserver for RecordingObserver {
            fn boundary(&self, signal: &str, _details: &Value) -> infra_host::Result<()> {
                self.0.lock().unwrap().push(signal.to_string());
                Ok(())
            }
        }

        let report = object!({
            "status": "completed",
            "summary": "Implemented",
            "checks": [{"name": "unit", "status": "passed", "detail": "ok"}],
            "changed_files": ["src/a.php"],
            "service_order": []
        });
        let report_str = encoded(&report).unwrap();

        for (provider, events) in [
            (
                "claude",
                vec![object!({
                    "type": "result",
                    "subtype": "success",
                    "is_error": false,
                    "structured_output": report.clone()
                })],
            ),
            (
                "codex",
                vec![
                    object!({
                        "type": "item.completed",
                        "item": {"id": "1", "type": "command_execution", "command": "cat src/a.php", "status": "completed"}
                    }),
                    object!({
                        "type": "item.completed",
                        "item": {"id": "2", "type": "agent_message", "text": report_str.clone()}
                    }),
                    object!({
                        "type": "turn.completed",
                        "usage": {"input_tokens": 10, "output_tokens": 5}
                    }),
                ],
            ),
            (
                "cursor",
                vec![object!({
                    "type": "result",
                    "subtype": "success",
                    "is_error": false,
                    "result": report_str
                })],
            ),
        ] {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let bridge = Arc::new(SignalBridge::default());
            let observer = Arc::new(RecordingObserver(Mutex::new(Vec::new())));
            let run_events = SystemRunEvents {
                observer: Some(observer.clone()),
                collector: Mutex::new(NativeEvidenceCollector::default()),
                raw_lines: Mutex::new(Vec::new()),
                bridge: Some(bridge.clone()),
                graph_ctx: DispatchGraphContext {
                    run_id: "r1".into(),
                    journal_run_id: "r1".into(),
                    launch_id: None,
                    execution_dir: PathBuf::from("/tmp/run"),
                    system_file: PathBuf::from("/tmp/sys/system.json"),
                    provider: provider.into(),
                    mode: "edit".into(),
                    access: "service".into(),
                    tell_agents_graph: Some(true),
                    phase: "service-orders".into(),
                    dispatch_id: "d1".into(),
                    attempt: 1,
                    service: "orders".into(),
                    root: PathBuf::from("/tmp/orders"),
                    roots: BTreeMap::new(),
                    selected: vec!["orders".into()],
                    readable: vec![],
                    writable: vec![],
                    base_head: None,
                    head_after: None,
                    plan: object!({}),
                    state: object!({}),
                },
                provider: provider.into(),
                root: PathBuf::from("/tmp/orders"),
                phase: "service-orders".into(),
                attempt: 1,
                tx: Mutex::new(Some(tx)),
            };
            run_events
                .boundary("guardian_spawned", &object!({}))
                .unwrap();
            for raw in &events {
                run_events.observe(Some(raw), None).unwrap();
            }
            run_events.close();

            let raw_bytes = run_events.take_raw_bytes();
            let parsed = native::worker_result(&raw_bytes, provider).unwrap();
            assert_eq!(parsed, report, "{provider}");
            assert_eq!(bridge.signals.lock().unwrap().len(), events.len());
            assert_eq!(
                observer.0.lock().unwrap().as_slice(),
                &["guardian_spawned".to_string()]
            );
            let mut emitted = Vec::new();
            while let Ok(ev) = rx.try_recv() {
                emitted.push(ev);
            }
            assert_eq!(emitted.len(), events.len());
            assert!(emitted.iter().all(|e| e["type"] == "output"));
            if provider == "codex" {
                let collector = run_events.take_collector();
                assert_eq!(collector.commands, 1);
            }
        }
    }

    fn write_exec_script(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(path, perms).unwrap();
        }
    }

    #[test]
    fn cached_system_avoids_reloads_when_clean_and_reloads_on_manifest_drift() {
        let dir =
            std::env::temp_dir().join(format!("infra-runner-cached-sys-{}", uuid::Uuid::new_v4()));
        let sys_dir = dir.join("sys");
        let orders_dir = dir.join("orders");
        std::fs::create_dir_all(&sys_dir).unwrap();
        std::fs::create_dir_all(&orders_dir).unwrap();
        std::fs::write(sys_dir.join("AGENTS.md"), "# Policy\n").unwrap();
        std::fs::write(
            sys_dir.join("system.json"),
            serde_json::to_string_pretty(&object!({
                "schema_version": 1,
                "name": "cache-sys",
                "services": [{"id": "orders", "root": "../orders", "manifest": "ai-service.json"}],
                "shared_sources": [{"kind": "policy", "path": "AGENTS.md"}]
            }))
            .unwrap(),
        )
        .unwrap();
        let make_manifest = |desc: &str| {
            serde_json::to_string_pretty(&object!({
                "schema_version": 1,
                "id": "orders",
                "description": desc,
                "owner": "@team",
                "capabilities": [{"id": "orders", "description": desc, "status": "implemented", "sources": []}],
                "provides": [],
                "consumes": [],
                "sources": []
            }))
            .unwrap()
        };
        std::fs::write(
            orders_dir.join("ai-service.json"),
            make_manifest("orders-v1"),
        )
        .unwrap();

        let system = System::new(&sys_dir.join("system.json"), std::slice::from_ref(&dir)).unwrap();
        let mut cached = CachedSystem::from_system(&system);
        let _ = cached.get_or_reload(std::slice::from_ref(&dir)).unwrap();
        let _ = cached.get_or_reload(std::slice::from_ref(&dir)).unwrap();
        assert_eq!(cached.reload_count, 0);

        // Mutate service manifest inside RACY_NS with exact same byte length -> triggers reload.
        std::fs::write(
            orders_dir.join("ai-service.json"),
            make_manifest("orders-v2"),
        )
        .unwrap();
        let desc = {
            let reloaded = cached.get_or_reload(std::slice::from_ref(&dir)).unwrap();
            reloaded.services["orders"].manifest.as_ref().unwrap()["description"]
                .as_str()
                .map(str::to_owned)
        };
        assert_eq!(cached.reload_count, 1);
        assert_eq!(desc.as_deref(), Some("orders-v2"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn zero_token_verify_respects_mode_and_prelaunch_health_clamps_and_switches_model() {
        let dir = std::env::temp_dir().join(format!("infra-runner-wave3-{}", uuid::Uuid::new_v4()));
        let sys_dir = dir.join("sys");
        let orders_dir = dir.join("orders");
        std::fs::create_dir_all(orders_dir.join("project-brain")).unwrap();
        std::fs::create_dir_all(&sys_dir).unwrap();
        std::fs::write(sys_dir.join("AGENTS.md"), "# Policy\n").unwrap();
        std::fs::write(
            sys_dir.join("system.json"),
            serde_json::to_string_pretty(&object!({
                "schema_version": 1,
                "name": "runner-w3-sys",
                "services": [{"id": "orders", "root": "../orders", "manifest": "ai-service.json"}],
                "shared_sources": [{"kind": "policy", "path": "AGENTS.md"}]
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            orders_dir.join("ai-service.json"),
            serde_json::to_string_pretty(&object!({
                "schema_version": 1,
                "id": "orders",
                "description": "orders",
                "owner": "@team",
                "capabilities": [{"id": "orders", "description": "orders", "status": "implemented", "sources": []}],
                "provides": [],
                "consumes": [],
                "sources": []
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            orders_dir.join("package.json"),
            r#"{"name":"orders","scripts":{"test":"phpunit"}}"#,
        )
        .unwrap();

        let fake_cli = dir.join("fake-claude.sh");
        let verify_log = dir.join("verify-llm.log");
        let script = format!(
            r#"#!/bin/sh
prompt=$(cat)
case "$prompt" in
  *'"phase":"verify"'*)
    echo "verify" >> "{vlog}"
    report='{{"status":"completed","summary":"Verify LLM","checks":[{{"name":"v","status":"passed","detail":"ok"}}],"changed_files":[],"service_order":[]}}'
    ;;
  *'"phase":"contracts"'*)
    report='{{"status":"completed","summary":"Contracts","checks":[{{"name":"c","status":"passed","detail":"ok"}}],"changed_files":[],"service_order":["orders"]}}'
    ;;
  *)
    report='{{"status":"completed","summary":"Orders","checks":[{{"name":"o","status":"passed","detail":"ok"}}],"changed_files":[],"service_order":[]}}'
    ;;
esac
echo '{{"type":"system","subtype":"init","session_id":"s1"}}'
printf '{{"type":"result","subtype":"success","is_error":false,"result":"ok","structured_output":%s,"session_id":"s1"}}\n' "$report"
"#,
            vlog = verify_log.display()
        );
        write_exec_script(&fake_cli, &script);
        let cli_str = fake_cli.to_string_lossy().into_owned();

        let mut system =
            System::new(&sys_dir.join("system.json"), std::slice::from_ref(&dir)).unwrap();
        let plan = system
            .plan("Check orders", "chg-w3", &["orders".into()], &[], 8000, 4)
            .unwrap();
        // Planner marks safe deterministic verify as require_llm_review: false.
        let planned_verify = plan["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == "verify")
            .unwrap();
        assert_eq!(planned_verify["require_llm_review"], false);

        // 1. Mode / mutating step gating on 0-token verify short-circuit.
        let ro_state = object!({
            "mode": "read-only",
            "steps": [{"id": "contracts", "mode": "read-only"}, {"id": "verify", "mode": "read-only"}],
        });
        assert!(!should_require_verify_llm_review(
            Some(planned_verify),
            &ro_state,
            &[],
        ));
        let edit_state = object!({
            "mode": "edit",
            "steps": [{"id": "contracts", "mode": "read-only"}, {"id": "verify", "mode": "read-only"}],
        });
        assert!(should_require_verify_llm_review(
            Some(planned_verify),
            &edit_state,
            &[],
        ));
        let mutated_prior = vec![object!({"changed_files": ["src/Order.php"]})];
        assert!(should_require_verify_llm_review(
            Some(planned_verify),
            &ro_state,
            &mutated_prior,
        ));
        let explicit_require = object!({"require_llm_review": true});
        assert!(should_require_verify_llm_review(
            Some(&explicit_require),
            &ro_state,
            &[],
        ));
        let det_only = object!({"require_llm_review": false, "deterministic_only": true});
        assert!(!should_require_verify_llm_review(
            Some(&det_only),
            &edit_state,
            &mutated_prior,
        ));

        // 2. Pre-launch health check clamps attempts before spawning and proactively selects healthier model.
        std::fs::create_dir_all(dir.join("runs")).unwrap();
        let health_db = dir.join("runs/system_health.sqlite3");
        for i in 0..4 {
            execution::record_system_health(
                &health_db,
                "system",
                "claude",
                None,
                &format!("bad-{i}"),
                "verify",
                "schema_error",
                true,
                true,
                1.0,
            );
            execution::record_system_health(
                &health_db,
                "system",
                "claude",
                Some("claude-healthy"),
                &format!("good-{i}"),
                "verify",
                "completed",
                false,
                false,
                0.2,
            );
        }
        let clamp_dir = dir.join("runs/clamp-run");
        native::create_run(
            &mut system,
            &plan,
            &clamp_dir,
            "claude",
            Some(&cli_str),
            "read-only",
            30,
            "service",
        )
        .await
        .unwrap();
        let mut state = read_json(&clamp_dir, "run.json", MAX_BYTES).unwrap();
        state["steps"][0]["attempt"] = object!(2u64);
        state["steps"][0]["dispatch_id"] = object!("0123456789abcdef0123456789abcdef");
        state["steps"][0]["input_sha256"] = object!(digest(b"prior-attempt"));
        save(&clamp_dir, "run.json", &state, false).unwrap();
        let mut clamp_events = Vec::new();
        let err = execute_run_locked(
            &mut system,
            &clamp_dir,
            None,
            false,
            Arc::new(AtomicBool::new(false)),
            None,
            &mut |ev| {
                clamp_events.push(ev);
                Ok(())
            },
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("Dispatch retry limit reached"),
            "{err}"
        );
        let health_ev = clamp_events
            .iter()
            .find(|ev| ev["type"] == "system_health")
            .expect("expected prelaunch system_health event");
        assert_eq!(health_ev["selected_model"], "claude-healthy");
        assert_eq!(health_ev["recommended_max_attempts"], 2);
        let saved_state = read_json(&clamp_dir, "run.json", MAX_BYTES).unwrap();
        assert!(
            saved_state["health_warnings"]
                .as_array()
                .is_some_and(|w| !w.is_empty()),
            "{saved_state}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
