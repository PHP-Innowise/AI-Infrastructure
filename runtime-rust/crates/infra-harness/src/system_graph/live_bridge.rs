use super::helpers::{
    augment_summaries_with_intra_callers, build_run_scoped_summaries,
    collect_opened_by_service_for_run, is_runtime_artifact_path, normalize_cross_unopened_entry,
};
use super::resolver::{
    ResolvedServiceTarget, SYSTEM_FOCUS_PREFIX, registered_project_roots, resolve_all_targets,
    resolve_targets_from_system_file, targets_from_loaded_system,
};
use crate::{
    Result,
    graph::{Graphs, Trigger, service_offer, shell_reads},
    graph_changes::Target,
    graph_live::{CreatedEntity, GraphFocus, LiveFocusTracker, predict_targets},
    graph_map,
    run_activity::{Enricher, redact_command, relative},
    store::SessionStore,
};
use infra_host::activity::normalize;
use infra_system::{
    CodeGraphBridge, CrossServiceImpactQuery, DispatchEvidence, DispatchGraphContext,
    EvidenceFileChange, ServiceGraphOffer, SystemGraph, catalog::System, git_changes_since,
    git_worktree_changes, summarize_run_evidence,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex, Weak},
};

// ---------------------------------------------------------------------------
// Global Weak Registry for Live In-Progress Dispatches
// ---------------------------------------------------------------------------

static ACTIVE_BRIDGES: Mutex<Vec<Weak<HarnessCodeGraphBridge>>> = Mutex::new(Vec::new());

fn register_global_bridge(bridge: &Arc<HarnessCodeGraphBridge>) {
    if let Ok(mut list) = ACTIVE_BRIDGES.lock() {
        list.retain(|w| w.strong_count() > 0);
        list.push(Arc::downgrade(bridge));
    }
}

pub(super) fn active_bridge_dispatches(run_id: &str) -> Option<Vec<DispatchEvidence>> {
    let bridges: Vec<Arc<HarnessCodeGraphBridge>> = ACTIVE_BRIDGES
        .lock()
        .ok()?
        .iter()
        .filter_map(Weak::upgrade)
        .collect();
    let mut out = Vec::new();
    for bridge in bridges {
        out.extend(bridge.snapshot_active_dispatches(run_id));
    }
    (!out.is_empty()).then_some(out)
}

// ---------------------------------------------------------------------------
// `HarnessCodeGraphBridge` (`impl infra_system::CodeGraphBridge`)
// ---------------------------------------------------------------------------

struct ActiveDispatch {
    ctx: DispatchGraphContext,
    enricher: Enricher,
    service_trackers: BTreeMap<String, LiveFocusTracker>,
    service_roots: BTreeMap<String, PathBuf>,
    shell_opened: BTreeSet<String>,
    shell_reads: BTreeMap<String, i64>,
    shell_events: Vec<Value>,
    file_touches: Vec<Value>,
    plan_items: Vec<Value>,
    command_runs: Vec<Value>,
    pending_commands: BTreeMap<String, String>,
    pending_paths: BTreeMap<String, (String, String)>,
    fallback_event_seq: i64,
    graph_asked: bool,
}

impl ActiveDispatch {
    fn snapshot_evidence(&self, git_changed: Vec<EvidenceFileChange>) -> DispatchEvidence {
        let (raw_opened, raw_edited) = self.enricher.ledger.files();
        let mut opened: BTreeSet<String> = raw_opened
            .into_iter()
            .map(|p| p.replace('\\', "/").trim_start_matches("./").to_owned())
            .filter(|p| !p.is_empty() && !p.starts_with("outside:"))
            .collect();
        for p in &self.shell_opened {
            opened.insert(p.clone());
        }
        for k in self.shell_reads.keys() {
            opened.insert(k.clone());
        }
        if self.ctx.service == "__system__" {
            for (sid, tracker) in &self.service_trackers {
                if sid != "__system__" {
                    for p in &tracker.focus().target_paths {
                        opened.insert(format!("{sid}/{p}"));
                    }
                }
            }
        }
        let ledger_edited = raw_edited
            .into_iter()
            .filter(|(p, _)| !p.starts_with("outside:"))
            .map(|(p, st)| EvidenceFileChange {
                path: p.replace('\\', "/").trim_start_matches("./").to_owned(),
                status: st.to_string(),
                from: None,
            })
            .collect();
        let receipt = self.enricher.ledger.receipt();
        let primary_focus = self.enricher.ledger.graph_focus();
        let mut created_entities: Vec<Value> = primary_focus
            .created_entities
            .iter()
            .filter_map(|e| serde_json::to_value(e).ok())
            .collect();
        for (sid, tracker) in &self.service_trackers {
            if *sid != self.ctx.service {
                for entity in &tracker.focus().created_entities {
                    if let Ok(mut val) = serde_json::to_value(entity) {
                        val["service"] = json!(sid);
                        if !created_entities.contains(&val) {
                            created_entities.push(val);
                        }
                    }
                }
            }
        }

        DispatchEvidence {
            phase: self.ctx.phase.clone(),
            attempt: self.ctx.attempt,
            launch_id: self.ctx.launch_id.clone(),
            service: self.ctx.service.clone(),
            root: self.ctx.root.to_string_lossy().into_owned(),
            base_head: self.ctx.base_head.clone(),
            head_after: self.ctx.head_after.clone(),
            git_changed,
            ledger_edited,
            opened: opened.into_iter().collect(),
            shell_reads: self.shell_reads.clone(),
            shell_events: self.shell_events.clone(),
            graph_asked: self.graph_asked || receipt["graph"].as_u64().unwrap_or(0) > 0,
            steps: receipt["steps"].as_u64().unwrap_or(0) as usize,
            searched: receipt["searched"].as_u64().unwrap_or(0) as usize,
            commands: receipt["commands"].as_u64().unwrap_or(0) as usize,
            helpers: receipt["helpers"].as_u64().unwrap_or(0) as usize,
            limited: receipt["limited"] == true,
            created_entities,
            target_folders: primary_focus.target_folders.clone(),
            target_paths: primary_focus.target_paths.clone(),
            target_nodes: primary_focus.target_nodes.clone(),
            file_touches: self.file_touches.clone(),
            plan_items: self.plan_items.clone(),
            command_runs: self.command_runs.clone(),
        }
    }
}

#[derive(Default)]
struct BridgeState {
    sessions: BTreeMap<String, (String, String)>,
    dispatches: BTreeMap<(String, String, u64), ActiveDispatch>,
    /// How many events of each run passed the bridge: what a run's
    /// evidence reaches is worked out anew once another one did.
    events: BTreeMap<String, u64>,
    /// Monotonic counter bumped whenever any active dispatch for `run_id`
    /// starts, mutates evidence (`opened`, `shell_reads`, entities, plan, commands),
    /// or finishes.
    run_epochs: BTreeMap<String, u64>,
}

impl BridgeState {
    fn bump_run_epoch(&mut self, run_id: &str) {
        let entry = self.run_epochs.entry(run_id.to_owned()).or_insert(0);
        *entry = entry.saturating_add(1);
    }
}

pub struct HarnessCodeGraphBridge {
    store: SessionStore,
    graphs: Arc<Graphs>,
    state: Mutex<BridgeState>,
    /// What the evidence of each run was last found to reach.
    reach: Mutex<BTreeMap<String, Reach>>,
    /// How often that was worked out, not taken from what was kept.
    pub(super) reached: std::sync::atomic::AtomicU64,
}

impl HarnessCodeGraphBridge {
    pub fn new(store: SessionStore, graphs: Arc<Graphs>) -> Arc<Self> {
        let me = Arc::new(Self {
            store,
            graphs,
            state: Mutex::new(BridgeState::default()),
            reach: Mutex::new(BTreeMap::new()),
            reached: std::sync::atomic::AtomicU64::new(0),
        });
        register_global_bridge(&me);
        me
    }

    pub fn register_session(&self, run_id: &str, sid: &str, generation: &str) {
        if let Ok(mut guard) = self.state.lock() {
            guard
                .sessions
                .insert(run_id.to_owned(), (sid.to_owned(), generation.to_owned()));
        }
    }

    pub fn unregister_session(&self, run_id: &str) {
        if let Ok(mut guard) = self.state.lock() {
            guard.sessions.remove(run_id);
        }
    }

    /// Receives every emitted session event (along with its inserted SQLite `events.id`) from
    /// `SystemWorkflow`'s `ActivityStream::new_with_ids` so `shell_reads` stores exact SQLite row IDs.
    pub fn on_emitted_session_event(&self, run_id: &str, event_id: Option<u64>, event: &Value) {
        if let Ok(mut guard) = self.state.lock() {
            *guard.events.entry(run_id.to_owned()).or_default() += 1;
        }
        let Some(eid) = event_id else {
            return;
        };
        if event["kind"] != "agent_activity"
            || event["type"] != "tool"
            || event["state"] != "completed"
        {
            return;
        }
        let Some(detail) = event["detail"].as_str().filter(|s| !s.is_empty()) else {
            return;
        };
        let phase_opt = event["phase_id"]
            .as_str()
            .or_else(|| event["agent"].as_str());
        let attempt_opt = event["attempt"].as_u64();

        if let Ok(mut guard) = self.state.lock() {
            let mut mutated = false;
            for ((rid, phase, attempt), active) in guard.dispatches.iter_mut() {
                if rid != run_id {
                    continue;
                }
                if let Some(p) = phase_opt
                    && p != phase
                {
                    continue;
                }
                if let Some(a) = attempt_opt
                    && a != *attempt
                {
                    continue;
                }
                if let Some((_, files)) =
                    shell_reads::completed_command(eid, detail, &active.ctx.root)
                {
                    for file in files {
                        active.shell_reads.insert(file.clone(), eid as i64);
                        active.shell_events.push(json!({
                            "id": eid,
                            "path": file,
                            "detail": detail,
                        }));
                        mutated = true;
                    }
                }
                if active.ctx.service == "__system__" {
                    for (sid, sroot) in &active.service_roots {
                        if sid == "__system__" {
                            continue;
                        }
                        if let Some((_, files)) = shell_reads::completed_command(eid, detail, sroot)
                        {
                            for file in files {
                                let qual = format!("{sid}/{file}");
                                active.shell_reads.insert(qual.clone(), eid as i64);
                                active.shell_events.push(json!({
                                    "id": eid,
                                    "path": qual,
                                    "detail": detail,
                                }));
                                mutated = true;
                            }
                        }
                    }
                }
            }
            if mutated {
                guard.bump_run_epoch(run_id);
            }
        }
    }

    pub(super) fn run_dispatch_snapshot_key(&self, run_id: &str) -> (u64, usize) {
        let Ok(guard) = self.state.lock() else {
            return (0, 0);
        };
        let epoch = guard.run_epochs.get(run_id).copied().unwrap_or(0);
        let active = guard
            .dispatches
            .keys()
            .filter(|(rid, _, _)| rid == run_id)
            .count();
        (epoch, active)
    }

    pub(super) fn snapshot_active_dispatches(&self, run_id: &str) -> Vec<DispatchEvidence> {
        let Ok(guard) = self.state.lock() else {
            return Vec::new();
        };
        guard
            .dispatches
            .iter()
            .filter(|((rid, _, _), _)| rid == run_id)
            .map(|(_, d)| d.snapshot_evidence(Vec::new()))
            .collect()
    }

    fn emit_focus_event(
        &self,
        run_id: &str,
        session_override: Option<&str>,
        service: &str,
        phase_id: &str,
        attempt: u64,
        focus: &GraphFocus,
    ) {
        let _ = self.persist_service_focus(run_id, service, focus);
        let session_pair = self
            .state
            .lock()
            .ok()
            .and_then(|g| g.sessions.get(run_id).cloned());
        let event = json!({
            "kind": "graph_focus",
            "run_id": run_id,
            "service": service,
            "phase_id": phase_id,
            "attempt": attempt,
            "phase": focus.phase,
            "summary": focus.summary,
            "target_folders": focus.target_folders,
            "target_paths": focus.target_paths,
            "target_nodes": focus.target_nodes,
            "created_entities": focus.created_entities,
            "focus": focus,
        });
        if let Some((sid, generation)) = session_pair {
            let _ = self.store.guarded_event(&sid, &generation, &event);
        } else if let Some(sid) = session_override.filter(|s| !s.is_empty())
            && let Ok(sess) = self.store.get(sid)
            && let Some(generation) = sess["generation"].as_str()
        {
            let _ = self.store.guarded_event(sid, generation, &event);
        }
    }

    fn persist_service_focus(&self, run_id: &str, service: &str, focus: &GraphFocus) -> Result<()> {
        let key = format!("{SYSTEM_FOCUS_PREFIX}{run_id}.{service}");
        let val = serde_json::to_value(focus)?;
        self.store.set_setting(&key, &val)?;

        let agg_key = format!("{SYSTEM_FOCUS_PREFIX}{run_id}.__aggregate__");
        let mut agg: GraphFocus = self
            .store
            .setting(&agg_key)?
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        agg.phase = focus.phase.clone();
        if !focus.summary.is_empty() {
            agg.summary = focus.summary.clone();
        }
        let prefix = if service == "__system__" {
            String::new()
        } else {
            format!("{service}/")
        };
        for f in &focus.target_folders {
            let q = if prefix.is_empty() {
                f.clone()
            } else {
                format!("{prefix}{f}")
            };
            if !agg.target_folders.contains(&q) && agg.target_folders.len() < 24 {
                agg.target_folders.push(q);
            }
        }
        for p in &focus.target_paths {
            let q = if prefix.is_empty() {
                p.clone()
            } else {
                format!("{prefix}{p}")
            };
            if !agg.target_paths.contains(&q) && agg.target_paths.len() < 48 {
                agg.target_paths.push(q);
            }
        }
        for n in &focus.target_nodes {
            if !agg.target_nodes.contains(n) && agg.target_nodes.len() < 48 {
                agg.target_nodes.push(n.clone());
            }
        }
        for ent in &focus.created_entities {
            let mut copy: CreatedEntity = ent.clone();
            if !prefix.is_empty() && !copy.path.starts_with(&prefix) {
                copy.path = format!("{prefix}{}", copy.path);
            }
            if !agg
                .created_entities
                .iter()
                .any(|e| e.id == copy.id && e.path == copy.path)
                && agg.created_entities.len() < 64
            {
                agg.created_entities.push(copy);
            }
        }
        self.store
            .set_setting(&agg_key, &serde_json::to_value(&agg)?)?;
        Ok(())
    }
}

fn prompt_seed_from_ctx(ctx: &DispatchGraphContext) -> String {
    let mut parts = Vec::new();
    if let Some(goal) = ctx.state["goal"]
        .as_str()
        .or_else(|| ctx.plan["goal"].as_str())
        .filter(|s| !s.is_empty())
    {
        parts.push(goal.to_owned());
    }
    if let Some(summary) = ctx.plan["summary"].as_str().filter(|s| !s.is_empty()) {
        parts.push(summary.to_owned());
    }
    if let Some(steps) = ctx.plan["steps"].as_array() {
        for step in steps {
            if step["service"].as_str() == Some(ctx.service.as_str())
                && let Some(text) = step["task"]
                    .as_str()
                    .or_else(|| step["summary"].as_str())
                    .filter(|s| !s.is_empty())
            {
                parts.push(text.to_owned());
            }
        }
    }
    parts.join("\n")
}

impl CodeGraphBridge for HarnessCodeGraphBridge {
    fn on_run_start(
        &self,
        _run_id: &str,
        state: &Value,
        system: &System,
    ) -> infra_core::Result<()> {
        if let Ok((by_root, _)) = registered_project_roots(&self.store) {
            let selected: Vec<String> = state["selected"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            for target in
                targets_from_loaded_system(Some(&self.graphs), &by_root, "", "system.json", system)
            {
                if target.service_id == "__system__"
                    || selected.is_empty()
                    || selected.contains(&target.service_id)
                {
                    let _ = self.graphs.refresh(&target.graph_id, Trigger::Launch);
                }
            }
        }
        Ok(())
    }

    fn offers_for_dispatch(
        &self,
        ctx: &DispatchGraphContext,
    ) -> infra_core::Result<Vec<ServiceGraphOffer>> {
        if ctx.tell_agents_graph == Some(false) {
            return Ok(Vec::new());
        }
        let Some((_, targets)) =
            resolve_targets_from_system_file(&self.store, Some(&self.graphs), &ctx.system_file)
        else {
            return Ok(Vec::new());
        };
        let force_offer = ctx.tell_agents_graph == Some(true);
        let mut wanted = BTreeSet::new();
        wanted.insert(ctx.service.clone());
        if ctx.phase == "contracts" || ctx.phase == "verify" || ctx.access == "all" {
            wanted.insert("__system__".to_owned());
            for sid in &ctx.selected {
                wanted.insert(sid.clone());
            }
        }
        for sid in &ctx.readable {
            wanted.insert(sid.clone());
        }

        let mut offers = Vec::new();
        for target in targets {
            if !wanted.contains(&target.service_id) {
                continue;
            }
            let Ok(Some(data)) = service_offer(
                &self.store,
                &target.graph_id,
                &target.service_id,
                force_offer,
            ) else {
                continue;
            };
            let edges = self
                .graphs
                .status(&target.graph_id, false)
                .ok()
                .and_then(|st| st["graph"]["edges"].as_u64())
                .unwrap_or(0) as usize;
            offers.push(ServiceGraphOffer {
                service: target.service_id,
                command: data.program_path.to_string_lossy().into_owned(),
                graph_path: data.graph_path,
                nodes: data.nodes.unwrap_or(0) as usize,
                edges,
                commit: data.commit,
                readable_paths: data.readable_paths,
                claude_allow_rules: data.claude_allow_rules,
                env: data.environment.into_iter().collect(),
                prompt_instructions: Some(data.prompt_block),
            });
        }
        Ok(offers)
    }

    fn on_dispatch_start(&self, ctx: &DispatchGraphContext) -> infra_core::Result<()> {
        let mut service_roots = ctx.roots.clone();
        service_roots
            .entry(ctx.service.clone())
            .or_insert_with(|| ctx.root.clone());

        let prompt_seed = prompt_seed_from_ctx(ctx);
        let mut service_trackers = BTreeMap::new();
        if let Some((_, targets)) =
            resolve_targets_from_system_file(&self.store, Some(&self.graphs), &ctx.system_file)
        {
            for target in &targets {
                service_roots
                    .entry(target.service_id.clone())
                    .or_insert_with(|| target.root.clone());
                if target.service_id == ctx.service
                    || ctx.phase == "contracts"
                    || ctx.phase == "verify"
                {
                    let _ = self.graphs.refresh(&target.graph_id, Trigger::Launch);
                }
                if target.service_id == ctx.service
                    || (ctx.phase == "verify" && ctx.selected.contains(&target.service_id))
                {
                    let idx = self.graphs.live_index(&target.graph_id);
                    let predicted =
                        predict_targets(&target.root, idx.as_deref(), &prompt_seed, &[], &[]);
                    let mut tracker = LiveFocusTracker::new(target.root.clone());
                    tracker.seed_focus(predicted);
                    self.emit_focus_event(
                        &ctx.run_id,
                        ctx.launch_id.as_deref(),
                        &target.service_id,
                        &ctx.phase,
                        ctx.attempt,
                        tracker.focus(),
                    );
                    service_trackers.insert(target.service_id.clone(), tracker);
                } else {
                    service_trackers.insert(
                        target.service_id.clone(),
                        LiveFocusTracker::new(target.root.clone()),
                    );
                }
            }
        }

        let ws_str = ctx.root.to_string_lossy().into_owned();
        let home = crate::run_activity::user_home();
        let mut enricher = Enricher::new(
            &ctx.provider,
            &ws_str,
            None,
            &[],
            home.as_deref(),
            None,
            false,
        );
        if let Some(primary_tracker) = service_trackers.get(&ctx.service) {
            enricher
                .ledger
                .seed_graph_focus(primary_tracker.focus().clone());
        } else {
            let predicted = predict_targets(&ctx.root, None, &prompt_seed, &[], &[]);
            enricher.ledger.seed_graph_focus(predicted.clone());
            self.emit_focus_event(
                &ctx.run_id,
                ctx.launch_id.as_deref(),
                &ctx.service,
                &ctx.phase,
                ctx.attempt,
                &predicted,
            );
        }

        if let Ok(mut guard) = self.state.lock() {
            guard.dispatches.insert(
                (ctx.run_id.clone(), ctx.phase.clone(), ctx.attempt),
                ActiveDispatch {
                    ctx: ctx.clone(),
                    enricher,
                    service_trackers,
                    service_roots,
                    shell_opened: BTreeSet::new(),
                    shell_reads: BTreeMap::new(),
                    shell_events: Vec::new(),
                    file_touches: Vec::new(),
                    plan_items: Vec::new(),
                    command_runs: Vec::new(),
                    pending_commands: BTreeMap::new(),
                    pending_paths: BTreeMap::new(),
                    fallback_event_seq: 1,
                    graph_asked: false,
                },
            );
            guard.bump_run_epoch(&ctx.run_id);
        }
        Ok(())
    }

    fn on_dispatch_signal(
        &self,
        ctx: &DispatchGraphContext,
        raw_line: &[u8],
    ) -> infra_core::Result<()> {
        let Ok(parsed) = crate::strict_json::parse(raw_line, 2 * 1024 * 1024) else {
            return Ok(());
        };
        let items = normalize(&ctx.provider, &parsed, true);
        if items.is_empty() {
            return Ok(());
        }
        let raw_codex_cmd = parsed["item"]["command"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let mut claude_raw_cmds: BTreeMap<String, String> = BTreeMap::new();
        if let Some(blocks) = parsed["message"]["content"].as_array() {
            for b in blocks {
                if b["type"] == "tool_use"
                    && let (Some(cid), Some(cmd)) =
                        (b["id"].as_str(), b["input"]["command"].as_str())
                {
                    claude_raw_cmds.insert(cid.to_owned(), cmd.to_owned());
                }
            }
        }

        let mut focus_emissions: Vec<(String, GraphFocus)> = Vec::new();
        if let Ok(mut guard) = self.state.lock() {
            let has_live_session =
                guard.sessions.contains_key(&ctx.run_id) || ctx.launch_id.is_some();
            let key = (ctx.run_id.clone(), ctx.phase.clone(), ctx.attempt);
            let Some(active) = guard.dispatches.get_mut(&key) else {
                return Ok(());
            };
            for item in items {
                let kind = item["kind"]
                    .as_str()
                    .or_else(|| item["type"].as_str())
                    .unwrap_or("");
                if kind == "plan" {
                    let mut plan_ev = item.clone();
                    plan_ev["kind"] = json!("plan");
                    if let Some(cleaned) = active.enricher.plan(&plan_ev) {
                        active.plan_items.push(json!({
                            "phase": ctx.phase,
                            "service": ctx.service,
                            "attempt": ctx.attempt,
                            "text": cleaned["text"],
                            "items": cleaned["items"],
                        }));
                    }
                    continue;
                }
                if kind != "tool" {
                    continue;
                }

                let raw_targets = item.get("targets").cloned().unwrap_or(Value::Null);
                let mut clean = item;
                clean["kind"] = json!("tool");
                let display = active.enricher.take(&mut clean, 0, 0);
                if let Some(fields) = display.as_ref().and_then(Value::as_object) {
                    for (k, v) in fields {
                        clean[k] = v.clone();
                    }
                }

                let state_str = clean["state"]
                    .as_str()
                    .or_else(|| raw_targets["state"].as_str())
                    .unwrap_or("started");
                let call_id = raw_targets["call"]
                    .as_str()
                    .or_else(|| clean["call"].as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                let mut tool_name = clean["tool"]
                    .as_str()
                    .or_else(|| raw_targets["tool"].as_str())
                    .unwrap_or("")
                    .to_owned();
                if state_str == "started"
                    && let Some(ref cid) = call_id
                    && let Some(raw_p) = raw_targets["path"].as_str()
                {
                    active
                        .pending_paths
                        .insert(cid.clone(), (tool_name.clone(), raw_p.to_owned()));
                }
                let completed_pending_path = if state_str == "completed" {
                    call_id
                        .as_ref()
                        .and_then(|cid| active.pending_paths.remove(cid))
                } else {
                    None
                };
                if tool_name.is_empty()
                    && let Some((ref saved_tool, _)) = completed_pending_path
                {
                    tool_name = saved_tool.clone();
                }

                if let Some(path_str) = clean["path"].as_str()
                    && active.file_touches.len() < 200
                {
                    active.file_touches.push(json!({
                        "phase": ctx.phase,
                        "service": ctx.service,
                        "attempt": ctx.attempt,
                        "tool": tool_name,
                        "state": state_str,
                        "path": path_str,
                        "outside": clean["outside"] == true,
                    }));
                }
                if let Some(changes) = clean["changes"].as_array() {
                    for ch in changes {
                        if let Some(path_str) = ch["path"].as_str()
                            && active.file_touches.len() < 200
                        {
                            active.file_touches.push(json!({
                                "phase": ctx.phase,
                                "service": ctx.service,
                                "attempt": ctx.attempt,
                                "tool": tool_name,
                                "state": state_str,
                                "path": path_str,
                                "kind": ch["kind"],
                                "outside": ch["outside"] == true,
                            }));
                        }
                    }
                }

                let explicit_cmd = call_id
                    .as_ref()
                    .and_then(|cid| claude_raw_cmds.get(cid).map(String::as_str))
                    .or(raw_codex_cmd.as_deref())
                    .or_else(|| {
                        if matches!(
                            tool_name.to_ascii_lowercase().as_str(),
                            "bash" | "shell" | "command" | "exec_command"
                        ) {
                            raw_targets["detail"]
                                .as_str()
                                .or_else(|| clean["detail"].as_str())
                        } else {
                            None
                        }
                    });
                if let Some(cmd_str) = explicit_cmd {
                    if crate::graph::asked(cmd_str) {
                        active.graph_asked = true;
                    }
                    if state_str == "started"
                        && let Some(ref cid) = call_id
                    {
                        active
                            .pending_commands
                            .insert(cid.clone(), cmd_str.to_owned());
                    }
                }
                let completed_cmd = if state_str == "completed" {
                    let popped = call_id
                        .as_ref()
                        .and_then(|cid| active.pending_commands.remove(cid));
                    explicit_cmd.map(str::to_owned).or(popped)
                } else {
                    None
                };

                if let Some(detail_raw) = completed_cmd.as_deref().filter(|s| !s.is_empty()) {
                    if crate::graph::asked(detail_raw) {
                        active.graph_asked = true;
                    }
                    if active.command_runs.len() < 200 {
                        active.command_runs.push(json!({
                            "phase": ctx.phase,
                            "service": ctx.service,
                            "attempt": ctx.attempt,
                            "command": redact_command(&detail_raw.chars().take(300).collect::<String>()),
                            "ok": clean["ok"] != false && raw_targets["ok"] != false,
                            "exit_code": clean["exit_code"],
                        }));
                    }
                    if clean["ok"] != false && raw_targets["ok"] != false {
                        let seq_id = active.fallback_event_seq;
                        active.fallback_event_seq = active.fallback_event_seq.saturating_add(1);
                        if let Some((_, files)) = shell_reads::completed_command(
                            seq_id as u64,
                            detail_raw,
                            &active.ctx.root,
                        ) {
                            for file in files {
                                active.shell_opened.insert(file.clone());
                                if !has_live_session {
                                    active.shell_reads.entry(file).or_insert(seq_id);
                                }
                            }
                        }
                        if active.ctx.service == "__system__" {
                            for (sid, sroot) in &active.service_roots {
                                if sid == "__system__" {
                                    continue;
                                }
                                if let Some((_, files)) =
                                    shell_reads::completed_command(seq_id as u64, detail_raw, sroot)
                                {
                                    for file in files {
                                        let qual = format!("{sid}/{file}");
                                        active.shell_opened.insert(qual.clone());
                                        if !has_live_session {
                                            active.shell_reads.entry(qual).or_insert(seq_id);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if !active.enricher.take_focus_events().is_empty() {
                    focus_emissions.push((
                        ctx.service.clone(),
                        active.enricher.ledger.graph_focus().clone(),
                    ));
                }

                if raw_targets.is_object() {
                    let effective_raw_path = raw_targets["path"]
                        .as_str()
                        .or_else(|| completed_pending_path.as_ref().map(|(_, p)| p.as_str()));
                    for (sid, sroot) in &active.service_roots {
                        if *sid == ctx.service {
                            continue;
                        }
                        let Some(tracker) = active.service_trackers.get_mut(sid) else {
                            continue;
                        };
                        let sroot_str = sroot.to_string_lossy();
                        let mut routed = json!({
                            "kind": "tool",
                            "state": state_str,
                            "tool": tool_name,
                            "ok": raw_targets["ok"] != false,
                        });
                        let mut matched_any = false;
                        if let Some(raw_path) = effective_raw_path
                            && let (Some(rel), false) = relative(raw_path, &sroot_str, None)
                        {
                            routed["path"] = json!(rel);
                            matched_any = true;
                        }
                        if let Some(arr) = raw_targets["changes"].as_array() {
                            let mut mapped_changes = Vec::new();
                            for ch in arr {
                                if let Some(raw_path) = ch["path"].as_str()
                                    && let (Some(rel), false) = relative(raw_path, &sroot_str, None)
                                {
                                    mapped_changes.push(json!({
                                        "path": rel,
                                        "kind": ch["kind"],
                                    }));
                                    matched_any = true;
                                }
                            }
                            if !mapped_changes.is_empty() {
                                routed["changes"] = json!(mapped_changes);
                            }
                        }
                        if matched_any && tracker.observe_tool_event(&routed).is_some() {
                            focus_emissions.push((sid.clone(), tracker.focus().clone()));
                        }
                    }
                }
            }
            guard.bump_run_epoch(&ctx.run_id);
        }

        for (service, focus) in focus_emissions {
            self.emit_focus_event(
                &ctx.run_id,
                ctx.launch_id.as_deref(),
                &service,
                &ctx.phase,
                ctx.attempt,
                &focus,
            );
        }
        Ok(())
    }

    fn on_dispatch_finish(
        &self,
        ctx: &DispatchGraphContext,
        _receipt: &Value,
    ) -> infra_core::Result<Option<DispatchEvidence>> {
        let removed = self.state.lock().ok().and_then(|mut g| {
            let item = g
                .dispatches
                .remove(&(ctx.run_id.clone(), ctx.phase.clone(), ctx.attempt));
            if item.is_some() {
                g.bump_run_epoch(&ctx.run_id);
            }
            item
        });

        let git_changed = if ctx.mode == "edit" {
            let mut changes = git_changes_since(&ctx.root, ctx.base_head.as_deref());
            for wc in git_worktree_changes(&ctx.root) {
                if !changes.iter().any(|c| c.path == wc.path) {
                    changes.push(wc);
                }
            }
            changes
                .into_iter()
                .filter(|c| !is_runtime_artifact_path(&c.path))
                .collect()
        } else {
            Vec::new()
        };

        let evidence = if let Some(mut active) = removed {
            active.ctx.head_after = ctx.head_after.clone();
            let mut completed_focus = active.enricher.ledger.graph_focus().clone();
            completed_focus.phase = "completed".to_owned();
            self.emit_focus_event(
                &ctx.run_id,
                ctx.launch_id.as_deref(),
                &ctx.service,
                &ctx.phase,
                ctx.attempt,
                &completed_focus,
            );
            for (sid, tracker) in &mut active.service_trackers {
                if *sid != ctx.service && !tracker.focus().is_empty() {
                    tracker.complete();
                    self.emit_focus_event(
                        &ctx.run_id,
                        ctx.launch_id.as_deref(),
                        sid,
                        &ctx.phase,
                        ctx.attempt,
                        tracker.focus(),
                    );
                }
            }
            Some(active.snapshot_evidence(git_changed))
        } else {
            Some(DispatchEvidence {
                phase: ctx.phase.clone(),
                attempt: ctx.attempt,
                launch_id: ctx.launch_id.clone(),
                service: ctx.service.clone(),
                root: ctx.root.to_string_lossy().into_owned(),
                base_head: ctx.base_head.clone(),
                head_after: ctx.head_after.clone(),
                git_changed,
                ..Default::default()
            })
        };

        if ctx.mode == "edit"
            && let Some((_, targets)) =
                resolve_targets_from_system_file(&self.store, Some(&self.graphs), &ctx.system_file)
        {
            for target in targets {
                if target.service_id == ctx.service || ctx.writable.contains(&target.service_id) {
                    let _ = self.graphs.refresh(&target.graph_id, Trigger::Finish);
                }
            }
        }

        Ok(evidence)
    }

    fn on_run_finish(
        &self,
        run_id: &str,
        _state: &Value,
        system: &System,
    ) -> infra_core::Result<()> {
        self.unregister_session(run_id);
        if let Ok((by_root, _)) = registered_project_roots(&self.store) {
            for target in
                targets_from_loaded_system(Some(&self.graphs), &by_root, "", "system.json", system)
            {
                let _ = self.graphs.refresh(&target.graph_id, Trigger::Finish);
            }
        }
        Ok(())
    }

    fn enrich_run_view(
        &self,
        run_id: &str,
        system: Option<&System>,
        state: Option<&Value>,
        dispatches: &[DispatchEvidence],
        view: &mut Value,
    ) -> infra_core::Result<()> {
        if run_id.is_empty() {
            return Ok(());
        }
        let pid = view["project_id"].as_str().unwrap_or("");
        let cfg = view["config_path"].as_str();

        let mut all_dispatches = dispatches.to_vec();
        for live in self.snapshot_active_dispatches(run_id) {
            if !all_dispatches
                .iter()
                .any(|d| d.phase == live.phase && d.attempt == live.attempt)
            {
                all_dispatches.push(live);
            }
        }

        let targets = if let Some(sys) = system
            && let Ok((by_root, _)) = registered_project_roots(&self.store)
        {
            targets_from_loaded_system(
                Some(&self.graphs),
                &by_root,
                pid,
                cfg.unwrap_or("system.json"),
                sys,
            )
        } else {
            resolve_all_targets(&self.store, Some(&self.graphs), None, pid, cfg)
                .map(|(_, _, t)| t)
                .unwrap_or_default()
        };

        // 1. Attach `graph_focus` (`services` + `aggregate`).
        let mut by_service = serde_json::Map::new();
        for target in &targets {
            let key = format!("{SYSTEM_FOCUS_PREFIX}{run_id}.{}", target.service_id);
            if let Ok(Some(val)) = self.store.setting(&key)
                && !val.is_null()
            {
                by_service.insert(target.service_id.clone(), val);
            }
        }
        let agg = self
            .store
            .setting(&format!("{SYSTEM_FOCUS_PREFIX}{run_id}.__aggregate__"))
            .ok()
            .flatten()
            .unwrap_or(Value::Null);
        view["graph_focus"] = json!({
            "services": by_service,
            "aggregate": agg,
        });

        // 2. Enrich `evidence.unopened_callers` and `cross_service_unopened` from ready member-service Graphify indexes.
        if let (Some(sys), Some(st)) = (system, state) {
            // The evidence of a run is summed up from its dispatches, with a
            // look at the checkout of every member, and what it reaches is
            // read from the graph of every edited service. A run that is
            // going on is asked for all of it about once a second. The same
            // evidence over the same graphs gives the same answer, so the
            // answer is kept until either of them changes.
            let key = self.reach_key(
                run_id,
                sys,
                st,
                &all_dispatches,
                &targets,
                &view["evidence"],
            );
            if let Some((evidence, cross)) = self.kept_reach(run_id, key) {
                view["evidence"] = evidence;
                view["cross_service_unopened"] = cross;
                return Ok(());
            }
            let clean_dispatches: Vec<DispatchEvidence> = all_dispatches
                .iter()
                .cloned()
                .map(|mut ev| {
                    ev.git_changed
                        .retain(|c| !is_runtime_artifact_path(&c.path));
                    ev.ledger_edited
                        .retain(|c| !is_runtime_artifact_path(&c.path));
                    ev
                })
                .collect();
            if !clean_dispatches.is_empty() || !view["evidence"].is_object() {
                view["evidence"] = summarize_run_evidence(Some(sys), st, &clean_dispatches);
            }
            let existing = view["evidence"]["cross_service_unopened"].clone();
            let (unopened_callers, merged_cross) = self.reach(
                run_id,
                sys,
                &targets,
                &clean_dispatches,
                &all_dispatches,
                &existing,
            );
            if view["evidence"].is_object() {
                view["evidence"]["unopened_callers"] = json!(unopened_callers);
                view["evidence"]["cross_service_unopened"] = json!(merged_cross);
                view["cross_service_unopened"] = view["evidence"]["cross_service_unopened"].clone();
            }
            self.keep_reach(
                run_id,
                key,
                &view["evidence"],
                &view["cross_service_unopened"],
            );
        }
        Ok(())
    }
}

/// How long what a run's evidence reaches is kept while the run goes on,
/// whatever the evidence says: a guard for an event that was stored without
/// passing the bridge.
const REACH_KEPT: std::time::Duration = std::time::Duration::from_secs(15);
/// Runs whose reach is kept at once; the least recently worked out gives way.
const REACH_RUNS: usize = 32;

/// The evidence of a run as it was last summed up, with what it was found
/// to reach, and what that was worked out from.
struct Reach {
    key: u64,
    at: std::time::Instant,
    evidence: Value,
    cross: Value,
}

impl HarnessCodeGraphBridge {
    /// What the evidence of a run and its reach are worked out from, as
    /// one number: the evidence of every dispatch, the summary the view
    /// came with, the system and its state, the published graph of every
    /// member, and how many events of the run passed the bridge.
    pub(super) fn reach_key(
        &self,
        run_id: &str,
        sys: &System,
        state: &Value,
        dispatches: &[DispatchEvidence],
        targets: &[ResolvedServiceTarget],
        existing: &Value,
    ) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        serde_json::to_string(dispatches)
            .unwrap_or_default()
            .hash(&mut hasher);
        existing.to_string().hash(&mut hasher);
        state["status"].to_string().hash(&mut hasher);
        sys.config.hash(&mut hasher);
        sys.config_data.to_string().hash(&mut hasher);
        for edge in &sys.edges {
            edge.to_string().hash(&mut hasher);
        }
        sys.snapshot.hash(&mut hasher);
        for target in targets {
            target.service_id.hash(&mut hasher);
            target.graph_id.hash(&mut hasher);
            self.graphs.live_stamp(&target.graph_id).hash(&mut hasher);
        }
        let events = self
            .state
            .lock()
            .ok()
            .and_then(|guard| guard.events.get(run_id).copied())
            .unwrap_or(0);
        events.hash(&mut hasher);
        hasher.finish()
    }
    /// The evidence kept for a run, where it was worked out from what
    /// `key` stands for and not long ago.
    fn kept_reach(&self, run_id: &str, key: u64) -> Option<(Value, Value)> {
        let guard = self.reach.lock().ok()?;
        let kept = guard.get(run_id)?;
        (kept.key == key && kept.at.elapsed() < REACH_KEPT)
            .then(|| (kept.evidence.clone(), kept.cross.clone()))
    }
    fn keep_reach(&self, run_id: &str, key: u64, evidence: &Value, cross: &Value) {
        let Ok(mut guard) = self.reach.lock() else {
            return;
        };
        if guard.len() >= REACH_RUNS && !guard.contains_key(run_id) {
            let oldest = guard
                .iter()
                .min_by_key(|(_, kept)| kept.at)
                .map(|(run, _)| run.clone());
            if let Some(oldest) = oldest {
                guard.remove(&oldest);
            }
        }
        guard.insert(
            run_id.to_owned(),
            Reach {
                key,
                at: std::time::Instant::now(),
                evidence: evidence.clone(),
                cross: cross.clone(),
            },
        );
    }
    /// What the evidence of a run reaches beyond what the run opened: in
    /// each edited service the files that use what was changed and were
    /// not opened, and across services the consumers of its contracts.
    pub(super) fn reach(
        &self,
        run_id: &str,
        sys: &System,
        targets: &[ResolvedServiceTarget],
        clean_dispatches: &[DispatchEvidence],
        all_dispatches: &[DispatchEvidence],
        existing: &Value,
    ) -> (Vec<Value>, Vec<Value>) {
        self.reached
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let mut impact_summaries = build_run_scoped_summaries(sys, clean_dispatches);
        let edited_services: BTreeSet<String> = impact_summaries
            .iter()
            .filter(|(sid, s)| *sid != "__system__" && !s.changed_files.is_empty())
            .map(|(sid, _)| sid.clone())
            .collect();

        let mut unopened_callers = Vec::new();
        let target_sysrun = Target::SysRun(run_id.to_owned());
        for target in targets {
            if target.service_id == "__system__" || !edited_services.contains(&target.service_id) {
                continue;
            }
            let Some(idx) = self.graphs.live_index(&target.graph_id) else {
                continue;
            };
            let mut added_for_service = false;
            if let Ok(map_val) = self.graphs.map(
                &target.graph_id,
                Some(&target_sysrun),
                &crate::graph::Shown {
                    scope: "",
                    focus: None,
                    focus_session: None,
                    certain: false,
                    filter: "",
                    followed: None,
                    notests: false,
                    spread: &[],
                    link: None,
                    path: None,
                    longer: 0,
                    failing: &[],
                    cycle: None,
                    base: None,
                    shared: None,
                    patterns: false,
                    history: false,
                    modules: false,
                    no_incoming: false,
                    launch: None,
                    compare_launch: None,
                    agent: None,
                    scenario: None,
                    pattern: None,
                    whole: false,
                    ..Default::default()
                },
            ) && let Some(items) = map_val["run"]["reach"]["rows"].as_array()
                && !items.is_empty()
            {
                for item in items.iter().take(16) {
                    let mut entry = item.clone();
                    entry["service"] = json!(target.service_id);
                    unopened_callers.push(entry);
                    added_for_service = true;
                }
            }
            if !added_for_service
                && let Some(svc_summary) = impact_summaries.get(&target.service_id)
            {
                let edited_ids: BTreeSet<u32> = svc_summary
                    .changed_files
                    .iter()
                    .filter_map(|ch| idx.file_id(&ch.path))
                    .collect();
                let opened_paths = &svc_summary.opened_files;
                let files = idx.files();
                let mut seen_callers = BTreeSet::new();
                for (from, to, pair) in idx.pairs() {
                    if edited_ids.contains(to)
                        && !edited_ids.contains(from)
                        && pair.weight(false) > 0
                        && seen_callers.insert(*from)
                        && let Some(caller_path) = files.get(*from as usize)
                        && !opened_paths.contains(caller_path.as_str())
                    {
                        let callee_path = files.get(*to as usize).cloned().unwrap_or_default();
                        unopened_callers.push(json!({
                            "service": target.service_id,
                            "path": caller_path,
                            "links": pair.weight(false),
                            "to": callee_path,
                            "test": graph_map::is_test_path(caller_path),
                        }));
                        if unopened_callers.len() >= 32 {
                            break;
                        }
                    }
                }
            }
        }

        augment_summaries_with_intra_callers(&self.graphs, targets, &mut impact_summaries, false);
        let opened_by_service = collect_opened_by_service_for_run(all_dispatches, None);
        let sys_graph = SystemGraph::from_system(sys, impact_summaries);
        let mut merged_cross = Vec::new();
        let mut seen_cross = BTreeSet::new();
        if let Some(existing) = existing.as_array() {
            for raw_item in existing {
                let item = normalize_cross_unopened_entry(raw_item);
                let key = (
                    item["consumer"].as_str().unwrap_or("").to_owned(),
                    item["contract"].as_str().unwrap_or("").to_owned(),
                    item["path"].as_str().unwrap_or("").to_owned(),
                );
                if seen_cross.insert(key) {
                    merged_cross.push(item);
                }
            }
        }
        for target in targets {
            if target.service_id == "__system__" || !edited_services.contains(&target.service_id) {
                continue;
            }
            let imp = sys_graph.impact_across_services(&CrossServiceImpactQuery {
                service: Some(target.service_id.clone()),
                path: None,
                contract: None,
                certain: false,
                opened_by_service: opened_by_service.clone(),
            });
            if let Some(arr) = imp["cross_service_unopened"].as_array() {
                for raw_item in arr {
                    let item = normalize_cross_unopened_entry(raw_item);
                    let key = (
                        item["consumer"].as_str().unwrap_or("").to_owned(),
                        item["contract"].as_str().unwrap_or("").to_owned(),
                        item["path"].as_str().unwrap_or("").to_owned(),
                    );
                    if seen_cross.insert(key) {
                        merged_cross.push(item);
                    }
                }
            }
        }
        (unopened_callers, merged_cross)
    }
}
