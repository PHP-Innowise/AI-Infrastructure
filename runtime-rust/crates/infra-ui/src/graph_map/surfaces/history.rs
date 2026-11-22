use super::*;
use std::cell::Cell;

type PinMapSnapshot = BTreeMap<String, (f64, f64)>;

thread_local! {
    pub(in crate::graph_map) static SURFACE_NAV_HISTORY: RefCell<HashMap<&'static str, graph_viewport::CameraSelectionHistory>> =
        RefCell::new(HashMap::new());
    pub(in crate::graph_map) static SURFACE_NAV_SNAPSHOTS: RefCell<HashMap<(&'static str, String), V>> =
        RefCell::new(HashMap::new());
    pub(in crate::graph_map) static SURFACE_NAV_EPOCHS: RefCell<HashMap<&'static str, u64>> =
        RefCell::new(HashMap::new());
    pub(in crate::graph_map) static SURFACE_PIN_UNDO: RefCell<HashMap<&'static str, Vec<PinMapSnapshot>>> =
        RefCell::new(HashMap::new());
    pub(in crate::graph_map) static LAST_DENSE_HOVER: RefCell<HashMap<&'static str, Option<(u8, usize)>>> =
        RefCell::new(HashMap::new());
    static NAV_RESTORING: Cell<bool> = const { Cell::new(false) };
}

pub(in crate::graph_map) fn current_dense_hover(canvas_id: &str) -> Option<(u8, usize)> {
    LAST_DENSE_HOVER.with(|h| h.borrow().get(canvas_id).copied().flatten())
}

/// Resolves the active Canvas2D hit target `(0=node | 1=edge, index)` for keyboard
/// actions (`Shift+F10` Explain) when no mounted DOM `.graph-map-node`/`.graph-map-hit`
/// holds focus. Checks active hover first, then roving focus / chosen node.
pub(crate) fn focused_dense_hit_for_canvas(canvas_id: &str) -> Option<(u8, usize)> {
    if let Some(hit) = current_dense_hover(canvas_id) {
        return Some(hit);
    }
    let surface = pan_surface_for_canvas(canvas_id)?;
    let is_dense =
        SURFACE_PLAN_CACHE.with(|cache| cache.borrow().get(canvas_id).is_some_and(|e| e.is_dense));
    if !is_dense {
        return None;
    }
    let state = surface_state(surface);
    if let Some(idx) = state["roving_focus"].as_u64().map(|v| v as usize) {
        return Some((0, idx));
    }
    SURFACE_PLAN_CACHE.with(|cache| {
        cache
            .borrow()
            .get(canvas_id)
            .and_then(|e| e.latest_roving_focus)
            .map(|idx| (0, idx))
    })
}

pub(in crate::graph_map) fn set_dense_hover(
    canvas_id: &'static str,
    next: Option<(u8, usize)>,
) -> bool {
    LAST_DENSE_HOVER.with(|h| {
        let mut guard = h.borrow_mut();
        let prev = guard.get(canvas_id).copied().flatten();
        if prev == next {
            return false;
        }
        if let Some(hit) = next {
            guard.insert(canvas_id, Some(hit));
        } else {
            guard.remove(canvas_id);
        }
        true
    })
}

pub(in crate::graph_map) fn push_pin_undo_snapshot(surface: MapSurface, pin_key: &str) {
    let snap = graph_viewport::load_pins(pin_key);
    SURFACE_PIN_UNDO.with(|u| {
        let mut map = u.borrow_mut();
        let stack = map.entry(surface.canvas).or_default();
        if stack.last() != Some(&snap) {
            stack.push(snap);
            if stack.len() > 24 {
                stack.remove(0);
            }
        }
    });
}

pub(in crate::graph_map) fn can_undo_pins(surface: MapSurface) -> bool {
    SURFACE_PIN_UNDO.with(|u| {
        u.borrow()
            .get(surface.canvas)
            .is_some_and(|s| !s.is_empty())
    })
}

pub(in crate::graph_map) fn undo_surface_pins(surface: MapSurface) -> bool {
    let popped =
        SURFACE_PIN_UNDO.with(|u| u.borrow_mut().get_mut(surface.canvas).and_then(Vec::pop));
    let Some(prev) = popped else {
        return false;
    };
    let state = surface_state(surface);
    let key = surface_pin_key(surface, &state);
    if prev.is_empty() {
        graph_viewport::clear_pins(&key);
    } else {
        graph_viewport::persist_pins_map(&key, &prev);
    }
    sync_history_and_pin_controls(surface, &state);
    rerender_surface_preserving_camera(surface);
    let live_id = if surface.embedded {
        format!("{}-live-announce", surface.id_prefix)
    } else {
        "graph-map-live-announce".to_owned()
    };
    graph_viewport::announce_live(&live_id, "Restored previous pin layout");
    true
}

pub(in crate::graph_map) fn nudge_surface_node_pin(
    surface: MapSurface,
    node_idx: usize,
    dx_world: f64,
    dy_world: f64,
) -> bool {
    let info = SURFACE_PLAN_CACHE.with(|cache| {
        let mut guard = cache.borrow_mut();
        let entry = guard.get_mut(surface.canvas)?;
        let placed = entry.nodes.get(node_idx)?;
        let key = entry.node_keys.get(node_idx)?.clone();
        if key.is_empty() {
            return None;
        }
        let nx = (placed.x + dx_world).clamp(8.0, (entry.width - BOX_WIDTH - 8.0).max(8.0));
        let ny = (placed.y + dy_world).clamp(8.0, (entry.height - BOX_HEIGHT - 8.0).max(8.0));
        entry.nodes[node_idx].x = nx;
        entry.nodes[node_idx].y = ny;
        let is_dense = entry.is_dense;
        let pairs = entry.pairs.clone();
        let node_coords: Vec<(f64, f64)> = entry.nodes.iter().map(|n| (n.x, n.y)).collect();
        Some((nx, ny, key, is_dense, pairs, node_coords))
    });
    let Some((nx, ny, node_key, is_dense, pairs, node_coords)) = info else {
        return false;
    };
    let state = surface_state(surface);
    let pin_key = surface_pin_key(surface, &state);
    push_pin_undo_snapshot(surface, &pin_key);
    graph_viewport::save_pin(&pin_key, &node_key, (nx, ny));
    sync_history_and_pin_controls(surface, &state);
    apply_live_node_pin_visuals(surface, node_idx, nx, ny, is_dense, &pairs, &node_coords);
    let live_id = if surface.embedded {
        format!("{}-live-announce", surface.id_prefix)
    } else {
        "graph-map-live-announce".to_owned()
    };
    graph_viewport::announce_live(
        &live_id,
        &format!("Pinned {} at {nx:.0}, {ny:.0}", base(&node_key)),
    );
    true
}

pub(in crate::graph_map) fn unpin_surface_node(surface: MapSurface, node_key: &str) -> bool {
    if node_key.is_empty() {
        return false;
    }
    let state = surface_state(surface);
    let pin_key = surface_pin_key(surface, &state);
    push_pin_undo_snapshot(surface, &pin_key);
    if !graph_viewport::remove_pin(&pin_key, node_key) {
        SURFACE_PIN_UNDO.with(|u| {
            if let Some(stack) = u.borrow_mut().get_mut(surface.canvas) {
                stack.pop();
            }
        });
        return false;
    }
    sync_history_and_pin_controls(surface, &state);
    rerender_surface_preserving_camera(surface);
    let live_id = if surface.embedded {
        format!("{}-live-announce", surface.id_prefix)
    } else {
        "graph-map-live-announce".to_owned()
    };
    graph_viewport::announce_live(&live_id, &format!("Unpinned {}", base(node_key)));
    true
}

pub(in crate::graph_map) fn record_surface_history(
    surface: MapSurface,
    state: &V,
    view: &V,
    cam: &Camera,
) {
    if NAV_RESTORING.with(Cell::get) {
        return;
    }
    if !cam.scale.is_finite() || cam.scale <= 0.0 || !view["nodes"].is_array() {
        return;
    }
    let focus = m::text(state, "focus");
    let scope = m::text(state, "scope");
    let target = effective_target(state);
    let key = if !focus.is_empty() {
        format!("focus:{focus}")
    } else if !scope.is_empty() {
        format!("scope:{scope}")
    } else {
        "root".to_owned()
    };
    let label = if !focus.is_empty() {
        base(&focus).to_owned()
    } else if !scope.is_empty() {
        scope.clone()
    } else if !target.is_empty() {
        format!("Changes ({})", base(&target))
    } else {
        "Project".to_owned()
    };
    SURFACE_NAV_HISTORY.with(|h| {
        let mut map = h.borrow_mut();
        let hist = map.entry(surface.canvas).or_default();
        hist.push(graph_viewport::CameraSelectionEntry {
            key: key.clone(),
            label,
            target: target.clone(),
            cam: cam.viewport_cam(),
        });
    });
    SURFACE_NAV_SNAPSHOTS.with(|s| {
        let mut map = s.borrow_mut();
        if map.len() > 256 {
            map.retain(|&(cid, _), _| cid == surface.canvas);
        }
        map.insert((surface.canvas, format!("{key}\0{target}")), state.clone());
    });
    sync_history_and_pin_controls(surface, state);
}

pub(in crate::graph_map) fn update_surface_active_camera(surface: MapSurface, cam: &Camera) {
    if NAV_RESTORING.with(Cell::get) {
        return;
    }
    SURFACE_NAV_HISTORY.with(|h| {
        if let Some(hist) = h.borrow_mut().get_mut(surface.canvas) {
            hist.update_active_camera(cam.viewport_cam());
        }
    });
}

pub(in crate::graph_map) fn surface_history_trail_html(surface: MapSurface) -> String {
    SURFACE_NAV_HISTORY.with(|h| {
        let guard = h.borrow();
        let Some(hist) = guard.get(surface.canvas) else {
            return String::new();
        };
        let entries = hist.entries();
        if entries.len() <= 1 {
            return String::new();
        }
        let cursor = hist.cursor();
        let start = entries.len().saturating_sub(5);
        let action = if surface.embedded {
            format!("{}-history-jump", surface.id_prefix)
        } else {
            "graph-map-history-jump".to_owned()
        };
        let mut out = String::from("<span class=\"graph-map-history-trail\" aria-label=\"View history\">");
        for (idx, entry) in entries.iter().enumerate().skip(start) {
            if idx > start {
                out.push_str("<span class=\"graph-map-history-sep\" aria-hidden=\"true\">›</span>");
            }
            let current_attr = if idx == cursor { " aria-current=\"location\"" } else { "" };
            out.push_str(&format!(
                "<button type=\"button\" class=\"graph-map-history-crumb\" data-action=\"{action}\" data-payload=\"{idx}\"{current_attr}>{}</button>",
                escape(&entry.label),
            ));
        }
        out.push_str("</span>");
        out
    })
}

pub(in crate::graph_map) fn sync_history_and_pin_controls(surface: MapSurface, state: &V) {
    let (can_back, can_forward) = SURFACE_NAV_HISTORY.with(|h| {
        h.borrow()
            .get(surface.canvas)
            .map_or((false, false), |hist| (hist.can_back(), hist.can_forward()))
    });
    let has_pins = graph_viewport::has_pins(&surface_pin_key(surface, state));
    let can_undo = can_undo_pins(surface);
    if surface.embedded {
        let prefix = surface.id_prefix;
        dom::disabled(&format!("{prefix}-nav-back"), !can_back);
        dom::disabled(&format!("{prefix}-nav-forward"), !can_forward);
        dom::hidden(&format!("{prefix}-reset-pins"), !has_pins);
        dom::hidden(&format!("{prefix}-pin-undo"), !can_undo);
    } else {
        dom::disabled("graph-map-nav-back", !can_back);
        dom::disabled("graph-map-nav-forward", !can_forward);
        dom::hidden("graph-map-reset-pins", !has_pins);
        dom::hidden("graph-map-pin-undo", !can_undo);
    }
}

pub(in crate::graph_map) fn sync_modal_lens_controls_from_state(st: &V) {
    dom::set_checked("graph-map-whole", m::yes(&st["whole"]));
    dom::set_checked("graph-map-notests", m::yes(&st["notests"]));
    dom::set_checked("graph-map-certain", m::yes(&st["certain"]));
    dom::set_checked("graph-map-patterns", m::yes(&st["patterns"]));
    dom::set_checked("graph-map-history", m::yes(&st["history"]));
    dom::set_checked("graph-map-modules", m::yes(&st["modules"]));
    dom::set_checked("graph-map-no-incoming", m::yes(&st["no_incoming"]));
    dom::set_checked("graph-map-fit", m::yes(&st["fit"]));
    dom::set_value("graph-map-filter", &m::text(st, "filter"));
    dom::set_value(
        "graph-map-isolate-depth",
        &st["isolate_depth"].as_u64().unwrap_or(2).to_string(),
    );
    let failing_lines: Vec<String> = m::array(&st["failing"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    dom::set_value("graph-map-failing", &failing_lines.join("\n"));
}

pub(in crate::graph_map) fn navigate_surface_history(
    surface: MapSurface,
    dir: isize,
    jump_idx: Option<usize>,
) -> bool {
    let entry_opt = SURFACE_NAV_HISTORY.with(|h| {
        let mut map = h.borrow_mut();
        let hist = map.get_mut(surface.canvas)?;
        if let Some(idx) = jump_idx {
            hist.jump_to(idx).cloned()
        } else if dir < 0 {
            hist.back().cloned()
        } else {
            hist.forward().cloned()
        }
    });
    let Some(entry) = entry_opt else {
        sync_history_and_pin_controls(surface, &surface_state(surface));
        return false;
    };
    let nav_epoch = SURFACE_NAV_EPOCHS.with(|e| {
        let mut map = e.borrow_mut();
        let next = map
            .get(surface.canvas)
            .copied()
            .unwrap_or(0)
            .saturating_add(1);
        map.insert(surface.canvas, next);
        next
    });
    let snapshot_opt = SURFACE_NAV_SNAPSHOTS.with(|s| {
        s.borrow()
            .get(&(surface.canvas, format!("{}\0{}", entry.key, entry.target)))
            .cloned()
    });
    let mut state = surface_state(surface);
    let cur_target = effective_target(&state);
    let prev_restoring = NAV_RESTORING.with(|f| f.replace(true));
    let (new_focus, new_scope) = if let Some(f) = entry.key.strip_prefix("focus:") {
        (f.to_owned(), m::text(&state, "scope"))
    } else if let Some(sc) = entry.key.strip_prefix("scope:") {
        (String::new(), sc.to_owned())
    } else {
        (String::new(), String::new())
    };
    let prev_patterns = m::yes(&state["patterns"]);
    let prev_history = m::yes(&state["history"]);
    let prev_modules = m::yes(&state["modules"]);
    let prev_notests = m::yes(&state["notests"]);
    let prev_certain = m::yes(&state["certain"]);
    let prev_no_incoming = m::yes(&state["no_incoming"]);
    if let Some(snap) = snapshot_opt {
        apply_surface_history_snapshot(&mut state, &snap);
    } else {
        state["focus"] = json!(new_focus);
        state["scope"] = json!(new_scope);
        if entry.target != cur_target {
            state["target"] = json!(entry.target);
            state["compare_base"] = json!("");
        }
    }
    let scope_changed = m::text(&state, "scope") != new_scope
        || entry.target != cur_target
        || m::yes(&state["patterns"]) != prev_patterns
        || m::yes(&state["history"]) != prev_history
        || m::yes(&state["modules"]) != prev_modules
        || m::yes(&state["notests"]) != prev_notests
        || m::yes(&state["certain"]) != prev_certain
        || m::yes(&state["no_incoming"]) != prev_no_incoming
        || snapshot_opt_changed_scope(&state, &surface_state(surface));
    save_surface_state(surface, &state);
    if !surface.embedded {
        sync_modal_lens_controls_from_state(&state);
    }
    let mut cam = load_surface_camera(surface);
    cam.apply_viewport_cam(entry.cam);
    cam.fit_mode = false;
    cam.needs_measure = false;
    save_surface_camera(surface, &cam);
    if scope_changed {
        spawn_local(async move {
            if surface.embedded {
                load_embedded_surface(surface, false).await;
            } else {
                load(false).await;
            }
            let current_nav =
                SURFACE_NAV_EPOCHS.with(|e| e.borrow().get(surface.canvas).copied().unwrap_or(0));
            if current_nav != nav_epoch {
                return;
            }
            let mut restored = load_surface_camera(surface);
            restored.apply_viewport_cam(entry.cam);
            restored.fit_mode = false;
            save_surface_camera(surface, &restored);
            apply_surface_camera(surface, &restored);
            NAV_RESTORING.with(|f| f.set(prev_restoring));
            sync_history_and_pin_controls(surface, &surface_state(surface));
        });
    } else {
        rerender_surface_preserving_camera(surface);
        apply_surface_camera(surface, &cam);
        NAV_RESTORING.with(|f| f.set(prev_restoring));
        sync_history_and_pin_controls(surface, &state);
    }
    true
}

/// Restores navigation-snapshot fields onto `state`, resetting nullable and
/// optional fields whenever `snap` explicitly recorded `null` or omitted them.
pub(in crate::graph_map) fn apply_surface_history_snapshot(state: &mut V, snap: &V) {
    for key in ["focus", "scope", "target", "filter", "follow", "side"] {
        state[key] = json!(m::text(snap, key));
    }
    state["side_tab"] = json!(match m::text(snap, "side_tab").as_str() {
        k @ ("changes" | "analysis") => k,
        _ => "details",
    });
    for key in [
        "compare_base",
        "run_id",
        "launch",
        "compare_launch",
        "agent",
        "traced_path",
        "cone",
        "pick",
        "cycle",
        "cycle_base",
        "cycle_return",
        "pattern",
        "pattern_return",
        "scenario",
        "scenario_return",
        "test_path_return",
    ] {
        state[key] = snap[key].clone();
    }
    for key in [
        "patterns",
        "modules",
        "history",
        "lens_tests",
        "lens_runs",
        "isolate_active",
        "whole",
        "notests",
        "certain",
        "no_incoming",
        "shared",
    ] {
        state[key] = json!(m::yes(&snap[key]));
    }
    state["isolate_depth"] = json!(snap["isolate_depth"].as_u64().unwrap_or(2).clamp(1, 3));
    state["scenario_skip"] = json!(snap["scenario_skip"].as_u64().unwrap_or(0));
    state["offset"] = json!(snap["offset"].as_u64().unwrap_or(0));
    for key in [
        "failing", "out", "into", "spread", "spreads", "path", "skip",
    ] {
        state[key] = if snap[key].is_array() {
            snap[key].clone()
        } else {
            json!([])
        };
    }
}

pub(in crate::graph_map) fn snapshot_opt_changed_scope(next: &V, prev: &V) -> bool {
    m::text(next, "scope") != m::text(prev, "scope")
        || m::text(next, "target") != m::text(prev, "target")
        || m::text(next, "compare_base") != m::text(prev, "compare_base")
        || m::text(next, "run_id") != m::text(prev, "run_id")
        || m::text(next, "launch") != m::text(prev, "launch")
        || m::text(next, "follow") != m::text(prev, "follow")
        || m::array(&next["failing"]) != m::array(&prev["failing"])
        || m::array(&next["out"]) != m::array(&prev["out"])
        || m::array(&next["into"]) != m::array(&prev["into"])
        || m::array(&next["spread"]) != m::array(&prev["spread"])
        || m::array(&next["path"]) != m::array(&prev["path"])
        || m::yes(&next["lens_tests"]) != m::yes(&prev["lens_tests"])
        || m::yes(&next["lens_runs"]) != m::yes(&prev["lens_runs"])
        || m::yes(&next["whole"]) != m::yes(&prev["whole"])
        || m::yes(&next["shared"]) != m::yes(&prev["shared"])
        || m::yes(&next["isolate_active"]) != m::yes(&prev["isolate_active"])
        || next["isolate_depth"].as_u64().unwrap_or(2).clamp(1, 3)
            != prev["isolate_depth"].as_u64().unwrap_or(2).clamp(1, 3)
        || next["offset"].as_u64().unwrap_or(0) != prev["offset"].as_u64().unwrap_or(0)
        || next["scenario_skip"].as_u64().unwrap_or(0)
            != prev["scenario_skip"].as_u64().unwrap_or(0)
        || next["cycle"] != prev["cycle"]
        || next["cycle_base"] != prev["cycle_base"]
        || next["pattern"] != prev["pattern"]
        || next["scenario"] != prev["scenario"]
}
