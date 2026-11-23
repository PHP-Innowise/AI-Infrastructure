use super::*;

pub fn contract_arrow_legend_html_for(surface: SysMapSurface, doc: &V, state: &V) -> String {
    let hidden = hidden_contract_kinds(state);
    let counts = contract_kind_counts(doc);
    let has_counts = counts.iter().any(|c| *c > 0);
    let any_hidden = hidden.iter().any(|h| *h);
    let mut html = String::from(
        "<div class=\"graph-arrow-legend system-arrow-legend\" role=\"list\" aria-label=\"Cross-service contract and dependency arrow types\"><span class=\"graph-arrow-legend-title\">Links:</span>",
    );
    for (idx, spec) in CONTRACT_ARROW_SPECS.iter().enumerate() {
        let shown = !hidden[idx];
        let count = counts[idx];
        let dash = spec
            .dash_array
            .map(|d| format!(" stroke-dasharray=\"{d}\""))
            .unwrap_or_default();
        let payload = payload_attr(&json!({
            "surface": surface.id,
            "kind": spec.kind,
        }));
        let title = if has_counts {
            format!(
                "{} · {} links (click to {}, Alt+click to isolate)",
                spec.description,
                count,
                if shown { "hide" } else { "show" }
            )
        } else {
            format!(
                "{} (click to {}, Alt+click to isolate)",
                spec.description,
                if shown { "hide" } else { "show" }
            )
        };
        let count_html = if count > 0 {
            format!("<span class=\"graph-arrow-legend-count\">{count}</span>")
        } else {
            String::new()
        };
        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-arrow-legend-item\" role=\"listitem\" data-action=\"system-map-kind-toggle\" data-surface=\"{}\" data-payload=\"{payload}\" data-sys-edge-kind=\"{}\" aria-pressed=\"{shown}\" title=\"{}\"><svg class=\"graph-arrow-swatch system-arrow-swatch\" data-sys-edge-kind=\"{}\" viewBox=\"0 0 30 12\" width=\"30\" height=\"12\" aria-hidden=\"true\"><line class=\"graph-arrow-swatch-line\" x1=\"2\" y1=\"6\" x2=\"{:.1}\" y2=\"6\"{dash}/><g transform=\"translate(20 2)\" class=\"system-map-arrow system-map-arrow-{}\">{}</g></svg><span>{}</span>{count_html}</button>",
            surface.id,
            spec.kind,
            escape(&title),
            spec.kind,
            spec.swatch_line_end,
            spec.kind,
            spec.marker_body,
            spec.label,
        ));
    }
    if any_hidden {
        let all_payload = payload_attr(&json!({
            "surface": surface.id,
            "preset": "all",
        }));
        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-arrow-legend-preset\" data-action=\"system-map-kind-preset\" data-surface=\"{}\" data-payload=\"{all_payload}\" title=\"Show all contract and code link types\">Show all</button>",
            surface.id,
        ));
    }
    html.push_str(
        "<span class=\"graph-arrow-legend-item system-legend-risk\" role=\"listitem\" title=\"Unmatched consumer contract or incompatible contract version\"><span class=\"system-legend-halo system-legend-halo-broken\" aria-hidden=\"true\"></span><span>Broken risk</span></span>",
    );
    html.push_str(
        "<span class=\"graph-arrow-legend-item system-legend-drift\" role=\"listitem\" title=\"Code imports or calls cross service boundaries without a declared contract\"><span class=\"system-legend-halo system-legend-halo-drift\" aria-hidden=\"true\"></span><span>Undeclared drift</span></span>",
    );
    html.push_str(
        "<span class=\"graph-arrow-legend-item system-legend-against\" role=\"listitem\" title=\"Upward dependency against service tier layering or closing a cross-service cycle (↻)\"><span class=\"system-legend-halo system-legend-halo-against\" aria-hidden=\"true\"></span><span>Layer violation ↻</span></span>",
    );
    html.push_str("</div>");
    html
}

/// Accessible legend of all 7 cross-service contract & code arrow kinds plus drift/risk halos.
#[cfg_attr(not(test), allow(dead_code))]
pub fn contract_arrow_legend_html() -> String {
    contract_arrow_legend_html_for(SERVICES_SURFACE, &V::Null, &default_surface_state())
}

pub(super) fn last_wave(blast: &BlastRadius) -> usize {
    blast.wave_by_service.values().copied().max().unwrap_or(0)
}
/// Steps through the waves of a change: all of them, or the change as far
/// as one wave, the services a later wave reaches left faint; and a button
/// that plays them one after another. Nothing where the change reaches no
/// service past what it starts in.
pub fn wave_scrubber_html(surface: SysMapSurface, blast: &BlastRadius, state: &V) -> String {
    let last = last_wave(blast);
    if last == 0 {
        return String::new();
    }
    let shown = state["wave"].as_u64().map(|wave| wave as usize);
    let count = |wave: usize| {
        blast
            .wave_by_service
            .values()
            .filter(|reached| **reached == wave)
            .count()
    };
    let mut html = String::from("<div class=\"system-waves\" role=\"group\" aria-label=\"Waves of the change\"><span class=\"graph-arrow-legend-title\">Waves:</span>");
    html.push_str(&format!(
        "<button type=\"button\" class=\"graph-arrow-legend-preset\" data-action=\"system-map-wave\" data-payload=\"{}\" aria-pressed=\"{}\" title=\"Every service the change reaches\">All</button>",
        payload_attr(&json!({"surface": surface.id, "wave": null})),
        shown.is_none(),
    ));
    for wave in 0..=last {
        let told = match wave {
            0 => format!("Where the change starts: {}", plural_services(count(0))),
            wave => format!(
                "As far as wave {wave}: {} more",
                plural_services(count(wave))
            ),
        };
        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-arrow-legend-preset\" data-action=\"system-map-wave\" data-payload=\"{}\" aria-pressed=\"{}\" title=\"{}\">{wave}</button>",
            payload_attr(&json!({"surface": surface.id, "wave": wave})),
            shown == Some(wave),
            escape(&told),
        ));
    }
    let playing = SYS_WAVE_PLAYING.with(|w| w.borrow().contains(surface.id));
    if playing {
        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-arrow-legend-preset\" data-action=\"system-map-wave-pause\" data-payload=\"{}\" aria-pressed=\"true\" title=\"Pause wave animation\">⏸ Pause</button></div>",
            payload_attr(&json!({"surface": surface.id})),
        ));
    } else {
        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-arrow-legend-preset\" data-action=\"system-map-wave-play\" data-payload=\"{}\" title=\"Show the waves one after another\">▶ Play</button></div>",
            payload_attr(&json!({"surface": surface.id})),
        ));
    }
    html
}
pub(super) fn plural_services(count: usize) -> String {
    if count == 1 {
        "1 service".to_owned()
    } else {
        format!("{count} services")
    }
}
/// Shows the change as far as one wave, then the next, until the last.
pub(super) fn play_wave(surface: SysMapSurface, wave: usize) {
    let mut state = surface_state(surface);
    let view_doc = filtered_doc(&m::data(surface.map_key), &state);
    let last = last_wave(&blast_of(&view_doc, &state));
    let clamped = wave.min(last);
    if clamped < last {
        SYS_WAVE_PLAYING.with(|w| {
            w.borrow_mut().insert(surface.id);
        });
    } else {
        SYS_WAVE_PLAYING.with(|w| {
            w.borrow_mut().remove(surface.id);
        });
    }
    state["wave"] = json!(clamped);
    save_surface_state(surface, &state);
    render_surface(surface, false);
    if wave < last {
        dom::later(
            &format!("{}-wave-play", surface.id_prefix),
            900,
            move || {
                if SYS_WAVE_PLAYING.with(|w| w.borrow().contains(surface.id)) {
                    play_wave(surface, wave + 1);
                }
            },
        );
    }
}
pub(super) struct SysVisualState {
    pub(super) focus_id: String,
    pub(super) sel_edge_pair: (String, String),
    pub(super) blast: BlastRadius,
    pub(super) shown_wave: Option<usize>,
    pub(super) step_states: V,
    pub(super) unopened_consumers: BTreeSet<String>,
    pub(super) highlight_nodes: BTreeSet<String>,
    pub(super) highlight_edges: BTreeSet<(String, String)>,
    pub(super) failed_services: BTreeSet<String>,
}

impl SysVisualState {
    fn beyond(&self, service: &str) -> bool {
        self.shown_wave.is_some_and(|shown| {
            self.blast
                .wave_by_service
                .get(service)
                .is_some_and(|wave| *wave > shown)
        })
    }

    fn blast_role(&self, id: &str) -> &'static str {
        if self.beyond(id) {
            ""
        } else if self.blast.seeds.contains(id) {
            "seed"
        } else if self.blast.direct_consumers.contains(id) {
            "direct"
        } else if self.blast.downstream_consumers.contains(id) {
            "downstream"
        } else {
            ""
        }
    }

    fn step_state(&self, id: &str) -> String {
        if self.step_states.is_object() {
            m::text(&self.step_states, id)
        } else {
            String::new()
        }
    }

    fn is_changed(&self, id: &str) -> bool {
        !self.beyond(id)
            && (self.blast.seeds.contains(id)
                || self.blast.direct_consumers.contains(id)
                || self.blast.downstream_consumers.contains(id)
                || !self.step_state(id).is_empty())
    }
}

pub(super) fn compute_sys_visual_state(view_doc: &V, state: &V) -> SysVisualState {
    let overlay = &state["overlay"];
    let seeds: Vec<String> = m::array(&overlay["seed_services"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let changed_contracts: Vec<String> = m::array(&overlay["changed_contracts"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let depth = overlay["depth"].as_u64().unwrap_or(32) as usize;
    let blast = compute_blast_radius(view_doc, &seeds, &changed_contracts, depth);
    let shown_wave = state["wave"].as_u64().map(|wave| wave as usize);
    let step_states = overlay["step_states"].clone();
    let unopened_consumers: BTreeSet<String> = m::array(&overlay["cross_service_unopened"])
        .into_iter()
        .filter_map(|v| {
            v.as_str().map(str::to_owned).or_else(|| {
                let s = first_non_empty(&v, &["consumer_service", "service", "consumer"]);
                (!s.is_empty()).then_some(s)
            })
        })
        .collect();

    let edges = m::array(&view_doc["edges"]);
    let tool = m::text(state, "tool");
    let mut highlight_nodes = BTreeSet::new();
    let mut highlight_edges = BTreeSet::new();

    if tool == "path" {
        let p_from = m::text(state, "path_from");
        let p_to = m::text(state, "path_to");
        let skip: BTreeSet<String> = m::array(&state["skip"])
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        for path in shortest_service_paths(view_doc, &p_from, &p_to, &skip) {
            for w in path.windows(2) {
                highlight_nodes.insert(w[0].clone());
                highlight_nodes.insert(w[1].clone());
                highlight_edges.insert((w[0].clone(), w[1].clone()));
            }
        }
    } else if tool == "shared" {
        let sa = m::text(state, "shared_a");
        let sb = m::text(state, "shared_b");
        let (both_use, uses_both) = shared_neighbors(view_doc, &sa, &sb);
        let active_list = if m::text(state, "shared_side") == "uses_both" {
            uses_both
        } else {
            both_use
        };
        if !sa.is_empty() && !sb.is_empty() {
            highlight_nodes.insert(sa.clone());
            highlight_nodes.insert(sb.clone());
            for mid in active_list {
                highlight_nodes.insert(mid.clone());
                highlight_edges.insert((sa.clone(), mid.clone()));
                highlight_edges.insert((sb.clone(), mid.clone()));
                highlight_edges.insert((mid.clone(), sa.clone()));
                highlight_edges.insert((mid.clone(), sb.clone()));
            }
        }
    } else if tool == "cycle" {
        let cycles = find_cross_service_cycles(view_doc);
        let idx = state["cycle_index"].as_u64().unwrap_or(0) as usize;
        if let Some(comp) = cycles.get(idx).or_else(|| cycles.first()) {
            let set: BTreeSet<String> = comp.iter().cloned().collect();
            for s in &set {
                highlight_nodes.insert(s.clone());
            }
            for e in &edges {
                let f = m::text(e, "from");
                let t = m::text(e, "to");
                if set.contains(&f) && set.contains(&t) {
                    highlight_edges.insert((f, t));
                }
            }
        }
    } else if tool == "follow" {
        let root = m::text(state, "follow");
        if !root.is_empty() {
            highlight_nodes.insert(root.clone());
            let dir = m::text(state, "follow_dir");
            let hops: BTreeSet<String> =
                m::array(&state[if dir == "into" { "into" } else { "out" }])
                    .into_iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .chain(std::iter::once(root))
                    .collect();
            for e in &edges {
                let f = m::text(e, "from");
                let t = m::text(e, "to");
                if dir == "into" && hops.contains(&t) {
                    highlight_nodes.insert(f.clone());
                    highlight_edges.insert((f, t));
                } else if dir != "into" && hops.contains(&f) {
                    highlight_nodes.insert(t.clone());
                    highlight_edges.insert((f, t));
                }
            }
        }
    }

    let focus_id = m::text(state, "focus");
    let sel_edge = &state["selected_edge"];
    let sel_edge_pair = sel_edge
        .as_object()
        .map(|o| {
            (
                o.get("from").and_then(V::as_str).unwrap_or("").to_owned(),
                o.get("to").and_then(V::as_str).unwrap_or("").to_owned(),
            )
        })
        .unwrap_or_default();
    let failed_services: BTreeSet<String> = m::array(&m::data("system-failed-services"))
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();

    SysVisualState {
        focus_id,
        sel_edge_pair,
        blast,
        shown_wave,
        step_states,
        unopened_consumers,
        highlight_nodes,
        highlight_edges,
        failed_services,
    }
}

/// Renders the SVG minimap radar for a laid-out system graph.
pub fn system_radar_html(doc: &V, state: &V, layout: &SysLayout) -> String {
    let (rw, rh) = radar_size((layout.width, layout.height));
    if rw <= 0.0 || layout.nodes.is_empty() {
        return String::new();
    }
    let vis_doc = filtered_doc(doc, state);
    let vis = compute_sys_visual_state(&vis_doc, state);
    let focus_id = vis.focus_id.clone();

    let mut svg = format!(
        "<svg class=\"graph-radar-picture\" width=\"{rw:.0}\" height=\"{rh:.0}\" viewBox=\"0 0 {:.0} {:.0}\" preserveAspectRatio=\"none\" data-w=\"{:.0}\" data-h=\"{:.0}\" focusable=\"false\">",
        layout.width, layout.height, layout.width, layout.height
    );
    for frame in &layout.frames {
        svg.push_str(&format!(
            "<rect class=\"graph-radar-frame\" x=\"{:.0}\" y=\"{:.0}\" width=\"{:.0}\" height=\"{:.0}\"/>",
            frame.x, frame.y, frame.w, frame.h
        ));
    }
    for placed in &layout.nodes {
        let beyond = vis.beyond(&placed.id);
        let failed =
            vis.failed_services.contains(&placed.id) || vis.step_state(&placed.id) == "failed";
        let active_wave = !beyond
            && (vis.blast.seeds.contains(&placed.id)
                || vis.blast.direct_consumers.contains(&placed.id)
                || vis.blast.downstream_consumers.contains(&placed.id)
                || !vis.step_state(&placed.id).is_empty());
        let state_attr = if placed.id == focus_id {
            " data-state=\"selected\""
        } else if failed || placed.cycle {
            " data-state=\"cycle\""
        } else if active_wave {
            " data-state=\"changed\""
        } else {
            ""
        };
        let cycle_attr = if placed.cycle {
            " data-cycle=\"true\""
        } else {
            ""
        };
        let failed_attr = if failed { " data-failed=\"true\"" } else { "" };
        let beyond_attr = if beyond { " data-beyond=\"true\"" } else { "" };
        svg.push_str(&format!(
            "<rect class=\"graph-radar-box\" data-n=\"{}\" x=\"{:.0}\" y=\"{:.0}\" width=\"{:.0}\" height=\"{:.0}\"{cycle_attr}{failed_attr}{beyond_attr}{state_attr}/>",
            escape(&placed.id),
            placed.x,
            placed.y,
            placed.w,
            placed.h
        ));
    }
    svg.push_str(
        "<rect class=\"graph-radar-view\" x=\"0\" y=\"0\" width=\"0\" height=\"0\"/></svg>",
    );
    svg
}

/// Renders the interactive SVG + HTML plane AND its minimap radar for `render_surface`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn render_canvas_with_radar(
    surface: SysMapSurface,
    doc: &V,
    state: &V,
) -> (String, String, f64, f64) {
    let view_doc = filtered_doc(doc, state);
    let layers_mode = m::yes(&state["layers"]);
    let pins = graph_viewport::load_pins(&sys_pin_key(surface, state));
    let layout = layout_system_nodes_with_pins(&view_doc, layers_mode, &pins);
    let plane_html = render_canvas_html_with_layout(surface, &view_doc, state, &layout);
    let radar_svg = system_radar_html(doc, state, &layout);
    (plane_html, radar_svg, layout.width, layout.height)
}

/// Renders the interactive SVG + HTML plane for `#system-map-canvas` or `#system-change-topology-canvas`.
#[cfg_attr(not(test), allow(dead_code))]
pub fn render_canvas_html(surface: SysMapSurface, doc: &V, state: &V) -> (String, f64, f64) {
    let view_doc = filtered_doc(doc, state);
    let layers_mode = m::yes(&state["layers"]);
    let layout = layout_system_nodes(&view_doc, layers_mode);
    let html = render_canvas_html_with_layout(surface, &view_doc, state, &layout);
    (html, layout.width, layout.height)
}

pub(super) struct SysRenderBundle {
    pub(super) html: String,
    pub(super) is_dense: bool,
    pub(super) node_htmls: Vec<String>,
    pub(super) node_seeds: Vec<bool>,
    pub(super) edge_endpoints: Vec<(String, String)>,
    pub(super) active_roving_id: String,
    pub(super) highlight_nodes: BTreeSet<String>,
    pub(super) failed_services: BTreeSet<String>,
    pub(super) pinned_ids: BTreeSet<String>,
}

impl SysRenderBundle {
    #[allow(dead_code)]
    pub(super) fn materialize_node_html(
        &self,
        surface: SysMapSurface,
        view_doc: &V,
        state: &V,
        layout: &SysLayout,
        idx: usize,
    ) -> Option<String> {
        if let Some(existing) = self.node_htmls.get(idx).filter(|s| !s.is_empty()) {
            return Some(existing.clone());
        }
        if !self.node_seeds.get(idx).copied().unwrap_or(false) {
            return None;
        }
        let placed = layout.nodes.get(idx)?;
        let nodes = m::array(&view_doc["nodes"]);
        let node = nodes.iter().find(|n| node_id(n) == placed.id)?;
        let vis = compute_sys_visual_state(view_doc, state);
        let pins = graph_viewport::load_pins(&sys_pin_key(surface, state));
        Some(render_single_system_node_html(
            surface,
            placed,
            node,
            &vis,
            &pins,
            idx,
            &self.active_roving_id,
            false,
        ))
    }
}

pub(super) fn render_canvas_html_with_layout(
    surface: SysMapSurface,
    view_doc: &V,
    state: &V,
    layout: &SysLayout,
) -> String {
    render_canvas_bundle_with_layout(surface, view_doc, state, layout).html
}

pub(super) fn render_canvas_bundle_with_layout(
    surface: SysMapSurface,
    view_doc: &V,
    state: &V,
    layout: &SysLayout,
) -> SysRenderBundle {
    let layers_mode = m::yes(&state["layers"]);
    let view_doc = view_doc.clone();
    let nodes = m::array(&view_doc["nodes"]);
    let edges = m::array(&view_doc["edges"]);

    if nodes.is_empty() {
        return SysRenderBundle {
            html: String::new(),
            is_dense: false,
            node_htmls: Vec::new(),
            node_seeds: Vec::new(),
            edge_endpoints: Vec::new(),
            active_roving_id: String::new(),
            highlight_nodes: BTreeSet::new(),
            failed_services: BTreeSet::new(),
            pinned_ids: BTreeSet::new(),
        };
    }

    let mut node_by_id = BTreeMap::new();
    for n in &nodes {
        node_by_id.insert(node_id(n), n);
    }

    let vis = compute_sys_visual_state(&view_doc, state);
    let (_, layer_violations) = assign_service_layers(&view_doc);
    let violation_set: BTreeSet<(String, String)> = layer_violations.into_iter().collect();
    let focus_id = vis.focus_id.clone();
    let sel_edge_pair = vis.sel_edge_pair.clone();

    let is_dense = layout.nodes.len() >= crate::graph_map::DENSE_GRAPH_NODE_THRESHOLD
        || layout.wires.len() >= crate::graph_map::DENSE_GRAPH_EDGE_THRESHOLD;
    let tier_attr = if is_dense {
        " data-scale-tier=\"dense\""
    } else {
        ""
    };
    let mut html = format!(
        "<div class=\"graph-map-plane system-map-plane\" data-surface=\"{}\"{tier_attr} style=\"width:{:.0}px;height:{:.0}px\"><svg width=\"{:.0}\" height=\"{:.0}\" role=\"group\" aria-label=\"Links between services\"><defs>{}</defs>",
        surface.id,
        layout.width,
        layout.height,
        layout.width,
        layout.height,
        contract_arrow_defs_svg(surface.id_prefix),
    );

    if !is_dense {
        // Architectural tier bands in Layers mode (Canvas2D paints these via meta.containers in dense mode)
        for (tier_label, bx, bw, bh) in &layout.tier_bands {
            let band_h = (bh - MARGIN + 12.0).max(120.0);
            html.push_str(&format!(
                "<g class=\"system-tier-band\"><rect class=\"graph-map-arch-band system-arch-band\" x=\"{bx:.1}\" y=\"{:.1}\" width=\"{bw:.1}\" height=\"{band_h:.1}\" rx=\"8\"/><text class=\"system-arch-label\" x=\"{:.1}\" y=\"{:.1}\">{}</text></g>",
                MARGIN - 4.0,
                bx + 10.0,
                MARGIN + 14.0,
                escape(tier_label),
            ));
        }

        // Unfolded service frames
        for frame in &layout.frames {
            html.push_str(&format!(
                "<g class=\"system-service-frame\" data-of=\"{}\"><rect class=\"graph-map-frame system-frame-rect\" x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" rx=\"10\"/><text class=\"system-frame-title\" x=\"{:.1}\" y=\"{:.1}\">{}</text></g>",
                escape(&frame.service_id),
                frame.x,
                frame.y,
                frame.w,
                frame.h,
                frame.x + 12.0,
                frame.y + 18.0,
                escape(&frame.title),
            ));
        }
    }

    let hidden_kinds = hidden_contract_kinds(state);
    let mut placed_labels: Vec<(f64, f64)> = Vec::new();
    let mut edge_endpoints: Vec<(String, String)> = Vec::with_capacity(layout.wires.len());

    // Edges (halos + visible strokes + wide click/contextmenu hit targets when non-dense)
    for wire in &layout.wires {
        let Some(edge) = edges.get(wire.edge_index) else {
            edge_endpoints.push((String::new(), String::new()));
            continue;
        };
        let from = m::text(edge, "from");
        let to = m::text(edge, "to");
        edge_endpoints.push((from.clone(), to.clone()));
        if is_dense {
            continue;
        }
        let kind = primary_edge_kind(edge);
        let kind_idx = contract_spec_index(kind);
        let spec = spec_for_kind(kind);
        let prov = m::text(edge, "provenance");
        let broken = m::yes(&edge["broken_risk"]);
        let against = m::yes(&edge["against"])
            || (layers_mode && violation_set.contains(&(from.clone(), to.clone())));
        let kind_hidden = hidden_kinds[kind_idx] && !against && !broken;
        let hidden_attr = if kind_hidden {
            " data-kind-hidden=\"true\""
        } else {
            ""
        };
        let is_selected = sel_edge_pair.0 == from && sel_edge_pair.1 == to && !from.is_empty();
        let is_highlighted = vis.highlight_edges.contains(&(from.clone(), to.clone()));

        if against {
            html.push_str(&format!(
                "<path class=\"graph-map-violation-halo system-map-violation-halo\" d=\"{}\" data-sys-edge-kind=\"{kind}\" data-from=\"{}\" data-to=\"{}\"{}/>",
                wire.path_d,
                escape(&from),
                escape(&to),
                if is_selected || is_highlighted { " data-edge-state=\"on\"" } else { "" }
            ));
        }
        if broken {
            html.push_str(&format!(
                "<path class=\"system-map-broken-halo\" d=\"{}\" data-sys-edge-kind=\"{kind}\" data-from=\"{}\" data-to=\"{}\"{hidden_attr}/>",
                wire.path_d,
                escape(&from),
                escape(&to),
            ));
        }

        let dash_attr = if prov == "undeclared_in_code" {
            " stroke-dasharray=\"4 3\"".to_owned()
        } else if let Some(d) = spec.dash_array {
            format!(" stroke-dasharray=\"{d}\"")
        } else {
            String::new()
        };
        // A link the change has not passed yet, at the wave shown.
        let dash_attr = if vis.beyond(&from) {
            format!("{dash_attr} data-beyond=\"true\"")
        } else {
            dash_attr
        };

        let marker_id = if against {
            format!("{}-arrow-against", surface.id_prefix)
        } else if broken {
            format!("{}-arrow-broken", surface.id_prefix)
        } else {
            format!("{}-arrow-{kind}", surface.id_prefix)
        };

        let mut edge_classes = format!("graph-map-edge system-map-edge system-map-edge-{kind}");
        if against {
            edge_classes.push_str(" graph-map-edge-against");
        }
        if is_selected || is_highlighted {
            edge_classes.push_str(" graph-map-edge-on");
        }

        html.push_str(&format!(
            "<path class=\"{edge_classes}\" d=\"{}\" data-sys-edge-kind=\"{kind}\" data-provenance=\"{}\" data-broken-risk=\"{}\" data-from=\"{}\" data-to=\"{}\"{dash_attr}{hidden_attr}{} marker-end=\"url(#{marker_id})\"/>",
            wire.path_d,
            escape(&prov),
            if broken { "true" } else { "false" },
            escape(&from),
            escape(&to),
            if is_highlighted { " data-shared-path=\"true\"" } else { "" },
        ));

        let contract_count = m::array(&edge["contracts"]).len();
        let pill_text = if broken {
            format!("! {}", spec.label)
        } else if prov == "undeclared_in_code" {
            "drift".to_owned()
        } else if contract_count > 1 {
            format!("{} · {contract_count}", spec.label)
        } else {
            spec.label.to_owned()
        };

        if !m::yes(&edge["child_link"]) {
            let (lx, ly) = place_sys_edge_label(&mut placed_labels, (wire.mid_x, wire.mid_y));
            let pill_w = (pill_text.chars().count() as f64 * 6.2 + 14.0).max(54.0);
            html.push_str(&format!(
                "<g class=\"graph-map-edge-tie system-map-edge-label\" aria-hidden=\"true\" data-sys-edge-kind=\"{kind}\" data-from=\"{}\" data-to=\"{}\"{}{hidden_attr}><rect x=\"{:.1}\" y=\"{:.1}\" width=\"{pill_w:.1}\" height=\"16\" rx=\"8\"/><text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\">{}</text></g>",
                escape(&from),
                escape(&to),
                if is_selected || is_highlighted { " data-edge-state=\"on\"" } else { "" },
                lx - pill_w * 0.5,
                ly - 8.0,
                lx,
                ly + 3.5,
                escape(&pill_text),
            ));
        }

        let explain_svcs = explain_services_for_endpoints(&from, &to, Some(&node_by_id));
        let hit_payload = payload_attr(&json!({
            "surface": surface.id,
            "from": from,
            "to": to,
            "kind": kind,
            "provenance": prov,
            "broken_risk": broken,
            "contracts": edge["contracts"],
        }));
        let aria = format!("{from} to {to} ({} link)", spec.label);
        html.push_str(&format!(
            "<path class=\"graph-map-hit system-map-hit\" d=\"{}\" role=\"button\" aria-label=\"{}\" data-action=\"system-map-edge\" data-sys-edge-kind=\"{kind}\" data-payload=\"{hit_payload}\" data-explain-services=\"{}\" data-from=\"{}\" data-to=\"{}\"{hidden_attr}><title>{}</title></path>",
            wire.path_d,
            escape(&aria),
            escape(&explain_svcs),
            escape(&from),
            escape(&to),
            escape(&aria),
        ));
    }

    html.push_str("</svg>");

    let pins = graph_viewport::load_pins(&sys_pin_key(surface, state));
    let cam = load_camera(surface);
    let (vw, vh, _) = viewport_size(surface.canvas_id);
    let visible_world = (cam.scale > 0.05).then(|| {
        (
            -cam.pan_x / cam.scale,
            -cam.pan_y / cam.scale,
            vw / cam.scale,
            vh / cam.scale,
        )
    });
    let active_roving_id = {
        let f = m::text(state, "roving_focus");
        if !f.is_empty() && layout.nodes.iter().any(|n| n.id == f) {
            f
        } else if !focus_id.is_empty() && layout.nodes.iter().any(|n| n.id == focus_id) {
            focus_id.clone()
        } else {
            layout
                .nodes
                .first()
                .map(|n| n.id.clone())
                .unwrap_or_default()
        }
    };
    let mut node_htmls = if is_dense {
        Vec::with_capacity(layout.nodes.len())
    } else {
        Vec::new()
    };
    let mut node_seeds = if is_dense {
        Vec::with_capacity(layout.nodes.len())
    } else {
        Vec::new()
    };
    let pinned_ids: BTreeSet<String> = pins.keys().cloned().collect();

    // Interactive service & unfolded child HTML node cards
    for (node_roving_idx, placed) in layout.nodes.iter().enumerate() {
        let Some(&node) = node_by_id.get(&placed.id) else {
            if is_dense {
                node_htmls.push(String::new());
                node_seeds.push(false);
            }
            continue;
        };
        let offscreen = visible_world.is_some_and(|(vx, vy, vw, vh)| {
            let pad = 120.0;
            placed.x + placed.w < vx - pad
                || placed.x > vx + vw + pad
                || placed.y + placed.h < vy - pad
                || placed.y > vy + vh + pad
        });
        let parent_svc = placed
            .child_of
            .clone()
            .unwrap_or_else(|| m::text(node, "service"));
        let svc_id = if parent_svc.is_empty() {
            placed.id.clone()
        } else {
            parent_svc
        };
        let is_important = focus_id == placed.id
            || vis.highlight_nodes.contains(&placed.id)
            || pins.contains_key(&placed.id)
            || vis.failed_services.contains(&placed.id)
            || vis.failed_services.contains(&svc_id)
            || active_roving_id == placed.id;
        if is_dense {
            // Defer dense node HTML allocation until mounted in DOM or needed by roving focus.
            node_htmls.push(String::new());
            node_seeds.push(true);
            if offscreen && !is_important {
                continue;
            }
        }
        let node_html = render_single_system_node_html(
            surface,
            placed,
            node,
            &vis,
            &pins,
            node_roving_idx,
            &active_roving_id,
            is_dense && offscreen && !is_important,
        );
        html.push_str(&node_html);
    }

    html.push_str("</div>");
    SysRenderBundle {
        html,
        is_dense,
        node_htmls,
        node_seeds,
        edge_endpoints,
        active_roving_id,
        highlight_nodes: vis.highlight_nodes,
        failed_services: vis.failed_services,
        pinned_ids,
    }
}

#[allow(clippy::too_many_arguments)]
fn render_single_system_node_html(
    surface: SysMapSurface,
    placed: &super::layout::PlacedSysNode,
    node: &V,
    vis: &SysVisualState,
    pins: &BTreeMap<String, (f64, f64)>,
    node_roving_idx: usize,
    active_roving_id: &str,
    mark_offscreen: bool,
) -> String {
    let focus_id = vis.focus_id.as_str();
    let kind = node["kind"].as_str().unwrap_or("service");
    let name = {
        let n = m::text(node, "name");
        if n.is_empty() {
            placed.id.clone()
        } else {
            n
        }
    };
    let parent_svc = placed
        .child_of
        .clone()
        .unwrap_or_else(|| m::text(node, "service"));
    let svc_id = if parent_svc.is_empty() {
        placed.id.clone()
    } else {
        parent_svc
    };
    let is_selected = focus_id == placed.id;
    let is_highlighted = vis.highlight_nodes.contains(&placed.id);
    let is_pinned = pins.contains_key(&placed.id);
    let is_failed =
        vis.failed_services.contains(&placed.id) || vis.failed_services.contains(&svc_id);
    let is_roving = active_roving_id == placed.id;
    let tabindex = if is_roving { "0" } else { "-1" };

    let waiting = vis.beyond(&placed.id);
    let blast_role = vis.blast_role(&placed.id);
    let step_state = vis.step_state(&placed.id);
    let unopened_warn = vis.unopened_consumers.contains(&placed.id);
    let graph_state = m::text(&node["graph_status"], "state");

    let payload = payload_attr(&json!({
        "surface": surface.id,
        "path": placed.id,
        "id": placed.id,
        "service": svc_id,
        "kind": kind,
    }));

    let child_extra = format!(
        "{}{}{} tabindex=\"{tabindex}\" data-roving-index=\"{node_roving_idx}\"",
        if is_pinned {
            " data-pinned=\"true\""
        } else {
            ""
        },
        if is_failed {
            " data-failed=\"true\""
        } else {
            ""
        },
        if mark_offscreen {
            " data-offscreen=\"true\""
        } else {
            ""
        },
    );

    if let Some(parent_id) = placed.child_of.as_deref() {
        let badge = match kind {
            "contract" => "C",
            "capability" => "Cap",
            "source" => "Src",
            "more" => "...",
            _ => "•",
        };
        let title_text = if kind == "more" {
            let d = m::text(node, "detail");
            if d.is_empty() {
                name.clone()
            } else {
                d
            }
        } else {
            name.clone()
        };
        let child_payload = if kind == "more" {
            payload_attr(&json!({
                "surface": surface.id,
                "path": parent_id,
                "id": parent_id,
                "service": parent_id,
                "kind": "service",
            }))
        } else {
            payload
        };
        format!(
            "<button type=\"button\" class=\"graph-map-node system-map-node system-map-child\" data-n=\"{}\" data-parent=\"{}\" data-kind=\"{}\" data-selected=\"{}\"{}{child_extra} style=\"left:{:.1}px;top:{:.1}px;width:{:.1}px;height:{:.1}px\" data-action=\"system-map-node\" data-payload=\"{child_payload}\" data-explain-services=\"{}\" title=\"{}\" aria-label=\"{}\"><span class=\"system-child-badge\">{}</span><span class=\"system-node-title\">{}</span></button>",
            escape(&placed.id),
            escape(parent_id),
            escape(kind),
            if is_selected { "true" } else { "false" },
            if is_highlighted {
                " data-shared-path=\"true\""
            } else {
                ""
            },
            placed.x,
            placed.y,
            placed.w,
            placed.h,
            escape(&svc_id),
            escape(&title_text),
            escape(&title_text),
            escape(badge),
            escape(&name),
        )
    } else {
        let mut meta_parts = Vec::new();
        let lang = m::text(node, "language");
        if !lang.is_empty() {
            meta_parts.push(lang);
        }
        let provides_n = m::array(&node["provides"]).len();
        if provides_n > 0 {
            meta_parts.push(format!("{provides_n} contracts"));
        } else if placed.id == "__system__" {
            let sh = m::array(&node["shared_sources"]).len();
            meta_parts.push(format!("{sh} shared docs"));
        }
        if !step_state.is_empty() {
            meta_parts.push(step_state.clone());
        } else if !blast_role.is_empty() {
            meta_parts.push(match blast_role {
                "seed" => "seed".to_owned(),
                "direct" => "wave 1".to_owned(),
                _ => format!(
                    "wave {}",
                    vis.blast
                        .wave_by_service
                        .get(&placed.id)
                        .copied()
                        .unwrap_or(2)
                ),
            });
        }
        let meta_line = meta_parts.join(" · ");

        let status_dot_state = if !step_state.is_empty() {
            step_state.as_str()
        } else if !graph_state.is_empty() {
            graph_state.as_str()
        } else {
            "idle"
        };

        let cycle_badge = if placed.cycle {
            "<span class=\"system-cycle-mark\" title=\"Participates in a cross-service dependency cycle\">↻</span>"
        } else {
            ""
        };
        let unopened_badge = if unopened_warn {
            "<span class=\"system-unopened-mark\" title=\"Unopened downstream contract consumer\">!</span>"
        } else {
            ""
        };

        let aria_label = if meta_line.is_empty() {
            name.clone()
        } else {
            format!("{name} ({meta_line})")
        };

        let mut node_extra = String::new();
        if is_pinned {
            push_raw_attr(&mut node_extra, "data-pinned", "true");
        }
        if is_failed {
            push_raw_attr(&mut node_extra, "data-failed", "true");
        }
        if mark_offscreen {
            push_raw_attr(&mut node_extra, "data-offscreen", "true");
        }
        push_attr(&mut node_extra, "tabindex", tabindex);
        push_raw_attr(
            &mut node_extra,
            "data-roving-index",
            &node_roving_idx.to_string(),
        );

        format!(
            "<button type=\"button\" class=\"graph-map-node system-map-node\" data-n=\"{}\" data-kind=\"{}\" data-selected=\"{}\" data-blast=\"{}\" data-step-state=\"{}\" data-unopened=\"{}\" data-graph-state=\"{}\"{}{}{} style=\"left:{:.1}px;top:{:.1}px;width:{:.1}px;height:{:.1}px\" data-action=\"system-map-node\" data-payload=\"{payload}\" data-explain-services=\"{}\" title=\"{} — double-click to open service code map\" aria-label=\"{}\"><span class=\"system-node-top\"><span class=\"system-node-dot\" data-state=\"{}\" aria-hidden=\"true\"></span><span class=\"system-node-title\">{}</span>{cycle_badge}{unopened_badge}</span><span class=\"system-node-meta\">{}</span></button>",
            escape(&placed.id),
            escape(kind),
            if is_selected { "true" } else { "false" },
            blast_role,
            escape(&step_state),
            if unopened_warn { "true" } else { "false" },
            escape(&graph_state),
            node_extra,
            if is_highlighted {
                " data-shared-path=\"true\""
            } else {
                ""
            },
            if waiting {
                " data-beyond=\"true\""
            } else {
                ""
            },
            placed.x,
            placed.y,
            placed.w,
            placed.h,
            escape(&svc_id),
            escape(&name),
            escape(&aria_label),
            escape(status_dot_state),
            escape(&name),
            escape(&meta_line),
        )
    }
}

thread_local! {
    pub(super) static SYS_SURFACE_DIGEST: RefCell<std::collections::HashMap<&'static str, u64>> =
        RefCell::new(std::collections::HashMap::new());
}

#[derive(Clone, Debug)]
pub(super) struct SysDenseCacheEntry {
    pub(super) surface: SysMapSurface,
    pub(super) view_doc: V,
    pub(super) state: V,
    pub(super) layout: SysLayout,
    pub(super) node_ids: Vec<String>,
    pub(super) node_index_by_id: HashMap<String, usize>,
    pub(super) node_htmls: Vec<String>,
    pub(super) node_seeds: Vec<bool>,
    pub(super) edge_endpoints: Vec<(String, String)>,
    pub(super) is_dense: bool,
    pub(super) latest_roving_id: String,
    pub(super) latest_focus_id: String,
    pub(super) highlight_nodes: BTreeSet<String>,
    pub(super) highlight_edges: BTreeSet<(String, String)>,
    pub(super) sel_edge_pair: (String, String),
    pub(super) failed_services: BTreeSet<String>,
    pub(super) pinned_ids: BTreeSet<String>,
    pub(super) blast_roles: HashMap<String, (&'static str, String, bool)>,
}

impl SysDenseCacheEntry {
    pub(super) fn materialize_node_html(&self, idx: usize) -> Option<String> {
        if let Some(existing) = self.node_htmls.get(idx).filter(|s| !s.is_empty()) {
            return Some(existing.clone());
        }
        if !self.node_seeds.get(idx).copied().unwrap_or(false) {
            return None;
        }
        let placed = self.layout.nodes.get(idx)?;
        let nodes = m::array(&self.view_doc["nodes"]);
        let node = nodes.iter().find(|n| node_id(n) == placed.id)?;
        let vis = compute_sys_visual_state(&self.view_doc, &self.state);
        let pins = graph_viewport::load_pins(&sys_pin_key(self.surface, &self.state));
        Some(render_single_system_node_html(
            self.surface,
            placed,
            node,
            &vis,
            &pins,
            idx,
            &self.latest_roving_id,
            false,
        ))
    }
}

thread_local! {
    pub(super) static SYS_DENSE_CACHE: RefCell<HashMap<&'static str, SysDenseCacheEntry>> =
        RefCell::new(HashMap::new());
}

pub(super) fn patch_system_node_el(
    el: &web_sys::Element,
    nid: &str,
    focus_id: &str,
    target_roving: &str,
    highlight_nodes: &BTreeSet<String>,
    failed_services: &BTreeSet<String>,
    blast_roles: &HashMap<String, (&'static str, String, bool)>,
) {
    let svc_id = el
        .get_attribute("data-explain-services")
        .unwrap_or_default();
    let _ = el.set_attribute(
        "data-selected",
        if nid == focus_id { "true" } else { "false" },
    );
    if highlight_nodes.contains(nid) {
        let _ = el.set_attribute("data-shared-path", "true");
    } else {
        let _ = el.remove_attribute("data-shared-path");
    }
    let is_failed = failed_services.contains(nid)
        || (!svc_id.is_empty() && failed_services.contains(svc_id.as_str()));
    if is_failed {
        let _ = el.set_attribute("data-failed", "true");
    } else {
        let _ = el.remove_attribute("data-failed");
    }
    if !target_roving.is_empty() {
        let _ = el.set_attribute("tabindex", if nid == target_roving { "0" } else { "-1" });
    }
    if let Some((blast_role, step_state, waiting)) = blast_roles.get(nid) {
        if *waiting {
            let _ = el.set_attribute("data-beyond", "true");
        } else {
            let _ = el.remove_attribute("data-beyond");
        }
        if !el.class_list().contains("system-map-child") {
            let _ = el.set_attribute("data-blast", blast_role);
            let _ = el.set_attribute("data-step-state", step_state);
            if let Ok(Some(dot)) = el.query_selector(".system-node-dot") {
                if !step_state.is_empty() {
                    let _ = dot.set_attribute("data-state", step_state);
                } else {
                    let fallback = el
                        .get_attribute("data-graph-state")
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "idle".to_owned());
                    let _ = dot.set_attribute("data-state", &fallback);
                }
            }
        }
    } else {
        let _ = el.remove_attribute("data-beyond");
        if !el.class_list().contains("system-map-child") {
            let _ = el.set_attribute("data-blast", "");
            let _ = el.set_attribute("data-step-state", "");
            if let Ok(Some(dot)) = el.query_selector(".system-node-dot") {
                let fallback = el
                    .get_attribute("data-graph-state")
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "idle".to_owned());
                let _ = dot.set_attribute("data-state", &fallback);
            }
        }
    }
}

pub(super) fn is_important_system_node(
    entry: &SysDenseCacheEntry,
    nid: &str,
    el: Option<&web_sys::Element>,
) -> bool {
    let failed = entry.failed_services.contains(nid)
        || el
            .and_then(|e| e.get_attribute("data-explain-services"))
            .is_some_and(|svc| entry.failed_services.contains(&svc));
    let el_important = el.is_some_and(|e| {
        e.get_attribute("data-selected").as_deref() == Some("true")
            || e.get_attribute("data-shared-path").as_deref() == Some("true")
            || e.get_attribute("data-pinned").as_deref() == Some("true")
            || e.get_attribute("data-failed").as_deref() == Some("true")
            || e.get_attribute("tabindex").as_deref() == Some("0")
    });
    el_important
        || nid == entry.latest_focus_id
        || nid == entry.latest_roving_id
        || entry.highlight_nodes.contains(nid)
        || entry.pinned_ids.contains(nid)
        || failed
}

pub(super) fn mount_system_dense_node_if_needed(
    surface: SysMapSurface,
    nid: &str,
) -> Option<web_sys::Element> {
    let canvas = dom::get(surface.canvas_id)?;
    if let Some(existing) = find_sys_node_el(&canvas, nid) {
        let _ = existing.remove_attribute("data-offscreen");
        return Some(existing);
    }
    let plane = canvas.query_selector(".system-map-plane").ok().flatten()?;
    let entry = SYS_DENSE_CACHE.with(|c| c.borrow().get(surface.canvas_id).cloned())?;
    if !entry.is_dense {
        return None;
    }
    let &idx = entry.node_index_by_id.get(nid)?;
    let html = entry.materialize_node_html(idx)?;
    let mut created = graph_viewport::append_html_elements(&plane, &html);
    let el = created.pop()?;
    let _ = el.remove_attribute("data-offscreen");
    patch_system_node_el(
        &el,
        nid,
        &entry.latest_focus_id,
        &entry.latest_roving_id,
        &entry.highlight_nodes,
        &entry.failed_services,
        &entry.blast_roles,
    );
    invalidate_hover_cache(surface.canvas_id);
    Some(el)
}

pub(super) fn sync_system_viewport_culling(surface: SysMapSurface, cam: &Camera) {
    let Some(entry) = SYS_DENSE_CACHE.with(|c| c.borrow().get(surface.canvas_id).cloned()) else {
        return;
    };
    let nodes = dom::all(&format!("#{} .system-map-node[data-n]", surface.canvas_id));
    let (vw, vh, measured) = viewport_size(surface.canvas_id);
    let bounds = measured
        .then(|| graph_viewport::world_viewport_bounds(cam.viewport_cam(), (vw, vh), 0.0))
        .flatten();
    let Some((min_x, min_y, max_x, max_y)) = bounds else {
        for el in nodes {
            let _ = el.remove_attribute("data-offscreen");
        }
        return;
    };
    let mut mounted_els: HashMap<usize, web_sys::Element> = HashMap::with_capacity(nodes.len());
    for el in nodes {
        if let Some(nid) = el.get_attribute("data-n") {
            if let Some(&idx) = entry.node_index_by_id.get(&nid) {
                mounted_els.insert(idx, el);
            }
        }
    }

    if entry.is_dense && entry.node_seeds.len() == entry.layout.nodes.len() {
        if let Some(plane) = dom::get(surface.canvas_id)
            .and_then(|c| c.query_selector(".system-map-plane").ok().flatten())
        {
            let rects: Vec<(f64, f64, f64, f64)> = entry
                .layout
                .nodes
                .iter()
                .map(|n| (n.x, n.y, n.w, n.h))
                .collect();
            let entry_ref = &entry;
            let outcome = graph_viewport::dense_dom::reconcile_dense_nodes(
                &plane,
                cam.viewport_cam(),
                (vw, vh),
                &rects,
                120.0,
                0.0,
                160.0,
                &mut mounted_els,
                |idx, el| {
                    let Some(n) = entry_ref.layout.nodes.get(idx) else {
                        return false;
                    };
                    if !entry_ref.node_seeds.get(idx).copied().unwrap_or(false) {
                        return false;
                    }
                    is_important_system_node(entry_ref, &n.id, el)
                },
                |idx| entry_ref.materialize_node_html(idx),
                |idx, el| {
                    let _ = el.remove_attribute("data-offscreen");
                    if let Some(placed) = entry_ref.layout.nodes.get(idx) {
                        patch_system_node_el(
                            el,
                            &placed.id,
                            &entry_ref.latest_focus_id,
                            &entry_ref.latest_roving_id,
                            &entry_ref.highlight_nodes,
                            &entry_ref.failed_services,
                            &entry_ref.blast_roles,
                        );
                    }
                },
                |_| {},
            );
            if outcome.dom_mutated {
                invalidate_hover_cache(surface.canvas_id);
            }
        }
        return;
    }

    for (idx, el) in mounted_els {
        let Some(placed) = entry.layout.nodes.get(idx) else {
            continue;
        };
        let important = is_important_system_node(&entry, &placed.id, Some(&el));
        let offscreen = !important
            && (placed.x + placed.w < min_x - 120.0
                || placed.x > max_x + 120.0
                || placed.y + placed.h < min_y - 120.0
                || placed.y > max_y + 120.0);
        if offscreen {
            let _ = el.set_attribute("data-offscreen", "true");
        } else {
            let _ = el.remove_attribute("data-offscreen");
        }
    }
}

pub(super) fn build_blast_roles_map(
    nodes: &[PlacedSysNode],
    vis: &SysVisualState,
) -> HashMap<String, (&'static str, String, bool)> {
    nodes
        .iter()
        .map(|n| {
            (
                n.id.clone(),
                (
                    vis.blast_role(&n.id),
                    vis.step_state(&n.id),
                    vis.beyond(&n.id),
                ),
            )
        })
        .collect()
}

pub(super) fn refresh_dense_scene_for_system(
    scene: &mut graph_viewport::canvas2d::DenseScene,
    vis: &SysVisualState,
    node_ids: &[String],
    edge_endpoints: &[(String, String)],
) {
    let mut focus_neighbors: HashSet<&str> = HashSet::new();
    if !vis.focus_id.is_empty() {
        for (f, t) in edge_endpoints {
            if f == &vis.focus_id {
                focus_neighbors.insert(t.as_str());
            } else if t == &vis.focus_id {
                focus_neighbors.insert(f.as_str());
            }
        }
    }
    for node in &mut scene.nodes {
        let nid = node_ids
            .get(node.index)
            .map(String::as_str)
            .unwrap_or(node.label.as_str());
        let beyond = vis.beyond(nid);
        node.selected = nid == vis.focus_id;
        let adjacent_to_focus = !vis.focus_id.is_empty() && focus_neighbors.contains(nid);
        node.lit = node.selected || adjacent_to_focus || vis.highlight_nodes.contains(nid);
        node.dim = beyond
            || (!vis.highlight_nodes.is_empty() && !vis.highlight_nodes.contains(nid))
            || (vis.highlight_nodes.is_empty() && !vis.focus_id.is_empty() && !node.lit);
        node.changed = !beyond
            && (vis.is_changed(nid)
                || matches!(
                    vis.step_state(nid).as_str(),
                    "seed" | "direct" | "downstream" | "running" | "passed"
                ));
        node.failed = vis.failed_services.contains(nid)
            || vis.step_state(nid) == "failed"
            || (!node.role.is_empty()
                && node.role != "service"
                && node.role != "system"
                && vis.failed_services.contains(node.role.as_str()));
    }
    for edge in &mut scene.edges {
        let (from, to) = edge_endpoints
            .get(edge.index)
            .map(|(f, t)| (f.as_str(), t.as_str()))
            .unwrap_or(("", ""));
        let beyond_edge = !from.is_empty() && vis.beyond(from);
        let is_hl = !from.is_empty()
            && !to.is_empty()
            && vis
                .highlight_edges
                .contains(&(from.to_owned(), to.to_owned()));
        let is_sel_edge = !vis.sel_edge_pair.0.is_empty()
            && from == vis.sel_edge_pair.0
            && to == vis.sel_edge_pair.1;
        let is_focus_adj = !vis.focus_id.is_empty() && (from == vis.focus_id || to == vis.focus_id);
        edge.on = is_hl || is_sel_edge || is_focus_adj;
        edge.lit = is_hl || is_sel_edge;
        edge.dim = beyond_edge
            || (!vis.highlight_edges.is_empty() && !is_hl)
            || (vis.highlight_edges.is_empty() && !vis.focus_id.is_empty() && !edge.on);
    }
}

pub(super) fn try_patch_system_canvas_in_place(
    surface: SysMapSurface,
    view_doc: &V,
    state: &V,
) -> bool {
    let Some(canvas) = dom::get(surface.canvas_id) else {
        return false;
    };
    if canvas
        .query_selector(".system-map-plane")
        .ok()
        .flatten()
        .is_none()
    {
        return false;
    }
    let vis = compute_sys_visual_state(view_doc, state);
    let focus_id = vis.focus_id.clone();
    let sel_from = vis.sel_edge_pair.0.clone();
    let sel_to = vis.sel_edge_pair.1.clone();
    let roving = m::text(state, "roving_focus");
    let target_roving = if !roving.is_empty() {
        roving
    } else {
        focus_id.clone()
    };

    let mut blast_roles = HashMap::new();
    let mut node_ids = Vec::new();
    let mut edge_endpoints = Vec::new();
    let mut is_dense = false;
    SYS_DENSE_CACHE.with(|cache| {
        if let Some(entry) = cache.borrow().get(surface.canvas_id) {
            is_dense = entry.is_dense;
            blast_roles = build_blast_roles_map(&entry.layout.nodes, &vis);
            node_ids = entry.node_ids.clone();
            edge_endpoints = entry.edge_endpoints.clone();
        }
    });
    if blast_roles.is_empty() {
        for n in m::array(&view_doc["nodes"]) {
            let nid = node_id(&n);
            blast_roles.insert(
                nid.clone(),
                (vis.blast_role(&nid), vis.step_state(&nid), vis.beyond(&nid)),
            );
        }
    }

    if let Ok(nodes) = canvas.query_selector_all(".system-map-node[data-n]") {
        for i in 0..nodes.length() {
            let Some(el) = nodes
                .item(i)
                .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
            else {
                continue;
            };
            let nid = el.get_attribute("data-n").unwrap_or_default();
            patch_system_node_el(
                &el,
                &nid,
                &focus_id,
                &target_roving,
                &vis.highlight_nodes,
                &vis.failed_services,
                &blast_roles,
            );
        }
    }

    SYS_DENSE_CACHE.with(|cache| {
        if let Some(entry) = cache.borrow_mut().get_mut(surface.canvas_id) {
            entry.latest_focus_id = focus_id.clone();
            if !target_roving.is_empty() {
                entry.latest_roving_id = target_roving.clone();
            }
            entry.highlight_nodes = vis.highlight_nodes.clone();
            entry.highlight_edges = vis.highlight_edges.clone();
            entry.sel_edge_pair = vis.sel_edge_pair.clone();
            entry.failed_services = vis.failed_services.clone();
            entry.blast_roles = blast_roles.clone();
        }
    });

    if is_dense {
        if !focus_id.is_empty() {
            let _ = mount_system_dense_node_if_needed(surface, &focus_id);
        }
        if !target_roving.is_empty() && target_roving != focus_id {
            let _ = mount_system_dense_node_if_needed(surface, &target_roving);
        }
        graph_viewport::canvas2d::update_scene(surface.canvas_id, |scene| {
            refresh_dense_scene_for_system(scene, &vis, &node_ids, &edge_endpoints);
        });
    } else {
        let hidden_kinds = hidden_contract_kinds(state);
        let layers_mode = m::yes(&state["layers"]);
        let (_, layer_violations) = assign_service_layers(view_doc);
        let violation_set: BTreeSet<(String, String)> = layer_violations.into_iter().collect();
        let mut edge_flags: HashMap<(String, String, String), (bool, bool)> = HashMap::new();
        for edge in m::array(&view_doc["edges"]) {
            let f = m::text(&edge, "from");
            let t = m::text(&edge, "to");
            let k = primary_edge_kind(&edge).to_owned();
            let broken = m::yes(&edge["broken_risk"]);
            let against = m::yes(&edge["against"])
                || (layers_mode && violation_set.contains(&(f.clone(), t.clone())));
            let flags = edge_flags.entry((f, t, k)).or_insert((false, false));
            flags.0 |= against;
            flags.1 |= broken;
        }
        if let Ok(edges) = canvas.query_selector_all(".system-map-edge[data-from][data-to]") {
            for i in 0..edges.length() {
                let Some(el) = edges
                    .item(i)
                    .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
                else {
                    continue;
                };
                let from = el.get_attribute("data-from").unwrap_or_default();
                let to = el.get_attribute("data-to").unwrap_or_default();
                let kind = el.get_attribute("data-sys-edge-kind").unwrap_or_default();
                let against = el.class_list().contains("graph-map-edge-against");
                let broken = el.get_attribute("data-broken-risk").as_deref() == Some("true");
                let kind_hidden = hidden_kinds[contract_spec_index(&kind)] && !against && !broken;
                if kind_hidden {
                    let _ = el.set_attribute("data-kind-hidden", "true");
                } else {
                    let _ = el.remove_attribute("data-kind-hidden");
                }
                let is_hl = vis.highlight_edges.contains(&(from.clone(), to.clone()));
                let is_sel = !sel_from.is_empty() && from == sel_from && to == sel_to;
                let cl = el.class_list();
                let _ = if is_hl || is_sel {
                    cl.add_1("graph-map-edge-on")
                } else {
                    cl.remove_1("graph-map-edge-on")
                };
                if is_hl {
                    let _ = el.set_attribute("data-shared-path", "true");
                } else {
                    let _ = el.remove_attribute("data-shared-path");
                }
                if vis.beyond(&from) {
                    let _ = el.set_attribute("data-beyond", "true");
                } else {
                    let _ = el.remove_attribute("data-beyond");
                }
            }
        }
        for sel in [
            ".system-map-violation-halo",
            ".system-map-broken-halo",
            ".system-map-edge-label",
            ".system-map-hit",
        ] {
            if let Ok(els) = canvas.query_selector_all(&format!("{sel}[data-from][data-to]")) {
                for i in 0..els.length() {
                    let Some(el) = els
                        .item(i)
                        .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
                    else {
                        continue;
                    };
                    let from = el.get_attribute("data-from").unwrap_or_default();
                    let to = el.get_attribute("data-to").unwrap_or_default();
                    let kind = el.get_attribute("data-sys-edge-kind").unwrap_or_default();
                    if sel != ".system-map-violation-halo" {
                        let (against, broken) = edge_flags
                            .get(&(from.clone(), to.clone(), kind.clone()))
                            .copied()
                            .unwrap_or((false, false));
                        let kind_hidden =
                            hidden_kinds[contract_spec_index(&kind)] && !against && !broken;
                        if kind_hidden {
                            let _ = el.set_attribute("data-kind-hidden", "true");
                        } else {
                            let _ = el.remove_attribute("data-kind-hidden");
                        }
                    }
                    if sel == ".system-map-violation-halo" || sel == ".system-map-edge-label" {
                        let is_hl = vis.highlight_edges.contains(&(from.clone(), to.clone()));
                        let is_sel =
                            (!sel_from.is_empty() && from == sel_from && to == sel_to) || is_hl;
                        if is_sel {
                            let _ = el.set_attribute("data-edge-state", "on");
                        } else {
                            let _ = el.remove_attribute("data-edge-state");
                        }
                    }
                }
            }
        }
    }

    for box_el in dom::all(&format!(
        "#{}-radar .graph-radar-box[data-n]",
        surface.canvas_id
    )) {
        let nid = box_el.get_attribute("data-n").unwrap_or_default();
        let beyond = vis.beyond(&nid);
        let failed = vis.failed_services.contains(&nid) || vis.step_state(&nid) == "failed";
        let active_wave = !beyond
            && (vis.blast.seeds.contains(&nid)
                || vis.blast.direct_consumers.contains(&nid)
                || vis.blast.downstream_consumers.contains(&nid)
                || !vis.step_state(&nid).is_empty());
        let next_state = if nid == focus_id {
            Some("selected")
        } else if failed || box_el.get_attribute("data-cycle").as_deref() == Some("true") {
            Some("cycle")
        } else if active_wave {
            Some("changed")
        } else {
            None
        };
        if let Some(st) = next_state {
            let _ = box_el.set_attribute("data-state", st);
        } else {
            let _ = box_el.remove_attribute("data-state");
        }
        if failed {
            let _ = box_el.set_attribute("data-failed", "true");
        } else {
            let _ = box_el.remove_attribute("data-failed");
        }
        if beyond {
            let _ = box_el.set_attribute("data-beyond", "true");
        } else {
            let _ = box_el.remove_attribute("data-beyond");
        }
    }
    true
}

#[cfg(test)]
pub(super) fn build_dense_scene_for_surface(
    surface: SysMapSurface,
    doc: &V,
    state: &V,
    layout: &SysLayout,
) -> Option<graph_viewport::canvas2d::DenseScene> {
    build_dense_scene_and_meta_for_surface(surface, doc, state, layout).map(|(scene, _)| scene)
}

pub(super) fn build_dense_scene_and_meta_for_surface(
    surface: SysMapSurface,
    doc: &V,
    state: &V,
    layout: &SysLayout,
) -> Option<(
    graph_viewport::canvas2d::DenseScene,
    graph_viewport::canvas2d::DenseSceneMeta,
)> {
    if layout.nodes.len() < crate::graph_map::DENSE_GRAPH_NODE_THRESHOLD
        && layout.wires.len() < crate::graph_map::DENSE_GRAPH_EDGE_THRESHOLD
    {
        return None;
    }
    let vis = compute_sys_visual_state(doc, state);
    let edges = m::array(&doc["edges"]);
    let pins = graph_viewport::load_pins(&sys_pin_key(surface, state));
    let layers_mode = m::yes(&state["layers"]);
    let hidden_kinds = hidden_contract_kinds(state);
    let (_, layer_violations) = assign_service_layers(doc);
    let violation_set: BTreeSet<(String, String)> = layer_violations.into_iter().collect();
    let doc_nodes = m::array(&doc["nodes"]);
    let mut node_by_id: BTreeMap<String, &V> = BTreeMap::new();
    let mut name_by_id: HashMap<String, String> = HashMap::new();
    for node in &doc_nodes {
        let id = node_id(node);
        if !id.is_empty() {
            node_by_id.insert(id.clone(), node);
        }
        let name = m::text(node, "name");
        if !id.is_empty() && !name.is_empty() {
            name_by_id.insert(id, name);
        }
    }
    let node_ids: Vec<String> = layout.nodes.iter().map(|n| n.id.clone()).collect();
    let mut node_index_by_id: HashMap<&str, usize> = HashMap::with_capacity(node_ids.len());
    let mut meta = graph_viewport::canvas2d::DenseSceneMeta::default();
    meta.containers
        .reserve(layout.tier_bands.len() + layout.frames.len());
    for (tier_label, bx, bw, bh) in &layout.tier_bands {
        meta.containers
            .push(graph_viewport::canvas2d::DenseContainerSpec {
                kind: graph_viewport::canvas2d::DenseContainerKind::TierBand,
                x: *bx,
                y: MARGIN - 4.0,
                w: *bw,
                h: (bh - MARGIN + 12.0).max(120.0),
                radius: 8.0,
                label: tier_label.clone(),
            });
    }
    for frame in &layout.frames {
        meta.containers
            .push(graph_viewport::canvas2d::DenseContainerSpec {
                kind: graph_viewport::canvas2d::DenseContainerKind::ServiceFrame,
                x: frame.x,
                y: frame.y,
                w: frame.w,
                h: frame.h,
                radius: 10.0,
                label: frame.title.clone(),
            });
    }
    let nodes = layout
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            node_index_by_id.insert(n.id.as_str(), i);
            let label = name_by_id
                .get(&n.id)
                .cloned()
                .unwrap_or_else(|| n.id.clone());
            let svc_id = endpoint_service_id(&n.id, Some(&node_by_id))
                .or_else(|| n.child_of.clone())
                .unwrap_or_else(|| n.id.clone());
            let node_services: Vec<String> = endpoint_service_id(&n.id, Some(&node_by_id))
                .into_iter()
                .collect();
            let node_kind = n.child_of.as_deref().map_or("service", |_| "contract");
            meta.nodes.insert(
                i,
                graph_viewport::canvas2d::DenseNodeMeta {
                    explain_kind: "node".to_owned(),
                    explain_payload: json!({
                        "id": n.id,
                        "path": n.id,
                        "service": svc_id,
                        "services": node_services,
                        "name": label,
                        "label": label,
                        "kind": node_kind,
                    }),
                    symbol_kind: node_kind.to_owned(),
                    tooltip: format!("{label} — double-click to open service code map"),
                    ..Default::default()
                },
            );
            graph_viewport::canvas2d::DenseNodeSpec {
                index: i,
                x: n.x,
                y: n.y,
                w: n.w,
                h: n.h,
                label,
                role: n.child_of.as_deref().unwrap_or("service").to_owned(),
                selected: n.id == vis.focus_id,
                lit: false,
                dim: false,
                changed: vis.is_changed(&n.id),
                pinned: pins.contains_key(&n.id),
                failed: vis.failed_services.contains(&n.id)
                    || n.child_of
                        .as_deref()
                        .is_some_and(|p| vis.failed_services.contains(p)),
            }
        })
        .collect();
    let mut edge_endpoints = vec![(String::new(), String::new()); layout.wires.len()];
    let dense_edges = layout
        .wires
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let edge = edges.get(w.edge_index);
            let from = edge.map(|e| m::text(e, "from")).unwrap_or_default();
            let to = edge.map(|e| m::text(e, "to")).unwrap_or_default();
            edge_endpoints[i] = (from.clone(), to.clone());
            let kind = edge.map(primary_edge_kind).unwrap_or("other").to_owned();
            let prov = edge.map(|e| m::text(e, "provenance")).unwrap_or_default();
            let broken = edge.is_some_and(|e| m::yes(&e["broken_risk"]));
            let against = edge.is_some_and(|e| m::yes(&e["against"]))
                || (layers_mode && violation_set.contains(&(from.clone(), to.clone())));
            let kind_hidden = hidden_kinds[contract_spec_index(&kind)] && !against && !broken;
            let path_for_scene = if kind_hidden { "" } else { w.path_d.as_str() };
            if !kind_hidden {
                let from_idx = node_index_by_id.get(from.as_str()).copied();
                let to_idx = node_index_by_id.get(to.as_str()).copied();
                let edge_services: Vec<String> =
                    explain_services_for_endpoints(&from, &to, Some(&node_by_id))
                        .split(',')
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .collect();
                meta.edges.insert(
                    i,
                    graph_viewport::canvas2d::DenseEdgeMeta {
                        explain_kind: "link".to_owned(),
                        explain_payload: json!({
                            "from": from,
                            "to": to,
                            "kind": kind,
                            "services": edge_services,
                            "provenance": prov,
                            "broken_risk": broken,
                            "against": against,
                        }),
                        from_index: from_idx,
                        to_index: to_idx,
                        summary: format!("{from} -> {to} ({kind})"),
                    },
                );
            }
            graph_viewport::canvas2d::DenseEdgeSpec::from_svg_path(
                i,
                path_for_scene,
                1,
                prov == "undeclared_in_code",
                against || broken,
                kind,
                false,
                false,
                false,
                broken,
            )
        })
        .collect();
    let mut scene = graph_viewport::canvas2d::DenseScene {
        width: layout.width,
        height: layout.height,
        nodes,
        edges: dense_edges,
    };
    refresh_dense_scene_for_system(&mut scene, &vis, &node_ids, &edge_endpoints);
    Some((scene, meta))
}

pub(crate) fn mark_failed_services(failed_ids: &[String]) {
    let arr: Vec<V> = failed_ids
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| json!(s))
        .collect();
    m::set("system-failed-services", V::Array(arr));
    if cfg!(target_arch = "wasm32") {
        for surface in [SERVICES_SURFACE, CHANGES_SURFACE] {
            if m::data(surface.map_key).is_object() {
                render_surface(surface, false);
            }
        }
    }
}

pub(super) fn render_surface(surface: SysMapSurface, refit: bool) {
    attach_wheel_listeners();
    ensure_hover_watch(surface);
    ensure_legend_hover_watch(surface);
    ensure_resize_watch();
    let state = surface_state(surface);
    let doc = m::data(surface.map_key);
    sync_toolbar_buttons(surface, &state);

    let svc_ids: Vec<String> = m::array(&doc["nodes"])
        .into_iter()
        .filter(|n| matches!(n["kind"].as_str(), Some("service" | "system")))
        .map(|n| node_id(&n))
        .filter(|s| !s.is_empty())
        .collect();
    let explain_scope = crate::graph::GraphTargetScope::SystemService {
        project_id: m::text(&state, "project_id"),
        config_path: m::text(&state, "config_path"),
        service_id: "__system__".to_owned(),
        services: svc_ids,
        run_id: {
            let rid = m::text(&state["overlay"], "run_id");
            (!rid.is_empty()).then_some(rid)
        },
        run_title: None,
        launches: Vec::new(),
    };
    crate::graph_explanation::surface_map_context(surface.canvas_id, &doc, &doc, &explain_scope);

    let view_doc = filtered_doc(&doc, &state);
    let pins = graph_viewport::load_pins(&sys_pin_key(surface, &state));
    let digest = system_structure_digest(&view_doc, &state, &pins);
    let prev_digest =
        SYS_SURFACE_DIGEST.with(|d| d.borrow().get(surface.canvas_id).copied().unwrap_or(0));
    let mut cam = load_camera(surface);

    let (pw, ph, needs_final_fit) = if !refit
        && digest != 0
        && digest == prev_digest
        && try_patch_system_canvas_in_place(surface, &view_doc, &state)
    {
        (cam.plane_w, cam.plane_h, false)
    } else {
        let layers_mode = m::yes(&state["layers"]);
        let layout = layout_system_nodes_with_pins(&view_doc, layers_mode, &pins);
        let pw = layout.width;
        let ph = layout.height;
        let size_changed = (cam.plane_w - pw).abs() > 1.0 || (cam.plane_h - ph).abs() > 1.0;
        cam.plane_w = pw;
        cam.plane_h = ph;
        if refit || cam.fit_mode || (size_changed && cam.needs_measure) {
            let (vw, vh, measured) = viewport_size(surface.canvas_id);
            cam.needs_measure = !measured;
            if pw > 0.0 && ph > 0.0 {
                cam.apply_viewport_cam(graph_viewport::fit(
                    (pw, ph),
                    (vw, vh),
                    graph_viewport::FitOptions::for_system_map(),
                ));
            }
            save_camera(surface, &cam);
        }
        let bundle = render_canvas_bundle_with_layout(surface, &view_doc, &state, &layout);
        let canvas_html = bundle.html;
        let radar_svg = system_radar_html(&view_doc, &state, &layout);
        invalidate_hover_cache(surface.canvas_id);
        let has_total_nodes = !m::array(&doc["nodes"]).is_empty();
        let combined_html = if canvas_html.is_empty() {
            let empty_inner = if has_total_nodes {
                let clear_p = payload_attr(&json!({ "surface": surface.id }));
                format!(
                    "<div class=\"system-empty\" role=\"status\"><p>No services match the active filters or isolation scope.</p><button type=\"button\" class=\"button compact primary\" data-action=\"system-map-clear-filters\" data-payload=\"{clear_p}\">Reset filters</button></div>"
                )
            } else {
                "<div class=\"system-empty\" role=\"status\"><p>No services or contracts mapped yet.</p></div>".to_owned()
            };
            format!(
                "<div class=\"graph-map-plane system-map-plane\" data-empty=\"true\" style=\"--graph-pan-x:0.0px;--graph-pan-y:0.0px;--graph-scale:1.0000;width:100%;height:100%;min-height:320px;transform:none;transition:none\">{empty_inner}</div>"
            )
        } else if radar_svg.is_empty() {
            canvas_html.clone()
        } else {
            format!(
                "{canvas_html}<div class=\"graph-map-radar system-map-radar\" id=\"{}-radar\" data-surface=\"{}\" aria-hidden=\"true\" hidden>{radar_svg}</div>",
                surface.canvas_id,
                surface.id,
            )
        };
        set_html(surface.canvas_id, &combined_html);
        SYS_SURFACE_DIGEST.with(|d| {
            d.borrow_mut().insert(surface.canvas_id, digest);
        });
        let vis_for_cache = compute_sys_visual_state(&view_doc, &state);
        let node_ids: Vec<String> = layout.nodes.iter().map(|n| n.id.clone()).collect();
        let node_index_by_id: HashMap<String, usize> = node_ids
            .iter()
            .enumerate()
            .map(|(i, id)| (id.clone(), i))
            .collect();
        let blast_roles = build_blast_roles_map(&layout.nodes, &vis_for_cache);
        let cache_entry = SysDenseCacheEntry {
            surface,
            view_doc: view_doc.clone(),
            state: state.clone(),
            layout: layout.clone(),
            node_ids,
            node_index_by_id,
            node_htmls: bundle.node_htmls,
            node_seeds: bundle.node_seeds,
            edge_endpoints: bundle.edge_endpoints,
            is_dense: bundle.is_dense,
            latest_roving_id: bundle.active_roving_id,
            latest_focus_id: vis_for_cache.focus_id.clone(),
            highlight_nodes: bundle.highlight_nodes,
            highlight_edges: vis_for_cache.highlight_edges.clone(),
            sel_edge_pair: vis_for_cache.sel_edge_pair.clone(),
            failed_services: bundle.failed_services,
            pinned_ids: bundle.pinned_ids,
            blast_roles,
        };
        SYS_DENSE_CACHE.with(|c| {
            c.borrow_mut().insert(surface.canvas_id, cache_entry);
        });
        if let Some((scene, meta)) =
            build_dense_scene_and_meta_for_surface(surface, &view_doc, &state, &layout)
        {
            graph_viewport::canvas2d::store_scene_with_meta(surface.canvas_id, scene, meta);
        } else {
            graph_viewport::canvas2d::clear_scene(surface.canvas_id);
        }
        dom::hidden(surface.legend_id, canvas_html.is_empty());
        (pw, ph, false)
    };
    set_html(
        surface.legend_id,
        &format!(
            "{}{}",
            wave_scrubber_html(surface, &blast_of(&view_doc, &state), &state),
            contract_arrow_legend_html_for(surface, &view_doc, &state)
        ),
    );

    let total_svcs = m::array(&doc["nodes"])
        .iter()
        .filter(|n| n["kind"] == "service")
        .count();
    let total_links = m::array(&doc["edges"])
        .iter()
        .filter(|e| !m::yes(&e["child_link"]))
        .count();
    let shown_svcs = m::array(&view_doc["nodes"])
        .iter()
        .filter(|n| n["kind"] == "service")
        .count();
    let shown_links = m::array(&view_doc["edges"])
        .iter()
        .filter(|e| !m::yes(&e["child_link"]))
        .count();
    let filtered = shown_svcs != total_svcs || shown_links != total_links;
    dom::text(
        surface.summary_id,
        &if total_svcs == 0 {
            String::new()
        } else if filtered {
            format!("{shown_svcs}/{total_svcs} services · {shown_links}/{total_links} links")
        } else {
            format!("{total_svcs} services · {total_links} links")
        },
    );

    let note = m::text(&doc, "note");
    dom::text(surface.note_id, &note);
    dom::hidden(surface.note_id, note.is_empty());

    set_html(surface.side_id, &render_side_html(surface, &doc, &state));

    if m::array(&view_doc["nodes"]).is_empty() {
        cam.plane_w = 0.0;
        cam.plane_h = 0.0;
        save_camera(surface, &cam);
        apply_camera(surface, &cam);
        return;
    }

    let size_changed = (cam.plane_w - pw).abs() > 1.0 || (cam.plane_h - ph).abs() > 1.0;
    cam.plane_w = pw;
    cam.plane_h = ph;
    if needs_final_fit || (size_changed && cam.needs_measure) {
        let (vw, vh, measured) = viewport_size(surface.canvas_id);
        cam.needs_measure = !measured;
        if pw > 0.0 && ph > 0.0 {
            cam.apply_viewport_cam(graph_viewport::fit(
                (pw, ph),
                (vw, vh),
                graph_viewport::FitOptions::for_system_map(),
            ));
        }
    }
    save_camera(surface, &cam);
    apply_camera(surface, &cam);
}
