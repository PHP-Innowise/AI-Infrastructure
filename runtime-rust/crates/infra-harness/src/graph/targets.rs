use super::*;

impl Graphs {
    /// The open pull requests as last listed, and listed now when they never
    /// were: what the current work is measured against must not depend on
    /// which request reaches the server first.
    pub(super) fn listed(&self, project: &str, root: &Path) -> Vec<Pull> {
        if let Ok(pulls) = self.pulls.lock()
            && let Some((_, pulls, _)) = pulls.get(project)
        {
            return pulls.clone();
        }
        let listed = graph_changes::pulls(root);
        if let Ok(mut pulls) = self.pulls.lock() {
            pulls.insert(
                project.to_owned(),
                (Instant::now(), listed.0.clone(), listed.1.clone()),
            );
        }
        listed.0
    }
    /// What the checkout is on and the open pull requests of its repository.
    /// Asking the forge takes a moment, so callers keep this off the runtime.
    pub fn pulls(&self, project: &str) -> Result<Value> {
        let root = self.root(project)?;
        let mut place = graph_changes::place(&root)?;
        if place["git"] != true {
            return Ok(place);
        }
        let known = lock(&self.pulls)?
            .get(project)
            .filter(|(at, ..)| at.elapsed() < PULLS_AGE)
            .map(|(_, pulls, reason)| (pulls.clone(), reason.clone()));
        let (pulls, reason) = match known {
            Some(known) => known,
            None => {
                let listed = graph_changes::pulls(&root);
                lock(&self.pulls)?.insert(
                    project.to_owned(),
                    (Instant::now(), listed.0.clone(), listed.1.clone()),
                );
                listed
            }
        };
        // The request the current branch belongs to names what it is stacked on.
        let own = pulls
            .iter()
            .find(|pull| place["branch"].as_str() == Some(pull.head.as_str()));
        place["pull"] = json!(own.map(|pull| pull.number));
        place["against"] = json!(own.map(|pull| pull.base.clone()));
        place["pulls"] = pulls.iter().map(Pull::json).collect();
        place["reason"] = json!(reason);
        Ok(place)
    }
    /// Pull requests in one state, narrowed by the forge's own search, for
    /// the picker. The forge is asked again after a minute.
    pub fn found(&self, project: &str, state: &str, query: &str) -> Result<Value> {
        let root = self.root(project)?;
        let key = (project.to_owned(), state.to_owned(), query.to_owned());
        let known = lock(&self.searches)?
            .get(&key)
            .filter(|(at, ..)| at.elapsed() < PULLS_AGE)
            .map(|(_, pulls, reason)| (pulls.clone(), reason.clone()));
        let (pulls, reason) = match known {
            Some(known) => known,
            None => {
                let listed = graph_changes::search(&root, state, query);
                let mut searches = lock(&self.searches)?;
                searches.retain(|_, (at, ..)| at.elapsed() < PULLS_AGE);
                searches.insert(key, (Instant::now(), listed.0.clone(), listed.1.clone()));
                listed
            }
        };
        Ok(json!({
            "pulls": pulls.iter().map(Pull::json).collect::<Vec<_>>(),
            "reason": reason,
        }))
    }
    /// A page of the project's history for the picker.
    pub fn commits(&self, project: &str, scope: &str, query: &str, skip: usize) -> Result<Value> {
        let (commits, more) = graph_changes::log(&self.root(project)?, scope, query, skip)?;
        Ok(json!({"commits": commits, "more": more, "skip": skip}))
    }
    /// The project's branches for the picker.
    pub fn branches(&self, project: &str) -> Result<Value> {
        let root = self.root(project)?;
        Ok(json!({
            "branches": graph_changes::branches(&root, &self.listed(project, &root))?,
        }))
    }
    /// How long builds of this project take, for the estimate of a commit's
    /// graph: the last such build, else the project's own, which runs the
    /// same passes over nearly the same files.
    pub(super) fn pace(&self, setting: &Value) -> Option<History> {
        let version = json!(self.resolve_backend(false).and_then(|b| b.tool_version()));
        [&setting["revision"], &setting["built"]]
            .into_iter()
            .find(|built| built["tool"] == version && built["seconds"].is_number())
            .and_then(|built| {
                Some(History {
                    seconds: built["seconds"].as_f64()?,
                    tail: built["tail"].as_f64(),
                })
            })
    }
    /// Whether the project's own graph is of this commit: it is when it was
    /// built from a clean checkout of it.
    pub(super) fn current(&self, project: &str, setting: &Value, commit: &str) -> bool {
        live(&self.store.state, project).is_file()
            && setting["built"]["print"].as_str()
                == Some(store::sha(format!("{commit}\0").as_bytes()).as_str())
    }
    pub(super) fn graph_of(
        &self,
        project: &str,
        root: &Path,
        resolved: &Resolved,
    ) -> Result<Found> {
        self.graph_at(
            project,
            root,
            resolved.revision.as_deref(),
            &resolved.prefix,
            false,
        )
    }
    pub(super) fn graph_at(
        &self,
        project: &str,
        root: &Path,
        commit: Option<&str>,
        prefix: &str,
        exact: bool,
    ) -> Result<Found> {
        let ready = |kept: String, graph: PathBuf, base: PathBuf| Found {
            state: "ready",
            build: Value::Null,
            failure: None,
            graph: Some((kept, graph, base)),
        };
        let published = live(&self.store.state, project);
        let Some(commit) = commit else {
            return Ok(ready(project.to_owned(), published, root.to_owned()));
        };
        let own = revision(&self.store.state, project, commit);
        if own.is_file() {
            let tree = own.with_file_name("tree");
            return Ok(ready(
                format!("{project}@{commit}"),
                own,
                if prefix.is_empty() {
                    tree
                } else {
                    tree.join(prefix)
                },
            ));
        }
        let setting = self.setting(project)?;
        if !exact && self.current(project, &setting, commit) {
            return Ok(ready(project.to_owned(), published, root.to_owned()));
        }
        let waiting = |state, build, failure| Found {
            state,
            build,
            failure,
            graph: None,
        };
        if let Some((building, job)) = lock(&self.revisions)?.get(project)
            && building == commit
        {
            return Ok(waiting("building", job.view(self.pace(&setting)), None));
        }
        Ok(
            match lock(&self.refused)?.get(&(project.to_owned(), commit.to_owned())) {
                Some(message) => waiting("failed", Value::Null, Some(message.clone())),
                None => waiting("missing", Value::Null, None),
            },
        )
    }
    /// Reads what a run did to the files of its project from the steps its
    /// session recorded, as the run's own receipt counts them: the files it
    /// changed, each with what was done to it, and the files it opened. A
    /// session of another project is no run of this one.
    pub(super) fn ran(&self, project: &str, root: &Path, session: &str) -> Result<RunChange> {
        Ok(self.run_launches(project, root, session, None, None)?.0)
    }

    pub(super) fn run_launches(
        &self,
        project: &str,
        root: &Path,
        session: &str,
        launch: Option<&str>,
        compare: Option<&str>,
    ) -> Result<(RunChange, Option<RunChange>)> {
        let record = self.store.get(session)?;
        if record["project_id"] != project {
            return Err(HarnessError::missing("This project has no such run."));
        }
        let record_meta = (
            record["title"].as_str().unwrap_or("").to_owned(),
            record["provider"].as_str().unwrap_or("").to_owned(),
            record["status"].as_str().unwrap_or("").to_owned(),
            record["project_path"].as_str().unwrap_or("").to_owned(),
        );
        let cache_key: RunCacheKey = (
            project.to_owned(),
            session.to_owned(),
            launch.map(str::to_owned),
            compare.map(str::to_owned),
        );
        let key_lock = self.run_lock_for(&cache_key)?;
        let _flight_guard = lock(&key_lock)?;
        let ledger = || {
            crate::run_activity::RunLedger::new(
                record["provider"].as_str().unwrap_or(""),
                None,
                false,
            )
        };
        let mut entry = {
            let mut caches = lock(&self.run_caches)?;
            caches
                .remove(&cache_key)
                .unwrap_or_else(|| CachedRunLaunches {
                    record_meta: record_meta.clone(),
                    launches: Vec::new(),
                    known: BTreeSet::new(),
                    last_event_id: 0,
                    primary: (ledger(), BTreeMap::new()),
                    followup: compare.map(|_| (ledger(), BTreeMap::new())),
                    cached: None,
                    last_used: 0,
                })
        };
        let workspace = Path::new(record["project_path"].as_str().unwrap_or(""));
        let mut after = entry.last_event_id;
        let mut new_events = false;
        loop {
            let events = self.store.events(session, after)?;
            let Some(last) = events.last().and_then(|event| event["id"].as_i64()) else {
                break;
            };
            new_events = true;
            for event in &events {
                let id = event["launch_id"]
                    .as_str()
                    .filter(|value| !value.is_empty());
                if let Some(id) = id
                    && entry.known.insert(id.to_owned())
                {
                    entry.launches.push(json!({
                        "id": id, "number": entry.launches.len() + 1, "at": event["at"],
                    }));
                }
                let selected = if launch.is_none() || launch == id {
                    Some(&mut entry.primary)
                } else if compare == id {
                    entry.followup.as_mut()
                } else {
                    None
                };
                if let Some((ledger, reads)) = selected {
                    if event["targets"] == "limited" {
                        ledger.limited = true;
                    }
                    if event["kind"] == "tool" {
                        ledger.observe(event, event["state"].is_string().then_some(event));
                        if let Some((id, files)) = shell_reads::completed(event, workspace) {
                            reads.extend(files.into_iter().map(|file| (file, id)));
                        }
                    }
                }
            }
            after = last;
        }
        entry.last_event_id = after;
        if launch
            .into_iter()
            .chain(compare)
            .any(|id| !entry.known.contains(id))
        {
            self.touch_insert_run_cache(cache_key, entry);
            return Err(HarnessError::missing(
                "This session has no such recorded launch.",
            ));
        }
        let pair = if !new_events
            && entry.record_meta == record_meta
            && let Some(ref cached_pair) = entry.cached
        {
            cached_pair.clone()
        } else {
            let mut primary = Self::run_change(root, session, &record, &entry.primary);
            primary.told["launches"] = json!(entry.launches);
            primary.told["launch"] = json!(launch);
            primary.told["compare_launch"] = json!(compare);
            let followup = entry
                .followup
                .as_ref()
                .map(|run| Self::run_change(root, session, &record, run));
            let pair = (primary, followup);
            entry.record_meta = record_meta;
            entry.cached = Some(pair.clone());
            pair
        };
        self.touch_insert_run_cache(cache_key, entry);
        Ok(pair)
    }

    pub(super) fn run_change(
        root: &Path,
        session: &str,
        record: &Value,
        (ledger, shell_reads): &(crate::run_activity::RunLedger, BTreeMap<String, u64>),
    ) -> RunChange {
        let plain = |path: &str| path.replace('\\', "/").trim_start_matches("./").to_owned();
        let (opened, edited) = ledger.files();
        let opened = opened.into_iter().map(plain).collect::<BTreeSet<_>>();
        let explicit_opened = opened.len();
        let opened = opened
            .into_iter()
            .chain(shell_reads.keys().cloned())
            .collect::<BTreeSet<_>>();
        let files = edited
            .into_iter()
            .map(|(path, status)| graph_changes::Change {
                path: plain(path),
                status,
                from: None,
            })
            .collect::<Vec<_>>();
        let receipt = ledger.receipt();
        let count = |key: &str| receipt[key].as_u64().unwrap_or(0);
        // A run in a folder of its own, as a worktree, names its files as
        // the project does; the graph is the project's all the same.
        let folder = record["project_path"].as_str().unwrap_or("");
        let elsewhere = !folder.is_empty() && Path::new(folder) != root;
        let title = record["title"].as_str().unwrap_or("");
        let told = json!({
            "session": session,
            "title": title,
            "provider": record["provider"],
            "status": record["status"],
            "steps": receipt["steps"],
            "opened": opened.len(),
            "edited": files.len(),
            "shell_opened": shell_reads.len(),
            "outside": (count("opened") + count("edited"))
                .saturating_sub((explicit_opened + files.len()) as u64),
            "searched": receipt["searched"],
            "commands": receipt["commands"],
            "helpers": receipt["helpers"],
            "limited": receipt["limited"],
            "elsewhere": elsewhere,
        });
        RunChange {
            resolved: Arc::new(Resolved {
                target: Target::Run(session.to_owned()),
                kind: "run",
                name: title.chars().take(80).collect(),
                title: None,
                pull: None,
                head: None,
                base: None,
                revision: None,
                prefix: String::new(),
                files,
                notes: Vec::new(),
            }),
            opened,
            shell_reads: shell_reads.clone(),
            told,
        }
    }
    /// Says in a picture of a folder's layers what the chosen change did
    /// to their order: the pairs of files that run against it now and did
    /// not where the change started, and those that did and no longer do.
    /// The graph of that base is one of the cached graphs of commits. Until
    /// it is built the picture says that it is being prepared, and where
    /// the base cannot be told or its graph cannot be built, that the
    /// comparison is not to be had.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn compare_layers(
        &self,
        project: &str,
        root: &Path,
        resolved: &Resolved,
        head: &crate::graph_map::Index,
        view: &mut Value,
        shown: &Shown,
        rules: Option<&crate::graph_map::layer_rules::Rules>,
    ) {
        let base_side = resolved.json()["base"].clone();
        let compare = |view: &mut Value| -> Result<Value> {
            let Some(base) = &resolved.base else {
                return Ok(json!({"state": "unavailable", "reason": "no_base"}));
            };
            let found =
                self.graph_at(project, root, Some(&base.commit), &resolved.prefix, false)?;
            let Some((key, graph, tree)) = found.graph else {
                if found.state == "failed" {
                    return Ok(json!({"state": "unavailable"}));
                }
                if found.state == "missing" {
                    // Beside the graph of a revision the base is kept as
                    // its pair; the checkout's own work has no revision.
                    self.revise(
                        project,
                        root,
                        &base.commit,
                        &resolved.prefix,
                        match &resolved.revision {
                            Some(head) => RevisionBuild::Comparison { head },
                            None => RevisionBuild::Normal,
                        },
                    )?;
                }
                return Ok(json!({"state": "building"}));
            };
            let base = self.maps.index(&key, &graph, &tree)?;
            // The project's own graph, where it stands for the commit, is
            // read with the files of the checkout; the graph of a commit
            // tells its tests by the spans kept beside it.
            if shown.notests
                && key.contains('@')
                && base.has_rust_sources()
                && !base.has_test_spans()
            {
                return Ok(json!({"state": "unavailable", "reason": "tests"}));
            }
            let base = if shown.notests {
                base.without_tests()
            } else {
                base
            };
            let renamed = resolved
                .files
                .iter()
                .filter_map(|file| {
                    (file.status == 'R').then_some((file.path.as_str(), file.from.as_deref()?))
                })
                .collect();
            if view["pattern"]["declared"] == true {
                let Some(rules) = rules else {
                    return Ok(json!({"state": "unavailable", "reason": "no_rules"}));
                };
                head.compare_declared_layers(&base, view, &renamed, shown.certain, rules);
            } else {
                head.compare_layers(&base, view, &renamed, shown.certain);
            }
            Ok(view["pattern"]["change"].take())
        };
        let mut change = compare(view).unwrap_or_else(|_| json!({"state": "unavailable"}));
        change["base"] = base_side;
        view["pattern"]["change"] = change;
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn compare_pr_cycle(
        &self,
        project: &str,
        root: &Path,
        resolved: &Resolved,
        raw_head: &crate::graph_map::Index,
        head: &crate::graph_map::Index,
        snapshot: bool,
        view: &mut Value,
        shown: &Shown,
    ) {
        let base_side = resolved.json()["base"].clone();
        let mut compare = || -> Result<Value> {
            let (Some(base), Some(head_commit)) = (&resolved.base, &resolved.revision) else {
                return Ok(json!({"state": "unavailable", "reason": "no_base"}));
            };
            // Keep ordinary PR views able to reuse the matching live graph.
            // Comparison needs a snapshot so its source filters describe
            // the revision even if the checkout subsequently changes.
            if !snapshot {
                let found =
                    self.graph_at(project, root, Some(head_commit), &resolved.prefix, true)?;
                if found.state == "failed" {
                    return Ok(
                        json!({"state": "unavailable", "reason": "head", "base": base_side}),
                    );
                }
                if found.state == "missing" {
                    self.revise(
                        project,
                        root,
                        head_commit,
                        &resolved.prefix,
                        RevisionBuild::Comparison { head: head_commit },
                    )?;
                }
                return Ok(json!({"state": "waiting_head", "base": base_side}));
            }
            let revision = &base.commit;
            let found = self.graph_at(project, root, Some(revision), &resolved.prefix, true)?;
            let Some((key, graph, tree)) = found.graph else {
                if found.state == "failed" {
                    return Ok(json!({"state": "unavailable", "base": base_side}));
                }
                if found.state == "missing" {
                    self.revise(
                        project,
                        root,
                        &base.commit,
                        &resolved.prefix,
                        RevisionBuild::Comparison { head: head_commit },
                    )?;
                }
                return Ok(json!({"state": "building", "base": base_side}));
            };
            let base = self.maps.index(&key, &graph, &tree)?;
            if shown.notests
                && ((snapshot && raw_head.has_rust_sources() && !raw_head.has_test_spans())
                    || (base.has_rust_sources() && !base.has_test_spans()))
            {
                return Ok(json!({"state": "unavailable", "reason": "tests", "base": base_side}));
            }
            let base = if shown.notests {
                base.without_tests()
            } else {
                base
            };
            let renames = resolved
                .files
                .iter()
                .filter_map(|file| {
                    (file.status == 'R').then_some((file.path.as_str(), file.from.as_deref()?))
                })
                .collect();
            let (origin, place) =
                head.compare_cycle(raw_head, &base, view, &renames, shown.certain);
            Ok(json!({
                "state": "ready", "base": base_side, "origin": origin, "place": place,
                "revision": revision,
            }))
        };
        let result =
            compare().unwrap_or_else(|_| json!({"state": "unavailable", "base": base_side}));
        view["cycle"]["comparison"] = result;
    }
    /// Works out what a target changed and, for a commit that is not the
    /// checkout, starts building its graph. A pull request is looked up and
    /// fetched here, so callers keep this off the runtime.
    pub fn choose(&self, project: &str, target: &Target) -> Result<Value> {
        let root = self.root(project)?;
        let resolved = match target {
            Target::Run(session) => self.ran(project, &root, session)?.resolved,
            Target::SysRun(run_id) => {
                crate::system_graph::sysrun_launches(
                    &self.store,
                    project,
                    &root,
                    run_id,
                    None,
                    None,
                    None,
                )?
                .0
                .resolved
            }
            _ => Arc::new(graph_changes::resolve(
                &root,
                target,
                &self.listed(project, &root),
            )?),
        };
        self.insert_chosen((project.to_owned(), target.text()), resolved.clone())?;
        if let Some(commit) = &resolved.revision {
            self.revise(
                project,
                &root,
                commit,
                &resolved.prefix,
                RevisionBuild::Normal,
            )?;
        }
        let found = self.graph_of(project, &root, &resolved)?;
        let message = revision_message(found.state, &found.build, found.failure.as_deref());
        Ok(json!({
            "project_id": project,
            "target": target.text(),
            "chosen": chosen(&resolved),
            "state": found.state,
            "message": message,
            "build": found.build,
            "failure": found.failure,
        }))
    }
    /// Starts the graph of a commit unless it exists or is being built. A
    /// build for another commit of the project gives way: the last one asked
    /// for is the one being looked at.
    pub(super) fn revise(
        &self,
        project: &str,
        root: &Path,
        commit: &str,
        prefix: &str,
        build: RevisionBuild<'_>,
    ) -> Result<()> {
        if commit.len() < 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(HarnessError::invalid("That is not a commit."));
        }
        if self.stopping.load(Ordering::Acquire)
            || revision(&self.store.state, project, commit).is_file()
            || (matches!(build, RevisionBuild::Normal)
                && self.current(project, &self.setting(project)?, commit))
        {
            return Ok(());
        }
        let backend = self.resolve_backend(true).ok_or_else(|| {
            HarnessError::unavailable(
                "Graphify is not installed. Install it to build the graph of that commit.",
            )
        })?;
        let Some(me) = self.me.upgrade() else {
            return Ok(());
        };
        let job = {
            let mut job = Job::new();
            job.stage.store(WAITING, Ordering::Release);
            let mut revisions = lock(&self.revisions)?;
            if revision(&self.store.state, project, commit).is_file() {
                return Ok(());
            }
            if let Some((building, running)) = revisions.get(project) {
                if building == commit || matches!(build, RevisionBuild::Comparison { .. }) {
                    return Ok(());
                }
                job.revision_gate = running.revision_gate.clone();
                running.cancel.store(true, Ordering::Release);
            }
            let job = Arc::new(job);
            revisions.insert(project.to_owned(), (commit.to_owned(), job.clone()));
            job
        };
        lock(&self.refused)?.remove(&(project.to_owned(), commit.to_owned()));
        let (name, root, commit, prefix) = (
            project.to_owned(),
            root.to_owned(),
            commit.to_owned(),
            prefix.to_owned(),
        );
        let running = job.clone();
        let paired = match build {
            RevisionBuild::Comparison { head } => Some(head.to_owned()),
            _ => None,
        };
        let worker = std::thread::Builder::new()
            .name("harness-graph-revision".into())
            .spawn(move || {
                let _gate = running
                    .revision_gate
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let outcome = me.grow(&name, &root, &backend, &commit, &prefix, &running);
                me.finish(
                    &name,
                    &commit,
                    &running,
                    outcome,
                    &backend,
                    paired.as_deref(),
                );
            });
        match worker {
            Ok(handle) => {
                let mut workers = lock(&self.workers)?;
                workers.retain(|worker| !worker.is_finished());
                workers.push(handle);
                Ok(())
            }
            Err(_) => {
                let mut revisions = lock(&self.revisions)?;
                if revisions
                    .get(project)
                    .is_some_and(|(_, running)| Arc::ptr_eq(running, &job))
                {
                    revisions.remove(project);
                }
                Err(HarnessError::unavailable(
                    "The code graph build could not start.",
                ))
            }
        }
    }
    pub(super) fn run_native_revision(
        &self,
        source: &Path,
        out: &Path,
        job: &Arc<Job>,
    ) -> std::result::Result<Ran, Outcome> {
        let stopped =
            || job.cancel.load(Ordering::Acquire) || self.stopping.load(Ordering::Acquire);
        if stopped() {
            return Err(Outcome::Cancelled);
        }
        let db_path = out.join("graph_ir.sqlite3");
        let ir = crate::graph_ir::GraphIrStore::open_at(&db_path)
            .map_err(|err| Outcome::Failed(err.message))?;
        let stats = ir
            .sync_workspace(source)
            .map_err(|err| Outcome::Failed(err.message))?;
        if stopped() {
            return Err(Outcome::Cancelled);
        }
        job.observe(
            &format!(
                "  AST extraction: {}/{} uncached files (100%)",
                stats.files_scanned, stats.files_scanned
            ),
            false,
        );
        let fresh = out.join("graph.json");
        let (nodes, edges) = ir
            .write_live_graph_atomic(&fresh)
            .map_err(|err| Outcome::Failed(err.message))?;
        drop(ir);
        let bytes = std::fs::metadata(&fresh).map_or(0, |m| m.len());
        if stopped() {
            return Err(Outcome::Cancelled);
        }
        Ok(Ran {
            fresh,
            elapsed: job.started().elapsed().as_secs_f64(),
            read: None,
            built: Some((nodes, edges)),
            bytes,
        })
    }
    /// Builds the graph of one commit: its files are written out beside the
    /// project's graph, the tool reads them as it reads the project, and of
    /// them only what the graph is read with is kept: see `thin`.
    pub(super) fn grow(
        &self,
        project: &str,
        root: &Path,
        backend: &BuildBackend,
        commit: &str,
        prefix: &str,
        job: &Arc<Job>,
    ) -> Outcome {
        let stopped =
            || job.cancel.load(Ordering::Acquire) || self.stopping.load(Ordering::Acquire);
        // One graph is built for a project at a time: two runs of the tool
        // take twice the memory.
        loop {
            if stopped() {
                return Outcome::Cancelled;
            }
            match self.jobs.lock() {
                Ok(jobs) if !jobs.contains_key(project) => break,
                _ => std::thread::sleep(Duration::from_millis(200)),
            }
        }
        let own = revision(&self.store.state, project, commit);
        if own.is_file() {
            return Outcome::Cancelled;
        }
        let home = own.with_file_name("");
        let (tree, out) = (own.with_file_name("tree"), own.with_file_name("work"));
        let _ = std::fs::remove_dir_all(&home);
        if std::fs::create_dir_all(&out).is_err()
            || infra_host::util::private_directory(&folder(&self.store.state, project)).is_err()
        {
            return Outcome::Failed("The graph folder could not be prepared.".into());
        }
        job.stage.store(EXPORTING, Ordering::Release);
        let outcome = (|| {
            if graph_changes::export(root, commit, &tree).is_err() {
                return Outcome::Failed("Git could not write out the files of that commit.".into());
            }
            let source = if prefix.is_empty() {
                tree.clone()
            } else {
                tree.join(prefix)
            };
            if !source.is_dir() {
                return Outcome::Failed("That commit does not have this project's folder.".into());
            }
            if stopped() {
                return Outcome::Cancelled;
            }
            job.restart();
            job.stage.store(0, Ordering::Release);
            let ran = match match backend {
                BuildBackend::Graphify(tool) => self.run(tool, &source, &out, job),
                BuildBackend::NativeIr => self.run_native_revision(&source, &out, job),
            } {
                Ok(ran) => ran,
                Err(outcome) => return outcome,
            };
            if let Some(tests) = thin(&tree) {
                let prefix = if prefix.is_empty() {
                    String::new()
                } else {
                    format!("{prefix}/")
                };
                let spans = tests
                    .iter()
                    .filter_map(|(path, spans)| Some((path.strip_prefix(&prefix)?, spans)))
                    .collect::<BTreeMap<_, _>>();
                if let Ok(spans) = serde_json::to_vec(&spans) {
                    let file = own.with_file_name(crate::graph_map::TEST_SPANS);
                    if std::fs::write(&file, spans).is_err() {
                        let _ = std::fs::remove_file(file);
                    }
                }
            }
            if stopped() {
                return Outcome::Cancelled;
            }
            if std::fs::rename(&ran.fresh, &own).is_err() {
                return Outcome::Failed("The new graph could not be published.".into());
            }
            Outcome::Built(ran.record(ran.bytes))
        })();
        // The files and the tool's cache are as large as the project. What
        // the graph is read with is not.
        let _ = std::fs::remove_dir_all(&out);
        if !matches!(outcome, Outcome::Built(_)) {
            let _ = std::fs::remove_dir_all(&home);
        }
        outcome
    }
    pub(super) fn finish(
        &self,
        project: &str,
        commit: &str,
        job: &Arc<Job>,
        outcome: Outcome,
        backend: &BuildBackend,
        paired: Option<&str>,
    ) {
        let purge = job.purge.load(Ordering::Acquire);
        if purge {
            self.stop_watcher(project);
            self.evict_ir_store(project);
            let _ = std::fs::remove_dir_all(folder(&self.store.state, project));
        }
        match outcome {
            Outcome::Built(built) if !purge => {
                let _ = self.edit(project, |setting| {
                    setting["revision"] = json!({
                        "seconds": built["seconds"],
                        "tail": built["tail"],
                        "tool": backend.tool_version(),
                    });
                });
                self.prune(project, commit, paired);
            }
            Outcome::Failed(message) => {
                if let Ok(mut refused) = self.refused.lock() {
                    refused.insert((project.to_owned(), commit.to_owned()), message);
                }
            }
            _ => {}
        }
        if let Ok(mut revisions) = self.revisions.lock()
            && revisions
                .get(project)
                .is_some_and(|(_, running)| Arc::ptr_eq(running, job))
        {
            revisions.remove(project);
        }
        // The graph was deleted while this build was ending: the folder
        // that was left to it goes now.
        if !purge && job.purge.load(Ordering::Acquire) {
            self.stop_watcher(project);
            self.evict_ir_store(project);
            let _ = std::fs::remove_dir_all(folder(&self.store.state, project));
        }
    }
    /// Keeps the newest graphs of other commits and removes the rest.
    pub(super) fn prune(&self, project: &str, keep: &str, paired: Option<&str>) {
        let home = folder(&self.store.state, project).join("revisions");
        let Ok(entries) = std::fs::read_dir(&home) else {
            return;
        };
        let mut built = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let changed = std::fs::metadata(entry.path().join("graph.json"))
                    .and_then(|meta| meta.modified())
                    .ok()?;
                Some((name == keep || Some(name.as_str()) == paired, changed, name))
            })
            .collect::<Vec<_>>();
        built.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        for (.., name) in built.into_iter().skip(REVISIONS) {
            let _ = std::fs::remove_dir_all(home.join(&name));
        }
    }
    /// The state the interface draws. `visit` also considers a rebuild, as when
    /// the project is opened.
    pub fn status(&self, project: &str, visit: bool) -> Result<Value> {
        let root = self.root(project)?;
        if visit {
            self.refresh(project, Trigger::Visit)?;
        }
        let setting = self.setting(project)?;
        let enabled = setting["enabled"] == true;
        let backend = self.resolve_backend(false);
        let tool = match &backend {
            Some(BuildBackend::Graphify(tool)) => Some(tool.clone()),
            _ => None,
        };
        let job = lock(&self.jobs)?.get(project).cloned();
        let built = &setting["built"];
        let published = built.is_object() && live(&self.store.state, project).is_file();
        let expected_version = backend.as_ref().and_then(BuildBackend::tool_version);
        let history = built["seconds"]
            .as_f64()
            .filter(|_| published && built["tool"] == json!(expected_version))
            .map(|seconds| History {
                seconds,
                tail: built["tail"].as_f64(),
            });
        let stale = enabled
            && job.is_none()
            && published
            && self
                .print(project, &root, false)
                .is_some_and(|now| built["print"].as_str() != Some(now.hash.as_str()));
        let state = if !enabled {
            "off"
        } else if job.is_some() {
            "building"
        } else if backend.is_none() {
            "missing"
        } else if setting["failure"].is_object() {
            "failed"
        } else if !published {
            "empty"
        } else if stale {
            "stale"
        } else {
            "ready"
        };
        let build = job.map(|job| job.view(history));
        let failure = if setting["failure"].is_object() {
            Some(json!({"message": setting["failure"]["message"], "at": setting["failure"]["at"]}))
        } else {
            None
        };
        let published_native =
            published && built["tool"].as_str() == Some(crate::graph_ir::NATIVE_IR_TOOL);
        let message = status_message_for_tool(
            state,
            build.as_ref(),
            failure.as_ref().and_then(|value| value["message"].as_str()),
            published_native.then_some(crate::graph_ir::NATIVE_IR_TOOL),
        );
        let tool_json = match &backend {
            Some(BuildBackend::Graphify(tool)) => json!({
                "available": true,
                "version": tool.version,
                "path": tool.path.to_string_lossy().into_owned(),
            }),
            Some(BuildBackend::NativeIr) => json!({
                "available": true,
                "backend": crate::graph_ir::NATIVE_IR_TOOL,
                "version": crate::graph_ir::NATIVE_IR_TOOL,
                "path": Value::Null,
            }),
            None => json!({
                "available": false,
                "version": Value::Null,
                "path": Value::Null,
            }),
        };
        let ir_summary = published_native
            .then(|| {
                self.maps.ir_summary_for_revision(
                    &self.store.state,
                    project,
                    self.ir_revision_of(project),
                )
            })
            .flatten();
        let graph_json = if published {
            let mut g = json!({
                "commit": built["commit"],
                "built_at": built["at"],
                "nodes": built["nodes"],
                "edges": built["edges"],
                "bytes": built["bytes"],
                "disk": built["disk"],
                "seconds": built["seconds"],
            });
            if published_native {
                g["backend"] = json!(crate::graph_ir::NATIVE_IR_TOOL);
                if let Some(summary) = &ir_summary {
                    g["ir_summary"] = serde_json::to_value(summary).unwrap_or(Value::Null);
                }
            }
            g
        } else {
            Value::Null
        };
        let rev = self.ir_revision_of(project);
        let mut response = json!({
            "project_id": project,
            "enabled": enabled,
            // Whether launches are told about the graph: a choice of its own.
            "agents": setting["agents"] == true,
            "state": state,
            "revision": rev,
            "ir_revision": rev,
            "message": message,
            "tool": tool_json,
            "graph": graph_json,
            "build": build,
            "failure": failure,
        });
        if published_native {
            response["backend"] = json!(crate::graph_ir::NATIVE_IR_TOOL);
        }
        let _ = tool;
        Ok(response)
    }
    pub fn act(&self, project: &str, action: &str) -> Result<Value> {
        self.root(project)?;
        match action {
            "enable" => {
                self.edit(project, |setting| {
                    setting["enabled"] = json!(true);
                    if let Some(fields) = setting.as_object_mut() {
                        fields.remove("disabled_by_user");
                    }
                })?;
                self.refresh(project, Trigger::Enabled)?;
            }
            "disable" => {
                self.stop_watcher(project);
                self.edit(project, |setting| {
                    setting["enabled"] = json!(false);
                    setting["disabled_by_user"] = json!(true);
                })?;
                if let Some(job) = lock(&self.jobs)?.get(project) {
                    job.cancel.store(true, Ordering::Release);
                }
            }
            // Telling agents about the graph is asked for on its own: the
            // map needs none of it, and it costs tokens on every launch.
            "offer" => {
                if self.setting(project)?["enabled"] != true {
                    return Err(HarnessError::invalid(
                        "Turn the code graph on before telling agents about it.",
                    ));
                }
                self.edit(project, |setting| setting["agents"] = json!(true))?;
            }
            "withhold" => {
                self.edit(project, |setting| setting["agents"] = json!(false))?;
            }
            "rebuild" => {
                if self.setting(project)?["enabled"] != true {
                    return Err(HarnessError::invalid(
                        "Turn the code graph on before rebuilding it.",
                    ));
                }
                if self.resolve_backend(true).is_none() {
                    return Err(HarnessError::unavailable(
                        "Graphify is not installed. Install it, then rebuild.",
                    ));
                }
                self.edit(project, |setting| {
                    if let Some(fields) = setting.as_object_mut() {
                        fields.remove("failure");
                    }
                })?;
                self.refresh(project, Trigger::Manual)?;
            }
            "delete" => {
                self.stop_watcher(project);
                self.evict_ir_store(project);
                self.edit(project, |setting| {
                    let target_root = setting.get("target_root").cloned();
                    let target_name = setting.get("target_name").cloned();
                    *setting = json!({"enabled": false, "disabled_by_user": true});
                    if let Some(root) = target_root {
                        setting["target_root"] = root;
                    }
                    if let Some(name) = target_name {
                        setting["target_name"] = name;
                    }
                })?;
                // A commit's graph that is still being built writes into
                // the folder. It is told to end first, and removes the
                // folder when it has ended: it is told while it is known to
                // run, so it cannot end without having heard.
                let growing = lock(&self.revisions)?
                    .get(project)
                    .map(|(_, job)| {
                        job.purge.store(true, Ordering::Release);
                        job.cancel.store(true, Ordering::Release);
                    })
                    .is_some();
                let running = lock(&self.jobs)?.get(project).cloned();
                match running {
                    // The build owns the folder until it has ended.
                    Some(job) => {
                        job.purge.store(true, Ordering::Release);
                        job.cancel.store(true, Ordering::Release);
                    }
                    None => {
                        self.cancel_live_json_writer(project);
                        let home = folder(&self.store.state, project);
                        // Under a commit's build the folder may not go at
                        // once: what that build still writes goes with it.
                        if home.exists()
                            && let Err(error) = std::fs::remove_dir_all(home)
                            && !growing
                        {
                            return Err(error.into());
                        }
                    }
                }
                self.clear_project_chosen(project)?;
                lock(&self.run_caches)?.retain(|(name, ..), _| name != project);
                lock(&self.run_locks)?.retain(|(name, ..), _| name != project);
                lock(&self.refused)?.retain(|(name, _), _| name != project);
                let plock = self.project_publish_lock(project);
                let _publish_guard = lock(&plock)?;
                let rev = self.bump_ir_revision(project);
                self.maps.forget(project);
                lock(&self.history)?.remove(project);
                self.emit_status_event(project, "off", "deleted", rev, None, None, None);
            }
            _ => {
                return Err(HarnessError::invalid(
                    "Choose enable, disable, offer, withhold, rebuild or delete.",
                ));
            }
        }
        if matches!(
            action,
            "enable" | "disable" | "offer" | "withhold" | "rebuild"
        ) {
            let status_val = self.status(project, false)?;
            let state = status_val["state"].as_str().unwrap_or("off");
            let reason = match action {
                "enable" => "enabled",
                "disable" => "disabled",
                "offer" => "agents_offered",
                "withhold" => "agents_withheld",
                _ => "rebuild",
            };
            let rev = self.ir_revision_of(project);
            self.emit_status_event(
                project,
                state,
                reason,
                rev,
                status_val["graph"]["nodes"].as_u64(),
                status_val["graph"]["edges"].as_u64(),
                status_val["message"].as_str().map(str::to_owned),
            );
            return Ok(status_val);
        }
        self.status(project, false)
    }
    pub(super) fn shutdown_watchers(&self) {
        if let Ok(mut watchers) = self.watchers.lock() {
            let owned = std::mem::take(&mut *watchers);
            drop(watchers);
            for mut watcher in owned.into_values() {
                watcher.shutdown();
            }
        }
    }
    /// Ends every build; the tool and its workers do not outlive the Harness.
    pub fn close(&self) {
        self.stopping.store(true, Ordering::Release);
        self.shutdown_watchers();
        if let Ok(mut writers) = self.live_json_writers.lock() {
            let entries: Vec<_> = std::mem::take(&mut *writers).into_values().collect();
            drop(writers);
            for slot in entries {
                slot.cancel.store(true, Ordering::Release);
                let _guard = slot.done_lock.lock();
            }
        }
        if let Ok(mut stores) = self.ir_stores.lock() {
            stores.clear();
        }
        let workers = match self.workers.lock() {
            Ok(mut workers) => std::mem::take(&mut *workers),
            Err(_) => return,
        };
        for worker in workers {
            let _ = worker.join();
        }
    }
}
