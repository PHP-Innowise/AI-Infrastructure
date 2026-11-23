//! Warm execution: a turn that runs a qualified capsule instead of reading
//! the skill.
//!
//! A turn is warm only when everything about it is what a capsule was
//! qualified for: an editing session of Claude or Codex, a message that routes
//! to exactly one skill, one active capsule of that skill that still matches
//! its skill, its environment and its rules. Anything else is a miss, and a
//! miss changes nothing about the turn. On a hit the prompt carries the
//! capsule's description, the bindings to take from the request, the opening
//! steps and the executor's protocol; it never carries an observed value, a
//! program's code or where the package is.
use crate::{
    HarnessError, Result,
    commands::ResolvedCommand,
    skillcache::{compile::Runtime, db, settings, staleness, trace},
    store::SessionStore,
};
use infra_host::provider::{Launch, LaunchAdapter, PreparedCommand};
use infra_skillcache::{
    Form, Step, capsule::Network, env_filter, package, prompt, state, trace as sc_trace,
};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

/// Removes leftover warm session state directories at server startup.
pub fn clear_sessions(state: &Path) {
    let _ = std::fs::remove_dir_all(state.join("skillcache/sessions"));
}

/// The experiment switch for choosing among several capsules of one skill by
/// their descriptions. Off until it has its own measurement.
pub const RETRIEVAL_BY_MU: &str = "skillcache.retrieval_by_mu";
/// The setting that lets a capsule's own warm turns ask for its
/// recompilation: `{window, failed_runs}`. Absent, the Harness only shows how
/// the turns went. There are no built-in thresholds, because none has been
/// measured; whoever sets them names them.
pub const RECOMPILE_THRESHOLDS: &str = "skillcache.recompile_thresholds";

/// `(window, failed_runs)` when the setting holds usable numbers.
fn recompile_thresholds(store: &SessionStore) -> Option<(i64, usize)> {
    let setting = store.setting(RECOMPILE_THRESHOLDS).ok().flatten()?;
    let window = setting["window"]
        .as_u64()
        .filter(|window| (1..=50).contains(window))?;
    let failed = setting["failed_runs"]
        .as_u64()
        .filter(|failed| (1..=window).contains(failed))?;
    Some((window as i64, failed as usize))
}

/// A turn that runs a capsule (or an evaluation ablation arm).
pub struct Warm {
    pub capsule_id: String,
    pub form: Form,
    pub eval_arm: Option<String>,
    pub name: String,
    /// What replaces the skill request in the prompt.
    pub block: String,
    /// Hybrid only: the executor's variables for the provider's process.
    pub environment: BTreeMap<String, String>,
    pub bin: Option<PathBuf>,
    pub package: Option<PathBuf>,
    pub state_dir: Option<PathBuf>,
    /// The last journal line already turned into a session event.
    seen: AtomicU64,
    /// The last sidecar `run-log.changes.jsonl` line already forwarded to focus tracking.
    seen_changes: AtomicU64,
}
impl Warm {
    fn agent_only(&self) -> bool {
        self.eval_arm.as_deref() == Some("skillcache_agent")
    }

    #[cfg(test)]
    pub(crate) fn for_changes_test(state_dir: PathBuf) -> Self {
        Self {
            capsule_id: "c-test".into(),
            form: Form::Hybrid,
            eval_arm: None,
            name: "test".into(),
            block: String::new(),
            environment: BTreeMap::new(),
            bin: None,
            package: None,
            state_dir: Some(state_dir),
            seen: AtomicU64::new(0),
            seen_changes: AtomicU64::new(0),
        }
    }
}

pub enum Decision {
    /// The session did not ask for warm turns.
    Off,
    Miss {
        reason: &'static str,
        text: String,
    },
    Hit(Arc<Warm>),
}
impl Decision {
    /// The session event that says what was decided.
    pub fn event(&self) -> Option<Value> {
        match self {
            Self::Off => None,
            Self::Miss { reason, text } => Some(json!({
                "kind": "skillcache",
                "phase": "miss",
                "reason": reason,
                "text": format!("SkillCache: this turn runs cold ({text})."),
            })),
            Self::Hit(warm) => {
                let mut event = json!({
                    "kind": "skillcache",
                    "phase": "hit",
                    "capsule_id": (!warm.capsule_id.is_empty()).then_some(warm.capsule_id.as_str()),
                    "form": warm.form.as_str(),
                    "mu": warm.name,
                    "text": if warm.eval_arm.as_deref() == Some("raw_trace") {
                        "SkillCache: injected observed trace (`raw_trace` evaluation arm).".to_string()
                    } else {
                        format!(
                            "SkillCache: capsule `{}` runs this request instead of the skill being read again.",
                            warm.name
                        )
                    },
                });
                if let Some(arm) = &warm.eval_arm {
                    event["eval_arm"] = arm.clone().into();
                }
                Some(event)
            }
        }
    }
    /// What the launch record keeps of the decision.
    pub fn record(&self) -> Option<Value> {
        match self {
            Self::Off => None,
            Self::Miss { reason, .. } => Some(json!({"decision": "miss", "reason": reason})),
            Self::Hit(warm) => {
                let mut record = json!({
                    "decision": "hit",
                    "capsule_id": (!warm.capsule_id.is_empty()).then_some(warm.capsule_id.as_str()),
                    "form": warm.form.as_str(),
                });
                if let Some(arm) = &warm.eval_arm {
                    record["eval_arm"] = arm.clone().into();
                }
                Some(record)
            }
        }
    }
}

fn miss(reason: &'static str, text: impl Into<String>) -> Decision {
    Decision::Miss {
        reason,
        text: text.into(),
    }
}

/// The engine under the two names a warm session calls, in a folder of the
/// Harness state that goes first on the provider's `PATH`. Copies, so that
/// their mode is their own; one set per engine build.
pub fn stage_tools(runtime: &Runtime) -> Result<PathBuf> {
    stage_tools_for(runtime, true)
}

fn stage_tools_for(runtime: &Runtime, include_run: bool) -> Result<PathBuf> {
    let engine = match &runtime.engine {
        Some(engine) => engine.clone(),
        None => std::env::current_exe()?,
    };
    let digest = runtime.engine_digest();
    if digest.len() < 16 {
        return Err(HarnessError::unavailable(
            "The engine cannot be identified.",
        ));
    }
    let folder = if include_run {
        "skillcache/bin"
    } else {
        "skillcache/bin-tool"
    };
    let root = runtime.store.state.join(folder);
    let bin = root.join(&digest[..16]);
    infra_core::fs::ensure_directory(&bin, true)?;
    let name = |tool: &str| bin.join(format!("{tool}{}", std::env::consts::EXE_SUFFIX));
    let stage = |target: &Path| -> Result<()> {
        if !target.is_file() {
            let staging = bin.join(format!(".staging-{}", crate::store::hexid()));
            #[cfg(unix)]
            let linked = std::os::unix::fs::symlink(&engine, &staging).is_ok();
            #[cfg(not(unix))]
            let linked = false;
            if !linked {
                std::fs::copy(&engine, &staging)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o500))?;
                }
            }
            std::fs::rename(&staging, target)?;
        }
        Ok(())
    };
    let tool = name("skillcache-tool");
    if include_run {
        let first = name("skillcache-run");
        stage(&first)?;
        if std::fs::symlink_metadata(&first).is_ok_and(|meta| meta.file_type().is_symlink())
            || (!tool.is_file() && std::fs::hard_link(&first, &tool).is_err())
        {
            stage(&tool)?;
        }
    } else {
        stage(&tool)?;
    }
    // An older build's copies are of no use to anyone.
    for entry in std::fs::read_dir(&root).into_iter().flatten().flatten() {
        if entry.path() != bin {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
    Ok(bin)
}

fn protocol(skill: &str) -> String {
    format!(
        "How to run it:\n\
         - Start with `skillcache-run run '{{\"name\": value, ...}}'`: one JSON object, as one shell argument, with the values above. If quoting is awkward, run `skillcache-run run - <<'EOF'` with the JSON on standard input.\n\
         - Every answer is one JSON object. Do what its `status` says:\n\
         \x20 - `needs_bindings`: supply the named values with `skillcache-run continue '{{...}}'`.\n\
         \x20 - `your_steps`: do those steps yourself with your normal tools, then `skillcache-run continue '{{...}}'` with what they produce.\n\
         \x20 - `program_failed`: do that one step yourself with your normal tools, following the source skill at `{skill}`. Do not repair or re-run the program. Then `skillcache-run continue '{{...}}'` with its `required_outputs`. The rest of the plan stays compiled.\n\
         \x20 - `complete`: the plan is done; `result` holds what the programs found. Answer from it.\n\
         - If the initial bindings were wrong and the run is not finished, `skillcache-run reset` clears state so you can restart with `skillcache-run run`.\n\
         - Do not start the programs yourself and do not look for the capsule's files.\n\
         - If `skillcache-run` itself cannot be started, read `{skill}` and follow the skill as usual.\n"
    )
}

/// What a hit puts in the prompt in place of the skill request.
fn block(stored: &package::Package, skill_path: &str) -> String {
    let capsule = &stored.capsule;
    let mut text = format!(
        "\n\nSkillCache capsule for this request: {} — {}\n",
        capsule.mu.name, capsule.mu.description
    );
    if let Some(markdown) = &stored.markdown {
        text.push_str(&format!(
            "This written procedure was compiled from the skill at `{skill_path}`. Follow it instead of reading the skill; read the skill only where the procedure does not cover what you meet.\n\n{}\n",
            markdown.trim()
        ));
        return text;
    }
    text.push_str(&format!(
        "The skill at `{skill_path}` was compiled into a short plan. Do not read the skill first: a local executor runs the plan's programs, and you do only what it hands back to you.\n"
    ));
    if !capsule.bindings.is_empty() {
        text.push_str("\nValues to take from the request before you start:\n");
        for binding in &capsule.bindings {
            text.push_str(&format!(
                "- `{}` ({}){}\n",
                binding.name,
                binding.kind.as_str(),
                binding
                    .description
                    .as_deref()
                    .map(|description| format!(": {description}"))
                    .unwrap_or_default()
            ));
        }
    }
    let opening = capsule
        .steps
        .iter()
        .take_while(|step| matches!(step, Step::Instruction { .. }))
        .collect::<Vec<_>>();
    if !opening.is_empty() {
        text.push_str("\nYour opening steps (do them yourself, then start the executor with what they produce as well):\n");
        for (index, step) in opening.iter().enumerate() {
            if let Step::Instruction {
                text: instruction,
                source,
                produces,
            } = step
            {
                text.push_str(&format!("{}. {instruction}", index + 1));
                if let Some(source) = source {
                    text.push_str(&format!(
                        " (source: `{}`{})",
                        source.path,
                        source
                            .section
                            .as_deref()
                            .map(|section| format!(", section \"{section}\""))
                            .unwrap_or_default()
                    ));
                }
                if !produces.is_empty() {
                    text.push_str(&format!(" Produces: {}.", produces.join(", ")));
                }
                text.push('\n');
            }
        }
    }
    text.push('\n');
    text.push_str(&protocol(skill_path));
    text
}

/// Deterministic fallback when multiple eligible active capsules match one skill:
/// lowest smoothed recent post-requalify failure/incomplete rate, then hybrid form,
/// then newest update.
fn best_by_health_and_recency(store: &SessionStore, candidates: &[Value]) -> Value {
    let db = store.connection().ok();
    let health_rate = |capsule_id: &str| -> f64 {
        let (total, unhealthy) = db
            .as_ref()
            .and_then(|conn| {
                conn.query_row(
                    "SELECT COUNT(*),
                            COALESCE(SUM(CASE WHEN deopts > 0 OR outcome IN ('unfinished','not_started') THEN 1 ELSE 0 END), 0)
                     FROM skillcache_runs
                     WHERE capsule_id=?1
                       AND rowid>COALESCE((SELECT runs_reset_rowid FROM skillcache_capsules WHERE capsule_id=?1),0)",
                    [capsule_id],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )
                .ok()
            })
            .unwrap_or((0, 0));
        (unhealthy as f64 + 0.5) / (total as f64 + 3.0)
    };
    candidates
        .iter()
        .min_by(|a, b| {
            let ra = health_rate(a["capsule_id"].as_str().unwrap_or(""));
            let rb = health_rate(b["capsule_id"].as_str().unwrap_or(""));
            let ha = a["form"] != "hybrid";
            let hb = b["form"] != "hybrid";
            let ua = a["updated_at"].as_str().unwrap_or("");
            let ub = b["updated_at"].as_str().unwrap_or("");
            let ida = a["capsule_id"].as_str().unwrap_or("");
            let idb = b["capsule_id"].as_str().unwrap_or("");
            ra.total_cmp(&rb)
                .then_with(|| ha.cmp(&hb))
                .then_with(|| ub.cmp(ua))
                .then_with(|| ida.cmp(idb))
        })
        .cloned()
        .unwrap_or_else(|| candidates[0].clone())
}

fn is_read_only_capsule(capsule_row: &Value) -> bool {
    if capsule_row["form"] == "textual" {
        return true;
    }
    let warm_dir = PathBuf::from(capsule_row["artifact_dir"].as_str().unwrap_or("")).join("warm");
    package::load(&warm_dir).is_ok_and(|stored| {
        stored
            .capsule
            .programs
            .iter()
            .all(|program| program.read_only)
    })
}

/// Among several active capsules of one skill, the one whose description
/// matches the request best. Used only behind [`RETRIEVAL_BY_MU`].
fn best_by_description(
    store: &SessionStore,
    project_id: &str,
    ids: &[String],
    request: &str,
) -> Option<String> {
    let words = request
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| word.chars().count() >= 3)
        .take(24)
        .map(|word| format!("\"{}\"", word.replace('"', "")))
        .collect::<Vec<_>>();
    if words.is_empty() {
        return None;
    }
    let db = store.connection().ok()?;
    let mut query = db
        .prepare(
            "SELECT capsule_id FROM skillcache_capsules_fts
             WHERE skillcache_capsules_fts MATCH ?1 AND project_id=?2
             ORDER BY bm25(skillcache_capsules_fts)",
        )
        .ok()?;
    let ranked = query
        .query_map(rusqlite::params![words.join(" OR "), project_id], |row| {
            row.get::<_, String>(0)
        })
        .ok()?
        .flatten()
        .collect::<Vec<_>>();
    ranked.into_iter().find(|id| ids.contains(id))
}

/// Decides whether this turn runs a capsule. Never fails the turn: whatever
/// cannot be established is a miss with its reason.
pub fn decide(
    runtime: &Runtime,
    session: &Value,
    route: &ResolvedCommand,
    message: &str,
    workspace: &Path,
    generation: &str,
) -> Decision {
    decide_for(
        runtime,
        session,
        route,
        message,
        workspace,
        Some(generation),
    )
}

/// What [`decide`] would say for a message that has not been sent. Nothing is
/// staged and no capsule changes its status: a preview has no effect.
pub fn preview(
    runtime: &Runtime,
    session: &Value,
    route: &ResolvedCommand,
    message: &str,
    workspace: &Path,
) -> Value {
    let decision = decide_for(runtime, session, route, message, workspace, None);
    let mode = settings::mode(session);
    match &decision {
        Decision::Off => json!({"mode": mode, "decision": "off"}),
        Decision::Miss { reason, text } => {
            json!({"mode": mode, "decision": "miss", "reason": reason, "text": text})
        }
        Decision::Hit(warm) => {
            let mut answer = json!({
                "mode": mode,
                "decision": "hit",
                "capsule": if warm.capsule_id.is_empty() {
                    Value::Null
                } else {
                    json!({
                        "capsule_id": warm.capsule_id,
                        "mu": warm.name,
                        "form": warm.form.as_str(),
                    })
                },
            });
            if let Some(arm) = &warm.eval_arm {
                answer["eval_arm"] = arm.clone().into();
            }
            answer
        }
    }
}

const RAW_TRACE_VIEW_BYTES: usize = 128 * 1024;

fn capsule_matches_arm(capsule: &Value, arm: Option<&str>) -> bool {
    let cap_arm = capsule["eval_arm"].as_str();
    match arm {
        None => cap_arm.is_none(),
        Some("written_procedure") => cap_arm == Some("written_procedure"),
        Some("skillcache_no_replay") => cap_arm == Some("skillcache_no_replay"),
        Some("skillcache_agent" | "skillcache_executor") => cap_arm.is_none() || cap_arm == arm,
        Some(_) => false,
    }
}

fn decide_raw_trace(
    runtime: &Runtime,
    session: &Value,
    route: &ResolvedCommand,
    workspace: &Path,
) -> Decision {
    let project_id = session["project_id"].as_str().unwrap_or("");
    let db = match runtime.store.connection() {
        Ok(db) => db,
        Err(error) => return miss("unavailable", error.to_string()),
    };
    let stored = if let Some(trace_gen) = settings::trace_generation(session) {
        let trace = match db::trace(&db, trace_gen) {
            Ok(trace) => trace,
            Err(error) => return miss("unavailable", error.to_string()),
        };
        match trace {
            Some(trace) => {
                let trace_sid = trace["session_id"].as_str().unwrap_or("");
                let trace_project = db
                    .query_row(
                        "SELECT project_id FROM sessions WHERE id=?1",
                        [trace_sid],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .ok()
                    .flatten();
                if trace_project.as_deref() != Some(project_id) {
                    None
                } else {
                    Some(trace)
                }
            }
            None => None,
        }
    } else if route.requests.len() == 1 {
        let request = &route.requests[0];
        let skill_name = request["name"].as_str().unwrap_or("");
        let skill_path = request["path"]
            .as_str()
            .and_then(|path| trace::spelled_skill(workspace, Path::new(path)));
        let found = if !skill_name.is_empty() {
            db::latest_trace_for_skill(&db, project_id, skill_name)
        } else {
            Ok(None)
        };
        match found {
            Ok(Some(trace)) => Some(trace),
            Ok(None) => match skill_path {
                Some(path) => match db::latest_trace_for_skill(&db, project_id, &path) {
                    Ok(trace) => trace,
                    Err(error) => return miss("unavailable", error.to_string()),
                },
                None => None,
            },
            Err(error) => return miss("unavailable", error.to_string()),
        }
    } else {
        return miss(
            "no_single_skill",
            "the message names no skill and no `trace_generation` was given",
        );
    };
    let Some(stored) = stored else {
        return miss(
            "no_trace",
            "no observed trace exists for `raw_trace` evaluation",
        );
    };
    let steps = stored["steps"].as_array().cloned().unwrap_or_default();
    let view = serde_json::to_string_pretty(&sc_trace::compiler_view(&steps, RAW_TRACE_VIEW_BYTES))
        .unwrap_or_else(|_| "[]".into());
    let name = stored["skill_refs"]
        .as_array()
        .and_then(|skills| skills.first())
        .and_then(|skill| skill["name"].as_str())
        .unwrap_or("raw_trace")
        .to_string();
    let block = format!(
        "\n\nObserved trace reference for this request (`raw_trace` evaluation arm):\n\
         Prior request: {}\n\
         Credential-redacted observed trace:\n```json\n{}\n```\n\
         Complete the request with your normal tools using this trace as reference.\n",
        stored["request"].as_str().unwrap_or(""),
        view
    );
    Decision::Hit(Arc::new(Warm {
        capsule_id: String::new(),
        form: Form::Textual,
        eval_arm: Some("raw_trace".into()),
        name,
        block,
        environment: BTreeMap::new(),
        bin: None,
        package: None,
        state_dir: None,
        seen: AtomicU64::new(0),
        seen_changes: AtomicU64::new(0),
    }))
}

fn message_matches_mu(message: &str, capsule: &Value, hint: Option<&str>) -> bool {
    let mu_name = capsule["mu"]["name"].as_str().unwrap_or("");
    if let Some(hint) = hint
        && hint.eq_ignore_ascii_case(mu_name)
    {
        return true;
    }
    let tokenize = |text: &str| -> std::collections::BTreeSet<String> {
        text.split(|ch: char| !ch.is_ascii_alphanumeric())
            .filter(|tok| tok.len() >= 3)
            .map(|tok| tok.to_ascii_lowercase())
            .collect()
    };
    let msg_tokens = tokenize(message);
    if msg_tokens.is_empty() {
        return false;
    }
    let desc = capsule["mu"]["description"].as_str().unwrap_or("");
    let mu_tokens = tokenize(&format!("{mu_name} {desc}"));
    msg_tokens.iter().any(|tok| mu_tokens.contains(tok))
}

fn decide_toolchain_candidate(
    runtime: &Runtime,
    session: &Value,
    provider: &str,
    arm: Option<&str>,
    plan_only: bool,
    workspace: &Path,
    message: &str,
) -> std::result::Result<(Value, String), Decision> {
    let project_id = session["project_id"].as_str().unwrap_or("");
    let candidates = match runtime.store.connection().and_then(|db| match arm {
        None => db::active_toolchain_capsules_for_project(&db, project_id, None),
        Some("written_procedure" | "skillcache_no_replay") => {
            db::active_toolchain_capsules_for_project(&db, project_id, arm)
        }
        Some("skillcache_agent" | "skillcache_executor") => {
            let dedicated = db::active_toolchain_capsules_for_project(&db, project_id, arm)?;
            Ok(if dedicated.is_empty() {
                db::active_toolchain_capsules_for_project(&db, project_id, None)?
            } else {
                dedicated
            })
        }
        Some(_) => Ok(Vec::new()),
    }) {
        Ok(rows) => rows,
        Err(error) => return Err(miss("unavailable", error.to_string())),
    };
    let hint = settings::skill_hint(session);
    let matching: Vec<Value> = candidates
        .into_iter()
        .filter(|capsule| message_matches_mu(message, capsule, hint))
        .collect();
    if matching.is_empty() {
        return Err(miss("no_single_skill", "the message names no skill"));
    }
    let has_any = !matching.is_empty();
    let eligible: Vec<Value> = if provider == "cursor" {
        matching
            .into_iter()
            .filter(|capsule| capsule["form"] == "textual")
            .collect()
    } else {
        matching
    };
    if eligible.is_empty() {
        if has_any && provider == "cursor" {
            return Err(miss(
                "provider",
                "Cursor runs textual capsules only; no active textual tool-chain capsule exists",
            ));
        }
        return Err(miss("no_single_skill", "the message names no skill"));
    }
    let had_provider_eligible = !eligible.is_empty();
    let eligible: Vec<Value> = if plan_only {
        eligible.into_iter().filter(is_read_only_capsule).collect()
    } else {
        eligible
    };
    if eligible.is_empty() {
        if plan_only && had_provider_eligible {
            return Err(miss(
                "plan_mode",
                "a capsule's programs write, and this is a read-only turn",
            ));
        }
        return Err(miss("no_single_skill", "the message names no skill"));
    }
    let fresh: Vec<Value> = eligible
        .iter()
        .filter(|capsule| staleness(capsule, workspace).is_none())
        .cloned()
        .collect();
    let pool = if fresh.is_empty() { &eligible } else { &fresh };
    let chosen = match pool.len() {
        1 => pool[0].clone(),
        _ => best_by_health_and_recency(&runtime.store, pool),
    };
    Ok((chosen, String::new()))
}

fn decide_for(
    runtime: &Runtime,
    session: &Value,
    route: &ResolvedCommand,
    message: &str,
    workspace: &Path,
    generation: Option<&str>,
) -> Decision {
    if !settings::warm(session) {
        return Decision::Off;
    }
    let arm = settings::eval_arm(session);
    if arm == Some("cold") {
        return Decision::Off;
    }
    let provider = session["provider"].as_str().unwrap_or("");
    let manual =
        crate::permissions::mode(session) == Some(infra_host::permission::PermissionMode::Manual);
    let bypass =
        crate::permissions::mode(session) == Some(infra_host::permission::PermissionMode::Bypass);
    if route.mode == "native" {
        return miss(
            "native_route",
            "a native command goes to the CLI as typed, without Harness instructions",
        );
    }
    let plan_only = session["mode"] != "edit";
    if !["claude", "codex", "cursor"].contains(&provider) {
        return miss(
            "provider",
            "warm turns are verified for Claude, Codex, and Cursor (textual) only",
        );
    }
    if provider == "cursor" && arm == Some("skillcache_executor") {
        return miss(
            "provider",
            "hybrid executor capsules run on Claude and Codex only",
        );
    }
    if provider == "codex" && manual && !matches!(arm, Some("raw_trace" | "written_procedure")) {
        return miss(
            "codex_manual",
            "Codex in Manual mode would ask before every step of the executor",
        );
    }
    if arm == Some("raw_trace") {
        return decide_raw_trace(runtime, session, route, workspace);
    }
    let project_id = session["project_id"].as_str().unwrap_or("");
    let pin = settings::capsule_pin(session);
    let (chosen, skill_path) = if let Some(pinned_id) = pin {
        if route.requests.len() > 1 {
            return miss("no_single_skill", "the message names more than one skill");
        }
        let requested_skill_path = if route.requests.len() == 1 {
            match route.requests[0]["path"]
                .as_str()
                .and_then(|path| trace::spelled_skill(workspace, Path::new(path)))
            {
                Some(path) => Some(path),
                None => {
                    return miss(
                        "no_capsule",
                        "the routed skill is not a skill folder of this project or of your home",
                    );
                }
            }
        } else {
            None
        };
        let requested_skill_name = route
            .requests
            .first()
            .and_then(|req| req["name"].as_str())
            .unwrap_or("");
        let requested_tree_sha = requested_skill_path
            .as_deref()
            .and_then(|path| trace::skill_file(workspace, path))
            .and_then(|file| file.parent().map(Path::to_path_buf))
            .and_then(|dir| infra_skillcache::fingerprint::tree_sha256(&dir).ok())
            .map(|tree| tree.sha256)
            .unwrap_or_default();
        let row = match runtime
            .store
            .connection()
            .and_then(|db| db::capsule(&db, pinned_id))
        {
            Ok(Some(row)) => row,
            Ok(None) => return miss("no_capsule", "pinned capsule not found"),
            Err(error) => return miss("unavailable", error.to_string()),
        };
        if row["project_id"] != project_id
            || row["status"] != "active"
            || !capsule_matches_arm(&row, arm)
            || requested_skill_path.as_deref().is_some_and(|path| {
                !db::capsule_matches_skill_or_mirror(
                    &row,
                    path,
                    requested_skill_name,
                    &requested_tree_sha,
                )
            })
        {
            return miss(
                "no_capsule",
                "pinned capsule is not active for this project, skill, and evaluation arm",
            );
        }
        if provider == "cursor" && row["form"] != "textual" {
            return miss(
                "provider",
                "Cursor runs textual capsules only; hybrid capsules require Claude or Codex",
            );
        }
        if plan_only && !is_read_only_capsule(&row) {
            return miss(
                "plan_mode",
                "a capsule's programs write, and this is a read-only turn",
            );
        }
        let skill_path = row["skill_path"].as_str().unwrap_or("").to_string();
        (row, skill_path)
    } else if route.requests.is_empty() && settings::tool_chain_enabled(&runtime.store, session) {
        match decide_toolchain_candidate(
            runtime, session, provider, arm, plan_only, workspace, message,
        ) {
            Ok(chosen) => chosen,
            Err(missed) => return missed,
        }
    } else {
        if route.requests.len() != 1 {
            return miss(
                "no_single_skill",
                if route.requests.is_empty() {
                    "the message names no skill"
                } else {
                    "the message names more than one skill"
                },
            );
        }
        let request = &route.requests[0];
        let Some(skill_path) = request["path"]
            .as_str()
            .and_then(|path| trace::spelled_skill(workspace, Path::new(path)))
        else {
            return miss(
                "no_capsule",
                "the routed skill is not a skill folder of this project or of your home",
            );
        };
        let skill_name = request["name"].as_str().unwrap_or("");
        let skill_tree_sha = trace::skill_file(workspace, &skill_path)
            .and_then(|file| file.parent().map(Path::to_path_buf))
            .and_then(|dir| infra_skillcache::fingerprint::tree_sha256(&dir).ok())
            .map(|tree| tree.sha256)
            .unwrap_or_default();
        let candidates = match runtime.store.connection().and_then(|db| match arm {
            None => db::active_capsules_for_skill_or_mirror(
                &db,
                project_id,
                &skill_path,
                skill_name,
                &skill_tree_sha,
                None,
            ),
            Some("written_procedure" | "skillcache_no_replay") => {
                db::active_capsules_for_skill_or_mirror(
                    &db,
                    project_id,
                    &skill_path,
                    skill_name,
                    &skill_tree_sha,
                    arm,
                )
            }
            Some("skillcache_agent" | "skillcache_executor") => {
                let dedicated = db::active_capsules_for_skill_or_mirror(
                    &db,
                    project_id,
                    &skill_path,
                    skill_name,
                    &skill_tree_sha,
                    arm,
                )?;
                Ok(if dedicated.is_empty() {
                    db::active_capsules_for_skill_or_mirror(
                        &db,
                        project_id,
                        &skill_path,
                        skill_name,
                        &skill_tree_sha,
                        None,
                    )?
                } else {
                    dedicated
                })
            }
            Some(_) => Ok(Vec::new()),
        }) {
            Ok(candidates) => candidates,
            Err(error) => return miss("unavailable", error.to_string()),
        };
        let has_any = !candidates.is_empty();
        let eligible = if provider == "cursor" {
            candidates
                .into_iter()
                .filter(|capsule| capsule["form"] == "textual")
                .collect::<Vec<_>>()
        } else {
            candidates
        };
        if eligible.is_empty() {
            if has_any && provider == "cursor" {
                return miss(
                    "provider",
                    "Cursor runs textual capsules only; no active textual capsule exists for this skill",
                );
            }
            return miss("no_capsule", "no qualified capsule exists for this skill");
        }
        let had_provider_eligible = !eligible.is_empty();
        let eligible = if plan_only {
            eligible
                .into_iter()
                .filter(is_read_only_capsule)
                .collect::<Vec<_>>()
        } else {
            eligible
        };
        let chosen = match eligible.len() {
            0 if plan_only && had_provider_eligible => {
                return miss(
                    "plan_mode",
                    "a capsule's programs write, and this is a read-only turn",
                );
            }
            0 => return miss("no_capsule", "no qualified capsule exists for this skill"),
            1 => eligible[0].clone(),
            _ => {
                let by_description = runtime
                    .store
                    .setting(RETRIEVAL_BY_MU)
                    .ok()
                    .flatten()
                    .is_some_and(|value| value == true);
                let ids = eligible
                    .iter()
                    .filter_map(|capsule| capsule["capsule_id"].as_str().map(String::from))
                    .collect::<Vec<_>>();
                let best = by_description
                    .then(|| best_by_description(&runtime.store, project_id, &ids, message))
                    .flatten();
                match best.and_then(|id| {
                    eligible
                        .iter()
                        .find(|capsule| capsule["capsule_id"] == id.as_str())
                        .cloned()
                }) {
                    Some(capsule) => capsule,
                    None => best_by_health_and_recency(&runtime.store, &eligible),
                }
            }
        };
        (chosen, skill_path)
    };
    let capsule_id = chosen["capsule_id"].as_str().unwrap_or("").to_string();
    if let Some(reason) = staleness(&chosen, workspace) {
        if generation.is_some() {
            if let Ok(db) = runtime.store.connection() {
                let _ = db::set_capsule_status(&db, &capsule_id, "stale", Some(&reason));
            }
            runtime.publish(
                project_id,
                "stale",
                json!({"capsule_id": capsule_id, "reason": reason}),
            );
        }
        return miss("stale", reason);
    }
    let warm_dir = PathBuf::from(chosen["artifact_dir"].as_str().unwrap_or("")).join("warm");
    let stored = match package::load(&warm_dir) {
        Ok(stored) => stored,
        Err(error) => return miss("unavailable", error.to_string()),
    };
    // Codex outside Bypass runs its commands without the network.
    if stored.capsule.capabilities.network == Network::Required && provider == "codex" && !bypass {
        return miss(
            "network",
            "the capsule's programs need the network and Codex's sandbox has none",
        );
    }
    let name = stored.capsule.mu.name.clone();
    let skill_label = if skill_path.is_empty() {
        "workspace tool-chain"
    } else {
        skill_path.as_str()
    };
    let agent_only = arm == Some("skillcache_agent");
    let text = if agent_only {
        prompt::render_agent_driven(&stored, skill_label)
    } else {
        block(&stored, skill_label)
    };
    let eval_arm = arm.map(String::from);
    let (true, Some(generation)) = (stored.capsule.form == Form::Hybrid, generation) else {
        // A written procedure is read, and a preview stages nothing.
        return Decision::Hit(Arc::new(Warm {
            capsule_id,
            form: stored.capsule.form,
            eval_arm,
            name,
            block: text,
            environment: BTreeMap::new(),
            bin: None,
            package: None,
            state_dir: None,
            seen: AtomicU64::new(0),
            seen_changes: AtomicU64::new(0),
        }));
    };
    let staged = (|| -> Result<(PathBuf, PathBuf)> {
        let bin = stage_tools_for(runtime, !agent_only)?;
        let state_dir = runtime
            .store
            .state
            .join("skillcache/sessions")
            .join(session["id"].as_str().unwrap_or("session"))
            .join(generation);
        infra_core::fs::ensure_directory(&state_dir, true)?;
        Ok((bin, state_dir))
    })();
    let (bin, state_dir) = match staged {
        Ok(staged) => staged,
        Err(error) => return miss("unavailable", error.to_string()),
    };
    let text_of = |path: &Path| path.display().to_string();
    let mut environment = BTreeMap::from([
        (env_filter::PACKAGE.to_string(), text_of(&warm_dir)),
        (env_filter::WORKSPACE.to_string(), text_of(workspace)),
        (env_filter::GENERATION.to_string(), generation.to_string()),
    ]);
    if agent_only {
        environment.insert(
            env_filter::LOG_FILE.to_string(),
            text_of(&state_dir.join("run-log.jsonl")),
        );
    } else {
        environment.insert(
            env_filter::STATE_FILE.to_string(),
            text_of(&state_dir.join("plan-state.json")),
        );
    }
    Decision::Hit(Arc::new(Warm {
        capsule_id,
        form: Form::Hybrid,
        eval_arm,
        name,
        block: text,
        environment,
        bin: Some(bin),
        package: Some(warm_dir),
        state_dir: Some(state_dir),
        seen: AtomicU64::new(0),
        seen_changes: AtomicU64::new(0),
    }))
}

/// Adds what a warm turn's provider process needs to the launch another
/// adapter prepared: the executor first on `PATH`, its variables, and leave
/// to call it without being asked.
pub struct WarmLaunchAdapter {
    pub inner: Arc<dyn LaunchAdapter>,
    pub warm: Arc<Warm>,
}
impl LaunchAdapter for WarmLaunchAdapter {
    fn adapt(&self, input: &Launch, command: PreparedCommand) -> Result<PreparedCommand> {
        let mut command = self.inner.adapt(input, command)?;
        let (Some(bin), Some(state_dir)) = (&self.warm.bin, &self.warm.state_dir) else {
            // A textual capsule is read; nothing is staged for it.
            return Ok(command);
        };
        let rest = env_filter::take_path(&mut command.environment)
            .or_else(|| std::env::var("PATH").ok())
            .unwrap_or_default();
        let path =
            std::env::join_paths(std::iter::once(bin.clone()).chain(std::env::split_paths(&rest)))
                .ok()
                .and_then(|joined| joined.into_string().ok())
                .ok_or_else(|| {
                    HarnessError::invalid("The executor's folder cannot be put on PATH.")
                })?;
        command.environment.insert("PATH".into(), path);
        command.environment.extend(self.warm.environment.clone());
        command.readable.push(bin.clone());
        if let Some(package) = &self.warm.package {
            command.readable.push(package.clone());
        }
        match input.provider.as_str() {
            "claude" => allow_claude(&mut command, !self.warm.agent_only())?,
            // Codex's sandbox writes only inside its workspace and the
            // folders it is given; the executor's state file is in neither.
            // Without the sandbox there is nothing to widen.
            "codex"
                if !command.argv.iter().any(|argument| {
                    argument == "app-server"
                        || argument == "--dangerously-bypass-approvals-and-sandbox"
                }) =>
            {
                let at = command
                    .argv
                    .iter()
                    .position(|argument| argument == "resume")
                    .unwrap_or_else(|| command.argv.len().saturating_sub(1));
                command.argv.splice(
                    at..at,
                    ["--add-dir".to_string(), state_dir.display().to_string()],
                );
            }
            _ => {}
        }
        Ok(command)
    }
}

/// Lets Claude Code run the SkillCache commands without asking.
fn allow_claude(command: &mut PreparedCommand, include_run: bool) -> Result<()> {
    let wrong = || HarnessError::invalid("Claude settings cannot take the SkillCache permission.");
    let index = command.argv.iter().position(|value| value == "--settings");
    let mut settings: Value = match index {
        Some(index) => serde_json::from_str(command.argv.get(index + 1).ok_or_else(wrong)?)?,
        None => json!({}),
    };
    let allow = settings
        .as_object_mut()
        .ok_or_else(wrong)?
        .entry("permissions")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(wrong)?
        .entry("allow")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(wrong)?;
    let rules: &[&str] = if include_run {
        &["Bash(skillcache-run:*)", "Bash(skillcache-tool:*)"]
    } else {
        &["Bash(skillcache-tool:*)"]
    };
    for &rule in rules {
        if !allow.iter().any(|known| known == rule) {
            allow.push(rule.into());
        }
    }
    match index {
        Some(index) => command.argv[index + 1] = settings.to_string(),
        None => command
            .argv
            .extend(["--settings".to_string(), settings.to_string()]),
    }
    Ok(())
}

fn phase_text(event: &Value) -> Option<(&'static str, String)> {
    let step = event["step_index"].as_u64().map(|index| index + 1);
    let program = event["program"].as_str().unwrap_or("");
    Some(match event["event"].as_str()? {
        "started" => (
            "started",
            format!(
                "SkillCache: the plan started ({} steps).",
                event["steps"].as_u64().unwrap_or_default()
            ),
        ),
        "needs_bindings" => (
            "needs_bindings",
            format!(
                "SkillCache: step {} needs {}.",
                step.unwrap_or_default(),
                event["names"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        "your_steps" => (
            "your_steps",
            "SkillCache: the next steps are the agent's own.".to_string(),
        ),
        "program_completed" => (
            "program_completed",
            format!(
                "SkillCache: program `{program}` ran (step {}).",
                step.unwrap_or_default()
            ),
        ),
        "program_failed" => (
            "program_failed",
            format!(
                "SkillCache: program `{program}` failed ({}); step {} is the agent's now, the rest of the plan stays compiled.",
                event["reason"].as_str().unwrap_or("failed"),
                step.unwrap_or_default()
            ),
        ),
        "interrupted" => (
            "interrupted",
            format!(
                "SkillCache: step {} was interrupted and is resumed.",
                step.unwrap_or_default()
            ),
        ),
        "complete" => (
            "complete",
            format!(
                "SkillCache: the plan is complete ({} failed).",
                event["failed"].as_u64().unwrap_or_default()
            ),
        ),
        _ => return None,
    })
}

impl Warm {
    fn log_file(&self) -> Option<PathBuf> {
        let dir = self.state_dir.as_ref()?;
        if self.agent_only() {
            Some(dir.join("run-log.jsonl"))
        } else {
            Some(state::log_path(&dir.join("plan-state.json")))
        }
    }
    /// Turns the executor's new journal lines into session events. A line is
    /// published once: the journal numbers them and the last one seen is kept.
    pub fn ingest(&self, emit: &dyn Fn(Value) -> Result<()>) {
        self.ingest_with_changes(emit, &|_| {});
    }

    /// Like [`Self::ingest`], also forwarding workspace `changed_files` mutations
    /// from the sibling `run-log.changes.jsonl` sidecar to `on_changes` as
    /// normalized `file_change` tool events for live focus tracking.
    pub fn ingest_with_changes(
        &self,
        emit: &dyn Fn(Value) -> Result<()>,
        on_changes: &dyn Fn(Value),
    ) {
        let Some(log_file) = self.log_file() else {
            return;
        };
        let after = self.seen.load(Ordering::Acquire);
        if let Ok(events) = state::read_log_at(&log_file, after) {
            for event in events {
                let Some(seq) = event["seq"].as_u64() else {
                    continue;
                };
                // Another caller may have published this line meanwhile.
                if self.seen.fetch_max(seq, Ordering::AcqRel) >= seq {
                    continue;
                }
                if let Some((phase, text)) = phase_text(&event) {
                    let _ = emit(json!({
                        "kind": "skillcache",
                        "phase": phase,
                        "capsule_id": self.capsule_id,
                        "seq": seq,
                        "step_index": event["step_index"],
                        "program": event["program"],
                        "reason": event["reason"],
                        "verification": event["verification"],
                        "text": text,
                    }));
                }
            }
        }
        let after_changes = self.seen_changes.load(Ordering::Acquire);
        if let Ok(change_events) =
            state::read_log_at(&state::changes_path(&log_file), after_changes)
        {
            for event in change_events {
                let Some(seq) = event["seq"].as_u64() else {
                    continue;
                };
                if self.seen_changes.fetch_max(seq, Ordering::AcqRel) >= seq {
                    continue;
                }
                if let Some(changes) = event["changes"].as_array().filter(|arr| !arr.is_empty()) {
                    on_changes(json!({
                        "kind": "tool",
                        "state": "completed",
                        "tool": "file_change",
                        "ok": true,
                        "changes": changes,
                    }));
                }
            }
        }
    }
    /// Records how the warm turn went and removes its working state.
    pub fn finish(
        &self,
        store: &SessionStore,
        session: &Value,
        generation: &str,
        emit: &dyn Fn(Value) -> Result<()>,
    ) {
        self.ingest(emit);
        let events = self
            .log_file()
            .and_then(|log_file| state::read_log_at(&log_file, 0).ok())
            .unwrap_or_default();
        let count = |name: &str| events.iter().filter(|event| event["event"] == name).count();
        let outcome = if self.form == Form::Textual {
            "read"
        } else if self.agent_only() {
            // The agent drives the programs itself, so nothing says "the plan
            // ended": it is complete when every program step of it ran to its
            // end and none failed, and unfinished when only some did.
            let planned = self
                .package
                .as_deref()
                .and_then(|dir| package::load(dir).ok())
                .map(|stored| {
                    stored
                        .capsule
                        .plan()
                        .filter(|step| step.program().is_some())
                        .count()
                });
            let completed = events
                .iter()
                .filter(|event| event["event"] == "program_completed")
                .filter_map(|event| event["step_index"].as_u64())
                .collect::<std::collections::BTreeSet<_>>()
                .len();
            let failed = count("program_failed");
            if failed == 0 && planned.is_some_and(|planned| planned > 0 && completed >= planned) {
                "complete"
            } else if completed > 0 || failed > 0 {
                "unfinished"
            } else {
                "not_started"
            }
        } else if count("complete") > 0 {
            "complete"
        } else if count("started") > 0 {
            "unfinished"
        } else {
            "not_started"
        };
        let steps = events
            .iter()
            .map(|event| {
                json!({
                    "seq": event["seq"], "event": event["event"], "step_index": event["step_index"],
                    "program": event["program"], "reason": event["reason"], "at_ms": event["at_ms"],
                })
            })
            .collect::<Vec<_>>();
        let deopts = count("program_failed") as i64;
        // Read before the connection is taken: the store has one, and both a
        // setting and a session event go through it.
        let thresholds = recompile_thresholds(store);
        let mut waits: Option<(String, Option<String>)> = None;
        if !self.capsule_id.is_empty()
            && let Ok(db) = store.connection()
        {
            let _ = db::record_run(
                &db,
                &db::Run {
                    generation,
                    session_id: session["id"].as_str().unwrap_or(""),
                    capsule_id: &self.capsule_id,
                    provider: session["provider"].as_str().unwrap_or(""),
                    model: session["model"].as_str(),
                    outcome,
                    steps: &Value::Array(steps),
                    deopts,
                },
            );
            let _ = db::touch_capsule(&db, &self.capsule_id);
            // Behind its setting only: enough failing warm turns since the
            // last qualification hand back to a person.
            if (deopts > 0 || matches!(outcome, "unfinished" | "not_started"))
                && let Some((window, failed)) = thresholds
                && let Ok(Some(cap)) = db::capsule(&db, &self.capsule_id)
                && let Some(since) = cap["updated_at"].as_str()
                && let Ok(recent) = db::capsule_runs_since(&db, &self.capsule_id, since, window)
            {
                let handed_back = recent
                    .iter()
                    .filter(|run| {
                        run["deopts"].as_i64().unwrap_or_default() > 0
                            || matches!(run["outcome"].as_str(), Some("unfinished" | "not_started"))
                    })
                    .count();
                let reason = format!(
                    "{handed_back} of its last {} warm turns handed a program step back.",
                    recent.len()
                );
                if handed_back >= failed
                    && db::set_capsule_status(
                        &db,
                        &self.capsule_id,
                        "needs_recompile",
                        Some(&reason),
                    )
                    .is_ok()
                {
                    let _ = db::mark_capsule_needs_trace(&db, &self.capsule_id);
                    let project_id = cap["project_id"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                        .map(String::from);
                    waits = Some((reason, project_id));
                }
            }
        }
        if let Some((reason, project_id)) = waits {
            store.publish_event(
                "skillcache_capsule",
                None,
                project_id.clone(),
                json!({
                    "state": "needs_recompile",
                    "project_id": project_id,
                    "capsule_id": self.capsule_id,
                    "reason": reason,
                    "needs_trace": true,
                }),
            );
            let _ = emit(json!({
                "kind": "skillcache",
                "phase": "needs_recompile",
                "capsule_id": self.capsule_id,
                "text": format!("SkillCache: {reason} The capsule is not used until it is requalified or compiled again."),
            }));
        }
        if let Some(state_dir) = &self.state_dir {
            let _ = std::fs::remove_dir_all(state_dir);
        }
    }

    /// Removes temporary state when a warm turn failed or was cancelled before finishing.
    pub(crate) fn cleanup_abandoned(&self) {
        if let Some(state_dir) = &self.state_dir {
            let _ = std::fs::remove_dir_all(state_dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use infra_skillcache::SkillCapsule;

    /// A capsule is not requalified when the engine is rebuilt, only when the
    /// executor's contract changes, and that is versioned by hand
    /// (`ENV_ABI_VERSION`). This pins what an agent is told of the contract
    /// and the variables a run gets to that version, so that changing either
    /// is noticed here. Raise the version when capsules qualified before the
    /// change must not run after it; then update the pin.
    #[test]
    fn the_executor_contract_is_pinned_to_its_version() {
        let told = [
            protocol("skill").as_str(),
            env_filter::INPUT,
            env_filter::WORKSPACE,
            env_filter::PACKAGE,
            env_filter::ASSETS,
            env_filter::STATE_FILE,
            env_filter::LOG_FILE,
            env_filter::GENERATION,
        ]
        .join("\n");
        assert_eq!(
            (
                infra_skillcache::fingerprint::ENV_ABI_VERSION,
                crate::store::sha(told.as_bytes()).as_str(),
            ),
            (
                1,
                "e09f4bc6eddef119136f540493b6062d6e987acb1f633d25d2ae67524b5ec1f6"
            ),
            "the executor's contract changed: decide on ENV_ABI_VERSION, then update this pin"
        );
    }

    fn stored(root: &Path) -> package::Package {
        let capsule = json!({
            "schema_version": 1, "form": "hybrid",
            "mu": {"name": "pdf-totals", "description": "Totals of a folder of invoice PDFs"},
            "source": {"skills": [{"name": "pdf", "path": ".agents/skills/pdf/SKILL.md", "tree_sha256": "0".repeat(64)}]},
            "capabilities": {"network": "none", "runtimes": ["python3"]},
            "bindings": [{"name": "input_dir", "type": "path", "origin": "request", "description": "Folder with the invoices"}],
            "programs": [{"id": "extract", "entry": ["python3", "programs/extract/main.py"],
                "inputs": ["input_dir", "quarter_files"], "outputs": [],
                "check": ["python3", "programs/extract/main.py", "--check"], "timeout_seconds": 60, "read_only": false}],
            "steps": [
                {"kind": "instruction", "text": "Confirm which invoices belong to the quarter.",
                 "source": {"path": ".agents/skills/pdf/SKILL.md", "section": "Selecting files"}, "produces": ["quarter_files"]},
                {"kind": "program", "program": "extract"}
            ],
            "verification": []
        });
        let parsed = SkillCapsule::parse(&serde_json::to_vec(&capsule).unwrap()).unwrap();
        let mut files = package::Files::new();
        files.insert(
            "programs/extract/main.py",
            b"SECRET_PROGRAM_SOURCE = 1\n".to_vec(),
        );
        let warm = root.join("warm");
        package::write_warm(&warm, &parsed, None, &files).unwrap();
        package::load(&warm).unwrap()
    }

    fn runtime(root: &Path) -> Runtime {
        Runtime {
            store: SessionStore::open(&root.join("state")).unwrap(),
            providers: Arc::new(crate::provider::ProviderDomain {
                providers: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
            }),
            engine: None,
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            enqueuer: None,
            requalifying: None,
        }
    }
    fn reason(decision: &Decision) -> &'static str {
        match decision {
            Decision::Off => "off",
            Decision::Miss { reason, .. } => reason,
            Decision::Hit(_) => "hit",
        }
    }

    #[test]
    fn a_turn_is_warm_only_for_what_a_capsule_was_qualified_for() {
        let root = crate::skillcache::test_support::scratch();
        let runtime = runtime(root.path());
        let session = |provider: &str, mode: &str, permissions: &str, skillcache: &str| {
            json!({
                "id": "s1", "project_id": "p1", "provider": provider, "mode": mode,
                "workflow": "native", "permissions": permissions,
                "skillcache": settings::validate(&json!({"mode": skillcache})).unwrap(),
            })
        };
        let route = |mode: &str, requests: Vec<Value>| ResolvedCommand {
            mode: mode.into(),
            text: "do it".into(),
            requests,
            notice: None,
        };
        let skill = json!({"name": "pdf", "path": "/nowhere/.agents/skills/pdf/SKILL.md"});
        let decide = |session: &Value, route: &ResolvedCommand| {
            reason(&super::decide(
                runtime_ref(&runtime),
                session,
                route,
                "do it",
                root.path(),
                "gen",
            ))
        };
        fn runtime_ref(runtime: &Runtime) -> &Runtime {
            runtime
        }
        let one = route("text", vec![skill.clone()]);
        // A session that did not ask is not looked at, whatever its turn is.
        assert_eq!(
            decide(&session("codex", "edit", "auto", "off"), &one),
            "off"
        );
        assert_eq!(
            decide(&session("codex", "edit", "auto", "observe"), &one),
            "off"
        );
        // warm_disabled_for_native_route: Claude Code runs a native command
        // itself; the Harness has no prompt to put a capsule in.
        assert_eq!(
            decide(
                &session("claude", "edit", "auto", "auto"),
                &route("native", vec![])
            ),
            "native_route"
        );
        assert_eq!(
            decide(&session("codex", "plan", "auto", "auto"), &one),
            "no_capsule"
        );
        assert_eq!(
            decide(&session("cursor", "edit", "auto", "warm_only"), &one),
            "no_capsule"
        );
        // warm_disabled_for_codex_manual
        assert_eq!(
            decide(&session("codex", "edit", "manual", "auto"), &one),
            "codex_manual"
        );
        // Claude in Manual is not Codex in Manual: it gets as far as the skill.
        assert_eq!(
            decide(&session("claude", "edit", "manual", "auto"), &one),
            "no_capsule"
        );
        assert_eq!(
            decide(
                &session("codex", "edit", "auto", "auto"),
                &route("text", vec![])
            ),
            "no_single_skill"
        );
        assert_eq!(
            decide(
                &session("codex", "edit", "auto", "auto"),
                &route("text", vec![skill.clone(), skill.clone()])
            ),
            "no_single_skill"
        );
        // A skill that is not a skill folder of the workspace or the home.
        assert_eq!(
            decide(&session("codex", "edit", "auto", "auto"), &one),
            "no_capsule"
        );
        // Every miss says why, and its launch record says so too.
        let decision = super::decide(
            &runtime,
            &session("codex", "edit", "manual", "auto"),
            &one,
            "do it",
            root.path(),
            "gen",
        );
        assert_eq!(decision.event().unwrap()["phase"], "miss");
        assert_eq!(
            decision.record().unwrap(),
            json!({"decision": "miss", "reason": "codex_manual"})
        );
        assert!(Decision::Off.event().is_none() && Decision::Off.record().is_none());
        // Nothing was staged for any of them.
        assert!(!root.path().join("state/skillcache/bin").exists());
        assert!(!root.path().join("state/skillcache/sessions").exists());
    }

    #[test]
    fn prompt_hit_block_excludes_examples_and_sources() {
        let root = crate::skillcache::test_support::scratch();
        let stored = stored(root.path());
        let text = block(&stored, ".agents/skills/pdf/SKILL.md");
        // What the agent needs: what the capsule is for, what to take from
        // the request, its own opening step and the protocol.
        for expected in [
            "pdf-totals — Totals of a folder of invoice PDFs",
            "`input_dir` (path): Folder with the invoices",
            "1. Confirm which invoices belong to the quarter.",
            "section \"Selecting files\"",
            "Produces: quarter_files.",
            "skillcache-run run",
            "`program_failed`",
            "following the source skill at `.agents/skills/pdf/SKILL.md`",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
        }
        // And nothing else: no program code, no program name or path, no
        // place where the package is.
        for withheld in [
            "SECRET_PROGRAM_SOURCE",
            "programs/extract",
            "main.py",
            root.path().to_str().unwrap(),
            "python3",
        ] {
            assert!(
                !text.contains(withheld),
                "the block shows {withheld:?}:\n{text}"
            );
        }
        package::remove(&root.path().join("warm")).unwrap();
    }

    #[test]
    fn textual_hit_prompt_has_capsule_md_only() {
        let root = crate::skillcache::test_support::scratch();
        let manifest = json!({
            "schema_version": 1, "form": "textual",
            "mu": {"name": "pdf-totals", "description": "Totals of a folder of invoice PDFs"},
            "source": {"skills": [{"name": "pdf", "path": ".agents/skills/pdf/SKILL.md", "tree_sha256": "0".repeat(64)}]}
        });
        let parsed = SkillCapsule::parse(&serde_json::to_vec(&manifest).unwrap()).unwrap();
        let warm = root.path().join("warm");
        package::write_warm(
            &warm,
            &parsed,
            Some(b"# Totals\n\n1. Read the invoices.\n"),
            &package::Files::new(),
        )
        .unwrap();
        let text = block(
            &package::load(&warm).unwrap(),
            ".agents/skills/pdf/SKILL.md",
        );
        assert!(text.contains("# Totals\n\n1. Read the invoices."));
        assert!(text.contains("read the skill only where the procedure does not cover"));
        // Nothing of the executor: a written procedure is read, not run.
        assert!(
            !text.contains("skillcache-run") && !text.contains("skillcache-tool"),
            "{text}"
        );
        package::remove(&warm).unwrap();
    }

    #[test]
    fn a_warm_launch_gets_the_executor_and_leave_to_call_it() {
        struct Plain;
        impl LaunchAdapter for Plain {
            fn adapt(&self, _: &Launch, command: PreparedCommand) -> Result<PreparedCommand> {
                Ok(command)
            }
        }
        let warm = Arc::new(Warm {
            capsule_id: "c1".into(),
            form: Form::Hybrid,
            eval_arm: None,
            name: "pdf-totals".into(),
            block: String::new(),
            environment: BTreeMap::from([(
                "SKILLCACHE_PACKAGE".to_string(),
                "/state/warm".to_string(),
            )]),
            bin: Some(PathBuf::from("/state/skillcache/bin/abc")),
            package: Some(PathBuf::from("/state/warm")),
            state_dir: Some(PathBuf::from("/state/skillcache/sessions/s/g")),
            seen: AtomicU64::new(0),
            seen_changes: AtomicU64::new(0),
        });
        let adapter = WarmLaunchAdapter {
            inner: Arc::new(Plain),
            warm: warm.clone(),
        };
        let launch = |provider: &str| Launch {
            provider: provider.into(),
            executable: PathBuf::from("/bin/true"),
            project: PathBuf::from("/work"),
            prompt: "x".into(),
            mode: "edit".into(),
            model: None,
            session: None,
            effort: None,
            agents: false,
            count: 1,
            usd: None,
            hook_events: false,
            permissions: None,
            approvals: None,
        };
        let command = |argv: &[&str]| PreparedCommand {
            argv: argv.iter().map(|argument| argument.to_string()).collect(),
            environment: BTreeMap::from([("PATH".to_string(), "/usr/bin".to_string())]),
            input: vec![],
            readable: vec![],
            remove: vec![],
            prompt_prefix: String::new(),
        };
        let claude = adapter
            .adapt(
                &launch("claude"),
                command(&[
                    "claude",
                    "--settings",
                    r#"{"env":{"A":"1"},"permissions":{"allow":["Bash(ls:*)"]}}"#,
                ]),
            )
            .unwrap();
        assert!(claude.environment["PATH"].starts_with("/state/skillcache/bin/abc"));
        assert!(claude.environment["PATH"].ends_with("/usr/bin"));
        assert_eq!(claude.environment["SKILLCACHE_PACKAGE"], "/state/warm");
        let settings: Value = serde_json::from_str(&claude.argv[2]).unwrap();
        assert_eq!(
            settings["permissions"]["allow"],
            json!([
                "Bash(ls:*)",
                "Bash(skillcache-run:*)",
                "Bash(skillcache-tool:*)"
            ])
        );
        assert_eq!(
            settings["env"]["A"], "1",
            "what the launch already set stays"
        );
        // The package is the executor's to read, not the model's: no --add-dir for it.
        assert!(!claude.argv.iter().any(|argument| argument == "--add-dir"));
        let codex = adapter
            .adapt(
                &launch("codex"),
                command(&[
                    "codex",
                    "--sandbox",
                    "workspace-write",
                    "exec",
                    "--json",
                    "-",
                ]),
            )
            .unwrap();
        assert_eq!(
            codex.argv,
            [
                "codex",
                "--sandbox",
                "workspace-write",
                "exec",
                "--json",
                "--add-dir",
                "/state/skillcache/sessions/s/g",
                "-"
            ]
        );
        let codex_resume = adapter
            .adapt(
                &launch("codex"),
                command(&[
                    "codex",
                    "--sandbox",
                    "workspace-write",
                    "exec",
                    "--skip-git-repo-check",
                    "resume",
                    "--json",
                    "thread-123",
                    "-",
                ]),
            )
            .unwrap();
        assert_eq!(
            codex_resume.argv,
            [
                "codex",
                "--sandbox",
                "workspace-write",
                "exec",
                "--skip-git-repo-check",
                "--add-dir",
                "/state/skillcache/sessions/s/g",
                "resume",
                "--json",
                "thread-123",
                "-",
            ]
        );
        // A textual capsule stages nothing and changes nothing of the launch.
        let textual = WarmLaunchAdapter {
            inner: Arc::new(Plain),
            warm: Arc::new(Warm {
                capsule_id: "c2".into(),
                form: Form::Textual,
                eval_arm: None,
                name: "t".into(),
                block: String::new(),
                environment: BTreeMap::new(),
                bin: None,
                package: None,
                state_dir: None,
                seen: AtomicU64::new(0),
                seen_changes: AtomicU64::new(0),
            }),
        };
        let plain = textual
            .adapt(&launch("claude"), command(&["claude"]))
            .unwrap();
        assert_eq!(plain.argv, ["claude"]);
        assert_eq!(plain.environment.len(), 1);
    }

    #[test]
    fn run_log_ingest_idempotent_emits_session_events() {
        let root = crate::skillcache::test_support::scratch();
        let state_dir = root.path().join("state");
        std::fs::create_dir_all(&state_dir).unwrap();
        let state_file = state_dir.join("plan-state.json");
        let mut log = state::RunLog::open(&state_file, "gen").unwrap();
        log.append("started", json!({"steps": 3})).unwrap();
        log.append(
            "program_started",
            json!({"step_index": 0, "program": "extract", "attempt": 1}),
        )
        .unwrap();
        log.append(
            "program_completed",
            json!({"step_index": 0, "program": "extract"}),
        )
        .unwrap();
        let warm = Warm {
            capsule_id: "c1".into(),
            form: Form::Hybrid,
            eval_arm: None,
            name: "pdf-totals".into(),
            block: String::new(),
            environment: BTreeMap::new(),
            bin: Some(root.path().join("bin")),
            package: None,
            state_dir: Some(state_dir.clone()),
            seen: AtomicU64::new(0),
            seen_changes: AtomicU64::new(0),
        };
        let seen = std::sync::Mutex::new(Vec::<Value>::new());
        let emit = |event: Value| -> Result<()> {
            seen.lock().unwrap().push(event);
            Ok(())
        };
        warm.ingest(&emit);
        warm.ingest(&emit);
        let phases = |seen: &std::sync::Mutex<Vec<Value>>| {
            seen.lock()
                .unwrap()
                .iter()
                .map(|event| event["phase"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        };
        // A program's start is not an event of its own; reading twice repeats nothing.
        assert_eq!(phases(&seen), ["started", "program_completed"]);
        log.append(
            "program_failed",
            json!({"step_index": 1, "program": "summarize", "reason": "timeout", "verification": false}),
        )
        .unwrap();
        log.append("complete", json!({"failed": 1, "completed": 2}))
            .unwrap();
        warm.ingest(&emit);
        assert_eq!(
            phases(&seen),
            ["started", "program_completed", "program_failed", "complete"]
        );
        let failed = seen.lock().unwrap()[2].clone();
        assert_eq!(failed["kind"], "skillcache");
        assert_eq!(failed["capsule_id"], "c1");
        assert_eq!(failed["step_index"], 1);
        assert_eq!(failed["reason"], "timeout");
        assert!(
            failed["text"]
                .as_str()
                .unwrap()
                .contains("the rest of the plan stays compiled")
        );
        // A torn last line, as a crash leaves it, is not an event and not an error.
        std::fs::OpenOptions::new()
            .append(true)
            .open(state::log_path(&state_file))
            .and_then(|mut file| {
                std::io::Write::write_all(&mut file, b"{\"seq\":9,\"event\":\"comp")
            })
            .unwrap();
        warm.ingest(&emit);
        assert_eq!(seen.lock().unwrap().len(), 4);
    }

    #[test]
    fn cursor_runs_textual_capsules_and_multiple_capsules_resolve_deterministically() {
        let root = crate::skillcache::test_support::scratch();
        let workspace = root.path().join("project");
        let skill_dir = workspace.join(".agents/skills/pdf");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let skill_file = skill_dir.join("SKILL.md");
        std::fs::write(&skill_file, "# PDF\nExtract invoice totals.\n").unwrap();
        let tree = infra_skillcache::fingerprint::tree_sha256(&skill_dir).unwrap();
        let runtime = runtime(root.path());
        let env_fp = infra_skillcache::fingerprint::env_fingerprint(
            &infra_skillcache::fingerprint::runtime_versions(&[]),
        );
        let project_id = runtime.store.register(&workspace).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let db = runtime.store.connection().unwrap();
        let skill_ref = json!({"name": "pdf", "path": ".agents/skills/pdf/SKILL.md", "tree_sha256": tree.sha256});
        let skill_files_json = json!({"skills": [{"name": "pdf", "path": ".agents/skills/pdf/SKILL.md", "tree_sha256": tree.sha256, "files": tree.files}]});

        // One textual capsule and one hybrid capsule for the same skill.
        let textual_dir = root.path().join("textual/warm");
        let textual_manifest = json!({
            "schema_version": 1, "form": "textual",
            "mu": {"name": "pdf-textual", "description": "Markdown guide for pdf invoices"},
            "source": {"skills": [skill_ref.clone()]}
        });
        let textual_pkg =
            SkillCapsule::parse(&serde_json::to_vec(&textual_manifest).unwrap()).unwrap();
        let textual_sha = package::write_warm(
            &textual_dir,
            &textual_pkg,
            Some(b"# PDF\n\n1. Read invoices.\n"),
            &package::Files::new(),
        )
        .unwrap();
        let textual_loaded = package::load(&textual_dir).unwrap();
        db::begin_capsule(
            &db,
            "cap-text",
            "comp-1",
            &project_id,
            "s1",
            "g1",
            &skill_ref,
            None,
        )
        .unwrap();
        db::activate_capsule(
            &db,
            &db::Qualified {
                capsule_id: "cap-text",
                mu_name: "pdf-textual",
                mu_description: "Markdown guide for pdf invoices",
                form: "textual",
                capsule_sha256: &textual_loaded.capsule_sha256,
                package_sha256: &textual_sha,
                skill_files: &skill_files_json,
                env_fingerprint: &env_fp,
                engine_digest: "",
                artifact_dir: textual_dir.parent().unwrap().to_str().unwrap(),
                capabilities: &json!({"network": "none", "runtimes": []}),
            },
        )
        .unwrap();

        let hybrid_dir = root.path().join("hybrid/warm");
        let hybrid_manifest = json!({
            "schema_version": 1, "form": "hybrid",
            "mu": {"name": "pdf-hybrid", "description": "Automated table parser"},
            "source": {"skills": [skill_ref.clone()]},
            "capabilities": {"network": "none", "runtimes": []},
            "bindings": [],
            "programs": [],
            "steps": [{"kind": "instruction", "text": "Read invoices.", "source": {"path": ".agents/skills/pdf/SKILL.md"}, "produces": []}],
            "verification": []
        });
        let hybrid_pkg =
            SkillCapsule::parse(&serde_json::to_vec(&hybrid_manifest).unwrap()).unwrap();
        let hybrid_sha =
            package::write_warm(&hybrid_dir, &hybrid_pkg, None, &package::Files::new()).unwrap();
        let hybrid_loaded = package::load(&hybrid_dir).unwrap();
        db::begin_capsule(
            &db,
            "cap-hybrid",
            "comp-2",
            &project_id,
            "s1",
            "g2",
            &skill_ref,
            None,
        )
        .unwrap();
        db::activate_capsule(
            &db,
            &db::Qualified {
                capsule_id: "cap-hybrid",
                mu_name: "pdf-hybrid",
                mu_description: "Automated table parser",
                form: "hybrid",
                capsule_sha256: &hybrid_loaded.capsule_sha256,
                package_sha256: &hybrid_sha,
                skill_files: &skill_files_json,
                env_fingerprint: &env_fp,
                engine_digest: "",
                artifact_dir: hybrid_dir.parent().unwrap().to_str().unwrap(),
                capabilities: &json!({"network": "none", "runtimes": []}),
            },
        )
        .unwrap();
        drop(db);

        let route = ResolvedCommand {
            mode: "text".into(),
            text: "do pdf".into(),
            requests: vec![json!({"name": "pdf", "path": skill_file})],
            notice: None,
        };
        let session = |provider: &str| {
            json!({
                "id": "s-warm", "project_id": project_id, "provider": provider, "mode": "edit",
                "workflow": "native", "permissions": "auto",
                "skillcache": settings::validate(&json!({"mode": "warm_only"})).unwrap(),
            })
        };
        // Cursor receives Form::Textual directly instead of missing as unsupported.
        let cursor = decide_for(
            &runtime,
            &session("cursor"),
            &route,
            "do pdf",
            &workspace,
            None,
        );
        match cursor {
            Decision::Hit(warm) => {
                assert_eq!(warm.capsule_id, "cap-text");
                assert_eq!(warm.form, Form::Textual);
                assert!(warm.bin.is_none());
            }
            other => panic!("expected cursor textual hit, got {:?}", reason(&other)),
        }
        // Multiple active capsules for Claude never collapse to miss("ambiguous").
        let claude = decide_for(
            &runtime,
            &session("claude"),
            &route,
            "run automated steps",
            &workspace,
            None,
        );
        match claude {
            Decision::Hit(warm) => assert_eq!(warm.capsule_id, "cap-hybrid"),
            other => panic!("expected deterministic hit, got {:?}", reason(&other)),
        }

        // Removing the textual capsule leaves only cap-hybrid, so Cursor reports "provider".
        let db = runtime.store.connection().unwrap();
        db::set_capsule_status(&db, "cap-text", "retired", None).unwrap();
        drop(db);
        let hybrid_only = decide_for(
            &runtime,
            &session("cursor"),
            &route,
            "do pdf",
            &workspace,
            None,
        );
        assert_eq!(reason(&hybrid_only), "provider");
    }

    #[test]
    fn ingest_with_changes_forwards_runlog_file_changes() {
        let root = crate::skillcache::test_support::scratch();
        let state_dir = root.path().join("state");
        std::fs::create_dir_all(&state_dir).unwrap();
        let state_file = state_dir.join("plan-state.json");
        let mut log = state::RunLog::open(&state_file, "g1").unwrap();
        log.append(
            "program_completed",
            json!({"step_index": 0, "program": "extract"}),
        )
        .unwrap();
        state::append_changes(
            &state::log_path(&state_file),
            "g1",
            &[json!({"path": "out/rows.json", "kind": "create"})],
        )
        .unwrap();
        let warm = Warm {
            capsule_id: "c1".into(),
            form: Form::Hybrid,
            eval_arm: None,
            name: "pdf".into(),
            block: String::new(),
            environment: BTreeMap::new(),
            bin: Some(root.path().join("bin")),
            package: None,
            state_dir: Some(state_dir),
            seen: AtomicU64::new(0),
            seen_changes: AtomicU64::new(0),
        };
        let changes = std::sync::Mutex::new(Vec::<Value>::new());
        warm.ingest_with_changes(&|_| Ok(()), &|ev| changes.lock().unwrap().push(ev));
        warm.ingest_with_changes(&|_| Ok(()), &|ev| changes.lock().unwrap().push(ev));
        let captured = changes.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0]["tool"], "file_change");
        assert_eq!(captured[0]["changes"][0]["path"], "out/rows.json");
    }

    #[cfg(unix)]
    #[test]
    fn stage_tools_uses_symlink_instead_of_copying_engine() {
        let scratch = crate::skillcache::test_support::scratch();
        let engine = scratch.path().join("fake-engine");
        std::fs::write(&engine, b"#!/bin/sh\necho ok\n").unwrap();
        let mut rt = runtime(scratch.path());
        rt.engine = Some(engine.clone());
        let bin = stage_tools(&rt).unwrap();
        let run_meta = std::fs::symlink_metadata(bin.join("skillcache-run")).unwrap();
        let tool_meta = std::fs::symlink_metadata(bin.join("skillcache-tool")).unwrap();
        assert!(run_meta.file_type().is_symlink());
        assert!(tool_meta.file_type().is_symlink());
        assert_eq!(
            std::fs::read_link(bin.join("skillcache-run")).unwrap(),
            engine
        );
    }

    #[test]
    fn health_rate_and_plan_mode_eligibility_behave_cleanly() {
        let root = crate::skillcache::test_support::scratch();
        let workspace = root.path().join("project");
        let skill_dir = workspace.join(".agents/skills/pdf");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "# PDF\n").unwrap();
        let tree = infra_skillcache::fingerprint::tree_sha256(&skill_dir).unwrap();
        let runtime = runtime(root.path());
        let env_fp = infra_skillcache::fingerprint::env_fingerprint(
            &infra_skillcache::fingerprint::runtime_versions(&[]),
        );
        let project_id = runtime.store.register(&workspace).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let db = runtime.store.connection().unwrap();
        let skill_ref = json!({"name": "pdf", "path": ".agents/skills/pdf/SKILL.md", "tree_sha256": tree.sha256});
        let skill_files_json = json!({"skills": [{"name": "pdf", "path": ".agents/skills/pdf/SKILL.md", "tree_sha256": tree.sha256, "files": tree.files}]});

        let ro_dir = root.path().join("ro/warm");
        let ro_manifest = json!({
            "schema_version": 1, "form": "hybrid",
            "mu": {"name": "pdf-ro", "description": "Read-only inspector"},
            "source": {"skills": [skill_ref.clone()]},
            "capabilities": {"network": "none", "runtimes": []},
            "bindings": [],
            "programs": [{"id": "inspect", "entry": ["python3", "programs/inspect/main.py"], "inputs": [], "outputs": [], "check": ["python3", "programs/inspect/main.py", "--check"], "timeout_seconds": 30, "read_only": true}],
            "steps": [{"kind": "program", "program": "inspect"}],
            "verification": []
        });
        let ro_pkg = SkillCapsule::parse(&serde_json::to_vec(&ro_manifest).unwrap()).unwrap();
        let mut ro_files = package::Files::new();
        ro_files.insert("programs/inspect/main.py", b"print('{}')\n".to_vec());
        let ro_sha = package::write_warm(&ro_dir, &ro_pkg, None, &ro_files).unwrap();
        let ro_loaded = package::load(&ro_dir).unwrap();
        db::begin_capsule(
            &db,
            "cap-ro",
            "comp-ro",
            &project_id,
            "s1",
            "g1",
            &skill_ref,
            None,
        )
        .unwrap();
        db::activate_capsule(
            &db,
            &db::Qualified {
                capsule_id: "cap-ro",
                mu_name: "pdf-ro",
                mu_description: "Read-only inspector",
                form: "hybrid",
                capsule_sha256: &ro_loaded.capsule_sha256,
                package_sha256: &ro_sha,
                skill_files: &skill_files_json,
                env_fingerprint: &env_fp,
                engine_digest: "",
                artifact_dir: ro_dir.parent().unwrap().to_str().unwrap(),
                capabilities: &json!({"network": "none", "runtimes": []}),
            },
        )
        .unwrap();

        let rw_dir = root.path().join("rw/warm");
        let rw_manifest = json!({
            "schema_version": 1, "form": "hybrid",
            "mu": {"name": "pdf-rw", "description": "Writer"},
            "source": {"skills": [skill_ref.clone()]},
            "capabilities": {"network": "none", "runtimes": []},
            "bindings": [],
            "programs": [{"id": "write", "entry": ["python3", "programs/write/main.py"], "inputs": [], "outputs": [], "check": ["python3", "programs/write/main.py", "--check"], "timeout_seconds": 30, "read_only": false}],
            "steps": [{"kind": "program", "program": "write"}],
            "verification": []
        });
        let rw_pkg = SkillCapsule::parse(&serde_json::to_vec(&rw_manifest).unwrap()).unwrap();
        let mut rw_files = package::Files::new();
        rw_files.insert("programs/write/main.py", b"print('{}')\n".to_vec());
        let rw_sha = package::write_warm(&rw_dir, &rw_pkg, None, &rw_files).unwrap();
        let rw_loaded = package::load(&rw_dir).unwrap();
        db::begin_capsule(
            &db,
            "cap-rw",
            "comp-rw",
            &project_id,
            "s1",
            "g2",
            &skill_ref,
            None,
        )
        .unwrap();
        db::activate_capsule(
            &db,
            &db::Qualified {
                capsule_id: "cap-rw",
                mu_name: "pdf-rw",
                mu_description: "Writer",
                form: "hybrid",
                capsule_sha256: &rw_loaded.capsule_sha256,
                package_sha256: &rw_sha,
                skill_files: &skill_files_json,
                env_fingerprint: &env_fp,
                engine_digest: "",
                artifact_dir: rw_dir.parent().unwrap().to_str().unwrap(),
                capabilities: &json!({"network": "none", "runtimes": []}),
            },
        )
        .unwrap();
        drop(db);

        let route = ResolvedCommand {
            mode: "text".into(),
            text: "do pdf".into(),
            requests: vec![json!({"name": "pdf", "path": ".agents/skills/pdf/SKILL.md"})],
            notice: None,
        };
        let plan_session = json!({
            "id": "s-plan", "project_id": project_id, "provider": "claude", "mode": "plan",
            "workflow": "native", "permissions": "auto",
            "skillcache": settings::validate(&json!({"mode": "warm_only"})).unwrap(),
        });
        match decide_for(&runtime, &plan_session, &route, "do pdf", &workspace, None) {
            Decision::Hit(warm) => assert_eq!(warm.capsule_id, "cap-ro"),
            other => panic!(
                "expected read-only hit in plan mode, got {:?}",
                reason(&other)
            ),
        }

        {
            let db = runtime.store.connection().unwrap();
            db::set_capsule_status(&db, "cap-ro", "retired", None).unwrap();
        }
        assert_eq!(
            reason(&decide_for(
                &runtime,
                &plan_session,
                &route,
                "do pdf",
                &workspace,
                None
            )),
            "plan_mode"
        );

        {
            let db = runtime.store.connection().unwrap();
            db::set_capsule_status(&db, "cap-ro", "active", None).unwrap();
            for i in 0..19 {
                db::record_run(
                    &db,
                    &db::Run {
                        capsule_id: "cap-ro",
                        session_id: "s1",
                        generation: &format!("ok-{i}"),
                        provider: "claude",
                        model: None,
                        outcome: "complete",
                        deopts: 0,
                        steps: &json!([]),
                    },
                )
                .unwrap();
            }
            db::record_run(
                &db,
                &db::Run {
                    capsule_id: "cap-ro",
                    session_id: "s1",
                    generation: "bad",
                    provider: "claude",
                    model: None,
                    outcome: "unfinished",
                    deopts: 0,
                    steps: &json!([]),
                },
            )
            .unwrap();
            db::record_run(
                &db,
                &db::Run {
                    capsule_id: "cap-rw",
                    session_id: "s1",
                    generation: "bad2",
                    provider: "claude",
                    model: None,
                    outcome: "not_started",
                    deopts: 0,
                    steps: &json!([]),
                },
            )
            .unwrap();
        }
        let best = best_by_health_and_recency(
            &runtime.store,
            &[
                json!({"capsule_id": "cap-rw", "form": "hybrid", "updated_at": "2026-12-01T00:00:00Z"}),
                json!({"capsule_id": "cap-ro", "form": "hybrid", "updated_at": "2026-01-01T00:00:00Z"}),
            ],
        );
        assert_eq!(best["capsule_id"], "cap-ro");
    }

    #[test]
    fn tool_chain_capsule_hits_across_worktrees_on_same_tree_sha() {
        let root = crate::skillcache::test_support::scratch();
        let wt1 = root.path().join("wt1");
        let wt2 = root.path().join("wt2");
        for wt in [&wt1, &wt2] {
            std::fs::create_dir_all(wt).unwrap();
            std::fs::write(wt.join("app.php"), "<?php\n").unwrap();
        }
        let store = SessionStore::open(&root.path().join("state")).unwrap();
        let project = store.register(&wt1).unwrap();
        let project_id = project["id"].as_str().unwrap().to_owned();
        let tree_sha = trace::workspace_tree_sha(&wt1).unwrap();
        assert_eq!(tree_sha, trace::workspace_tree_sha(&wt2).unwrap());

        let warm_dir = root.path().join("artifacts/tc-1/warm");
        let capsule_val = json!({
            "schema_version": 1,
            "form": "textual",
            "kind": "tool_chain",
            "mu": {"name": "inspect-php", "description": "Inspect PHP entrypoints safely"},
            "source": {"skills": [], "tool_chain": {"tree_sha256": tree_sha}},
        });
        let cap = SkillCapsule::from_value(&capsule_val).unwrap();
        let pkg_sha = infra_skillcache::package::write_warm(
            &warm_dir,
            &cap,
            Some(b"# Inspect PHP\n1. Read `app.php`.\n"),
            &infra_skillcache::package::Files::default(),
        )
        .unwrap();
        let env_fp = infra_skillcache::fingerprint::env_fingerprint(
            &infra_skillcache::fingerprint::runtime_versions(&[]),
        );
        let sess = store
            .create(
                &json!({"project_id": project_id, "provider": "claude", "mode": "edit", "workflow": "native", "permissions": "auto"}),
                "inspect php",
                &[],
            )
            .unwrap();
        let db = store.connection().unwrap();
        let sid = sess["id"].as_str().unwrap().to_owned();
        db::begin_capsule(
            &db,
            "cap-tc-1",
            "cmp-tc-1",
            &project_id,
            &sid,
            "g-tc",
            &json!({"name": "tool-chain", "path": "", "tree_sha256": tree_sha}),
            None,
        )
        .unwrap();
        db::activate_capsule(
            &db,
            &db::Qualified {
                capsule_id: "cap-tc-1",
                mu_name: "inspect-php",
                mu_description: "Inspect PHP entrypoints safely",
                form: "textual",
                capsule_sha256: "a",
                package_sha256: &pkg_sha,
                skill_files: &json!({}),
                env_fingerprint: &env_fp,
                engine_digest: "eng",
                artifact_dir: root.path().join("artifacts/tc-1").to_str().unwrap(),
                capabilities: &json!({"network": "none", "runtimes": []}),
            },
        )
        .unwrap();
        db::set_capsule_kind_meta(&db, "cap-tc-1", "tool_chain", &tree_sha).unwrap();
        drop(db);

        let rt = Runtime {
            store,
            providers: Arc::new(crate::provider::ProviderDomain {
                providers: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
            }),
            engine: None,
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            enqueuer: None,
            requalifying: None,
        };
        let route = ResolvedCommand {
            mode: "text".into(),
            text: "inspect php entrypoints".into(),
            requests: vec![],
            notice: None,
        };
        let mut s_off = sess.clone();
        s_off["skillcache"] = json!({"mode": "auto"});
        assert_eq!(
            reason(&decide_for(
                &rt,
                &s_off,
                &route,
                "inspect php entrypoints",
                &wt2,
                None
            )),
            "no_single_skill"
        );
        let mut s_on = sess;
        s_on["skillcache"] = json!({"mode": "auto", "tool_chain": true});
        match decide_for(&rt, &s_on, &route, "inspect php entrypoints", &wt2, None) {
            Decision::Hit(warm) => {
                assert_eq!(warm.capsule_id, "cap-tc-1");
                assert!(warm.block.contains("workspace tool-chain"));
            }
            other => panic!(
                "expected tool-chain hit on second worktree, got {}",
                reason(&other)
            ),
        }
    }

    #[test]
    fn mirror_skills_deduplicate_capsules_by_canonical_name_and_digest() {
        let root = crate::skillcache::test_support::scratch();
        let wt = root.path().join("wt");
        let skill_md =
            "---\nname: pdf\ndescription: Extract PDF rows\n---\n# PDF\nRead invoices.\n";
        for dir in [
            ".claude/skills/pdf",
            ".agents/skills/pdf",
            ".cursor/skills/pdf",
        ] {
            std::fs::create_dir_all(wt.join(dir)).unwrap();
            std::fs::write(wt.join(dir).join("SKILL.md"), skill_md).unwrap();
        }
        let store = SessionStore::open(&root.path().join("state")).unwrap();
        let project_id = store.register(&wt).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let route_claude = ResolvedCommand {
            mode: "text".into(),
            text: "pdf".into(),
            requests: vec![json!({"name": "pdf", "path": ".claude/skills/pdf/SKILL.md"})],
            notice: None,
        };
        let resolved = trace::resolve_skills(
            &wt,
            &route_claude,
            "$pdf extract",
            &infra_skillcache::trace::Trace {
                provider: "codex".into(),
                steps: Vec::new(),
                skills: Vec::new(),
                counters: Default::default(),
                complete: true,
            },
        );
        let tree_sha = resolved[0]["tree_sha256"].as_str().unwrap().to_owned();

        let warm_root = root.path().join("artifacts/mirror-1");
        let warm_dir = warm_root.join("warm");
        let capsule_val = json!({
            "schema_version": 1,
            "form": "textual",
            "mu": {"name": "pdf-mirror", "description": "Extract PDF rows"},
            "source": {"skills": [{"name": "pdf", "path": ".claude/skills/pdf/SKILL.md", "tree_sha256": tree_sha}]},
        });
        let cap = SkillCapsule::from_value(&capsule_val).unwrap();
        let pkg_sha = infra_skillcache::package::write_warm(
            &warm_dir,
            &cap,
            Some(b"# PDF\nUse cached workflow.\n"),
            &infra_skillcache::package::Files::new(),
        )
        .unwrap();
        let loaded = infra_skillcache::package::load(&warm_dir).unwrap();
        let env_fp = infra_skillcache::fingerprint::env_fingerprint(
            &crate::skillcache::runtime_versions(&[]),
        );
        {
            let db = store.connection().unwrap();
            let skill_ref = json!({
                "name": "pdf",
                "path": ".claude/skills/pdf/SKILL.md",
                "tree_sha256": tree_sha
            });
            db::begin_capsule(
                &db,
                "cap-mirror",
                "cmp-mirror",
                &project_id,
                "s1",
                "g1",
                &skill_ref,
                None,
            )
            .unwrap();
            db::activate_capsule(
                &db,
                &db::Qualified {
                    capsule_id: "cap-mirror",
                    mu_name: "pdf-mirror",
                    mu_description: "Extract PDF rows",
                    form: "textual",
                    capsule_sha256: &loaded.capsule_sha256,
                    package_sha256: &pkg_sha,
                    skill_files: &json!({}),
                    env_fingerprint: &env_fp,
                    engine_digest: "eng",
                    artifact_dir: warm_root.to_str().unwrap(),
                    capabilities: &json!({}),
                },
            )
            .unwrap();
            for gate in ["gate1", "gate2", "gate3"] {
                db::record_gate(&db, "cap-mirror", 1, gate, true, &json!({"passed": true}))
                    .unwrap();
            }
        }
        let rt = Runtime {
            store,
            providers: Arc::new(crate::provider::ProviderDomain {
                providers: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
            }),
            engine: None,
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            enqueuer: None,
            requalifying: None,
        };
        let sess = json!({
            "id": "s-mirror",
            "project_id": project_id,
            "provider": "codex",
            "mode": "edit",
            "workflow": "native",
            "permissions": "auto",
            "skillcache": settings::validate(&json!({"mode": "warm_only"})).unwrap(),
        });
        for mirror_path in [".agents/skills/pdf/SKILL.md", ".cursor/skills/pdf/SKILL.md"] {
            let route = ResolvedCommand {
                mode: "text".into(),
                text: "pdf".into(),
                requests: vec![json!({"name": "pdf", "path": mirror_path})],
                notice: None,
            };
            match decide_for(&rt, &sess, &route, "$pdf extract", &wt, None) {
                Decision::Hit(warm) => assert_eq!(warm.capsule_id, "cap-mirror"),
                other => panic!(
                    "expected mirror hit for {mirror_path}, got {}",
                    reason(&other)
                ),
            }
        }

        // Changing a mirror's content digest must prevent stale reuse.
        std::fs::write(
            wt.join(".agents/skills/pdf/SKILL.md"),
            "---\nname: pdf\n---\nChanged instructions\n",
        )
        .unwrap();
        let route_drifted = ResolvedCommand {
            mode: "text".into(),
            text: "pdf".into(),
            requests: vec![json!({"name": "pdf", "path": ".agents/skills/pdf/SKILL.md"})],
            notice: None,
        };
        assert_eq!(
            reason(&decide_for(
                &rt,
                &sess,
                &route_drifted,
                "$pdf extract",
                &wt,
                None
            )),
            "no_capsule"
        );
    }

    #[test]
    fn toolchain_capsule_survives_unrelated_git_commit_and_invalidates_on_dirty_touched_file() {
        let root = crate::skillcache::test_support::scratch();
        let wt = root.path().join("repo");
        std::fs::create_dir_all(wt.join("src")).unwrap();
        std::fs::write(wt.join("composer.json"), r#"{"name":"acme/app"}"#).unwrap();
        std::fs::write(wt.join("src/App.php"), "<?php echo 1;\n").unwrap();
        std::fs::write(wt.join("README.md"), "Docs\n").unwrap();

        let git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(&wt)
                .args([
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.invalid",
                ])
                .args(args)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?}");
        };
        git(&["init"]);
        git(&["add", "."]);
        git(&["commit", "-m", "init"]);

        let store = SessionStore::open(&root.path().join("state")).unwrap();
        let project_id = store.register(&wt).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let initial_tree = trace::workspace_tree_sha(&wt).unwrap();
        let snapshot = trace::toolchain_manifest_snapshot(
            &wt,
            &[json!({"kind": "tool", "tool": "Read", "input": {"file_path": "src/App.php"}})],
        );
        let warm_root = root.path().join("artifacts/tc-manifest");
        let warm_dir = warm_root.join("warm");
        let capsule_val = json!({
            "schema_version": 1,
            "form": "textual",
            "kind": "tool_chain",
            "mu": {"name": "inspect-app", "description": "Inspect PHP app"},
            "source": {"skills": [], "tool_chain": {"tree_sha256": initial_tree}},
        });
        let cap = SkillCapsule::from_value(&capsule_val).unwrap();
        let pkg_sha = infra_skillcache::package::write_warm(
            &warm_dir,
            &cap,
            Some(b"# Inspect\nInspect `src/App.php`.\n"),
            &infra_skillcache::package::Files::new(),
        )
        .unwrap();
        let loaded = infra_skillcache::package::load(&warm_dir).unwrap();
        let env_fp = infra_skillcache::fingerprint::env_fingerprint(
            &crate::skillcache::runtime_versions(&[]),
        );
        {
            let db = store.connection().unwrap();
            let tc_ref =
                json!({"name": "tool-chain", "path": ".tool_chain", "tree_sha256": initial_tree});
            db::begin_capsule(
                &db,
                "cap-tc-m",
                "cmp-tc-m",
                &project_id,
                "s1",
                "g1",
                &tc_ref,
                None,
            )
            .unwrap();
            db::activate_capsule(
                &db,
                &db::Qualified {
                    capsule_id: "cap-tc-m",
                    mu_name: "inspect-app",
                    mu_description: "Inspect PHP app",
                    form: "textual",
                    capsule_sha256: &loaded.capsule_sha256,
                    package_sha256: &pkg_sha,
                    skill_files: &json!({}),
                    env_fingerprint: &env_fp,
                    engine_digest: "eng",
                    artifact_dir: warm_root.to_str().unwrap(),
                    capabilities: &json!({}),
                },
            )
            .unwrap();
            db::set_capsule_kind_meta(&db, "cap-tc-m", "tool_chain", &initial_tree).unwrap();
            db::set_dependency_manifest(
                &db,
                "cap-tc-m",
                &db::ArtifactDependencyManifest {
                    skill_paths: Vec::new(),
                    watched_globs: Vec::new(),
                    manifest_files: vec!["composer.json".into()],
                    ast_selectors: Vec::new(),
                    route_selectors: Vec::new(),
                    table_selectors: Vec::new(),
                    caller_files: vec!["src/App.php".into()],
                    path_hashes: snapshot,
                    ir_revision: 1,
                    verified_ns: 1,
                },
            )
            .unwrap();
            for gate in ["gate1", "gate2", "gate3"] {
                db::record_gate(&db, "cap-tc-m", 1, gate, true, &json!({"passed": true})).unwrap();
            }
        }
        let rt = Runtime {
            store,
            providers: Arc::new(crate::provider::ProviderDomain {
                providers: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
            }),
            engine: None,
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            enqueuer: None,
            requalifying: None,
        };
        let route = ResolvedCommand {
            mode: "text".into(),
            text: "inspect php app".into(),
            requests: vec![],
            notice: None,
        };
        let sess = json!({
            "id": "s-tc-m",
            "project_id": project_id,
            "provider": "codex",
            "mode": "edit",
            "workflow": "native",
            "permissions": "auto",
            "skillcache": {"mode": "auto", "tool_chain": true},
        });

        // Commit an unrelated docs change: HEAD^{tree} changed, yet capsule still hits via path_hashes!
        std::fs::write(wt.join("README.md"), "Changed unrelated docs\n").unwrap();
        git(&["add", "README.md"]);
        git(&["commit", "-m", "docs"]);
        assert_ne!(trace::workspace_tree_sha(&wt).unwrap(), initial_tree);
        match decide_for(&rt, &sess, &route, "inspect php app", &wt, None) {
            Decision::Hit(warm) => assert_eq!(warm.capsule_id, "cap-tc-m"),
            other => panic!(
                "expected hit despite unrelated commit, got {}",
                reason(&other)
            ),
        }

        // Dirtying a touched file in the working tree invalidates the tool_chain capsule.
        std::fs::write(wt.join("src/App.php"), "<?php echo 999;\n").unwrap();
        assert_eq!(
            reason(&decide_for(
                &rt,
                &sess,
                &route,
                "inspect php app",
                &wt,
                None
            )),
            "stale"
        );
    }
}
