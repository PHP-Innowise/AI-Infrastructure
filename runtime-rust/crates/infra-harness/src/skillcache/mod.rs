//! SkillCache in the Harness.
//!
//! A session opts in with `skillcache: {"mode": ...}`; the default is `off`
//! and then nothing here runs. `observe` and `auto` keep the observed trace of
//! a cold turn, which stores tool results the Harness otherwise never stores:
//! that is why it is an explicit choice per session. The capsule's own
//! contracts live in the `infra-skillcache` crate.
pub mod compile;
pub(crate) mod db;
pub mod gates;
pub mod settings;
pub mod snapshot;
pub mod trace;
pub mod warm;

use crate::{
    HarnessError, Result,
    provider::ProviderDomain,
    services::{Domain, Payload, Reply},
    store::SessionStore,
};
use compile::{Job, Runtime};
use infra_skillcache::{compile_rules, fingerprint, package};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

/// Compilations and requalifications waiting for their turn. They run one at
/// a time, outside the session queue.
const WAITING: usize = 8;

enum Work {
    Compile(Job),
    Requalify { capsule_id: String },
}

/// The HTTP operations of SkillCache and the queue behind two of them.
pub struct SkillCacheDomain {
    runtime: Arc<Runtime>,
    sender: mpsc::Sender<Work>,
    receiver: Mutex<Option<mpsc::Receiver<Work>>>,
}

/// An installed runtime's version, remembered for a minute: the identity of
/// the environment is asked for on every listing and every warm decision.
fn runtime_versions(names: &[String]) -> BTreeMap<String, Option<String>> {
    static KNOWN: Mutex<BTreeMap<String, (Instant, Option<String>)>> = Mutex::new(BTreeMap::new());
    let mut known = KNOWN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    names
        .iter()
        .map(|name| {
            let fresh = known
                .get(name)
                .filter(|(at, _)| at.elapsed() < Duration::from_secs(60))
                .map(|(_, version)| version.clone());
            let version = fresh.unwrap_or_else(|| {
                let version = fingerprint::runtime_version(name);
                known.insert(name.clone(), (Instant::now(), version.clone()));
                version
            });
            (name.clone(), version)
        })
        .collect()
}

/// Why an active capsule may no longer be used in `workspace`, if it may not:
/// its skill, its environment, its rules or its package is not what it was
/// qualified with.
pub fn staleness(runtime: &Runtime, capsule: &Value, workspace: &Path) -> Option<String> {
    if capsule["schema_version"].as_u64() != Some(infra_skillcache::SCHEMA_VERSION) {
        return Some("The capsule was written in another schema version.".into());
    }
    if capsule["compile_rules_version"].as_str() != Some(compile_rules::version().as_str()) {
        return Some("The compiler's rules changed since this capsule was compiled.".into());
    }
    for skill in capsule["skill_files"]["skills"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let name = skill["name"].as_str().unwrap_or("");
        let Some(folder) = skill["path"]
            .as_str()
            .and_then(|path| trace::skill_file(workspace, path))
            .and_then(|file| file.parent().map(Path::to_path_buf))
        else {
            return Some(format!("The skill `{name}` cannot be located."));
        };
        let known = fingerprint::Tree {
            sha256: skill["tree_sha256"].as_str().unwrap_or("").into(),
            taken_ns: skill["taken_ns"]
                .as_str()
                .and_then(|text| text.parse().ok())
                .unwrap_or_default(),
            files: skill["files"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|file| {
                    Some(fingerprint::FileStat {
                        path: file["path"].as_str()?.into(),
                        size: file["size"].as_u64()?,
                        mtime_ns: file["mtime_ns"].as_str()?.parse().ok()?,
                    })
                })
                .collect(),
        };
        match fingerprint::verify_tree(&folder, &known) {
            Ok((true, _)) => {}
            Ok((false, _)) => {
                return Some(format!(
                    "The skill `{name}` changed since this capsule was compiled."
                ));
            }
            Err(_) => return Some(format!("The skill `{name}` is not readable.")),
        }
    }
    let runtimes = capsule["capabilities"]["runtimes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|name| name.as_str().map(String::from))
        .collect::<Vec<_>>();
    let environment =
        fingerprint::env_fingerprint(&runtime.engine_digest(), &runtime_versions(&runtimes));
    if capsule["env_fingerprint"].as_str() != Some(environment.as_str()) {
        return Some(
            "The engine or a runtime the capsule's programs use changed; requalify it.".into(),
        );
    }
    let warm = PathBuf::from(capsule["artifact_dir"].as_str().unwrap_or("")).join("warm");
    match package::digest(&warm) {
        Ok(digest) if capsule["package_sha256"].as_str() == Some(digest.as_str()) => None,
        _ => Some("The capsule's package is missing or was changed.".into()),
    }
}

impl SkillCacheDomain {
    pub fn new(
        store: SessionStore,
        providers: Arc<ProviderDomain>,
        cancel: Arc<AtomicBool>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel(WAITING);
        Self {
            runtime: Arc::new(Runtime {
                store,
                providers,
                engine: None,
                cancel,
            }),
            sender,
            receiver: Mutex::new(Some(receiver)),
        }
    }
    /// Names the engine the gates run, for a process that is not one itself.
    pub fn with_engine(mut self, engine: PathBuf) -> Self {
        let runtime = &self.runtime;
        self.runtime = Arc::new(Runtime {
            store: runtime.store.clone(),
            providers: runtime.providers.clone(),
            engine: Some(engine),
            cancel: runtime.cancel.clone(),
        });
        self
    }
    pub fn runtime(&self) -> Arc<Runtime> {
        self.runtime.clone()
    }
    /// Starts the one worker the first time work is queued.
    fn start(&self) {
        let Some(mut receiver) = self.receiver.lock().ok().and_then(|mut slot| slot.take()) else {
            return;
        };
        let runtime = self.runtime.clone();
        tokio::spawn(async move {
            while let Some(work) = receiver.recv().await {
                if runtime.cancel.load(Ordering::Acquire) {
                    break;
                }
                match work {
                    Work::Compile(job) => compile::run(&runtime, &job).await,
                    Work::Requalify { capsule_id } => {
                        let capsule = runtime
                            .store
                            .connection()
                            .and_then(|db| db::capsule(&db, &capsule_id));
                        if let Ok(Some(capsule)) = capsule
                            && let Ok(workspace) = project_path(&runtime.store, &capsule)
                            && let Err(error) =
                                compile::requalify(&runtime, &capsule, &workspace).await
                        {
                            if let Ok(db) = runtime.store.connection() {
                                let _ = db::set_capsule_status(
                                    &db,
                                    &capsule_id,
                                    "stale",
                                    Some(&error.to_string()),
                                );
                            }
                            runtime.publish(
                                capsule["project_id"].as_str().unwrap_or(""),
                                "failed",
                                json!({"capsule_id": capsule_id, "outcome": error.to_string()}),
                            );
                        }
                    }
                }
            }
        });
    }
    fn queue(&self, work: Work) -> Result<()> {
        self.start();
        self.sender.try_send(work).map_err(|_| {
            HarnessError::conflict(
                "The SkillCache queue is full; try again when a compilation has finished.",
            )
        })
    }
    /// A capsule row as it is now: an active capsule that no longer matches
    /// what it was qualified with is marked stale before it is shown.
    fn current(&self, capsule: Value) -> Result<Value> {
        if capsule["status"] != "active" {
            return Ok(capsule);
        }
        let Ok(workspace) = project_path(&self.runtime.store, &capsule) else {
            return Ok(capsule);
        };
        let Some(reason) = staleness(&self.runtime, &capsule, &workspace) else {
            return Ok(capsule);
        };
        let id = capsule["capsule_id"].as_str().unwrap_or("").to_string();
        let db = self.runtime.store.connection()?;
        db::set_capsule_status(&db, &id, "stale", Some(&reason))?;
        let updated = db::capsule(&db, &id)?.unwrap_or(capsule);
        drop(db);
        self.runtime.publish(
            updated["project_id"].as_str().unwrap_or(""),
            "stale",
            json!({"capsule_id": id, "reason": reason}),
        );
        Ok(updated)
    }
    fn owned(&self, project_id: &str, capsule_id: &str) -> Result<Value> {
        db::capsule(&*self.runtime.store.connection()?, capsule_id)?
            .filter(|capsule| capsule["project_id"] == project_id)
            .ok_or_else(|| HarnessError::missing("Capsule not found."))
    }
    fn list(&self, project_id: &str, query: &Value) -> Result<Value> {
        self.runtime.store.project(project_id)?;
        let filter = |name: &str| query[name].as_str().filter(|value| !value.is_empty());
        let rows = db::capsules(
            &*self.runtime.store.connection()?,
            project_id,
            filter("skill"),
            None,
            filter("form"),
        )?;
        let mut capsules = Vec::new();
        for row in rows {
            let row = self.current(row)?;
            if filter("status").is_none_or(|status| row["status"] == status) {
                capsules.push(summary(&row));
            }
        }
        Ok(json!({"project_id": project_id, "capsules": capsules}))
    }
    fn get(&self, project_id: &str, capsule_id: &str) -> Result<Value> {
        let capsule = self.current(self.owned(project_id, capsule_id)?)?;
        let warm = PathBuf::from(capsule["artifact_dir"].as_str().unwrap_or("")).join("warm");
        let stored = package::load(&warm).ok();
        let db = self.runtime.store.connection()?;
        let compilation = match capsule["compile_id"].as_str() {
            Some(id) => db::compile(&db, id)?,
            None => None,
        };
        let (runs, deopts): (i64, i64) = db.query_row(
            "SELECT COUNT(*),COALESCE(SUM(deopts),0) FROM skillcache_runs WHERE capsule_id=?1",
            [capsule_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let mut detail = summary(&capsule);
        // The plan as a person inspects it. Program sources stay in the package.
        detail["capsule"] = stored
            .as_ref()
            .map(|stored| stored.capsule.to_value())
            .unwrap_or(Value::Null);
        detail["markdown"] = stored
            .as_ref()
            .and_then(|stored| stored.markdown.clone())
            .map_or(Value::Null, Value::from);
        detail["files"] = stored
            .as_ref()
            .map(|stored| {
                stored
                    .files
                    .iter()
                    .map(|(path, content)| json!({"path": path, "bytes": content.len()}))
                    .collect::<Vec<_>>()
            })
            .into();
        detail["gates"] = db::gates(&db, capsule_id)?.into();
        detail["compile"] = compilation.unwrap_or(Value::Null);
        detail["runs"] = json!({"count": runs, "deopts": deopts});
        detail["skill_files"] = json!({
            "skills": capsule["skill_files"]["skills"].as_array().into_iter().flatten().map(|skill| json!({
                "name": skill["name"], "path": skill["path"], "tree_sha256": skill["tree_sha256"],
                "files": skill["files"].as_array().map_or(0, Vec::len),
            })).collect::<Vec<_>>()
        });
        Ok(detail)
    }
    fn invalidate(&self, project_id: &str, capsule_id: &str) -> Result<Value> {
        let capsule = self.owned(project_id, capsule_id)?;
        if !["active", "needs_recompile"].contains(&capsule["status"].as_str().unwrap_or("")) {
            return Err(HarnessError::conflict(
                "Only an active capsule can be invalidated.",
            ));
        }
        db::set_capsule_status(
            &*self.runtime.store.connection()?,
            capsule_id,
            "stale",
            Some("Invalidated by a person; requalify it to use it again."),
        )?;
        self.runtime
            .publish(project_id, "invalidated", json!({"capsule_id": capsule_id}));
        self.get(project_id, capsule_id)
    }
    fn requalify(&self, project_id: &str, capsule_id: &str) -> Result<Value> {
        let capsule = self.owned(project_id, capsule_id)?;
        if !["active", "stale", "needs_recompile"]
            .contains(&capsule["status"].as_str().unwrap_or(""))
        {
            return Err(HarnessError::conflict(
                "Only a compiled capsule can be requalified.",
            ));
        }
        self.queue(Work::Requalify {
            capsule_id: capsule_id.into(),
        })?;
        self.runtime.publish(
            project_id,
            "queued",
            json!({"capsule_id": capsule_id, "requalify": true}),
        );
        Ok(json!({"capsule_id": capsule_id, "status": "queued"}))
    }
    fn delete(&self, project_id: &str, capsule_id: &str) -> Result<Value> {
        let capsule = self.owned(project_id, capsule_id)?;
        if capsule["status"] == "compiling" {
            return Err(HarnessError::conflict(
                "Wait for the compilation to finish.",
            ));
        }
        let store = &self.runtime.store;
        let artifacts = compile::artifact_dir(store, project_id, capsule_id, 1);
        let _ = package::remove(&artifacts.join("warm"));
        if let Some(capsule_root) = artifacts.parent() {
            let _ = std::fs::remove_dir_all(capsule_root);
        }
        if let Some(qualification) = compile::qualification_dir(store, capsule_id, 1).parent() {
            let _ = std::fs::remove_dir_all(qualification);
        }
        let generation = capsule["source_generation"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let db = store.connection()?;
        db::delete_capsule(&db, capsule_id)?;
        // The trace goes with the last capsule that was compiled from it.
        let shared: i64 = db.query_row(
            "SELECT COUNT(*) FROM skillcache_capsules WHERE source_generation=?1",
            [&generation],
            |row| row.get(0),
        )?;
        if shared == 0 {
            db::delete_trace(&db, &generation)?;
            drop(db);
            let _ = snapshot::remove(&store.state, &generation);
        }
        self.runtime
            .publish(project_id, "deleted", json!({"capsule_id": capsule_id}));
        Ok(json!({"capsule_id": capsule_id, "deleted": true}))
    }
    fn compile(&self, session_id: &str, body: &Value) -> Result<Value> {
        if !crate::store::exact(body, &["generation"], &["compiler"]) {
            return Err(HarnessError::invalid(
                "Name the turn to compile: {\"generation\": ..., \"compiler\": {\"provider\": \"claude\", \"model\": ...}}.",
            ));
        }
        let store = &self.runtime.store;
        let session = store.get(session_id)?;
        let generation = crate::store::text(&body["generation"], "generation", 128)?;
        let chosen = settings::validate(&json!({
            "mode": "observe",
            "compiler": if body["compiler"].is_null() { session["skillcache"]["compiler"].clone() } else { body["compiler"].clone() },
        }))?;
        let project_id = session["project_id"].as_str().unwrap_or("").to_string();
        let workspace = PathBuf::from(session["project_path"].as_str().unwrap_or(""));
        let (compile_id, capsule_id) = (crate::store::id(), crate::store::id());
        {
            let db = store.connection()?;
            let stored = db::trace(&db, &generation)?
                .filter(|trace| trace["session_id"] == session_id)
                .ok_or_else(|| HarnessError::missing("This session kept no trace of that turn."))?;
            let skill = stored["skill_refs"]
                .as_array()
                .and_then(|skills| skills.first())
                .cloned()
                .ok_or_else(|| HarnessError::invalid("The trace names no skill."))?;
            if db::compile_pending(&db, &generation)? {
                return Err(HarnessError::conflict(
                    "This turn is already being compiled.",
                ));
            }
            if db::pending_compiles(&db)? >= WAITING as i64 {
                return Err(HarnessError::conflict(
                    "The SkillCache queue is full; try again when a compilation has finished.",
                ));
            }
            db::begin_capsule(
                &db,
                &capsule_id,
                &compile_id,
                &project_id,
                session_id,
                &generation,
                &skill,
            )?;
            db::begin_compile(
                &db,
                &compile_id,
                &capsule_id,
                &project_id,
                session_id,
                &generation,
                "claude",
                chosen["compiler"]["model"].as_str(),
            )?;
        }
        let job = Job {
            compile_id: compile_id.clone(),
            capsule_id: capsule_id.clone(),
            session_id: session_id.into(),
            generation,
            project_id: project_id.clone(),
            workspace,
            model: chosen["compiler"]["model"].as_str().map(String::from),
        };
        let detail = json!({"capsule_id": capsule_id, "compile_id": compile_id, "session_id": session_id, "generation": job.generation});
        if let Err(error) = self.queue(Work::Compile(job)) {
            let db = store.connection()?;
            let _ = db::finish_compile(
                &db,
                &compile_id,
                "failed",
                0,
                &error.to_string(),
                &Value::Null,
                &Value::Null,
            );
            let _ = db::set_capsule_status(&db, &capsule_id, "failed", Some(&error.to_string()));
            return Err(error);
        }
        self.runtime.publish(&project_id, "queued", detail);
        Ok(json!({"compile_id": compile_id, "capsule_id": capsule_id, "status": "queued"}))
    }
    /// What a session's SkillCache state is: its mode, the traces it kept and
    /// what was compiled from them. A trace is listed without its steps; they
    /// stay in the private state.
    fn session(&self, session_id: &str) -> Result<Value> {
        let store = &self.runtime.store;
        let session = store.get(session_id)?;
        let db = store.connection()?;
        let mut traces = db::session_traces(&db, session_id)?;
        for listed in &mut traces {
            let generation = listed["generation"].as_str().unwrap_or("").to_string();
            listed["replayable"] = snapshot::load(&store.state, &generation)
                .is_ok_and(|snapshot| snapshot.is_some())
                .into();
            listed["compiling"] = db::compile_pending(&db, &generation)?.into();
        }
        let compiles = db::session_compiles(&db, session_id)?;
        let runs = db::session_runs(&db, session_id)?;
        let mut capsules = Vec::new();
        for compiled in &compiles {
            if let Some(id) = compiled["capsule_id"].as_str()
                && let Some(capsule) = db::capsule(&db, id)?
            {
                capsules.push(summary(&capsule));
            }
        }
        Ok(json!({
            "session_id": session_id,
            "mode": settings::mode(&session),
            "settings": session["skillcache"],
            "traces": traces,
            "compiles": compiles,
            "capsules": capsules,
            "runs": runs,
            "retention": {"days": db::TRACE_DAYS, "per_session": db::TRACES_PER_SESSION},
        }))
    }
}

fn project_path(store: &SessionStore, capsule: &Value) -> Result<PathBuf> {
    let project = store.project(capsule["project_id"].as_str().unwrap_or(""))?;
    project["path"]
        .as_str()
        .map(PathBuf::from)
        .ok_or_else(|| HarnessError::missing("The capsule's project is not registered."))
}

/// What a list shows of a capsule: nothing of where its files are.
fn summary(capsule: &Value) -> Value {
    let mut summary = json!({});
    for field in [
        "capsule_id",
        "project_id",
        "skill_name",
        "skill_path",
        "mu",
        "form",
        "status",
        "reason",
        "revision",
        "capabilities",
        "source_session_id",
        "source_generation",
        "compile_id",
        "created_at",
        "updated_at",
        "last_used_at",
    ] {
        summary[field] = capsule[field].clone();
    }
    summary
}

impl Domain for SkillCacheDomain {
    fn execute<'a>(&'a self, operation: &'a str, request: Value) -> Reply<'a> {
        Box::pin(async move {
            let parameter = |name: &str| {
                request["params"][name]
                    .as_str()
                    .ok_or_else(|| HarnessError::invalid(format!("Missing {name}.")))
            };
            let value = match operation {
                "skillcache.session" => self.session(parameter("session_id")?)?,
                "skillcache.compile" => self.compile(parameter("session_id")?, &request["body"])?,
                "skillcache.list" => self.list(parameter("project_id")?, &request["query"])?,
                "skillcache.get" => self.get(parameter("project_id")?, parameter("capsule_id")?)?,
                "skillcache.requalify" => {
                    self.requalify(parameter("project_id")?, parameter("capsule_id")?)?
                }
                "skillcache.invalidate" => {
                    self.invalidate(parameter("project_id")?, parameter("capsule_id")?)?
                }
                "skillcache.delete" => {
                    self.delete(parameter("project_id")?, parameter("capsule_id")?)?
                }
                _ => return Err(HarnessError::missing("Unknown SkillCache operation.")),
            };
            Ok(Payload::Json(value))
        })
    }
}
