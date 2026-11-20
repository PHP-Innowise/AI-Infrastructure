#![allow(dead_code)]

use super::{
    append_html_elements, canvas2d, pan_to_keep_rect_in_view, plan_viewport_reconcile,
    rect_intersects_bounds, world_viewport_bounds, Camera,
};
use crate::dom;
use serde_json::{json, Value};
use std::collections::HashMap;
use wasm_bindgen::JsCast;
use web_sys::Element;

/// Default CSS selector identifying interactive controls or explicit empty states
/// that canvas pan/drag/double-click handlers must leave untouched.
pub const DEFAULT_INTERACTIVE_SELECTOR: &str =
    "button, a, input, select, textarea, summary, [contenteditable='true'], [data-empty='true']";

/// Pure empty-state plane attribute check shared across canvas pointer guards.
pub fn is_empty_plane_attr(attr: Option<&str>) -> bool {
    attr == Some("true")
}

/// Converts a desired screen-space pointer hit tolerance (in CSS px) to world units,
/// clamping effective camera scale at `0.25` so zoomed-out graphs stay selectable.
pub fn scale_normalized_hit_tol(tol_px: f64, scale: f64) -> f64 {
    let safe_tol = if tol_px.is_finite() && tol_px > 0.0 {
        tol_px
    } else {
        1.0
    };
    let safe_scale = if scale.is_finite() {
        scale.max(0.25)
    } else {
        1.0
    };
    safe_tol / safe_scale
}

/// Returns `true` when pointer events on a graph canvas should NOT start pan/hit/zoom
/// interactions because the plane is empty or the event landed inside an interactive control.
#[allow(unused_variables)]
pub fn is_interactive_or_empty_target(
    target: Option<&Element>,
    plane: Option<&Element>,
    extra_selector: Option<&str>,
) -> bool {
    if plane.is_some_and(|p| is_empty_plane_attr(p.get_attribute("data-empty").as_deref())) {
        return true;
    }
    let Some(el) = target else {
        return false;
    };
    if el
        .closest(DEFAULT_INTERACTIVE_SELECTOR)
        .ok()
        .flatten()
        .is_some()
    {
        return true;
    }
    if let Some(extra) = extra_selector.map(str::trim).filter(|s| !s.is_empty()) {
        if el.closest(extra).ok().flatten().is_some() {
            return true;
        }
    }
    false
}

/// Maps pointer client coordinates over `canvas` into world-space `(x, y)` coordinates.
#[allow(unused_variables)]
pub fn client_to_world(
    canvas: &Element,
    cam: Camera,
    client_x: f64,
    client_y: f64,
) -> Option<(f64, f64)> {
    #[cfg(target_arch = "wasm32")]
    {
        if !cam.scale.is_finite() || cam.scale <= 0.0 {
            return None;
        }
        let rect = canvas.get_bounding_client_rect();
        Some((
            (client_x - rect.left() - cam.tx) / cam.scale,
            (client_y - rect.top() - cam.ty) / cam.scale,
        ))
    }
    #[cfg(not(target_arch = "wasm32"))]
    None
}

/// Resolves `#<canvas_id>` bounding box and performs a scale-normalized Canvas2D hit test.
pub fn hit_test_pointer_in_canvas(
    canvas_id: &str,
    client_x: f64,
    client_y: f64,
    cam: Camera,
    tol_px: f64,
) -> Option<(u8, usize)> {
    let canvas = dom::get(canvas_id)?;
    let (wx, wy) = client_to_world(&canvas, cam, client_x, client_y)?;
    canvas2d::hit_test_scene(
        canvas_id,
        wx,
        wy,
        scale_normalized_hit_tol(tol_px, cam.scale),
    )
}

/// Summary of DOM reconciliation mutations performed by [`reconcile_dense_nodes`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReconcileOutcome {
    pub dom_mutated: bool,
    pub unmounted: Vec<usize>,
    pub mounted: Vec<usize>,
    pub offscreen_updates: usize,
}

/// Reconciles viewport-culled DOM node elements inside `plane`, batching inserts,
/// invoking `after_mount` hooks, removing culled elements + extras, and toggling `data-offscreen`.
#[allow(clippy::too_many_arguments)]
pub fn reconcile_dense_nodes<I, H, A, R>(
    plane: &Element,
    cam: Camera,
    viewport: (f64, f64),
    rects: &[(f64, f64, f64, f64)],
    margin_px: f64,
    mount_pad_px: f64,
    unmount_pad_px: f64,
    mounted_els: &mut HashMap<usize, Element>,
    is_important: I,
    mut build_html: H,
    mut after_mount: A,
    mut on_unmount: R,
) -> ReconcileOutcome
where
    I: Fn(usize, Option<&Element>) -> bool,
    H: FnMut(usize) -> Option<String>,
    A: FnMut(usize, &Element),
    R: FnMut(usize),
{
    let Some(visible) = world_viewport_bounds(cam, viewport, margin_px) else {
        return ReconcileOutcome::default();
    };
    let mount = (
        visible.0 - mount_pad_px,
        visible.1 - mount_pad_px,
        visible.2 + mount_pad_px,
        visible.3 + mount_pad_px,
    );
    let unmount = (
        visible.0 - unmount_pad_px,
        visible.1 - unmount_pad_px,
        visible.2 + unmount_pad_px,
        visible.3 + unmount_pad_px,
    );

    let mounted_flags: Vec<(usize, bool)> = mounted_els
        .iter()
        .map(|(&idx, el)| (idx, is_important(idx, Some(el))))
        .collect();
    let force_mount: Vec<usize> = (0..rects.len())
        .filter(|&idx| is_important(idx, mounted_els.get(&idx)))
        .collect();

    let plan =
        plan_viewport_reconcile(rects, &mounted_flags, visible, mount, unmount, &force_mount);
    let mut outcome = ReconcileOutcome {
        dom_mutated: !plan.to_unmount.is_empty() || !plan.to_mount.is_empty(),
        unmounted: Vec::new(),
        mounted: Vec::new(),
        offscreen_updates: 0,
    };

    for idx in plan.to_unmount {
        if let Some(el) = mounted_els.remove(&idx) {
            el.remove();
            on_unmount(idx);
            outcome.unmounted.push(idx);
        }
    }

    let mut batch_html = String::new();
    let mut batch_indices = Vec::new();
    for idx in plan.to_mount {
        if mounted_els.contains_key(&idx) {
            continue;
        }
        if let Some(snippet) = build_html(idx).filter(|s| !s.is_empty()) {
            batch_html.push_str(&snippet);
            batch_indices.push(idx);
        }
    }

    if !batch_html.is_empty() {
        let inserted = append_html_elements(plane, &batch_html);
        for (idx, el) in batch_indices.into_iter().zip(inserted) {
            after_mount(idx, &el);
            mounted_els.insert(idx, el);
            outcome.mounted.push(idx);
        }
    }

    let offscreen_map: HashMap<usize, bool> = plan.to_offscreen.into_iter().collect();
    let active_el = dom::document().active_element();
    for (&idx, el) in mounted_els.iter() {
        let keep_visible = is_important(idx, Some(el))
            || active_el
                .as_ref()
                .is_some_and(|act| el == act || el.contains(Some(act)));
        let out_of_view = offscreen_map.get(&idx).copied().unwrap_or_else(|| {
            rects
                .get(idx)
                .is_some_and(|&(x, y, w, h)| !rect_intersects_bounds(x, y, w, h, visible))
        });
        let offscreen = !keep_visible && out_of_view;
        let currently = el.get_attribute("data-offscreen").as_deref() == Some("true");
        if currently != offscreen {
            if offscreen {
                let _ = el.set_attribute("data-offscreen", "true");
            } else {
                let _ = el.remove_attribute("data-offscreen");
            }
            outcome.offscreen_updates += 1;
        }
    }

    outcome
}

/// Ensures `target_idx` is panned into view, mounted in `plane`, receives `tabindex="0"`
/// while siblings matching `node_selector` receive `tabindex="-1"`, and moves DOM focus to it.
/// Returns `Some(updated_cam)` when camera panning was needed.
#[allow(clippy::too_many_arguments)]
pub fn apply_roving_node_focus<I, H, A>(
    canvas: &Element,
    plane: &Element,
    cam: Camera,
    viewport: (f64, f64),
    rects: &[(f64, f64, f64, f64)],
    target_idx: usize,
    node_selector: &str,
    pan_margin_px: f64,
    idx_from_el: I,
    build_html: H,
    after_mount: A,
) -> Option<Camera>
where
    I: Fn(&Element) -> Option<usize>,
    H: FnOnce(usize) -> Option<String>,
    A: FnOnce(usize, &Element),
{
    let next_cam = rects
        .get(target_idx)
        .and_then(|&rect| pan_to_keep_rect_in_view(cam, rect, viewport, pan_margin_px));

    let mut target_el: Option<Element> = None;
    if let Ok(nodes) = canvas.query_selector_all(node_selector) {
        for i in 0..nodes.length() {
            let Some(el) = nodes.item(i).and_then(|n| n.dyn_into::<Element>().ok()) else {
                continue;
            };
            let is_target = idx_from_el(&el) == Some(target_idx);
            let _ = el.set_attribute("tabindex", if is_target { "0" } else { "-1" });
            if is_target {
                target_el = Some(el);
            }
        }
    }

    if target_el.is_none() {
        if let Some(snippet) = build_html(target_idx).filter(|s| !s.is_empty()) {
            let inserted = append_html_elements(plane, &snippet);
            if let Some(el) = inserted.into_iter().next() {
                after_mount(target_idx, &el);
                let _ = el.set_attribute("tabindex", "0");
                target_el = Some(el);
            }
        }
    }

    if let Some(el) = target_el {
        let _ = el.remove_attribute("data-offscreen");
        if let Ok(he) = el.dyn_into::<web_sys::HtmlElement>() {
            let _ = he.focus();
        }
    }

    next_cam
}

/// Synchronizes `data-hover="true"` on DOM nodes inside `plane` without scanning every child:
/// clears stale `[data-hover='true']` markers and sets `data-hover="true"` on the hovered node.
pub fn sync_dom_hover_attr<I>(
    plane: &Element,
    hovered_idx: Option<usize>,
    target_el: Option<&Element>,
    idx_from_el: I,
) where
    I: Fn(&Element) -> Option<usize>,
{
    if let Ok(prev) = plane.query_selector_all("[data-hover='true']") {
        for i in 0..prev.length() {
            if let Some(el) = prev.item(i).and_then(|n| n.dyn_into::<Element>().ok()) {
                if hovered_idx.is_none_or(|want| idx_from_el(&el) != Some(want)) {
                    let _ = el.remove_attribute("data-hover");
                }
            }
        }
    }
    if let Some(el) = target_el {
        if el.get_attribute("data-hover").as_deref() != Some("true") {
            let _ = el.set_attribute("data-hover", "true");
        }
    }
}

/// Result of ending a Shift+drag node pin gesture.
#[derive(Clone, Debug, PartialEq)]
pub enum PinDragOutcome {
    Committed {
        node_idx: usize,
        node_key: String,
        pos: (f64, f64),
    },
    Click {
        node_idx: usize,
        node_key: String,
    },
    Cancelled {
        node_idx: usize,
        node_key: String,
        start_pos: (f64, f64),
    },
}

/// Shared Shift+drag node positioning state machine usable across graph surfaces.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PinDragSession {
    pub active: bool,
    pub pointer_id: i32,
    pub node_idx: usize,
    pub node_key: String,
    pub start_client: (f64, f64),
    pub start_pos: (f64, f64),
    pub cur_pos: (f64, f64),
    pub moved: bool,
}

impl PinDragSession {
    pub fn begin(
        pointer_id: i32,
        node_idx: usize,
        node_key: impl Into<String>,
        client: (f64, f64),
        initial_pos: (f64, f64),
    ) -> Self {
        Self {
            active: true,
            pointer_id,
            node_idx,
            node_key: node_key.into(),
            start_client: client,
            start_pos: initial_pos,
            cur_pos: initial_pos,
            moved: false,
        }
    }

    /// Advances a Shift+drag session and returns the clamped `(x, y)` world position once the
    /// drag exceeds `threshold_px` (or continues moving). `bounds` is `(min_x, min_y, max_x, max_y)`.
    pub fn advance(
        &mut self,
        pointer_id: i32,
        client: (f64, f64),
        scale: f64,
        bounds: (f64, f64, f64, f64),
        threshold_px: f64,
    ) -> Option<(f64, f64)> {
        if !self.active || self.pointer_id != pointer_id {
            return None;
        }
        let dx_client = client.0 - self.start_client.0;
        let dy_client = client.1 - self.start_client.1;
        if !self.moved && dx_client.hypot(dy_client) < threshold_px {
            return None;
        }
        self.moved = true;
        let safe_scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        let min_x = bounds.0;
        let min_y = bounds.1;
        let max_x = bounds.2.max(min_x);
        let max_y = bounds.3.max(min_y);
        let nx = (self.start_pos.0 + dx_client / safe_scale).clamp(min_x, max_x);
        let ny = (self.start_pos.1 + dy_client / safe_scale).clamp(min_y, max_y);
        self.cur_pos = (nx, ny);
        Some(self.cur_pos)
    }

    pub fn end(&mut self, pointer_id: Option<i32>, cancel: bool) -> Option<PinDragOutcome> {
        if !self.active {
            return None;
        }
        if let Some(pid) = pointer_id {
            if pid != self.pointer_id {
                return None;
            }
        }
        let moved = self.moved;
        let node_idx = self.node_idx;
        let node_key = self.node_key.clone();
        let start_pos = self.start_pos;
        let cur_pos = self.cur_pos;
        *self = Self::default();
        if cancel {
            Some(PinDragOutcome::Cancelled {
                node_idx,
                node_key,
                start_pos,
            })
        } else if moved {
            Some(PinDragOutcome::Committed {
                node_idx,
                node_key,
                pos: cur_pos,
            })
        } else {
            Some(PinDragOutcome::Click { node_idx, node_key })
        }
    }

    pub fn from_json(v: &Value) -> Self {
        Self {
            active: v["active"].as_bool().unwrap_or(false),
            pointer_id: v["pointer_id"].as_i64().unwrap_or(0) as i32,
            node_idx: v["node_idx"].as_u64().unwrap_or(0) as usize,
            node_key: v["node_key"].as_str().unwrap_or_default().to_owned(),
            start_client: (
                v["sx"].as_f64().unwrap_or(0.0),
                v["sy"].as_f64().unwrap_or(0.0),
            ),
            start_pos: (
                v["bx"].as_f64().unwrap_or(0.0),
                v["by"].as_f64().unwrap_or(0.0),
            ),
            cur_pos: (
                v["cx"].as_f64().unwrap_or(0.0),
                v["cy"].as_f64().unwrap_or(0.0),
            ),
            moved: v["moved"].as_bool().unwrap_or(false),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "active": self.active,
            "pointer_id": self.pointer_id,
            "node_idx": self.node_idx,
            "node_key": self.node_key,
            "sx": self.start_client.0,
            "sy": self.start_client.1,
            "bx": self.start_pos.0,
            "by": self.start_pos.1,
            "cx": self.cur_pos.0,
            "cy": self.cur_pos.1,
            "moved": self.moved,
        })
    }
}
