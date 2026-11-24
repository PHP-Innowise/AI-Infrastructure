#![allow(unused_imports)]
use super::inspector::*;
use super::interactions::*;
use super::layout::*;
use super::model::*;
use super::render::*;
use super::*;

fn sample_catalog() -> V {
    json!({
        "system": { "name": "Harbor Market", "root": "harbor-system" },
        "shared_sources": ["policies/contracts.md"],
        "services": [
            {
                "id": "gateway",
                "name": "API Gateway",
                "role": "edge-gateway",
                "language": "typescript",
                "provides": [],
                "consumes": [
                    { "name": "orders.http", "kind": "http" },
                    { "name": "catalog.graphql", "kind": "graphql" }
                ]
            },
            {
                "id": "orders",
                "name": "Orders",
                "language": "php",
                "provides": [
                    { "name": "orders.http", "kind": "http", "version": "1.0" },
                    { "name": "order.placed", "kind": "event", "version": "2.0" }
                ],
                "consumes": [
                    { "name": "payments.charge", "kind": "rpc" },
                    { "name": "inventory.reserve", "kind": "http" }
                ],
                "shared_sources": ["policies/contracts.md"]
            },
            {
                "id": "inventory",
                "name": "Inventory",
                "language": "php",
                "provides": [{ "name": "inventory.reserve", "kind": "http" }],
                "consumes": [{ "name": "order.placed", "kind": "event" }]
            },
            {
                "id": "payments",
                "name": "Payments",
                "role": "core-platform",
                "language": "go",
                "provides": [{ "name": "payments.charge", "kind": "rpc" }],
                "consumes": [{ "name": "missing.audit", "kind": "other" }]
            },
            {
                "id": "catalog",
                "name": "Catalog",
                "language": "php",
                "provides": [{ "name": "catalog.graphql", "kind": "graphql" }],
                "consumes": []
            }
        ]
    })
}

#[test]
fn normalizes_all_seven_contract_arrow_kinds() {
    assert_eq!(normalize_contract_arrow_kind("REST"), "http");
    assert_eq!(normalize_contract_arrow_kind("kafka"), "event");
    assert_eq!(normalize_contract_arrow_kind("gRPC"), "rpc");
    assert_eq!(normalize_contract_arrow_kind("gql"), "graphql");
    assert_eq!(normalize_contract_arrow_kind("custom"), "other");
    assert_eq!(normalize_contract_arrow_kind("shared-source"), "shared");
    assert_eq!(normalize_contract_arrow_kind("imports"), "code");
    let legend = contract_arrow_legend_html();
    for kind in ["http", "event", "rpc", "graphql", "other", "shared", "code"] {
        assert!(legend.contains(&format!("data-sys-edge-kind=\"{kind}\"")));
    }
    assert!(legend.contains("Broken risk"));
    assert!(legend.contains("Undeclared drift"));
}

#[test]
fn synthesizes_topology_and_detects_cycles_and_broken_contracts() {
    let doc = synthesize_map_from_catalog(&sample_catalog(), &json!({}));
    let nodes = m::array(&doc["nodes"]);
    let edges = m::array(&doc["edges"]);
    assert_eq!(nodes.len(), 6); // 5 member services + __system__
    assert!(edges
        .iter()
        .any(|e| e["from"] == "gateway" && e["to"] == "orders" && e["kind"] == "http"));
    assert!(edges
        .iter()
        .any(|e| e["from"] == "orders" && e["to"] == "payments" && e["kind"] == "rpc"));
    // Unmatched contract missing.audit flags broken_risk!
    assert!(edges
        .iter()
        .any(|e| e["from"] == "payments" && m::yes(&e["broken_risk"])));

    // orders <-> inventory form a 2-service cycle (orders -> inventory.reserve, inventory -> order.placed)
    let cycles = find_cross_service_cycles(&doc);
    assert_eq!(cycles.len(), 1);
    assert!(cycles[0].contains(&"orders".to_owned()));
    assert!(cycles[0].contains(&"inventory".to_owned()));
}

#[test]
fn computes_blast_radius_depths_and_context_brief() {
    let doc = synthesize_map_from_catalog(&sample_catalog(), &json!({}));
    // Payments is consumed by Orders (wave 1), which is consumed by Gateway & Inventory (wave 2)
    let full = compute_blast_radius(&doc, &["payments".to_owned()], &[], 32);
    assert!(full.seeds.contains("payments"));
    assert!(full.direct_consumers.contains("orders"));
    assert!(full.downstream_consumers.contains("gateway"));
    assert!(full.downstream_consumers.contains("inventory"));

    let direct_only = compute_blast_radius(&doc, &["payments".to_owned()], &[], 1);
    assert!(direct_only.direct_consumers.contains("orders"));
    assert!(direct_only.downstream_consumers.is_empty());

    let none = compute_blast_radius(&doc, &["payments".to_owned()], &[], 0);
    assert!(none.direct_consumers.is_empty());

    let mut state = default_surface_state();
    state["overlay"] = json!({
        "seed_services": ["payments"],
        "depth": 32,
        "changed_contracts": []
    });
    let brief = build_context_brief(&doc, &state);
    assert!(brief.contains("payments"));
    assert!(brief.contains("orders"));
    assert!(brief.contains("gateway"));
}

#[test]
fn computes_paths_shared_neighbors_and_layers() {
    let doc = synthesize_map_from_catalog(&sample_catalog(), &json!({}));
    let paths = shortest_service_paths(&doc, "gateway", "payments", &BTreeSet::new());
    assert_eq!(paths, vec![vec!["gateway", "orders", "payments"]]);

    let mut skip = BTreeSet::new();
    skip.insert("orders".to_owned());
    assert!(shortest_service_paths(&doc, "gateway", "payments", &skip).is_empty());

    let (both_use, uses_both) = shared_neighbors(&doc, "gateway", "inventory");
    assert!(both_use.contains(&"orders".to_owned()));
    assert!(uses_both.is_empty());

    let (tiers, _) = assign_service_layers(&doc);
    assert_eq!(tiers.get("gateway"), Some(&0));
    assert_eq!(tiers.get("__system__"), Some(&3));
}

#[test]
fn cursor_anchored_zoom_keeps_world_point_under_cursor() {
    let anchor = (300.0, 200.0);
    let (s1, px1, py1) = (1.0, 40.0, 30.0);
    let wx = (anchor.0 - px1) / s1;
    let wy = (anchor.1 - py1) / s1;
    let (s2, px2, py2) = zoom_at(s1, px1, py1, anchor, 1.5);
    assert!((s2 - 1.5).abs() < 1e-6);
    let screen_x = wx * s2 + px2;
    let screen_y = wy * s2 + py2;
    assert!((screen_x - anchor.0).abs() <= 1.0);
    assert!((screen_y - anchor.1).abs() <= 1.0);
}

#[test]
fn renders_svg_canvas_with_explain_attributes_and_unfolded_frames() {
    let mut state = default_surface_state();
    state["spread"] = json!(["orders"]);
    state["overlay"] = json!({
        "seed_services": ["orders"],
        "depth": 1,
        "step_states": { "orders": "running" },
        "cross_service_unopened": ["inventory"]
    });
    let doc = expand_unfolded_nodes(
        &synthesize_map_from_catalog(&sample_catalog(), &json!({})),
        &state,
    );
    let (html, w, h) = render_canvas_html(CHANGES_SURFACE, &doc, &state);
    assert!(w > 300.0 && h > 200.0);
    assert!(html.contains("data-explain-services=\"orders\""));
    assert!(html.contains("data-step-state=\"running\""));
    assert!(html.contains("data-unopened=\"true\""));
    assert!(html.contains("system-service-frame"));
}

#[test]
fn normalize_map_doc_propagates_contract_risks_and_undeclared_code_drift() {
    let raw = json!({
        "nodes": [
            { "id": "gateway", "kind": "service" },
            { "id": "orders", "kind": "service" }
        ],
        "edges": [
            { "from": "gateway", "to": "orders", "kind": "code", "undeclared_in_code": true }
        ],
        "contract_risks": [
            { "consumer": "gateway", "provider": "orders", "contract": "orders.http", "kind": "http" },
            { "consumer": "orders", "contract": "billing.invoice", "kind": "rpc" }
        ]
    });
    let doc = normalize_map_doc(
        &raw,
        &sample_catalog(),
        &json!({}),
        &default_surface_state(),
    );
    let edges = m::array(&doc["edges"]);
    let gw_orders = edges
        .iter()
        .find(|e| e["from"] == "gateway" && e["to"] == "orders")
        .expect("gateway -> orders edge");
    assert_eq!(gw_orders["provenance"], "undeclared_in_code");
    assert_eq!(gw_orders["broken_risk"], true);
    assert!(edges.iter().any(|e| e["from"] == "orders"
        && e["to"] == "__unmatched__:billing.invoice"
        && e["broken_risk"] == true));
}

#[test]
fn normalizes_backend_catalog_declared_nesting_and_system_node_schema() {
    let backend_catalog = json!({
        "schema_version": 1,
        "system": "Harbor Market",
        "services": [
            {
                "id": "gateway",
                "root": "services/gateway",
                "access": "available",
                "declared": {
                    "owner": "edge-team",
                    "description": "Public HTTP gateway",
                    "stack": "typescript",
                    "capabilities": [{ "id": "public-routing", "description": "Edge routing" }],
                    "provides": [],
                    "consumes": [{ "service": "orders", "contract": "orders-api", "version": "1.0" }]
                }
            },
            {
                "id": "orders",
                "root": "services/orders",
                "access": "available",
                "declared": {
                    "owner": "checkout-team",
                    "description": "Order lifecycle",
                    "stack": "php",
                    "capabilities": [{ "id": "order-checkout", "description": "Checkout flow" }],
                    "provides": [{ "id": "orders-api", "kind": "http", "version": "1.2", "sources": ["routes/api.php"] }],
                    "consumes": []
                }
            }
        ],
        "relationships": [],
        "warnings": []
    });

    let raw_map = json!({
        "nodes": [
            {
                "id": "gateway",
                "label": "API Gateway",
                "kind": "service",
                "service": "gateway",
                "root": "services/gateway",
                "stack": "typescript"
            },
            {
                "id": "orders",
                "label": "Orders Service",
                "kind": "service",
                "service": "orders",
                "root": "services/orders",
                "stack": "php"
            },
            {
                "id": "contract:orders:orders-api",
                "label": "orders-api",
                "kind": "contract",
                "service": "orders",
                "parent": "orders",
                "contract_kind": "http",
                "version": "1.2"
            },
            {
                "id": "source:orders:routes/api.php",
                "label": "routes/api.php",
                "kind": "source",
                "service": "orders",
                "parent": "orders",
                "path": "routes/api.php"
            }
        ],
        "edges": [
            {
                "from": "gateway",
                "to": "orders",
                "consumer": "gateway",
                "provider": "orders",
                "level": "service",
                "kinds": ["http"],
                "provenance": "verified",
                "certainty": "EXTRACTED",
                "broken_risk": true,
                "contracts": [
                    {
                        "id": "orders-api",
                        "kind": "http",
                        "consumer_version": "1.0",
                        "provider_version": "1.2",
                        "provider_sources": ["routes/api.php"],
                        "consumer_sources": ["src/OrdersClient.ts"],
                        "broken_risk": true
                    }
                ]
            },
            {
                "from": "gateway",
                "to": "contract:orders:orders-api",
                "level": "detail",
                "kinds": ["http"],
                "provenance": "verified",
                "certainty": "EXTRACTED",
                "broken_risk": false,
                "contracts": []
            },
            {
                "from": "contract:orders:orders-api",
                "to": "source:orders:routes/api.php",
                "level": "detail",
                "kinds": ["code"],
                "provenance": "verified",
                "certainty": "EXTRACTED",
                "broken_risk": false,
                "contracts": []
            }
        ]
    });

    let mut state = default_surface_state();
    state["spread"] = json!(["orders"]);
    let doc = normalize_map_doc(&raw_map, &backend_catalog, &json!({}), &state);

    let nodes = m::array(&doc["nodes"]);
    let orders = nodes
        .iter()
        .find(|n| n["id"] == "orders")
        .expect("orders node");
    assert_eq!(orders["name"], "Orders Service");
    assert_eq!(orders["language"], "php");
    assert_eq!(orders["folder"], "services/orders");
    assert_eq!(orders["owner"], "checkout-team");
    assert!(nodes
        .iter()
        .any(|n| n["id"] == "__system__" && m::text(n, "name").contains("Harbor Market")));

    let detail_edge = m::array(&doc["edges"])
        .into_iter()
        .find(|e| e["to"] == "contract:orders:orders-api")
        .expect("detail edge");
    assert_eq!(detail_edge["child_link"], true);

    let (svg_html, _, _) = render_canvas_html(SERVICES_SURFACE, &doc, &state);
    assert!(!svg_html.contains("data-explain-services=\"gateway,contract:orders"));
    assert!(svg_html.contains("data-explain-services=\"gateway,orders\""));
    assert!(svg_html.contains("data-explain-services=\"orders\""));

    state["focus"] = json!("orders");
    let node_side = render_side_html(SERVICES_SURFACE, &doc, &state);
    assert!(node_side.contains("order-checkout"));

    state["selected_edge"] = json!({ "from": "gateway", "to": "orders" });
    let edge_side = render_side_html(SERVICES_SURFACE, &doc, &state);
    assert!(edge_side.contains("1.0 → 1.2"));
    assert!(edge_side.contains("routes/api.php"));
    assert!(edge_side.contains("Open orders map"));
}

#[test]
fn renders_backend_changes_and_impact_contract_consumers_shapes() {
    m::set(
        SERVICES_SURFACE.impact_key,
        json!({
            "service": "orders",
            "impacted_contracts": [{ "contract": "orders-api", "kind": "http" }],
            "contract_consumers": [
                {
                    "provider": "orders",
                    "consumer": "gateway",
                    "contract": "orders-api",
                    "consumer_sources": ["src/OrdersClient.ts"],
                    "unopened_sources": ["src/OrdersClient.ts"],
                    "broken_risk": false
                }
            ],
            "cross_service_unopened": [
                { "consumer": "gateway", "contract": "orders-api", "path": "src/OrdersClient.ts" }
            ]
        }),
    );
    m::set(
        SERVICES_SURFACE.changes_key,
        json!({
            "system": "Harbor Market",
            "total_files": 1,
            "services": [
                {
                    "service": "orders",
                    "changed_files": [{ "path": "src/OrderController.php" }]
                }
            ]
        }),
    );
    let mut state = default_surface_state();
    state["side_tab"] = json!("changes");
    let html = render_side_html(
        SERVICES_SURFACE,
        &json!({ "nodes": [], "edges": [] }),
        &state,
    );
    assert!(html.contains("Impact for orders-api"));
    assert!(html.contains("gateway (orders-api) src/OrdersClient.ts"));
    assert!(html.contains("src/OrderController.php"));
}

#[test]
fn a_link_between_services_shows_the_code_on_both_sides_when_asked() {
    // Not asked yet, or asked for another link: a button.
    let html = bridge_html(SERVICES_SURFACE, "web", "orders", &V::Null);
    assert!(html.contains("<h5 class=\"system-side-subheading\">Code on both sides</h5>"));
    assert!(html.contains("data-action=\"system-map-bridge\""), "{html}");
    assert!(
        html.contains("&quot;from&quot;:&quot;web&quot;")
            && html.contains("&quot;to&quot;:&quot;orders&quot;")
    );
    let bridge = json!({
        "kind": "system_bridge", "from": "web", "to": "orders",
        "contracts": ["orders-http-v1"],
        "consumer": {"service": "web", "graph": false, "more": 0, "sites": [
            {"path": "src/OrdersClient.ts", "contracts": ["orders-http-v1"], "in_graph": false, "reach": null}
        ]},
        "provider": {"service": "orders", "graph": true, "more": 2, "cut": true, "sites": [
            {"path": "src/OrderController.php", "contracts": ["orders-http-v1"], "declared": true, "found": false, "in_graph": true, "reach": {"files": 3, "tests": 1}},
            {"path": "src/routes.php", "contracts": ["orders-http-v1"], "declared": false, "found": true, "in_graph": true, "reach": {"files": 0, "tests": 0}}
        ]}
    });
    assert!(
        bridge_html(SERVICES_SURFACE, "billing", "orders", &bridge).contains("system-map-bridge")
    );
    let html = bridge_html(SERVICES_SURFACE, "web", "orders", &bridge);
    assert!(!html.contains("system-map-bridge"));
    assert!(html.contains("<p class=\"system-note\"><strong>web</strong> uses it · no code graph yet: how far each file reaches is not told</p>"), "{html}");
    assert!(html.contains("<p class=\"system-note\"><strong>orders</strong> makes it</p>"));
    assert!(html.contains(
        "<code>src/OrdersClient.ts</code><div class=\"system-note\">names it: orders-http-v1</div>"
    ));
    assert!(html.contains("<code>src/OrderController.php</code><div class=\"system-note\">made here by its passport: orders-http-v1 · a change reaches 3 files of orders</div>"));
    assert!(html.contains("<code>src/routes.php</code><div class=\"system-note\">names it: orders-http-v1 · nothing else in it uses this file</div>"));
    // A document that names the contract says it is one.
    let mut documented = bridge.clone();
    documented["consumer"]["sites"][0]["path"] = json!("README.md");
    documented["consumer"]["sites"][0]["document"] = json!(true);
    assert!(bridge_html(SERVICES_SURFACE, "web", "orders", &documented)
        .contains("<code>README.md</code><div class=\"system-note\">a document that names it: orders-http-v1</div>"));
    // Each file opens on the code map of its service.
    assert!(html.contains("data-action=\"inspect-system-file-graph\" data-payload=\"{&quot;path&quot;:&quot;src/OrderController.php&quot;,&quot;service&quot;:&quot;orders&quot;}\""), "{html}");
    assert_eq!(html.matches("inspect-system-file-graph").count(), 3);
    assert!(html.contains("<p class=\"system-note\">and 2 more</p>"));
    assert!(html.contains("Not every file of it was read: there may be more."));
    // A side with nothing to show says so.
    let mut empty = bridge.clone();
    empty["consumer"]["sites"] = json!([]);
    assert!(bridge_html(SERVICES_SURFACE, "web", "orders", &empty)
        .contains("No file was found to name it."));
    // Backend #31 additive fields: confidence, run_coverage, opened_by_run, from_symbols.
    let mut with_cov = bridge;
    with_cov["confidence"] = json!({"certain_refs": 4, "guessed_refs": 1, "ratio": 0.8});
    with_cov["run_coverage"] = json!({
        "run_id": "run-7",
        "consumer_opened": 1,
        "consumer_total": 2,
        "provider_opened": 2,
        "provider_total": 3,
    });
    with_cov["consumer"]["sites"][0]["opened_by_run"] = json!(true);
    with_cov["consumer"]["sites"][0]["from_symbols"] = json!(["OrdersClient.place"]);
    let cov_html = bridge_html(SERVICES_SURFACE, "web", "orders", &with_cov);
    assert!(cov_html.contains("80%"));
    assert!(cov_html.contains("1/2 consumer · 2/3 provider opened"));
    assert!(cov_html.contains("data-opened-by-run=\"true\""));
    assert!(cov_html.contains("via OrdersClient.place"));
    assert!(cov_html.contains("opened in run"));
}

#[test]
fn the_waves_of_a_change_are_stepped_through_and_what_is_not_reached_yet_waits() {
    let doc = synthesize_map_from_catalog(&sample_catalog(), &json!({}));
    let mut state = default_surface_state();
    state["overlay"] = json!({"seed_services": ["payments"], "depth": 32, "changed_contracts": []});
    // payments is where it starts, orders is wave 1, gateway and
    // inventory are wave 2.
    let blast = blast_of(&doc, &state);
    assert_eq!(last_wave(&blast), 2);
    let row = wave_scrubber_html(CHANGES_SURFACE, &blast, &state);
    assert!(row.starts_with("<div class=\"system-waves\" role=\"group\" aria-label=\"Waves of the change\"><span class=\"graph-arrow-legend-title\">Waves:</span>"));
    assert!(
        row.contains(
            "aria-pressed=\"true\" title=\"Every service the change reaches\">All</button>"
        ),
        "{row}"
    );
    assert!(row.contains("title=\"Where the change starts: 1 service\">0</button>"));
    assert!(
        row.contains("title=\"As far as wave 1: 1 service more\">1</button>"),
        "{row}"
    );
    assert!(row.contains("title=\"As far as wave 2: 2 services more\">2</button>"));
    assert!(row.contains("data-action=\"system-map-wave-play\""));
    // As far as wave 1: gateway and inventory wait, faint, and so do
    // the links from them; orders is reached.
    state["wave"] = json!(1);
    let row = wave_scrubber_html(CHANGES_SURFACE, &blast, &state);
    assert!(
        row.contains("aria-pressed=\"true\" title=\"As far as wave 1: 1 service more\">1</button>")
    );
    assert!(row.contains(
        "aria-pressed=\"false\" title=\"Every service the change reaches\">All</button>"
    ));
    let (html, _, _) = render_canvas_html(CHANGES_SURFACE, &doc, &state);
    let waiting = |id: &str| {
        let at = html
            .find(&format!("data-n=\"{id}\""))
            .unwrap_or_else(|| panic!("{id} in {html}"));
        let tag = &html[at..at + html[at..].find('>').unwrap()];
        tag.contains("data-beyond=\"true\"")
    };
    assert!(waiting("gateway") && waiting("inventory"));
    assert!(!waiting("orders") && !waiting("payments"));
    // The links the change has not passed yet wait as well: those
    // from the services of wave 2.
    let links = html
        .split("<path class=\"graph-map-edge system-map-edge")
        .skip(1)
        .map(|tag| &tag[..tag.find('>').unwrap()])
        .collect::<Vec<_>>();
    let link_waits = |from: &str, to: &str| {
        links
            .iter()
            .find(|tag| tag.contains(&format!("data-from=\"{from}\" data-to=\"{to}\"")))
            .unwrap_or_else(|| panic!("{from} -> {to} in {html}"))
            .contains("data-beyond=\"true\"")
    };
    assert!(link_waits("gateway", "orders"));
    assert!(!link_waits("orders", "payments"));
    // All of it again: nothing waits.
    state["wave"] = V::Null;
    let (html, _, _) = render_canvas_html(CHANGES_SURFACE, &doc, &state);
    assert!(!html.contains("data-beyond"));
    // A change that reaches nothing past where it starts has no waves.
    state["overlay"] = json!({"seed_services": ["gateway"], "depth": 32, "changed_contracts": []});
    assert_eq!(
        wave_scrubber_html(CHANGES_SURFACE, &blast_of(&doc, &state), &state),
        ""
    );
}

#[test]
fn hover_pointed_key_and_edge_adjacency_dedup_cleanly() {
    assert_eq!(hover_pointed_key(None, None), "");
    assert_eq!(hover_pointed_key(Some("orders"), None), "n:orders");
    assert_ne!(
        hover_pointed_key(Some("orders"), None),
        hover_pointed_key(None, Some(("orders", "payments")))
    );
    assert_eq!(
        hover_pointed_key(None, Some(("orders", "payments"))),
        "e:orders\u{1f}payments"
    );

    let adj = build_edge_adjacency(&[
        ("gateway".to_owned(), "orders".to_owned()),
        ("orders".to_owned(), "payments".to_owned()),
        ("".to_owned(), "ignored".to_owned()),
    ]);
    assert_eq!(
        adj.get("orders").cloned().unwrap_or_default(),
        BTreeSet::from(["gateway".to_owned(), "payments".to_owned()])
    );
    assert_eq!(
        adj.get("gateway").cloned().unwrap_or_default(),
        BTreeSet::from(["orders".to_owned()])
    );
    assert!(!adj.contains_key("ignored"));
}

#[test]
fn barycenter_ordering_and_port_spreading_untangle_crossed_services() {
    // Inputs in columns are inverted relative to edges:
    // Column 0: a, b  |  Column 1: y (connected to a), x (connected to b)
    // WAIT: to force a crossing before barycenter:
    // col 0: top=a, bottom=b; edges: a -> x, b -> y; if x was added after y, raw order puts y above x causing a crossing!
    let doc = json!({
        "nodes": [
            {"id": "a", "name": "Service A", "kind": "service"},
            {"id": "b", "name": "Service B", "kind": "service"},
            {"id": "y", "name": "Service Y", "kind": "service"},
            {"id": "x", "name": "Service X", "kind": "service"}
        ],
        "edges": [
            {"from": "a", "to": "x", "kind": "http"},
            {"from": "b", "to": "y", "kind": "rpc"},
            {"from": "a", "to": "y", "kind": "event"}
        ]
    });
    let layout = layout_system_nodes(&doc, false);
    let pos = |id: &str| {
        layout
            .nodes
            .iter()
            .find(|n| n.id == id)
            .map(|n| (n.x, n.y))
            .expect(id)
    };
    // `a` connects to both `x` and `y`, while `b` connects only to `y`.
    // Note `a` has TWO outgoing right-side wires (`a -> x`, `a -> y`), so their start Y ports must spread apart!
    assert_eq!(pos("a").0, MARGIN);
    let wire_starts: Vec<f64> = layout
        .wires
        .iter()
        .filter(|w| doc["edges"][w.edge_index]["from"] == "a")
        .filter_map(|w| {
            let after_m = w.path_d.strip_prefix('M')?;
            let first_pair = after_m.split_whitespace().next()?;
            let (_, y_str) = first_pair.split_once(',')?;
            y_str.parse::<f64>().ok()
        })
        .collect();
    assert_eq!(wire_starts.len(), 2);
    assert!(
        (wire_starts[0] - wire_starts[1]).abs() >= 2.0,
        "expected spread ports on multi-edge source, got {:?}",
        wire_starts
    );

    // Now verify crossing removal when two disjoint edges cross in input order:
    let crossed = json!({
        "nodes": [
            {"id": "g1", "name": "G1", "kind": "service"},
            {"id": "g2", "name": "G2", "kind": "service"},
            {"id": "s_for_g2", "name": "S2", "kind": "service"},
            {"id": "s_for_g1", "name": "S1", "kind": "service"}
        ],
        "edges": [
            {"from": "g1", "to": "s_for_g1", "kind": "http"},
            {"from": "g2", "to": "s_for_g2", "kind": "http"}
        ]
    });
    let cl = layout_system_nodes(&crossed, false);
    let y_of = |id: &str| cl.nodes.iter().find(|n| n.id == id).unwrap().y;
    assert!(
        y_of("s_for_g1") < y_of("s_for_g2"),
        "barycenter sweep should align s_for_g1 above s_for_g2 matching g1 above g2"
    );
}

#[test]
fn contract_legend_switchboard_toggles_solos_and_hides_matching_edges() {
    let doc = synthesize_map_from_catalog(&sample_catalog(), &json!({}));
    let mut state = default_surface_state();

    let legend = contract_arrow_legend_html_for(SERVICES_SURFACE, &doc, &state);
    crate::test_support::assert_no_inline_hex_colors(&legend);
    assert!(legend.contains("data-action=\"system-map-kind-toggle\""));
    assert!(legend.contains("<button type=\"button\" class=\"graph-arrow-legend-item\""));
    assert!(!legend.contains("Show all"));

    // Toggle rpc off -> Show all appears and rpc edge gets data-kind-hidden="true"
    state = switched_contract_kind(state, "rpc", false);
    assert_eq!(
        hidden_contract_kinds(&state),
        [false, false, true, false, false, false, false]
    );
    let legend_off = contract_arrow_legend_html_for(SERVICES_SURFACE, &doc, &state);
    assert!(legend_off.contains("Show all"));
    assert!(legend_off.contains("data-sys-edge-kind=\"rpc\" aria-pressed=\"false\""));

    let (canvas, _, _) = render_canvas_html(SERVICES_SURFACE, &doc, &state);
    crate::test_support::assert_no_inline_hex_colors(&canvas);
    assert!(canvas.contains("data-sys-edge-kind=\"rpc\""));
    assert!(canvas.contains("data-kind-hidden=\"true\""));

    // Alt+click solo on http isolates http only; second solo resets to all visible.
    let solo = switched_contract_kind(default_surface_state(), "http", true);
    assert_eq!(
        hidden_contract_kinds(&solo),
        [false, true, true, true, true, true, true]
    );
    let reset = switched_contract_kind(solo, "http", true);
    assert_eq!(hidden_contract_kinds(&reset), [false; 7]);
    let preset_all = preset_contract_kinds(state, "all");
    assert_eq!(hidden_contract_kinds(&preset_all), [false; 7]);
}

#[test]
fn system_radar_lod_and_edge_label_deoverlap_work_cleanly() {
    assert_eq!(lod(0.4), "far");
    assert_eq!(lod(0.9), "mid");
    assert_eq!(lod(1.4), "near");
    assert_eq!(lod(f64::NAN), "far");

    let doc = json!({
        "nodes": [
            {"id": "gw", "name": "Gateway", "kind": "service"},
            {"id": "ord", "name": "Orders", "kind": "service"}
        ],
        "edges": [
            {"from": "gw", "to": "ord", "kind": "http", "contracts": [{"id": "c1"}]},
            {"from": "gw", "to": "ord", "kind": "rpc", "contracts": [{"id": "c2"}]}
        ]
    });
    let mut state = default_surface_state();
    state["focus"] = json!("gw");
    let (plane_html, radar_svg, w, h) = render_canvas_with_radar(CHANGES_SURFACE, &doc, &state);
    crate::test_support::assert_no_inline_hex_colors(&plane_html);
    crate::test_support::assert_no_inline_hex_colors(&radar_svg);
    assert!(radar_svg.contains("class=\"graph-radar-picture\""));
    assert!(radar_svg.contains("data-state=\"selected\""));
    assert!(radar_svg.contains("class=\"graph-radar-view\""));

    // Parallel edge labels between gw -> ord must be de-overlapped to distinct Y coordinates.
    let label_ys: Vec<f64> = plane_html
        .split("<g class=\"graph-map-edge-tie system-map-edge-label\"")
        .skip(1)
        .filter_map(|chunk| {
            let text_at = chunk.find("<text ")?;
            let y_at = chunk[text_at..].find(" y=\"")?;
            let start = text_at + y_at + 4;
            let end = start + chunk[start..].find('"')?;
            chunk[start..end].parse::<f64>().ok()
        })
        .collect();
    assert_eq!(label_ys.len(), 2);
    assert!(
        (label_ys[0] - label_ys[1]).abs() >= 16.0,
        "overlapping edge labels should step apart: {:?}",
        label_ys
    );

    assert!(radar_window_rect((0.0, 0.0), 1.0, (w + 100.0, h + 100.0), (w, h)).is_none());
    let partial = radar_window_rect((-40.0, -20.0), 1.5, (300.0, 200.0), (w, h)).unwrap();
    assert!(partial.2 > 0.0 && partial.3 > 0.0);
    let centered = centred((200.0, 120.0), 1.5, (600.0, 400.0));
    assert_eq!(centered, (0.0, 20.0));
}

#[test]
fn system_map_delegates_camera_resize_and_radar_to_graph_viewport() {
    let fit_cam = graph_viewport::fit(
        (1200.0, 600.0),
        (800.0, 400.0),
        graph_viewport::FitOptions::for_system_map(),
    );
    assert_eq!(
        camera_for_fit((1200.0, 600.0), (800.0, 400.0)),
        (fit_cam.scale, fit_cam.tx, fit_cam.ty)
    );

    let mut cam = Camera::default();
    cam.apply_viewport_cam(fit_cam);
    assert_eq!(cam.viewport_cam(), fit_cam);

    assert!(matches!(
        graph_viewport::resize_refit_decision(
            cam.viewport_cam(),
            true,
            (1200.0, 600.0),
            (900.0, 500.0),
            true,
            graph_viewport::FitOptions::for_system_map(),
        ),
        graph_viewport::ResizeAction::Refit(_)
    ));
    assert_eq!(
        graph_viewport::resize_refit_decision(
            cam.viewport_cam(),
            false,
            (1200.0, 600.0),
            (900.0, 500.0),
            true,
            graph_viewport::FitOptions::for_system_map(),
        ),
        graph_viewport::ResizeAction::SyncRadar(fit_cam)
    );

    let mut st = default_surface_state();
    let doc = json!({"nodes": [], "edges": []});
    let details_html = render_side_html(SERVICES_SURFACE, &doc, &st);
    assert!(details_html.contains("aria-selected=\"true\" tabindex=\"0\""));
    assert_eq!(details_html.matches("tabindex=\"-1\"").count(), 2);

    st["side_tab"] = json!("analyze");
    let analyze_html = render_side_html(SERVICES_SURFACE, &doc, &st);
    assert_eq!(
        analyze_html
            .matches("aria-selected=\"true\" tabindex=\"0\"")
            .count(),
        1
    );
    assert_eq!(analyze_html.matches("tabindex=\"-1\"").count(), 2);
}

#[test]
fn bridge_html_renders_semantic_framework_and_db_bridges_without_inline_hex_colors() {
    let bridge = json!({
        "kind": "system_bridge",
        "from": "web",
        "to": "orders",
        "contracts": ["orders-http-v1"],
        "consumer": {"service": "web", "graph": true, "more": 0, "sites": []},
        "provider": {"service": "orders", "graph": true, "more": 0, "sites": []},
        "semantic_bridges": {
            "routes": [{
                "route": "/api/orders/{id}",
                "method": "POST",
                "methods": ["POST"],
                "direction": "from->to",
                "caller": "web",
                "provider": "orders",
                "caller_files": ["src/OrdersClient.ts"],
                "provider_files": ["src/OrderController.php"]
            }],
            "events": [{
                "event": "OrderCreated",
                "direction": "to->from",
                "producer": "orders",
                "consumer": "web",
                "producer_files": ["src/OrderService.php"],
                "consumer_files": ["src/OrderSyncListener.ts"]
            }],
            "di_bindings": [{
                "service": "PaymentGateway",
                "from_kinds": ["binds"],
                "to_kinds": ["implements"],
                "from_files": ["src/Providers/AppProvider.ts"],
                "to_files": ["src/StripeGateway.php"]
            }],
            "tables": [{
                "table": "orders",
                "coupling": "cross_read_write",
                "from_access": ["read"],
                "to_access": ["read", "write"],
                "from_files": ["src/OrderQuery.ts"],
                "to_files": ["src/OrderRepository.php"]
            }],
            "summary": {"routes": 1, "events": 1, "di_bindings": 1, "tables": 1, "total": 4}
        }
    });
    let html = bridge_html(SERVICES_SURFACE, "web", "orders", &bridge);
    crate::test_support::assert_no_inline_hex_colors(&html);
    assert!(
        html.contains("Semantic framework &amp; DB bridges"),
        "{html}"
    );
    assert!(
        html.contains("<span class=\"system-chip\">4</span>"),
        "{html}"
    );
    assert!(html.contains("/api/orders/{id}"), "{html}");
    assert!(html.contains("POST"), "{html}");
    assert!(html.contains("OrderCreated"), "{html}");
    assert!(html.contains("PaymentGateway"), "{html}");
    assert!(
        html.contains("data-coupling=\"cross_read_write\""),
        "{html}"
    );
    assert!(html.contains("read / write"), "{html}");
    assert!(html.contains("<code>orders</code>"), "{html}");
    let payloads = crate::test_support::payloads(&html, "inspect-system-file-graph");
    assert!(payloads.contains(&json!({"path": "src/OrderController.php", "service": "orders"})));
    assert!(payloads.contains(&json!({"path": "src/OrderSyncListener.ts", "service": "web"})));

    let mut empty_sem = bridge.clone();
    empty_sem["semantic_bridges"] = json!({
        "routes": [], "events": [], "di_bindings": [], "tables": []
    });
    let empty_html = bridge_html(SERVICES_SURFACE, "web", "orders", &empty_sem);
    crate::test_support::assert_no_inline_hex_colors(&empty_html);
    assert!(empty_html.contains("No shared routes, events, DI bindings, or tables detected."));
}

#[test]
fn system_structure_digest_pins_isolate_and_roving_attrs() {
    let doc = json!({
        "nodes": [
            {"id": "api", "name": "api", "kind": "service"},
            {"id": "orders", "name": "orders", "kind": "service"},
            {"id": "db", "name": "db", "kind": "service"}
        ],
        "edges": [
            {"from": "api", "to": "orders", "kind": "http"},
            {"from": "orders", "to": "db", "kind": "code"}
        ]
    });
    let base_state = json!({"focus": "api"});
    let sel_state = json!({"focus": "orders"});
    assert_eq!(
        system_structure_digest(&doc, &base_state, &BTreeMap::new()),
        system_structure_digest(&doc, &sel_state, &BTreeMap::new()),
    );

    let isolated = filtered_doc(
        &doc,
        &json!({"isolate_active": true, "isolate_seed": "api", "isolate_depth": 1}),
    );
    let kept_ids: Vec<String> = m::array(&isolated["nodes"]).iter().map(node_id).collect();
    assert_eq!(kept_ids, vec!["api", "orders"]);

    let mut pins = BTreeMap::new();
    pins.insert("api".to_owned(), (222.0, 111.0));
    let layout = layout_system_nodes_with_pins(&doc, false, &pins);
    assert_eq!(layout.nodes[0].x, 222.0);
    assert_eq!(layout.nodes[0].y, 111.0);
    let html = render_canvas_html_with_layout(SERVICES_SURFACE, &doc, &base_state, &layout);
    assert!(html.contains("tabindex=\"0\""));
    assert!(html.contains("tabindex=\"-1\""));
    crate::test_support::assert_no_inline_hex_colors(&html);
}

#[test]
fn system_digest_pin_deltas_and_dense_scene_sync_behave_consistently() {
    let doc = json!({
        "nodes": [
            {"id": "api", "name": "api", "kind": "service"},
            {"id": "api::c1", "name": "OrderCreate", "kind": "contract", "parent_service": "api"},
            {"id": "orders", "name": "orders", "kind": "service"}
        ],
        "edges": [
            {"from": "api", "to": "orders", "kind": "http"},
            {"from": "api::c1", "to": "orders", "kind": "contract"}
        ]
    });
    let s0 = json!({});
    let d0 = system_structure_digest(&doc, &s0, &BTreeMap::new());
    // Visual-only state changes keep structural digest equal so try_patch_system_canvas_in_place handles them.
    assert_eq!(
        d0,
        system_structure_digest(
            &doc,
            &json!({"overlay": {"seed_services": ["api"]}}),
            &BTreeMap::new()
        )
    );
    assert_eq!(
        d0,
        system_structure_digest(&doc, &json!({"wave": 1}), &BTreeMap::new())
    );
    assert_eq!(
        d0,
        system_structure_digest(
            &doc,
            &json!({"tool": "path", "path_from": "api", "path_to": "orders"}),
            &BTreeMap::new()
        )
    );
    m::set("system-failed-services", json!(["orders"]));
    let d_failed = system_structure_digest(&doc, &s0, &BTreeMap::new());
    m::take("system-failed-services");
    assert_eq!(d0, d_failed);
    // Structural toggles (layers, kinds_off, pins) still invalidate the digest.
    assert_ne!(
        d0,
        system_structure_digest(&doc, &json!({"layers": true}), &BTreeMap::new())
    );
    assert_ne!(
        d0,
        system_structure_digest(&doc, &json!({"kinds_off": ["http"]}), &BTreeMap::new())
    );

    // Pinning parent "api" moves both its frame and unfolded child "api::c1" by delta,
    // and moving "api" to the right of "orders" flips outgoing port/control side geometrically.
    let unpinned = layout_system_nodes_with_pins(&doc, false, &BTreeMap::new());
    let api0 = unpinned.nodes.iter().find(|n| n.id == "api").unwrap();
    let child0 = unpinned.nodes.iter().find(|n| n.id == "api::c1").unwrap();
    let orders0 = unpinned.nodes.iter().find(|n| n.id == "orders").unwrap();
    let rel_child_dx = child0.x - api0.x;
    let rel_child_dy = child0.y - api0.y;

    let mut pins = BTreeMap::new();
    pins.insert("api".to_owned(), (orders0.x + 500.0, api0.y + 80.0));
    let pinned = layout_system_nodes_with_pins(&doc, false, &pins);
    let api1 = pinned.nodes.iter().find(|n| n.id == "api").unwrap();
    let child1 = pinned.nodes.iter().find(|n| n.id == "api::c1").unwrap();
    assert!((child1.x - api1.x - rel_child_dx).abs() < 0.01);
    assert!((child1.y - api1.y - rel_child_dy).abs() < 0.01);
    // After pinning api right of orders, wire from api should exit left edge (fx = api1.x).
    assert!(
        pinned.wires[0]
            .path_d
            .starts_with(&format!("M{:.1},", api1.x)),
        "post-pin wire should leave left side of api when moved right of orders: {}",
        pinned.wires[0].path_d
    );

    // Dense system canvas omits SVG edge/hit/label nodes and uses surface pin key + visual state.
    let mut d_nodes = Vec::with_capacity(185);
    for i in 0..185 {
        d_nodes
            .push(json!({"id": format!("svc-{i}"), "name": format!("svc-{i}"), "kind": "service"}));
    }
    let d_edges = json!([
        {"from": "svc-0", "to": "svc-1", "kind": "http"},
        {"from": "svc-1", "to": "svc-2", "kind": "event"}
    ]);
    let dense_doc = json!({"nodes": d_nodes, "edges": d_edges});
    let dense_state = json!({
        "project_id": "proj-sys",
        "focus": "svc-0",
        "tool": "path",
        "path_from": "svc-0",
        "path_to": "svc-2",
    });
    let change_key = sys_pin_key(CHANGES_SURFACE, &dense_state);
    graph_viewport::save_pin(&change_key, "svc-0", (310.0, 220.0));
    let dense_pins = graph_viewport::load_pins(&change_key);
    let dense_layout = layout_system_nodes_with_pins(&dense_doc, false, &dense_pins);
    let bundle =
        render_canvas_bundle_with_layout(CHANGES_SURFACE, &dense_doc, &dense_state, &dense_layout);
    assert!(bundle.is_dense);
    assert!(!bundle.html.contains("system-map-edge"));
    assert!(!bundle.html.contains("system-map-hit"));
    assert!(!bundle.html.contains("system-map-edge-label"));
    assert_eq!(bundle.node_htmls.len(), dense_layout.nodes.len());
    assert!(bundle.node_htmls.iter().all(|h| h.is_empty()));
    let svc5_idx = dense_layout
        .nodes
        .iter()
        .position(|n| n.id == "svc-5")
        .expect("svc-5 layout index");
    assert!(bundle
        .materialize_node_html(
            CHANGES_SURFACE,
            &dense_doc,
            &dense_state,
            &dense_layout,
            svc5_idx
        )
        .is_some_and(|h| h.contains("data-n=\"svc-5\"")));
    // Shares lowered dense threshold (32 nodes).
    let thresh_nodes: Vec<V> = (0..crate::graph_map::DENSE_GRAPH_NODE_THRESHOLD)
        .map(|i| json!({"id": format!("s-{i}"), "name": format!("s-{i}"), "kind": "service"}))
        .collect();
    let thresh_doc = json!({"nodes": thresh_nodes, "edges": []});
    let thresh_layout = layout_system_nodes_with_pins(&thresh_doc, false, &BTreeMap::new());
    let thresh_bundle =
        render_canvas_bundle_with_layout(CHANGES_SURFACE, &thresh_doc, &json!({}), &thresh_layout);
    assert!(thresh_bundle.is_dense);

    let mut scene =
        build_dense_scene_for_surface(CHANGES_SURFACE, &dense_doc, &dense_state, &dense_layout)
            .expect("dense scene");
    let n0 = scene.nodes.iter().find(|n| n.label == "svc-0").unwrap();
    let n1 = scene.nodes.iter().find(|n| n.label == "svc-1").unwrap();
    assert!(n0.pinned, "should load pin from CHANGES_SURFACE key");
    assert!(n0.selected);
    assert!(n1.lit);
    assert!(scene.edges[0].on && scene.edges[0].lit);

    // Patch DenseScene when selection changes to svc-2 without tool highlight.
    let vis2 = compute_sys_visual_state(&dense_doc, &json!({"focus": "svc-2"}));
    let node_ids: Vec<String> = dense_layout.nodes.iter().map(|n| n.id.clone()).collect();
    refresh_dense_scene_for_system(&mut scene, &vis2, &node_ids, &bundle.edge_endpoints);
    let n0_after = scene.nodes.iter().find(|n| n.label == "svc-0").unwrap();
    let n2_after = scene.nodes.iter().find(|n| n.label == "svc-2").unwrap();
    assert!(!n0_after.selected);
    assert!(n2_after.selected);
    assert!(scene.edges[1].on);
    assert!(!scene.edges[0].on);
}

#[test]
fn sys_unmatched_unfold_a11y_wiring_and_dense_display_names_work() {
    // Synthetic __unmatched__:* nodes must not unfold as frame children when spread is active.
    let doc = json!({
        "nodes": [
            {
                "id": "api",
                "name": "API Gateway",
                "kind": "service",
                "provides": [{"name": "v1.order.created", "kind": "http"}],
                "sources": ["src/api.rs"]
            },
            {
                "id": "__unmatched__:legacy-queue",
                "name": "legacy-queue",
                "kind": "service",
                "unmatched": true
            },
            {
                "id": "orders",
                "name": "Orders Core",
                "kind": "service"
            }
        ],
        "edges": [
            {"from": "api", "to": "orders", "kind": "http", "contracts": ["a", "b", "c", "d"]},
            {"from": "orders", "to": "api", "kind": "event"}
        ]
    });
    let spread_state = json!({"spread": ["api"], "focus": "api"});
    let unfolded = expand_unfolded_nodes(&doc, &spread_state);
    assert!(
        !unfolded["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| node_id(n).starts_with("__unmatched__:")
                && !m::text(n, "parent_service").is_empty()),
        "synthetic __unmatched__ nodes must remain independent"
    );

    // Post-pin geometric back-routing must not emit layer violation halos unless layers/against say so.
    let mut pins = BTreeMap::new();
    pins.insert("api".to_owned(), (800.0, 200.0));
    let layout = layout_system_nodes_with_pins(&doc, false, &pins);
    let bundle = render_canvas_bundle_with_layout(SERVICES_SURFACE, &doc, &json!({}), &layout);
    assert!(
        !bundle.html.contains("system-map-violation-halo"),
        "geometric wire.back must not fabricate architectural violation halos"
    );
    for hit_frag in bundle
        .html
        .split("class=\"graph-map-hit system-map-hit\"")
        .skip(1)
    {
        let tag_end = hit_frag.find('>').unwrap_or(hit_frag.len());
        assert!(
            !hit_frag[..tag_end].contains("tabindex=\"0\""),
            "SVG edge hit target must not be a focus trap: {}",
            &hit_frag[..tag_end]
        );
    }

    // Dense scene stores human display name on label while refresh resolves by node_ids.
    let mut d_nodes = Vec::with_capacity(182);
    for i in 0..182 {
        d_nodes.push(json!({
            "id": format!("svc-{i}"),
            "name": format!("Display Service {i}"),
            "kind": "service"
        }));
    }
    let dense_doc = json!({
        "nodes": d_nodes,
        "edges": [
            {"from": "svc-0", "to": "svc-1", "kind": "http"},
            {"from": "svc-1", "to": "svc-2", "kind": "event"}
        ]
    });
    let dense_layout = layout_system_nodes_with_pins(&dense_doc, false, &BTreeMap::new());
    let hidden_http_state = json!({"focus": "svc-0", "kinds_off": ["http"]});
    let scene = build_dense_scene_for_surface(
        SERVICES_SURFACE,
        &dense_doc,
        &hidden_http_state,
        &dense_layout,
    )
    .expect("dense scene");
    let n0 = scene
        .nodes
        .iter()
        .find(|n| n.label == "Display Service 0")
        .unwrap();
    assert!(
        n0.selected,
        "selection lookup must use node_ids even when display label differs"
    );
    assert!(
        scene.edges[0].cmds.is_empty(),
        "hidden http kind must suppress dense wire geometry"
    );
    assert!(
        !scene.edges[1].cmds.is_empty(),
        "visible event kind remains drawn"
    );

    // CameraSelectionHistory back/forward & pause wave button coverage.
    let mut nav = graph_viewport::CameraSelectionHistory::new(32);
    nav.push(sys_nav_entry_from(
        &json!({ "focus": "api" }),
        &Camera::default(),
    ));
    nav.push(sys_nav_entry_from(
        &json!({ "focus": "orders" }),
        &Camera {
            pan_x: 10.0,
            pan_y: 20.0,
            scale: 1.2,
            ..Camera::default()
        },
    ));
    assert!(nav.can_back());
    let (back_focus, ..) = parse_sys_nav_entry(nav.back().unwrap());
    assert_eq!(back_focus, "api");
    assert!(nav.can_forward());
    let (fwd_focus, ..) = parse_sys_nav_entry(nav.forward().unwrap());
    assert_eq!(fwd_focus, "orders");

    SYS_WAVE_PLAYING.with(|w| {
        w.borrow_mut().insert(CHANGES_SURFACE.id);
    });
    let scrubber = wave_scrubber_html(
        CHANGES_SURFACE,
        &BlastRadius {
            seeds: ["api".to_owned()].into_iter().collect(),
            direct_consumers: ["orders".to_owned()].into_iter().collect(),
            downstream_consumers: BTreeSet::new(),
            wave_by_service: [("api".to_owned(), 0), ("orders".to_owned(), 1)]
                .into_iter()
                .collect(),
            matched_contracts: BTreeSet::new(),
        },
        &json!({"wave": 0}),
    );
    SYS_WAVE_PLAYING.with(|w| {
        w.borrow_mut().remove(CHANGES_SURFACE.id);
    });
    assert!(scrubber.contains("system-map-wave-pause"));
}

#[test]
fn bridge_confidence_null_and_from_symbols_payload_and_unfold_more_and_guards() {
    // 1) Bridge confidence unverified when ratio is null or zero evidence + declared/discovered sites + from_symbols Map payload.
    let bridge = json!({
        "kind": "system_bridge",
        "from": "web",
        "to": "orders",
        "confidence": {
            "certain_refs": 0,
            "guessed_refs": 0,
            "declared_sites": 0,
            "discovered_sites": 0,
            "total": 0,
            "status": "unverified",
            "ratio": null
        },
        "consumer": {
            "service": "web",
            "graph": true,
            "sites": [{
                "path": "src/OrdersClient.ts",
                "found": true,
                "from_symbols": ["OrdersClient::place", "OrdersClient::cancel"],
                "contracts": ["orders-api"]
            }],
            "more": 0
        },
        "provider": {
            "service": "orders",
            "graph": true,
            "sites": [],
            "more": 0,
            "cut": false
        }
    });
    let html = bridge_html(SERVICES_SURFACE, "web", "orders", &bridge);
    assert!(html.contains("Unverified (0 evidence)"), "{html}");
    assert!(
        html.contains("0 declared sites, 0 discovered sites"),
        "{html}"
    );
    let payloads = crate::test_support::payloads(&html, "inspect-system-file-graph");
    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0]["symbol"], json!("OrdersClient::place"));
    assert_eq!(
        payloads[0]["from_symbols"],
        json!(["OrdersClient::place", "OrdersClient::cancel"])
    );

    let mut verified_bridge = bridge;
    verified_bridge["confidence"] = json!({
        "certain_refs": 2,
        "guessed_refs": 1,
        "declared_sites": 3,
        "discovered_sites": 2,
        "ratio": 0.625
    });
    let verified_html = bridge_html(SERVICES_SURFACE, "web", "orders", &verified_bridge);
    assert!(verified_html.contains("63%"));
    assert!(verified_html
        .contains("2 certain refs, 1 guessed refs, 3 declared sites, 2 discovered sites"));

    // 2) expand_unfolded_nodes avoids early exit when another service already has detail nodes,
    //    and emits visible `+N more` badge node when capabilities/contracts/sources exceed caps.
    let multi_doc = json!({
        "nodes": [
            {
                "id": "api",
                "name": "API",
                "kind": "service",
                "capabilities": ["c1", "c2", "c3", "c4", "c5", "c6", "c7", "c8"],
                "provides": (0..10).map(|i| json!({"name": format!("rpc.{i}"), "kind": "http"})).collect::<Vec<_>>(),
                "shared_sources": ["a.md", "b.md", "c.md", "d.md", "e.md", "f.md", "g.md"]
            },
            {
                "id": "orders",
                "name": "Orders",
                "kind": "service"
            },
            {
                "id": "orders::contract::c1",
                "parent_service": "orders",
                "service": "orders",
                "kind": "contract",
                "name": "c1"
            }
        ],
        "edges": []
    });
    let unfolded = expand_unfolded_nodes(&multi_doc, &json!({ "whole": true }));
    let u_nodes = unfolded["nodes"].as_array().unwrap();
    assert!(
        u_nodes
            .iter()
            .any(|n| n["parent_service"] == "api" && n["kind"] == "contract"),
        "must unfold api contracts even though orders already had a contract node"
    );
    let more_node = u_nodes
        .iter()
        .find(|n| n["id"] == "api::more")
        .expect("+N more badge node for truncated capabilities/contracts/sources");
    // 2 extra caps + 2 extra contracts + 1 extra source = 5 more
    assert_eq!(more_node["name"], "+5 more");
    assert_eq!(more_node["more_count"], 5);
    let canvas_html = render_canvas_html_with_layout(
        SERVICES_SURFACE,
        &unfolded,
        &json!({ "whole": true }),
        &layout_system_nodes(&unfolded, false),
    );
    assert!(canvas_html.contains("+5 more"), "{canvas_html}");

    // 3) pointer guard rejects interactive elements and empty-state buttons/boxes.
    assert!(should_ignore_canvas_pointer(
        "pointerdown",
        true,
        false,
        true,
        false,
        true
    ));
    assert!(should_ignore_canvas_pointer(
        "pointerdown",
        true,
        false,
        false,
        true,
        false
    ));
    assert!(should_ignore_canvas_pointer(
        "dblclick", true, false, false, true, false
    ));
    assert!(should_ignore_canvas_pointer(
        "dblclick", true, false, true, false, false
    ));
    assert!(!should_ignore_canvas_pointer(
        "dblclick", true, false, true, false, true
    ));
}

#[test]
fn system_map_dense_scene_populates_meta_adjacency_and_explain_targets() {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for i in 0..185 {
        nodes.push(json!({
            "id": format!("svc-{i}"),
            "name": format!("Service {i}"),
            "kind": "service",
        }));
    }
    edges.push(json!({
        "from": "svc-0",
        "to": "svc-1",
        "kinds": ["http"],
    }));
    let doc = json!({ "nodes": nodes, "edges": edges });
    let state = json!({});
    let layout = layout_system_nodes(&doc, false);
    let (scene, mut meta) = super::render::build_dense_scene_and_meta_for_surface(
        SERVICES_SURFACE,
        &doc,
        &state,
        &layout,
    )
    .expect("dense scene + meta");
    meta.rebuild_caches(&scene);
    let idx_0 = layout.nodes.iter().position(|n| n.id == "svc-0").unwrap();
    let idx_1 = layout.nodes.iter().position(|n| n.id == "svc-1").unwrap();
    assert_eq!(
        meta.nodes[&idx_0].explain_payload["service"],
        json!("svc-0")
    );
    assert_eq!(
        meta.nodes[&idx_1].explain_payload["service"],
        json!("svc-1")
    );
    assert!(meta.edges.contains_key(&0));
    assert_eq!(meta.edges[&0].explain_kind, "link");
    assert_eq!(meta.edges[&0].explain_payload["from"], json!("svc-0"));
    assert_eq!(meta.edges[&0].explain_payload["to"], json!("svc-1"));
    let adj0 = meta.node_adjacency.get(&idx_0).expect("svc-0 adjacency");
    assert_eq!(adj0, &vec![(0, idx_1)]);
}

#[test]
fn system_map_dense_scene_populates_tier_band_and_service_frame_containers() {
    let mut nodes = vec![
        json!({ "id": "svc-0", "name": "Gateway", "kind": "service", "layer": "Edge" }),
        json!({ "id": "svc-0::ep0", "name": "GET /orders", "parent_service": "svc-0", "kind": "contract" }),
        json!({ "id": "svc-1", "name": "Orders", "kind": "service", "layer": "Core" }),
    ];
    for i in 2..184 {
        nodes.push(json!({
            "id": format!("svc-{i}"),
            "name": format!("Service {i}"),
            "kind": "service",
            "layer": if i % 2 == 0 { "Edge" } else { "Core" },
        }));
    }
    let edges = vec![json!({
        "from": "svc-0::ep0",
        "to": "svc-1",
        "kinds": ["http"],
        "provenance": "code_declared",
        "against": true,
    })];
    let doc = json!({ "nodes": nodes, "edges": edges });
    let state = json!({ "layers": true });
    let layout = layout_system_nodes(&doc, true);
    assert!(!layout.tier_bands.is_empty());
    assert!(!layout.frames.is_empty());

    let bundle =
        super::render::render_canvas_bundle_with_layout(SERVICES_SURFACE, &doc, &state, &layout);
    assert!(bundle.is_dense);
    assert!(!bundle.html.contains("system-tier-band"));
    assert!(!bundle.html.contains("system-service-frame"));

    let (_scene, meta) = super::render::build_dense_scene_and_meta_for_surface(
        SERVICES_SURFACE,
        &doc,
        &state,
        &layout,
    )
    .expect("dense containers");
    let tier_count = meta
        .containers
        .iter()
        .filter(|c| c.kind == graph_viewport::canvas2d::DenseContainerKind::TierBand)
        .count();
    let frame_count = meta
        .containers
        .iter()
        .filter(|c| c.kind == graph_viewport::canvas2d::DenseContainerKind::ServiceFrame)
        .count();
    assert_eq!(tier_count, layout.tier_bands.len());
    assert_eq!(frame_count, layout.frames.len());
    let ep_idx = layout
        .nodes
        .iter()
        .position(|n| n.id == "svc-0::ep0")
        .unwrap();
    assert_eq!(
        meta.nodes[&ep_idx].explain_payload["services"],
        json!(["svc-0"])
    );
    assert_eq!(
        meta.edges[&0].explain_payload["services"],
        json!(["svc-0", "svc-1"])
    );
    assert_eq!(meta.edges[&0].explain_payload["against"], json!(true));
}

#[test]
fn system_map_layout_routes_edges_around_intermediate_boxes() {
    // Chain a -> b -> c puts a (col 0), b (col 1), c (col 2), plus skip edge a -> c.
    // Same-column stack s0 -> s1 -> s2 plus skip edge s0 -> s2 checks gutter detour.
    let doc = json!({
        "nodes": [
            { "id": "a", "name": "Gateway" },
            { "id": "b", "name": "Auth" },
            { "id": "c", "name": "Billing" },
            { "id": "s0", "name": "Worker 0" },
            { "id": "s1", "name": "Worker 1" },
            { "id": "s2", "name": "Worker 2" }
        ],
        "edges": [
            { "from": "a", "to": "b", "kinds": ["http"] },
            { "from": "b", "to": "c", "kinds": ["http"] },
            { "from": "a", "to": "c", "kinds": ["rpc"] },
            { "from": "a", "to": "s0", "kinds": ["event"] },
            { "from": "a", "to": "s1", "kinds": ["event"] },
            { "from": "a", "to": "s2", "kinds": ["event"] },
            { "from": "s0", "to": "s2", "kinds": ["rpc"] }
        ]
    });
    let pins = BTreeMap::from([
        ("a".to_owned(), (40.0, 120.0)),
        ("b".to_owned(), (320.0, 120.0)),
        ("c".to_owned(), (620.0, 120.0)),
        ("s0".to_owned(), (40.0, 260.0)),
        ("s1".to_owned(), (40.0, 360.0)),
        ("s2".to_owned(), (40.0, 460.0)),
    ]);
    let layout = layout_system_nodes_with_pins(&doc, false, &pins);

    let check_wire_avoids = |edge_idx: usize, obstacle_id: &str| {
        let wire = layout
            .wires
            .iter()
            .find(|w| w.edge_index == edge_idx)
            .expect("wire present");
        let obs = layout
            .nodes
            .iter()
            .find(|n| n.id == obstacle_id)
            .expect("obstacle present");
        let spec = graph_viewport::canvas2d::DenseEdgeSpec::from_svg_path(
            edge_idx,
            &wire.path_d,
            1,
            false,
            false,
            "rpc".into(),
            false,
            false,
            false,
            false,
        );
        // Sample points from cmds directly via dense polyline sampling.
        let mut pts = Vec::new();
        let mut cur = (0.0, 0.0);
        for cmd in &spec.cmds {
            match *cmd {
                graph_viewport::canvas2d::DensePathCmd::Move(x, y) => cur = (x, y),
                graph_viewport::canvas2d::DensePathCmd::Line(x, y) => {
                    for s in 1..=8 {
                        let t = (s as f64) / 8.0;
                        pts.push((cur.0 + (x - cur.0) * t, cur.1 + (y - cur.1) * t));
                    }
                    cur = (x, y);
                }
                graph_viewport::canvas2d::DensePathCmd::Curve { c1, c2, to } => {
                    for s in 1..=12 {
                        let t = (s as f64) / 12.0;
                        let u = 1.0 - t;
                        pts.push((
                            u * u * u * cur.0
                                + 3.0 * u * u * t * c1.0
                                + 3.0 * u * t * t * c2.0
                                + t * t * t * to.0,
                            u * u * u * cur.1
                                + 3.0 * u * u * t * c1.1
                                + 3.0 * u * t * t * c2.1
                                + t * t * t * to.1,
                        ));
                    }
                    cur = to;
                }
            }
        }
        assert!(!pts.is_empty());
        for (px, py) in pts {
            assert!(
                !(px >= obs.x && px <= obs.x + obs.w && py >= obs.y && py <= obs.y + obs.h),
                "wire {edge_idx} point ({px:.1},{py:.1}) cut through {obstacle_id} [{:.1}..{:.1}, {:.1}..{:.1}] path={}",
                obs.x,
                obs.x + obs.w,
                obs.y,
                obs.y + obs.h,
                wire.path_d
            );
        }
    };

    check_wire_avoids(2, "b");
    check_wire_avoids(6, "s1");
}
