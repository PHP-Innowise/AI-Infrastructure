//! Native CLI command discovery and explicit custom-prompt/skill routing. No model is called by probes.
use crate::{
    HarnessError, Result,
    services::{Domain, Payload, Reply},
    store::SessionStore,
};
use infra_core::bank::publication::PublicationFiles;
use infra_host::{
    process::{self, Control, Limits},
    provider::{self, Provider},
};
use rusqlite::OptionalExtension;
use serde_json::{Value as V, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, RwLock, atomic::AtomicBool},
    time::{Duration, Instant},
};
const HEAD: usize = 16000;
const PROMPT_BYTES: usize = 64 * 1024;
const PROBE_BYTES: usize = 4 * 1024 * 1024;
const INIT_PROMPT: &str = "Create an AGENTS.md file in the current directory: a short contributor guide for this repository titled \"Repository Guidelines\". If AGENTS.md already exists, leave it unchanged and say so. Describe how this repository actually works: project structure, build, test and development commands, coding style and naming, testing, and commit and pull request conventions. Keep it to 200-400 words.";
#[derive(Clone)]
struct Entry {
    expires: Instant,
    loaded: Instant,
    value: Vec<V>,
    error: Option<String>,
}
#[derive(Default)]
struct Cache {
    entries: BTreeMap<String, Entry>,
    locks: BTreeMap<String, Arc<tokio::sync::Mutex<()>>>,
}
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
fn cache() -> &'static Mutex<Cache> {
    CACHE.get_or_init(|| Mutex::new(Cache::default()))
}
pub fn invalidate() {
    if let Ok(mut c) = cache().lock() {
        c.entries.clear();
    }
}
#[derive(Clone)]
pub struct CommandCatalog {
    pub store: SessionStore,
    pub source: PathBuf,
    pub providers: Arc<RwLock<BTreeMap<String, Provider>>>,
}
#[derive(Clone, Debug)]
pub struct ResolvedCommand {
    pub mode: String,
    pub text: String,
    pub requests: Vec<V>,
    pub notice: Option<String>,
}
impl ResolvedCommand {
    pub fn value(&self) -> V {
        json!({"mode":self.mode,"text":self.text,"requests":self.requests,"notice":self.notice})
    }
    pub fn skill_request(&self) -> String {
        request(&self.requests)
    }
}
impl CommandCatalog {
    pub fn new(
        store: SessionStore,
        source: PathBuf,
        providers: Arc<RwLock<BTreeMap<String, Provider>>>,
    ) -> Self {
        Self {
            store,
            source,
            providers,
        }
    }
    fn executable(&self, name: &str) -> Result<PathBuf> {
        self.providers
            .read()
            .map_err(|_| HarnessError::unavailable("Provider registry unavailable."))?
            .get(name)
            .filter(|p| p.available)
            .and_then(|p| p.executable.clone())
            .ok_or_else(|| HarnessError::unavailable("This provider CLI is unavailable."))
    }
    fn attached(&self, project: Option<&str>, workspace: &Path) -> Result<Option<PathBuf>> {
        let Some(project) = project else {
            return Ok(None);
        };
        let mut files = PublicationFiles::new(workspace)?;
        if files
            .read_prefix("memory-bank/runtime-contract.json", 0)?
            .is_some()
            || files
                .read_prefix("memory-bank/bin/infra-engine", 0)?
                .is_some()
            || files
                .read_prefix("memory-bank/bin/infra-engine.exe", 0)?
                .is_some()
        {
            return Ok(None);
        }
        let db = self.store.connection()?;
        let edition = db
            .query_row(
                "SELECT edition FROM project_accelerators WHERE project_id=?",
                [project],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(|_| HarnessError::unavailable("Accelerator registry unavailable."))?;
        let Some(edition) = edition else {
            return Ok(None);
        };
        if !["Laravel", "Symfony", "PHP Core", "Cms/wordpress"].contains(&edition.as_str()) {
            return Err(HarnessError::invalid(
                "Attached accelerator edition is invalid.",
            ));
        }
        let root = self.source.join(edition);
        PublicationFiles::new(&root)?;
        Ok(Some(root))
    }
    async fn cached<F, Fut>(
        &self,
        key: String,
        newer: Option<Duration>,
        load: F,
    ) -> Result<(Vec<V>, Option<String>)>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<V>>>,
    {
        let usable = |e: &Entry| {
            Instant::now() < e.expires && newer.map(|age| e.loaded.elapsed() < age).unwrap_or(true)
        };
        let gate = {
            let mut c = cache()
                .lock()
                .map_err(|_| HarnessError::unavailable("Command cache unavailable."))?;
            if let Some(entry) = c.entries.get(&key).filter(|e| usable(e)) {
                return Ok((entry.value.clone(), entry.error.clone()));
            }
            c.locks
                .entry(key.clone())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let _guard = gate.lock().await;
        {
            let c = cache()
                .lock()
                .map_err(|_| HarnessError::unavailable("Command cache unavailable."))?;
            if let Some(entry) = c.entries.get(&key).filter(|e| usable(e)) {
                return Ok((entry.value.clone(), entry.error.clone()));
            }
        }
        let (value, error, ttl) = match load().await {
            Ok(v) => (v, None, 60),
            Err(e) => (vec![], Some(e.message), 10),
        };
        let mut c = cache()
            .lock()
            .map_err(|_| HarnessError::unavailable("Command cache unavailable."))?;
        if c.entries.len() > 64 {
            c.entries.clear();
            c.locks.retain(|k, _| k == &key);
        }
        c.entries.insert(
            key,
            Entry {
                expires: Instant::now() + Duration::from_secs(ttl),
                loaded: Instant::now(),
                value: value.clone(),
                error: error.clone(),
            },
        );
        Ok((value, error))
    }
    async fn claude(
        &self,
        cwd: &Path,
        settings: &str,
        extra: &[String],
        newer: Option<Duration>,
    ) -> Result<(Vec<V>, Option<String>)> {
        let executable = match self.executable("claude") {
            Ok(e) => e,
            Err(e) => return Ok((vec![], Some(e.message))),
        };
        let key = json!(["claude", executable, cwd, settings, extra]).to_string();
        self.cached(key, newer, || {
            claude_commands(&executable, cwd, settings, extra, self.store.runner_lock())
        })
        .await
    }
    async fn codex(&self, cwd: &Path) -> Result<(Vec<V>, Option<String>)> {
        let executable = match self.executable("codex") {
            Ok(e) => e,
            Err(e) => return Ok((vec![], Some(e.message))),
        };
        let key = json!(["codex", executable, cwd]).to_string();
        self.cached(key, None, || {
            codex_skills(&executable, cwd, self.store.runner_lock())
        })
        .await
    }
    pub async fn catalogue(
        &self,
        provider: &str,
        cwd: &Path,
        settings: Option<&str>,
        project: Option<&str>,
        newer: Option<Duration>,
    ) -> Result<(Vec<V>, Option<String>)> {
        let home = self.attached(project, cwd)?;
        if provider == "claude" {
            let settings = settings
                .map(str::to_owned)
                .unwrap_or(claude_settings(false, 3, None)?);
            let extra = home
                .as_ref()
                .map(|h| vec!["--add-dir".into(), h.to_string_lossy().into_owned()])
                .unwrap_or_default();
            return self.claude(cwd, &settings, &extra, newer).await;
        }
        let (mut listed, error) = match provider {
            "codex" => {
                let (skills, error) = self.codex(cwd).await?;
                (skills.into_iter().map(|v|json!({"name":v["name"],"description":v["description"],"path":v["path"]})).collect::<Vec<_>>(),error)
            }
            "cursor" => (
                workspace_skills(cwd, provider)?
                    .0
                    .into_iter()
                    .filter(|s| s["user"] == true)
                    .collect(),
                None,
            ),
            _ => return Err(HarnessError::invalid("Choose a provider.")),
        };
        if let Some(home) = home {
            let (mut attached, _) = workspace_skills(&home, provider)?;
            let names = listed
                .iter()
                .filter_map(|v| v["name"].as_str().map(str::to_owned))
                .collect::<BTreeSet<_>>();
            for mut skill in attached.drain(..) {
                if let Some(path) = skill["path"].as_str() {
                    skill["path"] = json!(home.join(path));
                }
                if !names.contains(skill["name"].as_str().unwrap_or("")) {
                    listed.push(skill);
                }
            }
        }
        Ok((listed, error))
    }
    pub async fn listing(
        &self,
        provider: &str,
        cwd: &Path,
        settings: Option<&str>,
        project: Option<&str>,
    ) -> Result<V> {
        let (native, error) = self
            .catalogue(provider, cwd, settings, project, None)
            .await?;
        let(mut commands,skills)=match provider{
   "claude"=>(native.into_iter().map(|mut item|{let action=claude_page(item["name"].as_str().unwrap_or(""));item["kind"]=json!(if action.is_some(){"page"}else{"native"});item["action"]=action.map(|s|json!(s)).unwrap_or(V::Null);item}).collect::<Vec<_>>(),vec![]),
   "codex"=>{let mut commands=vec![];for(name,description,hint,action)in [("new","Start a new session in this project","","new"),("model","Choose the model for the next turn","[model]","model"),("diff","Show this session's changes","","diff"),("status","Show this session's usage","","status"),("init","Create an AGENTS.md contributor guide for this repository","","")]{commands.push(json!({"name":name,"description":description,"hint":hint,"aliases":[],"kind":if action.is_empty(){"prompt"}else{"page"},"action":if action.is_empty(){V::Null}else{json!(action)}}));}for prompt in codex_prompts()?{commands.push(json!({"name":prompt["name"],"description":prompt["description"],"hint":prompt["hint"],"aliases":[],"kind":"prompt","action":null}));}(commands,native)},
   "cursor"=>(native.into_iter().map(|item|json!({"name":item["name"],"description":item["description"],"hint":item["hint"],"aliases":[],"kind":"skill","action":null})).collect(),vec![]),_=>return Err(HarnessError::invalid("Choose a provider."))
  };
        navigation(&mut commands);
        Ok(json!({"provider":provider,"commands":commands,"skills":skills,"error":error}))
    }
    pub fn check(
        &self,
        provider: &str,
        message: &str,
        page: Option<&BTreeMap<String, String>>,
    ) -> Result<()> {
        let Some((name, args)) = leading(message) else {
            return Ok(());
        };
        let action = page
            .and_then(|p| p.get(name).map(String::as_str))
            .or_else(|| {
                if provider == "claude" {
                    claude_page(name)
                } else {
                    None
                }
            });
        if provider == "claude" {
            match action {
                Some("new") => {
                    return Err(HarnessError::invalid(format!(
                        "/{name} would start this conversation over. Use New session instead."
                    )));
                }
                Some("model" | "effort") => {
                    return Err(HarnessError::invalid(format!(
                        "The setting for the next turn is owned by this session: send /{name} with only a value, or choose it in session settings."
                    )));
                }
                _ => {}
            }
        }
        if provider == "codex" && name.starts_with("prompts:") {
            let prompt = codex_prompts()?
                .into_iter()
                .find(|v| v["name"] == name)
                .ok_or_else(|| {
                    HarnessError::missing(format!(
                        "There is no Codex prompt /{name} in the configured prompts directory."
                    ))
                })?;
            expand_prompt(name, prompt["body"].as_str().unwrap_or(""), args)?;
        }
        Ok(())
    }
    pub async fn route(&self, session: &V, message: &str) -> Result<ResolvedCommand> {
        let provider = session["provider"]
            .as_str()
            .ok_or_else(|| HarnessError::invalid("Missing session provider."))?;
        let cwd = PathBuf::from(
            session["project_path"]
                .as_str()
                .ok_or_else(|| HarnessError::invalid("Missing session workspace."))?,
        );
        let project = session["project_id"].as_str();
        let mut route = ResolvedCommand {
            mode: "text".into(),
            text: message.into(),
            requests: vec![],
            notice: None,
        };
        let lead = leading(message);
        if provider == "claude" {
            if let Some((name, _)) = lead {
                let settings = claude_settings(
                    session["agents_enabled"].as_bool().unwrap_or(false),
                    session["agent_count"].as_u64().unwrap_or(3) as u32,
                    session["thinking_effort"].as_str(),
                )?;
                let (mut commands, mut error) = self
                    .catalogue(provider, &cwd, Some(&settings), project, None)
                    .await?;
                let mut aliases = aliases(&commands);
                if !aliases.contains_key(name) && error.is_none() {
                    let fresh = self
                        .catalogue(
                            provider,
                            &cwd,
                            Some(&settings),
                            project,
                            Some(Duration::from_secs(5)),
                        )
                        .await?;
                    commands = fresh.0;
                    error = fresh.1;
                    aliases = self::aliases(&commands);
                }
                let page = aliases
                    .iter()
                    .filter_map(|(alias, target)| {
                        Some((alias.clone(), claude_page(target)?.to_owned()))
                    })
                    .collect::<BTreeMap<_, _>>();
                self.check(provider, message, Some(&page))?;
                if let Some(name) = aliases.get(name) {
                    route.mode = "native".into();
                    route.notice = Some(format!(
                        "Claude Code runs /{name} itself: the message went to it as typed, without Harness memory or instructions."
                    ));
                    if session["brain"].is_object() && session["brain"]["review"] != false {
                        return Err(HarnessError::invalid(
                            "This session reviews project memory before each run; a native Claude command goes to the CLI without it. Use automatic memory for native commands.",
                        ));
                    }
                    return Ok(route);
                }
                route.notice = Some(if let Some(error) = error {
                    format!(
                        "Claude Code could not list its commands ({error}), so /{name} went as text."
                    )
                } else {
                    format!("/{name} is not a Claude Code command here, so it went as text.")
                });
            }
        } else if provider == "codex" {
            if let Some((name, args)) = lead {
                if name.starts_with("prompts:") {
                    self.check(provider, message, None)?;
                    let prompt = codex_prompts()?
                        .into_iter()
                        .find(|v| v["name"] == name)
                        .ok_or_else(|| HarnessError::missing("Custom prompt disappeared."))?;
                    route.text = expand_prompt(name, prompt["body"].as_str().unwrap_or(""), args)?;
                    route.notice = Some(format!(
                        "Codex prompt /{name} expanded as the Codex app would."
                    ));
                } else if name == "init" {
                    route.text = if args.is_empty() {
                        INIT_PROMPT.into()
                    } else {
                        format!("{INIT_PROMPT}\n\nAlso: {args}")
                    };
                    route.notice = Some("Sent Codex /init as its instruction.".into());
                }
            }
            let mentioned = mentions(message);
            if !mentioned.is_empty() {
                let (skills, error) = self.catalogue(provider, &cwd, None, project, None).await?;
                for name in mentioned {
                    let name = if skills.iter().any(|s| s["name"] == name) {
                        name
                    } else {
                        strip_punctuation(&name)
                    };
                    if let Some(skill) = skills.iter().find(|s| s["name"] == name)
                        && !route.requests.contains(skill)
                    {
                        route.requests.push(skill.clone());
                    }
                    if route.requests.len() == 5 {
                        break;
                    }
                }
                let names = route
                    .requests
                    .iter()
                    .filter_map(|r| r["name"].as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                let notice = if !names.is_empty() {
                    Some(format!("Codex skills requested: {names}."))
                } else {
                    error.map(|e| format!("Codex could not list its skills ({e})."))
                };
                route.notice = join_notice(route.notice, notice);
            }
        } else if provider == "cursor" {
            if let Some((name, _)) = lead {
                let (skills, _) = self.catalogue(provider, &cwd, None, project, None).await?;
                if let Some(skill) = skills.iter().find(|s| s["name"] == name) {
                    route.requests.push(skill.clone());
                    route.notice = Some(format!("Cursor skill requested: {name}."));
                }
            }
        } else {
            return Err(HarnessError::invalid("Choose a provider."));
        }
        Ok(route)
    }
}
fn host(error: infra_host::HostError) -> HarnessError {
    HarnessError {
        status: error.status,
        code: error.code,
        message: error.message,
    }
}
pub fn claude_settings(agents: bool, count: u32, effort: Option<&str>) -> Result<String> {
    if !(1..=40).contains(&count) {
        return Err(HarnessError::invalid(
            "Helper count must be between 1 and 40.",
        ));
    }
    let ultra = effort == Some("ultracode");
    let mut settings = json!({"env":{"CLAUDE_CODE_MAX_CONCURRENT_SUBAGENTS":if agents{count.to_string()}else{"1".into()},"CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH":"1"},"ultracode":ultra});
    if !ultra {
        settings["disableWorkflows"] = json!(true);
    }
    Ok(settings.to_string())
}
async fn claude_commands(
    executable: &Path,
    cwd: &Path,
    settings: &str,
    extra: &[String],
    lock: Arc<std::fs::File>,
) -> Result<Vec<V>> {
    let mut settings: V = serde_json::from_str(settings)
        .map_err(|_| HarnessError::invalid("Invalid Claude settings."))?;
    settings["disableAllHooks"] = json!(true);
    let mut args =
        provider::command_argv(&[executable.to_string_lossy().into_owned()], Some("claude"))
            .map_err(host)?;
    args.extend(
        [
            "--print",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--settings",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    args.push(settings.to_string());
    args.extend_from_slice(extra);
    let raw=process::converse_environment(&args,cwd,&BTreeMap::new(),&["CONTEXT_CAPSULE_DELIVERED".into()],&[json!({"type":"control_request","request_id":"harness-commands","request":{"subtype":"initialize"}})],Limits{seconds:Some(10),output:PROBE_BYTES,line:PROBE_BYTES},Arc::new(AtomicBool::new(false)),Some(lock),|message|{if message["type"]!="control_response"||message["response"]["request_id"]!="harness-commands"{return Ok(Control::Continue);}let response=&message["response"];if response["subtype"]!="success"||!response["response"]["commands"].is_array(){return Err(infra_host::HostError::unavailable("Claude Code did not list its commands."));}Ok(Control::Done(response["response"]["commands"].clone()))}).await.map_err(host)?;
    let mut commands = vec![];
    for item in raw.as_array().into_iter().flatten().take(1000) {
        let Some(name) = item["name"].as_str().filter(|s| {
            !s.is_empty()
                && !s.starts_with('_')
                && s.chars().count() <= 200
                && !s.chars().any(|c| c.is_whitespace() || c < ' ')
        }) else {
            continue;
        };
        let aliases = item["aliases"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(V::as_str)
            .filter(|s| !s.is_empty() && !s.chars().any(char::is_whitespace))
            .take(8)
            .collect::<Vec<_>>();
        commands.push(json!({"name":name,"description":short(&item["description"],300),"hint":short(&item["argumentHint"],200),"aliases":aliases,"builtin":item["builtin"]==true}));
    }
    Ok(commands)
}
async fn codex_skills(executable: &Path, cwd: &Path, lock: Arc<std::fs::File>) -> Result<Vec<V>> {
    let args = provider::command_argv(
        &[
            executable.to_string_lossy().into_owned(),
            "app-server".into(),
        ],
        Some("codex"),
    )
    .map_err(host)?;
    let folder = cwd.to_string_lossy().into_owned();
    let raw=process::converse(&args,cwd,&BTreeMap::new(),&[json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"ai-infrastructure-harness","version":"1"}}})],Limits{seconds:Some(10),output:PROBE_BYTES,line:PROBE_BYTES},Arc::new(AtomicBool::new(false)),Some(lock),move|message|{if message["id"]==1{if message.get("error").is_some(){return Err(infra_host::HostError::unavailable("Codex refused the skills request."));}return Ok(Control::Write(vec![json!({"jsonrpc":"2.0","method":"initialized"}),json!({"jsonrpc":"2.0","id":2,"method":"skills/list","params":{"cwds":[folder]}})]));}
    if message["id"]==2{if message.get("error").is_some()||!message["result"]["data"].is_array(){return Err(infra_host::HostError::unavailable("Codex did not list its skills."));}return Ok(Control::Done(json!(message["result"]["data"].as_array().into_iter().flatten().flat_map(|e|e["skills"].as_array().into_iter().flatten().cloned()).collect::<Vec<_>>())));}Ok(Control::Continue)}).await.map_err(host)?;
    let mut skills = vec![];
    let mut seen = BTreeSet::new();
    for item in raw.as_array().into_iter().flatten() {
        let (Some(name), Some(path)) = (item["name"].as_str(), item["path"].as_str()) else {
            continue;
        };
        if !skill_name(name)
            || path.chars().count() > 4096
            || item["enabled"] == false
            || !seen.insert(name.to_owned())
        {
            continue;
        }
        skills.push(json!({"name":name,"description":short(&item["description"],300),"path":path,"scope":item["scope"].as_str().unwrap_or("")}));
        if skills.len() == 1000 {
            break;
        }
    }
    Ok(skills)
}
fn basic_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
fn skill_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 200
        && s.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
}
fn short(value: &V, limit: usize) -> String {
    let text = value
        .as_str()
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if text.chars().count() <= limit {
        text
    } else {
        format!(
            "{}…",
            text.chars()
                .take(limit.saturating_sub(1))
                .collect::<String>()
                .trim()
        )
    }
}
pub fn frontmatter(text: &str) -> V {
    let (mut head, mut body) = ("", text);
    if let Some(stripped) = text.strip_prefix("---")
        && let Some(end) = stripped.find("\n---")
    {
        head = &stripped[..end];
        body = stripped[end + 1..]
            .find('\n')
            .map(|n| &stripped[end + 2 + n..])
            .unwrap_or("");
    }
    let field = |key: &str| {
        let lines = head.lines().collect::<Vec<_>>();
        let mut found = None;
        for (i, line) in lines.iter().enumerate() {
            if let Some(value) = line.strip_prefix(&format!("{key}:")) {
                let mut text = value.trim().to_owned();
                for next in lines.iter().skip(i + 1) {
                    if !next.starts_with([' ', '\t']) {
                        break;
                    }
                    text.push(' ');
                    text.push_str(next.trim());
                }
                let mut value = text.trim();
                if let Some(v) = value.strip_prefix(['|', '>']) {
                    value = v.trim_start_matches(['-', '+']).trim();
                }
                if value.len() >= 2
                    && ((value.starts_with('"') && value.ends_with('"'))
                        || (value.starts_with('\'') && value.ends_with('\'')))
                {
                    value = &value[1..value.len() - 1];
                }
                found = Some(value.split_whitespace().collect::<Vec<_>>().join(" "));
                break;
            }
        }
        found
    };
    let name = field("name");
    let description = field("description").unwrap_or_default();
    let hint = field("argument-hint").unwrap_or_default();
    let disable = field("disable-model-invocation")
        .map(|s| s.to_lowercase())
        .as_deref()
        == Some("true");
    let user = field("user-invocable").map(|s| s.to_lowercase()).as_deref() != Some("false");
    json!({"name":name,"description":short(&json!(description),300),"hint":short(&json!(hint),200),"model":!disable,"user":user,"body":body})
}
pub fn workspace_skills(root: &Path, provider: &str) -> Result<(Vec<V>, bool)> {
    let base = match provider {
        "claude" => ".claude/skills",
        "codex" => ".agents/skills",
        "cursor" => ".cursor/skills",
        _ => return Ok((vec![], false)),
    };
    let mut files = PublicationFiles::new(root)?;
    let mut folders = match files.archive_entries(base, 20000) {
        Ok(v) => v,
        Err(e) if e.kind == infra_core::ErrorKind::NotFound => return Ok((vec![], false)),
        Err(e) => return Err(e.into()),
    };
    folders.sort();
    let mut skills = vec![];
    let mut truncated = false;
    for folder in folders {
        if !basic_name(&folder) {
            continue;
        }
        if skills.len() == 200 {
            truncated = true;
            break;
        }
        let path = format!("{base}/{folder}/SKILL.md");
        let raw = match files.read_prefix(&path, HEAD) {
            Ok(Some((_, b))) => b,
            _ => continue,
        };
        let meta = frontmatter(&String::from_utf8_lossy(&raw));
        let name = meta["name"]
            .as_str()
            .filter(|s| basic_name(s))
            .unwrap_or(&folder);
        skills.push(json!({"name":name,"path":path,"description":meta["description"],"hint":meta["hint"],"model":meta["model"],"user":meta["user"]}));
    }
    Ok((skills, truncated))
}
pub fn codex_prompts() -> Result<Vec<V>> {
    let folder = infra_host::model_catalog::codex_home().join("prompts");
    let mut files = match PublicationFiles::new(&folder) {
        Ok(v) => v,
        Err(_) => return Ok(vec![]),
    };
    let mut names = files.archive_entries("", 20000)?;
    names.sort();
    let mut prompts = vec![];
    for name in names {
        let Some(stem) = name
            .strip_suffix(".md")
            .filter(|s| !s.starts_with('.') && basic_name(s))
        else {
            continue;
        };
        let Some((size, raw)) = files.read_prefix(&name, PROMPT_BYTES)? else {
            continue;
        };
        if size > PROMPT_BYTES as u64 {
            continue;
        }
        let meta = frontmatter(&String::from_utf8_lossy(&raw));
        prompts.push(json!({"name":format!("prompts:{stem}"),"description":meta["description"],"hint":meta["hint"],"body":meta["body"]}));
        if prompts.len() == 200 {
            break;
        }
    }
    Ok(prompts)
}
pub fn expand_prompt(name: &str, body: &str, args: &str) -> Result<String> {
    let words = infra_host::command::split_posix(args)
        .map_err(|_| HarnessError::invalid(format!("Close the quotes in /{name} arguments.")))?;
    let named_pattern = regex::Regex::new(r"^[A-Z][A-Z0-9_]*=")
        .map_err(|_| HarnessError::invalid("Invalid internal argument expression."))?;
    let mut named = BTreeMap::new();
    let mut positional = vec![];
    for word in words {
        if named_pattern.is_match(&word) {
            if let Some((key, value)) = word.split_once('=') {
                named.insert(key.to_owned(), value.to_owned());
            }
        } else {
            positional.push(word);
        }
    }
    let token = regex::Regex::new(r"\$\$|\$[1-9]|\$[A-Z][A-Z0-9_]*")
        .map_err(|_| HarnessError::invalid("Invalid internal prompt expression."))?;
    let mut missing = BTreeSet::new();
    for m in token.find_iter(body) {
        let key = &m.as_str()[1..];
        if m.as_str() != "$$"
            && key != "ARGUMENTS"
            && !key.bytes().all(|b| b.is_ascii_digit())
            && !named.contains_key(key)
        {
            missing.insert(key.to_owned());
        }
    }
    if !missing.is_empty() {
        return Err(HarnessError::invalid(format!(
            "/{name} needs {}.",
            missing
                .iter()
                .map(|s| format!("{s}=…"))
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    Ok(token
        .replace_all(body, |captures: &regex::Captures| {
            let raw = &captures[0];
            let key = &raw[1..];
            if raw == "$$" {
                "$".into()
            } else if key == "ARGUMENTS" {
                positional.join(" ")
            } else if let Ok(n) = key.parse::<usize>() {
                positional.get(n - 1).cloned().unwrap_or_default()
            } else {
                named.get(key).cloned().unwrap_or_else(|| raw.into())
            }
        })
        .trim()
        .to_owned())
}
pub fn request(skills: &[V]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    format!(
        "\n\nHarness skill request: the user invoked these skills for this message. Load each one before you act and follow it for this request:\n{}",
        skills
            .iter()
            .map(|s| format!(
                "- {} ({}): read that file and follow it.",
                s["name"].as_str().unwrap_or(""),
                s["path"].as_str().unwrap_or("")
            ))
            .collect::<Vec<_>>()
            .join("\n")
    )
}
fn leading(message: &str) -> Option<(&str, &str)> {
    let tail = message.strip_prefix('/')?;
    let end = tail.find(char::is_whitespace).unwrap_or(tail.len());
    if end == 0 {
        return None;
    }
    Some((&tail[..end], tail[end..].trim()))
}
fn claude_page(name: &str) -> Option<&'static str> {
    match name {
        "clear" | "reset" | "new" => Some("new"),
        "model" => Some("model"),
        "effort" => Some("effort"),
        _ => None,
    }
}
fn aliases(commands: &[V]) -> BTreeMap<String, String> {
    let mut names = BTreeMap::new();
    for item in commands {
        let Some(name) = item["name"].as_str() else {
            continue;
        };
        names.insert(name.to_owned(), name.to_owned());
        for alias in item["aliases"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(V::as_str)
        {
            names.insert(alias.into(), name.into());
        }
    }
    names
}
fn navigation(commands: &mut Vec<V>) {
    let taken = aliases(commands);
    for (name, description, action) in [
        (
            "status",
            "Show this session's usage: tokens, cost and time",
            "status",
        ),
        ("cost", "Show this session's tokens and cost", "status"),
        ("diff", "Show this session's changes", "diff"),
        ("resume", "Open an earlier session from the list", "resume"),
        (
            "memory",
            "Open this project's memory in Knowledge",
            "memory",
        ),
        (
            "login",
            "Say whether the CLI is signed in, and how to sign it in",
            "login",
        ),
        ("help", "List the commands available here", "help"),
    ] {
        if !taken.contains_key(name) {
            commands.push(json!({"name":name,"description":description,"hint":"","action":action,"aliases":[],"kind":"page"}));
        }
    }
}
fn join_notice(a: Option<String>, b: Option<String>) -> Option<String> {
    let parts = [a, b]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}
fn strip_punctuation(value: &str) -> String {
    let mut value = value;
    while let Some(c) = value.chars().last() {
        if !".:-".contains(c) {
            break;
        }
        value = &value[..value.len() - c.len_utf8()];
    }
    value.into()
}
fn mentions(message: &str) -> Vec<String> {
    let chars = message.char_indices().collect::<Vec<_>>();
    let mut at = 0;
    let mut result = vec![];
    while at < chars.len() {
        let (index, ch) = chars[at];
        if ch == '\u{60}' {
            let triple = message[index..].starts_with("\u{60}\u{60}\u{60}");
            let marker = if triple {
                "\u{60}\u{60}\u{60}"
            } else {
                "\u{60}"
            };
            let begin = index + marker.len();
            if let Some(end) = message[begin..]
                .find(marker)
                .filter(|end| triple || !message[begin..begin + *end].contains('\n'))
            {
                let end = begin + end + marker.len();
                while at < chars.len() && chars[at].0 < end {
                    at += 1;
                }
                continue;
            }
            if triple {
                break;
            }
            at += 1;
            continue;
        }
        if ch != '$'
            || index > 0
                && !message[..index]
                    .chars()
                    .last()
                    .is_some_and(char::is_whitespace)
        {
            at += 1;
            continue;
        }
        let begin = index + 1;
        let end = message[begin..]
            .char_indices()
            .find(|(_, c)| !c.is_ascii_alphanumeric() && !"._:-".contains(*c))
            .map(|(i, _)| begin + i)
            .unwrap_or(message.len());
        let name = &message[begin..end];
        let next = message[end..].chars().next();
        let ending =
            next.is_none() || next.is_some_and(|c| c.is_whitespace() || ",;!?)]}".contains(c));
        let rest = message[end..].trim_start();
        if skill_name(name)
            && ending
            && !rest.starts_with('=')
            && !rest.starts_with("->")
            && !rest.starts_with("::")
        {
            result.push(name.into());
        }
        while at < chars.len() && chars[at].0 < end {
            at += 1;
        }
        if end == begin {
            at += 1;
        }
    }
    result
}

impl Domain for CommandCatalog {
    fn execute<'a>(&'a self, operation: &'a str, request: V) -> Reply<'a> {
        Box::pin(async move {
            let value = match operation {
                "commands.project" => {
                    let project_id = request["params"]["project_id"]
                        .as_str()
                        .ok_or_else(|| HarnessError::invalid("Missing project."))?;
                    let project = self.store.project(project_id)?;
                    let provider = request["query"]["provider"]
                        .as_str()
                        .ok_or_else(|| HarnessError::invalid("Choose a provider."))?;
                    let cwd =
                        if let Some(key) = request["query"]["worktree"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                        {
                            crate::projects::resolve_existing(&self.store, &project, key)?
                        } else {
                            PathBuf::from(project["path"].as_str().ok_or_else(|| {
                                HarnessError::invalid("Project folder unavailable.")
                            })?)
                        };
                    PublicationFiles::new(&cwd)?;
                    let mut value = self.listing(provider, &cwd, None, Some(project_id)).await?;
                    value["project_id"] = json!(project_id);
                    value
                }
                "commands.session" => {
                    let sid = request["params"]["session_id"]
                        .as_str()
                        .ok_or_else(|| HarnessError::invalid("Missing session."))?;
                    let session = self.store.get(sid)?;
                    let provider = session["provider"]
                        .as_str()
                        .ok_or_else(|| HarnessError::invalid("Missing provider."))?;
                    if session["workflow"] != "native"
                        || [
                            "fleet",
                            "clash",
                            "creator",
                            "system_run",
                            "system_discovery",
                        ]
                        .iter()
                        .any(|k| !session[*k].is_null() && session[*k] != false)
                    {
                        json!({"session_id":sid,"provider":provider,"commands":[],"skills":[],"error":null})
                    } else {
                        let cwd = PathBuf::from(
                            session["project_path"]
                                .as_str()
                                .ok_or_else(|| HarnessError::invalid("Missing workspace."))?,
                        );
                        let canonical = std::fs::canonicalize(&cwd).map_err(|_| {
                            HarnessError::unavailable("Session workspace unavailable.")
                        })?;
                        if canonical != cwd {
                            return Err(HarnessError::forbidden("Session workspace changed."));
                        }
                        PublicationFiles::new(&cwd)?;
                        let settings = claude_settings(
                            session["agents_enabled"].as_bool().unwrap_or(false),
                            session["agent_count"].as_u64().unwrap_or(3) as u32,
                            session["thinking_effort"].as_str(),
                        )?;
                        let mut value = self
                            .listing(
                                provider,
                                &cwd,
                                Some(&settings),
                                session["project_id"].as_str(),
                            )
                            .await?;
                        value["session_id"] = json!(sid);
                        value
                    }
                }
                _ => return Err(HarnessError::missing("Unknown command operation.")),
            };
            Ok(Payload::Json(value))
        })
    }
}
