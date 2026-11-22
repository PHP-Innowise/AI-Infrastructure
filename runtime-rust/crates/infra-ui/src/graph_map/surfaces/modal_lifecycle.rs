use super::*;

pub(in crate::graph_map) fn set_surface_attrs(surface: MapSurface, view: &V) {
    let side_family = if patterned(view) {
        m::text(&view["pattern"], "family")
    } else {
        String::new()
    };
    let side_layered = if layered(view) { "true" } else { "false" };
    for target_id in [surface.side, surface.note, surface.canvas] {
        dom::attr(target_id, "data-pattern-family", &side_family);
        dom::attr(target_id, "data-layered", side_layered);
    }
}

pub(in crate::graph_map) fn render(view: &V) {
    crate::graph_explanation::surface_map_context(
        MODAL_SURFACE.canvas,
        view,
        &state()["cycle_base"]["revision"],
        &surface_scope(MODAL_SURFACE),
    );
    show_launches(view);
    let module_mode = m::yes(&state()["modules"]);
    let scenario_mode = view["mode"] == "scenario";
    dom::set_checked("graph-map-modules", module_mode);
    dom::set_checked("graph-map-no-incoming", m::yes(&state()["no_incoming"]));
    dom::disabled(
        "graph-map-no-incoming",
        module_mode
            || scenario_mode
            || follows(view)
            || paths(view)
            || cycles(view)
            || shares(view)
            || patterned(view),
    );
    // A chosen change stays chosen while one of its files is open.
    let change = view["chosen"].is_object();
    let modal_state = state();
    let modal_changes = surface_changes_doc(MODAL_SURFACE, &modal_state);
    let banner_html = render_commit_banner_for_view(view, modal_changes.as_ref());
    html("graph-commit-banner", &banner_html);
    dom::hidden("graph-commit-banner", banner_html.trim().is_empty());
    if m::yes(&commit_dock()["open"]) {
        show_commit_dock();
    }
    dom::text("graph-map-chosen", &chosen(view));
    let building = change && view["state"] == "building";
    dom::hidden("graph-map-progress", !building);
    let mut phase = String::new();
    if building {
        let shown = crate::graph::graph_view(
            &json!({"enabled": true, "state": "building", "build": view["build"]}),
        );
        if let Some(bar) = dom::get("graph-map-progress-bar") {
            let _ = match shown.percent {
                Some(percent) => bar.set_attribute("value", &percent.to_string()),
                None => bar.remove_attribute("value"),
            };
        }
        dom::text("graph-map-progress-amount", &shown.amount);
        dom::text("graph-map-progress-left", &shown.left);
        phase = shown.note;
    }
    if change && view["state"] == "failed" {
        dom::error(
            "graph-map-error",
            &format!(
                "The graph of that commit could not be built: {}",
                format_graph_status_text(&m::text(view, "failure"))
            ),
        );
    }
    let before = interim(view, &phase);
    if change && !view["nodes"].is_array() {
        // Neither the commit's own graph nor the project's to draw on.
        blank();
        html("graph-commit-banner", &banner_html);
        dom::hidden("graph-commit-banner", banner_html.trim().is_empty());
        dom::text(
            "graph-map-note",
            if before.is_empty() && view["state"] != "failed" {
                "Reading the change…"
            } else {
                &before
            },
        );
        return;
    }
    let mut crumbs_html = crumbs(view);
    let trail = surface_history_trail_html(MODAL_SURFACE);
    if !trail.is_empty() {
        crumbs_html.push_str(&trail);
    }
    html("graph-map-crumbs", &crumbs_html);
    dom::text("graph-map-summary", &summary(view));
    dom::text("graph-map-note", &combine_note(before, view));
    dom::set_checked("graph-map-certain", m::yes(&view["certain"]));
    dom::set_checked("graph-map-notests", m::yes(&view["notests"]));

    dom::set_checked("graph-map-history", m::yes(&state()["history"]));
    dom::disabled("graph-map-history", module_mode || scenario_mode);
    dom::disabled("graph-map-patterns", module_mode || scenario_mode);
    dom::set_checked("graph-map-fit", m::yes(&state()["fit"]));
    dom::set_checked("graph-map-whole", m::yes(&state()["whole"]));
    dom::set_checked("graph-map-patterns", m::yes(&state()["patterns"]));
    dom::disabled(
        "graph-find-search",
        at_base(view) || module_mode || scenario_mode,
    );
    // Of what two places share, the filter keeps the names that hold it.
    dom::disabled(
        "graph-map-filter",
        follows(view) || paths(view) || cycles(view) || patterned(view),
    );
    // Keep keyboard focus on the selected node or sidebar control through a repaint.
    const NODE_BUTTONS: &str = "#graph-map-canvas .graph-map-node";
    let active = dom::document().active_element();
    let module_focus = module_mode
        && active.as_ref().is_some_and(|node| {
            node.matches(
                "#graph-map-canvas .graph-map-node, [data-action='graph-map-module-select']",
            )
            .unwrap_or(false)
        });
    let keyboard_node = active
        .as_ref()
        .filter(|node| {
            node.matches(&format!("{NODE_BUTTONS}:focus-visible"))
                .unwrap_or(false)
        })
        .and_then(|node| {
            let path = node.get_attribute("data-path");
            let payload = node.get_attribute("data-payload");
            (path.is_some() || payload.is_some()).then_some((path, payload))
        });
    struct SavedSideFocus {
        id: Option<String>,
        action: Option<String>,
        path: Option<String>,
        payload: Option<String>,
    }
    let side_focus = active
        .as_ref()
        .filter(|node| {
            node.closest("#graph-map-side").ok().flatten().is_some()
                && node.matches(":focus-visible").unwrap_or(false)
        })
        .map(|node| SavedSideFocus {
            id: node.get_attribute("id"),
            action: node.get_attribute("data-action"),
            path: node.get_attribute("data-path"),
            payload: node.get_attribute("data-payload"),
        });
    let modal_impact = cached_impact_for_selection(MODAL_SURFACE, &modal_state, Some(view));
    let modal_key = key_now(&modal_state);
    let extras = build_overlay_extras(MODAL_SURFACE, view, &modal_state);
    let modal_overlay = CanvasOverlayContext {
        live_focus: None,
        changes: modal_changes.as_ref(),
        impact: modal_impact.as_ref(),
        links_off: modal_key.off,
        hide_transitive: modal_key.hide_transitive,
        cone: modal_key.cone,
        pinned_positions: Some(&extras.pins),
        path_nodes: Some(&extras.path_nodes),
        path_edges: Some(&extras.path_edges),
        isolate_cone: extras.isolate_cone.as_ref(),
        viewport_bounds: extras.viewport_bounds,
        roving_focus: extras.roving_focus,
    };
    sync_modal_lens_and_display(&modal_state, view);
    if let Some(patched) = try_patch_canvas_dom_in_place("graph-map-canvas", view, &modal_overlay) {
        let mut cam = load_surface_camera(MODAL_SURFACE);
        cam.plane_w = patched.width;
        cam.plane_h = patched.height;
        cam.bbox = patched.selected_bbox.or(patched.focus.bbox);
        save_surface_camera(MODAL_SURFACE, &cam);
        apply_surface_camera(MODAL_SURFACE, &cam);
        record_surface_history(MODAL_SURFACE, &modal_state, view, &cam);
        update_surface_active_camera(MODAL_SURFACE, &cam);
    } else {
        let rendered = canvas_with_overlays(view, MODAL_SURFACE.id_prefix, modal_overlay.clone());
        store_surface_plan_cache("graph-map-canvas", view, &rendered, Some(&modal_overlay));
        html("graph-map-canvas", &rendered.html);
        html("graph-map-canvas-radar", &rendered.radar);
        invalidate_hover_cache("graph-map-canvas");
        let mut cam = load_surface_camera(MODAL_SURFACE);
        let fresh = cam.needs_measure || cam.plane_w <= 0.0 || cam.plane_h <= 0.0;
        cam.plane_w = rendered.width;
        cam.plane_h = rendered.height;
        save_surface_camera(MODAL_SURFACE, &cam);
        if let Some(scene) = rendered.dense_scene {
            let meta = rendered.dense_meta.unwrap_or_default();
            graph_viewport::canvas2d::store_scene_with_meta("graph-map-canvas", scene, meta);
        } else {
            graph_viewport::canvas2d::clear_scene("graph-map-canvas");
        }
        let cam_for_hist = load_surface_camera(MODAL_SURFACE);
        record_surface_history(MODAL_SURFACE, &modal_state, view, &cam_for_hist);
        if fresh || (m::yes(&state()["fit"]) && cam.fit_mode) {
            fit();
        } else {
            apply_surface_camera(MODAL_SURFACE, &cam);
        }
        let settled_cam = load_surface_camera(MODAL_SURFACE);
        update_surface_active_camera(MODAL_SURFACE, &settled_cam);
    }
    let crumbs_html = {
        let mut s = crumbs(view);
        let trail = surface_history_trail_html(MODAL_SURFACE);
        if !trail.is_empty() {
            s.push_str(&trail);
        }
        s
    };
    html("graph-map-crumbs", &crumbs_html);
    sync_history_and_pin_controls(MODAL_SURFACE, &modal_state);
    html(
        "graph-map-arrow-legend",
        &key_html(view, &modal_key, "modal"),
    );
    let restore_side_focus = |saved: Option<SavedSideFocus>| {
        let Some(SavedSideFocus {
            id,
            action,
            path,
            payload,
        }) = saved
        else {
            return;
        };
        let candidates = dom::all(
            "#graph-map-side button, #graph-map-side a, #graph-map-side input, #graph-map-side select, #graph-map-side [tabindex]",
        );
        let found = id
            .as_deref()
            .and_then(|wanted| {
                candidates
                    .iter()
                    .find(|el| el.get_attribute("id").as_deref() == Some(wanted))
            })
            .or_else(|| {
                action.as_deref().zip(path.as_deref()).and_then(|(act, p)| {
                    candidates.iter().find(|el| {
                        el.get_attribute("data-action").as_deref() == Some(act)
                            && el.get_attribute("data-path").as_deref() == Some(p)
                    })
                })
            })
            .or_else(|| {
                action
                    .as_deref()
                    .zip(payload.as_deref())
                    .and_then(|(act, pay)| {
                        candidates.iter().find(|el| {
                            el.get_attribute("data-action").as_deref() == Some(act)
                                && el.get_attribute("data-payload").as_deref() == Some(pay)
                        })
                    })
            })
            .or_else(|| {
                path.as_deref().and_then(|p| {
                    candidates
                        .iter()
                        .find(|el| el.get_attribute("data-path").as_deref() == Some(p))
                })
            })
            .or_else(|| {
                action
                    .as_deref()
                    .filter(|_| path.is_none() && payload.is_none())
                    .and_then(|act| {
                        candidates
                            .iter()
                            .find(|el| el.get_attribute("data-action").as_deref() == Some(act))
                    })
            })
            .and_then(|el| el.clone().dyn_into::<web_sys::HtmlElement>().ok());
        if let Some(el) = found {
            let _ = el.focus();
        }
    };
    let changes_tab = m::text(&modal_state, "side_tab") == "changes";
    if scenario_mode && !changes_tab {
        let focused = dom::document().active_element().is_some_and(|element| {
            element
                .get_attribute("data-action")
                .is_some_and(|action| action.starts_with("graph-map-scenario"))
        });
        m::take("graph-map-attachment");
        dom::attr("graph-map-canvas", "data-picking", "false");
        html(
            "graph-map-side",
            &format!(
                "{}{}",
                side_tabs_html("details", changes_count_of(modal_changes.as_ref()), "modal"),
                scenario_side(view)
            ),
        );
        dom::text(
            "graph-map-legend",
            "Arrows follow dependencies away from the starting point.",
        );
        if focused {
            dom::focus("graph-map-summary");
        } else {
            restore_side_focus(side_focus);
        }
        return;
    }
    if module_mode && !changes_tab {
        dom::attr("graph-map-canvas", "data-picking", "false");
        m::take("graph-map-attachment");
        html(
            "graph-map-side",
            &format!(
                "{}{}",
                side_tabs_html("details", changes_count_of(modal_changes.as_ref()), "modal"),
                modules_side(view)
            ),
        );
        dom::text(
            "graph-map-legend",
            "Arrows show dependencies between Graphify communities. Hollow links are inferred.",
        );
        if let Some(key) = view["focus"]["path"].as_str().filter(|_| module_focus) {
            let key = json!(key).to_string();
            if let Some(node) = dom::all(NODE_BUTTONS)
                .into_iter()
                .find(|node| node.get_attribute("data-payload").as_deref() == Some(key.as_str()))
                .and_then(|node| node.dyn_into::<web_sys::HtmlElement>().ok())
            {
                let _ = node.focus();
                return;
            }
        }
        restore_side_focus(side_focus);
        return;
    }
    if let Some((path, payload)) = keyboard_node {
        let nodes = dom::all(NODE_BUTTONS);
        let matched = path
            .as_deref()
            .and_then(|wanted| {
                nodes
                    .iter()
                    .find(|node| node.get_attribute("data-path").as_deref() == Some(wanted))
            })
            .or_else(|| {
                payload.as_deref().and_then(|wanted| {
                    nodes
                        .iter()
                        .find(|node| node.get_attribute("data-payload").as_deref() == Some(wanted))
                })
            })
            .and_then(|node| node.clone().dyn_into::<web_sys::HtmlElement>().ok());
        if let Some(node) = matched {
            let _ = node.focus();
        }
    }
    // While the other end of a path is chosen, a box is that end.
    let state = state();
    dom::attr(
        "graph-map-canvas",
        "data-picking",
        if state["pick"].is_object() {
            "true"
        } else {
            "false"
        },
    );
    let pull_place = view["chosen"]["kind"] == "pull"
        && (changes(view) || symbols(view))
        && view["focus"]["path"]
            .as_str()
            .is_some_and(|path| !path.is_empty());
    let show_cycle = !cycles(view)
        && !state["pick"].is_object()
        && (pull_place
            || !dom::all(
                "#graph-map-canvas .graph-map-node[data-selected=\"true\"]:is([data-cycle=\"true\"],[data-circle=\"through\"])",
            )
            .is_empty());
    let action = if cycles(view) {
        let mut actions = base_cycle_button(view, at_base(view));
        if state["cycle_return"].is_object() {
            actions.push_str(CYCLE_BACK);
        }
        actions
    } else if patterned(view) && state["pattern_return"].is_object() {
        PATTERN_BACK.to_owned()
    } else if show_cycle {
        format!(
            "<button type=\"button\" class=\"button compact\" id=\"graph-map-show-cycle\" data-action=\"graph-map-cycle\" data-payload=\"{}\">Show cycle</button>",
            payload(&view["focus"]["path"]),
        )
    } else {
        String::new()
    };
    let active_action = dom::document()
        .active_element()
        .filter(|element| element.matches(":focus-visible").unwrap_or(false))
        .and_then(|element| element.get_attribute("data-action"));
    let opening_cycle = cycles(view) && active_action.as_deref() == Some("graph-map-cycle");
    let returning = !cycles(view) && active_action.as_deref() == Some("graph-map-cycle-back");
    // A finding that was opened from the keyboard, or left, took the row or
    // the button that had the focus with it.
    let turned = (patterned(view) && active_action.as_deref() == Some("graph-map-pattern"))
        || (!patterned(view) && active_action.as_deref() == Some("graph-map-pattern-back"));
    let scenario_action = scenario_button(view);
    let test_back = if paths(view) && state["test_path_return"].is_object() {
        "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-test-back\">Back to changed files</button>"
    } else {
        ""
    };
    set_surface_attrs(MODAL_SURFACE, view);
    html(
        "graph-map-side",
        &map_side_html(
            MODAL_SURFACE,
            view,
            &state,
            &action,
            &scenario_action,
            test_back,
            "graph-map-attach",
        ),
    );
    if matches!(
        active_action.as_deref(),
        Some(
            "graph-map-test-path"
                | "graph-map-test-back"
                | "graph-map-scenario-back"
                | "graph-map-scenario-open"
        )
    ) {
        dom::focus("graph-map-summary");
    } else if returning && show_cycle {
        dom::focus("graph-map-show-cycle");
    } else if opening_cycle || returning || turned {
        dom::focus("graph-map-summary");
    } else if active_action.as_deref() == Some("graph-map-cycle-back") {
        dom::focus("graph-map-cycle-back");
    } else if active_action.as_deref() == Some("graph-map-cycle-switch") {
        if dom::get("graph-map-cycle-switch").is_some() {
            dom::focus("graph-map-cycle-switch");
        } else {
            dom::focus("graph-map-summary");
        }
    } else if show_cycle && active_action.as_deref() == Some("graph-map-cycle") {
        dom::focus("graph-map-show-cycle");
    } else {
        restore_side_focus(side_focus);
    }
    html("graph-map-legend", &legend(view, change));
}
/// Asks the server to work out a pull request, a branch or a commit, which
/// starts the graph of its commit when there is none. `false` when it could
/// not, or when something else was chosen meanwhile.
pub(in crate::graph_map) async fn choose(target: &str) -> bool {
    let scope = surface_scope(MODAL_SURFACE);
    if scope.is_empty() {
        dom::attr("graph-map-canvas", "aria-busy", "false");
        return false;
    }
    let scope_key = scope.scope_key();
    dom::attr("graph-map-canvas", "aria-busy", "true");
    dom::error("graph-map-error", "");
    dom::hidden("graph-map-progress", true);
    blank();
    dom::text("graph-map-chosen", "");
    dom::text("graph-map-note", "Reading the change…");
    let (path, body) = if target.starts_with("sysrun:") || target.is_empty() {
        scope.act_request("rebuild", None)
    } else {
        scope.act_request("changes", Some(target))
    };
    let asked = api::post(&path, body).await;
    let current_scope = surface_scope(MODAL_SURFACE).scope_key();
    if current_scope != scope_key || effective_target(&state()) != target {
        if current_scope.is_empty() || !open() {
            dom::attr("graph-map-canvas", "aria-busy", "false");
        }
        return false;
    }
    match asked {
        Ok(_) => true,
        Err(error) => {
            dom::attr("graph-map-canvas", "aria-busy", "false");
            dom::text("graph-map-note", "");
            dom::error("graph-map-error", &error.message);
            false
        }
    }
}
/// Reads the map as the Show list and the folder stand. `quiet` is a repeat
pub(in crate::graph_map) fn reconcile_view_state(
    mut state: V,
    view: &V,
    current_fit: V,
    target: &str,
) -> (V, bool) {
    let pinned_file_focus =
        if m::yes(&state["side_pinned"]) && looks_like_file_path(&m::text(&state, "focus")) {
            Some(state["focus"].clone())
        } else {
            None
        };
    // Fit can change locally while this request is in flight.
    state["fit"] = current_fit;
    if view["scope"].is_string() && !at_base(view) && view["mode"] != "modules" {
        state["scope"] = view["scope"].clone();
    }
    if follows(view) {
        // What is no longer drawn is no longer unfolded.
        let drawn = m::array(&view["nodes"]);
        for key in ["out", "into"] {
            let kept = unfolded(&state, key)
                .into_iter()
                .filter(|place| drawn.iter().any(|node| node["path"] == place.as_str()))
                .collect::<Vec<_>>();
            state[key] = json!(kept);
        }
    } else if shares(view) {
        state = begins(state, view["shared"]["skip"].as_u64().unwrap_or(0));
    } else if view["mode"] == "scenario" {
        state["scenario_skip"] = view["scenario"]["skip"].clone();
    } else if !paths(view) && !cycles(view) && !patterned(view) && !state["scenario"].is_string() {
        // The graph does not know the place, or one of the two
        // places, or the pattern, or they were left.
        state = unfollowed(state);
        if let Some(pinned) = pinned_file_focus {
            state["focus"] = pinned;
        }
    }
    // A graph that was never started, as after a restart of the
    // Harness, is asked for once more.
    let again = view["state"] == "missing" && state["asked"] != json!(target);
    if again {
        state["asked"] = json!(target);
    }
    (state, again)
}
/// Reads one picture of the map and draws it. `quiet` is set while polling
/// while a graph is being built, which leaves the picture as it is.
pub(in crate::graph_map) async fn load(quiet: bool) {
    let state = state();
    let scope = surface_scope(MODAL_SURFACE);
    if scope.is_empty() {
        return;
    }
    let scope_key = scope.scope_key();
    let target = effective_target(&state);
    let path = build_scoped_map_path(&scope, &state, false);
    if !quiet {
        dom::attr("graph-map-canvas", "aria-busy", "true");
        let ch_state = state.clone();
        spawn_local(async move {
            fetch_surface_changes(MODAL_SURFACE, ch_state).await;
        });
    } else if surface_changes_doc(MODAL_SURFACE, &state).is_none() {
        let ch_state = state.clone();
        spawn_local(async move {
            fetch_surface_changes(MODAL_SURFACE, ch_state).await;
        });
    }
    let mut again = false;
    match api::scoped("graph-map", &path).await {
        Ok((epoch, view)) => {
            if !m::current("graph-map", epoch)
                || surface_scope(MODAL_SURFACE).scope_key() != scope_key
            {
                return;
            }
            let (next_state, retry) =
                reconcile_view_state(state, &view, self::state()["fit"].clone(), &target);
            again = retry;
            m::set("graph-map", next_state.clone());
            m::set(MODAL_SURFACE.view_key, view.clone());
            dom::error("graph-map-error", "");
            dom::hidden("graph-map-stale-banner", true);
            render(&view);
            ensure_surface_impact(MODAL_SURFACE, &next_state, Some(&view), false);
            if open()
                && (view["state"] == "building"
                    || m::yes(&view["rebuilding"])
                    || matches!(
                        view["cycle"]["comparison"]["state"].as_str(),
                        Some("building" | "waiting_head")
                    )
                    || comparing_order(&view))
            {
                dom::later("graph-map-poll", 1000, || {
                    spawn_local(async {
                        load(true).await;
                    })
                });
            } else {
                dom::cancel_timer("graph-map-poll");
            }
        }
        Err(error) if error.aborted => return,
        Err(error) => {
            m::take("graph-map-attachment");
            dom::cancel_timer("graph-map-poll");
            if state["cycle_base"].is_object() {
                // A stale or pruned snapshot leaves a way back to the PR
                // cycle. The old drawing might be of another revision.
                html("graph-map-crumbs", "");
                html("graph-map-canvas", "");
                html("graph-map-canvas-radar", "");
                sync_radar("graph-map-canvas", None);
                let mut actions = base_cycle_button(&V::Null, true);
                if state["cycle_return"].is_object() {
                    actions.push_str(CYCLE_BACK);
                }
                html("graph-map-side", &actions);
                html("graph-map-legend", "");
                html("graph-map-arrow-legend", "");
                dom::hidden("graph-map-progress", true);
                dom::text("graph-map-note", "");
                dom::text("graph-map-summary", "PR base cycle");
                dom::disabled("graph-find-search", true);
                dom::error("graph-map-error", &error.message);
                dom::focus("graph-map-cycle-switch");
            } else if error.status == 409 && !target.is_empty() && state["asked"] != json!(target) {
                // The server no longer knows what was chosen.
                let mut state = state;
                state["fit"] = self::state()["fit"].clone();
                state["asked"] = json!(target);
                m::set("graph-map", state);
                again = true;
            } else {
                dom::hidden("graph-map-progress", true);
                dom::error("graph-map-error", &error.message);
            }
        }
    }
    dom::attr("graph-map-canvas", "aria-busy", "false");
    if again && choose(&target).await {
        Box::pin(load(false)).await;
    }
}
/// What the checkout is on and its open pull requests, for the Show list.
pub(in crate::graph_map) async fn place() {
    let scope = surface_scope(MODAL_SURFACE);
    if scope.is_empty() {
        return;
    }
    let scope_key = scope.scope_key();
    let path = format_scoped_endpoint(&scope, "pulls", Vec::new());
    let Ok(found) = api::get(&path).await else {
        return;
    };
    if surface_scope(MODAL_SURFACE).scope_key() == scope_key {
        m::set("graph-map-place", found.clone());
        list();
        if m::yes(&commit_dock()["open"]) {
            html(
                "graph-commit-graph-scope",
                &scopes(
                    &found,
                    &m::data("graph-pick-branches"),
                    &m::text(&commit_dock(), "scope"),
                ),
            );
        }
    }
}
pub(in crate::graph_map) fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}
pub(in crate::graph_map) struct ClickOutcome {
    pub(in crate::graph_map) handled: bool,
    pub(in crate::graph_map) reload: bool,
    pub(in crate::graph_map) rerender: bool,
    pub(in crate::graph_map) rerender_side: bool,
    pub(in crate::graph_map) force_impact: bool,
    pub(in crate::graph_map) clear_filter: Option<String>,
    pub(in crate::graph_map) clear_find: bool,
    pub(in crate::graph_map) disable_find: Option<bool>,
    pub(in crate::graph_map) save_notests: Option<bool>,
    pub(in crate::graph_map) save_fit: Option<bool>,
    pub(in crate::graph_map) open_link: Option<(String, String)>,
    pub(in crate::graph_map) switch_service: Option<(String, String)>,
}
impl ClickOutcome {
    pub(in crate::graph_map) fn unhandled() -> Self {
        Self {
            handled: false,
            reload: false,
            rerender: false,
            rerender_side: false,
            force_impact: false,
            clear_filter: None,
            clear_find: false,
            disable_find: None,
            save_notests: None,
            save_fit: None,
            open_link: None,
            switch_service: None,
        }
    }
    pub(in crate::graph_map) fn stay() -> Self {
        Self {
            handled: true,
            ..Self::unhandled()
        }
    }
    pub(in crate::graph_map) fn reload() -> Self {
        Self {
            handled: true,
            reload: true,
            ..Self::unhandled()
        }
    }
}
pub(in crate::graph_map) fn apply_click_action(
    state: &mut V,
    action: &str,
    payload: &V,
    surface: MapSurface,
    busy: bool,
    map_failed: bool,
) -> ClickOutcome {
    let mut out = ClickOutcome::reload();
    match action {
        "graph-side-tab" => {
            let tab = match payload["tab"].as_str() {
                Some("changes") => "changes",
                Some("analysis") => "analysis",
                _ => "details",
            };
            state["side_tab"] = json!(tab);
            state["side_pinned"] = json!(true);
            return ClickOutcome {
                handled: true,
                rerender: true,
                rerender_side: true,
                ..ClickOutcome::unhandled()
            };
        }
        "graph-changes-filter" => {
            let filter = payload["filter"]
                .as_str()
                .filter(|k| matches!(*k, "all" | "created" | "modified" | "deleted"))
                .unwrap_or("all");
            state["changes_filter"] = json!(filter);
            return ClickOutcome {
                handled: true,
                rerender: true,
                rerender_side: true,
                ..ClickOutcome::unhandled()
            };
        }
        "graph-map-member-filter" => {
            let filter = payload["filter"]
                .as_str()
                .filter(|k| matches!(*k, "all" | "used" | "exposed" | "risk"))
                .unwrap_or("all");
            state["member_filter"] = json!(filter);
            return ClickOutcome {
                handled: true,
                rerender: true,
                rerender_side: true,
                ..ClickOutcome::unhandled()
            };
        }
        "graph-map-impact-focus" => {
            let path = payload["path"].as_str().unwrap_or("");
            let view = m::data(surface.view_key);
            let (next_state, needs_reload) = state_focus_file(state.clone(), path, &view);
            *state = next_state;
            let file_path = normalize_rel_path(strip_symbol_suffix(path));
            if surface.embedded && !file_path.is_empty() && !surface.pan_target_key.is_empty() {
                m::set(surface.pan_target_key, json!(file_path));
            }
            return ClickOutcome {
                handled: true,
                reload: needs_reload,
                rerender: !needs_reload,
                force_impact: true,
                clear_filter: Some(m::text(state, "filter")),
                ..ClickOutcome::unhandled()
            };
        }
        "graph-map-switch-service-file" => {
            let svc = m::text(payload, "service");
            if svc.is_empty() {
                return ClickOutcome::stay();
            }
            let file_path = normalize_rel_path(strip_symbol_suffix(&m::text(payload, "path")));
            return ClickOutcome {
                handled: true,
                reload: false,
                switch_service: Some((svc, file_path)),
                ..ClickOutcome::unhandled()
            };
        }
        "graph-map-test-path" => {
            if busy || map_failed {
                return ClickOutcome::stay();
            }
            let previous = state.clone();
            state["pick"] = json!({"path": payload["from"]});
            let Some(mut showing) = led(state.clone(), payload["to"].as_str().unwrap_or("")) else {
                return ClickOutcome::stay();
            };
            showing["test_path_return"] = previous;
            *state = showing;
            out.clear_filter = Some(String::new());
        }
        "graph-map-test-back" => {
            let previous = state["test_path_return"].take();
            if !previous.is_object() {
                return ClickOutcome::stay();
            }
            *state = previous;
            out.clear_filter = Some(m::text(state, "filter"));
            out.save_notests = Some(m::yes(&state["notests"]));
        }
        "graph-map-path" => {
            state["pick"] = json!({"path": payload["path"], "name": payload["name"]});
        }
        "graph-map-path-cancel" => {
            state["pick"] = V::Null;
        }
        "graph-map-cycle" => {
            if busy {
                return ClickOutcome::stay();
            }
            let place = payload.as_str().unwrap_or("");
            if state["cycle"].as_str() != Some(place) {
                if map_failed {
                    return ClickOutcome::stay();
                }
                let Some(cycle) = showing_cycle(state.clone(), place) else {
                    return ClickOutcome::stay();
                };
                *state = cycle;
            }
            out.clear_filter = Some(String::new());
        }
        "graph-map-cycle-switch" => {
            let Some(cycle) = switch_cycle(state.clone(), payload) else {
                return ClickOutcome::stay();
            };
            *state = cycle;
            out.clear_find = true;
            out.disable_find = Some(state["cycle_base"].is_object());
        }
        "graph-map-cycle-back" => {
            let previous = state["cycle_return"].take();
            if previous.is_object() {
                *state = previous;
                out.clear_filter = Some(m::text(state, "filter"));
                out.save_notests = Some(m::yes(&state["notests"]));
                out.save_fit = Some(m::yes(&state["fit"]));
            } else if state["cycle"].is_string() {
                return ClickOutcome::stay();
            }
        }
        "graph-map-path-swap" => {
            let mut ends = unfolded(state, "path");
            ends.reverse();
            state["path"] = json!(ends);
            state["focus"] = json!("");
        }
        // How much longer than the shortest a way of the path may be.
        "graph-map-longer" => {
            if busy || unfolded(state, "path").len() != 2 {
                return ClickOutcome::stay();
            }
            state["longer"] = json!(payload["longer"].as_u64().unwrap_or(0).min(LONGER));
        }
        "graph-map-shared" => {
            match sharing(state.clone(), payload) {
                Some(shown) => *state = shown,
                None => return ClickOutcome::stay(),
            }
            out.clear_filter = Some(m::text(state, "filter"));
        }
        "graph-map-module-select" | "graph-map-node" if m::yes(&state["modules"]) => {
            let path = payload.as_str().unwrap_or("");
            if !path.starts_with("module:") {
                return ClickOutcome::stay();
            }
            state["focus"] = payload.clone();
            if action == "graph-map-module-select" {
                state["filter"] = json!("");
                out.clear_filter = Some(String::new());
            }
        }
        "graph-map-module-file" => {
            state["modules"] = json!(false);
            state["scope"] = payload.clone();
            state["focus"] = json!("");
            state["filter"] = json!("");
            out.clear_filter = Some(String::new());
        }
        "graph-map-node" if state["pick"].is_object() => {
            match led(state.clone(), payload.as_str().unwrap_or("")) {
                Some(shown) => *state = shown,
                None => return ClickOutcome::stay(),
            }
            out.clear_filter = Some(String::new());
        }
        "graph-map-node" => {
            let path = payload.as_str().unwrap_or("");
            let wanted = payload.to_string();
            let again = dom::all(&format!(
                "#{} .graph-map-node[data-selected=\"true\"][data-open=\"true\"]",
                surface.canvas
            ))
            .iter()
            .any(|node| node.get_attribute("data-payload").as_deref() == Some(wanted.as_str()));
            if again {
                state["scope"] = json!(path);
                state["focus"] = json!("");
                state["filter"] = json!("");
                state["side_tab"] = json!("details");
                out.clear_filter = Some(String::new());
            } else if (m::yes(&state["shared"]) || state["pattern"].is_string())
                && state["focus"] == path
            {
                state["focus"] = json!("");
            } else {
                state["focus"] = json!(path);
                state["side_tab"] = json!("details");
                state["side_pinned"] = json!(true);
            }
        }
        "graph-map-open" => {
            *state = unfollowed(state.clone());
            state["scope"] = payload.clone();
            state["focus"] = json!("");
            state["filter"] = json!("");
            out.clear_filter = Some(String::new());
        }
        "graph-map-scenario" => {
            let Some(origin) = payload.as_str().filter(|origin| !origin.is_empty()) else {
                return ClickOutcome::stay();
            };
            if busy || map_failed || state["pick"].is_object() {
                return ClickOutcome::stay();
            }
            let back = state.clone();
            *state = unfollowed(state.clone());
            state["scenario"] = json!(origin);
            state["scenario_return"] = back;
            state["focus"] = json!("");
            state["filter"] = json!("");
            out.clear_filter = Some(String::new());
            out.clear_find = true;
        }
        "graph-map-scenario-back" => {
            let mut back = state["scenario_return"].take();
            if !back.is_object() {
                return ClickOutcome::stay();
            }
            for key in ["notests", "certain", "fit"] {
                back[key] = state[key].clone();
            }
            *state = back;
            out.clear_filter = Some(m::text(state, "filter"));
        }
        "graph-map-scenario-page" => {
            state["scenario_skip"] = payload.clone();
        }
        "graph-map-scenario-open" => {
            *state = unfollowed(state.clone());
            state["scope"] = payload["file"].clone();
            state["focus"] = if m::yes(&payload["symbol"]) {
                payload["path"].clone()
            } else {
                json!("")
            };
            state["filter"] = json!("");
            out.clear_filter = Some(String::new());
        }
        "graph-map-follow" => {
            *state = unfollowed(state.clone());
            state["follow"] = payload.clone();
            state["out"] = json!([payload]);
            state["into"] = json!([payload]);
            state["focus"] = json!("");
            state["filter"] = json!("");
            out.clear_filter = Some(String::new());
        }
        "graph-map-unfold" | "graph-map-fold" => {
            let key = if payload["side"] == "in" {
                "into"
            } else {
                "out"
            };
            let mut places = unfolded(state, key);
            let place = payload["path"].as_str().unwrap_or("");
            if action == "graph-map-fold" {
                places.retain(|other| other != place);
            } else {
                places.push(place.to_owned());
            }
            state[key] = json!(places);
        }
        "graph-map-link" => {
            let (from, to) = (
                payload["from"].as_str().unwrap_or(""),
                payload["to"].as_str().unwrap_or(""),
            );
            match between(state.clone(), from, to) {
                Some(selected) => *state = selected,
                None => {
                    return ClickOutcome {
                        handled: true,
                        reload: false,
                        open_link: Some((from.to_owned(), to.to_owned())),
                        ..ClickOutcome::unhandled()
                    };
                }
            }
        }
        "graph-map-spread" | "graph-map-gather" => {
            *state = spread(
                state.clone(),
                payload.as_str().unwrap_or(""),
                action == "graph-map-gather",
            );
        }
        "graph-map-go" => {
            let path = payload["path"].as_str().unwrap_or("");
            if let Some(file) = payload["open"].as_str() {
                *state = unfollowed(state.clone());
                state["scope"] = json!(file);
                state["filter"] = json!("");
            } else if !m::yes(&payload["inside"]) {
                *state = unfollowed(state.clone());
                state["target"] = json!("");
                state["compare_base"] = json!("");
                state["scope"] = json!(parent(path));
                state["filter"] = json!("");
            }
            state["focus"] = json!(path);
        }
        "graph-map-shared-page" => {
            *state = paged(state.clone(), payload.as_u64());
        }
        "graph-map-pattern" => {
            match showing_pattern(state.clone(), payload.as_str().unwrap_or("")) {
                Some(shown) => *state = shown,
                None => return ClickOutcome::stay(),
            }
            out.clear_filter = Some(String::new());
        }
        "graph-map-pattern-back" => {
            match left_pattern(state.clone()) {
                Some(previous) => *state = previous,
                None => return ClickOutcome::stay(),
            }
            out.clear_filter = Some(m::text(state, "filter"));
        }
        "graph-map-pattern-kind" => {
            let kind = payload.as_str().unwrap_or("");
            let mut open = unfolded(state, "kinds");
            if open.iter().any(|known| known == kind) {
                open.retain(|known| known != kind);
            } else {
                open.push(kind.to_owned());
            }
            state["kinds"] = json!(open);
        }
        _ => return ClickOutcome::unhandled(),
    }
    out
}
/// Formats and appends a service-scoped dependency-map context block onto the `#system-task` draft.
pub fn append_system_task_draft(existing: &str, service_id: &str, context_text: &str) -> String {
    let body = context_text.trim();
    if body.is_empty() {
        return existing.to_owned();
    }
    let svc_label = if service_id.is_empty() || service_id == "__system__" {
        "__system__ (root workspace)".to_owned()
    } else {
        format!("service `{service_id}`")
    };
    let block = format!("### Code Map Context ({svc_label})\n{body}");
    let trimmed_existing = existing.trim_end();
    if trimmed_existing.is_empty() {
        block
    } else if trimmed_existing.contains(&block) {
        existing.to_owned()
    } else {
        format!("{trimmed_existing}\n\n{block}")
    }
}
pub(in crate::graph_map) fn attach_map_context_to_system_task(
    service_id: &str,
    context_text: &str,
) -> bool {
    if context_text.trim().is_empty() {
        return false;
    }
    let next = append_system_task_draft(&dom::value("system-task"), service_id, context_text);
    dom::set_value("system-task", &next);
    true
}
pub(in crate::graph_map) fn switch_surface_to_service_file(
    surface: MapSurface,
    next_service: &str,
    file_path: &str,
) -> bool {
    let cur_scope = surface_scope(surface);
    if !cur_scope.is_system() || next_service.trim().is_empty() {
        return false;
    }
    let next_scope = cur_scope.with_service(next_service);
    set_surface_scope(surface, next_scope);
    m::next(surface.map_epoch);
    m::next(surface.changes_epoch);
    m::next(surface.impact_epoch);
    m::take(surface.changes_key);
    m::take(surface.impact_key);
    m::set(surface.view_key, V::Null);
    let prev = surface_state(surface);
    let prev_target = m::text(&prev, "target");
    let kept_target = if prev_target.is_empty()
        || matches!(prev_target.as_str(), "work" | "uncommitted")
        || prev_target.starts_with("sysrun:")
    {
        prev_target
    } else {
        String::new()
    };
    let mut next_state = fresh(&kept_target);
    next_state["notests"] = prev["notests"].clone();
    next_state["certain"] = prev["certain"].clone();
    next_state["fit"] = prev["fit"].clone();
    next_state["patterns"] = prev["patterns"].clone();
    next_state["launch"] = prev["launch"].clone();
    next_state["compare_launch"] = prev["compare_launch"].clone();
    next_state["run_id"] = prev["run_id"].clone();
    let prev_agent = m::text(&prev, "agent");
    if prev_agent.starts_with("service-") {
        next_state["agent"] = json!(if next_service == "__system__" {
            String::new()
        } else {
            format!("service-{next_service}")
        });
    } else {
        next_state["agent"] = prev["agent"].clone();
    }
    let clean_file = normalize_rel_path(strip_symbol_suffix(file_path));
    if !clean_file.is_empty() {
        let folder = parent(&clean_file);
        if !folder.is_empty() {
            let mut acc = String::new();
            for part in folder.split('/').filter(|p| !p.is_empty()) {
                if !acc.is_empty() {
                    acc.push('/');
                }
                acc.push_str(part);
                next_state = spread(next_state, &acc, false);
            }
        }
        next_state["focus"] = json!(clean_file.clone());
        next_state["side_tab"] = json!("details");
        next_state["side_pinned"] = json!(true);
        if surface.embedded && !surface.pan_target_key.is_empty() {
            m::set(surface.pan_target_key, json!(clean_file));
        }
    }
    save_surface_state(surface, &next_state);
    true
}

/// Clears sub-mode precedence keys so switching top-level modal lens tabs does
/// not leave `build_scoped_map_path` trapped in an earlier cycle, pattern,
/// scenario, traced/followed path, isolation, or pagination offset.
pub(in crate::graph_map) fn clear_lens_submodes(state: &mut V) {
    let taken = std::mem::replace(state, V::Null);
    let mut cleared = unfollowed(taken);
    cleared["traced_path"] = V::Null;
    cleared["isolate_active"] = json!(false);
    cleared["offset"] = json!(0);
    *state = cleared;
}

/// Applies top-level modal lens selection (`dependencies`, `changes`,
/// `architecture`, `tests`, `runs`) onto `state`, returning `true` if `lens`
/// is recognized.
pub(in crate::graph_map) fn apply_modal_lens_switch(state: &mut V, lens: &str) -> bool {
    match lens {
        "dependencies" => {
            clear_lens_submodes(state);
            state["target"] = json!("");
            state["compare_base"] = V::Null;
            state["run_id"] = V::Null;
            state["history"] = json!(false);
            state["patterns"] = json!(false);
            state["modules"] = json!(false);
            state["lens_tests"] = json!(false);
            state["lens_runs"] = json!(false);
            state["failing"] = json!([]);
            state["side_tab"] = json!("details");
            true
        }
        "changes" => {
            clear_lens_submodes(state);
            state["history"] = json!(false);
            state["patterns"] = json!(false);
            state["modules"] = json!(false);
            state["lens_tests"] = json!(false);
            state["lens_runs"] = json!(false);
            state["failing"] = json!([]);
            state["side_tab"] = json!("changes");
            if m::text(state, "target").is_empty() {
                state["target"] = json!("work");
            }
            true
        }
        "architecture" => {
            clear_lens_submodes(state);
            state["target"] = json!("");
            state["compare_base"] = V::Null;
            state["run_id"] = V::Null;
            state["history"] = json!(false);
            state["patterns"] = json!(true);
            state["lens_tests"] = json!(false);
            state["lens_runs"] = json!(false);
            state["failing"] = json!([]);
            state["side_tab"] = json!("analysis");
            true
        }
        "tests" => {
            clear_lens_submodes(state);
            state["history"] = json!(false);
            state["patterns"] = json!(false);
            state["modules"] = json!(false);
            state["lens_tests"] = json!(true);
            state["lens_runs"] = json!(false);
            state["compare_base"] = V::Null;
            state["run_id"] = V::Null;
            state["side_tab"] = json!("analysis");
            if m::text(state, "target").is_empty() {
                state["target"] = json!("work");
            }
            true
        }
        "runs" => {
            clear_lens_submodes(state);
            state["target"] = json!("");
            state["compare_base"] = V::Null;
            state["lens_runs"] = json!(true);
            // `active_modal_lens` maps `history=1` to `"runs"`, and when `target`
            // is empty, `history=1` is the backend query flag that populates
            // `history_side(view)` in the Analysis tab instead of a plain map.
            state["history"] = json!(true);
            state["patterns"] = json!(false);
            state["modules"] = json!(false);
            state["lens_tests"] = json!(false);
            state["failing"] = json!([]);
            state["side_tab"] = json!("analysis");
            true
        }
        _ => false,
    }
}

pub async fn event(
    kind: &str,
    id: &str,
    action: &str,
    payload: &V,
    value: &str,
    checked: bool,
) -> bool {
    if kind == "click" && action == "graph-map-open-memory" {
        crate::knowledge::open_from_omnigraph(
            payload["scope"].as_str().unwrap_or("brain"),
            payload["project_id"].as_str().unwrap_or(""),
            payload["target_id"].as_str().unwrap_or(""),
        )
        .await;
        return true;
    }
    // The key of the links is the same in every picture.
    if kind == "click"
        && matches!(
            action,
            "graph-kind-toggle" | "graph-kind-preset" | "graph-transitive-toggle" | "graph-cone"
        )
    {
        switch_kinds(action, payload);
        return true;
    }
    // The failing tests of a change are read from their field when they
    // are asked to rank it, or dropped.
    if kind == "click" && action == "graph-map-failing" && payload["surface"] == "modal" {
        let mut asked = state();
        asked["failing"] = if m::yes(&payload["clear"]) {
            json!([])
        } else {
            json!(failing_lines(&dom::value("graph-map-failing")))
        };
        m::set("graph-map", asked);
        load(false).await;
        return true;
    }
    if id.starts_with("session-graph-")
        || id.starts_with("system-member-graph-")
        || id.starts_with("system-change-graph-")
        || action.starts_with("session-graph-")
        || action.starts_with("system-member-graph-")
        || action.starts_with("system-change-graph-")
        || matches!(
            payload["surface"].as_str(),
            Some("session" | "system-member" | "system-change")
        )
    {
        return false;
    }
    if kind == "click"
        && !open()
        && (action.starts_with("graph-map-")
            || matches!(action, "graph-side-tab" | "graph-changes-filter"))
        && action != "graph-map-shell-event"
    {
        return false;
    }
    let mut state = state();
    // A pull request, a branch or a commit is worked out before it is mapped.
    let mut chosen = false;
    // A place a search found is brought into view once it is drawn.
    let mut jumped = false;
    let busy = || {
        dom::get("graph-map-canvas")
            .and_then(|canvas| canvas.get_attribute("aria-busy"))
            .as_deref()
            == Some("true")
    };
    let map_failed =
        || dom::get("graph-map-error").is_some_and(|error| !error.has_attribute("hidden"));
    if kind == "click"
        && open()
        && matches!(
            action,
            "graph-map-node" | "graph-map-go" | "graph-map-link" | "graph-map-spread"
        )
        && m::yes(&surface_drag_data(MODAL_SURFACE)["suppress_click"])
    {
        clear_surface_drag_suppress(MODAL_SURFACE);
        return true;
    }
    if kind == "click"
        && state["cycle"].is_string()
        && (matches!(action, "graph-map-node" | "graph-map-go" | "graph-map-link")
            && (busy() || map_failed())
            || action == "graph-map-cycle-switch" && busy())
    {
        return true;
    }
    match (kind, id, action) {
        ("click", "graph-view" | "graph-changes", _) => {
            set_surface_scope(
                MODAL_SURFACE,
                GraphTargetScope::single(dom::value("project")),
            );
            dom::disabled("graph-find-search", false);
            let changes = id == "graph-changes";
            state = anew(if changes { "work" } else { "" });
            if changes {
                state["side_tab"] = json!("changes");
            }
            m::set("graph-map", state.clone());
            dom::set_value("graph-map-filter", "");
            dom::set_checked("graph-map-whole", false);
            // A map begun anew has nothing searched in it.
            m::next("graph-find");
            dom::cancel_timer("graph-find-search");
            m::set("graph-find", V::Null);
            dom::set_value("graph-find-search", "");
            show_found();
            dom::error("graph-map-error", "");
            dom::hidden("graph-map-progress", true);
            dom::text("graph-map-zoom-level", "100%");
            blank();
            dom::text("graph-map-chosen", "");
            dom::text(
                "graph-map-note",
                if changes {
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
            if changes {
                // The open pull requests say what the branch is stacked on.
                place().await;
            } else {
                spawn_local(place());
            }
        }
        ("change", "graph-map-service", _) => {
            if matches!(value, "__topology__" | "topology") {
                dom::cancel_timer("graph-map-poll");
                shut();
                close_found();
                dom::dialog("graph-map", false);
                if let Some(ws) = dom::get("system-map-workspace") {
                    if !ws.has_attribute("tabindex") {
                        let _ = ws.set_attribute("tabindex", "-1");
                    }
                }
                dom::focus("system-map-workspace");
                return true;
            }
            if !value.is_empty() && switch_surface_to_service_file(MODAL_SURFACE, value, "") {
                dom::set_value("graph-map-filter", "");
                dom::set_checked("graph-map-whole", false);
                m::next("graph-find");
                dom::cancel_timer("graph-find-search");
                m::set("graph-find", V::Null);
                dom::set_value("graph-find-search", "");
                show_found();
                dom::error("graph-map-error", "");
                blank();
                dom::text("graph-map-chosen", "");
                dom::text("graph-map-note", "Reading the graph…");
                shut();
                reset_commit_dock_for_scope();
                list();
                spawn_local(place());
                load(false).await;
            }
            return true;
        }
        ("click", _, "graph-map-shell-event") => {
            show_shell_event(payload).await;
            return true;
        }
        ("click", "graph-shell-event-close", _) => {
            m::next("graph-shell-event");
            dom::dialog("graph-shell-event", false);
            return true;
        }
        ("click", "graph-map-close", _) => {
            dom::cancel_timer("graph-map-poll");
            dom::cancel_timer("graph-commit-graph-search");
            m::next(MODAL_SURFACE.map_epoch);
            m::next(MODAL_SURFACE.changes_epoch);
            m::next(MODAL_SURFACE.impact_epoch);
            m::next("graph-commit-graph");
            dom::attr("graph-map-canvas", "aria-busy", "false");
            dom::hidden("graph-map-progress", true);
            shut();
            close_found();
            dom::dialog("graph-map", false);
            return true;
        }
        ("input", "graph-find-search", _) => {
            let mut finder = finder();
            let empty = value.trim().is_empty();
            finder["query"] = json!(value);
            finder["open"] = json!(!empty);
            finder["loading"] = json!(!empty);
            if empty {
                // Nothing is typed: nothing is found, and nothing is asked.
                for (key, cleared) in [
                    ("rows", json!([])),
                    ("found", json!(0)),
                    ("asked", json!("")),
                    ("for", json!("")),
                    ("active", json!(0)),
                    ("failed", V::Null),
                ] {
                    finder[key] = cleared;
                }
                dom::cancel_timer("graph-find-search");
                m::next("graph-find");
            }
            m::set("graph-find", finder);
            show_found();
            if !empty {
                // The server searches once the typing pauses.
                dom::later("graph-find-search", 160, || {
                    spawn_local(seek());
                });
            }
            return true;
        }
        ("focusin", "graph-find-search", _) => {
            // Back in the field, what was found is offered again, and it is
            // sought anew where the map has gone to another graph since.
            shut();
            let mut finder = finder();
            if m::yes(&finder["open"]) || find_path(&finder).is_none() {
                return true;
            }
            let stale = !answered(&finder);
            finder["open"] = json!(true);
            if stale {
                for (key, cleared) in [
                    ("rows", json!([])),
                    ("found", json!(0)),
                    ("for", json!("")),
                    ("failed", V::Null),
                    ("loading", json!(true)),
                ] {
                    finder[key] = cleared;
                }
            }
            m::set("graph-find", finder);
            show_found();
            if stale {
                dom::cancel_timer("graph-find-search");
                seek().await;
            }
            return true;
        }
        ("click", _, "graph-find-choose") => {
            let mut finder = finder();
            let rows = m::array(&finder["rows"]);
            let Some(row) = payload.as_u64().and_then(|at| rows.get(at as usize)) else {
                return true;
            };
            // The mark stays on what was chosen, so that the list, opened
            // again, goes on from there.
            finder["active"] = payload.clone();
            m::set("graph-find", finder);
            close_found();
            if state["pick"].is_object() {
                // The other end of a path is being chosen: this place is it.
                match led(state.clone(), row["path"].as_str().unwrap_or("")) {
                    Some(shown) => state = shown,
                    None => return true,
                }
                dom::set_value("graph-map-filter", "");
            } else {
                // Whether the picture that is open draws the place (DOM or Dense Canvas2D).
                let wanted_raw = row["path"].as_str().unwrap_or("");
                let wanted_json = row["path"].to_string();
                let drawn_in_cache = SURFACE_PLAN_CACHE.with(|cache| {
                    cache
                        .borrow()
                        .get("graph-map-canvas")
                        .is_some_and(|entry| entry.node_keys.iter().any(|k| k == wanted_raw))
                });
                let drawn = drawn_in_cache
                    || dom::all("#graph-map-canvas .graph-map-node")
                        .iter()
                        .any(|node| {
                            node.get_attribute("data-payload").as_deref()
                                == Some(wanted_json.as_str())
                        });
                state = sought(state, row, drawn);
                jumped = true;
            }
        }
        ("click", "graph-pick-open", _) => {
            let mut pick = picker();
            if m::yes(&pick["open"]) {
                shut();
                return true;
            }
            for (key, value) in [
                ("open", json!(true)),
                ("query", json!("")),
                ("active", json!(0)),
                ("rows", json!([])),
                ("more", json!(false)),
                ("failed", V::Null),
            ] {
                pick[key] = value;
            }
            dom::set_value("graph-pick-search", "");
            dom::set_value("graph-pick-state", &m::text(&pick, "state"));
            html(
                "graph-pick-scope",
                &scopes(
                    &m::data("graph-map-place"),
                    &m::data("graph-pick-branches"),
                    &m::text(&pick, "scope"),
                ),
            );
            m::set("graph-pick", pick);
            show();
            dom::focus("graph-pick-search");
            spawn_local(histories());
            fetch(false).await;
            return true;
        }
        ("click", "graph-pick-close", _) => {
            shut();
            dom::focus("graph-pick-open");
            return true;
        }
        (
            "click",
            "graph-pick-tab-commits" | "graph-pick-tab-pulls" | "graph-pick-tab-branches",
            _,
        ) => {
            let mut pick = picker();
            pick["tab"] = json!(id.rsplit('-').next().unwrap_or("commits"));
            // Each list is searched in its own words.
            pick["query"] = json!("");
            pick["rows"] = json!([]);
            pick["active"] = json!(0);
            pick["more"] = json!(false);
            pick["failed"] = V::Null;
            m::set("graph-pick", pick);
            dom::set_value("graph-pick-search", "");
            show();
            dom::focus("graph-pick-search");
            fetch(false).await;
            return true;
        }
        ("input", "graph-pick-search", _) => {
            let mut pick = picker();
            pick["query"] = json!(value);
            pick["active"] = json!(0);
            let local = pick["tab"] == "branches";
            m::set("graph-pick", pick);
            if local {
                show();
            } else {
                // The server searches once the typing pauses.
                dom::later("graph-pick-search", 220, || {
                    spawn_local(fetch(false));
                });
            }
            return true;
        }
        ("change", "graph-pick-scope" | "graph-pick-state", _) => {
            let mut pick = picker();
            pick[if id == "graph-pick-scope" {
                "scope"
            } else {
                "state"
            }] = json!(value);
            m::set("graph-pick", pick);
            fetch(false).await;
            return true;
        }
        ("click", _, "graph-pick-more") => {
            fetch(true).await;
            return true;
        }
        ("click", _, "graph-pick-choose") => {
            let Some(target) = payload.as_str() else {
                return true;
            };
            let prev_agent = m::text(&state, "agent");
            shut();
            state = anew(target);
            if target.starts_with("sysrun:") && !prev_agent.is_empty() {
                state["agent"] = json!(prev_agent);
            }
            chosen = true;
        }
        ("click", "graph-commit-graph-toggle", _) | ("click", _, "graph-commit-graph-toggle") => {
            let mut dock = commit_dock();
            if m::yes(&dock["open"]) {
                shut_commit_dock();
                return true;
            }
            let scope_key = surface_scope(MODAL_SURFACE).scope_key();
            if m::text(&dock, "scope_key") != scope_key {
                dock = fresh_commit_dock(&scope_key);
            }
            dock["open"] = json!(true);
            m::set("graph-commit-graph", dock.clone());
            dom::set_value("graph-commit-graph-search", &m::text(&dock, "query"));
            html(
                "graph-commit-graph-scope",
                &scopes(
                    &m::data("graph-map-place"),
                    &m::data("graph-pick-branches"),
                    &m::text(&dock, "scope"),
                ),
            );
            show_commit_dock();
            dom::focus("graph-commit-graph-list");
            if m::array(&m::data("graph-pick-branches")).is_empty() {
                spawn_local(dock_histories());
            }
            if m::array(&dock["rows"]).is_empty() {
                fetch_commit_dock(false).await;
            }
            return true;
        }
        ("click", "graph-commit-graph-close", _) | ("click", _, "graph-commit-graph-close") => {
            shut_commit_dock();
            dom::focus("graph-commit-graph-toggle");
            return true;
        }
        ("input", "graph-commit-graph-search", _) => {
            let mut dock = commit_dock();
            dock["query"] = json!(value);
            dock["active"] = V::Null;
            m::set("graph-commit-graph", dock);
            dom::later("graph-commit-graph-search", 220, || {
                spawn_local(fetch_commit_dock(false));
            });
            return true;
        }
        ("change", "graph-commit-graph-scope", _) => {
            let mut dock = commit_dock();
            dock["scope"] = json!(value);
            dock["active"] = V::Null;
            m::set("graph-commit-graph", dock);
            fetch_commit_dock(false).await;
            return true;
        }
        ("click", _, "graph-commit-graph-more") => {
            fetch_commit_dock(true).await;
            return true;
        }
        ("click", _, "graph-commit-range-clear") => {
            let mut dock = commit_dock();
            if dock.is_object() {
                dock["range_base"] = V::Null;
                dock["range_head"] = V::Null;
                dock["range_anchor"] = V::Null;
                m::set("graph-commit-graph", dock);
                show_commit_dock();
            }
            state["compare_base"] = V::Null;
            chosen = true;
        }
        ("click", _, "graph-commit-select" | "graph-commit-reset") => {
            let Some(target) = commit_action_target(action, payload) else {
                return true;
            };
            let shift_range =
                action == "graph-commit-select" && m::yes(&m::data("graph-commit-shift"));
            m::set("graph-commit-shift", json!(false));
            let mut dock = commit_dock();
            if shift_range && dock.is_object() {
                let rows = m::array(&dock["rows"]);
                let prev_anchor = m::text(&dock, "range_anchor");
                let prev_target = m::text(&state, "target");
                let anchor = if !prev_anchor.is_empty() {
                    prev_anchor
                } else {
                    prev_target
                };
                if let Some((base_sha, head_sha)) =
                    crate::git_commit_graph::resolve_commit_range_selection(
                        &rows,
                        (!anchor.is_empty()).then_some(anchor.as_str()),
                        &target,
                    )
                {
                    dock["range_base"] = json!(base_sha.clone());
                    dock["range_head"] = json!(head_sha.clone());
                    dock["range_anchor"] = json!(anchor);
                    m::set("graph-commit-graph", dock);
                    show_commit_dock();
                    state = anew(&format!("commit:{head_sha}"));
                    state["compare_base"] = json!(base_sha);
                    chosen = true;
                } else {
                    dock["range_anchor"] = json!(target.clone());
                    dock["range_base"] = V::Null;
                    dock["range_head"] = V::Null;
                    dock["active"] = V::Null;
                    m::set("graph-commit-graph", dock);
                    shut();
                    state = anew(&target);
                    chosen = true;
                }
            } else {
                if dock.is_object() {
                    dock["active"] = V::Null;
                    dock["range_anchor"] = json!(target.clone());
                    dock["range_base"] = V::Null;
                    dock["range_head"] = V::Null;
                    m::set("graph-commit-graph", dock);
                }
                shut();
                state = anew(&target);
                chosen = true;
            }
        }
        ("click", "graph-map-display-toggle", _) => {
            if let Some(pop) = dom::get("graph-map-display-popover") {
                let next_hidden = !pop.has_attribute("hidden");
                dom::hidden("graph-map-display-popover", next_hidden);
                dom::attr(
                    "graph-map-display-toggle",
                    "aria-expanded",
                    if next_hidden { "false" } else { "true" },
                );
            }
            return true;
        }
        (
            "click",
            id @ ("graph-map-lens-dependencies"
            | "graph-map-lens-changes"
            | "graph-map-lens-architecture"
            | "graph-map-lens-tests"
            | "graph-map-lens-runs"),
            _,
        ) => {
            let lens = id.strip_prefix("graph-map-lens-").unwrap_or("dependencies");
            chosen = apply_modal_lens_switch(&mut state, lens);
            if chosen {
                if lens == "architecture" {
                    dom::save(PATTERNS, Some("1"));
                }
                sync_modal_lens_controls_from_state(&state);
            }
        }
        ("click", "graph-map-rebuild", _) => {
            let scope = surface_scope(MODAL_SURFACE);
            if !scope.is_empty() {
                spawn_local(async move {
                    if crate::graph::trigger_rebuild_for_scope(scope, "graph-map")
                        .await
                        .is_ok()
                    {
                        load(false).await;
                    }
                });
            }
            return true;
        }
        ("click", "graph-map-stale-refresh", _) => {
            dom::hidden("graph-map-stale-banner", true);
            load(false).await;
            return true;
        }
        ("click", "graph-map-stale-dismiss", _) => {
            dom::hidden("graph-map-stale-banner", true);
            return true;
        }
        ("click", "graph-map-reset-pins", _) => {
            let pin_key = surface_pin_key(MODAL_SURFACE, &state);
            push_pin_undo_snapshot(MODAL_SURFACE, &pin_key);
            graph_viewport::clear_pins(&pin_key);
            sync_history_and_pin_controls(MODAL_SURFACE, &state);
            rerender_surface_preserving_camera(MODAL_SURFACE);
            graph_viewport::announce_live("graph-map-live-announce", "Reset pinned layout");
            return true;
        }
        ("click", "graph-map-pin-undo", _) => {
            return undo_surface_pins(MODAL_SURFACE);
        }
        ("click", _, "graph-map-unpin-node") => {
            let node_key = payload
                .as_str()
                .map(str::to_owned)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| m::text(&state, "focus"));
            return unpin_surface_node(MODAL_SURFACE, &node_key);
        }
        ("click", "graph-map-nav-back", _) => {
            return navigate_surface_history(MODAL_SURFACE, -1, None);
        }
        ("click", "graph-map-nav-forward", _) => {
            return navigate_surface_history(MODAL_SURFACE, 1, None);
        }
        ("click", _, "graph-map-history-jump") => {
            if let Some(idx) = payload.as_u64().map(|n| n as usize) {
                return navigate_surface_history(MODAL_SURFACE, 0, Some(idx));
            }
            return false;
        }
        ("click", "graph-map-isolate-toggle", _) => {
            let next = !m::yes(&state["isolate_active"]);
            let depth = dom::value("graph-map-isolate-depth")
                .trim()
                .parse::<u64>()
                .unwrap_or(2)
                .clamp(1, 3);
            let focus = m::text(&state, "focus");
            let active = next && !focus.is_empty();
            state["isolate_active"] = json!(active);
            state["isolate_depth"] = json!(depth);
            save_surface_state(MODAL_SURFACE, &state);
            load(false).await;
            graph_viewport::announce_live(
                "graph-map-live-announce",
                if active {
                    "Isolated neighborhood"
                } else if next {
                    "Select a node first to isolate its neighborhood"
                } else {
                    "Cleared isolation"
                },
            );
            return true;
        }
        ("change", "graph-map-isolate-depth", _) => {
            let depth = value.trim().parse::<u64>().unwrap_or(2).clamp(1, 3);
            state["isolate_depth"] = json!(depth);
            state["isolate_active"] = json!(!m::text(&state, "focus").is_empty());
            save_surface_state(MODAL_SURFACE, &state);
            load(false).await;
            return true;
        }
        ("click", "graph-map-path-trace", _) => {
            let from_q = dom::value("graph-map-path-from");
            let to_q = dom::value("graph-map-path-to");
            state["traced_path"] = json!({ "from": from_q.trim(), "to": to_q.trim() });
            save_surface_state(MODAL_SURFACE, &state);
            rerender_surface_preserving_camera(MODAL_SURFACE);
            graph_viewport::announce_live("graph-map-live-announce", "Updated path trace");
            return true;
        }
        ("click", "graph-map-path-clear", _) => {
            state["traced_path"] = V::Null;
            save_surface_state(MODAL_SURFACE, &state);
            rerender_surface_preserving_camera(MODAL_SURFACE);
            graph_viewport::announce_live("graph-map-live-announce", "Cleared path trace");
            return true;
        }
        ("click", _, "graph-map-open-omnigraph") => {
            let proj = m::text(payload, "project_id");
            let p = m::text(payload, "path");
            if !p.is_empty() {
                dom::dialog("graph-map", false);
                crate::omnigraph::focus_context_for_file(
                    (!proj.is_empty()).then_some(proj.as_str()),
                    &p,
                )
                .await;
            }
            return true;
        }
        ("change", "graph-map-target", _) => {
            let prev_agent = m::text(&state, "agent");
            shut();
            state = anew(value);
            if value.starts_with("sysrun:") && !prev_agent.is_empty() {
                state["agent"] = json!(prev_agent);
            }
            chosen = true;
        }
        ("change", "graph-map-launch", _) => {
            state["launch"] = json!(value);
            if value.is_empty() || state["compare_launch"] == value {
                state["compare_launch"] = V::Null;
            }
        }
        ("change", "graph-map-compare-launch", _) => {
            state["compare_launch"] = json!(value);
        }
        ("change", "graph-map-agent", _) => {
            state["agent"] = json!(value);
        }
        ("click", _, "graph-map-attach") => {
            if !open() || busy() || !m::data("graph-map-attachment")["text"].is_string() {
                return true;
            }
            let context = m::data("graph-map-attachment");
            let scope = GraphTargetScope::from_json(&context["scope"])
                .unwrap_or_else(|| surface_scope(MODAL_SURFACE));
            let Some(text) = context["text"].as_str() else {
                return true;
            };
            if let GraphTargetScope::SystemService { service_id, .. } = &scope {
                if attach_map_context_to_system_task(service_id, text) {
                    dom::cancel_timer("graph-map-poll");
                    shut();
                    close_found();
                    dom::dialog("graph-map", false);
                    dom::focus("system-task");
                }
                return true;
            }
            if crate::session::attach_map_context(&m::text(&context, "project"), text) {
                dom::cancel_timer("graph-map-poll");
                shut();
                close_found();
                dom::dialog("graph-map", false);
                if m::read(|s| s.view != "sessions") {
                    crate::session::open_view("sessions", true).await;
                }
                dom::focus("prompt");
            }
            return true;
        }
        ("click", "graph-link-close", _) => {
            dom::dialog("graph-link", false);
            return true;
        }
        ("click", _, "graph-link-node" | "graph-link-open") => {
            let key = payload.as_str().unwrap_or("");
            let mut link = looked();
            if action == "graph-link-node" && link["chosen"] != key {
                link["chosen"] = json!(key);
                m::set("graph-link", link);
                draw_link();
                return true;
            }
            if at_base(&link["view"]) {
                return true;
            }
            // Selected again, or asked for: its file opens on it in the map.
            let (names, _) = link_names(&link["view"]);
            let Some((_, file)) = names.iter().find(|(name, _)| name["key"] == key) else {
                return true;
            };
            dom::dialog("graph-link", false);
            let target_surface = surface_by_name(link["surface"].as_str().unwrap_or("modal"));
            if target_surface.embedded {
                let mut emb = unfollowed(surface_state(target_surface));
                emb["scope"] = file["path"].clone();
                emb["focus"] = json!(key);
                emb["side_tab"] = json!("details");
                emb["side_pinned"] = json!(true);
                emb["filter"] = json!("");
                dom::set_value(&format!("{}-filter", target_surface.id_prefix), "");
                save_surface_state(target_surface, &emb);
                ensure_surface_impact(target_surface, &emb, None, true);
                load_embedded_surface(target_surface, false).await;
                return true;
            }
            state = normalize_map_state(unfollowed(state));
            state["scope"] = file["path"].clone();
            state["focus"] = json!(key);
            state["side_tab"] = json!("details");
            state["filter"] = json!("");
            dom::set_value("graph-map-filter", "");
        }
        ("change", "graph-map-certain", _) => {
            // Another set of links: what two places share begins anew.
            state["certain"] = json!(checked);
            state["skip"] = json!([]);
        }
        ("change", "graph-map-whole", _) => {
            state["whole"] = json!(checked);
            if checked && !m::text(&state, "scope").is_empty() {
                state = normalize_map_state(unfollowed(state));
                state["scope"] = json!("");
                state["filter"] = json!("");
                dom::set_value("graph-map-filter", "");
            }
        }
        ("change", "graph-map-no-incoming", _) => {
            state["no_incoming"] = json!(checked);
        }
        ("change", "graph-map-lod", _) => {
            let clean = value.trim().to_ascii_lowercase();
            if matches!(clean.as_str(), "crate" | "folder" | "file" | "symbol") {
                state["lod"] = json!(clean);
            } else {
                state["lod"] = V::Null;
                state["auto_lod"] = json!(true);
            }
            state["offset"] = json!(0);
        }
        ("change", "graph-map-min-weight", _) => {
            let parsed = value
                .trim()
                .parse::<u64>()
                .ok()
                .filter(|&w| w >= 1)
                .map(|w| w.min(1_000_000));
            state["min_weight"] = parsed.map_or(V::Null, |w| json!(w));
            state["offset"] = json!(0);
        }
        ("change", "graph-map-modules", _) => {
            state = unfollowed(state);
            state["modules"] = json!(checked);
            state["focus"] = json!("");
            state["pick"] = V::Null;
            state["filter"] = json!("");
            dom::set_value("graph-map-filter", "");
            close_found();
        }
        ("change", "graph-map-history", _) => {
            state["history"] = json!(checked);
            if !checked {
                state["lens_runs"] = json!(false);
            }
        }
        ("change", "graph-map-notests", _) => {
            // The choice is the viewer's and holds for the next map too.
            state["notests"] = json!(checked);
            state["skip"] = json!([]);
            dom::save(NOTESTS, checked.then_some("1"));
        }
        ("change", "graph-map-patterns", _) => {
            // The choice is the viewer's and holds for the next map too.
            state["patterns"] = json!(checked);
            dom::save(PATTERNS, checked.then_some("1"));
        }
        ("change", "graph-map-fit", _) => {
            // So does this one, and it asks the server for nothing: the
            // picture that is there is drawn at another scale.
            state["fit"] = json!(checked);
            state["zoom"] = V::Null;
            dom::save(FIT, checked.then_some("1"));
            m::set("graph-map", state);
            let mut cam = load_surface_camera(MODAL_SURFACE);
            cam.fit_mode = checked;
            save_surface_camera(MODAL_SURFACE, &cam);
            fit();
            return true;
        }
        ("click", "graph-map-zoom-in", _) => {
            zoom_surface_camera_at(MODAL_SURFACE, ZOOM_STEP, None);
            return true;
        }
        ("click", "graph-map-zoom-out", _) => {
            zoom_surface_camera_at(MODAL_SURFACE, 1.0 / ZOOM_STEP, None);
            return true;
        }
        ("click", "graph-map-zoom-level", _) => {
            reset_surface_camera_100(MODAL_SURFACE);
            return true;
        }
        ("click", "graph-link-zoom-in", _) => {
            zoom_scroll_canvas("graph-link-canvas", ZOOM_STEP, None);
            return true;
        }
        ("click", "graph-link-zoom-out", _) => {
            zoom_scroll_canvas("graph-link-canvas", 1.0 / ZOOM_STEP, None);
            return true;
        }
        ("click", "graph-link-zoom-level", _) => {
            reset_scroll_canvas_zoom("graph-link-canvas");
            return true;
        }
        ("input", "graph-map-filter", _) => {
            // What a filter leaves begins with its nearest.
            state["filter"] = json!(value);
            state["scenario_skip"] = json!(0);
            state["skip"] = json!([]);
        }
        ("click", _, _) => {
            let outcome = apply_click_action(
                &mut state,
                action,
                payload,
                MODAL_SURFACE,
                busy(),
                map_failed(),
            );
            if !outcome.handled {
                return false;
            }
            if let Some((svc, file_path)) = outcome.switch_service {
                if switch_surface_to_service_file(MODAL_SURFACE, &svc, &file_path) {
                    dom::set_value("graph-map-filter", "");
                    dom::set_checked("graph-map-whole", false);
                    close_found();
                    dom::error("graph-map-error", "");
                    blank();
                    dom::text("graph-map-note", "Reading the graph…");
                    shut();
                    list();
                    let switched = self::state();
                    ensure_surface_impact(MODAL_SURFACE, &switched, None, true);
                    spawn_local(place());
                    load(false).await;
                }
                return true;
            }
            if let Some(filter) = outcome.clear_filter {
                dom::set_value("graph-map-filter", &filter);
            }
            if outcome.clear_find {
                close_found();
                dom::cancel_timer("graph-map-poll");
            }
            if let Some(disabled) = outcome.disable_find {
                dom::disabled("graph-find-search", disabled);
            }
            if let Some(notests) = outcome.save_notests {
                dom::save(NOTESTS, notests.then_some("1"));
            }
            if let Some(save_fit) = outcome.save_fit {
                dom::save(FIT, save_fit.then_some("1"));
            }
            if let Some((from, to)) = outcome.open_link {
                m::set("graph-map", state);
                look_from(MODAL_SURFACE, &from, &to).await;
                return true;
            }
            let view_cached = m::data(MODAL_SURFACE.view_key);
            ensure_surface_impact(
                MODAL_SURFACE,
                &state,
                view_cached.is_object().then_some(&view_cached),
                outcome.force_impact,
            );
            if (outcome.rerender || outcome.rerender_side) && !outcome.reload {
                m::set("graph-map", state.clone());
                if m::text(&state, "side_tab") == "changes"
                    && surface_changes_doc(MODAL_SURFACE, &state).is_none()
                {
                    let ch_state = state;
                    spawn_local(async move {
                        fetch_surface_changes(MODAL_SURFACE, ch_state).await;
                    });
                }
                if outcome.rerender_side {
                    rerender_surface_side_only(MODAL_SURFACE);
                } else {
                    rerender_surface_preserving_camera(MODAL_SURFACE);
                }
                return true;
            }
            if !outcome.reload {
                m::set("graph-map", state);
                return true;
            }
        }
        _ => return false,
    }
    m::set("graph-map", state.clone());
    let target = effective_target(&state);
    if chosen || (kind == "click" && ["graph-map-go", "graph-find-choose"].contains(&action)) {
        // The filter field follows: it is emptied where the place changed.
        dom::set_value("graph-map-filter", &m::text(&state, "filter"));
        list();
        if m::yes(&commit_dock()["open"]) {
            show_commit_dock();
        }
    }
    if chosen && target.contains(':') && !target.starts_with("sysrun:") && !choose(&target).await {
        return true;
    }
    load(false).await;
    if jumped {
        reveal();
    }
    true
}
