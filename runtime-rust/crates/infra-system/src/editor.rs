use crate::{
    backend::State,
    catalog::{self, System},
    common::*,
};
use infra_core::bank::publication::PublicationFiles;
use infra_core::{Result, RuntimeError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json as object};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FileSnapshot {
    pub sha256: Option<String>,
    pub body: Option<String>,
    pub mode: u32,
    pub parent: (u64, u64),
}
pub fn snapshot(root: &Path, name: &str) -> Result<FileSnapshot> {
    relative(name)?;
    let mut files = PublicationFiles::new(root)?;
    let raw = files.read(name, MAX_BYTES)?;
    let mode = files
        .read_mode(name)?
        .unwrap_or(if cfg!(unix) { 0o600 } else { 0o644 });
    let parent = Path::new(name)
        .parent()
        .and_then(|path| path.to_str())
        .unwrap_or("")
        .replace('\\', "/");
    let identities = files.directory_identities()?;
    let identity = if parent.is_empty() {
        files.root_identity()
    } else {
        *identities
            .get(&parent)
            .ok_or_else(|| fail("Metadata parent custody missing"))?
    };
    files.check()?;
    Ok(FileSnapshot {
        sha256: raw.as_ref().map(|bytes| digest(bytes)),
        body: raw.as_ref().map(|bytes| b64(bytes)),
        mode,
        parent: (identity.0, identity.1),
    })
}
pub fn replace(
    root: &Path,
    name: &str,
    raw: Option<&[u8]>,
    expected: &FileSnapshot,
    mode: u32,
) -> Result<()> {
    if snapshot(root, name)? != *expected {
        return Err(RuntimeError::conflict(
            "System files changed. Reload editor before saving",
        ));
    }
    let before = expected.body.as_deref().map(unb64).transpose()?;
    let mut files = PublicationFiles::new(root)?;
    let observed = files.read(name, MAX_BYTES)?;
    let parent = Path::new(name)
        .parent()
        .and_then(|path| path.to_str())
        .unwrap_or("")
        .replace('\\', "/");
    let identities = files.directory_identities()?;
    let identity = if parent.is_empty() {
        files.root_identity()
    } else {
        *identities
            .get(&parent)
            .ok_or_else(|| fail("Metadata parent custody missing"))?
    };
    if (identity.0, identity.1) != expected.parent || observed != before {
        return Err(RuntimeError::conflict(
            "Selected folder or file changed while saving",
        ));
    }
    match raw {
        Some(raw) => files.write_expected_mode_checked(
            name,
            raw,
            before.as_deref(),
            before.as_ref().map(|_| expected.mode),
            mode,
            MAX_BYTES,
        )?,
        None => {
            if let Some(before) = before {
                files.unlink_expected_limit(name, &before, MAX_BYTES)?;
            }
        }
    }
    files.check()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EditFile {
    pub root: PathBuf,
    pub name: String,
    pub before: FileSnapshot,
    pub after: String,
}
struct Preview {
    created: Instant,
    project_id: String,
    config_path: String,
    files: Vec<EditFile>,
    system: System,
    size: usize,
    discovery: Option<Value>,
}
pub struct Editor {
    previews: BTreeMap<String, Preview>,
}
impl Editor {
    pub fn new() -> Self {
        Self {
            previews: BTreeMap::new(),
        }
    }
    pub fn recover(&self, state: &State, key: Option<&str>, locked: bool) -> Result<()> {
        if !locked && state.host.active()? {
            return Ok(());
        }
        let rows = if let Some(key) = key {
            state
                .db
                .get("system_edits", key)?
                .map(|value| vec![(key.to_string(), value)])
                .unwrap_or_default()
        } else {
            state.db.all("system_edits", None)?
        };
        for (key, value) in rows {
            let files: Vec<EditFile> = serde_json::from_value(value["files"].clone())
                .map_err(|_| fail("Malformed interrupted system save"))?;
            let result = (|| -> Result<()> {
                let roots = files
                    .iter()
                    .map(|file| file.root.clone())
                    .collect::<Vec<_>>();
                let _locks = if locked {
                    vec![]
                } else {
                    workspace_locks_for_roots(&roots)?
                };
                let current = files
                    .iter()
                    .map(|file| snapshot(&file.root, &file.name))
                    .collect::<Result<Vec<_>>>()?;
                let expected_hashes = files
                    .iter()
                    .map(|file| unb64(&file.after).map(|bytes| digest(&bytes)))
                    .collect::<Result<Vec<_>>>()?;
                let completed = current.iter().zip(&files).zip(&expected_hashes).all(
                    |((current, file), hash)| {
                        current.sha256.as_deref() == Some(hash.as_str())
                            && current.parent == file.before.parent
                            && current.mode == file.before.mode
                    },
                );
                if !completed {
                    for (file, current) in files.iter().zip(&current).rev() {
                        if current == &file.before {
                            continue;
                        }
                        let after = unb64(&file.after)?;
                        if current.parent != file.before.parent
                            || current.mode != file.before.mode
                            || current.sha256.as_deref() != Some(digest(&after).as_str())
                        {
                            return Err(RuntimeError::conflict(
                                "Interrupted system save conflicts with external changes",
                            ));
                        }
                        let before = file.before.body.as_deref().map(unb64).transpose()?;
                        replace(
                            &file.root,
                            &file.name,
                            before.as_deref(),
                            current,
                            file.before.mode,
                        )?;
                    }
                }
                state.db.delete("system_edits", &key)?;
                Ok(())
            })();
            if result.is_err() {
                continue;
            }
        }
        Ok(())
    }
    pub fn check_recovery(&self, state: &State, root: &Path) -> Result<()> {
        self.recover(state, None, false)?;
        for (_, value) in state.db.all("system_edits", None)? {
            for file in value["files"].as_array().into_iter().flatten() {
                let prior = Path::new(string(&file["root"]));
                if prior.starts_with(root) || root.starts_with(prior) {
                    return Err(RuntimeError::conflict(
                        "Interrupted system save needs recovery; preserve external changes and inspect selected folders",
                    ));
                }
            }
        }
        Ok(())
    }
    pub fn location(
        &self,
        state: &State,
        project: &str,
        config: &str,
    ) -> Result<(PathBuf, PathBuf)> {
        let root = absolute(&state.project(project)?.path)?;
        relative(config)?;
        if !config.ends_with(".json") {
            return Err(fail("Choose relative system JSON file"));
        }
        self.check_recovery(state, &root)?;
        Ok((root.clone(), root.join(config)))
    }
    pub fn service(&self, state: &State, data: &Value) -> Result<Value> {
        fields(data, &["project_id"], &["folder"])?;
        let project = string(&data["project_id"]);
        let folder = data.get("folder").and_then(Value::as_str).unwrap_or(".");
        let root = state.folder(project, folder)?;
        self.check_recovery(state, &root)?;
        let before = snapshot(&root, "ai-service.json")?;
        let passport = if let Some(body) = &before.body {
            let passport = parse_json(&unb64(body)?, MAX_SOURCE_BYTES)?;
            catalog::manifest(&passport, string(&passport["id"]))?;
            passport
        } else {
            object!({"schema_version":1,"id":"","description":"","owner":"","relationships_complete":false,"capabilities":[],"provides":[],"consumes":[],"sources":[]})
        };
        Ok(
            object!({"project_id":project,"folder":folder,"path":root,"manifest":"ai-service.json","fingerprint":before.sha256,"passport":passport}),
        )
    }
    pub fn load(&self, state: &State, data: &Value) -> Result<Value> {
        fields(data, &["project_id", "config_path"], &[])?;
        let project = string(&data["project_id"]);
        let config_path = string(&data["config_path"]);
        let (root, config) = self.location(state, project, config_path)?;
        let before = snapshot(&root, config_path)?;
        let mut fingerprints = vec![object!([config, before.sha256])];
        let mut services = vec![];
        let (name, shared) = if before.body.is_none() {
            (
                root.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("system")
                    .to_string(),
                object!([]),
            )
        } else {
            let system = System::new(&config, &state.allowed()?)?;
            for entry in items(&system.config_data["services"], MAX_SERVICES)? {
                let service = system
                    .services
                    .get(string(&entry["id"]))
                    .ok_or_else(|| fail("Unknown configured service"))?;
                self.check_recovery(state, &service.root)?;
                if service.access != "available" {
                    return Err(fail(
                        "Service folder/passport unavailable; register before editing",
                    ));
                }
                let (pid, folder) = state.registered(&service.root)?;
                let prior = snapshot(&service.root, &service.manifest_path)?;
                services.push(object!({"project_id":pid,"folder":folder,"path":service.root,"manifest":service.manifest_path,"fingerprint":prior.sha256,"passport":service.manifest}));
                fingerprints.push(object!([
                    service.root.join(&service.manifest_path),
                    prior.sha256
                ]));
            }
            (
                string(&system.config_data["name"]).to_string(),
                system.config_data["shared_sources"].clone(),
            )
        };
        Ok(
            object!({"project_id":project,"config_path":config_path,"name":name,"services":services,"shared_sources":shared,"revision":digest(encoded(&object!(fingerprints))?.as_bytes())}),
        )
    }
    pub fn preview(&mut self, state: &State, data: &Value) -> Result<Value> {
        fields(
            data,
            &[
                "project_id",
                "config_path",
                "revision",
                "name",
                "services",
                "shared_sources",
            ],
            &["discovery_id"],
        )?;
        let project = string(&data["project_id"]);
        let config_path = string(&data["config_path"]);
        let discovery = if let Some(id) = data.get("discovery_id") {
            Some(crate::discovery::receipt(state, string(id), data)?)
        } else {
            None
        };
        let current = self.load(
            state,
            &object!({"project_id":project,"config_path":config_path}),
        )?;
        if data["revision"] != current["revision"] {
            return Err(RuntimeError::conflict(
                "System files changed. Reload editor before saving",
            ));
        }
        text(&data["name"], "system name", 200)?;
        for entry in items(&data["shared_sources"], MAX_ITEMS)? {
            source(entry)?;
        }
        let (root, config) = self.location(state, project, config_path)?;
        let mut files = vec![];
        let mut registry = vec![];
        let mut targets = BTreeSet::new();
        let mut size = 2usize;
        fn add(
            files: &mut Vec<EditFile>,
            targets: &mut BTreeSet<PathBuf>,
            size: &mut usize,
            root: &Path,
            name: &str,
            value: &Value,
            maximum: usize,
        ) -> Result<FileSnapshot> {
            let destination = root.join(name);
            if !targets.insert(destination) {
                return Err(fail("Two records would write same file"));
            }
            let raw = (infra_core::domain::json::serialize(value, false, false, true, false)?
                + "\n")
                .into_bytes();
            if raw.len() > maximum {
                return Err(fail("System metadata exceeds file size limit"));
            }
            let before = snapshot(root, name)?;
            let record = EditFile {
                root: root.to_path_buf(),
                name: name.into(),
                before: before.clone(),
                after: b64(&raw),
            };
            *size += encoded(&serde_json::to_value(&record)?)?.len() + 1;
            if *size > 8 * MAX_BYTES {
                return Err(fail("System edit too large; save fewer services"));
            }
            files.push(record);
            Ok(before)
        }
        for service in items(&data["services"], MAX_SERVICES)? {
            fields(
                service,
                &[
                    "project_id",
                    "folder",
                    "manifest",
                    "fingerprint",
                    "passport",
                ],
                &[],
            )?;
            let location =
                state.folder(string(&service["project_id"]), string(&service["folder"]))?;
            self.check_recovery(state, &location)?;
            let manifest_path = relative(string(&service["manifest"]))?;
            if !manifest_path.ends_with(".json") {
                return Err(fail("Service passports must be JSON files"));
            }
            catalog::manifest(&service["passport"], string(&service["passport"]["id"]))?;
            let prior = add(
                &mut files,
                &mut targets,
                &mut size,
                &location,
                &manifest_path,
                &service["passport"],
                MAX_SOURCE_BYTES,
            )?;
            if object!(&prior.sha256) != service["fingerprint"] {
                return Err(RuntimeError::conflict(
                    "Service passport changed; select folder again",
                ));
            }
            if let Some(body) = &prior.body {
                let previous = parse_json(&unb64(body)?, MAX_SOURCE_BYTES)?;
                catalog::manifest(&previous, string(&previous["id"]))?;
            }
            registry.push(object!({"id":service["passport"]["id"],"root":relative_location(&location,config.parent().ok_or_else(||fail("Configuration has no parent"))?)?,"manifest":manifest_path}));
        }
        add(
            &mut files,
            &mut targets,
            &mut size,
            &root,
            config_path,
            &object!({"schema_version":1,"name":data["name"],"services":registry,"shared_sources":data["shared_sources"]}),
            MAX_BYTES,
        )?;
        let proposed = files
            .iter()
            .map(|file| Ok((file.root.join(&file.name), unb64(&file.after)?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        let system = System::with_files(&config, &state.allowed()?, proposed)?;
        self.previews
            .retain(|_, preview| preview.created.elapsed() < Duration::from_secs(600));
        while !self.previews.is_empty()
            && (self.previews.len() >= 32
                || self
                    .previews
                    .values()
                    .map(|preview| preview.size)
                    .sum::<usize>()
                    + size
                    > 16 * MAX_BYTES)
        {
            let oldest = self
                .previews
                .iter()
                .min_by_key(|(_, preview)| preview.created)
                .map(|(key, _)| key.clone())
                .ok_or_else(|| fail("Preview queue lost"))?;
            self.previews.remove(&oldest);
        }
        let key = id32();
        let result =
            object!({"preview_id":key,"catalog":system.catalog(),"file_count":files.len()});
        self.previews.insert(
            key,
            Preview {
                created: Instant::now(),
                project_id: project.into(),
                config_path: config_path.into(),
                files,
                system,
                size,
                discovery,
            },
        );
        Ok(result)
    }
    pub fn apply(&mut self, state: &State, data: &Value) -> Result<Value> {
        fields(data, &["preview_id"], &[])?;
        let key = string(&data["preview_id"]);
        let preview = self
            .previews
            .remove(key)
            .ok_or_else(|| RuntimeError::conflict("Prepared save expired or already used"))?;
        if preview.created.elapsed() >= Duration::from_secs(600) {
            return Err(RuntimeError::conflict("Prepared save expired; save again"));
        }
        if state.host.active()? {
            return Err(RuntimeError::conflict(
                "Wait for active sessions before saving metadata",
            ));
        }
        self.location(state, &preview.project_id, &preview.config_path)?;
        let _locks = crate::execution::workspace_locks(
            &preview.system,
            &preview.system.services.keys().cloned().collect::<Vec<_>>(),
        )?;
        if let Some(discovery) = &preview.discovery {
            crate::discovery::check_fresh(discovery)?;
        }
        for file in &preview.files {
            if snapshot(&file.root, &file.name)? != file.before {
                return Err(RuntimeError::conflict(
                    "System files changed. Reload editor before saving",
                ));
            }
        }
        let edit_id = id32();
        state
            .db
            .put("system_edits", &edit_id, &object!({"files":preview.files}))?;
        let result = (|| -> Result<()> {
            for file in &preview.files {
                let after = unb64(&file.after)?;
                if file.before.sha256.as_deref() == Some(digest(&after).as_str()) {
                    continue;
                }
                replace(
                    &file.root,
                    &file.name,
                    Some(&after),
                    &file.before,
                    file.before.mode,
                )?;
            }
            state.db.delete("system_edits", &edit_id)
        })();
        if result.is_err() {
            self.recover(state, Some(&edit_id), true)?;
            return Err(RuntimeError::conflict(
                if state.db.get("system_edits", &edit_id)?.is_some() {
                    "Saving stopped; recovery conflicts with external changes. Inspect service folders."
                } else {
                    "Saving stopped. Prior metadata restored; reload editor before saving again."
                },
            ));
        }
        let loaded = self.load(
            state,
            &object!({"project_id":preview.project_id,"config_path":preview.config_path}),
        )?;
        let (root, config) = self.location(state, &preview.project_id, &preview.config_path)?;
        let system = System::new(&config, &state.allowed()?)?;
        Ok(
            object!({"editor":loaded,"catalog":system.catalog(),"diagram":system.diagram(),"root":root}),
        )
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}
fn relative_location(path: &Path, base: &Path) -> Result<String> {
    let first = path.components().collect::<Vec<_>>();
    let second = base.components().collect::<Vec<_>>();
    let common = first
        .iter()
        .zip(&second)
        .take_while(|(a, b)| a == b)
        .count();
    if common == 0 {
        return path_string(path);
    }
    let mut result = vec!["..".to_string(); second.len() - common];
    result.extend(
        first[common..]
            .iter()
            .map(|part| part.as_os_str().to_string_lossy().to_string()),
    );
    Ok(if result.is_empty() {
        ".".into()
    } else {
        result.join("/")
    })
}
fn workspace_locks_for_roots(roots: &[PathBuf]) -> Result<Vec<crate::execution::Lock>> {
    let mut ids = BTreeSet::new();
    #[cfg(unix)]
    let user = unsafe { libc::getuid() }.to_string();
    #[cfg(not(unix))]
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
    let directory = std::env::temp_dir().join(format!("ai-system-locks-{user}"));
    crate::native::private_directory(&directory)?;
    for root in roots {
        let identity = infra_core::git::run(
            root,
            &infra_core::git::strings(&["rev-parse", "--show-toplevel"]),
            None,
            Duration::from_secs(10),
            16384,
        )
        .ok()
        .filter(|result| result.code == 0)
        .map(|result| result.stdout)
        .unwrap_or(path_string(root)?.into_bytes());
        ids.insert(digest(&identity));
    }
    ids.into_iter()
        .map(|id| crate::execution::lock(&directory, &format!("{id}.lock")))
        .collect()
}

#[cfg(all(test, windows))]
mod missing_file_mode_tests {
    use super::*;
    #[test]
    fn interrupted_new_file_uses_the_same_mode_projection_as_recovery() {
        let root = std::env::temp_dir().join(format!("infra-system-mode-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let root = infra_core::fs::canonicalize(root).unwrap();
        let before = snapshot(&root, "new-system.json").unwrap();
        assert_eq!(before.mode, 0o644);
        assert_eq!(before.body, None);
        replace(&root, "new-system.json", Some(b"{}"), &before, before.mode).unwrap();
        let published = snapshot(&root, "new-system.json").unwrap();
        assert_eq!(
            published.mode, before.mode,
            "recovery mode guard rejects its own new writable file"
        );
        assert_eq!(published.parent, before.parent);
        // This is the exact partial-save rollback used by Editor::recover.
        replace(&root, "new-system.json", None, &published, before.mode).unwrap();
        assert_eq!(snapshot(&root, "new-system.json").unwrap(), before);
        std::fs::remove_dir(&root).unwrap();
    }
}
