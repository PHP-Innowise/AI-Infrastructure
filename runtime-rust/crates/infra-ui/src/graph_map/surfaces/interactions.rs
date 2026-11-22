use super::*;

pub(in crate::graph_map) fn handle_surface_input(
    surface: MapSurface,
    id: &str,
    value: &str,
    checked: Option<bool>,
) -> bool {
    let Some(suffix) = id.strip_prefix(surface.id_prefix) else {
        return false;
    };
    match suffix {
        "-filter" => {
            let mut state = surface_state(surface);
            if m::text(&state, "filter") == value {
                return true;
            }
            state["filter"] = json!(value);
            state["scenario_skip"] = json!(0);
            state["skip"] = json!([]);
            save_surface_state(surface, &state);
            dom::later(
                surface.filter_timer_key,
                if value.is_empty() { 0 } else { 140 },
                move || {
                    spawn_local(async move {
                        load_embedded_surface(surface, false).await;
                    });
                },
            );
            true
        }
        "-notests" => {
            let next = checked.unwrap_or_else(|| dom::checked(id));
            let mut state = surface_state(surface);
            if m::yes(&state["notests"]) == next {
                return true;
            }
            state["notests"] = json!(next);
            state["skip"] = json!([]);
            dom::save(NOTESTS, next.then_some("1"));
            save_surface_state(surface, &state);
            sync_surface_toolbar(
                surface,
                &state,
                surface_changes_doc(surface, &state).as_ref(),
            );
            let ch_state = state;
            spawn_local(async move {
                fetch_surface_changes(surface, ch_state).await;
            });
            spawn_local(async move {
                load_embedded_surface(surface, false).await;
            });
            true
        }
        "-certain" => {
            let next = checked.unwrap_or_else(|| dom::checked(id));
            let mut state = surface_state(surface);
            state["certain"] = json!(next);
            state["skip"] = json!([]);
            save_surface_state(surface, &state);
            ensure_surface_impact(surface, &state, None, true);
            spawn_local(async move {
                load_embedded_surface(surface, false).await;
            });
            true
        }
        "-launch" => {
            let mut state = surface_state(surface);
            state["launch"] = json!(value);
            if value.is_empty() || state["compare_launch"] == value {
                state["compare_launch"] = V::Null;
            }
            save_surface_state(surface, &state);
            spawn_local(async move {
                load_embedded_surface(surface, false).await;
            });
            true
        }
        "-compare-launch" => {
            let mut state = surface_state(surface);
            state["compare_launch"] = json!(value);
            save_surface_state(surface, &state);
            spawn_local(async move {
                load_embedded_surface(surface, false).await;
            });
            true
        }
        "-agent" => {
            let mut state = surface_state(surface);
            state["agent"] = json!(value);
            save_surface_state(surface, &state);
            spawn_local(async move {
                load_embedded_surface(surface, false).await;
            });
            true
        }
        "-find-search" => {
            let prefix = surface.id_prefix;
            if value.trim().is_empty() {
                m::next(&format!("{prefix}-find"));
                dom::hidden(&format!("{prefix}-find-found"), true);
                dom::attr(&format!("{prefix}-find-search"), "aria-expanded", "false");
                dom::attr(
                    &format!("{prefix}-find-search"),
                    "aria-activedescendant",
                    "",
                );
                return true;
            }
            let state = surface_state(surface);
            let scope = surface_scope(surface);
            let Some(url) = build_scoped_find_path(&scope, &state, value) else {
                dom::hidden(&format!("{prefix}-find-found"), true);
                dom::attr(&format!("{prefix}-find-search"), "aria-expanded", "false");
                dom::attr(
                    &format!("{prefix}-find-search"),
                    "aria-activedescendant",
                    "",
                );
                return true;
            };
            dom::later(&format!("{prefix}-find-deb"), 180, move || {
                spawn_local(async move {
                    match api::scoped(&format!("{prefix}-find"), &url).await {
                        Ok((epoch, res)) => {
                            if m::current(&format!("{prefix}-find"), epoch) {
                                let rows = m::array(&res["rows"]);
                                let mut first_id = String::new();
                                let list_html = rows
                                    .iter()
                                    .take(25)
                                    .enumerate()
                                    .map(|(idx, row)| {
                                        let p = m::text(row, "path");
                                        let name = m::text(row, "name");
                                        let lbl = if name.is_empty() {
                                            p.clone()
                                        } else {
                                            format!("{name} · {p}")
                                        };
                                        let opt_id = format!("{prefix}-find-row-{idx}");
                                        if idx == 0 {
                                            first_id = opt_id.clone();
                                        }
                                        format!(
                                            "<li id=\"{opt_id}\" class=\"graph-pick-row graph-find-row\" role=\"option\" tabindex=\"-1\" aria-selected=\"{}\" data-action=\"{prefix}-find-choose\" data-payload=\"{}\">{}</li>",
                                            idx == 0,
                                            payload(&json!({ "path": p })),
                                            escape(&lbl)
                                        )
                                    })
                                    .collect::<String>();
                                html(&format!("{prefix}-find-list"), &list_html);
                                dom::text(
                                    &format!("{prefix}-find-note"),
                                    &if rows.is_empty() {
                                        "No matches".to_owned()
                                    } else {
                                        format!("{} matches", rows.len())
                                    },
                                );
                                dom::hidden(&format!("{prefix}-find-found"), false);
                                dom::attr(
                                    &format!("{prefix}-find-search"),
                                    "aria-expanded",
                                    "true",
                                );
                                dom::attr(
                                    &format!("{prefix}-find-search"),
                                    "aria-activedescendant",
                                    &first_id,
                                );
                            }
                        }
                        Err(error) if error.aborted => {}
                        Err(error) => {
                            html(&format!("{prefix}-find-list"), "");
                            dom::text(
                                &format!("{prefix}-find-note"),
                                &format!("Search failed: {}", error.message),
                            );
                            dom::hidden(&format!("{prefix}-find-found"), false);
                            dom::attr(&format!("{prefix}-find-search"), "aria-expanded", "true");
                            dom::attr(
                                &format!("{prefix}-find-search"),
                                "aria-activedescendant",
                                "",
                            );
                        }
                    }
                });
            });
            true
        }
        "-isolate-depth" => {
            let depth = value.trim().parse::<u64>().unwrap_or(2).clamp(1, 3);
            let mut state = surface_state(surface);
            state["isolate_depth"] = json!(depth);
            state["isolate_active"] = json!(!m::text(&state, "focus").is_empty());
            save_surface_state(surface, &state);
            spawn_local(async move {
                load_embedded_surface(surface, false).await;
            });
            true
        }
        _ => false,
    }
}
/// Handles input/change events on embedded Sessions graph toolbar controls (`#session-graph-filter`, `#session-graph-notests`).
pub fn handle_session_graph_input(id: &str, value: &str, checked: Option<bool>) -> bool {
    handle_surface_input(SESSION_SURFACE, id, value, checked)
}
/// Handles `input` events on embedded System code-graph toolbar controls (`#system-member-graph-filter`, `#system-change-graph-filter`).
#[allow(dead_code)]
pub(crate) fn handle_system_input(id: &str, value: &str) -> bool {
    if id.starts_with(SYSTEM_EMBEDDED_SURFACE.id_prefix) {
        return handle_surface_input(SYSTEM_EMBEDDED_SURFACE, id, value, None);
    }
    if id.starts_with(SYSTEM_CHANGE_SURFACE.id_prefix) {
        return handle_surface_input(SYSTEM_CHANGE_SURFACE, id, value, None);
    }
    false
}
/// Handles `change` events on embedded System code-graph toolbar controls (`#system-member-graph-notests`, `#system-change-graph-notests`, launch/agent selectors).
#[allow(dead_code)]
pub(crate) fn handle_system_change(id: &str, value: &str, checked: bool) -> bool {
    if id.starts_with(SYSTEM_EMBEDDED_SURFACE.id_prefix) {
        return handle_surface_input(SYSTEM_EMBEDDED_SURFACE, id, value, Some(checked));
    }
    if id.starts_with(SYSTEM_CHANGE_SURFACE.id_prefix) {
        return handle_surface_input(SYSTEM_CHANGE_SURFACE, id, value, Some(checked));
    }
    false
}
pub(in crate::graph_map) fn handle_surface_click(
    surface: MapSurface,
    id: &str,
    action: &str,
    payload: &V,
) -> bool {
    let prefix = surface.id_prefix;
    let id_suffix = id.strip_prefix(prefix);
    let action_suffix = action.strip_prefix(prefix);
    let is_toolbar_or_strip = matches!(
        id_suffix,
        Some(
            "-zoom-in"
                | "-zoom-out"
                | "-zoom-level"
                | "-fit"
                | "-follow"
                | "-pause-motion"
                | "-expand"
                | "-mode-whole"
                | "-mode-changes"
                | "-changes-count"
                | "-unfold"
                | "-open-full"
                | "-build"
                | "-rebuild"
        )
    ) || matches!(
        action_suffix,
        Some(
            "-zoom-in"
                | "-zoom-out"
                | "-zoom-reset"
                | "-fit"
                | "-follow"
                | "-pause-motion"
                | "-expand"
                | "-mode-whole"
                | "-mode-changes"
                | "-unfold"
                | "-open-full"
                | "-focus-jump"
                | "-side-close"
                | "-attach"
                | "-build"
                | "-rebuild"
        )
    ) || matches!(
        action,
        "graph-side-tab"
            | "graph-changes-filter"
            | "graph-map-member-filter"
            | "graph-map-impact-focus"
            | "graph-map-switch-service-file"
            | "graph-map-shell-event"
    );
    if !is_toolbar_or_strip && m::yes(&surface_drag_data(surface)["suppress_click"]) {
        clear_surface_drag_suppress(surface);
        return true;
    }
    if action == "graph-map-shell-event" {
        let owned = payload.clone();
        spawn_local(async move {
            show_shell_event(&owned).await;
        });
        return true;
    }
    if action == "graph-map-failing" {
        let mut state = surface_state(surface);
        state["failing"] = if m::yes(&payload["clear"]) {
            json!([])
        } else {
            json!(failing_lines(&dom::value(&format!("{prefix}-failing"))))
        };
        save_surface_state(surface, &state);
        spawn_local(async move {
            load_embedded_surface(surface, false).await;
        });
        return true;
    }
    match (id_suffix, action_suffix) {
        (Some("-zoom-in"), _) | (_, Some("-zoom-in")) => {
            zoom_surface_camera_at(surface, ZOOM_STEP, None);
            return true;
        }
        (Some("-zoom-out"), _) | (_, Some("-zoom-out")) => {
            zoom_surface_camera_at(surface, 1.0 / ZOOM_STEP, None);
            return true;
        }
        (Some("-zoom-level"), _) | (_, Some("-zoom-reset")) => {
            reset_surface_camera_100(surface);
            return true;
        }
        (Some("-fit"), _) | (_, Some("-fit")) => {
            let mut cam = load_surface_camera(surface);
            fit_surface_camera(surface, &mut cam);
            return true;
        }
        (Some("-follow"), _) | (_, Some("-follow")) => {
            let mut cam = load_surface_camera(surface);
            cam.follow = !cam.follow;
            if cam.follow {
                if let Some(bbox) = cam.bbox {
                    zoom_surface_camera_to_bbox(surface, &mut cam, bbox);
                    return true;
                }
            }
            save_surface_camera(surface, &cam);
            apply_surface_camera(surface, &cam);
            return true;
        }
        (Some("-pause-motion"), _) | (_, Some("-pause-motion")) => {
            let mut cam = load_surface_camera(surface);
            cam.paused = !cam.paused;
            save_surface_camera(surface, &cam);
            apply_surface_camera(surface, &cam);
            return true;
        }
        (Some("-expand"), _) | (_, Some("-expand")) => {
            let mut cam = load_surface_camera(surface);
            cam.expanded = !cam.expanded;
            cam.needs_measure = true;
            save_surface_camera(surface, &cam);
            apply_surface_camera(surface, &cam);
            dom::later(surface.refit_timer_key, 60, move || {
                refit_surface_if_visible(surface);
            });
            return true;
        }
        (Some("-mode-whole"), _) | (_, Some("-mode-whole")) => {
            let mut state = surface_state(surface);
            state["target"] = json!("");
            state["compare_base"] = json!("");
            state["scope"] = json!("");
            state["focus"] = json!("");
            state["filter"] = json!("");
            state["side_tab"] = json!("details");
            state["side_pinned"] = json!(false);
            state["modules"] = json!(false);
            state["cycle"] = V::Null;
            state["pattern"] = V::Null;
            state["pick"] = V::Null;
            state["scenario"] = V::Null;
            state["skip"] = json!([]);
            save_surface_state(surface, &state);
            m::take(surface.impact_key);
            if !surface.pan_target_key.is_empty() {
                m::take(surface.pan_target_key);
            }
            dom::set_value(&format!("{prefix}-filter"), "");
            let mut cam = load_surface_camera(surface);
            cam.follow = false;
            cam.fit_mode = true;
            save_surface_camera(surface, &cam);
            sync_surface_toolbar(
                surface,
                &state,
                surface_changes_doc(surface, &state).as_ref(),
            );
            let ch_state = state;
            spawn_local(async move {
                fetch_surface_changes(surface, ch_state).await;
            });
            spawn_local(async move {
                load_embedded_surface(surface, false).await;
            });
            return true;
        }
        (Some("-mode-changes" | "-changes-count"), _) | (_, Some("-mode-changes")) => {
            let mut state = surface_state(surface);
            let scope = surface_scope(surface);
            let next_target = if surface.name == "system-change" {
                match &scope {
                    GraphTargetScope::SystemService {
                        run_id: Some(rid), ..
                    } if !rid.trim().is_empty() => format!("sysrun:{}", rid.trim()),
                    _ => "work".to_owned(),
                }
            } else {
                "work".to_owned()
            };
            state["target"] = json!(next_target);
            state["side_tab"] = json!("changes");
            state["side_pinned"] = json!(true);
            state["modules"] = json!(false);
            state["cycle"] = V::Null;
            state["pattern"] = V::Null;
            state["pick"] = V::Null;
            state["scenario"] = V::Null;
            state["skip"] = json!([]);
            save_surface_state(surface, &state);
            rerender_surface_preserving_camera(surface);
            let ch_state = state;
            spawn_local(async move {
                fetch_surface_changes(surface, ch_state).await;
            });
            spawn_local(async move {
                load_embedded_surface(surface, false).await;
            });
            return true;
        }
        (Some("-unfold"), _) | (_, Some("-unfold")) => {
            let mut state = surface_state(surface);
            let next_whole = !m::yes(&state["whole"]);
            state["whole"] = json!(next_whole);
            if next_whole {
                state["scope"] = json!("");
                state["modules"] = json!(false);
                state["cycle"] = V::Null;
                state["pattern"] = V::Null;
                state["scenario"] = V::Null;
            }
            save_surface_state(surface, &state);
            let mut cam = load_surface_camera(surface);
            cam.follow = false;
            cam.fit_mode = true;
            save_surface_camera(surface, &cam);
            sync_surface_toolbar(
                surface,
                &state,
                surface_changes_doc(surface, &state).as_ref(),
            );
            spawn_local(async move {
                load_embedded_surface(surface, false).await;
            });
            return true;
        }
        (Some("-open-full"), _) | (_, Some("-open-full")) => {
            spawn_local(async move {
                open_full_from_surface(surface).await;
            });
            return true;
        }
        (Some("-build" | "-rebuild"), _) | (_, Some("-build" | "-rebuild")) => {
            let scope = surface_scope(surface);
            if !scope.is_empty() {
                let prefix_owned = prefix.to_owned();
                spawn_local(async move {
                    if crate::graph::trigger_rebuild_for_scope(scope, &prefix_owned)
                        .await
                        .is_ok()
                    {
                        load_embedded_surface(surface, false).await;
                    }
                });
            }
            return true;
        }
        (Some("-stale-refresh"), _) | (_, Some("-stale-refresh")) => {
            dom::hidden(&format!("{prefix}-stale-banner"), true);
            spawn_local(async move {
                load_embedded_surface(surface, false).await;
            });
            return true;
        }
        (Some("-stale-dismiss"), _) | (_, Some("-stale-dismiss")) => {
            dom::hidden(&format!("{prefix}-stale-banner"), true);
            return true;
        }
        (Some("-reset-pins"), _) | (_, Some("-reset-pins")) => {
            let state = surface_state(surface);
            let pin_key = surface_pin_key(surface, &state);
            push_pin_undo_snapshot(surface, &pin_key);
            graph_viewport::clear_pins(&pin_key);
            sync_surface_toolbar(
                surface,
                &state,
                surface_changes_doc(surface, &state).as_ref(),
            );
            rerender_surface_preserving_camera(surface);
            graph_viewport::announce_live(
                &format!("{prefix}-live-announce"),
                "Reset pinned layout",
            );
            return true;
        }
        (Some("-pin-undo"), _) | (_, Some("-pin-undo")) => {
            return undo_surface_pins(surface);
        }
        (Some("-unpin-node"), _) | (_, Some("-unpin-node")) => {
            let state = surface_state(surface);
            let node_key = payload
                .as_str()
                .map(str::to_owned)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| m::text(&state, "focus"));
            return unpin_surface_node(surface, &node_key);
        }
        (Some("-nav-back"), _) | (_, Some("-nav-back")) => {
            return navigate_surface_history(surface, -1, None);
        }
        (Some("-nav-forward"), _) | (_, Some("-nav-forward")) => {
            return navigate_surface_history(surface, 1, None);
        }
        (_, Some("-history-jump")) => {
            if let Some(idx) = payload.as_u64().map(|n| n as usize) {
                return navigate_surface_history(surface, 0, Some(idx));
            }
            return false;
        }
        (Some("-isolate-toggle"), _) | (_, Some("-isolate-toggle")) => {
            let mut state = surface_state(surface);
            let next = !m::yes(&state["isolate_active"]);
            let depth = dom::value(&format!("{prefix}-isolate-depth"))
                .trim()
                .parse::<u64>()
                .unwrap_or(2)
                .clamp(1, 3);
            let has_focus = !m::text(&state, "focus").is_empty();
            let active = next && has_focus;
            state["isolate_active"] = json!(active);
            state["isolate_depth"] = json!(depth);
            save_surface_state(surface, &state);
            spawn_local(async move {
                load_embedded_surface(surface, false).await;
            });
            graph_viewport::announce_live(
                &format!("{prefix}-live-announce"),
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
        (Some("-path-trace"), _) | (_, Some("-path-trace")) => {
            let from_q = dom::value(&format!("{prefix}-path-from"));
            let to_q = dom::value(&format!("{prefix}-path-to"));
            let mut state = surface_state(surface);
            state["traced_path"] = json!({ "from": from_q.trim(), "to": to_q.trim() });
            save_surface_state(surface, &state);
            sync_surface_toolbar(
                surface,
                &state,
                surface_changes_doc(surface, &state).as_ref(),
            );
            rerender_surface_preserving_camera(surface);
            graph_viewport::announce_live(&format!("{prefix}-live-announce"), "Updated path trace");
            return true;
        }
        (Some("-path-clear"), _) | (_, Some("-path-clear")) => {
            let mut state = surface_state(surface);
            state["traced_path"] = V::Null;
            save_surface_state(surface, &state);
            sync_surface_toolbar(
                surface,
                &state,
                surface_changes_doc(surface, &state).as_ref(),
            );
            rerender_surface_preserving_camera(surface);
            graph_viewport::announce_live(&format!("{prefix}-live-announce"), "Cleared path trace");
            return true;
        }
        (_, Some("-find-choose")) => {
            dom::hidden(&format!("{prefix}-find-found"), true);
            dom::attr(&format!("{prefix}-find-search"), "aria-expanded", "false");
            dom::attr(
                &format!("{prefix}-find-search"),
                "aria-activedescendant",
                "",
            );
            let path = m::text(payload, "path");
            if !path.is_empty() {
                focus_file_on_surface(surface, &path);
            }
            return true;
        }
        (_, Some("-side-close")) => {
            let mut state = surface_state(surface);
            state["focus"] = json!("");
            state["side_tab"] = json!("details");
            state["side_pinned"] = json!(false);
            save_surface_state(surface, &state);
            m::take(surface.impact_key);
            if !surface.pan_target_key.is_empty() {
                m::take(surface.pan_target_key);
            }
            sync_surface_toolbar(
                surface,
                &state,
                surface_changes_doc(surface, &state).as_ref(),
            );
            if m::data(surface.view_key)["nodes"].is_array() {
                spawn_local(async move {
                    load_embedded_surface(surface, false).await;
                });
            } else {
                blank_embedded_surface(surface, "");
            }
            return true;
        }
        (_, Some("-entity" | "-focus-jump")) => {
            return jump_to_surface_entity_or_target(surface, payload);
        }
        (_, Some("-attach")) => {
            let context = m::data(surface.attachment_key);
            let scope = GraphTargetScope::from_json(&context["scope"])
                .unwrap_or_else(|| surface_scope(surface));
            if let Some(text) = context["text"].as_str() {
                if let GraphTargetScope::SystemService { service_id, .. } = &scope {
                    if attach_map_context_to_system_task(service_id, text) {
                        dom::focus("system-task");
                    }
                } else if crate::session::attach_map_context(&m::text(&context, "project"), text) {
                    dom::focus("prompt");
                }
            }
            return true;
        }
        _ => {}
    }
    if open()
        || !(action.starts_with("graph-map-")
            || matches!(action, "graph-side-tab" | "graph-changes-filter"))
    {
        return false;
    }
    if action == "graph-map-open-omnigraph" {
        let proj = m::text(payload, "project_id");
        let p = m::text(payload, "path");
        if !p.is_empty() {
            spawn_local(async move {
                crate::omnigraph::focus_context_for_file(
                    (!proj.is_empty()).then_some(proj.as_str()),
                    &p,
                )
                .await;
            });
        }
        return true;
    }
    let busy = dom::get(surface.canvas)
        .and_then(|el| el.get_attribute("aria-busy"))
        .as_deref()
        == Some("true");
    let map_failed = dom::get(surface.error_id).is_some_and(|error| !error.has_attribute("hidden"));
    let mut state = surface_state(surface);
    let outcome = apply_click_action(&mut state, action, payload, surface, busy, map_failed);
    if !outcome.handled {
        return false;
    }
    if let Some((svc, file_path)) = outcome.switch_service {
        if switch_surface_to_service_file(surface, &svc, &file_path) {
            dom::set_value(&format!("{prefix}-filter"), "");
            let switched = surface_state(surface);
            sync_surface_toolbar(
                surface,
                &switched,
                surface_changes_doc(surface, &switched).as_ref(),
            );
            ensure_surface_impact(surface, &switched, None, true);
            spawn_local(async move {
                load_embedded_surface(surface, false).await;
            });
        }
        return true;
    }
    save_surface_state(surface, &state);
    if let Some(filter_val) = outcome.clear_filter.as_deref() {
        dom::set_value(&format!("{prefix}-filter"), filter_val);
    }
    if let Some(notests) = outcome.save_notests {
        dom::save(NOTESTS, notests.then_some("1"));
    }
    sync_surface_toolbar(
        surface,
        &state,
        surface_changes_doc(surface, &state).as_ref(),
    );
    if let Some((from, to)) = outcome.open_link {
        spawn_local(async move {
            look_from(surface, &from, &to).await;
        });
        return true;
    }
    let view = m::data(surface.view_key);
    let view_opt = view["nodes"].is_array().then_some(&view);
    ensure_surface_impact(surface, &state, view_opt, outcome.force_impact);
    if m::text(&state, "side_tab") == "changes" && surface_changes_doc(surface, &state).is_none() {
        let ch_state = state;
        spawn_local(async move {
            fetch_surface_changes(surface, ch_state).await;
        });
    }
    if outcome.reload {
        spawn_local(async move {
            load_embedded_surface(surface, false).await;
        });
    } else if outcome.rerender_side {
        rerender_surface_side_only(surface);
    } else if outcome.rerender {
        rerender_surface_preserving_camera(surface);
    }
    true
}
/// Handles clicks inside `#session-graph-workspace` (toolbar zoom/pan/focus + embedded map navigation).
pub fn handle_session_graph_click(id: &str, action: &str, payload: &V) -> bool {
    handle_surface_click(SESSION_SURFACE, id, action, payload)
}
pub(in crate::graph_map) fn resolve_system_click_surface(
    id: &str,
    action: &str,
    payload: &V,
) -> Option<MapSurface> {
    if id.starts_with(SYSTEM_EMBEDDED_SURFACE.id_prefix)
        || action.starts_with(SYSTEM_EMBEDDED_SURFACE.id_prefix)
        || payload["surface"].as_str() == Some(SYSTEM_EMBEDDED_SURFACE.name)
    {
        return Some(SYSTEM_EMBEDDED_SURFACE);
    }
    if id.starts_with(SYSTEM_CHANGE_SURFACE.id_prefix)
        || action.starts_with(SYSTEM_CHANGE_SURFACE.id_prefix)
        || payload["surface"].as_str() == Some(SYSTEM_CHANGE_SURFACE.name)
    {
        return Some(SYSTEM_CHANGE_SURFACE);
    }
    if open() {
        return None;
    }
    if !(action.starts_with("graph-map-")
        || matches!(action, "graph-side-tab" | "graph-changes-filter"))
    {
        return None;
    }
    if let Some(active) = dom::document().active_element() {
        if active
            .closest(
                "#system-change-graph-workspace, #system-change-graph-canvas, #system-change-graph-side",
            )
            .ok()
            .flatten()
            .is_some()
        {
            return Some(SYSTEM_CHANGE_SURFACE);
        }
        if active
            .closest(
                "#system-member-graph-workspace, #system-member-graph-canvas, #system-member-graph-side",
            )
            .ok()
            .flatten()
            .is_some()
        {
            return Some(SYSTEM_EMBEDDED_SURFACE);
        }
    }
    match m::read(|s| s.view.clone()).as_str() {
        "system-changes" if !surface_scope(SYSTEM_CHANGE_SURFACE).is_empty() => {
            Some(SYSTEM_CHANGE_SURFACE)
        }
        "systems" if !surface_scope(SYSTEM_EMBEDDED_SURFACE).is_empty() => {
            Some(SYSTEM_EMBEDDED_SURFACE)
        }
        _ => None,
    }
}
/// Handles clicks inside `#system-member-graph-*` (`#systems-view`) and `#system-change-graph-*` (`#system-changes-view`).
#[allow(dead_code)]
pub(crate) fn handle_system_click(id: &str, action: &str, payload: &V) -> bool {
    let Some(surface) = resolve_system_click_surface(id, action, payload) else {
        return false;
    };
    handle_surface_click(surface, id, action, payload)
}
pub(in crate::graph_map) fn target_pan_surface(
    target_el: Option<&web_sys::Element>,
) -> Option<MapSurface> {
    let canvas = target_el?
        .closest("#graph-map-canvas, #session-graph-canvas, #system-member-graph-canvas, #system-change-graph-canvas")
        .ok()
        .flatten()?;
    pan_surface_for_canvas(&canvas.id())
}
pub(in crate::graph_map) fn active_drag_surface() -> Option<(MapSurface, graph_viewport::DragPan)> {
    for surface in [
        MODAL_SURFACE,
        SESSION_SURFACE,
        SYSTEM_EMBEDDED_SURFACE,
        SYSTEM_CHANGE_SURFACE,
    ] {
        let drag = graph_viewport::DragPan::from_json(&surface_drag_data(surface));
        if drag.active {
            return Some((surface, drag));
        }
    }
    None
}
pub(in crate::graph_map) fn apply_live_node_pin_visuals(
    surface: MapSurface,
    idx: usize,
    nx: f64,
    ny: f64,
    is_dense: bool,
    pairs: &[(usize, usize)],
    node_coords: &[(f64, f64)],
) {
    let cam = load_surface_camera(surface);
    if let Some(canvas) = dom::get(surface.canvas) {
        if let Ok(Some(node_el)) =
            canvas.query_selector(&format!(".graph-map-node[data-n='{idx}']"))
        {
            let _ = node_el.set_attribute("data-pinned", "true");
            if let Ok(html_el) = node_el.dyn_into::<web_sys::HtmlElement>() {
                let _ = html_el.style().set_property("left", &format!("{nx:.1}px"));
                let _ = html_el.style().set_property("top", &format!("{ny:.1}px"));
            }
        }
        // Live-update wires and SpatialHitGrid attached to the moved node in Dense Canvas2D and SVG modes.
        if is_dense {
            let moved_edges: Vec<usize> = pairs
                .iter()
                .enumerate()
                .filter_map(|(ei, &(from, to))| (from == idx || to == idx).then_some(ei))
                .collect();
            graph_viewport::canvas2d::update_scene_and_grid(
                surface.canvas,
                &[idx],
                &moved_edges,
                |scene| {
                    if let Some(dn) = scene.nodes.iter_mut().find(|n| n.index == idx) {
                        dn.x = nx;
                        dn.y = ny;
                        dn.pinned = true;
                    }
                    for de in &mut scene.edges {
                        let Some(&(from, to)) = pairs.get(de.index) else {
                            continue;
                        };
                        if from != idx && to != idx {
                            continue;
                        }
                        let a = if from == idx {
                            (nx, ny)
                        } else if let Some(&(ax, ay)) = node_coords.get(from) {
                            (ax, ay)
                        } else {
                            continue;
                        };
                        let b = if to == idx {
                            (nx, ny)
                        } else if let Some(&(bx, by)) = node_coords.get(to) {
                            (bx, by)
                        } else {
                            continue;
                        };
                        let d = route_pinned_wire_path(a, b, 0.0, 0.0, de.detour);
                        de.set_svg_path(&d);
                    }
                },
            );
            graph_viewport::canvas2d::paint_scene(
                surface.canvas,
                cam.viewport_cam(),
                current_dense_hover(surface.canvas),
                true,
            );
        } else {
            let mut detour_pairs: std::collections::HashSet<(usize, usize)> =
                std::collections::HashSet::new();
            if let Ok(detours) =
                canvas.query_selector_all(".graph-map-edge[data-detour='true'][data-ends]")
            {
                for i in 0..detours.length() {
                    if let Some(pair) = detours
                        .item(i)
                        .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
                        .and_then(|el| el.get_attribute("data-ends").as_deref().and_then(ends_of))
                    {
                        detour_pairs.insert(pair);
                    }
                }
            }
            if let Ok(wires) = canvas.query_selector_all(
                ".graph-map-edge[data-ends], .graph-map-edge-core[data-ends], .graph-map-violation-halo[data-ends], .graph-map-hit[data-ends]",
            ) {
                for i in 0..wires.length() {
                    let Some(w_el) = wires
                        .item(i)
                        .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
                    else {
                        continue;
                    };
                    let Some((from, to)) = w_el
                        .get_attribute("data-ends")
                        .as_deref()
                        .and_then(ends_of)
                    else {
                        continue;
                    };
                    if from != idx && to != idx {
                        continue;
                    }
                    let a = if from == idx {
                        (nx, ny)
                    } else if let Some(&(ax, ay)) = node_coords.get(from) {
                        (ax, ay)
                    } else {
                        continue;
                    };
                    let b = if to == idx {
                        (nx, ny)
                    } else if let Some(&(bx, by)) = node_coords.get(to) {
                        (bx, by)
                    } else {
                        continue;
                    };
                    let is_detour = detour_pairs.contains(&(from, to))
                        || w_el.get_attribute("data-detour").as_deref() == Some("true");
                    let d = route_pinned_wire_path(a, b, 0.0, 0.0, is_detour);
                    let _ = w_el.set_attribute("d", &d);
                }
            }
        }
    }
}

fn handle_surface_node_pin_drag(
    kind: &str,
    event: &web_sys::Event,
    target_el: Option<&web_sys::Element>,
) -> bool {
    match kind {
        "pointerdown" => {
            let Some(surface) = target_pan_surface(target_el) else {
                return false;
            };
            let Ok(pe) = event.clone().dyn_into::<web_sys::PointerEvent>() else {
                return false;
            };
            if pe.button() != 0 || !pe.shift_key() {
                return false;
            }
            let idx = if let Some(node_el) =
                target_el.and_then(|el| el.closest(".graph-map-node[data-n]").ok().flatten())
            {
                let Some(idx) = node_el
                    .get_attribute("data-n")
                    .and_then(|s| s.parse::<usize>().ok())
                else {
                    return false;
                };
                idx
            } else {
                let is_dense = SURFACE_PLAN_CACHE.with(|cache| {
                    cache
                        .borrow()
                        .get(surface.canvas)
                        .is_some_and(|entry| entry.is_dense)
                });
                if !is_dense {
                    return false;
                }
                let cam = load_surface_camera(surface);
                let Some((0, idx)) = graph_viewport::dense_dom::hit_test_pointer_in_canvas(
                    surface.canvas,
                    pe.client_x() as f64,
                    pe.client_y() as f64,
                    cam.viewport_cam(),
                    4.0,
                ) else {
                    return false;
                };
                let _ = mount_dense_node_if_needed(surface.canvas, idx);
                idx
            };
            let info = SURFACE_PLAN_CACHE.with(|cache| {
                cache.borrow().get(surface.canvas).and_then(|entry| {
                    let placed = entry.nodes.get(idx)?;
                    let key = entry.node_keys.get(idx)?.clone();
                    Some((placed.x, placed.y, key))
                })
            });
            let Some((orig_x, orig_y, node_key)) = info else {
                return false;
            };
            if let Some(canvas) = dom::get(surface.canvas) {
                graph_viewport::capture_pointer(&canvas, pe.pointer_id());
            }
            let session = graph_viewport::dense_dom::PinDragSession::begin(
                pe.pointer_id(),
                idx,
                node_key,
                (pe.client_x() as f64, pe.client_y() as f64),
                (orig_x, orig_y),
            );
            let mut drag_val = session.to_json();
            drag_val["surface"] = json!(surface.name);
            m::set("surface-node-drag", drag_val);
            true
        }
        "pointermove" | "pointerup" | "pointercancel" => {
            let drag = m::data("surface-node-drag");
            let mut session = graph_viewport::dense_dom::PinDragSession::from_json(&drag);
            if !session.active {
                return false;
            }
            let surface = surface_by_name(&m::text(&drag, "surface"));
            let Ok(pe) = event.clone().dyn_into::<web_sys::PointerEvent>() else {
                if kind != "pointermove" {
                    m::set("surface-node-drag", json!({}));
                }
                return false;
            };
            if pe.pointer_id() != session.pointer_id {
                return false;
            }
            if kind == "pointermove" {
                let cam = load_surface_camera(surface);
                let (is_dense, node_coords, pairs, plane_size) = SURFACE_PLAN_CACHE.with(|cache| {
                    cache.borrow().get(surface.canvas).map_or(
                        (false, Vec::new(), Vec::new(), (1600.0, 1200.0)),
                        |entry| {
                            let coords: Vec<(f64, f64)> =
                                entry.nodes.iter().map(|n| (n.x, n.y)).collect();
                            (
                                entry.is_dense,
                                coords,
                                entry.pairs.clone(),
                                (
                                    entry.width.max(BOX_WIDTH + 16.0),
                                    entry.height.max(BOX_HEIGHT + 16.0),
                                ),
                            )
                        },
                    )
                });
                let bounds = (
                    8.0,
                    8.0,
                    (plane_size.0 - BOX_WIDTH - 8.0).max(8.0),
                    (plane_size.1 - BOX_HEIGHT - 8.0).max(8.0),
                );
                let Some((nx, ny)) = session.advance(
                    pe.pointer_id(),
                    (pe.client_x() as f64, pe.client_y() as f64),
                    cam.scale,
                    bounds,
                    3.0,
                ) else {
                    return true;
                };
                let idx = session.node_idx;
                apply_live_node_pin_visuals(surface, idx, nx, ny, is_dense, &pairs, &node_coords);
                let mut drag_val = session.to_json();
                drag_val["surface"] = json!(surface.name);
                m::set("surface-node-drag", drag_val);
                return true;
            }
            let outcome = session.end(Some(pe.pointer_id()), kind == "pointercancel");
            m::set("surface-node-drag", json!({}));
            match outcome {
                Some(graph_viewport::dense_dom::PinDragOutcome::Committed {
                    node_key,
                    pos: (cur_x, cur_y),
                    ..
                }) => {
                    if !node_key.is_empty() {
                        let st = surface_state(surface);
                        let pin_key = surface_pin_key(surface, &st);
                        push_pin_undo_snapshot(surface, &pin_key);
                        graph_viewport::save_pin(&pin_key, &node_key, (cur_x, cur_y));
                        sync_history_and_pin_controls(surface, &st);
                    }
                    let mut suppress_state = surface_drag_data(surface);
                    if !suppress_state.is_object() {
                        suppress_state = json!({});
                    }
                    suppress_state["suppress_click"] = json!(true);
                    m::set(surface.drag_key, suppress_state);
                    dom::later(surface.drag_timer_key, 60, move || {
                        clear_surface_drag_suppress(surface);
                    });
                    rerender_surface_preserving_camera(surface);
                }
                Some(graph_viewport::dense_dom::PinDragOutcome::Cancelled { .. }) => {
                    rerender_surface_preserving_camera(surface);
                }
                Some(graph_viewport::dense_dom::PinDragOutcome::Click { .. }) | None => {}
            }
            true
        }
        _ => false,
    }
}

fn handle_link_canvas_pointer(
    kind: &str,
    event: &web_sys::Event,
    target_el: Option<&web_sys::Element>,
) -> bool {
    let on_link_canvas = target_el
        .and_then(|el| el.closest("#graph-link-canvas").ok().flatten())
        .is_some();
    match kind {
        "pointerdown" if on_link_canvas => {
            let cam = load_link_camera();
            if cam.plane_w <= 0.0 || cam.plane_h <= 0.0 {
                return false;
            }
            let mut drag = graph_viewport::DragPan::default();
            match graph_viewport::handle_canvas_drag(
                kind,
                event,
                "graph-link-canvas",
                cam.viewport_cam(),
                graph_viewport::DEFAULT_DRAG_THRESHOLD_PX,
                &mut drag,
            ) {
                graph_viewport::CanvasDragOutcome::Down => {
                    m::set("graph-link-drag", drag.to_json());
                    true
                }
                _ => false,
            }
        }
        "pointermove" | "pointerup" | "pointercancel" => {
            let mut drag = graph_viewport::DragPan::from_json(&m::data("graph-link-drag"));
            if !drag.active {
                return false;
            }
            let mut cam = load_link_camera();
            match graph_viewport::handle_canvas_drag(
                kind,
                event,
                "graph-link-canvas",
                cam.viewport_cam(),
                graph_viewport::DEFAULT_DRAG_THRESHOLD_PX,
                &mut drag,
            ) {
                graph_viewport::CanvasDragOutcome::Panned { tx, ty } => {
                    m::set("graph-link-drag", drag.to_json());
                    cam.pan_x = tx;
                    cam.pan_y = ty;
                    cam.fit_mode = false;
                    cam.needs_measure = false;
                    save_link_camera(&cam);
                    apply_link_camera(&cam);
                    true
                }
                graph_viewport::CanvasDragOutcome::Ended { suppress_click } => {
                    m::set("graph-link-drag", drag.to_json());
                    if suppress_click {
                        dom::later("graph-link-drag-clear", 60, || {
                            let mut d =
                                graph_viewport::DragPan::from_json(&m::data("graph-link-drag"));
                            d.suppress_click = false;
                            m::set("graph-link-drag", d.to_json());
                        });
                    }
                    true
                }
                graph_viewport::CanvasDragOutcome::Ignored
                | graph_viewport::CanvasDragOutcome::Down => {
                    m::set("graph-link-drag", drag.to_json());
                    true
                }
            }
        }
        "dblclick" if on_link_canvas => {
            if target_el
                .and_then(|el| el.closest(".graph-map-node, button, a").ok().flatten())
                .is_some()
            {
                return false;
            }
            let Ok(me) = event.clone().dyn_into::<web_sys::MouseEvent>() else {
                return false;
            };
            let cam = load_link_camera();
            if cam.scale < 1.35 {
                let anchor = dom::get("graph-link-canvas").map(|canvas| {
                    let rect = canvas.get_bounding_client_rect();
                    (
                        me.client_x() as f64 - rect.left(),
                        me.client_y() as f64 - rect.top(),
                    )
                });
                zoom_link_camera_at(1.5, anchor);
            } else {
                fit_link_camera();
            }
            true
        }
        _ => false,
    }
}

fn handle_dense_canvas_hit_interaction(
    kind: &str,
    surface: MapSurface,
    event: &web_sys::Event,
    target_el: Option<&web_sys::Element>,
    trigger_pick: bool,
) -> bool {
    let is_dense = SURFACE_PLAN_CACHE.with(|cache| {
        cache
            .borrow()
            .get(surface.canvas)
            .is_some_and(|entry| entry.is_dense)
    });
    if !is_dense {
        return false;
    }
    let Some(canvas) = dom::get(surface.canvas) else {
        return false;
    };
    let Ok(pe) = event.clone().dyn_into::<web_sys::PointerEvent>() else {
        return false;
    };
    let cam = load_surface_camera(surface);
    let hit = graph_viewport::dense_dom::hit_test_pointer_in_canvas(
        surface.canvas,
        pe.client_x() as f64,
        pe.client_y() as f64,
        cam.viewport_cam(),
        8.0,
    );
    if kind == "pointermove" {
        if set_dense_hover(surface.canvas, hit) {
            graph_viewport::canvas2d::paint_scene(surface.canvas, cam.viewport_cam(), hit, true);
        }
        let on_dom_part = target_el
            .and_then(|el| el.closest(".graph-map-node, .graph-map-hit").ok().flatten())
            .is_some();
        if let Ok(Some(plane)) = canvas.query_selector(".graph-map-plane") {
            let hover_node = match hit {
                Some((0, idx)) => Some(idx),
                _ => None,
            };
            let hovered_el = hover_node.and_then(|idx| {
                plane
                    .query_selector(&format!(".graph-map-node[data-n='{idx}']"))
                    .ok()
                    .flatten()
            });
            graph_viewport::dense_dom::sync_dom_hover_attr(
                &plane,
                hover_node,
                hovered_el.as_ref(),
                |el| {
                    el.get_attribute("data-n")
                        .and_then(|s| s.parse::<usize>().ok())
                },
            );
        }
        if on_dom_part {
            let _ = canvas.remove_attribute("title");
        } else if let Some((k, i)) = hit {
            if let Some(tip) = graph_viewport::canvas2d::hit_test_tooltip_text(surface.canvas, k, i)
            {
                let _ = canvas.set_attribute("title", &tip);
            } else {
                let _ = canvas.remove_attribute("title");
            }
        } else {
            let _ = canvas.remove_attribute("title");
        }
        if let Ok(html_canvas) = canvas.dyn_into::<web_sys::HtmlElement>() {
            let _ = html_canvas
                .style()
                .set_property("cursor", if hit.is_some() { "pointer" } else { "" });
        }
        return false;
    }
    if kind == "pointercancel" || !trigger_pick {
        return false;
    }
    if target_el
        .and_then(|el| {
            el.closest(".graph-map-node, .graph-map-hit, button, a")
                .ok()
                .flatten()
        })
        .is_some()
    {
        return false;
    }
    let Some(hit_item) = hit else {
        return false;
    };
    let payload_action = SURFACE_PLAN_CACHE
        .with(|cache| {
            let guard = cache.borrow();
            let entry = guard.get(surface.canvas)?;
            match hit_item {
                (0, node_idx) => entry
                    .node_keys
                    .get(node_idx)
                    .map(|key| ("graph-map-node", json!(key))),
                (1, edge_idx) => {
                    let (from_i, to_i) = *entry.pairs.get(edge_idx)?;
                    let from = entry.node_keys.get(from_i)?;
                    let to = entry.node_keys.get(to_i)?;
                    Some(("graph-map-link", json!({"from": from, "to": to})))
                }
                _ => None,
            }
        })
        .or_else(|| {
            let (kind_str, payload) = graph_viewport::canvas2d::explain_target_for_hit(
                surface.canvas,
                hit_item.0,
                hit_item.1,
            )?;
            let action = match kind_str.as_str() {
                "node" => "graph-map-node",
                "link" => "graph-map-link",
                _ => return None,
            };
            Some((action, payload))
        });
    let Some((action, action_payload)) = payload_action else {
        return false;
    };
    if surface.embedded {
        handle_surface_click(surface, "", action, &action_payload);
    } else {
        spawn_local(async move {
            let _ = self::event("click", "", action, &action_payload, "", false).await;
        });
    }
    true
}

/// Handles pointer pan (`pointerdown`/`pointermove`/`pointerup`/`pointercancel`) and `dblclick` zoom
/// across `#graph-map-canvas`, `#session-graph-canvas`, `#system-member-graph-canvas`, `#system-change-graph-canvas`, and `#graph-link-canvas`.
pub fn pointer_event(kind: &str, event: &web_sys::Event) -> bool {
    let target_el = event
        .target()
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok());
    if kind == "pointerdown" {
        if let (Some(el), Ok(pe)) = (
            target_el.as_ref(),
            event.clone().dyn_into::<web_sys::PointerEvent>(),
        ) {
            if el.closest(".git-graph-row").ok().flatten().is_some() {
                m::set("graph-commit-shift", json!(pe.shift_key()));
            }
        }
        // Any new pointerdown inside a pan surface ends any stale click-suppression window
        // from an earlier pan drag before interactive-target early returns run.
        if let Some(surface) = target_pan_surface(target_el.as_ref()) {
            clear_surface_drag_suppress(surface);
        }
    }
    if radar_pointer(kind, event, target_el.as_ref()) {
        return true;
    }
    if handle_surface_node_pin_drag(kind, event, target_el.as_ref()) {
        return true;
    }
    if handle_link_canvas_pointer(kind, event, target_el.as_ref()) {
        return true;
    }
    match kind {
        "pointerdown" => {
            let Some(surface) = target_pan_surface(target_el.as_ref()) else {
                return false;
            };
            let plane_el = dom::get(surface.canvas)
                .and_then(|c| c.query_selector(".graph-map-plane").ok().flatten());
            if graph_viewport::dense_dom::is_interactive_or_empty_target(
                target_el.as_ref(),
                plane_el.as_ref(),
                Some(".graph-map-empty, .graph-map-spread"),
            ) {
                return false;
            }
            let cam = load_surface_camera(surface);
            if cam.plane_w <= 0.0 || cam.plane_h <= 0.0 {
                return false;
            }
            let mut drag = graph_viewport::DragPan::default();
            match graph_viewport::handle_canvas_drag(
                kind,
                event,
                surface.canvas,
                cam.viewport_cam(),
                graph_viewport::DEFAULT_DRAG_THRESHOLD_PX,
                &mut drag,
            ) {
                graph_viewport::CanvasDragOutcome::Down => {
                    m::set(surface.drag_key, drag.to_json());
                    true
                }
                _ => true,
            }
        }
        "pointermove" | "pointerup" | "pointercancel" => {
            if let Some((surface, mut drag)) = active_drag_surface() {
                let mut cam = load_surface_camera(surface);
                return match graph_viewport::handle_canvas_drag(
                    kind,
                    event,
                    surface.canvas,
                    cam.viewport_cam(),
                    graph_viewport::DEFAULT_DRAG_THRESHOLD_PX,
                    &mut drag,
                ) {
                    graph_viewport::CanvasDragOutcome::Panned { tx, ty } => {
                        m::set(surface.drag_key, drag.to_json());
                        cam.pan_x = tx;
                        cam.pan_y = ty;
                        cam.fit_mode = false;
                        cam.needs_measure = false;
                        save_surface_camera(surface, &cam);
                        apply_surface_camera(surface, &cam);
                        true
                    }
                    graph_viewport::CanvasDragOutcome::Ended { suppress_click } => {
                        m::set(surface.drag_key, drag.to_json());
                        if suppress_click {
                            dom::later(surface.drag_timer_key, 60, move || {
                                clear_surface_drag_suppress(surface);
                            });
                        } else if kind == "pointerup" {
                            let _ = handle_dense_canvas_hit_interaction(
                                kind,
                                surface,
                                event,
                                target_el.as_ref(),
                                true,
                            );
                        }
                        true
                    }
                    graph_viewport::CanvasDragOutcome::Ignored
                    | graph_viewport::CanvasDragOutcome::Down => {
                        m::set(surface.drag_key, drag.to_json());
                        true
                    }
                };
            }
            if kind == "pointermove" {
                if let Some(surface) = target_pan_surface(target_el.as_ref()) {
                    let _ = handle_dense_canvas_hit_interaction(
                        kind,
                        surface,
                        event,
                        target_el.as_ref(),
                        false,
                    );
                }
            }
            false
        }
        "dblclick" => {
            let Some(surface) = target_pan_surface(target_el.as_ref()) else {
                return false;
            };
            let plane_el = dom::get(surface.canvas)
                .and_then(|c| c.query_selector(".graph-map-plane").ok().flatten());
            if graph_viewport::dense_dom::is_interactive_or_empty_target(
                target_el.as_ref(),
                plane_el.as_ref(),
                Some(
                    ".graph-map-node, .graph-map-entity-node, .graph-map-empty, .graph-map-spread",
                ),
            ) {
                return false;
            }
            let Ok(me) = event.clone().dyn_into::<web_sys::MouseEvent>() else {
                return false;
            };
            let cam = load_surface_camera(surface);
            if cam.plane_w <= 0.0 || cam.plane_h <= 0.0 {
                return false;
            }
            if cam.scale < 1.35 {
                let anchor = dom::get(surface.canvas).map(|canvas| {
                    let rect = canvas.get_bounding_client_rect();
                    (
                        me.client_x() as f64 - rect.left(),
                        me.client_y() as f64 - rect.top(),
                    )
                });
                zoom_surface_camera_at(surface, 1.5, anchor);
            } else {
                let mut cam = cam;
                fit_surface_camera(surface, &mut cam);
            }
            true
        }
        _ => false,
    }
}
