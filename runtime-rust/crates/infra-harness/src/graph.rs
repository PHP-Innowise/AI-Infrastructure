//! Optional code graph of a project, built by the external Graphify tool.
//!
//! The Harness finds an installed `graphify`, runs its deterministic `update`
//! pass in the background and tells an agent that the result exists. A build
//! starts without the previous graph, so a deleted file leaves no nodes behind,
//! and the published graph is replaced only by a build that succeeded.
mod builds;
mod domain;
pub(crate) mod file_types;
mod helpers;
mod map_ops;
pub(crate) mod shell_reads;
mod targets;
#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use self::helpers::publish_for_tests;
use self::helpers::*;
pub use self::helpers::{
    Grant, History, Observed, Print, Signal, Tool, Trigger, asked, expected, grant, live, prepare,
    print, progress, prompt_line, signal,
};
pub(crate) use self::helpers::{RunChange, service_offer};

use crate::{
    HarnessError, Result,
    graph_changes::{self, Pull, Resolved, Target},
    graph_map::Sharing,
    projects,
    services::{Domain, Payload, Reply},
    store::{self, SessionStore},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock, Weak,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// Server-side paging, neighborhood depth, level-of-detail, and weight filter controls for `graph.map`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MapQueryParams {
    pub offset: usize,
    pub limit: Option<usize>,
    pub depth: Option<usize>,
    pub lod: Option<crate::graph_map::Lod>,
    pub min_weight: Option<u64>,
}

impl MapQueryParams {
    /// Parses HTTP/domain query parameters for `offset` (`0..=1_000_000`),
    /// `limit` (`1..=500`), and `depth` (`1..=6`).
    pub fn parse(
        offset_raw: Option<&str>,
        limit_raw: Option<&str>,
        depth_raw: Option<&str>,
    ) -> Result<Self> {
        Self::parse_with(offset_raw, limit_raw, depth_raw, None, None)
    }

    /// Parses paging, depth, progressive `lod`, and `min_weight` (`1..=1_000_000`) parameters.
    pub fn parse_with(
        offset_raw: Option<&str>,
        limit_raw: Option<&str>,
        depth_raw: Option<&str>,
        lod_raw: Option<&str>,
        min_weight_raw: Option<&str>,
    ) -> Result<Self> {
        let offset = match offset_raw {
            None | Some("") => 0,
            Some(text) => text
                .parse::<usize>()
                .ok()
                .filter(|v| *v <= 1_000_000)
                .ok_or_else(|| HarnessError::invalid("Invalid code graph offset."))?,
        };
        let limit = match limit_raw {
            None | Some("") => None,
            Some(text) => Some(
                text.parse::<usize>()
                    .ok()
                    .filter(|v| (1..=500).contains(v))
                    .ok_or_else(|| HarnessError::invalid("Invalid code graph limit."))?,
            ),
        };
        let depth = match depth_raw {
            None | Some("") => None,
            Some(text) => Some(
                text.parse::<usize>()
                    .ok()
                    .filter(|v| (1..=6).contains(v))
                    .ok_or_else(|| HarnessError::invalid("Invalid code graph depth."))?,
            ),
        };
        let lod = crate::graph_map::Lod::parse(lod_raw)?;
        let min_weight = match min_weight_raw.map(str::trim) {
            None | Some("") => None,
            Some(text) => Some(
                text.parse::<u64>()
                    .ok()
                    .filter(|v| (1..=1_000_000).contains(v))
                    .ok_or_else(|| HarnessError::invalid("Invalid code graph min_weight."))?,
            ),
        };
        Ok(Self {
            offset,
            limit,
            depth,
            lod,
            min_weight,
        })
    }
}

/// Real-time status / invalidation event published for SSE streaming.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct GraphStatusEvent {
    pub project_id: String,
    pub state: String,
    pub reason: &'static str,
    pub revision: u64,
    pub ir_revision: u64,
    #[serde(skip_serializing)]
    pub state_dir: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nodes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edges: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

pub(super) fn global_graph_events() -> &'static tokio::sync::broadcast::Sender<GraphStatusEvent> {
    static TX: OnceLock<tokio::sync::broadcast::Sender<GraphStatusEvent>> = OnceLock::new();
    TX.get_or_init(|| tokio::sync::broadcast::channel(256).0)
}

/// Subscribes to process-wide `graph_status` events across `Graphs` instances (filter by `state_dir`).
pub fn subscribe_graph_events() -> tokio::sync::broadcast::Receiver<GraphStatusEvent> {
    global_graph_events().subscribe()
}

/// What of a graph a request for the map asks to see.
#[derive(Clone, Copy, Debug, Default)]
pub struct Shown<'a> {
    /// The folder or the file that is open; the project where it is empty.
    pub scope: &'a str,
    /// The place whose two sides are listed.
    pub focus: Option<&'a str>,
    /// Optional session ID whose live prompt target & entities are overlaid.
    pub focus_session: Option<&'a str>,
    /// Only the links the tool read from the code.
    pub certain: bool,
    /// Only the names that hold this.
    pub filter: &'a str,
    /// A place whose links are followed step by step, instead of a folder.
    pub followed: Option<&'a Followed>,
    /// Without tests: the files that are tests, and the tests inside others.
    pub notests: bool,
    /// Folders of the open one that are spread out in its picture.
    pub spread: &'a [String],
    /// One link of a picture to look into, instead of a picture: the place
    /// that uses and the place that is used.
    pub link: Option<(&'a str, &'a str)>,
    /// Two places to find the shortest ways between, instead of a folder:
    /// the one they lead from and the one they lead to.
    pub path: Option<(&'a str, &'a str)>,
    /// With a path, by how many links a way may be longer than the
    /// shortest and still be drawn: the ways round, besides the shortest.
    pub longer: usize,
    /// With a change, the tests that fail, as a person named them: the
    /// changed files are ranked by how likely each is behind them.
    pub failing: &'a [String],
    /// A place to find the shortest directed cycle through.
    pub cycle: Option<&'a str>,
    /// The selected PR's base identifier for a cycle or link inspection.
    pub base: Option<&'a str>,
    /// Two places to find what they share, instead of a folder, with
    /// which part of that to draw.
    pub shared: Option<Sharing<'a>>,
    /// Mark the picture with the patterns found where it is open.
    pub patterns: bool,
    pub history: bool,
    pub modules: bool,
    pub no_incoming: bool,
    pub scenario: Option<(&'a str, usize)>,
    pub launch: Option<&'a str>,
    pub compare_launch: Option<&'a str>,
    pub agent: Option<&'a str>,
    /// One found pattern to draw, instead of a folder: its pattern and the
    /// type it is named after.
    pub pattern: Option<&'a str>,
    /// Unfold all folders to render the entire project graph at once.
    pub whole: bool,
    /// Zero-based pagination offset into returned nodes.
    pub offset: usize,
    /// Optional page size cap (`1..=500`) for returned nodes.
    pub limit: Option<usize>,
    /// Optional hierarchy / trace expansion depth (`1..=6`).
    pub depth: Option<usize>,
    /// Optional progressive level of detail (`crate`, `folder`, `file`, `symbol`).
    pub lod: Option<crate::graph_map::Lod>,
    /// Optional minimum edge weight threshold (`1..=1_000_000`).
    pub min_weight: Option<u64>,
}
pub(super) struct ImpactContext {
    pub(super) root: PathBuf,
    pub(super) index: Option<Arc<crate::graph_map::Index>>,
    pub(super) resolved: Option<Arc<Resolved>>,
    pub(super) run_edited: Vec<(String, char)>,
    pub(super) live_focus: Option<crate::graph_live::GraphFocus>,
    pub(super) target_label: String,
}
/// How many places a request may name in one list, and in how many bytes.
pub(super) const UNFOLDED: usize = 256;
pub(super) const UNFOLDED_BYTES: usize = 32 * 1024;
/// A list of places as a request carries it: a JSON list of paths, which a
/// URL carries whatever the paths hold. `many` and `wrong` say what to do
/// about a list that is too long and one that is none.
pub fn places(text: &str, many: &'static str, wrong: &'static str) -> Result<Vec<String>> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    if text.len() > UNFOLDED_BYTES {
        return Err(HarnessError::invalid(many));
    }
    let places =
        serde_json::from_str::<Vec<String>>(text).map_err(|_| HarnessError::invalid(wrong))?;
    if places.len() > UNFOLDED {
        return Err(HarnessError::invalid(many));
    }
    if places
        .iter()
        .any(|place| place.is_empty() || place.len() > 1024)
    {
        return Err(HarnessError::invalid(wrong));
    }
    Ok(places)
}
/// A place whose links are followed step by step in the map, with the places
/// whose uses (`out`) and whose users (`into`) are unfolded, each named once
/// for every step that is unfolded.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Followed {
    pub root: String,
    pub out: Vec<String>,
    pub into: Vec<String>,
}
impl Followed {
    /// Reads what a request names: the place, and the unfolded places of
    /// each side as a list of paths. `None` where no place is followed.
    pub fn parse(root: &str, out: &str, into: &str) -> Result<Option<Self>> {
        if root.is_empty() {
            return Ok(None);
        }
        let list = |text: &str| {
            places(
                text,
                "Too much is unfolded; fold something first.",
                "Invalid unfolded places.",
            )
        };
        Ok(Some(Self {
            root: root.to_owned(),
            out: list(out)?,
            into: list(into)?,
        }))
    }
}
#[derive(Clone, Debug)]
pub(super) enum BuildBackend {
    Graphify(Tool),
    NativeIr,
}

impl BuildBackend {
    pub(super) fn tool_version(&self) -> Option<String> {
        match self {
            Self::Graphify(tool) => tool.version.clone(),
            Self::NativeIr => Some(crate::graph_ir::NATIVE_IR_TOOL.to_owned()),
        }
    }
}

pub(super) enum WatcherMsg {
    Paths(Vec<PathBuf>),
    Stop,
}

pub(super) struct ProjectWatcher {
    pub(super) root: PathBuf,
    pub(super) stop: Arc<AtomicBool>,
    pub(super) tx: std::sync::mpsc::Sender<WatcherMsg>,
    pub(super) watcher: Option<notify::RecommendedWatcher>,
    pub(super) thread: Option<JoinHandle<()>>,
}

impl ProjectWatcher {
    pub(super) fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.watcher.take();
        let _ = self.tx.send(WatcherMsg::Stop);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for ProjectWatcher {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub struct Graphs {
    me: Weak<Graphs>,
    store: SessionStore,
    program: Option<PathBuf>,
    force_native: bool,
    gap: Duration,
    jobs: Mutex<BTreeMap<String, Arc<Job>>>,
    workers: Mutex<Vec<JoinHandle<()>>>,
    tool: Mutex<Option<(Instant, Option<Tool>)>>,
    prints: Mutex<BTreeMap<String, (Instant, Option<Print>)>>,
    finished: Mutex<BTreeMap<String, Instant>>,
    writing: Mutex<()>,
    ir_write: Mutex<()>,
    stopping: AtomicBool,
    autostarting: AtomicBool,
    maps: crate::graph_map::Maps,
    ir_stores: Mutex<BTreeMap<String, Arc<crate::graph_ir::GraphIrStore>>>,
    watchers: Mutex<BTreeMap<String, ProjectWatcher>>,
    history: Mutex<BTreeMap<String, HistoryEntry>>,
    /// Open pull requests of each project as last listed, or why not.
    pulls: Mutex<BTreeMap<String, Listing>>,
    /// Pull requests found for a project, a state and a search.
    searches: Mutex<BTreeMap<(String, String, String), Listing>>,
    /// The change last worked out for each project and target (bounded LRU).
    chosen: Mutex<BTreeMap<(String, String), Arc<Resolved>>>,
    /// Access order for LRU eviction on `chosen`.
    chosen_order: Mutex<Vec<(String, String)>>,
    /// The commit whose graph is being built for each project.
    revisions: Mutex<BTreeMap<String, (String, Arc<Job>)>>,
    /// Why the last build of a commit's graph failed.
    refused: Mutex<BTreeMap<(String, String), String>>,
    /// Synthetic system service graph targets (`sys-<hash>` -> `(root, name)`).
    targets: Mutex<BTreeMap<String, (PathBuf, String)>>,
    /// Broadcast channel for real-time `graph_status` SSE updates.
    events: tokio::sync::broadcast::Sender<GraphStatusEvent>,
    /// Monotonic per-project revision counter bumped on IR/architecture/build updates.
    ir_revisions: Mutex<BTreeMap<String, Arc<AtomicU64>>>,
    /// Per-project coalesced background writer state for `live/graph.json`.
    live_json_writers: Mutex<BTreeMap<String, Arc<LiveJsonFlush>>>,
    /// Per-project lock serializing IR/index invalidation + cache publication against cold loads.
    publish_locks: Mutex<BTreeMap<String, Arc<Mutex<()>>>>,
    /// Incremental delta cache for `run_launches` per `(project, session, launch, compare)`.
    run_caches: Mutex<BTreeMap<RunCacheKey, CachedRunLaunches>>,
    /// Per-key lock providing single-flight serialization on `run_launches`.
    run_locks: Mutex<BTreeMap<RunCacheKey, Arc<Mutex<()>>>>,
    /// Monotonic access counter for `run_caches` LRU eviction.
    run_cache_seq: AtomicU64,
}

pub(super) type RunCacheKey = (String, String, Option<String>, Option<String>);

pub(super) struct CachedRunLaunches {
    pub(super) record_meta: (String, String, String, String),
    pub(super) launches: Vec<Value>,
    pub(super) known: BTreeSet<String>,
    pub(super) last_event_id: i64,
    pub(super) primary: (crate::run_activity::RunLedger, BTreeMap<String, u64>),
    pub(super) followup: Option<(crate::run_activity::RunLedger, BTreeMap<String, u64>)>,
    pub(super) cached: Option<(RunChange, Option<RunChange>)>,
    pub(super) last_used: u64,
}

#[derive(Clone)]
pub(super) enum PendingLiveJson {
    Elements {
        nodes: u64,
        edges: u64,
        ir_nodes: Arc<Vec<crate::graph_ir::IrNodeSpec>>,
        ir_links: Arc<Vec<crate::graph_ir::IrLinkSpec>>,
    },
    #[allow(dead_code)]
    Bytes {
        nodes: u64,
        edges: u64,
        bytes: Arc<Vec<u8>>,
    },
}

#[derive(Default)]
pub(super) struct LiveJsonState {
    pub(super) dirty: bool,
    pub(super) active: bool,
    pub(super) pending: Option<PendingLiveJson>,
}

#[derive(Default)]
pub(super) struct LiveJsonFlush {
    pub(super) cancel: AtomicBool,
    pub(super) state: Mutex<LiveJsonState>,
    pub(super) done_lock: Mutex<()>,
}

impl Drop for Graphs {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        self.shutdown_watchers();
    }
}
pub(super) type Listing = (Instant, Vec<Pull>, Option<String>);
pub(super) struct HistoryEntry {
    pub(super) revision: String,
    pub(super) read_at: Instant,
    pub(super) data: Arc<graph_changes::History>,
}
impl Graphs {
    /// `HARNESS_GRAPHIFY` names the executable when it is not on `PATH`.
    pub fn new(store: SessionStore) -> Arc<Self> {
        let program = std::env::var_os("HARNESS_GRAPHIFY")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        Self::with(store, program, AUTOMATIC_GAP)
    }
    pub fn with(store: SessionStore, program: Option<PathBuf>, gap: Duration) -> Arc<Self> {
        Self::with_options(store, program, Self::force_native_backend(), gap)
    }
    /// Constructs a `Graphs` service pinned to the native tree-sitter SQLite IR backend.
    pub fn with_native_backend(store: SessionStore, gap: Duration) -> Arc<Self> {
        Self::with_options(store, None, true, gap)
    }
    pub(super) fn with_options(
        store: SessionStore,
        program: Option<PathBuf>,
        force_native: bool,
        gap: Duration,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            store,
            program,
            force_native,
            gap,
            jobs: Mutex::new(BTreeMap::new()),
            workers: Mutex::new(Vec::new()),
            tool: Mutex::new(None),
            prints: Mutex::new(BTreeMap::new()),
            finished: Mutex::new(BTreeMap::new()),
            writing: Mutex::new(()),
            ir_write: Mutex::new(()),
            stopping: AtomicBool::new(false),
            autostarting: AtomicBool::new(false),
            maps: crate::graph_map::Maps::default(),
            ir_stores: Mutex::new(BTreeMap::new()),
            watchers: Mutex::new(BTreeMap::new()),
            history: Mutex::new(BTreeMap::new()),
            pulls: Mutex::new(BTreeMap::new()),
            searches: Mutex::new(BTreeMap::new()),
            chosen: Mutex::new(BTreeMap::new()),
            chosen_order: Mutex::new(Vec::new()),
            revisions: Mutex::new(BTreeMap::new()),
            refused: Mutex::new(BTreeMap::new()),
            targets: Mutex::new(BTreeMap::new()),
            events: tokio::sync::broadcast::channel(256).0,
            ir_revisions: Mutex::new(BTreeMap::new()),
            live_json_writers: Mutex::new(BTreeMap::new()),
            publish_locks: Mutex::new(BTreeMap::new()),
            run_caches: Mutex::new(BTreeMap::new()),
            run_locks: Mutex::new(BTreeMap::new()),
            run_cache_seq: AtomicU64::new(0),
        })
    }

    /// Subscribes to real-time `graph_status` events emitted by this `Graphs` instance.
    pub fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<GraphStatusEvent> {
        self.events.subscribe()
    }

    pub(super) fn ir_revision_counter(&self, project: &str) -> Arc<AtomicU64> {
        if let Ok(mut map) = self.ir_revisions.lock() {
            return map
                .entry(project.to_owned())
                .or_insert_with(|| Arc::new(AtomicU64::new(0)))
                .clone();
        }
        Arc::new(AtomicU64::new(0))
    }

    pub(super) fn ir_revision_of(&self, project: &str) -> u64 {
        self.ir_revision_counter(project).load(Ordering::Acquire)
    }

    pub(super) fn bump_ir_revision(&self, project: &str) -> u64 {
        self.ir_revision_counter(project)
            .fetch_add(1, Ordering::AcqRel)
            + 1
    }

    const CHOSEN_LRU_CAP: usize = 64;
    const RUN_CACHE_LRU_CAP: usize = 64;

    pub(super) fn get_chosen(&self, key: &(String, String)) -> Result<Option<Arc<Resolved>>> {
        let found = lock(&self.chosen)?.get(key).cloned();
        if found.is_some()
            && let Ok(mut order) = lock(&self.chosen_order)
        {
            if let Some(pos) = order.iter().position(|entry| entry == key) {
                order.remove(pos);
            }
            order.push(key.clone());
        }
        Ok(found)
    }

    pub(super) fn insert_chosen(
        &self,
        key: (String, String),
        resolved: Arc<Resolved>,
    ) -> Result<()> {
        let mut map = lock(&self.chosen)?;
        let mut order = lock(&self.chosen_order)?;
        if let Some(pos) = order.iter().position(|entry| entry == &key) {
            order.remove(pos);
        }
        order.push(key.clone());
        map.insert(key, resolved);
        while map.len() > Self::CHOSEN_LRU_CAP {
            if let Some(oldest) = (!order.is_empty()).then(|| order.remove(0)) {
                map.remove(&oldest);
            } else if let Some(first) = map.keys().next().cloned() {
                map.remove(&first);
            } else {
                break;
            }
        }
        Ok(())
    }

    pub(super) fn clear_project_chosen(&self, project: &str) -> Result<()> {
        let mut map = lock(&self.chosen)?;
        map.retain(|(name, _), _| name != project);
        if let Ok(mut order) = lock(&self.chosen_order) {
            order.retain(|(name, _)| name != project);
        }
        Ok(())
    }

    pub(super) fn run_lock_for(&self, key: &RunCacheKey) -> Result<Arc<Mutex<()>>> {
        let mut locks = lock(&self.run_locks)?;
        if locks.len() > 128 {
            locks.retain(|existing, m| existing == key || Arc::strong_count(m) > 1);
        }
        Ok(locks
            .entry(key.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone())
    }

    pub(super) fn touch_insert_run_cache(
        &self,
        cache_key: RunCacheKey,
        mut entry: CachedRunLaunches,
    ) {
        entry.last_used = self.run_cache_seq.fetch_add(1, Ordering::Relaxed) + 1;
        if let Ok(mut caches) = lock(&self.run_caches) {
            if !caches.contains_key(&cache_key)
                && caches.len() >= Self::RUN_CACHE_LRU_CAP
                && let Some(lru_key) = caches
                    .iter()
                    .min_by_key(|(_, cached)| cached.last_used)
                    .map(|(k, _)| k.clone())
            {
                caches.remove(&lru_key);
            }
            caches.insert(cache_key, entry);
        }
    }

    pub(super) fn project_publish_lock(&self, project: &str) -> Arc<Mutex<()>> {
        if let Ok(mut locks) = self.publish_locks.lock() {
            return locks
                .entry(project.to_owned())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone();
        }
        Arc::new(Mutex::new(()))
    }

    pub(super) fn publish_native_index(
        &self,
        project: &str,
        write_epoch: u64,
        index: Arc<crate::graph_map::Index>,
    ) -> u64 {
        let rev = self.bump_ir_revision(project);
        self.maps.forget_live(project);
        self.maps.put_live_ir(project, rev, write_epoch, index);
        rev
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_status_event(
        &self,
        project: &str,
        state: &str,
        reason: &'static str,
        revision: u64,
        nodes: Option<u64>,
        edges: Option<u64>,
        message: Option<String>,
    ) {
        let backend = self
            .setting(project)
            .ok()
            .and_then(|s| s["built"]["tool"].as_str().map(str::to_owned));
        let event = GraphStatusEvent {
            project_id: project.to_owned(),
            state: state.to_owned(),
            reason,
            revision,
            ir_revision: revision,
            state_dir: self.store.state.clone(),
            backend,
            nodes,
            edges,
            message,
        };
        let _ = self.events.send(event.clone());
        let _ = global_graph_events().send(event);
    }

    pub(super) fn live_index_with_hit(
        &self,
        project: &str,
        root: &Path,
    ) -> Result<(Arc<crate::graph_map::Index>, bool)> {
        let published = live(&self.store.state, project);
        let use_sqlite_ir = crate::graph_ir::GraphIrStore::exists(&self.store.state, project)
            && (self.force_native
                || !published.is_file()
                || self
                    .setting(project)
                    .ok()
                    .and_then(|s| s["built"]["tool"].as_str().map(str::to_owned))
                    .as_deref()
                    == Some(crate::graph_ir::NATIVE_IR_TOOL));
        if use_sqlite_ir && let Some(ir) = self.get_ir_store_readonly(project) {
            let epoch = ir.current_write_epoch();
            let rev = self.ir_revision_of(project);
            if let Some(index) = self.maps.cached_live_ir(project, rev, epoch) {
                return Ok((index, true));
            }
            let plock = self.project_publish_lock(project);
            let _publish_guard = lock(&plock)?;
            let epoch = ir.current_write_epoch();
            let rev = self.ir_revision_of(project);
            if let Some(hit) = self.maps.get_or_load_live_ir(project, rev, epoch, || {
                if ir.summary_counts()?.is_none() {
                    return Ok(None);
                }
                let (ir_nodes, ir_links, ..) = ir.export_ir_graph_elements()?;
                let index = Arc::new(crate::graph_map::Index::from_ir_parts(
                    root,
                    Some(ir.db_path().to_path_buf()),
                    Some(crate::graph_ir::NATIVE_IR_TOOL.to_owned()),
                    ir_nodes,
                    ir_links,
                )?);
                Ok(Some(index))
            })? {
                return Ok(hit);
            }
        }
        self.maps.index_with_cache_hit(project, &published, root)
    }

    pub(super) fn live_json_flush_slot(&self, project: &str) -> Arc<LiveJsonFlush> {
        if let Ok(mut writers) = self.live_json_writers.lock() {
            return writers
                .entry(project.to_owned())
                .or_insert_with(|| Arc::new(LiveJsonFlush::default()))
                .clone();
        }
        Arc::new(LiveJsonFlush::default())
    }

    pub(super) fn cancel_live_json_writer(&self, project: &str) {
        if let Ok(mut writers) = self.live_json_writers.lock()
            && let Some(slot) = writers.remove(project)
        {
            slot.cancel.store(true, Ordering::Release);
            if let Ok(mut st) = slot.state.lock() {
                st.dirty = false;
                st.pending = None;
            }
            let _guard = slot.done_lock.lock();
        }
    }

    pub(super) fn schedule_live_json_write(
        &self,
        project: &str,
        root: PathBuf,
        snapshot: Option<PendingLiveJson>,
    ) {
        if self.stopping.load(Ordering::Acquire) {
            return;
        }
        let slot = self.live_json_flush_slot(project);
        slot.cancel.store(false, Ordering::Release);
        let should_spawn = {
            let Ok(mut st) = slot.state.lock() else {
                return;
            };
            if let Some(snap) = snapshot {
                st.pending = Some(snap);
            }
            st.dirty = true;
            if st.active {
                false
            } else {
                st.active = true;
                true
            }
        };
        if !should_spawn {
            return;
        }
        let Some(me) = self.me.upgrade() else {
            if let Ok(mut st) = slot.state.lock() {
                st.active = false;
            }
            return;
        };
        let weak_me = Arc::downgrade(&me);
        let proj = project.to_owned();
        let worker_slot = Arc::clone(&slot);
        let spawn_res = std::thread::Builder::new()
            .name("harness-graph-json".into())
            .spawn(move || {
                let _done_guard = worker_slot.done_lock.lock();
                struct ActiveReset<'a>(&'a Mutex<LiveJsonState>);
                impl Drop for ActiveReset<'_> {
                    fn drop(&mut self) {
                        if let Ok(mut st) = self.0.lock() {
                            st.active = false;
                        }
                    }
                }
                let _reset = ActiveReset(&worker_slot.state);
                loop {
                    let Some(graphs) = weak_me.upgrade() else {
                        break;
                    };
                    let (should_run, queued_snapshot) = {
                        let Ok(mut st) = worker_slot.state.lock() else {
                            break;
                        };
                        if !st.dirty
                            || graphs.stopping.load(Ordering::Acquire)
                            || worker_slot.cancel.load(Ordering::Acquire)
                        {
                            st.active = false;
                            break;
                        }
                        st.dirty = false;
                        (true, st.pending.take())
                    };
                    if !should_run {
                        break;
                    }
                    if let Ok(setting) = graphs.setting(&proj)
                        && setting["enabled"] == true
                    {
                        let live_graph = live(&graphs.store.state, &proj);
                        let written = match queued_snapshot {
                            Some(PendingLiveJson::Bytes {
                                nodes,
                                edges,
                                bytes,
                            }) => crate::graph_ir::write_live_graph_json_bytes_atomic(
                                &live_graph,
                                &bytes,
                            )
                            .ok()
                            .map(|()| (nodes, edges)),
                            Some(PendingLiveJson::Elements {
                                nodes,
                                edges,
                                ir_nodes,
                                ir_links,
                            }) => crate::graph_ir::encode_node_link_payload_from_elements(
                                &ir_nodes, &ir_links,
                            )
                            .ok()
                            .and_then(|bytes| {
                                crate::graph_ir::write_live_graph_json_bytes_atomic(
                                    &live_graph,
                                    &bytes,
                                )
                                .ok()
                                .map(|()| (nodes, edges))
                            }),
                            None => graphs
                                .get_ir_store_readonly(&proj)
                                .and_then(|ir| ir.write_live_graph_atomic(&live_graph).ok()),
                        };
                        if let Some(latest_counts) = written {
                            let fresh_print = graphs.print(&proj, &root, false);
                            let commit = fresh_print.as_ref().and_then(|p| p.commit.as_deref());
                            write_live_summary(&live_graph, commit, Some(latest_counts), None);
                            let bytes = std::fs::metadata(&live_graph).map_or(0, |m| m.len());
                            let disk = size(&folder(&graphs.store.state, &proj));
                            let _ = graphs.edit(&proj, |st| {
                                if st["built"].is_object() {
                                    st["built"]["bytes"] = json!(bytes);
                                    st["built"]["disk"] = json!(disk);
                                    st["built"]["nodes"] = json!(latest_counts.0);
                                    st["built"]["edges"] = json!(latest_counts.1);
                                }
                            });
                        }
                    }
                }
            });
        if spawn_res.is_err()
            && let Ok(mut st) = slot.state.lock()
        {
            st.active = false;
        }
    }
    /// Registers a synthetic system-service graph workspace (`sys-<hash>`) backed by `root`.
    pub fn register_target(&self, project: &str, root: PathBuf, name: String) -> Result<()> {
        lock(&self.targets)?.insert(project.to_owned(), (root.clone(), name.clone()));
        if self.store.project(project).is_err() {
            let root_str = root.to_string_lossy().into_owned();
            let current = self.setting(project)?;
            if current["target_root"].as_str() != Some(root_str.as_str())
                || current["target_name"].as_str() != Some(name.as_str())
            {
                self.edit(project, |setting| {
                    setting["target_root"] = json!(root_str);
                    setting["target_name"] = json!(name);
                })?;
            }
        }
        Ok(())
    }
    pub(super) fn target_info(&self, project: &str) -> Result<(PathBuf, String)> {
        if let Ok(record) = self.store.project(project) {
            let root = PathBuf::from(record["path"].as_str().unwrap_or(""));
            let name = record["name"].as_str().unwrap_or("project").to_owned();
            return Ok((root, name));
        }
        if let Some(found) = lock(&self.targets)?.get(project).cloned() {
            return Ok(found);
        }
        let setting = self.setting(project)?;
        if let Some(path) = setting["target_root"].as_str().filter(|s| !s.is_empty()) {
            let root = PathBuf::from(path);
            let name = setting["target_name"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    root.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                });
            lock(&self.targets)?.insert(project.to_owned(), (root.clone(), name.clone()));
            return Ok((root, name));
        }
        Err(HarnessError::missing("Project not found."))
    }
    /// What tells one published graph of a project from the next, without
    /// reading it: the size of its file and when it was written. None while
    /// no graph is published.
    pub fn live_stamp(&self, project: &str) -> Option<(u64, std::time::SystemTime)> {
        let meta = std::fs::metadata(live(&self.store.state, project)).ok()?;
        Some((meta.len(), meta.modified().ok()?))
    }
    pub(crate) fn state_dir(&self) -> &Path {
        &self.store.state
    }
    pub fn live_index(&self, project: &str) -> Option<Arc<crate::graph_map::Index>> {
        let root = self.root(project).ok()?;
        self.live_index_with_hit(project, &root)
            .ok()
            .map(|(i, _)| i)
    }
    /// Returns the built snapshot index for `target_spec` when available (second tuple element `true`),
    /// otherwise falls back to the project's live index (`false`).
    pub(crate) fn index_for_target_or_live(
        &self,
        project: &str,
        target_spec: Option<&str>,
    ) -> Option<(Arc<crate::graph_map::Index>, bool)> {
        let Some(spec) = target_spec.map(str::trim).filter(|s| !s.is_empty()) else {
            return self.live_index(project).map(|i| (i, false));
        };
        if let Ok(parsed) = Target::parse(spec)
            && matches!(
                parsed,
                Target::Named(_) | Target::Commit(_) | Target::Pull(_) | Target::Range { .. }
            )
            && let Ok(root) = self.root(project)
            && let Ok(resolved) = self.resolve_chosen_or_commit(project, &root, &parsed)
            && let Ok(Found {
                graph: Some((kept, graph, base)),
                ..
            }) = self.graph_of(project, &root, &resolved)
            && let Ok(idx) = self.maps.index(&kept, &graph, &base)
        {
            return Some((idx, true));
        }
        self.live_index(project).map(|i| (i, false))
    }
    pub fn auto_start(&self, project: &str) -> Result<()> {
        if self.stopping.load(Ordering::Acquire) {
            return Ok(());
        }
        let root = self.root(project)?;
        if !root.is_dir() {
            return Ok(());
        }
        let setting = self.setting(project)?;
        if setting["disabled_by_user"] == true {
            return Ok(());
        }
        if setting["enabled"] != true {
            let _ = self.resolve_backend(true);
            self.edit(project, |setting| {
                setting["enabled"] = json!(true);
                if let Some(fields) = setting.as_object_mut() {
                    fields.remove("disabled_by_user");
                }
            })?;
            return self.refresh(project, Trigger::Enabled);
        }
        let published = setting["built"].is_object() && live(&self.store.state, project).is_file();
        if !published {
            return self.refresh(project, Trigger::Enabled);
        }
        let _ = self.print(project, &root, true);
        self.refresh(project, Trigger::Visit)
    }
    pub(super) fn auto_start_order(&self) -> Vec<String> {
        let Ok(projects) = self.store.projects() else {
            return Vec::new();
        };
        let mut ordered = Vec::new();
        let mut seen = BTreeSet::new();
        if let Ok(summaries) = self.store.summaries() {
            for session in summaries {
                if let Some(id) = session["project_id"].as_str().filter(|id| !id.is_empty())
                    && seen.insert(id.to_owned())
                {
                    ordered.push(id.to_owned());
                }
            }
        }
        for project in projects {
            if let Some(id) = project["id"].as_str()
                && seen.insert(id.to_owned())
            {
                ordered.push(id.to_owned());
            }
        }
        ordered
    }
    pub fn auto_start_projects(self: &Arc<Self>) {
        if self.stopping.load(Ordering::Acquire) {
            return;
        }
        if self
            .autostarting
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let me = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("harness-graph-autostart".into())
            .spawn(move || {
                struct Guard<'a>(&'a AtomicBool);
                impl Drop for Guard<'_> {
                    fn drop(&mut self) {
                        self.0.store(false, Ordering::Release);
                    }
                }
                let _guard = Guard(&me.autostarting);
                for id in me.auto_start_order() {
                    if me.stopping.load(Ordering::Acquire) {
                        break;
                    }
                    let _ = me.auto_start(&id);
                }
            });
        match spawned {
            Ok(handle) => {
                if let Ok(mut workers) = self.workers.lock() {
                    workers.retain(|worker| !worker.is_finished());
                    workers.push(handle);
                }
            }
            Err(_) => {
                self.autostarting.store(false, Ordering::Release);
            }
        }
    }
}
