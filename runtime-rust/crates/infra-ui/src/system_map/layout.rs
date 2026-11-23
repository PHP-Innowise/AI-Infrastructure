use super::*;

#[derive(Clone, Debug, PartialEq)]
pub struct PlacedSysNode {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub col: usize,
    pub cycle: bool,
    pub child_of: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlacedSysFrame {
    pub service_id: String,
    pub title: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlacedSysWire {
    pub edge_index: usize,
    pub path_d: String,
    pub mid_x: f64,
    pub mid_y: f64,
    pub back: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SysLayout {
    pub nodes: Vec<PlacedSysNode>,
    pub frames: Vec<PlacedSysFrame>,
    pub wires: Vec<PlacedSysWire>,
    pub tier_bands: Vec<(String, f64, f64, f64)>,
    pub width: f64,
    pub height: f64,
}

/// Computes Sugiyama-style left-to-right layered coordinates, unfolded service frames, and smooth cubic Bezier wires.
pub fn layout_system_nodes(doc: &V, layers_mode: bool) -> SysLayout {
    layout_system_nodes_with_pins(doc, layers_mode, &BTreeMap::new())
}

pub fn layout_system_nodes_with_pins(
    doc: &V,
    layers_mode: bool,
    pins: &BTreeMap<String, (f64, f64)>,
) -> SysLayout {
    let all_nodes = m::array(&doc["nodes"]);
    let all_edges = m::array(&doc["edges"]);
    if all_nodes.is_empty() {
        return SysLayout::default();
    }

    let cycles = find_cross_service_cycles(doc);
    let mut scc_of: HashMap<String, usize> = HashMap::new();
    for (scc_idx, comp) in cycles.iter().enumerate() {
        for id in comp {
            scc_of.insert(id.clone(), scc_idx);
        }
    }
    let cycle_members: BTreeSet<String> = cycles.into_iter().flatten().collect();
    let (tiers, _) = assign_service_layers(doc);

    let mut top_nodes = Vec::new();
    let mut children_by_service: BTreeMap<String, Vec<V>> = BTreeMap::new();

    for n in &all_nodes {
        let kind = n["kind"].as_str().unwrap_or("service");
        if is_child_node_kind(kind) && !node_id(n).starts_with("__unmatched__:") {
            let parent = first_non_empty(n, &["parent_service", "parent", "service"]);
            if !parent.is_empty() {
                children_by_service
                    .entry(parent)
                    .or_default()
                    .push(n.clone());
                continue;
            }
        }
        top_nodes.push(n.clone());
    }

    let mut top_idx_by_id = BTreeMap::new();
    for (idx, n) in top_nodes.iter().enumerate() {
        top_idx_by_id.insert(node_id(n), idx);
    }

    let count = top_nodes.len();
    let mut col_of = vec![0usize; count];

    if layers_mode {
        for (idx, n) in top_nodes.iter().enumerate() {
            let id = node_id(n);
            col_of[idx] = tiers.get(&id).copied().unwrap_or(1);
        }
    } else {
        // Longest-path layering on DAG (ignoring cycle back-edges by index order).
        let mut indeg = vec![0usize; count];
        let mut dag_adj = vec![Vec::new(); count];
        for e in &all_edges {
            if m::yes(&e["child_link"]) {
                continue;
            }
            let from = m::text(e, "from");
            let to = m::text(e, "to");
            if let (Some(&u), Some(&v)) = (top_idx_by_id.get(&from), top_idx_by_id.get(&to)) {
                if u == v {
                    continue;
                }
                if scc_of.contains_key(&from) && scc_of.get(&from) == scc_of.get(&to) && u > v {
                    continue;
                }
                dag_adj[u].push(v);
                indeg[v] += 1;
            }
        }
        let mut q = VecDeque::new();
        for (i, &deg) in indeg.iter().enumerate() {
            if deg == 0 {
                q.push_back(i);
            }
        }
        let mut visited = vec![false; count];
        while let Some(u) = q.pop_front() {
            visited[u] = true;
            for &v in &dag_adj[u] {
                col_of[v] = col_of[v].max(col_of[u] + 1).min(6);
                indeg[v] = indeg[v].saturating_sub(1);
                if indeg[v] == 0 {
                    q.push_back(v);
                }
            }
        }
        for i in 0..count {
            if !visited[i] {
                col_of[i] = col_of[i].max(1);
            }
            if node_id(&top_nodes[i]) == "__system__" {
                col_of[i] = col_of.iter().copied().max().unwrap_or(1).max(1);
            }
        }
    }

    // Group top-level services into columns.
    let max_col = col_of.iter().copied().max().unwrap_or(0);
    let mut cols: Vec<Vec<usize>> = vec![Vec::new(); max_col + 1];
    for (idx, &c) in col_of.iter().enumerate() {
        cols[c].push(idx);
    }

    // Compact empty columns when not in fixed 4-tier layers mode.
    let mut active_cols: Vec<(usize, Vec<usize>)> = if layers_mode {
        (0..=3)
            .map(|tier| {
                let members = cols.get(tier).cloned().unwrap_or_default();
                (tier, members)
            })
            .filter(|(_, m)| !m.is_empty())
            .collect()
    } else {
        cols.into_iter()
            .enumerate()
            .filter(|(_, m)| !m.is_empty())
            .collect()
    };

    // Multi-pass barycenter ordering across columns (inspired by graph_map::plan_roomy)
    // minimizes edge crossings while keeping `__system__` pinned last in its column.
    if active_cols.len() > 1 {
        let mut col_idx_by_node = vec![0usize; count];
        let mut row_by_node = vec![0usize; count];
        for (ci, (_, members)) in active_cols.iter().enumerate() {
            for (ri, &u) in members.iter().enumerate() {
                col_idx_by_node[u] = ci;
                row_by_node[u] = ri;
            }
        }
        let mut neighbors = vec![Vec::<usize>::new(); count];
        for e in &all_edges {
            if m::yes(&e["child_link"]) {
                continue;
            }
            if let (Some(&u), Some(&v)) = (
                top_idx_by_id.get(&m::text(e, "from")),
                top_idx_by_id.get(&m::text(e, "to")),
            ) {
                if u != v && col_idx_by_node[u] != col_idx_by_node[v] {
                    neighbors[u].push(v);
                    neighbors[v].push(u);
                }
            }
        }
        for pass in 0..8usize {
            let col_order: Vec<usize> = if pass % 2 == 0 {
                (1..active_cols.len()).collect()
            } else {
                (0..active_cols.len().saturating_sub(1)).rev().collect()
            };
            for ci in col_order {
                let prefer_left = pass % 2 == 0;
                let mut scored: Vec<(bool, f64, usize)> = active_cols[ci]
                    .1
                    .iter()
                    .map(|&u| {
                        let is_sys = node_id(&top_nodes[u]) == "__system__";
                        let preferred: Vec<usize> = neighbors[u]
                            .iter()
                            .copied()
                            .filter(|&v| {
                                if prefer_left {
                                    col_idx_by_node[v] < ci
                                } else {
                                    col_idx_by_node[v] > ci
                                }
                            })
                            .collect();
                        let pool = if preferred.is_empty() {
                            &neighbors[u]
                        } else {
                            &preferred
                        };
                        let score = if pool.is_empty() {
                            row_by_node[u] as f64
                        } else {
                            let sum: usize = pool.iter().map(|&v| row_by_node[v]).sum();
                            sum as f64 / pool.len() as f64
                        };
                        (is_sys, score, u)
                    })
                    .collect();
                scored.sort_by(|a, b| {
                    a.0.cmp(&b.0)
                        .then_with(|| a.1.total_cmp(&b.1))
                        .then_with(|| a.2.cmp(&b.2))
                });
                active_cols[ci].1 = scored.iter().map(|x| x.2).collect();
                for (ri, &u) in active_cols[ci].1.iter().enumerate() {
                    row_by_node[u] = ri;
                }
            }
        }
    }

    let mut placed_nodes = Vec::new();
    let mut frames = Vec::new();
    let mut tier_bands = Vec::new();
    let mut pos_by_id: BTreeMap<String, (f64, f64, f64, f64, usize)> = BTreeMap::new();

    let top_offset = if layers_mode { MARGIN + 30.0 } else { MARGIN };
    let mut cur_x = MARGIN;
    let mut max_y: f64 = 220.0;

    for (render_col, (logical_tier, indices)) in active_cols.iter().enumerate() {
        let mut col_w = NODE_WIDTH;
        for &idx in indices {
            let sid = node_id(&top_nodes[idx]);
            if children_by_service.contains_key(&sid) {
                col_w = col_w.max(NODE_WIDTH + 2.0 * FRAME_PAD);
            }
        }

        let mut cur_y = top_offset;
        for &idx in indices {
            let n = &top_nodes[idx];
            let sid = node_id(n);
            let in_cycle = cycle_members.contains(&sid);
            if let Some(children) = children_by_service.get(&sid) {
                let frame_x = cur_x;
                let frame_y = cur_y;
                let inner_x = frame_x + FRAME_PAD;
                let mut inner_y = frame_y + FRAME_HEAD;

                placed_nodes.push(PlacedSysNode {
                    id: sid.clone(),
                    x: inner_x,
                    y: inner_y,
                    w: NODE_WIDTH,
                    h: NODE_HEIGHT,
                    col: render_col,
                    cycle: in_cycle,
                    child_of: None,
                });
                pos_by_id.insert(
                    sid.clone(),
                    (inner_x, inner_y, NODE_WIDTH, NODE_HEIGHT, render_col),
                );
                inner_y += NODE_HEIGHT + 8.0;

                for child in children {
                    let cid = node_id(child);
                    placed_nodes.push(PlacedSysNode {
                        id: cid.clone(),
                        x: inner_x + 16.0,
                        y: inner_y,
                        w: CHILD_WIDTH,
                        h: CHILD_HEIGHT,
                        col: render_col,
                        cycle: false,
                        child_of: Some(sid.clone()),
                    });
                    pos_by_id.insert(
                        cid,
                        (
                            inner_x + 16.0,
                            inner_y,
                            CHILD_WIDTH,
                            CHILD_HEIGHT,
                            render_col,
                        ),
                    );
                    inner_y += CHILD_HEIGHT + 6.0;
                }

                let frame_h = (inner_y - frame_y) + FRAME_PAD - 6.0;
                let frame_w = NODE_WIDTH + 2.0 * FRAME_PAD;
                frames.push(PlacedSysFrame {
                    service_id: sid.clone(),
                    title: m::text(n, "name"),
                    x: frame_x,
                    y: frame_y,
                    w: frame_w,
                    h: frame_h,
                });
                cur_y += frame_h + ROW_GAP;
            } else {
                let nx = cur_x + ((col_w - NODE_WIDTH) / 2.0).round();
                let ny = cur_y;
                placed_nodes.push(PlacedSysNode {
                    id: sid.clone(),
                    x: nx,
                    y: ny,
                    w: NODE_WIDTH,
                    h: NODE_HEIGHT,
                    col: render_col,
                    cycle: in_cycle,
                    child_of: None,
                });
                pos_by_id.insert(sid, (nx, ny, NODE_WIDTH, NODE_HEIGHT, render_col));
                cur_y += NODE_HEIGHT + ROW_GAP;
            }
        }

        if layers_mode {
            let tier_name = match logical_tier {
                0 => "Edge & Gateways",
                1 => "Application Services",
                2 => "Core & Platform",
                _ => "Shared Authority",
            };
            tier_bands.push((tier_name.to_owned(), cur_x - 8.0, col_w + 16.0, cur_y));
        }

        max_y = max_y.max(cur_y);
        cur_x += col_w + COL_GAP;
    }

    // Apply user Shift+drag pin overrides before computing port routing & wires so edges follow pinned boxes.
    let mut extra_w: f64 = 0.0;
    let mut extra_h: f64 = 0.0;
    if !pins.is_empty() {
        let mut parent_deltas: BTreeMap<String, (f64, f64)> = BTreeMap::new();
        for n in &mut placed_nodes {
            if n.child_of.is_some() {
                continue;
            }
            if let Some(&(px, py)) = pins.get(&n.id) {
                let has_frame = frames.iter().any(|f| f.service_id == n.id);
                let px = px.max(if has_frame { FRAME_PAD + 4.0 } else { 8.0 });
                let py = py.max(if has_frame { FRAME_HEAD + 4.0 } else { 8.0 });
                let dx = px - n.x;
                let dy = py - n.y;
                n.x = px;
                n.y = py;
                if let Some(entry) = pos_by_id.get_mut(&n.id) {
                    entry.0 = px;
                    entry.1 = py;
                }
                parent_deltas.insert(n.id.clone(), (dx, dy));
                for f in &mut frames {
                    if f.service_id == n.id {
                        f.x = (f.x + dx).max(4.0);
                        f.y = (f.y + dy).max(4.0);
                        extra_w = extra_w.max(f.x + f.w + MARGIN);
                        extra_h = extra_h.max(f.y + f.h + MARGIN);
                    }
                }
                extra_w = extra_w.max(px + n.w + MARGIN);
                extra_h = extra_h.max(py + n.h + MARGIN);
            }
        }
        // Shift unfolded children cleanly with their pinned parent frame unless pinned individually.
        for n in &mut placed_nodes {
            let Some(parent_id) = n.child_of.as_ref() else {
                continue;
            };
            if let Some(&(px, py)) = pins.get(&n.id) {
                n.x = px.max(8.0);
                n.y = py.max(8.0);
            } else if let Some(&(dx, dy)) = parent_deltas.get(parent_id) {
                n.x += dx;
                n.y += dy;
            } else {
                continue;
            }
            if let Some(entry) = pos_by_id.get_mut(&n.id) {
                entry.0 = n.x;
                entry.1 = n.y;
            }
            if let Some(f) = frames.iter_mut().find(|f| &f.service_id == parent_id) {
                let right = n.x + n.w + FRAME_PAD;
                let bottom = n.y + n.h + FRAME_PAD;
                if n.x - FRAME_PAD < f.x {
                    let expand = f.x - (n.x - FRAME_PAD).max(4.0);
                    f.x = (n.x - FRAME_PAD).max(4.0);
                    f.w += expand;
                }
                if right > f.x + f.w {
                    f.w = right - f.x;
                }
                if bottom > f.y + f.h {
                    f.h = bottom - f.y;
                }
                extra_w = extra_w.max(f.x + f.w + MARGIN);
                extra_h = extra_h.max(f.y + f.h + MARGIN);
            }
            extra_w = extra_w.max(n.x + n.w + MARGIN);
            extra_h = extra_h.max(n.y + n.h + MARGIN);
        }
    }

    let mut total_w = (cur_x - COL_GAP + MARGIN).max(420.0).max(extra_w);
    let mut total_h = (max_y - ROW_GAP + MARGIN).max(240.0).max(extra_h);

    struct CandidateWire {
        edge_index: usize,
        from: String,
        to: String,
        fx: f64,
        fy: f64,
        fw: f64,
        fh: f64,
        fcol: usize,
        tx: f64,
        ty: f64,
        tw: f64,
        th: f64,
        tcol: usize,
        back: bool,
        src_side: u8,
        dst_side: u8,
    }

    struct PortSlot {
        other_y: f64,
        back: bool,
        cw_idx: usize,
        is_src: bool,
    }

    let mut candidates = Vec::new();
    let mut port_slots: BTreeMap<(String, u8), Vec<PortSlot>> = BTreeMap::new();

    for (edge_index, edge) in all_edges.iter().enumerate() {
        let from = m::text(edge, "from");
        let to = m::text(edge, "to");
        let Some(&(fx, fy, fw, fh, fcol)) = pos_by_id.get(&from) else {
            continue;
        };
        let Some(&(tx, ty, tw, th, tcol)) = pos_by_id.get(&to) else {
            continue;
        };
        if from == to {
            continue;
        }
        let f_mid_x = fx + fw * 0.5;
        let t_mid_x = tx + tw * 0.5;
        let either_pinned = pins.contains_key(&from) || pins.contains_key(&to);
        let back = if either_pinned {
            f_mid_x > t_mid_x + 20.0 || ((f_mid_x - t_mid_x).abs() <= 20.0 && fy > ty)
        } else {
            fcol > tcol || (fcol == tcol && fy > ty)
        };
        let same_x_col = (f_mid_x - t_mid_x).abs() < (fw.min(tw) * 0.25);
        let (src_side, dst_side) = if same_x_col {
            (1u8, 1u8)
        } else if f_mid_x < t_mid_x {
            (1u8, 0u8)
        } else {
            (0u8, 1u8)
        };
        let cw_idx = candidates.len();
        port_slots
            .entry((from.clone(), src_side))
            .or_default()
            .push(PortSlot {
                other_y: ty + th / 2.0,
                back,
                cw_idx,
                is_src: true,
            });
        port_slots
            .entry((to.clone(), dst_side))
            .or_default()
            .push(PortSlot {
                other_y: fy + fh / 2.0,
                back,
                cw_idx,
                is_src: false,
            });
        candidates.push(CandidateWire {
            edge_index,
            from,
            to,
            fx,
            fy,
            fw,
            fh,
            fcol,
            tx,
            ty,
            tw,
            th,
            tcol,
            back,
            src_side,
            dst_side,
        });
    }

    let mut port_ranks: BTreeMap<(usize, bool), (usize, usize)> = BTreeMap::new();
    for entries in port_slots.values_mut() {
        entries.sort_by(|a, b| {
            a.other_y
                .total_cmp(&b.other_y)
                .then_with(|| a.back.cmp(&b.back))
                .then_with(|| a.cw_idx.cmp(&b.cw_idx))
        });
        let total = entries.len();
        for (rank, slot) in entries.iter().enumerate() {
            port_ranks.insert((slot.cw_idx, slot.is_src), (rank, total));
        }
    }

    let spread_y = |top: f64, height: f64, base_frac: f64, rank: usize, total: usize| -> f64 {
        if total <= 1 {
            return top + height * base_frac;
        }
        let step = SYS_PORT_GAP.min((height * 0.55) / total as f64);
        let offset = ((rank as f64 - (total as f64 - 1.0) / 2.0) * step)
            .clamp(-SYS_PORT_SPREAD, SYS_PORT_SPREAD);
        (top + height * base_frac + offset).clamp(top + 6.0, top + height - 6.0)
    };

    let mut wires = Vec::with_capacity(candidates.len());
    let mut detour_lane_use: BTreeMap<i64, usize> = BTreeMap::new();
    for (cw_idx, cw) in candidates.into_iter().enumerate() {
        let f_mid_x = cw.fx + cw.fw * 0.5;
        let t_mid_x = cw.tx + cw.tw * 0.5;
        let same_x_col = (f_mid_x - t_mid_x).abs() < (cw.fw.min(cw.tw) * 0.25);
        let base_frac = 0.5;
        let (s_rank, s_total) = port_ranks.get(&(cw_idx, true)).copied().unwrap_or((0, 1));
        let (d_rank, d_total) = port_ranks.get(&(cw_idx, false)).copied().unwrap_or((0, 1));
        let sx = if cw.src_side == 1 {
            cw.fx + cw.fw
        } else {
            cw.fx
        };
        let ex = if cw.dst_side == 1 {
            cw.tx + cw.tw
        } else {
            cw.tx
        };
        let sy = spread_y(cw.fy, cw.fh, base_frac, s_rank, s_total);
        let ey = spread_y(cw.ty, cw.th, base_frac, d_rank, d_total);
        let dx = (ex - sx).abs().max(48.0) * 0.45;
        let (c1x, c2x) = if same_x_col {
            (sx + 44.0, ex + 44.0)
        } else if f_mid_x < t_mid_x {
            (sx + dx, ex - dx)
        } else {
            (sx - dx, ex + dx)
        };
        let _ = (cw.fcol, cw.tcol);
        let (path_d, mid_x, mid_y, max_route_x, max_route_y) = route_wire_avoiding_boxes(
            &placed_nodes,
            &cw.from,
            &cw.to,
            same_x_col,
            cw_idx,
            (sx, sy),
            (c1x, sy),
            (c2x, ey),
            (ex, ey),
            &mut detour_lane_use,
        );
        total_w = total_w.max(max_route_x + MARGIN);
        total_h = total_h.max(max_route_y + MARGIN);
        wires.push(PlacedSysWire {
            edge_index: cw.edge_index,
            path_d,
            mid_x,
            mid_y,
            back: cw.back,
        });
    }

    SysLayout {
        nodes: placed_nodes,
        frames,
        wires,
        tier_bands,
        width: total_w,
        height: total_h,
    }
}

const WIRE_OBSTACLE_PAD: f64 = 8.0;

fn point_in_padded_box(pt: (f64, f64), box_node: &PlacedSysNode, pad: f64) -> bool {
    pt.0 >= box_node.x - pad
        && pt.0 <= box_node.x + box_node.w + pad
        && pt.1 >= box_node.y - pad
        && pt.1 <= box_node.y + box_node.h + pad
}

fn segment_intersects_padded_box(
    a: (f64, f64),
    b: (f64, f64),
    box_node: &PlacedSysNode,
    pad: f64,
) -> bool {
    let x0 = box_node.x - pad;
    let y0 = box_node.y - pad;
    let x1 = box_node.x + box_node.w + pad;
    let y1 = box_node.y + box_node.h + pad;
    if a.0.max(b.0) < x0 || a.0.min(b.0) > x1 || a.1.max(b.1) < y0 || a.1.min(b.1) > y1 {
        return false;
    }
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let mut t0 = 0.0_f64;
    let mut t1 = 1.0_f64;
    for (p, q) in [
        (-dx, a.0 - x0),
        (dx, x1 - a.0),
        (-dy, a.1 - y0),
        (dy, y1 - a.1),
    ] {
        if p.abs() <= 1e-9 {
            if q < 0.0 {
                return false;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                if r > t1 {
                    return false;
                }
                t0 = t0.max(r);
            } else {
                if r < t0 {
                    return false;
                }
                t1 = t1.min(r);
            }
        }
    }
    t0 <= t1
}

fn cubic_intersects_obstacles(
    p0: (f64, f64),
    p1: (f64, f64),
    p2: (f64, f64),
    p3: (f64, f64),
    obstacles: &[&PlacedSysNode],
) -> bool {
    if obstacles.is_empty() {
        return false;
    }
    let steps = 14usize;
    let mut prev = p0;
    for i in 1..=steps {
        let t = (i as f64) / (steps as f64);
        let u = 1.0 - t;
        let pt = (
            u * u * u * p0.0 + 3.0 * u * u * t * p1.0 + 3.0 * u * t * t * p2.0 + t * t * t * p3.0,
            u * u * u * p0.1 + 3.0 * u * u * t * p1.1 + 3.0 * u * t * t * p2.1 + t * t * t * p3.1,
        );
        if obstacles
            .iter()
            .any(|b| segment_intersects_padded_box(prev, pt, b, WIRE_OBSTACLE_PAD))
        {
            return true;
        }
        prev = pt;
    }
    false
}

fn find_clear_horizontal_corridor(
    x_min: f64,
    x_max: f64,
    sy: f64,
    ey: f64,
    obstacles: &[&PlacedSysNode],
) -> f64 {
    let mut y_bands: Vec<(f64, f64)> = obstacles
        .iter()
        .filter(|b| b.x + b.w + WIRE_OBSTACLE_PAD >= x_min && b.x - WIRE_OBSTACLE_PAD <= x_max)
        .map(|b| (b.y - WIRE_OBSTACLE_PAD, b.y + b.h + WIRE_OBSTACLE_PAD))
        .collect();
    if y_bands.is_empty() {
        return 0.5 * (sy + ey);
    }
    y_bands.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));
    let mut merged: Vec<(f64, f64)> = Vec::with_capacity(y_bands.len());
    for (b0, b1) in y_bands {
        if let Some(last) = merged.last_mut() {
            if b0 <= last.1 + 10.0 {
                last.1 = last.1.max(b1);
                continue;
            }
        }
        merged.push((b0, b1));
    }

    let mut candidates = Vec::new();
    let top_y = merged[0].0 - 16.0;
    if top_y >= 10.0 {
        candidates.push(top_y);
    }
    for pair in merged.windows(2) {
        if pair[1].0 - pair[0].1 >= 12.0 {
            candidates.push(0.5 * (pair[0].1 + pair[1].0));
        }
    }
    let bottom_y = merged[merged.len() - 1].1 + 16.0;
    candidates.push(bottom_y);

    candidates
        .into_iter()
        .min_by(|a, b| {
            let cost_a = (a - sy).abs() + (a - ey).abs();
            let cost_b = (b - sy).abs() + (b - ey).abs();
            cost_a.total_cmp(&cost_b)
        })
        .unwrap_or(bottom_y)
}

#[allow(clippy::too_many_arguments)]
fn route_wire_avoiding_boxes(
    placed_nodes: &[PlacedSysNode],
    from_id: &str,
    to_id: &str,
    same_x_col: bool,
    cw_idx: usize,
    p0: (f64, f64),
    p1: (f64, f64),
    p2: (f64, f64),
    p3: (f64, f64),
    detour_lane_use: &mut BTreeMap<i64, usize>,
) -> (String, f64, f64, f64, f64) {
    let (sx, sy) = p0;
    let (c1x, c2x) = (p1.0, p2.0);
    let (ex, ey) = p3;
    let obstacles: Vec<&PlacedSysNode> = placed_nodes
        .iter()
        .filter(|b| {
            b.id != from_id
                && b.id != to_id
                && !point_in_padded_box(p0, b, 2.0)
                && !point_in_padded_box(p3, b, 2.0)
        })
        .collect();

    if same_x_col {
        let y_lo = sy.min(ey) - 4.0;
        let y_hi = sy.max(ey) + 4.0;
        let x_rightmost = obstacles
            .iter()
            .filter(|b| {
                b.y + b.h + WIRE_OBSTACLE_PAD >= y_lo
                    && b.y - WIRE_OBSTACLE_PAD <= y_hi
                    && b.x + b.w >= sx.min(ex) - 24.0
                    && b.x <= c1x.max(c2x) + 12.0
            })
            .map(|b| b.x + b.w)
            .reduce(f64::max);
        let hits = cubic_intersects_obstacles(p0, p1, p2, p3, &obstacles);
        if !hits && x_rightmost.is_none() {
            let path_d =
                format!("M{sx:.1},{sy:.1} C{c1x:.1},{sy:.1} {c2x:.1},{ey:.1} {ex:.1},{ey:.1}");
            let mid_x = 0.125 * sx + 0.375 * c1x + 0.375 * c2x + 0.125 * ex;
            return (
                path_d,
                mid_x,
                0.5 * (sy + ey),
                mid_x.max(sx).max(ex),
                sy.max(ey),
            );
        }
        let stagger = ((cw_idx % 4) as f64) * 10.0;
        let clear_x = x_rightmost
            .map(|rx| (rx + 30.0 + stagger).max(sx.max(ex) + 44.0 + stagger))
            .unwrap_or_else(|| c1x.max(c2x) + 28.0 + stagger);
        let path_d =
            format!("M{sx:.1},{sy:.1} C{clear_x:.1},{sy:.1} {clear_x:.1},{ey:.1} {ex:.1},{ey:.1}");
        let mid_x = 0.125 * sx + 0.75 * clear_x + 0.125 * ex;
        return (path_d, mid_x, 0.5 * (sy + ey), clear_x, sy.max(ey));
    }

    if !cubic_intersects_obstacles(p0, p1, p2, p3, &obstacles) {
        let path_d = format!("M{sx:.1},{sy:.1} C{c1x:.1},{sy:.1} {c2x:.1},{ey:.1} {ex:.1},{ey:.1}");
        let mid_x = 0.125 * sx + 0.375 * c1x + 0.375 * c2x + 0.125 * ex;
        return (path_d, mid_x, 0.5 * (sy + ey), sx.max(ex), sy.max(ey));
    }

    // Route through the nearest unobstructed horizontal inter-row / outer corridor.
    let dir = if ex >= sx { 1.0 } else { -1.0 };
    let span = (ex - sx).abs();
    let lead = (span * 0.16).clamp(18.0, 42.0);
    let gx0 = sx + dir * lead;
    let gx1 = ex - dir * lead;
    let base_lane_y =
        find_clear_horizontal_corridor(gx0.min(gx1) - 4.0, gx0.max(gx1) + 4.0, sy, ey, &obstacles);
    let bucket = base_lane_y.round() as i64;
    let slot = *detour_lane_use.get(&bucket).unwrap_or(&0);
    detour_lane_use.insert(bucket, slot + 1);
    let step = slot.div_ceil(2) as f64 * 4.0;
    let signed_offset = if slot == 0 {
        0.0
    } else if slot % 2 == 1 {
        -step
    } else {
        step
    };
    let candidate_y = (base_lane_y + signed_offset).max(10.0);
    let lane_y = if obstacles
        .iter()
        .any(|b| segment_intersects_padded_box((gx0, candidate_y), (gx1, candidate_y), b, 4.0))
    {
        base_lane_y
    } else {
        candidate_y
    };

    let c0x = sx + dir * lead * 0.55;
    let c3x = ex - dir * lead * 0.55;
    let path_d = format!(
        "M{sx:.1},{sy:.1} C{c0x:.1},{sy:.1} {c0x:.1},{lane_y:.1} {gx0:.1},{lane_y:.1} L{gx1:.1},{lane_y:.1} C{c3x:.1},{lane_y:.1} {c3x:.1},{ey:.1} {ex:.1},{ey:.1}"
    );
    let mid_x = 0.5 * (gx0 + gx1);
    (
        path_d,
        mid_x,
        lane_y,
        sx.max(ex).max(gx0).max(gx1),
        sy.max(ey).max(lane_y),
    )
}

pub(super) fn place_sys_edge_label(
    placed: &mut Vec<(f64, f64)>,
    candidate: (f64, f64),
) -> (f64, f64) {
    let clashes = |x: f64, y: f64, p: &(f64, f64)| (x - p.0).abs() < 56.0 && (y - p.1).abs() < 18.0;
    for dx in [0.0, 42.0, -42.0, 84.0, -84.0] {
        let x = (candidate.0 + dx).max(32.0);
        for step in 0..8i32 {
            let offsets: &[f64] = if step == 0 {
                &[0.0]
            } else {
                &[18.0 * step as f64, -18.0 * step as f64]
            };
            for &dy in offsets {
                let y = candidate.1 + dy;
                if y >= 10.0 && !placed.iter().any(|p| clashes(x, y, p)) {
                    placed.push((x, y));
                    return (x, y);
                }
            }
        }
    }
    placed.push(candidate);
    candidate
}

pub(super) fn live_drag_wire_path(
    (fx, fy, fw, fh): (f64, f64, f64, f64),
    (tx, ty, tw, th): (f64, f64, f64, f64),
) -> (String, f64, f64) {
    let f_mid_x = fx + fw * 0.5;
    let t_mid_x = tx + tw * 0.5;
    let same_x_col = (f_mid_x - t_mid_x).abs() < (fw.min(tw) * 0.25);
    let sx = if same_x_col || f_mid_x < t_mid_x {
        fx + fw
    } else {
        fx
    };
    let ex = if same_x_col {
        tx + tw
    } else if f_mid_x < t_mid_x {
        tx
    } else {
        tx + tw
    };
    let sy = fy + fh * 0.5;
    let ey = ty + th * 0.5;
    let dx = (ex - sx).abs().max(48.0) * 0.45;
    let (c1x, c2x) = if same_x_col {
        (sx + 44.0, ex + 44.0)
    } else if f_mid_x < t_mid_x {
        (sx + dx, ex - dx)
    } else {
        (sx - dx, ex + dx)
    };
    let path_d = format!("M{sx:.1},{sy:.1} C{c1x:.1},{sy:.1} {c2x:.1},{ey:.1} {ex:.1},{ey:.1}");
    let mid_x = 0.125 * sx + 0.375 * c1x + 0.375 * c2x + 0.125 * ex;
    let mid_y = 0.5 * (sy + ey);
    (path_d, mid_x, mid_y)
}
