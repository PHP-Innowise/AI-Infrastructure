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

fn resolve_system_service_member_project(sys_pid: &str, service_id: &str) -> Option<String> {
    if service_id.is_empty() || service_id == "__system__" {
        return (!sys_pid.is_empty()).then(|| sys_pid.to_owned());
    }
    let status = m::data("system-graph-status");
    let status_pid = m::text(&status, "project_id");
    let ui_sys_pid = if cfg!(target_arch = "wasm32") {
        dom::value("system-project")
    } else {
        String::new()
    };
    if sys_pid.is_empty() || status_pid == sys_pid || ui_sys_pid == sys_pid {
        if let Some(gid) = status["services"][service_id]["graph_id"]
            .as_str()
            .filter(|s| !s.is_empty())
        {
            return Some(gid.to_owned());
        }
    }
    let catalog = m::data("system-catalog")["catalog"].clone();
    for svc in m::array(&catalog["services"]) {
        if m::text(&svc, "id") != service_id {
            continue;
        }
        if let Some(gid) = svc["graph_id"].as_str().filter(|s| !s.is_empty()) {
            return Some(gid.to_owned());
        }
        let svc_root = normalize_fs_path(&m::text(&svc, "root"));
        if svc_root.is_empty() {
            continue;
        }
        let sys_path = normalize_fs_path(m::text(&m::project(sys_pid), "path").as_str());
        let is_abs =
            svc_root.starts_with('/') || (svc_root.len() >= 2 && svc_root.as_bytes()[1] == b':');
        let resolved_root = if !is_abs && !sys_path.is_empty() {
            normalize_fs_path(&format!(
                "{}/{}",
                sys_path.trim_end_matches('/'),
                svc_root.trim_start_matches("./")
            ))
        } else {
            svc_root
        };
        for proj in m::read(|s| m::array(&s.boot["projects"])) {
            let p_path = normalize_fs_path(&m::text(&proj, "path"));
            if !p_path.is_empty() && p_path == resolved_root {
                let pid = m::text(&proj, "id");
                if !pid.is_empty() {
                    return Some(pid);
                }
            }
        }
    }
    None
}

fn surface_matches_live_project(
    surface: MapSurface,
    target_project: &str,
    live_project: &str,
) -> bool {
    if target_project.is_empty() {
        return true;
    }
    if surface.name == "session" {
        return live_project.is_empty() || live_project == target_project;
    }
    if let Some(stored) = GraphTargetScope::from_json(&m::data(surface.scope_model_key)) {
        match &stored {
            GraphTargetScope::SingleProject { project_id } => {
                let eff = if surface.name == "modal" && !live_project.is_empty() {
                    live_project
                } else {
                    project_id.as_str()
                };
                return eff.is_empty() || eff == target_project;
            }
            GraphTargetScope::SystemService {
                project_id,
                service_id,
                ..
            } => {
                if project_id.is_empty() || project_id == target_project {
                    return true;
                }
                if resolve_system_service_member_project(project_id, service_id).as_deref()
                    == Some(target_project)
                {
                    return true;
                }
                if service_id == "__system__" {
                    let status = m::data("system-graph-status");
                    let cat = m::data("system-catalog")["catalog"].clone();
                    let sys_meta = m::project(project_id);
                    let ev_meta = m::project(target_project);
                    return system_project_matches(
                        target_project,
                        project_id,
                        &status,
                        &cat,
                        sys_meta["path"].as_str(),
                        ev_meta["path"].as_str(),
                    );
                }
                return false;
            }
        }
    }
    live_project.is_empty() || live_project == target_project
}

pub(crate) fn show_non_destructive_stale_banner(project_id: &str, message: &str) {
    let text = if message.trim().is_empty() {
        "Graph updated — Refresh"
    } else {
        message.trim()
    };
    let live_project = dom::value("project");
    if open() && surface_matches_live_project(MODAL_SURFACE, project_id, &live_project) {
        dom::text("graph-map-stale-text", text);
        dom::hidden("graph-map-stale-banner", false);
    }
    for surface in [
        SESSION_SURFACE,
        SYSTEM_EMBEDDED_SURFACE,
        SYSTEM_CHANGE_SURFACE,
    ] {
        if dom::get(surface.canvas).is_some()
            && surface_matches_live_project(surface, project_id, &live_project)
        {
            dom::text(&format!("{}-stale-text", surface.id_prefix), text);
            dom::hidden(&format!("{}-stale-banner", surface.id_prefix), false);
        }
    }
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
        if surface_matches_live_project(surface, project_id, live_project) {
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
fn refresh_active_external_views(proj: &str, poll_system_status: bool) {
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
            if poll_system_status {
                spawn_local(async {
                    crate::systems::poll_system_graph_status(false).await;
                });
            }
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

pub(crate) fn refresh_external_views_for_stale_sse(project_id: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        refresh_active_external_views(project_id, true);
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = project_id;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SseRefreshMode {
    SkipAlreadyCurrent,
    TargetedPatch,
    FullReload,
}

fn path_within_scope(path: &str, scope: &str) -> bool {
    let p = normalize_rel_path(path);
    let s = scope.trim_matches('/');
    !p.is_empty() && (s.is_empty() || p == s || p.starts_with(&format!("{s}/")))
}

fn edge_touches_removed(edge: &V, removed_set: &BTreeSet<String>) -> bool {
    let from = normalize_rel_path(&m::text(edge, "from"));
    let to = normalize_rel_path(&m::text(edge, "to"));
    removed_set.contains(&from) || removed_set.contains(&to)
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(super) fn plan_sse_graph_refresh(
    delta: &crate::graph::SseDeltaPayload,
    prev_nodes: Option<u64>,
    prev_edges: Option<u64>,
    view: &V,
    state: &V,
) -> SseRefreshMode {
    let Some(nodes) = view["nodes"].as_array() else {
        return SseRefreshMode::FullReload;
    };
    if !view.is_object() {
        return SseRefreshMode::FullReload;
    }
    if delta.full_reload
        || delta.schema_changed
        || delta.changed_files.len() > 32
        || delta.removed_files.len() > 32
    {
        return SseRefreshMode::FullReload;
    }
    let counts_changed =
        (delta.nodes.is_some() && prev_nodes.is_some() && delta.nodes != prev_nodes)
            || (delta.edges.is_some() && prev_edges.is_some() && delta.edges != prev_edges);
    let view_rev = view["ir_revision"].as_u64();
    if delta.revision.is_some()
        && view_rev == delta.revision
        && delta.changed_files.is_empty()
        && delta.removed_files.is_empty()
        && !counts_changed
    {
        return SseRefreshMode::SkipAlreadyCurrent;
    }
    if m::yes(&view["preliminary"]) || m::yes(&view["truncated"]) {
        return SseRefreshMode::FullReload;
    }
    let mode = view["mode"].as_str().unwrap_or("");
    let lod = view["lod"].as_str().unwrap_or("");
    if matches!(
        mode,
        "layers" | "architecture" | "tests" | "history" | "link" | "cycle"
    ) || matches!(lod, "crate" | "folder")
        || m::yes(&state["modules"])
        || m::yes(&state["history"])
        || state["cycle"].is_string()
        || state["pattern"].is_string()
        || !m::text(state, "path_nodes").is_empty()
    {
        return SseRefreshMode::FullReload;
    }

    let removed_set: BTreeSet<String> = delta
        .removed_files
        .iter()
        .map(|f| normalize_rel_path(f))
        .filter(|f| !f.is_empty())
        .collect();
    if !removed_set.is_empty() && view["focus"].is_object() {
        let fpath = normalize_rel_path(&m::text(&view["focus"], "path"));
        if !fpath.is_empty() && removed_set.contains(&fpath) {
            return SseRefreshMode::FullReload;
        }
    }

    let mut loaded_paths: BTreeSet<String> = BTreeSet::new();
    let mut removed_in_view = 0usize;
    for node in nodes {
        let p = normalize_rel_path(&m::text(node, "path"));
        let f = normalize_rel_path(&m::text(node, "file"));
        let is_folder = m::yes(&node["folder"]) || node["files"].is_u64();
        if is_folder
            && !p.is_empty()
            && removed_set
                .iter()
                .any(|rem| p == "." || path_within_scope(rem, &p))
        {
            // Removing a file rolled up into a folder node alters aggregated folder counts/edges.
            return SseRefreshMode::FullReload;
        }
        if (!p.is_empty() && removed_set.contains(&p))
            || (!f.is_empty() && removed_set.contains(&f))
        {
            removed_in_view += 1;
        }
        if !p.is_empty() {
            loaded_paths.insert(p);
        }
        if !f.is_empty() {
            loaded_paths.insert(f);
        }
    }

    let scope = m::text(state, "scope");
    let view_scope = m::text(view, "scope");
    let eff_scope = if !scope.trim_matches('/').is_empty() {
        scope.clone()
    } else {
        view_scope
    };
    let unscoped = eff_scope.trim_matches('/').is_empty()
        && m::text(state, "focus").is_empty()
        && !m::yes(&state["isolate_active"]);
    for rem in &removed_set {
        if !loaded_paths.contains(rem) && !unscoped && path_within_scope(rem, &eff_scope) {
            return SseRefreshMode::FullReload;
        }
    }
    for changed in &delta.changed_files {
        let norm = normalize_rel_path(changed);
        if norm.is_empty() || removed_set.contains(&norm) || loaded_paths.contains(&norm) {
            continue;
        }
        // Newly created/unloaded file visible in current scope requires FullReload to add its node.
        if unscoped || path_within_scope(&norm, &eff_scope) {
            return SseRefreshMode::FullReload;
        }
    }

    if unscoped {
        let surviving_nodes = nodes.len().saturating_sub(removed_in_view) as u64;
        if delta.nodes.is_some_and(|n| n != surviving_nodes) {
            return SseRefreshMode::FullReload;
        }
        if let Some(edges) = view["edges"].as_array() {
            let removed_edges = edges
                .iter()
                .filter(|e| edge_touches_removed(e, &removed_set))
                .count();
            let surviving_edges = edges.len().saturating_sub(removed_edges) as u64;
            if delta.edges.is_some_and(|e| e != surviving_edges) {
                return SseRefreshMode::FullReload;
            }
        } else if delta.edges.is_some_and(|e| e > 0) && removed_in_view == 0 {
            return SseRefreshMode::FullReload;
        }
    }

    if delta.changed_files.is_empty() && delta.removed_files.is_empty() && !counts_changed {
        return SseRefreshMode::FullReload;
    }

    SseRefreshMode::TargetedPatch
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(super) fn apply_sse_delta_patch_to_view(
    view: &mut V,
    delta: &crate::graph::SseDeltaPayload,
) -> usize {
    if let Some(rev) = delta.revision {
        view["ir_revision"] = json!(rev);
    }
    view["stale"] = json!(delta.state == "stale");
    let mut patched = 0usize;

    let removed_set: BTreeSet<String> = delta
        .removed_files
        .iter()
        .map(|f| normalize_rel_path(f))
        .filter(|f| !f.is_empty())
        .collect();
    if !removed_set.is_empty() {
        let mut dropped_node_paths: BTreeSet<String> = BTreeSet::new();
        let mut removed_file_count = 0u64;
        let mut dropped_nodes_count = 0usize;
        if let Some(nodes) = view["nodes"].as_array_mut() {
            let mut next_nodes = Vec::with_capacity(nodes.len());
            for mut node in std::mem::take(nodes) {
                let path = normalize_rel_path(&m::text(&node, "path"));
                let file = normalize_rel_path(&m::text(&node, "file"));
                let is_folder = m::yes(&node["folder"]) || node["files"].is_u64();
                let is_leaf_file = !is_folder
                    && !m::yes(&node["symbol"])
                    && !m::yes(&node["semantic"])
                    && !path.starts_with("sem:")
                    && !path.starts_with("__semantic__/");
                if !is_folder {
                    if (!path.is_empty() && removed_set.contains(&path))
                        || (!file.is_empty() && removed_set.contains(&file))
                    {
                        if !path.is_empty() {
                            dropped_node_paths.insert(path);
                        }
                        if is_leaf_file {
                            removed_file_count += 1;
                        }
                        dropped_nodes_count += 1;
                        patched += 1;
                        continue;
                    }
                    next_nodes.push(node);
                } else {
                    let child_rem = if path.is_empty() {
                        0u64
                    } else {
                        removed_set
                            .iter()
                            .filter(|rem| path == "." || path_within_scope(rem, &path))
                            .count() as u64
                    };
                    if child_rem > 0 {
                        if let Some(prev_files) = node["files"].as_u64() {
                            let decremented = child_rem.min(prev_files);
                            let remaining = prev_files.saturating_sub(child_rem);
                            removed_file_count += decremented;
                            if remaining == 0 {
                                if !path.is_empty() {
                                    dropped_node_paths.insert(path);
                                }
                                dropped_nodes_count += 1;
                                patched += 1;
                                continue;
                            }
                            node["files"] = json!(remaining);
                            patched += 1;
                        }
                    }
                    next_nodes.push(node);
                }
            }
            *nodes = next_nodes;
        }

        let mut lost_out: HashMap<String, u64> = HashMap::new();
        let mut lost_in: HashMap<String, u64> = HashMap::new();
        let mut removed_links = 0u64;
        let mut removed_guessed = 0u64;
        let mut dropped_edges_count = 0usize;
        if let Some(edges) = view["edges"].as_array_mut() {
            let before = edges.len();
            edges.retain(|edge| {
                let from = normalize_rel_path(&m::text(edge, "from"));
                let to = normalize_rel_path(&m::text(edge, "to"));
                let drop_from = removed_set.contains(&from) || dropped_node_paths.contains(&from);
                let drop_to = removed_set.contains(&to) || dropped_node_paths.contains(&to);
                if drop_from || drop_to {
                    let links = edge["links"].as_u64().unwrap_or(1).max(1);
                    let guessed = edge["guessed"].as_u64().unwrap_or(0).min(links);
                    removed_links += links;
                    removed_guessed += guessed;
                    if !drop_from && !from.is_empty() {
                        *lost_out.entry(from).or_default() += links;
                    }
                    if !drop_to && !to.is_empty() {
                        *lost_in.entry(to).or_default() += links;
                    }
                    false
                } else {
                    true
                }
            });
            dropped_edges_count = before.saturating_sub(edges.len());
            patched += dropped_edges_count;
        }

        if let Some(nodes) = view["nodes"].as_array_mut() {
            for node in nodes.iter_mut() {
                let path = normalize_rel_path(&m::text(node, "path"));
                if let Some(&d_out) = lost_out.get(&path) {
                    if let Some(prev) = node["out"].as_u64() {
                        node["out"] = json!(prev.saturating_sub(d_out));
                    }
                }
                if let Some(&d_in) = lost_in.get(&path) {
                    if let Some(prev) = node["in"].as_u64() {
                        node["in"] = json!(prev.saturating_sub(d_in));
                    }
                }
            }
        }
        if view["focus"].is_object() {
            for side in ["uses", "used_by"] {
                if let Some(arr) = view["focus"][side].as_array_mut() {
                    arr.retain(|entry| {
                        let p = normalize_rel_path(&m::text(entry, "path"));
                        p.is_empty()
                            || (!removed_set.contains(&p) && !dropped_node_paths.contains(&p))
                    });
                }
            }
        }
        if let Some(totals) = view["totals"].as_object_mut() {
            if let Some(prev) = totals.get("files").and_then(V::as_u64) {
                totals.insert(
                    "files".into(),
                    json!(prev.saturating_sub(removed_file_count)),
                );
            }
            if let Some(prev) = totals.get("links").and_then(V::as_u64) {
                totals.insert("links".into(), json!(prev.saturating_sub(removed_links)));
            }
            if let Some(prev) = totals.get("guessed").and_then(V::as_u64) {
                totals.insert(
                    "guessed".into(),
                    json!(prev.saturating_sub(removed_guessed)),
                );
            }
            if let Some(prev) = totals.get("read").and_then(V::as_u64) {
                let removed_read = removed_links.saturating_sub(removed_guessed);
                totals.insert("read".into(), json!(prev.saturating_sub(removed_read)));
            }
            if let Some(prev) = totals.get("nodes").and_then(V::as_u64) {
                totals.insert(
                    "nodes".into(),
                    json!(prev.saturating_sub(dropped_nodes_count as u64)),
                );
            }
            if let Some(prev) = totals.get("edges").and_then(V::as_u64) {
                totals.insert(
                    "edges".into(),
                    json!(prev.saturating_sub(dropped_edges_count as u64)),
                );
            }
        }
    }

    if delta.nodes.is_some() || delta.edges.is_some() {
        if let Some(totals) = view["totals"].as_object_mut() {
            if let Some(n) = delta.nodes {
                if totals.get("symbols").is_some_and(V::is_number) {
                    totals.insert("symbols".into(), json!(n));
                    patched += 1;
                }
            }
            if let Some(e) = delta.edges {
                if totals.get("relations").is_some_and(V::is_number) {
                    totals.insert("relations".into(), json!(e));
                    patched += 1;
                }
            }
        }
    }

    let changed_set: BTreeSet<String> = delta
        .changed_files
        .iter()
        .map(|f| normalize_rel_path(f))
        .filter(|f| !f.is_empty())
        .collect();
    let plain_map = view["chosen"].is_null();
    if !changed_set.is_empty() || plain_map {
        if let Some(nodes) = view["nodes"].as_array_mut() {
            for node in nodes {
                let path = normalize_rel_path(&m::text(node, "path"));
                let file = normalize_rel_path(&m::text(node, "file"));
                let is_folder = m::yes(&node["folder"]) || node["files"].is_u64();
                let match_count = if changed_set.is_empty() {
                    0u64
                } else if is_folder {
                    if path.is_empty() {
                        0
                    } else {
                        changed_set
                            .iter()
                            .filter(|c| path == "." || path_within_scope(c, &path))
                            .count() as u64
                    }
                } else if (!path.is_empty() && changed_set.contains(&path))
                    || (!file.is_empty() && changed_set.contains(&file))
                {
                    1
                } else {
                    0
                };
                if match_count > 0 {
                    node["changed"] = json!(match_count);
                    node["sse_changed"] = json!(true);
                    patched += 1;
                } else if node["sse_changed"] == json!(true)
                    || (plain_map
                        && (node["changed"].as_u64().unwrap_or(0) > 0
                            || node["changed"].as_bool() == Some(true)))
                {
                    node["changed"] = json!(0);
                    if let Some(obj) = node.as_object_mut() {
                        obj.remove("sse_changed");
                    }
                    patched += 1;
                }
            }
        }
        if view["focus"].is_object() {
            let fpath = normalize_rel_path(&m::text(&view["focus"], "path"));
            let matched = !fpath.is_empty() && changed_set.contains(&fpath);
            if matched {
                view["focus"]["changed"] = json!(1);
                view["focus"]["sse_changed"] = json!(true);
                patched += 1;
            } else if view["focus"]["sse_changed"] == json!(true)
                || (plain_map
                    && (view["focus"]["changed"].as_u64().unwrap_or(0) > 0
                        || view["focus"]["changed"].as_bool() == Some(true)))
            {
                view["focus"]["changed"] = json!(0);
                if let Some(obj) = view["focus"].as_object_mut() {
                    obj.remove("sse_changed");
                }
                patched += 1;
            }
        }
    }
    patched
}

#[cfg(target_arch = "wasm32")]
fn patch_surface_sse_delta(
    surface: MapSurface,
    delta: &crate::graph::SseDeltaPayload,
    prev_nodes: Option<u64>,
    prev_edges: Option<u64>,
) -> SseRefreshMode {
    let mut view = m::data(surface.view_key);
    let surf_state = surface_state(surface);
    let mode = plan_sse_graph_refresh(delta, prev_nodes, prev_edges, &view, &surf_state);
    if mode == SseRefreshMode::TargetedPatch {
        apply_sse_delta_patch_to_view(&mut view, delta);
        m::set(surface.view_key, view);
        rerender_surface_preserving_camera(surface);
        if delta.state != "stale" {
            dom::hidden(&format!("{}-stale-banner", surface.id_prefix), true);
        }
    }
    mode
}

/// Schedules a non-disruptive debounced reload (or targeted SSE delta patch when safe)
/// of open/mounted surfaces when no pointer pan or pin drag is in progress, keeping
/// camera and selection intact.
pub(crate) fn maybe_schedule_idle_refresh(project_id: &str) {
    if any_graph_drag_active() {
        return;
    }
    #[cfg(target_arch = "wasm32")]
    {
        let proj = project_id.to_owned();
        let timer_key = if proj.is_empty() {
            "graph-sse-idle-refresh".to_owned()
        } else {
            format!("graph-sse-idle-refresh:{proj}")
        };
        dom::later(&timer_key, 250, move || {
            if any_graph_drag_active() {
                return;
            }
            let raw_event = if proj.is_empty() {
                m::data("graph-sse-status")
            } else {
                let per_proj = m::data(&format!("graph-sse-status:{proj}"));
                if per_proj.is_object() {
                    per_proj
                } else {
                    m::data("graph-sse-status")
                }
            };
            let delta = crate::graph::extract_sse_delta_payload(&raw_event);
            let delta_meta = if proj.is_empty() {
                m::data("graph-sse-delta-meta")
            } else {
                let per_proj = m::data(&format!("graph-sse-delta-meta:{proj}"));
                if per_proj.is_object() {
                    per_proj
                } else {
                    m::data("graph-sse-delta-meta")
                }
            };
            let prev_nodes = delta_meta["prev_nodes"].as_u64();
            let prev_edges = delta_meta["prev_edges"].as_u64();
            let live_project = dom::value("project");

            let mut need_full_modal = false;
            if open() && surface_matches_live_project(MODAL_SURFACE, &proj, &live_project) {
                need_full_modal =
                    patch_surface_sse_delta(MODAL_SURFACE, &delta, prev_nodes, prev_edges)
                        == SseRefreshMode::FullReload;
            }

            let mut embedded_full = Vec::new();
            for surface in [
                SESSION_SURFACE,
                SYSTEM_EMBEDDED_SURFACE,
                SYSTEM_CHANGE_SURFACE,
            ] {
                if dom::get(surface.canvas).is_some()
                    && surface_matches_live_project(surface, &proj, &live_project)
                    && patch_surface_sse_delta(surface, &delta, prev_nodes, prev_edges)
                        == SseRefreshMode::FullReload
                {
                    embedded_full.push(surface);
                }
            }

            if need_full_modal {
                let also_session = embedded_full.iter().any(|s| s.name == SESSION_SURFACE.name);
                spawn_local(async move {
                    crate::graph::load(false).await;
                    if open() {
                        modal_lifecycle::load(true).await;
                    }
                    if !also_session {
                        for surface in embedded_full {
                            load_embedded_surface(surface, true).await;
                        }
                    } else {
                        for surface in embedded_full
                            .into_iter()
                            .filter(|s| s.name != SESSION_SURFACE.name)
                        {
                            load_embedded_surface(surface, true).await;
                        }
                    }
                });
            } else {
                for surface in embedded_full {
                    spawn_local(async move {
                        load_embedded_surface(surface, true).await;
                    });
                }
            }
            let from_sys_status = !m::text(&raw_event, "source_project_id").is_empty()
                || !m::text(&raw_event, "member_project_id").is_empty()
                || m::text(&raw_event, "action") == "ir_delta";
            refresh_active_external_views(&proj, !from_sys_status);
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AutoLodTier {
    Crate,
    Folder,
    Full,
}

pub(super) fn auto_lod_tier_for_scale(scale: f64) -> AutoLodTier {
    if scale < 0.35 {
        AutoLodTier::Crate
    } else if scale < 0.65 {
        AutoLodTier::Folder
    } else {
        AutoLodTier::Full
    }
}

pub(super) fn loaded_view_differs_from_lod(
    view: &V,
    target: &(Option<String>, Option<u64>),
) -> bool {
    if !view.is_object() {
        return false;
    }
    let view_lod = view["lod"]
        .as_str()
        .map(str::trim)
        .filter(|s| matches!(*s, "crate" | "folder"))
        .map(str::to_owned);
    let view_weight = view["min_weight"].as_u64().filter(|&w| w >= 2);
    (&view_lod, &view_weight) != (&target.0, &target.1)
}

pub(super) fn should_trigger_auto_lod_for_target(state: &V, view: &V, scale: f64) -> bool {
    if !m::yes(&state["auto_lod"])
        || state["lod"].as_str().is_some_and(|s| !s.trim().is_empty())
        || state["min_weight"].as_u64().is_some_and(|w| w >= 1)
    {
        return false;
    }
    let target = progressive_lod_for_state(state, Some(scale));
    loaded_view_differs_from_lod(view, &target)
}

pub(super) fn should_trigger_auto_lod_reload(
    state: &V,
    view: &V,
    prev_scale: f64,
    next_scale: f64,
) -> bool {
    if !prev_scale.is_finite() || !next_scale.is_finite() || next_scale <= 0.0 {
        return false;
    }
    if auto_lod_tier_for_scale(prev_scale) == auto_lod_tier_for_scale(next_scale)
        && !should_trigger_auto_lod_for_target(state, view, next_scale)
    {
        return false;
    }
    should_trigger_auto_lod_for_target(state, view, next_scale)
}

fn live_camera_scale_for_surface_key(state: &V, camera_key: &str) -> Option<f64> {
    if m::yes(&state["fit"]) || camera_key.is_empty() {
        return None;
    }
    let raw_cam = m::data(camera_key);
    if !raw_cam.is_object() || raw_cam["fit_mode"].as_bool() != Some(false) {
        return None;
    }
    raw_cam["scale"]
        .as_f64()
        .filter(|s| s.is_finite() && *s > 0.0)
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
        .or_else(|| state["zoom"].as_f64())
        .filter(|s| s.is_finite() && *s > 0.0)
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

pub(in crate::graph_map) fn build_surface_map_path(
    surface: MapSurface,
    scope: &GraphTargetScope,
    state: &V,
) -> String {
    build_scoped_map_path_with_camera_key(scope, state, surface.embedded, surface.camera_key)
}

pub fn build_scoped_map_path(scope: &GraphTargetScope, state: &V, embedded: bool) -> String {
    let camera_key = if embedded {
        SESSION_SURFACE.camera_key
    } else {
        MODAL_SURFACE.camera_key
    };
    build_scoped_map_path_with_camera_key(scope, state, embedded, camera_key)
}

fn build_scoped_map_path_with_camera_key(
    scope: &GraphTargetScope,
    state: &V,
    embedded: bool,
    camera_key: &str,
) -> String {
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
    let live_scale = live_camera_scale_for_surface_key(state, camera_key);
    let (lod_val, min_weight_val) = progressive_lod_for_state(state, live_scale);
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

    #[test]
    fn auto_lod_threshold_crossing_and_loaded_view_diff_checks() {
        let state = json!({
            "auto_lod": true,
            "scope": "",
            "focus": "",
        });
        let full_view = json!({ "mode": "map", "nodes": [] });
        let folder_view = json!({ "mode": "map", "lod": "folder", "nodes": [] });
        let crate_view = json!({ "mode": "map", "lod": "crate", "min_weight": 2, "nodes": [] });

        assert_eq!(auto_lod_tier_for_scale(0.20), AutoLodTier::Crate);
        assert_eq!(auto_lod_tier_for_scale(0.35), AutoLodTier::Folder);
        assert_eq!(auto_lod_tier_for_scale(0.65), AutoLodTier::Full);

        // Crossing 0.65 downward while full view is loaded triggers reload; staying at 0.8 -> 0.7 does not.
        assert!(!should_trigger_auto_lod_reload(
            &state, &full_view, 0.85, 0.72
        ));
        assert!(should_trigger_auto_lod_reload(
            &state, &full_view, 0.72, 0.55
        ));
        // Crossing 0.35 downward while folder view is loaded triggers crate reload.
        assert!(should_trigger_auto_lod_reload(
            &state,
            &folder_view,
            0.50,
            0.30
        ));
        // Already loaded at crate LOD -> zooming within < 0.35 does not reload again.
        assert!(!should_trigger_auto_lod_reload(
            &state,
            &crate_view,
            0.30,
            0.20
        ));

        // Embedded surfaces (SESSION_SURFACE, SYSTEM_EMBEDDED_SURFACE, SYSTEM_CHANGE_SURFACE)
        // share auto_lod camera-scale URL routing using each surface's own camera_key.
        for surf in [
            SESSION_SURFACE,
            SYSTEM_EMBEDDED_SURFACE,
            SYSTEM_CHANGE_SURFACE,
        ] {
            m::set(surf.camera_key, json!({ "scale": 0.25, "fit_mode": false }));
            let scope = GraphTargetScope::single("proj-emb");
            let url = build_surface_map_path(surf, &scope, &state);
            m::take(surf.camera_key);
            assert!(
                url.contains("lod=crate") && url.contains("min_weight=2"),
                "expected surface {} auto_lod URL to include crate LOD: {url}",
                surf.name
            );
        }
    }

    #[test]
    fn sse_refresh_planner_distinguishes_skip_patch_and_full_reload() {
        let base_state = json!({});
        let map_view = json!({
            "mode": "map",
            "ir_revision": 10,
            "nodes": [
                { "path": "src/auth.rs", "name": "auth.rs", "out": 1, "in": 0 },
                { "path": "src/db.rs", "name": "db.rs", "out": 0, "in": 1 }
            ],
            "edges": [
                { "from": "src/auth.rs", "to": "src/db.rs", "links": 1, "guessed": 0 }
            ],
            "totals": {
                "files": 2,
                "links": 1,
                "read": 1,
                "guessed": 0,
                "symbols": 12,
                "relations": 18,
                "nodes": 2,
                "edges": 1
            }
        });

        // Identical revision without changed files skips reload.
        let same_rev = crate::graph::extract_sse_delta_payload(&json!({
            "state": "ready",
            "reason": "ir_updated",
            "revision": 10
        }));
        assert_eq!(
            plan_sse_graph_refresh(&same_rev, Some(2), Some(1), &map_view, &base_state),
            SseRefreshMode::SkipAlreadyCurrent
        );

        // Incremental changed_files sets u64 `changed: 1` (compatible with render/mod.rs `.as_u64()`)
        // and subsequent deltas clear stale flags on untouched plain-map nodes.
        let inc = crate::graph::extract_sse_delta_payload(&json!({
            "state": "ready",
            "reason": "incremental_update",
            "revision": 11,
            "nodes": 2,
            "edges": 1,
            "changed_files": ["src/auth.rs"]
        }));
        assert_eq!(
            plan_sse_graph_refresh(&inc, Some(2), Some(1), &map_view, &base_state),
            SseRefreshMode::TargetedPatch
        );
        let mut patched_view = map_view.clone();
        let patched_n = apply_sse_delta_patch_to_view(&mut patched_view, &inc);
        assert!(patched_n >= 1);
        assert_eq!(patched_view["ir_revision"], json!(11));
        assert_eq!(patched_view["nodes"][0]["changed"], json!(1));
        assert!(patched_view["nodes"][1]["changed"].is_null());

        let inc2 = crate::graph::extract_sse_delta_payload(&json!({
            "state": "ready",
            "reason": "incremental_update",
            "revision": 12,
            "nodes": 2,
            "edges": 1,
            "changed_files": ["src/db.rs"]
        }));
        apply_sse_delta_patch_to_view(&mut patched_view, &inc2);
        assert_eq!(
            patched_view["nodes"][0]["changed"],
            json!(0),
            "stale changed flag on src/auth.rs must reset cleanly"
        );
        assert_eq!(patched_view["nodes"][1]["changed"], json!(1));

        // Safe node + edge removal decrements view totals (files/links/read) without conflating IR counts.
        let removed_safe = crate::graph::extract_sse_delta_payload(&json!({
            "state": "ready",
            "reason": "incremental_update",
            "revision": 12,
            "nodes": 1,
            "edges": 0,
            "removed_files": ["src/db.rs"],
            "changed_files": ["src/auth.rs"]
        }));
        assert_eq!(
            plan_sse_graph_refresh(&removed_safe, Some(2), Some(1), &map_view, &base_state),
            SseRefreshMode::TargetedPatch
        );
        let mut removed_patched = map_view.clone();
        let rem_cnt = apply_sse_delta_patch_to_view(&mut removed_patched, &removed_safe);
        assert!(rem_cnt >= 2);
        assert_eq!(removed_patched["nodes"].as_array().unwrap().len(), 1);
        assert_eq!(removed_patched["edges"].as_array().unwrap().len(), 0);
        assert_eq!(removed_patched["nodes"][0]["out"], json!(0));
        assert_eq!(removed_patched["totals"]["files"], json!(1));
        assert_eq!(removed_patched["totals"]["links"], json!(0));
        assert_eq!(removed_patched["totals"]["read"], json!(0));
        assert_eq!(removed_patched["totals"]["nodes"], json!(1));
        assert_eq!(removed_patched["totals"]["edges"], json!(0));
        assert_eq!(removed_patched["totals"]["symbols"], json!(1));
        assert_eq!(removed_patched["totals"]["relations"], json!(0));

        // Scoped views receiving out-of-scope global IR deltas update symbols/relations ONLY,
        // preserving view-level `files`/`links`/`nodes`/`edges`.
        let scoped_state = json!({ "scope": "src/auth" });
        let out_of_scope_counts = crate::graph::extract_sse_delta_payload(&json!({
            "state": "ready",
            "reason": "incremental_update",
            "revision": 13,
            "nodes": 400,
            "edges": 800,
            "changed_files": ["packages/billing/invoice.ts"]
        }));
        assert_eq!(
            plan_sse_graph_refresh(
                &out_of_scope_counts,
                Some(390),
                Some(780),
                &map_view,
                &scoped_state
            ),
            SseRefreshMode::TargetedPatch
        );
        let mut scoped_patched = map_view.clone();
        apply_sse_delta_patch_to_view(&mut scoped_patched, &out_of_scope_counts);
        assert_eq!(scoped_patched["totals"]["symbols"], json!(400));
        assert_eq!(scoped_patched["totals"]["relations"], json!(800));
        assert_eq!(scoped_patched["totals"]["files"], json!(2));
        assert_eq!(scoped_patched["totals"]["links"], json!(1));

        // Scoped folder view receiving removed_files for a child inside a collapsed folder must
        // request FullReload in planner AND cleanly decrement/drop folders if patched directly.
        let scoped_folder_view = json!({
            "mode": "map",
            "scope": "src",
            "ir_revision": 10,
            "nodes": [
                { "path": "src/nested", "name": "nested", "folder": true, "files": 2, "out": 1, "in": 0 },
                { "path": "src/single", "name": "single", "folder": true, "files": 1, "out": 0, "in": 1 }
            ],
            "edges": [
                { "from": "src/nested", "to": "src/single", "links": 2, "guessed": 1 }
            ],
            "totals": { "files": 3, "links": 2, "read": 1, "guessed": 1, "symbols": 20, "relations": 30 }
        });
        let child_removed_only = crate::graph::extract_sse_delta_payload(&json!({
            "state": "ready",
            "reason": "incremental_update",
            "revision": 14,
            "removed_files": ["src/single/only.rs"],
            "changed_files": []
        }));
        assert_eq!(
            plan_sse_graph_refresh(
                &child_removed_only,
                Some(20),
                Some(30),
                &scoped_folder_view,
                &json!({ "scope": "src" })
            ),
            SseRefreshMode::FullReload
        );
        let mut folder_patched = scoped_folder_view.clone();
        apply_sse_delta_patch_to_view(&mut folder_patched, &child_removed_only);
        assert_eq!(folder_patched["nodes"].as_array().unwrap().len(), 1);
        assert_eq!(folder_patched["nodes"][0]["path"], "src/nested");
        assert_eq!(folder_patched["edges"].as_array().unwrap().len(), 0);
        assert_eq!(folder_patched["totals"]["files"], json!(2));
        assert_eq!(folder_patched["totals"]["links"], json!(0));
        assert_eq!(folder_patched["totals"]["read"], json!(0));
        assert_eq!(folder_patched["totals"]["guessed"], json!(0));

        // Unscoped brand-new nodes, removing focused node, or aggregate modes still trigger FullReload.
        let count_changed_unloaded = crate::graph::extract_sse_delta_payload(&json!({
            "state": "ready",
            "reason": "incremental_update",
            "revision": 14,
            "nodes": 3,
            "edges": 1,
            "changed_files": ["src/auth.rs"]
        }));
        assert_eq!(
            plan_sse_graph_refresh(
                &count_changed_unloaded,
                Some(2),
                Some(1),
                &map_view,
                &base_state
            ),
            SseRefreshMode::FullReload
        );

        let focus_view = json!({
            "mode": "file",
            "ir_revision": 10,
            "focus": { "path": "src/old.rs" },
            "nodes": [
                { "path": "src/old.rs", "name": "old.rs" },
                { "path": "src/auth.rs", "name": "auth.rs" }
            ],
            "edges": []
        });
        let removed_focus = crate::graph::extract_sse_delta_payload(&json!({
            "state": "ready",
            "reason": "incremental_update",
            "revision": 15,
            "removed_files": ["src/old.rs"],
            "changed_files": []
        }));
        assert_eq!(
            plan_sse_graph_refresh(&removed_focus, Some(2), Some(0), &focus_view, &base_state),
            SseRefreshMode::FullReload
        );

        let layers_view = json!({
            "mode": "layers",
            "ir_revision": 10,
            "nodes": [{ "path": "layer:Api" }],
        });
        assert_eq!(
            plan_sse_graph_refresh(&inc, Some(2), Some(0), &layers_view, &base_state),
            SseRefreshMode::FullReload
        );
    }

    #[test]
    fn system_service_surfaces_match_member_graph_id_for_sse_invalidation() {
        m::set(
            "system-graph-status",
            json!({
                "project_id": "proj-sys",
                "services": {
                    "api": { "service": "api", "graph_id": "proj-api", "state": "ready" },
                    "worker": { "service": "worker", "graph_id": "proj-worker", "state": "ready" }
                }
            }),
        );
        let api_scope = GraphTargetScope::SystemService {
            project_id: "proj-sys".to_owned(),
            config_path: "system.json".to_owned(),
            service_id: "api".to_owned(),
            services: vec!["api".to_owned(), "worker".to_owned()],
            run_id: None,
            run_title: None,
            launches: Vec::new(),
        };
        m::set(SYSTEM_EMBEDDED_SURFACE.scope_model_key, api_scope.to_json());
        m::set(
            SYSTEM_EMBEDDED_SURFACE.changes_key,
            json!({"data": {"files": []}}),
        );

        assert!(surface_matches_live_project(
            SYSTEM_EMBEDDED_SURFACE,
            "proj-api",
            "proj-other"
        ));
        assert!(!surface_matches_live_project(
            SYSTEM_EMBEDDED_SURFACE,
            "proj-worker",
            "proj-other"
        ));
        invalidate_surface_analysis_caches_for_live_project("proj-api", "proj-other");
        assert!(m::data(SYSTEM_EMBEDDED_SURFACE.changes_key).is_null());

        let sys_root_scope = GraphTargetScope::SystemService {
            project_id: "proj-sys".to_owned(),
            config_path: "system.json".to_owned(),
            service_id: "__system__".to_owned(),
            services: vec!["api".to_owned(), "worker".to_owned()],
            run_id: None,
            run_title: None,
            launches: Vec::new(),
        };
        m::set(
            SYSTEM_CHANGE_SURFACE.scope_model_key,
            sys_root_scope.to_json(),
        );
        assert!(surface_matches_live_project(
            SYSTEM_CHANGE_SURFACE,
            "proj-api",
            "proj-other"
        ));
    }
}
