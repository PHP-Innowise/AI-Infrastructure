use crate::{
    backend::{SessionBridge, State},
    catalog::System,
    common::*,
    editor::Editor,
    execution, native,
    system_graph::{CodeGraphBridge, load_run_evidence, summarize_run_evidence},
};
use infra_core::{Result, RuntimeError};
use serde_json::{Value, json as object};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
pub const RUNNER_EVENTS: &[&str] = &["status", "text", "error", "result"];
pub fn active(value: &Value) -> bool {
    matches!(
        string(&value["status"]),
        "queued" | "starting" | "running" | "cancelling"
    )
}
pub struct SystemManager {
    pub state: Arc<State>,
    editor: Mutex<Editor>,
    code_graph: Mutex<Option<Arc<dyn CodeGraphBridge>>>,
}
impl SystemManager {
    pub fn new(source: PathBuf, state: PathBuf, host: Arc<dyn SessionBridge>) -> Result<Self> {
        native::private_directory(&state)?;
        native::private_directory(&state.join("ai-system"))?;
        let state = Arc::new(State {
            source,
            state: state.clone(),
            db: crate::backend::Database::new(&state)?,
            host,
        });
        let manager = Self {
            state,
            editor: Mutex::new(Editor::new()),
            code_graph: Mutex::new(None),
        };
        {
            let editor = manager
                .editor
                .lock()
                .map_err(|_| fail("System editor lock poisoned"))?;
            editor.recover(&manager.state, None, false)?;
            for (id, record) in manager.state.db.all("system_runs", None)? {
                manager.reconcile_record(&id, record)?;
            }
        }
        Ok(manager)
    }
    pub fn set_code_graph_bridge(&self, bridge: Arc<dyn CodeGraphBridge>) -> Result<()> {
        *self
            .code_graph
            .lock()
            .map_err(|_| fail("System code-graph bridge lock poisoned"))? = Some(bridge);
        Ok(())
    }
    pub fn with_code_graph_bridge(self, bridge: Arc<dyn CodeGraphBridge>) -> Result<Self> {
        self.set_code_graph_bridge(bridge)?;
        Ok(self)
    }
    /// Loads and validates a registered [`System`] enforcing `State::allowed()` and edit recovery.
    pub fn load_system(&self, project: &str, config: &str) -> Result<System> {
        let editor = self
            .editor
            .lock()
            .map_err(|_| fail("System manager lock poisoned"))?;
        self.system(&editor, project, config)
    }
    fn system(&self, editor: &Editor, project: &str, config: &str) -> Result<System> {
        let (_, path) = editor.location(&self.state, project, config)?;
        let system = System::new(&path, &self.state.allowed()?)?;
        for service in system.services.values() {
            editor.check_recovery(&self.state, &service.root)?;
        }
        Ok(system)
    }
    fn record(&self, rid: &str) -> Result<Value> {
        if !hex(rid, 32) {
            return Err(fail("Invalid system run ID"));
        }
        let record = self
            .state
            .db
            .get("system_runs", rid)?
            .ok_or_else(|| RuntimeError::not_found("Unknown system run"))?;
        self.reconcile_record(rid, record)
    }
    fn reconcile_record(&self, rid: &str, mut record: Value) -> Result<Value> {
        if !hex(rid, 32) {
            return Err(fail("Invalid system run ID"));
        }
        if let Some(nonce) = record["pending_nonce"]
            .as_str()
            .filter(|value| !value.is_empty())
            .map(str::to_string)
        {
            if let Some(session) = self.state.host.find_launch(rid, &nonce)? {
                let sid = string(&session["id"]);
                let mut launches = texts(&record["launches"]);
                if launches.is_empty()
                    && let Some(prior) = record["session_id"].as_str()
                {
                    launches.push(prior.into());
                }
                if !launches.iter().any(|prior| prior == sid) {
                    launches.push(sid.into());
                }
                record["session_id"] = sid.into();
                record["launches"] = object!(launches);
            }
            record["pending_nonce"] = Value::Null;
            self.save(&record)?;
        }
        Ok(record)
    }
    fn save(&self, record: &Value) -> Result<()> {
        self.state
            .db
            .put("system_runs", string(&record["id"]), record)
    }
    fn view(&self, editor: Option<&Editor>, record: &Value, detail: bool) -> Result<Value> {
        let mut result = record.clone();
        let session = record["session_id"]
            .as_str()
            .map(|sid| self.state.host.session(sid))
            .transpose()?;
        let is_active = session.as_ref().is_some_and(active);
        result["provider"] = record["provider"]
            .as_str()
            .map(|value| object!(value))
            .or_else(|| session.as_ref().map(|session| session["provider"].clone()))
            .unwrap_or(Value::Null);
        result["session_status"] = session
            .as_ref()
            .map(|session| session["status"].clone())
            .unwrap_or(Value::Null);
        result["active"] = is_active.into();
        // Before any launch a change is in its plan's state: a plan that could
        // not choose its services needs selection, not review.
        result["status"] = session
            .as_ref()
            .map(|session| session["status"].clone())
            .unwrap_or_else(|| {
                record
                    .get("plan_status")
                    .filter(|status| status.is_string())
                    .cloned()
                    .unwrap_or(object!("needs_review"))
            });
        result["access"] = record
            .get("access")
            .cloned()
            .filter(|value| value.is_string())
            .unwrap_or(object!("service"));
        result["tell_agents_graph"] = record
            .get("tell_agents_graph")
            .filter(|value| value.is_boolean())
            .cloned()
            .unwrap_or(Value::Bool(true));
        let directory = self
            .state
            .state
            .join("ai-system")
            .join(string(&record["id"]));
        let journal = directory.join("execution");
        let read = |name: &str| -> Result<Option<Value>> {
            if !journal.is_dir() {
                return Ok(None);
            }
            // As large as the run itself writes them: the handoff keeps every
            // complete report.
            match read_json(&journal, name, execution::MAX_STATE) {
                Ok(value) => Ok(Some(value)),
                Err(error) if is_active || error.kind == infra_core::ErrorKind::NotFound => {
                    Ok(None)
                }
                Err(error) => Err(error),
            }
        };
        let loaded_state = read("run.json")?;
        let dispatches = if detail && let Some(ref state) = loaded_state {
            load_run_evidence(&journal, state).unwrap_or_default()
        } else {
            vec![]
        };
        if let Some(ref state) = loaded_state {
            result["status"] = state["status"].clone();
            result["access"] = execution::access_scope(state).into();
            if let Some(tell) = state.get("tell_agents_graph").filter(|v| v.is_boolean()) {
                result["tell_agents_graph"] = tell.clone();
            }
            if is_active {
                result["status"] = session.as_ref().unwrap()["status"].clone();
            } else if matches!(string(&result["status"]), "prepared" | "running") {
                result["status"] = "interrupted".into();
            }
            if detail {
                result["execution"] = execution::run_summary(state, &journal);
                let mut receipts = vec![];
                for step in items(&state["steps"], MAX_SERVICES + 2)? {
                    if step["attempt"].as_u64().unwrap_or(0) > 0
                        && let Some(receipt) = read(&receipt_name(step))?
                    {
                        // The exact input the agent received, the run's shared
                        // context included, when this run saved it.
                        let input = journal.join(input_name(step));
                        let input = if input.is_file() {
                            object!(path_string(&input)?)
                        } else {
                            Value::Null
                        };
                        let step_ev = dispatches
                            .iter()
                            .find(|ev| {
                                ev.phase == string(&step["id"])
                                    && Some(ev.attempt) == step["attempt"].as_u64()
                            })
                            .and_then(|ev| serde_json::to_value(ev).ok())
                            .unwrap_or(Value::Null);
                        receipts.push(object!({"phase":step["id"],"service":step["service"],"attempt":step["attempt"],"ok":receipt["ok"],"error":receipt["error"],"rejection":receipt.get("rejection").cloned().unwrap_or(Value::Null),"failure":receipt.get("failure").cloned().unwrap_or(Value::Null),"report":receipt["report"],"input":input,"evidence":step_ev}));
                    }
                }
                result["receipts"] = object!(receipts);
                if let Some(handoff) = read("handoff.json")? {
                    result["handoff"] = handoff;
                }
            }
        } else if session.is_some() && !is_active {
            result["status"] = "interrupted".into();
        }
        if detail {
            result["plan"] = read_json(&directory, "approved-plan.json", MAX_BYTES)?;
            result["events"] = if let Some(session) = &session {
                object!(
                    self.state
                        .host
                        .events(string(&session["id"]), RUNNER_EVENTS)?
                )
            } else {
                object!([])
            };
            let mut launches = texts(&record["launches"]);
            if launches.is_empty()
                && let Some(sid) = record["session_id"].as_str()
            {
                launches.push(sid.into());
            }
            result["launches"] = object!(
                launches
                    .iter()
                    .enumerate()
                    .map(|(idx, sid)| {
                        let session = self.state.host.session(sid)?;
                        let launch_phases = dispatches
                            .iter()
                            .filter(|ev| ev.launch_id.as_deref() == Some(sid.as_str()))
                            .map(|ev| object!({ "phase": ev.phase, "attempt": ev.attempt, "service": ev.service }))
                            .collect::<Vec<_>>();
                        Ok(object!({
                            "session_id": sid,
                            "launch_number": idx + 1,
                            "status": session["status"],
                            "created_at": session.get("created_at").cloned().unwrap_or(Value::Null),
                            "updated_at": session.get("updated_at").cloned().unwrap_or(Value::Null),
                            "finished_at": session.get("finished_at").cloned().unwrap_or(Value::Null),
                            "dispatches": launch_phases,
                        }))
                    })
                    .collect::<Result<Vec<_>>>()?
            );
            let opt_system = editor.and_then(|ed| {
                self.system(
                    ed,
                    string(&record["project_id"]),
                    string(&record["config_path"]),
                )
                .ok()
            });
            let bridge_opt = self
                .code_graph
                .lock()
                .map_err(|_| fail("System code-graph bridge lock poisoned"))?
                .clone();
            // Summing the evidence up looks at the checkout of every member.
            // A bridge that knows the graphs sums it up itself, from these
            // dispatches and the ones still going on, and keeps its answer
            // while they stand as they are; a view it left without a
            // summary is summed up here.
            if let Some(bridge) = bridge_opt {
                bridge.enrich_run_view(
                    string(&record["id"]),
                    opt_system.as_ref(),
                    loaded_state.as_ref(),
                    &dispatches,
                    &mut result,
                )?;
            }
            if !result["evidence"].is_object() {
                result["evidence"] = summarize_run_evidence(
                    opt_system.as_ref(),
                    loaded_state.as_ref().unwrap_or(&Value::Null),
                    &dispatches,
                );
            }
            result["providers"] = object!(
                [
                    ("codex", "Codex"),
                    ("claude", "Claude Code"),
                    ("cursor", "Cursor Agent")
                ]
                .into_iter()
                .map(|(id, name)| self
                    .state
                    .host
                    .provider(id)
                    .map(|exe| object!({"id":id,"name":name,"available":exe.is_some()})))
                .collect::<Result<Vec<_>>>()?
            );
            result["codex_available"] = self.state.host.provider("codex")?.is_some().into();
        }
        Ok(result)
    }
    fn prepare(&self, editor: &Editor, data: &Value) -> Result<Value> {
        fields(
            data,
            &["project_id", "config_path", "task", "change_id"],
            &["services", "contracts", "budget", "depth"],
        )?;
        let task = data["task"]
            .as_str()
            .filter(|value| value.chars().count() <= 2000)
            .ok_or_else(|| fail("Describe task in at most 2000 characters"))?
            .replace(['\r', '\n'], " ");
        let services = bounded_selections(data.get("services"))?;
        let contracts = bounded_selections(data.get("contracts"))?;
        let mut system = self.system(
            editor,
            string(&data["project_id"]),
            string(&data["config_path"]),
        )?;
        let budget = integer(data.get("budget"), 8000, 0, 64000)? as usize;
        let depth = integer(data.get("depth"), 32, 0, 64)? as usize;
        let plan = system.plan(
            &task,
            string(&data["change_id"]),
            &services,
            &contracts,
            budget,
            depth,
        )?;
        let rid = id32();
        let directory = self.state.state.join("ai-system").join(&rid);
        create_private(&directory)?;
        create_new(&directory, "approved-plan.json", &plan)?;
        let record = object!({"id":rid,"revision":0,"created_at":now(),"project_id":data["project_id"],"config_path":data["config_path"],"system":plan["system"],"change_id":plan["change_id"],"plan_status":plan["status"],"session_id":null,"provider":null,"mode":null,"timeout":null});
        self.save(&record)?;
        self.view(Some(editor), &record, true)
    }
    fn act(&self, editor: &Editor, rid: &str, data: &Value) -> Result<Value> {
        fields(
            data,
            &["action", "revision"],
            &[
                "mode",
                "timeout",
                "provider",
                "access",
                "retry_step",
                "accept_source_changes",
                "accept_unrun_checks",
                "tell_agents_graph",
            ],
        )?;
        let mut record = self.record(rid)?;
        if !data["revision"].is_u64() || data["revision"] != record["revision"] {
            return Err(RuntimeError::conflict(
                "Run changed in another view. Refresh before acting",
            ));
        }
        let action = string(&data["action"]);
        let allowed = match action {
            "execute" => vec![
                "action",
                "revision",
                "mode",
                "timeout",
                "provider",
                "access",
                "tell_agents_graph",
            ],
            "resume" => vec![
                "action",
                "revision",
                "retry_step",
                "accept_source_changes",
                "accept_unrun_checks",
                "tell_agents_graph",
            ],
            "cancel" | "archive" => vec!["action", "revision"],
            _ => return Err(fail("Invalid system run action")),
        };
        if data
            .as_object()
            .unwrap()
            .keys()
            .any(|key| !allowed.contains(&key.as_str()))
        {
            return Err(fail("Invalid system run action fields"));
        }
        if data
            .get("tell_agents_graph")
            .is_some_and(|value| !value.is_null() && !value.is_boolean())
        {
            return Err(fail("tell_agents_graph must be boolean"));
        }
        let view = self.view(Some(editor), &record, true)?;
        if action == "cancel" {
            if view["active"] == true {
                self.state.host.cancel(string(&record["session_id"]))?;
            }
            return self.view(Some(editor), &record, true);
        }
        // Removing a change from the list keeps its plan, receipts and the
        // services' native tasks; only a stopped change can be removed.
        if action == "archive" {
            if view["active"] == true || record["pending_nonce"].is_string() {
                return Err(RuntimeError::conflict(
                    "Wait for this launch to stop before removing the change from the list",
                ));
            }
            record["archived"] = true.into();
            record["revision"] = (record["revision"].as_u64().unwrap_or(0) + 1).into();
            self.save(&record)?;
            return self.view(Some(editor), &record, true);
        }
        if view["active"] == true {
            return Err(RuntimeError::conflict(
                "Wait for this launch to stop before continuing",
            ));
        }
        self.state.host.admit()?;
        let mut system = self.system(
            editor,
            string(&record["project_id"]),
            string(&record["config_path"]),
        )?;
        let directory = self.state.state.join("ai-system").join(rid);
        let plan = read_json(&directory, "approved-plan.json", MAX_BYTES)?;
        let (provider, mode, timeout, access) = if action == "execute" {
            if !record["session_id"].is_null() {
                return Err(fail(
                    "This plan was already launched. Use explicit recovery",
                ));
            }
            system.selected_plan(&plan, true)?;
            if system.verify(&plan)?["fresh"] != true {
                return Err(RuntimeError::conflict(
                    "Plan is stale. Prepare and review a new plan",
                ));
            }
            let provider = data["provider"].as_str().unwrap_or("codex").to_string();
            if !["codex", "claude", "cursor"].contains(&provider.as_str()) {
                return Err(fail("Choose Codex, Claude or Cursor"));
            }
            let mode = data["mode"].as_str().unwrap_or("read-only").to_string();
            if !["read-only", "edit"].contains(&mode.as_str()) {
                return Err(fail("Choose read-only or edit mode"));
            }
            let timeout = integer(data.get("timeout"), 900, 1, 86400)?;
            let access = data["access"].as_str().unwrap_or("service").to_string();
            if !["service", "all"].contains(&access.as_str())
                || (access == "all" && provider == "cursor")
            {
                return Err(fail(
                    "Selected provider cannot grant requested service folders",
                ));
            }
            (provider, mode, timeout, access)
        } else {
            if record["session_id"].is_null() || view["status"] == "completed" {
                return Err(fail("Only unfinished launches can be resumed"));
            }
            if ["accept_source_changes", "accept_unrun_checks"]
                .iter()
                .any(|key| data.get(*key).is_some_and(|value| !value.is_boolean()))
            {
                return Err(fail("Source acknowledgement must be boolean"));
            }
            if data.get("retry_step").is_some_and(|value| {
                !value.is_null() && (!value.is_string() || string(value).len() > 100)
            }) {
                return Err(fail("Choose an unfinished dispatch to retry"));
            }
            let provider = string(&view["provider"]).to_string();
            if directory.join("execution/run.json").exists() {
                let state =
                    native::validate_state_sync(&mut system, &directory.join("execution"), false)?;
                if state["provider"] != provider {
                    return Err(fail("Saved provider differs from launch"));
                }
                if let Some(retry) = data["retry_step"].as_str()
                    && !items(&state["steps"], MAX_SERVICES + 2)?
                        .iter()
                        .any(|step| {
                            step["id"] == retry
                                && matches!(
                                    string(&step["status"]),
                                    "running" | "interrupted" | "blocked"
                                )
                        })
                {
                    return Err(fail("Only an unfinished dispatch may be retried"));
                }
            } else if system.verify(&plan)?["fresh"] != true {
                return Err(RuntimeError::conflict("Plan is stale. Prepare a new plan"));
            }
            (
                provider,
                string(&record["mode"]).to_string(),
                record["timeout"]
                    .as_u64()
                    .ok_or_else(|| fail("Invalid saved timeout"))?,
                string(&view["access"]).to_string(),
            )
        };
        let executable = self
            .state
            .host
            .provider(&provider)?
            .ok_or_else(|| fail("Selected system provider CLI is unavailable"))?;
        let tell_agents_graph = data
            .get("tell_agents_graph")
            .and_then(Value::as_bool)
            .or_else(|| record.get("tell_agents_graph").and_then(Value::as_bool));
        let nonce = id32();
        let mut request = object!({"directory":directory,"system_file":system.config,"allowed_roots":self.state.allowed()?,"provider":provider,"executable":executable,"mode":mode,"timeout":timeout,"access":access,"retry_step":data.get("retry_step"),"accept_source_changes":data["accept_source_changes"]==true,"accept_unrun_checks":data["accept_unrun_checks"]==true});
        if let Some(tell) = tell_agents_graph {
            request["tell_agents_graph"] = object!(tell);
            record["tell_agents_graph"] = object!(tell);
        }
        create_new(&directory, &format!("request-{nonce}.json"), &request)?;
        record["pending_nonce"] = nonce.clone().into();
        record["provider"] = provider.clone().into();
        record["mode"] = mode.clone().into();
        record["timeout"] = timeout.into();
        record["access"] = access.into();
        record["revision"] = (record["revision"].as_u64().unwrap_or(0) + 1).into();
        self.save(&record)?;
        let internal = object!({"run_id":rid,"nonce":nonce});
        let prompt = launch_title(&record, &plan, data["retry_step"].as_str());
        let session=self.state.host.launch(&object!({"project_id":record["project_id"],"provider":provider,"workflow":"native","mode":if mode=="edit"{"edit"}else{"plan"},"prompt":prompt,"project_context":false,"agents_enabled":false,"budgets":{"seconds":null,"tokens":null,"usd":null}}),Some(&internal),None)?;
        let sid = string(&session["id"]);
        if sid.is_empty() {
            return Err(fail("Queue returned no session identity"));
        }
        let mut launches = texts(&record["launches"]);
        if launches.is_empty()
            && let Some(prior) = record["session_id"].as_str()
        {
            launches.push(prior.into());
        }
        launches.push(sid.into());
        record["session_id"] = sid.into();
        record["pending_nonce"] = Value::Null;
        record["launches"] = object!(launches);
        self.save(&record)?;
        self.view(Some(editor), &record, true)
    }
    pub fn execute(&self, operation: &str, envelope: &Value) -> Result<Value> {
        let data = envelope
            .get("body")
            .filter(|value| !value.is_null())
            .unwrap_or(envelope);
        let params = envelope.get("params").unwrap_or(&Value::Null);
        let query = envelope.get("query").unwrap_or(&Value::Null);
        let mut editor = self
            .editor
            .lock()
            .map_err(|_| fail("System manager lock poisoned"))?;
        match operation {
            "systems.editor" | "systems.edit.load" => editor.load(&self.state, data),
            "systems.service" | "systems.edit.service" => editor.service(&self.state, data),
            "systems.preview" | "systems.edit.preview" => editor.preview(&self.state, data),
            "systems.apply" | "systems.edit.apply" => editor.apply(&self.state, data),
            "systems.catalog" => {
                fields(data, &["project_id", "config_path"], &[])?;
                let system = self.system(
                    &editor,
                    string(&data["project_id"]),
                    string(&data["config_path"]),
                )?;
                Ok(
                    object!({"catalog":system.catalog(),"diagram":system.diagram(),"root":system.root}),
                )
            }
            "systems.prepare" => self.prepare(&editor, data),
            "systems.list" => {
                let project = query["project_id"]
                    .as_str()
                    .or_else(|| params["project_id"].as_str())
                    .or_else(|| data["project_id"].as_str())
                    .ok_or_else(|| fail("Project identity required"))?;
                self.state.project(project)?;
                Ok(
                    object!({"runs":self.state.db.all("system_runs",Some(project))?.into_iter().filter(|(_,record)|record["archived"]!=true).map(|(rid,record)|self.reconcile_record(&rid,record).and_then(|record|self.view(Some(&editor),&record,false))).collect::<Result<Vec<_>>>()?}),
                )
            }
            "systems.get" => self.view(
                Some(&editor),
                &self.record(
                    params["id"]
                        .as_str()
                        .or_else(|| data["id"].as_str())
                        .ok_or_else(|| fail("Run identity required"))?,
                )?,
                true,
            ),
            "systems.act" => self.act(
                &editor,
                params["id"]
                    .as_str()
                    .or_else(|| data["id"].as_str())
                    .ok_or_else(|| fail("Run identity required"))?,
                data,
            ),
            "discovery.start" => crate::discovery::start(&self.state, &editor, data),
            "discovery.get" => crate::discovery::get(
                &self.state,
                params["id"]
                    .as_str()
                    .or_else(|| params["session_id"].as_str())
                    .or_else(|| data["id"].as_str())
                    .ok_or_else(|| fail("Discovery identity required"))?,
            ),
            _ => Err(fail("Unknown system manager operation")),
        }
    }
}
pub fn integer(value: Option<&Value>, default: u64, min: u64, max: u64) -> Result<u64> {
    match value {
        None => Ok(default),
        Some(value) => value
            .as_u64()
            .filter(|value| *value >= min && *value <= max)
            .ok_or_else(|| fail("Integer option outside bounds")),
    }
}
fn bounded_selections(value: Option<&Value>) -> Result<Vec<String>> {
    match value {
        None => Ok(vec![]),
        Some(value) => items(value, 128)?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| fail("Selections must be bounded ID lists"))
            })
            .collect(),
    }
}
pub fn create_private(path: &std::path::Path) -> Result<()> {
    let path = absolute(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| fail("New directory has no parent"))?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| fail("Invalid new directory name"))?;
    let mut custody = infra_core::bank::publication::PublicationFiles::new(parent)?;
    custody.create_directory(name)?;
    native::private_directory(&path)?;
    custody.check()
}
/// A session is titled by the first 80 characters of its prompt, so the
/// change and the launch lead: every launch of one run used to read the same.
fn launch_title(record: &Value, plan: &Value, retry: Option<&str>) -> String {
    let launches = texts(&record["launches"]);
    let previous = if launches.is_empty() {
        usize::from(record["session_id"].is_string())
    } else {
        launches.len()
    };
    let change = string(&plan["change_id"]);
    let task = string(&plan["context"]["task"]);
    match (previous, retry) {
        (0, _) => format!("System change {change}: {task}"),
        (_, Some(step)) => format!(
            "System change {change} · launch {} retries {step}: {task}",
            previous + 1
        ),
        _ => format!(
            "System change {change} · launch {} resumes: {task}",
            previous + 1
        ),
    }
}
#[cfg(test)]
mod launch_title_tests {
    use super::*;
    #[test]
    fn each_launch_of_a_run_is_titled_by_its_change_and_number() {
        let plan = object!({"change_id":"cancel-v3","context":{"task":"Allow cancellation"}});
        assert_eq!(
            launch_title(&object!({}), &plan, None),
            "System change cancel-v3: Allow cancellation"
        );
        let record = object!({"launches":["a","b"],"session_id":"b"});
        assert_eq!(
            launch_title(&record, &plan, None),
            "System change cancel-v3 · launch 3 resumes: Allow cancellation"
        );
        assert_eq!(
            launch_title(&record, &plan, Some("service-orders")),
            "System change cancel-v3 · launch 3 retries service-orders: Allow cancellation"
        );
        // A record from before launches were listed names only its session.
        assert_eq!(
            launch_title(&object!({"session_id":"a"}), &plan, None),
            "System change cancel-v3 · launch 2 resumes: Allow cancellation"
        );
    }
}
