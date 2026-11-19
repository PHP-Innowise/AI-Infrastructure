use super::*;
use std::cell::Cell;

type PinMapSnapshot = BTreeMap<String, (f64, f64)>;

thread_local! {
    pub(super) static SURFACE_NAV_HISTORY: RefCell<HashMap<&'static str, graph_viewport::CameraSelectionHistory>> =
        RefCell::new(HashMap::new());
    pub(super) static SURFACE_PIN_UNDO: RefCell<HashMap<&'static str, Vec<PinMapSnapshot>>> =
        RefCell::new(HashMap::new());
    pub(super) static LAST_DENSE_HOVER: RefCell<HashMap<&'static str, Option<(u8, usize)>>> =
        RefCell::new(HashMap::new());
    static NAV_RESTORING: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn current_dense_hover(canvas_id: &str) -> Option<(u8, usize)> {
    LAST_DENSE_HOVER.with(|h| h.borrow().get(canvas_id).copied().flatten())
}

pub(super) fn set_dense_hover(canvas_id: &'static str, next: Option<(u8, usize)>) -> bool {
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

pub(super) fn push_pin_undo_snapshot(surface: MapSurface, pin_key: &str) {
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

pub(super) fn can_undo_pins(surface: MapSurface) -> bool {
    SURFACE_PIN_UNDO.with(|u| {
        u.borrow()
            .get(surface.canvas)
            .is_some_and(|s| !s.is_empty())
    })
}

pub(super) fn undo_surface_pins(surface: MapSurface) -> bool {
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

pub(super) fn unpin_surface_node(surface: MapSurface, node_key: &str) -> bool {
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

pub(super) fn record_surface_history(surface: MapSurface, state: &V, view: &V, cam: &Camera) {
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
            key,
            label,
            target,
            cam: cam.viewport_cam(),
        });
    });
    sync_history_and_pin_controls(surface, state);
}

pub(super) fn update_surface_active_camera(surface: MapSurface, cam: &Camera) {
    if NAV_RESTORING.with(Cell::get) {
        return;
    }
    SURFACE_NAV_HISTORY.with(|h| {
        if let Some(hist) = h.borrow_mut().get_mut(surface.canvas) {
            hist.update_active_camera(cam.viewport_cam());
        }
    });
}

pub(super) fn surface_history_trail_html(surface: MapSurface) -> String {
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

pub(super) fn sync_history_and_pin_controls(surface: MapSurface, state: &V) {
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

pub(super) fn navigate_surface_history(
    surface: MapSurface,
    dir: isize,
    jump_idx: Option<usize>,
) -> bool {
    let entry_opt = SURFACE_NAV_HISTORY.with(|h| {
        let mut map = h.borrow_mut();
        let hist = map.get_mut(surface.canvas)?;
        let res = if let Some(idx) = jump_idx {
            hist.jump_to(idx).cloned()
        } else if dir < 0 {
            hist.back().cloned()
        } else {
            hist.forward().cloned()
        };
        res
    });
    let Some(entry) = entry_opt else {
        sync_history_and_pin_controls(surface, &surface_state(surface));
        return false;
    };
    let mut state = surface_state(surface);
    let cur_target = effective_target(&state);
    let prev_restoring = NAV_RESTORING.with(|f| {
        let was = f.get();
        f.set(true);
        was
    });
    let (new_focus, new_scope) = if let Some(f) = entry.key.strip_prefix("focus:") {
        (f.to_owned(), m::text(&state, "scope"))
    } else if let Some(sc) = entry.key.strip_prefix("scope:") {
        (String::new(), sc.to_owned())
    } else {
        (String::new(), String::new())
    };
    let scope_changed = new_scope != m::text(&state, "scope") || entry.target != cur_target;
    state["focus"] = json!(new_focus);
    state["scope"] = json!(new_scope);
    if entry.target != cur_target {
        state["target"] = json!(entry.target);
        state["compare_base"] = json!("");
    }
    save_surface_state(surface, &state);
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

#[derive(Clone, Copy)]
pub(super) struct MapSurface {
    pub(super) name: &'static str,
    pub(super) id_prefix: &'static str,
    pub(super) canvas: &'static str,
    pub(super) side: &'static str,
    pub(super) note: &'static str,
    pub(super) error_id: &'static str,
    pub(super) attachment_key: &'static str,
    pub(super) scope_model_key: &'static str,
    pub(super) state_model_key: &'static str,
    pub(super) camera_key: &'static str,
    pub(super) focus_key: &'static str,
    pub(super) drag_key: &'static str,
    pub(super) pan_target_key: &'static str,
    pub(super) view_key: &'static str,
    pub(super) map_epoch: &'static str,
    pub(super) changes_key: &'static str,
    pub(super) changes_epoch: &'static str,
    pub(super) impact_key: &'static str,
    pub(super) impact_epoch: &'static str,
    pub(super) wheel_timer_key: &'static str,
    pub(super) refit_timer_key: &'static str,
    pub(super) filter_timer_key: &'static str,
    pub(super) drag_timer_key: &'static str,
    pub(super) embedded: bool,
}
pub(super) const MODAL_SURFACE: MapSurface = MapSurface {
    name: "modal",
    id_prefix: "graph-map",
    canvas: "graph-map-canvas",
    side: "graph-map-side",
    note: "graph-map-note",
    error_id: "graph-map-error",
    attachment_key: "graph-map-attachment",
    scope_model_key: "graph-map-scope",
    state_model_key: "graph-map",
    camera_key: "graph-map-camera",
    focus_key: "",
    drag_key: "graph-map-drag",
    pan_target_key: "",
    view_key: "graph-map-view",
    map_epoch: "graph-map",
    changes_key: "graph-map-changes",
    changes_epoch: "graph-map-changes-fetch",
    impact_key: "graph-map-impact",
    impact_epoch: "graph-map-impact-fetch",
    wheel_timer_key: "graph-map-wheel",
    refit_timer_key: "graph-map-refit",
    filter_timer_key: "graph-map-filter",
    drag_timer_key: "graph-map-drag-clear",
    embedded: false,
};
pub(super) const SESSION_SURFACE: MapSurface = MapSurface {
    name: "session",
    id_prefix: "session-graph",
    canvas: "session-graph-canvas",
    side: "session-graph-side",
    note: "session-graph-note",
    error_id: "session-graph-error",
    attachment_key: "session-graph-attachment",
    scope_model_key: "session-graph-scope",
    state_model_key: "session-graph-state",
    camera_key: "session-graph-camera",
    focus_key: "session-graph-focus",
    drag_key: "session-graph-drag",
    pan_target_key: "session-graph-pan-target",
    view_key: "session-graph-view",
    map_epoch: "session-graph",
    changes_key: "session-graph-changes",
    changes_epoch: "session-graph-changes-fetch",
    impact_key: "session-graph-impact",
    impact_epoch: "session-graph-impact-fetch",
    wheel_timer_key: "session-graph-wheel",
    refit_timer_key: "session-graph-refit",
    filter_timer_key: "session-graph-filter",
    drag_timer_key: "session-graph-drag-clear",
    embedded: true,
};
pub(super) const SYSTEM_EMBEDDED_SURFACE: MapSurface = MapSurface {
    name: "system-member",
    id_prefix: "system-member-graph",
    canvas: "system-member-graph-canvas",
    side: "system-member-graph-side",
    note: "system-member-graph-note",
    error_id: "system-member-graph-error",
    attachment_key: "system-member-graph-attachment",
    scope_model_key: "system-member-graph-scope",
    state_model_key: "system-member-graph-state",
    camera_key: "system-member-graph-camera",
    focus_key: "system-member-graph-focus",
    drag_key: "system-member-graph-drag",
    pan_target_key: "system-member-graph-pan-target",
    view_key: "system-member-graph-view",
    map_epoch: "system-member-graph",
    changes_key: "system-member-graph-changes",
    changes_epoch: "system-member-graph-changes-fetch",
    impact_key: "system-member-graph-impact",
    impact_epoch: "system-member-graph-impact-fetch",
    wheel_timer_key: "system-member-graph-wheel",
    refit_timer_key: "system-member-graph-refit",
    filter_timer_key: "system-member-graph-filter",
    drag_timer_key: "system-member-graph-drag-clear",
    embedded: true,
};
pub(super) const SYSTEM_CHANGE_SURFACE: MapSurface = MapSurface {
    name: "system-change",
    id_prefix: "system-change-graph",
    canvas: "system-change-graph-canvas",
    side: "system-change-graph-side",
    note: "system-change-graph-note",
    error_id: "system-change-graph-error",
    attachment_key: "system-change-graph-attachment",
    scope_model_key: "system-change-graph-scope",
    state_model_key: "system-change-graph-state",
    camera_key: "system-change-graph-camera",
    focus_key: "system-change-graph-focus",
    drag_key: "system-change-graph-drag",
    pan_target_key: "system-change-graph-pan-target",
    view_key: "system-change-graph-view",
    map_epoch: "system-change-graph",
    changes_key: "system-change-graph-changes",
    changes_epoch: "system-change-graph-changes-fetch",
    impact_key: "system-change-graph-impact",
    impact_epoch: "system-change-graph-impact-fetch",
    wheel_timer_key: "system-change-graph-wheel",
    refit_timer_key: "system-change-graph-refit",
    filter_timer_key: "system-change-graph-filter",
    drag_timer_key: "system-change-graph-drag-clear",
    embedded: true,
};
impl SystemEmbeddedKind {
    pub(super) fn surface(self) -> MapSurface {
        match self {
            Self::Member => SYSTEM_EMBEDDED_SURFACE,
            Self::Change => SYSTEM_CHANGE_SURFACE,
        }
    }
}
pub(super) fn surface_by_name(name: &str) -> MapSurface {
    match name {
        "session" => SESSION_SURFACE,
        "system-member" => SYSTEM_EMBEDDED_SURFACE,
        "system-change" => SYSTEM_CHANGE_SURFACE,
        _ => MODAL_SURFACE,
    }
}
pub(super) fn surface_scope(surface: MapSurface) -> GraphTargetScope {
    if surface.name == "session" {
        return GraphTargetScope::single(dom::value("project"));
    }
    if surface.name == "modal" {
        if let Some(stored) = GraphTargetScope::from_json(&m::data(MODAL_SURFACE.scope_model_key)) {
            if stored.is_system() {
                return stored;
            }
            let live_project = dom::value("project");
            if !live_project.is_empty() {
                return GraphTargetScope::single(live_project);
            }
            return stored;
        }
        return GraphTargetScope::single(dom::value("project"));
    }
    GraphTargetScope::from_json(&m::data(surface.scope_model_key))
        .unwrap_or_else(|| GraphTargetScope::single(""))
}
pub(super) fn set_surface_scope(surface: MapSurface, scope: GraphTargetScope) {
    m::set(surface.scope_model_key, scope.to_json());
}
pub(super) fn surface_state(surface: MapSurface) -> V {
    let raw = m::data(surface.state_model_key);
    if raw.is_object() {
        normalize_map_state(raw)
    } else {
        fresh("")
    }
}
pub(super) fn effective_target_from(target: &str, compare_base: &str) -> String {
    let target = target.trim();
    let base = compare_base.trim();
    if !base.is_empty()
        && !target.is_empty()
        && target != "work"
        && !target.starts_with("range:")
        && !target.starts_with("sysrun:")
    {
        let head = target.strip_prefix("commit:").unwrap_or(target).trim();
        if !head.is_empty() && head != base {
            return format!("range:{base}..{head}");
        }
    }
    target.to_owned()
}
pub(super) fn effective_target(state: &V) -> String {
    effective_target_from(&m::text(state, "target"), &m::text(state, "compare_base"))
}
pub(super) fn save_surface_state(surface: MapSurface, state: &V) {
    m::set(surface.state_model_key, normalize_map_state(state.clone()));
}
pub(super) fn surface_changes_doc(surface: MapSurface, state: &V) -> Option<V> {
    let raw = m::data(surface.changes_key);
    let data = if raw["data"].is_object() {
        let scope = surface_scope(surface);
        if scope.is_empty() {
            return None;
        }
        let scope_key = scope.scope_key();
        let target = effective_target(state);
        let target_key = if target.is_empty() {
            "work"
        } else {
            target.as_str()
        };
        let stored_key = {
            let sk = m::text(&raw, "scope_key");
            if !sk.is_empty() {
                sk
            } else {
                let proj = m::text(&raw, "project");
                if proj.is_empty() {
                    String::new()
                } else {
                    format!("project:{proj}")
                }
            }
        };
        if stored_key == scope_key && m::text(&raw, "target") == target_key {
            raw["data"].clone()
        } else {
            return None;
        }
    } else if raw["files"].is_array() {
        raw
    } else {
        return None;
    };
    Some(data)
}
pub(super) fn impact_cache_key(scope_key: &str, path: &str, state: &V) -> String {
    let raw_target = effective_target(state);
    let target = if raw_target.is_empty() {
        "work"
    } else {
        raw_target.as_str()
    };
    let certain = if m::yes(&state["certain"]) { "1" } else { "0" };
    format!(
        "{scope_key}\n{}\n{target}\n{certain}",
        normalize_rel_path(path)
    )
}
pub(super) fn looks_like_file_path(path: &str) -> bool {
    let norm = normalize_rel_path(strip_symbol_suffix(path));
    if norm.is_empty()
        || norm.starts_with("module:")
        || norm.starts_with("layer:")
        || norm.starts_with("role:")
    {
        return false;
    }
    norm.rsplit('/')
        .next()
        .is_some_and(|name| name.contains('.'))
}
pub(super) fn is_explicit_folder(path: &str, view: Option<&V>) -> bool {
    let Some(view) = view else {
        return false;
    };
    if normalize_rel_path(&m::text(&view["focus"], "path")) == path
        && m::yes(&view["focus"]["folder"])
    {
        return true;
    }
    m::array(&view["nodes"])
        .iter()
        .any(|node| normalize_rel_path(&m::text(node, "path")) == path && m::yes(&node["folder"]))
}
pub(super) fn is_known_file(path: &str, view: Option<&V>, changes: Option<&V>) -> bool {
    if let Some(view) = view {
        if normalize_rel_path(&m::text(&view["focus"], "path")) == path
            && view["focus"].is_object()
            && !m::yes(&view["focus"]["folder"])
        {
            return true;
        }
        if m::array(&view["nodes"]).iter().any(|node| {
            normalize_rel_path(&m::text(node, "path")) == path
                && !m::yes(&node["folder"])
                && !m::yes(&node["symbol"])
        }) {
            return true;
        }
    }
    if let Some(changes) = changes {
        if m::array(&changes["files"])
            .iter()
            .any(|file| normalize_rel_path(&m::text(file, "path")) == path)
        {
            return true;
        }
    }
    looks_like_file_path(path)
}
pub fn selected_impact_file(state: &V, view: Option<&V>, changes: Option<&V>) -> Option<String> {
    let focus = normalize_rel_path(strip_symbol_suffix(&m::text(state, "focus")));
    if !focus.is_empty() {
        if is_explicit_folder(&focus, view) {
            return None;
        }
        if is_known_file(&focus, view, changes) {
            return Some(focus);
        }
        return None;
    }
    let scope = normalize_rel_path(strip_symbol_suffix(&m::text(state, "scope")));
    if !scope.is_empty()
        && !is_explicit_folder(&scope, view)
        && is_known_file(&scope, view, changes)
    {
        return Some(scope);
    }
    None
}
pub(super) fn cached_impact_for_selection(
    surface: MapSurface,
    state: &V,
    view: Option<&V>,
) -> Option<V> {
    let changes = surface_changes_doc(surface, state);
    let path = selected_impact_file(state, view, changes.as_ref())?;
    let scope = surface_scope(surface);
    if scope.is_empty() {
        return None;
    }
    let scope_key = scope.scope_key();
    let raw = m::data(surface.impact_key);
    if !raw.is_object() {
        return None;
    }
    if raw["key"].is_string() {
        let wanted = impact_cache_key(&scope_key, &path, state);
        if m::text(&raw, "key") == wanted && raw["data"].is_object() {
            return Some(raw["data"].clone());
        }
        return None;
    }
    if normalize_rel_path(strip_symbol_suffix(&m::text(&raw, "path"))) == path {
        return Some(raw);
    }
    None
}
#[cfg(target_arch = "wasm32")]
pub(super) fn encode_part(value: &str) -> String {
    dom::enc(value)
}
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn encode_part(value: &str) -> String {
    let mut out = String::new();
    for b in value.as_bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => out.push(*b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
#[cfg(target_arch = "wasm32")]
pub(super) fn encode_query(params: &[(&str, String)]) -> String {
    dom::query(params)
}
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn encode_query(params: &[(&str, String)]) -> String {
    if params.is_empty() {
        return String::new();
    }
    let parts = params
        .iter()
        .map(|(k, v)| format!("{}={}", encode_part(k), encode_part(v)))
        .collect::<Vec<_>>();
    format!("?{}", parts.join("&"))
}
pub(super) fn effective_sysrun_id(
    scope: &GraphTargetScope,
    target: &str,
    explicit_run_id: Option<&str>,
) -> Option<String> {
    if let Some(rid) = target
        .strip_prefix("sysrun:")
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(rid.to_owned());
    }
    if let Some(rid) = explicit_run_id.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(rid.to_owned());
    }
    if target == "sysrun" {
        if let GraphTargetScope::SystemService {
            run_id: Some(rid), ..
        } = scope
        {
            let trimmed = rid.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }
    }
    None
}
pub(super) fn format_scoped_endpoint(
    scope: &GraphTargetScope,
    endpoint: &str,
    query: Vec<(&str, String)>,
) -> String {
    let endpoint = endpoint.trim_start_matches('/');
    match scope {
        GraphTargetScope::SingleProject { project_id } => format!(
            "/api/projects/{}/graph/{}{}",
            encode_part(project_id),
            endpoint,
            encode_query(&query)
        ),
        GraphTargetScope::SystemService {
            project_id,
            config_path,
            service_id,
            ..
        } => {
            let mut full_query = Vec::with_capacity(query.len() + 3);
            full_query.push(("project_id", project_id.clone()));
            full_query.push(("config_path", config_path.clone()));
            full_query.push(("service", service_id.clone()));
            full_query.extend(query);
            format!(
                "/api/systems/service-graph/{}{}",
                endpoint,
                encode_query(&full_query)
            )
        }
    }
}
pub fn build_scoped_changes_path(
    scope: &GraphTargetScope,
    target: &str,
    focus_session: &str,
    notests: bool,
) -> String {
    let effective_target = if target.is_empty() { "work" } else { target };
    let mut query = vec![("target", effective_target.to_owned())];
    if scope.is_system() {
        if let Some(rid) = effective_sysrun_id(scope, effective_target, None) {
            query.push(("run_id", rid));
        }
    }
    if !focus_session.is_empty() {
        query.push(("focus_session", focus_session.to_owned()));
    }
    if notests {
        query.push(("notests", "1".to_owned()));
    }
    format_scoped_endpoint(scope, "changes", query)
}
#[allow(dead_code)]
pub fn build_changes_path(
    project: &str,
    target: &str,
    focus_session: &str,
    notests: bool,
) -> String {
    build_scoped_changes_path(
        &GraphTargetScope::single(project),
        target,
        focus_session,
        notests,
    )
}
pub fn build_scoped_file_impact_path(
    scope: &GraphTargetScope,
    path: &str,
    target: &str,
    focus_session: &str,
    certain: bool,
) -> String {
    let effective_target = if target.is_empty() { "work" } else { target };
    let mut query = vec![
        ("path", normalize_rel_path(strip_symbol_suffix(path))),
        ("target", effective_target.to_owned()),
    ];
    if scope.is_system() {
        if let Some(rid) = effective_sysrun_id(scope, effective_target, None) {
            query.push(("run_id", rid));
        }
    }
    if !focus_session.is_empty() {
        query.push(("focus_session", focus_session.to_owned()));
    }
    if certain {
        query.push(("certain", "1".to_owned()));
    }
    format_scoped_endpoint(scope, "file-impact", query)
}
#[allow(dead_code)]
pub fn build_file_impact_path(
    project: &str,
    path: &str,
    target: &str,
    focus_session: &str,
    certain: bool,
) -> String {
    build_scoped_file_impact_path(
        &GraphTargetScope::single(project),
        path,
        target,
        focus_session,
        certain,
    )
}
pub fn state_focus_file(mut state: V, raw_path: &str, view: &V) -> (V, bool) {
    let file_path = normalize_rel_path(strip_symbol_suffix(raw_path));
    if file_path.is_empty() {
        return (state, false);
    }
    state = normalize_map_state(unfollowed(state));
    state["modules"] = json!(false);
    state["cycle"] = V::Null;
    state["pattern"] = V::Null;
    state["scenario"] = V::Null;
    let visible = m::array(&view["nodes"])
        .iter()
        .any(|node| normalize_rel_path(&m::text(node, "path")) == file_path);
    if !visible {
        state["scope"] = json!("");
        if !m::yes(&state["whole"]) {
            let folder = parent(&file_path);
            if !folder.is_empty() {
                let mut acc = String::new();
                for part in folder.split('/').filter(|p| !p.is_empty()) {
                    if !acc.is_empty() {
                        acc.push('/');
                    }
                    acc.push_str(part);
                    state = spread(state, &acc, false);
                }
            }
        }
        if !m::text(&state, "filter").is_empty() {
            state["filter"] = json!("");
        }
    }
    let already_focused = normalize_rel_path(strip_symbol_suffix(&m::text(&state, "focus")))
        == file_path
        && normalize_rel_path(&m::text(&view["focus"], "path")) == file_path;
    state["focus"] = json!(file_path);
    state["side_tab"] = json!("details");
    state["side_pinned"] = json!(true);
    let needs_reload = !visible || !already_focused;
    (state, needs_reload)
}
pub(super) fn changes_count_of(changes: Option<&V>) -> u64 {
    let Some(doc) = changes.filter(|v| v.is_object()) else {
        return 0;
    };
    doc["total_files"]
        .as_u64()
        .unwrap_or_else(|| m::array(&doc["files"]).len() as u64)
}
pub(super) fn update_surface_changes_badge(surface: MapSurface, changes: Option<&V>) {
    if !surface.embedded {
        return;
    }
    let badge_id = format!("{}-changes-count", surface.id_prefix);
    let n = changes_count_of(changes);
    dom::text(&badge_id, &n.to_string());
    dom::hidden(&badge_id, n == 0);
}
pub(super) fn surface_pin_key(surface: MapSurface, state: &V) -> String {
    let scope = surface_scope(surface).scope_key();
    let mode = if m::yes(&state["modules"]) {
        "modules"
    } else if m::yes(&state["patterns"]) {
        "patterns"
    } else if m::yes(&state["history"]) {
        "history"
    } else if !m::text(state, "target").is_empty() {
        "changes"
    } else {
        "whole"
    };
    graph_viewport::pin_storage_key(surface.id_prefix, &scope, mode)
}

pub(super) struct OwnedOverlayExtras {
    pub(super) pins: BTreeMap<String, (f64, f64)>,
    pub(super) path_nodes: BTreeSet<usize>,
    pub(super) path_edges: BTreeSet<(usize, usize)>,
    pub(super) isolate_cone: Option<BTreeSet<usize>>,
    pub(super) viewport_bounds: Option<ViewportBounds>,
    pub(super) roving_focus: Option<usize>,
}

pub(super) fn resolve_node_index_by_query(nodes: &[V], raw_q: &str) -> Option<usize> {
    let q = raw_q.trim().to_ascii_lowercase();
    if q.is_empty() {
        return None;
    }
    nodes
        .iter()
        .position(|n| {
            m::text(n, "path").eq_ignore_ascii_case(&q)
                || m::text(n, "name").eq_ignore_ascii_case(&q)
        })
        .or_else(|| {
            nodes.iter().position(|n| {
                m::text(n, "path").to_ascii_lowercase().contains(&q)
                    || m::text(n, "name").to_ascii_lowercase().contains(&q)
            })
        })
}

pub(super) fn build_overlay_extras(surface: MapSurface, view: &V, state: &V) -> OwnedOverlayExtras {
    let pins = graph_viewport::load_pins(&surface_pin_key(surface, state));
    let nodes = m::array(&view["nodes"]);
    let edges = m::array(&view["edges"]);
    let n_count = nodes.len();
    let path_index = build_path_index(&nodes);
    let at = |p: &V| lookup_path_index(&nodes, &path_index, p);
    let mut out_adj = vec![Vec::new(); n_count];
    let mut in_adj = vec![Vec::new(); n_count];
    for e in &edges {
        if let (Some(u), Some(v)) = (at(&e["from"]), at(&e["to"])) {
            if u != v {
                out_adj[u].push(v);
                in_adj[v].push(u);
            }
        }
    }

    let mut path_nodes = BTreeSet::new();
    let mut path_edges = BTreeSet::new();
    let p_from = m::text(&state["traced_path"], "from");
    let p_to = m::text(&state["traced_path"], "to");
    if !p_from.is_empty() && !p_to.is_empty() {
        if let (Some(u), Some(v)) = (
            resolve_node_index_by_query(&nodes, &p_from),
            resolve_node_index_by_query(&nodes, &p_to),
        ) {
            if let Some(seq) = graph_viewport::shortest_path_bfs(n_count, &out_adj, false, u, v)
                .or_else(|| graph_viewport::shortest_path_bfs(n_count, &out_adj, true, u, v))
            {
                for w in seq.windows(2) {
                    path_nodes.insert(w[0]);
                    path_nodes.insert(w[1]);
                    path_edges.insert((w[0], w[1]));
                }
                if seq.len() == 1 {
                    path_nodes.insert(seq[0]);
                }
            }
        }
    }

    let isolate_cone = if m::yes(&state["isolate_active"]) {
        let seed_q = m::text(state, "focus");
        resolve_node_index_by_query(&nodes, &seed_q).map(|seed_idx| {
            let depth = state["isolate_depth"].as_u64().unwrap_or(2).clamp(1, 3) as usize;
            let dist =
                graph_viewport::hop_cone_distances(n_count, &out_adj, &in_adj, &[seed_idx], depth);
            dist.into_iter()
                .enumerate()
                .filter_map(|(idx, d)| d.map(|_| idx))
                .chain(path_nodes.iter().copied())
                .collect::<BTreeSet<usize>>()
        })
    } else {
        None
    };

    let cam = load_surface_camera(surface);
    let (vw, vh, measured) = viewport_size(surface.canvas);
    let viewport_bounds = measured
        .then(|| {
            visible_viewport_bounds((cam.pan_x, cam.pan_y), cam.scale, (vw, vh), BOX_WIDTH * 1.5)
        })
        .flatten();
    let roving_focus = state["roving_focus"].as_u64().map(|v| v as usize);

    OwnedOverlayExtras {
        pins,
        path_nodes,
        path_edges,
        isolate_cone,
        viewport_bounds,
        roving_focus,
    }
}

pub(crate) fn show_non_destructive_stale_banner(project_id: &str, message: &str) {
    let text = if message.trim().is_empty() {
        "Graph updated — Refresh"
    } else {
        message.trim()
    };
    if open() {
        let modal_proj = surface_scope(MODAL_SURFACE).project_id().to_owned();
        if project_id.is_empty() || modal_proj.is_empty() || modal_proj == project_id {
            dom::text("graph-map-stale-text", text);
            dom::hidden("graph-map-stale-banner", false);
        }
    }
    for surface in [
        SESSION_SURFACE,
        SYSTEM_EMBEDDED_SURFACE,
        SYSTEM_CHANGE_SURFACE,
    ] {
        if dom::get(surface.canvas).is_some() {
            let surf_proj = surface_scope(surface).project_id().to_owned();
            if project_id.is_empty() || surf_proj.is_empty() || surf_proj == project_id {
                dom::text(&format!("{}-stale-text", surface.id_prefix), text);
                dom::hidden(&format!("{}-stale-banner", surface.id_prefix), false);
            }
        }
    }
}

pub(super) fn active_modal_lens(state: &V, view: &V) -> &'static str {
    if m::yes(&state["history"]) || m::text(view, "mode") == "history" {
        "runs"
    } else if !m::array(&state["failing"]).is_empty()
        || m::text(view, "mode") == "tests"
        || m::yes(&state["lens_tests"])
    {
        "tests"
    } else if m::yes(&state["patterns"])
        || m::yes(&state["modules"])
        || m::text(view, "mode") == "architecture"
        || m::text(view, "mode") == "patterns"
    {
        "architecture"
    } else if !effective_target(state).is_empty() || m::text(view, "mode") == "changes" {
        "changes"
    } else {
        "dependencies"
    }
}

pub(super) fn sync_modal_lens_and_display(state: &V, view: &V) {
    let lens = active_modal_lens(state, view);
    for candidate in ["dependencies", "changes", "architecture", "tests", "runs"] {
        dom::attr(
            &format!("graph-map-lens-{candidate}"),
            "aria-selected",
            if candidate == lens { "true" } else { "false" },
        );
    }
    let graph_status = if surface_scope(MODAL_SURFACE).is_system() {
        m::data("system-map-status-cache")
    } else {
        m::data("graph")
    };
    crate::graph::sync_freshness_pill("graph-map", &graph_status, view);
    sync_history_and_pin_controls(MODAL_SURFACE, state);
    dom::attr(
        "graph-map-isolate-toggle",
        "aria-pressed",
        if m::yes(&state["isolate_active"]) {
            "true"
        } else {
            "false"
        },
    );
    let has_path = !m::text(&state["traced_path"], "from").is_empty()
        && !m::text(&state["traced_path"], "to").is_empty();
    dom::hidden("graph-map-path-clear", !has_path);
}

pub(super) fn sync_surface_toolbar(surface: MapSurface, state: &V, changes: Option<&V>) {
    if !surface.embedded {
        return;
    }
    let prefix = surface.id_prefix;
    let target = effective_target(state);
    let changes_mode = !target.is_empty();
    let whole = m::yes(&state["whole"]);
    dom::attr(
        &format!("{prefix}-mode-whole"),
        "aria-pressed",
        if changes_mode { "false" } else { "true" },
    );
    dom::attr(
        &format!("{prefix}-mode-changes"),
        "aria-pressed",
        if changes_mode { "true" } else { "false" },
    );
    dom::attr(
        &format!("{prefix}-unfold"),
        "aria-pressed",
        if whole { "true" } else { "false" },
    );
    dom::set_checked(&format!("{prefix}-notests"), m::yes(&state["notests"]));
    let filter_id = format!("{prefix}-filter");
    let wanted_filter = m::text(state, "filter");
    if dom::value(&filter_id) != wanted_filter {
        dom::set_value(&filter_id, &wanted_filter);
    }
    dom::hidden(
        &format!("{prefix}-reset-pins"),
        !graph_viewport::has_pins(&surface_pin_key(surface, state)),
    );
    dom::attr(
        &format!("{prefix}-isolate-toggle"),
        "aria-pressed",
        if m::yes(&state["isolate_active"]) {
            "true"
        } else {
            "false"
        },
    );
    let has_path = !m::text(&state["traced_path"], "from").is_empty()
        && !m::text(&state["traced_path"], "to").is_empty();
    dom::hidden(&format!("{prefix}-path-clear"), !has_path);
    let graph_status = if surface_scope(surface).is_system() {
        m::data("system-map-status-cache")
    } else {
        m::data("graph")
    };
    let view = m::data(surface.view_key);
    crate::graph::sync_freshness_pill(prefix, &graph_status, &view);
    update_surface_changes_badge(surface, changes);
    sync_history_and_pin_controls(surface, state);
}
pub(super) fn side_tabs_html(side_tab: &str, changes_count: u64, surface: &str) -> String {
    let tab = match side_tab {
        "changes" => "changes",
        "analysis" => "analysis",
        _ => "details",
    };
    let btn = |key: &str, label: String| {
        let sel = tab == key;
        let p = payload(&json!({"tab": key, "surface": surface}));
        format!(
            "<button type=\"button\" class=\"graph-side-tab\" role=\"tab\" aria-selected=\"{sel}\" tabindex=\"{}\" data-action=\"graph-side-tab\" data-tab=\"{key}\" data-payload=\"{p}\">{label}</button>",
            if sel { "0" } else { "-1" }
        )
    };
    format!(
        "<div class=\"graph-side-tabs graph-map-side-tabs\" role=\"tablist\" aria-label=\"Graph inspector tabs\">{}{}{}</div>",
        btn("details", "Details".to_owned()),
        btn(
            "changes",
            format!("Changes <span class=\"graph-map-total\">({changes_count})</span>")
        ),
        btn("analysis", "Analysis".to_owned()),
    )
}
pub(super) fn side_top_html(
    side_tab: &str,
    changes_count: u64,
    surface: &str,
    show_close: bool,
) -> String {
    let tabs = side_tabs_html(side_tab, changes_count, surface);
    if !show_close {
        return format!("<div class=\"graph-side-top\">{tabs}</div>");
    }
    let close_action = match surface {
        "system-member" => "system-member-graph-side-close",
        "system-change" => "system-change-graph-side-close",
        _ => "session-graph-side-close",
    };
    format!(
        "<div class=\"graph-side-top\">{tabs}<button type=\"button\" class=\"button compact graph-side-close\" data-action=\"{close_action}\" title=\"Close details\" aria-label=\"Close details\">Close</button></div>"
    )
}
pub fn changes_side_html(changes: &V, filter: &str, active_focus: &str, surface: &str) -> String {
    if !changes.is_object() {
        return "<div class=\"graph-side-panel\" data-tab=\"changes\"><p class=\"knowledge-note\">Reading workspace changes…</p></div>".to_owned();
    }
    let files = m::array(&changes["files"]);
    let mut created_count = 0u64;
    let mut modified_count = 0u64;
    let mut deleted_count = 0u64;
    for file in &files {
        match m::text(file, "status").to_ascii_lowercase().as_str() {
            "created" => created_count += 1,
            "deleted" => deleted_count += 1,
            "modified" | "renamed" => modified_count += 1,
            _ => {}
        }
    }
    let total_files = changes["total_files"]
        .as_u64()
        .unwrap_or(files.len() as u64);
    let created_entities = changes["created_entities"].as_u64().unwrap_or(0);
    let modified_entities = changes["modified_entities"].as_u64().unwrap_or(0);
    let deleted_entities = changes["deleted_entities"].as_u64().unwrap_or(0);
    let total_vis_warnings: u64 = changes["visibility_warnings"]
        .as_u64()
        .or_else(|| changes["summary"]["visibility_warnings"].as_u64())
        .unwrap_or_else(|| {
            files
                .iter()
                .map(|file| file["summary"]["visibility_warnings"].as_u64().unwrap_or(0))
                .sum()
        });
    let vis_warning_badge = if total_vis_warnings > 0 {
        format!(
            "<span class=\"graph-status-pill\" data-status=\"broken_risk\">!{} visibility {}</span>",
            count(total_vis_warnings),
            if total_vis_warnings == 1 {
                "warning"
            } else {
                "warnings"
            }
        )
    } else {
        String::new()
    };
    let mut html = format!(
        "<div class=\"graph-side-panel\" data-tab=\"changes\"><div class=\"graph-changes-summary\"><span>{} changed {}</span><span>+{} ~{} -{} entities</span>{vis_warning_badge}</div>",
        count(total_files),
        if total_files == 1 { "file" } else { "files" },
        count(created_entities),
        count(modified_entities),
        count(deleted_entities),
    );
    let active_filter = match filter {
        "created" | "modified" | "deleted" => filter,
        _ => "all",
    };
    html.push_str(
        "<div class=\"graph-changes-filters\" role=\"group\" aria-label=\"Filter changed files\">",
    );
    for (key, label, n) in [
        ("all", "All", files.len() as u64),
        ("created", "Created", created_count),
        ("modified", "Modified", modified_count),
        ("deleted", "Deleted", deleted_count),
    ] {
        let fpayload = payload(&json!({"filter": key, "surface": surface}));
        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-changes-filter\" aria-pressed=\"{}\" data-action=\"graph-changes-filter\" data-filter=\"{key}\" data-payload=\"{fpayload}\">{label} <span class=\"graph-map-total\">{}</span></button>",
            if active_filter == key { "true" } else { "false" },
            count(n),
        ));
    }
    html.push_str("</div>");
    let matching: Vec<&V> = files
        .iter()
        .filter(|file| {
            let status = m::text(file, "status").to_ascii_lowercase();
            match active_filter {
                "created" => status == "created",
                "modified" => matches!(status.as_str(), "modified" | "renamed"),
                "deleted" => status == "deleted",
                _ => true,
            }
        })
        .collect();
    if matching.is_empty() {
        html.push_str("<p class=\"knowledge-note\">No changed files match this filter.</p></div>");
        return html;
    }
    let norm_active = normalize_rel_path(strip_symbol_suffix(active_focus));
    html.push_str("<div class=\"graph-changes-list\">");
    for file in matching {
        let path = normalize_rel_path(&m::text(file, "path"));
        let prev = normalize_rel_path(&m::text(file, "previous_path"));
        let status = m::text(file, "status").to_ascii_lowercase();
        let (badge, status_word) = change_status_badge(&status);
        let exists = file["exists"].as_bool().unwrap_or(status != "deleted");
        let in_graph = file["in_graph"].as_bool().unwrap_or(true);
        let sum = &file["summary"];
        let uses_added = sum["uses_added"].as_u64().unwrap_or(0);
        let uses_removed = sum["uses_removed"].as_u64().unwrap_or(0);
        let at_risk = sum["callers_at_risk"].as_u64().unwrap_or(0);
        let impacted = sum["impacted_callers"].as_u64().unwrap_or(0);
        let vis_warnings = sum["visibility_warnings"].as_u64().unwrap_or(0);
        let selected = !norm_active.is_empty() && norm_active == path;
        let file_payload = payload(&json!({"path": path, "surface": surface}));
        html.push_str(&format!(
            "<div class=\"graph-changes-file\" data-selected=\"{selected}\"><button type=\"button\" class=\"graph-changes-file-btn\" data-action=\"graph-map-impact-focus\" data-path=\"{}\" data-payload=\"{file_payload}\" aria-label=\"{}, {status_word}\"><div class=\"graph-changes-file-head\"><span class=\"graph-changes-file-path\">{}</span><span class=\"graph-status-pill\" data-status=\"{}\">{badge}</span></div><div class=\"graph-changes-file-meta\">",
            escape(&path),
            escape(&path),
            escape(&path),
            escape(&status),
        ));
        if !prev.is_empty() && prev != path {
            html.push_str(&format!("<span>from {}</span>", escape(&prev)));
        }
        if !exists {
            html.push_str("<span class=\"graph-status-pill\" data-status=\"broken_risk\">Missing on disk</span>");
        } else if !in_graph {
            html.push_str(
                "<span class=\"graph-status-pill\" data-status=\"unchanged\">Not indexed</span>",
            );
        }
        if uses_added > 0 {
            html.push_str(&format!(
                "<span class=\"graph-status-pill\" data-status=\"added\">+{}</span>",
                count(uses_added)
            ));
        }
        if uses_removed > 0 {
            html.push_str(&format!(
                "<span class=\"graph-status-pill\" data-status=\"removed\">-{}</span>",
                count(uses_removed)
            ));
        }
        if at_risk > 0 {
            html.push_str(&format!(
                "<span class=\"graph-status-pill\" data-status=\"broken_risk\">!{} {}</span>",
                count(at_risk),
                if at_risk == 1 { "caller" } else { "callers" }
            ));
        } else if impacted > 0 {
            html.push_str(&format!(
                "<span class=\"graph-status-pill\" data-status=\"impacted\">~{} callers</span>",
                count(impacted)
            ));
        }
        if vis_warnings > 0 {
            html.push_str(&format!(
                "<span class=\"graph-status-pill\" data-status=\"broken_risk\">!{} visibility {}</span>",
                count(vis_warnings),
                if vis_warnings == 1 {
                    "warning"
                } else {
                    "warnings"
                }
            ));
        }
        html.push_str("</div></button>");
        let entities = m::array(&file["entities"]);
        if !entities.is_empty() {
            html.push_str("<div class=\"graph-changes-entities\">");
            for entity in &entities {
                let ename = m::text(entity, "name");
                let ekind = normalize_entity_kind(&m::text(entity, "kind"));
                let echange = m::text(entity, "change").to_ascii_lowercase();
                let (ebadge, _) = change_status_badge(&echange);
                let line = entity["line"].as_u64().unwrap_or(0);
                let sig = m::yes(&entity["signature_changed"]);
                let vis_pill = visibility_pill_html(entity["visibility"].as_str(), true);
                let prev_vis = entity["previous_visibility"].as_str();
                let curr_vis = entity["visibility"].as_str();
                let vis_changed = m::yes(&entity["visibility_narrowed"])
                    || m::yes(&entity["visibility_changed"])
                    || (prev_vis.is_some() && curr_vis.is_some() && prev_vis != curr_vis);
                let vis_delta = if vis_changed {
                    vis_delta_badge_html(prev_vis, curr_vis, m::yes(&entity["visibility_narrowed"]))
                } else {
                    String::new()
                };
                let epayload = payload(&json!({
                    "path": path,
                    "entity_id": m::text(entity, "id"),
                    "line": line,
                    "surface": surface,
                }));
                let loc = if line > 0 {
                    format!(":{line}")
                } else {
                    String::new()
                };
                html.push_str(&format!(
                    "<button type=\"button\" class=\"graph-changes-entity\" data-action=\"graph-map-impact-focus\" data-path=\"{}\" data-payload=\"{epayload}\"><span class=\"graph-changes-entity-kind\">{}</span>{vis_pill}<span class=\"graph-changes-entity-name\">{}{}</span>{vis_delta}{}<span class=\"graph-status-pill\" data-status=\"{}\">{ebadge}</span></button>",
                    escape(&path),
                    escape(entity_kind_badge(&ekind)),
                    escape(&ename),
                    loc,
                    if sig {
                        "<span class=\"graph-status-pill\" data-status=\"modified\">sig</span>"
                    } else {
                        ""
                    },
                    escape(if echange.is_empty() { "modified" } else { &echange }),
                ));
            }
            html.push_str("</div>");
        }
        html.push_str("</div>");
    }
    html.push_str("</div></div>");
    html
}
pub(super) fn metric_count(val: &V) -> u64 {
    val.as_u64()
        .or_else(|| val.as_array().map(|arr| arr.len() as u64))
        .unwrap_or(0)
}
#[allow(clippy::too_many_arguments)]
pub(super) fn impact_dep_group_html(
    title: &str,
    diff_key: &str,
    deps: &[V],
    self_path: &str,
    surface: &str,
    member_filter: &str,
    default_open: bool,
    collapsible: bool,
) -> String {
    if deps.is_empty() {
        return String::new();
    }
    let mut html = String::new();
    if collapsible {
        html.push_str(&format!(
            "<details class=\"graph-impact-group\"{}><summary><span class=\"graph-impact-group-title\">{title} <span class=\"graph-map-total\">{}</span></span></summary>",
            if default_open { " open" } else { "" },
            count(deps.len() as u64)
        ));
    } else {
        html.push_str(&format!(
            "<div class=\"graph-impact-group\"><h4>{title} <span class=\"graph-map-total\">{}</span></h4>",
            count(deps.len() as u64)
        ));
    }
    for dep in deps {
        let target = normalize_rel_path(strip_symbol_suffix(&m::text(dep, "target")));
        let resolved = dep["resolved"].as_bool().unwrap_or(!target.is_empty());
        let exists = dep["exists"].as_bool().unwrap_or(true);
        let kind = m::text(dep, "kind").replace('_', " ");
        let line = dep["line"].as_u64().unwrap_or(0);
        let evidence = m::text(dep, "evidence");
        let symbols: Vec<String> = m::array(&dep["symbols"])
            .iter()
            .filter_map(|s| s.as_str().map(str::to_owned))
            .collect();
        let member_refs = {
            let refs = m::array(&dep["member_refs"]);
            if refs.is_empty() {
                m::array(&dep["referenced_members"])
            } else {
                refs
            }
        };
        let target_members = {
            let exposed = m::array(&dep["target_members"]);
            if exposed.is_empty() {
                m::array(&dep["declared_members"])
            } else {
                exposed
            }
        };
        let warnings = m::array(&dep["visibility_warnings"]);
        let has_vis_risk = !warnings.is_empty()
            || member_refs.iter().any(member_has_severe_risk)
            || target_members.iter().any(member_has_severe_risk);
        let jump_payload = payload(&json!({"path": target, "surface": surface}));
        let link_payload = payload(&json!({"from": self_path, "to": target, "surface": surface}));
        let can_jump = !target.is_empty() && (resolved || exists);
        html.push_str(&format!(
            "<div class=\"graph-impact-row\" data-diff=\"{diff_key}\"><div class=\"graph-impact-row-main\">"
        ));
        if can_jump {
            html.push_str(&format!(
                "<button type=\"button\" class=\"graph-impact-row-btn\" data-action=\"graph-map-impact-focus\" data-path=\"{}\" data-payload=\"{jump_payload}\"><span class=\"graph-status-pill\" data-status=\"{diff_key}\">{}</span> <span>{}</span></button>",
                escape(&target),
                diff_badge_sign(diff_key),
                escape(&target),
            ));
        } else {
            html.push_str(&format!(
                "<span class=\"graph-impact-row-btn\"><span class=\"graph-status-pill\" data-status=\"{diff_key}\">{}</span> <span>{}</span></span>",
                diff_badge_sign(diff_key),
                escape(&target),
            ));
        }
        html.push_str("<div class=\"graph-impact-row-actions\">");
        if !exists {
            html.push_str(
                "<span class=\"graph-status-pill\" data-status=\"broken_risk\">missing</span>",
            );
        } else if !resolved {
            html.push_str(
                "<span class=\"graph-status-pill\" data-status=\"unchanged\">unresolved</span>",
            );
        } else if !target.is_empty() && target != self_path && looks_like_file_path(&target) {
            html.push_str(&format!(
                "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-link\" data-payload=\"{link_payload}\">Links</button>"
            ));
        }
        html.push_str("</div></div>");
        let mut meta = Vec::new();
        if !kind.is_empty() {
            meta.push(escape(&kind));
        }
        if line > 0 {
            meta.push(format!("L{line}"));
        }
        if !symbols.is_empty() {
            meta.push(escape(&symbols.join(", ")));
        }
        if !member_refs.is_empty() || !target_members.is_empty() {
            meta.push(format!(
                "{} used · {} exposed",
                member_refs.len(),
                target_members.len()
            ));
        }
        if has_vis_risk {
            meta.push(
                "<span class=\"graph-status-pill\" data-status=\"broken_risk\">vis risk</span>"
                    .to_owned(),
            );
        }
        if !meta.is_empty() {
            html.push_str(&format!(
                "<div class=\"graph-impact-symbols\">{}</div>",
                meta.join(" · ")
            ));
        }
        if !evidence.is_empty() {
            html.push_str(&format!(
                "<p class=\"graph-impact-evidence\">{}</p>",
                escape(&evidence)
            ));
        }
        html.push_str(&impact_members_details_html(
            &member_refs,
            &target_members,
            &warnings,
            MemberDetailsOpts {
                member_filter,
                used_title: "Called / Referenced",
                exposed_title: "Exposed by target",
                summary_title: "Properties &amp; methods",
                force_open: has_vis_risk,
            },
        ));
        html.push_str("</div>");
    }
    html.push_str(if collapsible { "</details>" } else { "</div>" });
    html
}
pub(super) fn caller_rank(impact: &str) -> u8 {
    match impact {
        "broken_risk" => 0,
        "impacted" => 1,
        "added_caller" => 2,
        _ => 3,
    }
}
pub(super) fn member_filter_bar_html(active: &str, surface: &str) -> String {
    let items = [
        ("all", "All"),
        ("used", "Used"),
        ("exposed", "Exposed"),
        ("risk", "Δ/Risk"),
    ];
    let mut html = String::from(
        "<div class=\"graph-member-filters\" role=\"group\" aria-label=\"Member filter\">",
    );
    for (key, label) in items {
        let pressed = if active == key { "true" } else { "false" };
        let filter_payload = payload(&json!({ "filter": key, "surface": surface }));
        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-member-filter\" data-action=\"graph-map-member-filter\" data-payload=\"{filter_payload}\" aria-pressed=\"{pressed}\">{label}</button>"
        ));
    }
    html.push_str("</div>");
    html
}
#[cfg(test)]
pub fn impact_side_html(impact: &V, view: &V, surface: &str) -> String {
    impact_side_html_with_filter(impact, view, surface, "all")
}
pub fn impact_side_html_with_filter(
    impact: &V,
    view: &V,
    surface: &str,
    member_filter: &str,
) -> String {
    let member_filter = match member_filter {
        "used" | "exposed" | "risk" => member_filter,
        _ => "all",
    };
    let path = normalize_rel_path(strip_symbol_suffix(&m::text(impact, "path")));
    if path.is_empty() {
        return String::new();
    }
    let prev = normalize_rel_path(&m::text(impact, "previous_path"));
    let status = m::text(impact, "status").to_ascii_lowercase();
    let status = if status.is_empty() {
        "unchanged".to_owned()
    } else {
        status
    };
    let exists = impact["exists"].as_bool().unwrap_or(status != "deleted");
    let in_graph = impact["in_graph"].as_bool().unwrap_or(true);
    let (_, status_label) = change_status_badge(&status);
    let mut html = format!(
        "<section class=\"graph-impact-card\"><div class=\"graph-impact-head\"><h3>{}</h3><span class=\"graph-status-pill\" data-status=\"{}\">{status_label}</span></div><p class=\"graph-map-path\">{}</p>",
        escape(base(&path)),
        escape(&status),
        escape(&path),
    );
    if !prev.is_empty() && prev != path {
        html.push_str(&format!(
            "<p class=\"knowledge-note\">Renamed from {}</p>",
            escape(&prev)
        ));
    }
    html.push_str("<div class=\"graph-impact-meta\">");
    if exists {
        html.push_str(
            "<span class=\"graph-status-pill\" data-status=\"created\">Exists in workspace</span>",
        );
    } else {
        html.push_str("<span class=\"graph-status-pill\" data-status=\"broken_risk\">Deleted — missing on disk</span>");
    }
    if m::yes(&impact["baseline"]["available"]) {
        let label = m::text(&impact["baseline"], "label");
        if !label.is_empty() {
            html.push_str(&format!("<span>Baseline: {}</span>", escape(&label)));
        }
    } else {
        let reason = m::text(&impact["baseline"], "reason");
        if !reason.is_empty() {
            html.push_str(&format!("<span>{}</span>", escape(&reason)));
        }
    }
    html.push_str("</div>");
    if !exists {
        html.push_str("<p class=\"graph-impact-missing\">File does not exist on disk in the current workspace.</p>");
    } else if !in_graph {
        html.push_str("<p class=\"graph-impact-missing\">Not yet in Graphify index — workspace dependency evidence shown below.</p>");
    }
    let sum = &impact["summary"];
    let uses_added_n = sum["uses_added"]
        .as_u64()
        .unwrap_or_else(|| m::array(&impact["uses"]["added"]).len() as u64);
    let uses_removed_n = sum["uses_removed"]
        .as_u64()
        .unwrap_or_else(|| m::array(&impact["uses"]["removed"]).len() as u64);
    let uses_unchanged_n = sum["uses_unchanged"]
        .as_u64()
        .unwrap_or_else(|| m::array(&impact["uses"]["unchanged"]).len() as u64);
    let callers_risk_n = sum["callers_at_risk"].as_u64().unwrap_or(0);
    let callers_impacted_n = sum["callers_impacted"].as_u64().unwrap_or(0);
    let added_callers_n = sum["added_callers"].as_u64().unwrap_or(0);
    let vis_warnings_n = sum["visibility_warnings"].as_u64().unwrap_or(0);
    let arch = &impact["architecture_impact"];
    let new_cycles = metric_count(&arch["new_cycles"]).max(sum["new_cycles"].as_u64().unwrap_or(0));
    let resolved_cycles =
        metric_count(&arch["resolved_cycles"]).max(sum["resolved_cycles"].as_u64().unwrap_or(0));
    let new_violations =
        metric_count(&arch["new_violations"]).max(sum["new_violations"].as_u64().unwrap_or(0));
    let resolved_violations = metric_count(&arch["resolved_violations"])
        .max(sum["resolved_violations"].as_u64().unwrap_or(0));
    html.push_str("<div class=\"graph-impact-summary\">");
    html.push_str(&format!(
        "<span class=\"graph-status-pill\" data-status=\"added\">+{} added</span><span class=\"graph-status-pill\" data-status=\"removed\">-{} removed</span><span class=\"graph-status-pill\" data-status=\"unchanged\">={} unchanged</span>",
        count(uses_added_n),
        count(uses_removed_n),
        count(uses_unchanged_n),
    ));
    if callers_risk_n > 0 {
        html.push_str(&format!(
            "<span class=\"graph-status-pill\" data-status=\"broken_risk\">!{} callers at risk</span>",
            count(callers_risk_n)
        ));
    }
    if vis_warnings_n > 0 {
        html.push_str(&format!(
            "<span class=\"graph-status-pill\" data-status=\"broken_risk\">!{} visibility warnings</span>",
            count(vis_warnings_n)
        ));
    }
    if callers_impacted_n > 0 {
        html.push_str(&format!(
            "<span class=\"graph-status-pill\" data-status=\"impacted\">~{} impacted callers</span>",
            count(callers_impacted_n)
        ));
    }
    if added_callers_n > 0 {
        html.push_str(&format!(
            "<span class=\"graph-status-pill\" data-status=\"added_caller\">+{} new callers</span>",
            count(added_callers_n)
        ));
    }
    if new_cycles > 0 || resolved_cycles > 0 {
        html.push_str(&format!(
            "<span class=\"graph-impact-arch\">↻ {} new / {} resolved cycles</span>",
            count(new_cycles),
            count(resolved_cycles)
        ));
    }
    html.push_str("</div>");
    html.push_str(&member_filter_bar_html(member_filter, surface));
    // Preserve Graphify open/follow/path actions when indexed and focused in the current view.
    let view_focus = &view["focus"];
    if view_focus.is_object()
        && normalize_rel_path(&m::text(view_focus, "path")) == path
        && !m::yes(&view_focus["folder"])
        && !at_base(view)
    {
        html.push_str("<div class=\"graph-impact-row-actions\">");
        if view_focus["symbols"].as_u64().unwrap_or(0) > 0 {
            html.push_str(&open_file(&view_focus["path"]));
        }
        if follows(view) || paths(view) || shares(view) || !m::yes(&view_focus["start"]) {
            html.push_str(&follow_links(&view_focus["path"]));
        }
        html.push_str(&path_to(view_focus));
        html.push_str("</div>");
    }
    let entities = m::array(&impact["entities"]);
    if !entities.is_empty() {
        html.push_str(&format!(
            "<div class=\"graph-impact-group\"><h4>Changed entities <span class=\"graph-map-total\">{}</span></h4>",
            count(entities.len() as u64)
        ));
        for ent in &entities {
            let echange = m::text(ent, "change").to_ascii_lowercase();
            let (ebadge, _) = change_status_badge(&echange);
            let ekind = normalize_entity_kind(&m::text(ent, "kind"));
            let line = ent["line"].as_u64().unwrap_or(0);
            let sig = m::yes(&ent["signature_changed"]);
            let vis_pill = visibility_pill_html(ent["visibility"].as_str(), false);
            let kind_pill = member_kind_pill_html(&ekind);
            let prev_vis = ent["previous_visibility"].as_str();
            let curr_vis = ent["visibility"].as_str();
            let vis_changed = m::yes(&ent["visibility_narrowed"])
                || m::yes(&ent["visibility_changed"])
                || (prev_vis.is_some() && curr_vis.is_some() && prev_vis != curr_vis);
            let vis_delta = if vis_changed {
                vis_delta_badge_html(prev_vis, curr_vis, m::yes(&ent["visibility_narrowed"]))
            } else {
                String::new()
            };
            html.push_str(&format!(
                "<div class=\"graph-impact-row\" data-diff=\"{}\"><div class=\"graph-impact-row-main\"><span class=\"graph-impact-row-btn\">{vis_pill}{kind_pill}<span class=\"graph-changes-entity-kind\">{}</span> <span class=\"graph-impact-entity-name\">{}{}</span></span><div class=\"graph-impact-row-actions\">{vis_delta}{}<span class=\"graph-status-pill\" data-status=\"{}\">{ebadge}</span></div></div></div>",
                escape(if echange.is_empty() { "unchanged" } else { &echange }),
                escape(entity_kind_badge(&ekind)),
                escape(&m::text(ent, "name")),
                if line > 0 { format!(":{line}") } else { String::new() },
                if sig {
                    "<span class=\"graph-status-pill\" data-status=\"modified\" title=\"signature changed\" aria-label=\"signature changed\">sig</span>"
                } else {
                    ""
                },
                escape(if echange.is_empty() { "unchanged" } else { &echange }),
            ));
        }
        html.push_str("</div>");
    }
    let added_deps = m::array(&impact["uses"]["added"]);
    let removed_deps = m::array(&impact["uses"]["removed"]);
    let unchanged_deps = m::array(&impact["uses"]["unchanged"]);
    html.push_str(&impact_dep_group_html(
        "Added dependencies (+)",
        "added",
        &added_deps,
        &path,
        surface,
        member_filter,
        true,
        false,
    ));
    html.push_str(&impact_dep_group_html(
        "Removed dependencies (-)",
        "removed",
        &removed_deps,
        &path,
        surface,
        member_filter,
        true,
        false,
    ));
    let mut callers = m::array(&impact["used_by"]);
    callers.sort_by(|a, b| {
        let ra = caller_rank(&m::text(a, "impact"));
        let rb = caller_rank(&m::text(b, "impact"));
        ra.cmp(&rb)
            .then_with(|| {
                b["links"]
                    .as_u64()
                    .unwrap_or(0)
                    .cmp(&a["links"].as_u64().unwrap_or(0))
            })
            .then_with(|| m::text(a, "path").cmp(&m::text(b, "path")))
    });
    if !callers.is_empty() {
        html.push_str(&format!(
            "<div class=\"graph-impact-group\"><h4>Used by (reverse dependents) <span class=\"graph-map-total\">{}</span></h4>",
            count(callers.len() as u64)
        ));
        for caller in &callers {
            let cpath = normalize_rel_path(strip_symbol_suffix(&m::text(caller, "path")));
            let cimpact = m::text(caller, "impact").to_ascii_lowercase();
            let cimpact = if cimpact.is_empty() {
                "unchanged".to_owned()
            } else {
                cimpact
            };
            let pill_label = match cimpact.as_str() {
                "broken_risk" => "Broken risk",
                "impacted" => "Impacted",
                "added_caller" => "Added caller",
                _ => "Unchanged",
            };
            let cexists = caller["exists"].as_bool().unwrap_or(true);
            let reason = m::text(caller, "reason");
            let links = caller["links"].as_u64().unwrap_or(0);
            let guessed = m::yes(&caller["guessed"]);
            let used_members = {
                let direct = m::array(&caller["used_members"]);
                if direct.is_empty() {
                    m::array(&caller["member_refs"])
                } else {
                    direct
                }
            };
            let caller_target_members = m::array(&caller["target_members"]);
            let vis_warnings = m::array(&caller["visibility_warnings"]);
            let has_vis_risk = !vis_warnings.is_empty()
                || used_members.iter().any(member_has_severe_risk)
                || caller_target_members.iter().any(member_has_severe_risk);
            let mut syms: Vec<String> = Vec::new();
            for sym in m::array(&caller["changed_symbols"])
                .into_iter()
                .chain(m::array(&caller["symbols"]))
                .filter_map(|item| {
                    item.as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                })
            {
                if !syms.iter().any(|existing| existing == &sym) {
                    syms.push(sym);
                }
            }
            let jump_payload = payload(&json!({"path": cpath, "surface": surface}));
            let link_payload = payload(&json!({"from": cpath, "to": path, "surface": surface}));
            html.push_str(&format!(
                "<div class=\"graph-impact-row\" data-risk=\"{}\" data-diff=\"{}\"><div class=\"graph-impact-row-main\"><button type=\"button\" class=\"graph-impact-row-btn\" data-action=\"graph-map-impact-focus\" data-path=\"{}\" data-payload=\"{jump_payload}\"><span class=\"graph-status-pill\" data-status=\"{}\">{pill_label}</span> <span>{}</span></button><div class=\"graph-impact-row-actions\">",
                cimpact == "broken_risk",
                if cimpact == "broken_risk" {
                    "risk"
                } else if cimpact == "added_caller" {
                    "added"
                } else {
                    "unchanged"
                },
                escape(&cpath),
                escape(&cimpact),
                escape(&cpath),
            ));
            if !cexists {
                html.push_str(
                    "<span class=\"graph-status-pill\" data-status=\"broken_risk\">missing</span>",
                );
            }
            if cexists && !cpath.is_empty() && cpath != path {
                html.push_str(&format!(
                    "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-link\" data-payload=\"{link_payload}\">Links</button>"
                ));
            }
            html.push_str("</div></div>");
            let mut meta = Vec::new();
            if links > 0 {
                meta.push(format!(
                    "{}{}",
                    plural(links, "link", "links"),
                    if guessed { " · guessed" } else { "" }
                ));
            }
            if !syms.is_empty() {
                meta.push(escape(&syms.join(", ")));
            }
            if !used_members.is_empty() {
                meta.push(format!("{} used", used_members.len()));
            }
            if has_vis_risk {
                meta.push(
                    "<span class=\"graph-status-pill\" data-status=\"broken_risk\">vis risk</span>"
                        .to_owned(),
                );
            }
            if !meta.is_empty() {
                html.push_str(&format!(
                    "<div class=\"graph-impact-symbols\">{}</div>",
                    meta.join(" · ")
                ));
            }
            if !reason.is_empty() {
                html.push_str(&format!(
                    "<p class=\"graph-impact-evidence\">{}</p>",
                    escape(&reason)
                ));
            }
            html.push_str(&impact_members_details_html(
                &used_members,
                &caller_target_members,
                &vis_warnings,
                MemberDetailsOpts {
                    member_filter,
                    used_title: "Referenced here",
                    exposed_title: "Exposed by target",
                    summary_title: "Properties &amp; methods",
                    force_open: cimpact == "broken_risk" || has_vis_risk,
                },
            ));
            html.push_str("</div>");
        }
        html.push_str("</div>");
    }
    html.push_str(&contract_consumers_html(impact, surface));
    html.push_str(&format!(
        "<div class=\"graph-impact-group\"><h4>Architecture &amp; cycles</h4><div class=\"graph-impact-arch\"><span>↻ {} new cycles · {} resolved</span><span>· {} new violations · {} resolved</span></div></div>",
        count(new_cycles),
        count(resolved_cycles),
        count(new_violations),
        count(resolved_violations),
    ));
    html.push_str(&impact_dep_group_html(
        "Unchanged dependencies (=)",
        "unchanged",
        &unchanged_deps,
        &path,
        surface,
        member_filter,
        added_deps.is_empty() && removed_deps.is_empty(),
        true,
    ));
    html.push_str("</section>");
    html
}
pub fn contract_consumers_html(impact: &V, surface: &str) -> String {
    let consumers = {
        let primary = m::array(&impact["contract_consumers"]);
        if primary.is_empty() {
            m::array(&impact["cross_service_consumers"])
        } else {
            primary
        }
    };
    if consumers.is_empty() {
        return String::new();
    }
    let mut html = format!(
        "<div class=\"graph-impact-group\" data-contract-consumers=\"true\"><h4>Cross-service contract consumers <span class=\"graph-map-total\">{}</span></h4>",
        count(consumers.len() as u64)
    );
    for item in &consumers {
        let service = {
            let s = m::text(item, "service");
            if !s.is_empty() {
                s
            } else {
                let cs = m::text(item, "consumer_service");
                if !cs.is_empty() {
                    cs
                } else {
                    m::text(item, "consumer")
                }
            }
        };
        let contract = {
            let c = m::text(item, "contract");
            if !c.is_empty() {
                c
            } else {
                m::text(item, "name")
            }
        };
        let kind = {
            let k = m::text(item, "kind");
            if !k.is_empty() {
                k
            } else {
                m::text(item, "protocol")
            }
        };
        let version = {
            let v = m::text(item, "version");
            if !v.is_empty() {
                v
            } else {
                let cv = m::text(item, "consumer_version");
                if !cv.is_empty() {
                    cv
                } else {
                    m::text(item, "provider_version")
                }
            }
        };
        let unopened_sources: Vec<String> = m::array(&item["unopened_sources"])
            .into_iter()
            .filter_map(|v| v.as_str().map(normalize_rel_path).filter(|s| !s.is_empty()))
            .collect();
        let mut consumer_paths = Vec::new();
        for v in m::array(&item["consumer_paths"])
            .into_iter()
            .chain(m::array(&item["consumer_sources"]))
            .chain(m::array(&item["unopened_sources"]))
            .chain(m::array(&item["files"]))
            .chain(m::array(&item["sources"]))
        {
            if let Some(norm) = v.as_str().map(normalize_rel_path).filter(|s| !s.is_empty()) {
                if !consumer_paths.contains(&norm) {
                    consumer_paths.push(norm);
                }
            }
        }
        let direct_path = {
            let p = normalize_rel_path(strip_symbol_suffix(&m::text(item, "path")));
            if !p.is_empty() {
                p
            } else {
                let cp = normalize_rel_path(strip_symbol_suffix(&m::text(item, "consumer_path")));
                if !cp.is_empty() {
                    cp
                } else {
                    unopened_sources
                        .first()
                        .or_else(|| consumer_paths.first())
                        .cloned()
                        .unwrap_or_default()
                }
            }
        };
        let raw_impact = m::text(item, "impact").to_ascii_lowercase();
        let unopened_flag = m::yes(&item["unopened"])
            || item["opened"].as_bool() == Some(false)
            || !unopened_sources.is_empty();
        let risk = m::yes(&item["at_risk"])
            || m::yes(&item["broken_risk"])
            || raw_impact == "broken_risk"
            || m::text(item, "risk") == "broken_risk";
        let (status_attr, pill_label) = if risk {
            ("broken_risk", "Broken risk")
        } else if unopened_flag || raw_impact == "unopened" {
            ("broken_risk", "Unopened consumer")
        } else {
            ("impacted", "Contract consumer")
        };
        let head_label = if !service.is_empty() && !direct_path.is_empty() {
            format!("{service} · {direct_path}")
        } else if !service.is_empty() {
            service.clone()
        } else if !direct_path.is_empty() {
            direct_path.clone()
        } else {
            contract.clone()
        };
        let jump_payload = payload(&json!({
            "service": service,
            "path": direct_path,
            "surface": surface,
        }));
        html.push_str(&format!(
            "<div class=\"graph-impact-row\" data-risk=\"{}\" data-diff=\"{}\"><div class=\"graph-impact-row-main\">",
            risk || unopened_flag,
            if risk || unopened_flag { "risk" } else { "impacted" },
        ));
        if !service.is_empty() {
            html.push_str(&format!(
                "<button type=\"button\" class=\"graph-impact-row-btn\" data-action=\"graph-map-switch-service-file\" data-service=\"{}\" data-path=\"{}\" data-payload=\"{jump_payload}\"><span class=\"graph-status-pill\" data-status=\"{status_attr}\">{pill_label}</span> <span>{}</span></button>",
                escape(&service),
                escape(&direct_path),
                escape(&head_label),
            ));
        } else {
            html.push_str(&format!(
                "<span class=\"graph-impact-row-btn\"><span class=\"graph-status-pill\" data-status=\"{status_attr}\">{pill_label}</span> <span>{}</span></span>",
                escape(&head_label),
            ));
        }
        html.push_str("</div>");
        let mut meta = Vec::new();
        if !contract.is_empty() {
            meta.push(format!("contract: {}", escape(&contract)));
        }
        if !kind.is_empty() {
            meta.push(escape(&kind));
        }
        if !version.is_empty() {
            let trimmed_ver = version.strip_prefix('v').unwrap_or(&version);
            meta.push(format!("v{}", escape(trimmed_ver)));
        }
        if unopened_flag {
            meta.push("not opened in run".to_owned());
        }
        if consumer_paths.len() > 1 {
            meta.push(escape(&consumer_paths.join(", ")));
        }
        if !meta.is_empty() {
            html.push_str(&format!(
                "<div class=\"graph-impact-symbols\">{}</div>",
                meta.join(" · ")
            ));
        }
        let reason = {
            let r = m::text(item, "reason");
            if !r.is_empty() {
                r
            } else {
                let rr = m::text(item, "risk_reason");
                if !rr.is_empty() {
                    rr
                } else {
                    m::text(item, "note")
                }
            }
        };
        if !reason.is_empty() {
            html.push_str(&format!(
                "<p class=\"graph-impact-evidence\">{}</p>",
                escape(&reason)
            ));
        }
        html.push_str("</div>");
    }
    html.push_str("</div>");
    html
}
pub fn build_scoped_changes_path_for_state(
    scope: &GraphTargetScope,
    state: &V,
    focus_session: &str,
) -> String {
    let raw_target = effective_target(state);
    let target = if raw_target.is_empty() {
        "work"
    } else {
        raw_target.as_str()
    };
    build_scoped_changes_path(scope, target, focus_session, m::yes(&state["notests"]))
}
pub(super) async fn fetch_surface_changes(surface: MapSurface, state: V) {
    let scope = surface_scope(surface);
    if scope.is_empty() {
        m::set(surface.changes_key, V::Null);
        update_surface_changes_badge(surface, None);
        return;
    }
    let scope_key = scope.scope_key();
    let raw_target = effective_target(&state);
    let effective_target = if raw_target.is_empty() {
        "work"
    } else {
        raw_target.as_str()
    };
    let sid = if surface.name == "session" {
        m::read(|s| s.sid.clone())
    } else {
        m::text(&state, "focus_session")
    };
    let url = build_scoped_changes_path_for_state(&scope, &state, &sid);
    match api::scoped(surface.changes_epoch, &url).await {
        Ok((epoch, doc)) => {
            if !m::current(surface.changes_epoch, epoch)
                || surface_scope(surface).scope_key() != scope_key
            {
                return;
            }
            m::set(
                surface.changes_key,
                json!({
                    "scope_key": scope_key,
                    "project": scope.project_id(),
                    "target": effective_target,
                    "data": doc
                }),
            );
            rerender_surface_preserving_camera(surface);
        }
        Err(error) if error.aborted => {}
        Err(error) => {
            if surface_scope(surface).scope_key() != scope_key {
                return;
            }
            m::set(
                surface.changes_key,
                json!({
                    "scope_key": scope_key,
                    "project": scope.project_id(),
                    "target": effective_target,
                    "error": error.message,
                    "data": {}
                }),
            );
            if m::text(&surface_state(surface), "side_tab") == "changes" {
                rerender_surface_side_only(surface);
            }
        }
    }
}
pub(super) async fn fetch_surface_impact(surface: MapSurface, state: V, path: String, key: String) {
    let scope = surface_scope(surface);
    if scope.is_empty() || path.is_empty() {
        return;
    }
    let scope_key = scope.scope_key();
    let raw_target = effective_target(&state);
    let effective_target = if raw_target.is_empty() {
        "work"
    } else {
        raw_target.as_str()
    };
    let sid = if surface.name == "session" {
        m::read(|s| s.sid.clone())
    } else {
        m::text(&state, "focus_session")
    };
    let certain = m::yes(&state["certain"]);
    let url = build_scoped_file_impact_path(&scope, &path, effective_target, &sid, certain);
    match api::scoped(surface.impact_epoch, &url).await {
        Ok((epoch, doc)) => {
            if !m::current(surface.impact_epoch, epoch)
                || surface_scope(surface).scope_key() != scope_key
            {
                return;
            }
            m::set(
                surface.impact_key,
                json!({"key": key, "loading": false, "data": doc}),
            );
            rerender_surface_preserving_camera(surface);
            if dom::get("graph-link").is_some_and(|d| d.has_attribute("open")) {
                draw_link();
            }
        }
        Err(error) if error.aborted => {}
        Err(_) => {
            let cur = m::data(surface.impact_key);
            if m::text(&cur, "key") == key {
                m::set(surface.impact_key, V::Null);
            }
        }
    }
}
pub(super) fn ensure_surface_impact(surface: MapSurface, state: &V, view: Option<&V>, force: bool) {
    let scope = surface_scope(surface);
    if scope.is_empty() {
        return;
    }
    let changes = surface_changes_doc(surface, state);
    let Some(path) = selected_impact_file(state, view, changes.as_ref()) else {
        return;
    };
    let wanted_key = impact_cache_key(&scope.scope_key(), &path, state);
    let cached = m::data(surface.impact_key);
    if !force && m::text(&cached, "key") == wanted_key {
        return;
    }
    m::set(
        surface.impact_key,
        json!({"key": wanted_key.clone(), "loading": true}),
    );
    let state_clone = state.clone();
    spawn_local(async move {
        fetch_surface_impact(surface, state_clone, path, wanted_key).await;
    });
}
pub(super) fn surface_unindexed_side_html(
    surface: MapSurface,
    state: &V,
    changes: Option<&V>,
) -> String {
    if surface_scope(surface).is_empty() {
        return String::new();
    }
    let side_tab = m::text(state, "side_tab");
    let changes_count = changes_count_of(changes);
    if side_tab == "changes" {
        let active = selected_impact_file(state, None, changes);
        return format!(
            "{}{}",
            side_top_html("changes", changes_count, surface.name, true),
            changes_side_html(
                changes.unwrap_or(&V::Null),
                &m::text(state, "changes_filter"),
                active.as_deref().unwrap_or(""),
                surface.name,
            ),
        );
    }
    if let Some(impact) = cached_impact_for_selection(surface, state, None) {
        return format!(
            "{}{}",
            side_top_html("details", changes_count, surface.name, true),
            impact_side_html_with_filter(
                &impact,
                &V::Null,
                surface.name,
                &m::text(state, "member_filter")
            ),
        );
    }
    String::new()
}
pub(super) fn build_surface_side_html(surface: MapSurface, view: &V, state: &V) -> String {
    let prefix = surface.id_prefix;
    let attach_action = format!("{prefix}-attach");
    let changes_tab = m::text(state, "side_tab") == "changes";
    let module_mode = m::yes(&state["modules"]);
    let scenario_mode = view["mode"] == "scenario";
    if scenario_mode && !changes_tab {
        m::take(surface.attachment_key);
        dom::attr(surface.canvas, "data-picking", "false");
        scenario_side(view)
    } else if module_mode && !changes_tab {
        m::take(surface.attachment_key);
        dom::attr(surface.canvas, "data-picking", "false");
        modules_side(view)
    } else {
        dom::attr(
            surface.canvas,
            "data-picking",
            if state["pick"].is_object() {
                "true"
            } else {
                "false"
            },
        );
        let action = if cycles(view) {
            let mut actions = base_cycle_button(view, at_base(view));
            if state["cycle_return"].is_object() {
                actions.push_str(CYCLE_BACK);
            }
            actions
        } else if patterned(view) && state["pattern_return"].is_object() {
            PATTERN_BACK.to_owned()
        } else {
            String::new()
        };
        let test_back = if paths(view) && state["test_path_return"].is_object() {
            "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-test-back\">Back to changed files</button>"
        } else {
            ""
        };
        set_surface_attrs(surface, view);
        map_side_html(
            surface,
            view,
            state,
            &action,
            &scenario_button(view),
            test_back,
            &attach_action,
        )
    }
}

pub(super) fn rerender_surface_side_only(surface: MapSurface) {
    let view = m::data(surface.view_key);
    let state = surface_state(surface);
    if surface.embedded {
        let changes = surface_changes_doc(surface, &state);
        sync_surface_toolbar(surface, &state, changes.as_ref());
        let was_empty = dom::get(surface.side).is_none_or(|el| el.inner_html().is_empty());
        let side_html = if view["nodes"].is_array() {
            build_surface_side_html(surface, &view, &state)
        } else {
            surface_unindexed_side_html(surface, &state, changes.as_ref())
        };
        html(surface.side, &side_html);
        let now_empty = side_html.is_empty();
        if was_empty != now_empty {
            let mut cam = load_surface_camera(surface);
            cam.needs_measure = true;
            save_surface_camera(surface, &cam);
            dom::later(surface.refit_timer_key, 60, move || {
                refit_surface_if_visible(surface);
            });
        }
    } else if open() && view.is_object() {
        let modal_changes = surface_changes_doc(MODAL_SURFACE, &state);
        let changes_tab = m::text(&state, "side_tab") == "changes";
        let module_mode = m::yes(&state["modules"]);
        let scenario_mode = view["mode"] == "scenario";
        if scenario_mode && !changes_tab {
            html(
                "graph-map-side",
                &format!(
                    "{}{}",
                    side_tabs_html("details", changes_count_of(modal_changes.as_ref()), "modal"),
                    scenario_side(&view)
                ),
            );
        } else if module_mode && !changes_tab {
            html(
                "graph-map-side",
                &format!(
                    "{}{}",
                    side_tabs_html("details", changes_count_of(modal_changes.as_ref()), "modal"),
                    modules_side(&view)
                ),
            );
        } else {
            let side_html = build_surface_side_html(MODAL_SURFACE, &view, &state);
            html("graph-map-side", &side_html);
        }
    }
}

pub(super) fn rerender_surface_preserving_camera(surface: MapSurface) {
    let view = m::data(surface.view_key);
    if surface.embedded {
        let state = surface_state(surface);
        let changes = surface_changes_doc(surface, &state);
        sync_surface_toolbar(surface, &state, changes.as_ref());
        if view["nodes"].is_array() {
            render_embedded_surface(surface, &view, false, false);
        } else {
            html(
                surface.side,
                &surface_unindexed_side_html(surface, &state, changes.as_ref()),
            );
        }
    } else if open() && view.is_object() {
        render(&view);
    }
}
pub fn build_scoped_map_path(scope: &GraphTargetScope, state: &V, embedded: bool) -> String {
    let mut query = vec![("scope", m::text(state, "scope"))];
    if embedded {
        let sid = if scope.is_system() {
            m::text(state, "focus_session")
        } else {
            m::read(|s| s.sid.clone())
        };
        if !sid.is_empty() {
            query.push(("focus_session", sid));
        }
    }
    let target = effective_target(state);
    if !target.is_empty() {
        query.push(("target", target.clone()));
    }
    for key in ["focus", "filter", "launch", "compare_launch"] {
        if !m::text(state, key).is_empty() {
            query.push((key, m::text(state, key)));
        }
    }
    if scope.is_system() {
        if let Some(rid) = effective_sysrun_id(scope, &target, state["run_id"].as_str()) {
            query.push(("run_id", rid));
        }
        let agent = m::text(state, "agent");
        if !agent.is_empty() {
            query.push(("agent", agent));
        }
    }
    for key in [
        "certain",
        "notests",
        "history",
        "modules",
        "no_incoming",
        "whole",
    ] {
        if m::yes(&state[key]) {
            query.push((key, "1".to_owned()));
        }
    }
    // The patterns found where the picture is open; the picture of one of
    // them is that pattern alone.
    if m::yes(&state["patterns"]) && !state["pattern"].is_string() {
        query.push(("patterns", "1".to_owned()));
    }
    let ends = unfolded(state, "path");
    let pathed = ends.len() == 2;
    for key in ["out", "into"] {
        let places = unfolded(state, key);
        if !pathed && !m::text(state, "follow").is_empty() && !places.is_empty() {
            query.push((key, json!(places).to_string()));
        }
    }
    let failing = m::array(&state["failing"])
        .iter()
        .filter_map(|told| told.as_str().map(str::to_owned))
        .take(FAILING)
        .collect::<Vec<_>>();
    if !target.is_empty() && !failing.is_empty() {
        query.push(("failing", json!(failing).to_string()));
    }
    if let Some(origin) = state["scenario"].as_str() {
        query.push(("scenario", origin.to_owned()));
        query.push((
            "skip",
            state["scenario_skip"].as_u64().unwrap_or(0).to_string(),
        ));
    }
    if let Some(offset) = state["offset"].as_u64().filter(|n| *n > 0) {
        query.push(("offset", offset.to_string()));
    }
    if let Some(limit) = state["limit"].as_u64().filter(|n| *n > 0) {
        query.push(("limit", limit.clamp(1, 500).to_string()));
    }
    if m::yes(&state["isolate_active"]) {
        let depth = state["isolate_depth"].as_u64().unwrap_or(2).clamp(1, 6);
        query.push(("depth", depth.to_string()));
    }
    if state["scenario"].is_string() {
        // already pushed above
    } else if let (Some(place), Some(revision)) = (
        state["cycle_base"]["place"].as_str(),
        state["cycle_base"]["revision"].as_str(),
    ) {
        query.push(("cycle", place.to_owned()));
        query.push(("base", revision.to_owned()));
    } else if let Some(cycle) = state["cycle"].as_str() {
        query.push(("cycle", cycle.to_owned()));
    } else if let Some(pattern) = state["pattern"].as_str() {
        query.push(("pattern", pattern.to_owned()));
    } else if pathed {
        let asked = if m::yes(&state["shared"]) {
            "shared"
        } else {
            "path"
        };
        query.push((asked, json!(ends).to_string()));
        // With a path, the ways longer than its shortest by as many links.
        let longer = state["longer"].as_u64().unwrap_or(0).min(LONGER);
        if asked == "path" && longer > 0 {
            query.push(("longer", longer.to_string()));
        }
        if asked == "shared" && !m::text(state, "side").is_empty() {
            query.push(("side", m::text(state, "side")));
        }
        if let Some(skip) = begun(state).last().filter(|_| asked == "shared") {
            query.push(("skip", skip.to_string()));
        }
    } else if !m::text(state, "follow").is_empty() {
        query.push(("follow", m::text(state, "follow")));
    } else if !unfolded(state, "spread").is_empty() {
        query.push(("spread", json!(unfolded(state, "spread")).to_string()));
    }
    format_scoped_endpoint(scope, "map", query)
}
#[cfg(test)]
pub(super) fn build_map_path(project: &str, state: &V, embedded: bool) -> String {
    build_scoped_map_path(&GraphTargetScope::single(project), state, embedded)
}
pub(super) fn combine_note(before: String, view: &V) -> String {
    format_graph_status_text(
        &[
            before,
            m::text(view, "architecture_error"),
            note(view),
            cycle_revision(view),
        ]
        .iter()
        .filter(|part| !part.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" "),
    )
}
pub(super) fn map_side_html(
    surface: MapSurface,
    view: &V,
    state: &V,
    action: &str,
    scenario_action: &str,
    test_back: &str,
    attach_action: &str,
) -> String {
    let attachment = map_attachment(view);
    let scope = surface_scope(surface);
    m::set(
        surface.attachment_key,
        json!({
            "project": if view["project_id"].is_string() { view["project_id"].clone() } else { json!(scope.project_id()) },
            "scope": scope.to_json(),
            "text": attachment
        }),
    );
    let attach = if attachment.is_some() {
        format!(
            "<button type=\"button\" class=\"button compact\" data-action=\"{attach_action}\">Attach map context</button>"
        )
    } else {
        String::new()
    };
    let focused_node_key = {
        let p = m::text(&view["focus"], "path");
        if !p.is_empty() {
            p
        } else {
            m::text(state, "focus")
        }
    };
    let unpin_btn = if !focused_node_key.is_empty()
        && graph_viewport::load_pins(&surface_pin_key(surface, state))
            .contains_key(&focused_node_key)
    {
        let unpin_action = if surface.embedded {
            format!("{}-unpin-node", surface.id_prefix)
        } else {
            "graph-map-unpin-node".to_owned()
        };
        format!(
            "<button type=\"button\" class=\"button compact graph-map-unpin-btn\" data-action=\"{unpin_action}\" data-payload=\"{}\">Unpin node</button>",
            payload(&json!(focused_node_key)),
        )
    } else {
        String::new()
    };
    let changes_doc = surface_changes_doc(surface, state);
    let changes_count = changes_count_of(changes_doc.as_ref());
    let side_tab = m::text(state, "side_tab");
    let changes_tab = side_tab == "changes";
    let analysis_tab = side_tab == "analysis";
    let active_focus = selected_impact_file(state, Some(view), changes_doc.as_ref());
    let impact_doc = cached_impact_for_selection(surface, state, Some(view));
    if surface.embedded
        && !changes_tab
        && !analysis_tab
        && !m::yes(&state["side_pinned"])
        && !view["focus"].is_object()
        && active_focus.is_none()
        && impact_doc.is_none()
        && !cycles(view)
        && !patterned(view)
        && view["mode"] != "scenario"
        && !state["pick"].is_object()
        && !view["chosen"].is_object()
    {
        return String::new();
    }
    let show_close = surface.embedded
        && (view["focus"].is_object()
            || active_focus.is_some()
            || changes_tab
            || analysis_tab
            || m::yes(&state["side_pinned"]));
    let header = side_top_html(
        if changes_tab {
            "changes"
        } else if analysis_tab {
            "analysis"
        } else {
            "details"
        },
        changes_count,
        surface.name,
        show_close,
    );
    if changes_tab {
        let changes_raw = m::data(surface.changes_key);
        let changes_err = m::text(&changes_raw, "error");
        let err_note = if !changes_err.is_empty() && changes_doc.is_none() {
            format!(
                "<p class=\"error-text\" role=\"alert\">Could not load changes: {}</p>",
                escape(&changes_err),
            )
        } else {
            String::new()
        };
        return format!(
            "{header}{err_note}{}",
            changes_side_html(
                changes_doc.as_ref().unwrap_or(&V::Null),
                &m::text(state, "changes_filter"),
                active_focus.as_deref().unwrap_or(""),
                surface.name,
            ),
        );
    }
    if analysis_tab {
        return format!(
            "{header}<div class=\"graph-side-panel\" data-tab=\"analysis\">{}{}{}{}{}{}{}{}{}{}</div>",
            choosing(state),
            run_comparison(view),
            blast_side(view),
            without_incoming_side(view),
            suspects_side(view, state, surface),
            test_paths(view),
            history_side(view),
            circles_side(view),
            architecture_side(view),
            found_patterns(view, &unfolded(state, "kinds")),
        );
    }
    let conf_badge = confidence_pill_html(view);
    let details_body = if let Some(impact) = impact_doc.as_ref() {
        format!(
            "{conf_badge}{}{}{test_back}{action}{scenario_action}{}{}{}{attach}{unpin_btn}{}{}{}{}{}{}{}{}",
            choosing(state),
            paging(view),
            run_comparison(view),
            shell_reads(view),
            impact_side_html_with_filter(
                impact,
                view,
                surface.name,
                &m::text(state, "member_filter")
            ),
            blast_side(view),
            without_incoming_side(view),
            suspects_side(view, state, surface),
            test_paths(view),
            history_side(view),
            circles_side(view),
            architecture_side(view),
            found_patterns(view, &unfolded(state, "kinds")),
        )
    } else if let Some(focused_path) = active_focus.as_deref() {
        if view["focus"].is_object() {
            format!(
                "{conf_badge}{}{}{test_back}{action}{scenario_action}{}{}{}{attach}{unpin_btn}{}<p class=\"knowledge-note\">Reading dependency impact…</p>{}{}{}{}{}{}{}",
                choosing(state),
                paging(view),
                run_comparison(view),
                shell_reads(view),
                side(view),
                blast_side(view),
                without_incoming_side(view),
                suspects_side(view, state, surface),
                test_paths(view),
                history_side(view),
                circles_side(view),
                architecture_side(view),
                found_patterns(view, &unfolded(state, "kinds")),
            )
        } else {
            format!(
                "{}{attach}{unpin_btn}<section class=\"graph-impact-card\"><div class=\"graph-impact-head\"><h3>{}</h3></div><p class=\"graph-map-path\">{}</p><p class=\"knowledge-note\">Reading file dependency impact…</p></section>",
                choosing(state),
                escape(base(focused_path)),
                escape(focused_path),
            )
        }
    } else if surface.embedded
        && !view["focus"].is_object()
        && !cycles(view)
        && !patterned(view)
        && view["mode"] != "scenario"
        && !state["pick"].is_object()
        && !view["chosen"].is_object()
    {
        format!(
            "{conf_badge}<p class=\"knowledge-note\">Select a file or folder on the map to inspect its dependencies and change impact.</p>"
        )
    } else {
        format!(
            "{conf_badge}{}{}{test_back}{action}{scenario_action}{}{}{}{attach}{unpin_btn}{}{}{}{}{}{}{}{}",
            choosing(state),
            paging(view),
            run_comparison(view),
            shell_reads(view),
            side(view),
            blast_side(view),
            without_incoming_side(view),
            suspects_side(view, state, surface),
            test_paths(view),
            history_side(view),
            circles_side(view),
            architecture_side(view),
            found_patterns(view, &unfolded(state, "kinds")),
        )
    };
    // What the project's memory says about the selected file fills in when
    // it arrives; the map does not wait for it.
    let memory = active_focus
        .as_deref()
        .map(|path| normalize_rel_path(strip_symbol_suffix(path)))
        .filter(|path| !path.is_empty() && !path.ends_with('/'))
        .map(|path| {
            let project = view["project_id"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| scope.project_id().to_string());
            memory_links_section(&project, &path)
        })
        .unwrap_or_default();
    format!(
        "{header}<div class=\"graph-side-panel\" data-tab=\"details\">{details_body}{memory}</div>"
    )
}
pub(super) const MEMORY_LINKS: &str = "graph-memory-links";
pub(super) const MEMORY_LINKS_LOOKING: &str = "<h4>Linked memory</h4><p class=\"knowledge-note\">Looking for Project Brain records and Memory Bank chunks that cite this file…</p>";
/// A file's links, or a request for them, are good for a minute: a record
/// saved since then shows on the next selection.
pub(super) fn memory_links_fresh(cached: &V) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        cached["at"]
            .as_f64()
            .is_none_or(|at| js_sys::Date::now() - at < 60_000.)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = cached;
        true
    }
}
/// The linked-memory section of a selected file: drawn from what an earlier
/// request returned, or a placeholder a request fills in.
pub(super) fn memory_links_section(project: &str, path: &str) -> String {
    let key = format!("{project}\u{1f}{path}");
    let cached = m::data(MEMORY_LINKS)[&key].clone();
    let fresh = cached.is_object() && memory_links_fresh(&cached);
    let body = if fresh && cached["pending"] != true {
        render_side_memory_bridge_html(project, &cached)
    } else if fresh {
        // The map redraws often; one request per file is in flight at a time.
        MEMORY_LINKS_LOOKING.to_owned()
    } else {
        #[cfg(target_arch = "wasm32")]
        {
            let mut cache = m::data(MEMORY_LINKS);
            if !cache.is_object() {
                cache = json!({});
            }
            cache[&key] = json!({"pending": true, "at": js_sys::Date::now()});
            m::set(MEMORY_LINKS, cache);
            let (project, path, key) = (project.to_owned(), path.to_owned(), key.clone());
            spawn_local(async move {
                let url = format!(
                    "/api/projects/{}/graph/memory-links?path={}",
                    dom::enc(&project),
                    dom::enc(&path)
                );
                let mut links = match crate::api::get(&url).await {
                    Ok(links) => links,
                    Err(error) => json!({"error": error.message}),
                };
                links["at"] = json!(js_sys::Date::now());
                let mut cache = m::data(MEMORY_LINKS);
                if !cache.is_object() {
                    cache = json!({});
                }
                cache[&key] = links.clone();
                m::set(MEMORY_LINKS, cache);
                for section in dom::all(".graph-memory-links") {
                    if section.get_attribute("data-path").as_deref() == Some(path.as_str()) {
                        section.set_inner_html(&render_side_memory_bridge_html(&project, &links));
                    }
                }
            });
        }
        MEMORY_LINKS_LOOKING.to_owned()
    };
    format!(
        "<section class=\"graph-memory-links\" data-path=\"{}\" aria-live=\"polite\" aria-label=\"Linked memory\">{body}</section>",
        escape(path)
    )
}
/// What the project's memory says about one file, with a way to open each
/// record or chunk and warnings to review. A failed request says so and
/// leaves the rest of the panel alone.
pub(crate) fn render_side_memory_bridge_html(project: &str, links: &V) -> String {
    let mut html = String::from("<h4>Linked memory</h4>");
    if let Some(error) = links["error"].as_str() {
        html.push_str(&format!(
            "<p class=\"knowledge-note\">Linked memory is unavailable: {}</p>",
            escape(error)
        ));
        return html;
    }
    for warning in m::array(&links["hygiene_warnings"]) {
        html.push_str(&format!(
            "<p class=\"graph-memory-warning\" data-kind=\"{}\" role=\"note\">Review: {}</p>",
            escape(warning["kind"].as_str().unwrap_or("")),
            escape(warning["text"].as_str().unwrap_or(""))
        ));
    }
    // Two atoms of the file that contradict each other: both stay as they
    // are, and the record or chunk behind either opens for a person to judge.
    for warning in m::array(&links["contradiction_warnings"]) {
        let opens = m::array(&warning["open"])
            .iter()
            .filter_map(|open| {
                let scope = open["scope"].as_str().filter(|s| matches!(*s, "brain" | "memory"))?;
                let target = open["target_id"].as_str().filter(|t| !t.is_empty())?;
                Some(format!(
                    " <button type=\"button\" class=\"button compact\" data-action=\"graph-map-open-memory\" data-payload=\"{}\">Open {}</button>",
                    payload(&json!({"scope": scope, "project_id": project, "target_id": target})),
                    escape(open["title"].as_str().unwrap_or(target))
                ))
            })
            .collect::<String>();
        html.push_str(&format!(
            "<p class=\"graph-memory-warning\" data-kind=\"{}\" role=\"note\">Review: {}{opens}</p>",
            escape(warning["kind"].as_str().unwrap_or("atom-contradiction")),
            escape(warning["text"].as_str().unwrap_or(""))
        ));
    }
    let open = |scope: &str, target: &str, label: String| {
        format!(
            "<li><button type=\"button\" class=\"graph-memory-link\" data-action=\"graph-map-open-memory\" data-payload=\"{}\">{label}</button></li>",
            payload(&json!({"scope": scope, "project_id": project, "target_id": target}))
        )
    };
    let records = m::array(&links["brain_records"]);
    let chunks = m::array(&links["memory_chunks"]);
    let atoms = m::array(&links["omnigraph_atoms"]);
    if records.is_empty() && chunks.is_empty() && atoms.is_empty() {
        html.push_str("<p class=\"knowledge-note\">No Project Brain record, Memory Bank chunk or shared context atom cites this file.</p>");
        return html;
    }
    if !records.is_empty() {
        html.push_str(
            "<p class=\"graph-memory-group\">Project Brain</p><ul class=\"graph-memory-list\">",
        );
        for record in &records {
            let target = record["external_id"]
                .as_str()
                .or_else(|| record["id"].as_str())
                .unwrap_or("");
            html.push_str(&open(
                "brain",
                target,
                format!(
                    "<span class=\"graph-memory-kind\">{}</span> {}",
                    escape(record["type"].as_str().unwrap_or("record")),
                    escape(record["title"].as_str().unwrap_or(target))
                ),
            ));
        }
        html.push_str("</ul>");
    }
    if !chunks.is_empty() {
        html.push_str(
            "<p class=\"graph-memory-group\">Memory Bank</p><ul class=\"graph-memory-list\">",
        );
        for chunk in &chunks {
            let id = chunk["id"].as_str().unwrap_or("");
            html.push_str(&open(
                "memory",
                id,
                format!(
                    "<span class=\"graph-memory-kind\">{}</span> {}{}",
                    escape(id),
                    escape(chunk["title"].as_str().unwrap_or("")),
                    if chunk["auto_promoted"] == true {
                        " <span class=\"graph-memory-badge\">auto-promoted</span>"
                    } else {
                        ""
                    }
                ),
            ));
        }
        html.push_str("</ul>");
    }
    if !atoms.is_empty() {
        html.push_str(
            "<p class=\"graph-memory-group\">Shared context</p><ul class=\"graph-memory-list\">",
        );
        for atom in &atoms {
            // The declarations of this file the atom names, as it named them.
            let symbols = m::array(&atom["symbols"])
                .iter()
                .filter_map(V::as_str)
                .take(6)
                .map(escape)
                .collect::<Vec<_>>();
            html.push_str(&format!(
                "<li class=\"graph-memory-atom\">{}{}</li>",
                escape(
                    atom["capsule_line"]
                        .as_str()
                        .or_else(|| atom["title"].as_str())
                        .unwrap_or("")
                        .trim_start_matches("- ")
                ),
                if symbols.is_empty() {
                    String::new()
                } else {
                    format!(
                        " <span class=\"graph-memory-badge\">names {}</span>",
                        symbols.join(", ")
                    )
                }
            ));
        }
        html.push_str("</ul>");
    }
    let linked_path = links["path"].as_str().unwrap_or("").trim();
    if !linked_path.is_empty() {
        let omni_payload = payload(&json!({"project_id": project, "path": linked_path}));
        html.push_str(&format!(
            "<div class=\"graph-memory-actions\"><button type=\"button\" class=\"button compact\" data-action=\"graph-map-open-omnigraph\" data-payload=\"{omni_payload}\">Open in Context Graph</button></div>"
        ));
    }
    html
}
pub(super) fn set_surface_attrs(surface: MapSurface, view: &V) {
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

pub(super) fn render(view: &V) {
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
        save_surface_camera(MODAL_SURFACE, &cam);
        apply_surface_camera(MODAL_SURFACE, &cam);
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
pub(super) async fn choose(target: &str) -> bool {
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
pub(super) fn reconcile_view_state(
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
pub(super) async fn load(quiet: bool) {
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
pub(super) async fn place() {
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
pub(super) fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}
pub(super) struct ClickOutcome {
    pub(super) handled: bool,
    pub(super) reload: bool,
    pub(super) rerender: bool,
    pub(super) rerender_side: bool,
    pub(super) force_impact: bool,
    pub(super) clear_filter: Option<String>,
    pub(super) clear_find: bool,
    pub(super) disable_find: Option<bool>,
    pub(super) save_notests: Option<bool>,
    pub(super) save_fit: Option<bool>,
    pub(super) open_link: Option<(String, String)>,
    pub(super) switch_service: Option<(String, String)>,
}
impl ClickOutcome {
    pub(super) fn unhandled() -> Self {
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
    pub(super) fn stay() -> Self {
        Self {
            handled: true,
            ..Self::unhandled()
        }
    }
    pub(super) fn reload() -> Self {
        Self {
            handled: true,
            reload: true,
            ..Self::unhandled()
        }
    }
}
pub(super) fn apply_click_action(
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
pub(super) fn attach_map_context_to_system_task(service_id: &str, context_text: &str) -> bool {
    if context_text.trim().is_empty() {
        return false;
    }
    let next = append_system_task_draft(&dom::value("system-task"), service_id, context_text);
    dom::set_value("system-task", &next);
    true
}
pub(super) fn switch_surface_to_service_file(
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
            m::next("graph-commit-graph");
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
            "graph-map-lens-dependencies"
            | "graph-map-lens-changes"
            | "graph-map-lens-architecture"
            | "graph-map-lens-tests"
            | "graph-map-lens-runs",
            _,
        ) => {
            let lens = id.strip_prefix("graph-map-lens-").unwrap_or("dependencies");
            match lens {
                "dependencies" => {
                    state["target"] = json!("");
                    state["compare_base"] = V::Null;
                    state["history"] = json!(false);
                    state["patterns"] = json!(false);
                    state["modules"] = json!(false);
                    state["lens_tests"] = json!(false);
                    state["side_tab"] = json!("details");
                    chosen = true;
                }
                "changes" => {
                    state["history"] = json!(false);
                    state["patterns"] = json!(false);
                    state["modules"] = json!(false);
                    state["lens_tests"] = json!(false);
                    state["side_tab"] = json!("changes");
                    if m::text(&state, "target").is_empty() {
                        state["target"] = json!("work");
                    }
                    chosen = true;
                }
                "architecture" => {
                    state["history"] = json!(false);
                    state["patterns"] = json!(true);
                    state["lens_tests"] = json!(false);
                    state["side_tab"] = json!("analysis");
                    dom::save(PATTERNS, Some("1"));
                    chosen = true;
                }
                "tests" => {
                    state["history"] = json!(false);
                    state["patterns"] = json!(false);
                    state["modules"] = json!(false);
                    state["lens_tests"] = json!(true);
                    state["side_tab"] = json!("analysis");
                    if m::text(&state, "target").is_empty() {
                        state["target"] = json!("work");
                    }
                    chosen = true;
                }
                "runs" => {
                    state["history"] = json!(true);
                    state["patterns"] = json!(false);
                    state["modules"] = json!(false);
                    state["lens_tests"] = json!(false);
                    state["side_tab"] = json!("analysis");
                    chosen = true;
                }
                _ => {}
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

#[derive(Clone, Debug)]
pub(super) struct Camera {
    pub(super) scale: f64,
    pub(super) pan_x: f64,
    pub(super) pan_y: f64,
    pub(super) fit_mode: bool,
    pub(super) follow: bool,
    pub(super) paused: bool,
    pub(super) expanded: bool,
    pub(super) plane_w: f64,
    pub(super) plane_h: f64,
    pub(super) needs_measure: bool,
    pub(super) bbox: Option<(f64, f64, f64, f64)>,
}
impl Camera {
    pub(super) fn viewport_cam(&self) -> graph_viewport::Camera {
        graph_viewport::Camera::new(self.pan_x, self.pan_y, self.scale)
    }
    pub(super) fn apply_viewport_cam(&mut self, cam: graph_viewport::Camera) {
        self.scale = cam.scale;
        self.pan_x = cam.tx;
        self.pan_y = cam.ty;
    }
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            scale: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            fit_mode: true,
            follow: true,
            paused: false,
            expanded: false,
            plane_w: 0.0,
            plane_h: 0.0,
            needs_measure: false,
            bbox: None,
        }
    }
}
pub(super) fn load_surface_camera(surface: MapSurface) -> Camera {
    if surface.camera_key.is_empty() {
        return Camera::default();
    }
    let raw = m::data(surface.camera_key);
    if !raw.is_object() {
        return Camera::default();
    }
    let bbox = raw["bbox"].as_array().and_then(|arr| {
        if arr.len() == 4 {
            Some((
                arr[0].as_f64()?,
                arr[1].as_f64()?,
                arr[2].as_f64()?,
                arr[3].as_f64()?,
            ))
        } else {
            None
        }
    });
    Camera {
        scale: raw["scale"]
            .as_f64()
            .unwrap_or(1.0)
            .clamp(CAMERA_MIN, CAMERA_MAX),
        pan_x: raw["pan_x"].as_f64().unwrap_or(0.0),
        pan_y: raw["pan_y"].as_f64().unwrap_or(0.0),
        fit_mode: raw["fit_mode"].as_bool().unwrap_or(true),
        follow: raw["follow"].as_bool().unwrap_or(true),
        paused: raw["paused"].as_bool().unwrap_or(false),
        expanded: raw["expanded"].as_bool().unwrap_or(false),
        plane_w: raw["plane_w"].as_f64().unwrap_or(0.0),
        plane_h: raw["plane_h"].as_f64().unwrap_or(0.0),
        needs_measure: raw["needs_measure"].as_bool().unwrap_or(false),
        bbox,
    }
}
pub(super) fn save_surface_camera(surface: MapSurface, cam: &Camera) {
    if surface.camera_key.is_empty() {
        return;
    }
    let bbox_val = match cam.bbox {
        Some((x0, y0, x1, y1)) => json!([x0, y0, x1, y1]),
        None => V::Null,
    };
    m::set(
        surface.camera_key,
        json!({
            "scale": cam.scale,
            "pan_x": cam.pan_x,
            "pan_y": cam.pan_y,
            "fit_mode": cam.fit_mode,
            "follow": cam.follow,
            "paused": cam.paused,
            "expanded": cam.expanded,
            "plane_w": cam.plane_w,
            "plane_h": cam.plane_h,
            "needs_measure": cam.needs_measure,
            "bbox": bbox_val,
        }),
    );
}
pub(super) fn current_surface_live_focus(surface: MapSurface) -> Option<LiveFocus> {
    if surface.focus_key.is_empty() {
        return None;
    }
    parse_live_focus(&m::data(surface.focus_key))
}
pub(super) fn viewport_size(canvas_id: &str) -> (f64, f64, bool) {
    graph_viewport::viewport_size(canvas_id, 23.5, (640.0, 420.0))
}
pub(super) fn apply_surface_plane_transform(surface: MapSurface, cam: &Camera) {
    dom::text(
        &format!("{}-zoom-level", surface.id_prefix),
        &format!("{}%", (cam.scale * 100.0).round() as i64),
    );
    let Some(canvas) = dom::get(surface.canvas) else {
        return;
    };
    let Ok(Some(plane)) = canvas.query_selector(".graph-map-plane") else {
        sync_radar(surface.canvas, None);
        return;
    };
    sync_surface_radar(surface, cam);
    if cam.plane_w <= 0.0 || cam.plane_h <= 0.0 {
        return;
    }
    let _ = graph_viewport::apply_transform_3d(
        &canvas,
        &plane,
        cam.viewport_cam(),
        (cam.plane_w, cam.plane_h),
        GRAPH_MAP_LOD,
    );
    let scale_text = format!("{:.4}", cam.scale);
    let _ = canvas.set_attribute("data-scale", &scale_text);
    let _ = plane.set_attribute("data-scale", &scale_text);
    graph_viewport::canvas2d::paint_scene(
        surface.canvas,
        cam.viewport_cam(),
        current_dense_hover(surface.canvas),
        true,
    );
    update_surface_active_camera(surface, cam);
    if plane.has_attribute("data-scale-tier") {
        let (rw, rh, measured) = viewport_size(surface.canvas);
        let bounds = measured
            .then(|| {
                visible_viewport_bounds(
                    (cam.pan_x, cam.pan_y),
                    cam.scale,
                    (rw, rh),
                    BOX_WIDTH * 1.5,
                )
            })
            .flatten();
        sync_viewport_culling(surface.canvas, bounds);
    }
}
pub(super) fn apply_surface_camera(surface: MapSurface, cam: &Camera) {
    apply_surface_plane_transform(surface, cam);
    if !surface.embedded {
        dom::set_checked("graph-map-fit", cam.fit_mode);
        return;
    }
    let prefix = surface.id_prefix;
    let ws_id = format!("{prefix}-workspace");
    dom::attr(
        &format!("{prefix}-fit"),
        "aria-pressed",
        if cam.fit_mode { "true" } else { "false" },
    );
    dom::attr(
        &format!("{prefix}-follow"),
        "aria-pressed",
        if cam.follow { "true" } else { "false" },
    );
    dom::attr(
        &format!("{prefix}-pause-motion"),
        "aria-pressed",
        if cam.paused { "true" } else { "false" },
    );
    dom::attr(
        &format!("{prefix}-expand"),
        "aria-expanded",
        if cam.expanded { "true" } else { "false" },
    );
    dom::attr(
        &ws_id,
        "data-motion-paused",
        if cam.paused { "true" } else { "false" },
    );
    let sync_sidebar = if surface.name == "session" {
        !m::read(|s| matches!(s.view.as_str(), "systems" | "system-changes"))
    } else {
        m::read(|s| {
            (surface.name == "system-member" && s.view == "systems")
                || (surface.name == "system-change" && s.view == "system-changes")
        })
    };
    if sync_sidebar {
        dom::attr(
            "sidebar-graph-pill",
            "data-motion-paused",
            if cam.paused { "true" } else { "false" },
        );
    }
    dom::attr(
        &ws_id,
        "data-expanded",
        if cam.expanded { "true" } else { "false" },
    );
    dom::attr(
        &ws_id,
        "data-following",
        if cam.follow { "true" } else { "false" },
    );
}
pub(super) fn fit_surface_camera(surface: MapSurface, cam: &mut Camera) {
    let (vw, vh, measured) = viewport_size(surface.canvas);
    cam.needs_measure = !measured;
    cam.fit_mode = true;
    if cam.plane_w > 0.0 && cam.plane_h > 0.0 {
        cam.apply_viewport_cam(graph_viewport::fit_proportional(
            (cam.plane_w, cam.plane_h),
            (vw, vh),
            0.94,
            CAMERA_MIN,
            1.0,
            12.0,
        ));
    }
    save_surface_camera(surface, cam);
    apply_surface_camera(surface, cam);
}
pub(super) fn zoom_surface_camera_to_bbox(
    surface: MapSurface,
    cam: &mut Camera,
    bbox: (f64, f64, f64, f64),
) {
    let (vw, vh, measured) = viewport_size(surface.canvas);
    cam.needs_measure = !measured;
    cam.apply_viewport_cam(graph_viewport::fit_bbox(
        bbox,
        (vw, vh),
        (112.0, 88.0),
        (BOX_WIDTH + 96.0, BOX_HEIGHT + 80.0),
        0.90,
        CAMERA_MIN,
        FOCUS_CAMERA_MAX.clamp(CAMERA_MIN, CAMERA_MAX),
    ));
    cam.fit_mode = false;
    save_surface_camera(surface, cam);
    apply_surface_camera(surface, cam);
}
pub(super) fn zoom_surface_camera_at(surface: MapSurface, factor: f64, anchor: Option<(f64, f64)>) {
    let mut cam = load_surface_camera(surface);
    let (vw, vh, measured) = viewport_size(surface.canvas);
    cam.needs_measure = !measured;
    let anchor = anchor.unwrap_or((vw / 2.0, vh / 2.0));
    cam.apply_viewport_cam(graph_viewport::zoom_at(
        cam.viewport_cam(),
        anchor,
        cam.scale * factor,
        CAMERA_MIN,
        CAMERA_MAX,
    ));
    cam.fit_mode = false;
    if !surface.embedded {
        let mut st = state();
        st["zoom"] = json!(cam.scale);
        st["fit"] = json!(false);
        m::set("graph-map", st);
    }
    save_surface_camera(surface, &cam);
    apply_surface_camera(surface, &cam);
}
pub(super) fn reset_surface_camera_100(surface: MapSurface) {
    let mut cam = load_surface_camera(surface);
    let (vw, vh, measured) = viewport_size(surface.canvas);
    cam.needs_measure = !measured;
    cam.apply_viewport_cam(graph_viewport::reset_100(
        (cam.plane_w, cam.plane_h),
        (vw, vh),
        12.0,
    ));
    cam.fit_mode = false;
    if !surface.embedded {
        let mut st = state();
        st["zoom"] = V::Null;
        st["fit"] = json!(false);
        m::set("graph-map", st);
    }
    save_surface_camera(surface, &cam);
    apply_surface_camera(surface, &cam);
}
pub(super) fn refit_surface_if_visible(surface: MapSurface) {
    let (_, _, measured) = viewport_size(surface.canvas);
    if !measured {
        return;
    }
    let mut cam = load_surface_camera(surface);
    if cam.plane_w <= 0.0 || cam.plane_h <= 0.0 {
        return;
    }
    if (cam.follow || !cam.fit_mode) && (cam.needs_measure || cam.fit_mode) {
        if let Some(bbox) = cam.bbox {
            zoom_surface_camera_to_bbox(surface, &mut cam, bbox);
            return;
        }
    }
    if cam.fit_mode || cam.needs_measure {
        fit_surface_camera(surface, &mut cam);
    }
}
pub(super) fn surface_drag_data(surface: MapSurface) -> V {
    if surface.drag_key.is_empty() {
        V::Null
    } else {
        m::data(surface.drag_key)
    }
}
pub(super) fn clear_surface_drag_suppress(surface: MapSurface) {
    if surface.drag_key.is_empty() {
        return;
    }
    let mut drag = surface_drag_data(surface);
    if drag.is_object() {
        drag["suppress_click"] = json!(false);
        m::set(surface.drag_key, drag);
    }
}
/// Builds clickable chips markup for `#{id_prefix}-focus-entities`.
pub fn focus_strip_chips_html_for_surface(focus: &LiveFocus, id_prefix: &str) -> String {
    let jump_action = format!("{id_prefix}-focus-jump");
    let mut html = String::new();
    for entity in focus.created_entities.iter().take(8) {
        let (change_attr, badge, change_verb) = entity_change_badge(entity.change.as_deref());
        let vis_pill = visibility_pill_html(entity.visibility.as_deref(), true);
        let vis_word = entity
            .visibility
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| format!("{s} "))
            .unwrap_or_default();
        let entity_payload = payload(&json!({
            "id": entity.id,
            "kind": entity.kind,
            "name": entity.name,
            "parent": entity.parent,
            "path": entity.path,
            "folder": entity.folder,
            "line": entity.line,
            "change": entity.change,
            "visibility": entity.visibility,
            "modifiers": entity.modifiers,
        }));
        let title = if entity.path.is_empty() {
            format!("{change_verb} {vis_word}{} {}", entity.kind, entity.name)
        } else {
            format!(
                "{change_verb} {vis_word}{} {} · {}",
                entity.kind, entity.name, entity.path
            )
        };
        html.push_str(&format!(
            "<button type=\"button\" class=\"session-graph-focus-chip\" data-action=\"{}\" data-entity-kind=\"{}\" data-entity-change=\"{}\" data-payload=\"{entity_payload}\" title=\"{}\"><span class=\"graph-map-entity-dot\" aria-hidden=\"true\"></span><span class=\"graph-map-entity-badge\">{badge}</span>{vis_pill}<span>{}: {}</span></button>",
            escape(&jump_action),
            escape(&entity.kind),
            escape(change_attr),
            escape(&title),
            escape(&entity.kind),
            escape(&entity.name),
        ));
    }
    if html.is_empty() {
        let mut seen = BTreeSet::new();
        for target in focus.target_paths.iter().chain(&focus.target_folders) {
            if seen.len() >= 5 || !seen.insert(target.clone()) {
                continue;
            }
            let short = target.rsplit('/').next().unwrap_or(target);
            let chip_payload = payload(&json!({ "path": target }));
            html.push_str(&format!(
                "<button type=\"button\" class=\"session-graph-focus-chip\" data-action=\"{}\" data-entity-kind=\"target\" data-payload=\"{chip_payload}\" title=\"{}\"><span>{}</span></button>",
                escape(&jump_action),
                escape(target),
                escape(short),
            ));
        }
    }
    html
}
/// Builds clickable chips markup for `#session-graph-focus-entities`.
#[allow(dead_code)]
pub fn focus_strip_chips_html(focus: &LiveFocus) -> String {
    focus_strip_chips_html_for_surface(focus, SESSION_SURFACE.id_prefix)
}
pub(super) fn render_surface_focus_strip(surface: MapSurface, focus: Option<&LiveFocus>) {
    let prefix = surface.id_prefix;
    let strip_id = format!("{prefix}-focus");
    let label_id = format!("{prefix}-focus-label");
    let entities_id = format!("{prefix}-focus-entities");
    let Some(focus) = focus else {
        dom::hidden(&strip_id, true);
        dom::text(&label_id, "");
        html(&entities_id, "");
        return;
    };
    let label = if !focus.summary.is_empty() {
        focus.summary.clone()
    } else if focus.phase == "predicting" {
        "Predicting target area…".to_owned()
    } else if !focus.created_entities.is_empty() {
        "Live created entities".to_owned()
    } else {
        "Focusing on live target area".to_owned()
    };
    dom::text(&label_id, &label);
    html(
        &entities_id,
        &focus_strip_chips_html_for_surface(focus, prefix),
    );
    dom::hidden(&strip_id, false);
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SurfaceViewSignature {
    pub(super) nodes_len: usize,
    pub(super) frames_len: usize,
    pub(super) edges_len: usize,
    pub(super) first_node_path: String,
    pub(super) last_node_path: String,
    pub(super) mode: String,
    pub(super) pattern_declared: bool,
    pub(super) layered: bool,
}

#[derive(Clone, Debug)]
pub(super) struct CachedSurfacePlan {
    pub(super) signature: SurfaceViewSignature,
    pub(super) nodes: Vec<Placed>,
    pub(super) frames: Vec<Framed>,
    pub(super) width: f64,
    pub(super) height: f64,
    pub(super) structure_hash: Option<u64>,
    pub(super) pairs: Vec<(usize, usize)>,
    pub(super) emitted_frames: Vec<usize>,
    pub(super) node_keys: Vec<String>,
    pub(super) node_labels: Vec<String>,
    pub(super) is_dense: bool,
    pub(super) dense_node_htmls: Vec<String>,
    pub(super) dense_extra_specs: Vec<Option<DenseNodeExtraSpec>>,
    pub(super) latest_patch: Option<CanvasSelectionPatch>,
    pub(super) latest_roving_focus: Option<usize>,
    pub(super) latest_path_nodes: BTreeSet<usize>,
}

thread_local! {
    pub(super) static SURFACE_PLAN_CACHE: RefCell<HashMap<&'static str, CachedSurfacePlan>> =
        RefCell::new(HashMap::new());
}

pub(super) fn pairs_for_view(view: &V, overlay: &CanvasOverlayContext<'_>) -> Vec<(usize, usize)> {
    let mut nodes = m::array(&view["nodes"]);
    let mut edges = m::array(&view["edges"]);
    let frames = m::array(&view["frames"]);
    inject_synthetic_deleted_nodes_and_edges(
        view,
        overlay.changes,
        &mut nodes,
        &mut edges,
        &frames,
    );
    let node_index = build_path_index(&nodes);
    let plain = plain_map(view);
    let mut left_out = if overlay.hide_transitive && plain {
        said_twice(view)
    } else {
        vec![false; edges.len()]
    };
    left_out.resize(edges.len(), false);
    edges
        .iter()
        .zip(&left_out)
        .filter(|(_, said)| !**said)
        .filter_map(|(edge, _)| {
            Some((
                lookup_path_index(&nodes, &node_index, &edge["from"])?,
                lookup_path_index(&nodes, &node_index, &edge["to"])?,
            ))
        })
        .collect()
}

pub(super) fn surface_view_signature(view: &V) -> SurfaceViewSignature {
    let nodes = m::array(&view["nodes"]);
    let frames = m::array(&view["frames"]);
    let edges = m::array(&view["edges"]);
    SurfaceViewSignature {
        nodes_len: nodes.len(),
        frames_len: frames.len(),
        edges_len: edges.len(),
        first_node_path: nodes
            .first()
            .map(|n| m::text(n, "path"))
            .unwrap_or_default(),
        last_node_path: nodes.last().map(|n| m::text(n, "path")).unwrap_or_default(),
        mode: m::text(view, "mode"),
        pattern_declared: m::yes(&view["pattern"]["declared"]),
        layered: layered(view),
    }
}

impl CachedSurfacePlan {
    pub(super) fn from_rendered(
        view: &V,
        rendered: &RenderedCanvas,
        overlay: Option<&CanvasOverlayContext<'_>>,
    ) -> Self {
        let (structure_hash, pairs) = match overlay {
            Some(ctx) => (
                Some(canvas_structure_digest(view, ctx)),
                pairs_for_view(view, ctx),
            ),
            None => (None, Vec::new()),
        };
        let emitted_frames = rendered
            .plan_frames
            .iter()
            .enumerate()
            .filter_map(|(idx, f)| (f.width > 0.0).then_some(idx))
            .collect();
        Self {
            signature: surface_view_signature(view),
            nodes: rendered.plan_nodes.clone(),
            frames: rendered.plan_frames.clone(),
            width: rendered.width,
            height: rendered.height,
            structure_hash,
            pairs,
            emitted_frames,
            node_keys: rendered.node_keys.clone(),
            node_labels: rendered.node_labels.clone(),
            is_dense: rendered.dense_scene.is_some(),
            dense_node_htmls: rendered.dense_node_htmls.clone(),
            dense_extra_specs: rendered.dense_extra_specs.clone(),
            latest_patch: None,
            latest_roving_focus: overlay.and_then(|ctx| ctx.roving_focus),
            latest_path_nodes: overlay
                .and_then(|ctx| ctx.path_nodes)
                .cloned()
                .unwrap_or_default(),
        }
    }
}

pub(super) fn invalidate_surface_plan_cache(canvas_id: &str) {
    SURFACE_PLAN_CACHE.with(|cache| {
        cache.borrow_mut().remove(canvas_id);
    });
}

pub(super) fn store_surface_plan_cache(
    canvas_id: &'static str,
    view: &V,
    rendered: &RenderedCanvas,
    overlay: Option<&CanvasOverlayContext<'_>>,
) {
    let entry = CachedSurfacePlan::from_rendered(view, rendered, overlay);
    SURFACE_PLAN_CACHE.with(|cache| {
        cache.borrow_mut().insert(canvas_id, entry);
    });
}

pub(super) struct PatchedCanvasMeta {
    pub(super) width: f64,
    pub(super) height: f64,
    pub(super) focus: FocusOverlay,
    pub(super) selected_bbox: Option<(f64, f64, f64, f64)>,
}

pub(super) fn toggle_bool_attr(el: &web_sys::Element, name: &str, active: bool) {
    let _ = if active {
        el.set_attribute(name, "true")
    } else {
        el.remove_attribute(name)
    };
}

pub(super) fn sync_node_blast_dom(el: &web_sys::Element, blast: Option<(u64, f64)>) {
    if let Some((hop, left)) = blast {
        let _ = el.set_attribute("data-blast", &hop.to_string());
        if let Ok(html_el) = el.clone().dyn_into::<web_sys::HtmlElement>() {
            let _ = html_el
                .style()
                .set_property("--blast", &format!("{left:.2}"));
        }
        if el
            .query_selector(".graph-map-node-blast")
            .ok()
            .flatten()
            .is_none()
        {
            if let Ok(span) = dom::document().create_element("span") {
                let _ = span.set_attribute("class", "graph-map-node-blast");
                let _ = span.set_attribute("aria-hidden", "true");
                if let Ok(Some(badges)) = el.query_selector(".graph-map-node-badges") {
                    let _ = el.insert_before(&span, Some(&badges));
                } else {
                    let _ = el.append_child(&span);
                }
            }
        }
    } else {
        let _ = el.remove_attribute("data-blast");
        if let Ok(html_el) = el.clone().dyn_into::<web_sys::HtmlElement>() {
            let _ = html_el.style().remove_property("--blast");
        }
        if let Ok(Some(span)) = el.query_selector(".graph-map-node-blast") {
            span.remove();
        }
    }
}

pub(super) fn patch_edge_state_dom(
    el: &web_sys::Element,
    edge_state: &str,
    shared_path: bool,
    with_classes: bool,
) {
    if edge_state.is_empty() {
        let _ = el.remove_attribute("data-edge-state");
    } else {
        let _ = el.set_attribute("data-edge-state", edge_state);
    }
    if with_classes {
        let list = el.class_list();
        if edge_state == "on" {
            let _ = list.add_1("graph-map-edge-on");
            let _ = list.remove_1("graph-map-edge-off");
        } else if edge_state == "off" {
            let _ = list.add_1("graph-map-edge-off");
            let _ = list.remove_1("graph-map-edge-on");
        } else {
            let _ = list.remove_1("graph-map-edge-on");
            let _ = list.remove_1("graph-map-edge-off");
        }
    }
    if el.class_list().contains("graph-map-edge") {
        toggle_bool_attr(el, "data-shared-path", shared_path);
    }
}

pub(super) fn apply_node_patch_to_el(
    node_el: &web_sys::Element,
    idx: usize,
    np: &NodeSelectionState,
    roving_focus: Option<usize>,
    path_nodes: Option<&BTreeSet<usize>>,
) {
    let shared_path = np.shared_path || path_nodes.is_some_and(|nodes| nodes.contains(&idx));
    let select_lit = np.lit || shared_path;
    let _ = node_el.set_attribute("data-selected", if np.selected { "true" } else { "false" });
    let _ = node_el.set_attribute("data-dim", if np.dim { "true" } else { "false" });
    toggle_bool_attr(node_el, "data-select-lit", select_lit);
    let hover_active = node_el
        .closest(".graph-map-plane[data-hover='true']")
        .ok()
        .flatten()
        .is_some();
    if !hover_active {
        toggle_bool_attr(node_el, "data-lit", select_lit);
    } else if select_lit {
        let _ = node_el.set_attribute("data-lit", "true");
    }
    toggle_bool_attr(node_el, "data-shared-path", shared_path);
    toggle_bool_attr(node_el, "data-focus-target", np.focus_target);
    if let Some(roving) = roving_focus {
        let _ = node_el.set_attribute("tabindex", if idx == roving { "0" } else { "-1" });
    }
    if let Some(pressed) = np.pressed {
        let _ = node_el.set_attribute("aria-pressed", if pressed { "true" } else { "false" });
    }
    sync_node_blast_dom(node_el, np.blast);
}

fn is_important_cached_node(cached: &CachedSurfacePlan, idx: usize) -> bool {
    if cached.latest_roving_focus == Some(idx) || cached.latest_path_nodes.contains(&idx) {
        return true;
    }
    cached
        .latest_patch
        .as_ref()
        .and_then(|patch| patch.nodes.get(idx))
        .is_some_and(|np| np.selected || np.lit || np.focus_target || np.shared_path || np.failed)
}

fn is_important_node_el(cached: &CachedSurfacePlan, idx: usize, el: &web_sys::Element) -> bool {
    is_important_cached_node(cached, idx)
        || el.get_attribute("data-selected").as_deref() == Some("true")
        || el.get_attribute("data-lit").as_deref() == Some("true")
        || el.get_attribute("data-focus-target").as_deref() == Some("true")
        || el.get_attribute("data-pinned").as_deref() == Some("true")
        || el.get_attribute("data-shared-path").as_deref() == Some("true")
        || el.get_attribute("tabindex").as_deref() == Some("0")
        || el
            .query_selector(".graph-map-node-suspect")
            .ok()
            .flatten()
            .is_some()
}

pub(super) fn mount_dense_node_if_needed(canvas_id: &str, idx: usize) -> Option<web_sys::Element> {
    let canvas = dom::get(canvas_id)?;
    if let Ok(Some(existing)) = canvas.query_selector(&format!(".graph-map-node[data-n='{idx}']")) {
        return Some(existing);
    }
    let plane = canvas.query_selector(".graph-map-plane").ok().flatten()?;
    let (html, extra_spec, patch, roving, path_nodes) = SURFACE_PLAN_CACHE.with(|cache| {
        let guard = cache.borrow();
        let entry = guard.get(canvas_id)?;
        if !entry.is_dense {
            return None;
        }
        let html = entry.dense_node_htmls.get(idx)?.clone();
        if html.is_empty() {
            return None;
        }
        let spec = entry.dense_extra_specs.get(idx).and_then(|s| s.clone());
        Some((
            html,
            spec,
            entry.latest_patch.clone(),
            entry.latest_roving_focus,
            entry.latest_path_nodes.clone(),
        ))
    })?;
    let mut created = graph_viewport::append_html_elements(&plane, &html);
    let el = created.pop()?;
    if let Some(np) = patch.as_ref().and_then(|p| p.nodes.get(idx)) {
        apply_node_patch_to_el(&el, idx, np, roving, Some(&path_nodes));
    } else if let Some(r) = roving {
        let _ = el.set_attribute("tabindex", if idx == r { "0" } else { "-1" });
    }
    if let Some(spec) = extra_spec.as_ref() {
        if let Some(surface) = pan_surface_for_canvas(canvas_id) {
            let view = m::data(surface.view_key);
            let dim_override = patch
                .as_ref()
                .and_then(|p| p.nodes.get(idx))
                .map(|n| n.portal_dim);
            let extra_html = render_dense_node_extras_for_mounted(&view, spec, dim_override);
            if !extra_html.is_empty() {
                let _ = graph_viewport::append_html_elements(&plane, &extra_html);
            }
        }
    }
    invalidate_hover_cache(canvas_id);
    Some(el)
}

/// Attempts to apply selection, highlight, cone, live-focus target, blast, and radar state
/// changes to the existing DOM without rebuilding canvas `innerHTML`.
pub(super) fn try_patch_canvas_dom_in_place(
    canvas_id: &str,
    view: &V,
    overlay: &CanvasOverlayContext<'_>,
) -> Option<PatchedCanvasMeta> {
    let expected_hash = canvas_structure_digest(view, overlay);
    let cached = SURFACE_PLAN_CACHE.with(|cache| {
        cache
            .borrow()
            .get(canvas_id)
            .filter(|entry| entry.structure_hash == Some(expected_hash))
            .cloned()
    })?;

    let canvas = dom::get(canvas_id)?;
    let plane = canvas.query_selector(".graph-map-plane").ok().flatten()?;
    let node_els = dom::all(&format!("#{canvas_id} .graph-map-node[data-n]"));
    let patch =
        compute_canvas_selection_patch(view, &cached.nodes, &cached.frames, &cached.pairs, overlay);
    if !cached.is_dense && node_els.len() != patch.nodes.len() {
        return None;
    }
    if cached.is_dense
        && (cached.dense_node_htmls.len() != patch.nodes.len()
            || node_els.iter().any(|el| {
                el.get_attribute("data-n")
                    .and_then(|s| s.parse::<usize>().ok())
                    .is_none_or(|idx| idx >= patch.nodes.len())
            }))
    {
        return None;
    }

    let mut mounted_indices = HashSet::with_capacity(node_els.len());
    for node_el in &node_els {
        let Some(idx) = node_el
            .get_attribute("data-n")
            .and_then(|s| s.parse::<usize>().ok())
        else {
            continue;
        };
        mounted_indices.insert(idx);
        let Some(np) = patch.nodes.get(idx) else {
            continue;
        };
        apply_node_patch_to_el(node_el, idx, np, overlay.roving_focus, overlay.path_nodes);
    }

    if cached.is_dense {
        let mut batch_html = String::new();
        let mut batch_indices = Vec::new();
        for (idx, np) in patch.nodes.iter().enumerate() {
            if mounted_indices.contains(&idx) {
                continue;
            }
            let important = np.selected
                || np.lit
                || np.focus_target
                || np.shared_path
                || np.failed
                || overlay.roving_focus == Some(idx)
                || overlay.path_nodes.is_some_and(|nodes| nodes.contains(&idx));
            if !important {
                continue;
            }
            if let Some(html) = cached.dense_node_htmls.get(idx).filter(|s| !s.is_empty()) {
                batch_html.push_str(html);
                batch_indices.push(idx);
            }
        }
        if !batch_html.is_empty() {
            let new_els = graph_viewport::append_html_elements(&plane, &batch_html);
            let mut extra_html = String::new();
            for (idx, el) in batch_indices.iter().copied().zip(new_els) {
                if let Some(np) = patch.nodes.get(idx) {
                    apply_node_patch_to_el(&el, idx, np, overlay.roving_focus, overlay.path_nodes);
                }
                if let Some(Some(spec)) = cached.dense_extra_specs.get(idx) {
                    let dim_over = patch.nodes.get(idx).map(|n| n.portal_dim);
                    extra_html
                        .push_str(&render_dense_node_extras_for_mounted(view, spec, dim_over));
                }
            }
            if !extra_html.is_empty() {
                let _ = graph_viewport::append_html_elements(&plane, &extra_html);
            }
            invalidate_hover_cache(canvas_id);
        }
    }

    for el in dom::all(&format!(
        "#{canvas_id} .graph-map-portal[data-of], #{canvas_id} .graph-map-spread[data-of]"
    )) {
        if let Some(idx) = el
            .get_attribute("data-of")
            .and_then(|s| s.parse::<usize>().ok())
        {
            if let Some(np) = patch.nodes.get(idx) {
                let _ = el.set_attribute("data-dim", if np.portal_dim { "true" } else { "false" });
            }
        }
    }

    let frame_els = dom::all(&format!("#{canvas_id} svg rect.graph-map-frame"));
    for (dom_idx, frame_el) in frame_els.iter().enumerate() {
        if let Some(&frame_idx) = cached.emitted_frames.get(dom_idx) {
            let is_target = patch.frame_targets.get(frame_idx).copied().unwrap_or(false);
            toggle_bool_attr(frame_el, "data-focus-target", is_target);
        }
    }

    if let Ok(Some(svg)) = canvas.query_selector(".graph-map-plane > svg") {
        let existing_zone = svg
            .query_selector("rect.graph-map-target-zone")
            .ok()
            .flatten();
        if let Some((x0, y0, x1, y1)) = patch.focus.bbox {
            let pad = 14.0;
            let zx = (x0 - pad).max(2.0);
            let zy = (y0 - pad).max(2.0);
            let zw = (x1 + pad - zx).max(BOX_WIDTH);
            let zh = (y1 + pad - zy).max(BOX_HEIGHT);
            let zone = existing_zone.or_else(|| {
                let created = dom::document()
                    .create_element_ns(Some("http://www.w3.org/2000/svg"), "rect")
                    .ok()?;
                let _ = created.set_attribute("class", "graph-map-target-zone");
                let _ = created.set_attribute("rx", "12");
                if let Ok(Some(before)) =
                    svg.query_selector(".graph-map-edge, .graph-map-violation-halo")
                {
                    let _ = svg.insert_before(&created, Some(&before));
                } else {
                    let _ = svg.append_child(&created);
                }
                Some(created)
            });
            if let Some(zone) = zone {
                let _ = zone.set_attribute("x", &format!("{zx:.1}"));
                let _ = zone.set_attribute("y", &format!("{zy:.1}"));
                let _ = zone.set_attribute("width", &format!("{zw:.1}"));
                let _ = zone.set_attribute("height", &format!("{zh:.1}"));
            }
        } else if let Some(zone) = existing_zone {
            zone.remove();
        }
    }

    for el in dom::all(&format!(
        "#{canvas_id} .graph-map-edge, #{canvas_id} .graph-map-edge-core, #{canvas_id} .graph-map-violation-halo, #{canvas_id} .graph-map-edge-mark, #{canvas_id} .graph-map-edge-chevron, #{canvas_id} .graph-map-edge-tie"
    )) {
        if let Some((from, to)) = el.get_attribute("data-ends").as_deref().and_then(ends_of) {
            if let Some(edge_patch) = patch.edges_by_pair.get(&(from, to)) {
                let list = el.class_list();
                let with_classes = list.contains("graph-map-edge")
                    || list.contains("graph-map-edge-core")
                    || list.contains("graph-map-edge-mark")
                    || list.contains("graph-map-edge-chevron");
                patch_edge_state_dom(
                    &el,
                    edge_patch.edge_state,
                    edge_patch.shared_path,
                    with_classes,
                );
            }
        }
    }

    for (idx, box_el) in dom::all(&format!("#{canvas_id}-radar .graph-radar-box"))
        .into_iter()
        .enumerate()
    {
        if let Some(np) = patch.nodes.get(idx) {
            if np.radar_state.is_empty() {
                let _ = box_el.remove_attribute("data-state");
            } else {
                let _ = box_el.set_attribute("data-state", np.radar_state);
            }
        }
    }

    let path_nodes_set = overlay.path_nodes.cloned().unwrap_or_default();
    SURFACE_PLAN_CACHE.with(|cache| {
        if let Some(entry) = cache.borrow_mut().get_mut(canvas_id) {
            entry.latest_patch = Some(patch.clone());
            if let Some(roving) = overlay.roving_focus {
                entry.latest_roving_focus = Some(roving);
            }
            entry.latest_path_nodes = path_nodes_set;
        }
    });

    if cached.is_dense {
        graph_viewport::canvas2d::update_scene(canvas_id, |scene| {
            refresh_dense_scene_for_patch(scene, &cached.pairs, &cached.node_keys, &patch, overlay);
        });
        if let Some(surface) = pan_surface_for_canvas(canvas_id) {
            let cam = load_surface_camera(surface);
            graph_viewport::canvas2d::paint_scene(
                canvas_id,
                cam.viewport_cam(),
                current_dense_hover(canvas_id),
                true,
            );
        }
    }

    Some(PatchedCanvasMeta {
        width: cached.width,
        height: cached.height,
        focus: patch.focus,
        selected_bbox: patch.selected_bbox,
    })
}

/// Marks offscreen nodes on large/dense canvases with `data-offscreen="true"` and dynamically
/// reconciles mounted DOM nodes against the cached dense layout when panning or zooming.
pub(super) fn sync_viewport_culling(canvas_id: &str, bounds: Option<ViewportBounds>) {
    let nodes = dom::all(&format!("#{canvas_id} .graph-map-node[data-n]"));
    let Some(bounds) = bounds else {
        for el in nodes {
            let _ = el.remove_attribute("data-offscreen");
        }
        return;
    };
    let cached = SURFACE_PLAN_CACHE.with(|cache| cache.borrow().get(canvas_id).cloned());
    let Some(cached) = cached else {
        return;
    };
    let mut mounted_els: HashMap<usize, web_sys::Element> = HashMap::with_capacity(nodes.len());
    for el in nodes {
        if let Some(idx) = el
            .get_attribute("data-n")
            .and_then(|s| s.parse::<usize>().ok())
        {
            mounted_els.insert(idx, el);
        }
    }

    if cached.is_dense && cached.dense_node_htmls.len() == cached.nodes.len() {
        if let Some(plane) =
            dom::get(canvas_id).and_then(|c| c.query_selector(".graph-map-plane").ok().flatten())
        {
            let rects: Vec<(f64, f64, f64, f64)> = cached
                .nodes
                .iter()
                .map(|p| (p.x, p.y, BOX_WIDTH, BOX_HEIGHT))
                .collect();
            let mounted: Vec<(usize, bool)> = mounted_els
                .iter()
                .map(|(&idx, el)| (idx, is_important_node_el(&cached, idx, el)))
                .collect();
            let mut force_mount = Vec::new();
            for (idx, html) in cached.dense_node_htmls.iter().enumerate() {
                if !html.is_empty() && is_important_cached_node(&cached, idx) {
                    force_mount.push(idx);
                }
            }
            let visible = (bounds.min_x, bounds.min_y, bounds.max_x, bounds.max_y);
            let mount = (
                bounds.min_x - 120.0,
                bounds.min_y - 120.0,
                bounds.max_x + 120.0,
                bounds.max_y + 120.0,
            );
            let unmount = (
                bounds.min_x - 280.0,
                bounds.min_y - 280.0,
                bounds.max_x + 280.0,
                bounds.max_y + 280.0,
            );
            let plan = graph_viewport::plan_viewport_reconcile(
                &rects,
                &mounted,
                visible,
                mount,
                unmount,
                &force_mount,
            );
            let dom_mutated = !plan.to_unmount.is_empty() || !plan.to_mount.is_empty();
            for idx in plan.to_unmount {
                if !cached
                    .dense_node_htmls
                    .get(idx)
                    .is_none_or(|s| s.is_empty())
                {
                    if let Some(el) = mounted_els.remove(&idx) {
                        el.remove();
                    }
                    for extra_el in dom::all(&format!("#{canvas_id} [data-of='{idx}']")) {
                        extra_el.remove();
                    }
                }
            }
            let mut batch_html = String::new();
            let mut batch_indices = Vec::new();
            for idx in plan.to_mount {
                if mounted_els.contains_key(&idx) {
                    continue;
                }
                if let Some(html) = cached.dense_node_htmls.get(idx).filter(|s| !s.is_empty()) {
                    batch_html.push_str(html);
                    batch_indices.push(idx);
                }
            }
            if !batch_html.is_empty() {
                let inserted = graph_viewport::append_html_elements(&plane, &batch_html);
                let mut extra_html = String::new();
                let view_opt = pan_surface_for_canvas(canvas_id).map(|s| m::data(s.view_key));
                for (idx, el) in batch_indices.into_iter().zip(inserted) {
                    if let Some(np) = cached.latest_patch.as_ref().and_then(|p| p.nodes.get(idx)) {
                        apply_node_patch_to_el(
                            &el,
                            idx,
                            np,
                            cached.latest_roving_focus,
                            Some(&cached.latest_path_nodes),
                        );
                    } else if let Some(roving) = cached.latest_roving_focus {
                        let _ =
                            el.set_attribute("tabindex", if idx == roving { "0" } else { "-1" });
                    }
                    if let (Some(view), Some(Some(spec))) =
                        (view_opt.as_ref(), cached.dense_extra_specs.get(idx))
                    {
                        if let Ok(None) = plane.query_selector(&format!("[data-of='{idx}']")) {
                            let dim_over = cached
                                .latest_patch
                                .as_ref()
                                .and_then(|p| p.nodes.get(idx))
                                .map(|n| n.portal_dim);
                            extra_html.push_str(&render_dense_node_extras_for_mounted(
                                view, spec, dim_over,
                            ));
                        }
                    }
                    mounted_els.insert(idx, el);
                }
                if !extra_html.is_empty() {
                    let _ = graph_viewport::append_html_elements(&plane, &extra_html);
                }
            }
            if dom_mutated {
                invalidate_hover_cache(canvas_id);
            }
        }
    }

    for (idx, el) in mounted_els {
        let is_important = is_important_node_el(&cached, idx, &el);
        let offscreen = if is_important {
            false
        } else if let Some(placed) = cached.nodes.get(idx) {
            !rect_intersects_viewport(placed.x, placed.y, BOX_WIDTH, BOX_HEIGHT, bounds)
        } else {
            false
        };
        toggle_bool_attr(&el, "data-offscreen", offscreen);
    }
}

pub(super) fn resolve_focus_bbox_with_cached_plan(
    view: &V,
    cached: Option<&CachedSurfacePlan>,
    probe: &LiveFocus,
) -> Option<(f64, f64, f64, f64)> {
    let expected = surface_view_signature(view);
    if let Some(entry) = cached.filter(|entry| entry.signature == expected) {
        let plan = Plan {
            nodes: entry.nodes.clone(),
            wires: Vec::new(),
            frames: entry.frames.clone(),
            width: entry.width,
            height: entry.height,
            apart: None,
        };
        return resolve_focus(view, &plan, probe).bbox;
    }
    let nodes = m::array(&view["nodes"]);
    let edges = m::array(&view["edges"]);
    let frames = m::array(&view["frames"]);
    let node_index = build_path_index(&nodes);
    let frame_index = build_path_index(&frames);
    let weighted: Vec<(usize, usize, u64)> = edges
        .iter()
        .filter_map(|e| {
            Some((
                lookup_path_index(&nodes, &node_index, &e["from"])?,
                lookup_path_index(&nodes, &node_index, &e["to"])?,
                e["links"].as_u64().unwrap_or(1),
            ))
        })
        .collect();
    let framed = Frames {
        of: nodes
            .iter()
            .map(|n| lookup_path_index(&frames, &frame_index, &n["holder"]))
            .collect(),
        holder: frames
            .iter()
            .map(|f| lookup_path_index(&frames, &frame_index, &f["holder"]))
            .collect(),
    };
    let plan = if layered(view) && !m::yes(&view["pattern"]["declared"]) {
        plan_row(nodes.len(), &weighted)
    } else {
        plan_beside(nodes.len(), &weighted, &framed, None)
    };
    resolve_focus(view, &plan, probe).bbox
}

pub(super) fn blank_embedded_surface(surface: MapSurface, note_text: &str) {
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
pub(super) fn render_embedded_surface(
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
pub(super) fn redraw_surface_overlay(surface: MapSurface, auto_focus_camera: bool) {
    let view = m::data(surface.view_key);
    if view["nodes"].is_array() {
        render_embedded_surface(surface, &view, auto_focus_camera, false);
    } else {
        render_surface_focus_strip(surface, current_surface_live_focus(surface).as_ref());
    }
}
pub(super) fn embedded_error_is_unbuilt(graph_status: &V, error_message: &str) -> bool {
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
pub(super) fn embedded_unbuilt_note(graph_status: &V) -> &'static str {
    embedded_unbuilt_note_for_scope(graph_status, false)
}
pub(super) fn embedded_unbuilt_note_for_scope(graph_status: &V, is_system: bool) -> &'static str {
    match m::text(graph_status, "state").as_str() {
        "building" => "Building code graph and dependency map…",
        "off" if is_system => "Code graph is turned off for this service.",
        "off" => "Code graph is turned off for this project.",
        _ if is_system => "Code graph not built yet for this service — click Build graph to index.",
        _ => "No published code graph yet for this project.",
    }
}
pub(super) async fn load_embedded_surface(surface: MapSurface, quiet: bool) {
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
    let path = build_scoped_map_path(&scope, &state, true);
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
pub(super) fn focus_surface_from_prompt(surface: MapSurface, prompt: &str) {
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
pub(super) fn apply_surface_live_focus(surface: MapSurface, focus: &V) {
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
pub(super) fn jump_to_surface_entity_or_target(surface: MapSurface, payload: &V) -> bool {
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
pub(super) fn focus_file_on_surface(surface: MapSurface, raw_path: &str) -> bool {
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
/// Focuses a workspace-relative file path on the embedded Sessions dependency map.
pub fn focus_file_in_session(path: &str) {
    focus_target_in_session(path, None);
}

/// Focuses a workspace-relative file path and optional symbol FQN on the Sessions dependency map.
pub(crate) fn focus_target_in_session(path: &str, symbol_fqn: Option<&str>) {
    let clean_path = normalize_rel_path(strip_symbol_suffix(path));
    if let Some(fqn) = symbol_fqn.map(str::trim).filter(|s| !s.is_empty()) {
        let mut st = surface_state(SESSION_SURFACE);
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
        save_surface_state(SESSION_SURFACE, &st);
        let _ = focus_file_on_surface(
            SESSION_SURFACE,
            if clean_path.is_empty() {
                fqn
            } else {
                &clean_path
            },
        );
        return;
    }
    let _ = focus_file_on_surface(SESSION_SURFACE, path);
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

pub(super) fn move_roving_node_focus(
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
    if let Some(placed) = cached.nodes.get(next_idx) {
        let (vw, vh, measured) = viewport_size(surface.canvas);
        if measured {
            let mut cam = load_surface_camera(surface);
            if let Some(panned) = graph_viewport::pan_to_keep_rect_in_view(
                cam.viewport_cam(),
                (placed.x, placed.y, BOX_WIDTH, BOX_HEIGHT),
                (vw, vh),
                32.0,
            ) {
                cam.apply_viewport_cam(panned);
                cam.fit_mode = false;
                save_surface_camera(surface, &cam);
                apply_surface_camera(surface, &cam);
            }
        }
    }
    let _ = mount_dense_node_if_needed(surface.canvas, next_idx);
    if let Some(canvas) = dom::get(surface.canvas) {
        if let Ok(nodes) = canvas.query_selector_all(".graph-map-node[data-n]") {
            for i in 0..nodes.length() {
                if let Some(el) = nodes
                    .item(i)
                    .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
                {
                    let is_next = el
                        .get_attribute("data-n")
                        .and_then(|s| s.parse::<usize>().ok())
                        == Some(next_idx);
                    let _ = el.set_attribute("tabindex", if is_next { "0" } else { "-1" });
                    if is_next {
                        let _ = el.remove_attribute("data-offscreen");
                        if let Ok(he) = el.dyn_into::<web_sys::HtmlElement>() {
                            let _ = he.focus();
                        }
                    }
                }
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

pub(super) fn transfer_surface_state_to_modal(emb: &V, mut modal: V) -> V {
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

pub(super) async fn open_full_from_surface(surface: MapSurface) {
    set_surface_scope(MODAL_SURFACE, surface_scope(surface));
    let emb = surface_state(surface);
    let target = effective_target(&emb);
    let modal = transfer_surface_state_to_modal(&emb, anew(&target));
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
    dom::text("graph-map-zoom-level", "100%");
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
/// Selects a file of a member service in the picture embedded in the System
/// view: the folders that hold it are opened there, the picture is brought
/// to it, and what the change does to it is listed beside. Nothing is opened
/// over the view; **Full map** of the embedded picture still does that.
pub(crate) fn focus_system_file(kind: SystemEmbeddedKind, path: &str) -> bool {
    focus_file_on_surface(kind.surface(), path)
}
pub(super) fn handle_surface_input(
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
pub(super) fn handle_surface_click(
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
pub(super) fn resolve_system_click_surface(
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
pub(super) fn target_pan_surface(target_el: Option<&web_sys::Element>) -> Option<MapSurface> {
    let canvas = target_el?
        .closest("#graph-map-canvas, #session-graph-canvas, #system-member-graph-canvas, #system-change-graph-canvas")
        .ok()
        .flatten()?;
    pan_surface_for_canvas(&canvas.id())
}
pub(super) fn active_drag_surface() -> Option<(MapSurface, graph_viewport::DragPan)> {
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
                let Some(canvas) = dom::get(surface.canvas) else {
                    return false;
                };
                let rect = canvas.get_bounding_client_rect();
                let cam = load_surface_camera(surface);
                let wx = (pe.client_x() as f64 - rect.left() - cam.pan_x) / cam.scale.max(0.05);
                let wy = (pe.client_y() as f64 - rect.top() - cam.pan_y) / cam.scale.max(0.05);
                let Some((0, idx)) =
                    graph_viewport::canvas2d::hit_test_scene(surface.canvas, wx, wy, 4.0)
                else {
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
            m::set(
                "surface-node-drag",
                json!({
                    "active": true,
                    "moved": false,
                    "surface": surface.name,
                    "pointer_id": pe.pointer_id(),
                    "start_x": pe.client_x() as f64,
                    "start_y": pe.client_y() as f64,
                    "orig_x": orig_x,
                    "orig_y": orig_y,
                    "cur_x": orig_x,
                    "cur_y": orig_y,
                    "node_idx": idx,
                    "node_key": node_key,
                }),
            );
            true
        }
        "pointermove" | "pointerup" | "pointercancel" => {
            let mut drag = m::data("surface-node-drag");
            if !m::yes(&drag["active"]) {
                return false;
            }
            let surface = surface_by_name(&m::text(&drag, "surface"));
            let pointer_id = drag["pointer_id"].as_i64().unwrap_or(0) as i32;
            let Ok(pe) = event.clone().dyn_into::<web_sys::PointerEvent>() else {
                if kind != "pointermove" {
                    m::set("surface-node-drag", json!({}));
                }
                return false;
            };
            if pe.pointer_id() != pointer_id {
                return false;
            }
            let moved = m::yes(&drag["moved"]);
            if kind == "pointermove" {
                let start_x = drag["start_x"].as_f64().unwrap_or(0.0);
                let start_y = drag["start_y"].as_f64().unwrap_or(0.0);
                let dx = pe.client_x() as f64 - start_x;
                let dy = pe.client_y() as f64 - start_y;
                if !moved && dx.hypot(dy) < 3.0 {
                    return true;
                }
                let cam = load_surface_camera(surface);
                let orig_x = drag["orig_x"].as_f64().unwrap_or(0.0);
                let orig_y = drag["orig_y"].as_f64().unwrap_or(0.0);
                let nx = (orig_x + dx / cam.scale.max(0.05)).max(8.0);
                let ny = (orig_y + dy / cam.scale.max(0.05)).max(8.0);
                let idx = drag["node_idx"].as_u64().unwrap_or(0) as usize;
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
                    // Live-update wires attached to the dragged node in both Dense Canvas2D and SVG modes.
                    let (is_dense, node_coords, pairs) = SURFACE_PLAN_CACHE.with(|cache| {
                        cache.borrow().get(surface.canvas).map_or(
                            (false, Vec::new(), Vec::new()),
                            |entry| {
                                let coords: Vec<(f64, f64)> =
                                    entry.nodes.iter().map(|n| (n.x, n.y)).collect();
                                (entry.is_dense, coords, entry.pairs.clone())
                            },
                        )
                    });
                    let wire_endpoints =
                        |a: (f64, f64), b: (f64, f64)| -> ((f64, f64), (f64, f64)) {
                            if a.0 + BOX_WIDTH <= b.0 {
                                (
                                    (a.0 + BOX_WIDTH, a.1 + BOX_HEIGHT * 0.5),
                                    (b.0, b.1 + BOX_HEIGHT * 0.5),
                                )
                            } else if b.0 + BOX_WIDTH <= a.0 {
                                (
                                    (a.0, a.1 + BOX_HEIGHT * 0.5),
                                    (b.0 + BOX_WIDTH, b.1 + BOX_HEIGHT * 0.5),
                                )
                            } else {
                                (
                                    (a.0 + BOX_WIDTH * 0.5, a.1 + BOX_HEIGHT * 0.5),
                                    (b.0 + BOX_WIDTH * 0.5, b.1 + BOX_HEIGHT * 0.5),
                                )
                            }
                        };
                    if is_dense {
                        graph_viewport::canvas2d::update_scene(surface.canvas, |scene| {
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
                                let (p0, p3) = wire_endpoints(a, b);
                                let dx = (p3.0 - p0.0) * 0.45;
                                let p1 = (p0.0 + dx, p0.1);
                                let p2 = (p3.0 - dx, p3.1);
                                de.pts = vec![p0, p1, p2, p3];
                                de.cmds.clear();
                            }
                        });
                        graph_viewport::canvas2d::paint_scene(
                            surface.canvas,
                            cam.viewport_cam(),
                            current_dense_hover(surface.canvas),
                            true,
                        );
                    } else if let Ok(wires) = canvas.query_selector_all(
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
                            let (s, e) = wire_endpoints(a, b);
                            let mut d = format!("M{:.1} {:.1}", s.0, s.1);
                            curve(&mut d, s, e);
                            let _ = w_el.set_attribute("d", &d);
                        }
                    }
                }
                drag["moved"] = json!(true);
                drag["cur_x"] = json!(nx);
                drag["cur_y"] = json!(ny);
                m::set("surface-node-drag", drag);
                return true;
            }
            m::set("surface-node-drag", json!({}));
            if moved && kind == "pointerup" {
                let key = m::text(&drag, "node_key");
                let cur_x = drag["cur_x"].as_f64().unwrap_or(0.0);
                let cur_y = drag["cur_y"].as_f64().unwrap_or(0.0);
                if !key.is_empty() {
                    let st = surface_state(surface);
                    let pin_key = surface_pin_key(surface, &st);
                    push_pin_undo_snapshot(surface, &pin_key);
                    graph_viewport::save_pin(&pin_key, &key, (cur_x, cur_y));
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
            } else if moved && kind == "pointercancel" {
                rerender_surface_preserving_camera(surface);
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
    let rect = canvas.get_bounding_client_rect();
    let cam = load_surface_camera(surface);
    let wx = (pe.client_x() as f64 - rect.left() - cam.pan_x) / cam.scale.max(0.05);
    let wy = (pe.client_y() as f64 - rect.top() - cam.pan_y) / cam.scale.max(0.05);
    let hit = graph_viewport::canvas2d::hit_test_scene(surface.canvas, wx, wy, 8.0);
    if kind == "pointermove" {
        if set_dense_hover(surface.canvas, hit) {
            graph_viewport::canvas2d::paint_scene(surface.canvas, cam.viewport_cam(), hit, true);
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
    let payload_action = SURFACE_PLAN_CACHE.with(|cache| {
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
            if target_el
                .as_ref()
                .and_then(|el| {
                    el.closest(
                        ".graph-map-node, .graph-map-entity-node, .graph-map-spread, button, a",
                    )
                    .ok()
                    .flatten()
                })
                .is_some()
            {
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
