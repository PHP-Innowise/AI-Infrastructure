use crate::common::*;
use infra_core::{BankService, ErrorKind, Result, Roots, RuntimeContext, RuntimeError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json as object};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
    time::Duration,
};

/// Prefixes a validation error with where in the passport it was found, so a
/// form of many services says which field to correct.
fn located(place: &str) -> impl Fn(RuntimeError) -> RuntimeError + '_ {
    move |mut error| {
        error.message = format!("{place}: {}", error.message);
        error
    }
}
pub fn manifest(value: &Value, sid: &str) -> Result<()> {
    let subject = if sid == "draft" {
        "Service".to_string()
    } else {
        format!("Service {sid}")
    };
    manifest_fields(value, sid).map_err(located(&subject))
}
fn manifest_fields(value: &Value, sid: &str) -> Result<()> {
    fields(
        value,
        &[
            "schema_version",
            "id",
            "description",
            "owner",
            "capabilities",
            "provides",
            "consumes",
            "sources",
        ],
        &["relationships_complete"],
    )?;
    if value["schema_version"] != 1 || identifier(string(&value["id"]))? != sid {
        return Err(fail("Manifest service ID/schema does not match registry"));
    }
    text(&value["description"], "description", 2000)?;
    text(&value["owner"], "owner", 200)?;
    if value
        .get("relationships_complete")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err(fail("relationships_complete must be boolean"));
    }
    let mut all_sources = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for (index, capability) in items(&value["capabilities"], MAX_ITEMS)
        .map_err(located("capabilities"))?
        .iter()
        .enumerate()
    {
        let place = format!("capability {}", index + 1);
        (|| -> Result<()> {
            fields(
                capability,
                &["id", "description", "status", "sources"],
                &["keywords"],
            )?;
            let id = identifier(string(&capability["id"]))?;
            if !seen.insert(id.clone()) {
                return Err(fail(format!("Duplicate capability {id}")));
            }
            text(&capability["description"], "capability description", 2000)?;
            choice(
                &capability["status"],
                &["implemented", "partial", "planned", "unknown"],
                "capability status",
            )?;
            for path in items(&capability["sources"], MAX_ITEMS)? {
                all_sources.insert(relative(string(path))?);
            }
            if let Some(keywords) = capability.get("keywords") {
                for keyword in items(keywords, MAX_ITEMS)? {
                    text(keyword, "keyword", 200)?;
                }
            }
            Ok(())
        })()
        .map_err(located(&place))?;
    }
    seen.clear();
    for (index, provided) in items(&value["provides"], MAX_ITEMS)
        .map_err(located("provided contracts"))?
        .iter()
        .enumerate()
    {
        let place = format!("provided contract {}", index + 1);
        (|| -> Result<()> {
            fields(provided, &["id", "kind", "version", "sources"], &[])?;
            let id = identifier(string(&provided["id"]))?;
            if !seen.insert(id.clone()) {
                return Err(fail(format!("Duplicate provided contract {id}")));
            }
            choice(
                &provided["kind"],
                &["http", "event", "rpc", "graphql", "other"],
                "contract kind",
            )?;
            text(&provided["version"], "contract version", 100)?;
            for path in items(&provided["sources"], MAX_ITEMS)? {
                all_sources.insert(relative(string(path))?);
            }
            Ok(())
        })()
        .map_err(located(&place))?;
    }
    let mut consumes = BTreeSet::new();
    for (index, consumed) in items(&value["consumes"], MAX_ITEMS)
        .map_err(located("consumed contracts"))?
        .iter()
        .enumerate()
    {
        let place = format!("consumed contract {}", index + 1);
        (|| -> Result<()> {
            fields(consumed, &["service", "contract", "version"], &[])?;
            let key = (
                identifier(string(&consumed["service"]))?,
                identifier(string(&consumed["contract"]))?,
            );
            if consumes.contains(&key) {
                return Err(fail(format!(
                    "Duplicate consumed contract {}:{}",
                    key.0, key.1
                )));
            }
            consumes.insert(key);
            text(&consumed["version"], "contract version", 100)?;
            Ok(())
        })()
        .map_err(located(&place))?;
    }
    seen.clear();
    for (index, entry) in items(&value["sources"], MAX_ITEMS)
        .map_err(located("sources of context"))?
        .iter()
        .enumerate()
    {
        source(entry).map_err(located(&format!("source {}", index + 1)))?;
        let path = string(&entry["path"]).to_string();
        if !seen.insert(path.clone()) {
            return Err(fail(format!("Duplicate source {path}")));
        }
        all_sources.insert(path);
    }
    if all_sources.len() > MAX_ITEMS {
        return Err(fail("Too many distinct sources in service manifest"));
    }
    Ok(())
}
pub fn git_head(root: &Path) -> Option<String> {
    let output = infra_core::git::run(
        root,
        &infra_core::git::strings(&["rev-parse", "--verify", "HEAD"]),
        None,
        Duration::from_secs(5),
        4096,
    )
    .ok()?;
    let head = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (output.code == 0
        && (40..=64).contains(&head.len())
        && head
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()))
    .then_some(head)
}
/// The folder whose `.git` Git finds first from `root` (Git's own overrides
/// are removed when it runs): services under it share one HEAD, and a root
/// with none has no HEAD to ask Git for.
fn checkout_top(root: &Path) -> Option<PathBuf> {
    root.ancestors()
        .find(|folder| std::fs::symlink_metadata(folder.join(".git")).is_ok())
        .map(Path::to_path_buf)
}
/// HEADs of `roots`, in order: one Git process per checkout, up to eight at a
/// time. Asked once per service, 200 services took a second on every catalog
/// before any work began, and a monorepo asked once per service folder.
pub fn git_heads(roots: &[PathBuf]) -> Vec<Option<String>> {
    let tops = roots
        .iter()
        .map(|root| checkout_top(root))
        .collect::<Vec<_>>();
    let distinct = tops
        .iter()
        .flatten()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let share = distinct.len().div_ceil(distinct.len().clamp(1, 8)).max(1);
    let heads = std::thread::scope(|scope| {
        distinct
            .chunks(share)
            .map(|part| {
                scope.spawn(move || {
                    part.iter()
                        .map(|top| (top.clone(), git_head(top)))
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .flat_map(|worker| worker.join().unwrap_or_default())
            .collect::<BTreeMap<_, _>>()
    });
    tops.iter()
        .map(|top| {
            top.as_ref()
                .and_then(|top| heads.get(top).cloned().flatten())
        })
        .collect()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Service {
    pub id: String,
    pub root: PathBuf,
    pub manifest_path: String,
    pub manifest: Option<Value>,
    pub access: String,
    pub head: Option<String>,
}
#[derive(Clone, Debug)]
pub struct System {
    pub config: PathBuf,
    pub root: PathBuf,
    pub allow_roots: Vec<PathBuf>,
    pub config_data: Value,
    pub services: BTreeMap<String, Service>,
    pub edges: Vec<Value>,
    pub snapshot: BTreeMap<(String, String), String>,
    pub missing: BTreeSet<(String, String)>,
    pub warnings: Vec<Value>,
    overrides: BTreeMap<PathBuf, Vec<u8>>,
}
#[derive(Clone, Debug)]
pub struct ReadBudget {
    pub files: usize,
    pub bytes: usize,
}
impl Default for ReadBudget {
    fn default() -> Self {
        Self {
            files: 2000,
            bytes: 32 * 1024 * 1024,
        }
    }
}
impl System {
    pub fn new(config: &Path, allow: &[PathBuf]) -> Result<Self> {
        Self::with_files(config, allow, BTreeMap::new())
    }
    pub fn with_files(
        config: &Path,
        allow: &[PathBuf],
        overrides: BTreeMap<PathBuf, Vec<u8>>,
    ) -> Result<Self> {
        let config = absolute(config)?;
        let root = config
            .parent()
            .ok_or_else(|| fail("System configuration has no parent"))?
            .to_path_buf();
        infra_core::bank::publication::PublicationFiles::new(&root)?;
        let mut system = Self {
            config: config.clone(),
            root: root.clone(),
            allow_roots: std::iter::once(Ok(root.clone()))
                .chain(allow.iter().map(|path| absolute(path)))
                .collect::<Result<Vec<_>>>()?,
            config_data: Value::Null,
            services: BTreeMap::new(),
            edges: vec![],
            snapshot: BTreeMap::new(),
            missing: BTreeSet::new(),
            warnings: vec![],
            overrides,
        };
        let name = config
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| fail("System filename is not UTF-8"))?
            .to_string();
        let raw = system.read("__system__", &root, &name, None, None)?;
        let document = parse_json(&raw, MAX_BYTES)?;
        fields(
            &document,
            &["schema_version", "name", "services", "shared_sources"],
            &[],
        )?;
        if document["schema_version"] != 1 {
            return Err(fail("Unsupported schema_version"));
        }
        text(&document["name"], "system name", 200)?;
        for entry in items(&document["shared_sources"], MAX_ITEMS)? {
            source(entry)?;
        }
        for entry in items(&document["services"], MAX_SERVICES)? {
            fields(entry, &["id", "root", "manifest"], &[])?;
            let sid = identifier(string(&entry["id"]))?;
            if system.services.contains_key(&sid) {
                return Err(fail("Duplicate service ID"));
            }
            let location = text(&entry["root"], "service root", 4096)?;
            let root = absolute(&root.join(location))?;
            let manifest_path = relative(string(&entry["manifest"]))?;
            system.services.insert(
                sid.clone(),
                Service {
                    id: sid,
                    root,
                    manifest_path,
                    manifest: None,
                    access: "unavailable".into(),
                    head: None,
                },
            );
        }
        system.config_data = document;
        let ids = system.services.keys().cloned().collect::<Vec<_>>();
        for sid in ids {
            let service = system.services[&sid].clone();
            if !system
                .allow_roots
                .iter()
                .any(|allowed| service.root.starts_with(allowed))
            {
                system
                    .services
                    .get_mut(&sid)
                    .ok_or_else(|| fail("Service disappeared"))?
                    .access = "denied".into();
                system.warn(&sid, "root_not_allowed", None);
                continue;
            }
            let raw = match system.read(&sid, &service.root, &service.manifest_path, None, None) {
                Ok(raw) => raw,
                Err(_) => {
                    system.warn(&sid, "manifest_unavailable", None);
                    continue;
                }
            };
            let passport = parse_json(&raw, MAX_SOURCE_BYTES)?;
            manifest(&passport, &sid)?;
            if passport.get("relationships_complete") != Some(&Value::Bool(true)) {
                system.warn(&sid, "relationships_incomplete", None);
            }
            let entry = system
                .services
                .get_mut(&sid)
                .ok_or_else(|| fail("Service disappeared"))?;
            entry.manifest = Some(passport);
            entry.access = "available".into();
        }
        let available = system
            .services
            .iter()
            .filter(|(_, service)| service.access == "available")
            .map(|(sid, service)| (sid.clone(), service.root.clone()))
            .collect::<Vec<_>>();
        let heads = git_heads(
            &available
                .iter()
                .map(|(_, root)| root.clone())
                .collect::<Vec<_>>(),
        );
        for ((sid, _), head) in available.into_iter().zip(heads) {
            if let Some(service) = system.services.get_mut(&sid) {
                service.head = head;
            }
        }
        for (sid, service) in system.services.clone() {
            let Some(passport) = service.manifest else {
                continue;
            };
            for consume in items(&passport["consumes"], MAX_ITEMS)? {
                let provider_id = string(&consume["service"]);
                let provider = system
                    .services
                    .get(provider_id)
                    .ok_or_else(|| {
                        fail(format!(
                            "{sid} consumes {} from {provider_id}, which is not registered in this system",
                            string(&consume["contract"])
                        ))
                    })?;
                let provided = provider
                    .manifest
                    .as_ref()
                    .and_then(|manifest| manifest["provides"].as_array())
                    .and_then(|contracts| {
                        contracts
                            .iter()
                            .find(|contract| contract["id"] == consume["contract"])
                    })
                    .cloned();
                if provider.manifest.is_some() && provided.is_none() {
                    return Err(fail(format!(
                        "{sid} consumes {} from {provider_id}, which does not declare it",
                        string(&consume["contract"])
                    )));
                }
                if provided.is_none() {
                    system.warn(&sid, "provider_contract_unavailable", None);
                } else if provided
                    .as_ref()
                    .is_some_and(|provided| provided["version"] != consume["version"])
                {
                    system.warn(&sid, "contract_version_mismatch", None);
                }
                system.edges.push(object!({"provider":provider_id,"consumer":sid,"contract":consume["contract"],"version":consume["version"],"provenance":"declared"}));
            }
        }
        Ok(system)
    }
    pub fn warn(&mut self, sid: &str, reason: &str, path: Option<&str>) {
        let mut warning = object!({"service":sid,"reason":reason});
        if let Some(path) = path {
            warning["path"] = path.into();
        }
        if !self.warnings.contains(&warning) {
            self.warnings.push(warning);
        }
    }
    pub fn read(
        &mut self,
        sid: &str,
        root: &Path,
        path: &str,
        mut budget: Option<&mut ReadBudget>,
        fits: Option<usize>,
    ) -> Result<Vec<u8>> {
        if let Some(budget) = budget.as_mut() {
            if budget.files == 0 {
                return Err(RuntimeError::coded(
                    ErrorKind::Unavailable,
                    "read_limit",
                    "read_limit",
                ));
            }
            budget.files -= 1;
        }
        let limit = if sid == "__system__"
            && self.config.file_name().and_then(|name| name.to_str()) == Some(path)
        {
            MAX_BYTES
        } else {
            MAX_SOURCE_BYTES
        };
        let before = |size: u64| -> Result<()> {
            if let Some(room) = fits {
                let least = 2000usize.min((size as usize).div_ceil(4) + 1);
                if least > room {
                    return Err(RuntimeError::coded(
                        ErrorKind::Unavailable,
                        "budget",
                        "budget",
                    ));
                }
            }
            if budget
                .as_ref()
                .is_some_and(|budget| size > budget.bytes as u64)
            {
                return Err(RuntimeError::coded(
                    ErrorKind::Unavailable,
                    "read_limit",
                    "read_limit",
                ));
            }
            Ok(())
        };
        let result = if let Some(raw) = self.overrides.get(&root.join(path)) {
            if raw.len() > limit {
                Err(fail("Source exceeds size limit"))
            } else {
                before(raw.len() as u64)?;
                Ok(raw.clone())
            }
        } else {
            read_checked(root, path, limit, before)
        };
        let raw = match result {
            Ok(raw) => raw,
            Err(error) => {
                if error.kind == ErrorKind::NotFound {
                    self.missing.insert((sid.into(), path.into()));
                }
                return Err(error);
            }
        };
        if let Some(budget) = budget {
            budget.bytes = budget.bytes.saturating_sub(raw.len());
        }
        self.snapshot
            .insert((sid.into(), path.into()), digest(&raw));
        Ok(raw)
    }
    pub fn catalog(&self) -> Value {
        object!({"schema_version":1,"system":self.config_data["name"],"services":self.services.iter().map(|(sid,service)|object!({"id":sid,"root":service.root,"access":service.access,"head":service.head,"declared":service.manifest})).collect::<Vec<_>>(),"relationships":self.edges,"warnings":self.warnings})
    }
    pub fn diagram(&self) -> String {
        let names = self
            .services
            .keys()
            .enumerate()
            .map(|(index, sid)| (sid.clone(), format!("s{index}")))
            .collect::<BTreeMap<_, _>>();
        let mut lines = vec!["flowchart LR".to_string()];
        for (sid, node) in &names {
            let availability = if self.services[sid].access == "available" {
                ""
            } else {
                " (unavailable)"
            };
            lines.push(format!("    {node}[\"{sid}{availability}\"]"));
        }
        for edge in &self.edges {
            if let (Some(provider), Some(consumer)) = (
                names.get(string(&edge["provider"])),
                names.get(string(&edge["consumer"])),
            ) {
                lines.push(format!(
                    "    {provider} -->|{}| {consumer}",
                    string(&edge["contract"])
                ));
            }
        }
        lines.join("\n") + "\n"
    }
    pub fn locate(&self, query: &str) -> Result<Value> {
        text(&object!(query), "task", 2000)?;
        let words = words(query);
        let mut candidates = vec![];
        for (sid, service) in &self.services {
            let Some(manifest) = &service.manifest else {
                continue;
            };
            let mut matches = vec![];
            for capability in items(&manifest["capabilities"], MAX_ITEMS)? {
                let subject = format!(
                    "{} {} {}",
                    string(&capability["id"]),
                    string(&capability["description"]),
                    texts(&capability["keywords"]).join(" ")
                );
                let matched = words
                    .intersection(&words_for(&subject))
                    .cloned()
                    .collect::<Vec<_>>();
                if !matched.is_empty() {
                    matches.push(object!({"capability":capability["id"],"terms":matched,"declared_status":capability["status"]}));
                }
            }
            if !matches.is_empty() {
                let score = matches
                    .iter()
                    .flat_map(|matched| texts(&matched["terms"]))
                    .collect::<BTreeSet<_>>()
                    .len();
                candidates.push(object!({"service":sid,"matches":matches,"score":score,"provenance":"inferred"}));
            }
        }
        // Best-supported candidates first: the most distinct task terms matched.
        candidates.sort_by(|a, b| {
            b["score"]
                .as_u64()
                .cmp(&a["score"].as_u64())
                .then_with(|| string(&a["service"]).cmp(string(&b["service"])))
        });
        let selection = if candidates.len() == 1 {
            "suggested"
        } else {
            "needs_selection"
        };
        Ok(object!({"candidates":candidates,"selection":selection}))
    }
    pub fn impact(
        &mut self,
        seeds: &[String],
        contracts: &[String],
        depth: usize,
    ) -> Result<Vec<Value>> {
        if depth > 64 {
            return Err(fail("impact depth must be between 0 and 64"));
        }
        let mut selected = BTreeMap::new();
        for sid in seeds.iter().collect::<BTreeSet<_>>() {
            if !self.services.contains_key(sid) {
                return Err(fail("Selected service is not registered"));
            }
            selected.insert(
                sid.clone(),
                object!({"service":sid,"reason":"selected","path":[sid]}),
            );
        }
        let mut changed = BTreeSet::new();
        for selector in contracts {
            let values = selector.split(':').collect::<Vec<_>>();
            if values.len() != 2 {
                return Err(fail("Contract selector must be SERVICE:CONTRACT"));
            }
            let (sid, cid) = (identifier(values[0])?, identifier(values[1])?);
            let provided = self
                .services
                .get(&sid)
                .and_then(|service| service.manifest.as_ref())
                .and_then(|value| value["provides"].as_array())
                .is_some_and(|entries| entries.iter().any(|entry| entry["id"] == cid));
            if !provided {
                return Err(fail("Selected contract unavailable or undeclared"));
            }
            changed.insert((sid.clone(), cid));
            selected
                .entry(sid.clone())
                .or_insert(object!({"service":sid,"reason":"contract_provider","path":[sid]}));
        }
        let mut queue = selected.keys().cloned().collect::<VecDeque<_>>();
        let mut traversed = BTreeSet::new();
        while let Some(sid) = queue.pop_front() {
            let path = texts(&selected[&sid]["path"]);
            for edge in self.edges.clone() {
                if edge["provider"] != sid {
                    continue;
                }
                if !changed.is_empty()
                    && path.len() == 1
                    && !changed.contains(&(sid.clone(), string(&edge["contract"]).to_string()))
                {
                    continue;
                }
                let consumer = string(&edge["consumer"]).to_string();
                traversed.insert((sid.clone(), consumer.clone()));
                if path.contains(&consumer) {
                    self.warn(&consumer, "dependency_cycle", None);
                }
                if selected.contains_key(&consumer) {
                    continue;
                }
                if path.len().saturating_sub(1) >= depth {
                    self.warn(&consumer, "impact_depth_exceeded", None);
                    continue;
                }
                let mut next = path.clone();
                next.push(consumer.clone());
                selected.insert(consumer.clone(),object!({"service":consumer,"reason":"potential_consumer","contract":edge["contract"],"path":next}));
                queue.push_back(consumer);
            }
        }
        let mut indegree = selected
            .keys()
            .map(|sid| (sid.clone(), 0usize))
            .collect::<BTreeMap<_, _>>();
        let mut outgoing = BTreeMap::<String, BTreeSet<String>>::new();
        for (provider, consumer) in traversed {
            if selected.contains_key(&provider) && selected.contains_key(&consumer) {
                outgoing
                    .entry(provider)
                    .or_default()
                    .insert(consumer.clone());
                *indegree.entry(consumer).or_default() += 1;
            }
        }
        let mut ready = indegree
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(sid, _)| sid.clone())
            .collect::<VecDeque<_>>();
        let mut removed = 0;
        while let Some(sid) = ready.pop_front() {
            removed += 1;
            for consumer in outgoing.get(&sid).into_iter().flatten() {
                let count = indegree
                    .get_mut(consumer)
                    .ok_or_else(|| fail("Missing dependency node"))?;
                *count -= 1;
                if *count == 0 {
                    ready.push_back(consumer.clone());
                }
            }
        }
        if removed != selected.len() {
            self.warn("__system__", "dependency_cycle", None);
        }
        Ok(selected.into_values().collect())
    }
    pub fn source_content(
        &mut self,
        sid: &str,
        root: &Path,
        entry: &Value,
        mut budget: Option<&mut ReadBudget>,
        room: Option<usize>,
    ) -> Result<Option<Value>> {
        let path = string(&entry["path"]).to_string();
        let result = (|| -> Result<Value> {
            let raw = self.read(sid, root, &path, budget.as_deref_mut(), room)?;
            let content = std::str::from_utf8(&raw).map_err(|_| fail("Source is not UTF-8"))?;
            if content.contains('\0') || sensitive(content) {
                return Err(fail("sensitive_or_binary"));
            }
            if entry["kind"] == "memory" {
                let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
                let raw = normalized
                    .strip_prefix("---\n")
                    .and_then(|value| value.split_once("\n---\n"))
                    .ok_or_else(|| fail("unverified_memory"))?
                    .0;
                let meta = parse_json(raw.as_bytes(), MAX_SOURCE_BYTES)?;
                if !meta.is_object()
                    || meta["status"] != "active"
                    || texts(&meta["tags"])
                        .iter()
                        .any(|tag| tag == "auto-promoted")
                    || !meta["superseded_by"].is_null()
                {
                    return Err(fail("ineligible_memory"));
                }
                let today = infra_core::domain::clock::local_today();
                for key in ["review_after", "valid_to"] {
                    if key == "valid_to" && meta[key].is_null() {
                        continue;
                    }
                    let date = infra_core::domain::python_date(string(&meta[key]))
                        .ok_or_else(|| fail("Invalid memory date"))?;
                    if date < today {
                        return Err(fail("memory_review_due"));
                    }
                }
                let config = match self.read(
                    sid,
                    root,
                    "project-brain/config/runtime.json",
                    budget.as_deref_mut(),
                    None,
                ) {
                    Ok(raw) => parse_json(&raw, MAX_SOURCE_BYTES)?,
                    Err(error) if error.kind == ErrorKind::NotFound => object!({}),
                    Err(error) => return Err(error),
                };
                if !config.is_object() {
                    return Err(fail("ineligible_memory"));
                }
                let privacy = config
                    .get("allowed_privacy")
                    .cloned()
                    .unwrap_or(object!(["public", "team"]));
                let authority = config
                    .get("allowed_authority")
                    .cloned()
                    .unwrap_or(object!(["observed", "verified"]));
                if !texts(&privacy).iter().any(|value| value == "public")
                    || !texts(&authority).iter().any(|value| value == "verified")
                {
                    return Err(fail("ineligible_memory"));
                }
                let sources = items(&meta["sources"], MAX_ITEMS)?
                    .iter()
                    .map(|value| relative(string(value).split('#').next().unwrap_or("")))
                    .collect::<Result<BTreeSet<_>>>()?;
                let fingerprints = items(&meta["source_digests"], MAX_ITEMS)?;
                if sources.is_empty() || fingerprints.len() != sources.len() {
                    return Err(fail("unverified_memory"));
                }
                let mut seen = BTreeSet::new();
                for fingerprint in fingerprints {
                    fields(fingerprint, &["path", "sha256"], &[])?;
                    let source = relative(string(&fingerprint["path"]))?;
                    if !sources.contains(&source)
                        || !seen.insert(source.clone())
                        || !hex(string(&fingerprint["sha256"]), 64)
                    {
                        return Err(fail("unverified_memory"));
                    }
                    if digest(&self.read(sid, root, &source, budget.as_deref_mut(), None)?)
                        != string(&fingerprint["sha256"])
                    {
                        return Err(fail("stale_memory"));
                    }
                }
                let context = RuntimeContext::new(
                    Roots::installed(root.to_path_buf())?,
                    "ai-system",
                    "governed",
                )?;
                let bank = BankService::new(context);
                infra_core::bank::validation::metadata(&bank, &path, &meta, false)?;
                if infra_core::domain::secret_label(content)?.is_some() {
                    return Err(fail("sensitive_memory"));
                }
            }
            Ok(
                object!({"service":sid,"path":path,"kind":entry["kind"],"sha256":digest(&raw),"trust":"source_evidence","excerpt":content.chars().take(2000).collect::<String>(),"truncated":content.chars().count()>2000}),
            )
        })();
        match result {
            Ok(value) => Ok(Some(value)),
            Err(error) if matches!(error.code.as_deref(), Some("budget" | "read_limit")) => {
                Err(error)
            }
            Err(_) => {
                self.warn(sid, "source_unavailable_or_ineligible", Some(&path));
                Ok(None)
            }
        }
    }
}
/// Function words carry no routing signal: every description written in prose
/// contains them, so matching on them makes every service a candidate.
const STOP_WORDS: &[&str] = &[
    "about",
    "above",
    "across",
    "after",
    "again",
    "against",
    "all",
    "also",
    "among",
    "and",
    "another",
    "any",
    "are",
    "been",
    "before",
    "being",
    "below",
    "between",
    "both",
    "but",
    "can",
    "cannot",
    "could",
    "did",
    "does",
    "doing",
    "done",
    "during",
    "each",
    "either",
    "every",
    "few",
    "for",
    "from",
    "further",
    "had",
    "has",
    "have",
    "having",
    "her",
    "here",
    "hers",
    "him",
    "his",
    "how",
    "into",
    "its",
    "itself",
    "just",
    "least",
    "less",
    "may",
    "might",
    "more",
    "most",
    "much",
    "must",
    "neither",
    "nor",
    "not",
    "off",
    "once",
    "only",
    "onto",
    "other",
    "our",
    "ours",
    "out",
    "over",
    "own",
    "per",
    "same",
    "shall",
    "she",
    "should",
    "since",
    "some",
    "such",
    "than",
    "that",
    "the",
    "their",
    "theirs",
    "them",
    "then",
    "there",
    "these",
    "they",
    "this",
    "those",
    "through",
    "too",
    "under",
    "unless",
    "until",
    "upon",
    "very",
    "via",
    "was",
    "were",
    "what",
    "when",
    "where",
    "whether",
    "which",
    "while",
    "who",
    "whom",
    "whose",
    "why",
    "will",
    "with",
    "within",
    "without",
    "would",
    "yet",
    "you",
    "your",
    "yours",
    "без",
    "был",
    "была",
    "были",
    "было",
    "быть",
    "весь",
    "все",
    "всё",
    "где",
    "для",
    "его",
    "если",
    "есть",
    "ещё",
    "еще",
    "или",
    "как",
    "когда",
    "который",
    "которая",
    "которые",
    "кто",
    "между",
    "может",
    "можно",
    "над",
    "надо",
    "нет",
    "них",
    "она",
    "они",
    "оно",
    "перед",
    "под",
    "после",
    "при",
    "так",
    "также",
    "там",
    "тоже",
    "только",
    "уже",
    "чем",
    "через",
    "что",
    "чтобы",
    "это",
    "этот",
    "эта",
    "эти",
];
/// Lowercase Unicode words of three or more characters, without function words,
/// with English plurals folded (`orders` matches `order`, `policies` `policy`),
/// applied alike to the task and to every capability.
static WORD: std::sync::LazyLock<Option<regex::Regex>> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"[^\W_]{3,}").ok());
fn words_for(value: &str) -> BTreeSet<String> {
    WORD.as_ref()
        .map(|pattern| {
            pattern
                .find_iter(&value.to_lowercase())
                .map(|word| word.as_str())
                .filter(|word| !STOP_WORDS.contains(word))
                .map(|word| {
                    if let Some(stem) = word.strip_suffix("ies").filter(|stem| stem.len() >= 2) {
                        format!("{stem}y")
                    } else if word.len() > 3
                        && word.ends_with('s')
                        && !word.ends_with("ss")
                        && !word.ends_with("us")
                    {
                        word[..word.len() - 1].to_string()
                    } else {
                        word.to_string()
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}
fn words(value: &str) -> BTreeSet<String> {
    words_for(value)
}
/// Routing words of text that may name code: `CancelOrder.php` reads as
/// `cancel order php`, so file names share terms with the task that names them.
pub fn terms(value: &str) -> BTreeSet<String> {
    let mut spaced = String::with_capacity(value.len() + 8);
    let mut previous_lower = false;
    for character in value.chars() {
        if character.is_uppercase() && previous_lower {
            spaced.push(' ');
        }
        previous_lower = character.is_lowercase();
        spaced.push(character);
    }
    words_for(&spaced)
}
