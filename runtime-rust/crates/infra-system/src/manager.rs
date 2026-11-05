use crate::{
    backend::{SessionBridge, State},
    catalog::System,
    common::*,
    editor::Editor,
    execution, native,
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
    fn view(&self, record: &Value, detail: bool) -> Result<Value> {
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
            match read_json(&journal, name, MAX_BYTES) {
                Ok(value) => Ok(Some(value)),
                Err(error) if is_active || error.kind == infra_core::ErrorKind::NotFound => {
                    Ok(None)
                }
                Err(error) => Err(error),
            }
        };
        if let Some(state) = read("run.json")? {
            result["status"] = state["status"].clone();
            result["access"] = execution::access_scope(&state).into();
            if is_active {
                result["status"] = session.as_ref().unwrap()["status"].clone();
            } else if matches!(string(&result["status"]), "prepared" | "running") {
                result["status"] = "interrupted".into();
            }
            if detail {
                result["execution"] = execution::run_summary(&state, &journal);
                let mut receipts = vec![];
                for step in items(&state["steps"], MAX_SERVICES + 2)? {
                    if step["attempt"].as_u64().unwrap_or(0) > 0
                        && let Some(receipt) = read(&receipt_name(step))?
                    {
                        receipts.push(object!({"phase":step["id"],"service":step["service"],"attempt":step["attempt"],"ok":receipt["ok"],"error":receipt["error"],"rejection":receipt.get("rejection").cloned().unwrap_or(Value::Null),"report":receipt["report"]}));
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
                    .map(|sid| self
                        .state
                        .host
                        .session(sid)
                        .map(|session| object!({"session_id":sid,"status":session["status"]})))
                    .collect::<Result<Vec<_>>>()?
            );
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
        self.view(&record, true)
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
                "action", "revision", "mode", "timeout", "provider", "access",
            ],
            "resume" => vec!["action", "revision", "retry_step", "accept_source_changes"],
            "cancel" => vec!["action", "revision"],
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
        let view = self.view(&record, true)?;
        if action == "cancel" {
            if view["active"] == true {
                self.state.host.cancel(string(&record["session_id"]))?;
            }
            return self.view(&record, true);
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
            if data
                .get("accept_source_changes")
                .is_some_and(|value| !value.is_boolean())
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
        let nonce = id32();
        let request = object!({"directory":directory,"system_file":system.config,"allowed_roots":self.state.allowed()?,"provider":provider,"executable":executable,"mode":mode,"timeout":timeout,"access":access,"retry_step":data.get("retry_step"),"accept_source_changes":data["accept_source_changes"]==true});
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
        self.view(&record, true)
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
                    object!({"runs":self.state.db.all("system_runs",Some(project))?.into_iter().map(|(rid,record)|self.reconcile_record(&rid,record).and_then(|record|self.view(&record,false))).collect::<Result<Vec<_>>>()?}),
                )
            }
            "systems.get" => self.view(
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
