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

use crate::{api, dom, model as m, systems};
use serde_json::{json, Value as V};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
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

const CAMERA_MIN: f64 = 0.25;
const CAMERA_MAX: f64 = 2.4;
const ZOOM_STEP: f64 = 1.25;
const DRAG_THRESHOLD_PX: f64 = 4.0;

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

/// Accessible legend of all 7 cross-service contract & code arrow kinds plus drift/risk halos.
pub fn contract_arrow_legend_html() -> String {
    let mut html = String::from(
        "<div class=\"graph-arrow-legend system-arrow-legend\" role=\"list\" aria-label=\"Cross-service contract and dependency arrow types\"><span class=\"graph-arrow-legend-title\">Links:</span>",
    );
    for spec in CONTRACT_ARROW_SPECS {
        let dash = spec
            .dash_array
            .map(|d| format!(" stroke-dasharray=\"{d}\""))
            .unwrap_or_default();
        html.push_str(&format!(
            "<span class=\"graph-arrow-legend-item\" role=\"listitem\" title=\"{}\"><svg class=\"graph-arrow-swatch system-arrow-swatch\" data-sys-edge-kind=\"{}\" viewBox=\"0 0 30 12\" width=\"30\" height=\"12\" aria-hidden=\"true\"><line class=\"graph-arrow-swatch-line\" x1=\"2\" y1=\"6\" x2=\"{:.1}\" y2=\"6\"{dash}/><g transform=\"translate(20 2)\" class=\"system-map-arrow system-map-arrow-{}\">{}</g></svg><span>{}</span></span>",
            escape(spec.description),
            spec.kind,
            spec.swatch_line_end,
            spec.kind,
            spec.marker_body,
            spec.label,
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

pub fn zoom_at(
    scale: f64,
    pan_x: f64,
    pan_y: f64,
    anchor: (f64, f64),
    target_scale: f64,
) -> (f64, f64, f64) {
    let cur = scale.clamp(CAMERA_MIN, CAMERA_MAX);
    let next = target_scale.clamp(CAMERA_MIN, CAMERA_MAX);
    let world_x = (anchor.0 - pan_x) / cur;
    let world_y = (anchor.1 - pan_y) / cur;
    let next_pan_x = (anchor.0 - world_x * next).round();
    let next_pan_y = (anchor.1 - world_y * next).round();
    (next, next_pan_x, next_pan_y)
}

pub fn camera_for_fit(plane: (f64, f64), viewport: (f64, f64)) -> (f64, f64, f64) {
    let pw = plane.0.max(1.0);
    let ph = plane.1.max(1.0);
    let vw = (viewport.0 - 24.0).max(120.0);
    let vh = (viewport.1 - 24.0).max(120.0);
    let scale = (vw / pw).min(vh / ph).clamp(CAMERA_MIN, 1.15);
    let pan_x = ((viewport.0 - pw * scale) / 2.0).max(12.0).round();
    let pan_y = ((viewport.1 - ph * scale) / 2.0).max(12.0).round();
    (scale, pan_x, pan_y)
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
    let Some(el) = dom::get(canvas_id).and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
    else {
        return (760.0, 380.0, false);
    };
    let w = el.client_width() as f64;
    let h = el.client_height() as f64;
    if w > 40.0 && h > 40.0 {
        (w, h, true)
    } else {
        (760.0, 380.0, false)
    }
}

fn apply_camera(surface: SysMapSurface, cam: &Camera) {
    let pct = format!("{}%", (cam.scale * 100.0).round() as i64);
    dom::text(&format!("{}-zoom-level", surface.id_prefix), &pct);
    let Some(canvas) = dom::get(surface.canvas_id) else {
        return;
    };
    let Ok(Some(plane)) = canvas.query_selector(".graph-map-plane") else {
        return;
    };
    let Some(plane_el) = plane.dyn_into::<web_sys::HtmlElement>().ok() else {
        return;
    };
    let style = format!(
        "width:{:.0}px;height:{:.0}px;--graph-scale:{:.4};--graph-pan-x:{:.1}px;--graph-pan-y:{:.1}px;transform:translate3d({:.1}px,{:.1}px,0) scale({:.4});transform-origin:0 0",
        cam.plane_w.max(320.0),
        cam.plane_h.max(220.0),
        cam.scale,
        cam.pan_x,
        cam.pan_y,
        cam.pan_x,
        cam.pan_y,
        cam.scale,
    );
    let _ = plane_el.set_attribute("style", &style);
}

fn fit_surface_camera(surface: SysMapSurface) {
    let mut cam = load_camera(surface);
    let (vw, vh, measured) = viewport_size(surface.canvas_id);
    cam.needs_measure = !measured;
    cam.fit_mode = true;
    if cam.plane_w > 0.0 && cam.plane_h > 0.0 {
        let (s, px, py) = camera_for_fit((cam.plane_w, cam.plane_h), (vw, vh));
        cam.scale = s;
        cam.pan_x = px;
        cam.pan_y = py;
    }
    save_camera(surface, &cam);
    apply_camera(surface, &cam);
}

fn reset_surface_camera_100(surface: SysMapSurface) {
    let mut cam = load_camera(surface);
    let (vw, vh, measured) = viewport_size(surface.canvas_id);
    cam.needs_measure = !measured;
    cam.scale = 1.0;
    cam.pan_x = if cam.plane_w > 0.0 && cam.plane_w < vw {
        ((vw - cam.plane_w) / 2.0).round()
    } else {
        16.0
    };
    cam.pan_y = if cam.plane_h > 0.0 && cam.plane_h < vh {
        ((vh - cam.plane_h) / 2.0).round()
    } else {
        16.0
    };
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
    let (s, px, py) = zoom_at(cam.scale, cam.pan_x, cam.pan_y, anchor, cam.scale * factor);
    cam.scale = s;
    cam.pan_x = px;
    cam.pan_y = py;
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
    let active_cols: Vec<(usize, Vec<usize>)> = if layers_mode {
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

    let total_w = (cur_x - COL_GAP + MARGIN).max(420.0);
    let total_h = (max_y - ROW_GAP + MARGIN).max(240.0);

    let mut wires = Vec::new();
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
        let (sx, sy, ex, ey) = if fcol < tcol {
            (fx + fw, fy + fh / 2.0, tx, ty + th / 2.0)
        } else if fcol > tcol {
            (fx, fy + fh * 0.35, tx + tw, ty + th * 0.35)
        } else {
            (fx + fw, fy + fh / 2.0, tx + tw, ty + th / 2.0)
        };
        let dx = (ex - sx).abs().max(48.0) * 0.45;
        let (c1x, c2x) = if fcol == tcol {
            (sx + 44.0, ex + 44.0)
        } else if fcol < tcol {
            (sx + dx, ex - dx)
        } else {
            (sx - dx, ex + dx)
        };
        let path_d = format!("M{sx:.1},{sy:.1} C{c1x:.1},{sy:.1} {c2x:.1},{ey:.1} {ex:.1},{ey:.1}");
        let mid_x = 0.125 * sx + 0.375 * c1x + 0.375 * c2x + 0.125 * ex;
        let mid_y = 0.5 * (sy + ey);
        wires.push(PlacedSysWire {
            edge_index,
            path_d,
            mid_x,
            mid_y,
            back,
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

fn filtered_doc(doc: &V, state: &V) -> V {
    let certain = m::yes(&state["certain"]);
    let filter = m::text(state, "filter").trim().to_ascii_lowercase();
    if !certain && filter.is_empty() {
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

    let mut out = doc.clone();
    out["nodes"] = json!(nodes);
    out["edges"] = json!(edges);
    out
}

/// Renders the interactive SVG + HTML plane for `#system-map-canvas` or `#system-change-topology-canvas`.
pub fn render_canvas_html(surface: SysMapSurface, doc: &V, state: &V) -> (String, f64, f64) {
    let view_doc = filtered_doc(doc, state);
    let layers_mode = m::yes(&state["layers"]);
    let layout = layout_system_nodes(&view_doc, layers_mode);
    let nodes = m::array(&view_doc["nodes"]);
    let edges = m::array(&view_doc["edges"]);

    if nodes.is_empty() {
        return (String::new(), 0.0, 0.0);
    }

    let mut node_by_id = BTreeMap::new();
    for n in &nodes {
        node_by_id.insert(node_id(n), n);
    }

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
    let blast = compute_blast_radius(&view_doc, &seeds, &changed_contracts, depth);

    let step_states = &overlay["step_states"];
    let unopened_consumers: BTreeSet<String> = m::array(&overlay["cross_service_unopened"])
        .into_iter()
        .filter_map(|v| {
            v.as_str().map(str::to_owned).or_else(|| {
                let s = first_non_empty(&v, &["consumer_service", "service", "consumer"]);
                (!s.is_empty()).then_some(s)
            })
        })
        .collect();

    let (_, layer_violations) = assign_service_layers(&view_doc);
    let violation_set: BTreeSet<(String, String)> = layer_violations.into_iter().collect();

    // Active analytical highlight sets (Path, Shared, Cycle, Follow).
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
        for path in shortest_service_paths(&view_doc, &p_from, &p_to, &skip) {
            for w in path.windows(2) {
                highlight_nodes.insert(w[0].clone());
                highlight_nodes.insert(w[1].clone());
                highlight_edges.insert((w[0].clone(), w[1].clone()));
            }
        }
    } else if tool == "shared" {
        let sa = m::text(state, "shared_a");
        let sb = m::text(state, "shared_b");
        let (both_use, uses_both) = shared_neighbors(&view_doc, &sa, &sb);
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
        let cycles = find_cross_service_cycles(&view_doc);
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

    let mut html = format!(
        "<div class=\"graph-map-plane system-map-plane\" data-surface=\"{}\" style=\"width:{:.0}px;height:{:.0}px\"><svg width=\"{:.0}\" height=\"{:.0}\" aria-hidden=\"true\"><defs>{}</defs>",
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

    // Edges (halos + visible strokes + wide click/contextmenu hit targets)
    for wire in &layout.wires {
        let Some(edge) = edges.get(wire.edge_index) else {
            continue;
        };
        let from = m::text(edge, "from");
        let to = m::text(edge, "to");
        let kind = primary_edge_kind(edge);
        let spec = spec_for_kind(kind);
        let prov = m::text(edge, "provenance");
        let broken = m::yes(&edge["broken_risk"]);
        let against = m::yes(&edge["against"])
            || wire.back
            || (layers_mode && violation_set.contains(&(from.clone(), to.clone())));
        let is_selected = sel_edge_pair.0 == from && sel_edge_pair.1 == to && !from.is_empty();
        let is_highlighted = highlight_edges.contains(&(from.clone(), to.clone()));

        if against {
            html.push_str(&format!(
                "<path class=\"graph-map-violation-halo system-map-violation-halo\" d=\"{}\" data-from=\"{}\" data-to=\"{}\"{}/>",
                wire.path_d,
                escape(&from),
                escape(&to),
                if is_selected || is_highlighted { " data-edge-state=\"on\"" } else { "" }
            ));
        }
        if broken {
            html.push_str(&format!(
                "<path class=\"system-map-broken-halo\" d=\"{}\" data-from=\"{}\" data-to=\"{}\"/>",
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
            "<path class=\"{edge_classes}\" d=\"{}\" data-sys-edge-kind=\"{kind}\" data-provenance=\"{}\" data-broken-risk=\"{}\" data-from=\"{}\" data-to=\"{}\"{dash_attr}{} marker-end=\"url(#{marker_id})\"/>",
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
            html.push_str(&format!(
                "<g class=\"graph-map-edge-tie system-map-edge-label\" data-from=\"{}\" data-to=\"{}\"{}><rect x=\"{:.1}\" y=\"{:.1}\" width=\"54\" height=\"16\" rx=\"8\"/><text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\">{}</text></g>",
                escape(&from),
                escape(&to),
                if is_selected || is_highlighted { " data-edge-state=\"on\"" } else { "" },
                wire.mid_x - 27.0,
                wire.mid_y - 8.0,
                wire.mid_x,
                wire.mid_y + 3.5,
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
            "<path class=\"graph-map-hit system-map-hit\" d=\"{}\" tabindex=\"0\" role=\"button\" aria-label=\"{}\" data-action=\"system-map-edge\" data-payload=\"{hit_payload}\" data-explain-services=\"{}\" data-from=\"{}\" data-to=\"{}\"><title>{}</title></path>",
            wire.path_d,
            escape(&aria),
            escape(&explain_svcs),
            escape(&from),
            escape(&to),
            escape(&aria),
        ));
    }

    html.push_str("</svg>");

    // Interactive service & unfolded child HTML node cards
    for placed in &layout.nodes {
        let Some(&node) = node_by_id.get(&placed.id) else {
            continue;
        };
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
        let is_highlighted = highlight_nodes.contains(&placed.id);

        let blast_role = if blast.seeds.contains(&placed.id) {
            "seed"
        } else if blast.direct_consumers.contains(&placed.id) {
            "direct"
        } else if blast.downstream_consumers.contains(&placed.id) {
            "downstream"
        } else {
            ""
        };

        let step_state = if step_states.is_object() {
            m::text(step_states, &placed.id)
        } else {
            String::new()
        };
        let unopened_warn = unopened_consumers.contains(&placed.id);
        let graph_state = m::text(&node["graph_status"], "state");

        let payload = payload_attr(&json!({
            "surface": surface.id,
            "path": placed.id,
            "id": placed.id,
            "service": svc_id,
            "kind": kind,
        }));

        if placed.child_of.is_some() {
            let badge = match kind {
                "contract" => "C",
                "capability" => "Cap",
                "source" => "Src",
                _ => "•",
            };
            html.push_str(&format!(
                "<button type=\"button\" class=\"graph-map-node system-map-node system-map-child\" data-n=\"{}\" data-kind=\"{}\" data-selected=\"{}\" style=\"left:{:.1}px;top:{:.1}px;width:{:.1}px;height:{:.1}px\" data-action=\"system-map-node\" data-payload=\"{payload}\" data-explain-services=\"{}\" title=\"{}\" aria-label=\"{}\"><span class=\"system-child-badge\">{}</span><span class=\"system-node-title\">{}</span></button>",
                escape(&placed.id),
                escape(kind),
                if is_selected { "true" } else { "false" },
                placed.x,
                placed.y,
                placed.w,
                placed.h,
                escape(&svc_id),
                escape(&name),
                escape(&name),
                escape(badge),
                escape(&name),
            ));
            continue;
        }

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
                    blast.wave_by_service.get(&placed.id).copied().unwrap_or(2)
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

        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-map-node system-map-node\" data-n=\"{}\" data-kind=\"{}\" data-selected=\"{}\" data-blast=\"{}\" data-step-state=\"{}\" data-unopened=\"{}\" data-graph-state=\"{}\"{} style=\"left:{:.1}px;top:{:.1}px;width:{:.1}px;height:{:.1}px\" data-action=\"system-map-node\" data-payload=\"{payload}\" data-explain-services=\"{}\" title=\"{} — double-click to open service code map\" aria-label=\"{}\"><span class=\"system-node-top\"><span class=\"system-node-dot\" data-state=\"{}\" aria-hidden=\"true\"></span><span class=\"system-node-title\">{}</span>{cycle_badge}{unopened_badge}</span><span class=\"system-node-meta\">{}</span></button>",
            escape(&placed.id),
            escape(kind),
            if is_selected { "true" } else { "false" },
            blast_role,
            escape(&step_state),
            if unopened_warn { "true" } else { "false" },
            escape(&graph_state),
            if is_highlighted { " data-shared-path=\"true\"" } else { "" },
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
        ));
    }

    html.push_str("</div>");
    (html, layout.width, layout.height)
}

/// Renders the rich `#system-map-side` or `#system-change-topology-side` inspector HTML.
pub fn render_side_html(surface: SysMapSurface, doc: &V, state: &V) -> String {
    let side_tab = {
        let t = m::text(state, "side_tab");
        if t.is_empty() {
            "details".to_owned()
        } else {
            t
        }
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
        let p = payload_attr(&json!({ "surface": surface.id, "tab": id }));
        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-pick-tab system-side-tab\" role=\"tab\" aria-selected=\"{}\" data-action=\"system-map-side-tab\" data-payload=\"{p}\">{}</button>",
            if side_tab == id { "true" } else { "false" },
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
}

fn render_surface(surface: SysMapSurface, refit: bool) {
    attach_wheel_listeners();
    ensure_hover_watch(surface);
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

    let (canvas_html, pw, ph) = render_canvas_html(surface, &doc, &state);
    set_html(surface.canvas_id, &canvas_html);
    set_html(surface.legend_id, &contract_arrow_legend_html());
    dom::hidden(surface.legend_id, canvas_html.is_empty());

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

    let mut cam = load_camera(surface);
    let size_changed = (cam.plane_w - pw).abs() > 1.0 || (cam.plane_h - ph).abs() > 1.0;
    cam.plane_w = pw;
    cam.plane_h = ph;
    if refit || cam.fit_mode || (size_changed && cam.needs_measure) {
        let (vw, vh, measured) = viewport_size(surface.canvas_id);
        cam.needs_measure = !measured;
        if pw > 0.0 && ph > 0.0 {
            let (s, px, py) = camera_for_fit((pw, ph), (vw, vh));
            cam.scale = s;
            cam.pan_x = px;
            cam.pan_y = py;
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
    let Some(canvas) = dom::get(canvas_id) else {
        return;
    };
    if canvas.get_attribute("data-wheel-bound").as_deref() == Some("true") {
        return;
    }
    let _ = canvas.set_attribute("data-wheel-bound", "true");
    let callback = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        let Ok(wheel) = event.dyn_into::<web_sys::WheelEvent>() else {
            return;
        };
        let Some(surface) = surface_for_canvas(canvas_id) else {
            return;
        };
        wheel.prevent_default();
        wheel.stop_propagation();
        let dy = wheel.delta_y();
        if dy.abs() < f64::EPSILON {
            return;
        }
        let factor = if dy < 0.0 { 1.14 } else { 1.0 / 1.14 };
        dom::attr(canvas_id, "data-wheeling", "true");
        zoom_surface_at(
            surface,
            factor,
            Some((wheel.client_x() as f64, wheel.client_y() as f64)),
        );
        let clear_key = format!("{canvas_id}-wheel-end");
        dom::later(&clear_key, 140, move || {
            dom::attr(canvas_id, "data-wheeling", "false");
        });
    });
    let options = web_sys::AddEventListenerOptions::new();
    options.set_passive(false);
    if canvas
        .add_event_listener_with_callback_and_add_event_listener_options(
            "wheel",
            callback.as_ref().unchecked_ref(),
            &options,
        )
        .is_ok()
    {
        callback.forget();
    }
}

pub fn attach_wheel_listeners() {
    attach_canvas_wheel(SERVICES_SURFACE.canvas_id);
    attach_canvas_wheel(CHANGES_SURFACE.canvas_id);
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
    if hovered_node.is_none() && hovered_edge.is_none() {
        let _ = plane.remove_attribute("data-hover");
        for el in dom::all(&format!("#{canvas_id} [data-lit]")) {
            let _ = el.remove_attribute("data-lit");
        }
        return;
    }

    let _ = plane.set_attribute("data-hover", "true");
    let mut lit_nodes = BTreeSet::new();
    for el in dom::all(&format!("#{canvas_id} .system-map-edge, #{canvas_id} .system-map-violation-halo, #{canvas_id} .system-map-edge-label")) {
        let f = el.get_attribute("data-from").unwrap_or_default();
        let t = el.get_attribute("data-to").unwrap_or_default();
        let matches = if let Some(nid) = hovered_node {
            f == nid || t == nid
        } else if let Some((ef, et)) = hovered_edge {
            f == ef && t == et
        } else {
            false
        };
        if matches {
            let _ = el.set_attribute("data-lit", "true");
            lit_nodes.insert(f);
            lit_nodes.insert(t);
        } else {
            let _ = el.remove_attribute("data-lit");
        }
    }
    if let Some(nid) = hovered_node {
        lit_nodes.insert(nid.to_owned());
    }
    for el in dom::all(&format!("#{canvas_id} [data-n]")) {
        let nid = el.get_attribute("data-n").unwrap_or_default();
        if lit_nodes.contains(&nid) {
            let _ = el.set_attribute("data-lit", "true");
        } else {
            let _ = el.remove_attribute("data-lit");
        }
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
        let drag = m::data(surface.drag_key);
        let drag_active = m::yes(&drag["active"]);

        match kind {
            "pointerdown" if in_canvas => {
                let Ok(pe) = event.clone().dyn_into::<web_sys::PointerEvent>() else {
                    return false;
                };
                if pe.button() != 0 {
                    return false;
                }
                let cam = load_camera(surface);
                m::set(
                    surface.drag_key,
                    json!({
                        "active": true,
                        "pointer_id": pe.pointer_id(),
                        "start_x": pe.client_x() as f64,
                        "start_y": pe.client_y() as f64,
                        "orig_pan_x": cam.pan_x,
                        "orig_pan_y": cam.pan_y,
                        "moved": false,
                        "suppress_click": false,
                    }),
                );
                return true;
            }
            "pointermove" if drag_active => {
                let Ok(pe) = event.clone().dyn_into::<web_sys::PointerEvent>() else {
                    return true;
                };
                if pe.pointer_id() as i64 != drag["pointer_id"].as_i64().unwrap_or(-1) {
                    return true;
                }
                let dx = pe.client_x() as f64 - drag["start_x"].as_f64().unwrap_or(0.0);
                let dy = pe.client_y() as f64 - drag["start_y"].as_f64().unwrap_or(0.0);
                let mut moved = m::yes(&drag["moved"]);
                if !moved {
                    if (dx * dx + dy * dy).sqrt() <= DRAG_THRESHOLD_PX {
                        return true;
                    }
                    moved = true;
                    dom::attr(surface.canvas_id, "data-panning", "true");
                    if let Some(c) = dom::get(surface.canvas_id) {
                        let _ = c.set_pointer_capture(pe.pointer_id());
                    }
                }
                let orig_x = drag["orig_pan_x"].as_f64().unwrap_or(0.0);
                let orig_y = drag["orig_pan_y"].as_f64().unwrap_or(0.0);
                let mut next_drag = drag;
                next_drag["moved"] = json!(moved);
                m::set(surface.drag_key, next_drag);
                let mut cam = load_camera(surface);
                cam.pan_x = (orig_x + dx).round();
                cam.pan_y = (orig_y + dy).round();
                cam.fit_mode = false;
                save_camera(surface, &cam);
                apply_camera(surface, &cam);
                return true;
            }
            "pointerup" | "pointercancel" if drag_active => {
                dom::attr(surface.canvas_id, "data-panning", "false");
                if let Ok(pe) = event.clone().dyn_into::<web_sys::PointerEvent>() {
                    if m::yes(&drag["moved"]) {
                        if let Some(c) = dom::get(surface.canvas_id) {
                            let _ = c.release_pointer_capture(pe.pointer_id());
                        }
                    }
                }
                let suppress = kind == "pointerup" && m::yes(&drag["moved"]);
                m::set(
                    surface.drag_key,
                    json!({
                        "active": false,
                        "moved": false,
                        "suppress_click": suppress,
                    }),
                );
                if suppress {
                    let dk = surface.drag_key;
                    let tk = format!("{dk}-clear");
                    dom::later(&tk, 70, move || {
                        let mut d = m::data(dk);
                        if d.is_object() {
                            d["suppress_click"] = json!(false);
                            m::set(dk, d);
                        }
                    });
                }
                return true;
            }
            "dblclick" if in_canvas => {
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
            _ => {}
        }
    }
    false
}

pub fn key(event: &web_sys::KeyboardEvent) -> bool {
    let view = m::read(|s| s.view.clone());
    if view != "systems" && view != "system-changes" {
        return false;
    }
    let surface = if view == "system-changes" {
        CHANGES_SURFACE
    } else {
        SERVICES_SURFACE
    };
    if event.key() == "Escape"
        && dom::get(surface.find_found_id).is_some_and(|el| !el.has_attribute("hidden"))
    {
        close_finder(surface);
        return true;
    }
    if event.key() == "/"
        && !event.ctrl_key()
        && !event.meta_key()
        && !event.alt_key()
        && event
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
            .is_none_or(|el| !matches!(el.tag_name().as_str(), "INPUT" | "TEXTAREA" | "SELECT"))
        && dom::get(surface.find_input_id)
            .is_some_and(|el| el.closest("[hidden]").ok().flatten().is_none())
    {
        event.prevent_default();
        dom::focus(surface.find_input_id);
        return true;
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
}
