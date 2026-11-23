use super::context_sync::{
    conflicts_with_values, find_atom_on_branch, query_edges_safe, run_stored_or_inline,
    stored_atom_params,
};
use super::helpers::{
    AtomProvenance, atom_matches_query, atom_symbols_for_project, build_upsert_atom_params,
    change_to_atom_entry, clean_opt, diff_edge_entry, enrich_commit_change, format_omnigraph_ts,
    list_atom_symbols, normalize_atom_row, normalize_file_row, normalize_session_row,
    parse_relation_entry, parse_string_list, parse_tags_opt, score_atom_fallback, str_field,
    upsert_atom_files, upsert_atom_symbols,
};
use super::{
    COMMIT_DIGEST_CONCURRENCY, COMMIT_DIGEST_PREFETCH_COUNT, CommitDigest,
    DEFAULT_GRAPH_ATOM_LIMIT, MAX_GRAPH_ATOM_LIMIT, OmnigraphManager, VALID_ATOM_KINDS,
};
use crate::omnigraph::{
    client::{
        BranchCreateRequest, BranchDiffQuery, BranchMergeRequest, ChangeOp, ChangeRequest,
        CommitListQuery, CommitOutput, EntityKind, OmnigraphClient, QueryRequest,
        StoredQueryRequest,
    },
    schema::{
        self, GQ_CONTEXT_SEARCH_BY_KIND, GQ_LINK_PUBLISHED_BY, GQ_LINK_RELATES_TO,
        GQ_LIST_ATOM_RELATIONS, GQ_LIST_CONSUMED_EDGES, GQ_LIST_FILE_EDGES, GQ_LIST_FILES,
        GQ_LIST_PUBLISHED_EDGES, GQ_LIST_SESSION_FOLLOWS, GQ_LIST_SESSIONS, GQ_LIST_SYMBOL_EDGES,
        GQ_LIST_SYMBOLS, GQ_UPSERT_ATOM, RECENT_ATOMS_LIMIT,
    },
    transfer::parse_ndjson_records,
};
use crate::{
    HarnessError, Result,
    store::{self, SessionStore},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex, OnceLock},
    time::{Instant, UNIX_EPOCH},
};

#[derive(Default)]
struct CodeGraphOverlay {
    ir_edges: Vec<Value>,
    ir_semantic_edges: Vec<Value>,
    architecture_violations: Vec<Value>,
    blast_markers: Vec<Value>,
}

const MAX_OVERLAY_IR_EDGES: usize = 250;
const MAX_OVERLAY_SEMANTIC_EDGES: usize = 250;
const MAX_OVERLAY_VIOLATIONS: usize = 200;
const MAX_OVERLAY_BLAST_FILES: usize = 32;
const MAX_OVERLAY_CACHE_ENTRIES: usize = 16;

#[derive(Clone, PartialEq, Eq)]
struct OverlayCacheKey {
    state_dir: PathBuf,
    project_id: String,
    project_root: Option<PathBuf>,
}

#[derive(Clone)]
struct CachedSemOverlayRow {
    file_path: String,
    kind: String,
    target: String,
    source_symbol: String,
    evidence: String,
    value: Value,
}

#[derive(Clone)]
struct CachedIrOverlayEdge {
    source_file: String,
    target_file: String,
    source_label: String,
    target_label: String,
    relation: String,
    value: Value,
}

#[derive(Clone)]
struct CachedViolationItem {
    kind: String,
    rule_id: String,
    from: String,
    to: String,
    message: String,
    value: Value,
}

struct CachedOverlaySnapshot {
    key: OverlayCacheKey,
    write_epoch: Option<u64>,
    db_stamp: Option<(u64, i128)>,
    live_json_stamp: Option<(u64, i128)>,
    rules_stamp: Option<(u64, i128)>,
    sem_rows: Vec<CachedSemOverlayRow>,
    ir_edges: Vec<CachedIrOverlayEdge>,
    index: Option<Arc<crate::graph_map::Index>>,
    idx_file_set: BTreeSet<String>,
    cycle_files: BTreeSet<String>,
    violations: Vec<CachedViolationItem>,
    viol_counts: BTreeMap<String, usize>,
    violation_files: BTreeSet<String>,
    blast_cache: StdMutex<HashMap<String, Option<Value>>>,
}

fn overlay_snapshot_cache() -> &'static StdMutex<Vec<Arc<CachedOverlaySnapshot>>> {
    static CACHE: OnceLock<StdMutex<Vec<Arc<CachedOverlaySnapshot>>>> = OnceLock::new();
    CACHE.get_or_init(|| StdMutex::new(Vec::new()))
}

fn file_mtime_len_stamp(path: &Path) -> Option<(u64, i128)> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    let ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as i128);
    Some((meta.len(), ns))
}

fn compute_overlay_violations(
    idx: Option<&Arc<crate::graph_map::Index>>,
    idx_file_set: &BTreeSet<String>,
    project_root: Option<&Path>,
) -> (
    Vec<CachedViolationItem>,
    BTreeMap<String, usize>,
    BTreeSet<String>,
) {
    let mut violations = Vec::new();
    let mut viol_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut violation_files: BTreeSet<String> = BTreeSet::new();
    let (Some(idx), Some(root)) = (idx, project_root) else {
        return (violations, viol_counts, violation_files);
    };
    if let Ok(Some(rules)) = crate::graph_map::layer_rules::Rules::read(root) {
        for v in idx.architecture_violations("", true, &rules) {
            if !v.from.is_empty() && idx_file_set.contains(&v.from) {
                *viol_counts.entry(v.from.clone()).or_insert(0) += 1;
                violation_files.insert(v.from.clone());
            }
            if matches!(v.kind.as_str(), "layer" | "package")
                && !v.to.is_empty()
                && v.to != v.from
                && idx_file_set.contains(&v.to)
            {
                *viol_counts.entry(v.to.clone()).or_insert(0) += 1;
                violation_files.insert(v.to.clone());
            }
            if let Ok(val) = serde_json::to_value(&v) {
                violations.push(CachedViolationItem {
                    kind: v.kind,
                    rule_id: v.rule_id.unwrap_or_default(),
                    from: v.from,
                    to: v.to,
                    message: v.message,
                    value: val,
                });
            }
        }
    }
    (violations, viol_counts, violation_files)
}

fn matches_optional_query(qf: Option<&str>, fields: &[&str]) -> bool {
    let Some(q) = qf else {
        return true;
    };
    fields
        .iter()
        .any(|f| !f.is_empty() && f.to_ascii_lowercase().contains(q))
}

fn is_semantic_relation(rel: &str) -> bool {
    rel.starts_with("sem:")
        || matches!(
            rel,
            "exposes_route"
                | "calls_route"
                | "publishes_event"
                | "handles_event"
                | "reads_table"
                | "writes_table"
                | "di_binds"
                | "di_binding"
        )
}

fn collect_code_graph_overlay(
    state_dir: &Path,
    project_id: &str,
    project_root: Option<&Path>,
    kept_files: &BTreeSet<String>,
    q_filter: Option<&str>,
) -> CodeGraphOverlay {
    collect_code_graph_overlay_with_hit(state_dir, project_id, project_root, kept_files, q_filter).0
}

fn collect_code_graph_overlay_with_hit(
    state_dir: &Path,
    project_id: &str,
    project_root: Option<&Path>,
    kept_files: &BTreeSet<String>,
    q_filter: Option<&str>,
) -> (CodeGraphOverlay, bool) {
    let canon_state = std::fs::canonicalize(state_dir).unwrap_or_else(|_| state_dir.to_path_buf());
    let canon_root =
        project_root.map(|r| std::fs::canonicalize(r).unwrap_or_else(|_| r.to_path_buf()));
    let cache_key = OverlayCacheKey {
        state_dir: canon_state,
        project_id: project_id.to_owned(),
        project_root: canon_root,
    };

    let ir_opt = crate::graph_ir::GraphIrStore::open_readonly_cached(state_dir, project_id)
        .ok()
        .flatten();
    let write_epoch = ir_opt.as_ref().map(|ir| ir.current_write_epoch());
    let db_stamp = ir_opt
        .as_ref()
        .and_then(|ir| file_mtime_len_stamp(ir.db_path()));
    let live_path = state_dir
        .join("graphs")
        .join(project_id)
        .join("live")
        .join("graph.json");
    let live_json_stamp = if ir_opt.is_none() {
        file_mtime_len_stamp(&live_path)
    } else {
        None
    };
    let rules_stamp = project_root
        .and_then(|r| file_mtime_len_stamp(&r.join(crate::graph_map::layer_rules::FILE)));

    let cached_hit = if let Ok(mut guard) = overlay_snapshot_cache().lock()
        && let Some(pos) = guard.iter().position(|entry| {
            entry.key == cache_key
                && entry.write_epoch == write_epoch
                && entry.db_stamp == db_stamp
                && entry.live_json_stamp == live_json_stamp
        }) {
        let entry = guard.remove(pos);
        let cloned = Arc::clone(&entry);
        guard.push(entry);
        Some(cloned)
    } else {
        None
    };

    let (snapshot, index_cache_hit) = match cached_hit {
        Some(snap) if snap.rules_stamp == rules_stamp => (snap, true),
        Some(snap) => {
            // Only `.harness-architecture.json` changed: reuse the cached `Index`, `ir_edges`,
            // `sem_rows`, and `cycle_files` while refreshing only rule violations.
            let (violations, viol_counts, violation_files) =
                compute_overlay_violations(snap.index.as_ref(), &snap.idx_file_set, project_root);
            let updated = Arc::new(CachedOverlaySnapshot {
                key: cache_key.clone(),
                write_epoch,
                db_stamp,
                live_json_stamp,
                rules_stamp,
                sem_rows: snap.sem_rows.clone(),
                ir_edges: snap.ir_edges.clone(),
                index: snap.index.clone(),
                idx_file_set: snap.idx_file_set.clone(),
                cycle_files: snap.cycle_files.clone(),
                violations,
                viol_counts,
                violation_files,
                blast_cache: StdMutex::new(HashMap::new()),
            });
            if let Ok(mut guard) = overlay_snapshot_cache().lock() {
                guard.retain(|entry| entry.key != cache_key);
                if guard.len() >= MAX_OVERLAY_CACHE_ENTRIES {
                    guard.remove(0);
                }
                guard.push(Arc::clone(&updated));
            }
            (updated, true)
        }
        None => {
            let mut sem_rows = Vec::new();
            let mut ir_edges = Vec::new();
            let mut ir_nodes_links = None;
            let mut ir_read_failed = false;
            if let Some(ref ir) = ir_opt {
                match ir.semantic_edges_for_system() {
                    Ok((_, raw_sem_rows)) => {
                        for (idx, (file_path, kind, target, src_sym, line, evidence)) in
                            raw_sem_rows.into_iter().enumerate()
                        {
                            let src_sym_str = src_sym.as_deref().unwrap_or("").to_owned();
                            let value = json!({
                                "id": format!("sem:{file_path}:{kind}:{target}:{line}:{idx}"),
                                "file_path": file_path,
                                "source": file_path,
                                "source_file": file_path,
                                "source_symbol": src_sym,
                                "target": target,
                                "kind": kind,
                                "relation": kind,
                                "line": line,
                                "evidence": evidence,
                            });
                            sem_rows.push(CachedSemOverlayRow {
                                file_path,
                                kind,
                                target,
                                source_symbol: src_sym_str,
                                evidence,
                                value,
                            });
                        }
                    }
                    Err(_) => {
                        ir_read_failed = true;
                    }
                }
                match ir.export_ir_graph_elements() {
                    Ok((nodes, links, _, _)) => {
                        let node_by_id: HashMap<&str, &crate::graph_ir::IrNodeSpec> =
                            nodes.iter().map(|n| (n.id.as_str(), n)).collect();
                        let mut seen_edges = BTreeSet::new();
                        for link in &links {
                            let rel = link.relation.as_deref().unwrap_or("depends_on");
                            if is_semantic_relation(rel) {
                                continue;
                            }
                            let src_node = node_by_id.get(link.source.as_str()).copied();
                            let dst_node = node_by_id.get(link.target.as_str()).copied();
                            let src_file = src_node
                                .map(|n| n.source_file.as_str())
                                .or(link.source_file.as_deref())
                                .unwrap_or("");
                            let dst_file = dst_node.map(|n| n.source_file.as_str()).unwrap_or("");
                            let src_label = src_node
                                .map(|n| n.label.as_str())
                                .unwrap_or(link.source.as_str());
                            let dst_label = dst_node
                                .map(|n| n.label.as_str())
                                .unwrap_or(link.target.as_str());
                            if !seen_edges.insert((
                                link.source.clone(),
                                link.target.clone(),
                                rel.to_owned(),
                            )) {
                                continue;
                            }
                            let intra_file = !src_file.is_empty() && src_file == dst_file;
                            let value = json!({
                                "id": format!("ir:{}->{}:{}", link.source, link.target, rel),
                                "source": link.source,
                                "target": link.target,
                                "from": link.source,
                                "to": link.target,
                                "source_file": src_file,
                                "target_file": dst_file,
                                "source_label": src_label,
                                "target_label": dst_label,
                                "kind": rel,
                                "relation": rel,
                                "confidence": link.confidence,
                                "intra_file": intra_file,
                            });
                            ir_edges.push(CachedIrOverlayEdge {
                                source_file: src_file.to_owned(),
                                target_file: dst_file.to_owned(),
                                source_label: src_label.to_owned(),
                                target_label: dst_label.to_owned(),
                                relation: rel.to_owned(),
                                value,
                            });
                        }
                        ir_nodes_links = Some((nodes, links));
                    }
                    Err(_) => {
                        ir_read_failed = true;
                    }
                }
            }

            let idx_opt = project_root.and_then(|root| {
                if let (Some(ir), Some((nodes, links))) = (ir_opt.as_ref(), ir_nodes_links) {
                    crate::graph_map::Index::from_ir_parts(
                        root,
                        Some(ir.db_path().to_path_buf()),
                        Some("ir".to_owned()),
                        nodes,
                        links,
                    )
                    .ok()
                    .map(Arc::new)
                } else {
                    std::fs::File::open(&live_path)
                        .ok()
                        .and_then(|file| crate::graph_map::Index::read(file, root).ok())
                        .map(Arc::new)
                }
            });

            let idx_file_set: BTreeSet<String> = idx_opt
                .as_ref()
                .map(|idx| idx.files().iter().cloned().collect())
                .unwrap_or_default();
            let cycle_files: BTreeSet<String> = idx_opt
                .as_ref()
                .map(|idx| {
                    idx.scc_groups(true)
                        .into_iter()
                        .flatten()
                        .filter(|p| idx_file_set.contains(p))
                        .collect()
                })
                .unwrap_or_default();
            let (violations, viol_counts, violation_files) =
                compute_overlay_violations(idx_opt.as_ref(), &idx_file_set, project_root);

            let built = Arc::new(CachedOverlaySnapshot {
                key: cache_key.clone(),
                write_epoch,
                db_stamp,
                live_json_stamp,
                rules_stamp,
                sem_rows,
                ir_edges,
                index: idx_opt,
                idx_file_set,
                cycle_files,
                violations,
                viol_counts,
                violation_files,
                blast_cache: StdMutex::new(HashMap::new()),
            });
            if !ir_read_failed && let Ok(mut guard) = overlay_snapshot_cache().lock() {
                guard.retain(|entry| entry.key != cache_key);
                if guard.len() >= MAX_OVERLAY_CACHE_ENTRIES {
                    guard.remove(0);
                }
                guard.push(Arc::clone(&built));
            }
            (built, false)
        }
    };

    let mut overlay = CodeGraphOverlay::default();
    {
        let mut prioritized = Vec::new();
        let mut secondary = Vec::new();
        for row in &snapshot.sem_rows {
            if !matches_optional_query(
                q_filter,
                &[
                    &row.file_path,
                    &row.kind,
                    &row.target,
                    &row.source_symbol,
                    &row.evidence,
                ],
            ) {
                continue;
            }
            if kept_files.contains(&row.file_path) {
                prioritized.push(row.value.clone());
            } else {
                secondary.push(row.value.clone());
            }
        }
        overlay.ir_semantic_edges = prioritized
            .into_iter()
            .chain(secondary)
            .take(MAX_OVERLAY_SEMANTIC_EDGES)
            .collect();
    }

    {
        let mut prioritized = Vec::new();
        let mut secondary = Vec::new();
        for edge in &snapshot.ir_edges {
            if !matches_optional_query(
                q_filter,
                &[
                    &edge.source_file,
                    &edge.target_file,
                    &edge.source_label,
                    &edge.target_label,
                    &edge.relation,
                ],
            ) {
                continue;
            }
            if (!edge.source_file.is_empty() && kept_files.contains(&edge.source_file))
                || (!edge.target_file.is_empty() && kept_files.contains(&edge.target_file))
            {
                prioritized.push(edge.value.clone());
            } else {
                secondary.push(edge.value.clone());
            }
        }
        overlay.ir_edges = prioritized
            .into_iter()
            .chain(secondary)
            .take(MAX_OVERLAY_IR_EDGES)
            .collect();
    }

    let Some(ref idx) = snapshot.index else {
        return (overlay, index_cache_hit);
    };

    for v in &snapshot.violations {
        if matches_optional_query(q_filter, &[&v.kind, &v.rule_id, &v.from, &v.to, &v.message]) {
            if overlay.architecture_violations.len() >= MAX_OVERLAY_VIOLATIONS {
                break;
            }
            overlay.architecture_violations.push(v.value.clone());
        }
    }

    let mut blast_targets: Vec<String> = Vec::new();
    let mut seen_targets: BTreeSet<String> = BTreeSet::new();
    for p in kept_files
        .iter()
        .chain(snapshot.violation_files.iter())
        .chain(snapshot.cycle_files.iter())
    {
        if matches_optional_query(q_filter, &[p]) && seen_targets.insert(p.clone()) {
            blast_targets.push(p.clone());
        }
    }
    if blast_targets.len() < MAX_OVERLAY_BLAST_FILES {
        for p in idx.files() {
            if blast_targets.len() >= MAX_OVERLAY_BLAST_FILES {
                break;
            }
            if !crate::graph_map::is_test_path(p)
                && matches_optional_query(q_filter, &[p])
                && seen_targets.insert(p.clone())
            {
                blast_targets.push(p.clone());
            }
        }
    }
    for path in blast_targets.into_iter().take(MAX_OVERLAY_BLAST_FILES) {
        let v_count = snapshot.viol_counts.get(&path).copied().unwrap_or(0);
        let in_cycle = snapshot.cycle_files.contains(&path);
        let cached_blast = snapshot
            .blast_cache
            .lock()
            .ok()
            .and_then(|guard| guard.get(&path).cloned());
        let blast_val = match cached_blast {
            Some(val) => val,
            None => {
                let computed = idx.blast(&path, true);
                if let Ok(mut guard) = snapshot.blast_cache.lock() {
                    guard.insert(path.clone(), computed.clone());
                }
                computed
            }
        };
        let blast_files = blast_val
            .as_ref()
            .and_then(|b| b["files"].as_u64())
            .unwrap_or(0);
        let blast_tests = blast_val
            .as_ref()
            .and_then(|b| b["tests"].as_u64())
            .unwrap_or(0);
        if blast_files == 0 && v_count == 0 && !in_cycle {
            continue;
        }
        let risk = if v_count > 0 || in_cycle || blast_files >= 8 {
            "high"
        } else if blast_files >= 3 {
            "medium"
        } else {
            "low"
        };
        let marked = blast_val
            .as_ref()
            .map(|b| b["marked"].clone())
            .unwrap_or(Value::Null);
        overlay.blast_markers.push(json!({
            "path": path,
            "blast_files": blast_files,
            "blast_tests": blast_tests,
            "violation_count": v_count,
            "in_cycle": in_cycle,
            "risk": risk,
            "marked": marked,
        }));
    }

    (overlay, index_cache_hit)
}

impl OmnigraphManager {
    async fn build_code_graph_overlay_async(
        &self,
        project_id: &str,
        project_root: Option<PathBuf>,
        kept_files: BTreeSet<String>,
        q_filter: Option<String>,
    ) -> CodeGraphOverlay {
        let state_dir = self.state_dir.clone();
        let pid = project_id.to_owned();
        tokio::task::spawn_blocking(move || {
            collect_code_graph_overlay(
                &state_dir,
                &pid,
                project_root.as_deref(),
                &kept_files,
                q_filter.as_deref(),
            )
        })
        .await
        .unwrap_or_default()
    }
    async fn enrich_canonical_symbol_ids_async(
        &self,
        project_id: &str,
        symbols: &mut [Value],
        symbol_edges: &mut [Value],
    ) {
        if symbols.is_empty() {
            return;
        }
        let file_paths: BTreeSet<String> = symbols
            .iter()
            .filter_map(|s| str_field(s, "file_path"))
            .collect();
        if file_paths.is_empty() {
            return;
        }
        let state_dir = self.state_dir.clone();
        let pid = project_id.to_owned();
        let Some(id_map) = tokio::task::spawn_blocking(move || {
            let ir = crate::graph_ir::GraphIrStore::open_readonly_cached(&state_dir, &pid)
                .ok()
                .flatten()?;
            ir.canonical_symbol_ids_for_files(&file_paths).ok()
        })
        .await
        .ok()
        .flatten() else {
            return;
        };
        for sym in symbols {
            if let Some(fqn) = str_field(sym, "fqn")
                && let Some(cid) = id_map.get(&fqn)
            {
                sym["symbol_id"] = json!(cid);
            }
        }
        for edge in symbol_edges {
            if let Some(fqn) = str_field(edge, "fqn")
                && let Some(cid) = id_map.get(&fqn)
            {
                edge["symbol_id"] = json!(cid);
            }
        }
    }

    /// Build full constellation graph + branch/commit DAG payload for `/api/omnigraph/graph`
    /// using default pagination (`limit = 250`, `offset = 0`).
    pub async fn graph_view_json(
        &self,
        project_id: &str,
        branch: Option<&str>,
        snapshot: Option<&str>,
        kind: Option<&str>,
        q: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<Value> {
        self.graph_view_json_paginated(
            project_id, branch, snapshot, kind, q, session_id, None, None, None,
        )
        .await
    }

    /// Build constellation graph + branch/commit DAG payload for `/api/omnigraph/graph`
    /// with optional `limit` (`1..=1000`, default `250`) and `offset` / `cursor` pagination,
    /// `SymbolRef` / `MentionsSymbol` exposure, and latency/truncation telemetry.
    #[expect(
        clippy::too_many_arguments,
        reason = "Mirrors the /api/omnigraph/graph query parameter contract."
    )]
    pub async fn graph_view_json_paginated(
        &self,
        project_id: &str,
        branch: Option<&str>,
        snapshot: Option<&str>,
        kind: Option<&str>,
        q: Option<&str>,
        session_id: Option<&str>,
        limit: Option<usize>,
        offset: Option<usize>,
        cursor: Option<&str>,
    ) -> Result<Value> {
        let started = Instant::now();
        let cfg = self.config().await;
        let active_branch = clean_opt(branch).unwrap_or("main").to_string();
        let snapshot_commit = clean_opt(snapshot).map(str::to_string);
        let effective_limit = limit
            .unwrap_or(DEFAULT_GRAPH_ATOM_LIMIT)
            .clamp(1, MAX_GRAPH_ATOM_LIMIT);
        let parsed_cursor = parse_graph_cursor(cursor)?;
        let effective_offset = offset.or(parsed_cursor).unwrap_or(0);

        let Ok(client) = self.client().await else {
            return Ok(empty_graph_view(
                &cfg.graph_id,
                &active_branch,
                snapshot_commit,
                effective_limit,
                effective_offset,
                started.elapsed().as_millis() as u64,
            ));
        };

        let target_branch = if snapshot_commit.is_some() {
            None
        } else {
            Some(active_branch.clone())
        };
        let params = json!({ "project_id": project_id });
        let branch_ref = target_branch.as_deref();
        let snap_ref = snapshot_commit.as_deref();
        let kind_filter = clean_opt(kind).map(str::to_ascii_lowercase);
        let q_filter = clean_opt(q).map(str::to_ascii_lowercase);
        let sid_filter = clean_opt(session_id);
        let need_session_filter_edges = sid_filter.is_some();
        let need_q_filter_edges = q_filter.is_some();

        let commits_fut = fetch_commits_with_digests(&client, &self.commit_cache, &active_branch);
        let atoms_fut = run_recent_atoms_query(
            &client,
            project_id,
            target_branch.clone(),
            snapshot_commit.clone(),
        );
        let sessions_fut = query_edges_safe(
            &client,
            GQ_LIST_SESSIONS,
            "list_sessions",
            &params,
            branch_ref,
            snap_ref,
        );
        let pub_stage1_fut = async {
            if need_session_filter_edges {
                Some(
                    query_edges_safe(
                        &client,
                        GQ_LIST_PUBLISHED_EDGES,
                        "list_published_edges",
                        &params,
                        branch_ref,
                        snap_ref,
                    )
                    .await,
                )
            } else {
                None
            }
        };
        let con_stage1_fut = async {
            if need_session_filter_edges {
                Some(
                    query_edges_safe(
                        &client,
                        GQ_LIST_CONSUMED_EDGES,
                        "list_consumed_edges",
                        &params,
                        branch_ref,
                        snap_ref,
                    )
                    .await,
                )
            } else {
                None
            }
        };
        let file_edges_stage1_fut = async {
            if need_q_filter_edges {
                Some(
                    query_edges_safe(
                        &client,
                        GQ_LIST_FILE_EDGES,
                        "list_file_edges",
                        &params,
                        branch_ref,
                        snap_ref,
                    )
                    .await,
                )
            } else {
                None
            }
        };
        let symbol_edges_stage1_fut = async {
            if need_q_filter_edges {
                Some(
                    query_edges_safe(
                        &client,
                        GQ_LIST_SYMBOL_EDGES,
                        "list_symbol_edges",
                        &params,
                        branch_ref,
                        snap_ref,
                    )
                    .await,
                )
            } else {
                None
            }
        };

        let (
            branch_res,
            (commits_resp, commit_digests),
            atoms_res,
            session_rows,
            pub_edges_stage1,
            con_edges_stage1,
            file_edges_stage1,
            symbol_edges_stage1,
        ) = tokio::join!(
            client.list_branches(true),
            commits_fut,
            atoms_fut,
            sessions_fut,
            pub_stage1_fut,
            con_stage1_fut,
            file_edges_stage1_fut,
            symbol_edges_stage1_fut,
        );

        let Ok(branch_list) = branch_res else {
            return Ok(empty_graph_view(
                &cfg.graph_id,
                &active_branch,
                snapshot_commit,
                effective_limit,
                effective_offset,
                started.elapsed().as_millis() as u64,
            ));
        };
        let atoms_res = atoms_res?;

        let branches_json = build_branches_json(&branch_list);
        let commits_json: Vec<Value> = commits_resp
            .commits
            .iter()
            .zip(commit_digests.iter())
            .map(|(c, digest)| commit_to_graph_json(c, &active_branch, digest))
            .collect();

        let head_commit = atoms_res.graph_commit_id.clone().or_else(|| {
            branch_list
                .details
                .as_ref()
                .and_then(|ds| ds.iter().find(|d| d.name == active_branch))
                .map(|d| d.head.graph_commit_id.clone())
        });

        let (raw_atom_rows, backend_capped) = if atoms_res.rows.len() >= RECENT_ATOMS_LIMIT {
            expand_capped_graph_atoms(
                &client,
                project_id,
                &active_branch,
                snapshot_commit.as_deref(),
                atoms_res.rows,
            )
            .await
        } else {
            (atoms_res.rows, false)
        };

        let mut files_by_atom = file_edges_stage1
            .as_deref()
            .map(|edges| group_edge_targets_by_source(edges, None))
            .unwrap_or_default();
        let mut symbols_by_atom_all = symbol_edges_stage1
            .as_deref()
            .map(|edges| group_edge_targets_by_source(edges, None))
            .unwrap_or_default();

        let mut session_linked_atoms = BTreeSet::new();
        if let Some(sid) = sid_filter {
            for e in pub_edges_stage1
                .iter()
                .flatten()
                .chain(con_edges_stage1.iter().flatten())
            {
                if str_field(e, "target_id").as_deref() == Some(sid)
                    && let Some(aid) = str_field(e, "source_id")
                {
                    session_linked_atoms.insert(aid);
                }
            }
        }

        let mut filtered_atoms = Vec::new();
        for row in &raw_atom_rows {
            let mut atom = normalize_atom_row(row);
            let aid = str_field(&atom, "atom_id").unwrap_or_default();
            if file_edges_stage1.is_some() {
                atom["files"] = files_by_atom
                    .get(&aid)
                    .map(|files| json!(files))
                    .unwrap_or_else(|| json!([]));
            }
            if symbol_edges_stage1.is_some() {
                atom["symbols"] = symbols_by_atom_all
                    .get(&aid)
                    .map(|syms| json!(syms))
                    .unwrap_or_else(|| json!([]));
            }
            if let Some(ref kf) = kind_filter
                && atom["kind"]
                    .as_str()
                    .map(str::to_ascii_lowercase)
                    .as_deref()
                    != Some(kf.as_str())
            {
                continue;
            }
            if let Some(sid) = sid_filter
                && atom["source_session_id"].as_str() != Some(sid)
                && !session_linked_atoms.contains(&aid)
            {
                continue;
            }
            if let Some(ref qf) = q_filter
                && !atom_matches_query(&atom, qf)
            {
                continue;
            }
            filtered_atoms.push(atom);
        }

        let total_atoms = filtered_atoms.len();
        let page_start = effective_offset.min(total_atoms);
        let page_end = page_start.saturating_add(effective_limit).min(total_atoms);
        let mut atoms = filtered_atoms[page_start..page_end].to_vec();
        let kept_atom_ids: BTreeSet<String> = atoms
            .iter()
            .filter_map(|atom| str_field(atom, "atom_id"))
            .collect();
        let has_more = page_end < total_atoms || backend_capped;
        let truncated = has_more || atoms.len() < total_atoms || backend_capped;
        let next_cursor = has_more.then(|| page_end.to_string());

        let has_filters = kind_filter.is_some() || q_filter.is_some() || sid_filter.is_some();
        let is_paginated_subset = page_start > 0 || has_more;
        let need_unlinked_view = !has_filters && !is_paginated_subset;
        let allow_symbol_q_match = kind_filter.is_none() && sid_filter.is_none();
        let need_edges_for_kept = !kept_atom_ids.is_empty();
        let need_symbols_query = need_unlinked_view || (allow_symbol_q_match && q_filter.is_some());
        let need_follows_query = !session_rows.is_empty()
            && !(sid_filter.is_some()
                && kept_atom_ids.is_empty()
                && !session_rows
                    .iter()
                    .any(|s| str_field(s, "session_id").as_deref() == sid_filter));

        let (
            pub_edges_stage2,
            con_edges_stage2,
            file_edges_stage2,
            symbol_edges_stage2,
            rel_edges,
            follow_edges,
            symbol_rows,
        ) = tokio::join!(
            async {
                if pub_edges_stage1.is_none() && need_edges_for_kept {
                    query_edges_safe(
                        &client,
                        GQ_LIST_PUBLISHED_EDGES,
                        "list_published_edges",
                        &params,
                        branch_ref,
                        snap_ref,
                    )
                    .await
                } else {
                    Vec::new()
                }
            },
            async {
                if con_edges_stage1.is_none() && need_edges_for_kept {
                    query_edges_safe(
                        &client,
                        GQ_LIST_CONSUMED_EDGES,
                        "list_consumed_edges",
                        &params,
                        branch_ref,
                        snap_ref,
                    )
                    .await
                } else {
                    Vec::new()
                }
            },
            async {
                if file_edges_stage1.is_none() && need_edges_for_kept {
                    query_edges_safe(
                        &client,
                        GQ_LIST_FILE_EDGES,
                        "list_file_edges",
                        &params,
                        branch_ref,
                        snap_ref,
                    )
                    .await
                } else {
                    Vec::new()
                }
            },
            async {
                if symbol_edges_stage1.is_none() && need_edges_for_kept {
                    query_edges_safe(
                        &client,
                        GQ_LIST_SYMBOL_EDGES,
                        "list_symbol_edges",
                        &params,
                        branch_ref,
                        snap_ref,
                    )
                    .await
                } else {
                    Vec::new()
                }
            },
            async {
                if need_edges_for_kept {
                    query_edges_safe(
                        &client,
                        GQ_LIST_ATOM_RELATIONS,
                        "list_atom_relations",
                        &params,
                        branch_ref,
                        snap_ref,
                    )
                    .await
                } else {
                    Vec::new()
                }
            },
            async {
                if need_follows_query {
                    query_edges_safe(
                        &client,
                        GQ_LIST_SESSION_FOLLOWS,
                        "list_session_follows",
                        &params,
                        branch_ref,
                        snap_ref,
                    )
                    .await
                } else {
                    Vec::new()
                }
            },
            async {
                if need_symbols_query {
                    query_edges_safe(
                        &client,
                        GQ_LIST_SYMBOLS,
                        "list_symbols",
                        &params,
                        branch_ref,
                        snap_ref,
                    )
                    .await
                } else {
                    Vec::new()
                }
            },
        );

        let pub_edges = pub_edges_stage1.unwrap_or(pub_edges_stage2);
        let con_edges = con_edges_stage1.unwrap_or(con_edges_stage2);
        let file_edges_fetched = file_edges_stage1.is_none();
        let symbol_edges_fetched = symbol_edges_stage1.is_none();
        let file_edges = file_edges_stage1.unwrap_or(file_edges_stage2);
        let symbol_edge_rows = symbol_edges_stage1.unwrap_or(symbol_edges_stage2);

        let has_kept_edge_files = (need_edges_for_kept
            && (file_edges
                .iter()
                .any(|e| str_field(e, "source_id").is_some_and(|s| kept_atom_ids.contains(&s)))
                || symbol_edge_rows.iter().any(|e| {
                    str_field(e, "source_id").is_some_and(|s| kept_atom_ids.contains(&s))
                })))
            || !symbol_rows.is_empty();
        let need_files_query = need_unlinked_view
            || (allow_symbol_q_match && q_filter.is_some())
            || has_kept_edge_files;
        let file_rows = if need_files_query {
            query_edges_safe(
                &client,
                GQ_LIST_FILES,
                "list_files",
                &params,
                branch_ref,
                snap_ref,
            )
            .await
        } else {
            Vec::new()
        };

        if file_edges_fetched || symbol_edges_fetched {
            if file_edges_fetched {
                files_by_atom = group_edge_targets_by_source(&file_edges, Some(&kept_atom_ids));
            }
            if symbol_edges_fetched {
                symbols_by_atom_all =
                    group_edge_targets_by_source(&symbol_edge_rows, Some(&kept_atom_ids));
            }
            for atom in &mut atoms {
                let aid = str_field(atom, "atom_id").unwrap_or_default();
                if file_edges_fetched {
                    atom["files"] = files_by_atom
                        .get(&aid)
                        .map(|files| json!(files))
                        .unwrap_or_else(|| json!([]));
                }
                if symbol_edges_fetched {
                    atom["symbols"] = symbols_by_atom_all
                        .get(&aid)
                        .map(|syms| json!(syms))
                        .unwrap_or_else(|| json!([]));
                }
            }
        }

        let mut kept_session_ids_filter: Option<BTreeSet<String>> = sid_filter.map(|sid| {
            let mut keep = BTreeSet::new();
            keep.insert(sid.to_owned());
            for atom in &atoms {
                if let Some(author_sid) = str_field(atom, "source_session_id") {
                    keep.insert(author_sid);
                }
            }
            for e in pub_edges.iter().chain(con_edges.iter()) {
                if let (Some(src), Some(dst)) =
                    (str_field(e, "source_id"), str_field(e, "target_id"))
                    && kept_atom_ids.contains(&src)
                {
                    keep.insert(dst);
                }
            }
            for e in &follow_edges {
                if let (Some(src), Some(dst)) =
                    (str_field(e, "source_id"), str_field(e, "target_id"))
                    && (src == sid || dst == sid)
                {
                    keep.insert(src);
                    keep.insert(dst);
                }
            }
            keep
        });
        let filtered_sessions: Vec<Value> = session_rows
            .iter()
            .filter(|row| {
                kept_session_ids_filter.as_mut().is_none_or(|keep| {
                    str_field(row, "session_id").is_some_and(|sid| keep.contains(&sid))
                })
            })
            .map(normalize_session_row)
            .collect();
        let session_meta_by_id = self
            .store
            .as_ref()
            .map(|store| {
                let unique_sids: BTreeSet<String> = filtered_sessions
                    .iter()
                    .filter_map(|obj| str_field(obj, "session_id"))
                    .collect();
                load_session_omnigraph_metadata_batch(store, &unique_sids)
            })
            .unwrap_or_default();
        let sessions: Vec<Value> = filtered_sessions
            .into_iter()
            .map(|mut obj| {
                if let Some(sid) = str_field(&obj, "session_id")
                    && let Some(og) = session_meta_by_id.get(&sid)
                {
                    if let Some(space) = og["shared_space"].as_str().filter(|s| !s.is_empty()) {
                        obj["shared_space"] = json!(space);
                    }
                    if og["linked_session_ids"].is_array() {
                        obj["linked_session_ids"] = og["linked_session_ids"].clone();
                    }
                }
                obj
            })
            .collect();

        let (mut symbols, mut symbol_edges) = build_symbol_graph_entries(
            project_id,
            &symbol_rows,
            &symbol_edge_rows,
            &kept_atom_ids,
            if has_filters || is_paginated_subset {
                q_filter.as_deref()
            } else {
                None
            },
            need_unlinked_view,
            allow_symbol_q_match,
        );
        self.enrich_canonical_symbol_ids_async(project_id, &mut symbols, &mut symbol_edges)
            .await;

        let kept_file_paths: BTreeSet<String> = if need_unlinked_view {
            BTreeSet::new()
        } else {
            let mut paths = BTreeSet::new();
            for atom in &atoms {
                for f in atom["files"].as_array().into_iter().flatten() {
                    if let Some(p) = f.as_str().filter(|p| !p.is_empty()) {
                        paths.insert(p.to_owned());
                    }
                }
            }
            for sym in &symbols {
                if let Some(p) = sym["file_path"].as_str().filter(|p| !p.is_empty()) {
                    paths.insert(p.to_owned());
                }
            }
            paths
        };
        let files: Vec<Value> = file_rows
            .iter()
            .filter(|row| {
                if need_unlinked_view {
                    return true;
                }
                let Some(path) = str_field(row, "path") else {
                    return false;
                };
                kept_file_paths.contains(&path)
                    || (allow_symbol_q_match
                        && q_filter
                            .as_deref()
                            .is_some_and(|qf| path.to_ascii_lowercase().contains(qf)))
            })
            .map(normalize_file_row)
            .collect();

        let mut edges = build_graph_edges(
            &pub_edges,
            &con_edges,
            &file_edges,
            &rel_edges,
            &follow_edges,
            &kept_atom_ids,
            sid_filter,
        );
        edges.extend(symbol_edges.iter().cloned());

        let mut overlay_files: BTreeSet<String> = kept_file_paths.clone();
        for f in &files {
            if let Some(p) = str_field(f, "path") {
                overlay_files.insert(p);
            }
        }
        let store_root = self
            .store
            .as_ref()
            .and_then(|s| s.project(project_id).ok())
            .and_then(|v| v["path"].as_str().map(PathBuf::from));
        let overlay = self
            .build_code_graph_overlay_async(project_id, store_root, overlay_files, q_filter.clone())
            .await;

        let elapsed_ms = started.elapsed().as_millis() as u64;
        let telemetry = json!({
            "elapsed_ms": elapsed_ms,
            "atom_count": atoms.len(),
            "total_atoms": total_atoms,
            "session_count": sessions.len(),
            "file_count": files.len(),
            "symbol_count": symbols.len(),
            "edge_count": edges.len(),
            "symbol_edge_count": symbol_edges.len(),
            "ir_edge_count": overlay.ir_edges.len(),
            "ir_semantic_edge_count": overlay.ir_semantic_edges.len(),
            "violation_count": overlay.architecture_violations.len(),
            "blast_count": overlay.blast_markers.len(),
            "commit_count": commits_json.len(),
            "truncated": truncated,
            "has_more": has_more,
        });

        Ok(json!({
            "connected": true,
            "graph_id": cfg.graph_id,
            "active_branch": active_branch,
            "snapshot_commit": snapshot_commit,
            "head_commit": head_commit,
            "branches": branches_json,
            "commits": commits_json,
            "sessions": sessions,
            "atoms": atoms,
            "files": files,
            "symbols": symbols,
            "edges": edges,
            "symbol_edges": symbol_edges,
            "ir_edges": overlay.ir_edges,
            "ir_semantic_edges": overlay.ir_semantic_edges,
            "architecture_violations": overlay.architecture_violations,
            "blast_markers": overlay.blast_markers,
            "total_atoms": total_atoms,
            "limit": effective_limit,
            "offset": page_start,
            "has_more": has_more,
            "truncated": truncated,
            "next_cursor": next_cursor,
            "telemetry": telemetry,
        }))
    }

    /// Hybrid BM25 + fallback ranking search returning `{ query, branch, hits, subgraph }`.
    pub async fn search_json(
        &self,
        project_id: &str,
        q: &str,
        branch: Option<&str>,
        kind: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Value> {
        let target_branch = clean_opt(branch).unwrap_or("main").to_string();
        let limit = limit.unwrap_or(25).clamp(1, 100);
        let client = self.client().await?;
        let kind_filter = clean_opt(kind).map(str::to_ascii_lowercase);
        let q_trimmed = q.trim();

        let matches_kind = |atom: &Value| -> bool {
            let Some(ref kf) = kind_filter else {
                return true;
            };
            atom["kind"]
                .as_str()
                .map(str::to_ascii_lowercase)
                .as_deref()
                == Some(kf.as_str())
        };

        let mut raw_hits: Vec<(Value, f64, &'static str)> = Vec::new();
        if !q_trimmed.is_empty() {
            if let Some(ref kf) = kind_filter
                && let Ok(resp) = client
                    .query(&QueryRequest {
                        query: GQ_CONTEXT_SEARCH_BY_KIND.into(),
                        name: Some("context_search_by_kind".into()),
                        params: Some(json!({
                            "q": q_trimmed,
                            "project_id": project_id,
                            "kind": kf,
                        })),
                        branch: Some(target_branch.clone()),
                        snapshot: None,
                        settings: None,
                    })
                    .await
            {
                for row in &resp.rows {
                    let atom = normalize_atom_row(row);
                    if matches_kind(&atom) {
                        let score = row["score"].as_f64().unwrap_or(0.5);
                        raw_hits.push((atom, score, "bm25"));
                    }
                }
            }

            if raw_hits.is_empty() {
                let mut search_params = json!({ "q": q_trimmed, "project_id": project_id });
                if let Some(ref kf) = kind_filter {
                    search_params["kind"] = json!(kf);
                }
                if let Ok(resp) = run_stored_or_inline(
                    &client,
                    "context_search",
                    schema::QUERY_CONTEXT_SEARCH,
                    search_params,
                    Some(target_branch.clone()),
                    None,
                )
                .await
                {
                    for row in &resp.rows {
                        let atom = normalize_atom_row(row);
                        if matches_kind(&atom) {
                            let score = row["score"].as_f64().unwrap_or(0.5);
                            raw_hits.push((atom, score, "bm25"));
                        }
                    }
                }
            }
        }

        let mut cached_recent_rows: Option<Vec<Value>> = None;
        if raw_hits.is_empty() {
            let recent =
                run_recent_atoms_query(&client, project_id, Some(target_branch.clone()), None)
                    .await?;
            let (rows, _) = if recent.rows.len() >= RECENT_ATOMS_LIMIT {
                expand_capped_graph_atoms(&client, project_id, &target_branch, None, recent.rows)
                    .await
            } else {
                (recent.rows, false)
            };
            let q_lower = q_trimmed.to_ascii_lowercase();
            for row in &rows {
                let atom = normalize_atom_row(row);
                if !matches_kind(&atom) {
                    continue;
                }
                let score = if q_lower.is_empty() {
                    atom["confidence"].as_f64().unwrap_or(0.5)
                } else {
                    score_atom_fallback(&atom, &q_lower)
                };
                if q_lower.is_empty() || score > 0.0 {
                    raw_hits.push((
                        atom,
                        score,
                        if q_lower.is_empty() {
                            "pinned"
                        } else {
                            "lexical"
                        },
                    ));
                }
            }
            raw_hits.sort_by(|a, b| b.1.total_cmp(&a.1));
            cached_recent_rows = Some(rows);
        }

        raw_hits.truncate(limit);
        if raw_hits.is_empty() {
            return Ok(json!({
                "query": q_trimmed,
                "branch": target_branch,
                "hits": [],
                "subgraph": {
                    "atoms": [],
                    "sessions": [],
                    "files": [],
                    "symbols": [],
                    "edges": [],
                    "symbol_edges": [],
                },
            }));
        }

        let params = json!({ "project_id": project_id });
        let branch_ref = Some(target_branch.as_str());
        let (
            file_edges,
            rel_edges,
            symbol_edge_rows,
            pub_edges,
            con_edges,
            session_rows,
            symbol_rows,
            file_rows,
        ) = tokio::join!(
            query_edges_safe(
                &client,
                GQ_LIST_FILE_EDGES,
                "list_file_edges",
                &params,
                branch_ref,
                None,
            ),
            query_edges_safe(
                &client,
                GQ_LIST_ATOM_RELATIONS,
                "list_atom_relations",
                &params,
                branch_ref,
                None,
            ),
            query_edges_safe(
                &client,
                GQ_LIST_SYMBOL_EDGES,
                "list_symbol_edges",
                &params,
                branch_ref,
                None,
            ),
            query_edges_safe(
                &client,
                GQ_LIST_PUBLISHED_EDGES,
                "list_published_edges",
                &params,
                branch_ref,
                None,
            ),
            query_edges_safe(
                &client,
                GQ_LIST_CONSUMED_EDGES,
                "list_consumed_edges",
                &params,
                branch_ref,
                None,
            ),
            query_edges_safe(
                &client,
                GQ_LIST_SESSIONS,
                "list_sessions",
                &params,
                branch_ref,
                None,
            ),
            query_edges_safe(
                &client,
                GQ_LIST_SYMBOLS,
                "list_symbols",
                &params,
                branch_ref,
                None,
            ),
            query_edges_safe(
                &client,
                GQ_LIST_FILES,
                "list_files",
                &params,
                branch_ref,
                None,
            ),
        );

        let mut hit_atom_ids = BTreeSet::new();
        let mut neighbor_atom_ids = BTreeSet::new();
        let mut hits = Vec::with_capacity(raw_hits.len());
        let mut subgraph_atoms_by_id: BTreeMap<String, Value> = BTreeMap::new();

        for (mut atom, score, matched_via) in raw_hits {
            let aid = str_field(&atom, "atom_id").unwrap_or_default();
            if !aid.is_empty() {
                hit_atom_ids.insert(aid.clone());
            }
            let mut files = Vec::new();
            let mut symbols = Vec::new();
            let mut neighbors = Vec::new();
            let mut seen_neighbors: BTreeSet<(String, String, String, String)> = BTreeSet::new();

            for fe in &file_edges {
                if str_field(fe, "source_id").as_deref() == Some(aid.as_str())
                    && let Some(path) = str_field(fe, "target_id")
                {
                    if !files.contains(&path) {
                        files.push(path.clone());
                    }
                    if seen_neighbors.insert((
                        "file".into(),
                        path.clone(),
                        "touches_file".into(),
                        "outgoing".into(),
                    )) {
                        neighbors.push(json!({
                            "kind": "file",
                            "id": path,
                            "relation": "touches_file",
                            "direction": "outgoing",
                        }));
                    }
                }
            }
            for se in &symbol_edge_rows {
                if str_field(se, "source_id").as_deref() == Some(aid.as_str())
                    && let Some(fqn) = str_field(se, "target_id")
                {
                    if !symbols.contains(&fqn) {
                        symbols.push(fqn.clone());
                    }
                    let role = str_field(se, "role").unwrap_or_else(|| "named".to_string());
                    if seen_neighbors.insert((
                        "symbol".into(),
                        fqn.clone(),
                        "mentions_symbol".into(),
                        "outgoing".into(),
                    )) {
                        neighbors.push(json!({
                            "kind": "symbol",
                            "id": fqn,
                            "relation": "mentions_symbol",
                            "direction": "outgoing",
                            "role": role,
                            "name": se.get("name").cloned().unwrap_or(Value::Null),
                            "file_path": se.get("file_path").cloned().unwrap_or(Value::Null),
                            "symbol_kind": se.get("kind").cloned().unwrap_or(Value::Null),
                        }));
                    }
                }
            }
            for re in &rel_edges {
                let rel = re["relation"].as_str().unwrap_or("relates_to").to_string();
                let src = str_field(re, "source_id");
                let dst = str_field(re, "target_id");
                if src.as_deref() == Some(aid.as_str())
                    && let Some(other) = dst
                {
                    if !other.is_empty() {
                        neighbor_atom_ids.insert(other.clone());
                    }
                    if seen_neighbors.insert((
                        "atom".into(),
                        other.clone(),
                        rel.clone(),
                        "outgoing".into(),
                    )) {
                        neighbors.push(json!({
                            "kind": "atom",
                            "id": other,
                            "relation": rel,
                            "direction": "outgoing",
                        }));
                    }
                } else if dst.as_deref() == Some(aid.as_str())
                    && let Some(other) = src
                    && other != aid
                {
                    if !other.is_empty() {
                        neighbor_atom_ids.insert(other.clone());
                    }
                    if seen_neighbors.insert((
                        "atom".into(),
                        other.clone(),
                        rel.clone(),
                        "incoming".into(),
                    )) {
                        neighbors.push(json!({
                            "kind": "atom",
                            "id": other,
                            "relation": rel,
                            "direction": "incoming",
                        }));
                    }
                }
            }
            atom["files"] = json!(files);
            atom["symbols"] = json!(symbols);
            if !aid.is_empty() {
                subgraph_atoms_by_id.insert(aid, atom.clone());
            }
            hits.push(json!({
                "atom": atom,
                "score": score,
                "matched_via": matched_via,
                "files": files,
                "symbols": symbols,
                "neighbors": neighbors,
            }));
        }

        let missing_neighbors: BTreeSet<String> = neighbor_atom_ids
            .difference(&hit_atom_ids)
            .cloned()
            .collect();
        if !missing_neighbors.is_empty() {
            let rows = match cached_recent_rows {
                Some(rows) => rows,
                None => {
                    let recent = run_recent_atoms_query(
                        &client,
                        project_id,
                        Some(target_branch.clone()),
                        None,
                    )
                    .await
                    .map(|r| r.rows)
                    .unwrap_or_default();
                    let still_missing = missing_neighbors.iter().any(|nid| {
                        !recent.iter().any(|r| {
                            str_field(r, "atom_id")
                                .or_else(|| str_field(r, "id"))
                                .as_deref()
                                == Some(nid.as_str())
                        })
                    });
                    if still_missing && recent.len() >= RECENT_ATOMS_LIMIT {
                        expand_capped_graph_atoms(&client, project_id, &target_branch, None, recent)
                            .await
                            .0
                    } else {
                        recent
                    }
                }
            };
            let files_by_atom = group_edge_targets_by_source(&file_edges, Some(&missing_neighbors));
            let symbols_by_atom =
                group_edge_targets_by_source(&symbol_edge_rows, Some(&missing_neighbors));
            for row in &rows {
                let mut atom = normalize_atom_row(row);
                if let Some(aid) = str_field(&atom, "atom_id")
                    && missing_neighbors.contains(&aid)
                    && !subgraph_atoms_by_id.contains_key(&aid)
                {
                    atom["files"] = files_by_atom
                        .get(&aid)
                        .map(|f| json!(f))
                        .unwrap_or_else(|| json!([]));
                    atom["symbols"] = symbols_by_atom
                        .get(&aid)
                        .map(|s| json!(s))
                        .unwrap_or_else(|| json!([]));
                    subgraph_atoms_by_id.insert(aid, atom);
                }
            }
        }

        let subgraph_atom_ids: BTreeSet<String> = subgraph_atoms_by_id.keys().cloned().collect();
        let subgraph_atoms: Vec<Value> = subgraph_atoms_by_id.into_values().collect();

        let (mut subgraph_symbols, mut subgraph_symbol_edges) = build_symbol_graph_entries(
            project_id,
            &symbol_rows,
            &symbol_edge_rows,
            &subgraph_atom_ids,
            None,
            false,
            false,
        );
        self.enrich_canonical_symbol_ids_async(
            project_id,
            &mut subgraph_symbols,
            &mut subgraph_symbol_edges,
        )
        .await;

        let mut kept_session_ids = BTreeSet::new();
        for atom in &subgraph_atoms {
            if let Some(sid) = str_field(atom, "source_session_id") {
                kept_session_ids.insert(sid);
            }
        }
        for e in pub_edges.iter().chain(con_edges.iter()) {
            if let (Some(src), Some(dst)) = (str_field(e, "source_id"), str_field(e, "target_id"))
                && subgraph_atom_ids.contains(&src)
            {
                kept_session_ids.insert(dst);
            }
        }
        let session_meta_by_id = self
            .store
            .as_ref()
            .map(|store| load_session_omnigraph_metadata_batch(store, &kept_session_ids))
            .unwrap_or_default();
        let subgraph_sessions: Vec<Value> = session_rows
            .iter()
            .filter(|row| {
                str_field(row, "session_id").is_some_and(|sid| kept_session_ids.contains(&sid))
            })
            .map(|row| {
                let mut obj = normalize_session_row(row);
                if let Some(sid) = str_field(&obj, "session_id")
                    && let Some(og) = session_meta_by_id.get(&sid)
                {
                    if let Some(space) = og["shared_space"].as_str().filter(|s| !s.is_empty()) {
                        obj["shared_space"] = json!(space);
                    }
                    if og["linked_session_ids"].is_array() {
                        obj["linked_session_ids"] = og["linked_session_ids"].clone();
                    }
                }
                obj
            })
            .collect();

        let mut kept_file_paths = BTreeSet::new();
        for atom in &subgraph_atoms {
            for f in atom["files"].as_array().into_iter().flatten() {
                if let Some(p) = f.as_str().filter(|p| !p.is_empty()) {
                    kept_file_paths.insert(p.to_owned());
                }
            }
        }
        for sym in &subgraph_symbols {
            if let Some(p) = str_field(sym, "file_path") {
                kept_file_paths.insert(p);
            }
        }
        let mut subgraph_files: Vec<Value> = Vec::new();
        let mut seen_file_paths = BTreeSet::new();
        for row in &file_rows {
            if let Some(path) = str_field(row, "path")
                && kept_file_paths.contains(&path)
                && seen_file_paths.insert(path)
            {
                subgraph_files.push(normalize_file_row(row));
            }
        }
        for path in kept_file_paths {
            if seen_file_paths.insert(path.clone()) {
                subgraph_files.push(json!({
                    "id": path,
                    "path": path,
                    "project_id": project_id,
                }));
            }
        }

        let mut subgraph_edges = build_graph_edges(
            &pub_edges,
            &con_edges,
            &file_edges,
            &rel_edges,
            &[],
            &subgraph_atom_ids,
            None,
        );
        subgraph_edges.extend(subgraph_symbol_edges.iter().cloned());

        Ok(json!({
            "query": q_trimmed,
            "branch": target_branch,
            "hits": hits,
            "subgraph": {
                "atoms": subgraph_atoms,
                "sessions": subgraph_sessions,
                "files": subgraph_files,
                "symbols": subgraph_symbols,
                "edges": subgraph_edges,
                "symbol_edges": subgraph_symbol_edges,
            },
        }))
    }

    /// Compare `source` branch against `target` (default `main`), including 3-way dry-run conflict checks.
    pub async fn branch_diff_json(
        &self,
        project_id: &str,
        source: &str,
        target: Option<&str>,
    ) -> Result<Value> {
        let source = source.trim();
        if source.is_empty() {
            return Err(HarnessError::invalid("`source` branch is required."));
        }
        let target = clean_opt(target).unwrap_or("main");
        let client = self.client().await?;

        let diff = client
            .branch_diff(&BranchDiffQuery {
                source: source.to_string(),
                target: Some(target.to_string()),
                limit: Some(500),
                page_token: None,
            })
            .await?;

        let mut added_atoms = Vec::new();
        let mut updated_atoms = Vec::new();
        let mut deleted_atoms = Vec::new();
        let mut added_edges = Vec::new();
        let mut deleted_edges = Vec::new();

        for ch in &diff.changes {
            match (ch.kind, ch.op) {
                (EntityKind::Node, ChangeOp::Insert) => {
                    if ch.entity_type.name == "ContextAtom" {
                        added_atoms.push(change_to_atom_entry(ch));
                    }
                }
                (EntityKind::Node, ChangeOp::Update) => {
                    if ch.entity_type.name == "ContextAtom" {
                        updated_atoms.push(change_to_atom_entry(ch));
                    }
                }
                (EntityKind::Node, ChangeOp::Delete) => {
                    if ch.entity_type.name == "ContextAtom" {
                        deleted_atoms.push(change_to_atom_entry(ch));
                    }
                }
                (EntityKind::Edge, ChangeOp::Insert | ChangeOp::Update) => {
                    added_edges.push(diff_edge_entry(ch));
                }
                (EntityKind::Edge, ChangeOp::Delete) => {
                    deleted_edges.push(diff_edge_entry(ch));
                }
            }
        }

        let (outcome, conflicts) = match client
            .merge_branches(&BranchMergeRequest {
                source: source.to_string(),
                target: Some(target.to_string()),
                delete_branch: false,
                as_commit: false,
                dry_run: true,
                settings: None,
            })
            .await
        {
            Ok(resp) => (resp.outcome.as_str().to_string(), Vec::new()),
            Err(err) if err.is_merge_conflict() => (
                "conflict".to_string(),
                conflicts_with_values(&client, project_id, source, target, &err.merge_conflicts)
                    .await,
            ),
            Err(err) => return Err(err.into()),
        };

        Ok(json!({
            "source": source,
            "target": target,
            "merge_base": diff.base_commit_id.unwrap_or(diff.from_commit_id),
            "to_commit_id": diff.to_commit_id,
            "outcome": outcome,
            "added_atoms": added_atoms,
            "updated_atoms": updated_atoms,
            "deleted_atoms": deleted_atoms,
            "added_edges": added_edges,
            "deleted_edges": deleted_edges,
            "conflicts": conflicts,
        }))
    }

    /// Create, merge, or delete an Omnigraph branch.
    pub async fn branch_action_json(&self, body: &Value) -> Result<Value> {
        let action = body["action"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| HarnessError::invalid("Missing branch `action`."))?;
        let client = self.client().await?;

        match action {
            "create" => {
                let name = store::text(&body["name"], "branch name", 128)?;
                let from = clean_opt(body["from"].as_str()).map(str::to_string);
                let from_commit = clean_opt(body["from_commit"].as_str()).map(str::to_string);
                let resp = client
                    .create_branch(&BranchCreateRequest {
                        name,
                        from,
                        from_commit,
                    })
                    .await?;
                Ok(json!({
                    "ok": true,
                    "action": "create",
                    "name": resp.name,
                    "from": resp.from,
                    "from_commit_id": resp.from_commit_id,
                    "actor_id": resp.actor_id,
                }))
            }
            "merge" => {
                let source = store::text(&body["source"], "source branch", 128)?;
                let target = clean_opt(body["target"].as_str())
                    .unwrap_or("main")
                    .to_string();
                let dry_run = body["dry_run"].as_bool().unwrap_or(false);
                let delete_branch = body["delete_branch"].as_bool().unwrap_or(false);
                let as_commit = body["as_commit"].as_bool().unwrap_or(false);

                match client
                    .merge_branches(&BranchMergeRequest {
                        source: source.clone(),
                        target: Some(target.clone()),
                        delete_branch,
                        as_commit,
                        dry_run,
                        settings: None,
                    })
                    .await
                {
                    Ok(resp) => Ok(json!({
                        "ok": true,
                        "action": "merge",
                        "source": resp.source,
                        "target": resp.target,
                        "outcome": resp.outcome.as_str(),
                        "dry_run": resp.dry_run,
                        "branch_deleted": resp.branch_deleted,
                        "commit": resp.commit,
                        "conflicts": [],
                    })),
                    Err(err) if err.is_merge_conflict() => {
                        // A conflicting merge publishes nothing, so both
                        // branches still hold the atoms in conflict.
                        let project_id = clean_opt(body["project_id"].as_str()).unwrap_or("");
                        let conflicts = conflicts_with_values(
                            &client,
                            project_id,
                            &source,
                            &target,
                            &err.merge_conflicts,
                        )
                        .await;
                        Ok(json!({
                            "ok": false,
                            "action": "merge",
                            "source": source,
                            "target": target,
                            "outcome": "conflict",
                            "dry_run": dry_run,
                            "added_atoms": [],
                            "updated_atoms": [],
                            "deleted_atoms": [],
                            "added_edges": [],
                            "deleted_edges": [],
                            "conflicts": conflicts,
                            "error": err.message,
                        }))
                    }
                    Err(err) => Err(err.into()),
                }
            }
            "keep" => {
                // Copy one atom onto `to` exactly as `from` stores it.
                // Omnigraph settles a `divergent_update` only once both
                // branches hold the same row; an ordinary update stamps its
                // own branch and time, so it would leave the conflict.
                let required = |key: &str| {
                    clean_opt(body[key].as_str())
                        .map(str::to_string)
                        .ok_or_else(|| {
                            HarnessError::invalid(format!("Keeping an atom needs `{key}`."))
                        })
                };
                let project_id = required("project_id")?;
                let atom_id = required("atom_id")?;
                let from = required("from")?;
                let to = required("to")?;
                if from == to {
                    return Err(HarnessError::invalid(
                        "Keep copies an atom from one branch to another.",
                    ));
                }
                let kept = find_atom_on_branch(&client, &project_id, &from, &atom_id)
                    .await
                    .ok_or_else(|| {
                        HarnessError::missing(format!("The atom {atom_id} is not on {from}."))
                    })?;
                // The person chose between the two versions the preview
                // showed; a session may have published a newer one on either
                // branch since, and neither is copied or overwritten unseen.
                let changed = |branch: &str| {
                    HarnessError::conflict(format!(
                        "The atom {atom_id} changed on {branch} after the merge preview. Preview the merge again."
                    ))
                };
                if let Some(expected) = clean_opt(body["expected_updated_at"].as_str())
                    && kept["updated_at"].as_str() != Some(expected)
                {
                    return Err(changed(&from));
                }
                if let Some(replaced) = clean_opt(body["replaced_updated_at"].as_str()) {
                    let current = find_atom_on_branch(&client, &project_id, &to, &atom_id).await;
                    if current
                        .as_ref()
                        .and_then(|atom| atom["updated_at"].as_str())
                        != Some(replaced)
                    {
                        return Err(changed(&to));
                    }
                }
                let params = stored_atom_params(&kept, &project_id);
                let resp = client
                    .mutate(&ChangeRequest {
                        query: GQ_UPSERT_ATOM.into(),
                        name: Some("upsert_atom".into()),
                        params: Some(params.clone()),
                        branch: Some(to.clone()),
                        settings: None,
                    })
                    .await?;
                Ok(json!({
                    "ok": true,
                    "action": "keep",
                    "atom_id": atom_id,
                    "from": from,
                    "to": to,
                    "atom": normalize_atom_row(&params),
                    "commit": resp.commit,
                }))
            }
            "delete" => {
                let name = store::text(&body["name"], "branch name", 128)?;
                if name == "main" {
                    return Err(HarnessError::invalid("Cannot delete the `main` branch."));
                }
                let resp = client.delete_branch(&name).await?;
                Ok(json!({
                    "ok": true,
                    "action": "delete",
                    "name": resp.name,
                    "actor_id": resp.actor_id,
                }))
            }
            _ => Err(HarnessError::invalid(format!(
                "Unknown branch action '{action}'."
            ))),
        }
    }

    /// Publish a new or updated `ContextAtom` (with optional linked files, session provenance, and relations).
    /// The declarations it names are read from the project the store knows
    /// for it, never from a path in the request.
    pub async fn publish_atom_json(&self, body: &Value) -> Result<Value> {
        let root = self.project_root(
            clean_opt(body["project_id"].as_str()).unwrap_or(""),
            clean_opt(body["session_id"].as_str()),
        );
        self.publish_atom_in(body, root.as_deref()).await
    }

    /// The folder a project's files are read from: the session's workspace
    /// when the session is the project's, else the registered project.
    pub(super) fn project_root(
        &self,
        project_id: &str,
        session_id: Option<&str>,
    ) -> Option<PathBuf> {
        let store = self.store.as_ref()?;
        if project_id.is_empty() {
            return None;
        }
        if let Some(session) = session_id.and_then(|sid| store.get(sid).ok())
            && session["project_id"].as_str() == Some(project_id)
            && let Some(path) = clean_opt(session["project_path"].as_str())
        {
            return Some(PathBuf::from(path));
        }
        store
            .project(project_id)
            .ok()
            .and_then(|project| clean_opt(project["path"].as_str()).map(PathBuf::from))
    }

    pub(super) async fn publish_atom_in(&self, body: &Value, root: Option<&Path>) -> Result<Value> {
        let project_id = clean_opt(body["project_id"].as_str())
            .unwrap_or("")
            .to_string();
        let branch = clean_opt(body["branch"].as_str())
            .unwrap_or("main")
            .to_string();
        let kind = clean_opt(body["kind"].as_str())
            .unwrap_or("insight")
            .to_ascii_lowercase();
        if !VALID_ATOM_KINDS.contains(&kind.as_str()) {
            return Err(HarnessError::invalid(format!(
                "Invalid ContextAtom kind '{kind}'."
            )));
        }
        let title = store::text(&body["title"], "atom title", 240)?;
        let summary = clean_opt(body["summary"].as_str())
            .map(str::to_string)
            .unwrap_or_else(|| title.clone());
        let atom_body = clean_opt(body["body"].as_str())
            .map(str::to_string)
            .unwrap_or_else(|| summary.clone());
        let atom_id = clean_opt(body["atom_id"].as_str())
            .or_else(|| clean_opt(body["id"].as_str()))
            .map(str::to_string)
            .unwrap_or_else(|| format!("atom-{}", &store::hexid()[..12]));
        let session_id = clean_opt(body["session_id"].as_str()).map(str::to_string);
        let tags = parse_tags_opt(&body["tags"]);
        let files = parse_string_list(&body["files"]);
        let confidence = body["confidence"].as_f64().unwrap_or(0.85).clamp(0.0, 1.0);
        let pin_priority = body["pin_priority"].as_i64().unwrap_or(10) as i32;
        let expected_commit = clean_opt(body["expected_commit"].as_str()).map(str::to_string);
        let ts = now_utc_iso_ms();

        let client = self.client().await?;
        // An update that leaves out a provenance field, the verification or
        // the creation time keeps the atom's own: the upsert writes every
        // field, and an edit of the text must not turn a Project Brain atom
        // into an unverified one with no source. A field sent as null clears.
        let named = clean_opt(body["atom_id"].as_str())
            .or_else(|| clean_opt(body["id"].as_str()))
            .is_some();
        let existing = if named {
            find_atom_on_branch(&client, &project_id, &branch, &atom_id).await
        } else {
            None
        };
        let mut merged = body.clone();
        if let Some(existing) = &existing
            && merged.is_object()
        {
            for key in [
                "origin_layer",
                "authority",
                "privacy",
                "brain_record_id",
                "memory_id",
                "record_revision",
                "verified",
            ] {
                if body.get(key).is_none() && !existing[key].is_null() {
                    merged[key] = existing[key].clone();
                }
            }
        }
        let verified = merged["verified"].as_bool().unwrap_or(false);
        let provenance = AtomProvenance::from_value(&merged);
        let created_at = existing
            .as_ref()
            .and_then(|atom| clean_opt(atom["created_at"].as_str()).map(str::to_string))
            .unwrap_or_else(|| ts.clone());
        // So does the session that wrote it: an update without `session_id`
        // keeps the stored one and links no session, so the Keep buttons of
        // the merge preview do not erase who published the atom.
        let source_session_id = if body.get("session_id").is_some() {
            session_id.clone()
        } else {
            existing
                .as_ref()
                .and_then(|atom| clean_opt(atom["source_session_id"].as_str()).map(str::to_string))
        };
        let atom_params = build_upsert_atom_params(
            &atom_id,
            &project_id,
            &kind,
            &title,
            &summary,
            &atom_body,
            tags,
            &branch,
            source_session_id.as_deref(),
            confidence,
            pin_priority,
            verified,
            &provenance,
            &created_at,
            &ts,
        );
        let req = ChangeRequest {
            query: GQ_UPSERT_ATOM.into(),
            name: Some("upsert_atom".into()),
            params: Some(atom_params.clone()),
            branch: Some(branch.clone()),
            settings: None,
        };
        let mut last_commit = if let Some(expected) = expected_commit.as_deref() {
            client.mutate_if_commit(&req, expected).await?.commit
        } else {
            client.mutate(&req).await?.commit
        };

        if let Some(sid) = session_id.as_deref() {
            let res = client
                .mutate(&ChangeRequest {
                    query: GQ_LINK_PUBLISHED_BY.into(),
                    name: Some("link_published_by".into()),
                    params: Some(json!({
                        "atom_id": atom_id,
                        "session_id": sid,
                        "role": "author",
                        "published_at": ts,
                    })),
                    branch: Some(branch.clone()),
                    settings: None,
                })
                .await;
            if let Ok(r) = res
                && r.commit.is_some()
            {
                last_commit = r.commit;
            }
        }

        if let Some(commit) =
            upsert_atom_files(&client, &project_id, &branch, &atom_id, &files, &ts).await
        {
            last_commit = Some(commit);
        }

        let symbols = {
            let state_dir = self.state_dir.clone();
            let pid = project_id.clone();
            let root_owned = root.map(Path::to_path_buf);
            let files_owned = files.clone();
            let sym_text = format!("{title}\n{summary}\n{atom_body}");
            tokio::task::spawn_blocking(move || {
                atom_symbols_for_project(
                    Some(&state_dir),
                    &pid,
                    root_owned.as_deref(),
                    &files_owned,
                    &sym_text,
                )
            })
            .await
            .unwrap_or_default()
        };
        let previous_symbols = if existing.is_some() {
            list_atom_symbols(&client, &project_id, &branch)
                .await
                .remove(&atom_id)
                .unwrap_or_default()
        } else {
            BTreeSet::new()
        };
        if let Some(commit) = upsert_atom_symbols(
            &client,
            &project_id,
            &branch,
            &atom_id,
            &symbols,
            &previous_symbols,
        )
        .await
        {
            last_commit = Some(commit);
        }

        if let Some(relations) = body["relates_to"].as_array() {
            for rel in relations {
                let Some((target_id, relation, note, rel_confidence)) = parse_relation_entry(rel)
                else {
                    continue;
                };
                if let Ok(r) = client
                    .mutate(&ChangeRequest {
                        query: GQ_LINK_RELATES_TO.into(),
                        name: Some("link_relates_to".into()),
                        params: Some(json!({
                            "source_id": atom_id,
                            "target_id": target_id,
                            "relation": relation,
                            "note": note,
                            "confidence": rel_confidence,
                            "created_at": ts,
                        })),
                        branch: Some(branch.clone()),
                        settings: None,
                    })
                    .await
                    && r.commit.is_some()
                {
                    last_commit = r.commit;
                }
            }
        }

        let atom_resp = normalize_atom_row(&atom_params);
        Ok(json!({
            "ok": true,
            "id": atom_id,
            "atom_id": atom_id,
            "branch": branch,
            "atom": atom_resp,
            "files": files,
            "symbols": symbols,
            "commit": last_commit,
        }))
    }

    /// Inspect commit entity changes and affected atoms (`GET /api/omnigraph/commits/{commit_id}`).
    pub async fn commit_detail_json(&self, _project_id: &str, commit_id: &str) -> Result<Value> {
        let commit_id = commit_id.trim();
        if commit_id.is_empty() {
            return Err(HarnessError::invalid("`commit_id` is required."));
        }
        let client = self.client().await?;
        match client.commit_changes(commit_id, Some(200), None).await {
            Ok(detail) => {
                let mut atoms = Vec::new();
                let mut changes = Vec::new();
                for ch in &detail.changes {
                    if ch.kind == EntityKind::Node && ch.entity_type.name == "ContextAtom" {
                        atoms.push(change_to_atom_entry(ch));
                    }
                    changes.push(enrich_commit_change(ch));
                }
                Ok(json!({
                    "commit": {
                        "graph_commit_id": detail.cause.graph_commit_id,
                        "commit_id": detail.cause.graph_commit_id,
                        "parent_commit_id": detail.cause.parent_commit_id,
                        "merged_parent_commit_id": detail.cause.merged_parent_commit_id,
                        "branch": detail.cause.authored_branch,
                        "actor_id": detail.cause.actor_id,
                        "authored_at": detail.cause.authored_at,
                        "created_at": format_omnigraph_ts(detail.cause.authored_at)
                            .unwrap_or_else(|| detail.cause.authored_at.to_string()),
                    },
                    "changes": changes,
                    "atoms": atoms,
                }))
            }
            Err(err) if err.status == 409 => Ok(json!({
                "commit": {
                    "graph_commit_id": commit_id,
                    "commit_id": commit_id,
                    "parentless": true,
                },
                "changes": [],
                "atoms": [],
            })),
            Err(err) => Err(err.into()),
        }
    }
}

fn empty_graph_view(
    graph_id: &str,
    active_branch: &str,
    snapshot_commit: Option<String>,
    limit: usize,
    offset: usize,
    elapsed_ms: u64,
) -> Value {
    json!({
        "connected": false,
        "graph_id": graph_id,
        "active_branch": active_branch,
        "snapshot_commit": snapshot_commit,
        "head_commit": Value::Null,
        "branches": [],
        "commits": [],
        "sessions": [],
        "atoms": [],
        "files": [],
        "symbols": [],
        "edges": [],
        "symbol_edges": [],
        "ir_edges": [],
        "ir_semantic_edges": [],
        "architecture_violations": [],
        "blast_markers": [],
        "total_atoms": 0,
        "limit": limit,
        "offset": offset,
        "has_more": false,
        "truncated": false,
        "next_cursor": Value::Null,
        "telemetry": {
            "elapsed_ms": elapsed_ms,
            "atom_count": 0,
            "total_atoms": 0,
            "session_count": 0,
            "file_count": 0,
            "symbol_count": 0,
            "edge_count": 0,
            "symbol_edge_count": 0,
            "ir_edge_count": 0,
            "ir_semantic_edge_count": 0,
            "violation_count": 0,
            "blast_count": 0,
            "commit_count": 0,
            "truncated": false,
            "has_more": false,
        },
    })
}

pub(super) fn now_utc_iso_ms() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn fallback_commit_digest(c: &CommitOutput) -> CommitDigest {
    let short: String = c.graph_commit_id.chars().take(12).collect();
    CommitDigest {
        added_count: 0,
        updated_count: 0,
        deleted_count: 0,
        atom_delta: 0,
        summary: format!("Commit {short}"),
    }
}

pub(super) fn build_commit_digest(
    c: &CommitOutput,
    changes: &[crate::omnigraph::client::EntityChange],
) -> CommitDigest {
    let mut added_count = 0usize;
    let mut updated_count = 0usize;
    let mut deleted_count = 0usize;
    let mut node_changes = 0usize;
    let mut edge_changes = 0usize;
    let mut atom_delta = 0isize;
    let mut atom_titles = Vec::new();
    let mut session_label: Option<String> = None;
    let mut snapshot_summary: Option<String> = None;

    for ch in changes {
        match ch.op {
            ChangeOp::Insert => added_count += 1,
            ChangeOp::Update => updated_count += 1,
            ChangeOp::Delete => deleted_count += 1,
        }
        match ch.kind {
            EntityKind::Node => node_changes += 1,
            EntityKind::Edge => edge_changes += 1,
        }
        let props = ch
            .after
            .as_ref()
            .or(ch.before.as_ref())
            .map(|img| &img.properties);
        if ch.kind == EntityKind::Node && ch.entity_type.name == "ContextAtom" {
            match ch.op {
                ChangeOp::Insert => atom_delta += 1,
                ChangeOp::Delete => atom_delta -= 1,
                ChangeOp::Update => {}
            }
            if let Some(title) = props
                .and_then(|p| p["title"].as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                atom_titles.push(title.to_string());
            }
        } else if ch.kind == EntityKind::Node
            && ch.entity_type.name == "Session"
            && session_label.is_none()
        {
            session_label = props
                .and_then(|p| p["title"].as_str().or_else(|| p["branch"].as_str()))
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| Some(ch.id.clone()));
        } else if ch.kind == EntityKind::Node
            && ch.entity_type.name == "TurnSnapshot"
            && snapshot_summary.is_none()
        {
            snapshot_summary = props
                .and_then(|p| p["summary"].as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
        }
    }

    let summary = if c.merged_parent_commit_id.is_some() {
        let branch = c.graph_branch.as_deref().unwrap_or("main");
        if changes.is_empty() {
            format!("Merged into {branch}")
        } else {
            format!("Merged into {branch} (+{node_changes} nodes · +{edge_changes} edges)")
        }
    } else if let Some(first_title) = atom_titles.first() {
        let clipped: String = if first_title.chars().count() > 56 {
            let prefix: String = first_title.chars().take(55).collect();
            format!("{prefix}…")
        } else {
            first_title.clone()
        };
        let extra_atoms = atom_titles.len().saturating_sub(1);
        match (extra_atoms, edge_changes) {
            (0, 0) => format!("Published: {clipped}"),
            (0, e) => format!(
                "Published: {clipped} (+{e} edge{})",
                if e == 1 { "" } else { "s" }
            ),
            (a, 0) => format!("Published: {clipped} (+{a} more)"),
            (a, e) => format!("Published: {clipped} (+{a} atoms, +{e} edges)"),
        }
    } else if let Some(label) = session_label {
        let clipped: String = if label.chars().count() > 52 {
            let prefix: String = label.chars().take(51).collect();
            format!("{prefix}…")
        } else {
            label
        };
        if edge_changes > 0 {
            format!("Session: {clipped} (+{edge_changes} edges)")
        } else {
            format!("Session: {clipped}")
        }
    } else if snapshot_summary.is_some() {
        if edge_changes > 0 {
            format!("Turn snapshot (+{edge_changes} edges)")
        } else {
            "Turn snapshot".to_string()
        }
    } else if changes.is_empty() {
        "Empty commit".to_string()
    } else {
        format!("+{node_changes} nodes · +{edge_changes} edges")
    };

    CommitDigest {
        added_count,
        updated_count,
        deleted_count,
        atom_delta,
        summary,
    }
}

pub(super) fn parse_opt_usize(v: &Value, label: &str) -> Result<Option<usize>> {
    match v {
        Value::Null => Ok(None),
        Value::Number(n) => n
            .as_u64()
            .and_then(|val| usize::try_from(val).ok())
            .map(Some)
            .ok_or_else(|| HarnessError::invalid(format!("Invalid `{label}` parameter."))),
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                trimmed
                    .parse::<usize>()
                    .map(Some)
                    .map_err(|_| HarnessError::invalid(format!("Invalid `{label}` parameter.")))
            }
        }
        _ => Err(HarnessError::invalid(format!(
            "Invalid `{label}` parameter."
        ))),
    }
}

fn parse_graph_cursor(cursor: Option<&str>) -> Result<Option<usize>> {
    let Some(raw) = clean_opt(cursor) else {
        return Ok(None);
    };
    let numeric = raw.strip_prefix("offset:").unwrap_or(raw).trim();
    numeric
        .parse::<usize>()
        .map(Some)
        .map_err(|_| HarnessError::invalid("Invalid `cursor` parameter."))
}

async fn run_recent_atoms_query(
    client: &OmnigraphClient,
    project_id: &str,
    branch: Option<String>,
    snapshot: Option<String>,
) -> Result<crate::omnigraph::client::QueryResponse> {
    let params = json!({ "project_id": project_id });
    match client
        .query(&QueryRequest {
            query: schema::QUERY_RECENT_ATOMS.into(),
            name: Some("recent_atoms".into()),
            params: Some(params.clone()),
            branch: branch.clone(),
            snapshot: snapshot.clone(),
            settings: None,
        })
        .await
    {
        Ok(resp) => Ok(resp),
        Err(err) if matches!(err.status, 401 | 403) => client
            .stored_query(
                "recent_atoms",
                &StoredQueryRequest {
                    params: Some(params),
                    branch,
                    snapshot,
                    expect_mutation: Some(false),
                },
            )
            .await
            .map_err(Into::into),
        Err(err) => Err(err.into()),
    }
}

async fn fetch_commits_with_digests(
    client: &OmnigraphClient,
    cache: &Arc<StdMutex<BTreeMap<String, CommitDigest>>>,
    active_branch: &str,
) -> (
    crate::omnigraph::client::CommitListResponse,
    Vec<CommitDigest>,
) {
    let commits_resp = client
        .list_commits(&CommitListQuery {
            branch: Some(active_branch.to_string()),
            limit: Some(50),
            page_token: None,
        })
        .await
        .unwrap_or_default();
    let digests = summarize_commits_concurrent(client, cache, &commits_resp.commits).await;
    (commits_resp, digests)
}

async fn summarize_commits_concurrent(
    client: &OmnigraphClient,
    cache: &Arc<StdMutex<BTreeMap<String, CommitDigest>>>,
    commits: &[CommitOutput],
) -> Vec<CommitDigest> {
    let mut digests: Vec<Option<CommitDigest>> = vec![None; commits.len()];
    let mut to_fetch = Vec::new();
    for (idx, c) in commits.iter().enumerate() {
        if idx >= COMMIT_DIGEST_PREFETCH_COUNT {
            digests[idx] = Some(fallback_commit_digest(c));
            continue;
        }
        if let Ok(guard) = cache.lock()
            && let Some(hit) = guard.get(&c.graph_commit_id)
        {
            digests[idx] = Some(hit.clone());
            continue;
        }
        to_fetch.push((idx, c.clone()));
    }

    if !to_fetch.is_empty() {
        let sem = Arc::new(tokio::sync::Semaphore::new(COMMIT_DIGEST_CONCURRENCY));
        let mut set = tokio::task::JoinSet::new();
        for (idx, c) in to_fetch {
            let client = client.clone();
            let cache = Arc::clone(cache);
            let permit_sem = Arc::clone(&sem);
            set.spawn(async move {
                let _permit = permit_sem.acquire_owned().await.ok();
                let digest = summarize_commit_cached(&client, &cache, &c).await;
                (idx, digest)
            });
        }
        while let Some(res) = set.join_next().await {
            if let Ok((idx, digest)) = res {
                digests[idx] = Some(digest);
            }
        }
    }

    commits
        .iter()
        .zip(digests)
        .map(|(c, d)| d.unwrap_or_else(|| fallback_commit_digest(c)))
        .collect()
}

fn build_branches_json(branch_list: &crate::omnigraph::client::BranchListResponse) -> Vec<Value> {
    let main_head = branch_list
        .details
        .as_ref()
        .and_then(|details| details.iter().find(|d| d.name == "main"))
        .map(|d| d.head.graph_commit_id.clone());

    if let Some(details) = &branch_list.details {
        details
            .iter()
            .map(|d| {
                let unmerged = d.name != "main"
                    && main_head
                        .as_deref()
                        .is_some_and(|mh| mh != d.head.graph_commit_id);
                json!({
                    "name": d.name,
                    "head_commit": d.head.graph_commit_id,
                    "head_commit_id": d.head.graph_commit_id,
                    "from": d.from,
                    "from_commit_id": d.from_commit_id,
                    "created_at": d.created_at,
                    "updated_at": d.updated_at,
                    "is_main": d.name == "main",
                    "unmerged": unmerged,
                })
            })
            .collect()
    } else {
        branch_list
            .branches
            .iter()
            .map(|name| {
                json!({
                    "name": name,
                    "is_main": name == "main",
                    "unmerged": name != "main",
                })
            })
            .collect()
    }
}

fn commit_to_graph_json(c: &CommitOutput, active_branch: &str, digest: &CommitDigest) -> Value {
    json!({
        "graph_commit_id": c.graph_commit_id,
        "commit_id": c.graph_commit_id,
        "branch": c.graph_branch.as_deref().unwrap_or(active_branch),
        "graph_branch": c.graph_branch,
        "parent_commit_id": c.parent_commit_id,
        "merged_parent_commit_id": c.merged_parent_commit_id,
        "actor_id": c.actor_id,
        "authored_at": c.created_at,
        "created_at": format_omnigraph_ts(c.created_at)
            .unwrap_or_else(|| c.created_at.to_string()),
        "published": c.published,
        "added_count": digest.added_count,
        "updated_count": digest.updated_count,
        "deleted_count": digest.deleted_count,
        "atom_delta": digest.atom_delta,
        "summary": digest.summary,
    })
}

async fn expand_capped_graph_atoms(
    client: &OmnigraphClient,
    project_id: &str,
    active_branch: &str,
    snapshot_commit: Option<&str>,
    initial_rows: Vec<Value>,
) -> (Vec<Value>, bool) {
    if snapshot_commit.is_some() {
        return (initial_rows, true);
    }
    let Ok(ndjson) = client
        .export_branch(Some(active_branch), &["ContextAtom"], None)
        .await
    else {
        return (initial_rows, true);
    };
    let Ok(parsed) = parse_ndjson_records(&ndjson) else {
        return (initial_rows, true);
    };
    let mut by_id: BTreeMap<String, Value> = BTreeMap::new();
    for row in initial_rows {
        if let Some(aid) = str_field(&row, "atom_id").or_else(|| str_field(&row, "id")) {
            by_id.insert(aid, row);
        }
    }
    for (aid, row) in parsed.atoms {
        let proj_ok =
            project_id.is_empty() || str_field(&row, "project_id").is_none_or(|p| p == project_id);
        if proj_ok {
            by_id.entry(aid).or_insert(row);
        }
    }
    let mut combined: Vec<Value> = by_id.into_values().collect();
    sort_atoms_recent_order(&mut combined);
    (combined, false)
}

fn sort_atoms_recent_order(atoms: &mut [Value]) {
    atoms.sort_by(|a, b| {
        let pa = a["pin_priority"].as_i64().unwrap_or(0);
        let pb = b["pin_priority"].as_i64().unwrap_or(0);
        pb.cmp(&pa)
            .then_with(|| {
                atom_timestamp_sort_key(&b["updated_at"])
                    .cmp(&atom_timestamp_sort_key(&a["updated_at"]))
            })
            .then_with(|| {
                let ia = str_field(a, "atom_id")
                    .or_else(|| str_field(a, "id"))
                    .unwrap_or_default();
                let ib = str_field(b, "atom_id")
                    .or_else(|| str_field(b, "id"))
                    .unwrap_or_default();
                ia.cmp(&ib)
            })
    });
}

fn atom_timestamp_sort_key(v: &Value) -> (i64, String) {
    if let Some(n) = v.as_i64() {
        let ms = if n >= 1_000_000_000_000_000 {
            n / 1_000
        } else if n >= 1_000_000_000_000 {
            n
        } else {
            n.saturating_mul(1_000)
        };
        return (ms, String::new());
    }
    let Some(s) = v.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
        return (0, String::new());
    };
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return (dt.timestamp_millis(), s.to_string());
    }
    if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
        return (ndt.and_utc().timestamp_millis(), s.to_string());
    }
    (0, s.to_string())
}

fn symbol_owner_and_label(fqn: &str, file_path: &str, name: &str) -> (Option<String>, String) {
    let scoped = if !file_path.is_empty() {
        fqn.strip_prefix(&format!("{file_path}::"))
    } else {
        None
    };
    if let Some(rest) = scoped {
        let owner = (!name.is_empty())
            .then(|| rest.strip_suffix(&format!("::{name}")))
            .flatten()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        return (owner, rest.to_string());
    }
    let label = if !name.is_empty() {
        name.to_string()
    } else {
        fqn.to_string()
    };
    (None, label)
}

fn symbol_matches_query(symbol: &Value, q_lower: &str) -> bool {
    for key in ["fqn", "name", "label", "owner", "file_path", "kind"] {
        if symbol[key]
            .as_str()
            .is_some_and(|s| s.to_ascii_lowercase().contains(q_lower))
        {
            return true;
        }
    }
    false
}

fn group_edge_targets_by_source(
    edges: &[Value],
    kept_sources: Option<&BTreeSet<String>>,
) -> BTreeMap<String, Vec<String>> {
    let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for edge in edges {
        if let (Some(src), Some(dst)) = (str_field(edge, "source_id"), str_field(edge, "target_id"))
            && kept_sources.is_none_or(|kept| kept.contains(&src))
        {
            let entry = grouped.entry(src).or_default();
            if !entry.contains(&dst) {
                entry.push(dst);
            }
        }
    }
    grouped
}

fn load_session_omnigraph_metadata_batch(
    store: &SessionStore,
    session_ids: &BTreeSet<String>,
) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    if session_ids.is_empty() {
        return out;
    }
    let Ok(db) = store.connection() else {
        return out;
    };
    let ids: Vec<&str> = session_ids.iter().map(String::as_str).collect();
    for chunk in ids.chunks(200) {
        let placeholders = std::iter::repeat_n("?", chunk.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT id, omnigraph FROM sessions WHERE id IN ({placeholders}) AND omnigraph IS NOT NULL"
        );
        let Ok(mut stmt) = db.prepare(&sql) else {
            continue;
        };
        let Ok(rows) = stmt.query_map(rusqlite::params_from_iter(chunk.iter().copied()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }) else {
            continue;
        };
        for row in rows.flatten() {
            if let Ok(parsed) = crate::strict_json::parse(row.1.as_bytes(), 16 * 1024 * 1024) {
                out.insert(row.0, parsed);
            }
        }
    }
    out
}

fn build_symbol_graph_entries(
    project_id: &str,
    symbol_rows: &[Value],
    symbol_edge_rows: &[Value],
    kept_atom_ids: &BTreeSet<String>,
    q_filter: Option<&str>,
    include_unlinked_symbols: bool,
    allow_symbol_q_match: bool,
) -> (Vec<Value>, Vec<Value>) {
    let mut symbol_map: BTreeMap<String, Value> = BTreeMap::new();
    let upsert_symbol_entry = |map: &mut BTreeMap<String, Value>,
                               fqn: &str,
                               file_path: String,
                               kind: String,
                               name: String,
                               proj: String| {
        let (owner, label) = symbol_owner_and_label(fqn, &file_path, &name);
        map.entry(fqn.to_string())
            .and_modify(|existing| {
                if str_field(existing, "file_path").is_none() && !file_path.is_empty() {
                    existing["file_path"] = json!(file_path);
                    existing["path"] = json!(file_path);
                }
                if str_field(existing, "name").is_none() && !name.is_empty() {
                    existing["name"] = json!(name);
                }
                if str_field(existing, "kind").is_none_or(|k| k == "symbol") && !kind.is_empty() {
                    existing["kind"] = json!(kind);
                }
            })
            .or_insert_with(|| {
                json!({
                    "id": fqn,
                    "fqn": fqn,
                    "project_id": proj,
                    "file_path": file_path,
                    "path": file_path,
                    "kind": if kind.is_empty() { "symbol".to_string() } else { kind },
                    "name": name,
                    "owner": owner,
                    "label": label,
                })
            });
    };

    for row in symbol_rows {
        let Some(fqn) = str_field(row, "fqn").or_else(|| str_field(row, "id")) else {
            continue;
        };
        let file_path = str_field(row, "file_path")
            .or_else(|| str_field(row, "path"))
            .unwrap_or_default();
        let kind = str_field(row, "kind").unwrap_or_else(|| "symbol".to_string());
        let name = str_field(row, "name").unwrap_or_default();
        let proj = str_field(row, "project_id").unwrap_or_else(|| project_id.to_string());
        upsert_symbol_entry(&mut symbol_map, &fqn, file_path, kind, name, proj);
    }

    let mut symbol_edges = Vec::new();
    let mut seen_edges: BTreeSet<(String, String)> = BTreeSet::new();
    let mut mentions_by_symbol: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e in symbol_edge_rows {
        let (Some(src), Some(fqn)) = (str_field(e, "source_id"), str_field(e, "target_id")) else {
            continue;
        };
        let keep_src = kept_atom_ids.contains(&src);
        if !keep_src && !(include_unlinked_symbols || (allow_symbol_q_match && q_filter.is_some()))
        {
            continue;
        }
        let file_path = str_field(e, "file_path")
            .or_else(|| str_field(e, "path"))
            .unwrap_or_default();
        let kind = str_field(e, "kind").unwrap_or_else(|| "symbol".to_string());
        let name = str_field(e, "name").unwrap_or_default();
        upsert_symbol_entry(
            &mut symbol_map,
            &fqn,
            file_path,
            kind,
            name,
            project_id.to_string(),
        );

        if !keep_src || !seen_edges.insert((src.clone(), fqn.clone())) {
            continue;
        }
        mentions_by_symbol
            .entry(fqn.clone())
            .or_default()
            .push(src.clone());

        let sym = symbol_map.get(&fqn);
        let resolved_file = sym
            .and_then(|s| str_field(s, "file_path"))
            .unwrap_or_default();
        let resolved_label = sym
            .and_then(|s| str_field(s, "label"))
            .unwrap_or_else(|| fqn.clone());
        let resolved_kind = sym
            .and_then(|s| str_field(s, "kind"))
            .unwrap_or_else(|| "symbol".to_string());
        let role = str_field(e, "role").unwrap_or_else(|| "named".to_string());
        symbol_edges.push(json!({
            "id": format!("sym:{src}->{fqn}"),
            "kind": "mentions_symbol",
            "type": "MentionsSymbol",
            "relation": "mentions_symbol",
            "source": src,
            "target": fqn,
            "from": src,
            "to": fqn,
            "atom_id": src,
            "fqn": fqn,
            "role": role,
            "label": role,
            "file_path": resolved_file,
            "symbol_name": resolved_label,
            "symbol_kind": resolved_kind,
        }));
    }

    let mut symbols = Vec::new();
    for (fqn, mut sym) in symbol_map {
        let atom_ids = mentions_by_symbol.remove(&fqn).unwrap_or_default();
        let mention_count = atom_ids.len();
        let q_match =
            allow_symbol_q_match && q_filter.is_some_and(|qf| symbol_matches_query(&sym, qf));
        if include_unlinked_symbols || mention_count > 0 || q_match {
            sym["mention_count"] = json!(mention_count);
            sym["atom_ids"] = json!(atom_ids);
            symbols.push(sym);
        }
    }

    (symbols, symbol_edges)
}

fn build_graph_edges(
    pub_edges: &[Value],
    con_edges: &[Value],
    file_edges: &[Value],
    rel_edges: &[Value],
    follow_edges: &[Value],
    kept_atom_ids: &BTreeSet<String>,
    sid_filter: Option<&str>,
) -> Vec<Value> {
    let mut edges = Vec::new();
    for e in pub_edges {
        if let (Some(src), Some(dst)) = (str_field(e, "source_id"), str_field(e, "target_id"))
            && kept_atom_ids.contains(&src)
        {
            edges.push(json!({
                "id": format!("pub:{src}->{dst}"),
                "kind": "published_by",
                "relation": "published_by",
                "source": src,
                "target": dst,
                "from": src,
                "to": dst,
                "label": e["role"].as_str().unwrap_or("published_by"),
                "published_at": e.get("published_at").cloned().unwrap_or(Value::Null),
            }));
        }
    }
    for e in con_edges {
        if let (Some(src), Some(dst)) = (str_field(e, "source_id"), str_field(e, "target_id"))
            && kept_atom_ids.contains(&src)
        {
            edges.push(json!({
                "id": format!("con:{src}->{dst}"),
                "kind": "consumed_by",
                "relation": "consumed_by",
                "source": src,
                "target": dst,
                "from": src,
                "to": dst,
                "label": e["injection_mode"].as_str().unwrap_or("consumed_by"),
                "relevance": e.get("relevance").cloned().unwrap_or(Value::Null),
            }));
        }
    }
    for e in file_edges {
        if let (Some(src), Some(dst)) = (str_field(e, "source_id"), str_field(e, "target_id"))
            && kept_atom_ids.contains(&src)
        {
            edges.push(json!({
                "id": format!("file:{src}->{dst}"),
                "kind": "touches_file",
                "relation": "touches_file",
                "source": src,
                "target": dst,
                "from": src,
                "to": dst,
                "label": e["access"].as_str().unwrap_or("touches_file"),
            }));
        }
    }
    for e in rel_edges {
        if let (Some(src), Some(dst)) = (str_field(e, "source_id"), str_field(e, "target_id"))
            && kept_atom_ids.contains(&src)
            && kept_atom_ids.contains(&dst)
        {
            let rel = e["relation"].as_str().unwrap_or("relates_to");
            edges.push(json!({
                "id": format!("rel:{src}:{rel}:{dst}"),
                "kind": "relates_to",
                "relation": rel,
                "source": src,
                "target": dst,
                "from": src,
                "to": dst,
                "label": rel,
                "note": e.get("note").cloned().unwrap_or(Value::Null),
                "confidence": e.get("confidence").cloned().unwrap_or(Value::Null),
            }));
        }
    }
    for e in follow_edges {
        if let (Some(src), Some(dst)) = (str_field(e, "source_id"), str_field(e, "target_id"))
            && sid_filter.is_none_or(|sid| src == sid || dst == sid)
        {
            // Edges written before the relation was normalized carry a
            // shared space's name.
            let rel = e["relation"]
                .as_str()
                .filter(|rel| matches!(*rel, "broadcast" | "shared_branch" | "linked"))
                .unwrap_or("shared_branch");
            edges.push(json!({
                "id": format!("follow:{src}->{dst}"),
                "kind": "session_follows",
                "relation": rel,
                "source": src,
                "target": dst,
                "from": src,
                "to": dst,
                "label": rel,
                "created_at": e.get("created_at").cloned().unwrap_or(Value::Null),
            }));
        }
    }
    edges
}

pub(super) async fn summarize_commit_cached(
    client: &OmnigraphClient,
    cache: &StdMutex<BTreeMap<String, CommitDigest>>,
    c: &CommitOutput,
) -> CommitDigest {
    if let Ok(guard) = cache.lock()
        && let Some(hit) = guard.get(&c.graph_commit_id)
    {
        return hit.clone();
    }
    let digest = match client
        .commit_changes(&c.graph_commit_id, Some(200), None)
        .await
    {
        Ok(detail) => build_commit_digest(c, &detail.changes),
        Err(err) if err.status == 409 => CommitDigest {
            added_count: 0,
            updated_count: 0,
            deleted_count: 0,
            atom_delta: 0,
            summary: "Initial root commit".to_string(),
        },
        Err(_) => return fallback_commit_digest(c),
    };
    if let Ok(mut guard) = cache.lock() {
        if guard.len() >= 512 {
            guard.clear();
        }
        guard.insert(c.graph_commit_id.clone(), digest.clone());
    }
    digest
}

#[cfg(test)]
mod overlay_cache_tests {
    use super::*;

    #[test]
    fn collect_code_graph_overlay_reuses_epoch_cached_snapshot_and_invalidates_on_write_epoch() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(
            root.join(".harness-architecture.json"),
            r#"{"version":1,"layers":[{"id":"api","name":"API","paths":["src/api.ts"],"forbids":["db"]},{"id":"db","name":"DB","paths":["src/db.ts"]}]}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("src/db.ts"),
            "export function queryDb(): number { return 1; }\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/api.ts"),
            "import { queryDb } from './db';\nexport function handle(): number { return queryDb(); }\n",
        )
        .unwrap();

        let ir = crate::graph_ir::GraphIrStore::open(&state, "omni-overlay-hit").unwrap();
        ir.sync_workspace(&root).unwrap();

        let kept = BTreeSet::from(["src/api.ts".to_owned()]);
        let (first, hit1) = collect_code_graph_overlay_with_hit(
            &state,
            "omni-overlay-hit",
            Some(&root),
            &kept,
            None,
        );
        assert!(!hit1, "cold overlay build must miss cache");
        assert!(!first.ir_edges.is_empty());
        assert!(!first.architecture_violations.is_empty());

        let (second, hit2) = collect_code_graph_overlay_with_hit(
            &state,
            "omni-overlay-hit",
            Some(&root),
            &kept,
            Some("api"),
        );
        assert!(
            hit2,
            "second call at same write_epoch must hit cached overlay snapshot"
        );
        assert!(!second.ir_edges.is_empty());
        assert!(!second.architecture_violations.is_empty());

        // Bumping IR write_epoch invalidates snapshot cleanly.
        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(
            root.join("src/db.ts"),
            "export function queryDb(): number { return 2; }\nexport function extra(): number { return 3; }\n",
        )
        .unwrap();
        ir.update_paths_with_delta(&root, &["src/db.ts".to_owned()])
            .unwrap();
        let (_, hit3) = collect_code_graph_overlay_with_hit(
            &state,
            "omni-overlay-hit",
            Some(&root),
            &kept,
            None,
        );
        assert!(
            !hit3,
            "IR write_epoch bump must invalidate overlay snapshot"
        );
    }
}
