use super::*;

/// Renders the rich `#system-map-side` or `#system-change-topology-side` inspector HTML.
pub fn render_side_html(surface: SysMapSurface, doc: &V, state: &V) -> String {
    let side_tab = match m::text(state, "side_tab").as_str() {
        "analyze" => "analyze".to_owned(),
        "changes" => "changes".to_owned(),
        _ => "details".to_owned(),
    };
    let nodes = m::array(&doc["nodes"]);
    let edges = m::array(&doc["edges"]);
    let services: Vec<V> = nodes
        .iter()
        .filter(|n| matches!(n["kind"].as_str(), Some("service" | "system") | None))
        .cloned()
        .collect();

    let (can_back, can_fwd) = SYS_NAV_HISTORY.with(|h| {
        h.borrow()
            .get(surface.id)
            .map_or((false, false), |nav| (nav.can_back(), nav.can_forward()))
    });
    let nav_p = payload_attr(&json!({ "surface": surface.id }));
    let mut html = format!(
        "<div class=\"system-nav-bar\" role=\"toolbar\" aria-label=\"Topology selection history\"><button type=\"button\" class=\"button compact\" data-action=\"system-map-nav-back\" data-payload=\"{nav_p}\"{} aria-label=\"Previous selection\">← Back</button><button type=\"button\" class=\"button compact\" data-action=\"system-map-nav-forward\" data-payload=\"{nav_p}\"{} aria-label=\"Next selection\">Forward →</button></div>",
        if can_back { "" } else { " disabled" },
        if can_fwd { "" } else { " disabled" },
    );
    html.push_str(
        "<div class=\"system-side-tabs\" role=\"tablist\" aria-label=\"Topology inspector views\">",
    );
    for (id, label) in [
        ("details", "Details"),
        ("analyze", "Analyze"),
        ("changes", "Changes & Impact"),
    ] {
        let selected = side_tab == id;
        let p = payload_attr(&json!({ "surface": surface.id, "tab": id }));
        html.push_str(&format!(
            "<button type=\"button\" class=\"graph-pick-tab system-side-tab\" role=\"tab\" aria-selected=\"{}\" tabindex=\"{}\" data-action=\"system-map-side-tab\" data-payload=\"{p}\">{}</button>",
            if selected { "true" } else { "false" },
            if selected { "0" } else { "-1" },
            escape(label),
        ));
    }
    html.push_str("</div>");

    if side_tab == "analyze" {
        html.push_str(&render_analyze_tab_html(surface, doc, state, &services));
        return html;
    }
    if side_tab == "changes" {
        html.push_str(&render_changes_tab_html(surface, doc, state));
        return html;
    }

    // Details tab: selected edge -> selected node -> system overview
    if let Some(sel) = state["selected_edge"].as_object() {
        let from = sel.get("from").and_then(V::as_str).unwrap_or("");
        let to = sel.get("to").and_then(V::as_str).unwrap_or("");
        if let Some(edge) = edges
            .iter()
            .find(|e| m::text(e, "from") == from && m::text(e, "to") == to)
        {
            html.push_str(&render_edge_details_html(surface, doc, edge));
            return html;
        }
    }

    let focus_id = m::text(state, "focus");
    if !focus_id.is_empty() {
        if let Some(node) = nodes.iter().find(|n| node_id(n) == focus_id) {
            html.push_str(&render_node_details_html(surface, doc, state, node));
            return html;
        }
    }

    html.push_str(&render_overview_side_html(surface, doc, state, &services));
    html
}

pub(super) fn render_overview_side_html(
    surface: SysMapSurface,
    doc: &V,
    state: &V,
    services: &[V],
) -> String {
    let edges = m::array(&doc["edges"]);
    let cycles = find_cross_service_cycles(doc);
    let broken_count = edges.iter().filter(|e| m::yes(&e["broken_risk"])).count();
    let drift_count = edges
        .iter()
        .filter(|e| m::text(e, "provenance") == "undeclared_in_code")
        .count();

    let mut html = String::from("<div class=\"system-side-section\">");
    html.push_str("<h4 class=\"system-side-heading\">System architecture</h4>");
    html.push_str("<p class=\"system-note\">Select a service or contract arrow to inspect schemas, provenance, and blast radius. Double-click any service to open its code map.</p>");

    html.push_str("<dl class=\"memory-metrics system-side-metrics\">");
    html.push_str(&format!(
        "<dt>Member services</dt><dd>{}</dd>",
        services
            .iter()
            .filter(|n| node_id(n) != "__system__")
            .count()
    ));
    html.push_str(&format!(
        "<dt>Cross-service links</dt><dd>{}</dd>",
        edges.iter().filter(|e| !m::yes(&e["child_link"])).count()
    ));
    html.push_str(&format!(
        "<dt>Broken / version risks</dt><dd data-zero=\"{}\">{broken_count}</dd>",
        if broken_count == 0 { "true" } else { "false" }
    ));
    html.push_str(&format!(
        "<dt>Undeclared code drift</dt><dd data-zero=\"{}\">{drift_count}</dd>",
        if drift_count == 0 { "true" } else { "false" }
    ));
    html.push_str(&format!(
        "<dt>Cross-service cycles</dt><dd data-zero=\"{}\">{}</dd>",
        if cycles.is_empty() { "true" } else { "false" },
        cycles.len()
    ));
    html.push_str("</dl>");

    let overlay = &state["overlay"];
    let unopened = m::array(&overlay["cross_service_unopened"]);
    if !unopened.is_empty() {
        html.push_str("<div class=\"system-side-callout system-side-callout-warn\"><strong>Unopened contract consumers</strong><ul class=\"system-side-list\">");
        for item in unopened.iter().take(8) {
            let label = item.as_str().map(str::to_owned).unwrap_or_else(|| {
                format!(
                    "{} (consumes {})",
                    first_non_empty(item, &["consumer_service", "service", "consumer"]),
                    first_non_empty(item, &["contract", "name"])
                )
            });
            html.push_str(&format!("<li>{}</li>", escape(&label)));
        }
        html.push_str("</ul></div>");
    }

    if !cycles.is_empty() {
        let p = payload_attr(&json!({ "surface": surface.id, "tool": "cycle", "index": 0 }));
        html.push_str(&format!(
            "<div class=\"system-side-actions\"><button type=\"button\" class=\"button compact\" data-action=\"system-map-tool\" data-payload=\"{p}\">Show cycle ↻ ({})</button></div>",
            cycles.len()
        ));
    }

    html.push_str("</div>");
    html
}

pub(super) fn render_node_details_html(
    surface: SysMapSurface,
    doc: &V,
    state: &V,
    node: &V,
) -> String {
    let id = node_id(node);
    let svc_id = {
        let p = first_non_empty(node, &["parent_service", "service"]);
        if p.is_empty() {
            id.clone()
        } else {
            p
        }
    };
    let name = m::text(node, "name");
    let kind = node["kind"].as_str().unwrap_or("service");
    let spread_list = m::array(&state["spread"]);
    let is_spread = spread_list
        .iter()
        .any(|v| v.as_str() == Some(svc_id.as_str()));

    let mut html = String::from("<div class=\"system-side-section\">");
    let clear_p = payload_attr(&json!({ "surface": surface.id }));
    html.push_str(&format!(
        "<div class=\"system-side-head\"><div><span class=\"system-side-kicker\">{}</span><h4 class=\"system-side-heading\">{}</h4></div><button type=\"button\" class=\"button compact\" data-action=\"system-map-clear-selection\" data-payload=\"{clear_p}\">Clear</button></div>",
        escape(kind),
        escape(if name.is_empty() { &id } else { &name }),
    ));

    let open_p = payload_attr(&json!({ "surface": surface.id, "service": svc_id }));
    let spread_p = payload_attr(&json!({ "surface": surface.id, "service": svc_id }));
    let changes_p = payload_attr(&json!({ "surface": surface.id, "service": svc_id }));
    let run_id_ctx = {
        let explicit = m::text(state, "run_id");
        if !explicit.is_empty() {
            explicit
        } else {
            let tgt = m::text(state, "target");
            tgt.strip_prefix("sysrun:")
                .or_else(|| tgt.strip_prefix("system-run:"))
                .unwrap_or("")
                .trim()
                .to_owned()
        }
    };
    let contract_ctx = if kind == "contract" {
        first_non_empty(node, &["contract", "name", "id"])
    } else {
        first_non_empty(node, &["contract"])
    };
    let file_ctx = first_non_empty(node, &["path", "file", "source_file"]);
    let mut omni_obj = serde_json::Map::new();
    omni_obj.insert("surface".into(), json!(surface.id));
    omni_obj.insert("service".into(), json!(svc_id));
    omni_obj.insert("id".into(), json!(id));
    if !contract_ctx.is_empty() {
        omni_obj.insert("contract".into(), json!(contract_ctx));
    }
    if !file_ctx.is_empty() {
        omni_obj.insert("file".into(), json!(file_ctx));
    }
    if !run_id_ctx.is_empty() {
        omni_obj.insert("run_id".into(), json!(run_id_ctx));
    }
    let omni_p = payload_attr(&V::Object(omni_obj));
    let pins = graph_viewport::load_pins(&sys_pin_key(surface, state));
    html.push_str("<div class=\"system-side-actions\">");
    html.push_str(&format!(
        "<button type=\"button\" class=\"button compact primary\" data-action=\"system-map-open-service\" data-payload=\"{open_p}\">Open service code map</button>"
    ));
    html.push_str(&format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"system-map-open-omnigraph\" data-payload=\"{omni_p}\">Context graph ↗</button>"
    ));
    html.push_str(&format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"system-map-spread\" data-payload=\"{spread_p}\">{}</button>",
        if is_spread {
            "Fold contracts"
        } else {
            "Unfold contracts & sources"
        }
    ));
    html.push_str(&format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"system-map-service-changes\" data-payload=\"{changes_p}\">Changes on map</button>"
    ));
    if pins.contains_key(&id) {
        let unpin_p = payload_attr(&json!({ "surface": surface.id, "id": id }));
        html.push_str(&format!(
            "<button type=\"button\" class=\"button compact\" data-action=\"system-map-unpin-node\" data-payload=\"{unpin_p}\">Unpin position</button>"
        ));
    }
    html.push_str("</div>");

    html.push_str("<dl class=\"memory-metrics system-side-metrics\">");
    html.push_str(&format!("<dt>Service ID</dt><dd>{}</dd>", escape(&svc_id)));
    let owner = m::text(node, "owner");
    if !owner.is_empty() {
        html.push_str(&format!("<dt>Owner</dt><dd>{}</dd>", escape(&owner)));
    }
    let folder = m::text(node, "folder");
    if !folder.is_empty() {
        html.push_str(&format!(
            "<dt>Folder</dt><dd title=\"{}\">{}</dd>",
            escape(&folder),
            escape(folder.rsplit('/').next().unwrap_or(&folder))
        ));
    }
    let lang = m::text(node, "language");
    if !lang.is_empty() {
        html.push_str(&format!("<dt>Stack</dt><dd>{}</dd>", escape(&lang)));
    }
    let g_state = m::text(&node["graph_status"], "state");
    if !g_state.is_empty() {
        html.push_str(&format!("<dt>Code graph</dt><dd>{}</dd>", escape(&g_state)));
    }
    let mem_count = node["memory"]["reviewed_count"]
        .as_u64()
        .or_else(|| node["memory"]["count"].as_u64());
    if let Some(mc) = mem_count {
        html.push_str(&format!(
            "<dt>Memory Bank chunks</dt><dd data-zero=\"{}\">{mc}</dd>",
            if mc == 0 { "true" } else { "false" }
        ));
    }
    html.push_str("</dl>");

    let caps = m::array(&node["capabilities"]);
    if !caps.is_empty() {
        html.push_str(
            "<h5 class=\"system-side-subheading\">Capabilities</h5><div class=\"system-chip-row\">",
        );
        for cap in caps.iter().take(12) {
            let label = capability_name(cap);
            if !label.is_empty() {
                html.push_str(&format!(
                    "<span class=\"system-chip\">{}</span>",
                    escape(&label)
                ));
            }
        }
        html.push_str("</div>");
    }

    let provides = m::array(&node["provides"]);
    if !provides.is_empty() {
        html.push_str("<h5 class=\"system-side-subheading\">Provided contracts</h5><ul class=\"system-side-list\">");
        for c in &provides {
            let cname = contract_name(c);
            let ckind = normalize_contract_arrow_kind(&m::text(c, "kind"));
            let ver = m::text(c, "version");
            let impact_p = payload_attr(&json!({
                "surface": surface.id,
                "service": svc_id,
                "contract": cname,
            }));
            html.push_str(&format!(
                "<li class=\"system-contract-row\"><span><strong>{}</strong> <span class=\"system-chip\">{ckind}</span>{}</span><button type=\"button\" class=\"button compact\" data-action=\"system-map-contract-impact\" data-payload=\"{impact_p}\">Impact</button></li>",
                escape(&cname),
                if ver.is_empty() { String::new() } else { format!(" <small>v{}</small>", escape(&ver)) },
            ));
        }
        html.push_str("</ul>");
    }

    let consumes = m::array(&node["consumes"]);
    if !consumes.is_empty() {
        html.push_str("<h5 class=\"system-side-subheading\">Consumed contracts</h5><ul class=\"system-side-list\">");
        for c in &consumes {
            let cname = contract_name(c);
            let ckind = normalize_contract_arrow_kind(&m::text(c, "kind"));
            html.push_str(&format!(
                "<li><strong>{}</strong> <span class=\"system-chip\">{ckind}</span></li>",
                escape(&cname)
            ));
        }
        html.push_str("</ul>");
    }

    // Quick analytical launchers for this service
    let follow_out = payload_attr(
        &json!({ "surface": surface.id, "tool": "follow", "service": svc_id, "dir": "out" }),
    );
    let follow_into = payload_attr(
        &json!({ "surface": surface.id, "tool": "follow", "service": svc_id, "dir": "into" }),
    );
    let path_start =
        payload_attr(&json!({ "surface": surface.id, "tool": "path", "from": svc_id }));
    let share_start =
        payload_attr(&json!({ "surface": surface.id, "tool": "shared", "a": svc_id }));

    html.push_str("<h5 class=\"system-side-subheading\">Explore links</h5><div class=\"system-side-actions\">");
    html.push_str(&format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"system-map-tool\" data-payload=\"{follow_out}\">Upstream providers</button>"
    ));
    html.push_str(&format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"system-map-tool\" data-payload=\"{follow_into}\">Downstream consumers</button>"
    ));
    html.push_str(&format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"system-map-tool\" data-payload=\"{path_start}\">Path from here…</button>"
    ));
    html.push_str(&format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"system-map-tool\" data-payload=\"{share_start}\">Compare shared…</button>"
    ));

    let cycles = find_cross_service_cycles(doc);
    if let Some(cidx) = cycles.iter().position(|comp| comp.contains(&svc_id)) {
        let cyc_p = payload_attr(&json!({ "surface": surface.id, "tool": "cycle", "index": cidx }));
        html.push_str(&format!(
            "<button type=\"button\" class=\"button compact\" data-action=\"system-map-tool\" data-payload=\"{cyc_p}\">Show cycle ↻</button>"
        ));
    }
    html.push_str("</div></div>");
    html
}

/// Where the code on both sides of the selected link is kept for a picture.
pub(super) fn bridge_key(surface: SysMapSurface) -> String {
    format!("{}-bridge", surface.id_prefix)
}
/// The code on both sides of a link between two services: the files of the
/// consumer that name what it uses, the files of the provider that make it,
/// and how far a change of each reaches inside its own service. Asked for
/// on demand: until then, a button.
pub fn bridge_html(surface: SysMapSurface, from: &str, to: &str, bridge: &V) -> String {
    let mut html = String::from("<h5 class=\"system-side-subheading\">Code on both sides</h5>");
    if bridge["from"].as_str() != Some(from) || bridge["to"].as_str() != Some(to) {
        let asked = payload_attr(&json!({"from": from, "surface": surface.id, "to": to}));
        html.push_str(&format!(
            "<p class=\"system-note\">The files of {} that name what it uses of {}, and those of {} that make it.</p><div class=\"system-side-actions\"><button type=\"button\" class=\"button compact\" data-action=\"system-map-bridge\" data-payload=\"{asked}\">Show the code</button></div>",
            escape(from),
            escape(to),
            escape(to),
        ));
        return html;
    }
    if bridge["confidence"].is_object() {
        let conf = &bridge["confidence"];
        let certain = conf["certain_refs"].as_u64().unwrap_or(0);
        let guessed = conf["guessed_refs"].as_u64().unwrap_or(0);
        let declared = conf["declared_sites"].as_u64().unwrap_or(0);
        let discovered = conf["discovered_sites"].as_u64().unwrap_or(0);
        let total_evidence = conf["total"]
            .as_u64()
            .unwrap_or(certain + guessed + declared + discovered);
        let conf_status = m::text(conf, "status");
        let ratio = conf["ratio"].as_f64();
        let ratio_label = if ratio.is_none() || total_evidence == 0 || conf_status == "unverified" {
            "Unverified (0 evidence)".to_owned()
        } else {
            let pct = (ratio.unwrap_or(0.0).clamp(0.0, 1.0) * 100.0).round() as u64;
            format!("<strong>{pct}%</strong>")
        };
        html.push_str(&format!(
            "<p class=\"system-note system-bridge-confidence\">Link confidence: {ratio_label} ({certain} certain refs, {guessed} guessed refs, {declared} declared sites, {discovered} discovered sites)</p>"
        ));
    }
    if bridge["run_coverage"].is_object() {
        let rc = &bridge["run_coverage"];
        let c_open = rc["consumer_opened"].as_u64().unwrap_or(0);
        let c_tot = rc["consumer_total"].as_u64().unwrap_or(0);
        let p_open = rc["provider_opened"].as_u64().unwrap_or(0);
        let p_tot = rc["provider_total"].as_u64().unwrap_or(0);
        let rid = m::text(rc, "run_id");
        html.push_str(&format!(
            "<p class=\"system-note system-bridge-coverage\">Run coverage{}: {c_open}/{c_tot} consumer · {p_open}/{p_tot} provider opened</p>",
            if rid.is_empty() {
                String::new()
            } else {
                format!(" ({})", escape(&rid))
            }
        ));
    }
    for (side, role, none) in [
        ("consumer", "uses it", "No file was found to name it."),
        (
            "provider",
            "makes it",
            "Its passport names no file, and none was found to name it.",
        ),
    ] {
        let part = &bridge[side];
        let service = m::text(part, "service");
        let sites = m::array(&part["sites"]);
        html.push_str(&format!(
            "<p class=\"system-note\"><strong>{}</strong> {role}{}</p>",
            escape(&service),
            if m::yes(&part["graph"]) {
                ""
            } else {
                " · no code graph yet: how far each file reaches is not told"
            },
        ));
        if sites.is_empty() {
            html.push_str(&format!("<p class=\"system-note\">{none}</p>"));
            continue;
        }
        html.push_str("<ul class=\"system-side-list system-bridge-sites\">");
        for site in &sites {
            let path = m::text(site, "path");
            let how = match (
                side == "provider" && m::yes(&site["declared"]),
                m::yes(&site["found"]),
                m::yes(&site["document"]),
            ) {
                (true, true, _) => "made here by its passport, and names it",
                (true, false, _) => "made here by its passport",
                (false, _, true) => "a document that names it",
                _ => "names it",
            };
            let reach = match site["reach"]["files"].as_u64() {
                Some(0) => " · nothing else in it uses this file".to_owned(),
                Some(files) => format!(
                    " · a change reaches {} of {}",
                    if files == 1 {
                        "1 file".to_owned()
                    } else {
                        format!("{files} files")
                    },
                    escape(&service)
                ),
                None => String::new(),
            };
            let opened_note = if m::yes(&site["opened_by_run"]) {
                " · opened in run"
            } else {
                ""
            };
            let mut raw_syms: Vec<String> = Vec::new();
            let mut first_sym_id: Option<String> = None;
            let mut first_sym_fqn: Option<String> = None;
            for item in m::array(&site["from_symbols"]) {
                let name_opt = item
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .or_else(|| {
                        let candidate =
                            first_non_empty(&item, &["fqn", "name", "symbol", "symbol_id"]);
                        (!candidate.is_empty()).then_some(candidate)
                    });
                if let Some(sym_name) = name_opt {
                    if first_sym_id.is_none() {
                        let sid = first_non_empty(&item, &["symbol_id", "id"]);
                        if !sid.is_empty() {
                            first_sym_id = Some(sid);
                        }
                    }
                    if first_sym_fqn.is_none() {
                        let fqn = first_non_empty(&item, &["fqn"]);
                        if !fqn.is_empty() {
                            first_sym_fqn = Some(fqn);
                        }
                    }
                    if !raw_syms.contains(&sym_name) {
                        raw_syms.push(sym_name);
                    }
                }
            }
            let syms_note = {
                let syms = raw_syms
                    .iter()
                    .take(4)
                    .map(|s| escape(s))
                    .collect::<Vec<_>>()
                    .join(", ");
                if syms.is_empty() {
                    String::new()
                } else {
                    format!(" · via {syms}")
                }
            };
            let mut open_obj = json!({"path": path, "service": service});
            if !raw_syms.is_empty() {
                open_obj["symbol"] = json!(raw_syms[0]);
                open_obj["from_symbols"] = json!(raw_syms);
                if let Some(sid) = first_sym_id {
                    open_obj["symbol_id"] = json!(sid);
                }
                if let Some(fqn) = first_sym_fqn {
                    open_obj["symbol_fqn"] = json!(fqn);
                }
            }
            let open = payload_attr(&open_obj);
            html.push_str(&format!(
                "<li class=\"system-bridge-site\" data-opened-by-run=\"{}\"><div><code>{}</code><div class=\"system-note\">{how}{}{reach}{syms_note}{opened_note}</div></div><button type=\"button\" class=\"button compact\" data-action=\"inspect-system-file-graph\" data-payload=\"{open}\" aria-label=\"Show {} on the code map of {}\">Map</button></li>",
                if m::yes(&site["opened_by_run"]) { "true" } else { "false" },
                escape(&path),
                {
                    let named = m::array(&site["contracts"])
                        .iter()
                        .filter_map(|c| c.as_str().map(escape))
                        .collect::<Vec<_>>()
                        .join(", ");
                    if named.is_empty() {
                        String::new()
                    } else {
                        format!(": {named}")
                    }
                },
                escape(&path),
                escape(&service),
            ));
        }
        html.push_str("</ul>");
        let more = part["more"].as_u64().unwrap_or(0);
        if more > 0 {
            html.push_str(&format!("<p class=\"system-note\">and {more} more</p>"));
        }
        if m::yes(&part["cut"]) {
            html.push_str(
                "<p class=\"system-note\">Not every file of it was read: there may be more.</p>",
            );
        }
    }
    html.push_str(&semantic_bridges_html(
        from,
        to,
        &bridge["semantic_bridges"],
    ));
    html
}

pub(super) fn join_text_array(value: &V) -> String {
    m::array(value)
        .iter()
        .filter_map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn semantic_bridge_file_buttons(service: &str, files: &V) -> String {
    let mut out = String::new();
    for item in m::array(files).into_iter().take(6) {
        let Some(path) = item.as_str().map(str::trim).filter(|p| !p.is_empty()) else {
            continue;
        };
        let open = payload_attr(&json!({"path": path, "service": service}));
        out.push_str(&format!(
            "<button type=\"button\" class=\"system-semantic-file\" data-action=\"inspect-system-file-graph\" data-payload=\"{open}\" title=\"Show {} on the code map of {}\"><code>{}</code></button>",
            escape(path),
            escape(service),
            escape(path),
        ));
    }
    out
}

pub(super) fn wrap_semantic_files(buttons: String) -> String {
    if buttons.is_empty() {
        String::new()
    } else {
        format!("<div class=\"system-semantic-files\">{buttons}</div>")
    }
}

pub(super) fn semantic_bridges_html(from: &str, to: &str, sem: &V) -> String {
    if !sem.is_object() {
        return String::new();
    }
    let routes = m::array(&sem["routes"]);
    let events = m::array(&sem["events"]);
    let di_bindings = m::array(&sem["di_bindings"]);
    let tables = m::array(&sem["tables"]);
    let total = routes.len() + events.len() + di_bindings.len() + tables.len();
    if total == 0 {
        return "<details class=\"system-semantic-bridges\"><summary class=\"system-semantic-bridges-summary\"><span>Semantic framework &amp; DB bridges</span><span class=\"system-chip\">0</span></summary><p class=\"system-note\">No shared routes, events, DI bindings, or tables detected.</p></details>".to_owned();
    }
    let mut body = String::new();
    if !routes.is_empty() {
        body.push_str(&format!(
            "<h6 class=\"system-semantic-group-title\">Routes <span class=\"system-chip\">{}</span></h6><ul class=\"system-semantic-list\">",
            routes.len()
        ));
        for r in &routes {
            let route = m::text(r, "route");
            let methods = {
                let joined = join_text_array(&r["methods"]);
                if !joined.is_empty() {
                    joined
                } else {
                    let single = m::text(r, "method");
                    if single.is_empty() {
                        "ANY".to_owned()
                    } else {
                        single
                    }
                }
            };
            let caller = {
                let c = m::text(r, "caller");
                if c.is_empty() {
                    from.to_owned()
                } else {
                    c
                }
            };
            let provider = {
                let p = m::text(r, "provider");
                if p.is_empty() {
                    to.to_owned()
                } else {
                    p
                }
            };
            let files = format!(
                "{}{}",
                semantic_bridge_file_buttons(&caller, &r["caller_files"]),
                semantic_bridge_file_buttons(&provider, &r["provider_files"]),
            );
            body.push_str(&format!(
                "<li class=\"system-semantic-item\" data-kind=\"route\"><div class=\"system-semantic-item-head\"><span class=\"system-semantic-badge\" data-kind=\"route\">{}</span><code>{}</code></div><div class=\"system-note\">{} → {}</div>{}</li>",
                escape(&methods),
                escape(&route),
                escape(&caller),
                escape(&provider),
                wrap_semantic_files(files),
            ));
        }
        body.push_str("</ul>");
    }
    if !events.is_empty() {
        body.push_str(&format!(
            "<h6 class=\"system-semantic-group-title\">Events <span class=\"system-chip\">{}</span></h6><ul class=\"system-semantic-list\">",
            events.len()
        ));
        for ev in &events {
            let event_name = m::text(ev, "event");
            let producer = {
                let p = m::text(ev, "producer");
                if p.is_empty() {
                    from.to_owned()
                } else {
                    p
                }
            };
            let consumer = {
                let c = m::text(ev, "consumer");
                if c.is_empty() {
                    to.to_owned()
                } else {
                    c
                }
            };
            let files = format!(
                "{}{}",
                semantic_bridge_file_buttons(&producer, &ev["producer_files"]),
                semantic_bridge_file_buttons(&consumer, &ev["consumer_files"]),
            );
            body.push_str(&format!(
                "<li class=\"system-semantic-item\" data-kind=\"event\"><div class=\"system-semantic-item-head\"><span class=\"system-semantic-badge\" data-kind=\"event\">event</span><code>{}</code></div><div class=\"system-note\">{} → {}</div>{}</li>",
                escape(&event_name),
                escape(&producer),
                escape(&consumer),
                wrap_semantic_files(files),
            ));
        }
        body.push_str("</ul>");
    }
    if !di_bindings.is_empty() {
        body.push_str(&format!(
            "<h6 class=\"system-semantic-group-title\">DI bindings <span class=\"system-chip\">{}</span></h6><ul class=\"system-semantic-list\">",
            di_bindings.len()
        ));
        for di in &di_bindings {
            let service = m::text(di, "service");
            let from_kinds = join_text_array(&di["from_kinds"]);
            let to_kinds = join_text_array(&di["to_kinds"]);
            let left_note = if from_kinds.is_empty() {
                escape(from)
            } else {
                format!("{}: {}", escape(from), escape(&from_kinds))
            };
            let right_note = if to_kinds.is_empty() {
                escape(to)
            } else {
                format!("{}: {}", escape(to), escape(&to_kinds))
            };
            let files = format!(
                "{}{}",
                semantic_bridge_file_buttons(from, &di["from_files"]),
                semantic_bridge_file_buttons(to, &di["to_files"]),
            );
            body.push_str(&format!(
                "<li class=\"system-semantic-item\" data-kind=\"di\"><div class=\"system-semantic-item-head\"><span class=\"system-semantic-badge\" data-kind=\"di\">DI</span><code>{}</code></div><div class=\"system-note\">{left_note} ↔ {right_note}</div>{}</li>",
                escape(&service),
                wrap_semantic_files(files),
            ));
        }
        body.push_str("</ul>");
    }
    if !tables.is_empty() {
        body.push_str(&format!(
            "<h6 class=\"system-semantic-group-title\">Shared DB tables <span class=\"system-chip\">{}</span></h6><ul class=\"system-semantic-list\">",
            tables.len()
        ));
        for tbl in &tables {
            let table_name = m::text(tbl, "table");
            let coupling_raw = m::text(tbl, "coupling");
            let (coupling_attr, coupling_label) = match coupling_raw.as_str() {
                "shared_write" => ("shared_write", "shared write"),
                "cross_read_write" => ("cross_read_write", "read / write"),
                "shared_read" => ("shared_read", "shared read"),
                _ => ("shared_read", "shared"),
            };
            let from_access = join_text_array(&tbl["from_access"]);
            let to_access = join_text_array(&tbl["to_access"]);
            let left_note = if from_access.is_empty() {
                escape(from)
            } else {
                format!("{} ({})", escape(from), escape(&from_access))
            };
            let right_note = if to_access.is_empty() {
                escape(to)
            } else {
                format!("{} ({})", escape(to), escape(&to_access))
            };
            let files = format!(
                "{}{}",
                semantic_bridge_file_buttons(from, &tbl["from_files"]),
                semantic_bridge_file_buttons(to, &tbl["to_files"]),
            );
            body.push_str(&format!(
                "<li class=\"system-semantic-item\" data-kind=\"table\"><div class=\"system-semantic-item-head\"><span class=\"system-semantic-badge\" data-kind=\"table\" data-coupling=\"{coupling_attr}\">{coupling_label}</span><code>{}</code></div><div class=\"system-note\">{left_note} · {right_note}</div>{}</li>",
                escape(&table_name),
                wrap_semantic_files(files),
            ));
        }
        body.push_str("</ul>");
    }
    format!(
        "<details class=\"system-semantic-bridges\" open><summary class=\"system-semantic-bridges-summary\"><span>Semantic framework &amp; DB bridges</span><span class=\"system-chip\">{total}</span></summary><div class=\"system-semantic-bridges-body\">{body}</div></details>"
    )
}
/// Asks for the code on both sides of a link and draws it beside the map.
pub(super) async fn fetch_bridge(surface: SysMapSurface, from: String, to: String) {
    let state = surface_state(surface);
    let project_id = m::text(&state, "project_id");
    if project_id.is_empty() {
        return;
    }
    let run_id = m::text(&state["overlay"], "run_id");
    let target = if !run_id.is_empty() {
        format!("sysrun:{run_id}")
    } else {
        String::new()
    };
    let q = dom::query(&[
        ("project_id", project_id),
        ("config_path", m::text(&state, "config_path")),
        ("from", from),
        ("to", to),
        ("target", target),
        ("run_id", run_id),
        (
            "certain",
            if m::yes(&state["certain"]) {
                "1".to_owned()
            } else {
                String::new()
            },
        ),
    ]);
    match api::get(&format!("/api/systems/graph/bridge{q}")).await {
        Ok(doc) => {
            dom::error(surface.error_id, "");
            m::set(&bridge_key(surface), doc);
            render_surface(surface, false);
        }
        Err(err) if !err.aborted => {
            dom::error(
                surface.error_id,
                &format!("Could not load link bridge: {}", err.message),
            );
        }
        Err(_) => {}
    }
}
pub(super) fn render_edge_details_html(surface: SysMapSurface, doc: &V, edge: &V) -> String {
    let from = m::text(edge, "from");
    let to = m::text(edge, "to");
    let nodes = m::array(&doc["nodes"]);
    let node_by_id: BTreeMap<String, &V> = nodes.iter().map(|n| (node_id(n), n)).collect();
    let from_svc = endpoint_service_id(&from, Some(&node_by_id)).unwrap_or_else(|| from.clone());
    let to_svc = endpoint_service_id(&to, Some(&node_by_id)).unwrap_or_else(|| to.clone());
    let kind = primary_edge_kind(edge);
    let prov = m::text(edge, "provenance");
    let broken = m::yes(&edge["broken_risk"]);
    let against = m::yes(&edge["against"]);
    let clear_p = payload_attr(&json!({ "surface": surface.id }));

    let mut html = String::from("<div class=\"system-side-section\">");
    html.push_str(&format!(
        "<div class=\"system-side-head\"><div><span class=\"system-side-kicker\">Contract link · {kind}</span><h4 class=\"system-side-heading\">{} → {}</h4></div><button type=\"button\" class=\"button compact\" data-action=\"system-map-clear-selection\" data-payload=\"{clear_p}\">Clear</button></div>",
        escape(&from),
        escape(&to),
    ));

    if broken {
        html.push_str("<p class=\"system-side-callout system-side-callout-warn\"><strong>Broken contract risk:</strong> Consumer contract has no matching provider definition or has an incompatible version.</p>");
    }
    if prov == "undeclared_in_code" {
        html.push_str("<p class=\"system-side-callout system-side-callout-warn\"><strong>Undeclared drift:</strong> Code references cross these service folders without a declared contract in the system manifest.</p>");
    }
    if against {
        html.push_str("<p class=\"system-side-callout system-side-callout-warn\"><strong>Layer violation:</strong> This link runs upward against architectural tiers or closes a cross-service cycle.</p>");
    }

    html.push_str("<dl class=\"memory-metrics system-side-metrics\">");
    html.push_str(&format!("<dt>Consumer</dt><dd>{}</dd>", escape(&from)));
    html.push_str(&format!("<dt>Provider</dt><dd>{}</dd>", escape(&to)));
    html.push_str(&format!("<dt>Primary transport</dt><dd>{kind}</dd>"));
    html.push_str(&format!(
        "<dt>Provenance</dt><dd>{}</dd>",
        escape(if prov.is_empty() { "declared" } else { &prov })
    ));
    html.push_str("</dl>");

    let contracts = m::array(&edge["contracts"]);
    if !contracts.is_empty() {
        html.push_str("<h5 class=\"system-side-subheading\">Contracts on this link</h5><ul class=\"system-side-list\">");
        for c in &contracts {
            let cname = contract_name(c);
            let ckind = normalize_contract_arrow_kind(&m::text(c, "kind"));
            let ver = {
                let direct = m::text(c, "version");
                if !direct.is_empty() {
                    direct
                } else {
                    let pv = m::text(c, "provider_version");
                    let cv = m::text(c, "consumer_version");
                    if !cv.is_empty() && !pv.is_empty() && cv != pv {
                        format!("{cv} → {pv}")
                    } else if !pv.is_empty() {
                        pv
                    } else {
                        cv
                    }
                }
            };
            let src = {
                let direct = first_non_empty(c, &["source", "schema", "provider_source"]);
                if !direct.is_empty() {
                    direct
                } else {
                    m::array(&c["provider_sources"])
                        .into_iter()
                        .chain(m::array(&c["consumer_sources"]))
                        .find_map(|v| {
                            v.as_str()
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .map(str::to_owned)
                        })
                        .unwrap_or_default()
                }
            };
            let impact_p = payload_attr(&json!({
                "surface": surface.id,
                "service": to_svc,
                "contract": cname,
            }));
            html.push_str(&format!(
                "<li class=\"system-contract-row\"><div><strong>{}</strong> <span class=\"system-chip\">{ckind}</span>{}{}</div><button type=\"button\" class=\"button compact\" data-action=\"system-map-contract-impact\" data-payload=\"{impact_p}\">Impact</button></li>",
                escape(&cname),
                if ver.is_empty() { String::new() } else { format!(" <small>v{}</small>", escape(&ver)) },
                if src.is_empty() { String::new() } else { format!("<div class=\"system-note\">{}</div>", escape(&src)) },
            ));
        }
        html.push_str("</ul>");
    }

    html.push_str(&bridge_html(
        surface,
        &from_svc,
        &to_svc,
        &m::data(&bridge_key(surface)),
    ));

    let open_from = payload_attr(&json!({ "surface": surface.id, "service": from_svc }));
    let open_to = payload_attr(&json!({ "surface": surface.id, "service": to_svc }));
    html.push_str("<div class=\"system-side-actions\">");
    html.push_str(&format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"system-map-open-service\" data-payload=\"{open_from}\">Open {} map</button>",
        escape(&from_svc)
    ));
    html.push_str(&format!(
        "<button type=\"button\" class=\"button compact\" data-action=\"system-map-open-service\" data-payload=\"{open_to}\">Open {} map</button>",
        escape(&to_svc)
    ));
    html.push_str("</div></div>");
    html
}

pub(super) fn render_analyze_tab_html(
    surface: SysMapSurface,
    doc: &V,
    state: &V,
    services: &[V],
) -> String {
    let tool = {
        let t = m::text(state, "tool");
        if t.is_empty() {
            "path".to_owned()
        } else {
            t
        }
    };

    let mut html = String::from("<div class=\"system-side-section\">");
    html.push_str(
        "<div class=\"system-side-actions\" role=\"group\" aria-label=\"Topology analysis tool\">",
    );
    for (id, label) in [
        ("follow", "Follow links"),
        ("path", "Path"),
        ("shared", "What both share"),
        ("cycle", "Cycles ↻"),
        ("layers", "Layers"),
    ] {
        let p = payload_attr(&json!({ "surface": surface.id, "tool": id }));
        html.push_str(&format!(
            "<button type=\"button\" class=\"button compact\" aria-pressed=\"{}\" data-action=\"system-map-tool\" data-payload=\"{p}\">{}</button>",
            if tool == id { "true" } else { "false" },
            escape(label),
        ));
    }
    html.push_str("</div>");

    let service_options = |selected: &str| -> String {
        let mut opts = String::from("<option value=\"\">Select service…</option>");
        for s in services {
            let sid = node_id(s);
            let sname = m::text(s, "name");
            opts.push_str(&format!(
                "<option value=\"{}\"{}>{}</option>",
                escape(&sid),
                if sid == selected { " selected" } else { "" },
                escape(if sname.is_empty() { &sid } else { &sname }),
            ));
        }
        opts
    };

    match tool.as_str() {
        "follow" => {
            let root = m::text(state, "follow");
            let dir = m::text(state, "follow_dir");
            html.push_str(&format!(
                "<label class=\"field\">Starting service<select data-action=\"system-map-follow-root\" data-surface=\"{}\">{}</select></label>",
                surface.id,
                service_options(&root)
            ));
            let p_out = payload_attr(
                &json!({ "surface": surface.id, "tool": "follow", "service": root, "dir": "out" }),
            );
            let p_into = payload_attr(
                &json!({ "surface": surface.id, "tool": "follow", "service": root, "dir": "into" }),
            );
            html.push_str(&format!(
                "<div class=\"system-side-actions\"><button type=\"button\" class=\"button compact\" aria-pressed=\"{}\" data-action=\"system-map-tool\" data-payload=\"{p_out}\">Upstream providers</button><button type=\"button\" class=\"button compact\" aria-pressed=\"{}\" data-action=\"system-map-tool\" data-payload=\"{p_into}\">Downstream consumers</button></div>",
                if dir != "into" { "true" } else { "false" },
                if dir == "into" { "true" } else { "false" },
            ));
            if !root.is_empty() {
                let key = if dir == "into" { "into" } else { "out" };
                let expanded: BTreeSet<String> = m::array(&state[key])
                    .into_iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .chain(std::iter::once(root.clone()))
                    .collect();
                html.push_str("<ul class=\"system-side-list\">");
                for e in m::array(&doc["edges"])
                    .iter()
                    .filter(|e| !m::yes(&e["child_link"]))
                {
                    let f = m::text(e, "from");
                    let t = m::text(e, "to");
                    let next = if dir == "into" && expanded.contains(&t) {
                        Some((&t, &f))
                    } else if dir != "into" && expanded.contains(&f) {
                        Some((&f, &t))
                    } else {
                        None
                    };
                    if let Some((src, dst)) = next {
                        let step_p = payload_attr(&json!({
                            "surface": surface.id,
                            "dir": dir,
                            "hop": dst,
                        }));
                        html.push_str(&format!(
                            "<li class=\"system-contract-row\"><span>{} → <strong>{}</strong></span><button type=\"button\" class=\"button compact\" data-action=\"system-map-follow-hop\" data-payload=\"{step_p}\">{}</button></li>",
                            escape(src),
                            escape(dst),
                            if expanded.contains(dst) { "Fold" } else { "Step further" },
                        ));
                    }
                }
                html.push_str("</ul>");
            }
        }
        "shared" => {
            let sa = m::text(state, "shared_a");
            let sb = m::text(state, "shared_b");
            let side = m::text(state, "shared_side");
            html.push_str(&format!(
                "<label class=\"field\">First service<select data-action=\"system-map-shared-a\" data-surface=\"{}\">{}</select></label><label class=\"field\">Second service<select data-action=\"system-map-shared-b\" data-surface=\"{}\">{}</select></label>",
                surface.id,
                service_options(&sa),
                surface.id,
                service_options(&sb),
            ));
            let p_bu = payload_attr(&json!({ "surface": surface.id, "side": "both_use" }));
            let p_ub = payload_attr(&json!({ "surface": surface.id, "side": "uses_both" }));
            html.push_str(&format!(
                "<div class=\"system-side-actions\"><button type=\"button\" class=\"button compact\" aria-pressed=\"{}\" data-action=\"system-map-shared-side\" data-payload=\"{p_bu}\">What both depend on</button><button type=\"button\" class=\"button compact\" aria-pressed=\"{}\" data-action=\"system-map-shared-side\" data-payload=\"{p_ub}\">What depends on both</button></div>",
                if side != "uses_both" { "true" } else { "false" },
                if side == "uses_both" { "true" } else { "false" },
            ));
            if !sa.is_empty() && !sb.is_empty() {
                let (both_use, uses_both) = shared_neighbors(doc, &sa, &sb);
                let items = if side == "uses_both" {
                    uses_both
                } else {
                    both_use
                };
                if items.is_empty() {
                    html.push_str(
                        "<p class=\"system-note\">No shared services in this direction.</p>",
                    );
                } else {
                    html.push_str("<ul class=\"system-side-list\">");
                    for item in items {
                        let p = payload_attr(&json!({ "surface": surface.id, "path": item }));
                        html.push_str(&format!(
                            "<li><button type=\"button\" class=\"button compact\" data-action=\"system-map-node\" data-payload=\"{p}\">{}</button></li>",
                            escape(&item)
                        ));
                    }
                    html.push_str("</ul>");
                }
            }
        }
        "cycle" => {
            let cycles = find_cross_service_cycles(doc);
            if cycles.is_empty() {
                html.push_str(
                    "<p class=\"system-note\">No cross-service dependency cycles detected.</p>",
                );
            } else {
                let active_idx = state["cycle_index"].as_u64().unwrap_or(0) as usize;
                html.push_str("<ul class=\"system-side-list\">");
                for (idx, comp) in cycles.iter().enumerate() {
                    let p = payload_attr(
                        &json!({ "surface": surface.id, "tool": "cycle", "index": idx }),
                    );
                    html.push_str(&format!(
                        "<li class=\"system-contract-row\"><span>↻ {}</span><button type=\"button\" class=\"button compact\" aria-pressed=\"{}\" data-action=\"system-map-tool\" data-payload=\"{p}\">Highlight</button></li>",
                        escape(&comp.join(" ↔ ")),
                        if idx == active_idx { "true" } else { "false" },
                    ));
                }
                html.push_str("</ul>");
            }
        }
        "layers" => {
            let (tiers, violations) = assign_service_layers(doc);
            let toggle_p = payload_attr(&json!({ "surface": surface.id }));
            html.push_str(&format!(
                "<div class=\"system-side-actions\"><button type=\"button\" class=\"button compact\" aria-pressed=\"{}\" data-action=\"system-map-toggle-layers\" data-payload=\"{toggle_p}\">Arrange canvas by tiers</button></div>",
                if m::yes(&state["layers"]) { "true" } else { "false" }
            ));
            for (tier_idx, label) in [
                (0usize, "1. Edge & Gateways"),
                (1, "2. Application Services"),
                (2, "3. Core & Platform"),
                (3, "4. Shared Authority"),
            ] {
                let members: Vec<String> = tiers
                    .iter()
                    .filter(|(_, &t)| t == tier_idx)
                    .map(|(k, _)| k.clone())
                    .collect();
                if !members.is_empty() {
                    html.push_str(&format!(
                        "<h5 class=\"system-side-subheading\">{}</h5><div class=\"system-chip-row\">{}</div>",
                        escape(label),
                        members
                            .iter()
                            .map(|m_id| format!("<span class=\"system-chip\">{}</span>", escape(m_id)))
                            .collect::<Vec<_>>()
                            .join("")
                    ));
                }
            }
            if !violations.is_empty() {
                html.push_str("<h5 class=\"system-side-subheading\">Upward tier violations</h5><ul class=\"system-side-list\">");
                for (f, t) in violations {
                    html.push_str(&format!(
                        "<li><strong>{}</strong> → <strong>{}</strong></li>",
                        escape(&f),
                        escape(&t)
                    ));
                }
                html.push_str("</ul>");
            }
        }
        _ => {
            // Path between services + "The other way"
            let p_from = m::text(state, "path_from");
            let p_to = m::text(state, "path_to");
            let skip: BTreeSet<String> = m::array(&state["skip"])
                .into_iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            let swap_p = payload_attr(&json!({ "surface": surface.id }));
            html.push_str(&format!(
                "<label class=\"field\">From consumer<select data-action=\"system-map-path-from\" data-surface=\"{}\">{}</select></label><label class=\"field\">To provider<select data-action=\"system-map-path-to\" data-surface=\"{}\">{}</select></label><div class=\"system-side-actions\"><button type=\"button\" class=\"button compact\" data-action=\"system-map-path-swap\" data-payload=\"{swap_p}\">The other way ⇄</button></div>",
                surface.id,
                service_options(&p_from),
                surface.id,
                service_options(&p_to),
            ));
            if !p_from.is_empty() && !p_to.is_empty() {
                let paths = shortest_service_paths(doc, &p_from, &p_to, &skip);
                if paths.is_empty() {
                    html.push_str("<p class=\"system-note\">No directed contract path connects those services in this direction.</p>");
                } else {
                    html.push_str("<ul class=\"system-side-list\">");
                    for path in &paths {
                        html.push_str(&format!("<li>{}</li>", escape(&path.join(" → "))));
                    }
                    html.push_str("</ul>");
                    let intermediates: BTreeSet<String> = paths
                        .iter()
                        .flat_map(|p| p.iter().skip(1).take(p.len().saturating_sub(2)))
                        .cloned()
                        .collect();
                    if !intermediates.is_empty() || !skip.is_empty() {
                        html.push_str("<h5 class=\"system-side-subheading\">Skip intermediate service</h5><div class=\"system-chip-row\">");
                        for mid in intermediates.union(&skip) {
                            let sp =
                                payload_attr(&json!({ "surface": surface.id, "service": mid }));
                            html.push_str(&format!(
                                "<button type=\"button\" class=\"button compact\" aria-pressed=\"{}\" data-action=\"system-map-path-skip\" data-payload=\"{sp}\">{}</button>",
                                if skip.contains(mid) { "true" } else { "false" },
                                escape(mid),
                            ));
                        }
                        html.push_str("</div>");
                    }
                }
            }
        }
    }

    html.push_str("</div>");
    html
}

pub(super) fn render_changes_tab_html(surface: SysMapSurface, _doc: &V, state: &V) -> String {
    let changes = m::data(surface.changes_key);
    let impact = m::data(surface.impact_key);
    let mut html = String::from("<div class=\"system-side-section\">");
    let refresh_p = payload_attr(&json!({ "surface": surface.id }));
    html.push_str(&format!(
        "<div class=\"system-side-head\"><h4 class=\"system-side-heading\">Changes &amp; cross-service impact</h4><button type=\"button\" class=\"button compact\" data-action=\"system-map-refresh-changes\" data-payload=\"{refresh_p}\">Refresh</button></div>"
    ));

    if impact.is_object() {
        let impact_err = m::text(&impact, "error");
        if !impact_err.is_empty() {
            html.push_str(&format!(
                "<p class=\"system-error\" role=\"alert\">Contract impact query failed: {}</p>",
                escape(&impact_err)
            ));
        }
        let mut subject = first_non_empty(&impact, &["contract", "path"]);
        if subject.is_empty() {
            if let Some(first_ic) = m::array(&impact["impacted_contracts"]).into_iter().next() {
                subject = first_non_empty(&first_ic, &["contract", "id", "name"]);
            }
        }
        if subject.is_empty() {
            subject = m::text(&impact, "service");
        }
        if !subject.is_empty() {
            html.push_str(&format!(
                "<div class=\"system-side-callout\"><strong>Impact for {}</strong>",
                escape(&subject)
            ));
            let consumers = if impact["contract_consumers"].is_array() {
                m::array(&impact["contract_consumers"])
            } else if impact["consumers"].is_array() {
                m::array(&impact["consumers"])
            } else {
                m::array(&impact["downstream_services"])
            };
            if !consumers.is_empty() {
                html.push_str("<ul class=\"system-side-list\">");
                for c in consumers.iter().take(12) {
                    let label = c.as_str().map(str::to_owned).unwrap_or_else(|| {
                        let svc =
                            first_non_empty(c, &["consumer", "consumer_service", "service", "id"]);
                        let contract = first_non_empty(c, &["contract", "name"]);
                        let mut src = m::text(c, "path");
                        if src.is_empty() {
                            src = m::array(&c["consumer_sources"])
                                .into_iter()
                                .chain(m::array(&c["unopened_sources"]))
                                .find_map(|v| {
                                    v.as_str()
                                        .map(str::trim)
                                        .filter(|s| !s.is_empty())
                                        .map(str::to_owned)
                                })
                                .unwrap_or_default();
                        }
                        let mut parts = Vec::new();
                        if !svc.is_empty() {
                            parts.push(svc);
                        }
                        if !contract.is_empty() {
                            parts.push(format!("({contract})"));
                        }
                        if !src.is_empty() {
                            parts.push(src);
                        }
                        parts.join(" ")
                    });
                    html.push_str(&format!("<li>{}</li>", escape(label.trim())));
                }
                html.push_str("</ul>");
            }
            let unopened = m::array(&impact["cross_service_unopened"]);
            if !unopened.is_empty() {
                html.push_str(&format!(
                    "<p class=\"system-note\">Unopened downstream consumers: {}</p>",
                    unopened.len()
                ));
            }
            html.push_str("</div>");
        }
    }

    let changes_err = m::text(&changes, "error");
    if !changes_err.is_empty() {
        html.push_str(&format!(
            "<p class=\"system-error\" role=\"alert\">Failed to load run changes: {}</p>",
            escape(&changes_err)
        ));
    }

    let by_service = if changes["services"].is_array() {
        m::array(&changes["services"])
    } else if changes["groups"].is_array() {
        m::array(&changes["groups"])
    } else {
        Vec::new()
    };

    if !by_service.is_empty() {
        for group in &by_service {
            let sid = first_non_empty(group, &["service", "id", "name"]);
            let files = if group["changed_files"].is_array() {
                m::array(&group["changed_files"])
            } else {
                m::array(&group["files"])
            };
            html.push_str(&format!(
                "<h5 class=\"system-side-subheading\">{} ({} files)</h5><ul class=\"system-side-list\">",
                escape(&sid),
                files.len()
            ));
            for f in files.iter().take(20) {
                let fpath = f
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| m::text(f, "path"));
                if fpath.is_empty() {
                    continue;
                }
                let file_p = payload_attr(&json!({
                    "service": sid,
                    "path": fpath,
                }));
                html.push_str(&format!(
                    "<li class=\"system-contract-row\"><span class=\"system-file-path\">{}</span><button type=\"button\" class=\"button compact\" data-action=\"inspect-system-file-graph\" data-service=\"{}\" data-path=\"{}\" data-payload=\"{file_p}\">Map</button></li>",
                    escape(&fpath),
                    escape(&sid),
                    escape(&fpath),
                ));
            }
            html.push_str("</ul>");
        }
    } else if changes["files"].is_array() {
        let files = m::array(&changes["files"]);
        if files.is_empty() {
            html.push_str("<p class=\"system-note\">No uncommitted or run-scoped changes reported across member services.</p>");
        } else {
            html.push_str("<ul class=\"system-side-list\">");
            for f in files.iter().take(30) {
                let sid = first_non_empty(f, &["service", "project"]);
                let fpath = m::text(f, "path");
                let file_p = payload_attr(&json!({ "service": sid, "path": fpath }));
                html.push_str(&format!(
                    "<li class=\"system-contract-row\"><span><strong>{}</strong> · {}</span><button type=\"button\" class=\"button compact\" data-action=\"inspect-system-file-graph\" data-service=\"{}\" data-path=\"{}\" data-payload=\"{file_p}\">Map</button></li>",
                    escape(&sid),
                    escape(&fpath),
                    escape(&sid),
                    escape(&fpath),
                ));
            }
            html.push_str("</ul>");
        }
    } else if !changes_err.is_empty() {
        // Error already displayed above.
    } else if changes["services"].is_array() || changes["groups"].is_array() {
        html.push_str("<p class=\"system-note\">No uncommitted or run-scoped changes reported across member services.</p>");
    } else {
        let overlay = &state["overlay"];
        if m::text(overlay, "run_id").is_empty() {
            html.push_str("<p class=\"system-note\">Working tree changes across member services appear here once queried.</p>");
        } else {
            html.push_str(
                "<p class=\"system-note\">Loading run changes across member services…</p>",
            );
        }
    }

    html.push_str("</div>");
    html
}

pub(super) async fn fetch_surface_map(
    surface: SysMapSurface,
    catalog: V,
    graph_status: V,
    refit: bool,
) {
    let state = surface_state(surface);
    let project_id = m::text(&state, "project_id");
    let config_path = m::text(&state, "config_path");
    if project_id.is_empty() {
        return;
    }

    let epoch = m::next(surface.fetch_epoch);
    let overlay = &state["overlay"];
    let run_id = m::text(overlay, "run_id");
    let target = if !run_id.is_empty() {
        format!("sysrun:{run_id}")
    } else {
        String::new()
    };
    let seeds = m::array(&overlay["seed_services"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect::<Vec<_>>()
        .join(",");
    let spread = m::array(&state["spread"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect::<Vec<_>>()
        .join(",");

    let query = dom::query(&[
        ("project_id", project_id),
        ("config_path", config_path),
        ("target", target),
        ("run_id", run_id),
        ("seed_services", seeds),
        (
            "depth",
            overlay["depth"]
                .as_u64()
                .map(|d| d.clamp(1, 16).to_string())
                .unwrap_or_default(),
        ),
        ("focus", m::text(&state, "focus")),
        ("spread", spread),
        (
            "whole",
            if m::yes(&state["whole"]) {
                "1".to_owned()
            } else {
                String::new()
            },
        ),
        (
            "certain",
            if m::yes(&state["certain"]) {
                "1".to_owned()
            } else {
                String::new()
            },
        ),
        (
            "layers",
            if m::yes(&state["layers"]) {
                "1".to_owned()
            } else {
                String::new()
            },
        ),
    ]);

    let path = format!("/api/systems/graph/map{query}");
    match api::get(&path).await {
        Ok(raw) => {
            if !m::current(surface.fetch_epoch, epoch) {
                return;
            }
            dom::error(surface.error_id, "");
            let cur_state = surface_state(surface);
            let normalized = normalize_map_doc(&raw, &catalog, &graph_status, &cur_state);
            m::set(surface.map_key, normalized);
            render_surface(surface, refit);
        }
        Err(err) => {
            if !m::current(surface.fetch_epoch, epoch) {
                return;
            }
            if !err.aborted {
                dom::error(
                    surface.error_id,
                    &format!(
                        "Showing local catalog synthesis — system map request failed: {}",
                        err.message
                    ),
                );
            }
            // Keep synthesized catalog map active if backend graph route is unavailable.
            let cur_state = surface_state(surface);
            let fallback = expand_unfolded_nodes(
                &synthesize_map_from_catalog(&catalog, &graph_status),
                &cur_state,
            );
            m::set(surface.map_key, fallback);
            render_surface(surface, refit);
        }
    }
}

pub(super) async fn fetch_surface_changes(surface: SysMapSurface) {
    let state = surface_state(surface);
    let project_id = m::text(&state, "project_id");
    let config_path = m::text(&state, "config_path");
    if project_id.is_empty() {
        return;
    }
    let epoch = m::next(surface.changes_epoch);
    let run_id = m::text(&state["overlay"], "run_id");
    let target = if !run_id.is_empty() {
        format!("sysrun:{run_id}")
    } else {
        "work".to_owned()
    };
    let q = dom::query(&[
        ("project_id", project_id),
        ("config_path", config_path),
        ("target", target),
        ("run_id", run_id),
        ("service", m::text(&state, "focus")),
    ]);
    match api::get(&format!("/api/systems/graph/changes{q}")).await {
        Ok(doc) => {
            if m::current(surface.changes_epoch, epoch) {
                dom::error(surface.error_id, "");
                m::set(surface.changes_key, doc);
                render_surface(surface, false);
            }
        }
        Err(err) => {
            if m::current(surface.changes_epoch, epoch) && !err.aborted {
                dom::error(
                    surface.error_id,
                    &format!("Changes query failed: {}", err.message),
                );
                m::set(surface.changes_key, json!({ "error": err.message }));
                render_surface(surface, false);
            }
        }
    }
}

pub(super) async fn fetch_contract_impact(
    surface: SysMapSurface,
    service: String,
    contract: String,
) {
    let state = surface_state(surface);
    let project_id = m::text(&state, "project_id");
    let config_path = m::text(&state, "config_path");
    if project_id.is_empty() {
        return;
    }
    let epoch = m::next(surface.impact_epoch);
    let run_id = m::text(&state["overlay"], "run_id");
    let q = dom::query(&[
        ("project_id", project_id),
        ("config_path", config_path),
        ("service", service),
        ("contract", contract),
        ("run_id", run_id),
    ]);
    match api::get(&format!("/api/systems/graph/impact{q}")).await {
        Ok(doc) => {
            if m::current(surface.impact_epoch, epoch) {
                dom::error(surface.error_id, "");
                m::set(surface.impact_key, doc);
                let mut next = surface_state(surface);
                next["side_tab"] = json!("changes");
                save_surface_state(surface, &next);
                render_surface(surface, false);
            }
        }
        Err(err) => {
            if m::current(surface.impact_epoch, epoch) && !err.aborted {
                dom::error(
                    surface.error_id,
                    &format!("Impact query failed: {}", err.message),
                );
                m::set(surface.impact_key, json!({ "error": err.message }));
                let mut next = surface_state(surface);
                next["side_tab"] = json!("changes");
                save_surface_state(surface, &next);
                render_surface(surface, false);
            }
        }
    }
}
