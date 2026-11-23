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
    time::{SystemTime, UNIX_EPOCH},
};
const EDITIONS: [&str; 4] = ["Laravel", "Symfony", "PHP Core", "Cms/wordpress"];
const ACCEL_RACY_NS: i128 = 2_000_000_000;
const MAX_SOURCE_STAMP_ENTRIES: usize = 8192;

pub struct Accelerators {
    pub store: SessionStore,
    pub source: PathBuf,
    pub providers: Arc<ProviderDomain>,
    sessions: Weak<Sessions>,
    serial: Mutex<()>,
    version: Mutex<Option<(String, String)>>,
    #[cfg(test)]
    source_version_recomputes: std::sync::atomic::AtomicUsize,
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
            #[cfg(test)]
            source_version_recomputes: std::sync::atomic::AtomicUsize::new(0),
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
        let sc_runtime = runner.skillcache_runtime();
        let _ = crate::skillcache::compile::admit_textual_templates_sync(
            &sc_runtime,
            project,
            &path,
            &edition,
        );
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let project_owned = project.to_owned();
            let workspace_owned = path.clone();
            let edition_owned = edition.clone();
            handle.spawn(async move {
                let _ = crate::skillcache::compile::admit_edition_templates_for_project(
                    &sc_runtime,
                    &project_owned,
                    &workspace_owned,
                    &edition_owned,
                )
                .await;
            });
        }
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

    /// Resolves and verifies a canonical workspace path owned by a registered project,
    /// enabling Gate-1 read-only program auto-approval for `session_id` on `desk`.
    pub fn register_read_only_auto_rules(
        &self,
        desk: &crate::permissions::PermissionDesk,
        session_id: &str,
        workspace: &Path,
    ) -> Result<()> {
        let canon = infra_core::fs::canonicalize(workspace)?;
        let registered = self.store.projects()?.into_iter().any(|p| {
            p["path"]
                .as_str()
                .and_then(|s| infra_core::fs::canonicalize(s).ok())
                .is_some_and(|proj| canon == proj || canon.starts_with(&proj))
        });
        if !registered {
            return Err(HarnessError::forbidden(
                "Read-only rule workspace is not registered with Harness.",
            ));
        }
        crate::permissions::warm_read_only_cache(&canon);
        desk.enable_read_only_auto_rules_with_store(session_id, canon, self.store.clone());
        Ok(())
    }

    /// Computes a deterministic scoped `session_rule_key` (`readonly:program:...`) for a
    /// verified read-only CLI command inside `workspace`.
    pub fn read_only_session_rule_key(
        workspace: &Path,
        command: &str,
    ) -> std::result::Result<Option<String>, String> {
        crate::permissions::classify_read_only_program_command(Some(workspace), command)
    }

    /// Computes a deterministic scoped `session_rule_key` (`readonly:program:...`) for a
    /// verified read-only CLI or active gate-verified capsule program inside `workspace`.
    pub fn read_only_session_rule_key_in_store(
        &self,
        workspace: &Path,
        command: &str,
    ) -> std::result::Result<Option<String>, String> {
        crate::permissions::classify_read_only_program_command_in_store(
            Some(workspace),
            Some(&self.store),
            command,
        )
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
        let root = infra_core::fs::canonicalize(root)?;
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
}

fn accel_time_ns(time: SystemTime) -> Option<i128> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i128::try_from(d.as_nanos()).ok())
}

fn accel_is_racy(mtime_ns: i128, now_ns: i128) -> bool {
    mtime_ns <= 0 || now_ns <= ACCEL_RACY_NS || (now_ns - mtime_ns).abs() < ACCEL_RACY_NS
}

fn accel_racy_file_stamp(path: &Path, max_hash_bytes: u64, now_ns: i128) -> Option<String> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Some("missing".into()),
        Err(_) => return None,
    };
    if meta.file_type().is_symlink() || !meta.is_file() {
        return None;
    }
    let mtime_ns = meta.modified().ok().and_then(accel_time_ns)?;
    if accel_is_racy(mtime_ns, now_ns) {
        if meta.len() > max_hash_bytes {
            return None;
        }
        let bytes = std::fs::read(path).ok()?;
        Some(format!(
            "racy:{}:{mtime_ns}:{}",
            meta.len(),
            store::sha(&bytes)
        ))
    } else {
        Some(format!("settled:{}:{mtime_ns}", meta.len()))
    }
}

fn resolve_accel_git_dirs(root: &Path) -> Option<(PathBuf, PathBuf)> {
    let dot_git = root.join(".git");
    let meta = std::fs::symlink_metadata(&dot_git).ok()?;
    let git_dir = if meta.is_dir() {
        dot_git
    } else if meta.is_file() {
        let content = std::fs::read_to_string(&dot_git).ok()?;
        let rel = content.trim().strip_prefix("gitdir:")?.trim();
        let cand = Path::new(rel);
        if cand.is_absolute() {
            cand.to_path_buf()
        } else {
            root.join(cand)
        }
    } else {
        return None;
    };
    let commondir_file = git_dir.join("commondir");
    let common_dir = if let Ok(raw) = std::fs::read_to_string(&commondir_file) {
        let rel = raw.trim();
        let p = Path::new(rel);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            git_dir.join(p)
        }
    } else {
        git_dir.clone()
    };
    Some((git_dir, common_dir))
}

fn stamp_accel_subdir(root: &Path, now_ns: i128, out: &mut Vec<String>) -> bool {
    let meta = match std::fs::symlink_metadata(root) {
        Ok(m) => m,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            out.push(format!("missing:{}", root.to_string_lossy()));
            return true;
        }
        Err(_) => return false,
    };
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return false;
    }
    let mut queue = std::collections::VecDeque::from([root.to_path_buf()]);
    while let Some(dir) = queue.pop_front() {
        let d_mtime = std::fs::symlink_metadata(&dir)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(accel_time_ns)
            .unwrap_or(0);
        out.push(format!("d:{}:{d_mtime}", dir.to_string_lossy()));
        if out.len() > MAX_SOURCE_STAMP_ENTRIES {
            return false;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return false;
        };
        let mut children: Vec<_> = rd.filter_map(|e| e.ok()).collect();
        children.sort_by_key(|e| e.file_name());
        for child in children {
            let p = child.path();
            let Ok(c_meta) = std::fs::symlink_metadata(&p) else {
                return false;
            };
            if c_meta.file_type().is_symlink() {
                return false;
            }
            if c_meta.is_dir() {
                queue.push_back(p);
            } else if c_meta.is_file() {
                let Some(mtime_ns) = c_meta.modified().ok().and_then(accel_time_ns) else {
                    return false;
                };
                let mut rec = format!("f:{}:{}:{mtime_ns}", p.to_string_lossy(), c_meta.len());
                if accel_is_racy(mtime_ns, now_ns) {
                    if c_meta.len() > 256 * 1024 {
                        return false;
                    }
                    let Ok(bytes) = std::fs::read(&p) else {
                        return false;
                    };
                    rec.push(':');
                    rec.push_str(&store::sha(&bytes));
                }
                out.push(rec);
                if out.len() > MAX_SOURCE_STAMP_ENTRIES {
                    return false;
                }
            }
        }
    }
    true
}

fn source_version_stamp_at(source: &Path, now_ns: i128) -> Option<String> {
    let Some((git_dir, common_dir)) = resolve_accel_git_dirs(source) else {
        let root_mtime = std::fs::symlink_metadata(source)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(accel_time_ns)
            .unwrap_or(0);
        return (!accel_is_racy(root_mtime, now_ns)).then(|| format!("no-git:{root_mtime}"));
    };
    let head_path = git_dir.join("HEAD");
    let head_stamp = accel_racy_file_stamp(&head_path, 64 * 1024, now_ns)?;
    let mut parts = vec![
        format!("HEAD={head_stamp}"),
        format!(
            "index={}",
            accel_racy_file_stamp(&git_dir.join("index"), 16 * 1024 * 1024, now_ns)?
        ),
        format!(
            "packed-refs={}",
            accel_racy_file_stamp(&common_dir.join("packed-refs"), 8 * 1024 * 1024, now_ns)?
        ),
    ];
    if let Ok(head_text) = std::fs::read_to_string(&head_path)
        && let Some(ref_rel) = head_text.trim().strip_prefix("ref: ").map(str::trim)
        && !ref_rel.is_empty()
        && !ref_rel.contains("..")
    {
        let ref_path = common_dir.join(ref_rel);
        parts.push(format!(
            "ref={}",
            accel_racy_file_stamp(&ref_path, 4096, now_ns)?
        ));
    }
    if !stamp_accel_subdir(&source.join("install/inventories"), now_ns, &mut parts) {
        return None;
    }
    for edition in EDITIONS {
        if !stamp_accel_subdir(&source.join(edition), now_ns, &mut parts) {
            return None;
        }
    }
    Some(store::sha(parts.join("\n").as_bytes()))
}

impl Accelerators {
    pub fn source_version(&self) -> Result<String> {
        let now_ns = accel_time_ns(SystemTime::now()).unwrap_or(0);
        self.source_version_at(now_ns)
    }

    fn source_version_at(&self, now_ns: i128) -> Result<String> {
        let stamp = source_version_stamp_at(&self.source, now_ns);
        {
            let cached = self
                .version
                .lock()
                .map_err(|_| HarnessError::unavailable("Accelerator version cache unavailable."))?;
            if let Some(ref current_stamp) = stamp
                && let Some((cached_stamp, value)) = &*cached
                && cached_stamp == current_stamp
            {
                return Ok(value.clone());
            }
        }
        #[cfg(test)]
        self.source_version_recomputes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
        let after_ns = accel_time_ns(SystemTime::now()).unwrap_or(now_ns);
        if let Some(after_stamp) = source_version_stamp_at(&self.source, after_ns.max(now_ns))
            && Some(&after_stamp) == stamp.as_ref()
            && let Ok(mut cached) = self.version.lock()
        {
            *cached = Some((after_stamp, version.clone()));
        }
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
    /// The Harness session this launch serves; a system dispatch has none.
    pub session_id: Option<String>,
}
impl infra_host::provider::LaunchAdapter for ProjectLaunchAdapter {
    fn adapt(
        &self,
        input: &infra_host::provider::Launch,
        mut command: infra_host::provider::PreparedCommand,
    ) -> Result<infra_host::provider::PreparedCommand> {
        if let Some(session_id) = &self.session_id
            && let Ok(session) = self.accelerators.store.get(session_id)
            && session["system_explain"].is_object()
        {
            return crate::system_graph::adapt_system_explain_launch(
                &self.accelerators.store,
                &session,
                input,
                command,
            );
        }
        let project = self.accelerators.store.project(&self.project_id)?;
        let registered = PathBuf::from(
            project["path"]
                .as_str()
                .ok_or_else(|| HarnessError::invalid("Registered project path missing."))?,
        );
        let requested = infra_core::fs::canonicalize(&input.project)?;
        if requested != infra_core::fs::canonicalize(&registered)? {
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
            // Cursor's ACP server takes its prompt over the protocol, so the
            // first turn's policy preamble goes there.
            if input.provider == "cursor"
                && input.session.is_none()
                && command.argv.last().is_some_and(|value| value == "acp")
            {
                command.prompt_prefix = overlay.prompt_prefix.clone();
            }
            for key in ["ACCELERATOR_HOME", "ACCELERATOR_STATE_DIR"] {
                if let Some(value) = overlay.environment.get(key) {
                    command.readable.push(PathBuf::from(value));
                }
            }
            command.environment.extend(overlay.environment);
        }
        if self.session_id.is_some() {
            // A session's prompt offers the project's published code graph as
            // three commands; the launch lets the host run exactly those.
            crate::graph::prepare(
                &self.accelerators.store,
                &self.project_id,
                &input.provider,
                &mut command,
            )?;
        }
        if input.provider == "claude"
            && let Some(session) = &self.session_id
        {
            // A merged task's first prompt names its archive, and every prompt
            // lists each upload, by a path under the Harness state. Claude Code
            // reads outside its working folder without asking only in Plan
            // mode; in Edit mode it refuses that path unless the launch names
            // the folder, on a first message and on every resumed one, whose
            // conversation still holds those paths. Only this session's own
            // folders are named, so a launch without a session names none.
            let store = &self.accelerators.store;
            let archive = crate::chat_merge::ChatMerge::new(store.clone()).directories(session);
            for dir in archive
                .into_iter()
                .chain(crate::attachments::directories(store, session)?)
            {
                command
                    .argv
                    .extend(["--add-dir".into(), dir.to_string_lossy().into_owned()]);
            }
        }
        Ok(command)
    }
}

/// Launches of a system run. One run dispatches into several registered projects
/// (the system folder, then each service folder), so the project is the one each
/// dispatch root belongs to, not the session's: a root that is a registered
/// project is launched exactly as that project's sessions are; a root inside one
/// (a monorepo service folder) gets no overlay. The run's roots were already
/// admitted against registered projects when its catalog was read.
pub struct SystemLaunchAdapter {
    pub accelerators: Arc<Accelerators>,
}
impl infra_host::provider::LaunchAdapter for SystemLaunchAdapter {
    fn adapt(
        &self,
        input: &infra_host::provider::Launch,
        command: infra_host::provider::PreparedCommand,
    ) -> Result<infra_host::provider::PreparedCommand> {
        let requested = infra_core::fs::canonicalize(&input.project)?;
        let mut inside = false;
        for project in self.accelerators.store.projects()? {
            let (Some(id), Some(path)) = (project["id"].as_str(), project["path"].as_str()) else {
                continue;
            };
            let Ok(path) = infra_core::fs::canonicalize(path) else {
                continue;
            };
            if path == requested {
                return ProjectLaunchAdapter {
                    accelerators: self.accelerators.clone(),
                    project_id: id.into(),
                    session_id: None,
                }
                .adapt(input, command);
            }
            inside |= requested.starts_with(&path);
        }
        if !inside {
            return Err(HarnessError::forbidden(
                "System dispatch folder is not inside a registered project.",
            ));
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
        let workspace = infra_core::fs::canonicalize(&self.workspace)?;
        if infra_core::fs::canonicalize(&input.project)? != workspace {
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

#[cfg(test)]
mod system_launch_tests {
    use super::*;
    use infra_host::provider::{Launch, LaunchAdapter, PreparedCommand};
    use std::{
        collections::BTreeMap,
        sync::{RwLock, Weak},
    };
    fn launch(project: &Path) -> Launch {
        Launch {
            provider: "codex".into(),
            executable: PathBuf::from("/bin/true"),
            project: project.into(),
            prompt: "dispatch".into(),
            mode: "plan".into(),
            model: None,
            session: None,
            effort: None,
            agents: false,
            count: 1,
            usd: None,
            hook_events: false,
            permissions: None,
            approvals: None,
        }
    }
    fn command() -> PreparedCommand {
        PreparedCommand {
            argv: vec!["codex".into(), "exec".into()],
            environment: BTreeMap::new(),
            input: vec![],
            readable: vec![],
            remove: vec![],
            prompt_prefix: String::new(),
        }
    }
    /// A prompt lists every upload by its path under the Harness state. Claude
    /// Code in Edit mode refuses to read there unless the launch names the
    /// folder, so a context or file attached to an Edit chat never reached it.
    #[test]
    fn a_claude_launch_names_the_folders_of_its_session_uploads() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let store = SessionStore::open(&root.path().join("state")).unwrap();
        let registered = store.register(&project).unwrap();
        let project_id = registered["id"].as_str().unwrap().to_owned();
        store
            .connection()
            .unwrap()
            .execute(
                "INSERT INTO sessions(id,title,project_id,project_path,provider,mode,workflow,project_context,status,created_at,updated_at) VALUES('chat','Example',?1,?2,'claude','edit','native',0,'running','created','updated')",
                params![project_id, registered["path"].as_str().unwrap()],
            )
            .unwrap();
        let saved = crate::attachments::save(
            &store,
            "chat",
            crate::attachments::validate(&json!([
                {"name":"context-example-0a1b2c3d.md","data":"IyBDb250ZXh0IGZyb20gYSB0YXNrCg=="},
                {"name":"notes.txt","data":"bm90ZXM="}
            ]))
            .unwrap(),
        )
        .unwrap();
        let named: Vec<String> = saved
            .iter()
            .flat_map(|item| {
                let folder = store
                    .state
                    .join("attachments")
                    .join(item["id"].as_str().unwrap());
                [
                    "--add-dir".to_owned(),
                    folder.to_string_lossy().into_owned(),
                ]
            })
            .collect();
        let providers = Arc::new(ProviderDomain {
            providers: Arc::new(RwLock::new(BTreeMap::new())),
        });
        let accelerators =
            Arc::new(Accelerators::new(store, root.path().into(), providers, Weak::new()).unwrap());
        let adapter = |session: Option<&str>| ProjectLaunchAdapter {
            accelerators: accelerators.clone(),
            project_id: project_id.clone(),
            session_id: session.map(String::from),
        };
        let claude = |resumed: Option<&str>| Launch {
            provider: "claude".into(),
            mode: "edit".into(),
            session: resumed.map(String::from),
            ..launch(&project)
        };
        // The first message and every resumed one: a resumed conversation still
        // holds the paths its earlier messages listed.
        for resumed in [None, Some("native-session")] {
            let argv = adapter(Some("chat"))
                .adapt(&claude(resumed), command())
                .unwrap()
                .argv;
            assert_eq!(argv[..2], ["codex", "exec"]);
            assert_eq!(argv[2..], named, "resumed: {resumed:?}");
        }
        // Codex reads what its sandbox shows it, and a launch of another session
        // or of none names nothing.
        for (session, launch) in [
            (Some("chat"), launch(&project)),
            (Some("other-chat"), claude(None)),
            (None, claude(None)),
        ] {
            assert_eq!(
                adapter(session).adapt(&launch, command()).unwrap().argv,
                ["codex", "exec"],
                "{session:?} {}",
                launch.provider
            );
        }
    }

    /// A merged task's first prompt names its archive under the Harness state,
    /// and the conversation keeps that path. Claude Code in Edit mode reads it
    /// only when the launch names the folder, so a follow-up that sent the
    /// agent back to the archive was refused while the chat still completed.
    #[test]
    fn a_claude_launch_names_the_archive_folder_of_its_merged_task() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let store = SessionStore::open(&root.path().join("state")).unwrap();
        let registered = store.register(&project).unwrap();
        let project_id = registered["id"].as_str().unwrap().to_owned();
        let chat = |prompt: &str, reply: Option<&str>| {
            let options = json!({"project_id":project_id,"provider":"claude","mode":"edit"});
            let created = store.create(&options, prompt, &[]).unwrap();
            let sid = created["id"].as_str().unwrap().to_owned();
            if let Some(reply) = reply {
                store
                    .event(&sid, &json!({"kind":"text","text":reply}))
                    .unwrap();
                store.patch(&sid, &json!({"status":"completed"})).unwrap();
            }
            sid
        };
        let sources = [
            chat("Why is the total off?", Some("Lines are rounded early.")),
            chat("Where is the total summed?", Some("In src/Invoice.php.")),
        ];
        let merger = crate::chat_merge::ChatMerge::new(store.clone());
        let merged = merger
            .merge(
                &json!({
                    "source_ids": sources,
                    "request_id": store::id(),
                    "destination": {"project_id":project_id,"provider":"claude","mode":"edit","prompt":"Compare both diagnoses."},
                }),
                |destination, _| Ok(destination.clone()),
            )
            .unwrap();
        let merged = merged["id"].as_str().unwrap().to_owned();
        let archive = store.state.join("merges").join(&merged);
        assert!(archive.join("context.json").is_file());
        let upload = crate::attachments::save(
            &store,
            &merged,
            crate::attachments::validate(&json!([{"name":"notes.txt","data":"bm90ZXM="}])).unwrap(),
        )
        .unwrap();
        let upload = store
            .state
            .join("attachments")
            .join(upload[0]["id"].as_str().unwrap());
        let named = |folders: &[&Path]| -> Vec<String> {
            ["codex", "exec"]
                .into_iter()
                .map(String::from)
                .chain(folders.iter().flat_map(|folder| {
                    [
                        "--add-dir".to_owned(),
                        folder.to_string_lossy().into_owned(),
                    ]
                }))
                .collect()
        };
        let providers = Arc::new(ProviderDomain {
            providers: Arc::new(RwLock::new(BTreeMap::new())),
        });
        let accelerators = Arc::new(
            Accelerators::new(store.clone(), root.path().into(), providers, Weak::new()).unwrap(),
        );
        let adapter = |session: Option<&str>| ProjectLaunchAdapter {
            accelerators: accelerators.clone(),
            project_id: project_id.clone(),
            session_id: session.map(String::from),
        };
        let claude = |resumed: Option<&str>| Launch {
            provider: "claude".into(),
            mode: "edit".into(),
            session: resumed.map(String::from),
            ..launch(&project)
        };
        let argv = |session: Option<&str>, launch: &Launch| {
            adapter(session).adapt(launch, command()).unwrap().argv
        };
        // Its first message, as the runner launches it: running, no native session yet.
        store.patch(&merged, &json!({"status":"running"})).unwrap();
        assert_eq!(
            argv(Some(&merged), &claude(None)),
            named(&[&archive, &upload])
        );
        // The archive belongs to that task alone: a launch for no session (a
        // system dispatch into the same project) and one for another chat name
        // nothing, even while the merged task is the only one starting.
        let other = chat("An unrelated question.", None);
        assert_eq!(argv(None, &claude(None)), named(&[]));
        assert_eq!(argv(Some(&other), &claude(None)), named(&[]));
        // Another chat of the project starting at the same moment takes nothing
        // from the merged task.
        store.patch(&other, &json!({"status":"running"})).unwrap();
        assert_eq!(
            argv(Some(&merged), &claude(None)),
            named(&[&archive, &upload])
        );
        // Every resumed message: the conversation still holds the archive path.
        store
            .patch(
                &merged,
                &json!({"status":"completed","native_session_id":"native-session"}),
            )
            .unwrap();
        assert_eq!(
            argv(Some(&merged), &claude(Some("native-session"))),
            named(&[&archive, &upload])
        );
        // Codex reads what its sandbox shows it.
        assert_eq!(argv(Some(&merged), &launch(&project)), named(&[]));
        // A deleted saved context leaves no folder to name.
        merger.delete(&merged).unwrap();
        assert!(!archive.exists());
        assert_eq!(
            argv(Some(&merged), &claude(Some("native-session"))),
            named(&[&upload])
        );
    }

    /// A session's prompt offers the project's published code graph as three
    /// commands. Claude Code in Edit mode refused them, a launch without a
    /// terminal having nobody to ask, so a session's launch permits exactly
    /// those. A system dispatch, whose prompt offers no graph, is left alone.
    #[test]
    fn a_session_launch_permits_the_graph_questions_its_prompt_offers() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let store = SessionStore::open(&root.path().join("state")).unwrap();
        let project_id = store.register(&project).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        crate::graph::publish_for_tests(&store, &project_id, &root.path().join("graphify-tool"));
        let rules = crate::graph::grant(&store, &project_id)
            .unwrap()
            .unwrap()
            .rules;
        let providers = Arc::new(ProviderDomain {
            providers: Arc::new(RwLock::new(BTreeMap::new())),
        });
        let accelerators =
            Arc::new(Accelerators::new(store, root.path().into(), providers, Weak::new()).unwrap());
        let prepared = |session: Option<&str>, provider: &str| {
            let mut command = command();
            command.argv = vec![provider.into(), "--settings".into(), "{}".into()];
            ProjectLaunchAdapter {
                accelerators: accelerators.clone(),
                project_id: project_id.clone(),
                session_id: session.map(String::from),
            }
            .adapt(
                &Launch {
                    provider: provider.into(),
                    mode: "edit".into(),
                    ..launch(&project)
                },
                command,
            )
            .unwrap()
        };
        let claude = prepared(Some("chat"), "claude");
        let settings: Value = serde_json::from_str(&claude.argv[2]).unwrap();
        assert_eq!(settings["permissions"]["allow"], json!(rules));
        assert_eq!(claude.environment["GRAPHIFY_NO_AUTO_REFRESH"], "1");
        // Codex runs what its sandbox allows: the switch only.
        let codex = prepared(Some("chat"), "codex");
        assert_eq!(codex.argv[2], "{}");
        assert_eq!(codex.environment["GRAPHIFY_NO_AUTO_REFRESH"], "1");
        let dispatch = prepared(None, "claude");
        assert_eq!(dispatch.argv[2], "{}");
        assert!(dispatch.environment.is_empty());
    }
    #[test]
    fn a_system_run_launches_in_every_registered_service_folder() {
        let root = tempfile::tempdir().unwrap();
        let system = root.path().join("system");
        let payments = root.path().join("payments");
        let monorepo = root.path().join("core");
        let orders = monorepo.join("services/orders");
        let outside = root.path().join("outside");
        for folder in [&system, &payments, &orders, &outside] {
            std::fs::create_dir_all(folder).unwrap();
        }
        let store = SessionStore::open(&root.path().join("state")).unwrap();
        for folder in [&system, &payments, &monorepo] {
            store.register(folder).unwrap();
        }
        let providers = Arc::new(ProviderDomain {
            providers: Arc::new(RwLock::new(BTreeMap::new())),
        });
        let accelerators =
            Arc::new(Accelerators::new(store, root.path().into(), providers, Weak::new()).unwrap());
        let adapter = SystemLaunchAdapter { accelerators };
        // The session belongs to the system folder, yet every dispatch folder of
        // the run is launched: the system itself, a service that is a project of
        // its own, and a service folder inside a registered monorepo.
        for folder in [&system, &payments, &orders] {
            let adapted = adapter.adapt(&launch(folder), command());
            assert!(adapted.is_ok(), "{}: {:?}", folder.display(), adapted.err());
            assert_eq!(adapted.unwrap().argv, ["codex", "exec"]);
        }
        let Err(refused) = adapter.adapt(&launch(&outside), command()) else {
            panic!("a folder outside every registered project was launched");
        };
        assert_eq!(
            refused.message,
            "System dispatch folder is not inside a registered project."
        );
    }

    #[tokio::test]
    async fn register_read_only_auto_rules_enforces_registered_workspace_and_gate1_safety() {
        let root = tempfile::tempdir().unwrap();
        let proj = infra_core::fs::canonicalize(root.path())
            .unwrap()
            .join("proj");
        let outside = infra_core::fs::canonicalize(root.path())
            .unwrap()
            .join("outside");
        std::fs::create_dir_all(proj.join("programs")).unwrap();
        let ro_bytes = b"print('hi')\n";
        std::fs::write(proj.join("programs/ro.py"), ro_bytes).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        let store = SessionStore::open(&root.path().join("state")).unwrap();
        let project_id = store.register(&proj).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let providers = Arc::new(ProviderDomain {
            providers: Arc::new(RwLock::new(BTreeMap::new())),
        });
        let accelerators =
            Accelerators::new(store.clone(), root.path().into(), providers, Weak::new()).unwrap();
        let desk = Arc::new(crate::permissions::PermissionDesk::default());
        assert!(
            accelerators
                .register_read_only_auto_rules(&desk, "s1", &outside)
                .is_err()
        );
        accelerators
            .register_read_only_auto_rules(&desk, "s1", &proj)
            .unwrap();

        // Bare workspace `programs/ro.py` without an active sealed capsule MUST NOT be approved.
        assert_eq!(
            Accelerators::read_only_session_rule_key(&proj, "python3 programs/ro.py").unwrap(),
            None
        );
        assert_eq!(
            accelerators
                .read_only_session_rule_key_in_store(&proj, "python3 programs/ro.py")
                .unwrap(),
            None
        );
        // Safe read-only CLI works even without a capsule.
        assert_eq!(
            Accelerators::read_only_session_rule_key(&proj, "git status --short")
                .unwrap()
                .as_deref(),
            Some("readonly:program:git status --short")
        );

        // Seed an active sealed, gate-verified read_only capsule in SessionStore.
        let warm_root = store
            .state
            .join(format!("skillcache/capsules/{project_id}/cap-accel-ro/1"));
        let warm_dir = warm_root.join("warm");
        let capsule_json = json!({
            "schema_version": infra_skillcache::SCHEMA_VERSION,
            "form": "hybrid",
            "mu": {"name": "ro-accel", "description": "Read-only accelerator check"},
            "source": {"skills": [{"name": "ro-accel", "path": ".agents/skills/ro-accel/SKILL.md", "tree_sha256": "0".repeat(64)}]},
            "capabilities": {"network": "none", "runtimes": ["python3"]},
            "bindings": [],
            "programs": [{
                "id": "main",
                "entry": ["python3", "programs/ro.py"],
                "inputs": [],
                "outputs": [],
                "check": ["python3", "programs/ro.py", "--check"],
                "timeout_seconds": 60,
                "read_only": true
            }],
            "steps": [{"kind": "program", "program": "main"}],
            "verification": []
        });
        let cap = infra_skillcache::capsule::SkillCapsule::from_value(&capsule_json).unwrap();
        let mut files = infra_skillcache::package::Files::new();
        files.insert("programs/ro.py", ro_bytes.to_vec());
        let pkg_sha = infra_skillcache::package::write_warm(&warm_dir, &cap, None, &files).unwrap();
        let loaded = infra_skillcache::package::load(&warm_dir).unwrap();
        {
            let db = store.connection().unwrap();
            let skill_ref = json!({"name": "ro-accel", "path": ".agents/skills/ro-accel/SKILL.md", "tree_sha256": "0".repeat(64)});
            crate::skillcache::db::begin_capsule(
                &db,
                "cap-accel-ro",
                "cmp-accel-ro",
                &project_id,
                "s1",
                "gen-accel-ro",
                &skill_ref,
                None,
            )
            .unwrap();
            crate::skillcache::db::activate_capsule(
                &db,
                &crate::skillcache::db::Qualified {
                    capsule_id: "cap-accel-ro",
                    mu_name: "ro-accel",
                    mu_description: "Read-only accelerator check",
                    form: "hybrid",
                    capsule_sha256: &loaded.capsule_sha256,
                    package_sha256: &pkg_sha,
                    skill_files: &json!({}),
                    env_fingerprint: "env",
                    engine_digest: "eng",
                    artifact_dir: warm_root.to_str().unwrap(),
                    capabilities: &json!({"network": "none", "runtimes": ["python3"]}),
                },
            )
            .unwrap();
            for gate in ["gate1", "gate2", "gate3"] {
                crate::skillcache::db::record_gate(
                    &db,
                    "cap-accel-ro",
                    1,
                    gate,
                    true,
                    &json!({"gate": gate, "passed": true}),
                )
                .unwrap();
            }
        }
        assert_eq!(
            accelerators
                .read_only_session_rule_key_in_store(&proj, "python3 programs/ro.py")
                .unwrap()
                .as_deref(),
            Some("readonly:program:python3 programs/ro.py")
        );

        let approvals = desk.approvals("s1", Arc::new(|_| Ok(())));
        let rx = approvals
            .ask_answer(&infra_host::permission::PermissionAsk {
                id: "ro-1".into(),
                provider: "codex".into(),
                tool: "Shell".into(),
                summary: "python3 programs/ro.py".into(),
                kind: "command".into(),
                session_rule_key: Some("readonly:program:python3 programs/ro.py".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            rx.await.unwrap().decision,
            infra_host::permission::Decision::Allow
        );
        let _ = infra_skillcache::package::remove(&warm_dir);
    }

    #[tokio::test]
    async fn attach_proactively_admits_matching_portable_edition_templates() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path().join("laravel-app");
        let skill_dir = root.join(".claude/skills/check-app");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "# Check App\n").unwrap();
        std::fs::write(root.join("artisan"), "#!/usr/bin/env php\n").unwrap();
        std::fs::write(
            root.join("composer.json"),
            r#"{"require":{"laravel/framework":"^11.0"}}"#,
        )
        .unwrap();
        let tree = infra_skillcache::fingerprint::tree_sha256(&skill_dir).unwrap();

        let source = scratch.path().join("acc-source");
        std::fs::create_dir_all(source.join("Laravel/.claude/skills")).unwrap();
        std::fs::create_dir_all(source.join("Laravel/.cursor/skills")).unwrap();
        std::fs::create_dir_all(source.join("Laravel/.agents/skills")).unwrap();
        std::fs::create_dir_all(source.join("Laravel/memory-bank")).unwrap();
        std::fs::write(source.join("Laravel/AGENTS.md"), "# Laravel\n").unwrap();

        struct NoSpecial;
        impl crate::sessions::BlackboardFactory for NoSpecial {
            fn for_session(&self, _session: &Value) -> Result<Arc<dyn crate::fleet::Blackboard>> {
                Err(HarnessError::unavailable("unused"))
            }
        }
        impl crate::sessions::SpecialWorkflow for NoSpecial {
            fn run<'a>(
                &'a self,
                _session: Value,
                _prompt: String,
                _generation: String,
                _cancel: Arc<std::sync::atomic::AtomicBool>,
                _emit: crate::sessions::Event,
            ) -> crate::sessions::RunReply<'a> {
                Box::pin(async { Ok(json!({})) })
            }
        }
        let store = SessionStore::open(&scratch.path().join("state")).unwrap();
        let providers = Arc::new(crate::provider::ProviderDomain {
            providers: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
        });
        let knowledge = Arc::new(crate::knowledge::KnowledgeDomain::new(
            store.clone(),
            source.clone(),
        ));
        let special = ["creator", "system_run", "system_discovery"]
            .into_iter()
            .map(|k| {
                (
                    k.to_owned(),
                    Arc::new(NoSpecial) as Arc<dyn crate::sessions::SpecialWorkflow>,
                )
            })
            .collect();
        let sessions = crate::sessions::Sessions::new(
            store.clone(),
            source.clone(),
            providers.clone(),
            knowledge,
            Arc::new(NoSpecial),
            special,
            crate::sessions::SessionLimits {
                seconds: 60,
                concurrency: 1,
            },
        )
        .unwrap();
        let accelerators =
            Accelerators::new(store.clone(), source, providers, Arc::downgrade(&sessions)).unwrap();
        let project = store.register(&root).unwrap();
        let project_id = project["id"].as_str().unwrap().to_owned();

        // Index a portable Laravel textual template for `check-app`.
        let skill = infra_skillcache::candidate::KnownSkill {
            name: "check-app".into(),
            path: ".claude/skills/check-app/SKILL.md".into(),
            tree_sha256: tree.sha256.clone(),
        };
        let answer = json!({
            "form": "textual",
            "capsule": {
                "mu": {"name": "check-app", "description": "Inspect Laravel routes"},
                "source": {"skills": [{"name": "check-app", "path": ".claude/skills/check-app/SKILL.md"}]}
            },
            "markdown": "# Check App\n\n1. Inspect `routes/web.php`.\n"
        });
        let cand = infra_skillcache::candidate::from_answer(
            &answer,
            std::slice::from_ref(&skill),
            infra_skillcache::Form::Textual,
        )
        .unwrap();
        let tpl = infra_skillcache::candidate::as_portable_template(&cand, "Laravel");
        crate::skillcache::db::upsert_template(
            &store.connection().unwrap(),
            "Laravel",
            "check-app",
            &tree.sha256,
            &tpl,
        )
        .unwrap();

        accelerators.attach(&project_id, Some("Laravel")).unwrap();
        let active = crate::skillcache::db::active_capsules_for_skill_or_mirror(
            &store.connection().unwrap(),
            &project_id,
            ".claude/skills/check-app/SKILL.md",
            "check-app",
            &tree.sha256,
            None,
        )
        .unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0]["kind"], "template");
        assert_eq!(active[0]["status"], "active");
    }

    #[test]
    fn source_version_uses_git_head_index_racy_ns_stamps_without_30s_ttl() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open(&dir.path().join("state")).unwrap();
        let source = dir.path().join("accel-src");
        std::fs::create_dir_all(source.join("Laravel")).unwrap();
        std::fs::create_dir_all(source.join("install/inventories")).unwrap();
        std::fs::write(source.join("Laravel/README.md"), "# Laravel v1\n").unwrap();
        std::fs::write(source.join("install/inventories/Laravel.json"), "{}").unwrap();

        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&source)
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.com")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.com")
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init"]);
        git(&["add", "."]);
        git(&["commit", "-m", "v1"]);

        let providers = Arc::new(ProviderDomain {
            providers: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
        });
        let accel = Accelerators::new(store, source.clone(), providers, Weak::new()).unwrap();

        let settled = accel_time_ns(SystemTime::now()).unwrap_or(0) + ACCEL_RACY_NS + 1_000_000;
        let v1 = accel.source_version_at(settled).unwrap();
        assert!(!v1.contains('+'));
        assert_eq!(
            accel
                .source_version_recomputes
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );

        // Repeat well past any 30s wall-clock horizon: still cached with 0 git subprocesses.
        let v1_repeat = accel
            .source_version_at(settled + 600_i128 * 1_000_000_000)
            .unwrap();
        assert_eq!(v1_repeat, v1);
        assert_eq!(
            accel
                .source_version_recomputes
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );

        // Unstaged edit in Laravel/ invalidates immediately.
        std::fs::write(source.join("Laravel/README.md"), "# Laravel dirty\n").unwrap();
        let now_ns = accel_time_ns(SystemTime::now()).unwrap_or(0);
        let v_dirty = accel.source_version_at(now_ns).unwrap();
        assert!(v_dirty.starts_with(&v1));
        assert!(v_dirty.contains('+'));
        assert_eq!(
            accel
                .source_version_recomputes
                .load(std::sync::atomic::Ordering::SeqCst),
            2
        );

        // Staging and committing updates HEAD/index stamps -> invalidates and settles again.
        git(&["add", "."]);
        git(&["commit", "-m", "v2"]);
        let settled2 = accel_time_ns(SystemTime::now()).unwrap_or(0) + ACCEL_RACY_NS + 1_000_000;
        let v2 = accel.source_version_at(settled2).unwrap();
        assert_ne!(v2, v1);
        assert!(!v2.contains('+'));
        assert_eq!(
            accel
                .source_version_recomputes
                .load(std::sync::atomic::Ordering::SeqCst),
            3
        );
        let v2_repeat = accel.source_version_at(settled2 + 1_000).unwrap();
        assert_eq!(v2_repeat, v2);
        assert_eq!(
            accel
                .source_version_recomputes
                .load(std::sync::atomic::Ordering::SeqCst),
            3
        );
    }
}
