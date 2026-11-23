use super::*;

pub(super) fn render_tabs() {
    let st = ui_state();
    let stage = st["stage_tab"].as_str().unwrap_or("constellation");
    for tab in ["constellation", "dag", "merge"] {
        let active = stage == tab;
        dom::attr(
            &format!("omni-tab-{tab}"),
            "aria-selected",
            if active { "true" } else { "false" },
        );
        dom::hidden(&format!("omni-pane-{tab}"), !active);
    }
    dom::hidden("omni-canvas-tools", stage != "constellation");

    let drawer = st["drawer_tab"].as_str().unwrap_or("inspector");
    for tab in ["inspector", "pulse", "publish"] {
        let active = drawer == tab;
        dom::attr(
            &format!("omni-drawer-tab-{tab}"),
            "aria-selected",
            if active { "true" } else { "false" },
        );
        dom::hidden(&format!("omni-drawer-{tab}"), !active);
    }
}

pub(super) fn sync_pulse_toggle() {
    let paused = m::yes(&ui_state()["pulse_paused"]);
    dom::attr(
        "omnigraph-view",
        "data-pulse-paused",
        if paused { "true" } else { "false" },
    );
    dom::text(
        "omni-toggle-pulse",
        if paused { "Play pulse" } else { "Pause pulse" },
    );
}

pub(super) fn sync_controls_from_state() {
    let st = ui_state();
    let kind = m::text(&st, "kind");
    for el in dom::all("#omnigraph-kind-chips .omni-kind-chip") {
        let k = el.get_attribute("data-kind").unwrap_or_default();
        let _ = el.set_attribute("aria-pressed", if k == kind { "true" } else { "false" });
    }
    let query = m::text(&st, "query");
    if dom::value("omnigraph-search") != query {
        dom::set_value("omnigraph-search", &query);
    }
    dom::hidden("omnigraph-search-clear", query.trim().is_empty());
    let branch = m::text(&st, "branch");
    if !branch.is_empty() {
        dom::set_value("omnigraph-branch", &branch);
        dom::hidden("omni-delete-branch", branch == "main");
    }
    dom::set_checked(
        "omnigraph-show-sessions",
        st["show_sessions"].as_bool().unwrap_or(true),
    );
    dom::set_checked(
        "omnigraph-show-files",
        st["show_files"].as_bool().unwrap_or(true),
    );
    let iso_active = m::yes(&st["isolate_active"]);
    dom::attr(
        "omni-isolate-toggle",
        "aria-pressed",
        if iso_active { "true" } else { "false" },
    );
    let iso_depth = st["isolate_depth"].as_u64().unwrap_or(1).clamp(1, 3);
    dom::set_value("omni-isolate-depth", &iso_depth.to_string());
    let path_from = m::text(&st, "path_from");
    let path_to = m::text(&st, "path_to");
    if !path_from.is_empty() || dom::value("omni-path-from") != path_from {
        dom::set_value("omni-path-from", &path_from);
    }
    if !path_to.is_empty() || dom::value("omni-path-to") != path_to {
        dom::set_value("omni-path-to", &path_to);
    }
    dom::hidden("omni-path-clear", m::array(&st["path_nodes"]).is_empty());
    let snapshot = m::text(&st, "snapshot");
    dom::hidden("omnigraph-timetravel-banner", snapshot.is_empty());
    if !snapshot.is_empty() {
        dom::text(
            "omnigraph-timetravel-text",
            &format!("Viewing commit {snapshot} on branch {branch}"),
        );
    }
}

pub(super) fn sync_branch_selectors(graph: &V) {
    let branches = branch_names_from_graph(graph);
    let st = ui_state();
    let active_branch = m::text(&st, "branch");
    let items: Vec<V> = branches
        .iter()
        .map(|name| json!({ "id": name, "name": name }))
        .collect();
    dom::options("omnigraph-branch", &items, m::summary, Some(&active_branch));
    dom::options(
        "omni-new-branch-from",
        &items,
        m::summary,
        Some(&active_branch),
    );
    dom::options("omni-pub-branch", &items, m::summary, Some(&active_branch));
    dom::options("omnigraph-existing-branch", &items, m::summary, None);

    let non_main: Vec<V> = branches
        .iter()
        .filter(|name| name.as_str() != "main")
        .map(|name| json!({ "id": name, "name": name }))
        .collect();
    let merge_src = if non_main.is_empty() {
        &items
    } else {
        &non_main
    };
    let prev_src = dom::value("omni-merge-source");
    dom::options(
        "omni-merge-source",
        merge_src,
        m::summary,
        if prev_src.is_empty() && active_branch != "main" {
            Some(&active_branch)
        } else {
            None
        },
    );
    dom::options("omni-merge-target", &items, m::summary, Some("main"));
    dom::hidden("omni-delete-branch", active_branch == "main");
}

pub(super) fn sync_session_selectors(graph: &V) {
    let mut items = vec![json!({ "id": "", "name": "No specific session" })];
    let mut seen = BTreeSet::new();
    for s in m::read(|s| s.sessions.clone()) {
        let id = m::text(&s, "id");
        if !id.is_empty() && seen.insert(id.clone()) {
            let label = m::summary(&s);
            items.push(json!({ "id": id, "name": label }));
        }
    }
    for s in m::array(&graph["sessions"]) {
        let id = session_node_id(&s);
        if !id.is_empty() && seen.insert(id.clone()) {
            let label = m::summary(&s);
            items.push(json!({ "id": id, "name": label }));
        }
    }
    dom::options("omni-pub-session", &items, m::summary, None);
}

pub(super) fn render_pagination_bar(graph: &V) {
    let (show, html) = render_pagination_bar_html(graph);
    let bar = if let Some(existing) = dom::get("omnigraph-pagination-bar") {
        existing
    } else if show {
        let Some(pane) = dom::get("omni-pane-constellation") else {
            return;
        };
        let el = dom::el("div", "omni-pagination-bar", "");
        let _ = el.set_attribute("id", "omnigraph-pagination-bar");
        let _ = el.set_attribute("role", "status");
        let legend = pane.query_selector(".omni-legend").ok().flatten();
        let _ = pane.insert_before(&el, legend.as_ref().map(|n| n.as_ref()));
        el
    } else {
        return;
    };
    dom::hide(&bar, !show);
    if show {
        bar.set_inner_html(&html);
    } else {
        bar.set_inner_html("");
    }
}

pub(super) fn render_studio() {
    render_tabs();
    sync_controls_from_state();
    render_constellation();
    render_pagination_bar(&m::data("omnigraph-graph"));
    render_dag();
    render_merge();
    render_inspector();
    render_pulse();
}

pub(super) fn omni_pin_key() -> String {
    let st = ui_state();
    graph_viewport::pin_storage_key("omnigraph", &current_project_id(), &m::text(&st, "branch"))
}

pub(super) fn anchor_filter_signature(st: &V) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}",
        current_project_id(),
        m::text(st, "branch"),
        m::text(st, "kind"),
        m::text(st, "query").trim(),
        st["show_files"].as_bool().unwrap_or(true),
        st["show_sessions"].as_bool().unwrap_or(true),
        m::yes(&st["isolate_active"]),
        st["isolate_depth"].as_u64().unwrap_or(1),
    )
}

pub(super) fn current_constellation_filter(
    st: &V,
    cam: &OmnigraphCamera,
    paged_count: usize,
) -> omnigraph_canvas::ConstellationFilter {
    let highlighted: BTreeSet<String> = m::array(&st["highlighted_ids"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let path_nodes: BTreeSet<String> = m::array(&st["path_nodes"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let path_edges: BTreeSet<(String, String)> = m::array(&st["path_edges"])
        .into_iter()
        .filter_map(|v| {
            let arr = v.as_array()?;
            Some((
                arr.first()?.as_str()?.to_owned(),
                arr.get(1)?.as_str()?.to_owned(),
            ))
        })
        .collect();
    let mut anchored_positions = BTreeMap::new();
    let sig = anchor_filter_signature(st);
    if paged_count > 1 && m::data("omnigraph-anchor-filter-key").as_str() == Some(sig.as_str()) {
        if let Some(obj) = m::data("omnigraph-raw-positions").as_object() {
            for (k, v) in obj {
                if let (Some(x), Some(y)) = (v[0].as_f64(), v[1].as_f64()) {
                    anchored_positions.insert(k.clone(), (x, y));
                }
            }
        }
    }
    let pinned_positions = graph_viewport::load_pins(&omni_pin_key());
    let (vw, vh, measured) = viewport_size_measured();
    let viewport_bounds = (measured && cam.scale > 0.05).then(|| {
        (
            -cam.pan_x / cam.scale,
            -cam.pan_y / cam.scale,
            vw / cam.scale,
            vh / cam.scale,
        )
    });
    let isolate_active = m::yes(&st["isolate_active"]);
    let isolate_depth = if isolate_active {
        st["isolate_depth"].as_u64().unwrap_or(1).clamp(1, 3) as usize
    } else {
        0
    };
    let selected_id = st["selected_id"].as_str().map(str::to_owned);
    let isolate_seed = st["isolate_seed"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| selected_id.clone());

    omnigraph_canvas::ConstellationFilter {
        query: m::text(st, "query"),
        kind: m::text(st, "kind"),
        branch: m::text(st, "branch"),
        show_files: st["show_files"].as_bool().unwrap_or(true),
        show_sessions: st["show_sessions"].as_bool().unwrap_or(true),
        selected_id,
        highlighted_ids: highlighted,
        anchored_positions,
        pinned_positions,
        path_nodes,
        path_edges,
        isolate_seed,
        isolate_depth,
        viewport_bounds,
        paged_count,
        focused_id: st["focused_id"].as_str().map(str::to_owned),
    }
}

#[derive(Clone, Debug)]
pub(super) struct OmniDenseCacheEntry {
    pub(super) node_rects: Vec<(String, f64, f64, f64, f64)>,
    pub(super) node_titles: Vec<String>,
    pub(super) node_index_by_id: HashMap<String, usize>,
    pub(super) node_htmls: Vec<String>,
    pub(super) edge_endpoints: Vec<(String, String)>,
    pub(super) node_edges: Vec<(usize, usize)>,
    pub(super) is_dense: bool,
    pub(super) latest_selected_id: Option<String>,
    pub(super) latest_focused_id: Option<String>,
    pub(super) latest_highlighted_ids: BTreeSet<String>,
    pub(super) latest_path_nodes: BTreeSet<String>,
    pub(super) latest_path_edges: BTreeSet<(String, String)>,
    pub(super) latest_neighbors: BTreeSet<String>,
    pub(super) pinned_ids: BTreeSet<String>,
}

thread_local! {
    pub(super) static OMNI_DENSE_CACHE: RefCell<Option<OmniDenseCacheEntry>> = const { RefCell::new(None) };
}

pub(super) fn patch_omni_node_el(
    el: &web_sys::Element,
    id: &str,
    selected: Option<&str>,
    neighbors: &BTreeSet<String>,
    highlighted_ids: &BTreeSet<String>,
    path_nodes: &BTreeSet<String>,
    roving_target: Option<&str>,
) {
    let has_sel = selected.is_some();
    let has_hl = !highlighted_ids.is_empty();
    let has_path = !path_nodes.is_empty();
    let is_sel = selected == Some(id);
    let is_nbr = !is_sel && neighbors.contains(id);
    let is_hl = highlighted_ids.contains(id) || path_nodes.contains(id);
    let is_dim = if has_path {
        !path_nodes.contains(id)
    } else if has_sel {
        !is_sel && !is_nbr && !is_hl
    } else if has_hl {
        !is_hl
    } else {
        false
    };
    let b = |v: bool| if v { "true" } else { "false" };
    let _ = el.set_attribute("data-selected", b(is_sel));
    let _ = el.set_attribute("data-neighbor", b(is_nbr));
    let _ = el.set_attribute("data-highlighted", b(is_hl));
    let _ = el.set_attribute("data-dimmed", b(is_dim));
    let _ = el.set_attribute("aria-pressed", b(is_sel));
    let _ = el.set_attribute(
        "tabindex",
        if roving_target == Some(id) { "0" } else { "-1" },
    );
}

pub(super) fn is_important_omni_node(
    entry: &OmniDenseCacheEntry,
    id: &str,
    el: Option<&web_sys::Element>,
) -> bool {
    let el_important = el.is_some_and(|e| {
        e.get_attribute("data-selected").as_deref() == Some("true")
            || e.get_attribute("data-neighbor").as_deref() == Some("true")
            || e.get_attribute("data-highlighted").as_deref() == Some("true")
            || e.get_attribute("data-pinned").as_deref() == Some("true")
            || e.get_attribute("tabindex").as_deref() == Some("0")
    });
    el_important
        || entry.latest_selected_id.as_deref() == Some(id)
        || entry.latest_focused_id.as_deref() == Some(id)
        || entry.latest_neighbors.contains(id)
        || entry.latest_highlighted_ids.contains(id)
        || entry.latest_path_nodes.contains(id)
        || entry.pinned_ids.contains(id)
}

pub(super) fn mount_omni_dense_node_if_needed(node_id: &str) -> Option<web_sys::Element> {
    let canvas = dom::get("omnigraph-canvas")?;
    if let Some(existing) = find_omni_node_el(&canvas, node_id) {
        let _ = existing.remove_attribute("data-offscreen");
        return Some(existing);
    }
    let plane = canvas.query_selector(".omni-plane").ok().flatten()?;
    let entry = OMNI_DENSE_CACHE.with(|c| c.borrow().clone())?;
    if !entry.is_dense {
        return None;
    }
    let &idx = entry.node_index_by_id.get(node_id)?;
    let html = entry.node_htmls.get(idx)?.as_str();
    if html.is_empty() {
        return None;
    }
    let mut created = graph_viewport::append_html_elements(&plane, html);
    let el = created.pop()?;
    let _ = el.remove_attribute("data-offscreen");
    let roving_target = entry
        .latest_focused_id
        .as_deref()
        .or(entry.latest_selected_id.as_deref())
        .or_else(|| entry.node_rects.first().map(|(id, ..)| id.as_str()));
    patch_omni_node_el(
        &el,
        node_id,
        entry.latest_selected_id.as_deref(),
        &entry.latest_neighbors,
        &entry.latest_highlighted_ids,
        &entry.latest_path_nodes,
        roving_target,
    );
    Some(el)
}

pub(super) fn sync_omnigraph_viewport_culling(cam: &OmnigraphCamera) {
    let Some(entry) = OMNI_DENSE_CACHE.with(|c| c.borrow().clone()) else {
        return;
    };
    if !entry.is_dense || entry.node_htmls.len() != entry.node_rects.len() {
        return;
    }
    let (vw, vh, measured) = viewport_size_measured();
    let Some(canvas) = dom::get("omnigraph-canvas") else {
        return;
    };
    let nodes = dom::all("#omnigraph-canvas .omni-node[data-node-id]");
    if !measured
        || graph_viewport::world_viewport_bounds(cam.to_viewport(), (vw, vh), 0.0).is_none()
    {
        for el in nodes {
            let _ = el.remove_attribute("data-offscreen");
        }
        return;
    }
    let Some(plane) = canvas.query_selector(".omni-plane").ok().flatten() else {
        return;
    };
    let mut mounted_els: HashMap<usize, web_sys::Element> = HashMap::with_capacity(nodes.len());
    for el in nodes {
        if let Some(id) = el.get_attribute("data-node-id") {
            if let Some(&idx) = entry.node_index_by_id.get(&id) {
                mounted_els.insert(idx, el);
            }
        }
    }
    let rects: Vec<(f64, f64, f64, f64)> = entry
        .node_rects
        .iter()
        .map(|(_, x, y, w, h)| (*x, *y, *w, *h))
        .collect();
    let roving_target = entry
        .latest_focused_id
        .as_deref()
        .or(entry.latest_selected_id.as_deref())
        .or_else(|| entry.node_rects.first().map(|(id, ..)| id.as_str()));
    let entry_ref = &entry;
    let _ = graph_viewport::dense_dom::reconcile_dense_nodes(
        &plane,
        cam.to_viewport(),
        (vw, vh),
        &rects,
        120.0,
        0.0,
        160.0,
        &mut mounted_els,
        |idx, el| {
            let Some((id, ..)) = entry_ref.node_rects.get(idx) else {
                return false;
            };
            if entry_ref.node_htmls.get(idx).is_none_or(|h| h.is_empty()) {
                return false;
            }
            is_important_omni_node(entry_ref, id, el)
        },
        |idx| {
            entry_ref
                .node_htmls
                .get(idx)
                .filter(|s| !s.is_empty())
                .cloned()
        },
        |idx, el| {
            let _ = el.remove_attribute("data-offscreen");
            if let Some((id, ..)) = entry_ref.node_rects.get(idx) {
                patch_omni_node_el(
                    el,
                    id,
                    entry_ref.latest_selected_id.as_deref(),
                    &entry_ref.latest_neighbors,
                    &entry_ref.latest_highlighted_ids,
                    &entry_ref.latest_path_nodes,
                    roving_target,
                );
            }
        },
        |_| {},
    );
}

pub(super) fn refresh_omni_dense_scene(
    scene: &mut graph_viewport::canvas2d::DenseScene,
    filter: &omnigraph_canvas::ConstellationFilter,
    neighbors: &BTreeSet<String>,
    node_ids: &[String],
    edge_endpoints: &[(String, String)],
) {
    let selected = filter.selected_id.as_deref();
    let has_sel = selected.is_some();
    let has_hl = !filter.highlighted_ids.is_empty();
    let has_path = !filter.path_nodes.is_empty();
    for node in &mut scene.nodes {
        let Some(id) = node_ids.get(node.index) else {
            continue;
        };
        let is_sel = selected == Some(id.as_str());
        let is_nbr = !is_sel && neighbors.contains(id.as_str());
        let is_hl = filter.highlighted_ids.contains(id) || filter.path_nodes.contains(id);
        let is_dim = if has_path {
            !filter.path_nodes.contains(id)
        } else if has_sel {
            !is_sel && !is_nbr && !is_hl
        } else if has_hl {
            !is_hl
        } else {
            false
        };
        node.selected = is_sel;
        node.lit = is_nbr || is_hl;
        node.dim = is_dim;
    }
    for edge in &mut scene.edges {
        let Some((from, to)) = edge_endpoints.get(edge.index) else {
            continue;
        };
        let is_sel_edge = selected.is_some_and(|s| from == s || to == s);
        let is_path_edge = filter.path_edges.contains(&(from.clone(), to.clone()))
            || filter.path_edges.contains(&(to.clone(), from.clone()));
        let is_hl_edge =
            filter.highlighted_ids.contains(from) || filter.highlighted_ids.contains(to);
        let lit = is_path_edge
            || is_sel_edge
            || (filter.highlighted_ids.contains(from) && filter.highlighted_ids.contains(to));
        let dim = if has_path {
            !is_path_edge
        } else if has_sel {
            !is_sel_edge && !is_hl_edge
        } else if has_hl {
            !is_hl_edge
        } else {
            false
        };
        edge.on = is_sel_edge || is_path_edge;
        edge.lit = lit;
        edge.dim = dim;
    }
}

pub(super) fn try_patch_constellation_selection_in_place(
    filter: &omnigraph_canvas::ConstellationFilter,
    node_edges: &[(usize, usize)],
    node_ids: &[String],
) -> bool {
    let Some(canvas) = dom::get("omnigraph-canvas") else {
        return false;
    };
    let Some(plane) = canvas.query_selector(".omni-plane").ok().flatten() else {
        return false;
    };
    let selected = filter.selected_id.as_deref();
    let index_by_id: HashMap<&str, usize> = node_ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    let mut neighbors = BTreeSet::new();
    if let Some(sel) = selected {
        if let Some(&sel_idx) = index_by_id.get(sel) {
            for &(u, v) in node_edges {
                if u == sel_idx {
                    if let Some(id) = node_ids.get(v) {
                        neighbors.insert(id.clone());
                    }
                } else if v == sel_idx {
                    if let Some(id) = node_ids.get(u) {
                        neighbors.insert(id.clone());
                    }
                }
            }
        }
    }
    let has_sel = selected.is_some();
    let has_hl = !filter.highlighted_ids.is_empty();
    let has_path = !filter.path_nodes.is_empty();
    let roving_target = filter
        .focused_id
        .as_deref()
        .or(selected)
        .or_else(|| node_ids.first().map(String::as_str));

    let mut mounted_ids = HashSet::new();
    if let Ok(nodes) = canvas.query_selector_all(".omni-node[data-node-id]") {
        for i in 0..nodes.length() {
            let Some(el) = nodes
                .item(i)
                .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
            else {
                continue;
            };
            let id = el.get_attribute("data-node-id").unwrap_or_default();
            mounted_ids.insert(id.clone());
            patch_omni_node_el(
                &el,
                &id,
                selected,
                &neighbors,
                &filter.highlighted_ids,
                &filter.path_nodes,
                roving_target,
            );
        }
    }

    let mut is_dense = false;
    let mut edge_endpoints = Vec::new();
    OMNI_DENSE_CACHE.with(|cache| {
        if let Some(entry) = cache.borrow_mut().as_mut() {
            is_dense = entry.is_dense;
            edge_endpoints = entry.edge_endpoints.clone();
            entry.latest_selected_id = filter.selected_id.clone();
            entry.latest_focused_id = filter.focused_id.clone();
            entry.latest_highlighted_ids = filter.highlighted_ids.clone();
            entry.latest_path_nodes = filter.path_nodes.clone();
            entry.latest_path_edges = filter.path_edges.clone();
            entry.latest_neighbors = neighbors.clone();
            if entry.is_dense && entry.node_htmls.len() == entry.node_rects.len() {
                let mut batch_html = String::new();
                let mut batch_ids = Vec::new();
                for (idx, (id, ..)) in entry.node_rects.iter().enumerate() {
                    if mounted_ids.contains(id) {
                        continue;
                    }
                    let important = selected == Some(id.as_str())
                        || neighbors.contains(id)
                        || filter.highlighted_ids.contains(id)
                        || filter.path_nodes.contains(id)
                        || roving_target == Some(id.as_str());
                    if important {
                        if let Some(html) = entry.node_htmls.get(idx).filter(|s| !s.is_empty()) {
                            batch_html.push_str(html);
                            batch_ids.push(id.as_str());
                        }
                    }
                }
                if !batch_html.is_empty() {
                    let inserted = graph_viewport::append_html_elements(&plane, &batch_html);
                    for (id, el) in batch_ids.into_iter().zip(inserted) {
                        patch_omni_node_el(
                            &el,
                            id,
                            selected,
                            &neighbors,
                            &filter.highlighted_ids,
                            &filter.path_nodes,
                            roving_target,
                        );
                    }
                }
            }
        }
    });

    if is_dense {
        graph_viewport::canvas2d::update_scene("omnigraph-canvas", |scene| {
            refresh_omni_dense_scene(scene, filter, &neighbors, node_ids, &edge_endpoints);
        });
    } else if let Ok(edges) = canvas.query_selector_all(".omni-edge[data-from][data-to]") {
        for i in 0..edges.length() {
            let Some(el) = edges
                .item(i)
                .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
            else {
                continue;
            };
            let from = el.get_attribute("data-from").unwrap_or_default();
            let to = el.get_attribute("data-to").unwrap_or_default();
            let is_sel = selected.is_some_and(|s| from == s || to == s);
            let is_path = filter.path_edges.contains(&(from.clone(), to.clone()))
                || filter.path_edges.contains(&(to.clone(), from.clone()));
            let is_hl =
                filter.highlighted_ids.contains(&from) || filter.highlighted_ids.contains(&to);
            let lit = is_path
                || is_sel
                || (filter.highlighted_ids.contains(&from) && filter.highlighted_ids.contains(&to));
            let dim = if has_path {
                !is_path
            } else if has_sel {
                !is_sel && !is_hl
            } else if has_hl {
                !is_hl
            } else {
                false
            };
            let b = |v: bool| if v { "true" } else { "false" };
            let _ = el.set_attribute("data-selected", b(is_sel || is_path));
            let _ = el.set_attribute("data-lit", b(lit));
            let _ = el.set_attribute("data-dimmed", b(dim));
            let _ = el.set_attribute("data-path", b(is_path));
        }
    }
    if let Some(radar) = dom::get("omnigraph-canvas-radar") {
        if let Ok(dots) = radar.query_selector_all(".omni-radar-dot[data-node-id]") {
            let b = |v: bool| if v { "true" } else { "false" };
            for i in 0..dots.length() {
                if let Some(dot) = dots
                    .item(i)
                    .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
                {
                    let nid = dot.get_attribute("data-node-id").unwrap_or_default();
                    let _ = dot.set_attribute("data-selected", b(selected == Some(nid.as_str())));
                    let _ = dot.set_attribute(
                        "data-highlighted",
                        b(filter.highlighted_ids.contains(&nid)),
                    );
                }
            }
        }
    }
    true
}

pub(super) fn attach_omni_hover_leave_once() {
    let Some(canvas) = dom::get("omnigraph-canvas") else {
        return;
    };
    if canvas.get_attribute("data-omni-leave-bound").as_deref() == Some("true") {
        return;
    }
    let _ = canvas.set_attribute("data-omni-leave-bound", "true");
    let leave = Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
        let cam = load_camera();
        clear_omni_hover_state(&cam);
    });
    let _ = canvas.add_event_listener_with_callback("pointerleave", leave.as_ref().unchecked_ref());
    leave.forget();
}

pub(super) fn title_for_omni_node(graph: &V, id: &str) -> String {
    for a in m::array(&graph["atoms"]) {
        if atom_id(&a) == id {
            if let Some(t) = a["title"].as_str().filter(|s| !s.is_empty()) {
                return t.to_owned();
            }
        }
    }
    for s in m::array(&graph["sessions"]) {
        if session_node_id(&s) == id {
            let t = m::summary(&s);
            if !t.is_empty() {
                return t;
            }
        }
    }
    for sym in m::array(&graph["symbols"]) {
        if symbol_node_id(&sym) == id || sym["fqn"].as_str() == Some(id) {
            return symbol_display_label(&sym);
        }
    }
    id.rsplit('/').next().unwrap_or(id).to_owned()
}

pub(super) fn render_omni_history_bar() {
    let Some(tools) = dom::get("omni-canvas-tools") else {
        return;
    };
    let bar = if let Some(existing) = dom::get("omni-history-bar") {
        existing
    } else {
        let el = dom::el("div", "omni-history-bar graph-history-group", "");
        let _ = el.set_attribute("id", "omni-history-bar");
        let _ = el.set_attribute("role", "group");
        let _ = el.set_attribute("aria-label", "Constellation navigation history");
        let _ = tools.append_child(&el);
        el
    };
    let (can_back, can_fwd, crumb) = OMNI_NAV_HISTORY.with(|h| {
        let hist = h.borrow();
        let label = hist
            .current()
            .and_then(|e| parse_omni_nav_entry(e).0)
            .unwrap_or_else(|| "Overview".to_owned());
        (hist.can_back(), hist.can_forward(), label)
    });
    let back_dis = if can_back { "" } else { " disabled" };
    let fwd_dis = if can_fwd { "" } else { " disabled" };
    bar.set_inner_html(&format!(
        "<button type=\"button\" class=\"button compact session-graph-btn\" id=\"omni-history-back\" data-action=\"omnigraph-history-back\" title=\"Navigate back in Constellation\" aria-label=\"Navigate back\"{back_dis}>←</button><button type=\"button\" class=\"button compact session-graph-btn\" id=\"omni-history-forward\" data-action=\"omnigraph-history-forward\" title=\"Navigate forward in Constellation\" aria-label=\"Navigate forward\"{fwd_dis}>→</button><span class=\"omni-meta-pill\" id=\"omni-history-crumb\">{}</span>",
        esc(crumb.rsplit('/').next().unwrap_or(&crumb))
    ));
}

pub(super) fn render_constellation() {
    let mut graph = m::data("omnigraph-graph");
    if !graph.is_object() {
        OMNI_DENSE_CACHE.with(|c| *c.borrow_mut() = None);
        OMNI_HOVER_HIT.with(|h| *h.borrow_mut() = None);
        graph_viewport::canvas2d::clear_scene("omnigraph-canvas");
        if let Some(el) = dom::get("omnigraph-canvas") {
            el.set_inner_html("");
        }
        if let Some(radar) = dom::get("omnigraph-canvas-radar") {
            radar.set_inner_html("");
            dom::hide(&radar, true);
        }
        return;
    }
    attach_omni_hover_leave_once();
    let st = ui_state();
    if m::text(&st, "snapshot").trim().is_empty() {
        let project = current_project_id();
        let (changes_doc, impact_doc) = resolve_workspace_drift_docs(&project);
        omnigraph_canvas::inject_workspace_drift(
            &mut graph,
            changes_doc.as_ref(),
            impact_doc.as_ref(),
        );
    }
    let paged_count = m::data("omnigraph-page-count").as_u64().unwrap_or(1).max(1) as usize;
    let mut cam = load_camera();
    let filter = current_constellation_filter(&st, &cam, paged_count);
    let has_pins = !filter.pinned_positions.is_empty();
    dom::hidden("omni-reset-pins", !has_pins);
    dom::attr(
        "omni-isolate-toggle",
        "aria-pressed",
        if filter.isolate_depth > 0 {
            "true"
        } else {
            "false"
        },
    );
    dom::hidden("omni-path-clear", filter.path_nodes.is_empty());

    let new_digest = omnigraph_canvas::constellation_structure_digest(&graph, &filter);
    let prev_digest = m::data("omnigraph-structure-digest").as_u64().unwrap_or(0);
    let cached_edges: Vec<(usize, usize)> = m::array(&m::data("omnigraph-nav-edges"))
        .into_iter()
        .filter_map(|v| {
            let a = v.as_array()?;
            Some((a.first()?.as_u64()? as usize, a.get(1)?.as_u64()? as usize))
        })
        .collect();
    let cached_ids: Vec<String> = m::array(&m::data("omnigraph-nav-rects"))
        .into_iter()
        .filter_map(|v| v["id"].as_str().map(str::to_owned))
        .collect();

    if prev_digest == new_digest
        && prev_digest != 0
        && !cached_ids.is_empty()
        && try_patch_constellation_selection_in_place(&filter, &cached_edges, &cached_ids)
    {
        sync_omnigraph_viewport_culling(&cam);
        let hover = OMNI_HOVER_HIT.with(|h| h.borrow().flatten());
        graph_viewport::canvas2d::paint_scene("omnigraph-canvas", cam.to_viewport(), hover, true);
        render_omni_history_bar();
        return;
    }

    let viewport = omnigraph_canvas::CanvasViewport {
        pan_x: cam.pan_x,
        pan_y: cam.pan_y,
        scale: cam.scale,
    };
    let rendered = omnigraph_canvas::render_constellation(&graph, &filter, &viewport);
    OMNI_HOVER_HIT.with(|h| *h.borrow_mut() = None);
    if let Some(el) = dom::get("omnigraph-canvas") {
        el.set_inner_html(&rendered.html);
    }
    if let Some(radar) = dom::get("omnigraph-canvas-radar") {
        radar.set_inner_html(&rendered.radar_html);
        dom::hide(&radar, rendered.radar_html.trim().is_empty());
    }
    m::set(
        "omnigraph-structure-digest",
        json!(rendered.structure_digest),
    );
    let node_index_by_id: HashMap<String, usize> = rendered
        .node_rects
        .iter()
        .enumerate()
        .map(|(i, (id, ..))| (id.clone(), i))
        .collect();
    let node_titles: Vec<String> = rendered
        .node_rects
        .iter()
        .map(|(id, ..)| title_for_omni_node(&graph, id))
        .collect();
    let is_dense = rendered.dense_scene.is_some();
    if let Some(scene) = rendered.dense_scene {
        let meta = build_omni_dense_scene_meta(
            &graph,
            &rendered.node_rects,
            &rendered.edge_endpoints,
            &node_titles,
            &node_index_by_id,
        );
        graph_viewport::canvas2d::store_scene_with_meta("omnigraph-canvas", scene, meta);
    } else {
        graph_viewport::canvas2d::clear_scene("omnigraph-canvas");
    }
    let mut cache_neighbors = BTreeSet::new();
    if let Some(sel) = filter.selected_id.as_deref() {
        if let Some(&sel_idx) = node_index_by_id.get(sel) {
            for &(u, v) in &rendered.node_edges {
                if u == sel_idx {
                    if let Some((id, ..)) = rendered.node_rects.get(v) {
                        cache_neighbors.insert(id.clone());
                    }
                } else if v == sel_idx {
                    if let Some((id, ..)) = rendered.node_rects.get(u) {
                        cache_neighbors.insert(id.clone());
                    }
                }
            }
        }
    }
    let cache_entry = OmniDenseCacheEntry {
        node_rects: rendered.node_rects.clone(),
        node_titles,
        node_index_by_id,
        node_htmls: rendered.node_htmls,
        edge_endpoints: rendered.edge_endpoints,
        node_edges: rendered.node_edges.clone(),
        is_dense,
        latest_selected_id: filter.selected_id.clone(),
        latest_focused_id: filter.focused_id.clone(),
        latest_highlighted_ids: filter.highlighted_ids.clone(),
        latest_path_nodes: filter.path_nodes.clone(),
        latest_path_edges: filter.path_edges.clone(),
        latest_neighbors: cache_neighbors,
        pinned_ids: filter.pinned_positions.keys().cloned().collect(),
    };
    OMNI_DENSE_CACHE.with(|cache| {
        *cache.borrow_mut() = Some(cache_entry);
    });

    let pos_map: V = rendered
        .positions
        .into_iter()
        .map(|(k, (x, y))| (k, json!([x, y])))
        .collect::<serde_json::Map<String, V>>()
        .into();
    m::set("omnigraph-positions", pos_map);

    let raw_map: V = rendered
        .raw_positions
        .into_iter()
        .map(|(k, (x, y))| (k, json!([x, y])))
        .collect::<serde_json::Map<String, V>>()
        .into();
    m::set("omnigraph-raw-positions", raw_map);
    m::set(
        "omnigraph-anchor-filter-key",
        json!(anchor_filter_signature(&st)),
    );

    let nav_rects: Vec<V> = rendered
        .node_rects
        .iter()
        .map(|(id, x, y, w, h)| json!({ "id": id, "x": x, "y": y, "w": w, "h": h }))
        .collect();
    m::set("omnigraph-nav-rects", V::Array(nav_rects));
    let nav_edges: Vec<V> = rendered
        .node_edges
        .iter()
        .map(|(u, v)| json!([u, v]))
        .collect();
    m::set("omnigraph-nav-edges", V::Array(nav_edges));

    if rendered.node_count == 0 {
        dom::attr("omnigraph-canvas", "data-lod", "near");
        dom::text("omni-zoom-level", "100%");
        if let Some(radar) = dom::get("omnigraph-canvas-radar") {
            dom::hide(&radar, true);
        }
    } else {
        cam.plane_w = rendered.width;
        cam.plane_h = rendered.height;
        save_camera(&cam);
        if cam.fit_mode
            && filter.anchored_positions.is_empty()
            && filter.pinned_positions.is_empty()
        {
            fit_camera();
        } else {
            apply_camera(&cam);
        }
    }
    render_omni_history_bar();
}

pub(super) fn render_dag() {
    let graph = m::data("omnigraph-graph");
    let Some(el) = dom::get("omnigraph-dag-container") else {
        return;
    };
    if !graph.is_object() {
        el.set_inner_html("<p class=\"empty-note\">No branch history loaded for this project.</p>");
        return;
    }
    let st = ui_state();
    let branch = m::text(&st, "branch");
    let snapshot = st["snapshot"].as_str();
    let html = omnigraph_canvas::render_branch_dag(&graph, &branch, snapshot);
    el.set_inner_html(&html);
}

pub(super) fn render_merge() {
    let diff = m::data("omnigraph-diff");
    let Some(el) = dom::get("omnigraph-merge-container") else {
        return;
    };
    if !diff.is_object() {
        el.set_inner_html("<p class=\"empty-note\">Choose a source and target branch above, then click <strong>Preview Merge</strong> to inspect additions, updates, deletions, and 3-way merge conflicts.</p>");
        return;
    }
    el.set_inner_html(&omnigraph_canvas::render_merge_diff(&diff));
}

pub(super) fn render_inspector() {
    let search_res = m::data("omnigraph-search-results");
    let st = ui_state();
    let query = m::text(&st, "query");
    if let Some(hits_el) = dom::get("omnigraph-search-hits") {
        if query.trim().is_empty() || !search_res.is_object() {
            dom::hide(&hits_el, true);
            hits_el.set_inner_html("");
        } else {
            hits_el.set_inner_html(&render_search_hits_html(&search_res));
            dom::hide(&hits_el, false);
        }
    }
    let Some(body) = dom::get("omnigraph-inspector-body") else {
        return;
    };
    let snapshot = m::text(&st, "snapshot");
    let selected_id = m::text(&st, "selected_id");
    let commit_detail = m::data("omnigraph-commit-detail");
    if selected_id.is_empty() && !snapshot.is_empty() && commit_detail.is_object() {
        body.set_inner_html(&render_commit_inspector_html(&commit_detail));
        return;
    }
    let graph = m::data("omnigraph-graph");
    let sessions = m::read(|s| s.sessions.clone());
    let branch = m::text(&st, "branch");
    let mut html = render_node_inspector_html(&graph, &selected_id, &sessions, &branch);
    if !selected_id.is_empty() {
        let mut extras = Vec::new();
        if graph_viewport::load_pins(&omni_pin_key()).contains_key(&selected_id) {
            extras.push(format!(
                "<button type=\"button\" class=\"button compact\" id=\"omni-unpin-node\" data-action=\"omnigraph-unpin-node\" data-payload=\"{}\">Unpin node</button>",
                esc(&json!({ "node_id": selected_id }).to_string())
            ));
        }
        let sys_cfg = m::data("system-map-catalog-cache");
        let has_sys = sys_cfg.is_object()
            && (!m::array(&sys_cfg["services"]).is_empty()
                || !dom::value("system-project").is_empty());
        if has_sys {
            extras.push(format!(
                "<button type=\"button\" class=\"button compact\" data-action=\"omnigraph-open-system-map\" data-payload=\"{}\">Open in System Map ↗</button>",
                esc(&json!({ "node_id": selected_id }).to_string())
            ));
        }
        if !extras.is_empty() {
            html.push_str(&format!(
                "<div class=\"omni-inspector-card\"><div class=\"omni-chip-list\">{}</div></div>",
                extras.join("")
            ));
        }
        let arch_card = build_arch_inspector_html(&graph, &selected_id);
        if !arch_card.is_empty() {
            html.push_str(&arch_card);
        }
    }
    body.set_inner_html(&html);
}

pub(super) fn build_arch_inspector_html(graph: &V, selected_id: &str) -> String {
    if selected_id.is_empty() || !graph.is_object() {
        return String::new();
    }
    let mut target_files: BTreeSet<String> = BTreeSet::new();
    for f in m::array(&graph["files"]) {
        let fid = file_node_id(&f);
        let path = first_omni_field(&f, &["path", "file_path", "name", "label"]);
        if fid == selected_id || path == selected_id {
            if !path.is_empty() {
                target_files.insert(path);
            } else {
                target_files.insert(fid);
            }
        }
    }
    for s in m::array(&graph["symbols"]) {
        if symbol_node_id(&s) == selected_id || s["fqn"].as_str() == Some(selected_id) {
            let fp = symbol_file_path(&s);
            if !fp.is_empty() {
                target_files.insert(fp);
            }
        }
    }
    for a in m::array(&graph["atoms"]) {
        if atom_id(&a) == selected_id {
            for f in m::array(&a["files"]) {
                if let Some(p) = f.as_str().filter(|s| !s.is_empty()) {
                    target_files.insert(p.to_owned());
                }
            }
        }
    }
    if target_files.is_empty() {
        target_files.insert(selected_id.to_owned());
    }

    let mut items = Vec::new();
    for bm in m::array(&graph["blast_markers"]) {
        let p = first_omni_field(&bm, &["path", "file_path"]);
        if target_files.contains(&p) {
            let bf = bm["blast_files"].as_u64().unwrap_or(0);
            let bt = bm["blast_tests"].as_u64().unwrap_or(0);
            let risk = first_omni_field(&bm, &["risk"]);
            let in_cycle = bm["in_cycle"].as_bool().unwrap_or(false);
            let cycle_txt = if in_cycle { " · in cycle" } else { "" };
            items.push(format!(
                "<p class=\"omni-arch-blast-summary\"><strong>Blast ({}):</strong> {bf} files, {bt} tests{cycle_txt}</p>",
                esc(&risk),
            ));
        }
    }
    let mut viol_list = Vec::new();
    for v in m::array(&graph["architecture_violations"]) {
        let kind = first_omni_field(&v, &["kind"]);
        let from = first_omni_field(&v, &["from"]);
        let to = first_omni_field(&v, &["to"]);
        let to_is_file = matches!(kind.as_str(), "layer" | "package");
        if target_files.contains(&from) || (to_is_file && target_files.contains(&to)) {
            let msg = first_omni_field(&v, &["message"]);
            viol_list.push(format!(
                "<li data-violation-kind=\"{}\"><strong>{}:</strong> {} ({} → {})</li>",
                esc(&kind),
                esc(&kind),
                esc(&msg),
                esc(&from),
                esc(&to),
            ));
        }
    }
    if !viol_list.is_empty() {
        items.push(format!(
            "<ul class=\"omni-arch-violation-list\">{}</ul>",
            viol_list.join("")
        ));
    }
    let mut sem_list = Vec::new();
    for se in m::array(&graph["ir_semantic_edges"]) {
        let fp = first_omni_field(&se, &["file_path", "source_file", "source"]);
        if target_files.contains(&fp) {
            let rel = first_omni_field(&se, &["relation", "kind"]);
            let tgt = first_omni_field(&se, &["target"]);
            if !rel.is_empty() && !tgt.is_empty() {
                sem_list.push(format!(
                    "<li><code>{}</code> → <code>{}</code></li>",
                    esc(&rel),
                    esc(&tgt)
                ));
            }
        }
    }
    if !sem_list.is_empty() {
        items.push(format!(
            "<ul class=\"omni-arch-semantic-list\">{}</ul>",
            sem_list.into_iter().take(12).collect::<Vec<_>>().join("")
        ));
    }
    if items.is_empty() {
        String::new()
    } else {
        format!(
            "<div class=\"omni-inspector-card omni-arch-inspector\"><h4>Code Graph &amp; Architecture</h4>{}</div>",
            items.join("")
        )
    }
}

pub(super) fn render_pulse() {
    let events = m::array(&m::data("omnigraph-pulse-events"));
    dom::text("omni-pulse-count", &events.len().to_string());
    if let Some(el) = dom::get("omnigraph-pulse-list") {
        el.set_inner_html(&render_pulse_list_html(&events));
    }
}

#[derive(Clone, Debug, Default)]
struct OmniNodeExplainInfo {
    node_type: String,
    kind: String,
    branch: String,
    file_path: String,
    symbol_fqn: String,
}

fn first_omni_field(item: &V, keys: &[&str]) -> String {
    for &key in keys {
        if let Some(val) = item[key].as_str().map(str::trim).filter(|s| !s.is_empty()) {
            return val.to_owned();
        }
    }
    String::new()
}

pub(super) fn build_omni_dense_scene_meta(
    graph: &V,
    node_rects: &[(String, f64, f64, f64, f64)],
    edge_endpoints: &[(String, String)],
    node_titles: &[String],
    node_index_by_id: &HashMap<String, usize>,
) -> graph_viewport::canvas2d::DenseSceneMeta {
    let mut node_info: HashMap<String, OmniNodeExplainInfo> = HashMap::new();
    let mut aliases: HashMap<String, String> = HashMap::new();

    for s in m::array(&graph["sessions"]) {
        let id = session_node_id(&s);
        if id.is_empty() || node_info.contains_key(&id) {
            continue;
        }
        let branch = first_omni_field(&s, &["branch", "source_branch", "graph_branch"]);
        node_info.insert(
            id,
            OmniNodeExplainInfo {
                node_type: "session".to_owned(),
                kind: "session".to_owned(),
                branch: if branch.is_empty() {
                    "main".to_owned()
                } else {
                    branch
                },
                file_path: String::new(),
                symbol_fqn: String::new(),
            },
        );
    }

    for a in m::array(&graph["atoms"]) {
        let id = atom_id(&a);
        if id.is_empty() || node_info.contains_key(&id) {
            continue;
        }
        let kind = first_omni_field(&a, &["kind"]);
        let branch = first_omni_field(&a, &["branch", "source_branch", "graph_branch"]);
        node_info.insert(
            id,
            OmniNodeExplainInfo {
                node_type: "atom".to_owned(),
                kind: if kind.is_empty() {
                    "insight".to_owned()
                } else {
                    kind
                },
                branch: if branch.is_empty() {
                    "main".to_owned()
                } else {
                    branch
                },
                file_path: first_omni_field(&a, &["file_path", "path"]),
                symbol_fqn: first_omni_field(&a, &["symbol_fqn", "fqn"]),
            },
        );
    }

    for f in m::array(&graph["files"]) {
        let (id, path) = if let Some(s) = f.as_str() {
            (s.to_owned(), s.to_owned())
        } else {
            let path = first_omni_field(&f, &["path", "file_path", "name", "label"]);
            let id = first_omni_field(&f, &["id", "@id"]);
            let resolved_id = if id.is_empty() { path.clone() } else { id };
            let resolved_path = if path.is_empty() {
                resolved_id.clone()
            } else {
                path
            };
            (resolved_id, resolved_path)
        };
        if id.is_empty() {
            continue;
        }
        if !path.is_empty() && path != id {
            aliases.entry(path.clone()).or_insert_with(|| id.clone());
        }
        if node_info.contains_key(&id) {
            continue;
        }
        let branch = if f.is_object() {
            first_omni_field(&f, &["branch", "source_branch", "graph_branch"])
        } else {
            String::new()
        };
        node_info.insert(
            id,
            OmniNodeExplainInfo {
                node_type: "file".to_owned(),
                kind: "file".to_owned(),
                branch,
                file_path: path,
                symbol_fqn: String::new(),
            },
        );
    }

    for sym in m::array(&graph["symbols"]) {
        let id = symbol_node_id(&sym);
        if id.is_empty() {
            continue;
        }
        let fqn = {
            let raw = first_omni_field(&sym, &["fqn", "id", "symbol_id"]);
            if raw.is_empty() {
                id.clone()
            } else {
                raw
            }
        };
        if !fqn.is_empty() && fqn != id {
            aliases.entry(fqn.clone()).or_insert_with(|| id.clone());
        }
        let sym_id = first_omni_field(&sym, &["symbol_id"]);
        if !sym_id.is_empty() && sym_id != id {
            aliases.entry(sym_id).or_insert_with(|| id.clone());
        }
        if node_info.contains_key(&id) {
            continue;
        }
        let kind = first_omni_field(&sym, &["kind", "symbol_kind"]);
        let branch = first_omni_field(&sym, &["branch", "source_branch", "graph_branch"]);
        node_info.insert(
            id,
            OmniNodeExplainInfo {
                node_type: "symbol".to_owned(),
                kind: if kind.is_empty() {
                    "symbol".to_owned()
                } else {
                    kind
                },
                branch,
                file_path: symbol_file_path(&sym),
                symbol_fqn: fqn,
            },
        );
    }

    let resolve_alias =
        |raw: &str| -> String { aliases.get(raw).cloned().unwrap_or_else(|| raw.to_owned()) };

    let known_file_set: HashSet<String> = node_info
        .values()
        .filter(|info| !info.file_path.is_empty())
        .map(|info| info.file_path.clone())
        .collect();
    let mut viol_counts: HashMap<String, usize> = HashMap::new();
    let mut cycle_files: HashSet<String> = HashSet::new();
    let mut violated_pairs: HashSet<(String, String)> = HashSet::new();
    for v in m::array(&graph["architecture_violations"]) {
        let kind = first_omni_field(&v, &["kind"]);
        let from = first_omni_field(&v, &["from"]);
        let to = first_omni_field(&v, &["to"]);
        let from_is_file =
            !from.is_empty() && (kind != "cycle_budget" || known_file_set.contains(&from));
        if from_is_file {
            *viol_counts.entry(from.clone()).or_default() += 1;
        }
        let is_edge_file_viol = matches!(kind.as_str(), "layer" | "package");
        if is_edge_file_viol && !to.is_empty() && to != from {
            *viol_counts.entry(to.clone()).or_default() += 1;
        }
        if is_edge_file_viol && !from.is_empty() && !to.is_empty() {
            violated_pairs.insert((from, to));
        }
    }
    let mut blast_by_file: HashMap<String, (u64, u64, usize, bool, String)> = HashMap::new();
    for bm in m::array(&graph["blast_markers"]) {
        let path = first_omni_field(&bm, &["path", "file_path"]);
        if path.is_empty() {
            continue;
        }
        let bf = bm["blast_files"].as_u64().unwrap_or(0);
        let bt = bm["blast_tests"].as_u64().unwrap_or(0);
        let vc = bm["violation_count"].as_u64().unwrap_or(0) as usize;
        let in_cycle = bm["in_cycle"].as_bool().unwrap_or(false);
        let risk = first_omni_field(&bm, &["risk"]);
        if vc > 0 {
            viol_counts
                .entry(path.clone())
                .and_modify(|prev| *prev = (*prev).max(vc))
                .or_insert(vc);
        }
        if in_cycle {
            cycle_files.insert(path.clone());
        }
        blast_by_file.insert(path, (bf, bt, vc, in_cycle, risk));
    }
    let mut semantic_counts: HashMap<String, usize> = HashMap::new();
    for se in m::array(&graph["ir_semantic_edges"]) {
        let fp = first_omni_field(&se, &["file_path", "source_file", "source"]);
        if !fp.is_empty() {
            *semantic_counts.entry(fp).or_default() += 1;
        }
    }

    let mut edge_meta_by_pair: HashMap<(String, String), (String, String)> = HashMap::new();
    for key in [
        "edges",
        "symbol_edges",
        "ir_edges",
        "ir_semantic_edges",
        "links",
    ] {
        for edge in m::array(&graph[key]) {
            let raw_from = first_omni_field(&edge, &["from", "source"]);
            let raw_to = first_omni_field(&edge, &["to", "target"]);
            let kind = first_omni_field(&edge, &["kind", "type", "edge_kind"]);
            let relation = first_omni_field(&edge, &["relation", "rel", "kind", "type"]);
            if !raw_from.is_empty() && !raw_to.is_empty() {
                let from = resolve_alias(&raw_from);
                let to = resolve_alias(&raw_to);
                edge_meta_by_pair
                    .entry((from, to))
                    .or_insert((kind.clone(), relation.clone()));
            }
            let sf = first_omni_field(&edge, &["source_file"]);
            let tf = first_omni_field(&edge, &["target_file"]);
            if !sf.is_empty() && !tf.is_empty() {
                let from = resolve_alias(&sf);
                let to = resolve_alias(&tf);
                edge_meta_by_pair
                    .entry((from, to))
                    .or_insert((kind, relation));
            }
        }
    }

    let mut file_by_node_id: HashMap<String, String> = HashMap::new();
    let mut meta = graph_viewport::canvas2d::DenseSceneMeta::default();
    for (i, (id, ..)) in node_rects.iter().enumerate() {
        let title = node_titles.get(i).cloned().unwrap_or_else(|| id.clone());
        let info = node_info.get(id).cloned().unwrap_or_else(|| {
            let inferred_type = if id.contains("::") {
                "symbol"
            } else if id.contains('/') || id.contains('.') {
                "file"
            } else {
                "node"
            };
            OmniNodeExplainInfo {
                node_type: inferred_type.to_owned(),
                kind: inferred_type.to_owned(),
                branch: String::new(),
                file_path: if inferred_type == "file" {
                    id.clone()
                } else if inferred_type == "symbol" {
                    id.split_once("::")
                        .map_or_else(String::new, |(p, _)| p.to_owned())
                } else {
                    String::new()
                },
                symbol_fqn: if inferred_type == "symbol" {
                    id.clone()
                } else {
                    String::new()
                },
            }
        });
        let fp = info.file_path.clone();
        if !fp.is_empty() {
            file_by_node_id.insert(id.clone(), fp.clone());
        }
        let v_count = viol_counts.get(&fp).copied().unwrap_or(0);
        let in_cycle = cycle_files.contains(&fp);
        let (bf, bt, risk) = blast_by_file
            .get(&fp)
            .map(|(f, t, _, _, r)| (*f, *t, r.clone()))
            .unwrap_or_default();
        let sem_count = semantic_counts.get(&fp).copied().unwrap_or(0);
        let blast_tag = if in_cycle {
            "cycle".to_owned()
        } else if v_count > 0 {
            format!("{v_count} viol")
        } else if bf > 0 {
            format!("blast {bf}f")
        } else {
            String::new()
        };
        let badge_text = if v_count > 0 && bf > 0 {
            format!("!{v_count} · {bf}f")
        } else if v_count > 0 {
            format!("!{v_count}")
        } else if bf > 0 {
            format!("{bf}f")
        } else {
            String::new()
        };
        meta.nodes.insert(
            i,
            graph_viewport::canvas2d::DenseNodeMeta {
                explain_kind: "node".to_owned(),
                explain_payload: json!({
                    "node_id": id,
                    "label": title,
                    "name": title,
                    "node_type": info.node_type,
                    "kind": info.kind,
                    "branch": info.branch,
                    "file_path": info.file_path,
                    "symbol_fqn": info.symbol_fqn,
                    "violation_count": v_count,
                    "blast_files": bf,
                    "blast_tests": bt,
                    "risk": risk,
                    "in_cycle": in_cycle,
                    "semantic_edge_count": sem_count,
                }),
                symbol_kind: info.kind,
                in_cycle,
                blast_tag,
                badge_text,
                tooltip: title,
                file_line: info.file_path,
                ..Default::default()
            },
        );
    }
    for (i, (from, to)) in edge_endpoints.iter().enumerate() {
        let (kind, relation) = edge_meta_by_pair
            .get(&(from.clone(), to.clone()))
            .cloned()
            .unwrap_or_default();
        let from_file = file_by_node_id
            .get(from.as_str())
            .map_or(from.as_str(), String::as_str);
        let to_file = file_by_node_id
            .get(to.as_str())
            .map_or(to.as_str(), String::as_str);
        let arch_violation = violated_pairs.contains(&(from_file.to_owned(), to_file.to_owned()));
        let summary = if kind.is_empty() {
            format!("{from} -> {to}")
        } else {
            format!("{from} -> {to} ({kind})")
        };
        meta.edges.insert(
            i,
            graph_viewport::canvas2d::DenseEdgeMeta {
                explain_kind: "link".to_owned(),
                explain_payload: json!({
                    "from": from,
                    "to": to,
                    "kind": kind,
                    "relation": relation,
                    "arch_violation": arch_violation,
                }),
                from_index: node_index_by_id.get(from.as_str()).copied(),
                to_index: node_index_by_id.get(to.as_str()).copied(),
                summary,
            },
        );
    }
    meta
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omnigraph_dense_scene_meta_populates_explain_fields_for_all_node_types() {
        let graph = json!({
            "sessions": [{
                "id": "sess-1",
                "title": "Auth refactor",
                "branch": "feat/auth"
            }],
            "atoms": [{
                "id": "atom-1",
                "kind": "decision",
                "title": "Use Ed25519 tokens",
                "branch": "knowledge/canonical"
            }],
            "files": [{
                "id": "file:src/auth.rs",
                "path": "src/auth.rs",
                "branch": "main"
            }],
            "symbols": [{
                "id": "sym-1",
                "fqn": "src/auth.rs::verify_token",
                "kind": "function",
                "file_path": "src/auth.rs",
                "branch": "feat/auth"
            }],
            "edges": [{
                "from": "atom-1",
                "to": "src/auth.rs",
                "kind": "mentions"
            }]
        });
        let node_rects = vec![
            ("sess-1".to_owned(), 0.0, 0.0, 120.0, 48.0),
            ("atom-1".to_owned(), 0.0, 80.0, 120.0, 48.0),
            ("file:src/auth.rs".to_owned(), 0.0, 160.0, 120.0, 48.0),
            ("sym-1".to_owned(), 0.0, 240.0, 120.0, 48.0),
        ];
        let node_titles: Vec<String> = node_rects
            .iter()
            .map(|(id, ..)| title_for_omni_node(&graph, id))
            .collect();
        let node_index_by_id: HashMap<String, usize> = node_rects
            .iter()
            .enumerate()
            .map(|(i, (id, ..))| (id.clone(), i))
            .collect();
        let edge_endpoints = vec![("atom-1".to_owned(), "file:src/auth.rs".to_owned())];
        let meta = build_omni_dense_scene_meta(
            &graph,
            &node_rects,
            &edge_endpoints,
            &node_titles,
            &node_index_by_id,
        );

        assert_eq!(
            meta.nodes[&0].explain_payload["node_type"],
            json!("session")
        );
        assert_eq!(meta.nodes[&0].explain_payload["kind"], json!("session"));
        assert_eq!(meta.nodes[&0].explain_payload["branch"], json!("feat/auth"));

        assert_eq!(meta.nodes[&1].explain_payload["node_type"], json!("atom"));
        assert_eq!(meta.nodes[&1].explain_payload["kind"], json!("decision"));

        assert_eq!(meta.nodes[&2].explain_payload["node_type"], json!("file"));
        assert_eq!(meta.nodes[&2].explain_payload["kind"], json!("file"));
        assert_eq!(
            meta.nodes[&2].explain_payload["file_path"],
            json!("src/auth.rs")
        );
        assert_eq!(meta.nodes[&2].file_line, "src/auth.rs");

        assert_eq!(meta.nodes[&3].explain_payload["node_type"], json!("symbol"));
        assert_eq!(meta.nodes[&3].explain_payload["kind"], json!("function"));
        assert_eq!(
            meta.nodes[&3].explain_payload["file_path"],
            json!("src/auth.rs")
        );
        assert_eq!(
            meta.nodes[&3].explain_payload["symbol_fqn"],
            json!("src/auth.rs::verify_token")
        );

        assert_eq!(meta.edges[&0].explain_payload["kind"], json!("mentions"));
        assert_eq!(meta.edges[&0].from_index, Some(1));
        assert_eq!(meta.edges[&0].to_index, Some(2));
    }
}
