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
    collections::{BTreeMap, BTreeSet, HashMap},
    future::Future,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, RwLock, atomic::AtomicBool},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
const HEAD: usize = 16000;
const PROMPT_BYTES: usize = 64 * 1024;
const PROBE_BYTES: usize = 4 * 1024 * 1024;
const COMMAND_RACY_NS: i128 = 2_000_000_000;
const MAX_STAMP_ENTRIES: usize = 1024;
const MAX_DIR_CACHE_ENTRIES: usize = 64;
const INIT_PROMPT: &str = "Create an AGENTS.md file in the current directory: a short contributor guide for this repository titled \"Repository Guidelines\". If AGENTS.md already exists, leave it unchanged and say so. Describe how this repository actually works: project structure, build, test and development commands, coding style and naming, testing, and commit and pull request conventions. Keep it to 200-400 words.";
#[derive(Clone)]
struct Entry {
    stamp: Option<String>,
    loaded: Instant,
    value: Vec<V>,
    error: Option<String>,
}
#[derive(Default)]
struct Cache {
    entries: BTreeMap<String, Entry>,
    locks: BTreeMap<String, Arc<tokio::sync::Mutex<()>>>,
}
#[derive(Clone)]
struct CachedList {
    stamp: String,
    items: Vec<V>,
    truncated: bool,
}
#[derive(Default)]
struct DirListCache {
    seq: u64,
    entries: HashMap<String, (CachedList, u64)>,
}
impl DirListCache {
    fn get(&mut self, key: &str, stamp: &str) -> Option<(Vec<V>, bool)> {
        self.seq = self.seq.saturating_add(1);
        let use_seq = self.seq;
        let entry = self.entries.get_mut(key)?;
        if entry.0.stamp != stamp {
            return None;
        }
        entry.1 = use_seq;
        Some((entry.0.items.clone(), entry.0.truncated))
    }
    fn insert(&mut self, key: String, stamp: String, items: Vec<V>, truncated: bool) {
        self.seq = self.seq.saturating_add(1);
        let use_seq = self.seq;
        if self.entries.len() >= MAX_DIR_CACHE_ENTRIES
            && !self.entries.contains_key(&key)
            && let Some(evict) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, s))| *s)
                .map(|(k, _)| k.clone())
        {
            self.entries.remove(&evict);
        }
        self.entries.insert(
            key,
            (
                CachedList {
                    stamp,
                    items,
                    truncated,
                },
                use_seq,
            ),
        );
    }
}
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
fn cache() -> &'static Mutex<Cache> {
    CACHE.get_or_init(|| Mutex::new(Cache::default()))
}
fn dir_list_cache() -> &'static Mutex<DirListCache> {
    static DIR_CACHE: OnceLock<Mutex<DirListCache>> = OnceLock::new();
    DIR_CACHE.get_or_init(|| Mutex::new(DirListCache::default()))
}
pub fn invalidate() {
    if let Ok(mut c) = cache().lock() {
        c.entries.clear();
    }
    if let Ok(mut d) = dir_list_cache().lock() {
        d.entries.clear();
    }
}
fn cmd_time_ns(t: SystemTime) -> Option<i128> {
    t.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i128::try_from(d.as_nanos()).ok())
}
fn is_racy_mtime(mtime_ns: i128, now_ns: i128) -> bool {
    mtime_ns <= 0 || now_ns <= COMMAND_RACY_NS || (now_ns - mtime_ns).abs() < COMMAND_RACY_NS
}
fn collect_path_stamp_into(
    path: &Path,
    now_ns: i128,
    follow_symlinks: bool,
    out: &mut Vec<String>,
) -> bool {
    let read_meta = |p: &Path| {
        if follow_symlinks {
            std::fs::metadata(p)
        } else {
            std::fs::symlink_metadata(p)
        }
    };
    let meta = match read_meta(path) {
        Ok(m) => m,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let parent_mtime = path
                .parent()
                .and_then(|p| read_meta(p).ok())
                .and_then(|m| m.modified().ok())
                .and_then(cmd_time_ns)
                .unwrap_or(0);
            out.push(format!(
                "missing:{}:pm={parent_mtime}",
                path.to_string_lossy()
            ));
            return true;
        }
        Err(_) => return false,
    };
    if !follow_symlinks && meta.file_type().is_symlink() {
        return false;
    }
    if meta.is_file() {
        let Some(mtime_ns) = meta.modified().ok().and_then(cmd_time_ns) else {
            return false;
        };
        let mut rec = format!("file:{}:{}:{mtime_ns}", path.to_string_lossy(), meta.len());
        if is_racy_mtime(mtime_ns, now_ns) {
            if meta.len() > 256 * 1024 {
                return false;
            }
            let Ok(bytes) = std::fs::read(path) else {
                return false;
            };
            rec.push(':');
            rec.push_str(&crate::store::sha(&bytes));
        }
        out.push(rec);
        return out.len() <= MAX_STAMP_ENTRIES;
    }
    if !meta.is_dir() {
        return false;
    }
    let mut visited_dirs = BTreeSet::new();
    let mut queue = std::collections::VecDeque::from([path.to_path_buf()]);
    while let Some(dir) = queue.pop_front() {
        if follow_symlinks {
            let Ok(canon) = infra_core::fs::canonicalize(&dir) else {
                continue;
            };
            if !visited_dirs.insert(canon) {
                continue;
            }
        }
        let Ok(d_meta) = read_meta(&dir) else {
            return false;
        };
        let d_mtime = d_meta.modified().ok().and_then(cmd_time_ns).unwrap_or(0);
        out.push(format!("dir:{}:{d_mtime}", dir.to_string_lossy()));
        if out.len() > MAX_STAMP_ENTRIES {
            return false;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return false;
        };
        let mut children: Vec<_> = rd.filter_map(|e| e.ok()).collect();
        children.sort_by_key(|e| e.file_name());
        for child in children {
            let name = child.file_name();
            if matches!(
                name.to_string_lossy().as_ref(),
                ".git" | "node_modules" | "vendor" | "target"
            ) {
                continue;
            }
            let p = child.path();
            let c_meta = match read_meta(&p) {
                Ok(m) => m,
                Err(err) if follow_symlinks && err.kind() == std::io::ErrorKind::NotFound => {
                    out.push(format!("dangling:{}", p.to_string_lossy()));
                    continue;
                }
                Err(_) => return false,
            };
            if !follow_symlinks && c_meta.file_type().is_symlink() {
                return false;
            }
            if c_meta.is_dir() {
                queue.push_back(p);
            } else if c_meta.is_file() {
                let Some(mtime_ns) = c_meta.modified().ok().and_then(cmd_time_ns) else {
                    return false;
                };
                let mut rec = format!("file:{}:{}:{mtime_ns}", p.to_string_lossy(), c_meta.len());
                if is_racy_mtime(mtime_ns, now_ns) {
                    if c_meta.len() > 256 * 1024 {
                        return false;
                    }
                    let Ok(bytes) = std::fs::read(&p) else {
                        return false;
                    };
                    rec.push(':');
                    rec.push_str(&crate::store::sha(&bytes));
                }
                out.push(rec);
                if out.len() > MAX_STAMP_ENTRIES {
                    return false;
                }
            }
        }
    }
    true
}
fn stamp_probe_paths(
    executable: Option<&Path>,
    dirs: &[PathBuf],
    now_ns: i128,
    follow_dir_symlinks: bool,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(exe) = executable
        && !collect_path_stamp_into(exe, now_ns, true, &mut parts)
    {
        return None;
    }
    for dir in dirs {
        if !collect_path_stamp_into(dir, now_ns, follow_dir_symlinks, &mut parts) {
            return None;
        }
    }
    Some(crate::store::sha(parts.join("\n").as_bytes()))
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
    fn claude_probe_dirs(cwd: &Path, extra: &[String]) -> Vec<PathBuf> {
        let mut dirs = vec![
            cwd.join(".claude/commands"),
            cwd.join(".claude/skills"),
            cwd.join(".claude/agents"),
            cwd.join(".claude/settings.json"),
            cwd.join(".claude/settings.local.json"),
        ];
        if let Some(home) = crate::run_activity::user_home().map(PathBuf::from) {
            dirs.push(home.join(".claude/commands"));
            dirs.push(home.join(".claude/skills"));
        }
        let mut i = 0;
        while i < extra.len() {
            if extra[i] == "--add-dir"
                && let Some(dir) = extra.get(i + 1)
            {
                let base = PathBuf::from(dir);
                dirs.push(base.join(".claude/commands"));
                dirs.push(base.join(".claude/skills"));
                i += 2;
                continue;
            }
            i += 1;
        }
        dirs
    }
    fn codex_probe_dirs(cwd: &Path) -> Vec<PathBuf> {
        let mut dirs = vec![cwd.join(".agents/skills"), cwd.join(".codex/skills")];
        let codex_home = infra_host::model_catalog::codex_home();
        dirs.push(codex_home.join("skills"));
        dirs.push(codex_home.join("prompts"));
        dirs
    }
    async fn cached<F, Fut>(
        &self,
        key: String,
        executable: Option<&Path>,
        probe_dirs: Vec<PathBuf>,
        newer: Option<Duration>,
        load: F,
    ) -> Result<(Vec<V>, Option<String>)>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<V>>>,
    {
        Box::pin(async move {
            let current_stamp = || {
                let now_ns = cmd_time_ns(SystemTime::now()).unwrap_or(0);
                stamp_probe_paths(executable, &probe_dirs, now_ns, true)
            };
            let usable = |e: &Entry, stamp: &Option<String>| {
                if !newer.map(|age| e.loaded.elapsed() < age).unwrap_or(true) {
                    return false;
                }
                if e.error.is_some() {
                    return e.loaded.elapsed() < Duration::from_secs(10)
                        && (stamp.is_none() || &e.stamp == stamp);
                }
                stamp.is_some() && &e.stamp == stamp
            };
            let gate = {
                let stamp = current_stamp();
                let mut c = cache()
                    .lock()
                    .map_err(|_| HarnessError::unavailable("Command cache unavailable."))?;
                if let Some(entry) = c.entries.get(&key).filter(|e| usable(e, &stamp)) {
                    return Ok((entry.value.clone(), entry.error.clone()));
                }
                c.locks
                    .entry(key.clone())
                    .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                    .clone()
            };
            let _guard = gate.lock().await;
            {
                let stamp = current_stamp();
                let c = cache()
                    .lock()
                    .map_err(|_| HarnessError::unavailable("Command cache unavailable."))?;
                if let Some(entry) = c.entries.get(&key).filter(|e| usable(e, &stamp)) {
                    return Ok((entry.value.clone(), entry.error.clone()));
                }
            }
            let result = Box::pin(load()).await;
            let stored_stamp = current_stamp();
            let (value, error) = match result {
                Ok(v) => (v, None),
                Err(e) => (vec![], Some(e.message)),
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
                    stamp: stored_stamp,
                    loaded: Instant::now(),
                    value: value.clone(),
                    error: error.clone(),
                },
            );
            Ok((value, error))
        })
        .await
    }
    async fn claude(
        &self,
        cwd: &Path,
        settings: &str,
        extra: &[String],
        newer: Option<Duration>,
    ) -> Result<(Vec<V>, Option<String>)> {
        Box::pin(async move {
            let executable = match self.executable("claude") {
                Ok(e) => e,
                Err(e) => return Ok((vec![], Some(e.message))),
            };
            let key = json!(["claude", executable, cwd, settings, extra]).to_string();
            let dirs = Self::claude_probe_dirs(cwd, extra);
            Box::pin(self.cached(key, Some(&executable), dirs, newer, || {
                claude_commands(&executable, cwd, settings, extra, self.store.runner_lock())
            }))
            .await
        })
        .await
    }
    async fn codex(&self, cwd: &Path) -> Result<(Vec<V>, Option<String>)> {
        Box::pin(async move {
            let executable = match self.executable("codex") {
                Ok(e) => e,
                Err(e) => return Ok((vec![], Some(e.message))),
            };
            let key = json!(["codex", executable, cwd]).to_string();
            let dirs = Self::codex_probe_dirs(cwd);
            Box::pin(self.cached(key, Some(&executable), dirs, None, || {
                codex_skills(&executable, cwd, self.store.runner_lock())
            }))
            .await
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
        Box::pin(async move {
            let home = self.attached(project, cwd)?;
            if provider == "claude" {
                let settings = settings
                    .map(str::to_owned)
                    .unwrap_or(claude_settings(false, 3, None)?);
                let extra = home
                    .as_ref()
                    .map(|h| vec!["--add-dir".into(), h.to_string_lossy().into_owned()])
                    .unwrap_or_default();
                return Box::pin(self.claude(cwd, &settings, &extra, newer)).await;
            }
            let (mut listed, error) = match provider {
                "codex" => {
                    let (skills, error) = Box::pin(self.codex(cwd)).await?;
                    (skills.into_iter().map(|v|json!({"name":v["name"],"description":v["description"],"path":v["path"]})).collect::<Vec<_>>(),error)
                }
                "cursor" => (user_skills(cwd, provider)?, None),
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
        })
        .await
    }
    pub async fn listing(
        &self,
        provider: &str,
        cwd: &Path,
        settings: Option<&str>,
        project: Option<&str>,
    ) -> Result<V> {
        let (native, error) =
            Box::pin(self.catalogue(provider, cwd, settings, project, None)).await?;
        let(mut commands,skills)=match provider{
   "claude"=>(native.into_iter().map(|mut item|{let action=claude_page(item["name"].as_str().unwrap_or(""));item["kind"]=json!(if action.is_some(){"page"}else{"native"});item["action"]=action.map(|s|json!(s)).unwrap_or(V::Null);item}).collect::<Vec<_>>(),claude_skills(cwd)?),
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
        Box::pin(async move {
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
                    let (mut commands, mut error) =
                        Box::pin(self.catalogue(provider, &cwd, Some(&settings), project, None))
                            .await?;
                    let mut aliases = aliases(&commands);
                    if !aliases.contains_key(name) && error.is_none() {
                        let fresh = Box::pin(self.catalogue(
                            provider,
                            &cwd,
                            Some(&settings),
                            project,
                            Some(Duration::from_secs(5)),
                        ))
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
                // A `$skill` mention is routed only for a session that opted into
                // SkillCache: an ordinary Claude session's message goes as typed
                // and its prompt stays what it was.
                let mentioned = if crate::skillcache::settings::mode(session) == "off" {
                    Vec::new()
                } else {
                    mentions(message)
                };
                if !mentioned.is_empty() {
                    let skills = claude_skills(&cwd)?;
                    resolve_mentions(&mentioned, &skills, &mut route);
                    let names = route
                        .requests
                        .iter()
                        .filter_map(|r| r["name"].as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    if !names.is_empty() {
                        route.notice = join_notice(
                            route.notice,
                            Some(format!("Claude skills requested: {names}.")),
                        );
                    }
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
                    let (skills, error) =
                        Box::pin(self.catalogue(provider, &cwd, None, project, None)).await?;
                    resolve_mentions(&mentioned, &skills, &mut route);
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
                let mentioned = if crate::skillcache::settings::mode(session) == "off" {
                    Vec::new()
                } else {
                    mentions(message)
                };
                if lead.is_some() || !mentioned.is_empty() {
                    let (skills, _) =
                        Box::pin(self.catalogue(provider, &cwd, None, project, None)).await?;
                    if let Some((name, _)) = lead
                        && let Some(skill) = skills.iter().find(|s| s["name"] == name)
                    {
                        route.requests.push(skill.clone());
                    }
                    if !mentioned.is_empty() {
                        resolve_mentions(&mentioned, &skills, &mut route);
                    }
                    let names = route
                        .requests
                        .iter()
                        .filter_map(|r| r["name"].as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    if !names.is_empty() {
                        route.notice = join_notice(
                            route.notice,
                            Some(format!("Cursor skills requested: {names}.")),
                        );
                    }
                }
            } else {
                return Err(HarnessError::invalid("Choose a provider."));
            }
            Ok(route)
        })
        .await
    }
    /// Lists the skills routable for `provider` in `cwd` without executing a
    /// live slash-command probe for Claude.
    pub async fn provider_skills(
        &self,
        provider: &str,
        cwd: &Path,
        project: Option<&str>,
    ) -> Result<(Vec<V>, Option<String>)> {
        if provider == "claude" {
            return Ok((claude_skills(cwd)?, None));
        }
        self.catalogue(provider, cwd, None, project, None).await
    }
    /// Resolves an explicit `skillcache.skill` hint when no skill was already
    /// routed from the prompt text and the prompt is not a native slash command.
    pub async fn apply_skill_hint(&self, session: &V, route: &mut ResolvedCommand) -> Result<()> {
        if route.mode == "native" || !route.requests.is_empty() {
            return Ok(());
        }
        let Some(hint) = crate::skillcache::settings::skill_hint(session) else {
            return Ok(());
        };
        let provider = session["provider"].as_str().unwrap_or("");
        if !["claude", "codex", "cursor"].contains(&provider) {
            return Ok(());
        }
        let cwd = PathBuf::from(session["project_path"].as_str().unwrap_or(""));
        if !cwd.is_dir() {
            return Ok(());
        }
        let (skills, _) = self
            .provider_skills(provider, &cwd, session["project_id"].as_str())
            .await?;
        if let Some(skill) = match_skill(&skills, hint) {
            route.requests.push(skill);
        }
        Ok(())
    }
}

pub fn match_skill(skills: &[V], hint: &str) -> Option<V> {
    let hint = hint.trim();
    if hint.is_empty() {
        return None;
    }
    skills
        .iter()
        .find(|skill| skill["name"].as_str() == Some(hint) || skill["path"].as_str() == Some(hint))
        .cloned()
}

fn claude_skills(cwd: &Path) -> Result<Vec<V>> {
    user_skills(cwd, "claude")
}

/// Lists user-invocable skills for `provider` from the workspace and user home.
fn user_skills(cwd: &Path, provider: &str) -> Result<Vec<V>> {
    let home = crate::run_activity::user_home().map(PathBuf::from);
    user_skills_in(cwd, home.as_deref(), provider)
}

/// Lists user-invocable skills for `provider` across `cwd` and an optional `home` directory.
fn user_skills_in(cwd: &Path, home: Option<&Path>, provider: &str) -> Result<Vec<V>> {
    let mut listed: Vec<V> = workspace_skills(cwd, provider)?
        .0
        .into_iter()
        .filter(|s| s["user"] == true)
        .collect();
    let mut names = listed
        .iter()
        .filter_map(|v| v["name"].as_str().map(str::to_owned))
        .collect::<BTreeSet<_>>();
    if let Some(home) = home
        && home.is_dir()
        && let Ok((home_skills, _)) = workspace_skills(home, provider)
    {
        for mut skill in home_skills {
            if skill["user"] != true {
                continue;
            }
            if let Some(path) = skill["path"].as_str() {
                skill["path"] = json!(home.join(path));
            }
            if skill["name"]
                .as_str()
                .is_some_and(|name| names.insert(name.to_owned()))
            {
                listed.push(skill);
            }
        }
    }
    Ok(listed)
}

fn resolve_mentions(mentioned: &[String], skills: &[V], route: &mut ResolvedCommand) {
    for name in mentioned {
        let name = if skills.iter().any(|s| s["name"] == *name) {
            name.clone()
        } else {
            strip_punctuation(name)
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
    Box::pin(async move {
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
        let raw=Box::pin(process::converse_environment(&args,cwd,&BTreeMap::new(),&["CONTEXT_CAPSULE_DELIVERED".into()],&[json!({"type":"control_request","request_id":"harness-commands","request":{"subtype":"initialize"}})],Limits{seconds:Some(10),output:PROBE_BYTES,line:PROBE_BYTES},Arc::new(AtomicBool::new(false)),Some(lock),|message|{if message["type"]!="control_response"||message["response"]["request_id"]!="harness-commands"{return Ok(Control::Continue);}let response=&message["response"];if response["subtype"]!="success"||!response["response"]["commands"].is_array(){return Err(infra_host::HostError::unavailable("Claude Code did not list its commands."));}Ok(Control::Done(response["response"]["commands"].clone()))})).await.map_err(host)?;
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
    })
    .await
}
async fn codex_skills(executable: &Path, cwd: &Path, lock: Arc<std::fs::File>) -> Result<Vec<V>> {
    Box::pin(async move {
        let args = provider::command_argv(
            &[
                executable.to_string_lossy().into_owned(),
                "app-server".into(),
            ],
            Some("codex"),
        )
        .map_err(host)?;
        let folder = cwd.to_string_lossy().into_owned();
        let raw=Box::pin(process::converse(&args,cwd,&BTreeMap::new(),&[json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"ai-infrastructure-harness","version":"1"}}})],Limits{seconds:Some(10),output:PROBE_BYTES,line:PROBE_BYTES},Arc::new(AtomicBool::new(false)),Some(lock),move|message|{if message["id"]==1{if message.get("error").is_some(){return Err(infra_host::HostError::unavailable("Codex refused the skills request."));}return Ok(Control::Write(vec![json!({"jsonrpc":"2.0","method":"initialized"}),json!({"jsonrpc":"2.0","id":2,"method":"skills/list","params":{"cwds":[folder]}})]));}
        if message["id"]==2{if message.get("error").is_some()||!message["result"]["data"].is_array(){return Err(infra_host::HostError::unavailable("Codex did not list its skills."));}return Ok(Control::Done(json!(message["result"]["data"].as_array().into_iter().flatten().flat_map(|e|e["skills"].as_array().into_iter().flatten().cloned()).collect::<Vec<_>>())));}Ok(Control::Continue)})).await.map_err(host)?;
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
    })
    .await
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
    let root_canon = infra_core::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let skill_root = root_canon.join(base);
    let cache_key = format!("ws:{provider}:{}", skill_root.to_string_lossy());
    let now_ns = cmd_time_ns(SystemTime::now()).unwrap_or(0);
    let stamp = stamp_probe_paths(None, std::slice::from_ref(&skill_root), now_ns, false);
    if let Some(ref s) = stamp
        && let Ok(mut guard) = dir_list_cache().lock()
        && let Some(cached) = guard.get(&cache_key, s)
    {
        return Ok(cached);
    }
    let mut files = PublicationFiles::new(root)?;
    let mut folders = match files.archive_entries(base, 20000) {
        Ok(v) => v,
        Err(e) if e.kind == infra_core::ErrorKind::NotFound => {
            if let Some(s) = stamp
                && let Ok(mut guard) = dir_list_cache().lock()
            {
                guard.insert(cache_key, s, vec![], false);
            }
            return Ok((vec![], false));
        }
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
    if let Some(s) = stamp
        && let Ok(mut guard) = dir_list_cache().lock()
    {
        guard.insert(cache_key, s, skills.clone(), truncated);
    }
    Ok((skills, truncated))
}
pub fn codex_prompts() -> Result<Vec<V>> {
    let folder = infra_host::model_catalog::codex_home().join("prompts");
    let cache_key = format!("prompts:{}", folder.to_string_lossy());
    let now_ns = cmd_time_ns(SystemTime::now()).unwrap_or(0);
    let stamp = stamp_probe_paths(None, std::slice::from_ref(&folder), now_ns, false);
    if let Some(ref s) = stamp
        && let Ok(mut guard) = dir_list_cache().lock()
        && let Some((cached, _)) = guard.get(&cache_key, s)
    {
        return Ok(cached);
    }
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
    if let Some(s) = stamp
        && let Ok(mut guard) = dir_list_cache().lock()
    {
        guard.insert(cache_key, s, prompts.clone(), false);
    }
    Ok(prompts)
}
pub fn expand_prompt(name: &str, body: &str, args: &str) -> Result<String> {
    static NAMED_PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    static TOKEN_PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    let words = infra_host::command::split_posix(args)
        .map_err(|_| HarnessError::invalid(format!("Close the quotes in /{name} arguments.")))?;
    let named_pattern = NAMED_PATTERN.get_or_init(|| {
        regex::Regex::new(r"^[A-Z][A-Z0-9_]*=").expect("valid named prompt arg regex")
    });
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
    let token = TOKEN_PATTERN.get_or_init(|| {
        regex::Regex::new(r"\$\$|\$[1-9]|\$[A-Z][A-Z0-9_]*").expect("valid prompt token regex")
    });
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
                        let canonical = infra_core::fs::canonicalize(&cwd).map_err(|_| {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog_in(root: &Path) -> CommandCatalog {
        let store = SessionStore::open(&root.join("state")).unwrap();
        let source = root.join("source");
        std::fs::create_dir_all(&source).unwrap();
        CommandCatalog::new(store, source, Arc::new(RwLock::new(BTreeMap::new())))
    }

    #[test]
    fn claude_skills_reads_frontmatter_names_and_hides_non_user_skills() {
        let root = tempfile::tempdir().unwrap();
        let dir_named = root.path().join(".claude/skills/sc-test-alpha-dir");
        let custom_named = root.path().join(".claude/skills/sc-test-custom-folder");
        let hidden = root.path().join(".claude/skills/sc-test-hidden");
        std::fs::create_dir_all(&dir_named).unwrap();
        std::fs::create_dir_all(&custom_named).unwrap();
        std::fs::create_dir_all(&hidden).unwrap();
        std::fs::write(
            dir_named.join("SKILL.md"),
            "---\ndescription: Alpha skill\n---\n# Alpha\n",
        )
        .unwrap();
        std::fs::write(
            custom_named.join("SKILL.md"),
            "---\nname: sc-test-beta\ndescription: Beta skill\n---\n# Beta\n",
        )
        .unwrap();
        std::fs::write(
            hidden.join("SKILL.md"),
            "---\nname: sc-test-hidden\nuser-invocable: false\n---\n# Hidden\n",
        )
        .unwrap();

        let listed = claude_skills(root.path()).unwrap();
        assert!(listed.iter().any(|s| {
            s["name"] == "sc-test-alpha-dir"
                && s["path"] == ".claude/skills/sc-test-alpha-dir/SKILL.md"
        }));
        assert!(listed.iter().any(|s| {
            s["name"] == "sc-test-beta"
                && s["path"] == ".claude/skills/sc-test-custom-folder/SKILL.md"
        }));
        assert!(!listed.iter().any(|s| s["name"] == "sc-test-hidden"));
    }

    #[tokio::test]
    async fn claude_routes_dollar_skill_mentions_and_skill_hints() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let skill_dir = workspace.join(".claude/skills/sc-test-pdf");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: sc-test-pdf\ndescription: PDF totals\n---\n# PDF\n",
        )
        .unwrap();
        let catalog = catalog_in(root.path());
        // A session that did not opt into SkillCache sends its message as it
        // was typed: the mention is not routed and the prompt gains nothing.
        let ordinary = json!({
            "provider": "claude",
            "project_path": workspace.display().to_string(),
            "agents_enabled": false,
            "agent_count": 1,
        });
        let untouched = catalog
            .route(&ordinary, "Please run $sc-test-pdf on invoice.pdf")
            .await
            .unwrap();
        assert!(untouched.requests.is_empty());
        assert!(untouched.notice.is_none());
        assert_eq!(untouched.skill_request(), "");
        let mut session = ordinary.clone();
        session["skillcache"] = json!({"mode": "auto"});

        let routed = catalog
            .route(&session, "Please run $sc-test-pdf on invoice.pdf")
            .await
            .unwrap();
        assert_eq!(routed.mode, "text");
        assert_eq!(routed.requests.len(), 1);
        assert_eq!(routed.requests[0]["name"], "sc-test-pdf");
        assert_eq!(
            routed.requests[0]["path"],
            ".claude/skills/sc-test-pdf/SKILL.md"
        );

        let mut hinted = catalog
            .route(&session, "Please summarize invoice.pdf")
            .await
            .unwrap();
        assert!(hinted.requests.is_empty());
        let with_hint = json!({
            "provider": "claude",
            "project_path": workspace.display().to_string(),
            "skillcache": {"mode": "warm_only", "skill": "sc-test-pdf"},
        });
        catalog
            .apply_skill_hint(&with_hint, &mut hinted)
            .await
            .unwrap();
        assert_eq!(hinted.requests.len(), 1);
        assert_eq!(hinted.requests[0]["name"], "sc-test-pdf");
    }

    #[tokio::test]
    async fn cursor_routes_dollar_mentions_skill_hints_and_home_skills() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let home = root.path().join("home");
        let ws_skill = workspace.join(".cursor/skills/sc-ws-pdf");
        let home_skill = home.join(".cursor/skills/sc-home-csv");
        std::fs::create_dir_all(&ws_skill).unwrap();
        std::fs::create_dir_all(&home_skill).unwrap();
        std::fs::write(
            ws_skill.join("SKILL.md"),
            "---\nname: sc-ws-pdf\ndescription: Workspace Cursor skill\n---\n# PDF\n",
        )
        .unwrap();
        std::fs::write(
            home_skill.join("SKILL.md"),
            "---\nname: sc-home-csv\ndescription: Home Cursor skill\n---\n# CSV\n",
        )
        .unwrap();

        let home_listed = user_skills_in(&workspace, Some(&home), "cursor").unwrap();
        assert!(home_listed.iter().any(|s| s["name"] == "sc-ws-pdf"));
        assert!(home_listed.iter().any(|s| {
            s["name"] == "sc-home-csv"
                && s["path"]
                    == home
                        .join(".cursor/skills/sc-home-csv/SKILL.md")
                        .to_string_lossy()
                        .as_ref()
        }));

        let catalog = catalog_in(root.path());
        let ordinary = json!({
            "provider": "cursor",
            "project_path": workspace.display().to_string(),
        });
        let untouched = catalog
            .route(&ordinary, "Run $sc-ws-pdf here")
            .await
            .unwrap();
        assert!(untouched.requests.is_empty());

        let mut session = ordinary.clone();
        session["skillcache"] = json!({"mode": "auto"});
        let routed = catalog
            .route(&session, "Run $sc-ws-pdf here")
            .await
            .unwrap();
        assert_eq!(routed.mode, "text");
        assert_eq!(routed.requests.len(), 1);
        assert_eq!(routed.requests[0]["name"], "sc-ws-pdf");
        assert_eq!(
            routed.requests[0]["path"],
            ".cursor/skills/sc-ws-pdf/SKILL.md"
        );

        let mut hinted = catalog
            .route(&session, "Process these files")
            .await
            .unwrap();
        assert!(hinted.requests.is_empty());
        let with_hint = json!({
            "provider": "cursor",
            "project_path": workspace.display().to_string(),
            "skillcache": {"mode": "warm_only", "skill": "sc-ws-pdf"},
        });
        catalog
            .apply_skill_hint(&with_hint, &mut hinted)
            .await
            .unwrap();
        assert_eq!(hinted.requests.len(), 1);
        assert_eq!(hinted.requests[0]["name"], "sc-ws-pdf");
    }

    #[tokio::test]
    async fn command_catalog_cached_uses_settled_racy_ns_stamps_without_wall_clock_ttl() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        let skill_dir = ws.join(".claude/skills/alpha");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let skill_md = skill_dir.join("SKILL.md");
        std::fs::write(&skill_md, "---\nname: alpha\ndescription: v1\n---\n").unwrap();
        let fake_cli = dir.path().join("fake-cli");
        std::fs::write(&fake_cli, "#!/bin/sh\necho ok\n").unwrap();

        let catalog = catalog_in(dir.path());
        let key = format!("test-stamp-{}", ws.display());
        let dirs = vec![
            ws.join(".claude/commands"),
            ws.join(".claude/skills"),
            ws.join(".claude/agents"),
        ];
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let load_once = |n: &str| {
            let calls = calls.clone();
            let v = n.to_owned();
            async move {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(vec![json!({"name": v})])
            }
        };

        let (v1, err1) = catalog
            .cached(key.clone(), Some(&fake_cli), dirs.clone(), None, || {
                load_once("cmd-v1")
            })
            .await
            .unwrap();
        assert!(err1.is_none());
        assert_eq!(v1, vec![json!({"name": "cmd-v1"})]);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

        // Even if loaded was > 60s ago, unchanged RACY_NS stamp MUST keep serving cached value.
        if let Ok(mut guard) = cache().lock()
            && let Some(entry) = guard.entries.get_mut(&key)
        {
            entry.loaded = Instant::now() - Duration::from_secs(600);
        }
        let (v2, _) = catalog
            .cached(key.clone(), Some(&fake_cli), dirs.clone(), None, || {
                load_once("cmd-v2")
            })
            .await
            .unwrap();
        assert_eq!(v2, vec![json!({"name": "cmd-v1"})]);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

        // Same-second overwrite of watched SKILL.md inside RACY_NS invalidates immediately.
        std::fs::write(&skill_md, "---\nname: alpha\ndescription: v2\n---\n").unwrap();
        let (v3, _) = catalog
            .cached(key.clone(), Some(&fake_cli), dirs.clone(), None, || {
                load_once("cmd-v3")
            })
            .await
            .unwrap();
        assert_eq!(v3, vec![json!({"name": "cmd-v3"})]);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);

        // Explicit `newer` override forces refresh when requested.
        if let Ok(mut guard) = cache().lock()
            && let Some(entry) = guard.entries.get_mut(&key)
        {
            entry.loaded = Instant::now() - Duration::from_secs(10);
        }
        let (v4, _) = catalog
            .cached(
                key,
                Some(&fake_cli),
                dirs,
                Some(Duration::from_secs(5)),
                || load_once("cmd-v4"),
            )
            .await
            .unwrap();
        assert_eq!(v4, vec![json!({"name": "cmd-v4"})]);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[test]
    fn workspace_skills_and_expand_prompt_cache_and_invalidate_same_second_edits() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join(".claude/skills/racy-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let md = skill_dir.join("SKILL.md");
        std::fs::write(&md, "---\nname: racy-skill\ndescription: First\n---\n").unwrap();

        let (s1, _) = workspace_skills(dir.path(), "claude").unwrap();
        assert_eq!(s1[0]["description"], "First");

        // Same-second overwrite with same length invalidates via SHA-256 stamp.
        std::fs::write(&md, "---\nname: racy-skill\ndescription: Secnd\n---\n").unwrap();
        let (s2, _) = workspace_skills(dir.path(), "claude").unwrap();
        assert_eq!(s2[0]["description"], "Secnd");

        let expanded = expand_prompt("p", "Run $TARGET with $1", "TARGET=auth unit").unwrap();
        assert_eq!(expanded, "Run auth with unit");
    }
}
