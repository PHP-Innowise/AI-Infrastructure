use super::*;

pub fn attach_wheel_listeners() {
    graph_viewport::attach_wheel_zoom(
        "omnigraph-canvas",
        "omnigraph-canvas-wheel-end",
        |factor, pt| zoom_canvas_at(factor, Some(pt)),
    );
}

pub(super) fn radar_element() -> Option<web_sys::Element> {
    dom::get("omnigraph-canvas-radar").and_then(|el| {
        el.query_selector(".omni-radar-svg")
            .ok()
            .flatten()
            .or(Some(el))
    })
}

pub fn pointer_event(kind: &str, event: &web_sys::Event) -> bool {
    if m::read(|s| s.view != "omnigraph") {
        return false;
    }
    let target_el = event
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
    let in_radar = target_el
        .as_ref()
        .and_then(|el| el.closest("#omnigraph-canvas-radar").ok().flatten())
        .is_some();
    let mut radar_drag = graph_viewport::RadarDrag::from_json(&m::data("omnigraph-radar-drag"));
    if in_radar || radar_drag.active {
        if let Some(radar) = radar_element() {
            let mut cam = load_camera();
            let plane = (cam.plane_w.max(320.0), cam.plane_h.max(240.0));
            match graph_viewport::handle_radar_pointer(
                kind,
                event,
                &radar,
                Some("omnigraph-canvas"),
                plane,
                &mut radar_drag,
            ) {
                graph_viewport::RadarAction::MoveTo { world_point } => {
                    m::set("omnigraph-radar-drag", radar_drag.to_json());
                    let next =
                        graph_viewport::center_camera_on(world_point, viewport_size(), cam.scale);
                    cam.apply_viewport(next);
                    cam.fit_mode = false;
                    save_camera(&cam);
                    apply_camera(&cam);
                    return true;
                }
                graph_viewport::RadarAction::Released => {
                    m::set("omnigraph-radar-drag", radar_drag.to_json());
                    return true;
                }
                graph_viewport::RadarAction::Ignored => {
                    if in_radar {
                        return false;
                    }
                }
            }
        }
    }
    let in_canvas = target_el
        .as_ref()
        .and_then(|el| el.closest("#omnigraph-canvas").ok().flatten())
        .is_some();

    fn sync_live_omni_pin_drag_edges(
        dragged_id: &str,
        nx: f64,
        ny: f64,
        is_dense: bool,
        cam: &OmnigraphCamera,
    ) {
        let mut updated_dense: Option<Vec<(usize, omnigraph_canvas::CubicPoints)>> = None;
        let mut svg_updates: Vec<(String, String, String)> = Vec::new();
        OMNI_DENSE_CACHE.with(|c| {
            let mut guard = c.borrow_mut();
            let Some(entry) = guard.as_mut() else {
                return;
            };
            let Some(&drag_idx) = entry.node_index_by_id.get(dragged_id) else {
                return;
            };
            if let Some(rect) = entry.node_rects.get_mut(drag_idx) {
                rect.1 = nx;
                rect.2 = ny;
            }
            let mut dense_list = Vec::new();
            for (edge_idx, (from, to)) in entry.edge_endpoints.iter().enumerate() {
                if from != dragged_id && to != dragged_id {
                    continue;
                }
                let Some(&u_idx) = entry.node_index_by_id.get(from) else {
                    continue;
                };
                let Some(&v_idx) = entry.node_index_by_id.get(to) else {
                    continue;
                };
                let (_, ux, uy, uw, uh) = entry.node_rects[u_idx];
                let (_, vx, vy, vw, vh) = entry.node_rects[v_idx];
                let cubic = omnigraph_canvas::route_rect_cubic(
                    (ux, uy, uw, uh),
                    (vx, vy, vw, vh),
                    from == to,
                    0,
                    1,
                    edge_idx,
                );
                if is_dense {
                    dense_list.push((edge_idx, cubic));
                } else {
                    let path_d = format!(
                        "M {:.1} {:.1} C {:.1} {:.1}, {:.1} {:.1}, {:.1} {:.1}",
                        cubic.0 .0,
                        cubic.0 .1,
                        cubic.1 .0,
                        cubic.1 .1,
                        cubic.2 .0,
                        cubic.2 .1,
                        cubic.3 .0,
                        cubic.3 .1
                    );
                    svg_updates.push((from.clone(), to.clone(), path_d));
                }
            }
            if is_dense {
                updated_dense = Some(dense_list);
            }
        });

        if let Some(edge_updates) = updated_dense {
            let drag_idx_opt = OMNI_DENSE_CACHE.with(|c| {
                c.borrow()
                    .as_ref()
                    .and_then(|e| e.node_index_by_id.get(dragged_id).copied())
            });
            graph_viewport::canvas2d::update_scene("omnigraph-canvas", |scene| {
                if let Some(d_idx) = drag_idx_opt {
                    if let Some(n) = scene.nodes.get_mut(d_idx) {
                        n.x = nx;
                        n.y = ny;
                        n.pinned = true;
                    }
                }
                for (edge_idx, (p0, p1, p2, p3)) in edge_updates {
                    if let Some(old) = scene.edges.get(edge_idx).cloned() {
                        scene.edges[edge_idx] = graph_viewport::canvas2d::DenseEdgeSpec::from_cubic(
                            old.index,
                            p0,
                            p1,
                            p2,
                            p3,
                            old.weight,
                            old.guessed,
                            old.against,
                            old.kind,
                            old.on,
                            old.lit,
                            old.dim,
                            old.portal,
                        );
                    }
                }
            });
            graph_viewport::canvas2d::paint_scene(
                "omnigraph-canvas",
                cam.to_viewport(),
                None,
                true,
            );
        } else if !svg_updates.is_empty() {
            if let Some(canvas) = dom::get("omnigraph-canvas") {
                for (from, to, d) in svg_updates {
                    let f_esc = crate::dom::escape_css_attr_value(&from);
                    let t_esc = crate::dom::escape_css_attr_value(&to);
                    if let Ok(edges) = canvas.query_selector_all(&format!(
                        ".omni-edge[data-from=\"{f_esc}\"][data-to=\"{t_esc}\"]"
                    )) {
                        for i in 0..edges.length() {
                            if let Some(el) = edges
                                .item(i)
                                .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
                            {
                                let _ = el.set_attribute("d", &d);
                            }
                        }
                    }
                }
            }
        }
    }

    fn select_omni_node_with_history(nid: String) {
        let branch = m::text(&ui_state(), "branch");
        let cam = load_camera();
        OMNI_NAV_HISTORY.with(|h| {
            h.borrow_mut()
                .push(omni_nav_entry_from(Some(nid.clone()), &branch, &cam));
        });
        set_ui_state(|s| {
            s["selected_id"] = json!(nid);
            s["focused_id"] = json!(nid);
            s["drawer_tab"] = json!("inspector");
        });
        render_tabs();
        render_constellation();
        render_inspector();
    }

    fn dense_file_target_for_node(graph: &V, nid: &str) -> Option<(String, Option<String>)> {
        if let Some(f) = m::array(&graph["files"])
            .into_iter()
            .find(|f| file_node_id(f) == nid || f["path"].as_str() == Some(nid))
        {
            let p = f["path"].as_str().unwrap_or(nid).trim();
            if !p.is_empty() {
                return Some((p.to_owned(), None));
            }
        }
        if let Some(sym) = m::array(&graph["symbols"]).into_iter().find(|s| {
            symbol_node_id(s) == nid
                || s["symbol_id"].as_str() == Some(nid)
                || s["fqn"].as_str() == Some(nid)
        }) {
            let p = symbol_file_path(&sym);
            if !p.is_empty() {
                let target_sym = sym["fqn"]
                    .as_str()
                    .or_else(|| sym["symbol_id"].as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                return Some((p, target_sym));
            }
        }
        if let Some(atom) = m::array(&graph["atoms"])
            .into_iter()
            .find(|a| atom_id(a) == nid)
        {
            if let Some(p) = m::array(&atom["files"])
                .into_iter()
                .filter_map(|v| {
                    v.as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                })
                .next()
            {
                return Some((p, None));
            }
        }
        None
    }

    let plane =
        dom::get("omnigraph-canvas").and_then(|c| c.query_selector(".omni-plane").ok().flatten());
    let plane_empty = plane.as_ref().is_some_and(|p| {
        graph_viewport::dense_dom::is_empty_plane_attr(p.get_attribute("data-empty").as_deref())
    });
    let in_empty = plane_empty
        || target_el
            .as_ref()
            .is_some_and(|el| el.closest(".omni-empty-state").ok().flatten().is_some());
    let in_omni_node = target_el.as_ref().is_some_and(|el| {
        el.closest(".omni-node[data-node-id]")
            .ok()
            .flatten()
            .is_some()
    });
    let in_interactive = graph_viewport::dense_dom::is_interactive_or_empty_target(
        target_el.as_ref(),
        plane.as_ref(),
        Some(".omni-empty-state"),
    );

    if kind == "pointerdown" && in_canvas {
        let mut drag = graph_viewport::DragPan::from_json(&m::data("omnigraph-drag"));
        if drag.suppress_click {
            drag.clear_suppress();
            m::set("omnigraph-drag", drag.to_json());
        }
    }

    // Shift+drag node pinning on constellation cards.
    if matches!(
        kind,
        "pointerdown" | "pointermove" | "pointerup" | "pointercancel"
    ) {
        let mut pin_drag =
            graph_viewport::dense_dom::PinDragSession::from_json(&m::data("omnigraph-node-drag"));
        if kind == "pointerdown" && in_canvas && !in_empty {
            if let Ok(pe) = event.clone().dyn_into::<web_sys::PointerEvent>() {
                if pe.shift_key() {
                    let resolved = if let Some(node_el) = target_el
                        .as_ref()
                        .and_then(|el| el.closest(".omni-node[data-node-id]").ok().flatten())
                    {
                        let nid = node_el.get_attribute("data-node-id").unwrap_or_default();
                        let idx = OMNI_DENSE_CACHE
                            .with(|c| {
                                c.borrow()
                                    .as_ref()
                                    .and_then(|e| e.node_index_by_id.get(&nid).copied())
                            })
                            .unwrap_or(0);
                        (nid, Some(node_el), idx)
                    } else {
                        let is_dense = OMNI_DENSE_CACHE
                            .with(|c| c.borrow().as_ref().is_some_and(|e| e.is_dense));
                        if is_dense {
                            let cam = load_camera();
                            graph_viewport::dense_dom::hit_test_pointer_in_canvas(
                                "omnigraph-canvas",
                                pe.client_x() as f64,
                                pe.client_y() as f64,
                                cam.to_viewport(),
                                4.0,
                            )
                            .and_then(|hit| match hit {
                                (0, idx) => {
                                    let nid = OMNI_DENSE_CACHE.with(|c| {
                                        c.borrow()
                                            .as_ref()?
                                            .node_rects
                                            .get(idx)
                                            .map(|(id, ..)| id.clone())
                                    })?;
                                    let el = mount_omni_dense_node_if_needed(&nid);
                                    Some((nid, el, idx))
                                }
                                _ => None,
                            })
                            .unwrap_or_default()
                        } else {
                            (String::new(), None, 0)
                        }
                    };
                    let (node_id, maybe_el, node_idx) = resolved;
                    let raw = m::data("omnigraph-raw-positions");
                    if let (false, Some(x0), Some(y0)) = (
                        node_id.is_empty(),
                        raw[&node_id][0].as_f64(),
                        raw[&node_id][1].as_f64(),
                    ) {
                        if let Some(cap) = maybe_el.or_else(|| dom::get("omnigraph-canvas")) {
                            graph_viewport::capture_pointer(&cap, pe.pointer_id());
                        }
                        let session = graph_viewport::dense_dom::PinDragSession::begin(
                            pe.pointer_id(),
                            node_idx,
                            node_id,
                            (pe.client_x() as f64, pe.client_y() as f64),
                            (x0, y0),
                        );
                        m::set("omnigraph-node-drag", session.to_json());
                        return true;
                    }
                }
            }
        } else if pin_drag.active {
            if let Ok(pe) = event.clone().dyn_into::<web_sys::PointerEvent>() {
                let cam = load_camera();
                let bounds = (8.0, 8.0, 1_000_000.0, 1_000_000.0);
                if kind == "pointermove" {
                    if let Some((nx, ny)) = pin_drag.advance(
                        pe.pointer_id(),
                        (pe.client_x() as f64, pe.client_y() as f64),
                        cam.scale.max(0.1),
                        bounds,
                        2.0,
                    ) {
                        let node_id = pin_drag.node_key.clone();
                        m::set("omnigraph-node-drag", pin_drag.to_json());
                        if let Some(el) = dom::get("omnigraph-canvas")
                            .and_then(|c| find_omni_node_el(&c, &node_id))
                        {
                            let _ = el.remove_attribute("data-offscreen");
                            let _ = el.set_attribute("data-pinned", "true");
                            if let Ok(html_el) = el.dyn_into::<web_sys::HtmlElement>() {
                                let _ = html_el.style().set_property("left", &format!("{nx:.1}px"));
                                let _ = html_el.style().set_property("top", &format!("{ny:.1}px"));
                            }
                        }
                        let is_dense = OMNI_DENSE_CACHE
                            .with(|c| c.borrow().as_ref().is_some_and(|e| e.is_dense));
                        sync_live_omni_pin_drag_edges(&node_id, nx, ny, is_dense, &cam);
                    }
                    return true;
                }
                if let Some(outcome) = pin_drag.end(Some(pe.pointer_id()), kind == "pointercancel")
                {
                    m::set("omnigraph-node-drag", V::Null);
                    match outcome {
                        graph_viewport::dense_dom::PinDragOutcome::Committed {
                            node_key,
                            pos,
                            ..
                        } => {
                            graph_viewport::save_pin(&omni_pin_key(), &node_key, pos);
                            dom::hidden("omni-reset-pins", false);
                            let mut drag =
                                graph_viewport::DragPan::from_json(&m::data("omnigraph-drag"));
                            drag.suppress_click = true;
                            m::set("omnigraph-drag", drag.to_json());
                            dom::later("omnigraph-drag-clear", 70, || {
                                let mut d =
                                    graph_viewport::DragPan::from_json(&m::data("omnigraph-drag"));
                                d.clear_suppress();
                                m::set("omnigraph-drag", d.to_json());
                            });
                            graph_viewport::announce_live(
                                "omnigraph-live-announce",
                                &format!("Pinned node {node_key}"),
                            );
                            render_constellation();
                            render_inspector();
                        }
                        graph_viewport::dense_dom::PinDragOutcome::Cancelled { .. } => {
                            render_constellation();
                        }
                        graph_viewport::dense_dom::PinDragOutcome::Click { .. } => {}
                    }
                    return true;
                }
            }
        }
    }

    let mut drag = graph_viewport::DragPan::from_json(&m::data("omnigraph-drag"));
    if kind == "pointerdown" && (!in_canvas || in_interactive) {
        return false;
    }
    if (kind == "pointerdown" && in_canvas)
        || (drag.active && matches!(kind, "pointermove" | "pointerup" | "pointercancel"))
    {
        let mut cam = load_camera();
        match graph_viewport::handle_canvas_drag(
            kind,
            event,
            "omnigraph-canvas",
            cam.to_viewport(),
            DRAG_THRESHOLD_PX,
            &mut drag,
        ) {
            graph_viewport::CanvasDragOutcome::Down => {
                m::set("omnigraph-drag", drag.to_json());
                return true;
            }
            graph_viewport::CanvasDragOutcome::Panned { tx, ty } => {
                m::set("omnigraph-drag", drag.to_json());
                cam.pan_x = tx;
                cam.pan_y = ty;
                cam.fit_mode = false;
                save_camera(&cam);
                apply_camera(&cam);
                return true;
            }
            graph_viewport::CanvasDragOutcome::Ended { suppress_click } => {
                m::set("omnigraph-drag", drag.to_json());
                if !suppress_click && kind == "pointerup" && in_canvas {
                    if let Some(me) = event.dyn_ref::<web_sys::MouseEvent>() {
                        if target_el
                            .as_ref()
                            .and_then(|el| el.closest(".omni-node").ok().flatten())
                            .is_none()
                        {
                            match graph_viewport::dense_dom::hit_test_pointer_in_canvas(
                                "omnigraph-canvas",
                                me.client_x() as f64,
                                me.client_y() as f64,
                                cam.to_viewport(),
                                8.0,
                            ) {
                                Some((0, idx)) => {
                                    if let Some(nid) = m::array(&m::data("omnigraph-nav-rects"))
                                        .get(idx)
                                        .and_then(|v| v["id"].as_str())
                                        .map(str::to_owned)
                                    {
                                        select_omni_node_with_history(nid);
                                    }
                                }
                                Some((1, edge_idx)) => {
                                    let pair = OMNI_DENSE_CACHE.with(|c| {
                                        c.borrow()
                                            .as_ref()
                                            .and_then(|e| e.edge_endpoints.get(edge_idx).cloned())
                                    });
                                    if let Some((from, to)) = pair {
                                        set_ui_state(|s| {
                                            s["selected_id"] = json!(from.clone());
                                            s["focused_id"] = json!(from.clone());
                                            s["highlighted_ids"] = json!([from, to]);
                                            s["drawer_tab"] = json!("inspector");
                                        });
                                        render_tabs();
                                        render_constellation();
                                        render_inspector();
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                if suppress_click {
                    dom::later("omnigraph-drag-clear", 70, || {
                        let mut d = graph_viewport::DragPan::from_json(&m::data("omnigraph-drag"));
                        d.clear_suppress();
                        m::set("omnigraph-drag", d.to_json());
                    });
                }
                return true;
            }
            graph_viewport::CanvasDragOutcome::Ignored => {
                if drag.active {
                    return true;
                }
            }
        }
    }
    if kind == "pointermove" && in_canvas {
        let is_dense = OMNI_DENSE_CACHE.with(|c| c.borrow().as_ref().is_some_and(|e| e.is_dense));
        if is_dense {
            if let (Some(me), Some(canvas_el)) = (
                event.dyn_ref::<web_sys::MouseEvent>(),
                dom::get("omnigraph-canvas"),
            ) {
                let cam = load_camera();
                let hit = graph_viewport::dense_dom::hit_test_pointer_in_canvas(
                    "omnigraph-canvas",
                    me.client_x() as f64,
                    me.client_y() as f64,
                    cam.to_viewport(),
                    8.0,
                );
                let changed = OMNI_HOVER_HIT.with(|prev| {
                    let mut guard = prev.borrow_mut();
                    if *guard == Some(hit) {
                        false
                    } else {
                        *guard = Some(hit);
                        true
                    }
                });
                if changed {
                    graph_viewport::canvas2d::paint_scene(
                        "omnigraph-canvas",
                        cam.to_viewport(),
                        hit,
                        true,
                    );
                    if let Some(plane_el) = plane.as_ref() {
                        let hovered_idx = hit.and_then(|(layer, idx)| (layer == 0).then_some(idx));
                        let hovered_id = hovered_idx.and_then(|idx| {
                            OMNI_DENSE_CACHE.with(|c| {
                                c.borrow()
                                    .as_ref()
                                    .and_then(|e| e.node_rects.get(idx).map(|(id, ..)| id.clone()))
                            })
                        });
                        let hovered_el = hovered_id
                            .as_deref()
                            .and_then(|nid| find_omni_node_el(&canvas_el, nid));
                        graph_viewport::dense_dom::sync_dom_hover_attr(
                            plane_el,
                            hovered_idx,
                            hovered_el.as_ref(),
                            |el| {
                                el.get_attribute("data-node-id").and_then(|nid| {
                                    OMNI_DENSE_CACHE.with(|c| {
                                        c.borrow()
                                            .as_ref()
                                            .and_then(|e| e.node_index_by_id.get(&nid).copied())
                                    })
                                })
                            },
                        );
                    }
                    if let Ok(he) = canvas_el.dyn_into::<web_sys::HtmlElement>() {
                        let _ = he
                            .style()
                            .set_property("cursor", if hit.is_some() { "pointer" } else { "" });
                    }
                }
            }
        }
    }
    if kind == "dblclick" && in_canvas && !in_empty && (!in_interactive || in_omni_node) {
        if let Some(node_el) = target_el
            .as_ref()
            .and_then(|el| el.closest(".omni-node[data-file-path]").ok().flatten())
        {
            if let Some(path) = node_el
                .get_attribute("data-file-path")
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
            {
                let symbol_fqn = node_el
                    .get_attribute("data-symbol-fqn")
                    .or_else(|| node_el.get_attribute("data-symbol-id"))
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty());
                let project = current_project_id();
                spawn_local(async move {
                    open_target_in_code_map(
                        (!project.is_empty()).then_some(project.as_str()),
                        &path,
                        symbol_fqn.as_deref(),
                    )
                    .await;
                });
                return true;
            }
        }
        let is_dense = OMNI_DENSE_CACHE.with(|c| c.borrow().as_ref().is_some_and(|e| e.is_dense));
        if is_dense {
            if let Some(me) = event.dyn_ref::<web_sys::MouseEvent>() {
                let cam = load_camera();
                if let Some((0, idx)) = graph_viewport::dense_dom::hit_test_pointer_in_canvas(
                    "omnigraph-canvas",
                    me.client_x() as f64,
                    me.client_y() as f64,
                    cam.to_viewport(),
                    8.0,
                ) {
                    let nid = OMNI_DENSE_CACHE.with(|c| {
                        c.borrow()
                            .as_ref()
                            .and_then(|e| e.node_rects.get(idx).map(|(id, ..)| id.clone()))
                    });
                    if let Some(node_id) = nid {
                        let graph = m::data("omnigraph-graph");
                        if let Some((path, symbol_fqn)) =
                            dense_file_target_for_node(&graph, &node_id)
                        {
                            let project = current_project_id();
                            spawn_local(async move {
                                open_target_in_code_map(
                                    (!project.is_empty()).then_some(project.as_str()),
                                    &path,
                                    symbol_fqn.as_deref(),
                                )
                                .await;
                            });
                            return true;
                        }
                    }
                }
            }
        }
        if let Ok(me) = event.clone().dyn_into::<web_sys::MouseEvent>() {
            zoom_canvas_at(
                ZOOM_STEP,
                Some((me.client_x() as f64, me.client_y() as f64)),
            );
        }
        return true;
    }
    false
}

pub fn constellation_hotkey_allowed(
    view: &str,
    stage_tab: &str,
    studio_hidden: bool,
    modal_open: bool,
    in_editable: bool,
    has_mods: bool,
) -> bool {
    view == "omnigraph"
        && (stage_tab.is_empty() || stage_tab == "constellation")
        && !studio_hidden
        && !modal_open
        && !in_editable
        && !has_mods
}

pub(super) fn any_omni_modal_open() -> bool {
    for id in [
        "omni-settings-dialog",
        "omnigraph-transfer-dialog",
        "omnigraph-share-dialog",
    ] {
        if dom::get(id).is_some_and(|el| el.has_attribute("open")) {
            return true;
        }
    }
    !dom::all("dialog[open]").is_empty()
}

pub fn key(event: &web_sys::KeyboardEvent) -> bool {
    let target = event
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
    if let Some(el) = target.as_ref() {
        if el.get_attribute("role").as_deref() == Some("tab")
            && el
                .closest("#omnigraph-transfer-dialog")
                .ok()
                .flatten()
                .is_some()
        {
            let key = event.key();
            if ["ArrowLeft", "ArrowRight", "Home", "End"].contains(&key.as_str()) {
                let next = match (el.id().as_str(), key.as_str()) {
                    (_, "Home") => "export",
                    (_, "End") => "import",
                    ("omni-transfer-tab-export", "ArrowRight" | "ArrowLeft") => "import",
                    ("omni-transfer-tab-import", "ArrowRight" | "ArrowLeft") => "export",
                    _ => "export",
                };
                set_transfer_tab(next);
                dom::focus(&format!("omni-transfer-tab-{next}"));
                return true;
            }
        }
    }

    if m::read(|s| s.view != "omnigraph") {
        return false;
    }
    let modal_open = any_omni_modal_open();
    let studio_hidden = dom::get("omnigraph-studio").is_none_or(|el| el.has_attribute("hidden"));
    let stage_tab = m::text(&ui_state(), "stage_tab");
    let stage_tab_str = if stage_tab.is_empty() {
        "constellation"
    } else {
        stage_tab.as_str()
    };
    let in_input = target.as_ref().is_some_and(|el| {
        matches!(el.tag_name().as_str(), "INPUT" | "TEXTAREA" | "SELECT")
            || el
                .closest("[contenteditable=\"true\"]")
                .ok()
                .flatten()
                .is_some()
    });
    let has_mods = event.ctrl_key() || event.meta_key() || event.alt_key();

    if event.key() == "/"
        && !has_mods
        && !in_input
        && !modal_open
        && !studio_hidden
        && dom::get("omnigraph-search")
            .is_some_and(|el| el.closest("[hidden]").ok().flatten().is_none())
    {
        event.prevent_default();
        dom::focus("omnigraph-search");
        return true;
    }

    if let Some(el) = target.as_ref() {
        if el.get_attribute("role").as_deref() == Some("tab")
            && el.closest("#omnigraph-view").ok().flatten().is_some()
            && !modal_open
        {
            let key = event.key();
            if ["ArrowLeft", "ArrowRight", "Home", "End"].contains(&key.as_str()) {
                let stage_tabs = ["constellation", "dag", "merge"];
                let drawer_tabs = ["inspector", "pulse", "publish"];
                let id = el.id();
                if let Some(pos) = stage_tabs
                    .iter()
                    .position(|t| id == format!("omni-tab-{t}"))
                {
                    event.prevent_default();
                    let next = match key.as_str() {
                        "ArrowRight" => (pos + 1) % stage_tabs.len(),
                        "ArrowLeft" => (pos + stage_tabs.len() - 1) % stage_tabs.len(),
                        "Home" => 0,
                        _ => stage_tabs.len() - 1,
                    };
                    let tab = stage_tabs[next];
                    set_ui_state(|s| s["stage_tab"] = json!(tab));
                    render_tabs();
                    dom::focus(&format!("omni-tab-{tab}"));
                    return true;
                }
                if let Some(pos) = drawer_tabs
                    .iter()
                    .position(|t| id == format!("omni-drawer-tab-{t}"))
                {
                    event.prevent_default();
                    let next = match key.as_str() {
                        "ArrowRight" => (pos + 1) % drawer_tabs.len(),
                        "ArrowLeft" => (pos + drawer_tabs.len() - 1) % drawer_tabs.len(),
                        "Home" => 0,
                        _ => drawer_tabs.len() - 1,
                    };
                    let tab = drawer_tabs[next];
                    set_ui_state(|s| s["drawer_tab"] = json!(tab));
                    render_tabs();
                    dom::focus(&format!("omni-drawer-tab-{tab}"));
                    return true;
                }
            }
        }
    }

    if constellation_hotkey_allowed(
        "omnigraph",
        stage_tab_str,
        studio_hidden,
        modal_open,
        in_input,
        has_mods,
    ) {
        let in_radar = target
            .as_ref()
            .is_some_and(|el| el.closest(".omni-radar").ok().flatten().is_some());
        if in_radar
            && matches!(
                event.key().as_str(),
                "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown"
            )
        {
            event.prevent_default();
            let mut cam = load_camera();
            let step = 80.0;
            match event.key().as_str() {
                "ArrowLeft" => cam.pan_x += step,
                "ArrowRight" => cam.pan_x -= step,
                "ArrowUp" => cam.pan_y += step,
                "ArrowDown" => cam.pan_y -= step,
                _ => {}
            }
            cam.fit_mode = false;
            save_camera(&cam);
            apply_camera(&cam);
            return true;
        }
        let in_constellation = target
            .as_ref()
            .is_some_and(|el| el.closest("#omnigraph-canvas").ok().flatten().is_some());
        if in_constellation
            && matches!(
                event.key().as_str(),
                "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" | "Home" | "End"
            )
        {
            event.prevent_default();
            let curr_id = target
                .as_ref()
                .and_then(|el| el.closest(".omni-node[data-node-id]").ok().flatten())
                .and_then(|el| el.get_attribute("data-node-id"))
                .or_else(|| ui_state()["focused_id"].as_str().map(str::to_owned))
                .or_else(|| ui_state()["selected_id"].as_str().map(str::to_owned));
            let (ids, titles, rects, curr_idx, edges) = OMNI_DENSE_CACHE
                .with(|c| {
                    let g = c.borrow();
                    let entry = g.as_ref()?;
                    let ids: Vec<String> =
                        entry.node_rects.iter().map(|(id, ..)| id.clone()).collect();
                    let titles = entry.node_titles.clone();
                    let rects: Vec<(f64, f64, f64, f64)> = entry
                        .node_rects
                        .iter()
                        .map(|(_, x, y, w, h)| (*x, *y, *w, *h))
                        .collect();
                    let curr_idx = curr_id
                        .as_deref()
                        .and_then(|cid| entry.node_index_by_id.get(cid).copied())
                        .unwrap_or(0);
                    Some((ids, titles, rects, curr_idx, entry.node_edges.clone()))
                })
                .unwrap_or_else(|| {
                    let rects_val = m::array(&m::data("omnigraph-nav-rects"));
                    let edges_val = m::array(&m::data("omnigraph-nav-edges"));
                    let ids: Vec<String> = rects_val
                        .iter()
                        .map(|r| r["id"].as_str().unwrap_or("").to_owned())
                        .collect();
                    let titles = ids.clone();
                    let rects: Vec<(f64, f64, f64, f64)> = rects_val
                        .iter()
                        .map(|r| {
                            (
                                r["x"].as_f64().unwrap_or(0.0),
                                r["y"].as_f64().unwrap_or(0.0),
                                r["w"].as_f64().unwrap_or(200.0),
                                r["h"].as_f64().unwrap_or(64.0),
                            )
                        })
                        .collect();
                    let curr_idx = curr_id
                        .as_deref()
                        .and_then(|cid| ids.iter().position(|id| id == cid))
                        .unwrap_or(0);
                    let edges: Vec<(usize, usize)> = edges_val
                        .iter()
                        .filter_map(|pair| {
                            Some((pair[0].as_u64()? as usize, pair[1].as_u64()? as usize))
                        })
                        .collect();
                    (ids, titles, rects, curr_idx, edges)
                });
            let mut connected = Vec::new();
            for (u, v) in edges {
                if u == curr_idx {
                    connected.push(v);
                } else if v == curr_idx {
                    connected.push(u);
                }
            }
            if let Some(next_idx) =
                graph_viewport::next_roving_node_index(&rects, &connected, curr_idx, &event.key())
            {
                if let Some(next_id) = ids.get(next_idx).cloned() {
                    set_ui_state(|s| s["focused_id"] = json!(next_id));
                    OMNI_DENSE_CACHE.with(|c| {
                        if let Some(entry) = c.borrow_mut().as_mut() {
                            entry.latest_focused_id = Some(next_id.clone());
                        }
                    });
                    if let (Some(canvas), Some(plane)) = (
                        dom::get("omnigraph-canvas"),
                        dom::get("omnigraph-canvas")
                            .and_then(|c| c.query_selector(".omni-plane").ok().flatten()),
                    ) {
                        let mut cam = load_camera();
                        let (vw, vh, measured) = viewport_size_measured();
                        let next_cam = graph_viewport::dense_dom::apply_roving_node_focus(
                            &canvas,
                            &plane,
                            cam.to_viewport(),
                            if measured { (vw, vh) } else { (0.0, 0.0) },
                            &rects,
                            next_idx,
                            ".omni-node[data-node-id]",
                            28.0,
                            |el| {
                                el.get_attribute("data-node-id")
                                    .and_then(|id| ids.iter().position(|x| x == &id))
                            },
                            |idx| {
                                OMNI_DENSE_CACHE.with(|c| {
                                    let g = c.borrow();
                                    let entry = g.as_ref()?;
                                    entry
                                        .is_dense
                                        .then(|| entry.node_htmls.get(idx).cloned())
                                        .flatten()
                                        .filter(|s| !s.is_empty())
                                })
                            },
                            |idx, el| {
                                if let Some(nid) = ids.get(idx) {
                                    OMNI_DENSE_CACHE.with(|c| {
                                        if let Some(entry) = c.borrow().as_ref() {
                                            let roving_target = entry
                                                .latest_focused_id
                                                .as_deref()
                                                .or(entry.latest_selected_id.as_deref())
                                                .or_else(|| {
                                                    entry
                                                        .node_rects
                                                        .first()
                                                        .map(|(id, ..)| id.as_str())
                                                });
                                            patch_omni_node_el(
                                                el,
                                                nid,
                                                entry.latest_selected_id.as_deref(),
                                                &entry.latest_neighbors,
                                                &entry.latest_highlighted_ids,
                                                &entry.latest_path_nodes,
                                                roving_target,
                                            );
                                        }
                                    });
                                }
                            },
                        );
                        if let (Some(updated_cam), true) = (next_cam, measured) {
                            cam.scale = updated_cam.scale;
                            cam.pan_x = updated_cam.tx;
                            cam.pan_y = updated_cam.ty;
                            cam.fit_mode = false;
                            save_camera(&cam);
                            apply_camera(&cam);
                        }
                    }
                    let label = titles
                        .get(next_idx)
                        .filter(|s| !s.is_empty())
                        .cloned()
                        .unwrap_or_else(|| next_id.clone());
                    graph_viewport::announce_live(
                        "omnigraph-live-announce",
                        &format!("Focused {label}"),
                    );
                    return true;
                }
            }
        }
        match event.key().as_str() {
            "+" | "=" => {
                zoom_canvas_at(ZOOM_STEP, None);
                return true;
            }
            "-" => {
                zoom_canvas_at(1.0 / ZOOM_STEP, None);
                return true;
            }
            "0" => {
                fit_camera();
                return true;
            }
            "1" => {
                reset_camera_100();
                return true;
            }
            "Escape" => {
                let st = ui_state();
                if !m::array(&st["path_nodes"]).is_empty() || m::yes(&st["isolate_active"]) {
                    set_ui_state(|s| {
                        s["path_nodes"] = json!([]);
                        s["path_edges"] = json!([]);
                        s["isolate_active"] = json!(false);
                    });
                    render_constellation();
                    graph_viewport::announce_live(
                        "omnigraph-live-announce",
                        "Cleared path and isolation",
                    );
                    return true;
                }
                if st["selected_id"].as_str().is_some_and(|s| !s.is_empty()) {
                    set_ui_state(|s| s["selected_id"] = V::Null);
                    render_constellation();
                    render_inspector();
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

pub async fn event(
    kind: &str,
    id: &str,
    action: &str,
    payload: &V,
    value: &str,
    checked: bool,
) -> bool {
    if kind == "input" && id == "omnigraph-search" {
        let q = value.to_owned();
        dom::later("omnigraph-search-debounce", 220, move || {
            spawn_local(async move {
                run_search(q).await;
            });
        });
        return true;
    }
    if kind == "input" && id == "omni-import-text" {
        update_import_preview();
        return true;
    }
    if kind == "input" && (id == "omni-share-space" || id == "omni-share-filter") {
        render_share_dialog_state();
        return true;
    }

    if kind == "change" {
        match id {
            "omnigraph-project" => {
                crate::session::sync_project(value);
                on_project_changed();
                return true;
            }
            "omnigraph-branch" => {
                let _ = switch_branch_state(value, true);
                reset_pulse_cursor();
                m::set("omnigraph-pulse-events", json!([]));
                m::set("omnigraph-head-commit", V::Null);
                render_pulse();
                m::set("omnigraph-commit-detail", V::Null);
                refresh_graph().await;
                poll_changes(true).await;
                return true;
            }
            "omnigraph-show-sessions" => {
                set_ui_state(|s| s["show_sessions"] = json!(checked));
                render_constellation();
                return true;
            }
            "omnigraph-show-files" => {
                set_ui_state(|s| s["show_files"] = json!(checked));
                render_constellation();
                return true;
            }
            "omnigraph-search" => {
                run_search(value.to_owned()).await;
                return true;
            }
            "omni-isolate-depth" => {
                let depth = value.trim().parse::<u64>().unwrap_or(1).clamp(1, 3);
                let st = ui_state();
                let seed = st["isolate_seed"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .or_else(|| st["selected_id"].as_str().filter(|s| !s.is_empty()))
                    .map(str::to_owned)
                    .unwrap_or_default();
                if seed.is_empty() {
                    set_ui_state(|s| {
                        s["isolate_depth"] = json!(depth);
                        s["isolate_active"] = json!(false);
                    });
                    sync_controls_from_state();
                    graph_viewport::announce_live(
                        "omnigraph-live-announce",
                        "Select a node first to isolate its neighborhood",
                    );
                } else {
                    set_ui_state(|s| {
                        s["isolate_depth"] = json!(depth);
                        s["isolate_seed"] = json!(seed);
                        s["isolate_active"] = json!(true);
                    });
                    render_constellation();
                    graph_viewport::announce_live(
                        "omnigraph-live-announce",
                        &format!("Isolated {depth} hops"),
                    );
                }
                return true;
            }
            "omni-export-scope"
            | "omni-export-format"
            | "omni-export-session"
            | "omni-export-branch"
            | "omni-export-include-inherited" => {
                sync_export_scope_fields();
                run_export_preview().await;
                return true;
            }
            "omni-import-file" => {
                if let Some(input) = dom::get("omni-import-file")
                    .and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
                {
                    if let Some(files) = input.files() {
                        if let Some(file) = files.item(0) {
                            read_import_file(file).await;
                        }
                    }
                }
                return true;
            }
            "omni-import-text" | "omni-import-inject" => {
                update_import_preview();
                return true;
            }
            "omni-share-mode-shared"
            | "omni-share-mode-broadcast"
            | "omni-share-source"
            | "omni-share-space" => {
                render_share_dialog_state();
                return true;
            }
            _ if action == "omni-share-task-check" => {
                let mut set = share_selected_set();
                if checked {
                    set.insert(value.to_owned());
                } else {
                    set.remove(value);
                }
                set_share_selected_set(&set);
                render_share_dialog_state();
                return true;
            }
            _ => {}
        }
    }

    if kind == "submit" {
        match id {
            "omni-import-form" => {
                submit_import_form().await;
                return true;
            }
            "omni-share-form" => {
                submit_share_form().await;
                return true;
            }
            "omnigraph-connect-form" => {
                run_setup_action(
                    "connect",
                    Some(json!({
                        "endpoint": dom::value("omnigraph-ext-endpoint").trim(),
                        "token": dom::value("omnigraph-ext-token").trim(),
                        "graph_id": dom::value("omnigraph-ext-graph").trim(),
                    })),
                    "omnigraph-status-message",
                    "omnigraph-error",
                )
                .await;
                return true;
            }
            "omni-dlg-connect-form" => {
                run_setup_action(
                    "connect",
                    Some(json!({
                        "endpoint": dom::value("omni-dlg-endpoint").trim(),
                        "token": dom::value("omni-dlg-token").trim(),
                        "graph_id": dom::value("omni-dlg-graph").trim(),
                    })),
                    "omni-dlg-status",
                    "omni-dlg-error",
                )
                .await;
                return true;
            }
            "omni-create-branch-form" => {
                let name = dom::value("omni-new-branch-name").trim().to_owned();
                if name.is_empty() {
                    return true;
                }
                let from = dom::value("omni-new-branch-from");
                let project = current_project_id();
                match execute_branch_action(
                    json!({
                        "action": "create",
                        "project_id": project,
                        "name": name,
                        "from": if from.is_empty() { "main" } else { &from },
                    }),
                    &format!("Created branch {name}."),
                )
                .await
                {
                    Ok(_) => {
                        dom::set_value("omni-new-branch-name", "");
                        let _ = switch_branch_state(&name, true);
                        reset_pulse_cursor();
                        refresh_graph().await;
                    }
                    Err(e) => dom::error("omnigraph-error", &e.message),
                }
                return true;
            }
            "omni-publish-form" => {
                dom::error("omni-pub-error", "");
                dom::hidden("omni-pub-status", true);
                let title = dom::value("omni-pub-title").trim().to_owned();
                let summary = dom::value("omni-pub-summary").trim().to_owned();
                if title.is_empty() || summary.is_empty() {
                    dom::error("omni-pub-error", "Title and summary are required.");
                    return true;
                }
                let project = current_project_id();
                let branch = dom::value("omni-pub-branch");
                let session_id = dom::value("omni-pub-session");
                let confidence = dom::value("omni-pub-confidence")
                    .trim()
                    .parse::<f64>()
                    .unwrap_or(0.9)
                    .clamp(0.0, 1.0);
                let pin_priority = dom::value("omni-pub-pin")
                    .trim()
                    .parse::<i64>()
                    .unwrap_or(50)
                    .clamp(0, 100);
                let mut body = json!({
                    "project_id": project,
                    "branch": if branch.is_empty() { "main" } else { &branch },
                    "kind": dom::value("omni-pub-kind"),
                    "title": title,
                    "summary": summary,
                    "body": dom::value("omni-pub-body").trim(),
                    "tags": parse_csv_list(&dom::value("omni-pub-tags")),
                    "files": parse_csv_list(&dom::value("omni-pub-files")),
                    "relates_to": parse_relates_to(&dom::value("omni-pub-relates")),
                    "confidence": confidence,
                    "pin_priority": pin_priority,
                });
                if !session_id.is_empty() {
                    body["session_id"] = json!(session_id);
                }
                match api::post("/api/omnigraph/atoms", body).await {
                    Ok(res) => {
                        if res["ok"].as_bool() == Some(false) {
                            let msg = res["error"]
                                .as_str()
                                .filter(|s| !s.is_empty())
                                .unwrap_or("Failed to publish context atom.");
                            dom::error("omni-pub-error", msg);
                            return true;
                        }
                        let new_id = res["atom"]["atom_id"]
                            .as_str()
                            .or_else(|| res["atom"]["id"].as_str())
                            .or_else(|| res["atom_id"].as_str())
                            .or_else(|| res["id"].as_str())
                            .unwrap_or("")
                            .to_owned();
                        dom::set_value("omni-pub-title", "");
                        dom::set_value("omni-pub-summary", "");
                        dom::set_value("omni-pub-body", "");
                        dom::set_value("omni-pub-tags", "");
                        dom::set_value("omni-pub-files", "");
                        dom::set_value("omni-pub-relates", "");
                        dom::text("omni-pub-status", "Published context atom to Omnigraph.");
                        dom::hidden("omni-pub-status", false);
                        if !new_id.is_empty() {
                            set_ui_state(|s| {
                                s["selected_id"] = json!(new_id);
                                s["drawer_tab"] = json!("inspector");
                            });
                        }
                        refresh_pill().await;
                        refresh_graph().await;
                        if !new_id.is_empty() {
                            fly_to_node(&new_id);
                        }
                    }
                    Err(e) => dom::error("omni-pub-error", &e.message),
                }
                return true;
            }
            _ => {}
        }
    }

    if kind != "click" {
        return false;
    }

    if action == "omnigraph-select-node" && m::yes(&m::data("omnigraph-drag")["suppress_click"]) {
        return true;
    }

    match action {
        "omnigraph-install-start" | "omni-dlg-install" => {
            let status_el = if action == "omni-dlg-install" {
                "omni-dlg-status"
            } else {
                "omnigraph-status-message"
            };
            let err_el = if action == "omni-dlg-install" {
                "omni-dlg-error"
            } else {
                "omnigraph-error"
            };
            run_setup_action("install", None, status_el, err_el).await;
            true
        }
        "omnigraph-start-local" | "omni-dlg-start-local" => {
            let status_el = if action == "omni-dlg-start-local" {
                "omni-dlg-status"
            } else {
                "omnigraph-status-message"
            };
            let err_el = if action == "omni-dlg-start-local" {
                "omni-dlg-error"
            } else {
                "omnigraph-error"
            };
            run_setup_action("start_local", None, status_el, err_el).await;
            true
        }
        "omnigraph-start-docker" | "omni-dlg-start-docker" => {
            let status_el = if action == "omni-dlg-start-docker" {
                "omni-dlg-status"
            } else {
                "omnigraph-status-message"
            };
            let err_el = if action == "omni-dlg-start-docker" {
                "omni-dlg-error"
            } else {
                "omnigraph-error"
            };
            run_setup_action("start_docker", None, status_el, err_el).await;
            true
        }
        "omnigraph-apply-schema" | "omni-dlg-apply-schema" => {
            let status_el = if action == "omni-dlg-apply-schema" {
                "omni-dlg-status"
            } else {
                "omnigraph-status-message"
            };
            let err_el = if action == "omni-dlg-apply-schema" {
                "omni-dlg-error"
            } else {
                "omnigraph-error"
            };
            run_setup_action("apply_schema", None, status_el, err_el).await;
            true
        }
        "omnigraph-stop" | "omni-dlg-stop" => {
            let status_el = if action == "omni-dlg-stop" {
                "omni-dlg-status"
            } else {
                "omnigraph-status-message"
            };
            let err_el = if action == "omni-dlg-stop" {
                "omni-dlg-error"
            } else {
                "omnigraph-error"
            };
            run_setup_action("stop", None, status_el, err_el).await;
            true
        }
        "omnigraph-open-settings" => {
            let status = m::data("omnigraph-status");
            dom::set_value("omni-dlg-endpoint", &m::text(&status, "endpoint"));
            dom::set_value("omni-dlg-graph", &m::text(&status, "graph_id"));
            dom::hidden("omni-dlg-status", true);
            dom::error("omni-dlg-error", "");
            dom::dialog("omni-settings-dialog", true);
            true
        }
        "omni-settings-close" => {
            dom::dialog("omni-settings-dialog", false);
            true
        }
        "omnigraph-mcp-copy" => {
            let mcp = m::text(&m::data("omnigraph-status"), "mcp_url");
            if !mcp.is_empty() {
                let _ =
                    JsFuture::from(dom::window().navigator().clipboard().write_text(&mcp)).await;
                dom::text(
                    "omnigraph-status-message",
                    &format!("Copied MCP URL: {mcp}"),
                );
                dom::hidden("omnigraph-status-message", false);
            }
            true
        }
        "omnigraph-refresh-header" | "omni-refresh" => {
            load().await;
            true
        }
        "omni-sync-knowledge" => {
            let project = current_project_id();
            if project.is_empty() {
                dom::error("omnigraph-error", "Select a project first.");
                return true;
            }
            dom::error("omnigraph-error", "");
            dom::text(
                "omnigraph-status-message",
                "Syncing Project Brain & Memory Bank into Omnigraph…",
            );
            dom::hidden("omnigraph-status-message", false);
            dom::disabled("omni-sync-knowledge", true);
            let mut body = json!({ "project_id": project });
            if dom::value("brain-project") == project {
                let bank = dom::value("brain-bank");
                if !bank.is_empty() {
                    body["bank"] = json!(bank);
                }
            }
            if let Some(sid) =
                m::read(|s| matching_project_session_id(&s.selected, &s.sid, &project))
            {
                body["session_id"] = json!(sid);
            }
            match api::post("/api/omnigraph/sync-knowledge", body).await {
                Ok(res) => {
                    if res["ok"].as_bool() == Some(false) {
                        let msg = response_error_message(
                            &res,
                            "Could not sync Brain & Memory into Omnigraph.",
                        );
                        dom::hidden("omnigraph-status-message", true);
                        dom::error("omnigraph-error", &msg);
                    } else {
                        let branch = res["branch"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .unwrap_or(CANONICAL_KNOWLEDGE_BRANCH)
                            .to_owned();
                        let synced = res["synced"].as_u64().unwrap_or(0);
                        let brain_count = res["brain_count"].as_u64().unwrap_or(0);
                        let memory_count = res["memory_count"].as_u64().unwrap_or(0);
                        let skipped = res["skipped"].as_u64().unwrap_or(0);
                        let _ = switch_branch_state(&branch, true);
                        sync_controls_from_state();
                        refresh_pill().await;
                        refresh_graph().await;
                        dom::text(
                            "omnigraph-status-message",
                            &format!(
                                "Synced {synced} item(s) ({brain_count} Brain, {memory_count} Memory, {skipped} skipped) to {branch}."
                            ),
                        );
                        dom::hidden("omnigraph-status-message", false);
                    }
                }
                Err(e) => {
                    dom::hidden("omnigraph-status-message", true);
                    dom::error("omnigraph-error", &e.message);
                }
            }
            let connected = m::yes(&m::data("omnigraph-status")["connected"]);
            dom::disabled("omni-sync-knowledge", !connected);
            true
        }
        "omnigraph-search-clear" => {
            dom::set_value("omnigraph-search", "");
            run_search(String::new()).await;
            dom::focus("omnigraph-search");
            true
        }
        "omnigraph-clear-filters" => {
            dom::set_value("omnigraph-search", "");
            m::set("omnigraph-search-results", V::Null);
            let cur_branch = m::text(&ui_state(), "branch");
            if !cur_branch.is_empty() && cur_branch != "main" {
                let _ = switch_branch_state("main", false);
            }
            set_ui_state(|s| {
                s["branch"] = json!("main");
                s["query"] = json!("");
                s["kind"] = json!("");
                s["show_files"] = json!(true);
                s["show_sessions"] = json!(true);
                s["highlighted_ids"] = json!([]);
                s["path_nodes"] = json!([]);
                s["path_edges"] = json!([]);
                s["isolate_active"] = json!(false);
                s["isolate_seed"] = V::Null;
            });
            dom::hidden("omnigraph-search-clear", true);
            sync_controls_from_state();
            let mut cam = load_camera();
            cam.fit_mode = true;
            save_camera(&cam);
            refresh_graph().await;
            true
        }
        "omnigraph-open-publish" => {
            set_ui_state(|s| s["drawer_tab"] = json!("publish"));
            render_tabs();
            dom::focus("omni-pub-title");
            true
        }
        "omnigraph-filter-kind" => {
            let next_kind = payload["kind"]
                .as_str()
                .or_else(|| payload.as_str())
                .unwrap_or("");
            set_ui_state(|s| s["kind"] = json!(next_kind));
            sync_controls_from_state();
            refresh_graph().await;
            true
        }
        "omnigraph-stage-tab" => {
            let tab = payload["tab"]
                .as_str()
                .or_else(|| payload.as_str())
                .unwrap_or_else(|| id.strip_prefix("omni-tab-").unwrap_or("constellation"));
            set_ui_state(|s| s["stage_tab"] = json!(tab));
            render_tabs();
            if tab == "constellation" {
                render_constellation();
            } else if tab == "dag" {
                render_dag();
            } else if tab == "merge" {
                let src = dom::value("omni-merge-source");
                let dst = dom::value("omni-merge-target");
                if !src.is_empty() && !dst.is_empty() && !m::data("omnigraph-diff").is_object() {
                    preview_merge(&src, &dst).await;
                } else {
                    render_merge();
                }
            }
            true
        }
        "omnigraph-drawer-tab" => {
            let tab = payload["tab"]
                .as_str()
                .or_else(|| payload.as_str())
                .unwrap_or_else(|| id.strip_prefix("omni-drawer-tab-").unwrap_or("inspector"));
            set_ui_state(|s| s["drawer_tab"] = json!(tab));
            render_tabs();
            true
        }
        "omni-zoom-in" => {
            zoom_canvas_at(ZOOM_STEP, None);
            true
        }
        "omni-zoom-out" => {
            zoom_canvas_at(1.0 / ZOOM_STEP, None);
            true
        }
        "omni-zoom-level" => {
            reset_camera_100();
            true
        }
        "omni-fit" => {
            fit_camera();
            true
        }
        "omni-toggle-pulse" => {
            let paused = !m::yes(&ui_state()["pulse_paused"]);
            set_ui_state(|s| s["pulse_paused"] = json!(paused));
            sync_pulse_toggle();
            if paused {
                dom::cancel_timer("omnigraph-pulse");
            } else {
                schedule_pulse_poll();
            }
            true
        }
        "omnigraph-return-live" | "omni-return-live" => {
            set_ui_state(|s| {
                s["snapshot"] = V::Null;
                s["selected_commit_id"] = V::Null;
            });
            reset_pulse_cursor();
            m::set("omnigraph-commit-detail", V::Null);
            sync_controls_from_state();
            refresh_graph().await;
            true
        }
        "omnigraph-select-node" | "omnigraph-focus-hit" => {
            let node_id = payload["node_id"]
                .as_str()
                .or_else(|| payload.as_str())
                .unwrap_or("")
                .to_owned();
            if !node_id.is_empty() {
                if action == "omnigraph-focus-hit" {
                    let mut graph = m::data("omnigraph-graph");
                    if graph.is_object() {
                        let already_loaded = m::array(&graph["atoms"])
                            .iter()
                            .any(|a| atom_id(a) == node_id)
                            || m::array(&graph["files"])
                                .iter()
                                .any(|f| file_node_id(f) == node_id)
                            || m::array(&graph["sessions"])
                                .iter()
                                .any(|s| session_node_id(s) == node_id)
                            || m::array(&graph["symbols"])
                                .iter()
                                .any(|s| symbol_node_id(s) == node_id);
                        if !already_loaded {
                            if let Some(hit_atom) =
                                m::array(&m::data("omnigraph-search-results")["hits"])
                                    .into_iter()
                                    .map(|h| h["atom"].clone())
                                    .find(|a| atom_id(a) == node_id)
                            {
                                let mut atoms = m::array(&graph["atoms"]);
                                atoms.push(hit_atom);
                                graph["atoms"] = V::Array(atoms);
                                m::set("omnigraph-graph", graph);
                            }
                        }
                    }
                }
                let branch = m::text(&ui_state(), "branch");
                let cam = load_camera();
                OMNI_NAV_HISTORY.with(|h| {
                    h.borrow_mut()
                        .push(omni_nav_entry_from(Some(node_id.clone()), &branch, &cam));
                });
                set_ui_state(|s| {
                    s["selected_id"] = json!(node_id);
                    s["focused_id"] = json!(node_id);
                    s["drawer_tab"] = json!("inspector");
                    if action == "omnigraph-focus-hit" {
                        s["stage_tab"] = json!("constellation");
                    }
                });
                render_tabs();
                render_constellation();
                render_inspector();
                fly_to_node(&node_id);
            }
            true
        }
        "omnigraph-unpin-node" => {
            let node_id = payload["node_id"]
                .as_str()
                .or_else(|| payload.as_str())
                .unwrap_or("")
                .to_owned();
            if !node_id.is_empty() {
                let key = omni_pin_key();
                let mut pins = graph_viewport::load_pins(&key);
                if pins.remove(&node_id).is_some() {
                    graph_viewport::persist_pins_map(&key, &pins);
                    dom::hidden("omni-reset-pins", pins.is_empty());
                    render_constellation();
                    render_inspector();
                    graph_viewport::announce_live(
                        "omnigraph-live-announce",
                        &format!("Unpinned node {node_id}"),
                    );
                }
            }
            true
        }
        "omnigraph-history-back" | "omnigraph-history-forward" => {
            let back = action == "omnigraph-history-back";
            let target = OMNI_NAV_HISTORY.with(|h| {
                let mut hist = h.borrow_mut();
                if back {
                    hist.back().cloned()
                } else {
                    hist.forward().cloned()
                }
            });
            if let Some(entry) = target {
                let (selected_id, branch, cam) = parse_omni_nav_entry(&entry);
                let cur_branch = m::text(&ui_state(), "branch");
                if !branch.is_empty() && branch != cur_branch {
                    let _ = switch_branch_state(&branch, false);
                }
                set_ui_state(|s| {
                    s["selected_id"] = selected_id.map(|id| json!(id)).unwrap_or(V::Null);
                    s["focused_id"] = s["selected_id"].clone();
                    s["stage_tab"] = json!("constellation");
                });
                save_camera(&cam);
                sync_controls_from_state();
                render_tabs();
                render_constellation();
                render_inspector();
                apply_camera(&cam);
            }
            true
        }
        "inspect-file-graph" => {
            if m::read(|s| s.view != "omnigraph") {
                return false;
            }
            if let Some((path, fqn)) = resolve_code_map_target(payload) {
                let project = current_project_id();
                open_target_in_code_map(
                    (!project.is_empty()).then_some(project.as_str()),
                    &path,
                    fqn.as_deref(),
                )
                .await;
                return true;
            }
            false
        }
        "omnigraph-open-system-map" => {
            let node_id = payload["node_id"]
                .as_str()
                .or_else(|| payload.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| m::text(&ui_state(), "selected_id"));
            let graph = m::data("omnigraph-graph");
            let initial_match = resolve_system_service_target(
                &graph,
                &m::data("system-map-catalog-cache"),
                &node_id,
            );
            crate::session::open_view("systems", true).await;
            let matched = initial_match.or_else(|| {
                resolve_system_service_target(
                    &graph,
                    &m::data("system-map-catalog-cache"),
                    &node_id,
                )
            });
            if let Some(svc) = matched {
                crate::systems::switch_services_to_topology();
                crate::system_map::focus_service_topology(&svc);
            }
            crate::systems::focus_system_graph_workspace();
            true
        }
        "omnigraph-pulse-focus" => {
            let node_id = payload["node_id"].as_str().unwrap_or("").to_owned();
            let branch = payload["branch"].as_str().unwrap_or("").to_owned();
            if switch_branch_state(&branch, true) {
                reset_pulse_cursor();
            }
            set_ui_state(|s| {
                if !node_id.is_empty() {
                    s["selected_id"] = json!(node_id);
                }
                s["stage_tab"] = json!("constellation");
                s["drawer_tab"] = json!("inspector");
            });
            render_tabs();
            refresh_graph().await;
            if !node_id.is_empty() {
                fly_to_node(&node_id);
            }
            true
        }
        "omnigraph-open-git-commit" => {
            let sha = payload["sha"].as_str().unwrap_or("").trim().to_owned();
            let project = current_project_id();
            if is_git_sha(&sha) && !project.is_empty() {
                if dom::value("project") != project {
                    crate::session::sync_project(&project);
                }
                crate::graph_map::open_project_commit(&project, &sha);
            }
            true
        }
        "omnigraph-open-code-map" => {
            if let Some((path, fqn)) = resolve_code_map_target(payload) {
                let proj = payload["project_id"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(current_project_id);
                open_target_in_code_map(
                    (!proj.is_empty()).then_some(proj.as_str()),
                    &path,
                    fqn.as_deref(),
                )
                .await;
            }
            true
        }
        "omni-reset-pins" => {
            graph_viewport::clear_pins(&omni_pin_key());
            dom::hidden("omni-reset-pins", true);
            render_constellation();
            graph_viewport::announce_live("omnigraph-live-announce", "Reset pinned layout");
            true
        }
        "omni-isolate-toggle" => {
            let st = ui_state();
            let next_active = !m::yes(&st["isolate_active"]);
            let depth = dom::value("omni-isolate-depth")
                .trim()
                .parse::<u64>()
                .unwrap_or(1)
                .clamp(1, 3);
            let seed = st["selected_id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    if !next_active {
                        st["isolate_seed"].as_str().filter(|s| !s.is_empty())
                    } else {
                        None
                    }
                })
                .unwrap_or("")
                .to_owned();
            if next_active && seed.is_empty() {
                set_ui_state(|s| {
                    s["isolate_active"] = json!(false);
                    s["isolate_depth"] = json!(depth);
                });
                sync_controls_from_state();
                graph_viewport::announce_live(
                    "omnigraph-live-announce",
                    "Select a node first to isolate its neighborhood",
                );
                return true;
            }
            set_ui_state(|s| {
                s["isolate_active"] = json!(next_active);
                s["isolate_depth"] = json!(depth);
                if next_active {
                    s["isolate_seed"] = json!(seed);
                } else {
                    s["isolate_seed"] = V::Null;
                }
            });
            sync_controls_from_state();
            render_constellation();
            graph_viewport::announce_live(
                "omnigraph-live-announce",
                if next_active {
                    "Isolation enabled"
                } else {
                    "Isolation cleared"
                },
            );
            true
        }
        "omni-path-trace" => {
            let from_q = dom::value("omni-path-from");
            let to_q = dom::value("omni-path-to");
            let graph = m::data("omnigraph-graph");
            let st = ui_state();
            let paged_count = m::data("omnigraph-page-count").as_u64().unwrap_or(1).max(1) as usize;
            let cam = load_camera();
            let base_filter = current_constellation_filter(&st, &cam, paged_count);
            set_ui_state(|s| {
                s["path_from"] = json!(from_q.trim());
                s["path_to"] = json!(to_q.trim());
            });
            if let Some((path_nodes, path_edges)) =
                omnigraph_canvas::compute_constellation_path(&graph, &base_filter, &from_q, &to_q)
            {
                let edge_json: Vec<V> = path_edges.iter().map(|(a, b)| json!([a, b])).collect();
                let count = path_nodes.len();
                set_ui_state(|s| {
                    s["path_nodes"] = json!(path_nodes);
                    s["path_edges"] = V::Array(edge_json);
                });
                dom::hidden("omnigraph-status-message", true);
                render_constellation();
                graph_viewport::announce_live(
                    "omnigraph-live-announce",
                    &format!("Traced path with {count} nodes"),
                );
            } else {
                dom::text("omnigraph-status-message", "No path found between A and B.");
                dom::hidden("omnigraph-status-message", false);
                graph_viewport::announce_live(
                    "omnigraph-live-announce",
                    "No path found between A and B",
                );
            }
            true
        }
        "omni-path-clear" => {
            set_ui_state(|s| {
                s["path_from"] = json!("");
                s["path_to"] = json!("");
                s["path_nodes"] = json!([]);
                s["path_edges"] = json!([]);
            });
            dom::hidden("omnigraph-status-message", true);
            sync_controls_from_state();
            render_constellation();
            graph_viewport::announce_live("omnigraph-live-announce", "Cleared path trace");
            true
        }
        "omnigraph-load-more" => {
            load_more_graph().await;
            true
        }
        "omnigraph-select-commit" => {
            let commit_id = payload["commit_id"].as_str().unwrap_or("").to_owned();
            let branch = payload["branch"].as_str().unwrap_or("").to_owned();
            if !commit_id.is_empty() {
                set_ui_state(|s| s["selected_id"] = V::Null);
                load_commit_detail(&commit_id, &branch).await;
            }
            true
        }
        "omnigraph-preview-diff" => {
            let branch = payload["branch"].as_str().unwrap_or("").to_owned();
            if !branch.is_empty() {
                dom::set_value("omni-merge-source", &branch);
                dom::set_value("omni-merge-target", "main");
            }
            set_ui_state(|s| s["stage_tab"] = json!("merge"));
            render_tabs();
            let src = dom::value("omni-merge-source");
            let dst = dom::value("omni-merge-target");
            preview_merge(&src, &dst).await;
            true
        }
        "omnigraph-switch-branch" | "omnigraph-open-branch" => {
            let branch = payload["branch"].as_str().unwrap_or("").to_owned();
            if !branch.is_empty() {
                let _ = switch_branch_state(&branch, true);
                reset_pulse_cursor();
                m::set("omnigraph-commit-detail", V::Null);
                sync_controls_from_state();
                refresh_graph().await;
            }
            true
        }
        "omni-delete-branch" => {
            let branch = m::text(&ui_state(), "branch");
            if !branch.is_empty() && branch != "main" {
                let project = current_project_id();
                match execute_branch_action(
                    json!({
                        "action": "delete",
                        "project_id": project,
                        "name": branch,
                    }),
                    &format!("Deleted branch {branch}."),
                )
                .await
                {
                    Ok(_) => {
                        let _ = switch_branch_state("main", true);
                        reset_pulse_cursor();
                        refresh_graph().await;
                    }
                    Err(e) => dom::error("omnigraph-error", &e.message),
                }
            }
            true
        }
        "omni-merge-preview" => {
            preview_merge(
                &dom::value("omni-merge-source"),
                &dom::value("omni-merge-target"),
            )
            .await;
            true
        }
        "omni-merge-exec" | "omni-merge-delete" => {
            let src = dom::value("omni-merge-source");
            let dst = dom::value("omni-merge-target");
            let delete_branch = action == "omni-merge-delete";
            if src.is_empty() || dst.is_empty() {
                dom::error(
                    "omnigraph-error",
                    "Choose source and target branches to merge.",
                );
                return true;
            }
            let project = current_project_id();
            match execute_branch_action(
                json!({
                    "action": "merge",
                    "project_id": project,
                    "source": src,
                    "target": dst,
                    "dry_run": false,
                    "delete_branch": delete_branch,
                }),
                &format!("Merged {src} into {dst}."),
            )
            .await
            {
                Ok(mut res) => {
                    if let Err(msg) = branch_merge_outcome(&res) {
                        dom::error("omnigraph-error", &msg);
                        if !m::array(&res["conflicts"]).is_empty() || res["outcome"] == "conflict" {
                            if res["source"].is_null() {
                                res["source"] = json!(src);
                            }
                            if res["target"].is_null() {
                                res["target"] = json!(dst);
                            }
                            m::set("omnigraph-diff", res);
                            render_merge();
                        } else {
                            preview_merge(&src, &dst).await;
                        }
                    } else {
                        m::set("omnigraph-diff", res);
                        if delete_branch && m::text(&ui_state(), "branch") == src {
                            let _ = switch_branch_state(&dst, true);
                        }
                        render_merge();
                        refresh_graph().await;
                    }
                }
                Err(e) => {
                    dom::error("omnigraph-error", &e.message);
                    if e.status == 409 {
                        preview_merge(&src, &dst).await;
                    }
                }
            }
            true
        }
        "omnigraph-conflict-keep" => {
            keep_conflict_side(payload).await;
            true
        }
        "omnigraph-share-atom" => {
            dom::error("omni-share-error", "");
            dom::hidden("omni-share-status", true);
            let atom_id_str = payload["atom_id"].as_str().unwrap_or("").to_owned();
            let branch = payload["branch"].as_str().unwrap_or("main").to_owned();
            let target_session_id = dom::value("omni-share-session");
            let mode = dom::value("omni-share-mode");
            if atom_id_str.is_empty() || target_session_id.is_empty() {
                dom::error("omni-share-error", "Select a target session first.");
                return true;
            }
            let project = current_project_id();
            match api::post(
                "/api/omnigraph/share",
                json!({
                    "project_id": project,
                    "target_session_id": target_session_id,
                    "atom_ids": [atom_id_str],
                    "source_branch": branch,
                    "mode": if mode.is_empty() { "inject" } else { &mode },
                }),
            )
            .await
            {
                Ok(res) => {
                    if res["session"].is_object() {
                        m::upsert(res["session"].clone());
                        let updated_id = m::text(&res["session"], "id");
                        if !updated_id.is_empty()
                            && m::read(|s| m::text(&s.selected, "id")) == updated_id
                        {
                            render_session_bar(&res["session"]);
                        }
                    }
                    dom::text("omni-share-status", "Shared atom into session.");
                    dom::hidden("omni-share-status", false);
                    poll_changes(true).await;
                }
                Err(e) => dom::error("omni-share-error", &e.message),
            }
            true
        }
        "omnigraph-copy-snippet" => {
            let aid = payload["atom_id"].as_str().unwrap_or("");
            if let Some(atom) = m::array(&m::data("omnigraph-graph")["atoms"])
                .into_iter()
                .find(|a| atom_id(a) == aid)
            {
                let snippet = format_context_snippet(&atom);
                let _ = JsFuture::from(dom::window().navigator().clipboard().write_text(&snippet))
                    .await;
                dom::text("omni-share-status", "Copied context snippet to clipboard.");
                dom::hidden("omni-share-status", false);
            }
            true
        }
        "omnigraph-cite-atom" => {
            let aid = payload["atom_id"].as_str().unwrap_or("");
            if !aid.is_empty() {
                let cur = dom::value("omni-pub-relates");
                let mut list = parse_csv_list(&cur);
                if !list.iter().any(|x| {
                    x == aid
                        || x.split_once(':')
                            .is_some_and(|(_, target)| target.trim() == aid)
                }) {
                    list.push(aid.to_owned());
                }
                dom::set_value("omni-pub-relates", &list.join(", "));
                set_ui_state(|s| s["drawer_tab"] = json!("publish"));
                render_tabs();
                dom::focus("omni-pub-title");
            }
            true
        }
        "omnigraph-promote-atom" => {
            dom::error("omni-promote-error", "");
            dom::hidden("omni-promote-status", true);
            let atom_id_str = payload["atom_id"].as_str().unwrap_or("").trim().to_owned();
            let branch = payload["branch"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| m::text(&ui_state(), "branch"));
            let project = current_project_id();
            if project.is_empty() || atom_id_str.is_empty() {
                dom::error("omni-promote-error", "Select a project and atom first.");
                return true;
            }
            let mut body = json!({
                "project_id": project,
                "atom_id": atom_id_str,
            });
            if !branch.is_empty() {
                body["branch"] = json!(branch);
            }
            let kind_choice = dom::value("omni-promote-kind");
            if !kind_choice.is_empty() {
                body["kind"] = json!(kind_choice);
            }
            let privacy_choice = dom::value("omni-promote-privacy");
            if !privacy_choice.is_empty() {
                body["privacy"] = json!(privacy_choice);
            }
            if dom::value("brain-project") == project {
                let bank = dom::value("brain-bank");
                if !bank.is_empty() {
                    body["bank"] = json!(bank);
                }
            }
            if let Some(sid) =
                m::read(|s| matching_project_session_id(&s.selected, &s.sid, &project))
            {
                body["session_id"] = json!(sid);
            }
            dom::disabled("omni-promote-atom", true);
            dom::text("omni-promote-status", "Promoting to Project Brain…");
            dom::hidden("omni-promote-status", false);
            match api::post("/api/omnigraph/promote", body).await {
                Ok(res) => {
                    if res["ok"].as_bool() == Some(false) {
                        let msg = response_error_message(&res, "Could not promote atom to Brain.");
                        dom::hidden("omni-promote-status", true);
                        dom::error("omni-promote-error", &msg);
                        dom::disabled("omni-promote-atom", false);
                    } else {
                        let brain_id = m::text(&res, "brain_record_id");
                        let rec_type = m::text(&res, "record_type");
                        let rev = res["revision"].as_i64().unwrap_or(1);
                        let verified_count = m::array(&res["verified_files"]).len();
                        let status_msg =
                            format!(
                            "Promoted to Project Brain as {} ({}) · rev {} · {} verified file(s).",
                            if rec_type.is_empty() { "record" } else { &rec_type },
                            brain_id,
                            rev,
                            verified_count
                        );
                        refresh_pill().await;
                        refresh_graph().await;
                        if dom::value("brain-project") == project {
                            crate::knowledge::load("brain", false).await;
                        }
                        if dom::value("project") == project && m::read(|s| s.sid.is_empty()) {
                            crate::knowledge::load_brain_link().await;
                        }
                        if m::text(&ui_state(), "selected_id") == atom_id_str {
                            dom::text("omni-promote-status", &status_msg);
                            dom::hidden("omni-promote-status", false);
                        }
                        dom::text("omnigraph-status-message", &status_msg);
                        dom::hidden("omnigraph-status-message", false);
                    }
                }
                Err(e) => {
                    dom::hidden("omni-promote-status", true);
                    dom::error("omni-promote-error", &e.message);
                    dom::disabled("omni-promote-atom", false);
                }
            }
            true
        }
        "omnigraph-open-brain" => {
            let record_id = payload["record_id"]
                .as_str()
                .or_else(|| payload.as_str())
                .unwrap_or("")
                .trim()
                .to_owned();
            let project = current_project_id();
            if !record_id.is_empty() {
                crate::knowledge::open_from_omnigraph("brain", &project, &record_id).await;
            }
            true
        }
        "omnigraph-open-memory" => {
            let memory_id = payload["memory_id"]
                .as_str()
                .or_else(|| payload.as_str())
                .unwrap_or("")
                .trim()
                .to_owned();
            let project = current_project_id();
            if !memory_id.is_empty() {
                crate::knowledge::open_from_omnigraph("memory", &project, &memory_id).await;
            }
            true
        }
        "omni-pulse-poll" => {
            poll_changes(true).await;
            true
        }
        "omnigraph-session-atoms-toggle" => {
            let session = m::read(|s| s.selected.clone());
            if payload["expanded"] == true {
                m::set("omnigraph-session-atoms-expanded", session["id"].clone());
            } else {
                m::take("omnigraph-session-atoms-expanded");
            }
            render_session_bar(&session);
            // The toggle was drawn again; keep the keyboard on it.
            if let Some(toggle) = dom::all("#session-omnigraph-atoms .omni-session-atom-more")
                .into_iter()
                .next()
                .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
            {
                let _ = toggle.focus();
            }
            true
        }
        "omnigraph-open-atom" => {
            let aid = payload["atom_id"].as_str().unwrap_or("").to_owned();
            let branch = payload["branch"].as_str().unwrap_or("main").to_owned();
            let _ = switch_branch_state(&branch, true);
            set_ui_state(|s| {
                s["selected_id"] = json!(aid);
                s["stage_tab"] = json!("constellation");
                s["drawer_tab"] = json!("inspector");
            });
            crate::session::open_view("omnigraph", true).await;
            if !aid.is_empty() {
                fly_to_node(&aid);
            }
            true
        }
        "session-omnigraph-merge" => {
            let session = m::read(|s| s.selected.clone());
            let sid = m::text(&session, "id");
            let branch = session["omnigraph"]["branch"].as_str().unwrap_or("");
            let project = m::text(&session, "project_id");
            if branch.is_empty() || branch == "main" {
                return true;
            }
            dom::error("session-omnigraph-error", "");
            let mut body = json!({
                "action": "merge",
                "project_id": if project.is_empty() { current_project_id() } else { project },
                "source": branch,
                "target": "main",
                "dry_run": false,
                "delete_branch": false,
            });
            if !sid.is_empty() {
                body["session_id"] = json!(sid);
            }
            match api::post("/api/omnigraph/branches", body).await {
                Ok(res) => {
                    if let Err(msg) = branch_merge_outcome(&res) {
                        dom::error("session-omnigraph-error", &msg);
                    } else {
                        apply_merged_session_state(&sid, branch, &res);
                        let cur = m::read(|s| s.selected.clone());
                        if !sid.is_empty() && m::text(&cur, "id") == sid {
                            render_session_bar(&cur);
                        }
                        dom::text("session-omnigraph-meta", &format!("Merged {branch} → main"));
                        dom::hidden("session-omnigraph-merge", true);
                        refresh_pill().await;
                    }
                }
                Err(e) => dom::error("session-omnigraph-error", &e.message),
            }
            true
        }
        "session-omnigraph-export" => {
            let sid = m::read(|s| m::text(&s.selected, "id"));
            let branch = m::read(|s| m::omnigraph_branch(&s.selected));
            open_transfer_dialog(
                "export",
                Some(if sid.is_empty() { "branch" } else { "task" }),
                (!sid.is_empty()).then_some(sid.as_str()),
                Some(&branch),
            )
            .await;
            true
        }
        "session-omnigraph-import" => {
            let sid = m::read(|s| m::text(&s.selected, "id"));
            let branch = m::read(|s| m::omnigraph_branch(&s.selected));
            open_transfer_dialog(
                "import",
                None,
                (!sid.is_empty()).then_some(sid.as_str()),
                Some(&branch),
            )
            .await;
            true
        }
        "session-omnigraph-share-tasks" => {
            let sid = m::read(|s| m::text(&s.selected, "id"));
            open_share_dialog((!sid.is_empty()).then_some(sid.as_str())).await;
            true
        }
        "omnigraph-export-open" => {
            let branch = m::text(&ui_state(), "branch");
            open_transfer_dialog("export", Some("branch"), None, Some(&branch)).await;
            true
        }
        "omnigraph-import-open" => {
            let branch = m::text(&ui_state(), "branch");
            open_transfer_dialog("import", None, None, Some(&branch)).await;
            true
        }
        "omni-transfer-close" => {
            dom::dialog("omnigraph-transfer-dialog", false);
            true
        }
        "omnigraph-transfer-tab" => {
            let tab = payload["tab"]
                .as_str()
                .or_else(|| payload.as_str())
                .unwrap_or_else(|| id.strip_prefix("omni-transfer-tab-").unwrap_or("export"));
            set_transfer_tab(tab);
            if tab == "export" {
                run_export_preview().await;
            }
            true
        }
        "omni-export-preview-btn" => {
            run_export_preview().await;
            true
        }
        "omni-export-download" => {
            trigger_export_download().await;
            true
        }
        "omni-export-copy" => {
            copy_export_payload().await;
            true
        }
        "omni-import-browse" | "omni-import-dropzone" => {
            if let Some(el) =
                dom::get("omni-import-file").and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
            {
                el.click();
            }
            true
        }
        "omni-import-clear" => {
            dom::set_value("omni-import-text", "");
            dom::set_value("omni-import-file", "");
            dom::text("omni-import-file-name", "No file selected");
            dom::hidden("omni-import-status", true);
            dom::error("omni-import-error", "");
            update_import_preview();
            true
        }
        "omni-share-dialog-close" | "omni-share-cancel" | "omni-share-done" => {
            dom::dialog("omnigraph-share-dialog", false);
            true
        }
        "omni-share-open-setup" => {
            dom::dialog("omnigraph-share-dialog", false);
            crate::session::open_view("omnigraph", true).await;
            true
        }
        "omni-share-open-graph" => {
            let branch = m::string(&m::data("omnigraph-share-result-branch-val"));
            dom::dialog("omnigraph-share-dialog", false);
            if !branch.is_empty() {
                focus_branch(&branch);
            }
            crate::session::open_view("omnigraph", true).await;
            true
        }
        "omni-share-select-all" => {
            let needle = dom::value("omni-share-filter").trim().to_ascii_lowercase();
            let mut set = share_selected_set();
            for s in share_project_sessions() {
                if share_session_matches_filter(&s, &needle) {
                    set.insert(m::text(&s, "id"));
                }
            }
            set_share_selected_set(&set);
            render_share_dialog_state();
            true
        }
        "omni-share-clear" => {
            set_share_selected_set(&BTreeSet::new());
            render_share_dialog_state();
            true
        }
        _ => false,
    }
}
