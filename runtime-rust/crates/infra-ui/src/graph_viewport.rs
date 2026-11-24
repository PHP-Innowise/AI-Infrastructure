//! Shared 2D graph viewport camera, LOD, pointer-capture drag-pan, radar minimap,
//! wheel zoom, and resize-refit helpers for Harness graph canvases.
//!
//! Pure geometry and state-transition functions live separately from `web_sys`
//! DOM bindings so native `cargo test` runs deterministic unit tests without a
//! browser DOM while `wasm32-unknown-unknown` shares the exact same math.

use crate::dom;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::rc::Rc;
use wasm_bindgen::{prelude::*, JsCast};
use web_sys::Element;

pub mod dense_dom;

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
    let raw_tx = (viewport.0 - pw * scale) / 2.0;
    let raw_ty = (viewport.1 - ph * scale) / 2.0;
    let tx = if pw * scale <= viewport.0 - options.margin_x * 2.0 + 0.5 {
        raw_tx.max(options.margin_x).floor()
    } else {
        raw_tx.round()
    };
    let ty = if ph * scale <= viewport.1 - options.margin_y * 2.0 + 0.5 {
        raw_ty.max(options.margin_y).floor()
    } else {
        raw_ty.round()
    };
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
        "width:{w:.0}px;height:{h:.0}px;--graph-scale:{:.4};--graph-pan-x:{:.1}px;--graph-pan-y:{:.1}px;transform:translate3d({:.1}px,{:.1}px,0) scale({:.4});transform-origin:0 0;",
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
    let mut style = plane_transform_style(cam, plane_size);
    if canvas.get_attribute("data-dense").as_deref() == Some("true") {
        style.push_str("transition:none;");
    }
    let _ = plane.set_attribute("style", &style);
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
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (canvas_id, min_dim);
        (fallback.0, fallback.1, false)
    }
    #[cfg(target_arch = "wasm32")]
    {
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
    let min_w = if min_side.is_finite() {
        min_side.max(0.0).min(plane.0)
    } else {
        0.0
    };
    let min_h = if min_side.is_finite() {
        min_side.max(0.0).min(plane.1)
    } else {
        0.0
    };
    let vp_w = if viewport.0.is_finite() {
        viewport.0.max(0.0)
    } else {
        0.0
    };
    let vp_h = if viewport.1.is_finite() {
        viewport.1.max(0.0)
    } else {
        0.0
    };
    let max_vx = (plane.0 - min_w).max(0.0);
    let max_vy = (plane.1 - min_h).max(0.0);
    let vx = (-cam.tx / cam.scale).clamp(0.0, max_vx);
    let vy = (-cam.ty / cam.scale).clamp(0.0, max_vy);
    let max_vw = (plane.0 - vx).max(min_w);
    let max_vh = (plane.1 - vy).max(min_h);
    let vw = (vp_w / cam.scale).clamp(min_w, max_vw);
    let vh = (vp_h / cam.scale).clamp(min_h, max_vh);
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

/// Registers an idempotent `window` `resize` observer keyed by `guard_key` on `dom::on_viewport_resize`.
/// When `debounce_key` is `Some` and `debounce_ms > 0`, rapid resize events
/// are coalesced through `dom::later` before calling `on_resize`.
pub fn ensure_resize_observer(
    guard_key: &str,
    debounce_key: Option<&str>,
    debounce_ms: i32,
    on_resize: impl Fn() + 'static,
) {
    let effective_ms = if debounce_key.is_some() {
        debounce_ms
    } else {
        0
    };
    dom::on_viewport_resize(guard_key, effective_ms, on_resize);
}

// =========================================================================
// Shared Pinning Overrides, Path/Isolation Math, Roving Focus & Canvas2D
// =========================================================================

type PinMap = BTreeMap<String, (f64, f64)>;
type PinCacheMap = HashMap<String, PinMap>;

thread_local! {
    static PIN_CACHE: RefCell<PinCacheMap> =
        RefCell::new(HashMap::new());
}

pub fn pin_storage_key(surface: &str, scope_key: &str, mode_key: &str) -> String {
    let clean = |s: &str| {
        s.trim()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/') {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>()
    };
    format!(
        "infra-graph-pins:{}:{}:{}",
        clean(surface),
        if scope_key.trim().is_empty() {
            "global".to_owned()
        } else {
            clean(scope_key)
        },
        if mode_key.trim().is_empty() {
            "default".to_owned()
        } else {
            clean(mode_key)
        }
    )
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn parse_pins_json(raw: &Value) -> BTreeMap<String, (f64, f64)> {
    let mut out = BTreeMap::new();
    let Some(obj) = raw.as_object() else {
        return out;
    };
    for (k, v) in obj {
        let x = v["x"].as_f64();
        let y = v["y"].as_f64();
        if let (Some(x), Some(y)) = (x, y) {
            if x.is_finite() && y.is_finite() {
                out.insert(k.clone(), (x, y));
            }
        }
    }
    out
}

pub fn load_pins(storage_key: &str) -> BTreeMap<String, (f64, f64)> {
    if let Some(cached) = PIN_CACHE.with(|c| c.borrow().get(storage_key).cloned()) {
        return cached;
    }
    #[cfg(target_arch = "wasm32")]
    {
        let parsed = parse_pins_json(&dom::json_storage(storage_key));
        PIN_CACHE.with(|c| {
            c.borrow_mut()
                .insert(storage_key.to_owned(), parsed.clone());
        });
        parsed
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        BTreeMap::new()
    }
}

pub fn save_pin(storage_key: &str, node_key: &str, pos: (f64, f64)) {
    if storage_key.is_empty() || node_key.is_empty() || !pos.0.is_finite() || !pos.1.is_finite() {
        return;
    }
    let mut map = load_pins(storage_key);
    map.insert(node_key.to_owned(), (pos.0.max(4.0), pos.1.max(4.0)));
    persist_pins_map(storage_key, &map);
}

pub fn persist_pins_map(storage_key: &str, map: &BTreeMap<String, (f64, f64)>) {
    PIN_CACHE.with(|c| {
        c.borrow_mut().insert(storage_key.to_owned(), map.clone());
    });
    #[cfg(target_arch = "wasm32")]
    {
        let mut obj = serde_json::Map::new();
        for (k, (x, y)) in map {
            obj.insert(k.clone(), json!({ "x": x, "y": y }));
        }
        dom::save(storage_key, Some(&Value::Object(obj).to_string()));
    }
}

pub fn clear_pins(storage_key: &str) {
    PIN_CACHE.with(|c| {
        c.borrow_mut().remove(storage_key);
    });
    #[cfg(target_arch = "wasm32")]
    dom::save(storage_key, None);
}

/// Removes a single node pin from persistent storage and returns `true` if present.
#[allow(dead_code)]
pub fn remove_pin(storage_key: &str, node_key: &str) -> bool {
    if storage_key.is_empty() || node_key.is_empty() {
        return false;
    }
    let mut map = load_pins(storage_key);
    let removed = map.remove(node_key).is_some();
    if removed {
        if map.is_empty() {
            clear_pins(storage_key);
        } else {
            persist_pins_map(storage_key, &map);
        }
    }
    removed
}

#[allow(unused_imports)]
pub(crate) use crate::dom::escape_css_attr_value;

/// Shifts `cam.tx`/`cam.ty` minimally (keeping `cam.scale`) so world `rect` `(x, y, w, h)`
/// sits inside the `[margin .. viewport - margin]` safe region. Returns `None` when already visible.
#[allow(dead_code)]
pub fn pan_to_keep_rect_in_view(
    cam: Camera,
    rect: (f64, f64, f64, f64),
    viewport: (f64, f64),
    margin: f64,
) -> Option<Camera> {
    if !cam.scale.is_finite()
        || cam.scale <= 0.0
        || !rect.0.is_finite()
        || !rect.1.is_finite()
        || !rect.2.is_finite()
        || !rect.3.is_finite()
        || !viewport.0.is_finite()
        || !viewport.1.is_finite()
        || viewport.0 <= 0.0
        || viewport.1 <= 0.0
    {
        return None;
    }
    let sw = (rect.2.max(0.0)) * cam.scale;
    let sh = (rect.3.max(0.0)) * cam.scale;
    let sx0 = rect.0 * cam.scale + cam.tx;
    let sy0 = rect.1 * cam.scale + cam.ty;
    let sx1 = sx0 + sw;
    let sy1 = sy0 + sh;
    let safe_margin = if margin.is_finite() {
        margin.max(0.0)
    } else {
        0.0
    };
    let mx = safe_margin.min((viewport.0 - 4.0).max(0.0) * 0.4);
    let my = safe_margin.min((viewport.1 - 4.0).max(0.0) * 0.4);

    let dx = if sw > viewport.0 - mx * 2.0 {
        if sx1 < mx || sx0 > viewport.0 - mx {
            (viewport.0 - sw) * 0.5 - sx0
        } else {
            0.0
        }
    } else if sx0 < mx {
        mx - sx0
    } else if sx1 > viewport.0 - mx {
        (viewport.0 - mx) - sx1
    } else {
        0.0
    };

    let dy = if sh > viewport.1 - my * 2.0 {
        if sy1 < my || sy0 > viewport.1 - my {
            (viewport.1 - sh) * 0.5 - sy0
        } else {
            0.0
        }
    } else if sy0 < my {
        my - sy0
    } else if sy1 > viewport.1 - my {
        (viewport.1 - my) - sy1
    } else {
        0.0
    };

    if dx.abs() < 0.5 && dy.abs() < 0.5 {
        return None;
    }
    Some(Camera {
        tx: (cam.tx + dx).round(),
        ty: (cam.ty + dy).round(),
        scale: cam.scale,
    })
}

/// Snapshot of camera + focus/target state for Back/Forward navigation & breadcrumbs.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub struct CameraSelectionEntry {
    pub key: String,
    pub label: String,
    pub target: String,
    pub cam: Camera,
}

/// Bounded browser-style Back/Forward camera & selection history stack.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct CameraSelectionHistory {
    entries: Vec<CameraSelectionEntry>,
    cursor: usize,
    max_entries: usize,
}

impl Default for CameraSelectionHistory {
    fn default() -> Self {
        Self::new(48)
    }
}

#[allow(dead_code)]
impl CameraSelectionHistory {
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Vec::new(),
            cursor: 0,
            max_entries: max_entries.max(2),
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.cursor = 0;
    }

    pub fn entries(&self) -> &[CameraSelectionEntry] {
        &self.entries
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn current(&self) -> Option<&CameraSelectionEntry> {
        self.entries.get(self.cursor)
    }

    pub fn can_back(&self) -> bool {
        !self.entries.is_empty() && self.cursor > 0
    }

    pub fn can_forward(&self) -> bool {
        self.cursor + 1 < self.entries.len()
    }

    pub fn update_active_camera(&mut self, cam: Camera) {
        if !cam.scale.is_finite() || cam.scale <= 0.0 {
            return;
        }
        if let Some(active) = self.entries.get_mut(self.cursor) {
            active.cam = cam;
        }
    }

    pub fn push(&mut self, entry: CameraSelectionEntry) {
        if !entry.cam.scale.is_finite() || entry.cam.scale <= 0.0 {
            return;
        }
        if let Some(cur) = self.entries.get(self.cursor) {
            if cur.key == entry.key && cur.target == entry.target {
                if let Some(mut_cur) = self.entries.get_mut(self.cursor) {
                    mut_cur.cam = entry.cam;
                    if !entry.label.is_empty() {
                        mut_cur.label = entry.label;
                    }
                }
                return;
            }
        }
        if !self.entries.is_empty() && self.cursor + 1 < self.entries.len() {
            self.entries.truncate(self.cursor + 1);
        }
        self.entries.push(entry);
        if self.entries.len() > self.max_entries {
            let overflow = self.entries.len() - self.max_entries;
            self.entries.drain(0..overflow);
        }
        self.cursor = self.entries.len().saturating_sub(1);
    }

    pub fn back(&mut self) -> Option<&CameraSelectionEntry> {
        if !self.can_back() {
            return None;
        }
        self.cursor -= 1;
        self.entries.get(self.cursor)
    }

    pub fn forward(&mut self) -> Option<&CameraSelectionEntry> {
        if !self.can_forward() {
            return None;
        }
        self.cursor += 1;
        self.entries.get(self.cursor)
    }

    pub fn jump_to(&mut self, index: usize) -> Option<&CameraSelectionEntry> {
        if index >= self.entries.len() {
            return None;
        }
        self.cursor = index;
        self.entries.get(self.cursor)
    }
}

pub fn has_pins(storage_key: &str) -> bool {
    !load_pins(storage_key).is_empty()
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn merge_pin_positions(
    keys: &[String],
    positions: &mut [(f64, f64)],
    pins: &BTreeMap<String, (f64, f64)>,
) -> usize {
    let mut count = 0usize;
    for (idx, key) in keys.iter().enumerate() {
        if idx >= positions.len() {
            break;
        }
        if let Some(&(x, y)) = pins.get(key) {
            positions[idx] = (x, y);
            count += 1;
        }
    }
    count
}

pub fn shortest_path_bfs(
    node_count: usize,
    out_adj: &[Vec<usize>],
    undirected: bool,
    from: usize,
    to: usize,
) -> Option<Vec<usize>> {
    if from >= node_count || to >= node_count {
        return None;
    }
    if from == to {
        return Some(vec![from]);
    }
    let mut rev_adj: Vec<Vec<usize>> = if undirected {
        let mut rev = vec![Vec::new(); node_count];
        for (u, nbrs) in out_adj.iter().enumerate() {
            for &v in nbrs {
                if v < node_count {
                    rev[v].push(u);
                }
            }
        }
        rev
    } else {
        Vec::new()
    };
    let mut prev: Vec<Option<usize>> = vec![None; node_count];
    let mut seen = vec![false; node_count];
    let mut queue = VecDeque::new();
    seen[from] = true;
    queue.push_back(from);
    while let Some(curr) = queue.pop_front() {
        if curr == to {
            let mut path = vec![to];
            let mut walk = to;
            while let Some(p) = prev[walk] {
                path.push(p);
                walk = p;
            }
            path.reverse();
            return Some(path);
        }
        if let Some(nbrs) = out_adj.get(curr) {
            for &next in nbrs {
                if next < node_count && !seen[next] {
                    seen[next] = true;
                    prev[next] = Some(curr);
                    queue.push_back(next);
                }
            }
        }
        if undirected {
            if let Some(in_nbrs) = rev_adj.get_mut(curr) {
                for &next in in_nbrs.iter() {
                    if next < node_count && !seen[next] {
                        seen[next] = true;
                        prev[next] = Some(curr);
                        queue.push_back(next);
                    }
                }
            }
        }
    }
    None
}

pub fn hop_cone_distances(
    node_count: usize,
    out_adj: &[Vec<usize>],
    in_adj: &[Vec<usize>],
    seeds: &[usize],
    max_hops: usize,
) -> Vec<Option<usize>> {
    let mut dist = vec![None; node_count];
    let mut queue = VecDeque::new();
    for &seed in seeds {
        if seed < node_count && dist[seed].is_none() {
            dist[seed] = Some(0);
            queue.push_back(seed);
        }
    }
    while let Some(curr) = queue.pop_front() {
        let d = dist[curr].unwrap_or(0);
        if d >= max_hops {
            continue;
        }
        if let Some(out) = out_adj.get(curr) {
            for &next in out {
                if next < node_count && dist[next].is_none() {
                    dist[next] = Some(d + 1);
                    queue.push_back(next);
                }
            }
        }
        if let Some(inc) = in_adj.get(curr) {
            for &next in inc {
                if next < node_count && dist[next].is_none() {
                    dist[next] = Some(d + 1);
                    queue.push_back(next);
                }
            }
        }
    }
    dist
}

pub fn next_roving_node_index(
    rects: &[(f64, f64, f64, f64)],
    connected_neighbors: &[usize],
    current: usize,
    key: &str,
) -> Option<usize> {
    if rects.is_empty() {
        return None;
    }
    if key == "Home" {
        return (0..rects.len()).min_by(|&a, &b| {
            let (ax, ay, _, _) = rects[a];
            let (bx, by, _, _) = rects[b];
            ax.total_cmp(&bx).then_with(|| ay.total_cmp(&by))
        });
    }
    if key == "End" {
        return (0..rects.len()).max_by(|&a, &b| {
            let (ax, ay, _, _) = rects[a];
            let (bx, by, _, _) = rects[b];
            ax.total_cmp(&bx).then_with(|| ay.total_cmp(&by))
        });
    }
    let dir = match key {
        "ArrowRight" => (1.0, 0.0),
        "ArrowLeft" => (-1.0, 0.0),
        "ArrowDown" => (0.0, 1.0),
        "ArrowUp" => (0.0, -1.0),
        _ => return None,
    };
    let (cx, cy, cw, ch) = *rects.get(current)?;
    let ox = cx + cw * 0.5;
    let oy = cy + ch * 0.5;

    let best_from = |candidates: &[usize]| -> Option<usize> {
        let mut best: Option<(usize, f64)> = None;
        for &idx in candidates {
            if idx == current || idx >= rects.len() {
                continue;
            }
            let (nx, ny, nw, nh) = rects[idx];
            let dx = (nx + nw * 0.5) - ox;
            let dy = (ny + nh * 0.5) - oy;
            let along = dx * dir.0 + dy * dir.1;
            if along <= 2.0 {
                continue;
            }
            let cross = (dx * dir.1 - dy * dir.0).abs();
            // Favor connected / tightly aligned nodes in the pressed arrow direction.
            let score = along + cross * 1.65;
            if best.is_none_or(|(_, s)| score < s) {
                best = Some((idx, score));
            }
        }
        best.map(|(i, _)| i)
    };

    if !connected_neighbors.is_empty() {
        if let Some(hit) = best_from(connected_neighbors) {
            return Some(hit);
        }
    }
    let all: Vec<usize> = (0..rects.len()).collect();
    best_from(&all)
}

#[allow(unused_variables)]
pub fn announce_live(element_id: &str, message: &str) {
    #[cfg(target_arch = "wasm32")]
    if !element_id.is_empty() {
        dom::text(element_id, message);
    }
}

/// Pure reconciliation plan for dense viewport DOM culling / reveal-on-pan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewportReconcilePlan {
    pub to_mount: Vec<usize>,
    pub to_unmount: Vec<usize>,
    pub to_offscreen: Vec<(usize, bool)>,
}

/// Returns true when rectangle `(x, y, w, h)` intersects `(min_x, min_y, max_x, max_y)`.
pub fn rect_intersects_bounds(
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    bounds: (f64, f64, f64, f64),
) -> bool {
    x + w >= bounds.0 && x <= bounds.2 && y + h >= bounds.1 && y <= bounds.3
}

/// Computes world-space viewport bounds `(min_x, min_y, max_x, max_y)` with margin.
pub fn world_viewport_bounds(
    cam: Camera,
    viewport: (f64, f64),
    margin: f64,
) -> Option<(f64, f64, f64, f64)> {
    if !(cam.scale.is_finite() && cam.scale > 0.0 && viewport.0 > 0.0 && viewport.1 > 0.0) {
        return None;
    }
    let pad = margin.max(0.0);
    Some((
        (-cam.tx / cam.scale) - pad,
        (-cam.ty / cam.scale) - pad,
        (-cam.tx + viewport.0) / cam.scale + pad,
        (-cam.ty + viewport.1) / cam.scale + pad,
    ))
}

/// Plans which nodes to mount, unmount (with hysteresis), or toggle `data-offscreen`
/// on pan/zoom so dense graphs reveal offscreen nodes without leaking unbounded DOM nodes.
pub fn plan_viewport_reconcile(
    rects: &[(f64, f64, f64, f64)],
    mounted: &[(usize, bool)],
    visible_bounds: (f64, f64, f64, f64),
    mount_bounds: (f64, f64, f64, f64),
    unmount_bounds: (f64, f64, f64, f64),
    force_mount: &[usize],
) -> ViewportReconcilePlan {
    let mut mounted_keep: HashMap<usize, bool> = HashMap::with_capacity(mounted.len());
    for &(idx, keep) in mounted {
        mounted_keep
            .entry(idx)
            .and_modify(|prev| *prev |= keep)
            .or_insert(keep);
    }
    let force_set: HashSet<usize> = force_mount.iter().copied().collect();
    let mut plan = ViewportReconcilePlan::default();

    for (idx, &(x, y, w, h)) in rects.iter().enumerate() {
        let forced = force_set.contains(&idx);
        let in_mount = rect_intersects_bounds(x, y, w, h, mount_bounds);
        let in_vis = rect_intersects_bounds(x, y, w, h, visible_bounds);
        if let Some(&keep) = mounted_keep.get(&idx) {
            let offscreen = !forced && !keep && !in_vis;
            if !forced && !keep && !rect_intersects_bounds(x, y, w, h, unmount_bounds) {
                plan.to_unmount.push(idx);
            } else {
                plan.to_offscreen.push((idx, offscreen));
            }
        } else if forced || in_mount {
            plan.to_mount.push(idx);
        }
    }

    for &(idx, _) in mounted {
        if idx >= rects.len() {
            plan.to_unmount.push(idx);
        }
    }
    plan
}

/// Parses an HTML fragment containing sibling node elements and appends them into `plane`,
/// returning the appended elements so the caller can sync attributes on them in one pass.
#[cfg(target_arch = "wasm32")]
pub fn append_html_elements(plane: &Element, batch_html: &str) -> Vec<Element> {
    if batch_html.is_empty() {
        return Vec::new();
    }
    let Ok(wrapper) = dom::document().create_element("div") else {
        return Vec::new();
    };
    wrapper.set_inner_html(batch_html);
    let mut inserted = Vec::new();
    while let Some(child) = wrapper.first_element_child() {
        if plane.append_child(&child).is_ok() {
            inserted.push(child);
        } else {
            child.remove();
        }
    }
    inserted
}

#[cfg(not(target_arch = "wasm32"))]
pub fn append_html_elements(_plane: &Element, _batch_html: &str) -> Vec<Element> {
    Vec::new()
}

pub(crate) fn hash_json_value<H: std::hash::Hasher>(val: &Value, state: &mut H) {
    use std::hash::Hash;
    match val {
        Value::Null => 0u8.hash(state),
        Value::Bool(b) => {
            1u8.hash(state);
            b.hash(state);
        }
        Value::Number(n) => {
            2u8.hash(state);
            n.to_string().hash(state);
        }
        Value::String(s) => {
            3u8.hash(state);
            s.hash(state);
        }
        Value::Array(items) => {
            4u8.hash(state);
            items.len().hash(state);
            for item in items {
                hash_json_value(item, state);
            }
        }
        Value::Object(map) => {
            5u8.hash(state);
            map.len().hash(state);
            for (k, v) in map {
                k.hash(state);
                hash_json_value(v, state);
            }
        }
    }
}

pub(crate) mod canvas2d {
    use super::*;

    const NODE_HIT_PAD: f64 = 2.0;
    const EDGE_HIT_PAD: f64 = 12.0;

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct DenseNodeSpec {
        pub index: usize,
        pub x: f64,
        pub y: f64,
        pub w: f64,
        pub h: f64,
        pub label: String,
        pub role: String,
        pub selected: bool,
        pub lit: bool,
        pub dim: bool,
        pub changed: bool,
        pub pinned: bool,
        pub failed: bool,
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum DensePathCmd {
        Move(f64, f64),
        Line(f64, f64),
        Curve {
            c1: (f64, f64),
            c2: (f64, f64),
            to: (f64, f64),
        },
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct DenseEdgeSpec {
        pub index: usize,
        pub pts: Vec<(f64, f64)>,
        pub cmds: Vec<DensePathCmd>,
        pub weight: u64,
        pub guessed: bool,
        pub against: bool,
        pub kind: String,
        pub on: bool,
        pub lit: bool,
        pub dim: bool,
        pub portal: bool,
        pub diff: String,
        pub impact: String,
        pub commit_touched: bool,
        pub back: bool,
        pub detour: bool,
        pub kinds: Vec<String>,
    }

    #[allow(clippy::too_many_arguments)]
    impl DenseEdgeSpec {
        pub fn from_svg_path(
            index: usize,
            path_d: &str,
            weight: u64,
            guessed: bool,
            against: bool,
            kind: String,
            on: bool,
            lit: bool,
            dim: bool,
            portal: bool,
        ) -> Self {
            let cmds = parse_svg_path_cmds(path_d);
            let pts = sample_cmds_polyline(&cmds);
            Self {
                index,
                pts,
                cmds,
                weight,
                guessed,
                against,
                kind,
                on,
                lit,
                dim,
                portal,
                ..Default::default()
            }
        }

        pub fn from_cubic(
            index: usize,
            p0: (f64, f64),
            p1: (f64, f64),
            p2: (f64, f64),
            p3: (f64, f64),
            weight: u64,
            guessed: bool,
            against: bool,
            kind: String,
            on: bool,
            lit: bool,
            dim: bool,
            portal: bool,
        ) -> Self {
            let cmds = vec![
                DensePathCmd::Move(p0.0, p0.1),
                DensePathCmd::Curve {
                    c1: p1,
                    c2: p2,
                    to: p3,
                },
            ];
            let pts = sample_cmds_polyline(&cmds);
            Self {
                index,
                pts,
                cmds,
                weight,
                guessed,
                against,
                kind,
                on,
                lit,
                dim,
                portal,
                ..Default::default()
            }
        }

        #[allow(dead_code)]
        pub fn with_semantics(
            mut self,
            diff: impl Into<String>,
            impact: impl Into<String>,
            commit_touched: bool,
            back: bool,
            detour: bool,
            kinds: Vec<String>,
        ) -> Self {
            self.diff = diff.into();
            self.impact = impact.into();
            self.commit_touched = commit_touched;
            self.back = back;
            self.detour = detour;
            self.kinds = kinds;
            self
        }

        #[allow(dead_code)]
        pub fn set_svg_path(&mut self, path_d: &str) {
            self.cmds = parse_svg_path_cmds(path_d);
            self.pts = sample_cmds_polyline(&self.cmds);
        }

        #[allow(dead_code)]
        pub fn set_cubic(
            &mut self,
            p0: (f64, f64),
            p1: (f64, f64),
            p2: (f64, f64),
            p3: (f64, f64),
        ) {
            self.cmds = vec![
                DensePathCmd::Move(p0.0, p0.1),
                DensePathCmd::Curve {
                    c1: p1,
                    c2: p2,
                    to: p3,
                },
            ];
            self.pts = sample_cmds_polyline(&self.cmds);
        }
    }

    /// Parses SVG `M`/`m`/`L`/`l`/`H`/`h`/`V`/`v`/`C`/`c`/`Z`/`z` path commands into absolute `DensePathCmd`s.
    pub fn parse_svg_path_cmds(path_d: &str) -> Vec<DensePathCmd> {
        let bytes = path_d.as_bytes();
        let mut i = 0usize;
        let len = bytes.len();
        let skip_sep = |idx: &mut usize| {
            while *idx < len && (bytes[*idx].is_ascii_whitespace() || bytes[*idx] == b',') {
                *idx += 1;
            }
        };
        let next_num = |idx: &mut usize| -> Option<f64> {
            skip_sep(idx);
            if *idx >= len || bytes[*idx].is_ascii_alphabetic() {
                return None;
            }
            let start = *idx;
            if bytes[*idx] == b'+' || bytes[*idx] == b'-' {
                *idx += 1;
            }
            let mut has_digit = false;
            while *idx < len && bytes[*idx].is_ascii_digit() {
                has_digit = true;
                *idx += 1;
            }
            if *idx < len && bytes[*idx] == b'.' {
                *idx += 1;
                while *idx < len && bytes[*idx].is_ascii_digit() {
                    has_digit = true;
                    *idx += 1;
                }
            }
            if !has_digit {
                *idx = start;
                return None;
            }
            if *idx < len && (bytes[*idx] == b'e' || bytes[*idx] == b'E') {
                let exp_pos = *idx;
                *idx += 1;
                if *idx < len && (bytes[*idx] == b'+' || bytes[*idx] == b'-') {
                    *idx += 1;
                }
                let mut exp_digits = false;
                while *idx < len && bytes[*idx].is_ascii_digit() {
                    exp_digits = true;
                    *idx += 1;
                }
                if !exp_digits {
                    *idx = exp_pos;
                }
            }
            path_d[start..*idx].parse::<f64>().ok()
        };

        let mut cmds = Vec::new();
        let mut cur = (0.0_f64, 0.0_f64);
        let mut start_pt = (0.0_f64, 0.0_f64);
        let mut active_cmd: Option<u8> = None;

        while i < len {
            skip_sep(&mut i);
            if i >= len {
                break;
            }
            if bytes[i].is_ascii_alphabetic() {
                let cmd = bytes[i];
                i += 1;
                if cmd == b'Z' || cmd == b'z' {
                    if (cur.0 - start_pt.0).abs() > 1e-6 || (cur.1 - start_pt.1).abs() > 1e-6 {
                        cmds.push(DensePathCmd::Line(start_pt.0, start_pt.1));
                        cur = start_pt;
                    }
                    active_cmd = None;
                } else {
                    active_cmd = Some(cmd);
                }
                continue;
            }
            let Some(cmd) = active_cmd else {
                break;
            };
            match cmd {
                b'M' | b'm' => {
                    let (Some(x), Some(y)) = (next_num(&mut i), next_num(&mut i)) else {
                        break;
                    };
                    let pt = if cmd == b'm' {
                        (cur.0 + x, cur.1 + y)
                    } else {
                        (x, y)
                    };
                    cmds.push(DensePathCmd::Move(pt.0, pt.1));
                    cur = pt;
                    start_pt = pt;
                    active_cmd = Some(if cmd == b'm' { b'l' } else { b'L' });
                }
                b'L' | b'l' => {
                    let (Some(x), Some(y)) = (next_num(&mut i), next_num(&mut i)) else {
                        break;
                    };
                    let pt = if cmd == b'l' {
                        (cur.0 + x, cur.1 + y)
                    } else {
                        (x, y)
                    };
                    cmds.push(DensePathCmd::Line(pt.0, pt.1));
                    cur = pt;
                }
                b'H' | b'h' => {
                    let Some(x) = next_num(&mut i) else {
                        break;
                    };
                    let nx = if cmd == b'h' { cur.0 + x } else { x };
                    cur = (nx, cur.1);
                    cmds.push(DensePathCmd::Line(cur.0, cur.1));
                }
                b'V' | b'v' => {
                    let Some(y) = next_num(&mut i) else {
                        break;
                    };
                    let ny = if cmd == b'v' { cur.1 + y } else { y };
                    cur = (cur.0, ny);
                    cmds.push(DensePathCmd::Line(cur.0, cur.1));
                }
                b'C' | b'c' => {
                    let (Some(x1), Some(y1), Some(x2), Some(y2), Some(x), Some(y)) = (
                        next_num(&mut i),
                        next_num(&mut i),
                        next_num(&mut i),
                        next_num(&mut i),
                        next_num(&mut i),
                        next_num(&mut i),
                    ) else {
                        break;
                    };
                    let (c1, c2, to) = if cmd == b'c' {
                        (
                            (cur.0 + x1, cur.1 + y1),
                            (cur.0 + x2, cur.1 + y2),
                            (cur.0 + x, cur.1 + y),
                        )
                    } else {
                        ((x1, y1), (x2, y2), (x, y))
                    };
                    cmds.push(DensePathCmd::Curve { c1, c2, to });
                    cur = to;
                }
                _ => {
                    let _ = next_num(&mut i);
                }
            }
        }
        cmds
    }

    fn sample_cubic_into(
        p0: (f64, f64),
        c1: (f64, f64),
        c2: (f64, f64),
        p3: (f64, f64),
        out: &mut Vec<(f64, f64)>,
    ) {
        let poly_len = (c1.0 - p0.0).hypot(c1.1 - p0.1)
            + (c2.0 - c1.0).hypot(c2.1 - c1.1)
            + (p3.0 - c2.0).hypot(p3.1 - c2.1);
        let steps = ((poly_len / 12.0).ceil() as usize).clamp(8, 32);
        for step in 1..=steps {
            let t = step as f64 / steps as f64;
            let mt = 1.0 - t;
            let b0 = mt * mt * mt;
            let b1 = 3.0 * mt * mt * t;
            let b2 = 3.0 * mt * t * t;
            let b3 = t * t * t;
            out.push((
                b0 * p0.0 + b1 * c1.0 + b2 * c2.0 + b3 * p3.0,
                b0 * p0.1 + b1 * c1.1 + b2 * c2.1 + b3 * p3.1,
            ));
        }
    }

    /// Samples a sequence of `DensePathCmd`s into an ordered polyline along the actual curves.
    pub fn sample_cmds_polyline(cmds: &[DensePathCmd]) -> Vec<(f64, f64)> {
        let mut pts = Vec::new();
        let mut cur = (0.0_f64, 0.0_f64);
        for cmd in cmds {
            match *cmd {
                DensePathCmd::Move(x, y) => {
                    cur = (x, y);
                    if pts.last().copied() != Some(cur) {
                        pts.push(cur);
                    }
                }
                DensePathCmd::Line(x, y) => {
                    cur = (x, y);
                    pts.push(cur);
                }
                DensePathCmd::Curve { c1, c2, to } => {
                    if pts.is_empty() {
                        pts.push(cur);
                    }
                    sample_cubic_into(cur, c1, c2, to, &mut pts);
                    cur = to;
                }
            }
        }
        pts
    }

    fn cmds_bounds(cmds: &[DensePathCmd]) -> Option<(f64, f64, f64, f64)> {
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        let mut push = |px: f64, py: f64| {
            min_x = min_x.min(px);
            min_y = min_y.min(py);
            max_x = max_x.max(px);
            max_y = max_y.max(py);
        };
        for cmd in cmds {
            match *cmd {
                DensePathCmd::Move(x, y) | DensePathCmd::Line(x, y) => push(x, y),
                DensePathCmd::Curve { c1, c2, to } => {
                    push(c1.0, c1.1);
                    push(c2.0, c2.1);
                    push(to.0, to.1);
                }
            }
        }
        min_x.is_finite().then_some((min_x, min_y, max_x, max_y))
    }

    fn pts_bounds(pts: &[(f64, f64)]) -> Option<(f64, f64, f64, f64)> {
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for &(px, py) in pts {
            min_x = min_x.min(px);
            min_y = min_y.min(py);
            max_x = max_x.max(px);
            max_y = max_y.max(py);
        }
        min_x.is_finite().then_some((min_x, min_y, max_x, max_y))
    }

    fn edge_bounds(edge: &DenseEdgeSpec) -> Option<(f64, f64, f64, f64)> {
        if !edge.cmds.is_empty() {
            cmds_bounds(&edge.cmds)
        } else if edge.pts.len() >= 2 {
            pts_bounds(&edge.pts)
        } else {
            None
        }
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct DenseScene {
        pub width: f64,
        pub height: f64,
        pub nodes: Vec<DenseNodeSpec>,
        pub edges: Vec<DenseEdgeSpec>,
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct DenseEdgeMeta {
        pub explain_kind: String,
        pub explain_payload: Value,
        pub from_index: Option<usize>,
        pub to_index: Option<usize>,
        pub summary: String,
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct DenseNodeMeta {
        pub explain_kind: String,
        pub explain_payload: Value,
        pub symbol_kind: String,
        pub visibility: String,
        pub in_cycle: bool,
        pub blast_tag: String,
        pub badge_text: String,
        pub tooltip: String,
        pub file_line: String,
    }

    pub const CLUSTER_HULL_MAX_SCALE: f64 = 0.35;

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum DenseContainerKind {
        #[default]
        TierBand,
        ServiceFrame,
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct DenseContainerSpec {
        pub kind: DenseContainerKind,
        pub x: f64,
        pub y: f64,
        pub w: f64,
        pub h: f64,
        pub radius: f64,
        pub label: String,
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct DenseClusterHull {
        pub key: String,
        pub label: String,
        pub color_slot: usize,
        pub node_count: usize,
        pub polygon: Vec<(f64, f64)>,
        pub centroid: (f64, f64),
        pub bounds: (f64, f64, f64, f64),
    }

    #[allow(dead_code)]
    pub fn should_render_cluster_hulls(scale: f64) -> bool {
        scale.is_finite() && scale > 0.0 && scale < CLUSTER_HULL_MAX_SCALE
    }

    #[allow(dead_code)]
    pub fn should_paint_cluster_hull_labels(overlay_only: bool, show_cluster_hulls: bool) -> bool {
        !overlay_only && show_cluster_hulls
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct DenseSceneMeta {
        pub nodes: HashMap<usize, DenseNodeMeta>,
        pub edges: HashMap<usize, DenseEdgeMeta>,
        pub containers: Vec<DenseContainerSpec>,
        pub cluster_hulls: Vec<DenseClusterHull>,
        pub kind_hover: Option<String>,
        pub hidden_kinds: HashSet<String>,
        pub label_min_scale: Option<f64>,
        pub commit_changes_active: bool,
        pub active_layer: Option<String>,
        pub layer_of_node: HashMap<usize, String>,
        pub node_pos_by_idx: HashMap<usize, usize>,
        pub edge_pos_by_idx: HashMap<usize, usize>,
        pub edge_bounds: HashMap<usize, (f64, f64, f64, f64)>,
        pub node_adjacency: HashMap<usize, Vec<(usize, usize)>>,
    }

    impl DenseSceneMeta {
        pub fn rebuild_caches(&mut self, scene: &DenseScene) {
            self.hidden_kinds = self
                .hidden_kinds
                .drain()
                .map(|k| k.trim().to_ascii_lowercase())
                .filter(|k| !k.is_empty())
                .collect();
            self.node_pos_by_idx.clear();
            self.node_pos_by_idx.reserve(scene.nodes.len());
            for (pos, node) in scene.nodes.iter().enumerate() {
                self.node_pos_by_idx.insert(node.index, pos);
            }
            self.edge_pos_by_idx.clear();
            self.edge_pos_by_idx.reserve(scene.edges.len());
            self.edge_bounds.clear();
            self.edge_bounds.reserve(scene.edges.len());
            for (pos, edge) in scene.edges.iter().enumerate() {
                self.edge_pos_by_idx.insert(edge.index, pos);
                if let Some(b) = edge_bounds(edge) {
                    self.edge_bounds.insert(edge.index, b);
                }
            }
            self.node_adjacency.clear();
            for (&edge_idx, em) in &self.edges {
                match (em.from_index, em.to_index) {
                    (Some(from), Some(to)) => {
                        self.node_adjacency
                            .entry(from)
                            .or_default()
                            .push((edge_idx, to));
                        if to != from {
                            self.node_adjacency
                                .entry(to)
                                .or_default()
                                .push((edge_idx, from));
                        }
                    }
                    (Some(from), None) => {
                        self.node_adjacency
                            .entry(from)
                            .or_default()
                            .push((edge_idx, from));
                    }
                    (None, Some(to)) => {
                        self.node_adjacency
                            .entry(to)
                            .or_default()
                            .push((edge_idx, to));
                    }
                    (None, None) => {}
                }
            }
            self.cluster_hulls = if self.containers.is_empty() {
                compute_dense_cluster_hulls(scene, self)
            } else {
                Vec::new()
            };
        }

        pub fn update_edge_bounds(&mut self, scene: &DenseScene, edge_indices: &[usize]) {
            if edge_indices.is_empty() {
                self.rebuild_caches(scene);
                return;
            }
            for &edge_idx in edge_indices {
                let edge_opt = scene
                    .edges
                    .get(edge_idx)
                    .filter(|e| e.index == edge_idx)
                    .or_else(|| scene.edges.iter().find(|e| e.index == edge_idx));
                if let Some(edge) = edge_opt {
                    if let Some(b) = edge_bounds(edge) {
                        self.edge_bounds.insert(edge_idx, b);
                    } else {
                        self.edge_bounds.remove(&edge_idx);
                    }
                }
            }
        }
    }

    fn strip_path_prefix_and_line(raw: &str) -> String {
        let mut s = raw.trim().replace('\\', "/");
        for prefix in ["file:", "sym:", "module:", "layer:", "session:"] {
            if let Some(rest) = s.strip_prefix(prefix) {
                s = rest.to_owned();
                break;
            }
        }
        if let Some((head, tail)) = s.rsplit_once(':') {
            if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) {
                s = head.to_owned();
            }
        }
        s
    }

    fn node_path_segments(node: &DenseNodeSpec, nm: Option<&DenseNodeMeta>) -> Option<Vec<String>> {
        if let Some(meta) = nm {
            for candidate in [
                meta.explain_payload["module"].as_str(),
                meta.explain_payload["service"].as_str(),
            ]
            .into_iter()
            .flatten()
            {
                let trimmed = candidate.trim();
                if !trimmed.is_empty()
                    && trimmed != "__system__"
                    && !trimmed.starts_with("__unmatched__")
                {
                    let segs: Vec<String> = trimmed
                        .split(['/', ':'])
                        .map(str::trim)
                        .filter(|s| !s.is_empty() && *s != ".")
                        .map(str::to_owned)
                        .collect();
                    if !segs.is_empty() {
                        return Some(segs);
                    }
                }
            }
            for raw in [
                meta.explain_payload["file_path"].as_str(),
                meta.explain_payload["path"].as_str(),
                (!meta.file_line.trim().is_empty()).then_some(meta.file_line.as_str()),
                meta.explain_payload.as_str(),
                meta.explain_payload["node_id"].as_str(),
            ]
            .into_iter()
            .flatten()
            {
                let cleaned = strip_path_prefix_and_line(raw);
                if cleaned.is_empty() {
                    continue;
                }
                if cleaned.contains('/') {
                    let trailing_dir = cleaned.ends_with('/');
                    let mut parts: Vec<String> = cleaned
                        .split('/')
                        .map(str::trim)
                        .filter(|p| !p.is_empty() && *p != ".")
                        .map(str::to_owned)
                        .collect();
                    if !trailing_dir && parts.len() >= 2 {
                        parts.pop();
                    }
                    if !parts.is_empty() {
                        return Some(parts);
                    }
                } else if cleaned.contains("::") {
                    let mut parts: Vec<String> = cleaned
                        .split("::")
                        .map(str::trim)
                        .filter(|p| !p.is_empty())
                        .map(str::to_owned)
                        .collect();
                    if parts.len() >= 2 {
                        parts.pop();
                    }
                    if !parts.is_empty() {
                        return Some(parts);
                    }
                }
            }
        }
        let cleaned = strip_path_prefix_and_line(&node.label);
        if cleaned.contains('/') {
            let mut parts: Vec<String> = cleaned
                .split('/')
                .map(str::trim)
                .filter(|p| !p.is_empty() && *p != ".")
                .map(str::to_owned)
                .collect();
            if parts.len() >= 2 {
                parts.pop();
            }
            if !parts.is_empty() {
                return Some(parts);
            }
        }
        None
    }

    fn count_multi_node_clusters<'a>(keys: impl Iterator<Item = Option<&'a str>>) -> usize {
        let mut counts: HashMap<&'a str, usize> = HashMap::new();
        for key in keys.flatten() {
            *counts.entry(key).or_insert(0) += 1;
        }
        counts.values().filter(|&&c| c >= 2).count()
    }

    fn pick_path_cluster_keys(
        scene: &DenseScene,
        meta: &DenseSceneMeta,
    ) -> Option<Vec<Option<(String, String)>>> {
        let segs_per_node: Vec<Option<Vec<String>>> = scene
            .nodes
            .iter()
            .map(|n| node_path_segments(n, meta.nodes.get(&n.index)))
            .collect();
        if segs_per_node.iter().filter(|s| s.is_some()).count() < 2 {
            return None;
        }
        let make_keys = |depth: usize| -> Vec<Option<String>> {
            segs_per_node
                .iter()
                .map(|opt| {
                    opt.as_ref().and_then(|segs| {
                        let take_n = segs.len().min(depth);
                        (take_n > 0).then(|| segs[..take_n].join("/"))
                    })
                })
                .collect()
        };
        let keys_d2 = make_keys(2);
        let keys_d1 = make_keys(1);
        let multi_d2 = count_multi_node_clusters(keys_d2.iter().map(|k| k.as_deref()));
        let multi_d1 = count_multi_node_clusters(keys_d1.iter().map(|k| k.as_deref()));
        let chosen = if multi_d2 >= 2 && (multi_d2 <= 28 || multi_d1 < 2) {
            keys_d2
        } else if multi_d1 >= 2 {
            keys_d1
        } else if multi_d2 >= 2 {
            keys_d2
        } else {
            return None;
        };
        Some(
            chosen
                .into_iter()
                .map(|opt| opt.map(|k| (format!("dir:{k}"), k)))
                .collect(),
        )
    }

    fn community_label_propagation(
        scene: &DenseScene,
        meta: &DenseSceneMeta,
    ) -> Vec<Option<(String, String)>> {
        let n = scene.nodes.len();
        if n < 2 || meta.node_adjacency.is_empty() {
            return vec![None; n];
        }
        let mut labels: Vec<usize> = scene.nodes.iter().map(|node| node.index).collect();
        for _ in 0..10 {
            let mut changed = false;
            for pos in 0..n {
                let node_idx = scene.nodes[pos].index;
                let Some(adj) = meta.node_adjacency.get(&node_idx) else {
                    continue;
                };
                let mut freq: HashMap<usize, usize> = HashMap::new();
                for &(_, neighbor_idx) in adj {
                    if neighbor_idx == node_idx {
                        continue;
                    }
                    if let Some(&npos) = meta.node_pos_by_idx.get(&neighbor_idx) {
                        *freq.entry(labels[npos]).or_insert(0) += 1;
                    }
                }
                if let Some((&best_comm, _)) = freq
                    .iter()
                    .max_by(|(ca, va), (cb, vb)| va.cmp(vb).then_with(|| cb.cmp(ca)))
                {
                    if best_comm != labels[pos] {
                        labels[pos] = best_comm;
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        let mut counts: HashMap<usize, usize> = HashMap::new();
        for &cid in &labels {
            *counts.entry(cid).or_insert(0) += 1;
        }
        let mut multi_cids: Vec<usize> = counts
            .into_iter()
            .filter_map(|(cid, count)| (count >= 2).then_some(cid))
            .collect();
        multi_cids.sort_unstable();
        let ordinal_by_cid: HashMap<usize, usize> = multi_cids
            .into_iter()
            .enumerate()
            .map(|(i, cid)| (cid, i + 1))
            .collect();
        labels
            .into_iter()
            .map(|cid| {
                ordinal_by_cid
                    .get(&cid)
                    .map(|ord| (format!("comm:{cid}"), format!("Cluster {ord}")))
            })
            .collect()
    }

    fn convex_hull(mut pts: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
        pts.retain(|p| p.0.is_finite() && p.1.is_finite());
        pts.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));
        pts.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6);
        if pts.len() <= 3 {
            return pts;
        }
        let cross = |o: (f64, f64), a: (f64, f64), b: (f64, f64)| -> f64 {
            (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
        };
        let mut lower: Vec<(f64, f64)> = Vec::new();
        for &p in &pts {
            while lower.len() >= 2
                && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0
            {
                lower.pop();
            }
            lower.push(p);
        }
        let mut upper: Vec<(f64, f64)> = Vec::new();
        for &p in pts.iter().rev() {
            while upper.len() >= 2
                && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0
            {
                upper.pop();
            }
            upper.push(p);
        }
        lower.pop();
        upper.pop();
        lower.extend(upper);
        lower
    }

    pub fn compute_dense_cluster_hulls(
        scene: &DenseScene,
        meta: &DenseSceneMeta,
    ) -> Vec<DenseClusterHull> {
        if scene.nodes.len() < 2 {
            return Vec::new();
        }
        // 1. Prefer explicit architecture layers when multiple layers group nodes cleanly.
        let mut layer_assign: Vec<Option<(String, String)>> = Vec::with_capacity(scene.nodes.len());
        let mut multi_layer_counts: HashMap<&str, usize> = HashMap::new();
        for node in &scene.nodes {
            if let Some(layer) = meta
                .layer_of_node
                .get(&node.index)
                .map(String::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                *multi_layer_counts.entry(layer).or_insert(0) += 1;
                layer_assign.push(Some((format!("layer:{layer}"), layer.to_owned())));
            } else {
                layer_assign.push(None);
            }
        }
        let multi_layers = multi_layer_counts.values().filter(|&&c| c >= 2).count();
        let assignments = if multi_layers >= 2 {
            layer_assign
        } else if let Some(path_assign) = pick_path_cluster_keys(scene, meta) {
            path_assign
        } else {
            let comm_assign = community_label_propagation(scene, meta);
            if comm_assign.iter().any(Option::is_some) {
                comm_assign
            } else {
                let mut role_counts: HashMap<&str, usize> = HashMap::new();
                for node in &scene.nodes {
                    let r = node.role.trim();
                    if !r.is_empty() {
                        *role_counts.entry(r).or_insert(0) += 1;
                    }
                }
                if role_counts.values().filter(|&&c| c >= 2).count() >= 2 {
                    scene
                        .nodes
                        .iter()
                        .map(|n| {
                            let r = n.role.trim();
                            (!r.is_empty()).then(|| (format!("role:{r}"), r.to_owned()))
                        })
                        .collect()
                } else {
                    return Vec::new();
                }
            }
        };

        let mut grouped: HashMap<String, (String, Vec<&DenseNodeSpec>)> = HashMap::new();
        for (node, assign) in scene.nodes.iter().zip(assignments) {
            let Some((key, label)) = assign else {
                continue;
            };
            let entry = grouped.entry(key).or_insert_with(|| (label, Vec::new()));
            entry.1.push(node);
        }

        const HULL_PAD: f64 = 18.0;
        let mut hulls: Vec<DenseClusterHull> = Vec::new();
        for (key, (label, members)) in grouped {
            if members.len() < 2 {
                continue;
            }
            let mut pts = Vec::with_capacity(members.len() * 4);
            let mut sum_cx = 0.0;
            let mut sum_cy = 0.0;
            let mut min_x = f64::INFINITY;
            let mut min_y = f64::INFINITY;
            let mut max_x = f64::NEG_INFINITY;
            let mut max_y = f64::NEG_INFINITY;
            for node in &members {
                let x0 = node.x - HULL_PAD;
                let y0 = node.y - HULL_PAD;
                let x1 = node.x + node.w.max(10.0) + HULL_PAD;
                let y1 = node.y + node.h.max(10.0) + HULL_PAD;
                pts.push((x0, y0));
                pts.push((x1, y0));
                pts.push((x1, y1));
                pts.push((x0, y1));
                sum_cx += node.x + node.w * 0.5;
                sum_cy += node.y + node.h * 0.5;
                min_x = min_x.min(x0);
                min_y = min_y.min(y0);
                max_x = max_x.max(x1);
                max_y = max_y.max(y1);
            }
            let polygon = convex_hull(pts);
            if polygon.len() < 3 {
                continue;
            }
            let denom = members.len() as f64;
            hulls.push(DenseClusterHull {
                key,
                label,
                color_slot: 0,
                node_count: members.len(),
                polygon,
                centroid: (sum_cx / denom, sum_cy / denom),
                bounds: (min_x, min_y, max_x, max_y),
            });
        }
        hulls.sort_by(|a, b| {
            b.node_count
                .cmp(&a.node_count)
                .then_with(|| a.key.cmp(&b.key))
        });
        for (idx, hull) in hulls.iter_mut().enumerate() {
            hull.color_slot = idx % 6;
        }
        hulls
    }

    #[derive(Clone, Debug, PartialEq)]
    pub struct PlacedClusterLabel {
        pub label: String,
        pub color_slot: usize,
        pub x: f64,
        pub y: f64,
        pub w: f64,
        pub h: f64,
    }

    /// Computes non-overlapping screen-space pill labels for cluster hulls at overview zoom.
    #[allow(dead_code)]
    pub fn layout_cluster_hull_labels<M>(
        hulls: &[DenseClusterHull],
        cam: Camera,
        vw: f64,
        vh: f64,
        mut measure_w: M,
    ) -> Vec<PlacedClusterLabel>
    where
        M: FnMut(&str) -> f64,
    {
        if !should_render_cluster_hulls(cam.scale) || hulls.is_empty() {
            return Vec::new();
        }
        let mut placed: Vec<PlacedClusterLabel> = Vec::new();
        for hull in hulls {
            let hull_screen_w = (hull.bounds.2 - hull.bounds.0) * cam.scale;
            let hull_screen_h = (hull.bounds.3 - hull.bounds.1) * cam.scale;
            if hull_screen_w.max(hull_screen_h) < 22.0 {
                continue;
            }
            let sx = cam.tx + hull.centroid.0 * cam.scale;
            let sy = cam.ty + hull.centroid.1 * cam.scale;
            let short = truncate_label_chars(&hull.label, 24);
            let pill_text = format!("{short} · {}", hull.node_count);
            let text_w = measure_w(&pill_text).max(20.0);
            let pw = (text_w + 14.0).clamp(38.0, 220.0);
            let ph = 20.0;
            let px = (sx - pw * 0.5).clamp(6.0, (vw - pw - 6.0).max(6.0));
            let py = (sy - ph * 0.5).clamp(6.0, (vh - ph - 6.0).max(6.0));
            if sx + pw < 0.0 || sx - pw > vw || sy + ph < 0.0 || sy - ph > vh {
                continue;
            }
            let pad = 6.0;
            let overlaps = placed.iter().any(|existing| {
                px < existing.x + existing.w + pad
                    && px + pw + pad > existing.x
                    && py < existing.y + existing.h + pad
                    && py + ph + pad > existing.y
            });
            if overlaps {
                continue;
            }
            placed.push(PlacedClusterLabel {
                label: pill_text,
                color_slot: hull.color_slot,
                x: px,
                y: py,
                w: pw,
                h: ph,
            });
        }
        placed
    }

    #[allow(dead_code)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum EdgeDashKind {
        Solid,
        Dashed,
        Dotted,
        DashDot,
    }

    #[allow(dead_code)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum EdgeColorSlot {
        Base,
        Calls,
        Imports,
        Extends,
        Implements,
        Writes,
        Reads,
        Uses,
        Http,
        Event,
        Rpc,
        Graphql,
        Shared,
        Code,
        Against,
    }

    #[allow(dead_code)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct EdgeVisualStyle {
        pub color: EdgeColorSlot,
        pub dash: EdgeDashKind,
        pub hollow_arrow: bool,
    }

    pub fn edge_kind_matches_lower(want_lower: &str, edge_kind: &str) -> bool {
        if want_lower.is_empty() {
            return false;
        }
        edge_kind
            .split(|c: char| c.is_whitespace() || c == ',' || c == '|')
            .map(str::trim)
            .any(|tok| !tok.is_empty() && tok.eq_ignore_ascii_case(want_lower))
    }

    pub fn edge_matches_kind_lower(want_lower: &str, edge: &DenseEdgeSpec) -> bool {
        edge_kind_matches_lower(want_lower, &edge.kind)
            || edge
                .kinds
                .iter()
                .any(|extra| edge_kind_matches_lower(want_lower, extra))
    }

    #[cfg(test)]
    pub fn edge_kind_matches(filter_kind: &str, edge_kind: &str) -> bool {
        let want = filter_kind.trim().to_ascii_lowercase();
        edge_kind_matches_lower(&want, edge_kind)
    }

    #[cfg(test)]
    pub fn edge_matches_kind(filter_kind: &str, edge: &DenseEdgeSpec) -> bool {
        let want = filter_kind.trim().to_ascii_lowercase();
        edge_matches_kind_lower(&want, edge)
    }

    #[allow(dead_code)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum DenseEdgeTone {
        Base(EdgeColorSlot),
        Against,
        DiffRemoved,
        DiffAdded,
        DiffModified,
        ImpactRisk,
        CommitTouched,
        BackEdge,
        Selection,
    }

    #[allow(dead_code)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct DenseEdgeAppearance {
        pub tone: DenseEdgeTone,
        pub dash: EdgeDashKind,
        pub hollow_arrow: bool,
        pub highlighted: bool,
    }

    /// Resolves semantic edge stroke tone and dash style, preserving `against` and
    /// diff/impact/risk tones over selection highlight (`purple`).
    #[allow(dead_code)]
    pub fn resolve_dense_edge_stroke(edge: &DenseEdgeSpec, hovered: bool) -> DenseEdgeAppearance {
        let base = edge_visual_style(&edge.kind, edge.guessed, edge.against);
        let tone = if edge.against {
            DenseEdgeTone::Against
        } else if edge.diff == "removed" {
            DenseEdgeTone::DiffRemoved
        } else if edge.impact == "broken_risk" {
            DenseEdgeTone::ImpactRisk
        } else if edge.diff == "added" {
            DenseEdgeTone::DiffAdded
        } else if edge.diff == "changed" || edge.diff == "modified" {
            DenseEdgeTone::DiffModified
        } else if edge.commit_touched {
            DenseEdgeTone::CommitTouched
        } else if hovered || edge.on || edge.lit {
            DenseEdgeTone::Selection
        } else if edge.back {
            DenseEdgeTone::BackEdge
        } else {
            DenseEdgeTone::Base(base.color)
        };
        let dash = if edge.against || edge.diff == "removed" || edge.impact == "broken_risk" {
            EdgeDashKind::Dashed
        } else if edge.back {
            EdgeDashKind::Dotted
        } else if edge.diff == "added" {
            EdgeDashKind::Solid
        } else {
            base.dash
        };
        let highlighted = hovered
            || edge.on
            || edge.lit
            || edge.diff == "added"
            || edge.diff == "removed"
            || edge.diff == "changed"
            || edge.diff == "modified"
            || edge.impact == "broken_risk"
            || edge.commit_touched;
        DenseEdgeAppearance {
            tone,
            dash,
            hollow_arrow: base.hollow_arrow,
            highlighted,
        }
    }

    #[allow(dead_code)]
    pub fn truncate_label_chars(label: &str, max_chars: usize) -> String {
        if label.chars().count() <= max_chars {
            return label.to_owned();
        }
        if max_chars <= 1 {
            return "...".to_owned();
        }
        let prefix: String = label.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{prefix}…")
    }

    /// Truncates `label` with `…` at UTF-8 char boundaries using `measure` so labels never squish horizontally.
    #[allow(dead_code)]
    pub fn fit_text_with_ellipsis<M>(label: &str, max_w: f64, mut measure: M) -> String
    where
        M: FnMut(&str) -> f64,
    {
        if label.is_empty() || max_w <= 6.0 || measure(label) <= max_w {
            return label.to_owned();
        }
        let boundaries: Vec<usize> = label.char_indices().map(|(i, _)| i).collect();
        if boundaries.is_empty() {
            return String::new();
        }
        let mut lo = 0usize;
        let mut hi = boundaries.len();
        let mut best = String::from("…");
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let candidate = if mid < boundaries.len() {
                format!("{}…", &label[..boundaries[mid]])
            } else {
                label.to_owned()
            };
            if measure(&candidate) <= max_w {
                best = candidate;
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        best
    }

    #[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
    pub(crate) type DenseEdgeBatchKey = (bool, DenseEdgeTone, EdgeDashKind, u16, u8);

    #[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
    pub fn dense_edge_batch_key(
        active: bool,
        appearance: &DenseEdgeAppearance,
        width: f64,
        alpha: f64,
    ) -> DenseEdgeBatchKey {
        (
            active,
            appearance.tone,
            appearance.dash,
            (width.clamp(0.1, 20.0) * 20.0).round() as u16,
            (alpha.clamp(0.0, 1.0) * 100.0).round() as u8,
        )
    }

    thread_local! {
        static LABEL_FIT_CACHE: RefCell<HashMap<(String, i32, &'static str), String>> =
            RefCell::new(HashMap::new());
        static BADGE_WIDTH_CACHE: RefCell<HashMap<String, f64>> =
            RefCell::new(HashMap::new());
    }

    #[allow(dead_code)]
    pub fn fit_label_cached_with<M>(
        label: &str,
        max_w: f64,
        font: &'static str,
        measure: M,
    ) -> String
    where
        M: FnMut(&str) -> f64,
    {
        if label.is_empty() || max_w <= 6.0 {
            return if label.is_empty() {
                String::new()
            } else {
                label.to_owned()
            };
        }
        let w_key = (max_w * 2.0).round() as i32;
        if let Some(hit) =
            LABEL_FIT_CACHE.with(|c| c.borrow().get(&(label.to_owned(), w_key, font)).cloned())
        {
            return hit;
        }
        let fitted = fit_text_with_ellipsis(label, max_w, measure);
        LABEL_FIT_CACHE.with(|c| {
            let mut map = c.borrow_mut();
            if map.len() >= 1024 {
                map.clear();
            }
            map.insert((label.to_owned(), w_key, font), fitted.clone());
        });
        fitted
    }

    #[allow(dead_code)]
    pub fn badge_width_cached_with<M>(badge: &str, mut measure: M) -> f64
    where
        M: FnMut(&str) -> f64,
    {
        if let Some(w) = BADGE_WIDTH_CACHE.with(|c| c.borrow().get(badge).copied()) {
            return w;
        }
        let w = measure(badge).ceil();
        BADGE_WIDTH_CACHE.with(|c| {
            let mut map = c.borrow_mut();
            if map.len() >= 256 {
                map.clear();
            }
            map.insert(badge.to_owned(), w);
        });
        w
    }

    #[allow(dead_code)]
    pub fn hit_test_tooltip_text(canvas_id: &str, kind: u8, index: usize) -> Option<String> {
        DENSE_SCENES.with(|s| {
            let guard = s.borrow();
            let (scene, _, meta) = guard.get(canvas_id)?;
            match kind {
                0 => {
                    if let Some(nm) = meta.nodes.get(&index) {
                        if !nm.tooltip.is_empty() {
                            return Some(nm.tooltip.clone());
                        }
                        let base = scene
                            .nodes
                            .get(index)
                            .filter(|n| n.index == index)
                            .or_else(|| scene.nodes.iter().find(|n| n.index == index))
                            .map(|n| n.label.as_str())
                            .unwrap_or("");
                        let mut parts = Vec::new();
                        if !base.is_empty() {
                            parts.push(base.to_owned());
                        }
                        if !nm.symbol_kind.is_empty() {
                            parts.push(nm.symbol_kind.clone());
                        }
                        if !nm.visibility.is_empty() {
                            parts.push(nm.visibility.clone());
                        }
                        if nm.in_cycle {
                            parts.push("cycle".to_owned());
                        }
                        if !nm.blast_tag.is_empty() {
                            parts.push(nm.blast_tag.clone());
                        }
                        if !nm.file_line.is_empty() {
                            parts.push(nm.file_line.clone());
                        }
                        if !parts.is_empty() {
                            return Some(parts.join(" · "));
                        }
                    }
                    scene
                        .nodes
                        .iter()
                        .find(|n| n.index == index && !n.label.is_empty())
                        .map(|n| n.label.clone())
                }
                1 => {
                    if let Some(em) = meta.edges.get(&index) {
                        if !em.summary.is_empty() {
                            return Some(em.summary.clone());
                        }
                    }
                    scene
                        .edges
                        .iter()
                        .find(|e| e.index == index)
                        .map(|e| format!("{} ({})", e.kind, e.weight.max(1)))
                }
                _ => None,
            }
        })
    }

    #[allow(dead_code)]
    pub fn edge_visual_style(kind: &str, guessed: bool, against: bool) -> EdgeVisualStyle {
        let primary = kind
            .split(|c: char| c.is_whitespace() || c == ',' || c == '|')
            .find(|s| !s.trim().is_empty())
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let (color, mut dash, hollow_arrow) = match primary.as_str() {
            "calls" | "call" => (EdgeColorSlot::Calls, EdgeDashKind::Solid, false),
            "imports" | "import" | "include" => {
                (EdgeColorSlot::Imports, EdgeDashKind::Dashed, false)
            }
            "extends" | "extend" | "inherit" => (EdgeColorSlot::Extends, EdgeDashKind::Solid, true),
            "implements" | "implement" | "trait" => {
                (EdgeColorSlot::Implements, EdgeDashKind::DashDot, true)
            }
            "writes" | "write" => (EdgeColorSlot::Writes, EdgeDashKind::Solid, false),
            "reads" | "read" => (EdgeColorSlot::Reads, EdgeDashKind::Dotted, true),
            "uses" | "use" => (EdgeColorSlot::Uses, EdgeDashKind::Solid, false),
            "http" | "rest" => (EdgeColorSlot::Http, EdgeDashKind::Solid, false),
            "event" | "events" => (EdgeColorSlot::Event, EdgeDashKind::Dashed, false),
            "rpc" | "grpc" => (EdgeColorSlot::Rpc, EdgeDashKind::Solid, false),
            "graphql" | "gql" => (EdgeColorSlot::Graphql, EdgeDashKind::DashDot, false),
            "shared" => (EdgeColorSlot::Shared, EdgeDashKind::Dashed, false),
            "code" => (EdgeColorSlot::Code, EdgeDashKind::Solid, false),
            _ => (EdgeColorSlot::Base, EdgeDashKind::Solid, false),
        };
        if guessed && dash == EdgeDashKind::Solid {
            dash = EdgeDashKind::Dashed;
        }
        if against {
            return EdgeVisualStyle {
                color: EdgeColorSlot::Against,
                dash: EdgeDashKind::Dashed,
                hollow_arrow,
            };
        }
        EdgeVisualStyle {
            color,
            dash,
            hollow_arrow,
        }
    }

    #[allow(dead_code)]
    pub fn should_show_dense_labels(scale: f64, label_min_scale: Option<f64>) -> bool {
        let min_scale = label_min_scale
            .filter(|s| s.is_finite() && *s > 0.0)
            .unwrap_or(DEFAULT_LOD.far_below);
        scale.is_finite() && scale >= min_scale
    }

    #[allow(dead_code)]
    pub fn edge_end_tangent(edge: &DenseEdgeSpec) -> Option<((f64, f64), (f64, f64))> {
        let unit = |from: (f64, f64), to: (f64, f64)| -> Option<(f64, f64)> {
            let dx = to.0 - from.0;
            let dy = to.1 - from.1;
            let len = (dx * dx + dy * dy).sqrt();
            (len > 1e-4).then(|| (dx / len, dy / len))
        };
        if !edge.cmds.is_empty() {
            let mut cur = (0.0, 0.0);
            let mut tip = None;
            let mut prev_dir = None;
            for cmd in &edge.cmds {
                match *cmd {
                    DensePathCmd::Move(x, y) => {
                        cur = (x, y);
                    }
                    DensePathCmd::Line(x, y) => {
                        let to = (x, y);
                        if let Some(u) = unit(cur, to) {
                            prev_dir = Some((to, u));
                        }
                        tip = Some(to);
                        cur = to;
                    }
                    DensePathCmd::Curve { c1, c2, to } => {
                        let u = unit(c2, to)
                            .or_else(|| unit(c1, to))
                            .or_else(|| unit(cur, to));
                        if let Some(u) = u {
                            prev_dir = Some((to, u));
                        }
                        tip = Some(to);
                        cur = to;
                    }
                }
            }
            if let (Some(_), Some(res)) = (tip, prev_dir) {
                return Some(res);
            }
        }
        if edge.pts.len() >= 2 {
            let last = *edge.pts.last()?;
            for &prev in edge.pts[..edge.pts.len() - 1].iter().rev() {
                if let Some(u) = unit(prev, last) {
                    return Some((last, u));
                }
            }
        }
        None
    }

    #[allow(dead_code)]
    pub fn edge_midpoint(edge: &DenseEdgeSpec) -> Option<(f64, f64)> {
        if !edge.pts.is_empty() {
            if edge.pts.len() == 4 && edge.cmds.is_empty() {
                let p0 = edge.pts[0];
                let p1 = edge.pts[1];
                let p2 = edge.pts[2];
                let p3 = edge.pts[3];
                let mx = 0.125 * p0.0 + 0.375 * p1.0 + 0.375 * p2.0 + 0.125 * p3.0;
                let my = 0.125 * p0.1 + 0.375 * p1.1 + 0.375 * p2.1 + 0.125 * p3.1;
                return Some((mx, my));
            }
            return Some(edge.pts[edge.pts.len() / 2]);
        }
        if !edge.cmds.is_empty() {
            let sampled = sample_cmds_polyline(&edge.cmds);
            if !sampled.is_empty() {
                return Some(sampled[sampled.len() / 2]);
            }
        }
        None
    }

    fn sample_edge_polyline(edge: &DenseEdgeSpec) -> Vec<(f64, f64)> {
        if !edge.cmds.is_empty() {
            sample_cmds_polyline(&edge.cmds)
        } else if edge.pts.len() == 4 {
            sample_cmds_polyline(&[
                DensePathCmd::Move(edge.pts[0].0, edge.pts[0].1),
                DensePathCmd::Curve {
                    c1: edge.pts[1],
                    c2: edge.pts[2],
                    to: edge.pts[3],
                },
            ])
        } else {
            edge.pts.clone()
        }
    }

    fn node_touched_cells(x: f64, y: f64, w: f64, h: f64, cell: f64) -> Vec<(i32, i32)> {
        let x0 = ((x - NODE_HIT_PAD) / cell).floor() as i32;
        let x1 = ((x + w + NODE_HIT_PAD) / cell).floor() as i32;
        let y0 = ((y - NODE_HIT_PAD) / cell).floor() as i32;
        let y1 = ((y + h + NODE_HIT_PAD) / cell).floor() as i32;
        let mut keys = Vec::new();
        for gx in x0..=x1 {
            for gy in y0..=y1 {
                keys.push((gx, gy));
            }
        }
        keys
    }

    fn edge_touched_cells(sampled: &[(f64, f64)], cell: f64) -> Vec<(i32, i32)> {
        let mut touched: BTreeSet<(i32, i32)> = BTreeSet::new();
        for pair in sampled.windows(2) {
            let min_x = pair[0].0.min(pair[1].0);
            let max_x = pair[0].0.max(pair[1].0);
            let min_y = pair[0].1.min(pair[1].1);
            let max_y = pair[0].1.max(pair[1].1);
            let x0 = ((min_x - EDGE_HIT_PAD) / cell).floor() as i32;
            let x1 = ((max_x + EDGE_HIT_PAD) / cell).floor() as i32;
            let y0 = ((min_y - EDGE_HIT_PAD) / cell).floor() as i32;
            let y1 = ((max_y + EDGE_HIT_PAD) / cell).floor() as i32;
            for gx in x0..=x1 {
                for gy in y0..=y1 {
                    touched.insert((gx, gy));
                }
            }
        }
        touched.into_iter().collect()
    }

    fn remove_slot_from_cells(
        map: &mut HashMap<(i32, i32), Vec<usize>>,
        keys: &[(i32, i32)],
        slot: usize,
    ) {
        for key in keys {
            if let Some(bucket) = map.get_mut(key) {
                bucket.retain(|&s| s != slot);
                if bucket.is_empty() {
                    map.remove(key);
                }
            }
        }
    }

    #[derive(Clone, Debug, Default)]
    pub struct SpatialHitGrid {
        cell_size: f64,
        node_cells: HashMap<(i32, i32), Vec<usize>>,
        edge_cells: HashMap<(i32, i32), Vec<usize>>,
        node_rects: Vec<(usize, f64, f64, f64, f64)>,
        edge_polys: Vec<(usize, Vec<(f64, f64)>)>,
        node_slot_by_idx: HashMap<usize, usize>,
        edge_slot_by_idx: HashMap<usize, usize>,
        node_cells_for_slot: Vec<Vec<(i32, i32)>>,
        edge_cells_for_slot: Vec<Vec<(i32, i32)>>,
    }

    impl SpatialHitGrid {
        pub fn build(scene: &DenseScene, cell_size: f64) -> Self {
            let cell = cell_size.max(32.0);
            let mut node_cells: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
            let mut edge_cells: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
            let mut node_rects = Vec::with_capacity(scene.nodes.len());
            let mut edge_polys = Vec::with_capacity(scene.edges.len());
            let mut node_slot_by_idx = HashMap::with_capacity(scene.nodes.len());
            let mut edge_slot_by_idx = HashMap::with_capacity(scene.edges.len());
            let mut node_cells_for_slot = Vec::with_capacity(scene.nodes.len());
            let mut edge_cells_for_slot = Vec::with_capacity(scene.edges.len());

            for node in &scene.nodes {
                let slot = node_rects.len();
                node_rects.push((node.index, node.x, node.y, node.w, node.h));
                node_slot_by_idx.insert(node.index, slot);
                let keys = node_touched_cells(node.x, node.y, node.w, node.h, cell);
                for &key in &keys {
                    node_cells.entry(key).or_default().push(slot);
                }
                node_cells_for_slot.push(keys);
            }

            for edge in &scene.edges {
                let sampled = sample_edge_polyline(edge);
                if sampled.len() < 2 {
                    continue;
                }
                let slot = edge_polys.len();
                let keys = edge_touched_cells(&sampled, cell);
                edge_polys.push((edge.index, sampled));
                edge_slot_by_idx.insert(edge.index, slot);
                for &key in &keys {
                    edge_cells.entry(key).or_default().push(slot);
                }
                edge_cells_for_slot.push(keys);
            }

            Self {
                cell_size: cell,
                node_cells,
                edge_cells,
                node_rects,
                edge_polys,
                node_slot_by_idx,
                edge_slot_by_idx,
                node_cells_for_slot,
                edge_cells_for_slot,
            }
        }

        /// Incrementally updates `moved_nodes` and `moved_edges` after Shift+drag pinning
        /// without rebuilding unaffected spatial grid buckets.
        pub fn update_nodes_and_edges(
            &mut self,
            scene: &DenseScene,
            moved_nodes: &[usize],
            moved_edges: &[usize],
        ) {
            if (moved_nodes.is_empty() && moved_edges.is_empty())
                || (moved_nodes.len() + moved_edges.len()) * 4
                    > (scene.nodes.len() + scene.edges.len()).max(8)
            {
                *self = Self::build(scene, self.cell_size);
                return;
            }

            for &node_idx in moved_nodes {
                let Some(node) = scene
                    .nodes
                    .get(node_idx)
                    .filter(|n| n.index == node_idx)
                    .or_else(|| scene.nodes.iter().find(|n| n.index == node_idx))
                else {
                    continue;
                };
                if let Some(&slot) = self.node_slot_by_idx.get(&node_idx) {
                    remove_slot_from_cells(
                        &mut self.node_cells,
                        &self.node_cells_for_slot[slot],
                        slot,
                    );
                    self.node_rects[slot] = (node.index, node.x, node.y, node.w, node.h);
                    let keys = node_touched_cells(node.x, node.y, node.w, node.h, self.cell_size);
                    for &key in &keys {
                        self.node_cells.entry(key).or_default().push(slot);
                    }
                    self.node_cells_for_slot[slot] = keys;
                } else {
                    let slot = self.node_rects.len();
                    self.node_rects
                        .push((node.index, node.x, node.y, node.w, node.h));
                    self.node_slot_by_idx.insert(node.index, slot);
                    let keys = node_touched_cells(node.x, node.y, node.w, node.h, self.cell_size);
                    for &key in &keys {
                        self.node_cells.entry(key).or_default().push(slot);
                    }
                    self.node_cells_for_slot.push(keys);
                }
            }

            for &edge_idx in moved_edges {
                let Some(edge) = scene
                    .edges
                    .get(edge_idx)
                    .filter(|e| e.index == edge_idx)
                    .or_else(|| scene.edges.iter().find(|e| e.index == edge_idx))
                else {
                    continue;
                };
                let sampled = sample_edge_polyline(edge);
                if let Some(&slot) = self.edge_slot_by_idx.get(&edge_idx) {
                    remove_slot_from_cells(
                        &mut self.edge_cells,
                        &self.edge_cells_for_slot[slot],
                        slot,
                    );
                    if sampled.len() < 2 {
                        self.edge_polys[slot] = (edge.index, Vec::new());
                        self.edge_cells_for_slot[slot].clear();
                    } else {
                        let keys = edge_touched_cells(&sampled, self.cell_size);
                        self.edge_polys[slot] = (edge.index, sampled);
                        for &key in &keys {
                            self.edge_cells.entry(key).or_default().push(slot);
                        }
                        self.edge_cells_for_slot[slot] = keys;
                    }
                } else if sampled.len() >= 2 {
                    let slot = self.edge_polys.len();
                    let keys = edge_touched_cells(&sampled, self.cell_size);
                    self.edge_polys.push((edge.index, sampled));
                    self.edge_slot_by_idx.insert(edge.index, slot);
                    for &key in &keys {
                        self.edge_cells.entry(key).or_default().push(slot);
                    }
                    self.edge_cells_for_slot.push(keys);
                }
            }
        }

        #[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
        fn collect_visible_cell_indices(
            cell_size: f64,
            cells: &HashMap<(i32, i32), Vec<usize>>,
            slot_len: usize,
            index_at_slot: impl Fn(usize) -> usize,
            bounds: (f64, f64, f64, f64),
        ) -> Option<Vec<usize>> {
            const MAX_VIEWPORT_CELLS: usize = 4096;
            let (x0, y0, x1, y1) = bounds;
            if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
                return None;
            }
            let gx0 = (x0 / cell_size).floor() as i32;
            let gx1 = (x1 / cell_size).floor() as i32;
            let gy0 = (y0 / cell_size).floor() as i32;
            let gy1 = (y1 / cell_size).floor() as i32;
            if gx1 < gx0 || gy1 < gy0 {
                return Some(Vec::new());
            }
            let cols = (gx1 as i64 - gx0 as i64 + 1) as usize;
            let rows = (gy1 as i64 - gy0 as i64 + 1) as usize;
            if cols.saturating_mul(rows) > MAX_VIEWPORT_CELLS {
                return None;
            }
            let mut seen = vec![false; slot_len];
            let mut slots = Vec::new();
            for gx in gx0..=gx1 {
                for gy in gy0..=gy1 {
                    if let Some(bucket) = cells.get(&(gx, gy)) {
                        for &slot in bucket {
                            if slot < seen.len() && !seen[slot] {
                                seen[slot] = true;
                                slots.push(slot);
                            }
                        }
                    }
                }
            }
            slots.sort_unstable();
            Some(slots.into_iter().map(index_at_slot).collect())
        }

        #[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
        pub fn visible_node_indices(
            &self,
            x0: f64,
            y0: f64,
            x1: f64,
            y1: f64,
        ) -> Option<Vec<usize>> {
            Self::collect_visible_cell_indices(
                self.cell_size,
                &self.node_cells,
                self.node_rects.len(),
                |slot| self.node_rects[slot].0,
                (x0, y0, x1, y1),
            )
        }

        #[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
        pub fn visible_edge_indices(
            &self,
            x0: f64,
            y0: f64,
            x1: f64,
            y1: f64,
        ) -> Option<Vec<usize>> {
            Self::collect_visible_cell_indices(
                self.cell_size,
                &self.edge_cells,
                self.edge_polys.len(),
                |slot| self.edge_polys[slot].0,
                (x0, y0, x1, y1),
            )
        }

        pub fn hit_test(&self, world_x: f64, world_y: f64, tol: f64) -> Option<(u8, usize)> {
            self.hit_test_filtered(world_x, world_y, tol, |_| true)
        }

        pub fn hit_test_filtered<F>(
            &self,
            world_x: f64,
            world_y: f64,
            tol: f64,
            mut allow_edge: F,
        ) -> Option<(u8, usize)>
        where
            F: FnMut(usize) -> bool,
        {
            let gx = (world_x / self.cell_size).floor() as i32;
            let gy = (world_y / self.cell_size).floor() as i32;
            if let Some(slots) = self.node_cells.get(&(gx, gy)) {
                for &slot in slots.iter().rev() {
                    let (idx, x, y, w, h) = self.node_rects[slot];
                    if world_x >= x - NODE_HIT_PAD
                        && world_x <= x + w + NODE_HIT_PAD
                        && world_y >= y - NODE_HIT_PAD
                        && world_y <= y + h + NODE_HIT_PAD
                    {
                        return Some((0, idx));
                    }
                }
            }
            let tol_sq = tol * tol;
            let mut best_edge: Option<(usize, f64)> = None;
            let extra = (tol - EDGE_HIT_PAD).max(0.0);
            let ex0 = ((world_x - extra) / self.cell_size).floor() as i32;
            let ex1 = ((world_x + extra) / self.cell_size).floor() as i32;
            let ey0 = ((world_y - extra) / self.cell_size).floor() as i32;
            let ey1 = ((world_y + extra) / self.cell_size).floor() as i32;
            for cx in ex0..=ex1 {
                for cy in ey0..=ey1 {
                    if let Some(slots) = self.edge_cells.get(&(cx, cy)) {
                        for &slot in slots {
                            let (idx, ref pts) = self.edge_polys[slot];
                            if !allow_edge(idx) {
                                continue;
                            }
                            for pair in pts.windows(2) {
                                let d2 =
                                    point_to_segment_dist_sq(world_x, world_y, pair[0], pair[1]);
                                if d2 <= tol_sq && best_edge.is_none_or(|(_, prev)| d2 < prev) {
                                    best_edge = Some((idx, d2));
                                }
                            }
                        }
                    }
                }
            }
            best_edge.map(|(idx, _)| (1, idx))
        }
    }

    fn point_to_segment_dist_sq(px: f64, py: f64, a: (f64, f64), b: (f64, f64)) -> f64 {
        let vx = b.0 - a.0;
        let vy = b.1 - a.1;
        let len_sq = vx * vx + vy * vy;
        if len_sq <= 1e-6 {
            let dx = px - a.0;
            let dy = py - a.1;
            return dx * dx + dy * dy;
        }
        let t = (((px - a.0) * vx + (py - a.1) * vy) / len_sq).clamp(0.0, 1.0);
        let proj_x = a.0 + t * vx;
        let proj_y = a.1 + t * vy;
        let dx = px - proj_x;
        let dy = py - proj_y;
        dx * dx + dy * dy
    }

    type LastPaintEntry = (Camera, Option<(u8, usize)>, bool);

    /// Pure-Rust cache key for the cached offscreen static Canvas2D base layer.
    /// Excludes transient pointer `hover` so pointer move reuses the static base blit.
    #[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct OffscreenBaseKey {
        pub scene_digest: u64,
        pub dpr_bits: u64,
        pub vw_bits: u64,
        pub vh_bits: u64,
        pub tx_bits: u64,
        pub ty_bits: u64,
        pub scale_bits: u64,
        pub palette_epoch: u64,
        pub draw_nodes: bool,
        pub kind_hover: Option<String>,
        pub active_layer: Option<String>,
        pub hidden_kinds_digest: u64,
    }

    #[inline]
    fn fnv1a_mix_u64(hash: &mut u64, value: u64) {
        for b in value.to_le_bytes() {
            *hash ^= u64::from(b);
            *hash = hash.wrapping_mul(0x1000_0000_01b3);
        }
    }

    #[inline]
    fn fnv1a_mix_str(hash: &mut u64, s: &str) {
        fnv1a_mix_u64(hash, s.len() as u64);
        for &b in s.as_bytes() {
            *hash ^= u64::from(b);
            *hash = hash.wrapping_mul(0x1000_0000_01b3);
        }
    }

    #[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
    pub fn hidden_kinds_digest(hidden_kinds: &HashSet<String>) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325_u64;
        if hidden_kinds.is_empty() {
            return h;
        }
        let mut sorted: Vec<&str> = hidden_kinds.iter().map(String::as_str).collect();
        sorted.sort_unstable();
        for kind in sorted {
            fnv1a_mix_str(&mut h, kind);
        }
        h
    }

    pub fn compute_scene_digest(scene: &DenseScene, meta: &DenseSceneMeta) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325_u64;
        fnv1a_mix_u64(&mut h, scene.width.to_bits());
        fnv1a_mix_u64(&mut h, scene.height.to_bits());
        fnv1a_mix_u64(&mut h, scene.nodes.len() as u64);
        fnv1a_mix_u64(&mut h, scene.edges.len() as u64);
        fnv1a_mix_u64(&mut h, meta.containers.len() as u64);
        fnv1a_mix_u64(&mut h, meta.cluster_hulls.len() as u64);
        fnv1a_mix_u64(&mut h, u64::from(meta.commit_changes_active));
        fnv1a_mix_u64(&mut h, meta.label_min_scale.map_or(0, f64::to_bits));
        for node in &scene.nodes {
            fnv1a_mix_u64(&mut h, node.index as u64);
            fnv1a_mix_u64(&mut h, node.x.to_bits());
            fnv1a_mix_u64(&mut h, node.y.to_bits());
            fnv1a_mix_u64(&mut h, node.w.to_bits());
            fnv1a_mix_u64(&mut h, node.h.to_bits());
            let flags = u64::from(node.selected)
                | (u64::from(node.lit) << 1)
                | (u64::from(node.dim) << 2)
                | (u64::from(node.changed) << 3)
                | (u64::from(node.pinned) << 4)
                | (u64::from(node.failed) << 5);
            fnv1a_mix_u64(&mut h, flags);
            fnv1a_mix_str(&mut h, &node.label);
            fnv1a_mix_str(&mut h, &node.role);
            if let Some(nm) = meta.nodes.get(&node.index) {
                fnv1a_mix_u64(&mut h, u64::from(nm.in_cycle));
                fnv1a_mix_str(&mut h, &nm.symbol_kind);
                fnv1a_mix_str(&mut h, &nm.visibility);
                fnv1a_mix_str(&mut h, &nm.blast_tag);
                fnv1a_mix_str(&mut h, &nm.badge_text);
            }
            if let Some(layer) = meta.layer_of_node.get(&node.index) {
                fnv1a_mix_str(&mut h, layer);
            }
        }
        for edge in &scene.edges {
            fnv1a_mix_u64(&mut h, edge.index as u64);
            fnv1a_mix_u64(&mut h, edge.weight);
            fnv1a_mix_u64(&mut h, edge.pts.len() as u64);
            fnv1a_mix_u64(&mut h, edge.cmds.len() as u64);
            let flags = u64::from(edge.against)
                | (u64::from(edge.portal) << 1)
                | (u64::from(edge.on) << 2)
                | (u64::from(edge.lit) << 3)
                | (u64::from(edge.dim) << 4)
                | (u64::from(edge.detour) << 5)
                | (u64::from(edge.guessed) << 6)
                | (u64::from(edge.commit_touched) << 7)
                | (u64::from(edge.back) << 8);
            fnv1a_mix_u64(&mut h, flags);
            fnv1a_mix_str(&mut h, &edge.kind);
            fnv1a_mix_str(&mut h, &edge.diff);
            fnv1a_mix_str(&mut h, &edge.impact);
            fnv1a_mix_u64(&mut h, edge.kinds.len() as u64);
            for k in &edge.kinds {
                fnv1a_mix_str(&mut h, k);
            }
            for &(x, y) in &edge.pts {
                fnv1a_mix_u64(&mut h, x.to_bits());
                fnv1a_mix_u64(&mut h, y.to_bits());
            }
            for cmd in &edge.cmds {
                match *cmd {
                    DensePathCmd::Move(x, y) => {
                        fnv1a_mix_u64(&mut h, 1);
                        fnv1a_mix_u64(&mut h, x.to_bits());
                        fnv1a_mix_u64(&mut h, y.to_bits());
                    }
                    DensePathCmd::Line(x, y) => {
                        fnv1a_mix_u64(&mut h, 2);
                        fnv1a_mix_u64(&mut h, x.to_bits());
                        fnv1a_mix_u64(&mut h, y.to_bits());
                    }
                    DensePathCmd::Curve { c1, c2, to } => {
                        fnv1a_mix_u64(&mut h, 3);
                        fnv1a_mix_u64(&mut h, c1.0.to_bits());
                        fnv1a_mix_u64(&mut h, c1.1.to_bits());
                        fnv1a_mix_u64(&mut h, c2.0.to_bits());
                        fnv1a_mix_u64(&mut h, c2.1.to_bits());
                        fnv1a_mix_u64(&mut h, to.0.to_bits());
                        fnv1a_mix_u64(&mut h, to.1.to_bits());
                    }
                }
            }
        }
        for c in &meta.containers {
            let kind_tag = match c.kind {
                DenseContainerKind::TierBand => 0_u64,
                DenseContainerKind::ServiceFrame => 1_u64,
            };
            fnv1a_mix_u64(&mut h, kind_tag);
            fnv1a_mix_u64(&mut h, c.x.to_bits());
            fnv1a_mix_u64(&mut h, c.y.to_bits());
            fnv1a_mix_u64(&mut h, c.w.to_bits());
            fnv1a_mix_u64(&mut h, c.h.to_bits());
            fnv1a_mix_u64(&mut h, c.radius.to_bits());
            fnv1a_mix_str(&mut h, &c.label);
        }
        for hull in &meta.cluster_hulls {
            fnv1a_mix_str(&mut h, &hull.key);
            fnv1a_mix_str(&mut h, &hull.label);
            fnv1a_mix_u64(&mut h, hull.color_slot as u64);
            fnv1a_mix_u64(&mut h, hull.node_count as u64);
            fnv1a_mix_u64(&mut h, hull.bounds.0.to_bits());
            fnv1a_mix_u64(&mut h, hull.bounds.1.to_bits());
            fnv1a_mix_u64(&mut h, hull.bounds.2.to_bits());
            fnv1a_mix_u64(&mut h, hull.bounds.3.to_bits());
            fnv1a_mix_u64(&mut h, hull.centroid.0.to_bits());
            fnv1a_mix_u64(&mut h, hull.centroid.1.to_bits());
            fnv1a_mix_u64(&mut h, hull.polygon.len() as u64);
            for &(x, y) in &hull.polygon {
                fnv1a_mix_u64(&mut h, x.to_bits());
                fnv1a_mix_u64(&mut h, y.to_bits());
            }
        }
        h
    }

    #[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
    #[allow(clippy::too_many_arguments)]
    pub fn build_offscreen_base_key(
        scene_digest: u64,
        dpr: f64,
        vw: f64,
        vh: f64,
        cam: Camera,
        palette_epoch: u64,
        draw_nodes: bool,
        kind_hover: Option<&str>,
        active_layer: Option<&str>,
        hidden_kinds: &HashSet<String>,
    ) -> OffscreenBaseKey {
        OffscreenBaseKey {
            scene_digest,
            dpr_bits: dpr.to_bits(),
            vw_bits: vw.to_bits(),
            vh_bits: vh.to_bits(),
            tx_bits: cam.tx.to_bits(),
            ty_bits: cam.ty.to_bits(),
            scale_bits: cam.scale.to_bits(),
            palette_epoch,
            draw_nodes,
            kind_hover: kind_hover
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty()),
            active_layer: active_layer
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty()),
            hidden_kinds_digest: hidden_kinds_digest(hidden_kinds),
        }
    }

    #[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
    pub fn should_reuse_offscreen_base(
        cached: Option<&OffscreenBaseKey>,
        next: &OffscreenBaseKey,
    ) -> bool {
        cached == Some(next)
    }

    #[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
    pub fn is_overlay_edge(
        edge: &DenseEdgeSpec,
        hovered_edges: &HashSet<usize>,
        active_kind_hover_lower: Option<&str>,
    ) -> bool {
        hovered_edges.contains(&edge.index)
            || edge.on
            || edge.lit
            || active_kind_hover_lower.is_some_and(|hk| edge_matches_kind_lower(hk, edge))
    }

    #[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
    pub fn is_overlay_node(
        node: &DenseNodeSpec,
        hover: Option<(u8, usize)>,
        hovered_neighbors: &HashSet<usize>,
    ) -> bool {
        hover == Some((0, node.index))
            || hovered_neighbors.contains(&node.index)
            || node.selected
            || node.lit
    }

    #[cfg(target_arch = "wasm32")]
    struct OffscreenBaseLayer {
        key: OffscreenBaseKey,
        canvas: wasm_ctx::CanvasEl,
    }

    thread_local! {
        static DENSE_SCENES: RefCell<HashMap<String, (DenseScene, SpatialHitGrid, DenseSceneMeta)>> =
            RefCell::new(HashMap::new());
        static SCENE_DIGESTS: RefCell<HashMap<String, u64>> = RefCell::new(HashMap::new());
        static LAST_CAMERAS: RefCell<HashMap<String, Camera>> = RefCell::new(HashMap::new());
        static LAST_PAINT_STATE: RefCell<HashMap<String, LastPaintEntry>> =
            RefCell::new(HashMap::new());
        static LAST_PAINTED_VIEWPORT: RefCell<HashMap<String, (u32, u32)>> =
            RefCell::new(HashMap::new());
        static REPAINT_LISTENERS: RefCell<HashMap<String, Rc<dyn Fn()>>> =
            RefCell::new(HashMap::new());
        #[cfg(target_arch = "wasm32")]
        static OFFSCREEN_BASE_LAYERS: RefCell<HashMap<String, OffscreenBaseLayer>> =
            RefCell::new(HashMap::new());
    }

    fn invalidate_offscreen_base_layer(canvas_id: &str) {
        #[cfg(target_arch = "wasm32")]
        OFFSCREEN_BASE_LAYERS.with(|layers| {
            layers.borrow_mut().remove(canvas_id);
        });
        #[cfg(not(target_arch = "wasm32"))]
        let _ = canvas_id;
    }

    #[allow(dead_code)]
    pub fn store_scene(canvas_id: &str, scene: DenseScene) {
        store_scene_with_meta(canvas_id, scene, DenseSceneMeta::default());
    }

    pub fn store_scene_with_meta(canvas_id: &str, scene: DenseScene, mut meta: DenseSceneMeta) {
        meta.rebuild_caches(&scene);
        let digest = compute_scene_digest(&scene, &meta);
        let grid = SpatialHitGrid::build(&scene, 128.0);
        DENSE_SCENES.with(|s| {
            s.borrow_mut()
                .insert(canvas_id.to_owned(), (scene, grid, meta));
        });
        SCENE_DIGESTS.with(|d| {
            d.borrow_mut().insert(canvas_id.to_owned(), digest);
        });
        invalidate_offscreen_base_layer(canvas_id);
        #[cfg(target_arch = "wasm32")]
        if let Some(host) = dom::get(canvas_id) {
            let _ = host.set_attribute("data-dense", "true");
            if host.scroll_left() != 0 {
                host.set_scroll_left(0);
            }
            if host.scroll_top() != 0 {
                host.set_scroll_top(0);
            }
        }
    }

    #[allow(dead_code)]
    pub fn set_scene_meta(canvas_id: &str, mut meta: DenseSceneMeta) -> bool {
        DENSE_SCENES.with(|s| {
            let mut guard = s.borrow_mut();
            let Some((scene, _, existing_meta)) = guard.get_mut(canvas_id) else {
                return false;
            };
            meta.rebuild_caches(scene);
            *existing_meta = meta;
            true
        })
    }

    #[allow(dead_code)]
    pub fn set_kind_hover(canvas_id: &str, kind: Option<&str>) -> bool {
        let changed = DENSE_SCENES.with(|s| {
            let mut guard = s.borrow_mut();
            let Some((_, _, meta)) = guard.get_mut(canvas_id) else {
                return false;
            };
            let next = kind
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .map(str::to_owned);
            if meta.kind_hover == next {
                return false;
            }
            meta.kind_hover = next;
            true
        });
        if changed {
            invalidate_offscreen_base_layer(canvas_id);
        }
        changed
    }

    #[allow(dead_code)]
    pub fn set_active_layer(canvas_id: &str, layer: Option<&str>) -> bool {
        let changed = DENSE_SCENES.with(|s| {
            let mut guard = s.borrow_mut();
            let Some((_, _, meta)) = guard.get_mut(canvas_id) else {
                return false;
            };
            let next = layer
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned);
            if meta.active_layer == next {
                return false;
            }
            meta.active_layer = next;
            true
        });
        if changed {
            invalidate_offscreen_base_layer(canvas_id);
        }
        changed
    }

    #[allow(dead_code)]
    pub fn set_hidden_kinds(canvas_id: &str, hidden: HashSet<String>) -> bool {
        let changed = DENSE_SCENES.with(|s| {
            let mut guard = s.borrow_mut();
            let Some((_, _, meta)) = guard.get_mut(canvas_id) else {
                return false;
            };
            meta.hidden_kinds = hidden
                .into_iter()
                .map(|k| k.trim().to_ascii_lowercase())
                .filter(|k| !k.is_empty())
                .collect();
            true
        });
        if changed {
            invalidate_offscreen_base_layer(canvas_id);
        }
        changed
    }

    #[allow(dead_code)]
    pub fn explain_target_for_hit(
        canvas_id: &str,
        kind: u8,
        index: usize,
    ) -> Option<(String, Value)> {
        DENSE_SCENES.with(|s| {
            let guard = s.borrow();
            let (scene, _, meta) = guard.get(canvas_id)?;
            match kind {
                0 => meta
                    .nodes
                    .get(&index)
                    .map(|n| (n.explain_kind.clone(), n.explain_payload.clone()))
                    .or_else(|| {
                        scene
                            .nodes
                            .iter()
                            .find(|n| n.index == index && !n.label.is_empty())
                            .map(|n| ("node".to_owned(), Value::String(n.label.clone())))
                    }),
                1 => meta
                    .edges
                    .get(&index)
                    .map(|e| (e.explain_kind.clone(), e.explain_payload.clone())),
                _ => None,
            }
        })
    }

    #[allow(dead_code)]
    pub fn last_camera(canvas_id: &str) -> Option<Camera> {
        LAST_CAMERAS.with(|c| c.borrow().get(canvas_id).copied())
    }

    #[cfg(target_arch = "wasm32")]
    #[allow(dead_code)]
    pub fn hit_test_client(
        canvas_id: &str,
        client_x: f64,
        client_y: f64,
        tol_px: f64,
    ) -> Option<(u8, usize)> {
        let host = dom::get(canvas_id)?;
        let cam = last_camera(canvas_id)?;
        if !cam.scale.is_finite() || cam.scale <= 0.0 {
            return None;
        }
        let rect = host.get_bounding_client_rect();
        let wx = (client_x - rect.left() - cam.tx) / cam.scale;
        let wy = (client_y - rect.top() - cam.ty) / cam.scale;
        hit_test_scene(
            canvas_id,
            wx,
            wy,
            dense_dom::scale_normalized_hit_tol(tol_px, cam.scale),
        )
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[allow(dead_code)]
    pub fn hit_test_client(
        _canvas_id: &str,
        _client_x: f64,
        _client_y: f64,
        _tol_px: f64,
    ) -> Option<(u8, usize)> {
        None
    }

    /// Updates visual state flags (`selected`, `lit`, `dim`, `on`, `failed`, etc.) of an
    /// already-stored `DenseScene` without rebuilding the spatial hit grid.
    pub fn update_scene<F>(canvas_id: &str, f: F) -> bool
    where
        F: FnOnce(&mut DenseScene),
    {
        let digest_opt = DENSE_SCENES.with(|s| {
            let mut guard = s.borrow_mut();
            let (scene, _, meta) = guard.get_mut(canvas_id)?;
            f(scene);
            Some(compute_scene_digest(scene, meta))
        });
        if let Some(digest) = digest_opt {
            SCENE_DIGESTS.with(|d| {
                d.borrow_mut().insert(canvas_id.to_owned(), digest);
            });
            invalidate_offscreen_base_layer(canvas_id);
            true
        } else {
            false
        }
    }

    /// Mutates the stored `DenseScene` (e.g. during Shift+drag node repositioning) and
    /// refreshes both the `SpatialHitGrid` and cached edge bounds for `moved_nodes`/`moved_edges`.
    /// Passing empty slices performs a full rebuild of hit grid and edge-bound caches.
    #[allow(dead_code)]
    pub fn update_scene_and_grid<F>(
        canvas_id: &str,
        moved_nodes: &[usize],
        moved_edges: &[usize],
        f: F,
    ) -> bool
    where
        F: FnOnce(&mut DenseScene),
    {
        let digest_opt = DENSE_SCENES.with(|s| {
            let mut guard = s.borrow_mut();
            let (scene, grid, meta) = guard.get_mut(canvas_id)?;
            f(scene);
            if moved_nodes.is_empty() && moved_edges.is_empty() {
                *grid = SpatialHitGrid::build(scene, 128.0);
                meta.rebuild_caches(scene);
            } else {
                grid.update_nodes_and_edges(scene, moved_nodes, moved_edges);
                meta.update_edge_bounds(scene, moved_edges);
                if !moved_nodes.is_empty() && meta.containers.is_empty() {
                    meta.cluster_hulls = compute_dense_cluster_hulls(scene, meta);
                }
            }
            Some(compute_scene_digest(scene, meta))
        });
        if let Some(digest) = digest_opt {
            SCENE_DIGESTS.with(|d| {
                d.borrow_mut().insert(canvas_id.to_owned(), digest);
            });
            invalidate_offscreen_base_layer(canvas_id);
            true
        } else {
            false
        }
    }

    #[allow(dead_code)]
    pub fn rebuild_grid(canvas_id: &str) -> bool {
        update_scene_and_grid(canvas_id, &[], &[], |_| {})
    }

    #[allow(dead_code)]
    pub fn set_repaint_listener(canvas_id: &str, cb: Rc<dyn Fn()>) {
        REPAINT_LISTENERS.with(|hooks| {
            hooks.borrow_mut().insert(canvas_id.to_owned(), cb);
        });
    }

    pub fn clear_scene(canvas_id: &str) {
        DENSE_SCENES.with(|s| {
            s.borrow_mut().remove(canvas_id);
        });
        SCENE_DIGESTS.with(|d| {
            d.borrow_mut().remove(canvas_id);
        });
        invalidate_offscreen_base_layer(canvas_id);
        LAST_CAMERAS.with(|c| {
            c.borrow_mut().remove(canvas_id);
        });
        LAST_PAINT_STATE.with(|p| {
            p.borrow_mut().remove(canvas_id);
        });
        LAST_PAINTED_VIEWPORT.with(|v| {
            v.borrow_mut().remove(canvas_id);
        });
        REPAINT_LISTENERS.with(|hooks| {
            hooks.borrow_mut().remove(canvas_id);
        });
        #[cfg(target_arch = "wasm32")]
        if let Some(host) = dom::get(canvas_id) {
            let _ = host.remove_attribute("data-dense");
            if let Ok(Some(el)) = host.query_selector("canvas[data-dense-canvas='true']") {
                el.remove();
            }
        }
    }

    pub fn hit_test_scene(
        canvas_id: &str,
        world_x: f64,
        world_y: f64,
        tol: f64,
    ) -> Option<(u8, usize)> {
        DENSE_SCENES.with(|s| {
            let guard = s.borrow();
            let (scene, grid, meta) = guard.get(canvas_id)?;
            if meta.hidden_kinds.is_empty() {
                return grid.hit_test(world_x, world_y, tol);
            }
            grid.hit_test_filtered(world_x, world_y, tol, |edge_idx| {
                let edge_opt = scene
                    .edges
                    .get(edge_idx)
                    .filter(|e| e.index == edge_idx)
                    .or_else(|| scene.edges.iter().find(|e| e.index == edge_idx));
                let Some(edge) = edge_opt else {
                    return true;
                };
                !meta
                    .hidden_kinds
                    .iter()
                    .any(|hk| edge_matches_kind_lower(hk, edge))
            })
        })
    }

    #[cfg(target_arch = "wasm32")]
    mod wasm_ctx {
        use wasm_bindgen::prelude::*;

        #[wasm_bindgen]
        extern "C" {
            pub(super) type CanvasEl;
            #[wasm_bindgen(method, getter = width)]
            pub(super) fn width(this: &CanvasEl) -> u32;
            #[wasm_bindgen(method, setter = width)]
            pub(super) fn set_width(this: &CanvasEl, v: u32);
            #[wasm_bindgen(method, getter = height)]
            pub(super) fn height(this: &CanvasEl) -> u32;
            #[wasm_bindgen(method, setter = height)]
            pub(super) fn set_height(this: &CanvasEl, v: u32);
            #[wasm_bindgen(method, js_name = getContext)]
            pub(super) fn get_context(this: &CanvasEl, kind: &str) -> Option<Ctx2d>;

            pub(super) type TextMetrics;
            #[wasm_bindgen(method, getter = width)]
            pub(super) fn width(this: &TextMetrics) -> f64;

            pub(super) type Ctx2d;
            #[wasm_bindgen(method, js_name = setTransform)]
            pub(super) fn set_transform(
                this: &Ctx2d,
                a: f64,
                b: f64,
                c: f64,
                d: f64,
                e: f64,
                f: f64,
            );
            #[wasm_bindgen(method, js_name = clearRect)]
            pub(super) fn clear_rect(this: &Ctx2d, x: f64, y: f64, w: f64, h: f64);
            #[wasm_bindgen(method, js_name = drawImage)]
            pub(super) fn draw_image(
                this: &Ctx2d,
                image: &CanvasEl,
                dx: f64,
                dy: f64,
                dw: f64,
                dh: f64,
            );
            #[wasm_bindgen(method, setter = strokeStyle)]
            pub(super) fn set_stroke_style(this: &Ctx2d, v: &str);
            #[wasm_bindgen(method, setter = fillStyle)]
            pub(super) fn set_fill_style(this: &Ctx2d, v: &str);
            #[wasm_bindgen(method, setter = lineWidth)]
            pub(super) fn set_line_width(this: &Ctx2d, v: f64);
            #[wasm_bindgen(method, setter = globalAlpha)]
            pub(super) fn set_global_alpha(this: &Ctx2d, v: f64);
            #[wasm_bindgen(method, setter = font)]
            pub(super) fn set_font(this: &Ctx2d, v: &str);
            #[wasm_bindgen(method, js_name = setLineDash)]
            pub(super) fn set_line_dash(this: &Ctx2d, dash: &js_sys::Array);
            #[wasm_bindgen(method, js_name = beginPath)]
            pub(super) fn begin_path(this: &Ctx2d);
            #[wasm_bindgen(method, js_name = closePath)]
            pub(super) fn close_path(this: &Ctx2d);
            #[wasm_bindgen(method, js_name = moveTo)]
            pub(super) fn move_to(this: &Ctx2d, x: f64, y: f64);
            #[wasm_bindgen(method, js_name = lineTo)]
            pub(super) fn line_to(this: &Ctx2d, x: f64, y: f64);
            #[wasm_bindgen(method, js_name = bezierCurveTo)]
            pub(super) fn bezier_curve_to(
                this: &Ctx2d,
                cp1x: f64,
                cp1y: f64,
                cp2x: f64,
                cp2y: f64,
                x: f64,
                y: f64,
            );
            #[wasm_bindgen(method)]
            pub(super) fn stroke(this: &Ctx2d);
            #[wasm_bindgen(method)]
            pub(super) fn fill(this: &Ctx2d);
            #[wasm_bindgen(method, js_name = fillRect)]
            pub(super) fn fill_rect(this: &Ctx2d, x: f64, y: f64, w: f64, h: f64);
            #[wasm_bindgen(method, js_name = strokeRect)]
            pub(super) fn stroke_rect(this: &Ctx2d, x: f64, y: f64, w: f64, h: f64);
            #[wasm_bindgen(method, js_name = measureText)]
            pub(super) fn measure_text(this: &Ctx2d, text: &str) -> TextMetrics;
            #[wasm_bindgen(method, js_name = fillText)]
            pub(super) fn fill_text(this: &Ctx2d, text: &str, x: f64, y: f64, max_width: f64);
            #[wasm_bindgen(method, js_name = fillText)]
            pub(super) fn fill_text_unscaled(this: &Ctx2d, text: &str, x: f64, y: f64);

            pub(super) type JsMutationObserver;
            #[wasm_bindgen(constructor, js_class = MutationObserver)]
            pub(super) fn new(cb: &js_sys::Function) -> JsMutationObserver;
            #[wasm_bindgen(method)]
            pub(super) fn observe(
                this: &JsMutationObserver,
                target: &web_sys::Node,
                init: &JsValue,
            );

            pub(super) type JsResizeObserver;
            #[wasm_bindgen(constructor, js_class = ResizeObserver)]
            pub(super) fn new(cb: &js_sys::Function) -> JsResizeObserver;
            #[wasm_bindgen(method)]
            pub(super) fn observe(this: &JsResizeObserver, target: &web_sys::Element);
        }
    }

    #[cfg(target_arch = "wasm32")]
    #[derive(Clone)]
    struct CanvasPalette {
        paper: String,
        soft: String,
        line: String,
        ink: String,
        muted: String,
        purple: String,
        against: String,
        changed: String,
        warn: String,
        diff_added: String,
        diff_removed: String,
        diff_risk: String,
        diff_modified: String,
        diff_touched: String,
        edge_base: String,
        edge_calls: String,
        edge_imports: String,
        edge_extends: String,
        edge_implements: String,
        edge_writes: String,
        edge_reads: String,
        edge_uses: String,
        sys_http: String,
        sys_event: String,
        sys_rpc: String,
        sys_graphql: String,
        sys_shared: String,
        sys_code: String,
    }

    #[cfg(target_arch = "wasm32")]
    struct DashArrays {
        solid: js_sys::Array,
        dashed: js_sys::Array,
        dotted: js_sys::Array,
        dash_dot: js_sys::Array,
    }

    /// Computes backed Canvas2D pixel dimensions `(target_w, target_h)` for a
    /// laid-out CSS box `(css_w, css_h)` at clamped DPR `[1.0, 2.0]`.
    #[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
    pub(crate) fn dense_bitmap_target_size(css_w: f64, css_h: f64, dpr: f64) -> Option<(u32, u32)> {
        if !css_w.is_finite() || !css_h.is_finite() || css_w < 64.0 || css_h < 64.0 {
            return None;
        }
        let clamped_dpr = if dpr.is_finite() {
            dpr.clamp(1.0, 2.0)
        } else {
            1.0
        };
        Some((
            ((css_w * clamped_dpr).round() as u32).max(1),
            ((css_h * clamped_dpr).round() as u32).max(1),
        ))
    }

    #[cfg(target_arch = "wasm32")]
    struct ThemeWatcherHandles {
        _observer: wasm_ctx::JsMutationObserver,
        _obs_cb: Closure<dyn FnMut(JsValue)>,
        _mq_forced: Option<web_sys::MediaQueryList>,
        _mq_dark: Option<web_sys::MediaQueryList>,
        _mq_cb: Closure<dyn FnMut(web_sys::Event)>,
    }

    #[cfg(target_arch = "wasm32")]
    struct ResizeWatcherHandle {
        observer: wasm_ctx::JsResizeObserver,
        _cb: Closure<dyn FnMut(js_sys::Array)>,
    }

    #[cfg(target_arch = "wasm32")]
    thread_local! {
        static PALETTE_THEME_EPOCH: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
        static RESOLVED_THEME_META: RefCell<Option<(u64, String, bool)>> = const { RefCell::new(None) };
        static PALETTE_CACHE: RefCell<HashMap<(String, u64), CanvasPalette>> =
            RefCell::new(HashMap::new());
        static DASH_CACHE: RefCell<Option<DashArrays>> = const { RefCell::new(None) };
        static THEME_WATCHERS: RefCell<Option<ThemeWatcherHandles>> = const { RefCell::new(None) };
        static RESIZE_WATCHER: RefCell<Option<ResizeWatcherHandle>> = const { RefCell::new(None) };
    }

    #[cfg(target_arch = "wasm32")]
    fn measure_dense_canvas_box(
        canvas_el: Option<&Element>,
        canvas_id: &str,
    ) -> Option<(f64, f64, f64, u32, u32)> {
        let (mut vw, mut vh, measured) = viewport_size(canvas_id, 64.0, (800.0, 500.0));
        if !measured {
            return None;
        }
        if let Some(el) = canvas_el {
            let rect = el.get_bounding_client_rect();
            if rect.width() >= 64.0 && rect.height() >= 64.0 {
                vw = rect.width();
                vh = rect.height();
            }
        }
        let dpr = dom::window().device_pixel_ratio().clamp(1.0, 2.0);
        let (target_w, target_h) = dense_bitmap_target_size(vw, vh, dpr)?;
        Some((vw, vh, dpr, target_w, target_h))
    }

    #[cfg(target_arch = "wasm32")]
    fn repaint_resized_canvases(entries: &js_sys::Array) {
        let mut pending: Vec<(String, LastPaintEntry)> = Vec::new();
        for entry in entries.iter() {
            let Ok(target_val) = js_sys::Reflect::get(&entry, &"target".into()) else {
                continue;
            };
            let Ok(host) = target_val.dyn_into::<Element>() else {
                continue;
            };
            let id = host.id();
            if id.is_empty() {
                continue;
            }
            let Some(paint_state) = LAST_PAINT_STATE.with(|p| p.borrow().get(&id).copied()) else {
                continue;
            };
            let canvas_el = host
                .query_selector("canvas[data-dense-canvas='true']")
                .ok()
                .flatten();
            let Some((_, _, _, cur_w, cur_h)) = measure_dense_canvas_box(canvas_el.as_ref(), &id)
            else {
                continue;
            };
            let prev = LAST_PAINTED_VIEWPORT.with(|v| v.borrow().get(&id).copied());
            let backing_stale = canvas_el.as_ref().is_some_and(|el| {
                let c: wasm_ctx::CanvasEl = el.clone().unchecked_into();
                c.width() != cur_w || c.height() != cur_h
            });
            if prev != Some((cur_w, cur_h)) || backing_stale {
                pending.push((id, paint_state));
            }
        }
        for (id, (cam, hover, draw_nodes)) in pending {
            paint_scene(&id, cam, hover, draw_nodes);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn ensure_resize_watcher(host: &Element) {
        if host.get_attribute("data-resize-observed").as_deref() == Some("true") {
            return;
        }
        RESIZE_WATCHER.with(|cell| {
            let mut slot = cell.borrow_mut();
            if slot.is_none() {
                let has_ro = js_sys::Reflect::get(&dom::window(), &"ResizeObserver".into())
                    .is_ok_and(|v| v.is_function());
                if !has_ro {
                    return;
                }
                let cb = Closure::wrap(Box::new(move |entries: js_sys::Array| {
                    repaint_resized_canvases(&entries);
                }) as Box<dyn FnMut(js_sys::Array)>);
                let observer = wasm_ctx::JsResizeObserver::new(cb.as_ref().unchecked_ref());
                *slot = Some(ResizeWatcherHandle { observer, _cb: cb });
            }
            if let Some(ref handle) = *slot {
                handle.observer.observe(host);
                let _ = host.set_attribute("data-resize-observed", "true");
            }
        });
    }

    #[allow(dead_code)]
    pub fn invalidate_palettes_and_repaint() {
        #[cfg(target_arch = "wasm32")]
        {
            PALETTE_THEME_EPOCH.with(|e| e.set(e.get().wrapping_add(1)));
            RESOLVED_THEME_META.with(|m| *m.borrow_mut() = None);
            PALETTE_CACHE.with(|c| c.borrow_mut().clear());
            OFFSCREEN_BASE_LAYERS.with(|layers| layers.borrow_mut().clear());
            LABEL_FIT_CACHE.with(|c| c.borrow_mut().clear());
            BADGE_WIDTH_CACHE.with(|c| c.borrow_mut().clear());
            let paints: Vec<(String, LastPaintEntry)> = LAST_PAINT_STATE.with(|p| {
                p.borrow()
                    .iter()
                    .map(|(id, &entry)| (id.clone(), entry))
                    .collect()
            });
            let listeners: Vec<Rc<dyn Fn()>> = REPAINT_LISTENERS.with(|hooks| {
                paints
                    .iter()
                    .filter_map(|(id, _)| hooks.borrow().get(id).cloned())
                    .collect()
            });
            for (id, (cam, hover, draw_nodes)) in paints {
                paint_scene(&id, cam, hover, draw_nodes);
            }
            for cb in listeners {
                cb();
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn ensure_theme_watchers() {
        THEME_WATCHERS.with(|cell| {
            if cell.borrow().is_some() {
                return;
            }
            let obs_cb = Closure::wrap(Box::new(move |_records: JsValue| {
                invalidate_palettes_and_repaint();
            }) as Box<dyn FnMut(JsValue)>);
            let observer = wasm_ctx::JsMutationObserver::new(obs_cb.as_ref().unchecked_ref());
            if let Some(root) = dom::document().document_element() {
                let opts = js_sys::Object::new();
                let _ = js_sys::Reflect::set(&opts, &"attributes".into(), &JsValue::TRUE);
                let filter = js_sys::Array::new();
                filter.push(&"data-theme".into());
                let _ = js_sys::Reflect::set(&opts, &"attributeFilter".into(), &filter);
                observer.observe(root.unchecked_ref(), opts.as_ref());
            }

            let mq_cb = Closure::wrap(Box::new(move |_ev: web_sys::Event| {
                invalidate_palettes_and_repaint();
            }) as Box<dyn FnMut(web_sys::Event)>);
            let mq_forced = dom::window()
                .match_media("(forced-colors: active)")
                .ok()
                .flatten();
            if let Some(ref mq) = mq_forced {
                let _ =
                    mq.add_event_listener_with_callback("change", mq_cb.as_ref().unchecked_ref());
            }
            let mq_dark = dom::window()
                .match_media("(prefers-color-scheme: dark)")
                .ok()
                .flatten();
            if let Some(ref mq) = mq_dark {
                let _ =
                    mq.add_event_listener_with_callback("change", mq_cb.as_ref().unchecked_ref());
            }

            *cell.borrow_mut() = Some(ThemeWatcherHandles {
                _observer: observer,
                _obs_cb: obs_cb,
                _mq_forced: mq_forced,
                _mq_dark: mq_dark,
                _mq_cb: mq_cb,
            });
        });
    }

    #[cfg(target_arch = "wasm32")]
    fn css_token(
        style: Option<&web_sys::CssStyleDeclaration>,
        var: &str,
        fallback: &str,
    ) -> String {
        if let Some(style) = style {
            if let Ok(val) = style.get_property_value(var) {
                let t = val.trim();
                if !t.is_empty() {
                    return t.to_owned();
                }
            }
        }
        fallback.to_owned()
    }

    #[cfg(target_arch = "wasm32")]
    fn resolve_palette(canvas_id: &str, host: &Element) -> CanvasPalette {
        let epoch = PALETTE_THEME_EPOCH.with(|e| e.get());
        let key = (canvas_id.to_owned(), epoch);
        if let Some(cached) = PALETTE_CACHE.with(|c| c.borrow().get(&key).cloned()) {
            return cached;
        }
        let (_theme_key, forced_colors) = RESOLVED_THEME_META.with(|cell| {
            let mut slot = cell.borrow_mut();
            if let Some((e, tk, fc)) = slot.as_ref() {
                if *e == epoch {
                    return (tk.clone(), *fc);
                }
            }
            let tk = dom::document()
                .document_element()
                .and_then(|root| root.get_attribute("data-theme"))
                .unwrap_or_default();
            let fc = dom::window()
                .match_media("(forced-colors: active)")
                .ok()
                .flatten()
                .is_some_and(|mq| mq.matches());
            *slot = Some((epoch, tk.clone(), fc));
            (tk, fc)
        });
        let palette = if forced_colors {
            CanvasPalette {
                paper: "Canvas".to_owned(),
                soft: "Canvas".to_owned(),
                line: "CanvasText".to_owned(),
                ink: "CanvasText".to_owned(),
                muted: "GrayText".to_owned(),
                purple: "Highlight".to_owned(),
                against: "Highlight".to_owned(),
                changed: "LinkText".to_owned(),
                warn: "Highlight".to_owned(),
                diff_added: "LinkText".to_owned(),
                diff_removed: "Highlight".to_owned(),
                diff_risk: "Highlight".to_owned(),
                diff_modified: "Highlight".to_owned(),
                diff_touched: "LinkText".to_owned(),
                edge_base: "CanvasText".to_owned(),
                edge_calls: "CanvasText".to_owned(),
                edge_imports: "CanvasText".to_owned(),
                edge_extends: "LinkText".to_owned(),
                edge_implements: "LinkText".to_owned(),
                edge_writes: "CanvasText".to_owned(),
                edge_reads: "GrayText".to_owned(),
                edge_uses: "CanvasText".to_owned(),
                sys_http: "LinkText".to_owned(),
                sys_event: "LinkText".to_owned(),
                sys_rpc: "Highlight".to_owned(),
                sys_graphql: "Highlight".to_owned(),
                sys_shared: "CanvasText".to_owned(),
                sys_code: "CanvasText".to_owned(),
            }
        } else {
            let style = dom::window().get_computed_style(host).ok().flatten();
            let s = style.as_ref();
            CanvasPalette {
                paper: css_token(s, "--paper", "rgb(255, 255, 255)"),
                soft: css_token(s, "--soft", "rgb(248, 248, 250)"),
                line: css_token(s, "--line", "rgb(203, 213, 225)"),
                ink: css_token(s, "--ink", "rgb(15, 23, 42)"),
                muted: css_token(s, "--muted", "rgb(100, 116, 139)"),
                purple: css_token(s, "--purple", "rgb(124, 58, 237)"),
                against: css_token(s, "--error", "rgb(220, 38, 38)"),
                changed: css_token(s, "--success", "rgb(22, 163, 74)"),
                warn: css_token(s, "--warning", "rgb(217, 119, 6)"),
                diff_added: css_token(s, "--graph-diff-added", "rgb(21, 110, 57)"),
                diff_removed: css_token(s, "--graph-diff-removed", "rgb(180, 35, 24)"),
                diff_risk: css_token(s, "--graph-diff-risk", "rgb(154, 77, 0)"),
                diff_modified: css_token(s, "--graph-diff-modified", "rgb(94, 56, 189)"),
                diff_touched: css_token(s, "--graph-diff-touched", "rgb(29, 95, 214)"),
                edge_base: css_token(s, "--map-edge-from", "rgb(100, 116, 139)"),
                edge_calls: css_token(s, "--map-edge-calls", "rgb(34, 102, 194)"),
                edge_imports: css_token(s, "--map-edge-imports", "rgb(92, 89, 104)"),
                edge_extends: css_token(s, "--map-edge-extends", "rgb(104, 65, 198)"),
                edge_implements: css_token(s, "--map-edge-implements", "rgb(12, 107, 120)"),
                edge_writes: css_token(s, "--map-edge-writes", "rgb(184, 75, 20)"),
                edge_reads: css_token(s, "--map-edge-reads", "rgb(122, 92, 0)"),
                edge_uses: css_token(s, "--map-edge-uses", "rgb(145, 41, 118)"),
                sys_http: css_token(s, "--sys-edge-http", "rgb(29, 95, 214)"),
                sys_event: css_token(s, "--sys-edge-event", "rgb(12, 107, 120)"),
                sys_rpc: css_token(s, "--sys-edge-rpc", "rgb(104, 65, 198)"),
                sys_graphql: css_token(s, "--sys-edge-graphql", "rgb(145, 41, 118)"),
                sys_shared: css_token(s, "--sys-edge-shared", "rgb(21, 110, 57)"),
                sys_code: css_token(s, "--sys-edge-code", "rgb(184, 75, 20)"),
            }
        };
        PALETTE_CACHE.with(|c| {
            let mut map = c.borrow_mut();
            if map.len() > 32 {
                map.clear();
            }
            map.insert(key, palette.clone());
        });
        palette
    }

    #[cfg(target_arch = "wasm32")]
    fn with_dash_arrays<F: FnOnce(&DashArrays)>(f: F) {
        DASH_CACHE.with(|cell| {
            let mut slot = cell.borrow_mut();
            let arrays = slot.get_or_insert_with(|| {
                let solid = js_sys::Array::new();
                let dashed = js_sys::Array::new();
                dashed.push(&wasm_bindgen::JsValue::from_f64(7.0));
                dashed.push(&wasm_bindgen::JsValue::from_f64(4.0));
                let dotted = js_sys::Array::new();
                dotted.push(&wasm_bindgen::JsValue::from_f64(2.5));
                dotted.push(&wasm_bindgen::JsValue::from_f64(3.5));
                let dash_dot = js_sys::Array::new();
                dash_dot.push(&wasm_bindgen::JsValue::from_f64(6.0));
                dash_dot.push(&wasm_bindgen::JsValue::from_f64(3.0));
                dash_dot.push(&wasm_bindgen::JsValue::from_f64(2.0));
                dash_dot.push(&wasm_bindgen::JsValue::from_f64(3.0));
                DashArrays {
                    solid,
                    dashed,
                    dotted,
                    dash_dot,
                }
            });
            f(arrays);
        });
    }

    #[cfg(target_arch = "wasm32")]
    fn palette_slot(p: &CanvasPalette, slot: EdgeColorSlot) -> &str {
        match slot {
            EdgeColorSlot::Base => &p.edge_base,
            EdgeColorSlot::Calls => &p.edge_calls,
            EdgeColorSlot::Imports => &p.edge_imports,
            EdgeColorSlot::Extends => &p.edge_extends,
            EdgeColorSlot::Implements => &p.edge_implements,
            EdgeColorSlot::Writes => &p.edge_writes,
            EdgeColorSlot::Reads => &p.edge_reads,
            EdgeColorSlot::Uses => &p.edge_uses,
            EdgeColorSlot::Http => &p.sys_http,
            EdgeColorSlot::Event => &p.sys_event,
            EdgeColorSlot::Rpc => &p.sys_rpc,
            EdgeColorSlot::Graphql => &p.sys_graphql,
            EdgeColorSlot::Shared => &p.sys_shared,
            EdgeColorSlot::Code => &p.sys_code,
            EdgeColorSlot::Against => &p.against,
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn palette_for_tone(p: &CanvasPalette, tone: DenseEdgeTone) -> &str {
        match tone {
            DenseEdgeTone::Against => &p.against,
            DenseEdgeTone::DiffRemoved => &p.diff_removed,
            DenseEdgeTone::DiffAdded => &p.diff_added,
            DenseEdgeTone::DiffModified => &p.diff_modified,
            DenseEdgeTone::ImpactRisk => &p.diff_risk,
            DenseEdgeTone::CommitTouched => &p.diff_touched,
            DenseEdgeTone::BackEdge => &p.warn,
            DenseEdgeTone::Selection => &p.purple,
            DenseEdgeTone::Base(slot) => palette_slot(p, slot),
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn dense_node_badge_label(nm: &DenseNodeMeta) -> Option<String> {
        if !nm.badge_text.is_empty() {
            return Some(nm.badge_text.clone());
        }
        if !nm.blast_tag.is_empty() {
            return Some(nm.blast_tag.clone());
        }
        if nm.in_cycle {
            return Some("cyc".to_owned());
        }
        match nm.visibility.as_str() {
            "public" | "pub" => return Some("pub".to_owned()),
            "protected" | "prot" => return Some("prot".to_owned()),
            "private" | "priv" => return Some("priv".to_owned()),
            "internal" => return Some("int".to_owned()),
            _ => {}
        }
        match nm.symbol_kind.as_str() {
            "" => None,
            "function" | "method" => Some("fn".to_owned()),
            "class" => Some("cls".to_owned()),
            "interface" | "trait" | "protocol" => Some("ifc".to_owned()),
            "enum" => Some("enum".to_owned()),
            "struct" => Some("str".to_owned()),
            other => Some(truncate_label_chars(other, 5)),
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn palette_for_cluster_slot(p: &CanvasPalette, slot: usize) -> &str {
        match slot % 6 {
            0 => &p.edge_imports,
            1 => &p.edge_calls,
            2 => &p.edge_extends,
            3 => &p.edge_writes,
            4 => &p.sys_rpc,
            _ => &p.purple,
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn trace_rounded_rect(ctx: &wasm_ctx::Ctx2d, x: f64, y: f64, w: f64, h: f64, radius: f64) {
        let r = radius.max(0.0).min(w * 0.5).min(h * 0.5);
        ctx.begin_path();
        if r <= 0.5 {
            ctx.move_to(x, y);
            ctx.line_to(x + w, y);
            ctx.line_to(x + w, y + h);
            ctx.line_to(x, y + h);
            ctx.close_path();
            return;
        }
        let k = r * 0.552_284_749_8;
        ctx.move_to(x + r, y);
        ctx.line_to(x + w - r, y);
        ctx.bezier_curve_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
        ctx.line_to(x + w, y + h - r);
        ctx.bezier_curve_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
        ctx.line_to(x + r, y + h);
        ctx.bezier_curve_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
        ctx.line_to(x, y + r);
        ctx.bezier_curve_to(x, y + r - k, x + r - k, y, x + r, y);
        ctx.close_path();
    }

    #[cfg(target_arch = "wasm32")]
    #[allow(clippy::too_many_arguments)]
    fn paint_dense_containers(
        ctx: &wasm_ctx::Ctx2d,
        containers: &[DenseContainerSpec],
        pal: &CanvasPalette,
        dashes: &DashArrays,
        cam_scale: f64,
        view_bounds: (f64, f64, f64, f64),
        current_font: &mut Option<&'static str>,
    ) {
        if containers.is_empty() {
            return;
        }
        let (view_x0, view_y0, view_x1, view_y1) = view_bounds;
        let show_headers = cam_scale >= 0.20;
        let stroke_w = (1.1 / cam_scale.max(0.25)).clamp(1.0, 2.2);
        for pass_kind in [
            DenseContainerKind::TierBand,
            DenseContainerKind::ServiceFrame,
        ] {
            for container in containers.iter().filter(|c| c.kind == pass_kind) {
                if !container.x.is_finite()
                    || !container.y.is_finite()
                    || !container.w.is_finite()
                    || !container.h.is_finite()
                    || container.w <= 0.0
                    || container.h <= 0.0
                {
                    continue;
                }
                if container.x + container.w < view_x0
                    || container.x > view_x1
                    || container.y + container.h < view_y0
                    || container.y > view_y1
                {
                    continue;
                }
                trace_rounded_rect(
                    ctx,
                    container.x,
                    container.y,
                    container.w,
                    container.h,
                    container.radius,
                );
                match container.kind {
                    DenseContainerKind::TierBand => {
                        ctx.set_line_dash(&dashes.solid);
                        ctx.set_global_alpha(0.65);
                        ctx.set_fill_style(&pal.soft);
                        ctx.fill();
                        ctx.set_global_alpha(0.55);
                        ctx.set_line_width(stroke_w);
                        ctx.set_stroke_style(&pal.line);
                        ctx.stroke();
                    }
                    DenseContainerKind::ServiceFrame => {
                        ctx.set_global_alpha(0.32);
                        ctx.set_fill_style(&pal.soft);
                        ctx.fill();
                        ctx.set_line_dash(&dashes.dashed);
                        ctx.set_global_alpha(0.85);
                        ctx.set_line_width(stroke_w * 1.1);
                        ctx.set_stroke_style(&pal.line);
                        ctx.stroke();
                    }
                }
                if show_headers && !container.label.is_empty() {
                    let header_font = "600 11px sans-serif";
                    if *current_font != Some(header_font) {
                        ctx.set_font(header_font);
                        *current_font = Some(header_font);
                    }
                    let (lx, ly, alpha, color) = match container.kind {
                        DenseContainerKind::TierBand => {
                            (container.x + 10.0, container.y + 18.0, 0.85, &pal.muted)
                        }
                        DenseContainerKind::ServiceFrame => {
                            (container.x + 12.0, container.y + 18.0, 0.92, &pal.ink)
                        }
                    };
                    ctx.set_global_alpha(alpha);
                    ctx.set_fill_style(color);
                    let max_w = (container.w - 22.0).max(16.0);
                    ctx.fill_text(&container.label, lx, ly, max_w);
                }
            }
        }
        ctx.set_line_dash(&dashes.solid);
        ctx.set_global_alpha(1.0);
    }

    #[cfg(target_arch = "wasm32")]
    fn paint_cluster_hulls_underlay(
        ctx: &wasm_ctx::Ctx2d,
        hulls: &[DenseClusterHull],
        pal: &CanvasPalette,
        dashes: &DashArrays,
        cam_scale: f64,
        view_bounds: (f64, f64, f64, f64),
    ) {
        if hulls.is_empty() {
            return;
        }
        let (view_x0, view_y0, view_x1, view_y1) = view_bounds;
        let stroke_w = (1.3 / cam_scale.max(0.12)).clamp(1.2, 6.0);
        ctx.set_line_width(stroke_w);
        for hull in hulls {
            if hull.polygon.len() < 3 {
                continue;
            }
            if hull.bounds.2 < view_x0
                || hull.bounds.0 > view_x1
                || hull.bounds.3 < view_y0
                || hull.bounds.1 > view_y1
            {
                continue;
            }
            let color = palette_for_cluster_slot(pal, hull.color_slot);
            ctx.begin_path();
            ctx.move_to(hull.polygon[0].0, hull.polygon[0].1);
            for pt in &hull.polygon[1..] {
                ctx.line_to(pt.0, pt.1);
            }
            ctx.close_path();
            ctx.set_global_alpha(0.09);
            ctx.set_fill_style(color);
            ctx.fill();
            ctx.set_line_dash(&dashes.dashed);
            ctx.set_global_alpha(0.45);
            ctx.set_stroke_style(color);
            ctx.stroke();
        }
        ctx.set_line_dash(&dashes.solid);
        ctx.set_global_alpha(1.0);
    }

    #[cfg(target_arch = "wasm32")]
    #[allow(clippy::too_many_arguments)]
    fn paint_cluster_hull_labels(
        ctx: &wasm_ctx::Ctx2d,
        hulls: &[DenseClusterHull],
        pal: &CanvasPalette,
        cam: Camera,
        dpr: f64,
        vw: f64,
        vh: f64,
        current_font: &mut Option<&'static str>,
    ) {
        if hulls.is_empty() {
            return;
        }
        ctx.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0);
        let pill_font = "600 11px sans-serif";
        if *current_font != Some(pill_font) {
            ctx.set_font(pill_font);
            *current_font = Some(pill_font);
        }
        let placed =
            layout_cluster_hull_labels(hulls, cam, vw, vh, |s| ctx.measure_text(s).width());
        for pill in placed {
            let color = palette_for_cluster_slot(pal, pill.color_slot);
            trace_rounded_rect(ctx, pill.x, pill.y, pill.w, pill.h, 6.0);
            ctx.set_global_alpha(0.92);
            ctx.set_fill_style(&pal.paper);
            ctx.fill();
            ctx.set_global_alpha(0.82);
            ctx.set_line_width(1.2);
            ctx.set_stroke_style(color);
            ctx.stroke();
            ctx.set_global_alpha(1.0);
            ctx.set_fill_style(&pal.ink);
            ctx.fill_text(
                &pill.label,
                pill.x + 7.0,
                pill.y + 13.5,
                (pill.w - 14.0).max(8.0),
            );
        }
        ctx.set_global_alpha(1.0);
    }

    #[cfg(target_arch = "wasm32")]
    struct PreparedEdge<'a> {
        edge: &'a DenseEdgeSpec,
        batch: DenseEdgeBatchKey,
        hollow_arrow: bool,
        active: bool,
        alpha: f64,
        width: f64,
    }

    #[cfg(target_arch = "wasm32")]
    #[allow(clippy::too_many_arguments)]
    fn push_prepared_edge<'a>(
        out: &mut Vec<PreparedEdge<'a>>,
        edge: &'a DenseEdgeSpec,
        meta: &DenseSceneMeta,
        view_bounds: (f64, f64, f64, f64),
        hover: Option<(u8, usize)>,
        hovered_edges: &HashSet<usize>,
        active_kind_hover_lower: Option<&str>,
    ) {
        if !meta.hidden_kinds.is_empty()
            && meta
                .hidden_kinds
                .iter()
                .any(|hk| edge_matches_kind_lower(hk, edge))
        {
            return;
        }
        let Some((ex0, ey0, ex1, ey1)) = meta
            .edge_bounds
            .get(&edge.index)
            .copied()
            .or_else(|| edge_bounds(edge))
        else {
            return;
        };
        let (view_x0, view_y0, view_x1, view_y1) = view_bounds;
        if ex1 < view_x0 || ex0 > view_x1 || ey1 < view_y0 || ey0 > view_y1 {
            return;
        }
        let hovered = hovered_edges.contains(&edge.index);
        let hover_dimmed = hover.is_some() && !hovered;
        let kind_matched =
            active_kind_hover_lower.is_some_and(|hk| edge_matches_kind_lower(hk, edge));
        let kind_dimmed = active_kind_hover_lower.is_some() && !kind_matched;
        let appearance = resolve_dense_edge_stroke(edge, hovered);
        let active = hovered || kind_matched || edge.on || edge.lit;

        let alpha = if (edge.dim || kind_dimmed || hover_dimmed) && !active {
            0.15
        } else if active {
            0.98
        } else if edge.portal && !edge.against {
            0.22
        } else if edge.detour {
            0.50
        } else if meta.commit_changes_active && !appearance.highlighted {
            0.38
        } else if appearance.highlighted {
            0.94
        } else {
            0.76
        };
        let width = if active {
            2.4
        } else if appearance.highlighted || edge.against {
            2.0
        } else {
            (1.0 + (edge.weight.saturating_sub(1) as f64) * 0.25).min(2.6)
        };
        let batch = dense_edge_batch_key(active, &appearance, width, alpha);
        out.push(PreparedEdge {
            edge,
            batch,
            hollow_arrow: appearance.hollow_arrow,
            active,
            alpha,
            width,
        });
    }

    #[cfg(target_arch = "wasm32")]
    #[allow(clippy::too_many_arguments)]
    fn paint_dense_layer(
        ctx: &wasm_ctx::Ctx2d,
        scene: &DenseScene,
        grid: &SpatialHitGrid,
        meta: &DenseSceneMeta,
        pal: &CanvasPalette,
        cam: Camera,
        dpr: f64,
        vw: f64,
        vh: f64,
        target_w: u32,
        target_h: u32,
        hover: Option<(u8, usize)>,
        hovered_neighbors: &HashSet<usize>,
        hovered_edges: &HashSet<usize>,
        active_kind_hover_lower: Option<&str>,
        draw_nodes: bool,
        overlay_only: bool,
    ) {
        if !overlay_only {
            ctx.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
            ctx.clear_rect(0.0, 0.0, target_w as f64, target_h as f64);
        }
        ctx.set_transform(
            dpr * cam.scale,
            0.0,
            0.0,
            dpr * cam.scale,
            dpr * cam.tx,
            dpr * cam.ty,
        );

        let view_x0 = -cam.tx / cam.scale.max(0.05) - 64.0;
        let view_y0 = -cam.ty / cam.scale.max(0.05) - 64.0;
        let view_x1 = view_x0 + vw / cam.scale.max(0.05) + 128.0;
        let view_y1 = view_y0 + vh / cam.scale.max(0.05) + 128.0;
        let view_bounds = (view_x0, view_y0, view_x1, view_y1);
        let draw_arrows = cam.scale >= 0.35;
        let badge_min_scale = 0.55;
        let mut current_font: Option<&'static str> = None;
        let show_cluster_hulls =
            should_render_cluster_hulls(cam.scale) && meta.containers.is_empty();

        with_dash_arrays(|dashes| {
            if !overlay_only {
                paint_dense_containers(
                    ctx,
                    &meta.containers,
                    pal,
                    dashes,
                    cam.scale,
                    view_bounds,
                    &mut current_font,
                );
                if show_cluster_hulls {
                    paint_cluster_hulls_underlay(
                        ctx,
                        &meta.cluster_hulls,
                        pal,
                        dashes,
                        cam.scale,
                        view_bounds,
                    );
                }
            }

            let mut prepared: Vec<PreparedEdge<'_>> = Vec::with_capacity(if overlay_only {
                32
            } else {
                scene.edges.len().min(256)
            });
            if let Some(edge_indices) = grid.visible_edge_indices(
                view_x0 - 8.0,
                view_y0 - 8.0,
                view_x1 + 8.0,
                view_y1 + 8.0,
            ) {
                for edge_idx in edge_indices {
                    if let Some(&pos) = meta.edge_pos_by_idx.get(&edge_idx) {
                        if let Some(edge) = scene.edges.get(pos) {
                            if !overlay_only
                                || is_overlay_edge(edge, hovered_edges, active_kind_hover_lower)
                            {
                                push_prepared_edge(
                                    &mut prepared,
                                    edge,
                                    meta,
                                    view_bounds,
                                    hover,
                                    hovered_edges,
                                    active_kind_hover_lower,
                                );
                            }
                        }
                    }
                }
            } else {
                for edge in &scene.edges {
                    if overlay_only
                        && !is_overlay_edge(edge, hovered_edges, active_kind_hover_lower)
                    {
                        continue;
                    }
                    push_prepared_edge(
                        &mut prepared,
                        edge,
                        meta,
                        view_bounds,
                        hover,
                        hovered_edges,
                        active_kind_hover_lower,
                    );
                }
            }

            prepared.sort_by_key(|p| p.batch);

            let mut open_batch: Option<DenseEdgeBatchKey> = None;
            let mut last_alpha: Option<u8> = None;
            let mut last_tone: Option<DenseEdgeTone> = None;
            let mut last_width: Option<u16> = None;
            let mut last_dash: Option<EdgeDashKind> = None;

            for item in &prepared {
                if open_batch != Some(item.batch) {
                    if open_batch.is_some() {
                        ctx.stroke();
                    }
                    let (_, tone, dash, width_q, alpha_q) = item.batch;
                    if last_alpha != Some(alpha_q) {
                        ctx.set_global_alpha(item.alpha);
                        last_alpha = Some(alpha_q);
                    }
                    if last_tone != Some(tone) {
                        ctx.set_stroke_style(palette_for_tone(pal, tone));
                        last_tone = Some(tone);
                    }
                    if last_width != Some(width_q) {
                        ctx.set_line_width(item.width);
                        last_width = Some(width_q);
                    }
                    if last_dash != Some(dash) {
                        ctx.set_line_dash(match dash {
                            EdgeDashKind::Solid => &dashes.solid,
                            EdgeDashKind::Dashed => &dashes.dashed,
                            EdgeDashKind::Dotted => &dashes.dotted,
                            EdgeDashKind::DashDot => &dashes.dash_dot,
                        });
                        last_dash = Some(dash);
                    }
                    ctx.begin_path();
                    open_batch = Some(item.batch);
                }

                let edge = item.edge;
                if !edge.cmds.is_empty() {
                    for cmd in &edge.cmds {
                        match *cmd {
                            DensePathCmd::Move(x, y) => ctx.move_to(x, y),
                            DensePathCmd::Line(x, y) => ctx.line_to(x, y),
                            DensePathCmd::Curve { c1, c2, to } => {
                                ctx.bezier_curve_to(c1.0, c1.1, c2.0, c2.1, to.0, to.1);
                            }
                        }
                    }
                } else if !edge.pts.is_empty() {
                    ctx.move_to(edge.pts[0].0, edge.pts[0].1);
                    if edge.pts.len() == 4 {
                        ctx.bezier_curve_to(
                            edge.pts[1].0,
                            edge.pts[1].1,
                            edge.pts[2].0,
                            edge.pts[2].1,
                            edge.pts[3].0,
                            edge.pts[3].1,
                        );
                    } else {
                        for p in &edge.pts[1..] {
                            ctx.line_to(p.0, p.1);
                        }
                    }
                }
            }
            if open_batch.is_some() {
                ctx.stroke();
            }

            if last_dash != Some(EdgeDashKind::Solid) {
                ctx.set_line_dash(&dashes.solid);
            }

            if draw_arrows {
                type DenseArrowBatchKey = (bool, DenseEdgeTone, u8);
                type DenseArrowBatchItem = (DenseArrowBatchKey, (f64, f64), (f64, f64), bool);
                let mut arrow_batch: Option<DenseArrowBatchKey> = None;
                let mut arrow_items: Vec<DenseArrowBatchItem> = Vec::new();
                for item in &prepared {
                    if let Some((tip, dir)) = edge_end_tangent(item.edge) {
                        arrow_items.push((
                            (item.hollow_arrow, item.batch.1, item.batch.4),
                            tip,
                            dir,
                            item.active,
                        ));
                    }
                }
                arrow_items.sort_by_key(|row| row.0);
                for (key, tip, dir, active) in arrow_items {
                    let (hollow, tone, alpha_q) = key;
                    let len = if active { 7.5 } else { 6.5 };
                    let half_w = if active { 3.4 } else { 2.9 };
                    let base_x = tip.0 - dir.0 * len;
                    let base_y = tip.1 - dir.1 * len;
                    let nx = -dir.1 * half_w;
                    let ny = dir.0 * half_w;
                    if hollow {
                        if arrow_batch.take().is_some() {
                            ctx.fill();
                        }
                        ctx.set_global_alpha(f64::from(alpha_q) / 100.0);
                        ctx.set_stroke_style(palette_for_tone(pal, tone));
                        ctx.set_fill_style(&pal.paper);
                        ctx.set_line_width(1.2);
                        ctx.begin_path();
                        ctx.move_to(tip.0, tip.1);
                        ctx.line_to(base_x + nx, base_y + ny);
                        ctx.line_to(base_x - nx, base_y - ny);
                        ctx.close_path();
                        ctx.fill();
                        ctx.stroke();
                    } else {
                        if arrow_batch != Some(key) {
                            if arrow_batch.is_some() {
                                ctx.fill();
                            }
                            ctx.set_global_alpha(f64::from(alpha_q) / 100.0);
                            ctx.set_fill_style(palette_for_tone(pal, tone));
                            ctx.begin_path();
                            arrow_batch = Some(key);
                        }
                        ctx.move_to(tip.0, tip.1);
                        ctx.line_to(base_x + nx, base_y + ny);
                        ctx.line_to(base_x - nx, base_y - ny);
                        ctx.close_path();
                    }
                }
                if arrow_batch.is_some() {
                    ctx.fill();
                }
            }

            for item in &prepared {
                let edge = item.edge;
                if edge.weight > 1 && (item.active || cam.scale >= badge_min_scale) {
                    if let Some((mx, my)) = edge_midpoint(edge) {
                        let stroke_col = palette_for_tone(pal, item.batch.1);
                        let label = edge.weight.to_string();
                        let bw = ((label.len() as f64) * 6.2 + 8.0).max(16.0);
                        let bh = 14.0;
                        let bx = mx - bw * 0.5;
                        let by = my - bh * 0.5;
                        ctx.set_global_alpha(item.alpha);
                        ctx.set_fill_style(&pal.paper);
                        ctx.fill_rect(bx, by, bw, bh);
                        ctx.set_line_width(1.0);
                        ctx.set_stroke_style(stroke_col);
                        ctx.stroke_rect(bx, by, bw, bh);
                        if current_font != Some("600 9.5px sans-serif") {
                            ctx.set_font("600 9.5px sans-serif");
                            current_font = Some("600 9.5px sans-serif");
                        }
                        ctx.set_fill_style(stroke_col);
                        ctx.fill_text(&label, bx + 4.0, by + 10.5, bw - 8.0);
                    }
                }
            }
        });

        if draw_nodes {
            let show_labels = should_show_dense_labels(cam.scale, meta.label_min_scale);
            let mut paint_node = |node: &DenseNodeSpec| {
                if node.x + node.w < view_x0
                    || node.x > view_x1
                    || node.y + node.h < view_y0
                    || node.y > view_y1
                {
                    return;
                }
                if overlay_only && !is_overlay_node(node, hover, hovered_neighbors) {
                    return;
                }
                let nm = meta.nodes.get(&node.index);
                let in_cycle = nm.is_some_and(|m| m.in_cycle);
                let has_blast = nm.is_some_and(|m| !m.blast_tag.is_empty());
                let layer_dim = meta.active_layer.as_deref().is_some_and(|al| {
                    meta.layer_of_node.get(&node.index).map(String::as_str) != Some(al)
                });
                let hovered =
                    hover == Some((0, node.index)) || hovered_neighbors.contains(&node.index);
                let hover_dimmed = hover.is_some() && !hovered;
                ctx.set_global_alpha(
                    if (node.dim || hover_dimmed || layer_dim)
                        && !hovered
                        && !node.selected
                        && !node.lit
                    {
                        0.32
                    } else {
                        0.96
                    },
                );
                ctx.set_fill_style(&pal.paper);
                ctx.fill_rect(node.x, node.y, node.w, node.h);

                let border_col = if node.failed {
                    &pal.against
                } else if in_cycle {
                    &pal.warn
                } else if node.selected || hovered || node.lit {
                    &pal.purple
                } else if node.changed {
                    &pal.changed
                } else if node.pinned {
                    &pal.warn
                } else {
                    &pal.line
                };
                ctx.set_line_width(if node.selected || hovered { 2.0 } else { 1.1 });
                ctx.set_stroke_style(border_col);
                ctx.stroke_rect(node.x, node.y, node.w, node.h);

                let accent_col = if node.failed {
                    Some(&pal.against)
                } else if in_cycle || has_blast {
                    Some(&pal.warn)
                } else if node.changed {
                    Some(&pal.changed)
                } else if node.pinned {
                    Some(&pal.diff_touched)
                } else {
                    None
                };
                let label_left_pad = if let Some(col) = accent_col {
                    ctx.set_fill_style(col);
                    ctx.fill_rect(node.x + 1.0, node.y + 1.0, 3.0, (node.h - 2.0).max(2.0));
                    9.0
                } else {
                    8.0
                };

                let mut right_reserve = 8.0;
                if show_labels && cam.scale >= badge_min_scale {
                    if let Some(badge) = nm.and_then(dense_node_badge_label) {
                        if current_font != Some("600 9px sans-serif") {
                            ctx.set_font("600 9px sans-serif");
                            current_font = Some("600 9px sans-serif");
                        }
                        let text_w =
                            badge_width_cached_with(&badge, |b| ctx.measure_text(b).width());
                        let pill_w = (text_w + 8.0).clamp(18.0, 44.0);
                        if node.w - label_left_pad - pill_w - 10.0 >= 28.0 {
                            let pill_h = 13.0;
                            let px = node.x + node.w - pill_w - 5.0;
                            let py = node.y + (node.h - pill_h) * 0.5;
                            ctx.set_fill_style(&pal.soft);
                            ctx.fill_rect(px, py, pill_w, pill_h);
                            ctx.set_line_width(0.8);
                            ctx.set_stroke_style(border_col);
                            ctx.stroke_rect(px, py, pill_w, pill_h);
                            ctx.set_fill_style(&pal.muted);
                            ctx.fill_text(&badge, px + 4.0, py + 9.5, (pill_w - 8.0).max(4.0));
                            right_reserve = pill_w + 10.0;
                        }
                    }
                }

                if show_labels && !node.label.is_empty() {
                    if current_font != Some("500 11px sans-serif") {
                        ctx.set_font("500 11px sans-serif");
                        current_font = Some("500 11px sans-serif");
                    }
                    ctx.set_fill_style(if node.dim && !node.selected {
                        &pal.muted
                    } else {
                        &pal.ink
                    });
                    let avail_w = (node.w - label_left_pad - right_reserve).max(12.0);
                    let fitted =
                        fit_label_cached_with(&node.label, avail_w, "500 11px sans-serif", |s| {
                            ctx.measure_text(s).width()
                        });
                    ctx.fill_text_unscaled(
                        &fitted,
                        node.x + label_left_pad,
                        node.y + node.h * 0.62,
                    );
                }
            };
            if let Some(node_indices) =
                grid.visible_node_indices(view_x0, view_y0, view_x1, view_y1)
            {
                for node_idx in node_indices {
                    if let Some(&pos) = meta.node_pos_by_idx.get(&node_idx) {
                        if let Some(node) = scene.nodes.get(pos) {
                            paint_node(node);
                        }
                    }
                }
            } else {
                for node in &scene.nodes {
                    paint_node(node);
                }
            }
        }
        if should_paint_cluster_hull_labels(overlay_only, show_cluster_hulls) {
            paint_cluster_hull_labels(
                ctx,
                &meta.cluster_hulls,
                pal,
                cam,
                dpr,
                vw,
                vh,
                &mut current_font,
            );
        }
        ctx.set_global_alpha(1.0);
    }

    #[allow(unused_variables)]
    pub fn paint_scene(canvas_id: &str, cam: Camera, hover: Option<(u8, usize)>, draw_nodes: bool) {
        #[cfg(target_arch = "wasm32")]
        {
            ensure_theme_watchers();
            let empty_neighbors: HashSet<usize> = HashSet::new();
            let empty_edges: HashSet<usize> = HashSet::new();
            let painted = DENSE_SCENES.with(|store| {
                {
                    let mut mut_guard = store.borrow_mut();
                    if let Some((scene, _, meta)) = mut_guard.get_mut(canvas_id) {
                        let missing_bounds = meta.edge_bounds.len() < scene.edges.len();
                        let missing_adj = !meta.edges.is_empty() && meta.node_adjacency.is_empty();
                        if missing_bounds || missing_adj {
                            meta.rebuild_caches(scene);
                            let digest = compute_scene_digest(scene, meta);
                            SCENE_DIGESTS.with(|d| {
                                d.borrow_mut().insert(canvas_id.to_owned(), digest);
                            });
                            invalidate_offscreen_base_layer(canvas_id);
                        }
                    }
                }
                let guard = store.borrow();
                let (scene, grid, meta) = guard.get(canvas_id)?;
                let host = dom::get(canvas_id)?;
                if !cam.scale.is_finite() || cam.scale <= 0.0 {
                    return None;
                }
                if host.scroll_left() != 0 {
                    host.set_scroll_left(0);
                }
                if host.scroll_top() != 0 {
                    host.set_scroll_top(0);
                }
                ensure_resize_watcher(&host);
                let canvas_el = match host.query_selector("canvas[data-dense-canvas='true']") {
                    Ok(Some(existing)) => existing,
                    _ => {
                        let created = dom::document().create_element("canvas").ok()?;
                        let _ = created.set_attribute("class", "graph-dense-canvas");
                        let _ = created.set_attribute("data-dense-canvas", "true");
                        let _ = created.set_attribute("aria-hidden", "true");
                        let _ = host.prepend_with_node_1(&created);
                        created
                    }
                };
                let (vw, vh, dpr, target_w, target_h) =
                    measure_dense_canvas_box(Some(&canvas_el), canvas_id)?;
                LAST_CAMERAS.with(|c| {
                    c.borrow_mut().insert(canvas_id.to_owned(), cam);
                });
                let canvas_js: wasm_ctx::CanvasEl = canvas_el.clone().unchecked_into();
                if canvas_js.width() != target_w {
                    canvas_js.set_width(target_w);
                }
                if canvas_js.height() != target_h {
                    canvas_js.set_height(target_h);
                }
                let ctx = canvas_js.get_context("2d")?;

                let pal = resolve_palette(canvas_id, &host);
                let host_kind_hover = host
                    .get_attribute("data-kind-hover")
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty());
                let active_kind_hover_lower = host_kind_hover
                    .as_deref()
                    .or(meta.kind_hover.as_deref())
                    .map(|s| s.trim().to_ascii_lowercase())
                    .filter(|s| !s.is_empty());

                let scene_digest = SCENE_DIGESTS
                    .with(|d| d.borrow().get(canvas_id).copied())
                    .unwrap_or_else(|| compute_scene_digest(scene, meta));
                let palette_epoch = PALETTE_THEME_EPOCH.with(|e| e.get());
                let base_key = build_offscreen_base_key(
                    scene_digest,
                    dpr,
                    vw,
                    vh,
                    cam,
                    palette_epoch,
                    draw_nodes,
                    active_kind_hover_lower.as_deref(),
                    meta.active_layer.as_deref(),
                    &meta.hidden_kinds,
                );

                // Ensure offscreen static base canvas matches `base_key` (hover-invariant).
                let base_drawn = OFFSCREEN_BASE_LAYERS.with(|layers| {
                    let mut layers_mut = layers.borrow_mut();
                    let need_rebuild = !should_reuse_offscreen_base(
                        layers_mut.get(canvas_id).map(|l| &l.key),
                        &base_key,
                    );
                    if need_rebuild {
                        let base_canvas: wasm_ctx::CanvasEl =
                            if let Some(existing) = layers_mut.get(canvas_id) {
                                existing.canvas.clone().unchecked_into()
                            } else {
                                let Ok(el) = dom::document().create_element("canvas") else {
                                    return None;
                                };
                                el.unchecked_into()
                            };
                        if base_canvas.width() != target_w {
                            base_canvas.set_width(target_w);
                        }
                        if base_canvas.height() != target_h {
                            base_canvas.set_height(target_h);
                        }
                        let base_ctx = base_canvas.get_context("2d")?;
                        paint_dense_layer(
                            &base_ctx,
                            scene,
                            grid,
                            meta,
                            &pal,
                            cam,
                            dpr,
                            vw,
                            vh,
                            target_w,
                            target_h,
                            None,
                            &empty_neighbors,
                            &empty_edges,
                            active_kind_hover_lower.as_deref(),
                            draw_nodes,
                            false,
                        );
                        layers_mut.insert(
                            canvas_id.to_owned(),
                            OffscreenBaseLayer {
                                key: base_key.clone(),
                                canvas: base_canvas.clone().unchecked_into(),
                            },
                        );
                        Some(base_canvas)
                    } else {
                        layers_mut
                            .get(canvas_id)
                            .map(|l| l.canvas.clone().unchecked_into())
                    }
                });

                if let Some(ref base_canvas) = base_drawn {
                    ctx.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
                    ctx.clear_rect(0.0, 0.0, target_w as f64, target_h as f64);
                    ctx.set_global_alpha(if hover.is_some() { 0.26 } else { 1.0 });
                    ctx.draw_image(base_canvas, 0.0, 0.0, target_w as f64, target_h as f64);
                    ctx.set_global_alpha(1.0);
                }

                if hover.is_some() || base_drawn.is_none() {
                    let mut hovered_neighbors: HashSet<usize> = HashSet::new();
                    let mut hovered_edges: HashSet<usize> = HashSet::new();
                    match hover {
                        Some((0, h_node)) => {
                            hovered_neighbors.insert(h_node);
                            if let Some(adj) = meta.node_adjacency.get(&h_node) {
                                for &(edge_idx, neighbor) in adj {
                                    hovered_edges.insert(edge_idx);
                                    hovered_neighbors.insert(neighbor);
                                }
                            }
                        }
                        Some((1, h_edge)) => {
                            hovered_edges.insert(h_edge);
                            if let Some(em) = meta.edges.get(&h_edge) {
                                if let Some(from) = em.from_index {
                                    hovered_neighbors.insert(from);
                                }
                                if let Some(to) = em.to_index {
                                    hovered_neighbors.insert(to);
                                }
                            }
                        }
                        _ => {}
                    }
                    paint_dense_layer(
                        &ctx,
                        scene,
                        grid,
                        meta,
                        &pal,
                        cam,
                        dpr,
                        vw,
                        vh,
                        target_w,
                        target_h,
                        hover,
                        &hovered_neighbors,
                        &hovered_edges,
                        active_kind_hover_lower.as_deref(),
                        draw_nodes,
                        base_drawn.is_some(),
                    );
                }
                Some((target_w, target_h))
            });
            if let Some((target_w, target_h)) = painted {
                LAST_PAINTED_VIEWPORT.with(|v| {
                    v.borrow_mut()
                        .insert(canvas_id.to_owned(), (target_w, target_h));
                });
                LAST_PAINT_STATE.with(|p| {
                    p.borrow_mut()
                        .insert(canvas_id.to_owned(), (cam, hover, draw_nodes));
                });
            }
        }
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
        assert!(
            style.ends_with("transform-origin:0 0;"),
            "plane_transform_style must terminate transform-origin with `;` so transition:none applies: {style}"
        );
        let dense_style = format!("{style}transition:none;");
        assert!(dense_style.contains("transform-origin:0 0;transition:none;"));
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

    #[test]
    fn shared_pin_store_bfs_path_hop_cone_roving_nav_and_spatial_grid() {
        let key = pin_storage_key("session", "proj/main", "whole");
        clear_pins(&key);
        assert!(!has_pins(&key));
        save_pin(&key, "src/a.rs", (120.0, 240.0));
        assert!(has_pins(&key));
        let mut positions = vec![(10.0, 20.0), (30.0, 40.0)];
        let keys = vec!["src/a.rs".to_owned(), "src/b.rs".to_owned()];
        let loaded = load_pins(&key);
        let parsed = parse_pins_json(&serde_json::json!({"src/a.rs": {"x": 120.0, "y": 240.0}}));
        assert_eq!(parsed.get("src/a.rs"), Some(&(120.0, 240.0)));
        assert_eq!(merge_pin_positions(&keys, &mut positions, &loaded), 1);
        assert_eq!(positions[0], (120.0, 240.0));
        assert_eq!(positions[1], (30.0, 40.0));
        clear_pins(&key);
        assert!(!has_pins(&key));

        let out = vec![vec![1], vec![2, 3], vec![4], vec![], vec![]];
        let inc = vec![vec![], vec![0], vec![1], vec![1], vec![2]];
        assert_eq!(
            shortest_path_bfs(5, &out, false, 0, 4),
            Some(vec![0, 1, 2, 4])
        );
        assert_eq!(shortest_path_bfs(5, &out, false, 4, 0), None);
        assert_eq!(
            shortest_path_bfs(5, &out, true, 4, 0),
            Some(vec![4, 2, 1, 0])
        );

        let cone = hop_cone_distances(5, &out, &inc, &[1], 1);
        assert_eq!(cone, vec![Some(1), Some(0), Some(1), Some(1), None]);

        let rects = vec![
            (0.0, 0.0, 100.0, 30.0),
            (200.0, 0.0, 100.0, 30.0),
            (200.0, 120.0, 100.0, 30.0),
        ];
        assert_eq!(
            next_roving_node_index(&rects, &[1], 0, "ArrowRight"),
            Some(1)
        );
        assert_eq!(next_roving_node_index(&rects, &[], 1, "ArrowDown"), Some(2));
        assert_eq!(next_roving_node_index(&rects, &[], 2, "Home"), Some(0));
        assert_eq!(next_roving_node_index(&rects, &[], 0, "End"), Some(2));

        let scene = canvas2d::DenseScene {
            width: 500.0,
            height: 400.0,
            nodes: vec![canvas2d::DenseNodeSpec {
                index: 3,
                x: 40.0,
                y: 50.0,
                w: 120.0,
                h: 32.0,
                label: "NodeA".to_owned(),
                role: "file".to_owned(),
                selected: false,
                lit: false,
                dim: false,
                changed: false,
                pinned: false,
                failed: false,
            }],
            edges: vec![canvas2d::DenseEdgeSpec {
                index: 7,
                pts: vec![(160.0, 66.0), (300.0, 200.0)],
                cmds: Vec::new(),
                weight: 2,
                guessed: false,
                against: false,
                kind: "calls".to_owned(),
                on: false,
                lit: false,
                dim: false,
                portal: false,
                ..Default::default()
            }],
        };
        let grid = canvas2d::SpatialHitGrid::build(&scene, 64.0);
        assert_eq!(grid.hit_test(90.0, 65.0, 6.0), Some((0, 3)));
        assert_eq!(grid.hit_test(230.0, 133.0, 6.0), Some((1, 7)));
        assert_eq!(grid.hit_test(10.0, 10.0, 6.0), None);
    }

    #[test]
    fn dense_edge_preserves_multi_segment_bezier_and_hit_grid_covers_pad_and_curves() {
        let path_d = "M0.0 0.0 C100.0 0.0 100.0 100.0 200.0 100.0 L300.0 100.0 C350.0 100.0 350.0 200.0 400.0 200.0";
        let edge = canvas2d::DenseEdgeSpec::from_svg_path(
            42,
            path_d,
            1,
            false,
            false,
            "calls".to_owned(),
            false,
            false,
            false,
            false,
        );
        assert_eq!(
            edge.cmds,
            vec![
                canvas2d::DensePathCmd::Move(0.0, 0.0),
                canvas2d::DensePathCmd::Curve {
                    c1: (100.0, 0.0),
                    c2: (100.0, 100.0),
                    to: (200.0, 100.0),
                },
                canvas2d::DensePathCmd::Line(300.0, 100.0),
                canvas2d::DensePathCmd::Curve {
                    c1: (350.0, 100.0),
                    c2: (350.0, 200.0),
                    to: (400.0, 200.0),
                },
            ]
        );
        assert!(edge.pts.len() > 16);

        // Node aligned exactly on a 128px grid cell boundary (128.0) must still hit within 2.0px pad at 126.5!
        let scene = canvas2d::DenseScene {
            width: 500.0,
            height: 300.0,
            nodes: vec![canvas2d::DenseNodeSpec {
                index: 1,
                x: 128.0,
                y: 128.0,
                w: 127.0,
                h: 30.0,
                label: "CellEdge".to_owned(),
                role: "file".to_owned(),
                selected: false,
                lit: false,
                dim: false,
                changed: false,
                pinned: false,
                failed: false,
            }],
            edges: vec![edge],
        };
        let grid = canvas2d::SpatialHitGrid::build(&scene, 128.0);
        // 1.5px outside left/top cell boundary (cell 0) and 1.5px outside right/bottom boundary
        assert_eq!(grid.hit_test(126.5, 140.0, 6.0), Some((0, 1)));
        assert_eq!(grid.hit_test(140.0, 126.5, 6.0), Some((0, 1)));
        assert_eq!(grid.hit_test(256.5, 140.0, 6.0), Some((0, 1)));
        assert_eq!(grid.hit_test(140.0, 159.5, 6.0), Some((0, 1)));
        // Midpoint of first cubic Bezier (t=0.5 -> (100.0, 50.0)) hits
        assert_eq!(grid.hit_test(100.0, 50.0, 6.0), Some((1, 42)));
        // Straight L segment (250.0, 100.0) hits
        assert_eq!(grid.hit_test(250.0, 100.0, 6.0), Some((1, 42)));
        // Second cubic Bezier midpoint (350.0, 150.0) hits
        assert_eq!(grid.hit_test(350.0, 150.0, 6.0), Some((1, 42)));
        // Raw control point (100.0, 0.0) is >30px off the actual curve, so it must NOT hit!
        assert_eq!(grid.hit_test(100.0, 0.0, 6.0), None);
    }

    #[test]
    fn viewport_reconcile_mounts_on_pan_with_hysteresis_and_keeps_forced_nodes() {
        let rects = vec![
            (10.0, 10.0, 160.0, 30.0),   // 0: initially visible
            (900.0, 10.0, 160.0, 30.0),  // 1: offscreen right, revealed on pan
            (2500.0, 10.0, 160.0, 30.0), // 2: far offscreen, forced (roving/selected)
            (-600.0, 10.0, 160.0, 30.0), // 3: far left, outside unmount bounds
        ];
        // Initial mount only had node 0 and node 3 mounted (node 2 forced)
        let mounted = vec![(0, false), (3, false)];
        // Pan right so viewport covers x in [800, 1400]
        let plan = plan_viewport_reconcile(
            &rects,
            &mounted,
            (800.0, 0.0, 1400.0, 600.0),
            (700.0, -100.0, 1500.0, 700.0),
            (460.0, -340.0, 1740.0, 940.0),
            &[2],
        );
        assert_eq!(plan.to_mount, vec![1, 2]);
        assert_eq!(plan.to_unmount, vec![0, 3]);
    }

    #[test]
    fn radar_viewport_rect_clamps_without_overhang_or_tiny_plane_panic() {
        // Far-right / far-bottom pan clamps inside plane bounds (`vx + vw <= plane.0`).
        let rect = radar_viewport_rect(
            Camera::new(-995.0, -495.0, 1.0),
            (200.0, 100.0),
            (1000.0, 500.0),
            24.0,
        )
        .expect("valid rect");
        assert!(rect.0 + rect.2 <= 1000.0 + 1e-6);
        assert!(rect.1 + rect.3 <= 500.0 + 1e-6);
        assert!(rect.2 >= 24.0);
        assert!(rect.3 >= 24.0);

        // Plane smaller than `min_side` must never panic on `clamp(min, max)`.
        let tiny = radar_viewport_rect(
            Camera::new(-40.0, -40.0, 1.0),
            (300.0, 200.0),
            (10.0, 12.0),
            24.0,
        )
        .expect("tiny plane rect");
        assert_eq!(tiny, (0.0, 0.0, 10.0, 12.0));
    }

    #[test]
    fn pan_to_keep_rect_in_view_and_escape_css_attr_work() {
        let cam = Camera::new(0.0, 0.0, 1.0);
        assert_eq!(
            pan_to_keep_rect_in_view(cam, (100.0, 80.0, 140.0, 32.0), (800.0, 500.0), 24.0),
            None
        );
        // Node clipped off the right edge shifts camera left just enough to reveal it + margin.
        let panned =
            pan_to_keep_rect_in_view(cam, (900.0, 80.0, 140.0, 32.0), (800.0, 500.0), 24.0)
                .expect("should pan right-clipped node into view");
        assert_eq!(panned.scale, 1.0);
        assert!(900.0 + 140.0 + panned.tx <= 800.0 - 24.0 + 1.0);
        assert!(panned.dy_or_zero(cam) == 0.0);

        assert_eq!(
            escape_css_attr_value("src/a\"b\\c\n"),
            "src/a\\\"b\\\\c\\a "
        );
    }

    #[test]
    fn camera_selection_history_tracks_back_forward_dedup_and_breadcrumbs() {
        let mut hist = CameraSelectionHistory::new(3);
        assert!(!hist.can_back());
        assert!(!hist.can_forward());

        hist.push(CameraSelectionEntry {
            key: "".into(),
            label: "Root".into(),
            target: "work".into(),
            cam: Camera::new(0.0, 0.0, 1.0),
        });
        hist.push(CameraSelectionEntry {
            key: "a.rs".into(),
            label: "a.rs".into(),
            target: "work".into(),
            cam: Camera::new(-50.0, -20.0, 1.2),
        });
        // Same target + key updates camera instead of pushing duplicate entry.
        hist.push(CameraSelectionEntry {
            key: "a.rs".into(),
            label: "a.rs".into(),
            target: "work".into(),
            cam: Camera::new(-80.0, -30.0, 1.5),
        });
        assert_eq!(hist.entries().len(), 2);
        assert!(hist.can_back());
        assert!(!hist.can_forward());

        let back = hist.back().expect("back entry").clone();
        assert_eq!(back.label, "Root");
        assert!(hist.can_forward());

        // Pushing from back truncates forward branch and honors capacity 3.
        hist.push(CameraSelectionEntry {
            key: "b.rs".into(),
            label: "b.rs".into(),
            target: "work".into(),
            cam: Camera::new(-10.0, -10.0, 1.0),
        });
        assert!(!hist.can_forward());
        hist.push(CameraSelectionEntry {
            key: "c.rs".into(),
            label: "c.rs".into(),
            target: "work".into(),
            cam: Camera::new(-20.0, -20.0, 1.0),
        });
        hist.push(CameraSelectionEntry {
            key: "d.rs".into(),
            label: "d.rs".into(),
            target: "work".into(),
            cam: Camera::new(-30.0, -30.0, 1.0),
        });
        assert_eq!(hist.entries().len(), 3);
        assert_eq!(hist.entries()[0].key, "b.rs");
        assert_eq!(hist.jump_to(1).expect("jump").key, "c.rs");
        assert!(hist.can_back());
        assert!(hist.can_forward());
    }

    #[test]
    fn canvas2d_styles_tangents_hidden_kinds_and_explain_meta_work() {
        let calls = canvas2d::edge_visual_style("calls imports", false, false);
        assert_eq!(calls.color, canvas2d::EdgeColorSlot::Calls);
        assert_eq!(calls.dash, canvas2d::EdgeDashKind::Solid);
        assert!(!calls.hollow_arrow);

        let impl_edge = canvas2d::edge_visual_style("implements", false, false);
        assert_eq!(impl_edge.color, canvas2d::EdgeColorSlot::Implements);
        assert_eq!(impl_edge.dash, canvas2d::EdgeDashKind::DashDot);
        assert!(impl_edge.hollow_arrow);

        let against = canvas2d::edge_visual_style("calls", false, true);
        assert_eq!(against.color, canvas2d::EdgeColorSlot::Against);
        assert_eq!(against.dash, canvas2d::EdgeDashKind::Dashed);

        assert!(canvas2d::edge_kind_matches("imports", "calls imports"));
        assert!(!canvas2d::edge_kind_matches("writes", "calls imports"));
        assert!(!canvas2d::should_show_dense_labels(0.45, None));
        assert!(canvas2d::should_show_dense_labels(0.50, None));

        let edge = canvas2d::DenseEdgeSpec::from_svg_path(
            9,
            "M 0 0 L 100 0 L 100 50",
            4,
            false,
            false,
            "imports".to_owned(),
            false,
            false,
            false,
            false,
        );
        let (tip, dir) = canvas2d::edge_end_tangent(&edge).expect("tangent");
        assert!((tip.0 - 100.0).abs() < 1e-6 && (tip.1 - 50.0).abs() < 1e-6);
        assert!(dir.0.abs() < 1e-6 && (dir.1 - 1.0).abs() < 1e-6);
        assert!(canvas2d::edge_midpoint(&edge).is_some());

        let scene = canvas2d::DenseScene {
            width: 300.0,
            height: 200.0,
            nodes: vec![],
            edges: vec![edge],
        };
        let mut meta = canvas2d::DenseSceneMeta::default();
        meta.edges.insert(
            9,
            canvas2d::DenseEdgeMeta {
                explain_kind: "link".to_owned(),
                explain_payload: json!({"from": "a.rs", "to": "b.rs"}),
                from_index: Some(0),
                to_index: Some(1),
                ..Default::default()
            },
        );
        canvas2d::store_scene_with_meta("test-dense-meta", scene, meta);
        assert_eq!(
            canvas2d::hit_test_scene("test-dense-meta", 100.0, 25.0, 6.0),
            Some((1, 9))
        );
        assert_eq!(
            canvas2d::explain_target_for_hit("test-dense-meta", 1, 9),
            Some(("link".to_owned(), json!({"from": "a.rs", "to": "b.rs"})))
        );
        let mut hidden = HashSet::new();
        hidden.insert("imports".to_owned());
        assert!(canvas2d::set_hidden_kinds("test-dense-meta", hidden));
        assert_eq!(
            canvas2d::hit_test_scene("test-dense-meta", 100.0, 25.0, 6.0),
            None
        );
        canvas2d::clear_scene("test-dense-meta");
    }

    #[test]
    fn dense_edge_stroke_preserves_risk_and_diff_colors_over_selection() {
        let against = canvas2d::DenseEdgeSpec::from_svg_path(
            0,
            "M 0 0 L 50 50",
            1,
            false,
            true,
            "calls".to_owned(),
            true,
            true,
            false,
            false,
        );
        let app = canvas2d::resolve_dense_edge_stroke(&against, true);
        assert_eq!(app.tone, canvas2d::DenseEdgeTone::Against);
        assert_eq!(app.dash, canvas2d::EdgeDashKind::Dashed);

        let risk = canvas2d::DenseEdgeSpec::from_svg_path(
            1,
            "M 0 0 L 50 50",
            1,
            false,
            false,
            "calls".to_owned(),
            true,
            false,
            false,
            false,
        )
        .with_semantics("", "broken_risk", false, false, false, Vec::new());
        assert_eq!(
            canvas2d::resolve_dense_edge_stroke(&risk, true).tone,
            canvas2d::DenseEdgeTone::ImpactRisk
        );

        let added = canvas2d::DenseEdgeSpec::from_svg_path(
            2,
            "M 0 0 L 50 50",
            1,
            false,
            false,
            "calls".to_owned(),
            true,
            false,
            false,
            false,
        )
        .with_semantics("added", "", false, false, false, vec!["uses".to_owned()]);
        assert_eq!(
            canvas2d::resolve_dense_edge_stroke(&added, false).tone,
            canvas2d::DenseEdgeTone::DiffAdded
        );
        assert!(canvas2d::edge_matches_kind("uses", &added));
        assert!(canvas2d::edge_matches_kind("calls", &added));
    }

    #[test]
    fn update_scene_and_grid_tracks_shift_drag_moves_and_rebuilds_edge_bounds() {
        let scene = canvas2d::DenseScene {
            width: 800.0,
            height: 600.0,
            nodes: vec![
                canvas2d::DenseNodeSpec {
                    index: 0,
                    x: 40.0,
                    y: 40.0,
                    w: 100.0,
                    h: 30.0,
                    label: "A".to_owned(),
                    role: "file".to_owned(),
                    selected: false,
                    lit: false,
                    dim: false,
                    changed: false,
                    pinned: false,
                    failed: false,
                },
                canvas2d::DenseNodeSpec {
                    index: 1,
                    x: 400.0,
                    y: 40.0,
                    w: 100.0,
                    h: 30.0,
                    label: "B".to_owned(),
                    role: "file".to_owned(),
                    selected: false,
                    lit: false,
                    dim: false,
                    changed: false,
                    pinned: false,
                    failed: false,
                },
            ],
            edges: vec![canvas2d::DenseEdgeSpec::from_svg_path(
                0,
                "M 140 55 L 400 55",
                1,
                false,
                false,
                "calls".to_owned(),
                false,
                false,
                false,
                false,
            )],
        };
        let mut meta = canvas2d::DenseSceneMeta::default();
        meta.edges.insert(
            0,
            canvas2d::DenseEdgeMeta {
                explain_kind: "link".to_owned(),
                explain_payload: json!({"from": "a", "to": "b"}),
                from_index: Some(0),
                to_index: Some(1),
                summary: "A -> B".to_owned(),
            },
        );
        canvas2d::store_scene_with_meta("test-shift-grid", scene, meta);
        assert_eq!(
            canvas2d::hit_test_scene("test-shift-grid", 90.0, 55.0, 6.0),
            Some((0, 0))
        );
        assert_eq!(
            canvas2d::hit_test_scene("test-shift-grid", 260.0, 55.0, 6.0),
            Some((1, 0))
        );
        assert!(canvas2d::update_scene_and_grid(
            "test-shift-grid",
            &[0],
            &[0],
            |s| {
                s.nodes[0].x = 40.0;
                s.nodes[0].y = 300.0;
                s.edges[0].set_svg_path("M 140 315 L 400 55");
            }
        ));
        assert_eq!(
            canvas2d::hit_test_scene("test-shift-grid", 90.0, 55.0, 6.0),
            None
        );
        assert_eq!(
            canvas2d::hit_test_scene("test-shift-grid", 90.0, 315.0, 6.0),
            Some((0, 0))
        );
        assert_eq!(
            canvas2d::hit_test_scene("test-shift-grid", 270.0, 185.0, 8.0),
            Some((1, 0))
        );
        assert_eq!(
            canvas2d::hit_test_tooltip_text("test-shift-grid", 1, 0).as_deref(),
            Some("A -> B")
        );
        canvas2d::clear_scene("test-shift-grid");
    }

    #[test]
    fn scale_tol_label_ellipsis_and_pin_drag_session_work_across_scales() {
        assert!((dense_dom::scale_normalized_hit_tol(8.0, 0.5) - 16.0).abs() < 1e-6);
        assert!((dense_dom::scale_normalized_hit_tol(8.0, 0.1) - 32.0).abs() < 1e-6);
        assert!((dense_dom::scale_normalized_hit_tol(8.0, 2.0) - 4.0).abs() < 1e-6);

        let fitted = canvas2d::fit_text_with_ellipsis("VeryLongModuleServiceClass", 80.0, |s| {
            (s.chars().count() as f64) * 7.0
        });
        assert!(fitted.ends_with('…'));
        assert!((fitted.chars().count() as f64) * 7.0 <= 80.0);

        let mut drag =
            dense_dom::PinDragSession::begin(5, 2, "src/App.php", (100.0, 100.0), (50.0, 60.0));
        assert_eq!(
            drag.advance(5, (101.0, 101.0), 0.5, (0.0, 0.0, 500.0, 400.0), 5.0),
            None
        );
        let moved = drag
            .advance(5, (130.0, 140.0), 0.5, (0.0, 0.0, 500.0, 400.0), 5.0)
            .expect("exceeded drag threshold");
        assert!((moved.0 - 110.0).abs() < 1e-6);
        assert!((moved.1 - 140.0).abs() < 1e-6);
        let roundtrip = dense_dom::PinDragSession::from_json(&drag.to_json());
        assert_eq!(roundtrip, drag);
        assert_eq!(
            drag.end(Some(5), false),
            Some(dense_dom::PinDragOutcome::Committed {
                node_idx: 2,
                node_key: "src/App.php".to_owned(),
                pos: (110.0, 140.0),
            })
        );
    }

    #[test]
    fn canvas2d_batch_keys_group_identical_styles_and_cache_ellipsized_labels() {
        let e1 = canvas2d::DenseEdgeSpec {
            kind: "calls".into(),
            ..canvas2d::DenseEdgeSpec::default()
        };
        let e2 = canvas2d::DenseEdgeSpec {
            kind: "calls".into(),
            ..canvas2d::DenseEdgeSpec::default()
        };
        let e3 = canvas2d::DenseEdgeSpec {
            kind: "calls".into(),
            against: true,
            ..canvas2d::DenseEdgeSpec::default()
        };
        let a1 = canvas2d::resolve_dense_edge_stroke(&e1, false);
        let a2 = canvas2d::resolve_dense_edge_stroke(&e2, false);
        let a3 = canvas2d::resolve_dense_edge_stroke(&e3, false);
        assert_eq!(
            canvas2d::dense_edge_batch_key(false, &a1, 1.0, 0.76),
            canvas2d::dense_edge_batch_key(false, &a2, 1.0, 0.76)
        );
        assert_ne!(
            canvas2d::dense_edge_batch_key(false, &a1, 1.0, 0.76),
            canvas2d::dense_edge_batch_key(false, &a3, 2.0, 0.94)
        );

        let mut measure_calls = 0usize;
        let first = canvas2d::fit_label_cached_with(
            "OrderController::checkoutSessionHandler",
            90.0,
            "500 11px sans-serif",
            |s| {
                measure_calls += 1;
                (s.chars().count() as f64) * 7.0
            },
        );
        let calls_after_first = measure_calls;
        assert!(calls_after_first > 0);
        let second = canvas2d::fit_label_cached_with(
            "OrderController::checkoutSessionHandler",
            90.0,
            "500 11px sans-serif",
            |s| {
                measure_calls += 1;
                (s.chars().count() as f64) * 7.0
            },
        );
        assert_eq!(first, second);
        assert_eq!(measure_calls, calls_after_first);

        let mut badge_calls = 0usize;
        let w1 = canvas2d::badge_width_cached_with("cls", |s| {
            badge_calls += 1;
            (s.len() as f64) * 6.0
        });
        let w2 = canvas2d::badge_width_cached_with("cls", |s| {
            badge_calls += 1;
            (s.len() as f64) * 6.0
        });
        assert_eq!(w1, w2);
        assert_eq!(badge_calls, 1);

        // `SpatialHitGrid` viewport culling returns matching visible node and edge indices.
        let cull_scene = canvas2d::DenseScene {
            width: 2000.0,
            height: 2000.0,
            nodes: vec![
                canvas2d::DenseNodeSpec {
                    index: 0,
                    x: 20.0,
                    y: 20.0,
                    w: 100.0,
                    h: 40.0,
                    label: "near".into(),
                    ..canvas2d::DenseNodeSpec::default()
                },
                canvas2d::DenseNodeSpec {
                    index: 1,
                    x: 1500.0,
                    y: 1500.0,
                    w: 100.0,
                    h: 40.0,
                    label: "far".into(),
                    ..canvas2d::DenseNodeSpec::default()
                },
            ],
            edges: vec![
                canvas2d::DenseEdgeSpec {
                    index: 5,
                    pts: vec![(30.0, 30.0), (90.0, 50.0)],
                    kind: "Calls".into(),
                    ..canvas2d::DenseEdgeSpec::default()
                },
                canvas2d::DenseEdgeSpec {
                    index: 9,
                    pts: vec![(1510.0, 1510.0), (1590.0, 1530.0)],
                    kind: "Imports".into(),
                    ..canvas2d::DenseEdgeSpec::default()
                },
            ],
        };
        let grid = canvas2d::SpatialHitGrid::build(&cull_scene, 128.0);
        assert_eq!(
            grid.visible_node_indices(0.0, 0.0, 300.0, 300.0),
            Some(vec![0])
        );
        assert_eq!(
            grid.visible_edge_indices(0.0, 0.0, 300.0, 300.0),
            Some(vec![5])
        );
        assert_eq!(
            grid.visible_node_indices(1400.0, 1400.0, 1700.0, 1700.0),
            Some(vec![1])
        );
        let mut hidden = std::collections::HashSet::new();
        hidden.insert("  Calls ".to_owned());
        let mut meta = canvas2d::DenseSceneMeta {
            hidden_kinds: hidden,
            ..Default::default()
        };
        meta.rebuild_caches(&cull_scene);
        assert!(meta.hidden_kinds.contains("calls"));
        assert!(canvas2d::edge_matches_kind_lower(
            "calls",
            &cull_scene.edges[0]
        ));
        assert!(!canvas2d::edge_matches_kind_lower(
            "calls",
            &cull_scene.edges[1]
        ));
    }

    #[test]
    fn dense_cluster_hulls_group_by_layer_directory_and_community_with_convex_bounds() {
        // 1. Directory path grouping selects coherent sub-clusters (src/api vs src/db).
        let dir_nodes = vec![
            canvas2d::DenseNodeSpec {
                index: 0,
                x: 40.0,
                y: 40.0,
                w: 120.0,
                h: 36.0,
                label: "routes.rs".into(),
                ..Default::default()
            },
            canvas2d::DenseNodeSpec {
                index: 1,
                x: 200.0,
                y: 60.0,
                w: 120.0,
                h: 36.0,
                label: "handlers.rs".into(),
                ..Default::default()
            },
            canvas2d::DenseNodeSpec {
                index: 2,
                x: 40.0,
                y: 420.0,
                w: 120.0,
                h: 36.0,
                label: "pool.rs".into(),
                ..Default::default()
            },
            canvas2d::DenseNodeSpec {
                index: 3,
                x: 210.0,
                y: 440.0,
                w: 120.0,
                h: 36.0,
                label: "queries.rs".into(),
                ..Default::default()
            },
        ];
        let dir_scene = canvas2d::DenseScene {
            width: 600.0,
            height: 600.0,
            nodes: dir_nodes,
            edges: vec![],
        };
        let mut dir_meta = canvas2d::DenseSceneMeta::default();
        for (idx, path) in [
            (0usize, "src/api/routes.rs"),
            (1, "src/api/handlers.rs"),
            (2, "src/db/pool.rs"),
            (3, "src/db/queries.rs"),
        ] {
            dir_meta.nodes.insert(
                idx,
                canvas2d::DenseNodeMeta {
                    explain_kind: "node".into(),
                    explain_payload: json!({ "path": path }),
                    ..Default::default()
                },
            );
        }
        dir_meta.rebuild_caches(&dir_scene);
        assert_eq!(dir_meta.cluster_hulls.len(), 2);
        let api_hull = dir_meta
            .cluster_hulls
            .iter()
            .find(|h| h.label == "src/api")
            .expect("api hull");
        assert_eq!(api_hull.node_count, 2);
        assert!(api_hull.polygon.len() >= 4);
        assert!(api_hull.bounds.0 < 40.0 && api_hull.bounds.2 > 320.0);

        // 2. Explicit layer assignments take priority when multiple multi-node layers exist.
        let mut layered_meta = dir_meta.clone();
        layered_meta.layer_of_node.insert(0, "Http".into());
        layered_meta.layer_of_node.insert(1, "Http".into());
        layered_meta.layer_of_node.insert(2, "Storage".into());
        layered_meta.layer_of_node.insert(3, "Storage".into());
        layered_meta.rebuild_caches(&dir_scene);
        let layer_labels: Vec<&str> = layered_meta
            .cluster_hulls
            .iter()
            .map(|h| h.label.as_str())
            .collect();
        assert!(layer_labels.contains(&"Http"));
        assert!(layer_labels.contains(&"Storage"));

        // 3. Pure graph topology with no path/layer metadata falls back to community propagation.
        let bare_nodes: Vec<canvas2d::DenseNodeSpec> = (0..4)
            .map(|i| canvas2d::DenseNodeSpec {
                index: i,
                x: if i < 2 {
                    50.0 + (i as f64) * 140.0
                } else {
                    550.0 + ((i - 2) as f64) * 140.0
                },
                y: 80.0,
                w: 100.0,
                h: 32.0,
                label: format!("n{i}"),
                ..Default::default()
            })
            .collect();
        let bare_edges = vec![
            canvas2d::DenseEdgeSpec {
                index: 0,
                pts: vec![(100.0, 96.0), (190.0, 96.0)],
                kind: "calls".into(),
                ..Default::default()
            },
            canvas2d::DenseEdgeSpec {
                index: 1,
                pts: vec![(600.0, 96.0), (690.0, 96.0)],
                kind: "calls".into(),
                ..Default::default()
            },
        ];
        let bare_scene = canvas2d::DenseScene {
            width: 900.0,
            height: 300.0,
            nodes: bare_nodes,
            edges: bare_edges,
        };
        let mut bare_meta = canvas2d::DenseSceneMeta::default();
        bare_meta.edges.insert(
            0,
            canvas2d::DenseEdgeMeta {
                from_index: Some(0),
                to_index: Some(1),
                ..Default::default()
            },
        );
        bare_meta.edges.insert(
            1,
            canvas2d::DenseEdgeMeta {
                from_index: Some(2),
                to_index: Some(3),
                ..Default::default()
            },
        );
        bare_meta.rebuild_caches(&bare_scene);
        assert_eq!(bare_meta.cluster_hulls.len(), 2);
    }

    #[test]
    fn dense_cluster_hulls_label_overlap_culling_and_scale_threshold() {
        assert!(canvas2d::should_render_cluster_hulls(0.20));
        assert!(canvas2d::should_render_cluster_hulls(0.349));
        assert!(!canvas2d::should_render_cluster_hulls(0.35));
        assert!(!canvas2d::should_render_cluster_hulls(0.80));
        assert!(!canvas2d::should_render_cluster_hulls(0.0));

        let hulls = vec![
            canvas2d::DenseClusterHull {
                key: "dir:large".into(),
                label: "core/engine".into(),
                color_slot: 0,
                node_count: 18,
                polygon: vec![(0.0, 0.0), (900.0, 0.0), (900.0, 700.0), (0.0, 700.0)],
                centroid: (450.0, 350.0),
                bounds: (0.0, 0.0, 900.0, 700.0),
            },
            canvas2d::DenseClusterHull {
                key: "dir:overlapping".into(),
                label: "core/sub".into(),
                color_slot: 1,
                node_count: 5,
                polygon: vec![
                    (380.0, 300.0),
                    (620.0, 300.0),
                    (620.0, 440.0),
                    (380.0, 440.0),
                ],
                centroid: (460.0, 355.0),
                bounds: (380.0, 300.0, 620.0, 440.0),
            },
            canvas2d::DenseClusterHull {
                key: "dir:far".into(),
                label: "cli/cmd".into(),
                color_slot: 2,
                node_count: 9,
                polygon: vec![
                    (1400.0, 200.0),
                    (2000.0, 200.0),
                    (2000.0, 600.0),
                    (1400.0, 600.0),
                ],
                centroid: (1700.0, 400.0),
                bounds: (1400.0, 200.0, 2000.0, 600.0),
            },
        ];
        let placed = canvas2d::layout_cluster_hull_labels(
            &hulls,
            Camera::new(0.0, 0.0, 0.25),
            1200.0,
            800.0,
            |s| (s.len() as f64) * 6.5,
        );
        // Overlapping smaller cluster pill is culled while non-overlapping ones remain.
        let texts: Vec<&str> = placed.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(texts, vec!["core/engine · 18", "cli/cmd · 9"]);

        // Above 0.35 zoom, cluster hull pills are suppressed in favor of regular node labels.
        assert!(canvas2d::layout_cluster_hull_labels(
            &hulls,
            Camera::new(0.0, 0.0, 0.40),
            1200.0,
            800.0,
            |s| (s.len() as f64) * 6.5,
        )
        .is_empty());
    }

    #[test]
    fn dense_scene_meta_containers_preserve_default_construction_and_suppress_auto_hulls() {
        let scene = canvas2d::DenseScene {
            width: 800.0,
            height: 600.0,
            nodes: vec![
                canvas2d::DenseNodeSpec {
                    index: 0,
                    x: 40.0,
                    y: 40.0,
                    w: 140.0,
                    h: 54.0,
                    label: "api/orders".into(),
                    ..Default::default()
                },
                canvas2d::DenseNodeSpec {
                    index: 1,
                    x: 40.0,
                    y: 120.0,
                    w: 140.0,
                    h: 54.0,
                    label: "api/billing".into(),
                    ..Default::default()
                },
            ],
            edges: vec![],
        };
        let mut meta = canvas2d::DenseSceneMeta {
            containers: vec![
                canvas2d::DenseContainerSpec {
                    kind: canvas2d::DenseContainerKind::TierBand,
                    x: 16.0,
                    y: 16.0,
                    w: 220.0,
                    h: 220.0,
                    radius: 8.0,
                    label: "ENTRY".into(),
                },
                canvas2d::DenseContainerSpec {
                    kind: canvas2d::DenseContainerKind::ServiceFrame,
                    x: 28.0,
                    y: 28.0,
                    w: 180.0,
                    h: 180.0,
                    radius: 10.0,
                    label: "api".into(),
                },
            ],
            ..Default::default()
        };
        meta.rebuild_caches(&scene);
        assert_eq!(meta.containers.len(), 2);
        assert!(meta.cluster_hulls.is_empty());
    }

    #[test]
    fn offscreen_base_key_reuses_hover_only_changes_and_invalidates_on_camera_dpr_scene_or_filters()
    {
        let mut scene = canvas2d::DenseScene {
            width: 800.0,
            height: 600.0,
            nodes: vec![
                canvas2d::DenseNodeSpec {
                    index: 0,
                    x: 20.0,
                    y: 20.0,
                    w: 140.0,
                    h: 54.0,
                    label: "n0".into(),
                    ..Default::default()
                },
                canvas2d::DenseNodeSpec {
                    index: 1,
                    x: 240.0,
                    y: 20.0,
                    w: 140.0,
                    h: 54.0,
                    label: "n1".into(),
                    lit: true,
                    ..Default::default()
                },
            ],
            edges: vec![canvas2d::DenseEdgeSpec {
                index: 0,
                kind: "calls".into(),
                pts: vec![(160.0, 47.0), (240.0, 47.0)],
                weight: 1,
                ..Default::default()
            }],
        };
        let mut meta = canvas2d::DenseSceneMeta::default();
        meta.rebuild_caches(&scene);
        let d0 = canvas2d::compute_scene_digest(&scene, &meta);
        let cam0 = Camera {
            scale: 1.0,
            tx: 10.0,
            ty: 20.0,
        };
        let hidden = HashSet::new();
        let k_base = canvas2d::build_offscreen_base_key(
            d0, 2.0, 1024.0, 768.0, cam0, 1, true, None, None, &hidden,
        );
        // Transient pointer hover changes do NOT alter OffscreenBaseKey -> offscreen base reuses cache.
        let k_hover_same = canvas2d::build_offscreen_base_key(
            d0, 2.0, 1024.0, 768.0, cam0, 1, true, None, None, &hidden,
        );
        assert!(canvas2d::should_reuse_offscreen_base(
            Some(&k_base),
            &k_hover_same
        ));

        // Any change to pan/scale, viewport, DPR, palette, filters, or scene invalidates base key.
        assert!(!canvas2d::should_reuse_offscreen_base(
            Some(&k_base),
            &canvas2d::build_offscreen_base_key(
                d0,
                2.0,
                1024.0,
                768.0,
                Camera { tx: 11.0, ..cam0 },
                1,
                true,
                None,
                None,
                &hidden,
            )
        ));
        assert!(!canvas2d::should_reuse_offscreen_base(
            Some(&k_base),
            &canvas2d::build_offscreen_base_key(
                d0, 1.5, 1024.0, 768.0, cam0, 1, true, None, None, &hidden,
            )
        ));
        assert!(!canvas2d::should_reuse_offscreen_base(
            Some(&k_base),
            &canvas2d::build_offscreen_base_key(
                d0, 2.0, 800.0, 768.0, cam0, 1, true, None, None, &hidden,
            )
        ));
        assert!(!canvas2d::should_reuse_offscreen_base(
            Some(&k_base),
            &canvas2d::build_offscreen_base_key(
                d0, 2.0, 1024.0, 768.0, cam0, 2, true, None, None, &hidden,
            )
        ));
        let mut hidden_calls = hidden.clone();
        hidden_calls.insert("calls".into());
        assert!(!canvas2d::should_reuse_offscreen_base(
            Some(&k_base),
            &canvas2d::build_offscreen_base_key(
                d0,
                2.0,
                1024.0,
                768.0,
                cam0,
                1,
                true,
                Some("calls"),
                None,
                &hidden_calls,
            )
        ));

        scene.nodes[0].selected = true;
        let d1 = canvas2d::compute_scene_digest(&scene, &meta);
        assert_ne!(d0, d1);

        let mut hov_edges = HashSet::new();
        hov_edges.insert(0usize);
        let mut hov_nodes = HashSet::new();
        hov_nodes.insert(0usize);
        assert!(canvas2d::is_overlay_edge(&scene.edges[0], &hov_edges, None));
        assert!(!canvas2d::is_overlay_edge(
            &scene.edges[0],
            &HashSet::new(),
            None
        ));
        assert!(canvas2d::is_overlay_node(
            &scene.nodes[0],
            Some((0, 0)),
            &hov_nodes
        ));
        assert!(canvas2d::is_overlay_node(
            &scene.nodes[1],
            Some((0, 0)),
            &hov_nodes
        ));
    }

    #[test]
    fn scene_digest_tracks_all_rendered_node_edge_container_and_hull_fields() {
        let mut scene = canvas2d::DenseScene {
            width: 800.0,
            height: 600.0,
            nodes: vec![canvas2d::DenseNodeSpec {
                index: 0,
                x: 20.0,
                y: 20.0,
                w: 120.0,
                h: 48.0,
                label: "svc".into(),
                ..Default::default()
            }],
            edges: vec![canvas2d::DenseEdgeSpec {
                index: 0,
                kind: "calls".into(),
                pts: vec![(20.0, 20.0), (80.0, 35.0), (140.0, 40.0)],
                cmds: vec![
                    canvas2d::DensePathCmd::Move(20.0, 20.0),
                    canvas2d::DensePathCmd::Line(80.0, 35.0),
                    canvas2d::DensePathCmd::Line(140.0, 40.0),
                ],
                weight: 1,
                kinds: vec!["calls".into()],
                ..Default::default()
            }],
        };
        let mut meta = canvas2d::DenseSceneMeta::default();
        meta.nodes.insert(
            0,
            canvas2d::DenseNodeMeta {
                symbol_kind: "class".into(),
                visibility: "public".into(),
                ..Default::default()
            },
        );
        meta.containers.push(canvas2d::DenseContainerSpec {
            kind: canvas2d::DenseContainerKind::TierBand,
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 200.0,
            radius: 8.0,
            label: "Domain".into(),
        });
        meta.rebuild_caches(&scene);
        meta.cluster_hulls.push(canvas2d::DenseClusterHull {
            key: "c0".into(),
            label: "Domain".into(),
            color_slot: 1,
            node_count: 3,
            polygon: vec![(10.0, 10.0), (150.0, 10.0), (150.0, 90.0)],
            centroid: (80.0, 50.0),
            bounds: (10.0, 10.0, 150.0, 90.0),
        });

        let base = canvas2d::compute_scene_digest(&scene, &meta);

        // DenseNodeMeta.{symbol_kind, visibility}
        meta.nodes.get_mut(&0).unwrap().symbol_kind = "interface".into();
        let d = canvas2d::compute_scene_digest(&scene, &meta);
        assert_ne!(base, d, "symbol_kind must affect scene_digest");
        meta.nodes.get_mut(&0).unwrap().symbol_kind = "class".into();

        meta.nodes.get_mut(&0).unwrap().visibility = "private".into();
        assert_ne!(
            base,
            canvas2d::compute_scene_digest(&scene, &meta),
            "visibility must affect scene_digest"
        );
        meta.nodes.get_mut(&0).unwrap().visibility = "public".into();

        // DenseEdgeSpec.{guessed, impact, commit_touched, back, kinds}, interior pts, cmds
        scene.edges[0].guessed = true;
        assert_ne!(base, canvas2d::compute_scene_digest(&scene, &meta));
        scene.edges[0].guessed = false;

        scene.edges[0].impact = "high".into();
        assert_ne!(base, canvas2d::compute_scene_digest(&scene, &meta));
        scene.edges[0].impact.clear();

        scene.edges[0].commit_touched = true;
        assert_ne!(base, canvas2d::compute_scene_digest(&scene, &meta));
        scene.edges[0].commit_touched = false;

        scene.edges[0].back = true;
        assert_ne!(base, canvas2d::compute_scene_digest(&scene, &meta));
        scene.edges[0].back = false;

        scene.edges[0].kinds.push("imports".into());
        assert_ne!(base, canvas2d::compute_scene_digest(&scene, &meta));
        scene.edges[0].kinds.pop();

        scene.edges[0].pts[1] = (90.0, 55.0);
        assert_ne!(base, canvas2d::compute_scene_digest(&scene, &meta));
        scene.edges[0].pts[1] = (80.0, 35.0);

        scene.edges[0].cmds[1] = canvas2d::DensePathCmd::Curve {
            c1: (50.0, 20.0),
            c2: (110.0, 40.0),
            to: (140.0, 40.0),
        };
        assert_ne!(base, canvas2d::compute_scene_digest(&scene, &meta));
        scene.edges[0].cmds[1] = canvas2d::DensePathCmd::Line(80.0, 35.0);

        // DenseContainerSpec.{kind, radius}
        meta.containers[0].kind = canvas2d::DenseContainerKind::ServiceFrame;
        assert_ne!(base, canvas2d::compute_scene_digest(&scene, &meta));
        meta.containers[0].kind = canvas2d::DenseContainerKind::TierBand;

        meta.containers[0].radius = 16.0;
        assert_ne!(base, canvas2d::compute_scene_digest(&scene, &meta));
        meta.containers[0].radius = 8.0;

        // DenseClusterHull bounds.{1,3}, color_slot, node_count, polygon, centroid
        meta.cluster_hulls[0].bounds.1 = 15.0;
        assert_ne!(base, canvas2d::compute_scene_digest(&scene, &meta));
        meta.cluster_hulls[0].bounds.1 = 10.0;

        meta.cluster_hulls[0].bounds.3 = 95.0;
        assert_ne!(base, canvas2d::compute_scene_digest(&scene, &meta));
        meta.cluster_hulls[0].bounds.3 = 90.0;

        meta.cluster_hulls[0].color_slot = 2;
        assert_ne!(base, canvas2d::compute_scene_digest(&scene, &meta));
        meta.cluster_hulls[0].color_slot = 1;

        meta.cluster_hulls[0].node_count = 4;
        assert_ne!(base, canvas2d::compute_scene_digest(&scene, &meta));
        meta.cluster_hulls[0].node_count = 3;

        meta.cluster_hulls[0].centroid = (85.0, 50.0);
        assert_ne!(base, canvas2d::compute_scene_digest(&scene, &meta));
        meta.cluster_hulls[0].centroid = (80.0, 50.0);

        meta.cluster_hulls[0].polygon[1] = (155.0, 12.0);
        assert_ne!(base, canvas2d::compute_scene_digest(&scene, &meta));
        meta.cluster_hulls[0].polygon[1] = (150.0, 10.0);

        assert_eq!(base, canvas2d::compute_scene_digest(&scene, &meta));

        assert!(canvas2d::should_paint_cluster_hull_labels(false, true));
        assert!(!canvas2d::should_paint_cluster_hull_labels(true, true));
        assert!(!canvas2d::should_paint_cluster_hull_labels(false, false));
    }

    #[test]
    fn dense_bitmap_target_size_scales_and_validates_bounds() {
        assert_eq!(canvas2d::dense_bitmap_target_size(63.9, 400.0, 2.0), None);
        assert_eq!(canvas2d::dense_bitmap_target_size(900.0, 63.0, 2.0), None);
        assert_eq!(
            canvas2d::dense_bitmap_target_size(f64::NAN, 400.0, 2.0),
            None
        );
        assert_eq!(
            canvas2d::dense_bitmap_target_size(908.0, 467.59, 2.0),
            Some((1816, 935))
        );
        assert_eq!(
            canvas2d::dense_bitmap_target_size(908.0, 467.59, 3.0),
            Some((1816, 935))
        );
        assert_eq!(
            canvas2d::dense_bitmap_target_size(908.0, 467.59, 0.5),
            Some((908, 468))
        );
    }

    trait CameraTestExt {
        fn dy_or_zero(self, other: Self) -> f64;
    }

    impl CameraTestExt for Camera {
        fn dy_or_zero(self, other: Self) -> f64 {
            self.ty - other.ty
        }
    }
}
