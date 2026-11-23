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
pub const SKILL_FOLDERS: [&str; 3] = [".claude/skills", ".agents/skills", ".cursor/skills"];

/// Whether `workspace` or the user's home directory contains at least one
/// discoverable `SKILL.md` folder for Claude, Codex, or Cursor.
pub fn workspace_has_skills(workspace: &Path) -> bool {
    let home = run_activity::user_home().map(PathBuf::from);
    for root in [Some(workspace), home.as_deref()].into_iter().flatten() {
        for folder in SKILL_FOLDERS {
            let Ok(entries) = std::fs::read_dir(root.join(folder)) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path().join("SKILL.md");
                if std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_file()) {
                    return true;
                }
            }
        }
    }
    false
}

/// Whether this cold turn should pay the pre-turn workspace snapshot + hash
/// cost before the provider runs.
///
/// Unrouted chat turns in `auto` mode carry no requested skill and skip
/// pre-turn hashing; explicit `observe` turns snapshot whenever a skill is
/// routed or at least one skill folder is present in the workspace/home.
pub fn should_snapshot_for_observation(
    session: &Value,
    route: &ResolvedCommand,
    workspace: &Path,
) -> bool {
    if !route.requests.is_empty() || route.mode == "native" {
        return true;
    }
    crate::skillcache::settings::mode(session) == "observe" && workspace_has_skills(workspace)
}

/// As [`should_snapshot_for_observation`], also enabling snapshots when
/// opt-in `ToolChain` capture is enabled on `store` or `session`.
pub fn should_snapshot_for_observation_in_store(
    store: &SessionStore,
    session: &Value,
    route: &ResolvedCommand,
    workspace: &Path,
) -> bool {
    should_snapshot_for_observation(session, route, workspace)
        || (crate::skillcache::settings::records(session)
            && crate::skillcache::settings::tool_chain_enabled(store, session))
}

/// Stable 64-hex tree digest shared across subagents and linked worktrees on the same commit.
pub fn workspace_tree_sha(workspace: &Path) -> Result<String> {
    if let Ok(output) = std::process::Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(["rev-parse", "--verify", "--quiet", "HEAD^{tree}"])
        .output()
        && output.status.success()
    {
        let tree_oid = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !tree_oid.is_empty() {
            return Ok(fingerprint::sha256(
                format!("git-tree:{tree_oid}").as_bytes(),
            ));
        }
    }
    Ok(fingerprint::tree_sha256(workspace)?.sha256)
}

/// Conservative allowlist of strictly read-only/search tool names permitted in `ToolChain` traces.
pub fn is_read_only_tool_name(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "read"
            | "grep"
            | "glob"
            | "ls"
            | "read_file"
            | "view_file"
            | "view_file_outline"
            | "view_code_item"
            | "grep_search"
            | "find_by_name"
            | "list_dir"
            | "search"
    )
}

/// Store setting JSON controlling short-turn observation admission
/// (`{"min_tool_calls": 2, "min_output_tokens": ...}`).
pub const OBSERVE_MIN_ROI: &str = "skillcache_observe_min_roi";
const DEFAULT_MIN_TOOL_CALLS: usize = 2;

pub struct TraceRecorder {
    inner: Mutex<Option<(Recorder, crate::cost_observation::UsageComponents)>>,
}
impl TraceRecorder {
    pub fn new(provider: &str) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Some((
                Recorder::new(provider),
                crate::cost_observation::UsageComponents::new(provider),
            ))),
        })
    }
    /// One event in the provider's own format.
    pub fn observe(&self, raw: &Value) {
        if let Ok(mut guard) = self.inner.lock()
            && let Some((recorder, usage)) = guard.as_mut()
        {
            recorder.observe(raw);
            usage.observe("main", raw);
        }
    }
    /// The recorded trace and normalized usage snapshot; a recorder gives them once.
    pub fn finish_with_usage(&self, redactor: &dyn Redactor) -> Option<(Trace, Value)> {
        let (recorder, usage) = self.inner.lock().ok()?.take()?;
        Some((recorder.finish(redactor), usage.snapshot()))
    }
    /// The recorded trace; a recorder gives it once.
    pub fn finish(&self, redactor: &dyn Redactor) -> Option<Trace> {
        self.finish_with_usage(redactor).map(|(trace, _)| trace)
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
#[derive(Clone)]
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

/// How the skill whose entry document is `file` is named in a capsule, when
/// it is a skill folder of the workspace or of the user's home.
pub fn spelled_skill(workspace: &Path, file: &Path) -> Option<String> {
    let workspace = infra_core::fs::canonicalize(workspace).ok()?;
    let home = run_activity::user_home().map(PathBuf::from);
    let file = if file.is_absolute() {
        file.to_path_buf()
    } else {
        workspace.join(file)
    };
    let file = infra_core::fs::canonicalize(&file).ok()?;
    if file.file_name()?.to_str()? != "SKILL.md" || !file.is_file() {
        return None;
    }
    spelled(&workspace, home.as_deref(), &file)
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

fn frontmatter_name(file: &Path) -> Option<String> {
    use std::io::Read;
    // The name is in the first lines: the read is bounded, and a cut inside
    // a multi-byte character must not lose the lines before it.
    let mut bytes = Vec::new();
    std::fs::File::open(file)
        .ok()?
        .take(8192)
        .read_to_end(&mut bytes)
        .ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let rest = text
        .strip_prefix("---\r\n")
        .or_else(|| text.strip_prefix("---\n"))?;
    for line in rest.lines() {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("name:") {
            let value = value.trim().trim_matches(|ch| ch == '"' || ch == '\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
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
        for folder in SKILL_FOLDERS {
            if let Some(found) = [Some(workspace.as_path()), home]
                .into_iter()
                .flatten()
                .find_map(|root| {
                    skill_entry(
                        &workspace,
                        home,
                        &root.join(folder).join(name).join("SKILL.md"),
                        Some(name),
                    )
                })
            {
                return Some(found);
            }
        }
        for folder in SKILL_FOLDERS {
            for root in [Some(workspace.as_path()), home].into_iter().flatten() {
                let Ok(entries) = std::fs::read_dir(root.join(folder)) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let skill_md = entry.path().join("SKILL.md");
                    if frontmatter_name(&skill_md).as_deref() == Some(name)
                        && let Some(found) = skill_entry(&workspace, home, &skill_md, Some(name))
                    {
                        return Some(found);
                    }
                }
            }
        }
        None
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
    pub usage: &'a Value,
}

fn observe_min_roi(store: &SessionStore) -> (usize, Option<u64>) {
    let setting = store.setting(OBSERVE_MIN_ROI).ok().flatten();
    let min_tools = setting
        .as_ref()
        .and_then(|value| value["min_tool_calls"].as_u64())
        .and_then(|count| usize::try_from(count).ok())
        .unwrap_or(DEFAULT_MIN_TOOL_CALLS);
    let min_output = setting
        .as_ref()
        .and_then(|value| value["min_output_tokens"].as_u64());
    (min_tools, min_output)
}

/// Checks whether an observed trace meets recording eligibility without
/// writing to disk or computing workspace diffs.
pub fn check_admissible(
    store: &SessionStore,
    workspace: &Path,
    route: &ResolvedCommand,
    message: &str,
    trace: &Trace,
    usage: &Value,
) -> std::result::Result<(Vec<Value>, usize), Value> {
    let (skills, tools, _) =
        check_admissible_with_toolchain(store, false, workspace, route, message, trace, usage)?;
    Ok((skills, tools))
}

/// Checks trace admissibility, additionally admitting skillless read-only tool
/// chains when `allow_tool_chain` is true.
pub fn check_admissible_with_toolchain(
    store: &SessionStore,
    allow_tool_chain: bool,
    workspace: &Path,
    route: &ResolvedCommand,
    message: &str,
    trace: &Trace,
    usage: &Value,
) -> std::result::Result<(Vec<Value>, usize, Option<String>), Value> {
    let tools = trace
        .steps
        .iter()
        .filter(|step| step["kind"] == "tool")
        .count();
    let skills = resolve_skills(workspace, route, message, trace);
    let skipped = |reason: &str, text: &str| json!({"kind": "skillcache", "phase": "not_observed", "reason": reason, "text": text});
    if tools == 0 {
        return Err(skipped(
            "no_tool_calls",
            "SkillCache kept no trace: this turn ran no tool.",
        ));
    }
    let mut tool_chain_tree_sha = None;
    if skills.is_empty() {
        if !allow_tool_chain {
            return Err(skipped(
                "no_skill",
                "SkillCache kept no trace: no skill of this project or of your home folder was used.",
            ));
        }
        let has_mutating = trace
            .steps
            .iter()
            .filter(|step| step["kind"] == "tool")
            .any(|step| {
                let name = step["tool"]
                    .as_str()
                    .or_else(|| step["name"].as_str())
                    .unwrap_or("");
                !is_read_only_tool_name(name)
            });
        if has_mutating {
            return Err(skipped(
                "mutating_tool_chain",
                "SkillCache kept no trace: tool-chain capture admits read-only tool sequences only.",
            ));
        }
        tool_chain_tree_sha = Some(workspace_tree_sha(workspace).map_err(|err| {
            skipped(
                "storage",
                &format!("SkillCache could not fingerprint workspace tree: {err}"),
            )
        })?);
    }
    let (min_tools, min_output) = observe_min_roi(store);
    if tools < min_tools {
        return Err(skipped(
            "low_roi",
            "SkillCache kept no trace: this short turn is unlikely to pay back compilation.",
        ));
    }
    if let Some(min_output) = min_output
        && let Some(output_tokens) = usage["output_tokens"].as_u64()
        && output_tokens < min_output
    {
        return Err(skipped(
            "low_roi",
            "SkillCache kept no trace: this short turn produced too few tokens to pay back compilation.",
        ));
    }
    Ok((skills, tools, tool_chain_tree_sha))
}

/// Stores the trace of a completed turn. Returns the session event that says
/// what was kept, or why nothing was.
pub fn save(
    store: &SessionStore,
    turn: &ObservedTurn<'_>,
    trace: Trace,
    redactor: &dyn Redactor,
) -> Result<Value> {
    let allow_tool_chain = crate::skillcache::settings::tool_chain_enabled(store, turn.session);
    let (skills, tools, tool_chain_tree_sha) = match check_admissible_with_toolchain(
        store,
        allow_tool_chain,
        turn.workspace,
        turn.route,
        turn.message,
        &trace,
        turn.usage,
    ) {
        Ok(ok) => ok,
        Err(skipped) => return Ok(skipped),
    };
    let (request, _) = redactor.redact(turn.message);
    let mut redaction = trace.redaction();
    redaction["complete"] = trace.complete.into();
    if let Some(tree_sha) = &tool_chain_tree_sha {
        redaction["trace_kind"] = "tool_chain".into();
        redaction["tool_chain_tree_sha"] = tree_sha.clone().into();
    }
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
            usage: turn.usage,
            redaction: &redaction,
        },
    )?;
    let names = skills
        .iter()
        .filter_map(|skill| skill["name"].as_str())
        .collect::<Vec<_>>();
    let text = if tool_chain_tree_sha.is_some() {
        format!(
            "SkillCache kept this turn's read-only tool-chain trace ({tools} tool calls). It can be compiled into a tool-chain capsule."
        )
    } else {
        format!(
            "SkillCache kept this turn's trace ({tools} tool calls; skill: {}). It can be compiled into a capsule.",
            names.join(", ")
        )
    };
    Ok(json!({
        "kind": "skillcache",
        "phase": "observed",
        "generation": turn.generation,
        "steps": steps.as_array().map_or(0, Vec::len),
        "tool_calls": tools,
        "skills": names,
        "redaction": redaction,
        "text": text,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frontmatter_name_is_read_from_a_long_file_that_is_not_ascii() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("SKILL.md");
        let mut text = String::from("---\nname: отчёты\ndescription: d\n---\n");
        while text.len() < 8191 {
            text.push('a');
        }
        // The 8192nd byte is the first half of a two-byte character.
        text.push_str(&"ж".repeat(16));
        std::fs::write(&file, &text).unwrap();
        assert!(std::str::from_utf8(&text.as_bytes()[..8192]).is_err());
        assert_eq!(frontmatter_name(&file).as_deref(), Some("отчёты"));
        assert_eq!(frontmatter_name(&root.path().join("absent.md")), None);
    }

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

    #[test]
    fn save_persists_usage_components_and_enforces_roi_threshold() {
        let root = tempfile::tempdir().unwrap();
        let workspace = infra_core::fs::canonicalize(root.path()).unwrap();
        let skill_dir = workspace.join(".claude/skills/pdf");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "# PDF\n").unwrap();
        let store = SessionStore::open(&workspace.join(".state")).unwrap();
        let project_id = store.register(&workspace).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let sid = store
            .create(
                &json!({
                    "project_id": project_id,
                    "provider": "claude",
                    "mode": "edit",
                    "workflow": "native",
                    "permissions": "auto"
                }),
                "do pdf",
                &[],
            )
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let session = store.get(&sid).unwrap();
        let route = ResolvedCommand {
            mode: "text".into(),
            text: "do pdf".into(),
            requests: vec![json!({"name": "pdf", "path": skill_dir.join("SKILL.md")})],
            notice: None,
        };
        let redactor = SessionRedactor::new(&workspace, std::iter::empty::<(String, String)>());

        // Single-tool-call turn is skipped by default low-ROI filter.
        let one_tool = Trace {
            provider: "claude".into(),
            steps: vec![json!({"index": 0, "kind": "tool", "tool": "Read"})],
            skills: vec![],
            counters: Default::default(),
            complete: true,
        };
        let skipped = save(
            &store,
            &ObservedTurn {
                session: &session,
                generation: "g-short",
                message: "do pdf",
                route: &route,
                workspace: &workspace,
                usage: &json!({"input_tokens": 100, "output_tokens": 10, "source": "claude"}),
            },
            one_tool,
            &redactor,
        )
        .unwrap();
        assert_eq!(skipped["phase"], "not_observed");
        assert_eq!(skipped["reason"], "low_roi");

        // Configure min_output_tokens=500 -> low output token turn is skipped; >=500 is stored with usage.
        store
            .set_setting(
                OBSERVE_MIN_ROI,
                &json!({"min_tool_calls": 2, "min_output_tokens": 500}),
            )
            .unwrap();
        let two_tools = Trace {
            provider: "claude".into(),
            steps: vec![
                json!({"index": 0, "kind": "tool", "tool": "Read"}),
                json!({"index": 1, "kind": "tool", "tool": "Bash"}),
            ],
            skills: vec![],
            counters: Default::default(),
            complete: true,
        };
        let small_out = save(
            &store,
            &ObservedTurn {
                session: &session,
                generation: "g-small-tokens",
                message: "do pdf",
                route: &route,
                workspace: &workspace,
                usage: &json!({"input_tokens": 100, "output_tokens": 120, "source": "claude"}),
            },
            two_tools.clone(),
            &redactor,
        )
        .unwrap();
        assert_eq!(small_out["reason"], "low_roi");

        let usage = json!({"input_tokens": 300, "output_tokens": 700, "source": "claude"});
        let kept = save(
            &store,
            &ObservedTurn {
                session: &session,
                generation: "g-good",
                message: "do pdf",
                route: &route,
                workspace: &workspace,
                usage: &usage,
            },
            two_tools,
            &redactor,
        )
        .unwrap();
        assert_eq!(kept["phase"], "observed");
        let db = store.connection().unwrap();
        let stored = db::trace(&db, "g-good").unwrap().unwrap();
        assert_eq!(stored["usage"], usage);
    }

    #[test]
    fn resolve_skills_and_workspace_scan_include_cursor_skills() {
        let root = tempfile::tempdir().unwrap();
        let workspace = infra_core::fs::canonicalize(root.path()).unwrap();
        let cursor_dir = workspace.join(".cursor/skills/sc-cursor-only");
        std::fs::create_dir_all(&cursor_dir).unwrap();
        std::fs::write(cursor_dir.join("SKILL.md"), "# Cursor Skill\n").unwrap();
        assert!(workspace_has_skills(&workspace));

        let trace = Trace {
            provider: "cursor".into(),
            steps: vec![],
            skills: vec![SkillRef {
                name: Some("sc-cursor-only".into()),
                path: None,
            }],
            counters: Default::default(),
            complete: true,
        };
        let route = ResolvedCommand {
            mode: "text".into(),
            text: "$sc-cursor-only run".into(),
            requests: vec![],
            notice: None,
        };
        let resolved = resolve_skills(&workspace, &route, "$sc-cursor-only run", &trace);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0]["name"], "sc-cursor-only");
        assert_eq!(
            resolved[0]["path"],
            ".cursor/skills/sc-cursor-only/SKILL.md"
        );
    }

    #[test]
    fn check_admissible_with_toolchain_enforces_read_only_and_shares_tree_sha() {
        let root = tempfile::tempdir().unwrap();
        let ws = root.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let workspace = infra_core::fs::canonicalize(&ws).unwrap();
        std::fs::write(workspace.join("composer.json"), "{}").unwrap();
        let store = SessionStore::open(&root.path().join("state")).unwrap();
        let route = ResolvedCommand {
            mode: "text".into(),
            text: "inspect composer dependencies".into(),
            requests: vec![],
            notice: None,
        };
        let mut ro_trace = Trace {
            provider: "claude".into(),
            steps: vec![
                json!({"kind": "tool", "tool": "Read", "input": {"file_path": "composer.json"}}),
                json!({"kind": "tool", "tool": "Grep", "input": {"pattern": "laravel"}}),
            ],
            skills: vec![],
            counters: Default::default(),
            complete: true,
        };
        let default_denied = check_admissible_with_toolchain(
            &store,
            false,
            &workspace,
            &route,
            "inspect composer dependencies",
            &ro_trace,
            &Value::Null,
        )
        .unwrap_err();
        assert_eq!(default_denied["reason"], "no_skill");

        let (skills, tools, tree_sha) = check_admissible_with_toolchain(
            &store,
            true,
            &workspace,
            &route,
            "inspect composer dependencies",
            &ro_trace,
            &Value::Null,
        )
        .unwrap();
        assert!(skills.is_empty());
        assert_eq!(tools, 2);
        assert!(tree_sha.is_some());

        ro_trace
            .steps
            .push(json!({"kind": "tool", "tool": "Write", "input": {"file_path": "a.txt"}}));
        let mutating = check_admissible_with_toolchain(
            &store,
            true,
            &workspace,
            &route,
            "inspect composer dependencies",
            &ro_trace,
            &Value::Null,
        )
        .unwrap_err();
        assert_eq!(mutating["reason"], "mutating_tool_chain");
    }
}
