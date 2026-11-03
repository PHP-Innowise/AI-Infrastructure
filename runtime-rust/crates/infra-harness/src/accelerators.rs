//! Project lending and version-bound installed updates, with explicit hook approval.
use crate::{
    HarnessError, Result,
    provider::ProviderDomain,
    services::{Domain, Payload, Reply},
    sessions::Sessions,
    store::{self, SessionStore},
};
use infra_core::bank::publication::PublicationFiles;
use infra_tooling::{
    attach::{self, Overlay},
    installer::{InstallOptions, Installer},
};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};
const EDITIONS: [&str; 4] = ["Laravel", "Symfony", "PHP Core", "Cms/wordpress"];
pub struct Accelerators {
    pub store: SessionStore,
    pub source: PathBuf,
    pub providers: Arc<ProviderDomain>,
    sessions: Weak<Sessions>,
    serial: Mutex<()>,
    version: Mutex<Option<(Instant, String)>>,
}
impl Accelerators {
    pub fn new(
        store: SessionStore,
        source: PathBuf,
        providers: Arc<ProviderDomain>,
        sessions: Weak<Sessions>,
    ) -> Result<Self> {
        store.connection()?.execute_batch("CREATE TABLE IF NOT EXISTS project_accelerators(project_id TEXT PRIMARY KEY,edition TEXT NOT NULL,attached_at TEXT NOT NULL);CREATE TABLE IF NOT EXISTS accelerator_syncs(project_id TEXT PRIMARY KEY,source TEXT NOT NULL,synced_at TEXT NOT NULL,report TEXT NOT NULL);")?;
        Ok(Self {
            store,
            source,
            providers,
            sessions,
            serial: Mutex::new(()),
            version: Mutex::new(None),
        })
    }
    fn edition(&self, project: &str) -> Result<Option<String>> {
        let value: Option<String> = self
            .store
            .connection()?
            .query_row(
                "SELECT edition FROM project_accelerators WHERE project_id=?1",
                [project],
                |r| r.get(0),
            )
            .optional()?;
        value
            .map(|e| {
                if EDITIONS.contains(&e.as_str()) {
                    Ok(e)
                } else {
                    Err(HarnessError::invalid(
                        "The attached edition is unsupported.",
                    ))
                }
            })
            .transpose()
    }
    pub fn installed(path: &Path) -> Result<bool> {
        let mut files = PublicationFiles::new(path)?;
        Ok(files
            .read_prefix("memory-bank/runtime-contract.json", 0)?
            .is_some()
            || files
                .read_prefix(
                    if cfg!(windows) {
                        "memory-bank/bin/infra-engine.exe"
                    } else {
                        "memory-bank/bin/infra-engine"
                    },
                    0,
                )?
                .is_some())
    }
    pub fn state(&self, project: &str) -> Result<PathBuf> {
        self.store.project(project)?;
        Ok(self.store.state.join("attached").join(project))
    }
    pub fn summary(&self, project: &str) -> Result<Value> {
        let p = self.store.project(project)?;
        let installed =
            Self::installed(Path::new(p["path"].as_str().unwrap_or(""))).unwrap_or(false);
        let edition = self.edition(project)?;
        Ok(
            json!({"mode":if installed{Some("installed")}else if edition.is_some(){Some("attached")}else{None},"edition":edition}),
        )
    }
    pub fn get(&self, project: &str) -> Result<Value> {
        let p = self.store.project(project)?;
        let path = Path::new(p["path"].as_str().unwrap_or(""));
        let (detected, evidence) =
            attach::detect(path).unwrap_or((None, "The project folder is unavailable.".into()));
        let edition = self.edition(project)?;
        let installed = Self::installed(path).unwrap_or(false);
        Ok(
            json!({"project_id":project,"mode":if installed{Some("installed")}else if edition.is_some(){Some("attached")}else{None},"edition":edition,"detected":detected,"evidence":evidence,"editions":EDITIONS,"home":if !installed{edition.as_ref().map(|e|self.source.join(e))}else{None},"state":self.state(project)?,"sync":if installed{self.last_sync(project)?}else{Value::Null}}),
        )
    }
    pub fn attach(&self, project: &str, edition: Option<&str>) -> Result<Value> {
        let runner = self
            .sessions
            .upgrade()
            .ok_or_else(|| HarnessError::unavailable("The session runner is stopping."))?;
        let operation = runner.operation_lock();
        let _operation = operation
            .try_lock()
            .map_err(|_| HarnessError::conflict("A workspace operation is completing."))?;
        runner.idle()?;
        let _serial = self
            .serial
            .try_lock()
            .map_err(|_| HarnessError::conflict("Accelerator preparation is busy."))?;
        let p = self.store.project(project)?;
        let path = PathBuf::from(p["path"].as_str().unwrap_or(""));
        let path = attach::independent_project(&self.source, &path)?;
        let edition = edition
            .map(str::to_owned)
            .or(attach::detect(&path)?.0)
            .ok_or_else(|| {
                HarnessError::invalid("No accelerator edition fits this project. Choose one.")
            })?;
        if !EDITIONS.contains(&edition.as_str()) {
            return Err(HarnessError::invalid("Choose a supported edition."));
        }
        if Self::installed(&path)? {
            return Err(HarnessError::conflict(
                "This project already has an installed accelerator.",
            ));
        }
        attach::prepare_state(&self.source, &path, &self.state(project)?, &edition)?;
        self.store.connection()?.execute("INSERT INTO project_accelerators(project_id,edition,attached_at)VALUES(?1,?2,?3)ON CONFLICT(project_id)DO UPDATE SET edition=excluded.edition,attached_at=excluded.attached_at",params![project,edition,store::now()])?;
        self.get(project)
    }
    pub fn detach(&self, project: &str) -> Result<Value> {
        let runner = self
            .sessions
            .upgrade()
            .ok_or_else(|| HarnessError::unavailable("The runner is stopping."))?;
        let operation = runner.operation_lock();
        let _operation = operation
            .try_lock()
            .map_err(|_| HarnessError::conflict("A workspace operation is completing."))?;
        runner.idle()?;
        self.store.project(project)?;
        self.store.connection()?.execute(
            "DELETE FROM project_accelerators WHERE project_id=?1",
            [project],
        )?;
        self.get(project)
    }
    pub fn auto_attach(&self, project: &str) -> Result<()> {
        let p = self.store.project(project)?;
        let root = Path::new(p["path"].as_str().unwrap_or(""));
        if self.edition(project)?.is_some() || Self::installed(root)? {
            return Ok(());
        }
        if let Some(edition) = attach::detect(root)?.0 {
            self.attach(project, Some(&edition))?;
        }
        Ok(())
    }
    pub fn overlay(&self, project: &str, host: &str, cwd: &Path) -> Result<Option<Overlay>> {
        self.store.project(project)?;
        if Self::installed(cwd)? {
            return Ok(None);
        }
        let Some(edition) = self.edition(project)? else {
            return Ok(None);
        };
        Ok(Some(attach::build_overlay(
            &self.source,
            cwd,
            &self.state(project)?,
            &edition,
            host,
        )?))
    }
    pub fn overlay_for_workspace(&self, root: &Path, host: &str) -> Result<Option<Overlay>> {
        let root = std::fs::canonicalize(root)?;
        let spelling = root.to_string_lossy().into_owned();
        let project = {
            let db = self.store.connection()?;
            let mut q=db.prepare("SELECT DISTINCT project_id FROM sessions WHERE project_path=?1 ORDER BY project_id")?;
            let values = q
                .query_map([&spelling], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if values.len() > 1 {
                return Err(HarnessError::conflict(
                    "Workspace is associated with multiple registered projects.",
                ));
            }
            values.into_iter().next()
        };
        let project = if let Some(project) = project {
            Some(project)
        } else {
            self.store
                .projects()?
                .iter()
                .find(|p| p["path"].as_str() == Some(&spelling))
                .and_then(|p| p["id"].as_str())
                .map(str::to_owned)
        };
        project
            .map(|p| self.overlay(&p, host, &root))
            .transpose()
            .map(Option::flatten)
    }
    pub fn source_version(&self) -> Result<String> {
        let mut cached = self
            .version
            .lock()
            .map_err(|_| HarnessError::unavailable("Accelerator version cache unavailable."))?;
        if let Some((at, value)) = &*cached
            && at.elapsed() < Duration::from_secs(30)
        {
            return Ok(value.clone());
        }
        let head = crate::projects::git(&self.source, &["rev-parse".into(), "HEAD".into()], 20)?;
        let mut version = if head.code == 0 {
            infra_core::domain::rstrip(&String::from_utf8_lossy(&head.stdout)).to_owned()
        } else {
            "no-git".into()
        };
        let mut args = vec![
            "status".into(),
            "--porcelain".into(),
            "-z".into(),
            "--".into(),
            "install/inventories".into(),
        ];
        args.extend(EDITIONS.map(str::to_owned));
        let dirty = crate::projects::git(&self.source, &args, 20)?;
        if dirty.code == 0 && !dirty.stdout.is_empty() {
            version.push('+');
            version.push_str(&store::sha(&dirty.stdout)[..12]);
        }
        *cached = Some((Instant::now(), version.clone()));
        Ok(version)
    }
    pub fn keep_current(&self, project: &str) -> Result<Option<Value>> {
        let p = self.store.project(project)?;
        let path = PathBuf::from(p["path"].as_str().unwrap_or(""));
        if !Self::installed(&path)? {
            return Ok(None);
        }
        let version = self.source_version()?;
        let previous: Option<String> = self
            .store
            .connection()?
            .query_row(
                "SELECT source FROM accelerator_syncs WHERE project_id=?1",
                [project],
                |r| r.get(0),
            )
            .optional()?;
        if previous.as_deref() == Some(&version) {
            return Ok(None);
        }
        let _serial = self
            .serial
            .try_lock()
            .map_err(|_| HarnessError::conflict("Accelerator update is busy."))?;
        // Hook approvals are written only by the explicit review action. An update never grants trust.
        let report = match Installer::new(&self.source)?.sync(&path, &InstallOptions::default()) {
            Ok(report) => report,
            Err(error) => {
                json!({"target":path,"changed":[],"kept":[],"backups":[],"error":error.to_string()})
            }
        };
        self.store.connection()?.execute("INSERT INTO accelerator_syncs(project_id,source,synced_at,report)VALUES(?1,?2,?3,?4)ON CONFLICT(project_id)DO UPDATE SET source=excluded.source,synced_at=excluded.synced_at,report=excluded.report",params![project,version,store::now(),report.to_string()])?;
        Ok(Some(report))
    }
    pub fn keep_all_current(&self) -> Result<Vec<Value>> {
        let mut reports = vec![];
        for p in self.store.projects()? {
            let id = p["id"].as_str().unwrap_or("");
            match self.keep_current(id) {
                Ok(Some(report)) => reports.push(json!({"project_id":id,"report":report})),
                Ok(None) => {}
                Err(error) => {
                    reports.push(json!({"project_id":id,"report":{"error":error.message}}))
                }
            }
        }
        Ok(reports)
    }
    pub fn last_sync(&self, project: &str) -> Result<Value> {
        let row: Option<(String, String, String)> = self
            .store
            .connection()?
            .query_row(
                "SELECT source,synced_at,report FROM accelerator_syncs WHERE project_id=?1",
                [project],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((source, at, raw)) = row else {
            return Ok(Value::Null);
        };
        let report = crate::strict_json::parse(raw.as_bytes(), 8 * 1024 * 1024)?;
        Ok(
            json!({"source":source,"synced_at":at,"edition":report["edition"],"release":report["release"],"changed":report["changed"].as_array().map(Vec::len).unwrap_or(0),"kept":report["kept"],"backups":report["backups"],"error":report["error"],"partial":report["partial"],"codex_trust":report["codex_trust"]}),
        )
    }
    pub fn codex_hooks(&self, project: &str, approve: bool) -> Result<Value> {
        let info = self.get(project)?;
        if info["mode"] != "attached" {
            return Err(HarnessError::invalid(
                "Attach an accelerator before reviewing its Codex hooks.",
            ));
        }
        let p = self.store.project(project)?;
        let path = PathBuf::from(p["path"].as_str().unwrap_or(""));
        let exe = self
            .providers
            .get("codex")?
            .executable
            .ok_or_else(|| HarnessError::unavailable("Codex CLI is unavailable."))?;
        let _serial = self
            .serial
            .try_lock()
            .map_err(|_| HarnessError::conflict("Hook review is busy."))?;
        let overlay = self
            .overlay(project, "codex", &path)?
            .ok_or_else(|| HarnessError::invalid("The project is not attached."))?;
        let result = attach::hook_trust(
            exe.to_str()
                .ok_or_else(|| HarnessError::invalid("CLI path is not UTF-8."))?,
            &overlay,
            &path,
            approve,
        )?;
        let entries = result["entries"].as_array().cloned().unwrap_or_default();
        let trusted = entries
            .iter()
            .filter(|v| ["trusted", "managed"].contains(&v["status"].as_str().unwrap_or("")))
            .count();
        Ok(json!({"project_id":project,"hooks":entries,"trusted":trusted,"total":entries.len()}))
    }
    pub fn startup(&self) -> Result<Value> {
        let mut source = PublicationFiles::new(&self.source)?;
        let config = source
            .read("scripts/token_budget.json", 1024 * 1024)?
            .map(|b| crate::strict_json::parse(&b, 1024 * 1024))
            .transpose()?
            .unwrap_or(json!({}));
        let ratios = [
            ("agents_md_bytes", 4.76),
            ("descriptor_bytes", 4.91),
            ("command_bytes", 5.10),
            ("agent_bytes", 5.14),
        ];
        let mut rows = vec![];
        for edition in EDITIONS {
            match infra_tooling::budget::measure(&self.source,edition){Ok(measured)=>{
   let mut bytes=json!({});let mut ceilings=json!({});let mut total=0u64;let mut ceiling_total=Some(0u64);let mut tokens=0u64;
   for(key,ratio)in ratios{bytes[key]=measured[key].clone();ceilings[key]=config["editions"][if edition=="Cms/wordpress"{"WordPress"}else{edition}][key].clone();total+=measured[key].as_u64().unwrap_or(0);ceiling_total=ceiling_total.and_then(|v|ceilings[key].as_u64().and_then(|n|v.checked_add(n)));tokens+=(measured[key].as_u64().unwrap_or(0)as f64/ratio).round_ties_even()as u64;}
   rows.push(json!({"edition":edition,"skills":measured["skills"],"bytes":bytes,"total":total,"ceilings":ceilings,"ceiling_total":ceiling_total,"tokens":tokens}));
  },Err(error)=>rows.push(json!({"edition":edition,"error":error.to_string().chars().take(300).collect::<String>()}))}
        }
        Ok(
            json!({"editions":rows,"labels":{"agents_md_bytes":"AGENTS.md","descriptor_bytes":"skill descriptors","command_bytes":"command listing","agent_bytes":"agent listing"},"ratios":ratios.into_iter().collect::<BTreeMap<_,_>>(),"calibration":"docs/TOKEN-ECONOMY-RESEARCH.md as of 9435dfc1^"}),
        )
    }
}
impl Domain for Accelerators {
    fn execute<'a>(&'a self, op: &'a str, request: Value) -> Reply<'a> {
        Box::pin(async move {
            let project = request["params"]["project_id"].as_str().unwrap_or("");
            let body = &request["body"];
            let value = match op {
                "accelerators.get" => self.get(project)?,
                "accelerators.registered" => {
                    self.auto_attach(project)?;
                    self.keep_current(project)?;
                    self.get(project)?
                }
                "accelerators.startup" => self.startup()?,
                "accelerators.codex_hooks" => self.codex_hooks(project, false)?,
                "accelerators.trust_codex_hooks" => {
                    if !store::exact(body, &[], &[]) {
                        return Err(HarnessError::invalid(
                            "Hook approval requires an empty object.",
                        ));
                    }
                    self.codex_hooks(project, true)?
                }
                "accelerators.act" => {
                    if !store::exact(body, &["action"], &["edition"]) {
                        return Err(HarnessError::invalid("Invalid accelerator action."));
                    }
                    match body["action"].as_str() {
                        Some("attach") => self.attach(project, body["edition"].as_str())?,
                        Some("detach") => self.detach(project)?,
                        _ => return Err(HarnessError::invalid("Choose attach or detach.")),
                    }
                }
                _ => return Err(HarnessError::missing("Unknown accelerator operation.")),
            };
            Ok(Payload::Json(value))
        })
    }
}

pub struct ProjectLaunchAdapter {
    pub accelerators: Arc<Accelerators>,
    pub project_id: String,
}
impl infra_host::provider::LaunchAdapter for ProjectLaunchAdapter {
    fn adapt(
        &self,
        input: &infra_host::provider::Launch,
        mut command: infra_host::provider::PreparedCommand,
    ) -> Result<infra_host::provider::PreparedCommand> {
        let project = self.accelerators.store.project(&self.project_id)?;
        let registered = PathBuf::from(
            project["path"]
                .as_str()
                .ok_or_else(|| HarnessError::invalid("Registered project path missing."))?,
        );
        let requested = std::fs::canonicalize(&input.project)?;
        if requested != std::fs::canonicalize(&registered)? {
            let a = infra_core::git::GitFrame::discover(&registered)?.ok_or_else(|| {
                HarnessError::forbidden("Session workspace is outside the registered project.")
            })?;
            let b = infra_core::git::GitFrame::discover(&requested)?.ok_or_else(|| {
                HarnessError::forbidden("Session workspace is outside the registered checkout.")
            })?;
            if a.common_dir()? != b.common_dir()? || a.prefix_bytes != b.prefix_bytes {
                return Err(HarnessError::forbidden(
                    "Session workspace belongs to another project.",
                ));
            }
        }
        if let Some(overlay) =
            self.accelerators
                .overlay(&self.project_id, &input.provider, &requested)?
        {
            command.argv = attach::apply_overlay(
                &input.provider,
                &command.argv,
                &overlay,
                input.session.is_none(),
            )?;
            for key in ["ACCELERATOR_HOME", "ACCELERATOR_STATE_DIR"] {
                if let Some(value) = overlay.environment.get(key) {
                    command.readable.push(PathBuf::from(value));
                }
            }
            command.environment.extend(overlay.environment);
        }
        if input.provider == "claude" && input.session.is_none() {
            let db = self.accelerators.store.connection()?;
            let mut q=db.prepare("SELECT id FROM sessions WHERE project_id=?1 AND project_path=?2 AND native_session_id IS NULL AND status='running'")?;
            let ids = q
                .query_map(params![self.project_id, requested.to_string_lossy()], |r| {
                    r.get::<_, String>(0)
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if ids.len() == 1 {
                for dir in crate::chat_merge::ChatMerge::new(self.accelerators.store.clone())
                    .directories(&ids[0])
                {
                    command
                        .argv
                        .extend(["--add-dir".into(), dir.to_string_lossy().into_owned()]);
                }
            }
        }
        Ok(command)
    }
}

pub struct EvidenceLaunchAdapter {
    pub workspace: PathBuf,
}
impl infra_host::provider::LaunchAdapter for EvidenceLaunchAdapter {
    fn adapt(
        &self,
        input: &infra_host::provider::Launch,
        mut command: infra_host::provider::PreparedCommand,
    ) -> Result<infra_host::provider::PreparedCommand> {
        let workspace = std::fs::canonicalize(&self.workspace)?;
        if std::fs::canonicalize(&input.project)? != workspace {
            return Err(HarnessError::forbidden(
                "Discovery launch is outside its owned evidence workspace.",
            ));
        }
        for key in [
            "ACCELERATOR_HOME",
            "ACCELERATOR_STATE_DIR",
            "ACCELERATOR_PROJECT_DIR",
            "CONTEXT_TASK_ID",
            "CONTEXT_CAPSULE_DELIVERED",
            "CONTEXT_MEMORY_RECOVERY",
        ] {
            command.environment.remove(key);
            command.remove.push(key.into());
        }
        if command
            .environment
            .keys()
            .any(|k| k.starts_with("ACCELERATOR_"))
        {
            return Err(HarnessError::forbidden(
                "Discovery cannot inherit an attached accelerator boundary.",
            ));
        }
        command.readable = vec![];
        Ok(command)
    }
}
