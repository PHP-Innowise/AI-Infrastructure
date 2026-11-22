use crate::{api, dom, graph_viewport, model as m, omnigraph_canvas};
use serde_json::{json, Value as V};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::{spawn_local, JsFuture};

pub(super) const CAMERA_MIN: f64 = graph_viewport::DEFAULT_MIN_SCALE;
pub(super) const CAMERA_MAX: f64 = graph_viewport::DEFAULT_MAX_SCALE;
pub(super) const ZOOM_STEP: f64 = 1.22;
pub(super) const DRAG_THRESHOLD_PX: f64 = graph_viewport::DEFAULT_DRAG_THRESHOLD_PX;
pub(super) const PULSE_INTERVAL_MS: i32 = 3500;
pub(super) const MAX_PULSE_EVENTS: usize = 80;
pub const CAPSULE_CHAR_LIMIT: usize = 6000;
pub const CANONICAL_KNOWLEDGE_BRANCH: &str = "knowledge/canonical";

#[derive(Clone, Debug, PartialEq)]
pub struct OmnigraphCamera {
    pub scale: f64,
    pub pan_x: f64,
    pub pan_y: f64,
    pub plane_w: f64,
    pub plane_h: f64,
    pub fit_mode: bool,
}

impl Default for OmnigraphCamera {
    fn default() -> Self {
        Self {
            scale: 1.0,
            pan_x: 16.0,
            pan_y: 16.0,
            plane_w: 0.0,
            plane_h: 0.0,
            fit_mode: true,
        }
    }
}

impl OmnigraphCamera {
    fn to_viewport(&self) -> graph_viewport::Camera {
        graph_viewport::Camera::new(self.pan_x, self.pan_y, self.scale)
    }

    fn apply_viewport(&mut self, cam: graph_viewport::Camera) {
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
    let next = graph_viewport::fit(plane, viewport, graph_viewport::FitOptions::for_omnigraph());
    (next.scale, next.tx, next.ty)
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn camera_for_fly_to(
    node_pos: (f64, f64),
    viewport: (f64, f64),
    current_scale: f64,
) -> (f64, f64, f64) {
    let next = graph_viewport::fly_to_point(node_pos, viewport, current_scale, 0.75, CAMERA_MAX);
    (next.scale, next.tx, next.ty)
}

pub fn esc(s: &str) -> String {
    dom::escape_html(s)
}

pub fn format_count(n: usize) -> String {
    let raw = n.to_string();
    let mut out = String::with_capacity(raw.len() + raw.len() / 3);
    for (idx, ch) in raw.chars().rev().enumerate() {
        if idx > 0 && idx % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

pub(super) fn response_error_message(res: &V, fallback: &str) -> String {
    res["error"]["message"]
        .as_str()
        .or_else(|| res["error"].as_str())
        .or_else(|| res["message"].as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

pub(super) fn matching_project_session_id(
    selected: &V,
    sid: &str,
    project_id: &str,
) -> Option<String> {
    let sid = sid.trim();
    let project_id = project_id.trim();
    if sid.is_empty() || project_id.is_empty() {
        return None;
    }
    (selected["project_id"].as_str() == Some(project_id)).then(|| sid.to_owned())
}

pub(super) fn promote_target_branch<'a>(atom: &'a V, active_branch: &'a str) -> &'a str {
    let viewed = active_branch.trim();
    if !viewed.is_empty() {
        viewed
    } else {
        atom_branch(atom, "main")
    }
}

pub(super) fn kind_glyph(kind: &str) -> &'static str {
    match kind {
        "finding" => "✦",
        "decision" => "◆",
        "warning" => "▲",
        "contract" => "⬡",
        "insight" => "◉",
        "artifact" => "▣",
        "question" => "?",
        "session" => "◎",
        "file" => "▤",
        "symbol" => "⟨⟩",
        _ => "◈",
    }
}

pub(super) fn kind_title(kind: &str) -> &'static str {
    match kind {
        "finding" => "Finding",
        "decision" => "Decision",
        "warning" => "Warning",
        "contract" => "Contract",
        "insight" => "Insight",
        "artifact" => "Artifact",
        "question" => "Question",
        "session" => "Session",
        "file" => "File",
        "symbol" => "Symbol",
        _ => "Atom",
    }
}

pub(super) fn atom_id(atom: &V) -> String {
    atom["id"]
        .as_str()
        .or_else(|| atom["atom_id"].as_str())
        .unwrap_or("")
        .to_owned()
}

pub(super) fn atom_branch<'a>(atom: &'a V, fallback: &'a str) -> &'a str {
    atom["source_branch"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| atom["branch"].as_str().filter(|s| !s.is_empty()))
        .unwrap_or(fallback)
}

pub(super) fn atom_session_id(atom: &V) -> &str {
    atom["source_session_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| atom["session_id"].as_str().filter(|s| !s.is_empty()))
        .or_else(|| atom["author_session_id"].as_str().filter(|s| !s.is_empty()))
        .unwrap_or("")
}

pub fn format_timestamp_val(v: &V) -> String {
    if let Some(s) = v.as_str().map(str::trim).filter(|s| !s.is_empty()) {
        return s.to_owned();
    }
    let Some(raw) = v
        .as_i64()
        .or_else(|| v.as_u64().and_then(|u| i64::try_from(u).ok()))
        .or_else(|| v.as_f64().map(|f| f as i64))
    else {
        return String::new();
    };
    if raw <= 0 {
        return String::new();
    }
    let secs = if raw >= 1_000_000_000_000_000_000 {
        raw / 1_000_000_000
    } else if raw >= 1_000_000_000_000_000 {
        raw / 1_000_000
    } else if raw >= 1_000_000_000_000 {
        raw / 1_000
    } else {
        raw
    };
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let hour = rem / 3_600;
    let minute = (rem % 3_600) / 60;
    let second = rem % 60;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    if m <= 2 {
        y += 1;
    }
    format!("{y:04}-{m:02}-{d:02} {hour:02}:{minute:02}:{second:02} UTC")
}

pub(super) fn first_timestamp_val(vals: &[&V]) -> String {
    for val in vals {
        let formatted = format_timestamp_val(val);
        if !formatted.is_empty() {
            return formatted;
        }
    }
    String::new()
}

pub(super) fn session_node_id(session: &V) -> String {
    session["id"]
        .as_str()
        .or_else(|| session["session_id"].as_str())
        .unwrap_or("")
        .to_owned()
}

pub(super) fn file_node_id(file: &V) -> String {
    file["id"]
        .as_str()
        .or_else(|| file["path"].as_str())
        .unwrap_or("")
        .to_owned()
}

pub(super) fn symbol_node_id(symbol: &V) -> String {
    symbol["id"]
        .as_str()
        .or_else(|| symbol["fqn"].as_str())
        .unwrap_or("")
        .to_owned()
}

pub(super) fn symbol_file_path(symbol: &V) -> String {
    let direct = symbol["file_path"]
        .as_str()
        .or_else(|| symbol["path"].as_str())
        .unwrap_or("")
        .trim();
    if !direct.is_empty() {
        return direct.to_owned();
    }
    let fqn = symbol["fqn"]
        .as_str()
        .or_else(|| symbol["id"].as_str())
        .unwrap_or("")
        .trim();
    if let Some((head, _)) = fqn.split_once("::") {
        if head.contains('/') || head.contains('.') {
            return head.to_owned();
        }
    }
    String::new()
}

pub(super) fn symbol_display_label(symbol: &V) -> String {
    if let Some(label) = symbol["label"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return label.to_owned();
    }
    let owner = symbol["owner"].as_str().unwrap_or("").trim();
    let name = symbol["name"].as_str().unwrap_or("").trim();
    if !owner.is_empty() && !name.is_empty() {
        return format!("{owner}::{name}");
    }
    if !name.is_empty() {
        return name.to_owned();
    }
    let fqn = symbol_node_id(symbol);
    let path = symbol_file_path(symbol);
    if !path.is_empty() {
        if let Some(rest) = fqn.strip_prefix(&format!("{path}::")) {
            if !rest.is_empty() {
                return rest.to_owned();
            }
        }
    }
    fqn
}

pub(super) fn short_symbol_from_fqn(fqn: &str) -> String {
    let trimmed = fqn.trim();
    if let Some((head, rest)) = trimmed.split_once("::") {
        if (head.contains('/') || head.contains('.')) && !rest.is_empty() {
            return rest.to_owned();
        }
    }
    trimmed.to_owned()
}

pub(super) fn edge_from(edge: &V) -> String {
    edge["from"]
        .as_str()
        .or_else(|| edge["source"].as_str())
        .or_else(|| edge["src"].as_str())
        .unwrap_or("")
        .to_owned()
}

pub(super) fn edge_to(edge: &V) -> String {
    edge["to"]
        .as_str()
        .or_else(|| edge["target"].as_str())
        .or_else(|| edge["dst"].as_str())
        .unwrap_or("")
        .to_owned()
}

pub(super) fn edge_rel(edge: &V) -> String {
    edge["kind"]
        .as_str()
        .or_else(|| edge["relation"].as_str())
        .or_else(|| edge["rel"].as_str())
        .or_else(|| edge["edge_type"].as_str())
        .unwrap_or("relates_to")
        .to_owned()
}

pub(super) fn branch_names_from_graph(graph: &V) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for item in m::array(&graph["branches"]) {
        let name = item
            .as_str()
            .or_else(|| item["name"].as_str())
            .unwrap_or("")
            .trim();
        if !name.is_empty() && seen.insert(name.to_owned()) {
            out.push(name.to_owned());
        }
    }
    if seen.insert("main".to_owned()) {
        out.insert(0, "main".to_owned());
    }
    out
}

pub(crate) fn current_project_id() -> String {
    let p = dom::value("omnigraph-project");
    if !p.is_empty() {
        return p;
    }
    dom::value("project")
}

pub(super) fn ui_state() -> V {
    let v = m::data("omnigraph-ui");
    if v.is_object() {
        return v;
    }
    let init = json!({
        "stage_tab": "constellation",
        "drawer_tab": "inspector",
        "branch": "main",
        "snapshot": null,
        "kind": "",
        "query": "",
        "show_files": true,
        "show_sessions": true,
        "selected_id": null,
        "selected_commit_id": null,
        "pulse_paused": false,
        "highlighted_ids": [],
    });
    m::set("omnigraph-ui", init.clone());
    init
}

pub(super) fn set_ui_state(patch: impl FnOnce(&mut V)) {
    let mut state = ui_state();
    patch(&mut state);
    m::set("omnigraph-ui", state);
}

pub(super) fn finite_or(raw: &V, fallback: f64) -> f64 {
    raw.as_f64().filter(|v| v.is_finite()).unwrap_or(fallback)
}

pub(super) fn find_omni_node_el(
    canvas: &web_sys::Element,
    node_id: &str,
) -> Option<web_sys::Element> {
    let escaped = crate::dom::escape_css_attr_value(node_id);
    canvas
        .query_selector(&format!(".omni-node[data-node-id=\"{escaped}\"]"))
        .ok()
        .flatten()
}

pub(super) fn clamped_radar_rect(
    cam: graph_viewport::Camera,
    viewport: (f64, f64),
    plane: (f64, f64),
    min_side: f64,
) -> Option<(f64, f64, f64, f64)> {
    let min_w = min_side.min(plane.0);
    let min_h = min_side.min(plane.1);
    let (vx, vy, vw, vh) =
        graph_viewport::radar_viewport_rect(cam, viewport, plane, min_w.min(min_h))?;
    let vw = vw.clamp(min_w, plane.0);
    let vh = vh.clamp(min_h, plane.1);
    let vx = vx.clamp(0.0, (plane.0 - vw).max(0.0));
    let vy = vy.clamp(0.0, (plane.1 - vh).max(0.0));
    Some((vx, vy, vw, vh))
}

pub(super) fn omni_nav_entry_from(
    selected_id: Option<String>,
    branch: &str,
    cam: &OmnigraphCamera,
) -> graph_viewport::CameraSelectionEntry {
    let sel = selected_id.unwrap_or_default();
    let branch_clean = if branch.trim().is_empty() {
        "main"
    } else {
        branch.trim()
    };
    let target = json!({
        "selected_id": if sel.is_empty() { V::Null } else { json!(sel) },
        "branch": branch_clean,
    })
    .to_string();
    let label = if sel.is_empty() {
        "Overview".to_owned()
    } else {
        sel.clone()
    };
    graph_viewport::CameraSelectionEntry {
        key: format!("{branch_clean}::{sel}"),
        label,
        target,
        cam: cam.to_viewport(),
    }
}

pub(super) fn parse_omni_nav_entry(
    entry: &graph_viewport::CameraSelectionEntry,
) -> (Option<String>, String, OmnigraphCamera) {
    let parsed: V = serde_json::from_str(&entry.target).unwrap_or(V::Null);
    let selected_id = parsed["selected_id"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let branch = parsed["branch"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("main")
        .to_owned();
    let cur_cam = load_camera();
    let cam = OmnigraphCamera {
        scale: entry.cam.scale,
        pan_x: entry.cam.tx,
        pan_y: entry.cam.ty,
        plane_w: cur_cam.plane_w,
        plane_h: cur_cam.plane_h,
        fit_mode: false,
    };
    (selected_id, branch, cam)
}

thread_local! {
    pub(super) static OMNI_HOVER_HIT: RefCell<Option<Option<(u8, usize)>>> = const { RefCell::new(None) };
    pub(super) static OMNI_NAV_HISTORY: RefCell<graph_viewport::CameraSelectionHistory> =
        RefCell::new(graph_viewport::CameraSelectionHistory::new(32));
}

pub(super) fn clear_omni_hover_state(cam: &OmnigraphCamera) {
    let had_hover = OMNI_HOVER_HIT.with(|h| h.borrow_mut().take().flatten().is_some());
    if had_hover {
        graph_viewport::canvas2d::paint_scene("omnigraph-canvas", cam.to_viewport(), None, true);
    }
    if let Some(he) =
        dom::get("omnigraph-canvas").and_then(|c| c.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = he.style().set_property("cursor", "");
    }
}

pub(super) fn load_camera() -> OmnigraphCamera {
    let raw = m::data("omnigraph-camera");
    if !raw.is_object() {
        return OmnigraphCamera::default();
    }
    OmnigraphCamera {
        scale: finite_or(&raw["scale"], 1.0).clamp(CAMERA_MIN, CAMERA_MAX),
        pan_x: finite_or(&raw["pan_x"], 16.0),
        pan_y: finite_or(&raw["pan_y"], 16.0),
        plane_w: finite_or(&raw["plane_w"], 0.0).max(0.0),
        plane_h: finite_or(&raw["plane_h"], 0.0).max(0.0),
        fit_mode: raw["fit_mode"].as_bool().unwrap_or(true),
    }
}

pub(super) fn save_camera(cam: &OmnigraphCamera) {
    m::set(
        "omnigraph-camera",
        json!({
            "scale": cam.scale,
            "pan_x": cam.pan_x,
            "pan_y": cam.pan_y,
            "plane_w": cam.plane_w,
            "plane_h": cam.plane_h,
            "fit_mode": cam.fit_mode,
        }),
    );
}

pub(super) fn viewport_size_measured() -> (f64, f64, bool) {
    graph_viewport::viewport_size("omnigraph-canvas", 40.0, (760.0, 420.0))
}

pub(super) fn viewport_size() -> (f64, f64) {
    let (w, h, _) = viewport_size_measured();
    (w, h)
}

pub(super) fn sync_omni_radar(cam: &OmnigraphCamera) {
    let Some(radar) = dom::get("omnigraph-canvas-radar") else {
        return;
    };
    let (vw, vh, measured) = viewport_size_measured();
    let visible = measured
        && cam.plane_w > 0.0
        && cam.plane_h > 0.0
        && graph_viewport::radar_window_rect(
            cam.to_viewport(),
            (vw, vh),
            (cam.plane_w, cam.plane_h),
        )
        .is_some();
    let Some(radar_view) = radar.query_selector(".omni-radar-view").ok().flatten() else {
        dom::hide(&radar, true);
        return;
    };
    if !visible {
        dom::hide(&radar, true);
        return;
    }
    if let Some((vx, vy, vw, vh)) = clamped_radar_rect(
        cam.to_viewport(),
        (vw, vh),
        (cam.plane_w, cam.plane_h),
        16.0,
    ) {
        let _ = radar_view.set_attribute("x", &format!("{vx:.1}"));
        let _ = radar_view.set_attribute("y", &format!("{vy:.1}"));
        let _ = radar_view.set_attribute("width", &format!("{vw:.1}"));
        let _ = radar_view.set_attribute("height", &format!("{vh:.1}"));
        dom::hide(&radar, false);
    } else {
        dom::hide(&radar, true);
    }
}

pub(super) fn apply_camera(cam: &OmnigraphCamera) {
    let has_empty = dom::get("omnigraph-canvas").is_some_and(|c| {
        c.query_selector(".graph-map-plane[data-empty=\"true\"]")
            .ok()
            .flatten()
            .is_some()
    });
    if has_empty {
        dom::attr("omnigraph-canvas", "data-lod", "near");
        dom::text("omni-zoom-level", "100%");
        if let Some(radar) = dom::get("omnigraph-canvas-radar") {
            dom::hide(&radar, true);
        }
        return;
    }
    let plane_size = (cam.plane_w.max(320.0), cam.plane_h.max(240.0));
    graph_viewport::apply_transform_3d_by_id(
        "omnigraph-canvas",
        Some("omni-zoom-level"),
        cam.to_viewport(),
        plane_size,
        graph_viewport::OMNIGRAPH_LOD,
    );
    sync_omni_radar(cam);
    sync_omnigraph_viewport_culling(cam);
    let hover = OMNI_HOVER_HIT.with(|h| h.borrow().flatten());
    graph_viewport::canvas2d::paint_scene("omnigraph-canvas", cam.to_viewport(), hover, true);
}

pub(super) fn fit_camera() {
    let mut cam = load_camera();
    let vw_vh = viewport_size();
    cam.fit_mode = true;
    if cam.plane_w > 0.0 && cam.plane_h > 0.0 {
        let fitted = graph_viewport::fit(
            (cam.plane_w, cam.plane_h),
            vw_vh,
            graph_viewport::FitOptions::for_omnigraph(),
        );
        cam.apply_viewport(fitted);
    }
    save_camera(&cam);
    apply_camera(&cam);
}

pub(super) fn reset_camera_100() {
    let mut cam = load_camera();
    let next = graph_viewport::reset_100((cam.plane_w, cam.plane_h), viewport_size(), 16.0);
    cam.apply_viewport(next);
    cam.fit_mode = false;
    save_camera(&cam);
    apply_camera(&cam);
}

pub(super) fn zoom_canvas_at(factor: f64, client_anchor: Option<(f64, f64)>) {
    let mut cam = load_camera();
    let (vw, vh) = viewport_size();
    let anchor = if let Some((cx, cy)) = client_anchor {
        if let Some(canvas) = dom::get("omnigraph-canvas") {
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
    let zoomed = graph_viewport::zoom_at(
        cam.to_viewport(),
        anchor,
        cam.scale * factor,
        CAMERA_MIN,
        CAMERA_MAX,
    );
    cam.apply_viewport(zoomed);
    cam.fit_mode = false;
    save_camera(&cam);
    apply_camera(&cam);
}

pub(super) fn fly_to_node(node_id: &str) {
    let positions = m::data("omnigraph-positions");
    let Some(arr) = positions[node_id].as_array() else {
        return;
    };
    let (Some(nx), Some(ny)) = (
        arr.first().and_then(|v| v.as_f64()),
        arr.get(1).and_then(|v| v.as_f64()),
    ) else {
        return;
    };
    let mut cam = load_camera();
    let next = graph_viewport::fly_to_point((nx, ny), viewport_size(), cam.scale, 0.75, CAMERA_MAX);
    cam.apply_viewport(next);
    cam.fit_mode = false;
    save_camera(&cam);
    apply_camera(&cam);
}

pub(super) fn refit_if_visible() {
    if m::read(|s| s.view != "omnigraph") || m::text(&ui_state(), "stage_tab") != "constellation" {
        return;
    }
    let (vw, vh, measured) = viewport_size_measured();
    let mut cam = load_camera();
    match graph_viewport::resize_refit_decision(
        cam.to_viewport(),
        cam.fit_mode,
        (cam.plane_w, cam.plane_h),
        (vw, vh),
        measured,
        graph_viewport::FitOptions::for_omnigraph(),
    ) {
        graph_viewport::ResizeAction::Skip => {}
        graph_viewport::ResizeAction::Refit(next) => {
            cam.apply_viewport(next);
            save_camera(&cam);
            apply_camera(&cam);
        }
        graph_viewport::ResizeAction::SyncRadar(_) => {
            apply_camera(&cam);
        }
    }
}

thread_local! {
    pub(super) static OMNI_SSE_BOUND: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(super) fn ensure_resize_watch() {
    graph_viewport::ensure_resize_observer(
        "omnigraph-resize-watched",
        Some("omnigraph-resize-debounce"),
        80,
        refit_if_visible,
    );
}

mod constellation_view;
mod dialogs;
mod inspector;
mod interactions;
mod pulse;
mod session_bar;
mod status;
#[cfg(test)]
mod tests;

#[allow(unused_imports)]
use constellation_view::*;
#[allow(unused_imports)]
use dialogs::*;
#[allow(unused_imports)]
use inspector::*;
#[allow(unused_imports)]
use interactions::*;
#[allow(unused_imports)]
use pulse::*;
#[allow(unused_imports)]
use session_bar::*;
#[allow(unused_imports)]
use status::*;

pub use dialogs::open_share_dialog;
#[allow(unused_imports)]
pub use dialogs::{
    export_download_url, export_preview_summary, export_request_body, import_request_body,
    inspect_import_payload, link_sessions_request_body, open_transfer_dialog, percent_encode,
    render_share_task_list_html, render_share_topology_html, share_summary_text, ImportFormat,
    ParsedImportPreview,
};
#[allow(unused_imports)]
pub use inspector::{
    atom_is_verified, format_context_snippet, render_commit_inspector_html,
    render_node_inspector_html, render_pulse_list_html, render_search_hits_html,
};
#[allow(unused_imports)]
pub use interactions::{
    attach_wheel_listeners, constellation_hotkey_allowed, event, key, pointer_event,
};
#[allow(unused_imports)]
pub use pulse::{
    apply_system_jump_context_to_state, branch_merge_outcome, classify_changes_poll_response,
    collect_search_hit_ids, extract_baseline_graph_patch, find_context_node_for_file, focus_branch,
    graph_pagination_summary, merge_graph_page, merge_search_subgraph_into_graph,
    open_knowledge_atom, render_pagination_bar_html, resolve_code_map_jump_context,
    resolve_code_map_target, resolve_code_map_target_path, resolve_code_map_target_with_symbol_id,
    resolve_system_service_target, resolve_workspace_drift_docs, ChangesPollDecision,
    CodeMapJumpContext, GraphPaginationSummary,
};
#[allow(unused_imports)]
pub(crate) use pulse::{
    focus_context_for_file, open_from_system_context, open_from_system_service,
    open_target_in_code_map, open_target_in_code_map_with_context,
};

pub(crate) async fn refresh_graph_preserving_pages() {
    pulse::refresh_graph_preserving_pages().await;
}
#[cfg(test)]
#[allow(unused_imports)]
pub use session_bar::session_payload_from_fields;
#[allow(unused_imports)]
pub use session_bar::{
    apply_session_settings, clear_draft_peers, followup_payload, followup_payload_from_fields,
    load_session_branches, parse_csv_list, parse_relates_to, render_panel_peer_checklist_html,
    render_session_atoms_html, render_session_bar, render_session_peers_html, session_bar_error,
    session_bar_metrics, session_bar_summary, session_controls, session_payload,
    session_payload_with_links, sync_peer_draft_from_dom, sync_sessions_from_response,
    SessionOmnigraphMetrics,
};
#[allow(unused_imports)]
pub use status::{
    degraded_status_from_error, hydrate_status, initialize, leave, load, local_install_copy,
    on_project_changed, pill_presentation, refresh_pill, studio_visibility_plan, visibility,
};
