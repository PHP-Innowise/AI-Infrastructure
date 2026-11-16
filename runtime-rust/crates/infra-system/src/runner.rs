use crate::{
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
use serde_json::{Value, json as object};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

/// Optional code-graph bridge and launch identity hooks threaded through a system run.
#[derive(Clone, Default)]
pub struct RunnerGraphHooks {
    pub bridge: Option<Arc<dyn CodeGraphBridge>>,
    pub system_run_id: Option<String>,
    pub launch_id: Option<String>,
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

fn notify(event: &mut impl FnMut(Value) -> Result<()>, value: Value) {
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
    event: &mut (impl FnMut(Value) -> Result<()> + Send),
) -> Result<Value> {
    let directory = absolute(directory)?;
    let _run_lock = execution::lock(&directory, ".run.lock")?;
    let initial = match native::validate_state(system, &directory, true).await {
        Ok(state) => state,
        Err(error) => {
            // A CLI that went missing or changed (a self-updating CLI) stops
            // the run in its journal too, not only on the terminal that
            // resumed it: the run view showed such a run as interrupted with
            // no cause.
            if let Ok(mut state) = native::validate_state(system, &directory, false).await
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
    let mut state = native::validate_state(system, &directory, true).await?;
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
    let mut plan = read_json(&directory, "checkpoint.json", execution::MAX_STATE)?;
    system.selected_plan(&plan, true)?;
    let mut retry = retry.map(str::to_string);
    let result = Box::pin(async {
 let blocked=state["steps"].as_array().into_iter().flatten().find(|step|["running","blocked","interrupted"].contains(&string(&step["status"]))).cloned();
 if let Some(step)=blocked{let receipt=match dispatch_receipt(&directory,&state,&step){Ok(value)=>Some(value),Err(error)if error.kind==ErrorKind::NotFound=>None,Err(error)=>return Err(error)};
 let unrun=receipt.as_ref().and_then(|receipt|execution::unrun_checks(receipt,&step));
 if accept.unrun_checks&&unrun.is_none(){return Err(fail(format!("Only a completed verification whose only gaps are checks it could not run can be accepted; retry {id} instead (CLI: --retry-step {id})",id=string(&step["id"]))));}
 if accept.unrun_checks&&retry.is_some(){return Err(fail("Accept the checks verification could not run, or retry verification, not both"));}
 if receipt.as_ref().is_some_and(|receipt|receipt["ok"]==true)||accept.unrun_checks{let receipt=receipt.ok_or_else(||fail("Receipt disappeared"))?;if system.verify(&plan)?["fresh"]!=true{if !accept.source_changes{return Err(fail("Successful receipt has uncheckpointed edits; inspect and explicitly accept source changes"));}plan=check_changes(system,&plan,Some(&step),Some(&receipt["report"]),&state,true)?;save(&directory,"checkpoint.json",&plan,false)?;}
 if let Some(unrun)=unrun.filter(|_|accept.unrun_checks){let index=step_index(&state,string(&step["id"]))?;state["steps"][index]["accepted_unrun_checks"]=object!(unrun);}finish(&directory,&mut state,&step,&receipt,&brains)?;context_completed(&dispatch_context,&state,&step,&receipt)?;retry=None;}
 else{if retry.as_deref()!=Some(string(&step["id"])){return Err(fail(format!("Dispatch {id} stopped without a successful receipt and may have changed files; inspect them, then retry {id} explicitly (CLI: --retry-step {id})",id=string(&step["id"]))));}
 if system.verify(&plan)?["fresh"]!=true{if !accept.source_changes{return Err(fail("Partial edits detected; inspect and explicitly accept source changes"));}plan=check_changes(system,&plan,Some(&step),None,&state,true)?;save(&directory,"checkpoint.json",&plan,false)?;}
 let index=step_index(&state,string(&step["id"]))?;state["steps"][index]["status"]="pending".into();}}
 else if retry.is_some(){return Err(fail("No unfinished dispatch matches the dispatch chosen for retry"));}
 else if accept.unrun_checks{return Err(fail("No stopped verification has checks to accept"));}
 // Between dispatches only a person's accepted system-folder edit is adopted;
 // any other change is named and needs a new plan.
 if system.verify(&plan)?["fresh"]!=true{plan=check_changes(system,&plan,None,None,&state,accept.source_changes)?;save(&directory,"checkpoint.json",&plan,false)?;}plan=system.refresh_checkpoint(&plan,&BTreeSet::new())?;
 let ids=brains.keys().cloned().collect::<Vec<_>>();for sid in &ids{let brain=brains.get(sid).ok_or_else(||fail("Missing Brain"))?;let record=brain.ensure(reference_mut(&mut state,sid)?)?;if record["status"]=="completed"&&!texts(&state["closed_tasks"]).contains(sid){if state["steps"].as_array().is_some_and(|steps|steps.iter().any(|step|step["status"]!="completed")){return Err(fail("Native task terminal before worker verification"));}let run=string(&state["run_id"]).to_string();brain.complete(reference_mut(&mut state,sid)?,&run)?;state["closed_tasks"].as_array_mut().ok_or_else(||fail("Closed tasks missing"))?.push(sid.clone().into());}save(&directory,"run.json",&state,false)?;}
 state["status"]="running".into();state["error"]=Value::Null;save(&directory,"run.json",&state,false)?;
 let steps=state["steps"].as_array().cloned().ok_or_else(||fail("Missing steps"))?;
 for step in steps{if step["status"]=="completed"{continue;}
 if cancel.load(Ordering::SeqCst){return Err(RuntimeError::coded(ErrorKind::Unavailable,"interrupted","interrupted"));}
 let mut current=System::new(&system.config,&system.allow_roots)?;let verdict=current.verify(&plan)?;if verdict["fresh"]!=true{
 // Memory that stopped holding between dispatches leaves the shared context; any other change needs a new plan.
 if verdict["changed"].as_array().is_some_and(|changed|changed.iter().all(|change|change["reason"]=="memory_ineligible")){plan=current.refresh_checkpoint(&plan,&BTreeSet::new())?;save(&directory,"checkpoint.json",&plan,false)?;}else{check_changes(&mut current,&plan,None,None,&state,false)?;return Err(fail("Sources changed between dispatches; prepare new plan"));}}
 let prior=reports(&directory,&state)?;let attempt=step["attempt"].as_u64().filter(|attempt|*attempt<100).ok_or_else(||fail("Dispatch retry limit reached"))?+1;let mut preview=step.clone();preview["dispatch_id"]=id32().into();preview["status"]="running".into();preview["attempt"]=object!(attempt);
 let sid=string(&step["service"]).to_string();let provider=string(&state["provider"]).to_string();let executable=PathBuf::from(string(&state["executable"]));let scratch=native::read_only_scratch(&provider,&executable,string(&step["mode"]),&directory)?;
 let access=dispatch_access(&state,&step);let root=PathBuf::from(string(&state["roots"][&sid]));let schema=execution::result_schema(&texts(&state["selected"]));
 let roots_map=state["roots"].as_object().ok_or_else(||fail("Missing run roots"))?.iter().map(|(k,v)|(k.clone(),PathBuf::from(string(v)))).collect::<BTreeMap<_,_>>();
 let base_head=git_head(&root);
 let pre_existing_dirty:BTreeSet<String>=if access_scope(&state)=="all"&&step["mode"]=="edit"{texts(&state["selected"]).into_iter().flat_map(|s|{let sr=Path::new(string(&state["roots"][&s]));git_worktree_changes(sr).into_iter().map(move|c|format!("{s}/{}",c.path))}).collect()}else if step["mode"]=="edit"{git_worktree_changes(&root).into_iter().map(|c|c.path).collect()}else{BTreeSet::new()};
 let journal_run_id=string(&state["run_id"]).to_string();
 let outer_run_id=hooks.system_run_id.clone().unwrap_or_else(||journal_run_id.clone());
 let mut graph_ctx=DispatchGraphContext{run_id:outer_run_id,journal_run_id,launch_id:hooks.launch_id.clone(),execution_dir:directory.clone(),system_file:system.config.clone(),provider:provider.clone(),mode:string(&preview["mode"]).to_string(),access:access_scope(&state).to_string(),tell_agents_graph:state.get("tell_agents_graph").and_then(Value::as_bool),phase:string(&preview["id"]).to_string(),dispatch_id:string(&preview["dispatch_id"]).to_string(),attempt,service:sid.clone(),root:root.clone(),roots:roots_map,selected:texts(&state["selected"]),readable:texts(&access["readable"]),writable:texts(&access["writable"]),base_head:base_head.clone(),head_after:None,plan:plan.clone(),state:state.clone()};
 let raw_offers=match &hooks.bridge{Some(bridge)=>bridge.offers_for_dispatch(&graph_ctx)?,None=>vec![]};
 let step_offers=crate::workers::offers_for_step(&state,&preview,&raw_offers);
 let prompt=match &dispatch_context {Some(context)=>context.lock().map_err(|_|fail("Dispatch context lock poisoned"))?.prompt(&state,&plan,&preview,&prior,scratch.as_deref())?,None=>native::prompt_for_with_graphs(&state,&plan,&preview,&prior,scratch.as_deref(),&step_offers)?};
 let prepared=native::prepare_launch(&provider,&executable,&root,&prompt,&schema,&directory.join("result-schema.json"),string(&step["mode"]),&execution::extra_directories(&state,&step,&directory),&execution::read_directories(&state,&step,&directory),adapter.as_deref())?;
 let prepared=match &scratch{Some(scratch)=>native::use_scratch(prepared,scratch)?,None=>prepared};
 let prepared=crate::workers::prepare_worker(&provider,string(&step["mode"]),prepared,&step_offers)?;
 preview["input_sha256"]=digest(prompt.as_bytes()).into();let index=step_index(&state,string(&step["id"]))?;state["steps"][index]=preview.clone();graph_ctx.state=state.clone();save(&directory,"run.json",&state,false)?;
 // The exact input this agent receives, next to its receipt: what every agent of the run shared can be read back.
 save(&directory,&input_name(&preview),&object!({"schema_version":1,"kind":"ai-system-dispatch-input","dispatch_id":preview["dispatch_id"],"phase":preview["id"],"attempt":attempt,"input_sha256":preview["input_sha256"],"prompt":prompt}),false)?;
 brains.get(&sid).ok_or_else(||fail("Missing dispatch Brain"))?.progress(reference_mut(&mut state,&sid)?,&format!("Dispatch {} started: {}",string(&preview["dispatch_id"]),string(&preview["id"])))?;
 if let Some(bridge)=&hooks.bridge{bridge.on_dispatch_start(&graph_ctx)?;}
 notify(event,object!({"type":"dispatch_started","phase":preview["id"],"service":sid,"attempt":attempt,"dispatch_id":preview["dispatch_id"],"mode":preview["mode"],"provider":provider,"root":root,"readable":access["readable"],"writable":access["writable"]}));
 let started=Instant::now();let timeout=state["timeout"].as_u64().ok_or_else(||fail("Worker timeout missing"))?;let phase=string(&preview["id"]).to_string();
 let mut native_collector=NativeEvidenceCollector::default();
 let bridge_for_lines=hooks.bridge.clone();let ctx_for_lines=graph_ctx.clone();let provider_for_lines=provider.clone();let root_for_lines=root.clone();
 let output=Box::pin(native::run_raw_monitored(&prepared.argv,&root,&prepared.environment,&prepared.remove,&prepared.input,timeout,cancel.clone(),runner_lock.clone(),observer.clone(),|line|{native_collector.observe_line(line,&provider_for_lines,&root_for_lines);if let Some(bridge)=&bridge_for_lines{let _=bridge.on_dispatch_signal(&ctx_for_lines,line);}let line=String::from_utf8_lossy(line).to_string();notify(event,object!({"type":"output","phase":phase,"attempt":attempt,"line":line}));Ok(())})).await;
 let elapsed=started.elapsed().as_secs_f64();let mut receipt=object!({"dispatch_id":preview["dispatch_id"],"phase":preview["id"],"input_sha256":preview["input_sha256"],"mode":preview["mode"],"returncode":null,"error":null,"duration_seconds":elapsed,"ok":false,"report":null});
 match output{Ok(output)=>{receipt["returncode"]=object!(output.code);if output.code==0{match native::worker_result(&output.bytes,&provider).and_then(|report|{if untrusted_context&&infra_core::domain::secret_label(&encoded(&report)?)?.is_some(){return Err(fail("Sensitive experimental worker result"));}execution::validate_report(&report,&preview,&texts(&state["selected"]),access_scope(&state))?;Ok(report)}){Ok(report)=>{receipt["report"]=report;receipt["ok"]=object!(execution::success(&receipt,&preview,state["mode"]=="read-only"));if receipt["ok"]!=true{receipt["error"]=if preview["mode"]=="read-only"&&!texts(&receipt["report"]["changed_files"]).is_empty(){"read_only_worker_reported_writes"}else{"worker_blocked_or_verification_incomplete"}.into();}},Err(error)=>{receipt["error"]="invalid_or_sensitive_worker_result".into();receipt["rejection"]=if untrusted_context{"Invalid experimental worker report".into()}else{error.message.chars().take(500).collect::<String>().into()};}}}else{receipt["error"]="worker_exit_failure".into();if let Some(failure)=native::worker_failure(&output.bytes,&provider){receipt["failure"]=failure.into();}}},
 // A CLI that could not start or be supervised says why: a real verification stopped as only "unavailable" after the engine binary was replaced under a running Harness.
 Err(error)=>{let interrupted=cancel.load(Ordering::SeqCst);receipt["error"]=if interrupted{"interrupted"}else{error.code.as_deref().unwrap_or("worker_process_failure")}.into();if !interrupted&&!error.message.trim().is_empty(){receipt["failure"]=native::shown_failure(&error.message).into();}}}
 save(&directory,&receipt_name(&preview),&receipt,true)?;
 let head_after=git_head(&root);graph_ctx.head_after=head_after.clone();
 let git_changed=collect_step_git_changes(&state,&preview,&root,base_head.as_deref(),&pre_existing_dirty,receipt.get("report").filter(|v|v.is_object()));
 let bridge_evidence=match &hooks.bridge{Some(bridge)=>bridge.on_dispatch_finish(&graph_ctx,&receipt)?,None=>None};
 let evidence=match bridge_evidence{
  Some(mut ev)=>{
   ev.phase=string(&preview["id"]).to_string();
   ev.attempt=attempt;
   if ev.launch_id.is_none(){ev.launch_id=hooks.launch_id.clone();}
   ev.service=sid.clone();
   ev.root=path_string(&root)?;
   if ev.base_head.is_none(){ev.base_head=base_head;}
   if ev.head_after.is_none(){ev.head_after=head_after;}
   ev.git_changed=git_changed;
   ev
  }
  None=>{
   let mut ledger_edited:Vec<EvidenceFileChange>=native_collector.edited.into_iter().map(|(path,status)|EvidenceFileChange{path,status,from:None}).collect();
   if ledger_edited.is_empty(){
    for entry in texts(&receipt["report"]["changed_files"]){
     ledger_edited.push(EvidenceFileChange{path:entry,status:"modified".into(),from:None});
    }
   }
   DispatchEvidence{phase:string(&preview["id"]).to_string(),attempt,launch_id:hooks.launch_id.clone(),service:sid.clone(),root:path_string(&root)?,base_head,head_after,git_changed,ledger_edited,opened:native_collector.opened.into_iter().collect(),shell_reads:native_collector.shell_reads,shell_events:native_collector.shell_events,graph_asked:native_collector.graph_asked,steps:native_collector.steps,searched:native_collector.searched,commands:native_collector.commands,helpers:native_collector.helpers,limited:false,created_entities:vec![],target_folders:vec![],target_paths:vec![],target_nodes:vec![],file_touches:native_collector.file_touches,plan_items:native_collector.plan_items,command_runs:native_collector.command_runs}
  }
 };
 save(&directory,&evidence_name(&preview),&serde_json::to_value(&evidence)?,false)?;
 let finished=object!({"type":"dispatch_finished","phase":preview["id"],"service":sid,"attempt":attempt,"ok":receipt["ok"],"error":if receipt["rejection"].is_string(){object!(format!("{}: {}",string(&receipt["error"]),string(&receipt["rejection"])))}else if receipt["failure"].is_string(){object!(format!("{}: {}",string(&receipt["error"]),string(&receipt["failure"])))}else{receipt["error"].clone()},"duration_seconds":elapsed,"report":receipt["report"]});
 if receipt["ok"]!=true{let interrupted=cancel.load(Ordering::SeqCst)||receipt["error"]=="interrupted";state["steps"][index]["status"]=if interrupted{"interrupted"}else{"blocked"}.into();state["status"]=if interrupted{"interrupted"}else{"blocked"}.into();state["error"]=if receipt["failure"].is_string(){object!(match receipt["returncode"].as_i64(){Some(code)=>format!("The {provider} CLI exited with code {code}: {}",string(&receipt["failure"])),None=>format!("The {provider} CLI could not run: {}",string(&receipt["failure"]))})}else{receipt["error"].clone()};save(&directory,"run.json",&state,false)?;notify(event,finished);return Ok(state.clone());}
 current=System::new(&system.config,&system.allow_roots)?;plan=check_changes(&mut current,&plan,Some(&preview),Some(&receipt["report"]),&state,false)?;save(&directory,"checkpoint.json",&plan,false)?;finish(&directory,&mut state,&preview,&receipt,&brains)?;context_completed(&dispatch_context,&state,&preview,&receipt)?;notify(event,finished);
 if preview["id"]=="contracts"{break;}}
 if state["steps"].as_array().is_some_and(|steps|steps.iter().any(|step|step["status"]!="completed")){return Ok(state.clone());}
 let reports=reports(&directory,&state)?;
 // The run's outcome becomes the system's memory by itself; failing to share it never fails a completed run.
 let memory=if untrusted_context{object!({"status":"skipped","reason":"experimental worker claims stay in receipts and the private journal; no verified Brain outcome or promotion"})}else{match brains.get("__system__"){Some(brain)=>brain.share_outcome(&state,&plan,&reports).unwrap_or_else(|error|object!({"error":error.message})),None=>Value::Null}};
 let shared=if memory["status"]=="applied"{" Its outcome is now the system's memory: later plans share it with every agent."}else if memory["review_mode"]=="human"{" Its outcome waits as a proposal for the system's memory, since the system folder turns automatic promotion off: approve it with memory promote-review, then promote-apply."}else{""};
 save(&directory,"handoff.json",&object!({"schema_version":1,"kind":"ai-system-handoff","run_id":state["run_id"],"change_id":state["change_id"],"tasks":state["tasks"],"reports":reports,"memory":memory,"receipts":state["steps"].as_array().into_iter().flatten().map(receipt_name).collect::<Vec<_>>(),"knowledge":format!("Review contract decisions and reusable lessons in receipts; publish through each owning runtime's promotion workflow. Raw context is not durable knowledge.{shared} If the change added, versioned or removed a contract or a dependency, update the service passports (Fill with AI or the system editor) before planning the next change: plans route and trace impact only through declared contracts."),"meaning":"Worker-reported completion and checks; no commit, merge or deployment is implied."}),false)?;
 for sid in &ids{if !texts(&state["closed_tasks"]).contains(sid){let run=string(&state["run_id"]).to_string();brains.get(sid).ok_or_else(||fail("Missing native Brain"))?.complete(reference_mut(&mut state,sid)?,&run)?;state["closed_tasks"].as_array_mut().ok_or_else(||fail("Closed tasks missing"))?.push(sid.clone().into());save(&directory,"run.json",&state,false)?;}}
 state["status"]="completed".into();state["finished_at"]=now().into();save(&directory,"run.json",&state,false)?;Ok(state.clone())
 }).await;
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
    Box::pin(execute_run_context(
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
    ))
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

    let first = Box::pin(execute_run_context(
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
    ))
    .await;

    let final_result = match first {
        Ok(state)
            if state["status"] == "running"
                && state["steps"]
                    .as_array()
                    .is_some_and(|steps| steps.iter().all(|step| step["status"] != "running")) =>
        {
            Box::pin(execute_run_context(
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
            ))
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
