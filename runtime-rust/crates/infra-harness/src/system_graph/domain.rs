use super::helpers::{
    augment_summaries_with_intra_callers, build_run_scoped_summaries,
    collect_opened_by_service_for_run, collect_service_code_files, intra_service_caller_paths,
    is_runtime_artifact_path, normalize_cross_unopened_list, normalize_graph_action,
};
use super::live_bridge::HarnessCodeGraphBridge;
use super::resolver::{
    MAX_UNDECLARED_SCAN_BYTES, MAX_UNDECLARED_SCAN_FILES, ResolvedServiceTarget, agent_matches,
    load_system_focus_value, load_system_run_record, resolve_all_targets, resolve_service_target,
};
use super::semantics::{BridgeViewContext, bridge_view, contract_probes};
use crate::{
    HarnessError, Result,
    graph::Graphs,
    graph_changes::{self, Target},
    graph_map::{self, Index},
    services::{Domain, Payload, Reply},
    store::{self, SessionStore},
};
use infra_system::{
    ChangedEntityEntry, ChangedFileEntry, CodeReference, CrossServiceImpactQuery, DispatchEvidence,
    LayerViolationEntry, ServiceCodeSummary, SystemGraph, SystemManager, SystemMapQuery,
    catalog::System, collect_git_service_summaries, collect_run_service_summaries,
    git_changes_since, load_run_evidence, summarize_run_evidence,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
    time::{Duration, Instant, UNIX_EPOCH},
};

const SYSTEM_SUMMARY_CACHE_CAP: usize = 32;
const SYSTEM_SUMMARY_LIVE_TTL: Duration = Duration::from_secs(2);

type ServiceSummaryStamp = (String, u64, Option<u64>, Option<bool>, Option<(u64, i128)>);

#[derive(Clone, PartialEq, Eq)]
struct SystemSummaryKey {
    catalog_hash: [u8; 32],
    target_spec: Option<String>,
    run_id: Option<String>,
    agent_filter: Option<String>,
    run_stamp: Option<(u64, i128)>,
    live_dispatch_epoch: Option<(u64, usize)>,
    service_stamps: Vec<ServiceSummaryStamp>,
}

struct SystemSummaryEntry {
    key: SystemSummaryKey,
    created_at: Instant,
    graph_ids: BTreeSet<String>,
    roots: Vec<PathBuf>,
    config_paths: Vec<String>,
    summaries: Arc<BTreeMap<String, ServiceCodeSummary>>,
}

type SummaryCacheSub = (PathBuf, Weak<Mutex<Vec<SystemSummaryEntry>>>);
static SUMMARY_CACHE_SUBSCRIBERS: OnceLock<Mutex<Vec<SummaryCacheSub>>> = OnceLock::new();

fn canonical_or_owned(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn register_summary_cache(state_dir: &Path, cache: &Arc<Mutex<Vec<SystemSummaryEntry>>>) {
    let subs = SUMMARY_CACHE_SUBSCRIBERS.get_or_init(|| Mutex::new(Vec::new()));
    let canon_state = canonical_or_owned(state_dir);
    if let Ok(mut guard) = subs.lock() {
        let mut already = false;
        guard.retain(|(dir, weak)| {
            let Some(live) = weak.upgrade() else {
                return false;
            };
            if Arc::ptr_eq(&live, cache) && *dir == canon_state {
                already = true;
            }
            true
        });
        if !already {
            guard.push((canon_state, Arc::downgrade(cache)));
        }
    }
}

fn is_service_or_system_graph_path(rel_path: &str, config_paths: &[String]) -> bool {
    let norm = rel_path.trim().replace('\\', "/");
    let clean = norm.trim_start_matches("./");
    if clean.is_empty() {
        return false;
    }
    let path = Path::new(clean);
    if crate::graph_ast::is_graph_architecture_path(path)
        || crate::graph_ast::is_graph_manifest_path(path)
    {
        return true;
    }
    if path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| {
            name.eq_ignore_ascii_case("ai-system.json")
                || name.eq_ignore_ascii_case("ai-service.json")
        })
    {
        return true;
    }
    config_paths.iter().any(|cfg| {
        let cfg_clean = cfg.trim().replace('\\', "/");
        !cfg_clean.is_empty() && (clean == cfg_clean || clean.ends_with(&format!("/{cfg_clean}")))
    })
}

fn entry_affected_by_ir_delta(
    entry: &SystemSummaryEntry,
    project_id: &str,
    canon_root: &Path,
    delta: &crate::graph_ir::IrUpdateDelta,
) -> bool {
    let has_code_delta = delta.manifest_changed
        || delta.semantic_changed
        || !delta.edited_files.is_empty()
        || !delta.removed_files.is_empty()
        || !delta.re_resolved_files.is_empty()
        || !delta.incoming_caller_files.is_empty();
    let has_graph_path = delta
        .changed_paths
        .iter()
        .any(|p| is_service_or_system_graph_path(p, &entry.config_paths));
    if !has_code_delta && !has_graph_path {
        return false;
    }
    if entry.graph_ids.contains(project_id) {
        return true;
    }
    entry.roots.iter().any(|root| {
        if root == canon_root {
            return true;
        }
        if let Ok(sub) = root.strip_prefix(canon_root) {
            let sub_prefix = sub
                .to_string_lossy()
                .replace('\\', "/")
                .trim_matches('/')
                .to_owned();
            if sub_prefix.is_empty() {
                return true;
            }
            let prefix_slash = format!("{sub_prefix}/");
            return delta
                .changed_paths
                .iter()
                .chain(delta.edited_files.iter())
                .chain(delta.removed_files.iter())
                .chain(delta.incoming_caller_files.iter())
                .any(|p| {
                    let clean = p.replace('\\', "/");
                    clean.starts_with(&prefix_slash)
                        || is_service_or_system_graph_path(&clean, &entry.config_paths)
                });
        }
        false
    })
}

fn file_mtime_len_stamp(path: &std::path::Path) -> Option<(u64, i128)> {
    let meta = fs::symlink_metadata(path).ok()?;
    let ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as i128);
    Some((meta.len(), ns))
}

fn system_catalog_hash(system: &System, targets: &[ResolvedServiceTarget]) -> [u8; 32] {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(system.config.to_string_lossy().as_bytes());
    h.update([0]);
    if let Ok(cfg_bytes) = serde_json::to_vec(&system.config_data) {
        h.update(&cfg_bytes);
    }
    h.update([0]);
    for target in targets {
        h.update(target.service_id.as_bytes());
        h.update([0]);
        h.update(target.graph_id.as_bytes());
        h.update([0]);
        h.update(target.root.to_string_lossy().as_bytes());
        h.update([0]);
        if let Some(svc) = system.services.get(&target.service_id)
            && let Some(manifest) = &svc.manifest
            && let Ok(bytes) = serde_json::to_vec(manifest)
        {
            h.update(&bytes);
        }
        h.update([1]);
    }
    h.finalize().into()
}

fn split_route_probe(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let body = trimmed.strip_prefix("route:").unwrap_or(trimmed).trim();
    if let Some((verb, rest)) = body.split_once(char::is_whitespace) {
        let verb_up = verb.trim();
        let rest_trim = rest.trim();
        let is_verb = matches!(
            verb_up.to_ascii_uppercase().as_str(),
            "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS" | "ANY" | "*"
        );
        if is_verb && !rest_trim.is_empty() {
            return crate::graph_ast::normalize_route_contract(Some(verb_up), rest_trim, false);
        }
    }
    if body.starts_with('/') || body.starts_with("http://") || body.starts_with("https://") {
        return crate::graph_ast::normalize_route_contract(None, body, false);
    }
    None
}

fn normalized_routes_match(a: &str, b: &str) -> bool {
    let Some(a_body) = a.strip_prefix("route:") else {
        return false;
    };
    let Some(b_body) = b.strip_prefix("route:") else {
        return false;
    };
    let Some((va, pa)) = a_body.split_once(' ') else {
        return false;
    };
    let Some((vb, pb)) = b_body.split_once(' ') else {
        return false;
    };
    if va != "*" && vb != "*" && !va.eq_ignore_ascii_case(vb) {
        return false;
    }
    if pa == pb {
        return true;
    }
    let segs_a: Vec<&str> = pa.trim_start_matches('/').split('/').collect();
    let segs_b: Vec<&str> = pb.trim_start_matches('/').split('/').collect();
    if segs_a.len() != segs_b.len() || (segs_a.len() == 1 && segs_a[0].is_empty()) {
        return false;
    }
    segs_a
        .iter()
        .zip(segs_b.iter())
        .all(|(sa, sb)| sa == sb || *sa == "{param}" || *sb == "{param}")
}

fn host_matches_provider_url(raw: &str, provider_id: &str) -> bool {
    let candidate = raw.strip_prefix("route:").unwrap_or(raw).trim();
    let Some(after_scheme) = candidate
        .strip_prefix("http://")
        .or_else(|| candidate.strip_prefix("https://"))
    else {
        return false;
    };
    let host_port = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    let host = host_port.split(':').next().unwrap_or("");
    !host.is_empty() && host.eq_ignore_ascii_case(provider_id)
}

fn exact_identifier_or_qualified_match(target: &str, probe: &str) -> bool {
    let t = target.trim();
    let p = probe.trim();
    if t.is_empty() || p.is_empty() {
        return false;
    }
    if t.eq_ignore_ascii_case(p) {
        return true;
    }
    let tl = t.to_ascii_lowercase();
    let pl = p.to_ascii_lowercase();
    if let Some(prefix) = tl.strip_suffix(pl.as_str())
        && let Some(sep) = prefix.chars().next_back()
    {
        return matches!(sep, '\\' | ':' | '.' | '/');
    }
    if let Some(prefix) = pl.strip_suffix(tl.as_str())
        && let Some(sep) = prefix.chars().next_back()
    {
        return matches!(sep, '\\' | ':' | '.' | '/');
    }
    false
}

fn semantic_edge_matches_contract(
    sem_kind: &str,
    sem_target: &str,
    provider_id: &str,
    ckind: &str,
    probes: &[String],
) -> Option<String> {
    let kind_lower = ckind.trim().to_ascii_lowercase();
    let edge_kind = sem_kind.trim().to_ascii_lowercase();

    if let Some(raw_route) = sem_target.strip_prefix("route:").or_else(|| {
        matches!(edge_kind.as_str(), "calls_route" | "exposes_route").then_some(sem_target)
    }) {
        if matches!(
            kind_lower.as_str(),
            "event" | "queue" | "topic" | "db" | "table" | "storage" | "database"
        ) {
            return None;
        }
        if let Some(sem_norm) = split_route_probe(raw_route) {
            for probe in probes {
                if let Some(probe_norm) = split_route_probe(probe)
                    && normalized_routes_match(&sem_norm, &probe_norm)
                {
                    return Some(probe.clone());
                }
            }
        }
        if host_matches_provider_url(sem_target, provider_id) {
            return Some(format!("http://{provider_id}"));
        }
        return None;
    }

    if let Some(ev_name) = sem_target.strip_prefix("event:").or_else(|| {
        matches!(edge_kind.as_str(), "publishes_event" | "handles_event").then_some(sem_target)
    }) {
        if matches!(
            kind_lower.as_str(),
            "http" | "route" | "api" | "rest" | "db" | "table" | "storage" | "database"
        ) {
            return None;
        }
        for probe in probes {
            if exact_identifier_or_qualified_match(ev_name, probe) {
                return Some(probe.clone());
            }
        }
        return None;
    }

    if let Some(svc_name) = sem_target
        .strip_prefix("service:")
        .or_else(|| (edge_kind == "di_binds").then_some(sem_target))
    {
        if matches!(kind_lower.as_str(), "db" | "table" | "storage" | "database") {
            return None;
        }
        for probe in probes {
            if exact_identifier_or_qualified_match(svc_name, probe) {
                return Some(probe.clone());
            }
        }
        return None;
    }

    if let Some(tbl_name) = sem_target.strip_prefix("table:").or_else(|| {
        matches!(edge_kind.as_str(), "reads_table" | "writes_table").then_some(sem_target)
    }) {
        if !kind_lower.is_empty()
            && !matches!(
                kind_lower.as_str(),
                "db" | "table" | "storage" | "database" | "other"
            )
        {
            return None;
        }
        for probe in probes {
            if exact_identifier_or_qualified_match(tbl_name, probe) {
                return Some(probe.clone());
            }
        }
        return None;
    }

    None
}

// ---------------------------------------------------------------------------
// `SystemGraphDomain` (`impl Domain`) — Member-Service & System-Wide Endpoints
// ---------------------------------------------------------------------------

pub struct SystemGraphDomain {
    pub store: SessionStore,
    pub graphs: Arc<Graphs>,
    pub bridge: Arc<HarnessCodeGraphBridge>,
    pub manager: Option<Arc<SystemManager>>,
    summary_cache: Arc<Mutex<Vec<SystemSummaryEntry>>>,
}

impl SystemGraphDomain {
    pub fn new(
        store: SessionStore,
        graphs: Arc<Graphs>,
        bridge: Arc<HarnessCodeGraphBridge>,
        manager: Option<Arc<SystemManager>>,
    ) -> Self {
        let summary_cache = Arc::new(Mutex::new(Vec::new()));
        register_summary_cache(&store.state, &summary_cache);
        Self {
            store,
            graphs,
            bridge,
            manager,
            summary_cache,
        }
    }

    fn with_summary_cache(mut self, summary_cache: Arc<Mutex<Vec<SystemSummaryEntry>>>) -> Self {
        register_summary_cache(&self.store.state, &summary_cache);
        self.summary_cache = summary_cache;
        self
    }

    /// Invalidates cached `SystemGraphDomain::summary_cache` entries affected by `delta` across live
    /// domain instances scoped to `state_dir`. Returns the number of evicted cache entries.
    pub(crate) fn invalidate_summary_cache_for_ir_delta(
        state_dir: &Path,
        project_id: &str,
        project_root: &Path,
        delta: &crate::graph_ir::IrUpdateDelta,
    ) -> usize {
        let Some(subs) = SUMMARY_CACHE_SUBSCRIBERS.get() else {
            return 0;
        };
        let canon_state = canonical_or_owned(state_dir);
        let canon_root = canonical_or_owned(project_root);
        let active_caches: Vec<Arc<Mutex<Vec<SystemSummaryEntry>>>> = {
            let Ok(mut guard) = subs.lock() else {
                return 0;
            };
            let mut out = Vec::new();
            guard.retain(|(dir, weak)| {
                let Some(live) = weak.upgrade() else {
                    return false;
                };
                if *dir == canon_state {
                    out.push(live);
                }
                true
            });
            out
        };
        let mut evicted = 0usize;
        for cache in active_caches {
            if let Ok(mut guard) = cache.lock() {
                let before = guard.len();
                guard.retain(|entry| {
                    !entry_affected_by_ir_delta(entry, project_id, &canon_root, delta)
                });
                evicted += before.saturating_sub(guard.len());
            }
        }
        evicted
    }

    fn parse_check_flag(query: &Value) -> Result<bool> {
        match query["check"].as_str() {
            None | Some("") | Some("0") => Ok(false),
            Some("1") => Ok(true),
            Some(_) => Err(HarnessError::invalid("Invalid graph check flag.")),
        }
    }

    fn parse_bool_flag(query: &Value, name: &str) -> Result<bool> {
        match query[name].as_str() {
            None | Some("") | Some("0") | Some("false") => Ok(false),
            Some("1") | Some("true") => Ok(true),
            Some(_) => Err(HarnessError::invalid(format!("Invalid flag '{name}'."))),
        }
    }

    fn normalize_target_and_run_id(
        raw_target: Option<&str>,
        raw_run_id: Option<&str>,
    ) -> (Option<String>, Option<String>) {
        let explicit_rid = raw_run_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|rid| {
                rid.strip_prefix("sysrun:")
                    .or_else(|| rid.strip_prefix("system-run:"))
                    .unwrap_or(rid)
                    .trim()
                    .to_owned()
            })
            .filter(|s| !s.is_empty());

        let trimmed_target = raw_target.map(str::trim).filter(|s| !s.is_empty());
        let target_rid = trimmed_target
            .and_then(|t| {
                t.strip_prefix("sysrun:")
                    .or_else(|| t.strip_prefix("system-run:"))
            })
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);

        let run_id = explicit_rid.or(target_rid);
        let is_bare_sysrun = matches!(trimmed_target, Some("sysrun" | "system-run"));

        let normalized_target = match (trimmed_target, &run_id) {
            (None, Some(rid)) => Some(format!("sysrun:{rid}")),
            (Some(_), Some(rid)) if is_bare_sysrun => Some(format!("sysrun:{rid}")),
            (Some(_), None) if is_bare_sysrun => None,
            (Some(t), _) => Some(t.to_owned()),
            (None, None) => None,
        };
        (normalized_target, run_id)
    }

    fn normalize_target_query_with(
        query: &Value,
        range_via_from_to: bool,
    ) -> Result<(Option<String>, Option<String>)> {
        let raw_target = query["target"].as_str().map(str::trim).unwrap_or("");
        let (raw_from, raw_to, range_label) = if range_via_from_to {
            (
                query["from"].as_str().map(str::trim).unwrap_or(""),
                query["to"].as_str().map(str::trim).unwrap_or(""),
                "from/to",
            )
        } else {
            (
                query["target_from"].as_str().map(str::trim).unwrap_or(""),
                query["target_to"].as_str().map(str::trim).unwrap_or(""),
                "target_from/target_to",
            )
        };
        let effective_target = if !raw_target.is_empty() {
            if !raw_from.is_empty() || !raw_to.is_empty() {
                return Err(HarnessError::invalid(format!(
                    "Specify either target or {range_label} range parameters, not both."
                )));
            }
            Some(raw_target.to_owned())
        } else if !raw_from.is_empty() || !raw_to.is_empty() {
            if raw_from.is_empty() || raw_to.is_empty() {
                return Err(HarnessError::invalid(
                    "Both 'from' and 'to' are required for a range.",
                ));
            }
            let range = format!("range:{raw_from}..{raw_to}");
            Target::parse(&range)?;
            Some(range)
        } else {
            None
        };
        let (norm_target, run_id) = Self::normalize_target_and_run_id(
            effective_target.as_deref(),
            query["run_id"].as_str(),
        );
        if let Some(ref ts) = norm_target {
            Target::parse(ts)?;
        }
        Ok((norm_target, run_id))
    }

    fn normalize_target_query(query: &Value) -> Result<(Option<String>, Option<String>)> {
        Self::normalize_target_query_with(query, true)
    }

    /// Normalizes `target`, `run_id`, and disambiguated `target_from`/`target_to`
    /// range parameters on endpoints where `from`/`to` identify graph or service nodes.
    fn normalize_endpoint_safe_target_query(
        query: &Value,
    ) -> Result<(Option<String>, Option<String>)> {
        Self::normalize_target_query_with(query, false)
    }

    fn resolve_service_summaries_for_target(
        &self,
        system: &System,
        targets: &[ResolvedServiceTarget],
        target_spec: Option<&str>,
        run_id: Option<&str>,
        agent_filter: Option<&str>,
        dispatches: &[DispatchEvidence],
    ) -> Result<BTreeMap<String, ServiceCodeSummary>> {
        let explicit_nonwork_target = if run_id.is_none() {
            match target_spec.filter(|t| !t.is_empty() && *t != "work" && *t != "uncommitted") {
                Some(tspec) => Some(Target::parse(tspec)?),
                None => None,
            }
        } else {
            None
        };
        let mut summaries = if let Some(rid) = run_id {
            let mut run_summaries = build_run_scoped_summaries(system, dispatches);
            if dispatches.is_empty()
                && agent_filter.is_none()
                && let Ok(record) = load_system_run_record(&self.store, rid)
            {
                for target in targets {
                    if let Some(base_sha) = record["baseline_commits"][&target.service_id].as_str()
                    {
                        let entry = run_summaries.entry(target.service_id.clone()).or_default();
                        for ch in git_changes_since(&target.root, Some(base_sha)) {
                            let clean = ch
                                .path
                                .replace('\\', "/")
                                .trim_start_matches("./")
                                .to_owned();
                            if !clean.is_empty()
                                && !is_runtime_artifact_path(&clean)
                                && !entry.changed_files.iter().any(|f| f.path == clean)
                            {
                                entry.changed_files.push(ChangedFileEntry {
                                    is_test: graph_map::is_test_path(&clean),
                                    path: clean,
                                    status: ch.status,
                                    from: ch.from,
                                    additions: None,
                                    deletions: None,
                                });
                            }
                        }
                    }
                }
            }
            run_summaries
        } else if explicit_nonwork_target.is_some() {
            collect_run_service_summaries(system, &[])
        } else {
            collect_git_service_summaries(system, dispatches)
        };

        if let Some(ref parsed_target) = explicit_nonwork_target {
            for target in targets {
                if let Ok(resolved) = graph_changes::resolve(&target.root, parsed_target, &[]) {
                    let entry = summaries.entry(target.service_id.clone()).or_default();
                    entry.changed_files = resolved
                        .files
                        .into_iter()
                        .map(|ch| ChangedFileEntry {
                            is_test: graph_map::is_test_path(&ch.path),
                            path: ch.path,
                            status: ch.status.to_string(),
                            from: ch.from,
                            additions: None,
                            deletions: None,
                        })
                        .collect();
                }
            }
        }
        Ok(summaries)
    }

    fn load_dispatches_for_run(&self, run_id: &str) -> Vec<DispatchEvidence> {
        let exec_dir = self
            .store
            .state
            .join("ai-system")
            .join(run_id)
            .join("execution");
        let run_state = fs::read(exec_dir.join("run.json"))
            .ok()
            .and_then(|b| crate::strict_json::parse(&b, 16 * 1024 * 1024).ok())
            .unwrap_or_else(|| json!({}));
        let mut dispatches = load_run_evidence(&exec_dir, &run_state).unwrap_or_default();
        for live in self.bridge.snapshot_active_dispatches(run_id) {
            if !dispatches
                .iter()
                .any(|d| d.phase == live.phase && d.attempt == live.attempt)
            {
                dispatches.push(live);
            }
        }
        dispatches
    }

    fn opened_by_service_for_run(
        &self,
        dispatches: &[DispatchEvidence],
        agent_filter: Option<&str>,
    ) -> BTreeMap<String, BTreeSet<String>> {
        collect_opened_by_service_for_run(dispatches, agent_filter)
    }

    fn build_enriched_system_graph_summaries(
        &self,
        system: &System,
        targets: &[ResolvedServiceTarget],
        target_spec: Option<&str>,
        run_id: Option<&str>,
        agent_filter: Option<&str>,
    ) -> Result<BTreeMap<String, ServiceCodeSummary>> {
        let run_stamp = run_id.and_then(|rid| {
            file_mtime_len_stamp(
                &self
                    .store
                    .state
                    .join("ai-system")
                    .join(rid)
                    .join("execution")
                    .join("run.json"),
            )
        });
        let mut service_stamps = Vec::with_capacity(targets.len());
        for target in targets {
            let ir_rev = self
                .graphs
                .status(&target.graph_id, false)
                .ok()
                .and_then(|st| st["ir_revision"].as_u64())
                .unwrap_or(0);
            let idx_info = self
                .graphs
                .index_for_target_or_live(&target.graph_id, target_spec)
                .map(|(idx, commit)| (idx.index_id(), commit));
            let arch_stamp = file_mtime_len_stamp(&target.root.join(graph_map::layer_rules::FILE));
            service_stamps.push((
                target.service_id.clone(),
                ir_rev,
                idx_info.as_ref().map(|(id, _)| *id),
                idx_info.map(|(_, is_commit)| is_commit),
                arch_stamp,
            ));
        }
        let live_dispatch_epoch = run_id.map(|rid| self.bridge.run_dispatch_snapshot_key(rid));
        let has_active_dispatches = live_dispatch_epoch.is_some_and(|(_, active)| active > 0);
        let cache_key = SystemSummaryKey {
            catalog_hash: system_catalog_hash(system, targets),
            target_spec: target_spec.map(str::to_owned),
            run_id: run_id.map(str::to_owned),
            agent_filter: agent_filter.map(str::to_owned),
            run_stamp,
            live_dispatch_epoch,
            service_stamps,
        };
        let is_live_work = has_active_dispatches
            || (run_id.is_none()
                && target_spec.is_none_or(|t| t.is_empty() || t == "work" || t == "uncommitted"));
        if let Ok(mut guard) = self.summary_cache.lock()
            && let Some(pos) = guard.iter().position(|entry| {
                entry.key == cache_key
                    && (!is_live_work || entry.created_at.elapsed() < SYSTEM_SUMMARY_LIVE_TTL)
            })
        {
            let entry = guard.remove(pos);
            let cached = Arc::clone(&entry.summaries);
            guard.push(entry);
            return Ok((*cached).clone());
        }

        let dispatches: Vec<DispatchEvidence> = match run_id {
            Some(rid) => self
                .load_dispatches_for_run(rid)
                .into_iter()
                .filter(|ev| agent_matches(agent_filter, &ev.phase, &ev.service))
                .collect(),
            None => Vec::new(),
        };
        let mut summaries = self.resolve_service_summaries_for_target(
            system,
            targets,
            target_spec,
            run_id,
            agent_filter,
            &dispatches,
        )?;

        let provided_tokens: Vec<(String, String, String, Vec<String>)> = system
            .services
            .iter()
            .flat_map(|(sid, svc)| {
                svc.manifest
                    .as_ref()
                    .and_then(|m| m["provides"].as_array())
                    .into_iter()
                    .flatten()
                    .map(move |prov| {
                        let cid = prov["id"].as_str().unwrap_or("").to_owned();
                        let ckind = prov["kind"].as_str().unwrap_or("other").to_owned();
                        (sid.clone(), cid, ckind, contract_probes(prov))
                    })
            })
            .collect();

        for target in targets {
            let entry = summaries.entry(target.service_id.clone()).or_default();
            if let Ok(st) = self.graphs.status(&target.graph_id, false) {
                entry.state = st["state"].as_str().unwrap_or("off").to_owned();
                entry.stale = st["state"] == "stale";
                if let Some(gobj) = st["graph"].as_object() {
                    entry.nodes = gobj.get("nodes").and_then(Value::as_u64).unwrap_or(0) as usize;
                    entry.edges = gobj.get("edges").and_then(Value::as_u64).unwrap_or(0) as usize;
                    if let Some(c) = gobj.get("commit").and_then(Value::as_str) {
                        entry.commit = Some(c.to_owned());
                    }
                }
            }

            let live_idx = self
                .graphs
                .index_for_target_or_live(&target.graph_id, target_spec)
                .map(|(idx, _)| idx);
            if let Some(idx) = &live_idx {
                let changed_set: BTreeSet<&str> = entry
                    .changed_files
                    .iter()
                    .map(|f| f.path.as_str())
                    .collect();
                for (sym_idx, sym) in idx.symbol_entries().iter().enumerate() {
                    if let Some(fpath) = idx.files().get(sym.file as usize)
                        && changed_set.contains(fpath.as_str())
                        && entry.changed_entities.len() < 48
                    {
                        entry.changed_entities.push(ChangedEntityEntry {
                            name: idx.symbol_label(sym_idx as u32).to_owned(),
                            kind: sym.kind.word().to_owned(),
                            path: fpath.clone(),
                            change: "modified".to_owned(),
                            visibility: None,
                            visibility_narrowed: false,
                        });
                    }
                }
                if let Ok(Some(rules)) = graph_map::layer_rules::Rules::read(&target.root) {
                    for (from_f, to_f, pair) in idx.pairs() {
                        if pair.weight(false) == 0 {
                            continue;
                        }
                        let (Some(fp), Some(tp)) = (
                            idx.files().get(*from_f as usize),
                            idx.files().get(*to_f as usize),
                        ) else {
                            continue;
                        };
                        if let Some((_, from_layer, _, to_layer)) =
                            graph_map::check_layer_rule(&rules, fp, tp)
                            && entry.layer_violations.len() < 32
                        {
                            entry.layer_violations.push(LayerViolationEntry {
                                from_path: fp.clone(),
                                to_path: tp.clone(),
                                from_layer,
                                to_layer,
                                reason: Some(format!("{fp} depends upward on {tp}")),
                            });
                        }
                    }
                }
            }

            if target.service_id != "__system__" && !provided_tokens.is_empty() {
                let ir_data = self
                    .graphs
                    .get_ir_store_readonly(&target.graph_id)
                    .and_then(|ir| ir.semantic_edges_for_system().ok());
                if let Some((_, ref sem_edges)) = ir_data {
                    for (rel_path, sem_kind, sem_target, from_sym, _line, _ev) in sem_edges {
                        for (provider_id, cid, ckind, probes) in &provided_tokens {
                            if provider_id == &target.service_id {
                                continue;
                            }
                            if let Some(token) = semantic_edge_matches_contract(
                                sem_kind,
                                sem_target,
                                provider_id,
                                ckind,
                                probes,
                            ) {
                                let contract_opt = (!cid.is_empty()).then(|| cid.clone());
                                let already = entry.code_refs.iter().any(|r| {
                                    r.from_path == *rel_path
                                        && r.target_service == *provider_id
                                        && r.contract == contract_opt
                                });
                                if !already {
                                    entry.code_refs.push(CodeReference {
                                        from_path: rel_path.clone(),
                                        from_symbol: from_sym.clone(),
                                        target_service: provider_id.clone(),
                                        contract: contract_opt,
                                        kind: ckind.clone(),
                                        certainty: "EXTRACTED".to_owned(),
                                        detail: Some(format!("IR edge references `{token}`")),
                                    });
                                }
                            }
                        }
                    }
                }
                let ir_populated = ir_data
                    .as_ref()
                    .is_some_and(|(ir_files, _)| !ir_files.is_empty());
                let candidate_files: Vec<String> = if ir_populated && !entry.stale {
                    // Clean populated IR already provided semantic edges; skip disk walks.
                    Vec::new()
                } else if ir_populated {
                    entry
                        .changed_files
                        .iter()
                        .map(|f| f.path.as_str())
                        .filter(|p| !graph_map::is_test_path(p))
                        .take(MAX_UNDECLARED_SCAN_FILES)
                        .map(str::to_owned)
                        .collect()
                } else if let Some(idx) = &live_idx {
                    idx.files()
                        .iter()
                        .filter(|p| !graph_map::is_test_path(p))
                        .take(MAX_UNDECLARED_SCAN_FILES)
                        .cloned()
                        .collect()
                } else {
                    collect_service_code_files(&target.root, MAX_UNDECLARED_SCAN_FILES)
                };
                for rel_path in candidate_files {
                    if rel_path == "ai-service.json" || rel_path.ends_with(".lock") {
                        continue;
                    }
                    let full_path = target.root.join(&rel_path);
                    let Ok(meta) = fs::symlink_metadata(&full_path) else {
                        continue;
                    };
                    if !meta.is_file() || meta.len() > MAX_UNDECLARED_SCAN_BYTES {
                        continue;
                    }
                    let Ok(content) = fs::read_to_string(&full_path) else {
                        continue;
                    };
                    for (provider_id, cid, ckind, probes) in &provided_tokens {
                        if provider_id == &target.service_id {
                            continue;
                        }
                        let matched_probe = probes
                            .iter()
                            .find(|probe| !probe.is_empty() && content.contains(probe.as_str()));
                        let service_url_probe = format!("http://{provider_id}");
                        let matched = matched_probe.cloned().or_else(|| {
                            content
                                .contains(&service_url_probe)
                                .then_some(service_url_probe)
                        });
                        if let Some(token) = matched {
                            let contract_opt = (!cid.is_empty()).then(|| cid.clone());
                            let already = entry.code_refs.iter().any(|r| {
                                r.from_path == rel_path
                                    && r.target_service == *provider_id
                                    && r.contract == contract_opt
                            });
                            if !already {
                                entry.code_refs.push(CodeReference {
                                    from_path: rel_path.clone(),
                                    from_symbol: None,
                                    target_service: provider_id.clone(),
                                    contract: contract_opt,
                                    kind: ckind.clone(),
                                    certainty: "EXTRACTED".to_owned(),
                                    detail: Some(format!("code references `{token}`")),
                                });
                            }
                        }
                    }
                }
            }
        }

        if let Ok(mut guard) = self.summary_cache.lock() {
            if let Some(pos) = guard.iter().position(|entry| entry.key == cache_key) {
                guard.remove(pos);
            }
            if guard.len() >= SYSTEM_SUMMARY_CACHE_CAP {
                guard.remove(0);
            }
            let mut graph_ids = BTreeSet::new();
            let mut roots = Vec::with_capacity(targets.len() + 1);
            let mut config_paths = Vec::new();
            for target in targets {
                graph_ids.insert(target.project_id.clone());
                graph_ids.insert(target.graph_id.clone());
                let canon_target = canonical_or_owned(&target.root);
                if !roots.contains(&canon_target) {
                    roots.push(canon_target);
                }
                if !target.config_path.is_empty() && !config_paths.contains(&target.config_path) {
                    config_paths.push(target.config_path.clone());
                }
            }
            let canon_sys = canonical_or_owned(&system.root);
            if !roots.contains(&canon_sys) {
                roots.push(canon_sys);
            }
            guard.push(SystemSummaryEntry {
                key: cache_key,
                created_at: Instant::now(),
                graph_ids,
                roots,
                config_paths,
                summaries: Arc::new(summaries.clone()),
            });
        }
        Ok(summaries)
    }

    fn build_enriched_system_graph(
        &self,
        system: &System,
        targets: &[ResolvedServiceTarget],
        target_spec: Option<&str>,
        run_id: Option<&str>,
        agent_filter: Option<&str>,
    ) -> Result<SystemGraph> {
        let summaries = self.build_enriched_system_graph_summaries(
            system,
            targets,
            target_spec,
            run_id,
            agent_filter,
        )?;
        Ok(SystemGraph::from_system(system, summaries))
    }

    fn compute_system_status(
        &self,
        project_id: &str,
        config_path: Option<&str>,
        check: bool,
        service_filter: Option<&str>,
    ) -> Result<Value> {
        if let Some(sid) = service_filter.map(str::trim).filter(|s| !s.is_empty()) {
            return self.compute_service_status(project_id, config_path, sid, check);
        }
        let (system, cfg_rel, targets) = resolve_all_targets(
            &self.store,
            Some(&self.graphs),
            self.manager.as_deref(),
            project_id,
            config_path,
        )?;
        if check {
            for target in &targets {
                let _ = self.graphs.auto_start(&target.graph_id);
            }
        }
        let mut services_map = serde_json::Map::new();
        let mut system_root_status = Value::Null;
        let mut ready_count = 0usize;
        let mut building_count = 0usize;
        let mut stale_count = 0usize;
        let mut failed_count = 0usize;
        let mut missing_count = 0usize;
        let mut enabled_count = 0usize;
        let mut agents_count = 0usize;
        let mut total_count = 0usize;
        let mut first_build = Value::Null;

        for target in &targets {
            let mut st = self.graphs.status(&target.graph_id, false)?;
            st["service"] = json!(target.service_id);
            st["graph_id"] = json!(target.graph_id);
            st["reused_project"] = json!(target.reused_project);
            st["root"] = json!(target.root.to_string_lossy());
            if target.service_id == "__system__" {
                system_root_status = st;
                continue;
            }
            total_count += 1;
            if st["enabled"] == true {
                enabled_count += 1;
            }
            if st["agents"] == true {
                agents_count += 1;
            }
            match st["state"].as_str().unwrap_or("off") {
                "building" => {
                    building_count += 1;
                    if first_build.is_null() {
                        first_build = st["build"].clone();
                    }
                }
                "ready" => ready_count += 1,
                "stale" => {
                    ready_count += 1;
                    stale_count += 1;
                }
                "failed" => failed_count += 1,
                "missing" => missing_count += 1,
                _ => {}
            }
            services_map.insert(target.service_id.clone(), st);
        }

        for (sid, svc) in &system.services {
            if svc.access != "available" {
                total_count += 1;
                services_map.insert(
                    sid.clone(),
                    json!({
                        "service": sid,
                        "enabled": false,
                        "agents": false,
                        "state": "off",
                        "access": svc.access,
                        "message": format!("Service root unavailable ({}).", svc.access),
                    }),
                );
            }
        }

        let enabled = enabled_count > 0 || system_root_status["enabled"] == true;
        let tell_agents = agents_count > 0 || system_root_status["agents"] == true;
        let state = if !enabled && total_count > 0 {
            "off"
        } else if building_count > 0 || system_root_status["state"] == "building" {
            "building"
        } else if missing_count > 0 && ready_count == 0 {
            "missing"
        } else if failed_count > 0 && ready_count == 0 {
            "failed"
        } else if total_count == 0 {
            system_root_status["state"].as_str().unwrap_or("ready")
        } else if ready_count == 0 {
            "empty"
        } else if stale_count > 0 {
            "stale"
        } else {
            "ready"
        };

        let message = match state {
            "building" => format!(
                "Building code graphs ({building_count} building, {ready_count}/{total_count} ready)."
            ),
            "ready" => format!(
                "System code graph ready ({ready_count}/{total_count} member services indexed)."
            ),
            "stale" => format!(
                "System code graph available ({ready_count}/{total_count} indexed, {stale_count} stale)."
            ),
            "off" => "Code graphs are turned off for this system.".to_owned(),
            "missing" => {
                "Graphify is not installed; architecture map uses passport & Git metadata."
                    .to_owned()
            }
            "failed" => "Code graph build failed for one or more member services.".to_owned(),
            _ => format!("{ready_count}/{total_count} member service graphs ready."),
        };

        Ok(json!({
            "project_id": project_id,
            "config_path": cfg_rel,
            "state": state,
            "enabled": enabled,
            "agents": tell_agents,
            "tell_agents": tell_agents,
            "ready_count": ready_count,
            "total_count": total_count,
            "building_count": building_count,
            "message": message,
            "build": first_build,
            "system": system_root_status,
            "services": services_map,
        }))
    }

    fn compute_service_status(
        &self,
        project_id: &str,
        config_path: Option<&str>,
        service_id: &str,
        check: bool,
    ) -> Result<Value> {
        let (_, target) = resolve_service_target(
            &self.store,
            Some(&self.graphs),
            self.manager.as_deref(),
            project_id,
            config_path,
            service_id,
        )?;
        if check {
            self.graphs.auto_start(&target.graph_id)?;
        }
        let mut st = self.graphs.status(&target.graph_id, check)?;
        st["project_id"] = json!(project_id);
        st["config_path"] = json!(target.config_path);
        st["service"] = json!(target.service_id);
        st["graph_id"] = json!(target.graph_id);
        st["reused_project"] = json!(target.reused_project);
        st["root"] = json!(target.root.to_string_lossy());
        Ok(st)
    }

    fn execute_service_action(
        &self,
        project_id: &str,
        config_path: Option<&str>,
        service_id: &str,
        raw_action: &str,
        target_arg: Option<&str>,
    ) -> Result<Value> {
        let action = normalize_graph_action(raw_action);
        let (_, target) = resolve_service_target(
            &self.store,
            Some(&self.graphs),
            self.manager.as_deref(),
            project_id,
            config_path,
            service_id,
        )?;
        let mut out = if action == "changes" {
            let raw_t = target_arg
                .filter(|s| !s.is_empty())
                .ok_or_else(|| HarnessError::invalid("Target is required for 'changes' action."))?;
            let parsed = Target::parse(raw_t)?;
            self.graphs.choose(&target.graph_id, &parsed)?
        } else {
            self.graphs.act(&target.graph_id, action)?
        };
        out["project_id"] = json!(project_id);
        out["config_path"] = json!(target.config_path);
        out["service"] = json!(target.service_id);
        out["graph_id"] = json!(target.graph_id);
        out["reused_project"] = json!(target.reused_project);
        Ok(out)
    }

    fn execute_system_action(&self, request: &Value) -> Result<Value> {
        let body = &request["body"];
        let query = &request["query"];
        if !body.is_object()
            || !store::exact(
                body,
                &["action"],
                &["project_id", "config_path", "service", "target"],
            )
        {
            return Err(HarnessError::invalid("Invalid system code graph action."));
        }
        let project_id = body["project_id"]
            .as_str()
            .or_else(|| query["project_id"].as_str())
            .unwrap_or("");
        let config_path = body["config_path"]
            .as_str()
            .or_else(|| query["config_path"].as_str());
        let service_opt = body["service"]
            .as_str()
            .or_else(|| query["service"].as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let action = normalize_graph_action(body["action"].as_str().unwrap_or(""));
        let target_arg = body["target"].as_str();

        if let Some(sid) = service_opt {
            return self.execute_service_action(project_id, config_path, sid, action, target_arg);
        }

        let (_, cfg_rel, targets) = resolve_all_targets(
            &self.store,
            Some(&self.graphs),
            self.manager.as_deref(),
            project_id,
            config_path,
        )?;
        if targets.is_empty() {
            return self.compute_system_status(project_id, Some(&cfg_rel), false, None);
        }

        match action {
            "enable" | "disable" | "withhold" | "delete" => {
                for t in &targets {
                    let _ = self.graphs.act(&t.graph_id, action)?;
                }
            }
            "offer" => {
                let mut offered_any = false;
                for t in &targets {
                    if self.graphs.status(&t.graph_id, false)?["enabled"] == true {
                        self.graphs.act(&t.graph_id, "offer")?;
                        offered_any = true;
                    }
                }
                if !offered_any {
                    return Err(HarnessError::invalid(
                        "Turn the code graph on before telling agents about it.",
                    ));
                }
            }
            "rebuild" => {
                let mut rebuilt_any = false;
                let mut last_err = None;
                for t in &targets {
                    if self.graphs.status(&t.graph_id, false)?["enabled"] == true {
                        match self.graphs.act(&t.graph_id, "rebuild") {
                            Ok(_) => rebuilt_any = true,
                            Err(err) => last_err = Some(err),
                        }
                    }
                }
                if !rebuilt_any {
                    if let Some(err) = last_err {
                        return Err(err);
                    }
                    return Err(HarnessError::invalid(
                        "Turn the code graph on before rebuilding it.",
                    ));
                }
            }
            "changes" => {
                let raw_t = target_arg.filter(|s| !s.is_empty()).ok_or_else(|| {
                    HarnessError::invalid("Target is required for 'changes' action.")
                })?;
                let parsed = Target::parse(raw_t)?;
                for t in &targets {
                    let _ = self.graphs.choose(&t.graph_id, &parsed);
                }
            }
            _ => {
                return Err(HarnessError::invalid(
                    "Choose enable, disable, offer, withhold, rebuild or delete.",
                ));
            }
        }
        self.compute_system_status(project_id, Some(&cfg_rel), false, None)
    }

    async fn delegate_to_graphs(
        &self,
        op: &'static str,
        target: &ResolvedServiceTarget,
        mut query: Value,
        body: Value,
    ) -> Result<Value> {
        if let Some(qobj) = query.as_object_mut() {
            let range_via_from_to = matches!(op, "graph.changes" | "graph.file_impact");
            let (norm_target, _) =
                Self::normalize_target_query_with(&Value::Object(qobj.clone()), range_via_from_to)?;
            qobj.remove("project_id");
            qobj.remove("config_path");
            qobj.remove("service");
            qobj.remove("run_id");
            qobj.remove("target_from");
            qobj.remove("target_to");
            if range_via_from_to {
                qobj.remove("from");
                qobj.remove("to");
            }
            match norm_target {
                Some(nt) => {
                    qobj.insert("target".to_owned(), json!(nt));
                }
                None => {
                    qobj.remove("target");
                }
            }
            if op == "graph.find"
                && !qobj.contains_key("query")
                && let Some(qval) = qobj.remove("q")
            {
                qobj.insert("query".to_owned(), qval);
            }
        }
        let delegated = json!({
            "params": { "project_id": target.graph_id },
            "query": query,
            "body": body,
        });
        match self.graphs.execute(op, delegated).await? {
            Payload::Json(mut val) => {
                if val.is_object() {
                    val["project_id"] = json!(target.project_id);
                    val["config_path"] = json!(target.config_path);
                    val["service"] = json!(target.service_id);
                    val["graph_id"] = json!(target.graph_id);
                    val["reused_project"] = json!(target.reused_project);
                }
                Ok(val)
            }
            _ => Err(HarnessError::unavailable(
                "Unexpected code graph response payload.",
            )),
        }
    }

    fn execute_blocking(&self, operation: &str, request: &Value) -> Result<Value> {
        let query = &request["query"];
        match operation {
            "system_graph.service_status" => {
                let check = Self::parse_check_flag(query)?;
                self.compute_service_status(
                    query["project_id"].as_str().unwrap_or(""),
                    query["config_path"].as_str(),
                    query["service"].as_str().unwrap_or(""),
                    check,
                )
            }
            "system_graph.service_act" => {
                let body = &request["body"];
                if !body.is_object()
                    || !store::exact(
                        body,
                        &["action"],
                        &["project_id", "config_path", "service", "target"],
                    )
                {
                    return Err(HarnessError::invalid(
                        "Invalid member-service code graph action.",
                    ));
                }
                let pid = body["project_id"]
                    .as_str()
                    .or_else(|| query["project_id"].as_str())
                    .unwrap_or("");
                let cfg = body["config_path"]
                    .as_str()
                    .or_else(|| query["config_path"].as_str());
                let sid = body["service"]
                    .as_str()
                    .or_else(|| query["service"].as_str())
                    .unwrap_or("");
                self.execute_service_action(
                    pid,
                    cfg,
                    sid,
                    body["action"].as_str().unwrap_or(""),
                    body["target"].as_str(),
                )
            }
            "system_graph.status" => {
                let check = Self::parse_check_flag(query)?;
                self.compute_system_status(
                    query["project_id"].as_str().unwrap_or(""),
                    query["config_path"].as_str(),
                    check,
                    query["service"].as_str(),
                )
            }
            "system_graph.act" => self.execute_system_action(request),
            "system_graph.map" => {
                let project_id = query["project_id"].as_str().unwrap_or("");
                let config_path = query["config_path"].as_str();
                let (target_spec, run_id) = Self::normalize_endpoint_safe_target_query(query)?;
                let agent_filter = query["agent"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let (system, cfg_rel, targets) = resolve_all_targets(
                    &self.store,
                    Some(&self.graphs),
                    self.manager.as_deref(),
                    project_id,
                    config_path,
                )?;
                let sys_graph = self.build_enriched_system_graph(
                    &system,
                    &targets,
                    target_spec.as_deref(),
                    run_id.as_deref(),
                    agent_filter,
                )?;
                let map_query = parse_system_map_query(query)?;
                let mut view = sys_graph.query(&map_query);
                view["project_id"] = json!(project_id);
                view["config_path"] = json!(cfg_rel);
                if let Some(ts) = &target_spec {
                    view["target"] = json!(ts);
                }
                if let Some(af) = agent_filter {
                    view["agent"] = json!(af);
                }
                if let Some(rid) = &run_id {
                    let dispatches: Vec<DispatchEvidence> = self
                        .load_dispatches_for_run(rid)
                        .into_iter()
                        .filter(|ev| agent_matches(agent_filter, &ev.phase, &ev.service))
                        .map(|mut ev| {
                            ev.git_changed
                                .retain(|c| !is_runtime_artifact_path(&c.path));
                            ev.ledger_edited
                                .retain(|c| !is_runtime_artifact_path(&c.path));
                            ev
                        })
                        .collect();
                    let exec_dir = self
                        .store
                        .state
                        .join("ai-system")
                        .join(rid)
                        .join("execution");
                    let run_state = fs::read(exec_dir.join("run.json"))
                        .ok()
                        .and_then(|b| crate::strict_json::parse(&b, 16 * 1024 * 1024).ok())
                        .unwrap_or_else(|| json!({}));
                    view["run"] = summarize_run_evidence(Some(&system), &run_state, &dispatches);
                    if let Ok(Some(focus)) = load_system_focus_value(&self.store, rid, project_id) {
                        view["graph_focus"] = focus;
                    }
                }
                Ok(view)
            }
            "system_graph.find" => {
                let project_id = query["project_id"].as_str().unwrap_or("");
                let config_path = query["config_path"].as_str();
                let wanted = query["query"]
                    .as_str()
                    .or_else(|| query["q"].as_str())
                    .unwrap_or("");
                if wanted.len() > 400 || wanted.chars().any(char::is_control) {
                    return Err(HarnessError::invalid("Invalid search query."));
                }
                let notests = Self::parse_bool_flag(query, "notests")?;
                let service_filter = query["service"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let (system, cfg_rel, targets) = resolve_all_targets(
                    &self.store,
                    Some(&self.graphs),
                    self.manager.as_deref(),
                    project_id,
                    config_path,
                )?;
                let sys_graph =
                    self.build_enriched_system_graph(&system, &targets, None, None, None)?;
                let mut out = sys_graph.find(wanted);
                out["project_id"] = json!(project_id);
                out["config_path"] = json!(cfg_rel);

                if let Some(sf) = service_filter
                    && let Some(results) = out["results"].as_array_mut()
                {
                    results.retain(|item| {
                        item["service"].as_str() == Some(sf) || item["id"].as_str() == Some(sf)
                    });
                    out["total"] = json!(results.len());
                }

                if !wanted.trim().is_empty()
                    && let Some(results) = out["results"].as_array_mut()
                {
                    for target in &targets {
                        if let Some(sf) = service_filter
                            && target.service_id != sf
                        {
                            continue;
                        }
                        let Some(raw_idx) = self.graphs.live_index(&target.graph_id) else {
                            continue;
                        };
                        let idx: Arc<Index> = if notests {
                            raw_idx.without_tests()
                        } else {
                            raw_idx
                        };
                        let found = idx.find(wanted);
                        if let Some(rows) = found["rows"].as_array() {
                            for row in rows.iter().take(25) {
                                if results.len() >= 120 {
                                    break;
                                }
                                let mut item = row.clone();
                                item["kind"] =
                                    item.get("what").cloned().unwrap_or_else(|| json!("symbol"));
                                item["service"] = json!(target.service_id);
                                item["graph_id"] = json!(target.graph_id);
                                results.push(item);
                            }
                        }
                    }
                    out["total"] = json!(results.len());
                }
                Ok(out)
            }
            "system_graph.changes" => {
                let project_id = query["project_id"].as_str().unwrap_or("");
                let config_path = query["config_path"].as_str();
                let notests = Self::parse_bool_flag(query, "notests")?;
                let service_filter = query["service"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let (target_spec, run_id) = Self::normalize_target_query(query)?;
                let agent_filter = query["agent"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let (system, cfg_rel, targets) = resolve_all_targets(
                    &self.store,
                    Some(&self.graphs),
                    self.manager.as_deref(),
                    project_id,
                    config_path,
                )?;
                let sys_graph = self.build_enriched_system_graph(
                    &system,
                    &targets,
                    target_spec.as_deref(),
                    run_id.as_deref(),
                    agent_filter,
                )?;
                let mut out = sys_graph.changes_across_services(service_filter, notests);
                out["project_id"] = json!(project_id);
                out["config_path"] = json!(cfg_rel);
                out["target"] = json!(target_spec.as_deref().unwrap_or("work"));
                if let Some(af) = agent_filter {
                    out["agent"] = json!(af);
                }
                Ok(out)
            }
            "system_graph.impact" => {
                let project_id = query["project_id"].as_str().unwrap_or("");
                let config_path = query["config_path"].as_str();
                let certain = Self::parse_bool_flag(query, "certain")?;
                let (target_spec, run_id) = Self::normalize_target_query(query)?;
                let agent_filter = query["agent"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let focus_session = query["focus_session"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let parsed_target = match target_spec.as_deref() {
                    None => None,
                    Some(ts) => Some(Target::parse(ts)?),
                };
                let service_opt = query["service"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                let path_opt = query["path"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                let contract_opt = query["contract"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                let (system, cfg_rel, targets) = resolve_all_targets(
                    &self.store,
                    Some(&self.graphs),
                    self.manager.as_deref(),
                    project_id,
                    config_path,
                )?;
                let dispatches = run_id
                    .as_deref()
                    .map(|rid| self.load_dispatches_for_run(rid))
                    .unwrap_or_default();
                let opened_by_service = self.opened_by_service_for_run(&dispatches, agent_filter);
                let mut summaries = self.build_enriched_system_graph_summaries(
                    &system,
                    &targets,
                    target_spec.as_deref(),
                    run_id.as_deref(),
                    agent_filter,
                )?;
                let query_path_for_sys = if let (Some(sid), Some(p)) = (&service_opt, &path_opt)
                    && let Some(svc_target) = targets.iter().find(|t| t.service_id == *sid)
                {
                    let clean_p = p.replace('\\', "/").trim_start_matches("./").to_owned();
                    let mut expanded = vec![clean_p.clone()];
                    if let Some((idx, _)) = self
                        .graphs
                        .index_for_target_or_live(&svc_target.graph_id, target_spec.as_deref())
                    {
                        expanded.extend(intra_service_caller_paths(
                            &idx,
                            std::slice::from_ref(&clean_p),
                            certain,
                        ));
                    }
                    let entry = summaries.entry(sid.clone()).or_default();
                    entry.changed_files = expanded
                        .into_iter()
                        .map(|path| ChangedFileEntry {
                            is_test: graph_map::is_test_path(&path),
                            path,
                            status: "M".to_owned(),
                            from: None,
                            additions: None,
                            deletions: None,
                        })
                        .collect();
                    None
                } else {
                    augment_summaries_with_intra_callers(
                        &self.graphs,
                        &targets,
                        &mut summaries,
                        certain,
                    );
                    path_opt.clone()
                };
                let sys_graph = SystemGraph::from_system(&system, summaries);
                let mut out = sys_graph.impact_across_services(&CrossServiceImpactQuery {
                    service: service_opt.clone(),
                    path: query_path_for_sys,
                    contract: contract_opt,
                    certain,
                    opened_by_service,
                });
                out["project_id"] = json!(project_id);
                out["config_path"] = json!(cfg_rel);
                if let Some(ts) = &target_spec {
                    out["target"] = json!(ts);
                }
                if let Some(af) = agent_filter {
                    out["agent"] = json!(af);
                }
                if let (Some(sid), Some(path)) = (&service_opt, &path_opt)
                    && let Some(target) = targets.iter().find(|t| t.service_id == *sid)
                    && let Ok(intra) = self.graphs.file_impact_with_agent(
                        &target.graph_id,
                        path,
                        parsed_target.as_ref(),
                        focus_session,
                        agent_filter,
                        certain,
                    )
                {
                    out["intra_service"] = intra;
                }
                Ok(out)
            }
            "system_graph.bridge" => {
                let project_id = query["project_id"].as_str().unwrap_or("");
                let config_path = query["config_path"].as_str();
                let certain = Self::parse_bool_flag(query, "certain")?;
                let (target_spec, run_id) = Self::normalize_endpoint_safe_target_query(query)?;
                let agent_filter = query["agent"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let named = |key: &str| -> Result<String> {
                    let value = query[key].as_str().map(str::trim).unwrap_or("");
                    if value.len() > 256 || value.chars().any(char::is_control) {
                        return Err(HarnessError::invalid("Invalid service link."));
                    }
                    Ok(value.to_owned())
                };
                let (from, to, contract) = (named("from")?, named("to")?, named("contract")?);
                if from.is_empty() || to.is_empty() || from == to {
                    return Err(HarnessError::invalid("Invalid service link."));
                }
                let (system, cfg_rel, targets) = resolve_all_targets(
                    &self.store,
                    Some(&self.graphs),
                    self.manager.as_deref(),
                    project_id,
                    config_path,
                )?;
                let dispatches = run_id
                    .as_deref()
                    .map(|rid| self.load_dispatches_for_run(rid))
                    .unwrap_or_default();
                let opened_by_service = self.opened_by_service_for_run(&dispatches, agent_filter);
                let sys_graph = self.build_enriched_system_graph(
                    &system,
                    &targets,
                    target_spec.as_deref(),
                    run_id.as_deref(),
                    agent_filter,
                )?;
                let link = sys_graph.link(&from, &to, certain);
                let ctx = BridgeViewContext {
                    target_spec: target_spec.as_deref(),
                    run_id: run_id.as_deref(),
                    opened_by_service: run_id.is_some().then_some(&opened_by_service),
                };
                let mut view = bridge_view(
                    &self.graphs,
                    &system,
                    &targets,
                    &link,
                    (!contract.is_empty()).then_some(contract.as_str()),
                    certain,
                    &ctx,
                );
                view["project_id"] = json!(project_id);
                view["config_path"] = json!(cfg_rel);
                Ok(view)
            }
            _ => Err(HarnessError::missing(
                "Unknown system code graph operation.",
            )),
        }
    }
}

impl Domain for SystemGraphDomain {
    fn execute<'a>(&'a self, operation: &'a str, request: Value) -> Reply<'a> {
        Box::pin(async move {
            let query = &request["query"];
            match operation {
                "system_graph.service_map"
                | "system_graph.service_find"
                | "system_graph.service_changes"
                | "system_graph.service_file_impact"
                | "system_graph.service_pulls"
                | "system_graph.service_commits"
                | "system_graph.service_branches" => {
                    let project_id = query["project_id"].as_str().unwrap_or("");
                    let config_path = query["config_path"].as_str();
                    let service_id = query["service"].as_str().unwrap_or("");
                    let (system, target) = resolve_service_target(
                        &self.store,
                        Some(&self.graphs),
                        self.manager.as_deref(),
                        project_id,
                        config_path,
                        service_id,
                    )?;
                    let graph_op = match operation {
                        "system_graph.service_map" => "graph.map",
                        "system_graph.service_find" => "graph.find",
                        "system_graph.service_changes" => "graph.changes",
                        "system_graph.service_file_impact" => "graph.file_impact",
                        "system_graph.service_pulls" => "graph.pulls",
                        "system_graph.service_commits" => "graph.commits",
                        _ => "graph.branches",
                    };
                    let mut out = self
                        .delegate_to_graphs(
                            graph_op,
                            &target,
                            query.clone(),
                            request["body"].clone(),
                        )
                        .await?;

                    if operation == "system_graph.service_map" && target.service_id != "__system__"
                    {
                        let (target_spec, run_id) =
                            Self::normalize_endpoint_safe_target_query(query)?;
                        let target_str = out["target"].as_str().unwrap_or("");
                        let effective_target = target_spec
                            .as_deref()
                            .or_else(|| (!target_str.is_empty()).then_some(target_str));
                        if run_id.is_some() || effective_target.is_some() {
                            let agent_filter = query["agent"]
                                .as_str()
                                .map(str::trim)
                                .filter(|s| !s.is_empty());
                            let selected_launch =
                                out["run"]["launch"].as_str().filter(|s| !s.is_empty());
                            let dispatches: Vec<DispatchEvidence> = run_id
                                .as_deref()
                                .map(|rid| self.load_dispatches_for_run(rid))
                                .unwrap_or_default()
                                .into_iter()
                                .filter(|ev| {
                                    selected_launch.is_none_or(|sel| {
                                        ev.launch_id.as_deref().is_none_or(|lid| lid == sel)
                                    })
                                })
                                .collect();
                            let opened_by_service =
                                self.opened_by_service_for_run(&dispatches, agent_filter);
                            let certain = Self::parse_bool_flag(query, "certain").unwrap_or(false);
                            let filtered_for_edits: Vec<DispatchEvidence> = dispatches
                                .iter()
                                .filter(|ev| agent_matches(agent_filter, &ev.phase, &ev.service))
                                .cloned()
                                .collect();
                            let mut summaries = self.resolve_service_summaries_for_target(
                                &system,
                                std::slice::from_ref(&target),
                                effective_target,
                                run_id.as_deref(),
                                agent_filter,
                                &filtered_for_edits,
                            )?;
                            let has_edits = summaries
                                .get(&target.service_id)
                                .is_some_and(|s| !s.changed_files.is_empty());
                            let (cross_arr, consumers_val) = if has_edits {
                                augment_summaries_with_intra_callers(
                                    &self.graphs,
                                    std::slice::from_ref(&target),
                                    &mut summaries,
                                    certain,
                                );
                                let sys_graph = SystemGraph::from_system(&system, summaries);
                                let impact =
                                    sys_graph.impact_across_services(&CrossServiceImpactQuery {
                                        service: Some(target.service_id.clone()),
                                        path: None,
                                        contract: None,
                                        certain,
                                        opened_by_service,
                                    });
                                (
                                    normalize_cross_unopened_list(
                                        &impact["cross_service_unopened"],
                                    ),
                                    impact["contract_consumers"].clone(),
                                )
                            } else {
                                (json!([]), json!([]))
                            };
                            out["cross_service_unopened"] = cross_arr.clone();
                            out["contract_consumers"] = consumers_val;
                            if out["run"]["reach"].is_object() {
                                out["run"]["reach"]["cross_service_unopened"] = cross_arr.clone();
                            }
                            if out["reach"].is_object() {
                                out["reach"]["cross_service_unopened"] = cross_arr;
                            }
                        }
                    }

                    if operation == "system_graph.service_file_impact" {
                        let (target_spec, run_id) = Self::normalize_target_query(query)?;
                        let agent_filter = query["agent"]
                            .as_str()
                            .map(str::trim)
                            .filter(|s| !s.is_empty());
                        let dispatches = run_id
                            .as_deref()
                            .map(|rid| self.load_dispatches_for_run(rid))
                            .unwrap_or_default();
                        let opened_by_service =
                            self.opened_by_service_for_run(&dispatches, agent_filter);
                        let certain = Self::parse_bool_flag(query, "certain").unwrap_or(false);
                        let path_opt = query["path"]
                            .as_str()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned);
                        let mut summaries = self.resolve_service_summaries_for_target(
                            &system,
                            std::slice::from_ref(&target),
                            target_spec.as_deref(),
                            run_id.as_deref(),
                            agent_filter,
                            &dispatches,
                        )?;
                        let query_path_for_sys = if let Some(ref p) = path_opt {
                            let clean_p = p.replace('\\', "/").trim_start_matches("./").to_owned();
                            let mut expanded = vec![clean_p.clone()];
                            if let Some((idx, _)) = self
                                .graphs
                                .index_for_target_or_live(&target.graph_id, target_spec.as_deref())
                            {
                                expanded.extend(intra_service_caller_paths(
                                    &idx,
                                    std::slice::from_ref(&clean_p),
                                    certain,
                                ));
                            }
                            let entry = summaries.entry(target.service_id.clone()).or_default();
                            entry.changed_files = expanded
                                .into_iter()
                                .map(|path| ChangedFileEntry {
                                    is_test: graph_map::is_test_path(&path),
                                    path,
                                    status: "M".to_owned(),
                                    from: None,
                                    additions: None,
                                    deletions: None,
                                })
                                .collect();
                            None
                        } else {
                            augment_summaries_with_intra_callers(
                                &self.graphs,
                                std::slice::from_ref(&target),
                                &mut summaries,
                                certain,
                            );
                            None
                        };
                        let sys_graph = SystemGraph::from_system(&system, summaries);
                        let impact = sys_graph.impact_across_services(&CrossServiceImpactQuery {
                            service: Some(target.service_id.clone()),
                            path: query_path_for_sys,
                            contract: None,
                            certain,
                            opened_by_service,
                        });
                        out["contract_consumers"] = impact["contract_consumers"].clone();
                        out["impacted_contracts"] = impact["impacted_contracts"].clone();
                        out["impacted_capabilities"] = impact["impacted_capabilities"].clone();
                        out["downstream_services"] = impact["downstream_services"].clone();
                        out["cross_service_unopened"] =
                            normalize_cross_unopened_list(&impact["cross_service_unopened"]);
                    }

                    Ok(Payload::Json(out))
                }
                _ => {
                    if tokio::runtime::Handle::try_current().is_ok() {
                        let store = self.store.clone();
                        let graphs = self.graphs.clone();
                        let bridge = self.bridge.clone();
                        let manager = self.manager.clone();
                        let summary_cache = self.summary_cache.clone();
                        let op = operation.to_owned();
                        let val = tokio::task::spawn_blocking(move || {
                            let domain = SystemGraphDomain::new(store, graphs, bridge, manager)
                                .with_summary_cache(summary_cache);
                            domain.execute_blocking(&op, &request)
                        })
                        .await
                        .map_err(|_| {
                            HarnessError::unavailable("System code graph worker stopped.")
                        })??;
                        Ok(Payload::Json(val))
                    } else {
                        Ok(Payload::Json(self.execute_blocking(operation, &request)?))
                    }
                }
            }
        })
    }
}

fn parse_csv_or_json_list(raw: Option<&str>, max_items: usize) -> Result<Vec<String>> {
    let Some(text) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(Vec::new());
    };
    if text.starts_with('[') {
        let parsed: Vec<String> = serde_json::from_str(text)
            .map_err(|_| HarnessError::invalid("Invalid list parameter in system graph query."))?;
        if parsed.len() > max_items {
            return Err(HarnessError::invalid(
                "Too many items in system graph query parameter.",
            ));
        }
        return Ok(parsed
            .into_iter()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect());
    }
    let items: Vec<String> = text
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if items.len() > max_items {
        return Err(HarnessError::invalid(
            "Too many items in system graph query parameter.",
        ));
    }
    Ok(items)
}

fn parse_hop_count(raw: Option<&str>) -> Result<usize> {
    let Some(text) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(0);
    };
    if let Ok(n) = text.parse::<usize>() {
        if n > 64 {
            return Err(HarnessError::invalid("Hop count exceeds bound."));
        }
        return Ok(n);
    }
    let list = parse_csv_or_json_list(Some(text), 64)?;
    Ok(list.len().min(64))
}

fn parse_system_map_query(query: &Value) -> Result<SystemMapQuery> {
    let text_opt = |name: &str| -> Result<Option<String>> {
        let Some(v) = query[name]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return Ok(None);
        };
        if v.len() > 1024 || v.chars().any(char::is_control) {
            return Err(HarnessError::invalid(format!(
                "Invalid system graph parameter '{name}'."
            )));
        }
        Ok(Some(v.to_owned()))
    };
    let bool_flag = |name: &str| SystemGraphDomain::parse_bool_flag(query, name);
    let depth = match query["depth"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None => None,
        Some(d) => Some(
            d.parse::<usize>()
                .ok()
                .filter(|n| *n <= 64)
                .ok_or_else(|| HarnessError::invalid("Invalid blast-radius depth."))?,
        ),
    };
    let skip = match query["skip"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None => 0,
        Some(s) => s
            .parse::<usize>()
            .ok()
            .filter(|n| *n <= 10_000)
            .ok_or_else(|| HarnessError::invalid("Invalid skip parameter."))?,
    };
    let mut from = text_opt("from")?;
    let mut to = text_opt("to")?;
    let mut is_path = false;
    let mut is_shared = false;

    match query["path"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None | Some("0") | Some("false") => {}
        Some("1") | Some("true") => {
            is_path = true;
        }
        Some(raw_path) => {
            let path_items = parse_csv_or_json_list(Some(raw_path), 2)?;
            let [a, b] = path_items.as_slice() else {
                return Err(HarnessError::invalid(
                    "Invalid system graph path endpoints.",
                ));
            };
            from = Some(a.clone());
            to = Some(b.clone());
            is_path = true;
        }
    }

    match query["shared"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None | Some("0") | Some("false") => {}
        Some("1") | Some("true") => {
            if is_path {
                return Err(HarnessError::invalid(
                    "Cannot combine 'path' and 'shared' queries.",
                ));
            }
            is_shared = true;
        }
        Some(raw_shared) => {
            if is_path {
                return Err(HarnessError::invalid(
                    "Cannot combine 'path' and 'shared' queries.",
                ));
            }
            let shared_items = parse_csv_or_json_list(Some(raw_shared), 2)?;
            let [a, b] = shared_items.as_slice() else {
                return Err(HarnessError::invalid(
                    "Invalid system graph shared endpoints.",
                ));
            };
            from = Some(a.clone());
            to = Some(b.clone());
            is_shared = true;
        }
    }

    if (is_path || is_shared) && (from.is_none() || to.is_none()) {
        return Err(HarnessError::invalid(
            "Both 'from' and 'to' are required for 'path' or 'shared' system graph queries.",
        ));
    }
    if from.is_some() != to.is_some() {
        return Err(HarnessError::invalid(
            "Both 'from' and 'to' are required when inspecting a system graph link.",
        ));
    }

    let cycle_raw = text_opt("cycle")?;
    let is_cycle = match cycle_raw.as_deref() {
        None | Some("0") | Some("false") => false,
        Some(_) => true,
    };

    let mut scope = text_opt("scope")?;
    if scope.is_none()
        && is_cycle
        && let Some(c) = &cycle_raw
        && c != "1"
        && c != "true"
    {
        scope = Some(c.clone());
    }

    Ok(SystemMapQuery {
        scope,
        focus: text_opt("focus")?,
        filter: text_opt("filter")?,
        spread: parse_csv_or_json_list(query["spread"].as_str(), 64)?,
        whole: bool_flag("whole")?,
        certain: bool_flag("certain")?,
        follow: text_opt("follow")?,
        out: parse_hop_count(query["out"].as_str())?,
        into: parse_hop_count(query["into"].as_str())?,
        from,
        to,
        path: is_path,
        shared: is_shared,
        side: text_opt("side")?,
        skip,
        cycle: is_cycle,
        layers: bool_flag("layers")?,
        seed_services: parse_csv_or_json_list(query["seed_services"].as_str(), 64)?,
        contracts: parse_csv_or_json_list(query["contracts"].as_str(), 32)?,
        depth,
    })
}

#[cfg(test)]
mod domain_cache_tests {
    use super::*;

    #[test]
    fn system_summary_cache_hits_and_invalidates_on_run_stamp_change() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let sys_dir = dir.path().join("sys");
        let svc_dir = sys_dir.join("svc-a");
        std::fs::create_dir_all(&svc_dir).unwrap();
        std::fs::write(
            svc_dir.join("ai-service.json"),
            r#"{"schema_version":1,"id":"svc-a","description":"Service A","owner":"platform","capabilities":[],"sources":[],"provides":[],"consumes":[]}"#,
        )
        .unwrap();
        std::fs::write(
            sys_dir.join("ai-system.json"),
            r#"{"schema_version":1,"name":"sys-1","shared_sources":[],"services":[{"id":"svc-a","root":"svc-a","manifest":"ai-service.json"}]}"#,
        )
        .unwrap();

        let store = SessionStore::open(&state).unwrap();
        let proj = store.register(&sys_dir).unwrap();
        let sys_proj_id = proj["id"].as_str().unwrap().to_owned();
        let (system, _, _) =
            crate::system_graph::resolve_system(&store, None, &sys_proj_id, Some("ai-system.json"))
                .unwrap();
        let svc_proj = store.register(&svc_dir).unwrap();
        let graph_id = svc_proj["id"].as_str().unwrap().to_owned();

        let graphs = Graphs::with_native_backend(store.clone(), Duration::ZERO);
        let bridge = HarnessCodeGraphBridge::new(store.clone(), graphs.clone());
        let domain = SystemGraphDomain::new(store.clone(), graphs.clone(), bridge, None);

        let run_dir = state.join("ai-system/run-1/execution");
        std::fs::create_dir_all(&run_dir).unwrap();
        std::fs::write(run_dir.join("run.json"), "{\"status\":\"running\"}\n").unwrap();

        let targets = vec![ResolvedServiceTarget {
            project_id: sys_proj_id,
            config_path: "ai-system.json".to_owned(),
            service_id: "svc-a".to_owned(),
            graph_id,
            root: svc_dir,
            name: "Service A".to_owned(),
            reused_project: true,
        }];

        let _ = domain
            .build_enriched_system_graph_summaries(&system, &targets, None, Some("run-1"), None)
            .unwrap();
        assert_eq!(domain.summary_cache.lock().unwrap().len(), 1);
        let _ = domain
            .build_enriched_system_graph_summaries(&system, &targets, None, Some("run-1"), None)
            .unwrap();
        assert_eq!(domain.summary_cache.lock().unwrap().len(), 1);

        // Changing `run.json` bytes/size changes `run_stamp` -> inserts updated cache entry.
        std::fs::write(
            run_dir.join("run.json"),
            "{\"status\":\"completed\",\"done\":true}\n",
        )
        .unwrap();
        let _ = domain
            .build_enriched_system_graph_summaries(&system, &targets, None, Some("run-1"), None)
            .unwrap();
        assert_eq!(domain.summary_cache.lock().unwrap().len(), 2);

        // Live dispatch mutations invalidate `summary_cache` immediately without changing `run.json`.
        let dispatch_ctx = infra_system::DispatchGraphContext {
            run_id: "run-1".to_owned(),
            journal_run_id: "run-1".to_owned(),
            launch_id: None,
            execution_dir: run_dir.clone(),
            system_file: sys_dir.join("ai-system.json"),
            provider: "codex".to_owned(),
            mode: "edit".to_owned(),
            access: "write".to_owned(),
            tell_agents_graph: None,
            phase: "implement".to_owned(),
            dispatch_id: "d-1".to_owned(),
            attempt: 1,
            service: "svc-a".to_owned(),
            root: svc_dir_path_for_test(&targets[0].root),
            roots: BTreeMap::new(),
            selected: Vec::new(),
            readable: Vec::new(),
            writable: Vec::new(),
            base_head: None,
            head_after: None,
            plan: Value::Null,
            state: Value::Null,
        };
        infra_system::CodeGraphBridge::on_dispatch_start(&*domain.bridge, &dispatch_ctx).unwrap();
        let key_after_start = domain.bridge.run_dispatch_snapshot_key("run-1");
        assert_eq!(key_after_start.1, 1);
        let _ = domain
            .build_enriched_system_graph_summaries(&system, &targets, None, Some("run-1"), None)
            .unwrap();
        assert_eq!(domain.summary_cache.lock().unwrap().len(), 3);

        // Simulate tool signal / file read inside active dispatch bumping the run epoch.
        domain.bridge.on_emitted_session_event(
            "run-1",
            Some(99),
            &json!({
                "kind": "agent_activity",
                "type": "tool",
                "state": "completed",
                "phase_id": "implement",
                "attempt": 1,
                "detail": "cat ai-service.json",
            }),
        );
        let key_after_read = domain.bridge.run_dispatch_snapshot_key("run-1");
        assert!(key_after_read.0 > key_after_start.0);
        let _ = domain
            .build_enriched_system_graph_summaries(&system, &targets, None, Some("run-1"), None)
            .unwrap();
        assert_eq!(domain.summary_cache.lock().unwrap().len(), 4);

        let _ = infra_system::CodeGraphBridge::on_dispatch_finish(
            &*domain.bridge,
            &dispatch_ctx,
            &json!({}),
        )
        .unwrap();
        let key_after_finish = domain.bridge.run_dispatch_snapshot_key("run-1");
        assert_eq!(key_after_finish.1, 0);
        assert!(key_after_finish.0 > key_after_read.0);

        graphs.close();
    }

    fn svc_dir_path_for_test(p: &std::path::Path) -> std::path::PathBuf {
        p.to_path_buf()
    }

    #[test]
    fn exact_contract_matching_rejects_partial_route_and_identifier_substrings() {
        // Routes normalize cleanly and enforce verb + full segment match.
        let route_probes = vec!["GET /users/{id}".to_owned(), "users".to_owned()];
        assert!(
            semantic_edge_matches_contract(
                "calls_route",
                "route:GET /users/{param}",
                "auth",
                "http",
                &route_probes
            )
            .is_some()
        );
        assert!(
            semantic_edge_matches_contract(
                "calls_route",
                "route:POST /users/{param}",
                "auth",
                "http",
                &route_probes
            )
            .is_none()
        );
        assert!(
            semantic_edge_matches_contract(
                "calls_route",
                "route:GET /users-admin/export",
                "auth",
                "http",
                &route_probes
            )
            .is_none()
        );

        // Provider host URL matching respects host boundary.
        assert!(
            semantic_edge_matches_contract(
                "calls_route",
                "http://billing/api/charge",
                "billing",
                "http",
                &["/api/charge".to_owned()]
            )
            .is_some()
        );
        assert!(
            semantic_edge_matches_contract(
                "calls_route",
                "http://billing-v2/api/other",
                "billing",
                "http",
                &["/api/pay".to_owned()]
            )
            .is_none()
        );

        // Event/service identifiers require exact identifier or namespace suffix match.
        let order_probes = vec!["Order".to_owned()];
        assert!(
            semantic_edge_matches_contract(
                "publishes_event",
                "event:OrderItem",
                "orders",
                "event",
                &order_probes
            )
            .is_none()
        );
        assert!(
            semantic_edge_matches_contract(
                "publishes_event",
                "event:App\\Events\\Order",
                "orders",
                "event",
                &order_probes
            )
            .is_some()
        );
        assert!(
            semantic_edge_matches_contract(
                "reads_table",
                "table:users",
                "orders",
                "http",
                &["users".to_owned()]
            )
            .is_none()
        );
        assert!(
            semantic_edge_matches_contract(
                "reads_table",
                "table:users",
                "warehouse",
                "db",
                &["users".to_owned()]
            )
            .is_some()
        );
    }

    #[test]
    fn ir_delta_bus_invalidates_only_affected_service_or_system_summary_cache_entries() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let sys_dir = dir.path().join("sys");
        let svc_dir = sys_dir.join("svc-a");
        std::fs::create_dir_all(svc_dir.join("src")).unwrap();
        std::fs::write(svc_dir.join("src/lib.rs"), "pub fn svc_fn() -> i32 { 1 }\n").unwrap();
        std::fs::write(
            svc_dir.join("ai-service.json"),
            r#"{"schema_version":1,"id":"svc-a","description":"Service A","owner":"platform","capabilities":[],"sources":[],"provides":[],"consumes":[]}"#,
        )
        .unwrap();
        std::fs::write(
            sys_dir.join("ai-system.json"),
            r#"{"schema_version":1,"name":"sys-1","shared_sources":[],"services":[{"id":"svc-a","root":"svc-a","manifest":"ai-service.json"}]}"#,
        )
        .unwrap();

        let store = SessionStore::open(&state).unwrap();
        let sys_proj = store.register(&sys_dir).unwrap();
        let sys_proj_id = sys_proj["id"].as_str().unwrap().to_owned();
        let (system, _, _) =
            crate::system_graph::resolve_system(&store, None, &sys_proj_id, Some("ai-system.json"))
                .unwrap();
        let svc_proj = store.register(&svc_dir).unwrap();
        let graph_id = svc_proj["id"].as_str().unwrap().to_owned();
        let graphs = Graphs::with_native_backend(store.clone(), Duration::ZERO);
        let bridge = HarnessCodeGraphBridge::new(store.clone(), graphs.clone());
        let domain = SystemGraphDomain::new(store, graphs.clone(), bridge, None);

        let targets = vec![ResolvedServiceTarget {
            project_id: sys_proj_id.clone(),
            config_path: "ai-system.json".to_owned(),
            service_id: "svc-a".to_owned(),
            graph_id: graph_id.clone(),
            root: svc_dir.clone(),
            name: "Service A".to_owned(),
            reused_project: true,
        }];

        let _ = domain
            .build_enriched_system_graph_summaries(&system, &targets, None, None, None)
            .unwrap();
        assert_eq!(domain.summary_cache.lock().unwrap().len(), 1);

        // Non-graph README change must leave cached system summary intact.
        let readme_delta = crate::graph_ir::IrUpdateDelta {
            changed_paths: vec!["README.md".to_owned()],
            ..Default::default()
        };
        assert_eq!(
            SystemGraphDomain::invalidate_summary_cache_for_ir_delta(
                &state,
                &graph_id,
                &svc_dir,
                &readme_delta
            ),
            0
        );
        assert_eq!(domain.summary_cache.lock().unwrap().len(), 1);

        // Editing ai-service.json evicts matching summary_cache entry immediately.
        let service_delta = crate::graph_ir::IrUpdateDelta {
            changed_paths: vec!["ai-service.json".to_owned()],
            ..Default::default()
        };
        assert_eq!(
            SystemGraphDomain::invalidate_summary_cache_for_ir_delta(
                &state,
                &graph_id,
                &svc_dir,
                &service_delta
            ),
            1
        );
        assert_eq!(domain.summary_cache.lock().unwrap().len(), 0);

        // Re-populate and verify system manifest edit on system project root also invalidates.
        let _ = domain
            .build_enriched_system_graph_summaries(&system, &targets, None, None, None)
            .unwrap();
        assert_eq!(domain.summary_cache.lock().unwrap().len(), 1);
        let sys_delta = crate::graph_ir::IrUpdateDelta {
            changed_paths: vec!["ai-system.json".to_owned()],
            ..Default::default()
        };
        assert_eq!(
            SystemGraphDomain::invalidate_summary_cache_for_ir_delta(
                &state,
                &sys_proj_id,
                &sys_dir,
                &sys_delta
            ),
            1
        );
        assert_eq!(domain.summary_cache.lock().unwrap().len(), 0);

        graphs.close();
    }
}
