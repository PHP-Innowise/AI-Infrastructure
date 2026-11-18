//! Durable browser approvals around the native Creator worker.
use crate::{
    HarnessError, Result, creator_files as files, projects,
    services::{Domain, Payload, Reply},
    sessions::{Sessions, budgets},
    store::{self, SessionStore},
};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
};
pub struct CreatorManager {
    store: SessionStore,
    source: PathBuf,
    root: PathBuf,
    sessions: Weak<Sessions>,
    serial: Mutex<()>,
}
fn active(v: &Value) -> bool {
    ["scanning", "generating", "applying", "rolling_back"]
        .contains(&v["status"].as_str().unwrap_or(""))
}
fn rid(value: &str) -> Result<()> {
    if value.len() != 32
        || !value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(HarnessError::missing("Unknown Creator run."));
    }
    Ok(())
}
fn overlap(left: &Path, right: &Path) -> bool {
    let norm = |p: &Path| {
        let text = p.to_string_lossy().replace('\\', "/");
        if cfg!(windows) {
            text.to_lowercase()
        } else {
            text
        }
    };
    let (a, b) = (norm(left), norm(right));
    a == b || a.starts_with(&(b.clone() + "/")) || b.starts_with(&(a + "/"))
}
fn limited(value: &Value, max: usize, blank: bool) -> Result<String> {
    let text = value
        .as_str()
        .ok_or_else(|| HarnessError::invalid("Expected text."))?;
    if text.len() > max || text.contains('\0') || !blank && text.trim().is_empty() {
        return Err(HarnessError::invalid(
            "Text is blank or exceeds its byte bound.",
        ));
    }
    Ok(text.into())
}
fn routing(run: &mut Value, data: &Value) -> Result<()> {
    let r = &data["routing"];
    if r.is_null() {
        return Ok(());
    }
    if !store::exact(
        r,
        &[],
        &[
            "model",
            "thinking_effort",
            "agents_enabled",
            "agent_count",
            "permissions",
        ],
    ) {
        return Err(HarnessError::invalid("Invalid Creator routing."));
    }
    for key in ["model", "thinking_effort"] {
        if let Some(v) = r.get(key) {
            if !v.is_null() {
                limited(v, 200, false)?;
            }
            run[key] = v.clone();
        }
    }
    if let Some(v) = r.get("agents_enabled") {
        if !v.is_boolean() {
            return Err(HarnessError::invalid("Invalid additional-agent settings."));
        }
        run["agents_enabled"] = v.clone();
    }
    if let Some(v) = r.get("agent_count") {
        if v.as_u64().is_none_or(|n| !(1..=40).contains(&n)) {
            return Err(HarnessError::invalid("Invalid additional-agent settings."));
        }
        run["agent_count"] = v.clone();
    }
    if let Some(v) = r.get("permissions") {
        run["permissions"] = crate::permissions::validate(v)?;
    }
    Ok(())
}
impl CreatorManager {
    pub fn new(store: SessionStore, source: PathBuf, sessions: Weak<Sessions>) -> Result<Self> {
        let root = store.state.join("creator");
        infra_host::util::private_directory(&root)?;
        store.connection()?.execute_batch(
            "CREATE TABLE IF NOT EXISTS creator_runs(id TEXT PRIMARY KEY,data TEXT NOT NULL);",
        )?;
        Ok(Self {
            store,
            source,
            root,
            sessions,
            serial: Mutex::new(()),
        })
    }
    fn sessions(&self) -> Result<Arc<Sessions>> {
        self.sessions
            .upgrade()
            .ok_or_else(|| HarnessError::unavailable("The session runner is stopping."))
    }
    fn save(&self, run: &Value) -> Result<()> {
        self.store.connection()?.execute("INSERT INTO creator_runs(id,data)VALUES(?1,?2)ON CONFLICT(id)DO UPDATE SET data=excluded.data",params![run["id"].as_str(),run.to_string()])?;
        Ok(())
    }
    fn load(&self, id: &str) -> Result<Value> {
        rid(id)?;
        let raw: Option<String> = self
            .store
            .connection()?
            .query_row("SELECT data FROM creator_runs WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()?;
        let mut run = crate::strict_json::parse(
            raw.ok_or_else(|| HarnessError::missing("Unknown Creator run."))?
                .as_bytes(),
            2 * 1024 * 1024,
        )?;
        let directory = self.root.join(id);
        let sid = run["session_id"].as_str().map(str::to_owned);
        if active(&run)
            && let Some(sid) = sid.as_deref()
        {
            let session = self.store.get(sid)?;
            if !["queued", "running"].contains(&session["status"].as_str().unwrap_or("")) {
                let result = run["result_file"]
                    .as_str()
                    .map(|name| files::object(&directory, name, false))
                    .transpose()?
                    .flatten()
                    .unwrap_or(json!({}));
                let status = if session["status"] == "completed"
                    && [
                        "review",
                        "preview",
                        "needs_input",
                        "complete",
                        "rolled_back",
                        "failed",
                    ]
                    .contains(&result["status"].as_str().unwrap_or(""))
                {
                    result["status"].clone()
                } else if session["status"] == "completed" {
                    json!("failed")
                } else {
                    session["status"].clone()
                };
                run["status"] = status;
                run["message"] = result
                    .get("message")
                    .cloned()
                    .unwrap_or(json!("Inspect the session events and retry this phase."));
                run["revision"] = json!(run["revision"].as_u64().unwrap_or(0) + 1);
                self.save(&run)?;
            }
        }
        run["recovery_available"] = false.into();
        if !["applying", "rolling_back"].contains(&run["status"].as_str().unwrap_or("")) {
            let journal = directory.join("journal");
            if journal.is_dir() {
                let receipt = files::object(&journal, "journal.json", false);
                let safe = receipt.ok().flatten().is_some_and(|v| {
                    ["verified", "rolled-back"].contains(&v["status"].as_str().unwrap_or(""))
                });
                run["recovery_available"] = (!safe).into();
            }
        }
        Ok(run)
    }
    fn view(&self, run: Value) -> Result<Value> {
        let id = run["id"].as_str().unwrap_or("").to_owned();
        let task = self.root.join(&id).join("agent").join(files::TASK);
        let mut reports = vec![];
        for name in files::REPORTS {
            let captured = infra_core::bank::publication::PublicationFiles::new(&task)
                .and_then(|mut fs| fs.read_prefix(name, 192000));
            match captured{
   Ok(Some((total,bytes)))=>reports.push(json!({"name":name,"text":String::from_utf8_lossy(&bytes),"truncated":total>bytes.len()as u64,"hash":files::read(&task,name,true,files::LIMIT).ok().flatten().map(|b|store::sha(&b))})),
   Ok(None)=>{},Err(_)if ["scanning","generating"].contains(&run["status"].as_str().unwrap_or(""))=>{},
   Err(_)=>reports.push(json!({"name":name,"text":"Report is missing, changing, or unsafe to read.","truncated":false}))
  }
        }
        let sid = run["session_id"].as_str();
        let session = sid.map(|id| self.store.get(id)).transpose()?;
        let events = sid
            .map(|id| self.store.events(id, 0))
            .transpose()?
            .unwrap_or_default();
        let waiting_permissions = sid
            .map(|id| {
                self.sessions()
                    .map(|s| s.waiting_permissions(id))
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        let mut value = json!({
            "run": run,
            "reports": reports,
            "session": session,
            "events": events,
            "waiting_permissions": waiting_permissions,
        });
        if value["run"]["status"] == "preview" {
            value["preview"] = files::preview(&value["run"], &self.root.join(&id))?;
        }
        Ok(value)
    }
    pub fn get(&self, id: &str) -> Result<Value> {
        let _g = self
            .serial
            .lock()
            .map_err(|_| HarnessError::unavailable("Creator state lock failed."))?;
        self.view(self.load(id)?)
    }
    pub fn list(&self, project: &str) -> Result<Value> {
        self.store.project(project)?;
        let _g = self
            .serial
            .lock()
            .map_err(|_| HarnessError::unavailable("Creator state lock failed."))?;
        let ids = {
            let db = self.store.connection()?;
            let mut q = db.prepare("SELECT id FROM creator_runs ORDER BY rowid DESC LIMIT 200")?;
            q.query_map([], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        let mut runs = vec![];
        for id in ids {
            let run = self.load(&id)?;
            if run["project_id"] == project {
                runs.push(run);
            }
        }
        Ok(json!({"runs":runs}))
    }
    fn launch(&self, mut run: Value, phase: &str) -> Result<Value> {
        let sessions = self.sessions()?;
        sessions.admit()?;
        let provider = run["provider"].as_str().unwrap_or("");
        infra_host::model_catalog::validate_model_effort(
            &sessions.providers.catalog(provider)?,
            provider,
            run["model"].as_str(),
            run["thinking_effort"].as_str(),
            run["agents_enabled"] == true,
        )?;
        let nonce = store::hexid();
        let id = run["id"]
            .as_str()
            .ok_or_else(|| HarnessError::invalid("Creator identity missing."))?
            .to_owned();
        let directory = self.root.join(&id);
        if phase == "apply" && directory.join("journal").exists() {
            let old = files::object(&directory.join("journal"), "journal.json", true)?.unwrap();
            if old["status"] != "rolled-back" {
                return Err(HarnessError::conflict(
                    "Recover the previous publication before applying again.",
                ));
            }
            std::fs::rename(
                directory.join("journal"),
                directory.join(format!("journal-{nonce}")),
            )?;
        }
        run.as_object_mut().unwrap().remove("message");
        run["phase"] = phase.into();
        run["result_file"] = format!("result-{nonce}.json").into();
        run["status"] = match phase {
            "scan" => "scanning",
            "generate" => "generating",
            "apply" => "applying",
            "rollback" => "rolling_back",
            _ => return Err(HarnessError::invalid("Invalid Creator phase.")),
        }
        .into();
        let mut request = run.clone();
        request["directory"] = directory.to_string_lossy().into_owned().into();
        request["executable"] = serde_json::to_value(
            sessions
                .providers
                .get(run["provider"].as_str().unwrap_or(""))?
                .executable
                .ok_or_else(|| HarnessError::unavailable("Native executable is unavailable."))?,
        )?;
        request["protected_roots"] = json!([
            self.store
                .project(run["project_id"].as_str().unwrap_or(""))?["path"],
            self.store.state,
            self.source
        ]);
        if cfg!(windows) {
            request["sandbox_executable"] =
                serde_json::to_value(sessions.providers.get("codex")?.executable)?;
        }
        files::write(&directory, &format!("request-{nonce}.json"), &request)?;
        let session = sessions.launch_internal(
            &json!({
                "project_id": run["project_id"],
                "provider": run["provider"],
                "prompt": format!(
                    "Creator {phase}: {}",
                    Path::new(run["target"].as_str().unwrap_or(""))
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                ),
                "mode": "edit",
                "workflow": "native",
                "model": run["model"],
                "thinking_effort": run["thinking_effort"],
                "agents_enabled": run["agents_enabled"],
                "agent_count": run["agent_count"],
                "permissions": run["permissions"],
                "budgets": if ["scan", "generate"].contains(&phase) {
                    run["budgets"].clone()
                } else {
                    json!({})
                }
            }),
            Some(&json!({"run_id": id, "nonce": nonce, "phase": phase})),
            None,
            None,
        )?;
        run["session_id"] = session["id"].clone();
        self.save(&run)?;
        self.view(run)
    }
    pub fn start(&self, data: &Value) -> Result<Value> {
        if !store::exact(
            data,
            &["project_id", "provider", "tools"],
            &[
                "model",
                "thinking_effort",
                "agents_enabled",
                "agent_count",
                "operation",
                "workspace",
                "worktree_branch",
                "goal",
                "budgets",
                "permissions",
            ],
        ) {
            return Err(HarnessError::invalid("Invalid Creator options."));
        }
        let permissions = crate::permissions::validate(&data["permissions"])?;
        let sessions = self.sessions()?;
        let project = self
            .store
            .project(data["project_id"].as_str().unwrap_or(""))?;
        let provider = data["provider"]
            .as_str()
            .ok_or_else(|| HarnessError::invalid("Choose an available provider."))?;
        let native = sessions.providers.get(provider)?;
        if !native.available {
            return Err(HarnessError::invalid("Choose an available provider."));
        }
        let tools = files::texts(&data["tools"])?;
        if tools.is_empty()
            || tools
                .iter()
                .any(|v| !["claude", "codex", "cursor"].contains(&v.as_str()))
            || tools.iter().collect::<BTreeSet<_>>().len() != tools.len()
        {
            return Err(HarnessError::invalid("Choose distinct target tools."));
        }
        let operation = data["operation"].as_str().unwrap_or("generate");
        if !["generate", "update"].contains(&operation) {
            return Err(HarnessError::invalid("Choose generation or update."));
        }
        let goal = data
            .get("goal")
            .map(|v| limited(v, 8000, true))
            .transpose()?
            .unwrap_or_default();
        let enabled = data["agents_enabled"].as_bool().unwrap_or(false);
        let count = data["agent_count"].as_u64().unwrap_or(3);
        if data.get("agents_enabled").is_some_and(|v| !v.is_boolean()) || !(1..=40).contains(&count)
        {
            return Err(HarnessError::invalid("Invalid additional-agent settings."));
        }
        let source =
            infra_core::fs::canonicalize(Path::new(project["path"].as_str().unwrap_or("")))?;
        for boundary in [&self.source, &self.store.state] {
            if overlap(&source, boundary) {
                return Err(HarnessError::forbidden(
                    "Choose a target outside the Harness source and runner state.",
                ));
            }
        }
        let php = source.join("composer.json").is_file()
            || ["", "src", "app"].iter().any(|part| {
                std::fs::read_dir(source.join(part))
                    .ok()
                    .is_some_and(|entries| {
                        entries
                            .filter_map(|e| e.ok())
                            .take(20000)
                            .any(|e| e.path().extension().is_some_and(|x| x == "php"))
                    })
            });
        if !php {
            return Err(HarnessError::invalid(
                "No PHP project entry point detected. Use stack adaptation separately for a non-PHP target.",
            ));
        }
        let _g = self
            .serial
            .lock()
            .map_err(|_| HarnessError::unavailable("Creator state lock failed."))?;
        sessions.admit()?;
        let id = store::hexid();
        let (target, branch, common) = projects::workspace(&self.store, &project, data, &id)?;
        let identity = files::identity(&target)?;
        if operation == "update"
            && files::read(&target, ".infra-manifest.json", false, 4 * 1024 * 1024)?.is_none()
        {
            return Err(HarnessError::invalid(
                "Update requires the existing .infra-manifest.json; no ownership is inferred.",
            ));
        }
        let directory = self.root.join(&id);
        infra_host::util::private_directory(&directory)?;
        let agent = directory.join("agent");
        let mut engine = files::admit_engine(&std::env::current_exe()?)?;
        files::copy_generator(
            &self.source.join("Infrastructure-Creator"),
            &agent,
            &mut engine,
        )?;
        let spec = crate::creator_runner::sandbox_spec(
            &json!({"target":target,"directory":directory,"provider":provider,"executable":native.executable,"protected_roots":[self.source,self.store.state,common],"sandbox_executable":if cfg!(windows){json!(sessions.providers.get("codex")?.executable)}else{Value::Null}}),
        )?;
        crate::sandbox::creator(
            &spec,
            &[
                std::env::current_exe()?.to_string_lossy().into_owned(),
                "version".into(),
            ],
        )?;
        let mut run = json!({
            "id": id,
            "project_id": project["id"],
            "target": target,
            "identity": identity,
            "workspace": data["workspace"].as_str().unwrap_or("project"),
            "branch": branch,
            "operation": operation,
            "tools": tools,
            "provider": provider,
            "model": data["model"],
            "thinking_effort": data["thinking_effort"],
            "agents_enabled": enabled,
            "agent_count": count,
            "permissions": permissions,
            "budgets": budgets(data.get("budgets").unwrap_or(&json!({})), provider, false)?,
            "goal": goal,
            "created_at": store::now(),
            "revision": 1,
            "status": "new",
            "answers": "",
            "phase": "scan"
        });
        let next_routing = json!({
            "routing": {
                "model": run["model"],
                "thinking_effort": run["thinking_effort"],
                "agents_enabled": enabled,
                "agent_count": count,
                "permissions": run["permissions"]
            }
        });
        routing(&mut run, &next_routing)?;
        self.launch(run, "scan")
    }
    pub fn act(&self, id: &str, data: &Value) -> Result<Value> {
        if !store::exact(
            data,
            &["action"],
            &[
                "revision",
                "answers",
                "preview_id",
                "replace",
                "budgets",
                "routing",
            ],
        ) {
            return Err(HarnessError::invalid("Invalid Creator action."));
        }
        let _g = self
            .serial
            .lock()
            .map_err(|_| HarnessError::unavailable("Creator state lock failed."))?;
        let mut run = self.load(id)?;
        let action = data["action"].as_str().unwrap_or("");
        if action == "cancel" {
            if ["applying", "rolling_back"].contains(&run["status"].as_str().unwrap_or("")) {
                return Err(HarnessError::conflict(
                    "Publication is completing verification or rollback.",
                ));
            }
            if ["scanning", "generating"].contains(&run["status"].as_str().unwrap_or("")) {
                self.sessions()?
                    .cancel(run["session_id"].as_str().unwrap_or(""))?;
            }
            return self.view(self.load(id)?);
        }
        if data["revision"].as_u64() != run["revision"].as_u64() {
            return Err(HarnessError::conflict(
                "This Creator view is stale. Refresh before continuing.",
            ));
        }
        if active(&run) {
            return Err(HarnessError::conflict(
                "Wait for the current phase to finish.",
            ));
        }
        let revision = run["revision"].as_u64().unwrap_or(0) + 1;
        run["revision"] = revision.into();
        if action == "rollback" && run["recovery_available"] == true {
            return self.launch(run, "rollback");
        }
        if run["recovery_available"] == true {
            return Err(HarnessError::conflict(
                "Recover the interrupted publication before continuing.",
            ));
        }
        if action == "budgets" {
            if !store::exact(data, &["action", "revision", "budgets"], &[]) {
                return Err(HarnessError::invalid(
                    "Provide only budgets and the current revision.",
                ));
            }
            run["budgets"] = budgets(
                &data["budgets"],
                run["provider"].as_str().unwrap_or(""),
                false,
            )?;
            self.save(&run)?;
            return self.view(run);
        }
        if data.get("budgets").is_some() {
            return Err(HarnessError::invalid(
                "Save budgets before starting a phase.",
            ));
        }
        if action == "apply" && run["status"] == "preview" {
            let preview = files::preview(&run, &self.root.join(id))?;
            let replacements = files::texts(data.get("replace").unwrap_or(&json!([])))?;
            let expected = preview["files"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|v| v["requires_decision"] == true)
                .filter_map(|v| v["path"].as_str())
                .collect::<BTreeSet<_>>();
            if preview["preview_id"] != data["preview_id"]
                || replacements.len() != replacements.iter().collect::<BTreeSet<_>>().len()
                || replacements
                    .iter()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>()
                    != expected
            {
                return Err(HarnessError::conflict(
                    "Review the current diff and explicitly approve each replacement.",
                ));
            }
            if preview["can_apply"] != true {
                return Err(HarnessError::conflict(
                    "Publication is blocked by reported conflicts.",
                ));
            }
            files::write(&self.root.join(id), "approved-preview.json", &preview)?;
            return self.launch(run, "apply");
        }
        if action == "generate" && run["status"] == "review" {
            routing(&mut run, data)?;
            run["approved"] = files::hashes(&self.root.join(id))?;
            return self.launch(run, "generate");
        }
        if action == "rescan" && run["status"] != "complete" {
            routing(&mut run, data)?;
            run.as_object_mut().unwrap().remove("approved");
            run["answers"] = data
                .get("answers")
                .map(|v| limited(v, 16000, true))
                .transpose()?
                .unwrap_or_default()
                .into();
            return self.launch(run, "scan");
        }
        if action == "revise"
            && [
                "review",
                "needs_input",
                "preview",
                "failed",
                "interrupted",
                "cancelled",
                "rolled_back",
            ]
            .contains(&run["status"].as_str().unwrap_or(""))
        {
            routing(&mut run, data)?;
            run["answers"] = limited(&data["answers"], 16000, false)?.into();
            let phase = if run["phase"] == "scan" || run["approved"].is_null() {
                "scan"
            } else {
                "generate"
            };
            return self.launch(run, phase);
        }
        Err(HarnessError::conflict(
            "This action is not available in the current Creator phase.",
        ))
    }
}
impl Domain for CreatorManager {
    fn execute<'a>(&'a self, op: &'a str, request: Value) -> Reply<'a> {
        Box::pin(async move {
            let result = match op {
                "creator.list" => {
                    self.list(request["query"]["project_id"].as_str().unwrap_or(""))?
                }
                "creator.get" => self.get(request["params"]["id"].as_str().unwrap_or(""))?,
                "creator.start" => self.start(&request["body"])?,
                "creator.act" => self.act(
                    request["params"]["id"].as_str().unwrap_or(""),
                    &request["body"],
                )?,
                _ => return Err(HarnessError::missing("Unknown Creator operation.")),
            };
            Ok(Payload::Json(result))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accelerators::Accelerators,
        bindings::{ResultsBridge, SessionLinks},
        commands::CommandCatalog,
        knowledge::KnowledgeDomain,
        provider::{Provider, ProviderDomain},
        results::Results,
        sessions::{BlackboardFactory, SessionLimits, SpecialWorkflow},
    };
    use std::{
        collections::BTreeMap,
        sync::{RwLock, atomic::AtomicBool},
    };

    struct Unused;
    impl SpecialWorkflow for Unused {
        fn run<'a>(
            &'a self,
            _session: Value,
            _prompt: String,
            _generation: String,
            _cancel: Arc<AtomicBool>,
            _emit: crate::sessions::Event,
        ) -> crate::sessions::RunReply<'a> {
            Box::pin(async { Ok(json!({})) })
        }
    }
    impl BlackboardFactory for Unused {
        fn for_session(&self, _: &Value) -> Result<Arc<dyn crate::fleet::Blackboard>> {
            unreachable!()
        }
    }

    async fn test_harness() -> (tempfile::TempDir, Arc<Sessions>, CreatorManager, String) {
        static CATALOG: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
        CATALOG
            .get_or_init(|| async {
                for id in ["claude", "codex", "cursor"] {
                    infra_host::model_catalog::model_options(id, None, None)
                        .await
                        .unwrap();
                }
            })
            .await;
        let dir = tempfile::tempdir().unwrap();
        let root = infra_core::fs::canonicalize(dir.path()).unwrap();
        let target = root.join("target-php");
        let source = root.join("source");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(target.join("composer.json"), "{}").unwrap();
        let store = SessionStore::open(&root.join("state")).unwrap();
        let project_id = store.register(&target).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let providers = Arc::new(ProviderDomain {
            providers: Arc::new(RwLock::new(BTreeMap::from([(
                "claude".into(),
                Provider {
                    id: "claude".into(),
                    name: "Fake Claude".into(),
                    available: true,
                    executable: Some(PathBuf::from("/bin/echo")),
                    detail: "test".into(),
                },
            )]))),
        });
        let knowledge = Arc::new(KnowledgeDomain::new(store.clone(), source.clone()));
        let special = ["creator", "system_run", "system_discovery"]
            .into_iter()
            .map(|name| (name.into(), Arc::new(Unused) as Arc<dyn SpecialWorkflow>))
            .collect();
        let sessions = Sessions::new(
            store.clone(),
            source.clone(),
            providers.clone(),
            knowledge.clone(),
            Arc::new(Unused),
            special,
            SessionLimits {
                seconds: 30,
                concurrency: 1,
            },
        )
        .unwrap();
        let links = Arc::new(SessionLinks::new());
        links.bind(Arc::downgrade(&sessions)).unwrap();
        let results = Arc::new(
            Results::new(
                store.clone(),
                Arc::new(ResultsBridge {
                    links,
                    knowledge,
                    operation: sessions.operation_lock(),
                    cancel: sessions.shutdown_cancel(),
                }),
            )
            .unwrap(),
        );
        sessions.install_results(results).unwrap();
        sessions
            .install_commands(Arc::new(CommandCatalog::new(
                store.clone(),
                source.clone(),
                providers.providers.clone(),
            )))
            .unwrap();
        sessions
            .install_accelerators(Arc::new(
                Accelerators::new(
                    store.clone(),
                    source.clone(),
                    providers,
                    Arc::downgrade(&sessions),
                )
                .unwrap(),
            ))
            .unwrap();
        sessions.activate().unwrap();
        let manager = CreatorManager::new(store, source, Arc::downgrade(&sessions)).unwrap();
        (dir, sessions, manager, project_id)
    }

    #[test]
    fn routing_validates_and_updates_permissions() {
        let mut run = json!({"permissions": "auto"});
        routing(&mut run, &json!({"routing": {"permissions": "manual"}})).unwrap();
        assert_eq!(run["permissions"], "manual");

        routing(&mut run, &json!({"routing": {"permissions": "bypass"}})).unwrap();
        assert_eq!(run["permissions"], "bypass");

        routing(&mut run, &json!({"routing": {"permissions": null}})).unwrap();
        assert_eq!(run["permissions"], Value::Null);

        let err = routing(&mut run, &json!({"routing": {"permissions": "invalid"}})).unwrap_err();
        assert_eq!(err.status, 400);
        assert!(
            err.message.contains("Auto, Manual or Bypass"),
            "{}",
            err.message
        );
    }

    #[tokio::test]
    async fn start_rejects_invalid_permissions_and_launch_act_view_preserve_permissions() {
        let (_dir, sessions, manager, project_id) = test_harness().await;
        let err = manager
            .start(&json!({
                "project_id": project_id,
                "provider": "claude",
                "tools": ["claude"],
                "permissions": "bogus"
            }))
            .unwrap_err();
        assert_eq!(err.status, 400);
        assert!(
            err.message.contains("Auto, Manual or Bypass"),
            "{}",
            err.message
        );

        let target = manager.store.project(&project_id).unwrap()["path"]
            .as_str()
            .unwrap()
            .to_owned();
        let id = store::hexid();
        let run_dir = manager.root.join(&id);
        std::fs::create_dir_all(run_dir.join("agent").join(files::TASK)).unwrap();
        let identity = files::identity(Path::new(&target)).unwrap();
        let run = json!({
            "id": id,
            "project_id": project_id,
            "target": target,
            "identity": identity,
            "workspace": "project",
            "branch": Value::Null,
            "operation": "generate",
            "tools": ["claude"],
            "provider": "claude",
            "model": Value::Null,
            "thinking_effort": Value::Null,
            "agents_enabled": false,
            "agent_count": 3,
            "permissions": "auto",
            "budgets": {"usd": Value::Null, "tokens": Value::Null, "seconds": 120},
            "goal": "goal",
            "created_at": store::now(),
            "revision": 1,
            "status": "new",
            "answers": "",
            "phase": "scan"
        });
        let launched = manager.launch(run, "scan").unwrap();
        assert_eq!(launched["run"]["permissions"], "auto");
        assert_eq!(launched["session"]["permissions"], "auto");
        assert_eq!(launched["waiting_permissions"], json!([]));

        let nonce = launched["session"]["creator"]["nonce"].as_str().unwrap();
        let req = files::object(&run_dir, &format!("request-{nonce}.json"), true)
            .unwrap()
            .unwrap();
        assert_eq!(req["permissions"], "auto");

        // While a permission request is pending on the backing session, view exposes waiting_permissions.
        let sid = launched["session"]["id"].as_str().unwrap().to_owned();
        let emit: crate::sessions::Event = Arc::new(|_| Ok(()));
        let approvals = sessions
            .creator_approvals(&sid, &json!({"permissions": "manual"}), &emit)
            .unwrap();
        let rx = approvals
            .ask_answer(&infra_host::permission::PermissionAsk {
                id: "creator-1".into(),
                provider: "claude".into(),
                tool: "Bash".into(),
                summary: "Run php -v".into(),
                kind: "command".into(),
                ..Default::default()
            })
            .unwrap();
        let viewed = manager.get(&id).unwrap();
        assert_eq!(viewed["waiting_permissions"].as_array().unwrap().len(), 1);
        assert_eq!(viewed["waiting_permissions"][0]["tool"], "Bash");
        sessions.expire_permissions(&sid);
        assert!(rx.await.is_err());

        // Move run to review and test rescan/revise/generate with routing.permissions.
        let mut stored = manager.load(&id).unwrap();
        stored["status"] = json!("review");
        stored["revision"] = json!(2);
        manager.save(&stored).unwrap();
        let rescanned = manager
            .act(
                &id,
                &json!({
                    "action": "rescan",
                    "revision": 2,
                    "answers": "",
                    "routing": {"permissions": "manual"}
                }),
            )
            .unwrap();
        assert_eq!(rescanned["run"]["permissions"], "manual");
        assert_eq!(rescanned["session"]["permissions"], "manual");
        let nonce2 = rescanned["session"]["creator"]["nonce"].as_str().unwrap();
        let req2 = files::object(&run_dir, &format!("request-{nonce2}.json"), true)
            .unwrap()
            .unwrap();
        assert_eq!(req2["permissions"], "manual");

        let mut stored = manager.load(&id).unwrap();
        stored["status"] = json!("needs_input");
        stored["revision"] = json!(4);
        manager.save(&stored).unwrap();
        let revised = manager
            .act(
                &id,
                &json!({
                    "action": "revise",
                    "revision": 4,
                    "answers": "Use bypass",
                    "routing": {"permissions": "bypass"}
                }),
            )
            .unwrap();
        assert_eq!(revised["run"]["permissions"], "bypass");
        assert_eq!(revised["session"]["permissions"], "bypass");

        // Write required report files to satisfy hashes() for generate.
        let task_dir = run_dir.join("agent").join(files::TASK);
        for name in &files::REPORTS[..3] {
            std::fs::write(task_dir.join(name), "{}").unwrap();
        }
        let mut stored = manager.load(&id).unwrap();
        stored["status"] = json!("review");
        stored["revision"] = json!(6);
        manager.save(&stored).unwrap();
        let generated = manager
            .act(
                &id,
                &json!({
                    "action": "generate",
                    "revision": 6,
                    "routing": {"permissions": "manual"}
                }),
            )
            .unwrap();
        assert_eq!(generated["run"]["permissions"], "manual");
        assert_eq!(generated["session"]["permissions"], "manual");
        let nonce3 = generated["session"]["creator"]["nonce"].as_str().unwrap();
        let req3 = files::object(&run_dir, &format!("request-{nonce3}.json"), true)
            .unwrap()
            .unwrap();
        assert_eq!(req3["permissions"], "manual");

        let _ = sessions.close();
        let _ = sessions.wait_closed().await;
    }
}
