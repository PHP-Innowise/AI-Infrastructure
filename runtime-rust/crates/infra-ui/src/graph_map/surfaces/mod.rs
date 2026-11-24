use super::*;
mod camera_dom;
mod embedded;
mod history;
mod inspector;
mod interactions;
mod modal_lifecycle;
mod state_urls;

#[derive(Clone, Copy)]
pub(super) struct MapSurface {
    pub(super) name: &'static str,
    pub(super) id_prefix: &'static str,
    pub(super) canvas: &'static str,
    pub(super) side: &'static str,
    pub(super) note: &'static str,
    pub(super) error_id: &'static str,
    pub(super) attachment_key: &'static str,
    pub(super) scope_model_key: &'static str,
    pub(super) state_model_key: &'static str,
    pub(super) camera_key: &'static str,
    pub(super) focus_key: &'static str,
    pub(super) drag_key: &'static str,
    pub(super) pan_target_key: &'static str,
    pub(super) view_key: &'static str,
    pub(super) map_epoch: &'static str,
    pub(super) changes_key: &'static str,
    pub(super) changes_epoch: &'static str,
    pub(super) impact_key: &'static str,
    pub(super) impact_epoch: &'static str,
    pub(super) wheel_timer_key: &'static str,
    pub(super) refit_timer_key: &'static str,
    pub(super) filter_timer_key: &'static str,
    pub(super) drag_timer_key: &'static str,
    pub(super) embedded: bool,
}
pub(super) const MODAL_SURFACE: MapSurface = MapSurface {
    name: "modal",
    id_prefix: "graph-map",
    canvas: "graph-map-canvas",
    side: "graph-map-side",
    note: "graph-map-note",
    error_id: "graph-map-error",
    attachment_key: "graph-map-attachment",
    scope_model_key: "graph-map-scope",
    state_model_key: "graph-map",
    camera_key: "graph-map-camera",
    focus_key: "",
    drag_key: "graph-map-drag",
    pan_target_key: "",
    view_key: "graph-map-view",
    map_epoch: "graph-map",
    changes_key: "graph-map-changes",
    changes_epoch: "graph-map-changes-fetch",
    impact_key: "graph-map-impact",
    impact_epoch: "graph-map-impact-fetch",
    wheel_timer_key: "graph-map-wheel",
    refit_timer_key: "graph-map-refit",
    filter_timer_key: "graph-map-filter",
    drag_timer_key: "graph-map-drag-clear",
    embedded: false,
};
pub(super) const SESSION_SURFACE: MapSurface = MapSurface {
    name: "session",
    id_prefix: "session-graph",
    canvas: "session-graph-canvas",
    side: "session-graph-side",
    note: "session-graph-note",
    error_id: "session-graph-error",
    attachment_key: "session-graph-attachment",
    scope_model_key: "session-graph-scope",
    state_model_key: "session-graph-state",
    camera_key: "session-graph-camera",
    focus_key: "session-graph-focus",
    drag_key: "session-graph-drag",
    pan_target_key: "session-graph-pan-target",
    view_key: "session-graph-view",
    map_epoch: "session-graph",
    changes_key: "session-graph-changes",
    changes_epoch: "session-graph-changes-fetch",
    impact_key: "session-graph-impact",
    impact_epoch: "session-graph-impact-fetch",
    wheel_timer_key: "session-graph-wheel",
    refit_timer_key: "session-graph-refit",
    filter_timer_key: "session-graph-filter",
    drag_timer_key: "session-graph-drag-clear",
    embedded: true,
};
pub(super) const SYSTEM_EMBEDDED_SURFACE: MapSurface = MapSurface {
    name: "system-member",
    id_prefix: "system-member-graph",
    canvas: "system-member-graph-canvas",
    side: "system-member-graph-side",
    note: "system-member-graph-note",
    error_id: "system-member-graph-error",
    attachment_key: "system-member-graph-attachment",
    scope_model_key: "system-member-graph-scope",
    state_model_key: "system-member-graph-state",
    camera_key: "system-member-graph-camera",
    focus_key: "system-member-graph-focus",
    drag_key: "system-member-graph-drag",
    pan_target_key: "system-member-graph-pan-target",
    view_key: "system-member-graph-view",
    map_epoch: "system-member-graph",
    changes_key: "system-member-graph-changes",
    changes_epoch: "system-member-graph-changes-fetch",
    impact_key: "system-member-graph-impact",
    impact_epoch: "system-member-graph-impact-fetch",
    wheel_timer_key: "system-member-graph-wheel",
    refit_timer_key: "system-member-graph-refit",
    filter_timer_key: "system-member-graph-filter",
    drag_timer_key: "system-member-graph-drag-clear",
    embedded: true,
};
pub(super) const SYSTEM_CHANGE_SURFACE: MapSurface = MapSurface {
    name: "system-change",
    id_prefix: "system-change-graph",
    canvas: "system-change-graph-canvas",
    side: "system-change-graph-side",
    note: "system-change-graph-note",
    error_id: "system-change-graph-error",
    attachment_key: "system-change-graph-attachment",
    scope_model_key: "system-change-graph-scope",
    state_model_key: "system-change-graph-state",
    camera_key: "system-change-graph-camera",
    focus_key: "system-change-graph-focus",
    drag_key: "system-change-graph-drag",
    pan_target_key: "system-change-graph-pan-target",
    view_key: "system-change-graph-view",
    map_epoch: "system-change-graph",
    changes_key: "system-change-graph-changes",
    changes_epoch: "system-change-graph-changes-fetch",
    impact_key: "system-change-graph-impact",
    impact_epoch: "system-change-graph-impact-fetch",
    wheel_timer_key: "system-change-graph-wheel",
    refit_timer_key: "system-change-graph-refit",
    filter_timer_key: "system-change-graph-filter",
    drag_timer_key: "system-change-graph-drag-clear",
    embedded: true,
};
impl SystemEmbeddedKind {
    pub(super) fn surface(self) -> MapSurface {
        match self {
            Self::Member => SYSTEM_EMBEDDED_SURFACE,
            Self::Change => SYSTEM_CHANGE_SURFACE,
        }
    }
}

#[allow(unused_imports)]
pub(super) use camera_dom::{
    apply_node_patch_to_el, apply_surface_camera, apply_surface_plane_transform,
    clear_surface_drag_suppress, current_surface_live_focus, fit_surface_camera,
    invalidate_surface_plan_cache, load_surface_camera, mount_dense_node_if_needed, pairs_for_view,
    patch_edge_state_dom, refit_surface_if_visible, render_surface_focus_strip,
    reset_surface_camera_100, resolve_focus_bbox_with_cached_plan, save_surface_camera,
    store_surface_plan_cache, surface_drag_data, surface_view_signature, sync_node_blast_dom,
    sync_surface_after_chrome_layout, sync_viewport_culling, toggle_bool_attr,
    try_patch_canvas_dom_in_place, viewport_size, zoom_surface_camera_at,
    zoom_surface_camera_to_bbox, CachedSurfacePlan, Camera, PatchedCanvasMeta,
    SurfaceViewSignature, SURFACE_PLAN_CACHE,
};
#[allow(unused_imports)]
pub use camera_dom::{focus_strip_chips_html, focus_strip_chips_html_for_surface};
#[cfg(test)]
#[allow(unused_imports)]
pub(super) use embedded::embedded_unbuilt_note;
#[allow(unused_imports)]
pub(crate) use embedded::{
    apply_code_map_jump_context, apply_system_live_focus, focus_system_file,
    focus_system_from_prompt, focus_system_target, focus_target_in_session,
    focus_target_in_session_with_context, load_system_embedded, localize_failure_request,
    localize_failures_on_map, open_system_service_map,
};
#[allow(unused_imports)]
pub use embedded::{
    apply_focus, focus_file_in_session, focus_from_prompt, sync_embedded, sync_embedded_changes,
};
#[allow(unused_imports)]
pub(super) use embedded::{
    apply_surface_live_focus, blank_embedded_surface, embedded_error_is_unbuilt,
    embedded_unbuilt_note_for_scope, focus_file_on_surface, focus_surface_from_prompt,
    focus_target_on_surface, jump_to_surface_entity_or_target, load_embedded_surface,
    move_roving_node_focus, open_full_from_surface, redraw_surface_overlay,
    render_embedded_surface, transfer_surface_state_to_modal,
};
#[allow(unused_imports)]
pub(crate) use history::focused_dense_hit_for_canvas;
#[allow(unused_imports)]
pub(super) use history::{
    apply_surface_history_snapshot, can_undo_pins, current_dense_hover, navigate_surface_history,
    nudge_surface_node_pin, push_pin_undo_snapshot, record_surface_history, set_dense_hover,
    snapshot_opt_changed_scope, surface_history_trail_html, sync_history_and_pin_controls,
    sync_modal_lens_controls_from_state, undo_surface_pins, unpin_surface_node,
    update_surface_active_camera, LAST_DENSE_HOVER, SURFACE_NAV_EPOCHS, SURFACE_NAV_HISTORY,
    SURFACE_NAV_SNAPSHOTS, SURFACE_PIN_UNDO,
};
#[cfg(test)]
#[allow(unused_imports)]
pub use inspector::impact_side_html;
#[allow(unused_imports)]
pub(crate) use inspector::render_side_memory_bridge_html;
#[allow(unused_imports)]
pub use inspector::{
    build_scoped_changes_path_for_state, changes_side_html, contract_consumers_html,
    impact_side_html_with_filter,
};
#[allow(unused_imports)]
pub(super) use inspector::{
    build_surface_side_html, caller_rank, combine_note, ensure_surface_impact,
    fetch_surface_changes, fetch_surface_impact, impact_dep_group_html, map_side_html,
    member_filter_bar_html, memory_links_fresh, memory_links_section, metric_count,
    rerender_surface_preserving_camera, rerender_surface_side_only, side_tabs_html, side_top_html,
    surface_unindexed_side_html, MEMORY_LINKS, MEMORY_LINKS_LOOKING,
};
#[allow(unused_imports)]
pub(super) use interactions::{
    active_drag_surface, apply_live_node_pin_visuals, handle_surface_click, handle_surface_input,
    resolve_system_click_surface, target_pan_surface,
};
#[allow(unused_imports)]
pub use interactions::{handle_session_graph_click, handle_session_graph_input, pointer_event};
#[allow(unused_imports)]
pub(crate) use interactions::{handle_system_change, handle_system_click, handle_system_input};
#[allow(unused_imports)]
pub use modal_lifecycle::{append_system_task_draft, event};
#[allow(unused_imports)]
pub(in crate::graph_map) use modal_lifecycle::{
    apply_click_action, apply_modal_lens_switch, attach_map_context_to_system_task, choose,
    clear_lens_submodes, load, modal_explain_button_state, parent, place, reconcile_view_state,
    render, set_surface_attrs, switch_surface_to_service_file, sync_modal_explain_button,
    ClickOutcome,
};
#[allow(unused_imports)]
pub(super) use state_urls::{
    active_modal_lens, build_overlay_extras, cached_impact_for_selection, changes_count_of,
    effective_sysrun_id, effective_target, effective_target_from, encode_part, encode_query,
    format_scoped_endpoint, impact_cache_key, is_explicit_folder, is_known_file,
    looks_like_file_path, resolve_node_index_by_query, save_surface_state, set_surface_scope,
    surface_by_name, surface_changes_doc, surface_pin_key, surface_scope, surface_state,
    sync_modal_lens_and_display, sync_surface_toolbar, update_surface_changes_badge,
    OwnedOverlayExtras,
};
#[allow(unused_imports)]
pub use state_urls::{
    build_changes_path, build_file_impact_path, build_scoped_changes_path,
    build_scoped_file_impact_path, build_scoped_map_path, selected_impact_file, state_focus_file,
};
#[cfg(test)]
#[allow(unused_imports)]
pub(super) use state_urls::{build_map_path, progressive_lod_for_state};
#[allow(unused_imports)]
pub(crate) use state_urls::{
    invalidate_surface_analysis_caches, invalidate_surface_analysis_caches_for_live_project,
    maybe_schedule_idle_refresh, refresh_external_views_for_stale_sse,
    show_non_destructive_stale_banner,
};
