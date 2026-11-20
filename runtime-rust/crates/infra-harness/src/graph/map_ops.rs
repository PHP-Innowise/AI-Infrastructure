use super::*;

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
        let (mut view, cache_hit, active_index) = self.map_inner(project, target, shown)?;
        if let Value::Object(obj) = &mut view {
            if !obj.contains_key("ir_revision") {
                obj.insert("ir_revision".into(), json!(self.ir_revision_of(project)));
            }
            if !obj.contains_key("depth") {
                obj.insert("depth".into(), json!(shown.depth));
            }
        }
        if let Some(ref idx) = active_index {
            crate::graph_map::apply_lod_and_min_weight(
                idx,
                &mut view,
                shown.lod,
                shown.min_weight,
                shown.scope,
                shown.focus,
                shown.filter,
                shown.certain,
                shown.offset > 0 || shown.limit.is_some(),
            );
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
            lod: _,
            min_weight: _,
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
            wants_layers || (patterns && !modules && scenario.is_none() && link.is_none());
        // The links followed from a place, where one is followed and the
        // graph knows it; or, before that, one link looked into. `edited`
        // gives what the chosen change did to a file, where that can be told.
        let follow = |index: &crate::graph_map::Index,
                      changed: &[(String, char)],
                      edited: &dyn Fn(&str) -> Option<crate::graph_map::Edited>,
                      rules: &std::result::Result<Option<Rules>, String>| {
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
                    offset,
                    limit,
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
            if let Err(error) = rules {
                view["architecture_error"] = json!(error);
            }
            if patterns
                && !modules
                && scenario.is_none()
                && link.is_none()
                && view["mode"] != "pattern"
                && view["nodes"].is_array()
            {
                index.mark_patterns(view, certain);
                match rules {
                    Ok(Some(rules)) => index.mark_declared_layers(view, Some(rules)),
                    Err(_) => index.mark_declared_layers(view, None),
                    Ok(None) => {}
                }
                if let Ok(rules) = rules {
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
        let Some(target) = target else {
            let (raw_index, cache_hit) = self.live_index_with_hit(project, &root)?;
            let index = asked(raw_index);
            let rules = if reads_rules {
                Rules::read(&root)
            } else {
                Ok(None)
            };
            let mut view = follow(&index, &[], &|_| None, &rules).unwrap_or_else(|| {
                index.view_with_whole_options(
                    name, scope, spread, focus, certain, filter, whole, offset, limit, depth,
                )
            });
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
                let mut view = follow(&index, &[], &|_| None, &Ok(None))
                    .ok_or_else(|| HarnessError::invalid("Choose a base cycle or link."))?;
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
                Some(revision) => graph_changes::architecture_at(&root, revision, &resolved.prefix),
                None => Rules::read(&root),
            }
        } else {
            Ok(None)
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
                let mut view = if let Some(mut view) = follow(&index, &changed, &touched, &rules) {
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
                        && (view["pattern"]["declared"] != true
                            || rules.as_ref().is_ok_and(Option::is_some))
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
                            let rules_ref = rules.as_ref().ok().and_then(|r| r.as_ref());
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
                (view, Some(index), cache_hit)
            }
            // Until the commit's own graph is there, the project's graph
            // gives a first picture: the files changed are the commit's, the
            // links are those the checkout's files have.
            None if published.is_file() => {
                let (raw_index, cache_hit) =
                    self.maps.index_with_cache_hit(project, &published, &root)?;
                let index = asked(raw_index);
                let mut view = follow(&index, &changed, &|_| None, &rules).unwrap_or_else(|| {
                    index.changes(name, &changed, scope, focus, certain, filter)
                });
                marked(&index, &mut view, &rules);
                view["preliminary"] = json!(true);
                if view["pattern"]["sort"] == "layers"
                    && (view["pattern"]["declared"] != true
                        || rules.as_ref().is_ok_and(Option::is_some))
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
            let extra_rules: Option<Rules> = if reads_rules {
                None
            } else {
                match &resolved.revision {
                    Some(rev) => graph_changes::architecture_at(&root, rev, &resolved.prefix)
                        .ok()
                        .flatten(),
                    None => Rules::read(&root).ok().flatten(),
                }
            };
            let overlay_rules = if reads_rules {
                rules.as_ref().ok().and_then(|r| r.as_ref())
            } else {
                extra_rules.as_ref()
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
            view["diff_overlay"] = crate::graph_map::structural_diff_overlay(
                head_idx,
                base_idx.as_deref(),
                &changed_paths,
                &renamed,
                certain,
                overlay_rules,
            );
        }
        told(&mut view);
        Ok((view, cache_hit, active_index))
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
