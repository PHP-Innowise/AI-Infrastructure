//! Execution of all 6 SkillCache evaluation arms through in-process Harness.
use super::{
    config::{DriverConfig, ModelConfig, TaskConfig},
    usage::{self, Components},
    workspace,
};
use infra_core::fs as core_fs;
use infra_harness::{
    HarnessError, Result,
    accelerators::Accelerators,
    bindings::{ResultsBridge, SessionLinks},
    commands::CommandCatalog,
    fleet,
    knowledge::KnowledgeDomain,
    provider::{Provider, ProviderDomain},
    results::Results,
    services::{Domain, Payload},
    sessions::{BlackboardFactory, Event, RunReply, SessionLimits, Sessions, SpecialWorkflow},
    skillcache::SkillCacheDomain,
    store::{self, SessionStore},
};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, RwLock, atomic::AtomicBool},
    time::{Duration, Instant},
};

struct UnusedSpecial;
impl SpecialWorkflow for UnusedSpecial {
    fn run<'a>(
        &'a self,
        _session: Value,
        _prompt: String,
        _generation: String,
        _cancel: Arc<AtomicBool>,
        _emit: Event,
    ) -> RunReply<'a> {
        Box::pin(async {
            Err(HarnessError::invalid(
                "No special workflows in pilot driver.",
            ))
        })
    }
}
impl BlackboardFactory for UnusedSpecial {
    fn for_session(&self, _session: &Value) -> Result<Arc<dyn fleet::Blackboard>> {
        Err(HarnessError::invalid("No blackboard in pilot driver."))
    }
}

struct HarnessEnv {
    store: SessionStore,
    sessions: Arc<Sessions>,
    skillcache: Arc<SkillCacheDomain>,
    project_id: String,
}

impl HarnessEnv {
    async fn open(
        state_dir: &Path,
        workspace: &Path,
        config: &DriverConfig,
        active_models: &[&ModelConfig],
        protected: &[PathBuf],
    ) -> Result<Self> {
        infra_host::util::private_directory(state_dir)?;
        let source = state_dir.join("source");
        core_fs::ensure_directory(&source, true)?;

        let store = SessionStore::open(state_dir)?;
        store.set_setting("skillcache.eval_arms", &json!(true))?;
        let registered = store.register(workspace)?;
        let project_id = registered["id"]
            .as_str()
            .ok_or_else(|| HarnessError::invalid("Registered project id missing."))?
            .to_owned();

        let mut overrides = BTreeMap::new();
        for model_cfg in active_models {
            if let Some(exe) = &model_cfg.executable {
                overrides.insert(
                    model_cfg.provider.clone(),
                    exe.to_string_lossy().into_owned(),
                );
            }
        }
        let discovered = infra_host::provider::discover(&overrides).await?;
        let mut provider_map = BTreeMap::new();
        for mut provider in discovered {
            if let Some(exe) = active_models
                .iter()
                .rev()
                .find(|m| m.provider == provider.id)
                .and_then(|m| m.executable.as_ref())
            {
                provider.available = true;
                provider.executable = Some(exe.clone());
            }
            provider_map.insert(provider.id.clone(), provider);
        }
        for model_cfg in active_models {
            if let Some(exe) = &model_cfg.executable {
                provider_map
                    .entry(model_cfg.provider.clone())
                    .and_modify(|p| {
                        p.available = true;
                        p.executable = Some(exe.clone());
                    })
                    .or_insert_with(|| Provider {
                        id: model_cfg.provider.clone(),
                        name: model_cfg.provider.clone(),
                        available: true,
                        executable: Some(exe.clone()),
                        detail: "Configured pilot provider".into(),
                    });
            }
        }
        for id in ["claude", "codex", "cursor"] {
            let exe = provider_map
                .get(id)
                .filter(|_p| {
                    !active_models
                        .iter()
                        .any(|m| m.provider == id && m.executable.is_some())
                })
                .and_then(|p| p.executable.as_deref());
            infra_host::model_catalog::model_options(id, exe, None).await?;
        }

        let providers = Arc::new(ProviderDomain {
            providers: Arc::new(RwLock::new(provider_map)),
        });
        let knowledge = Arc::new(KnowledgeDomain::new(store.clone(), source.clone()));
        let special = ["creator", "system_run", "system_discovery"]
            .into_iter()
            .map(|name| {
                (
                    name.into(),
                    Arc::new(UnusedSpecial) as Arc<dyn SpecialWorkflow>,
                )
            })
            .collect();
        let sessions = Sessions::new(
            store.clone(),
            source.clone(),
            providers.clone(),
            knowledge.clone(),
            Arc::new(UnusedSpecial),
            special,
            SessionLimits {
                seconds: config.session_timeout_seconds,
                concurrency: 1,
            },
        )?;
        if !protected.is_empty() {
            sessions.install_protected(protected.to_vec(), Some(config.engine.clone()))?;
        }
        let links = Arc::new(SessionLinks::new());
        links.bind(Arc::downgrade(&sessions))?;
        let results = Arc::new(Results::new(
            store.clone(),
            Arc::new(ResultsBridge {
                links,
                knowledge,
                operation: sessions.operation_lock(),
                cancel: sessions.shutdown_cancel(),
            }),
        )?);
        sessions.install_results(results)?;
        sessions.install_commands(Arc::new(CommandCatalog::new(
            store.clone(),
            source.clone(),
            providers.providers.clone(),
        )))?;
        sessions.install_accelerators(Arc::new(Accelerators::new(
            store.clone(),
            source,
            providers.clone(),
            Arc::downgrade(&sessions),
        )?))?;
        let skillcache = Arc::new(
            SkillCacheDomain::new(store.clone(), providers, sessions.shutdown_cancel())
                .with_engine(config.engine.clone()),
        );
        sessions.install_skillcache(skillcache.runtime())?;
        sessions.activate()?;
        Ok(Self {
            store,
            sessions,
            skillcache,
            project_id,
        })
    }

    async fn close(self) -> Result<()> {
        let closing = self.sessions.close();
        let waited = self.sessions.wait_closed().await;
        closing?;
        waited?;
        if let Ok(db) = self.store.connection() {
            let _ = db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        }
        Ok(())
    }
}

fn json_payload(payload: Payload) -> Result<Value> {
    match payload {
        Payload::Json(value) => Ok(value),
        _ => Err(HarnessError::invalid("Expected JSON payload from Harness.")),
    }
}

/// Ensures every arm sends the same `$skill` routing prefix, including `cold`.
fn skill_prompt(skill: &str, request: &str) -> String {
    let needle = format!("${skill}");
    let already = request.split_whitespace().any(|word| {
        let trimmed = word.trim_end_matches(|ch: char| ",.:;!?)]}\"'".contains(ch));
        trimmed == needle
    });
    if already {
        request.to_owned()
    } else {
        format!("{needle} {request}")
    }
}

fn session_create_body(
    project_id: &str,
    model_cfg: &ModelConfig,
    prompt: &str,
    skillcache: Value,
) -> Value {
    let mut body = json!({
        "project_id": project_id,
        "provider": model_cfg.provider,
        "mode": "edit",
        "workflow": "native",
        "project_context": false,
        "permissions": model_cfg.permissions,
        "prompt": prompt,
        "skillcache": skillcache,
    });
    if let Some(model) = &model_cfg.model {
        body["model"] = json!(model);
    }
    body
}

async fn run_session(
    env: &HarnessEnv,
    model_cfg: &ModelConfig,
    prompt: &str,
    skillcache: Value,
    timeout_seconds: u64,
) -> Result<(String, String, Value)> {
    let body = session_create_body(&env.project_id, model_cfg, prompt, skillcache);
    let reply = json_payload(
        env.sessions
            .execute("sessions.create", json!({"body": body}))
            .await?,
    )?;
    let sid = reply["session"]["id"]
        .as_str()
        .ok_or_else(|| HarnessError::unavailable("Created session has no id."))?
        .to_owned();
    let generation = env.store.generation(&sid)?;
    let deadline = Instant::now() + Duration::from_secs(timeout_seconds.saturating_add(60));
    loop {
        let session = env.store.get(&sid)?;
        let status = session["status"].as_str().unwrap_or("");
        if matches!(status, "completed" | "failed" | "interrupted" | "cancelled") {
            return Ok((sid, generation, session));
        }
        if matches!(status, "awaiting_context" | "awaiting_approval") {
            let _ = env.sessions.cancel(&sid);
            return Err(HarnessError::unavailable(format!(
                "Pilot session entered unexpected waiting status `{status}`."
            )));
        }
        if Instant::now() >= deadline {
            let _ = env.sessions.cancel(&sid);
            return Err(HarnessError::unavailable(
                "Pilot session deadline reached before completion.",
            ));
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn load_session_launches(store: &SessionStore, sid: &str) -> Result<Vec<Value>> {
    let db = store.connection()?;
    let mut stmt = db.prepare(
        "SELECT id,status,settings,usage FROM launches WHERE session_id=?1 ORDER BY started_at,rowid",
    )?;
    let rows = stmt.query_map([sid], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, status, settings, usage_raw) = row?;
        out.push(json!({
            "id": id,
            "status": status,
            "settings": serde_json::from_str::<Value>(&settings)?,
            "usage": usage_raw
                .as_deref()
                .map(serde_json::from_str::<Value>)
                .transpose()?
                .unwrap_or(Value::Null),
        }));
    }
    Ok(out)
}

fn load_run_row(store: &SessionStore, generation: &str) -> Result<Option<Value>> {
    let db = store.connection()?;
    Ok(db
        .query_row(
            "SELECT outcome,steps,deopts FROM skillcache_runs WHERE generation=?1",
            [generation],
            |row| {
                let steps_raw: String = row.get(1)?;
                Ok(json!({
                    "outcome": row.get::<_, String>(0)?,
                    "steps": serde_json::from_str::<Value>(&steps_raw).unwrap_or_else(|_| json!([])),
                    "deopts": row.get::<_, i64>(2)?,
                }))
            },
        )
        .optional()?)
}

fn has_trace(store: &SessionStore, generation: &str) -> Result<bool> {
    let db = store.connection()?;
    let count: i64 = db.query_row(
        "SELECT COUNT(*) FROM skillcache_traces WHERE generation=?1",
        [generation],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn load_compile_row(store: &SessionStore, compile_id: &str) -> Result<Value> {
    let db = store.connection()?;
    db.query_row(
        "SELECT compile_id,status,rounds,outcome,usage,receipt,capsule_id
         FROM skillcache_compiles WHERE compile_id=?1",
        [compile_id],
        |row| {
            let usage: Option<String> = row.get(4)?;
            let receipt: Option<String> = row.get(5)?;
            Ok(json!({
                "compile_id": row.get::<_, String>(0)?,
                "status": row.get::<_, String>(1)?,
                "rounds": row.get::<_, i64>(2)?,
                "outcome": row.get::<_, Option<String>>(3)?,
                "usage": usage.as_deref().and_then(|s| serde_json::from_str::<Value>(s).ok()).unwrap_or(Value::Null),
                "receipt": receipt.as_deref().and_then(|s| serde_json::from_str::<Value>(s).ok()).unwrap_or(Value::Null),
                "capsule_id": row.get::<_, Option<String>>(6)?,
            }))
        },
    )
    .map_err(HarnessError::from)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    core_fs::ensure_directory(dst, true)?;
    let mut entries = fs::read_dir(src)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let src_path = entry.path();
        let meta = fs::symlink_metadata(&src_path)?;
        if core_fs::linked_at(&src_path, &meta)? {
            continue;
        }
        let dst_path = dst.join(entry.file_name());
        if meta.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if meta.is_file() {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

fn clear_dir_if_exists(path: &Path) -> Result<()> {
    if path.exists() {
        infra_skillcache::package::release(path);
        fs::remove_dir_all(path)?;
    }
    Ok(())
}

fn sha256_json(value: &Value) -> String {
    store::sha(value.to_string().as_bytes())
}

fn observe_key(seal_sha256: &str, task_id: &str, observe_model: &str) -> String {
    sha256_json(&json!({
        "seal_sha256": seal_sha256,
        "task": task_id,
        "observe_model": observe_model,
    }))
}

fn prep_key(
    seal_sha256: &str,
    task_id: &str,
    arm: &str,
    compile_model: Option<&str>,
    executor_model: &str,
) -> String {
    sha256_json(&json!({
        "seal_sha256": seal_sha256,
        "task": task_id,
        "arm": arm,
        "compile_model": compile_model,
        "executor_model": executor_model,
    }))
}

fn lookup_model<'a>(config: &'a DriverConfig, name: &str, role: &str) -> Result<&'a ModelConfig> {
    config.models.get(name).ok_or_else(|| {
        HarnessError::invalid(format!(
            "`driver.json` `models` has no entry for {role} `{name}`."
        ))
    })
}

fn lookup_task<'a>(config: &'a DriverConfig, id: &str) -> Result<&'a TaskConfig> {
    config.tasks.get(id).ok_or_else(|| {
        HarnessError::invalid(format!(
            "`driver.json` `tasks` has no entry for task `{id}`."
        ))
    })
}

fn require_str<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| HarnessError::invalid(format!("Missing or invalid `{field}`.")))
}

#[derive(Clone)]
struct ObservationRecord {
    session_id: String,
    generation: String,
    trace_recorded: bool,
    observe_usage: Value,
    state_dir: PathBuf,
}

struct ObserveTarget<'a> {
    config: &'a DriverConfig,
    seal_sha256: &'a str,
    task_id: &'a str,
    task_cfg: &'a TaskConfig,
    skill: &'a str,
    observe_prompt: &'a str,
    observe_model_name: &'a str,
    protected: &'a [PathBuf],
}

async fn ensure_observation(target: ObserveTarget<'_>) -> Result<ObservationRecord> {
    let ObserveTarget {
        config,
        seal_sha256,
        task_id,
        task_cfg,
        skill,
        observe_prompt,
        observe_model_name,
        protected,
    } = target;
    let key = observe_key(seal_sha256, task_id, observe_model_name);
    let root = config.state_root.join("observe").join(&key);
    let meta_path = root.join("obs.json");
    let state_dir = root.join("harness");
    if meta_path.is_file() && state_dir.join("sessions.sqlite3").is_file() {
        let bytes = core_fs::read_regular(&meta_path)?;
        let doc = infra_harness::strict_json::parse(&bytes, 1024 * 1024)?;
        return Ok(ObservationRecord {
            session_id: doc["session_id"].as_str().unwrap_or("").to_owned(),
            generation: doc["generation"].as_str().unwrap_or("").to_owned(),
            trace_recorded: doc["trace_recorded"] == true,
            observe_usage: doc["observe_usage"].clone(),
            state_dir,
        });
    }

    let ws_dir = config.workspaces_root.join("observe").join(&key).join("ws");
    clear_dir_if_exists(&root)?;
    clear_dir_if_exists(&ws_dir)?;
    infra_host::util::private_directory(&root)?;
    let ws = workspace::materialize(config, task_cfg, &ws_dir, protected)?;

    let obs_model = lookup_model(config, observe_model_name, "observe_model")?;
    let env = HarnessEnv::open(&state_dir, &ws, config, &[obs_model], protected).await?;
    let prompt = skill_prompt(skill, observe_prompt);
    let (sid, generation, _session) = run_session(
        &env,
        obs_model,
        &prompt,
        json!({"mode": "observe"}),
        config.session_timeout_seconds,
    )
    .await?;
    let launches = load_session_launches(&env.store, &sid)?;
    let obs_components = usage::sum_launch_components(&launches);
    let trace_recorded = has_trace(&env.store, &generation)?;
    env.close().await?;

    let observe_usage = obs_components.to_usage_json();
    let doc = json!({
        "session_id": sid,
        "generation": generation,
        "trace_recorded": trace_recorded,
        "observe_usage": observe_usage,
    });
    core_fs::atomic_json(&meta_path, &doc)?;
    Ok(ObservationRecord {
        session_id: doc["session_id"].as_str().unwrap_or("").to_owned(),
        generation: doc["generation"].as_str().unwrap_or("").to_owned(),
        trace_recorded,
        observe_usage: doc["observe_usage"].clone(),
        state_dir,
    })
}

fn seed_state_from(src_state: &Path, dst_state: &Path, copy_snapshots: bool) -> Result<()> {
    infra_host::util::private_directory(dst_state)?;
    fs::copy(
        src_state.join("sessions.sqlite3"),
        dst_state.join("sessions.sqlite3"),
    )?;
    if copy_snapshots {
        let src_snaps = src_state.join("skillcache/snapshots");
        if src_snaps.is_dir() {
            copy_dir_recursive(&src_snaps, &dst_state.join("skillcache/snapshots"))?;
        }
    }
    Ok(())
}

fn rebind_project_in_db(store: &SessionStore, project_id: &str, workspace: &Path) -> Result<()> {
    let ws_str = workspace
        .to_str()
        .ok_or_else(|| HarnessError::invalid("Workspace path is not UTF-8."))?;
    let db = store.connection()?;
    db.execute(
        "UPDATE sessions SET project_id=?1, project_path=?2",
        params![project_id, ws_str],
    )?;
    db.execute(
        "UPDATE skillcache_capsules SET project_id=?1",
        params![project_id],
    )?;
    db.execute(
        "UPDATE skillcache_capsules_fts SET project_id=?1",
        params![project_id],
    )?;
    Ok(())
}

async fn run_prepare(
    config: &DriverConfig,
    request: &Value,
    protected: &[PathBuf],
) -> Result<Value> {
    let started = Instant::now();
    let seal_sha256 = require_str(&request["seal_sha256"], "seal_sha256")?;
    let task_id = require_str(&request["task"], "task")?;
    let skill = require_str(&request["skill"], "skill")?;
    let observe_prompt = require_str(&request["observe"], "observe")?;
    let arm = require_str(&request["arm"], "arm")?;
    let executor_model_name = require_str(&request["executor_model"], "executor_model")?;
    let observe_model_name = require_str(&request["observe_model"], "observe_model")?;
    let compile_model_name = request["compile_model"].as_str();
    let task_cfg = lookup_task(config, task_id)?;
    let _ = lookup_model(config, executor_model_name, "executor_model")?;
    let _ = lookup_model(config, observe_model_name, "observe_model")?;

    if arm == "cold" {
        return Ok(json!({
            "prepared": true,
            "usage": usage::zero_usage_json(),
            "observe_usage": Value::Null,
            "seconds": started.elapsed().as_secs_f64(),
        }));
    }

    let obs = ensure_observation(ObserveTarget {
        config,
        seal_sha256,
        task_id,
        task_cfg,
        skill,
        observe_prompt,
        observe_model_name,
        protected,
    })
    .await?;

    let pkey = prep_key(
        seal_sha256,
        task_id,
        arm,
        compile_model_name,
        executor_model_name,
    );
    let prep_root = config.state_root.join("prep").join(&pkey);
    let prep_meta = prep_root.join("prep.json");
    let prep_state_dir = prep_root.join("harness");

    if arm == "raw_trace" {
        clear_dir_if_exists(&prep_root)?;
        infra_host::util::private_directory(&prep_root)?;
        core_fs::atomic_json(
            &prep_meta,
            &json!({
                "prepared": obs.trace_recorded,
                "arm": arm,
                "state_dir": obs.state_dir,
                "trace_generation": obs.generation,
                "capsule_id": Value::Null,
            }),
        )?;
        return Ok(json!({
            "prepared": obs.trace_recorded,
            "usage": usage::zero_usage_json(),
            "observe_usage": obs.observe_usage,
            "seconds": started.elapsed().as_secs_f64(),
        }));
    }

    let compile_name = compile_model_name
        .ok_or_else(|| HarnessError::invalid(format!("Arm `{arm}` requires `compile_model`.")))?;
    let compile_cfg = lookup_model(config, compile_name, "compile_model")?;
    if compile_cfg.provider != "claude" {
        return Err(HarnessError::invalid(format!(
            "SkillCache compiler `{compile_name}` must use provider `claude`, got `{}`.",
            compile_cfg.provider
        )));
    }

    if !obs.trace_recorded {
        clear_dir_if_exists(&prep_root)?;
        infra_host::util::private_directory(&prep_root)?;
        core_fs::atomic_json(
            &prep_meta,
            &json!({
                "prepared": false,
                "arm": arm,
                "state_dir": Value::Null,
                "trace_generation": obs.generation,
                "capsule_id": Value::Null,
            }),
        )?;
        return Ok(json!({
            "prepared": false,
            "usage": Components::default().to_usage_json(),
            "observe_usage": obs.observe_usage,
            "seconds": started.elapsed().as_secs_f64(),
        }));
    }

    let prep_ws_dir = config.workspaces_root.join("prep").join(&pkey).join("ws");
    clear_dir_if_exists(&prep_root)?;
    clear_dir_if_exists(&prep_ws_dir)?;
    infra_host::util::private_directory(&prep_root)?;
    let prep_ws = workspace::materialize(config, task_cfg, &prep_ws_dir, protected)?;
    seed_state_from(&obs.state_dir, &prep_state_dir, true)?;

    let env =
        HarnessEnv::open(&prep_state_dir, &prep_ws, config, &[compile_cfg], protected).await?;
    rebind_project_in_db(&env.store, &env.project_id, &prep_ws)?;

    let mut compiler = json!({"provider": "claude"});
    if let Some(model) = &compile_cfg.model {
        compiler["model"] = json!(model);
    }
    let compile_reply = json_payload(
        env.skillcache
            .execute(
                "skillcache.compile",
                json!({
                    "params": {"session_id": obs.session_id},
                    "body": {
                        "generation": obs.generation,
                        "compiler": compiler,
                        "eval_arm": arm,
                    },
                }),
            )
            .await?,
    )?;
    let compile_id = compile_reply["compile_id"]
        .as_str()
        .ok_or_else(|| HarnessError::unavailable("Compile queue returned no `compile_id`."))?
        .to_owned();

    let deadline =
        Instant::now() + Duration::from_secs(config.session_timeout_seconds.saturating_add(300));
    let compile_row = loop {
        let row = load_compile_row(&env.store, &compile_id)?;
        let status = row["status"].as_str().unwrap_or("");
        if matches!(status, "completed" | "failed" | "interrupted") {
            break row;
        }
        if Instant::now() >= deadline {
            env.close().await?;
            return Err(HarnessError::unavailable(
                "SkillCache compile deadline reached before completion.",
            ));
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    let capsule_id = compile_row["capsule_id"].as_str().map(str::to_owned);
    let prepared = compile_row["status"] == "completed" && capsule_id.is_some();
    let compile_components = usage::from_compile_row(&compile_row);
    env.close().await?;

    core_fs::atomic_json(
        &prep_meta,
        &json!({
            "prepared": prepared,
            "arm": arm,
            "state_dir": prep_state_dir,
            "trace_generation": obs.generation,
            "capsule_id": capsule_id,
        }),
    )?;

    Ok(json!({
        "prepared": prepared,
        "usage": compile_components.to_usage_json(),
        "observe_usage": obs.observe_usage,
        "seconds": started.elapsed().as_secs_f64(),
    }))
}

async fn run_attempt(
    config: &DriverConfig,
    request: &Value,
    protected: &[PathBuf],
) -> Result<Value> {
    let started = Instant::now();
    let seal_sha256 = require_str(&request["seal_sha256"], "seal_sha256")?;
    let slot = require_str(&request["slot"], "slot")?;
    let task_id = require_str(&request["task"], "task")?;
    let skill = require_str(&request["skill"], "skill")?;
    let req_text = require_str(&request["request"], "request")?;
    let arm = require_str(&request["arm"], "arm")?;
    let executor_model_name = require_str(&request["executor_model"], "executor_model")?;
    let compile_model_name = request["compile_model"].as_str();
    let task_cfg = lookup_task(config, task_id)?;
    let exec_model = lookup_model(config, executor_model_name, "executor_model")?;

    let attempt_ws_dir = config
        .workspaces_root
        .join("attempts")
        .join(slot)
        .join("ws");
    let attempt_state_dir = config
        .state_root
        .join("attempts")
        .join(slot)
        .join("harness");
    clear_dir_if_exists(&attempt_ws_dir)?;
    clear_dir_if_exists(&attempt_state_dir)?;
    let attempt_ws = workspace::materialize(config, task_cfg, &attempt_ws_dir, protected)?;

    let mut seeded_state: Option<PathBuf> = None;
    let skillcache_option = if arm == "cold" {
        cold_option()
    } else {
        let pkey = prep_key(
            seal_sha256,
            task_id,
            arm,
            compile_model_name,
            executor_model_name,
        );
        let prep_meta = config.state_root.join("prep").join(&pkey).join("prep.json");
        if !prep_meta.is_file() {
            cold_option()
        } else {
            let bytes = core_fs::read_regular(&prep_meta)?;
            let prep = infra_harness::strict_json::parse(&bytes, 1024 * 1024)?;
            if prep["prepared"] != true {
                cold_option()
            } else if arm == "raw_trace" {
                let src_state = PathBuf::from(prep["state_dir"].as_str().unwrap_or(""));
                let trace_gen = prep["trace_generation"].as_str().unwrap_or("");
                if !src_state.join("sessions.sqlite3").is_file() || trace_gen.is_empty() {
                    cold_option()
                } else {
                    seeded_state = Some(src_state);
                    json!({
                        "mode": "warm_only",
                        "eval_arm": "raw_trace",
                        "trace_generation": trace_gen,
                    })
                }
            } else {
                let src_state = PathBuf::from(prep["state_dir"].as_str().unwrap_or(""));
                let capsule_id = prep["capsule_id"].as_str().unwrap_or("");
                if !src_state.join("sessions.sqlite3").is_file() || capsule_id.is_empty() {
                    cold_option()
                } else {
                    seeded_state = Some(src_state);
                    json!({
                        "mode": "warm_only",
                        "eval_arm": arm,
                        "capsule_id": capsule_id,
                    })
                }
            }
        }
    };

    if let Some(src_state) = &seeded_state {
        seed_state_from(src_state, &attempt_state_dir, false)?;
    }

    let env = HarnessEnv::open(
        &attempt_state_dir,
        &attempt_ws,
        config,
        &[exec_model],
        protected,
    )
    .await?;
    if seeded_state.is_some() {
        rebind_project_in_db(&env.store, &env.project_id, &attempt_ws)?;
    }

    let prompt = skill_prompt(skill, req_text);
    let (sid, generation, session) = run_session(
        &env,
        exec_model,
        &prompt,
        skillcache_option,
        config.session_timeout_seconds,
    )
    .await?;

    let launches = load_session_launches(&env.store, &sid)?;
    let components = usage::sum_launch_components(&launches);
    let run_row = load_run_row(&env.store, &generation)?;
    let (program_steps, failed_steps) = usage::steps_from_run(run_row.as_ref());
    env.close().await?;

    let accepted = session["status"].as_str() == Some("completed");
    // Whether the arm's treatment ran: the turn's own decision, as its launch
    // recorded it. A cold arm has none to apply; an arm that fell back to a
    // cold turn says so instead of passing for its treatment.
    let applied = (arm != "cold").then(|| {
        launches
            .iter()
            .any(|launch| launch["settings"]["skillcache"]["decision"] == "hit")
    });
    Ok(json!({
        "workspace": attempt_ws,
        "accepted": accepted,
        "usage": components.to_usage_json(),
        "model_calls": components.model_calls,
        "seconds": started.elapsed().as_secs_f64(),
        "program_steps": program_steps,
        "failed_steps": failed_steps,
        "applied": applied,
    }))
}

/// A turn without the arm's treatment that is still routed as every other
/// arm's: a `$skill` mention is resolved for a session that opted into
/// SkillCache, and the `cold` arm keeps the capsule out of it.
fn cold_option() -> Value {
    json!({"mode": "warm_only", "eval_arm": "cold"})
}

pub async fn handle(
    config: &DriverConfig,
    request: &Value,
    protected: &[PathBuf],
) -> Result<Value> {
    match request["phase"].as_str() {
        Some("prepare") => run_prepare(config, request, protected).await,
        Some("attempt") => run_attempt(config, request, protected).await,
        _ => Err(HarnessError::invalid(
            "Request `phase` must be `prepare` or `attempt`.",
        )),
    }
}
