use super::*;

pub(in crate::graph_map) struct SelectionContext {
    pub(in crate::graph_map) chosen: Option<usize>,
    pub(in crate::graph_map) chosen_link: Option<(usize, usize)>,
    pub(in crate::graph_map) selected: bool,
    pub(in crate::graph_map) cycle_view: bool,
    pub(in crate::graph_map) cycles: bool,
    pub(in crate::graph_map) shared: bool,
    pub(in crate::graph_map) pattern_view: bool,
    pub(in crate::graph_map) shared_edges: BTreeSet<(usize, usize)>,
    pub(in crate::graph_map) depth: Option<Vec<Option<i32>>>,
}

impl SelectionContext {
    pub(in crate::graph_map) fn from_view(
        view: &V,
        nodes: &[V],
        pairs: &[(usize, usize)],
        cone_reach: (usize, usize),
    ) -> Self {
        let path_index = build_path_index(nodes);
        let at = |path: &V| lookup_path_index(nodes, &path_index, path);
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

    pub(in crate::graph_map) fn lit(&self, pair: (usize, usize)) -> bool {
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
}

pub(in crate::graph_map) fn compute_selected_bbox(
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

pub(in crate::graph_map) fn radar_box_state(
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
pub(in crate::graph_map) struct NodeSelectionState {
    pub(in crate::graph_map) selected: bool,
    pub(in crate::graph_map) lit: bool,
    pub(in crate::graph_map) dim: bool,
    pub(in crate::graph_map) shared_path: bool,
    pub(in crate::graph_map) focus_target: bool,
    pub(in crate::graph_map) changed: bool,
    pub(in crate::graph_map) failed: bool,
    pub(in crate::graph_map) pressed: Option<bool>,
    pub(in crate::graph_map) blast: Option<(u64, f64)>,
    pub(in crate::graph_map) radar_state: &'static str,
    pub(in crate::graph_map) portal_dim: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::graph_map) struct EdgeSelectionState {
    pub(in crate::graph_map) edge_state: &'static str,
    pub(in crate::graph_map) lit: bool,
    pub(in crate::graph_map) shared_path: bool,
    pub(in crate::graph_map) diff: Option<String>,
    pub(in crate::graph_map) impact: Option<String>,
    pub(in crate::graph_map) commit_touched: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::graph_map) struct CanvasSelectionPatch {
    pub(in crate::graph_map) nodes: Vec<NodeSelectionState>,
    pub(in crate::graph_map) edges_by_pair: HashMap<(usize, usize), EdgeSelectionState>,
    pub(in crate::graph_map) frame_targets: Vec<bool>,
    pub(in crate::graph_map) focus: FocusOverlay,
    pub(in crate::graph_map) selected_bbox: Option<(f64, f64, f64, f64)>,
}

/// Computes pure selection/highlight/focus-target/blast attribute updates without rebuilding canvas HTML.
pub(in crate::graph_map) fn compute_canvas_selection_patch(
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

    let has_traced_path = overlay.path_nodes.is_some_and(|pn| !pn.is_empty())
        || overlay.path_edges.is_some_and(|pe| !pe.is_empty());
    let mut near_nodes = vec![false; nodes.len()];
    if let Some(chosen) = sel.chosen {
        if chosen < near_nodes.len() {
            near_nodes[chosen] = true;
        }
    }
    if let Some(path_nodes) = overlay.path_nodes {
        for &idx in path_nodes {
            if let Some(slot) = near_nodes.get_mut(idx) {
                *slot = true;
            }
        }
    }
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
    let mut edges_by_pair = HashMap::with_capacity(pairs.len());
    for &(from, to) in pairs {
        let on_traced = sel.shared_edges.contains(&(from, to))
            || overlay
                .path_edges
                .is_some_and(|pe| pe.contains(&(from, to)) || pe.contains(&(to, from)));
        let is_lit = sel.lit((from, to)) || on_traced;
        if is_lit {
            if let Some(slot) = near_nodes.get_mut(from) {
                *slot = true;
            }
            if let Some(slot) = near_nodes.get_mut(to) {
                *slot = true;
            }
        }
        let edge_state = if has_traced_path {
            if on_traced {
                "on"
            } else {
                "off"
            }
        } else if sel.cycle_view || sel.selected {
            if is_lit {
                "on"
            } else {
                "off"
            }
        } else {
            ""
        };
        let diff = impact_lookup
            .as_ref()
            .and_then(|lookup| lookup.edge_diff(&nodes, from, to))
            .map(str::to_owned);
        let impact = impact_lookup
            .as_ref()
            .and_then(|lookup| lookup.edge_impact(&nodes, from, to))
            .map(str::to_owned);
        let commit_touched =
            commit_changes_active && (node_is_changed[from] || node_is_changed[to]);
        edges_by_pair.insert(
            (from, to),
            EdgeSelectionState {
                edge_state,
                lit: is_lit,
                shared_path: on_traced,
                diff,
                impact,
                commit_touched,
            },
        );
    }

    let node_patches = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let is_lit = near_nodes[index];
            let on_traced_node = overlay.path_nodes.is_some_and(|pn| pn.contains(&index));
            let placed_cycle = plan_nodes.get(index).is_some_and(|p| p.cycle);
            let changed =
                node["changed"].as_u64().unwrap_or(0) > 0 || node_change_statuses[index].is_some();
            NodeSelectionState {
                selected: sel.chosen == Some(index),
                lit: is_lit,
                dim: (sel.selected || has_traced_path) && !is_lit && !on_traced_node,
                shared_path: (!sel.shared_edges.is_empty() && is_lit) || on_traced_node,
                focus_target: focus_overlay.nodes.contains(&index),
                changed,
                failed: node["suspect"].as_u64().is_some(),
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
                portal_dim: (sel.chosen.is_some() || has_traced_path) && !is_lit,
            }
        })
        .collect();

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

/// Refreshes `DenseScene` visual selection/highlight/path/diff flags during an in-place patch
/// so Canvas2D stays synchronized with DOM attributes.
pub(in crate::graph_map) fn refresh_dense_scene_for_patch(
    scene: &mut graph_viewport::canvas2d::DenseScene,
    pairs: &[(usize, usize)],
    node_keys: &[String],
    patch: &CanvasSelectionPatch,
    overlay: &CanvasOverlayContext<'_>,
) {
    let has_traced = overlay.path_nodes.is_some_and(|pn| !pn.is_empty());
    for dn in &mut scene.nodes {
        let on_traced = overlay.path_nodes.is_some_and(|pn| pn.contains(&dn.index));
        if let Some(np) = patch.nodes.get(dn.index) {
            dn.selected = np.selected;
            dn.lit = np.lit || on_traced;
            dn.dim = np.dim || (has_traced && !on_traced);
            dn.changed = np.changed;
            dn.failed = np.failed;
        }
        if let Some(pins) = overlay.pinned_positions {
            dn.pinned = node_keys
                .get(dn.index)
                .is_some_and(|k| pins.contains_key(k));
        }
    }
    for de in &mut scene.edges {
        let Some(&(from, to)) = pairs.get(de.index) else {
            continue;
        };
        let on_traced = overlay
            .path_edges
            .is_some_and(|pe| pe.contains(&(from, to)) || pe.contains(&(to, from)));
        if let Some(ep) = patch.edges_by_pair.get(&(from, to)) {
            let edge_state = if has_traced {
                if on_traced {
                    "on"
                } else {
                    "off"
                }
            } else {
                ep.edge_state
            };
            de.on = edge_state == "on" || ep.shared_path || on_traced;
            de.lit = ep.lit || on_traced;
            de.dim = edge_state == "off";
            if let Some(ref d) = ep.diff {
                de.diff = d.clone();
            }
            if let Some(ref imp) = ep.impact {
                de.impact = imp.clone();
            }
            de.commit_touched = ep.commit_touched;
        }
    }
}
