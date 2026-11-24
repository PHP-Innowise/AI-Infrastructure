//! Dependency map of the project's code graph: one folder at a time, what
//! uses what. Whatever depends on something stands to its left, so entry
//! points gather on the left edge and foundations on the right. A file opens
//! in turn to its classes, methods and functions and the files around them.
use crate::{api, dom, graph::GraphTargetScope, graph_viewport, model as m};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as V};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use wasm_bindgen::{closure::Closure, JsCast};
use wasm_bindgen_futures::spawn_local;

mod diff_impact;
mod finder_commit;
mod layout;
#[cfg(test)]
mod memory_bridge_tests;
mod render;
mod surfaces;
#[cfg(test)]
mod tests;

#[cfg(test)]
#[allow(unused_imports)]
pub use diff_impact::arrow_kind_for_edge;
#[cfg(test)]
#[allow(unused_imports)]
pub use diff_impact::arrow_kinds_legend_html;
#[allow(unused_imports)]
pub use diff_impact::chosen;
#[allow(unused_imports)]
pub use diff_impact::cone_of;
#[allow(unused_imports)]
pub use diff_impact::cross_service_unopened_html;
#[allow(unused_imports)]
pub(crate) use diff_impact::format_graph_backend_label;
#[allow(unused_imports)]
pub(crate) use diff_impact::format_graph_status_text;
#[allow(unused_imports)]
pub use diff_impact::key_html;
#[allow(unused_imports)]
pub use diff_impact::kind_counts;
#[allow(unused_imports)]
pub use diff_impact::kinds_legend_html;
#[allow(unused_imports)]
pub use diff_impact::kinds_off;
#[allow(unused_imports)]
pub use diff_impact::normalize_edge_arrow_kind;
#[allow(unused_imports)]
pub use diff_impact::note;
#[allow(unused_imports)]
pub use diff_impact::preset_kinds;
#[allow(unused_imports)]
pub use diff_impact::preset_off;
#[allow(unused_imports)]
pub use diff_impact::reaching;
#[allow(unused_imports)]
pub use diff_impact::service_switcher_options_html;
#[allow(unused_imports)]
pub use diff_impact::summary;
#[allow(unused_imports)]
pub use diff_impact::switched_kind;
#[allow(unused_imports)]
use diff_impact::synthetic_edge_path;
#[allow(unused_imports)]
pub use diff_impact::targets;
#[allow(unused_imports)]
pub use diff_impact::transitive_links;
#[allow(unused_imports)]
pub use diff_impact::typed;
#[allow(unused_imports)]
pub use diff_impact::with_run;
#[allow(unused_imports)]
pub use diff_impact::with_system_run;
#[allow(unused_imports)]
pub use diff_impact::CanvasOverlayContext;
#[allow(unused_imports)]
pub use diff_impact::EdgeKinds;
#[allow(unused_imports)]
pub use diff_impact::KeyState;
#[allow(unused_imports)]
pub use diff_impact::KindsOff;
#[allow(unused_imports)]
pub use diff_impact::CONE_REACH;
#[allow(unused_imports)]
pub use finder_commit::agent_options_html;
#[allow(unused_imports)]
pub use finder_commit::ago;
#[allow(unused_imports)]
pub(crate) use finder_commit::attach_system_wheel_listeners;
#[allow(unused_imports)]
pub use finder_commit::attach_wheel_listeners;
#[allow(unused_imports)]
pub use finder_commit::begins;
#[allow(unused_imports)]
pub use finder_commit::between;
#[allow(unused_imports)]
pub use finder_commit::build_scoped_find_path;
#[allow(unused_imports)]
pub(crate) use finder_commit::build_scoped_graph_request;
#[allow(unused_imports)]
pub(crate) use finder_commit::build_scoped_link_request;
#[allow(unused_imports)]
pub use finder_commit::build_scoped_picker_path;
#[allow(unused_imports)]
pub use finder_commit::centred;
#[allow(unused_imports)]
pub use finder_commit::choosing;
#[allow(unused_imports)]
pub use finder_commit::failing_lines;
#[allow(unused_imports)]
pub use finder_commit::found;
#[allow(unused_imports)]
pub use finder_commit::found_note;
#[allow(unused_imports)]
pub(crate) use finder_commit::graph_request;
#[allow(unused_imports)]
pub use finder_commit::in_window;
#[allow(unused_imports)]
pub use finder_commit::interim;
#[allow(unused_imports)]
pub use finder_commit::key;
#[allow(unused_imports)]
pub use finder_commit::led;
#[allow(unused_imports)]
pub use finder_commit::left_pattern;
#[allow(unused_imports)]
pub(crate) use finder_commit::link_request;
#[allow(unused_imports)]
pub use finder_commit::lit_link;
#[allow(unused_imports)]
pub use finder_commit::lit_place;
#[allow(unused_imports)]
pub use finder_commit::lod;
#[allow(unused_imports)]
pub use finder_commit::marked;
#[allow(unused_imports)]
pub use finder_commit::offered;
#[allow(unused_imports)]
pub(crate) use finder_commit::open_project_commit;
#[allow(unused_imports)]
pub use finder_commit::options;
#[allow(unused_imports)]
pub use finder_commit::paged;
#[allow(unused_imports)]
pub use finder_commit::paging;
#[allow(unused_imports)]
pub use finder_commit::pointed;
#[allow(unused_imports)]
pub use finder_commit::radar_size;
#[allow(unused_imports)]
pub use finder_commit::remark;
#[allow(unused_imports)]
pub use finder_commit::scale;
#[allow(unused_imports)]
pub use finder_commit::scaled;
#[allow(unused_imports)]
pub use finder_commit::scopes;
#[allow(unused_imports)]
pub use finder_commit::scroll_after_zoom;
#[allow(unused_imports)]
pub use finder_commit::sharing;
#[allow(unused_imports)]
pub use finder_commit::showing_pattern;
#[allow(unused_imports)]
pub use finder_commit::sought;
#[allow(unused_imports)]
pub use finder_commit::wheel_zoom_factor;
#[allow(unused_imports)]
pub use finder_commit::Pointed;
#[allow(unused_imports)]
use layout::apply_pin_overrides_to_plan;
#[allow(unused_imports)]
pub use layout::camera_for_bbox;
#[allow(unused_imports)]
pub use layout::camera_for_fit;
#[allow(unused_imports)]
pub use layout::cone;
#[allow(unused_imports)]
pub use layout::entity_room;
#[allow(unused_imports)]
pub use layout::in_cone;
#[allow(unused_imports)]
pub use layout::match_prompt_targets;
#[allow(unused_imports)]
pub use layout::parse_live_focus;
#[cfg(test)]
#[allow(unused_imports)]
pub use layout::plan;
#[allow(unused_imports)]
pub use layout::plan_beside;
#[allow(unused_imports)]
pub use layout::plan_roomy;
#[allow(unused_imports)]
pub use layout::plan_routed;
#[allow(unused_imports)]
pub use layout::plan_row;
#[cfg(test)]
#[allow(unused_imports)]
pub use layout::plan_with;
#[allow(unused_imports)]
pub use layout::resolve_focus;
#[allow(unused_imports)]
pub use layout::transitive;
#[allow(unused_imports)]
pub use layout::zoom_at;
#[allow(unused_imports)]
pub use layout::CreatedEntity;
#[allow(unused_imports)]
pub use layout::FocusOverlay;
#[allow(unused_imports)]
pub use layout::Framed;
#[allow(unused_imports)]
pub use layout::Frames;
#[allow(unused_imports)]
pub use layout::LiveFocus;
#[allow(unused_imports)]
pub use layout::Placed;
#[allow(unused_imports)]
pub use layout::PlacedEntity;
#[allow(unused_imports)]
pub use layout::Plan;
#[allow(unused_imports)]
pub use layout::Routing;
#[allow(unused_imports)]
pub(crate) use layout::SystemEmbeddedKind;
#[allow(unused_imports)]
pub use layout::Wire;
#[allow(unused_imports)]
pub use layout::PORTAL_SPAN;
#[cfg(test)]
#[allow(unused_imports)]
pub use render::canvas;
#[cfg(test)]
#[allow(unused_imports)]
pub use render::canvas_with;
#[allow(unused_imports)]
pub use render::canvas_with_overlays;
#[allow(unused_imports)]
pub use render::count_nodes_in_viewport;
#[allow(unused_imports)]
pub use render::ends;
#[allow(unused_imports)]
pub use render::found_patterns;
#[allow(unused_imports)]
pub use render::graph_scale_tier;
#[cfg(test)]
#[allow(unused_imports)]
pub use render::link_canvas;
#[allow(unused_imports)]
pub use render::link_canvas_with;
#[allow(unused_imports)]
pub use render::link_note;
#[cfg(test)]
#[allow(unused_imports)]
pub use render::link_side;
#[allow(unused_imports)]
pub use render::link_side_with;
#[allow(unused_imports)]
pub use render::link_summary;
#[allow(unused_imports)]
pub use render::link_title;
#[allow(unused_imports)]
pub use render::rect_intersects_viewport;
#[allow(unused_imports)]
pub use render::side;
#[allow(unused_imports)]
pub use render::visible_viewport_bounds;
#[allow(unused_imports)]
pub use render::Ends;
#[allow(unused_imports)]
pub use render::GraphScaleTier;
#[allow(unused_imports)]
pub use render::RenderedCanvas;
#[allow(unused_imports)]
pub use render::ViewportBounds;
#[allow(unused_imports)]
pub use render::DENSE_GRAPH_EDGE_THRESHOLD;
#[allow(unused_imports)]
pub use render::DENSE_GRAPH_NODE_THRESHOLD;
#[allow(unused_imports)]
pub use render::LARGE_GRAPH_EDGE_THRESHOLD;
#[allow(unused_imports)]
pub use render::LARGE_GRAPH_NODE_THRESHOLD;
#[allow(unused_imports)]
pub use surfaces::append_system_task_draft;
#[allow(unused_imports)]
pub(crate) use surfaces::apply_code_map_jump_context;
#[allow(unused_imports)]
pub use surfaces::apply_focus;
#[allow(unused_imports)]
pub(crate) use surfaces::apply_system_live_focus;
#[allow(unused_imports)]
pub use surfaces::build_changes_path;
#[allow(unused_imports)]
pub use surfaces::build_file_impact_path;
#[allow(unused_imports)]
pub use surfaces::build_scoped_changes_path;
#[allow(unused_imports)]
pub use surfaces::build_scoped_file_impact_path;
#[allow(unused_imports)]
pub use surfaces::build_scoped_map_path;
#[allow(unused_imports)]
pub use surfaces::changes_side_html;
#[allow(unused_imports)]
pub use surfaces::contract_consumers_html;
#[allow(unused_imports)]
pub use surfaces::event;
#[allow(unused_imports)]
pub use surfaces::focus_file_in_session;
#[allow(unused_imports)]
pub use surfaces::focus_from_prompt;
#[allow(unused_imports)]
pub use surfaces::focus_strip_chips_html;
#[allow(unused_imports)]
pub use surfaces::focus_strip_chips_html_for_surface;
#[allow(unused_imports)]
pub(crate) use surfaces::focus_system_file;
#[allow(unused_imports)]
pub(crate) use surfaces::focus_system_from_prompt;
#[allow(unused_imports)]
pub(crate) use surfaces::focus_system_target;
#[allow(unused_imports)]
pub(crate) use surfaces::focus_target_in_session_with_context;
#[allow(unused_imports)]
pub(crate) use surfaces::focused_dense_hit_for_canvas;
#[allow(unused_imports)]
pub use surfaces::handle_session_graph_click;
#[allow(unused_imports)]
pub use surfaces::handle_session_graph_input;
#[allow(unused_imports)]
pub(crate) use surfaces::handle_system_change;
#[allow(unused_imports)]
pub(crate) use surfaces::handle_system_click;
#[allow(unused_imports)]
pub(crate) use surfaces::handle_system_input;
#[cfg(test)]
#[allow(unused_imports)]
pub use surfaces::impact_side_html;
#[allow(unused_imports)]
pub use surfaces::impact_side_html_with_filter;
#[allow(unused_imports)]
pub(crate) use surfaces::invalidate_surface_analysis_caches;
#[allow(unused_imports)]
pub(crate) use surfaces::invalidate_surface_analysis_caches_for_live_project;
#[allow(unused_imports)]
pub(crate) use surfaces::load_system_embedded;
#[allow(unused_imports)]
pub(crate) use surfaces::maybe_schedule_idle_refresh;
#[allow(unused_imports)]
pub(crate) use surfaces::open_system_service_map;
#[allow(unused_imports)]
pub use surfaces::pointer_event;
#[allow(unused_imports)]
pub(crate) use surfaces::refresh_external_views_for_stale_sse;
#[allow(unused_imports)]
pub(crate) use surfaces::render_side_memory_bridge_html;
#[allow(unused_imports)]
pub use surfaces::selected_impact_file;
#[allow(unused_imports)]
pub use surfaces::state_focus_file;
#[allow(unused_imports)]
pub use surfaces::sync_embedded;
#[allow(unused_imports)]
pub use surfaces::sync_embedded_changes;

#[allow(unused_imports)]
use diff_impact::above;
#[allow(unused_imports)]
use diff_impact::arrow_class_for_kind;
#[allow(unused_imports)]
use diff_impact::arrow_defs_svg;
#[allow(unused_imports)]
use diff_impact::arrow_marker_suffix;
#[allow(unused_imports)]
use diff_impact::at_base;
#[allow(unused_imports)]
use diff_impact::base;
#[allow(unused_imports)]
use diff_impact::capital;
#[allow(unused_imports)]
use diff_impact::change_note;
#[allow(unused_imports)]
use diff_impact::change_status_badge;
#[allow(unused_imports)]
use diff_impact::change_status_letter;
#[allow(unused_imports)]
use diff_impact::changed_order;
#[allow(unused_imports)]
use diff_impact::changes;
#[allow(unused_imports)]
use diff_impact::clean_symbol_token;
#[allow(unused_imports)]
use diff_impact::comparing_order;
#[allow(unused_imports)]
use diff_impact::context;
#[allow(unused_imports)]
use diff_impact::count;
#[allow(unused_imports)]
use diff_impact::cycle_comparison;
#[allow(unused_imports)]
use diff_impact::cycle_revision;
#[allow(unused_imports)]
use diff_impact::cycles;
#[allow(unused_imports)]
use diff_impact::edge_arrow_spec_index;
#[allow(unused_imports)]
use diff_impact::edge_diff_primary_kind;
#[cfg(test)]
#[allow(unused_imports)]
use diff_impact::edge_kind_stats;
#[allow(unused_imports)]
use diff_impact::entity_change_badge;
#[allow(unused_imports)]
use diff_impact::escape;
#[allow(unused_imports)]
use diff_impact::extract_cross_service_unopened;
#[allow(unused_imports)]
use diff_impact::file_note;
#[allow(unused_imports)]
use diff_impact::follow_note;
#[allow(unused_imports)]
use diff_impact::follows;
#[allow(unused_imports)]
use diff_impact::graph_backend_label;
#[allow(unused_imports)]
use diff_impact::halfway;
#[allow(unused_imports)]
use diff_impact::happened;
#[allow(unused_imports)]
use diff_impact::impact_members_details_html;
#[allow(unused_imports)]
use diff_impact::is_native_ir_marker;
#[allow(unused_imports)]
use diff_impact::layer_accent;
#[allow(unused_imports)]
use diff_impact::layered;
#[allow(unused_imports)]
use diff_impact::leaning;
#[allow(unused_imports)]
use diff_impact::left_out;
#[allow(unused_imports)]
use diff_impact::letter;
#[allow(unused_imports)]
use diff_impact::longer_ways;
#[allow(unused_imports)]
use diff_impact::member_has_risk;
#[allow(unused_imports)]
use diff_impact::member_has_severe_risk;
#[allow(unused_imports)]
use diff_impact::member_kind_pill_html;
#[allow(unused_imports)]
use diff_impact::member_kind_tag;
#[allow(unused_imports)]
use diff_impact::member_rows_html;
#[allow(unused_imports)]
use diff_impact::middle;
#[allow(unused_imports)]
use diff_impact::names;
#[allow(unused_imports)]
use diff_impact::names_off;
#[allow(unused_imports)]
use diff_impact::normalize_change_status;
#[allow(unused_imports)]
use diff_impact::normalize_visibility;
#[allow(unused_imports)]
use diff_impact::on_curve;
#[allow(unused_imports)]
use diff_impact::order_comparison;
#[allow(unused_imports)]
use diff_impact::passes_layers;
#[allow(unused_imports)]
use diff_impact::path_matches_node;
#[allow(unused_imports)]
use diff_impact::path_note;
#[allow(unused_imports)]
use diff_impact::paths;
#[allow(unused_imports)]
use diff_impact::pattern_family_of;
#[allow(unused_imports)]
use diff_impact::pattern_note;
#[allow(unused_imports)]
use diff_impact::patterned;
#[allow(unused_imports)]
use diff_impact::payload;
#[allow(unused_imports)]
use diff_impact::pick_precedence_kind;
#[allow(unused_imports)]
use diff_impact::pieces;
#[allow(unused_imports)]
use diff_impact::place_tie;
#[allow(unused_imports)]
use diff_impact::plain_map;
#[allow(unused_imports)]
use diff_impact::plural;
#[allow(unused_imports)]
use diff_impact::reach;
#[allow(unused_imports)]
use diff_impact::resolve_canvas_node_by_path;
#[allow(unused_imports)]
use diff_impact::resolved_member_kind;
#[allow(unused_imports)]
use diff_impact::run_notes;
#[allow(unused_imports)]
use diff_impact::run_reach;
#[allow(unused_imports)]
use diff_impact::said_twice;
#[allow(unused_imports)]
use diff_impact::shared_note;
#[allow(unused_imports)]
use diff_impact::shares;
#[allow(unused_imports)]
use diff_impact::short_tie;
#[allow(unused_imports)]
use diff_impact::shown;
#[allow(unused_imports)]
use diff_impact::spreads;
#[allow(unused_imports)]
use diff_impact::symbol_name_matches;
#[allow(unused_imports)]
use diff_impact::symbols;
#[allow(unused_imports)]
use diff_impact::synthetic_edge_points;
#[allow(unused_imports)]
use diff_impact::unopened;
#[allow(unused_imports)]
use diff_impact::vis_delta_badge_html;
#[allow(unused_imports)]
use diff_impact::vis_warnings_html;
#[allow(unused_imports)]
use diff_impact::visibility_pill_html;
#[allow(unused_imports)]
use diff_impact::weight;
#[allow(unused_imports)]
use diff_impact::what;
#[allow(unused_imports)]
use diff_impact::ChangedEntityDiff;
#[allow(unused_imports)]
use diff_impact::ChangesLookup;
#[allow(unused_imports)]
use diff_impact::EdgeArrowSpec;
#[allow(unused_imports)]
use diff_impact::ImpactLookup;
#[allow(unused_imports)]
use diff_impact::MemberDetailsOpts;
#[allow(unused_imports)]
use diff_impact::Piece;
#[allow(unused_imports)]
use diff_impact::CURVE_STEPS;
#[allow(unused_imports)]
use diff_impact::EDGE_ARROW_SPECS;
#[allow(unused_imports)]
use diff_impact::KIND_PRESETS;
#[allow(unused_imports)]
use diff_impact::LONGER;
#[allow(unused_imports)]
use finder_commit::active_embedded_surface_for_key;
#[allow(unused_imports)]
use finder_commit::anew;
#[allow(unused_imports)]
use finder_commit::answered;
#[allow(unused_imports)]
use finder_commit::apply_link_camera;
#[allow(unused_imports)]
use finder_commit::apply_memory_links_batch_response;
#[allow(unused_imports)]
use finder_commit::architecture_side;
#[allow(unused_imports)]
use finder_commit::attach_canvas_hover;
#[allow(unused_imports)]
use finder_commit::attach_canvas_wheel;
#[allow(unused_imports)]
use finder_commit::attach_legend_hover;
#[allow(unused_imports)]
use finder_commit::attach_side_hover;
#[allow(unused_imports)]
use finder_commit::base_cycle_button;
#[allow(unused_imports)]
use finder_commit::begun;
#[allow(unused_imports)]
use finder_commit::blank;
#[allow(unused_imports)]
use finder_commit::build_memory_links_batch_payload;
#[allow(unused_imports)]
use finder_commit::close_found;
#[allow(unused_imports)]
use finder_commit::commit_action_target;
#[allow(unused_imports)]
use finder_commit::commit_dock;
#[allow(unused_imports)]
use finder_commit::commit_dock_key;
#[allow(unused_imports)]
use finder_commit::commit_dock_note;
#[allow(unused_imports)]
use finder_commit::commit_memory_section;
#[allow(unused_imports)]
use finder_commit::coverage_pill_html;
#[allow(unused_imports)]
use finder_commit::coverage_provenance_pills_html;
#[allow(unused_imports)]
use finder_commit::day;
#[allow(unused_imports)]
use finder_commit::dock_histories;
#[allow(unused_imports)]
use finder_commit::draw_link;
#[allow(unused_imports)]
use finder_commit::ends_of;
#[allow(unused_imports)]
use finder_commit::ensure_commit_dock_scroll_listener;
#[allow(unused_imports)]
use finder_commit::ensure_resize_watch;
#[allow(unused_imports)]
use finder_commit::ensure_surface_watch;
#[allow(unused_imports)]
use finder_commit::fetch;
#[allow(unused_imports)]
use finder_commit::fetch_commit_dock;
#[allow(unused_imports)]
use finder_commit::find_key;
#[allow(unused_imports)]
use finder_commit::find_path;
#[allow(unused_imports)]
use finder_commit::finder;
#[allow(unused_imports)]
use finder_commit::fit;
#[allow(unused_imports)]
use finder_commit::fit_link_camera;
#[allow(unused_imports)]
use finder_commit::format_coverage_summary;
#[allow(unused_imports)]
use finder_commit::fresh;
#[allow(unused_imports)]
use finder_commit::fresh_commit_dock;
#[allow(unused_imports)]
use finder_commit::highlight_layer;
#[allow(unused_imports)]
use finder_commit::histories;
#[allow(unused_imports)]
use finder_commit::history_side;
#[allow(unused_imports)]
use finder_commit::into_view;
#[allow(unused_imports)]
use finder_commit::invalidate_hover_cache;
#[allow(unused_imports)]
use finder_commit::is_commit_view;
#[allow(unused_imports)]
use finder_commit::is_editable_key_target;
#[allow(unused_imports)]
use finder_commit::key_now;
#[allow(unused_imports)]
use finder_commit::key_store;
#[allow(unused_imports)]
use finder_commit::launch_options;
#[allow(unused_imports)]
use finder_commit::layer_color;
#[allow(unused_imports)]
use finder_commit::link_plane_size;
#[allow(unused_imports)]
use finder_commit::links_off_now;
#[allow(unused_imports)]
use finder_commit::list;
#[allow(unused_imports)]
use finder_commit::load_link_camera;
#[allow(unused_imports)]
use finder_commit::look;
#[allow(unused_imports)]
use finder_commit::look_at;
#[allow(unused_imports)]
use finder_commit::look_from;
#[allow(unused_imports)]
use finder_commit::looked;
#[allow(unused_imports)]
use finder_commit::map_attachment;
#[allow(unused_imports)]
use finder_commit::mark;
#[allow(unused_imports)]
use finder_commit::mark_commit_dock;
#[allow(unused_imports)]
use finder_commit::mark_found;
#[allow(unused_imports)]
use finder_commit::modules_side;
#[allow(unused_imports)]
use finder_commit::normalize_map_state;
#[allow(unused_imports)]
use finder_commit::open;
#[allow(unused_imports)]
use finder_commit::pan_surface_for_canvas;
#[allow(unused_imports)]
use finder_commit::parse_style_px;
#[allow(unused_imports)]
use finder_commit::picker;
#[allow(unused_imports)]
use finder_commit::point;
#[allow(unused_imports)]
use finder_commit::point_part;
#[allow(unused_imports)]
use finder_commit::radar;
#[allow(unused_imports)]
use finder_commit::radar_plane_size;
#[allow(unused_imports)]
use finder_commit::radar_pointer;
#[allow(unused_imports)]
use finder_commit::radar_target_element;
#[cfg(target_arch = "wasm32")]
#[allow(unused_imports)]
use finder_commit::refresh_commit_memory;
#[allow(unused_imports)]
use finder_commit::rename_memory_key;
#[allow(unused_imports)]
use finder_commit::render_commit_banner_for_view;
#[cfg(test)]
#[allow(unused_imports)]
use finder_commit::render_commit_dock_list;
#[allow(unused_imports)]
use finder_commit::reset_commit_dock_for_scope;
#[allow(unused_imports)]
use finder_commit::reset_link_camera_100;
#[allow(unused_imports)]
use finder_commit::reset_scroll_canvas_zoom;
#[allow(unused_imports)]
use finder_commit::reveal;
#[allow(unused_imports)]
use finder_commit::run_comparison;
#[allow(unused_imports)]
use finder_commit::save_link_camera;
#[allow(unused_imports)]
use finder_commit::save_radar_drag;
#[allow(unused_imports)]
use finder_commit::scenario_button;
#[allow(unused_imports)]
use finder_commit::scenario_side;
#[allow(unused_imports)]
use finder_commit::seek;
#[allow(unused_imports)]
use finder_commit::shell_event_button;
#[allow(unused_imports)]
use finder_commit::shell_reads;
#[allow(unused_imports)]
use finder_commit::show;
#[allow(unused_imports)]
use finder_commit::show_commit_dock;
#[allow(unused_imports)]
use finder_commit::show_found;
#[allow(unused_imports)]
use finder_commit::show_launches;
#[allow(unused_imports)]
use finder_commit::show_shell_event;
#[allow(unused_imports)]
use finder_commit::showing_cycle;
#[allow(unused_imports)]
use finder_commit::shut;
#[allow(unused_imports)]
use finder_commit::shut_commit_dock;
#[allow(unused_imports)]
use finder_commit::spread;
#[allow(unused_imports)]
use finder_commit::state;
#[allow(unused_imports)]
use finder_commit::suspects_side;
#[allow(unused_imports)]
use finder_commit::switch_cycle;
#[allow(unused_imports)]
use finder_commit::switch_kinds;
#[allow(unused_imports)]
use finder_commit::sync_modal_service_switcher;
#[allow(unused_imports)]
use finder_commit::sync_radar;
#[allow(unused_imports)]
use finder_commit::sync_surface_radar;
#[allow(unused_imports)]
use finder_commit::test_paths;
#[allow(unused_imports)]
use finder_commit::unfolded;
#[allow(unused_imports)]
use finder_commit::unfollowed;
#[allow(unused_imports)]
use finder_commit::watch;
#[allow(unused_imports)]
use finder_commit::without_incoming_side;
#[allow(unused_imports)]
use finder_commit::zoom_link_camera_at;
#[allow(unused_imports)]
use finder_commit::zoom_scroll_canvas;
#[allow(unused_imports)]
use finder_commit::COMMIT_MEMORY_COUNTS;
#[allow(unused_imports)]
use finder_commit::COMMIT_MEMORY_ERRORS;
#[allow(unused_imports)]
use finder_commit::COMMIT_MEMORY_FILES;
#[allow(unused_imports)]
use finder_commit::CYCLE_BACK;
#[allow(unused_imports)]
use finder_commit::FAILING;
#[allow(unused_imports)]
use finder_commit::FAILING_NAME;
#[allow(unused_imports)]
use finder_commit::FIT;
#[allow(unused_imports)]
use finder_commit::GRAPH_MAP_LOD;
#[allow(unused_imports)]
use finder_commit::HIDE_TRANSITIVE;
#[allow(unused_imports)]
use finder_commit::LINKS_OFF;
#[allow(unused_imports)]
use finder_commit::MONTHS;
#[allow(unused_imports)]
use finder_commit::NOTESTS;
#[allow(unused_imports)]
use finder_commit::PATTERNS;
#[allow(unused_imports)]
use finder_commit::PATTERN_BACK;
#[allow(unused_imports)]
use finder_commit::RADAR_DRAG;
#[allow(unused_imports)]
use finder_commit::RADAR_LEAST;
#[allow(unused_imports)]
use finder_commit::RADAR_ROOM;
#[allow(unused_imports)]
use finder_commit::SPREAD;
#[allow(unused_imports)]
use finder_commit::TABS;
#[allow(unused_imports)]
use layout::best_ancestor_node;
#[allow(unused_imports)]
use layout::curve;
#[allow(unused_imports)]
use layout::entity_kind_badge;
#[allow(unused_imports)]
use layout::find_entity_anchor;
#[allow(unused_imports)]
use layout::groups;
#[allow(unused_imports)]
use layout::normalize_entity_kind;
#[allow(unused_imports)]
use layout::normalize_rel_path;
#[allow(unused_imports)]
use layout::order;
#[allow(unused_imports)]
use layout::prompt_tokens;
#[allow(unused_imports)]
use layout::route_pinned_wire_path;
#[allow(unused_imports)]
use layout::string_list;
#[allow(unused_imports)]
use layout::strip_symbol_suffix;
#[allow(unused_imports)]
use layout::union_bbox;
#[allow(unused_imports)]
use layout::EntityAnchor;
#[allow(unused_imports)]
use layout::ARC_APART;
#[allow(unused_imports)]
use layout::ARC_FIRST;
#[allow(unused_imports)]
use layout::ARC_STEP;
#[allow(unused_imports)]
use layout::BEND_GAP;
#[allow(unused_imports)]
use layout::BEND_HEIGHT;
#[allow(unused_imports)]
use layout::BOX_HEIGHT;
#[allow(unused_imports)]
use layout::BOX_WIDTH;
#[allow(unused_imports)]
use layout::CAMERA_MAX;
#[allow(unused_imports)]
use layout::CAMERA_MIN;
#[allow(unused_imports)]
use layout::CAPTION;
#[allow(unused_imports)]
use layout::COLUMN_GAP;
#[allow(unused_imports)]
use layout::ENTITY_HEIGHT;
#[allow(unused_imports)]
use layout::ENTITY_WIDTH;
#[allow(unused_imports)]
use layout::FOCUS_CAMERA_MAX;
#[allow(unused_imports)]
use layout::FRAME_HEAD;
#[allow(unused_imports)]
use layout::FRAME_PAD;
#[allow(unused_imports)]
use layout::LINK_BETWEEN;
#[allow(unused_imports)]
use layout::LINK_BOX;
#[allow(unused_imports)]
use layout::LINK_GAP;
#[allow(unused_imports)]
use layout::LINK_ROW;
#[allow(unused_imports)]
use layout::MARGIN;
#[allow(unused_imports)]
use layout::PORT_GAP;
#[allow(unused_imports)]
use layout::PORT_SPREAD;
#[allow(unused_imports)]
use layout::ROW_GAP;
#[allow(unused_imports)]
use layout::SMALLEST;
#[allow(unused_imports)]
use layout::ZOOM_STEP;
#[allow(unused_imports)]
use render::apart;
#[allow(unused_imports)]
use render::architecture_cut_hints_html;
#[allow(unused_imports)]
use render::ask_shared;
#[allow(unused_imports)]
use render::asks;
#[allow(unused_imports)]
use render::blast_marks;
#[allow(unused_imports)]
use render::blast_side;
#[allow(unused_imports)]
use render::build_node_badges_strip;
#[allow(unused_imports)]
use render::build_path_index;
#[allow(unused_imports)]
use render::canvas_structure_digest;
#[allow(unused_imports)]
use render::circle_told;
#[allow(unused_imports)]
use render::circles_side;
#[allow(unused_imports)]
use render::compute_canvas_selection_patch;
#[allow(unused_imports)]
use render::compute_selected_bbox;
#[allow(unused_imports)]
use render::confidence_pill_html;
#[allow(unused_imports)]
use render::crumbs;
#[allow(unused_imports)]
use render::cycle_cut_hints_html;
#[allow(unused_imports)]
use render::diff_badge_sign;
#[allow(unused_imports)]
use render::extract_view_graph_parts;
#[allow(unused_imports)]
use render::file_declared_members_html;
#[allow(unused_imports)]
use render::file_side;
#[allow(unused_imports)]
use render::focus_member_meta_html;
#[allow(unused_imports)]
use render::follow_links;
#[allow(unused_imports)]
use render::handles;
#[allow(unused_imports)]
use render::hash_json_value;
#[allow(unused_imports)]
use render::html;
#[allow(unused_imports)]
use render::impact_canvas_digest;
#[allow(unused_imports)]
use render::inject_synthetic_deleted_nodes_and_edges;
#[allow(unused_imports)]
use render::kinds;
#[allow(unused_imports)]
use render::layers_side;
#[allow(unused_imports)]
use render::legend;
#[allow(unused_imports)]
use render::link_group_members_html;
#[allow(unused_imports)]
use render::link_names;
#[allow(unused_imports)]
use render::lookup_path_index;
#[allow(unused_imports)]
use render::merge_diff_priority;
#[allow(unused_imports)]
use render::name_side;
#[allow(unused_imports)]
use render::norm_sym_token;
#[allow(unused_imports)]
use render::open_file;
#[allow(unused_imports)]
use render::pair_rows;
#[allow(unused_imports)]
use render::pair_rows_of;
#[allow(unused_imports)]
use render::path_end;
#[allow(unused_imports)]
use render::path_to;
#[allow(unused_imports)]
use render::paths_overlap;
#[allow(unused_imports)]
use render::pattern_brief;
#[allow(unused_imports)]
use render::pattern_row;
#[allow(unused_imports)]
use render::pattern_side;
#[allow(unused_imports)]
use render::radar_box_state;
#[allow(unused_imports)]
use render::reached;
#[allow(unused_imports)]
use render::refresh_dense_scene_for_patch;
#[allow(unused_imports)]
use render::render_dense_node_extras_for_mounted;
#[allow(unused_imports)]
use render::role;
#[allow(unused_imports)]
use render::rows;
#[allow(unused_imports)]
use render::symbol_focus_payload;
#[allow(unused_imports)]
use render::CanvasSelectionPatch;
#[allow(unused_imports)]
use render::DenseNodeExtraSpec;
#[allow(unused_imports)]
use render::EdgeSelectionState;
#[allow(unused_imports)]
use render::LinkDiffInfo;
#[allow(unused_imports)]
use render::NodeSelectionState;
#[allow(unused_imports)]
use render::SelectionContext;
#[allow(unused_imports)]
use render::FILE_LEGEND;
#[allow(unused_imports)]
use render::FIRST;
#[allow(unused_imports)]
use render::HANDLE_GAP;
#[allow(unused_imports)]
use render::LAYER_LEGEND;
#[allow(unused_imports)]
use render::LEGEND;
#[allow(unused_imports)]
use render::NAME_LEGEND;
#[allow(unused_imports)]
use render::WAY;
#[allow(unused_imports)]
use surfaces::active_drag_surface;
#[allow(unused_imports)]
use surfaces::active_modal_lens;
#[allow(unused_imports)]
use surfaces::apply_click_action;
#[allow(unused_imports)]
use surfaces::apply_modal_lens_switch;
#[allow(unused_imports)]
use surfaces::apply_node_patch_to_el;
#[allow(unused_imports)]
use surfaces::apply_surface_camera;
#[allow(unused_imports)]
use surfaces::apply_surface_history_snapshot;
#[allow(unused_imports)]
use surfaces::apply_surface_live_focus;
#[allow(unused_imports)]
use surfaces::apply_surface_plane_transform;
#[allow(unused_imports)]
use surfaces::attach_map_context_to_system_task;
#[allow(unused_imports)]
use surfaces::blank_embedded_surface;
#[cfg(test)]
#[allow(unused_imports)]
use surfaces::build_map_path;
#[allow(unused_imports)]
use surfaces::build_overlay_extras;
#[allow(unused_imports)]
use surfaces::build_scoped_changes_path_for_state;
#[allow(unused_imports)]
use surfaces::build_surface_side_html;
#[allow(unused_imports)]
use surfaces::cached_impact_for_selection;
#[allow(unused_imports)]
use surfaces::caller_rank;
#[allow(unused_imports)]
use surfaces::changes_count_of;
#[allow(unused_imports)]
use surfaces::choose;
#[allow(unused_imports)]
use surfaces::clear_lens_submodes;
#[allow(unused_imports)]
use surfaces::clear_surface_drag_suppress;
#[allow(unused_imports)]
use surfaces::combine_note;
#[allow(unused_imports)]
use surfaces::current_surface_live_focus;
#[allow(unused_imports)]
use surfaces::effective_sysrun_id;
#[allow(unused_imports)]
use surfaces::effective_target;
#[allow(unused_imports)]
use surfaces::effective_target_from;
#[allow(unused_imports)]
use surfaces::embedded_error_is_unbuilt;
#[cfg(test)]
#[allow(unused_imports)]
use surfaces::embedded_unbuilt_note;
#[allow(unused_imports)]
use surfaces::embedded_unbuilt_note_for_scope;
#[cfg(target_arch = "wasm32")]
#[allow(unused_imports)]
use surfaces::encode_part;
#[cfg(not(target_arch = "wasm32"))]
#[allow(unused_imports)]
use surfaces::encode_part;
#[cfg(target_arch = "wasm32")]
#[allow(unused_imports)]
use surfaces::encode_query;
#[cfg(not(target_arch = "wasm32"))]
#[allow(unused_imports)]
use surfaces::encode_query;
#[allow(unused_imports)]
use surfaces::ensure_surface_impact;
#[allow(unused_imports)]
use surfaces::fetch_surface_changes;
#[allow(unused_imports)]
use surfaces::fetch_surface_impact;
#[allow(unused_imports)]
use surfaces::fit_surface_camera;
#[allow(unused_imports)]
use surfaces::focus_file_on_surface;
#[allow(unused_imports)]
use surfaces::focus_surface_from_prompt;
#[allow(unused_imports)]
pub(crate) use surfaces::focus_target_in_session;
#[allow(unused_imports)]
use surfaces::format_scoped_endpoint;
#[allow(unused_imports)]
use surfaces::handle_surface_click;
#[allow(unused_imports)]
use surfaces::handle_surface_input;
#[allow(unused_imports)]
use surfaces::impact_cache_key;
#[allow(unused_imports)]
use surfaces::impact_dep_group_html;
#[allow(unused_imports)]
use surfaces::invalidate_surface_plan_cache;
#[allow(unused_imports)]
use surfaces::is_explicit_folder;
#[allow(unused_imports)]
use surfaces::is_known_file;
#[allow(unused_imports)]
use surfaces::jump_to_surface_entity_or_target;
#[allow(unused_imports)]
use surfaces::load;
#[allow(unused_imports)]
use surfaces::load_embedded_surface;
#[allow(unused_imports)]
use surfaces::load_surface_camera;
#[allow(unused_imports)]
pub(crate) use surfaces::localize_failure_request;
#[allow(unused_imports)]
pub(crate) use surfaces::localize_failures_on_map;
#[allow(unused_imports)]
use surfaces::looks_like_file_path;
#[allow(unused_imports)]
use surfaces::map_side_html;
#[allow(unused_imports)]
use surfaces::member_filter_bar_html;
#[allow(unused_imports)]
use surfaces::memory_links_fresh;
#[allow(unused_imports)]
use surfaces::memory_links_section;
#[allow(unused_imports)]
use surfaces::metric_count;
#[allow(unused_imports)]
use surfaces::mount_dense_node_if_needed;
#[allow(unused_imports)]
use surfaces::move_roving_node_focus;
#[allow(unused_imports)]
use surfaces::open_full_from_surface;
#[allow(unused_imports)]
use surfaces::pairs_for_view;
#[allow(unused_imports)]
use surfaces::parent;
#[allow(unused_imports)]
use surfaces::patch_edge_state_dom;
#[allow(unused_imports)]
use surfaces::place;
#[allow(unused_imports)]
use surfaces::reconcile_view_state;
#[allow(unused_imports)]
use surfaces::redraw_surface_overlay;
#[allow(unused_imports)]
use surfaces::refit_surface_if_visible;
#[allow(unused_imports)]
use surfaces::render;
#[allow(unused_imports)]
use surfaces::render_embedded_surface;
#[allow(unused_imports)]
use surfaces::render_surface_focus_strip;
#[allow(unused_imports)]
use surfaces::rerender_surface_preserving_camera;
#[allow(unused_imports)]
use surfaces::rerender_surface_side_only;
#[allow(unused_imports)]
use surfaces::reset_surface_camera_100;
#[allow(unused_imports)]
use surfaces::resolve_focus_bbox_with_cached_plan;
#[allow(unused_imports)]
use surfaces::resolve_node_index_by_query;
#[allow(unused_imports)]
use surfaces::resolve_system_click_surface;
#[allow(unused_imports)]
use surfaces::save_surface_camera;
#[allow(unused_imports)]
use surfaces::save_surface_state;
#[allow(unused_imports)]
use surfaces::set_dense_hover;
#[allow(unused_imports)]
use surfaces::set_surface_attrs;
#[allow(unused_imports)]
use surfaces::set_surface_scope;
#[allow(unused_imports)]
pub(crate) use surfaces::show_non_destructive_stale_banner;
#[allow(unused_imports)]
use surfaces::side_tabs_html;
#[allow(unused_imports)]
use surfaces::side_top_html;
#[allow(unused_imports)]
use surfaces::snapshot_opt_changed_scope;
#[allow(unused_imports)]
use surfaces::store_surface_plan_cache;
#[allow(unused_imports)]
use surfaces::surface_by_name;
#[allow(unused_imports)]
use surfaces::surface_changes_doc;
#[allow(unused_imports)]
use surfaces::surface_drag_data;
#[allow(unused_imports)]
use surfaces::surface_pin_key;
#[allow(unused_imports)]
use surfaces::surface_scope;
#[allow(unused_imports)]
use surfaces::surface_state;
#[allow(unused_imports)]
use surfaces::surface_unindexed_side_html;
#[allow(unused_imports)]
use surfaces::surface_view_signature;
#[allow(unused_imports)]
use surfaces::switch_surface_to_service_file;
#[allow(unused_imports)]
use surfaces::sync_modal_lens_and_display;
#[allow(unused_imports)]
use surfaces::sync_node_blast_dom;
#[allow(unused_imports)]
use surfaces::sync_surface_toolbar;
#[allow(unused_imports)]
use surfaces::sync_viewport_culling;
#[allow(unused_imports)]
use surfaces::target_pan_surface;
#[allow(unused_imports)]
use surfaces::toggle_bool_attr;
#[allow(unused_imports)]
use surfaces::transfer_surface_state_to_modal;
#[allow(unused_imports)]
use surfaces::try_patch_canvas_dom_in_place;
#[allow(unused_imports)]
use surfaces::update_surface_changes_badge;
#[allow(unused_imports)]
use surfaces::viewport_size;
#[allow(unused_imports)]
use surfaces::zoom_surface_camera_at;
#[allow(unused_imports)]
use surfaces::zoom_surface_camera_to_bbox;
#[allow(unused_imports)]
use surfaces::CachedSurfacePlan;
#[allow(unused_imports)]
use surfaces::Camera;
#[allow(unused_imports)]
use surfaces::ClickOutcome;
#[allow(unused_imports)]
use surfaces::MapSurface;
#[allow(unused_imports)]
use surfaces::OwnedOverlayExtras;
#[allow(unused_imports)]
use surfaces::PatchedCanvasMeta;
#[allow(unused_imports)]
use surfaces::SurfaceViewSignature;
#[allow(unused_imports)]
use surfaces::MEMORY_LINKS;
#[allow(unused_imports)]
use surfaces::MEMORY_LINKS_LOOKING;
#[allow(unused_imports)]
use surfaces::MODAL_SURFACE;
#[allow(unused_imports)]
use surfaces::SESSION_SURFACE;
#[allow(unused_imports)]
use surfaces::SURFACE_PLAN_CACHE;
#[allow(unused_imports)]
use surfaces::SYSTEM_CHANGE_SURFACE;
#[allow(unused_imports)]
use surfaces::SYSTEM_EMBEDDED_SURFACE;
