//! Native project knowledge access. Project scripts are never imported or executed.
use crate::{
    HarnessError, Result,
    services::{Domain, Payload, Reply},
    store::SessionStore,
};
use core::bank::publication::PublicationFiles;
use infra_core::{
    self as core, BankService, BrainRepository, Roots, RuntimeConfig, RuntimeContext,
    application::MemoryService,
};
use rusqlite::OptionalExtension;
use serde_json::{Value as V, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io::{Cursor, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use uuid::Uuid;
const BANKS: &[(&str, &str)] = &[
    ("memory-bank", "Project memory"),
    ("Laravel/memory-bank", "Laravel"),
    ("Symfony/memory-bank", "Symfony"),
    ("PHP Core/memory-bank", "PHP Core"),
    ("Cms/wordpress/memory-bank", "WordPress"),
];
const CONTEXT: &[&str] = &[
    "AGENTS.md",
    "CLAUDE.md",
    "README.md",
    "project-brain/README.md",
    "specs/MANIFEST.md",
];
const FOLDERS: &[&str] = &[
    "dynamic/tasks",
    "dynamic/findings",
    "dynamic/bugs",
    "dynamic/incidents",
    "dynamic/decisions",
    "dynamic/events",
    "control/handoffs",
    "control/promotions",
    "control/retrieval-manifests",
    "archive/task",
    "archive/finding",
    "archive/bug",
    "archive/incident",
    "archive/decision",
    "archive/event",
    "archive/handoffs",
];
const READ_LIMIT: usize = 256 * 1024;
const EXPORT_LIMIT: usize = 32 * 1024 * 1024;
type ExportCache = VecDeque<(String, Vec<u8>)>;
#[derive(Clone)]
pub struct KnowledgeDomain {
    pub store: SessionStore,
    pub source: PathBuf,
    operation: Arc<Mutex<()>>,
    exports: Arc<Mutex<ExportCache>>,
}
impl KnowledgeDomain {
    pub fn new(store: SessionStore, source: PathBuf) -> Self {
        Self {
            store,
            source,
            operation: Arc::new(Mutex::new(())),
            exports: Arc::new(Mutex::new(VecDeque::new())),
        }
    }
    pub fn context(
        &self,
        project_id: &str,
        workspace_override: Option<&Path>,
    ) -> Result<RuntimeContext> {
        let _source_frame = core::git::source_frame_scope();
        let project = self.store.project(project_id)?;
        let registered = PathBuf::from(required(&project, "path", 32768)?);
        std::fs::metadata(&registered)
            .map_err(|_| HarnessError::missing("The registered project is unavailable."))?;
        let registered = core::fs::python_realpath(&registered)
            .map_err(|_| HarnessError::missing("The registered project is unavailable."))?;
        let workspace = if let Some(workspace) = workspace_override {
            std::fs::metadata(workspace)
                .map_err(|_| HarnessError::missing("The session workspace is unavailable."))?;
            let path = core::fs::python_realpath(workspace)
                .map_err(|_| HarnessError::missing("The session workspace is unavailable."))?;
            if path != registered {
                let a = core::git::GitFrame::discover(&registered)?.ok_or_else(|| {
                    HarnessError::forbidden("Session workspace is outside the registered project.")
                })?;
                let b = core::git::GitFrame::discover(&path)?.ok_or_else(|| {
                    HarnessError::forbidden("Session workspace is outside the registered checkout.")
                })?;
                if a.common_dir()? != b.common_dir()? || a.prefix_bytes != b.prefix_bytes {
                    return Err(HarnessError::forbidden(
                        "The session workspace belongs to another project.",
                    ));
                }
            }
            path
        } else {
            registered.clone()
        };
        let mut local = PublicationFiles::new(&workspace)?;
        let installed = local
            .read_prefix("memory-bank/runtime-contract.json", 65536)?
            .is_some()
            || local
                .read_prefix("memory-bank/bin/infra-engine", 4)?
                .is_some()
            || local
                .read_prefix("memory-bank/bin/infra-engine.exe", 4)?
                .is_some();
        let edition = if installed {
            None
        } else {
            let db = self.store.connection()?;
            db.query_row(
                "SELECT edition FROM project_accelerators WHERE project_id=?1",
                [project_id],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(|_| {
                HarnessError::unavailable("Accelerator attachment state is unavailable.")
            })?
        };
        let roots = if let Some(edition) = edition {
            let relative = match edition.as_str() {
                "Laravel" | "laravel" => "Laravel",
                "Symfony" | "symfony" => "Symfony",
                "PHP Core" | "php-core" | "php" => "PHP Core",
                "Cms/wordpress" | "WordPress" | "wordpress" => "Cms/wordpress",
                _ => {
                    return Err(HarnessError::invalid(
                        "The attached accelerator edition is unsupported.",
                    ));
                }
            };
            let tooling = core::fs::python_realpath(&self.source.join(relative))?;
            PublicationFiles::new(&tooling)?;
            let state =
                core::fs::python_realpath(&self.store.state.join("attached").join(project_id))?;
            Roots {
                tooling,
                project: workspace,
                state,
                shared: false,
            }
        } else {
            Roots {
                tooling: workspace.clone(),
                project: workspace.clone(),
                state: workspace,
                shared: false,
            }
        };
        let config = RuntimeConfig::load(&roots)?;
        let mode = config.mode(None)?;
        let actor = std::env::var("PROJECT_BRAIN_OWNER")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "local".into());
        Ok(RuntimeContext::new(roots, actor, mode)?)
    }
    fn selected(
        &self,
        project_id: &str,
        bank: Option<&str>,
        workspace: Option<&Path>,
    ) -> Result<(RuntimeContext, V)> {
        let mut context = self.context(project_id, workspace)?;
        let mut found = vec![];
        for (name, label) in BANKS {
            let path = context.roots.state.join(name);
            if let Ok(true) = core::fs::admit_directory(&path) {
                let mut files = PublicationFiles::new(&context.roots.state)?;
                files.read_prefix(&format!("{name}/.custody-probe"), 0)?;
                found.push(json!({"id":name,"path":name,"name":label}));
            }
        }
        let selected = if let Some(bank) = bank {
            relative(bank)?;
            if !found.iter().any(|v| v["id"] == bank) {
                return Err(HarnessError::missing("Memory bank is unavailable."));
            }
            Some(bank.to_owned())
        } else {
            found
                .first()
                .and_then(|v| v["id"].as_str().map(str::to_owned))
        };
        let root = selected
            .as_ref()
            .and_then(|v| Path::new(v).parent())
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(""))
            .to_path_buf();
        let attached = context.roots.attached();
        if !root.as_os_str().is_empty() {
            context.roots.state = context.roots.state.join(&root);
            context.roots.tooling = context.roots.tooling.join(&root);
            if !attached {
                context.roots.project = context.roots.project.join(&root);
            }
        }
        let available = selected.is_some()
            && (context
                .roots
                .tooling
                .join("memory-bank/runtime-contract.json")
                .is_file()
                || context
                    .roots
                    .tooling
                    .join("memory-bank/bin/infra-engine")
                    .is_file()
                || context
                    .roots
                    .tooling
                    .join("memory-bank/bin/infra-engine.exe")
                    .is_file());
        let brain = core::fs::admit_directory(&context.roots.brain_root()).unwrap_or(false);
        let info = json!({"project_id":project_id,"banks":found,"bank_id":selected,"runtime_available":available,"brain_available":brain,"root":root.to_string_lossy().replace('\\',"/"),"mode":if selected.is_some(){json!(context.mode)}else{V::Null},"attached":if attached{json!({"home":context.roots.tooling,"state":context.roots.state})}else{V::Null}});
        Ok((context, info))
    }
    pub fn bound_context(&self, session: &V) -> Result<RuntimeContext> {
        let project = required(session, "project_id", 200)?;
        let workspace = PathBuf::from(required(session, "project_path", 32768)?);
        let (context, info) =
            self.selected(project, session["brain"]["bank"].as_str(), Some(&workspace))?;
        if info["runtime_available"] != true || context.mode != "governed" {
            return Err(HarnessError::invalid(
                "Automatic memory requires an installed governed native runtime.",
            ));
        }
        self.preflight(&context)?;
        Ok(context)
    }
    pub fn prepared_sources(
        &self,
        context: &RuntimeContext,
        capsule: &V,
        task: &V,
        provenance: Option<&V>,
        allow_local: bool,
    ) -> Result<V> {
        prepared_sources(context, capsule, task, provenance, allow_local)
    }
    pub fn capsule_material(&self, capsule: &V) -> V {
        capsule_material(capsule)
    }
    pub fn automatic_scope(&self, session: &V, context: &RuntimeContext) -> Result<V> {
        let project = required(session, "project_id", 200)?;
        let workspace = PathBuf::from(required(session, "project_path", 32768)?);
        std::fs::metadata(&workspace)
            .map_err(|_| HarnessError::unavailable("The session workspace is unavailable."))?;
        let workspace = core::fs::python_realpath(&workspace)
            .map_err(|_| HarnessError::unavailable("The session workspace is unavailable."))?;
        let base = self.context(project, Some(&workspace))?;
        let absolute = |path: &Path| -> Result<PathBuf> { Ok(core::fs::python_realpath(path)?) };
        let attached = base.roots.attached();
        Ok(
            json!({"project":project,"workspace":workspace,"bank":session["brain"]["bank"],"task":session["brain"]["record_id"],"folder":absolute(&base.roots.state)?,"sources":absolute(&base.roots.project)?,"scripts":if attached{json!(absolute(&base.roots.tooling.join("memory-bank/scripts"))?)}else{V::Null},"owner":context.actor}),
        )
    }
    pub fn with_automatic_operation<T>(
        &self,
        session: &V,
        work: impl FnOnce(&RuntimeContext) -> Result<T>,
    ) -> Result<T> {
        let _source_frame = core::git::source_frame_scope();
        let _guard = self.operation.try_lock().map_err(|_| knowledge_busy())?;
        let context = self.bound_context(session)?;
        work(&context)
    }
    pub fn info(&self, project_id: &str, bank: Option<&str>) -> Result<V> {
        Ok(self.selected(project_id, bank, None)?.1)
    }
    pub fn project_context(&self, project_id: &str) -> Result<V> {
        let project = self.store.project(project_id)?;
        let root = PathBuf::from(required(&project, "path", 32768)?);
        let mut files = PublicationFiles::new(&root)?;
        let mut output = vec![];
        for name in CONTEXT {
            let content = files.read_prefix(name, 0)?;
            output.push(json!({"path":name,"exists":content.is_some(),"bytes":content.map(|x|x.0).unwrap_or(0)}));
        }
        let available = output.iter().any(|v| v["exists"] == true);
        Ok(json!({"project_id":project_id,"files":output,"context_available":available}))
    }
    pub fn read(
        &self,
        project_id: &str,
        bank: Option<&str>,
        document: Option<&str>,
        brain: bool,
    ) -> Result<V> {
        let (context, mut info) = self.selected(project_id, bank, None)?;
        let mut files = PublicationFiles::new(&context.roots.state)?;
        let base = if brain {
            "project-brain"
        } else {
            "memory-bank"
        };
        if let Some(document) = document {
            relative(document)?;
            let parts = document.split('/').collect::<Vec<_>>();
            let valid = if brain {
                FOLDERS.contains(&parts[..parts.len() - 1].join("/").as_str())
                    && matches!(
                        Path::new(document).extension().and_then(|s| s.to_str()),
                        Some("md" | "json")
                    )
            } else {
                (parts.len() == 1 || parts.len() == 2 && ["chunks", "local"].contains(&parts[0]))
                    && document.ends_with(".md")
                    && !parts.last().unwrap_or(&"").starts_with('.')
            };
            if !valid {
                return Err(HarnessError::invalid(
                    "Select a supported knowledge document.",
                ));
            }
            let (size, raw) = files
                .read_prefix(&format!("{base}/{document}"), READ_LIMIT)?
                .ok_or_else(|| HarnessError::missing("Knowledge document is unavailable."))?;
            let content = String::from_utf8_lossy(&raw).replace("\r\n", "\n");
            let meta = metadata(&content, document);
            merge(&mut info, entry(document, size, &content, brain));
            info["content"] = json!(content);
            info["metadata"] = meta;
            info["truncated"] = json!(size > READ_LIMIT as u64);
            return Ok(info);
        }
        let folders = if brain {
            FOLDERS.to_vec()
        } else {
            vec!["chunks", "", "local"]
        };
        let (mut scanned, mut consumed, mut truncated) = (0, 0usize, false);
        let mut entries = vec![];
        for folder in folders {
            let path = if folder.is_empty() {
                base.into()
            } else {
                format!("{base}/{folder}")
            };
            for name in names(&context.roots.state, &path, &mut files)? {
                scanned += 1;
                if scanned > 5000 || entries.len() >= 500 || brain && consumed >= 8 * 1024 * 1024 {
                    truncated = true;
                    break;
                }
                if name.starts_with('.')
                    || if brain {
                        !name.ends_with(".md") && !name.ends_with(".json")
                    } else {
                        !name.ends_with(".md")
                    }
                {
                    continue;
                }
                let document = if folder.is_empty() {
                    name
                } else {
                    format!("{folder}/{name}")
                };
                if let Some((size, bytes)) = files.read_prefix(
                    &format!("{base}/{document}"),
                    if brain { 65536 } else { 8192 },
                )? {
                    consumed += bytes.len();
                    entries.push(entry(
                        &document,
                        size,
                        &String::from_utf8_lossy(&bytes),
                        brain,
                    ));
                }
            }
            if truncated {
                break;
            }
        }
        entries.sort_by_key(|v| {
            (
                if brain {
                    v["category"].as_str().unwrap_or("").to_owned()
                } else {
                    if v["kind"] == "chunk" {
                        "0".into()
                    } else {
                        "1".into()
                    }
                },
                v["title"].as_str().unwrap_or("").to_lowercase(),
                v["path"].as_str().unwrap_or("").to_owned(),
            )
        });
        info["entries"] = json!(entries);
        info["truncated"] = json!(truncated);
        Ok(info)
    }
    fn preflight(&self, context: &RuntimeContext) -> Result<()> {
        let mut files = PublicationFiles::new(&context.roots.state)?;
        let mut queue = VecDeque::from(["memory-bank".to_owned(), "project-brain".to_owned()]);
        let mut count = 0;
        while let Some(path) = queue.pop_front() {
            let absolute = context.roots.state.join(&path);
            if !core::fs::admit_directory(&absolute)? {
                continue;
            }
            for name in names(&context.roots.state, &path, &mut files)? {
                count += 1;
                if count > 20000 {
                    return Err(HarnessError::invalid(
                        "Knowledge storage exceeds the supported file count.",
                    ));
                }
                let relative = format!("{path}/{name}");
                let meta = std::fs::symlink_metadata(context.roots.state.join(&relative))
                    .map_err(|_| HarnessError::conflict("Knowledge storage changed."))?;
                if core::fs::linked(&meta) {
                    return Err(HarnessError::forbidden("Knowledge storage contains links."));
                }
                if meta.is_dir() {
                    queue.push_back(relative);
                } else if !meta.is_file() || files.image_identity(&relative)?.is_none() {
                    return Err(HarnessError::forbidden(
                        "Knowledge storage contains unsupported file types.",
                    ));
                }
            }
        }
        files.check()?;
        Ok(())
    }
    pub fn run(
        &self,
        project_id: &str,
        data: &V,
        workspace: Option<&Path>,
        person: bool,
    ) -> Result<V> {
        let _source_frame = core::git::source_frame_scope();
        let action = required(data, "action", 80)?;
        validate_action(action, data)?;
        let _guard = self.operation.try_lock().map_err(|_| knowledge_busy())?;
        let (context, info) = self.selected(project_id, data["bank"].as_str(), workspace)?;
        if info["runtime_available"] != true {
            return Err(HarnessError::missing(
                "The selected knowledge root has no installed native runtime.",
            ));
        }
        if [
            "start",
            "brain-create",
            "brain-update",
            "complete",
            "rebind",
            "promote-propose",
            "promote-review",
            "promote-apply",
            "promote-auto",
            "record-result",
            "prepare-result",
            "replay-result",
        ]
        .contains(&action)
            && context.mode != "governed"
        {
            return Err(HarnessError::invalid(
                "Project Brain editing requires governed mode.",
            ));
        }
        self.preflight(&context)?;
        let mut request = data.clone();
        request
            .as_object_mut()
            .expect("validated object")
            .remove("action");
        request
            .as_object_mut()
            .expect("validated object")
            .remove("bank");
        if person && ["brain-create", "brain-update"].contains(&action) {
            request["attestation"] = json!("person");
        }
        if action == "search" {
            request["limit"] = json!(20);
        }
        if ["retrieve", "refresh"].contains(&action) {
            request["limit"] = json!(3);
        }
        if action == "refresh" && !request["host"].is_string() {
            request["capsule_profile"] = json!("full");
        }
        if ["promote-propose", "promote-apply"].contains(&action) {
            self.promotion_admission(&context, data)?;
        }
        let temp = if action == "export" {
            Some(
                tempfile::Builder::new()
                    .prefix("knowledge-export-")
                    .tempdir_in(&self.store.state)
                    .map_err(|_| HarnessError::unavailable("Could not stage knowledge export."))?,
            )
        } else {
            None
        };
        if let Some(temp) = temp.as_ref() {
            request["destination"] = json!(temp.path());
        }
        request["json"] = json!(true);
        let result = MemoryService::new(context.clone(), None).execute(action, &request, &[]);
        match result {
            Ok(result) => {
                let mut response = json!({"ok":true,"action":action,"result":result});
                if let Some(temp) = temp.as_ref() {
                    let bytes = package_export(temp.path())?;
                    let id = Uuid::new_v4().to_string();
                    let mut cache = self
                        .exports
                        .lock()
                        .map_err(|_| HarnessError::unavailable("Export cache unavailable."))?;
                    while cache.len() >= 8 {
                        cache.pop_front();
                    }
                    cache.push_back((id.clone(), bytes));
                    response["download_id"] = json!(id);
                    if let Some(result) = response["result"].as_object_mut() {
                        result.remove("destination");
                    }
                }
                Ok(response)
            }
            Err(error) => {
                let code = error.code.clone();
                let safe = match error.kind {
                    core::ErrorKind::Conflict => {
                        "This record changed. Reload it and use its current revision."
                    }
                    core::ErrorKind::Unauthorized => {
                        "The configured Project Brain owner is not authorized for this record."
                    }
                    core::ErrorKind::Policy => {
                        "The runtime refused the operation under its source, privacy or lifecycle policy."
                    }
                    _ => {
                        "The native context runtime could not finish this operation. Refresh before retrying; changes may have been applied."
                    }
                };
                let mut response = json!({"ok":false,"action":action,"result":null,"error":safe});
                if let Some(code) = code {
                    response["error_code"] = json!(code.trim_start_matches("memory-result-v1:"));
                }
                Ok(response)
            }
        }
    }
    fn promotion_admission(&self, context: &RuntimeContext, data: &V) -> Result<()> {
        let bank = BankService::new(context.clone());
        let brain = BrainRepository::new(context.clone())?;
        let ids = if let Some(id) = data["promotion_id"].as_str() {
            let proposal = core::bank::promotion::list(&bank, true)?
                .into_iter()
                .find(|v| v["id"] == id)
                .ok_or_else(|| HarnessError::missing("Promotion unavailable."))?;
            proposal["source_records"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|r| r["id"].as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        } else {
            string_list(&data["source_ids"], 20, "source IDs")?
        };
        if ids.is_empty() {
            return Err(HarnessError::invalid("Select eligible source records."));
        }
        for id in ids {
            let record = brain.find(&id, None, true)?.record;
            if !promotable(context, &record)? {
                return Err(HarnessError::forbidden(
                    "Only verified resolved reusable records with fresh sources and allowed privacy can be promoted.",
                ));
            }
        }
        Ok(())
    }
    pub fn inspect(
        &self,
        project_id: &str,
        bank: Option<&str>,
        workspace: Option<&Path>,
        task: Option<&str>,
        task_only: bool,
    ) -> Result<V> {
        let _source_frame = core::git::source_frame_scope();
        let (context, mut info) = self.selected(project_id, bank, workspace)?;
        if info["runtime_available"] != true || context.mode != "governed" {
            return Err(HarnessError::invalid(
                "An installed governed native Brain runtime is required.",
            ));
        }
        self.preflight(&context)?;
        let brain = BrainRepository::new(context.clone())?;
        let current = if let Some(task) = task {
            core::serving::TaskService::new(context.clone(), None).task(task, false)?
        } else {
            V::Null
        };
        if task_only {
            info["task"] = current;
            return Ok(info);
        }
        let records = brain.records(true)?;
        if records.len() > 500 {
            return Err(HarnessError::invalid(
                "Too many Brain records for this view.",
            ));
        }
        let bank = BankService::new(context.clone());
        let proposals = core::bank::promotion::list(&bank, true)?;
        if proposals.len() > 500 {
            return Err(HarnessError::invalid("Too many promotions for this view."));
        }
        let promoted = proposals
            .iter()
            .filter(|p| p["status"] == "applied")
            .flat_map(|p| {
                p["source_records"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|r| r["id"].as_str().map(str::to_owned))
            })
            .collect::<BTreeSet<_>>();
        let mut eligible = vec![];
        let mut all = vec![];
        for image in records {
            let r = image.record;
            if !promoted.contains(r["id"].as_str().unwrap_or("")) && promotable(&context, &r)? {
                let mut candidate = r.clone();
                candidate["promotion_content"] = json!(core::bank::promotion::content(&r));
                eligible.push(candidate);
            }
            all.push(r);
        }
        let proposals = proposals
            .into_iter()
            .map(|mut p| {
                if let Some(o) = p.as_object_mut() {
                    o.remove("publication");
                }
                p
            })
            .collect::<Vec<_>>();
        merge(
            &mut info,
            json!({"task":current,"records":all,"eligible_sources":eligible,"promotions":proposals,"automatic_promotion":RuntimeConfig::load(&context.roots)?.enabled("automatic_promotion")}),
        );
        Ok(info)
    }
    pub fn prepare(&self, session: &V, query: &str) -> Result<V> {
        let _source_frame = core::git::source_frame_scope();
        self.prepare_internal(session, query, None)
    }
    pub fn prepare_guarded(
        &self,
        session: &V,
        query: &str,
        generation: &str,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<V> {
        self.prepare_internal(session, query, Some((generation, cancel)))
    }
    fn prepare_internal(
        &self,
        session: &V,
        query: &str,
        guard: Option<(&str, &std::sync::atomic::AtomicBool)>,
    ) -> Result<V> {
        if guard.is_some_and(|(_, cancel)| cancel.load(std::sync::atomic::Ordering::Acquire)) {
            return Err(HarnessError::conflict("Cancellation requested."));
        }
        let _guard = self.operation.try_lock().map_err(|_| knowledge_busy())?;
        text(query, "query", 4000)?;
        let project = required(session, "project_id", 200)?;
        let workspace = PathBuf::from(required(session, "project_path", 32768)?);
        let mut brain = session["brain"].clone();
        if !brain.is_object() {
            return Err(HarnessError::invalid(
                "This session has no linked Brain task.",
            ));
        }
        brain["query"] = json!(query);
        let (context, info) = self.selected(project, brain["bank"].as_str(), Some(&workspace))?;
        if info["runtime_available"] != true || context.mode != "governed" {
            return Err(HarnessError::invalid(
                "The linked task requires an installed governed runtime.",
            ));
        }
        let task = required(&brain, "task_id", 200)?.to_owned();
        let service = MemoryService::new(context.clone(), None);
        let binding = if brain["create"] == true {
            service.execute(
                "start",
                &json!({"task_id":task,"goal":required(&brain,"goal",16000)?}),
                &[],
            )?
        } else {
            service.execute(
                "rebind",
                &json!({"task_id":task,"record_id":brain["record_id"]}),
                &[],
            )?
        };
        let id = binding["task_uuid"].as_str().ok_or_else(|| {
            HarnessError::invalid("The runtime did not return the linked task identity.")
        })?;
        if !core::domain::is_uuid4(id) {
            return Err(HarnessError::invalid(
                "The runtime returned an invalid task identity.",
            ));
        }
        brain["create"] = json!(false);
        brain["record_id"] = json!(id);
        brain["approved"] = json!(false);
        brain["context_id"] = V::Null;
        brain["capsule"] = V::Null;
        let sid = required(session, "id", 100)?;
        let current = self.store.get(sid)?;
        if current["brain"]["task_id"] != task {
            return Err(HarnessError::conflict(
                "The linked task changed during preparation.",
            ));
        }
        brain["query"] = current["brain"]["query"]
            .as_str()
            .map(|s| json!(s))
            .unwrap_or_else(|| json!(query));
        if let Some((generation, cancel)) = guard {
            self.store
                .patch_guarded(sid, generation, &json!({"brain":brain}), cancel)?;
        } else {
            self.store.patch(sid, &json!({"brain":brain}))?;
        }
        let current = core::serving::TaskService::new(context.clone(), None).task(id, true)?;
        let automatic = brain["review"] == false;
        let refreshed=service.execute(if automatic{"refresh"}else{"retrieve"},&json!({"query":brain["query"],"task_id":id,"host":if automatic{session["provider"].clone()}else{json!("cli")},"gate":"off","sanitize":automatic,"ephemeral":automatic,"provenance_response":automatic,"limit":3,"capsule_profile":if automatic{"recall"}else{"full"},"json":true}),&[])?;
        let capsule = refreshed["capsule"]
            .as_object()
            .map(|_| refreshed["capsule"].clone())
            .unwrap_or_else(|| refreshed.clone());
        if !capsule.is_object() {
            return Err(HarnessError::invalid(
                "The runtime did not return a valid task capsule.",
            ));
        }
        if guard.is_some_and(|(_, cancel)| cancel.load(std::sync::atomic::Ordering::Acquire)) {
            return Err(HarnessError::conflict("Cancellation requested."));
        }
        brain["context_id"] = json!(Uuid::new_v4().to_string());
        brain["capsule"] = capsule.clone();
        brain["capsule_text"] = refreshed["capsule_text"].clone();
        brain["task"] = current;
        brain["source_hashes"] = prepared_sources(
            &context,
            &capsule,
            &brain["task"],
            refreshed.get("source_provenance"),
            automatic,
        )?;
        brain["approved"] = json!(false);
        Ok(brain)
    }
    pub fn verify_prepared(&self, session: &V) -> Result<()> {
        let _source_frame = core::git::source_frame_scope();
        let _guard = self.operation.try_lock().map_err(|_| knowledge_busy())?;
        let brain = &session["brain"];
        if !brain["context_id"]
            .as_str()
            .map(core::domain::is_uuid4)
            .unwrap_or(false)
            || !brain["capsule"].is_object()
            || !brain["task"].is_object()
        {
            return Err(HarnessError::invalid(
                "Prepare and review the linked task context before running.",
            ));
        }
        let project = required(session, "project_id", 200)?;
        let workspace = PathBuf::from(required(session, "project_path", 32768)?);
        let (context, _) = self.selected(project, brain["bank"].as_str(), Some(&workspace))?;
        let id = brain["record_id"]
            .as_str()
            .or_else(|| brain["task_id"].as_str())
            .ok_or_else(|| HarnessError::invalid("The linked task identity is unavailable."))?;
        let task = core::serving::TaskService::new(context.clone(), None).task(id, true)?;
        if task["id"] != brain["task"]["id"]
            || task["revision"] != brain["task"]["revision"]
            || task["external_id"] != brain["task_id"]
        {
            return Err(HarnessError::conflict(
                "The linked task changed. Prepare and approve its context again.",
            ));
        }
        if matches!(task["status"].as_str(), Some("completed" | "cancelled")) {
            return Err(HarnessError::conflict(
                "The linked task is terminal. Prepare another task.",
            ));
        }
        let refreshed=MemoryService::new(context.clone(),None).execute("retrieve",&json!({"task_id":id,"query":brain["query"],"ephemeral":true,"host":"cli","gate":"off","limit":3,"json":true,"provenance_response":true}),&[])?;
        let capsule = if refreshed["response_profile"] == "local-source-provenance-v1" {
            &refreshed["capsule"]
        } else {
            &refreshed
        };
        let hashes = prepared_sources(
            &context,
            capsule,
            &task,
            refreshed.get("source_provenance"),
            true,
        )?;
        if capsule_material(capsule) != capsule_material(&brain["capsule"])
            || hashes != brain["source_hashes"]
        {
            return Err(HarnessError::conflict(
                "Task context or selected sources changed. Prepare and approve the updated context.",
            ));
        }
        Ok(())
    }
    pub fn brain_action(&self, session: &V, data: &V) -> Result<V> {
        if ["queued", "running"].contains(&session["status"].as_str().unwrap_or("")) {
            return Err(HarnessError::conflict(
                "Wait for this session to finish before editing its Brain.",
            ));
        }
        let action = required(data, "action", 80)?;
        if ![
            "brain-update",
            "rebind",
            "complete",
            "brain-create",
            "promote-propose",
            "promote-review",
            "promote-apply",
        ]
        .contains(&action)
            || data.get("bank").is_some()
        {
            return Err(HarnessError::invalid(
                "Select a supported linked-task operation.",
            ));
        }
        if action == "complete" && session["status"] != "completed" {
            return Err(HarnessError::conflict(
                "Finish the session before recording its task outcome.",
            ));
        }
        let brain = &session["brain"];
        let mut fields = data.clone();
        fields["bank"] = brain["bank"].clone();
        if action == "brain-update" && fields.get("record_id").is_none() {
            fields["record_id"] = brain["record_id"].clone();
        }
        if ["complete", "rebind"].contains(&action) {
            if fields["task_id"]
                .as_str()
                .is_some_and(|id| Some(id) != brain["task_id"].as_str())
            {
                return Err(HarnessError::forbidden(
                    "This operation must use the linked task identity.",
                ));
            }
            fields["task_id"] = brain["task_id"].clone();
        }
        if action == "rebind" {
            if fields["record_id"]
                .as_str()
                .is_some_and(|id| Some(id) != brain["record_id"].as_str())
            {
                return Err(HarnessError::forbidden(
                    "The linked task record cannot change.",
                ));
            }
            fields["record_id"] = brain["record_id"].clone();
        }
        let workspace = PathBuf::from(required(session, "project_path", 32768)?);
        self.run(
            required(session, "project_id", 200)?,
            &fields,
            Some(&workspace),
            true,
        )
    }
    pub fn save_memory(&self, session: &V, data: &V) -> Result<V> {
        let _source_frame = core::git::source_frame_scope();
        if ["queued", "running"].contains(&session["status"].as_str().unwrap_or("")) {
            return Err(HarnessError::conflict(
                "Wait for this session to finish before saving memory.",
            ));
        }
        strict(
            data,
            &[
                "progress",
                "next_steps",
                "learnings",
                "verified",
                "used_memory",
                "capture_decision",
            ],
        )?;
        let progress = data["progress"].as_str().unwrap_or("");
        if progress.chars().count() > 1000 {
            return Err(HarnessError::invalid(
                "Write progress in at most1000 characters.",
            ));
        }
        let steps = if data.get("next_steps").is_some() {
            string_list(&data["next_steps"], 3, "next steps")?
        } else {
            vec![]
        };
        if steps.iter().any(|s| s.chars().count() > 300) {
            return Err(HarnessError::invalid(
                "Write next steps in at most300 characters.",
            ));
        }
        let learnings = data["learnings"].as_array().cloned().unwrap_or_default();
        if learnings.len() > 3 || !learnings.is_empty() && data["verified"] != true {
            return Err(HarnessError::invalid(
                "Keep at most3 source-checked learnings and attest verified=true.",
            ));
        }
        let workspace = PathBuf::from(required(session, "project_path", 32768)?);
        let (context, _) = self.selected(
            required(session, "project_id", 200)?,
            session["brain"]["bank"].as_str(),
            Some(&workspace),
        )?;
        let service = core::ResultService::new(context.clone());
        let mut request =
            json!({"learnings":learnings,"verified":data["verified"],"next_steps":steps});
        if !progress.trim().is_empty() {
            request["progress"] = json!(progress);
        }
        if data.get("capture_decision").is_some() {
            request["capture_decision"] = data["capture_decision"].clone();
        }
        let normalized = service.normalize(&request, true)?;
        for item in &normalized.learnings {
            if item.title.chars().count() > 200 || item.consequence.chars().count() > 1000 {
                return Err(HarnessError::invalid(
                    "Learning title or consequence exceeds the draft limits.",
                ));
            }
        }
        let id = session["brain"]["record_id"]
            .as_str()
            .or_else(|| session["brain"]["task_id"].as_str())
            .ok_or_else(|| {
                HarnessError::invalid("Prepare the linked task before saving memory.")
            })?;
        let task = core::serving::TaskService::new(context, None).task(id, true)?;
        let result_id = format!(
            "harness-{}-{}",
            required(session, "id", 100)?,
            Uuid::new_v4().simple()
        );
        let result_id = result_id.chars().take(64).collect::<String>();
        let fields = json!({"action":"record-result","bank":session["brain"]["bank"],"task_id":id,"result_id":result_id,"revision":task["revision"],"request":request,"attestation":"person","reason":"Saved from a reviewed Harness session"});
        let result = self.run(
            required(session, "project_id", 200)?,
            &fields,
            Some(&workspace),
            true,
        )?;
        Ok(json!({"ok":result["ok"],"saved":result["result"],"skipped":[],"error":result["error"]}))
    }
    fn usage(&self, project: &str, bank: Option<&str>) -> Result<V> {
        let (context, info) = self.selected(project, bank, None)?;
        let mut payload = json!({"project_id":project,"banks":info["banks"],"bank_id":info["bank_id"],"root":info["root"],"mode":info["mode"],"brain_available":info["brain_available"],"check_available":false,"today":core::bank::codec::today().to_string(),"chunks":null,"brain":null,"promotions":null,"retrievals":null,"health":null,"history":null});
        if info["bank_id"].is_null() {
            return Ok(payload);
        }
        let mut files = PublicationFiles::new(&context.roots.state)?;
        let chunks = usage_chunks(&context, &mut files)?;
        let chunk_paths = chunks["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| Some((v["path"].as_str()?.to_owned(), v["id"].as_str()?.to_owned())))
            .collect::<BTreeMap<_, _>>();
        if info["brain_available"] == true && info["mode"] == "governed" {
            let (promotions, promoted) = usage_promotions(&context, &mut files)?;
            let records = usage_records(&context, &mut files, &promoted)?;
            payload["brain"] = records;
            payload["promotions"] = promotions;
        }
        let tasks = {
            let db = self.store.connection()?;
            let mut q = db
                .prepare("SELECT brain FROM sessions WHERE project_id=? AND brain IS NOT NULL")
                .map_err(db_error)?;
            let values = q
                .query_map([project], |row| row.get::<_, String>(0))
                .map_err(db_error)?;
            let mut tasks = BTreeSet::new();
            for row in values {
                let value = core::domain::json::parse(&row.map_err(db_error)?).unwrap_or(V::Null);
                if let Some(task) = value["record_id"].as_str() {
                    tasks.insert(task.to_owned());
                }
            }
            tasks
        };
        let (rows, merged) = usage_retrievals(&context, &mut files, &chunk_paths, &tasks)?;
        let history = self.history(project, info["bank_id"].as_str().unwrap_or(""), &rows)?;
        let found = rows.len();
        payload["retrievals"] = json!({"items":rows.into_iter().rev().take(200).collect::<Vec<_>>().into_iter().rev().map(|mut v|{if let Some(o)=v.as_object_mut(){o.remove("key");}v}).collect::<Vec<_>>(),"found":found,"merged":merged,"limit":200});
        payload["history"] = json!({"days":history,"horizon":120});
        payload["health"] = usage_health(&mut files)?;
        payload["check_available"] = json!(info["runtime_available"] == true && !chunks.is_null());
        payload["chunks"] = chunks;
        files.check()?;
        Ok(payload)
    }
    pub fn fold_project(&self, project: &str) -> Result<()> {
        let info = self.info(project, None)?;
        for bank in info["banks"].as_array().into_iter().flatten() {
            if let Some(id) = bank["id"].as_str() {
                self.usage(project, Some(id))?;
            }
        }
        Ok(())
    }
    fn history(&self, project: &str, bank: &str, rows: &[V]) -> Result<Vec<V>> {
        use chrono::{DateTime, Duration, Local};
        let horizon = (Local::now().date_naive() - Duration::days(120)).to_string();
        let mut db = self.store.connection()?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS retrieval_days(project_id TEXT NOT NULL,bank TEXT NOT NULL,day TEXT NOT NULL,data TEXT NOT NULL,PRIMARY KEY(project_id,bank,day));CREATE TABLE IF NOT EXISTS retrieval_seen(project_id TEXT NOT NULL,bank TEXT NOT NULL,manifest TEXT NOT NULL,day TEXT NOT NULL,PRIMARY KEY(project_id,bank,manifest));").map_err(db_error)?;
        let tx = db.transaction().map_err(db_error)?;
        let seen = {
            let mut q = tx
                .prepare("SELECT manifest FROM retrieval_seen WHERE project_id=? AND bank=?")
                .map_err(db_error)?;
            let values = q
                .query_map(rusqlite::params![project, bank], |r| r.get::<_, String>(0))
                .map_err(db_error)?;
            let mut seen = BTreeSet::new();
            for row in values {
                seen.insert(row.map_err(db_error)?);
            }
            seen
        };
        let mut days = BTreeMap::<String, V>::new();
        for row in rows {
            let Some(at) = row["at"]
                .as_str()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            else {
                continue;
            };
            let day = at.with_timezone(&Local).date_naive().to_string();
            let key = row["key"].as_str().unwrap_or("");
            if seen.contains(key) || day < horizon {
                continue;
            }
            let bucket=days.entry(day.clone()).or_insert_with(||json!({"retrievals":0,"with_chunk":0,"brain":0,"bank":0,"rules":0,"cuts":0,"routes":{},"chunks":{}}));
            for (name, count) in [
                ("retrievals", 1),
                (
                    "with_chunk",
                    u64::from(
                        row["chunks"]
                            .as_array()
                            .map(|v| !v.is_empty())
                            .unwrap_or(false),
                    ),
                ),
                ("brain", row["brain"].as_u64().unwrap_or(0)),
                ("bank", row["bank"].as_u64().unwrap_or(0)),
                ("rules", row["rules"].as_u64().unwrap_or(0)),
                (
                    "cuts",
                    row["cuts"].as_array().map(Vec::len).unwrap_or(0) as u64,
                ),
            ] {
                bucket[name] = json!(bucket[name].as_u64().unwrap_or(0) + count);
            }
            let route = row["route"].as_str().unwrap_or("not recorded");
            bucket["routes"][route] = json!(bucket["routes"][route].as_u64().unwrap_or(0) + 1);
            let ids = row["chunks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(V::as_str)
                .collect::<BTreeSet<_>>();
            for id in ids {
                bucket["chunks"][id] = json!(bucket["chunks"][id].as_u64().unwrap_or(0) + 1);
            }
            tx.execute(
                "INSERT OR IGNORE INTO retrieval_seen VALUES(?,?,?,?)",
                rusqlite::params![project, bank, key, day],
            )
            .map_err(db_error)?;
        }
        for (day, bucket) in days {
            let prior = tx
                .query_row(
                    "SELECT data FROM retrieval_days WHERE project_id=? AND bank=? AND day=?",
                    rusqlite::params![project, bank, day],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .map_err(db_error)?;
            let mut total = prior
                .and_then(|s| core::domain::json::parse(&s).ok())
                .filter(V::is_object)
                .unwrap_or(json!({}));
            for (k, v) in bucket.as_object().into_iter().flatten() {
                if let Some(counts) = v.as_object() {
                    if !total[k].is_object() {
                        total[k] = json!({});
                    }
                    for (name, count) in counts {
                        total[k][name] = json!(
                            total[k][name].as_u64().unwrap_or(0) + count.as_u64().unwrap_or(0)
                        );
                    }
                } else {
                    total[k] = json!(total[k].as_u64().unwrap_or(0) + v.as_u64().unwrap_or(0));
                }
            }
            tx.execute(
                "INSERT OR REPLACE INTO retrieval_days VALUES(?,?,?,?)",
                rusqlite::params![project, bank, day, total.to_string()],
            )
            .map_err(db_error)?;
        }
        tx.execute(
            "DELETE FROM retrieval_seen WHERE project_id=? AND bank=? AND day<?",
            rusqlite::params![project, bank, horizon],
        )
        .map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        let mut q=db.prepare("SELECT day,data FROM retrieval_days WHERE project_id=? AND bank=? ORDER BY day DESC LIMIT 366").map_err(db_error)?;
        let values = q
            .query_map(rusqlite::params![project, bank], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(db_error)?;
        let mut out = vec![];
        for row in values {
            let (day, text) = row.map_err(db_error)?;
            let mut value = core::domain::json::parse(&text).unwrap_or(json!({}));
            value["day"] = json!(day);
            out.push(value);
        }
        out.reverse();
        Ok(out)
    }
    fn check(&self, project: &str, bank: Option<&str>) -> Result<V> {
        let (context, info) = self.selected(project, bank, None)?;
        let _guard = self.operation.try_lock().map_err(|_| knowledge_busy())?;
        let service = BankService::new(context.clone());
        let config = RuntimeConfig::load(&context.roots)?;
        let mut held = BTreeMap::<String, u64>::new();
        let mut eligible = 0u64;
        let proposals = core::bank::promotion::list(&service, true)?;
        let mut pending = BTreeSet::new();
        let mut latest = BTreeMap::<String, V>::new();
        for proposal in &proposals {
            for source in proposal["source_records"].as_array().into_iter().flatten() {
                if let Some(id) = source["id"].as_str() {
                    if matches!(proposal["status"].as_str(), Some("proposed" | "reviewed")) {
                        pending.insert(id.to_owned());
                    }
                    if proposal["status"] == "applied"
                        && latest
                            .get(id)
                            .map(|p| {
                                p["updated_at"].as_str().unwrap_or("")
                                    < proposal["updated_at"].as_str().unwrap_or("")
                            })
                            .unwrap_or(true)
                    {
                        latest.insert(id.into(), proposal.clone());
                    }
                }
            }
        }
        for image in BrainRepository::new(context.clone())?
            .records(true)?
            .into_iter()
            .take(5000)
        {
            let record = &image.record;
            let kind = record["type"].as_str().unwrap_or("");
            let status = record["status"].as_str().unwrap_or("");
            if promotable_state(kind) != Some(status) {
                continue;
            }
            let id = record["id"].as_str().unwrap_or("");
            if pending.contains(id) {
                continue;
            }
            let content = core::bank::promotion::content(record);
            if let Some(previous) = latest.get(id) {
                let source = previous["source_records"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|s| s["id"] == id);
                if source
                    .and_then(|v| v["revision"].as_u64())
                    .unwrap_or(u64::MAX)
                    >= record["revision"].as_u64().unwrap_or(0)
                {
                    continue;
                }
                if previous["content"].as_str() == content.as_deref() {
                    continue;
                }
                if previous["review_mode"] != "automatic" {
                    *held.entry("other".into()).or_default() += 1;
                    continue;
                }
                let active = previous["destination_memory_id"]
                    .as_str()
                    .map(|id| {
                        let mut files = PublicationFiles::new(&context.roots.state)?;
                        core::bank::codec::find(&service, id, &mut files)
                            .map(|c| c.metadata["status"] == "active")
                    })
                    .transpose()?
                    .unwrap_or(false);
                if !active {
                    *held.entry("other".into()).or_default() += 1;
                    continue;
                }
            }
            let reason = if record["authority"] != "verified" {
                Some("authority")
            } else if !config
                .strings("allowed_privacy")?
                .iter()
                .any(|p| record["privacy"].as_str() == Some(p))
            {
                Some("privacy")
            } else if content.is_none() {
                Some("no content")
            } else if !core::domain::sources_are_fresh(&context.roots, record)? {
                Some("cited source changed")
            } else if core::domain::sensitive_label(content.as_deref().unwrap_or(""))?.is_some()
                || core::bank::promotion::instruction_like(
                    record["title"].as_str().unwrap_or(""),
                    content.as_deref().unwrap_or(""),
                )?
                || record["sources"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(V::as_str)
                    .any(|s| s.starts_with("https://") || s.starts_with("http://"))
            {
                Some("other")
            } else {
                None
            };
            if let Some(reason) = reason {
                *held.entry(reason.into()).or_default() += 1;
            } else {
                eligible += 1;
            }
        }
        let mut skips = BTreeMap::<String, String>::new();
        let mut files = PublicationFiles::new(&context.roots.state)?;
        for path in core::bank::codec::paths(&service)?.into_iter().take(500) {
            let chunk = match core::bank::codec::read(&mut files, &path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let id = chunk.metadata["id"]
                .as_str()
                .filter(|id| {
                    core::bank::codec::id_pattern()
                        .map(|p| p.is_match(id))
                        .unwrap_or(false)
                })
                .map(str::to_owned)
                .or_else(|| {
                    core::bank::codec::filename_id(
                        Path::new(&path)
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or(""),
                    )
                    .ok()
                    .flatten()
                });
            let Some(id) = id else { continue };
            let reason = if let Some(reason) = core::index::memory_temporal_reason(&chunk.metadata)
            {
                Some(reason)
            } else if core::bank::validation::metadata(&service, &path, &chunk.metadata, false)
                .is_err()
            {
                Some("invalid".into())
            } else if core::domain::secret_label(&String::from_utf8_lossy(&chunk.raw))?.is_some() {
                Some("secret".into())
            } else if chunk.metadata["tags"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|v| v == "auto-promoted")
                && core::bank::promotion::instruction_like(
                    chunk.metadata["title"].as_str().unwrap_or(""),
                    &chunk.body,
                )?
            {
                Some("provenance".into())
            } else {
                None
            };
            if let Some(reason) = reason {
                skips.insert(id, reason.chars().take(40).collect());
            }
        }
        Ok(
            json!({"project_id":project,"bank_id":info["bank_id"],"checked_at":chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs,false),"held_back":held,"eligible":eligible,"skips":skips}),
        )
    }
    fn download(&self, id: &str) -> Result<Payload> {
        if Uuid::parse_str(id).is_err() {
            return Err(HarnessError::missing("Export is unavailable."));
        }
        let cache = self
            .exports
            .lock()
            .map_err(|_| HarnessError::unavailable("Export cache unavailable."))?;
        let bytes = cache
            .iter()
            .find(|(key, _)| key == id)
            .map(|(_, v)| v.clone())
            .ok_or_else(|| HarnessError::missing("Export is unavailable. Create a new export."))?;
        Ok(Payload::Bytes {
            bytes,
            content_type: "application/zip".into(),
            filename: Some(format!("knowledge-{id}.zip")),
            etag: None,
            cache: Some("no-store".into()),
        })
    }
    fn dispatch(&self, operation: &str, request: V) -> Result<Payload> {
        let _source_frame = core::git::source_frame_scope();
        let project = request["params"]["project_id"].as_str().unwrap_or("");
        let query = &request["query"];
        let bank = query["bank"].as_str();
        let path = query["path"].as_str();
        let value = match operation {
            "context.project" => self.project_context(project)?,
            "knowledge.info" => self.info(project, bank)?,
            "knowledge.brain" => self.read(project, bank, path, true)?,
            "memory.read" => self.read(project, bank, path, false)?,
            "memory.usage" => self.usage(project, bank)?,
            "memory.check" => self.check(project, request["body"]["bank"].as_str().or(bank))?,
            "knowledge.run" => self.run(project, &request["body"], None, true)?,
            "knowledge.download" => {
                return self.download(request["params"]["id"].as_str().unwrap_or(""));
            }
            _ => return Err(HarnessError::missing("Unknown knowledge operation.")),
        };
        Ok(Payload::Json(value))
    }
}
impl Domain for KnowledgeDomain {
    fn execute<'a>(&'a self, operation: &'a str, request: V) -> Reply<'a> {
        let own = self.clone();
        let operation = operation.to_owned();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || own.dispatch(&operation, request))
                .await
                .map_err(|_| HarnessError::unavailable("Knowledge worker stopped."))?
        })
    }
}
fn text<'a>(value: &'a str, label: &str, limit: usize) -> Result<&'a str> {
    if value.trim().is_empty()
        || value.chars().count() > limit
        || value
            .chars()
            .any(|c| c == '\0' || c == '\u{7f}' || (c < ' ' && c != '\n' && c != '\t'))
    {
        return Err(HarnessError::invalid(format!("Invalid {label}.")));
    }
    Ok(value)
}
fn required<'a>(v: &'a V, key: &str, limit: usize) -> Result<&'a str> {
    text(
        v[key]
            .as_str()
            .ok_or_else(|| HarnessError::invalid(format!("Missing {key}.")))?,
        key,
        limit,
    )
}
pub(crate) fn relative(value: &str) -> Result<&str> {
    text(value, "relative path", 1024)?;
    if value.starts_with('/')
        || value.contains('\\')
        || Path::new(value).is_absolute()
        || value.split('/').any(|p| {
            p.is_empty()
                || p == "."
                || p == ".."
                || p == ".git"
                || cfg!(windows) && (p.contains(':') || p.ends_with(' ') || p.ends_with('.'))
        })
    {
        return Err(HarnessError::invalid(
            "Use a canonical relative path without parent traversal.",
        ));
    }
    Ok(value)
}
fn string_list(v: &V, max: usize, label: &str) -> Result<Vec<String>> {
    let list = v
        .as_array()
        .filter(|v| v.len() <= max)
        .ok_or_else(|| HarnessError::invalid(format!("Use at most{max} values for {label}.")))?;
    list.iter()
        .map(|v| {
            text(
                v.as_str()
                    .ok_or_else(|| HarnessError::invalid("List values must be text."))?,
                label,
                4000,
            )
            .map(str::to_owned)
        })
        .collect()
}
fn strict(v: &V, allowed: &[&str]) -> Result<()> {
    let o = v
        .as_object()
        .ok_or_else(|| HarnessError::invalid("Request must be an object."))?;
    if o.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(HarnessError::invalid("Unknown request field."));
    }
    Ok(())
}
fn validate_action(action: &str, v: &V) -> Result<()> {
    let fields: &[&str] = match action {
        "status" | "index" | "validate" | "bank-audit" | "reindex-bank" | "compact"
        | "promote-auto" | "cursor-rule-retire" => &[],
        "search" => &["query", "layer"],
        "retrieve" => &["query", "task_id", "paths"],
        "refresh" => &["query", "task_id"],
        "rebind" => &["task_id", "record_id"],
        "bank-reverify" => &["memory_id", "review_after"],
        "bank-retire" => &["memory_id", "valid_to", "superseded_by", "reason"],
        "start" => &["task_id", "goal", "files", "sources"],
        "brain-create" => &[
            "record_type",
            "external_id",
            "title",
            "goal",
            "files",
            "sources",
            "privacy",
            "authority",
        ],
        "brain-update" => &[
            "record_id",
            "revision",
            "progress",
            "next_steps",
            "replace_next_steps",
            "phase",
            "transition",
            "reason",
            "authority",
        ],
        "complete" => &["task_id", "revision", "outcome", "verification", "sources"],
        "promote-propose" => &["source_ids", "title", "content"],
        "promote-review" => &["promotion_id", "reviewer", "reject"],
        "promote-apply" => &["promotion_id"],
        "record-result" | "replay-result" => &[
            "task_id",
            "result_id",
            "revision",
            "request",
            "attestation",
            "reason",
            "prepared_scope",
        ],
        "prepare-result" => &[
            "task_id",
            "result_id",
            "revision",
            "request",
            "attestation",
            "reason",
        ],
        "export" => &["include_archive", "include_superseded"],
        "turn" => &["task_id"],
        _ => {
            return Err(HarnessError::invalid(
                "Select a supported knowledge operation.",
            ));
        }
    };
    let mut allowed = fields.to_vec();
    allowed.extend(["action", "bank"]);
    strict(v, &allowed)?;
    for (field, value) in v.as_object().expect("strict object") {
        if ["action", "bank", "request"].contains(&field.as_str()) {
            continue;
        }
        if [
            "include_archive",
            "include_superseded",
            "reject",
            "replace_next_steps",
        ]
        .contains(&field.as_str())
        {
            if !value.is_boolean() {
                return Err(HarnessError::invalid("Operation options must be booleans."));
            }
        } else if field == "revision" {
            if !value
                .as_u64()
                .is_some_and(|v| (1..=2147483647).contains(&v))
            {
                return Err(HarnessError::invalid(
                    "Use the record's positive integer revision.",
                ));
            }
        } else if [
            "files",
            "sources",
            "next_steps",
            "verification",
            "paths",
            "source_ids",
        ]
        .contains(&field.as_str())
        {
            let list = string_list(value, if field == "source_ids" { 20 } else { 50 }, field)?;
            for path in list {
                if ["files", "paths"].contains(&field.as_str()) {
                    relative(&path)?;
                }
            }
        } else {
            text(
                value
                    .as_str()
                    .ok_or_else(|| HarnessError::invalid("Operation field must be text."))?,
                field,
                if ["goal", "progress", "outcome", "content", "query"].contains(&field.as_str()) {
                    16000
                } else {
                    2000
                },
            )?;
        }
    }
    if ["record-result", "prepare-result", "replay-result"].contains(&action)
        && (!v["request"].is_object()
            || serde_json::to_vec(&v["request"])
                .map_err(|_| HarnessError::invalid("Invalid result JSON."))?
                .len()
                > 262144)
    {
        return Err(HarnessError::invalid(
            "A memory result must be an object of at most256KiB.",
        ));
    }
    Ok(())
}
fn metadata(content: &str, path: &str) -> V {
    if path.ends_with(".json") {
        return core::domain::json::parse(content)
            .ok()
            .filter(V::is_object)
            .unwrap_or(json!({}));
    }
    content
        .strip_prefix("---\n")
        .and_then(|s| s.split_once("\n---").map(|(s, _)| s))
        .and_then(|s| core::domain::json::parse(s).ok())
        .filter(V::is_object)
        .unwrap_or(json!({}))
}
fn entry(path: &str, size: u64, content: &str, brain: bool) -> V {
    let meta = metadata(content, path);
    let title = meta["title"]
        .as_str()
        .or_else(|| meta["objective"].as_str())
        .filter(|s| !s.trim().is_empty())
        .or_else(|| content.lines().find_map(|s| s.strip_prefix("# ")))
        .unwrap_or_else(|| path.rsplit('/').next().unwrap_or(path));
    let mut v = json!({"path":path,"title":title.chars().take(200).collect::<String>(),"bytes":size,"category":path.rsplit_once('/').map(|(p,_)|p).unwrap_or(""),"revision":meta["revision"].as_i64()});
    for key in ["type", "status", "id", "external_id", "owner", "phase"] {
        v[key] = meta[key]
            .as_str()
            .map(|s| json!(s.chars().take(200).collect::<String>()))
            .unwrap_or(V::Null);
    }
    if !brain {
        v["kind"] = json!(if path.starts_with("chunks/") {
            "chunk"
        } else if path.starts_with("local/") {
            "local"
        } else {
            "document"
        });
    }
    v
}
fn merge(target: &mut V, value: V) {
    if let (Some(a), Some(b)) = (target.as_object_mut(), value.as_object()) {
        for (k, v) in b {
            a.insert(k.clone(), v.clone());
        }
    }
}
fn names(root: &Path, relative_path: &str, files: &mut PublicationFiles) -> Result<Vec<String>> {
    relative(relative_path)?;
    files.read_prefix(&format!("{relative_path}/.custody-probe"), 0)?;
    let path = root.join(relative_path);
    if !core::fs::admit_directory(&path)? {
        return Ok(vec![]);
    }
    let mut out = vec![];
    for entry in std::fs::read_dir(&path)
        .map_err(|_| HarnessError::unavailable("Knowledge folder is unavailable."))?
        .take(20001)
    {
        let entry = entry.map_err(|_| HarnessError::unavailable("Knowledge folder changed."))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| HarnessError::invalid("Unsupported knowledge filename."))?;
        relative(&name)?;
        out.push(name);
    }
    files.check()?;
    out.sort();
    Ok(out)
}
fn promotable_state(kind: &str) -> Option<&'static str> {
    match kind {
        "finding" | "bug" => Some("resolved"),
        "incident" => Some("closed"),
        "decision" => Some("accepted"),
        _ => None,
    }
}
fn promotable(context: &RuntimeContext, r: &V) -> Result<bool> {
    let config = RuntimeConfig::load(&context.roots)?;
    Ok(
        promotable_state(r["type"].as_str().unwrap_or("")) == r["status"].as_str()
            && r["authority"] == "verified"
            && ["public", "team"].contains(&r["privacy"].as_str().unwrap_or(""))
            && config.value["allowed_privacy"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|v| v == &r["privacy"])
            && core::domain::sources_are_fresh(&context.roots, r)?
            && core::bank::promotion::content(r).is_some(),
    )
}
pub fn validate_capsule(capsule: &V, task: &V) -> Result<()> {
    if !capsule.is_object()
        || capsule["task_id"] != task["external_id"]
        || capsule["task_uuid"] != task["id"]
        || capsule["task_revision"].as_u64().is_none()
        || capsule["task_revision"] != task["revision"]
        || !capsule["working"].is_object()
        || capsule["working"]["task_id"] != task["external_id"]
    {
        return Err(HarnessError::conflict(
            "Retrieved context does not match the linked task and revision. Prepare it again.",
        ));
    }
    for (layer, limit) in [("procedural", 2), ("semantic", 3), ("episodic", 3)] {
        if capsule[layer]
            .as_array()
            .is_none_or(|v| v.len() > limit || v.iter().any(|v| !v.is_object()))
        {
            return Err(HarnessError::invalid(
                "The runtime returned an invalid task capsule.",
            ));
        }
    }
    let warnings = &capsule["warnings"];
    if warnings.as_array().is_some_and(|v| !v.is_empty())
        || warnings.as_str().is_some_and(|s| !s.is_empty())
        || warnings.as_object().is_some_and(|v| !v.is_empty())
        || warnings == true
    {
        return Err(HarnessError::conflict(
            "Context refresh reported warnings. Resolve the runtime warnings before preparing this session.",
        ));
    }
    let encoded = core::domain::json::serialize(capsule, false, false, false, false)?;
    if encoded.chars().count() > 8000 {
        return Err(HarnessError::invalid(
            "The task capsule exceeds the native 8000-character limit.",
        ));
    }
    Ok(())
}
pub fn capsule_material(capsule: &V) -> V {
    fn content(value: &V) -> V {
        match value {
            V::Object(o) => V::Object(
                o.iter()
                    .filter(|(k, _)| !["estimated_tokens", "score", "rank"].contains(&k.as_str()))
                    .map(|(k, v)| (k.clone(), content(v)))
                    .collect(),
            ),
            V::Array(a) => json!(a.iter().map(content).collect::<Vec<_>>()),
            _ => value.clone(),
        }
    }
    let mut selected = json!({});
    for key in [
        "query",
        "task_id",
        "task_uuid",
        "task_revision",
        "working",
        "procedural",
        "semantic",
        "episodic",
        "selected",
    ] {
        selected[key] = capsule[key].clone();
    }
    content(&selected)
}
fn capsule_path(context: &RuntimeContext, value: &V) -> Result<String> {
    let value = value
        .as_str()
        .ok_or_else(|| HarnessError::invalid("A capsule source path is invalid."))?;
    if context.roots.attached() && (value.starts_with('/') || Path::new(value).is_absolute()) {
        let path = Path::new(value);
        if !path
            .components()
            .any(|p| matches!(p, std::path::Component::ParentDir))
            && path.starts_with(&context.roots.tooling)
        {
            return Ok(value.into());
        }
    }
    relative(value)?;
    Ok(value.into())
}
fn visible_sources(
    context: &RuntimeContext,
    capsule: &V,
    strict: bool,
) -> Result<BTreeSet<String>> {
    let mut visible = BTreeSet::new();
    for layer in ["procedural", "semantic", "episodic", "selected"] {
        if strict && !capsule[layer].is_array() {
            return Err(HarnessError::invalid(
                "Local source provenance has invalid capsule coverage.",
            ));
        }
        for item in capsule[layer].as_array().into_iter().flatten() {
            if item["path"].is_string() {
                visible.insert(capsule_path(context, &item["path"])?);
            }
        }
    }
    Ok(visible)
}
fn valid_digest(value: &V) -> Option<&str> {
    value.as_str().filter(|s| {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
pub fn prepared_sources(
    context: &RuntimeContext,
    capsule: &V,
    task: &V,
    provenance: Option<&V>,
    allow_local: bool,
) -> Result<V> {
    validate_capsule(capsule, task)?;
    let manifest = &capsule["manifest"];
    let has_sources = ["procedural", "semantic", "episodic", "selected"]
        .iter()
        .any(|layer| {
            capsule[*layer]
                .as_array()
                .into_iter()
                .flatten()
                .any(|item| item["path"].is_string())
        });
    if manifest.is_null() && (provenance.is_none() || !allow_local) && !has_sources {
        return Ok(json!({}));
    }
    if manifest.is_null() && allow_local {
        let proof = provenance
            .ok_or_else(|| HarnessError::invalid("The local source provenance is unavailable."))?;
        strict(
            proof,
            &[
                "schema_version",
                "profile",
                "scope",
                "publication",
                "task_uuid",
                "task_revision",
                "capsule_sha256",
                "selected",
            ],
        )?;
        if proof.as_object().map(|o| o.len()) != Some(8)
            || proof["schema_version"] != 1
            || proof["profile"] != "local-source-provenance-v1"
            || proof["scope"] != "local"
            || proof["publication"] != "unconfirmed"
            || capsule["manifest_scope"] != "local"
            || !capsule["task_uuid"]
                .as_str()
                .is_some_and(core::domain::is_uuid4)
            || proof["task_uuid"] != capsule["task_uuid"]
            || capsule["task_revision"].as_u64().is_none_or(|n| n < 1)
            || proof["task_revision"] != capsule["task_revision"]
            || valid_digest(&proof["capsule_sha256"]).is_none()
            || proof["selected"].as_array().is_none_or(|a| a.len() > 32)
        {
            return Err(HarnessError::invalid(
                "The local source provenance is unavailable or invalid.",
            ));
        }
        let encoded = serde_json::to_vec(proof)
            .map_err(|_| HarnessError::invalid("Invalid local source provenance."))?;
        let frame = core::domain::json::canonical(capsule)?;
        if encoded.len() > 65536
            || Some(core::query::sha256(frame.as_bytes()).as_str())
                != proof["capsule_sha256"].as_str()
        {
            return Err(HarnessError::conflict(
                "The local source provenance does not match the capsule.",
            ));
        }
        let visible = visible_sources(context, capsule, true)?;
        let mut hashes = BTreeMap::<String, String>::new();
        for item in proof["selected"].as_array().into_iter().flatten() {
            if item.as_object().is_none_or(|o| {
                o.len() != 2 || !o.contains_key("path") || !o.contains_key("source_hash")
            }) {
                return Err(HarnessError::invalid(
                    "Local source provenance has invalid selected sources.",
                ));
            }
            let path = capsule_path(context, &item["path"])?;
            let digest = valid_digest(&item["source_hash"]).ok_or_else(|| {
                HarnessError::invalid("Local source provenance has an invalid source fingerprint.")
            })?;
            if hashes.insert(path, digest.into()).is_some() {
                return Err(HarnessError::invalid(
                    "Local source provenance has duplicate selected sources.",
                ));
            }
        }
        if hashes.keys().cloned().collect::<BTreeSet<_>>() != visible {
            return Err(HarnessError::conflict(
                "The local source provenance does not cover the approved context.",
            ));
        }
        return Ok(json!(hashes));
    }
    let manifest = manifest
        .as_str()
        .ok_or_else(|| HarnessError::invalid("The task capsule has no valid source manifest."))?;
    let pattern = regex::Regex::new(
        r"^(?:project-brain/control|memory-bank/local)/retrieval-manifests/[0-9a-f-]{36}\.json$",
    )
    .map_err(|_| HarnessError::invalid("Invalid internal manifest expression."))?;
    if !pattern.is_match(manifest) {
        return Err(HarnessError::invalid(
            "The task capsule has no valid source manifest.",
        ));
    }
    let mut files = PublicationFiles::new(&context.roots.state)?;
    let (size, raw) = files
        .read_prefix(manifest, 512 * 1024)?
        .ok_or_else(|| HarnessError::missing("The task context manifest is unavailable."))?;
    if size > 512 * 1024 {
        return Err(HarnessError::invalid(
            "The task context manifest exceeds its byte bound.",
        ));
    }
    let metadata = core::results::strict_json(&raw)?;
    if !metadata.is_object()
        || metadata["task_id"] != capsule["task_uuid"]
        || metadata["task_revision"] != capsule["task_revision"]
        || !metadata["selected"].is_array()
    {
        return Err(HarnessError::conflict(
            "The task context manifest has an invalid task binding.",
        ));
    }
    let visible = visible_sources(context, capsule, false)?;
    let mut hashes = BTreeMap::<String, String>::new();
    for item in metadata["selected"].as_array().into_iter().flatten() {
        if !item.is_object() {
            continue;
        }
        let Some(path) = item["path"].as_str().filter(|p| visible.contains(*p)) else {
            continue;
        };
        let digest = valid_digest(&item["source_hash"]).ok_or_else(|| {
            HarnessError::invalid("A selected context source has no valid fingerprint.")
        })?;
        hashes.insert(path.into(), digest.into());
    }
    if hashes.keys().cloned().collect::<BTreeSet<_>>() != visible {
        return Err(HarnessError::conflict(
            "The source manifest does not cover the approved context.",
        ));
    }
    Ok(json!(hashes))
}
fn package_export(root: &Path) -> Result<Vec<u8>> {
    let mut files = PublicationFiles::new(root)?;
    let mut queue = VecDeque::from([String::new()]);
    let mut total = 0usize;
    let mut count = 0;
    let stream = Cursor::new(Vec::new());
    let mut archive = zip::ZipWriter::new(stream);
    while let Some(folder) = queue.pop_front() {
        let entries = std::fs::read_dir(root.join(&folder))
            .map_err(|_| HarnessError::unavailable("Export folder unavailable."))?;
        for entry in entries {
            let entry = entry.map_err(|_| HarnessError::unavailable("Export folder changed."))?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| HarnessError::invalid("Export filename is unsupported."))?;
            let path = if folder.is_empty() {
                name
            } else {
                format!("{folder}/{name}")
            };
            relative(&path)?;
            let meta = entry
                .metadata()
                .map_err(|_| HarnessError::unavailable("Export changed."))?;
            if meta.is_dir() {
                files.read_prefix(&format!("{path}/.custody-probe"), 0)?;
                queue.push_back(path);
            } else {
                count += 1;
                if count > 5000 {
                    return Err(HarnessError::invalid("Export file count exceeds5000."));
                }
                let raw = files
                    .read(&path, EXPORT_LIMIT)?
                    .ok_or_else(|| HarnessError::conflict("Export changed."))?;
                total += raw.len();
                if total > EXPORT_LIMIT {
                    return Err(HarnessError::invalid("Export exceeds32MiB."));
                }
                archive
                    .start_file(
                        path,
                        zip::write::FileOptions::default()
                            .compression_method(zip::CompressionMethod::Deflated),
                    )
                    .map_err(|_| HarnessError::unavailable("Export packaging failed."))?;
                archive
                    .write_all(&raw)
                    .map_err(|_| HarnessError::unavailable("Export packaging failed."))?;
            }
        }
    }
    files.check()?;
    Ok(archive
        .finish()
        .map_err(|_| HarnessError::unavailable("Export packaging failed."))?
        .into_inner())
}

fn db_error(_: rusqlite::Error) -> HarnessError {
    HarnessError::unavailable("The Harness analytics database is unavailable.")
}
fn usage_day(value: &V) -> V {
    value
        .as_str()
        .filter(|s| s.len() == 10)
        .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        .map(|d| json!(d.to_string()))
        .unwrap_or(V::Null)
}
fn usage_moment(value: &V) -> V {
    value
        .as_str()
        .filter(|s| s.len() <= 64)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| json!(d.with_timezone(&chrono::Utc).to_rfc3339()))
        .unwrap_or(V::Null)
}
fn usage_listing(
    context: &RuntimeContext,
    folder: &str,
    suffix: &str,
    limit: usize,
    files: &mut PublicationFiles,
) -> Result<Option<(Vec<String>, bool)>> {
    if !core::fs::admit_directory(&context.roots.state.join(folder))? {
        return Ok(None);
    }
    let mut entries = vec![];
    for name in names(&context.roots.state, folder, files)? {
        if name.starts_with('.') || !name.ends_with(suffix) {
            continue;
        }
        if let Ok(Some(image)) = files.image_identity(&format!("{folder}/{name}")) {
            entries.push((image.5, name));
        }
    }
    entries.sort_by(|a, b| b.cmp(a));
    let truncated = entries.len() > limit;
    Ok(Some((
        entries.into_iter().take(limit).map(|(_, n)| n).collect(),
        truncated,
    )))
}
fn usage_chunks(context: &RuntimeContext, files: &mut PublicationFiles) -> Result<V> {
    let Some((mut names, truncated)) =
        usage_listing(context, "memory-bank/chunks", ".md", 500, files)?
    else {
        return Ok(V::Null);
    };
    names.sort();
    let id_pattern = regex::Regex::new(r"^(MEM-(?:\d{8}-[0-9a-f]{8}|\d{4,}))")
        .map_err(|_| HarnessError::invalid("Invalid internal memory identity expression."))?;
    let mut chunks = vec![];
    let mut total = 0u64;
    let mut sources = PublicationFiles::new(&context.roots.project)?;
    for name in names {
        let Some((size, bytes)) =
            files.read_prefix(&format!("memory-bank/chunks/{name}"), 65536)?
        else {
            continue;
        };
        total += size;
        let (meta, _) = core::bank::codec::parse_raw(&bytes).unwrap_or((json!({}), String::new()));
        let tags = meta["tags"].as_array().cloned().unwrap_or_default();
        let id = meta["id"]
            .as_str()
            .filter(|v| {
                core::bank::codec::id_pattern()
                    .map(|p| p.is_match(v))
                    .unwrap_or(false)
            })
            .map(str::to_owned)
            .or_else(|| id_pattern.captures(&name).map(|c| c[1].to_owned()))
            .unwrap_or_else(|| repeated_suffix_removed(&name, ".md").to_owned());
        let changed =
            if let Some(digests) = meta["source_digests"].as_array().filter(|v| !v.is_empty()) {
                let mut unknown = false;
                let mut changed = false;
                for item in digests {
                    let Some(path) = item["path"].as_str() else {
                        changed = true;
                        break;
                    };
                    let Some(expected) = item["sha256"].as_str() else {
                        changed = true;
                        break;
                    };
                    if relative(path).is_err() {
                        changed = true;
                        break;
                    }
                    match sources.read_prefix(path, 32 * 1024 * 1024) {
                        Ok(Some((size, _))) if size > 32 * 1024 * 1024 => unknown = true,
                        Ok(Some((_, raw))) => {
                            if core::query::sha256(&raw) != expected {
                                changed = true;
                                break;
                            }
                        }
                        Ok(None) | Err(_) => {
                            changed = true;
                            break;
                        }
                    }
                }
                if changed {
                    json!(true)
                } else if unknown {
                    V::Null
                } else {
                    json!(false)
                }
            } else {
                V::Null
            };
        let short = |key: &str, n: usize| {
            meta[key]
                .as_str()
                .map(|s| json!(s.chars().take(n).collect::<String>()))
                .unwrap_or(V::Null)
        };
        chunks.push(json!({"id":id,"title":short("title",200),"type":short("type",40),"status":short("status",40),"created":usage_day(&meta["created"]),"last_verified":usage_day(&meta["last_verified"]),"review_after":usage_day(&meta["review_after"]),"valid_to":usage_day(&meta["valid_to"]),"promoted":tags.iter().any(|v|v=="promoted"),"auto":tags.iter().any(|v|v=="auto-promoted"),"bytes":size,"sources":meta["sources"].as_array().map(Vec::len).unwrap_or(0),"sources_changed":changed,"path":format!("chunks/{name}")}));
    }
    Ok(json!({"items":chunks,"truncated":truncated,"bytes":total}))
}
fn usage_promotions(
    context: &RuntimeContext,
    files: &mut PublicationFiles,
) -> Result<(V, BTreeSet<String>)> {
    let Some((mut names, truncated)) = usage_listing(
        context,
        "project-brain/control/promotions",
        ".json",
        500,
        files,
    )?
    else {
        return Ok((json!({"items":[],"truncated":false}), BTreeSet::new()));
    };
    names.sort();
    let mut items = vec![];
    let mut promoted = BTreeSet::new();
    for name in names {
        let Some((_, raw)) =
            files.read_prefix(&format!("project-brain/control/promotions/{name}"), 65536)?
        else {
            continue;
        };
        let p = core::domain::json::parse(&String::from_utf8_lossy(&raw)).unwrap_or(V::Null);
        if !matches!(
            p["status"].as_str(),
            Some("proposed" | "reviewed" | "rejected" | "applied")
        ) {
            continue;
        }
        if p["status"] != "rejected" {
            for item in p["source_records"].as_array().into_iter().flatten() {
                if let Some(id) = item["id"].as_str() {
                    promoted.insert(id.to_owned());
                }
            }
        }
        let memory = p["destination_memory_id"]
            .as_str()
            .filter(|s| {
                core::bank::codec::id_pattern()
                    .map(|p| p.is_match(s))
                    .unwrap_or(false)
            })
            .map(|s| json!(s))
            .unwrap_or(V::Null);
        items.push(json!({"status":p["status"],"mode":if p["review_mode"]=="automatic"{"automatic"}else{"human"},"created_at":usage_moment(&p["created_at"]),"reviewed_at":usage_moment(&p["reviewed_at"]),"applied_at":if p["status"]=="applied"{usage_moment(&p["updated_at"])}else{V::Null},"memory_id":memory}));
    }
    Ok((json!({"items":items,"truncated":truncated}), promoted))
}
fn usage_records(
    context: &RuntimeContext,
    files: &mut PublicationFiles,
    promoted: &BTreeSet<String>,
) -> Result<V> {
    let mut items = vec![];
    let mut total = 0usize;
    let mut truncated = false;
    for base in ["dynamic", "archive"] {
        let folder = format!("project-brain/{base}");
        let mut groups = vec![String::new()];
        if core::fs::admit_directory(&context.roots.state.join(&folder))? {
            for name in names(&context.roots.state, &folder, files)? {
                if !name.starts_with('.')
                    && name != "handoffs"
                    && name != "messages"
                    && core::fs::admit_directory(&context.roots.state.join(&folder).join(&name))
                        .unwrap_or(false)
                {
                    groups.push(name);
                }
            }
        }
        for group in groups {
            let folder = if group.is_empty() {
                folder.clone()
            } else {
                format!("{folder}/{group}")
            };
            let Some((mut names, more)) = usage_listing(context, &folder, ".md", 5000, files)?
            else {
                continue;
            };
            truncated |= more;
            names.sort();
            for name in names {
                if items.len() >= 5000 || total >= 8 * 1024 * 1024 {
                    return Ok(json!({"items":items,"truncated":true}));
                }
                let Some((_, raw)) = files.read_prefix(&format!("{folder}/{name}"), 65536)? else {
                    continue;
                };
                total += raw.len();
                let record = metadata(&String::from_utf8_lossy(&raw), &name);
                let Some(kind) = record["type"].as_str().filter(|v| {
                    ["task", "finding", "bug", "incident", "decision", "event"].contains(v)
                }) else {
                    continue;
                };
                let Some(status) = record["status"].as_str() else {
                    continue;
                };
                let resolved = promotable_state(kind);
                let mut at = V::Null;
                for transition in record["transitions"].as_array().into_iter().flatten() {
                    if transition["to"].as_str() == resolved {
                        let moment = usage_moment(&transition["at"]);
                        if !moment.is_null() {
                            at = moment;
                        }
                    }
                }
                let open = match kind {
                    "task" => ["active", "blocked", "verifying"].contains(&status),
                    "finding" => ["open", "investigating"].contains(&status),
                    "bug" => ["reported", "triaged", "fixing", "verifying"].contains(&status),
                    "incident" => ["open", "contained", "recovering", "resolved"].contains(&status),
                    "decision" => status == "proposed",
                    _ => false,
                };
                let mut row = json!({"archived":base=="archive","open":open,"resolved_at":at,"promotable":resolved==Some(status),"promoted":record["id"].as_str().map(|id|promoted.contains(id)).unwrap_or(false)});
                if matches!(record["privacy"].as_str(), Some("public" | "team")) {
                    row["type"] = json!(kind);
                    row["status"] = json!(status.chars().take(40).collect::<String>());
                } else {
                    row["private"] = json!(true);
                }
                items.push(row);
            }
        }
    }
    Ok(json!({"items":items,"truncated":truncated}))
}
fn usage_chunk_id(path: &V, chunk_paths: &BTreeMap<String, String>) -> Option<String> {
    let path = path.as_str()?.strip_prefix("memory-bank/")?;
    if !path.starts_with("chunks/")
        || !path.ends_with(".md")
        || path.trim_start_matches("chunks/").contains('/')
    {
        return None;
    }
    chunk_paths.get(path).cloned().or_else(|| {
        regex::Regex::new(r"^(MEM-(?:\d{8}-[0-9a-f]{8}|\d{4,}))")
            .ok()?
            .captures(path.trim_start_matches("chunks/"))
            .map(|c| c[1].to_owned())
    })
}
fn usage_retrievals(
    context: &RuntimeContext,
    files: &mut PublicationFiles,
    chunk_paths: &BTreeMap<String, String>,
    harness_tasks: &BTreeSet<String>,
) -> Result<(Vec<V>, usize)> {
    let mut manifests = vec![];
    for (store, folder) in [
        ("governed", "project-brain/control/retrieval-manifests"),
        ("local", "memory-bank/local/retrieval-manifests"),
    ] {
        if let Some((names, _)) = usage_listing(context, folder, ".json", 400, files)? {
            for name in names {
                let Some((size, raw)) =
                    files.read_prefix(&format!("{folder}/{name}"), 256 * 1024)?
                else {
                    continue;
                };
                if size > 256 * 1024 {
                    continue;
                }
                let manifest =
                    core::domain::json::parse(&String::from_utf8_lossy(&raw)).unwrap_or(V::Null);
                let at = usage_moment(&manifest["created_at"]);
                if let Some(at) = at.as_str().filter(|_| manifest["selected"].is_array()) {
                    manifests.push((
                        at.to_owned(),
                        store.to_owned(),
                        manifest,
                        format!("{store}:{name}"),
                    ));
                }
            }
        }
    }
    manifests.sort_by(|a, b| a.0.cmp(&b.0));
    let mut tasks = BTreeMap::<String, usize>::new();
    let mut prepared = BTreeMap::<String, usize>::new();
    let mut rows: Vec<V> = vec![];
    let mut merged = 0usize;
    for (at, store, manifest, source) in manifests {
        let selected = manifest["selected"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|v| v.is_object())
            .collect::<Vec<_>>();
        let pairs = selected
            .iter()
            .map(|v| (v["path"].to_string(), v["source_hash"].to_string()))
            .collect::<BTreeSet<_>>();
        let key = serde_json::to_string(&(
            manifest["task_id"].clone(),
            manifest["task_revision"].clone(),
            pairs,
        ))
        .map_err(|_| HarnessError::invalid("Invalid manifest selection."))?;
        let host = manifest["host"].as_str().unwrap_or("");
        let entry = manifest["entry_point"].as_str().unwrap_or("");
        let ungated = !manifest["gate"].is_object() || manifest["gate"]["mode"] == "off";
        let reviewed = ungated && host == "cli" && entry == "retrieve";
        let harness_shape = reviewed
            || ungated && ["claude", "codex", "cursor"].contains(&host) && entry == "refresh";
        if store == "local"
            && reviewed
            && let Some(index) = prepared.get(&key)
        {
            rows[*index]["merged"] = json!(rows[*index]["merged"].as_u64().unwrap_or(0) + 1);
            merged += 1;
            continue;
        }
        let mut counts = BTreeMap::from([("brain", 0u64), ("bank", 0), ("rules", 0)]);
        let mut chunks = vec![];
        for item in selected {
            let category = item["category"].as_str().unwrap_or("");
            let kind = match category {
                "policy" | "evidence" => Some("rules"),
                "durable" => Some("bank"),
                "handoff" | "dynamic" => Some("brain"),
                _ => None,
            };
            if let Some(kind) = kind {
                *counts.entry(kind).or_default() += 1;
            }
            if category == "durable"
                && let Some(id) = usage_chunk_id(&item["path"], chunk_paths)
            {
                chunks.push(json!(id));
            }
        }
        if let Some(n) = manifest["local_episode_count"].as_u64().filter(|n| *n > 0) {
            *counts.entry("brain").or_default() += n;
        }
        let mut cuts = vec![];
        for item in manifest["excluded"].as_array().into_iter().flatten() {
            if let (Some(id), Some(reason)) = (
                usage_chunk_id(&item["path"], chunk_paths),
                item["reason"].as_str(),
            ) {
                cuts.push(json!([id, reason.chars().take(40).collect::<String>()]));
            }
        }
        let hook = match host {
            "claude" => Some("Claude hook"),
            "codex" => Some("Codex hook"),
            "cursor" => Some("Cursor hook"),
            "cli" => Some("CLI hook"),
            _ => None,
        };
        let route = if let Some(hook) = hook {
            if entry == "hook-context" {
                hook
            } else if harness_shape
                && manifest["task_id"]
                    .as_str()
                    .map(|id| harness_tasks.contains(id))
                    .unwrap_or(false)
            {
                "Harness"
            } else if ["claude", "codex", "cursor"].contains(&host) && entry == "refresh" {
                hook
            } else {
                "CLI"
            }
        } else {
            "not recorded"
        };
        let task_key = manifest["task_id"].to_string();
        let next = tasks.len();
        let task = *tasks.entry(task_key).or_insert(next);
        let row = json!({"key":source,"at":at,"version":manifest["schema_version"].as_i64(),"route":route,"task":task,"brain":counts["brain"],"bank":counts["bank"],"rules":counts["rules"],"chunks":chunks,"cuts":cuts,"merged":0});
        if store == "governed" && reviewed {
            prepared.insert(key, rows.len());
        }
        rows.push(row);
    }
    Ok((rows, merged))
}
fn usage_health(files: &mut PublicationFiles) -> Result<V> {
    let Some((size, raw)) =
        files.read_prefix("memory-bank/local/refresh-health.ndjson", 2 * 1024 * 1024)?
    else {
        return Ok(V::Null);
    };
    let text = String::from_utf8_lossy(&raw);
    let mut lines = text.lines().collect::<Vec<_>>();
    if size > 2 * 1024 * 1024 {
        lines.pop();
    }
    let mut items = vec![];
    for line in lines {
        let record = core::domain::json::parse(line).unwrap_or(V::Null);
        let at = usage_moment(&record["at"]);
        if at.is_null() {
            continue;
        }
        let counts = record["omitted"]
            .as_object()
            .map(|o| o.values().filter_map(V::as_i64).collect::<Vec<_>>())
            .unwrap_or_default();
        items.push(json!({"at":at,"dropped":if counts.is_empty(){V::Null}else{json!(counts.iter().any(|n|*n>0))}}));
    }
    Ok(json!({"items":items,"truncated":size>2*1024*1024}))
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

fn knowledge_busy() -> HarnessError {
    HarnessError {
        status: 409,
        code: "knowledge-busy",
        message: "Another knowledge operation is in progress. Try again shortly.".into(),
    }
}
