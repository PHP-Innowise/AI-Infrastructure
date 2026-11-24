use super::*;

#[test]
fn render_side_memory_bridge_surfaces_links_and_rename_warnings() {
    let links = json!({
        "path": "src/Auth/TokenService.php",
        "brain_records": [{"id": "11111111-2222-4333-8444-555555555555", "external_id": "finding-7", "type": "finding", "title": "Tokens <expire>"}],
        "memory_chunks": [{"id": "MEM-20261117-aabbccdd", "title": "TTL rule", "auto_promoted": true}],
        "omnigraph_atoms": [{"capsule_line": "- [project-brain decision · 11111111] Tokens expire (from `knowledge/canonical`)"}],
        "hygiene_warnings": [{"kind": "cited-file-renamed", "text": "This file was src/Auth/Tokens.php; the memory that cites the old name may need its citation updated.", "recommended_action": "review"}]
    });
    let html = render_side_memory_bridge_html("proj-1", &links);
    assert!(html.contains("data-kind=\"cited-file-renamed\" role=\"note\">Review: This file was src/Auth/Tokens.php"), "{html}");
    assert!(
        html.contains("data-action=\"graph-map-open-memory\""),
        "{html}"
    );
    assert!(
        html.contains("&quot;target_id&quot;:&quot;finding-7&quot;"),
        "{html}"
    );
    assert!(html.contains("Tokens &lt;expire&gt;"), "{html}");
    assert!(html.contains("MEM-20261117-aabbccdd</span> TTL rule <span class=\"graph-memory-badge\">auto-promoted</span>"), "{html}");
    assert!(
        html.contains("[project-brain decision · 11111111] Tokens expire"),
        "{html}"
    );
    assert!(!html.contains('#') || !html.contains("color:#"), "{html}");
    // An empty answer says so; a failed one says why and draws nothing else.
    let empty = render_side_memory_bridge_html(
        "proj-1",
        &json!({"brain_records": [], "memory_chunks": [], "omnigraph_atoms": []}),
    );
    assert!(empty.contains("No Project Brain record"), "{empty}");
    let failed = render_side_memory_bridge_html("proj-1", &json!({"error": "Project not found"}));
    assert!(
        failed.contains("Linked memory is unavailable: Project not found"),
        "{failed}"
    );
    assert!(!failed.contains("<button"), "{failed}");
}

#[test]
fn a_selected_file_gets_a_placeholder_until_its_links_arrive() {
    let html = memory_links_section("proj-1", "src/Auth/TokenService.php");
    assert!(
        html.contains("class=\"graph-memory-links\" data-path=\"src/Auth/TokenService.php\""),
        "{html}"
    );
    assert!(html.contains("Looking for Project Brain records"), "{html}");
    m::set(
        MEMORY_LINKS,
        json!({"proj-1\u{1f}src/Auth/TokenService.php": {"brain_records": [], "memory_chunks": [], "omnigraph_atoms": []}}),
    );
    let cached = memory_links_section("proj-1", "src/Auth/TokenService.php");
    assert!(cached.contains("No Project Brain record"), "{cached}");
    m::take(MEMORY_LINKS);
}

#[test]
fn contradictions_and_named_declarations_are_shown_for_review() {
    let links = json!({
        "brain_records": [],
        "memory_chunks": [],
        "omnigraph_atoms": [{
            "capsule_line": "- [unverified reference — verify against code & Brain] [finding] Replay is fine (from `main`)",
            "symbols": ["TokenGuard::check", "<b>"]
        }],
        "hygiene_warnings": [],
        "contradiction_warnings": [{
            "kind": "atom-contradiction",
            "text": "\"Replay is fine\" contradicts \"Tokens <expire>\"; check both against the code before relying on either.",
            "open": [
                {"scope": "brain", "target_id": "11111111-2222-4333-8444-555555555555", "title": "Tokens <expire>"},
                {"scope": "shell", "target_id": "rm -rf"},
                {"scope": "memory", "target_id": ""}
            ]
        }]
    });
    let html = render_side_memory_bridge_html("proj-1", &links);
    assert!(
        html.contains("<p class=\"graph-memory-warning\" data-kind=\"atom-contradiction\" role=\"note\">Review: &quot;Replay is fine&quot; contradicts &quot;Tokens &lt;expire&gt;&quot;; check both against the code before relying on either. <button type=\"button\" class=\"button compact\" data-action=\"graph-map-open-memory\""),
        "{html}"
    );
    assert!(
        html.contains(">Open Tokens &lt;expire&gt;</button></p>"),
        "{html}"
    );
    assert!(
        html.contains("&quot;target_id&quot;:&quot;11111111-2222-4333-8444-555555555555&quot;"),
        "{html}"
    );
    // Only a Knowledge record or chunk opens; nothing else is offered.
    assert_eq!(
        html.matches("data-action=\"graph-map-open-memory\"")
            .count(),
        1,
        "{html}"
    );
    assert!(!html.contains("rm -rf"), "{html}");
    assert!(
        html.contains(
            "<span class=\"graph-memory-badge\">names TokenGuard::check, &lt;b&gt;</span>"
        ),
        "{html}"
    );
}

#[test]
fn a_commit_banner_reviews_memory_citing_the_names_it_renamed() {
    let view = json!({
        "project_id": "proj-1",
        "chosen": {"kind": "commit", "head": {"commit": "aaaa1111bbbb2222"}},
        "commit_info": {"commit": "aaaa1111bbbb2222", "short": "aaaa111", "subject": "Rename the guard", "parents": []}
    });
    let changes = json!({
        "renamed_files": 1,
        "files": [{"path": "src/Auth/Guard.php", "previous_path": "src/Auth/TokenGuard.php", "status": "renamed"}]
    });
    // Until the answer is in, the banner says it is checking and no
    // dock count is recorded.
    let asking = render_commit_banner_for_view(&view, Some(&changes));
    assert!(
        asking.contains("<div class=\"graph-commit-memory\" data-commit=\"aaaa1111bbbb2222\" aria-live=\"polite\"><p class=\"knowledge-note\">Checking the memory that cites 1 renamed file…</p></div>"),
        "{asking}"
    );
    assert!(m::data(COMMIT_MEMORY_COUNTS)["aaaa1111bbbb2222"].is_null());
    m::set(
        MEMORY_LINKS,
        json!({"proj-1\u{1f}src/Auth/Guard.php\u{1f}src/Auth/TokenGuard.php": {
            "hygiene_warnings": [{"kind": "cited-file-renamed", "linked": ["MEM-20261117-aabbccdd"], "recommended_action": "review", "text": "This file was src/Auth/TokenGuard.php; the memory that cites the old name may need its citation updated."}]
        }}),
    );
    let answered = render_commit_banner_for_view(&view, Some(&changes));
    assert!(
        answered.contains(
            "Review: <code>src/Auth/TokenGuard.php</code> → <code>src/Auth/Guard.php</code>"
        ),
        "{answered}"
    );
    // The commit's dock row now shows the count; another row shows none.
    assert_eq!(m::data(COMMIT_MEMORY_COUNTS)["aaaa1111bbbb2222"], 1);
    let dock = json!({"rows": [
        {"commit": "aaaa1111bbbb2222", "short": "aaaa111", "subject": "Rename the guard", "parents": ["cccc3333dddd4444"]},
        {"commit": "cccc3333dddd4444", "short": "cccc333", "subject": "Base", "parents": []}
    ]});
    let rows = render_commit_dock_list(&dock, "", 0);
    assert_eq!(rows.matches("1 to review").count(), 1, "{rows}");
    assert!(m::data(COMMIT_MEMORY_ERRORS)["aaaa1111bbbb2222"].is_null());

    // When the batch memory check fails, clear any cached count and mark the
    // dock row + banner with an error indicator instead of caching 0.
    m::set(
        MEMORY_LINKS,
        json!({"proj-1\u{1f}src/Auth/Guard.php\u{1f}src/Auth/TokenGuard.php": {
            "error": "network down"
        }}),
    );
    let failed_banner = render_commit_banner_for_view(&view, Some(&changes));
    assert!(
        failed_banner
            .contains("Could not check the memory citing src/Auth/TokenGuard.php: network down"),
        "{failed_banner}"
    );
    assert!(m::data(COMMIT_MEMORY_COUNTS)["aaaa1111bbbb2222"].is_null());
    assert_eq!(m::data(COMMIT_MEMORY_ERRORS)["aaaa1111bbbb2222"], true);
    let failed_rows = render_commit_dock_list(&dock, "", 0);
    assert_eq!(
        failed_rows.matches("check failed").count(),
        1,
        "{failed_rows}"
    );
    assert!(
        failed_rows.contains("data-kind=\"memory-error\""),
        "{failed_rows}"
    );
    assert!(!failed_rows.contains("to review"), "{failed_rows}");

    // A commit that renamed nothing asks nothing and adds nothing.
    let plain = render_commit_banner_for_view(&view, Some(&json!({"modified_files": 1})));
    assert!(!plain.contains("graph-commit-memory"), "{plain}");
    m::take(MEMORY_LINKS);
    m::take(COMMIT_MEMORY_COUNTS);
    m::take(COMMIT_MEMORY_ERRORS);
}

#[test]
fn batched_memory_links_payload_and_response_populate_per_file_cache() {
    let missing = vec![
        (
            "src/Auth/Guard.php".to_owned(),
            "src/Auth/TokenGuard.php".to_owned(),
        ),
        (
            "src/Http/Middleware.php".to_owned(),
            "src/Http/LegacyMiddleware.php".to_owned(),
        ),
    ];
    let payload = build_memory_links_batch_payload(&missing);
    assert_eq!(
        payload,
        json!({
            "files": [
                {"path": "src/Auth/Guard.php", "previous_path": "src/Auth/TokenGuard.php"},
                {"path": "src/Http/Middleware.php", "previous_path": "src/Http/LegacyMiddleware.php"}
            ]
        })
    );

    let mut cache = json!({});
    let resp = json!({
        "project_id": "proj-1",
        "items": [
            {
                "path": "src/Auth/Guard.php",
                "previous_path": "src/Auth/TokenGuard.php",
                "hygiene_warnings": [{"kind": "cited-file-renamed", "linked": ["MEM-1"]}]
            },
            {
                "path": "src/Http/Middleware.php",
                "previous_path": "src/Http/LegacyMiddleware.php",
                "hygiene_warnings": []
            }
        ]
    });
    apply_memory_links_batch_response(&mut cache, "proj-1", &missing, Ok(&resp), 1234.0);
    let k0 = rename_memory_key("proj-1", "src/Auth/Guard.php", "src/Auth/TokenGuard.php");
    let k1 = rename_memory_key(
        "proj-1",
        "src/Http/Middleware.php",
        "src/Http/LegacyMiddleware.php",
    );
    assert_eq!(cache[&k0]["hygiene_warnings"].as_array().unwrap().len(), 1);
    assert_eq!(cache[&k0]["at"], 1234.0);
    assert_eq!(cache[&k1]["hygiene_warnings"], json!([]));
    assert_eq!(cache[&k1]["at"], 1234.0);

    let mut err_cache = json!({});
    apply_memory_links_batch_response(
        &mut err_cache,
        "proj-1",
        &missing,
        Err("network down"),
        5678.0,
    );
    assert_eq!(err_cache[&k0]["error"], "network down");
    assert_eq!(err_cache[&k1]["error"], "network down");
}

#[test]
fn cached_surface_plan_matches_uncached_focus_bbox() {
    let view = json!({
        "project_id": "proj-1",
        "frames": [
            {"path": "src/Auth", "name": "Auth", "holder": ""}
        ],
        "nodes": [
            {"path": "src/Auth/Guard.php", "name": "Guard.php", "holder": "src/Auth"},
            {"path": "src/Auth/Token.php", "name": "Token.php", "holder": "src/Auth"}
        ],
        "edges": [
            {"from": "src/Auth/Guard.php", "to": "src/Auth/Token.php", "links": 2, "kind": "uses"}
        ]
    });
    let rendered = canvas_with_overlays(&view, "session-graph", CanvasOverlayContext::default());
    assert_eq!(rendered.plan_nodes.len(), 2);
    assert_eq!(rendered.plan_frames.len(), 1);

    let cached = CachedSurfacePlan::from_rendered(&view, &rendered, None);
    let probe = LiveFocus {
        target_paths: vec!["src/Auth/Guard.php".to_owned()],
        target_nodes: vec!["src/Auth/Guard.php".to_owned()],
        ..LiveFocus::default()
    };
    let fast_bbox = resolve_focus_bbox_with_cached_plan(&view, Some(&cached), &probe);
    let fallback_bbox = resolve_focus_bbox_with_cached_plan(&view, None, &probe);
    assert!(fast_bbox.is_some());
    assert_eq!(fast_bbox, fallback_bbox);

    // Stale cache with mismatched node count falls back instead of using old coordinates.
    let mut modified_view = view.clone();
    modified_view["nodes"].as_array_mut().unwrap().push(json!({
        "path": "src/Auth/Policy.php",
        "name": "Policy.php",
        "holder": "src/Auth"
    }));
    assert_ne!(surface_view_signature(&modified_view), cached.signature);
    let recomputed = resolve_focus_bbox_with_cached_plan(&modified_view, Some(&cached), &probe);
    assert!(recomputed.is_some());
}

#[test]
fn declutters_secondary_overlays_into_badge_strip_preserving_data_attrs() {
    let view = json!({
        "nodes": [
            {
                "path": "src/Checkout/Cart.php",
                "name": "Cart.php",
                "folder": false,
                "out": 2,
                "in": 3,
                "changed": 1,
                "status": "M",
                "visibility": "public",
                "patterns": 2,
                "opened": 3,
                "suspect": {"rank": 1}
            },
            {
                "path": "src/Checkout/Tax.php",
                "name": "Tax.php",
                "folder": false,
                "out": 0,
                "in": 1
            }
        ],
        "edges": [
            {"from": "src/Checkout/Cart.php", "to": "src/Checkout/Tax.php", "links": 2},
            {"from": "src/Checkout/Tax.php", "to": "src/Checkout/Cart.php", "links": 1}
        ]
    });
    let rendered = canvas_with_overlays(&view, "session-graph", CanvasOverlayContext::default());
    assert!(rendered.html.contains("data-has-badges=\"true\""));
    assert!(rendered.html.contains("data-badge-count=\"5\""));
    assert!(rendered.html.contains("data-visibility=\"public\""));
    assert!(rendered.html.contains(
        "<span class=\"graph-map-node-name\">Cart.php</span><span class=\"graph-map-node-status\" data-status=\"modified\" aria-hidden=\"true\">M</span>"
    ));
    assert!(rendered.html.contains(
        "<span class=\"graph-map-node-badges\" aria-hidden=\"true\"><span class=\"graph-vis-pill\""
    ));
    assert!(rendered
        .html
        .contains("<span class=\"graph-map-node-cycle\" aria-hidden=\"true\">↻</span>"));
    assert!(rendered
        .html
        .contains("<span class=\"graph-map-node-pattern\" aria-hidden=\"true\">◆2</span>"));
    assert!(rendered
        .html
        .contains("<span class=\"graph-map-node-opened\" aria-hidden=\"true\">◉3</span>"));
    assert!(rendered.html.contains(
        "<span class=\"graph-map-node-suspect\" aria-hidden=\"true\">#1</span></span></button>"
    ));

    let mut no_suspect = view;
    no_suspect["nodes"][0]
        .as_object_mut()
        .unwrap()
        .remove("suspect");
    let clean = canvas_with_overlays(
        &no_suspect,
        "session-graph",
        CanvasOverlayContext::default(),
    );
    crate::test_support::assert_no_inline_hex_colors(&clean.html);
    crate::test_support::assert_no_inline_hex_colors(&clean.radar);
}

#[test]
fn side_only_click_actions_skip_canvas_rerender_and_selection_patch_preserves_structure() {
    let mut st = fresh("");
    for (action, payload) in [
        ("graph-side-tab", json!({"tab": "changes"})),
        ("graph-changes-filter", json!({"filter": "modified"})),
        ("graph-map-member-filter", json!({"filter": "exposed"})),
    ] {
        let outcome = apply_click_action(&mut st, action, &payload, MODAL_SURFACE, false, false);
        assert!(outcome.handled, "{action}");
        assert!(outcome.rerender_side, "{action}");
        assert!(outcome.rerender, "{action}");
        assert!(!outcome.reload, "{action}");
    }

    let view = json!({
        "frames": [{"path": "src/Auth", "name": "Auth", "holder": ""}],
        "nodes": [
            {"path": "src/Auth/Guard.php", "name": "Guard.php", "holder": "src/Auth"},
            {"path": "src/Auth/Token.php", "name": "Token.php", "holder": "src/Auth"},
            {"path": "src/Auth/Store.php", "name": "Store.php", "holder": "src/Auth"}
        ],
        "edges": [
            {"from": "src/Auth/Guard.php", "to": "src/Auth/Token.php", "links": 1},
            {"from": "src/Auth/Token.php", "to": "src/Auth/Store.php", "links": 1}
        ]
    });
    let base_ctx = CanvasOverlayContext::default();
    let base_digest = canvas_structure_digest(&view, &base_ctx);
    let rendered = canvas_with_overlays(&view, "graph-map", base_ctx.clone());
    let pairs = pairs_for_view(&view, &base_ctx);

    // Selection, cone reach, and live target highlight keep the structure digest intact.
    let mut focused_view = view.clone();
    focused_view["focus"] = json!({"path": "src/Auth/Guard.php"});
    let live = LiveFocus {
        target_paths: vec!["src/Auth/Token.php".to_owned()],
        target_nodes: vec!["src/Auth/Token.php".to_owned()],
        ..LiveFocus::default()
    };
    let focused_ctx = CanvasOverlayContext {
        live_focus: Some(&live),
        cone: (2, 2),
        ..Default::default()
    };
    assert_eq!(
        base_digest,
        canvas_structure_digest(&focused_view, &focused_ctx)
    );

    let patch = compute_canvas_selection_patch(
        &focused_view,
        &rendered.plan_nodes,
        &rendered.plan_frames,
        &pairs,
        &focused_ctx,
    );
    assert!(patch.nodes[0].selected);
    assert!(!patch.nodes[0].dim);
    assert!(!patch.nodes[1].dim);
    assert!(patch.nodes[1].focus_target);
    assert_eq!(patch.nodes[0].radar_state, "selected");
    assert_eq!(patch.edges_by_pair[&(0, 1)].edge_state, "on");
    assert!(patch.focus.bbox.is_some());

    // Switching link kinds off changes SVG defs/edges, invalidating structural cache.
    let mut hidden_calls = base_ctx;
    hidden_calls.links_off[0] = true;
    assert_ne!(base_digest, canvas_structure_digest(&view, &hidden_calls));
}

#[test]
fn large_and_dense_scale_thresholds_elide_details_and_cull_viewport() {
    assert_eq!(LARGE_GRAPH_NODE_THRESHOLD, 24);
    assert_eq!(LARGE_GRAPH_EDGE_THRESHOLD, 60);
    assert_eq!(DENSE_GRAPH_NODE_THRESHOLD, 32);
    assert_eq!(DENSE_GRAPH_EDGE_THRESHOLD, 80);
    assert_eq!(graph_scale_tier(10, 20), GraphScaleTier::Normal);
    assert_eq!(
        graph_scale_tier(LARGE_GRAPH_NODE_THRESHOLD, 10),
        GraphScaleTier::Large
    );
    assert_eq!(
        graph_scale_tier(DENSE_GRAPH_NODE_THRESHOLD, 10),
        GraphScaleTier::Dense
    );
    assert_eq!(
        graph_scale_tier(10, DENSE_GRAPH_EDGE_THRESHOLD),
        GraphScaleTier::Dense
    );

    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for i in 0..30usize {
        nodes.push(json!({
            "path": format!("src/File{i}.php"),
            "name": format!("File{i}.php"),
            "out": 1,
            "in": 1
        }));
        if i + 1 < 30 {
            edges.push(json!({
                "from": format!("src/File{i}.php"),
                "to": format!("src/File{}.php", i + 1),
                "links": 1
            }));
        }
    }
    let view = json!({
        "nodes": nodes,
        "edges": edges,
    });
    let rendered = canvas_with_overlays(&view, "graph-map", CanvasOverlayContext::default());
    assert!(rendered.html.contains("data-scale-tier=\"large\""));
    assert!(!rendered.html.contains("graph-map-edge-chevron"));

    // >= 32 nodes (including default 40-node paginated File/Symbol views) enters Dense mode with Canvas2D scene and lazy DOM html slots.
    let mut dense_nodes = Vec::new();
    for i in 0..40usize {
        dense_nodes.push(json!({
            "path": format!("src/Dense{i}.php"),
            "name": format!("Dense{i}.php"),
            "out": 1,
            "in": 1
        }));
    }
    let dense_view = json!({ "nodes": dense_nodes, "edges": [] });
    let dense_rendered =
        canvas_with_overlays(&dense_view, "graph-map", CanvasOverlayContext::default());
    assert!(dense_rendered.html.contains("data-scale-tier=\"dense\""));
    assert!(dense_rendered.dense_scene.is_some());
    assert!(dense_rendered.dense_node_htmls.iter().all(|h| h.is_empty()));

    let bounds = visible_viewport_bounds((0.0, 0.0), 1.0, (400.0, 300.0), 20.0).unwrap();
    let visible = count_nodes_in_viewport(&rendered.plan_nodes, bounds);
    assert!(visible < rendered.plan_nodes.len());
}

#[test]
fn modal_and_embedded_surfaces_share_graph_viewport_camera_math() {
    assert_eq!(
        pan_surface_for_canvas("graph-map-canvas").map(|s| s.name),
        Some("modal")
    );
    assert_eq!(MODAL_SURFACE.camera_key, "graph-map-camera");
    assert_eq!(MODAL_SURFACE.drag_key, "graph-map-drag");

    assert_eq!(
        parse_style_px("left:340px;top:120px;width:164px", "left"),
        Some(340.0)
    );
    assert_eq!(
        parse_style_px("left: 340px; top: 120.5px", "top"),
        Some(120.5)
    );
    assert_eq!(parse_style_px("width:164px", "left"), None);

    let fit_cam = graph_viewport::fit_proportional(
        (1200.0, 800.0),
        (800.0, 600.0),
        0.94,
        CAMERA_MIN,
        1.0,
        12.0,
    );
    assert_eq!(
        camera_for_fit((1200.0, 800.0), (800.0, 600.0)),
        (fit_cam.scale, fit_cam.tx, fit_cam.ty)
    );

    let mut modal_cam = Camera::default();
    modal_cam.apply_viewport_cam(fit_cam);
    let mut drag = graph_viewport::DragPan::default();
    drag.begin(7, (100.0, 100.0), modal_cam.viewport_cam());
    assert_eq!(
        drag.advance(7, (102.0, 102.0), graph_viewport::DEFAULT_DRAG_THRESHOLD_PX),
        graph_viewport::DragMove::BelowThreshold
    );
    match drag.advance(7, (140.0, 125.0), graph_viewport::DEFAULT_DRAG_THRESHOLD_PX) {
        graph_viewport::DragMove::StartedPan { tx, ty } => {
            assert_eq!(
                (tx, ty),
                ((fit_cam.tx + 40.0).round(), (fit_cam.ty + 25.0).round())
            );
        }
        other => panic!("expected pan start, got {other:?}"),
    }
    let end = drag.end(Some(7), false).expect("drag end");
    assert!(end.suppress_click);

    let details_tabs = side_tabs_html("details", 3, "session");
    assert!(details_tabs.contains("aria-selected=\"true\" tabindex=\"0\""));
    assert!(details_tabs.contains("aria-selected=\"false\" tabindex=\"-1\""));
    let changes_tabs = side_tabs_html("changes", 3, "modal");
    assert!(changes_tabs.contains(
        "aria-selected=\"true\" tabindex=\"0\" data-action=\"graph-side-tab\" data-tab=\"changes\""
    ));
    assert!(changes_tabs.contains("aria-selected=\"false\" tabindex=\"-1\" data-action=\"graph-side-tab\" data-tab=\"details\""));
}

#[test]
fn phase2c_coverage_badges_and_native_ir_labels_render_cleanly_without_hex_colors() {
    let view = json!({
        "mode": "changes",
        "scope": "",
        "backend": "native-tree-sitter-ir",
        "built": {"commit": "deadbeef", "tool": "native-tree-sitter-ir"},
        "chosen": {"kind": "work", "name": "Uncommitted changes", "target": "work"},
        "change": {"files": 2, "graphed": 2, "here": 2, "affected": 0, "needed": 0, "removed": 0},
        "focus": {"path": "src/cart.php", "uses": [], "used_by": []},
        "nodes": [
            {"path": "src/cart.php", "name": "cart.php", "folder": false, "changed": 1, "status": "M",
             "suspect": {"rank": 1, "score": 1.0}},
            {"path": "src/price.php", "name": "price.php", "folder": false, "changed": 1, "status": "M"}
        ],
        "edges": [],
        "test_reach": [
            {
                "path": "src/cart.php",
                "state": "ready",
                "total": 2,
                "coverage": {"format": "lcov", "covered_lines": 34, "total_lines": 40, "rate": 0.85},
                "tests": [
                    {"path": "tests/CartTest.php", "to": "src/cart.php", "file": "tests/CartTest.php", "name": "tests/CartTest.php", "steps": 1, "coverage_verified": true},
                    {"path": "DynamicSuite::testTotal", "to": "src/cart.php", "file": "DynamicSuite::testTotal", "name": "DynamicSuite::testTotal", "steps": 1, "source": "coverage", "coverage_verified": true}
                ]
            },
            {
                "path": "src/price.php",
                "state": "ready",
                "total": 0,
                "coverage": {"format": "clover", "covered_lines": 3, "total_lines": 4, "rate": 0.75},
                "tests": []
            }
        ],
        "suspects": {
            "failing": ["tests/CartTest.php"],
            "unmatched": [],
            "tests": 2,
            "rows": [
                {
                    "path": "src/cart.php",
                    "status": "M",
                    "rank": 1,
                    "score": 1.0,
                    "failing": 1,
                    "passing": 0,
                    "nearest": 1,
                    "coverage": {"format": "lcov", "covered_lines": 34, "total_lines": 40, "rate": 0.85},
                    "coverage_verified": true,
                    "source": "coverage"
                }
            ],
            "more": 0,
            "clear": 0,
            "complete": true
        }
    });

    let reach_html = test_paths(&view);
    crate::test_support::assert_no_inline_hex_colors(&reach_html);
    assert!(reach_html.contains("LCOV 85% (34/40)"), "{reach_html}");
    assert!(reach_html.contains("Clover 75% (3/4)"), "{reach_html}");
    assert!(
        reach_html.contains("aria-label=\"Coverage LCOV 85% (34/40)\""),
        "{reach_html}"
    );
    assert!(reach_html.contains("data-kind=\"source\""), "{reach_html}");
    assert!(reach_html.contains("from coverage"), "{reach_html}");
    assert!(
        reach_html.contains("data-kind=\"verified\""),
        "{reach_html}"
    );
    assert!(reach_html.contains("coverage verified"), "{reach_html}");

    let suspects_html = suspects_side(&view, &json!({"failing": ["CartTest"]}), SESSION_SURFACE);
    crate::test_support::assert_no_inline_hex_colors(&suspects_html.replace(">#1<", ">rank-1<"));
    assert!(!suspects_html.contains("style="), "{suspects_html}");
    assert!(
        suspects_html.contains("LCOV 85% (34/40)"),
        "{suspects_html}"
    );
    assert!(suspects_html.contains("from coverage"), "{suspects_html}");
    assert!(
        suspects_html.contains("coverage verified"),
        "{suspects_html}"
    );

    let sum = summary(&view);
    assert!(sum.contains("Native tree-sitter IR"), "{sum}");
    assert!(!sum.contains("native-tree-sitter-ir"), "{sum}");

    assert_eq!(
        format_graph_backend_label("native-tree-sitter-ir"),
        "Native tree-sitter IR"
    );
    let status_line = format_graph_status_text(
        "Code graph & dependency map ready (native tree-sitter SQLite IR).",
    );
    assert_eq!(
        status_line,
        "Code graph & dependency map ready (Native tree-sitter IR)."
    );
    let combined = combine_note(
        interim(
            &json!({"state": "building", "preliminary": false}),
            "Indexing via native-tree-sitter-ir",
        ),
        &json!({"mode": "whole"}),
    );
    assert!(combined.contains("Native tree-sitter IR"), "{combined}");
    assert!(!combined.contains("native-tree-sitter-ir"), "{combined}");

    let attachment = map_attachment(&view).expect("map attachment");
    assert!(
        attachment.contains("Graph backend: Native tree-sitter IR"),
        "{attachment}"
    );
    assert!(
        attachment.contains("Observed coverage: LCOV 85% (34/40)"),
        "{attachment}"
    );
    assert!(
        attachment.contains("[from coverage, coverage verified]"),
        "{attachment}"
    );
}
