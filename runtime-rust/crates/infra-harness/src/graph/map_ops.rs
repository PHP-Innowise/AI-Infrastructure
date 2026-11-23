use super::*;
use crate::graph_map::layer_rules::Rules;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

const RULES_RACY_NS: i128 = 2_000_000_000;
const MAX_RULES_CACHE_ENTRIES: usize = 64;
const MAX_RULES_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
enum RulesDiskStamp {
    Missing { root_mtime_ns: i128 },
    Present { len: u64, mtime_ns: i128 },
}

struct CachedRulesEntry {
    stamp: RulesDiskStamp,
    content_sha: Option<String>,
    rules: Arc<std::result::Result<Option<Rules>, String>>,
    last_used: u64,
}

#[derive(Default)]
struct RulesCache {
    seq: u64,
    entries: HashMap<PathBuf, CachedRulesEntry>,
}

fn rules_cache() -> &'static Mutex<RulesCache> {
    static CACHE: OnceLock<Mutex<RulesCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(RulesCache::default()))
}

#[cfg(test)]
static RULES_PARSE_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn rules_time_ns(time: SystemTime) -> Option<i128> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i128::try_from(d.as_nanos()).ok())
}

fn current_rules_unix_ns() -> i128 {
    rules_time_ns(SystemTime::now()).unwrap_or(0)
}

fn cached_rules_read(root: &Path) -> Arc<std::result::Result<Option<Rules>, String>> {
    cached_rules_read_at(root, current_rules_unix_ns())
}

fn cached_rules_read_at(
    root: &Path,
    now_ns: i128,
) -> Arc<std::result::Result<Option<Rules>, String>> {
    let canon_root = infra_core::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let arch_path = canon_root.join(crate::graph_map::layer_rules::FILE);
    let meta = match std::fs::symlink_metadata(&arch_path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let root_mtime_ns = std::fs::symlink_metadata(&canon_root)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(rules_time_ns)
                .unwrap_or(0);
            let settled = root_mtime_ns > 0
                && now_ns > RULES_RACY_NS
                && root_mtime_ns <= now_ns - RULES_RACY_NS;
            let stamp = RulesDiskStamp::Missing { root_mtime_ns };
            if settled && let Ok(mut guard) = rules_cache().lock() {
                guard.seq = guard.seq.saturating_add(1);
                let seq = guard.seq;
                if let Some(entry) = guard.entries.get_mut(&canon_root)
                    && entry.stamp == stamp
                {
                    entry.last_used = seq;
                    return Arc::clone(&entry.rules);
                }
            }
            #[cfg(test)]
            RULES_PARSE_COUNT.fetch_add(1, Ordering::Relaxed);
            let result = Arc::new(Ok(None));
            if settled && let Ok(mut guard) = rules_cache().lock() {
                insert_rules_cache_entry(
                    &mut guard,
                    canon_root,
                    CachedRulesEntry {
                        stamp,
                        content_sha: None,
                        rules: Arc::clone(&result),
                        last_used: 0,
                    },
                );
            }
            return result;
        }
        Ok(m)
            if !m.file_type().is_symlink() && m.is_file() && m.len() <= MAX_RULES_BYTES as u64 =>
        {
            m
        }
        _ => {
            #[cfg(test)]
            RULES_PARSE_COUNT.fetch_add(1, Ordering::Relaxed);
            return Arc::new(Rules::read(&canon_root));
        }
    };

    let mtime_ns = meta.modified().ok().and_then(rules_time_ns).unwrap_or(0);
    let stamp = RulesDiskStamp::Present {
        len: meta.len(),
        mtime_ns,
    };
    let racy =
        mtime_ns == 0 || now_ns <= RULES_RACY_NS || (now_ns - mtime_ns).abs() < RULES_RACY_NS;

    if !racy && let Ok(mut guard) = rules_cache().lock() {
        guard.seq = guard.seq.saturating_add(1);
        let seq = guard.seq;
        if let Some(entry) = guard.entries.get_mut(&canon_root)
            && entry.stamp == stamp
        {
            entry.last_used = seq;
            return Arc::clone(&entry.rules);
        }
    }

    let Ok(bytes) = std::fs::read(&arch_path) else {
        #[cfg(test)]
        RULES_PARSE_COUNT.fetch_add(1, Ordering::Relaxed);
        return Arc::new(Rules::read(&canon_root));
    };
    if bytes.len() > MAX_RULES_BYTES {
        #[cfg(test)]
        RULES_PARSE_COUNT.fetch_add(1, Ordering::Relaxed);
        return Arc::new(Rules::read(&canon_root));
    }
    let content_sha = crate::store::sha(&bytes);

    if racy && let Ok(mut guard) = rules_cache().lock() {
        guard.seq = guard.seq.saturating_add(1);
        let seq = guard.seq;
        if let Some(entry) = guard.entries.get_mut(&canon_root)
            && entry.stamp == stamp
            && entry.content_sha.as_deref() == Some(content_sha.as_str())
        {
            entry.last_used = seq;
            return Arc::clone(&entry.rules);
        }
    }

    #[cfg(test)]
    RULES_PARSE_COUNT.fetch_add(1, Ordering::Relaxed);
    let parsed = Arc::new(Rules::parse(&bytes).map(Some));
    if let Ok(mut guard) = rules_cache().lock() {
        insert_rules_cache_entry(
            &mut guard,
            canon_root,
            CachedRulesEntry {
                stamp,
                content_sha: Some(content_sha),
                rules: Arc::clone(&parsed),
                last_used: 0,
            },
        );
    }
    parsed
}

fn insert_rules_cache_entry(guard: &mut RulesCache, key: PathBuf, mut entry: CachedRulesEntry) {
    guard.seq = guard.seq.saturating_add(1);
    let seq = guard.seq;
    entry.last_used = seq;
    if guard.entries.len() >= MAX_RULES_CACHE_ENTRIES
        && !guard.entries.contains_key(&key)
        && let Some(evict_key) = guard
            .entries
            .iter()
            .min_by_key(|(_, e)| e.last_used)
            .map(|(k, _)| k.clone())
    {
        guard.entries.remove(&evict_key);
    }
    guard.entries.insert(key, entry);
}

impl Graphs {
    /// One folder of the published graph's dependency map, or, with a
    /// target, the map of what that target changed. With a place followed,
    /// the links of that place step by step instead, on the same graph and
    /// with the same change marked; a place the graph does not know falls
    /// back to the folder. Without tests, all of it is read from the graph
    /// as it is without them. Reading a graph of a large project takes
    /// seconds, so callers keep this off the runtime.
    pub fn map(&self, project: &str, target: Option<&Target>, shown: &Shown) -> Result<Value> {
        let started = Instant::now();
        let (mut view, cache_hit, _active_index) = self.map_inner(project, target, shown)?;
        if let Value::Object(obj) = &mut view {
            if !obj.contains_key("ir_revision") {
                obj.insert("ir_revision".into(), json!(self.ir_revision_of(project)));
            }
            if !obj.contains_key("depth") {
                obj.insert("depth".into(), json!(shown.depth));
            }
        }
        if let Value::Object(obj) = &mut view
            && !obj.get("paging").is_some_and(Value::is_object)
        {
            let mut anchors = Vec::new();
            if let Some(f) = shown.focus.filter(|s| !s.is_empty()) {
                anchors.push(f);
            }
            crate::graph_map::paginate_lens_view(obj, shown.offset, shown.limit, &anchors);
        }
        let elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        crate::graph_map::finalize_telemetry(&mut view, elapsed_ms, cache_hit);
        Ok(view)
    }

    pub(super) fn map_inner(
        &self,
        project: &str,
        target: Option<&Target>,
        shown: &Shown,
    ) -> Result<(Value, bool, Option<Arc<crate::graph_map::Index>>)> {
        let (root, owned_name) = self.target_info(project)?;
        let name = owned_name.as_str();
        let built = &self.setting(project)?["built"];
        let Shown {
            scope,
            focus,
            focus_session,
            certain,
            filter,
            followed,
            notests,
            spread,
            link,
            path,
            longer,
            failing,
            cycle,
            shared,
            base,
            patterns,
            history,
            modules,
            no_incoming,
            scenario,
            launch,
            compare_launch,
            agent,
            pattern,
            whole,
            offset,
            limit,
            depth,
            lod,
            min_weight,
        } = *shown;
        if base.is_some() {
            let valid_pr_base = matches!(target, Some(Target::Pull(_)))
                && cycle.is_some() != link.is_some()
                && path.is_none()
                && shared.is_none()
                && pattern.is_none()
                && followed.is_none();
            let valid_range_base = matches!(target, Some(Target::Range { .. }))
                && !(cycle.is_some() && link.is_some())
                && path.is_none()
                && shared.is_none()
                && pattern.is_none()
                && followed.is_none();
            if !valid_pr_base && !valid_range_base {
                return Err(HarnessError::invalid(
                    "Choose a pull request to inspect a base cycle or link.",
                ));
            }
        }
        if (launch.is_some() || compare_launch.is_some())
            && (!matches!(target, Some(Target::Run(_) | Target::SysRun(_)))
                || compare_launch.is_some() && (launch.is_none() || launch == compare_launch))
        {
            return Err(HarnessError::invalid(
                "Choose two distinct launches of the mapped session to compare.",
            ));
        }
        // The graph as it was read, or as it is without its tests, with how
        // much that leaves out.
        let left = std::cell::Cell::new(None);
        let asked = |index: Arc<crate::graph_map::Index>| {
            if notests {
                let plain = index.without_tests();
                left.set(Some(plain.left_out()));
                plain
            } else {
                index
            }
        };
        use crate::graph_map::layer_rules::Rules;
        let wants_layers = pattern.is_some_and(|id| id.starts_with("layers@"));
        let reads_rules =
            target.is_some() || wants_layers || (!modules && scenario.is_none() && link.is_none());
        // The links followed from a place, where one is followed and the
        // graph knows it; or, before that, one link looked into. `edited`
        // gives what the chosen change did to a file, where that can be told.
        let follow = |index: &crate::graph_map::Index,
                      changed: &[(String, char)],
                      edited: &dyn Fn(&str) -> Option<crate::graph_map::Edited>,
                      rules: &std::result::Result<Option<Rules>, String>,
                      defer_paging: bool| {
            if let Some((origin, skip)) = scenario {
                return Some(index.scenario(name, origin, certain, filter, skip));
            }
            if modules {
                return Some(index.modules(name, focus, certain, filter));
            }
            if let Some((from, to)) = link {
                return Some(index.link(from, to, certain));
            }
            if let Some(place) = cycle {
                return Some(index.cycle(name, place, changed, edited, focus, certain));
            }
            if let Some(scope) = pattern.and_then(|id| id.strip_prefix("layers@")) {
                match rules {
                    Ok(Some(rules)) => {
                        return Some(index.declared_layers(name, scope, focus, certain, rules));
                    }
                    Err(_) => return None,
                    Ok(None) => {}
                }
            }
            // One found pattern; a graph that no longer has it leaves
            // what else was asked.
            if let Some(id) = pattern
                && let Some(view) = index.pattern(name, id, changed, edited, focus, certain)
            {
                return Some(view);
            }
            // The ways between two places, or what the two share, come
            // before a followed place; a place the graph does not know
            // leaves what else was asked.
            if let Some((from, to)) = path
                && let Some(view) =
                    index.path_with(name, from, to, changed, edited, focus, certain, longer)
            {
                return Some(view);
            }
            if let Some(asked) = shared
                && let Some(view) =
                    index.shared(name, asked, changed, edited, focus, certain, filter)
            {
                return Some(view);
            }
            let (follow_offset, follow_limit) = if defer_paging {
                (0, None)
            } else {
                (offset, limit)
            };
            followed.and_then(|asked| {
                index.follow_with_depth(
                    name,
                    &asked.root,
                    &asked.out,
                    &asked.into,
                    changed,
                    edited,
                    focus,
                    certain,
                    depth,
                    follow_offset,
                    follow_limit,
                )
            })
        };
        // What every answer says of the tests: whether they are left out,
        // and how many that is.
        let told = |view: &mut Value| {
            view["notests"] = json!(notests);
            if let Some((files, inside)) = left.get() {
                view["tests"] = json!({"files": files, "inside": inside});
            }
        };
        // The patterns found where a picture is open, on its boxes and
        // beside it, where they were asked for. A link looked into is no
        // picture of places, and a pattern's own picture is the pattern.
        let marked = |index: &crate::graph_map::Index,
                      view: &mut Value,
                      rules: &std::result::Result<Option<Rules>, String>| {
            if let Err(error) = rules
                && !modules
                && scenario.is_none()
                && link.is_none()
            {
                view["architecture_error"] = json!(error);
            }
            if !modules
                && scenario.is_none()
                && link.is_none()
                && view["mode"] != "pattern"
                && view["nodes"].is_array()
            {
                if patterns {
                    index.mark_patterns(view, certain);
                    match rules {
                        Ok(Some(rules)) => index.mark_declared_layers(view, Some(rules)),
                        Err(_) => index.mark_declared_layers(view, None),
                        Ok(None) => {}
                    }
                }
                if (patterns || matches!(rules, Ok(Some(_))))
                    && let Ok(rules) = rules
                {
                    index.mark_architecture(view, certain, rules.as_ref());
                }
            }
        };
        let mark_history =
            |index: &crate::graph_map::Index, view: &mut Value, revision: Option<&str>| {
                if history
                    && !modules
                    && scenario.is_none()
                    && link.is_none()
                    && !view["path"].is_object()
                    && !view["shared"].is_object()
                    && !view["pattern"].is_object()
                    && !view["cycle"].is_object()
                {
                    match self.git_history(project, &root, revision) {
                        Ok(Some(found)) => index.mark_history(view, &found),
                        Ok(None) => view["git_history"] = json!({"state": "empty"}),
                        Err(_) => view["git_history"] = json!({"state": "unavailable"}),
                    }
                }
            };
        let mark_incoming = |index: &crate::graph_map::Index, view: &mut Value| {
            if no_incoming
                && link.is_none()
                && matches!(view["mode"].as_str(), None | Some("changes" | "file"))
                && view["nodes"].is_array()
            {
                index.mark_without_incoming(view, certain, filter);
            }
        };
        let apply_lod = |index: &crate::graph_map::Index, view: &mut Value| {
            crate::graph_map::apply_lod_and_min_weight(
                index, view, lod, min_weight, scope, focus, filter, certain, offset, limit,
            );
        };
        let Some(target) = target else {
            let (raw_index, cache_hit) = self.live_index_with_hit(project, &root)?;
            let index = asked(raw_index);
            let rules = if reads_rules {
                cached_rules_read(&root)
            } else {
                Arc::new(Ok(None))
            };
            let lod_opts = crate::graph_map::LodFilter {
                scope,
                focus,
                filter,
                certain,
                offset,
                limit,
            };
            let (mut view, pre_shaped) = match follow(
                &index,
                &[],
                &|_| None,
                &rules,
                lod.is_some() || min_weight.is_some(),
            ) {
                Some(view) => (view, false),
                None => match lod.filter(|_| index.opens(scope).is_none()) {
                    Some(level) => {
                        let built = if offset == 0 && limit.is_none() {
                            crate::graph_map::view_at_lod(&index, name, level, None, &lod_opts)
                        } else {
                            index.view_at_lod_unfiltered(name, level, &lod_opts)
                        };
                        (built, true)
                    }
                    None => (
                        index.view_with_whole_options(
                            name, scope, spread, focus, certain, filter, whole, offset, limit,
                            depth,
                        ),
                        false,
                    ),
                },
            };
            marked(&index, &mut view, &rules);
            // The sources of the checkout say what the graph does not:
            // which state a file reads of another, and which it writes.
            index.mark_access(&mut view);
            // Which of the places drawn are part of a circle of files,
            // wherever the rest of the circle lies.
            index.mark_circles(&mut view);
            // What a change to the selected place reaches, a few links on.
            index.mark_blast(&mut view);
            mark_history(&index, &mut view, None);
            mark_incoming(&index, &mut view);
            let reshaped = pre_shaped || index.apply_lod_shape_only(&mut view, lod, &lod_opts);
            crate::graph_map::Index::apply_min_weight_and_pagination(
                &mut view, min_weight, focus, offset, limit, reshaped,
            );
            view["project_id"] = json!(project);
            let native_backend = built["tool"].as_str() == Some(crate::graph_ir::NATIVE_IR_TOOL)
                || index.backend() == Some(crate::graph_ir::NATIVE_IR_TOOL);
            if native_backend {
                view["built"] = json!({
                    "commit": built["commit"],
                    "at": built["at"],
                    "tool": crate::graph_ir::NATIVE_IR_TOOL,
                });
                view["message"] = json!(NATIVE_READY_MESSAGE);
            } else {
                view["built"] = json!({"commit": built["commit"], "at": built["at"]});
                view["message"] = json!("Code graph & dependency map ready.");
            }
            let backend_label = native_backend
                .then_some(crate::graph_ir::NATIVE_IR_TOOL)
                .or_else(|| index.backend());
            let ir_summary = self.maps.ir_summary_for_revision(
                &self.store.state,
                project,
                self.ir_revision_of(project),
            );
            crate::graph_map::attach_ir_metadata(&mut view, backend_label, ir_summary.as_ref());
            crate::graph_map::attach_confidence_metadata(&index, &mut view);
            apply_graph_focus(
                &self.store,
                project,
                focus_session,
                Some(&index),
                &mut view,
                false,
            )?;
            told(&mut view);
            return Ok((view, cache_hit, Some(index)));
        };
        // A run is read from its session every time: it may still be going.
        let (ran, comparison) = match target {
            Target::Run(session) => {
                let (ran, comparison) =
                    self.run_launches(project, &root, session, launch, compare_launch)?;
                (Some(ran), comparison)
            }
            Target::SysRun(run_id) => {
                let (ran, comparison) = crate::system_graph::sysrun_launches(
                    &self.store,
                    project,
                    &root,
                    run_id,
                    agent,
                    launch,
                    compare_launch,
                )?;
                (Some(ran), comparison)
            }
            _ => (None, None),
        };
        let resolved = match (target, &ran) {
            (_, Some(ran)) => ran.resolved.clone(),
            // The checkout is asked every time: it is local, and it moves.
            (Target::Work | Target::Uncommitted, _) => Arc::new(graph_changes::resolve(
                &root,
                target,
                &self.listed(project, &root),
            )?),
            _ => self.resolve_chosen_or_commit(project, &root, target)?,
        };
        if let Some(expected) = base {
            let matches_base = |side: &crate::graph_changes::Side| {
                side.commit == expected
                    || (matches!(target, Target::Range { .. })
                        && (side.name == expected
                            || (expected.len() >= 4 && side.commit.starts_with(expected))))
            };
            if cycle.is_some() || link.is_some() {
                let chosen = chosen(&resolved);
                let commit = &resolved
                    .base
                    .as_ref()
                    .filter(|base| matches_base(base))
                    .ok_or_else(|| {
                        HarnessError::conflict(
                            "The pull request base changed. Reopen the PR cycle.",
                        )
                    })?
                    .commit;
                let found = self.graph_at(project, &root, Some(commit), &resolved.prefix, true)?;
                let Some((key, graph, tree)) = found.graph else {
                    if found.state == "missing" {
                        let _ = self.revise(
                            project,
                            &root,
                            commit,
                            &resolved.prefix,
                            RevisionBuild::Normal,
                        );
                    }
                    return Err(HarnessError::unavailable(
                        if matches!(target, Target::Range { .. }) {
                            "The range base graph is not ready yet. Retry shortly once its revision build completes."
                        } else {
                            "The PR base graph is no longer cached. Return to the PR cycle to prepare it again."
                        },
                    ));
                };
                let (index, cache_hit) = self.maps.index_with_cache_hit(&key, &graph, &tree)?;
                if notests && index.has_rust_sources() && !index.has_test_spans() {
                    return Err(HarnessError::unavailable(
                        "This cached base graph cannot be shown with Without tests.",
                    ));
                }
                let index = asked(index);
                let mut view = follow(&index, &[], &|_| None, &Ok(None), false)
                    .ok_or_else(|| HarnessError::invalid("Choose a base cycle or link."))?;
                apply_lod(&index, &mut view);
                view["project_id"] = json!(project);
                view["target"] = json!(target.text());
                view["base"] = chosen["base"].clone();
                view["chosen"] = chosen;
                view["state"] = json!("ready");
                told(&mut view);
                return Ok((view, cache_hit, Some(index)));
            } else if !resolved.base.as_ref().is_some_and(matches_base) {
                return Err(HarnessError::conflict("The range base changed."));
            }
        }
        let rules = if reads_rules {
            match &resolved.revision {
                Some(revision) => Arc::new(graph_changes::architecture_at(
                    &root,
                    revision,
                    &resolved.prefix,
                )),
                None => cached_rules_read(&root),
            }
        } else {
            Arc::new(Ok(None))
        };
        let found = self.graph_of(project, &root, &resolved)?;
        let pull_cycle =
            cycle.is_some() && matches!(target, Target::Pull(_) | Target::Range { .. });
        let changed = resolved
            .files
            .iter()
            .map(|change| (change.path.clone(), change.status))
            .collect::<Vec<_>>();
        let published = live(&self.store.state, project);
        // The checkout's own graph may be older than its files, and
        // looking at its changes is a reason to bring it up to date.
        let own = resolved.revision.is_none();
        let mut stale = false;
        if own {
            self.refresh(project, Trigger::Visit)?;
            let now = self.print(project, &root, false);
            stale = now.is_some_and(|now| built["print"].as_str() != Some(now.hash.as_str()));
        }
        let (mut view, active_index, cache_hit) = match &found.graph {
            Some((kept, graph, base)) => {
                let (raw, cache_hit) = if kept == project {
                    self.live_index_with_hit(project, base)?
                } else {
                    self.maps.index_with_cache_hit(kept, graph, base)?
                };
                let index = asked(raw.clone());
                // The symbols a change touched are told by line, and the
                // lines of a graph older than the files are not the files'
                // own: then links are followed without those marks.
                let touched = |path: &str| {
                    if stale {
                        return None;
                    }
                    graph_changes::edited(&root, &resolved, path).ok().flatten()
                };
                let mut view = if let Some(mut view) = follow(
                    &index,
                    &changed,
                    &touched,
                    &rules,
                    lod.is_some() || min_weight.is_some(),
                ) {
                    if pull_cycle && view["mode"] == "cycle" {
                        let snapshot = kept.contains('@');
                        self.compare_pr_cycle(
                            project, &root, &resolved, &raw, &index, snapshot, &mut view, shown,
                        );
                    }
                    // The layers of a folder say what the change did to
                    // their order. A graph older than the files of the
                    // checkout does not hold what they do to it yet. Tests
                    // inside a Rust file are told by the spans kept beside
                    // the graph of a commit: a graph without them cannot
                    // be compared without its tests.
                    if view["pattern"]["sort"] == "layers"
                        && (view["pattern"]["declared"] != true || matches!(&*rules, Ok(Some(_))))
                    {
                        let untold = if stale {
                            Some("stale")
                        } else if notests
                            && kept.contains('@')
                            && raw.has_rust_sources()
                            && !raw.has_test_spans()
                        {
                            Some("tests")
                        } else {
                            None
                        };
                        if let Some(reason) = untold {
                            view["pattern"]["change"] = json!({
                                "state": "unavailable", "reason": reason,
                                "base": resolved.json()["base"],
                            });
                        } else {
                            let rules_ref = (*rules).as_ref().ok().and_then(Option::as_ref);
                            self.compare_layers(
                                project, &root, &resolved, &index, &mut view, shown, rules_ref,
                            );
                        }
                    }
                    view
                } else {
                    // An opened file marks the symbols the change touched.
                    // They are told by line, and the lines of a graph older
                    // than the files are not the files' own.
                    let edited = match index.opens(scope) {
                        Some(path) if !stale => {
                            graph_changes::edited(&root, &resolved, path).ok().flatten()
                        }
                        _ => None,
                    };
                    match index.symbols(
                        name,
                        scope,
                        &changed,
                        edited.as_ref(),
                        focus,
                        certain,
                        filter,
                    ) {
                        Some(view) => view,
                        None => index.changes(name, &changed, scope, focus, certain, filter),
                    }
                };
                marked(&index, &mut view, &rules);
                // Reads and writes are told from the preserved sources on disk:
                // either the fresh checkout tree or the thinned commit/PR tree.
                let access_sources_fresh = if own {
                    !stale
                } else if kept.contains('@') {
                    true
                } else {
                    self.print(project, &root, false)
                        .is_some_and(|now| built["print"].as_str() == Some(now.hash.as_str()))
                };
                if access_sources_fresh {
                    index.mark_access(&mut view);
                }
                index.mark_circles(&mut view);
                index.mark_blast(&mut view);
                mark_history(&index, &mut view, resolved.revision.as_deref());
                mark_incoming(&index, &mut view);
                if !notests && !stale {
                    index.tests_reaching_changes(&mut view, &changed, scope, certain);
                }
                // The tests are looked for in the whole graph, also where
                // the picture leaves them out.
                if !failing.is_empty() {
                    raw.suspects(&mut view, &changed, failing, certain);
                }
                // What a run opened is laid over whatever picture of its
                // change is drawn.
                if let Some(ran) = &ran
                    && !modules
                    && scenario.is_none()
                {
                    view["run"] = ran.told.clone();
                    let followup = comparison.as_ref().map(|run| {
                        let edited = run
                            .resolved
                            .files
                            .iter()
                            .map(|file| file.path.as_str())
                            .collect();
                        (run, edited)
                    });
                    index.overlay_run_with_comparison(
                        &mut view,
                        &changed,
                        (&ran.opened, &ran.shell_reads),
                        certain,
                        followup
                            .as_ref()
                            .map(|(run, edited)| ((&run.opened, &run.shell_reads), edited)),
                    );
                }
                apply_lod(&index, &mut view);
                (view, Some(index), cache_hit)
            }
            // Until the commit's own graph is there, the project's graph
            // gives a first picture: the files changed are the commit's, the
            // links are those the checkout's files have.
            None if published.is_file() => {
                let (raw_index, cache_hit) =
                    self.maps.index_with_cache_hit(project, &published, &root)?;
                let index = asked(raw_index);
                let mut view = follow(
                    &index,
                    &changed,
                    &|_| None,
                    &rules,
                    lod.is_some() || min_weight.is_some(),
                )
                .unwrap_or_else(|| index.changes(name, &changed, scope, focus, certain, filter));
                marked(&index, &mut view, &rules);
                apply_lod(&index, &mut view);
                view["preliminary"] = json!(true);
                if view["pattern"]["sort"] == "layers"
                    && (view["pattern"]["declared"] != true || matches!(&*rules, Ok(Some(_))))
                {
                    view["pattern"]["change"] = if found.state == "failed" {
                        json!({"state": "unavailable", "reason": "head"})
                    } else {
                        json!({"state": "waiting_head"})
                    };
                }
                if pull_cycle && view["mode"] == "cycle" {
                    view["cycle"]["comparison"] = if found.state == "failed" {
                        json!({"state": "unavailable", "reason": "head"})
                    } else {
                        json!({"state": "waiting_head"})
                    };
                }
                (view, Some(index), cache_hit)
            }
            None => (json!({"mode": "changes"}), None, false),
        };
        let keep_run = view["run"].is_object();
        view["project_id"] = json!(project);
        view["target"] = json!(target.text());
        let resolved_chosen = chosen(&resolved);
        if matches!(target, Target::Range { .. }) && base.is_some() {
            view["base"] = resolved_chosen["base"].clone();
        }
        view["chosen"] = resolved_chosen;
        if matches!(target, Target::Commit(_) | Target::Range { .. })
            && let Some(revision) = resolved
                .revision
                .as_deref()
                .or_else(|| resolved.head.as_ref().map(|side| side.commit.as_str()))
            && let Ok(Some(info)) = graph_changes::commit_info(&root, revision)
        {
            view["commit_info"] = info;
        }
        view["state"] = json!(found.state);
        view["build"] = found.build;
        view["failure"] = json!(found.failure);
        let native_backend = (own
            && built["tool"].as_str() == Some(crate::graph_ir::NATIVE_IR_TOOL))
            || active_index.as_ref().and_then(|idx| idx.backend())
                == Some(crate::graph_ir::NATIVE_IR_TOOL);
        let revision_tool =
            (found.state == "ready" && native_backend).then_some(crate::graph_ir::NATIVE_IR_TOOL);
        view["message"] = json!(match (found.state, revision_tool) {
            ("missing", _) => "Code graph for this revision is not built yet.".to_owned(),
            _ => status_message_for_tool(
                found.state,
                Some(&view["build"]),
                found.failure.as_deref(),
                revision_tool,
            ),
        });
        let backend_label = native_backend
            .then_some(crate::graph_ir::NATIVE_IR_TOOL)
            .or_else(|| active_index.as_ref().and_then(|idx| idx.backend()));
        let ir_summary = own
            .then(|| {
                self.maps.ir_summary_for_revision(
                    &self.store.state,
                    project,
                    self.ir_revision_of(project),
                )
            })
            .flatten();
        crate::graph_map::attach_ir_metadata(&mut view, backend_label, ir_summary.as_ref());
        if let Some(idx) = active_index.as_deref() {
            crate::graph_map::attach_confidence_metadata(idx, &mut view);
        }
        let sysrun_focus_id;
        let focus_sid = match target {
            Target::Run(session) => Some(session.as_str()),
            Target::SysRun(run_id) => {
                sysrun_focus_id = format!("sysrun:{run_id}");
                Some(sysrun_focus_id.as_str())
            }
            _ => focus_session,
        };
        apply_graph_focus(
            &self.store,
            project,
            focus_sid,
            active_index.as_deref(),
            &mut view,
            keep_run,
        )?;
        if own {
            view["stale"] = json!(stale);
            view["rebuilding"] = json!(lock(&self.jobs)?.contains_key(project));
            view["built"] = if built["tool"].as_str() == Some(crate::graph_ir::NATIVE_IR_TOOL) {
                json!({
                    "commit": built["commit"],
                    "at": built["at"],
                    "tool": crate::graph_ir::NATIVE_IR_TOOL,
                })
            } else {
                json!({"commit": built["commit"], "at": built["at"]})
            };
        }
        if let Some(ref head_idx) = active_index {
            let base_idx = resolved
                .base
                .as_ref()
                .and_then(|b| {
                    self.graph_at(project, &root, Some(&b.commit), &resolved.prefix, false)
                        .ok()
                        .and_then(|found| found.graph)
                })
                .and_then(|(k, g, t)| self.maps.index(&k, &g, &t).ok())
                .map(|b| if notests { b.without_tests() } else { b });
            let overlay_rules = match rules.as_ref() {
                Ok(opt) => opt.as_ref(),
                Err(_) => None,
            };
            let changed_paths: Vec<String> = resolved
                .files
                .iter()
                .filter(|f| f.status != 'D')
                .map(|f| f.path.clone())
                .collect();
            let renamed: std::collections::HashMap<&str, &str> = resolved
                .files
                .iter()
                .filter_map(|file| {
                    (file.status == 'R').then_some((file.path.as_str(), file.from.as_deref()?))
                })
                .collect();
            view["diff_overlay"] = self.cached_diff_overlay(
                project,
                resolved.revision.as_deref(),
                resolved.base.as_ref().map(|b| b.commit.as_str()),
                head_idx,
                base_idx.as_deref(),
                &changed_paths,
                &renamed,
                certain,
                notests,
                overlay_rules,
            );
        }
        told(&mut view);
        Ok((view, cache_hit, active_index))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn cached_diff_overlay(
        &self,
        project: &str,
        head_rev: Option<&str>,
        base_rev: Option<&str>,
        head_idx: &crate::graph_map::Index,
        base_idx: Option<&crate::graph_map::Index>,
        changed_paths: &[String],
        renamed: &std::collections::HashMap<&str, &str>,
        certain: bool,
        notests: bool,
        rules: Option<&Rules>,
    ) -> Value {
        const DIFF_OVERLAY_LRU_CAP: usize = 64;
        let (Some(head), Some(base)) = (head_rev, base_rev) else {
            return crate::graph_map::structural_diff_overlay(
                head_idx,
                base_idx,
                changed_paths,
                renamed,
                certain,
                rules,
            );
        };
        let changed_hash: [u8; 32] = {
            use sha2::Digest;
            let mut h = sha2::Sha256::new();
            let mut sorted_changed: Vec<&str> = changed_paths.iter().map(String::as_str).collect();
            sorted_changed.sort_unstable();
            for p in sorted_changed {
                h.update(p.as_bytes());
                h.update([0]);
            }
            let mut sorted_renames: Vec<(&str, &str)> =
                renamed.iter().map(|(&k, &v)| (k, v)).collect();
            sorted_renames.sort_unstable();
            h.update([1]);
            for (k, v) in sorted_renames {
                h.update(k.as_bytes());
                h.update([0]);
                h.update(v.as_bytes());
                h.update([0]);
            }
            h.finalize().into()
        };
        let key = DiffOverlayCacheKey {
            project: project.to_owned(),
            head_rev: head.to_owned(),
            base_rev: base.to_owned(),
            head_index_id: head_idx.index_id(),
            base_index_id: base_idx.map(|b| b.index_id()),
            rules_hash: rules.map(Rules::sha256),
            certain,
            notests,
            changed_hash,
        };
        if let Ok(mut cache) = self.diff_overlays.lock()
            && let Some(pos) = cache.iter().position(|(k, _)| k == &key)
        {
            let (_, val) = cache.remove(pos);
            cache.push((key, val.clone()));
            return val;
        }
        let val = crate::graph_map::structural_diff_overlay(
            head_idx,
            base_idx,
            changed_paths,
            renamed,
            certain,
            rules,
        );
        if let Ok(mut cache) = self.diff_overlays.lock() {
            if let Some(pos) = cache.iter().position(|(k, _)| k == &key) {
                cache.remove(pos);
            }
            if cache.len() >= DIFF_OVERLAY_LRU_CAP {
                cache.remove(0);
            }
            cache.push((key, val.clone()));
        }
        val
    }
    pub(super) fn git_history(
        &self,
        project: &str,
        root: &Path,
        revision: Option<&str>,
    ) -> Result<Option<Arc<graph_changes::History>>> {
        let Some(revision) = graph_changes::history_revision(root, revision)? else {
            return Ok(None);
        };
        if let Some(cached) = lock(&self.history)?.get(project).filter(|entry| {
            entry.revision == revision && entry.read_at.elapsed() < Duration::from_secs(60)
        }) {
            return Ok(Some(cached.data.clone()));
        }
        let found = Arc::new(graph_changes::history(root, &revision)?);
        lock(&self.history)?.insert(
            project.to_owned(),
            HistoryEntry {
                revision,
                read_at: Instant::now(),
                data: found.clone(),
            },
        );
        Ok(Some(found))
    }

    /// What a search finds in the graph the map of `target` is drawn from:
    /// the project's own for the whole project and for the checkout's
    /// changes, a commit's own once it is built, and the project's until
    /// then. Without tests, what is a test is not found either. Like the
    /// map, it reads a graph, so callers keep it off the runtime.
    pub fn find(
        &self,
        project: &str,
        target: Option<&Target>,
        query: &str,
        notests: bool,
    ) -> Result<Value> {
        let (root, _) = self.target_info(project)?;
        let published = live(&self.store.state, project);
        let graph = match target {
            None
            | Some(Target::Work | Target::Uncommitted | Target::Run(_) | Target::SysRun(_)) => {
                Some((project.to_owned(), published, root))
            }
            Some(target) => {
                let resolved = self.resolve_chosen_or_commit(project, &root, target)?;
                match self.graph_of(project, &root, &resolved)?.graph {
                    Some(own) => Some(own),
                    None if published.is_file() => Some((project.to_owned(), published, root)),
                    None => None,
                }
            }
        };
        let mut found = match graph {
            Some((kept, graph, base)) => {
                let index = if kept == project {
                    self.live_index_with_hit(project, &base)?.0
                } else {
                    self.maps.index(&kept, &graph, &base)?
                };
                if notests {
                    index.without_tests().find(query)
                } else {
                    index.find(query)
                }
            }
            // Neither the commit's own graph nor the project's to search.
            None => {
                json!({"mode": "find", "query": query.trim(), "found": 0, "rows": [], "missing": true})
            }
        };
        found["project_id"] = json!(project);
        found["notests"] = json!(notests);
        Ok(found)
    }

    pub(super) fn resolve_chosen_or_commit(
        &self,
        project: &str,
        root: &Path,
        target: &Target,
    ) -> Result<Arc<Resolved>> {
        let key = (project.to_owned(), target.text());
        if let Some(resolved) = self.get_chosen(&key)? {
            return Ok(resolved);
        }
        if matches!(target, Target::Commit(_) | Target::Range { .. }) {
            let resolved = Arc::new(graph_changes::resolve(root, target, &[])?);
            self.insert_chosen(key, resolved.clone())?;
            if matches!(target, Target::Range { .. })
                && let Some(commit) = &resolved.revision
                && self
                    .setting(project)
                    .ok()
                    .is_some_and(|s| s["enabled"] == true)
                && self.resolve_backend(true).is_some()
            {
                let _ = self.revise(
                    project,
                    root,
                    commit,
                    &resolved.prefix,
                    RevisionBuild::Normal,
                );
            }
            return Ok(resolved);
        }
        Err(HarnessError::conflict("Choose the change again."))
    }

    pub(super) fn load_project_focus(
        &self,
        project: &str,
        focus_session: Option<&str>,
    ) -> Result<Option<crate::graph_live::GraphFocus>> {
        if let Some(run_id) = focus_session.and_then(|s| s.strip_prefix("sysrun:"))
            && let Some(value) =
                crate::system_graph::load_system_focus_value(&self.store, run_id, project)?
        {
            return Ok(serde_json::from_value(value).ok());
        }
        let value = crate::graph_live::project_focus_value(&self.store, project, focus_session)?;
        if value.is_null() {
            return Ok(None);
        }
        Ok(serde_json::from_value(value).ok())
    }

    pub(super) fn recent_session_for_project(&self, project: &str) -> Result<Option<String>> {
        let sessions = self.store.summaries()?;
        let active_statuses = ["running", "starting", "queued", "active"];
        for pass_active in [true, false] {
            for session in &sessions {
                if session["project_id"].as_str() != Some(project) {
                    continue;
                }
                let status = session["status"].as_str().unwrap_or("");
                if pass_active && !active_statuses.contains(&status) {
                    continue;
                }
                if let Some(sid) = session["id"].as_str() {
                    return Ok(Some(sid.to_owned()));
                }
            }
        }
        Ok(None)
    }

    pub(super) fn impact_context(
        &self,
        project: &str,
        target: Option<&Target>,
        focus_session: Option<&str>,
        agent: Option<&str>,
    ) -> Result<ImpactContext> {
        let focus_session = match focus_session.map(str::trim).filter(|s| !s.is_empty()) {
            Some(sid) if sid.len() <= 1024 && !sid.chars().any(char::is_control) => Some(sid),
            Some(_) => return Err(HarnessError::invalid("Invalid code graph session.")),
            None => None,
        };
        let root = self.root(project)?;
        let live_index = || {
            self.live_index_with_hit(project, &root)
                .ok()
                .map(|(idx, _)| idx)
        };
        match target {
            Some(Target::Run(session)) => {
                let ran = self.ran(project, &root, session)?;
                let run_edited = ran
                    .resolved
                    .files
                    .iter()
                    .map(|change| (change.path.clone(), change.status))
                    .collect();
                let live_focus = self.load_project_focus(project, Some(session.as_str()))?;
                let index = live_index();
                Ok(ImpactContext {
                    root,
                    index,
                    resolved: Some(ran.resolved),
                    run_edited,
                    live_focus,
                    target_label: Target::Run(session.clone()).text(),
                })
            }
            Some(Target::SysRun(run_id)) => {
                let (ran, _) = crate::system_graph::sysrun_launches(
                    &self.store,
                    project,
                    &root,
                    run_id,
                    agent,
                    None,
                    None,
                )?;
                let run_edited = ran
                    .resolved
                    .files
                    .iter()
                    .map(|change| (change.path.clone(), change.status))
                    .collect();
                let sys_sid = format!("sysrun:{run_id}");
                let live_focus = self
                    .load_project_focus(project, Some(&sys_sid))?
                    .or(self.load_project_focus(project, focus_session)?);
                let index = live_index();
                Ok(ImpactContext {
                    root,
                    index,
                    resolved: Some(ran.resolved),
                    run_edited,
                    live_focus,
                    target_label: Target::SysRun(run_id.clone()).text(),
                })
            }
            Some(
                t @ (Target::Named(_) | Target::Commit(_) | Target::Pull(_) | Target::Range { .. }),
            ) => {
                let existing = self.get_chosen(&(project.to_owned(), t.text()))?;
                let resolved = match existing {
                    Some(res) => res,
                    None => Arc::new(graph_changes::resolve(
                        &root,
                        t,
                        &self.listed(project, &root),
                    )?),
                };
                let index = match self.graph_of(project, &root, &resolved) {
                    Ok(Found {
                        graph: Some((kept, graph, base)),
                        ..
                    }) => self
                        .maps
                        .index(&kept, &graph, &base)
                        .ok()
                        .or_else(live_index),
                    _ => live_index(),
                };
                Ok(ImpactContext {
                    root,
                    index,
                    resolved: Some(resolved),
                    run_edited: Vec::new(),
                    live_focus: None,
                    target_label: t.text(),
                })
            }
            None | Some(Target::Work | Target::Uncommitted) => {
                let is_git = graph_changes::place(&root)
                    .ok()
                    .is_some_and(|place| place["git"] == true);
                let resolved_git = if is_git {
                    let effective = target.unwrap_or(&Target::Work);
                    Some(Arc::new(graph_changes::resolve(
                        &root,
                        effective,
                        &self.listed(project, &root),
                    )?))
                } else {
                    None
                };
                let session_for_edits = if !matches!(target, Some(Target::Uncommitted)) {
                    if let Some(sid) = focus_session {
                        Some(sid.to_owned())
                    } else if !is_git {
                        self.recent_session_for_project(project)?
                    } else {
                        None
                    }
                } else {
                    None
                };
                let run_edited = session_for_edits
                    .as_deref()
                    .and_then(|sid| self.ran(project, &root, sid).ok())
                    .map(|ran| {
                        ran.resolved
                            .files
                            .iter()
                            .map(|change| (change.path.clone(), change.status))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let resolved = match (resolved_git, run_edited.is_empty()) {
                    (res, true) => res,
                    (Some(res), false) => {
                        let mut combined = (*res).clone();
                        let known: BTreeSet<String> = combined
                            .files
                            .iter()
                            .map(|file| file.path.clone())
                            .collect();
                        for (path, status) in &run_edited {
                            if !known.contains(path) {
                                combined.files.push(graph_changes::Change {
                                    path: path.clone(),
                                    status: *status,
                                    from: None,
                                });
                            }
                        }
                        Some(Arc::new(combined))
                    }
                    (None, false) => Some(Arc::new(Resolved {
                        target: Target::Run(session_for_edits.unwrap_or_default()),
                        kind: "run",
                        name: String::new(),
                        title: None,
                        pull: None,
                        head: None,
                        base: None,
                        revision: None,
                        prefix: String::new(),
                        files: run_edited
                            .iter()
                            .map(|(path, status)| graph_changes::Change {
                                path: path.clone(),
                                status: *status,
                                from: None,
                            })
                            .collect(),
                        notes: Vec::new(),
                    })),
                };
                let live_focus = self.load_project_focus(project, focus_session)?;
                let target_label = match target {
                    Some(t) => t.text(),
                    None if is_git => "work".to_owned(),
                    None => "workspace".to_owned(),
                };
                let index = live_index();
                Ok(ImpactContext {
                    root,
                    index,
                    resolved,
                    run_edited,
                    live_focus,
                    target_label,
                })
            }
        }
    }

    /// Untruncated catalog of changed files and changed entities for a project target.
    pub fn changes_catalog(
        &self,
        project: &str,
        target: Option<&Target>,
        focus_session: Option<&str>,
        notests: bool,
    ) -> Result<Value> {
        self.changes_catalog_with_agent(project, target, focus_session, None, notests)
    }

    pub fn changes_catalog_with_agent(
        &self,
        project: &str,
        target: Option<&Target>,
        focus_session: Option<&str>,
        agent: Option<&str>,
        notests: bool,
    ) -> Result<Value> {
        let ctx = self.impact_context(project, target, focus_session, agent)?;
        let ir = if ctx.resolved.as_ref().is_none_or(|r| r.revision.is_none()) {
            self.get_ir_store_readonly(project)
        } else {
            None
        };
        let mut catalog = crate::graph_impact::changed_catalog_with_ir(
            &ctx.root,
            ctx.index.as_deref(),
            ir.as_deref(),
            ctx.resolved.as_deref(),
            ctx.live_focus.as_ref(),
            &ctx.run_edited,
            &ctx.target_label,
            notests,
        );
        catalog["project_id"] = json!(project);
        catalog["notests"] = json!(notests);
        Ok(catalog)
    }

    /// Detailed dependency and reverse-dependent change-impact report for one file.
    pub fn file_impact(
        &self,
        project: &str,
        path: &str,
        target: Option<&Target>,
        focus_session: Option<&str>,
        certain: bool,
    ) -> Result<Value> {
        self.file_impact_with_agent(project, path, target, focus_session, None, certain)
    }

    pub fn file_impact_with_agent(
        &self,
        project: &str,
        path: &str,
        target: Option<&Target>,
        focus_session: Option<&str>,
        agent: Option<&str>,
        certain: bool,
    ) -> Result<Value> {
        if path.trim().is_empty() || path.len() > 1024 || path.chars().any(char::is_control) {
            return Err(HarnessError::invalid("Invalid code graph file path."));
        }
        let ctx = self.impact_context(project, target, focus_session, agent)?;
        let ir = if ctx.resolved.as_ref().is_none_or(|r| r.revision.is_none()) {
            self.get_ir_store_readonly(project)
        } else {
            None
        };
        let mut report = crate::graph_impact::file_impact_with_ir(
            &ctx.root,
            ctx.index.as_deref(),
            ir.as_deref(),
            ctx.resolved.as_deref(),
            ctx.live_focus.as_ref(),
            path,
            certain,
        )?;
        report["project_id"] = json!(project);
        Ok(report)
    }
    pub(super) fn setting(&self, project: &str) -> Result<Value> {
        Ok(self
            .store
            .setting(&key(project))?
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({"enabled": false})))
    }
    pub(super) fn edit(&self, project: &str, change: impl FnOnce(&mut Value)) -> Result<()> {
        let _writing = lock(&self.writing)?;
        let mut setting = self.setting(project)?;
        change(&mut setting);
        self.store.set_setting(&key(project), &setting)
    }
    pub(super) fn tool(&self, fresh: bool) -> Option<Tool> {
        let mut cached = self.tool.lock().ok()?;
        if !fresh
            && let Some((at, found)) = cached.as_ref()
            && at.elapsed() < TOOL_AGE
        {
            return found.clone();
        }
        let path = match &self.program {
            Some(path) => path.is_file().then(|| path.clone()),
            None => infra_host::provider::find_executable("graphify"),
        };
        let found = path.map(|path| Tool {
            version: version(&path),
            path,
        });
        *cached = Some((Instant::now(), found.clone()));
        found
    }
    pub(super) fn force_native_backend() -> bool {
        std::env::var_os("HARNESS_GRAPH_BACKEND").is_some_and(|value| {
            value
                .to_string_lossy()
                .trim()
                .eq_ignore_ascii_case("native")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_runs_marks_before_lod_min_weight_and_reads_architecture_without_patterns_flag() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join("src/api")).unwrap();
        std::fs::create_dir_all(root.join("src/core")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"mapops\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub mod api;\npub mod core;\n").unwrap();
        std::fs::write(
            root.join("src/api/mod.rs"),
            "pub fn api_entry() -> i32 { crate::core::core_leaf() }\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/core/mod.rs"),
            "pub fn core_leaf() -> i32 { 7 }\n",
        )
        .unwrap();
        std::fs::write(
            root.join(".harness-architecture.json"),
            json!({
                "version": 1,
                "layers": [
                    {"id": "api", "name": "API", "paths": ["src/api/**"], "allows": []},
                    {"id": "core", "name": "Core", "paths": ["src/core/**"], "allows": []}
                ]
            })
            .to_string(),
        )
        .unwrap();

        let store = SessionStore::open(&state).unwrap();
        let proj = store.register(&root).unwrap();
        let pid = proj["id"].as_str().unwrap().to_owned();
        let graphs = Graphs::with_native_backend(store, Duration::ZERO);
        graphs.act(&pid, "enable").unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if graphs.status(&pid, false).unwrap()["state"] == "ready" {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(graphs.status(&pid, false).unwrap()["state"], "ready");

        // 1. `.harness-architecture.json` is loaded and marked even when `patterns` is `false`.
        let plain = graphs
            .map(
                &pid,
                None,
                &Shown {
                    patterns: false,
                    ..Shown::default()
                },
            )
            .unwrap();
        assert_eq!(plain["architecture"]["declared"], true, "{plain}");
        assert!(
            plain["architecture"]["violations"]["pairs"]
                .as_u64()
                .unwrap_or(0)
                >= 1,
            "{plain}"
        );
        assert!(plain["patterns"].is_null(), "{plain}");

        // 2. Symbol LOD preserves focus blast and marks architecture_violation on symbol edges.
        let sym_overview = graphs
            .map(
                &pid,
                None,
                &Shown {
                    lod: Some(crate::graph_map::Lod::Symbol),
                    patterns: true,
                    ..Shown::default()
                },
            )
            .unwrap();
        let core_sym = sym_overview["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find_map(|n| {
                let p = n["path"].as_str()?;
                p.contains("src/core/mod.rs#").then_some(p.to_owned())
            })
            .expect("core_leaf symbol key");
        let sym_focused = graphs
            .map(
                &pid,
                None,
                &Shown {
                    lod: Some(crate::graph_map::Lod::Symbol),
                    focus: Some(&core_sym),
                    patterns: true,
                    ..Shown::default()
                },
            )
            .unwrap();
        assert_eq!(sym_focused["focus"]["path"], core_sym);
        assert!(
            sym_focused["focus"]["blast"]["files"].as_u64().unwrap_or(0) >= 1,
            "{}",
            sym_focused["focus"]
        );
        assert!(
            sym_focused["edges"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["architecture_violation"].as_u64().unwrap_or(0) >= 1),
            "{}",
            sym_focused["edges"]
        );

        // 3. `min_weight` filters AFTER `mark_architecture` so `kept: "violation"` weight-1 edges
        //    do not sneak past `min_weight = 2`.
        let weighted = graphs
            .map(
                &pid,
                None,
                &Shown {
                    lod: Some(crate::graph_map::Lod::Folder),
                    min_weight: Some(2),
                    patterns: true,
                    ..Shown::default()
                },
            )
            .unwrap();
        assert!(
            weighted["edges"]
                .as_array()
                .unwrap()
                .iter()
                .all(|e| e["links"].as_u64().unwrap_or(0) >= 2),
            "{}",
            weighted["edges"]
        );
        assert!(
            weighted["architecture"]["violations"]["pairs"]
                .as_u64()
                .unwrap_or(0)
                >= 1
        );

        graphs.close();
    }

    #[test]
    fn cached_rules_read_uses_racy_ns_stamps_and_invalidates_same_second_edits() {
        let dir = tempfile::tempdir().unwrap();
        let root = infra_core::fs::canonicalize(dir.path()).unwrap();
        let arch = root.join(crate::graph_map::layer_rules::FILE);

        let rules_v1 = json!({
            "version": 1,
            "layers": [
                {"id": "api", "name": "API", "paths": ["src/api/**"], "forbids": ["db"]},
                {"id": "db", "name": "DB", "paths": ["src/db/**"]}
            ]
        });
        std::fs::write(&arch, rules_v1.to_string()).unwrap();
        let mtime1 = rules_time_ns(
            std::fs::symlink_metadata(&arch)
                .unwrap()
                .modified()
                .unwrap(),
        )
        .unwrap();

        // Inside RULES_RACY_NS window: first call parses and stores SHA-256; identical bytes hit cache without re-parse.
        let r1 = cached_rules_read_at(&root, mtime1 + RULES_RACY_NS - 10);
        let r2 = cached_rules_read_at(&root, mtime1 + RULES_RACY_NS - 5);
        assert!(
            Arc::ptr_eq(&r1, &r2),
            "unchanged racy bytes must hit Arc cache"
        );
        assert!(matches!(&*r1, Ok(Some(_))));

        // Settled past RULES_RACY_NS: hits purely on (len, mtime_ns) stat without reading/parsing.
        let r3 = cached_rules_read_at(&root, mtime1 + RULES_RACY_NS + 100);
        assert!(
            Arc::ptr_eq(&r1, &r3),
            "settled file must reuse cached Rules Arc"
        );

        // Same-length rewrite inside RACY_NS invalidates via content SHA-256.
        let rules_v2 = json!({
            "version": 1,
            "layers": [
                {"id": "web", "name": "WEB", "paths": ["src/web/**"], "forbids": ["db"]},
                {"id": "db", "name": "DB", "paths": ["src/db/**"]}
            ]
        });
        assert_eq!(rules_v1.to_string().len(), rules_v2.to_string().len());
        std::fs::write(&arch, rules_v2.to_string()).unwrap();
        let mtime2 = rules_time_ns(
            std::fs::symlink_metadata(&arch)
                .unwrap()
                .modified()
                .unwrap(),
        )
        .unwrap();
        let r4 = cached_rules_read_at(&root, mtime2 + 1);
        assert!(
            !Arc::ptr_eq(&r1, &r4),
            "modified content within RACY_NS must re-parse"
        );
        let sha1 = r1.as_ref().as_ref().unwrap().as_ref().unwrap().sha256();
        let sha4 = r4.as_ref().as_ref().unwrap().as_ref().unwrap().sha256();
        assert_ne!(sha1, sha4);

        // Removing the file returns Ok(None).
        std::fs::remove_file(&arch).unwrap();
        let r5 = cached_rules_read_at(&root, mtime2 + RULES_RACY_NS + 100);
        assert!(matches!(&*r5, Ok(None)));
    }
}
