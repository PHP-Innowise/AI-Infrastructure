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

const NODE_WIDTH: f64 = 196.0;
const NODE_HEIGHT: f64 = 58.0;
const CHILD_WIDTH: f64 = 164.0;
const CHILD_HEIGHT: f64 = 30.0;
const COL_GAP: f64 = 96.0;
const ROW_GAP: f64 = 20.0;
const MARGIN: f64 = 24.0;
const FRAME_HEAD: f64 = 28.0;
const FRAME_PAD: f64 = 12.0;

const CAMERA_MIN: f64 = graph_viewport::DEFAULT_MIN_SCALE;
const CAMERA_MAX: f64 = 2.4;
const ZOOM_STEP: f64 = 1.25;
const LOD_FAR: f64 = 0.55;
const LOD_NEAR: f64 = 1.25;
const SYSTEM_MAP_LOD: graph_viewport::LodThresholds = graph_viewport::LodThresholds {
    far_below: LOD_FAR,
    near_at: LOD_NEAR,
    near_inclusive: true,
    invalid_fallback: "far",
};
const RADAR_WIDTH: f64 = 176.0;
const RADAR_HEIGHT: f64 = 112.0;
const RADAR_DRAG_KEY: &str = "system-map-radar-drag";
const SYS_PORT_GAP: f64 = 6.0;
const SYS_PORT_SPREAD: f64 = 18.0;

#[derive(Clone, Copy)]
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

fn surface_by_id(id: &str) -> SysMapSurface {
    if id == CHANGES_SURFACE_ID || id.starts_with("system-change-topology") {
        CHANGES_SURFACE
    } else {
        SERVICES_SURFACE
    }
}

fn surface_for_canvas(canvas_id: &str) -> Option<SysMapSurface> {
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

fn spec_for_kind(kind: &str) -> &'static ContractArrowSpec {
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

fn escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn payload_attr(value: &V) -> String {
    escape(&value.to_string())
}

fn contract_arrow_defs_svg(id_prefix: &str) -> String {
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

fn contract_spec_index(kind: &str) -> usize {
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

fn write_hidden_contract_kinds(mut state: V, hidden: [bool; 7]) -> V {
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
pub fn contract_arrow_legend_html_for(surface: SysMapSurface, doc: &V, state: &V) -> String {
    let hidden = hidden_contract_kinds(state);
    let counts = contract_kind_counts(doc);
    let has_counts = counts.iter().any(|c| *c > 0);
    let any_hidden = hidden.iter().any(|h| *h);
    let mut html = String::from(
        "<div class=\"graph-arrow-legend system-arrow-legend\" role=\"list\" aria-label=\"Cross-service contract and dependency arrow types\"><span class=\"graph-arrow-legend-title\">Links:</span>",
    );
    for (idx, spec) in CONTRACT_ARROW_SPECS.iter().enumerate() {
        let shown = !hidden[idx];
        let count = counts[idx];
        let dash = spec
            .dash_array
            .map(|d| format!(" stroke-dasharray=\"{d}\""))
            .unwrap_or_default();
        let payload = payload_attr(&json!({
            "surface": surface.id,
            "kind": spec.kind,
        }));
        let title = if has_counts {
            format!(
                "{} · {} links (click to {}, Alt+click to isolate)",
                spec.description,
                count,
                if shown { "hide" } else { "show" }
            )
        } else {
            format!(
                "{} (click to {}, Alt+click to isolate)",
                spec.description,
                if shown { "hide" } else { "show" }
            )
        };
        let count_html = if count > 0 {
            format!("<span class=\"graph-arrow-legend-count\">{count}</span>")
        } else {
            String::new()
        };
        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-arrow-legend-item\" role=\"listitem\" data-action=\"system-map-kind-toggle\" data-surface=\"{}\" data-payload=\"{payload}\" data-sys-edge-kind=\"{}\" aria-pressed=\"{shown}\" title=\"{}\"><svg class=\"graph-arrow-swatch system-arrow-swatch\" data-sys-edge-kind=\"{}\" viewBox=\"0 0 30 12\" width=\"30\" height=\"12\" aria-hidden=\"true\"><line class=\"graph-arrow-swatch-line\" x1=\"2\" y1=\"6\" x2=\"{:.1}\" y2=\"6\"{dash}/><g transform=\"translate(20 2)\" class=\"system-map-arrow system-map-arrow-{}\">{}</g></svg><span>{}</span>{count_html}</button>",
            surface.id,
            spec.kind,
            escape(&title),
            spec.kind,
            spec.swatch_line_end,
            spec.kind,
            spec.marker_body,
            spec.label,
        ));
    }
    if any_hidden {
        let all_payload = payload_attr(&json!({
            "surface": surface.id,
            "preset": "all",
        }));
        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-arrow-legend-preset\" data-action=\"system-map-kind-preset\" data-surface=\"{}\" data-payload=\"{all_payload}\" title=\"Show all contract and code link types\">Show all</button>",
            surface.id,
        ));
    }
    html.push_str(
        "<span class=\"graph-arrow-legend-item system-legend-risk\" role=\"listitem\" title=\"Unmatched consumer contract or incompatible contract version\"><span class=\"system-legend-halo system-legend-halo-broken\" aria-hidden=\"true\"></span><span>Broken risk</span></span>",
    );
    html.push_str(
        "<span class=\"graph-arrow-legend-item system-legend-drift\" role=\"listitem\" title=\"Code imports or calls cross service boundaries without a declared contract\"><span class=\"system-legend-halo system-legend-halo-drift\" aria-hidden=\"true\"></span><span>Undeclared drift</span></span>",
    );
    html.push_str(
        "<span class=\"graph-arrow-legend-item system-legend-against\" role=\"listitem\" title=\"Upward dependency against service tier layering or closing a cross-service cycle (↻)\"><span class=\"system-legend-halo system-legend-halo-against\" aria-hidden=\"true\"></span><span>Layer violation ↻</span></span>",
    );
    html.push_str("</div>");
    html
}

/// Accessible legend of all 7 cross-service contract & code arrow kinds plus drift/risk halos.
#[cfg_attr(not(test), allow(dead_code))]
pub fn contract_arrow_legend_html() -> String {
    contract_arrow_legend_html_for(SERVICES_SURFACE, &V::Null, &default_surface_state())
}

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

fn load_camera(surface: SysMapSurface) -> Camera {
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

fn save_camera(surface: SysMapSurface, cam: &Camera) {
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

fn viewport_size(canvas_id: &str) -> (f64, f64, bool) {
    graph_viewport::viewport_size(canvas_id, 40.0, (760.0, 380.0))
}

/// Returns the zoom LOD bucket (`"far"`, `"mid"`, or `"near"`) for CSS decluttering.
#[cfg_attr(not(test), allow(dead_code))]
pub fn lod(scale: f64) -> &'static str {
    graph_viewport::lod_with(scale, SYSTEM_MAP_LOD)
}

fn radar_size(picture: (f64, f64)) -> (f64, f64) {
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

fn sync_system_radar(surface: SysMapSurface, cam: &Camera) {
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

fn apply_camera(surface: SysMapSurface, cam: &Camera) {
    let zoom_id = format!("{}-zoom-level", surface.id_prefix);
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
    sync_system_viewport_culling(surface, cam);
    graph_viewport::canvas2d::paint_scene(surface.canvas_id, cam.viewport_cam(), None, true);
}

fn fit_surface_camera(surface: SysMapSurface) {
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

fn reset_surface_camera_100(surface: SysMapSurface) {
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

fn zoom_surface_at(surface: SysMapSurface, factor: f64, client_anchor: Option<(f64, f64)>) {
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

fn default_surface_state() -> V {
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

fn surface_state(surface: SysMapSurface) -> V {
    let s = m::data(surface.state_key);
    if s.is_object() {
        s
    } else {
        default_surface_state()
    }
}

fn save_surface_state(surface: SysMapSurface, state: &V) {
    m::set(surface.state_key, state.clone());
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlacedSysNode {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub col: usize,
    pub cycle: bool,
    pub child_of: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlacedSysFrame {
    pub service_id: String,
    pub title: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlacedSysWire {
    pub edge_index: usize,
    pub path_d: String,
    pub mid_x: f64,
    pub mid_y: f64,
    pub back: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SysLayout {
    pub nodes: Vec<PlacedSysNode>,
    pub frames: Vec<PlacedSysFrame>,
    pub wires: Vec<PlacedSysWire>,
    pub tier_bands: Vec<(String, f64, f64, f64)>,
    pub width: f64,
    pub height: f64,
}

/// Synthesizes a normalized cross-service map document from `/api/systems/catalog`
/// so the map renders immediately and gracefully merges with `/api/systems/graph/map`.
pub fn synthesize_map_from_catalog(catalog: &V, graph_status: &V) -> V {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut provider_by_contract: BTreeMap<String, Vec<String>> = BTreeMap::new();

    let services = m::array(&catalog["services"]);
    let status_services = &graph_status["services"];

    for svc in &services {
        let id = m::text(svc, "id");
        if id.is_empty() {
            continue;
        }
        let declared = &svc["declared"];
        let svc_provides = if svc["provides"].is_array() {
            &svc["provides"]
        } else {
            &declared["provides"]
        };
        let svc_consumes = if svc["consumes"].is_array() {
            &svc["consumes"]
        } else {
            &declared["consumes"]
        };
        let svc_capabilities = if svc["capabilities"].is_array() {
            &svc["capabilities"]
        } else {
            &declared["capabilities"]
        };
        for c in m::array(svc_provides) {
            let cname = contract_name(&c);
            if !cname.is_empty() {
                provider_by_contract
                    .entry(cname)
                    .or_default()
                    .push(id.clone());
            }
        }
        let g_status = if status_services.is_object() {
            status_services[&id].clone()
        } else {
            V::Null
        };
        let role = {
            let r = m::text(svc, "role");
            if r.is_empty() {
                m::text(declared, "role")
            } else {
                r
            }
        };
        let owner = {
            let o = m::text(svc, "owner");
            if o.is_empty() {
                m::text(declared, "owner")
            } else {
                o
            }
        };
        let description = {
            let d = m::text(svc, "description");
            if d.is_empty() {
                m::text(declared, "description")
            } else {
                d
            }
        };
        let language = {
            let l = first_non_empty(svc, &["language", "stack"]);
            if l.is_empty() {
                first_non_empty(declared, &["language", "stack"])
            } else {
                l
            }
        };
        nodes.push(json!({
            "id": id,
            "path": id,
            "service": id,
            "kind": "service",
            "name": service_display_name(svc),
            "role": role,
            "owner": owner,
            "description": description,
            "folder": m::text(svc, "root").chars().next().map(|_| m::text(svc, "root")).unwrap_or_else(|| m::text(svc, "path")),
            "language": language,
            "capabilities": svc_capabilities,
            "provides": svc_provides,
            "consumes": svc_consumes,
            "memory": svc["memory"],
            "brain": svc["brain"],
            "git": svc["git"],
            "graph_status": g_status,
            "warnings": svc["warnings"],
        }));
    }

    let shared_sources = m::array(&catalog["shared_sources"]);
    let has_system_graph = status_services.is_object() && status_services["__system__"].is_object();
    let sys_str_name = catalog["system"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    if !shared_sources.is_empty()
        || has_system_graph
        || catalog["system"].is_object()
        || sys_str_name.is_some()
    {
        let sys_name = {
            let n = sys_str_name.unwrap_or_else(|| m::text(&catalog["system"], "name"));
            if n.is_empty() {
                "System shared context".to_owned()
            } else {
                format!("{n} (shared)")
            }
        };
        let sys_folder = if catalog["system"].is_object() {
            m::text(&catalog["system"], "root")
        } else {
            String::new()
        };
        nodes.push(json!({
            "id": "__system__",
            "path": "__system__",
            "service": "__system__",
            "kind": "system",
            "name": sys_name,
            "role": "shared-authority",
            "folder": sys_folder,
            "shared_sources": shared_sources,
            "graph_status": if status_services.is_object() { status_services["__system__"].clone() } else { V::Null },
        }));
    }

    let rels = m::array(&catalog["relationships"]);
    let mut pair_edges: BTreeMap<(String, String), V> = BTreeMap::new();

    if !rels.is_empty() {
        for rel in &rels {
            let from = first_non_empty(rel, &["consumer", "from", "source"]);
            let to = first_non_empty(rel, &["provider", "to", "target"]);
            if from.is_empty() || to.is_empty() || from == to {
                continue;
            }
            let cname = first_non_empty(rel, &["contract", "name", "id"]);
            let kind = normalize_contract_arrow_kind(&first_non_empty(rel, &["kind", "type"]));
            let broken = m::yes(&rel["unmatched"])
                || m::yes(&rel["broken_risk"])
                || m::yes(&rel["version_mismatch"]);
            let entry = pair_edges
                .entry((from.clone(), to.clone()))
                .or_insert_with(|| {
                    json!({
                        "from": from,
                        "to": to,
                        "kind": kind,
                        "kinds": [],
                        "contracts": [],
                        "provenance": "declared",
                        "broken_risk": false,
                        "against": false,
                    })
                });
            if broken {
                entry["broken_risk"] = json!(true);
            }
            push_unique_str(&mut entry["kinds"], kind);
            if !cname.is_empty() {
                push_contract_obj(
                    &mut entry["contracts"],
                    json!({
                        "name": cname,
                        "kind": kind,
                        "version": m::text(rel, "version"),
                        "broken_risk": broken,
                    }),
                );
            }
        }
    } else {
        for svc in &services {
            let consumer = m::text(svc, "id");
            if consumer.is_empty() {
                continue;
            }
            let svc_consumes = if svc["consumes"].is_array() {
                &svc["consumes"]
            } else {
                &svc["declared"]["consumes"]
            };
            for c in m::array(svc_consumes) {
                let raw = contract_name(&c);
                if raw.is_empty() {
                    continue;
                }
                let explicit_svc = first_non_empty(&c, &["service", "provider"]);
                let (explicit_prov, cname) = if !explicit_svc.is_empty() {
                    (Some(explicit_svc), raw.clone())
                } else if let Some((p, n)) = raw.split_once(':') {
                    (Some(p.trim().to_owned()), n.trim().to_owned())
                } else {
                    (None, raw.clone())
                };
                let kind = normalize_contract_arrow_kind(&m::text(&c, "kind"));
                let providers = if let Some(p) = explicit_prov {
                    vec![p]
                } else {
                    provider_by_contract
                        .get(&cname)
                        .cloned()
                        .unwrap_or_default()
                };
                if providers.is_empty() {
                    let target = if nodes.iter().any(|n| n["id"] == "__system__") {
                        "__system__".to_owned()
                    } else {
                        continue;
                    };
                    let entry = pair_edges
                        .entry((consumer.clone(), target.clone()))
                        .or_insert_with(|| {
                            json!({
                                "from": consumer,
                                "to": target,
                                "kind": kind,
                                "kinds": [kind],
                                "contracts": [],
                                "provenance": "declared",
                                "broken_risk": true,
                                "against": false,
                            })
                        });
                    entry["broken_risk"] = json!(true);
                    push_contract_obj(
                        &mut entry["contracts"],
                        json!({
                            "name": cname,
                            "kind": kind,
                            "unmatched": true,
                            "broken_risk": true,
                        }),
                    );
                } else {
                    for prov in providers {
                        if prov == consumer {
                            continue;
                        }
                        let entry = pair_edges
                            .entry((consumer.clone(), prov.clone()))
                            .or_insert_with(|| {
                                json!({
                                    "from": consumer,
                                    "to": prov,
                                    "kind": kind,
                                    "kinds": [],
                                    "contracts": [],
                                    "provenance": "declared",
                                    "broken_risk": false,
                                    "against": false,
                                })
                            });
                        push_unique_str(&mut entry["kinds"], kind);
                        push_contract_obj(
                            &mut entry["contracts"],
                            json!({
                                "name": cname,
                                "kind": kind,
                                "version": m::text(&c, "version"),
                            }),
                        );
                    }
                }
            }
        }
    }

    if !shared_sources.is_empty() && nodes.iter().any(|n| n["id"] == "__system__") {
        for svc in &services {
            let sid = m::text(svc, "id");
            if sid.is_empty() {
                continue;
            }
            if svc["shared_sources"]
                .as_array()
                .is_some_and(|arr| !arr.is_empty())
            {
                let entry = pair_edges
                    .entry((sid.clone(), "__system__".to_owned()))
                    .or_insert_with(|| {
                        json!({
                            "from": sid,
                            "to": "__system__",
                            "kind": "shared",
                            "kinds": ["shared"],
                            "contracts": [],
                            "provenance": "declared",
                            "broken_risk": false,
                            "against": false,
                        })
                    });
                push_unique_str(&mut entry["kinds"], "shared");
            }
        }
    }

    edges.extend(pair_edges.into_values());
    json!({
        "nodes": nodes,
        "edges": edges,
        "synthesized": true,
    })
}

/// Normalizes a server `/api/systems/graph/map` document and enriches it with
/// catalog metadata + live service graph statuses.
pub fn normalize_map_doc(raw: &V, catalog: &V, graph_status: &V, state: &V) -> V {
    let base = synthesize_map_from_catalog(catalog, graph_status);
    if !raw.is_object() || (raw["nodes"].is_null() && raw["services"].is_null()) {
        return expand_unfolded_nodes(&base, state);
    }

    let mut cat_by_id = BTreeMap::new();
    for node in m::array(&base["nodes"]) {
        let id = node_id(&node);
        if !id.is_empty() {
            cat_by_id.insert(id, node);
        }
    }

    let raw_nodes = if raw["nodes"].is_array() {
        m::array(&raw["nodes"])
    } else {
        m::array(&raw["services"])
    };

    let status_services = &graph_status["services"];
    let mut nodes = Vec::new();
    let mut seen = BTreeSet::new();

    for mut n in raw_nodes {
        let id = node_id(&n);
        if id.is_empty() || !seen.insert(id.clone()) {
            continue;
        }
        if n["name"].is_null() || n["name"] == "" {
            let lbl = m::text(&n, "label");
            if !lbl.is_empty() {
                n["name"] = json!(lbl);
            }
        }
        if n["language"].is_null() || n["language"] == "" {
            let stack = m::text(&n, "stack");
            if !stack.is_empty() {
                n["language"] = json!(stack);
            }
        }
        if n["folder"].is_null() || n["folder"] == "" {
            let root = m::text(&n, "root");
            if !root.is_empty() {
                n["folder"] = json!(root);
            }
        }
        if n["parent_service"].is_null() || n["parent_service"] == "" {
            let parent = m::text(&n, "parent");
            if !parent.is_empty() {
                n["parent_service"] = json!(parent);
            }
        }
        if n["graph_status"].is_null() && n["code_graph"].is_object() {
            n["graph_status"] = n["code_graph"].clone();
        }
        if let Some(cat) = cat_by_id.get(&id) {
            for field in [
                "name",
                "role",
                "owner",
                "description",
                "folder",
                "language",
                "capabilities",
                "provides",
                "consumes",
                "memory",
                "brain",
                "git",
                "shared_sources",
                "warnings",
            ] {
                if n[field].is_null() || n[field] == "" {
                    n[field] = cat[field].clone();
                }
            }
        }
        if status_services.is_object() && status_services[&id].is_object() {
            n["graph_status"] = status_services[&id].clone();
        }
        if n["id"].is_null() {
            n["id"] = json!(id.clone());
        }
        if n["path"].is_null() {
            n["path"] = json!(id.clone());
        }
        if n["kind"].is_null() {
            n["kind"] = json!(if id == "__system__" {
                "system"
            } else {
                "service"
            });
        }
        nodes.push(n);
    }

    for (id, cat_node) in cat_by_id {
        if !seen.contains(&id) && raw["filtered"].as_bool() != Some(true) {
            nodes.push(cat_node);
        }
    }

    let raw_edges = if raw["edges"].is_array() {
        m::array(&raw["edges"])
    } else if raw["links"].is_array() {
        m::array(&raw["links"])
    } else {
        m::array(&raw["relationships"])
    };

    let mut edges = Vec::new();
    for mut e in raw_edges {
        let from = first_non_empty(&e, &["from", "consumer", "source"]);
        let to = first_non_empty(&e, &["to", "provider", "target"]);
        if from.is_empty() || to.is_empty() {
            continue;
        }
        e["from"] = json!(from);
        e["to"] = json!(to);
        if e["level"].as_str() == Some("detail") {
            e["child_link"] = json!(true);
        }
        let pkind = primary_edge_kind(&e);
        e["kind"] = json!(pkind);
        let prov = first_non_empty(&e, &["provenance", "verification", "status"]);
        if !prov.is_empty() {
            e["provenance"] = json!(prov);
        } else if m::yes(&e["undeclared"]) || m::yes(&e["undeclared_in_code"]) {
            e["provenance"] = json!("undeclared_in_code");
        } else if m::yes(&e["verified"]) || m::yes(&e["confirmed"]) {
            e["provenance"] = json!("verified");
        } else {
            e["provenance"] = json!("declared");
        }
        let broken = m::yes(&e["broken_risk"])
            || m::yes(&e["unmatched"])
            || m::yes(&e["version_mismatch"])
            || m::yes(&e["drift"]);
        e["broken_risk"] = json!(broken);
        let against = m::yes(&e["against"]) || m::yes(&e["violation"]);
        e["against"] = json!(against);
        edges.push(e);
    }

    if edges.is_empty() && !raw["edges"].is_array() {
        edges = m::array(&base["edges"]);
    }

    for risk in m::array(&raw["contract_risks"]) {
        let consumer = first_non_empty(&risk, &["consumer", "from", "service"]);
        let provider = first_non_empty(&risk, &["provider", "to"]);
        let cname = first_non_empty(&risk, &["contract", "name", "id"]);
        let rkind = normalize_contract_arrow_kind(&m::text(&risk, "kind"));
        if consumer.is_empty() {
            continue;
        }
        if !provider.is_empty() {
            let mut matched = false;
            for e in &mut edges {
                if e["from"].as_str() == Some(&consumer) && e["to"].as_str() == Some(&provider) {
                    e["broken_risk"] = json!(true);
                    matched = true;
                }
            }
            if !matched {
                edges.push(json!({
                    "from": consumer,
                    "to": provider,
                    "kind": rkind,
                    "kinds": [rkind],
                    "contracts": [{ "name": cname, "kind": rkind }],
                    "provenance": "declared",
                    "broken_risk": true,
                    "against": false,
                }));
            }
        } else if !cname.is_empty() {
            let un_id = format!("__unmatched__:{cname}");
            if !nodes.iter().any(|n| node_id(n) == un_id) {
                nodes.push(json!({
                    "id": un_id,
                    "path": un_id,
                    "kind": "contract",
                    "name": format!("? {cname}"),
                    "unmatched": true,
                }));
            }
            if !edges
                .iter()
                .any(|e| e["from"].as_str() == Some(&consumer) && e["to"].as_str() == Some(&un_id))
            {
                edges.push(json!({
                    "from": consumer,
                    "to": un_id,
                    "kind": rkind,
                    "kinds": [rkind],
                    "contracts": [{ "name": cname, "kind": rkind }],
                    "provenance": "declared",
                    "broken_risk": true,
                    "against": false,
                }));
            }
        }
    }

    let mut out = raw.clone();
    out["nodes"] = json!(nodes);
    out["edges"] = json!(edges);
    expand_unfolded_nodes(&out, state)
}

fn expand_unfolded_nodes(doc: &V, state: &V) -> V {
    let whole = m::yes(&state["whole"]);
    let spread_set: BTreeSet<String> = m::array(&state["spread"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    if !whole && spread_set.is_empty() {
        return doc.clone();
    }

    let base_nodes = m::array(&doc["nodes"]);
    // If server already emitted child contract/capability/source nodes, keep them.
    if base_nodes.iter().any(|n| {
        matches!(
            n["kind"].as_str(),
            Some("contract" | "capability" | "source")
        )
    }) {
        return doc.clone();
    }

    let mut nodes = base_nodes.clone();
    let mut edges = m::array(&doc["edges"]);
    let mut contract_node_by_key: BTreeMap<(String, String), String> = BTreeMap::new();

    for svc in &base_nodes {
        let sid = node_id(svc);
        if sid.is_empty() || (!whole && !spread_set.contains(&sid)) {
            continue;
        }
        for cap in m::array(&svc["capabilities"]).into_iter().take(6) {
            let cname = capability_name(&cap);
            if cname.trim().is_empty() {
                continue;
            }
            let cid = format!("{sid}::cap::{cname}");
            nodes.push(json!({
                "id": cid,
                "path": cid,
                "service": sid,
                "parent_service": sid,
                "kind": "capability",
                "name": cname,
            }));
        }
        for c in m::array(&svc["provides"]).into_iter().take(8) {
            let cname = contract_name(&c);
            if cname.is_empty() {
                continue;
            }
            let ckind = normalize_contract_arrow_kind(&m::text(&c, "kind"));
            let cid = format!("{sid}::contract::{cname}");
            contract_node_by_key.insert((sid.clone(), cname.clone()), cid.clone());
            nodes.push(json!({
                "id": cid,
                "path": cid,
                "service": sid,
                "parent_service": sid,
                "kind": "contract",
                "contract_kind": ckind,
                "name": cname,
                "version": m::text(&c, "version"),
                "source": first_non_empty(&c, &["source", "schema", "path"]),
            }));
        }
        for src in m::array(&svc["shared_sources"]).into_iter().take(6) {
            let spath = src
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| m::text(&src, "path"));
            if spath.trim().is_empty() {
                continue;
            }
            let cid = format!("{sid}::source::{spath}");
            nodes.push(json!({
                "id": cid,
                "path": spath,
                "service": sid,
                "parent_service": sid,
                "kind": "source",
                "name": spath.rsplit('/').next().unwrap_or(&spath),
            }));
        }
    }

    for edge in m::array(&doc["edges"]) {
        let from = m::text(&edge, "from");
        let to = m::text(&edge, "to");
        for c in m::array(&edge["contracts"]) {
            let cname = contract_name(&c);
            if let Some(cid) = contract_node_by_key.get(&(to.clone(), cname.clone())) {
                edges.push(json!({
                    "from": from,
                    "to": cid,
                    "kind": normalize_contract_arrow_kind(&m::text(&c, "kind")),
                    "contracts": [c],
                    "provenance": edge["provenance"],
                    "broken_risk": edge["broken_risk"],
                    "against": edge["against"],
                    "child_link": true,
                }));
            }
        }
    }

    let mut out = doc.clone();
    out["nodes"] = json!(nodes);
    out["edges"] = json!(edges);
    out
}

/// Computes cross-service blast radius from `seed_services` + `changed_contracts` up to `depth` hops.
/// In our graph convention, `edge.from` consumes `edge.to` (`consumer -> provider`).
/// Therefore when a provider `P` changes, downstream impacted consumers are nodes `C` with an edge `C -> P`!
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlastRadius {
    pub seeds: BTreeSet<String>,
    pub direct_consumers: BTreeSet<String>,
    pub downstream_consumers: BTreeSet<String>,
    pub matched_contracts: BTreeSet<String>,
    pub wave_by_service: BTreeMap<String, usize>,
}

pub fn compute_blast_radius(
    doc: &V,
    seed_services: &[String],
    changed_contracts: &[String],
    depth: usize,
) -> BlastRadius {
    let mut result = BlastRadius::default();
    let nodes = m::array(&doc["nodes"]);
    let edges = m::array(&doc["edges"]);

    for s in seed_services {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            result.seeds.insert(trimmed.to_owned());
        }
    }

    for raw_c in changed_contracts {
        let token = raw_c.trim();
        if token.is_empty() {
            continue;
        }
        let (prov_opt, cname) = if let Some((p, c)) = token.split_once(':') {
            (Some(p.trim()), c.trim())
        } else {
            (None, token)
        };
        if !cname.is_empty() {
            result.matched_contracts.insert(cname.to_owned());
        }
        if let Some(p) = prov_opt.filter(|p| !p.is_empty()) {
            result.seeds.insert(p.to_owned());
        } else {
            for node in &nodes {
                let sid = node_id(node);
                if m::array(&node["provides"])
                    .iter()
                    .any(|p| contract_name(p).eq_ignore_ascii_case(cname))
                {
                    result.seeds.insert(sid);
                }
            }
        }
    }

    // Build provider -> consumers adjacency (reverse of consumer -> provider edges).
    let mut consumers_of: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for edge in &edges {
        if m::yes(&edge["child_link"]) {
            continue;
        }
        let consumer = m::text(edge, "from");
        let provider = m::text(edge, "to");
        if !consumer.is_empty() && !provider.is_empty() && consumer != "__system__" {
            consumers_of.entry(provider).or_default().push(consumer);
        }
    }

    let mut queue = VecDeque::new();
    for seed in &result.seeds {
        result.wave_by_service.insert(seed.clone(), 0);
        queue.push_back((seed.clone(), 0usize));
    }

    while let Some((curr, dist)) = queue.pop_front() {
        if dist >= depth {
            continue;
        }
        if let Some(list) = consumers_of.get(&curr) {
            for next in list {
                let next_dist = dist + 1;
                if !result.wave_by_service.contains_key(next) {
                    result.wave_by_service.insert(next.clone(), next_dist);
                    if next_dist == 1 {
                        result.direct_consumers.insert(next.clone());
                    } else {
                        result.downstream_consumers.insert(next.clone());
                    }
                    queue.push_back((next.clone(), next_dist));
                }
            }
        }
    }

    result
}

/// The waves of the change a picture shows, as its overlay asks for them.
fn blast_of(doc: &V, state: &V) -> BlastRadius {
    let overlay = &state["overlay"];
    let texts = |key: &str| {
        m::array(&overlay[key])
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect::<Vec<_>>()
    };
    compute_blast_radius(
        doc,
        &texts("seed_services"),
        &texts("changed_contracts"),
        overlay["depth"].as_u64().unwrap_or(32) as usize,
    )
}
/// How many waves a change takes to reach every service it reaches.
fn last_wave(blast: &BlastRadius) -> usize {
    blast.wave_by_service.values().copied().max().unwrap_or(0)
}
/// Steps through the waves of a change: all of them, or the change as far
/// as one wave, the services a later wave reaches left faint; and a button
/// that plays them one after another. Nothing where the change reaches no
/// service past what it starts in.
pub fn wave_scrubber_html(surface: SysMapSurface, blast: &BlastRadius, state: &V) -> String {
    let last = last_wave(blast);
    if last == 0 {
        return String::new();
    }
    let shown = state["wave"].as_u64().map(|wave| wave as usize);
    let count = |wave: usize| {
        blast
            .wave_by_service
            .values()
            .filter(|reached| **reached == wave)
            .count()
    };
    let mut html = String::from("<div class=\"system-waves\" role=\"group\" aria-label=\"Waves of the change\"><span class=\"graph-arrow-legend-title\">Waves:</span>");
    html.push_str(&format!(
        "<button type=\"button\" class=\"graph-arrow-legend-preset\" data-action=\"system-map-wave\" data-payload=\"{}\" aria-pressed=\"{}\" title=\"Every service the change reaches\">All</button>",
        payload_attr(&json!({"surface": surface.id, "wave": null})),
        shown.is_none(),
    ));
    for wave in 0..=last {
        let told = match wave {
            0 => format!("Where the change starts: {}", plural_services(count(0))),
            wave => format!(
                "As far as wave {wave}: {} more",
                plural_services(count(wave))
            ),
        };
        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-arrow-legend-preset\" data-action=\"system-map-wave\" data-payload=\"{}\" aria-pressed=\"{}\" title=\"{}\">{wave}</button>",
            payload_attr(&json!({"surface": surface.id, "wave": wave})),
            shown == Some(wave),
            escape(&told),
        ));
    }
    html.push_str(&format!(
        "<button type=\"button\" class=\"graph-arrow-legend-preset\" data-action=\"system-map-wave-play\" data-payload=\"{}\" title=\"Show the waves one after another\">▶ Play</button></div>",
        payload_attr(&json!({"surface": surface.id})),
    ));
    html
}
fn plural_services(count: usize) -> String {
    if count == 1 {
        "1 service".to_owned()
    } else {
        format!("{count} services")
    }
}
/// Shows the change as far as one wave, then the next, until the last.
fn play_wave(surface: SysMapSurface, wave: usize) {
    let mut state = surface_state(surface);
    let last = last_wave(&blast_of(&m::data(surface.map_key), &state));
    state["wave"] = json!(wave.min(last));
    save_surface_state(surface, &state);
    render_surface(surface, false);
    if wave < last {
        dom::later(
            &format!("{}-wave-play", surface.id_prefix),
            900,
            move || {
                play_wave(surface, wave + 1);
            },
        );
    }
}
/// Detects cross-service dependency cycles using Tarjan's strongly connected components algorithm.
pub fn find_cross_service_cycles(doc: &V) -> Vec<Vec<String>> {
    let mut service_ids = Vec::new();
    let mut id_to_idx = BTreeMap::new();
    for node in m::array(&doc["nodes"]) {
        if matches!(
            node["kind"].as_str(),
            Some("contract" | "capability" | "source")
        ) {
            continue;
        }
        let id = node_id(&node);
        if !id.is_empty() && id != "__system__" && !id_to_idx.contains_key(&id) {
            id_to_idx.insert(id.clone(), service_ids.len());
            service_ids.push(id);
        }
    }
    let n = service_ids.len();
    let mut adj = vec![Vec::new(); n];
    for edge in m::array(&doc["edges"]) {
        if m::yes(&edge["child_link"]) {
            continue;
        }
        if let (Some(&u), Some(&v)) = (
            id_to_idx.get(&m::text(&edge, "from")),
            id_to_idx.get(&m::text(&edge, "to")),
        ) {
            if u != v && !adj[u].contains(&v) {
                adj[u].push(v);
            }
        }
    }

    struct Tarjan<'a> {
        adj: &'a [Vec<usize>],
        index: usize,
        indices: Vec<Option<usize>>,
        lowlink: Vec<usize>,
        on_stack: Vec<bool>,
        stack: Vec<usize>,
        sccs: Vec<Vec<usize>>,
    }

    fn strongconnect(v: usize, t: &mut Tarjan<'_>) {
        t.indices[v] = Some(t.index);
        t.lowlink[v] = t.index;
        t.index += 1;
        t.stack.push(v);
        t.on_stack[v] = true;

        for &w in &t.adj[v] {
            if t.indices[w].is_none() {
                strongconnect(w, t);
                t.lowlink[v] = t.lowlink[v].min(t.lowlink[w]);
            } else if t.on_stack[w] {
                t.lowlink[v] = t.lowlink[v].min(t.indices[w].unwrap_or(0));
            }
        }

        if Some(t.lowlink[v]) == t.indices[v] {
            let mut comp = Vec::new();
            while let Some(w) = t.stack.pop() {
                t.on_stack[w] = false;
                comp.push(w);
                if w == v {
                    break;
                }
            }
            if comp.len() > 1 {
                comp.reverse();
                t.sccs.push(comp);
            }
        }
    }

    let mut tarjan = Tarjan {
        adj: &adj,
        index: 0,
        indices: vec![None; n],
        lowlink: vec![0; n],
        on_stack: vec![false; n],
        stack: Vec::new(),
        sccs: Vec::new(),
    };
    for v in 0..n {
        if tarjan.indices[v].is_none() {
            strongconnect(v, &mut tarjan);
        }
    }
    tarjan
        .sccs
        .into_iter()
        .map(|comp| comp.into_iter().map(|i| service_ids[i].clone()).collect())
        .collect()
}

/// Computes shortest directed cross-service paths from `from_id` to `to_id`, skipping `skip_ids`.
pub fn shortest_service_paths(
    doc: &V,
    from_id: &str,
    to_id: &str,
    skip_ids: &BTreeSet<String>,
) -> Vec<Vec<String>> {
    if from_id.is_empty() || to_id.is_empty() || from_id == to_id {
        return Vec::new();
    }
    let mut adj: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for edge in m::array(&doc["edges"]) {
        if m::yes(&edge["child_link"]) {
            continue;
        }
        let u = m::text(&edge, "from");
        let v = m::text(&edge, "to");
        if u.is_empty() || v.is_empty() || u == v {
            continue;
        }
        if (u != from_id && u != to_id && skip_ids.contains(&u))
            || (v != from_id && v != to_id && skip_ids.contains(&v))
        {
            continue;
        }
        let entry = adj.entry(u).or_default();
        if !entry.contains(&v) {
            entry.push(v);
        }
    }

    let mut queue = VecDeque::new();
    queue.push_back(vec![from_id.to_owned()]);
    let mut best_len: Option<usize> = None;
    let mut found = Vec::new();
    let mut visited_depth: BTreeMap<String, usize> = BTreeMap::new();
    visited_depth.insert(from_id.to_owned(), 0);

    while let Some(path) = queue.pop_front() {
        if let Some(best) = best_len {
            if path.len() > best {
                break;
            }
        }
        let last = path.last().expect("non-empty path");
        if last == to_id {
            best_len = Some(path.len());
            found.push(path);
            if found.len() >= 4 {
                break;
            }
            continue;
        }
        if path.len() >= 10 {
            continue;
        }
        if let Some(neighbors) = adj.get(last) {
            for next in neighbors {
                if path.contains(next) {
                    continue;
                }
                let next_depth = path.len();
                if visited_depth
                    .get(next)
                    .is_none_or(|&prev| next_depth <= prev)
                {
                    visited_depth.insert(next.clone(), next_depth);
                    let mut next_path = path.clone();
                    next_path.push(next.clone());
                    queue.push_back(next_path);
                }
            }
        }
    }
    found
}

/// Computes `both_use` (shared upstream providers) and `uses_both` (shared downstream consumers) for two services.
pub fn shared_neighbors(doc: &V, a: &str, b: &str) -> (Vec<String>, Vec<String>) {
    if a.is_empty() || b.is_empty() || a == b {
        return (Vec::new(), Vec::new());
    }
    let mut uses_a = BTreeSet::new();
    let mut uses_b = BTreeSet::new();
    let mut used_by_a = BTreeSet::new();
    let mut used_by_b = BTreeSet::new();

    for edge in m::array(&doc["edges"]) {
        if m::yes(&edge["child_link"]) {
            continue;
        }
        let from = m::text(&edge, "from");
        let to = m::text(&edge, "to");
        if from == a {
            uses_a.insert(to.clone());
        }
        if from == b {
            uses_b.insert(to.clone());
        }
        if to == a {
            used_by_a.insert(from.clone());
        }
        if to == b {
            used_by_b.insert(from.clone());
        }
    }
    let both_use = uses_a.intersection(&uses_b).cloned().collect();
    let uses_both = used_by_a.intersection(&used_by_b).cloned().collect();
    (both_use, uses_both)
}

/// Assigns architectural tiers (`0 = Edge / Gateway`, `1 = Application / Workflow`, `2 = Core Domain`, `3 = Shared Authority`)
/// and detects upward cross-tier violations.
pub fn assign_service_layers(doc: &V) -> (BTreeMap<String, usize>, Vec<(String, String)>) {
    let nodes = m::array(&doc["nodes"]);
    let edges = m::array(&doc["edges"]);
    let mut incoming: BTreeMap<String, usize> = BTreeMap::new();
    let mut outgoing: BTreeMap<String, usize> = BTreeMap::new();
    let mut ids = Vec::new();

    for n in &nodes {
        if matches!(
            n["kind"].as_str(),
            Some("contract" | "capability" | "source")
        ) {
            continue;
        }
        let id = node_id(n);
        if !id.is_empty() {
            incoming.entry(id.clone()).or_insert(0);
            outgoing.entry(id.clone()).or_insert(0);
            ids.push((id, m::text(n, "role").to_ascii_lowercase()));
        }
    }

    for e in &edges {
        if m::yes(&e["child_link"]) {
            continue;
        }
        let from = m::text(e, "from");
        let to = m::text(e, "to");
        if from != to && incoming.contains_key(&from) && incoming.contains_key(&to) {
            *outgoing.entry(from).or_default() += 1;
            *incoming.entry(to).or_default() += 1;
        }
    }

    let mut tiers = BTreeMap::new();
    for (id, role) in &ids {
        let tier = if id == "__system__" || role.contains("shared") {
            3
        } else if role.contains("gateway")
            || role.contains("edge")
            || role.contains("bff")
            || id.contains("gateway")
            || (incoming[id] == 0 && outgoing[id] > 0)
        {
            0
        } else if outgoing[id] == 0
            || role.contains("identity")
            || role.contains("auth")
            || role.contains("platform")
            || role.contains("core")
        {
            2
        } else {
            1
        };
        tiers.insert(id.clone(), tier);
    }

    let mut violations = Vec::new();
    for e in &edges {
        if m::yes(&e["child_link"]) {
            continue;
        }
        let from = m::text(e, "from");
        let to = m::text(e, "to");
        if let (Some(&tf), Some(&tt)) = (tiers.get(&from), tiers.get(&to)) {
            // Normal flow goes left-to-right (lower tier index -> higher/equal tier index).
            if tf > tt && to != "__system__" {
                violations.push((from, to));
            }
        }
    }
    (tiers, violations)
}

/// Computes Sugiyama-style left-to-right layered coordinates, unfolded service frames, and smooth cubic Bezier wires.
pub fn layout_system_nodes(doc: &V, layers_mode: bool) -> SysLayout {
    layout_system_nodes_with_pins(doc, layers_mode, &BTreeMap::new())
}

pub fn layout_system_nodes_with_pins(
    doc: &V,
    layers_mode: bool,
    pins: &BTreeMap<String, (f64, f64)>,
) -> SysLayout {
    let all_nodes = m::array(&doc["nodes"]);
    let all_edges = m::array(&doc["edges"]);
    if all_nodes.is_empty() {
        return SysLayout::default();
    }

    let cycles = find_cross_service_cycles(doc);
    let cycle_members: BTreeSet<String> = cycles.into_iter().flatten().collect();
    let (tiers, _) = assign_service_layers(doc);

    let mut top_nodes = Vec::new();
    let mut children_by_service: BTreeMap<String, Vec<V>> = BTreeMap::new();

    for n in &all_nodes {
        let kind = n["kind"].as_str().unwrap_or("service");
        if matches!(kind, "contract" | "capability" | "source")
            && !node_id(n).starts_with("__unmatched__:")
        {
            let parent = first_non_empty(n, &["parent_service", "parent", "service"]);
            if !parent.is_empty() {
                children_by_service
                    .entry(parent)
                    .or_default()
                    .push(n.clone());
                continue;
            }
        }
        top_nodes.push(n.clone());
    }

    let mut top_idx_by_id = BTreeMap::new();
    for (idx, n) in top_nodes.iter().enumerate() {
        top_idx_by_id.insert(node_id(n), idx);
    }

    let count = top_nodes.len();
    let mut col_of = vec![0usize; count];

    if layers_mode {
        for (idx, n) in top_nodes.iter().enumerate() {
            let id = node_id(n);
            col_of[idx] = tiers.get(&id).copied().unwrap_or(1);
        }
    } else {
        // Longest-path layering on DAG (ignoring cycle back-edges by index order).
        let mut indeg = vec![0usize; count];
        let mut dag_adj = vec![Vec::new(); count];
        for e in &all_edges {
            if m::yes(&e["child_link"]) {
                continue;
            }
            let from = m::text(e, "from");
            let to = m::text(e, "to");
            if let (Some(&u), Some(&v)) = (top_idx_by_id.get(&from), top_idx_by_id.get(&to)) {
                if u == v {
                    continue;
                }
                if cycle_members.contains(&from) && cycle_members.contains(&to) && u > v {
                    continue;
                }
                dag_adj[u].push(v);
                indeg[v] += 1;
            }
        }
        let mut q = VecDeque::new();
        for (i, &deg) in indeg.iter().enumerate() {
            if deg == 0 {
                q.push_back(i);
            }
        }
        let mut visited = vec![false; count];
        while let Some(u) = q.pop_front() {
            visited[u] = true;
            for &v in &dag_adj[u] {
                col_of[v] = col_of[v].max(col_of[u] + 1).min(6);
                indeg[v] = indeg[v].saturating_sub(1);
                if indeg[v] == 0 {
                    q.push_back(v);
                }
            }
        }
        for i in 0..count {
            if !visited[i] {
                col_of[i] = col_of[i].max(1);
            }
            if node_id(&top_nodes[i]) == "__system__" {
                col_of[i] = col_of.iter().copied().max().unwrap_or(1).max(1);
            }
        }
    }

    // Group top-level services into columns.
    let max_col = col_of.iter().copied().max().unwrap_or(0);
    let mut cols: Vec<Vec<usize>> = vec![Vec::new(); max_col + 1];
    for (idx, &c) in col_of.iter().enumerate() {
        cols[c].push(idx);
    }

    // Compact empty columns when not in fixed 4-tier layers mode.
    let mut active_cols: Vec<(usize, Vec<usize>)> = if layers_mode {
        (0..=3)
            .map(|tier| {
                let members = cols.get(tier).cloned().unwrap_or_default();
                (tier, members)
            })
            .filter(|(_, m)| !m.is_empty())
            .collect()
    } else {
        cols.into_iter()
            .enumerate()
            .filter(|(_, m)| !m.is_empty())
            .collect()
    };

    // Multi-pass barycenter ordering across columns (inspired by graph_map::plan_roomy)
    // minimizes edge crossings while keeping `__system__` pinned last in its column.
    if active_cols.len() > 1 {
        let mut col_idx_by_node = vec![0usize; count];
        let mut row_by_node = vec![0usize; count];
        for (ci, (_, members)) in active_cols.iter().enumerate() {
            for (ri, &u) in members.iter().enumerate() {
                col_idx_by_node[u] = ci;
                row_by_node[u] = ri;
            }
        }
        let mut neighbors = vec![Vec::<usize>::new(); count];
        for e in &all_edges {
            if m::yes(&e["child_link"]) {
                continue;
            }
            if let (Some(&u), Some(&v)) = (
                top_idx_by_id.get(&m::text(e, "from")),
                top_idx_by_id.get(&m::text(e, "to")),
            ) {
                if u != v && col_idx_by_node[u] != col_idx_by_node[v] {
                    neighbors[u].push(v);
                    neighbors[v].push(u);
                }
            }
        }
        for pass in 0..8usize {
            let col_order: Vec<usize> = if pass % 2 == 0 {
                (1..active_cols.len()).collect()
            } else {
                (0..active_cols.len().saturating_sub(1)).rev().collect()
            };
            for ci in col_order {
                let prefer_left = pass % 2 == 0;
                let mut scored: Vec<(bool, f64, usize)> = active_cols[ci]
                    .1
                    .iter()
                    .map(|&u| {
                        let is_sys = node_id(&top_nodes[u]) == "__system__";
                        let preferred: Vec<usize> = neighbors[u]
                            .iter()
                            .copied()
                            .filter(|&v| {
                                if prefer_left {
                                    col_idx_by_node[v] < ci
                                } else {
                                    col_idx_by_node[v] > ci
                                }
                            })
                            .collect();
                        let pool = if preferred.is_empty() {
                            &neighbors[u]
                        } else {
                            &preferred
                        };
                        let score = if pool.is_empty() {
                            row_by_node[u] as f64
                        } else {
                            let sum: usize = pool.iter().map(|&v| row_by_node[v]).sum();
                            sum as f64 / pool.len() as f64
                        };
                        (is_sys, score, u)
                    })
                    .collect();
                scored.sort_by(|a, b| {
                    a.0.cmp(&b.0)
                        .then_with(|| a.1.total_cmp(&b.1))
                        .then_with(|| a.2.cmp(&b.2))
                });
                active_cols[ci].1 = scored.iter().map(|x| x.2).collect();
                for (ri, &u) in active_cols[ci].1.iter().enumerate() {
                    row_by_node[u] = ri;
                }
            }
        }
    }

    let mut placed_nodes = Vec::new();
    let mut frames = Vec::new();
    let mut tier_bands = Vec::new();
    let mut pos_by_id: BTreeMap<String, (f64, f64, f64, f64, usize)> = BTreeMap::new();

    let top_offset = if layers_mode { MARGIN + 30.0 } else { MARGIN };
    let mut cur_x = MARGIN;
    let mut max_y: f64 = 220.0;

    for (render_col, (logical_tier, indices)) in active_cols.iter().enumerate() {
        let mut col_w = NODE_WIDTH;
        for &idx in indices {
            let sid = node_id(&top_nodes[idx]);
            if children_by_service.contains_key(&sid) {
                col_w = col_w.max(NODE_WIDTH + 2.0 * FRAME_PAD);
            }
        }

        let mut cur_y = top_offset;
        for &idx in indices {
            let n = &top_nodes[idx];
            let sid = node_id(n);
            let in_cycle = cycle_members.contains(&sid);
            if let Some(children) = children_by_service.get(&sid) {
                let frame_x = cur_x;
                let frame_y = cur_y;
                let inner_x = frame_x + FRAME_PAD;
                let mut inner_y = frame_y + FRAME_HEAD;

                placed_nodes.push(PlacedSysNode {
                    id: sid.clone(),
                    x: inner_x,
                    y: inner_y,
                    w: NODE_WIDTH,
                    h: NODE_HEIGHT,
                    col: render_col,
                    cycle: in_cycle,
                    child_of: None,
                });
                pos_by_id.insert(
                    sid.clone(),
                    (inner_x, inner_y, NODE_WIDTH, NODE_HEIGHT, render_col),
                );
                inner_y += NODE_HEIGHT + 8.0;

                for child in children {
                    let cid = node_id(child);
                    placed_nodes.push(PlacedSysNode {
                        id: cid.clone(),
                        x: inner_x + 16.0,
                        y: inner_y,
                        w: CHILD_WIDTH,
                        h: CHILD_HEIGHT,
                        col: render_col,
                        cycle: false,
                        child_of: Some(sid.clone()),
                    });
                    pos_by_id.insert(
                        cid,
                        (
                            inner_x + 16.0,
                            inner_y,
                            CHILD_WIDTH,
                            CHILD_HEIGHT,
                            render_col,
                        ),
                    );
                    inner_y += CHILD_HEIGHT + 6.0;
                }

                let frame_h = (inner_y - frame_y) + FRAME_PAD - 6.0;
                let frame_w = NODE_WIDTH + 2.0 * FRAME_PAD;
                frames.push(PlacedSysFrame {
                    service_id: sid.clone(),
                    title: m::text(n, "name"),
                    x: frame_x,
                    y: frame_y,
                    w: frame_w,
                    h: frame_h,
                });
                cur_y += frame_h + ROW_GAP;
            } else {
                let nx = cur_x + ((col_w - NODE_WIDTH) / 2.0).round();
                let ny = cur_y;
                placed_nodes.push(PlacedSysNode {
                    id: sid.clone(),
                    x: nx,
                    y: ny,
                    w: NODE_WIDTH,
                    h: NODE_HEIGHT,
                    col: render_col,
                    cycle: in_cycle,
                    child_of: None,
                });
                pos_by_id.insert(sid, (nx, ny, NODE_WIDTH, NODE_HEIGHT, render_col));
                cur_y += NODE_HEIGHT + ROW_GAP;
            }
        }

        if layers_mode {
            let tier_name = match logical_tier {
                0 => "Edge & Gateways",
                1 => "Application Services",
                2 => "Core & Platform",
                _ => "Shared Authority",
            };
            tier_bands.push((tier_name.to_owned(), cur_x - 8.0, col_w + 16.0, cur_y));
        }

        max_y = max_y.max(cur_y);
        cur_x += col_w + COL_GAP;
    }

    // Apply user Shift+drag pin overrides before computing port routing & wires so edges follow pinned boxes.
    let mut extra_w: f64 = 0.0;
    let mut extra_h: f64 = 0.0;
    if !pins.is_empty() {
        let mut parent_deltas: BTreeMap<String, (f64, f64)> = BTreeMap::new();
        for n in &mut placed_nodes {
            if n.child_of.is_some() {
                continue;
            }
            if let Some(&(px, py)) = pins.get(&n.id) {
                let dx = px - n.x;
                let dy = py - n.y;
                n.x = px;
                n.y = py;
                if let Some(entry) = pos_by_id.get_mut(&n.id) {
                    entry.0 = px;
                    entry.1 = py;
                }
                parent_deltas.insert(n.id.clone(), (dx, dy));
                for f in &mut frames {
                    if f.service_id == n.id {
                        f.x += dx;
                        f.y += dy;
                        extra_w = extra_w.max(f.x + f.w + MARGIN);
                        extra_h = extra_h.max(f.y + f.h + MARGIN);
                    }
                }
                extra_w = extra_w.max(px + n.w + MARGIN);
                extra_h = extra_h.max(py + n.h + MARGIN);
            }
        }
        // Shift unfolded children cleanly with their pinned parent frame unless pinned individually.
        for n in &mut placed_nodes {
            let Some(parent_id) = n.child_of.as_ref() else {
                continue;
            };
            if let Some(&(px, py)) = pins.get(&n.id) {
                n.x = px;
                n.y = py;
            } else if let Some(&(dx, dy)) = parent_deltas.get(parent_id) {
                n.x += dx;
                n.y += dy;
            } else {
                continue;
            }
            if let Some(entry) = pos_by_id.get_mut(&n.id) {
                entry.0 = n.x;
                entry.1 = n.y;
            }
            extra_w = extra_w.max(n.x + n.w + MARGIN);
            extra_h = extra_h.max(n.y + n.h + MARGIN);
        }
    }

    let total_w = (cur_x - COL_GAP + MARGIN).max(420.0).max(extra_w);
    let total_h = (max_y - ROW_GAP + MARGIN).max(240.0).max(extra_h);

    struct CandidateWire {
        edge_index: usize,
        from: String,
        to: String,
        fx: f64,
        fy: f64,
        fw: f64,
        fh: f64,
        fcol: usize,
        tx: f64,
        ty: f64,
        tw: f64,
        th: f64,
        tcol: usize,
        back: bool,
        src_side: u8,
        dst_side: u8,
    }

    struct PortSlot {
        other_y: f64,
        back: bool,
        cw_idx: usize,
        is_src: bool,
    }

    let mut candidates = Vec::new();
    let mut port_slots: BTreeMap<(String, u8), Vec<PortSlot>> = BTreeMap::new();

    for (edge_index, edge) in all_edges.iter().enumerate() {
        let from = m::text(edge, "from");
        let to = m::text(edge, "to");
        let Some(&(fx, fy, fw, fh, fcol)) = pos_by_id.get(&from) else {
            continue;
        };
        let Some(&(tx, ty, tw, th, tcol)) = pos_by_id.get(&to) else {
            continue;
        };
        if from == to {
            continue;
        }
        let back = fcol > tcol || (fcol == tcol && fy > ty);
        let f_mid_x = fx + fw * 0.5;
        let t_mid_x = tx + tw * 0.5;
        let same_x_col = (f_mid_x - t_mid_x).abs() < (fw.min(tw) * 0.45);
        let (src_side, dst_side) = if same_x_col {
            (1u8, 1u8)
        } else if f_mid_x < t_mid_x {
            (1u8, 0u8)
        } else {
            (0u8, 1u8)
        };
        let cw_idx = candidates.len();
        port_slots
            .entry((from.clone(), src_side))
            .or_default()
            .push(PortSlot {
                other_y: ty + th / 2.0,
                back,
                cw_idx,
                is_src: true,
            });
        port_slots
            .entry((to.clone(), dst_side))
            .or_default()
            .push(PortSlot {
                other_y: fy + fh / 2.0,
                back,
                cw_idx,
                is_src: false,
            });
        candidates.push(CandidateWire {
            edge_index,
            from,
            to,
            fx,
            fy,
            fw,
            fh,
            fcol,
            tx,
            ty,
            tw,
            th,
            tcol,
            back,
            src_side,
            dst_side,
        });
    }

    let mut port_ranks: BTreeMap<(usize, bool), (usize, usize)> = BTreeMap::new();
    for entries in port_slots.values_mut() {
        entries.sort_by(|a, b| {
            a.other_y
                .total_cmp(&b.other_y)
                .then_with(|| a.back.cmp(&b.back))
                .then_with(|| a.cw_idx.cmp(&b.cw_idx))
        });
        let total = entries.len();
        for (rank, slot) in entries.iter().enumerate() {
            port_ranks.insert((slot.cw_idx, slot.is_src), (rank, total));
        }
    }

    let spread_y = |top: f64, height: f64, base_frac: f64, rank: usize, total: usize| -> f64 {
        if total <= 1 {
            return top + height * base_frac;
        }
        let step = SYS_PORT_GAP.min((height * 0.55) / total as f64);
        let offset = ((rank as f64 - (total as f64 - 1.0) / 2.0) * step)
            .clamp(-SYS_PORT_SPREAD, SYS_PORT_SPREAD);
        (top + height * base_frac + offset).clamp(top + 6.0, top + height - 6.0)
    };

    let mut wires = Vec::with_capacity(candidates.len());
    for (cw_idx, cw) in candidates.into_iter().enumerate() {
        let f_mid_x = cw.fx + cw.fw * 0.5;
        let t_mid_x = cw.tx + cw.tw * 0.5;
        let same_x_col = (f_mid_x - t_mid_x).abs() < (cw.fw.min(cw.tw) * 0.45);
        let base_frac = if !same_x_col && f_mid_x > t_mid_x {
            0.35
        } else {
            0.5
        };
        let (s_rank, s_total) = port_ranks.get(&(cw_idx, true)).copied().unwrap_or((0, 1));
        let (d_rank, d_total) = port_ranks.get(&(cw_idx, false)).copied().unwrap_or((0, 1));
        let sx = if cw.src_side == 1 {
            cw.fx + cw.fw
        } else {
            cw.fx
        };
        let ex = if cw.dst_side == 1 {
            cw.tx + cw.tw
        } else {
            cw.tx
        };
        let sy = spread_y(cw.fy, cw.fh, base_frac, s_rank, s_total);
        let ey = spread_y(cw.ty, cw.th, base_frac, d_rank, d_total);
        let dx = (ex - sx).abs().max(48.0) * 0.45;
        let (c1x, c2x) = if same_x_col {
            (sx + 44.0, ex + 44.0)
        } else if f_mid_x < t_mid_x {
            (sx + dx, ex - dx)
        } else {
            (sx - dx, ex + dx)
        };
        let _ = (cw.fcol, cw.tcol);
        let path_d = format!("M{sx:.1},{sy:.1} C{c1x:.1},{sy:.1} {c2x:.1},{ey:.1} {ex:.1},{ey:.1}");
        let mid_x = 0.125 * sx + 0.375 * c1x + 0.375 * c2x + 0.125 * ex;
        let mid_y = 0.5 * (sy + ey);
        let _ = (cw.from, cw.to);
        wires.push(PlacedSysWire {
            edge_index: cw.edge_index,
            path_d,
            mid_x,
            mid_y,
            back: cw.back,
        });
    }

    SysLayout {
        nodes: placed_nodes,
        frames,
        wires,
        tier_bands,
        width: total_w,
        height: total_h,
    }
}

/// Builds a concise, human- and agent-readable system topology / blast-radius brief
/// suitable for attaching straight into `#system-task` in `Changes`.
pub fn build_context_brief(doc: &V, state: &V) -> String {
    let overlay = &state["overlay"];
    let seeds: Vec<String> = m::array(&overlay["seed_services"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let contracts: Vec<String> = m::array(&overlay["changed_contracts"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let depth = overlay["depth"].as_u64().unwrap_or(32) as usize;
    let blast = compute_blast_radius(doc, &seeds, &contracts, depth);

    let mut lines = Vec::new();
    lines.push("[System Architecture & Blast-Radius Context]".to_owned());

    let focus = m::text(state, "focus");
    if !focus.is_empty() {
        if let Some(node) = m::array(&doc["nodes"])
            .into_iter()
            .find(|n| node_id(n) == focus)
        {
            let provides = m::array(&node["provides"])
                .iter()
                .map(contract_name)
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>();
            let consumes = m::array(&node["consumes"])
                .iter()
                .map(contract_name)
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>();
            lines.push(format!(
                "- Focused service: {} ({})",
                m::text(&node, "name"),
                focus
            ));
            if !provides.is_empty() {
                lines.push(format!("  Provides contracts: {}", provides.join(", ")));
            }
            if !consumes.is_empty() {
                lines.push(format!("  Consumes contracts: {}", consumes.join(", ")));
            }
        }
    }

    if !blast.seeds.is_empty() {
        let seed_list = blast.seeds.iter().cloned().collect::<Vec<_>>().join(", ");
        lines.push(format!(
            "- Starting seed services: {seed_list} (depth {depth})"
        ));
        if !blast.direct_consumers.is_empty() {
            lines.push(format!(
                "- Direct contract consumers: {}",
                blast
                    .direct_consumers
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !blast.downstream_consumers.is_empty() {
            lines.push(format!(
                "- Transitive downstream wave: {}",
                blast
                    .downstream_consumers
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    if let Some(sel) = state["selected_edge"].as_object() {
        let from = sel.get("from").and_then(V::as_str).unwrap_or("");
        let to = sel.get("to").and_then(V::as_str).unwrap_or("");
        if !from.is_empty() && !to.is_empty() {
            lines.push(format!("- Selected contract link: {from} -> {to}"));
        }
    }

    let broken_edges = m::array(&doc["edges"])
        .into_iter()
        .filter(|e| m::yes(&e["broken_risk"]) || e["provenance"] == "undeclared_in_code")
        .take(4)
        .map(|e| format!("{} -> {}", m::text(&e, "from"), m::text(&e, "to")))
        .collect::<Vec<_>>();
    if !broken_edges.is_empty() {
        lines.push(format!(
            "- Contract risk / drift links to verify: {}",
            broken_edges.join(", ")
        ));
    }

    if lines.len() == 1 {
        let svcs = m::array(&doc["nodes"])
            .into_iter()
            .filter(|n| n["kind"] == "service")
            .map(|n| node_id(&n))
            .collect::<Vec<_>>();
        lines.push(format!(
            "- Member services ({}): {}",
            svcs.len(),
            svcs.join(", ")
        ));
    }

    lines.join("\n")
}

fn node_id(node: &V) -> String {
    first_non_empty(node, &["id", "service", "path", "name"])
}

fn service_display_name(svc: &V) -> String {
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

fn contract_name(c: &V) -> String {
    if let Some(s) = c.as_str() {
        return s.trim().to_owned();
    }
    first_non_empty(c, &["name", "contract", "id", "topic", "endpoint"])
}

fn capability_name(cap: &V) -> String {
    if let Some(s) = cap.as_str() {
        return s.trim().to_owned();
    }
    first_non_empty(cap, &["name", "id", "label", "title"])
}

fn endpoint_service_id(
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

fn explain_services_for_endpoints(
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

fn first_non_empty(v: &V, keys: &[&str]) -> String {
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

fn push_unique_str(arr_val: &mut V, item: &str) {
    if !arr_val.is_array() {
        *arr_val = json!([]);
    }
    let arr = arr_val.as_array_mut().expect("array");
    if !arr.iter().any(|x| x.as_str() == Some(item)) {
        arr.push(json!(item));
    }
}

fn push_contract_obj(arr_val: &mut V, obj: V) {
    if !arr_val.is_array() {
        *arr_val = json!([]);
    }
    let cname = m::text(&obj, "name");
    let arr = arr_val.as_array_mut().expect("array");
    if cname.is_empty() || !arr.iter().any(|x| m::text(x, "name") == cname) {
        arr.push(obj);
    }
}

fn sys_pin_key(surface: SysMapSurface, state: &V) -> String {
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

pub fn system_structure_digest(doc: &V, state: &V, pins: &BTreeMap<String, (f64, f64)>) -> u64 {
    let mut hasher = DefaultHasher::new();
    m::yes(&state["layers"]).hash(&mut hasher);
    m::yes(&state["whole"]).hash(&mut hasher);
    m::yes(&state["certain"]).hash(&mut hasher);
    m::text(state, "filter").hash(&mut hasher);
    m::yes(&state["isolate_active"]).hash(&mut hasher);
    state["isolate_depth"]
        .as_u64()
        .unwrap_or(0)
        .hash(&mut hasher);
    m::text(state, "isolate_seed").hash(&mut hasher);
    graph_viewport::hash_json_value(&state["overlay"], &mut hasher);
    state["wave"].as_u64().hash(&mut hasher);
    m::text(state, "tool").hash(&mut hasher);
    m::text(state, "path_from").hash(&mut hasher);
    m::text(state, "path_to").hash(&mut hasher);
    m::text(state, "shared_a").hash(&mut hasher);
    m::text(state, "shared_b").hash(&mut hasher);
    m::text(state, "shared_side").hash(&mut hasher);
    state["cycle_index"].as_u64().unwrap_or(0).hash(&mut hasher);
    m::text(state, "follow").hash(&mut hasher);
    m::text(state, "follow_dir").hash(&mut hasher);
    graph_viewport::hash_json_value(&state["hidden_kinds"], &mut hasher);
    graph_viewport::hash_json_value(&state["skip"], &mut hasher);
    graph_viewport::hash_json_value(&m::data("system-failed-services"), &mut hasher);
    let nodes = m::array(&doc["nodes"]);
    nodes.len().hash(&mut hasher);
    for n in &nodes {
        let id = node_id(n);
        id.hash(&mut hasher);
        if let Some((px, py)) = pins.get(&id) {
            px.to_bits().hash(&mut hasher);
            py.to_bits().hash(&mut hasher);
        }
    }
    let edges = m::array(&doc["edges"]);
    edges.len().hash(&mut hasher);
    for e in &edges {
        m::text(e, "from").hash(&mut hasher);
        m::text(e, "to").hash(&mut hasher);
    }
    hasher.finish()
}

fn filtered_doc(doc: &V, state: &V) -> V {
    let certain = m::yes(&state["certain"]);
    let filter = m::text(state, "filter").trim().to_ascii_lowercase();
    let isolate_active = m::yes(&state["isolate_active"]);
    if !certain && filter.is_empty() && !isolate_active {
        return doc.clone();
    }

    let mut nodes = m::array(&doc["nodes"]);
    let mut edges = m::array(&doc["edges"]);

    if certain {
        edges.retain(|e| {
            let prov = m::text(e, "provenance");
            let cert = m::text(e, "certainty");
            (cert.is_empty() || cert == "EXTRACTED") && prov != "guessed" && !m::yes(&e["guessed"])
        });
    }

    if !filter.is_empty() {
        let mut matched_ids = BTreeSet::new();
        for n in &nodes {
            let hay = format!(
                "{} {} {} {}",
                node_id(n),
                m::text(n, "name"),
                m::text(n, "role"),
                m::text(n, "language")
            )
            .to_ascii_lowercase();
            let contract_match = m::array(&n["provides"])
                .iter()
                .chain(m::array(&n["consumes"]).iter())
                .any(|c| contract_name(c).to_ascii_lowercase().contains(&filter));
            if hay.contains(&filter) || contract_match {
                matched_ids.insert(node_id(n));
                let p = m::text(n, "parent_service");
                if !p.is_empty() {
                    matched_ids.insert(p);
                }
            }
        }
        nodes.retain(|n| matched_ids.contains(&node_id(n)));
        edges.retain(|e| {
            matched_ids.contains(&m::text(e, "from")) && matched_ids.contains(&m::text(e, "to"))
        });
    }

    if isolate_active {
        let seed_id = {
            let s = m::text(state, "isolate_seed");
            if s.is_empty() {
                m::text(state, "focus")
            } else {
                s
            }
        };
        if !seed_id.is_empty() {
            let id_to_idx: BTreeMap<String, usize> = nodes
                .iter()
                .enumerate()
                .map(|(i, n)| (node_id(n), i))
                .collect();
            if let Some(&seed_idx) = id_to_idx.get(&seed_id) {
                let mut out_adj = vec![Vec::new(); nodes.len()];
                let mut in_adj = vec![Vec::new(); nodes.len()];
                for e in &edges {
                    if let (Some(&u), Some(&v)) = (
                        id_to_idx.get(&m::text(e, "from")),
                        id_to_idx.get(&m::text(e, "to")),
                    ) {
                        out_adj[u].push(v);
                        in_adj[v].push(u);
                    }
                }
                let depth = state["isolate_depth"].as_u64().unwrap_or(2).clamp(1, 3) as usize;
                let dist = graph_viewport::hop_cone_distances(
                    nodes.len(),
                    &out_adj,
                    &in_adj,
                    &[seed_idx],
                    depth,
                );
                let mut kept: BTreeSet<String> = nodes
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| dist[*i].is_some())
                    .map(|(_, n)| node_id(n))
                    .collect();
                // Keep children of kept services so unfolded frames remain intact.
                for n in &nodes {
                    let parent = first_non_empty(n, &["parent_service", "parent", "service"]);
                    if !parent.is_empty() && kept.contains(&parent) {
                        kept.insert(node_id(n));
                    }
                }
                nodes.retain(|n| kept.contains(&node_id(n)));
                edges.retain(|e| {
                    kept.contains(&m::text(e, "from")) && kept.contains(&m::text(e, "to"))
                });
            }
        }
    }

    let mut out = doc.clone();
    out["nodes"] = json!(nodes);
    out["edges"] = json!(edges);
    out
}

fn place_sys_edge_label(placed: &mut Vec<(f64, f64)>, candidate: (f64, f64)) -> (f64, f64) {
    let clashes = |x: f64, y: f64, p: &(f64, f64)| (x - p.0).abs() < 56.0 && (y - p.1).abs() < 18.0;
    for step in 0..8i32 {
        let offsets: &[f64] = if step == 0 {
            &[0.0]
        } else {
            &[18.0 * step as f64, -18.0 * step as f64]
        };
        for &dy in offsets {
            let y = candidate.1 + dy;
            if y >= 10.0 && !placed.iter().any(|p| clashes(candidate.0, y, p)) {
                placed.push((candidate.0, y));
                return (candidate.0, y);
            }
        }
    }
    placed.push(candidate);
    candidate
}

struct SysVisualState {
    focus_id: String,
    sel_edge_pair: (String, String),
    blast: BlastRadius,
    shown_wave: Option<usize>,
    step_states: V,
    unopened_consumers: BTreeSet<String>,
    highlight_nodes: BTreeSet<String>,
    highlight_edges: BTreeSet<(String, String)>,
    failed_services: BTreeSet<String>,
}

impl SysVisualState {
    fn beyond(&self, service: &str) -> bool {
        self.shown_wave.is_some_and(|shown| {
            self.blast
                .wave_by_service
                .get(service)
                .is_some_and(|wave| *wave > shown)
        })
    }

    fn blast_role(&self, id: &str) -> &'static str {
        if self.beyond(id) {
            ""
        } else if self.blast.seeds.contains(id) {
            "seed"
        } else if self.blast.direct_consumers.contains(id) {
            "direct"
        } else if self.blast.downstream_consumers.contains(id) {
            "downstream"
        } else {
            ""
        }
    }

    fn step_state(&self, id: &str) -> String {
        if self.step_states.is_object() {
            m::text(&self.step_states, id)
        } else {
            String::new()
        }
    }

    fn is_changed(&self, id: &str) -> bool {
        !self.beyond(id)
            && (self.blast.seeds.contains(id)
                || self.blast.direct_consumers.contains(id)
                || self.blast.downstream_consumers.contains(id)
                || !self.step_state(id).is_empty())
    }
}

fn compute_sys_visual_state(view_doc: &V, state: &V) -> SysVisualState {
    let overlay = &state["overlay"];
    let seeds: Vec<String> = m::array(&overlay["seed_services"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let changed_contracts: Vec<String> = m::array(&overlay["changed_contracts"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let depth = overlay["depth"].as_u64().unwrap_or(32) as usize;
    let blast = compute_blast_radius(view_doc, &seeds, &changed_contracts, depth);
    let shown_wave = state["wave"].as_u64().map(|wave| wave as usize);
    let step_states = overlay["step_states"].clone();
    let unopened_consumers: BTreeSet<String> = m::array(&overlay["cross_service_unopened"])
        .into_iter()
        .filter_map(|v| {
            v.as_str().map(str::to_owned).or_else(|| {
                let s = first_non_empty(&v, &["consumer_service", "service", "consumer"]);
                (!s.is_empty()).then_some(s)
            })
        })
        .collect();

    let edges = m::array(&view_doc["edges"]);
    let tool = m::text(state, "tool");
    let mut highlight_nodes = BTreeSet::new();
    let mut highlight_edges = BTreeSet::new();

    if tool == "path" {
        let p_from = m::text(state, "path_from");
        let p_to = m::text(state, "path_to");
        let skip: BTreeSet<String> = m::array(&state["skip"])
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        for path in shortest_service_paths(view_doc, &p_from, &p_to, &skip) {
            for w in path.windows(2) {
                highlight_nodes.insert(w[0].clone());
                highlight_nodes.insert(w[1].clone());
                highlight_edges.insert((w[0].clone(), w[1].clone()));
            }
        }
    } else if tool == "shared" {
        let sa = m::text(state, "shared_a");
        let sb = m::text(state, "shared_b");
        let (both_use, uses_both) = shared_neighbors(view_doc, &sa, &sb);
        let active_list = if m::text(state, "shared_side") == "uses_both" {
            uses_both
        } else {
            both_use
        };
        if !sa.is_empty() && !sb.is_empty() {
            highlight_nodes.insert(sa.clone());
            highlight_nodes.insert(sb.clone());
            for mid in active_list {
                highlight_nodes.insert(mid.clone());
                highlight_edges.insert((sa.clone(), mid.clone()));
                highlight_edges.insert((sb.clone(), mid.clone()));
                highlight_edges.insert((mid.clone(), sa.clone()));
                highlight_edges.insert((mid.clone(), sb.clone()));
            }
        }
    } else if tool == "cycle" {
        let cycles = find_cross_service_cycles(view_doc);
        let idx = state["cycle_index"].as_u64().unwrap_or(0) as usize;
        if let Some(comp) = cycles.get(idx).or_else(|| cycles.first()) {
            let set: BTreeSet<String> = comp.iter().cloned().collect();
            for s in &set {
                highlight_nodes.insert(s.clone());
            }
            for e in &edges {
                let f = m::text(e, "from");
                let t = m::text(e, "to");
                if set.contains(&f) && set.contains(&t) {
                    highlight_edges.insert((f, t));
                }
            }
        }
    } else if tool == "follow" {
        let root = m::text(state, "follow");
        if !root.is_empty() {
            highlight_nodes.insert(root.clone());
            let dir = m::text(state, "follow_dir");
            let hops: BTreeSet<String> =
                m::array(&state[if dir == "into" { "into" } else { "out" }])
                    .into_iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .chain(std::iter::once(root))
                    .collect();
            for e in &edges {
                let f = m::text(e, "from");
                let t = m::text(e, "to");
                if dir == "into" && hops.contains(&t) {
                    highlight_nodes.insert(f.clone());
                    highlight_edges.insert((f, t));
                } else if dir != "into" && hops.contains(&f) {
                    highlight_nodes.insert(t.clone());
                    highlight_edges.insert((f, t));
                }
            }
        }
    }

    let focus_id = m::text(state, "focus");
    let sel_edge = &state["selected_edge"];
    let sel_edge_pair = sel_edge
        .as_object()
        .map(|o| {
            (
                o.get("from").and_then(V::as_str).unwrap_or("").to_owned(),
                o.get("to").and_then(V::as_str).unwrap_or("").to_owned(),
            )
        })
        .unwrap_or_default();
    let failed_services: BTreeSet<String> = m::array(&m::data("system-failed-services"))
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();

    SysVisualState {
        focus_id,
        sel_edge_pair,
        blast,
        shown_wave,
        step_states,
        unopened_consumers,
        highlight_nodes,
        highlight_edges,
        failed_services,
    }
}

/// Renders the SVG minimap radar for a laid-out system graph.
pub fn system_radar_html(doc: &V, state: &V, layout: &SysLayout) -> String {
    let (rw, rh) = radar_size((layout.width, layout.height));
    if rw <= 0.0 || layout.nodes.is_empty() {
        return String::new();
    }
    let focus_id = m::text(state, "focus");
    let overlay = &state["overlay"];
    let seeds: BTreeSet<String> = m::array(&overlay["seed_services"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let blast_doc = filtered_doc(doc, state);
    let seed_vec: Vec<String> = seeds.iter().cloned().collect();
    let changed_contracts: Vec<String> = m::array(&overlay["changed_contracts"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let depth = overlay["depth"].as_u64().unwrap_or(32) as usize;
    let blast = compute_blast_radius(&blast_doc, &seed_vec, &changed_contracts, depth);

    let mut svg = format!(
        "<svg class=\"graph-radar-picture\" width=\"{rw:.0}\" height=\"{rh:.0}\" viewBox=\"0 0 {:.0} {:.0}\" preserveAspectRatio=\"none\" data-w=\"{:.0}\" data-h=\"{:.0}\" focusable=\"false\">",
        layout.width, layout.height, layout.width, layout.height
    );
    for frame in &layout.frames {
        svg.push_str(&format!(
            "<rect class=\"graph-radar-frame\" x=\"{:.0}\" y=\"{:.0}\" width=\"{:.0}\" height=\"{:.0}\"/>",
            frame.x, frame.y, frame.w, frame.h
        ));
    }
    for placed in &layout.nodes {
        let state_attr = if placed.id == focus_id {
            " data-state=\"selected\""
        } else if seeds.contains(&placed.id)
            || blast.direct_consumers.contains(&placed.id)
            || blast.downstream_consumers.contains(&placed.id)
        {
            " data-state=\"changed\""
        } else if placed.cycle {
            " data-state=\"cycle\""
        } else {
            ""
        };
        let cycle_attr = if placed.cycle {
            " data-cycle=\"true\""
        } else {
            ""
        };
        svg.push_str(&format!(
            "<rect class=\"graph-radar-box\" data-n=\"{}\" x=\"{:.0}\" y=\"{:.0}\" width=\"{:.0}\" height=\"{:.0}\"{cycle_attr}{state_attr}/>",
            escape(&placed.id),
            placed.x,
            placed.y,
            placed.w,
            placed.h
        ));
    }
    svg.push_str(
        "<rect class=\"graph-radar-view\" x=\"0\" y=\"0\" width=\"0\" height=\"0\"/></svg>",
    );
    svg
}

/// Renders the interactive SVG + HTML plane AND its minimap radar for `render_surface`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn render_canvas_with_radar(
    surface: SysMapSurface,
    doc: &V,
    state: &V,
) -> (String, String, f64, f64) {
    let view_doc = filtered_doc(doc, state);
    let layers_mode = m::yes(&state["layers"]);
    let pins = graph_viewport::load_pins(&sys_pin_key(surface, state));
    let layout = layout_system_nodes_with_pins(&view_doc, layers_mode, &pins);
    let plane_html = render_canvas_html_with_layout(surface, &view_doc, state, &layout);
    let radar_svg = system_radar_html(doc, state, &layout);
    (plane_html, radar_svg, layout.width, layout.height)
}

/// Renders the interactive SVG + HTML plane for `#system-map-canvas` or `#system-change-topology-canvas`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn render_canvas_html(surface: SysMapSurface, doc: &V, state: &V) -> (String, f64, f64) {
    let view_doc = filtered_doc(doc, state);
    let layers_mode = m::yes(&state["layers"]);
    let layout = layout_system_nodes(&view_doc, layers_mode);
    let html = render_canvas_html_with_layout(surface, &view_doc, state, &layout);
    (html, layout.width, layout.height)
}

struct SysRenderBundle {
    html: String,
    is_dense: bool,
    node_htmls: Vec<String>,
    edge_endpoints: Vec<(String, String)>,
    active_roving_id: String,
    highlight_nodes: BTreeSet<String>,
    failed_services: BTreeSet<String>,
    pinned_ids: BTreeSet<String>,
}

fn render_canvas_html_with_layout(
    surface: SysMapSurface,
    view_doc: &V,
    state: &V,
    layout: &SysLayout,
) -> String {
    render_canvas_bundle_with_layout(surface, view_doc, state, layout).html
}

fn render_canvas_bundle_with_layout(
    surface: SysMapSurface,
    view_doc: &V,
    state: &V,
    layout: &SysLayout,
) -> SysRenderBundle {
    let layers_mode = m::yes(&state["layers"]);
    let view_doc = view_doc.clone();
    let nodes = m::array(&view_doc["nodes"]);
    let edges = m::array(&view_doc["edges"]);

    if nodes.is_empty() {
        return SysRenderBundle {
            html: String::new(),
            is_dense: false,
            node_htmls: Vec::new(),
            edge_endpoints: Vec::new(),
            active_roving_id: String::new(),
            highlight_nodes: BTreeSet::new(),
            failed_services: BTreeSet::new(),
            pinned_ids: BTreeSet::new(),
        };
    }

    let mut node_by_id = BTreeMap::new();
    for n in &nodes {
        node_by_id.insert(node_id(n), n);
    }

    let vis = compute_sys_visual_state(&view_doc, state);
    let (_, layer_violations) = assign_service_layers(&view_doc);
    let violation_set: BTreeSet<(String, String)> = layer_violations.into_iter().collect();
    let focus_id = vis.focus_id.clone();
    let sel_edge_pair = vis.sel_edge_pair.clone();

    let is_dense = layout.nodes.len() >= 180 || layout.wires.len() >= 400;
    let tier_attr = if is_dense {
        " data-scale-tier=\"dense\""
    } else {
        ""
    };
    let mut html = format!(
        "<div class=\"graph-map-plane system-map-plane\" data-surface=\"{}\"{tier_attr} style=\"width:{:.0}px;height:{:.0}px\"><svg width=\"{:.0}\" height=\"{:.0}\" role=\"group\" aria-label=\"Links between services\"><defs>{}</defs>",
        surface.id,
        layout.width,
        layout.height,
        layout.width,
        layout.height,
        contract_arrow_defs_svg(surface.id_prefix),
    );

    // Architectural tier bands in Layers mode
    for (tier_label, bx, bw, bh) in &layout.tier_bands {
        let band_h = (bh - MARGIN + 12.0).max(120.0);
        html.push_str(&format!(
            "<g class=\"system-tier-band\"><rect class=\"graph-map-arch-band system-arch-band\" x=\"{bx:.1}\" y=\"{:.1}\" width=\"{bw:.1}\" height=\"{band_h:.1}\" rx=\"8\"/><text class=\"system-arch-label\" x=\"{:.1}\" y=\"{:.1}\">{}</text></g>",
            MARGIN - 4.0,
            bx + 10.0,
            MARGIN + 14.0,
            escape(tier_label),
        ));
    }

    // Unfolded service frames
    for frame in &layout.frames {
        html.push_str(&format!(
            "<g class=\"system-service-frame\" data-of=\"{}\"><rect class=\"graph-map-frame system-frame-rect\" x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" rx=\"10\"/><text class=\"system-frame-title\" x=\"{:.1}\" y=\"{:.1}\">{}</text></g>",
            escape(&frame.service_id),
            frame.x,
            frame.y,
            frame.w,
            frame.h,
            frame.x + 12.0,
            frame.y + 18.0,
            escape(&frame.title),
        ));
    }

    let hidden_kinds = hidden_contract_kinds(state);
    let mut placed_labels: Vec<(f64, f64)> = Vec::new();
    let mut edge_endpoints: Vec<(String, String)> = Vec::with_capacity(layout.wires.len());

    // Edges (halos + visible strokes + wide click/contextmenu hit targets when non-dense)
    for wire in &layout.wires {
        let Some(edge) = edges.get(wire.edge_index) else {
            edge_endpoints.push((String::new(), String::new()));
            continue;
        };
        let from = m::text(edge, "from");
        let to = m::text(edge, "to");
        edge_endpoints.push((from.clone(), to.clone()));
        if is_dense {
            continue;
        }
        let kind = primary_edge_kind(edge);
        let kind_idx = contract_spec_index(kind);
        let spec = spec_for_kind(kind);
        let prov = m::text(edge, "provenance");
        let broken = m::yes(&edge["broken_risk"]);
        let against = m::yes(&edge["against"])
            || wire.back
            || (layers_mode && violation_set.contains(&(from.clone(), to.clone())));
        let kind_hidden = hidden_kinds[kind_idx] && !against && !broken;
        let hidden_attr = if kind_hidden {
            " data-kind-hidden=\"true\""
        } else {
            ""
        };
        let is_selected = sel_edge_pair.0 == from && sel_edge_pair.1 == to && !from.is_empty();
        let is_highlighted = vis.highlight_edges.contains(&(from.clone(), to.clone()));

        if against {
            html.push_str(&format!(
                "<path class=\"graph-map-violation-halo system-map-violation-halo\" d=\"{}\" data-sys-edge-kind=\"{kind}\" data-from=\"{}\" data-to=\"{}\"{}/>",
                wire.path_d,
                escape(&from),
                escape(&to),
                if is_selected || is_highlighted { " data-edge-state=\"on\"" } else { "" }
            ));
        }
        if broken {
            html.push_str(&format!(
                "<path class=\"system-map-broken-halo\" d=\"{}\" data-sys-edge-kind=\"{kind}\" data-from=\"{}\" data-to=\"{}\"{hidden_attr}/>",
                wire.path_d,
                escape(&from),
                escape(&to),
            ));
        }

        let dash_attr = if prov == "undeclared_in_code" {
            " stroke-dasharray=\"4 3\"".to_owned()
        } else if let Some(d) = spec.dash_array {
            format!(" stroke-dasharray=\"{d}\"")
        } else {
            String::new()
        };
        // A link the change has not passed yet, at the wave shown.
        let dash_attr = if vis.beyond(&from) {
            format!("{dash_attr} data-beyond=\"true\"")
        } else {
            dash_attr
        };

        let marker_id = if against {
            format!("{}-arrow-against", surface.id_prefix)
        } else if broken {
            format!("{}-arrow-broken", surface.id_prefix)
        } else {
            format!("{}-arrow-{kind}", surface.id_prefix)
        };

        let mut edge_classes = format!("graph-map-edge system-map-edge system-map-edge-{kind}");
        if against {
            edge_classes.push_str(" graph-map-edge-against");
        }
        if is_selected || is_highlighted {
            edge_classes.push_str(" graph-map-edge-on");
        }

        html.push_str(&format!(
            "<path class=\"{edge_classes}\" d=\"{}\" data-sys-edge-kind=\"{kind}\" data-provenance=\"{}\" data-broken-risk=\"{}\" data-from=\"{}\" data-to=\"{}\"{dash_attr}{hidden_attr}{} marker-end=\"url(#{marker_id})\"/>",
            wire.path_d,
            escape(&prov),
            if broken { "true" } else { "false" },
            escape(&from),
            escape(&to),
            if is_highlighted { " data-shared-path=\"true\"" } else { "" },
        ));

        let contract_count = m::array(&edge["contracts"]).len();
        let pill_text = if broken {
            format!("! {}", spec.label)
        } else if prov == "undeclared_in_code" {
            "drift".to_owned()
        } else if contract_count > 1 {
            format!("{} · {contract_count}", spec.label)
        } else {
            spec.label.to_owned()
        };

        if !m::yes(&edge["child_link"]) {
            let (lx, ly) = place_sys_edge_label(&mut placed_labels, (wire.mid_x, wire.mid_y));
            html.push_str(&format!(
                "<g class=\"graph-map-edge-tie system-map-edge-label\" aria-hidden=\"true\" data-sys-edge-kind=\"{kind}\" data-from=\"{}\" data-to=\"{}\"{}{hidden_attr}><rect x=\"{:.1}\" y=\"{:.1}\" width=\"54\" height=\"16\" rx=\"8\"/><text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\">{}</text></g>",
                escape(&from),
                escape(&to),
                if is_selected || is_highlighted { " data-edge-state=\"on\"" } else { "" },
                lx - 27.0,
                ly - 8.0,
                lx,
                ly + 3.5,
                escape(&pill_text),
            ));
        }

        let explain_svcs = explain_services_for_endpoints(&from, &to, Some(&node_by_id));
        let hit_payload = payload_attr(&json!({
            "surface": surface.id,
            "from": from,
            "to": to,
            "kind": kind,
            "provenance": prov,
            "broken_risk": broken,
            "contracts": edge["contracts"],
        }));
        let aria = format!("{from} to {to} ({} link)", spec.label);
        html.push_str(&format!(
            "<path class=\"graph-map-hit system-map-hit\" d=\"{}\" tabindex=\"0\" role=\"button\" aria-label=\"{}\" data-action=\"system-map-edge\" data-sys-edge-kind=\"{kind}\" data-payload=\"{hit_payload}\" data-explain-services=\"{}\" data-from=\"{}\" data-to=\"{}\"{hidden_attr}><title>{}</title></path>",
            wire.path_d,
            escape(&aria),
            escape(&explain_svcs),
            escape(&from),
            escape(&to),
            escape(&aria),
        ));
    }

    html.push_str("</svg>");

    let pins = graph_viewport::load_pins(&sys_pin_key(surface, state));
    let cam = load_camera(surface);
    let (vw, vh, _) = viewport_size(surface.canvas_id);
    let visible_world = (cam.scale > 0.05).then(|| {
        (
            -cam.pan_x / cam.scale,
            -cam.pan_y / cam.scale,
            vw / cam.scale,
            vh / cam.scale,
        )
    });
    let active_roving_id = {
        let f = m::text(state, "roving_focus");
        if !f.is_empty() && layout.nodes.iter().any(|n| n.id == f) {
            f
        } else if !focus_id.is_empty() && layout.nodes.iter().any(|n| n.id == focus_id) {
            focus_id.clone()
        } else {
            layout
                .nodes
                .first()
                .map(|n| n.id.clone())
                .unwrap_or_default()
        }
    };
    let mut node_htmls = if is_dense {
        Vec::with_capacity(layout.nodes.len())
    } else {
        Vec::new()
    };
    let pinned_ids: BTreeSet<String> = pins.keys().cloned().collect();

    // Interactive service & unfolded child HTML node cards
    for (node_roving_idx, placed) in layout.nodes.iter().enumerate() {
        let Some(&node) = node_by_id.get(&placed.id) else {
            if is_dense {
                node_htmls.push(String::new());
            }
            continue;
        };
        let offscreen = visible_world.is_some_and(|(vx, vy, vw, vh)| {
            let pad = 120.0;
            placed.x + placed.w < vx - pad
                || placed.x > vx + vw + pad
                || placed.y + placed.h < vy - pad
                || placed.y > vy + vh + pad
        });
        let kind = node["kind"].as_str().unwrap_or("service");
        let name = {
            let n = m::text(node, "name");
            if n.is_empty() {
                placed.id.clone()
            } else {
                n
            }
        };
        let parent_svc = placed
            .child_of
            .clone()
            .unwrap_or_else(|| m::text(node, "service"));
        let svc_id = if parent_svc.is_empty() {
            placed.id.clone()
        } else {
            parent_svc
        };
        let is_selected = focus_id == placed.id;
        let is_highlighted = vis.highlight_nodes.contains(&placed.id);
        let is_pinned = pins.contains_key(&placed.id);
        let is_failed =
            vis.failed_services.contains(&placed.id) || vis.failed_services.contains(&svc_id);
        let is_roving = active_roving_id == placed.id;
        let is_important = is_selected || is_highlighted || is_pinned || is_failed || is_roving;
        let tabindex = if is_roving { "0" } else { "-1" };

        let waiting = vis.beyond(&placed.id);
        let blast_role = vis.blast_role(&placed.id);
        let step_state = vis.step_state(&placed.id);
        let unopened_warn = vis.unopened_consumers.contains(&placed.id);
        let graph_state = m::text(&node["graph_status"], "state");

        let payload = payload_attr(&json!({
            "surface": surface.id,
            "path": placed.id,
            "id": placed.id,
            "service": svc_id,
            "kind": kind,
        }));

        let child_extra = format!(
            "{}{}{} tabindex=\"{tabindex}\" data-roving-index=\"{node_roving_idx}\"",
            if is_pinned {
                " data-pinned=\"true\""
            } else {
                ""
            },
            if is_failed {
                " data-failed=\"true\""
            } else {
                ""
            },
            if offscreen && !is_important {
                " data-offscreen=\"true\""
            } else {
                ""
            },
        );

        let node_html = if placed.child_of.is_some() {
            let badge = match kind {
                "contract" => "C",
                "capability" => "Cap",
                "source" => "Src",
                _ => "•",
            };
            format!(
                "<button type=\"button\" class=\"graph-map-node system-map-node system-map-child\" data-n=\"{}\" data-kind=\"{}\" data-selected=\"{}\"{}{child_extra} style=\"left:{:.1}px;top:{:.1}px;width:{:.1}px;height:{:.1}px\" data-action=\"system-map-node\" data-payload=\"{payload}\" data-explain-services=\"{}\" title=\"{}\" aria-label=\"{}\"><span class=\"system-child-badge\">{}</span><span class=\"system-node-title\">{}</span></button>",
                escape(&placed.id),
                escape(kind),
                if is_selected { "true" } else { "false" },
                if is_highlighted {
                    " data-shared-path=\"true\""
                } else {
                    ""
                },
                placed.x,
                placed.y,
                placed.w,
                placed.h,
                escape(&svc_id),
                escape(&name),
                escape(&name),
                escape(badge),
                escape(&name),
            )
        } else {
            let mut meta_parts = Vec::new();
            let lang = m::text(node, "language");
            if !lang.is_empty() {
                meta_parts.push(lang);
            }
            let provides_n = m::array(&node["provides"]).len();
            if provides_n > 0 {
                meta_parts.push(format!("{provides_n} contracts"));
            } else if placed.id == "__system__" {
                let sh = m::array(&node["shared_sources"]).len();
                meta_parts.push(format!("{sh} shared docs"));
            }
            if !step_state.is_empty() {
                meta_parts.push(step_state.clone());
            } else if !blast_role.is_empty() {
                meta_parts.push(match blast_role {
                    "seed" => "seed".to_owned(),
                    "direct" => "wave 1".to_owned(),
                    _ => format!(
                        "wave {}",
                        vis.blast
                            .wave_by_service
                            .get(&placed.id)
                            .copied()
                            .unwrap_or(2)
                    ),
                });
            }
            let meta_line = meta_parts.join(" · ");

            let status_dot_state = if !step_state.is_empty() {
                step_state.as_str()
            } else if !graph_state.is_empty() {
                graph_state.as_str()
            } else {
                "idle"
            };

            let cycle_badge = if placed.cycle {
                "<span class=\"system-cycle-mark\" title=\"Participates in a cross-service dependency cycle\">↻</span>"
            } else {
                ""
            };
            let unopened_badge = if unopened_warn {
                "<span class=\"system-unopened-mark\" title=\"Unopened downstream contract consumer\">!</span>"
            } else {
                ""
            };

            let aria_label = if meta_line.is_empty() {
                name.clone()
            } else {
                format!("{name} ({meta_line})")
            };

            let node_extra = format!(
                "{}{}{} tabindex=\"{tabindex}\" data-roving-index=\"{node_roving_idx}\"",
                if is_pinned {
                    " data-pinned=\"true\""
                } else {
                    ""
                },
                if is_failed {
                    " data-failed=\"true\""
                } else {
                    ""
                },
                if offscreen && !is_important {
                    " data-offscreen=\"true\""
                } else {
                    ""
                },
            );

            format!(
                "<button type=\"button\" class=\"graph-map-node system-map-node\" data-n=\"{}\" data-kind=\"{}\" data-selected=\"{}\" data-blast=\"{}\" data-step-state=\"{}\" data-unopened=\"{}\" data-graph-state=\"{}\"{}{}{} style=\"left:{:.1}px;top:{:.1}px;width:{:.1}px;height:{:.1}px\" data-action=\"system-map-node\" data-payload=\"{payload}\" data-explain-services=\"{}\" title=\"{} — double-click to open service code map\" aria-label=\"{}\"><span class=\"system-node-top\"><span class=\"system-node-dot\" data-state=\"{}\" aria-hidden=\"true\"></span><span class=\"system-node-title\">{}</span>{cycle_badge}{unopened_badge}</span><span class=\"system-node-meta\">{}</span></button>",
                escape(&placed.id),
                escape(kind),
                if is_selected { "true" } else { "false" },
                blast_role,
                escape(&step_state),
                if unopened_warn { "true" } else { "false" },
                escape(&graph_state),
                node_extra,
                if is_highlighted {
                    " data-shared-path=\"true\""
                } else {
                    ""
                },
                if waiting {
                    " data-beyond=\"true\""
                } else {
                    ""
                },
                placed.x,
                placed.y,
                placed.w,
                placed.h,
                escape(&svc_id),
                escape(&name),
                escape(&aria_label),
                escape(status_dot_state),
                escape(&name),
                escape(&meta_line),
            )
        };
        if is_dense {
            node_htmls.push(node_html.clone());
        }
        if is_dense && offscreen && !is_important {
            continue;
        }
        html.push_str(&node_html);
    }

    html.push_str("</div>");
    SysRenderBundle {
        html,
        is_dense,
        node_htmls,
        edge_endpoints,
        active_roving_id,
        highlight_nodes: vis.highlight_nodes,
        failed_services: vis.failed_services,
        pinned_ids,
    }
}

/// Renders the rich `#system-map-side` or `#system-change-topology-side` inspector HTML.
pub fn render_side_html(surface: SysMapSurface, doc: &V, state: &V) -> String {
    let side_tab = match m::text(state, "side_tab").as_str() {
        "analyze" => "analyze".to_owned(),
        "changes" => "changes".to_owned(),
        _ => "details".to_owned(),
    };
    let nodes = m::array(&doc["nodes"]);
    let edges = m::array(&doc["edges"]);
    let services: Vec<V> = nodes
        .iter()
        .filter(|n| matches!(n["kind"].as_str(), Some("service" | "system") | None))
        .cloned()
        .collect();

    let mut html = String::new();
    html.push_str(
        "<div class=\"system-side-tabs\" role=\"tablist\" aria-label=\"Topology inspector views\">",
    );
    for (id, label) in [
        ("details", "Details"),
        ("analyze", "Analyze"),
        ("changes", "Changes & Impact"),
    ] {
        let selected = side_tab == id;
        let p = payload_attr(&json!({ "surface": surface.id, "tab": id }));
        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-pick-tab system-side-tab\" role=\"tab\" aria-selected=\"{}\" tabindex=\"{}\" data-action=\"system-map-side-tab\" data-payload=\"{p}\">{}</button>",
            if selected { "true" } else { "false" },
            if selected { "0" } else { "-1" },
            escape(label),
        ));
    }
    html.push_str("</div>");

    if side_tab == "analyze" {
        html.push_str(&render_analyze_tab_html(surface, doc, state, &services));
        return html;
    }
    if side_tab == "changes" {
        html.push_str(&render_changes_tab_html(surface, doc, state));
        return html;
    }

    // Details tab: selected edge -> selected node -> system overview
    if let Some(sel) = state["selected_edge"].as_object() {
        let from = sel.get("from").and_then(V::as_str).unwrap_or("");
        let to = sel.get("to").and_then(V::as_str).unwrap_or("");
        if let Some(edge) = edges
            .iter()
            .find(|e| m::text(e, "from") == from && m::text(e, "to") == to)
        {
            html.push_str(&render_edge_details_html(surface, doc, edge));
            return html;
        }
    }

    let focus_id = m::text(state, "focus");
    if !focus_id.is_empty() {
        if let Some(node) = nodes.iter().find(|n| node_id(n) == focus_id) {
            html.push_str(&render_node_details_html(surface, doc, state, node));
            return html;
        }
    }

    html.push_str(&render_overview_side_html(surface, doc, state, &services));
    html
}

fn render_overview_side_html(surface: SysMapSurface, doc: &V, state: &V, services: &[V]) -> String {
    let edges = m::array(&doc["edges"]);
    let cycles = find_cross_service_cycles(doc);
    let broken_count = edges.iter().filter(|e| m::yes(&e["broken_risk"])).count();
    let drift_count = edges
        .iter()
        .filter(|e| m::text(e, "provenance") == "undeclared_in_code")
        .count();

    let mut html = String::from("<div class=\"system-side-section\">");
    html.push_str("<h4 class=\"system-side-heading\">System architecture</h4>");
    html.push_str("<p class=\"system-note\">Select a service or contract arrow to inspect schemas, provenance, and blast radius. Double-click any service to open its code map.</p>");

    html.push_str("<dl class=\"memory-metrics system-side-metrics\">");
    html.push_str(&format!(
        "<dt>Member services</dt><dd>{}</dd>",
        services
            .iter()
            .filter(|n| node_id(n) != "__system__")
            .count()
    ));
    html.push_str(&format!(
        "<dt>Cross-service links</dt><dd>{}</dd>",
        edges.iter().filter(|e| !m::yes(&e["child_link"])).count()
    ));
    html.push_str(&format!(
        "<dt>Broken / version risks</dt><dd data-zero=\"{}\">{broken_count}</dd>",
        if broken_count == 0 { "true" } else { "false" }
    ));
    html.push_str(&format!(
        "<dt>Undeclared code drift</dt><dd data-zero=\"{}\">{drift_count}</dd>",
        if drift_count == 0 { "true" } else { "false" }
    ));
    html.push_str(&format!(
        "<dt>Cross-service cycles</dt><dd data-zero=\"{}\">{}</dd>",
        if cycles.is_empty() { "true" } else { "false" },
        cycles.len()
    ));
    html.push_str("</dl>");

    let overlay = &state["overlay"];
    let unopened = m::array(&overlay["cross_service_unopened"]);
    if !unopened.is_empty() {
        html.push_str("<div class=\"system-side-callout system-side-callout-warn\"><strong>Unopened contract consumers</strong><ul class=\"system-side-list\">");
        for item in unopened.iter().take(8) {
            let label = item.as_str().map(str::to_owned).unwrap_or_else(|| {
                format!(
                    "{} (consumes {})",
                    first_non_empty(item, &["consumer_service", "service", "consumer"]),
                    first_non_empty(item, &["contract", "name"])
                )
            });
            html.push_str(&format!("<li>{}</li>", escape(&label)));
        }
        html.push_str("</ul></div>");
    }

    if !cycles.is_empty() {
        let p = payload_attr(&json!({ "surface": surface.id, "tool": "cycle", "index": 0 }));
        html.push_str(&format!(
            "<div class=\"system-side-actions\"><button type=\"button\" class=\"button compact\" data-action=\"system-map-tool\" data-payload=\"{p}\">Show cycle ↻ ({})</button></div>",
            cycles.len()
        ));
    }

    html.push_str("</div>");
    html
}

fn render_node_details_html(surface: SysMapSurface, doc: &V, state: &V, node: &V) -> String {
    let id = node_id(node);
    let svc_id = {
        let p = first_non_empty(node, &["parent_service", "service"]);
        if p.is_empty() {
            id.clone()
        } else {
            p
        }
    };
    let name = m::text(node, "name");
    let kind = node["kind"].as_str().unwrap_or("service");
    let spread_list = m::array(&state["spread"]);
    let is_spread = spread_list
        .iter()
        .any(|v| v.as_str() == Some(svc_id.as_str()));

    let mut html = String::from("<div class=\"system-side-section\">");
    let clear_p = payload_attr(&json!({ "surface": surface.id }));
    html.push_str(&format!(
        "<div class=\"system-side-head\"><div><span class=\"system-side-kicker\">{}</span><h4 class=\"system-side-heading\">{}</h4></div><button type=\"button\" class=\"button compact\" data-action=\"system-map-clear-selection\" data-payload=\"{clear_p}\">Clear</button></div>",
        escape(kind),
        escape(if name.is_empty() { &id } else { &name }),
    ));

    let open_p = payload_attr(&json!({ "surface": surface.id, "service": svc_id }));
    let spread_p = payload_attr(&json!({ "surface": surface.id, "service": svc_id }));
    let changes_p = payload_attr(&json!({ "surface": surface.id, "service": svc_id }));
    html.push_str("<div class=\"system-side-actions\">");
    html.push_str(&format!(
        "<button type=\"button\" class=\"button compact primary\" data-action=\"system-map-open-service\" data-payload=\"{open_p}\">Open service code map</button>"
    ));
    html.push_str(&format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"system-map-spread\" data-payload=\"{spread_p}\">{}</button>",
        if is_spread {
            "Fold contracts"
        } else {
            "Unfold contracts & sources"
        }
    ));
    html.push_str(&format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"system-map-service-changes\" data-payload=\"{changes_p}\">Changes on map</button>"
    ));
    html.push_str("</div>");

    html.push_str("<dl class=\"memory-metrics system-side-metrics\">");
    html.push_str(&format!("<dt>Service ID</dt><dd>{}</dd>", escape(&svc_id)));
    let owner = m::text(node, "owner");
    if !owner.is_empty() {
        html.push_str(&format!("<dt>Owner</dt><dd>{}</dd>", escape(&owner)));
    }
    let folder = m::text(node, "folder");
    if !folder.is_empty() {
        html.push_str(&format!(
            "<dt>Folder</dt><dd title=\"{}\">{}</dd>",
            escape(&folder),
            escape(folder.rsplit('/').next().unwrap_or(&folder))
        ));
    }
    let lang = m::text(node, "language");
    if !lang.is_empty() {
        html.push_str(&format!("<dt>Stack</dt><dd>{}</dd>", escape(&lang)));
    }
    let g_state = m::text(&node["graph_status"], "state");
    if !g_state.is_empty() {
        html.push_str(&format!("<dt>Code graph</dt><dd>{}</dd>", escape(&g_state)));
    }
    let mem_count = node["memory"]["reviewed_count"]
        .as_u64()
        .or_else(|| node["memory"]["count"].as_u64());
    if let Some(mc) = mem_count {
        html.push_str(&format!(
            "<dt>Memory Bank chunks</dt><dd data-zero=\"{}\">{mc}</dd>",
            if mc == 0 { "true" } else { "false" }
        ));
    }
    html.push_str("</dl>");

    let caps = m::array(&node["capabilities"]);
    if !caps.is_empty() {
        html.push_str(
            "<h5 class=\"system-side-subheading\">Capabilities</h5><div class=\"system-chip-row\">",
        );
        for cap in caps.iter().take(12) {
            let label = capability_name(cap);
            if !label.is_empty() {
                html.push_str(&format!(
                    "<span class=\"system-chip\">{}</span>",
                    escape(&label)
                ));
            }
        }
        html.push_str("</div>");
    }

    let provides = m::array(&node["provides"]);
    if !provides.is_empty() {
        html.push_str("<h5 class=\"system-side-subheading\">Provided contracts</h5><ul class=\"system-side-list\">");
        for c in &provides {
            let cname = contract_name(c);
            let ckind = normalize_contract_arrow_kind(&m::text(c, "kind"));
            let ver = m::text(c, "version");
            let impact_p = payload_attr(&json!({
                "surface": surface.id,
                "service": svc_id,
                "contract": cname,
            }));
            html.push_str(&format!(
                "<li class=\"system-contract-row\"><span><strong>{}</strong> <span class=\"system-chip\">{ckind}</span>{}</span><button type=\"button\" class=\"button compact\" data-action=\"system-map-contract-impact\" data-payload=\"{impact_p}\">Impact</button></li>",
                escape(&cname),
                if ver.is_empty() { String::new() } else { format!(" <small>v{}</small>", escape(&ver)) },
            ));
        }
        html.push_str("</ul>");
    }

    let consumes = m::array(&node["consumes"]);
    if !consumes.is_empty() {
        html.push_str("<h5 class=\"system-side-subheading\">Consumed contracts</h5><ul class=\"system-side-list\">");
        for c in &consumes {
            let cname = contract_name(c);
            let ckind = normalize_contract_arrow_kind(&m::text(c, "kind"));
            html.push_str(&format!(
                "<li><strong>{}</strong> <span class=\"system-chip\">{ckind}</span></li>",
                escape(&cname)
            ));
        }
        html.push_str("</ul>");
    }

    // Quick analytical launchers for this service
    let follow_out = payload_attr(
        &json!({ "surface": surface.id, "tool": "follow", "service": svc_id, "dir": "out" }),
    );
    let follow_into = payload_attr(
        &json!({ "surface": surface.id, "tool": "follow", "service": svc_id, "dir": "into" }),
    );
    let path_start =
        payload_attr(&json!({ "surface": surface.id, "tool": "path", "from": svc_id }));
    let share_start =
        payload_attr(&json!({ "surface": surface.id, "tool": "shared", "a": svc_id }));

    html.push_str("<h5 class=\"system-side-subheading\">Explore links</h5><div class=\"system-side-actions\">");
    html.push_str(&format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"system-map-tool\" data-payload=\"{follow_out}\">Upstream providers</button>"
    ));
    html.push_str(&format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"system-map-tool\" data-payload=\"{follow_into}\">Downstream consumers</button>"
    ));
    html.push_str(&format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"system-map-tool\" data-payload=\"{path_start}\">Path from here…</button>"
    ));
    html.push_str(&format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"system-map-tool\" data-payload=\"{share_start}\">Compare shared…</button>"
    ));

    let cycles = find_cross_service_cycles(doc);
    if let Some(cidx) = cycles.iter().position(|comp| comp.contains(&svc_id)) {
        let cyc_p = payload_attr(&json!({ "surface": surface.id, "tool": "cycle", "index": cidx }));
        html.push_str(&format!(
            "<button type=\"button\" class=\"button compact\" data-action=\"system-map-tool\" data-payload=\"{cyc_p}\">Show cycle ↻</button>"
        ));
    }
    html.push_str("</div></div>");
    html
}

/// Where the code on both sides of the selected link is kept for a picture.
fn bridge_key(surface: SysMapSurface) -> String {
    format!("{}-bridge", surface.id_prefix)
}
/// The code on both sides of a link between two services: the files of the
/// consumer that name what it uses, the files of the provider that make it,
/// and how far a change of each reaches inside its own service. Asked for
/// on demand: until then, a button.
pub fn bridge_html(surface: SysMapSurface, from: &str, to: &str, bridge: &V) -> String {
    let mut html = String::from("<h5 class=\"system-side-subheading\">Code on both sides</h5>");
    if bridge["from"].as_str() != Some(from) || bridge["to"].as_str() != Some(to) {
        let asked = payload_attr(&json!({"from": from, "surface": surface.id, "to": to}));
        html.push_str(&format!(
            "<p class=\"system-note\">The files of {} that name what it uses of {}, and those of {} that make it.</p><div class=\"system-side-actions\"><button type=\"button\" class=\"button compact\" data-action=\"system-map-bridge\" data-payload=\"{asked}\">Show the code</button></div>",
            escape(from),
            escape(to),
            escape(to),
        ));
        return html;
    }
    for (side, role, none) in [
        ("consumer", "uses it", "No file was found to name it."),
        (
            "provider",
            "makes it",
            "Its passport names no file, and none was found to name it.",
        ),
    ] {
        let part = &bridge[side];
        let service = m::text(part, "service");
        let sites = m::array(&part["sites"]);
        html.push_str(&format!(
            "<p class=\"system-note\"><strong>{}</strong> {role}{}</p>",
            escape(&service),
            if m::yes(&part["graph"]) {
                ""
            } else {
                " · no code graph yet: how far each file reaches is not told"
            },
        ));
        if sites.is_empty() {
            html.push_str(&format!("<p class=\"system-note\">{none}</p>"));
            continue;
        }
        html.push_str("<ul class=\"system-side-list system-bridge-sites\">");
        for site in &sites {
            let path = m::text(site, "path");
            let how = match (
                side == "provider" && m::yes(&site["declared"]),
                m::yes(&site["found"]),
                m::yes(&site["document"]),
            ) {
                (true, true, _) => "made here by its passport, and names it",
                (true, false, _) => "made here by its passport",
                (false, _, true) => "a document that names it",
                _ => "names it",
            };
            let reach = match site["reach"]["files"].as_u64() {
                Some(0) => " · nothing else in it uses this file".to_owned(),
                Some(files) => format!(
                    " · a change reaches {} of {}",
                    if files == 1 {
                        "1 file".to_owned()
                    } else {
                        format!("{files} files")
                    },
                    escape(&service)
                ),
                None => String::new(),
            };
            let open = payload_attr(&json!({"path": path, "service": service}));
            html.push_str(&format!(
                "<li class=\"system-bridge-site\"><div><code>{}</code><div class=\"system-note\">{how}{}{reach}</div></div><button type=\"button\" class=\"button compact\" data-action=\"inspect-system-file-graph\" data-payload=\"{open}\" aria-label=\"Show {} on the code map of {}\">Map</button></li>",
                escape(&path),
                {
                    let named = m::array(&site["contracts"])
                        .iter()
                        .filter_map(|c| c.as_str().map(escape))
                        .collect::<Vec<_>>()
                        .join(", ");
                    if named.is_empty() {
                        String::new()
                    } else {
                        format!(": {named}")
                    }
                },
                escape(&path),
                escape(&service),
            ));
        }
        html.push_str("</ul>");
        let more = part["more"].as_u64().unwrap_or(0);
        if more > 0 {
            html.push_str(&format!("<p class=\"system-note\">and {more} more</p>"));
        }
        if m::yes(&part["cut"]) {
            html.push_str(
                "<p class=\"system-note\">Not every file of it was read: there may be more.</p>",
            );
        }
    }
    html.push_str(&semantic_bridges_html(
        from,
        to,
        &bridge["semantic_bridges"],
    ));
    html
}

fn join_text_array(value: &V) -> String {
    m::array(value)
        .iter()
        .filter_map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn semantic_bridge_file_buttons(service: &str, files: &V) -> String {
    let mut out = String::new();
    for item in m::array(files).into_iter().take(6) {
        let Some(path) = item.as_str().map(str::trim).filter(|p| !p.is_empty()) else {
            continue;
        };
        let open = payload_attr(&json!({"path": path, "service": service}));
        out.push_str(&format!(
            "<button type=\"button\" class=\"system-semantic-file\" data-action=\"inspect-system-file-graph\" data-payload=\"{open}\" title=\"Show {} on the code map of {}\"><code>{}</code></button>",
            escape(path),
            escape(service),
            escape(path),
        ));
    }
    out
}

fn wrap_semantic_files(buttons: String) -> String {
    if buttons.is_empty() {
        String::new()
    } else {
        format!("<div class=\"system-semantic-files\">{buttons}</div>")
    }
}

fn semantic_bridges_html(from: &str, to: &str, sem: &V) -> String {
    if !sem.is_object() {
        return String::new();
    }
    let routes = m::array(&sem["routes"]);
    let events = m::array(&sem["events"]);
    let di_bindings = m::array(&sem["di_bindings"]);
    let tables = m::array(&sem["tables"]);
    let total = routes.len() + events.len() + di_bindings.len() + tables.len();
    if total == 0 {
        return "<details class=\"system-semantic-bridges\"><summary class=\"system-semantic-bridges-summary\"><span>Semantic framework &amp; DB bridges</span><span class=\"system-chip\">0</span></summary><p class=\"system-note\">No shared routes, events, DI bindings, or tables detected.</p></details>".to_owned();
    }
    let mut body = String::new();
    if !routes.is_empty() {
        body.push_str(&format!(
            "<h6 class=\"system-semantic-group-title\">Routes <span class=\"system-chip\">{}</span></h6><ul class=\"system-semantic-list\">",
            routes.len()
        ));
        for r in &routes {
            let route = m::text(r, "route");
            let methods = {
                let joined = join_text_array(&r["methods"]);
                if !joined.is_empty() {
                    joined
                } else {
                    let single = m::text(r, "method");
                    if single.is_empty() {
                        "ANY".to_owned()
                    } else {
                        single
                    }
                }
            };
            let caller = {
                let c = m::text(r, "caller");
                if c.is_empty() {
                    from.to_owned()
                } else {
                    c
                }
            };
            let provider = {
                let p = m::text(r, "provider");
                if p.is_empty() {
                    to.to_owned()
                } else {
                    p
                }
            };
            let files = format!(
                "{}{}",
                semantic_bridge_file_buttons(&caller, &r["caller_files"]),
                semantic_bridge_file_buttons(&provider, &r["provider_files"]),
            );
            body.push_str(&format!(
                "<li class=\"system-semantic-item\" data-kind=\"route\"><div class=\"system-semantic-item-head\"><span class=\"system-semantic-badge\" data-kind=\"route\">{}</span><code>{}</code></div><div class=\"system-note\">{} → {}</div>{}</li>",
                escape(&methods),
                escape(&route),
                escape(&caller),
                escape(&provider),
                wrap_semantic_files(files),
            ));
        }
        body.push_str("</ul>");
    }
    if !events.is_empty() {
        body.push_str(&format!(
            "<h6 class=\"system-semantic-group-title\">Events <span class=\"system-chip\">{}</span></h6><ul class=\"system-semantic-list\">",
            events.len()
        ));
        for ev in &events {
            let event_name = m::text(ev, "event");
            let producer = {
                let p = m::text(ev, "producer");
                if p.is_empty() {
                    from.to_owned()
                } else {
                    p
                }
            };
            let consumer = {
                let c = m::text(ev, "consumer");
                if c.is_empty() {
                    to.to_owned()
                } else {
                    c
                }
            };
            let files = format!(
                "{}{}",
                semantic_bridge_file_buttons(&producer, &ev["producer_files"]),
                semantic_bridge_file_buttons(&consumer, &ev["consumer_files"]),
            );
            body.push_str(&format!(
                "<li class=\"system-semantic-item\" data-kind=\"event\"><div class=\"system-semantic-item-head\"><span class=\"system-semantic-badge\" data-kind=\"event\">event</span><code>{}</code></div><div class=\"system-note\">{} → {}</div>{}</li>",
                escape(&event_name),
                escape(&producer),
                escape(&consumer),
                wrap_semantic_files(files),
            ));
        }
        body.push_str("</ul>");
    }
    if !di_bindings.is_empty() {
        body.push_str(&format!(
            "<h6 class=\"system-semantic-group-title\">DI bindings <span class=\"system-chip\">{}</span></h6><ul class=\"system-semantic-list\">",
            di_bindings.len()
        ));
        for di in &di_bindings {
            let service = m::text(di, "service");
            let from_kinds = join_text_array(&di["from_kinds"]);
            let to_kinds = join_text_array(&di["to_kinds"]);
            let left_note = if from_kinds.is_empty() {
                escape(from)
            } else {
                format!("{}: {}", escape(from), escape(&from_kinds))
            };
            let right_note = if to_kinds.is_empty() {
                escape(to)
            } else {
                format!("{}: {}", escape(to), escape(&to_kinds))
            };
            let files = format!(
                "{}{}",
                semantic_bridge_file_buttons(from, &di["from_files"]),
                semantic_bridge_file_buttons(to, &di["to_files"]),
            );
            body.push_str(&format!(
                "<li class=\"system-semantic-item\" data-kind=\"di\"><div class=\"system-semantic-item-head\"><span class=\"system-semantic-badge\" data-kind=\"di\">DI</span><code>{}</code></div><div class=\"system-note\">{left_note} ↔ {right_note}</div>{}</li>",
                escape(&service),
                wrap_semantic_files(files),
            ));
        }
        body.push_str("</ul>");
    }
    if !tables.is_empty() {
        body.push_str(&format!(
            "<h6 class=\"system-semantic-group-title\">Shared DB tables <span class=\"system-chip\">{}</span></h6><ul class=\"system-semantic-list\">",
            tables.len()
        ));
        for tbl in &tables {
            let table_name = m::text(tbl, "table");
            let coupling_raw = m::text(tbl, "coupling");
            let (coupling_attr, coupling_label) = match coupling_raw.as_str() {
                "shared_write" => ("shared_write", "shared write"),
                "cross_read_write" => ("cross_read_write", "read / write"),
                "shared_read" => ("shared_read", "shared read"),
                _ => ("shared_read", "shared"),
            };
            let from_access = join_text_array(&tbl["from_access"]);
            let to_access = join_text_array(&tbl["to_access"]);
            let left_note = if from_access.is_empty() {
                escape(from)
            } else {
                format!("{} ({})", escape(from), escape(&from_access))
            };
            let right_note = if to_access.is_empty() {
                escape(to)
            } else {
                format!("{} ({})", escape(to), escape(&to_access))
            };
            let files = format!(
                "{}{}",
                semantic_bridge_file_buttons(from, &tbl["from_files"]),
                semantic_bridge_file_buttons(to, &tbl["to_files"]),
            );
            body.push_str(&format!(
                "<li class=\"system-semantic-item\" data-kind=\"table\"><div class=\"system-semantic-item-head\"><span class=\"system-semantic-badge\" data-kind=\"table\" data-coupling=\"{coupling_attr}\">{coupling_label}</span><code>{}</code></div><div class=\"system-note\">{left_note} · {right_note}</div>{}</li>",
                escape(&table_name),
                wrap_semantic_files(files),
            ));
        }
        body.push_str("</ul>");
    }
    format!(
        "<details class=\"system-semantic-bridges\" open><summary class=\"system-semantic-bridges-summary\"><span>Semantic framework &amp; DB bridges</span><span class=\"system-chip\">{total}</span></summary><div class=\"system-semantic-bridges-body\">{body}</div></details>"
    )
}
/// Asks for the code on both sides of a link and draws it beside the map.
async fn fetch_bridge(surface: SysMapSurface, from: String, to: String) {
    let state = surface_state(surface);
    let project_id = m::text(&state, "project_id");
    if project_id.is_empty() {
        return;
    }
    let q = dom::query(&[
        ("project_id", project_id),
        ("config_path", m::text(&state, "config_path")),
        ("from", from),
        ("to", to),
        (
            "certain",
            if m::yes(&state["certain"]) {
                "1".to_owned()
            } else {
                String::new()
            },
        ),
    ]);
    if let Ok(doc) = api::get(&format!("/api/systems/graph/bridge{q}")).await {
        m::set(&bridge_key(surface), doc);
        render_surface(surface, false);
    }
}
fn render_edge_details_html(surface: SysMapSurface, doc: &V, edge: &V) -> String {
    let from = m::text(edge, "from");
    let to = m::text(edge, "to");
    let nodes = m::array(&doc["nodes"]);
    let node_by_id: BTreeMap<String, &V> = nodes.iter().map(|n| (node_id(n), n)).collect();
    let from_svc = endpoint_service_id(&from, Some(&node_by_id)).unwrap_or_else(|| from.clone());
    let to_svc = endpoint_service_id(&to, Some(&node_by_id)).unwrap_or_else(|| to.clone());
    let kind = primary_edge_kind(edge);
    let prov = m::text(edge, "provenance");
    let broken = m::yes(&edge["broken_risk"]);
    let against = m::yes(&edge["against"]);
    let clear_p = payload_attr(&json!({ "surface": surface.id }));

    let mut html = String::from("<div class=\"system-side-section\">");
    html.push_str(&format!(
        "<div class=\"system-side-head\"><div><span class=\"system-side-kicker\">Contract link · {kind}</span><h4 class=\"system-side-heading\">{} → {}</h4></div><button type=\"button\" class=\"button compact\" data-action=\"system-map-clear-selection\" data-payload=\"{clear_p}\">Clear</button></div>",
        escape(&from),
        escape(&to),
    ));

    if broken {
        html.push_str("<p class=\"system-side-callout system-side-callout-warn\"><strong>Broken contract risk:</strong> Consumer contract has no matching provider definition or has an incompatible version.</p>");
    }
    if prov == "undeclared_in_code" {
        html.push_str("<p class=\"system-side-callout system-side-callout-warn\"><strong>Undeclared drift:</strong> Code references cross these service folders without a declared contract in the system manifest.</p>");
    }
    if against {
        html.push_str("<p class=\"system-side-callout system-side-callout-warn\"><strong>Layer violation:</strong> This link runs upward against architectural tiers or closes a cross-service cycle.</p>");
    }

    html.push_str("<dl class=\"memory-metrics system-side-metrics\">");
    html.push_str(&format!("<dt>Consumer</dt><dd>{}</dd>", escape(&from)));
    html.push_str(&format!("<dt>Provider</dt><dd>{}</dd>", escape(&to)));
    html.push_str(&format!("<dt>Primary transport</dt><dd>{kind}</dd>"));
    html.push_str(&format!(
        "<dt>Provenance</dt><dd>{}</dd>",
        escape(if prov.is_empty() { "declared" } else { &prov })
    ));
    html.push_str("</dl>");

    let contracts = m::array(&edge["contracts"]);
    if !contracts.is_empty() {
        html.push_str("<h5 class=\"system-side-subheading\">Contracts on this link</h5><ul class=\"system-side-list\">");
        for c in &contracts {
            let cname = contract_name(c);
            let ckind = normalize_contract_arrow_kind(&m::text(c, "kind"));
            let ver = {
                let direct = m::text(c, "version");
                if !direct.is_empty() {
                    direct
                } else {
                    let pv = m::text(c, "provider_version");
                    let cv = m::text(c, "consumer_version");
                    if !cv.is_empty() && !pv.is_empty() && cv != pv {
                        format!("{cv} → {pv}")
                    } else if !pv.is_empty() {
                        pv
                    } else {
                        cv
                    }
                }
            };
            let src = {
                let direct = first_non_empty(c, &["source", "schema", "provider_source"]);
                if !direct.is_empty() {
                    direct
                } else {
                    m::array(&c["provider_sources"])
                        .into_iter()
                        .chain(m::array(&c["consumer_sources"]))
                        .find_map(|v| {
                            v.as_str()
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .map(str::to_owned)
                        })
                        .unwrap_or_default()
                }
            };
            let impact_p = payload_attr(&json!({
                "surface": surface.id,
                "service": to_svc,
                "contract": cname,
            }));
            html.push_str(&format!(
                "<li class=\"system-contract-row\"><div><strong>{}</strong> <span class=\"system-chip\">{ckind}</span>{}{}</div><button type=\"button\" class=\"button compact\" data-action=\"system-map-contract-impact\" data-payload=\"{impact_p}\">Impact</button></li>",
                escape(&cname),
                if ver.is_empty() { String::new() } else { format!(" <small>v{}</small>", escape(&ver)) },
                if src.is_empty() { String::new() } else { format!("<div class=\"system-note\">{}</div>", escape(&src)) },
            ));
        }
        html.push_str("</ul>");
    }

    html.push_str(&bridge_html(
        surface,
        &from_svc,
        &to_svc,
        &m::data(&bridge_key(surface)),
    ));

    let open_from = payload_attr(&json!({ "surface": surface.id, "service": from_svc }));
    let open_to = payload_attr(&json!({ "surface": surface.id, "service": to_svc }));
    html.push_str("<div class=\"system-side-actions\">");
    html.push_str(&format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"system-map-open-service\" data-payload=\"{open_from}\">Open {} map</button>",
        escape(&from_svc)
    ));
    html.push_str(&format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"system-map-open-service\" data-payload=\"{open_to}\">Open {} map</button>",
        escape(&to_svc)
    ));
    html.push_str("</div></div>");
    html
}

fn render_analyze_tab_html(surface: SysMapSurface, doc: &V, state: &V, services: &[V]) -> String {
    let tool = {
        let t = m::text(state, "tool");
        if t.is_empty() {
            "path".to_owned()
        } else {
            t
        }
    };

    let mut html = String::from("<div class=\"system-side-section\">");
    html.push_str(
        "<div class=\"system-side-actions\" role=\"group\" aria-label=\"Topology analysis tool\">",
    );
    for (id, label) in [
        ("follow", "Follow links"),
        ("path", "Path"),
        ("shared", "What both share"),
        ("cycle", "Cycles ↻"),
        ("layers", "Layers"),
    ] {
        let p = payload_attr(&json!({ "surface": surface.id, "tool": id }));
        html.push_str(&format!(
            "<button type=\"button\" class=\"button compact\" aria-pressed=\"{}\" data-action=\"system-map-tool\" data-payload=\"{p}\">{}</button>",
            if tool == id { "true" } else { "false" },
            escape(label),
        ));
    }
    html.push_str("</div>");

    let service_options = |selected: &str| -> String {
        let mut opts = String::from("<option value=\"\">Select service…</option>");
        for s in services {
            let sid = node_id(s);
            let sname = m::text(s, "name");
            opts.push_str(&format!(
                "<option value=\"{}\"{}>{}</option>",
                escape(&sid),
                if sid == selected { " selected" } else { "" },
                escape(if sname.is_empty() { &sid } else { &sname }),
            ));
        }
        opts
    };

    match tool.as_str() {
        "follow" => {
            let root = m::text(state, "follow");
            let dir = m::text(state, "follow_dir");
            html.push_str(&format!(
                "<label class=\"field\">Starting service<select data-action=\"system-map-follow-root\" data-surface=\"{}\">{}</select></label>",
                surface.id,
                service_options(&root)
            ));
            let p_out = payload_attr(
                &json!({ "surface": surface.id, "tool": "follow", "service": root, "dir": "out" }),
            );
            let p_into = payload_attr(
                &json!({ "surface": surface.id, "tool": "follow", "service": root, "dir": "into" }),
            );
            html.push_str(&format!(
                "<div class=\"system-side-actions\"><button type=\"button\" class=\"button compact\" aria-pressed=\"{}\" data-action=\"system-map-tool\" data-payload=\"{p_out}\">Upstream providers</button><button type=\"button\" class=\"button compact\" aria-pressed=\"{}\" data-action=\"system-map-tool\" data-payload=\"{p_into}\">Downstream consumers</button></div>",
                if dir != "into" { "true" } else { "false" },
                if dir == "into" { "true" } else { "false" },
            ));
            if !root.is_empty() {
                let key = if dir == "into" { "into" } else { "out" };
                let expanded: BTreeSet<String> = m::array(&state[key])
                    .into_iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .chain(std::iter::once(root.clone()))
                    .collect();
                html.push_str("<ul class=\"system-side-list\">");
                for e in m::array(&doc["edges"])
                    .iter()
                    .filter(|e| !m::yes(&e["child_link"]))
                {
                    let f = m::text(e, "from");
                    let t = m::text(e, "to");
                    let next = if dir == "into" && expanded.contains(&t) {
                        Some((&t, &f))
                    } else if dir != "into" && expanded.contains(&f) {
                        Some((&f, &t))
                    } else {
                        None
                    };
                    if let Some((src, dst)) = next {
                        let step_p = payload_attr(&json!({
                            "surface": surface.id,
                            "dir": dir,
                            "hop": dst,
                        }));
                        html.push_str(&format!(
                            "<li class=\"system-contract-row\"><span>{} → <strong>{}</strong></span><button type=\"button\" class=\"button compact\" data-action=\"system-map-follow-hop\" data-payload=\"{step_p}\">{}</button></li>",
                            escape(src),
                            escape(dst),
                            if expanded.contains(dst) { "Fold" } else { "Step further" },
                        ));
                    }
                }
                html.push_str("</ul>");
            }
        }
        "shared" => {
            let sa = m::text(state, "shared_a");
            let sb = m::text(state, "shared_b");
            let side = m::text(state, "shared_side");
            html.push_str(&format!(
                "<label class=\"field\">First service<select data-action=\"system-map-shared-a\" data-surface=\"{}\">{}</select></label><label class=\"field\">Second service<select data-action=\"system-map-shared-b\" data-surface=\"{}\">{}</select></label>",
                surface.id,
                service_options(&sa),
                surface.id,
                service_options(&sb),
            ));
            let p_bu = payload_attr(&json!({ "surface": surface.id, "side": "both_use" }));
            let p_ub = payload_attr(&json!({ "surface": surface.id, "side": "uses_both" }));
            html.push_str(&format!(
                "<div class=\"system-side-actions\"><button type=\"button\" class=\"button compact\" aria-pressed=\"{}\" data-action=\"system-map-shared-side\" data-payload=\"{p_bu}\">What both depend on</button><button type=\"button\" class=\"button compact\" aria-pressed=\"{}\" data-action=\"system-map-shared-side\" data-payload=\"{p_ub}\">What depends on both</button></div>",
                if side != "uses_both" { "true" } else { "false" },
                if side == "uses_both" { "true" } else { "false" },
            ));
            if !sa.is_empty() && !sb.is_empty() {
                let (both_use, uses_both) = shared_neighbors(doc, &sa, &sb);
                let items = if side == "uses_both" {
                    uses_both
                } else {
                    both_use
                };
                if items.is_empty() {
                    html.push_str(
                        "<p class=\"system-note\">No shared services in this direction.</p>",
                    );
                } else {
                    html.push_str("<ul class=\"system-side-list\">");
                    for item in items {
                        let p = payload_attr(&json!({ "surface": surface.id, "path": item }));
                        html.push_str(&format!(
                            "<li><button type=\"button\" class=\"button compact\" data-action=\"system-map-node\" data-payload=\"{p}\">{}</button></li>",
                            escape(&item)
                        ));
                    }
                    html.push_str("</ul>");
                }
            }
        }
        "cycle" => {
            let cycles = find_cross_service_cycles(doc);
            if cycles.is_empty() {
                html.push_str(
                    "<p class=\"system-note\">No cross-service dependency cycles detected.</p>",
                );
            } else {
                let active_idx = state["cycle_index"].as_u64().unwrap_or(0) as usize;
                html.push_str("<ul class=\"system-side-list\">");
                for (idx, comp) in cycles.iter().enumerate() {
                    let p = payload_attr(
                        &json!({ "surface": surface.id, "tool": "cycle", "index": idx }),
                    );
                    html.push_str(&format!(
                        "<li class=\"system-contract-row\"><span>↻ {}</span><button type=\"button\" class=\"button compact\" aria-pressed=\"{}\" data-action=\"system-map-tool\" data-payload=\"{p}\">Highlight</button></li>",
                        escape(&comp.join(" ↔ ")),
                        if idx == active_idx { "true" } else { "false" },
                    ));
                }
                html.push_str("</ul>");
            }
        }
        "layers" => {
            let (tiers, violations) = assign_service_layers(doc);
            let toggle_p = payload_attr(&json!({ "surface": surface.id }));
            html.push_str(&format!(
                "<div class=\"system-side-actions\"><button type=\"button\" class=\"button compact\" aria-pressed=\"{}\" data-action=\"system-map-toggle-layers\" data-payload=\"{toggle_p}\">Arrange canvas by tiers</button></div>",
                if m::yes(&state["layers"]) { "true" } else { "false" }
            ));
            for (tier_idx, label) in [
                (0usize, "1. Edge & Gateways"),
                (1, "2. Application Services"),
                (2, "3. Core & Platform"),
                (3, "4. Shared Authority"),
            ] {
                let members: Vec<String> = tiers
                    .iter()
                    .filter(|(_, &t)| t == tier_idx)
                    .map(|(k, _)| k.clone())
                    .collect();
                if !members.is_empty() {
                    html.push_str(&format!(
                        "<h5 class=\"system-side-subheading\">{}</h5><div class=\"system-chip-row\">{}</div>",
                        escape(label),
                        members
                            .iter()
                            .map(|m_id| format!("<span class=\"system-chip\">{}</span>", escape(m_id)))
                            .collect::<Vec<_>>()
                            .join("")
                    ));
                }
            }
            if !violations.is_empty() {
                html.push_str("<h5 class=\"system-side-subheading\">Upward tier violations</h5><ul class=\"system-side-list\">");
                for (f, t) in violations {
                    html.push_str(&format!(
                        "<li><strong>{}</strong> → <strong>{}</strong></li>",
                        escape(&f),
                        escape(&t)
                    ));
                }
                html.push_str("</ul>");
            }
        }
        _ => {
            // Path between services + "The other way"
            let p_from = m::text(state, "path_from");
            let p_to = m::text(state, "path_to");
            let skip: BTreeSet<String> = m::array(&state["skip"])
                .into_iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            let swap_p = payload_attr(&json!({ "surface": surface.id }));
            html.push_str(&format!(
                "<label class=\"field\">From consumer<select data-action=\"system-map-path-from\" data-surface=\"{}\">{}</select></label><label class=\"field\">To provider<select data-action=\"system-map-path-to\" data-surface=\"{}\">{}</select></label><div class=\"system-side-actions\"><button type=\"button\" class=\"button compact\" data-action=\"system-map-path-swap\" data-payload=\"{swap_p}\">The other way ⇄</button></div>",
                surface.id,
                service_options(&p_from),
                surface.id,
                service_options(&p_to),
            ));
            if !p_from.is_empty() && !p_to.is_empty() {
                let paths = shortest_service_paths(doc, &p_from, &p_to, &skip);
                if paths.is_empty() {
                    html.push_str("<p class=\"system-note\">No directed contract path connects those services in this direction.</p>");
                } else {
                    html.push_str("<ul class=\"system-side-list\">");
                    for path in &paths {
                        html.push_str(&format!("<li>{}</li>", escape(&path.join(" → "))));
                    }
                    html.push_str("</ul>");
                    let intermediates: BTreeSet<String> = paths
                        .iter()
                        .flat_map(|p| p.iter().skip(1).take(p.len().saturating_sub(2)))
                        .cloned()
                        .collect();
                    if !intermediates.is_empty() || !skip.is_empty() {
                        html.push_str("<h5 class=\"system-side-subheading\">Skip intermediate service</h5><div class=\"system-chip-row\">");
                        for mid in intermediates.union(&skip) {
                            let sp =
                                payload_attr(&json!({ "surface": surface.id, "service": mid }));
                            html.push_str(&format!(
                                "<button type=\"button\" class=\"button compact\" aria-pressed=\"{}\" data-action=\"system-map-path-skip\" data-payload=\"{sp}\">{}</button>",
                                if skip.contains(mid) { "true" } else { "false" },
                                escape(mid),
                            ));
                        }
                        html.push_str("</div>");
                    }
                }
            }
        }
    }

    html.push_str("</div>");
    html
}

fn render_changes_tab_html(surface: SysMapSurface, _doc: &V, state: &V) -> String {
    let changes = m::data(surface.changes_key);
    let impact = m::data(surface.impact_key);
    let mut html = String::from("<div class=\"system-side-section\">");
    let refresh_p = payload_attr(&json!({ "surface": surface.id }));
    html.push_str(&format!(
        "<div class=\"system-side-head\"><h4 class=\"system-side-heading\">Changes &amp; cross-service impact</h4><button type=\"button\" class=\"button compact\" data-action=\"system-map-refresh-changes\" data-payload=\"{refresh_p}\">Refresh</button></div>"
    ));

    if impact.is_object() {
        let mut subject = first_non_empty(&impact, &["contract", "path"]);
        if subject.is_empty() {
            if let Some(first_ic) = m::array(&impact["impacted_contracts"]).into_iter().next() {
                subject = first_non_empty(&first_ic, &["contract", "id", "name"]);
            }
        }
        if subject.is_empty() {
            subject = m::text(&impact, "service");
        }
        if !subject.is_empty() {
            html.push_str(&format!(
                "<div class=\"system-side-callout\"><strong>Impact for {}</strong>",
                escape(&subject)
            ));
            let consumers = if impact["contract_consumers"].is_array() {
                m::array(&impact["contract_consumers"])
            } else if impact["consumers"].is_array() {
                m::array(&impact["consumers"])
            } else {
                m::array(&impact["downstream_services"])
            };
            if !consumers.is_empty() {
                html.push_str("<ul class=\"system-side-list\">");
                for c in consumers.iter().take(12) {
                    let label = c.as_str().map(str::to_owned).unwrap_or_else(|| {
                        let svc =
                            first_non_empty(c, &["consumer", "consumer_service", "service", "id"]);
                        let contract = first_non_empty(c, &["contract", "name"]);
                        let mut src = m::text(c, "path");
                        if src.is_empty() {
                            src = m::array(&c["consumer_sources"])
                                .into_iter()
                                .chain(m::array(&c["unopened_sources"]))
                                .find_map(|v| {
                                    v.as_str()
                                        .map(str::trim)
                                        .filter(|s| !s.is_empty())
                                        .map(str::to_owned)
                                })
                                .unwrap_or_default();
                        }
                        let mut parts = Vec::new();
                        if !svc.is_empty() {
                            parts.push(svc);
                        }
                        if !contract.is_empty() {
                            parts.push(format!("({contract})"));
                        }
                        if !src.is_empty() {
                            parts.push(src);
                        }
                        parts.join(" ")
                    });
                    html.push_str(&format!("<li>{}</li>", escape(label.trim())));
                }
                html.push_str("</ul>");
            }
            let unopened = m::array(&impact["cross_service_unopened"]);
            if !unopened.is_empty() {
                html.push_str(&format!(
                    "<p class=\"system-note\">Unopened downstream consumers: {}</p>",
                    unopened.len()
                ));
            }
            html.push_str("</div>");
        }
    }

    let by_service = if changes["services"].is_array() {
        m::array(&changes["services"])
    } else if changes["groups"].is_array() {
        m::array(&changes["groups"])
    } else {
        Vec::new()
    };

    if !by_service.is_empty() {
        for group in &by_service {
            let sid = first_non_empty(group, &["service", "id", "name"]);
            let files = if group["changed_files"].is_array() {
                m::array(&group["changed_files"])
            } else {
                m::array(&group["files"])
            };
            html.push_str(&format!(
                "<h5 class=\"system-side-subheading\">{} ({} files)</h5><ul class=\"system-side-list\">",
                escape(&sid),
                files.len()
            ));
            for f in files.iter().take(20) {
                let fpath = f
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| m::text(f, "path"));
                if fpath.is_empty() {
                    continue;
                }
                let file_p = payload_attr(&json!({
                    "service": sid,
                    "path": fpath,
                }));
                html.push_str(&format!(
                    "<li class=\"system-contract-row\"><span class=\"system-file-path\">{}</span><button type=\"button\" class=\"button compact\" data-action=\"inspect-system-file-graph\" data-service=\"{}\" data-path=\"{}\" data-payload=\"{file_p}\">Map</button></li>",
                    escape(&fpath),
                    escape(&sid),
                    escape(&fpath),
                ));
            }
            html.push_str("</ul>");
        }
    } else if changes["files"].is_array() {
        let files = m::array(&changes["files"]);
        if files.is_empty() {
            html.push_str("<p class=\"system-note\">No uncommitted or run-scoped changes reported across member services.</p>");
        } else {
            html.push_str("<ul class=\"system-side-list\">");
            for f in files.iter().take(30) {
                let sid = first_non_empty(f, &["service", "project"]);
                let fpath = m::text(f, "path");
                let file_p = payload_attr(&json!({ "service": sid, "path": fpath }));
                html.push_str(&format!(
                    "<li class=\"system-contract-row\"><span><strong>{}</strong> · {}</span><button type=\"button\" class=\"button compact\" data-action=\"inspect-system-file-graph\" data-service=\"{}\" data-path=\"{}\" data-payload=\"{file_p}\">Map</button></li>",
                    escape(&sid),
                    escape(&fpath),
                    escape(&sid),
                    escape(&fpath),
                ));
            }
            html.push_str("</ul>");
        }
    } else if changes["services"].is_array() || changes["groups"].is_array() {
        html.push_str("<p class=\"system-note\">No uncommitted or run-scoped changes reported across member services.</p>");
    } else {
        let overlay = &state["overlay"];
        if m::text(overlay, "run_id").is_empty() {
            html.push_str("<p class=\"system-note\">Working tree changes across member services appear here once queried.</p>");
        } else {
            html.push_str(
                "<p class=\"system-note\">Loading run changes across member services…</p>",
            );
        }
    }

    html.push_str("</div>");
    html
}

fn set_html(id: &str, content: &str) {
    if let Some(el) = dom::get(id) {
        el.set_inner_html(content);
    }
}

fn sync_toolbar_buttons(surface: SysMapSurface, state: &V) {
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
}

thread_local! {
    static SYS_SURFACE_DIGEST: RefCell<std::collections::HashMap<&'static str, u64>> =
        RefCell::new(std::collections::HashMap::new());
}

#[derive(Clone, Debug)]
struct SysDenseCacheEntry {
    layout: SysLayout,
    node_index_by_id: HashMap<String, usize>,
    node_htmls: Vec<String>,
    edge_endpoints: Vec<(String, String)>,
    is_dense: bool,
    latest_roving_id: String,
    latest_focus_id: String,
    highlight_nodes: BTreeSet<String>,
    highlight_edges: BTreeSet<(String, String)>,
    sel_edge_pair: (String, String),
    failed_services: BTreeSet<String>,
    pinned_ids: BTreeSet<String>,
    blast_roles: HashMap<String, (&'static str, String, bool)>,
}

thread_local! {
    static SYS_DENSE_CACHE: RefCell<HashMap<&'static str, SysDenseCacheEntry>> =
        RefCell::new(HashMap::new());
}

fn patch_system_node_el(
    el: &web_sys::Element,
    nid: &str,
    focus_id: &str,
    target_roving: &str,
    highlight_nodes: &BTreeSet<String>,
    failed_services: &BTreeSet<String>,
    blast_roles: &HashMap<String, (&'static str, String, bool)>,
) {
    let svc_id = el
        .get_attribute("data-explain-services")
        .unwrap_or_default();
    let _ = el.set_attribute(
        "data-selected",
        if nid == focus_id { "true" } else { "false" },
    );
    if highlight_nodes.contains(nid) {
        let _ = el.set_attribute("data-shared-path", "true");
    } else {
        let _ = el.remove_attribute("data-shared-path");
    }
    let is_failed = failed_services.contains(nid)
        || (!svc_id.is_empty() && failed_services.contains(svc_id.as_str()));
    if is_failed {
        let _ = el.set_attribute("data-failed", "true");
    } else {
        let _ = el.remove_attribute("data-failed");
    }
    if !target_roving.is_empty() {
        let _ = el.set_attribute("tabindex", if nid == target_roving { "0" } else { "-1" });
    }
    if let Some((blast_role, step_state, waiting)) = blast_roles.get(nid) {
        if *waiting {
            let _ = el.set_attribute("data-beyond", "true");
        } else {
            let _ = el.remove_attribute("data-beyond");
        }
        if !el.class_list().contains("system-map-child") {
            let _ = el.set_attribute("data-blast", blast_role);
            let _ = el.set_attribute("data-step-state", step_state);
            if let Ok(Some(dot)) = el.query_selector(".system-node-dot") {
                if !step_state.is_empty() {
                    let _ = dot.set_attribute("data-state", step_state);
                } else {
                    let fallback = el
                        .get_attribute("data-graph-state")
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "idle".to_owned());
                    let _ = dot.set_attribute("data-state", &fallback);
                }
            }
        }
    }
}

fn is_important_system_node(
    entry: &SysDenseCacheEntry,
    nid: &str,
    el: Option<&web_sys::Element>,
) -> bool {
    let failed = entry.failed_services.contains(nid)
        || el
            .and_then(|e| e.get_attribute("data-explain-services"))
            .is_some_and(|svc| entry.failed_services.contains(&svc));
    let el_important = el.is_some_and(|e| {
        e.get_attribute("data-selected").as_deref() == Some("true")
            || e.get_attribute("data-shared-path").as_deref() == Some("true")
            || e.get_attribute("data-pinned").as_deref() == Some("true")
            || e.get_attribute("data-failed").as_deref() == Some("true")
            || e.get_attribute("tabindex").as_deref() == Some("0")
    });
    el_important
        || nid == entry.latest_focus_id
        || nid == entry.latest_roving_id
        || entry.highlight_nodes.contains(nid)
        || entry.pinned_ids.contains(nid)
        || failed
}

fn mount_system_dense_node_if_needed(
    surface: SysMapSurface,
    nid: &str,
) -> Option<web_sys::Element> {
    let canvas = dom::get(surface.canvas_id)?;
    if let Ok(Some(existing)) = canvas.query_selector(&format!(".system-map-node[data-n='{nid}']"))
    {
        return Some(existing);
    }
    let plane = canvas.query_selector(".system-map-plane").ok().flatten()?;
    let entry = SYS_DENSE_CACHE.with(|c| c.borrow().get(surface.canvas_id).cloned())?;
    if !entry.is_dense {
        return None;
    }
    let &idx = entry.node_index_by_id.get(nid)?;
    let html = entry.node_htmls.get(idx)?.as_str();
    if html.is_empty() {
        return None;
    }
    let mut created = graph_viewport::append_html_elements(&plane, html);
    let el = created.pop()?;
    patch_system_node_el(
        &el,
        nid,
        &entry.latest_focus_id,
        &entry.latest_roving_id,
        &entry.highlight_nodes,
        &entry.failed_services,
        &entry.blast_roles,
    );
    Some(el)
}

fn sync_system_viewport_culling(surface: SysMapSurface, cam: &Camera) {
    let Some(entry) = SYS_DENSE_CACHE.with(|c| c.borrow().get(surface.canvas_id).cloned()) else {
        return;
    };
    let nodes = dom::all(&format!("#{} .system-map-node[data-n]", surface.canvas_id));
    let (vw, vh, measured) = viewport_size(surface.canvas_id);
    let bounds = measured
        .then(|| graph_viewport::world_viewport_bounds(cam.viewport_cam(), (vw, vh), 0.0))
        .flatten();
    let Some((min_x, min_y, max_x, max_y)) = bounds else {
        for el in nodes {
            let _ = el.remove_attribute("data-offscreen");
        }
        return;
    };
    let mut mounted_els: HashMap<usize, web_sys::Element> = HashMap::with_capacity(nodes.len());
    for el in nodes {
        if let Some(nid) = el.get_attribute("data-n") {
            if let Some(&idx) = entry.node_index_by_id.get(&nid) {
                mounted_els.insert(idx, el);
            }
        }
    }

    if entry.is_dense && entry.node_htmls.len() == entry.layout.nodes.len() {
        if let Some(plane) = dom::get(surface.canvas_id)
            .and_then(|c| c.query_selector(".system-map-plane").ok().flatten())
        {
            let rects: Vec<(f64, f64, f64, f64)> = entry
                .layout
                .nodes
                .iter()
                .map(|n| (n.x, n.y, n.w, n.h))
                .collect();
            let mounted: Vec<(usize, bool)> = mounted_els
                .iter()
                .map(|(&idx, el)| {
                    let nid = entry
                        .layout
                        .nodes
                        .get(idx)
                        .map(|n| n.id.as_str())
                        .unwrap_or("");
                    (idx, is_important_system_node(&entry, nid, Some(el)))
                })
                .collect();
            let force_mount: Vec<usize> = entry
                .layout
                .nodes
                .iter()
                .enumerate()
                .filter_map(|(idx, n)| {
                    (!entry.node_htmls.get(idx).is_none_or(|h| h.is_empty())
                        && is_important_system_node(&entry, &n.id, None))
                    .then_some(idx)
                })
                .collect();
            let visible = (min_x, min_y, max_x, max_y);
            let mount = (min_x - 120.0, min_y - 120.0, max_x + 120.0, max_y + 120.0);
            let unmount = (min_x - 280.0, min_y - 280.0, max_x + 280.0, max_y + 280.0);
            let plan = graph_viewport::plan_viewport_reconcile(
                &rects,
                &mounted,
                visible,
                mount,
                unmount,
                &force_mount,
            );
            for idx in plan.to_unmount {
                if !entry.node_htmls.get(idx).is_none_or(|s| s.is_empty()) {
                    if let Some(el) = mounted_els.remove(&idx) {
                        el.remove();
                    }
                }
            }
            let mut batch_html = String::new();
            let mut batch_indices = Vec::new();
            for idx in plan.to_mount {
                if mounted_els.contains_key(&idx) {
                    continue;
                }
                if let Some(html) = entry.node_htmls.get(idx).filter(|s| !s.is_empty()) {
                    batch_html.push_str(html);
                    batch_indices.push(idx);
                }
            }
            if !batch_html.is_empty() {
                let inserted = graph_viewport::append_html_elements(&plane, &batch_html);
                for (idx, el) in batch_indices.into_iter().zip(inserted) {
                    if let Some(placed) = entry.layout.nodes.get(idx) {
                        patch_system_node_el(
                            &el,
                            &placed.id,
                            &entry.latest_focus_id,
                            &entry.latest_roving_id,
                            &entry.highlight_nodes,
                            &entry.failed_services,
                            &entry.blast_roles,
                        );
                    }
                    mounted_els.insert(idx, el);
                }
            }
        }
    }

    for (idx, el) in mounted_els {
        let Some(placed) = entry.layout.nodes.get(idx) else {
            continue;
        };
        let important = is_important_system_node(&entry, &placed.id, Some(&el));
        let offscreen = !important
            && (placed.x + placed.w < min_x - 120.0
                || placed.x > max_x + 120.0
                || placed.y + placed.h < min_y - 120.0
                || placed.y > max_y + 120.0);
        if offscreen {
            let _ = el.set_attribute("data-offscreen", "true");
        } else {
            let _ = el.remove_attribute("data-offscreen");
        }
    }
}

fn build_blast_roles_map(
    nodes: &[PlacedSysNode],
    vis: &SysVisualState,
) -> HashMap<String, (&'static str, String, bool)> {
    nodes
        .iter()
        .map(|n| {
            (
                n.id.clone(),
                (
                    vis.blast_role(&n.id),
                    vis.step_state(&n.id),
                    vis.beyond(&n.id),
                ),
            )
        })
        .collect()
}

fn refresh_dense_scene_for_system(
    scene: &mut graph_viewport::canvas2d::DenseScene,
    vis: &SysVisualState,
    edge_endpoints: &[(String, String)],
) {
    let mut focus_neighbors: HashSet<&str> = HashSet::new();
    if !vis.focus_id.is_empty() {
        for (f, t) in edge_endpoints {
            if f == &vis.focus_id {
                focus_neighbors.insert(t.as_str());
            } else if t == &vis.focus_id {
                focus_neighbors.insert(f.as_str());
            }
        }
    }
    for node in &mut scene.nodes {
        let nid = node.label.as_str();
        node.selected = nid == vis.focus_id;
        let adjacent_to_focus = !vis.focus_id.is_empty() && focus_neighbors.contains(nid);
        node.lit = node.selected || adjacent_to_focus || vis.highlight_nodes.contains(nid);
        node.dim = (!vis.highlight_nodes.is_empty() && !vis.highlight_nodes.contains(nid))
            || (vis.highlight_nodes.is_empty() && !vis.focus_id.is_empty() && !node.lit);
        node.changed = vis.is_changed(nid);
        node.failed = vis.failed_services.contains(nid)
            || (!node.role.is_empty()
                && node.role != "service"
                && node.role != "system"
                && vis.failed_services.contains(node.role.as_str()));
    }
    for edge in &mut scene.edges {
        let (from, to) = edge_endpoints
            .get(edge.index)
            .map(|(f, t)| (f.as_str(), t.as_str()))
            .unwrap_or(("", ""));
        let is_hl = !from.is_empty()
            && !to.is_empty()
            && vis
                .highlight_edges
                .contains(&(from.to_owned(), to.to_owned()));
        let is_sel_edge = !vis.sel_edge_pair.0.is_empty()
            && from == vis.sel_edge_pair.0
            && to == vis.sel_edge_pair.1;
        let is_focus_adj = !vis.focus_id.is_empty() && (from == vis.focus_id || to == vis.focus_id);
        edge.on = is_hl || is_sel_edge || is_focus_adj;
        edge.lit = is_hl || is_sel_edge;
        edge.dim = (!vis.highlight_edges.is_empty() && !is_hl)
            || (vis.highlight_edges.is_empty() && !vis.focus_id.is_empty() && !edge.on);
    }
}

fn try_patch_system_canvas_in_place(surface: SysMapSurface, view_doc: &V, state: &V) -> bool {
    let Some(canvas) = dom::get(surface.canvas_id) else {
        return false;
    };
    if canvas
        .query_selector(".system-map-plane")
        .ok()
        .flatten()
        .is_none()
    {
        return false;
    }
    let vis = compute_sys_visual_state(view_doc, state);
    let focus_id = vis.focus_id.clone();
    let sel_from = vis.sel_edge_pair.0.clone();
    let sel_to = vis.sel_edge_pair.1.clone();
    let roving = m::text(state, "roving_focus");
    let target_roving = if !roving.is_empty() {
        roving
    } else {
        focus_id.clone()
    };

    let mut blast_roles = HashMap::new();
    let mut edge_endpoints = Vec::new();
    let mut is_dense = false;
    SYS_DENSE_CACHE.with(|cache| {
        if let Some(entry) = cache.borrow().get(surface.canvas_id) {
            is_dense = entry.is_dense;
            blast_roles = build_blast_roles_map(&entry.layout.nodes, &vis);
            edge_endpoints = entry.edge_endpoints.clone();
        }
    });
    if blast_roles.is_empty() {
        for n in m::array(&view_doc["nodes"]) {
            let nid = node_id(&n);
            blast_roles.insert(
                nid.clone(),
                (vis.blast_role(&nid), vis.step_state(&nid), vis.beyond(&nid)),
            );
        }
    }

    if let Ok(nodes) = canvas.query_selector_all(".system-map-node[data-n]") {
        for i in 0..nodes.length() {
            let Some(el) = nodes
                .item(i)
                .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
            else {
                continue;
            };
            let nid = el.get_attribute("data-n").unwrap_or_default();
            patch_system_node_el(
                &el,
                &nid,
                &focus_id,
                &target_roving,
                &vis.highlight_nodes,
                &vis.failed_services,
                &blast_roles,
            );
        }
    }

    SYS_DENSE_CACHE.with(|cache| {
        if let Some(entry) = cache.borrow_mut().get_mut(surface.canvas_id) {
            entry.latest_focus_id = focus_id.clone();
            if !target_roving.is_empty() {
                entry.latest_roving_id = target_roving.clone();
            }
            entry.highlight_nodes = vis.highlight_nodes.clone();
            entry.highlight_edges = vis.highlight_edges.clone();
            entry.sel_edge_pair = vis.sel_edge_pair.clone();
            entry.failed_services = vis.failed_services.clone();
            entry.blast_roles = blast_roles.clone();
        }
    });

    if is_dense {
        if !focus_id.is_empty() {
            let _ = mount_system_dense_node_if_needed(surface, &focus_id);
        }
        if !target_roving.is_empty() && target_roving != focus_id {
            let _ = mount_system_dense_node_if_needed(surface, &target_roving);
        }
        graph_viewport::canvas2d::update_scene(surface.canvas_id, |scene| {
            refresh_dense_scene_for_system(scene, &vis, &edge_endpoints);
        });
    } else {
        if let Ok(edges) = canvas.query_selector_all(".system-map-edge[data-from][data-to]") {
            for i in 0..edges.length() {
                let Some(el) = edges
                    .item(i)
                    .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
                else {
                    continue;
                };
                let from = el.get_attribute("data-from").unwrap_or_default();
                let to = el.get_attribute("data-to").unwrap_or_default();
                let is_hl = vis.highlight_edges.contains(&(from.clone(), to.clone()));
                let is_sel = (!focus_id.is_empty() && (from == focus_id || to == focus_id))
                    || (!sel_from.is_empty() && from == sel_from && to == sel_to);
                let cl = el.class_list();
                let _ = if is_hl || is_sel {
                    cl.add_1("graph-map-edge-on")
                } else {
                    cl.remove_1("graph-map-edge-on")
                };
                if is_hl {
                    let _ = el.set_attribute("data-shared-path", "true");
                } else {
                    let _ = el.remove_attribute("data-shared-path");
                }
                if vis.beyond(&from) {
                    let _ = el.set_attribute("data-beyond", "true");
                } else {
                    let _ = el.remove_attribute("data-beyond");
                }
            }
        }
        for sel in [".system-map-violation-halo", ".system-map-edge-label"] {
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
                    let is_hl = vis.highlight_edges.contains(&(from.clone(), to.clone()));
                    let is_sel =
                        (!sel_from.is_empty() && from == sel_from && to == sel_to) || is_hl;
                    if is_sel {
                        let _ = el.set_attribute("data-edge-state", "on");
                    } else {
                        let _ = el.remove_attribute("data-edge-state");
                    }
                }
            }
        }
    }

    for box_el in dom::all(&format!(
        "#{}-radar .graph-radar-box[data-n]",
        surface.canvas_id
    )) {
        let nid = box_el.get_attribute("data-n").unwrap_or_default();
        let next_state = if nid == focus_id {
            Some("selected")
        } else if vis.blast.seeds.contains(&nid)
            || vis.blast.direct_consumers.contains(&nid)
            || vis.blast.downstream_consumers.contains(&nid)
        {
            Some("changed")
        } else if box_el.get_attribute("data-cycle").as_deref() == Some("true") {
            Some("cycle")
        } else {
            None
        };
        if let Some(st) = next_state {
            let _ = box_el.set_attribute("data-state", st);
        } else {
            let _ = box_el.remove_attribute("data-state");
        }
    }
    true
}

fn build_dense_scene_for_surface(
    surface: SysMapSurface,
    doc: &V,
    state: &V,
    layout: &SysLayout,
) -> Option<graph_viewport::canvas2d::DenseScene> {
    if layout.nodes.len() < 180 && layout.wires.len() < 400 {
        return None;
    }
    let vis = compute_sys_visual_state(doc, state);
    let edges = m::array(&doc["edges"]);
    let pins = graph_viewport::load_pins(&sys_pin_key(surface, state));
    let nodes = layout
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| graph_viewport::canvas2d::DenseNodeSpec {
            index: i,
            x: n.x,
            y: n.y,
            w: n.w,
            h: n.h,
            label: n.id.clone(),
            role: n.child_of.as_deref().unwrap_or("service").to_owned(),
            selected: n.id == vis.focus_id,
            lit: false,
            dim: false,
            changed: vis.is_changed(&n.id),
            pinned: pins.contains_key(&n.id),
            failed: vis.failed_services.contains(&n.id)
                || n.child_of
                    .as_deref()
                    .is_some_and(|p| vis.failed_services.contains(p)),
        })
        .collect();
    let mut edge_endpoints = vec![(String::new(), String::new()); layout.wires.len()];
    let dense_edges = layout
        .wires
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let edge = edges.get(w.edge_index);
            let from = edge.map(|e| m::text(e, "from")).unwrap_or_default();
            let to = edge.map(|e| m::text(e, "to")).unwrap_or_default();
            edge_endpoints[i] = (from, to);
            let kind = edge.map(primary_edge_kind).unwrap_or("other").to_owned();
            graph_viewport::canvas2d::DenseEdgeSpec::from_svg_path(
                i, &w.path_d, 1, false, w.back, kind, false, false, false, false,
            )
        })
        .collect();
    let mut scene = graph_viewport::canvas2d::DenseScene {
        width: layout.width,
        height: layout.height,
        nodes,
        edges: dense_edges,
    };
    refresh_dense_scene_for_system(&mut scene, &vis, &edge_endpoints);
    Some(scene)
}

pub(crate) fn mark_failed_services(failed_ids: &[String]) {
    let arr: Vec<V> = failed_ids
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| json!(s))
        .collect();
    m::set("system-failed-services", V::Array(arr));
    if cfg!(target_arch = "wasm32") {
        for surface in [SERVICES_SURFACE, CHANGES_SURFACE] {
            if m::data(surface.map_key).is_object() {
                render_surface(surface, false);
            }
        }
    }
}

fn render_surface(surface: SysMapSurface, refit: bool) {
    attach_wheel_listeners();
    ensure_hover_watch(surface);
    ensure_legend_hover_watch(surface);
    ensure_resize_watch();
    let state = surface_state(surface);
    let doc = m::data(surface.map_key);
    sync_toolbar_buttons(surface, &state);

    let svc_ids: Vec<String> = m::array(&doc["nodes"])
        .into_iter()
        .filter(|n| matches!(n["kind"].as_str(), Some("service" | "system")))
        .map(|n| node_id(&n))
        .filter(|s| !s.is_empty())
        .collect();
    let explain_scope = crate::graph::GraphTargetScope::SystemService {
        project_id: m::text(&state, "project_id"),
        config_path: m::text(&state, "config_path"),
        service_id: "__system__".to_owned(),
        services: svc_ids,
        run_id: {
            let rid = m::text(&state["overlay"], "run_id");
            (!rid.is_empty()).then_some(rid)
        },
        run_title: None,
        launches: Vec::new(),
    };
    crate::graph_explanation::surface_map_context(surface.canvas_id, &doc, &doc, &explain_scope);

    let view_doc = filtered_doc(&doc, &state);
    let pins = graph_viewport::load_pins(&sys_pin_key(surface, &state));
    let digest = system_structure_digest(&view_doc, &state, &pins);
    let prev_digest =
        SYS_SURFACE_DIGEST.with(|d| d.borrow().get(surface.canvas_id).copied().unwrap_or(0));
    let mut cam = load_camera(surface);

    let (pw, ph) = if !refit
        && digest != 0
        && digest == prev_digest
        && try_patch_system_canvas_in_place(surface, &view_doc, &state)
    {
        (cam.plane_w, cam.plane_h)
    } else {
        let layers_mode = m::yes(&state["layers"]);
        let layout = layout_system_nodes_with_pins(&view_doc, layers_mode, &pins);
        let bundle = render_canvas_bundle_with_layout(surface, &view_doc, &state, &layout);
        let canvas_html = bundle.html;
        let radar_svg = system_radar_html(&doc, &state, &layout);
        let pw = layout.width;
        let ph = layout.height;
        invalidate_hover_cache(surface.canvas_id);
        let combined_html = if canvas_html.is_empty() {
            String::new()
        } else if radar_svg.is_empty() {
            canvas_html.clone()
        } else {
            format!(
                "{canvas_html}<div class=\"graph-map-radar system-map-radar\" id=\"{}-radar\" data-surface=\"{}\" aria-hidden=\"true\" hidden>{radar_svg}</div>",
                surface.canvas_id,
                surface.id,
            )
        };
        set_html(surface.canvas_id, &combined_html);
        SYS_SURFACE_DIGEST.with(|d| {
            d.borrow_mut().insert(surface.canvas_id, digest);
        });
        let vis_for_cache = compute_sys_visual_state(&view_doc, &state);
        let node_index_by_id: HashMap<String, usize> = layout
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.clone(), i))
            .collect();
        let blast_roles = build_blast_roles_map(&layout.nodes, &vis_for_cache);
        let cache_entry = SysDenseCacheEntry {
            layout: layout.clone(),
            node_index_by_id,
            node_htmls: bundle.node_htmls,
            edge_endpoints: bundle.edge_endpoints,
            is_dense: bundle.is_dense,
            latest_roving_id: bundle.active_roving_id,
            latest_focus_id: vis_for_cache.focus_id.clone(),
            highlight_nodes: bundle.highlight_nodes,
            highlight_edges: vis_for_cache.highlight_edges.clone(),
            sel_edge_pair: vis_for_cache.sel_edge_pair.clone(),
            failed_services: bundle.failed_services,
            pinned_ids: bundle.pinned_ids,
            blast_roles,
        };
        SYS_DENSE_CACHE.with(|c| {
            c.borrow_mut().insert(surface.canvas_id, cache_entry);
        });
        if let Some(scene) = build_dense_scene_for_surface(surface, &view_doc, &state, &layout) {
            graph_viewport::canvas2d::store_scene(surface.canvas_id, scene);
        } else {
            graph_viewport::canvas2d::clear_scene(surface.canvas_id);
        }
        dom::hidden(surface.legend_id, canvas_html.is_empty());
        (pw, ph)
    };
    set_html(
        surface.legend_id,
        &format!(
            "{}{}",
            wave_scrubber_html(surface, &blast_of(&doc, &state), &state),
            contract_arrow_legend_html_for(surface, &view_doc, &state)
        ),
    );

    let svcs = m::array(&doc["nodes"])
        .iter()
        .filter(|n| n["kind"] == "service")
        .count();
    let links = m::array(&doc["edges"])
        .iter()
        .filter(|e| !m::yes(&e["child_link"]))
        .count();
    dom::text(
        surface.summary_id,
        &if svcs == 0 {
            String::new()
        } else {
            format!("{svcs} services · {links} links")
        },
    );

    let note = m::text(&doc, "note");
    dom::text(surface.note_id, &note);
    dom::hidden(surface.note_id, note.is_empty());

    set_html(surface.side_id, &render_side_html(surface, &doc, &state));

    let size_changed = (cam.plane_w - pw).abs() > 1.0 || (cam.plane_h - ph).abs() > 1.0;
    cam.plane_w = pw;
    cam.plane_h = ph;
    if refit || cam.fit_mode || (size_changed && cam.needs_measure) {
        let (vw, vh, measured) = viewport_size(surface.canvas_id);
        cam.needs_measure = !measured;
        if pw > 0.0 && ph > 0.0 {
            cam.apply_viewport_cam(graph_viewport::fit(
                (pw, ph),
                (vw, vh),
                graph_viewport::FitOptions::for_system_map(),
            ));
        }
    }
    save_camera(surface, &cam);
    apply_camera(surface, &cam);
}

async fn fetch_surface_map(surface: SysMapSurface, catalog: V, graph_status: V, refit: bool) {
    let state = surface_state(surface);
    let project_id = m::text(&state, "project_id");
    let config_path = m::text(&state, "config_path");
    if project_id.is_empty() {
        return;
    }

    let epoch = m::next(surface.fetch_epoch);
    let overlay = &state["overlay"];
    let run_id = m::text(overlay, "run_id");
    let target = if !run_id.is_empty() {
        format!("sysrun:{run_id}")
    } else {
        String::new()
    };
    let seeds = m::array(&overlay["seed_services"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect::<Vec<_>>()
        .join(",");
    let spread = m::array(&state["spread"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect::<Vec<_>>()
        .join(",");

    let query = dom::query(&[
        ("project_id", project_id),
        ("config_path", config_path),
        ("target", target),
        ("run_id", run_id),
        ("seed_services", seeds),
        (
            "depth",
            overlay["depth"]
                .as_u64()
                .map(|d| d.clamp(1, 16).to_string())
                .unwrap_or_default(),
        ),
        ("focus", m::text(&state, "focus")),
        ("spread", spread),
        (
            "whole",
            if m::yes(&state["whole"]) {
                "1".to_owned()
            } else {
                String::new()
            },
        ),
        (
            "certain",
            if m::yes(&state["certain"]) {
                "1".to_owned()
            } else {
                String::new()
            },
        ),
        (
            "layers",
            if m::yes(&state["layers"]) {
                "1".to_owned()
            } else {
                String::new()
            },
        ),
    ]);

    let path = format!("/api/systems/graph/map{query}");
    match api::get(&path).await {
        Ok(raw) => {
            if !m::current(surface.fetch_epoch, epoch) {
                return;
            }
            dom::error(surface.error_id, "");
            let cur_state = surface_state(surface);
            let normalized = normalize_map_doc(&raw, &catalog, &graph_status, &cur_state);
            m::set(surface.map_key, normalized);
            render_surface(surface, refit);
        }
        Err(_) => {
            if !m::current(surface.fetch_epoch, epoch) {
                return;
            }
            // Keep synthesized catalog map active if backend graph route is unavailable.
            let cur_state = surface_state(surface);
            let fallback = expand_unfolded_nodes(
                &synthesize_map_from_catalog(&catalog, &graph_status),
                &cur_state,
            );
            m::set(surface.map_key, fallback);
            render_surface(surface, refit);
        }
    }
}

async fn fetch_surface_changes(surface: SysMapSurface) {
    let state = surface_state(surface);
    let project_id = m::text(&state, "project_id");
    let config_path = m::text(&state, "config_path");
    if project_id.is_empty() {
        return;
    }
    let epoch = m::next(surface.changes_epoch);
    let run_id = m::text(&state["overlay"], "run_id");
    let target = if !run_id.is_empty() {
        format!("sysrun:{run_id}")
    } else {
        "work".to_owned()
    };
    let q = dom::query(&[
        ("project_id", project_id),
        ("config_path", config_path),
        ("target", target),
        ("run_id", run_id),
        ("service", m::text(&state, "focus")),
    ]);
    if let Ok(doc) = api::get(&format!("/api/systems/graph/changes{q}")).await {
        if m::current(surface.changes_epoch, epoch) {
            m::set(surface.changes_key, doc);
            render_surface(surface, false);
        }
    }
}

async fn fetch_contract_impact(surface: SysMapSurface, service: String, contract: String) {
    let state = surface_state(surface);
    let project_id = m::text(&state, "project_id");
    let config_path = m::text(&state, "config_path");
    if project_id.is_empty() {
        return;
    }
    let epoch = m::next(surface.impact_epoch);
    let run_id = m::text(&state["overlay"], "run_id");
    let q = dom::query(&[
        ("project_id", project_id),
        ("config_path", config_path),
        ("service", service),
        ("contract", contract),
        ("run_id", run_id),
    ]);
    if let Ok(doc) = api::get(&format!("/api/systems/graph/impact{q}")).await {
        if m::current(surface.impact_epoch, epoch) {
            m::set(surface.impact_key, doc);
            let mut next = surface_state(surface);
            next["side_tab"] = json!("changes");
            save_surface_state(surface, &next);
            render_surface(surface, false);
        }
    }
}

async fn run_finder_search(surface: SysMapSurface, query_str: String) {
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

    if !project_id.is_empty() {
        let q = dom::query(&[
            ("project_id", project_id),
            ("config_path", config_path),
            ("query", q_trim.clone()),
        ]);
        if let Ok(remote) = api::get(&format!("/api/systems/graph/find{q}")).await {
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
    }

    if !m::current(surface.find_epoch, epoch) {
        return;
    }

    let mut seen = BTreeSet::new();
    let mut list_html = String::new();
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
        list_html.push_str(&format!(
            "<li role=\"option\"><button type=\"button\" class=\"system-find-item\" data-action=\"system-map-find-pick\" data-payload=\"{p}\"><span class=\"system-chip\">{}</span><strong>{}</strong><span class=\"system-note\">{}</span></button></li>",
            escape(if kind.is_empty() { "item" } else { &kind }),
            escape(&name),
            escape(&svc),
        ));
    }

    set_html(surface.find_list_id, &list_html);
    dom::text(
        surface.find_note_id,
        if list_html.is_empty() {
            "No matching services, contracts, capabilities, or symbols."
        } else {
            "Select a service or contract to focus here, or a member symbol/file to open its service code map."
        },
    );
    dom::hidden(surface.find_found_id, false);
    dom::attr(surface.find_input_id, "aria-expanded", "true");
}

fn close_finder(surface: SysMapSurface) {
    dom::hidden(surface.find_found_id, true);
    dom::attr(surface.find_input_id, "aria-expanded", "false");
}

pub fn refresh_services_map(project_id: &str, config_path: &str, catalog: &V, graph_status: &V) {
    let mut state = surface_state(SERVICES_SURFACE);
    let project_changed = m::text(&state, "project_id") != project_id
        || m::text(&state, "config_path") != config_path;
    state["project_id"] = json!(project_id);
    state["config_path"] = json!(config_path);
    if project_changed {
        state["focus"] = json!("");
        state["selected_edge"] = V::Null;
        save_camera(SERVICES_SURFACE, &Camera::default());
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
    if project_changed {
        state["focus"] = json!("");
        state["selected_edge"] = V::Null;
        save_camera(CHANGES_SURFACE, &Camera::default());
    }
    save_surface_state(CHANGES_SURFACE, &state);

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
    state["selected_edge"] = V::Null;
    state["side_tab"] = json!("changes");
    save_surface_state(SERVICES_SURFACE, &state);
    render_surface(SERVICES_SURFACE, false);
    spawn_local(async {
        fetch_surface_changes(SERVICES_SURFACE).await;
    });
}

pub fn current_context_brief() -> String {
    let state = surface_state(CHANGES_SURFACE);
    let doc = m::data(CHANGES_SURFACE.map_key);
    build_context_brief(&doc, &state)
}

fn attach_canvas_wheel(canvas_id: &'static str) {
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
struct HoverIndex {
    node_els: BTreeMap<String, Vec<web_sys::Element>>,
    edge_els: Vec<web_sys::Element>,
    edges_by_node: BTreeMap<String, Vec<usize>>,
    edges_by_pair: BTreeMap<(String, String), Vec<usize>>,
    adjacency: BTreeMap<String, BTreeSet<String>>,
    lit: Vec<web_sys::Element>,
}

thread_local! {
    static HOVER_CACHE: RefCell<BTreeMap<String, HoverIndex>> = const { RefCell::new(BTreeMap::new()) };
}

fn invalidate_hover_cache(canvas_id: &str) {
    HOVER_CACHE.with(|cache| {
        cache.borrow_mut().remove(canvas_id);
    });
}

#[cfg_attr(not(test), allow(dead_code))]
fn hover_pointed_key(hovered_node: Option<&str>, hovered_edge: Option<(&str, &str)>) -> String {
    match (hovered_node, hovered_edge) {
        (Some(nid), _) => format!("n:{nid}"),
        (None, Some((ef, et))) => format!("e:{ef}\u{1f}{et}"),
        (None, None) => String::new(),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn build_edge_adjacency(edges: &[(String, String)]) -> BTreeMap<String, BTreeSet<String>> {
    let mut adj: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (from, to) in edges {
        if !from.is_empty() && !to.is_empty() {
            adj.entry(from.clone()).or_default().insert(to.clone());
            adj.entry(to.clone()).or_default().insert(from.clone());
        }
    }
    adj
}

fn build_hover_index(canvas_id: &str) -> HoverIndex {
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
    for el in dom::all(&format!("#{canvas_id} [data-n]")) {
        let nid = el.get_attribute("data-n").unwrap_or_default();
        if !nid.is_empty() {
            index.node_els.entry(nid).or_default().push(el);
        }
    }
    index
}

fn apply_hover_lighting(
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

fn look_at_system_point(surface: SysMapSurface, point: (f64, f64)) {
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

fn radar_target_element(radar: &web_sys::Element) -> web_sys::Element {
    radar
        .query_selector(".graph-radar-picture")
        .ok()
        .flatten()
        .unwrap_or_else(|| radar.clone())
}

fn save_radar_drag(surface_id: &str, radar_id: &str, drag: graph_viewport::RadarDrag) {
    let mut raw = drag.to_json();
    raw["surface"] = json!(surface_id);
    raw["radar_id"] = json!(radar_id);
    m::set(RADAR_DRAG_KEY, raw);
}

fn radar_pointer_event(
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

fn ensure_resize_watch() {
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

fn ensure_legend_hover_watch(surface: SysMapSurface) {
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
            let _ = match held {
                Some(kind) => canvas.set_attribute("data-sys-kind-hover", &kind),
                None => canvas.remove_attribute("data-sys-kind-hover"),
            };
        });
        let _ = legend.add_event_listener_with_callback(event_kind, moved.as_ref().unchecked_ref());
        moved.forget();
    }
}

fn ensure_hover_watch(surface: SysMapSurface) {
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
        if let Some(node_el) = target.closest("[data-n]").ok().flatten() {
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
    });
    let _ = canvas.add_event_listener_with_callback("pointerleave", leave.as_ref().unchecked_ref());
    leave.forget();
}

pub fn pointer_event(kind: &str, event: &web_sys::Event) -> bool {
    let target_el = event
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok());

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

        // Shift+drag node pinning on system service cards.
        if matches!(
            kind,
            "pointerdown" | "pointermove" | "pointerup" | "pointercancel"
        ) {
            let drag_store_key = format!("{}-node-drag", surface.id_prefix);
            let mut node_drag = m::data(&drag_store_key);
            if kind == "pointerdown" && in_canvas && !in_radar {
                if let Ok(pe) = event.clone().dyn_into::<web_sys::PointerEvent>() {
                    if pe.shift_key() {
                        let resolved = if let Some(node_el) = target_el
                            .as_ref()
                            .and_then(|el| el.closest(".system-map-node[data-n]").ok().flatten())
                        {
                            let nid = node_el.get_attribute("data-n").unwrap_or_default();
                            let pos = SYS_DENSE_CACHE.with(|c| {
                                let g = c.borrow();
                                let entry = g.get(surface.canvas_id)?;
                                let &idx = entry.node_index_by_id.get(&nid)?;
                                let placed = entry.layout.nodes.get(idx)?;
                                Some((placed.x, placed.y))
                            });
                            Some((nid, Some(node_el), pos))
                        } else {
                            let is_dense = SYS_DENSE_CACHE.with(|c| {
                                c.borrow()
                                    .get(surface.canvas_id)
                                    .is_some_and(|e| e.is_dense)
                            });
                            if is_dense {
                                dom::get(surface.canvas_id).and_then(|canvas| {
                                    let rect = canvas.get_bounding_client_rect();
                                    let cam = load_camera(surface);
                                    let wx = (pe.client_x() as f64 - rect.left() - cam.pan_x)
                                        / cam.scale.max(0.05);
                                    let wy = (pe.client_y() as f64 - rect.top() - cam.pan_y)
                                        / cam.scale.max(0.05);
                                    let (0, idx) = graph_viewport::canvas2d::hit_test_scene(
                                        surface.canvas_id,
                                        wx,
                                        wy,
                                        4.0,
                                    )?
                                    else {
                                        return None;
                                    };
                                    let (nid, pos) = SYS_DENSE_CACHE.with(|c| {
                                        let g = c.borrow();
                                        let entry = g.get(surface.canvas_id)?;
                                        let placed = entry.layout.nodes.get(idx)?;
                                        Some((placed.id.clone(), (placed.x, placed.y)))
                                    })?;
                                    let el = mount_system_dense_node_if_needed(surface, &nid);
                                    Some((nid, el, Some(pos)))
                                })
                            } else {
                                None
                            }
                        };
                        if let Some((nid, maybe_el, cached_pos)) = resolved {
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
                                m::set(
                                    &drag_store_key,
                                    json!({
                                        "active": true,
                                        "id": nid,
                                        "start_cx": pe.client_x() as f64,
                                        "start_cy": pe.client_y() as f64,
                                        "orig_x": orig_x,
                                        "orig_y": orig_y,
                                        "moved": false,
                                    }),
                                );
                                return true;
                            }
                        }
                    }
                }
            } else if m::yes(&node_drag["active"]) {
                if let Ok(pe) = event.clone().dyn_into::<web_sys::PointerEvent>() {
                    let cam = load_camera(surface);
                    let nid = m::text(&node_drag, "id");
                    let dx = ((pe.client_x() as f64)
                        - node_drag["start_cx"].as_f64().unwrap_or(0.0))
                        / cam.scale.max(0.1);
                    let dy = ((pe.client_y() as f64)
                        - node_drag["start_cy"].as_f64().unwrap_or(0.0))
                        / cam.scale.max(0.1);
                    let nx = (node_drag["orig_x"].as_f64().unwrap_or(0.0) + dx).max(8.0);
                    let ny = (node_drag["orig_y"].as_f64().unwrap_or(0.0) + dy).max(8.0);
                    if kind == "pointermove" {
                        node_drag["moved"] = json!(dx.abs() > 2.0 || dy.abs() > 2.0);
                        m::set(&drag_store_key, node_drag);
                        if let Some(canvas) = dom::get(surface.canvas_id) {
                            if let Ok(Some(el)) = canvas
                                .query_selector(&format!(".system-map-node[data-n=\"{nid}\"]"))
                            {
                                let _ = el.set_attribute("data-pinned", "true");
                                if let Ok(he) = el.dyn_into::<web_sys::HtmlElement>() {
                                    let _ = he.style().set_property("left", &format!("{nx:.1}px"));
                                    let _ = he.style().set_property("top", &format!("{ny:.1}px"));
                                }
                            }
                        }
                        return true;
                    }
                    let moved = m::yes(&node_drag["moved"]);
                    m::set(&drag_store_key, V::Null);
                    if kind == "pointerup" && moved {
                        let state = surface_state(surface);
                        graph_viewport::save_pin(&sys_pin_key(surface, &state), &nid, (nx, ny));
                        graph_viewport::announce_live(
                            &format!("{}-live-announce", surface.id_prefix),
                            &format!("Pinned {nid}"),
                        );
                        render_surface(surface, false);
                    }
                    return true;
                }
            }
        }

        let mut drag = graph_viewport::DragPan::from_json(&m::data(surface.drag_key));
        let drag_active = drag.active;

        if kind == "dblclick" && in_canvas && !in_radar {
            if let Some(node_el) = target_el
                .as_ref()
                .and_then(|el| el.closest("[data-n]").ok().flatten())
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
                    let rect = canvas.get_bounding_client_rect();
                    let cam = load_camera(surface);
                    let wx = (pe.client_x() as f64 - rect.left() - cam.pan_x) / cam.scale.max(0.05);
                    let wy = (pe.client_y() as f64 - rect.top() - cam.pan_y) / cam.scale.max(0.05);
                    let hit =
                        graph_viewport::canvas2d::hit_test_scene(surface.canvas_id, wx, wy, 8.0);
                    graph_viewport::canvas2d::paint_scene(
                        surface.canvas_id,
                        cam.viewport_cam(),
                        hit,
                        true,
                    );
                    if let Ok(he) = canvas.dyn_into::<web_sys::HtmlElement>() {
                        let _ = he
                            .style()
                            .set_property("cursor", if hit.is_some() { "pointer" } else { "" });
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
                    } else if target_el
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
                            if let (Ok(pe), Some(canvas)) = (
                                event.clone().dyn_into::<web_sys::PointerEvent>(),
                                dom::get(surface.canvas_id),
                            ) {
                                let rect = canvas.get_bounding_client_rect();
                                let wx = (pe.client_x() as f64 - rect.left() - cam.pan_x)
                                    / cam.scale.max(0.05);
                                let wy = (pe.client_y() as f64 - rect.top() - cam.pan_y)
                                    / cam.scale.max(0.05);
                                if let Some(hit) = graph_viewport::canvas2d::hit_test_scene(
                                    surface.canvas_id,
                                    wx,
                                    wy,
                                    8.0,
                                ) {
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
                                            state["selected_edge"] = V::Null;
                                        } else {
                                            state["selected_edge"] = json!({"from": a, "to": b});
                                        }
                                        state["side_tab"] = json!("details");
                                        save_surface_state(surface, &state);
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
                    let _ = mount_system_dense_node_if_needed(surface, &next_id);
                    if let Some(canvas) = dom::get(surface.canvas_id) {
                        if let Ok(nodes) = canvas.query_selector_all(".system-map-node[data-n]") {
                            for i in 0..nodes.length() {
                                if let Some(el) = nodes
                                    .item(i)
                                    .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
                                {
                                    let is_next = el.get_attribute("data-n").as_deref()
                                        == Some(next_id.as_str());
                                    let _ = el.set_attribute(
                                        "tabindex",
                                        if is_next { "0" } else { "-1" },
                                    );
                                    if is_next {
                                        if let Ok(he) = el.dyn_into::<web_sys::HtmlElement>() {
                                            let _ = he.focus();
                                        }
                                    }
                                }
                            }
                        }
                    }
                    graph_viewport::announce_live(
                        &format!("{}-live-announce", surface.id_prefix),
                        &format!("Focused {next_id}"),
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
        if kind == "click" {
            if id == format!("{p}-zoom-in") {
                zoom_surface_at(surface, ZOOM_STEP, None);
                return true;
            }
            if id == format!("{p}-zoom-out") {
                zoom_surface_at(surface, 1.0 / ZOOM_STEP, None);
                return true;
            }
            if id == format!("{p}-zoom-level") {
                reset_surface_camera_100(surface);
                return true;
            }
            if id == format!("{p}-fit") {
                fit_surface_camera(surface);
                return true;
            }
            if id == format!("{p}-expand") {
                let mut state = surface_state(surface);
                state["expanded"] = json!(!m::yes(&state["expanded"]));
                save_surface_state(surface, &state);
                sync_toolbar_buttons(surface, &state);
                fit_surface_camera(surface);
                return true;
            }
            if id == format!("{p}-unfold") {
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
            if id == format!("{p}-certain") {
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
            if id == format!("{p}-layers") {
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
            if id == format!("{p}-reset-pins") {
                let state = surface_state(surface);
                graph_viewport::clear_pins(&sys_pin_key(surface, &state));
                render_surface(surface, false);
                graph_viewport::announce_live(&format!("{p}-live-announce"), "Reset pinned layout");
                return true;
            }
            if id == format!("{p}-isolate-toggle") {
                let mut state = surface_state(surface);
                let next = !m::yes(&state["isolate_active"]);
                let seed = m::text(&state, "focus");
                let depth = dom::value(&format!("{p}-isolate-depth"))
                    .trim()
                    .parse::<u64>()
                    .unwrap_or(2)
                    .clamp(1, 3);
                state["isolate_active"] = json!(next && !seed.is_empty());
                state["isolate_depth"] = json!(depth);
                if !seed.is_empty() {
                    state["isolate_seed"] = json!(seed);
                }
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
            if id == format!("{p}-path-trace") {
                let from_q = dom::value(&format!("{p}-path-from"));
                let to_q = dom::value(&format!("{p}-path-to"));
                let mut state = surface_state(surface);
                state["tool"] = json!("path");
                state["path_from"] = json!(from_q.trim());
                state["path_to"] = json!(to_q.trim());
                save_surface_state(surface, &state);
                render_surface(surface, false);
                graph_viewport::announce_live(&format!("{p}-live-announce"), "Updated path trace");
                return true;
            }
            if id == format!("{p}-path-clear") {
                let mut state = surface_state(surface);
                state["tool"] = json!("");
                state["path_from"] = json!("");
                state["path_to"] = json!("");
                save_surface_state(surface, &state);
                render_surface(surface, false);
                graph_viewport::announce_live(&format!("{p}-live-announce"), "Cleared path trace");
                return true;
            }
        }
        if kind == "change" && id == format!("{p}-isolate-depth") {
            let depth = dom::value(id)
                .trim()
                .parse::<u64>()
                .unwrap_or(2)
                .clamp(1, 3);
            let mut state = surface_state(surface);
            state["isolate_depth"] = json!(depth);
            state["isolate_active"] = json!(!m::text(&state, "focus").is_empty());
            save_surface_state(surface, &state);
            render_surface(surface, true);
            return true;
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
                spawn_local(async move {
                    run_finder_search(surface, q).await;
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
            let mut state = surface_state(surface);
            state["focus"] = json!(nid);
            state["selected_edge"] = V::Null;
            state["side_tab"] = json!("details");
            save_surface_state(surface, &state);
            render_surface(surface, false);
            true
        }
        "system-map-edge" => {
            let mut state = surface_state(surface);
            state["selected_edge"] = json!({
                "from": m::text(payload, "from"),
                "to": m::text(payload, "to"),
            });
            state["side_tab"] = json!("details");
            save_surface_state(surface, &state);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_catalog() -> V {
        json!({
            "system": { "name": "Harbor Market", "root": "harbor-system" },
            "shared_sources": ["policies/contracts.md"],
            "services": [
                {
                    "id": "gateway",
                    "name": "API Gateway",
                    "role": "edge-gateway",
                    "language": "typescript",
                    "provides": [],
                    "consumes": [
                        { "name": "orders.http", "kind": "http" },
                        { "name": "catalog.graphql", "kind": "graphql" }
                    ]
                },
                {
                    "id": "orders",
                    "name": "Orders",
                    "language": "php",
                    "provides": [
                        { "name": "orders.http", "kind": "http", "version": "1.0" },
                        { "name": "order.placed", "kind": "event", "version": "2.0" }
                    ],
                    "consumes": [
                        { "name": "payments.charge", "kind": "rpc" },
                        { "name": "inventory.reserve", "kind": "http" }
                    ],
                    "shared_sources": ["policies/contracts.md"]
                },
                {
                    "id": "inventory",
                    "name": "Inventory",
                    "language": "php",
                    "provides": [{ "name": "inventory.reserve", "kind": "http" }],
                    "consumes": [{ "name": "order.placed", "kind": "event" }]
                },
                {
                    "id": "payments",
                    "name": "Payments",
                    "role": "core-platform",
                    "language": "go",
                    "provides": [{ "name": "payments.charge", "kind": "rpc" }],
                    "consumes": [{ "name": "missing.audit", "kind": "other" }]
                },
                {
                    "id": "catalog",
                    "name": "Catalog",
                    "language": "php",
                    "provides": [{ "name": "catalog.graphql", "kind": "graphql" }],
                    "consumes": []
                }
            ]
        })
    }

    #[test]
    fn normalizes_all_seven_contract_arrow_kinds() {
        assert_eq!(normalize_contract_arrow_kind("REST"), "http");
        assert_eq!(normalize_contract_arrow_kind("kafka"), "event");
        assert_eq!(normalize_contract_arrow_kind("gRPC"), "rpc");
        assert_eq!(normalize_contract_arrow_kind("gql"), "graphql");
        assert_eq!(normalize_contract_arrow_kind("custom"), "other");
        assert_eq!(normalize_contract_arrow_kind("shared-source"), "shared");
        assert_eq!(normalize_contract_arrow_kind("imports"), "code");
        let legend = contract_arrow_legend_html();
        for kind in ["http", "event", "rpc", "graphql", "other", "shared", "code"] {
            assert!(legend.contains(&format!("data-sys-edge-kind=\"{kind}\"")));
        }
        assert!(legend.contains("Broken risk"));
        assert!(legend.contains("Undeclared drift"));
    }

    #[test]
    fn synthesizes_topology_and_detects_cycles_and_broken_contracts() {
        let doc = synthesize_map_from_catalog(&sample_catalog(), &json!({}));
        let nodes = m::array(&doc["nodes"]);
        let edges = m::array(&doc["edges"]);
        assert_eq!(nodes.len(), 6); // 5 member services + __system__
        assert!(edges
            .iter()
            .any(|e| e["from"] == "gateway" && e["to"] == "orders" && e["kind"] == "http"));
        assert!(edges
            .iter()
            .any(|e| e["from"] == "orders" && e["to"] == "payments" && e["kind"] == "rpc"));
        // Unmatched contract missing.audit flags broken_risk!
        assert!(edges
            .iter()
            .any(|e| e["from"] == "payments" && m::yes(&e["broken_risk"])));

        // orders <-> inventory form a 2-service cycle (orders -> inventory.reserve, inventory -> order.placed)
        let cycles = find_cross_service_cycles(&doc);
        assert_eq!(cycles.len(), 1);
        assert!(cycles[0].contains(&"orders".to_owned()));
        assert!(cycles[0].contains(&"inventory".to_owned()));
    }

    #[test]
    fn computes_blast_radius_depths_and_context_brief() {
        let doc = synthesize_map_from_catalog(&sample_catalog(), &json!({}));
        // Payments is consumed by Orders (wave 1), which is consumed by Gateway & Inventory (wave 2)
        let full = compute_blast_radius(&doc, &["payments".to_owned()], &[], 32);
        assert!(full.seeds.contains("payments"));
        assert!(full.direct_consumers.contains("orders"));
        assert!(full.downstream_consumers.contains("gateway"));
        assert!(full.downstream_consumers.contains("inventory"));

        let direct_only = compute_blast_radius(&doc, &["payments".to_owned()], &[], 1);
        assert!(direct_only.direct_consumers.contains("orders"));
        assert!(direct_only.downstream_consumers.is_empty());

        let none = compute_blast_radius(&doc, &["payments".to_owned()], &[], 0);
        assert!(none.direct_consumers.is_empty());

        let mut state = default_surface_state();
        state["overlay"] = json!({
            "seed_services": ["payments"],
            "depth": 32,
            "changed_contracts": []
        });
        let brief = build_context_brief(&doc, &state);
        assert!(brief.contains("payments"));
        assert!(brief.contains("orders"));
        assert!(brief.contains("gateway"));
    }

    #[test]
    fn computes_paths_shared_neighbors_and_layers() {
        let doc = synthesize_map_from_catalog(&sample_catalog(), &json!({}));
        let paths = shortest_service_paths(&doc, "gateway", "payments", &BTreeSet::new());
        assert_eq!(paths, vec![vec!["gateway", "orders", "payments"]]);

        let mut skip = BTreeSet::new();
        skip.insert("orders".to_owned());
        assert!(shortest_service_paths(&doc, "gateway", "payments", &skip).is_empty());

        let (both_use, uses_both) = shared_neighbors(&doc, "gateway", "inventory");
        assert!(both_use.contains(&"orders".to_owned()));
        assert!(uses_both.is_empty());

        let (tiers, _) = assign_service_layers(&doc);
        assert_eq!(tiers.get("gateway"), Some(&0));
        assert_eq!(tiers.get("__system__"), Some(&3));
    }

    #[test]
    fn cursor_anchored_zoom_keeps_world_point_under_cursor() {
        let anchor = (300.0, 200.0);
        let (s1, px1, py1) = (1.0, 40.0, 30.0);
        let wx = (anchor.0 - px1) / s1;
        let wy = (anchor.1 - py1) / s1;
        let (s2, px2, py2) = zoom_at(s1, px1, py1, anchor, 1.5);
        assert!((s2 - 1.5).abs() < 1e-6);
        let screen_x = wx * s2 + px2;
        let screen_y = wy * s2 + py2;
        assert!((screen_x - anchor.0).abs() <= 1.0);
        assert!((screen_y - anchor.1).abs() <= 1.0);
    }

    #[test]
    fn renders_svg_canvas_with_explain_attributes_and_unfolded_frames() {
        let mut state = default_surface_state();
        state["spread"] = json!(["orders"]);
        state["overlay"] = json!({
            "seed_services": ["orders"],
            "depth": 1,
            "step_states": { "orders": "running" },
            "cross_service_unopened": ["inventory"]
        });
        let doc = expand_unfolded_nodes(
            &synthesize_map_from_catalog(&sample_catalog(), &json!({})),
            &state,
        );
        let (html, w, h) = render_canvas_html(CHANGES_SURFACE, &doc, &state);
        assert!(w > 300.0 && h > 200.0);
        assert!(html.contains("data-explain-services=\"orders\""));
        assert!(html.contains("data-step-state=\"running\""));
        assert!(html.contains("data-unopened=\"true\""));
        assert!(html.contains("system-service-frame"));
    }

    #[test]
    fn normalize_map_doc_propagates_contract_risks_and_undeclared_code_drift() {
        let raw = json!({
            "nodes": [
                { "id": "gateway", "kind": "service" },
                { "id": "orders", "kind": "service" }
            ],
            "edges": [
                { "from": "gateway", "to": "orders", "kind": "code", "undeclared_in_code": true }
            ],
            "contract_risks": [
                { "consumer": "gateway", "provider": "orders", "contract": "orders.http", "kind": "http" },
                { "consumer": "orders", "contract": "billing.invoice", "kind": "rpc" }
            ]
        });
        let doc = normalize_map_doc(
            &raw,
            &sample_catalog(),
            &json!({}),
            &default_surface_state(),
        );
        let edges = m::array(&doc["edges"]);
        let gw_orders = edges
            .iter()
            .find(|e| e["from"] == "gateway" && e["to"] == "orders")
            .expect("gateway -> orders edge");
        assert_eq!(gw_orders["provenance"], "undeclared_in_code");
        assert_eq!(gw_orders["broken_risk"], true);
        assert!(edges.iter().any(|e| e["from"] == "orders"
            && e["to"] == "__unmatched__:billing.invoice"
            && e["broken_risk"] == true));
    }

    #[test]
    fn normalizes_backend_catalog_declared_nesting_and_system_node_schema() {
        let backend_catalog = json!({
            "schema_version": 1,
            "system": "Harbor Market",
            "services": [
                {
                    "id": "gateway",
                    "root": "services/gateway",
                    "access": "available",
                    "declared": {
                        "owner": "edge-team",
                        "description": "Public HTTP gateway",
                        "stack": "typescript",
                        "capabilities": [{ "id": "public-routing", "description": "Edge routing" }],
                        "provides": [],
                        "consumes": [{ "service": "orders", "contract": "orders-api", "version": "1.0" }]
                    }
                },
                {
                    "id": "orders",
                    "root": "services/orders",
                    "access": "available",
                    "declared": {
                        "owner": "checkout-team",
                        "description": "Order lifecycle",
                        "stack": "php",
                        "capabilities": [{ "id": "order-checkout", "description": "Checkout flow" }],
                        "provides": [{ "id": "orders-api", "kind": "http", "version": "1.2", "sources": ["routes/api.php"] }],
                        "consumes": []
                    }
                }
            ],
            "relationships": [],
            "warnings": []
        });

        let raw_map = json!({
            "nodes": [
                {
                    "id": "gateway",
                    "label": "API Gateway",
                    "kind": "service",
                    "service": "gateway",
                    "root": "services/gateway",
                    "stack": "typescript"
                },
                {
                    "id": "orders",
                    "label": "Orders Service",
                    "kind": "service",
                    "service": "orders",
                    "root": "services/orders",
                    "stack": "php"
                },
                {
                    "id": "contract:orders:orders-api",
                    "label": "orders-api",
                    "kind": "contract",
                    "service": "orders",
                    "parent": "orders",
                    "contract_kind": "http",
                    "version": "1.2"
                },
                {
                    "id": "source:orders:routes/api.php",
                    "label": "routes/api.php",
                    "kind": "source",
                    "service": "orders",
                    "parent": "orders",
                    "path": "routes/api.php"
                }
            ],
            "edges": [
                {
                    "from": "gateway",
                    "to": "orders",
                    "consumer": "gateway",
                    "provider": "orders",
                    "level": "service",
                    "kinds": ["http"],
                    "provenance": "verified",
                    "certainty": "EXTRACTED",
                    "broken_risk": true,
                    "contracts": [
                        {
                            "id": "orders-api",
                            "kind": "http",
                            "consumer_version": "1.0",
                            "provider_version": "1.2",
                            "provider_sources": ["routes/api.php"],
                            "consumer_sources": ["src/OrdersClient.ts"],
                            "broken_risk": true
                        }
                    ]
                },
                {
                    "from": "gateway",
                    "to": "contract:orders:orders-api",
                    "level": "detail",
                    "kinds": ["http"],
                    "provenance": "verified",
                    "certainty": "EXTRACTED",
                    "broken_risk": false,
                    "contracts": []
                },
                {
                    "from": "contract:orders:orders-api",
                    "to": "source:orders:routes/api.php",
                    "level": "detail",
                    "kinds": ["code"],
                    "provenance": "verified",
                    "certainty": "EXTRACTED",
                    "broken_risk": false,
                    "contracts": []
                }
            ]
        });

        let mut state = default_surface_state();
        state["spread"] = json!(["orders"]);
        let doc = normalize_map_doc(&raw_map, &backend_catalog, &json!({}), &state);

        let nodes = m::array(&doc["nodes"]);
        let orders = nodes
            .iter()
            .find(|n| n["id"] == "orders")
            .expect("orders node");
        assert_eq!(orders["name"], "Orders Service");
        assert_eq!(orders["language"], "php");
        assert_eq!(orders["folder"], "services/orders");
        assert_eq!(orders["owner"], "checkout-team");
        assert!(nodes
            .iter()
            .any(|n| n["id"] == "__system__" && m::text(n, "name").contains("Harbor Market")));

        let detail_edge = m::array(&doc["edges"])
            .into_iter()
            .find(|e| e["to"] == "contract:orders:orders-api")
            .expect("detail edge");
        assert_eq!(detail_edge["child_link"], true);

        let (svg_html, _, _) = render_canvas_html(SERVICES_SURFACE, &doc, &state);
        assert!(!svg_html.contains("data-explain-services=\"gateway,contract:orders"));
        assert!(svg_html.contains("data-explain-services=\"gateway,orders\""));
        assert!(svg_html.contains("data-explain-services=\"orders\""));

        state["focus"] = json!("orders");
        let node_side = render_side_html(SERVICES_SURFACE, &doc, &state);
        assert!(node_side.contains("order-checkout"));

        state["selected_edge"] = json!({ "from": "gateway", "to": "orders" });
        let edge_side = render_side_html(SERVICES_SURFACE, &doc, &state);
        assert!(edge_side.contains("1.0 → 1.2"));
        assert!(edge_side.contains("routes/api.php"));
        assert!(edge_side.contains("Open orders map"));
    }

    #[test]
    fn renders_backend_changes_and_impact_contract_consumers_shapes() {
        m::set(
            SERVICES_SURFACE.impact_key,
            json!({
                "service": "orders",
                "impacted_contracts": [{ "contract": "orders-api", "kind": "http" }],
                "contract_consumers": [
                    {
                        "provider": "orders",
                        "consumer": "gateway",
                        "contract": "orders-api",
                        "consumer_sources": ["src/OrdersClient.ts"],
                        "unopened_sources": ["src/OrdersClient.ts"],
                        "broken_risk": false
                    }
                ],
                "cross_service_unopened": [
                    { "consumer": "gateway", "contract": "orders-api", "path": "src/OrdersClient.ts" }
                ]
            }),
        );
        m::set(
            SERVICES_SURFACE.changes_key,
            json!({
                "system": "Harbor Market",
                "total_files": 1,
                "services": [
                    {
                        "service": "orders",
                        "changed_files": [{ "path": "src/OrderController.php" }]
                    }
                ]
            }),
        );
        let mut state = default_surface_state();
        state["side_tab"] = json!("changes");
        let html = render_side_html(
            SERVICES_SURFACE,
            &json!({ "nodes": [], "edges": [] }),
            &state,
        );
        assert!(html.contains("Impact for orders-api"));
        assert!(html.contains("gateway (orders-api) src/OrdersClient.ts"));
        assert!(html.contains("src/OrderController.php"));
    }

    #[test]
    fn a_link_between_services_shows_the_code_on_both_sides_when_asked() {
        // Not asked yet, or asked for another link: a button.
        let html = bridge_html(SERVICES_SURFACE, "web", "orders", &V::Null);
        assert!(html.contains("<h5 class=\"system-side-subheading\">Code on both sides</h5>"));
        assert!(html.contains("data-action=\"system-map-bridge\""), "{html}");
        assert!(
            html.contains("&quot;from&quot;:&quot;web&quot;")
                && html.contains("&quot;to&quot;:&quot;orders&quot;")
        );
        let bridge = json!({
            "kind": "system_bridge", "from": "web", "to": "orders",
            "contracts": ["orders-http-v1"],
            "consumer": {"service": "web", "graph": false, "more": 0, "sites": [
                {"path": "src/OrdersClient.ts", "contracts": ["orders-http-v1"], "in_graph": false, "reach": null}
            ]},
            "provider": {"service": "orders", "graph": true, "more": 2, "cut": true, "sites": [
                {"path": "src/OrderController.php", "contracts": ["orders-http-v1"], "declared": true, "found": false, "in_graph": true, "reach": {"files": 3, "tests": 1}},
                {"path": "src/routes.php", "contracts": ["orders-http-v1"], "declared": false, "found": true, "in_graph": true, "reach": {"files": 0, "tests": 0}}
            ]}
        });
        assert!(bridge_html(SERVICES_SURFACE, "billing", "orders", &bridge)
            .contains("system-map-bridge"));
        let html = bridge_html(SERVICES_SURFACE, "web", "orders", &bridge);
        assert!(!html.contains("system-map-bridge"));
        assert!(html.contains("<p class=\"system-note\"><strong>web</strong> uses it · no code graph yet: how far each file reaches is not told</p>"), "{html}");
        assert!(html.contains("<p class=\"system-note\"><strong>orders</strong> makes it</p>"));
        assert!(html.contains("<code>src/OrdersClient.ts</code><div class=\"system-note\">names it: orders-http-v1</div>"));
        assert!(html.contains("<code>src/OrderController.php</code><div class=\"system-note\">made here by its passport: orders-http-v1 · a change reaches 3 files of orders</div>"));
        assert!(html.contains("<code>src/routes.php</code><div class=\"system-note\">names it: orders-http-v1 · nothing else in it uses this file</div>"));
        // A document that names the contract says it is one.
        let mut documented = bridge.clone();
        documented["consumer"]["sites"][0]["path"] = json!("README.md");
        documented["consumer"]["sites"][0]["document"] = json!(true);
        assert!(bridge_html(SERVICES_SURFACE, "web", "orders", &documented)
            .contains("<code>README.md</code><div class=\"system-note\">a document that names it: orders-http-v1</div>"));
        // Each file opens on the code map of its service.
        assert!(html.contains("data-action=\"inspect-system-file-graph\" data-payload=\"{&quot;path&quot;:&quot;src/OrderController.php&quot;,&quot;service&quot;:&quot;orders&quot;}\""), "{html}");
        assert_eq!(html.matches("inspect-system-file-graph").count(), 3);
        assert!(html.contains("<p class=\"system-note\">and 2 more</p>"));
        assert!(html.contains("Not every file of it was read: there may be more."));
        // A side with nothing to show says so.
        let mut empty = bridge.clone();
        empty["consumer"]["sites"] = json!([]);
        assert!(bridge_html(SERVICES_SURFACE, "web", "orders", &empty)
            .contains("No file was found to name it."));
    }

    #[test]
    fn the_waves_of_a_change_are_stepped_through_and_what_is_not_reached_yet_waits() {
        let doc = synthesize_map_from_catalog(&sample_catalog(), &json!({}));
        let mut state = default_surface_state();
        state["overlay"] =
            json!({"seed_services": ["payments"], "depth": 32, "changed_contracts": []});
        // payments is where it starts, orders is wave 1, gateway and
        // inventory are wave 2.
        let blast = blast_of(&doc, &state);
        assert_eq!(last_wave(&blast), 2);
        let row = wave_scrubber_html(CHANGES_SURFACE, &blast, &state);
        assert!(row.starts_with("<div class=\"system-waves\" role=\"group\" aria-label=\"Waves of the change\"><span class=\"graph-arrow-legend-title\">Waves:</span>"));
        assert!(
            row.contains(
                "aria-pressed=\"true\" title=\"Every service the change reaches\">All</button>"
            ),
            "{row}"
        );
        assert!(row.contains("title=\"Where the change starts: 1 service\">0</button>"));
        assert!(
            row.contains("title=\"As far as wave 1: 1 service more\">1</button>"),
            "{row}"
        );
        assert!(row.contains("title=\"As far as wave 2: 2 services more\">2</button>"));
        assert!(row.contains("data-action=\"system-map-wave-play\""));
        // As far as wave 1: gateway and inventory wait, faint, and so do
        // the links from them; orders is reached.
        state["wave"] = json!(1);
        let row = wave_scrubber_html(CHANGES_SURFACE, &blast, &state);
        assert!(row.contains(
            "aria-pressed=\"true\" title=\"As far as wave 1: 1 service more\">1</button>"
        ));
        assert!(row.contains(
            "aria-pressed=\"false\" title=\"Every service the change reaches\">All</button>"
        ));
        let (html, _, _) = render_canvas_html(CHANGES_SURFACE, &doc, &state);
        let waiting = |id: &str| {
            let at = html
                .find(&format!("data-n=\"{id}\""))
                .unwrap_or_else(|| panic!("{id} in {html}"));
            let tag = &html[at..at + html[at..].find('>').unwrap()];
            tag.contains("data-beyond=\"true\"")
        };
        assert!(waiting("gateway") && waiting("inventory"));
        assert!(!waiting("orders") && !waiting("payments"));
        // The links the change has not passed yet wait as well: those
        // from the services of wave 2.
        let links = html
            .split("<path class=\"graph-map-edge system-map-edge")
            .skip(1)
            .map(|tag| &tag[..tag.find('>').unwrap()])
            .collect::<Vec<_>>();
        let link_waits = |from: &str, to: &str| {
            links
                .iter()
                .find(|tag| tag.contains(&format!("data-from=\"{from}\" data-to=\"{to}\"")))
                .unwrap_or_else(|| panic!("{from} -> {to} in {html}"))
                .contains("data-beyond=\"true\"")
        };
        assert!(link_waits("gateway", "orders"));
        assert!(!link_waits("orders", "payments"));
        // All of it again: nothing waits.
        state["wave"] = V::Null;
        let (html, _, _) = render_canvas_html(CHANGES_SURFACE, &doc, &state);
        assert!(!html.contains("data-beyond"));
        // A change that reaches nothing past where it starts has no waves.
        state["overlay"] =
            json!({"seed_services": ["gateway"], "depth": 32, "changed_contracts": []});
        assert_eq!(
            wave_scrubber_html(CHANGES_SURFACE, &blast_of(&doc, &state), &state),
            ""
        );
    }

    #[test]
    fn hover_pointed_key_and_edge_adjacency_dedup_cleanly() {
        assert_eq!(hover_pointed_key(None, None), "");
        assert_eq!(hover_pointed_key(Some("orders"), None), "n:orders");
        assert_ne!(
            hover_pointed_key(Some("orders"), None),
            hover_pointed_key(None, Some(("orders", "payments")))
        );
        assert_eq!(
            hover_pointed_key(None, Some(("orders", "payments"))),
            "e:orders\u{1f}payments"
        );

        let adj = build_edge_adjacency(&[
            ("gateway".to_owned(), "orders".to_owned()),
            ("orders".to_owned(), "payments".to_owned()),
            ("".to_owned(), "ignored".to_owned()),
        ]);
        assert_eq!(
            adj.get("orders").cloned().unwrap_or_default(),
            BTreeSet::from(["gateway".to_owned(), "payments".to_owned()])
        );
        assert_eq!(
            adj.get("gateway").cloned().unwrap_or_default(),
            BTreeSet::from(["orders".to_owned()])
        );
        assert!(!adj.contains_key("ignored"));
    }

    #[test]
    fn barycenter_ordering_and_port_spreading_untangle_crossed_services() {
        // Inputs in columns are inverted relative to edges:
        // Column 0: a, b  |  Column 1: y (connected to a), x (connected to b)
        // WAIT: to force a crossing before barycenter:
        // col 0: top=a, bottom=b; edges: a -> x, b -> y; if x was added after y, raw order puts y above x causing a crossing!
        let doc = json!({
            "nodes": [
                {"id": "a", "name": "Service A", "kind": "service"},
                {"id": "b", "name": "Service B", "kind": "service"},
                {"id": "y", "name": "Service Y", "kind": "service"},
                {"id": "x", "name": "Service X", "kind": "service"}
            ],
            "edges": [
                {"from": "a", "to": "x", "kind": "http"},
                {"from": "b", "to": "y", "kind": "rpc"},
                {"from": "a", "to": "y", "kind": "event"}
            ]
        });
        let layout = layout_system_nodes(&doc, false);
        let pos = |id: &str| {
            layout
                .nodes
                .iter()
                .find(|n| n.id == id)
                .map(|n| (n.x, n.y))
                .expect(id)
        };
        // `a` connects to both `x` and `y`, while `b` connects only to `y`.
        // Note `a` has TWO outgoing right-side wires (`a -> x`, `a -> y`), so their start Y ports must spread apart!
        assert_eq!(pos("a").0, MARGIN);
        let wire_starts: Vec<f64> = layout
            .wires
            .iter()
            .filter(|w| doc["edges"][w.edge_index]["from"] == "a")
            .filter_map(|w| {
                let after_m = w.path_d.strip_prefix('M')?;
                let first_pair = after_m.split_whitespace().next()?;
                let (_, y_str) = first_pair.split_once(',')?;
                y_str.parse::<f64>().ok()
            })
            .collect();
        assert_eq!(wire_starts.len(), 2);
        assert!(
            (wire_starts[0] - wire_starts[1]).abs() >= 2.0,
            "expected spread ports on multi-edge source, got {:?}",
            wire_starts
        );

        // Now verify crossing removal when two disjoint edges cross in input order:
        let crossed = json!({
            "nodes": [
                {"id": "g1", "name": "G1", "kind": "service"},
                {"id": "g2", "name": "G2", "kind": "service"},
                {"id": "s_for_g2", "name": "S2", "kind": "service"},
                {"id": "s_for_g1", "name": "S1", "kind": "service"}
            ],
            "edges": [
                {"from": "g1", "to": "s_for_g1", "kind": "http"},
                {"from": "g2", "to": "s_for_g2", "kind": "http"}
            ]
        });
        let cl = layout_system_nodes(&crossed, false);
        let y_of = |id: &str| cl.nodes.iter().find(|n| n.id == id).unwrap().y;
        assert!(
            y_of("s_for_g1") < y_of("s_for_g2"),
            "barycenter sweep should align s_for_g1 above s_for_g2 matching g1 above g2"
        );
    }

    #[test]
    fn contract_legend_switchboard_toggles_solos_and_hides_matching_edges() {
        let doc = synthesize_map_from_catalog(&sample_catalog(), &json!({}));
        let mut state = default_surface_state();

        let legend = contract_arrow_legend_html_for(SERVICES_SURFACE, &doc, &state);
        crate::test_support::assert_no_inline_hex_colors(&legend);
        assert!(legend.contains("data-action=\"system-map-kind-toggle\""));
        assert!(legend.contains("<button type=\"button\" class=\"graph-arrow-legend-item\""));
        assert!(!legend.contains("Show all"));

        // Toggle rpc off -> Show all appears and rpc edge gets data-kind-hidden="true"
        state = switched_contract_kind(state, "rpc", false);
        assert_eq!(
            hidden_contract_kinds(&state),
            [false, false, true, false, false, false, false]
        );
        let legend_off = contract_arrow_legend_html_for(SERVICES_SURFACE, &doc, &state);
        assert!(legend_off.contains("Show all"));
        assert!(legend_off.contains("data-sys-edge-kind=\"rpc\" aria-pressed=\"false\""));

        let (canvas, _, _) = render_canvas_html(SERVICES_SURFACE, &doc, &state);
        crate::test_support::assert_no_inline_hex_colors(&canvas);
        assert!(canvas.contains("data-sys-edge-kind=\"rpc\""));
        assert!(canvas.contains("data-kind-hidden=\"true\""));

        // Alt+click solo on http isolates http only; second solo resets to all visible.
        let solo = switched_contract_kind(default_surface_state(), "http", true);
        assert_eq!(
            hidden_contract_kinds(&solo),
            [false, true, true, true, true, true, true]
        );
        let reset = switched_contract_kind(solo, "http", true);
        assert_eq!(hidden_contract_kinds(&reset), [false; 7]);
        let preset_all = preset_contract_kinds(state, "all");
        assert_eq!(hidden_contract_kinds(&preset_all), [false; 7]);
    }

    #[test]
    fn system_radar_lod_and_edge_label_deoverlap_work_cleanly() {
        assert_eq!(lod(0.4), "far");
        assert_eq!(lod(0.9), "mid");
        assert_eq!(lod(1.4), "near");
        assert_eq!(lod(f64::NAN), "far");

        let doc = json!({
            "nodes": [
                {"id": "gw", "name": "Gateway", "kind": "service"},
                {"id": "ord", "name": "Orders", "kind": "service"}
            ],
            "edges": [
                {"from": "gw", "to": "ord", "kind": "http", "contracts": [{"id": "c1"}]},
                {"from": "gw", "to": "ord", "kind": "rpc", "contracts": [{"id": "c2"}]}
            ]
        });
        let mut state = default_surface_state();
        state["focus"] = json!("gw");
        let (plane_html, radar_svg, w, h) = render_canvas_with_radar(CHANGES_SURFACE, &doc, &state);
        crate::test_support::assert_no_inline_hex_colors(&plane_html);
        crate::test_support::assert_no_inline_hex_colors(&radar_svg);
        assert!(radar_svg.contains("class=\"graph-radar-picture\""));
        assert!(radar_svg.contains("data-state=\"selected\""));
        assert!(radar_svg.contains("class=\"graph-radar-view\""));

        // Parallel edge labels between gw -> ord must be de-overlapped to distinct Y coordinates.
        let label_ys: Vec<f64> = plane_html
            .split("<g class=\"graph-map-edge-tie system-map-edge-label\"")
            .skip(1)
            .filter_map(|chunk| {
                let text_at = chunk.find("<text ")?;
                let y_at = chunk[text_at..].find(" y=\"")?;
                let start = text_at + y_at + 4;
                let end = start + chunk[start..].find('"')?;
                chunk[start..end].parse::<f64>().ok()
            })
            .collect();
        assert_eq!(label_ys.len(), 2);
        assert!(
            (label_ys[0] - label_ys[1]).abs() >= 16.0,
            "overlapping edge labels should step apart: {:?}",
            label_ys
        );

        assert!(radar_window_rect((0.0, 0.0), 1.0, (w + 100.0, h + 100.0), (w, h)).is_none());
        let partial = radar_window_rect((-40.0, -20.0), 1.5, (300.0, 200.0), (w, h)).unwrap();
        assert!(partial.2 > 0.0 && partial.3 > 0.0);
        let centered = centred((200.0, 120.0), 1.5, (600.0, 400.0));
        assert_eq!(centered, (0.0, 20.0));
    }

    #[test]
    fn system_map_delegates_camera_resize_and_radar_to_graph_viewport() {
        let fit_cam = graph_viewport::fit(
            (1200.0, 600.0),
            (800.0, 400.0),
            graph_viewport::FitOptions::for_system_map(),
        );
        assert_eq!(
            camera_for_fit((1200.0, 600.0), (800.0, 400.0)),
            (fit_cam.scale, fit_cam.tx, fit_cam.ty)
        );

        let mut cam = Camera::default();
        cam.apply_viewport_cam(fit_cam);
        assert_eq!(cam.viewport_cam(), fit_cam);

        assert!(matches!(
            graph_viewport::resize_refit_decision(
                cam.viewport_cam(),
                true,
                (1200.0, 600.0),
                (900.0, 500.0),
                true,
                graph_viewport::FitOptions::for_system_map(),
            ),
            graph_viewport::ResizeAction::Refit(_)
        ));
        assert_eq!(
            graph_viewport::resize_refit_decision(
                cam.viewport_cam(),
                false,
                (1200.0, 600.0),
                (900.0, 500.0),
                true,
                graph_viewport::FitOptions::for_system_map(),
            ),
            graph_viewport::ResizeAction::SyncRadar(fit_cam)
        );

        let mut st = default_surface_state();
        let doc = json!({"nodes": [], "edges": []});
        let details_html = render_side_html(SERVICES_SURFACE, &doc, &st);
        assert!(details_html.contains("aria-selected=\"true\" tabindex=\"0\""));
        assert_eq!(details_html.matches("tabindex=\"-1\"").count(), 2);

        st["side_tab"] = json!("analyze");
        let analyze_html = render_side_html(SERVICES_SURFACE, &doc, &st);
        assert_eq!(
            analyze_html
                .matches("aria-selected=\"true\" tabindex=\"0\"")
                .count(),
            1
        );
        assert_eq!(analyze_html.matches("tabindex=\"-1\"").count(), 2);
    }

    #[test]
    fn bridge_html_renders_semantic_framework_and_db_bridges_without_inline_hex_colors() {
        let bridge = json!({
            "kind": "system_bridge",
            "from": "web",
            "to": "orders",
            "contracts": ["orders-http-v1"],
            "consumer": {"service": "web", "graph": true, "more": 0, "sites": []},
            "provider": {"service": "orders", "graph": true, "more": 0, "sites": []},
            "semantic_bridges": {
                "routes": [{
                    "route": "/api/orders/{id}",
                    "method": "POST",
                    "methods": ["POST"],
                    "direction": "from->to",
                    "caller": "web",
                    "provider": "orders",
                    "caller_files": ["src/OrdersClient.ts"],
                    "provider_files": ["src/OrderController.php"]
                }],
                "events": [{
                    "event": "OrderCreated",
                    "direction": "to->from",
                    "producer": "orders",
                    "consumer": "web",
                    "producer_files": ["src/OrderService.php"],
                    "consumer_files": ["src/OrderSyncListener.ts"]
                }],
                "di_bindings": [{
                    "service": "PaymentGateway",
                    "from_kinds": ["binds"],
                    "to_kinds": ["implements"],
                    "from_files": ["src/Providers/AppProvider.ts"],
                    "to_files": ["src/StripeGateway.php"]
                }],
                "tables": [{
                    "table": "orders",
                    "coupling": "cross_read_write",
                    "from_access": ["read"],
                    "to_access": ["read", "write"],
                    "from_files": ["src/OrderQuery.ts"],
                    "to_files": ["src/OrderRepository.php"]
                }],
                "summary": {"routes": 1, "events": 1, "di_bindings": 1, "tables": 1, "total": 4}
            }
        });
        let html = bridge_html(SERVICES_SURFACE, "web", "orders", &bridge);
        crate::test_support::assert_no_inline_hex_colors(&html);
        assert!(
            html.contains("Semantic framework &amp; DB bridges"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"system-chip\">4</span>"),
            "{html}"
        );
        assert!(html.contains("/api/orders/{id}"), "{html}");
        assert!(html.contains("POST"), "{html}");
        assert!(html.contains("OrderCreated"), "{html}");
        assert!(html.contains("PaymentGateway"), "{html}");
        assert!(
            html.contains("data-coupling=\"cross_read_write\""),
            "{html}"
        );
        assert!(html.contains("read / write"), "{html}");
        assert!(html.contains("<code>orders</code>"), "{html}");
        let payloads = crate::test_support::payloads(&html, "inspect-system-file-graph");
        assert!(payloads.contains(&json!({"path": "src/OrderController.php", "service": "orders"})));
        assert!(payloads.contains(&json!({"path": "src/OrderSyncListener.ts", "service": "web"})));

        let mut empty_sem = bridge.clone();
        empty_sem["semantic_bridges"] = json!({
            "routes": [], "events": [], "di_bindings": [], "tables": []
        });
        let empty_html = bridge_html(SERVICES_SURFACE, "web", "orders", &empty_sem);
        crate::test_support::assert_no_inline_hex_colors(&empty_html);
        assert!(empty_html.contains("No shared routes, events, DI bindings, or tables detected."));
    }

    #[test]
    fn system_structure_digest_pins_isolate_and_roving_attrs() {
        let doc = json!({
            "nodes": [
                {"id": "api", "name": "api", "kind": "service"},
                {"id": "orders", "name": "orders", "kind": "service"},
                {"id": "db", "name": "db", "kind": "service"}
            ],
            "edges": [
                {"from": "api", "to": "orders", "kind": "http"},
                {"from": "orders", "to": "db", "kind": "code"}
            ]
        });
        let base_state = json!({"focus": "api"});
        let sel_state = json!({"focus": "orders"});
        assert_eq!(
            system_structure_digest(&doc, &base_state, &BTreeMap::new()),
            system_structure_digest(&doc, &sel_state, &BTreeMap::new()),
        );

        let isolated = filtered_doc(
            &doc,
            &json!({"isolate_active": true, "isolate_seed": "api", "isolate_depth": 1}),
        );
        let kept_ids: Vec<String> = m::array(&isolated["nodes"]).iter().map(node_id).collect();
        assert_eq!(kept_ids, vec!["api", "orders"]);

        let mut pins = BTreeMap::new();
        pins.insert("api".to_owned(), (222.0, 111.0));
        let layout = layout_system_nodes_with_pins(&doc, false, &pins);
        assert_eq!(layout.nodes[0].x, 222.0);
        assert_eq!(layout.nodes[0].y, 111.0);
        let html = render_canvas_html_with_layout(SERVICES_SURFACE, &doc, &base_state, &layout);
        assert!(html.contains("tabindex=\"0\""));
        assert!(html.contains("tabindex=\"-1\""));
        crate::test_support::assert_no_inline_hex_colors(&html);
    }

    #[test]
    fn system_digest_pin_deltas_and_dense_scene_sync_behave_consistently() {
        let doc = json!({
            "nodes": [
                {"id": "api", "name": "api", "kind": "service"},
                {"id": "api::c1", "name": "OrderCreate", "kind": "contract", "parent_service": "api"},
                {"id": "orders", "name": "orders", "kind": "service"}
            ],
            "edges": [
                {"from": "api", "to": "orders", "kind": "http"},
                {"from": "api::c1", "to": "orders", "kind": "contract"}
            ]
        });
        let s0 = json!({});
        let d0 = system_structure_digest(&doc, &s0, &BTreeMap::new());
        assert_ne!(
            d0,
            system_structure_digest(
                &doc,
                &json!({"overlay": {"seed_services": ["api"]}}),
                &BTreeMap::new()
            )
        );
        assert_ne!(
            d0,
            system_structure_digest(&doc, &json!({"wave": 1}), &BTreeMap::new())
        );
        assert_ne!(
            d0,
            system_structure_digest(
                &doc,
                &json!({"tool": "path", "path_from": "api", "path_to": "orders"}),
                &BTreeMap::new()
            )
        );
        m::set("system-failed-services", json!(["orders"]));
        let d_failed = system_structure_digest(&doc, &s0, &BTreeMap::new());
        m::take("system-failed-services");
        assert_ne!(d0, d_failed);

        // Pinning parent "api" moves both its frame and unfolded child "api::c1" by delta,
        // and moving "api" to the right of "orders" flips outgoing port/control side geometrically.
        let unpinned = layout_system_nodes_with_pins(&doc, false, &BTreeMap::new());
        let api0 = unpinned.nodes.iter().find(|n| n.id == "api").unwrap();
        let child0 = unpinned.nodes.iter().find(|n| n.id == "api::c1").unwrap();
        let orders0 = unpinned.nodes.iter().find(|n| n.id == "orders").unwrap();
        let rel_child_dx = child0.x - api0.x;
        let rel_child_dy = child0.y - api0.y;

        let mut pins = BTreeMap::new();
        pins.insert("api".to_owned(), (orders0.x + 500.0, api0.y + 80.0));
        let pinned = layout_system_nodes_with_pins(&doc, false, &pins);
        let api1 = pinned.nodes.iter().find(|n| n.id == "api").unwrap();
        let child1 = pinned.nodes.iter().find(|n| n.id == "api::c1").unwrap();
        assert!((child1.x - api1.x - rel_child_dx).abs() < 0.01);
        assert!((child1.y - api1.y - rel_child_dy).abs() < 0.01);
        // After pinning api right of orders, wire from api should exit left edge (fx = api1.x).
        assert!(
            pinned.wires[0]
                .path_d
                .starts_with(&format!("M{:.1},", api1.x)),
            "post-pin wire should leave left side of api when moved right of orders: {}",
            pinned.wires[0].path_d
        );

        // Dense system canvas omits SVG edge/hit/label nodes and uses surface pin key + visual state.
        let mut d_nodes = Vec::with_capacity(185);
        for i in 0..185 {
            d_nodes.push(
                json!({"id": format!("svc-{i}"), "name": format!("svc-{i}"), "kind": "service"}),
            );
        }
        let d_edges = json!([
            {"from": "svc-0", "to": "svc-1", "kind": "http"},
            {"from": "svc-1", "to": "svc-2", "kind": "event"}
        ]);
        let dense_doc = json!({"nodes": d_nodes, "edges": d_edges});
        let dense_state = json!({
            "project_id": "proj-sys",
            "focus": "svc-0",
            "tool": "path",
            "path_from": "svc-0",
            "path_to": "svc-2",
        });
        let change_key = sys_pin_key(CHANGES_SURFACE, &dense_state);
        graph_viewport::save_pin(&change_key, "svc-0", (310.0, 220.0));
        let dense_pins = graph_viewport::load_pins(&change_key);
        let dense_layout = layout_system_nodes_with_pins(&dense_doc, false, &dense_pins);
        let bundle = render_canvas_bundle_with_layout(
            CHANGES_SURFACE,
            &dense_doc,
            &dense_state,
            &dense_layout,
        );
        assert!(bundle.is_dense);
        assert!(!bundle.html.contains("system-map-edge"));
        assert!(!bundle.html.contains("system-map-hit"));
        assert!(!bundle.html.contains("system-map-edge-label"));
        assert_eq!(bundle.node_htmls.len(), dense_layout.nodes.len());

        let mut scene =
            build_dense_scene_for_surface(CHANGES_SURFACE, &dense_doc, &dense_state, &dense_layout)
                .expect("dense scene");
        let n0 = scene.nodes.iter().find(|n| n.label == "svc-0").unwrap();
        let n1 = scene.nodes.iter().find(|n| n.label == "svc-1").unwrap();
        assert!(n0.pinned, "should load pin from CHANGES_SURFACE key");
        assert!(n0.selected);
        assert!(n1.lit);
        assert!(scene.edges[0].on && scene.edges[0].lit);

        // Patch DenseScene when selection changes to svc-2 without tool highlight.
        let vis2 = compute_sys_visual_state(&dense_doc, &json!({"focus": "svc-2"}));
        refresh_dense_scene_for_system(&mut scene, &vis2, &bundle.edge_endpoints);
        let n0_after = scene.nodes.iter().find(|n| n.label == "svc-0").unwrap();
        let n2_after = scene.nodes.iter().find(|n| n.label == "svc-2").unwrap();
        assert!(!n0_after.selected);
        assert!(n2_after.selected);
        assert!(scene.edges[1].on);
        assert!(!scene.edges[0].on);
    }
}
