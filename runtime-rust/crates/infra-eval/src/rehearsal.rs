//! Native hook rehearsal shared by the lifecycle experiment. All project mutations use private copies.
use crate::executable;
use crate::{Result, artifact, bench_driver, process, reconstruction};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
pub const PROMPT: &str = "Rehearsal: where does this project keep its main configuration?";
pub fn event_names(host: &str) -> Result<[&'static str; 3]> {
    match host {
        "claude" | "codex" => Ok(["SessionStart", "UserPromptSubmit", "Stop"]),
        "cursor" => Ok(["sessionStart", "beforeSubmitPrompt", "stop"]),
        _ => Err(artifact::invalid("unknown rehearsal host")),
    }
}
pub fn wiring(host: &str) -> Result<&'static str> {
    match host {
        "claude" => Ok(".claude/settings.json"),
        "codex" => Ok(".codex/hooks.json"),
        "cursor" => Ok(".cursor/hooks.json"),
        _ => Err(artifact::invalid("unknown hook host")),
    }
}
pub fn commands(settings: &Value, event: &str, host: &str) -> Vec<String> {
    let groups = &settings["hooks"][event];
    if host == "cursor" {
        groups
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v["command"].as_str().map(String::from))
            .collect()
    } else {
        groups
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|v| v["hooks"].as_array().into_iter().flatten())
            .filter_map(|v| v["command"].as_str().map(String::from))
            .collect()
    }
}
pub fn run_command(
    argv: Vec<String>,
    cwd: &Path,
    env: &BTreeMap<String, String>,
    input: Vec<u8>,
    timeout: Duration,
) -> Result<Value> {
    let scratch = artifact::Scratch::new(&std::env::temp_dir(), "hook-call")?;
    let out = scratch.0.join("stdout");
    let err = scratch.0.join("stderr");
    let started = Instant::now();
    let spec = process::CommandSpec {
        argv: argv.clone(),
        cwd: cwd.into(),
        env: env.clone(),
        input,
    };
    let budget = process::Budget {
        timeout,
        cap: usize::MAX,
        max_bytes: 8 * 1024 * 1024,
        native: false,
        strict_events: false,
        error_diagnostics_terminal: false,
    };
    let session = process::execute(&spec, &budget, Some(&out), Some(&err), |_, _| None)?;
    let bytes = |p: &Path| {
        artifact::read_file(p, 8 * 1024 * 1024)
            .map(|r| String::from_utf8_lossy(&r.0).into_owned())
            .unwrap_or_default()
    };
    Ok(
        json!({"hook":argv.last().map(|s|s.chars().take(100).collect::<String>()),"exit":if session.stopped=="timeout"{124}else{session.exit.unwrap_or(-1)},"ms":started.elapsed().as_millis(),"stdout":bytes(&out),"stderr":bytes(&err),"stopped":session.stopped}),
    )
}
#[derive(Clone)]
pub struct Harness {
    pub source: PathBuf,
    pub runtime: PathBuf,
    pub project: PathBuf,
    pub state: PathBuf,
    pub work: PathBuf,
    pub edition: String,
    pub attached: bool,
    pub env: BTreeMap<String, String>,
}
impl Harness {
    pub fn new(
        source: &Path,
        runtime: &Path,
        project: &Path,
        work: &Path,
        edition: &str,
        attached: bool,
    ) -> Result<Self> {
        for p in [
            work.join("home"),
            work.join("tmp"),
            work.join("codex-home"),
            work.join("home/.claude/projects"),
            work.join("codex-home/sessions"),
        ] {
            artifact::private_dir(&p)?;
        }
        let mut env = bench_driver::clean_env();
        env.insert("HOME".into(), work.join("home").to_string_lossy().into());
        env.insert("TMPDIR".into(), work.join("tmp").to_string_lossy().into());
        env.insert(
            "CODEX_HOME".into(),
            work.join("codex-home").to_string_lossy().into(),
        );
        env.insert(
            "CLAUDE_CONFIG_DIR".into(),
            work.join("home/.claude").to_string_lossy().into(),
        );
        env.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
        env.insert(
            "GIT_CONFIG_GLOBAL".into(),
            work.join("home/.gitconfig").to_string_lossy().into(),
        );
        std::fs::write(
            work.join("home/.gitconfig"),
            "[user]\nname=Rehearsal\nemail=rehearsal@example.invalid\n[safe]\ndirectory=*\n",
        )?;
        let state = if attached {
            let state = infra_tooling::attach::state_directory(
                project,
                Some(&work.join("attached-state")),
            )?;
            infra_tooling::attach::build_overlay(source, project, &state, edition, "claude")?;
            state
        } else {
            project.into()
        };
        Ok(Self {
            source: source.into(),
            runtime: runtime.into(),
            project: project.into(),
            state,
            work: work.into(),
            edition: edition.into(),
            attached,
            env,
        })
    }
    pub fn context(&self, project: &Path, owner: &str) -> Result<infra_core::RuntimeContext> {
        let roots = if self.attached {
            infra_core::Roots {
                tooling: self.source.join(&self.edition),
                project: project.into(),
                state: self.state.clone(),
                shared: false,
            }
        } else {
            infra_core::Roots::installed(&self.state)?
        };
        infra_core::RuntimeContext::new(roots, owner, "governed")
    }
    pub fn dispatcher(&self, host: &str) -> Result<PathBuf> {
        let folder = self.work.join(format!("{host}-dispatcher"));
        artifact::private_dir(&folder)?;
        let engine = folder.join("infra-engine");
        if !engine.exists() {
            let (raw, mode) = executable::read(&self.runtime)?;
            artifact::fresh_file(&engine, &raw, mode | 0o100)?;
        }
        let path = folder.join("ai-accelerator-dispatch.sh");
        if !path.exists() {
            let script = format!(
                "#!/bin/sh\nexec \"$(dirname -- \"$0\")/infra-engine\" dispatch {} --store \"$(dirname -- \"$0\")\" \"$@\"\n",
                shell_words::quote(host)
            );
            artifact::fresh_file(&path, script.as_bytes(), 0o755)?;
        }
        std::fs::write(
            folder.join("projects"),
            format!("{}\n", self.project.display()),
        )?;
        artifact::write_json(
            &folder.join("native-dispatch.json"),
            &json!({"profile":"infra-dispatch-v1","version":if host=="claude"{6}else{2},"engine":engine,"sha256":artifact::sha(&artifact::read_file(&engine,512*1024*1024)?.0)}),
        )?;
        Ok(path)
    }
    pub fn hook_env(&self, host: &str, launch: &Path) -> Result<BTreeMap<String, String>> {
        let mut env = self.env.clone();
        if self.attached {
            let overlay = infra_tooling::attach::build_overlay(
                &self.source,
                launch,
                &self.state,
                &self.edition,
                host,
            )?;
            env.extend(overlay.environment);
            for (k, v) in overlay.settings["env"].as_object().into_iter().flatten() {
                if let Some(v) = v.as_str() {
                    env.insert(k.clone(), v.into());
                }
            }
        }
        if host == "claude" {
            env.insert("CLAUDE_PROJECT_DIR".into(), launch.to_string_lossy().into());
        }
        Ok(env)
    }
    pub fn settings(&self, host: &str) -> Result<Value> {
        if self.attached {
            let definitions = infra_tooling::attach::hook_definitions(
                &self.source,
                &self.edition,
                host,
                &self.runtime,
            )?;
            Ok(json!({"hooks":definitions}))
        } else {
            artifact::read_json(&self.project.join(wiring(host)?), artifact::MAX_FILE)
        }
    }
    pub fn argv(
        &self,
        host: &str,
        event: &str,
        through: bool,
    ) -> Result<Vec<(String, Vec<String>)>> {
        let mut out = vec![];
        let commands = commands(&self.settings(host)?, event, host);
        let role_expression = regex::Regex::new(r"([\w.-]+\.sh)").ok();
        for command in commands {
            let role = role_expression
                .as_ref()
                .and_then(|r| r.captures_iter(&command).last().map(|m| m[1].to_string()))
                .or_else(|| {
                    shell_words::split(&command)
                        .ok()
                        .and_then(|words| words.iter().find(|s| s.ends_with(".sh")).cloned())
                })
                .unwrap_or_default();
            let argv = if through {
                if role.is_empty() {
                    return Err(artifact::invalid("dispatcher hook role unavailable"));
                }
                vec![
                    "bash".into(),
                    self.dispatcher(host)?.to_string_lossy().into(),
                    role.clone(),
                ]
            } else {
                vec!["bash".into(), "-c".into(), command.clone()]
            };
            out.push((
                if role.is_empty() {
                    command.chars().take(60).collect()
                } else {
                    role
                },
                argv,
            ));
        }
        Ok(out)
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "Hook rehearsal preserves host, launch and workspace custody together with the exact event, payload, dispatch policy and timeout."
    )]
    pub fn event(
        &self,
        host: &str,
        launch: &Path,
        place: &Path,
        event: &str,
        payload: &Value,
        through: bool,
        timeout: Duration,
    ) -> Result<Vec<Value>> {
        let env = self.hook_env(host, launch)?;
        let mut out = vec![];
        for (label, argv) in self.argv(host, event, through && !self.attached)? {
            let mut result = run_command(argv, place, &env, serde_json::to_vec(payload)?, timeout)?;
            result["hook"] = label.into();
            out.push(result);
        }
        Ok(out)
    }
    pub fn session_env(
        &self,
        host: &str,
        launch: &Path,
        place: &Path,
        through: bool,
        timeout: Duration,
    ) -> Result<BTreeMap<String, String>> {
        let mut env = self.hook_env(host, launch)?;
        if host == "claude" && through && !self.attached {
            let file = self
                .work
                .join(format!("session-{}.env", uuid::Uuid::new_v4()));
            artifact::fresh_file(&file, &[], 0o600)?;
            let mut extra = env.clone();
            extra.insert("CLAUDE_ENV_FILE".into(), file.to_string_lossy().into());
            let dispatcher = self.dispatcher(host)?;
            run_command(
                vec![
                    "bash".into(),
                    dispatcher.to_string_lossy().into(),
                    "environment".into(),
                ],
                place,
                &extra,
                b"{}".to_vec(),
                timeout,
            )?;
            let shell = "set -a; . \"$1\" >/dev/null 2>&1; set +a; env -0";
            let result = run_command(
                vec![
                    "bash".into(),
                    "-c".into(),
                    shell.into(),
                    "bash".into(),
                    file.to_string_lossy().into(),
                ],
                place,
                &env,
                vec![],
                timeout,
            )?;
            let _ = std::fs::remove_file(file);
            for row in result["stdout"].as_str().unwrap_or("").split('\0') {
                if let Some((k, v)) = row.split_once('=')
                    && !["PATH", "PWD", "SHLVL", "OLDPWD", "_"].contains(&k)
                {
                    env.insert(k.into(), v.into());
                }
            }
        }
        Ok(env)
    }
    pub fn journal(&self) -> Vec<Value> {
        std::fs::read_to_string(self.state.join("memory-bank/local/native-results.ndjson"))
            .unwrap_or_default()
            .lines()
            .filter_map(|s| artifact::strict(s.as_bytes()).ok())
            .collect()
    }
}
pub fn payload(host: &str, session: &str, event: &str, place: &Path, fields: Value) -> Value {
    let mut v = if host == "cursor" {
        json!({"conversation_id":session,"generation_id":uuid::Uuid::new_v4().to_string(),"workspace_roots":[place],"cursor_version":"1.7.36"})
    } else {
        json!({"session_id":session,"cwd":place,"turn_id":uuid::Uuid::new_v4().to_string()})
    };
    v["hook_event_name"] = event.into();
    for (k, value) in fields.as_object().into_iter().flatten() {
        v[k] = value.clone();
    }
    v
}
pub fn capsule(root: &Path, host: &str, results: &[Value]) -> String {
    if host == "cursor" {
        return std::fs::read_to_string(root.join(".cursor/rules/working-memory.mdc"))
            .unwrap_or_default();
    }
    results
        .iter()
        .filter_map(|r| r["stdout"].as_str())
        .filter_map(|s| artifact::strict(s.as_bytes()).ok())
        .map(|v| {
            v["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .or_else(|| v["additional_context"].as_str())
                .unwrap_or("")
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
pub fn request_of(results: &[Value]) -> Option<String> {
    for r in results {
        let Ok(v) = artifact::strict(r["stdout"].as_str().unwrap_or("").as_bytes()) else {
            continue;
        };
        if let Some(s) = v["followup_message"].as_str() {
            return Some(s.into());
        }
        if let Some(s) = v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .filter(|s| s.contains("native-result"))
        {
            return Some(s.into());
        }
        if v["decision"] == "block" {
            return Some(v["reason"].as_str().unwrap_or("").into());
        }
    }
    None
}
/// The exact command a native save request names: Claude Code's
/// `cd ROOT && echo -E '<JSON>' | memory-bank/bin/infra-engine memory ...`
/// (with an optional cd back), the engine path Codex and Cursor pipe into,
/// or the retired runtime's `(cd ROOT && ...)`.
pub const SAVE_COMMAND: &str = r"exact command: (\(cd .*?--json\)|cd [^\n]*?--json(?: && cd [^\n]*)?|'[^'\n]*infra-engine(?:\.exe)?' memory [^\n]*?--json)\n";
pub fn save_command(request: &str, data: &Value) -> Result<(String, Vec<u8>)> {
    let pattern =
        regex::Regex::new(SAVE_COMMAND).map_err(|_| artifact::invalid("save expression"))?;
    let command = pattern
        .captures(request)
        .map(|m| m[1].to_string())
        .ok_or_else(|| artifact::invalid("request names no exact command"))?;
    let raw = serde_json::to_string(data)?;
    if command.contains("'<JSON>'") {
        Ok((
            command.replace("'<JSON>'", &shell_words::quote(&raw)),
            vec![],
        ))
    } else {
        Ok((command, raw.into_bytes()))
    }
}
pub fn rehearse(
    h: &Harness,
    host: &str,
    profile: &str,
    place: &Path,
    timeout: Duration,
) -> Result<Value> {
    let names = event_names(host)?;
    let through = place != h.project;
    let mut runner = h.clone();
    if profile != "config" {
        runner
            .env
            .insert("CONTEXT_CAPSULE_PROFILE".into(), profile.into());
    }
    let h = &runner;
    let before = h.journal().len();
    let marker = format!(
        "rehearsal{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    let source = bench_driver::source_in(&h.project)?;
    let first = uuid::Uuid::new_v4().to_string();
    let mut steps = BTreeMap::<String, Vec<Value>>::new();
    let event = |session: &str, index: usize, fields: Value| {
        h.event(
            host,
            &h.project,
            place,
            names[index],
            &payload(host, session, names[index], place, fields),
            through,
            timeout,
        )
    };
    steps.insert(
        "start".into(),
        event(&first, 0, json!({"source":"startup"}))?,
    );
    steps.insert(
        "prompt".into(),
        event(&first, 1, json!({"prompt":PROMPT,"attachments":[]}))?,
    );
    let text = capsule(place, host, &steps["prompt"]);
    steps.insert("stop".into(),event(&first,2,json!({"stop_hook_active":false,"last_assistant_message":"Checked.","status":"completed","loop_count":0}))?);
    let request = request_of(&steps["stop"]);
    let mut save = json!({"requested":request.is_some(),"saved":false,"refusal":null});
    if let Some(request) = request {
        let data = json!({"progress":"Rehearsal turn: checked where the configuration lives.","verified":true,"learnings":[{"type":"finding","title":format!("Configuration entry point {marker}"),"consequence":format!("The project's configuration starts at {source} ({marker})."),"sources":[source]}]});
        match save_command(&request, &data) {
            Ok((command, input)) => {
                let env = h.session_env(host, &h.project, place, through, timeout)?;
                let done = run_command(
                    vec!["bash".into(), "-c".into(), command],
                    place,
                    &env,
                    input,
                    timeout,
                )?;
                save["saved"] = (done["exit"] == 0).into();
                save["save_ms"] = done["ms"].clone();
                if done["exit"] != 0 {
                    let output = format!(
                        "{}{}",
                        done["stdout"].as_str().unwrap_or(""),
                        done["stderr"].as_str().unwrap_or("")
                    );
                    let label =
                        regex::Regex::new(r"\b(memory-result-v1:[a-z_]+|native-result:[a-z-]+)")
                            .ok()
                            .and_then(|r| r.captures(&output).map(|m| m[1].to_string()))
                            .unwrap_or_else(|| {
                                output
                                    .chars()
                                    .rev()
                                    .take(300)
                                    .collect::<String>()
                                    .chars()
                                    .rev()
                                    .collect()
                            });
                    save["refusal"] = label.into();
                }
            }
            Err(_) => save["refusal"] = "the request names no command".into(),
        }
    }
    steps.insert("closing stop".into(),event(&first,2,json!({"stop_hook_active":true,"last_assistant_message":"Saved.","status":"completed","loop_count":1}))?);
    steps.insert("next prompt".into(),event(&uuid::Uuid::new_v4().to_string(),1,json!({"prompt":format!("What did the last session find about {marker}?"),"attachments":[]}))?);
    let recalled = capsule(place, host, &steps["next prompt"]).contains(&marker);
    let failures = steps
        .iter()
        .flat_map(|(name, rs)| {
            rs.iter()
                .filter(|r| r["exit"] != 0)
                .map(move |r| format!("{name}: {} exit {}", r["hook"], r["exit"]))
        })
        .collect::<Vec<_>>();
    let mut report = json!({"host":host,"profile":profile,"source":source,"capsule_chars":text.chars().count(),"ms":steps.iter().map(|(k,v)|(k.clone(),v.iter().filter_map(|r|r["ms"].as_u64()).sum::<u64>())).collect::<BTreeMap<_,_>>(),"recalled":recalled,"hook_failures":failures,"stderr":steps.values().flatten().filter(|r|r["stderr"].as_str().is_some_and(|s|!s.is_empty())).map(|r|format!("{}: {}",r["hook"],r["stderr"])).collect::<Vec<_>>(),"journal":h.journal().into_iter().skip(before).map(|v|["stage","event","code"].iter().filter_map(|k|v[*k].as_str()).collect::<Vec<_>>().join(":")).collect::<Vec<_>>(),"ok":save["saved"]==true&&recalled&&failures.is_empty()});
    for (k, v) in save.as_object().unwrap() {
        report[k] = v.clone();
    }
    if through {
        let branch = reconstruction::git(place, &["branch", "--show-current"])?;
        let ctx = h.context(place, "local")?;
        let task = infra_core::BrainRepository::new(ctx)?
            .inventory(false)?
            .records
            .iter()
            .any(|r| r.record["type"] == "task" && r.record["external_id"] == branch);
        let clean = reconstruction::git(place, &["status", "--porcelain", "--ignored"])?.is_empty();
        report["place"] = "worktree".into();
        report["worktree_task"] = task.into();
        report["worktree_clean"] = clean.into();
        report["ok"] = (report["ok"] == true && task && clean).into();
    }
    Ok(report)
}
fn values(v: &Value, key: &str, defaults: &[&str]) -> Vec<String> {
    match &v[key] {
        Value::String(s) => s
            .split(',')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
        Value::Array(a) => a
            .iter()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect(),
        _ => defaults.iter().map(|s| s.to_string()).collect(),
    }
}
pub fn run(source: &Path, runtime: &Path, v: &Value) -> Result<Value> {
    let targets = values(v, "target", &[]);
    if targets.is_empty() {
        return Err(artifact::invalid("--target required"));
    }
    let hosts = values(v, "hosts", &["claude", "codex"]);
    for h in &hosts {
        event_names(h)?;
    }
    let profiles = values(v, "profiles", &["config", "recall"]);
    if profiles
        .iter()
        .any(|p| !["config", "full", "navigation", "recall"].contains(&p.as_str()))
    {
        return Err(artifact::invalid("invalid rehearsal profile"));
    }
    let timeout = v["timeout"].as_f64().unwrap_or(60.0);
    if !timeout.is_finite() || timeout <= 0.0 {
        return Err(artifact::invalid("invalid hook budget"));
    }
    let timeout = Duration::from_secs_f64(timeout);
    let base = v["work"].as_str().map(PathBuf::from).unwrap_or_else(|| {
        if Path::new("/dev/shm").is_dir() {
            PathBuf::from("/dev/shm")
        } else {
            std::env::temp_dir()
        }
    });
    let work = artifact::Scratch::new(&base, "memory-rollout")?;
    let mut reports = vec![];
    let installer = infra_tooling::installer::Installer::new(source)?.with_engine(runtime)?;
    for (target_index, target) in targets.iter().enumerate() {
        if crate::cancel::interrupted() {
            break;
        }
        let target = artifact::ordinary(Path::new(target))?;
        if artifact::overlaps(source, &target)?
            || target.join(".git").is_file()
            || installer.detect_edition(&target)?.is_none()
        {
            return Err(artifact::invalid(
                "target must be standalone installed project outside tooling",
            ));
        }
        let dir = work.0.join(format!("target-{target_index}"));
        artifact::private_dir(&dir)?;
        let base = dir.join("synced");
        let mut excluded = BTreeSet::from([
            "node_modules".into(),
            "vendor".into(),
            "var".into(),
            ".worktrees".into(),
            ".claude/worktrees".into(),
        ]);
        for p in values(v, "exclude", &[]) {
            artifact::relative(&p)?;
            excluded.insert(p);
        }
        bench_driver::copy_tree(&target, &base, &excluded)?;
        let sync = installer.sync(&base, &Default::default())?;
        let profile = artifact::read_json(
            &base.join("project-brain/config/runtime.json"),
            artifact::MAX_FILE,
        )
        .ok()
        .map(|v| v["capsule_profile"].clone())
        .unwrap_or(Value::Null);
        let edition = installer
            .detect_edition(&base)?
            .ok_or_else(|| artifact::invalid("copied edition undetermined"))?;
        let mut runs = vec![];
        for host in &hosts {
            for profile in &profiles {
                let copy = dir.join(format!("{host}-{profile}"));
                bench_driver::copy_tree(&base, &copy, &BTreeSet::new())?;
                let harness = Harness::new(source, runtime, &copy, &dir, &edition, false)?;
                runs.push(rehearse(&harness, host, profile, &copy, timeout)?);
                if v["worktree"] == true && host != "cursor" {
                    let place = copy.join(".claude/worktrees/rehearsal");
                    if let Some(p) = place.parent() {
                        std::fs::create_dir_all(p)?;
                    }
                    let branch = format!(
                        "rehearsal/worktree-{}",
                        &uuid::Uuid::new_v4().simple().to_string()[..8]
                    );
                    reconstruction::git(
                        &copy,
                        &[
                            "worktree",
                            "add",
                            "-q",
                            "-b",
                            &branch,
                            place
                                .to_str()
                                .ok_or_else(|| artifact::invalid("worktree encoding"))?,
                        ],
                    )?;
                    runs.push(rehearse(&harness, host, profile, &place, timeout)?);
                }
            }
        }
        reports.push(json!({"target":target,"sync":sync,"profile_in_config":profile,"runs":runs}));
    }
    let ok = reports.len() == targets.len()
        && reports.iter().all(|r| {
            r["runs"]
                .as_array()
                .is_some_and(|rs| !rs.is_empty() && rs.iter().all(|r| r["ok"] == true))
        });
    let mut result = json!(reports);
    if v["keep"] == true {
        let path = work.0.clone();
        std::mem::forget(work);
        for r in result.as_array_mut().unwrap() {
            r["work"] = json!(path);
        }
    }
    let stdout = if v["json"] == true {
        format!(
            "{}\n",
            infra_core::domain::json::serialize(&result, false, false, true, false)?
        )
    } else {
        render(&result)
    };
    Ok(
        json!({"_cli":{"code":if crate::cancel::interrupted(){128+crate::cancel::signal()}else if ok{0}else{1},"stdout":stdout,"stderr":""},"result":result}),
    )
}
pub fn render(v: &Value) -> String {
    let mut lines = vec![];
    for r in v.as_array().into_iter().flatten() {
        lines.push(r["target"].as_str().unwrap_or("").to_string());
        let sync = &r["sync"];
        lines.push(format!(
            "  sync: {} changed, {} kept",
            sync["changed"].as_array().map(Vec::len).unwrap_or(0),
            sync["kept"].as_array().map(Vec::len).unwrap_or(0)
        ));
        lines.push(format!(
            "  capsule profile in runtime.json: {}",
            r["profile_in_config"]
        ));
        for run in r["runs"].as_array().into_iter().flatten() {
            let label = format!(
                "{}{}",
                run["profile"].as_str().unwrap_or(""),
                if run["place"] == "worktree" {
                    "@worktree"
                } else {
                    ""
                }
            );
            let outcome = if run["saved"] == true {
                "saved".into()
            } else if run["requested"] == true {
                format!("refused ({})", run["refusal"])
            } else {
                "no request".into()
            };
            lines.push(format!(
                "  {:6} {:19} prompt {:5} ms  stop {:5} ms  {:28} recalled {}",
                run["host"].as_str().unwrap_or(""),
                label,
                run["ms"]["prompt"],
                run["ms"]["stop"],
                outcome,
                if run["recalled"] == true { "yes" } else { "NO" }
            ));
        }
    }
    format!("{}\n", lines.join("\n"))
}

#[cfg(test)]
mod save_command_tests {
    use super::*;

    #[test]
    fn requests_name_their_save_in_every_host_spelling() {
        let data = json!({"progress":"It's checked.","verified":true,"learnings":[]});
        let claude = "This is an internal continuation. Use your shell tool to run this exact command: cd '/srv/shop' && echo -E '<JSON>' | memory-bank/bin/infra-engine memory --root '/srv/shop' native-result --request ab12 --token cd34 --input - --json\nPut one JSON object in place of <JSON>.";
        let (command, input) = save_command(claude, &data).unwrap();
        assert!(
            command.starts_with("cd '/srv/shop' && echo -E ")
                && !command.contains("<JSON>")
                && command.ends_with(" --input - --json")
                && input.is_empty(),
            "{command}"
        );
        let attached = "Use your shell tool to run this exact command: cd '/clone/PHP Core' && echo -E '<JSON>' | memory-bank/bin/infra-engine memory --root '/state' --project '/work' native-result --request ab12 --token cd34 --json && cd '/work'\n";
        let (command, _) = save_command(attached, &data).unwrap();
        assert!(command.ends_with(" --json && cd '/work'"), "{command}");
        for engine in [
            "'/srv/shop/memory-bank/bin/infra-engine'",
            r"'C:\shop\memory-bank\bin\infra-engine.exe'",
        ] {
            let piped = format!(
                "INTERNAL_NATIVE_MEMORY_RESULT cd34\nUse your shell tool to pipe one JSON object into this exact command: {engine} memory --root '/srv/shop' native-result --request ab12 --token cd34 --input - --json\nNative result reference schema v1."
            );
            let (command, input) = save_command(&piped, &data).unwrap();
            assert_eq!(
                command,
                format!(
                    "{engine} memory --root '/srv/shop' native-result --request ab12 --token cd34 --input - --json"
                )
            );
            assert_eq!(input, serde_json::to_vec(&data).unwrap());
        }
        let retired = "pipe one JSON object into this exact command: (cd /srv/shop && python3 memory-bank/scripts/context.py --root /srv/shop native-result --input - --json)\n";
        assert!(save_command(retired, &data).is_ok());
        assert!(save_command("no command here\n", &data).is_err());
    }
}
