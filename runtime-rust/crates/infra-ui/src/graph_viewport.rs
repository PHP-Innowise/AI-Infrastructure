//! Shared 2D graph viewport camera, LOD, pointer-capture drag-pan, radar minimap,
//! wheel zoom, and resize-refit helpers for Harness graph canvases.
//!
//! Pure geometry and state-transition functions live separately from `web_sys`
//! DOM bindings so native `cargo test` runs deterministic unit tests without a
//! browser DOM while `wasm32-unknown-unknown` shares the exact same math.

use crate::dom;
use serde_json::{json, Value};
use std::rc::Rc;
use wasm_bindgen::{prelude::*, JsCast};
use web_sys::Element;

pub const DEFAULT_MIN_SCALE: f64 = 0.25;
pub const DEFAULT_MAX_SCALE: f64 = 2.6;
pub const DEFAULT_DRAG_THRESHOLD_PX: f64 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub tx: f64,
    pub ty: f64,
    pub scale: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            tx: 0.0,
            ty: 0.0,
            scale: 1.0,
        }
    }
}

impl Camera {
    pub const fn new(tx: f64, ty: f64, scale: f64) -> Self {
        Self { tx, ty, scale }
    }

    pub fn clamped(self, min_scale: f64, max_scale: f64) -> Self {
        let (lo, hi) = ordered_bounds(min_scale, max_scale);
        let scale = if self.scale.is_finite() && self.scale > 0.0 {
            self.scale.clamp(lo, hi)
        } else {
            1.0_f64.clamp(lo, hi)
        };
        let tx = if self.tx.is_finite() { self.tx } else { 0.0 };
        let ty = if self.ty.is_finite() { self.ty } else { 0.0 };
        Self { tx, ty, scale }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FitOptions {
    pub min_scale: f64,
    pub max_scale: f64,
    pub margin_x: f64,
    pub margin_y: f64,
    pub min_inner_w: f64,
    pub min_inner_h: f64,
}

impl Default for FitOptions {
    fn default() -> Self {
        Self {
            min_scale: DEFAULT_MIN_SCALE,
            max_scale: 1.15,
            margin_x: 24.0,
            margin_y: 24.0,
            min_inner_w: 120.0,
            min_inner_h: 120.0,
        }
    }
}

impl FitOptions {
    pub const fn for_omnigraph() -> Self {
        Self {
            min_scale: DEFAULT_MIN_SCALE,
            max_scale: 1.15,
            margin_x: 36.0,
            margin_y: 24.0,
            min_inner_w: 160.0,
            min_inner_h: 140.0,
        }
    }

    #[allow(dead_code)]
    pub const fn for_system_map() -> Self {
        Self {
            min_scale: DEFAULT_MIN_SCALE,
            max_scale: 1.15,
            margin_x: 12.0,
            margin_y: 12.0,
            min_inner_w: 120.0,
            min_inner_h: 120.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LodThresholds {
    pub far_below: f64,
    pub near_at: f64,
    pub near_inclusive: bool,
    pub invalid_fallback: &'static str,
}

#[allow(dead_code)]
pub const DEFAULT_LOD: LodThresholds = LodThresholds {
    far_below: 0.5,
    near_at: 1.4,
    near_inclusive: false,
    invalid_fallback: "mid",
};

pub const OMNIGRAPH_LOD: LodThresholds = LodThresholds {
    far_below: 0.55,
    near_at: 0.95,
    near_inclusive: true,
    invalid_fallback: "near",
};

#[allow(dead_code)]
pub fn lod(scale: f64) -> &'static str {
    lod_with(scale, DEFAULT_LOD)
}

pub fn lod_with(scale: f64, thresholds: LodThresholds) -> &'static str {
    if !scale.is_finite() || scale <= 0.0 {
        thresholds.invalid_fallback
    } else if scale < thresholds.far_below {
        "far"
    } else if (thresholds.near_inclusive && scale >= thresholds.near_at)
        || (!thresholds.near_inclusive && scale > thresholds.near_at)
    {
        "near"
    } else {
        "mid"
    }
}

fn ordered_bounds(min_scale: f64, max_scale: f64) -> (f64, f64) {
    let lo = if min_scale.is_finite() && min_scale > 0.0 {
        min_scale
    } else {
        DEFAULT_MIN_SCALE
    };
    let hi = if max_scale.is_finite() && max_scale >= lo {
        max_scale
    } else {
        lo.max(DEFAULT_MAX_SCALE)
    };
    (lo, hi)
}

/// Zooms around a viewport-relative `anchor` `(ax, ay)` while keeping the
/// underlying world point under `anchor` stationary.
pub fn zoom_at(
    cam: Camera,
    anchor: (f64, f64),
    target_scale: f64,
    min_scale: f64,
    max_scale: f64,
) -> Camera {
    let (lo, hi) = ordered_bounds(min_scale, max_scale);
    let cur = cam.clamped(lo, hi);
    let next = if target_scale.is_finite() && target_scale > 0.0 {
        target_scale.clamp(lo, hi)
    } else {
        cur.scale
    };
    let world_x = (anchor.0 - cur.tx) / cur.scale;
    let world_y = (anchor.1 - cur.ty) / cur.scale;
    Camera {
        tx: (anchor.0 - world_x * next).round(),
        ty: (anchor.1 - world_y * next).round(),
        scale: next,
    }
}

/// Fits a `plane` `(width, height)` inside `viewport` `(width, height)` using
/// explicit margins and scale bounds.
pub fn fit(plane: (f64, f64), viewport: (f64, f64), options: FitOptions) -> Camera {
    let (lo, hi) = ordered_bounds(options.min_scale, options.max_scale);
    if !plane.0.is_finite()
        || !plane.1.is_finite()
        || !viewport.0.is_finite()
        || !viewport.1.is_finite()
        || plane.0 <= 0.0
        || plane.1 <= 0.0
        || viewport.0 <= 0.0
        || viewport.1 <= 0.0
    {
        return Camera::default();
    }
    let pw = plane.0.max(1.0);
    let ph = plane.1.max(1.0);
    let vw = (viewport.0 - options.margin_x * 2.0).max(options.min_inner_w);
    let vh = (viewport.1 - options.margin_y * 2.0).max(options.min_inner_h);
    let scale = (vw / pw).min(vh / ph).clamp(lo, hi);
    let tx = ((viewport.0 - pw * scale) / 2.0)
        .max(options.margin_x)
        .round();
    let ty = ((viewport.1 - ph * scale) / 2.0)
        .max(options.margin_y)
        .round();
    Camera { tx, ty, scale }
}

/// Fits a `plane` inside `viewport` using a proportional fill ratio (e.g. `0.94`)
/// and `overflow_pad` when the scaled plane exceeds the viewport.
#[allow(dead_code)]
pub fn fit_proportional(
    plane: (f64, f64),
    viewport: (f64, f64),
    fill_ratio: f64,
    min_scale: f64,
    max_scale: f64,
    overflow_pad: f64,
) -> Camera {
    let (lo, hi) = ordered_bounds(min_scale, max_scale);
    if plane.0 <= 0.0 || plane.1 <= 0.0 || viewport.0 <= 0.0 || viewport.1 <= 0.0 {
        return Camera::default();
    }
    let ratio = if fill_ratio.is_finite() && fill_ratio > 0.0 {
        fill_ratio
    } else {
        0.94
    };
    let scale = ((viewport.0 * ratio) / plane.0)
        .min((viewport.1 * ratio) / plane.1)
        .clamp(lo, hi);
    let scaled_w = plane.0 * scale;
    let scaled_h = plane.1 * scale;
    let tx = if scaled_w <= viewport.0 {
        ((viewport.0 - scaled_w) / 2.0).round()
    } else {
        overflow_pad
    };
    let ty = if scaled_h <= viewport.1 {
        ((viewport.1 - scaled_h) / 2.0).round()
    } else {
        overflow_pad
    };
    Camera { tx, ty, scale }
}

/// Centers a single world `point` inside `viewport`, clamping `scale` to `[min_scale, max_scale]`.
pub fn fly_to_point(
    point: (f64, f64),
    viewport: (f64, f64),
    scale: f64,
    min_scale: f64,
    max_scale: f64,
) -> Camera {
    let (lo, hi) = ordered_bounds(min_scale, max_scale);
    let clamped_scale = if scale.is_finite() && scale > 0.0 {
        scale.clamp(lo, hi)
    } else {
        lo
    };
    center_camera_on(point, viewport, clamped_scale)
}

/// Fits and centers a bounding box `(x0, y0, x1, y1)` inside `viewport`.
#[allow(dead_code)]
pub fn fit_bbox(
    bbox: (f64, f64, f64, f64),
    viewport: (f64, f64),
    pad: (f64, f64),
    min_size: (f64, f64),
    fill_ratio: f64,
    min_scale: f64,
    max_scale: f64,
) -> Camera {
    if viewport.0 <= 0.0 || viewport.1 <= 0.0 {
        return Camera::default();
    }
    let (lo, hi) = ordered_bounds(min_scale, max_scale);
    let bw = ((bbox.2 - bbox.0).abs() + pad.0).max(min_size.0);
    let bh = ((bbox.3 - bbox.1).abs() + pad.1).max(min_size.1);
    let ratio = if fill_ratio.is_finite() && fill_ratio > 0.0 {
        fill_ratio
    } else {
        0.90
    };
    let scale = ((viewport.0 * ratio) / bw)
        .min((viewport.1 * ratio) / bh)
        .clamp(lo, hi);
    let cx = (bbox.0 + bbox.2) / 2.0;
    let cy = (bbox.1 + bbox.3) / 2.0;
    center_camera_on((cx, cy), viewport, scale)
}

/// Resets camera to `1.0` scale, centering `plane` when smaller than `viewport`.
pub fn reset_100(plane: (f64, f64), viewport: (f64, f64), overflow_pad: f64) -> Camera {
    let tx = if plane.0 > 0.0 && plane.0 < viewport.0 {
        ((viewport.0 - plane.0) / 2.0).round()
    } else {
        overflow_pad
    };
    let ty = if plane.1 > 0.0 && plane.1 < viewport.1 {
        ((viewport.1 - plane.1) / 2.0).round()
    } else {
        overflow_pad
    };
    Camera { tx, ty, scale: 1.0 }
}

/// Centers `world_point` inside `viewport` at `scale` without altering `scale`.
pub fn center_camera_on(world_point: (f64, f64), viewport: (f64, f64), scale: f64) -> Camera {
    let safe_scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    Camera {
        tx: (viewport.0 / 2.0 - world_point.0 * safe_scale).round(),
        ty: (viewport.1 / 2.0 - world_point.1 * safe_scale).round(),
        scale: safe_scale,
    }
}

/// Computes the multiplicative zoom factor for a single DOM `wheel` event.
/// Returns `None` for zero or non-finite vertical deltas so pure horizontal
/// trackpad swipes pass through unintercepted.
pub fn wheel_zoom_factor(delta_y: f64, delta_mode: u32) -> Option<f64> {
    if !delta_y.is_finite() || delta_y == 0.0 {
        return None;
    }
    let normalized = match delta_mode {
        1 => delta_y * 24.0,
        2 => delta_y * 160.0,
        _ => delta_y,
    };
    let clamped = normalized.clamp(-240.0, 240.0);
    Some((-clamped * 0.0022).exp())
}

/// Formats the inline `style` attribute for a GPU-accelerated `translate3d` plane.
pub fn plane_transform_style(cam: Camera, plane_size: (f64, f64)) -> String {
    let w = plane_size.0.max(1.0);
    let h = plane_size.1.max(1.0);
    format!(
        "width:{w:.0}px;height:{h:.0}px;--graph-scale:{:.4};--graph-pan-x:{:.1}px;--graph-pan-y:{:.1}px;transform:translate3d({:.1}px,{:.1}px,0) scale({:.4});transform-origin:0 0",
        cam.scale, cam.tx, cam.ty, cam.tx, cam.ty, cam.scale,
    )
}

/// Applies `translate3d` + `scale` transform and `data-lod` (`far`/`mid`/`near`)
/// onto both `canvas` and `plane`. Returns the applied LOD label.
pub fn apply_transform_3d(
    canvas: &Element,
    plane: &Element,
    cam: Camera,
    plane_size: (f64, f64),
    thresholds: LodThresholds,
) -> &'static str {
    let level = lod_with(cam.scale, thresholds);
    let _ = canvas.set_attribute("data-lod", level);
    let _ = plane.set_attribute("data-lod", level);
    let _ = plane.set_attribute("style", &plane_transform_style(cam, plane_size));
    level
}

/// Convenience helper locating `#<canvas_id>` + `.graph-map-plane` (and optional
/// zoom readout `#<zoom_label_id>`) and applying `apply_transform_3d`.
pub fn apply_transform_3d_by_id(
    canvas_id: &str,
    zoom_label_id: Option<&str>,
    cam: Camera,
    plane_size: (f64, f64),
    thresholds: LodThresholds,
) -> Option<&'static str> {
    if let Some(label_id) = zoom_label_id {
        let pct = format!("{}%", (cam.scale * 100.0).round() as i64);
        dom::text(label_id, &pct);
    }
    let canvas = dom::get(canvas_id)?;
    let level = lod_with(cam.scale, thresholds);
    let _ = canvas.set_attribute("data-lod", level);
    let plane = canvas.query_selector(".graph-map-plane").ok().flatten()?;
    Some(apply_transform_3d(
        &canvas, &plane, cam, plane_size, thresholds,
    ))
}

/// Measures `#<canvas_id>` `clientWidth`/`clientHeight`, returning `(w, h, measured)`.
pub fn viewport_size(canvas_id: &str, min_dim: f64, fallback: (f64, f64)) -> (f64, f64, bool) {
    let Some(el) = dom::get(canvas_id) else {
        return (fallback.0, fallback.1, false);
    };
    let w = el.client_width() as f64;
    let h = el.client_height() as f64;
    if w > min_dim && h > min_dim {
        (w, h, true)
    } else {
        (fallback.0, fallback.1, false)
    }
}

/// Computes the radar minimap viewport rectangle `(vx, vy, vw, vh)` in plane
/// coordinates, always clamped inside `plane` with at least `min_side`.
pub fn radar_viewport_rect(
    cam: Camera,
    viewport: (f64, f64),
    plane: (f64, f64),
    min_side: f64,
) -> Option<(f64, f64, f64, f64)> {
    if !cam.scale.is_finite()
        || cam.scale <= 0.0
        || !plane.0.is_finite()
        || !plane.1.is_finite()
        || plane.0 <= 0.0
        || plane.1 <= 0.0
    {
        return None;
    }
    let vx = (-cam.tx / cam.scale).clamp(0.0, plane.0);
    let vy = (-cam.ty / cam.scale).clamp(0.0, plane.1);
    let vw = (viewport.0 / cam.scale).clamp(min_side, (plane.0 - vx).max(min_side));
    let vh = (viewport.1 / cam.scale).clamp(min_side, (plane.1 - vy).max(min_side));
    Some((vx, vy, vw, vh))
}

/// Computes the radar viewport rectangle `(vx, vy, vw, vh)` only when the
/// viewport clips part of `plane` (returning `None` once the whole plane fits).
#[allow(dead_code)]
pub fn radar_window_rect(
    cam: Camera,
    viewport: (f64, f64),
    plane: (f64, f64),
) -> Option<(f64, f64, f64, f64)> {
    if !(cam.scale.is_finite() && cam.scale > 0.0 && plane.0 > 0.0 && plane.1 > 0.0) {
        return None;
    }
    let side = |corner: f64, room: f64, total: f64| {
        let from = -corner / cam.scale;
        let to = from + room / cam.scale;
        let all = from <= 0.5 / cam.scale && to >= total - 0.5 / cam.scale;
        let (from, to) = (from.clamp(0.0, total), to.clamp(0.0, total));
        (from, (to - from).max(0.0), all)
    };
    let (x, width, wide) = side(cam.tx, viewport.0, plane.0);
    let (y, height, tall) = side(cam.ty, viewport.1, plane.1);
    (!(wide && tall)).then_some((x, y, width, height))
}

/// Maps client pointer coordinates `(cx, cy)` inside a radar element's bounding
/// rectangle `(left, top, width, height)` onto world coordinates on `plane`.
pub fn radar_pointer_map(
    client: (f64, f64),
    radar_rect: (f64, f64, f64, f64),
    plane: (f64, f64),
) -> Option<(f64, f64)> {
    if !client.0.is_finite()
        || !client.1.is_finite()
        || !radar_rect.2.is_finite()
        || !radar_rect.3.is_finite()
        || radar_rect.2 <= 1.0
        || radar_rect.3 <= 1.0
        || !plane.0.is_finite()
        || !plane.1.is_finite()
        || plane.0 <= 0.0
        || plane.1 <= 0.0
    {
        return None;
    }
    let rx = ((client.0 - radar_rect.0) / radar_rect.2).clamp(0.0, 1.0);
    let ry = ((client.1 - radar_rect.1) / radar_rect.3).clamp(0.0, 1.0);
    Some((rx * plane.0, ry * plane.1))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct RadarDrag {
    pub active: bool,
    pub pointer_id: i32,
    pub moved: bool,
}

impl RadarDrag {
    pub fn from_json(v: &Value) -> Self {
        Self {
            active: v["active"].as_bool().unwrap_or(false),
            pointer_id: v["pointer_id"].as_i64().unwrap_or(-1) as i32,
            moved: v["moved"].as_bool().unwrap_or(false),
        }
    }

    pub fn to_json(self) -> Value {
        json!({
            "active": self.active,
            "pointer_id": self.pointer_id,
            "moved": self.moved,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RadarAction {
    Ignored,
    MoveTo { world_point: (f64, f64) },
    Released,
}

pub fn capture_pointer(el: &Element, pointer_id: i32) {
    let _ = el.set_pointer_capture(pointer_id);
}

pub fn release_pointer(el: &Element, pointer_id: i32) {
    let _ = el.release_pointer_capture(pointer_id);
}

/// Handles pointer-captured drag/jump interactions on a radar minimap element.
pub fn handle_radar_pointer(
    kind: &str,
    event: &web_sys::Event,
    radar_el: &Element,
    canvas_id: Option<&str>,
    plane: (f64, f64),
    drag: &mut RadarDrag,
) -> RadarAction {
    let Ok(pe) = event.clone().dyn_into::<web_sys::PointerEvent>() else {
        return RadarAction::Ignored;
    };
    match kind {
        "pointerdown" if pe.button() == 0 => {
            event.prevent_default();
            capture_pointer(radar_el, pe.pointer_id());
            if let Some(cid) = canvas_id {
                dom::attr(cid, "data-panning", "true");
            }
            *drag = RadarDrag {
                active: true,
                pointer_id: pe.pointer_id(),
                moved: false,
            };
            let rect = radar_el.get_bounding_client_rect();
            match radar_pointer_map(
                (pe.client_x() as f64, pe.client_y() as f64),
                (rect.left(), rect.top(), rect.width(), rect.height()),
                plane,
            ) {
                Some(world_point) => RadarAction::MoveTo { world_point },
                None => RadarAction::Ignored,
            }
        }
        "pointermove" if drag.active && pe.pointer_id() == drag.pointer_id => {
            if pe.buttons() & 1 == 0 {
                release_pointer(radar_el, pe.pointer_id());
                if let Some(cid) = canvas_id {
                    dom::attr(cid, "data-panning", "false");
                }
                *drag = RadarDrag::default();
                return RadarAction::Released;
            }
            drag.moved = true;
            let rect = radar_el.get_bounding_client_rect();
            match radar_pointer_map(
                (pe.client_x() as f64, pe.client_y() as f64),
                (rect.left(), rect.top(), rect.width(), rect.height()),
                plane,
            ) {
                Some(world_point) => RadarAction::MoveTo { world_point },
                None => RadarAction::Ignored,
            }
        }
        "pointerup" | "pointercancel" if drag.active && pe.pointer_id() == drag.pointer_id => {
            release_pointer(radar_el, pe.pointer_id());
            if let Some(cid) = canvas_id {
                dom::attr(cid, "data-panning", "false");
            }
            *drag = RadarDrag::default();
            RadarAction::Released
        }
        _ => RadarAction::Ignored,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DragPan {
    pub active: bool,
    pub pointer_id: i32,
    pub start_x: f64,
    pub start_y: f64,
    pub orig_tx: f64,
    pub orig_ty: f64,
    pub moved: bool,
    pub suppress_click: bool,
}

impl Default for DragPan {
    fn default() -> Self {
        Self {
            active: false,
            pointer_id: -1,
            start_x: 0.0,
            start_y: 0.0,
            orig_tx: 0.0,
            orig_ty: 0.0,
            moved: false,
            suppress_click: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DragMove {
    Ignored,
    BelowThreshold,
    StartedPan { tx: f64, ty: f64 },
    Panned { tx: f64, ty: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DragEnd {
    pub was_moved: bool,
    pub suppress_click: bool,
}

impl DragPan {
    pub fn begin(&mut self, pointer_id: i32, client: (f64, f64), cam: Camera) {
        *self = Self {
            active: true,
            pointer_id,
            start_x: client.0,
            start_y: client.1,
            orig_tx: cam.tx,
            orig_ty: cam.ty,
            moved: false,
            suppress_click: false,
        };
    }

    pub fn advance(&mut self, pointer_id: i32, client: (f64, f64), threshold_px: f64) -> DragMove {
        if !self.active || pointer_id != self.pointer_id {
            return DragMove::Ignored;
        }
        let dx = client.0 - self.start_x;
        let dy = client.1 - self.start_y;
        let threshold = if threshold_px.is_finite() && threshold_px >= 0.0 {
            threshold_px
        } else {
            DEFAULT_DRAG_THRESHOLD_PX
        };
        let just_started = if !self.moved {
            if (dx * dx + dy * dy).sqrt() <= threshold {
                return DragMove::BelowThreshold;
            }
            self.moved = true;
            true
        } else {
            false
        };
        let tx = (self.orig_tx + dx).round();
        let ty = (self.orig_ty + dy).round();
        if just_started {
            DragMove::StartedPan { tx, ty }
        } else {
            DragMove::Panned { tx, ty }
        }
    }

    pub fn end(&mut self, pointer_id: Option<i32>, is_cancel: bool) -> Option<DragEnd> {
        if !self.active {
            return None;
        }
        if let Some(pid) = pointer_id {
            if pid != self.pointer_id {
                return None;
            }
        }
        let was_moved = self.moved;
        let suppress_click = !is_cancel && was_moved;
        *self = Self {
            active: false,
            pointer_id: -1,
            start_x: 0.0,
            start_y: 0.0,
            orig_tx: 0.0,
            orig_ty: 0.0,
            moved: false,
            suppress_click,
        };
        Some(DragEnd {
            was_moved,
            suppress_click,
        })
    }

    pub fn clear_suppress(&mut self) {
        self.suppress_click = false;
    }

    #[allow(dead_code)]
    pub fn should_suppress_click(&self) -> bool {
        self.suppress_click
    }

    pub fn from_json(v: &Value) -> Self {
        if !v.is_object() {
            return Self::default();
        }
        Self {
            active: v["active"].as_bool().unwrap_or(false),
            pointer_id: v["pointer_id"].as_i64().unwrap_or(-1) as i32,
            start_x: v["start_x"].as_f64().unwrap_or(0.0),
            start_y: v["start_y"].as_f64().unwrap_or(0.0),
            orig_tx: v["orig_tx"]
                .as_f64()
                .or_else(|| v["orig_pan_x"].as_f64())
                .unwrap_or(0.0),
            orig_ty: v["orig_ty"]
                .as_f64()
                .or_else(|| v["orig_pan_y"].as_f64())
                .unwrap_or(0.0),
            moved: v["moved"].as_bool().unwrap_or(false),
            suppress_click: v["suppress_click"].as_bool().unwrap_or(false),
        }
    }

    pub fn to_json(self) -> Value {
        json!({
            "active": self.active,
            "pointer_id": self.pointer_id,
            "start_x": self.start_x,
            "start_y": self.start_y,
            "orig_tx": self.orig_tx,
            "orig_ty": self.orig_ty,
            "moved": self.moved,
            "suppress_click": self.suppress_click,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CanvasDragOutcome {
    Ignored,
    Down,
    Panned { tx: f64, ty: f64 },
    Ended { suppress_click: bool },
}

/// Handles pointer-captured drag-to-pan across a graph canvas element.
pub fn handle_canvas_drag(
    kind: &str,
    event: &web_sys::Event,
    canvas_id: &str,
    cam: Camera,
    threshold_px: f64,
    drag: &mut DragPan,
) -> CanvasDragOutcome {
    match kind {
        "pointerdown" => {
            let Ok(pe) = event.clone().dyn_into::<web_sys::PointerEvent>() else {
                return CanvasDragOutcome::Ignored;
            };
            if pe.button() != 0 {
                return CanvasDragOutcome::Ignored;
            }
            drag.begin(
                pe.pointer_id(),
                (pe.client_x() as f64, pe.client_y() as f64),
                cam,
            );
            CanvasDragOutcome::Down
        }
        "pointermove" if drag.active => {
            let Ok(pe) = event.clone().dyn_into::<web_sys::PointerEvent>() else {
                return CanvasDragOutcome::Ignored;
            };
            match drag.advance(
                pe.pointer_id(),
                (pe.client_x() as f64, pe.client_y() as f64),
                threshold_px,
            ) {
                DragMove::StartedPan { tx, ty } => {
                    dom::attr(canvas_id, "data-panning", "true");
                    if let Some(canvas) = dom::get(canvas_id) {
                        capture_pointer(&canvas, pe.pointer_id());
                    }
                    CanvasDragOutcome::Panned { tx, ty }
                }
                DragMove::Panned { tx, ty } => CanvasDragOutcome::Panned { tx, ty },
                DragMove::Ignored | DragMove::BelowThreshold => CanvasDragOutcome::Ignored,
            }
        }
        "pointerup" | "pointercancel" if drag.active => {
            let pe = event.clone().dyn_into::<web_sys::PointerEvent>().ok();
            let pid = pe.as_ref().map(|p| p.pointer_id());
            let Some(end) = drag.end(pid, kind == "pointercancel") else {
                return CanvasDragOutcome::Ignored;
            };
            dom::attr(canvas_id, "data-panning", "false");
            if end.was_moved {
                if let (Some(canvas), Some(pointer_id)) = (dom::get(canvas_id), pid) {
                    release_pointer(&canvas, pointer_id);
                }
            }
            CanvasDragOutcome::Ended {
                suppress_click: end.suppress_click,
            }
        }
        _ => CanvasDragOutcome::Ignored,
    }
}

/// Attaches a non-passive `wheel` listener on `#<canvas_id>` (idempotently via
/// `data-wheel-bound="true"`), converting vertical deltas into zoom factors and
/// toggling `data-wheeling` while scrolling.
pub fn attach_wheel_zoom(
    canvas_id: &str,
    wheel_timer_key: &str,
    on_zoom: impl Fn(f64, (f64, f64)) + 'static,
) {
    let Some(canvas) = dom::get(canvas_id) else {
        return;
    };
    if canvas.get_attribute("data-wheel-bound").as_deref() == Some("true") {
        return;
    }
    let _ = canvas.set_attribute("data-wheel-bound", "true");
    let cid = canvas_id.to_owned();
    let timer_key = wheel_timer_key.to_owned();
    let callback = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        let Ok(wheel) = event.dyn_into::<web_sys::WheelEvent>() else {
            return;
        };
        let Some(factor) = wheel_zoom_factor(wheel.delta_y(), wheel.delta_mode()) else {
            return;
        };
        wheel.prevent_default();
        wheel.stop_propagation();
        dom::attr(&cid, "data-wheeling", "true");
        on_zoom(factor, (wheel.client_x() as f64, wheel.client_y() as f64));
        let cid_clear = cid.clone();
        dom::later(&timer_key, 140, move || {
            dom::attr(&cid_clear, "data-wheeling", "false");
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ResizeAction {
    Skip,
    Refit(Camera),
    SyncRadar(Camera),
}

/// Pure decision helper determining whether a resize event should refit the
/// plane, keep manual pan/zoom and re-sync radar, or skip when unmeasured.
pub fn resize_refit_decision(
    cam: Camera,
    fit_mode: bool,
    plane: (f64, f64),
    viewport: (f64, f64),
    measured: bool,
    options: FitOptions,
) -> ResizeAction {
    if !measured || plane.0 <= 0.0 || plane.1 <= 0.0 || viewport.0 <= 0.0 || viewport.1 <= 0.0 {
        return ResizeAction::Skip;
    }
    if fit_mode {
        ResizeAction::Refit(fit(plane, viewport, options))
    } else {
        ResizeAction::SyncRadar(cam)
    }
}

/// Registers an idempotent `window` `resize` observer keyed by `guard_key`.
/// When `debounce_key` is `Some` and `debounce_ms > 0`, rapid resize events
/// are coalesced through `dom::later` before calling `on_resize`.
pub fn ensure_resize_observer(
    guard_key: &str,
    debounce_key: Option<&str>,
    debounce_ms: i32,
    on_resize: impl Fn() + 'static,
) {
    if crate::model::yes(&crate::model::data(guard_key)) {
        return;
    }
    crate::model::set(guard_key, json!(true));
    let cb: Rc<dyn Fn()> = Rc::new(on_resize);
    let dkey = debounce_key.map(str::to_owned);
    let listener = Closure::<dyn FnMut()>::new(move || {
        if let Some(key) = dkey.as_deref() {
            if debounce_ms > 0 {
                let inner = Rc::clone(&cb);
                dom::later(key, debounce_ms, move || inner());
                return;
            }
        }
        cb();
    });
    if dom::window()
        .add_event_listener_with_callback("resize", listener.as_ref().unchecked_ref())
        .is_ok()
    {
        listener.forget();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_at_preserves_anchor_world_point_and_clamps_bounds() {
        let cam = Camera::new(20.0, 40.0, 1.0);
        let anchor = (220.0, 140.0);
        let zoomed = zoom_at(cam, anchor, 1.5, 0.25, 2.6);
        assert!((zoomed.scale - 1.5).abs() < 1e-6);
        let wx_before = (anchor.0 - cam.tx) / cam.scale;
        let wx_after = (anchor.0 - zoomed.tx) / zoomed.scale;
        let wy_before = (anchor.1 - cam.ty) / cam.scale;
        let wy_after = (anchor.1 - zoomed.ty) / zoomed.scale;
        assert!((wx_before - wx_after).abs() < 1.0);
        assert!((wy_before - wy_after).abs() < 1.0);

        let clamped_hi = zoom_at(cam, anchor, 10.0, 0.25, 2.4);
        assert_eq!(clamped_hi.scale, 2.4);
        let clamped_lo = zoom_at(cam, anchor, 0.01, 0.25, 2.4);
        assert_eq!(clamped_lo.scale, 0.25);
    }

    #[test]
    fn fit_and_proportional_fit_respect_margins_and_center_small_planes() {
        let omni = fit((1040.0, 540.0), (800.0, 500.0), FitOptions::for_omnigraph());
        assert!(omni.tx >= 36.0);
        assert!(omni.ty >= 24.0);
        assert!(omni.tx + 1040.0 * omni.scale <= 800.0 - 35.0);
        assert!(omni.scale <= 1.15);

        let small = fit_proportional((300.0, 200.0), (800.0, 500.0), 0.94, 0.25, 1.0, 12.0);
        assert_eq!(small, Camera::new(250.0, 150.0, 1.0));

        assert_eq!(
            fit((0.0, 100.0), (800.0, 500.0), FitOptions::default()),
            Camera::default()
        );
    }

    #[test]
    fn lod_and_transform_3d_formatting_match_thresholds() {
        assert_eq!(lod(0.49), "far");
        assert_eq!(lod(0.5), "mid");
        assert_eq!(lod(1.4), "mid");
        assert_eq!(lod(1.41), "near");
        assert_eq!(lod(0.0), "mid");
        assert_eq!(lod(f64::NAN), "mid");

        assert_eq!(lod_with(0.4, OMNIGRAPH_LOD), "far");
        assert_eq!(lod_with(0.6, OMNIGRAPH_LOD), "mid");
        assert_eq!(lod_with(0.94, OMNIGRAPH_LOD), "mid");
        assert_eq!(lod_with(0.95, OMNIGRAPH_LOD), "near");
        assert_eq!(lod_with(0.0, OMNIGRAPH_LOD), "near");

        let style = plane_transform_style(Camera::new(-24.0, 12.0, 1.25), (960.0, 540.0));
        assert!(style.contains("width:960px;height:540px;"));
        assert!(style.contains("transform:translate3d(-24.0px,12.0px,0) scale(1.2500);"));
        assert!(style.contains("--graph-scale:1.2500;--graph-pan-x:-24.0px;--graph-pan-y:12.0px;"));
    }

    #[test]
    fn radar_pointer_map_and_viewport_rect_handle_bounds_and_invalid_inputs() {
        assert_eq!(
            radar_pointer_map((100.0, 50.0), (100.0, 50.0, 200.0, 100.0), (1000.0, 500.0)),
            Some((0.0, 0.0))
        );
        assert_eq!(
            radar_pointer_map((200.0, 100.0), (100.0, 50.0, 200.0, 100.0), (1000.0, 500.0)),
            Some((500.0, 250.0))
        );
        // Clamps out-of-bounds pointer positions onto the plane edges.
        assert_eq!(
            radar_pointer_map((999.0, 999.0), (100.0, 50.0, 200.0, 100.0), (1000.0, 500.0)),
            Some((1000.0, 500.0))
        );
        assert_eq!(
            radar_pointer_map((10.0, 10.0), (0.0, 0.0, 1.0, 100.0), (1000.0, 500.0)),
            None
        );

        let rect = radar_viewport_rect(
            Camera::new(-100.0, -50.0, 2.0),
            (400.0, 200.0),
            (1000.0, 500.0),
            16.0,
        );
        assert_eq!(rect, Some((50.0, 25.0, 200.0, 100.0)));

        // Whole plane fits in viewport -> radar_window_rect returns None.
        assert_eq!(
            radar_window_rect(Camera::new(0.0, 0.0, 1.0), (800.0, 600.0), (400.0, 300.0)),
            None
        );
        assert!(
            radar_window_rect(Camera::new(0.0, 0.0, 1.0), (400.0, 300.0), (1000.0, 600.0))
                .is_some()
        );
    }

    #[test]
    fn drag_pan_state_enforces_threshold_pointer_id_and_click_suppression() {
        let mut drag = DragPan::default();
        drag.begin(7, (100.0, 200.0), Camera::new(10.0, 20.0, 1.0));

        // Unrelated pointer is ignored.
        assert_eq!(drag.advance(8, (200.0, 200.0), 4.0), DragMove::Ignored);
        assert_eq!(drag.end(Some(8), false), None);

        // Within threshold keeps click intact.
        assert_eq!(
            drag.advance(7, (102.0, 202.0), 4.0),
            DragMove::BelowThreshold
        );
        assert!(!drag.moved);

        // Crossing threshold starts pan cleanly.
        assert_eq!(
            drag.advance(7, (115.0, 190.0), 4.0),
            DragMove::StartedPan { tx: 25.0, ty: 10.0 }
        );
        assert_eq!(
            drag.advance(7, (125.0, 185.0), 4.0),
            DragMove::Panned { tx: 35.0, ty: 5.0 }
        );

        let end = drag.end(Some(7), false).expect("active drag ends");
        assert!(end.was_moved);
        assert!(end.suppress_click);
        assert!(drag.should_suppress_click());

        let roundtrip = DragPan::from_json(&drag.to_json());
        assert_eq!(roundtrip, drag);
        drag.clear_suppress();
        assert!(!drag.should_suppress_click());

        // Cancel does not suppress click.
        drag.begin(3, (0.0, 0.0), Camera::default());
        assert_eq!(
            drag.advance(3, (20.0, 0.0), 4.0),
            DragMove::StartedPan { tx: 20.0, ty: 0.0 }
        );
        let cancelled = drag.end(Some(3), true).unwrap();
        assert!(cancelled.was_moved);
        assert!(!cancelled.suppress_click);
    }

    #[test]
    fn resize_refit_decision_and_wheel_zoom_factor_behave_cleanly() {
        let cam = Camera::new(30.0, 40.0, 1.5);
        assert_eq!(
            resize_refit_decision(
                cam,
                true,
                (1000.0, 500.0),
                (800.0, 500.0),
                false,
                FitOptions::for_omnigraph()
            ),
            ResizeAction::Skip
        );
        match resize_refit_decision(
            cam,
            true,
            (1000.0, 500.0),
            (800.0, 500.0),
            true,
            FitOptions::for_omnigraph(),
        ) {
            ResizeAction::Refit(next) => assert!(next.scale < 1.0),
            other => panic!("expected Refit, got {other:?}"),
        }
        assert_eq!(
            resize_refit_decision(
                cam,
                false,
                (1000.0, 500.0),
                (800.0, 500.0),
                true,
                FitOptions::for_omnigraph()
            ),
            ResizeAction::SyncRadar(cam)
        );
        assert_eq!(wheel_zoom_factor(0.0, 0), None);
        assert!(wheel_zoom_factor(-120.0, 0).unwrap() > 1.0);
        assert!(wheel_zoom_factor(120.0, 0).unwrap() < 1.0);
    }

    #[test]
    fn shared_lod_proportional_fit_bbox_and_fly_to_match_contract() {
        assert_eq!(lod(0.49), "far");
        assert_eq!(lod(0.5), "mid");
        assert_eq!(lod(1.4), "mid");
        assert_eq!(lod(1.41), "near");
        assert_eq!(lod(0.0), "mid");
        assert_eq!(lod(f64::NAN), "mid");

        let prop = fit_proportional((2000.0, 1000.0), (1000.0, 600.0), 0.94, 0.25, 2.4, 24.0);
        assert!((prop.scale - 0.47).abs() < 1e-6);

        let boxed = fit_bbox(
            (100.0, 200.0, 500.0, 400.0),
            (1000.0, 800.0),
            (40.0, 40.0),
            (180.0, 140.0),
            0.9,
            0.25,
            2.0,
        );
        let center_x = boxed.tx + 300.0 * boxed.scale;
        let center_y = boxed.ty + 300.0 * boxed.scale;
        assert!((center_x - 500.0).abs() < 1e-6);
        assert!((center_y - 400.0).abs() < 1e-6);

        let flown = fly_to_point((600.0, 400.0), (800.0, 600.0), 1.5, 0.75, 2.4);
        assert_eq!(flown.scale, 1.5);
        assert!((flown.tx + 600.0 * 1.5 - 400.0).abs() < 1e-6);
        assert!((flown.ty + 400.0 * 1.5 - 300.0).abs() < 1e-6);

        let sys_opts = FitOptions::for_system_map();
        assert_eq!(sys_opts.margin_x, 12.0);
        assert_eq!(
            radar_window_rect(Camera::new(0.0, 0.0, 1.0), (400.0, 300.0), (1200.0, 900.0)),
            Some((0.0, 0.0, 400.0, 300.0))
        );
    }
}
