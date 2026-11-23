use super::*;

pub(super) async fn run_finder_search(surface: SysMapSurface, query_str: String) {
    let q_trim = query_str.trim().to_owned();
    if q_trim.is_empty() {
        close_finder(surface);
        return;
    }
    let state = surface_state(surface);
    let project_id = m::text(&state, "project_id");
    let config_path = m::text(&state, "config_path");
    let epoch = m::next(surface.find_epoch);

    let mut items: Vec<V> = Vec::new();
    let doc = m::data(surface.map_key);
    let needle = q_trim.to_ascii_lowercase();

    for node in m::array(&doc["nodes"]) {
        let id = node_id(&node);
        let name = m::text(&node, "name");
        if id.to_ascii_lowercase().contains(&needle) || name.to_ascii_lowercase().contains(&needle)
        {
            items.push(json!({
                "kind": node["kind"].as_str().unwrap_or("service"),
                "service": id,
                "name": if name.is_empty() { id.clone() } else { name },
                "path": id,
            }));
        }
        for c in m::array(&node["provides"]) {
            let cname = contract_name(&c);
            if cname.to_ascii_lowercase().contains(&needle) {
                items.push(json!({
                    "kind": "contract",
                    "service": id,
                    "name": cname,
                    "path": id,
                }));
            }
        }
        for cap in m::array(&node["capabilities"]) {
            let cap_name = capability_name(&cap);
            if cap_name.to_ascii_lowercase().contains(&needle) {
                items.push(json!({
                    "kind": "capability",
                    "service": id,
                    "name": cap_name,
                    "path": id,
                }));
            }
        }
    }

    let mut remote_err: Option<String> = None;
    if !project_id.is_empty() {
        let q = dom::query(&[
            ("project_id", project_id),
            ("config_path", config_path),
            ("query", q_trim.clone()),
        ]);
        match api::get(&format!("/api/systems/graph/find{q}")).await {
            Ok(remote) => {
                if !m::current(surface.find_epoch, epoch) {
                    return;
                }
                let remote_items = if remote["results"].is_array() {
                    m::array(&remote["results"])
                } else {
                    m::array(&remote["items"])
                };
                for r in remote_items {
                    items.push(r);
                }
            }
            Err(err) => {
                if !m::current(surface.find_epoch, epoch) {
                    return;
                }
                if !err.aborted {
                    remote_err = Some(err.message);
                }
            }
        }
    }

    if !m::current(surface.find_epoch, epoch) {
        return;
    }

    let mut seen = BTreeSet::new();
    let mut list_html = String::new();
    let mut opt_idx = 0usize;
    for item in items.into_iter().take(24) {
        let kind = first_non_empty(&item, &["kind", "type"]);
        let svc = first_non_empty(&item, &["service", "project", "id"]);
        let name = first_non_empty(&item, &["label", "name", "title", "contract", "path"]);
        let path = first_non_empty(&item, &["path", "file", "id"]);
        let dedup_key = format!("{kind}:{svc}:{name}:{path}");
        if !seen.insert(dedup_key) {
            continue;
        }
        let p = payload_attr(&json!({
            "surface": surface.id,
            "kind": kind,
            "service": svc,
            "path": path,
            "name": name,
        }));
        let opt_id = format!("{}-find-opt-{opt_idx}", surface.id_prefix);
        opt_idx += 1;
        list_html.push_str(&format!(
            "<li role=\"none\"><button type=\"button\" role=\"option\" id=\"{opt_id}\" aria-selected=\"false\" tabindex=\"-1\" class=\"system-find-item\" data-action=\"system-map-find-pick\" data-payload=\"{p}\"><span class=\"system-chip\">{}</span><strong>{}</strong><span class=\"system-note\">{}</span></button></li>",
            escape(if kind.is_empty() { "item" } else { &kind }),
            escape(&name),
            escape(&svc),
        ));
    }

    set_html(surface.find_list_id, &list_html);
    let note_msg = match (list_html.is_empty(), remote_err) {
        (true, Some(err)) => format!("No local matches (remote search failed: {err})."),
        (true, None) => "No matching services, contracts, capabilities, or symbols.".to_owned(),
        (false, Some(err)) => {
            format!("Showing cached matches only (remote search failed: {err}).")
        }
        (false, None) => "Select a service or contract to focus here, or a member symbol/file to open its service code map.".to_owned(),
    };
    dom::text(surface.find_note_id, &note_msg);
    dom::hidden(surface.find_found_id, false);
    dom::attr(surface.find_input_id, "aria-expanded", "true");
    dom::attr(surface.find_input_id, "aria-activedescendant", "");
}

pub(super) fn close_finder(surface: SysMapSurface) {
    dom::hidden(surface.find_found_id, true);
    dom::attr(surface.find_input_id, "aria-expanded", "false");
    dom::attr(surface.find_input_id, "aria-activedescendant", "");
}

pub fn refresh_services_map(project_id: &str, config_path: &str, catalog: &V, graph_status: &V) {
    let mut state = surface_state(SERVICES_SURFACE);
    let project_changed = m::text(&state, "project_id") != project_id
        || m::text(&state, "config_path") != config_path;
    state["project_id"] = json!(project_id);
    state["config_path"] = json!(config_path);
    if project_changed {
        state["focus"] = json!("");
        state["roving_focus"] = json!("");
        state["selected_edge"] = V::Null;
        save_camera(SERVICES_SURFACE, &Camera::default());
        SYS_NAV_HISTORY.with(|h| {
            h.borrow_mut().remove(SERVICES_SURFACE.id);
        });
    }
    save_surface_state(SERVICES_SURFACE, &state);
    m::set("system-map-catalog-cache", catalog.clone());
    m::set("system-map-status-cache", graph_status.clone());

    let immediate =
        expand_unfolded_nodes(&synthesize_map_from_catalog(catalog, graph_status), &state);
    if project_changed || !m::data(SERVICES_SURFACE.map_key)["nodes"].is_array() {
        m::set(SERVICES_SURFACE.map_key, immediate);
        render_surface(SERVICES_SURFACE, project_changed);
    } else {
        let cur = normalize_map_doc(
            &m::data(SERVICES_SURFACE.map_key),
            catalog,
            graph_status,
            &state,
        );
        m::set(SERVICES_SURFACE.map_key, cur);
        render_surface(SERVICES_SURFACE, false);
    }

    let cat = catalog.clone();
    let st = graph_status.clone();
    spawn_local(async move {
        fetch_surface_map(SERVICES_SURFACE, cat, st, project_changed).await;
    });
}

pub fn refresh_changes_map(
    project_id: &str,
    config_path: &str,
    catalog: &V,
    graph_status: &V,
    overlay: &V,
) {
    let mut state = surface_state(CHANGES_SURFACE);
    let project_changed = m::text(&state, "project_id") != project_id
        || m::text(&state, "config_path") != config_path;
    let prev_run = m::text(&state["overlay"], "run_id");
    let next_run = m::text(overlay, "run_id");
    state["project_id"] = json!(project_id);
    state["config_path"] = json!(config_path);
    state["overlay"] = overlay.clone();
    if prev_run != next_run {
        state["wave"] = V::Null;
        dom::cancel_timer(&format!("{}-wave-play", CHANGES_SURFACE.id_prefix));
        SYS_WAVE_PLAYING.with(|playing| {
            playing.borrow_mut().remove(CHANGES_SURFACE.id);
        });
    }
    if project_changed {
        state["focus"] = json!("");
        state["roving_focus"] = json!("");
        state["selected_edge"] = V::Null;
        save_camera(CHANGES_SURFACE, &Camera::default());
        SYS_NAV_HISTORY.with(|h| {
            h.borrow_mut().remove(CHANGES_SURFACE.id);
        });
    }
    save_surface_state(CHANGES_SURFACE, &state);
    m::set("system-map-catalog-cache", catalog.clone());
    m::set("system-map-status-cache", graph_status.clone());

    let cur_doc = m::data(CHANGES_SURFACE.map_key);
    if project_changed || !cur_doc["nodes"].is_array() {
        let immediate =
            expand_unfolded_nodes(&synthesize_map_from_catalog(catalog, graph_status), &state);
        m::set(CHANGES_SURFACE.map_key, immediate);
    } else {
        let merged = normalize_map_doc(&cur_doc, catalog, graph_status, &state);
        m::set(CHANGES_SURFACE.map_key, merged);
    }
    render_surface(CHANGES_SURFACE, project_changed);

    let cat = catalog.clone();
    let st = graph_status.clone();
    let fetch_changes_too = prev_run != next_run || m::text(&state, "side_tab") == "changes";
    spawn_local(async move {
        fetch_surface_map(CHANGES_SURFACE, cat, st, project_changed).await;
        if fetch_changes_too {
            fetch_surface_changes(CHANGES_SURFACE).await;
        }
    });
}

pub fn focus_service_changes(service_id: &str) {
    let mut state = surface_state(SERVICES_SURFACE);
    state["focus"] = json!(service_id);
    state["roving_focus"] = json!(service_id);
    state["selected_edge"] = V::Null;
    state["side_tab"] = json!("changes");
    save_surface_state(SERVICES_SURFACE, &state);
    render_surface(SERVICES_SURFACE, false);
    spawn_local(async {
        fetch_surface_changes(SERVICES_SURFACE).await;
    });
}

/// Focuses and pans a service onto the center/viewport of the System Topology map (`SERVICES_SURFACE`).
pub fn focus_service_topology(service_id: &str) -> bool {
    let target = service_id.trim();
    if target.is_empty() {
        return false;
    }
    let mut state = surface_state(SERVICES_SURFACE);
    let doc = m::data(SERVICES_SURFACE.map_key);
    let matched_id = m::array(&doc["nodes"])
        .into_iter()
        .find(|n| {
            let id = node_id(n);
            id.eq_ignore_ascii_case(target)
                || m::text(n, "name").eq_ignore_ascii_case(target)
                || m::text(n, "service").eq_ignore_ascii_case(target)
        })
        .map(|n| node_id(&n))
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| target.to_owned());

    let cam = load_camera(SERVICES_SURFACE);
    let need_seed = SYS_NAV_HISTORY.with(|h| {
        h.borrow()
            .get(SERVICES_SURFACE.id)
            .is_none_or(|nav| nav.entries().is_empty())
    });
    if need_seed {
        push_sys_nav_history(SERVICES_SURFACE, &state, &cam);
    }
    state["focus"] = json!(matched_id);
    state["roving_focus"] = json!(matched_id);
    state["selected_edge"] = V::Null;
    state["side_tab"] = json!("details");
    save_surface_state(SERVICES_SURFACE, &state);
    render_surface(SERVICES_SURFACE, false);

    let rect_opt = SYS_DENSE_CACHE.with(|c| {
        let g = c.borrow();
        let entry = g.get(SERVICES_SURFACE.canvas_id)?;
        let &idx = entry.node_index_by_id.get(&matched_id)?;
        let n = entry.layout.nodes.get(idx)?;
        Some((n.x, n.y, n.w, n.h))
    });
    let mut next_cam = load_camera(SERVICES_SURFACE);
    if let Some((nx, ny, nw, nh)) = rect_opt {
        let (vw, vh, measured) = viewport_size(SERVICES_SURFACE.canvas_id);
        if measured && pan_to_keep_rect_in_view(&mut next_cam, (nx, ny, nw, nh), (vw, vh), 48.0) {
            save_camera(SERVICES_SURFACE, &next_cam);
            apply_camera(SERVICES_SURFACE, &next_cam);
        }
    }
    push_sys_nav_history(SERVICES_SURFACE, &state, &next_cam);
    true
}

pub fn current_context_brief() -> String {
    let state = surface_state(CHANGES_SURFACE);
    let doc = m::data(CHANGES_SURFACE.map_key);
    build_context_brief(&doc, &state)
}

pub(super) fn attach_canvas_wheel(canvas_id: &'static str) {
    let timer_key = format!("{canvas_id}-wheel-end");
    graph_viewport::attach_wheel_zoom(canvas_id, &timer_key, move |factor, anchor| {
        if let Some(surface) = surface_for_canvas(canvas_id) {
            zoom_surface_at(surface, factor, Some(anchor));
        }
    });
}

pub fn attach_wheel_listeners() {
    attach_canvas_wheel(SERVICES_SURFACE.canvas_id);
    attach_canvas_wheel(CHANGES_SURFACE.canvas_id);
}

#[derive(Default)]
pub(super) struct HoverIndex {
    node_els: BTreeMap<String, Vec<web_sys::Element>>,
    edge_els: Vec<web_sys::Element>,
    edges_by_node: BTreeMap<String, Vec<usize>>,
    edges_by_pair: BTreeMap<(String, String), Vec<usize>>,
    adjacency: BTreeMap<String, BTreeSet<String>>,
    lit: Vec<web_sys::Element>,
}

thread_local! {
    pub(super) static HOVER_CACHE: RefCell<BTreeMap<String, HoverIndex>> = const { RefCell::new(BTreeMap::new()) };
}

pub(super) fn invalidate_hover_cache(canvas_id: &str) {
    HOVER_CACHE.with(|cache| {
        cache.borrow_mut().remove(canvas_id);
    });
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn hover_pointed_key(
    hovered_node: Option<&str>,
    hovered_edge: Option<(&str, &str)>,
) -> String {
    match (hovered_node, hovered_edge) {
        (Some(nid), _) => format!("n:{nid}"),
        (None, Some((ef, et))) => format!("e:{ef}\u{1f}{et}"),
        (None, None) => String::new(),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn build_edge_adjacency(
    edges: &[(String, String)],
) -> BTreeMap<String, BTreeSet<String>> {
    let mut adj: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (from, to) in edges {
        if !from.is_empty() && !to.is_empty() {
            adj.entry(from.clone()).or_default().insert(to.clone());
            adj.entry(to.clone()).or_default().insert(from.clone());
        }
    }
    adj
}

pub(super) fn build_hover_index(canvas_id: &str) -> HoverIndex {
    let mut index = HoverIndex::default();
    let mut pairs = Vec::new();
    for el in dom::all(&format!(
        "#{canvas_id} .system-map-edge, #{canvas_id} .system-map-violation-halo, #{canvas_id} .system-map-edge-label"
    )) {
        let f = el.get_attribute("data-from").unwrap_or_default();
        let t = el.get_attribute("data-to").unwrap_or_default();
        let idx = index.edge_els.len();
        index.edge_els.push(el);
        if !f.is_empty() {
            index.edges_by_node.entry(f.clone()).or_default().push(idx);
        }
        if !t.is_empty() && t != f {
            index.edges_by_node.entry(t.clone()).or_default().push(idx);
        }
        if !f.is_empty() || !t.is_empty() {
            index
                .edges_by_pair
                .entry((f.clone(), t.clone()))
                .or_default()
                .push(idx);
            pairs.push((f, t));
        }
    }
    index.adjacency = build_edge_adjacency(&pairs);
    for el in dom::all(&format!("#{canvas_id} .system-map-node[data-n]")) {
        let nid = el.get_attribute("data-n").unwrap_or_default();
        if !nid.is_empty() {
            index.node_els.entry(nid).or_default().push(el);
        }
    }
    index
}

pub(super) fn apply_hover_lighting(
    canvas_id: &str,
    hovered_node: Option<&str>,
    hovered_edge: Option<(&str, &str)>,
) {
    let Some(canvas) = dom::get(canvas_id) else {
        return;
    };
    let Ok(Some(plane)) = canvas.query_selector(".graph-map-plane") else {
        return;
    };
    let pointed_key = hover_pointed_key(hovered_node, hovered_edge);
    if plane.get_attribute("data-pointed").unwrap_or_default() == pointed_key {
        return;
    }
    let _ = plane.set_attribute("data-pointed", &pointed_key);

    HOVER_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let cleared_existing = if let Some(entry) = cache.get_mut(canvas_id) {
            for el in entry.lit.drain(..) {
                let _ = el.remove_attribute("data-lit");
            }
            true
        } else {
            false
        };
        if !cleared_existing {
            for el in dom::all(&format!("#{canvas_id} [data-lit]")) {
                let _ = el.remove_attribute("data-lit");
            }
        }

        if hovered_node.is_none() && hovered_edge.is_none() {
            let _ = plane.remove_attribute("data-hover");
            return;
        }

        let entry = cache
            .entry(canvas_id.to_owned())
            .or_insert_with(|| build_hover_index(canvas_id));

        let _ = plane.set_attribute("data-hover", "true");
        let mut lit_nodes = BTreeSet::new();

        if let Some(nid) = hovered_node {
            if let Some(edge_indices) = entry.edges_by_node.get(nid) {
                for &idx in edge_indices {
                    if let Some(el) = entry.edge_els.get(idx) {
                        let _ = el.set_attribute("data-lit", "true");
                        entry.lit.push(el.clone());
                    }
                }
            }
            lit_nodes.insert(nid.to_owned());
            if let Some(neighbors) = entry.adjacency.get(nid) {
                for neighbor in neighbors {
                    lit_nodes.insert(neighbor.clone());
                }
            }
        } else if let Some((ef, et)) = hovered_edge {
            if let Some(edge_indices) = entry.edges_by_pair.get(&(ef.to_owned(), et.to_owned())) {
                for &idx in edge_indices {
                    if let Some(el) = entry.edge_els.get(idx) {
                        let _ = el.set_attribute("data-lit", "true");
                        entry.lit.push(el.clone());
                    }
                }
            }
            if !ef.is_empty() {
                lit_nodes.insert(ef.to_owned());
            }
            if !et.is_empty() {
                lit_nodes.insert(et.to_owned());
            }
        }

        for nid in &lit_nodes {
            if let Some(els) = entry.node_els.get(nid) {
                for el in els {
                    let _ = el.set_attribute("data-lit", "true");
                    entry.lit.push(el.clone());
                }
            }
        }
    });
}

pub(super) fn look_at_system_point(surface: SysMapSurface, point: (f64, f64)) {
    let (vw, vh, measured) = viewport_size(surface.canvas_id);
    if !measured {
        return;
    }
    let mut cam = load_camera(surface);
    let next = graph_viewport::center_camera_on(point, (vw, vh), cam.scale);
    cam.apply_viewport_cam(next);
    cam.fit_mode = false;
    cam.needs_measure = false;
    save_camera(surface, &cam);
    apply_camera(surface, &cam);
}

pub(super) fn radar_target_element(radar: &web_sys::Element) -> web_sys::Element {
    radar
        .query_selector(".graph-radar-picture")
        .ok()
        .flatten()
        .unwrap_or_else(|| radar.clone())
}

pub(super) fn save_radar_drag(surface_id: &str, radar_id: &str, drag: graph_viewport::RadarDrag) {
    let mut raw = drag.to_json();
    raw["surface"] = json!(surface_id);
    raw["radar_id"] = json!(radar_id);
    m::set(RADAR_DRAG_KEY, raw);
}

pub(super) fn radar_pointer_event(
    kind: &str,
    event: &web_sys::Event,
    target_el: Option<&web_sys::Element>,
) -> bool {
    let held = m::data(RADAR_DRAG_KEY);
    let mut drag = graph_viewport::RadarDrag::from_json(&held);
    match kind {
        "pointerdown" => {
            let Some(radar) =
                target_el.and_then(|el| el.closest(".system-map-radar").ok().flatten())
            else {
                return false;
            };
            let surf_id = radar.get_attribute("data-surface").unwrap_or_default();
            let surface = surface_by_id(&surf_id);
            close_finder(surface);
            let cam = load_camera(surface);
            let target = radar_target_element(&radar);
            match graph_viewport::handle_radar_pointer(
                kind,
                event,
                &target,
                Some(surface.canvas_id),
                (cam.plane_w, cam.plane_h),
                &mut drag,
            ) {
                graph_viewport::RadarAction::MoveTo { world_point } => {
                    save_radar_drag(surface.id, &radar.id(), drag);
                    look_at_system_point(surface, world_point);
                }
                graph_viewport::RadarAction::Ignored => {
                    if drag.active {
                        save_radar_drag(surface.id, &radar.id(), drag);
                    }
                }
                graph_viewport::RadarAction::Released => {
                    m::set(RADAR_DRAG_KEY, json!({"active": false}));
                }
            }
            true
        }
        "pointermove" | "pointerup" | "pointercancel" => {
            if !drag.active {
                return false;
            }
            let surf_id = m::text(&held, "surface");
            let surface = surface_by_id(&surf_id);
            let radar_id = m::text(&held, "radar_id");
            let Some(radar) = dom::get(&radar_id) else {
                dom::attr(surface.canvas_id, "data-panning", "false");
                m::set(RADAR_DRAG_KEY, json!({"active": false}));
                return true;
            };
            let cam = load_camera(surface);
            let target = radar_target_element(&radar);
            match graph_viewport::handle_radar_pointer(
                kind,
                event,
                &target,
                Some(surface.canvas_id),
                (cam.plane_w, cam.plane_h),
                &mut drag,
            ) {
                graph_viewport::RadarAction::MoveTo { world_point } => {
                    save_radar_drag(surface.id, &radar_id, drag);
                    look_at_system_point(surface, world_point);
                }
                graph_viewport::RadarAction::Released => {
                    m::set(RADAR_DRAG_KEY, json!({"active": false}));
                }
                graph_viewport::RadarAction::Ignored => {
                    save_radar_drag(surface.id, &radar_id, drag);
                }
            }
            true
        }
        _ => false,
    }
}

pub(super) fn ensure_resize_watch() {
    graph_viewport::ensure_resize_observer(
        "system-map-resize-watched",
        Some("system-map-resize"),
        60,
        || {
            for surface in [SERVICES_SURFACE, CHANGES_SURFACE] {
                let visible = dom::get(surface.canvas_id)
                    .is_some_and(|el| el.closest("[hidden]").ok().flatten().is_none());
                let has_plane = dom::get(surface.canvas_id)
                    .and_then(|el| el.query_selector(".graph-map-plane").ok().flatten())
                    .is_some();
                if !(visible && has_plane) {
                    continue;
                }
                let mut cam = load_camera(surface);
                let (vw, vh, measured) = viewport_size(surface.canvas_id);
                match graph_viewport::resize_refit_decision(
                    cam.viewport_cam(),
                    cam.fit_mode || cam.needs_measure,
                    (cam.plane_w, cam.plane_h),
                    (vw, vh),
                    measured,
                    graph_viewport::FitOptions::for_system_map(),
                ) {
                    graph_viewport::ResizeAction::Skip => {}
                    graph_viewport::ResizeAction::Refit(next) => {
                        cam.apply_viewport_cam(next);
                        cam.fit_mode = true;
                        cam.needs_measure = false;
                        save_camera(surface, &cam);
                        apply_camera(surface, &cam);
                    }
                    graph_viewport::ResizeAction::SyncRadar(_) => {
                        apply_camera(surface, &cam);
                    }
                }
            }
        },
    );
}

pub(super) fn ensure_legend_hover_watch(surface: SysMapSurface) {
    let Some(legend) = dom::get(surface.legend_id) else {
        return;
    };
    if legend.get_attribute("data-hover-bound").as_deref() == Some("true") {
        return;
    }
    let _ = legend.set_attribute("data-hover-bound", "true");
    let cid = surface.canvas_id;
    for (event_kind, rests) in [
        ("mouseover", true),
        ("focusin", true),
        ("mouseleave", false),
        ("focusout", false),
    ] {
        let moved = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            let held = rests
                .then(|| {
                    event
                        .target()
                        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                })
                .flatten()
                .and_then(|target| {
                    target
                        .closest(
                            ".graph-arrow-legend-item[aria-pressed='true'][data-sys-edge-kind]",
                        )
                        .ok()
                        .flatten()
                })
                .and_then(|item| item.get_attribute("data-sys-edge-kind"));
            let Some(canvas) = dom::get(cid) else {
                return;
            };
            let _ = match held.as_deref() {
                Some(kind) => {
                    let _ = canvas.set_attribute("data-sys-kind-hover", kind);
                    canvas.set_attribute("data-kind-hover", kind)
                }
                None => {
                    let _ = canvas.remove_attribute("data-sys-kind-hover");
                    canvas.remove_attribute("data-kind-hover")
                }
            };
            let is_dense =
                SYS_DENSE_CACHE.with(|c| c.borrow().get(cid).is_some_and(|e| e.is_dense));
            if is_dense {
                if let Some(surf) = surface_for_canvas(cid) {
                    let state = surface_state(surf);
                    let view_doc = filtered_doc(&m::data(surf.map_key), &state);
                    let vis = compute_sys_visual_state(&view_doc, &state);
                    let (node_ids, edge_endpoints) = SYS_DENSE_CACHE.with(|c| {
                        c.borrow()
                            .get(cid)
                            .map(|e| (e.node_ids.clone(), e.edge_endpoints.clone()))
                            .unwrap_or_default()
                    });
                    graph_viewport::canvas2d::update_scene(cid, |scene| {
                        refresh_dense_scene_for_system(scene, &vis, &node_ids, &edge_endpoints);
                        if let Some(kind) = held.as_deref() {
                            for edge in &mut scene.edges {
                                let match_kind = edge.kind == kind;
                                edge.lit = match_kind;
                                edge.dim = !match_kind;
                            }
                        }
                    });
                    let cam = load_camera(surf);
                    let hover = SYS_HOVER_HIT.with(|h| h.borrow().get(cid).copied().flatten());
                    graph_viewport::canvas2d::paint_scene(cid, cam.viewport_cam(), hover, true);
                }
            }
        });
        let _ = legend.add_event_listener_with_callback(event_kind, moved.as_ref().unchecked_ref());
        moved.forget();
    }
}

pub(super) fn ensure_hover_watch(surface: SysMapSurface) {
    let Some(canvas) = dom::get(surface.canvas_id) else {
        return;
    };
    if canvas.get_attribute("data-hover-bound").as_deref() == Some("true") {
        return;
    }
    let _ = canvas.set_attribute("data-hover-bound", "true");
    let cid = surface.canvas_id;

    let over = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        let Some(target) = event
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        else {
            return;
        };
        if target.closest(".system-map-radar").ok().flatten().is_some() {
            apply_hover_lighting(cid, None, None);
            return;
        }
        if let Some(node_el) = target.closest(".system-map-node[data-n]").ok().flatten() {
            let nid = node_el.get_attribute("data-n").unwrap_or_default();
            apply_hover_lighting(cid, Some(&nid), None);
        } else if let Some(hit_el) = target.closest(".system-map-hit").ok().flatten() {
            let f = hit_el.get_attribute("data-from").unwrap_or_default();
            let t = hit_el.get_attribute("data-to").unwrap_or_default();
            apply_hover_lighting(cid, None, Some((&f, &t)));
        } else {
            apply_hover_lighting(cid, None, None);
        }
    });
    let _ = canvas.add_event_listener_with_callback("pointerover", over.as_ref().unchecked_ref());
    let _ = canvas.add_event_listener_with_callback("focusin", over.as_ref().unchecked_ref());
    over.forget();

    let leave = Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
        apply_hover_lighting(cid, None, None);
        SYS_HOVER_HIT.with(|h| {
            h.borrow_mut().remove(cid);
        });
        if let Some(canvas_el) = dom::get(cid) {
            if let Ok(he) = canvas_el.dyn_into::<web_sys::HtmlElement>() {
                let _ = he.style().set_property("cursor", "");
            }
        }
        let is_dense = SYS_DENSE_CACHE.with(|c| c.borrow().get(cid).is_some_and(|e| e.is_dense));
        if is_dense {
            if let Some(surf) = surface_for_canvas(cid) {
                let cam = load_camera(surf);
                graph_viewport::canvas2d::paint_scene(cid, cam.viewport_cam(), None, true);
            }
        }
    });
    let _ = canvas.add_event_listener_with_callback("pointerleave", leave.as_ref().unchecked_ref());
    leave.forget();
}

pub(super) fn update_sys_live_drag(
    surface: SysMapSurface,
    nid: &str,
    orig_x: f64,
    orig_y: f64,
    nx: f64,
    ny: f64,
    cam: &Camera,
) {
    let Some(canvas) = dom::get(surface.canvas_id) else {
        return;
    };
    let dx = nx - orig_x;
    let dy = ny - orig_y;
    if let Some(el) = find_sys_node_el(&canvas, nid) {
        let _ = el.set_attribute("data-pinned", "true");
        if let Ok(he) = el.dyn_into::<web_sys::HtmlElement>() {
            let _ = he.style().set_property("left", &format!("{nx:.1}px"));
            let _ = he.style().set_property("top", &format!("{ny:.1}px"));
        }
    }

    let snapshot = SYS_DENSE_CACHE.with(|c| {
        let g = c.borrow();
        let entry = g.get(surface.canvas_id)?;
        let mut rects: HashMap<String, (f64, f64, f64, f64)> = HashMap::new();
        let mut moved_ids: HashSet<String> = HashSet::new();
        moved_ids.insert(nid.to_owned());
        let mut frame_rect: Option<(f64, f64)> = None;
        for f in &entry.layout.frames {
            if f.service_id == nid {
                frame_rect = Some(((f.x + dx).max(4.0), (f.y + dy).max(4.0)));
                break;
            }
        }
        let mut moved_children: Vec<(String, f64, f64)> = Vec::new();
        for p in &entry.layout.nodes {
            let (px, py) = if p.id == nid {
                (nx, ny)
            } else if p.child_of.as_deref() == Some(nid) && !entry.pinned_ids.contains(&p.id) {
                moved_ids.insert(p.id.clone());
                let cx = (p.x + dx).max(8.0);
                let cy = (p.y + dy).max(8.0);
                moved_children.push((p.id.clone(), cx, cy));
                (cx, cy)
            } else {
                (p.x, p.y)
            };
            rects.insert(p.id.clone(), (px, py, p.w, p.h));
        }
        Some((
            entry.is_dense,
            frame_rect,
            moved_children,
            moved_ids,
            rects,
            entry.edge_endpoints.clone(),
            entry.node_index_by_id.clone(),
        ))
    });

    let Some((is_dense, frame_rect, moved_children, moved_ids, rects, edge_endpoints, idx_by_id)) =
        snapshot
    else {
        return;
    };

    if let (Some((fx, fy)), Some(frame_g)) = (frame_rect, find_sys_frame_el(&canvas, nid)) {
        if let Ok(Some(rect)) = frame_g.query_selector("rect.system-frame-rect") {
            let _ = rect.set_attribute("x", &format!("{fx:.1}"));
            let _ = rect.set_attribute("y", &format!("{fy:.1}"));
        }
        if let Ok(Some(title)) = frame_g.query_selector("text.system-frame-title") {
            let _ = title.set_attribute("x", &format!("{:.1}", fx + 12.0));
            let _ = title.set_attribute("y", &format!("{:.1}", fy + 18.0));
        }
    }
    for (cid, cx, cy) in &moved_children {
        if let Some(el) = find_sys_node_el(&canvas, cid) {
            if let Ok(he) = el.dyn_into::<web_sys::HtmlElement>() {
                let _ = he.style().set_property("left", &format!("{cx:.1}px"));
                let _ = he.style().set_property("top", &format!("{cy:.1}px"));
            }
        }
    }

    if is_dense {
        graph_viewport::canvas2d::update_scene(surface.canvas_id, |scene| {
            for id in &moved_ids {
                if let (Some(&ni), Some(&(px, py, _, _))) = (idx_by_id.get(id), rects.get(id)) {
                    if let Some(node) = scene.nodes.get_mut(ni) {
                        node.x = px;
                        node.y = py;
                        node.pinned = true;
                    }
                }
            }
            for edge in &mut scene.edges {
                let Some((from, to)) = edge_endpoints.get(edge.index) else {
                    continue;
                };
                if !moved_ids.contains(from) && !moved_ids.contains(to) {
                    continue;
                }
                if let (Some(&f_rect), Some(&t_rect)) = (rects.get(from), rects.get(to)) {
                    let (d, _, _) = live_drag_wire_path(f_rect, t_rect);
                    let updated = graph_viewport::canvas2d::DenseEdgeSpec::from_svg_path(
                        edge.index,
                        &d,
                        edge.weight,
                        edge.guessed,
                        edge.against,
                        edge.kind.clone(),
                        edge.on,
                        edge.lit,
                        edge.dim,
                        edge.portal,
                    );
                    edge.cmds = updated.cmds;
                    edge.pts = updated.pts;
                }
            }
        });
        graph_viewport::canvas2d::paint_scene(surface.canvas_id, cam.viewport_cam(), None, true);
        return;
    }

    let mut path_by_pair: HashMap<(String, String), (String, f64, f64)> = HashMap::new();
    for (from, to) in &edge_endpoints {
        if (!moved_ids.contains(from) && !moved_ids.contains(to))
            || path_by_pair.contains_key(&(from.clone(), to.clone()))
        {
            continue;
        }
        if let (Some(&f_rect), Some(&t_rect)) = (rects.get(from), rects.get(to)) {
            path_by_pair.insert(
                (from.clone(), to.clone()),
                live_drag_wire_path(f_rect, t_rect),
            );
        }
    }
    if path_by_pair.is_empty() {
        return;
    }
    for sel in [
        ".system-map-edge",
        ".system-map-violation-halo",
        ".system-map-broken-halo",
        ".system-map-hit",
    ] {
        if let Ok(els) = canvas.query_selector_all(&format!("{sel}[data-from][data-to]")) {
            for i in 0..els.length() {
                let Some(el) = els
                    .item(i)
                    .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
                else {
                    continue;
                };
                let from = el.get_attribute("data-from").unwrap_or_default();
                let to = el.get_attribute("data-to").unwrap_or_default();
                if let Some((d, _, _)) = path_by_pair.get(&(from, to)) {
                    let _ = el.set_attribute("d", d);
                }
            }
        }
    }
    if let Ok(labels) = canvas.query_selector_all(".system-map-edge-label[data-from][data-to]") {
        for i in 0..labels.length() {
            let Some(el) = labels
                .item(i)
                .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
            else {
                continue;
            };
            let from = el.get_attribute("data-from").unwrap_or_default();
            let to = el.get_attribute("data-to").unwrap_or_default();
            if let Some((_, mx, my)) = path_by_pair.get(&(from, to)) {
                if let Ok(Some(rect)) = el.query_selector("rect") {
                    let w = rect
                        .get_attribute("width")
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(54.0);
                    let _ = rect.set_attribute("x", &format!("{:.1}", mx - w * 0.5));
                    let _ = rect.set_attribute("y", &format!("{:.1}", my - 8.0));
                }
                if let Ok(Some(text)) = el.query_selector("text") {
                    let _ = text.set_attribute("x", &format!("{mx:.1}"));
                    let _ = text.set_attribute("y", &format!("{:.1}", my + 3.5));
                }
            }
        }
    }
}

pub fn should_ignore_canvas_pointer(
    kind: &str,
    in_canvas: bool,
    in_radar: bool,
    in_interactive: bool,
    in_empty: bool,
    in_map_node: bool,
) -> bool {
    if !in_canvas || in_radar {
        return false;
    }
    if in_empty {
        return matches!(kind, "pointerdown" | "dblclick");
    }
    match kind {
        "pointerdown" => in_interactive,
        "dblclick" => in_interactive && !in_map_node,
        _ => false,
    }
}

pub fn pointer_event(kind: &str, event: &web_sys::Event) -> bool {
    let target_el = event
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
    let in_empty = target_el
        .as_ref()
        .is_some_and(|el| el.closest(".system-empty").ok().flatten().is_some());
    let in_map_node = target_el.as_ref().is_some_and(|el| {
        el.closest(".system-map-node[data-n]")
            .ok()
            .flatten()
            .is_some()
    });

    if radar_pointer_event(kind, event, target_el.as_ref()) {
        return true;
    }

    for surface in [SERVICES_SURFACE, CHANGES_SURFACE] {
        if kind == "pointerdown"
            && dom::get(surface.find_found_id).is_some_and(|el| !el.has_attribute("hidden"))
            && target_el
                .as_ref()
                .and_then(|el| {
                    el.closest(&format!("#{}", surface.find_wrap_id))
                        .ok()
                        .flatten()
                })
                .is_none()
        {
            close_finder(surface);
        }
        let in_canvas = target_el
            .as_ref()
            .and_then(|el| {
                el.closest(&format!("#{}", surface.canvas_id))
                    .ok()
                    .flatten()
            })
            .is_some();
        let in_radar = target_el
            .as_ref()
            .and_then(|el| el.closest(".system-map-radar").ok().flatten())
            .is_some();
        let plane = dom::get(surface.canvas_id)
            .and_then(|c| c.query_selector(".system-map-plane").ok().flatten());
        let plane_empty = plane.as_ref().is_some_and(|p| {
            graph_viewport::dense_dom::is_empty_plane_attr(p.get_attribute("data-empty").as_deref())
        });
        let in_interactive = graph_viewport::dense_dom::is_interactive_or_empty_target(
            target_el.as_ref(),
            plane.as_ref(),
            Some(".system-empty"),
        );

        // Shift+drag node pinning on system service cards.
        if matches!(
            kind,
            "pointerdown" | "pointermove" | "pointerup" | "pointercancel"
        ) {
            let drag_store_key = format!("{}-node-drag", surface.id_prefix);
            let mut pin_drag =
                graph_viewport::dense_dom::PinDragSession::from_json(&m::data(&drag_store_key));
            if kind == "pointerdown" && in_canvas && !in_radar && !(in_empty || plane_empty) {
                if let Ok(pe) = event.clone().dyn_into::<web_sys::PointerEvent>() {
                    if pe.shift_key() {
                        let resolved = if let Some(node_el) = target_el
                            .as_ref()
                            .and_then(|el| el.closest(".system-map-node[data-n]").ok().flatten())
                        {
                            let nid = node_el.get_attribute("data-n").unwrap_or_default();
                            let info = SYS_DENSE_CACHE.with(|c| {
                                let g = c.borrow();
                                let entry = g.get(surface.canvas_id)?;
                                let &idx = entry.node_index_by_id.get(&nid)?;
                                let placed = entry.layout.nodes.get(idx)?;
                                Some((idx, (placed.x, placed.y)))
                            });
                            Some((
                                nid,
                                Some(node_el),
                                info.map(|(i, _)| i).unwrap_or(0),
                                info.map(|(_, p)| p),
                            ))
                        } else {
                            let is_dense = SYS_DENSE_CACHE.with(|c| {
                                c.borrow()
                                    .get(surface.canvas_id)
                                    .is_some_and(|e| e.is_dense)
                            });
                            if is_dense {
                                let cam = load_camera(surface);
                                graph_viewport::dense_dom::hit_test_pointer_in_canvas(
                                    surface.canvas_id,
                                    pe.client_x() as f64,
                                    pe.client_y() as f64,
                                    cam.viewport_cam(),
                                    4.0,
                                )
                                .and_then(|hit| match hit {
                                    (0, idx) => {
                                        let (nid, pos) = SYS_DENSE_CACHE.with(|c| {
                                            let g = c.borrow();
                                            let entry = g.get(surface.canvas_id)?;
                                            let placed = entry.layout.nodes.get(idx)?;
                                            Some((placed.id.clone(), (placed.x, placed.y)))
                                        })?;
                                        let el = mount_system_dense_node_if_needed(surface, &nid);
                                        Some((nid, el, idx, Some(pos)))
                                    }
                                    _ => None,
                                })
                            } else {
                                None
                            }
                        };
                        if let Some((nid, maybe_el, node_idx, cached_pos)) = resolved {
                            let orig = cached_pos.or_else(|| {
                                let state = surface_state(surface);
                                let doc = filtered_doc(&m::data(surface.map_key), &state);
                                let pins = graph_viewport::load_pins(&sys_pin_key(surface, &state));
                                let layout = layout_system_nodes_with_pins(
                                    &doc,
                                    m::yes(&state["layers"]),
                                    &pins,
                                );
                                layout
                                    .nodes
                                    .iter()
                                    .find(|n| n.id == nid)
                                    .map(|p| (p.x, p.y))
                            });
                            if let Some((orig_x, orig_y)) = orig {
                                if let Some(cap) = maybe_el.or_else(|| dom::get(surface.canvas_id))
                                {
                                    graph_viewport::capture_pointer(&cap, pe.pointer_id());
                                }
                                let session = graph_viewport::dense_dom::PinDragSession::begin(
                                    pe.pointer_id(),
                                    node_idx,
                                    nid,
                                    (pe.client_x() as f64, pe.client_y() as f64),
                                    (orig_x, orig_y),
                                );
                                m::set(&drag_store_key, session.to_json());
                                return true;
                            }
                        }
                    }
                }
            } else if pin_drag.active {
                if let Ok(pe) = event.clone().dyn_into::<web_sys::PointerEvent>() {
                    let cam = load_camera(surface);
                    let has_frame = SYS_DENSE_CACHE.with(|c| {
                        c.borrow().get(surface.canvas_id).is_some_and(|e| {
                            e.layout
                                .frames
                                .iter()
                                .any(|f| f.service_id == pin_drag.node_key)
                        })
                    });
                    let bounds = (
                        if has_frame { FRAME_PAD + 4.0 } else { 8.0 },
                        if has_frame { FRAME_HEAD + 4.0 } else { 8.0 },
                        1_000_000.0,
                        1_000_000.0,
                    );
                    if kind == "pointermove" {
                        if let Some((nx, ny)) = pin_drag.advance(
                            pe.pointer_id(),
                            (pe.client_x() as f64, pe.client_y() as f64),
                            cam.scale.max(0.1),
                            bounds,
                            2.0,
                        ) {
                            let nid = pin_drag.node_key.clone();
                            let orig = pin_drag.start_pos;
                            m::set(&drag_store_key, pin_drag.to_json());
                            update_sys_live_drag(surface, &nid, orig.0, orig.1, nx, ny, &cam);
                        }
                        return true;
                    }
                    if let Some(outcome) =
                        pin_drag.end(Some(pe.pointer_id()), kind == "pointercancel")
                    {
                        m::set(&drag_store_key, V::Null);
                        match outcome {
                            graph_viewport::dense_dom::PinDragOutcome::Committed {
                                node_key,
                                pos,
                                ..
                            } => {
                                let mut drag_meta =
                                    graph_viewport::DragPan::from_json(&m::data(surface.drag_key));
                                drag_meta.active = false;
                                drag_meta.suppress_click = true;
                                m::set(surface.drag_key, drag_meta.to_json());
                                let dk = surface.drag_key;
                                dom::later(&format!("{dk}-clear"), 70, move || {
                                    let mut d = graph_viewport::DragPan::from_json(&m::data(dk));
                                    d.clear_suppress();
                                    m::set(dk, d.to_json());
                                });
                                let state = surface_state(surface);
                                graph_viewport::save_pin(
                                    &sys_pin_key(surface, &state),
                                    &node_key,
                                    pos,
                                );
                                graph_viewport::announce_live(
                                    &format!("{}-live-announce", surface.id_prefix),
                                    &format!("Pinned {node_key}"),
                                );
                                render_surface(surface, false);
                            }
                            graph_viewport::dense_dom::PinDragOutcome::Cancelled { .. } => {
                                render_surface(surface, false);
                            }
                            graph_viewport::dense_dom::PinDragOutcome::Click { .. } => {}
                        }
                        return true;
                    }
                }
            }
        }

        let mut drag = graph_viewport::DragPan::from_json(&m::data(surface.drag_key));
        let drag_active = drag.active;
        if kind == "pointerdown" && in_canvas && drag.suppress_click {
            drag.clear_suppress();
            m::set(surface.drag_key, drag.to_json());
        }

        if should_ignore_canvas_pointer(
            kind,
            in_canvas,
            in_radar,
            in_interactive,
            in_empty || plane_empty,
            in_map_node,
        ) {
            return false;
        }

        if kind == "dblclick" && in_canvas && !in_radar {
            if let Some(node_el) = target_el
                .as_ref()
                .and_then(|el| el.closest(".system-map-node[data-n]").ok().flatten())
            {
                let svc = node_el
                    .get_attribute("data-explain-services")
                    .unwrap_or_default();
                let sid = svc.split(',').next().unwrap_or("").trim().to_owned();
                if !sid.is_empty() {
                    systems::drill_down_service_map(surface.id, &sid, None);
                    return true;
                }
            }
            let is_dense = SYS_DENSE_CACHE.with(|c| {
                c.borrow()
                    .get(surface.canvas_id)
                    .is_some_and(|e| e.is_dense)
            });
            if is_dense {
                if let Ok(me) = event.clone().dyn_into::<web_sys::MouseEvent>() {
                    let cam = load_camera(surface);
                    if let Some((0, idx)) = graph_viewport::dense_dom::hit_test_pointer_in_canvas(
                        surface.canvas_id,
                        me.client_x() as f64,
                        me.client_y() as f64,
                        cam.viewport_cam(),
                        8.0,
                    ) {
                        let sid = SYS_DENSE_CACHE.with(|c| {
                            let g = c.borrow();
                            let entry = g.get(surface.canvas_id)?;
                            let placed = entry.layout.nodes.get(idx)?;
                            Some(placed.child_of.clone().unwrap_or_else(|| placed.id.clone()))
                        });
                        if let Some(sid) = sid.filter(|s| !s.is_empty()) {
                            systems::drill_down_service_map(surface.id, &sid, None);
                            return true;
                        }
                    }
                }
            }
            if let Ok(me) = event.clone().dyn_into::<web_sys::MouseEvent>() {
                zoom_surface_at(
                    surface,
                    ZOOM_STEP,
                    Some((me.client_x() as f64, me.client_y() as f64)),
                );
            }
            return true;
        }

        if kind == "pointermove"
            && in_canvas
            && !in_radar
            && !drag_active
            && !m::yes(&m::data(&format!("{}-node-drag", surface.id_prefix))["active"])
        {
            let is_dense = SYS_DENSE_CACHE.with(|c| {
                c.borrow()
                    .get(surface.canvas_id)
                    .is_some_and(|e| e.is_dense)
            });
            if is_dense {
                if let (Ok(pe), Some(canvas)) = (
                    event.clone().dyn_into::<web_sys::PointerEvent>(),
                    dom::get(surface.canvas_id),
                ) {
                    let cam = load_camera(surface);
                    let hit = graph_viewport::dense_dom::hit_test_pointer_in_canvas(
                        surface.canvas_id,
                        pe.client_x() as f64,
                        pe.client_y() as f64,
                        cam.viewport_cam(),
                        8.0,
                    );
                    let changed = SYS_HOVER_HIT.with(|h| {
                        let mut map = h.borrow_mut();
                        let prev = map.get(surface.canvas_id).copied().flatten();
                        if prev == hit {
                            false
                        } else {
                            map.insert(surface.canvas_id, hit);
                            true
                        }
                    });
                    if changed {
                        graph_viewport::canvas2d::paint_scene(
                            surface.canvas_id,
                            cam.viewport_cam(),
                            hit,
                            true,
                        );
                        if let Some(plane_el) = plane.as_ref() {
                            let hovered_idx =
                                hit.and_then(|(layer, idx)| (layer == 0).then_some(idx));
                            let hovered_id = hovered_idx.and_then(|idx| {
                                SYS_DENSE_CACHE.with(|c| {
                                    c.borrow()
                                        .get(surface.canvas_id)
                                        .and_then(|e| e.node_ids.get(idx).cloned())
                                })
                            });
                            let hovered_el = hovered_id
                                .as_deref()
                                .and_then(|nid| find_sys_node_el(&canvas, nid));
                            let canvas_id = surface.canvas_id;
                            graph_viewport::dense_dom::sync_dom_hover_attr(
                                plane_el,
                                hovered_idx,
                                hovered_el.as_ref(),
                                |el| {
                                    el.get_attribute("data-n").and_then(|nid| {
                                        SYS_DENSE_CACHE.with(|c| {
                                            c.borrow()
                                                .get(canvas_id)
                                                .and_then(|e| e.node_index_by_id.get(&nid).copied())
                                        })
                                    })
                                },
                            );
                        }
                        if let Ok(he) = canvas.dyn_into::<web_sys::HtmlElement>() {
                            let _ = he
                                .style()
                                .set_property("cursor", if hit.is_some() { "pointer" } else { "" });
                        }
                    }
                }
            }
        }

        if (kind == "pointerdown" && in_canvas && !in_radar)
            || (drag_active && matches!(kind, "pointermove" | "pointerup" | "pointercancel"))
        {
            let mut cam = load_camera(surface);
            match graph_viewport::handle_canvas_drag(
                kind,
                event,
                surface.canvas_id,
                cam.viewport_cam(),
                graph_viewport::DEFAULT_DRAG_THRESHOLD_PX,
                &mut drag,
            ) {
                graph_viewport::CanvasDragOutcome::Down => {
                    m::set(surface.drag_key, drag.to_json());
                    return true;
                }
                graph_viewport::CanvasDragOutcome::Panned { tx, ty } => {
                    m::set(surface.drag_key, drag.to_json());
                    cam.pan_x = tx;
                    cam.pan_y = ty;
                    cam.fit_mode = false;
                    save_camera(surface, &cam);
                    apply_camera(surface, &cam);
                    return true;
                }
                graph_viewport::CanvasDragOutcome::Ended { suppress_click } => {
                    m::set(surface.drag_key, drag.to_json());
                    if suppress_click {
                        let dk = surface.drag_key;
                        let tk = format!("{dk}-clear");
                        dom::later(&tk, 70, move || {
                            let mut d = graph_viewport::DragPan::from_json(&m::data(dk));
                            d.clear_suppress();
                            m::set(dk, d.to_json());
                        });
                    } else if kind == "pointerup"
                        && target_el
                            .as_ref()
                            .and_then(|el| {
                                el.closest(".system-map-node, .system-map-hit, button, a")
                                    .ok()
                                    .flatten()
                            })
                            .is_none()
                    {
                        let is_dense = SYS_DENSE_CACHE.with(|c| {
                            c.borrow()
                                .get(surface.canvas_id)
                                .is_some_and(|e| e.is_dense)
                        });
                        if is_dense {
                            if let Ok(pe) = event.clone().dyn_into::<web_sys::PointerEvent>() {
                                if let Some(hit) =
                                    graph_viewport::dense_dom::hit_test_pointer_in_canvas(
                                        surface.canvas_id,
                                        pe.client_x() as f64,
                                        pe.client_y() as f64,
                                        cam.viewport_cam(),
                                        8.0,
                                    )
                                {
                                    let picked = SYS_DENSE_CACHE.with(|c| {
                                        let g = c.borrow();
                                        let entry = g.get(surface.canvas_id)?;
                                        match hit {
                                            (0, node_idx) => entry
                                                .layout
                                                .nodes
                                                .get(node_idx)
                                                .map(|n| (true, n.id.clone(), String::new())),
                                            (1, wire_idx) => entry
                                                .edge_endpoints
                                                .get(wire_idx)
                                                .filter(|(f, t)| !f.is_empty() && !t.is_empty())
                                                .map(|(f, t)| (false, f.clone(), t.clone())),
                                            _ => None,
                                        }
                                    });
                                    if let Some((is_node, a, b)) = picked {
                                        let mut state = surface_state(surface);
                                        if is_node {
                                            state["focus"] = json!(a);
                                            state["roving_focus"] = json!(a);
                                            state["selected_edge"] = V::Null;
                                        } else {
                                            state["selected_edge"] = json!({"from": a, "to": b});
                                        }
                                        state["side_tab"] = json!("details");
                                        save_surface_state(surface, &state);
                                        push_sys_nav_history(surface, &state, &cam);
                                        render_surface(surface, false);
                                        return true;
                                    }
                                }
                            }
                        }
                    }
                    return true;
                }
                graph_viewport::CanvasDragOutcome::Ignored => {
                    if drag.active {
                        m::set(surface.drag_key, drag.to_json());
                        return true;
                    }
                }
            }
        }
    }
    false
}

pub fn key(event: &web_sys::KeyboardEvent) -> bool {
    if event.is_composing() {
        return false;
    }
    if dom::get("graph-map").is_some_and(|d| d.has_attribute("open"))
        || dom::get("graph-link").is_some_and(|d| d.has_attribute("open"))
    {
        return false;
    }
    let view = m::read(|s| s.view.clone());
    if view != "systems" && view != "system-changes" {
        return false;
    }
    let surface = if view == "system-changes" {
        CHANGES_SURFACE
    } else {
        SERVICES_SURFACE
    };
    let canvas_visible = dom::get(surface.canvas_id)
        .is_some_and(|el| el.closest("[hidden]").ok().flatten().is_none());
    if !canvas_visible {
        return false;
    }
    if event.key() == "Escape" {
        if dom::get(surface.find_found_id).is_some_and(|el| !el.has_attribute("hidden")) {
            close_finder(surface);
            return true;
        }
        let mut state = surface_state(surface);
        if m::text(&state, "tool") == "path" || m::yes(&state["isolate_active"]) {
            state["tool"] = json!("");
            state["path_from"] = json!("");
            state["path_to"] = json!("");
            state["isolate_active"] = json!(false);
            save_surface_state(surface, &state);
            render_surface(surface, false);
            return true;
        }
        if m::yes(&state["expanded"]) {
            state["expanded"] = json!(false);
            save_surface_state(surface, &state);
            sync_toolbar_buttons(surface, &state);
            fit_surface_camera(surface);
            return true;
        }
        return false;
    }
    let target = event
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok());

    if let Some(tab_el) = target
        .as_ref()
        .and_then(|el| el.closest(".system-side-tabs [role='tab']").ok().flatten())
    {
        let key_name = event.key();
        if matches!(
            key_name.as_str(),
            "ArrowLeft" | "ArrowRight" | "Home" | "End"
        ) {
            event.prevent_default();
            let tabs = ["details", "analyze", "changes"];
            let mut state = surface_state(surface);
            let cur = match m::text(&state, "side_tab").as_str() {
                "analyze" => 1,
                "changes" => 2,
                _ => 0,
            };
            let next_idx = match key_name.as_str() {
                "ArrowRight" => (cur + 1) % tabs.len(),
                "ArrowLeft" => (cur + tabs.len() - 1) % tabs.len(),
                "Home" => 0,
                "End" => tabs.len() - 1,
                _ => cur,
            };
            let next_tab = tabs[next_idx];
            state["side_tab"] = json!(next_tab);
            save_surface_state(surface, &state);
            render_surface(surface, false);
            if let Some(next_el) = dom::all(&format!(
                "#{} .system-side-tabs [role='tab'][aria-selected='true']",
                surface.side_id
            ))
            .into_iter()
            .next()
            .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
            {
                let _ = next_el.focus();
            }
            if next_tab == "changes" {
                spawn_local(async move {
                    fetch_surface_changes(surface).await;
                });
            }
            let _ = tab_el;
            return true;
        }
    }

    let in_finder = target.as_ref().is_some_and(|el| {
        el.closest(&format!("#{}", surface.find_wrap_id))
            .ok()
            .flatten()
            .is_some()
    });
    if in_finder && dom::get(surface.find_found_id).is_some_and(|el| !el.has_attribute("hidden")) {
        let opts = dom::all(&format!("#{} [role='option']", surface.find_list_id));
        if !opts.is_empty() {
            match event.key().as_str() {
                "ArrowDown" | "ArrowUp" | "Home" | "End" => {
                    event.prevent_default();
                    let active_id = dom::get(surface.find_input_id)
                        .and_then(|el| el.get_attribute("aria-activedescendant"))
                        .unwrap_or_default();
                    let cur_pos = opts.iter().position(|o| o.id() == active_id).or_else(|| {
                        opts.iter().position(|o| {
                            o.get_attribute("aria-selected").as_deref() == Some("true")
                        })
                    });
                    let next_pos = match (event.key().as_str(), cur_pos) {
                        ("Home", _) => 0,
                        ("End", _) => opts.len() - 1,
                        ("ArrowDown", Some(i)) => (i + 1) % opts.len(),
                        ("ArrowDown", None) => 0,
                        ("ArrowUp", Some(i)) => (i + opts.len() - 1) % opts.len(),
                        ("ArrowUp", None) => opts.len() - 1,
                        _ => 0,
                    };
                    for (idx, opt) in opts.iter().enumerate() {
                        let _ = opt.set_attribute(
                            "aria-selected",
                            if idx == next_pos { "true" } else { "false" },
                        );
                    }
                    let chosen = &opts[next_pos];
                    dom::attr(surface.find_input_id, "aria-activedescendant", &chosen.id());
                    chosen.scroll_into_view();
                    return true;
                }
                "Enter" => {
                    let active_id = dom::get(surface.find_input_id)
                        .and_then(|el| el.get_attribute("aria-activedescendant"))
                        .unwrap_or_default();
                    let target_opt = opts
                        .iter()
                        .find(|o| o.id() == active_id)
                        .or_else(|| {
                            opts.iter().find(|o| {
                                o.get_attribute("aria-selected").as_deref() == Some("true")
                            })
                        })
                        .or_else(|| opts.first());
                    if let Some(opt) =
                        target_opt.and_then(|e| e.clone().dyn_into::<web_sys::HtmlElement>().ok())
                    {
                        event.prevent_default();
                        opt.click();
                        return true;
                    }
                }
                _ => {}
            }
        }
    }

    let in_input = target
        .as_ref()
        .is_some_and(|el| matches!(el.tag_name().as_str(), "INPUT" | "TEXTAREA" | "SELECT"));
    if !in_input && !event.ctrl_key() && !event.meta_key() && !event.alt_key() {
        let in_canvas = target.as_ref().is_some_and(|el| {
            el.closest(&format!("#{}", surface.canvas_id))
                .ok()
                .flatten()
                .is_some()
        });
        if in_canvas
            && matches!(
                event.key().as_str(),
                "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" | "Home" | "End"
            )
        {
            event.prevent_default();
            let mut state = surface_state(surface);
            let doc = filtered_doc(&m::data(surface.map_key), &state);
            let cached_layout = SYS_DENSE_CACHE
                .with(|c| c.borrow().get(surface.canvas_id).map(|e| e.layout.clone()));
            let layout = cached_layout.unwrap_or_else(|| {
                let pins = graph_viewport::load_pins(&sys_pin_key(surface, &state));
                layout_system_nodes_with_pins(&doc, m::yes(&state["layers"]), &pins)
            });
            let rects: Vec<(f64, f64, f64, f64)> =
                layout.nodes.iter().map(|n| (n.x, n.y, n.w, n.h)).collect();
            let index_by_id: HashMap<&str, usize> = layout
                .nodes
                .iter()
                .enumerate()
                .map(|(i, n)| (n.id.as_str(), i))
                .collect();
            let curr_id = target
                .as_ref()
                .and_then(|el| el.closest(".system-map-node[data-n]").ok().flatten())
                .and_then(|el| el.get_attribute("data-n"))
                .unwrap_or_else(|| {
                    let f = m::text(&state, "roving_focus");
                    if f.is_empty() {
                        m::text(&state, "focus")
                    } else {
                        f
                    }
                });
            let curr_idx = index_by_id.get(curr_id.as_str()).copied().unwrap_or(0);
            let mut connected = Vec::new();
            for e in m::array(&doc["edges"]) {
                let f = m::text(&e, "from");
                let t = m::text(&e, "to");
                if f == curr_id {
                    if let Some(&i) = index_by_id.get(t.as_str()) {
                        connected.push(i);
                    }
                } else if t == curr_id {
                    if let Some(&i) = index_by_id.get(f.as_str()) {
                        connected.push(i);
                    }
                }
            }
            if let Some(next_idx) =
                graph_viewport::next_roving_node_index(&rects, &connected, curr_idx, &event.key())
            {
                if let Some(next_node) = layout.nodes.get(next_idx) {
                    let next_id = next_node.id.clone();
                    state["roving_focus"] = json!(next_id);
                    save_surface_state(surface, &state);
                    SYS_DENSE_CACHE.with(|c| {
                        if let Some(entry) = c.borrow_mut().get_mut(surface.canvas_id) {
                            entry.latest_roving_id = next_id.clone();
                        }
                    });
                    if let (Some(canvas), Some(plane)) = (
                        dom::get(surface.canvas_id),
                        dom::get(surface.canvas_id)
                            .and_then(|c| c.query_selector(".system-map-plane").ok().flatten()),
                    ) {
                        let mut cam = load_camera(surface);
                        let (vw, vh, measured) = viewport_size(surface.canvas_id);
                        let next_cam = graph_viewport::dense_dom::apply_roving_node_focus(
                            &canvas,
                            &plane,
                            cam.viewport_cam(),
                            if measured { (vw, vh) } else { (0.0, 0.0) },
                            &rects,
                            next_idx,
                            ".system-map-node[data-n]",
                            40.0,
                            |el| {
                                el.get_attribute("data-n")
                                    .and_then(|id| index_by_id.get(id.as_str()).copied())
                            },
                            |idx| {
                                SYS_DENSE_CACHE.with(|c| {
                                    let g = c.borrow();
                                    let entry = g.get(surface.canvas_id)?;
                                    entry
                                        .is_dense
                                        .then(|| entry.materialize_node_html(idx))
                                        .flatten()
                                })
                            },
                            |idx, el| {
                                if let Some(nid) = layout.nodes.get(idx).map(|n| n.id.as_str()) {
                                    SYS_DENSE_CACHE.with(|c| {
                                        if let Some(entry) = c.borrow().get(surface.canvas_id) {
                                            patch_system_node_el(
                                                el,
                                                nid,
                                                &entry.latest_focus_id,
                                                &entry.latest_roving_id,
                                                &entry.highlight_nodes,
                                                &entry.failed_services,
                                                &entry.blast_roles,
                                            );
                                        }
                                    });
                                    invalidate_hover_cache(surface.canvas_id);
                                }
                            },
                        );
                        if let (Some(updated_cam), true) = (next_cam, measured) {
                            cam.apply_viewport_cam(updated_cam);
                            save_camera(surface, &cam);
                            apply_camera(surface, &cam);
                        }
                    }
                    let label = m::array(&doc["nodes"])
                        .into_iter()
                        .find(|n| node_id(n) == next_id)
                        .map(|n| m::text(&n, "name"))
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| next_id.clone());
                    graph_viewport::announce_live(
                        &format!("{}-live-announce", surface.id_prefix),
                        &format!("Focused {label}"),
                    );
                    return true;
                }
            }
        }
        match event.key().as_str() {
            "/" => {
                if dom::get(surface.find_input_id)
                    .is_some_and(|el| el.closest("[hidden]").ok().flatten().is_none())
                {
                    event.prevent_default();
                    dom::focus(surface.find_input_id);
                    return true;
                }
            }
            "+" | "=" => {
                zoom_surface_at(surface, ZOOM_STEP, None);
                return true;
            }
            "-" => {
                zoom_surface_at(surface, 1.0 / ZOOM_STEP, None);
                return true;
            }
            "0" => {
                fit_surface_camera(surface);
                return true;
            }
            "1" => {
                reset_surface_camera_100(surface);
                return true;
            }
            _ => {}
        }
    }
    false
}

pub fn event(kind: &str, id: &str, action: &str, payload: &V, element: &web_sys::Element) -> bool {
    // Toolbar controls by element ID for both surfaces
    for surface in [SERVICES_SURFACE, CHANGES_SURFACE] {
        let p = surface.id_prefix;
        if let Some(suffix) = id.strip_prefix(p) {
            match (kind, suffix) {
                ("click", "-zoom-in") => {
                    zoom_surface_at(surface, ZOOM_STEP, None);
                    return true;
                }
                ("click", "-zoom-out") => {
                    zoom_surface_at(surface, 1.0 / ZOOM_STEP, None);
                    return true;
                }
                ("click", "-zoom-level") => {
                    reset_surface_camera_100(surface);
                    return true;
                }
                ("click", "-fit") => {
                    fit_surface_camera(surface);
                    return true;
                }
                ("click", "-expand") => {
                    let mut state = surface_state(surface);
                    state["expanded"] = json!(!m::yes(&state["expanded"]));
                    save_surface_state(surface, &state);
                    sync_toolbar_buttons(surface, &state);
                    fit_surface_camera(surface);
                    return true;
                }
                ("click", "-unfold") => {
                    let mut state = surface_state(surface);
                    state["whole"] = json!(!m::yes(&state["whole"]));
                    save_surface_state(surface, &state);
                    let cat = m::data("system-map-catalog-cache");
                    let st = m::data("system-map-status-cache");
                    spawn_local(async move {
                        fetch_surface_map(surface, cat, st, true).await;
                    });
                    return true;
                }
                ("click", "-certain") => {
                    let mut state = surface_state(surface);
                    state["certain"] = json!(!m::yes(&state["certain"]));
                    save_surface_state(surface, &state);
                    render_surface(surface, false);
                    let cat = m::data("system-map-catalog-cache");
                    let st = m::data("system-map-status-cache");
                    spawn_local(async move {
                        fetch_surface_map(surface, cat, st, false).await;
                    });
                    return true;
                }
                ("click", "-layers") => {
                    let mut state = surface_state(surface);
                    let next = !m::yes(&state["layers"]);
                    state["layers"] = json!(next);
                    if next {
                        state["side_tab"] = json!("analyze");
                        state["tool"] = json!("layers");
                    }
                    save_surface_state(surface, &state);
                    render_surface(surface, true);
                    return true;
                }
                ("click", "-reset-pins") => {
                    let state = surface_state(surface);
                    graph_viewport::clear_pins(&sys_pin_key(surface, &state));
                    render_surface(surface, false);
                    graph_viewport::announce_live(
                        &format!("{p}-live-announce"),
                        "Reset pinned layout",
                    );
                    return true;
                }
                ("click", "-isolate-toggle") => {
                    let mut state = surface_state(surface);
                    let next = !m::yes(&state["isolate_active"]);
                    let seed = {
                        let f = m::text(&state, "focus");
                        if !f.is_empty() {
                            f
                        } else {
                            m::text(&state, "isolate_seed")
                        }
                    };
                    if next && seed.is_empty() {
                        state["isolate_active"] = json!(false);
                        save_surface_state(surface, &state);
                        sync_toolbar_buttons(surface, &state);
                        graph_viewport::announce_live(
                            &format!("{p}-live-announce"),
                            "Select a service first to isolate its neighborhood",
                        );
                        return true;
                    }
                    let depth = dom::value(&format!("{p}-isolate-depth"))
                        .trim()
                        .parse::<u64>()
                        .unwrap_or(1)
                        .clamp(1, 3);
                    state["isolate_active"] = json!(next);
                    state["isolate_depth"] = json!(depth);
                    state["isolate_seed"] = if next { json!(seed) } else { json!("") };
                    save_surface_state(surface, &state);
                    render_surface(surface, true);
                    graph_viewport::announce_live(
                        &format!("{p}-live-announce"),
                        if next {
                            "Isolated neighborhood"
                        } else {
                            "Cleared isolation"
                        },
                    );
                    return true;
                }
                ("click", "-path-trace") => {
                    let from_q = dom::value(&format!("{p}-path-from"));
                    let to_q = dom::value(&format!("{p}-path-to"));
                    let mut state = surface_state(surface);
                    state["tool"] = json!("path");
                    state["path_from"] = json!(from_q.trim());
                    state["path_to"] = json!(to_q.trim());
                    save_surface_state(surface, &state);
                    render_surface(surface, false);
                    let view_doc = filtered_doc(&m::data(surface.map_key), &state);
                    let vis = compute_sys_visual_state(&view_doc, &state);
                    let msg = if state["path_from"].as_str().unwrap_or("").is_empty()
                        || state["path_to"].as_str().unwrap_or("").is_empty()
                    {
                        "Enter both source and destination services to trace a path"
                    } else if vis.highlight_nodes.is_empty() {
                        "No contract path found between selected services"
                    } else {
                        "Highlighted path between selected services"
                    };
                    graph_viewport::announce_live(&format!("{p}-live-announce"), msg);
                    return true;
                }
                ("click", "-path-clear") => {
                    let mut state = surface_state(surface);
                    state["tool"] = json!("");
                    state["path_from"] = json!("");
                    state["path_to"] = json!("");
                    save_surface_state(surface, &state);
                    render_surface(surface, false);
                    graph_viewport::announce_live(
                        &format!("{p}-live-announce"),
                        "Cleared path trace",
                    );
                    return true;
                }
                ("change", "-isolate-depth") => {
                    let depth = dom::value(id)
                        .trim()
                        .parse::<u64>()
                        .unwrap_or(1)
                        .clamp(1, 3);
                    let mut state = surface_state(surface);
                    state["isolate_depth"] = json!(depth);
                    let seed = {
                        let s = m::text(&state, "isolate_seed");
                        if !s.is_empty() {
                            s
                        } else {
                            m::text(&state, "focus")
                        }
                    };
                    if seed.is_empty() {
                        state["isolate_active"] = json!(false);
                        save_surface_state(surface, &state);
                        sync_toolbar_buttons(surface, &state);
                        graph_viewport::announce_live(
                            &format!("{p}-live-announce"),
                            "Select a service first to isolate its neighborhood",
                        );
                        return true;
                    }
                    state["isolate_seed"] = json!(seed);
                    state["isolate_active"] = json!(true);
                    save_surface_state(surface, &state);
                    render_surface(surface, true);
                    return true;
                }
                _ => {}
            }
        }
        if kind == "input" {
            if id == surface.filter_id {
                let mut state = surface_state(surface);
                state["filter"] = json!(dom::value(surface.filter_id));
                save_surface_state(surface, &state);
                render_surface(surface, false);
                return true;
            }
            if id == surface.find_input_id {
                let q = dom::value(surface.find_input_id);
                let timer_key = format!("{p}-find-debounce");
                if q.trim().is_empty() {
                    dom::cancel_timer(&timer_key);
                    close_finder(surface);
                    return true;
                }
                dom::later(&timer_key, 160, move || {
                    spawn_local(async move {
                        run_finder_search(surface, q).await;
                    });
                });
                return true;
            }
        }
    }

    if kind == "change" {
        let surf_attr = element.get_attribute("data-surface").unwrap_or_default();
        let surface = surface_by_id(&surf_attr);
        match action {
            "system-map-follow-root" => {
                let mut state = surface_state(surface);
                state["follow"] = json!(dom::value_of(element));
                state["out"] = json!([]);
                state["into"] = json!([]);
                save_surface_state(surface, &state);
                render_surface(surface, false);
                return true;
            }
            "system-map-path-from" => {
                let mut state = surface_state(surface);
                state["path_from"] = json!(dom::value_of(element));
                state["skip"] = json!([]);
                save_surface_state(surface, &state);
                render_surface(surface, false);
                return true;
            }
            "system-map-path-to" => {
                let mut state = surface_state(surface);
                state["path_to"] = json!(dom::value_of(element));
                state["skip"] = json!([]);
                save_surface_state(surface, &state);
                render_surface(surface, false);
                return true;
            }
            "system-map-shared-a" => {
                let mut state = surface_state(surface);
                state["shared_a"] = json!(dom::value_of(element));
                save_surface_state(surface, &state);
                render_surface(surface, false);
                return true;
            }
            "system-map-shared-b" => {
                let mut state = surface_state(surface);
                state["shared_b"] = json!(dom::value_of(element));
                save_surface_state(surface, &state);
                render_surface(surface, false);
                return true;
            }
            _ => {}
        }
    }

    if kind != "click" {
        return false;
    }

    let surface = surface_by_id(&m::text(payload, "surface"));
    if m::yes(&m::data(surface.drag_key)["suppress_click"]) {
        return true;
    }

    match action {
        "system-map-node" => {
            let nid = first_non_empty(payload, &["path", "id", "service"]);
            let cam = load_camera(surface);
            let mut state = surface_state(surface);
            let need_seed = SYS_NAV_HISTORY.with(|h| {
                h.borrow()
                    .get(surface.id)
                    .is_none_or(|nav| nav.entries().is_empty())
            });
            if need_seed {
                push_sys_nav_history(surface, &state, &cam);
            }
            state["focus"] = json!(nid);
            state["roving_focus"] = json!(nid);
            state["selected_edge"] = V::Null;
            state["side_tab"] = json!("details");
            save_surface_state(surface, &state);
            push_sys_nav_history(surface, &state, &cam);
            render_surface(surface, false);
            true
        }
        "system-map-edge" => {
            let cam = load_camera(surface);
            let mut state = surface_state(surface);
            let need_seed = SYS_NAV_HISTORY.with(|h| {
                h.borrow()
                    .get(surface.id)
                    .is_none_or(|nav| nav.entries().is_empty())
            });
            if need_seed {
                push_sys_nav_history(surface, &state, &cam);
            }
            state["selected_edge"] = json!({
                "from": m::text(payload, "from"),
                "to": m::text(payload, "to"),
            });
            state["side_tab"] = json!("details");
            save_surface_state(surface, &state);
            push_sys_nav_history(surface, &state, &cam);
            render_surface(surface, false);
            true
        }
        "system-map-clear-selection" => {
            let mut state = surface_state(surface);
            state["focus"] = json!("");
            state["selected_edge"] = V::Null;
            save_surface_state(surface, &state);
            render_surface(surface, false);
            true
        }
        "system-map-side-tab" => {
            let tab = m::text(payload, "tab");
            let mut state = surface_state(surface);
            state["side_tab"] = json!(tab.clone());
            save_surface_state(surface, &state);
            render_surface(surface, false);
            if tab == "changes" {
                spawn_local(async move {
                    fetch_surface_changes(surface).await;
                });
            }
            true
        }
        "system-map-open-service" => {
            let svc = m::text(payload, "service");
            if !svc.is_empty() {
                systems::drill_down_service_map(surface.id, &svc, None);
            }
            true
        }
        "system-map-spread" => {
            let svc = m::text(payload, "service");
            let mut state = surface_state(surface);
            let mut list: Vec<String> = m::array(&state["spread"])
                .into_iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            if let Some(pos) = list.iter().position(|x| x == &svc) {
                list.remove(pos);
            } else if !svc.is_empty() {
                list.push(svc);
            }
            state["spread"] = json!(list);
            save_surface_state(surface, &state);
            let cat = m::data("system-map-catalog-cache");
            let st = m::data("system-map-status-cache");
            spawn_local(async move {
                fetch_surface_map(surface, cat, st, false).await;
            });
            true
        }
        "system-map-service-changes" => {
            let svc = m::text(payload, "service");
            let mut state = surface_state(surface);
            state["focus"] = json!(svc);
            state["side_tab"] = json!("changes");
            save_surface_state(surface, &state);
            render_surface(surface, false);
            spawn_local(async move {
                fetch_surface_changes(surface).await;
            });
            true
        }
        "system-map-wave" => {
            dom::cancel_timer(&format!("{}-wave-play", surface.id_prefix));
            SYS_WAVE_PLAYING.with(|playing| {
                playing.borrow_mut().remove(surface.id);
            });
            let mut state = surface_state(surface);
            state["wave"] = match payload["wave"].as_u64() {
                Some(wave) => json!(wave),
                None => V::Null,
            };
            save_surface_state(surface, &state);
            render_surface(surface, false);
            true
        }
        "system-map-wave-play" => {
            dom::cancel_timer(&format!("{}-wave-play", surface.id_prefix));
            play_wave(surface, 0);
            true
        }
        "system-map-wave-pause" => {
            dom::cancel_timer(&format!("{}-wave-play", surface.id_prefix));
            SYS_WAVE_PLAYING.with(|playing| {
                playing.borrow_mut().remove(surface.id);
            });
            render_surface(surface, false);
            true
        }
        "system-map-clear-filters" => {
            let mut state = surface_state(surface);
            state["filter"] = json!("");
            state["certain"] = json!(false);
            state["isolate_active"] = json!(false);
            state["isolate_seed"] = json!("");
            state["tool"] = json!("");
            state["path_from"] = json!("");
            state["path_to"] = json!("");
            state["hidden_kinds"] = json!([]);
            save_surface_state(surface, &state);
            dom::set_value(surface.filter_id, "");
            render_surface(surface, true);
            graph_viewport::announce_live(
                &format!("{}-live-announce", surface.id_prefix),
                "Reset system map filters",
            );
            true
        }
        "system-map-unpin-node" => {
            let nid = first_non_empty(payload, &["id", "path", "service"]);
            if !nid.is_empty() {
                let state = surface_state(surface);
                let pin_key = sys_pin_key(surface, &state);
                let mut pins = graph_viewport::load_pins(&pin_key);
                if pins.remove(&nid).is_some() {
                    graph_viewport::clear_pins(&pin_key);
                    for (k, pos) in pins {
                        graph_viewport::save_pin(&pin_key, &k, pos);
                    }
                    graph_viewport::announce_live(
                        &format!("{}-live-announce", surface.id_prefix),
                        &format!("Unpinned {nid}"),
                    );
                    render_surface(surface, false);
                }
            }
            true
        }
        "system-map-nav-back" | "system-map-nav-forward" => {
            let back = action == "system-map-nav-back";
            let entry = SYS_NAV_HISTORY.with(|h| {
                let mut map = h.borrow_mut();
                let nav = map.get_mut(surface.id)?;
                if back {
                    nav.back().cloned()
                } else {
                    nav.forward().cloned()
                }
            });
            if let Some(entry) = entry {
                let (focus, selected_edge, pan_x, pan_y, scale) = parse_sys_nav_entry(&entry);
                let mut state = surface_state(surface);
                state["focus"] = json!(focus);
                state["roving_focus"] = json!(focus);
                state["selected_edge"] = selected_edge;
                state["side_tab"] = json!("details");
                save_surface_state(surface, &state);
                let mut cam = load_camera(surface);
                cam.pan_x = pan_x;
                cam.pan_y = pan_y;
                cam.scale = scale;
                cam.fit_mode = false;
                save_camera(surface, &cam);
                render_surface(surface, false);
            }
            true
        }
        "system-map-open-omnigraph" => {
            let svc = first_non_empty(payload, &["service", "id"]);
            let contract = first_non_empty(payload, &["contract"]);
            let file = first_non_empty(payload, &["file", "path"]);
            let run_id = {
                let direct = first_non_empty(payload, &["run_id"]);
                if !direct.is_empty() {
                    direct
                } else {
                    let st = surface_state(surface);
                    let r = m::text(&st, "run_id");
                    if !r.is_empty() {
                        r
                    } else {
                        let tgt = m::text(&st, "target");
                        tgt.strip_prefix("sysrun:")
                            .or_else(|| tgt.strip_prefix("system-run:"))
                            .unwrap_or("")
                            .trim()
                            .to_owned()
                    }
                }
            };
            spawn_local(async move {
                crate::omnigraph::open_from_system_context(
                    svc,
                    (!contract.is_empty()).then_some(contract),
                    (!file.is_empty()).then_some(file),
                    (!run_id.is_empty()).then_some(run_id),
                )
                .await;
            });
            true
        }
        "system-map-bridge" => {
            let (from, to) = (m::text(payload, "from"), m::text(payload, "to"));
            spawn_local(async move {
                fetch_bridge(surface, from, to).await;
            });
            true
        }
        "system-map-contract-impact" => {
            let svc = m::text(payload, "service");
            let contract = m::text(payload, "contract");
            spawn_local(async move {
                fetch_contract_impact(surface, svc, contract).await;
            });
            true
        }
        "system-map-tool" => {
            let tool = m::text(payload, "tool");
            let mut state = surface_state(surface);
            state["side_tab"] = json!("analyze");
            state["tool"] = json!(tool.clone());
            if !m::text(payload, "service").is_empty() {
                state["follow"] = payload["service"].clone();
            }
            if !m::text(payload, "dir").is_empty() {
                state["follow_dir"] = payload["dir"].clone();
            }
            if !m::text(payload, "from").is_empty() {
                state["path_from"] = payload["from"].clone();
            }
            if !m::text(payload, "a").is_empty() {
                state["shared_a"] = payload["a"].clone();
            }
            if payload["index"].is_u64() {
                state["cycle_index"] = payload["index"].clone();
            }
            save_surface_state(surface, &state);
            render_surface(surface, false);
            true
        }
        "system-map-follow-hop" => {
            let dir = m::text(payload, "dir");
            let hop = m::text(payload, "hop");
            let key = if dir == "into" { "into" } else { "out" };
            let mut state = surface_state(surface);
            let mut list: Vec<String> = m::array(&state[key])
                .into_iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            if let Some(pos) = list.iter().position(|x| x == &hop) {
                list.remove(pos);
            } else if !hop.is_empty() {
                list.push(hop);
            }
            state[key] = json!(list);
            save_surface_state(surface, &state);
            render_surface(surface, false);
            true
        }
        "system-map-path-swap" => {
            let mut state = surface_state(surface);
            let a = m::text(&state, "path_from");
            let b = m::text(&state, "path_to");
            state["path_from"] = json!(b);
            state["path_to"] = json!(a);
            state["skip"] = json!([]);
            save_surface_state(surface, &state);
            render_surface(surface, false);
            true
        }
        "system-map-path-skip" => {
            let svc = m::text(payload, "service");
            let mut state = surface_state(surface);
            let mut list: Vec<String> = m::array(&state["skip"])
                .into_iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            if let Some(pos) = list.iter().position(|x| x == &svc) {
                list.remove(pos);
            } else if !svc.is_empty() {
                list.push(svc);
            }
            state["skip"] = json!(list);
            save_surface_state(surface, &state);
            render_surface(surface, false);
            true
        }
        "system-map-shared-side" => {
            let mut state = surface_state(surface);
            state["shared_side"] = payload["side"].clone();
            save_surface_state(surface, &state);
            render_surface(surface, false);
            true
        }
        "system-map-toggle-layers" => {
            let mut state = surface_state(surface);
            state["layers"] = json!(!m::yes(&state["layers"]));
            save_surface_state(surface, &state);
            render_surface(surface, true);
            true
        }
        "system-map-refresh-changes" => {
            spawn_local(async move {
                fetch_surface_changes(surface).await;
            });
            true
        }
        "system-map-kind-toggle" | "system-map-kind-preset" => {
            let state = surface_state(surface);
            let next = if action == "system-map-kind-preset" {
                preset_contract_kinds(state, &m::text(payload, "preset"))
            } else {
                switched_contract_kind(
                    state,
                    &m::text(payload, "kind"),
                    m::yes(&m::data("pointer-mods")["alt"]),
                )
            };
            save_surface_state(surface, &next);
            render_surface(surface, false);
            let wanted = payload.to_string();
            if let Some(button) =
                dom::all(&format!("#{} [data-action='{action}']", surface.legend_id))
                    .into_iter()
                    .find(|el| el.get_attribute("data-payload").as_deref() == Some(wanted.as_str()))
                    .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
            {
                let _ = button.focus();
            }
            true
        }
        "system-map-find-pick" => {
            close_finder(surface);
            let kind = m::text(payload, "kind");
            let svc = m::text(payload, "service");
            let mut path = m::text(payload, "path");
            if let Some(rest) = path.strip_prefix(&format!("source:{svc}:")) {
                path = rest.to_owned();
            }
            if matches!(
                kind.as_str(),
                "entity" | "source" | "symbol" | "file" | "class" | "method" | "function"
            ) && !svc.is_empty()
            {
                systems::drill_down_service_map(surface.id, &svc, Some(&path));
            } else {
                let mut state = surface_state(surface);
                state["focus"] = json!(if !svc.is_empty() { svc } else { path });
                state["selected_edge"] = V::Null;
                state["side_tab"] = json!("details");
                save_surface_state(surface, &state);
                render_surface(surface, false);
            }
            true
        }
        _ => false,
    }
}
