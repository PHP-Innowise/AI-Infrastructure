//! Recording the observed trace of a cold turn.
//!
//! The run's one event slot belongs to the lifecycle binding; a [`Tee`] gives
//! the recorder the same raw provider events without changing what the
//! binding sees. Observing is cheap and never fails the run. When the turn
//! completes, the trace is redacted, bounded and stored; a turn that failed,
//! used no tool or no identifiable skill leaves nothing behind.
use crate::{
    Result, commands::ResolvedCommand, lifecycle::RunBinding, run_activity, skillcache::db,
    store::SessionStore,
};
use infra_host::{process::RunObserver, provider::RunEvents};
use infra_skillcache::{
    env_filter, fingerprint,
    trace::{Recorder, Redactor, SkillRef, Trace},
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// The shortest environment value worth replacing wherever it appears.
const SECRET_CHARS: usize = 8;
const SKILLS: usize = 8;

pub struct TraceRecorder {
    recorder: Mutex<Option<Recorder>>,
}
impl TraceRecorder {
    pub fn new(provider: &str) -> Arc<Self> {
        Arc::new(Self {
            recorder: Mutex::new(Some(Recorder::new(provider))),
        })
    }
    /// One event in the provider's own format.
    pub fn observe(&self, raw: &Value) {
        if let Ok(mut recorder) = self.recorder.lock()
            && let Some(recorder) = recorder.as_mut()
        {
            recorder.observe(raw);
        }
    }
    /// The recorded trace; a recorder gives it once.
    pub fn finish(&self, redactor: &dyn Redactor) -> Option<Trace> {
        let recorder = self.recorder.lock().ok()?.take()?;
        Some(recorder.finish(redactor))
    }
}

/// Feeds the recorder the raw events the run's binding observes.
pub struct Tee {
    pub primary: RunBinding,
    pub recorder: Arc<TraceRecorder>,
}
impl RunObserver for Tee {
    fn boundary(&self, signal: &str, details: &Value) -> infra_host::Result<()> {
        self.primary.boundary(signal, details)
    }
}
impl RunEvents for Tee {
    fn observe(&self, raw: Option<&Value>, normalized: Option<&Value>) -> infra_host::Result<()> {
        if let Some(raw) = raw {
            self.recorder.observe(raw);
        }
        self.primary.observe(raw, normalized)
    }
    fn enrich(&self, event: &Value) -> infra_host::Result<Value> {
        self.primary.enrich(event)
    }
}

/// What a session's trace must not keep: the values of the launch's
/// credential-like environment variables, command-line credentials, anything
/// the shared secret policy names, and the host's own paths.
pub struct SessionRedactor {
    secrets: Vec<String>,
    roots: Vec<String>,
    home: Option<String>,
}
impl SessionRedactor {
    /// `environment` is what the provider process was started with.
    pub fn new(workspace: &Path, environment: impl IntoIterator<Item = (String, String)>) -> Self {
        let mut secrets = environment
            .into_iter()
            .filter(|(name, value)| {
                let name = name.to_uppercase();
                value.chars().count() >= SECRET_CHARS
                    && (env_filter::withheld(&name)
                        || ["SECRET", "CREDENTIAL", "PASSWD", "AUTH"]
                            .iter()
                            .any(|part| name.contains(part)))
            })
            .map(|(_, value)| value)
            .collect::<Vec<_>>();
        // The longest first, so that a value containing another is replaced whole.
        secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
        secrets.dedup();
        Self {
            secrets,
            roots: vec![workspace.display().to_string()],
            home: run_activity::user_home(),
        }
    }
}
impl Redactor for SessionRedactor {
    fn redact(&self, text: &str) -> (String, usize) {
        let mut changed = 0;
        let mut clean = text.to_string();
        for secret in &self.secrets {
            let found = clean.matches(secret.as_str()).count();
            if found > 0 {
                changed += found;
                clean = clean.replace(secret.as_str(), "[redacted: environment]");
            }
        }
        let commands = run_activity::redact_command(&clean);
        if commands != clean {
            changed += 1;
            clean = commands;
        }
        match infra_core::domain::mask_secrets(&clean) {
            Ok((masked, count)) => {
                changed += count;
                clean = masked;
            }
            Err(_) => {
                return (
                    "[redacted: a possible secret could not be masked]".into(),
                    changed + 1,
                );
            }
        }
        // Host paths become workspace-relative; that is not counted as a
        // redaction, it is how the trace names files.
        (
            run_activity::strip_root(&clean, &self.roots, self.home.as_deref()),
            changed,
        )
    }
}

/// How a skill's entry document is named in a trace and a capsule: relative
/// to the workspace, or `~/...` for a skill of the user's home.
fn spelled(workspace: &Path, home: Option<&Path>, file: &Path) -> Option<String> {
    let text = |relative: &Path| {
        relative
            .components()
            .map(|part| part.as_os_str().to_str())
            .collect::<Option<Vec<_>>>()
            .map(|parts| parts.join("/"))
    };
    if let Ok(relative) = file.strip_prefix(workspace) {
        return text(relative);
    }
    let home = infra_core::fs::canonicalize(home?).ok()?;
    file.strip_prefix(&home)
        .ok()
        .and_then(text)
        .map(|relative| format!("~/{relative}"))
}

/// The skill whose entry document is `file`, when it is an ordinary skill
/// folder inside the workspace or the user's home.
fn skill_entry(
    workspace: &Path,
    home: Option<&Path>,
    file: &Path,
    name: Option<&str>,
) -> Option<Value> {
    let file = infra_core::fs::canonicalize(file).ok()?;
    if file.file_name()?.to_str()? != "SKILL.md" || !file.is_file() {
        return None;
    }
    let folder = file.parent()?;
    let path = spelled(workspace, home, &file)?;
    let tree = fingerprint::tree_sha256(folder).ok()?;
    let name = name
        .map(String::from)
        .or_else(|| folder.file_name()?.to_str().map(String::from))?;
    Some(json!({"name": name, "path": path, "tree_sha256": tree.sha256}))
}

/// Where a skill's files are, from how its path is spelled in a trace.
pub fn skill_file(workspace: &Path, path: &str) -> Option<PathBuf> {
    match path.strip_prefix("~/") {
        Some(relative) => Some(PathBuf::from(run_activity::user_home()?).join(relative)),
        None => Some(workspace.join(path)),
    }
}

/// The skills `S` a turn used: the ones the message routed to and the ones
/// the trace shows the agent loading. Each is named with the digest of its
/// folder at this moment, so a later compile can tell that it has changed.
pub fn resolve_skills(
    workspace: &Path,
    route: &ResolvedCommand,
    message: &str,
    trace: &Trace,
) -> Vec<Value> {
    let Ok(workspace) = infra_core::fs::canonicalize(workspace) else {
        return Vec::new();
    };
    let home = run_activity::user_home().map(PathBuf::from);
    let home = home.as_deref();
    let by_name = |name: &str| -> Option<Value> {
        // A plugin's skill (`plugin:skill`) has no folder this resolves.
        if name.is_empty() || name.contains([':', '/', '\\']) || name.starts_with('.') {
            return None;
        }
        [Some(workspace.as_path()), home]
            .into_iter()
            .flatten()
            .find_map(|root| {
                skill_entry(
                    &workspace,
                    home,
                    &root.join(".claude/skills").join(name).join("SKILL.md"),
                    Some(name),
                )
            })
    };
    let by_path = |path: &str, name: Option<&str>| -> Option<Value> {
        let file = match path.strip_prefix("~/") {
            Some(relative) => home?.join(relative),
            None if Path::new(path).is_absolute() => PathBuf::from(path),
            None => workspace.join(path),
        };
        skill_entry(&workspace, home, &file, name)
    };
    let mut skills: Vec<Value> = Vec::new();
    let mut add = |skill: Option<Value>| {
        if let Some(skill) = skill
            && skills.len() < SKILLS
            && !skills.iter().any(|known| known["path"] == skill["path"])
        {
            skills.push(skill);
        }
    };
    for request in &route.requests {
        add(request["path"]
            .as_str()
            .and_then(|path| by_path(path, request["name"].as_str())));
    }
    // A native command is sent as typed; its name is the skill it starts.
    if route.mode == "native"
        && let Some(name) = message
            .strip_prefix('/')
            .and_then(|tail| tail.split_whitespace().next())
    {
        add(by_name(name));
    }
    for SkillRef { name, path } in &trace.skills {
        add(match (name, path) {
            (_, Some(path)) => by_path(path, None),
            (Some(name), None) => by_name(name),
            (None, None) => None,
        });
    }
    skills
}

/// The turn a trace was observed in.
pub struct ObservedTurn<'a> {
    pub session: &'a Value,
    pub generation: &'a str,
    /// The message as the person sent it.
    pub message: &'a str,
    pub route: &'a ResolvedCommand,
    pub workspace: &'a Path,
}

/// Stores the trace of a completed turn. Returns the session event that says
/// what was kept, or why nothing was.
pub fn save(
    store: &SessionStore,
    turn: &ObservedTurn<'_>,
    trace: Trace,
    redactor: &dyn Redactor,
) -> Result<Value> {
    let tools = trace
        .steps
        .iter()
        .filter(|step| step["kind"] == "tool")
        .count();
    let skills = resolve_skills(turn.workspace, turn.route, turn.message, &trace);
    let skipped = |reason: &str, text: &str| json!({"kind": "skillcache", "phase": "not_observed", "reason": reason, "text": text});
    if tools == 0 {
        return Ok(skipped(
            "no_tool_calls",
            "SkillCache kept no trace: this turn ran no tool.",
        ));
    }
    if skills.is_empty() {
        return Ok(skipped(
            "no_skill",
            "SkillCache kept no trace: no skill of this project or of your home folder was used.",
        ));
    }
    let (request, _) = redactor.redact(turn.message);
    let mut redaction = trace.redaction();
    redaction["complete"] = trace.complete.into();
    let steps = Value::Array(trace.steps);
    let db = store.connection()?;
    db::save_trace(
        &db,
        &db::TraceRow {
            generation: turn.generation,
            session_id: turn.session["id"].as_str().unwrap_or(""),
            provider: &trace.provider,
            model: turn.session["model"].as_str(),
            request: &request,
            skill_refs: &Value::Array(skills.clone()),
            steps: &steps,
            redaction: &redaction,
        },
    )?;
    let names = skills
        .iter()
        .filter_map(|skill| skill["name"].as_str())
        .collect::<Vec<_>>();
    Ok(json!({
        "kind": "skillcache",
        "phase": "observed",
        "generation": turn.generation,
        "steps": steps.as_array().map_or(0, Vec::len),
        "tool_calls": tools,
        "skills": names,
        "redaction": redaction,
        "text": format!(
            "SkillCache kept this turn's trace ({tools} tool calls; skill: {}). It can be compiled into a capsule.",
            names.join(", ")
        ),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_redactor_replaces_environment_secrets_and_host_paths() {
        let redactor = SessionRedactor::new(
            Path::new("/work/project"),
            [
                (
                    "DEPLOY_TOKEN".to_string(),
                    "deploy-value-123456".to_string(),
                ),
                ("DB_PASSWORD".to_string(), "short".to_string()),
                ("EDITOR".to_string(), "not-a-secret-editor".to_string()),
                (
                    "SERVICE_SECRET".to_string(),
                    "service-secret-value".to_string(),
                ),
            ],
        );
        let (clean, changed) = redactor.redact(
            "curl -H 'Authorization: Bearer abcdefghijklmnop' https://x/?t=deploy-value-123456 \
             && cat /work/project/src/a.rs && echo service-secret-value not-a-secret-editor short",
        );
        assert!(!clean.contains("deploy-value-123456"), "{clean}");
        assert!(!clean.contains("service-secret-value"), "{clean}");
        assert!(!clean.contains("abcdefghijklmnop"), "{clean}");
        // A value too short to be told from ordinary text stays; so does a
        // variable that is not credential-like.
        assert!(clean.contains("short") && clean.contains("not-a-secret-editor"));
        assert!(!clean.contains("/work/project"), "{clean}");
        assert!(clean.contains("src/a.rs"), "{clean}");
        assert!(changed >= 3, "{changed}");
        assert_eq!(redactor.redact("plain text").1, 0);
    }
}
