//! Interactive visual graph, branch metro DAG, and 3-way merge diff renderer
//! for Omnigraph context sharing across Harness sessions.
//!
//! Pure HTML + SVG string generation (no `web_sys` dependency) so both the
//! `wasm32-unknown-unknown` UI bundle and native unit tests share one deterministic
//! rendering pipeline. All colors come from CSS tokens and classes defined in
//! `harness/web/app.css` (zero inline color hexes), and every status, kind,
//! relation, and diff marker pairs iconography or geometry with explicit text
//! labels for WCAG 1.4.1 compliance.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const SESSION_W: f64 = 216.0;
const SESSION_H: f64 = 78.0;
const ATOM_W: f64 = 240.0;
const ATOM_H: f64 = 96.0;
const FILE_W: f64 = 208.0;
const FILE_H: f64 = 58.0;

const MARGIN_X: f64 = 48.0;
const MARGIN_Y: f64 = 58.0;
const TRACK_GAP_X: f64 = 128.0;
const ATOM_COL_GAP_X: f64 = 44.0;
const NODE_GAP_X: f64 = 22.0;
const NODE_GAP_Y: f64 = 20.0;
const CLUSTER_EXTRA_GAP_Y: f64 = 12.0;
const MIN_PLANE_W: f64 = 760.0;
const MIN_PLANE_H: f64 = 460.0;

const RADAR_MAX_W: f64 = 176.0;
const RADAR_MAX_H: f64 = 116.0;
const RADAR_MIN_SIDE: f64 = 24.0;
const NOMINAL_VIEWPORT_W: f64 = 920.0;
const NOMINAL_VIEWPORT_H: f64 = 580.0;

const DAG_ROW_H: f64 = 56.0;
const DAG_LANE_W: f64 = 24.0;
const DAG_RAIL_PAD_X: f64 = 20.0;

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasViewport {
    pub pan_x: f64,
    pub pan_y: f64,
    pub scale: f64,
}

impl Default for CanvasViewport {
    fn default() -> Self {
        Self {
            pan_x: 0.0,
            pan_y: 0.0,
            scale: 1.0,
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub struct ConstellationFilter {
    pub query: String,
    pub kind: String, // "" or finding|decision|warning|contract|insight|artifact|question
    pub branch: String,
    pub show_files: bool,
    pub show_sessions: bool,
    pub selected_id: Option<String>,
    pub highlighted_ids: BTreeSet<String>,
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RenderedCanvas {
    pub html: String,
    pub radar_html: String,
    pub width: f64,
    pub height: f64,
    pub node_count: usize,
    pub edge_count: usize,
    pub positions: BTreeMap<String, (f64, f64)>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum NodeKindGroup {
    Session,
    Atom,
    File,
}

impl NodeKindGroup {
    fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Atom => "atom",
            Self::File => "file",
        }
    }

    fn size(self) -> (f64, f64) {
        match self {
            Self::Session => (SESSION_W, SESSION_H),
            Self::Atom => (ATOM_W, ATOM_H),
            Self::File => (FILE_W, FILE_H),
        }
    }
}

struct KindVisual {
    key: &'static str,
    label: &'static str,
    rank: usize,
    svg_body: &'static str,
}

const ATOM_KINDS: [KindVisual; 7] = [
    KindVisual {
        key: "finding",
        label: "Finding",
        rank: 0,
        svg_body: "<circle cx=\"7\" cy=\"7\" r=\"4.3\"/><path d=\"M10.4 10.4 L14 14\"/>",
    },
    KindVisual {
        key: "decision",
        label: "Decision",
        rank: 1,
        svg_body: "<path d=\"M8 1.5 L14.5 8 L8 14.5 L1.5 8 Z\"/><path d=\"M5.5 8.1 L7.2 9.8 L10.8 6.2\"/>",
    },
    KindVisual {
        key: "warning",
        label: "Warning",
        rank: 2,
        svg_body: "<path d=\"M8 2 L14.5 13.5 H1.5 Z\"/><path d=\"M8 6 V9.2\"/><circle cx=\"8\" cy=\"11.4\" r=\"0.7\" fill=\"currentColor\" stroke=\"none\"/>",
    },
    KindVisual {
        key: "contract",
        label: "Contract",
        rank: 3,
        svg_body: "<path d=\"M8 1.5 L13.5 3.8 V8 C13.5 11.2 11.1 13.6 8 14.5 C4.9 13.6 2.5 11.2 2.5 8 V3.8 Z\"/><path d=\"M5.5 7.4 H10.5 M5.5 10 H9\"/>",
    },
    KindVisual {
        key: "insight",
        label: "Insight",
        rank: 4,
        svg_body: "<path d=\"M8 1.5 L9.6 6.4 L14.5 8 L9.6 9.6 L8 14.5 L6.4 9.6 L1.5 8 L6.4 6.4 Z\"/>",
    },
    KindVisual {
        key: "artifact",
        label: "Artifact",
        rank: 5,
        svg_body: "<path d=\"M8 1.8 L13.5 4.8 V11.2 L8 14.2 L2.5 11.2 V4.8 Z\"/><path d=\"M2.5 4.8 L8 7.8 L13.5 4.8 M8 7.8 V14.2\"/>",
    },
    KindVisual {
        key: "question",
        label: "Question",
        rank: 6,
        svg_body: "<circle cx=\"8\" cy=\"8\" r=\"6.2\"/><path d=\"M6.2 6.1 A1.9 1.9 0 1 1 8.9 7.8 C8.2 8.2 8 8.7 8 9.5\"/><circle cx=\"8\" cy=\"11.7\" r=\"0.7\" fill=\"currentColor\" stroke=\"none\"/>",
    },
];

const SESSION_SVG_BODY: &str =
    "<rect x=\"1.8\" y=\"2.5\" width=\"12.4\" height=\"11\" rx=\"2.2\"/><path d=\"M4.8 6.2 L7.3 8.1 L4.8 10 M8.4 10 H11.2\"/>";
const FILE_SVG_BODY: &str =
    "<path d=\"M3.5 1.8 H9.5 L12.5 4.8 V14.2 H3.5 Z\"/><path d=\"M9.5 1.8 V4.8 H12.5 M5.5 8 H10.5 M5.5 10.8 H9\"/>";

struct EdgeVisualSpec {
    key: &'static str,
    label: &'static str,
    dash_array: Option<&'static str>,
    marker_body: &'static str,
}

const EDGE_SPECS: [EdgeVisualSpec; 12] = [
    EdgeVisualSpec {
        key: "published_by",
        label: "published by",
        dash_array: None,
        marker_body: "<path d=\"M0 1 L8 4 L0 7 Z\"/>",
    },
    EdgeVisualSpec {
        key: "consumed_by",
        label: "consumed by",
        dash_array: Some("7 3"),
        marker_body: "<path d=\"M0.4 0.8 L7.8 4 L0.4 7.2 L2.5 4 Z\"/>",
    },
    EdgeVisualSpec {
        key: "touches_file",
        label: "touches",
        dash_array: Some("4 3"),
        marker_body: "<path d=\"M0.8 1 L7.6 4 L0.8 7 Z\"/>",
    },
    EdgeVisualSpec {
        key: "supports",
        label: "supports",
        dash_array: None,
        marker_body: "<path d=\"M0.5 4 L4 1 L7.5 4 L4 7 Z\"/>",
    },
    EdgeVisualSpec {
        key: "contradicts",
        label: "contradicts",
        dash_array: Some("6 3 2 3"),
        marker_body: "<path d=\"M0.8 1 L7.4 4 L0.8 7 M5.8 1 V7\"/>",
    },
    EdgeVisualSpec {
        key: "supersedes",
        label: "supersedes",
        dash_array: Some("9 3"),
        marker_body: "<path d=\"M1.2 1 L7.6 4 L1.2 7 Z M0.4 1.8 V6.2\"/>",
    },
    EdgeVisualSpec {
        key: "depends_on",
        label: "depends on",
        dash_array: Some("2 3"),
        marker_body: "<path d=\"M1 1 L7.4 4 L1 7\"/>",
    },
    EdgeVisualSpec {
        key: "refines",
        label: "refines",
        dash_array: Some("8 2 2 2"),
        marker_body: "<path d=\"M0.8 4 L4 1.2 L7.4 4 L4 6.8 Z M4 2.7 V5.3\"/>",
    },
    EdgeVisualSpec {
        key: "relates_to",
        label: "relates to",
        dash_array: Some("5 3"),
        marker_body: "<path d=\"M1 4 L4 1.2 L7 4 L4 6.8 Z\"/>",
    },
    EdgeVisualSpec {
        key: "mentions_symbol",
        label: "mentions",
        dash_array: Some("3 3"),
        marker_body: "<path d=\"M1 1.2 L7.2 4 L1 6.8\"/>",
    },
    EdgeVisualSpec {
        key: "derived_from",
        label: "derived from",
        dash_array: Some("8 3 2 3"),
        marker_body: "<path d=\"M0.8 1 L7.6 4 L0.8 7 Z\"/>",
    },
    EdgeVisualSpec {
        key: "session_follows",
        label: "follows",
        dash_array: None,
        marker_body: "<path d=\"M0 1 L8 4 L0 7 Z\"/>",
    },
];

#[derive(Clone)]
struct CanvasNode {
    id: String,
    group: NodeKindGroup,
    kind: String,
    title: String,
    subtitle: String,
    branch: String,
    shared_space: String,
    session_id: String,
    status: String,
    provider: String,
    confidence: Option<u8>,
    pinned: bool,
    pin_priority: i64,
    tags: Vec<String>,
    touch_count: usize,
    search_blob: String,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[derive(Clone)]
struct CanvasEdge {
    from: String,
    to: String,
    relation: String,
    subkind: String,
    arrow_key: String,
    label: String,
}

#[allow(dead_code)]
pub fn render_constellation(
    graph: &Value,
    filter: &ConstellationFilter,
    viewport: &CanvasViewport,
) -> RenderedCanvas {
    let vp = sanitize_viewport(viewport);
    let (mut nodes, mut edges, aliases) = extract_graph_items(graph);

    // Resolve edge endpoints onto canonical node IDs and compute touch counts.
    let mut resolved_edges = Vec::with_capacity(edges.len());
    let mut touch_counts: BTreeMap<String, usize> = BTreeMap::new();
    for mut edge in edges.drain(..) {
        if let Some(canon_from) = aliases.get(&edge.from) {
            edge.from = canon_from.clone();
        }
        if let Some(canon_to) = aliases.get(&edge.to) {
            edge.to = canon_to.clone();
        }
        *touch_counts.entry(edge.to.clone()).or_default() += 1;
        resolved_edges.push(edge);
    }
    for node in &mut nodes {
        if node.group == NodeKindGroup::File && node.touch_count == 0 {
            node.touch_count = touch_counts.get(&node.id).copied().unwrap_or(0);
        }
    }

    let active_branch = str_field(graph, &["active_branch"]);
    let (kept_nodes, kept_edges) =
        apply_constellation_filter(nodes, resolved_edges, filter, &active_branch);
    if kept_nodes.is_empty() {
        let html = render_constellation_empty(filter, &vp);
        return RenderedCanvas {
            html,
            radar_html: String::new(),
            width: MIN_PLANE_W,
            height: 400.0,
            node_count: 0,
            edge_count: 0,
            positions: BTreeMap::new(),
        };
    }

    let (mut placed_nodes, width, height) = layout_constellation(kept_nodes, &kept_edges);
    // Deterministic render order by (group, y, x, id)
    placed_nodes.sort_by(|a, b| {
        a.group
            .cmp(&b.group)
            .then_with(|| a.y.total_cmp(&b.y))
            .then_with(|| a.x.total_cmp(&b.x))
            .then_with(|| a.id.cmp(&b.id))
    });

    let mut positions = BTreeMap::new();
    let mut node_by_id: BTreeMap<String, usize> = BTreeMap::new();
    for (idx, n) in placed_nodes.iter().enumerate() {
        positions.insert(n.id.clone(), (n.x + n.w * 0.5, n.y + n.h * 0.5));
        node_by_id.insert(n.id.clone(), idx);
    }

    let selected_id = filter
        .selected_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty() && node_by_id.contains_key(*id));
    let mut neighbor_ids: BTreeSet<String> = BTreeSet::new();
    if let Some(sel) = selected_id {
        for edge in &kept_edges {
            if edge.from == sel {
                neighbor_ids.insert(edge.to.clone());
            } else if edge.to == sel {
                neighbor_ids.insert(edge.from.clone());
            }
        }
    }
    let has_selection_focus = selected_id.is_some();
    let has_highlight_focus = !filter.highlighted_ids.is_empty();

    // Group parallel edges between the same unordered node pair so curves separate cleanly.
    let mut pair_totals: BTreeMap<(String, String), usize> = BTreeMap::new();
    for edge in &kept_edges {
        let key = ordered_pair(&edge.from, &edge.to);
        *pair_totals.entry(key).or_default() += 1;
    }
    let mut pair_seen: BTreeMap<(String, String), usize> = BTreeMap::new();

    struct RoutedEdge {
        p0: (f64, f64),
        p3: (f64, f64),
        mid: (f64, f64),
        path_d: String,
        selected: bool,
        lit: bool,
        dimmed: bool,
    }

    let mut routed = Vec::with_capacity(kept_edges.len());
    for (idx, edge) in kept_edges.iter().enumerate() {
        let (Some(&u_idx), Some(&v_idx)) = (node_by_id.get(&edge.from), node_by_id.get(&edge.to))
        else {
            continue;
        };
        let u = &placed_nodes[u_idx];
        let v = &placed_nodes[v_idx];
        let pair_key = ordered_pair(&edge.from, &edge.to);
        let slot = *pair_seen.get(&pair_key).unwrap_or(&0);
        pair_seen.insert(pair_key.clone(), slot + 1);
        let total = *pair_totals.get(&pair_key).unwrap_or(&1);

        let curve = route_bezier_ports(u, v, slot, total, idx);
        let mid = bezier_midpoint(&curve);
        let path_d = format!(
            "M {:.1} {:.1} C {:.1} {:.1}, {:.1} {:.1}, {:.1} {:.1}",
            curve.p0.0,
            curve.p0.1,
            curve.p1.0,
            curve.p1.1,
            curve.p2.0,
            curve.p2.1,
            curve.p3.0,
            curve.p3.1
        );
        let (p0, p3) = (curve.p0, curve.p3);

        let is_selected_edge = selected_id.is_some_and(|s| edge.from == s || edge.to == s);
        let is_highlighted_edge = filter.highlighted_ids.contains(&edge.from)
            || filter.highlighted_ids.contains(&edge.to);
        let lit = is_selected_edge
            || (filter.highlighted_ids.contains(&edge.from)
                && filter.highlighted_ids.contains(&edge.to));
        let dimmed = if has_selection_focus {
            !is_selected_edge && !is_highlighted_edge
        } else if has_highlight_focus {
            !is_highlighted_edge
        } else {
            false
        };

        routed.push((
            edge,
            RoutedEdge {
                p0,
                p3,
                mid,
                path_d,
                selected: is_selected_edge,
                lit,
                dimmed,
            },
        ));
    }

    let mut html = format!(
        "<div class=\"graph-map-plane omni-plane\" style=\"--graph-pan-x:{:.1}px;--graph-pan-y:{:.1}px;--graph-scale:{:.4};width:{:.0}px;height:{:.0}px\" data-node-count=\"{}\" data-edge-count=\"{}\">",
        vp.pan_x,
        vp.pan_y,
        vp.scale,
        width,
        height,
        placed_nodes.len(),
        routed.len()
    );

    // SVG plane (orbital guides, arrow markers, gradient flows, Bezier paths, mid-edge labels)
    html.push_str(&format!(
        "<svg class=\"omni-svg\" width=\"{:.0}\" height=\"{:.0}\" viewBox=\"0 0 {:.0} {:.0}\" aria-hidden=\"true\"><defs>",
        width, height, width, height
    ));
    html.push_str(&render_edge_marker_defs());
    for (idx, (edge, r)) in routed.iter().enumerate() {
        let subkind_attr = if edge.subkind.is_empty() {
            ""
        } else {
            edge.subkind.as_str()
        };
        html.push_str(&format!(
            "<linearGradient id=\"omni-flow-{idx}\" class=\"omni-flow omni-flow-{}\" data-rel=\"{}\" data-subkind=\"{}\" gradientUnits=\"userSpaceOnUse\" x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\"><stop offset=\"0\" class=\"omni-flow-from\"/><stop offset=\"0.5\" class=\"omni-flow-mid\"/><stop offset=\"1\" class=\"omni-flow-to\"/></linearGradient>",
            escape_attr_token(&edge.arrow_key),
            escape_attr_token(&edge.relation),
            escape_attr_token(subkind_attr),
            r.p0.0,
            r.p0.1,
            r.p3.0,
            r.p3.1,
        ));
    }
    html.push_str("</defs>");

    html.push_str(&render_orbital_guides(&placed_nodes, width, height));

    for (idx, (edge, r)) in routed.iter().enumerate() {
        let spec = edge_spec(&edge.arrow_key);
        let dash_attr = spec
            .dash_array
            .map(|d| format!(" stroke-dasharray=\"{d}\""))
            .unwrap_or_default();
        let subkind_attr = if edge.subkind.is_empty() {
            edge.relation.as_str()
        } else {
            edge.subkind.as_str()
        };
        html.push_str(&format!(
            "<path class=\"omni-edge omni-edge-{}\" d=\"{}\" stroke=\"url(#omni-flow-{idx})\" marker-end=\"url(#omni-arrow-{})\" data-rel=\"{}\" data-subkind=\"{}\" data-from=\"{}\" data-to=\"{}\" data-selected=\"{}\" data-lit=\"{}\" data-dimmed=\"{}\"{dash_attr}/>",
            escape_attr_token(&edge.arrow_key),
            r.path_d,
            escape_attr_token(&edge.arrow_key),
            escape_attr_token(&edge.relation),
            escape_attr_token(subkind_attr),
            esc(&edge.from),
            esc(&edge.to),
            bool_str(r.selected),
            bool_str(r.lit),
            bool_str(r.dimmed),
        ));
        html.push_str(&format!(
            "<circle class=\"omni-edge-origin\" cx=\"{:.1}\" cy=\"{:.1}\" r=\"2.6\" data-rel=\"{}\" data-lit=\"{}\" data-dimmed=\"{}\"/>",
            r.p0.0,
            r.p0.1,
            escape_attr_token(&edge.arrow_key),
            bool_str(r.lit),
            bool_str(r.dimmed),
        ));
    }

    // Mid-edge pill labels when zoomed in or connected to selected/highlighted node.
    let zoomed_in = vp.scale >= 0.95;
    for (edge, r) in &routed {
        if !(zoomed_in || r.selected || r.lit) {
            continue;
        }
        let pill_w = (edge.label.chars().count() as f64 * 6.2 + 16.0).max(52.0);
        html.push_str(&format!(
            "<g class=\"omni-edge-label\" data-rel=\"{}\" data-subkind=\"{}\" data-lit=\"{}\" data-dimmed=\"{}\"><rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"18\" rx=\"9\"/><text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\">{}</text></g>",
            escape_attr_token(&edge.arrow_key),
            escape_attr_token(if edge.subkind.is_empty() {
                &edge.relation
            } else {
                &edge.subkind
            }),
            bool_str(r.lit || r.selected),
            bool_str(r.dimmed),
            r.mid.0 - pill_w * 0.5,
            r.mid.1 - 9.0,
            pill_w,
            r.mid.0,
            r.mid.1 + 3.5,
            esc(&edge.label),
        ));
    }

    html.push_str("</svg>");

    // Interactive HTML node cards
    for node in &placed_nodes {
        let is_selected = selected_id == Some(node.id.as_str());
        let is_neighbor = !is_selected && neighbor_ids.contains(&node.id);
        let is_highlighted = filter.highlighted_ids.contains(&node.id);
        let is_dimmed = if has_selection_focus {
            !is_selected && !is_neighbor && !is_highlighted
        } else if has_highlight_focus {
            !is_highlighted
        } else {
            false
        };
        html.push_str(&render_constellation_node(
            node,
            is_selected,
            is_neighbor,
            is_highlighted,
            is_dimmed,
        ));
    }

    html.push_str("</div>");

    let radar_html = render_constellation_radar(
        &placed_nodes,
        width,
        height,
        &vp,
        selected_id,
        &filter.highlighted_ids,
    );

    RenderedCanvas {
        html,
        radar_html,
        width,
        height,
        node_count: placed_nodes.len(),
        edge_count: routed.len(),
        positions,
    }
}

#[allow(dead_code)]
pub fn render_branch_dag(
    graph: &Value,
    active_branch: &str,
    active_snapshot: Option<&str>,
) -> String {
    let active_branch = if active_branch.trim().is_empty() {
        str_field(graph, &["active_branch"]).if_empty("main")
    } else {
        active_branch.trim().to_owned()
    };
    let active_snapshot_clean = active_snapshot
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    let (branches, commits) = extract_dag_data(graph, &active_branch);

    // Build deterministic lane assignment: lane 0 is always "main".
    let mut lane_of: BTreeMap<String, usize> = BTreeMap::new();
    let mut lane_names: Vec<String> = Vec::new();
    let mut assign_lane = |name: &str| {
        let trimmed = name.trim();
        if trimmed.is_empty() || lane_of.contains_key(trimmed) {
            return;
        }
        let idx = lane_names.len();
        lane_of.insert(trimmed.to_owned(), idx);
        lane_names.push(trimmed.to_owned());
    };
    assign_lane("main");
    if active_branch != "main" {
        assign_lane(&active_branch);
    }
    for br in &branches {
        assign_lane(&br.name);
    }
    for cm in &commits {
        assign_lane(&cm.branch);
    }

    let lane_count = lane_names.len().max(1);
    let rail_w = DAG_RAIL_PAD_X * 2.0 + (lane_count.saturating_sub(1) as f64) * DAG_LANE_W + 8.0;

    let mut out = format!(
        "<div class=\"omni-dag\" data-active-branch=\"{}\" data-active-snapshot=\"{}\" data-lane-count=\"{lane_count}\">",
        esc(&active_branch),
        esc(active_snapshot_clean.as_deref().unwrap_or("")),
    );

    // Branch header rail chips with quick actions
    out.push_str("<div class=\"omni-dag-branches\" role=\"list\" aria-label=\"Context branches\">");
    for (lane, name) in lane_names.iter().enumerate() {
        let info = branches.iter().find(|b| b.name == *name);
        let is_active = *name == active_branch;
        let is_default = name == "main" || info.is_some_and(|b| b.is_default);
        let ahead = info.map_or(0, |b| b.ahead_count);
        let head_id = info
            .map(|b| b.head_commit.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                commits
                    .iter()
                    .find(|c| c.branch == *name)
                    .map(|c| c.id.as_str())
            })
            .unwrap_or("");

        out.push_str(&format!(
            "<div class=\"omni-dag-branch-chip\" role=\"listitem\" data-branch=\"{}\" data-lane=\"{lane}\" data-lane-tone=\"{}\" data-active=\"{}\">",
            esc(name),
            lane % 6,
            bool_str(is_active),
        ));
        out.push_str(&format!(
            "<span class=\"omni-dag-branch-swatch\" data-lane-tone=\"{}\" aria-hidden=\"true\"></span>",
            lane % 6
        ));
        out.push_str(&format!(
            "<span class=\"omni-dag-branch-name\">{}</span>",
            esc(name)
        ));
        if is_default {
            out.push_str("<span class=\"omni-dag-badge\" data-kind=\"default\">Default</span>");
        }
        if is_active {
            out.push_str("<span class=\"omni-dag-badge\" data-kind=\"active\">Active</span>");
        }
        if ahead > 0 && name != "main" {
            out.push_str(&format!(
                "<span class=\"omni-dag-badge\" data-kind=\"ahead\">+{ahead}</span>"
            ));
        }
        if !head_id.is_empty() {
            out.push_str(&format!(
                "<code class=\"omni-dag-branch-head\">{}</code>",
                esc(&short_hash(head_id))
            ));
        }
        out.push_str("<span class=\"omni-dag-branch-actions\">");
        if !is_active {
            out.push_str(&format!(
                "<button type=\"button\" class=\"omni-branch-action\" data-action=\"omnigraph-switch-branch\" data-branch=\"{}\">Switch</button>",
                esc(name)
            ));
        }
        if name != "main" {
            out.push_str(&format!(
                "<button type=\"button\" class=\"omni-branch-action\" data-action=\"omnigraph-preview-diff\" data-branch=\"{}\">Preview Diff</button>",
                esc(name)
            ));
        }
        out.push_str("</span></div>");
    }
    out.push_str("</div>");

    if commits.is_empty() {
        out.push_str("<div class=\"omni-dag-empty\"><p>No commits recorded on these branches yet. Publish context or run a session to build history.</p></div></div>");
        return out;
    }

    let total_h = (commits.len() as f64) * DAG_ROW_H;
    let mut row_of_commit: BTreeMap<String, usize> = BTreeMap::new();
    for (idx, cm) in commits.iter().enumerate() {
        if !cm.id.is_empty() {
            row_of_commit.insert(cm.id.clone(), idx);
        }
    }

    let lane_x = |lane: usize| -> f64 { DAG_RAIL_PAD_X + (lane as f64) * DAG_LANE_W };
    let row_y = |row: usize| -> f64 { (row as f64) * DAG_ROW_H + DAG_ROW_H * 0.5 };

    out.push_str("<div class=\"omni-dag-timeline\">");
    out.push_str(&format!(
        "<svg class=\"omni-dag-svg\" width=\"{:.0}\" height=\"{:.0}\" viewBox=\"0 0 {:.0} {:.0}\" aria-hidden=\"true\">",
        rail_w, total_h, rail_w, total_h
    ));

    // Vertical swimlane rail guides
    for (lane, name) in lane_names.iter().enumerate() {
        let lx = lane_x(lane);
        out.push_str(&format!(
            "<line class=\"omni-dag-lane-guide\" x1=\"{lx:.1}\" y1=\"0\" x2=\"{lx:.1}\" y2=\"{total_h:.1}\" data-lane=\"{lane}\" data-lane-tone=\"{}\" data-branch=\"{}\"/>",
            lane % 6,
            esc(name),
        ));
    }

    // Commit-to-parent and fork/merge Bezier rails
    for (row, cm) in commits.iter().enumerate() {
        let c_lane = *lane_of.get(&cm.branch).unwrap_or(&0);
        let cx = lane_x(c_lane);
        let cy = row_y(row);

        let mut drew_parent = false;
        for (p_idx, pid) in cm.parents.iter().enumerate() {
            if let Some(&prow) = row_of_commit.get(pid) {
                drew_parent = true;
                let p_branch = commits[prow].branch.as_str();
                let p_lane = *lane_of.get(p_branch).unwrap_or(&0);
                let px = lane_x(p_lane);
                let py = row_y(prow);
                let kind = if p_lane == c_lane {
                    "straight"
                } else if p_idx > 0 || cm.parents.len() > 1 && p_lane != c_lane {
                    "merge"
                } else {
                    "fork"
                };
                let tone = if kind == "merge" {
                    p_lane % 6
                } else {
                    c_lane % 6
                };
                let mid_y = (cy + py) * 0.5;
                out.push_str(&format!(
                    "<path class=\"omni-dag-rail\" d=\"M {cx:.1} {cy:.1} C {cx:.1} {mid_y:.1}, {px:.1} {mid_y:.1}, {px:.1} {py:.1}\" data-kind=\"{kind}\" data-lane=\"{c_lane}\" data-lane-tone=\"{tone}\"/>"
                ));
            }
        }

        // If a merge commit names `merged_from` without listing a second parent row, draw a merge curve from that branch lane.
        if !cm.merged_from.is_empty() && cm.parents.len() < 2 {
            if let Some(&src_lane) = lane_of.get(&cm.merged_from) {
                if src_lane != c_lane {
                    let sx = lane_x(src_lane);
                    let sy = (cy + DAG_ROW_H * 0.85).min(total_h - 6.0);
                    let mid_y = (cy + sy) * 0.5;
                    out.push_str(&format!(
                        "<path class=\"omni-dag-rail\" d=\"M {cx:.1} {cy:.1} C {cx:.1} {mid_y:.1}, {sx:.1} {mid_y:.1}, {sx:.1} {sy:.1}\" data-kind=\"merge\" data-lane=\"{src_lane}\" data-lane-tone=\"{}\"/>",
                        src_lane % 6
                    ));
                }
            }
        }

        // If this is the oldest commit on a non-main branch without a visible parent commit, show its fork curve from parent branch (or lane 0).
        if !drew_parent && c_lane > 0 {
            let parent_branch = branches
                .iter()
                .find(|b| b.name == cm.branch)
                .map(|b| b.parent_branch.as_str())
                .filter(|p| !p.is_empty())
                .unwrap_or("main");
            let p_lane = *lane_of.get(parent_branch).unwrap_or(&0);
            if p_lane != c_lane {
                let px = lane_x(p_lane);
                let py = (cy + DAG_ROW_H * 0.75).min(total_h - 4.0);
                let mid_y = (cy + py) * 0.5;
                out.push_str(&format!(
                    "<path class=\"omni-dag-rail\" d=\"M {cx:.1} {cy:.1} C {cx:.1} {mid_y:.1}, {px:.1} {mid_y:.1}, {px:.1} {py:.1}\" data-kind=\"fork\" data-lane=\"{c_lane}\" data-lane-tone=\"{}\"/>",
                    c_lane % 6
                ));
            }
        }
    }

    // Commit dots on top of rails
    for (row, cm) in commits.iter().enumerate() {
        let c_lane = *lane_of.get(&cm.branch).unwrap_or(&0);
        let cx = lane_x(c_lane);
        let cy = row_y(row);
        let is_snapshot = active_snapshot_clean.as_deref() == Some(cm.id.as_str());
        let is_active_branch = cm.branch == active_branch;
        let is_merge = cm.parents.len() > 1 || !cm.merged_from.is_empty();
        let is_head = branches
            .iter()
            .any(|b| b.name == cm.branch && b.head_commit == cm.id);

        if is_snapshot || is_head {
            out.push_str(&format!(
                "<circle class=\"omni-dag-dot-halo\" cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"9.5\" data-snapshot=\"{}\" data-head=\"{}\" data-lane-tone=\"{}\"/>",
                bool_str(is_snapshot),
                bool_str(is_head),
                c_lane % 6,
            ));
        }
        out.push_str(&format!(
            "<circle class=\"omni-dag-dot\" cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"{}\" data-lane=\"{c_lane}\" data-lane-tone=\"{}\" data-active-branch=\"{}\" data-snapshot=\"{}\" data-merge=\"{}\"/>",
            if is_merge { "6.2" } else { "5.0" },
            c_lane % 6,
            bool_str(is_active_branch),
            bool_str(is_snapshot),
            bool_str(is_merge),
        ));
    }

    out.push_str("</svg>");

    // Interactive commit rows aligned beside the SVG rail
    out.push_str("<div class=\"omni-dag-rows\" role=\"list\" aria-label=\"Commit history\">");
    for cm in &commits {
        let c_lane = *lane_of.get(&cm.branch).unwrap_or(&0);
        let is_active_branch = cm.branch == active_branch;
        let is_snapshot = active_snapshot_clean.as_deref() == Some(cm.id.as_str());
        let is_head = branches
            .iter()
            .any(|b| b.name == cm.branch && b.head_commit == cm.id);
        let is_merge = cm.parents.len() > 1 || !cm.merged_from.is_empty();

        let delta_sign = if cm.atom_delta > 0 {
            "pos"
        } else if cm.atom_delta < 0 {
            "neg"
        } else {
            "zero"
        };
        let delta_text = if cm.atom_delta > 0 {
            format!("+{} atoms", cm.atom_delta)
        } else if cm.atom_delta < 0 {
            format!("{} atoms", cm.atom_delta)
        } else {
            "0 atoms".to_owned()
        };

        let aria_label = format!(
            "Commit {} on {}: {}, by {}, {}",
            short_hash(&cm.id),
            cm.branch,
            cm.summary,
            cm.actor,
            delta_text
        );

        out.push_str(&format!(
            "<div class=\"omni-dag-row\" role=\"listitem\" style=\"padding-left:{rail_w:.0}px;min-height:{DAG_ROW_H:.0}px\" data-commit-id=\"{}\" data-branch=\"{}\" data-lane=\"{c_lane}\" data-lane-tone=\"{}\" data-active-branch=\"{}\" data-snapshot=\"{}\" data-head=\"{}\">",
            esc(&cm.id),
            esc(&cm.branch),
            c_lane % 6,
            bool_str(is_active_branch),
            bool_str(is_snapshot),
            bool_str(is_head),
        ));

        out.push_str(&format!(
            "<button type=\"button\" class=\"omni-commit-node\" data-action=\"omnigraph-select-commit\" data-commit-id=\"{}\" data-branch=\"{}\" data-active-branch=\"{}\" data-snapshot=\"{}\" data-head=\"{}\" aria-current=\"{}\" aria-label=\"{}\">",
            esc(&cm.id),
            esc(&cm.branch),
            bool_str(is_active_branch),
            bool_str(is_snapshot),
            bool_str(is_head),
            if is_snapshot { "step" } else { "false" },
            esc(&aria_label),
        ));
        out.push_str("<span class=\"omni-commit-head\">");
        out.push_str(&format!(
            "<code class=\"omni-commit-hash\">{}</code>",
            esc(&short_hash(&cm.id))
        ));
        if is_merge {
            out.push_str("<span class=\"omni-commit-kind-badge\">Merge</span>");
        }
        out.push_str(&format!(
            "<span class=\"omni-commit-summary\">{}</span>",
            esc(&cm.summary)
        ));
        out.push_str("</span>");

        out.push_str("<span class=\"omni-commit-meta\">");
        out.push_str(&format!(
            "<span class=\"omni-branch-pill\" data-lane-tone=\"{}\">{}</span>",
            c_lane % 6,
            esc(&cm.branch)
        ));
        out.push_str(&format!(
            "<span class=\"omni-actor-badge\">{}</span>",
            esc(&cm.actor)
        ));
        out.push_str(&format!(
            "<span class=\"omni-delta-badge\" data-sign=\"{delta_sign}\">{delta_text}</span>"
        ));
        if is_snapshot {
            out.push_str("<span class=\"omni-snapshot-badge\">Snapshot</span>");
        } else if is_head {
            out.push_str("<span class=\"omni-head-badge\">HEAD</span>");
        }
        if !cm.created_at.is_empty() {
            out.push_str(&format!(
                "<span class=\"omni-commit-time\">{}</span>",
                esc(&cm.created_at)
            ));
        }
        out.push_str("</span></button>");

        out.push_str("<span class=\"omni-commit-actions\">");
        if !is_active_branch {
            out.push_str(&format!(
                "<button type=\"button\" class=\"omni-branch-action\" data-action=\"omnigraph-switch-branch\" data-branch=\"{}\" title=\"Switch active branch to {}\">Switch</button>",
                esc(&cm.branch),
                esc(&cm.branch),
            ));
        }
        if cm.branch != "main" {
            out.push_str(&format!(
                "<button type=\"button\" class=\"omni-branch-action\" data-action=\"omnigraph-preview-diff\" data-branch=\"{}\" title=\"Preview diff of {} against main\">Diff</button>",
                esc(&cm.branch),
                esc(&cm.branch),
            ));
        }
        out.push_str("</span></div>");
    }
    out.push_str("</div></div></div>");
    out
}

#[allow(dead_code)]
pub fn render_merge_diff(diff: &Value) -> String {
    let source = str_field(diff, &["source", "source_branch"]).if_empty("source");
    let target = str_field(diff, &["target", "target_branch"]).if_empty("main");
    let merge_base = str_field(diff, &["merge_base", "base_commit", "base"]);

    let mut added_atoms = array_items(diff, &["added_atoms", "added"]);
    let mut updated_atoms = array_items(diff, &["updated_atoms", "updated", "modified_atoms"]);
    let mut deleted_atoms = array_items(diff, &["deleted_atoms", "removed_atoms", "deleted"]);
    let mut added_edges = array_items(diff, &["added_edges", "edges", "edge_changes"]);
    let mut deleted_edges = array_items(diff, &["deleted_edges", "removed_edges"]);
    if added_atoms.is_empty()
        && updated_atoms.is_empty()
        && deleted_atoms.is_empty()
        && added_edges.is_empty()
        && deleted_edges.is_empty()
    {
        partition_entity_changes(
            diff,
            &mut added_atoms,
            &mut updated_atoms,
            &mut deleted_atoms,
            &mut added_edges,
            &mut deleted_edges,
        );
    }
    let conflicts = array_items(diff, &["conflicts", "merge_conflicts"]);

    let raw_outcome = str_field(diff, &["outcome", "status", "code"]);
    let outcome = normalize_outcome(
        &raw_outcome,
        !conflicts.is_empty(),
        added_atoms.is_empty()
            && updated_atoms.is_empty()
            && deleted_atoms.is_empty()
            && added_edges.is_empty()
            && deleted_edges.is_empty(),
    );

    let mut out = format!(
        "<div class=\"omni-diff\" data-outcome=\"{}\" data-source=\"{}\" data-target=\"{}\">",
        esc(outcome),
        esc(&source),
        esc(&target),
    );

    // Outcome banner
    let (outcome_title, outcome_icon) = match outcome {
        "conflict" => (
            "409 Conflict — Manual Resolution Needed",
            "<svg class=\"omni-outcome-icon\" viewBox=\"0 0 16 16\" width=\"16\" height=\"16\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.6\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\"><path d=\"M8 2 L14.5 13.5 H1.5 Z\"/><path d=\"M8 6 V9.2\"/><circle cx=\"8\" cy=\"11.4\" r=\"0.7\" fill=\"currentColor\" stroke=\"none\"/></svg>",
        ),
        "fast_forward" => (
            "Fast-Forward Merge Ready",
            "<svg class=\"omni-outcome-icon\" viewBox=\"0 0 16 16\" width=\"16\" height=\"16\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.6\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\"><path d=\"M2.5 4.5 L7.5 8 L2.5 11.5 V4.5 Z M8.5 4.5 L13.5 8 L8.5 11.5 V4.5 Z\"/></svg>",
        ),
        "merged" => (
            "3-Way Merge Clean",
            "<svg class=\"omni-outcome-icon\" viewBox=\"0 0 16 16\" width=\"16\" height=\"16\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.6\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\"><circle cx=\"4.5\" cy=\"3.5\" r=\"1.8\"/><circle cx=\"11.5\" cy=\"4.5\" r=\"1.8\"/><circle cx=\"4.5\" cy=\"12.5\" r=\"1.8\"/><path d=\"M4.5 5.3 V10.7 M11.5 6.3 C11.5 9.2 6.5 8.8 4.5 10.7\"/></svg>",
        ),
        _ => (
            "Already Up to Date",
            "<svg class=\"omni-outcome-icon\" viewBox=\"0 0 16 16\" width=\"16\" height=\"16\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.6\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\"><circle cx=\"8\" cy=\"8\" r=\"6\"/><path d=\"M5.3 8.1 L7.2 10 L10.8 6.2\"/></svg>",
        ),
    };

    out.push_str(&format!(
        "<div class=\"omni-diff-banner\" data-outcome=\"{}\"><div class=\"omni-diff-banner-main\">{outcome_icon}<div class=\"omni-diff-banner-copy\"><span class=\"omni-diff-outcome-badge\" data-outcome=\"{}\">{outcome_title}</span><div class=\"omni-diff-route\"><span class=\"omni-branch-pill\">{}</span><span class=\"omni-diff-arrow\" aria-label=\"into\">→</span><span class=\"omni-branch-pill\">{}</span>",
        esc(outcome),
        esc(outcome),
        esc(&source),
        esc(&target),
    ));
    if !merge_base.is_empty() {
        out.push_str(&format!(
            "<span class=\"omni-diff-base\">Base <code>{}</code></span>",
            esc(&short_hash(&merge_base))
        ));
    }
    out.push_str("</div></div></div>");

    out.push_str("<div class=\"omni-diff-counts\">");
    out.push_str(&format!(
        "<span class=\"omni-diff-count\" data-change=\"added\">+{} added</span>",
        added_atoms.len()
    ));
    out.push_str(&format!(
        "<span class=\"omni-diff-count\" data-change=\"updated\">~{} updated</span>",
        updated_atoms.len()
    ));
    out.push_str(&format!(
        "<span class=\"omni-diff-count\" data-change=\"deleted\">-{} deleted</span>",
        deleted_atoms.len()
    ));
    let total_edge_changes = added_edges.len() + deleted_edges.len();
    if total_edge_changes > 0 {
        out.push_str(&format!(
            "<span class=\"omni-diff-count\" data-change=\"edges\">{total_edge_changes} edges</span>"
        ));
    }
    if !conflicts.is_empty() {
        out.push_str(&format!(
            "<span class=\"omni-diff-count\" data-change=\"conflict\">! {} conflicts</span>",
            conflicts.len()
        ));
    }
    out.push_str("</div></div>");

    // 409 Conflict cards
    if !conflicts.is_empty() {
        out.push_str("<section class=\"omni-diff-conflicts\" aria-label=\"Merge conflicts\">");
        out.push_str(&format!(
            "<h4 class=\"omni-diff-heading\">Typed 409 Conflicts ({})</h4>",
            conflicts.len()
        ));
        for conflict in &conflicts {
            out.push_str(&render_conflict_card(conflict, &source, &target));
        }
        out.push_str("</section>");
    }

    // Added atoms
    if !added_atoms.is_empty() {
        out.push_str("<section class=\"omni-diff-section\" data-change=\"added\">");
        out.push_str(&format!(
            "<h4 class=\"omni-diff-heading\"><span class=\"omni-diff-mark\" aria-hidden=\"true\">+</span> Added Context Atoms ({})</h4>",
            added_atoms.len()
        ));
        for item in &added_atoms {
            out.push_str(&render_diff_atom_card(item, "added", "+", "Added"));
        }
        out.push_str("</section>");
    }

    // Updated atoms
    if !updated_atoms.is_empty() {
        out.push_str("<section class=\"omni-diff-section\" data-change=\"updated\">");
        out.push_str(&format!(
            "<h4 class=\"omni-diff-heading\"><span class=\"omni-diff-mark\" aria-hidden=\"true\">~</span> Updated Context Atoms ({})</h4>",
            updated_atoms.len()
        ));
        for item in &updated_atoms {
            out.push_str(&render_diff_atom_card(item, "updated", "~", "Updated"));
        }
        out.push_str("</section>");
    }

    // Deleted atoms
    if !deleted_atoms.is_empty() {
        out.push_str("<section class=\"omni-diff-section\" data-change=\"deleted\">");
        out.push_str(&format!(
            "<h4 class=\"omni-diff-heading\"><span class=\"omni-diff-mark\" aria-hidden=\"true\">-</span> Deleted Context Atoms ({})</h4>",
            deleted_atoms.len()
        ));
        for item in &deleted_atoms {
            out.push_str(&render_diff_atom_card(item, "deleted", "-", "Deleted"));
        }
        out.push_str("</section>");
    }

    // Edge changes
    if total_edge_changes > 0 {
        out.push_str("<section class=\"omni-diff-section\" data-change=\"edges\">");
        out.push_str(&format!(
            "<h4 class=\"omni-diff-heading\">Context Edge Changes ({total_edge_changes})</h4>"
        ));
        for edge in &added_edges {
            let change = str_field(edge, &["change", "op", "status"]);
            let (kind, mark, label) = if change.eq_ignore_ascii_case("deleted")
                || change.eq_ignore_ascii_case("removed")
            {
                ("deleted", "-", "Deleted")
            } else {
                ("added", "+", "Added")
            };
            out.push_str(&render_diff_edge_card(edge, kind, mark, label));
        }
        for edge in &deleted_edges {
            out.push_str(&render_diff_edge_card(edge, "deleted", "-", "Deleted"));
        }
        out.push_str("</section>");
    }

    if added_atoms.is_empty()
        && updated_atoms.is_empty()
        && deleted_atoms.is_empty()
        && total_edge_changes == 0
        && conflicts.is_empty()
    {
        out.push_str(&format!(
            "<div class=\"omni-diff-empty\"><p>Branches <strong>{}</strong> and <strong>{}</strong> share the same context state with no diverging atoms or edges.</p></div>",
            esc(&source),
            esc(&target),
        ));
    }

    out.push_str("</div>");
    out
}

fn render_conflict_card(conflict: &Value, source: &str, target: &str) -> String {
    let raw_kind = str_field(conflict, &["kind", "conflict_type", "type", "code"])
        .if_empty("divergent_update");
    let kind_token = escape_attr_token(&raw_kind);
    let kind_label = title_case_token(&raw_kind);
    let entity_id = str_field(conflict, &["entity_id", "atom_id", "id", "target_id"]);
    let entity_kind = str_field(conflict, &["entity_kind"]);
    let type_name = str_field(conflict, &["type_name", "node_type"]);
    let field = str_field(conflict, &["field", "property"]);
    let message = str_field(conflict, &["message", "reason", "title", "summary"]);
    let hint = str_field(conflict, &["resolution_hint", "hint"]);

    let base_val = pick_first_value(conflict, &["base", "base_value"]);
    let source_val = pick_first_value(conflict, &["source", "source_value", "ours"]);
    let target_val = pick_first_value(conflict, &["target", "target_value", "theirs"]);
    let has_compare = base_val.is_some() || source_val.is_some() || target_val.is_some();

    let mut html = format!(
        "<article class=\"omni-conflict-card\" data-conflict-kind=\"{kind_token}\" data-entity-id=\"{}\">",
        esc(&entity_id),
    );
    html.push_str("<div class=\"omni-conflict-head\">");
    html.push_str(&render_kind_icon("warning"));
    html.push_str(&format!(
        "<span class=\"omni-conflict-badge\" data-conflict-kind=\"{kind_token}\">{}</span>",
        esc(&kind_label)
    ));
    if !type_name.is_empty() || !entity_kind.is_empty() {
        let type_badge = match (entity_kind.is_empty(), type_name.is_empty()) {
            (false, false) => format!("{entity_kind} · {type_name}"),
            (false, true) => entity_kind.clone(),
            (true, false) => type_name.clone(),
            (true, true) => String::new(),
        };
        html.push_str(&format!(
            "<span class=\"omni-conflict-type\">{}</span>",
            esc(&type_badge)
        ));
    }
    if !entity_id.is_empty() {
        html.push_str(&format!(
            "<code class=\"omni-conflict-entity\">{}</code>",
            esc(&entity_id)
        ));
    }
    if !field.is_empty() {
        html.push_str(&format!(
            "<span class=\"omni-conflict-field\">field: <code>{}</code></span>",
            esc(&field)
        ));
    }
    html.push_str("</div>");

    if !message.is_empty() {
        html.push_str(&format!(
            "<p class=\"omni-conflict-message\">{}</p>",
            esc(&message)
        ));
    }

    if has_compare {
        html.push_str("<div class=\"omni-conflict-compare\">");
        html.push_str(&render_conflict_pane("base", "Base (Ancestor)", base_val));
        html.push_str(&render_conflict_pane(
            "source",
            &format!("Source ({source})"),
            source_val,
        ));
        html.push_str(&render_conflict_pane(
            "target",
            &format!("Target ({target})"),
            target_val,
        ));
        html.push_str("</div>");
    }

    if !hint.is_empty() {
        html.push_str(&format!(
            "<p class=\"omni-conflict-hint\">{}</p>",
            esc(&hint)
        ));
    }
    html.push_str("</article>");
    html
}

fn render_conflict_pane(side: &str, heading: &str, val: Option<&Value>) -> String {
    let text = format_diff_value(val);
    let is_deleted = text == "(deleted)" || text == "—" || val.is_some_and(Value::is_null);
    format!(
        "<div class=\"omni-conflict-pane\" data-side=\"{side}\" data-empty=\"{}\"><span class=\"omni-conflict-pane-label\">{}</span><pre class=\"omni-conflict-value\">{}</pre></div>",
        bool_str(is_deleted),
        esc(heading),
        esc(&text),
    )
}

fn partition_entity_changes(
    diff: &Value,
    added_atoms: &mut Vec<Value>,
    updated_atoms: &mut Vec<Value>,
    deleted_atoms: &mut Vec<Value>,
    added_edges: &mut Vec<Value>,
    deleted_edges: &mut Vec<Value>,
) {
    let Some(changes) = diff.get("changes").and_then(Value::as_array) else {
        return;
    };
    for ch in changes {
        let entity_kind = str_field(ch, &["kind"]).to_ascii_lowercase();
        let op = str_field(ch, &["op"]).to_ascii_lowercase();
        let id = str_field(ch, &["id"]);
        let type_name = ch
            .get("type")
            .and_then(|t| t.as_str().or_else(|| t.get("name").and_then(Value::as_str)))
            .unwrap_or("");
        if entity_kind == "edge" {
            let img = ch
                .get("after")
                .filter(|v| !v.is_null())
                .or_else(|| ch.get("before").filter(|v| !v.is_null()));
            let endpoints = img.and_then(|v| v.get("endpoints"));
            let from = endpoints
                .and_then(|e| e.get("from"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let to = endpoints
                .and_then(|e| e.get("to"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let subkind = img
                .and_then(|v| v.get("properties"))
                .and_then(|p| p.get("rel_kind").or_else(|| p.get("label")))
                .and_then(Value::as_str)
                .unwrap_or("");
            let edge_val = serde_json::json!({
                "from": from,
                "to": to,
                "relation": type_name,
                "subkind": subkind,
            });
            if op == "delete" || op == "remove" {
                deleted_edges.push(edge_val);
            } else {
                added_edges.push(edge_val);
            }
        } else {
            let unwrap_img = |v: Option<&Value>| -> Value {
                match v {
                    Some(obj) if obj.is_object() => {
                        let props = obj
                            .get("properties")
                            .filter(|p| p.is_object())
                            .unwrap_or(obj);
                        let mut cloned = props.clone();
                        if let Some(m) = cloned.as_object_mut() {
                            m.entry("id".to_owned())
                                .or_insert_with(|| Value::String(id.clone()));
                            if !type_name.is_empty() {
                                m.entry("type_name".to_owned())
                                    .or_insert_with(|| Value::String(type_name.to_owned()));
                            }
                        }
                        cloned
                    }
                    _ => Value::Null,
                }
            };
            let before = unwrap_img(ch.get("before"));
            let after = unwrap_img(ch.get("after"));
            match op.as_str() {
                "delete" | "remove" => deleted_atoms.push(if before.is_object() {
                    before
                } else {
                    ch.clone()
                }),
                "update" | "upsert" => updated_atoms.push(serde_json::json!({
                    "id": id,
                    "before": before,
                    "after": after,
                })),
                _ => added_atoms.push(if after.is_object() { after } else { ch.clone() }),
            }
        }
    }
}

fn render_diff_atom_card(item: &Value, change: &str, marker: &str, sr_label: &str) -> String {
    let primary_raw = item
        .get("after")
        .or_else(|| item.get("source"))
        .or_else(|| item.get("atom"))
        .filter(|v| v.is_object())
        .unwrap_or(item);
    let primary = primary_raw
        .get("properties")
        .filter(|v| v.is_object())
        .unwrap_or(primary_raw);
    let previous_raw = item
        .get("before")
        .or_else(|| item.get("target"))
        .or_else(|| item.get("base"))
        .filter(|v| v.is_object());
    let previous = previous_raw.map(|p| p.get("properties").filter(|v| v.is_object()).unwrap_or(p));

    let id = str_field(primary, &["id", "atom_id", "@id"])
        .if_empty_from(|| str_field(primary_raw, &["id", "atom_id", "@id"]))
        .if_empty_from(|| str_field(item, &["id", "atom_id", "entity_id"]));
    let raw_kind = str_field(primary, &["kind"])
        .if_empty_from(|| str_field(item, &["kind"]))
        .if_empty("insight");
    let kind_spec = atom_kind_spec(&raw_kind);
    let title = str_field(primary, &["title", "name", "label"])
        .if_empty_from(|| str_field(item, &["title", "name"]))
        .if_empty(&id);
    let summary = str_field(primary, &["summary", "body", "content"]);
    let branch = str_field(primary, &["branch", "source_branch", "graph_branch"])
        .if_empty_from(|| str_field(item, &["branch", "source_branch", "graph_branch"]));

    let mut html = format!(
        "<article class=\"omni-diff-card\" data-change=\"{change}\" data-kind=\"{}\" data-node-id=\"{}\">",
        kind_spec.key,
        esc(&id),
    );
    html.push_str("<div class=\"omni-diff-card-head\">");
    html.push_str(&format!(
        "<span class=\"omni-diff-mark\" data-change=\"{change}\" aria-hidden=\"true\">{marker}</span><span class=\"sr-only\">{sr_label}</span>"
    ));
    html.push_str(&render_kind_icon(kind_spec.key));
    html.push_str(&format!(
        "<span class=\"omni-kind-badge\" data-kind=\"{}\">{}</span>",
        kind_spec.key, kind_spec.label
    ));
    html.push_str(&format!(
        "<strong class=\"omni-diff-card-title\">{}</strong>",
        esc(&title)
    ));
    if !branch.is_empty() {
        html.push_str(&format!(
            "<span class=\"omni-branch-pill\">{}</span>",
            esc(&branch)
        ));
    }
    html.push_str("</div>");

    if change == "updated" {
        if let Some(prev) = previous {
            let prev_summary = str_field(prev, &["summary", "body", "content", "title"]);
            let next_summary = if summary.is_empty() {
                title.clone()
            } else {
                summary.clone()
            };
            html.push_str("<div class=\"omni-diff-before-after\">");
            if !prev_summary.is_empty() {
                html.push_str(&format!(
                    "<div class=\"omni-diff-side\" data-side=\"before\"><span class=\"omni-diff-side-label\">Before</span><p>{}</p></div>",
                    esc(&prev_summary)
                ));
            }
            if !next_summary.is_empty() {
                html.push_str(&format!(
                    "<div class=\"omni-diff-side\" data-side=\"after\"><span class=\"omni-diff-side-label\">After</span><p>{}</p></div>",
                    esc(&next_summary)
                ));
            }
            html.push_str("</div>");
        } else if !summary.is_empty() {
            html.push_str(&format!(
                "<p class=\"omni-diff-card-summary\">{}</p>",
                esc(&summary)
            ));
        }
    } else if !summary.is_empty() {
        html.push_str(&format!(
            "<p class=\"omni-diff-card-summary\">{}</p>",
            esc(&summary)
        ));
    }

    html.push_str("</article>");
    html
}

fn extract_canvas_edge(edge: &Value) -> Option<CanvasEdge> {
    let endpoints = edge.get("endpoints");
    let props = edge.get("properties");
    let from = str_field(edge, &["from", "src", "source", "@src"]).if_empty_from(|| {
        endpoints
            .map(|ep| str_field(ep, &["from", "src", "source", "@src"]))
            .unwrap_or_default()
    });
    let to = str_field(edge, &["to", "dst", "target", "@dst"]).if_empty_from(|| {
        endpoints
            .map(|ep| str_field(ep, &["to", "dst", "target", "@dst"]))
            .unwrap_or_default()
    });
    if from.is_empty() || to.is_empty() {
        return None;
    }
    let top_rel = str_field(edge, &["relation", "kind", "type", "@type"]);
    let prop_rel = props
        .map(|p| str_field(p, &["relation", "rel_kind", "relation_kind", "kind"]))
        .unwrap_or_default();
    let raw_rel = if !top_rel.is_empty() {
        top_rel
    } else if !prop_rel.is_empty() {
        prop_rel.clone()
    } else {
        "relates_to".to_owned()
    };
    let mut subkind = str_field(edge, &["subkind", "rel_kind", "relation_kind", "label"])
        .if_empty_from(|| {
            props
                .map(|p| str_field(p, &["subkind", "rel_kind", "relation_kind", "label"]))
                .unwrap_or_default()
        });
    if subkind.is_empty() && !prop_rel.is_empty() && prop_rel != raw_rel {
        subkind = prop_rel;
    }
    let (relation, subkind, arrow_key, label) = normalize_edge_relation(&raw_rel, &subkind);
    Some(CanvasEdge {
        from,
        to,
        relation,
        subkind,
        arrow_key,
        label,
    })
}

fn render_diff_edge_card(edge: &Value, change: &str, marker: &str, sr_label: &str) -> String {
    let Some(parsed) = extract_canvas_edge(edge) else {
        return String::new();
    };
    format!(
        "<div class=\"omni-diff-edge-card\" data-change=\"{change}\" data-rel=\"{}\"><span class=\"omni-diff-mark\" data-change=\"{change}\" aria-hidden=\"true\">{marker}</span><span class=\"sr-only\">{sr_label}</span><span class=\"omni-diff-edge-rel\">{}</span><code class=\"omni-diff-edge-end\">{}</code><span class=\"omni-diff-arrow\" aria-label=\"to\">→</span><code class=\"omni-diff-edge-end\">{}</code></div>",
        escape_attr_token(&parsed.arrow_key),
        esc(&parsed.label),
        esc(&parsed.from),
        esc(&parsed.to),
    )
}

fn extract_graph_items(
    graph: &Value,
) -> (Vec<CanvasNode>, Vec<CanvasEdge>, BTreeMap<String, String>) {
    let mut nodes = Vec::new();
    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    let mut seen_ids: BTreeSet<String> = BTreeSet::new();

    if let Some(arr) = graph.get("sessions").and_then(Value::as_array) {
        for item in arr {
            let id = str_field(item, &["id", "session_id", "@id"]);
            if id.is_empty() || !seen_ids.insert(id.clone()) {
                continue;
            }
            let title = str_field(item, &["title", "name", "label"]).if_empty(&id);
            let branch =
                str_field(item, &["branch", "source_branch", "graph_branch"]).if_empty("main");
            let shared_space =
                str_field(item, &["shared_space", "space_name"]).if_empty_from(|| {
                    item.get("omnigraph")
                        .map(|o| str_field(o, &["shared_space", "space_name"]))
                        .unwrap_or_default()
                });
            let status = str_field(item, &["status", "state"]).if_empty("idle");
            let provider = str_field(item, &["provider"]);
            let actor = str_field(item, &["actor", "actor_id"]);
            let tags = string_list(item, &["tags"]);
            let blob = format!(
                "{id} {title} {branch} {shared_space} {status} {provider} {actor} {}",
                tags.join(" ")
            )
            .to_ascii_lowercase();
            let (w, h) = NodeKindGroup::Session.size();
            nodes.push(CanvasNode {
                id: id.clone(),
                group: NodeKindGroup::Session,
                kind: "session".to_owned(),
                title,
                subtitle: actor,
                branch,
                shared_space,
                session_id: id,
                status,
                provider,
                confidence: None,
                pinned: false,
                pin_priority: 0,
                tags,
                touch_count: 0,
                search_blob: blob,
                x: 0.0,
                y: 0.0,
                w,
                h,
            });
        }
    }

    if let Some(arr) = graph.get("atoms").and_then(Value::as_array) {
        for item in arr {
            let id = str_field(item, &["id", "atom_id", "@id"]);
            if id.is_empty() || !seen_ids.insert(id.clone()) {
                continue;
            }
            let raw_kind = str_field(item, &["kind"]).if_empty("insight");
            let kind = atom_kind_spec(&raw_kind).key.to_owned();
            let title = str_field(item, &["title", "name", "label"]).if_empty(&id);
            let summary = str_field(item, &["summary", "body", "content"]);
            let branch =
                str_field(item, &["branch", "source_branch", "graph_branch"]).if_empty("main");
            let session_id = str_field(
                item,
                &[
                    "session_id",
                    "source_session_id",
                    "author_session_id",
                    "published_by",
                ],
            );
            let actor = str_field(item, &["actor", "actor_id"]);
            let tags = string_list(item, &["tags"]);
            let confidence = f64_field(item, &["confidence"]).map(|c| {
                let pct = if (0.0..=1.0).contains(&c) {
                    c * 100.0
                } else {
                    c
                };
                pct.round().clamp(0.0, 100.0) as u8
            });
            let pin_priority = i64_field(item, &["pin_priority"]).unwrap_or(0);
            let pinned = bool_field(item, &["pinned"]) || pin_priority > 0;
            let blob = format!(
                "{id} {kind} {title} {summary} {branch} {session_id} {actor} {}",
                tags.join(" ")
            )
            .to_ascii_lowercase();
            let (w, h) = NodeKindGroup::Atom.size();
            nodes.push(CanvasNode {
                id,
                group: NodeKindGroup::Atom,
                kind,
                title,
                subtitle: summary,
                branch,
                shared_space: String::new(),
                session_id,
                status: String::new(),
                provider: String::new(),
                confidence,
                pinned,
                pin_priority,
                tags,
                touch_count: 0,
                search_blob: blob,
                x: 0.0,
                y: 0.0,
                w,
                h,
            });
        }
    }

    if let Some(arr) = graph.get("files").and_then(Value::as_array) {
        for item in arr {
            let (id, path) = if let Some(s) = item.as_str() {
                (s.to_owned(), s.to_owned())
            } else {
                let path = str_field(item, &["path", "file_path", "name", "label"]);
                let id = str_field(item, &["id", "@id"]).if_empty(&path);
                let resolved = if path.is_empty() { id.clone() } else { path };
                (id, resolved)
            };
            if id.is_empty() || !seen_ids.insert(id.clone()) {
                continue;
            }
            if path != id && !path.is_empty() {
                aliases.entry(path.clone()).or_insert_with(|| id.clone());
            }
            let branch = str_field(item, &["branch", "source_branch", "graph_branch"]);
            let lang = str_field(item, &["language", "kind"]);
            let touch_count = i64_field(item, &["touch_count", "atom_count"])
                .unwrap_or(0)
                .max(0) as usize;
            let (base_name, dir_hint) = split_file_label(&path);
            let blob = format!("{id} {path} {branch} {lang}").to_ascii_lowercase();
            let (w, h) = NodeKindGroup::File.size();
            nodes.push(CanvasNode {
                id,
                group: NodeKindGroup::File,
                kind: "file".to_owned(),
                title: base_name,
                subtitle: dir_hint,
                branch,
                shared_space: String::new(),
                session_id: String::new(),
                status: String::new(),
                provider: lang,
                confidence: None,
                pinned: false,
                pin_priority: 0,
                tags: Vec::new(),
                touch_count,
                search_blob: blob,
                x: 0.0,
                y: 0.0,
                w,
                h,
            });
        }
    }

    let mut edges = Vec::new();
    let mut seen_edges: BTreeSet<(String, String, String, String)> = BTreeSet::new();
    if let Some(arr) = graph.get("edges").and_then(Value::as_array) {
        for item in arr {
            let Some(edge) = extract_canvas_edge(item) else {
                continue;
            };
            if !seen_edges.insert((
                edge.from.clone(),
                edge.to.clone(),
                edge.relation.clone(),
                edge.subkind.clone(),
            )) {
                continue;
            }
            edges.push(edge);
        }
    }

    (nodes, edges, aliases)
}

fn apply_constellation_filter(
    nodes: Vec<CanvasNode>,
    edges: Vec<CanvasEdge>,
    filter: &ConstellationFilter,
    active_branch: &str,
) -> (Vec<CanvasNode>, Vec<CanvasEdge>) {
    let query = filter.query.trim().to_ascii_lowercase();
    let kind_filter = filter.kind.trim().to_ascii_lowercase();
    let branch_filter = filter.branch.trim();
    let branch_active = !branch_filter.is_empty() && branch_filter != "*" && branch_filter != "all";
    // `/api/omnigraph/graph?branch=X` already returns the materialized snapshot for `X`
    // (including inherited `main` atoms on `session/*` branches), so never hide them.
    let snapshot_matches_branch =
        !active_branch.is_empty() && active_branch.trim().eq_ignore_ascii_case(branch_filter);
    let kind_active = !kind_filter.is_empty() && kind_filter != "*" && kind_filter != "all";

    let mut atom_ids_kept: BTreeSet<String> = BTreeSet::new();
    let mut direct_match_ids: BTreeSet<String> = BTreeSet::new();

    for node in &nodes {
        let branch_ok = !branch_active
            || snapshot_matches_branch
            || node.branch.is_empty()
            || node.branch == branch_filter;
        let query_ok = query.is_empty() || node.search_blob.contains(&query);
        match node.group {
            NodeKindGroup::Atom => {
                let kind_ok = !kind_active || node.kind == kind_filter;
                if branch_ok && kind_ok && query_ok {
                    atom_ids_kept.insert(node.id.clone());
                    direct_match_ids.insert(node.id.clone());
                }
            }
            NodeKindGroup::Session => {
                if filter.show_sessions && branch_ok && query_ok {
                    direct_match_ids.insert(node.id.clone());
                }
            }
            NodeKindGroup::File => {
                if filter.show_files && branch_ok && query_ok {
                    direct_match_ids.insert(node.id.clone());
                }
            }
        }
    }

    // Sessions and files linked to a kept atom remain visible when their visibility toggle is on
    // (unless explicitly excluded by branch filter).
    let mut connected_to_kept_atom: BTreeSet<String> = BTreeSet::new();
    for edge in &edges {
        if atom_ids_kept.contains(&edge.from) {
            connected_to_kept_atom.insert(edge.to.clone());
        }
        if atom_ids_kept.contains(&edge.to) {
            connected_to_kept_atom.insert(edge.from.clone());
        }
    }

    let mut kept_nodes = Vec::new();
    let mut kept_ids: BTreeSet<String> = BTreeSet::new();
    for node in nodes {
        let branch_ok = !branch_active
            || snapshot_matches_branch
            || node.branch.is_empty()
            || node.branch == branch_filter;
        let include = match node.group {
            NodeKindGroup::Atom => atom_ids_kept.contains(&node.id),
            NodeKindGroup::Session => {
                if !filter.show_sessions {
                    false
                } else if kind_active {
                    connected_to_kept_atom.contains(&node.id) && branch_ok
                } else if !query.is_empty() {
                    direct_match_ids.contains(&node.id)
                        || (connected_to_kept_atom.contains(&node.id) && branch_ok)
                } else {
                    branch_ok || connected_to_kept_atom.contains(&node.id)
                }
            }
            NodeKindGroup::File => {
                if !filter.show_files {
                    false
                } else if kind_active || !query.is_empty() {
                    direct_match_ids.contains(&node.id) || connected_to_kept_atom.contains(&node.id)
                } else {
                    branch_ok || connected_to_kept_atom.contains(&node.id)
                }
            }
        };
        if include {
            kept_ids.insert(node.id.clone());
            kept_nodes.push(node);
        }
    }

    let kept_edges = edges
        .into_iter()
        .filter(|e| kept_ids.contains(&e.from) && kept_ids.contains(&e.to))
        .collect();

    (kept_nodes, kept_edges)
}

fn layout_constellation(
    mut nodes: Vec<CanvasNode>,
    edges: &[CanvasEdge],
) -> (Vec<CanvasNode>, f64, f64) {
    let mut session_indices = Vec::new();
    let mut atom_indices = Vec::new();
    let mut file_indices = Vec::new();
    for (idx, n) in nodes.iter().enumerate() {
        match n.group {
            NodeKindGroup::Session => session_indices.push(idx),
            NodeKindGroup::Atom => atom_indices.push(idx),
            NodeKindGroup::File => file_indices.push(idx),
        }
    }

    // Infer publishing session for atoms from published_by edges when atom.session_id is blank.
    let node_group_by_id: BTreeMap<String, NodeKindGroup> =
        nodes.iter().map(|n| (n.id.clone(), n.group)).collect();
    let mut inferred_session: BTreeMap<String, String> = BTreeMap::new();
    for edge in edges {
        if edge.relation == "published_by"
            && node_group_by_id.get(&edge.from) == Some(&NodeKindGroup::Atom)
            && node_group_by_id.get(&edge.to) == Some(&NodeKindGroup::Session)
        {
            inferred_session
                .entry(edge.from.clone())
                .or_insert_with(|| edge.to.clone());
        }
    }
    for &idx in &atom_indices {
        if nodes[idx].session_id.is_empty() {
            if let Some(sid) = inferred_session.get(&nodes[idx].id) {
                nodes[idx].session_id = sid.clone();
            }
        }
    }

    // 1. Place Sessions on left orbital arc(s).
    session_indices.sort_by(|&a, &b| {
        nodes[a]
            .branch
            .cmp(&nodes[b].branch)
            .then_with(|| nodes[a].title.cmp(&nodes[b].title))
            .then_with(|| nodes[a].id.cmp(&nodes[b].id))
    });

    let mut next_x = MARGIN_X;
    let session_span_h = if !session_indices.is_empty() {
        let s_cols = if session_indices.len() > 7 { 2 } else { 1 };
        let s_rows = session_indices.len().div_ceil(s_cols);
        let col_h = s_rows as f64 * SESSION_H + (s_rows.saturating_sub(1) as f64) * NODE_GAP_Y;
        let mid_idx = (s_rows.saturating_sub(1)) as f64 * 0.5;
        for (i, &idx) in session_indices.iter().enumerate() {
            let col = i / s_rows;
            let row = i % s_rows;
            let t = if mid_idx > 0.0 {
                (row as f64 - mid_idx) / mid_idx
            } else {
                0.0
            };
            let arc_dx = t * t * 34.0;
            nodes[idx].x = next_x + (col as f64) * (SESSION_W + 30.0) + arc_dx;
            nodes[idx].y = MARGIN_Y + (row as f64) * (SESSION_H + NODE_GAP_Y);
        }
        let max_s_right = session_indices
            .iter()
            .map(|&i| nodes[i].x + nodes[i].w)
            .fold(next_x, f64::max);
        next_x = max_s_right + TRACK_GAP_X;
        col_h
    } else {
        0.0
    };

    // 2. Place ContextAtoms clustered cleanly by kind and session across balanced orbital columns.
    atom_indices.sort_by(|&a, &b| {
        let ra = atom_kind_spec(&nodes[a].kind).rank;
        let rb = atom_kind_spec(&nodes[b].kind).rank;
        ra.cmp(&rb)
            .then_with(|| nodes[a].session_id.cmp(&nodes[b].session_id))
            .then_with(|| nodes[b].pin_priority.cmp(&nodes[a].pin_priority))
            .then_with(|| nodes[b].confidence.cmp(&nodes[a].confidence))
            .then_with(|| nodes[a].id.cmp(&nodes[b].id))
    });

    let atom_span_h = if !atom_indices.is_empty() {
        let n_atoms = atom_indices.len();
        let a_cols = if n_atoms <= 5 {
            1
        } else if n_atoms <= 14 {
            2
        } else if n_atoms <= 27 {
            3
        } else {
            ((n_atoms as f64) / 7.5).sqrt().ceil().clamp(3.0, 5.0) as usize
        };
        let a_rows = n_atoms.div_ceil(a_cols);
        let mut max_col_h: f64 = 0.0;
        for c in 0..a_cols {
            let start = c * a_rows;
            let end = ((c + 1) * a_rows).min(n_atoms);
            if start >= end {
                continue;
            }
            let slice = &atom_indices[start..end];
            let mut y = MARGIN_Y;
            let mut prev_kind = String::new();
            let col_len = slice.len();
            let mid_r = (col_len.saturating_sub(1)) as f64 * 0.5;
            for (r, &idx) in slice.iter().enumerate() {
                if r > 0 && nodes[idx].kind != prev_kind {
                    y += CLUSTER_EXTRA_GAP_Y;
                }
                let t = if mid_r > 0.0 {
                    (r as f64 - mid_r) / mid_r
                } else {
                    0.0
                };
                let orbital_curve = (1.0 - t * t) * 16.0;
                nodes[idx].x = next_x + (c as f64) * (ATOM_W + ATOM_COL_GAP_X) + orbital_curve;
                nodes[idx].y = y;
                prev_kind = nodes[idx].kind.clone();
                y += ATOM_H + NODE_GAP_Y;
            }
            max_col_h = max_col_h.max(y - NODE_GAP_Y - MARGIN_Y);
        }
        let max_a_right = atom_indices
            .iter()
            .map(|&i| nodes[i].x + nodes[i].w)
            .fold(next_x, f64::max);
        next_x = max_a_right + TRACK_GAP_X;
        max_col_h
    } else {
        0.0
    };

    // 3. Place FileRefs on outer/right orbital arc ordered by barycenter of touching atoms.
    let pos_by_id: BTreeMap<String, f64> = nodes
        .iter()
        .filter(|n| n.group != NodeKindGroup::File)
        .map(|n| (n.id.clone(), n.y + n.h * 0.5))
        .collect();
    let mut file_bary: BTreeMap<String, f64> = BTreeMap::new();
    for &idx in &file_indices {
        let fid = &nodes[idx].id;
        let mut sum = 0.0;
        let mut cnt = 0usize;
        for edge in edges {
            if edge.to == *fid {
                if let Some(&y) = pos_by_id.get(&edge.from) {
                    sum += y;
                    cnt += 1;
                }
            } else if edge.from == *fid {
                if let Some(&y) = pos_by_id.get(&edge.to) {
                    sum += y;
                    cnt += 1;
                }
            }
        }
        file_bary.insert(
            fid.clone(),
            if cnt > 0 {
                sum / (cnt as f64)
            } else {
                f64::MAX
            },
        );
    }
    file_indices.sort_by(|&a, &b| {
        let ba = file_bary.get(&nodes[a].id).copied().unwrap_or(f64::MAX);
        let bb = file_bary.get(&nodes[b].id).copied().unwrap_or(f64::MAX);
        ba.total_cmp(&bb)
            .then_with(|| nodes[a].id.cmp(&nodes[b].id))
    });

    let file_span_h = if !file_indices.is_empty() {
        let f_cols = if file_indices.len() > 10 { 2 } else { 1 };
        let f_rows = file_indices.len().div_ceil(f_cols);
        let col_h = f_rows as f64 * FILE_H + (f_rows.saturating_sub(1) as f64) * NODE_GAP_Y;
        let mid_idx = (f_rows.saturating_sub(1)) as f64 * 0.5;
        for (i, &idx) in file_indices.iter().enumerate() {
            let col = i / f_rows;
            let row = i % f_rows;
            let t = if mid_idx > 0.0 {
                (row as f64 - mid_idx) / mid_idx
            } else {
                0.0
            };
            let arc_dx = (1.0 - t * t) * 34.0;
            nodes[idx].x = next_x + (col as f64) * (FILE_W + 30.0) + arc_dx;
            nodes[idx].y = MARGIN_Y + (row as f64) * (FILE_H + NODE_GAP_Y);
        }
        col_h
    } else {
        0.0
    };

    // 4. Vertically center each track around the tallest track so small/medium graphs feel balanced.
    let target_span = session_span_h.max(atom_span_h).max(file_span_h);
    let recenter = |indices: &[usize], span: f64, nodes: &mut [CanvasNode]| {
        if indices.is_empty() || target_span <= span + 1.0 {
            return;
        }
        let dy = (target_span - span) * 0.5;
        for &idx in indices {
            nodes[idx].y += dy;
        }
    };
    recenter(&session_indices, session_span_h, &mut nodes);
    recenter(&atom_indices, atom_span_h, &mut nodes);
    recenter(&file_indices, file_span_h, &mut nodes);

    // 5. Deterministic AABB anti-overlap pass guaranteeing minimum spacing between all cards.
    resolve_node_overlaps(&mut nodes);

    // 6. Normalize coordinates so top/left margins stay crisp.
    let min_x = nodes.iter().map(|n| n.x).fold(f64::INFINITY, f64::min);
    let min_y = nodes.iter().map(|n| n.y).fold(f64::INFINITY, f64::min);
    let shift_x = MARGIN_X - min_x;
    let shift_y = MARGIN_Y - min_y;
    for n in &mut nodes {
        n.x += shift_x;
        n.y += shift_y;
    }

    let max_x = nodes.iter().map(|n| n.x + n.w).fold(0.0, f64::max);
    let max_y = nodes.iter().map(|n| n.y + n.h).fold(0.0, f64::max);
    let right_pad = MARGIN_X + if file_indices.is_empty() { 0.0 } else { 44.0 };
    let width = (max_x + right_pad).max(MIN_PLANE_W);
    let height = (max_y + MARGIN_Y).max(MIN_PLANE_H);
    (nodes, width, height)
}

fn resolve_node_overlaps(nodes: &mut [CanvasNode]) {
    let mut order: Vec<usize> = (0..nodes.len()).collect();
    for _pass in 0..8 {
        let mut moved = false;
        order.sort_by(|&a, &b| {
            nodes[a]
                .y
                .total_cmp(&nodes[b].y)
                .then_with(|| nodes[a].x.total_cmp(&nodes[b].x))
                .then_with(|| nodes[a].id.cmp(&nodes[b].id))
        });
        for i_pos in 0..order.len() {
            let i = order[i_pos];
            for &j in &order[(i_pos + 1)..] {
                let x_overlap = nodes[i].x < nodes[j].x + nodes[j].w + NODE_GAP_X
                    && nodes[j].x < nodes[i].x + nodes[i].w + NODE_GAP_X;
                if !x_overlap {
                    continue;
                }
                let min_y = nodes[i].y + nodes[i].h + NODE_GAP_Y;
                if nodes[j].y < min_y {
                    nodes[j].y = min_y;
                    moved = true;
                }
            }
        }
        if !moved {
            break;
        }
    }
}

fn render_orbital_guides(nodes: &[CanvasNode], width: f64, height: f64) -> String {
    let mut svg = String::new();
    for (group, label) in [
        (NodeKindGroup::Session, "SESSIONS"),
        (NodeKindGroup::Atom, "CONTEXT ATOMS"),
        (NodeKindGroup::File, "FILE ANCHORS"),
    ] {
        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        let mut count = 0usize;
        for n in nodes.iter().filter(|n| n.group == group) {
            min_x = min_x.min(n.x);
            max_x = max_x.max(n.x + n.w);
            min_y = min_y.min(n.y);
            max_y = max_y.max(n.y + n.h);
            count += 1;
        }
        if count == 0 {
            continue;
        }
        let cx = (min_x + max_x) * 0.5;
        let pad_top = (min_y - 24.0).max(20.0);
        let pad_bot = (max_y + 18.0).min(height - 16.0);
        let curve_x = match group {
            NodeKindGroup::Session => (cx - 26.0).max(16.0),
            NodeKindGroup::Atom => cx,
            NodeKindGroup::File => (cx + 26.0).min(width - 16.0),
        };
        svg.push_str(&format!(
            "<path class=\"omni-orbit-ring\" data-track=\"{}\" d=\"M {cx:.1} {pad_top:.1} Q {curve_x:.1} {:.1}, {cx:.1} {pad_bot:.1}\"/>",
            group.as_str(),
            (pad_top + pad_bot) * 0.5,
        ));
        svg.push_str(&format!(
            "<text class=\"omni-orbit-label\" data-track=\"{}\" x=\"{cx:.1}\" y=\"{:.1}\" text-anchor=\"middle\">{label}</text>",
            group.as_str(),
            (pad_top - 4.0).max(16.0),
        ));
    }
    svg
}

fn render_edge_marker_defs() -> String {
    let mut defs = String::new();
    for spec in EDGE_SPECS {
        defs.push_str(&format!(
            "<marker id=\"omni-arrow-{}\" class=\"omni-arrow omni-arrow-{}\" viewBox=\"0 0 8 8\" refX=\"7\" refY=\"4\" markerWidth=\"7\" markerHeight=\"7\" markerUnits=\"userSpaceOnUse\" orient=\"auto\">{}</marker>",
            spec.key, spec.key, spec.marker_body,
        ));
    }
    defs
}

struct CubicCurve {
    p0: (f64, f64),
    p1: (f64, f64),
    p2: (f64, f64),
    p3: (f64, f64),
}

fn route_bezier_ports(
    u: &CanvasNode,
    v: &CanvasNode,
    slot: usize,
    total: usize,
    edge_idx: usize,
) -> CubicCurve {
    let ucx = u.x + u.w * 0.5;
    let ucy = u.y + u.h * 0.5;
    let vcx = v.x + v.w * 0.5;
    let vcy = v.y + v.h * 0.5;

    let lane_offset = if total > 1 {
        let centered = slot as f64 - (total as f64 - 1.0) * 0.5;
        centered * 22.0
    } else {
        0.0
    };

    if u.id == v.id {
        let x0 = u.x + u.w;
        let y0 = ucy - 10.0;
        let y3 = ucy + 10.0;
        return CubicCurve {
            p0: (x0, y0),
            p1: (x0 + 54.0, y0 - 28.0),
            p2: (x0 + 54.0, y3 + 28.0),
            p3: (x0, y3),
        };
    }

    if vcx > ucx + u.w * 0.48 {
        let p0 = (u.x + u.w, ucy);
        let p3 = (v.x, vcy);
        let dx = ((p3.0 - p0.0).abs() * 0.46).max(46.0);
        CubicCurve {
            p0,
            p1: (p0.0 + dx, p0.1 + lane_offset),
            p2: (p3.0 - dx, p3.1 + lane_offset),
            p3,
        }
    } else if ucx > vcx + v.w * 0.48 {
        let p0 = (u.x, ucy);
        let p3 = (v.x + v.w, vcy);
        let dx = ((p0.0 - p3.0).abs() * 0.46).max(46.0);
        CubicCurve {
            p0,
            p1: (p0.0 - dx, p0.1 + lane_offset),
            p2: (p3.0 + dx, p3.1 + lane_offset),
            p3,
        }
    } else {
        let dir = if edge_idx.is_multiple_of(2) {
            1.0
        } else {
            -1.0
        };
        let x0 = if dir > 0.0 { u.x + u.w } else { u.x };
        let x3 = if dir > 0.0 { v.x + v.w } else { v.x };
        let arc = (44.0 + (vcy - ucy).abs() * 0.16 + lane_offset.abs()) * dir;
        CubicCurve {
            p0: (x0, ucy),
            p1: (x0 + arc, ucy),
            p2: (x3 + arc, vcy),
            p3: (x3, vcy),
        }
    }
}

fn bezier_midpoint(curve: &CubicCurve) -> (f64, f64) {
    (
        0.125 * curve.p0.0 + 0.375 * curve.p1.0 + 0.375 * curve.p2.0 + 0.125 * curve.p3.0,
        0.125 * curve.p0.1 + 0.375 * curve.p1.1 + 0.375 * curve.p2.1 + 0.125 * curve.p3.1,
    )
}

fn render_constellation_node(
    node: &CanvasNode,
    selected: bool,
    neighbor: bool,
    highlighted: bool,
    dimmed: bool,
) -> String {
    let kind_label = match node.group {
        NodeKindGroup::Session => "Session",
        NodeKindGroup::File => "File",
        NodeKindGroup::Atom => atom_kind_spec(&node.kind).label,
    };
    let kind_key = match node.group {
        NodeKindGroup::Session => "session",
        NodeKindGroup::File => "file",
        NodeKindGroup::Atom => atom_kind_spec(&node.kind).key,
    };

    let mut indicators = String::new();
    match node.group {
        NodeKindGroup::Atom => {
            if node.pinned {
                let pin_label = if node.pin_priority > 1 {
                    format!("Pin P{}", node.pin_priority)
                } else {
                    "Pinned".to_owned()
                };
                indicators.push_str(&format!(
                    "<span class=\"omni-pin-badge\" title=\"Pinned priority context\">{pin_label}</span>"
                ));
            }
            if let Some(pct) = node.confidence {
                let tier = if pct >= 85 {
                    "high"
                } else if pct >= 60 {
                    "medium"
                } else {
                    "low"
                };
                indicators.push_str(&format!(
                    "<span class=\"omni-confidence\" data-tier=\"{tier}\" title=\"Confidence {pct}%\">{pct}%</span>"
                ));
            }
        }
        NodeKindGroup::Session => {
            if !node.provider.is_empty() {
                indicators.push_str(&format!(
                    "<span class=\"omni-provider-badge\">{}</span>",
                    esc(&node.provider)
                ));
            }
            let status_token = escape_attr_token(&node.status);
            let status_label = title_case_token(&node.status);
            indicators.push_str(&format!(
                "<span class=\"omni-status-badge\" data-status=\"{status_token}\">{}</span>",
                esc(&status_label)
            ));
        }
        NodeKindGroup::File => {
            let suffix = if node.touch_count == 1 { "ref" } else { "refs" };
            indicators.push_str(&format!(
                "<span class=\"omni-file-count\">{} {suffix}</span>",
                node.touch_count
            ));
        }
    }

    let mut foot = String::new();
    let max_tags = node.tags.len().min(2);
    for tag in node.tags.iter().take(max_tags) {
        foot.push_str(&format!(
            "<span class=\"omni-tag-pill\">{}</span>",
            esc(tag)
        ));
    }
    if node.tags.len() > 2 {
        foot.push_str(&format!(
            "<span class=\"omni-tag-more\">+{}</span>",
            node.tags.len() - 2
        ));
    }
    if node.group == NodeKindGroup::Session && !node.shared_space.is_empty() {
        foot.push_str(&format!(
            "<span class=\"omni-space-pill\" title=\"Shared space: {}\"><span aria-hidden=\"true\">⇄</span> {}</span>",
            esc(&node.shared_space),
            esc(&node.shared_space),
        ));
    }
    if !node.branch.is_empty() {
        foot.push_str(&format!(
            "<span class=\"omni-branch-pill\">{}</span>",
            esc(&node.branch)
        ));
    } else if node.group == NodeKindGroup::File && !node.subtitle.is_empty() {
        foot.push_str(&format!(
            "<span class=\"omni-file-dir\">{}</span>",
            esc(&node.subtitle)
        ));
    }

    let aria_label = if node.branch.is_empty() {
        format!("{kind_label}: {}", node.title)
    } else {
        format!("{kind_label}: {} on {}", node.title, node.branch)
    };

    format!(
        "<button type=\"button\" class=\"omni-node omni-node-{} omni-kind-{kind_key}\" style=\"left:{:.1}px;top:{:.1}px;width:{:.1}px;height:{:.1}px\" data-action=\"omnigraph-select-node\" data-node-id=\"{}\" data-node-type=\"{}\" data-kind=\"{kind_key}\" data-branch=\"{}\" data-selected=\"{}\" data-neighbor=\"{}\" data-highlighted=\"{}\" data-dimmed=\"{}\" data-pinned=\"{}\" aria-pressed=\"{}\" aria-label=\"{}\" title=\"{}\"><span class=\"omni-node-head\">{icon}<span class=\"omni-kind-badge\" data-kind=\"{kind_key}\">{kind_label}</span><span class=\"omni-node-indicators\">{indicators}</span></span><span class=\"omni-node-title\">{}</span><span class=\"omni-node-foot\">{foot}</span></button>",
        node.group.as_str(),
        node.x,
        node.y,
        node.w,
        node.h,
        esc(&node.id),
        node.group.as_str(),
        esc(&node.branch),
        bool_str(selected),
        bool_str(neighbor),
        bool_str(highlighted),
        bool_str(dimmed),
        bool_str(node.pinned),
        bool_str(selected),
        esc(&aria_label),
        esc(&aria_label),
        esc(&node.title),
        icon = render_kind_icon(kind_key),
    )
}

fn render_constellation_radar(
    nodes: &[CanvasNode],
    width: f64,
    height: f64,
    vp: &CanvasViewport,
    selected_id: Option<&str>,
    highlighted_ids: &BTreeSet<String>,
) -> String {
    if nodes.is_empty() || width <= 0.0 || height <= 0.0 {
        return String::new();
    }
    let scale_fit = (RADAR_MAX_W / width).min(RADAR_MAX_H / height);
    let rw = (width * scale_fit).round().max(RADAR_MIN_SIDE);
    let rh = (height * scale_fit).round().max(RADAR_MIN_SIDE);

    let vx = (-vp.pan_x / vp.scale).clamp(0.0, width);
    let vy = (-vp.pan_y / vp.scale).clamp(0.0, height);
    let vw = (NOMINAL_VIEWPORT_W / vp.scale)
        .clamp(16.0, width - vx)
        .max(16.0);
    let vh = (NOMINAL_VIEWPORT_H / vp.scale)
        .clamp(16.0, height - vy)
        .max(16.0);

    let mut out = format!(
        "<div class=\"omni-radar\" data-w=\"{width:.0}\" data-h=\"{height:.0}\" role=\"region\" aria-label=\"Constellation minimap\"><svg class=\"omni-radar-svg\" width=\"{rw:.0}\" height=\"{rh:.0}\" viewBox=\"0 0 {width:.0} {height:.0}\" preserveAspectRatio=\"xMidYMid meet\" focusable=\"false\" aria-hidden=\"true\"><rect class=\"omni-radar-frame\" x=\"2\" y=\"2\" width=\"{:.0}\" height=\"{:.0}\" rx=\"10\"/>",
        (width - 4.0).max(10.0),
        (height - 4.0).max(10.0),
    );

    for node in nodes {
        let cx = node.x + node.w * 0.5;
        let cy = node.y + node.h * 0.5;
        let is_sel = selected_id == Some(node.id.as_str());
        let is_lit = highlighted_ids.contains(&node.id);
        let r = if is_sel {
            13.0
        } else if node.group == NodeKindGroup::Session {
            10.0
        } else if node.group == NodeKindGroup::File {
            7.5
        } else {
            9.0
        };
        out.push_str(&format!(
            "<circle class=\"omni-radar-dot omni-radar-dot-{}\" cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"{r:.1}\" data-node-id=\"{}\" data-node-type=\"{}\" data-kind=\"{}\" data-selected=\"{}\" data-highlighted=\"{}\"/>",
            node.group.as_str(),
            esc(&node.id),
            node.group.as_str(),
            escape_attr_token(&node.kind),
            bool_str(is_sel),
            bool_str(is_lit),
        ));
    }

    out.push_str(&format!(
        "<rect class=\"omni-radar-view\" x=\"{vx:.1}\" y=\"{vy:.1}\" width=\"{vw:.1}\" height=\"{vh:.1}\" rx=\"6\"/></svg></div>"
    ));
    out
}

fn render_constellation_empty(filter: &ConstellationFilter, vp: &CanvasViewport) -> String {
    let has_filter = !filter.query.trim().is_empty()
        || !filter.kind.trim().is_empty()
        || (!filter.branch.trim().is_empty() && filter.branch.trim() != "all");
    let (heading, detail, action, action_label) = if has_filter {
        (
            "No context nodes match your filter",
            "Try clearing the search query, kind pill, or branch filter to see the full constellation.",
            "omnigraph-clear-filters",
            "Reset filters",
        )
    } else {
        (
            "Context constellation is empty",
            "Publish a finding, decision, contract, or start an Omnigraph-enabled session to illuminate shared context.",
            "omnigraph-open-publish",
            "Publish context atom",
        )
    };

    format!(
        "<div class=\"graph-map-plane omni-plane\" data-empty=\"true\" style=\"--graph-pan-x:{:.1}px;--graph-pan-y:{:.1}px;--graph-scale:{:.4};width:{MIN_PLANE_W:.0}px;height:400px\"><div class=\"omni-empty-state\" role=\"status\"><svg class=\"omni-empty-illustration\" viewBox=\"0 0 96 64\" width=\"96\" height=\"64\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.4\" aria-hidden=\"true\"><circle cx=\"22\" cy=\"36\" r=\"7\"/><circle cx=\"48\" cy=\"18\" r=\"8\"/><circle cx=\"74\" cy=\"42\" r=\"6\"/><path d=\"M28 33 L41 22 M55 22 L69 38\" stroke-dasharray=\"3 3\"/></svg><h3 class=\"omni-empty-title\">{heading}</h3><p class=\"omni-empty-body\">{detail}</p><button type=\"button\" class=\"button compact omni-empty-action\" data-action=\"{action}\">{action_label}</button></div></div>",
        vp.pan_x, vp.pan_y, vp.scale
    )
}

struct DagBranch {
    name: String,
    head_commit: String,
    parent_branch: String,
    is_default: bool,
    ahead_count: usize,
}

struct DagCommit {
    id: String,
    branch: String,
    parents: Vec<String>,
    merged_from: String,
    actor: String,
    summary: String,
    created_at: String,
    atom_delta: i64,
}

fn extract_dag_data(graph: &Value, active_branch: &str) -> (Vec<DagBranch>, Vec<DagCommit>) {
    let mut branches = Vec::new();
    let mut seen_branches: BTreeSet<String> = BTreeSet::new();

    if let Some(arr) = graph.get("branches").and_then(Value::as_array) {
        for item in arr {
            let (name, head, parent, is_default, ahead) = if let Some(s) = item.as_str() {
                (
                    s.trim().to_owned(),
                    String::new(),
                    String::new(),
                    s.trim() == "main",
                    0,
                )
            } else {
                let name = str_field(item, &["name", "branch", "id"]);
                let head = str_field(
                    item,
                    &["head_commit", "head_commit_id", "head", "commit_id"],
                )
                .if_empty_from(|| {
                    item.get("head")
                        .map(|h| str_field(h, &["graph_commit_id", "id", "commit_id"]))
                        .unwrap_or_default()
                });
                let parent = str_field(
                    item,
                    &["parent_branch", "from", "base_branch", "forked_from"],
                );
                let is_default = name == "main" || bool_field(item, &["is_default", "default"]);
                let ahead = i64_field(item, &["ahead", "ahead_count", "unmerged_count"])
                    .unwrap_or(0)
                    .max(0) as usize;
                (name, head, parent, is_default, ahead)
            };
            if name.is_empty() || !seen_branches.insert(name.clone()) {
                continue;
            }
            branches.push(DagBranch {
                name,
                head_commit: head,
                parent_branch: parent,
                is_default,
                ahead_count: ahead,
            });
        }
    }

    if !seen_branches.contains("main") {
        branches.insert(
            0,
            DagBranch {
                name: "main".to_owned(),
                head_commit: String::new(),
                parent_branch: String::new(),
                is_default: true,
                ahead_count: 0,
            },
        );
        seen_branches.insert("main".to_owned());
    }
    if !active_branch.is_empty() && !seen_branches.contains(active_branch) {
        branches.push(DagBranch {
            name: active_branch.to_owned(),
            head_commit: String::new(),
            parent_branch: "main".to_owned(),
            is_default: false,
            ahead_count: 0,
        });
    }

    let mut commits = Vec::new();
    if let Some(arr) = graph.get("commits").and_then(Value::as_array) {
        for item in arr {
            let id = str_field(item, &["id", "graph_commit_id", "commit_id"]);
            if id.is_empty() {
                continue;
            }
            let branch =
                str_field(item, &["branch", "source_branch", "graph_branch"]).if_empty("main");
            let mut parents = string_list(item, &["parents", "parent_ids"]);
            if parents.is_empty() {
                let single = str_field(item, &["parent_commit_id", "parent", "parent_id"]);
                if !single.is_empty() {
                    parents.push(single);
                }
            }
            let merged_parent = str_field(item, &["merged_parent_commit_id"]);
            if !merged_parent.is_empty() && !parents.contains(&merged_parent) {
                parents.push(merged_parent);
            }
            let merged_from = str_field(item, &["merged_from", "source_branch"]);
            let actor = str_field(item, &["actor", "actor_id"]).if_empty("harness");
            let summary = str_field(item, &["summary", "message", "title", "kind"])
                .if_empty("Context checkpoint");
            let created_at = str_field(item, &["created_at", "authored_at", "timestamp"])
                .if_empty_from(|| {
                    i64_field(item, &["created_at", "authored_at", "timestamp"])
                        .map(format_timestamp_i64)
                        .unwrap_or_default()
                });
            let atom_delta = i64_field(
                item,
                &[
                    "atom_delta",
                    "delta_atoms",
                    "atoms_added",
                    "added_count",
                    "changes_count",
                ],
            )
            .unwrap_or(0);

            commits.push(DagCommit {
                id,
                branch,
                parents,
                merged_from,
                actor,
                summary,
                created_at,
                atom_delta,
            });
        }
    }

    (branches, commits)
}

fn render_kind_icon(kind: &str) -> String {
    let body = match kind {
        "session" => SESSION_SVG_BODY,
        "file" => FILE_SVG_BODY,
        other => atom_kind_spec(other).svg_body,
    };
    format!(
        "<svg class=\"omni-kind-icon\" viewBox=\"0 0 16 16\" width=\"14\" height=\"14\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.5\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\">{body}</svg>"
    )
}

fn atom_kind_spec(raw: &str) -> &'static KindVisual {
    let lower = raw.trim().to_ascii_lowercase();
    ATOM_KINDS
        .iter()
        .find(|spec| spec.key == lower)
        .unwrap_or(&ATOM_KINDS[4]) // insight fallback
}

fn edge_spec(key: &str) -> &'static EdgeVisualSpec {
    EDGE_SPECS
        .iter()
        .find(|spec| spec.key == key)
        .unwrap_or(&EDGE_SPECS[8]) // relates_to fallback
}

fn format_timestamp_i64(raw: i64) -> String {
    if raw <= 0 {
        return String::new();
    }
    let secs = if raw >= 100_000_000_000_000 {
        raw / 1_000_000
    } else if raw >= 100_000_000_000 {
        raw / 1_000
    } else {
        raw
    };
    if secs < 86_400 {
        return format!("{secs}s");
    }
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let hour = rem / 3_600;
    let minute = (rem % 3_600) / 60;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02} UTC")
}

fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m as u32, d as u32)
}

fn normalize_edge_relation(raw_rel: &str, raw_subkind: &str) -> (String, String, String, String) {
    let rel_norm = to_snake_token(raw_rel);
    let sub_norm = to_snake_token(raw_subkind);

    let canonical_rel = match rel_norm.as_str() {
        "published" | "published_by" => "published_by",
        "consumed" | "consumed_by" => "consumed_by",
        "touches" | "touches_file" | "file" => "touches_file",
        "mentions" | "mentions_symbol" => "mentions_symbol",
        "derived" | "derived_from" => "derived_from",
        "follows" | "session_follows" => "session_follows",
        "supports" | "contradicts" | "supersedes" | "depends_on" | "refines" => "relates_to",
        _ => "relates_to",
    };

    let effective_sub = if !sub_norm.is_empty() {
        sub_norm
    } else if matches!(
        rel_norm.as_str(),
        "supports" | "contradicts" | "supersedes" | "depends_on" | "refines"
    ) {
        rel_norm.clone()
    } else {
        String::new()
    };

    let arrow_key = if canonical_rel == "relates_to"
        && matches!(
            effective_sub.as_str(),
            "supports" | "contradicts" | "supersedes" | "depends_on" | "refines"
        ) {
        effective_sub.clone()
    } else {
        canonical_rel.to_owned()
    };

    let label = edge_spec(&arrow_key).label.to_owned();
    (canonical_rel.to_owned(), effective_sub, arrow_key, label)
}

fn normalize_outcome(raw: &str, has_conflicts: bool, all_empty: bool) -> &'static str {
    if has_conflicts {
        return "conflict";
    }
    match raw.trim().to_ascii_lowercase().as_str() {
        "conflict" | "conflicts" | "409" => "conflict",
        "fast_forward" | "fast-forward" | "ff" => "fast_forward",
        "merged" | "clean" => "merged",
        "already_up_to_date" | "up_to_date" | "uptodate" | "noop" => "already_up_to_date",
        _ if all_empty => "already_up_to_date",
        _ => "merged",
    }
}

fn sanitize_viewport(vp: &CanvasViewport) -> CanvasViewport {
    let scale = if vp.scale.is_finite() && vp.scale > 0.0 {
        vp.scale.clamp(0.15, 4.0)
    } else {
        1.0
    };
    let pan_x = if vp.pan_x.is_finite() { vp.pan_x } else { 0.0 };
    let pan_y = if vp.pan_y.is_finite() { vp.pan_y } else { 0.0 };
    CanvasViewport {
        pan_x,
        pan_y,
        scale,
    }
}

fn ordered_pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_owned(), b.to_owned())
    } else {
        (b.to_owned(), a.to_owned())
    }
}

fn split_file_label(path: &str) -> (String, String) {
    let trimmed = path.trim();
    match trimmed.rsplit_once('/') {
        Some((dir, file)) if !file.is_empty() => (file.to_owned(), dir.to_owned()),
        _ => (trimmed.to_owned(), String::new()),
    }
}

fn format_diff_value(val: Option<&Value>) -> String {
    match val {
        None => "—".to_owned(),
        Some(Value::Null) => "(deleted)".to_owned(),
        Some(Value::String(s)) => {
            if s.trim().is_empty() {
                "(empty)".to_owned()
            } else {
                s.clone()
            }
        }
        Some(Value::Object(map)) => {
            let mut lines = Vec::new();
            for key in [
                "title",
                "kind",
                "summary",
                "body",
                "confidence",
                "tags",
                "status",
            ] {
                if let Some(v) = map.get(key) {
                    if !v.is_null() {
                        lines.push(format!("{key}: {}", compact_val(v)));
                    }
                }
            }
            if lines.is_empty() {
                serde_json::to_string_pretty(map).unwrap_or_default()
            } else {
                lines.join("\n")
            }
        }
        Some(other) => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    }
}

fn compact_val(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(arr) => arr.iter().map(compact_val).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

fn pick_first_value<'a>(obj: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|k| obj.get(*k))
}

fn array_items(obj: &Value, keys: &[&str]) -> Vec<Value> {
    for k in keys {
        if let Some(arr) = obj.get(*k).and_then(Value::as_array) {
            return arr.clone();
        }
    }
    Vec::new()
}

fn str_field(v: &Value, keys: &[&str]) -> String {
    for k in keys {
        if let Some(s) = v.get(*k).and_then(Value::as_str) {
            let t = s.trim();
            if !t.is_empty() {
                return t.to_owned();
            }
        }
    }
    String::new()
}

fn bool_field(v: &Value, keys: &[&str]) -> bool {
    keys.iter()
        .any(|k| v.get(*k).and_then(Value::as_bool) == Some(true))
}

fn f64_field(v: &Value, keys: &[&str]) -> Option<f64> {
    for k in keys {
        if let Some(val) = v.get(*k) {
            if let Some(n) = val.as_f64().filter(|n| n.is_finite()) {
                return Some(n);
            }
            if let Some(s) = val.as_str() {
                if let Ok(n) = s.trim().parse::<f64>() {
                    if n.is_finite() {
                        return Some(n);
                    }
                }
            }
        }
    }
    None
}

fn i64_field(v: &Value, keys: &[&str]) -> Option<i64> {
    for k in keys {
        if let Some(val) = v.get(*k) {
            if let Some(n) = val.as_i64() {
                return Some(n);
            }
            if let Some(n) = val.as_u64() {
                return i64::try_from(n).ok();
            }
        }
    }
    None
}

fn string_list(v: &Value, keys: &[&str]) -> Vec<String> {
    for k in keys {
        if let Some(arr) = v.get(*k).and_then(Value::as_array) {
            let mut out = Vec::new();
            for item in arr {
                if let Some(s) = item.as_str() {
                    let t = s.trim();
                    if !t.is_empty() {
                        out.push(t.to_owned());
                    }
                } else {
                    let s = str_field(item, &["name", "id", "path"]);
                    if !s.is_empty() {
                        out.push(s);
                    }
                }
            }
            return out;
        }
    }
    Vec::new()
}

fn short_hash(id: &str) -> String {
    let trimmed = id.trim();
    let core = trimmed.strip_prefix("hb1.").unwrap_or(trimmed);
    let len = core.chars().count();
    if len <= 10 {
        core.to_owned()
    } else {
        let tail: String = core
            .chars()
            .rev()
            .take(8)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("…{tail}")
    }
}

fn title_case_token(raw: &str) -> String {
    let mut words = Vec::new();
    for part in raw.trim().split(['_', '-']) {
        if part.is_empty() {
            continue;
        }
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            let mut word = first.to_uppercase().collect::<String>();
            word.push_str(&chars.as_str().to_ascii_lowercase());
            words.push(word);
        }
    }
    if words.is_empty() {
        "Unknown".to_owned()
    } else {
        words.join(" ")
    }
}

fn to_snake_token(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut prev_sep = false;
    for ch in raw.trim().chars() {
        if ch == '-' || ch == ' ' || ch == '_' {
            if !prev_sep && !out.is_empty() {
                out.push('_');
                prev_sep = true;
            }
        } else if ch.is_ascii_uppercase() {
            if !out.is_empty() && !prev_sep {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
            prev_sep = false;
        } else if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_sep = false;
        }
    }
    out.trim_matches('_').to_owned()
}

fn escape_attr_token(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "unknown".to_owned()
    } else {
        out
    }
}

fn bool_str(v: bool) -> &'static str {
    if v {
        "true"
    } else {
        "false"
    }
}

fn esc(raw: &str) -> String {
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

trait StringFallbackExt {
    fn if_empty(self, fallback: &str) -> String;
    fn if_empty_from(self, fallback: impl FnOnce() -> String) -> String;
}

impl StringFallbackExt for String {
    fn if_empty(self, fallback: &str) -> String {
        if self.is_empty() {
            fallback.to_owned()
        } else {
            self
        }
    }

    fn if_empty_from(self, fallback: impl FnOnce() -> String) -> String {
        if self.is_empty() {
            fallback()
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_graph() -> Value {
        json!({
            "graph_id": "harness-context",
            "active_branch": "session/s1",
            "branches": [
                {"name": "main", "head_commit": "c100", "is_default": true},
                {"name": "session/s1", "head_commit": "c102", "parent_branch": "main", "ahead_count": 2},
                {"name": "session/s2", "head_commit": "c103", "parent_branch": "main", "ahead_count": 1}
            ],
            "commits": [
                {"id": "c103", "branch": "session/s2", "parents": ["c100"], "actor": "codex-s2", "summary": "Add JWT rotation warning", "atom_delta": 1, "created_at": "18:04"},
                {"id": "c102", "branch": "session/s1", "parents": ["c101"], "actor": "claude-s1", "summary": "Record auth contract & insight", "atom_delta": 2, "created_at": "18:02"},
                {"id": "c101", "branch": "session/s1", "parents": ["c100"], "actor": "claude-s1", "summary": "Initial token finding", "atom_delta": 1, "created_at": "18:01"},
                {"id": "c100", "branch": "main", "parents": [], "actor": "harness", "summary": "Bootstrap context baseline", "atom_delta": 1, "created_at": "18:00"}
            ],
            "sessions": [
                {"id": "sess-1", "title": "Auth Refactor", "branch": "session/s1", "status": "running", "provider": "claude"},
                {"id": "sess-2", "title": "Security Audit", "branch": "session/s2", "status": "completed", "provider": "codex"}
            ],
            "atoms": [
                {
                    "id": "atom-1",
                    "kind": "finding",
                    "title": "Refresh tokens lack expiry check",
                    "summary": "VerifyRefreshToken skips leeway check",
                    "branch": "session/s1",
                    "session_id": "sess-1",
                    "confidence": 0.94,
                    "pinned": true,
                    "pin_priority": 2,
                    "tags": ["auth", "jwt", "security"]
                },
                {
                    "id": "atom-2",
                    "kind": "decision",
                    "title": "Use RS256 asymmetric keys",
                    "branch": "main",
                    "confidence": 88,
                    "tags": ["crypto"]
                },
                {
                    "id": "atom-3",
                    "kind": "warning",
                    "title": "Legacy HS256 fallback still active",
                    "branch": "session/s2",
                    "session_id": "sess-2",
                    "confidence": 0.72,
                    "tags": ["migration"]
                },
                {
                    "id": "atom-4",
                    "kind": "contract",
                    "title": "SessionHeader schema v2",
                    "branch": "session/s1",
                    "confidence": 0.99
                },
                {
                    "id": "atom-5",
                    "kind": "insight",
                    "title": "Middleware caches JWKS for 5m",
                    "branch": "main"
                },
                {
                    "id": "atom-6",
                    "kind": "artifact",
                    "title": "auth_contract.schema.json",
                    "branch": "session/s1"
                },
                {
                    "id": "atom-7",
                    "kind": "question",
                    "title": "Should refresh revoke old families?",
                    "branch": "session/s2"
                }
            ],
            "files": [
                {"id": "file:src/auth/jwt.rs", "path": "src/auth/jwt.rs", "branch": "session/s1"},
                {"id": "file:src/middleware.rs", "path": "src/middleware.rs", "branch": "main"}
            ],
            "edges": [
                {"from": "atom-1", "to": "sess-1", "relation": "published_by"},
                {"from": "atom-1", "to": "sess-2", "relation": "consumed_by"},
                {"from": "atom-1", "to": "src/auth/jwt.rs", "relation": "touches_file"},
                {"from": "atom-2", "to": "atom-1", "relation": "relates_to", "subkind": "supports"},
                {"from": "atom-3", "to": "atom-2", "relation": "relates_to", "subkind": "contradicts"},
                {"from": "atom-4", "to": "atom-2", "relation": "relates_to", "subkind": "supersedes"},
                {"from": "atom-6", "to": "atom-4", "relation": "relates_to", "subkind": "depends_on"},
                {"from": "atom-5", "to": "file:src/middleware.rs", "relation": "touches_file"}
            ]
        })
    }

    #[test]
    fn constellation_renders_all_kinds_edges_icons_and_non_overlapping_layout() {
        let graph = sample_graph();
        let filter = ConstellationFilter {
            show_files: true,
            show_sessions: true,
            selected_id: Some("atom-1".to_owned()),
            ..ConstellationFilter::default()
        };
        let viewport = CanvasViewport {
            pan_x: -24.0,
            pan_y: 12.0,
            scale: 1.15,
        };
        let res = render_constellation(&graph, &filter, &viewport);
        assert_eq!(res.node_count, 11);
        assert_eq!(res.edge_count, 8);
        assert_eq!(res.positions.len(), 11);
        assert!(res.width >= MIN_PLANE_W);
        assert!(res.height >= MIN_PLANE_H);

        // Plane CSS variables and attributes
        assert!(res.html.contains(
            "<div class=\"graph-map-plane omni-plane\" style=\"--graph-pan-x:-24.0px;--graph-pan-y:12.0px;--graph-scale:1.1500;"
        ));
        assert!(res.html.contains("<svg class=\"omni-svg\""));
        assert!(res.html.contains("<linearGradient id=\"omni-flow-0\""));

        // Distinct marker definitions and mid-edge pill labels when zoomed in
        for marker in [
            "omni-arrow-published_by",
            "omni-arrow-consumed_by",
            "omni-arrow-touches_file",
            "omni-arrow-supports",
            "omni-arrow-contradicts",
            "omni-arrow-supersedes",
            "omni-arrow-depends_on",
        ] {
            assert!(res.html.contains(marker), "missing marker {marker}");
        }
        assert!(res.html.contains("class=\"omni-edge-label\""));

        // Every kind displays both SVG icon and text badge (WCAG 1.4.1), tags capped at 2 + "+1", and branch pill
        for badge in [
            "Finding", "Decision", "Warning", "Contract", "Insight", "Artifact", "Question",
            "Session", "File",
        ] {
            assert!(res.html.contains(badge), "missing kind badge {badge}");
        }
        assert!(res.html.contains("Pin P2"));
        assert!(res.html.contains("94%"));
        assert!(res
            .html
            .contains("<span class=\"omni-tag-pill\">auth</span>"));
        assert!(res
            .html
            .contains("<span class=\"omni-tag-pill\">jwt</span>"));
        assert!(!res
            .html
            .contains("<span class=\"omni-tag-pill\">security</span>"));
        assert!(res.html.contains("<span class=\"omni-tag-more\">+1</span>"));

        // Selection and neighbor highlighting
        assert!(res.html.contains("data-node-id=\"atom-1\" data-node-type=\"atom\" data-kind=\"finding\" data-branch=\"session/s1\" data-selected=\"true\" data-neighbor=\"false\""));
        assert!(res.html.contains("data-node-id=\"sess-1\" data-node-type=\"session\" data-kind=\"session\" data-branch=\"session/s1\" data-selected=\"false\" data-neighbor=\"true\""));
        assert!(res.html.contains("data-node-id=\"atom-5\" data-node-type=\"atom\" data-kind=\"insight\" data-branch=\"main\" data-selected=\"false\" data-neighbor=\"false\" data-highlighted=\"false\" data-dimmed=\"true\""));

        // Radar minimap contains dots and viewport rectangle
        assert!(res.radar_html.contains("class=\"omni-radar\""));
        assert_eq!(
            res.radar_html.matches("class=\"omni-radar-dot ").count(),
            11
        );
        assert!(res.radar_html.contains("class=\"omni-radar-view\""));

        // Track ordering: sessions left of atoms, atoms left of files
        let sess_x = res.positions["sess-1"].0;
        let atom_x = res.positions["atom-1"].0;
        let file_x = res.positions["file:src/auth/jwt.rs"].0;
        assert!(sess_x < atom_x, "sess_x={sess_x} atom_x={atom_x}");
        assert!(atom_x < file_x, "atom_x={atom_x} file_x={file_x}");

        // Zero inline color hexes anywhere in emitted HTML or radar
        assert_no_inline_hex_colors(&res.html);
        assert_no_inline_hex_colors(&res.radar_html);
    }

    fn assert_no_inline_hex_colors(markup: &str) {
        let bytes = markup.as_bytes();
        for (idx, &b) in bytes.iter().enumerate() {
            if b == b'#' {
                let prev = idx.checked_sub(1).and_then(|p| bytes.get(p)).copied();
                let next = bytes.get(idx + 1).copied();
                // Allow HTML entity &#39; and SVG fragment url(#omni-...)
                let is_entity = prev == Some(b'&');
                let is_url_ref = idx >= 4 && &bytes[idx - 4..idx] == b"url(";
                if !is_entity && !is_url_ref && next.is_some_and(|c| c.is_ascii_hexdigit()) {
                    panic!(
                        "unexpected inline hex color in markup around byte {idx}: {}",
                        &markup[idx..markup.len().min(idx + 16)]
                    );
                }
            }
        }
    }

    #[test]
    fn constellation_layout_prevents_card_overlaps_for_dense_graphs() {
        let mut atoms = Vec::new();
        for i in 0..24 {
            let kind = ATOM_KINDS[i % ATOM_KINDS.len()].key;
            atoms.push(json!({
                "id": format!("a-{i}"),
                "kind": kind,
                "title": format!("Atom {i}"),
                "branch": "main"
            }));
        }
        let graph = json!({
            "sessions": [
                {"id": "s1", "title": "S1", "branch": "main"},
                {"id": "s2", "title": "S2", "branch": "main"}
            ],
            "atoms": atoms,
            "files": [
                {"id": "f1", "path": "src/a.rs"},
                {"id": "f2", "path": "src/b.rs"}
            ],
            "edges": []
        });
        let filter = ConstellationFilter {
            show_files: true,
            show_sessions: true,
            ..ConstellationFilter::default()
        };
        let res = render_constellation(&graph, &filter, &CanvasViewport::default());
        assert_eq!(res.node_count, 28);

        // Verify all atom centers are spaced by at least ATOM_W/ATOM_H when in same band
        let atom_boxes: Vec<(f64, f64)> =
            (0..24).map(|i| res.positions[&format!("a-{i}")]).collect();
        for i in 0..atom_boxes.len() {
            for j in (i + 1)..atom_boxes.len() {
                let dx = (atom_boxes[i].0 - atom_boxes[j].0).abs();
                let dy = (atom_boxes[i].1 - atom_boxes[j].1).abs();
                assert!(
                    dx >= ATOM_W + NODE_GAP_X - 2.0 || dy >= ATOM_H + NODE_GAP_Y - 2.0,
                    "overlap between a-{i} {:?} and a-{j} {:?}",
                    atom_boxes[i],
                    atom_boxes[j]
                );
            }
        }
    }

    #[test]
    fn constellation_escapes_untrusted_text_and_renders_empty_state() {
        let xss_graph = json!({
            "sessions": [],
            "atoms": [
                {
                    "id": "atom\"<xss>",
                    "kind": "finding",
                    "title": "<script>alert('xss')</script>",
                    "branch": "session/\"evil\"",
                    "tags": ["<img onerror=1>"]
                }
            ],
            "files": [],
            "edges": []
        });
        let res = render_constellation(
            &xss_graph,
            &ConstellationFilter::default(),
            &CanvasViewport::default(),
        );
        assert!(!res.html.contains("<script>"));
        assert!(!res.html.contains("<img"));
        assert!(res
            .html
            .contains("&lt;script&gt;alert(&#39;xss&#39;)&lt;/script&gt;"));
        assert!(res.html.contains("data-node-id=\"atom&quot;&lt;xss&gt;\""));

        // Filtering out everything yields graceful empty state
        let empty = render_constellation(
            &xss_graph,
            &ConstellationFilter {
                query: "definitely-not-here".to_owned(),
                ..ConstellationFilter::default()
            },
            &CanvasViewport::default(),
        );
        assert_eq!(empty.node_count, 0);
        assert_eq!(empty.radar_html, "");
        assert!(empty.html.contains("omni-empty-state"));
        assert!(empty
            .html
            .contains("data-action=\"omnigraph-clear-filters\""));
    }

    #[test]
    fn branch_dag_renders_multi_lane_rails_fork_merge_and_snapshot_highlight() {
        let mut graph = sample_graph();
        // Add a merge commit on main
        graph["commits"].as_array_mut().unwrap().insert(
            0,
            json!({
                "id": "c104",
                "branch": "main",
                "parents": ["c100", "c102"],
                "merged_from": "session/s1",
                "actor": "harness",
                "summary": "Merge session/s1 into main",
                "atom_delta": 3,
                "created_at": "18:06"
            }),
        );
        let html = render_branch_dag(&graph, "session/s1", Some("c101"));
        assert!(html.contains("class=\"omni-dag\""));
        assert!(html.contains("data-active-branch=\"session/s1\""));
        assert!(html.contains("data-active-snapshot=\"c101\""));

        // main is lane 0; session/s1 and session/s2 get separate lanes
        assert!(html.contains("data-branch=\"main\" data-lane=\"0\""));
        assert!(html.contains("data-branch=\"session/s1\" data-lane=\"1\""));
        assert!(html.contains("data-branch=\"session/s2\" data-lane=\"2\""));

        // Fork and merge Bezier curves present
        assert!(html.contains("data-kind=\"fork\""));
        assert!(html.contains("data-kind=\"merge\""));

        // Interactive commit buttons + quick branch action buttons without nested buttons
        assert!(html.contains("data-action=\"omnigraph-select-commit\" data-commit-id=\"c101\" data-branch=\"session/s1\" data-active-branch=\"true\" data-snapshot=\"true\""));
        assert!(html.contains("<span class=\"omni-snapshot-badge\">Snapshot</span>"));
        assert!(html.contains("data-action=\"omnigraph-preview-diff\" data-branch=\"session/s1\""));
        assert!(html.contains("data-action=\"omnigraph-switch-branch\" data-branch=\"session/s2\""));
        assert!(html.contains("+3 atoms"));
    }

    #[test]
    fn merge_diff_renders_outcome_changes_and_409_conflicts() {
        let diff = json!({
            "source": "session/s1",
            "target": "main",
            "merge_base": "c100abcdef",
            "outcome": "conflict",
            "added_atoms": [
                {"id": "a-new", "kind": "contract", "title": "Token TTL contract", "summary": "Max 900s", "branch": "session/s1"}
            ],
            "updated_atoms": [
                {
                    "id": "a-upd",
                    "kind": "decision",
                    "title": "Signing algorithm",
                    "before": {"summary": "HS256 shared secret"},
                    "after": {"summary": "RS256 rotated keys"}
                }
            ],
            "deleted_atoms": [
                {"id": "a-del", "kind": "warning", "title": "Deprecated session cookie", "summary": "Removed"}
            ],
            "added_edges": [
                {"from": "a-new", "to": "sess-1", "relation": "published_by"}
            ],
            "conflicts": [
                {
                    "kind": "divergent_update",
                    "entity_id": "atom-2",
                    "field": "summary",
                    "message": "Both session/s1 and main modified atom-2 since c100",
                    "base": "Use symmetric HS256",
                    "source": "Use RS256 with 24h rotation",
                    "target": "Use EdDSA with 12h rotation",
                    "resolution_hint": "Keep target or create superseding decision"
                },
                {
                    "kind": "delete_vs_update",
                    "entity_id": "atom-9",
                    "message": "Source deleted atom-9 while target updated it",
                    "base": {"title": "Old cache policy"},
                    "source": null,
                    "target": {"title": "New 10m cache policy"}
                }
            ]
        });

        let html = render_merge_diff(&diff);
        assert!(html.contains("data-outcome=\"conflict\""));
        assert!(html.contains("409 Conflict"));
        assert!(html.contains("divergent_update"));
        assert!(html.contains("delete_vs_update"));
        assert!(html.contains("Use symmetric HS256"));
        assert!(html.contains("Use RS256 with 24h rotation"));
        assert!(html.contains("Use EdDSA with 12h rotation"));
        assert!(html.contains("(deleted)"));

        // Non-color markers (+ / ~ / -) and kind badges
        assert!(html.contains(
            "<span class=\"omni-diff-mark\" data-change=\"added\" aria-hidden=\"true\">+</span>"
        ));
        assert!(html.contains(
            "<span class=\"omni-diff-mark\" data-change=\"updated\" aria-hidden=\"true\">~</span>"
        ));
        assert!(html.contains(
            "<span class=\"omni-diff-mark\" data-change=\"deleted\" aria-hidden=\"true\">-</span>"
        ));
        assert!(html.contains("Contract"));
        assert!(html.contains("Decision"));
        assert!(html.contains("Warning"));
        assert!(html.contains("published by"));

        // Clean fast-forward outcome
        let ff = json!({
            "source": "session/s2",
            "target": "main",
            "outcome": "fast_forward",
            "added_atoms": [],
            "updated_atoms": [],
            "deleted_atoms": [],
            "added_edges": [],
            "conflicts": []
        });
        let ff_html = render_merge_diff(&ff);
        assert!(ff_html.contains("data-outcome=\"fast_forward\""));
        assert!(ff_html.contains("Fast-Forward Merge Ready"));
        assert_no_inline_hex_colors(&html);
        assert_no_inline_hex_colors(&ff_html);
    }

    #[test]
    fn constellation_filters_toggle_tracks_zoom_labels_and_sanitizes_viewport() {
        let graph = sample_graph();
        // Hide sessions and files, filter to decision kind, zoomed out -> no mid-edge pill labels
        let filter = ConstellationFilter {
            kind: "decision".to_owned(),
            show_files: false,
            show_sessions: false,
            ..ConstellationFilter::default()
        };
        let bad_vp = CanvasViewport {
            pan_x: f64::NAN,
            pan_y: f64::INFINITY,
            scale: 0.0,
        };
        let res = render_constellation(&graph, &filter, &bad_vp);
        assert_eq!(res.node_count, 1);
        assert_eq!(res.edge_count, 0);
        assert!(res
            .html
            .contains("--graph-pan-x:0.0px;--graph-pan-y:0.0px;--graph-scale:1.0000;"));
        assert!(!res.html.contains("NaN"));

        // Zoomed out (<0.95) hides mid-edge pills unless connected to selected/highlighted node
        let zoomed_out = CanvasViewport {
            pan_x: 0.0,
            pan_y: 0.0,
            scale: 0.6,
        };
        let all_filter = ConstellationFilter {
            show_files: true,
            show_sessions: true,
            ..ConstellationFilter::default()
        };
        let far = render_constellation(&graph, &all_filter, &zoomed_out);
        assert!(!far.html.contains("class=\"omni-edge-label\""));

        // Highlighted IDs without selected_id dims non-highlighted nodes
        let mut lit = BTreeSet::new();
        lit.insert("atom-3".to_owned());
        let hi_filter = ConstellationFilter {
            show_files: true,
            show_sessions: true,
            highlighted_ids: lit,
            ..ConstellationFilter::default()
        };
        let hi_res = render_constellation(&graph, &hi_filter, &zoomed_out);
        assert!(hi_res.html.contains("data-node-id=\"atom-3\" data-node-type=\"atom\" data-kind=\"warning\" data-branch=\"session/s2\" data-selected=\"false\" data-neighbor=\"false\" data-highlighted=\"true\" data-dimmed=\"false\""));
        assert!(hi_res.html.contains("data-node-id=\"atom-1\" data-node-type=\"atom\" data-kind=\"finding\" data-branch=\"session/s1\" data-selected=\"false\" data-neighbor=\"false\" data-highlighted=\"false\" data-dimmed=\"true\""));
    }

    #[test]
    fn branch_dag_handles_empty_commits_and_diff_handles_already_up_to_date() {
        let empty_dag =
            render_branch_dag(&json!({"branches": ["main"], "commits": []}), "main", None);
        assert!(empty_dag.contains("class=\"omni-dag\""));
        assert!(empty_dag.contains("class=\"omni-dag-empty\""));
        assert_no_inline_hex_colors(&empty_dag);

        let up_to_date = render_merge_diff(&json!({
            "source": "session/s3",
            "target": "main",
            "outcome": "already_up_to_date"
        }));
        assert!(up_to_date.contains("data-outcome=\"already_up_to_date\""));
        assert!(up_to_date.contains("Already Up to Date"));
        assert!(up_to_date.contains("class=\"omni-diff-empty\""));
        assert_no_inline_hex_colors(&up_to_date);
    }

    #[test]
    fn supports_raw_omnigraph_v015_dag_changes_and_409_merge_conflicts_shapes() {
        let raw_dag = json!({
            "branches": [
                {"name": "main", "head": {"graph_commit_id": "cm-main-2"}},
                {"name": "session/s5", "from": "main", "head": {"graph_commit_id": "cm-s5-1"}}
            ],
            "commits": [
                {
                    "graph_commit_id": "cm-main-2",
                    "graph_branch": "main",
                    "parent_commit_id": "cm-main-1",
                    "merged_parent_commit_id": "cm-s5-1",
                    "actor_id": "act-harness",
                    "kind": "published"
                },
                {
                    "graph_commit_id": "cm-s5-1",
                    "graph_branch": "session/s5",
                    "parent_commit_id": "cm-main-1",
                    "actor_id": "act-session"
                },
                {
                    "graph_commit_id": "cm-main-1",
                    "graph_branch": "main",
                    "actor_id": "act-harness"
                }
            ]
        });
        let dag_html = render_branch_dag(&raw_dag, "session/s5", Some("cm-s5-1"));
        assert!(dag_html.contains("data-branch=\"session/s5\" data-lane=\"1\""));
        assert!(dag_html.contains("data-kind=\"merge\""));
        assert!(dag_html.contains("data-snapshot=\"true\""));
        assert_no_inline_hex_colors(&dag_html);

        let raw_409 = json!({
            "source": "session/s5",
            "target": "main",
            "code": "conflict",
            "merge_conflicts": [
                {
                    "entity_kind": "node",
                    "type_name": "ContextAtom",
                    "entity_id": "atom-42",
                    "kind": "divergent_update",
                    "message": "Conflicting updates on atom-42"
                }
            ],
            "changes": [
                {
                    "kind": "node",
                    "type": {"name": "ContextAtom"},
                    "id": "atom-88",
                    "op": "insert",
                    "after": {"properties": {"kind": "finding", "title": "Inserted from DiffOutput"}}
                },
                {
                    "kind": "edge",
                    "type": {"name": "TouchesFile"},
                    "id": "e-88",
                    "op": "insert",
                    "after": {"endpoints": {"from": "atom-88", "to": "src/lib.rs"}, "properties": {}}
                }
            ]
        });
        let diff_html = render_merge_diff(&raw_409);
        assert!(diff_html.contains("data-outcome=\"conflict\""));
        assert!(diff_html.contains("node · ContextAtom"));
        assert!(diff_html.contains("Inserted from DiffOutput"));
        assert!(diff_html.contains("touches"));
        assert!(!diff_html.contains("class=\"omni-conflict-compare\""));
        assert_no_inline_hex_colors(&diff_html);
    }

    #[test]
    fn preserves_inherited_main_atoms_on_active_branch_and_handles_backend_diff_dag_fields() {
        // 1. BLOCKER #4 & MINOR #7: `source_branch`, `source_session_id`, and active_branch == filter.branch
        let backend_graph = json!({
            "active_branch": "session/s9",
            "branches": [
                {"name": "main", "head_commit_id": "cm-main-1"},
                {"name": "session/s9", "head_commit_id": "cm-s9-1", "parent_branch": "main"}
            ],
            "commits": [
                {
                    "graph_commit_id": "cm-s9-1",
                    "graph_branch": "session/s9",
                    "parent_commit_id": "cm-main-1",
                    "actor_id": "codex",
                    "summary": "Session finding",
                    "created_at": 1_714_000_000_000_000_i64
                },
                {
                    "graph_commit_id": "cm-main-1",
                    "graph_branch": "main",
                    "actor_id": "harness",
                    "summary": "Baseline",
                    "authored_at": 1_713_990_000_i64
                }
            ],
            "sessions": [
                {"id": "s9", "title": "Session Nine", "source_branch": "session/s9", "status": "running"}
            ],
            "atoms": [
                {
                    "atom_id": "a-inherited",
                    "kind": "decision",
                    "title": "Inherited main decision",
                    "source_branch": "main",
                    "source_session_id": "s0"
                },
                {
                    "atom_id": "a-local",
                    "kind": "finding",
                    "title": "Local branch finding",
                    "source_branch": "session/s9",
                    "source_session_id": "s9"
                }
            ],
            "edges": [
                {"from": "a-local", "to": "a-inherited", "relation": "relates_to", "subkind": "refines"},
                {"from": "a-local", "to": "s9", "relation": "published_by"}
            ]
        });

        let branch_filter = ConstellationFilter {
            branch: "session/s9".to_owned(),
            show_sessions: true,
            show_files: true,
            ..ConstellationFilter::default()
        };
        let res = render_constellation(&backend_graph, &branch_filter, &CanvasViewport::default());
        assert_eq!(
            res.node_count, 3,
            "inherited main atom must stay visible when active_branch matches filter.branch"
        );
        assert_eq!(res.edge_count, 2);
        assert!(res.html.contains("data-node-id=\"a-inherited\""));
        assert!(res.html.contains("data-node-id=\"a-local\""));
        assert!(res.html.contains("omni-arrow-refines"));
        assert!(res.html.contains("refines"));

        // Ensure `source_session_id` clusters `a-local` with `s9` and `head_commit_id` + i64 timestamps render in DAG
        let dag_html = render_branch_dag(&backend_graph, "session/s9", None);
        assert!(dag_html.contains("cm-s9-1"));
        assert!(dag_html.contains("2024-04-24 23:06 UTC"));
        assert!(dag_html.contains("2024-04-24 20:20 UTC"));
        assert!(dag_html.contains("class=\"omni-head-badge\">HEAD</span>"));

        // 2. BLOCKER #5: nested `endpoints` + `properties.relation`, `deleted_edges`, and `source_branch` in diff
        let backend_diff = json!({
            "source": "session/s9",
            "target": "main",
            "outcome": "merged",
            "added_atoms": [
                {"id": "a-local", "kind": "finding", "title": "Local branch finding", "source_branch": "session/s9"}
            ],
            "added_edges": [
                {"endpoints": {"from": "a-local", "to": "a-inherited"}, "properties": {"relation": "refines"}}
            ],
            "deleted_edges": [
                {"endpoints": {"from": "a-old", "to": "s0"}, "properties": {"relation": "published_by"}}
            ]
        });
        let diff_html = render_merge_diff(&backend_diff);
        assert!(diff_html.contains("session/s9"));
        assert!(diff_html.contains("<span class=\"omni-diff-edge-rel\">refines</span><code class=\"omni-diff-edge-end\">a-local</code>"));
        assert!(diff_html.contains("<code class=\"omni-diff-edge-end\">a-inherited</code>"));
        assert!(diff_html.contains("<span class=\"omni-diff-edge-rel\">published by</span><code class=\"omni-diff-edge-end\">a-old</code>"));
        assert!(diff_html.contains("<code class=\"omni-diff-edge-end\">s0</code>"));
        assert!(diff_html.contains("data-change=\"deleted\" data-rel=\"published_by\""));
        assert_no_inline_hex_colors(&diff_html);
    }

    #[test]
    fn short_hash_uses_distinguishing_tail_and_file_track_has_extra_right_padding() {
        let hb_dag = render_branch_dag(
            &json!({
                "branches": [{"name": "main", "head_commit_id": "hb1.01M7JWJ7AAAAAAAAAA93XWDY"}],
                "commits": [
                    {
                        "graph_commit_id": "hb1.01M7JWJ7AAAAAAAAAA93XWDY",
                        "graph_branch": "main",
                        "summary": "First"
                    },
                    {
                        "graph_commit_id": "hb1.01M7JWJ7BBBBBBBBBB71KLPQ",
                        "graph_branch": "main",
                        "summary": "Second"
                    }
                ]
            }),
            "main",
            None,
        );
        assert!(hb_dag.contains("…AA93XWDY"));
        assert!(hb_dag.contains("…BB71KLPQ"));
        assert!(!hb_dag.contains(">hb1.01M7<"));

        // File anchors get >= 40px extra right-side canvas padding beyond MARGIN_X
        let wide_graph = json!({
            "sessions": [
                {"id": "s1", "title": "S1", "branch": "main"}
            ],
            "atoms": [
                {"id": "a1", "kind": "finding", "title": "A1", "branch": "main"},
                {"id": "a2", "kind": "decision", "title": "A2", "branch": "main"},
                {"id": "a3", "kind": "contract", "title": "A3", "branch": "main"},
                {"id": "a4", "kind": "insight", "title": "A4", "branch": "main"},
                {"id": "a5", "kind": "warning", "title": "A5", "branch": "main"},
                {"id": "a6", "kind": "artifact", "title": "A6", "branch": "main"}
            ],
            "files": [
                {"id": "f1", "path": "src/main.rs"}
            ],
            "edges": [
                {"from": "a1", "to": "f1", "relation": "touches_file"}
            ]
        });
        let res = render_constellation(
            &wide_graph,
            &ConstellationFilter {
                show_files: true,
                show_sessions: true,
                ..ConstellationFilter::default()
            },
            &CanvasViewport::default(),
        );
        let file_right = res.positions["f1"].0 + FILE_W * 0.5;
        assert!(
            res.width >= file_right + MARGIN_X + 40.0,
            "width {} should leave extra right padding past file_right {}",
            res.width,
            file_right
        );
    }

    #[test]
    fn session_nodes_render_shared_space_pill_and_session_follows_edges() {
        let graph = json!({
            "active_branch": "shared/sprint-auth",
            "sessions": [
                {
                    "id": "s-alpha",
                    "title": "Auth API",
                    "branch": "shared/sprint-auth",
                    "shared_space": "shared/sprint-auth",
                    "provider": "claude",
                    "status": "running"
                },
                {
                    "id": "s-beta",
                    "title": "Auth UI",
                    "omnigraph": { "shared_space": "shared/sprint-auth" },
                    "branch": "shared/sprint-auth",
                    "provider": "codex",
                    "status": "completed"
                }
            ],
            "atoms": [],
            "files": [],
            "edges": [
                { "from": "s-beta", "to": "s-alpha", "relation": "SessionFollows" }
            ]
        });
        let res = render_constellation(
            &graph,
            &ConstellationFilter {
                branch: "shared/sprint-auth".into(),
                show_files: true,
                show_sessions: true,
                ..ConstellationFilter::default()
            },
            &CanvasViewport::default(),
        );
        assert_eq!(res.node_count, 2);
        assert_eq!(res.edge_count, 1);
        assert!(res.html.contains("omni-space-pill"));
        assert!(res.html.contains("shared/sprint-auth"));
        assert!(res.html.contains("omni-edge-session_follows"));
        assert!(res.html.contains("url(#omni-arrow-session_follows)"));
        assert!(res.html.contains(">follows<"));
        assert_no_inline_hex_colors(&res.html);
    }
}
