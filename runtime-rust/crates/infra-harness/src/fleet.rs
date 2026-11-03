use crate::{
    HarnessError, Result, legacy,
    process::Limits,
    provider::{self, Launch, WorkerResult},
    store::{self, SessionStore},
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};
#[derive(Clone)]
pub struct Config {
    pub project: PathBuf,
    pub provider: String,
    pub executable: PathBuf,
    pub lenses: Vec<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub budget: Option<f64>,
    pub seconds: u64,
    pub concurrency: usize,
    pub dry_run: bool,
    pub reports: PathBuf,
    pub adapter: Option<Arc<dyn provider::LaunchAdapter>>,
    pub runner_lock: Option<Arc<std::fs::File>>,
}
pub trait Blackboard: Send + Sync {
    fn available(&self) -> bool;
    fn ensure_task(&self, task: &str, title: &str) -> Result<()>;
    fn dispatch_spawn(&self, task: &str, lens: &str, capsule: &str) -> Result<()>;
    fn dispatch_complete(&self, task: &str, lens: &str, message: &str) -> Result<()>;
    fn progress(&self, task: &str, message: &str) -> Result<()>;
}
#[derive(Clone)]
pub struct Fleet {
    pub store: SessionStore,
    pub blackboard: Arc<dyn Blackboard>,
}
fn cost(v: &Value) -> Option<f64> {
    v.as_f64().filter(|n| n.is_finite() && *n >= 0.)
}
pub fn parse_findings(text: &str, lens: &str) -> Vec<Value> {
    for (start, _) in text.match_indices('{') {
        let mut stream = serde_json::Deserializer::from_str(&text[start..]).into_iter::<Value>();
        if let Some(Ok(v)) = stream.next()
            && let Some(items) = v["findings"].as_array()
        {
            return items
                .iter()
                .filter(|v| v.is_object() && v["claim"].as_str().is_some_and(|s| !s.is_empty()))
                .map(|v| {
                    let mut v = v.clone();
                    v["lens"] = lens.into();
                    v
                })
                .collect();
        }
    }
    let clean = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.is_empty() {
        vec![]
    } else {
        vec![
            json!({"lens":lens,"severity":"unparsed","claim":clean.chars().take(300).collect::<String>(),"file":"","evidence":"worker output did not contain a findings object"}),
        ]
    }
}
pub fn dedupe(items: &[Value]) -> Vec<Value> {
    let mut indexed = items.iter().enumerate().collect::<Vec<_>>();
    indexed.sort_by_key(|(i, v)| {
        (
            match v["severity"].as_str() {
                Some("high") => 0,
                Some("medium") => 1,
                Some("low") => 2,
                Some("unparsed") => 3,
                _ => 4,
            },
            *i,
        )
    });
    let mut seen = BTreeSet::new();
    indexed
        .into_iter()
        .filter_map(|(_, v)| {
            let key = (
                v["file"].as_str().unwrap_or("").to_string(),
                v["claim"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(120)
                    .collect::<String>()
                    .to_lowercase(),
            );
            seen.insert(key).then(|| v.clone())
        })
        .collect()
}
pub fn report(state: &Value) -> String {
    let lenses = state["lenses"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    let label = cost(&state["cost_usd"])
        .map(|v| format!("{}{v:.2}", char::from(36)))
        .unwrap_or("unknown".into());
    let mut out = format!(
        "# Fleet review — {}\n\nScope: {}\nTask: {}; lenses: {}; cost: {}\n\n",
        state["thread_id"].as_str().unwrap_or(""),
        state["scope"].as_str().unwrap_or(""),
        state["task_id"].as_str().unwrap_or(""),
        lenses,
        label
    );
    let items = dedupe(
        state["findings"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]),
    );
    if items.is_empty() {
        out.push_str("No findings.\n");
    }
    for item in items {
        let mut location = item["file"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("-")
            .to_string();
        if let Some(n) = item["line"].as_u64().filter(|n| *n > 0) {
            location.push_str(&format!(":{n}"));
        }
        out.push_str(&format!(
            "- [{}] {} ({}): {}",
            item["severity"].as_str().unwrap_or("?"),
            location,
            item["lens"].as_str().unwrap_or(""),
            item["claim"].as_str().unwrap_or("")
        ));
        if let Some(e) = item["evidence"].as_str().filter(|s| !s.is_empty()) {
            out.push_str(&format!(" — {e}"));
        }
        out.push('\n');
    }
    out
}
impl Fleet {
    pub fn initialize(
        &self,
        thread: &str,
        scope: &str,
        task: &str,
        c: &Config,
        legacy_database: Option<&Path>,
        action: Option<&str>,
    ) -> Result<()> {
        if c.lenses.is_empty()
            || c.lenses.len() > 40
            || c.concurrency == 0
            || c.concurrency > 40
            || c.lenses.iter().any(String::is_empty)
            || c.budget.is_some_and(|v| !v.is_finite() || v <= 0.)
        {
            return Err(HarnessError::invalid("Invalid Fleet configuration."));
        }
        {
            let db = self.store.connection()?;
            if db
                .query_row(
                    "SELECT 1 FROM native_fleet_runs WHERE thread_id=?1",
                    [thread],
                    |_| Ok(()),
                )
                .optional()?
                .is_some()
            {
                return Ok(());
            }
        }
        if let Some(path) = legacy_database
            && path.exists()
            && let Some(run) = legacy::decode_checkpoint(path, thread, action)?
        {
            return legacy::persist(&self.store, &run);
        }
        let state = json!({"scope":scope,"task_id":task,"thread_id":thread,"lenses":c.lenses,"findings":[],"cost_usd":0.0,"skipped":[]});
        self.store.connection()?.execute(
            "INSERT INTO native_fleet_runs(thread_id,state,stage)VALUES(?1,?2,'scope')",
            params![thread, state.to_string()],
        )?;
        Ok(())
    }
    pub fn snapshot(&self, thread: &str) -> Result<(Value, String, Option<bool>, i64)> {
        let db = self.store.connection()?;
        let (raw, stage, decision, rev) = db.query_row(
            "SELECT state,stage,decision,revision FROM native_fleet_runs WHERE thread_id=?1",
            [thread],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            },
        )?;
        Ok((
            crate::strict_json::parse(raw.as_bytes(), 16 * 1024 * 1024)?,
            stage,
            decision.map(|v| v != 0),
            rev,
        ))
    }
    fn transition(&self, thread: &str, rev: i64, state: &Value, stage: &str) -> Result<()> {
        if self.store.connection()?.execute("UPDATE native_fleet_runs SET state=?1,stage=?2,revision=revision+1 WHERE thread_id=?3 AND revision=?4",params![state.to_string(),stage,thread,rev])?!=1{return Err(HarnessError::conflict("Fleet state changed."));}
        Ok(())
    }
    pub fn decide(&self, thread: &str, approve: bool) -> Result<()> {
        let mut db = self.store.connection()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (stage, old): (String, Option<i64>) = tx.query_row(
            "SELECT stage,decision FROM native_fleet_runs WHERE thread_id=?1",
            [thread],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if old.is_some_and(|v| v != i64::from(approve))
            || !["gate", "record", "done"].contains(&stage.as_str())
        {
            return Err(HarnessError::conflict(
                "Fleet decision conflicts or the gate is unavailable.",
            ));
        }
        tx.execute("UPDATE native_fleet_runs SET decision=?1,stage=CASE WHEN stage='gate' THEN 'record' ELSE stage END,revision=revision+1 WHERE thread_id=?2",params![i64::from(approve),thread])?;
        tx.commit()?;
        Ok(())
    }
    fn tasks(&self, thread: &str) -> Result<Vec<(String, Value)>> {
        let db = self.store.connection()?;
        let mut q=db.prepare("SELECT task_id,input FROM native_fleet_tasks WHERE thread_id=?1 AND status!='completed' ORDER BY ordinal")?;
        let rows = q.query_map([thread], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, raw) = row?;
            out.push((id, crate::strict_json::parse(raw.as_bytes(), 1024 * 1024)?));
        }
        Ok(out)
    }
    fn finish(&self, thread: &str, tid: &str, result: &WorkerResult) -> Result<()> {
        let mut db = self.store.connection()?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (input, status): (String, String) = tx.query_row(
            "SELECT input,status FROM native_fleet_tasks WHERE thread_id=?1 AND task_id=?2",
            params![thread, tid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if status == "completed" {
            return Ok(());
        }
        if !result.ok {
            tx.execute("UPDATE native_fleet_tasks SET status='pending',error=?1 WHERE thread_id=?2 AND task_id=?3",params![result.error,thread,tid])?;
            tx.commit()?;
            return Err(HarnessError::unavailable(
                result.error.clone().unwrap_or("Reviewer failed.".into()),
            ));
        }
        let input = crate::strict_json::parse(input.as_bytes(), 1024 * 1024)?;
        let raw: String = tx.query_row(
            "SELECT state FROM native_fleet_runs WHERE thread_id=?1",
            [thread],
            |r| r.get(0),
        )?;
        let mut state = crate::strict_json::parse(raw.as_bytes(), 16 * 1024 * 1024)?;
        let findings = parse_findings(&result.text, input["lens"].as_str().unwrap_or(""));
        state["findings"]
            .as_array_mut()
            .ok_or_else(|| HarnessError::invalid("Invalid Fleet findings."))?
            .extend(findings.iter().cloned());
        state["cost_usd"] = json!(
            cost(&state["cost_usd"])
                .zip(result.cost_usd)
                .map(|(a, b)| a + b)
        );
        tx.execute("UPDATE native_fleet_tasks SET status='completed',output=?1,error=NULL WHERE thread_id=?2 AND task_id=?3",params![json!({"findings":findings,"cost_usd":result.cost_usd}).to_string(),thread,tid])?;
        tx.execute(
            "UPDATE native_fleet_runs SET state=?1,revision=revision+1 WHERE thread_id=?2",
            params![state.to_string(), thread],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub async fn drive(
        &self,
        thread: &str,
        c: &Config,
        cancel: Arc<AtomicBool>,
        emit: Arc<dyn Fn(Value) -> Result<()> + Send + Sync>,
    ) -> Result<Value> {
        loop {
            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                return Err(HarnessError::conflict("Cancellation requested."));
            }
            let (mut state, stage, decision, rev) = self.snapshot(thread)?;
            match stage.as_str() {
                "scope" => {
                    emit(json!({"kind":"fleet_stage","stage":"scope","status":"running"}))?;
                    if !c.dry_run {
                        self.blackboard.ensure_task(
                            state["task_id"].as_str().unwrap_or(""),
                            &format!(
                                "Unattended fleet review ({})",
                                state["scope"].as_str().unwrap_or("")
                            ),
                        )?;
                    }
                    let allowance = c.budget.map(|budget| {
                        cost(&state["cost_usd"])
                            .map(|spent| {
                                ((budget - spent).max(0.) / c.lenses.len() as f64 * 1_000_000.)
                                    .floor()
                                    / 1_000_000.
                            })
                            .unwrap_or(0.)
                    });
                    {
                        let mut db = self.store.connection()?;
                        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
                        for (n, lens) in c.lenses.iter().enumerate() {
                            let tid = store::sha(format!("{thread}\0{n}\0{lens}").as_bytes());
                            let input = json!({"scope":state["scope"],"task_id":state["task_id"],"thread_id":thread,"lens":lens,"budget_usd":allowance});
                            let skipped = allowance.is_some_and(|v| v <= 0.);
                            tx.execute("INSERT OR IGNORE INTO native_fleet_tasks(thread_id,task_id,lens,ordinal,status,input,output)VALUES(?1,?2,?3,?4,?5,?6,?7)",params![thread,tid,lens,n as i64,if skipped{"completed"}else{"pending"},input.to_string(),skipped.then(||json!({"skipped":[lens],"cost_usd":0.0}).to_string())])?;
                            if skipped {
                                state["skipped"]
                                    .as_array_mut()
                                    .ok_or_else(|| HarnessError::invalid("Invalid skipped list."))?
                                    .push(Value::String(lens.clone()));
                            }
                        }
                        tx.execute("UPDATE native_fleet_runs SET state=?1,stage='review',revision=revision+1 WHERE thread_id=?2 AND revision=?3",params![state.to_string(),thread,rev])?;
                        tx.commit()?;
                    }
                    emit(json!({"kind":"fleet_stage","stage":"scope","status":"completed"}))?;
                }
                "review" => {
                    emit(json!({"kind":"fleet_stage","stage":"review","status":"running"}))?;
                    let semaphore = Arc::new(tokio::sync::Semaphore::new(c.concurrency));
                    let mut running = tokio::task::JoinSet::new();
                    for (tid, input) in self.tasks(thread)? {
                        let permit =
                            semaphore.clone().acquire_owned().await.map_err(|_| {
                                HarnessError::unavailable("Fleet scheduler stopped.")
                            })?;
                        let engine = self.clone();
                        let cfg = c.clone();
                        let cancel = cancel.clone();
                        let emit = emit.clone();
                        let thread = thread.to_string();
                        running.spawn(async move{let _permit=permit;let lens=input["lens"].as_str().ok_or_else(||HarnessError::invalid("Missing lens."))?;let task=input["task_id"].as_str().unwrap_or("");emit(json!({"kind":"fleet_reviewer","lens":lens,"status":"running","cost_usd":null}))?;
       let capsule=format!("**Objective** - review through the {lens} lens.\n**Output format** - JSON findings with file,line,severity(high|medium|low),claim,evidence.\n**Tool and source guidance** - project root; scope {}. Task ID: {task}.\n**Task boundaries** - report only; do not modify files, run fixes or spawn agents.\n**Decisions and assumptions so far** - unattended run {thread}; no user amendments.\n",input["scope"].as_str().unwrap_or(""));if !cfg.dry_run{engine.blackboard.dispatch_spawn(task,lens,&capsule)?;}
       let result=if cfg.dry_run{WorkerResult{text:json!({"findings":[{"file":"DEMO.md","line":1,"severity":"low","claim":format!("Offline demonstration finding ({lens})"),"evidence":"Synthetic finding; no project code was reviewed."}]}).to_string(),ok:true,cost_usd:Some(0.),session_id:None,duration_seconds:0.,error:None,limit_reached:None}}
       else{let launch=Launch{provider:cfg.provider.clone(),executable:cfg.executable.clone(),project:cfg.project.clone(),prompt:format!("Act as {lens}. Return only JSON findings.\n\n{capsule}"),mode:"plan".into(),model:cfg.model.clone(),session:None,agents:false,count:1,effort:cfg.effort.clone(),usd:cost(&input["budget_usd"]),hook_events:false};let emit=emit.clone();provider::run_monitored_locked(&launch,Limits{seconds:Some(cfg.seconds),..Limits::default()},cancel,None,cfg.runner_lock.clone(),cfg.adapter.as_deref(),&std::collections::BTreeMap::new(),None,move|e|emit(e)).await?};
       if !cfg.dry_run{engine.blackboard.dispatch_complete(task,lens,if result.ok{&result.text}else{"Reviewer failed."})?;}emit(json!({"kind":"fleet_reviewer","lens":lens,"status":if result.ok{"completed"}else{"failed"},"cost_usd":result.cost_usd,"duration_seconds":result.duration_seconds,"limit_reached":result.limit_reached}))?;engine.finish(&thread,&tid,&result)
      });
                    }
                    let mut failure = None;
                    while let Some(v) = running.join_next().await {
                        match v {
                            Ok(Ok(())) => {}
                            Ok(Err(e)) => {
                                failure.get_or_insert(e);
                            }
                            Err(_) => {
                                failure.get_or_insert(HarnessError::unavailable(
                                    "Reviewer task failed.",
                                ));
                            }
                        }
                    }
                    if let Some(e) = failure {
                        return Err(e);
                    }
                    let (state, _, _, rev) = self.snapshot(thread)?;
                    self.transition(thread, rev, &state, "collect")?;
                }
                "collect" => {
                    emit(json!({"kind":"fleet_stage","stage":"review","status":"completed"}))?;
                    self.transition(thread, rev, &state, "gate")?;
                    emit(json!({"kind":"fleet_stage","stage":"gate","status":"waiting"}))?;
                }
                "gate" => {
                    return Ok(
                        json!({"kind":"fleet_state","status":"awaiting_approval","findings":dedupe(state["findings"].as_array().map(Vec::as_slice).unwrap_or(&[])),"cost_usd":state["cost_usd"],"skipped":state["skipped"],"report":"","preview":report(&state),"dry_run":c.dry_run,"brain_available":!c.dry_run&&self.blackboard.available(),"task_id":state["task_id"]}),
                    );
                }
                "record" => {
                    let approve = decision
                        .ok_or_else(|| HarnessError::conflict("Fleet decision is missing."))?;
                    state["approved"] = approve.into();
                    let mut rendered = report(&state);
                    if c.dry_run {
                        rendered = format!(
                            "> Offline dry-run demonstration. No project code was reviewed; no models were called.\n\n{rendered}"
                        );
                    }
                    if approve {
                        infra_core::fs::ensure_directory(&c.reports, true)?;
                        let path = c.reports.join(format!("{thread}.md"));
                        infra_core::fs::admit_descendant(&c.reports, &path)?;
                        infra_core::fs::atomic_write(&path, rendered.as_bytes())?;
                        if !c.dry_run {
                            self.blackboard.progress(
                                state["task_id"].as_str().unwrap_or(""),
                                &format!(
                                    "fleet-review {thread}: approved report {}",
                                    path.display()
                                ),
                            )?;
                        }
                    } else if !c.dry_run {
                        self.blackboard.progress(
                            state["task_id"].as_str().unwrap_or(""),
                            &format!("fleet-review {thread}: rejected at the gate"),
                        )?;
                    }
                    state["report"] = if approve { rendered.into() } else { "".into() };
                    self.transition(thread, rev, &state, "done")?;
                }
                "done" => {
                    return Ok(
                        json!({"kind":"fleet_state","status":if state["approved"]==true{"completed"}else{"rejected"},"findings":dedupe(state["findings"].as_array().map(Vec::as_slice).unwrap_or(&[])),"cost_usd":state["cost_usd"],"skipped":state["skipped"],"report":state["report"],"preview":report(&state),"dry_run":c.dry_run,"brain_available":!c.dry_run,"task_id":state["task_id"]}),
                    );
                }
                _ => return Err(HarnessError::invalid("Invalid native Fleet stage.")),
            }
        }
    }
}
