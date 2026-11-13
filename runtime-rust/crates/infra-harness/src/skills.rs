//! Native skill discovery and one-use reviewed filesystem publication.
use crate::{
    HarnessError, Result,
    services::{Domain, Payload, Reply},
    store::SessionStore,
};
use infra_core::{self as core, bank::publication::PublicationFiles};
use serde::{Deserialize, Serialize};
use serde_json::{Value as V, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;
const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_FILES: usize = 5000;
const TTL: Duration = Duration::from_secs(900);
const AGENTS: &[(&str, &str, &str)] = &[
    ("claude", ".claude/skills", "Claude Code"),
    ("codex", ".agents/skills", "Codex"),
    ("cursor", ".cursor/skills", "Cursor"),
];
type Namespace = (u64, u64, u32);
type Identity = (u64, u64, u32, u64, u64, i128, i128);
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Fingerprint {
    hash: String,
    mode: u32,
}
#[derive(Clone)]
struct Image {
    body: Vec<u8>,
    fingerprint: Fingerprint,
    identity: Identity,
}
#[derive(Clone)]
struct Asset {
    body: Vec<u8>,
    mode: u32,
}
#[derive(Clone)]
struct Skill {
    title: String,
    description: String,
    source: String,
    revision: String,
    files: BTreeMap<String, Asset>,
}
#[derive(Clone)]
struct Preview {
    created: Instant,
    project_id: String,
    root: PathBuf,
    identity: Namespace,
    directories: BTreeMap<String, Namespace>,
    _source_id: String,
    operation: String,
    path: Option<String>,
    before: BTreeMap<String, Image>,
    payloads: BTreeMap<String, Asset>,
    records_before: Option<Vec<u8>>,
    records_after: V,
    can_apply: bool,
}
#[derive(Default)]
struct State {
    loaded: BTreeMap<String, BTreeMap<String, Skill>>,
    previews: VecDeque<(String, Preview)>,
}
#[derive(Clone)]
pub struct SkillsDomain {
    pub store: SessionStore,
    pub source: PathBuf,
    sources: Arc<BTreeMap<String, V>>,
    state: Arc<Mutex<State>>,
    operation: Arc<Mutex<()>>,
}
impl SkillsDomain {
    pub fn new(store: SessionStore, source: PathBuf) -> Result<Self> {
        let mut files = PublicationFiles::new(&source)?;
        let raw = files
            .read("install/open-source-kit/resources.json", 4 * 1024 * 1024)?
            .ok_or_else(|| HarnessError::missing("The skill source catalog is unavailable."))?;
        let catalog = core::results::strict_json(&raw)?;
        let mut sources = BTreeMap::new();
        for item in catalog["resources"].as_array().into_iter().flatten() {
            if item["skills_source"].is_string() {
                let id = required(item, "id", 200)?.to_owned();
                repository(required(item, "skills_source", 300)?)?;
                if sources.insert(id, item.clone()).is_some() {
                    return Err(HarnessError::invalid(
                        "The skill source catalog contains duplicate IDs.",
                    ));
                }
            }
        }
        Ok(Self {
            store,
            source,
            sources: Arc::new(sources),
            state: Arc::new(Mutex::new(State::default())),
            operation: Arc::new(Mutex::new(())),
        })
    }
    fn state(&self) -> Result<std::sync::MutexGuard<'_, State>> {
        self.state
            .lock()
            .map_err(|_| HarnessError::unavailable("Skill state is unavailable."))
    }
    fn catalog(&self) -> V {
        json!({"sources":self.sources.values().map(|item| json!({"id":item["id"],"name":item["name"],"description":item["description"],"url":item["url"]})).collect::<Vec<_>>(),"agents":AGENTS.iter().map(|(id,_,name)|json!({"id":id,"name":name})).collect::<Vec<_>>(),"available":true,"detail":"Load an immutable source revision to choose individual skills."})
    }
    fn project(&self, id: &str) -> Result<(V, PathBuf)> {
        let project = self.store.project(id)?;
        let root = PathBuf::from(required(&project, "path", 32768)?);
        if !root.is_absolute() {
            return Err(HarnessError::forbidden(
                "The registered project path is unsafe.",
            ));
        }
        PublicationFiles::new(&root)?;
        Ok((project, root))
    }
    fn record_path(project: &V) -> String {
        format!(
            "skills-{}.json",
            core::query::sha256(project["id"].as_str().unwrap_or("").as_bytes())
        )
    }
    fn records(&self, project: &V) -> Result<(Option<Vec<u8>>, V)> {
        let mut state = PublicationFiles::new(&self.store.state)?;
        let raw = state.read(&Self::record_path(project), 4 * 1024 * 1024)?;
        let value = if let Some(raw) = raw.as_ref() {
            core::results::strict_json(raw)?
        } else {
            json!({"root":project["path"],"skills":{}})
        };
        if value["root"] != project["path"]
            || !value["skills"].is_object()
            || value["skills"].as_object().map_or(0, |o| o.len()) > 1500
        {
            return Err(HarnessError::invalid(
                "Skill tracking metadata is invalid. Restore runner state before managing skills.",
            ));
        }
        for (path, record) in value["skills"].as_object().expect("object") {
            let agent = required(record, "agent", 50)?;
            let name = required(record, "name", 100)?;
            valid_name(name, false)?;
            let base = agent_base(agent)?;
            if path != &format!("{base}/{name}")
                || !record["files"].is_object()
                || record["files"].as_object().map_or(0, |o| o.len()) == 0
                || record["files"].as_object().map_or(0, |o| o.len()) > MAX_FILES
                || !record["source_id"].is_string()
                || !record["source"].is_string()
                || record["revision"].as_str().is_none_or(|s| s.len() > 128)
                || record["identity"].as_array().map_or(0, Vec::len) != 2
            {
                return Err(HarnessError::invalid("Skill tracking metadata is invalid."));
            }
            for (relative, fp) in record["files"].as_object().expect("object") {
                safe(relative)?;
                if !hex(fp["hash"].as_str().unwrap_or(""), 64)
                    || !fp["mode"].as_u64().is_some_and(|n| n <= 0o777)
                {
                    return Err(HarnessError::invalid(
                        "Skill tracking fingerprints are invalid.",
                    ));
                }
            }
        }
        Ok((raw, value))
    }
    fn save_records(&self, project: &V, value: &V, expected: Option<&[u8]>) -> Result<()> {
        let body = serde_json::to_vec(value)
            .map_err(|_| HarnessError::invalid("Invalid skill tracking JSON."))?;
        if body.len() > 4 * 1024 * 1024 {
            return Err(HarnessError::invalid("Skill tracking exceeds4MiB."));
        }
        let mut files = PublicationFiles::new(&self.store.state)?;
        files.write_expected_mode(
            &Self::record_path(project),
            &body,
            expected,
            0o600,
            4 * 1024 * 1024,
        )?;
        Ok(())
    }
    fn load(&self, source_id: &str) -> Result<BTreeMap<String, Skill>> {
        let catalog = self
            .sources
            .get(source_id)
            .ok_or_else(|| HarnessError::invalid("Select a supported skill source."))?;
        let source = repository(required(catalog, "skills_source", 300)?)?;
        let client = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .user_agent("AI-Infrastructure-Harness")
            .build()
            .map_err(|_| {
                HarnessError::unavailable("Could not initialize the skill source client.")
            })?;
        let url = format!("https://api.github.com/repos/{source}/commits/HEAD");
        let mut response = client
            .get(&url)
            .header("Accept", "application/vnd.github.sha")
            .send()
            .map_err(|_| {
                HarnessError::unavailable("Could not resolve the skill source revision.")
            })?;
        if !response.status().is_success() || response.url().as_str() != url {
            return Err(HarnessError::unavailable(
                "Unexpected skill source response or redirect.",
            ));
        }
        let mut revision = String::new();
        response
            .by_ref()
            .take(129)
            .read_to_string(&mut revision)
            .map_err(|_| HarnessError::unavailable("Invalid skill source revision response."))?;
        let revision = revision.trim().to_owned();
        if !hex(&revision, 40) {
            return Err(HarnessError::invalid(
                "The source did not return a valid immutable commit.",
            ));
        }
        let url = format!("https://codeload.github.com/{source}/tar.gz/{revision}");
        let mut response = client.get(&url).send().map_err(|_| {
            HarnessError::unavailable("Could not download the skill source archive.")
        })?;
        if !response.status().is_success() || response.url().as_str() != url {
            return Err(HarnessError::unavailable(
                "Unexpected skill archive response or redirect.",
            ));
        }
        let mut compressed = vec![];
        response
            .by_ref()
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut compressed)
            .map_err(|_| HarnessError::unavailable("Could not read the skill archive."))?;
        if compressed.len() > 16 * 1024 * 1024 {
            return Err(HarnessError::invalid("The skill archive exceeds16MiB."));
        }
        let archive_limit = MAX_BYTES + MAX_FILES * 2048;
        let mut expanded = vec![];
        flate2::read::GzDecoder::new(Cursor::new(compressed))
            .take(archive_limit as u64 + 1)
            .read_to_end(&mut expanded)
            .map_err(|_| HarnessError::invalid("Invalid skill archive compression."))?;
        if expanded.len() > archive_limit {
            return Err(HarnessError::invalid(
                "The expanded skill archive exceeds its limit.",
            ));
        }
        let mut archive = tar::Archive::new(Cursor::new(expanded));
        let mut members = BTreeMap::<String, Asset>::new();
        let mut links = vec![];
        let (mut count, mut total) = (0, 0usize);
        for member in archive
            .entries()
            .map_err(|_| HarnessError::invalid("Invalid skill archive."))?
        {
            let mut entry =
                member.map_err(|_| HarnessError::invalid("Invalid skill archive member."))?;
            count += 1;
            let name = repeated_suffix_removed(
                entry
                    .path()
                    .map_err(|_| HarnessError::invalid("Invalid archive path."))?
                    .to_str()
                    .ok_or_else(|| HarnessError::invalid("Non-UTF8 archive path."))?,
                "/",
            )
            .to_owned();
            safe(&name)?;
            let size = usize::try_from(entry.size())
                .map_err(|_| HarnessError::invalid("Skill archive size overflow."))?;
            total = total
                .checked_add(size)
                .ok_or_else(|| HarnessError::invalid("Skill archive size overflow."))?;
            if count > MAX_FILES || total > MAX_BYTES {
                return Err(HarnessError::invalid(
                    "The skill archive exceeds file or byte bounds.",
                ));
            }
            let kind = entry.header().entry_type();
            if kind.is_symlink() || kind.is_hard_link() {
                links.push(name);
                continue;
            }
            if kind.is_dir() {
                continue;
            }
            if !kind.is_file() {
                return Err(HarnessError::invalid(
                    "The skill archive contains unsupported file types.",
                ));
            }
            let mut body = vec![];
            entry
                .by_ref()
                .take(size as u64 + 1)
                .read_to_end(&mut body)
                .map_err(|_| HarnessError::invalid("Could not read skill archive member."))?;
            if body.len() != size {
                return Err(HarnessError::invalid(
                    "Skill archive member length changed.",
                ));
            }
            let mode = if entry.header().mode().unwrap_or(0) & 0o111 != 0 {
                0o755
            } else {
                0o644
            };
            if members.insert(name, Asset { body, mode }).is_some() {
                return Err(HarnessError::invalid("Duplicate skill archive path."));
            }
        }
        let roots = members
            .keys()
            .filter_map(|path| path.strip_suffix("/SKILL.md").map(str::to_owned))
            .collect::<Vec<_>>();
        if roots.is_empty() {
            return Err(HarnessError::missing(
                "No SKILL.md files were found in this source.",
            ));
        }
        if links
            .iter()
            .any(|link| roots.iter().any(|r| link.starts_with(&format!("{r}/"))))
        {
            return Err(HarnessError::forbidden(
                "Links inside skill directories are unsupported.",
            ));
        }
        let mut skills = BTreeMap::new();
        for root in roots {
            let raw = &members[&format!("{root}/SKILL.md")].body;
            let text = std::str::from_utf8(raw)
                .map_err(|_| HarnessError::invalid("Skill metadata must be UTF8."))?;
            let front = text
                .strip_prefix("---\n")
                .or_else(|| text.strip_prefix("---\r\n"))
                .and_then(|tail| tail.split_once("\n---").map(|(v, _)| v))
                .ok_or_else(|| HarnessError::invalid("Skill metadata frontmatter is missing."))?;
            let metadata: V = serde_yaml::from_str(front)
                .map_err(|_| HarnessError::invalid("Skill metadata is invalid."))?;
            let name = metadata["name"]
                .as_str()
                .unwrap_or_else(|| root.rsplit('/').next().unwrap_or(""));
            valid_name(name, false)?;
            let description = metadata["description"]
                .as_str()
                .unwrap_or("")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(1200)
                .collect();
            let prefix = format!("{root}/");
            let files = members
                .iter()
                .filter_map(|(p, a)| p.strip_prefix(&prefix).map(|r| (r.to_owned(), a.clone())))
                .collect::<BTreeMap<_, _>>();
            if skills
                .insert(
                    name.to_owned(),
                    Skill {
                        title: name.to_owned(),
                        description,
                        source: source.into(),
                        revision: revision.clone(),
                        files,
                    },
                )
                .is_some()
            {
                return Err(HarnessError::invalid("Source skills have duplicate names."));
            }
            if skills.len() > 500 {
                return Err(HarnessError::invalid("Source contains too many skills."));
            }
        }
        Ok(skills)
    }
    fn discover(&self, source_id: &str, refresh: bool) -> Result<V> {
        let _operation = self
            .operation
            .try_lock()
            .map_err(|_| HarnessError::conflict("Another skill operation is in progress."))?;
        let needs = refresh || !self.state()?.loaded.contains_key(source_id);
        if needs {
            let skills = self.load(source_id)?;
            self.state()?.loaded.insert(source_id.into(), skills);
        }
        let state = self.state()?;
        let skills = state
            .loaded
            .get(source_id)
            .ok_or_else(|| HarnessError::missing("The source was not loaded."))?;
        Ok(
            json!({"source_id":source_id,"skills":skills.iter().map(|(id,s)|json!({"id":id,"name":s.title,"description":s.description})).collect::<Vec<_>>(),"truncated":false,"summary":format!("{} skills loaded at an immutable source revision.",skills.len())}),
        )
    }
    fn tree(
        root: &Path,
        path: &str,
        files: &mut PublicationFiles,
    ) -> Result<BTreeMap<String, Image>> {
        safe(path)?;
        let mut result = BTreeMap::new();
        let mut queue = VecDeque::from([path.to_owned()]);
        let mut dirs = 0usize;
        let mut total = 0usize;
        while let Some(current) = queue.pop_front() {
            dirs += 1;
            if dirs > MAX_FILES || current.matches('/').count() > 64 {
                return Err(HarnessError::invalid(
                    "The skill contains too many directories.",
                ));
            }
            files.read_prefix(&format!("{current}/.custody-probe"), 0)?;
            let absolute = root.join(&current);
            if !core::fs::admit_directory(&absolute)? {
                continue;
            }
            for entry in std::fs::read_dir(&absolute)
                .map_err(|_| HarnessError::unavailable("Skill directory unavailable."))?
            {
                let entry =
                    entry.map_err(|_| HarnessError::conflict("Skill directory changed."))?;
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| HarnessError::invalid("Unsupported skill filename."))?;
                let full = format!("{current}/{name}");
                safe(&full)?;
                let meta = std::fs::symlink_metadata(root.join(&full))
                    .map_err(|_| HarnessError::conflict("Skill path changed."))?;
                if core::fs::linked(&meta) {
                    return Err(HarnessError::forbidden("Skill paths must not be links."));
                }
                if meta.is_dir() {
                    queue.push_back(full);
                    continue;
                }
                let identity = files
                    .image_identity(&full)?
                    .ok_or_else(|| HarnessError::conflict("Skill file changed."))?;
                let body = files
                    .read(&full, MAX_BYTES)?
                    .ok_or_else(|| HarnessError::conflict("Skill file disappeared."))?;
                total += body.len();
                if result.len() >= MAX_FILES || total > MAX_BYTES {
                    return Err(HarnessError::invalid(
                        "The skill exceeds file or byte limits.",
                    ));
                }
                let mode = files
                    .read_mode(&full)?
                    .ok_or_else(|| HarnessError::conflict("Skill mode is unavailable."))?;
                let hash = core::query::sha256(&body);
                let relative = full
                    .strip_prefix(&format!("{path}/"))
                    .ok_or_else(|| HarnessError::invalid("Skill path is outside its root."))?
                    .to_owned();
                result.insert(
                    relative,
                    Image {
                        body,
                        fingerprint: Fingerprint {
                            hash,
                            mode: mode & 0o777,
                        },
                        identity,
                    },
                );
            }
            files.check()?;
        }
        Ok(result)
    }
    fn installed(&self, project_id: &str) -> Result<V> {
        let (project, root) = self.project(project_id)?;
        let (_, records) = self.records(&project)?;
        let mut files = PublicationFiles::new(&root)?;
        let identity = files.root_identity();
        let mut entries = BTreeMap::new();
        for (agent, base, _) in AGENTS {
            let folder = root.join(base);
            if !core::fs::admit_directory(&folder)? {
                continue;
            }
            files.read_prefix(&format!("{base}/.custody-probe"), 0)?;
            let mut names = std::fs::read_dir(&folder)
                .map_err(|_| HarnessError::unavailable("Skill directory unavailable."))?
                .filter_map(|e| e.ok())
                .map(|e| e.file_name())
                .collect::<Vec<_>>();
            names.sort();
            for name in names.into_iter().take(500) {
                let Some(name) = name.to_str() else { continue };
                if valid_name(name, false).is_err() {
                    continue;
                }
                let path = format!("{base}/{name}");
                if files.read_prefix(&format!("{path}/SKILL.md"), 0)?.is_some() {
                    entries.insert(path.clone(),json!({"name":name,"agent":agent,"path":path,"managed":false,"state":"untracked","can_update":false,"can_remove":false}));
                }
            }
        }
        for (path, record) in records["skills"].as_object().expect("validated tracking") {
            let status = match Self::tree(&root, path, &mut files) {
                Ok(tree) => {
                    if identity_json(identity) == record["identity"]
                        && fingerprints(&tree) == record["files"]
                    {
                        "clean"
                    } else {
                        "modified"
                    }
                }
                Err(_) => "unsafe",
            };
            let source_id = required(record, "source_id", 300)?;
            entries.insert(path.clone(),json!({"name":record["name"],"agent":record["agent"],"path":path,"managed":true,"source_id":source_id,"source":record["source"],"revision":record["revision"],"state":status,"can_update":status=="clean"&&self.sources.contains_key(source_id),"can_remove":status=="clean"}));
        }
        files.check()?;
        Ok(json!({"project_id":project_id,"installed":entries.into_values().collect::<Vec<_>>()}))
    }
    fn preview(&self, data: &V) -> Result<V> {
        exact(data, &["project_id", "source_id", "skills", "agents"])?;
        let project_id = required(data, "project_id", 200)?;
        let source_id = required(data, "source_id", 300)?;
        let skills = selection(&data["skills"], 500, "skills")?;
        let agents = selection(&data["agents"], 3, "agents")?;
        let state = self.state()?;
        let source = state.loaded.get(source_id).ok_or_else(|| {
            HarnessError::invalid("Load this source before previewing installation.")
        })?;
        let mut payloads = BTreeMap::new();
        let mut definitions = BTreeMap::new();
        for agent in agents {
            let base = agent_base(&agent)?;
            for name in &skills {
                let skill = source
                    .get(name)
                    .ok_or_else(|| HarnessError::invalid("Select a loaded skill."))?;
                definitions.insert(format!("{base}/{name}"),json!({"name":name,"agent":agent,"source_id":source_id,"source":skill.source,"revision":skill.revision,"files":asset_fingerprints(&skill.files)}));
                for (relative, asset) in &skill.files {
                    payloads.insert(format!("{base}/{name}/{relative}"), asset.clone());
                }
            }
        }
        drop(state);
        self.install_preview(project_id, source_id, payloads, definitions)
    }
    fn create_preview(&self, data: &V) -> Result<V> {
        exact(
            data,
            &[
                "project_id",
                "agents",
                "name",
                "description",
                "instructions",
            ],
        )?;
        let project = required(data, "project_id", 200)?;
        let name = required(data, "name", 64)?.trim();
        valid_name(name, true)?;
        let description = required(data, "description", 1024)?;
        if description.contains(['<', '>']) {
            return Err(HarnessError::invalid(
                "Skill descriptions must not contain angle brackets.",
            ));
        }
        let summary = description.split_whitespace().collect::<Vec<_>>().join(" ");
        let instructions = required(data, "instructions", 32000)?
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .trim()
            .to_owned();
        if instructions.len() > 32000 {
            return Err(HarnessError::invalid(
                "Skill instructions exceed32000 UTF8 bytes.",
            ));
        }
        for s in [name, description, &instructions] {
            if s.chars().any(|c|matches!(c,'\u{0}'..='\u{8}'|'\u{b}'|'\u{c}'|'\u{e}'..='\u{1f}'|'\u{7f}'..='\u{9f}'|'\u{fffe}'|'\u{ffff}')){return Err(HarnessError::invalid("Skill text contains unsupported characters."));}
        }
        let content = format!(
            "---\nname: {}\ndescription: {}\n---\n\n{}\n",
            serde_json::to_string(name)
                .map_err(|_| HarnessError::invalid("Invalid skill name."))?,
            serde_json::to_string(&summary)
                .map_err(|_| HarnessError::invalid("Invalid skill description."))?,
            instructions
        );
        let mut payloads = BTreeMap::new();
        let mut definitions = BTreeMap::new();
        for agent in selection(&data["agents"], 3, "tools")? {
            let base = agent_base(&agent)?;
            let prefix = format!("{base}/{name}");
            let asset = Asset {
                body: content.as_bytes().to_vec(),
                mode: 0o644,
            };
            definitions.insert(prefix.clone(),json!({"name":name,"agent":agent,"source_id":"local","source":"local","revision":format!("sha256:{}",core::query::sha256(content.as_bytes())),"files":{"SKILL.md":{"hash":core::query::sha256(content.as_bytes()),"mode":0o644}}}));
            payloads.insert(format!("{prefix}/SKILL.md"), asset);
        }
        let mut response = self.install_preview(project, "local", payloads, definitions)?;
        response["name"] = json!(name);
        response["content"] = json!(content);
        Ok(response)
    }
    fn install_preview(
        &self,
        project_id: &str,
        source_id: &str,
        payloads: BTreeMap<String, Asset>,
        mut definitions: BTreeMap<String, V>,
    ) -> Result<V> {
        if payloads.len() > MAX_FILES
            || payloads.values().map(|a| a.body.len()).sum::<usize>() > MAX_BYTES
        {
            return Err(HarnessError::invalid("Select fewer skills or tools."));
        }
        let (project, root) = self.project(project_id)?;
        let mut files = PublicationFiles::new(&root)?;
        let identity = files.root_identity();
        let (records_before, mut records) = self.records(&project)?;
        let mut before = BTreeMap::new();
        let mut rows = vec![];
        let mut can = true;
        for (path, payload) in &payloads {
            safe(path)?;
            let existing = files.read(path, MAX_BYTES)?;
            let status = if let Some(body) = existing {
                let mode = files
                    .read_mode(path)?
                    .ok_or_else(|| HarnessError::conflict("Skill mode changed."))?;
                let id = files
                    .image_identity(path)?
                    .ok_or_else(|| HarnessError::conflict("Skill identity changed."))?;
                let status = if body == payload.body {
                    "identical"
                } else {
                    "conflict"
                };
                before.insert(
                    path.clone(),
                    Image {
                        fingerprint: Fingerprint {
                            hash: core::query::sha256(&body),
                            mode: mode & 0o777,
                        },
                        body,
                        identity: id,
                    },
                );
                status
            } else {
                "new"
            };
            if status == "conflict" {
                can = false;
            }
            rows.push(json!({"path":path,"status":status}));
        }
        for (path, record) in &mut definitions {
            record["identity"] = identity_json(identity);
            records["skills"][path] = record.clone();
        }
        let directories = files.directory_identities()?;
        let id = Uuid::new_v4().simple().to_string();
        let preview = Preview {
            created: Instant::now(),
            project_id: project_id.into(),
            root,
            identity,
            directories,
            _source_id: source_id.into(),
            operation: "install".into(),
            path: None,
            before,
            payloads,
            records_before,
            records_after: records,
            can_apply: can,
        };
        self.cache(id.clone(), preview)?;
        Ok(
            json!({"preview_id":id,"project_id":project_id,"source_id":source_id,"files":rows,"can_install":can,"expires_at":expires(),"summary":format!("{} files. {}",rows.len(),if can{"Ready to install after review."}else{"Existing files differ or a path is unsafe; resolve conflicts first."})}),
        )
    }
    fn cache(&self, id: String, preview: Preview) -> Result<()> {
        let mut state = self.state()?;
        state.previews.retain(|(_, p)| p.created.elapsed() < TTL);
        state.previews.push_back((id, preview));
        while state.previews.len() > 32
            || state
                .previews
                .iter()
                .map(|(_, p)| p.payloads.values().map(|a| a.body.len()).sum::<usize>())
                .sum::<usize>()
                > MAX_BYTES
        {
            state.previews.pop_front();
        }
        Ok(())
    }
    fn consume(&self, id: &str, operation: &str) -> Result<Preview> {
        let mut state = self.state()?;
        let index = state
            .previews
            .iter()
            .position(|(key, _)| key == id)
            .ok_or_else(|| {
                HarnessError::conflict("The preview expired or was already used. Preview again.")
            })?;
        let (_, preview) = state
            .previews
            .remove(index)
            .ok_or_else(|| HarnessError::conflict("Preview disappeared."))?;
        if preview.created.elapsed() >= TTL
            || !preview.can_apply
            || if operation == "install" {
                preview.operation != "install"
            } else {
                preview.operation == "install"
            }
        {
            return Err(HarnessError::conflict(
                "The preview is blocked, expired or already used.",
            ));
        }
        Ok(preview)
    }
    fn no_active(&self) -> Result<std::sync::MutexGuard<'_, rusqlite::Connection>> {
        let db = self.store.connection()?;
        let active: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE status IN('queued','running'))",
                [],
                |r| r.get(0),
            )
            .map_err(|_| HarnessError::unavailable("Session admission state is unavailable."))?;
        if active {
            return Err(HarnessError::conflict(
                "Wait for active sessions to finish before changing skills.",
            ));
        }
        Ok(db)
    }
    fn apply(&self, id: &str, install: bool) -> Result<V> {
        let preview = self.consume(id, if install { "install" } else { "change" })?;
        let _operation = self
            .operation
            .try_lock()
            .map_err(|_| HarnessError::conflict("Another skill operation is in progress."))?;
        let (project, root) = self.project(&preview.project_id)?;
        let (records_before, mut records) = self.records(&project)?;
        if records_before != preview.records_before || root != preview.root {
            return Err(HarnessError::conflict(
                "The project or tracking records changed. Preview again.",
            ));
        }
        let _admission = self.no_active()?;
        let mut files = PublicationFiles::new(&root)?;
        if files.root_identity() != preview.identity {
            return Err(HarnessError::conflict(
                "The project directory changed. Preview again.",
            ));
        }
        files.check_directory_identities(&preview.directories)?;
        for (path, image) in &preview.before {
            if files.image_identity(path)? != Some(image.identity)
                || files.read_mode(path)?.map(|n| n & 0o777) != Some(image.fingerprint.mode)
                || files.read(path, MAX_BYTES)?.as_deref() != Some(image.body.as_slice())
            {
                return Err(HarnessError::conflict(
                    "Skill files changed after preview. No files were changed.",
                ));
            }
        }
        if let Some(path) = preview.path.as_ref() {
            let tree = Self::tree(&root, path, &mut files)?;
            if !matches_reviewed_tree(path, &tree, &preview.before) {
                return Err(HarnessError::conflict(
                    "Skill content or extra files changed after preview.",
                ));
            }
        }
        for (path, asset) in &preview.payloads {
            if !preview.before.contains_key(path) && files.read(path, MAX_BYTES)?.is_some() {
                return Err(HarnessError::conflict(
                    "A new file appeared after preview. Preview again.",
                ));
            }
            if asset.mode != 0o644 && asset.mode != 0o755 {
                return Err(HarnessError::invalid("Invalid reviewed skill mode."));
            }
        }
        let mut paths = preview
            .before
            .keys()
            .chain(preview.payloads.keys())
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        paths.sort_by(|left, right| {
            (left.ends_with("/SKILL.md"), left).cmp(&(right.ends_with("/SKILL.md"), right))
        });
        let (mut changed, mut unchanged) = (vec![], vec![]);
        let result = (|| -> Result<()> {
            for path in paths {
                files.check_directory_identities(&preview.directories)?;
                let old = preview.before.get(&path);
                let new = preview.payloads.get(&path);
                if let (Some(old), Some(new)) = (old, new)
                    && old.body == new.body
                    && (install || old.fingerprint.mode == new.mode)
                {
                    unchanged.push(path);
                    continue;
                }
                match (old, new) {
                    (Some(old), None) => {
                        files.unlink_expected_limit(&path, &old.body, MAX_BYTES)?;
                    }
                    (_, Some(new)) => {
                        files.write_expected_mode_checked(
                            &path,
                            &new.body,
                            old.map(|v| v.body.as_slice()),
                            old.map(|v| v.fingerprint.mode),
                            new.mode,
                            MAX_BYTES,
                        )?;
                    }
                    _ => continue,
                }
                changed.push(path);
            }
            if install {
                for (path, record) in preview.records_after["skills"]
                    .as_object()
                    .ok_or_else(|| HarnessError::invalid("Invalid reviewed tracking state."))?
                {
                    if records["skills"].get(path) == Some(record) {
                        continue;
                    }
                    let actual = Self::tree(&root, path, &mut files)?;
                    if fingerprints(&actual) == record["files"] {
                        records["skills"][path] = record.clone();
                    }
                }
            } else {
                let path = preview
                    .path
                    .as_ref()
                    .ok_or_else(|| HarnessError::invalid("Change preview has no skill path."))?;
                let actual = Self::tree(&root, path, &mut files)?;
                let prefix = format!("{path}/");
                let expected = preview.payloads.iter().filter_map(|(name, asset)| {
                    name.strip_prefix(&prefix).map(|relative| (relative, asset))
                });
                if fingerprints(&actual) != asset_fingerprints_from(expected) {
                    return Err(HarnessError::conflict(
                        "The resulting skill differs from the reviewed files.",
                    ));
                }
                if preview.operation == "remove" {
                    records["skills"]
                        .as_object_mut()
                        .expect("tracking object")
                        .remove(path);
                } else {
                    records["skills"][path] = preview.records_after["skills"][path].clone();
                }
            }
            files.check()?;
            self.save_records(&project, &records, records_before.as_deref())?;
            Ok(())
        })();
        if !changed.is_empty() || result.is_ok() {
            crate::commands::invalidate();
        }
        if let Err(error) = result {
            if changed.is_empty() {
                return Err(error);
            }
            return Ok(
                json!({"ok":false,"changed":changed,"installed":if install{changed.clone()}else{vec![]},"unchanged":unchanged,"summary":"The operation stopped after some files changed. Tracking was not advanced.","error":"Partial result: preserve these files and inspect the skill locally before retrying."}),
            );
        }
        drop(_admission);
        self.store.set_setting(
            "command-catalog-generation",
            &json!(Uuid::new_v4().to_string()),
        )?;
        if install {
            Ok(
                json!({"ok":true,"installed":changed,"unchanged":unchanged,"summary":"Reviewed files installed. Extra or mismatched files were kept. Start a new agent session to load skills."}),
            )
        } else {
            Ok(
                json!({"ok":true,"changed":changed,"summary":if preview.operation=="remove"{"Skill removed."}else{"Reviewed update applied. Start a new agent session to load it."}}),
            )
        }
    }
    fn change_preview(&self, data: &V) -> Result<V> {
        exact(data, &["project_id", "agent", "name", "operation"])?;
        let operation = required(data, "operation", 20)?;
        if !["update", "remove"].contains(&operation) {
            return Err(HarnessError::invalid("Select update or remove."));
        }
        let project_id = required(data, "project_id", 200)?;
        let agent = required(data, "agent", 50)?;
        let name = required(data, "name", 100)?;
        valid_name(name, false)?;
        let path = format!("{}/{name}", agent_base(agent)?);
        let _operation = self
            .operation
            .try_lock()
            .map_err(|_| HarnessError::conflict("Another skill operation is in progress."))?;
        let (project, root) = self.project(project_id)?;
        let (records_before, mut records_after) = self.records(&project)?;
        let record = records_after["skills"][&path].clone();
        if !record.is_object() {
            return Err(HarnessError::invalid(
                "This skill is untracked. Preview matching source installation first.",
            ));
        }
        let mut files = PublicationFiles::new(&root)?;
        let identity = files.root_identity();
        if record["identity"] != identity_json(identity) {
            return Err(HarnessError::conflict("The project directory changed."));
        }
        let tree = Self::tree(&root, &path, &mut files)?;
        let clean = fingerprints(&tree) == record["files"];
        let mut payloads = BTreeMap::new();
        if operation == "update" {
            let source_id = required(&record, "source_id", 300)?;
            if !self.sources.contains_key(source_id) {
                return Err(HarnessError::invalid(
                    "Locally created skills have no upstream updates.",
                ));
            }
            let loaded = self.load(source_id)?;
            let skill=loaded.get(name).ok_or_else(||HarnessError::missing("The skill no longer exists upstream. Nothing was changed; removal is separate."))?;
            if skill.source != record["source"].as_str().unwrap_or("") {
                return Err(HarnessError::conflict(
                    "The catalog source changed. Review a new installation.",
                ));
            }
            let mut next = record.clone();
            next["revision"] = json!(skill.revision);
            next["files"] = asset_fingerprints(&skill.files);
            records_after["skills"][&path] = next;
            for (relative, asset) in &skill.files {
                payloads.insert(format!("{path}/{relative}"), asset.clone());
            }
            self.state()?.loaded.insert(source_id.into(), loaded);
        } else {
            records_after["skills"]
                .as_object_mut()
                .expect("tracking object")
                .remove(&path);
        }
        let before = tree
            .into_iter()
            .map(|(name, v)| (format!("{path}/{name}"), v))
            .collect::<BTreeMap<_, _>>();
        if before.len() + payloads.len() > MAX_FILES
            || before.values().map(|v| v.body.len()).sum::<usize>()
                + payloads.values().map(|v| v.body.len()).sum::<usize>()
                > MAX_BYTES
        {
            return Err(HarnessError::invalid(
                "The skill exceeds preview size bounds.",
            ));
        }
        let mut rows = vec![];
        let mut diff_budget = 256 * 1024usize;
        for full in before
            .keys()
            .chain(payloads.keys())
            .cloned()
            .collect::<BTreeSet<_>>()
        {
            let old = before.get(&full);
            let new = payloads.get(&full);
            let relative = full.strip_prefix(&format!("{path}/")).unwrap_or("");
            let fingerprint = old.map(|v| json!(v.fingerprint));
            let status = if fingerprint.as_ref() != record["files"].get(relative) {
                "conflict"
            } else if new.is_none() {
                "delete"
            } else if old.is_none() {
                "new"
            } else if old
                .zip(new)
                .is_some_and(|(a, b)| a.body == b.body && a.fingerprint.mode == b.mode)
            {
                "identical"
            } else {
                "update"
            };
            let before_bytes = old.map(|v| v.body.as_slice()).unwrap_or(&[]);
            let after_bytes = new.map(|v| v.body.as_slice()).unwrap_or(&[]);
            let mut diff = unified_diff(&full, before_bytes, after_bytes);
            let truncated = diff.len() > diff_budget;
            if truncated {
                while !diff.is_char_boundary(diff_budget) {
                    diff_budget = diff_budget.saturating_sub(1);
                }
                diff.truncate(diff_budget);
            }
            diff_budget = diff_budget.saturating_sub(diff.len());
            rows.push(json!({"path":full,"status":status,"diff":diff,"diff_truncated":truncated,"before_bytes":before_bytes.len(),"after_bytes":after_bytes.len(),"before_mode":old.map(|v|v.fingerprint.mode),"after_mode":new.map(|v|v.mode)}));
        }
        let id = Uuid::new_v4().simple().to_string();
        let next_revision = records_after["skills"][&path]["revision"].clone();
        let preview = Preview {
            created: Instant::now(),
            project_id: project_id.into(),
            root,
            identity,
            directories: files.directory_identities()?,
            _source_id: record["source_id"].as_str().unwrap_or("").into(),
            operation: operation.into(),
            path: Some(path.clone()),
            before,
            payloads,
            records_before,
            records_after,
            can_apply: clean,
        };
        self.cache(id.clone(), preview)?;
        Ok(
            json!({"preview_id":id,"project_id":project_id,"path":path,"operation":operation,"source":record["source"],"revision":record["revision"],"next_revision":next_revision,"files":rows,"can_apply":clean,"expires_at":expires(),"summary":if clean{"Review the exact file changes before applying."}else{"Local changes or extra files detected. No files will be overwritten."}}),
        )
    }
    fn dispatch(&self, operation: &str, request: V) -> Result<Payload> {
        let body = &request["body"];
        let project = request["params"]["project_id"].as_str().unwrap_or("");
        let result = match operation {
            "skills.catalog" => self.catalog(),
            "skills.installed" => self.installed(project)?,
            "skills.discover" => {
                exact(body, &["source_id", "refresh"])?;
                let refresh = body["refresh"]
                    .as_bool()
                    .ok_or_else(|| HarnessError::invalid("Source refresh must be boolean."))?;
                self.discover(required(body, "source_id", 300)?, refresh)?
            }
            "skills.preview" => self.preview(body)?,
            "skills.create_preview" => self.create_preview(body)?,
            "skills.install" => {
                exact(body, &["preview_id"])?;
                self.apply(required(body, "preview_id", 100)?, true)?
            }
            "skills.change_preview" => self.change_preview(body)?,
            "skills.apply_change" => {
                exact(body, &["preview_id"])?;
                self.apply(required(body, "preview_id", 100)?, false)?
            }
            _ => return Err(HarnessError::missing("Unknown skill operation.")),
        };
        Ok(Payload::Json(result))
    }
}
impl Domain for SkillsDomain {
    fn execute<'a>(&'a self, operation: &'a str, request: V) -> Reply<'a> {
        let own = self.clone();
        let operation = operation.to_owned();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || own.dispatch(&operation, request))
                .await
                .map_err(|_| HarnessError::unavailable("Skill worker stopped."))?
        })
    }
}
fn required<'a>(v: &'a V, key: &str, limit: usize) -> Result<&'a str> {
    let value = v[key]
        .as_str()
        .ok_or_else(|| HarnessError::invalid(format!("Missing {key}.")))?;
    if value.trim().is_empty() || value.chars().count() > limit || value.contains('\0') {
        return Err(HarnessError::invalid(format!("Invalid {key}.")));
    }
    Ok(value)
}
fn exact(v: &V, fields: &[&str]) -> Result<()> {
    if v.as_object()
        .is_none_or(|v| v.len() != fields.len() || fields.iter().any(|k| !v.contains_key(*k)))
    {
        return Err(HarnessError::invalid(
            "Unexpected or missing skill request fields.",
        ));
    }
    Ok(())
}
fn safe(name: &str) -> Result<&str> {
    crate::knowledge::relative(name)
}
fn valid_name(name: &str, local: bool) -> Result<&str> {
    let expression = if local {
        r"^[a-z0-9]+(?:-[a-z0-9]+)*$"
    } else {
        r"^[a-zA-Z0-9][a-zA-Z0-9._-]{0,99}$"
    };
    let max = if local { 64 } else { 100 };
    if name.len() > max
        || !regex::Regex::new(expression)
            .map_err(|_| HarnessError::invalid("Invalid internal name rule."))?
            .is_match(name)
    {
        return Err(HarnessError::invalid("Unsupported skill name."));
    }
    Ok(name)
}
fn repository(source: &str) -> Result<&str> {
    if !regex::Regex::new(r"^[A-Za-z0-9_][A-Za-z0-9_.-]*/[A-Za-z0-9_][A-Za-z0-9_.-]*$")
        .map_err(|_| HarnessError::invalid("Invalid internal source rule."))?
        .is_match(source)
    {
        return Err(HarnessError::invalid("Unsupported GitHub skill source."));
    }
    Ok(source)
}
fn agent_base(agent: &str) -> Result<&'static str> {
    AGENTS
        .iter()
        .find(|(id, _, _)| *id == agent)
        .map(|(_, base, _)| *base)
        .ok_or_else(|| HarnessError::invalid("Select a supported tool."))
}
fn selection(v: &V, max: usize, label: &str) -> Result<Vec<String>> {
    let values = v
        .as_array()
        .filter(|v| !v.is_empty() && v.len() <= max)
        .ok_or_else(|| HarnessError::invalid(format!("Select valid {label}.")))?;
    let mut seen = BTreeSet::new();
    for v in values {
        let value = v
            .as_str()
            .ok_or_else(|| HarnessError::invalid("Selections must be text."))?;
        if !seen.insert(value.to_owned()) {
            return Err(HarnessError::invalid("Select distinct values."));
        }
    }
    Ok(seen.into_iter().collect())
}
fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn identity_json(id: Namespace) -> V {
    json!([id.0, id.1])
}
fn fingerprints(tree: &BTreeMap<String, Image>) -> V {
    json!(
        tree.iter()
            .map(|(k, v)| (k.clone(), v.fingerprint.clone()))
            .collect::<BTreeMap<_, _>>()
    )
}
fn matches_reviewed_tree(
    prefix: &str,
    tree: &BTreeMap<String, Image>,
    before: &BTreeMap<String, Image>,
) -> bool {
    tree.len() == before.len()
        && tree.iter().all(|(name, image)| {
            before
                .get(&format!("{prefix}/{name}"))
                .is_some_and(|before| {
                    before.identity == image.identity && before.fingerprint == image.fingerprint
                })
        })
}
fn asset_fingerprints(tree: &BTreeMap<String, Asset>) -> V {
    asset_fingerprints_from(tree.iter().map(|(name, asset)| (name.as_str(), asset)))
}
fn asset_fingerprints_from<'a>(assets: impl IntoIterator<Item = (&'a str, &'a Asset)>) -> V {
    json!(
        assets
            .into_iter()
            .map(|(k, v)| (
                k.to_owned(),
                Fingerprint {
                    hash: core::query::sha256(&v.body),
                    mode: v.mode
                }
            ))
            .collect::<BTreeMap<_, _>>()
    )
}
fn expires() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        + 900
}
fn unified_diff(path: &str, before: &[u8], after: &[u8]) -> String {
    if before == after {
        return String::new();
    }
    let (Ok(before), Ok(after)) = (std::str::from_utf8(before), std::str::from_utf8(after)) else {
        return "Binary content differs.".into();
    };
    let a = before.lines().collect::<Vec<_>>();
    let b = after.lines().collect::<Vec<_>>();
    let mut prefix = 0;
    while prefix < a.len().min(b.len()) && a[prefix] == b[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix
        < a.len()
            .saturating_sub(prefix)
            .min(b.len().saturating_sub(prefix))
        && a[a.len() - 1 - suffix] == b[b.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let start = prefix.saturating_sub(3);
    let end_a = (a.len() - suffix + 3).min(a.len());
    let end_b = (b.len() - suffix + 3).min(b.len());
    let mut out = format!(
        "--- a/{path}\n+++ b/{path}\n@@ -{},{} +{},{} @@\n",
        start + 1,
        end_a - start,
        start + 1,
        end_b - start
    );
    for line in &a[start..prefix] {
        out.push(' ');
        out.push_str(line);
        out.push('\n');
    }
    for line in &a[prefix..a.len() - suffix] {
        out.push('-');
        out.push_str(line);
        out.push('\n');
    }
    for line in &b[prefix..b.len() - suffix] {
        out.push('+');
        out.push_str(line);
        out.push('\n');
    }
    for line in &a[a.len() - suffix..end_a] {
        out.push(' ');
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn repeated_suffix_removed<'a>(mut value: &'a str, suffix: &str) -> &'a str {
    if suffix.is_empty() {
        return value;
    }
    while let Some(rest) = value.strip_suffix(suffix) {
        value = rest;
    }
    value
}

#[cfg(test)]
mod apply_tests {
    use super::*;

    const PREFIX: &str = ".agents/skills/fixture";

    // Frozen pre-optimization comparison: cloning bodies is intentional in
    // this small reference oracle, never in the production comparison.
    fn original_tree_matches(
        prefix: &str,
        tree: &BTreeMap<String, Image>,
        before: &BTreeMap<String, Image>,
    ) -> bool {
        let actual = tree
            .iter()
            .map(|(name, image)| (format!("{prefix}/{name}"), image.clone()))
            .collect::<BTreeMap<_, _>>();
        !(actual.len() != before.len()
            || actual.iter().any(|(name, image)| {
                before.get(name).is_none_or(|before| {
                    before.identity != image.identity || before.fingerprint != image.fingerprint
                })
            }))
    }
    fn original_asset_fingerprints(tree: &BTreeMap<String, Asset>) -> V {
        json!(
            tree.iter()
                .map(|(name, asset)| (
                    name.clone(),
                    Fingerprint {
                        hash: core::query::sha256(&asset.body),
                        mode: asset.mode
                    }
                ))
                .collect::<BTreeMap<_, _>>()
        )
    }
    fn image(body: &[u8], inode: u64) -> Image {
        Image {
            body: body.to_vec(),
            fingerprint: Fingerprint {
                hash: core::query::sha256(body),
                mode: 0o644,
            },
            identity: (1, inode, 0o100644, 1, body.len() as u64, 1, 1),
        }
    }
    #[test]
    fn borrowed_tree_comparison_preserves_identity_and_fingerprint_contract() {
        let tree = BTreeMap::from([
            ("SKILL.md".into(), image(b"instructions", 1)),
            ("assets/\u{fc}.bin".into(), image(&[0, 255, 1], 2)),
        ]);
        let before = tree
            .iter()
            .map(|(name, image)| (format!("{PREFIX}/{name}"), image.clone()))
            .collect::<BTreeMap<_, _>>();
        assert!(matches_reviewed_tree(PREFIX, &tree, &before));
        let mut cases = vec![(tree.clone(), true)];
        let mut missing = tree.clone();
        missing.remove("SKILL.md");
        cases.push((missing, false));
        let mut extra = tree.clone();
        extra.insert("extra.txt".into(), image(b"extra", 3));
        cases.push((extra, false));
        let mut replaced = tree.clone();
        replaced.get_mut("SKILL.md").unwrap().identity.1 += 1;
        cases.push((replaced, false));
        let mut fingerprint = tree.clone();
        fingerprint.get_mut("SKILL.md").unwrap().fingerprint.hash = "different".into();
        cases.push((fingerprint, false));
        let mut mode = tree.clone();
        mode.get_mut("SKILL.md").unwrap().fingerprint.mode = 0o755;
        cases.push((mode, false));
        let mut opaque_body = tree.clone();
        opaque_body.get_mut("SKILL.md").unwrap().body = b"not part of this comparison".to_vec();
        cases.push((opaque_body, true));
        for (actual, expected) in cases {
            assert_eq!(matches_reviewed_tree(PREFIX, &actual, &before), expected);
            assert_eq!(
                matches_reviewed_tree(PREFIX, &actual, &before),
                original_tree_matches(PREFIX, &actual, &before)
            );
        }
        assert!(!matches_reviewed_tree(
            ".agents/skills/other",
            &tree,
            &before
        ));
        assert!(matches_reviewed_tree(
            PREFIX,
            &BTreeMap::new(),
            &BTreeMap::new()
        ));
    }

    #[test]
    fn borrowed_asset_fingerprints_keep_exact_bytes_modes_order_and_prefix_selection() {
        let payloads = BTreeMap::from([
            (
                format!("{PREFIX}/SKILL.md"),
                Asset {
                    body: b"abc".to_vec(),
                    mode: 0o644,
                },
            ),
            (
                format!("{PREFIX}/data/\u{fc}.bin"),
                Asset {
                    body: vec![],
                    mode: 0o755,
                },
            ),
            (
                format!("{PREFIX}-other/SKILL.md"),
                Asset {
                    body: b"excluded".to_vec(),
                    mode: 0o644,
                },
            ),
        ]);
        let prefix = format!("{PREFIX}/");
        let original = payloads
            .iter()
            .filter_map(|(name, asset)| {
                name.strip_prefix(&prefix)
                    .map(|name| (name.to_owned(), asset.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        let expected = json!({
            "SKILL.md":{"hash":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad","mode":420},
            "data/\u{fc}.bin":{"hash":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","mode":493}
        });
        let actual = asset_fingerprints_from(
            payloads
                .iter()
                .filter_map(|(name, asset)| name.strip_prefix(&prefix).map(|name| (name, asset))),
        );
        assert_eq!(
            serde_json::to_vec(&actual).unwrap(),
            serde_json::to_vec(&expected).unwrap()
        );
        assert_eq!(
            serde_json::to_vec(&actual).unwrap(),
            serde_json::to_vec(&original_asset_fingerprints(&original)).unwrap()
        );
        assert_eq!(asset_fingerprints(&original), actual);
        assert_eq!(asset_fingerprints_from(std::iter::empty()), json!({}));
    }

    struct Fixture {
        domain: SkillsDomain,
        project: V,
        root: PathBuf,
        _directory: tempfile::TempDir,
    }
    impl Fixture {
        fn new() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let base = infra_core::fs::canonicalize(directory.path()).unwrap();
            let source = base.join("source");
            std::fs::create_dir_all(source.join("install/open-source-kit")).unwrap();
            std::fs::write(
                source.join("install/open-source-kit/resources.json"),
                br#"{"resources":[]}"#,
            )
            .unwrap();
            let root = base.join("project");
            std::fs::create_dir(&root).unwrap();
            let store = SessionStore::open(&base.join("state")).unwrap();
            let project = store.register(&root).unwrap();
            let domain = SkillsDomain::new(store, source).unwrap();
            Self {
                domain,
                project,
                root,
                _directory: directory,
            }
        }
        fn call(&self, operation: &str, body: V) -> Result<V> {
            match self.domain.dispatch(
                operation,
                json!({"body":body,"params":{"project_id":self.project["id"]},"query":{}}),
            )? {
                Payload::Json(value) => Ok(value),
                Payload::Bytes { .. } => panic!("skill route must return JSON"),
            }
        }
        fn create_and_install(&self) -> (V, Vec<u8>) {
            let preview = self.call("skills.create_preview", json!({
                "project_id":self.project["id"],"agents":["codex"],"name":"fixture",
                "description":"Private skill fixture","instructions":"# Original\n\nKeep the existing files."
            })).unwrap();
            let content = preview["content"].as_str().unwrap().as_bytes().to_vec();
            let installed = self
                .call(
                    "skills.install",
                    json!({"preview_id":preview["preview_id"]}),
                )
                .unwrap();
            assert_eq!(installed["ok"], true);
            assert_eq!(
                installed["installed"],
                json!([format!("{PREFIX}/SKILL.md")])
            );
            assert_eq!(
                std::fs::read(self.root.join(PREFIX).join("SKILL.md")).unwrap(),
                content
            );
            (preview, content)
        }
        // Build the same owned reviewed-change image as upstream previewing,
        // substituting local bytes for its network-only source fetch. Every
        // subsequent apply/validation/publication uses the actual dispatcher.
        fn reviewed_update(&self) -> String {
            let removal = self.call("skills.change_preview", json!({
                "project_id":self.project["id"],"agent":"codex","name":"fixture","operation":"remove"
            })).unwrap();
            let mut preview = self
                .domain
                .consume(removal["preview_id"].as_str().unwrap(), "change")
                .unwrap();
            let assets = BTreeMap::from([
                (
                    "SKILL.md".to_owned(),
                    Asset {
                        body: b"# Reviewed update\n".to_vec(),
                        mode: 0o644,
                    },
                ),
                (
                    "assets/data.bin".to_owned(),
                    Asset {
                        body: vec![0, 255, 1, 2],
                        mode: 0o644,
                    },
                ),
                (
                    "z.txt".to_owned(),
                    Asset {
                        body: b"reviewed\n".to_vec(),
                        mode: 0o644,
                    },
                ),
            ]);
            let (_, mut records) = self.domain.records(&self.project).unwrap();
            records["skills"][PREFIX]["files"] = original_asset_fingerprints(&assets);
            records["skills"][PREFIX]["revision"] = json!("reviewed-local-update");
            preview.operation = "update".into();
            preview.payloads = assets
                .into_iter()
                .map(|(name, asset)| (format!("{PREFIX}/{name}"), asset))
                .collect();
            preview.records_after = records;
            let id = Uuid::new_v4().simple().to_string();
            self.domain.cache(id.clone(), preview).unwrap();
            id
        }
        fn tracking(&self) -> Vec<u8> {
            self.domain.records(&self.project).unwrap().0.unwrap()
        }
        fn activate(&self, status: &str) {
            self.domain.store.connection().unwrap().execute(
                "INSERT INTO sessions(id,title,project_id,project_path,provider,mode,workflow,project_context,status,created_at,updated_at)VALUES('active','fixture',?1,?2,'codex','edit','native',0,?3,'created','updated')",
                rusqlite::params![self.project["id"].as_str().unwrap(), self.project["path"].as_str().unwrap(), status],
            ).unwrap();
        }
    }
    fn assert_conflict(result: Result<V>, message: &str) {
        let error = result.unwrap_err();
        assert_eq!(
            (error.status, error.code, error.message.as_str()),
            (409, "conflict", message)
        );
    }

    #[test]
    fn actual_custom_skill_create_update_remove_and_one_use_previews() {
        let fixture = Fixture::new();
        let (created, _) = fixture.create_and_install();
        assert_conflict(
            fixture.call(
                "skills.install",
                json!({"preview_id":created["preview_id"]}),
            ),
            "The preview expired or was already used. Preview again.",
        );
        let update = fixture.reviewed_update();
        let changed = fixture
            .call("skills.apply_change", json!({"preview_id":update}))
            .unwrap();
        assert_eq!(changed["ok"], true);
        assert_eq!(
            changed["changed"],
            json!([
                format!("{PREFIX}/assets/data.bin"),
                format!("{PREFIX}/z.txt"),
                format!("{PREFIX}/SKILL.md")
            ])
        );
        assert_eq!(
            std::fs::read(fixture.root.join(PREFIX).join("SKILL.md")).unwrap(),
            b"# Reviewed update\n"
        );
        assert_eq!(
            std::fs::read(fixture.root.join(PREFIX).join("assets/data.bin")).unwrap(),
            [0, 255, 1, 2]
        );
        let installed = fixture.call("skills.installed", json!({})).unwrap();
        assert_eq!(installed["installed"][0]["state"], "clean");
        assert_eq!(
            installed["installed"][0]["revision"],
            "reviewed-local-update"
        );
        assert_conflict(
            fixture.call("skills.apply_change", json!({"preview_id":update})),
            "The preview expired or was already used. Preview again.",
        );
        let remove = fixture.call("skills.change_preview", json!({
            "project_id":fixture.project["id"],"agent":"codex","name":"fixture","operation":"remove"
        })).unwrap();
        let removed = fixture
            .call(
                "skills.apply_change",
                json!({"preview_id":remove["preview_id"]}),
            )
            .unwrap();
        assert_eq!(removed["ok"], true);
        for name in ["SKILL.md", "assets/data.bin", "z.txt"] {
            assert!(!fixture.root.join(PREFIX).join(name).exists());
        }
        assert!(
            fixture.domain.records(&fixture.project).unwrap().1["skills"]
                .as_object()
                .unwrap()
                .is_empty()
        );
        assert_conflict(
            fixture.call(
                "skills.apply_change",
                json!({"preview_id":remove["preview_id"]}),
            ),
            "The preview expired or was already used. Preview again.",
        );
    }

    #[test]
    fn changed_extra_and_replaced_sources_fail_before_any_publication() {
        for mutation in ["body", "extra", "inode"] {
            let fixture = Fixture::new();
            let (_, original) = fixture.create_and_install();
            let update = fixture.reviewed_update();
            let tracking = fixture.tracking();
            let skill = fixture.root.join(PREFIX).join("SKILL.md");
            match mutation {
                "body" => std::fs::write(&skill, b"unreviewed change\n").unwrap(),
                "extra" => {
                    std::fs::write(fixture.root.join(PREFIX).join("extra.txt"), b"preserve\n")
                        .unwrap()
                }
                "inode" => {
                    std::fs::rename(&skill, fixture.root.join("preserved-original.md")).unwrap();
                    std::fs::write(&skill, &original).unwrap();
                }
                _ => unreachable!(),
            }
            let expected_body = std::fs::read(&skill).unwrap();
            assert_conflict(
                fixture.call("skills.apply_change", json!({"preview_id":update})),
                if mutation == "extra" {
                    "Skill content or extra files changed after preview."
                } else {
                    "Skill files changed after preview. No files were changed."
                },
            );
            assert_eq!(std::fs::read(&skill).unwrap(), expected_body);
            assert_eq!(fixture.tracking(), tracking);
            assert!(!fixture.root.join(PREFIX).join("assets/data.bin").exists());
            assert!(!fixture.root.join(PREFIX).join("z.txt").exists());
            if mutation == "extra" {
                assert_eq!(
                    std::fs::read(fixture.root.join(PREFIX).join("extra.txt")).unwrap(),
                    b"preserve\n"
                );
            }
        }
    }

    #[test]
    fn active_admission_refusal_keeps_priority_over_changed_source() {
        for status in ["queued", "running"] {
            let fixture = Fixture::new();
            fixture.create_and_install();
            let update = fixture.reviewed_update();
            let tracking = fixture.tracking();
            let skill = fixture.root.join(PREFIX).join("SKILL.md");
            std::fs::write(&skill, b"unreviewed change\n").unwrap();
            fixture.activate(status);
            assert_conflict(
                fixture.call("skills.apply_change", json!({"preview_id":update})),
                "Wait for active sessions to finish before changing skills.",
            );
            assert_eq!(std::fs::read(skill).unwrap(), b"unreviewed change\n");
            assert_eq!(fixture.tracking(), tracking);
            assert!(!fixture.root.join(PREFIX).join("assets/data.bin").exists());
            assert!(!fixture.root.join(PREFIX).join("z.txt").exists());
        }
    }
}
