//! Interactive cross-service architecture and contract dependency map for System Orchestration.
//!
//! Renders and coordinates two synchronized surfaces:
//! - `#system-map-*` in the `Services` tab (`SERVICES_SURFACE`)
//! - `#system-change-topology-*` in the `Changes` tab (`CHANGES_SURFACE`)
//!
//! Backed by `GET /api/systems/graph/{map,find,changes,impact}` with instant
//! catalog synthesis fallback so blast-radius, wave status, contract links,
//! cycles, paths, shared dependencies, and architectural tiers work smoothly
//! both before and after member code graphs finish indexing.

use crate::{api, dom, graph_viewport, model as m, systems};
use serde_json::{json, Value as V};
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use wasm_bindgen::{closure::Closure, JsCast};
use wasm_bindgen_futures::spawn_local;

pub const SERVICES_SURFACE_ID: &str = "services";
pub const CHANGES_SURFACE_ID: &str = "changes";

pub(super) const NODE_WIDTH: f64 = 196.0;
pub(super) const NODE_HEIGHT: f64 = 58.0;
pub(super) const CHILD_WIDTH: f64 = 164.0;
pub(super) const CHILD_HEIGHT: f64 = 30.0;
pub(super) const COL_GAP: f64 = 96.0;
pub(super) const ROW_GAP: f64 = 20.0;
pub(super) const MARGIN: f64 = 24.0;
pub(super) const FRAME_HEAD: f64 = 28.0;
pub(super) const FRAME_PAD: f64 = 12.0;

pub(super) const CAMERA_MIN: f64 = graph_viewport::DEFAULT_MIN_SCALE;
pub(super) const CAMERA_MAX: f64 = 2.4;
pub(super) const ZOOM_STEP: f64 = 1.25;
pub(super) const LOD_FAR: f64 = 0.55;
pub(super) const LOD_NEAR: f64 = 1.25;
pub(super) const SYSTEM_MAP_LOD: graph_viewport::LodThresholds = graph_viewport::LodThresholds {
    far_below: LOD_FAR,
    near_at: LOD_NEAR,
    near_inclusive: true,
    invalid_fallback: "far",
};
pub(super) const RADAR_WIDTH: f64 = 176.0;
pub(super) const RADAR_HEIGHT: f64 = 112.0;
pub(super) const RADAR_DRAG_KEY: &str = "system-map-radar-drag";
pub(super) const SYS_PORT_GAP: f64 = 6.0;
pub(super) const SYS_PORT_SPREAD: f64 = 18.0;

#[derive(Clone, Copy, Debug)]
pub struct SysMapSurface {
    pub id: &'static str,
    pub id_prefix: &'static str,
    pub workspace_id: &'static str,
    pub canvas_id: &'static str,
    pub side_id: &'static str,
    pub summary_id: &'static str,
    pub note_id: &'static str,
    pub error_id: &'static str,
    pub legend_id: &'static str,
    pub find_wrap_id: &'static str,
    pub find_input_id: &'static str,
    pub find_found_id: &'static str,
    pub find_list_id: &'static str,
    pub find_note_id: &'static str,
    pub filter_id: &'static str,
    pub state_key: &'static str,
    pub map_key: &'static str,
    pub changes_key: &'static str,
    pub impact_key: &'static str,
    pub camera_key: &'static str,
    pub drag_key: &'static str,
    pub fetch_epoch: &'static str,
    pub find_epoch: &'static str,
    pub changes_epoch: &'static str,
    pub impact_epoch: &'static str,
}

pub const SERVICES_SURFACE: SysMapSurface = SysMapSurface {
    id: SERVICES_SURFACE_ID,
    id_prefix: "system-map",
    workspace_id: "system-map-workspace",
    canvas_id: "system-map-canvas",
    side_id: "system-map-side",
    summary_id: "system-map-summary",
    note_id: "system-map-note",
    error_id: "system-map-error",
    legend_id: "system-map-arrow-legend",
    find_wrap_id: "system-map-find",
    find_input_id: "system-map-find-search",
    find_found_id: "system-map-find-found",
    find_list_id: "system-map-find-list",
    find_note_id: "system-map-find-note",
    filter_id: "system-map-filter",
    state_key: "system-map-state",
    map_key: "system-map-doc",
    changes_key: "system-map-changes-doc",
    impact_key: "system-map-impact-doc",
    camera_key: "system-map-camera",
    drag_key: "system-map-drag",
    fetch_epoch: "system-map-fetch",
    find_epoch: "system-map-find-fetch",
    changes_epoch: "system-map-changes-fetch",
    impact_epoch: "system-map-impact-fetch",
};

pub const CHANGES_SURFACE: SysMapSurface = SysMapSurface {
    id: CHANGES_SURFACE_ID,
    id_prefix: "system-change-topology",
    workspace_id: "system-change-map-workspace",
    canvas_id: "system-change-topology-canvas",
    side_id: "system-change-topology-side",
    summary_id: "system-change-topology-summary",
    note_id: "system-change-topology-note",
    error_id: "system-change-topology-error",
    legend_id: "system-change-topology-arrow-legend",
    find_wrap_id: "system-change-topology-find",
    find_input_id: "system-change-topology-find-search",
    find_found_id: "system-change-topology-find-found",
    find_list_id: "system-change-topology-find-list",
    find_note_id: "system-change-topology-find-note",
    filter_id: "system-change-topology-filter",
    state_key: "system-change-topology-state",
    map_key: "system-change-topology-doc",
    changes_key: "system-change-topology-changes-doc",
    impact_key: "system-change-topology-impact-doc",
    camera_key: "system-change-topology-camera",
    drag_key: "system-change-topology-drag",
    fetch_epoch: "system-change-topology-fetch",
    find_epoch: "system-change-topology-find-fetch",
    changes_epoch: "system-change-topology-changes-fetch",
    impact_epoch: "system-change-topology-impact-fetch",
};

pub(super) fn surface_by_id(id: &str) -> SysMapSurface {
    if id == CHANGES_SURFACE_ID || id.starts_with("system-change-topology") {
        CHANGES_SURFACE
    } else {
        SERVICES_SURFACE
    }
}

pub(super) fn surface_for_canvas(canvas_id: &str) -> Option<SysMapSurface> {
    match canvas_id {
        "system-map-canvas" => Some(SERVICES_SURFACE),
        "system-change-topology-canvas" => Some(CHANGES_SURFACE),
        _ => None,
    }
}

#[derive(Clone, Copy)]
pub struct ContractArrowSpec {
    pub kind: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub dash_array: Option<&'static str>,
    pub marker_body: &'static str,
    pub swatch_line_end: f64,
}

pub const CONTRACT_ARROW_SPECS: [ContractArrowSpec; 7] = [
    ContractArrowSpec {
        kind: "http",
        label: "HTTP",
        description: "HTTP / REST endpoint contract (solid, filled triangle)",
        dash_array: None,
        marker_body: "<path d=\"M0 0 L8 4 L0 8 z\"/>",
        swatch_line_end: 21.0,
    },
    ContractArrowSpec {
        kind: "event",
        label: "Event",
        description: "Async domain event / message bus contract (dashed, filled diamond)",
        dash_array: Some("7 4"),
        marker_body: "<path d=\"M0.5 4 L4 0.8 L7.5 4 L4 7.2 Z\"/>",
        swatch_line_end: 21.0,
    },
    ContractArrowSpec {
        kind: "rpc",
        label: "RPC",
        description: "gRPC / RPC service contract (solid, barbed chevron)",
        dash_array: None,
        marker_body: "<path d=\"M0.4 0.8 L7.8 4 L0.4 7.2 L2.5 4 z\"/>",
        swatch_line_end: 22.0,
    },
    ContractArrowSpec {
        kind: "graphql",
        label: "GraphQL",
        description: "GraphQL schema / query contract (dash-dot, hollow diamond)",
        dash_array: Some("6 3 2 3"),
        marker_body: "<path d=\"M0.8 4 L4 1.0 L7.2 4 L4 7.0 Z\"/>",
        swatch_line_end: 21.0,
    },
    ContractArrowSpec {
        kind: "other",
        label: "Other",
        description: "Custom or untyped declared contract (dotted, open chevron)",
        dash_array: Some("3 3"),
        marker_body: "<path d=\"M1 1 L7.5 4 L1 7\"/>",
        swatch_line_end: 26.5,
    },
    ContractArrowSpec {
        kind: "shared",
        label: "Shared",
        description: "Shared system source, policy, or schema dependency (long-dash, hollow triangle)",
        dash_array: Some("10 4"),
        marker_body: "<path d=\"M0.8 0.8 L7.8 4 L0.8 7.2 Z\"/>",
        swatch_line_end: 21.0,
    },
    ContractArrowSpec {
        kind: "code",
        label: "Code",
        description: "Cross-service code reference (solid when verified, warning-dashed when undeclared drift)",
        dash_array: None,
        marker_body: "<path d=\"M0.5 1 L7.8 4 L0.5 7 L2.4 4 z\"/>",
        swatch_line_end: 22.0,
    },
];

pub fn normalize_contract_arrow_kind(raw: &str) -> &'static str {
    let key = raw.trim().to_ascii_lowercase().replace([' ', '-'], "_");
    match key.as_str() {
        "http" | "rest" | "openapi" | "webhook" | "api" | "endpoint" => "http",
        "event" | "events" | "kafka" | "amqp" | "queue" | "pubsub" | "topic" | "message" => "event",
        "rpc" | "grpc" | "protobuf" | "twirp" | "jsonrpc" => "rpc",
        "graphql" | "gql" | "federation" => "graphql",
        "shared" | "shared_source" | "policy" | "spec" | "schema" | "system" => "shared",
        "code" | "imports" | "calls" | "symbol" | "reference" | "undeclared" => "code",
        "other" | "contract" | "custom" => "other",
        _ => "http",
    }
}

pub(super) fn spec_for_kind(kind: &str) -> &'static ContractArrowSpec {
    let norm = normalize_contract_arrow_kind(kind);
    CONTRACT_ARROW_SPECS
        .iter()
        .find(|spec| spec.kind == norm)
        .unwrap_or(&CONTRACT_ARROW_SPECS[0])
}

pub fn primary_edge_kind(edge: &V) -> &'static str {
    if let Some(kind) = edge["kind"].as_str().filter(|s| !s.trim().is_empty()) {
        return normalize_contract_arrow_kind(kind);
    }
    if let Some(kinds) = edge["kinds"].as_array() {
        for item in kinds {
            if let Some(s) = item.as_str().filter(|s| !s.trim().is_empty()) {
                return normalize_contract_arrow_kind(s);
            }
            let k = m::text(item, "kind");
            if !k.trim().is_empty() {
                return normalize_contract_arrow_kind(&k);
            }
        }
    }
    if let Some(contracts) = edge["contracts"].as_array() {
        for c in contracts {
            if let Some(k) = c["kind"].as_str().filter(|s| !s.trim().is_empty()) {
                return normalize_contract_arrow_kind(k);
            }
        }
    }
    if m::text(edge, "provenance") == "undeclared_in_code" {
        return "code";
    }
    if m::text(edge, "to") == "__system__" || m::text(edge, "from") == "__system__" {
        return "shared";
    }
    "http"
}

pub(super) fn escape(raw: &str) -> String {
    crate::dom::escape_html(raw)
}

#[inline]
pub(super) fn push_attr(buf: &mut String, name: &str, val: &str) {
    buf.push(' ');
    buf.push_str(name);
    buf.push_str("=\"");
    buf.push_str(&escape(val));
    buf.push('"');
}

#[inline]
pub(super) fn push_raw_attr(buf: &mut String, name: &str, val: &str) {
    buf.push(' ');
    buf.push_str(name);
    buf.push_str("=\"");
    buf.push_str(val);
    buf.push('"');
}

pub(super) fn payload_attr(value: &V) -> String {
    escape(&value.to_string())
}

pub(super) fn contract_arrow_defs_svg(id_prefix: &str) -> String {
    let mut defs = String::new();
    for spec in CONTRACT_ARROW_SPECS {
        defs.push_str(&format!(
            "<marker id=\"{id_prefix}-arrow-{}\" class=\"system-map-arrow system-map-arrow-{}\" viewBox=\"0 0 8 8\" refX=\"7\" refY=\"4\" markerWidth=\"7\" markerHeight=\"7\" markerUnits=\"userSpaceOnUse\" orient=\"auto\">{}</marker>",
            spec.kind, spec.kind, spec.marker_body,
        ));
    }
    defs.push_str(&format!(
        "<marker id=\"{id_prefix}-arrow-against\" class=\"system-map-arrow system-map-arrow-against\" viewBox=\"0 0 8 8\" refX=\"7\" refY=\"4\" markerWidth=\"7\" markerHeight=\"7\" markerUnits=\"userSpaceOnUse\" orient=\"auto\"><path d=\"M0 0 L8 4 L0 8 z\"/></marker>"
    ));
    defs.push_str(&format!(
        "<marker id=\"{id_prefix}-arrow-broken\" class=\"system-map-arrow system-map-arrow-broken\" viewBox=\"0 0 8 8\" refX=\"7\" refY=\"4\" markerWidth=\"7\" markerHeight=\"7\" markerUnits=\"userSpaceOnUse\" orient=\"auto\"><path d=\"M0 0 L8 4 L0 8 z\"/></marker>"
    ));
    defs
}

pub(super) fn contract_spec_index(kind: &str) -> usize {
    let norm = normalize_contract_arrow_kind(kind);
    CONTRACT_ARROW_SPECS
        .iter()
        .position(|s| s.kind == norm)
        .unwrap_or(0)
}

/// Returns which of the 7 contract arrow kinds are hidden in `state["kinds_off"]`.
pub fn hidden_contract_kinds(state: &V) -> [bool; 7] {
    let mut hidden = [false; 7];
    for entry in m::array(&state["kinds_off"]) {
        if let Some(kind) = entry.as_str() {
            let idx = CONTRACT_ARROW_SPECS
                .iter()
                .position(|s| s.kind == kind.trim().to_ascii_lowercase());
            if let Some(i) = idx {
                hidden[i] = true;
            }
        }
    }
    hidden
}

/// Counts edges matching each of the 7 canonical contract arrow kinds in `doc`.
pub fn contract_kind_counts(doc: &V) -> [u64; 7] {
    let mut counts = [0u64; 7];
    for edge in m::array(&doc["edges"]) {
        if m::yes(&edge["child_link"]) {
            continue;
        }
        let kind = primary_edge_kind(&edge);
        counts[contract_spec_index(kind)] += 1;
    }
    counts
}

pub(super) fn write_hidden_contract_kinds(mut state: V, hidden: [bool; 7]) -> V {
    let list: Vec<&'static str> = CONTRACT_ARROW_SPECS
        .iter()
        .enumerate()
        .filter_map(|(i, s)| hidden[i].then_some(s.kind))
        .collect();
    state["kinds_off"] = json!(list);
    state
}

/// Toggles or isolates (`solo == true`, e.g. Alt+click) one contract kind in `state["kinds_off"]`.
/// Never allows hiding all 7 kinds at once; if hiding the last visible kind, keeps at least one shown.
pub fn switched_contract_kind(state: V, kind: &str, solo: bool) -> V {
    let idx = contract_spec_index(kind);
    let current = hidden_contract_kinds(&state);
    let mut next = current;
    if solo {
        let already_solo = (0..7).all(|i| current[i] == (i != idx));
        if already_solo {
            next = [false; 7];
        } else {
            for (i, slot) in next.iter_mut().enumerate() {
                *slot = i != idx;
            }
        }
    } else {
        next[idx] = !current[idx];
        if next.iter().all(|h| *h) {
            next = [false; 7];
        }
    }
    write_hidden_contract_kinds(state, next)
}

/// Resets or applies a preset (`"all"`) on contract kind visibility in `state["kinds_off"]`.
pub fn preset_contract_kinds(state: V, preset: &str) -> V {
    match preset {
        "all" => write_hidden_contract_kinds(state, [false; 7]),
        _ => state,
    }
}

/// Interactive filter switchboard + accessible legend of all 7 cross-service contract & code arrow kinds.
#[derive(Clone, Debug, PartialEq)]
pub struct Camera {
    pub scale: f64,
    pub pan_x: f64,
    pub pan_y: f64,
    pub plane_w: f64,
    pub plane_h: f64,
    pub fit_mode: bool,
    pub needs_measure: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            scale: 1.0,
            pan_x: 16.0,
            pan_y: 16.0,
            plane_w: 0.0,
            plane_h: 0.0,
            fit_mode: true,
            needs_measure: true,
        }
    }
}

impl Camera {
    fn viewport_cam(&self) -> graph_viewport::Camera {
        graph_viewport::Camera::new(self.pan_x, self.pan_y, self.scale)
    }

    fn apply_viewport_cam(&mut self, cam: graph_viewport::Camera) {
        self.scale = cam.scale;
        self.pan_x = cam.tx;
        self.pan_y = cam.ty;
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn zoom_at(
    scale: f64,
    pan_x: f64,
    pan_y: f64,
    anchor: (f64, f64),
    target_scale: f64,
) -> (f64, f64, f64) {
    let next = graph_viewport::zoom_at(
        graph_viewport::Camera::new(pan_x, pan_y, scale),
        anchor,
        target_scale,
        CAMERA_MIN,
        CAMERA_MAX,
    );
    (next.scale, next.tx, next.ty)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn camera_for_fit(plane: (f64, f64), viewport: (f64, f64)) -> (f64, f64, f64) {
    let cam = graph_viewport::fit(
        plane,
        viewport,
        graph_viewport::FitOptions::for_system_map(),
    );
    (cam.scale, cam.tx, cam.ty)
}

pub(super) fn load_camera(surface: SysMapSurface) -> Camera {
    let raw = m::data(surface.camera_key);
    if !raw.is_object() {
        return Camera::default();
    }
    Camera {
        scale: raw["scale"]
            .as_f64()
            .unwrap_or(1.0)
            .clamp(CAMERA_MIN, CAMERA_MAX),
        pan_x: raw["pan_x"].as_f64().unwrap_or(16.0),
        pan_y: raw["pan_y"].as_f64().unwrap_or(16.0),
        plane_w: raw["plane_w"].as_f64().unwrap_or(0.0),
        plane_h: raw["plane_h"].as_f64().unwrap_or(0.0),
        fit_mode: raw["fit_mode"].as_bool().unwrap_or(true),
        needs_measure: raw["needs_measure"].as_bool().unwrap_or(false),
    }
}

pub(super) fn save_camera(surface: SysMapSurface, cam: &Camera) {
    m::set(
        surface.camera_key,
        json!({
            "scale": cam.scale,
            "pan_x": cam.pan_x,
            "pan_y": cam.pan_y,
            "plane_w": cam.plane_w,
            "plane_h": cam.plane_h,
            "fit_mode": cam.fit_mode,
            "needs_measure": cam.needs_measure,
        }),
    );
}

pub(super) fn viewport_size(canvas_id: &str) -> (f64, f64, bool) {
    graph_viewport::viewport_size(canvas_id, 40.0, (760.0, 380.0))
}

/// Returns the zoom LOD bucket (`"far"`, `"mid"`, or `"near"`) for CSS decluttering.
#[cfg_attr(not(test), allow(dead_code))]
pub fn lod(scale: f64) -> &'static str {
    graph_viewport::lod_with(scale, SYSTEM_MAP_LOD)
}

pub(super) fn radar_size(picture: (f64, f64)) -> (f64, f64) {
    let (pw, ph) = picture;
    if !(pw > 0.0 && ph > 0.0) {
        return (0.0, 0.0);
    }
    let scale = (RADAR_WIDTH / pw).min(RADAR_HEIGHT / ph);
    (
        (pw * scale).round().max(24.0),
        (ph * scale).round().max(18.0),
    )
}

/// Computes the visible rectangle in plane coordinates for the minimap viewport frame.
/// Returns `None` if the entire map already fits inside the viewport.
pub fn radar_window_rect(
    corner: (f64, f64),
    scale: f64,
    room: (f64, f64),
    picture: (f64, f64),
) -> Option<(f64, f64, f64, f64)> {
    graph_viewport::radar_window_rect(
        graph_viewport::Camera::new(corner.0, corner.1, scale),
        room,
        picture,
    )
}

/// Returns camera pan `(pan_x, pan_y)` that centers world `point` in `room` at `scale`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn centred(point: (f64, f64), scale: f64, room: (f64, f64)) -> (f64, f64) {
    let cam = graph_viewport::center_camera_on(point, room, scale);
    (cam.tx, cam.ty)
}

pub(super) fn sync_system_radar(surface: SysMapSurface, cam: &Camera) {
    let radar_id = format!("{}-radar", surface.canvas_id);
    let Some(radar) = dom::get(&radar_id) else {
        return;
    };
    let (vw, vh, measured) = viewport_size(surface.canvas_id);
    let seen = (measured && cam.plane_w > 0.0 && cam.plane_h > 0.0)
        .then(|| {
            radar_window_rect(
                (cam.pan_x, cam.pan_y),
                cam.scale,
                (vw, vh),
                (cam.plane_w, cam.plane_h),
            )
        })
        .flatten();
    let frame = radar.query_selector(".graph-radar-view").ok().flatten();
    let (Some(seen), Some(frame)) = (seen, frame) else {
        dom::hide(&radar, true);
        return;
    };
    for (k, v) in [
        ("x", seen.0),
        ("y", seen.1),
        ("width", seen.2),
        ("height", seen.3),
    ] {
        let _ = frame.set_attribute(k, &format!("{v:.1}"));
    }
    dom::hide(&radar, false);
}

pub(super) fn find_sys_node_el(canvas: &web_sys::Element, nid: &str) -> Option<web_sys::Element> {
    let escaped = crate::dom::escape_css_attr_value(nid);
    canvas
        .query_selector(&format!(".system-map-node[data-n=\"{escaped}\"]"))
        .ok()
        .flatten()
}

pub(super) fn find_sys_frame_el(
    canvas: &web_sys::Element,
    svc_id: &str,
) -> Option<web_sys::Element> {
    let escaped = crate::dom::escape_css_attr_value(svc_id);
    canvas
        .query_selector(&format!(".system-service-frame[data-of=\"{escaped}\"]"))
        .ok()
        .flatten()
}

pub(super) fn pan_to_keep_rect_in_view(
    cam: &mut Camera,
    rect: (f64, f64, f64, f64),
    viewport: (f64, f64),
    margin: f64,
) -> bool {
    let cur = graph_viewport::Camera {
        scale: cam.scale.max(0.05),
        tx: cam.pan_x,
        ty: cam.pan_y,
    };
    let Some(next) = graph_viewport::pan_to_keep_rect_in_view(cur, rect, viewport, margin) else {
        return false;
    };
    cam.pan_x = next.tx;
    cam.pan_y = next.ty;
    cam.fit_mode = false;
    true
}

pub(super) fn sys_nav_entry_from(state: &V, cam: &Camera) -> graph_viewport::CameraSelectionEntry {
    let focus = m::text(state, "focus");
    let edge = &state["selected_edge"];
    let target = json!({
        "focus": focus,
        "selected_edge": edge,
    })
    .to_string();
    let (key, label) = if edge.is_object() {
        let f = m::text(edge, "from");
        let t = m::text(edge, "to");
        (format!("edge:{f}->{t}"), format!("{f} → {t}"))
    } else {
        (format!("node:{focus}"), focus.clone())
    };
    graph_viewport::CameraSelectionEntry {
        key,
        label,
        target,
        cam: cam.viewport_cam(),
    }
}

pub(super) fn parse_sys_nav_entry(
    entry: &graph_viewport::CameraSelectionEntry,
) -> (String, V, f64, f64, f64) {
    let parsed: V = serde_json::from_str(&entry.target).unwrap_or(V::Null);
    let focus = parsed["focus"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| entry.key.strip_prefix("node:").unwrap_or("").to_owned());
    let selected_edge = if parsed["selected_edge"].is_object() {
        parsed["selected_edge"].clone()
    } else {
        V::Null
    };
    (
        focus,
        selected_edge,
        entry.cam.tx,
        entry.cam.ty,
        entry.cam.scale,
    )
}

thread_local! {
    pub(super) static SYS_WAVE_PLAYING: RefCell<HashSet<&'static str>> = RefCell::new(HashSet::new());
    pub(super) static SYS_HOVER_HIT: RefCell<HashMap<&'static str, Option<(u8, usize)>>> = RefCell::new(HashMap::new());
    pub(super) static SYS_NAV_HISTORY: RefCell<HashMap<&'static str, graph_viewport::CameraSelectionHistory>> =
        RefCell::new(HashMap::new());
}

pub(super) fn push_sys_nav_history(surface: SysMapSurface, state: &V, cam: &Camera) {
    let entry = sys_nav_entry_from(state, cam);
    SYS_NAV_HISTORY.with(|h| {
        h.borrow_mut()
            .entry(surface.id)
            .or_insert_with(|| graph_viewport::CameraSelectionHistory::new(32))
            .push(entry);
    });
}

pub(super) fn canvas_has_empty_plane(canvas_id: &str) -> bool {
    dom::get(canvas_id).is_some_and(|c| {
        c.query_selector(".graph-map-plane[data-empty=\"true\"]")
            .ok()
            .flatten()
            .is_some()
    })
}

pub(super) fn apply_camera(surface: SysMapSurface, cam: &Camera) {
    let zoom_id = format!("{}-zoom-level", surface.id_prefix);
    if canvas_has_empty_plane(surface.canvas_id) {
        dom::attr(surface.canvas_id, "data-scale", "1.0000");
        dom::attr(surface.canvas_id, "data-lod", "near");
        dom::text(&zoom_id, "100%");
        sync_system_radar(surface, cam);
        return;
    }
    let plane_size = (cam.plane_w.max(320.0), cam.plane_h.max(220.0));
    let _ = graph_viewport::apply_transform_3d_by_id(
        surface.canvas_id,
        Some(&zoom_id),
        cam.viewport_cam(),
        plane_size,
        SYSTEM_MAP_LOD,
    );
    if let Some(canvas) = dom::get(surface.canvas_id) {
        let scale_text = format!("{:.4}", cam.scale);
        let _ = canvas.set_attribute("data-scale", &scale_text);
        if let Ok(Some(plane)) = canvas.query_selector(".graph-map-plane") {
            let _ = plane.set_attribute("data-scale", &scale_text);
            let _ = plane.set_attribute(
                "data-lod",
                graph_viewport::lod_with(cam.scale, SYSTEM_MAP_LOD),
            );
        }
    }
    sync_system_radar(surface, cam);
    let is_dense = SYS_DENSE_CACHE.with(|c| {
        c.borrow()
            .get(surface.canvas_id)
            .is_some_and(|e| e.is_dense)
    });
    if is_dense {
        sync_system_viewport_culling(surface, cam);
        let hover = SYS_HOVER_HIT.with(|h| h.borrow().get(surface.canvas_id).copied().flatten());
        graph_viewport::canvas2d::paint_scene(surface.canvas_id, cam.viewport_cam(), hover, true);
    }
}

pub(super) fn fit_surface_camera(surface: SysMapSurface) {
    let mut cam = load_camera(surface);
    let (vw, vh, measured) = viewport_size(surface.canvas_id);
    cam.needs_measure = !measured;
    cam.fit_mode = true;
    if cam.plane_w > 0.0 && cam.plane_h > 0.0 {
        cam.apply_viewport_cam(graph_viewport::fit(
            (cam.plane_w, cam.plane_h),
            (vw, vh),
            graph_viewport::FitOptions::for_system_map(),
        ));
    }
    save_camera(surface, &cam);
    apply_camera(surface, &cam);
}

pub(super) fn reset_surface_camera_100(surface: SysMapSurface) {
    let mut cam = load_camera(surface);
    let (vw, vh, measured) = viewport_size(surface.canvas_id);
    cam.needs_measure = !measured;
    cam.apply_viewport_cam(graph_viewport::reset_100(
        (cam.plane_w, cam.plane_h),
        (vw, vh),
        16.0,
    ));
    cam.fit_mode = false;
    save_camera(surface, &cam);
    apply_camera(surface, &cam);
}

pub(super) fn zoom_surface_at(
    surface: SysMapSurface,
    factor: f64,
    client_anchor: Option<(f64, f64)>,
) {
    let mut cam = load_camera(surface);
    let (vw, vh, measured) = viewport_size(surface.canvas_id);
    cam.needs_measure = !measured;
    let anchor = if let Some((cx, cy)) = client_anchor {
        if let Some(canvas) = dom::get(surface.canvas_id) {
            let rect = canvas.get_bounding_client_rect();
            (
                (cx - rect.left()).clamp(0.0, vw),
                (cy - rect.top()).clamp(0.0, vh),
            )
        } else {
            (vw / 2.0, vh / 2.0)
        }
    } else {
        (vw / 2.0, vh / 2.0)
    };
    let next = graph_viewport::zoom_at(
        cam.viewport_cam(),
        anchor,
        cam.scale * factor,
        CAMERA_MIN,
        CAMERA_MAX,
    );
    cam.apply_viewport_cam(next);
    cam.fit_mode = false;
    save_camera(surface, &cam);
    apply_camera(surface, &cam);
}

pub(super) fn default_surface_state() -> V {
    json!({
        "project_id": "",
        "config_path": "",
        "focus": "",
        "selected_edge": null,
        "filter": "",
        "whole": false,
        "certain": false,
        "layers": false,
        "expanded": false,
        "spread": [],
        "side_tab": "details",
        "tool": "",
        "follow": "",
        "follow_dir": "out",
        "out": [],
        "into": [],
        "path_from": "",
        "path_to": "",
        "skip": [],
        "shared_a": "",
        "shared_b": "",
        "shared_side": "both_use",
        "shared_page": 0,
        "cycle_index": null,
        "kinds_off": [],
        "overlay": {},
    })
}

pub(super) fn surface_state(surface: SysMapSurface) -> V {
    let s = m::data(surface.state_key);
    if s.is_object() {
        s
    } else {
        default_surface_state()
    }
}

pub(super) fn save_surface_state(surface: SysMapSurface, state: &V) {
    m::set(surface.state_key, state.clone());
}

pub(super) fn node_id(node: &V) -> String {
    first_non_empty(node, &["id", "service", "path", "name"])
}

pub(super) fn service_display_name(svc: &V) -> String {
    let mut name = first_non_empty(svc, &["name", "title", "label"]);
    if name.is_empty() && svc["declared"].is_object() {
        name = first_non_empty(&svc["declared"], &["name", "title", "label"]);
    }
    if name.is_empty() {
        name = m::text(svc, "id");
    }
    if name.is_empty() {
        "Service".to_owned()
    } else {
        name
    }
}

pub(super) fn contract_name(c: &V) -> String {
    if let Some(s) = c.as_str() {
        return s.trim().to_owned();
    }
    first_non_empty(c, &["name", "contract", "id", "topic", "endpoint"])
}

pub(super) fn capability_name(cap: &V) -> String {
    if let Some(s) = cap.as_str() {
        return s.trim().to_owned();
    }
    first_non_empty(cap, &["name", "id", "label", "title"])
}

pub(super) fn endpoint_service_id(
    endpoint: &str,
    node_by_id: Option<&BTreeMap<String, &V>>,
) -> Option<String> {
    let ep = endpoint.trim();
    if ep.is_empty() || ep.starts_with("__unmatched__:") {
        return None;
    }
    if let Some(node) = node_by_id.and_then(|map| map.get(ep)) {
        let owned = first_non_empty(node, &["parent_service", "parent", "service"]);
        if !owned.is_empty() && !owned.starts_with("__unmatched__:") {
            return Some(owned);
        }
        if matches!(node["kind"].as_str(), Some("service" | "system") | None) {
            return Some(ep.to_owned());
        }
    }
    if ep.starts_with("contract:") || ep.starts_with("capability:") || ep.starts_with("source:") {
        return ep
            .split(':')
            .nth(1)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
    }
    ep.split("::")
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with("__unmatched__:"))
        .map(str::to_owned)
}

pub(super) fn explain_services_for_endpoints(
    from: &str,
    to: &str,
    node_by_id: Option<&BTreeMap<String, &V>>,
) -> String {
    let mut svcs = Vec::new();
    for ep in [from, to] {
        if let Some(sid) = endpoint_service_id(ep, node_by_id) {
            if !svcs.iter().any(|s| s == &sid) {
                svcs.push(sid);
            }
        }
    }
    svcs.join(",")
}

pub(super) fn first_non_empty(v: &V, keys: &[&str]) -> String {
    for k in keys {
        if let Some(s) = v[*k].as_str() {
            let t = s.trim();
            if !t.is_empty() {
                return t.to_owned();
            }
        }
    }
    String::new()
}

pub(super) fn push_unique_str(arr_val: &mut V, item: &str) {
    if !arr_val.is_array() {
        *arr_val = json!([]);
    }
    let arr = arr_val.as_array_mut().expect("array");
    if !arr.iter().any(|x| x.as_str() == Some(item)) {
        arr.push(json!(item));
    }
}

pub(super) fn push_contract_obj(arr_val: &mut V, obj: V) {
    if !arr_val.is_array() {
        *arr_val = json!([]);
    }
    let cname = m::text(&obj, "name");
    let arr = arr_val.as_array_mut().expect("array");
    if cname.is_empty() || !arr.iter().any(|x| m::text(x, "name") == cname) {
        arr.push(obj);
    }
}

pub(super) fn sys_pin_key(surface: SysMapSurface, state: &V) -> String {
    let proj = m::text(state, "project_id");
    let mode = if m::yes(&state["layers"]) {
        "layers"
    } else if m::yes(&state["whole"]) {
        "whole"
    } else {
        "default"
    };
    graph_viewport::pin_storage_key(surface.id_prefix, &proj, mode)
}

pub(super) fn set_html(id: &str, content: &str) {
    if let Some(el) = dom::get(id) {
        el.set_inner_html(content);
    }
}

pub(super) fn sync_toolbar_buttons(surface: SysMapSurface, state: &V) {
    let p = surface.id_prefix;
    dom::attr(
        &format!("{p}-unfold"),
        "aria-pressed",
        if m::yes(&state["whole"]) {
            "true"
        } else {
            "false"
        },
    );
    dom::attr(
        &format!("{p}-certain"),
        "aria-pressed",
        if m::yes(&state["certain"]) {
            "true"
        } else {
            "false"
        },
    );
    dom::attr(
        &format!("{p}-layers"),
        "aria-pressed",
        if m::yes(&state["layers"]) {
            "true"
        } else {
            "false"
        },
    );
    dom::attr(
        &format!("{p}-expand"),
        "aria-expanded",
        if m::yes(&state["expanded"]) {
            "true"
        } else {
            "false"
        },
    );
    dom::attr(
        surface.workspace_id,
        "data-expanded",
        if m::yes(&state["expanded"]) {
            "true"
        } else {
            "false"
        },
    );
    dom::hidden(
        &format!("{p}-reset-pins"),
        !graph_viewport::has_pins(&sys_pin_key(surface, state)),
    );
    dom::attr(
        &format!("{p}-isolate-toggle"),
        "aria-pressed",
        if m::yes(&state["isolate_active"]) {
            "true"
        } else {
            "false"
        },
    );
    let has_path = m::text(state, "tool") == "path"
        && (!m::text(state, "path_from").is_empty() || !m::text(state, "path_to").is_empty());
    dom::hidden(&format!("{p}-path-clear"), !has_path);
    dom::set_value(&format!("{p}-path-from"), &m::text(state, "path_from"));
    dom::set_value(&format!("{p}-path-to"), &m::text(state, "path_to"));
    let depth_str = state["isolate_depth"]
        .as_u64()
        .filter(|&d| d > 0)
        .map(|d| d.to_string())
        .unwrap_or_else(|| "1".to_owned());
    dom::set_value(&format!("{p}-isolate-depth"), &depth_str);
}

mod inspector;
mod interactions;
mod layout;
mod model;
mod render;
#[cfg(test)]
mod tests;

#[allow(unused_imports)]
use inspector::*;
#[allow(unused_imports)]
use interactions::*;
#[allow(unused_imports)]
use layout::*;
#[allow(unused_imports)]
use model::*;
#[allow(unused_imports)]
use render::*;

#[allow(unused_imports)]
pub use inspector::{bridge_html, render_side_html};
#[allow(unused_imports)]
pub use interactions::{
    attach_wheel_listeners, current_context_brief, event, focus_service_changes,
    focus_service_topology, key, pointer_event, refresh_changes_map, refresh_services_map,
    should_ignore_canvas_pointer,
};
#[allow(unused_imports)]
pub use layout::{
    layout_system_nodes, layout_system_nodes_with_pins, PlacedSysFrame, PlacedSysNode,
    PlacedSysWire, SysLayout,
};
#[allow(unused_imports)]
pub use model::{
    assign_service_layers, build_context_brief, compute_blast_radius, find_cross_service_cycles,
    normalize_map_doc, shared_neighbors, shortest_service_paths, synthesize_map_from_catalog,
    system_structure_digest, BlastRadius,
};
#[allow(unused_imports)]
pub(crate) use render::mark_failed_services;
#[allow(unused_imports)]
pub use render::{
    contract_arrow_legend_html, contract_arrow_legend_html_for, render_canvas_html,
    render_canvas_with_radar, system_radar_html, wave_scrubber_html,
};
