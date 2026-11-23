use super::*;

pub fn render_constellation(
    graph: &Value,
    filter: &ConstellationFilter,
    viewport: &CanvasViewport,
) -> RenderedCanvas {
    let vp = sanitize_viewport(viewport);
    let (nodes, resolved_edges) = prepare_constellation_graph(graph);
    let structure_digest = constellation_structure_digest(graph, filter);

    let active_branch = str_field(graph, &["active_branch"]);
    let (kept_nodes, kept_edges) =
        apply_constellation_filter(nodes, resolved_edges, filter, &active_branch);
    if kept_nodes.is_empty() {
        let html = render_constellation_empty(filter);
        return RenderedCanvas {
            html,
            radar_html: String::new(),
            width: MIN_PLANE_W,
            height: 400.0,
            node_count: 0,
            edge_count: 0,
            positions: BTreeMap::new(),
            raw_positions: BTreeMap::new(),
            structure_digest,
            dense_scene: None,
            node_rects: Vec::new(),
            node_edges: Vec::new(),
            node_htmls: Vec::new(),
            edge_endpoints: Vec::new(),
        };
    }

    let (mut placed_nodes, width, height) = layout_constellation_with_anchors(
        kept_nodes,
        &kept_edges,
        &filter.anchored_positions,
        &filter.pinned_positions,
    );
    // Deterministic render order by (group, y, x, id)
    placed_nodes.sort_by(|a, b| {
        a.group
            .cmp(&b.group)
            .then_with(|| a.y.total_cmp(&b.y))
            .then_with(|| a.x.total_cmp(&b.x))
            .then_with(|| a.id.cmp(&b.id))
    });

    let mut positions = BTreeMap::new();
    let mut raw_positions = BTreeMap::new();
    let mut node_by_id: BTreeMap<String, usize> = BTreeMap::new();
    let mut node_rects = Vec::with_capacity(placed_nodes.len());
    for (idx, n) in placed_nodes.iter().enumerate() {
        positions.insert(n.id.clone(), (n.x + n.w * 0.5, n.y + n.h * 0.5));
        raw_positions.insert(n.id.clone(), (n.x, n.y));
        node_by_id.insert(n.id.clone(), idx);
        node_rects.push((n.id.clone(), n.x, n.y, n.w, n.h));
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
    let has_path_focus = !filter.path_nodes.is_empty();

    // Group parallel edges between the same unordered node pair so curves separate cleanly.
    let mut pair_totals: BTreeMap<(String, String), usize> = BTreeMap::new();
    for edge in &kept_edges {
        let key = ordered_pair(&edge.from, &edge.to);
        *pair_totals.entry(key).or_default() += 1;
    }
    let mut pair_seen: BTreeMap<(String, String), usize> = BTreeMap::new();

    struct RoutedEdge {
        p0: (f64, f64),
        p1: (f64, f64),
        p2: (f64, f64),
        p3: (f64, f64),
        mid: (f64, f64),
        path_d: String,
        flow_suffix: &'static str,
        selected: bool,
        lit: bool,
        dimmed: bool,
        on_path: bool,
        against: bool,
        guessed: bool,
        code_edge: bool,
    }

    let overlay = extract_arch_overlay(graph);
    let file_of_node = |n: &CanvasNode| -> String {
        if !n.file_path.is_empty() {
            n.file_path.clone()
        } else {
            n.id.clone()
        }
    };

    let mut routed = Vec::with_capacity(kept_edges.len());
    let mut node_edges = Vec::with_capacity(kept_edges.len());
    for (idx, edge) in kept_edges.iter().enumerate() {
        let (Some(&u_idx), Some(&v_idx)) = (node_by_id.get(&edge.from), node_by_id.get(&edge.to))
        else {
            continue;
        };
        node_edges.push((u_idx, v_idx));
        let u = &placed_nodes[u_idx];
        let v = &placed_nodes[v_idx];
        let u_file = file_of_node(u);
        let v_file = file_of_node(v);
        let against = overlay.is_violated_edge(&u_file, &v_file);
        let guessed = overlay.is_inferred_edge(&edge.from, &edge.to, &u_file, &v_file);
        let code_edge = is_code_ir_subkind(&edge.subkind);
        let pair_key = ordered_pair(&edge.from, &edge.to);
        let slot = *pair_seen.get(&pair_key).unwrap_or(&0);
        pair_seen.insert(pair_key.clone(), slot + 1);
        let total = *pair_totals.get(&pair_key).unwrap_or(&1);

        let curve = route_bezier_ports(u, v, slot, total, idx);
        let mid = bezier_midpoint(&curve);
        let dx = curve.p3.0 - curve.p0.0;
        let dy = curve.p3.1 - curve.p0.1;
        let flow_suffix = if dx.abs() >= dy.abs() {
            if dx < -1.0 {
                "-rtl"
            } else {
                ""
            }
        } else if dy < -1.0 {
            "-btt"
        } else {
            "-ttb"
        };
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

        let is_selected_edge = selected_id.is_some_and(|s| edge.from == s || edge.to == s);
        let is_path_edge = filter
            .path_edges
            .contains(&(edge.from.clone(), edge.to.clone()))
            || filter
                .path_edges
                .contains(&(edge.to.clone(), edge.from.clone()));
        let is_highlighted_edge = filter.highlighted_ids.contains(&edge.from)
            || filter.highlighted_ids.contains(&edge.to);
        let lit = is_path_edge
            || is_selected_edge
            || (filter.highlighted_ids.contains(&edge.from)
                && filter.highlighted_ids.contains(&edge.to));
        let dimmed = if has_path_focus {
            !is_path_edge
        } else if has_selection_focus {
            !is_selected_edge && !is_highlighted_edge
        } else if has_highlight_focus {
            !is_highlighted_edge
        } else {
            false
        };

        routed.push((
            edge,
            RoutedEdge {
                p0: curve.p0,
                p1: curve.p1,
                p2: curve.p2,
                p3: curve.p3,
                mid,
                path_d,
                flow_suffix,
                selected: is_selected_edge || is_path_edge,
                lit,
                dimmed,
                on_path: is_path_edge,
                against,
                guessed,
                code_edge,
            },
        ));
    }

    let is_dense = placed_nodes.len() >= 180 || routed.len() >= 400 || filter.paged_count > 1;
    let plane_lod = lod(vp.scale);
    let tier_attr = if is_dense {
        " data-scale-tier=\"dense\""
    } else {
        ""
    };
    let mut html = format!(
        "<div class=\"graph-map-plane omni-plane\" style=\"--graph-pan-x:{:.1}px;--graph-pan-y:{:.1}px;--graph-scale:{:.4};width:{:.0}px;height:{:.0}px\" data-lod=\"{plane_lod}\"{tier_attr} data-node-count=\"{}\" data-edge-count=\"{}\">",
        vp.pan_x,
        vp.pan_y,
        vp.scale,
        width,
        height,
        placed_nodes.len(),
        routed.len()
    );

    // SVG plane (orbital guides, arrow markers, shared gradient flows; full paths rendered in SVG for small/normal graphs, offloaded to Canvas2D for Dense tier)
    html.push_str(&format!(
        "<svg class=\"omni-svg\" width=\"{:.0}\" height=\"{:.0}\" viewBox=\"0 0 {:.0} {:.0}\" aria-hidden=\"true\"><defs>",
        width, height, width, height
    ));
    html.push_str(&render_edge_marker_defs());
    html.push_str(&render_edge_gradient_defs());
    html.push_str("</defs>");

    html.push_str(&render_orbital_guides(&placed_nodes, width, height));

    let edge_endpoints: Vec<(String, String)> = routed
        .iter()
        .map(|(edge, _)| (edge.from.clone(), edge.to.clone()))
        .collect();
    let dense_scene = if is_dense {
        let dense_edges = routed
            .iter()
            .enumerate()
            .map(|(i, (edge, r))| {
                graph_viewport::canvas2d::DenseEdgeSpec::from_cubic(
                    i,
                    r.p0,
                    r.p1,
                    r.p2,
                    r.p3,
                    if r.on_path { 3 } else { 1 },
                    r.guessed,
                    r.against,
                    edge.arrow_key.clone(),
                    r.selected || r.on_path,
                    r.lit,
                    r.dimmed,
                    false,
                )
            })
            .collect();
        let dense_nodes = placed_nodes
            .iter()
            .enumerate()
            .map(|(i, n)| {
                let is_selected = selected_id == Some(n.id.as_str());
                let is_neighbor = !is_selected && neighbor_ids.contains(&n.id);
                let is_highlighted =
                    filter.highlighted_ids.contains(&n.id) || filter.path_nodes.contains(&n.id);
                let is_dimmed = if has_path_focus {
                    !filter.path_nodes.contains(&n.id)
                } else if has_selection_focus {
                    !is_selected && !is_neighbor && !is_highlighted
                } else if has_highlight_focus {
                    !is_highlighted
                } else {
                    false
                };
                graph_viewport::canvas2d::DenseNodeSpec {
                    index: i,
                    x: n.x,
                    y: n.y,
                    w: n.w,
                    h: n.h,
                    label: n.title.clone(),
                    role: n.group.as_str().to_owned(),
                    selected: is_selected,
                    lit: is_neighbor || is_highlighted,
                    dim: is_dimmed,
                    changed: !n.drift_status.is_empty(),
                    pinned: n.pinned || n.user_pinned,
                    failed: n.broken_risk,
                }
            })
            .collect();
        Some(graph_viewport::canvas2d::DenseScene {
            width,
            height,
            nodes: dense_nodes,
            edges: dense_edges,
        })
    } else {
        None
    };

    if !is_dense {
        for (edge, r) in &routed {
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
            let arrow_key = escape_attr_token(&edge.arrow_key);
            let violation_class = if r.against {
                " omni-edge-violation"
            } else {
                ""
            };
            let code_class = if r.code_edge { " omni-edge-code" } else { "" };
            let violation_attr = if r.against {
                " data-arch-violation=\"true\""
            } else {
                ""
            };
            let code_attr = if r.code_edge {
                " data-code-edge=\"true\""
            } else {
                ""
            };
            html.push_str(&format!(
                "<path class=\"omni-edge omni-edge-{arrow_key}{violation_class}{code_class}\" d=\"{}\" stroke=\"url(#omni-flow-{arrow_key}{})\" marker-end=\"url(#omni-arrow-{arrow_key})\" data-rel=\"{}\" data-subkind=\"{}\" data-from=\"{}\" data-to=\"{}\" data-selected=\"{}\" data-lit=\"{}\" data-dimmed=\"{}\" data-path=\"{}\"{violation_attr}{code_attr}{dash_attr}><title>{} {} {}</title></path>",
                r.path_d,
                r.flow_suffix,
                escape_attr_token(&edge.relation),
                escape_attr_token(subkind_attr),
                esc(&edge.from),
                esc(&edge.to),
                bool_str(r.selected),
                bool_str(r.lit),
                bool_str(r.dimmed),
                bool_str(r.on_path),
                esc(&edge.from),
                esc(&edge.label),
                esc(&edge.to),
            ));
            html.push_str(&format!(
                "<circle class=\"omni-edge-origin\" cx=\"{:.1}\" cy=\"{:.1}\" r=\"2.6\" data-rel=\"{arrow_key}\" data-lit=\"{}\" data-dimmed=\"{}\"/>",
                r.p0.0,
                r.p0.1,
                bool_str(r.lit),
                bool_str(r.dimmed),
            ));
        }

        // Mid-edge pill labels: omitted in `far`/`mid` via CSS `[data-lod]` without re-render.
        for (edge, r) in &routed {
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
    }

    html.push_str("</svg>");

    let active_roving_id = filter
        .focused_id
        .as_deref()
        .filter(|id| node_by_id.contains_key(*id))
        .or(selected_id)
        .or_else(|| placed_nodes.first().map(|n| n.id.as_str()));

    let mut node_htmls = if is_dense {
        Vec::with_capacity(placed_nodes.len())
    } else {
        Vec::new()
    };

    // Interactive HTML node cards (viewport-virtualized in Dense tier when bounds are known).
    for (idx, node) in placed_nodes.iter().enumerate() {
        let is_selected = selected_id == Some(node.id.as_str());
        let is_neighbor = !is_selected && neighbor_ids.contains(&node.id);
        let is_highlighted =
            filter.highlighted_ids.contains(&node.id) || filter.path_nodes.contains(&node.id);
        let is_dimmed = if has_path_focus {
            !filter.path_nodes.contains(&node.id)
        } else if has_selection_focus {
            !is_selected && !is_neighbor && !is_highlighted
        } else if has_highlight_focus {
            !is_highlighted
        } else {
            false
        };
        let offscreen = filter.viewport_bounds.is_some_and(|(vx, vy, vw, vh)| {
            let pad = 120.0;
            node.x + node.w < vx - pad
                || node.x > vx + vw + pad
                || node.y + node.h < vy - pad
                || node.y > vy + vh + pad
        });
        let is_roving_active = active_roving_id == Some(node.id.as_str());
        let offscreen_attr = offscreen
            && !is_roving_active
            && !is_selected
            && !is_neighbor
            && !is_highlighted
            && !node.user_pinned;
        let node_html = render_constellation_node(
            node,
            idx,
            is_selected,
            is_neighbor,
            is_highlighted,
            is_dimmed,
            offscreen_attr,
            is_roving_active,
            Some(&overlay),
        );
        if is_dense {
            node_htmls.push(node_html.clone());
        }
        if is_dense
            && offscreen
            && !is_selected
            && !is_neighbor
            && !is_highlighted
            && !node.user_pinned
            && !is_roving_active
        {
            continue;
        }
        html.push_str(&node_html);
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
        raw_positions,
        structure_digest,
        dense_scene,
        node_rects,
        node_edges,
        node_htmls,
        edge_endpoints,
    }
}

#[derive(Default)]
pub(super) struct ArchOverlayIndex {
    violations_by_file: BTreeMap<String, usize>,
    violation_msgs_by_file: BTreeMap<String, Vec<String>>,
    violated_pairs: BTreeSet<(String, String)>,
    inferred_pairs: BTreeSet<(String, String)>,
    cycle_files: BTreeSet<String>,
    blast_by_file: BTreeMap<String, (u64, u64, String)>,
    semantic_by_file: BTreeMap<String, Vec<(String, String)>>,
}

impl ArchOverlayIndex {
    fn is_violated_edge(&self, from_file: &str, to_file: &str) -> bool {
        if from_file.is_empty() || to_file.is_empty() {
            return false;
        }
        self.violated_pairs
            .contains(&(from_file.to_owned(), to_file.to_owned()))
    }

    fn is_inferred_edge(&self, from_id: &str, to_id: &str, from_file: &str, to_file: &str) -> bool {
        self.inferred_pairs
            .contains(&(from_id.to_owned(), to_id.to_owned()))
            || (!from_file.is_empty()
                && !to_file.is_empty()
                && self
                    .inferred_pairs
                    .contains(&(from_file.to_owned(), to_file.to_owned())))
    }

    fn violation_count(&self, path: &str) -> usize {
        self.violations_by_file.get(path).copied().unwrap_or(0)
    }

    fn in_cycle(&self, path: &str) -> bool {
        self.cycle_files.contains(path)
    }

    fn blast_summary(&self, path: &str) -> Option<(u64, u64, &str)> {
        self.blast_by_file
            .get(path)
            .map(|(f, t, r)| (*f, *t, r.as_str()))
    }

    fn semantic_count(&self, path: &str) -> usize {
        self.semantic_by_file.get(path).map_or(0, Vec::len)
    }
}

fn is_code_ir_subkind(subkind: &str) -> bool {
    matches!(
        subkind,
        "imports"
            | "calls"
            | "extends"
            | "implements"
            | "uses"
            | "type_ref"
            | "exposes_route"
            | "calls_route"
            | "publishes_event"
            | "handles_event"
            | "reads_table"
            | "writes_table"
            | "di_binds"
            | "di_binding"
            | "shares_route"
            | "shares_event"
            | "shares_table"
            | "shares_di"
    )
}

pub(super) fn extract_arch_overlay(graph: &Value) -> ArchOverlayIndex {
    let mut overlay = ArchOverlayIndex::default();

    if let Some(arr) = graph
        .get("architecture_violations")
        .and_then(Value::as_array)
    {
        for v in arr {
            let kind = str_field(v, &["kind"]);
            let from = str_field(v, &["from"]);
            let to = str_field(v, &["to"]);
            let msg = str_field(v, &["message"]);
            let from_is_file = !from.is_empty()
                && (kind != "cycle_budget" || from.contains('/') || from.contains('.'));
            if from_is_file {
                *overlay.violations_by_file.entry(from.clone()).or_default() += 1;
                if !msg.is_empty() {
                    overlay
                        .violation_msgs_by_file
                        .entry(from.clone())
                        .or_default()
                        .push(msg.clone());
                }
            }
            let is_edge_file_viol = matches!(kind.as_str(), "layer" | "package");
            if is_edge_file_viol && !to.is_empty() && to != from {
                *overlay.violations_by_file.entry(to.clone()).or_default() += 1;
                if !msg.is_empty() {
                    overlay
                        .violation_msgs_by_file
                        .entry(to.clone())
                        .or_default()
                        .push(msg);
                }
            }
            if is_edge_file_viol && !from.is_empty() && !to.is_empty() {
                overlay.violated_pairs.insert((from, to));
            }
        }
    }

    if let Some(arr) = graph.get("blast_markers").and_then(Value::as_array) {
        for m in arr {
            let path = str_field(m, &["path", "file_path"]);
            if path.is_empty() {
                continue;
            }
            let bf = m["blast_files"].as_u64().unwrap_or(0);
            let bt = m["blast_tests"].as_u64().unwrap_or(0);
            let vc = m["violation_count"].as_u64().unwrap_or(0) as usize;
            let in_cycle = bool_field(m, &["in_cycle"]);
            let risk = str_field(m, &["risk"]);
            if vc > 0 {
                overlay
                    .violations_by_file
                    .entry(path.clone())
                    .and_modify(|prev| *prev = (*prev).max(vc))
                    .or_insert(vc);
            }
            if in_cycle {
                overlay.cycle_files.insert(path.clone());
            }
            overlay.blast_by_file.insert(path, (bf, bt, risk));
        }
    }

    if let Some(arr) = graph.get("ir_edges").and_then(Value::as_array) {
        for e in arr {
            let conf = str_field(e, &["confidence"]);
            if !conf.eq_ignore_ascii_case("INFERRED") {
                continue;
            }
            let from = str_field(e, &["from", "source"]);
            let to = str_field(e, &["to", "target"]);
            if !from.is_empty() && !to.is_empty() {
                overlay.inferred_pairs.insert((from, to));
            }
            let sf = str_field(e, &["source_file"]);
            let tf = str_field(e, &["target_file"]);
            if !sf.is_empty() && !tf.is_empty() {
                overlay.inferred_pairs.insert((sf, tf));
            }
        }
    }

    if let Some(arr) = graph.get("ir_semantic_edges").and_then(Value::as_array) {
        for s in arr {
            let fp = str_field(s, &["file_path", "source_file", "source"]);
            let rel = str_field(s, &["relation", "kind"]);
            let tgt = str_field(s, &["target"]);
            if fp.is_empty() || rel.is_empty() || tgt.is_empty() {
                continue;
            }
            let list = overlay.semantic_by_file.entry(fp).or_default();
            let pair = (rel, tgt);
            if !list.contains(&pair) {
                list.push(pair);
            }
        }
    }

    overlay
}

fn enrich_node_from_overlay(
    overlay: &ArchOverlayIndex,
    file_path: &str,
    broken_risk: &mut bool,
    drift_status: &mut String,
    blob: &mut String,
) {
    if file_path.is_empty() {
        return;
    }
    let v_count = overlay.violation_count(file_path);
    let in_cycle = overlay.in_cycle(file_path);
    let blast = overlay.blast_summary(file_path);
    let sem_count = overlay.semantic_count(file_path);
    if v_count > 0 || in_cycle || blast.is_some_and(|(_, _, r)| r == "high") {
        *broken_risk = true;
    }
    if drift_status.is_empty() {
        if in_cycle {
            *drift_status = "cycle".to_owned();
        } else if v_count > 0 {
            *drift_status = "violation".to_owned();
        } else if blast.is_some_and(|(_, _, r)| r == "high") {
            *drift_status = "high blast".to_owned();
        }
    }
    if v_count > 0 {
        blob.push_str(" violation architecture");
    }
    if in_cycle {
        blob.push_str(" cycle");
    }
    if let Some((bf, _, risk)) = blast {
        blob.push_str(&format!(" blast {risk} blast:{bf}"));
    }
    if sem_count > 0 {
        blob.push_str(" semantic");
    }
}

pub(super) fn extract_canvas_edge(edge: &Value) -> Option<CanvasEdge> {
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
    let edge_kind = str_field(edge, &["kind", "type", "@type"]);
    let top_rel = str_field(edge, &["relation"]);
    let prop_rel = props
        .map(|p| str_field(p, &["relation", "rel_kind", "relation_kind", "kind"]))
        .unwrap_or_default();
    let kind_snake = to_snake_token(&edge_kind);
    let is_structural_kind = matches!(
        kind_snake.as_str(),
        "session_follows"
            | "follows"
            | "published_by"
            | "published"
            | "consumed_by"
            | "consumed"
            | "touches_file"
            | "touches"
            | "mentions_symbol"
            | "mentions"
            | "derived_from"
            | "derived"
    );
    let raw_rel = if is_structural_kind {
        edge_kind
    } else if !top_rel.is_empty() {
        top_rel.clone()
    } else if !edge_kind.is_empty() {
        edge_kind
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
    if subkind.is_empty()
        && !top_rel.is_empty()
        && to_snake_token(&top_rel) != to_snake_token(&raw_rel)
    {
        subkind = top_rel;
    } else if subkind.is_empty() && !prop_rel.is_empty() && prop_rel != raw_rel {
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

pub(super) fn extract_graph_items(
    graph: &Value,
) -> (Vec<CanvasNode>, Vec<CanvasEdge>, BTreeMap<String, String>) {
    let mut nodes = Vec::new();
    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    let mut seen_ids: BTreeSet<String> = BTreeSet::new();
    let mut file_to_node_id: BTreeMap<String, String> = BTreeMap::new();
    let overlay = extract_arch_overlay(graph);

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
                file_path: String::new(),
                symbol_id: String::new(),
                symbol_fqn: String::new(),
                symbol_kind: String::new(),
                symbols: Vec::new(),
                confidence: None,
                pinned: false,
                pin_priority: 0,
                tags,
                touch_count: 0,
                search_blob: blob,
                drift_status: String::new(),
                broken_risk: false,
                user_pinned: false,
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
            let files = string_list(item, &["files"]);
            let symbols = string_list(item, &["symbols"]);
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
                "{id} {kind} {title} {summary} {branch} {session_id} {actor} {} {} {}",
                tags.join(" "),
                files.join(" "),
                symbols.join(" ")
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
                file_path: String::new(),
                symbol_id: String::new(),
                symbol_fqn: String::new(),
                symbol_kind: String::new(),
                symbols,
                confidence,
                pinned,
                pin_priority,
                tags,
                touch_count: 0,
                search_blob: blob,
                drift_status: String::new(),
                broken_risk: false,
                user_pinned: false,
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
            if !path.is_empty() {
                file_to_node_id
                    .entry(path.clone())
                    .or_insert_with(|| id.clone());
            }
            let branch = str_field(item, &["branch", "source_branch", "graph_branch"]);
            let lang = str_field(item, &["language", "kind"]);
            let touch_count = i64_field(item, &["touch_count", "atom_count"])
                .unwrap_or(0)
                .max(0) as usize;
            let mut broken_risk = bool_field(item, &["broken_risk"]);
            let mut drift_status = if broken_risk {
                "broken".to_owned()
            } else {
                str_field(item, &["drift_status", "drift", "change_status"])
            };
            let (base_name, dir_hint) = split_file_label(&path);
            let mut blob =
                format!("{id} {path} {branch} {lang} {drift_status}").to_ascii_lowercase();
            enrich_node_from_overlay(
                &overlay,
                &path,
                &mut broken_risk,
                &mut drift_status,
                &mut blob,
            );
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
                file_path: path,
                symbol_id: String::new(),
                symbol_fqn: String::new(),
                symbol_kind: String::new(),
                symbols: Vec::new(),
                confidence: None,
                pinned: false,
                pin_priority: 0,
                tags: Vec::new(),
                touch_count,
                search_blob: blob,
                drift_status,
                broken_risk,
                user_pinned: false,
                x: 0.0,
                y: 0.0,
                w,
                h,
            });
        }
    }

    if let Some(arr) = graph.get("symbols").and_then(Value::as_array) {
        for item in arr {
            let id = str_field(item, &["id", "fqn", "symbol_id", "@id"]);
            if id.is_empty() || !seen_ids.insert(id.clone()) {
                continue;
            }
            let fqn = str_field(item, &["fqn", "id", "symbol_id"]).if_empty(&id);
            if fqn != id && !fqn.is_empty() {
                aliases.entry(fqn.clone()).or_insert_with(|| id.clone());
            }
            let symbol_id = str_field(item, &["symbol_id"]);
            if !symbol_id.is_empty() && symbol_id != id {
                aliases
                    .entry(symbol_id.clone())
                    .or_insert_with(|| id.clone());
            }
            let file_path = str_field(item, &["file_path", "path"])
                .if_empty_from(|| split_symbol_fqn_file(&fqn));
            let owner = str_field(item, &["owner"]);
            let name = str_field(item, &["name"]);
            let title = str_field(item, &["label"])
                .if_empty_from(|| format_symbol_title(&fqn, &owner, &name));
            let symbol_kind = str_field(item, &["kind", "symbol_kind"]).if_empty("symbol");
            let branch = str_field(item, &["branch", "source_branch", "graph_branch"]);
            let atom_ids = string_list(item, &["atom_ids"]);
            let mention_count = i64_field(item, &["mention_count", "atom_count"])
                .unwrap_or(atom_ids.len() as i64)
                .max(0) as usize;
            let mut broken_risk = bool_field(item, &["broken_risk"]);
            let mut drift_status = if broken_risk {
                "broken".to_owned()
            } else {
                str_field(item, &["drift_status", "drift", "change_status"])
            };
            let mut blob = format!(
                "{id} {fqn} {symbol_id} {title} {file_path} {symbol_kind} {owner} {name} {branch} {drift_status}"
            )
            .to_ascii_lowercase();
            enrich_node_from_overlay(
                &overlay,
                &file_path,
                &mut broken_risk,
                &mut drift_status,
                &mut blob,
            );
            let (w, h) = NodeKindGroup::Symbol.size();
            nodes.push(CanvasNode {
                id,
                group: NodeKindGroup::Symbol,
                kind: "symbol".to_owned(),
                title,
                subtitle: file_path.clone(),
                branch,
                shared_space: String::new(),
                session_id: String::new(),
                status: String::new(),
                provider: symbol_kind.clone(),
                file_path,
                symbol_id,
                symbol_fqn: fqn,
                symbol_kind,
                symbols: Vec::new(),
                confidence: None,
                pinned: false,
                pin_priority: 0,
                tags: Vec::new(),
                touch_count: mention_count,
                search_blob: blob,
                drift_status,
                broken_risk,
                user_pinned: false,
                x: 0.0,
                y: 0.0,
                w,
                h,
            });
        }
    }

    let mut edges = Vec::new();
    let mut seen_edges: BTreeSet<(String, String, String, String)> = BTreeSet::new();
    for key in ["edges", "symbol_edges"] {
        if let Some(arr) = graph.get(key).and_then(Value::as_array) {
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
    }

    let mut synth_files = 0usize;
    let resolve_or_synth_file = |raw_ep: &str,
                                 fallback_file: &str,
                                 seen_ids: &mut BTreeSet<String>,
                                 file_to_node_id: &mut BTreeMap<String, String>,
                                 nodes: &mut Vec<CanvasNode>,
                                 synth_files: &mut usize|
     -> Option<String> {
        if let Some(resolved) = aliases.get(raw_ep) {
            if seen_ids.contains(resolved) {
                return Some(resolved.clone());
            }
        }
        if seen_ids.contains(raw_ep) {
            return Some(raw_ep.to_owned());
        }
        if !fallback_file.is_empty() {
            if let Some(existing) = file_to_node_id.get(fallback_file) {
                return Some(existing.clone());
            }
            if *synth_files < 24 && seen_ids.insert(fallback_file.to_owned()) {
                *synth_files += 1;
                file_to_node_id.insert(fallback_file.to_owned(), fallback_file.to_owned());
                let mut broken_risk = false;
                let mut drift_status = String::new();
                let (base_name, dir_hint) = split_file_label(fallback_file);
                let mut blob = format!("{fallback_file} code ir").to_ascii_lowercase();
                enrich_node_from_overlay(
                    &overlay,
                    fallback_file,
                    &mut broken_risk,
                    &mut drift_status,
                    &mut blob,
                );
                let (w, h) = NodeKindGroup::File.size();
                nodes.push(CanvasNode {
                    id: fallback_file.to_owned(),
                    group: NodeKindGroup::File,
                    kind: "file".to_owned(),
                    title: base_name,
                    subtitle: dir_hint,
                    branch: String::new(),
                    shared_space: String::new(),
                    session_id: String::new(),
                    status: String::new(),
                    provider: String::new(),
                    file_path: fallback_file.to_owned(),
                    symbol_id: String::new(),
                    symbol_fqn: String::new(),
                    symbol_kind: String::new(),
                    symbols: Vec::new(),
                    confidence: None,
                    pinned: false,
                    pin_priority: 0,
                    tags: Vec::new(),
                    touch_count: 0,
                    search_blob: blob,
                    drift_status,
                    broken_risk,
                    user_pinned: false,
                    x: 0.0,
                    y: 0.0,
                    w,
                    h,
                });
                return Some(fallback_file.to_owned());
            }
        }
        None
    };

    if let Some(arr) = graph.get("ir_edges").and_then(Value::as_array) {
        for item in arr {
            let raw_from = str_field(item, &["from", "src", "source"]);
            let raw_to = str_field(item, &["to", "dst", "target"]);
            let sf = str_field(item, &["source_file"]);
            let tf = str_field(item, &["target_file"]);
            let Some(from_id) = resolve_or_synth_file(
                &raw_from,
                &sf,
                &mut seen_ids,
                &mut file_to_node_id,
                &mut nodes,
                &mut synth_files,
            ) else {
                continue;
            };
            let Some(to_id) = resolve_or_synth_file(
                &raw_to,
                &tf,
                &mut seen_ids,
                &mut file_to_node_id,
                &mut nodes,
                &mut synth_files,
            ) else {
                continue;
            };
            let raw_rel = str_field(item, &["relation", "kind"]).if_empty("depends_on");
            let subkind_hint = str_field(item, &["subkind"]).if_empty(&raw_rel);
            let (relation, subkind, arrow_key, _) =
                normalize_edge_relation(&raw_rel, &subkind_hint);
            let label = subkind_hint.replace('_', " ");
            if !seen_edges.insert((
                from_id.clone(),
                to_id.clone(),
                relation.clone(),
                subkind.clone(),
            )) {
                continue;
            }
            edges.push(CanvasEdge {
                from: from_id,
                to: to_id,
                relation,
                subkind,
                arrow_key,
                label,
            });
        }
    }

    if let Some(arr) = graph.get("ir_semantic_edges").and_then(Value::as_array) {
        let mut by_target: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for item in arr {
            let sf = str_field(item, &["file_path", "source_file", "source"]);
            let rel = str_field(item, &["relation", "kind"]);
            let tgt = str_field(item, &["target"]);
            if sf.is_empty() || rel.is_empty() || tgt.is_empty() {
                continue;
            }
            by_target.entry(tgt).or_default().push((sf, rel));
        }
        let mut sem_links = 0usize;
        for (tgt, entries) in by_target {
            if sem_links >= 64 {
                break;
            }
            let producers: Vec<&(String, String)> = entries
                .iter()
                .filter(|(_, r)| {
                    matches!(
                        r.as_str(),
                        "exposes_route"
                            | "publishes_event"
                            | "writes_table"
                            | "di_binds"
                            | "di_binding"
                    )
                })
                .collect();
            let consumers: Vec<&(String, String)> = entries
                .iter()
                .filter(|(_, r)| {
                    matches!(
                        r.as_str(),
                        "calls_route"
                            | "handles_event"
                            | "reads_table"
                            | "writes_table"
                            | "di_binds"
                    )
                })
                .collect();
            for (src_file, rel) in &producers {
                for (dst_file, _) in &consumers {
                    if src_file == dst_file || sem_links >= 64 {
                        continue;
                    }
                    let Some(from_id) = resolve_or_synth_file(
                        src_file,
                        src_file,
                        &mut seen_ids,
                        &mut file_to_node_id,
                        &mut nodes,
                        &mut synth_files,
                    ) else {
                        continue;
                    };
                    let Some(to_id) = resolve_or_synth_file(
                        dst_file,
                        dst_file,
                        &mut seen_ids,
                        &mut file_to_node_id,
                        &mut nodes,
                        &mut synth_files,
                    ) else {
                        continue;
                    };
                    let (relation, subkind, arrow_key, _) = normalize_edge_relation(rel, rel);
                    if seen_edges.insert((
                        from_id.clone(),
                        to_id.clone(),
                        relation.clone(),
                        subkind.clone(),
                    )) {
                        sem_links += 1;
                        edges.push(CanvasEdge {
                            from: from_id,
                            to: to_id,
                            relation,
                            subkind,
                            arrow_key,
                            label: tgt.clone(),
                        });
                    }
                }
            }
        }
    }

    (nodes, edges, aliases)
}

pub(super) fn apply_constellation_filter(
    nodes: Vec<CanvasNode>,
    edges: Vec<CanvasEdge>,
    filter: &ConstellationFilter,
    active_branch: &str,
) -> (Vec<CanvasNode>, Vec<CanvasEdge>) {
    let query = filter.query.trim().to_ascii_lowercase();
    let query_terms: Vec<&str> = query.split_whitespace().collect();
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
        let query_ok = query_terms.is_empty()
            || filter.highlighted_ids.contains(&node.id)
            || query_terms
                .iter()
                .all(|term| node.search_blob.contains(term));
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
            NodeKindGroup::File | NodeKindGroup::Symbol => {
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
            NodeKindGroup::File | NodeKindGroup::Symbol => {
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

    let kept_edges: Vec<CanvasEdge> = edges
        .into_iter()
        .filter(|e| kept_ids.contains(&e.from) && kept_ids.contains(&e.to))
        .collect();

    if filter.isolate_depth > 0 {
        if let Some(seed_id) = filter
            .isolate_seed
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let mut id_to_idx: BTreeMap<&str, usize> = BTreeMap::new();
            for (i, n) in kept_nodes.iter().enumerate() {
                id_to_idx.insert(n.id.as_str(), i);
            }
            if let Some(&seed_idx) = id_to_idx.get(seed_id) {
                let mut out_adj = vec![Vec::new(); kept_nodes.len()];
                let mut in_adj = vec![Vec::new(); kept_nodes.len()];
                for e in &kept_edges {
                    if let (Some(&u), Some(&v)) =
                        (id_to_idx.get(e.from.as_str()), id_to_idx.get(e.to.as_str()))
                    {
                        out_adj[u].push(v);
                        in_adj[v].push(u);
                    }
                }
                let dist = graph_viewport::hop_cone_distances(
                    kept_nodes.len(),
                    &out_adj,
                    &in_adj,
                    &[seed_idx],
                    filter.isolate_depth.clamp(1, 3),
                );
                let cone_ids: BTreeSet<String> = kept_nodes
                    .iter()
                    .enumerate()
                    .filter(|(i, n)| dist[*i].is_some() || filter.path_nodes.contains(&n.id))
                    .map(|(_, n)| n.id.clone())
                    .collect();
                let isolated_nodes: Vec<CanvasNode> = kept_nodes
                    .into_iter()
                    .filter(|n| cone_ids.contains(&n.id))
                    .collect();
                let isolated_edges: Vec<CanvasEdge> = kept_edges
                    .into_iter()
                    .filter(|e| cone_ids.contains(&e.from) && cone_ids.contains(&e.to))
                    .collect();
                return (isolated_nodes, isolated_edges);
            }
        }
    }

    (kept_nodes, kept_edges)
}

#[allow(dead_code)]
pub(super) fn layout_constellation(
    nodes: Vec<CanvasNode>,
    edges: &[CanvasEdge],
) -> (Vec<CanvasNode>, f64, f64) {
    layout_constellation_with_anchors(nodes, edges, &BTreeMap::new(), &BTreeMap::new())
}

pub(super) fn layout_constellation_with_anchors(
    mut nodes: Vec<CanvasNode>,
    edges: &[CanvasEdge],
    anchors: &BTreeMap<String, (f64, f64)>,
    pins: &BTreeMap<String, (f64, f64)>,
) -> (Vec<CanvasNode>, f64, f64) {
    // When loading subsequent pages, keep already-placed nodes anchored where they stand
    // and pack newly added nodes into extension rows per track so cards never jump.
    if !anchors.is_empty() && nodes.iter().any(|n| anchors.contains_key(&n.id)) {
        let mut max_y_by_track = [MARGIN_Y; 3];
        let mut x_by_track = [
            MARGIN_X,
            MARGIN_X + SESSION_W + TRACK_GAP_X,
            MARGIN_X + SESSION_W + ATOM_W + TRACK_GAP_X * 2.0,
        ];
        let mut seen_track = [false; 3];
        let mut locked = vec![false; nodes.len()];
        let mut new_by_track: [Vec<usize>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for (idx, n) in nodes.iter_mut().enumerate() {
            let track = match n.group {
                NodeKindGroup::Session => 0,
                NodeKindGroup::Atom => 1,
                NodeKindGroup::File | NodeKindGroup::Symbol => 2,
            };
            if let Some(&(px, py)) = pins.get(&n.id) {
                n.x = px;
                n.y = py;
                n.user_pinned = true;
                locked[idx] = true;
            } else if let Some(&(ax, ay)) = anchors.get(&n.id) {
                n.x = ax;
                n.y = ay;
                locked[idx] = true;
                if !seen_track[track] {
                    x_by_track[track] = ax;
                    seen_track[track] = true;
                } else {
                    x_by_track[track] = x_by_track[track].min(ax);
                }
                max_y_by_track[track] = max_y_by_track[track].max(n.y + n.h + NODE_GAP_Y);
            } else {
                new_by_track[track].push(idx);
            }
        }
        for track in 0..3 {
            let new_len = new_by_track[track].len();
            let col_count = match track {
                0 => {
                    if new_len > 7 {
                        2
                    } else {
                        1
                    }
                }
                1 => ((new_len as f64) / 6.0).sqrt().ceil().clamp(2.0, 6.0) as usize,
                _ => {
                    if new_len > 10 {
                        2
                    } else {
                        1
                    }
                }
            };
            let base_y = max_y_by_track[track];
            let base_x = x_by_track[track];
            for (pos, &idx) in new_by_track[track].iter().enumerate() {
                let col = pos % col_count;
                let row = pos / col_count;
                let step_x = if track == 1 {
                    ATOM_W + ATOM_COL_GAP_X
                } else if track == 0 {
                    SESSION_W + 30.0
                } else {
                    FILE_W + 30.0
                };
                let step_h = nodes[idx].h + NODE_GAP_Y;
                nodes[idx].x = base_x + (col as f64) * step_x;
                nodes[idx].y = base_y + (row as f64) * step_h;
            }
        }
        resolve_node_overlaps_respecting_locks(&mut nodes, &locked);
        let max_x = nodes.iter().map(|n| n.x + n.w).fold(0.0, f64::max);
        let max_y = nodes.iter().map(|n| n.y + n.h).fold(0.0, f64::max);
        let width = (max_x + MARGIN_X + 44.0).max(MIN_PLANE_W);
        let height = (max_y + MARGIN_Y).max(MIN_PLANE_H);
        return (nodes, width, height);
    }
    let mut session_indices = Vec::new();
    let mut atom_indices = Vec::new();
    let mut file_indices = Vec::new();
    for (idx, n) in nodes.iter().enumerate() {
        match n.group {
            NodeKindGroup::Session => session_indices.push(idx),
            NodeKindGroup::Atom => atom_indices.push(idx),
            NodeKindGroup::File | NodeKindGroup::Symbol => file_indices.push(idx),
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
            ((n_atoms as f64) / 5.5).sqrt().ceil().clamp(3.0, 8.0) as usize
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

    // 3. Place FileRefs and SymbolRefs on outer/right orbital arc ordered by barycenter of touching atoms.
    let pos_by_id: BTreeMap<String, f64> = nodes
        .iter()
        .filter(|n| !matches!(n.group, NodeKindGroup::File | NodeKindGroup::Symbol))
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
    // Group Symbols under their owning File by shared file_path barycenter.
    let mut group_bary: BTreeMap<String, (f64, usize)> = BTreeMap::new();
    for &idx in &file_indices {
        let fid = &nodes[idx].id;
        let group_key = if nodes[idx].file_path.is_empty() {
            fid.clone()
        } else {
            nodes[idx].file_path.clone()
        };
        let bary = file_bary.get(fid).copied().unwrap_or(f64::MAX);
        if bary < f64::MAX {
            let entry = group_bary.entry(group_key).or_insert((0.0, 0));
            entry.0 += bary;
            entry.1 += 1;
        }
    }
    file_indices.sort_by(|&a, &b| {
        let ga = if nodes[a].file_path.is_empty() {
            &nodes[a].id
        } else {
            &nodes[a].file_path
        };
        let gb = if nodes[b].file_path.is_empty() {
            &nodes[b].id
        } else {
            &nodes[b].file_path
        };
        let ba = group_bary
            .get(ga)
            .filter(|(_, c)| *c > 0)
            .map(|(s, c)| s / (*c as f64))
            .unwrap_or_else(|| file_bary.get(&nodes[a].id).copied().unwrap_or(f64::MAX));
        let bb = group_bary
            .get(gb)
            .filter(|(_, c)| *c > 0)
            .map(|(s, c)| s / (*c as f64))
            .unwrap_or_else(|| file_bary.get(&nodes[b].id).copied().unwrap_or(f64::MAX));
        ba.total_cmp(&bb)
            .then_with(|| ga.cmp(gb))
            .then_with(|| nodes[a].group.cmp(&nodes[b].group))
            .then_with(|| nodes[a].id.cmp(&nodes[b].id))
    });

    let file_span_h = if !file_indices.is_empty() {
        let f_cols = if file_indices.len() <= 10 {
            1
        } else if file_indices.len() <= 26 {
            2
        } else {
            ((file_indices.len() as f64) / 8.0)
                .sqrt()
                .ceil()
                .clamp(2.0, 5.0) as usize
        };
        let f_rows = file_indices.len().div_ceil(f_cols);
        let mut max_col_h: f64 = 0.0;
        for col in 0..f_cols {
            let start = col * f_rows;
            let end = ((col + 1) * f_rows).min(file_indices.len());
            if start >= end {
                continue;
            }
            let slice = &file_indices[start..end];
            let mid_idx = (slice.len().saturating_sub(1)) as f64 * 0.5;
            let mut y = MARGIN_Y;
            for (row, &idx) in slice.iter().enumerate() {
                let t = if mid_idx > 0.0 {
                    (row as f64 - mid_idx) / mid_idx
                } else {
                    0.0
                };
                let arc_dx = (1.0 - t * t) * 34.0;
                let sym_indent = if nodes[idx].group == NodeKindGroup::Symbol {
                    14.0
                } else {
                    0.0
                };
                nodes[idx].x = next_x + (col as f64) * (FILE_W + 30.0) + arc_dx + sym_indent;
                nodes[idx].y = y;
                let gap = if row + 1 < slice.len()
                    && nodes[slice[row + 1]].group == NodeKindGroup::Symbol
                    && !nodes[idx].file_path.is_empty()
                    && nodes[slice[row + 1]].file_path == nodes[idx].file_path
                {
                    8.0
                } else {
                    NODE_GAP_Y
                };
                y += nodes[idx].h + gap;
            }
            max_col_h = max_col_h.max(y - NODE_GAP_Y - MARGIN_Y);
        }
        max_col_h
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

    // 6. Apply user pin overrides without shifting unpinned nodes when pins sit near top-left.
    if !pins.is_empty() {
        for n in &mut nodes {
            if let Some(&(px, py)) = pins.get(&n.id) {
                n.x = px.max(8.0);
                n.y = py.max(8.0);
                n.user_pinned = true;
            }
        }
        let unpinned_min_x = nodes
            .iter()
            .filter(|n| !n.user_pinned)
            .map(|n| n.x)
            .fold(f64::INFINITY, f64::min);
        let unpinned_min_y = nodes
            .iter()
            .filter(|n| !n.user_pinned)
            .map(|n| n.y)
            .fold(f64::INFINITY, f64::min);
        if unpinned_min_x.is_finite()
            && unpinned_min_y.is_finite()
            && (unpinned_min_x < MARGIN_X || unpinned_min_y < MARGIN_Y)
        {
            let shift_x = (MARGIN_X - unpinned_min_x).max(0.0);
            let shift_y = (MARGIN_Y - unpinned_min_y).max(0.0);
            for n in &mut nodes {
                if !n.user_pinned {
                    n.x += shift_x;
                    n.y += shift_y;
                }
            }
        }
    } else {
        let min_x = nodes.iter().map(|n| n.x).fold(f64::INFINITY, f64::min);
        let min_y = nodes.iter().map(|n| n.y).fold(f64::INFINITY, f64::min);
        if min_x.is_finite() && min_y.is_finite() {
            let shift_x = MARGIN_X - min_x;
            let shift_y = MARGIN_Y - min_y;
            for n in &mut nodes {
                n.x += shift_x;
                n.y += shift_y;
            }
        }
    }

    let max_x = nodes.iter().map(|n| n.x + n.w).fold(0.0, f64::max);
    let max_y = nodes.iter().map(|n| n.y + n.h).fold(0.0, f64::max);
    let right_pad = MARGIN_X + if file_indices.is_empty() { 0.0 } else { 44.0 };
    let width = (max_x + right_pad).max(MIN_PLANE_W);
    let height = (max_y + MARGIN_Y).max(MIN_PLANE_H);
    (nodes, width, height)
}

pub(super) fn resolve_node_overlaps_respecting_locks(nodes: &mut [CanvasNode], locked: &[bool]) {
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
            let min_y = nodes[i].y + nodes[i].h + NODE_GAP_Y;
            for &j in &order[(i_pos + 1)..] {
                if nodes[j].y >= min_y {
                    break;
                }
                if locked.get(j).copied().unwrap_or(false) {
                    continue;
                }
                let x_overlap = nodes[i].x < nodes[j].x + nodes[j].w + NODE_GAP_X
                    && nodes[j].x < nodes[i].x + nodes[i].w + NODE_GAP_X;
                if !x_overlap {
                    continue;
                }
                nodes[j].y = min_y;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
}

pub(super) fn resolve_node_overlaps(nodes: &mut [CanvasNode]) {
    let empty_locks = vec![false; nodes.len()];
    resolve_node_overlaps_respecting_locks(nodes, &empty_locks);
}

pub(super) fn render_orbital_guides(nodes: &[CanvasNode], width: f64, height: f64) -> String {
    let mut svg = String::new();
    let has_symbols = nodes.iter().any(|n| n.group == NodeKindGroup::Symbol);
    let anchor_label = if has_symbols {
        "FILE & SYMBOL ANCHORS"
    } else {
        "FILE ANCHORS"
    };
    for (group, label) in [
        (NodeKindGroup::Session, "SESSIONS"),
        (NodeKindGroup::Atom, "CONTEXT ATOMS"),
        (NodeKindGroup::File, anchor_label),
    ] {
        let mut min_x = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        let mut count = 0usize;
        let matches_group = |n: &&CanvasNode| {
            n.group == group || (group == NodeKindGroup::File && n.group == NodeKindGroup::Symbol)
        };
        let has_unpinned = nodes.iter().any(|n| matches_group(&n) && !n.user_pinned);
        for n in nodes
            .iter()
            .filter(|n| matches_group(n) && (!has_unpinned || !n.user_pinned))
        {
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
            NodeKindGroup::File | NodeKindGroup::Symbol => (cx + 26.0).min(width - 16.0),
        };
        svg.push_str(&format!(
            "<path class=\"omni-orbit-ring\" data-track=\"{}\" d=\"M {cx:.1} {pad_top:.1} Q {curve_x:.1} {:.1}, {cx:.1} {pad_bot:.1}\"/>",
            group.as_str(),
            (pad_top + pad_bot) * 0.5,
        ));
        svg.push_str(&format!(
            "<text class=\"omni-orbit-label\" data-track=\"{}\" x=\"{cx:.1}\" y=\"{:.1}\" text-anchor=\"middle\">{}</text>",
            group.as_str(),
            (pad_top - 4.0).max(16.0),
            esc(label),
        ));
    }
    svg
}

pub(super) fn render_edge_marker_defs() -> String {
    let mut defs = String::new();
    for spec in EDGE_SPECS {
        defs.push_str(&format!(
            "<marker id=\"omni-arrow-{}\" class=\"omni-arrow omni-arrow-{}\" viewBox=\"0 0 8 8\" refX=\"7\" refY=\"4\" markerWidth=\"7\" markerHeight=\"7\" markerUnits=\"userSpaceOnUse\" orient=\"auto\">{}</marker>",
            spec.key, spec.key, spec.marker_body,
        ));
    }
    defs
}

pub(super) fn render_edge_gradient_defs() -> String {
    let mut defs = String::new();
    for spec in EDGE_SPECS {
        let (rel, subkind) = match spec.key {
            "supports" | "contradicts" | "supersedes" | "depends_on" | "refines" => {
                ("relates_to", spec.key)
            }
            _ => (spec.key, ""),
        };
        for (suffix, x1, y1, x2, y2) in [
            ("", "0%", "0%", "100%", "0%"),
            ("-rtl", "100%", "0%", "0%", "0%"),
            ("-ttb", "0%", "0%", "0%", "100%"),
            ("-btt", "0%", "100%", "0%", "0%"),
        ] {
            defs.push_str(&format!(
                "<linearGradient id=\"omni-flow-{}{suffix}\" class=\"omni-flow omni-flow-{}\" data-rel=\"{rel}\" data-subkind=\"{subkind}\" gradientUnits=\"objectBoundingBox\" x1=\"{x1}\" y1=\"{y1}\" x2=\"{x2}\" y2=\"{y2}\"><stop offset=\"0\" class=\"omni-flow-from\"/><stop offset=\"0.5\" class=\"omni-flow-mid\"/><stop offset=\"1\" class=\"omni-flow-to\"/></linearGradient>",
                spec.key, spec.key,
            ));
        }
    }
    defs
}

pub(super) struct CubicCurve {
    p0: (f64, f64),
    p1: (f64, f64),
    p2: (f64, f64),
    p3: (f64, f64),
}

pub(crate) type CubicPoints = ((f64, f64), (f64, f64), (f64, f64), (f64, f64));

pub(crate) fn route_rect_cubic(
    u: (f64, f64, f64, f64),
    v: (f64, f64, f64, f64),
    same_id: bool,
    slot: usize,
    total: usize,
    edge_idx: usize,
) -> CubicPoints {
    let ucx = u.0 + u.2 * 0.5;
    let ucy = u.1 + u.3 * 0.5;
    let vcx = v.0 + v.2 * 0.5;
    let vcy = v.1 + v.3 * 0.5;

    let lane_offset = if total > 1 {
        let centered = slot as f64 - (total as f64 - 1.0) * 0.5;
        centered * 22.0
    } else {
        0.0
    };

    if same_id {
        let x0 = u.0 + u.2;
        let y0 = ucy - 10.0;
        let y3 = ucy + 10.0;
        return (
            (x0, y0),
            (x0 + 54.0, y0 - 28.0),
            (x0 + 54.0, y3 + 28.0),
            (x0, y3),
        );
    }

    if vcx > ucx + u.2 * 0.48 {
        let p0 = (u.0 + u.2, ucy);
        let p3 = (v.0, vcy);
        let dx = ((p3.0 - p0.0).abs() * 0.46).max(46.0);
        // Ensure a non-zero SVG bounding box height so `objectBoundingBox` gradients render on horizontal links.
        let bow = if (p0.1 - p3.1).abs() < 0.2 && lane_offset == 0.0 {
            0.4
        } else {
            0.0
        };
        (
            p0,
            (p0.0 + dx, p0.1 + lane_offset - bow),
            (p3.0 - dx, p3.1 + lane_offset + bow),
            p3,
        )
    } else if ucx > vcx + v.2 * 0.48 {
        let p0 = (u.0, ucy);
        let p3 = (v.0 + v.2, vcy);
        let dx = ((p0.0 - p3.0).abs() * 0.46).max(46.0);
        let bow = if (p0.1 - p3.1).abs() < 0.2 && lane_offset == 0.0 {
            0.4
        } else {
            0.0
        };
        (
            p0,
            (p0.0 - dx, p0.1 + lane_offset - bow),
            (p3.0 + dx, p3.1 + lane_offset + bow),
            p3,
        )
    } else {
        let dir = if edge_idx.is_multiple_of(2) {
            1.0
        } else {
            -1.0
        };
        let x0 = if dir > 0.0 { u.0 + u.2 } else { u.0 };
        let x3 = if dir > 0.0 { v.0 + v.2 } else { v.0 };
        let max_arc = ATOM_COL_GAP_X * 0.85;
        let arc = (20.0 + (vcy - ucy).abs() * 0.08 + lane_offset.abs() * 0.35).min(max_arc) * dir;
        ((x0, ucy), (x0 + arc, ucy), (x3 + arc, vcy), (x3, vcy))
    }
}

pub(super) fn route_bezier_ports(
    u: &CanvasNode,
    v: &CanvasNode,
    slot: usize,
    total: usize,
    edge_idx: usize,
) -> CubicCurve {
    let (p0, p1, p2, p3) = route_rect_cubic(
        (u.x, u.y, u.w, u.h),
        (v.x, v.y, v.w, v.h),
        u.id == v.id,
        slot,
        total,
        edge_idx,
    );
    CubicCurve { p0, p1, p2, p3 }
}

pub(super) fn bezier_midpoint(curve: &CubicCurve) -> (f64, f64) {
    (
        0.125 * curve.p0.0 + 0.375 * curve.p1.0 + 0.375 * curve.p2.0 + 0.125 * curve.p3.0,
        0.125 * curve.p0.1 + 0.375 * curve.p1.1 + 0.375 * curve.p2.1 + 0.125 * curve.p3.1,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_constellation_node(
    node: &CanvasNode,
    roving_index: usize,
    selected: bool,
    neighbor: bool,
    highlighted: bool,
    dimmed: bool,
    offscreen: bool,
    roving_active: bool,
    overlay: Option<&ArchOverlayIndex>,
) -> String {
    let kind_label = match node.group {
        NodeKindGroup::Session => "Session",
        NodeKindGroup::File => "File",
        NodeKindGroup::Symbol => "Symbol",
        NodeKindGroup::Atom => atom_kind_spec(&node.kind).label,
    };
    let kind_key = match node.group {
        NodeKindGroup::Session => "session",
        NodeKindGroup::File => "file",
        NodeKindGroup::Symbol => "symbol",
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
            if !node.drift_status.is_empty() {
                let drift_label = if node.broken_risk {
                    "broken risk"
                } else {
                    node.drift_status.as_str()
                };
                indicators.push_str(&format!(
                    "<span class=\"omni-node-drift\" data-drift=\"{}\">{}</span>",
                    escape_attr_token(&node.drift_status),
                    esc(drift_label)
                ));
            }
        }
        NodeKindGroup::Symbol => {
            if !node.symbol_kind.is_empty() {
                indicators.push_str(&format!(
                    "<span class=\"omni-symbol-kind-pill\">{}</span>",
                    esc(&node.symbol_kind)
                ));
            }
            let suffix = if node.touch_count == 1 { "ref" } else { "refs" };
            indicators.push_str(&format!(
                "<span class=\"omni-symbol-count\">{} {suffix}</span>",
                node.touch_count
            ));
            if !node.drift_status.is_empty() {
                let drift_label = if node.broken_risk {
                    "broken risk"
                } else {
                    node.drift_status.as_str()
                };
                indicators.push_str(&format!(
                    "<span class=\"omni-node-drift\" data-drift=\"{}\">{}</span>",
                    escape_attr_token(&node.drift_status),
                    esc(drift_label)
                ));
            }
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
    if node.group == NodeKindGroup::Atom && !node.symbols.is_empty() {
        let first_label = short_symbol_label(&node.symbols[0]);
        let badge_text = if node.symbols.len() == 1 {
            first_label
        } else {
            format!("{first_label} +{}", node.symbols.len() - 1)
        };
        let full_title = node
            .symbols
            .iter()
            .map(|s| short_symbol_label(s))
            .collect::<Vec<_>>()
            .join(", ");
        foot.push_str(&format!(
            "<span class=\"omni-symbol-badge\" title=\"Mentioned symbols: {}\">&lt;/&gt; {}</span>",
            esc(&full_title),
            esc(&badge_text),
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
    } else if matches!(node.group, NodeKindGroup::File | NodeKindGroup::Symbol)
        && !node.subtitle.is_empty()
    {
        foot.push_str(&format!(
            "<span class=\"omni-file-dir\">{}</span>",
            esc(&node.subtitle)
        ));
    }

    let aria_label = match node.group {
        NodeKindGroup::Symbol => {
            if node.file_path.is_empty() {
                format!("Symbol ({}): {}", node.symbol_kind, node.title)
            } else {
                format!(
                    "Symbol ({}): {} in {} (double-click to open in Code Map)",
                    node.symbol_kind, node.title, node.file_path
                )
            }
        }
        NodeKindGroup::File => {
            format!(
                "File: {} (double-click to open in Code Map)",
                if node.file_path.is_empty() {
                    &node.title
                } else {
                    &node.file_path
                }
            )
        }
        _ if node.branch.is_empty() => format!("{kind_label}: {}", node.title),
        _ => format!("{kind_label}: {} on {}", node.title, node.branch),
    };

    let mut extra_attrs = String::new();
    if !node.file_path.is_empty() {
        push_attr(&mut extra_attrs, "data-file-path", &node.file_path);
    }
    if !node.symbol_id.is_empty() {
        push_attr(&mut extra_attrs, "data-symbol-id", &node.symbol_id);
    }
    if !node.symbol_fqn.is_empty() {
        push_attr(&mut extra_attrs, "data-symbol-fqn", &node.symbol_fqn);
    }
    if !node.symbol_kind.is_empty() {
        push_attr(&mut extra_attrs, "data-symbol-kind", &node.symbol_kind);
    }
    if !node.drift_status.is_empty() {
        let drift_tok = escape_attr_token(&node.drift_status);
        push_raw_attr(&mut extra_attrs, "data-drift", &drift_tok);
    }
    if node.broken_risk {
        push_raw_attr(&mut extra_attrs, "data-broken-risk", "true");
    }
    if matches!(node.group, NodeKindGroup::File | NodeKindGroup::Symbol) {
        if let Some(ov) = overlay {
            let ov_path = if node.file_path.is_empty() {
                node.id.as_str()
            } else {
                node.file_path.as_str()
            };
            let v_count = ov.violation_count(ov_path);
            let in_cycle = ov.in_cycle(ov_path);
            let blast = ov.blast_summary(ov_path);
            let sem_count = ov.semantic_count(ov_path);
            if v_count > 0 {
                let v_str = v_count.to_string();
                push_raw_attr(&mut extra_attrs, "data-arch-violations", &v_str);
                let title_hint = ov
                    .violation_msgs_by_file
                    .get(ov_path)
                    .map(|msgs| msgs.join("; "))
                    .unwrap_or_else(|| format!("{v_count} architectural violation(s)"));
                let plural = if v_count == 1 { "" } else { "s" };
                indicators.push_str(&format!(
                    "<span class=\"omni-arch-badge\" data-violations=\"{v_count}\" title=\"{}\">{v_count} viol{plural}</span>",
                    esc(&title_hint)
                ));
            }
            if in_cycle {
                push_raw_attr(&mut extra_attrs, "data-in-cycle", "true");
                indicators.push_str(
                    "<span class=\"omni-cycle-badge\" title=\"File participates in dependency cycle\">cycle</span>",
                );
            }
            if let Some((bf, bt, risk)) = blast {
                if bf > 0 {
                    let bf_str = bf.to_string();
                    push_raw_attr(&mut extra_attrs, "data-blast-files", &bf_str);
                    push_attr(&mut extra_attrs, "data-blast-risk", risk);
                    indicators.push_str(&format!(
                        "<span class=\"omni-blast-badge\" data-risk=\"{}\" title=\"Blast radius: {bf} files, {bt} tests\">{bf} blast</span>",
                        escape_attr_token(risk)
                    ));
                }
            }
            if sem_count > 0 {
                indicators.push_str(&format!(
                    "<span class=\"omni-sem-badge\" title=\"Semantic IR edges\">{sem_count} sem</span>"
                ));
            }
        }
    }
    if offscreen {
        push_raw_attr(&mut extra_attrs, "data-offscreen", "true");
    }
    let tabindex = if roving_active { "0" } else { "-1" };

    format!(
        "<button type=\"button\" tabindex=\"{tabindex}\" data-roving-index=\"{roving_index}\" class=\"omni-node omni-node-{} omni-kind-{kind_key}\" style=\"left:{:.1}px;top:{:.1}px;width:{:.1}px;height:{:.1}px\" data-action=\"omnigraph-select-node\" data-node-id=\"{}\" data-node-type=\"{}\" data-kind=\"{kind_key}\" data-branch=\"{}\"{extra_attrs} data-selected=\"{}\" data-neighbor=\"{}\" data-highlighted=\"{}\" data-dimmed=\"{}\" data-pinned=\"{}\" aria-pressed=\"{}\" aria-label=\"{}\" title=\"{}\"><span class=\"omni-node-head\">{icon}<span class=\"omni-kind-badge\" data-kind=\"{kind_key}\">{kind_label}</span><span class=\"omni-node-indicators\">{indicators}</span></span><span class=\"omni-node-title\">{}</span><span class=\"omni-node-foot\">{foot}</span></button>",
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
        bool_str(node.pinned || node.user_pinned),
        bool_str(selected),
        esc(&aria_label),
        esc(&aria_label),
        esc(&node.title),
        icon = render_kind_icon(kind_key),
    )
}

pub(super) fn render_constellation_radar(
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

    let min_side = 16.0_f64.min(width).min(height).max(1.0);
    let raw_vw = (NOMINAL_VIEWPORT_W / vp.scale).max(min_side);
    let raw_vh = (NOMINAL_VIEWPORT_H / vp.scale).max(min_side);
    let vw = raw_vw.min(width);
    let vh = raw_vh.min(height);
    let vx = (-vp.pan_x / vp.scale).clamp(0.0, (width - vw).max(0.0));
    let vy = (-vp.pan_y / vp.scale).clamp(0.0, (height - vh).max(0.0));

    let mut out = format!(
        "<div class=\"omni-radar\" data-w=\"{width:.0}\" data-h=\"{height:.0}\" tabindex=\"0\" role=\"region\" aria-label=\"Constellation minimap — click, drag, or use arrow keys to pan\"><svg class=\"omni-radar-svg\" width=\"{rw:.0}\" height=\"{rh:.0}\" viewBox=\"0 0 {width:.0} {height:.0}\" preserveAspectRatio=\"xMidYMid meet\" focusable=\"false\" aria-hidden=\"true\"><rect class=\"omni-radar-frame\" x=\"2\" y=\"2\" width=\"{:.0}\" height=\"{:.0}\" rx=\"10\"/>",
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
        } else if matches!(node.group, NodeKindGroup::File | NodeKindGroup::Symbol) {
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

pub(super) fn render_constellation_empty(filter: &ConstellationFilter) -> String {
    let branch = filter.branch.trim();
    let has_filter = !filter.query.trim().is_empty()
        || !filter.kind.trim().is_empty()
        || !filter.show_files
        || !filter.show_sessions
        || filter.isolate_depth > 0
        || !filter.path_nodes.is_empty()
        || (!branch.is_empty() && branch != "all" && branch != "main");
    let (heading, detail, action, action_label) = if has_filter {
        (
            "No context nodes match your filter",
            "Try clearing the search query, kind pill, isolation, or branch filter to see the full constellation.",
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
        "<div class=\"graph-map-plane omni-plane\" data-empty=\"true\" style=\"--graph-pan-x:0.0px;--graph-pan-y:0.0px;--graph-scale:1.0000;width:100%;height:100%;min-height:360px;transform:none;transition:none\"><div class=\"omni-empty-state\" role=\"status\"><svg class=\"omni-empty-illustration\" viewBox=\"0 0 96 64\" width=\"96\" height=\"64\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.4\" aria-hidden=\"true\"><circle cx=\"22\" cy=\"36\" r=\"7\"/><circle cx=\"48\" cy=\"18\" r=\"8\"/><circle cx=\"74\" cy=\"42\" r=\"6\"/><path d=\"M28 33 L41 22 M55 22 L69 38\" stroke-dasharray=\"3 3\"/></svg><h3 class=\"omni-empty-title\">{heading}</h3><p class=\"omni-empty-body\">{detail}</p><button type=\"button\" class=\"button compact omni-empty-action\" data-action=\"{action}\">{action_label}</button></div></div>"
    )
}
