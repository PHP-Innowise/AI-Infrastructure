use super::*;

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
    pub node_keys: Vec<String>,
    pub pairs: Vec<(usize, usize)>,
    pub node_labels: Vec<String>,
}
/// Scale thresholds beyond which non-essential marks and offscreen details are simplified.
pub const LARGE_GRAPH_NODE_THRESHOLD: usize = 80;
pub const LARGE_GRAPH_EDGE_THRESHOLD: usize = 160;
pub const DENSE_GRAPH_NODE_THRESHOLD: usize = 180;
pub const DENSE_GRAPH_EDGE_THRESHOLD: usize = 400;

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

pub(super) fn hash_json_value<H: std::hash::Hasher>(val: &V, state: &mut H) {
    use std::hash::Hash;
    match val {
        V::Null => 0u8.hash(state),
        V::Bool(b) => {
            1u8.hash(state);
            b.hash(state);
        }
        V::Number(n) => {
            2u8.hash(state);
            n.to_string().hash(state);
        }
        V::String(s) => {
            3u8.hash(state);
            s.hash(state);
        }
        V::Array(items) => {
            4u8.hash(state);
            items.len().hash(state);
            for item in items {
                hash_json_value(item, state);
            }
        }
        V::Object(map) => {
            5u8.hash(state);
            map.len().hash(state);
            for (k, v) in map {
                k.hash(state);
                hash_json_value(v, state);
            }
        }
    }
}

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

pub(super) struct SelectionContext {
    pub(super) chosen: Option<usize>,
    pub(super) chosen_link: Option<(usize, usize)>,
    pub(super) selected: bool,
    pub(super) cycle_view: bool,
    pub(super) cycles: bool,
    pub(super) shared: bool,
    pub(super) pattern_view: bool,
    pub(super) shared_edges: BTreeSet<(usize, usize)>,
    pub(super) depth: Option<Vec<Option<i32>>>,
}

impl SelectionContext {
    pub(super) fn from_view(
        view: &V,
        nodes: &[V],
        pairs: &[(usize, usize)],
        cone_reach: (usize, usize),
    ) -> Self {
        let at = |path: &V| nodes.iter().position(|node| &node["path"] == path);
        let focus = view["focus"]["path"].as_str();
        let chosen = focus.and_then(|path| at(&json!(path)));
        let chosen_link = Some(&view["focus"])
            .filter(|focus| m::yes(&focus["link"]))
            .and_then(|focus| Some((at(&focus["from"]["path"])?, at(&focus["to"]["path"])?)));
        let selected = chosen.is_some() || chosen_link.is_some();
        let plain = plain_map(view);
        let opened = symbols(view);
        let shared = shares(view);
        let cycle_view = cycles(view);
        let pattern_view = patterned(view);
        let cycles = !changes(view) && !opened;
        let shared_edges: BTreeSet<_> = view["focus"]["shared_edges"]
            .as_array()
            .filter(|_| shared && chosen.is_some())
            .into_iter()
            .flatten()
            .filter_map(|edge| Some((at(&edge[0])?, at(&edge[1])?)))
            .collect();
        let reach = (cone_reach.0.max(1), cone_reach.1.max(1));
        let depth = chosen
            .filter(|_| plain && shared_edges.is_empty() && reach != (1, 1))
            .map(|chosen| cone(nodes.len(), pairs, chosen, reach.0, reach.1));
        Self {
            chosen,
            chosen_link,
            selected,
            cycle_view,
            cycles,
            shared,
            pattern_view,
            shared_edges,
            depth,
        }
    }

    pub(super) fn lit(&self, pair: (usize, usize)) -> bool {
        self.cycle_view
            || self.chosen_link == Some(pair)
            || match (&self.depth, self.chosen) {
                (Some(depth), _) => in_cone(depth, pair),
                (None, Some(chosen)) if self.shared_edges.is_empty() => {
                    pair.0 == chosen || pair.1 == chosen
                }
                _ => self.shared_edges.contains(&pair),
            }
    }

    pub(super) fn near(&self, pairs: &[(usize, usize)], node: usize) -> bool {
        self.chosen == Some(node)
            || pairs
                .iter()
                .any(|&(from, to)| (from == node || to == node) && self.lit((from, to)))
    }
}

pub(super) fn compute_selected_bbox(
    nodes: &[V],
    plan_nodes: &[Placed],
    pairs: &[(usize, usize)],
    selected_idx: Option<usize>,
    impact_lookup: Option<&ImpactLookup>,
) -> Option<(f64, f64, f64, f64)> {
    selected_idx
        .and_then(|sel| plan_nodes.get(sel).map(|p| (sel, p)))
        .map(|(sel, p)| {
            let mut min_x = p.x;
            let mut min_y = p.y;
            let mut max_x = p.x + BOX_WIDTH;
            let mut max_y = p.y + BOX_HEIGHT;
            let mut include_node = |n_idx: usize| {
                if let Some(np) = plan_nodes.get(n_idx) {
                    min_x = min_x.min(np.x);
                    min_y = min_y.min(np.y);
                    max_x = max_x.max(np.x + BOX_WIDTH);
                    max_y = max_y.max(np.y + BOX_HEIGHT);
                }
            };
            for (from, to) in pairs {
                if *from == sel {
                    include_node(*to);
                } else if *to == sel {
                    include_node(*from);
                }
            }
            if let Some(lookup) = impact_lookup {
                for target in lookup.uses_added.iter().chain(&lookup.uses_removed) {
                    if let Some(target_idx) = resolve_canvas_node_by_path(nodes, target) {
                        include_node(target_idx);
                    }
                }
                for caller in lookup.callers_added.iter().chain(&lookup.callers_risk) {
                    if let Some(caller_idx) = resolve_canvas_node_by_path(nodes, caller) {
                        include_node(caller_idx);
                    }
                }
            }
            (min_x, min_y, max_x, max_y)
        })
}

pub(super) fn radar_box_state(
    selected_idx: Option<usize>,
    index: usize,
    node: &V,
    placed_cycle: bool,
    cycles: bool,
    has_change_status: bool,
) -> &'static str {
    if selected_idx == Some(index) {
        "selected"
    } else if node["changed"].as_u64().unwrap_or(0) > 0
        || node["status"].is_string()
        || has_change_status
    {
        "changed"
    } else if placed_cycle && cycles {
        "cycle"
    } else {
        ""
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct NodeSelectionState {
    pub(super) selected: bool,
    pub(super) dim: bool,
    pub(super) shared_path: bool,
    pub(super) focus_target: bool,
    pub(super) pressed: Option<bool>,
    pub(super) blast: Option<(u64, f64)>,
    pub(super) radar_state: &'static str,
    pub(super) portal_dim: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct EdgeSelectionState {
    pub(super) edge_state: &'static str,
    pub(super) shared_path: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct CanvasSelectionPatch {
    pub(super) nodes: Vec<NodeSelectionState>,
    pub(super) edges_by_pair: HashMap<(usize, usize), EdgeSelectionState>,
    pub(super) frame_targets: Vec<bool>,
    pub(super) focus: FocusOverlay,
    pub(super) selected_bbox: Option<(f64, f64, f64, f64)>,
}

/// Computes pure selection/highlight/focus-target/blast attribute updates without rebuilding canvas HTML.
pub(super) fn compute_canvas_selection_patch(
    view: &V,
    plan_nodes: &[Placed],
    plan_frames: &[Framed],
    pairs: &[(usize, usize)],
    overlay: &CanvasOverlayContext<'_>,
) -> CanvasSelectionPatch {
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
    let changes_lookup = overlay.changes.and_then(ChangesLookup::from_value);
    let impact_lookup = ImpactLookup::from_sources(overlay.impact, overlay.changes, &nodes);
    let sel = SelectionContext::from_view(view, &nodes, pairs, overlay.cone);
    let node_change_statuses: Vec<Option<String>> = nodes
        .iter()
        .map(|node| {
            changes_lookup
                .as_ref()
                .and_then(|lookup| lookup.status_for_node(node, &nodes))
        })
        .collect();
    let plan = Plan {
        nodes: plan_nodes.to_vec(),
        wires: Vec::new(),
        frames: plan_frames.to_vec(),
        width: 0.0,
        height: 0.0,
        apart: None,
    };
    let focus_overlay = overlay
        .live_focus
        .map(|focus| resolve_focus(view, &plan, focus))
        .unwrap_or_default();
    let blast = blast_marks(view, &nodes);
    let selected_idx = sel.chosen.or_else(|| {
        impact_lookup
            .as_ref()
            .and_then(|lookup| lookup.selected_idx)
    });
    let selected_bbox = compute_selected_bbox(
        &nodes,
        plan_nodes,
        pairs,
        selected_idx,
        impact_lookup.as_ref(),
    );

    let node_patches = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let is_lit = sel.near(pairs, index);
            let placed_cycle = plan_nodes.get(index).is_some_and(|p| p.cycle);
            NodeSelectionState {
                selected: sel.chosen == Some(index),
                dim: sel.selected && !is_lit,
                shared_path: !sel.shared_edges.is_empty() && is_lit,
                focus_target: focus_overlay.nodes.contains(&index),
                pressed: (sel.shared || sel.pattern_view).then_some(sel.chosen == Some(index)),
                blast: blast[index],
                radar_state: radar_box_state(
                    selected_idx,
                    index,
                    node,
                    placed_cycle,
                    sel.cycles,
                    node_change_statuses[index].is_some(),
                ),
                portal_dim: sel.chosen.is_some() && !is_lit,
            }
        })
        .collect();

    let mut edges_by_pair = HashMap::with_capacity(pairs.len());
    for &(from, to) in pairs {
        let edge_state = if sel.cycle_view || sel.selected {
            if sel.lit((from, to)) {
                "on"
            } else {
                "off"
            }
        } else {
            ""
        };
        edges_by_pair.insert(
            (from, to),
            EdgeSelectionState {
                edge_state,
                shared_path: sel.shared_edges.contains(&(from, to)),
            },
        );
    }

    let frame_targets = (0..frames.len())
        .map(|index| focus_overlay.frames.contains(&index))
        .collect();

    CanvasSelectionPatch {
        nodes: node_patches,
        edges_by_pair,
        frame_targets,
        focus: focus_overlay,
        selected_bbox,
    }
}
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

/// Renders a dependency map plane with live focus, changed-files markers, and file-impact diff edges.
pub fn canvas_with_overlays(
    view: &V,
    id_prefix: &str,
    overlay: CanvasOverlayContext<'_>,
) -> RenderedCanvas {
    let live_focus = overlay.live_focus;
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
    let changes_lookup = overlay.changes.and_then(ChangesLookup::from_value);
    let impact_lookup = ImpactLookup::from_sources(overlay.impact, overlay.changes, &nodes);
    let at = |path: &V| nodes.iter().position(|node| &node["path"] == path);
    // A link whose end is not drawn is not drawn either. Nor is, where
    // that was asked for, a link that a longer way already joins: without
    // those the picture holds fewer links that pass layers, and every box
    // still reaches what it reached.
    let plain = plain_map(view);
    let mut left_out = if overlay.hide_transitive && plain {
        said_twice(view)
    } else {
        vec![false; edges.len()]
    };
    left_out.resize(edges.len(), false);
    let edges = edges
        .iter()
        .zip(&left_out)
        .filter(|(edge, said)| !**said && at(&edge["from"]).is_some() && at(&edge["to"]).is_some())
        .map(|(edge, _)| edge)
        .collect::<Vec<_>>();
    let pairs = edges
        .iter()
        .filter_map(|edge| Some((at(&edge["from"])?, at(&edge["to"])?)))
        .collect::<Vec<_>>();
    let weighted = pairs
        .iter()
        .zip(&edges)
        .map(|((from, to), edge)| (*from, *to, edge["links"].as_u64().unwrap_or(1)))
        .collect::<Vec<_>>();
    // A folder that is spread out is a frame around what it holds.
    let frame = |path: &V| frames.iter().position(|frame| &frame["path"] == path);
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
    let near = |node| {
        chosen == Some(node)
            || pairs
                .iter()
                .any(|&(from, to)| (from == node || to == node) && lit((from, to)))
    };
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
    // What each box is joined with by links that are led through no layer:
    // the boxes it uses far away, and the boxes far away that use it.
    let mut far: Vec<(Vec<usize>, Vec<usize>)> = vec![(vec![], vec![]); nodes.len()];
    for (wire_i, (wire, kinds)) in plan.wires.iter().zip(&wire_kinds).enumerate() {
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
        if against {
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
            let nums: Vec<f64> = wire
                .path
                .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
                .filter(|s| !s.is_empty())
                .filter_map(|s| s.parse::<f64>().ok())
                .collect();
            let pts: Vec<(f64, f64)> = nums
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&[x, y]| (x, y))
                .collect();
            dense_edges.push(graph_viewport::canvas2d::DenseEdgeSpec {
                index: wire_i,
                pts,
                weight: links.max(1),
                guessed: guess,
                against,
                kind: arrow_kind.to_owned(),
                on: edge_state == "on" || on_path,
                lit: lit((from, to)),
                dim: edge_state == "off",
                portal,
            });
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
        // no word, a chevron that says which way it runs.
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
    if let Some(lookup) = impact_lookup.as_ref() {
        let rendered_pairs: BTreeSet<(usize, usize)> = pairs.iter().copied().collect();
        let mut synthetic_drawn: BTreeSet<(usize, usize, &'static str)> = BTreeSet::new();
        let mut add_synthetic = |from_idx: usize,
                                 to_idx: usize,
                                 diff: &'static str,
                                 label: &str,
                                 kind: &'static str,
                                 impact: Option<&'static str>| {
            if from_idx == to_idx
                || rendered_pairs.contains(&(from_idx, to_idx))
                || !synthetic_drawn.insert((from_idx, to_idx, diff))
            {
                return;
            }
            let d = synthetic_edge_path(from_idx, to_idx, &plan.nodes, synthetic_drawn.len());
            let mark_symbol = if diff == "added" { "+" } else { "-" };
            let impact_attr = impact
                .map(|imp| format!(" data-impact=\"{imp}\""))
                .unwrap_or_default();
            let touched_attr = if commit_changes_active
                && (node_is_changed[from_idx] || node_is_changed[to_idx])
            {
                " data-commit-touched=\"true\""
            } else {
                ""
            };
            html.push_str(&format!(
                "<path class=\"graph-map-edge graph-map-edge-synthetic\" data-ends=\"{from_idx} {to_idx}\"{impact_attr}{touched_attr} data-diff=\"{diff}\" d=\"{d}\" data-edge-kind=\"{kind}\" data-kinds=\"{kind}\" stroke-width=\"2\" marker-end=\"url(#{id_prefix}-arrow{})\"><title>{} ({label} · {kind})</title></path>",
                arrow_marker_suffix(kind, false),
                escape(&m::text(&nodes[from_idx], "name")),
            ));
            if let Some((mx, my)) = middle(&d) {
                marks.push_str(&format!(
                    "<text class=\"graph-map-edge-mark\" data-ends=\"{from_idx} {to_idx}\" data-diff=\"{diff}\" x=\"{mx:.1}\" y=\"{my:.1}\">{mark_symbol}</text>"
                ));
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
    html.push_str(&marks);
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
            "<path class=\"graph-map-hit{}\" data-ends=\"{from} {to}\"{} d=\"{}\" data-action=\"graph-map-link\" data-payload=\"{}\" tabindex=\"0\" role=\"button\"><title>{} uses {}{}{breakdown_note} · select to see {}</title></path>",
            if wire.portal && edge["architecture_violation"].as_u64().unwrap_or(0) == 0 { " graph-map-hit-portal" } else { "" },
            if *hidden { " data-kind-hidden=\"true\"" } else { "" },
            wire.path,
            payload(&json!({"from": nodes[from]["path"], "to": nodes[to]["path"]})),
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
    let mut dense_nodes = Vec::new();
    let active_roving = overlay
        .roving_focus
        .filter(|i| *i < nodes.len())
        .or(chosen)
        .unwrap_or(0);
    for (index, node) in nodes.iter().enumerate() {
        let placed = &plan.nodes[index];
        if let Some(cone) = overlay.isolate_cone {
            if !cone.contains(&index) {
                continue;
            }
        }
        let is_pinned = pinned_set.contains(&index);
        if is_dense {
            let on_traced = overlay.path_nodes.is_some_and(|pn| pn.contains(&index));
            let lit_here = near(index);
            dense_nodes.push(graph_viewport::canvas2d::DenseNodeSpec {
                index,
                x: placed.x,
                y: placed.y,
                w: BOX_WIDTH,
                h: BOX_HEIGHT,
                label: m::text(node, "name"),
                role: resolved_member_kind(node).to_owned(),
                selected: chosen == Some(index),
                lit: lit_here || on_traced,
                dim: (selected && !lit_here)
                    || (overlay
                        .path_nodes
                        .is_some_and(|pn| !pn.is_empty() && !on_traced)),
                changed: node["changed"].as_u64().unwrap_or(0) > 0
                    || node_change_statuses[index].is_some(),
                pinned: is_pinned,
                failed: node["suspect"].as_u64().is_some(),
            });
            if let Some(bounds) = overlay.viewport_bounds {
                let is_important = chosen == Some(index)
                    || is_pinned
                    || node["suspect"].as_u64().is_some()
                    || overlay.path_nodes.is_some_and(|pn| pn.contains(&index));
                if !is_important
                    && !rect_intersects_viewport(
                        placed.x - 120.0,
                        placed.y - 120.0,
                        BOX_WIDTH + 240.0,
                        BOX_HEIGHT + 240.0,
                        bounds,
                    )
                {
                    continue;
                }
            }
        }
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
        // The whole project says more than the picture: a box is part of
        // a circle of files also where the rest of the circle is not drawn.
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
        // Where it stands among the changes the failing tests reach.
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
        // What a change to the selected place leaves here.
        let blast_attr = match blast[index] {
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
        // Of a path, a box that lies on a longer way only.
        let detour_attr = if m::yes(&node["detour"]) {
            label.push_str(", on a longer way only");
            " data-detour=\"true\""
        } else {
            ""
        };
        let lit = near(index);
        let path_mark = if !shared_edges.is_empty() && lit {
            " data-shared-path=\"true\""
        } else {
            ""
        };
        // What a run opened: a file itself, a folder by its files.
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
            format!(" aria-pressed=\"{}\"", chosen == Some(index))
        } else {
            String::new()
        };
        // Where followed links start, where a path starts and ends, or
        // which the two places are that share the rest.
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
        // What the two share: what both use, or what uses both.
        let both = shared && node["shares"].is_array();
        if both {
            label.push_str(if above(view) {
                ", uses both"
            } else {
                ", both use it"
            });
        }
        let changed = node["changed"].as_u64().unwrap_or(0);
        // What a file around an opened one is to it, or what a place of a
        // change map is to the change.
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
            // A type whose own lines stand as they were can still hold
            // methods the change touched.
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
        // What a part is in the pattern that is drawn, or in how many found
        // patterns a place of any other picture takes part.
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
        // A folder opens to its files, a file to what it holds. Among
        // followed links, on a path and in what two places share a box is
        // only selected: opening it would leave them.
        let opens = !scenario
            && !followed
            && !pathed
            && !shared
            && !cycle_view
            && !pattern_view
            && (folder || (!symbol && node["symbols"].as_u64().unwrap_or(0) > 0));
        // A symbol lies in the open file, or, among followed symbols, in
        // the file it says.
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
            // A layer is no place of the project: it has a name, no path.
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
        let position = match blast[index] {
            Some((_, left)) => {
                position.replacen(" style=\"", &format!(" style=\"--blast:{left:.2};"), 1)
            }
            None => position,
        };
        let mut pattern_node_attrs = String::new();
        if patterns > 0 {
            pattern_node_attrs.push_str(" data-has-patterns=\"true\"");
            if let Some(found_family) = pattern_family_of(view, &m::text(node, "path")) {
                pattern_node_attrs.push_str(&format!(" data-pattern-family=\"{found_family}\""));
            }
        } else if !layered_view {
            if let Some(family) = &pattern_family {
                pattern_node_attrs
                    .push_str(&format!(" data-pattern-family=\"{}\"", escape(family)));
                if let Some(role) = node["role"].as_str() {
                    pattern_node_attrs
                        .push_str(&format!(" data-pattern-role=\"{}\"", escape(role)));
                }
            }
        }
        let prefix_badge = if layered_view {
            let badge = if m::yes(&view["pattern"]["rings"]) && index == 0 {
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
        let node_change_status = node_change_statuses[index].as_deref();
        let node_change_attr = node_change_status
            .map(|status| format!(" data-change-status=\"{}\"", escape(status)))
            .unwrap_or_default();
        let node_impact_attr = impact_lookup
            .as_ref()
            .and_then(|lookup| lookup.node_impact(&nodes, index))
            .map(|impact| format!(" data-impact=\"{impact}\""))
            .unwrap_or_default();
        let (out, into) = flow[index];
        let focus_target_attr = if focus_overlay.nodes.contains(&index) {
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
        // A second line for a picture drawn large: through how many links
        // the box is used and uses others, and where it lies.
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
            secondary_badges.push(
                "<span class=\"graph-map-node-anchor\" aria-hidden=\"true\">★</span>".to_owned(),
            );
        }
        if cycle || circle.is_object() {
            // A folder says how many of its files are part of circles.
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
        let pin_attr = if is_pinned {
            " data-pinned=\"true\""
        } else {
            ""
        };
        let roving_tabindex = if active_roving == index { "0" } else { "-1" };
        let is_on_traced_path = overlay.path_nodes.is_some_and(|pn| pn.contains(&index));
        let dim_node = (selected && !lit)
            || (overlay
                .path_nodes
                .is_some_and(|pn| !pn.is_empty() && !is_on_traced_path));
        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-map-node\" data-n=\"{index}\" data-action=\"{node_action}\" data-payload=\"{}\" data-folder=\"{folder}\" data-kind=\"{}\" data-open=\"{opens}\" data-outside=\"{}\" data-changed=\"{}\" data-selected=\"{}\" data-dim=\"{}\" data-cycle=\"{}\" data-flow=\"{}\" data-out=\"{}\" data-in=\"{}\" data-start=\"{start}\"{}{}{}{path_mark}{run_mark}{focus_target_attr}{circle_attr}{detour_attr}{blast_attr}{node_change_attr}{node_impact_attr}{node_vis_attr}{pattern_node_attrs}{badge_attrs}{pin_attr}{position}{node_path_attr} tabindex=\"{roving_tabindex}\" data-roving-index=\"{index}\" title=\"{}\"{pressed} aria-label=\"{}\">{prefix_badge}{}<span class=\"graph-map-node-name\">{}</span>",
            payload(&node_payload),
            escape(kind),
            opened && !symbol,
            changed > 0,
            chosen == Some(index),
            dim_node,
            cycle,
            leaning(out, into),
            out > 0,
            into > 0,
            if end { " data-end=\"true\"" } else { "" },
            if both { " data-shared=\"true\"" } else { "" },
            if is_on_traced_path { " data-shared-path=\"true\"" } else { "" },
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
        ));
        if changed > 0 {
            // A changed file carries Git's letter and a folder how many
            // changed. So does a symbol: the letter where its own lines
            // changed, else how many symbols changed inside it.
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
            html.push_str(&format!(
                "<span class=\"graph-map-node-status\" data-status=\"{}\" aria-hidden=\"true\">{pill_text}</span>",
                escape(&pill_kind),
            ));
        } else if let Some(status) = node_change_status {
            let (badge, _) = change_status_badge(status);
            if !badge.is_empty() {
                html.push_str(&format!(
                    "<span class=\"graph-map-node-status\" data-status=\"{}\" aria-hidden=\"true\">{badge}</span>",
                    escape(status)
                ));
            }
        } else if folder {
            html.push_str(&format!(
                "<span class=\"graph-map-node-count\" aria-hidden=\"true\">{}</span>",
                count(node["files"].as_u64().unwrap_or(0))
            ));
        }
        // The second line is the last of what a box holds: it is laid out
        // under the rest, and only where the picture is drawn large.
        if !detail.is_empty() {
            html.push_str(&format!(
                "<span class=\"graph-map-node-detail\" aria-hidden=\"true\">{}</span>",
                escape(&detail)
            ));
        }
        if !blast_attr.is_empty() {
            html.push_str("<span class=\"graph-map-node-blast\" aria-hidden=\"true\"></span>");
        }
        html.push_str(&badges_strip);
        html.push_str("</button>");
        if followed {
            html.push_str(&handles(
                node,
                index,
                placed.x,
                placed.y,
                chosen.is_some() && !near(index),
            ));
        }
        // The links of this box that are led through no layer: a mark at
        // the side they leave by, and one at the side they arrive at, each
        // with how many there are. Resting on a mark draws them.
        for (side, others, sign) in [("out", &far[index].0, "⇢"), ("in", &far[index].1, "⇠")] {
            if others.is_empty() {
                continue;
            }
            let names = others
                .iter()
                .take(6)
                .map(|other| m::text(&nodes[*other], "name"))
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
            html.push_str(&format!(
                "<button type=\"button\" class=\"graph-map-portal\" data-of=\"{index}\" data-side=\"{side}\" data-dim=\"{}\" data-action=\"{node_action}\" data-payload=\"{}\" style=\"left:{:.0}px;top:{:.0}px\" title=\"{}\" aria-label=\"{}\">{sign}{}</button>",
                chosen.is_some() && !near(index),
                payload(&node_payload),
                if side == "out" { placed.x + BOX_WIDTH } else { placed.x },
                placed.y + BOX_HEIGHT,
                escape(&told),
                escape(&told),
                count(others.len() as u64),
            ));
        }
        if folder && spreads(view) {
            // A folder opens here too: + at its corner spreads it out.
            let told = format!(
                "Open {} here, among the others",
                escape(&m::text(node, "name"))
            );
            html.push_str(&format!(
                "<button type=\"button\" class=\"graph-map-spread\" data-of=\"{index}\" data-dim=\"{}\" data-action=\"graph-map-spread\" data-payload=\"{}\" style=\"left:{:.0}px;top:{:.0}px\" title=\"{told}\" aria-label=\"{told}\">+</button>",
                chosen.is_some() && !near(index),
                payload(&node["path"]),
                placed.x,
                placed.y,
            ));
        }
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
        node_keys,
        pairs,
        node_labels,
    }
}
/// How far past a corner the second handle of a side hangs.
pub(super) const HANDLE_GAP: f64 = 11.0;
/// What the whole project says of a box and the circles of files: a file is
/// part of one, a folder holds files of some, and of those some lead out of
/// the folder and back.
pub(super) fn circle_told(circle: &V, folder: bool) -> Option<String> {
    let files = circle["files"].as_u64().filter(|files| *files > 0)?;
    Some(if !folder {
        format!(
            ", part of a cycle of {}",
            plural(circle["largest"].as_u64().unwrap_or(0), "file", "files")
        )
    } else if m::yes(&circle["leaves"]) {
        format!(
            ", holds {} of cycles that lead out of it and back",
            plural(files, "file", "files")
        )
    } else {
        format!(
            ", holds {} that use each other in cycles",
            plural(files, "file", "files")
        )
    })
}
/// The circles of files that touch what is open, the largest first. Each
/// is shown as the shortest cycle through the file that has most to do
/// with the others.
pub(super) fn blast_side(view: &V) -> String {
    let blast = &view["focus"]["blast"];
    if !blast.is_object() {
        return String::new();
    }
    let number = |key: &str| blast[key].as_u64().unwrap_or(0);
    let (files, hops) = (number("files"), number("hops"));
    let mut html = format!(
        "<h4>A change here reaches <span class=\"graph-map-total\">{}</span></h4>",
        count(files)
    );
    if files == 0 {
        html.push_str(&format!(
            "<p class=\"knowledge-note\">Nothing else uses it within {}.</p>",
            plural(hops, "link", "links")
        ));
        return html;
    }
    // How many files each link further first reaches.
    let steps = m::array(&blast["by_hop"])
        .iter()
        .filter(|row| row["files"].as_u64().unwrap_or(0) > 0)
        .map(|row| {
            format!(
                "{} at {}",
                count(row["files"].as_u64().unwrap_or(0)),
                plural(row["hop"].as_u64().unwrap_or(0), "link", "links")
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let tests = number("tests");
    html.push_str(&format!(
        "<p class=\"knowledge-note\">Files that use it, and what uses those, up to {} away: {steps}.{} Each link passes on less of a change than it receives: a type built on it and a call most, an import little, a guessed link less; past an interface the type implements, less again. The bar under a box says how much is left there.{}</p>",
        plural(hops, "link", "links"),
        match tests {
            0 => String::new(),
            1 => " 1 of them is a test.".to_owned(),
            tests => format!(" {} of them are tests.", count(tests)),
        },
        if m::yes(&blast["cut"]) {
            " The walk stopped before the end: there is more."
        } else {
            ""
        },
    ));
    html.push_str("<ul class=\"graph-map-rows\">");
    for row in m::array(&blast["listed"]) {
        let path = m::text(&row, "path");
        let folder = parent(&path);
        let hop = row["hop"].as_u64().unwrap_or(0);
        let left = (row["left"].as_f64().unwrap_or(0.0) * 100.0).round() as u64;
        let through = m::yes(&row["through"]);
        html.push_str(&format!(
            "<li><button type=\"button\" class=\"graph-map-row\" data-blast-hop=\"{hop}\" data-action=\"graph-map-open\" data-payload=\"{}\" title=\"{} · reached at {} by {}{} · select to open it\"><span class=\"graph-map-row-name\">{}</span><span class=\"graph-map-row-links\">{}{} · {left} %{}{}</span></button></li>",
            payload(&row["path"]),
            escape(&path),
            escape(&m::text(&row, "name")),
            escape(&m::text(&row, "via")),
            if through { " through an interface" } else { "" },
            escape(base(&path)),
            if folder.is_empty() {
                String::new()
            } else {
                format!("{} · ", escape(folder))
            },
            plural(hop, "link", "links"),
            if through { " · past an interface" } else { "" },
            if m::yes(&row["test"]) { " · test" } else { "" },
        ));
    }
    html.push_str("</ul>");
    let more = number("more");
    if more > 0 {
        html.push_str(&format!(
            "<p class=\"knowledge-note\">and {} more</p>",
            count(more)
        ));
    }
    html
}
/// What a change to the selected place reaches of each box of a picture:
/// the fewest links to it and the most of the change left there. A file is
/// told by itself, a name by its file, a folder by the most reached file in
/// it. The selected box is told nothing.
pub(super) fn blast_marks(view: &V, nodes: &[V]) -> Vec<Option<(u64, f64)>> {
    let marked = m::array(&view["focus"]["blast"]["marked"])
        .iter()
        .filter_map(|row| {
            Some((
                row[0].as_str()?.to_owned(),
                row[1].as_u64()?,
                row[2].as_f64()?,
            ))
        })
        .collect::<Vec<_>>();
    if marked.is_empty() {
        return vec![None; nodes.len()];
    }
    let focus = m::text(&view["focus"], "path");
    nodes
        .iter()
        .map(|node| {
            let path = m::text(node, "path");
            if path == focus {
                return None;
            }
            let folder = m::yes(&node["folder"]);
            let file = node["file"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| strip_symbol_suffix(&path).to_owned());
            marked
                .iter()
                .filter(|(reached, ..)| {
                    if folder {
                        reached
                            .strip_prefix(path.as_str())
                            .is_some_and(|rest| rest.starts_with('/'))
                    } else {
                        *reached == file
                    }
                })
                .fold(None, |best: Option<(u64, f64)>, (_, hop, left)| {
                    Some(best.map_or((*hop, *left), |(near, most)| {
                        (near.min(*hop), most.max(*left))
                    }))
                })
        })
        .collect()
}
pub(super) fn circles_side(view: &V) -> String {
    let circles = &view["circles"];
    let listed = m::array(&circles["listed"]);
    if listed.is_empty() || cycles(view) {
        return String::new();
    }
    let number = |key: &str| circles[key].as_u64().unwrap_or(0);
    let (groups, here) = (number("groups"), number("here"));
    let mut html = format!(
        "<h4>Cycles of files <span class=\"graph-map-total\">{}</span></h4><p class=\"knowledge-note\">{}</p><ul class=\"graph-map-rows\">",
        count(here),
        if here == groups {
            format!(
                "{} of the project use each other in {}; the largest holds {}. A box marked ↻ is part of one, wherever the rest of it lies.",
                plural(number("files"), "file", "files"),
                plural(groups, "cycle", "cycles"),
                count(number("largest")),
            )
        } else {
            format!(
                "{} of the {} of the project touch what is open. A box marked ↻ is part of one, wherever the rest of it lies.",
                count(here),
                plural(groups, "cycle", "cycles"),
            )
        }
    );
    for circle in &listed {
        let start = m::text(circle, "start");
        let more = circle["more"].as_u64().unwrap_or(0);
        let names = m::array(&circle["names"])
            .iter()
            .filter_map(|name| name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        html.push_str(&format!(
            "<li><button type=\"button\" class=\"graph-map-row\" data-action=\"graph-map-cycle\" data-payload=\"{}\" title=\"Show the shortest cycle through {}\"><span class=\"graph-map-row-name\">{}{}</span><span class=\"graph-map-row-links\">{}</span></button></li>",
            payload(&circle["start"]),
            escape(&start),
            escape(&names),
            if more > 0 {
                format!(", +{}", count(more))
            } else {
                String::new()
            },
            plural(circle["files"].as_u64().unwrap_or(0), "file", "files"),
        ));
    }
    html.push_str("</ul>");
    if here > listed.len() as u64 {
        html.push_str(&format!(
            "<p class=\"knowledge-note\">and {} more</p>",
            count(here - listed.len() as u64)
        ));
    }
    html
}
/// The handles at the top corners of a box whose links are followed. The
/// right corner is for what the box uses and the left one for what uses it,
/// as the links leave on the right and arrive on the left. A side that is
/// unfolded has − to fold it back; a side with places not drawn has + with
/// how many, which unfolds the next of them.
pub(super) fn handles(node: &V, at: usize, x: f64, y: f64, dim: bool) -> String {
    let name = escape(&m::text(node, "name"));
    let mut html = String::new();
    for (side, corner, outward, hang) in [
        ("out", x + BOX_WIDTH, 1.0, "after"),
        ("in", x, -1.0, "before"),
    ] {
        let what = if side == "out" {
            format!("what {name} uses")
        } else {
            format!("what uses {name}")
        };
        let more = node[format!("more_{side}").as_str()].as_u64().unwrap_or(0);
        let unfolded = m::yes(&node[format!("unfolded_{side}").as_str()]);
        let asked = payload(&json!({"path": node["path"], "side": side}));
        if unfolded {
            html.push_str(&format!(
                "<button type=\"button\" class=\"graph-map-more\" data-of=\"{at}\" data-side=\"{side}\" data-dim=\"{dim}\" data-action=\"graph-map-fold\" data-payload=\"{asked}\" style=\"left:{corner:.0}px;top:{y:.0}px\" title=\"Hide {what}\" aria-label=\"Hide {what}\">−</button>"
            ));
        }
        if more > 0 {
            // Beside a − the + hangs outside the corner, clear of the box.
            let (at, hang) = if unfolded {
                (corner + outward * HANDLE_GAP, hang)
            } else {
                (corner, "")
            };
            let told = if unfolded {
                format!("Show more of {what}: {} not drawn", count(more))
            } else {
                format!("Show {what}: {} not drawn", count(more))
            };
            html.push_str(&format!(
                "<button type=\"button\" class=\"graph-map-more\" data-of=\"{at}\" data-side=\"{side}\" data-hang=\"{hang}\" data-dim=\"{dim}\" data-action=\"graph-map-unfold\" data-payload=\"{asked}\" style=\"left:{at:.0}px;top:{y:.0}px\" title=\"{told}\" aria-label=\"{told}\">+{}</button>",
                count(more)
            ));
        }
    }
    html
}
/// Where the names of a looked-into link stand: what uses in a column on
/// the left, what is used in a column on the right, each file a frame around
/// its names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ends {
    /// Where each name stands, in the order the names were given: those
    /// that use, file by file, then those that are used.
    pub names: Vec<(f64, f64)>,
    /// The frame of each file, in the same order.
    pub files: Vec<Framed>,
    /// The curve of each link.
    pub wires: Vec<String>,
    pub width: f64,
    pub height: f64,
}
/// Lays out the two ends of a link. `users` and `used` say how many names
/// each file of an end holds; a link names its two names by their numbers
/// among all of them, those that use first. Files and the names in them are
/// ordered so that the links cross as little as a few rounds can make them.
/// Both ends begin at the top, so a long end does not push the short one
/// out of sight.
pub fn ends(users: &[usize], used: &[usize], links: &[(usize, usize)]) -> Ends {
    let left = users.iter().sum::<usize>();
    let count = left + used.iter().sum::<usize>();
    let mut file = Vec::with_capacity(count);
    for (group, names) in users.iter().chain(used).enumerate() {
        file.extend(std::iter::repeat_n(group, *names));
    }
    let mut near = vec![vec![]; count];
    for (from, to) in links {
        if *from < left && (left..count).contains(to) {
            near[*from].push(*to);
            near[*to].push(*from);
        }
    }
    let mut order = [(0..left).collect::<Vec<_>>(), (left..count).collect()];
    let mut place = (0..count)
        .map(|name| (if name < left { name } else { name - left }) as f64)
        .collect::<Vec<_>>();
    // Each name moves towards the average place of what it is linked with
    // at the other end, and a file towards that of its names.
    for round in 0..6 {
        let side = 1 - round % 2;
        let mut keyed = order[side]
            .iter()
            .map(|name| {
                let linked = &near[*name];
                let middle = if linked.is_empty() {
                    place[*name]
                } else {
                    linked.iter().map(|other| place[*other]).sum::<f64>() / linked.len() as f64
                };
                (middle, place[*name], *name)
            })
            .collect::<Vec<_>>();
        let mut middles: BTreeMap<usize, (f64, f64)> = BTreeMap::new();
        for (middle, _, name) in &keyed {
            let of_file = middles.entry(file[*name]).or_insert((0.0, 0.0));
            of_file.0 += middle;
            of_file.1 += 1.0;
        }
        let middle = |name: usize| middles[&file[name]].0 / middles[&file[name]].1;
        keyed.sort_by(|a, b| {
            middle(a.2)
                .total_cmp(&middle(b.2))
                .then(file[a.2].cmp(&file[b.2]))
                .then(a.0.total_cmp(&b.0))
                .then(a.1.total_cmp(&b.1))
        });
        order[side] = keyed.into_iter().map(|(.., name)| name).collect();
        for (index, name) in order[side].iter().enumerate() {
            place[*name] = index as f64;
        }
    }
    let mut names = vec![(0.0, 0.0); count];
    let mut files = vec![Framed::default(); users.len() + used.len()];
    let mut tall = [0.0f64; 2];
    for (side, order) in order.iter().enumerate() {
        let x = MARGIN + FRAME_PAD + side as f64 * (LINK_BOX + LINK_GAP);
        let mut y = MARGIN;
        let mut open: Option<usize> = None;
        for name in order {
            let of = file[*name];
            if open == Some(of) {
                y += LINK_ROW;
            } else {
                // The frame before ends under its last name; this one
                // begins with the room for the name of its file.
                if open.is_some() {
                    y += FRAME_PAD + LINK_BETWEEN;
                }
                open = Some(of);
                files[of] = Framed {
                    x: x - FRAME_PAD,
                    y,
                    width: LINK_BOX + 2.0 * FRAME_PAD,
                    height: 0.0,
                };
                y += FRAME_HEAD;
            }
            names[*name] = (x, y);
            y += BOX_HEIGHT;
            files[of].height = y + FRAME_PAD - files[of].y;
        }
        if open.is_some() {
            y += FRAME_PAD;
        }
        tall[side] = y;
    }
    let tallest = tall[0].max(tall[1]);
    let wires = links
        .iter()
        .map(|(from, to)| {
            if *from >= left || !(left..count).contains(to) {
                return String::new();
            }
            let start = (names[*from].0 + LINK_BOX, names[*from].1 + BOX_HEIGHT / 2.0);
            let end = (names[*to].0, names[*to].1 + BOX_HEIGHT / 2.0);
            let mut path = format!("M{:.1} {:.1}", start.0, start.1);
            curve(&mut path, start, end);
            path
        })
        .collect();
    Ends {
        names,
        files,
        wires,
        width: 2.0 * (MARGIN + FRAME_PAD + LINK_BOX) + LINK_GAP,
        height: (tallest + MARGIN).max(BOX_HEIGHT + 2.0 * MARGIN),
    }
}
/// The names of a looked-into link in one list, those that use first, each
/// with the file it lies in; and how many names each file of each end holds.
pub(super) fn link_names(view: &V) -> (Vec<(V, V)>, [Vec<usize>; 2]) {
    let mut names = vec![];
    let mut held = [vec![], vec![]];
    for (side, end) in ["users", "used"].iter().enumerate() {
        for mut group in m::array(&view[*end]["groups"]) {
            let of_group = m::array(&group["names"]);
            held[side].push(of_group.len());
            // A name carries its file, not the file's other names.
            group["names"] = V::Null;
            names.extend(of_group.into_iter().map(|name| (name, group.clone())));
        }
    }
    (names, held)
}
#[derive(Default)]
pub(super) struct LinkDiffInfo {
    pub(super) pair_diff: Option<&'static str>,
    pub(super) symbol_diffs: BTreeMap<String, &'static str>,
    pub(super) symbol_risks: BTreeMap<String, String>,
    pub(super) warnings: Vec<V>,
}

pub(super) fn paths_overlap(a: &str, b: &str) -> bool {
    let na = normalize_rel_path(strip_symbol_suffix(a));
    let nb = normalize_rel_path(strip_symbol_suffix(b));
    if na.is_empty() || nb.is_empty() {
        return false;
    }
    na == nb || nb.starts_with(&format!("{na}/")) || na.starts_with(&format!("{nb}/"))
}

pub(super) fn merge_diff_priority(
    current: Option<&'static str>,
    next: &'static str,
) -> &'static str {
    match (current, next) {
        (Some("added"), _) | (_, "added") => "added",
        (Some("removed"), _) | (_, "removed") => "removed",
        _ => "unchanged",
    }
}

pub(super) fn norm_sym_token(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('.')
        .trim_end_matches("()")
        .to_ascii_lowercase()
}

impl LinkDiffInfo {
    pub(super) fn record_member_ref(&mut self, mref: &V, fallback_diff: Option<&'static str>) {
        let tok = norm_sym_token(&m::text(mref, "name"));
        if tok.is_empty() {
            return;
        }
        let change = m::text(mref, "change").trim().to_ascii_lowercase();
        let mapped = match change.as_str() {
            "added" => Some("added"),
            "removed" => Some("removed"),
            "modified" | "visibility_changed" | "unchanged" => Some("unchanged"),
            _ => fallback_diff,
        };
        if let Some(diff) = mapped {
            let merged = merge_diff_priority(self.symbol_diffs.get(&tok).copied(), diff);
            self.symbol_diffs.insert(tok.clone(), merged);
        }
        let risk = m::text(mref, "access_risk").trim().to_owned();
        if !risk.is_empty() {
            self.symbol_risks.insert(tok, risk);
        } else if m::yes(&mref["visibility_narrowed"]) {
            self.symbol_risks
                .insert(tok, "visibility_narrowed".to_owned());
        }
    }

    pub(super) fn record_syms(&mut self, syms: &V, diff: &'static str) {
        for sym in m::array(syms) {
            if let Some(name) = sym.as_str().map(norm_sym_token).filter(|s| !s.is_empty()) {
                let merged = merge_diff_priority(self.symbol_diffs.get(&name).copied(), diff);
                self.symbol_diffs.insert(name, merged);
            }
        }
    }

    pub(super) fn record_warning(&mut self, w: V) {
        let tok = norm_sym_token(&m::text(&w, "member"));
        if !tok.is_empty() {
            let reason = m::text(&w, "reason");
            self.symbol_risks.insert(
                tok,
                if reason.is_empty() {
                    "visibility risk".to_owned()
                } else {
                    reason
                },
            );
        }
        self.warnings.push(w);
    }

    pub(super) fn from_view(link_view: &V, impact: Option<&V>) -> Self {
        let mut info = Self::default();
        for w in m::array(&link_view["visibility_warnings"]) {
            info.record_warning(w);
        }
        for mref in m::array(&link_view["member_refs"]) {
            info.record_member_ref(&mref, None);
        }
        let Some(impact) = impact.filter(|v| v.is_object()) else {
            return info;
        };
        let impact_path = normalize_rel_path(strip_symbol_suffix(&m::text(impact, "path")));
        if impact_path.is_empty() {
            return info;
        }
        let from_path = normalize_rel_path(&m::text(&link_view["from"], "path"));
        let to_path = normalize_rel_path(&m::text(&link_view["to"], "path"));
        if paths_overlap(&from_path, &impact_path) {
            for (diff, list) in [
                ("added", &impact["uses"]["added"]),
                ("removed", &impact["uses"]["removed"]),
                ("unchanged", &impact["uses"]["unchanged"]),
            ] {
                for dep in m::array(list) {
                    let target = m::text(&dep, "target");
                    if paths_overlap(&to_path, &target) {
                        info.pair_diff = Some(merge_diff_priority(info.pair_diff, diff));
                        info.record_syms(&dep["symbols"], diff);
                        for mref in m::array(&dep["member_refs"]) {
                            info.record_member_ref(&mref, Some(diff));
                        }
                        for w in m::array(&dep["visibility_warnings"]) {
                            info.record_warning(w);
                        }
                    }
                }
            }
        }
        if paths_overlap(&to_path, &impact_path) {
            for caller in m::array(&impact["used_by"]) {
                let caller_path = m::text(&caller, "path");
                if paths_overlap(&from_path, &caller_path) {
                    let diff = match m::text(&caller, "impact").as_str() {
                        "broken_risk" => "removed",
                        "added_caller" => "added",
                        _ => "unchanged",
                    };
                    info.pair_diff = Some(merge_diff_priority(info.pair_diff, diff));
                    info.record_syms(&caller["changed_symbols"], diff);
                    info.record_syms(&caller["symbols"], diff);
                    for mref in m::array(&caller["used_members"]) {
                        info.record_member_ref(&mref, Some(diff));
                    }
                    for w in m::array(&caller["visibility_warnings"]) {
                        info.record_warning(w);
                    }
                }
            }
        }
        info
    }

    pub(super) fn diff_for_names(&self, from_name: &str, to_name: &str) -> Option<&'static str> {
        for candidate in [norm_sym_token(from_name), norm_sym_token(to_name)] {
            if let Some(diff) = self.symbol_diffs.get(&candidate) {
                return Some(diff);
            }
        }
        self.pair_diff
    }

    pub(super) fn risk_for_names(&self, from_name: &str, to_name: &str) -> Option<&str> {
        for candidate in [norm_sym_token(to_name), norm_sym_token(from_name)] {
            if let Some(risk) = self.symbol_risks.get(&candidate) {
                return Some(risk.as_str());
            }
        }
        None
    }
}

pub(super) fn diff_badge_sign(diff: &str) -> &'static str {
    match diff {
        "added" => "+",
        "removed" => "-",
        _ => "=",
    }
}

/// The picture of one link, looked into: the names that use on the left,
/// the names that are used on the right, each in the frame of its file, and
/// every link between two of them. `chosen` is the name whose links are lit.
#[cfg(test)]
pub fn link_canvas(view: &V, chosen: &str) -> String {
    link_canvas_with(view, chosen, None, &[false; 7])
}
/// Renders the link detail canvas with optional change-impact diff annotations (`+`/`-`/`=`).
/// A link of a kind the key has switched off (`off`) keeps its place and is not drawn.
pub fn link_canvas_with(view: &V, chosen: &str, impact: Option<&V>, off: &KindsOff) -> String {
    let diff_info = LinkDiffInfo::from_view(view, impact);
    let (names, held) = link_names(view);
    let at = |key: &V| names.iter().position(|(name, _)| &name["key"] == key);
    let edges = m::array(&view["edges"]);
    let pairs = edges
        .iter()
        .filter_map(|edge| Some((at(&edge["from"])?, at(&edge["to"])?, edge)))
        .collect::<Vec<_>>();
    let plan = ends(
        &held[0],
        &held[1],
        &pairs
            .iter()
            .map(|(from, to, _)| (*from, *to))
            .collect::<Vec<_>>(),
    );
    let users = held[0].iter().sum::<usize>();
    let picked = names.iter().position(|(name, _)| name["key"] == chosen);
    let near = |name: usize| {
        picked.is_some_and(|picked| {
            picked == name
                || pairs.iter().any(|(from, to, _)| {
                    (*from, *to) == (picked, name) || (*from, *to) == (name, picked)
                })
        })
    };
    // Every link runs between the same two columns, so the links of one
    // kind share one shading: from the colour of leaving to that of the kind.
    let link_kinds = pairs
        .iter()
        .map(|(_, _, edge)| EdgeKinds::of(edge))
        .collect::<Vec<_>>();
    let shadings: BTreeSet<&'static str> = link_kinds
        .iter()
        .filter_map(|kinds| kinds.primary(off))
        .chain(["calls"])
        .collect();
    let mut html = format!(
        "<div class=\"graph-map-plane\" style=\"width:{:.0}px;height:{:.0}px\"><svg width=\"{:.0}\" height=\"{:.0}\" aria-hidden=\"true\"><defs>{}",
        plan.width,
        plan.height,
        plan.width,
        plan.height,
        arrow_defs_svg("graph-link", false),
    );
    for kind in &shadings {
        let (suffix, kind_attr) = if *kind == "calls" {
            (String::new(), String::new())
        } else {
            (format!("-{kind}"), format!(" data-edge-kind=\"{kind}\""))
        };
        html.push_str(&format!(
            "<linearGradient id=\"graph-link-flow{suffix}\" class=\"graph-map-flow\"{kind_attr} gradientUnits=\"userSpaceOnUse\" x1=\"{:.0}\" y1=\"0\" x2=\"{:.0}\" y2=\"0\"><stop offset=\"0\" class=\"graph-map-flow-uses\"/><stop offset=\"0.5\" class=\"graph-map-flow-between\"/><stop offset=\"1\" class=\"graph-map-flow-used\"/></linearGradient>",
            MARGIN + FRAME_PAD + LINK_BOX,
            MARGIN + FRAME_PAD + LINK_BOX + LINK_GAP,
        ));
    }
    html.push_str("</defs>");
    for frame in &plan.files {
        if frame.width > 0.0 {
            html.push_str(&format!(
                "<rect class=\"graph-map-frame\" data-inner=\"false\" x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" rx=\"8\"/>",
                frame.x, frame.y, frame.width, frame.height
            ));
        }
    }
    for (((from, to, edge), wire), kinds) in pairs.iter().zip(&plan.wires).zip(&link_kinds) {
        let state = match picked {
            Some(picked) if picked == *from || picked == *to => " graph-map-edge-on",
            Some(_) => " graph-map-edge-off",
            None => "",
        };
        let guessed = m::yes(&edge["guessed"]);
        let from_name = m::text(&names[*from].0, "name");
        let to_name = m::text(&names[*to].0, "name");
        let edge_diff = diff_info.diff_for_names(&from_name, &to_name);
        let diff_attr = edge_diff
            .map(|diff| format!(" data-diff=\"{diff}\""))
            .unwrap_or_default();
        let diff_note = match edge_diff {
            Some("added") => " · + added",
            Some("removed") => " · - removed",
            Some("unchanged") => " · = unchanged",
            _ => "",
        };
        let from_vis = names[*from].0["visibility"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let to_vis = names[*to].0["visibility"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let vis_note = match (from_vis, to_vis) {
            (Some(fv), Some(tv)) => format!(" · {fv} → {tv}"),
            (None, Some(tv)) => format!(" · visibility: {tv}"),
            (Some(fv), None) => format!(" · visibility: {fv}"),
            (None, None) => String::new(),
        };
        let edge_risk = edge["access_risk"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                edge["visibility_warning"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            })
            .or_else(|| diff_info.risk_for_names(&from_name, &to_name));
        let risk_note = edge_risk
            .map(|r| format!(" · risk: {r}"))
            .unwrap_or_default();
        let shown_kind = kinds.primary(off);
        let hidden_attr = if shown_kind.is_none() {
            " data-kind-hidden=\"true\""
        } else {
            ""
        };
        let arrow_kind = shown_kind.unwrap_or_else(|| pick_precedence_kind(&kinds.counts));
        let multi_kind = kinds.multi(off);
        let kinds_breakdown = kinds.breakdown.clone();
        let raw_kind = m::text(edge, "kind");
        let kind_label = match (raw_kind.is_empty(), kinds_breakdown) {
            (true, Some(breakdown)) => breakdown,
            (false, Some(breakdown)) => {
                if multi_kind {
                    breakdown
                } else {
                    raw_kind.replace('_', " ")
                }
            }
            (false, None) => raw_kind.replace('_', " "),
            (true, None) => arrow_kind.to_owned(),
        };
        let multi_attr = if multi_kind {
            " data-edge-multi=\"true\""
        } else {
            ""
        };
        let marker_suffix = arrow_marker_suffix(arrow_kind, false);
        html.push_str(&format!(
            "<path class=\"graph-map-edge{state}{}\" data-ends=\"{from} {to}\"{diff_attr}{hidden_attr} d=\"{wire}\" data-edge-kind=\"{arrow_kind}\" data-kinds=\"{}\"{multi_attr} stroke=\"url(#graph-link-flow{})\" stroke-width=\"{}\" marker-start=\"url(#graph-link-port{})\" marker-end=\"url(#graph-link-arrow{marker_suffix})\"><title>{} uses {} · {}{}{diff_note}{}{}</title></path>",
            if guessed { " graph-map-edge-guess" } else { "" },
            kinds.held(),
            if arrow_kind == "calls" { String::new() } else { format!("-{arrow_kind}") },
            if guessed { "3.4" } else { "1.4" },
            if multi_kind { "-multi" } else { "" },
            escape(&from_name),
            escape(&to_name),
            escape(&kind_label),
            if guessed { ", guessed by name" } else { "" },
            escape(&vis_note),
            escape(&risk_note),
        ));
        // What was only guessed by name is drawn hollow.
        if guessed {
            html.push_str(&format!(
                "<path class=\"graph-map-edge-core\" data-ends=\"{from} {to}\"{hidden_attr} d=\"{wire}\" data-edge-kind=\"{arrow_kind}\" stroke-width=\"1.4\"/>"
            ));
        }
    }
    html.push_str("</svg>");
    // The head of a frame: the file its names lie in.
    let mut first = 0;
    for (frame, size) in plan.files.iter().zip(held.iter().flatten()) {
        if let Some((_, file)) = names.get(first).filter(|_| *size > 0) {
            html.push_str(&format!(
                "<span class=\"graph-map-frame-name\" style=\"left:{:.0}px;top:{:.0}px;max-width:{:.0}px\" title=\"{}\">{}</span>",
                frame.x + FRAME_PAD,
                frame.y + 3.0,
                frame.width - 2.0 * FRAME_PAD,
                escape(&m::text(file, "path")),
                escape(&m::text(file, "name")),
            ));
        }
        first += size;
    }
    for (index, (name, file)) in names.iter().enumerate() {
        let (x, y) = plan.names[index];
        let uses = index < users;
        let kind = resolved_member_kind(name);
        let line = name["line"].as_u64().unwrap_or(0);
        let links = name["links"].as_u64().unwrap_or(0);
        let node_vis = name["visibility"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let node_vis_attr = node_vis
            .and_then(|v| normalize_visibility(Some(v)))
            .map(|(bucket, _, _)| format!(" data-visibility=\"{bucket}\""))
            .unwrap_or_default();
        let node_vis_pill = visibility_pill_html(node_vis, true);
        let node_risk = name["access_risk"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| diff_info.risk_for_names(&m::text(name, "name"), &m::text(name, "name")));
        let place = match line {
            0 => m::text(file, "path"),
            line => format!("{}:{line}", m::text(file, "path")),
        };
        let title_place = if let Some(vis) = node_vis {
            format!("{place} · {vis}")
        } else {
            place.clone()
        };
        let mut label = format!(
            "{}, {}, {}, {} here",
            m::text(name, "name"),
            what(name),
            place,
            if uses {
                format!("uses through {}", plural(links, "link", "links"))
            } else {
                format!("is used through {}", plural(links, "link", "links"))
            }
        );
        if let Some(vis) = node_vis {
            label.push_str(&format!(", visibility {vis}"));
        }
        if let Some(risk) = node_risk {
            label.push_str(&format!(", visibility risk: {risk}"));
        }
        let (badge_attrs, badges_strip) = if node_vis_pill.is_empty() {
            (String::new(), String::new())
        } else {
            build_node_badges_strip(&[node_vis_pill])
        };
        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-map-node\" data-n=\"{index}\" data-action=\"graph-link-node\" data-payload=\"{}\" data-folder=\"false\" data-kind=\"{}\" data-selected=\"{}\" data-dim=\"{}\" data-flow=\"{}\" data-out=\"{uses}\" data-in=\"{}\"{node_vis_attr}{badge_attrs} style=\"left:{x:.0}px;top:{y:.0}px\" title=\"{}\" aria-label=\"{}\">{}<span class=\"graph-map-node-name\">{}</span>{}{badges_strip}</button>",
            payload(&name["key"]),
            escape(kind),
            picked == Some(index),
            picked.is_some() && !near(index),
            if uses { "uses" } else { "used" },
            !uses,
            escape(&title_place),
            escape(&label),
            match letter(kind) {
                "" => String::new(),
                letter => format!(
                    "<span class=\"graph-map-node-kind\" aria-hidden=\"true\">{letter}</span>"
                ),
            },
            escape(&m::text(name, "name")),
            if links > 1 {
                format!(
                    "<span class=\"graph-map-node-count\" aria-hidden=\"true\">{}</span>",
                    count(links)
                )
            } else {
                String::new()
            },
        ));
    }
    html.push_str("</div>");
    html
}
/// What the window of a link is called: what uses, and what is used.
pub fn link_title(view: &V) -> String {
    format!(
        "{} uses {}",
        m::text(&view["from"], "name"),
        m::text(&view["to"], "name")
    )
}
/// The link in a line: how many links it stands for and how they were
/// found, and how many names in how many files lie at each end.
pub fn link_summary(view: &V) -> String {
    if m::yes(&view["missing"]) {
        return "The graph does not know one end of this link any more.".to_owned();
    }
    let links = view["links"].as_u64().unwrap_or(0);
    let guessed = view["guessed"].as_u64().unwrap_or(0);
    if links == 0 {
        return if m::yes(&view["certain"]) {
            "No link read from the code runs this way.".to_owned()
        } else {
            "No link runs this way.".to_owned()
        };
    }
    let end = |side: &str| {
        let names = view[side]["names"].as_u64().unwrap_or(0);
        match view[side]["files"].as_u64().unwrap_or(0) {
            1 => plural(names, "name", "names"),
            files => format!(
                "{} in {} files",
                plural(names, "name", "names"),
                count(files)
            ),
        }
    };
    format!(
        "{}: {} read from the code, {} guessed by name · {} {} {}",
        plural(links, "link", "links"),
        count(links - guessed.min(links)),
        count(guessed),
        end("users"),
        if view["users"]["names"] == 1 {
            "uses"
        } else {
            "use"
        },
        end("used"),
    )
}
/// What of a link is not drawn, where it is more than a window holds.
pub fn link_note(view: &V) -> String {
    let (names, _) = link_names(view);
    let users = m::array(&view["users"]["groups"])
        .iter()
        .map(|group| m::array(&group["names"]).len() as u64)
        .sum::<u64>();
    let used = names.len() as u64 - users;
    let number = |side: &str| view[side]["names"].as_u64().unwrap_or(0);
    let drawn = m::array(&view["edges"]).len() as u64;
    let links = view["links"].as_u64().unwrap_or(0);
    let mut parts = vec![];
    if at_base(view) {
        parts.push(format!(
            "Links at PR base {}.",
            m::text(&view["base"], "commit")
        ));
    }
    if m::yes(&view["preliminary"]) {
        parts.push(
            "The links are those the files have in the checkout, not in that commit.".to_owned(),
        );
    }
    if users < number("users") || used < number("used") {
        parts.push(format!(
            "Showing the most linked names: {} of {} that use, {} of {} that are used, and {} of the {} between them. Look into a narrower link to see the rest: open the folder, or open it here.",
            count(users),
            count(number("users")),
            count(used),
            count(number("used")),
            count(drawn),
            plural(links, "link", "links"),
        ));
    } else if drawn < links {
        parts.push(format!(
            "Drawing {} of the {}.",
            count(drawn),
            plural(links, "link", "links")
        ));
    }
    parts.join(" ")
}
/// Beside the picture of a link: what the chosen name is and what it is
/// linked with here; with no name chosen, how to choose one and of which
/// kinds the links are.
#[cfg(test)]
pub fn link_side(view: &V, chosen: &str) -> String {
    link_side_with(view, chosen, None)
}
pub(super) fn link_group_members_html(view: &V) -> String {
    let mut html = String::new();
    for (side_key, side_label) in [("users", "Caller"), ("used", "Target")] {
        for group in m::array(&view[side_key]["groups"]) {
            let ref_members = m::array(&group["referenced_members"]);
            let decl_members = m::array(&group["declared_members"]);
            let gwarnings = m::array(&group["visibility_warnings"]);
            if ref_members.is_empty() && decl_members.is_empty() && gwarnings.is_empty() {
                continue;
            }
            let gname = m::text(&group, "name");
            let label = if gname.is_empty() {
                side_label.to_owned()
            } else {
                format!("{} ({side_label})", escape(&gname))
            };
            html.push_str(&impact_members_details_html(
                &ref_members,
                &decl_members,
                &gwarnings,
                MemberDetailsOpts {
                    member_filter: "all",
                    used_title: "Referenced members",
                    exposed_title: "Declared members",
                    summary_title: &label,
                    force_open: !gwarnings.is_empty(),
                },
            ));
        }
    }
    html
}
/// Renders the link detail sidebar with optional change-impact diff badges (`+`/`-`/`=`).
pub fn link_side_with(view: &V, chosen: &str, impact: Option<&V>) -> String {
    let diff_info = LinkDiffInfo::from_view(view, impact);
    let banner = diff_info
        .pair_diff
        .map(|diff| {
            let label = match diff {
                "added" => "Added dependency in this change",
                "removed" => "Removed or changed dependency",
                _ => "Kept dependency",
            };
            format!(
                "<p class=\"knowledge-note\"><span class=\"graph-status-pill\" data-status=\"{diff}\">{}</span> {label}</p>",
                diff_badge_sign(diff)
            )
        })
        .unwrap_or_default();
    let link_refs = m::array(&view["member_refs"]);
    let link_warnings = if diff_info.warnings.is_empty() {
        String::new()
    } else {
        vis_warnings_html(&diff_info.warnings)
    };
    let link_refs_html = impact_members_details_html(
        &link_refs,
        &[],
        &[],
        MemberDetailsOpts {
            member_filter: "all",
            used_title: "Member references",
            exposed_title: "",
            summary_title: "Member references",
            force_open: !diff_info.warnings.is_empty(),
        },
    );
    let group_members_html = link_group_members_html(view);
    let (names, _) = link_names(view);
    let Some((name, file)) = names.iter().find(|(name, _)| name["key"] == chosen) else {
        let kinds = m::array(&view["kinds"])
            .iter()
            .map(|kind| {
                format!(
                    "{} {}",
                    escape(&m::text(kind, "kind").replace('_', " ")),
                    count(kind["links"].as_u64().unwrap_or(0))
                )
            })
            .collect::<Vec<_>>();
        return format!(
            "{banner}<p class=\"knowledge-note\">Select a name to light its links.{}</p>{}{link_warnings}{link_refs_html}{group_members_html}",
            if at_base(view) {
                ""
            } else {
                " Select it again to open its file on it in the map."
            },
            if kinds.is_empty() {
                String::new()
            } else {
                format!(
                    "<p class=\"knowledge-note\">Links by kind: {}</p>",
                    kinds.join(" · ")
                )
            }
        );
    };
    let line = name["line"].as_u64().unwrap_or(0);
    let mut html = format!(
        "{banner}<h3>{}</h3><p class=\"graph-map-path\">{}{}</p><p class=\"knowledge-note\">{}</p>{}{link_warnings}",
        escape(&m::text(name, "name")),
        escape(&m::text(file, "path")),
        if line > 0 {
            format!(":{line}")
        } else {
            String::new()
        },
        escape(&capital(&what(name))),
        focus_member_meta_html(name),
    );
    if !at_base(view) {
        html.push_str(&format!(
            "<button type=\"button\" class=\"button compact\" data-action=\"graph-link-open\" data-payload=\"{}\">Open in the map</button>",
            payload(&name["key"]),
        ));
    }
    // What it is linked with in this link, each with the way it is.
    let edges = m::array(&view["edges"]);
    let self_name = m::text(name, "name");
    for (title, own, other) in [("Uses here", "from", "to"), ("Used by here", "to", "from")] {
        let linked = edges
            .iter()
            .filter(|edge| edge[own] == name["key"])
            .filter_map(|edge| {
                let (other, _) = names.iter().find(|(name, _)| name["key"] == edge[other])?;
                Some((other, edge))
            })
            .collect::<Vec<_>>();
        if linked.is_empty() {
            continue;
        }
        html.push_str(&format!(
            "<h4>{title} <span class=\"graph-map-total\">{}</span></h4><ul class=\"graph-map-rows\">",
            count(linked.len() as u64)
        ));
        for (other, edge) in linked {
            let other_name = m::text(other, "name");
            let other_vis = visibility_pill_html(other["visibility"].as_str(), true);
            let okind = resolved_member_kind(other);
            let other_kind = if !other_vis.is_empty()
                || matches!(
                    okind.trim().to_ascii_lowercase().as_str(),
                    "property" | "prop" | "field" | "const" | "constant" | "class_const"
                ) {
                member_kind_pill_html(okind)
            } else {
                String::new()
            };
            let risk_pill = diff_info
                .risk_for_names(&self_name, &other_name)
                .or_else(|| edge["access_risk"].as_str().filter(|s| !s.is_empty()))
                .map(|risk| {
                    format!(
                        " <span class=\"graph-status-pill\" data-status=\"broken_risk\" title=\"{}\">vis!</span>",
                        escape(risk)
                    )
                })
                .unwrap_or_default();
            let diff_pill = diff_info
                .diff_for_names(&self_name, &other_name)
                .map(|diff| {
                    format!(
                        " <span class=\"graph-status-pill\" data-status=\"{diff}\">{}</span>",
                        diff_badge_sign(diff)
                    )
                })
                .unwrap_or_default();
            html.push_str(&format!(
                "<li><button type=\"button\" class=\"graph-map-row\" data-action=\"graph-link-node\" data-payload=\"{}\"><span class=\"graph-map-row-name\">{other_vis}{other_kind}{}</span><span class=\"graph-map-row-links\">{}{}{risk_pill}{diff_pill}</span></button></li>",
                payload(&other["key"]),
                escape(&other_name),
                escape(&m::text(edge, "kind").replace('_', " ")),
                if m::yes(&edge["guessed"]) { ", guessed" } else { "" },
            ));
        }
        html.push_str("</ul>");
    }
    html.push_str(&link_refs_html);
    html.push_str(&group_members_html);
    html
}
/// The rows of one side of a selected place. `look` names the selected
/// place and whether the rows are what it uses: each row then stands for a
/// link between two places, and a button beside it looks into that link.
pub(super) fn rows(title: &str, rows: &V, more: &V, look: Option<(&V, bool)>) -> String {
    let rows = m::array(rows);
    let more = more.as_u64().unwrap_or(0);
    let mut html = format!(
        "<h4>{title} <span class=\"graph-map-total\">{}</span></h4>",
        count(rows.len() as u64 + more)
    );
    if rows.is_empty() {
        html.push_str("<p class=\"knowledge-note\">Nothing.</p>");
        return html;
    }
    html.push_str("<ul class=\"graph-map-rows\">");
    for row in &rows {
        // A symbol says which file it lies in where that is another than
        // the open one, and is reached by opening that file unless the
        // picture draws it, as among followed symbols.
        let home = row["file"].as_str().filter(|_| m::yes(&row["symbol"]));
        let elsewhere = home.filter(|_| !m::yes(&row["inside"]));
        let guessed = row["guessed"].as_u64().unwrap_or(0);
        // Two symbols are linked once, in one way; a place stands for many
        // links; what a type holds has a line.
        let told = match (row["relation"].as_str(), row["links"].as_u64()) {
            (Some(relation), _) => format!(
                "{}{}",
                escape(&relation.replace('_', " ")),
                if guessed > 0 { ", guessed" } else { "" }
            ),
            (None, Some(links)) => format!(
                "{}{}{}",
                weight(links, guessed),
                if m::yes(&row["against"]) {
                    " · against the order"
                } else {
                    ""
                },
                if m::yes(&row["opened"]) {
                    " · opened by the run"
                } else {
                    ""
                }
            ),
            _ => match row["line"].as_u64() {
                Some(line) if line > 0 => format!("line {line}"),
                _ => String::new(),
            },
        };
        let vis_pill = visibility_pill_html(row["visibility"].as_str(), true);
        let row_kind = resolved_member_kind(row);
        let kind_pill = if !vis_pill.is_empty()
            || matches!(
                row_kind.trim().to_ascii_lowercase().as_str(),
                "property" | "prop" | "field" | "const" | "constant" | "class_const"
            ) {
            member_kind_pill_html(row_kind)
        } else {
            String::new()
        };
        let ref_members = {
            let r = m::array(&row["referenced_members"]);
            if !r.is_empty() {
                r
            } else {
                let refs = m::array(&row["member_refs"]);
                if !refs.is_empty() {
                    refs
                } else {
                    m::array(&row["used_members"])
                }
            }
        };
        let decl_members = {
            let d = m::array(&row["declared_members"]);
            if !d.is_empty() {
                d
            } else {
                m::array(&row["target_members"])
            }
        };
        let row_warnings = m::array(&row["visibility_warnings"]);
        let members_details = impact_members_details_html(
            &ref_members,
            &decl_members,
            &row_warnings,
            MemberDetailsOpts {
                member_filter: "all",
                used_title: "Referenced members",
                exposed_title: "Exposed members",
                summary_title: "Properties &amp; methods",
                force_open: false,
            },
        );
        html.push_str(&format!(
            "<li><button type=\"button\" class=\"graph-map-row\" data-action=\"graph-map-go\" data-changed=\"{}\" data-payload=\"{}\" title=\"{}\"><span class=\"graph-map-row-name\">{vis_pill}{kind_pill}{}{}{}</span><span class=\"graph-map-row-links\">{}</span></button>{}{members_details}</li>",
            m::yes(&row["changed"]),
            payload(&json!({"path": row["path"], "inside": row["inside"], "open": elsewhere})),
            escape(&m::text(row, "path")),
            escape(&m::text(row, "name")),
            if m::yes(&row["folder"]) { "/" } else { "" },
            match home {
                Some(file) => format!(
                    "<span class=\"graph-map-row-file\">{}</span>",
                    escape(base(file))
                ),
                None => String::new(),
            },
            told,
            match look {
                Some((place, uses)) => {
                    let (from, to) = if uses {
                        (place, &row["path"])
                    } else {
                        (&row["path"], place)
                    };
                    format!(
                        "<button type=\"button\" class=\"graph-map-row-look\" data-action=\"graph-map-link\" data-payload=\"{}\" title=\"See what makes up this link\" aria-label=\"See what makes up the link with {}\">↗</button>",
                        payload(&json!({"from": from, "to": to})),
                        escape(&m::text(row, "name")),
                    )
                }
                _ => String::new(),
            },
        ));
    }
    html.push_str("</ul>");
    if more > 0 {
        html.push_str(&format!(
            "<p class=\"knowledge-note\">and {} more</p>",
            count(more)
        ));
    }
    html
}
pub(super) fn kinds(focus: &V) -> String {
    let kinds = m::array(&focus["kinds"])
        .iter()
        .map(|kind| {
            format!(
                "{} {}",
                escape(&m::text(kind, "kind").replace('_', " ")),
                count(kind["links"].as_u64().unwrap_or(0))
            )
        })
        .collect::<Vec<_>>();
    if kinds.is_empty() {
        String::new()
    } else {
        format!(
            "<p class=\"knowledge-note\">Uses by kind: {}</p>",
            kinds.join(" · ")
        )
    }
}
pub(super) fn open_file(path: &V) -> String {
    format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-open\" data-payload=\"{}\">Open file</button>",
        payload(path)
    )
}
/// Follows the links of a folder, a file or a symbol step by step, wherever
/// they lead.
pub(super) fn follow_links(path: &V) -> String {
    format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-follow\" data-payload=\"{}\">Follow links</button>",
        payload(path)
    )
}
/// Asks for the shortest ways from a place to another, which is chosen
/// next: a box of any picture, or a place the search finds.
pub(super) fn path_to(place: &V) -> String {
    // The keys stand as the alphabet has them, which is how a payload is
    // written whether or not the build's JSON keeps their order.
    format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-path\" data-payload=\"{}\">Path to…</button>",
        payload(&json!({"name": place["name"], "path": place["path"]}))
    )
}
/// What a symbol is, with what the change did to it: to its own lines, and
/// to how many symbols inside it.
pub(super) fn reached(symbol: &V, mut text: String) -> String {
    if let Some(status) = symbol["status"].as_str() {
        text.push_str(&format!(" · {} by the change", happened(status)));
    }
    match symbol["touched"].as_u64() {
        Some(inside) if inside > 0 => {
            text.push_str(&format!(" · {} changed inside", count(inside)));
        }
        _ => {}
    }
    text
}
pub(super) fn symbol_focus_payload(member: &V, nodes: &[V]) -> Option<V> {
    let explicit_path = m::text(member, "path");
    if !explicit_path.is_empty() {
        return Some(json!({ "path": explicit_path, "inside": true }));
    }
    let mname = m::text(member, "name");
    let mline = member["line"].as_u64().unwrap_or(0);
    if mname.is_empty() {
        return None;
    }
    nodes.iter().find_map(|node| {
        if !m::yes(&node["symbol"]) {
            return None;
        }
        let nname = m::text(node, "name");
        let nline = node["line"].as_u64().unwrap_or(0);
        let name_matches = nname == mname
            || nname.ends_with(&format!("::{mname}"))
            || nname.ends_with(&format!(".{mname}"));
        let line_matches = mline > 0 && nline == mline;
        if name_matches || line_matches {
            Some(json!({ "path": node["path"], "inside": true }))
        } else {
            None
        }
    })
}
pub(super) fn file_declared_members_html(members: &[V], view: &V) -> String {
    if members.is_empty() {
        return String::new();
    }
    let nodes = m::array(&view["nodes"]);
    let mut html = format!(
        "<details class=\"graph-impact-members\" open><summary class=\"graph-impact-members-summary\"><span class=\"graph-impact-members-label\">File properties &amp; methods ({})</span></summary><ul class=\"graph-member-list\">",
        members.len()
    );
    for member in members {
        let vis_pill = visibility_pill_html(member["visibility"].as_str(), false);
        let kind_pill = member_kind_pill_html(resolved_member_kind(member));
        let name = m::text(member, "name");
        let sig = m::text(member, "signature");
        let display_sig = if sig.is_empty() {
            name.clone()
        } else if !name.is_empty() && !sig.contains(&name) {
            format!("{name}{sig}")
        } else {
            sig.clone()
        };
        let title = if sig.is_empty() { &name } else { &sig };
        let line = member["line"]
            .as_u64()
            .filter(|&l| l > 0)
            .map(|l| format!("<span class=\"graph-member-line\">:{l}</span>"))
            .unwrap_or_default();
        if let Some(go_payload) = symbol_focus_payload(member, &nodes) {
            html.push_str(&format!(
                "<li><button type=\"button\" class=\"graph-map-row graph-member-row graph-member-row-btn\" data-action=\"graph-map-go\" data-payload=\"{}\" title=\"{}\">{vis_pill}{kind_pill}<span class=\"graph-member-sig\">{}</span>{line}</button></li>",
                payload(&go_payload),
                escape(title),
                escape(&display_sig),
            ));
        } else {
            html.push_str(&format!(
                "<li class=\"graph-member-row\">{vis_pill}{kind_pill}<span class=\"graph-member-sig\" title=\"{}\">{}</span>{line}</li>",
                escape(title),
                escape(&display_sig),
            ));
        }
    }
    html.push_str("</ul></details>");
    html
}
pub(super) fn focus_member_meta_html(focus: &V) -> String {
    let vis_pill = visibility_pill_html(focus["visibility"].as_str(), false);
    let kind_str = resolved_member_kind(focus);
    let kind_pill = if !vis_pill.is_empty()
        || matches!(
            kind_str.trim().to_ascii_lowercase().as_str(),
            "property" | "prop" | "field" | "const" | "constant" | "class_const"
        ) {
        member_kind_pill_html(kind_str)
    } else {
        String::new()
    };
    let prev_vis = focus["previous_visibility"].as_str();
    let curr_vis = focus["visibility"].as_str();
    let vis_changed = m::yes(&focus["visibility_narrowed"])
        || m::yes(&focus["visibility_changed"])
        || (prev_vis.is_some() && curr_vis.is_some() && prev_vis != curr_vis);
    let vis_delta = if vis_changed {
        vis_delta_badge_html(prev_vis, curr_vis, m::yes(&focus["visibility_narrowed"]))
    } else {
        String::new()
    };
    if vis_pill.is_empty() && kind_pill.is_empty() && vis_delta.is_empty() {
        String::new()
    } else {
        format!("<div class=\"graph-member-meta\">{vis_pill}{kind_pill}{vis_delta}</div>")
    }
}
/// The side panel of an opened file: what the selected symbol uses and what
/// uses it, or which symbols of a file around are linked with this one.
pub(super) fn file_side(view: &V) -> String {
    let focus = &view["focus"];
    if !focus.is_object() {
        let declared = m::array(&view["declared_members"]);
        let members_html = file_declared_members_html(&declared, view);
        return format!(
            "<p class=\"knowledge-note\">Select a class, a method or a function to see what it uses and what uses it; Follow links then walks on from it, a step at a time, into whatever files they lead to. A box without a letter is another file that this one uses or is used by: select it again to open it.</p>{members_html}{}",
            unopened(view)
        );
    }
    if m::yes(&focus["symbol"]) {
        let line = focus["line"].as_u64().unwrap_or(0);
        let mut html = format!(
            "<h3>{}</h3><p class=\"graph-map-path\">{}{}</p><p class=\"knowledge-note\">{}</p>{}",
            escape(&m::text(focus, "name")),
            escape(&m::text(focus, "file")),
            if line > 0 {
                format!(":{line}")
            } else {
                String::new()
            },
            escape(&reached(focus, capital(&what(focus)))),
            focus_member_meta_html(focus),
        );
        // What is linked with nothing has no links to follow, and no path
        // leads from it.
        if focus["out"].as_u64().unwrap_or(0) + focus["in"].as_u64().unwrap_or(0) > 0 {
            html.push_str(&follow_links(&focus["path"]));
            html.push_str(&path_to(focus));
        }
        if !m::array(&focus["members"]).is_empty() {
            html.push_str(&rows(
                "Members",
                &focus["members"],
                &focus["members_more"],
                None,
            ));
        }
        let ref_members = m::array(&focus["referenced_members"]);
        let decl_members = m::array(&focus["declared_members"]);
        let vis_warnings = m::array(&focus["visibility_warnings"]);
        html.push_str(&impact_members_details_html(
            &ref_members,
            &decl_members,
            &vis_warnings,
            MemberDetailsOpts {
                member_filter: "all",
                used_title: "Referenced members",
                exposed_title: "Declared members",
                summary_title: "Properties &amp; methods",
                force_open: false,
            },
        ));
        html.push_str(&rows("Uses", &focus["uses"], &focus["uses_more"], None));
        html.push_str(&rows(
            "Used by",
            &focus["used_by"],
            &focus["used_by_more"],
            None,
        ));
        html.push_str(&kinds(focus));
        return html;
    }
    let held = focus["symbols"].as_u64().unwrap_or(0);
    let mut html = format!(
        "<h3>{}</h3><p class=\"graph-map-path\">{}</p><p class=\"knowledge-note\">{}{} · {}</p>",
        escape(&m::text(focus, "name")),
        escape(&m::text(focus, "path")),
        match focus["status"].as_str() {
            Some(status) => format!("Changed file · {}", happened(status)),
            None => "Another file".to_owned(),
        },
        match focus["role"].as_str() {
            Some("uses") => " · uses this file",
            Some("used") => " · used by this file",
            _ => " · uses this file and is used by it",
        },
        plural(held, "symbol", "symbols"),
    );
    if held > 0 {
        html.push_str(&open_file(&focus["path"]));
    }
    html.push_str(&follow_links(&focus["path"]));
    html.push_str(&path_to(focus));
    let vis_warnings = m::array(&focus["visibility_warnings"]);
    html.push_str(&vis_warnings_html(&vis_warnings));
    let ref_members = m::array(&focus["referenced_members"]);
    let decl_members = m::array(&focus["declared_members"]);
    html.push_str(&impact_members_details_html(
        &ref_members,
        &decl_members,
        &[],
        MemberDetailsOpts {
            member_filter: "all",
            used_title: "Referenced members",
            exposed_title: "Declared members",
            summary_title: "Properties &amp; methods",
            force_open: !vis_warnings.is_empty(),
        },
    ));
    // Its own symbols at the other end of the links with this file.
    html.push_str(&rows(
        "Uses this file",
        &focus["uses"],
        &focus["uses_more"],
        None,
    ));
    html.push_str(&rows(
        "Used by this file",
        &focus["used_by"],
        &focus["used_by_more"],
        None,
    ));
    html.push_str(&kinds(focus));
    html
}
/// What a selected box of a path is to the path: its start or its end.
pub(super) fn path_end(focus: &V) -> &'static str {
    match (m::yes(&focus["start"]), m::yes(&focus["end"])) {
        (true, _) => " · the path starts here",
        (_, true) => " · the path ends here",
        _ => "",
    }
}
/// What a selected box is to the picture it is selected in: where followed
/// links start, an end of a path, or, of two places that share something,
/// one of the two or what they share, with how many links each of them is
/// away.
pub(super) fn role(view: &V, focus: &V) -> String {
    if cycles(view) {
        return if m::yes(&focus["start"]) {
            " · the cycle starts and ends here".to_owned()
        } else {
            String::new()
        };
    }
    if paths(view) {
        return path_end(focus).to_owned();
    }
    if !shares(view) {
        return if m::yes(&focus["start"]) {
            " · the links are followed from here".to_owned()
        } else {
            String::new()
        };
    }
    if m::yes(&focus["start"]) {
        return " · the first of the two places".to_owned();
    }
    if m::yes(&focus["end"]) {
        return " · the second of the two places".to_owned();
    }
    let shared = &view["shared"];
    let far = m::array(&focus["shares"]);
    let [first, second] = far.as_slice() else {
        return String::new();
    };
    let links = |far: &V| plural(far.as_u64().unwrap_or(0), "link", "links");
    let (told, joined) = if above(view) {
        ("uses both", "to")
    } else {
        ("both use it", "from")
    };
    format!(
        " · {told}: {} {joined} {}, {} {joined} {}",
        links(first),
        m::text(&shared["from"], "name"),
        links(second),
        m::text(&shared["to"], "name")
    )
}
/// The side panel of a symbol selected among followed symbols, on a path or
/// in what two places share: what it is and where it lies, with what `role`
/// says it is to the picture, the way to its file, to its own links and to
/// a path from it, and what it uses and what uses it, each with the file it
/// lies in. Where the boxes lie on ways (`ways`) those are the next and the
/// one before.
pub(super) fn name_side(view: &V, focus: &V, ways: bool, role: &str) -> String {
    let line = focus["line"].as_u64().unwrap_or(0);
    let mut told = reached(focus, capital(&what(focus)));
    told.push_str(role);
    let mut html = format!(
        "<h3>{}</h3><p class=\"graph-map-path\">{}{}</p><p class=\"knowledge-note\">{}</p>{}",
        escape(&m::text(focus, "name")),
        escape(&m::text(focus, "file")),
        if line > 0 {
            format!(":{line}")
        } else {
            String::new()
        },
        escape(&told),
        focus_member_meta_html(focus),
    );
    if !at_base(view) {
        html.push_str(&format!(
            "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-go\" data-payload=\"{}\">Open in its file</button>",
            payload(&json!({"open": focus["file"], "path": focus["path"]})),
        ));
        // The symbol the links are followed from is followed already.
        if ways || !m::yes(&focus["start"]) {
            html.push_str(&follow_links(&focus["path"]));
        }
        html.push_str(&path_to(focus));
    }
    let (uses, used) = if ways { WAY } else { ("Uses", "Used by") };
    let look = |outgoing| at_base(view).then_some((&focus["path"], outgoing));
    html.push_str(&rows(uses, &focus["uses"], &focus["uses_more"], look(true)));
    html.push_str(&rows(
        used,
        &focus["used_by"],
        &focus["used_by_more"],
        look(false),
    ));
    let ref_members = m::array(&focus["referenced_members"]);
    let decl_members = m::array(&focus["declared_members"]);
    let vis_warnings = m::array(&focus["visibility_warnings"]);
    html.push_str(&impact_members_details_html(
        &ref_members,
        &decl_members,
        &vis_warnings,
        MemberDetailsOpts {
            member_filter: "all",
            used_title: "Referenced members",
            exposed_title: "Declared members",
            summary_title: "Properties &amp; methods",
            force_open: false,
        },
    ));
    html.push_str(&kinds(focus));
    html
}
/// Asks another question of the same two places: what they share (`true`),
/// one kind of it by its name, or the path between them again (`false`).
pub(super) fn ask_shared(asked: V, class: &str) -> String {
    format!(
        "<button type=\"button\" class=\"{class}\" data-action=\"graph-map-shared\" data-payload=\"{}\">{}</button>",
        payload(&asked),
        match (asked.as_str(), asked.as_bool()) {
            (Some("uses_both"), _) => "What uses both",
            (Some(_), _) => "What both use",
            (None, Some(true)) => "What they share",
            _ => "The path between them",
        }
    )
}
/// The other questions that two places shown can be asked, as their
/// payloads: of a path, what the two share; of what they share, the other
/// kind where there is any of it, and the path between them. Two places of
/// which one lies in the other can be asked nothing more.
pub(super) fn asks(view: &V) -> Vec<V> {
    if paths(view) {
        return if m::yes(&view["path"]["within"]) {
            vec![]
        } else {
            vec![json!(true)]
        };
    }
    let shared = &view["shared"];
    if !shares(view) || m::yes(&shared["within"]) {
        return vec![];
    }
    let other = if above(view) { "both_use" } else { "uses_both" };
    let mut asks = vec![];
    if shared[other].as_u64().unwrap_or(0) > 0 {
        asks.push(json!(other));
    }
    asks.push(json!(false));
    asks
}
/// Beside a picture of two places that draws nothing: a path, because no
/// chain of links joins its two ends; what the two share, because there is
/// nothing of the kind; or either, because one of the two lies in the
/// other. The other questions that can be asked of the two come first,
/// then each of the two with the way to its own links and to a path from
/// it to another place.
pub(super) fn apart(view: &V) -> String {
    let asks = asks(view);
    let ends = if paths(view) {
        &view["path"]
    } else {
        &view["shared"]
    };
    let mut html = "<p class=\"knowledge-note\">Nothing is drawn.".to_owned();
    for ask in &asks {
        html.push_str(match (ask.as_str(), ask.as_bool()) {
            (Some("uses_both"), _) => " What uses both is another picture.",
            (Some(_), _) => " What both use is another picture.",
            (None, Some(true)) => " What they share shows what both use and what uses both.",
            _ => " The path between them asks how one of the two reaches the other.",
        });
    }
    html.push_str(" Follow links shows what each of the two reaches, and Path to… asks for the way from it to another place.</p>");
    for ask in asks {
        html.push_str(&ask_shared(ask, "button compact"));
    }
    for end in [&ends["from"], &ends["to"]] {
        // A symbol lies at a line of its file; a file or a folder is its path.
        let lies = if m::yes(&end["symbol"]) {
            let line = end["line"].as_u64().unwrap_or(0);
            format!(
                "{}{}",
                m::text(end, "file"),
                if line > 0 {
                    format!(":{line}")
                } else {
                    String::new()
                }
            )
        } else {
            m::text(end, "path")
        };
        html.push_str(&format!(
            "<h4>{}</h4><p class=\"graph-map-path\">{}</p>{}{}",
            escape(&m::text(end, "name")),
            escape(&lies),
            follow_links(&end["path"]),
            path_to(end)
        ));
    }
    html
}
/// Pairs of files of two layers of which the first needs the second: each a
/// row that opens what makes up their link. `layers` says of each pair which
/// layers its two files are filed in, where the pairs are not all of the
/// same two; `none` is said where there are no pairs.
pub(super) fn pair_rows(title: &str, pairs: &V, none: &str, layers: bool) -> String {
    pair_rows_of(title, pairs, none, layers, false)
}
/// `gone` is for pairs a change took away: the link is no longer there to
/// look into, so such a row is no button.
pub(super) fn pair_rows_of(title: &str, pairs: &V, none: &str, layers: bool, gone: bool) -> String {
    let rows = m::array(&pairs["rows"]);
    let more = pairs["more"].as_u64().unwrap_or(0);
    let mut html = format!(
        "<h4>{title} <span class=\"graph-map-total\">{}</span></h4>",
        count(rows.len() as u64 + more)
    );
    if rows.is_empty() {
        html.push_str(&format!("<p class=\"knowledge-note\">{none}</p>"));
        return html;
    }
    html.push_str("<ul class=\"graph-map-rows\">");
    for row in &rows {
        let (from, to) = (&row["from"], &row["to"]);
        let told = format!(
            "<span class=\"graph-map-row-name\">{} → {}</span><span class=\"graph-map-row-links\">{}{}{}</span>",
            escape(&m::text(from, "name")),
            escape(&m::text(to, "name")),
            if layers {
                format!(
                    "{} → {} · ",
                    escape(&m::text(from, "layer")),
                    escape(&m::text(to, "layer"))
                )
            } else {
                String::new()
            },
            weight(
                row["links"].as_u64().unwrap_or(0),
                row["guessed"].as_u64().unwrap_or(0)
            ),
            if m::yes(&row["added"]) {
                " · added by this change"
            } else {
                ""
            },
        );
        html.push_str(&if gone {
            format!(
                "<li><span class=\"graph-map-row\" data-pair=\"true\" data-gone=\"true\" title=\"{} used {}\">{told}</span></li>",
                escape(&m::text(from, "path")),
                escape(&m::text(to, "path")),
            )
        } else {
            format!(
                "<li><button type=\"button\" class=\"graph-map-row\" data-pair=\"true\" data-action=\"graph-map-link\" data-payload=\"{}\" title=\"{} uses {} · select to see what makes up this link\">{told}</button></li>",
                payload(&json!({"from": from["path"], "to": to["path"]})),
                escape(&m::text(from, "path")),
                escape(&m::text(to, "path")),
            )
        });
    }
    html.push_str("</ul>");
    if more > 0 {
        html.push_str(&format!(
            "<p class=\"knowledge-note\">and {} more</p>",
            count(more)
        ));
    }
    html
}
/// The side panel of the layers of a folder: what runs against their order;
/// of a selected layer, what places a file in it, what it has with the other
/// layers, what of that runs against the order, and its files; or, of a
/// selected link between two layers, the pairs of files it is made of.
pub(super) fn layers_side(view: &V) -> String {
    let declared = m::yes(&view["pattern"]["declared"]);
    let focus = &view["focus"];
    if !focus.is_object() {
        // What a chosen change did to the order comes first: it is what
        // the change is looked at for.
        let change = &view["pattern"]["change"];
        let did = if change["state"] == "ready" {
            let mut did = pair_rows(
                "Added by this change",
                &change["added"],
                "This change sets nothing against the order.",
                true,
            );
            if change["removed"]["pairs"].as_u64().unwrap_or(0) > 0 {
                did.push_str(&pair_rows_of(
                    "Gone with this change",
                    &change["removed"],
                    "",
                    true,
                    true,
                ));
            }
            did
        } else {
            String::new()
        };
        let intro = if declared {
            "Select a layer to see its files or a link to inspect its dependencies."
        } else {
            "A box is a layer: the files a name or a folder places in it. Select a layer to see its files and what it has with the others, or a link to see the pairs of files it is made of. Select it again to let go of it."
        };
        return format!(
            "<p class=\"knowledge-note\">{intro}</p>{did}{}",
            pair_rows(
                if declared {
                    "Rule violations"
                } else {
                    "Against the order"
                },
                &view["pattern"]["against"],
                if declared {
                    "No forbidden dependencies found."
                } else {
                    "No layer here needs one before it."
                },
                true
            )
        );
    }
    if m::yes(&focus["link"]) {
        // The two layers are said once, above the pairs.
        return format!(
            "<h3>{} → {}</h3><p class=\"knowledge-note\">Link between two layers · {} · {}{}</p>{}",
            escape(&m::text(&focus["from"], "name")),
            escape(&m::text(&focus["to"], "name")),
            weight(
                focus["links"].as_u64().unwrap_or(0),
                focus["guessed"].as_u64().unwrap_or(0)
            ),
            plural(
                focus["pairs"].as_u64().unwrap_or(0),
                "pair of files",
                "pairs of files"
            ),
            if m::yes(&focus["against"]) {
                if declared {
                    " · forbidden by project rules"
                } else {
                    " · against the order"
                }
            } else {
                ""
            },
            pair_rows("Pairs of files", focus, "None.", false),
        );
    }
    let mut html = format!(
        "<h3>{}</h3><p class=\"knowledge-note\">Layer · {} · placed by {}</p>",
        escape(&m::text(focus, "name")),
        plural(focus["files"].as_u64().unwrap_or(0), "file", "files"),
        escape(&m::text(focus, "signs")),
    );
    // Each row is a link of the layer with another: looked into, it is
    // selected, and lists its pairs of files.
    let place = &focus["path"];
    html.push_str(&rows("Uses", &focus["uses"], &V::Null, Some((place, true))));
    html.push_str(&rows(
        "Used by",
        &focus["used_by"],
        &V::Null,
        Some((place, false)),
    ));
    html.push_str(&pair_rows(
        if declared {
            "Rule violations from this layer"
        } else {
            "Against the order, from here"
        },
        &focus["against"],
        if declared {
            "No forbidden dependencies found."
        } else {
            "Nothing here needs a layer before this one."
        },
        true,
    ));
    html.push_str(&rows(
        "Files",
        &focus["members"],
        &focus["members_more"],
        None,
    ));
    html
}
/// Structured summary card shown in the side panel for one focused pattern
/// (`architecture` or `design`), including its family, confidence badges, and
/// interactive role participant chips.
pub(super) fn pattern_brief(view: &V) -> String {
    let pattern = &view["pattern"];
    if !pattern.is_object() {
        return String::new();
    }
    let family = match pattern["family"].as_str() {
        Some("architecture") => "architecture",
        _ => "design",
    };
    let family_label = if family == "architecture" {
        "Architecture pattern"
    } else {
        "Design pattern"
    };
    let mut badges = format!(
        "<span class=\"graph-map-family-pill\" data-family=\"{family}\">{family_label}</span>"
    );
    if m::yes(&pattern["named"]) {
        badges
            .push_str("<span class=\"graph-map-evidence-pill\" data-tone=\"named\">By name</span>");
    }
    if m::yes(&pattern["guessed"]) {
        badges.push_str("<span class=\"graph-map-evidence-pill\" data-tone=\"guessed\">Includes guessed links</span>");
    } else {
        badges.push_str("<span class=\"graph-map-evidence-pill\" data-tone=\"verified\">Verified from code</span>");
    }
    let mut html = format!(
        "<section class=\"graph-map-pattern-brief\" data-family=\"{family}\"><div class=\"graph-map-badges\">{badges}</div><h4>{} <span class=\"graph-map-total\">{}</span></h4>",
        escape(&m::text(pattern, "name")),
        escape(&m::text(&pattern["of"], "name")),
    );
    let tells = m::text(pattern, "tells");
    if !tells.is_empty() {
        html.push_str(&format!(
            "<p class=\"graph-map-pattern-tells\">{}</p>",
            escape(&tells)
        ));
    }
    let told = m::array(&pattern["told"]);
    if !told.is_empty() {
        html.push_str("<ul class=\"graph-map-told-list\">");
        for item in told {
            if let Some(text) = item.as_str().filter(|t| !t.is_empty()) {
                html.push_str(&format!("<li>{}</li>", escape(text)));
            }
        }
        html.push_str("</ul>");
    }
    let frames = m::array(&view["frames"]);
    let nodes = m::array(&view["nodes"]);
    if !frames.is_empty() && !nodes.is_empty() {
        html.push_str("<div class=\"graph-map-role-groups\">");
        for frame in frames {
            let holder_path = m::text(&frame, "path");
            let members: Vec<_> = nodes
                .iter()
                .filter(|node| m::text(node, "holder") == holder_path)
                .collect();
            if members.is_empty() {
                continue;
            }
            let anchor_frame = holder_path == "role:0";
            let (places, drawn) = (
                frame["places"].as_u64().unwrap_or(members.len() as u64),
                frame["drawn"].as_u64().unwrap_or(members.len() as u64),
            );
            let count_text = if drawn < places {
                format!("{} of {}", count(drawn), count(places))
            } else {
                count(places)
            };
            html.push_str(&format!(
                "<div class=\"graph-map-role-group\" data-anchor=\"{anchor_frame}\"><div class=\"graph-map-role-head\"><span>{}</span><span class=\"graph-map-total\">{count_text}</span></div><div class=\"graph-map-role-chips\">",
                escape(&m::text(&frame, "name")),
            ));
            for member in members {
                let start = m::yes(&member["start"]);
                html.push_str(&format!(
                    "<button type=\"button\" class=\"graph-map-part-chip\" data-action=\"graph-map-node\" data-payload=\"{}\" data-family=\"{family}\" data-anchor=\"{start}\" title=\"{}\">{}{}</button>",
                    payload(&member["path"]),
                    escape(&m::text(member, "file")),
                    if start { "★ " } else { "" },
                    escape(&m::text(member, "name")),
                ));
            }
            html.push_str("</div></div>");
        }
        html.push_str("</div>");
    }
    html.push_str("</section>");
    html
}
/// The side panel of one found pattern: the selected part with what it is
/// in the pattern and what joins it to the others, or what to do.
pub(super) fn pattern_side(view: &V) -> String {
    if layered(view) {
        return layers_side(view);
    }
    let focus = &view["focus"];
    if !focus.is_object() {
        return format!(
            "<p class=\"knowledge-note\">A frame holds the parts that are one thing in the pattern. Select a part to see what joins it to the others, and select it again to let go of it. Select a link to see what makes it up.</p>{}",
            pattern_brief(view),
        );
    }
    name_side(
        view,
        focus,
        false,
        &format!(" · {} in this pattern", m::text(focus, "role")),
    )
}
/// Findings of one pattern a list shows before it is asked for all of them.
pub(super) const FIRST: usize = 3;
/// One found pattern as a row that opens its picture. Beside a selected
/// place the row begins with the pattern, in a list of one pattern with the
/// type the finding is named after.
pub(super) fn pattern_row(row: &V, whole: bool, family: Option<&str>) -> String {
    let line = row["line"].as_u64().unwrap_or(0);
    let mut told = m::text(row, "parts");
    for (key, tag) in [("named", "by name"), ("guessed", "guessed")] {
        if m::yes(&row[key]) {
            if !told.is_empty() {
                told.push_str(" · ");
            }
            told.push_str(tag);
        }
    }
    let family_attr = family.map_or_else(String::new, |fam| {
        format!(" data-family=\"{}\"", escape(fam))
    });
    let sort = m::text(row, "sort");
    let sort_attr = if sort.is_empty() {
        String::new()
    } else {
        format!(" data-sort=\"{}\"", escape(&sort))
    };
    format!(
        "<li><button type=\"button\" class=\"graph-map-row\"{family_attr}{sort_attr} data-action=\"graph-map-pattern\" data-payload=\"{}\" title=\"{}{}\"><span class=\"graph-map-row-name\">{}{}<span class=\"graph-map-row-file\">{}</span></span><span class=\"graph-map-row-links\">{}</span></button></li>",
        payload(&row["id"]),
        escape(&m::text(row, "file")),
        if line > 0 {
            format!(":{line}")
        } else {
            String::new()
        },
        if whole {
            format!("{}: ", escape(&m::text(row, "name")))
        } else {
            String::new()
        },
        escape(&m::text(row, "of")),
        escape(base(&m::text(row, "file"))),
        escape(&told),
    )
}
/// The patterns found where the picture is open, below the rest of the side
/// panel: those a selected place is part of, or, with nothing selected, all
/// of them, architecture before design, each pattern with its first few
/// findings. `open` names the patterns whose whole list is shown. Nothing
/// where patterns are not asked for.
pub fn found_patterns(view: &V, open: &[String]) -> String {
    let found = &view["patterns"];
    if !found.is_object() {
        return String::new();
    }
    let focus = &view["focus"];
    if focus.is_object() {
        let rows = m::array(&focus["patterns"]);
        let mut html = format!(
            "<h4>Patterns <span class=\"graph-map-total\">{}</span></h4>",
            count(rows.len() as u64)
        );
        if rows.is_empty() {
            html.push_str("<p class=\"knowledge-note\">No found pattern has a part here.</p>");
            return html;
        }
        html.push_str("<ul class=\"graph-map-rows\">");
        for row in &rows {
            let row_family = match row["sort"].as_str() {
                Some("layers" | "repository" | "events" | "commands") => Some("architecture"),
                Some(_) => Some("design"),
                None => None,
            };
            html.push_str(&pattern_row(row, true, row_family));
        }
        html.push_str("</ul>");
        return html;
    }
    // Beside followed links, a path, shared places or a cycle the side
    // panel is theirs: only a selected place says what it is part of.
    if follows(view) || paths(view) || shares(view) || cycles(view) {
        return String::new();
    }
    let kinds = m::array(&found["kinds"]);
    let mut html = format!(
        "<h4>Patterns found here <span class=\"graph-map-total\">{}</span></h4>",
        count(found["found"].as_u64().unwrap_or(0))
    );
    if kinds.is_empty() {
        html.push_str("<p class=\"knowledge-note\">None. A pattern is told by how types extend, implement, hold, take and make each other, and a few by a name.</p>");
        return html;
    }
    html.push_str("<p class=\"knowledge-note\">A box with ◆ holds a part of one. Select a finding to see its parts.</p>");
    for (family, title) in [("architecture", "Architecture"), ("design", "Design")] {
        let of = kinds
            .iter()
            .filter(|kind| kind["family"] == family)
            .collect::<Vec<_>>();
        if of.is_empty() {
            continue;
        }
        html.push_str(&format!(
            "<section class=\"graph-map-family-group\" data-family=\"{family}\"><p class=\"graph-map-family\">{title}</p>"
        ));
        for kind in of {
            let id = m::text(kind, "id");
            let tells = m::text(kind, "tells");
            let rows = m::array(&kind["instances"]);
            let all = open.contains(&id);
            let (found, more) = (
                kind["found"].as_u64().unwrap_or(0),
                kind["more"].as_u64().unwrap_or(0),
            );
            html.push_str(&format!(
                "<div class=\"graph-map-kind-card\" data-family=\"{family}\" data-kind=\"{}\"><p class=\"graph-map-pattern-name\" title=\"{}\">{} <span class=\"graph-map-total\">{}</span></p>",
                escape(&id),
                escape(&tells),
                escape(&m::text(kind, "name")),
                count(found),
            ));
            if !tells.is_empty() {
                html.push_str(&format!(
                    "<p class=\"graph-map-pattern-tells\">{}</p>",
                    escape(&tells),
                ));
            }
            html.push_str("<ul class=\"graph-map-rows\">");
            for row in rows.iter().take(if all { rows.len() } else { FIRST }) {
                html.push_str(&pattern_row(row, false, Some(family)));
            }
            html.push_str("</ul>");
            if rows.len() > FIRST {
                html.push_str(&format!(
                    "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-pattern-kind\" data-payload=\"{}\" aria-expanded=\"{all}\">{}</button>",
                    payload(&json!(id)),
                    if all {
                        format!("First {FIRST}")
                    } else {
                        format!("All {}", count(rows.len() as u64))
                    },
                ));
            }
            if all && more > 0 {
                html.push_str(&format!(
                    "<p class=\"knowledge-note\">and {} more: open a folder to see those of it.</p>",
                    count(more)
                ));
            }
            html.push_str("</div>");
        }
        html.push_str("</section>");
    }
    html
}
/// What the two lists beside a selected box of a path are called.
pub(super) const WAY: (&str, &str) = ("Next on the way", "Before it on the way");
/// The side panel: what the selected place uses and what uses it.
pub fn side(view: &V) -> String {
    if symbols(view) {
        return file_side(view);
    }
    if patterned(view) {
        return pattern_side(view);
    }
    let focus = &view["focus"];
    let followed = follows(view);
    let pathed = paths(view);
    let shared = shares(view);
    // On a path and in what two places share a box lies on a way.
    let cycle = cycles(view);
    let ways = pathed || shared || cycle;
    if (followed || ways) && m::yes(&focus["symbol"]) {
        return name_side(view, focus, ways, &role(view, focus));
    }
    if cycle && !focus.is_object() {
        if let Some(places) = view["cycle"]["places"].as_array() {
            let mut html = String::from("<ol class=\"graph-map-cycle-places\">");
            for (at, place) in places.iter().enumerate() {
                let guessed = place["guessed"].as_u64().unwrap_or(0);
                let evidence = if guessed == 0 {
                    ""
                } else if place["links"] == guessed {
                    " · incoming link inferred"
                } else {
                    " · includes inferred links"
                };
                html.push_str(&format!(
                    "<li>{}{evidence}{}",
                    escape(&m::text(place, "path")),
                    if m::yes(&place["added"]) {
                        " · incoming link added in PR"
                    } else {
                        ""
                    },
                ));
                if at_base(view) && at > 0 {
                    html.push_str(&format!(
                        " <button type=\"button\" class=\"button compact\" data-action=\"graph-map-link\" data-payload=\"{}\">Inspect link</button>",
                        payload(&json!({"from": places[at - 1]["path"], "to": place["path"]})),
                    ));
                }
                html.push_str("</li>");
            }
            html.push_str("</ol>");
            return html;
        }
        return String::new();
    }
    if !focus.is_object() && pathed {
        // A path that draws nothing leaves its two ends to go on from.
        if !m::yes(&view["path"]["found"]) {
            return apart(view);
        }
        return "<p class=\"knowledge-note\">Select a box to see what it leads to on the way and what leads to it. Select a link to see what makes it up. The path above the picture leads back; beside it, The other way asks for the ways from the second place to the first, and What they share for what both use and what uses both.</p>".to_owned();
    }
    if !focus.is_object() && shared {
        // So does a picture of what two places share with nothing in it.
        if m::array(&view["nodes"]).is_empty() {
            return apart(view);
        }
        return "<p class=\"knowledge-note\">Select a shared place to highlight both paths. Select a link to see what makes it up.</p>".to_owned();
    }
    if !focus.is_object() {
        return if followed {
            "<p class=\"knowledge-note\">Select a box to see what it uses and what uses it. + at the right corner of a box shows more of what it uses, + at the left corner more of what uses it, and − folds them back. Select a link to see what makes it up.</p>"
        } else if changes(view) {
            // A run says first what it left unopened of what uses its change.
            return format!(
                "<p class=\"knowledge-note\">Select a file to see what it uses and what uses it. The highlighted ones changed; the others use them or are used by them. Select a file again to open it to its classes, methods and functions. Select a link to see what makes it up.</p>{}",
                unopened(view)
            );
        } else {
            "<p class=\"knowledge-note\">Select a folder or a file to see what it uses and what uses it. Select it again to open it: a folder to what it holds, a file to its classes, methods and functions. + at the corner of a folder opens it here, among the others. Select a link to see what makes it up.</p>"
        }
        .to_owned();
    }
    let folder = m::yes(&focus["folder"]);
    let changed = focus["changed"].as_u64().unwrap_or(0);
    let held = focus["symbols"].as_u64().unwrap_or(0);
    // Beside a change, a file's links are the ones it has with the change.
    let beside = changes(view) && changed == 0;
    // What the place is, and what it is to the picture: that may name the
    // two places that share it, so it is text like the name above it.
    let told = if changes(view) {
        match (changed, focus["status"].as_str(), focus["role"].as_str()) {
            (0, _, role) => format!(
                "{} · {}",
                if m::yes(&focus["altered"]) {
                    "Changed too, outside the open folder"
                } else {
                    "Not changed"
                },
                match role {
                    Some("affected") => "uses the change",
                    Some("needed") => "used by the change",
                    _ => "uses the change and is used by it",
                }
            ),
            (_, Some(status), _) => format!("Changed file · {}", happened(status)),
            _ => format!(
                "Folder · {}",
                plural(changed, "changed file", "changed files")
            ),
        }
    } else if followed || ways {
        // Among followed links, on a path and in what two places share
        // a place may be one of a chosen change.
        let mut told = if folder {
            format!(
                "Folder · {}",
                plural(focus["files"].as_u64().unwrap_or(0), "file", "files")
            )
        } else if held > 0 {
            format!("File · {}", plural(held, "symbol", "symbols"))
        } else {
            "File".to_owned()
        };
        match (changed, focus["status"].as_str()) {
            (0, _) => {}
            (_, Some(status)) => told.push_str(&format!(" · {}", happened(status))),
            _ => told.push_str(&format!(
                " · {} inside",
                plural(changed, "changed file", "changed files")
            )),
        }
        told.push_str(&role(view, focus));
        told
    } else if folder {
        format!(
            "Folder · {} · {} inside it",
            plural(focus["files"].as_u64().unwrap_or(0), "file", "files"),
            plural(focus["within"].as_u64().unwrap_or(0), "link", "links")
        )
    } else if held > 0 {
        format!("File · {}", plural(held, "symbol", "symbols"))
    } else {
        "File".to_owned()
    };
    let mut html = format!(
        "<h3>{}</h3><p class=\"graph-map-path\">{}</p><p class=\"knowledge-note\">{}</p>",
        escape(&m::text(focus, "name")),
        escape(&m::text(focus, "path")),
        escape(&told),
    );
    if !at_base(view) && folder {
        html.push_str(&format!(
            "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-open\" data-payload=\"{}\">Open folder</button>",
            payload(&focus["path"])
        ));
        if spreads(view) && m::yes(&focus["inside"]) {
            html.push_str(&format!(
                "<button type=\"button\" class=\"button compact\" data-action=\"graph-map-spread\" data-payload=\"{}\">Open here</button>",
                payload(&focus["path"])
            ));
        }
    } else if !at_base(view) && held > 0 {
        html.push_str(&open_file(&focus["path"]));
    }
    // The place the links are followed from is followed already.
    if !at_base(view) {
        if ways || !m::yes(&focus["start"]) {
            html.push_str(&follow_links(&focus["path"]));
        }
        html.push_str(&path_to(focus));
    }
    let (uses, used) = if ways {
        WAY
    } else if beside {
        ("Uses in the change", "Used by the change")
    } else {
        ("Uses", "Used by")
    };
    // Each row is a link of the selected place, and can be looked into.
    let place = &focus["path"];
    html.push_str(&rows(
        uses,
        &focus["uses"],
        &focus["uses_more"],
        Some((place, true)),
    ));
    html.push_str(&rows(
        used,
        &focus["used_by"],
        &focus["used_by_more"],
        Some((place, false)),
    ));
    html.push_str(&kinds(focus));
    html
}
pub(super) fn crumbs(view: &V) -> String {
    let crumbs = m::array(&view["crumbs"]);
    let last = crumbs.len().saturating_sub(1);
    let mut html = crumbs
        .iter()
        .enumerate()
        .map(|(index, crumb)| {
            if index == last {
                format!(
                    "<span class=\"graph-map-crumb\" aria-current=\"location\">{}{}</span>",
                    escape(&m::text(crumb, "name")),
                    if follows(view) {
                        ": links"
                    } else if paths(view) {
                        ": path"
                    } else if cycles(view) {
                        ": cycle"
                    } else if !shares(view) {
                        ""
                    } else if above(view) {
                        ": what uses both"
                    } else {
                        ": what both use"
                    }
                )
            } else if at_base(view) {
                format!("<span class=\"graph-map-crumb\">{}</span><span aria-hidden=\"true\">/</span>", escape(&m::text(crumb, "name")))
            } else {
                format!(
                    "<button type=\"button\" class=\"graph-map-crumb\" data-action=\"graph-map-open\" data-payload=\"{}\">{}</button><span aria-hidden=\"true\">/</span>",
                    payload(&crumb["path"]),
                    escape(&m::text(crumb, "name"))
                )
            }
        })
        .collect::<String>();
    // A path that was found can be asked for the other way round, whatever
    // is selected in it: two places of a cycle lead to each other.
    if paths(view) && m::yes(&view["path"]["found"]) {
        html.push_str("<span aria-hidden=\"true\">·</span><button type=\"button\" class=\"graph-map-crumb\" data-action=\"graph-map-path-swap\">The other way</button>");
    }
    // Two places that lie apart can be asked what they share, whether or
    // not a path joins them, and from there for the other kind of it and
    // for the path again.
    for ask in asks(view) {
        html.push_str("<span aria-hidden=\"true\">·</span>");
        html.push_str(&ask_shared(ask, "graph-map-crumb"));
    }
    html
}
pub(super) fn html(id: &str, markup: &str) {
    if let Some(element) = dom::get(id) {
        element.set_inner_html(markup);
    }
}
pub(super) const LEGEND: &str = "Dotted: against the main direction, closing a cycle (↻).";
pub(super) const FILE_LEGEND: &str = "T type · m method · f function · P property · K const · c case · # heading. A box without a letter is another file, a plain name, or the open file itself for its imports.";
pub(super) const NAME_LEGEND: &str = "T type · m method · f function · P property · K const · c case · # heading. A box without a letter is a plain name, or a file itself for its imports.";
pub(super) const LAYER_LEGEND: &str = "A box is a layer; the layers stand in their order, and each may use those after it. A link to the next layer runs straight, one that passes a layer runs over the row. A link under the row, in red, runs back: against the order.";
/// What the colours and the lines of the map mean, each colour beside a
/// patch of it, then what is particular to the picture: a folder, an opened
/// file, or a change (`change` says a change is chosen).
pub(super) fn legend(view: &V, change: bool) -> String {
    let patch = |side: &str| {
        format!("<span class=\"graph-map-key graph-map-key-{side}\" aria-hidden=\"true\"></span>")
    };
    // The mark of what a run opened, where a run is shown.
    let run = if view["run"].is_object() {
        " ◉: opened by the run; on a folder, how many of its files."
    } else {
        ""
    };
    let particular = match (symbols(view), change) {
        (true, true) => format!("{FILE_LEGEND} Highlighted: touched by the change.{run}"),
        (true, false) => FILE_LEGEND.to_owned(),
        _ if changes(view) => {
            format!("Highlighted: changed. The rest use the change or are used by it.{run}")
        }
        _ if layered(view) => {
            let mut text = if m::yes(&view["pattern"]["declared"]) {
                "A box is a declared layer. A red arrow is a dependency forbidden by the project rules.".to_owned()
            } else {
                LAYER_LEGEND.to_owned()
            };
            if m::array(&view["edges"])
                .iter()
                .any(|edge| edge["added"].is_u64())
            {
                text.push_str(" The number beside one counts the links this change added.");
            }
            text
        }
        _ if patterned(view) => format!(
            "A frame holds the parts that are one thing in the pattern; the ringed box is the type it is named after. {NAME_LEGEND}"
        ),
        _ if cycles(view) => {
            let mut text = format!("{LEGEND} The ringed box is where the cycle starts and ends.");
            if m::array(&view["edges"])
                .iter()
                .any(|edge| m::yes(&edge["added"]))
            {
                text.push_str(" Green links were added in this PR.");
            }
            if names(view) {
                text.push(' ');
                text.push_str(NAME_LEGEND);
            }
            text
        }
        // What two places share can use each other, which a path cannot.
        _ if paths(view) || shares(view) => format!(
            "{}{}{}",
            if !shares(view) {
                "The ringed boxes are the two ends of the path.".to_owned()
            } else if above(view) {
                format!(
                    "{LEGEND} The ringed boxes are the two places. A name in bold is what uses both; a box with a plain name lies on its way to them."
                )
            } else {
                format!(
                    "{LEGEND} The ringed boxes are the two places. A name in bold is what both use; a box with a plain name lies on the way to it."
                )
            },
            if names(view) {
                format!(" {NAME_LEGEND}")
            } else {
                String::new()
            },
            match (change, names(view)) {
                (true, true) => " Highlighted: touched by the change.",
                (true, false) => " Highlighted: changed.",
                _ => "",
            }
        ),
        _ if names(view) => format!(
            "{LEGEND} + and − at the corners of a box unfold and fold what it uses, on the right, and what uses it, on the left. {NAME_LEGEND}{}",
            if change {
                " Highlighted: touched by the change."
            } else {
                ""
            }
        ),
        _ if follows(view) => format!(
            "{LEGEND} + and − at the corners of a box unfold and fold what it uses, on the right, and what uses it, on the left.{}",
            if change { " Highlighted: changed." } else { "" }
        ),
        _ if !m::array(&view["frames"]).is_empty() => {
            format!("{LEGEND} A frame is a folder opened here: − at its corner closes it.")
        }
        _ => LEGEND.to_owned(),
    };
    let diff_key = if change || changes(view) {
        format!(
            " Diff: {} Added/Created, {} Modified, {} Deleted, {} Renamed, {} Caller risk, {} Touched, {} Added / {} Removed edges.",
            patch("diff-added"),
            patch("diff-modified"),
            patch("diff-removed"),
            patch("diff-renamed"),
            patch("diff-risk"),
            patch("diff-touched"),
            patch("diff-added"),
            patch("diff-removed"),
        )
    } else {
        String::new()
    };
    format!(
        "An arrow runs from what uses {} to what is used {}, and a box is tinted by which it does more here. Its head and its dashes tell its kind, as the key of the links shows. Hollow: only guessed by name. Thicker: more links.{diff_key} {particular}{}",
        patch("uses"),
        patch("used"),
        if view["patterns"].is_object() {
            " ◆: a part of a found pattern, or a place that holds one."
        } else {
            ""
        },
    )
}
