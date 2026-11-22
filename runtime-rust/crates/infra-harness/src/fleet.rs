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
fn reviewer_event_status(ok: bool, schema_error: Option<&str>) -> &'static str {
    if !ok {
        "failed"
    } else if schema_error.is_some() {
        "schema_error"
    } else {
        "completed"
    }
}

/// Strictly validates Fleet JSON findings (`{"findings": [...]}`) without creating
/// heuristic `"severity": "unparsed"` placeholder items.
pub fn parse_findings_strict(text: &str, lens: &str) -> std::result::Result<Vec<Value>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("empty reviewer response".into());
    }
    let root = if let Ok(v) = serde_json::from_str::<Value>(trimmed)
        && v.is_object()
    {
        v
    } else if let Some(fenced) = crate::clash::extract_json_fence(trimmed)
        && let Ok(v) = serde_json::from_str::<Value>(fenced)
        && v.is_object()
    {
        v
    } else {
        return Err("reviewer output is not a valid JSON object or fenced ```json block".into());
    };
    let Some(items) = root.get("findings").and_then(Value::as_array) else {
        return Err("missing `findings` array in JSON output".into());
    };
    let mut validated = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        let Some(obj) = item.as_object() else {
            return Err(format!("`findings[{idx}]` must be an object"));
        };
        let Some(claim) = obj
            .get("claim")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return Err(format!(
                "`findings[{idx}].claim` must be a non-empty string"
            ));
        };
        let sev = obj
            .get("severity")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_ascii_lowercase())
            .unwrap_or_default();
        if !["critical", "high", "medium", "low", "info"].contains(&sev.as_str()) {
            return Err(format!(
                "`findings[{idx}].severity` must be critical|high|medium|low|info (got `{sev}`)"
            ));
        }
        let mut entry = Value::Object(obj.clone());
        entry["lens"] = lens.into();
        entry["severity"] = sev.into();
        entry["claim"] = claim.chars().take(2000).collect::<String>().into();
        validated.push(entry);
    }
    Ok(validated)
}

pub fn parse_findings(text: &str, lens: &str) -> Vec<Value> {
    parse_findings_strict(text, lens).unwrap_or_default()
}

pub fn dedupe(items: &[Value]) -> Vec<Value> {
    let mut indexed = items.iter().enumerate().collect::<Vec<_>>();
    indexed.sort_by_key(|(i, v)| {
        (
            match v["severity"].as_str() {
                Some("critical") => 0,
                Some("high") => 1,
                Some("medium") => 2,
                Some("low") => 3,
                Some("info") => 4,
                _ => 5,
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
        self.initialize_with_precheck(thread, scope, task, c, legacy_database, action, None)
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "Fleet initialization accepts optional precheck settings alongside legacy checkpoint import."
    )]
    pub fn initialize_with_precheck(
        &self,
        thread: &str,
        scope: &str,
        task: &str,
        c: &Config,
        legacy_database: Option<&Path>,
        action: Option<&str>,
        precheck: Option<&Value>,
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
        let precheck_val = match precheck.filter(|v| !v.is_null()) {
            Some(v) => crate::clash::validate_precheck(Some(v))?,
            None => {
                let stored = self
                    .store
                    .get(thread)
                    .ok()
                    .and_then(|s| s.get("fleet").and_then(|f| f.get("precheck")).cloned())
                    .filter(|v| !v.is_null());
                match stored {
                    Some(v) => crate::clash::validate_precheck(Some(&v))?,
                    None => Value::Null,
                }
            }
        };
        let mut state = json!({"scope":scope,"task_id":task,"thread_id":thread,"lenses":c.lenses,"findings":[],"cost_usd":0.0,"usage_components":{"input_tokens":0,"output_tokens":0,"cache_read_input_tokens":0,"cache_write_5m_input_tokens":0,"cache_write_1h_input_tokens":0,"model_calls":0},"skipped":[]});
        if !precheck_val.is_null() {
            state["precheck"] = precheck_val;
        }
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
    fn finish(
        &self,
        thread: &str,
        tid: &str,
        result: &WorkerResult,
        usage_components: &Value,
        schema_error: Option<&str>,
    ) -> Result<()> {
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
        let lens = input["lens"].as_str().unwrap_or("");
        let (findings, schema_err) = match parse_findings_strict(&result.text, lens) {
            Ok(items) => (items, schema_error.map(str::to_owned)),
            Err(err) => (vec![], Some(schema_error.map(str::to_owned).unwrap_or(err))),
        };
        state["findings"]
            .as_array_mut()
            .ok_or_else(|| HarnessError::invalid("Invalid Fleet findings."))?
            .extend(findings.iter().cloned());
        state["cost_usd"] = json!(
            cost(&state["cost_usd"])
                .zip(result.cost_usd)
                .map(|(a, b)| a + b)
        );
        crate::clash::merge_usage_components(&mut state["usage_components"], usage_components);
        if let Some(err) = &schema_err {
            let mut errs = state["schema_errors"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if errs.len() < 32 {
                errs.push(json!({"lens": lens, "error": err}));
            }
            state["schema_errors"] = json!(errs);
            tx.execute(
                "UPDATE native_fleet_tasks SET status='schema_error',output=?1,error=?2 WHERE thread_id=?3 AND task_id=?4",
                params![
                    json!({"findings":findings,"cost_usd":result.cost_usd,"usage_components":usage_components,"schema_error":err}).to_string(),
                    err,
                    thread,
                    tid
                ],
            )?;
            tx.execute(
                "UPDATE native_fleet_runs SET state=?1,revision=revision+1 WHERE thread_id=?2",
                params![state.to_string(), thread],
            )?;
            tx.commit()?;
            return Err(HarnessError::unavailable(format!(
                "Fleet reviewer `{lens}` failed strict findings schema validation: {err}"
            )));
        }
        if let Some(errs) = state["schema_errors"].as_array_mut() {
            errs.retain(|e| e["lens"] != lens);
        }
        tx.execute("UPDATE native_fleet_tasks SET status='completed',output=?1,error=NULL WHERE thread_id=?2 AND task_id=?3",params![json!({"findings":findings,"cost_usd":result.cost_usd,"usage_components":usage_components,"schema_error":Value::Null}).to_string(),thread,tid])?;
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
                    if !c.dry_run
                        && let Some(pre) = state.get("precheck").filter(|v| !v.is_null())
                        && let Some(record) =
                            crate::clash::run_precheck_sandboxed(&c.project, pre).await
                    {
                        let (mut st, _, _, r) = self.snapshot(thread)?;
                        st["deterministic_precheck"] = record;
                        self.transition(thread, r, &st, "review")?;
                    }
                    emit(json!({"kind":"fleet_stage","stage":"scope","status":"completed"}))?;
                }
                "review" => {
                    emit(json!({"kind":"fleet_stage","stage":"review","status":"running"}))?;
                    let pre_note = match state
                        .get("deterministic_precheck")
                        .filter(|v| v.is_object())
                    {
                        Some(pre) => {
                            let summary = pre["summary"].as_str().unwrap_or("").trim();
                            if !summary.is_empty() {
                                format!("\n**Deterministic pre-stage check** -\n{summary}\n")
                            } else if pre["ok"] == false {
                                let code = pre["exit_code"]
                                    .as_i64()
                                    .map(|c| format!(" (exit code {c})"))
                                    .unwrap_or_default();
                                format!(
                                    "\n**Deterministic pre-stage check** -\nPrecheck failed{code}.\n"
                                )
                            } else {
                                String::new()
                            }
                        }
                        None => String::new(),
                    };
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
                        let pre_note = pre_note.clone();
                        running.spawn(async move {
                            let _permit = permit;
                            let lens = input["lens"].as_str().ok_or_else(|| HarnessError::invalid("Missing lens."))?;
                            let task = input["task_id"].as_str().unwrap_or("");
                            emit(json!({"kind":"fleet_reviewer","lens":lens,"status":"running","cost_usd":null}))?;
                            let capsule = format!(
                                "**Objective** - review through the {lens} lens.\n**Output format** - JSON findings with file,line,severity(critical|high|medium|low|info),claim,evidence.\n**Tool and source guidance** - project root; scope {}. Task ID: {task}.\n**Task boundaries** - report only; do not modify files, run fixes or spawn agents.\n**Decisions and assumptions so far** - unattended run {thread}; no user amendments.\n{pre_note}",
                                input["scope"].as_str().unwrap_or("")
                            );
                            if !cfg.dry_run {
                                engine.blackboard.dispatch_spawn(task, lens, &capsule)?;
                            }
                            let (result, usage_components, schema_error) = if cfg.dry_run {
                                (
                                    WorkerResult {
                                        text: json!({"findings":[{"file":"DEMO.md","line":1,"severity":"low","claim":format!("Offline demonstration finding ({lens})"),"evidence":"Synthetic finding; no project code was reviewed."}]}).to_string(),
                                        ok: true,
                                        cost_usd: Some(0.),
                                        session_id: None,
                                        duration_seconds: 0.,
                                        error: None,
                                        limit_reached: None,
                                    },
                                    json!({"input_tokens":0u64,"output_tokens":0u64,"cache_read_input_tokens":0u64,"cache_write_5m_input_tokens":0u64,"cache_write_1h_input_tokens":0u64,"model_calls":0u64,"source":"deterministic"}),
                                    None,
                                )
                            } else {
                                run_reviewer_with_repair(
                                    &cfg,
                                    lens,
                                    &capsule,
                                    cost(&input["budget_usd"]),
                                    cancel,
                                    emit.clone(),
                                )
                                .await?
                            };
                            if !cfg.dry_run {
                                let note = if !result.ok {
                                    "Reviewer failed."
                                } else if let Some(err) = &schema_error {
                                    err.as_str()
                                } else {
                                    &result.text
                                };
                                engine.blackboard.dispatch_complete(task, lens, note)?;
                            }
                            let reviewer_status =
                                reviewer_event_status(result.ok, schema_error.as_deref());
                            emit(json!({
                                "kind":"fleet_reviewer",
                                "lens":lens,
                                "status":reviewer_status,
                                "cost_usd":result.cost_usd,
                                "usage_components":usage_components,
                                "schema_error":schema_error,
                                "duration_seconds":result.duration_seconds,
                                "limit_reached":result.limit_reached
                            }))?;
                            engine.finish(&thread, &tid, &result, &usage_components, schema_error.as_deref())
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
                        json!({"kind":"fleet_state","status":"awaiting_approval","findings":dedupe(state["findings"].as_array().map(Vec::as_slice).unwrap_or(&[])),"cost_usd":state["cost_usd"],"usage_components":state["usage_components"],"schema_errors":state["schema_errors"],"deterministic_precheck":state["deterministic_precheck"],"skipped":state["skipped"],"report":"","preview":report(&state),"dry_run":c.dry_run,"brain_available":!c.dry_run&&self.blackboard.available(),"task_id":state["task_id"]}),
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
                        json!({"kind":"fleet_state","status":if state["approved"]==true{"completed"}else{"rejected"},"findings":dedupe(state["findings"].as_array().map(Vec::as_slice).unwrap_or(&[])),"cost_usd":state["cost_usd"],"usage_components":state["usage_components"],"schema_errors":state["schema_errors"],"deterministic_precheck":state["deterministic_precheck"],"skipped":state["skipped"],"report":state["report"],"preview":report(&state),"dry_run":c.dry_run,"brain_available":!c.dry_run,"task_id":state["task_id"]}),
                    );
                }
                _ => return Err(HarnessError::invalid("Invalid native Fleet stage.")),
            }
        }
    }
}

async fn run_reviewer_with_repair(
    cfg: &Config,
    lens: &str,
    capsule: &str,
    budget_usd: Option<f64>,
    cancel: Arc<AtomicBool>,
    emit: Arc<dyn Fn(Value) -> Result<()> + Send + Sync>,
) -> Result<(WorkerResult, Value, Option<String>)> {
    let usage = Arc::new(std::sync::Mutex::new(json!({
        "input_tokens": 0u64,
        "output_tokens": 0u64,
        "cache_read_input_tokens": 0u64,
        "cache_write_5m_input_tokens": 0u64,
        "cache_write_1h_input_tokens": 0u64,
        "model_calls": 0u64,
        "source": cfg.provider,
    })));
    let run_once = |prompt: String, usd: Option<f64>| {
        let cfg = cfg.clone();
        let cancel = cancel.clone();
        let emit = emit.clone();
        let usage = usage.clone();
        async move {
            let launch = Launch {
                provider: cfg.provider.clone(),
                executable: cfg.executable.clone(),
                project: cfg.project.clone(),
                prompt,
                mode: "plan".into(),
                model: cfg.model.clone(),
                session: None,
                agents: false,
                count: 1,
                effort: cfg.effort.clone(),
                usd,
                hook_events: false,
                permissions: None,
                approvals: None,
            };
            provider::run_monitored_locked(
                &launch,
                Limits {
                    seconds: Some(cfg.seconds),
                    ..Limits::default()
                },
                cancel,
                None,
                cfg.runner_lock.clone(),
                cfg.adapter.as_deref(),
                &std::collections::BTreeMap::new(),
                None,
                move |e| {
                    if e["kind"] == "usage"
                        && let Ok(mut slot) = usage.lock()
                    {
                        crate::clash::observe_usage_event(&mut slot, &e);
                    }
                    emit(e)
                },
            )
            .await
        }
    };
    let first = run_once(
        format!("Act as {lens}. Return only JSON findings.\n\n{capsule}"),
        budget_usd,
    )
    .await?;
    let mut combined = first.clone();
    let mut schema_error = None;
    if first.ok
        && let Err(err) = parse_findings_strict(&first.text, lens)
    {
        let remaining_usd = match (budget_usd, first.cost_usd) {
            (Some(b), Some(used)) => Some((b - used).max(0.0)),
            (b, _) => b,
        };
        let can_repair = !cancel.load(std::sync::atomic::Ordering::Acquire)
            && remaining_usd.is_none_or(|r| r >= 0.01);
        if can_repair {
            let snippet: String = first.text.chars().take(1024).collect();
            let repair_prompt = format!(
                "Act as {lens}. Your previous response failed strict JSON schema validation ({err}).\nReturn ONLY a single JSON object `{{\"findings\":[{{\"file\":\"...\",\"line\":1,\"severity\":\"high|medium|low\",\"claim\":\"...\",\"evidence\":\"...\"}}]}}`.\nPrevious output:\n{snippet}"
            );
            if let Ok(repaired) = run_once(repair_prompt, remaining_usd).await {
                combined.cost_usd = match (first.cost_usd, repaired.cost_usd) {
                    (Some(a), Some(b)) => Some(a + b),
                    (a, b) => a.or(b),
                };
                combined.duration_seconds += repaired.duration_seconds;
                if repaired.ok {
                    match parse_findings_strict(&repaired.text, lens) {
                        Ok(_) => combined.text = repaired.text,
                        Err(err2) => schema_error = Some(err2),
                    }
                } else {
                    schema_error = Some(err);
                }
            } else {
                schema_error = Some(err);
            }
        } else {
            schema_error = Some(err);
        }
    }
    let usage_val = usage
        .lock()
        .map(|g| g.clone())
        .unwrap_or_else(|_| json!({}));
    Ok((combined, usage_val, schema_error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_findings_parser_refuses_unparsed_prose_and_normalizes_valid_json() {
        let prose = "No defects found cleanly without JSON.";
        assert!(parse_findings_strict(prose, "security").is_err());
        assert!(parse_findings(prose, "security").is_empty());

        let invalid_sev = r#"{"findings":[{"severity":"bogus","claim":"bad","evidence":"x"}]}"#;
        assert!(parse_findings_strict(invalid_sev, "security").is_err());

        let valid_fenced = "```json\n{\"findings\":[{\"severity\":\"HIGH\",\"claim\":\"SQL injection\",\"file\":\"app.php\",\"line\":10,\"evidence\":\"raw sql\"}]}\n```";
        let findings = parse_findings_strict(valid_fenced, "security").unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0]["severity"], "high");
        assert_eq!(findings[0]["lens"], "security");

        let mut acc = json!({});
        crate::clash::merge_usage_components(
            &mut acc,
            &json!({"input_tokens": 20, "output_tokens": 5, "cache_read_input_tokens": 80, "model_calls": 1}),
        );
        assert_eq!(acc["input_tokens"], 20);
        assert_eq!(acc["output_tokens"], 5);
        assert_eq!(acc["cache_read_input_tokens"], 80);
        assert_eq!(acc["model_calls"], 1);
    }

    struct NoopBlackboard;
    impl Blackboard for NoopBlackboard {
        fn available(&self) -> bool {
            false
        }
        fn ensure_task(&self, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn dispatch_spawn(&self, _: &str, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn dispatch_complete(&self, _: &str, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn progress(&self, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn fleet_initialize_with_precheck_persists_precheck_and_rejects_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&dir.path().join("state")).unwrap();
        let fleet = Fleet {
            store: store.clone(),
            blackboard: Arc::new(NoopBlackboard),
        };
        let cfg = Config {
            project: dir.path().to_path_buf(),
            provider: "claude".into(),
            executable: dir.path().join("fake-cli"),
            lenses: vec!["code-reviewer".into()],
            model: None,
            effort: None,
            budget: None,
            seconds: 30,
            concurrency: 1,
            dry_run: true,
            reports: dir.path().join("reports"),
            adapter: None,
            runner_lock: None,
        };

        assert!(
            fleet
                .initialize_with_precheck(
                    "t-bad",
                    "scope",
                    "harness/t-bad",
                    &cfg,
                    None,
                    None,
                    Some(&json!({"argv": []})),
                )
                .is_err()
        );

        let precheck = json!({"argv": ["composer", "validate"], "timeout_seconds": 20});
        fleet
            .initialize_with_precheck(
                "t-pre",
                "scope",
                "harness/t-pre",
                &cfg,
                None,
                None,
                Some(&precheck),
            )
            .unwrap();
        let (state, stage, _, _) = fleet.snapshot("t-pre").unwrap();
        assert_eq!(stage, "scope");
        assert_eq!(state["precheck"], precheck);
    }

    #[tokio::test]
    async fn fleet_schema_error_never_marks_task_completed_and_retries_cleanly() {
        assert_eq!(reviewer_event_status(false, None), "failed");
        assert_eq!(
            reviewer_event_status(true, Some("bad json")),
            "schema_error"
        );
        assert_eq!(reviewer_event_status(true, None), "completed");

        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&dir.path().join("state")).unwrap();
        let fleet = Fleet {
            store: store.clone(),
            blackboard: Arc::new(NoopBlackboard),
        };
        let cfg = Config {
            project: dir.path().to_path_buf(),
            provider: "claude".into(),
            executable: dir.path().join("fake-cli"),
            lenses: vec!["code-reviewer".into(), "security-reviewer".into()],
            model: None,
            effort: None,
            budget: None,
            seconds: 30,
            concurrency: 2,
            dry_run: true,
            reports: dir.path().join("reports"),
            adapter: None,
            runner_lock: None,
        };
        let thread = "t-schema";
        fleet
            .initialize(thread, "src/", "harness/t-schema", &cfg, None, None)
            .unwrap();
        // Seed both tasks in `review` stage mirroring `scope`.
        let (state, _, _, rev) = fleet.snapshot(thread).unwrap();
        fleet.transition(thread, rev, &state, "review").unwrap();
        let mut tids = Vec::new();
        {
            let db = store.connection().unwrap();
            for (n, lens) in cfg.lenses.iter().enumerate() {
                let tid = store::sha(format!("{thread}\0{n}\0{lens}").as_bytes());
                let input = json!({"scope":"src/","task_id":"harness/t-schema","thread_id":thread,"lens":lens,"budget_usd":null});
                db.execute(
                    "INSERT INTO native_fleet_tasks(thread_id,task_id,lens,ordinal,status,input)VALUES(?1,?2,?3,?4,'pending',?5)",
                    params![thread, tid, lens, n as i64, input.to_string()],
                )
                .unwrap();
                tids.push(tid);
            }
        }

        let usage = json!({"input_tokens": 10, "output_tokens": 5, "model_calls": 1});
        // Lens 0 succeeds with valid structured JSON.
        fleet
            .finish(
                thread,
                &tids[0],
                &WorkerResult {
                    text: r#"{"findings":[{"file":"a.php","line":1,"severity":"medium","claim":"Check","evidence":"e"}]}"#.into(),
                    ok: true,
                    cost_usd: Some(0.02),
                    session_id: None,
                    duration_seconds: 1.0,
                    error: None,
                    limit_reached: None,
                },
                &usage,
                None,
            )
            .unwrap();

        // Lens 1 returns invalid prose after repair -> must NOT mark task `completed`!
        let err = fleet
            .finish(
                thread,
                &tids[1],
                &WorkerResult {
                    text: "No JSON findings here".into(),
                    ok: true,
                    cost_usd: Some(0.03),
                    session_id: None,
                    duration_seconds: 1.0,
                    error: None,
                    limit_reached: None,
                },
                &usage,
                Some("reviewer output is not a valid JSON object"),
            )
            .unwrap_err();
        assert!(err.message.contains("security-reviewer"), "{}", err.message);

        let (task_status, task_err): (String, Option<String>) = store
            .connection()
            .unwrap()
            .query_row(
                "SELECT status, error FROM native_fleet_tasks WHERE thread_id=?1 AND task_id=?2",
                params![thread, tids[1]],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(task_status, "schema_error");
        assert!(task_err.is_some());

        let (state_after_err, stage_after_err, _, _) = fleet.snapshot(thread).unwrap();
        assert_eq!(stage_after_err, "review");
        assert_eq!(state_after_err["findings"].as_array().unwrap().len(), 1);
        assert_eq!(
            state_after_err["schema_errors"].as_array().unwrap().len(),
            1
        );
        assert_eq!(
            state_after_err["schema_errors"][0]["lens"],
            "security-reviewer"
        );

        // Pending tasks for resume include ONLY the schema_error lens.
        let pending = fleet.tasks(thread).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, tids[1]);

        // Retry succeeds -> clears schema_error for that lens and advances cleanly via drive().
        fleet
            .finish(
                thread,
                &tids[1],
                &WorkerResult {
                    text: r#"{"findings":[{"file":"b.php","line":4,"severity":"high","claim":"Auth bypass","evidence":"missing check"}]}"#.into(),
                    ok: true,
                    cost_usd: Some(0.02),
                    session_id: None,
                    duration_seconds: 1.0,
                    error: None,
                    limit_reached: None,
                },
                &usage,
                None,
            )
            .unwrap();
        assert!(fleet.tasks(thread).unwrap().is_empty());

        let gated = fleet
            .drive(
                thread,
                &cfg,
                Arc::new(AtomicBool::new(false)),
                Arc::new(|_| Ok(())),
            )
            .await
            .unwrap();
        assert_eq!(gated["status"], "awaiting_approval");
        assert_eq!(gated["findings"].as_array().unwrap().len(), 2);
        assert_eq!(gated["schema_errors"].as_array().unwrap().len(), 0);
    }
}
