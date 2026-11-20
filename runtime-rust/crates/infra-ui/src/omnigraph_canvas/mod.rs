//! Interactive visual graph, branch metro DAG, and 3-way merge diff renderer
//! for Omnigraph context sharing across Harness sessions.
//!
//! Pure HTML + SVG string generation (no `web_sys` dependency) so both the
//! `wasm32-unknown-unknown` UI bundle and native unit tests share one deterministic
//! rendering pipeline. All colors come from CSS tokens and classes defined in
//! `harness/web/app.css` (zero inline color hexes), and every status, kind,
//! relation, and diff marker pairs iconography or geometry with explicit text
//! labels for WCAG 1.4.1 compliance.

use crate::graph_viewport;
use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};

pub(super) const SESSION_W: f64 = 216.0;
pub(super) const SESSION_H: f64 = 78.0;
pub(super) const ATOM_W: f64 = 240.0;
pub(super) const ATOM_H: f64 = 96.0;
pub(super) const FILE_W: f64 = 208.0;
pub(super) const FILE_H: f64 = 58.0;
pub(super) const SYMBOL_W: f64 = 208.0;
pub(super) const SYMBOL_H: f64 = 64.0;

pub(super) const MARGIN_X: f64 = 48.0;
pub(super) const MARGIN_Y: f64 = 58.0;
pub(super) const TRACK_GAP_X: f64 = 128.0;
pub(super) const ATOM_COL_GAP_X: f64 = 44.0;
pub(super) const NODE_GAP_X: f64 = 22.0;
pub(super) const NODE_GAP_Y: f64 = 20.0;
pub(super) const CLUSTER_EXTRA_GAP_Y: f64 = 12.0;
pub(super) const MIN_PLANE_W: f64 = 760.0;
pub(super) const MIN_PLANE_H: f64 = 460.0;

pub(super) const RADAR_MAX_W: f64 = 176.0;
pub(super) const RADAR_MAX_H: f64 = 116.0;
pub(super) const RADAR_MIN_SIDE: f64 = 24.0;
pub(super) const NOMINAL_VIEWPORT_W: f64 = 920.0;
pub(super) const NOMINAL_VIEWPORT_H: f64 = 580.0;

pub(super) const DAG_ROW_H: f64 = 56.0;
pub(super) const DAG_LANE_W: f64 = 24.0;
pub(super) const DAG_RAIL_PAD_X: f64 = 20.0;

#[allow(dead_code)]
pub const LOD_LABEL_MIN_SCALE: f64 = 0.95;

pub fn lod(scale: f64) -> &'static str {
    crate::graph_viewport::lod_with(scale, crate::graph_viewport::OMNIGRAPH_LOD)
}

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
    pub anchored_positions: BTreeMap<String, (f64, f64)>,
    pub pinned_positions: BTreeMap<String, (f64, f64)>,
    pub path_nodes: BTreeSet<String>,
    pub path_edges: BTreeSet<(String, String)>,
    pub isolate_seed: Option<String>,
    pub isolate_depth: usize,
    pub viewport_bounds: Option<(f64, f64, f64, f64)>,
    pub paged_count: usize,
    pub focused_id: Option<String>,
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
    pub raw_positions: BTreeMap<String, (f64, f64)>,
    pub structure_digest: u64,
    pub dense_scene: Option<graph_viewport::canvas2d::DenseScene>,
    pub node_rects: Vec<(String, f64, f64, f64, f64)>,
    pub node_edges: Vec<(usize, usize)>,
    pub node_htmls: Vec<String>,
    pub edge_endpoints: Vec<(String, String)>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum NodeKindGroup {
    Session,
    Atom,
    File,
    Symbol,
}

impl NodeKindGroup {
    fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Atom => "atom",
            Self::File => "file",
            Self::Symbol => "symbol",
        }
    }

    fn size(self) -> (f64, f64) {
        match self {
            Self::Session => (SESSION_W, SESSION_H),
            Self::Atom => (ATOM_W, ATOM_H),
            Self::File => (FILE_W, FILE_H),
            Self::Symbol => (SYMBOL_W, SYMBOL_H),
        }
    }
}

pub(super) struct KindVisual {
    key: &'static str,
    label: &'static str,
    rank: usize,
    svg_body: &'static str,
}

pub(super) const ATOM_KINDS: [KindVisual; 7] = [
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

pub(super) const SESSION_SVG_BODY: &str =
    "<rect x=\"1.8\" y=\"2.5\" width=\"12.4\" height=\"11\" rx=\"2.2\"/><path d=\"M4.8 6.2 L7.3 8.1 L4.8 10 M8.4 10 H11.2\"/>";
pub(super) const FILE_SVG_BODY: &str =
    "<path d=\"M3.5 1.8 H9.5 L12.5 4.8 V14.2 H3.5 Z\"/><path d=\"M9.5 1.8 V4.8 H12.5 M5.5 8 H10.5 M5.5 10.8 H9\"/>";
pub(super) const SYMBOL_SVG_BODY: &str =
    "<path d=\"M5.5 4 L2.5 8 L5.5 12 M10.5 4 L13.5 8 L10.5 12 M9.1 3 L6.9 13\"/>";

pub(super) struct EdgeVisualSpec {
    key: &'static str,
    label: &'static str,
    dash_array: Option<&'static str>,
    marker_body: &'static str,
}

pub(super) const EDGE_SPECS: [EdgeVisualSpec; 12] = [
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
pub(super) struct CanvasNode {
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
    file_path: String,
    symbol_id: String,
    symbol_fqn: String,
    symbol_kind: String,
    symbols: Vec<String>,
    confidence: Option<u8>,
    pinned: bool,
    pin_priority: i64,
    tags: Vec<String>,
    touch_count: usize,
    search_blob: String,
    drift_status: String,
    broken_risk: bool,
    user_pinned: bool,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[derive(Clone)]
pub(super) struct CanvasEdge {
    from: String,
    to: String,
    relation: String,
    subkind: String,
    arrow_key: String,
    label: String,
}

pub(super) fn prepare_constellation_graph(graph: &Value) -> (Vec<CanvasNode>, Vec<CanvasEdge>) {
    let (mut nodes, mut edges, aliases) = extract_graph_items(graph);
    let mut resolved_edges = Vec::with_capacity(edges.len());
    let mut touch_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut atom_symbols_from_edges: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for mut edge in edges.drain(..) {
        if let Some(canon_from) = aliases.get(&edge.from) {
            edge.from = canon_from.clone();
        }
        if let Some(canon_to) = aliases.get(&edge.to) {
            edge.to = canon_to.clone();
        }
        *touch_counts.entry(edge.to.clone()).or_default() += 1;
        if edge.relation == "mentions_symbol" {
            let list = atom_symbols_from_edges
                .entry(edge.from.clone())
                .or_default();
            if !list.contains(&edge.to) {
                list.push(edge.to.clone());
            }
        }
        resolved_edges.push(edge);
    }
    for node in &mut nodes {
        if matches!(node.group, NodeKindGroup::File | NodeKindGroup::Symbol)
            && node.touch_count == 0
        {
            node.touch_count = touch_counts.get(&node.id).copied().unwrap_or(0);
        }
        if node.group == NodeKindGroup::Atom {
            if let Some(edge_syms) = atom_symbols_from_edges.get(&node.id) {
                for sym in edge_syms {
                    if !node.symbols.contains(sym) {
                        node.symbols.push(sym.clone());
                    }
                }
            }
        }
    }
    (nodes, resolved_edges)
}

#[allow(dead_code)]
pub fn compute_constellation_cone(
    graph: &Value,
    filter: &ConstellationFilter,
    seed_id: &str,
    max_hops: usize,
) -> BTreeSet<String> {
    let seed = seed_id.trim();
    if seed.is_empty() || max_hops == 0 {
        return BTreeSet::new();
    }
    let (nodes, edges) = prepare_constellation_graph(graph);
    let active_branch = str_field(graph, &["active_branch"]);
    let base_filter = ConstellationFilter {
        isolate_seed: None,
        isolate_depth: 0,
        ..filter.clone()
    };
    let (kept_nodes, kept_edges) =
        apply_constellation_filter(nodes, edges, &base_filter, &active_branch);
    let mut id_to_idx: BTreeMap<String, usize> = BTreeMap::new();
    for (idx, n) in kept_nodes.iter().enumerate() {
        id_to_idx.insert(n.id.clone(), idx);
    }
    let Some(&seed_idx) = id_to_idx.get(seed) else {
        return BTreeSet::new();
    };
    let mut out_adj = vec![Vec::new(); kept_nodes.len()];
    let mut in_adj = vec![Vec::new(); kept_nodes.len()];
    for e in &kept_edges {
        if let (Some(&u), Some(&v)) = (id_to_idx.get(&e.from), id_to_idx.get(&e.to)) {
            out_adj[u].push(v);
            in_adj[v].push(u);
        }
    }
    let dist = graph_viewport::hop_cone_distances(
        kept_nodes.len(),
        &out_adj,
        &in_adj,
        &[seed_idx],
        max_hops.clamp(1, 3),
    );
    kept_nodes
        .into_iter()
        .enumerate()
        .filter(|(idx, _)| dist[*idx].is_some())
        .map(|(_, n)| n.id)
        .collect()
}

pub type ConstellationPath = (Vec<String>, Vec<(String, String)>);

#[allow(dead_code)]
pub fn compute_constellation_path(
    graph: &Value,
    filter: &ConstellationFilter,
    from_query: &str,
    to_query: &str,
) -> Option<ConstellationPath> {
    let fq = from_query.trim().to_ascii_lowercase();
    let tq = to_query.trim().to_ascii_lowercase();
    if fq.is_empty() || tq.is_empty() {
        return None;
    }
    let (nodes, edges) = prepare_constellation_graph(graph);
    let active_branch = str_field(graph, &["active_branch"]);
    let (kept_nodes, kept_edges) = apply_constellation_filter(nodes, edges, filter, &active_branch);
    let resolve = |q: &str| -> Option<usize> {
        kept_nodes
            .iter()
            .position(|n| n.id.eq_ignore_ascii_case(q) || n.title.eq_ignore_ascii_case(q))
            .or_else(|| {
                kept_nodes.iter().position(|n| {
                    n.file_path.eq_ignore_ascii_case(q) || n.symbol_fqn.eq_ignore_ascii_case(q)
                })
            })
            .or_else(|| kept_nodes.iter().position(|n| n.search_blob.contains(q)))
    };
    let u_idx = resolve(&fq)?;
    let v_idx = resolve(&tq)?;
    let id_to_idx: BTreeMap<&str, usize> = kept_nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    let mut adj = vec![Vec::new(); kept_nodes.len()];
    for e in &kept_edges {
        if let (Some(&u), Some(&v)) = (id_to_idx.get(e.from.as_str()), id_to_idx.get(e.to.as_str()))
        {
            adj[u].push(v);
        }
    }
    let path_indices =
        graph_viewport::shortest_path_bfs(kept_nodes.len(), &adj, false, u_idx, v_idx).or_else(
            || graph_viewport::shortest_path_bfs(kept_nodes.len(), &adj, true, u_idx, v_idx),
        )?;
    let path_ids: Vec<String> = path_indices
        .iter()
        .map(|&i| kept_nodes[i].id.clone())
        .collect();
    let path_pairs: Vec<(String, String)> = path_ids
        .windows(2)
        .map(|w| (w[0].clone(), w[1].clone()))
        .collect();
    Some((path_ids, path_pairs))
}

#[allow(dead_code)]
pub fn constellation_structure_digest(graph: &Value, filter: &ConstellationFilter) -> u64 {
    let (nodes, edges) = prepare_constellation_graph(graph);
    let active_branch = str_field(graph, &["active_branch"]);
    let (kept_nodes, kept_edges) = apply_constellation_filter(nodes, edges, filter, &active_branch);
    let mut hasher = DefaultHasher::new();
    kept_nodes.len().hash(&mut hasher);
    for n in &kept_nodes {
        n.id.hash(&mut hasher);
        n.group.as_str().hash(&mut hasher);
        n.drift_status.hash(&mut hasher);
        n.broken_risk.hash(&mut hasher);
        if let Some((px, py)) = filter.pinned_positions.get(&n.id) {
            px.to_bits().hash(&mut hasher);
            py.to_bits().hash(&mut hasher);
        }
    }
    kept_edges.len().hash(&mut hasher);
    for e in &kept_edges {
        e.from.hash(&mut hasher);
        e.to.hash(&mut hasher);
        e.relation.hash(&mut hasher);
    }
    filter.isolate_seed.hash(&mut hasher);
    filter.isolate_depth.hash(&mut hasher);
    filter.paged_count.hash(&mut hasher);
    hasher.finish()
}

#[allow(dead_code)]
pub fn inject_workspace_drift(
    graph: &mut Value,
    changes_doc: Option<&Value>,
    impact_doc: Option<&Value>,
) {
    let mut file_drift: BTreeMap<String, (&'static str, bool)> = BTreeMap::new();
    if let Some(changes) = changes_doc {
        if let Some(arr) = changes["files"]
            .as_array()
            .or_else(|| changes["items"].as_array())
        {
            for item in arr {
                let p = item["path"]
                    .as_str()
                    .or_else(|| item["file"].as_str())
                    .unwrap_or("")
                    .trim();
                if p.is_empty() {
                    continue;
                }
                let broken = item["broken_risk"].as_bool().unwrap_or(false)
                    || item["risk"].as_str() == Some("high")
                    || item["impact"].as_str() == Some("broken_risk")
                    || item["status"].as_str() == Some("deleted");
                let status = if broken {
                    "broken"
                } else if item["stale"].as_bool().unwrap_or(false) {
                    "stale"
                } else {
                    "changed"
                };
                file_drift.insert(p.to_owned(), (status, broken));
            }
        }
        if let Some(risk_callers) = changes["edge_diffs"]["risk_callers"].as_array() {
            for rc in risk_callers {
                let broken = rc["impact"].as_str() == Some("broken_risk")
                    || rc["broken_risk"].as_bool().unwrap_or(false);
                if broken {
                    for key in ["from", "to"] {
                        if let Some(p) = rc[key].as_str().map(str::trim).filter(|s| !s.is_empty()) {
                            file_drift.insert(p.to_owned(), ("broken", true));
                        }
                    }
                }
            }
        }
    }
    if let Some(imp) = impact_doc {
        for key in ["broken", "risks", "suspects"] {
            if let Some(arr) = imp[key].as_array() {
                for item in arr {
                    let p = item["path"]
                        .as_str()
                        .or_else(|| item.as_str())
                        .unwrap_or("")
                        .trim();
                    if !p.is_empty() {
                        file_drift.insert(p.to_owned(), ("broken", true));
                    }
                }
            }
        }
    }
    if file_drift.is_empty() {
        return;
    }
    let apply = |arr_opt: Option<&mut Vec<Value>>| {
        let Some(arr) = arr_opt else {
            return;
        };
        for item in arr.iter_mut() {
            let Some(obj) = item.as_object_mut() else {
                continue;
            };
            let p = obj
                .get("path")
                .or_else(|| obj.get("file_path"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            if let Some(&(status, broken)) = file_drift.get(&p) {
                obj.insert("drift_status".to_owned(), serde_json::json!(status));
                obj.insert("broken_risk".to_owned(), serde_json::json!(broken));
            }
        }
    };
    apply(graph.get_mut("files").and_then(Value::as_array_mut));
    apply(graph.get_mut("symbols").and_then(Value::as_array_mut));
}

#[allow(dead_code)]
pub(super) fn render_kind_icon(kind: &str) -> String {
    let body = match kind {
        "session" => SESSION_SVG_BODY,
        "file" => FILE_SVG_BODY,
        "symbol" => SYMBOL_SVG_BODY,
        other => atom_kind_spec(other).svg_body,
    };
    format!(
        "<svg class=\"omni-kind-icon\" viewBox=\"0 0 16 16\" width=\"14\" height=\"14\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.5\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\">{body}</svg>"
    )
}

pub(super) fn atom_kind_spec(raw: &str) -> &'static KindVisual {
    let lower = raw.trim().to_ascii_lowercase();
    ATOM_KINDS
        .iter()
        .find(|spec| spec.key == lower)
        .unwrap_or(&ATOM_KINDS[4]) // insight fallback
}

pub(super) fn edge_spec(key: &str) -> &'static EdgeVisualSpec {
    EDGE_SPECS
        .iter()
        .find(|spec| spec.key == key)
        .unwrap_or(&EDGE_SPECS[8]) // relates_to fallback
}

pub(super) fn format_timestamp_i64(raw: i64) -> String {
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

pub(super) fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
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

pub(super) fn normalize_edge_relation(
    raw_rel: &str,
    raw_subkind: &str,
) -> (String, String, String, String) {
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

pub(super) fn sanitize_viewport(vp: &CanvasViewport) -> CanvasViewport {
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

pub(super) fn ordered_pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_owned(), b.to_owned())
    } else {
        (b.to_owned(), a.to_owned())
    }
}

pub(super) fn split_file_label(path: &str) -> (String, String) {
    let trimmed = path.trim();
    match trimmed.rsplit_once('/') {
        Some((dir, file)) if !file.is_empty() => (file.to_owned(), dir.to_owned()),
        _ => (trimmed.to_owned(), String::new()),
    }
}

pub(super) fn split_symbol_fqn_file(fqn: &str) -> String {
    let trimmed = fqn.trim();
    if let Some((head, _)) = trimmed.split_once("::") {
        if head.contains('/') || head.contains('.') {
            return head.to_owned();
        }
    }
    String::new()
}

pub(super) fn format_symbol_title(fqn: &str, owner: &str, name: &str) -> String {
    if !owner.is_empty() && !name.is_empty() {
        return format!("{owner}::{name}");
    }
    if !name.is_empty() {
        return name.to_owned();
    }
    short_symbol_label(fqn)
}

pub(super) fn short_symbol_label(fqn: &str) -> String {
    let trimmed = fqn.trim();
    let file = split_symbol_fqn_file(trimmed);
    if !file.is_empty() {
        if let Some(rest) = trimmed.strip_prefix(&format!("{file}::")) {
            if !rest.is_empty() {
                return rest.to_owned();
            }
        }
    }
    trimmed.to_owned()
}

pub(super) fn compact_val(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(arr) => arr.iter().map(compact_val).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

pub(super) fn pick_first_value<'a>(obj: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|k| obj.get(*k))
}

pub(super) fn array_items(obj: &Value, keys: &[&str]) -> Vec<Value> {
    for k in keys {
        if let Some(arr) = obj.get(*k).and_then(Value::as_array) {
            return arr.clone();
        }
    }
    Vec::new()
}

pub(super) fn str_field(v: &Value, keys: &[&str]) -> String {
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

pub(super) fn bool_field(v: &Value, keys: &[&str]) -> bool {
    keys.iter()
        .any(|k| v.get(*k).and_then(Value::as_bool) == Some(true))
}

pub(super) fn f64_field(v: &Value, keys: &[&str]) -> Option<f64> {
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

pub(super) fn i64_field(v: &Value, keys: &[&str]) -> Option<i64> {
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

pub(super) fn string_list(v: &Value, keys: &[&str]) -> Vec<String> {
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

pub(super) fn short_hash(id: &str) -> String {
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

pub(super) fn title_case_token(raw: &str) -> String {
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

pub(super) fn to_snake_token(raw: &str) -> String {
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

pub(super) fn escape_attr_token(raw: &str) -> String {
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

pub(super) fn bool_str(v: bool) -> &'static str {
    if v {
        "true"
    } else {
        "false"
    }
}

pub(super) fn esc(raw: &str) -> String {
    crate::dom::escape_html(raw)
}

#[inline]
pub(super) fn push_attr(buf: &mut String, name: &str, val: &str) {
    buf.push(' ');
    buf.push_str(name);
    buf.push_str("=\"");
    buf.push_str(&esc(val));
    buf.push('"');
}

#[inline]
pub(super) fn push_raw_attr(buf: &mut String, name: &str, val: &str) {
    buf.push(' ');
    buf.push_str(name);
    buf.push_str("=\"");
    buf.push_str(val);
    buf.push('"');
}

pub(super) trait StringFallbackExt {
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

mod constellation;
mod dag;
mod merge_diff;
#[cfg(test)]
mod tests;

pub use constellation::render_constellation;
use constellation::{apply_constellation_filter, extract_graph_items};
#[allow(unused_imports)]
pub(crate) use constellation::{route_rect_cubic, CubicPoints};
pub use dag::render_branch_dag;
#[allow(unused_imports)]
pub(crate) use merge_diff::conflict_keep_request;
pub use merge_diff::render_merge_diff;
