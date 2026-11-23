use super::*;

pub(in crate::graph_map) fn blank_embedded_surface(surface: MapSurface, note_text: &str) {
    invalidate_surface_plan_cache(surface.canvas);
    graph_viewport::canvas2d::clear_scene(surface.canvas);
    invalidate_hover_cache(surface.canvas);
    LAST_DENSE_HOVER.with(|h| {
        h.borrow_mut().remove(surface.canvas);
    });
    let prefix = surface.id_prefix;
    let state = surface_state(surface);
    let changes = surface_changes_doc(surface, &state);
    sync_surface_toolbar(surface, &state, changes.as_ref());
    let crumbs_id = format!("{prefix}-crumbs");
    let legend_id = format!("{prefix}-arrow-legend");
    let summary_id = format!("{prefix}-summary");
    html(&crumbs_id, "");
    dom::hidden(&crumbs_id, true);
    html(surface.canvas, "");
    html(&legend_id, "");
    dom::hidden(&legend_id, true);
    html(
        surface.side,
        &surface_unindexed_side_html(surface, &state, changes.as_ref()),
    );
    m::take(surface.attachment_key);
    dom::text(&summary_id, "");
    dom::text(surface.note, note_text);
    dom::hidden(surface.note, note_text.is_empty());
}
pub(in crate::graph_map) fn render_embedded_surface(
    surface: MapSurface,
    view: &V,
    auto_focus_camera: bool,
    scope_changed: bool,
) {
    ensure_surface_watch(surface);
    let prefix = surface.id_prefix;
    let live = current_surface_live_focus(surface);
    render_surface_focus_strip(surface, live.as_ref());
    if !view.is_object() || !view["nodes"].is_array() {
        let view_state = m::text(view, "state");
        let fallback = if view_state == "failed" {
            format!(
                "Could not build code graph: {}",
                format_graph_status_text(&m::text(view, "failure"))
            )
        } else if view_state == "building" {
            let msg = format_graph_status_text(&m::text(&view["build"], "message"));
            if msg.is_empty() {
                "Building code graph and dependency map…".to_owned()
            } else {
                msg
            }
        } else {
            let note_text = format_graph_status_text(&note(view));
            if note_text.is_empty() {
                format_graph_status_text(&m::text(view, "message"))
            } else {
                note_text
            }
        };
        let default_note = if surface_scope(surface).is_system() {
            "Code graph not built yet for this service — click Build graph to index."
        } else {
            "Code graph will appear once indexed for this project."
        };
        blank_embedded_surface(
            surface,
            if fallback.is_empty() {
                default_note
            } else {
                &fallback
            },
        );
        let mut cam = load_surface_camera(surface);
        cam.plane_w = 0.0;
        cam.plane_h = 0.0;
        cam.bbox = None;
        save_surface_camera(surface, &cam);
        apply_surface_camera(surface, &cam);
        return;
    }
    let state = surface_state(surface);
    crate::graph_explanation::surface_map_context(
        surface.canvas,
        view,
        &state["cycle_base"]["revision"],
        &surface_scope(surface),
    );
    let changes_doc = surface_changes_doc(surface, &state);
    sync_surface_toolbar(surface, &state, changes_doc.as_ref());
    let impact_doc = cached_impact_for_selection(surface, &state, Some(view));
    let crumbs_id = format!("{prefix}-crumbs");
    let legend_id = format!("{prefix}-arrow-legend");
    let summary_id = format!("{prefix}-summary");
    let crumbs_html = crumbs(view);
    html(&crumbs_id, &crumbs_html);
    dom::hidden(&crumbs_id, crumbs_html.is_empty());
    dom::text(&summary_id, &summary(view));
    let before = interim(view, "");
    let note_text = combine_note(before, view);
    dom::text(surface.note, &note_text);
    dom::hidden(surface.note, note_text.is_empty());

    let surface_key = key_now(&state);
    let extras = build_overlay_extras(surface, view, &state);
    let overlay_ctx = CanvasOverlayContext {
        live_focus: live.as_ref(),
        changes: changes_doc.as_ref(),
        impact: impact_doc.as_ref(),
        links_off: surface_key.off,
        hide_transitive: surface_key.hide_transitive,
        cone: surface_key.cone,
        pinned_positions: Some(&extras.pins),
        path_nodes: Some(&extras.path_nodes),
        path_edges: Some(&extras.path_edges),
        isolate_cone: extras.isolate_cone.as_ref(),
        viewport_bounds: extras.viewport_bounds,
        roving_focus: extras.roving_focus,
    };
    let (plane_w, plane_h, focus_bbox, selected_bbox) =
        if let Some(patched) = try_patch_canvas_dom_in_place(surface.canvas, view, &overlay_ctx) {
            (
                patched.width,
                patched.height,
                patched.focus.bbox,
                patched.selected_bbox,
            )
        } else {
            let rendered = canvas_with_overlays(view, prefix, overlay_ctx.clone());
            store_surface_plan_cache(surface.canvas, view, &rendered, Some(&overlay_ctx));
            html(surface.canvas, &rendered.html);
            html(&format!("{}-radar", surface.canvas), &rendered.radar);
            invalidate_hover_cache(surface.canvas);
            if let Some(scene) = rendered.dense_scene {
                let meta = rendered.dense_meta.unwrap_or_default();
                graph_viewport::canvas2d::store_scene_with_meta(surface.canvas, scene, meta);
            } else {
                graph_viewport::canvas2d::clear_scene(surface.canvas);
            }
            (
                rendered.width,
                rendered.height,
                rendered.focus.bbox,
                rendered.selected_bbox,
            )
        };
    html(&legend_id, &key_html(view, &surface_key, surface.name));
    dom::hidden(&legend_id, false);
    let was_empty = dom::get(surface.side).is_none_or(|el| el.inner_html().is_empty());
    let side_html = build_surface_side_html(surface, view, &state);
    html(surface.side, &side_html);
    let now_empty = side_html.is_empty();
    let side_changed = was_empty != now_empty;
    let pan_target = if surface.pan_target_key.is_empty() {
        V::Null
    } else {
        m::take(surface.pan_target_key)
    };
    let mut cam = load_surface_camera(surface);
    cam.plane_w = plane_w;
    cam.plane_h = plane_h;
    cam.bbox = selected_bbox.or(focus_bbox);
    let (_, _, measured) = viewport_size(surface.canvas);
    cam.needs_measure = !measured || side_changed;
    let mut zoomed_bbox = false;
    if !pan_target.is_null() {
        if let Some(bbox) = selected_bbox {
            zoom_surface_camera_to_bbox(surface, &mut cam, bbox);
            zoomed_bbox = true;
        }
    }
    if !zoomed_bbox && auto_focus_camera && cam.follow {
        if let Some(bbox) = cam.bbox {
            zoom_surface_camera_to_bbox(surface, &mut cam, bbox);
            zoomed_bbox = true;
        }
    }
    if !zoomed_bbox {
        if cam.fit_mode || scope_changed || side_changed {
            fit_surface_camera(surface, &mut cam);
        } else {
            save_surface_camera(surface, &cam);
            apply_surface_camera(surface, &cam);
        }
    }
    let settled_cam = load_surface_camera(surface);
    record_surface_history(surface, &state, view, &settled_cam);
    let crumbs_id = format!("{prefix}-crumbs");
    let trail_html = surface_history_trail_html(surface);
    let mut crumbs_out = crumbs(view);
    if !trail_html.is_empty() {
        crumbs_out.push_str(&trail_html);
    }
    html(&crumbs_id, &crumbs_out);
    dom::hidden(&crumbs_id, crumbs_out.trim().is_empty());
    sync_history_and_pin_controls(surface, &state);
    if side_changed {
        cam.needs_measure = true;
        save_surface_camera(surface, &cam);
        dom::later(surface.refit_timer_key, 60, move || {
            refit_surface_if_visible(surface);
        });
    }
}
pub(in crate::graph_map) fn redraw_surface_overlay(surface: MapSurface, auto_focus_camera: bool) {
    let view = m::data(surface.view_key);
    if view["nodes"].is_array() {
        render_embedded_surface(surface, &view, auto_focus_camera, false);
    } else {
        render_surface_focus_strip(surface, current_surface_live_focus(surface).as_ref());
    }
}
pub(in crate::graph_map) fn embedded_error_is_unbuilt(
    graph_status: &V,
    error_message: &str,
) -> bool {
    let state = m::text(graph_status, "state");
    if !graph_status["graph"].is_object()
        && matches!(state.as_str(), "building" | "empty" | "missing" | "off")
    {
        return true;
    }
    let lower = error_message.to_ascii_lowercase();
    lower.contains("no published")
        || lower.contains("not built")
        || lower.contains("not indexed")
        || lower.contains("building")
        || lower.contains("not enabled")
        || lower.contains("turned off")
}
#[cfg(test)]
pub(in crate::graph_map) fn embedded_unbuilt_note(graph_status: &V) -> &'static str {
    embedded_unbuilt_note_for_scope(graph_status, false)
}
pub(in crate::graph_map) fn embedded_unbuilt_note_for_scope(
    graph_status: &V,
    is_system: bool,
) -> &'static str {
    match m::text(graph_status, "state").as_str() {
        "building" => "Building code graph and dependency map…",
        "off" if is_system => "Code graph is turned off for this service.",
        "off" => "Code graph is turned off for this project.",
        _ if is_system => "Code graph not built yet for this service — click Build graph to index.",
        _ => "No published code graph yet for this project.",
    }
}
pub(in crate::graph_map) async fn load_embedded_surface(surface: MapSurface, quiet: bool) {
    let state = surface_state(surface);
    let scope = surface_scope(surface);
    let poll_key = format!("{}-poll", surface.id_prefix);
    if scope.is_empty() {
        dom::cancel_timer(&poll_key);
        m::next(surface.changes_epoch);
        m::next(surface.impact_epoch);
        m::take(surface.changes_key);
        m::take(surface.impact_key);
        if !surface.pan_target_key.is_empty() {
            m::take(surface.pan_target_key);
        }
        m::set(surface.view_key, V::Null);
        dom::error(surface.error_id, "");
        blank_embedded_surface(
            surface,
            if surface.name == "session" {
                "Select a project to view its code graph."
            } else {
                "Select a system service to view its code graph."
            },
        );
        return;
    }
    let scope_key = scope.scope_key();
    let target = effective_target(&state);
    let prev_scope = m::text(&m::data(surface.view_key), "scope");
    let path = super::state_urls::build_surface_map_path(surface, &scope, &state);
    if !quiet {
        dom::attr(surface.canvas, "aria-busy", "true");
    }
    if !quiet || surface_changes_doc(surface, &state).is_none() {
        let ch_state = state.clone();
        spawn_local(async move {
            fetch_surface_changes(surface, ch_state).await;
        });
    }
    match api::scoped(surface.map_epoch, &path).await {
        Ok((epoch, view)) => {
            if !m::current(surface.map_epoch, epoch)
                || surface_scope(surface).scope_key() != scope_key
            {
                return;
            }
            if surface.name != "session" {
                crate::graph::sync_embedded_progress_for_surface(surface.id_prefix, &view);
                if view["state"] == "building" {
                    dom::later(&poll_key, 1200, move || {
                        let active_view = m::read(|s| s.view.clone());
                        let visible = (surface.name == "system-member" && active_view == "systems")
                            || (surface.name == "system-change" && active_view == "system-changes");
                        if visible && !surface_scope(surface).is_empty() {
                            spawn_local(load_embedded_surface(surface, true));
                        }
                    });
                } else {
                    dom::cancel_timer(&poll_key);
                }
            }
            let (next_state, _) = reconcile_view_state(
                state,
                &view,
                json!(load_surface_camera(surface).fit_mode),
                &target,
            );
            save_surface_state(surface, &next_state);
            dom::error(surface.error_id, "");
            dom::hidden(&format!("{}-stale-banner", surface.id_prefix), true);
            let next_scope = m::text(&view, "scope");
            let scope_changed = !prev_scope.is_empty() && prev_scope != next_scope;
            let mut auto_focus = false;
            if view["graph_focus"].is_object() && !surface.focus_key.is_empty() {
                let incoming_phase = m::text(&view["graph_focus"], "phase");
                let current = m::data(surface.focus_key);
                let keep_predicting =
                    m::text(&current, "phase") == "predicting" && incoming_phase == "idle";
                if !keep_predicting && current != view["graph_focus"] {
                    m::set(surface.focus_key, view["graph_focus"].clone());
                    auto_focus = true;
                }
            }
            m::set(surface.view_key, view.clone());
            render_embedded_surface(surface, &view, auto_focus, scope_changed);
            ensure_surface_impact(surface, &next_state, Some(&view), false);
        }
        Err(error) if error.aborted => return,
        Err(error) => {
            dom::cancel_timer(&poll_key);
            let graph_status = if surface.name == "session" {
                m::data("graph")
            } else {
                V::Null
            };
            if embedded_error_is_unbuilt(&graph_status, &error.message) {
                dom::error(surface.error_id, "");
                blank_embedded_surface(
                    surface,
                    embedded_unbuilt_note_for_scope(&graph_status, scope.is_system()),
                );
            } else {
                dom::error(surface.error_id, &error.message);
                if !m::data(surface.view_key)["nodes"].is_array() {
                    blank_embedded_surface(surface, "");
                }
            }
        }
    }
    dom::attr(surface.canvas, "aria-busy", "false");
}
/// Refreshes the inline Sessions graph `/graph/changes` catalog and toolbar badge.
pub fn sync_embedded_changes() {
    let project = dom::value("project");
    let state = surface_state(SESSION_SURFACE);
    if project.is_empty() {
        m::next(SESSION_SURFACE.changes_epoch);
        m::take(SESSION_SURFACE.changes_key);
        sync_surface_toolbar(SESSION_SURFACE, &state, None);
        return;
    }
    spawn_local(async move {
        fetch_surface_changes(SESSION_SURFACE, state).await;
    });
}
/// Syncs the inline Sessions dependency map for the active project, fetching `/graph/map` when needed.
pub fn sync_embedded(force: bool) {
    ensure_surface_watch(SESSION_SURFACE);
    let project = dom::value("project");
    let current = surface_state(SESSION_SURFACE);
    if project.is_empty() {
        m::next("session-graph");
        m::next(SESSION_SURFACE.changes_epoch);
        m::next(SESSION_SURFACE.impact_epoch);
        let reset = fresh("");
        save_surface_state(SESSION_SURFACE, &reset);
        m::set(SESSION_SURFACE.view_key, V::Null);
        m::set(SESSION_SURFACE.focus_key, V::Null);
        m::take(SESSION_SURFACE.changes_key);
        m::take(SESSION_SURFACE.impact_key);
        m::take(SESSION_SURFACE.pan_target_key);
        render_surface_focus_strip(SESSION_SURFACE, None);
        dom::error(SESSION_SURFACE.error_id, "");
        blank_embedded_surface(SESSION_SURFACE, "Select a project to view its code graph.");
        dom::attr(SESSION_SURFACE.canvas, "aria-busy", "false");
        return;
    }
    let project_changed = m::text(&current, "project") != project;
    if project_changed {
        m::next(SESSION_SURFACE.changes_epoch);
        m::next(SESSION_SURFACE.impact_epoch);
        m::take(SESSION_SURFACE.changes_key);
        m::take(SESSION_SURFACE.impact_key);
        m::take(SESSION_SURFACE.pan_target_key);
        let mut next = fresh("");
        next["project"] = json!(project);
        save_surface_state(SESSION_SURFACE, &next);
        m::set(SESSION_SURFACE.view_key, V::Null);
        m::set(SESSION_SURFACE.focus_key, V::Null);
        let mut cam = load_surface_camera(SESSION_SURFACE);
        cam.fit_mode = true;
        cam.needs_measure = true;
        cam.bbox = None;
        cam.plane_w = 0.0;
        cam.plane_h = 0.0;
        save_surface_camera(SESSION_SURFACE, &cam);
        render_surface_focus_strip(SESSION_SURFACE, None);
        dom::error(SESSION_SURFACE.error_id, "");
        blank_embedded_surface(SESSION_SURFACE, "Loading code graph…");
        apply_surface_camera(SESSION_SURFACE, &cam);
    }
    let loaded_view = m::data(SESSION_SURFACE.view_key);
    let has_rendered = dom::get(SESSION_SURFACE.canvas).is_some_and(|el| {
        el.query_selector(".graph-map-plane")
            .ok()
            .flatten()
            .is_some()
    });
    if !force && !project_changed && loaded_view["nodes"].is_array() && has_rendered {
        sync_surface_toolbar(
            SESSION_SURFACE,
            &current,
            surface_changes_doc(SESSION_SURFACE, &current).as_ref(),
        );
        if surface_changes_doc(SESSION_SURFACE, &current).is_none() {
            sync_embedded_changes();
        }
        refit_surface_if_visible(SESSION_SURFACE);
        return;
    }
    let quiet = loaded_view["nodes"].is_array() && !project_changed;
    spawn_local(async move {
        load_embedded_surface(SESSION_SURFACE, quiet).await;
    });
}
/// Syncs/loads an embedded System code-graph surface (`Member` in `#systems-view` or `Change` in `#system-changes-view`).
#[allow(dead_code)]
pub(crate) fn load_system_embedded(
    kind: SystemEmbeddedKind,
    scope: Option<GraphTargetScope>,
    force: bool,
) {
    let surface = kind.surface();
    ensure_surface_watch(surface);
    let poll_key = format!("{}-poll", surface.id_prefix);
    let Some(next_scope) = scope.filter(|s| !s.is_empty()) else {
        dom::cancel_timer(&poll_key);
        set_surface_scope(surface, GraphTargetScope::single(""));
        m::next(surface.map_epoch);
        m::next(surface.changes_epoch);
        m::next(surface.impact_epoch);
        let reset = fresh("");
        save_surface_state(surface, &reset);
        m::set(surface.view_key, V::Null);
        m::set(surface.focus_key, V::Null);
        m::take(surface.changes_key);
        m::take(surface.impact_key);
        m::take(surface.pan_target_key);
        render_surface_focus_strip(surface, None);
        dom::error(surface.error_id, "");
        blank_embedded_surface(surface, "Select a system service to view its code graph.");
        dom::attr(surface.canvas, "aria-busy", "false");
        return;
    };
    let prev_scope = surface_scope(surface);
    let scope_changed = prev_scope.scope_key() != next_scope.scope_key();
    let incoming_run_id = match &next_scope {
        GraphTargetScope::SystemService { run_id, .. } => {
            run_id.as_deref().map(str::trim).unwrap_or("").to_owned()
        }
        GraphTargetScope::SingleProject { .. } => String::new(),
    };
    set_surface_scope(surface, next_scope);
    let mut current = surface_state(surface);
    let mut run_changed = false;
    if scope_changed {
        dom::cancel_timer(&poll_key);
        m::next(surface.map_epoch);
        m::next(surface.changes_epoch);
        m::next(surface.impact_epoch);
        m::take(surface.changes_key);
        m::take(surface.impact_key);
        m::take(surface.pan_target_key);
        let default_target =
            if matches!(kind, SystemEmbeddedKind::Change) && !incoming_run_id.is_empty() {
                format!("sysrun:{incoming_run_id}")
            } else {
                String::new()
            };
        let mut next_state = fresh(&default_target);
        if !incoming_run_id.is_empty() {
            next_state["run_id"] = json!(incoming_run_id);
        }
        if matches!(kind, SystemEmbeddedKind::Change) && !default_target.is_empty() {
            next_state["side_tab"] = json!("changes");
            next_state["side_pinned"] = json!(false);
        }
        save_surface_state(surface, &next_state);
        current = next_state;
        m::set(surface.view_key, V::Null);
        m::set(surface.focus_key, V::Null);
        let mut cam = load_surface_camera(surface);
        cam.fit_mode = true;
        cam.needs_measure = true;
        cam.bbox = None;
        cam.plane_w = 0.0;
        cam.plane_h = 0.0;
        save_surface_camera(surface, &cam);
        render_surface_focus_strip(surface, None);
        dom::error(surface.error_id, "");
        blank_embedded_surface(surface, "Loading service code graph…");
        apply_surface_camera(surface, &cam);
    } else if matches!(kind, SystemEmbeddedKind::Change)
        && m::text(&current, "run_id") != incoming_run_id
    {
        run_changed = true;
        current["run_id"] = json!(incoming_run_id.clone());
        let cur_target = m::text(&current, "target");
        if cur_target.is_empty() || cur_target.starts_with("sysrun:") {
            current["target"] = json!(if incoming_run_id.is_empty() {
                String::new()
            } else {
                format!("sysrun:{incoming_run_id}")
            });
        }
        m::next(surface.changes_epoch);
        m::next(surface.impact_epoch);
        m::take(surface.changes_key);
        m::take(surface.impact_key);
        save_surface_state(surface, &current);
    }
    let loaded_view = m::data(surface.view_key);
    let has_rendered = dom::get(surface.canvas).is_some_and(|el| {
        el.query_selector(".graph-map-plane")
            .ok()
            .flatten()
            .is_some()
    });
    if !force && !scope_changed && !run_changed && loaded_view["nodes"].is_array() && has_rendered {
        sync_surface_toolbar(
            surface,
            &current,
            surface_changes_doc(surface, &current).as_ref(),
        );
        if surface_changes_doc(surface, &current).is_none() {
            let ch_state = current;
            spawn_local(async move {
                fetch_surface_changes(surface, ch_state).await;
            });
        }
        refit_surface_if_visible(surface);
        return;
    }
    let quiet = loaded_view["nodes"].is_array() && !scope_changed && !run_changed;
    spawn_local(async move {
        load_embedded_surface(surface, quiet).await;
    });
}
pub(in crate::graph_map) fn focus_surface_from_prompt(surface: MapSurface, prompt: &str) {
    let view = m::data(surface.view_key);
    let Some(matched) = match_prompt_targets(&view, prompt) else {
        return;
    };
    let focus_json = json!({
        "phase": matched.phase,
        "summary": matched.summary,
        "target_folders": matched.target_folders,
        "target_paths": matched.target_paths,
        "target_nodes": matched.target_nodes,
        "created_entities": [],
    });
    if m::data(surface.focus_key) == focus_json {
        return;
    }
    m::set(surface.focus_key, focus_json);
    redraw_surface_overlay(surface, true);
}
/// Immediately highlights and zooms onto graph nodes matching `prompt` on Send (0ms local match).
pub fn focus_from_prompt(prompt: &str) {
    focus_surface_from_prompt(SESSION_SURFACE, prompt);
}
/// Immediately highlights and zooms onto graph nodes matching `prompt` on an embedded System surface.
#[allow(dead_code)]
pub(crate) fn focus_system_from_prompt(kind: SystemEmbeddedKind, prompt: &str) {
    focus_surface_from_prompt(kind.surface(), prompt);
}
pub(in crate::graph_map) fn apply_surface_live_focus(surface: MapSurface, focus: &V) {
    let parsed = parse_live_focus(focus);
    let next_val = if parsed.is_some() {
        focus.clone()
    } else {
        V::Null
    };
    if m::data(surface.focus_key) == next_val {
        return;
    }
    m::set(surface.focus_key, next_val);
    redraw_surface_overlay(surface, parsed.is_some());
}
/// Applies a backend `GraphFocus` update (`Value::Null` clears) to the Sessions map & camera.
pub fn apply_focus(focus: &V) {
    apply_surface_live_focus(SESSION_SURFACE, focus);
}
/// Applies a backend `GraphFocus` update (`Value::Null` clears) to an embedded System code-graph surface.
#[allow(dead_code)]
pub(crate) fn apply_system_live_focus(kind: SystemEmbeddedKind, focus: &V) {
    apply_surface_live_focus(kind.surface(), focus);
}
pub(in crate::graph_map) fn jump_to_surface_entity_or_target(
    surface: MapSurface,
    payload: &V,
) -> bool {
    let raw_path = m::text(payload, "path");
    let raw_parent = m::text(payload, "parent");
    let raw_folder = m::text(payload, "folder");
    let entity_file = normalize_rel_path(strip_symbol_suffix(if !raw_path.is_empty() {
        &raw_path
    } else if looks_like_file_path(&raw_parent) {
        &raw_parent
    } else {
        ""
    }));
    let is_entity_item = !m::text(payload, "id").is_empty()
        || (!m::text(payload, "kind").is_empty() && m::text(payload, "kind") != "target");
    if !entity_file.is_empty() && (is_entity_item || looks_like_file_path(&entity_file)) {
        return focus_file_on_surface(surface, &entity_file);
    }

    let target_x = payload["x"].as_f64();
    let target_y = payload["y"].as_f64();
    let view = m::data(surface.view_key);
    if let (Some(x), Some(y)) = (target_x, target_y) {
        let mut cam = load_surface_camera(surface);
        zoom_surface_camera_to_bbox(
            surface,
            &mut cam,
            (x, y, x + ENTITY_WIDTH, y + ENTITY_HEIGHT),
        );
        return true;
    }
    if view["nodes"].is_array() {
        let mut probe_targets = Vec::new();
        for item in [&raw_path, &raw_parent, &raw_folder] {
            let norm = normalize_rel_path(item);
            if !norm.is_empty() && !probe_targets.contains(&norm) {
                probe_targets.push(norm);
            }
        }
        let probe = LiveFocus {
            phase: "active".to_owned(),
            summary: String::new(),
            target_folders: if raw_folder.is_empty() {
                Vec::new()
            } else {
                vec![normalize_rel_path(&raw_folder)]
            },
            target_paths: probe_targets.clone(),
            target_nodes: probe_targets,
            created_entities: Vec::new(),
        };
        let cached_bbox = SURFACE_PLAN_CACHE.with(|cache| {
            let borrowed = cache.borrow();
            resolve_focus_bbox_with_cached_plan(&view, borrowed.get(surface.canvas), &probe)
        });
        if let Some(bbox) = cached_bbox {
            let mut cam = load_surface_camera(surface);
            zoom_surface_camera_to_bbox(surface, &mut cam, bbox);
            return true;
        }
    }
    let next_scope = if !raw_folder.is_empty() {
        raw_folder
    } else if !raw_parent.is_empty() {
        normalize_rel_path(strip_symbol_suffix(&raw_parent))
    } else {
        normalize_rel_path(parent(strip_symbol_suffix(&raw_path)))
    };
    if !next_scope.is_empty() {
        let mut state = surface_state(surface);
        state = unfollowed(state);
        state["scope"] = json!(next_scope);
        state["focus"] = json!("");
        state["filter"] = json!("");
        save_surface_state(surface, &state);
        spawn_local(async move {
            load_embedded_surface(surface, false).await;
        });
        return true;
    }
    false
}
pub(in crate::graph_map) fn focus_file_on_surface(surface: MapSurface, raw_path: &str) -> bool {
    let file_path = normalize_rel_path(strip_symbol_suffix(raw_path));
    if file_path.is_empty() {
        return false;
    }
    let view = m::data(surface.view_key);
    let (next_state, needs_reload) = state_focus_file(surface_state(surface), &file_path, &view);
    save_surface_state(surface, &next_state);
    if !surface.pan_target_key.is_empty() {
        m::set(surface.pan_target_key, json!(file_path));
    }
    dom::set_value(
        &format!("{}-filter", surface.id_prefix),
        &m::text(&next_state, "filter"),
    );
    sync_surface_toolbar(
        surface,
        &next_state,
        surface_changes_doc(surface, &next_state).as_ref(),
    );
    let view_opt = view["nodes"].is_array().then_some(&view);
    ensure_surface_impact(surface, &next_state, view_opt, true);
    if needs_reload {
        spawn_local(async move {
            load_embedded_surface(surface, false).await;
        });
    } else {
        rerender_surface_preserving_camera(surface);
    }
    true
}
/// Applies cross-graph `{filter, target_commit, semantic_kind}` context from Omnigraph
/// onto a Code Map surface state without discarding explicit symbol/file focus.
pub(crate) fn apply_code_map_jump_context(
    st: &mut V,
    filter: Option<&str>,
    target_commit: Option<&str>,
    semantic_kind: Option<&str>,
    short_sym: Option<&str>,
) {
    if let Some(commit) = target_commit.map(str::trim).filter(|s| !s.is_empty()) {
        let valid_sha = commit.len() >= 7 && commit.bytes().all(|b| b.is_ascii_hexdigit());
        if valid_sha {
            st["target"] = json!(format!("commit:{commit}"));
        } else if commit.starts_with("commit:") || commit.starts_with("range:") {
            st["target"] = json!(commit);
        }
    }
    if let Some(sk) = semantic_kind.map(str::trim).filter(|s| !s.is_empty()) {
        st["semantic_kind"] = json!(sk);
    }
    if let Some(raw_filter) = filter.map(str::trim).filter(|s| !s.is_empty()) {
        st["omnigraph_filter"] = json!(raw_filter);
        if short_sym.is_none_or(|s| s.trim().is_empty()) {
            st["filter"] = json!(raw_filter);
        }
    }
}

/// Focuses a workspace-relative file path and optional symbol FQN on a map surface.
pub(in crate::graph_map) fn focus_target_on_surface(
    surface: MapSurface,
    path: &str,
    symbol_fqn: Option<&str>,
) -> bool {
    focus_target_on_surface_with_context(surface, path, symbol_fqn, None, None, None)
}

pub(in crate::graph_map) fn focus_target_on_surface_with_context(
    surface: MapSurface,
    path: &str,
    symbol_fqn: Option<&str>,
    filter: Option<&str>,
    target_commit: Option<&str>,
    semantic_kind: Option<&str>,
) -> bool {
    let clean_path = normalize_rel_path(strip_symbol_suffix(path));
    if let Some(fqn) = symbol_fqn.map(str::trim).filter(|s| !s.is_empty()) {
        let mut st = surface_state(surface);
        let short_sym = fqn.rsplit("::").next().unwrap_or(fqn).trim();
        if !clean_path.is_empty() {
            st["scope"] = json!(clean_path);
            st["focus"] = json!(fqn);
        } else {
            st["focus"] = json!(fqn);
        }
        if !short_sym.is_empty() {
            st["filter"] = json!(short_sym);
        }
        apply_code_map_jump_context(
            &mut st,
            filter,
            target_commit,
            semantic_kind,
            Some(short_sym),
        );
        if !surface.pan_target_key.is_empty() {
            m::set(surface.pan_target_key, json!(fqn));
        }
        save_surface_state(surface, &st);
        dom::set_value(
            &format!("{}-filter", surface.id_prefix),
            &m::text(&st, "filter"),
        );
        let view = m::data(surface.view_key);
        let needs_reload = clean_path.is_empty()
            || view["mode"] != "file"
            || normalize_rel_path(&m::text(&view, "scope")) != clean_path
            || target_commit.is_some_and(|c| !c.trim().is_empty());
        let view_opt = view["nodes"].is_array().then_some(&view);
        ensure_surface_impact(surface, &st, view_opt, true);
        if needs_reload {
            spawn_local(async move {
                load_embedded_surface(surface, false).await;
            });
        } else {
            rerender_surface_preserving_camera(surface);
        }
        return true;
    }
    let has_ctx = filter.is_some_and(|s| !s.trim().is_empty())
        || target_commit.is_some_and(|s| !s.trim().is_empty())
        || semantic_kind.is_some_and(|s| !s.trim().is_empty());
    if has_ctx {
        let mut st = surface_state(surface);
        apply_code_map_jump_context(&mut st, filter, target_commit, semantic_kind, None);
        save_surface_state(surface, &st);
        dom::set_value(
            &format!("{}-filter", surface.id_prefix),
            &m::text(&st, "filter"),
        );
    }
    focus_file_on_surface(surface, path)
}

/// Focuses a workspace-relative file path on the embedded Sessions dependency map.
pub fn focus_file_in_session(path: &str) {
    focus_target_in_session(path, None);
}

/// Focuses a workspace-relative file path and optional symbol FQN on the Sessions dependency map.
pub(crate) fn focus_target_in_session(path: &str, symbol_fqn: Option<&str>) {
    let _ = focus_target_on_surface(SESSION_SURFACE, path, symbol_fqn);
}

/// Focuses a target on the Sessions dependency map while preserving `{filter, target_commit, semantic_kind}`.
pub(crate) fn focus_target_in_session_with_context(
    path: &str,
    symbol_fqn: Option<&str>,
    filter: Option<&str>,
    target_commit: Option<&str>,
    semantic_kind: Option<&str>,
) {
    let _ = focus_target_on_surface_with_context(
        SESSION_SURFACE,
        path,
        symbol_fqn,
        filter,
        target_commit,
        semantic_kind,
    );
}

/// Localizes failing tests, Activity commands, or failed System service checks directly on the
/// dependency map's Changes + Suspects inspector (or opens `#graph-map` modal when requested).
#[allow(dead_code)]
pub(crate) fn localize_failures_on_map(
    scope: GraphTargetScope,
    failing_queries: &[String],
    hint_paths: &[String],
    prefer_full_modal: bool,
) {
    let normalized_failing: Vec<String> = failing_queries
        .iter()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .take(FAILING)
        .collect();
    let normalized_hints: Vec<String> = hint_paths
        .iter()
        .map(|s| normalize_rel_path(strip_symbol_suffix(s)))
        .filter(|s| !s.is_empty())
        .collect();

    let surface = if prefer_full_modal || open() {
        set_surface_scope(MODAL_SURFACE, scope.clone());
        MODAL_SURFACE
    } else if scope.is_system() {
        let (sid, rid) = match &scope {
            GraphTargetScope::SystemService {
                service_id, run_id, ..
            } => (service_id.clone(), run_id.clone()),
            _ => (String::new(), None),
        };
        if !sid.is_empty() {
            crate::system_map::mark_failed_services(&[sid]);
        }
        if rid.is_some() && dom::get(SYSTEM_CHANGE_SURFACE.canvas).is_some() {
            set_surface_scope(SYSTEM_CHANGE_SURFACE, scope.clone());
            SYSTEM_CHANGE_SURFACE
        } else {
            set_surface_scope(SYSTEM_EMBEDDED_SURFACE, scope.clone());
            SYSTEM_EMBEDDED_SURFACE
        }
    } else {
        set_surface_scope(SESSION_SURFACE, scope.clone());
        SESSION_SURFACE
    };

    let mut st = surface_state(surface);
    if m::text(&st, "target").is_empty() {
        st["target"] = json!("work");
    }
    st["failing"] = json!(normalized_failing);
    st["side_tab"] = json!("changes");
    st["side_pinned"] = json!(true);
    if let Some(first_hint) = normalized_hints.first() {
        st["focus"] = json!(first_hint);
    }
    save_surface_state(surface, &st);

    if surface.embedded {
        let prefix = surface.id_prefix;
        if !normalized_failing.is_empty() {
            dom::set_value(&format!("{prefix}-failing"), &normalized_failing.join("\n"));
        }
        let ch_state = st.clone();
        spawn_local(async move {
            fetch_surface_changes(surface, ch_state).await;
            load_embedded_surface(surface, false).await;
        });
    } else {
        if !normalized_failing.is_empty() {
            dom::set_value("graph-map-failing", &normalized_failing.join("\n"));
        }
        if !open() {
            dom::dialog("graph-map", true);
        }
        spawn_local(async move {
            load(false).await;
        });
    }
}

/// Bridge adapter accepting `@ui-chat`'s JSON failure localization payload (`#22`).
#[allow(dead_code)]
pub(crate) fn localize_failure_request(req: &V) {
    let project_id = m::text(req, "project_id");
    let service_id = m::text(req, "service_id");
    let config_path = m::text(req, "config_path");
    let run_id = m::text(req, "run_id");
    let prefer_full_modal = m::yes(&req["prefer_full_modal"]);
    let failing_queries: Vec<String> = m::array(&req["failing_queries"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let hint_paths: Vec<String> = m::array(&req["hint_paths"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let scope = if !service_id.is_empty() {
        GraphTargetScope::SystemService {
            project_id,
            config_path,
            service_id,
            services: Vec::new(),
            run_id: (!run_id.is_empty()).then_some(run_id),
            run_title: None,
            launches: Vec::new(),
        }
    } else {
        GraphTargetScope::single(project_id)
    };
    localize_failures_on_map(scope, &failing_queries, &hint_paths, prefer_full_modal);
}

pub(in crate::graph_map) fn move_roving_node_focus(
    surface: MapSurface,
    key: &str,
    target: Option<&web_sys::Element>,
) -> bool {
    let cached = SURFACE_PLAN_CACHE.with(|cache| cache.borrow().get(surface.canvas).cloned());
    let Some(cached) = cached else {
        return false;
    };
    if cached.nodes.is_empty() {
        return false;
    }
    let mut state = surface_state(surface);
    let view = m::data(surface.view_key);
    let extras = if view["nodes"].is_array() {
        Some(build_overlay_extras(surface, &view, &state))
    } else {
        None
    };
    let cone_opt = extras.as_ref().and_then(|e| e.isolate_cone.as_ref());
    let allowed_indices: Vec<usize> = (0..cached.nodes.len())
        .filter(|i| cone_opt.is_none_or(|c| c.contains(i)))
        .collect();
    if allowed_indices.is_empty() {
        return false;
    }
    let rects: Vec<(f64, f64, f64, f64)> = allowed_indices
        .iter()
        .map(|&i| {
            let n = &cached.nodes[i];
            (n.x, n.y, BOX_WIDTH, BOX_HEIGHT)
        })
        .collect();
    let curr_global = target
        .and_then(|el| el.closest(".graph-map-node[data-n]").ok().flatten())
        .and_then(|el| el.get_attribute("data-n"))
        .and_then(|s| s.parse::<usize>().ok())
        .or_else(|| state["roving_focus"].as_u64().map(|v| v as usize))
        .unwrap_or(allowed_indices[0]);
    let curr_local = allowed_indices
        .iter()
        .position(|&i| i == curr_global)
        .unwrap_or(0);
    let mut global_to_local: HashMap<usize, usize> = HashMap::with_capacity(allowed_indices.len());
    for (local, &global) in allowed_indices.iter().enumerate() {
        global_to_local.insert(global, local);
    }
    let mut connected = Vec::new();
    for &(u, v) in &cached.pairs {
        if u == curr_global {
            if let Some(&lv) = global_to_local.get(&v) {
                connected.push(lv);
            }
        } else if v == curr_global {
            if let Some(&lu) = global_to_local.get(&u) {
                connected.push(lu);
            }
        }
    }
    let Some(next_local) =
        graph_viewport::next_roving_node_index(&rects, &connected, curr_local, key)
    else {
        return false;
    };
    let next_idx = allowed_indices[next_local];
    state["roving_focus"] = json!(next_idx);
    save_surface_state(surface, &state);
    SURFACE_PLAN_CACHE.with(|cache| {
        if let Some(entry) = cache.borrow_mut().get_mut(surface.canvas) {
            entry.latest_roving_focus = Some(next_idx);
        }
    });
    if let Some(canvas) = dom::get(surface.canvas) {
        if let Some(plane) = canvas.query_selector(".graph-map-plane").ok().flatten() {
            let rects: Vec<(f64, f64, f64, f64)> = cached
                .nodes
                .iter()
                .map(|p| (p.x, p.y, BOX_WIDTH, BOX_HEIGHT))
                .collect();
            let cam = load_surface_camera(surface);
            let (vw, vh, measured) = viewport_size(surface.canvas);
            let viewport = if measured { (vw, vh) } else { (800.0, 500.0) };
            let view_doc = m::data(surface.view_key);
            let next_cam = graph_viewport::dense_dom::apply_roving_node_focus(
                &canvas,
                &plane,
                cam.viewport_cam(),
                viewport,
                &rects,
                next_idx,
                ".graph-map-node[data-n]",
                32.0,
                |el| {
                    el.get_attribute("data-n")
                        .and_then(|s| s.parse::<usize>().ok())
                },
                |idx| cached.materialize_node_html(Some(&view_doc), idx),
                |idx, el| {
                    if let Some(np) = cached.latest_patch.as_ref().and_then(|p| p.nodes.get(idx)) {
                        apply_node_patch_to_el(
                            el,
                            idx,
                            np,
                            Some(next_idx),
                            Some(&cached.latest_path_nodes),
                        );
                    }
                    if let Some(Some(spec)) = cached.dense_extra_specs.get(idx) {
                        if let Ok(None) = plane.query_selector(&format!("[data-of='{idx}']")) {
                            let dim_over = cached
                                .latest_patch
                                .as_ref()
                                .and_then(|p| p.nodes.get(idx))
                                .map(|n| n.portal_dim);
                            let extra_html =
                                render_dense_node_extras_for_mounted(&view, spec, dim_over);
                            if !extra_html.is_empty() {
                                let _ = graph_viewport::append_html_elements(&plane, &extra_html);
                            }
                        }
                    }
                    invalidate_hover_cache(surface.canvas);
                },
            );
            if let Some(panned) = next_cam {
                let mut updated = load_surface_camera(surface);
                updated.apply_viewport_cam(panned);
                updated.fit_mode = false;
                save_surface_camera(surface, &updated);
                apply_surface_camera(surface, &updated);
            }
        }
    }
    let label = cached
        .node_labels
        .get(next_idx)
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| format!("Node {next_idx}"));
    let live_id = if surface.embedded {
        format!("{}-live-announce", surface.id_prefix)
    } else {
        "graph-map-live-announce".to_owned()
    };
    graph_viewport::announce_live(&live_id, &format!("Focused {label}"));
    true
}

pub(in crate::graph_map) fn transfer_surface_state_to_modal(emb: &V, mut modal: V) -> V {
    modal["whole"] = json!(m::yes(&emb["whole"]));
    modal["notests"] = json!(m::yes(&emb["notests"]));
    modal["certain"] = json!(m::yes(&emb["certain"]));
    modal["filter"] = json!(m::text(emb, "filter"));
    modal["scope"] = json!(m::text(emb, "scope"));
    modal["spread"] = emb["spread"].clone();
    modal["focus"] = emb["focus"].clone();
    modal["launch"] = emb["launch"].clone();
    modal["compare_launch"] = emb["compare_launch"].clone();
    modal["agent"] = emb["agent"].clone();
    modal["run_id"] = emb["run_id"].clone();
    modal["traced_path"] = emb["traced_path"].clone();
    modal["isolate_active"] = json!(m::yes(&emb["isolate_active"]));
    modal["isolate_depth"] = json!(emb["isolate_depth"].as_u64().unwrap_or(2).clamp(1, 3));
    modal["failing"] = emb["failing"].clone();
    modal["roving_focus"] = emb["roving_focus"].clone();
    modal["compare_base"] = json!(m::text(emb, "compare_base"));
    modal["patterns"] = json!(m::yes(&emb["patterns"]));
    modal["history"] = json!(m::yes(&emb["history"]));
    modal["modules"] = json!(m::yes(&emb["modules"]));
    modal["no_incoming"] = json!(m::yes(&emb["no_incoming"]));
    modal["lens_tests"] = json!(m::yes(&emb["lens_tests"]));
    for key in [
        "out",
        "into",
        "follow",
        "path",
        "shared",
        "side",
        "skip",
        "cycle",
        "cycle_base",
        "cycle_return",
        "pattern",
        "pattern_return",
        "kinds",
        "cone",
    ] {
        if !emb[key].is_null() {
            modal[key] = emb[key].clone();
        }
    }
    if emb["auto_lod"].is_boolean() {
        modal["auto_lod"] = emb["auto_lod"].clone();
    }
    if !emb["lod"].is_null() {
        modal["lod"] = emb["lod"].clone();
    }
    if !emb["min_weight"].is_null() {
        modal["min_weight"] = emb["min_weight"].clone();
    }
    if !emb["fit"].is_null() {
        modal["fit"] = json!(m::yes(&emb["fit"]));
    }
    if !m::text(emb, "range_base").is_empty() {
        modal["range_base"] = json!(m::text(emb, "range_base"));
    }
    if !m::text(emb, "range_head").is_empty() {
        modal["range_head"] = json!(m::text(emb, "range_head"));
    }
    if !m::text(emb, "range_anchor").is_empty() {
        modal["range_anchor"] = json!(m::text(emb, "range_anchor"));
    }
    modal["side_tab"] = json!(match m::text(emb, "side_tab").as_str() {
        k @ ("changes" | "analysis") => k,
        _ => "details",
    });
    modal["changes_filter"] = json!(match m::text(emb, "changes_filter").as_str() {
        k @ ("all" | "created" | "modified" | "deleted") => k,
        _ => "all",
    });
    modal["member_filter"] = json!(match m::text(emb, "member_filter").as_str() {
        k @ ("all" | "used" | "exposed" | "risk") => k,
        _ => "all",
    });
    modal["side_pinned"] = json!(m::yes(&emb["side_pinned"]));
    normalize_map_state(modal)
}

pub(in crate::graph_map) async fn open_full_from_surface(surface: MapSurface) {
    set_surface_scope(MODAL_SURFACE, surface_scope(surface));
    let emb = surface_state(surface);
    let emb_cam = load_surface_camera(surface);
    let target = effective_target(&emb);
    let mut modal = transfer_surface_state_to_modal(&emb, anew(&target));
    let keep_emb_cam = !emb_cam.needs_measure && emb_cam.plane_w > 0.0 && !emb_cam.fit_mode;
    if keep_emb_cam {
        modal["fit"] = json!(false);
    }
    m::set("graph-map", modal.clone());

    let src_pin_key = surface_pin_key(surface, &emb);
    let dst_pin_key = surface_pin_key(MODAL_SURFACE, &modal);
    if src_pin_key != dst_pin_key {
        let src_pins = graph_viewport::load_pins(&src_pin_key);
        if !src_pins.is_empty() && !graph_viewport::has_pins(&dst_pin_key) {
            graph_viewport::persist_pins_map(&dst_pin_key, &src_pins);
        }
    }

    let emb_changes = m::data(surface.changes_key);
    if emb_changes.is_object() {
        m::set(MODAL_SURFACE.changes_key, emb_changes);
    } else {
        m::take(MODAL_SURFACE.changes_key);
    }
    let emb_impact = m::data(surface.impact_key);
    if emb_impact.is_object() {
        m::set(MODAL_SURFACE.impact_key, emb_impact);
    } else {
        m::take(MODAL_SURFACE.impact_key);
    }

    dom::disabled("graph-find-search", false);
    dom::set_value("graph-map-filter", &m::text(&modal, "filter"));
    dom::set_checked("graph-map-whole", m::yes(&modal["whole"]));
    dom::set_checked("graph-map-notests", m::yes(&modal["notests"]));
    dom::set_checked("graph-map-certain", m::yes(&modal["certain"]));
    dom::set_checked("graph-map-patterns", m::yes(&modal["patterns"]));
    dom::set_checked("graph-map-history", m::yes(&modal["history"]));
    dom::set_checked("graph-map-modules", m::yes(&modal["modules"]));
    dom::set_checked("graph-map-no-incoming", m::yes(&modal["no_incoming"]));
    dom::set_checked("graph-map-fit", m::yes(&modal["fit"]));
    dom::set_value(
        "graph-map-path-from",
        &m::text(&modal["traced_path"], "from"),
    );
    dom::set_value("graph-map-path-to", &m::text(&modal["traced_path"], "to"));
    dom::set_value(
        "graph-map-isolate-depth",
        &modal["isolate_depth"].as_u64().unwrap_or(2).to_string(),
    );
    let failing_lines: Vec<String> = m::array(&modal["failing"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    dom::set_value("graph-map-failing", &failing_lines.join("\n"));
    m::next("graph-find");
    dom::cancel_timer("graph-find-search");
    m::set("graph-find", V::Null);
    dom::set_value("graph-find-search", "");
    show_found();
    dom::error("graph-map-error", "");
    dom::hidden("graph-map-progress", true);
    blank();
    if keep_emb_cam {
        let mut cam = emb_cam;
        cam.fit_mode = false;
        cam.needs_measure = false;
        save_surface_camera(MODAL_SURFACE, &cam);
        dom::text(
            "graph-map-zoom-level",
            &format!("{:.0}%", cam.scale * 100.0),
        );
    } else {
        dom::text(
            "graph-map-zoom-level",
            if m::yes(&modal["fit"]) { "Fit" } else { "100%" },
        );
    }
    dom::text("graph-map-chosen", "");
    dom::text(
        "graph-map-note",
        if !target.is_empty() {
            "Reading the change…"
        } else {
            "Reading the graph…"
        },
    );
    shut();
    reset_commit_dock_for_scope();
    list();
    dom::dialog("graph-map", true);
    watch();
    spawn_local(place());
    load(false).await;
}
/// Opens the full `#graph-map` modal scoped to a System member service (`__system__`, standalone
/// repo, or monorepo subfolder service) without mutating `#project`.
#[allow(dead_code)]
pub(crate) fn open_system_service_map(scope: GraphTargetScope, initial_target: Option<&str>) {
    let target = initial_target.unwrap_or("").trim().to_owned();
    set_surface_scope(MODAL_SURFACE, scope);
    m::next(MODAL_SURFACE.map_epoch);
    m::next(MODAL_SURFACE.changes_epoch);
    m::next(MODAL_SURFACE.impact_epoch);
    m::take(MODAL_SURFACE.changes_key);
    m::take(MODAL_SURFACE.impact_key);
    m::set(MODAL_SURFACE.view_key, V::Null);

    let mut modal = anew(&target);
    if let Some(rid) = target.strip_prefix("sysrun:").filter(|s| !s.is_empty()) {
        modal["run_id"] = json!(rid);
    }
    if target == "work" {
        modal["side_tab"] = json!("changes");
    }
    m::set("graph-map", modal.clone());

    dom::disabled("graph-find-search", false);
    dom::set_value("graph-map-filter", "");
    dom::set_checked("graph-map-whole", false);
    dom::set_checked("graph-map-notests", m::yes(&modal["notests"]));
    dom::set_checked("graph-map-certain", m::yes(&modal["certain"]));
    dom::set_checked("graph-map-patterns", m::yes(&modal["patterns"]));
    dom::set_checked("graph-map-history", m::yes(&modal["history"]));
    dom::set_checked("graph-map-modules", m::yes(&modal["modules"]));
    dom::set_checked("graph-map-no-incoming", m::yes(&modal["no_incoming"]));
    dom::set_checked("graph-map-fit", m::yes(&modal["fit"]));
    dom::set_value("graph-map-path-from", "");
    dom::set_value("graph-map-path-to", "");
    dom::set_value("graph-map-isolate-depth", "2");
    dom::set_value("graph-map-failing", "");
    dom::hidden("graph-map-path-clear", true);
    dom::hidden("graph-map-stale-banner", true);
    m::next("graph-find");
    dom::cancel_timer("graph-find-search");
    m::set("graph-find", V::Null);
    dom::set_value("graph-find-search", "");
    show_found();
    dom::error("graph-map-error", "");
    dom::hidden("graph-map-progress", true);
    dom::text(
        "graph-map-zoom-level",
        if m::yes(&modal["fit"]) { "Fit" } else { "100%" },
    );
    blank();
    dom::text("graph-map-chosen", "");
    dom::text(
        "graph-map-note",
        if !target.is_empty() {
            "Reading the change…"
        } else {
            "Reading the graph…"
        },
    );
    shut();
    reset_commit_dock_for_scope();
    list();
    dom::dialog("graph-map", true);
    watch();
    spawn_local(async move {
        let _ = place().await;
        if target.contains(':') && !target.starts_with("sysrun:") && !choose(&target).await {
            return;
        }
        load(false).await;
    });
}
/// Selects a file (or file + symbol target) of a member service in the picture embedded in the System view.
pub(crate) fn focus_system_target(
    kind: SystemEmbeddedKind,
    path: &str,
    symbol: Option<&str>,
) -> bool {
    focus_target_on_surface(kind.surface(), path, symbol)
}

#[allow(dead_code)]
pub(crate) fn focus_system_file(kind: SystemEmbeddedKind, path: &str) -> bool {
    focus_system_target(kind, path, None)
}
