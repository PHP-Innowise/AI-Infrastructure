use super::*;
use crate::graph_map::render::{
    render_dense_node_dom_html, DenseNodeDomSeed, DenseRenderModeFlags,
};

#[derive(Clone, Debug)]
pub(in crate::graph_map) struct Camera {
    pub(in crate::graph_map) scale: f64,
    pub(in crate::graph_map) pan_x: f64,
    pub(in crate::graph_map) pan_y: f64,
    pub(in crate::graph_map) fit_mode: bool,
    pub(in crate::graph_map) follow: bool,
    pub(in crate::graph_map) paused: bool,
    pub(in crate::graph_map) expanded: bool,
    pub(in crate::graph_map) plane_w: f64,
    pub(in crate::graph_map) plane_h: f64,
    pub(in crate::graph_map) needs_measure: bool,
    pub(in crate::graph_map) bbox: Option<(f64, f64, f64, f64)>,
}
impl Camera {
    pub(in crate::graph_map) fn viewport_cam(&self) -> graph_viewport::Camera {
        graph_viewport::Camera::new(self.pan_x, self.pan_y, self.scale)
    }
    pub(in crate::graph_map) fn apply_viewport_cam(&mut self, cam: graph_viewport::Camera) {
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
pub(in crate::graph_map) fn load_surface_camera(surface: MapSurface) -> Camera {
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
pub(in crate::graph_map) fn save_surface_camera(surface: MapSurface, cam: &Camera) {
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
pub(in crate::graph_map) fn current_surface_live_focus(surface: MapSurface) -> Option<LiveFocus> {
    if surface.focus_key.is_empty() {
        return None;
    }
    parse_live_focus(&m::data(surface.focus_key))
}
pub(in crate::graph_map) fn viewport_size(canvas_id: &str) -> (f64, f64, bool) {
    graph_viewport::viewport_size(canvas_id, 23.5, (640.0, 420.0))
}
pub(in crate::graph_map) fn apply_surface_plane_transform(surface: MapSurface, cam: &Camera) {
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
pub(in crate::graph_map) fn apply_surface_camera(surface: MapSurface, cam: &Camera) {
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
pub(in crate::graph_map) fn fit_surface_camera(surface: MapSurface, cam: &mut Camera) {
    let prev_scale = cam.scale;
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
    let mut st = surface_state(surface);
    st["fit"] = json!(true);
    st["zoom"] = V::Null;
    st["scale"] = V::Null;
    save_surface_state(surface, &st);
    save_surface_camera(surface, cam);
    apply_surface_camera(surface, cam);
    maybe_schedule_auto_lod_reload(surface, prev_scale, 1.0);
}
pub(in crate::graph_map) fn zoom_surface_camera_to_bbox(
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
fn maybe_schedule_auto_lod_reload(surface: MapSurface, prev_scale: f64, next_scale: f64) {
    let st = surface_state(surface);
    let view = m::data(surface.view_key);
    if super::state_urls::should_trigger_auto_lod_reload(&st, &view, prev_scale, next_scale) {
        #[cfg(target_arch = "wasm32")]
        {
            let timer_key = format!("{}-auto-lod", surface.id_prefix);
            dom::later(&timer_key, 180, move || {
                let surface_active = if surface.embedded {
                    dom::get(surface.canvas).is_some() && !surface_scope(surface).is_empty()
                } else {
                    open()
                };
                if !surface_active || active_drag_surface().is_some() {
                    return;
                }
                let cur_cam = load_surface_camera(surface);
                let cur_st = surface_state(surface);
                let cur_view = m::data(surface.view_key);
                let effective_scale = if cur_cam.fit_mode || m::yes(&cur_st["fit"]) {
                    1.0
                } else {
                    cur_cam.scale
                };
                if !super::state_urls::should_trigger_auto_lod_for_target(
                    &cur_st,
                    &cur_view,
                    effective_scale,
                ) {
                    return;
                }
                spawn_local(async move {
                    if surface.embedded {
                        super::embedded::load_embedded_surface(surface, true).await;
                    } else {
                        modal_lifecycle::load(true).await;
                    }
                });
            });
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = surface;
    }
}

pub(in crate::graph_map) fn zoom_surface_camera_at(
    surface: MapSurface,
    factor: f64,
    anchor: Option<(f64, f64)>,
) {
    let mut cam = load_surface_camera(surface);
    let prev_scale = cam.scale;
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
    let next_scale = cam.scale;
    let mut st = surface_state(surface);
    st["zoom"] = json!(next_scale);
    st["scale"] = json!(next_scale);
    st["fit"] = json!(false);
    save_surface_state(surface, &st);
    save_surface_camera(surface, &cam);
    apply_surface_camera(surface, &cam);
    maybe_schedule_auto_lod_reload(surface, prev_scale, next_scale);
}
pub(in crate::graph_map) fn reset_surface_camera_100(surface: MapSurface) {
    let mut cam = load_surface_camera(surface);
    let prev_scale = cam.scale;
    let (vw, vh, measured) = viewport_size(surface.canvas);
    cam.needs_measure = !measured;
    cam.apply_viewport_cam(graph_viewport::reset_100(
        (cam.plane_w, cam.plane_h),
        (vw, vh),
        12.0,
    ));
    cam.fit_mode = false;
    let next_scale = cam.scale;
    let mut st = surface_state(surface);
    st["zoom"] = V::Null;
    st["scale"] = V::Null;
    st["fit"] = json!(false);
    save_surface_state(surface, &st);
    save_surface_camera(surface, &cam);
    apply_surface_camera(surface, &cam);
    maybe_schedule_auto_lod_reload(surface, prev_scale, next_scale);
}
pub(in crate::graph_map) fn refit_surface_if_visible(surface: MapSurface) {
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
pub(in crate::graph_map) fn viewport_box_changed(
    before: (f64, f64, bool),
    after: (f64, f64, bool),
) -> bool {
    if !after.2 {
        return false;
    }
    !before.2 || (after.0 - before.0).abs() > 0.5 || (after.1 - before.1).abs() > 0.5
}
/// Re-fits or repaints a surface when chrome elements rendered after the canvas
/// (crumbs, arrow legend, side panel, footer legend) changed its viewport height.
pub(in crate::graph_map) fn sync_surface_after_chrome_layout(
    surface: MapSurface,
    before: (f64, f64, bool),
) {
    let after = viewport_size(surface.canvas);
    if !viewport_box_changed(before, after) {
        return;
    }
    let cam = load_surface_camera(surface);
    if cam.plane_w <= 0.0 || cam.plane_h <= 0.0 {
        return;
    }
    if (cam.fit_mode || cam.needs_measure) && !surface.embedded && m::yes(&state()["fit"]) {
        fit();
        let settled = load_surface_camera(surface);
        update_surface_active_camera(surface, &settled);
        return;
    }
    if cam.fit_mode || cam.needs_measure {
        refit_surface_if_visible(surface);
        let settled = load_surface_camera(surface);
        update_surface_active_camera(surface, &settled);
    } else {
        apply_surface_camera(surface, &cam);
        update_surface_active_camera(surface, &cam);
    }
}
pub(in crate::graph_map) fn surface_drag_data(surface: MapSurface) -> V {
    if surface.drag_key.is_empty() {
        V::Null
    } else {
        m::data(surface.drag_key)
    }
}
pub(in crate::graph_map) fn clear_surface_drag_suppress(surface: MapSurface) {
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
pub(in crate::graph_map) fn render_surface_focus_strip(
    surface: MapSurface,
    focus: Option<&LiveFocus>,
) {
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
pub(in crate::graph_map) struct SurfaceViewSignature {
    pub(in crate::graph_map) nodes_len: usize,
    pub(in crate::graph_map) frames_len: usize,
    pub(in crate::graph_map) edges_len: usize,
    pub(in crate::graph_map) first_node_path: String,
    pub(in crate::graph_map) last_node_path: String,
    pub(in crate::graph_map) mode: String,
    pub(in crate::graph_map) pattern_declared: bool,
    pub(in crate::graph_map) layered: bool,
}

#[derive(Clone, Debug)]
pub(in crate::graph_map) struct CachedSurfacePlan {
    pub(in crate::graph_map) signature: SurfaceViewSignature,
    pub(in crate::graph_map) nodes: Vec<Placed>,
    pub(in crate::graph_map) frames: Vec<Framed>,
    pub(in crate::graph_map) width: f64,
    pub(in crate::graph_map) height: f64,
    pub(in crate::graph_map) structure_hash: Option<u64>,
    pub(in crate::graph_map) pairs: Vec<(usize, usize)>,
    pub(in crate::graph_map) emitted_frames: Vec<usize>,
    pub(in crate::graph_map) node_keys: Vec<String>,
    pub(in crate::graph_map) node_labels: Vec<String>,
    pub(in crate::graph_map) is_dense: bool,
    pub(in crate::graph_map) dense_node_htmls: Vec<String>,
    pub(in crate::graph_map) dense_node_seeds: Vec<Option<DenseNodeDomSeed>>,
    pub(in crate::graph_map) dense_render_modes: DenseRenderModeFlags,
    pub(in crate::graph_map) dense_extra_specs: Vec<Option<DenseNodeExtraSpec>>,
    pub(in crate::graph_map) latest_patch: Option<CanvasSelectionPatch>,
    pub(in crate::graph_map) latest_roving_focus: Option<usize>,
    pub(in crate::graph_map) latest_path_nodes: BTreeSet<usize>,
}

thread_local! {
    pub(in crate::graph_map) static SURFACE_PLAN_CACHE: RefCell<HashMap<&'static str, CachedSurfacePlan>> =
        RefCell::new(HashMap::new());
}

pub(in crate::graph_map) fn pairs_for_view(
    view: &V,
    overlay: &CanvasOverlayContext<'_>,
) -> Vec<(usize, usize)> {
    extract_view_graph_parts(view, overlay.changes, overlay.hide_transitive).pairs
}

pub(in crate::graph_map) fn surface_view_signature(view: &V) -> SurfaceViewSignature {
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
    pub(in crate::graph_map) fn from_rendered(
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
            dense_node_seeds: rendered.dense_node_seeds.clone(),
            dense_render_modes: rendered.dense_render_modes,
            dense_extra_specs: rendered.dense_extra_specs.clone(),
            latest_patch: None,
            latest_roving_focus: overlay.and_then(|ctx| ctx.roving_focus),
            latest_path_nodes: overlay
                .and_then(|ctx| ctx.path_nodes)
                .cloned()
                .unwrap_or_default(),
        }
    }

    pub(in crate::graph_map) fn materialize_node_html(
        &self,
        view: Option<&V>,
        idx: usize,
    ) -> Option<String> {
        if let Some(existing) = self.dense_node_htmls.get(idx).filter(|s| !s.is_empty()) {
            return Some(existing.clone());
        }
        let view = view?;
        let seed = self.dense_node_seeds.get(idx)?.as_ref()?;
        let nodes = m::array(&view["nodes"]);
        let node = nodes.get(idx)?;
        Some(render_dense_node_dom_html(
            view,
            node,
            seed,
            self.dense_render_modes,
            self.latest_roving_focus == Some(idx),
        ))
    }
}

pub(in crate::graph_map) fn invalidate_surface_plan_cache(canvas_id: &str) {
    SURFACE_PLAN_CACHE.with(|cache| {
        cache.borrow_mut().remove(canvas_id);
    });
}

pub(in crate::graph_map) fn store_surface_plan_cache(
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

pub(in crate::graph_map) struct PatchedCanvasMeta {
    pub(in crate::graph_map) width: f64,
    pub(in crate::graph_map) height: f64,
    pub(in crate::graph_map) focus: FocusOverlay,
    pub(in crate::graph_map) selected_bbox: Option<(f64, f64, f64, f64)>,
}

pub(in crate::graph_map) fn toggle_bool_attr(el: &web_sys::Element, name: &str, active: bool) {
    let _ = if active {
        el.set_attribute(name, "true")
    } else {
        el.remove_attribute(name)
    };
}

pub(in crate::graph_map) fn sync_node_blast_dom(el: &web_sys::Element, blast: Option<(u64, f64)>) {
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

pub(in crate::graph_map) fn patch_edge_state_dom(
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

pub(in crate::graph_map) fn apply_node_patch_to_el(
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

pub(in crate::graph_map) fn mount_dense_node_if_needed(
    canvas_id: &str,
    idx: usize,
) -> Option<web_sys::Element> {
    let canvas = dom::get(canvas_id)?;
    if let Ok(Some(existing)) = canvas.query_selector(&format!(".graph-map-node[data-n='{idx}']")) {
        return Some(existing);
    }
    let plane = canvas.query_selector(".graph-map-plane").ok().flatten()?;
    let surface_view = pan_surface_for_canvas(canvas_id).map(|s| m::data(s.view_key));
    let (html, extra_spec, patch, roving, path_nodes) = SURFACE_PLAN_CACHE.with(|cache| {
        let guard = cache.borrow();
        let entry = guard.get(canvas_id)?;
        if !entry.is_dense {
            return None;
        }
        let html = entry.materialize_node_html(surface_view.as_ref(), idx)?;
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
pub(in crate::graph_map) fn try_patch_canvas_dom_in_place(
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
        && (cached.nodes.len() != patch.nodes.len()
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
            if let Some(html) = cached.materialize_node_html(Some(view), idx) {
                batch_html.push_str(&html);
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
pub(in crate::graph_map) fn sync_viewport_culling(canvas_id: &str, bounds: Option<ViewportBounds>) {
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

    if cached.is_dense && cached.dense_node_seeds.len() == cached.nodes.len() {
        if let Some(plane) =
            dom::get(canvas_id).and_then(|c| c.query_selector(".graph-map-plane").ok().flatten())
        {
            let rects: Vec<(f64, f64, f64, f64)> = cached
                .nodes
                .iter()
                .map(|p| (p.x, p.y, BOX_WIDTH, BOX_HEIGHT))
                .collect();
            let synth_cam = graph_viewport::Camera {
                scale: 1.0,
                tx: -bounds.min_x,
                ty: -bounds.min_y,
            };
            let viewport = (
                (bounds.max_x - bounds.min_x).max(1.0),
                (bounds.max_y - bounds.min_y).max(1.0),
            );
            let view_opt = pan_surface_for_canvas(canvas_id).map(|s| m::data(s.view_key));
            let mut extra_html = String::new();
            let mut pre_extras: HashMap<usize, Vec<web_sys::Element>> = HashMap::new();
            if let Ok(extras) = plane.query_selector_all("[data-of]") {
                for i in 0..extras.length() {
                    if let Some(el) = extras
                        .item(i)
                        .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
                    {
                        if let Some(idx) = el
                            .get_attribute("data-of")
                            .and_then(|s| s.parse::<usize>().ok())
                        {
                            pre_extras.entry(idx).or_default().push(el);
                        }
                    }
                }
            }
            let extras_by_idx = std::cell::RefCell::new(pre_extras);
            let outcome = graph_viewport::dense_dom::reconcile_dense_nodes(
                &plane,
                synth_cam,
                viewport,
                &rects,
                0.0,
                120.0,
                280.0,
                &mut mounted_els,
                |idx, el_opt| {
                    cached.dense_node_seeds.get(idx).is_none_or(|s| s.is_none())
                        || el_opt.map_or_else(
                            || is_important_cached_node(&cached, idx),
                            |el| is_important_node_el(&cached, idx, el),
                        )
                },
                |idx| cached.materialize_node_html(view_opt.as_ref(), idx),
                |idx, el| {
                    if let Some(np) = cached.latest_patch.as_ref().and_then(|p| p.nodes.get(idx)) {
                        apply_node_patch_to_el(
                            el,
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
                        if !extras_by_idx.borrow().contains_key(&idx) {
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
                },
                |idx| {
                    if let Some(removed) = extras_by_idx.borrow_mut().remove(&idx) {
                        for extra_el in removed {
                            extra_el.remove();
                        }
                    }
                },
            );
            if !extra_html.is_empty() {
                let _ = graph_viewport::append_html_elements(&plane, &extra_html);
            }
            if outcome.dom_mutated {
                invalidate_hover_cache(canvas_id);
            }
            return;
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

pub(in crate::graph_map) fn resolve_focus_bbox_with_cached_plan(
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

#[cfg(test)]
mod tests {
    use super::viewport_box_changed;

    #[test]
    fn viewport_box_changed_detects_post_chrome_resize() {
        assert!(!viewport_box_changed(
            (908.0, 490.0, true),
            (908.0, 490.0, true)
        ));
        assert!(!viewport_box_changed(
            (908.0, 490.0, true),
            (908.4, 490.2, true)
        ));
        assert!(viewport_box_changed(
            (908.0, 490.0, true),
            (908.0, 468.0, true)
        ));
        assert!(viewport_box_changed(
            (800.0, 500.0, false),
            (908.0, 468.0, true)
        ));
        assert!(!viewport_box_changed(
            (908.0, 490.0, true),
            (800.0, 500.0, false)
        ));
    }
}
