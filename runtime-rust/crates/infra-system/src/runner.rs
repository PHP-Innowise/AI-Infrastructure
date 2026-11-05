use crate::{
    catalog::System,
    common::*,
    execution::{
        self, Brain, access_scope, changed_path, dispatch_access, dispatch_receipt, reports,
    },
    native,
};
use infra_core::{ErrorKind, Result, RuntimeError};
use serde_json::{Value, json as object};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

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
        let why = if !writable.iter().any(|value| value == sid) {
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
    event: &mut (impl FnMut(Value) -> Result<()> + Send),
) -> Result<Value> {
    let directory = absolute(directory)?;
    let _run_lock = execution::lock(&directory, ".run.lock")?;
    let initial = native::validate_state(system, &directory, true).await?;
    let _workspaces = execution::workspace_locks(system, &texts(&initial["selected"]))?;
    let mut state = native::validate_state(system, &directory, true).await?;
    if state["status"] == "completed" {
        return Ok(state);
    }
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
    let result=async{
 let blocked=state["steps"].as_array().into_iter().flatten().find(|step|["running","blocked","interrupted"].contains(&string(&step["status"]))).cloned();
 if let Some(step)=blocked{let receipt=match dispatch_receipt(&directory,&state,&step){Ok(value)=>Some(value),Err(error)if error.kind==ErrorKind::NotFound=>None,Err(error)=>return Err(error)};
 let unrun=receipt.as_ref().and_then(|receipt|execution::unrun_checks(receipt,&step));
 if accept.unrun_checks&&unrun.is_none(){return Err(fail(format!("Only a completed verification whose only gaps are checks it could not run can be accepted; retry {id} instead (CLI: --retry-step {id})",id=string(&step["id"]))));}
 if accept.unrun_checks&&retry.is_some(){return Err(fail("Accept the checks verification could not run, or retry verification, not both"));}
 if receipt.as_ref().is_some_and(|receipt|receipt["ok"]==true)||accept.unrun_checks{let receipt=receipt.ok_or_else(||fail("Receipt disappeared"))?;if system.verify(&plan)?["fresh"]!=true{if !accept.source_changes{return Err(fail("Successful receipt has uncheckpointed edits; inspect and explicitly accept source changes"));}plan=check_changes(system,&plan,Some(&step),Some(&receipt["report"]),&state,true)?;save(&directory,"checkpoint.json",&plan,false)?;}
 if let Some(unrun)=unrun.filter(|_|accept.unrun_checks){let index=step_index(&state,string(&step["id"]))?;state["steps"][index]["accepted_unrun_checks"]=object!(unrun);}finish(&directory,&mut state,&step,&receipt,&brains)?;retry=None;}
 else{if retry.as_deref()!=Some(string(&step["id"])){return Err(fail(format!("Dispatch {id} stopped without a successful receipt and may have changed files; inspect them, then retry {id} explicitly (CLI: --retry-step {id})",id=string(&step["id"]))));}
 if system.verify(&plan)?["fresh"]!=true{if !accept.source_changes{return Err(fail("Partial edits detected; inspect and explicitly accept source changes"));}plan=check_changes(system,&plan,Some(&step),None,&state,true)?;save(&directory,"checkpoint.json",&plan,false)?;}
 let index=step_index(&state,string(&step["id"]))?;state["steps"][index]["status"]="pending".into();}}
 else if retry.is_some(){return Err(fail("No unfinished dispatch matches the dispatch chosen for retry"));}
 else if accept.unrun_checks{return Err(fail("No stopped verification has checks to accept"));}
 if system.verify(&plan)?["fresh"]!=true{return Err(fail("Checkpoint sources changed; prepare new plan"));}plan=system.refresh_checkpoint(&plan,&BTreeSet::new())?;
 let ids=brains.keys().cloned().collect::<Vec<_>>();for sid in &ids{let brain=brains.get(sid).ok_or_else(||fail("Missing Brain"))?;let record=brain.ensure(reference_mut(&mut state,sid)?)?;if record["status"]=="completed"&&!texts(&state["closed_tasks"]).contains(sid){if state["steps"].as_array().is_some_and(|steps|steps.iter().any(|step|step["status"]!="completed")){return Err(fail("Native task terminal before worker verification"));}let run=string(&state["run_id"]).to_string();brain.complete(reference_mut(&mut state,sid)?,&run)?;state["closed_tasks"].as_array_mut().ok_or_else(||fail("Closed tasks missing"))?.push(sid.clone().into());}save(&directory,"run.json",&state,false)?;}
 state["status"]="running".into();state["error"]=Value::Null;save(&directory,"run.json",&state,false)?;
 let steps=state["steps"].as_array().cloned().ok_or_else(||fail("Missing steps"))?;
 for step in steps{if step["status"]=="completed"{continue;}
 if cancel.load(Ordering::SeqCst){return Err(RuntimeError::coded(ErrorKind::Unavailable,"interrupted","interrupted"));}
 let mut current=System::new(&system.config,&system.allow_roots)?;if current.verify(&plan)?["fresh"]!=true{return Err(fail("Sources changed between dispatches; prepare new plan"));}
 let prior=reports(&directory,&state)?;let attempt=step["attempt"].as_u64().filter(|attempt|*attempt<100).ok_or_else(||fail("Dispatch retry limit reached"))?+1;let mut preview=step.clone();preview["dispatch_id"]=id32().into();preview["status"]="running".into();preview["attempt"]=object!(attempt);
 let sid=string(&step["service"]).to_string();let provider=string(&state["provider"]).to_string();let executable=std::path::PathBuf::from(string(&state["executable"]));let scratch=native::read_only_scratch(&provider,&executable,string(&step["mode"]),&directory)?;
 let prompt=native::prompt_for(&state,&plan,&preview,&prior,&directory,scratch.as_deref())?;if prompt.chars().count()>128000{return Err(fail("Worker prompt exceeds 128000 characters; narrow plan"));}let access=dispatch_access(&state,&step);let root=std::path::PathBuf::from(string(&state["roots"][&sid]));let schema=execution::result_schema(&texts(&state["selected"]));
 let prepared=native::prepare_launch(&provider,&executable,&root,&prompt,&schema,&directory.join("result-schema.json"),string(&step["mode"]),&execution::extra_directories(&state,&step),adapter.as_deref())?;let prepared=match &scratch{Some(scratch)=>native::use_scratch(prepared,scratch)?,None=>prepared};
 preview["input_sha256"]=digest(prompt.as_bytes()).into();let index=step_index(&state,string(&step["id"]))?;state["steps"][index]=preview.clone();save(&directory,"run.json",&state,false)?;
 brains.get(&sid).ok_or_else(||fail("Missing dispatch Brain"))?.progress(reference_mut(&mut state,&sid)?,&format!("Dispatch {} started: {}",string(&preview["dispatch_id"]),string(&preview["id"])))?;
 notify(event,object!({"type":"dispatch_started","phase":preview["id"],"service":sid,"attempt":attempt,"dispatch_id":preview["dispatch_id"],"mode":preview["mode"],"provider":provider,"root":root,"readable":access["readable"],"writable":access["writable"]}));
 let started=Instant::now();let timeout=state["timeout"].as_u64().ok_or_else(||fail("Worker timeout missing"))?;let phase=string(&preview["id"]).to_string();
 let output=native::run_raw_monitored(&prepared.argv,&root,&prepared.environment,&prepared.remove,&prepared.input,timeout,cancel.clone(),runner_lock.clone(),observer.clone(),|line|{let line=String::from_utf8_lossy(line).to_string();notify(event,object!({"type":"output","phase":phase,"attempt":attempt,"line":line}));Ok(())}).await;
 let elapsed=started.elapsed().as_secs_f64();let mut receipt=object!({"dispatch_id":preview["dispatch_id"],"phase":preview["id"],"input_sha256":preview["input_sha256"],"mode":preview["mode"],"returncode":null,"error":null,"duration_seconds":elapsed,"ok":false,"report":null});
 match output{Ok(output)=>{receipt["returncode"]=object!(output.code);if output.code==0{match native::worker_result(&output.bytes,&provider).and_then(|report|{execution::validate_report(&report,&preview,&texts(&state["selected"]),access_scope(&state))?;Ok(report)}){Ok(report)=>{receipt["report"]=report;receipt["ok"]=object!(execution::success(&receipt,&preview));if receipt["ok"]!=true{receipt["error"]=if preview["mode"]=="read-only"&&!texts(&receipt["report"]["changed_files"]).is_empty(){"read_only_worker_reported_writes"}else{"worker_blocked_or_verification_incomplete"}.into();}},Err(error)=>{receipt["error"]="invalid_or_sensitive_worker_result".into();receipt["rejection"]=error.message.chars().take(500).collect::<String>().into();}}}else{receipt["error"]="worker_exit_failure".into();if let Some(failure)=native::worker_failure(&output.bytes,&provider){receipt["failure"]=failure.into();}}},
 Err(error)=>{receipt["error"]=if cancel.load(Ordering::SeqCst){"interrupted"}else{error.code.as_deref().unwrap_or("worker_process_failure")}.into();}}
 save(&directory,&receipt_name(&preview),&receipt,true)?;let finished=object!({"type":"dispatch_finished","phase":preview["id"],"service":sid,"attempt":attempt,"ok":receipt["ok"],"error":if receipt["rejection"].is_string(){object!(format!("{}: {}",string(&receipt["error"]),string(&receipt["rejection"])))}else if receipt["failure"].is_string(){object!(format!("{}: {}",string(&receipt["error"]),string(&receipt["failure"])))}else{receipt["error"].clone()},"duration_seconds":elapsed,"report":receipt["report"]});
 if receipt["ok"]!=true{let interrupted=cancel.load(Ordering::SeqCst)||receipt["error"]=="interrupted";state["steps"][index]["status"]=if interrupted{"interrupted"}else{"blocked"}.into();state["status"]=if interrupted{"interrupted"}else{"blocked"}.into();state["error"]=if receipt["failure"].is_string(){object!(format!("The {} CLI exited with code {}: {}",provider,receipt["returncode"].as_i64().map_or_else(||"?".to_string(),|code|code.to_string()),string(&receipt["failure"])))}else{receipt["error"].clone()};save(&directory,"run.json",&state,false)?;notify(event,finished);return Ok(state.clone());}
 current=System::new(&system.config,&system.allow_roots)?;plan=check_changes(&mut current,&plan,Some(&preview),Some(&receipt["report"]),&state,false)?;save(&directory,"checkpoint.json",&plan,false)?;finish(&directory,&mut state,&preview,&receipt,&brains)?;notify(event,finished);
 if preview["id"]=="contracts"{break;}}
 if state["steps"].as_array().is_some_and(|steps|steps.iter().any(|step|step["status"]!="completed")){return Ok(state.clone());}
 let reports=reports(&directory,&state)?;save(&directory,"handoff.json",&object!({"schema_version":1,"kind":"ai-system-handoff","run_id":state["run_id"],"change_id":state["change_id"],"tasks":state["tasks"],"reports":reports,"receipts":state["steps"].as_array().into_iter().flatten().map(receipt_name).collect::<Vec<_>>(),"knowledge":"Review contract decisions and reusable lessons in receipts; publish through each owning runtime's promotion workflow. Raw context is not durable knowledge. If the change added, versioned or removed a contract or a dependency, update the service passports (Fill with AI or the system editor) before planning the next change: plans route and trace impact only through declared contracts.","meaning":"Worker-reported completion and checks; no commit, merge or deployment is implied."}),false)?;
 for sid in &ids{if !texts(&state["closed_tasks"]).contains(sid){let run=string(&state["run_id"]).to_string();brains.get(sid).ok_or_else(||fail("Missing native Brain"))?.complete(reference_mut(&mut state,sid)?,&run)?;state["closed_tasks"].as_array_mut().ok_or_else(||fail("Closed tasks missing"))?.push(sid.clone().into());save(&directory,"run.json",&state,false)?;}}
 state["status"]="completed".into();state["finished_at"]=now().into();save(&directory,"run.json",&state,false)?;Ok(state.clone())
 }.await;
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
    execute_run_context(
        system,
        directory,
        retry,
        Acceptance::sources(accept),
        cancel,
        runner_lock,
        None,
        None,
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
    event: &mut (impl FnMut(Value) -> Result<()> + Send),
) -> Result<Value> {
    let state = execute_run_context(
        system,
        directory,
        retry,
        accept,
        cancel.clone(),
        runner_lock.clone(),
        adapter.clone(),
        observer.clone(),
        event,
    )
    .await?;
    if state["status"] == "running"
        && state["steps"]
            .as_array()
            .is_some_and(|steps| steps.iter().all(|step| step["status"] != "running"))
    {
        execute_run_context(
            system,
            directory,
            None,
            Acceptance::default(),
            cancel,
            runner_lock,
            adapter,
            observer,
            event,
        )
        .await
    } else {
        Ok(state)
    }
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
        system, directory, retry, accept, cancel, None, None, None, event,
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
        event,
    )
    .await
}
