use super::*;

/// Lightweight metadata required to lazily render handles/portals/spreads when an
/// offscreen dense node is scrolled into view and mounted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DenseNodeExtraSpec {
    pub index: usize,
    pub x: f64,
    pub y: f64,
    pub followed: bool,
    pub portal_out: Vec<usize>,
    pub portal_in: Vec<usize>,
    pub folder_spread: bool,
    pub dim: bool,
}

/// Per-view boolean modes needed to lazily render a single dense node `<button>` on demand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DenseRenderModeFlags {
    pub layered_view: bool,
    pub opened: bool,
    pub followed: bool,
    pub pathed: bool,
    pub shared: bool,
    pub cycle_view: bool,
    pub pattern_view: bool,
    pub scenario: bool,
    pub cycles: bool,
    pub named: bool,
    pub pattern_rings: bool,
}

/// Compact precomputed overlay/geometry seed for lazy dense node DOM materialization.
#[derive(Clone, Debug, PartialEq)]
pub struct DenseNodeDomSeed {
    pub index: usize,
    pub placed: Placed,
    pub flow: (u64, u64),
    pub blast: Option<(u64, f64)>,
    pub change_status: Option<String>,
    pub impact: Option<&'static str>,
    pub chosen: bool,
    pub selected: bool,
    pub lit: bool,
    pub pinned: bool,
    pub roving: bool,
    pub focus_target: bool,
    pub on_traced_path: bool,
    pub any_traced_path: bool,
    pub shared_path_mark: bool,
}

/// Canvas markup plus resolved plane size and live focus overlay coordinates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RenderedCanvas {
    pub html: String,
    pub width: f64,
    pub height: f64,
    pub focus: FocusOverlay,
    pub selected_bbox: Option<(f64, f64, f64, f64)>,
    /// The picture in small, for the corner of its window.
    pub radar: String,
    pub plan_nodes: Vec<Placed>,
    pub plan_frames: Vec<Framed>,
    pub dense_scene: Option<graph_viewport::canvas2d::DenseScene>,
    pub dense_meta: Option<graph_viewport::canvas2d::DenseSceneMeta>,
    pub dense_node_htmls: Vec<String>,
    pub dense_node_seeds: Vec<Option<DenseNodeDomSeed>>,
    pub dense_render_modes: DenseRenderModeFlags,
    pub dense_extra_specs: Vec<Option<DenseNodeExtraSpec>>,
    pub node_keys: Vec<String>,
    pub pairs: Vec<(usize, usize)>,
    pub node_labels: Vec<String>,
}

impl RenderedCanvas {
    /// Lazily formats the DOM `<button>` markup for `idx` when needed (roving focus or mount).
    #[allow(dead_code)]
    pub fn materialize_dense_node_html(&self, view: &V, idx: usize) -> Option<String> {
        if let Some(existing) = self.dense_node_htmls.get(idx).filter(|s| !s.is_empty()) {
            return Some(existing.clone());
        }
        let seed = self.dense_node_seeds.get(idx)?.as_ref()?;
        let nodes = m::array(&view["nodes"]);
        let node = nodes.get(idx)?;
        Some(render_dense_node_dom_html(
            view,
            node,
            seed,
            self.dense_render_modes,
            false,
        ))
    }
}

pub(super) fn build_path_index(items: &[V]) -> HashMap<&str, usize> {
    let mut map = HashMap::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        if let Some(p) = item["path"].as_str() {
            map.entry(p).or_insert(idx);
        }
    }
    map
}

pub(super) fn lookup_path_index(items: &[V], map: &HashMap<&str, usize>, key: &V) -> Option<usize> {
    if let Some(s) = key.as_str() {
        map.get(s).copied()
    } else {
        items.iter().position(|item| &item["path"] == key)
    }
}
/// Scale thresholds beyond which non-essential marks and offscreen details are simplified.
pub const LARGE_GRAPH_NODE_THRESHOLD: usize = 24;
pub const LARGE_GRAPH_EDGE_THRESHOLD: usize = 60;
pub const DENSE_GRAPH_NODE_THRESHOLD: usize = 32;
pub const DENSE_GRAPH_EDGE_THRESHOLD: usize = 80;

/// Scale classification for detail/culling optimizations on large dependency graphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphScaleTier {
    Normal,
    Large,
    Dense,
}

impl GraphScaleTier {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Large => "large",
            Self::Dense => "dense",
        }
    }

    pub const fn elide_non_essential_marks(self) -> bool {
        matches!(self, Self::Large | Self::Dense)
    }

    pub const fn elide_duplicate_hits(self) -> bool {
        matches!(self, Self::Dense)
    }
}

/// Classifies a graph's visible node/edge counts into `Normal`, `Large`, or `Dense`.
pub fn graph_scale_tier(visible_nodes: usize, visible_edges: usize) -> GraphScaleTier {
    if visible_nodes >= DENSE_GRAPH_NODE_THRESHOLD || visible_edges >= DENSE_GRAPH_EDGE_THRESHOLD {
        GraphScaleTier::Dense
    } else if visible_nodes >= LARGE_GRAPH_NODE_THRESHOLD
        || visible_edges >= LARGE_GRAPH_EDGE_THRESHOLD
    {
        GraphScaleTier::Large
    } else {
        GraphScaleTier::Normal
    }
}

/// Logical map coordinates covered by the current viewport (plus culling padding).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportBounds {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

/// Computes logical viewport bounds in plane coordinates given pan offset, zoom scale, room size, and padding.
pub fn visible_viewport_bounds(
    pan: (f64, f64),
    scale: f64,
    room: (f64, f64),
    margin: f64,
) -> Option<ViewportBounds> {
    if !(scale.is_finite() && scale > 0.0 && room.0 > 0.0 && room.1 > 0.0) {
        return None;
    }
    let pad = margin.max(0.0);
    Some(ViewportBounds {
        min_x: (-pan.0 / scale) - pad,
        min_y: (-pan.1 / scale) - pad,
        max_x: (-pan.0 + room.0) / scale + pad,
        max_y: (-pan.1 + room.1) / scale + pad,
    })
}

/// Returns true when a rectangle intersects `bounds`.
pub fn rect_intersects_viewport(x: f64, y: f64, w: f64, h: f64, bounds: ViewportBounds) -> bool {
    x + w >= bounds.min_x && x <= bounds.max_x && y + h >= bounds.min_y && y <= bounds.max_y
}

/// Counts how many laid-out nodes intersect `bounds`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn count_nodes_in_viewport(nodes: &[Placed], bounds: ViewportBounds) -> usize {
    nodes
        .iter()
        .filter(|placed| {
            rect_intersects_viewport(placed.x, placed.y, BOX_WIDTH, BOX_HEIGHT, bounds)
        })
        .count()
}

/// Groups secondary node overlays into a compact pill strip (`data-has-badges`, `data-badge-count`)
/// so the primary identifier row stays uncluttered.
pub(super) fn build_node_badges_strip(badges: &[String]) -> (String, String) {
    if badges.is_empty() {
        return (String::new(), String::new());
    }
    let attrs = format!(
        " data-has-badges=\"true\" data-badge-count=\"{}\"",
        badges.len()
    );
    let mut strip = String::from("<span class=\"graph-map-node-badges\" aria-hidden=\"true\">");
    for badge in badges {
        strip.push_str(badge);
    }
    strip.push_str("</span>");
    (attrs, strip)
}

pub(super) use crate::graph_viewport::hash_json_value;

pub(super) fn impact_canvas_digest<H: std::hash::Hasher>(
    lookup: Option<&ImpactLookup>,
    state: &mut H,
) {
    use std::hash::Hash;
    let Some(lookup) = lookup else {
        false.hash(state);
        return;
    };
    true.hash(state);
    lookup.uses_added.hash(state);
    lookup.uses_removed.hash(state);
    lookup.callers_added.hash(state);
    lookup.callers_risk.hash(state);
    lookup.commit_edges_added.hash(state);
    lookup.commit_edges_removed.hash(state);
    lookup.commit_risk_callers.hash(state);
}

/// Computes a deterministic structural digest of `view` and `overlay` excluding selection/cone/focus-target
/// highlights so pure state-attribute updates can skip rebuilding canvas `innerHTML`.
pub(super) fn canvas_structure_digest(view: &V, overlay: &CanvasOverlayContext<'_>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for key in [
        "mode",
        "scope",
        "whole",
        "modules",
        "notests",
        "certain",
        "history",
        "above",
        "spreads",
        "chosen",
        "pattern",
        "patterns",
        "architecture",
        "nodes",
        "edges",
        "frames",
    ] {
        hash_json_value(&view[key], &mut hasher);
    }
    overlay.links_off.hash(&mut hasher);
    overlay.hide_transitive.hash(&mut hasher);
    if let Some(changes) = overlay.changes {
        true.hash(&mut hasher);
        hash_json_value(changes, &mut hasher);
    } else {
        false.hash(&mut hasher);
    }
    let mut nodes = m::array(&view["nodes"]);
    let mut edges = m::array(&view["edges"]);
    let frames = m::array(&view["frames"]);
    inject_synthetic_deleted_nodes_and_edges(
        view,
        overlay.changes,
        &mut nodes,
        &mut edges,
        &frames,
    );
    let impact_lookup = ImpactLookup::from_sources(overlay.impact, overlay.changes, &nodes);
    impact_canvas_digest(impact_lookup.as_ref(), &mut hasher);
    if let Some(live) = overlay.live_focus {
        live.created_entities.len().hash(&mut hasher);
        for ent in &live.created_entities {
            ent.id.hash(&mut hasher);
            ent.kind.hash(&mut hasher);
            ent.name.hash(&mut hasher);
            ent.parent.hash(&mut hasher);
            ent.path.hash(&mut hasher);
            ent.folder.hash(&mut hasher);
            ent.line.hash(&mut hasher);
            ent.change.hash(&mut hasher);
            ent.visibility.hash(&mut hasher);
            ent.modifiers.hash(&mut hasher);
        }
    } else {
        0usize.hash(&mut hasher);
    }
    if let Some(pins) = overlay.pinned_positions {
        pins.len().hash(&mut hasher);
        for (k, (x, y)) in pins {
            k.hash(&mut hasher);
            x.to_bits().hash(&mut hasher);
            y.to_bits().hash(&mut hasher);
        }
    }
    if let Some(path_nodes) = overlay.path_nodes {
        path_nodes.len().hash(&mut hasher);
        for idx in path_nodes {
            idx.hash(&mut hasher);
        }
    }
    if let Some(iso) = overlay.isolate_cone {
        iso.len().hash(&mut hasher);
        for idx in iso {
            idx.hash(&mut hasher);
        }
    }
    hasher.finish()
}

mod link_inspector;
mod selection_patch;
mod side_cards;

pub use link_inspector::{
    ends, link_canvas_with, link_note, link_side_with, link_summary, link_title, Ends,
};
#[cfg(test)]
pub use link_inspector::{link_canvas, link_side};
pub use side_cards::{found_patterns, side};

#[allow(unused_imports)]
pub(super) use link_inspector::{
    diff_badge_sign, handles, link_group_members_html, link_names, merge_diff_priority,
    norm_sym_token, paths_overlap, LinkDiffInfo,
};
#[allow(unused_imports)]
pub(super) use selection_patch::{
    compute_canvas_selection_patch, compute_selected_bbox, radar_box_state,
    refresh_dense_scene_for_patch, CanvasSelectionPatch, EdgeSelectionState, NodeSelectionState,
    SelectionContext,
};
#[allow(unused_imports)]
pub(super) use side_cards::{
    apart, architecture_cut_hints_html, ask_shared, asks, blast_marks, blast_side, circle_told,
    circles_side, confidence_pill_html, crumbs, cycle_cut_hints_html, file_declared_members_html,
    file_side, focus_member_meta_html, follow_links, html, kinds, layers_side, legend, name_side,
    open_file, pair_rows, pair_rows_of, path_end, path_to, pattern_brief, pattern_cut_hints_html,
    pattern_row, pattern_side, reached, role, rows, symbol_focus_payload, FILE_LEGEND, FIRST,
    LAYER_LEGEND, LEGEND, NAME_LEGEND, WAY,
};

/// Renders a dependency map view using the default `"graph-map"` SVG ID prefix.
#[cfg(test)]
pub fn canvas(view: &V) -> String {
    canvas_with(view, "graph-map", None).html
}
/// Renders a dependency map plane with parameterized SVG IDs and optional live focus overlays.
#[cfg(test)]
pub fn canvas_with(view: &V, id_prefix: &str, live_focus: Option<&LiveFocus>) -> RenderedCanvas {
    canvas_with_overlays(
        view,
        id_prefix,
        CanvasOverlayContext {
            live_focus,
            ..Default::default()
        },
    )
}
pub(super) fn inject_synthetic_deleted_nodes_and_edges(
    view: &V,
    changes: Option<&V>,
    nodes: &mut Vec<V>,
    edges: &mut Vec<V>,
    frames: &[V],
) {
    let Some(changes_doc) = changes else {
        return;
    };
    if !plain_map(view) || symbols(view) || follows(view) {
        return;
    }
    let commit_or_change_view = view["chosen"].is_object()
        || self::changes(view)
        || matches!(changes_doc["target"].as_str(), None | Some(""))
        || changes_doc["target"].as_str().is_some_and(|t| t != "work");
    if !commit_or_change_view {
        return;
    }
    let files = m::array(&changes_doc["files"]);
    if files.is_empty() {
        return;
    }
    let scope = normalize_rel_path(&m::text(view, "scope"));
    let scope_prefix = if scope.is_empty() {
        String::new()
    } else {
        format!("{scope}/")
    };
    let mut existing_paths: BTreeSet<String> = nodes
        .iter()
        .map(|node| normalize_rel_path(&m::text(node, "path")))
        .filter(|path| !path.is_empty())
        .collect();
    let frame_paths: BTreeSet<String> = frames
        .iter()
        .map(|frame| normalize_rel_path(&m::text(frame, "path")))
        .filter(|path| !path.is_empty())
        .collect();
    let mut edge_diff_callers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for bucket in [
        &changes_doc["edge_diffs"]["removed"],
        &changes_doc["edge_diffs"]["risk_callers"],
    ] {
        for item in m::array(bucket) {
            let from = normalize_rel_path(strip_symbol_suffix(&m::text(&item, "from")));
            let to = normalize_rel_path(strip_symbol_suffix(&m::text(&item, "to")));
            if !from.is_empty() && !to.is_empty() {
                let list = edge_diff_callers.entry(to).or_default();
                if !list.contains(&from) {
                    list.push(from);
                }
            }
        }
    }

    let mut injected: Vec<(String, Vec<String>)> = Vec::new();
    for file in files {
        if normalize_change_status(&m::text(&file, "status")) != "deleted" {
            continue;
        }
        let path = normalize_rel_path(&m::text(&file, "path"));
        if path.is_empty() || existing_paths.contains(&path) {
            continue;
        }
        if !scope_prefix.is_empty() && !path.starts_with(&scope_prefix) {
            continue;
        }
        let parent_dir = parent(&path);
        let collapsed_ancestor = nodes.iter().any(|node| {
            m::yes(&node["folder"])
                && !frame_paths.contains(&normalize_rel_path(&m::text(node, "path")))
                && path_matches_node(&m::text(node, "path"), true, &path)
        });
        if scope != parent_dir && collapsed_ancestor {
            continue;
        }
        let holder = if !parent_dir.is_empty() && frame_paths.contains(parent_dir) {
            json!(parent_dir)
        } else {
            V::Null
        };
        let mut callers = Vec::new();
        for caller in m::array(&file["callers"]) {
            if let Some(raw) = caller.as_str() {
                let norm = normalize_rel_path(strip_symbol_suffix(raw));
                if !norm.is_empty() && !callers.contains(&norm) {
                    callers.push(norm);
                }
            }
        }
        if let Some(extra) = edge_diff_callers.get(&path) {
            for norm in extra {
                if !callers.contains(norm) {
                    callers.push(norm.clone());
                }
            }
        }
        nodes.push(json!({
            "path": path,
            "name": base(&path),
            "folder": false,
            "files": 1,
            "symbols": 0,
            "within": 0,
            "out": 0,
            "in": callers.len(),
            "changed": 1,
            "status": "D",
            "kind": "file",
            "holder": holder,
            "synthetic_deleted": true,
        }));
        existing_paths.insert(path.clone());
        injected.push((path, callers));
    }

    if injected.is_empty() {
        return;
    }
    let mut existing_edges: BTreeSet<(String, String)> = edges
        .iter()
        .map(|edge| {
            (
                normalize_rel_path(&m::text(edge, "from")),
                normalize_rel_path(&m::text(edge, "to")),
            )
        })
        .collect();
    for (del_path, callers) in injected {
        for caller in callers {
            if let Some(caller_idx) = resolve_canvas_node_by_path(nodes, &caller) {
                let from_path = m::text(&nodes[caller_idx], "path");
                let norm_from = normalize_rel_path(&from_path);
                if !from_path.is_empty()
                    && norm_from != del_path
                    && existing_edges.insert((norm_from, del_path.clone()))
                {
                    edges.push(json!({
                        "from": from_path,
                        "to": del_path,
                        "links": 1,
                        "guessed": 0,
                        "kinds": [{"kind": "calls", "links": 1}],
                        "synthetic_diff": "removed",
                    }));
                }
            }
        }
    }
}

/// Filtered graph arrays extracted from a view for canvas layout, patching, and pair queries.
pub(super) struct ExtractedViewGraphParts {
    pub(super) nodes: Vec<V>,
    pub(super) edges: Vec<V>,
    pub(super) frames: Vec<V>,
    pub(super) pairs: Vec<(usize, usize)>,
}

/// Deduplicates filtered graph extraction across `canvas_with_overlays` and `surfaces::pairs_for_view`.
pub(super) fn extract_view_graph_parts(
    view: &V,
    changes: Option<&V>,
    hide_transitive: bool,
) -> ExtractedViewGraphParts {
    let mut nodes = m::array(&view["nodes"]);
    let mut edges = m::array(&view["edges"]);
    let frames = m::array(&view["frames"]);
    inject_synthetic_deleted_nodes_and_edges(view, changes, &mut nodes, &mut edges, &frames);
    let path_index = build_path_index(&nodes);
    let at = |path: &V| lookup_path_index(&nodes, &path_index, path);
    let plain = plain_map(view);
    let mut left_out = if hide_transitive && plain {
        said_twice(view)
    } else {
        vec![false; edges.len()]
    };
    left_out.resize(edges.len(), false);
    let mut filtered_edges = Vec::with_capacity(edges.len());
    let mut pairs = Vec::with_capacity(edges.len());
    for (edge, said) in edges.into_iter().zip(&left_out) {
        if *said {
            continue;
        }
        if let (Some(u), Some(v)) = (at(&edge["from"]), at(&edge["to"])) {
            pairs.push((u, v));
            filtered_edges.push(edge);
        }
    }
    ExtractedViewGraphParts {
        nodes,
        edges: filtered_edges,
        frames,
        pairs,
    }
}

/// Renders a dependency map plane with live focus, changed-files markers, and file-impact diff edges.
pub fn canvas_with_overlays(
    view: &V,
    id_prefix: &str,
    overlay: CanvasOverlayContext<'_>,
) -> RenderedCanvas {
    let live_focus = overlay.live_focus;
    let ExtractedViewGraphParts {
        nodes,
        edges: edges_owned,
        frames,
        pairs,
    } = extract_view_graph_parts(view, overlay.changes, overlay.hide_transitive);
    let path_index = build_path_index(&nodes);
    let at = |path: &V| lookup_path_index(&nodes, &path_index, path);
    let changes_lookup = overlay.changes.and_then(ChangesLookup::from_value);
    let impact_lookup = ImpactLookup::from_sources(overlay.impact, overlay.changes, &nodes);
    let plain = plain_map(view);
    // A link whose end is not drawn is not drawn either. Nor is, where
    // that was asked for, a link that a longer way already joins: without
    // those the picture holds fewer links that pass layers, and every box
    // still reaches what it reached.
    let edges: Vec<&V> = edges_owned.iter().collect();
    let weighted = pairs
        .iter()
        .zip(&edges)
        .map(|((from, to), edge)| (*from, *to, edge["links"].as_u64().unwrap_or(1)))
        .collect::<Vec<_>>();
    // A folder that is spread out is a frame around what it holds.
    let frame_index = build_path_index(&frames);
    let frame = |path: &V| lookup_path_index(&frames, &frame_index, path);
    let framed = Frames {
        of: nodes.iter().map(|node| frame(&node["holder"])).collect(),
        holder: frames.iter().map(|held| frame(&held["holder"])).collect(),
    };
    // The two places that share what else is drawn stand side by side:
    // before what both use, or after what uses both.
    let two = |key: &str| nodes.iter().position(|node| m::yes(&node[key]));
    let pair = two("start")
        .zip(two("end"))
        .filter(|_| shares(view))
        .map(|(first, second)| ([first, second], above(view)));
    // Layers stand in a row in their own order, whatever their links say.
    let layered_view = layered(view);
    let declared = m::yes(&view["pattern"]["declared"]);
    // In a picture of a way, of a cycle, of what two places share, of a
    // pattern or of a scenario every link is what is asked for: there no
    // link is held back until its box is rested on.
    let routing = if plain {
        Routing::CLEAN
    } else {
        Routing {
            portals: None,
            ..Routing::CLEAN
        }
    };
    let mut plan = if layered_view && !declared {
        plan_row(nodes.len(), &weighted)
    } else {
        let plan = plan_routed(nodes.len(), &weighted, &framed, pair, routing);
        // What a session made hangs under the box of its file as cards.
        // Where there are some, the picture is laid out again with room
        // for them, so that they lie on no other box.
        let under = live_focus
            .map(|focus| resolve_focus(view, &plan, focus).under)
            .unwrap_or_default();
        if under.is_empty() {
            plan
        } else {
            let mut room = vec![0.0; nodes.len()];
            for (node, cards) in under {
                if let Some(room) = room.get_mut(node) {
                    *room = entity_room(cards);
                }
            }
            plan_roomy(nodes.len(), &weighted, &framed, pair, routing, &room)
        }
    };
    let node_keys: Vec<String> = nodes
        .iter()
        .map(|node| {
            let p = m::text(node, "path");
            if p.is_empty() {
                m::text(node, "name")
            } else {
                p
            }
        })
        .collect();
    let node_labels: Vec<String> = nodes.iter().map(|node| m::text(node, "name")).collect();
    let pinned_set = if let Some(pins) = overlay.pinned_positions {
        apply_pin_overrides_to_plan(&mut plan, &node_keys, &pairs, pins)
    } else {
        BTreeSet::new()
    };
    // How much each box uses and is used through the links drawn here.
    let mut flow = vec![(0u64, 0u64); nodes.len()];
    for (from, to, links) in &weighted {
        if from != to {
            flow[*from].0 += (*links).max(1);
            flow[*to].1 += (*links).max(1);
        }
    }
    // A link starts in the colour every link leaves in and ends in the
    // colour of its kind. It shades along the way between its two columns,
    // so the links of one kind between the same two columns share one
    // shading.
    let span = |wire: &Wire| {
        let (from, to) = pairs[wire.edge];
        let (start, end) = (plan.nodes[from].x, plan.nodes[to].x);
        if wire.back {
            (start.round() as i64, (end + BOX_WIDTH).round() as i64)
        } else {
            ((start + BOX_WIDTH).round() as i64, end.round() as i64)
        }
    };
    let mut spans = plan.wires.iter().map(span).collect::<Vec<_>>();
    spans.sort_unstable();
    spans.dedup();
    // The kinds each link holds, and the kind it is drawn as: the
    // strongest of those the key of the picture has not switched off.
    let links_off = overlay.links_off;
    let wire_kinds = plan
        .wires
        .iter()
        .map(|wire| EdgeKinds::of(edges[wire.edge]))
        .collect::<Vec<_>>();
    let shadings: BTreeSet<(usize, &'static str)> = plan
        .wires
        .iter()
        .zip(&wire_kinds)
        .filter_map(|(wire, kinds)| {
            Some((
                spans.binary_search(&span(wire)).unwrap_or(0),
                kinds.primary(&links_off)?,
            ))
        })
        .collect();
    let focus = view["focus"]["path"].as_str();
    let chosen = focus.and_then(|path| at(&json!(path)));
    // Among layers a link is selected too: its two ends.
    let chosen_link = Some(&view["focus"])
        .filter(|focus| m::yes(&focus["link"]))
        .and_then(|focus| Some((at(&focus["from"]["path"])?, at(&focus["to"]["path"])?)));
    let selected = chosen.is_some() || chosen_link.is_some();
    // A folder of changed files both uses and is used by the files around
    // it, so in a change map a link running back is no finding; nor is it
    // around an opened file.
    let opened = symbols(view);
    let followed = follows(view);
    let pathed = paths(view);
    let shared = shares(view);
    let cycle_view = cycles(view);
    let pattern_view = patterned(view);
    let scenario = view["mode"] == "scenario";
    let cycles = !changes(view) && !opened;
    let shared_edges: BTreeSet<_> = view["focus"]["shared_edges"]
        .as_array()
        .filter(|_| shared && chosen.is_some())
        .into_iter()
        .flatten()
        .filter_map(|edge| Some((at(&edge[0])?, at(&edge[1])?)))
        .collect();
    // The links of the selected box are lit, and, where more was asked
    // for, those that lead on from them: so many links along the arrows
    // to what it uses, and so many against them to what uses it.
    let reach = (overlay.cone.0.max(1), overlay.cone.1.max(1));
    let depth = chosen
        .filter(|_| plain && shared_edges.is_empty() && reach != (1, 1))
        .map(|chosen| cone(nodes.len(), &pairs, chosen, reach.0, reach.1));
    let lit = |pair: (usize, usize)| {
        cycle_view
            || chosen_link == Some(pair)
            || match (&depth, chosen) {
                (Some(depth), _) => in_cone(depth, pair),
                (None, Some(chosen)) if shared_edges.is_empty() => {
                    pair.0 == chosen || pair.1 == chosen
                }
                _ => shared_edges.contains(&pair),
            }
    };
    let mut near_nodes = vec![false; nodes.len()];
    if let Some(chosen_idx) = chosen {
        if chosen_idx < near_nodes.len() {
            near_nodes[chosen_idx] = true;
        }
    }
    for &(from, to) in &pairs {
        if lit((from, to)) {
            if let Some(slot) = near_nodes.get_mut(from) {
                *slot = true;
            }
            if let Some(slot) = near_nodes.get_mut(to) {
                *slot = true;
            }
        }
    }
    let near = |node: usize| near_nodes.get(node).copied().unwrap_or(false);
    // Among followed symbols a name too long for its box keeps its end,
    // where the method stands after its class.
    let named = names(view);
    let pattern_family = pattern_view
        .then(|| m::text(&view["pattern"], "family"))
        .filter(|family| !family.is_empty());
    let node_change_statuses: Vec<Option<String>> = nodes
        .iter()
        .map(|node| {
            changes_lookup
                .as_ref()
                .and_then(|lookup| lookup.status_for_node(node, &nodes))
        })
        .collect();
    let node_is_changed: Vec<bool> = nodes
        .iter()
        .zip(&node_change_statuses)
        .map(|(node, status)| {
            node["changed"].as_u64().unwrap_or(0) > 0
                || m::yes(&node["changed"])
                || status.is_some()
        })
        .collect();
    let commit_changes_active = view["chosen"].is_object()
        || changes(view)
        || overlay.impact.is_some()
        || overlay.changes.is_some_and(|doc| {
            let target = m::text(doc, "target");
            (!target.is_empty() && target != "work")
                || doc["edge_diffs"].is_object()
                || (doc["target"].is_null() && node_change_statuses.iter().any(Option::is_some))
        });
    let visible_edges_count = wire_kinds
        .iter()
        .filter(|kinds| kinds.primary(&links_off).is_some())
        .count();
    let scale_tier = graph_scale_tier(nodes.len(), visible_edges_count);
    let mut plane_attrs = String::new();
    if named {
        plane_attrs.push_str(" data-names=\"true\"");
    }
    if scale_tier != GraphScaleTier::Normal {
        plane_attrs.push_str(&format!(" data-scale-tier=\"{}\"", scale_tier.as_str()));
    }
    if cycle_view {
        plane_attrs.push_str(" data-cycle-view=\"true\"");
    }
    if commit_changes_active {
        plane_attrs.push_str(" data-commit-changes=\"true\"");
    }
    if let Some(family) = &pattern_family {
        plane_attrs.push_str(&format!(
            " data-pattern-family=\"{}\" data-pattern-sort=\"{}\"",
            escape(family),
            escape(&m::text(&view["pattern"], "sort")),
        ));
    }
    if layered_view {
        plane_attrs.push_str(" data-layered=\"true\"");
        if m::yes(&view["pattern"]["rings"]) {
            plane_attrs.push_str(" data-rings=\"true\"");
        }
        if declared {
            plane_attrs.push_str(" data-declared=\"true\"");
        }
    }
    if view["architecture"].is_object() {
        plane_attrs.push_str(" data-architecture=\"true\"");
    }
    if view["patterns"].is_object() {
        plane_attrs.push_str(" data-patterns-on=\"true\"");
    }
    let focus_overlay = live_focus
        .map(|focus| resolve_focus(view, &plan, focus))
        .unwrap_or_default();
    let (mut plane_w, mut plane_h) = (plan.width, plan.height);
    for entity in &focus_overlay.entities {
        plane_w = plane_w.max(entity.x + ENTITY_WIDTH + MARGIN);
        plane_h = plane_h.max(entity.y + ENTITY_HEIGHT + MARGIN);
    }
    let mut html = format!(
        "<div class=\"graph-map-plane\"{plane_attrs} style=\"width:{:.0}px;height:{:.0}px\"><svg width=\"{:.0}\" height=\"{:.0}\" aria-hidden=\"true\"><defs>",
        plane_w, plane_h, plane_w, plane_h
    );
    // The head of an arrow lies at what is used.
    let include_against = layered_view
        || view["architecture"].is_object()
        || edges
            .iter()
            .any(|edge| edge["architecture_violation"].as_u64().unwrap_or(0) > 0);
    html.push_str(&arrow_defs_svg(id_prefix, include_against));
    for (index, kind) in &shadings {
        let (start, end) = spans[*index];
        let (suffix, kind_attr) = if *kind == "calls" {
            (String::new(), String::new())
        } else {
            (format!("-{kind}"), format!(" data-edge-kind=\"{kind}\""))
        };
        html.push_str(&format!(
            "<linearGradient id=\"{id_prefix}-flow-{index}{suffix}\" class=\"graph-map-flow\"{kind_attr} gradientUnits=\"userSpaceOnUse\" x1=\"{start}\" y1=\"0\" x2=\"{end}\" y2=\"0\"><stop offset=\"0\" class=\"graph-map-flow-uses\"/><stop offset=\"0.5\" class=\"graph-map-flow-between\"/><stop offset=\"1\" class=\"graph-map-flow-used\"/></linearGradient>"
        ));
    }
    html.push_str("</defs>");
    if layered_view {
        if let (Some(first), Some(last)) = (plan.nodes.first(), plan.nodes.last()) {
            html.push_str(&format!(
                "<rect class=\"graph-map-arch-band\" x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" rx=\"10\"/>",
                first.x - 6.0,
                first.y - 6.0,
                (last.x + BOX_WIDTH - first.x) + 12.0,
                BOX_HEIGHT + 12.0,
            ));
        }
    }
    for (index, place) in plan.frames.iter().enumerate() {
        if place.width > 0.0 {
            let role_attrs = pattern_family
                .as_deref()
                .map_or_else(String::new, |family| {
                    let role_frame = if frames[index]["path"] == "role:0" {
                        "anchor"
                    } else {
                        "part"
                    };
                    format!(
                        " data-pattern-family=\"{}\" data-role-frame=\"{role_frame}\"",
                        escape(family)
                    )
                });
            let focus_attr = if focus_overlay.frames.contains(&index) {
                " data-focus-target=\"true\""
            } else {
                ""
            };
            html.push_str(&format!(
                "<rect class=\"graph-map-frame\" data-inner=\"{}\"{role_attrs}{focus_attr} x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" rx=\"8\"/>",
                framed.holder[index].is_some(),
                place.x,
                place.y,
                place.width,
                place.height
            ));
        }
    }
    if let Some((x0, y0, x1, y1)) = focus_overlay.bbox {
        let pad = 14.0;
        let zx = (x0 - pad).max(2.0);
        let zy = (y0 - pad).max(2.0);
        let zw = (x1 + pad - zx).max(BOX_WIDTH);
        let zh = (y1 + pad - zy).max(BOX_HEIGHT);
        html.push_str(&format!(
            "<rect class=\"graph-map-target-zone\" x=\"{zx:.1}\" y=\"{zy:.1}\" width=\"{zw:.1}\" height=\"{zh:.1}\" rx=\"12\"/>"
        ));
    }
    // Beside a link against the order of the layers: how many of its links
    // a chosen change added. They lie over all the links, and each is lit
    // and steps back with its link.
    let mut marks = String::new();
    let mut placed_ties = Vec::new();
    let mut hidden_wires = Vec::with_capacity(plan.wires.len());
    let mut dense_edges = Vec::new();
    let is_dense = scale_tier == GraphScaleTier::Dense;
    let mut dense_meta = graph_viewport::canvas2d::DenseSceneMeta {
        label_min_scale: Some(0.24),
        commit_changes_active,
        ..Default::default()
    };
    // What each box is joined with by links that are led through no layer:
    // the boxes it uses far away, and the boxes far away that use it.
    let mut far: Vec<(Vec<usize>, Vec<usize>)> = vec![(vec![], vec![]); nodes.len()];
    for (wire, kinds) in plan.wires.iter().zip(&wire_kinds) {
        let edge = edges[wire.edge];
        let (links, guessed) = (
            edge["links"].as_u64().unwrap_or(0),
            edge["guessed"].as_u64().unwrap_or(0),
        );
        let (from, to) = pairs[wire.edge];
        if let Some(cone) = overlay.isolate_cone {
            if !cone.contains(&from) || !cone.contains(&to) {
                hidden_wires.push(true);
                continue;
            }
        }
        // Among layers a link back is no weaker direction of a cycle: it
        // runs against their order, under the row.
        let violation = edge["architecture_violation"].as_u64().unwrap_or(0);
        let against = if violation > 0 {
            true
        } else if declared {
            m::yes(&edge["against"])
        } else {
            layered_view && wire.back
        };
        let grew = edge["added"].as_u64().filter(|_| against);
        // Of a path, a link that only a longer way has.
        let detour_attr = if m::yes(&edge["detour"]) {
            " data-detour=\"true\""
        } else {
            ""
        };
        let back = wire.back && cycles && !layered_view;
        let on_traced_path = overlay
            .path_edges
            .is_some_and(|pe| pe.contains(&(from, to)) || pe.contains(&(to, from)));
        let on_path = shared_edges.contains(&(from, to)) || on_traced_path;
        let added = cycle_view && m::yes(&edge["added"]);
        let has_traced_path = overlay.path_nodes.is_some_and(|pn| !pn.is_empty());
        let edge_state = if has_traced_path {
            if on_traced_path {
                "on"
            } else {
                "off"
            }
        } else if cycle_view || selected {
            if lit((from, to)) {
                "on"
            } else {
                "off"
            }
        } else {
            ""
        };
        let state = match edge_state {
            "on" => " graph-map-edge-on",
            "off" => " graph-map-edge-off",
            _ => "",
        };
        let edge_state_attr = if edge_state.is_empty() {
            String::new()
        } else {
            format!(" data-edge-state=\"{edge_state}\"")
        };
        // Between two parts of a pattern a link says how the one stands to
        // the other.
        let (user, used) = (
            escape(&m::text(&nodes[from], "name")),
            escape(&m::text(&nodes[to], "name")),
        );
        let told = match edge["tie"].as_str() {
            Some(tie) => format!(
                "{user} {} {used}{}",
                escape(tie),
                if guessed > 0 { " · guessed" } else { "" }
            ),
            None => format!("{user} uses {used} · {}", weight(links, guessed)),
        };
        let synthetic_diff = edge["synthetic_diff"].as_str();
        let edge_diff = impact_lookup
            .as_ref()
            .and_then(|lookup| lookup.edge_diff(&nodes, from, to))
            .map(|diff| {
                if diff == "unchanged" {
                    synthetic_diff.unwrap_or("unchanged")
                } else {
                    diff
                }
            })
            .or(synthetic_diff);
        let edge_impact = impact_lookup
            .as_ref()
            .and_then(|lookup| lookup.edge_impact(&nodes, from, to));
        let commit_touched =
            commit_changes_active && (node_is_changed[from] || node_is_changed[to]);
        let edge_impact_attr = edge_impact
            .map(|impact| format!(" data-impact=\"{impact}\""))
            .unwrap_or_default();
        let commit_touched_attr = if commit_touched {
            " data-commit-touched=\"true\""
        } else {
            ""
        };
        let edge_diff_attr = edge_diff
            .map(|diff| format!(" data-diff=\"{diff}\""))
            .unwrap_or_default();
        // A link whose every kind the key has switched off stays in the
        // picture, so that nothing moves, and is not drawn. A link against
        // the rules is drawn whatever the key says.
        let shown_kind = kinds.primary(&links_off);
        let hidden = shown_kind.is_none() && !against;
        let hidden_attr = if hidden {
            " data-kind-hidden=\"true\""
        } else {
            ""
        };
        hidden_wires.push(hidden);
        if against && !is_dense {
            html.push_str(&format!(
                "<path class=\"graph-map-violation-halo\"{edge_state_attr} data-ends=\"{from} {to}\" d=\"{}\"/>",
                wire.path,
            ));
        }
        let arrow_kind = shown_kind.unwrap_or_else(|| pick_precedence_kind(&kinds.counts));
        let multi_kind = kinds.multi(&links_off);
        let multi_attr = if multi_kind {
            " data-edge-multi=\"true\""
        } else {
            ""
        };
        let marker_suffix = arrow_marker_suffix(arrow_kind, against);
        let breakdown_note = kinds
            .breakdown
            .as_deref()
            .map(|text| format!(" · {}", escape(text)))
            .unwrap_or_default();
        // What the tool only inferred is drawn hollow: the link a little
        // wider, and over it its own middle in the colour of the ground.
        let guess = guessed == links;
        let width = (1.0 + ((links.max(1) as f64).log2() * 0.5).min(2.5))
            // A link that closes a cycle is a row of dots, kept thin.
            .min(if back { 1.8 } else { 3.5 })
            + if added { 1.5 } else { 0.0 };
        // A link that passes many layers is led through none of them: it
        // is drawn from end to end while its box is rested on or selected,
        // and a mark at each of its ends says that it is there. One against
        // the rules is always drawn.
        let portal = wire.portal && !against;
        if is_dense && !hidden {
            let mut summary = format!("{user} -> {used} ({arrow_kind}");
            if links > 1 {
                summary.push_str(&format!(", {links}"));
            }
            if against {
                summary.push_str(", violation");
            }
            if let Some(d) = edge_diff.filter(|d| *d != "unchanged") {
                summary.push_str(&format!(", {d}"));
            }
            summary.push(')');
            dense_meta.edges.insert(
                wire.edge,
                graph_viewport::canvas2d::DenseEdgeMeta {
                    explain_kind: "link".to_owned(),
                    explain_payload: json!({"from": nodes[from]["path"], "to": nodes[to]["path"]}),
                    from_index: Some(from),
                    to_index: Some(to),
                    summary,
                },
            );
            dense_edges.push(
                graph_viewport::canvas2d::DenseEdgeSpec::from_svg_path(
                    wire.edge,
                    &wire.path,
                    links.max(1),
                    guess,
                    against,
                    arrow_kind.to_owned(),
                    edge_state == "on" || on_path,
                    lit((from, to)),
                    edge_state == "off",
                    portal,
                )
                .with_semantics(
                    edge_diff.unwrap_or(""),
                    edge_impact.unwrap_or(""),
                    commit_touched,
                    back,
                    m::yes(&edge["detour"]),
                    kinds.held_vec(),
                ),
            );
        }
        if !is_dense {
            html.push_str(&format!(
            "<path class=\"graph-map-edge{state}{}{}{}{}\" data-ends=\"{from} {to}\"{edge_impact_attr}{commit_touched_attr}{edge_diff_attr}{}{}{hidden_attr}{detour_attr} d=\"{}\" data-edge-kind=\"{arrow_kind}\" data-kinds=\"{}\"{multi_attr} stroke=\"url(#{id_prefix}-flow-{}{})\" stroke-width=\"{:.1}\" marker-start=\"url(#{id_prefix}-port{})\" marker-end=\"url(#{id_prefix}-arrow{marker_suffix})\"><title>{told}{}{}{}{breakdown_note}</title></path>",
            if guess { " graph-map-edge-guess" } else { "" },
            if back { " graph-map-edge-back" } else { "" },
            if against { " graph-map-edge-against" } else { "" },
            if portal { " graph-map-edge-portal" } else { "" },
            if on_path { " data-shared-path=\"true\"" } else { "" },
            if added { " data-cycle-added=\"true\"" } else { "" },
            wire.path,
            kinds.held(),
            spans.binary_search(&span(wire)).unwrap_or(0),
            if arrow_kind == "calls" { String::new() } else { format!("-{arrow_kind}") },
            width + if guess { 2.0 } else { 0.0 },
            if multi_kind { "-multi" } else { "" },
            if !detour_attr.is_empty() {
                " · on a longer way only"
            } else if back {
                " · against the main direction, closes a cycle"
            } else {
                ""
            },
            match grew {
                Some(links) => format!(
                    " · against the order of the layers · {} added by this change",
                    plural(links, "link", "links")
                ),
                None if violation > 0 => format!(" · {} against layer rules", plural(violation, "link", "links")),
                None if against && declared => " · forbidden by project rules".to_owned(),
                None if against => " · against the order of the layers".to_owned(),
                None => String::new(),
            },
            if added { " · added in PR" } else { "" },
        ));
            if guess {
                html.push_str(&format!(
                "<path class=\"graph-map-edge-core{}{}{}\" data-ends=\"{from} {to}\"{hidden_attr} d=\"{}\" data-edge-kind=\"{arrow_kind}\" stroke-width=\"{width:.1}\"/>",
                if back { " graph-map-edge-back" } else { "" },
                if portal { " graph-map-edge-portal" } else { "" },
                if portal { state } else { "" },
                wire.path,
            ));
            }
        }
        if portal && !hidden {
            far[from].0.push(to);
            far[to].1.push(from);
        }
        // What stands at the middle of a link: a word of a pattern, what a
        // change did to it, or, on a link that passes layers and carries
        // no word, a chevron that says which way it runs. In dense mode
        // Canvas2D draws edges and badges directly so SVG mark strings stay unallocated.
        if !is_dense {
            let mut worded = false;
            if pattern_view && !layered_view {
                if let Some((tie, (x, y))) = edge["tie"]
                    .as_str()
                    .zip(middle(&wire.path).and_then(|mid| place_tie(&mut placed_ties, mid)))
                {
                    worded = true;
                    marks.push_str(&format!(
                        "<text class=\"graph-map-edge-tie\"{edge_state_attr} data-ends=\"{from} {to}\" data-pattern-family=\"{}\"{hidden_attr} x=\"{x:.1}\" y=\"{y:.1}\">{}</text>",
                        escape(pattern_family.as_deref().unwrap_or("design")),
                        escape(&short_tie(tie)),
                    ));
                }
            }
            if let Some((links, (x, y))) = grew.zip(middle(&wire.path)) {
                worded = true;
                marks.push_str(&format!(
                    "<text class=\"graph-map-edge-mark{state}\" data-ends=\"{from} {to}\" x=\"{x:.1}\" y=\"{y:.1}\">+{}</text>",
                    count(links)
                ));
            } else if let Some((diff, (x, y))) = edge_diff
                .filter(|diff| matches!(*diff, "added" | "removed"))
                .zip(middle(&wire.path))
            {
                worded = true;
                let symbol = if diff == "added" { "+" } else { "-" };
                marks.push_str(&format!(
                    "<text class=\"graph-map-edge-mark{state}\" data-ends=\"{from} {to}\" data-diff=\"{diff}\"{hidden_attr} x=\"{x:.1}\" y=\"{y:.1}\">{symbol}</text>"
                ));
            }
            if !worded
                && passes_layers(&wire.path)
                && (!scale_tier.elide_non_essential_marks() || against || lit((from, to)))
            {
                if let Some((x, y, angle)) = halfway(&wire.path) {
                    marks.push_str(&format!(
                        "<path class=\"graph-map-edge-chevron{state}{}\" data-ends=\"{from} {to}\" data-edge-kind=\"{arrow_kind}\" data-kinds=\"{}\"{hidden_attr} transform=\"translate({x:.1} {y:.1}) rotate({angle:.0})\" d=\"M-3 -3.4 L1.4 0 L-3 3.4\"/>",
                        if against { " graph-map-edge-against" } else { "" },
                        kinds.held(),
                    ));
                }
            }
        }
    }
    if let Some(lookup) = impact_lookup.as_ref() {
        let rendered_pairs: BTreeSet<(usize, usize)> = pairs.iter().copied().collect();
        let mut synthetic_drawn: BTreeSet<(usize, usize, &'static str)> = BTreeSet::new();
        let mut synthetic_seq = pairs.len();
        let mut add_synthetic = |from_idx: usize,
                                 to_idx: usize,
                                 diff: &'static str,
                                 label: &str,
                                 kind: &'static str,
                                 impact: Option<&'static str>| {
            if let Some(cone) = overlay.isolate_cone {
                if !cone.contains(&from_idx) || !cone.contains(&to_idx) {
                    return;
                }
            }
            if from_idx == to_idx
                || rendered_pairs.contains(&(from_idx, to_idx))
                || !synthetic_drawn.insert((from_idx, to_idx, diff))
            {
                return;
            }
            let d = synthetic_edge_path(from_idx, to_idx, &plan.nodes, synthetic_drawn.len());
            let from_name = m::text(&nodes[from_idx], "name");
            let to_name = m::text(&nodes[to_idx], "name");
            let is_touched =
                commit_changes_active && (node_is_changed[from_idx] || node_is_changed[to_idx]);
            if is_dense {
                let syn_idx = synthetic_seq;
                synthetic_seq += 1;
                dense_meta.edges.insert(
                    syn_idx,
                    graph_viewport::canvas2d::DenseEdgeMeta {
                        explain_kind: "link".to_owned(),
                        explain_payload: json!({
                            "from": nodes[from_idx]["path"],
                            "to": nodes[to_idx]["path"],
                        }),
                        from_index: Some(from_idx),
                        to_index: Some(to_idx),
                        summary: format!("{from_name} -> {to_name} ({label} · {kind})"),
                    },
                );
                dense_edges.push(
                    graph_viewport::canvas2d::DenseEdgeSpec::from_svg_path(
                        syn_idx,
                        &d,
                        1,
                        false,
                        impact == Some("broken_risk"),
                        kind.to_owned(),
                        false,
                        lit((from_idx, to_idx)),
                        false,
                        false,
                    )
                    .with_semantics(
                        diff,
                        impact.unwrap_or(""),
                        is_touched,
                        false,
                        false,
                        vec![kind.to_owned()],
                    ),
                );
            }
            let mark_symbol = if diff == "added" { "+" } else { "-" };
            let impact_attr = impact
                .map(|imp| format!(" data-impact=\"{imp}\""))
                .unwrap_or_default();
            let touched_attr = if is_touched {
                " data-commit-touched=\"true\""
            } else {
                ""
            };
            if !is_dense {
                html.push_str(&format!(
                    "<path class=\"graph-map-edge graph-map-edge-synthetic\" data-ends=\"{from_idx} {to_idx}\"{impact_attr}{touched_attr} data-diff=\"{diff}\" d=\"{d}\" data-edge-kind=\"{kind}\" data-kinds=\"{kind}\" stroke-width=\"2\" marker-end=\"url(#{id_prefix}-arrow{})\"><title>{} -> {} ({label} · {kind})</title></path>",
                    arrow_marker_suffix(kind, false),
                    escape(&from_name),
                    escape(&to_name),
                ));
                if let Some((mx, my)) = middle(&d) {
                    marks.push_str(&format!(
                        "<text class=\"graph-map-edge-mark\" data-ends=\"{from_idx} {to_idx}\" data-diff=\"{diff}\" x=\"{mx:.1}\" y=\"{my:.1}\">{mark_symbol}</text>"
                    ));
                }
            }
        };
        if let Some(sel_idx) = lookup.selected_idx {
            for target in &lookup.uses_added {
                if let Some(target_idx) = resolve_canvas_node_by_path(&nodes, target) {
                    add_synthetic(
                        sel_idx,
                        target_idx,
                        "added",
                        "added dependency",
                        lookup.dep_kind(target, true),
                        None,
                    );
                }
            }
            for target in &lookup.uses_removed {
                if let Some(target_idx) = resolve_canvas_node_by_path(&nodes, target) {
                    add_synthetic(
                        sel_idx,
                        target_idx,
                        "removed",
                        "removed dependency",
                        lookup.dep_kind(target, false),
                        None,
                    );
                }
            }
            for caller in &lookup.callers_added {
                if let Some(caller_idx) = resolve_canvas_node_by_path(&nodes, caller) {
                    add_synthetic(caller_idx, sel_idx, "added", "added caller", "calls", None);
                }
            }
            for caller in &lookup.callers_risk {
                if let Some(caller_idx) = resolve_canvas_node_by_path(&nodes, caller) {
                    add_synthetic(
                        caller_idx,
                        sel_idx,
                        "removed",
                        "caller at risk",
                        "calls",
                        Some("broken_risk"),
                    );
                }
            }
        }
        if commit_changes_active {
            for (from, to, kind) in &lookup.commit_edges_added {
                if let (Some(from_idx), Some(to_idx)) = (
                    resolve_canvas_node_by_path(&nodes, from),
                    resolve_canvas_node_by_path(&nodes, to),
                ) {
                    add_synthetic(from_idx, to_idx, "added", "added dependency", kind, None);
                }
            }
            for (from, to, kind) in &lookup.commit_edges_removed {
                if let (Some(from_idx), Some(to_idx)) = (
                    resolve_canvas_node_by_path(&nodes, from),
                    resolve_canvas_node_by_path(&nodes, to),
                ) {
                    add_synthetic(
                        from_idx,
                        to_idx,
                        "removed",
                        "removed dependency",
                        kind,
                        None,
                    );
                }
            }
            for (from, to, imp) in &lookup.commit_risk_callers {
                if imp == "broken_risk" && !to.is_empty() {
                    if let (Some(from_idx), Some(to_idx)) = (
                        resolve_canvas_node_by_path(&nodes, from),
                        resolve_canvas_node_by_path(&nodes, to),
                    ) {
                        add_synthetic(
                            from_idx,
                            to_idx,
                            "removed",
                            "caller at risk",
                            "calls",
                            Some("broken_risk"),
                        );
                    }
                }
            }
        }
    }
    if !is_dense && !marks.is_empty() {
        html.push_str(&marks);
    }
    // Over each link lies a wider one that cannot be seen and is easy to
    // hit: selecting a link looks into it. They lie over all the links, so
    // none of them is hidden under a link drawn later. A link between two
    // layers is many links of many files: selecting it lists the pairs of
    // files it is made of beside the picture.
    for ((wire, kinds), hidden) in plan
        .wires
        .iter()
        .zip(&wire_kinds)
        .zip(&hidden_wires)
        .filter(|_| !is_dense && view["mode"] != "modules")
    {
        let (from, to) = pairs[wire.edge];
        if let Some(cone) = overlay.isolate_cone {
            if !cone.contains(&from) || !cone.contains(&to) {
                continue;
            }
        }
        if scale_tier.elide_duplicate_hits() && (*hidden || (selected && !lit((from, to)))) {
            continue;
        }
        let edge = edges[wire.edge];
        let breakdown_note = kinds
            .breakdown
            .as_deref()
            .map(|text| format!(" · {}", escape(text)))
            .unwrap_or_default();
        html.push_str(&format!(
            "<path class=\"graph-map-hit{}\" data-ends=\"{from} {to}\"{} d=\"{}\" data-action=\"graph-map-link\" data-payload=\"{}\" role=\"button\"{}><title>{} uses {}{}{breakdown_note} · select to see {}</title></path>",
            if wire.portal && edge["architecture_violation"].as_u64().unwrap_or(0) == 0 { " graph-map-hit-portal" } else { "" },
            if *hidden { " data-kind-hidden=\"true\"" } else { "" },
            wire.path,
            payload(&json!({"from": nodes[from]["path"], "to": nodes[to]["path"]})),
            if *hidden { " aria-hidden=\"true\"" } else { " tabindex=\"0\" focusable=\"true\"" },
            escape(&m::text(&nodes[from], "name")),
            escape(&m::text(&nodes[to], "name")),
            if cycle_view && m::yes(&edge["added"]) { " · added in PR" } else { "" },
            if layered_view {
                "the pairs of files it is made of"
            } else {
                "what makes up this link"
            },
        ));
    }
    html.push_str("</svg>");
    if let Some(apart) = plan.apart {
        html.push_str(&format!(
            "<span class=\"graph-map-caption\" style=\"top:{apart:.0}px\">Not linked to anything here</span>"
        ));
    }
    let blast = blast_marks(view, &nodes);
    let dense_render_modes = DenseRenderModeFlags {
        layered_view,
        opened,
        followed,
        pathed,
        shared,
        cycle_view,
        pattern_view,
        scenario,
        cycles,
        named,
        pattern_rings: m::yes(&view["pattern"]["rings"]),
    };
    let mut dense_nodes = Vec::new();
    let mut dense_node_htmls = if is_dense {
        Vec::with_capacity(nodes.len())
    } else {
        Vec::new()
    };
    let mut dense_node_seeds = if is_dense {
        Vec::with_capacity(nodes.len())
    } else {
        Vec::new()
    };
    let mut dense_extra_specs = if is_dense {
        Vec::with_capacity(nodes.len())
    } else {
        Vec::new()
    };
    let active_roving = overlay
        .roving_focus
        .filter(|i| *i < nodes.len() && overlay.isolate_cone.is_none_or(|c| c.contains(i)))
        .or_else(|| chosen.filter(|i| overlay.isolate_cone.is_none_or(|c| c.contains(i))))
        .or_else(|| {
            overlay
                .isolate_cone
                .and_then(|c| c.iter().copied().find(|&i| i < nodes.len()))
        })
        .unwrap_or(0);
    for (index, node) in nodes.iter().enumerate() {
        let placed = &plan.nodes[index];
        if let Some(cone) = overlay.isolate_cone {
            if !cone.contains(&index) {
                if is_dense {
                    dense_node_htmls.push(String::new());
                    dense_node_seeds.push(None);
                    dense_extra_specs.push(None);
                }
                continue;
            }
        }
        let is_pinned = pinned_set.contains(&index);
        let mut skip_offscreen_dom = false;
        let is_on_traced = overlay.path_nodes.is_some_and(|pn| pn.contains(&index));
        let any_traced = overlay.path_nodes.is_some_and(|pn| !pn.is_empty());
        let lit_here = near(index);
        let dom_seed = DenseNodeDomSeed {
            index,
            placed: placed.clone(),
            flow: flow[index],
            blast: blast[index],
            change_status: node_change_statuses[index].clone(),
            impact: impact_lookup
                .as_ref()
                .and_then(|lookup| lookup.node_impact(&nodes, index)),
            chosen: chosen == Some(index),
            selected,
            lit: lit_here,
            pinned: is_pinned,
            roving: active_roving == index,
            focus_target: focus_overlay.nodes.contains(&index),
            on_traced_path: is_on_traced,
            any_traced_path: any_traced,
            shared_path_mark: !shared_edges.is_empty() && lit_here,
        };
        if is_dense {
            dense_nodes.push(graph_viewport::canvas2d::DenseNodeSpec {
                index,
                x: placed.x,
                y: placed.y,
                w: BOX_WIDTH,
                h: BOX_HEIGHT,
                label: m::text(node, "name"),
                role: resolved_member_kind(node).to_owned(),
                selected: dom_seed.chosen,
                lit: lit_here || is_on_traced,
                dim: (selected && !lit_here) || (any_traced && !is_on_traced),
                changed: node["changed"].as_u64().unwrap_or(0) > 0
                    || dom_seed.change_status.is_some(),
                pinned: is_pinned,
                failed: node["suspect"].as_u64().is_some(),
            });
            if let Some(bounds) = overlay.viewport_bounds {
                let is_important = dom_seed.chosen
                    || is_pinned
                    || index == active_roving
                    || node["suspect"].as_u64().is_some()
                    || is_on_traced;
                if !is_important
                    && !rect_intersects_viewport(
                        placed.x - 120.0,
                        placed.y - 120.0,
                        BOX_WIDTH + 240.0,
                        BOX_HEIGHT + 240.0,
                        bounds,
                    )
                {
                    skip_offscreen_dom = true;
                }
            }
        }
        let folder = m::yes(&node["folder"]);
        let symbol = m::yes(&node["symbol"]);
        let kind = resolved_member_kind(node);
        let node_vis = node["visibility"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let line = node["line"].as_u64().unwrap_or(0);
        let home = match node["file"].as_str() {
            Some(file) => file.to_owned(),
            None => m::text(view, "scope"),
        };
        let node_payload = if scenario {
            json!({
                "file": node["file"].as_str().or(node["path"].as_str()),
                "path": node["path"], "symbol": symbol
            })
        } else {
            node["path"].clone()
        };
        let node_action = if scenario {
            "graph-map-scenario-open"
        } else {
            "graph-map-node"
        };
        if is_dense {
            let cycle = placed.cycle && cycles;
            let circle = &node["circle"];
            let suspect = node["suspect"]["rank"].as_u64();
            let is_semantic = m::text(node, "node_kind") == "semantic"
                || m::text(node, "path").starts_with("sem:");
            let arch_color = layer_color(&node["architecture_layer"]);
            let arch_layer_id = if arch_color.is_some() {
                m::text(&node["architecture_layer"], "id")
            } else if layered_view {
                let path = m::text(node, "path");
                path.strip_prefix("layer:").unwrap_or(&path).to_owned()
            } else {
                String::new()
            };
            let base_title = match (symbol, line) {
                (true, 0) => format!("{} · {home}", m::text(node, "name")),
                (true, line) => format!("{} · {home}:{line}", m::text(node, "name")),
                _ if m::yes(&node["layer"]) || m::yes(&node["module"]) => m::text(node, "name"),
                _ => m::text(node, "path"),
            };
            let title = if let Some(vis) = node_vis {
                format!("{base_title} · {vis}")
            } else {
                base_title
            };
            let blast_tag = blast[index]
                .map(|(hop, _)| format!("{hop}h"))
                .unwrap_or_default();
            let badge_text = if is_semantic {
                "hub".to_owned()
            } else if let Some(rank) = suspect {
                format!("#{rank}")
            } else if !blast_tag.is_empty() {
                blast_tag.clone()
            } else if cycle {
                "cyc".to_owned()
            } else if let Some(vis) = node_vis {
                vis.to_owned()
            } else if folder {
                format!("{}f", node["files"].as_u64().unwrap_or(0))
            } else {
                String::new()
            };
            let file_line = match (symbol, line) {
                (true, 0) if !home.is_empty() => home.clone(),
                (true, l) if l > 0 && !home.is_empty() => format!("{home}:{l}"),
                _ => m::text(node, "path"),
            };
            if !arch_layer_id.is_empty() {
                dense_meta.layer_of_node.insert(index, arch_layer_id);
            }
            dense_meta.nodes.insert(
                index,
                graph_viewport::canvas2d::DenseNodeMeta {
                    explain_kind: "node".to_owned(),
                    explain_payload: node["path"].clone(),
                    symbol_kind: if is_semantic {
                        "contract".to_owned()
                    } else if symbol {
                        what(node)
                    } else {
                        kind.to_owned()
                    },
                    visibility: node_vis.unwrap_or("").to_owned(),
                    in_cycle: cycle || circle.is_object(),
                    blast_tag,
                    badge_text,
                    tooltip: title,
                    file_line,
                },
            );
            // Defer HTML string allocation until roving focus or viewport mount needs it.
            dense_node_htmls.push(String::new());
            dense_node_seeds.push(Some(dom_seed.clone()));
            let has_extras = followed
                || !far[index].0.is_empty()
                || !far[index].1.is_empty()
                || (folder && spreads(view));
            dense_extra_specs.push(has_extras.then(|| DenseNodeExtraSpec {
                index,
                x: placed.x,
                y: placed.y,
                followed,
                portal_out: far[index].0.clone(),
                portal_in: far[index].1.clone(),
                folder_spread: folder && spreads(view),
                dim: chosen.is_some() && !near(index),
            }));
        }
        if skip_offscreen_dom {
            continue;
        }
        let node_buf = render_dense_node_dom_html(view, node, &dom_seed, dense_render_modes, false);
        html.push_str(&node_buf);
        html.push_str(&node_extras_html_for_node(
            view,
            &nodes,
            node,
            index,
            placed.x,
            placed.y,
            followed,
            &far[index].0,
            &far[index].1,
            folder && spreads(view),
            chosen.is_some() && !near(index),
            node_action,
            &node_payload,
        ));
    }
    // The head of a frame: − draws the folder as one box again, then its
    // name, and how much of it is drawn where that is not all.
    for (held, place) in frames.iter().zip(&plan.frames) {
        if place.width <= 0.0 {
            continue;
        }
        let name = escape(&m::text(held, "name"));
        let (places, drawn) = (
            held["places"].as_u64().unwrap_or(0),
            held["drawn"].as_u64().unwrap_or(0),
        );
        // A frame of a pattern is no folder to draw as one box again.
        let fixed = m::yes(&held["fixed"]);
        if !fixed {
            html.push_str(&format!(
                "<button type=\"button\" class=\"graph-map-spread\" data-action=\"graph-map-gather\" data-payload=\"{}\" style=\"left:{:.0}px;top:{:.0}px\" title=\"Draw {name} as one box again\" aria-label=\"Draw {name} as one box again\">−</button>",
                payload(&held["path"]),
                place.x + 13.0,
                place.y + 11.0,
            ));
        }
        let role_attrs = pattern_family
            .as_deref()
            .map_or_else(String::new, |family| {
                let role_frame = if held["path"] == "role:0" {
                    "anchor"
                } else {
                    "part"
                };
                format!(
                    " data-pattern-family=\"{}\" data-role-frame=\"{role_frame}\"",
                    escape(family)
                )
            });
        html.push_str(&format!(
            "<span class=\"graph-map-frame-name\"{role_attrs} style=\"left:{:.0}px;top:{:.0}px;max-width:{:.0}px\" title=\"{}\">{name}{}</span>",
            place.x + if fixed { 8.0 } else { 26.0 },
            place.y + 3.0,
            (place.width - if fixed { 14.0 } else { 32.0 }).max(40.0),
            escape(&if fixed { m::text(held, "name") } else { m::text(held, "path") }),
            if drawn < places {
                format!(
                    "<span class=\"graph-map-frame-count\"> · {} of {} drawn</span>",
                    count(drawn),
                    count(places)
                )
            } else {
                String::new()
            },
        ));
    }
    for placed_entity in &focus_overlay.entities {
        let entity = &placed_entity.entity;
        let kind_short = entity_kind_badge(&entity.kind);
        let (change_key, badge_label, title_prefix) = entity_change_badge(entity.change.as_deref());
        let loc = if !entity.path.is_empty() && entity.line > 0 {
            format!(" · {}:{}", entity.path, entity.line)
        } else if !entity.path.is_empty() {
            format!(" · {}", entity.path)
        } else {
            String::new()
        };
        let vis_attr = entity
            .visibility
            .as_deref()
            .and_then(|v| normalize_visibility(Some(v)))
            .map(|(bucket, _, _)| format!(" data-visibility=\"{bucket}\""))
            .unwrap_or_default();
        let vis_pill = visibility_pill_html(entity.visibility.as_deref(), true);
        let vis_word = entity
            .visibility
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| format!("{s} "))
            .unwrap_or_default();
        let title = format!(
            "{title_prefix} {vis_word}{} {}{loc}",
            entity.kind, entity.name
        );
        let entity_payload = payload(&json!({
            "id": entity.id,
            "kind": entity.kind,
            "name": entity.name,
            "parent": entity.parent,
            "path": entity.path,
            "folder": entity.folder,
            "line": entity.line,
            "change": change_key,
            "visibility": entity.visibility,
            "modifiers": entity.modifiers,
            "x": placed_entity.x,
            "y": placed_entity.y,
        }));
        let entity_action = if id_prefix.starts_with("system-") {
            format!("{id_prefix}-entity")
        } else {
            "session-graph-entity".to_owned()
        };
        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-map-node graph-map-entity-node is-blinking\" data-entity-id=\"{}\" data-entity-kind=\"{}\" data-entity-change=\"{change_key}\"{vis_attr} data-action=\"{entity_action}\" data-payload=\"{entity_payload}\" style=\"left:{:.0}px;top:{:.0}px\" title=\"{}\" aria-label=\"{}\"><span class=\"graph-map-entity-dot\" aria-hidden=\"true\"></span><span class=\"graph-map-entity-kind\" aria-hidden=\"true\">{kind_short}</span>{vis_pill}<span class=\"graph-map-entity-name\">{}</span><span class=\"graph-map-entity-badge\">{badge_label}</span></button>",
            escape(&entity.id),
            escape(&entity.kind),
            placed_entity.x,
            placed_entity.y,
            escape(&title),
            escape(&title),
            escape(&entity.name),
        ));
    }
    html.push_str("</div>");
    let selected_idx = chosen.or_else(|| {
        impact_lookup
            .as_ref()
            .and_then(|lookup| lookup.selected_idx)
    });
    let selected_bbox = compute_selected_bbox(
        &nodes,
        &plan.nodes,
        &pairs,
        selected_idx,
        impact_lookup.as_ref(),
    );
    // In small a box tells one thing: that it is selected, else that it
    // changed, else that it closes a cycle.
    let boxes = nodes
        .iter()
        .zip(&plan.nodes)
        .enumerate()
        .map(|(index, (node, placed))| {
            let state = radar_box_state(
                selected_idx,
                index,
                node,
                placed.cycle,
                cycles,
                node_change_statuses[index].is_some(),
            );
            (placed.x, placed.y, state)
        })
        .collect::<Vec<_>>();
    let radar = radar((plane_w, plane_h), &plan.frames, &boxes);
    let dense_scene = is_dense.then_some(graph_viewport::canvas2d::DenseScene {
        width: plane_w,
        height: plane_h,
        nodes: dense_nodes,
        edges: dense_edges,
    });
    RenderedCanvas {
        html,
        width: plane_w,
        height: plane_h,
        focus: focus_overlay,
        selected_bbox,
        radar,
        plan_nodes: plan.nodes,
        plan_frames: plan.frames,
        dense_scene,
        dense_meta: is_dense.then_some(dense_meta),
        dense_node_htmls,
        dense_node_seeds,
        dense_render_modes,
        dense_extra_specs,
        node_keys,
        pairs,
        node_labels,
    }
}

/// Formats one interactive `<button class="graph-map-node">` HTML element for either
/// initial mount or lazy on-demand mounting during pan / roving keyboard focus.
pub(in crate::graph_map) fn render_dense_node_dom_html(
    view: &V,
    node: &V,
    seed: &DenseNodeDomSeed,
    modes: DenseRenderModeFlags,
    force_tabindex_zero: bool,
) -> String {
    let DenseRenderModeFlags {
        layered_view,
        opened,
        followed,
        pathed,
        shared,
        cycle_view,
        pattern_view,
        scenario,
        cycles,
        named,
        pattern_rings,
    } = modes;
    let index = seed.index;
    let placed = &seed.placed;
    let folder = m::yes(&node["folder"]);
    let symbol = m::yes(&node["symbol"]);
    let line = node["line"].as_u64().unwrap_or(0);
    let (uses, used) = (
        node["out"].as_u64().unwrap_or(0),
        node["in"].as_u64().unwrap_or(0),
    );
    let mut label = format!(
        "{}, {}",
        m::text(node, "name"),
        if symbol {
            match line {
                0 => what(node),
                line => format!("{}, line {line}", what(node)),
            }
        } else if m::yes(&node["module"]) {
            format!(
                "module, {}",
                plural(
                    node["within"].as_u64().unwrap_or(0),
                    "internal link",
                    "internal links"
                )
            )
        } else if folder {
            format!(
                "{}, {}",
                if m::yes(&node["layer"]) {
                    "layer"
                } else {
                    "folder"
                },
                plural(node["files"].as_u64().unwrap_or(0), "file", "files")
            )
        } else {
            "file".to_owned()
        }
    );
    label.push_str(&format!(
        ", uses others through {}, is used through {}",
        plural(uses, "link", "links"),
        plural(used, "link", "links")
    ));
    let cycle = placed.cycle && cycles;
    let circle = &node["circle"];
    if let Some(told) = circle_told(circle, folder) {
        label.push_str(&told);
    } else if cycle {
        label.push_str(", part of a cycle");
    }
    let circle_attr = match (circle.is_object(), m::yes(&circle["leaves"])) {
        (true, true) => " data-circle=\"through\"",
        (true, false) => " data-circle=\"inside\"",
        _ => "",
    };
    let suspect = node["suspect"]["rank"].as_u64();
    if let Some(rank) = suspect {
        label.push_str(&match node["suspect"]["files"].as_u64() {
            Some(files) => format!(
                ", holds {} the failing tests reach, the first ranked {rank}",
                plural(files, "changed file", "changed files")
            ),
            None => format!(", ranked {rank} among the changes by the failing tests"),
        });
    }
    let blast_attr = match seed.blast {
        Some((hop, left)) => {
            label.push_str(&format!(
                ", a change to the selected place reaches it in {}, {} % of it",
                plural(hop, "link", "links"),
                (left * 100.0).round() as u64
            ));
            format!(" data-blast=\"{hop}\"")
        }
        None => String::new(),
    };
    let detour_attr = if m::yes(&node["detour"]) {
        label.push_str(", on a longer way only");
        " data-detour=\"true\""
    } else {
        ""
    };
    let path_mark = if seed.shared_path_mark {
        " data-shared-path=\"true\""
    } else {
        ""
    };
    let run_opened = match (&node["opened"], node["opened"].as_u64()) {
        (_, Some(files)) => {
            label.push_str(&format!(
                ", the run opened {} in it",
                plural(files, "file", "files")
            ));
            Some(count(files))
        }
        (seen, None) if m::yes(seen) => {
            label.push_str(", opened by the run");
            Some(String::new())
        }
        _ => None,
    };
    let run_mark = if run_opened.is_some() {
        " data-run-opened=\"true\""
    } else {
        ""
    };
    let pressed = if shared || pattern_view {
        format!(" aria-pressed=\"{}\"", seed.chosen)
    } else {
        String::new()
    };
    let (start, end) = (
        m::yes(&node["start"]),
        (pathed || shared) && m::yes(&node["end"]),
    );
    if start {
        label.push_str(if shared {
            ", the first of the two places"
        } else if pathed {
            ", the path starts here"
        } else if cycle_view {
            ", the cycle starts and ends here"
        } else if pattern_view {
            ", the pattern is named after it"
        } else if scenario {
            ", scenario starting point"
        } else {
            ", the links are followed from here"
        });
    }
    if end {
        label.push_str(if shared {
            ", the second of the two places"
        } else {
            ", the path ends here"
        });
    }
    let both = shared && node["shares"].is_array();
    if both {
        label.push_str(if above(view) {
            ", uses both"
        } else {
            ", both use it"
        });
    }
    let changed = node["changed"].as_u64().unwrap_or(0);
    let mark = if opened || symbol {
        let mut mark = match node["role"].as_str() {
            Some("uses") => ", uses this file",
            Some("used") => ", used by this file",
            Some("both") => ", uses this file and is used by it",
            _ => "",
        }
        .to_owned();
        if let Some(status) = node["status"].as_str() {
            mark.push_str(&format!(", {}", happened(status)));
        }
        match node["touched"].as_u64() {
            Some(inside) if symbol && inside > 0 => {
                mark.push_str(&format!(", {} changed inside", count(inside)));
            }
            _ => {}
        }
        mark
    } else {
        match (changed, node["status"].as_str(), node["role"].as_str()) {
            (0, _, Some("affected")) => ", uses the change".to_owned(),
            (0, _, Some("needed")) => ", used by the change".to_owned(),
            (0, _, Some("both")) => ", uses the change and is used by it".to_owned(),
            (0, ..) => String::new(),
            (_, Some(status), _) => format!(", {}", happened(status)),
            _ => format!(
                ", {} inside",
                plural(changed, "changed file", "changed files")
            ),
        }
    };
    label.push_str(&mark);
    if let Some(role) = node["role"].as_str().filter(|_| pattern_view) {
        label.push_str(&format!(", {role} in the pattern"));
    }
    let patterns = node["patterns"].as_u64().unwrap_or(0);
    if patterns > 0 {
        label.push_str(&format!(
            ", {} {}",
            if symbol { "part of" } else { "holds parts of" },
            plural(patterns, "found pattern", "found patterns")
        ));
    }
    if node["shell_read"].is_number() {
        label.push_str(", read via shell");
    }
    let kind = resolved_member_kind(node);
    let opens = !scenario
        && !followed
        && !pathed
        && !shared
        && !cycle_view
        && !pattern_view
        && (folder || (!symbol && node["symbols"].as_u64().unwrap_or(0) > 0));
    let home = match node["file"].as_str() {
        Some(file) => file.to_owned(),
        None => m::text(view, "scope"),
    };
    let (node_action, node_payload) = if scenario {
        (
            "graph-map-scenario-open",
            json!({
                "file": node["file"].as_str().or(node["path"].as_str()),
                "path": node["path"], "symbol": symbol
            }),
        )
    } else {
        ("graph-map-node", node["path"].clone())
    };
    let node_vis = node["visibility"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(vis) = node_vis {
        label.push_str(&format!(", visibility {vis}"));
    }
    let title = match (symbol, line) {
        (true, 0) => format!("{} · {home}", m::text(node, "name")),
        (true, line) => format!("{} · {home}:{line}", m::text(node, "name")),
        _ if m::yes(&node["layer"]) || m::yes(&node["module"]) => m::text(node, "name"),
        _ => m::text(node, "path"),
    };
    let title = if let Some(vis) = node_vis {
        format!("{title} · {vis}")
    } else {
        title
    };
    let node_vis_attr = node_vis
        .and_then(|v| normalize_visibility(Some(v)))
        .map(|(bucket, _, _)| format!(" data-visibility=\"{bucket}\""))
        .unwrap_or_default();
    let node_vis_pill = visibility_pill_html(node_vis, true);
    let arch_color = layer_color(&node["architecture_layer"]);
    let arch_layer_id = if arch_color.is_some() {
        m::text(&node["architecture_layer"], "id")
    } else if layered_view {
        let path = m::text(node, "path");
        path.strip_prefix("layer:").unwrap_or(&path).to_owned()
    } else {
        String::new()
    };
    let layer_color_value = arch_color
        .clone()
        .or_else(|| layered_view.then(|| layer_accent(index as u64)));
    let architecture = match (arch_color.is_some(), layered_view, &layer_color_value) {
        (true, _, Some(color)) => {
            label.push_str(&format!(
                ", layer {}",
                m::text(&node["architecture_layer"], "name")
            ));
            format!(
                " data-architecture-layer=\"true\" data-arch-layer-id=\"{}\" style=\"--architecture-color:{color};left:{:.0}px;top:{:.0}px\"",
                escape(&arch_layer_id),
                placed.x,
                placed.y
            )
        }
        (false, true, Some(color)) => format!(
            " data-layer=\"true\" data-arch-layer-id=\"{}\" data-layer-order=\"{}\" style=\"--architecture-color:{color};left:{:.0}px;top:{:.0}px\"",
            escape(&arch_layer_id),
            index + 1,
            placed.x,
            placed.y
        ),
        _ => String::new(),
    };
    let position = if architecture.is_empty() {
        format!(" style=\"left:{:.0}px;top:{:.0}px\"", placed.x, placed.y)
    } else {
        architecture
    };
    let position = match seed.blast {
        Some((_, left)) => {
            position.replacen(" style=\"", &format!(" style=\"--blast:{left:.2};"), 1)
        }
        None => position,
    };
    let pattern_family = pattern_view
        .then(|| m::text(&view["pattern"], "family"))
        .filter(|family| !family.is_empty());
    let mut pattern_node_attrs = String::new();
    if patterns > 0 {
        pattern_node_attrs.push_str(" data-has-patterns=\"true\"");
        if let Some(found_family) = pattern_family_of(view, &m::text(node, "path")) {
            pattern_node_attrs.push_str(&format!(" data-pattern-family=\"{found_family}\""));
        }
    } else if !layered_view {
        if let Some(family) = &pattern_family {
            pattern_node_attrs.push_str(&format!(" data-pattern-family=\"{}\"", escape(family)));
            if let Some(role) = node["role"].as_str() {
                pattern_node_attrs.push_str(&format!(" data-pattern-role=\"{}\"", escape(role)));
            }
        }
    }
    let prefix_badge = if layered_view {
        let badge = if pattern_rings && index == 0 {
            "◎".to_owned()
        } else {
            format!("{}", index + 1)
        };
        format!("<span class=\"graph-map-layer-order\" aria-hidden=\"true\">{badge}</span>")
    } else if arch_color.is_some() {
        "<span class=\"graph-map-node-layer\" aria-hidden=\"true\"></span>".to_owned()
    } else {
        String::new()
    };
    let node_change_status = seed.change_status.as_deref();
    let node_change_attr = node_change_status
        .map(|status| format!(" data-change-status=\"{}\"", escape(status)))
        .unwrap_or_default();
    let node_impact_attr = seed
        .impact
        .map(|impact| format!(" data-impact=\"{impact}\""))
        .unwrap_or_default();
    let (out, into) = seed.flow;
    let focus_target_attr = if seed.focus_target {
        " data-focus-target=\"true\""
    } else {
        ""
    };
    let node_path_attr = {
        let p = m::text(node, "path");
        if p.is_empty() {
            String::new()
        } else {
            format!(" data-path=\"{}\"", escape(&p))
        }
    };
    let place = if m::yes(&node["layer"]) || m::yes(&node["module"]) {
        String::new()
    } else if symbol {
        let file = home.rsplit('/').next().unwrap_or(&home);
        match line {
            0 => file.to_owned(),
            line => format!("{file}:{line}"),
        }
    } else if folder {
        plural(node["files"].as_u64().unwrap_or(0), "file", "files")
    } else {
        m::text(node, "path")
            .rsplit_once('/')
            .map(|(folder, _)| folder.to_owned())
            .unwrap_or_default()
    };
    let detail = [
        if uses + used > 0 {
            format!("{}→ →{}", count(used), count(uses))
        } else {
            String::new()
        },
        place,
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" · ");
    let mut secondary_badges: Vec<String> = Vec::new();
    if !node_vis_pill.is_empty() {
        secondary_badges.push(node_vis_pill);
    }
    if pattern_view && !layered_view && start {
        secondary_badges
            .push("<span class=\"graph-map-node-anchor\" aria-hidden=\"true\">★</span>".to_owned());
    }
    if cycle || circle.is_object() {
        let held = circle["files"]
            .as_u64()
            .filter(|_| folder)
            .map(count)
            .unwrap_or_default();
        secondary_badges.push(format!(
            "<span class=\"graph-map-node-cycle\" aria-hidden=\"true\">↻{held}</span>"
        ));
    }
    if patterns > 0 {
        secondary_badges.push(format!(
            "<span class=\"graph-map-node-pattern\" aria-hidden=\"true\">◆{}</span>",
            if patterns > 1 {
                count(patterns)
            } else {
                String::new()
            }
        ));
    }
    if let Some(files) = &run_opened {
        secondary_badges.push(format!(
            "<span class=\"graph-map-node-opened\" aria-hidden=\"true\">◉{files}</span>"
        ));
    }
    if let Some(rank) = suspect {
        secondary_badges.push(format!(
            "<span class=\"graph-map-node-suspect\" aria-hidden=\"true\">#{rank}</span>"
        ));
    }
    let (badge_attrs, badges_strip) = build_node_badges_strip(&secondary_badges);
    let pin_attr = if seed.pinned {
        " data-pinned=\"true\""
    } else {
        ""
    };
    let roving_tabindex = if force_tabindex_zero || seed.roving {
        "0"
    } else {
        "-1"
    };
    let dim_node = (seed.selected && !seed.lit) || (seed.any_traced_path && !seed.on_traced_path);
    let is_semantic =
        m::text(node, "node_kind") == "semantic" || m::text(node, "path").starts_with("sem:");
    let semantic_attr = if is_semantic {
        " data-semantic=\"true\""
    } else {
        ""
    };
    let mut node_buf = format!(
        "<button type=\"button\" class=\"graph-map-node\" data-n=\"{index}\" data-action=\"{node_action}\" data-payload=\"{}\" data-folder=\"{folder}\" data-kind=\"{}\" data-open=\"{opens}\" data-outside=\"{}\" data-changed=\"{}\" data-selected=\"{}\" data-dim=\"{}\" data-cycle=\"{}\" data-flow=\"{}\" data-out=\"{}\" data-in=\"{}\" data-start=\"{start}\"{semantic_attr}{}{}{}{}{path_mark}{run_mark}{focus_target_attr}{circle_attr}{detour_attr}{blast_attr}{node_change_attr}{node_impact_attr}{node_vis_attr}{pattern_node_attrs}{badge_attrs}{pin_attr}{position}{node_path_attr} tabindex=\"{roving_tabindex}\" data-roving-index=\"{index}\" title=\"{}\"{pressed} aria-label=\"{}\">{prefix_badge}{}<span class=\"graph-map-node-name\">{}</span>",
        payload(&node_payload),
        escape(if is_semantic && kind.is_empty() {
            "semantic"
        } else {
            kind
        }),
        opened && !symbol,
        changed > 0,
        seed.chosen,
        dim_node,
        cycle,
        leaning(out, into),
        out > 0,
        into > 0,
        if seed.lit || seed.on_traced_path {
            " data-lit=\"true\" data-select-lit=\"true\""
        } else {
            ""
        },
        if end { " data-end=\"true\"" } else { "" },
        if both { " data-shared=\"true\"" } else { "" },
        if seed.on_traced_path {
            " data-shared-path=\"true\""
        } else {
            ""
        },
        escape(&title),
        escape(&label),
        match letter(kind) {
            "" => String::new(),
            letter => format!(
                "<span class=\"graph-map-node-kind\" aria-hidden=\"true\">{letter}</span>"
            ),
        },
        if named {
            format!("<bdi>{}</bdi>", escape(&m::text(node, "name")))
        } else {
            escape(&m::text(node, "name"))
        },
    );
    if changed > 0 {
        let pill_kind = node_change_status
            .filter(|s| matches!(*s, "created" | "modified" | "deleted" | "renamed"))
            .map(str::to_owned)
            .or_else(|| {
                node["status"]
                    .as_str()
                    .map(normalize_change_status)
                    .filter(|s| {
                        matches!(s.as_str(), "created" | "modified" | "deleted" | "renamed")
                    })
            })
            .unwrap_or_else(|| "modified".to_owned());
        let pill_text = match node["status"].as_str() {
            Some(status) => escape(status),
            None if symbol => count(node["touched"].as_u64().unwrap_or(0)),
            None if node_change_status.is_some() && changed == 1 && !folder => {
                change_status_letter(node_change_status.unwrap_or("")).to_owned()
            }
            None => count(changed),
        };
        node_buf.push_str(&format!(
            "<span class=\"graph-map-node-status\" data-status=\"{}\" aria-hidden=\"true\">{pill_text}</span>",
            escape(&pill_kind),
        ));
    } else if let Some(status) = node_change_status {
        let (badge, _) = change_status_badge(status);
        if !badge.is_empty() {
            node_buf.push_str(&format!(
                "<span class=\"graph-map-node-status\" data-status=\"{}\" aria-hidden=\"true\">{badge}</span>",
                escape(status)
            ));
        }
    } else if folder {
        node_buf.push_str(&format!(
            "<span class=\"graph-map-node-count\" aria-hidden=\"true\">{}</span>",
            count(node["files"].as_u64().unwrap_or(0))
        ));
    }
    if !detail.is_empty() {
        node_buf.push_str(&format!(
            "<span class=\"graph-map-node-detail\" aria-hidden=\"true\">{}</span>",
            escape(&detail)
        ));
    }
    if !blast_attr.is_empty() {
        node_buf.push_str("<span class=\"graph-map-node-blast\" aria-hidden=\"true\"></span>");
    }
    node_buf.push_str(&badges_strip);
    node_buf.push_str("</button>");
    node_buf
}

#[allow(clippy::too_many_arguments)]
pub(super) fn node_extras_html_for_node(
    _view: &V,
    nodes: &[V],
    node: &V,
    index: usize,
    x: f64,
    y: f64,
    followed: bool,
    portal_out: &[usize],
    portal_in: &[usize],
    folder_spread: bool,
    dim: bool,
    node_action: &str,
    node_payload: &V,
) -> String {
    let mut out = String::new();
    if followed {
        out.push_str(&handles(node, index, x, y, dim));
    }
    for (side, others, sign) in [("out", portal_out, "⇢"), ("in", portal_in, "⇠")] {
        if others.is_empty() {
            continue;
        }
        let names = others
            .iter()
            .take(6)
            .filter_map(|other| nodes.get(*other).map(|n| m::text(n, "name")))
            .collect::<Vec<_>>()
            .join(", ");
        let told = format!(
            "{} {} {}{} — drawn while this is rested on or the box is selected",
            m::text(node, "name"),
            if side == "out" {
                "uses, far from here:"
            } else {
                "is used from far away by:"
            },
            names,
            if others.len() > 6 { ", …" } else { "" },
        );
        out.push_str(&format!(
            "<button type=\"button\" class=\"graph-map-portal\" data-of=\"{index}\" data-side=\"{side}\" data-dim=\"{dim}\" data-action=\"{node_action}\" data-payload=\"{}\" style=\"left:{:.0}px;top:{:.0}px\" title=\"{}\" aria-label=\"{}\">{sign}{}</button>",
            payload(node_payload),
            if side == "out" { x + BOX_WIDTH } else { x },
            y + BOX_HEIGHT,
            escape(&told),
            escape(&told),
            count(others.len() as u64),
        ));
    }
    if folder_spread {
        let told = format!(
            "Open {} here, among the others",
            escape(&m::text(node, "name"))
        );
        out.push_str(&format!(
            "<button type=\"button\" class=\"graph-map-spread\" data-of=\"{index}\" data-dim=\"{dim}\" data-action=\"graph-map-spread\" data-payload=\"{}\" style=\"left:{x:.0}px;top:{y:.0}px\" title=\"{told}\" aria-label=\"{told}\">+</button>",
            payload(&node["path"]),
        ));
    }
    out
}

pub(super) fn render_dense_node_extras_for_mounted(
    view: &V,
    spec: &DenseNodeExtraSpec,
    dim_override: Option<bool>,
) -> String {
    let nodes = m::array(&view["nodes"]);
    let Some(node) = nodes.get(spec.index) else {
        return String::new();
    };
    let folder = m::yes(&node["folder"]);
    let symbol = m::yes(&node["symbol"]);
    let (node_action, node_payload) = if folder {
        ("graph-map-open", node["path"].clone())
    } else if view["mode"] == "scenario" {
        (
            "graph-map-scenario-open",
            json!({
                "file": node["file"].as_str().or(node["path"].as_str()),
                "path": node["path"],
                "symbol": symbol,
            }),
        )
    } else {
        ("graph-map-node", node["path"].clone())
    };
    node_extras_html_for_node(
        view,
        &nodes,
        node,
        spec.index,
        spec.x,
        spec.y,
        spec.followed,
        &spec.portal_out,
        &spec.portal_in,
        spec.folder_spread,
        dim_override.unwrap_or(spec.dim),
        node_action,
        &node_payload,
    )
}
/// How far past a corner the second handle of a side hangs.
pub(super) const HANDLE_GAP: f64 = 11.0;
