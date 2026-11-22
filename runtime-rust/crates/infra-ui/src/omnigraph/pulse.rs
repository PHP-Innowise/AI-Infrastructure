use super::*;

pub(super) fn reset_pulse_cursor() {
    m::set("omnigraph-pulse-cursor", json!({ "value": "" }));
}

pub(super) fn switch_branch_state(branch: &str, clear_snapshot: bool) -> bool {
    let target = branch.trim();
    if target.is_empty() || target == "auto" {
        return false;
    }
    let changed = m::text(&ui_state(), "branch") != target;
    set_ui_state(|s| {
        s["branch"] = json!(target);
        if clear_snapshot {
            s["snapshot"] = V::Null;
            s["selected_commit_id"] = V::Null;
        }
    });
    if changed {
        reset_pulse_cursor();
    }
    changed
}

pub fn focus_branch(branch: &str) {
    let _ = switch_branch_state(branch, true);
}

pub async fn open_knowledge_atom(project_id: &str, target_atom_id: &str) {
    if !project_id.is_empty() && dom::value("omnigraph-project") != project_id {
        crate::session::sync_project(project_id);
    }
    let _ = switch_branch_state(CANONICAL_KNOWLEDGE_BRANCH, true);
    set_ui_state(|s| {
        s["kind"] = json!("");
        s["query"] = json!("");
        if target_atom_id.is_empty() {
            s["selected_id"] = V::Null;
        } else {
            s["selected_id"] = json!(target_atom_id);
        }
        s["stage_tab"] = json!("constellation");
        s["drawer_tab"] = json!("inspector");
    });
    dom::set_value("omnigraph-search", "");
    m::set("omnigraph-search-results", V::Null);
    dom::hidden("omnigraph-search-clear", true);
    crate::session::open_view("omnigraph", true).await;

    if target_atom_id.is_empty() {
        return;
    }
    let graph = m::data("omnigraph-graph");
    let raw_id = target_atom_id
        .strip_prefix("brain-")
        .or_else(|| target_atom_id.strip_prefix("mem-"))
        .unwrap_or(target_atom_id);
    let matched_id = m::array(&graph["atoms"])
        .into_iter()
        .find(|a| {
            atom_id(a) == target_atom_id
                || a["brain_record_id"].as_str() == Some(raw_id)
                || a["memory_id"].as_str() == Some(raw_id)
        })
        .map(|a| atom_id(&a));
    if let Some(found_id) = matched_id {
        if found_id != target_atom_id {
            set_ui_state(|s| s["selected_id"] = json!(found_id));
            render_constellation();
            render_inspector();
        }
        fly_to_node(&found_id);
    } else {
        let has_canonical = branch_names_from_graph(&graph)
            .iter()
            .any(|b| b == CANONICAL_KNOWLEDGE_BRANCH);
        if !has_canonical {
            let _ = switch_branch_state("main", true);
            sync_controls_from_state();
            refresh_graph().await;
        }
        dom::set_value("omnigraph-search", raw_id);
        run_search(raw_id.to_owned()).await;
        let hits = m::array(&m::data("omnigraph-search-results")["hits"]);
        if let Some(first_id) = hits
            .first()
            .map(|h| atom_id(&h["atom"]))
            .filter(|s| !s.is_empty())
        {
            set_ui_state(|s| s["selected_id"] = json!(first_id));
            render_constellation();
            render_inspector();
            fly_to_node(&first_id);
        }
    }
}

pub(super) fn service_match_tokens(service: &str, sys_catalog: &V) -> Vec<String> {
    let mut tokens = Vec::new();
    let trimmed = service.trim();
    if trimmed.is_empty() {
        return tokens;
    }
    tokens.push(trimmed.to_ascii_lowercase());
    for svc in m::array(&sys_catalog["services"]) {
        let sid = m::text(&svc, "id");
        let name = m::text(&svc, "name");
        if sid.eq_ignore_ascii_case(trimmed)
            || (!name.is_empty() && name.eq_ignore_ascii_case(trimmed))
        {
            if !name.is_empty() {
                tokens.push(name.to_ascii_lowercase());
            }
            for root_key in ["root", "path", "folder"] {
                let root = m::text(&svc, root_key)
                    .trim_matches('/')
                    .to_ascii_lowercase();
                if !root.is_empty() && !tokens.contains(&root) {
                    tokens.push(root);
                }
            }
        }
    }
    tokens
}

pub fn local_service_match_ids(graph: &V, service: &str, sys_catalog: &V) -> Vec<String> {
    let tokens = service_match_tokens(service, sys_catalog);
    if tokens.is_empty() {
        return Vec::new();
    }
    let matches_any = |hay: &str| {
        let low = hay.to_ascii_lowercase();
        tokens.iter().any(|t| low.contains(t))
    };
    let mut ids = Vec::new();
    for a in m::array(&graph["atoms"]) {
        let aid = atom_id(&a);
        if aid.is_empty() {
            continue;
        }
        let hit = matches_any(a["title"].as_str().unwrap_or(""))
            || matches_any(a["summary"].as_str().unwrap_or(""))
            || m::array(&a["tags"])
                .iter()
                .any(|t| t.as_str().is_some_and(matches_any))
            || m::array(&a["files"])
                .iter()
                .any(|f| f.as_str().is_some_and(matches_any));
        if hit && !ids.contains(&aid) {
            ids.push(aid);
        }
    }
    for f in m::array(&graph["files"]) {
        let fid = file_node_id(&f);
        let path = f["path"].as_str().unwrap_or(&fid);
        if !fid.is_empty() && matches_any(path) && !ids.contains(&fid) {
            ids.push(fid);
        }
    }
    for sym in m::array(&graph["symbols"]) {
        let sid = symbol_node_id(&sym);
        let path = symbol_file_path(&sym);
        let fqn = sym["fqn"].as_str().unwrap_or(&sid);
        if !sid.is_empty() && (matches_any(&path) || matches_any(fqn)) && !ids.contains(&sid) {
            ids.push(sid);
        }
    }
    ids
}

fn non_empty_trimmed(val: Option<&str>) -> Option<&str> {
    val.map(str::trim).filter(|s| !s.is_empty())
}

pub fn apply_system_jump_context_to_state(
    s: &mut V,
    service: &str,
    contract: Option<&str>,
    file: Option<&str>,
    run_id: Option<&str>,
) -> String {
    let svc = service.trim();
    let contract_opt = non_empty_trimmed(contract);
    let file_opt = non_empty_trimmed(file);
    let run_opt = non_empty_trimmed(run_id);
    // Focus/highlight matching nodes in the constellation instead of filtering
    // `#omnigraph-search` (which hides unrelated context nodes).
    s["query"] = json!("");
    s["stage_tab"] = json!("constellation");
    s["drawer_tab"] = json!("inspector");
    if let Some(run) = run_opt {
        s["run_id"] = json!(run);
        s["filter_run_id"] = json!(run);
    } else {
        s["run_id"] = V::Null;
        s["filter_run_id"] = V::Null;
    }
    if svc.is_empty() && contract_opt.is_none() && file_opt.is_none() && run_opt.is_none() {
        s["system_context"] = V::Null;
    } else {
        s["system_context"] = json!({
            "service": (!svc.is_empty()).then_some(svc),
            "contract": contract_opt,
            "file": file_opt,
            "run_id": run_opt,
        });
    }
    file_opt
        .or(contract_opt)
        .or((!svc.is_empty()).then_some(svc))
        .unwrap_or("")
        .to_owned()
}

pub(crate) async fn open_from_system_context(
    service: String,
    contract: Option<String>,
    file: Option<String>,
    run_id: Option<String>,
) {
    let svc = service.trim().to_owned();
    let contract_norm = non_empty_trimmed(contract.as_deref()).map(str::to_owned);
    let file_norm = non_empty_trimmed(file.as_deref()).map(str::to_owned);
    let mut query = String::new();
    set_ui_state(|s| {
        query = apply_system_jump_context_to_state(
            s,
            &svc,
            contract_norm.as_deref(),
            file_norm.as_deref(),
            run_id.as_deref(),
        );
    });
    dom::set_value("omnigraph-search", "");
    m::set("omnigraph-search-results", V::Null);
    dom::hidden("omnigraph-search-clear", true);
    crate::session::open_view("omnigraph", true).await;
    if query.is_empty() {
        return;
    }

    let project = current_project_id();
    let st = ui_state();
    let local_graph = m::data("omnigraph-graph");
    let sys_catalog = m::data("system-map-catalog-cache");
    let mut matched_ids = Vec::new();
    let push_unique = |out: &mut Vec<String>, id: String| {
        if !id.is_empty() && !out.contains(&id) {
            out.push(id);
        }
    };
    if let Some(ref fpath) = file_norm {
        if let Some(fid) = find_context_node_for_file(&local_graph, fpath) {
            push_unique(&mut matched_ids, fid);
        }
        for id in local_service_match_ids(&local_graph, fpath, &sys_catalog) {
            push_unique(&mut matched_ids, id);
        }
    }
    if let Some(ref c) = contract_norm {
        for id in local_service_match_ids(&local_graph, c, &sys_catalog) {
            push_unique(&mut matched_ids, id);
        }
    }
    if !svc.is_empty() {
        for id in local_service_match_ids(&local_graph, &svc, &sys_catalog) {
            push_unique(&mut matched_ids, id);
        }
    }

    let mut search_queries = vec![query.clone()];
    if !svc.is_empty() && svc != query {
        search_queries.push(svc.clone());
    }
    for sq in &search_queries {
        let search_path = format!(
            "/api/omnigraph/search{}",
            dom::query(&[
                ("project_id", project.clone()),
                ("q", sq.clone()),
                ("branch", m::text(&st, "branch")),
                ("limit", "20".into()),
            ])
        );
        if let Ok((epoch, res)) = api::scoped("omnigraph-service-focus", &search_path).await {
            if m::current("omnigraph-service-focus", epoch) {
                let (merged_graph, _) =
                    merge_search_subgraph_into_graph(m::data("omnigraph-graph"), &res);
                m::set("omnigraph-graph", merged_graph);
                for id in collect_search_hit_ids(&res) {
                    push_unique(&mut matched_ids, id);
                }
            }
        }
        if !matched_ids.is_empty() {
            break;
        }
    }
    let first_id = matched_ids.first().cloned();
    set_ui_state(|s| {
        s["highlighted_ids"] = json!(matched_ids);
        if let Some(ref fid) = first_id {
            s["selected_id"] = json!(fid);
            s["focused_id"] = json!(fid);
        }
    });
    if let Some(ref fid) = first_id {
        render_studio();
        fly_to_node(fid);
    } else {
        render_studio();
        let label = if !svc.is_empty() { &svc } else { &query };
        dom::text(
            "omnigraph-status-message",
            &format!(
                "No Omnigraph atoms matched service '{label}' yet — showing full constellation."
            ),
        );
        dom::attr("omnigraph-status-message", "data-degraded", "false");
        dom::hidden("omnigraph-status-message", false);
    }
}

#[allow(dead_code)]
pub(crate) async fn open_from_system_service(service: String) {
    open_from_system_context(service, None, None, None).await;
}

pub(super) fn schedule_pulse_poll() {
    dom::cancel_timer("omnigraph-pulse");
    if dom::document().hidden()
        || m::read(|s| s.view != "omnigraph")
        || !m::yes(&m::data("omnigraph-status")["connected"])
        || m::yes(&ui_state()["pulse_paused"])
    {
        return;
    }
    dom::later("omnigraph-pulse", PULSE_INTERVAL_MS, || {
        spawn_local(async {
            poll_changes(false).await;
            schedule_pulse_poll();
        });
    });
}

pub(super) const GRAPH_PAGE_LIMIT: usize = 250;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphPaginationSummary {
    pub shown_atoms: usize,
    pub total_atoms: usize,
    pub symbol_count: usize,
    pub has_more: bool,
    pub truncated: bool,
    pub next_cursor: String,
    pub next_offset: usize,
}

pub fn graph_pagination_summary(graph: &V) -> GraphPaginationSummary {
    let shown_atoms = m::array(&graph["atoms"]).len();
    let total_atoms = graph["total_atoms"]
        .as_u64()
        .or_else(|| graph["telemetry"]["total_atoms"].as_u64())
        .map(|n| (n as usize).max(shown_atoms))
        .unwrap_or(shown_atoms);
    let symbol_count = m::array(&graph["symbols"]).len();
    let has_more = graph["has_more"].as_bool().unwrap_or(false)
        || graph["telemetry"]["has_more"].as_bool().unwrap_or(false)
        || shown_atoms < total_atoms;
    let truncated = graph["truncated"].as_bool().unwrap_or(false)
        || graph["telemetry"]["truncated"].as_bool().unwrap_or(false);
    let next_cursor = graph["next_cursor"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_default();
    let offset = graph["offset"].as_u64().unwrap_or(0) as usize;
    let next_offset = offset.saturating_add(shown_atoms).max(shown_atoms);
    GraphPaginationSummary {
        shown_atoms,
        total_atoms,
        symbol_count,
        has_more,
        truncated,
        next_cursor,
        next_offset,
    }
}

pub fn render_pagination_bar_html(graph: &V) -> (bool, String) {
    if !graph.is_object() {
        return (false, String::new());
    }
    let summary = graph_pagination_summary(graph);
    let show = summary.has_more
        || summary.truncated
        || (summary.total_atoms > 0 && summary.shown_atoms < summary.total_atoms);
    if !show {
        return (false, String::new());
    }
    let mut parts = vec![format!(
        "Showing {} of {} atoms",
        format_count(summary.shown_atoms),
        format_count(summary.total_atoms)
    )];
    if summary.symbol_count > 0 {
        parts.push(format!("{} symbols", format_count(summary.symbol_count)));
    }
    if summary.truncated && !summary.has_more {
        parts.push("truncated at backend limit".to_owned());
    }
    let mut html = format!(
        "<span class=\"omni-pagination-meta\">{}</span>",
        esc(&parts.join(" · "))
    );
    if summary.has_more {
        let remaining = summary.total_atoms.saturating_sub(summary.shown_atoms);
        let btn_label = if remaining > 0 {
            format!(
                "Load more (+{} remaining)",
                format_count(remaining.min(GRAPH_PAGE_LIMIT))
            )
        } else {
            "Load more atoms".to_owned()
        };
        html.push_str(&format!(
            "<button type=\"button\" class=\"button compact\" id=\"omnigraph-load-more\" data-action=\"omnigraph-load-more\">{}</button>",
            esc(&btn_label)
        ));
    }
    (true, html)
}

pub(super) fn merge_graph_collections(existing: &mut V, patch: &V) -> bool {
    if !patch.is_object() {
        return false;
    }
    if !existing.is_object() {
        *existing = json!({
            "atoms": [],
            "sessions": [],
            "files": [],
            "symbols": [],
            "edges": [],
            "symbol_edges": []
        });
    }
    let mut changed = false;
    let merge_array =
        |dst_root: &mut V, src_root: &V, key: &str, id_fn: fn(&V) -> String| -> bool {
            let incoming = m::array(&src_root[key]);
            if incoming.is_empty() {
                return false;
            }
            let mut combined = m::array(&dst_root[key]);
            let mut seen: BTreeSet<String> = combined
                .iter()
                .map(id_fn)
                .filter(|id| !id.is_empty())
                .collect();
            let mut added = false;
            for item in incoming {
                let id = id_fn(&item);
                if id.is_empty() || seen.insert(id) {
                    combined.push(item);
                    added = true;
                }
            }
            dst_root[key] = V::Array(combined);
            added
        };
    changed |= merge_array(existing, patch, "atoms", atom_id);
    changed |= merge_array(existing, patch, "sessions", session_node_id);
    changed |= merge_array(existing, patch, "files", file_node_id);
    changed |= merge_array(existing, patch, "symbols", symbol_node_id);

    let edge_key = |e: &V| -> String {
        let eid = e["id"].as_str().unwrap_or("").trim();
        if !eid.is_empty() {
            return eid.to_owned();
        }
        format!("{}|{}|{}", edge_from(e), edge_rel(e), edge_to(e))
    };
    for key in ["edges", "symbol_edges"] {
        let incoming = m::array(&patch[key]);
        if incoming.is_empty() {
            continue;
        }
        let mut combined = m::array(&existing[key]);
        let mut seen: BTreeSet<String> = combined
            .iter()
            .map(edge_key)
            .filter(|k| !k.is_empty())
            .collect();
        for edge in incoming {
            let k = edge_key(&edge);
            if k.is_empty() || seen.insert(k) {
                combined.push(edge);
                changed = true;
            }
        }
        existing[key] = V::Array(combined);
    }
    changed
}

pub fn merge_graph_page(mut existing: V, page: V) -> V {
    if !existing.is_object() {
        return page;
    }
    if !page.is_object() {
        return existing;
    }
    let _ = merge_graph_collections(&mut existing, &page);

    for meta_key in ["total_atoms", "has_more", "truncated", "limit", "telemetry"] {
        if !page[meta_key].is_null() {
            existing[meta_key] = page[meta_key].clone();
        }
    }
    // Explicitly overwrite next_cursor (even when null on the final page) so a
    // stale cursor from an earlier page does not linger on merged state.
    existing["next_cursor"] = page["next_cursor"].clone();
    existing
}

/// Merges `/api/omnigraph/search` hit subgraph & entities into the cached graph
/// without mutating pagination cursors so search hits beyond page 1 (`limit=250`)
/// stay visible on the constellation and selectable cleanly.
pub fn merge_search_subgraph_into_graph(mut existing: V, search_res: &V) -> (V, bool) {
    if !existing.is_object() || !search_res.is_object() {
        return (existing, false);
    }
    let mut changed = false;
    if search_res["subgraph"].is_object() {
        changed |= merge_graph_collections(&mut existing, &search_res["subgraph"]);
    }
    let mut patch_atoms = Vec::new();
    let mut patch_files = Vec::new();
    let mut patch_symbols = Vec::new();
    let mut patch_sessions = Vec::new();
    let mut patch_edges = Vec::new();
    let mut patch_sym_edges = Vec::new();

    for hit in m::array(&search_res["hits"]) {
        if hit["atom"].is_object() {
            patch_atoms.push(hit["atom"].clone());
        }
        for f in m::array(&hit["files"]) {
            if f.is_object() {
                patch_files.push(f);
            } else if let Some(s) = f.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                patch_files.push(json!({ "id": s, "path": s }));
            }
        }
        for sym in m::array(&hit["symbols"]) {
            if sym.is_object() {
                patch_symbols.push(sym);
            } else if let Some(s) = sym.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                patch_symbols.push(json!({ "id": s, "fqn": s }));
            }
        }
        for edge in m::array(&hit["edges"]) {
            if edge["relation"].as_str() == Some("mentions_symbol")
                || edge["kind"].as_str() == Some("mentions_symbol")
            {
                patch_sym_edges.push(edge);
            } else {
                patch_edges.push(edge);
            }
        }
        for neighbor in m::array(&hit["neighbors"]) {
            if let Some(obj) = neighbor.as_object() {
                let kind = obj.get("kind").and_then(V::as_str).unwrap_or("");
                let nid = obj
                    .get("id")
                    .or_else(|| obj.get("node_id"))
                    .and_then(V::as_str)
                    .unwrap_or("")
                    .trim();
                if nid.is_empty() {
                    continue;
                }
                match kind {
                    "file" => patch_files.push(json!({ "id": nid, "path": nid })),
                    "symbol" => patch_symbols.push(json!({ "id": nid, "fqn": nid })),
                    "session" => patch_sessions.push(json!({ "id": nid })),
                    _ => {}
                }
            }
        }
    }

    let hit_patch = json!({
        "atoms": patch_atoms,
        "files": patch_files,
        "symbols": patch_symbols,
        "sessions": patch_sessions,
        "edges": patch_edges,
        "symbol_edges": patch_sym_edges,
    });
    changed |= merge_graph_collections(&mut existing, &hit_patch);
    (existing, changed)
}

pub fn collect_search_hit_ids(res: &V) -> Vec<String> {
    let mut ids = Vec::new();
    let mut push_unique = |id: &str| {
        let t = id.trim();
        if !t.is_empty() && !ids.iter().any(|x| x == t) {
            ids.push(t.to_owned());
        }
    };
    for hit in m::array(&res["hits"]) {
        push_unique(&atom_id(&hit["atom"]));
        for f in m::array(&hit["files"]) {
            if let Some(s) = f.as_str() {
                push_unique(s);
            } else {
                push_unique(&file_node_id(&f));
            }
        }
        for sym in m::array(&hit["symbols"]) {
            if let Some(s) = sym.as_str() {
                push_unique(s);
            } else {
                push_unique(&symbol_node_id(&sym));
            }
        }
        for neighbor in m::array(&hit["neighbors"]) {
            if let Some(nid) = neighbor
                .as_str()
                .or_else(|| neighbor["id"].as_str())
                .or_else(|| neighbor["node_id"].as_str())
            {
                push_unique(nid);
            }
        }
    }
    if let Some(sub) = res["subgraph"].as_object() {
        for a in sub.get("atoms").and_then(V::as_array).into_iter().flatten() {
            push_unique(&atom_id(a));
        }
        for s in sub
            .get("sessions")
            .and_then(V::as_array)
            .into_iter()
            .flatten()
        {
            push_unique(&session_node_id(s));
        }
        for f in sub.get("files").and_then(V::as_array).into_iter().flatten() {
            push_unique(&file_node_id(f));
        }
        for sym in sub
            .get("symbols")
            .and_then(V::as_array)
            .into_iter()
            .flatten()
        {
            push_unique(&symbol_node_id(sym));
        }
    }
    ids
}

pub(super) fn graph_head_commit_id(graph: &V, branch: &str) -> String {
    for k in ["head_commit_id", "head_commit", "snapshot_commit_id"] {
        if let Some(s) = graph[k].as_str().map(str::trim).filter(|s| !s.is_empty()) {
            return s.to_owned();
        }
    }
    let target_branch = branch.trim();
    if !target_branch.is_empty() {
        for b in m::array(&graph["branches"]) {
            let name = b["name"].as_str().unwrap_or("").trim();
            if name == target_branch {
                for ck in ["head_commit_id", "head", "commit_id", "head_commit"] {
                    if let Some(c) = b[ck].as_str().map(str::trim).filter(|s| !s.is_empty()) {
                        return c.to_owned();
                    }
                }
            }
        }
    }
    for c in m::array(&graph["commits"]) {
        if target_branch.is_empty() || c["branch"].as_str().map(str::trim) == Some(target_branch) {
            if let Some(id) = c["id"]
                .as_str()
                .or_else(|| c["commit_id"].as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                return id.to_owned();
            }
        }
    }
    String::new()
}

pub(super) fn graph_request_path(
    project: String,
    branch: String,
    snapshot: String,
    kind: String,
    query: String,
    cursor: String,
    offset: Option<usize>,
) -> String {
    let mut params = vec![
        ("project_id", project),
        ("branch", branch),
        ("snapshot", snapshot),
        ("kind", kind),
        ("q", query),
        ("limit", GRAPH_PAGE_LIMIT.to_string()),
    ];
    if !cursor.is_empty() {
        params.push(("cursor", cursor));
    } else if let Some(off) = offset.filter(|n| *n > 0) {
        params.push(("offset", off.to_string()));
    }
    format!("/api/omnigraph/graph{}", dom::query(&params))
}

pub(super) fn unwrap_scoped_doc(
    raw: &V,
    expected_project: &str,
    require_work_target: bool,
) -> Option<V> {
    if !raw.is_object() {
        return None;
    }
    if raw["data"].is_object() {
        let proj = m::text(raw, "project");
        let scope_key = m::text(raw, "scope_key");
        let target = m::text(raw, "target");
        if require_work_target && !target.is_empty() && target != "work" {
            return None;
        }
        if !expected_project.is_empty() {
            let matches_proj = proj == expected_project
                || (!scope_key.is_empty() && scope_key == format!("project:{expected_project}"));
            if !proj.is_empty() || !scope_key.is_empty() {
                if !matches_proj {
                    return None;
                }
            } else if let Some(key) = raw["key"].as_str() {
                if !key.starts_with(&format!("project:{expected_project}|")) {
                    return None;
                }
            }
        }
        return Some(raw["data"].clone());
    }
    if raw["files"].is_array()
        || raw["items"].is_array()
        || raw["edge_diffs"].is_object()
        || raw["broken"].is_array()
        || raw["risks"].is_array()
        || raw["suspects"].is_array()
    {
        return Some(raw.clone());
    }
    None
}

pub fn resolve_workspace_drift_docs(project_id: &str) -> (Option<V>, Option<V>) {
    let changes_doc = [
        "omnigraph-workspace-changes",
        "graph-map-changes",
        "session-graph-changes",
    ]
    .iter()
    .find_map(|key| unwrap_scoped_doc(&m::data(key), project_id, true));

    let impact_doc = [
        "omnigraph-workspace-impact",
        "graph-map-impact",
        "session-graph-impact",
    ]
    .iter()
    .find_map(|key| unwrap_scoped_doc(&m::data(key), project_id, false));

    (changes_doc, impact_doc)
}

pub(super) async fn fetch_workspace_changes_for_project(project_id: String) {
    if project_id.trim().is_empty() {
        return;
    }
    let path = format!(
        "/api/projects/{}/graph/changes?target=work",
        dom::enc(project_id.trim())
    );
    if let Ok((epoch, doc)) = api::scoped("omnigraph-workspace-changes", &path).await {
        if !m::current("omnigraph-workspace-changes", epoch) || current_project_id() != project_id {
            return;
        }
        m::set(
            "omnigraph-workspace-changes",
            json!({
                "project": project_id,
                "target": "work",
                "data": doc
            }),
        );
        if m::read(|s| s.view == "omnigraph") && m::text(&ui_state(), "snapshot").trim().is_empty()
        {
            render_constellation();
        }
    }
}

pub(super) fn ensure_workspace_changes_loaded(project_id: &str) {
    let pid = project_id.trim().to_owned();
    if pid.is_empty() || resolve_workspace_drift_docs(&pid).0.is_some() {
        return;
    }
    spawn_local(async move {
        fetch_workspace_changes_for_project(pid).await;
    });
}

pub(super) async fn refresh_graph_with_limit(limit: usize, preserve_pages: Option<usize>) {
    let project = current_project_id();
    let st = ui_state();
    let branch = m::text(&st, "branch");
    let params = vec![
        ("project_id", project.clone()),
        ("branch", branch.clone()),
        ("snapshot", m::text(&st, "snapshot")),
        ("kind", m::text(&st, "kind")),
        ("q", String::new()),
        ("limit", limit.max(GRAPH_PAGE_LIMIT).to_string()),
    ];
    let path = format!("/api/omnigraph/graph{}", dom::query(&params));
    match api::scoped("omnigraph-graph", &path).await {
        Ok((epoch, graph)) if m::current("omnigraph-graph", epoch) => {
            m::set("omnigraph-page-count", json!(preserve_pages.unwrap_or(1)));
            if preserve_pages.is_none() {
                m::set("omnigraph-raw-positions", V::Null);
            }
            let head = graph_head_commit_id(&graph, &branch);
            if !head.is_empty() {
                m::set("omnigraph-head-commit", json!(head));
            }
            m::set("omnigraph-structure-digest", V::Null);
            m::set("omnigraph-graph", graph.clone());
            sync_branch_selectors(&graph);
            sync_session_selectors(&graph);
            ensure_workspace_changes_loaded(&project);
            render_studio();
        }
        Err(e) if !e.aborted => dom::error("omnigraph-error", &e.message),
        _ => {}
    }
}

pub(super) async fn refresh_graph() {
    refresh_graph_with_limit(GRAPH_PAGE_LIMIT, None).await;
}

pub(super) async fn refresh_graph_preserving_pages() {
    let pages = m::data("omnigraph-page-count").as_u64().unwrap_or(1).max(1) as usize;
    let target_limit = pages
        .saturating_mul(GRAPH_PAGE_LIMIT)
        .min(GRAPH_PAGE_LIMIT * 8);
    refresh_graph_with_limit(target_limit, Some(pages)).await;
}

pub(super) async fn load_more_graph() {
    let current = m::data("omnigraph-graph");
    if !current.is_object() {
        return;
    }
    let summary = graph_pagination_summary(&current);
    if !summary.has_more {
        return;
    }
    dom::disabled("omnigraph-load-more", true);
    let st = ui_state();
    let path = graph_request_path(
        current_project_id(),
        m::text(&st, "branch"),
        m::text(&st, "snapshot"),
        m::text(&st, "kind"),
        String::new(),
        summary.next_cursor,
        Some(summary.next_offset),
    );
    match api::scoped("omnigraph-graph", &path).await {
        Ok((epoch, page)) if m::current("omnigraph-graph", epoch) => {
            let pages = m::data("omnigraph-page-count").as_u64().unwrap_or(1).max(1) + 1;
            m::set("omnigraph-page-count", json!(pages));
            let merged = merge_graph_page(m::data("omnigraph-graph"), page);
            m::set("omnigraph-graph", merged.clone());
            sync_branch_selectors(&merged);
            sync_session_selectors(&merged);
            render_studio();
        }
        Err(e) if !e.aborted => {
            dom::disabled("omnigraph-load-more", false);
            dom::error("omnigraph-error", &e.message);
        }
        _ => dom::disabled("omnigraph-load-more", false),
    }
}

pub(super) async fn run_search(query: String) {
    let project = current_project_id();
    set_ui_state(|s| {
        s["query"] = json!(query);
    });
    let trimmed = query.trim().to_owned();
    dom::hidden("omnigraph-search-clear", trimmed.is_empty());
    if trimmed.is_empty() {
        m::set("omnigraph-search-results", V::Null);
        set_ui_state(|s| {
            s["highlighted_ids"] = json!([]);
        });
        render_studio();
        return;
    }
    let st = ui_state();
    let path = format!(
        "/api/omnigraph/search{}",
        dom::query(&[
            ("project_id", project),
            ("q", trimmed),
            ("branch", m::text(&st, "branch")),
            ("kind", m::text(&st, "kind")),
            ("limit", "20".into()),
        ])
    );
    match api::scoped("omnigraph-search", &path).await {
        Ok((epoch, res)) if m::current("omnigraph-search", epoch) => {
            m::set("omnigraph-search-results", res.clone());
            let (merged_graph, changed) =
                merge_search_subgraph_into_graph(m::data("omnigraph-graph"), &res);
            if changed {
                m::set("omnigraph-graph", merged_graph);
            }
            let ids = collect_search_hit_ids(&res);
            set_ui_state(|s| {
                s["highlighted_ids"] = json!(ids);
            });
            render_studio();
        }
        Err(e) if !e.aborted => dom::error("omnigraph-error", &e.message),
        _ => {}
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangesPollDecision {
    pub next_cursor: String,
    pub next_head_commit: String,
    pub reset: bool,
    pub head_changed: bool,
    pub should_refresh_graph: bool,
    pub has_baseline: bool,
}

pub fn classify_changes_poll_response(
    prev_head_commit: &str,
    data: &V,
    force: bool,
) -> ChangesPollDecision {
    let next_cursor = m::text(data, "cursor");
    let next_head_commit = m::text(data, "snapshot_commit_id");
    let reset = m::yes(&data["reset"]);
    let head_changed = !prev_head_commit.trim().is_empty()
        && !next_head_commit.is_empty()
        && prev_head_commit.trim() != next_head_commit;
    let incoming_len = m::array(&data["events"]).len();
    let has_baseline = data["baseline"].is_object();
    let should_refresh_graph = reset || head_changed || (!force && incoming_len > 0);
    ChangesPollDecision {
        next_cursor,
        next_head_commit,
        reset,
        head_changed,
        should_refresh_graph,
        has_baseline,
    }
}

pub fn extract_baseline_graph_patch(baseline: &V) -> Option<V> {
    if !baseline.is_object() {
        return None;
    }
    let has_collections = [
        "atoms",
        "sessions",
        "files",
        "symbols",
        "edges",
        "symbol_edges",
    ]
    .iter()
    .any(|k| baseline[k].is_array());
    if has_collections {
        return Some(baseline.clone());
    }
    let records = m::array(&baseline["records"]);
    if records.is_empty() {
        return None;
    }
    let mut atoms = Vec::new();
    let mut files = Vec::new();
    let mut sessions = Vec::new();
    let mut edges = Vec::new();
    for rec in records {
        let props = if rec["after"]["properties"].is_object() {
            &rec["after"]["properties"]
        } else if rec["properties"].is_object() {
            &rec["properties"]
        } else {
            &rec
        };
        let etype = rec["entity_type"]["name"]
            .as_str()
            .or_else(|| rec["entity_type"].as_str())
            .or_else(|| rec["kind"].as_str())
            .unwrap_or("");
        let kind = rec["kind"].as_str().unwrap_or("");
        if kind == "edge" || (!edge_from(&rec).is_empty() && !edge_to(&rec).is_empty()) {
            edges.push(rec);
        } else if etype == "FileRef" || props["path"].is_string() && props["title"].is_null() {
            let mut obj = props.clone();
            if obj["id"].is_null() && rec["id"].is_string() {
                obj["id"] = rec["id"].clone();
            }
            files.push(obj);
        } else if etype == "Session" {
            let mut obj = props.clone();
            if obj["id"].is_null() && rec["id"].is_string() {
                obj["id"] = rec["id"].clone();
            }
            sessions.push(obj);
        } else {
            let mut obj = props.clone();
            if obj["id"].is_null() && rec["id"].is_string() {
                obj["id"] = rec["id"].clone();
            }
            atoms.push(obj);
        }
    }
    Some(json!({
        "atoms": atoms,
        "sessions": sessions,
        "files": files,
        "edges": edges,
    }))
}

pub(super) async fn poll_changes(force: bool) {
    if !force && (m::read(|s| s.view != "omnigraph") || m::yes(&ui_state()["pulse_paused"])) {
        return;
    }
    let project = current_project_id();
    let st = ui_state();
    let cursor = m::text(&m::data("omnigraph-pulse-cursor"), "value");
    let path = format!(
        "/api/omnigraph/changes{}",
        dom::query(&[
            ("project_id", project),
            ("cursor", cursor),
            ("branch", m::text(&st, "branch")),
        ])
    );
    match api::scoped("omnigraph-changes", &path).await {
        Ok((epoch, data)) if m::current("omnigraph-changes", epoch) => {
            if m::yes(&m::data("omnigraph-pulse-error")) {
                m::set("omnigraph-pulse-error", json!(false));
                dom::error("omnigraph-error", "");
            }
            let prev_head = m::data("omnigraph-head-commit")
                .as_str()
                .unwrap_or("")
                .to_owned();
            let decision = classify_changes_poll_response(&prev_head, &data, force);
            if !decision.next_cursor.is_empty() {
                m::set(
                    "omnigraph-pulse-cursor",
                    json!({ "value": decision.next_cursor }),
                );
            }
            if !decision.next_head_commit.is_empty() {
                m::set("omnigraph-head-commit", json!(decision.next_head_commit));
            }
            if decision.has_baseline {
                if let Some(patch) = extract_baseline_graph_patch(&data["baseline"]) {
                    let mut current_graph = m::data("omnigraph-graph");
                    if merge_graph_collections(&mut current_graph, &patch) {
                        m::set("omnigraph-graph", current_graph);
                    }
                }
            }
            let incoming = m::array(&data["events"]);
            if !incoming.is_empty() {
                let mut existing = m::array(&m::data("omnigraph-pulse-events"));
                let mut merged = incoming;
                merged.append(&mut existing);
                merged.truncate(MAX_PULSE_EVENTS);
                m::set("omnigraph-pulse-events", json!(merged));
            }
            render_pulse();
            if decision.reset {
                dom::text(
                    "omnigraph-status-message",
                    "Change feed reset detected — synced baseline context graph.",
                );
                dom::attr("omnigraph-status-message", "data-degraded", "false");
                dom::hidden("omnigraph-status-message", false);
            }
            if decision.should_refresh_graph {
                refresh_graph_preserving_pages().await;
                refresh_pill().await;
            }
        }
        Err(e) if !e.aborted => {
            m::set("omnigraph-pulse-error", json!(true));
            dom::error(
                "omnigraph-error",
                &format!("Live Pulse poll failed: {}", e.message),
            );
        }
        _ => {}
    }
}

pub(super) async fn load_commit_detail(commit_id: &str, branch: &str) {
    let project = current_project_id();
    let _ = switch_branch_state(branch, false);
    set_ui_state(|s| {
        s["snapshot"] = json!(commit_id);
        s["selected_commit_id"] = json!(commit_id);
        s["drawer_tab"] = json!("inspector");
    });
    render_tabs();
    sync_controls_from_state();
    refresh_graph().await;
    let path = format!(
        "/api/omnigraph/commits/{}{}",
        dom::enc(commit_id),
        dom::query(&[("project_id", project)])
    );
    match api::scoped("omnigraph-commit", &path).await {
        Ok((epoch, detail)) if m::current("omnigraph-commit", epoch) => {
            m::set("omnigraph-commit-detail", detail);
            render_inspector();
        }
        Err(e) if !e.aborted => dom::error("omnigraph-error", &e.message),
        _ => {}
    }
}

pub(super) async fn preview_merge(source: &str, target: &str) {
    if source.is_empty() || target.is_empty() {
        dom::error("omnigraph-error", "Select both source and target branches.");
        return;
    }
    dom::error("omnigraph-error", "");
    let project = current_project_id();
    let path = format!(
        "/api/omnigraph/branches/diff{}",
        dom::query(&[
            ("project_id", project),
            ("source", source.into()),
            ("target", target.into()),
        ])
    );
    match api::scoped("omnigraph-diff", &path).await {
        Ok((epoch, diff)) if m::current("omnigraph-diff", epoch) => {
            m::set("omnigraph-diff", diff.clone());
            render_merge();
        }
        Err(e) if !e.aborted => dom::error("omnigraph-error", &e.message),
        _ => {}
    }
}

/// Settle one Context Atom conflict by copying the kept side's atom onto
/// the other branch as it is stored, then preview the merge again.
pub(super) async fn keep_conflict_side(payload: &V) {
    let project = current_project_id();
    dom::error("omnigraph-error", "");
    for button in dom::all("[data-action=\"omnigraph-conflict-keep\"]") {
        let _ = button.set_attribute("disabled", "");
    }
    let outcome = match omnigraph_canvas::conflict_keep_request(&project, payload) {
        Ok(body) => match api::post("/api/omnigraph/branches", body).await {
            Ok(res) if res["ok"].as_bool() != Some(false) => Ok(()),
            Ok(res) => Err(res["error"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("Could not keep the atom.")
                .to_owned()),
            Err(e) => Err(e.message),
        },
        Err(message) => Err(message),
    };
    if let Err(message) = outcome {
        dom::error("omnigraph-error", &message);
        render_merge();
        return;
    }
    dom::text(
        "omnigraph-status-message",
        &format!(
            "Copied the {} atom {} to {}.",
            m::text(payload, "keep"),
            m::text(payload, "atom_id"),
            m::text(payload, "write_branch")
        ),
    );
    dom::hidden("omnigraph-status-message", false);
    preview_merge(
        &m::text(payload, "source_branch"),
        &m::text(payload, "target_branch"),
    )
    .await;
    refresh_graph().await;
}

pub fn branch_merge_outcome(res: &V) -> Result<(), String> {
    let outcome = res["outcome"].as_str().unwrap_or("");
    let ok = res["ok"].as_bool();
    if outcome == "conflict" || ok == Some(false) {
        let conflicts = m::array(&res["conflicts"]);
        let msg = res["error"]
            .as_str()
            .or_else(|| res["message"].as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                if !conflicts.is_empty() {
                    format!(
                        "Merge conflict across {} item(s); review the diff below.",
                        conflicts.len()
                    )
                } else if outcome == "conflict" {
                    "Merge conflict detected; resolve conflicts before merging.".to_owned()
                } else {
                    "Branch operation failed.".to_owned()
                }
            });
        Err(msg)
    } else {
        Ok(())
    }
}

pub(super) async fn execute_branch_action(body: V, status_msg: &str) -> Result<V, api::Error> {
    dom::error("omnigraph-error", "");
    let res = api::post("/api/omnigraph/branches", body).await?;
    if branch_merge_outcome(&res).is_ok() {
        dom::text("omnigraph-status-message", status_msg);
        dom::hidden("omnigraph-status-message", false);
    } else {
        dom::hidden("omnigraph-status-message", true);
    }
    refresh_pill().await;
    refresh_graph().await;
    Ok(res)
}

pub fn resolve_code_map_target_with_symbol_id(
    payload: &V,
) -> Option<(String, Option<String>, Option<String>)> {
    let symbol_id = payload["symbol_id"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let fqn = payload["fqn"]
        .as_str()
        .or_else(|| payload["symbol_fqn"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            payload["node_id"]
                .as_str()
                .map(str::trim)
                .filter(|s| s.contains("::"))
                .map(str::to_owned)
        })
        .or_else(|| symbol_id.clone());
    let resolved_sym_id = symbol_id.or_else(|| fqn.clone());
    let direct = payload
        .as_str()
        .or_else(|| payload["path"].as_str())
        .or_else(|| payload["file_path"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(path) = direct {
        return Some((path.to_owned(), fqn, resolved_sym_id));
    }
    let raw_fqn = fqn.as_deref().or_else(|| {
        payload["fqn"]
            .as_str()
            .or_else(|| payload["symbol_fqn"].as_str())
            .or_else(|| payload["symbol_id"].as_str())
            .or_else(|| payload["node_id"].as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    })?;
    let path = raw_fqn
        .split_once("::")
        .map(|(head, _)| head.trim())
        .filter(|head| !head.is_empty() && (head.contains('/') || head.contains('.')))?
        .to_owned();
    Some((path, Some(raw_fqn.to_owned()), resolved_sym_id))
}

pub fn resolve_code_map_target(payload: &V) -> Option<(String, Option<String>)> {
    resolve_code_map_target_with_symbol_id(payload).map(|(p, fqn, sym_id)| (p, fqn.or(sym_id)))
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn resolve_code_map_target_path(payload: &V) -> Option<String> {
    resolve_code_map_target(payload).map(|(p, _)| p)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodeMapJumpContext {
    pub filter: Option<String>,
    pub target_commit: Option<String>,
    pub semantic_kind: Option<String>,
}

fn non_empty_v_str(val: &V) -> Option<String> {
    val.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

pub fn resolve_code_map_jump_context(payload: &V, ui_st: &V) -> CodeMapJumpContext {
    CodeMapJumpContext {
        filter: non_empty_v_str(&payload["filter"]).or_else(|| non_empty_v_str(&ui_st["query"])),
        target_commit: non_empty_v_str(&payload["target_commit"])
            .or_else(|| non_empty_v_str(&payload["commit"]))
            .or_else(|| non_empty_v_str(&ui_st["selected_commit_id"])),
        semantic_kind: non_empty_v_str(&payload["semantic_kind"])
            .or_else(|| non_empty_v_str(&payload["kind"]))
            .or_else(|| non_empty_v_str(&ui_st["kind"])),
    }
}

pub(crate) async fn open_target_in_code_map_with_context(
    project: Option<&str>,
    path: &str,
    symbol_fqn: Option<&str>,
    ctx: CodeMapJumpContext,
) {
    let resolved_project = project
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(current_project_id);
    if !resolved_project.is_empty() && dom::value("project") != resolved_project {
        crate::session::sync_project(&resolved_project);
    }
    if m::read(|s| s.view != "sessions") {
        crate::session::open_view("sessions", true).await;
    }
    crate::session::set_mobile_pane("graph");
    crate::graph_map::focus_target_in_session_with_context(
        path,
        symbol_fqn,
        ctx.filter.as_deref(),
        ctx.target_commit.as_deref(),
        ctx.semantic_kind.as_deref(),
    );
}

#[allow(dead_code)]
pub(crate) async fn open_target_in_code_map(
    project: Option<&str>,
    path: &str,
    symbol_fqn: Option<&str>,
) {
    open_target_in_code_map_with_context(project, path, symbol_fqn, CodeMapJumpContext::default())
        .await;
}

#[allow(dead_code)]
pub(super) async fn open_file_in_code_map(path: &str) {
    open_target_in_code_map(None, path, None).await;
}

pub fn find_context_node_for_file(graph: &V, path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() || !graph.is_object() {
        return None;
    }
    m::array(&graph["files"])
        .into_iter()
        .find(|f| {
            f["path"].as_str() == Some(trimmed)
                || f["file_path"].as_str() == Some(trimmed)
                || f.as_str() == Some(trimmed)
        })
        .map(|f| file_node_id(&f))
        .filter(|id| !id.is_empty())
        .or_else(|| {
            m::array(&graph["symbols"])
                .into_iter()
                .find(|s| symbol_file_path(s) == trimmed)
                .map(|s| symbol_node_id(&s))
                .filter(|id| !id.is_empty())
        })
        .or_else(|| {
            m::array(&graph["atoms"])
                .into_iter()
                .find(|a| {
                    a["files"]
                        .as_array()
                        .is_some_and(|fs| fs.iter().any(|x| x.as_str() == Some(trimmed)))
                })
                .map(|a| atom_id(&a))
                .filter(|id| !id.is_empty())
        })
}

pub fn resolve_system_service_target(
    graph: &V,
    sys_catalog: &V,
    raw_node_id: &str,
) -> Option<String> {
    let nid = raw_node_id.trim();
    if nid.is_empty() {
        return None;
    }
    let mut candidates = vec![nid.to_owned()];
    for f in m::array(&graph["files"]) {
        if file_node_id(&f) == nid || f["path"].as_str() == Some(nid) {
            if let Some(p) = f["path"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
                candidates.push(p.to_owned());
            }
        }
    }
    for s in m::array(&graph["symbols"]) {
        if symbol_node_id(&s) == nid
            || s["symbol_id"].as_str() == Some(nid)
            || s["fqn"].as_str() == Some(nid)
        {
            let p = symbol_file_path(&s);
            if !p.is_empty() {
                candidates.push(p);
            }
        }
    }
    for a in m::array(&graph["atoms"]) {
        if atom_id(&a) == nid {
            for f in m::array(&a["files"]) {
                if let Some(p) = f.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                    candidates.push(p.to_owned());
                }
            }
            for tag in m::array(&a["tags"]) {
                if let Some(t) = tag.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                    candidates.push(t.to_owned());
                }
            }
        }
    }

    let norm = |s: &str| {
        s.trim()
            .trim_start_matches("./")
            .trim_matches('/')
            .to_ascii_lowercase()
    };
    let services = m::array(&sys_catalog["services"]);
    let mut best_prefix: Option<(usize, String)> = None;
    for cand in &candidates {
        let nc = norm(cand);
        if nc.is_empty() {
            continue;
        }
        for svc in &services {
            let sid = m::text(svc, "id");
            if sid.is_empty() {
                continue;
            }
            let sname = m::text(svc, "name");
            if nc == norm(&sid) || (!sname.is_empty() && nc == norm(&sname)) {
                return Some(sid);
            }
            for root_key in ["root", "path", "folder"] {
                let root = norm(&m::text(svc, root_key));
                if root.is_empty() {
                    continue;
                }
                if nc == root || nc.starts_with(&format!("{root}/")) {
                    let len = root.len();
                    if best_prefix.as_ref().is_none_or(|(b, _)| len > *b) {
                        best_prefix = Some((len, sid.clone()));
                    }
                }
            }
        }
        for sh in m::array(&sys_catalog["shared_sources"]) {
            let sp = sh
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| m::text(&sh, "path"));
            if !sp.is_empty() && norm(&sp) == nc {
                return Some("__system__".to_owned());
            }
        }
    }
    best_prefix.map(|(_, sid)| sid)
}

pub(crate) async fn focus_context_for_file(project: Option<&str>, path: &str) {
    if let Some(p) = project.map(str::trim).filter(|s| !s.is_empty()) {
        dom::set_value("omnigraph-project", p);
    }
    if m::read(|s| s.view != "omnigraph") {
        crate::session::open_view("omnigraph", true).await;
    }
    let trimmed = path.trim();
    if !trimmed.is_empty() {
        dom::set_value("omnigraph-search", trimmed);
        set_ui_state(|s| {
            s["query"] = json!(trimmed);
            s["show_files"] = json!(true);
            s["stage_tab"] = json!("constellation");
        });
    }
    render_tabs();
    refresh_graph().await;
    let graph = m::data("omnigraph-graph");
    let matched_id = find_context_node_for_file(&graph, trimmed);
    if let Some(nid) = matched_id {
        set_ui_state(|s| {
            s["selected_id"] = json!(nid);
            s["drawer_tab"] = json!("inspector");
        });
        render_tabs();
        render_constellation();
        render_inspector();
        fly_to_node(&nid);
    }
}
