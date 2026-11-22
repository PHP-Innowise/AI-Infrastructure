use super::*;

pub(in crate::graph_map) fn surface_by_name(name: &str) -> MapSurface {
    match name {
        "session" => SESSION_SURFACE,
        "system-member" => SYSTEM_EMBEDDED_SURFACE,
        "system-change" => SYSTEM_CHANGE_SURFACE,
        _ => MODAL_SURFACE,
    }
}
pub(in crate::graph_map) fn surface_scope(surface: MapSurface) -> GraphTargetScope {
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
pub(in crate::graph_map) fn set_surface_scope(surface: MapSurface, scope: GraphTargetScope) {
    m::set(surface.scope_model_key, scope.to_json());
}
pub(in crate::graph_map) fn surface_state(surface: MapSurface) -> V {
    let raw = m::data(surface.state_model_key);
    if raw.is_object() {
        normalize_map_state(raw)
    } else {
        fresh("")
    }
}
pub(in crate::graph_map) fn effective_target_from(target: &str, compare_base: &str) -> String {
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
pub(in crate::graph_map) fn effective_target(state: &V) -> String {
    effective_target_from(&m::text(state, "target"), &m::text(state, "compare_base"))
}
pub(in crate::graph_map) fn save_surface_state(surface: MapSurface, state: &V) {
    m::set(surface.state_model_key, normalize_map_state(state.clone()));
}
pub(in crate::graph_map) fn surface_changes_doc(surface: MapSurface, state: &V) -> Option<V> {
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
pub(in crate::graph_map) fn impact_cache_key(scope_key: &str, path: &str, state: &V) -> String {
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
pub(in crate::graph_map) fn looks_like_file_path(path: &str) -> bool {
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
pub(in crate::graph_map) fn is_explicit_folder(path: &str, view: Option<&V>) -> bool {
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
pub(in crate::graph_map) fn is_known_file(
    path: &str,
    view: Option<&V>,
    changes: Option<&V>,
) -> bool {
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
pub(in crate::graph_map) fn cached_impact_for_selection(
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
pub(in crate::graph_map) fn encode_part(value: &str) -> String {
    dom::enc(value)
}
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::graph_map) fn encode_part(value: &str) -> String {
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
pub(in crate::graph_map) fn encode_query(params: &[(&str, String)]) -> String {
    dom::query(params)
}
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::graph_map) fn encode_query(params: &[(&str, String)]) -> String {
    if params.is_empty() {
        return String::new();
    }
    let parts = params
        .iter()
        .map(|(k, v)| format!("{}={}", encode_part(k), encode_part(v)))
        .collect::<Vec<_>>();
    format!("?{}", parts.join("&"))
}
pub(in crate::graph_map) fn effective_sysrun_id(
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
pub(in crate::graph_map) fn format_scoped_endpoint(
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
pub(in crate::graph_map) fn changes_count_of(changes: Option<&V>) -> u64 {
    let Some(doc) = changes.filter(|v| v.is_object()) else {
        return 0;
    };
    doc["total_files"]
        .as_u64()
        .unwrap_or_else(|| m::array(&doc["files"]).len() as u64)
}
pub(in crate::graph_map) fn update_surface_changes_badge(surface: MapSurface, changes: Option<&V>) {
    if !surface.embedded {
        return;
    }
    let badge_id = format!("{}-changes-count", surface.id_prefix);
    let n = changes_count_of(changes);
    dom::text(&badge_id, &n.to_string());
    dom::hidden(&badge_id, n == 0);
}
pub(in crate::graph_map) fn surface_pin_key(surface: MapSurface, state: &V) -> String {
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

pub(in crate::graph_map) struct OwnedOverlayExtras {
    pub(in crate::graph_map) pins: BTreeMap<String, (f64, f64)>,
    pub(in crate::graph_map) path_nodes: BTreeSet<usize>,
    pub(in crate::graph_map) path_edges: BTreeSet<(usize, usize)>,
    pub(in crate::graph_map) isolate_cone: Option<BTreeSet<usize>>,
    pub(in crate::graph_map) viewport_bounds: Option<ViewportBounds>,
    pub(in crate::graph_map) roving_focus: Option<usize>,
}

pub(in crate::graph_map) fn resolve_node_index_by_query(nodes: &[V], raw_q: &str) -> Option<usize> {
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

pub(in crate::graph_map) fn build_overlay_extras(
    surface: MapSurface,
    view: &V,
    state: &V,
) -> OwnedOverlayExtras {
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

fn resolved_surface_project_for_invalidation(surface: MapSurface, live_project: &str) -> String {
    if surface.name == "session" {
        return live_project.to_owned();
    }
    if let Some(stored) = GraphTargetScope::from_json(&m::data(surface.scope_model_key)) {
        if stored.is_system() || live_project.is_empty() {
            return stored.project_id().to_owned();
        }
        return live_project.to_owned();
    }
    live_project.to_owned()
}

pub(crate) fn invalidate_surface_analysis_caches_for_live_project(
    project_id: &str,
    live_project: &str,
) {
    for surface in [
        MODAL_SURFACE,
        SESSION_SURFACE,
        SYSTEM_EMBEDDED_SURFACE,
        SYSTEM_CHANGE_SURFACE,
    ] {
        let surf_proj = resolved_surface_project_for_invalidation(surface, live_project);
        if project_id.is_empty() || surf_proj.is_empty() || surf_proj == project_id {
            m::next(surface.changes_epoch);
            m::next(surface.impact_epoch);
            m::set(surface.changes_key, V::Null);
            m::set(surface.impact_key, V::Null);
        }
    }
}

/// Invalidates `changes_key`/`impact_key` caches and advances fetch epochs for surfaces
/// belonging to `project_id` (or all surfaces when `project_id` is empty).
pub(crate) fn invalidate_surface_analysis_caches(project_id: &str) {
    let live = if project_id.is_empty() {
        String::new()
    } else {
        dom::value("project")
    };
    invalidate_surface_analysis_caches_for_live_project(project_id, &live);
}

fn any_graph_drag_active() -> bool {
    active_drag_surface().is_some()
        || [
            "surface-node-drag",
            "system-map-drag",
            "system-change-topology-drag",
            "system-map-node-drag",
            "system-change-topology-node-drag",
            "system-map-radar-drag",
            "omnigraph-drag",
            "omnigraph-node-drag",
            "omnigraph-radar-drag",
        ]
        .iter()
        .any(|key| m::yes(&m::data(key)["active"]))
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(super) fn project_scope_matches(event_project: &str, surface_project: &str) -> bool {
    event_project.is_empty() || surface_project.is_empty() || surface_project == event_project
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn normalize_fs_path(raw: &str) -> String {
    let norm = raw.trim().replace('\\', "/");
    let trimmed = norm.trim_end_matches('/');
    if trimmed.is_empty() && norm.starts_with('/') {
        "/".to_owned()
    } else {
        trimmed.to_owned()
    }
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(super) fn system_project_matches(
    event_project: &str,
    system_project: &str,
    graph_status: &V,
    catalog: &V,
    system_project_path: Option<&str>,
    event_project_path: Option<&str>,
) -> bool {
    if system_project.is_empty() {
        return false;
    }
    if event_project.is_empty() || system_project == event_project {
        return true;
    }
    if m::text(graph_status, "project_id") == event_project
        || m::text(&graph_status["system"], "graph_id") == event_project
    {
        return true;
    }
    if graph_status["services"]
        .as_object()
        .is_some_and(|services| {
            services
                .values()
                .any(|svc| m::text(svc, "graph_id") == event_project)
        })
    {
        return true;
    }
    let ev_path = event_project_path
        .map(normalize_fs_path)
        .filter(|p| !p.is_empty());
    let sys_path = system_project_path
        .map(normalize_fs_path)
        .filter(|p| !p.is_empty());
    if let Some(ev_path) = ev_path {
        for svc in m::array(&catalog["services"]) {
            let root = normalize_fs_path(&m::text(&svc, "root"));
            if root.is_empty() {
                continue;
            }
            if root == ev_path {
                return true;
            }
            let is_abs = root.starts_with('/') || (root.len() >= 2 && root.as_bytes()[1] == b':');
            if let Some(base) = sys_path.as_deref().filter(|_| !is_abs) {
                let joined = format!(
                    "{}/{}",
                    base.trim_end_matches('/'),
                    root.trim_start_matches("./")
                );
                if normalize_fs_path(&joined) == ev_path {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(target_arch = "wasm32")]
fn dom_visible(id: &str) -> bool {
    dom::get(id).is_some_and(|el| el.closest("[hidden]").ok().flatten().is_none())
}

#[cfg(target_arch = "wasm32")]
fn refresh_active_external_views(proj: &str) {
    let active_view = m::read(|s| s.view.clone());
    if matches!(active_view.as_str(), "systems" | "system-changes") {
        let sys_project = dom::value("system-project");
        let sys_config = dom::value("system-config");
        let cat = m::data("system-catalog")["catalog"].clone();
        let graph_status = m::data("system-graph-status");
        let sys_proj_meta = m::project(&sys_project);
        let ev_proj_meta = m::project(proj);
        if system_project_matches(
            proj,
            &sys_project,
            &graph_status,
            &cat,
            sys_proj_meta["path"].as_str(),
            ev_proj_meta["path"].as_str(),
        ) {
            if active_view == "systems"
                && dom_visible("system-map-canvas")
                && (cat.is_object() || m::data("system-map-doc")["nodes"].is_array())
            {
                crate::system_map::refresh_services_map(
                    &sys_project,
                    &sys_config,
                    &cat,
                    &graph_status,
                );
            }
            if active_view == "system-changes" && dom_visible("system-change-topology-canvas") {
                let run_doc = m::data("system-run");
                if cat.is_object()
                    || run_doc.is_object()
                    || m::data("system-change-topology-doc")["nodes"].is_array()
                {
                    let checked_seeds: Vec<String> = dom::selected_checks("system-plan-services")
                        .into_iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect();
                    let depth = dom::value("system-depth").parse::<u64>().unwrap_or(32);
                    let contracts_raw = dom::value("system-contracts");
                    let plan_form_open =
                        dom::get("system-plan-form").is_some_and(|f| !f.has_attribute("hidden"));
                    let overlay = crate::systems::build_changes_overlay(
                        &run_doc,
                        plan_form_open,
                        &checked_seeds,
                        depth,
                        &contracts_raw,
                    );
                    crate::system_map::refresh_changes_map(
                        &sys_project,
                        &sys_config,
                        &cat,
                        &graph_status,
                        &overlay,
                    );
                }
            }
        }
    }
    if active_view == "omnigraph"
        && m::yes(&m::data("omnigraph-status")["connected"])
        && dom_visible("omnigraph-studio")
    {
        let omni_proj = crate::omnigraph::current_project_id();
        if project_scope_matches(proj, &omni_proj) {
            m::take("omnigraph-workspace-changes");
            m::take("omnigraph-workspace-impact");
            spawn_local(async {
                crate::omnigraph::refresh_graph_preserving_pages().await;
            });
        }
    }
}

/// Schedules a non-disruptive debounced reload of open/mounted surfaces when no pointer
/// pan or pin drag is in progress, keeping camera and selection intact.
pub(crate) fn maybe_schedule_idle_refresh(project_id: &str) {
    if any_graph_drag_active() {
        return;
    }
    #[cfg(target_arch = "wasm32")]
    {
        let proj = project_id.to_owned();
        dom::later("graph-sse-idle-refresh", 250, move || {
            if any_graph_drag_active() {
                return;
            }
            if open() {
                let modal_proj = surface_scope(MODAL_SURFACE).project_id().to_owned();
                if project_scope_matches(&proj, &modal_proj) {
                    spawn_local(async {
                        crate::graph::load(false).await;
                        if open() {
                            modal_lifecycle::load(true).await;
                        }
                    });
                }
            }
            for surface in [
                SESSION_SURFACE,
                SYSTEM_EMBEDDED_SURFACE,
                SYSTEM_CHANGE_SURFACE,
            ] {
                if dom::get(surface.canvas).is_some() {
                    let surf_proj = surface_scope(surface).project_id().to_owned();
                    if project_scope_matches(&proj, &surf_proj) {
                        spawn_local(async move {
                            load_embedded_surface(surface, true).await;
                        });
                    }
                }
            }
            refresh_active_external_views(&proj);
        });
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = project_id;
}

pub(in crate::graph_map) fn active_modal_lens(state: &V, view: &V) -> &'static str {
    if m::yes(&state["lens_runs"])
        || m::yes(&state["history"])
        || m::text(view, "mode") == "history"
    {
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

pub(in crate::graph_map) fn sync_modal_lens_and_display(state: &V, view: &V) {
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
    let wanted_lod = state["lod"]
        .as_str()
        .map(str::trim)
        .filter(|s| matches!(*s, "crate" | "folder" | "file" | "symbol"))
        .unwrap_or("");
    if dom::value("graph-map-lod") != wanted_lod {
        dom::set_value("graph-map-lod", wanted_lod);
    }
    let wanted_weight = state["min_weight"]
        .as_u64()
        .filter(|&w| w >= 1)
        .map(|w| w.to_string())
        .unwrap_or_default();
    if dom::value("graph-map-min-weight") != wanted_weight {
        dom::set_value("graph-map-min-weight", &wanted_weight);
    }
}

pub(in crate::graph_map) fn sync_surface_toolbar(
    surface: MapSurface,
    state: &V,
    changes: Option<&V>,
) {
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

pub fn progressive_lod_for_state(
    state: &V,
    camera_scale: Option<f64>,
) -> (Option<String>, Option<u64>) {
    let explicit_lod = state["lod"]
        .as_str()
        .map(str::trim)
        .filter(|s| matches!(*s, "crate" | "folder" | "file" | "symbol"))
        .map(str::to_owned);
    let explicit_weight = state["min_weight"]
        .as_u64()
        .filter(|&w| w >= 1)
        .map(|w| w.min(1_000_000));
    if explicit_lod.is_some() || explicit_weight.is_some() {
        return (explicit_lod, explicit_weight);
    }
    if !m::yes(&state["auto_lod"]) {
        return (None, None);
    }
    let scale = camera_scale
        .or_else(|| state["scale"].as_f64())
        .unwrap_or(1.0);
    let no_submode = state["cycle"].is_null()
        && state["pattern"].is_null()
        && state["scenario"].is_null()
        && m::text(state, "follow").is_empty()
        && !m::yes(&state["modules"])
        && !m::yes(&state["history"]);
    if no_submode && m::text(state, "scope").is_empty() && m::text(state, "focus").is_empty() {
        if scale < 0.35 {
            return (Some("crate".to_owned()), Some(2));
        }
        if scale < 0.65 {
            return (Some("folder".to_owned()), None);
        }
    }
    (None, None)
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
    let (lod_val, min_weight_val) = progressive_lod_for_state(state, None);
    if let Some(level) = lod_val {
        query.push(("lod", level));
    }
    if let Some(weight) = min_weight_val {
        query.push(("min_weight", weight.to_string()));
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
pub(in crate::graph_map) fn build_map_path(project: &str, state: &V, embedded: bool) -> String {
    build_scoped_map_path(&GraphTargetScope::single(project), state, embedded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_and_system_scope_matching_respects_empty_and_member_projects() {
        assert!(project_scope_matches("", "proj-a"));
        assert!(project_scope_matches("proj-a", ""));
        assert!(project_scope_matches("proj-a", "proj-a"));
        assert!(!project_scope_matches("proj-a", "proj-b"));

        let catalog = json!({
            "schema_version": 1,
            "system": "sys-demo",
            "services": [
                { "id": "api", "root": "services/api", "access": "available", "head": "abc", "declared": {} },
                { "id": "worker", "root": "/repos/worker", "access": "available", "head": "def", "declared": {} }
            ],
            "relationships": [],
            "warnings": []
        });
        let graph_status = json!({
            "project_id": "proj-sys",
            "system": { "service": "__system__", "graph_id": "proj-sys" },
            "services": {
                "api": { "service": "api", "graph_id": "proj-api", "state": "ready" },
                "worker": { "service": "worker", "graph_id": "proj-worker", "state": "ready" }
            }
        });
        assert!(!system_project_matches(
            "proj-sys",
            "",
            &graph_status,
            &catalog,
            None,
            None,
        ));
        assert!(system_project_matches(
            "",
            "proj-sys",
            &graph_status,
            &catalog,
            None,
            None,
        ));
        assert!(system_project_matches(
            "proj-sys",
            "proj-sys",
            &graph_status,
            &catalog,
            None,
            None,
        ));
        assert!(system_project_matches(
            "proj-api",
            "proj-sys",
            &graph_status,
            &catalog,
            None,
            None,
        ));
        assert!(!system_project_matches(
            "proj-other",
            "proj-sys",
            &graph_status,
            &catalog,
            None,
            None,
        ));
        // Fallback before system-graph-status arrives: match catalog[*].root (relative or absolute) against boot project path.
        assert!(system_project_matches(
            "proj-api",
            "proj-sys",
            &V::Null,
            &catalog,
            Some("/repos/sys"),
            Some("/repos/sys/services/api/"),
        ));
        assert!(system_project_matches(
            "proj-worker",
            "proj-sys",
            &V::Null,
            &catalog,
            Some("/repos/sys"),
            Some("/repos/worker"),
        ));
        assert!(!system_project_matches(
            "proj-other",
            "proj-sys",
            &V::Null,
            &catalog,
            Some("/repos/sys"),
            Some("/repos/unrelated"),
        ));
    }

    #[test]
    fn any_graph_drag_active_covers_system_and_omnigraph_drags() {
        for key in [
            "surface-node-drag",
            "system-map-drag",
            "system-change-topology-drag",
            "system-map-node-drag",
            "system-change-topology-node-drag",
            "system-map-radar-drag",
            "omnigraph-drag",
            "omnigraph-node-drag",
            "omnigraph-radar-drag",
        ] {
            m::take(key);
        }
        assert!(!any_graph_drag_active());

        m::set("system-map-drag", json!({ "active": true }));
        assert!(any_graph_drag_active());
        m::take("system-map-drag");

        m::set("omnigraph-node-drag", json!({ "active": true }));
        assert!(any_graph_drag_active());
        m::take("omnigraph-node-drag");
        assert!(!any_graph_drag_active());
    }
}
