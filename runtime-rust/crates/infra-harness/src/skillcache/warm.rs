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
use infra_skillcache::{Form, Step, capsule::Network, env_filter, package, state};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

/// The experiment switch for choosing among several capsules of one skill by
/// their descriptions. Off until it has its own measurement.
pub const RETRIEVAL_BY_MU: &str = "skillcache.retrieval_by_mu";

/// A turn that runs a capsule.
pub struct Warm {
    pub capsule_id: String,
    pub form: Form,
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
            Self::Hit(warm) => Some(json!({
                "kind": "skillcache",
                "phase": "hit",
                "capsule_id": warm.capsule_id,
                "form": warm.form.as_str(),
                "mu": warm.name,
                "text": format!(
                    "SkillCache: capsule `{}` runs this request instead of the skill being read again.",
                    warm.name
                ),
            })),
        }
    }
    /// What the launch record keeps of the decision.
    pub fn record(&self) -> Option<Value> {
        match self {
            Self::Off => None,
            Self::Miss { reason, .. } => Some(json!({"decision": "miss", "reason": reason})),
            Self::Hit(warm) => Some(json!({
                "decision": "hit",
                "capsule_id": warm.capsule_id,
                "form": warm.form.as_str(),
            })),
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
    let root = runtime.store.state.join("skillcache/bin");
    let bin = root.join(&digest[..16]);
    infra_core::fs::ensure_directory(&bin, true)?;
    let name = |tool: &str| bin.join(format!("{tool}{}", std::env::consts::EXE_SUFFIX));
    let first = name("skillcache-run");
    if !first.is_file() {
        let staging = bin.join(format!(".staging-{}", crate::store::hexid()));
        std::fs::copy(&engine, &staging)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o500))?;
        }
        std::fs::rename(&staging, &first)?;
    }
    let second = name("skillcache-tool");
    if !second.is_file() && std::fs::hard_link(&first, &second).is_err() {
        std::fs::copy(&first, &second)?;
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
         - Start with `skillcache-run run '{{\"name\": value, ...}}'`: one JSON object, as one shell argument, with the values above. If quoting is awkward, run `skillcache-run run -` and write the JSON to standard input.\n\
         - Every answer is one JSON object. Do what its `status` says:\n\
         \x20 - `needs_bindings`: supply the named values with `skillcache-run continue '{{...}}'`.\n\
         \x20 - `your_steps`: do those steps yourself with your normal tools, then `skillcache-run continue '{{...}}'` with what they produce.\n\
         \x20 - `program_failed`: do that one step yourself with your normal tools, following the source skill at `{skill}`. Do not repair or re-run the program. Then `skillcache-run continue '{{...}}'` with its `required_outputs`. The rest of the plan stays compiled.\n\
         \x20 - `complete`: the plan is done; `result` holds what the programs found. Answer from it.\n\
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
        Decision::Hit(warm) => json!({
            "mode": mode,
            "decision": "hit",
            "capsule": {"capsule_id": warm.capsule_id, "mu": warm.name, "form": warm.form.as_str()},
        }),
    }
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
    if session["mode"] != "edit" {
        return miss(
            "plan_mode",
            "a capsule's programs write, and this is a read-only turn",
        );
    }
    if !["claude", "codex"].contains(&provider) {
        return miss(
            "provider",
            "warm turns are verified for Claude and Codex only",
        );
    }
    if provider == "codex" && manual {
        return miss(
            "codex_manual",
            "Codex in Manual mode would ask before every step of the executor",
        );
    }
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
    let project_id = session["project_id"].as_str().unwrap_or("");
    let candidates = match runtime
        .store
        .connection()
        .and_then(|db| db::active_capsules(&db, project_id, &skill_path))
    {
        Ok(candidates) => candidates,
        Err(error) => return miss("unavailable", error.to_string()),
    };
    let chosen = match candidates.len() {
        0 => return miss("no_capsule", "no qualified capsule exists for this skill"),
        1 => candidates[0].clone(),
        _ => {
            let by_description = runtime
                .store
                .setting(RETRIEVAL_BY_MU)
                .ok()
                .flatten()
                .is_some_and(|value| value == true);
            let ids = candidates
                .iter()
                .filter_map(|capsule| capsule["capsule_id"].as_str().map(String::from))
                .collect::<Vec<_>>();
            let best = by_description
                .then(|| best_by_description(&runtime.store, project_id, &ids, message))
                .flatten();
            match best.and_then(|id| {
                candidates
                    .iter()
                    .find(|capsule| capsule["capsule_id"] == id.as_str())
            }) {
                Some(capsule) => capsule.clone(),
                None => {
                    return miss(
                        "ambiguous",
                        "this skill has several capsules and choosing among them is not enabled",
                    );
                }
            }
        }
    };
    let capsule_id = chosen["capsule_id"].as_str().unwrap_or("").to_string();
    if let Some(reason) = staleness(runtime, &chosen, workspace) {
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
    let text = block(&stored, &skill_path);
    let (true, Some(generation)) = (stored.capsule.form == Form::Hybrid, generation) else {
        // A written procedure is read, and a preview stages nothing.
        return Decision::Hit(Arc::new(Warm {
            capsule_id,
            form: stored.capsule.form,
            name,
            block: text,
            environment: BTreeMap::new(),
            bin: None,
            package: None,
            state_dir: None,
            seen: AtomicU64::new(0),
        }));
    };
    let staged = (|| -> Result<(PathBuf, PathBuf)> {
        let bin = stage_tools(runtime)?;
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
    let environment = BTreeMap::from([
        (env_filter::PACKAGE.to_string(), text_of(&warm_dir)),
        (env_filter::WORKSPACE.to_string(), text_of(workspace)),
        (
            env_filter::STATE_FILE.to_string(),
            text_of(&state_dir.join("plan-state.json")),
        ),
        (env_filter::GENERATION.to_string(), generation.to_string()),
    ]);
    Decision::Hit(Arc::new(Warm {
        capsule_id,
        form: Form::Hybrid,
        name,
        block: text,
        environment,
        bin: Some(bin),
        package: Some(warm_dir),
        state_dir: Some(state_dir),
        seen: AtomicU64::new(0),
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
        let rest = command
            .environment
            .get("PATH")
            .cloned()
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
            "claude" => allow_claude(&mut command)?,
            // Codex's sandbox writes only inside its workspace and the
            // folders it is given; the executor's state file is in neither.
            // Without the sandbox there is nothing to widen.
            "codex"
                if !command.argv.iter().any(|argument| {
                    argument == "app-server"
                        || argument == "--dangerously-bypass-approvals-and-sandbox"
                }) =>
            {
                let at = command.argv.len().saturating_sub(1);
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

/// Lets Claude Code run the two commands of the protocol without asking.
fn allow_claude(command: &mut PreparedCommand) -> Result<()> {
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
    for rule in ["Bash(skillcache-run:*)", "Bash(skillcache-tool:*)"] {
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
    fn state_file(&self) -> Option<PathBuf> {
        self.state_dir
            .as_ref()
            .map(|dir| dir.join("plan-state.json"))
    }
    /// Turns the executor's new journal lines into session events. A line is
    /// published once: the journal numbers them and the last one seen is kept.
    pub fn ingest(&self, emit: &dyn Fn(Value) -> Result<()>) {
        let Some(state_file) = self.state_file() else {
            return;
        };
        let after = self.seen.load(Ordering::Acquire);
        let Ok(events) = state::read_log(&state_file, after) else {
            return;
        };
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
            .state_file()
            .and_then(|state_file| state::read_log(&state_file, 0).ok())
            .unwrap_or_default();
        let count = |name: &str| events.iter().filter(|event| event["event"] == name).count();
        let outcome = if self.form == Form::Textual {
            "read"
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
        if let Ok(db) = store.connection() {
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
                    deopts: count("program_failed") as i64,
                },
            );
            let _ = db::touch_capsule(&db, &self.capsule_id);
        }
        if let Some(state_dir) = &self.state_dir {
            let _ = std::fs::remove_dir_all(state_dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use infra_skillcache::SkillCapsule;

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
        let root = tempfile::tempdir().unwrap();
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
            "plan_mode"
        );
        assert_eq!(
            decide(&session("cursor", "edit", "auto", "warm_only"), &one),
            "provider"
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
        let root = tempfile::tempdir().unwrap();
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
        let root = tempfile::tempdir().unwrap();
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
        // A textual capsule stages nothing and changes nothing of the launch.
        let textual = WarmLaunchAdapter {
            inner: Arc::new(Plain),
            warm: Arc::new(Warm {
                capsule_id: "c2".into(),
                form: Form::Textual,
                name: "t".into(),
                block: String::new(),
                environment: BTreeMap::new(),
                bin: None,
                package: None,
                state_dir: None,
                seen: AtomicU64::new(0),
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
        let root = tempfile::tempdir().unwrap();
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
            name: "pdf-totals".into(),
            block: String::new(),
            environment: BTreeMap::new(),
            bin: Some(root.path().join("bin")),
            package: None,
            state_dir: Some(state_dir.clone()),
            seen: AtomicU64::new(0),
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
}
