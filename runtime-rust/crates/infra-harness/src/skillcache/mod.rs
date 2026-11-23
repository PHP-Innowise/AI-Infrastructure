//! SkillCache in the Harness.
//!
//! A session opts in with `skillcache: {"mode": ...}`; the default is `off`
//! and then nothing here runs. `observe` and `auto` keep the observed trace of
//! a cold turn, which stores tool results the Harness otherwise never stores:
//! that is why it is an explicit choice per session. The capsule's own
//! contracts live in the `infra-skillcache` crate.
pub mod compile;
pub(crate) mod db;
pub mod gates;
pub(crate) mod replay_result;
pub mod settings;
pub mod snapshot;
pub mod trace;
pub mod warm;

use crate::{
    HarnessError, Result,
    provider::ProviderDomain,
    services::{Domain, Payload, Reply},
    store::SessionStore,
};
use compile::{EnqueueCompileFn, Job, Runtime};
use infra_skillcache::{compile_rules, fingerprint, package};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Mutex as AsyncMutex, Semaphore};

/// Compilations and requalifications waiting or active across projects.
/// Different projects run concurrently under separate project locks.
const WAITING: usize = 8;

enum Work {
    Compile(Job),
    Requalify {
        project_id: String,
        capsule_id: String,
        prior_status: String,
    },
}

struct WorkQueue {
    self_ref: std::sync::Weak<WorkQueue>,
    slots: Arc<Semaphore>,
    project_locks: Mutex<BTreeMap<String, Arc<AsyncMutex<()>>>>,
    requalifying: Arc<Mutex<BTreeSet<String>>>,
}

impl WorkQueue {
    fn new() -> Arc<Self> {
        Arc::new_cyclic(|weak| Self {
            self_ref: weak.clone(),
            slots: Arc::new(Semaphore::new(WAITING)),
            project_locks: Mutex::new(BTreeMap::new()),
            requalifying: Arc::new(Mutex::new(BTreeSet::new())),
        })
    }

    fn project_lock(&self, project_id: &str) -> Arc<AsyncMutex<()>> {
        let mut map = self
            .project_locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        map.entry(project_id.to_owned())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    fn queue(&self, runtime: &Arc<Runtime>, work: Work) -> Result<()> {
        if runtime.cancel.load(Ordering::Acquire) {
            return Err(HarnessError::unavailable("SkillCache is stopping."));
        }
        let permit = self.slots.clone().try_acquire_owned().map_err(|_| {
            HarnessError::conflict(
                "The SkillCache queue is full; try again when a compilation has finished.",
            )
        })?;
        let project_id = match &work {
            Work::Compile(job) => job.project_id.clone(),
            Work::Requalify { project_id, .. } => project_id.clone(),
        };
        let lock = self.project_lock(&project_id);
        let runtime = runtime.clone();
        let requalifying = self.requalifying.clone();
        let queue_weak = self.self_ref.clone();
        tokio::spawn(async move {
            let project_guard = lock.lock().await;
            if runtime.cancel.load(Ordering::Acquire) {
                if let Work::Requalify { capsule_id, .. } = &work
                    && let Ok(mut pending) = requalifying.lock()
                {
                    pending.remove(capsule_id);
                }
                return;
            }
            match work {
                Work::Compile(job) => compile::run(&runtime, &job).await,
                Work::Requalify {
                    capsule_id,
                    prior_status,
                    ..
                } => {
                    let capsule = runtime
                        .store
                        .connection()
                        .and_then(|db| db::capsule(&db, &capsule_id));
                    match capsule {
                        Ok(Some(capsule)) => {
                            let was_needs_recompile = prior_status == "needs_recompile"
                                || capsule["status"] == "needs_recompile";
                            match project_path(&runtime.store, &capsule) {
                                Ok(workspace) => {
                                    match compile::requalify(&runtime, &capsule, &workspace).await {
                                        Ok(report) => {
                                            if let Ok(db) = runtime.store.connection() {
                                                let _ =
                                                    db::delete_pending_requalify(&db, &capsule_id);
                                            }
                                            if was_needs_recompile && report["passed"] != true {
                                                let _ = enqueue_recompile_for_capsule(
                                                    &runtime, &capsule,
                                                );
                                            }
                                        }
                                        Err(error) => {
                                            let error_text = error.to_string();
                                            let transient = error.code == "unavailable";
                                            if let Ok(db) = runtime.store.connection() {
                                                let drop_pending =
                                                    if runtime.cancel.load(Ordering::Acquire) {
                                                        false
                                                    } else if transient {
                                                        db::record_pending_requalify_attempt(
                                                            &db,
                                                            &capsule_id,
                                                            &error_text,
                                                        )
                                                        .is_ok_and(|attempts| attempts >= 3)
                                                    } else {
                                                        true
                                                    };
                                                if drop_pending {
                                                    let _ = db::delete_pending_requalify(
                                                        &db,
                                                        &capsule_id,
                                                    );
                                                }
                                                let target = if capsule["status"] == "retired" {
                                                    "retired"
                                                } else if was_needs_recompile {
                                                    "needs_recompile"
                                                } else {
                                                    "stale"
                                                };
                                                let _ = db::set_capsule_status(
                                                    &db,
                                                    &capsule_id,
                                                    target,
                                                    Some(&error_text),
                                                );
                                            }
                                            runtime.publish(
                                                capsule["project_id"].as_str().unwrap_or(""),
                                                "failed",
                                                json!({"capsule_id": capsule_id, "outcome": error_text}),
                                            );
                                        }
                                    }
                                }
                                Err(error) => {
                                    if let Ok(db) = runtime.store.connection() {
                                        let _ = db::delete_pending_requalify(&db, &capsule_id);
                                        let target = if capsule["status"] == "retired" {
                                            "retired"
                                        } else if was_needs_recompile {
                                            "needs_recompile"
                                        } else {
                                            "stale"
                                        };
                                        let _ = db::set_capsule_status(
                                            &db,
                                            &capsule_id,
                                            target,
                                            Some(&error.to_string()),
                                        );
                                    }
                                }
                            }
                        }
                        Ok(None) => {
                            if let Ok(db) = runtime.store.connection() {
                                let _ = db::delete_pending_requalify(&db, &capsule_id);
                            }
                        }
                        Err(_) => {}
                    }
                    if let Ok(mut pending) = requalifying.lock() {
                        pending.remove(&capsule_id);
                    }
                }
            }
            drop(project_guard);
            drop(permit);
            if let Some(wq) = queue_weak.upgrade() {
                drain_pending_requalifies(&runtime, &wq);
            }
        });
        Ok(())
    }
}

/// The HTTP operations of SkillCache and the queue behind two of them.
pub struct SkillCacheDomain {
    runtime: Arc<Runtime>,
    work: Arc<WorkQueue>,
    requalifying: Arc<Mutex<BTreeSet<String>>>,
}

/// An installed runtime's version, remembered for a minute: the identity of
/// the environment is asked for on every listing and every warm decision.
fn runtime_versions(names: &[String]) -> BTreeMap<String, Option<String>> {
    type RuntimeVersionCache = BTreeMap<(String, String), (Instant, Option<String>)>;
    static KNOWN: Mutex<RuntimeVersionCache> = Mutex::new(BTreeMap::new());
    let path_key = std::env::var("PATH").unwrap_or_default();
    let mut known = KNOWN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    names
        .iter()
        .map(|name| {
            let key = (name.clone(), path_key.clone());
            let fresh = known
                .get(&key)
                .filter(|(at, _)| at.elapsed() < Duration::from_secs(60))
                .map(|(_, version)| version.clone());
            let version = fresh.unwrap_or_else(|| {
                let version = fingerprint::runtime_version(name);
                known.insert(key, (Instant::now(), version.clone()));
                version
            });
            (name.clone(), version)
        })
        .collect()
}

/// Why an active capsule may no longer be used in `workspace`, if it may not:
/// its skill, its environment, its rules or its package is not what it was
/// qualified with.
pub fn staleness(capsule: &Value, workspace: &Path) -> Option<String> {
    if capsule["schema_version"].as_u64() != Some(infra_skillcache::SCHEMA_VERSION) {
        return Some("The capsule was written in another schema version.".into());
    }
    if capsule["compile_rules_version"].as_str() != Some(compile_rules::version().as_str()) {
        return Some("The compiler's rules changed since this capsule was compiled.".into());
    }
    if capsule["kind"] == "tool_chain" {
        let manifest = db::ArtifactDependencyManifest::from_value(&capsule["dependency_manifest"]);
        if !manifest.path_hashes.is_empty() {
            if !trace::verify_toolchain_manifest(workspace, &manifest.path_hashes) {
                return Some(
                    "Watched workspace files changed since this tool-chain capsule was compiled."
                        .into(),
                );
            }
        } else if let Some(expected_tree) = capsule["tool_chain_tree_sha"]
            .as_str()
            .filter(|sha| !sha.is_empty())
        {
            match trace::workspace_tree_sha(workspace) {
                Ok(now_tree) if now_tree == expected_tree => {}
                Ok(_) => {
                    return Some(
                        "The workspace tree changed since this tool-chain capsule was compiled."
                            .into(),
                    );
                }
                Err(_) => {
                    return Some("The workspace tree could not be fingerprinted.".into());
                }
            }
        }
    }
    for skill in capsule["skill_files"]["skills"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let name = skill["name"].as_str().unwrap_or("");
        let Some(folder) = skill["path"]
            .as_str()
            .and_then(|path| trace::skill_file(workspace, path))
            .and_then(|file| file.parent().map(Path::to_path_buf))
        else {
            return Some(format!("The skill `{name}` cannot be located."));
        };
        let known = fingerprint::Tree {
            sha256: skill["tree_sha256"].as_str().unwrap_or("").into(),
            taken_ns: skill["taken_ns"]
                .as_str()
                .and_then(|text| text.parse().ok())
                .unwrap_or_default(),
            files: skill["files"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|file| {
                    Some(fingerprint::FileStat {
                        path: file["path"].as_str()?.into(),
                        size: file["size"].as_u64()?,
                        mtime_ns: file["mtime_ns"].as_str()?.parse().ok()?,
                    })
                })
                .collect(),
        };
        match fingerprint::verify_tree(&folder, &known) {
            Ok((true, _)) => {}
            Ok((false, _)) => {
                return Some(format!(
                    "The skill `{name}` changed since this capsule was compiled."
                ));
            }
            Err(_) => return Some(format!("The skill `{name}` is not readable.")),
        }
    }
    let runtimes = capsule["capabilities"]["runtimes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|name| name.as_str().map(String::from))
        .collect::<Vec<_>>();
    let environment = fingerprint::env_fingerprint(&runtime_versions(&runtimes));
    if capsule["env_fingerprint"].as_str() != Some(environment.as_str()) {
        return Some(
            "A runtime or SkillCache environment version the capsule depends on changed; requalify it.".into(),
        );
    }
    let warm = PathBuf::from(capsule["artifact_dir"].as_str().unwrap_or("")).join("warm");
    match cached_package_digest(&warm) {
        Ok(digest) if capsule["package_sha256"].as_str() == Some(digest.as_str()) => None,
        _ => Some("The capsule's package is missing or was changed.".into()),
    }
}

#[derive(Clone)]
struct CachedPackageMeta {
    stats: Vec<(String, u64, u128)>,
    digest: String,
    read_only: bool,
}

#[derive(Default)]
struct PackageMetaCache {
    entries: HashMap<PathBuf, CachedPackageMeta>,
    order: VecDeque<PathBuf>,
}

const MAX_PACKAGE_META_CACHE: usize = 256;

fn package_meta_cache() -> &'static Mutex<PackageMetaCache> {
    static CACHE: OnceLock<Mutex<PackageMetaCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(PackageMetaCache::default()))
}

fn stat_package_tree(
    root: &Path,
    dir: &Path,
    depth: usize,
    now_ns: u128,
    stats: &mut Vec<(String, u64, u128)>,
    racy_safe: &mut bool,
) -> bool {
    if depth > 8 {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            return false;
        };
        if infra_core::fs::linked(&meta) {
            return false;
        }
        if meta.is_dir() {
            if !stat_package_tree(root, &path, depth + 1, now_ns, stats, racy_safe) {
                return false;
            }
        } else if meta.is_file() {
            let Ok(rel) = path.strip_prefix(root) else {
                return false;
            };
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            let mtime_ns = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            if mtime_ns == 0 || now_ns.saturating_sub(mtime_ns) < db::RACY_NS {
                *racy_safe = false;
            }
            stats.push((rel_str, meta.len(), mtime_ns));
        } else {
            return false;
        }
    }
    true
}

fn cached_package_meta(
    warm: &Path,
) -> std::result::Result<(String, bool), infra_core::RuntimeError> {
    let canonical_warm = std::fs::canonicalize(warm).unwrap_or_else(|_| warm.to_path_buf());
    let now_ns = db::current_unix_ns();
    let mut stats = Vec::new();
    let mut racy_safe = true;
    let stat_ok = std::fs::symlink_metadata(warm)
        .is_ok_and(|m| !infra_core::fs::linked(&m) && m.is_dir())
        && stat_package_tree(warm, warm, 0, now_ns, &mut stats, &mut racy_safe);
    if stat_ok {
        stats.sort();
        if racy_safe
            && let Ok(guard) = package_meta_cache().lock()
            && let Some(cached) = guard.entries.get(&canonical_warm)
            && cached.stats == stats
        {
            return Ok((cached.digest.clone(), cached.read_only));
        }
    }
    let loaded = package::load(warm)?;
    let read_only = loaded.capsule.form == infra_skillcache::Form::Textual
        || loaded.capsule.programs.iter().all(|p| p.read_only);
    let digest = loaded.package_sha256;
    if stat_ok
        && racy_safe
        && let Ok(mut guard) = package_meta_cache().lock()
    {
        if !guard.entries.contains_key(&canonical_warm) {
            if guard.entries.len() >= MAX_PACKAGE_META_CACHE
                && let Some(evicted) = guard.order.pop_front()
            {
                guard.entries.remove(&evicted);
            }
            guard.order.push_back(canonical_warm.clone());
        }
        guard.entries.insert(
            canonical_warm,
            CachedPackageMeta {
                stats,
                digest: digest.clone(),
                read_only,
            },
        );
    }
    Ok((digest, read_only))
}

pub(super) fn cached_package_digest(
    warm: &Path,
) -> std::result::Result<String, infra_core::RuntimeError> {
    cached_package_meta(warm).map(|(digest, _)| digest)
}

pub(super) fn cached_package_read_only(warm: &Path) -> Option<bool> {
    cached_package_meta(warm).ok().map(|(_, ro)| ro)
}

fn ir_revisions() -> &'static Mutex<HashMap<String, u64>> {
    static REVS: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    REVS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn record_ir_revision(project_id: &str, ir_revision: u64) {
    if ir_revision == 0 || project_id.is_empty() {
        return;
    }
    if let Ok(mut guard) = ir_revisions().lock() {
        let entry = guard.entry(project_id.to_owned()).or_insert(0);
        if ir_revision > *entry {
            *entry = ir_revision;
        }
    }
}

pub(super) fn latest_ir_revision(state: &Path, project_id: &str) -> u64 {
    if let Ok(guard) = ir_revisions().lock()
        && let Some(&rev) = guard.get(project_id)
        && rev > 0
    {
        return rev;
    }
    crate::graph_ir::GraphIrStore::open_readonly_cached(state, project_id)
        .ok()
        .flatten()
        .map(|ir| ir.current_write_epoch())
        .unwrap_or(0)
}

#[derive(Clone)]
struct RequalifyHook {
    runtime: std::sync::Weak<Runtime>,
    work: std::sync::Weak<WorkQueue>,
}

fn requalify_hooks() -> &'static Mutex<BTreeMap<PathBuf, RequalifyHook>> {
    static HOOKS: std::sync::OnceLock<Mutex<BTreeMap<PathBuf, RequalifyHook>>> =
        std::sync::OnceLock::new();
    HOOKS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn register_requalify_hook(runtime: &Arc<Runtime>, work: &Arc<WorkQueue>) {
    let key =
        std::fs::canonicalize(&runtime.store.state).unwrap_or_else(|_| runtime.store.state.clone());
    if let Ok(mut guard) = requalify_hooks().lock() {
        guard.insert(
            key,
            RequalifyHook {
                runtime: Arc::downgrade(runtime),
                work: Arc::downgrade(work),
            },
        );
    }
}

fn drain_pending_requalifies(runtime: &Arc<Runtime>, wq: &Arc<WorkQueue>) {
    if runtime.cancel.load(Ordering::Acquire) || tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    let Ok(db) = runtime.store.connection() else {
        return;
    };
    let Ok(rows) = db::list_pending_requalify(&db, WAITING as i64) else {
        return;
    };
    let Ok(pending_compiles) = db::pending_compiles(&db) else {
        return;
    };
    for row in rows {
        if let Ok(Some(cap)) = db::capsule(&db, &row.capsule_id) {
            if matches!(cap["status"].as_str(), Some("retired" | "compiling")) {
                let _ = db::delete_pending_requalify(&db, &row.capsule_id);
                continue;
            }
        } else {
            let _ = db::delete_pending_requalify(&db, &row.capsule_id);
            continue;
        }
        {
            let Ok(mut pending) = wq.requalifying.lock() else {
                return;
            };
            if pending.contains(&row.capsule_id) {
                continue;
            }
            if pending_compiles + pending.len() as i64 >= WAITING as i64 {
                break;
            }
            pending.insert(row.capsule_id.clone());
        }
        if wq
            .queue(
                runtime,
                Work::Requalify {
                    project_id: row.project_id,
                    capsule_id: row.capsule_id.clone(),
                    prior_status: row.prior_status,
                },
            )
            .is_err()
        {
            if let Ok(mut pending) = wq.requalifying.lock() {
                pending.remove(&row.capsule_id);
            }
            break;
        }
    }
}

fn trigger_background_requalify(db: &rusqlite::Connection) {
    if tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    let Some(db_path) = db.path().filter(|p| !p.is_empty()) else {
        return;
    };
    let Some(state_dir) = Path::new(db_path).parent() else {
        return;
    };
    let key = std::fs::canonicalize(state_dir).unwrap_or_else(|_| state_dir.to_path_buf());
    let hook = requalify_hooks()
        .lock()
        .ok()
        .and_then(|guard| guard.get(&key).cloned());
    let Some(hook) = hook else {
        return;
    };
    let (Some(rt), Some(wq)) = (hook.runtime.upgrade(), hook.work.upgrade()) else {
        return;
    };
    drain_pending_requalifies(&rt, &wq);
}

impl SkillCacheDomain {
    pub fn new(
        store: SessionStore,
        providers: Arc<ProviderDomain>,
        cancel: Arc<AtomicBool>,
    ) -> Self {
        let work = WorkQueue::new();
        let requalifying = work.requalifying.clone();
        let queue_for_runtime = work.clone();
        let enqueuer: EnqueueCompileFn = Arc::new(move |rt: &Arc<Runtime>, job: Job| {
            queue_for_runtime.queue(rt, Work::Compile(job))
        });
        let domain = Self {
            runtime: Arc::new(Runtime {
                store,
                providers,
                engine: None,
                cancel,
                enqueuer: Some(enqueuer),
                requalifying: Some(requalifying.clone()),
            }),
            work,
            requalifying,
        };
        register_requalify_hook(&domain.runtime, &domain.work);
        drain_pending_requalifies(&domain.runtime, &domain.work);
        domain
    }
    /// Names the engine the gates run, for a process that is not one itself.
    pub fn with_engine(mut self, engine: PathBuf) -> Self {
        let runtime = &self.runtime;
        self.runtime = Arc::new(Runtime {
            store: runtime.store.clone(),
            providers: runtime.providers.clone(),
            engine: Some(engine),
            cancel: runtime.cancel.clone(),
            enqueuer: runtime.enqueuer.clone(),
            requalifying: runtime.requalifying.clone(),
        });
        register_requalify_hook(&self.runtime, &self.work);
        drain_pending_requalifies(&self.runtime, &self.work);
        self
    }
    pub fn runtime(&self) -> Arc<Runtime> {
        self.runtime.clone()
    }
    fn queue(&self, work: Work) -> Result<()> {
        self.work.queue(&self.runtime, work)
    }
    /// A capsule row as it is now: an active capsule that no longer matches
    /// what it was qualified with is marked stale before it is shown.
    fn current(&self, capsule: Value) -> Result<Value> {
        if capsule["status"] != "active" {
            return Ok(capsule);
        }
        let Ok(workspace) = project_path(&self.runtime.store, &capsule) else {
            return Ok(capsule);
        };
        let Some(reason) = staleness(&capsule, &workspace) else {
            return Ok(capsule);
        };
        let id = capsule["capsule_id"].as_str().unwrap_or("").to_string();
        let db = self.runtime.store.connection()?;
        db::set_capsule_status(&db, &id, "stale", Some(&reason))?;
        let updated = db::capsule(&db, &id)?.unwrap_or(capsule);
        drop(db);
        self.runtime.publish(
            updated["project_id"].as_str().unwrap_or(""),
            "stale",
            json!({"capsule_id": id, "reason": reason}),
        );
        Ok(updated)
    }
    fn owned(&self, project_id: &str, capsule_id: &str) -> Result<Value> {
        db::capsule(&*self.runtime.store.connection()?, capsule_id)?
            .filter(|capsule| capsule["project_id"] == project_id)
            .ok_or_else(|| HarnessError::missing("Capsule not found."))
    }
    fn list(&self, project_id: &str, query: &Value) -> Result<Value> {
        self.runtime.store.project(project_id)?;
        let filter = |name: &str| query[name].as_str().filter(|value| !value.is_empty());
        let status = filter("status");
        let asked =
            |name: &str| query[name] == true || matches!(query[name].as_str(), Some("true" | "1"));
        let include_retired = status == Some("retired") || asked("include_retired");
        let rows = db::capsules(
            &*self.runtime.store.connection()?,
            project_id,
            filter("skill"),
            None,
            filter("form"),
            include_retired,
            // An evaluation arm's capsules are not what a person manages;
            // they are listed on request, so that they can be found and
            // deleted.
            asked("include_eval"),
        )?;
        let mut capsules = Vec::new();
        for row in rows {
            let row = self.current(row)?;
            if status.is_none_or(|status| row["status"] == status) {
                capsules.push(summary(&row));
            }
        }
        Ok(json!({"project_id": project_id, "capsules": capsules}))
    }
    fn get(&self, project_id: &str, capsule_id: &str) -> Result<Value> {
        let capsule = self.current(self.owned(project_id, capsule_id)?)?;
        let warm = PathBuf::from(capsule["artifact_dir"].as_str().unwrap_or("")).join("warm");
        let stored = package::load(&warm).ok();
        let db = self.runtime.store.connection()?;
        let compilation = match capsule["compile_id"].as_str() {
            Some(id) => db::compile(&db, id)?,
            None => None,
        };
        let (total_runs, _, _, _): (i64, i64, i64, i64) = db.query_row(
            "SELECT COUNT(*),COALESCE(SUM(outcome='complete'),0),COALESCE(SUM(deopts>0),0),
                    COALESCE(SUM(deopts),0)
             FROM skillcache_runs WHERE capsule_id=?1",
            [capsule_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        let (runs, complete, handed_back, deopts): (i64, i64, i64, i64) = db.query_row(
            "SELECT COUNT(*),COALESCE(SUM(outcome='complete'),0),COALESCE(SUM(deopts>0),0),
                    COALESCE(SUM(deopts),0)
             FROM skillcache_runs WHERE capsule_id=?1
               AND rowid>COALESCE((SELECT runs_reset_rowid FROM skillcache_capsules WHERE capsule_id=?1),0)",
            [capsule_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        let recent = db::capsule_runs(&db, capsule_id, 20)?;
        let mut detail = summary(&capsule);
        // The plan as a person inspects it. Program sources stay in the package.
        detail["capsule"] = stored
            .as_ref()
            .map(|stored| stored.capsule.to_value())
            .unwrap_or(Value::Null);
        detail["markdown"] = stored
            .as_ref()
            .and_then(|stored| stored.markdown.clone())
            .map_or(Value::Null, Value::from);
        detail["files"] = stored
            .as_ref()
            .map(|stored| {
                stored
                    .files
                    .iter()
                    .map(|(path, content)| json!({"path": path, "bytes": content.len()}))
                    .collect::<Vec<_>>()
            })
            .into();
        detail["gates"] = db::gates(&db, capsule_id)?.into();
        // What the Harness knows of the capsule's cost: what compiling it cost
        // and how its warm turns went. What a reuse saves is not known here;
        // a sealed evaluation measures it, and until then it is not a zero.
        detail["amortization"] = json!({
            "compile_usd": compilation.as_ref().and_then(|compile| compile["usage"]["cost_usd"].as_f64()),
            "warm_turns": runs,
            "warm_turns_total": total_runs,
            "saving_per_reuse_usd": null,
            "breakeven_reuses": null,
        });
        detail["compile"] = compilation.unwrap_or(Value::Null);
        detail["runs"] = json!({
            "count": runs,
            "total": total_runs,
            "complete": complete,
            "handed_back": handed_back,
            "deopts": deopts,
            "recent": recent,
        });
        detail["skill_files"] = json!({
            "skills": capsule["skill_files"]["skills"].as_array().into_iter().flatten().map(|skill| json!({
                "name": skill["name"], "path": skill["path"], "tree_sha256": skill["tree_sha256"],
                "files": skill["files"].as_array().map_or(0, Vec::len),
            })).collect::<Vec<_>>()
        });
        Ok(detail)
    }
    fn invalidate(&self, project_id: &str, capsule_id: &str) -> Result<Value> {
        let capsule = self.owned(project_id, capsule_id)?;
        if !["active", "needs_recompile"].contains(&capsule["status"].as_str().unwrap_or("")) {
            return Err(HarnessError::conflict(
                "Only an active capsule can be invalidated.",
            ));
        }
        db::set_capsule_status(
            &*self.runtime.store.connection()?,
            capsule_id,
            "stale",
            Some("Invalidated by a person; requalify it to use it again."),
        )?;
        self.runtime
            .publish(project_id, "invalidated", json!({"capsule_id": capsule_id}));
        self.get(project_id, capsule_id)
    }
    fn retire(&self, project_id: &str, capsule_id: &str, body: &Value) -> Result<Value> {
        if !body.is_null() && !crate::store::exact(body, &[], &["reason"]) {
            return Err(HarnessError::invalid(
                "Retire takes only an optional `reason`.",
            ));
        }
        let custom_reason = match &body["reason"] {
            Value::Null => None,
            Value::String(text) => {
                let trimmed = text.trim();
                if trimmed.len() > 500 || text.contains('\0') {
                    return Err(HarnessError::invalid("Invalid retire reason."));
                }
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            }
            _ => return Err(HarnessError::invalid("Invalid retire reason.")),
        };
        let capsule = self.owned(project_id, capsule_id)?;
        if !["active", "stale", "needs_recompile"]
            .contains(&capsule["status"].as_str().unwrap_or(""))
        {
            return Err(HarnessError::conflict(
                "Only an active, stale, or needs_recompile capsule can be retired.",
            ));
        }
        let reason = custom_reason
            .as_deref()
            .unwrap_or("Retired by a person; requalify it to use it again.");
        db::set_capsule_status(
            &*self.runtime.store.connection()?,
            capsule_id,
            "retired",
            Some(reason),
        )?;
        self.runtime.publish(
            project_id,
            "retired",
            json!({"capsule_id": capsule_id, "reason": reason}),
        );
        self.get(project_id, capsule_id)
    }
    fn requalify(&self, project_id: &str, capsule_id: &str) -> Result<Value> {
        let capsule = self.owned(project_id, capsule_id)?;
        if !["active", "stale", "needs_recompile", "retired"]
            .contains(&capsule["status"].as_str().unwrap_or(""))
        {
            return Err(HarnessError::conflict(
                "Only a compiled capsule can be requalified.",
            ));
        }
        let prior_status = capsule["status"].as_str().unwrap_or("active").to_owned();
        {
            let db = self.runtime.store.connection()?;
            let pending_compiles = db::pending_compiles(&db)?;
            let mut pending = self.requalifying.lock().map_err(|_| {
                HarnessError::unavailable("SkillCache requalify state lock failed.")
            })?;
            if pending.contains(capsule_id) {
                return Err(HarnessError::conflict(
                    "This capsule is already being requalified.",
                ));
            }
            if pending_compiles + pending.len() as i64 >= WAITING as i64 {
                return Err(HarnessError::conflict(
                    "The SkillCache queue is full; try again when a compilation has finished.",
                ));
            }
            pending.insert(capsule_id.into());
        }
        if let Err(error) = self.queue(Work::Requalify {
            project_id: project_id.into(),
            capsule_id: capsule_id.into(),
            prior_status,
        }) {
            if let Ok(mut pending) = self.requalifying.lock() {
                pending.remove(capsule_id);
            }
            return Err(error);
        }
        self.runtime.publish(
            project_id,
            "queued",
            json!({"capsule_id": capsule_id, "requalify": true}),
        );
        Ok(json!({"capsule_id": capsule_id, "status": "queued"}))
    }
    fn delete(&self, project_id: &str, capsule_id: &str) -> Result<Value> {
        let capsule = self.owned(project_id, capsule_id)?;
        if capsule["status"] == "compiling" {
            return Err(HarnessError::conflict(
                "Wait for the compilation to finish.",
            ));
        }
        let store = &self.runtime.store;
        let artifacts = compile::artifact_dir(store, project_id, capsule_id, 1);
        let _ = package::remove(&artifacts.join("warm"));
        if let Some(capsule_root) = artifacts.parent() {
            package::release(capsule_root);
            let _ = std::fs::remove_dir_all(capsule_root);
        }
        if let Some(qualification) = compile::qualification_dir(store, capsule_id, 1).parent() {
            let _ = std::fs::remove_dir_all(qualification);
        }
        let generation = capsule["source_generation"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let db = store.connection()?;
        db::delete_capsule(&db, capsule_id)?;
        // The trace goes with the last capsule that was compiled from it.
        let shared: i64 = db.query_row(
            "SELECT COUNT(*) FROM skillcache_capsules WHERE source_generation=?1",
            [&generation],
            |row| row.get(0),
        )?;
        if shared == 0 {
            db::delete_trace(&db, &generation)?;
            drop(db);
            let _ = snapshot::remove(&store.state, &generation);
        }
        self.runtime
            .publish(project_id, "deleted", json!({"capsule_id": capsule_id}));
        Ok(json!({"capsule_id": capsule_id, "deleted": true}))
    }
    fn compile(&self, session_id: &str, body: &Value) -> Result<Value> {
        if !crate::store::exact(body, &["generation"], &["compiler", "eval_arm"]) {
            return Err(HarnessError::invalid(
                "Name the turn to compile: {\"generation\": ..., \"compiler\": {\"provider\": \"claude\", \"model\": ...}}.",
            ));
        }
        let store = &self.runtime.store;
        let session = store.get(session_id)?;
        let generation = crate::store::text(&body["generation"], "generation", 128)?;
        let eval_arm = if body["eval_arm"].is_null() {
            None
        } else {
            if !settings::eval_enabled(store) {
                return Err(HarnessError::invalid(
                    "SkillCache ablation arms are available only in evaluation runs.",
                ));
            }
            let arm = body["eval_arm"]
                .as_str()
                .filter(|arm| {
                    matches!(
                        *arm,
                        "written_procedure"
                            | "skillcache_no_replay"
                            | "skillcache_agent"
                            | "skillcache_executor"
                    )
                })
                .ok_or_else(|| {
                    HarnessError::invalid(
                        "Invalid `eval_arm` for compile; expected `written_procedure`, `skillcache_no_replay`, `skillcache_agent`, or `skillcache_executor`.",
                    )
                })?;
            Some(arm.to_string())
        };
        let compiler_override = (!body["compiler"].is_null()).then_some(&body["compiler"]);
        enqueue_compile_for_trace(
            &self.runtime,
            &session,
            &generation,
            compiler_override,
            eval_arm,
            &|job| self.queue(Work::Compile(job)),
        )
    }
    /// What a session's SkillCache state is: its mode, the traces it kept and
    /// what was compiled from them. A trace is listed without its steps; they
    /// stay in the private state.
    fn session(&self, session_id: &str) -> Result<Value> {
        let store = &self.runtime.store;
        let session = store.get(session_id)?;
        let db = store.connection()?;
        let mut traces = db::session_traces(&db, session_id)?;
        for listed in &mut traces {
            let generation = listed["generation"].as_str().unwrap_or("").to_string();
            let reason = if !snapshot::load(&store.state, &generation)
                .is_ok_and(|snapshot| snapshot.is_some())
            {
                Some(snapshot::unavailable_reason())
            } else if !replay_result::available(&store.state, &generation) {
                Some(replay_result::MISSING.to_owned())
            } else {
                None
            };
            listed["replayable"] = reason.is_none().into();
            if let Some(reason) = reason {
                listed["replay_reason"] = reason.into();
            }
            listed["compiling"] = db::compile_pending(&db, &generation)?.into();
        }
        let compiles = db::session_compiles(&db, session_id)?;
        let runs = db::session_runs(&db, session_id)?;
        let mut capsules = Vec::new();
        for compiled in &compiles {
            if let Some(id) = compiled["capsule_id"].as_str()
                && let Some(capsule) = db::capsule(&db, id)?
            {
                capsules.push(summary(&capsule));
            }
        }
        Ok(json!({
            "session_id": session_id,
            "mode": settings::mode(&session),
            "settings": session["skillcache"],
            "traces": traces,
            "compiles": compiles,
            "capsules": capsules,
            "runs": runs,
            "retention": {"days": db::TRACE_DAYS, "per_session": db::TRACES_PER_SESSION},
        }))
    }
}

fn project_path(store: &SessionStore, capsule: &Value) -> Result<PathBuf> {
    let project = store.project(capsule["project_id"].as_str().unwrap_or(""))?;
    project["path"]
        .as_str()
        .map(PathBuf::from)
        .ok_or_else(|| HarnessError::missing("The capsule's project is not registered."))
}

/// What a list shows of a capsule: nothing of where its files are.
fn summary(capsule: &Value) -> Value {
    let mut summary = json!({});
    for field in [
        "capsule_id",
        "project_id",
        "skill_name",
        "skill_path",
        "mu",
        "form",
        "status",
        "reason",
        "revision",
        "capabilities",
        "engine_digest",
        "eval_arm",
        "source_session_id",
        "source_generation",
        "compile_id",
        "created_at",
        "updated_at",
        "last_used_at",
    ] {
        summary[field] = capsule[field].clone();
    }
    if let Some(kind) = capsule["kind"].as_str().filter(|k| *k != "skill") {
        summary["kind"] = kind.into();
    }
    if capsule["needs_trace"] == true {
        summary["needs_trace"] = true.into();
    }
    summary
}

impl Domain for SkillCacheDomain {
    fn execute<'a>(&'a self, operation: &'a str, request: Value) -> Reply<'a> {
        Box::pin(async move {
            let parameter = |name: &str| {
                request["params"][name]
                    .as_str()
                    .ok_or_else(|| HarnessError::invalid(format!("Missing {name}.")))
            };
            let value = match operation {
                "skillcache.session" => self.session(parameter("session_id")?)?,
                "skillcache.compile" => self.compile(parameter("session_id")?, &request["body"])?,
                "skillcache.list" => self.list(parameter("project_id")?, &request["query"])?,
                "skillcache.get" => self.get(parameter("project_id")?, parameter("capsule_id")?)?,
                "skillcache.requalify" => {
                    self.requalify(parameter("project_id")?, parameter("capsule_id")?)?
                }
                "skillcache.recompile" => {
                    let capsule = self.owned(parameter("project_id")?, parameter("capsule_id")?)?;
                    enqueue_recompile_for_capsule(&self.runtime, &capsule)?
                }
                "skillcache.invalidate" => {
                    self.invalidate(parameter("project_id")?, parameter("capsule_id")?)?
                }
                "skillcache.retire" => self.retire(
                    parameter("project_id")?,
                    parameter("capsule_id")?,
                    &request["body"],
                )?,
                "skillcache.delete" => {
                    self.delete(parameter("project_id")?, parameter("capsule_id")?)?
                }
                _ => return Err(HarnessError::missing("Unknown SkillCache operation.")),
            };
            Ok(Payload::Json(value))
        })
    }
}

/// Marks every `active` or `needs_recompile` capsule for `(project_id, skill_path)`
/// as `stale` with `reason` and publishes SSE `"stale"` events. Returns the count updated.
pub(crate) fn invalidate_for_skill_path(
    store: &SessionStore,
    project_id: &str,
    skill_path: &str,
    reason: &str,
) -> Result<usize> {
    let needle = skill_path.trim().replace('\\', "/");
    let needle = needle.trim_end_matches('/');
    if needle.is_empty() {
        return Ok(0);
    }
    let db = store.connection()?;
    let rows = {
        let mut query = db.prepare(
            "SELECT capsule_id, skill_path FROM skillcache_capsules
             WHERE project_id=?1 AND status IN ('active','needs_recompile')",
        )?;
        query
            .query_map(rusqlite::params![project_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    let needle_prefix = format!("{needle}/");
    let needle_suffix = format!("/{needle}");
    let needle_mid = format!("/{needle}/");
    let ids: Vec<String> = rows
        .into_iter()
        .filter_map(|(id, raw_path)| {
            let normalized = raw_path.trim().replace('\\', "/");
            let sp = normalized.trim_end_matches('/');
            if sp.is_empty() {
                return None;
            }
            let matches = sp == needle
                || sp.starts_with(&needle_prefix)
                || needle.starts_with(&format!("{sp}/"))
                || sp.ends_with(&needle_suffix)
                || sp.contains(&needle_mid);
            matches.then_some(id)
        })
        .collect();
    for id in &ids {
        db::set_capsule_status(&db, id, "stale", Some(reason))?;
    }
    drop(db);
    for id in &ids {
        store.publish_event(
            "skillcache_capsule",
            None,
            Some(project_id.to_owned()),
            json!({
                "state": "stale",
                "project_id": project_id,
                "capsule_id": id,
                "reason": reason,
            }),
        );
    }
    Ok(ids.len())
}

/// Evaluates IR path/caller updates against each active or needs_recompile
/// capsule's `ArtifactDependencyManifest` with a 2-second `RACY_NS` guard.
/// Marks drifted capsules `stale`, publishes SSE `"stale"`, and queues
/// background `Work::Requalify` when non-skill workspace dependencies drifted.
pub fn on_ir_paths_updated(
    db: &rusqlite::Connection,
    hub: &crate::store::StreamHub,
    project_id: &str,
    changed_paths: &[String],
    caller_files: &[String],
    ir_revision: u64,
) -> Result<usize> {
    record_ir_revision(project_id, ir_revision);
    if changed_paths.is_empty() && caller_files.is_empty() {
        return Ok(0);
    }
    let capsules = db::active_or_needs_recompile_capsules(db, project_id)?;
    let now_ns = db::current_unix_ns();
    let mut touched = Vec::new();
    for capsule in capsules {
        let Some(capsule_id) = capsule["capsule_id"].as_str().map(str::to_owned) else {
            continue;
        };
        let mut manifest =
            db::ArtifactDependencyManifest::from_value(&capsule["dependency_manifest"]);
        if manifest.skill_paths.is_empty()
            && let Some(skill_path) = capsule["skill_path"]
                .as_str()
                .filter(|p| !p.trim().is_empty())
        {
            manifest
                .skill_paths
                .push(db::normalize_rel_path(skill_path));
        }
        let delta = manifest.evaluate_delta(changed_paths, caller_files, ir_revision, now_ns);
        if delta == db::DeltaMatch::None {
            continue;
        }
        let prior_status = capsule["status"].as_str().unwrap_or("active").to_owned();
        let reason = match delta {
            db::DeltaMatch::ExactSkill => "The source skill changed.",
            db::DeltaMatch::Dependency => "Watched project files or IR dependencies changed.",
            db::DeltaMatch::None => unreachable!(),
        };
        let next_status = if prior_status == "needs_recompile" {
            "needs_recompile"
        } else {
            "stale"
        };
        if next_status != prior_status {
            db::set_capsule_status(db, &capsule_id, next_status, Some(reason))?;
        }
        if delta == db::DeltaMatch::Dependency {
            db::enqueue_pending_requalify(
                db,
                project_id,
                &capsule_id,
                &prior_status,
                Some(reason),
            )?;
        }
        touched.push((capsule_id, delta, reason, next_status));
    }
    for (id, _, reason, state) in &touched {
        hub.publish(
            "skillcache_capsule",
            None,
            Some(project_id.to_owned()),
            json!({
                "state": state,
                "project_id": project_id,
                "capsule_id": id,
                "reason": reason,
                "ir_revision": ir_revision,
            }),
        );
    }
    if touched
        .iter()
        .any(|(_, delta, _, _)| *delta == db::DeltaMatch::Dependency)
    {
        trigger_background_requalify(db);
    }
    Ok(touched.len())
}

fn trace_matches_capsule(trace: &Value, capsule: &Value) -> bool {
    if capsule["kind"] == "tool_chain" {
        return trace["redaction"]["trace_kind"] == "tool_chain";
    }
    let cap_path = capsule["skill_path"].as_str().unwrap_or("");
    let cap_name = capsule["skill_name"].as_str().unwrap_or("");
    let cap_mirror = db::canonical_mirror_skill(cap_path);
    trace["skill_refs"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|sref| {
            let sp = sref["path"].as_str().unwrap_or("");
            let sn = sref["name"].as_str().unwrap_or("");
            sp == cap_path
                || (!cap_name.is_empty() && sn == cap_name)
                || (cap_mirror.is_some() && db::canonical_mirror_skill(sp) == cap_mirror)
        })
}

/// Enqueues compilation of an observed turn when `compile_after_observe` is set
/// on the session, or when `auto_recompile` is enabled and a matching capsule
/// was flagged `needs_trace` after a warm deopt.
pub(crate) fn maybe_enqueue_after_observe(
    runtime: &Arc<Runtime>,
    session: &Value,
    generation: &str,
) -> Option<Value> {
    let compile_after_observe = session["skillcache"]["compile_after_observe"] == true;
    let auto_recompile = runtime
        .store
        .setting(warm::RECOMPILE_THRESHOLDS)
        .ok()
        .flatten()
        .is_some_and(|v| v["auto_recompile"] == true);
    let eval_arm = if settings::eval_enabled(&runtime.store) {
        session["skillcache"]["eval_arm"].as_str().map(String::from)
    } else {
        None
    };
    let project_id = session["project_id"].as_str().unwrap_or("");
    let matching_deopted: Vec<String> = if let Ok(db) = runtime.store.connection()
        && let Ok(Some(stored_trace)) = db::trace(&db, generation)
        && let Ok(waiting) = db::capsules_needing_trace(&db, project_id)
    {
        waiting
            .into_iter()
            .filter(|cap| {
                cap["eval_arm"].as_str() == eval_arm.as_deref()
                    && trace_matches_capsule(&stored_trace, cap)
            })
            .filter_map(|cap| cap["capsule_id"].as_str().map(str::to_owned))
            .collect()
    } else {
        Vec::new()
    };
    if !compile_after_observe && (!auto_recompile || matching_deopted.is_empty()) {
        return None;
    }
    let enqueuer = runtime.enqueuer.as_ref()?;
    match enqueue_compile_for_trace(runtime, session, generation, None, eval_arm, &|job| {
        enqueuer(runtime, job)
    }) {
        Ok(queued) => {
            if !matching_deopted.is_empty()
                && let Ok(db) = runtime.store.connection()
            {
                for cap_id in &matching_deopted {
                    let _ = db::clear_capsule_needs_trace(&db, cap_id);
                }
            }
            let text = if !compile_after_observe {
                "SkillCache queued recompilation from this fresh observed trace."
            } else {
                "SkillCache queued compilation for this observed turn."
            };
            Some(json!({
                "kind": "skillcache",
                "phase": "queued",
                "compile_id": queued["compile_id"],
                "capsule_id": queued["capsule_id"],
                "generation": generation,
                "text": text,
            }))
        }
        Err(error) => Some(json!({
            "kind": "skillcache",
            "phase": "not_queued",
            "reason": if error.code == "conflict" { "queue_full" } else { "unavailable" },
            "generation": generation,
            "text": format!("SkillCache could not queue compilation: {error}"),
        })),
    }
}

/// Triggers recompilation from a fresh observed trace (when `needs_trace` is set)
/// or from the capsule's stored source trace otherwise.
pub(crate) fn enqueue_recompile_for_capsule(
    runtime: &Arc<Runtime>,
    capsule: &Value,
) -> Result<Value> {
    let enqueuer = runtime
        .enqueuer
        .as_ref()
        .ok_or_else(|| HarnessError::unavailable("SkillCache compiler queue is unavailable."))?;
    let capsule_id = capsule["capsule_id"].as_str().unwrap_or("");
    let project_id = capsule["project_id"].as_str().unwrap_or("");
    let needs_trace = capsule["needs_trace"] == true;
    let (session_id, generation) = if needs_trace {
        let db = runtime.store.connection()?;
        let fresh = db::find_fresh_trace_after(&db, project_id, capsule)?.ok_or_else(|| {
            HarnessError::conflict(
                "This capsule deopted during a warm turn and needs a fresh observed trace before recompiling.",
            )
        })?;
        let sid = fresh["session_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                HarnessError::missing("The fresh trace has no source session to recompile from.")
            })?
            .to_owned();
        let generation_id = fresh["generation"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                HarnessError::missing("The fresh trace has no generation to recompile from.")
            })?
            .to_owned();
        (sid, generation_id)
    } else {
        let sid = capsule["source_session_id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                HarnessError::missing("This capsule has no source session to recompile from.")
            })?
            .to_owned();
        let generation_id = capsule["source_generation"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                HarnessError::missing("This capsule has no source trace to recompile from.")
            })?
            .to_owned();
        (sid, generation_id)
    };
    let session = runtime.store.get(&session_id)?;
    let eval_arm = if settings::eval_enabled(&runtime.store) {
        capsule["eval_arm"].as_str().map(String::from)
    } else {
        None
    };
    let queued =
        enqueue_compile_for_trace(runtime, &session, &generation, None, eval_arm, &|job| {
            enqueuer(runtime, job)
        })?;
    if needs_trace
        && !capsule_id.is_empty()
        && let Ok(db) = runtime.store.connection()
    {
        let _ = db::clear_capsule_needs_trace(&db, capsule_id);
    }
    Ok(queued)
}

/// When `RECOMPILE_THRESHOLDS["auto_recompile"]` is enabled and a warm turn
/// moved `capsule_id` into `"needs_recompile"`, flags it `needs_trace` so the
/// next cold run captures a fresh observed trace instead of looping on the
/// failed `source_generation`.
pub(crate) fn maybe_enqueue_recompile_on_finish(
    runtime: &Arc<Runtime>,
    capsule_id: &str,
    emit: &dyn Fn(Value),
) {
    let auto_recompile = runtime
        .store
        .setting(warm::RECOMPILE_THRESHOLDS)
        .ok()
        .flatten()
        .is_some_and(|v| v["auto_recompile"] == true);
    let Ok(db) = runtime.store.connection() else {
        return;
    };
    let Ok(Some(capsule)) = db::capsule(&db, capsule_id) else {
        return;
    };
    if capsule["status"] != "needs_recompile" {
        return;
    }
    if capsule["needs_trace"] != true {
        let _ = db::mark_capsule_needs_trace(&db, capsule_id);
    }
    if !auto_recompile {
        return;
    }
    emit(json!({
        "kind": "skillcache",
        "phase": "not_queued",
        "reason": "needs_trace",
        "capsule_id": capsule_id,
        "generation": capsule["source_generation"],
        "text": "SkillCache deferred recompilation until the next cold turn captures a fresh observed trace.",
    }));
}

fn enqueue_compile_for_trace(
    runtime: &Arc<Runtime>,
    session: &Value,
    generation: &str,
    compiler_override: Option<&Value>,
    eval_arm: Option<String>,
    enqueue: &dyn Fn(Job) -> Result<()>,
) -> Result<Value> {
    let store = &runtime.store;
    let session_id = session["id"].as_str().unwrap_or("");
    let chosen = settings::validate(&json!({
        "mode": "observe",
        "compiler": compiler_override.cloned().unwrap_or_else(|| session["skillcache"]["compiler"].clone()),
    }))?;
    let project_id = session["project_id"].as_str().unwrap_or("").to_string();
    let workspace = PathBuf::from(session["project_path"].as_str().unwrap_or(""));
    let (compile_id, capsule_id) = (crate::store::id(), crate::store::id());
    {
        let db = store.connection()?;
        let stored = db::trace(&db, generation)?
            .filter(|trace| trace["session_id"] == session_id)
            .ok_or_else(|| HarnessError::missing("This session kept no trace of that turn."))?;
        let (skill, tool_chain_sha) = match stored["skill_refs"]
            .as_array()
            .and_then(|skills| skills.first())
            .cloned()
        {
            Some(skill) => (skill, None),
            None if stored["redaction"]["trace_kind"] == "tool_chain" => {
                let tree_sha = stored["redaction"]["tool_chain_tree_sha"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        HarnessError::invalid("Tool-chain trace lacks workspace digest.")
                    })?
                    .to_string();
                (
                    json!({"name": "tool-chain", "path": "", "tree_sha256": tree_sha}),
                    Some(tree_sha),
                )
            }
            None => return Err(HarnessError::invalid("The trace names no skill.")),
        };
        if db::compile_pending(&db, generation)? {
            return Err(HarnessError::conflict(
                "This turn is already being compiled.",
            ));
        }
        let pending_requalifies = runtime
            .requalifying
            .as_ref()
            .and_then(|set| set.lock().ok())
            .map_or(0i64, |set| set.len() as i64);
        if db::pending_compiles(&db)? + pending_requalifies >= WAITING as i64 {
            return Err(HarnessError::conflict(
                "The SkillCache queue is full; try again when a compilation has finished.",
            ));
        }
        db::begin_capsule(
            &db,
            &capsule_id,
            &compile_id,
            &project_id,
            session_id,
            generation,
            &skill,
            eval_arm.as_deref(),
        )?;
        if let Some(tree_sha) = &tool_chain_sha {
            db::set_capsule_kind_meta(&db, &capsule_id, "tool_chain", tree_sha)?;
        }
        db::begin_compile(
            &db,
            &compile_id,
            &capsule_id,
            &project_id,
            session_id,
            generation,
            "claude",
            chosen["compiler"]["model"].as_str(),
            eval_arm.as_deref(),
        )?;
    }
    let job = Job {
        compile_id: compile_id.clone(),
        capsule_id: capsule_id.clone(),
        session_id: session_id.into(),
        generation: generation.into(),
        project_id: project_id.clone(),
        workspace,
        model: chosen["compiler"]["model"].as_str().map(String::from),
        eval_arm: eval_arm.clone(),
    };
    let mut detail = json!({"capsule_id": capsule_id, "compile_id": compile_id, "session_id": session_id, "generation": job.generation});
    if let Some(arm) = &eval_arm {
        detail["eval_arm"] = arm.clone().into();
    }
    if let Err(error) = enqueue(job) {
        let db = store.connection()?;
        let _ = db::finish_compile(
            &db,
            &compile_id,
            "failed",
            0,
            &error.to_string(),
            &Value::Null,
            &Value::Null,
        );
        let _ = db::set_capsule_status(&db, &capsule_id, "failed", Some(&error.to_string()));
        return Err(error);
    }
    runtime.publish(&project_id, "queued", detail);
    Ok(json!({"compile_id": compile_id, "capsule_id": capsule_id, "status": "queued"}))
}

#[cfg(test)]
pub(crate) mod test_support {
    /// A temporary folder that is removed although it held sealed packages,
    /// whose folders are read-only.
    pub(crate) struct Scratch(tempfile::TempDir);
    impl Scratch {
        pub(crate) fn path(&self) -> &std::path::Path {
            self.0.path()
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            infra_skillcache::package::release(self.0.path());
        }
    }
    pub(crate) fn scratch() -> Scratch {
        Scratch(tempfile::tempdir().unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalidate_for_skill_path_marks_active_capsules_stale() {
        let scratch = test_support::scratch();
        let store = SessionStore::open(&scratch.path().join("state")).unwrap();
        let project = scratch.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let project_id = store.register(&project).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let db = store.connection().unwrap();
        let pdf_file =
            json!({"name": "pdf", "path": ".agents/skills/pdf/SKILL.md", "tree_sha256": "0"});
        let pdf_dir = json!({"name": "pdf", "path": ".agents/skills/pdf", "tree_sha256": "0"});
        let pdf_v2 =
            json!({"name": "pdf-v2", "path": ".agents/skills/pdf-v2/SKILL.md", "tree_sha256": "0"});
        db::begin_capsule(
            &db,
            "c-act",
            "k-1",
            &project_id,
            "s1",
            "g1",
            &pdf_file,
            None,
        )
        .unwrap();
        db::begin_capsule(&db, "c-dir", "k-2", &project_id, "s1", "g2", &pdf_dir, None).unwrap();
        db::begin_capsule(&db, "c-v2", "k-3", &project_id, "s1", "g3", &pdf_v2, None).unwrap();
        for (id, name) in [("c-act", "pdf"), ("c-dir", "pdf"), ("c-v2", "pdf-v2")] {
            db::activate_capsule(
                &db,
                &db::Qualified {
                    capsule_id: id,
                    mu_name: name,
                    mu_description: "d",
                    form: "textual",
                    capsule_sha256: "a",
                    package_sha256: "b",
                    skill_files: &json!({}),
                    env_fingerprint: "e",
                    engine_digest: "",
                    artifact_dir: "/tmp/no",
                    capabilities: &json!({}),
                },
            )
            .unwrap();
        }
        drop(db);
        let crate::store::ReplayStart::Frames { mut rx, .. } = store.stream().subscribe(None)
        else {
            panic!("expected frames");
        };
        let changed = invalidate_for_skill_path(
            &store,
            &project_id,
            ".agents/skills/pdf",
            "Skill edited on disk.",
        )
        .unwrap();
        assert_eq!(changed, 2);
        let db = store.connection().unwrap();
        assert_eq!(
            db::capsule(&db, "c-act").unwrap().unwrap()["status"],
            "stale"
        );
        assert_eq!(
            db::capsule(&db, "c-dir").unwrap().unwrap()["status"],
            "stale"
        );
        assert_eq!(
            db::capsule(&db, "c-v2").unwrap().unwrap()["status"],
            "active"
        );
        let mut stale_ids = Vec::new();
        while let Ok(frame) = rx.try_recv() {
            if frame.event == "skillcache_capsule" && frame.data["state"] == "stale" {
                stale_ids.push(frame.data["capsule_id"].as_str().unwrap().to_owned());
            }
        }
        stale_ids.sort();
        assert_eq!(stale_ids, vec!["c-act", "c-dir"]);
    }

    #[test]
    fn maybe_enqueue_after_observe_emits_not_queued_when_queue_is_full() {
        let scratch = test_support::scratch();
        let workspace = scratch.path().join("project");
        let skill_dir = workspace.join(".agents/skills/pdf");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "# PDF\n").unwrap();
        let store = SessionStore::open(&scratch.path().join("state")).unwrap();
        let project = store.register(&workspace).unwrap();
        let project_id = project["id"].as_str().unwrap().to_owned();
        let session = store
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
            .unwrap();
        let sid = session["id"].as_str().unwrap().to_owned();
        let db = store.connection().unwrap();
        db::save_trace(
            &db,
            &db::TraceRow {
                generation: "gen-full",
                session_id: &sid,
                provider: "claude",
                model: None,
                request: "do pdf",
                skill_refs: &json!([{"name": "pdf", "path": ".agents/skills/pdf/SKILL.md", "tree_sha256": "0"}]),
                steps: &json!([{"index": 0, "kind": "tool", "tool": "Read"}]),
                usage: &Value::Null,
                redaction: &json!({"redacted": 0}),
            },
        )
        .unwrap();
        drop(db);
        let rt = Arc::new(Runtime {
            store,
            providers: Arc::new(ProviderDomain {
                providers: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
            }),
            engine: None,
            cancel: Arc::new(AtomicBool::new(false)),
            enqueuer: Some(Arc::new(|_, _| Err(HarnessError::conflict("queue full")))),
            requalifying: None,
        });
        let mut s = session;
        s["skillcache"] = json!({"mode": "observe", "compile_after_observe": true});
        let event = maybe_enqueue_after_observe(&rt, &s, "gen-full").unwrap();
        assert_eq!(event["phase"], "not_queued");
        assert_eq!(event["reason"], "queue_full");
        assert_eq!(event["generation"], "gen-full");
    }

    #[tokio::test]
    async fn work_queue_allows_distinct_projects_to_run_concurrently() {
        let queue = WorkQueue::new();
        let scratch = test_support::scratch();
        let store = SessionStore::open(&scratch.path().join("state")).unwrap();
        let rt = Arc::new(Runtime {
            store,
            providers: Arc::new(ProviderDomain {
                providers: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
            }),
            engine: None,
            cancel: Arc::new(AtomicBool::new(false)),
            enqueuer: None,
            requalifying: None,
        });
        // Two distinct project locks can be locked concurrently while same project serializes.
        let lock_a = queue.project_lock("proj-a");
        let lock_b = queue.project_lock("proj-b");
        let _guard_a = lock_a.lock().await;
        assert!(lock_b.try_lock().is_ok());
        assert!(queue.project_lock("proj-a").try_lock().is_err());
        assert!(!rt.cancel.load(Ordering::Acquire));
    }

    #[test]
    fn on_ir_paths_updated_invalidates_dependencies_with_racy_ns_guard() {
        let scratch = test_support::scratch();
        let workspace = scratch.path().join("project");
        std::fs::create_dir_all(workspace.join(".claude/skills/deploy")).unwrap();
        std::fs::write(
            workspace.join(".claude/skills/deploy/SKILL.md"),
            "# Deploy\n",
        )
        .unwrap();
        let store = SessionStore::open(&scratch.path().join("state")).unwrap();
        let project = store.register(&workspace).unwrap();
        let project_id = project["id"].as_str().unwrap().to_owned();
        let session = store
            .create(
                &json!({
                    "project_id": project_id,
                    "provider": "claude",
                    "mode": "edit",
                    "workflow": "native",
                    "permissions": "auto"
                }),
                "deploy",
                &[],
            )
            .unwrap();
        let sid = session["id"].as_str().unwrap().to_owned();
        let db = store.connection().unwrap();
        let skill = json!({
            "name": "deploy",
            "path": ".claude/skills/deploy/SKILL.md",
            "tree_sha256": "a".repeat(64)
        });
        db::begin_capsule(
            &db,
            "cap-ir",
            "cmp-ir",
            &project_id,
            &sid,
            "g-ir",
            &skill,
            None,
        )
        .unwrap();
        db::activate_capsule(
            &db,
            &db::Qualified {
                capsule_id: "cap-ir",
                mu_name: "deploy",
                mu_description: "Deploy app",
                form: "textual",
                capsule_sha256: "b",
                package_sha256: "p",
                skill_files: &json!({}),
                env_fingerprint: "e",
                engine_digest: "d",
                artifact_dir: "/tmp/cap-ir",
                capabilities: &json!({}),
            },
        )
        .unwrap();
        let stale_old_rev = db::ArtifactDependencyManifest {
            skill_paths: vec![".claude/skills/deploy/SKILL.md".into()],
            manifest_files: vec!["composer.json".into()],
            watched_globs: vec!["routes/**".into()],
            caller_files: vec!["app/Deployer.php".into()],
            ir_revision: 50,
            verified_ns: 0,
            ..Default::default()
        };
        db::set_dependency_manifest(&db, "cap-ir", &stale_old_rev).unwrap();
        let crate::store::ReplayStart::Frames { mut rx, .. } = store.stream().subscribe(None)
        else {
            panic!("expected replay frames");
        };
        // Unrelated file changes nothing.
        assert_eq!(
            on_ir_paths_updated(
                &db,
                store.stream(),
                &project_id,
                &["README.md".into()],
                &[],
                51
            )
            .unwrap(),
            0
        );
        // Old revision outside RACY_NS is already covered -> no invalidation.
        assert_eq!(
            on_ir_paths_updated(
                &db,
                store.stream(),
                &project_id,
                &["composer.json".into()],
                &[],
                49
            )
            .unwrap(),
            0
        );
        // Newer IR revision matching watched_globs marks stale and publishes SSE "stale".
        assert_eq!(
            on_ir_paths_updated(
                &db,
                store.stream(),
                &project_id,
                &["routes/web.php".into()],
                &[],
                51
            )
            .unwrap(),
            1
        );
        assert_eq!(
            db::capsule(&db, "cap-ir").unwrap().unwrap()["status"],
            "stale"
        );
        let frame = rx.try_recv().unwrap();
        assert_eq!(frame.event, "skillcache_capsule");
        assert_eq!(frame.data["state"], "stale");
        assert_eq!(frame.data["capsule_id"], "cap-ir");
        assert_eq!(frame.data["ir_revision"], 51);
    }

    #[test]
    fn warm_deopt_flags_needs_trace_and_blocks_stale_recompile_until_fresh_trace() {
        let root = test_support::scratch();
        let ws = root.path().join("proj");
        std::fs::create_dir_all(ws.join(".claude/skills/pdf")).unwrap();
        std::fs::write(ws.join(".claude/skills/pdf/SKILL.md"), "# PDF\n").unwrap();
        let store = SessionStore::open(&root.path().join("state")).unwrap();
        let project_id = store.register(&ws).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let session = store
            .create(
                &json!({"project_id": project_id, "provider": "codex"}),
                "Run pdf",
                &[],
            )
            .unwrap();
        let sid = session["id"].as_str().unwrap().to_owned();
        let tree_sha = "c".repeat(64);
        {
            let db = store.connection().unwrap();
            db.execute(
                "INSERT OR REPLACE INTO harness_settings(name,value) VALUES(?1,?2)",
                rusqlite::params![
                    warm::RECOMPILE_THRESHOLDS,
                    json!({"auto_recompile": true}).to_string()
                ],
            )
            .unwrap();
            db::save_trace(
                &db,
                &db::TraceRow {
                    generation: "g-old",
                    session_id: &sid,
                    provider: "codex",
                    model: None,
                    request: "Run pdf",
                    skill_refs: &json!([{"name": "pdf", "path": ".claude/skills/pdf/SKILL.md", "tree_sha256": tree_sha}]),
                    steps: &json!([{"kind": "command", "command": "php -v"}]),
                    usage: &json!({}),
                    redaction: &json!({"admissible": true}),
                },
            )
            .unwrap();
            let skill_ref = json!({
                "name": "pdf",
                "path": ".claude/skills/pdf/SKILL.md",
                "tree_sha256": tree_sha
            });
            db::begin_capsule(
                &db,
                "cap-deopt",
                "cmp-deopt",
                &project_id,
                &sid,
                "g-old",
                &skill_ref,
                None,
            )
            .unwrap();
            db::activate_capsule(
                &db,
                &db::Qualified {
                    capsule_id: "cap-deopt",
                    mu_name: "pdf-run",
                    mu_description: "PDF",
                    form: "textual",
                    capsule_sha256: "cap-sha",
                    package_sha256: "pkg-sha",
                    skill_files: &json!({}),
                    env_fingerprint: "env",
                    engine_digest: "eng",
                    artifact_dir: "/tmp/cap-deopt",
                    capabilities: &json!({}),
                },
            )
            .unwrap();
            db::set_capsule_status(&db, "cap-deopt", "needs_recompile", Some("Warm deopt"))
                .unwrap();
            db::mark_capsule_needs_trace(&db, "cap-deopt").unwrap();
        }

        let queued = Arc::new(Mutex::new(Vec::<String>::new()));
        let queued_hook = Arc::clone(&queued);
        let rt = Arc::new(Runtime {
            store: store.clone(),
            providers: Arc::new(ProviderDomain {
                providers: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
            }),
            engine: None,
            cancel: Arc::new(AtomicBool::new(false)),
            enqueuer: Some(Arc::new(move |_, job| {
                queued_hook.lock().unwrap().push(job.generation);
                Ok(())
            })),
            requalifying: None,
        });

        let deopted = {
            let db = store.connection().unwrap();
            db::capsule(&db, "cap-deopt").unwrap().unwrap()
        };
        assert_eq!(deopted["needs_trace"], true);
        let err = enqueue_recompile_for_capsule(&rt, &deopted).unwrap_err();
        assert_eq!(err.status, 409);
        assert!(queued.lock().unwrap().is_empty());

        let emitted = Arc::new(Mutex::new(Vec::<Value>::new()));
        let emitted_hook = Arc::clone(&emitted);
        maybe_enqueue_recompile_on_finish(&rt, "cap-deopt", &move |v| {
            emitted_hook.lock().unwrap().push(v);
        });
        assert!(queued.lock().unwrap().is_empty());
        let events = emitted.lock().unwrap().clone();
        assert!(
            events
                .iter()
                .any(|e| e["phase"] == "not_queued" && e["reason"] == "needs_trace"),
            "{events:?}"
        );

        // Once a fresh observed trace lands, recompile picks the fresh generation and clears needs_trace.
        {
            let db = store.connection().unwrap();
            db::save_trace(
                &db,
                &db::TraceRow {
                    generation: "g-fresh",
                    session_id: &sid,
                    provider: "codex",
                    model: None,
                    request: "Run pdf",
                    skill_refs: &json!([{"name": "pdf", "path": ".agents/skills/pdf/SKILL.md", "tree_sha256": tree_sha}]),
                    steps: &json!([{"kind": "command", "command": "php -v"}]),
                    usage: &json!({}),
                    redaction: &json!({"admissible": true}),
                },
            )
            .unwrap();
        }
        let refreshed = {
            let db = store.connection().unwrap();
            db::capsule(&db, "cap-deopt").unwrap().unwrap()
        };
        enqueue_recompile_for_capsule(&rt, &refreshed).unwrap();
        assert_eq!(*queued.lock().unwrap(), vec!["g-fresh".to_string()]);
        let cleared = {
            let db = store.connection().unwrap();
            db::capsule(&db, "cap-deopt").unwrap().unwrap()
        };
        assert_eq!(cleared["needs_trace"], false);
    }

    #[test]
    fn ir_delta_preserves_needs_recompile_and_persists_pending_requalify_when_queue_saturated() {
        let root = test_support::scratch();
        let ws = root.path().join("proj");
        std::fs::create_dir_all(&ws).unwrap();
        let store = SessionStore::open(&root.path().join("state")).unwrap();
        let project_id = store.register(&ws).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let session = store
            .create(
                &json!({"project_id": project_id, "provider": "codex"}),
                "Inspect routes",
                &[],
            )
            .unwrap();
        let sid = session["id"].as_str().unwrap().to_owned();

        let wq = WorkQueue::new();
        let rt = Arc::new(Runtime {
            store: store.clone(),
            providers: Arc::new(ProviderDomain {
                providers: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
            }),
            engine: None,
            cancel: Arc::new(AtomicBool::new(false)),
            enqueuer: None,
            requalifying: Some(Arc::clone(&wq.requalifying)),
        });
        register_requalify_hook(&rt, &wq);

        let manifest = db::ArtifactDependencyManifest {
            skill_paths: vec![".claude/skills/routes/SKILL.md".into()],
            watched_globs: Vec::new(),
            manifest_files: Vec::new(),
            ast_selectors: Vec::new(),
            route_selectors: vec!["routes/api.php".into()],
            table_selectors: Vec::new(),
            caller_files: Vec::new(),
            path_hashes: BTreeMap::new(),
            ir_revision: 10,
            verified_ns: 1,
        };
        {
            let db = store.connection().unwrap();
            let skill_ref = json!({
                "name": "routes",
                "path": ".claude/skills/routes/SKILL.md",
                "tree_sha256": "a".repeat(64)
            });
            for (cid, cmp_id, gen_id, status) in [
                ("cap-active", "cmp-a", "g-a", "active"),
                ("cap-recomp", "cmp-r", "g-r", "needs_recompile"),
            ] {
                db::save_trace(
                    &db,
                    &db::TraceRow {
                        generation: gen_id,
                        session_id: &sid,
                        provider: "codex",
                        model: None,
                        request: "Inspect routes",
                        skill_refs: &json!([skill_ref]),
                        steps: &json!([]),
                        usage: &json!({}),
                        redaction: &json!({}),
                    },
                )
                .unwrap();
                db::begin_capsule(
                    &db,
                    cid,
                    cmp_id,
                    &project_id,
                    &sid,
                    gen_id,
                    &skill_ref,
                    None,
                )
                .unwrap();
                db::activate_capsule(
                    &db,
                    &db::Qualified {
                        capsule_id: cid,
                        mu_name: cid,
                        mu_description: "Routes",
                        form: "textual",
                        capsule_sha256: "cap-sha",
                        package_sha256: "pkg-sha",
                        skill_files: &json!({}),
                        env_fingerprint: "env",
                        engine_digest: "eng",
                        artifact_dir: "/tmp/cap",
                        capabilities: &json!({}),
                    },
                )
                .unwrap();
                db::set_dependency_manifest(&db, cid, &manifest).unwrap();
                if status != "active" {
                    db::set_capsule_status(&db, cid, status, Some("Deopted")).unwrap();
                }
            }
        }

        // Saturate all WAITING = 8 queue slots so background requalify cannot grab a slot now.
        let held = wq
            .slots
            .clone()
            .try_acquire_many_owned(WAITING as u32)
            .unwrap();
        {
            let db = store.connection().unwrap();
            let invalidated = on_ir_paths_updated(
                &db,
                store.stream(),
                &project_id,
                &["routes/api.php".into()],
                &[],
                11,
            )
            .unwrap();
            assert_eq!(invalidated, 2);
            assert_eq!(
                db::capsule(&db, "cap-active").unwrap().unwrap()["status"],
                "stale"
            );
            assert_eq!(
                db::capsule(&db, "cap-recomp").unwrap().unwrap()["status"],
                "needs_recompile"
            );
            assert_eq!(db::pending_requalify_count(&db).unwrap(), 2);
            let pending = db::list_pending_requalify(&db, 10).unwrap();
            let active_row = pending
                .iter()
                .find(|r| r.capsule_id == "cap-active")
                .unwrap();
            let recomp_row = pending
                .iter()
                .find(|r| r.capsule_id == "cap-recomp")
                .unwrap();
            assert_eq!(active_row.prior_status, "active");
            assert_eq!(recomp_row.prior_status, "needs_recompile");

            db::set_capsule_status(&db, "cap-active", "retired", Some("Superseded")).unwrap();
            db::set_capsule_status(&db, "cap-recomp", "retired", Some("Superseded")).unwrap();
        }
        drop(held);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                drain_pending_requalifies(&rt, &wq);
            });
        let db = store.connection().unwrap();
        assert_eq!(db::pending_requalify_count(&db).unwrap(), 0);
    }

    #[test]
    fn package_meta_cache_validates_with_racy_ns_and_avoids_stale_reads() {
        let dir = test_support::scratch();
        let warm = dir.path().join("warm");
        let cap = infra_skillcache::SkillCapsule::from_value(&json!({
            "schema_version": 1,
            "form": "hybrid",
            "mu": {"name": "cached-pkg", "description": "Test cached digest"},
            "source": {"skills": [{"name": "s", "path": ".claude/skills/s/SKILL.md", "tree_sha256": "0".repeat(64)}]},
            "capabilities": {"network": "none", "runtimes": ["python3"]},
            "bindings": [],
            "programs": [{
                "id": "scan",
                "entry": ["python3", "programs/scan/main.py"],
                "inputs": [],
                "outputs": [],
                "check": ["python3", "programs/scan/main.py", "--check"],
                "timeout_seconds": 30,
                "read_only": true
            }],
            "steps": [{"kind": "program", "program": "scan"}],
            "verification": []
        }))
        .unwrap();
        let mut files = infra_skillcache::package::Files::new();
        files.insert("programs/scan/main.py", b"print('ok')\n".to_vec());
        let expected_digest = package::write_warm(&warm, &cap, None, &files).unwrap();

        // Backdate package files beyond RACY_NS so cache populates and hits cleanly.
        let past = std::time::SystemTime::now() - std::time::Duration::from_secs(10);
        for rel in ["capsule.json", "programs/scan/main.py"] {
            let file_path = warm.join(rel);
            let orig_perms = std::fs::metadata(&file_path).unwrap().permissions();
            #[cfg(unix)]
            let writable = {
                use std::os::unix::fs::PermissionsExt;
                std::fs::Permissions::from_mode(orig_perms.mode() | 0o200)
            };
            #[cfg(not(unix))]
            #[expect(clippy::permissions_set_readonly_false)]
            let writable = {
                let mut p = orig_perms.clone();
                p.set_readonly(false);
                p
            };
            std::fs::set_permissions(&file_path, writable).unwrap();
            std::fs::File::options()
                .write(true)
                .open(&file_path)
                .unwrap()
                .set_modified(past)
                .unwrap();
            std::fs::set_permissions(&file_path, orig_perms).unwrap();
        }
        assert_eq!(cached_package_digest(&warm).unwrap(), expected_digest);
        assert_eq!(cached_package_read_only(&warm), Some(true));
        let canon_warm = std::fs::canonicalize(&warm).unwrap();
        assert!(
            package_meta_cache()
                .lock()
                .unwrap()
                .entries
                .contains_key(&canon_warm)
        );
        let _ = package::remove(&warm);
    }
}
