use super::*;

pub(super) type Kept = (String, u64, Option<SystemTime>, Arc<Index>);

#[derive(Default)]
pub(super) struct MapsCache {
    pub(super) kept_live: Vec<Kept>,
    pub(super) kept_revisions: Vec<Kept>,
    pub(super) kept_live_ir: Vec<(String, u64, u64, Arc<Index>)>,
    pub(super) kept_ir_summaries:
        HashMap<(PathBuf, String), (u64, Option<crate::graph_ir::IrSummary>)>,
    pub(super) inflight: HashMap<String, Arc<InFlight>>,
    pub(super) ir_inflight: HashMap<String, Arc<IrInFlight>>,
}

impl MapsCache {
    pub(super) fn pool_mut(&mut self, key: &str) -> (&mut Vec<Kept>, usize) {
        if key.contains('@') {
            (&mut self.kept_revisions, KEPT_REVISIONS)
        } else {
            (&mut self.kept_live, KEPT_LIVE)
        }
    }
}

pub(super) struct IrInFlight {
    pub(super) ir_revision: u64,
    pub(super) write_epoch: u64,
    pub(super) state: Mutex<Option<std::result::Result<Arc<Index>, HarnessError>>>,
    pub(super) ready: Condvar,
}

pub(super) struct IrInFlightGuard<'a> {
    pub(super) maps: &'a Maps,
    pub(super) project: &'a str,
    pub(super) ir_revision: u64,
    pub(super) write_epoch: u64,
    pub(super) slot: Arc<IrInFlight>,
    pub(super) finished: bool,
}

impl IrInFlightGuard<'_> {
    pub(super) fn complete(
        mut self,
        result: std::result::Result<Arc<Index>, HarnessError>,
    ) -> Result<Arc<Index>> {
        self.finished = true;
        if let Ok(mut cache) = self.maps.cache.lock() {
            let still_current = cache
                .ir_inflight
                .get(self.project)
                .is_some_and(|current| Arc::ptr_eq(current, &self.slot));
            if still_current {
                cache.ir_inflight.remove(self.project);
                if let Ok(ref index) = result {
                    put_live_ir_lru(
                        &mut cache.kept_live_ir,
                        self.project,
                        self.ir_revision,
                        self.write_epoch,
                        index.clone(),
                    );
                }
            }
        }
        let shared = match &result {
            Ok(index) => Ok(index.clone()),
            Err(err) => Err(clone_error(err)),
        };
        if let Ok(mut state) = self.slot.state.lock() {
            *state = Some(shared);
            self.slot.ready.notify_all();
        }
        result
    }
}

impl Drop for IrInFlightGuard<'_> {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        if let Ok(mut cache) = self.maps.cache.lock()
            && cache
                .ir_inflight
                .get(self.project)
                .is_some_and(|current| Arc::ptr_eq(current, &self.slot))
        {
            cache.ir_inflight.remove(self.project);
        }
        if let Ok(mut state) = self.slot.state.lock() {
            *state = Some(Err(HarnessError::unavailable(
                "The code graph IR loader stopped.",
            )));
            self.slot.ready.notify_all();
        }
    }
}

pub(super) fn put_live_ir_lru(
    pool: &mut Vec<(String, u64, u64, Arc<Index>)>,
    project: &str,
    ir_revision: u64,
    write_epoch: u64,
    index: Arc<Index>,
) {
    if let Some(at) = pool.iter().position(|(name, ..)| name == project) {
        let (_, prev_rev, prev_epoch, _) = pool[at];
        if (prev_rev, prev_epoch) > (ir_revision, write_epoch) {
            return;
        }
        pool.remove(at);
    }
    if pool.len() >= KEPT_LIVE {
        pool.remove(0);
    }
    pool.push((project.to_owned(), ir_revision, write_epoch, index));
}

pub(super) struct InFlight {
    pub(super) len: u64,
    pub(super) changed: Option<SystemTime>,
    pub(super) state: Mutex<Option<std::result::Result<Arc<Index>, HarnessError>>>,
    pub(super) ready: Condvar,
}

pub(super) fn clone_error(err: &HarnessError) -> HarnessError {
    HarnessError {
        status: err.status,
        code: err.code,
        message: err.message.clone(),
    }
}

pub(super) struct InFlightGuard<'a> {
    pub(super) maps: &'a Maps,
    pub(super) project: &'a str,
    pub(super) slot: Arc<InFlight>,
    pub(super) finished: bool,
}

impl InFlightGuard<'_> {
    pub(super) fn complete(
        mut self,
        len: u64,
        changed: Option<SystemTime>,
        result: std::result::Result<Arc<Index>, HarnessError>,
    ) -> Result<Arc<Index>> {
        self.finished = true;
        if let Ok(mut cache) = self.maps.cache.lock() {
            let still_current = cache
                .inflight
                .get(self.project)
                .is_some_and(|current| Arc::ptr_eq(current, &self.slot));
            if still_current {
                cache.inflight.remove(self.project);
                if let Ok(ref index) = result {
                    let (pool, cap) = cache.pool_mut(self.project);
                    if let Some(at) = pool.iter().position(|(name, ..)| name == self.project) {
                        pool.remove(at);
                    }
                    if pool.len() >= cap {
                        pool.remove(0);
                    }
                    pool.push((self.project.to_owned(), len, changed, index.clone()));
                }
            }
        }
        let shared = match &result {
            Ok(index) => Ok(index.clone()),
            Err(err) => Err(clone_error(err)),
        };
        if let Ok(mut state) = self.slot.state.lock() {
            *state = Some(shared);
            self.slot.ready.notify_all();
        }
        result
    }
}

impl Drop for InFlightGuard<'_> {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        if let Ok(mut cache) = self.maps.cache.lock()
            && cache
                .inflight
                .get(self.project)
                .is_some_and(|current| Arc::ptr_eq(current, &self.slot))
        {
            cache.inflight.remove(self.project);
        }
        if let Ok(mut state) = self.slot.state.lock() {
            *state = Some(Err(HarnessError::unavailable(
                "The code graph map loader stopped.",
            )));
            self.slot.ready.notify_all();
        }
    }
}

/// Indexes of the graphs looked at most recently, each tied to the size and
/// time of the file it was read from.
#[derive(Default)]
pub struct Maps {
    pub(super) cache: Mutex<MapsCache>,
}
impl Maps {
    /// Reads and caches a project's graph index without holding the cache
    /// mutex during parsing, deduplicating concurrent reads of the same key
    /// and evicting entries in LRU order.
    pub fn index(&self, project: &str, graph: &Path, root: &Path) -> Result<Arc<Index>> {
        self.index_with_cache_hit(project, graph, root)
            .map(|(index, _)| index)
    }

    /// Reads and caches a project's graph index, also reporting whether the
    /// index came from an in-memory cache hit (`true`) or had to be loaded (`false`).
    pub fn index_with_cache_hit(
        &self,
        project: &str,
        graph: &Path,
        root: &Path,
    ) -> Result<(Arc<Index>, bool)> {
        self.load_with_hit(project, graph, |path| {
            let mut index = Index::read(std::fs::File::open(path)?, root)?;
            if project.contains('@')
                && let Ok(spans) = std::fs::File::open(path.with_file_name(TEST_SPANS))
            {
                index.revision_tests = serde_json::from_reader(std::io::BufReader::new(spans)).ok();
            }
            Ok(index)
        })
    }

    #[cfg(test)]
    pub(super) fn load_with<F>(&self, project: &str, graph: &Path, reader: F) -> Result<Arc<Index>>
    where
        F: FnOnce(&Path) -> Result<Index>,
    {
        self.load_with_hit(project, graph, reader)
            .map(|(index, _)| index)
    }

    pub(super) fn load_with_hit<F>(
        &self,
        project: &str,
        graph: &Path,
        reader: F,
    ) -> Result<(Arc<Index>, bool)>
    where
        F: FnOnce(&Path) -> Result<Index>,
    {
        let meta = std::fs::metadata(graph)
            .map_err(|_| HarnessError::missing("This project has no published code graph."))?;
        if meta.len() > BYTE_LIMIT {
            return Err(HarnessError::invalid(
                "The code graph is larger than 2 GiB and cannot be mapped.",
            ));
        }
        let len = meta.len();
        let changed = meta.modified().ok();
        let mut reader = Some(reader);
        loop {
            enum Action {
                Hit(Arc<Index>),
                Wait(Arc<InFlight>),
                Lead(Arc<InFlight>),
            }
            let action = {
                let mut cache = self
                    .cache
                    .lock()
                    .map_err(|_| HarnessError::unavailable("The code graph map is unavailable."))?;
                let (hit_index, wait_slot) = {
                    let (pool, _) = cache.pool_mut(project);
                    if let Some(at) = pool.iter().position(|(name, kept_len, kept_changed, _)| {
                        name == project && *kept_len == len && *kept_changed == changed
                    }) {
                        let entry = pool.remove(at);
                        let index = entry.3.clone();
                        pool.push(entry);
                        (Some(index), None)
                    } else {
                        (None, cache.inflight.get(project).cloned())
                    }
                };
                if let Some(index) = hit_index {
                    Action::Hit(index)
                } else if let Some(existing) = wait_slot {
                    Action::Wait(existing)
                } else {
                    let slot = Arc::new(InFlight {
                        len,
                        changed,
                        state: Mutex::new(None),
                        ready: Condvar::new(),
                    });
                    cache.inflight.insert(project.to_owned(), slot.clone());
                    Action::Lead(slot)
                }
            };
            match action {
                Action::Hit(index) => return Ok((index, true)),
                Action::Wait(slot) => {
                    let outcome = {
                        let mut guard = slot.state.lock().map_err(|_| {
                            HarnessError::unavailable("The code graph map is unavailable.")
                        })?;
                        while guard.is_none() {
                            guard = slot.ready.wait(guard).map_err(|_| {
                                HarnessError::unavailable("The code graph map is unavailable.")
                            })?;
                        }
                        match guard.as_ref().expect("ready state populated") {
                            Ok(index) => Ok(index.clone()),
                            Err(err) => Err(clone_error(err)),
                        }
                    };
                    if slot.len == len && slot.changed == changed {
                        return outcome.map(|index| (index, false));
                    }
                }
                Action::Lead(slot) => {
                    let guard = InFlightGuard {
                        maps: self,
                        project,
                        slot,
                        finished: false,
                    };
                    let parsed = reader.take().expect("single-flight reader called once")(graph)
                        .map(Arc::new);
                    return guard
                        .complete(len, changed, parsed)
                        .map(|index| (index, false));
                }
            }
        }
    }

    /// Stores an in-memory `Index` built directly from SQLite IR without requiring `live/graph.json`.
    pub(crate) fn put_live_ir(
        &self,
        project: &str,
        ir_revision: u64,
        write_epoch: u64,
        index: Arc<Index>,
    ) {
        if let Ok(mut cache) = self.cache.lock() {
            put_live_ir_lru(
                &mut cache.kept_live_ir,
                project,
                ir_revision,
                write_epoch,
                index,
            );
            cache
                .kept_ir_summaries
                .retain(|(_, name), _| name != project);
        }
    }

    /// Re-stamps the most recently cached live IR `Index` for `project` to `ir_revision`
    /// without rebuilding it (used for architecture/non-code updates that leave code IR intact).
    pub(crate) fn rebind_live_ir_revision(&self, project: &str, ir_revision: u64) {
        if let Ok(mut cache) = self.cache.lock() {
            if let Some((_, rev, _, _)) = cache
                .kept_live_ir
                .iter_mut()
                .rfind(|(name, ..)| name == project)
            {
                *rev = ir_revision.max(*rev);
            }
            cache
                .kept_ir_summaries
                .retain(|(_, name), _| name != project);
        }
    }

    /// Returns a live in-memory `Index` built from SQLite IR when present, updating LRU order.
    #[cfg(test)]
    pub(crate) fn live_ir(&self, project: &str) -> Option<(u64, Arc<Index>)> {
        let mut cache = self.cache.lock().ok()?;
        let pos = cache
            .kept_live_ir
            .iter()
            .position(|(name, ..)| name == project)?;
        let entry = cache.kept_live_ir.remove(pos);
        let out = (entry.1, entry.3.clone());
        cache.kept_live_ir.push(entry);
        Some(out)
    }

    /// Returns the most recently cached live IR `Index` for `project`, if any.
    pub(crate) fn latest_live_ir_index(&self, project: &str) -> Option<Arc<Index>> {
        let cache = self.cache.lock().ok()?;
        cache
            .kept_live_ir
            .iter()
            .rfind(|(name, ..)| name == project)
            .map(|entry| entry.3.clone())
    }

    /// Returns a live IR `Index` iff cached at exact `(ir_revision, write_epoch)` without blocking.
    pub(crate) fn cached_live_ir(
        &self,
        project: &str,
        ir_revision: u64,
        write_epoch: u64,
    ) -> Option<Arc<Index>> {
        let mut cache = self.cache.lock().ok()?;
        let pos = cache
            .kept_live_ir
            .iter()
            .position(|(name, rev, epoch, _)| {
                name == project && *rev == ir_revision && *epoch == write_epoch
            })?;
        let entry = cache.kept_live_ir.remove(pos);
        let idx = entry.3.clone();
        cache.kept_live_ir.push(entry);
        Some(idx)
    }

    /// Loads or returns a cached live in-memory IR `Index` keyed on `(ir_revision, write_epoch)`.
    pub(crate) fn get_or_load_live_ir<F>(
        &self,
        project: &str,
        ir_revision: u64,
        write_epoch: u64,
        loader: F,
    ) -> Result<Option<(Arc<Index>, bool)>>
    where
        F: FnOnce() -> Result<Option<Arc<Index>>>,
    {
        let mut loader = Some(loader);
        loop {
            enum IrAction {
                Hit(Arc<Index>),
                Wait(Arc<IrInFlight>),
                Lead(Arc<IrInFlight>),
            }
            let action = {
                let mut cache = self
                    .cache
                    .lock()
                    .map_err(|_| HarnessError::unavailable("The code graph map is unavailable."))?;
                if let Some(pos) = cache.kept_live_ir.iter().position(|(name, rev, epoch, _)| {
                    name == project && *rev == ir_revision && *epoch == write_epoch
                }) {
                    let entry = cache.kept_live_ir.remove(pos);
                    let idx = entry.3.clone();
                    cache.kept_live_ir.push(entry);
                    IrAction::Hit(idx)
                } else if let Some(existing) = cache
                    .ir_inflight
                    .get(project)
                    .filter(|slot| {
                        slot.ir_revision == ir_revision && slot.write_epoch == write_epoch
                    })
                    .cloned()
                {
                    IrAction::Wait(existing)
                } else {
                    let slot = Arc::new(IrInFlight {
                        ir_revision,
                        write_epoch,
                        state: Mutex::new(None),
                        ready: Condvar::new(),
                    });
                    cache.ir_inflight.insert(project.to_owned(), slot.clone());
                    IrAction::Lead(slot)
                }
            };
            match action {
                IrAction::Hit(index) => return Ok(Some((index, true))),
                IrAction::Wait(slot) => {
                    let outcome = {
                        let mut guard = slot.state.lock().map_err(|_| {
                            HarnessError::unavailable("The code graph map is unavailable.")
                        })?;
                        while guard.is_none() {
                            guard = slot.ready.wait(guard).map_err(|_| {
                                HarnessError::unavailable("The code graph map is unavailable.")
                            })?;
                        }
                        match guard.as_ref().expect("ready state populated") {
                            Ok(index) => Ok(index.clone()),
                            Err(err) => Err(clone_error(err)),
                        }
                    };
                    if slot.ir_revision == ir_revision && slot.write_epoch == write_epoch {
                        return outcome.map(|idx| Some((idx, false)));
                    }
                }
                IrAction::Lead(slot) => {
                    let guard = IrInFlightGuard {
                        maps: self,
                        project,
                        ir_revision,
                        write_epoch,
                        slot,
                        finished: false,
                    };
                    match loader.take().expect("single-flight IR loader called once")() {
                        Ok(Some(idx)) => {
                            return guard.complete(Ok(idx)).map(|idx| Some((idx, false)));
                        }
                        Ok(None) => {
                            let _ = guard.complete(Err(HarnessError::missing(
                                "This project has no native IR index.",
                            )));
                            return Ok(None);
                        }
                        Err(err) => return guard.complete(Err(err)).map(|idx| Some((idx, false))),
                    }
                }
            }
        }
    }

    /// Drops ONLY live cached entries for `project`, preserving immutable `project@commit` entries.
    pub(crate) fn forget_live(&self, project: &str) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.kept_live.retain(|(name, ..)| name != project);
            cache.kept_live_ir.retain(|(name, ..)| name != project);
            cache
                .kept_ir_summaries
                .retain(|(_, name), _| name != project);
            cache.inflight.retain(|name, _| name != project);
            cache.ir_inflight.retain(|name, _| name != project);
        }
    }

    /// Drops what was read for a project: its own graph and, under
    /// `project@commit`, the graphs of its other commits.
    pub fn forget(&self, project: &str) {
        if let Ok(mut cache) = self.cache.lock() {
            let matches = |name: &str| {
                name == project
                    || name
                        .strip_prefix(project)
                        .is_some_and(|rest| rest.starts_with('@'))
            };
            cache.kept_live.retain(|(name, ..)| !matches(name));
            cache.kept_revisions.retain(|(name, ..)| !matches(name));
            cache.kept_live_ir.retain(|(name, ..)| !matches(name));
            cache
                .kept_ir_summaries
                .retain(|(_, name), _| !matches(name));
            cache.inflight.retain(|name, _| !matches(name));
            cache.ir_inflight.retain(|name, _| !matches(name));
        }
    }

    /// Reads the optional native SQLite IR summary counters for `project` when present on disk.
    pub(crate) fn ir_summary(
        &self,
        state: &Path,
        project: &str,
    ) -> Option<crate::graph_ir::IrSummary> {
        crate::graph_ir::GraphIrStore::open_readonly_cached(state, project)
            .ok()
            .flatten()
            .and_then(|ir| ir.summary_counts().ok().flatten())
    }

    /// Reads native SQLite IR summary counters cached by `(state, project, ir_revision)`.
    pub(crate) fn ir_summary_for_revision(
        &self,
        state: &Path,
        project: &str,
        ir_revision: u64,
    ) -> Option<crate::graph_ir::IrSummary> {
        let key = (state.to_path_buf(), project.to_owned());
        if let Ok(cache) = self.cache.lock()
            && let Some((cached_rev, summary)) = cache.kept_ir_summaries.get(&key)
            && *cached_rev == ir_revision
        {
            return summary.clone();
        }
        let summary = self.ir_summary(state, project);
        if summary.is_some()
            && let Ok(mut cache) = self.cache.lock()
        {
            cache
                .kept_ir_summaries
                .insert(key, (ir_revision, summary.clone()));
        }
        summary
    }
}

/// Attaches non-breaking `"backend"` and `"ir_summary"` metadata onto a `graph.map` view.
pub(crate) fn attach_ir_metadata(
    view: &mut Value,
    backend: Option<&str>,
    ir_summary: Option<&crate::graph_ir::IrSummary>,
) {
    if let Value::Object(map) = view {
        if let Some(backend) = backend.filter(|s| !s.is_empty()) {
            map.insert("backend".into(), json!(backend));
        }
        if let Some(summary) = ir_summary
            && let Ok(val) = serde_json::to_value(summary)
        {
            map.insert("ir_summary".into(), val);
        }
    }
}

/// Attaches non-breaking `"confidence"` summary (`extracted`, `inferred`, `extracted_ratio`, `ratio`)
/// onto a `graph.map` view from `index`'s edge flows.
pub(crate) fn attach_confidence_metadata(index: &Index, view: &mut Value) {
    if !view.is_object() {
        return;
    }
    let (mut extracted, mut inferred) = (0u64, 0u64);
    for (_, _, pair) in index.pairs() {
        extracted += pair.read;
        inferred += pair.guessed;
    }
    let total = extracted + inferred;
    let ratio = if total == 0 {
        1.0
    } else {
        ((extracted as f64 / total as f64) * 1000.0).round() / 1000.0
    };
    view["confidence"] = json!({
        "extracted": extracted,
        "inferred": inferred,
        "extracted_ratio": ratio,
        "ratio": ratio,
    });
}
