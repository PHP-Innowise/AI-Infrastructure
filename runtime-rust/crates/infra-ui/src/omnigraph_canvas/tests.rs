#![allow(unused_imports)]
use super::constellation::*;
use super::dag::*;
use super::merge_diff::*;
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
    assert_eq!(
        res.html.matches("<linearGradient ").count(),
        EDGE_SPECS.len() * 4
    );
    for spec in EDGE_SPECS {
        for suffix in ["", "-rtl", "-ttb", "-btt"] {
            assert!(
                res.html.contains(&format!(
                    "<linearGradient id=\"omni-flow-{}{suffix}\"",
                    spec.key
                )),
                "missing gradient def for {}{suffix}",
                spec.key
            );
        }
    }
    assert!(
        res.html
            .contains("stroke=\"url(#omni-flow-published_by-ttb)\""),
        "published_by flows from top atom-1 toward vertically centered sess-1"
    );
    assert!(
        res.html.contains("stroke=\"url(#omni-flow-touches_file)\"")
            || res
                .html
                .contains("stroke=\"url(#omni-flow-touches_file-ttb)\"")
            || res
                .html
                .contains("stroke=\"url(#omni-flow-touches_file-btt)\""),
        "atom->file touches_file never gets -rtl"
    );
    assert!(res.html.contains("data-lod=\"near\""));

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
        "Finding", "Decision", "Warning", "Contract", "Insight", "Artifact", "Question", "Session",
        "File",
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

use crate::test_support::assert_no_inline_hex_colors;

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
    let atom_boxes: Vec<(f64, f64)> = (0..24).map(|i| res.positions[&format!("a-{i}")]).collect();
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

    // Zoomed out (<0.95) sets `data-lod="mid"` / `"far"` so CSS hides unlit mid-edge pills while keeping lit ones visible.
    assert_eq!(lod(0.4), "far");
    assert_eq!(lod(0.6), "mid");
    assert_eq!(lod(0.94), "mid");
    assert_eq!(lod(0.95), "near");
    assert_eq!(lod(1.15), "near");
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
    assert!(far.html.contains("data-lod=\"mid\""));
    assert!(far.html.contains("class=\"omni-edge-label\""));
    assert!(!far.html.contains("data-lit=\"true\""));

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
    let empty_dag = render_branch_dag(&json!({"branches": ["main"], "commits": []}), "main", None);
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
            { "from": "s-beta", "to": "s-alpha", "kind": "session_follows", "relation": "shared_branch" },
            { "from": "s-alpha", "to": "s-beta", "type": "SessionFollows", "relation": "broadcast" }
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
    assert_eq!(res.edge_count, 2);
    assert!(res.html.contains("omni-space-pill"));
    assert!(res.html.contains("shared/sprint-auth"));
    assert!(res.html.contains("omni-edge-session_follows"));
    assert!(res.html.contains("url(#omni-arrow-session_follows)"));
    assert!(!res.html.contains("omni-edge-relates_to"));
    assert!(res.html.contains(">follows<"));
    assert_no_inline_hex_colors(&res.html);
}

#[test]
fn conflict_card_offers_keep_only_for_atom_updates_with_both_values() {
    use crate::test_support::payloads;
    let source_atom = json!({"kind": "decision", "title": "Signing", "summary": "RS256 rotated keys", "body": "Rotate daily.", "tags": ["auth"], "confidence": 0.9, "pin_priority": 40, "updated_at": "2026-11-17T11:00:00"});
    let target_atom = json!({"kind": "decision", "title": "Signing", "summary": "EdDSA keys", "body": "Rotate weekly.", "tags": ["auth"], "confidence": 0.8, "pin_priority": 40, "authority": "verified", "brain_record_id": "rec-1", "updated_at": "2026-11-17T12:00:00"});
    let diff = json!({
        "source": "session/s1",
        "target": "main",
        "outcome": "conflict",
        "conflicts": [
            {"entity_kind": "node", "type_name": "ContextAtom", "entity_id": "atom-2", "kind": "divergent_update", "message": "Both changed atom-2", "source_value": source_atom, "target_value": target_atom},
            {"entity_kind": "node", "type_name": "ContextAtom", "entity_id": "atom-9", "kind": "delete_vs_update", "source_value": null, "target_value": {"title": "Cache policy"}},
            {"entity_kind": "edge", "type_name": "RelatesTo", "entity_id": "edge-4", "kind": "divergent_update", "source_value": {"relation": "supports"}, "target_value": {"relation": "contradicts"}},
            {"entity_kind": "node", "type_name": "ContextAtom", "entity_id": "atom-42", "kind": "divergent_update", "message": "Conflicting updates on atom-42"},
            {"kind": "divergent_update", "entity_id": "atom-7", "field": "summary", "source": "A", "target": "B"}
        ]
    });
    let html = render_merge_diff(&diff);
    // Only the Context Atom update with both values gets live buttons:
    // keeping the source copies it onto the target, and the reverse.
    let keeps = payloads(&html, "omnigraph-conflict-keep");
    assert_eq!(keeps.len(), 2, "{html}");
    assert_eq!(keeps[0]["keep"], "source");
    assert_eq!(keeps[0]["write_branch"], "main");
    assert_eq!(keeps[0]["value"], source_atom);
    assert_eq!(keeps[0]["base"], target_atom);
    assert_eq!(keeps[1]["keep"], "target");
    assert_eq!(keeps[1]["write_branch"], "session/s1");
    assert_eq!(keeps[1]["value"], target_atom);
    for keep in &keeps {
        assert_eq!(keep["atom_id"], "atom-2");
        assert_eq!(keep["source_branch"], "session/s1");
        assert_eq!(keep["target_branch"], "main");
        assert!(keep["field"].is_null());
    }
    // Each button asks the server to copy the kept atom as stored,
    // naming both versions the preview showed.
    let request = |keep: &Value| conflict_keep_request("proj-1", keep).expect("live button");
    assert_eq!(
        request(&keeps[0]),
        json!({"action": "keep", "project_id": "proj-1", "atom_id": "atom-2", "from": "session/s1", "to": "main", "expected_updated_at": "2026-11-17T11:00:00", "replaced_updated_at": "2026-11-17T12:00:00"})
    );
    assert_eq!(
        request(&keeps[1]),
        json!({"action": "keep", "project_id": "proj-1", "atom_id": "atom-2", "from": "main", "to": "session/s1", "expected_updated_at": "2026-11-17T12:00:00", "replaced_updated_at": "2026-11-17T11:00:00"})
    );
    assert!(html
        .contains("Keep source copies the source's atom to <strong>main</strong> as it is stored"));
    // Every other card shows both buttons disabled and says why.
    assert_eq!(
        html.matches("class=\"button compact\" disabled>Keep source</button>")
            .count(),
        4,
        "{html}"
    );
    for note in [
        "One branch deleted this atom while the other changed it.",
        "Edge conflicts are not resolved here",
        "The merge preview did not return both values.",
        "Only two different updates of one Context Atom can be resolved here.",
    ] {
        assert!(html.contains(note), "{note}: {html}");
    }
    assert_no_inline_hex_colors(&html);
}

#[test]
fn conflict_keep_request_copies_the_kept_atom_between_the_branches() {
    let keep = json!({
        "atom_id": "atom-2",
        "keep": "source",
        "source_branch": "session/s1",
        "target_branch": "main",
        "write_branch": "main",
        "field": null,
        "value": {"properties": {"title": "Signing", "summary": "RS256 rotated keys", "updated_at": "2026-11-17T11:00:00"}},
        "base": {"title": "Signing", "summary": "EdDSA keys", "updated_at": "2026-11-17T12:00:00"}
    });
    assert_eq!(
        conflict_keep_request("proj-1", &keep).expect("whole atom"),
        json!({"action": "keep", "project_id": "proj-1", "atom_id": "atom-2", "from": "session/s1", "to": "main", "expected_updated_at": "2026-11-17T11:00:00", "replaced_updated_at": "2026-11-17T12:00:00"})
    );

    // Omnigraph merges whole atoms, so a field-level conflict copies the
    // kept atom too; values without `updated_at` name no version.
    let field = json!({"atom_id": "atom-7", "keep": "target", "source_branch": "session/s1", "target_branch": "main", "write_branch": "session/s1", "field": "summary", "value": "Use EdDSA", "base": "Use RS256"});
    assert_eq!(
        conflict_keep_request("proj-1", &field).expect("one field"),
        json!({"action": "keep", "project_id": "proj-1", "atom_id": "atom-7", "from": "main", "to": "session/s1"})
    );

    // Without a project, an atom, a known side or two branches there is
    // nothing to copy.
    assert!(conflict_keep_request("", &keep).is_err());
    let mut broken = keep.clone();
    broken["atom_id"] = json!("");
    assert!(conflict_keep_request("proj-1", &broken).is_err());
    let mut broken = keep.clone();
    broken["keep"] = json!("both");
    assert!(conflict_keep_request("proj-1", &broken).is_err());
    let mut broken = keep.clone();
    broken["write_branch"] = json!("session/s1");
    assert!(conflict_keep_request("proj-1", &broken).is_err());
}

#[test]
fn symbol_ref_nodes_and_mentions_symbol_edges_render_without_inline_hex_colors() {
    let mut graph = sample_graph();
    graph["atoms"][0]["symbols"] = json!(["src/auth/jwt.rs::JwtVerifier::verify"]);
    graph["symbols"] = json!([
        {
            "id": "src/auth/jwt.rs::JwtVerifier::verify",
            "fqn": "src/auth/jwt.rs::JwtVerifier::verify",
            "project_id": "proj-1",
            "file_path": "src/auth/jwt.rs",
            "path": "src/auth/jwt.rs",
            "kind": "method",
            "name": "verify",
            "owner": "JwtVerifier",
            "label": "JwtVerifier::verify",
            "mention_count": 2,
            "atom_ids": ["atom-1", "atom-3"]
        }
    ]);
    graph["edges"].as_array_mut().unwrap().push(json!({
        "id": "edge-sym-1",
        "kind": "mentions_symbol",
        "type": "MentionsSymbol",
        "relation": "mentions_symbol",
        "from": "atom-1",
        "to": "src/auth/jwt.rs::JwtVerifier::verify",
        "fqn": "src/auth/jwt.rs::JwtVerifier::verify",
        "file_path": "src/auth/jwt.rs"
    }));
    graph["symbol_edges"] = json!([
        {
            "id": "edge-sym-1",
            "kind": "mentions_symbol",
            "relation": "mentions_symbol",
            "source": "atom-1",
            "target": "src/auth/jwt.rs::JwtVerifier::verify",
            "fqn": "src/auth/jwt.rs::JwtVerifier::verify"
        },
        {
            "id": "edge-sym-2",
            "kind": "mentions_symbol",
            "relation": "mentions_symbol",
            "source": "atom-3",
            "target": "src/auth/jwt.rs::JwtVerifier::verify",
            "fqn": "src/auth/jwt.rs::JwtVerifier::verify"
        }
    ]);

    let filter = ConstellationFilter {
        show_files: true,
        show_sessions: true,
        ..Default::default()
    };
    let rendered = render_constellation(&graph, &filter, &CanvasViewport::default());
    assert_no_inline_hex_colors(&rendered.html);
    assert_no_inline_hex_colors(&rendered.radar_html);

    // Symbol card renders on the outer orbital track with full bridge metadata.
    assert!(
        rendered.html.contains("data-kind=\"symbol\""),
        "{}",
        rendered.html
    );
    assert!(
        rendered
            .html
            .contains("data-symbol-fqn=\"src/auth/jwt.rs::JwtVerifier::verify\""),
        "{}",
        rendered.html
    );
    assert!(
        rendered.html.contains("data-file-path=\"src/auth/jwt.rs\""),
        "{}",
        rendered.html
    );
    assert!(
        rendered.html.contains("JwtVerifier::verify"),
        "{}",
        rendered.html
    );

    // MentionsSymbol edges are deduplicated (edge-sym-1 appears in edges & symbol_edges),
    // while edge-sym-2 from symbol_edges is preserved.
    assert_eq!(
        rendered
            .html
            .matches("omni-edge omni-edge-mentions_symbol")
            .count(),
        2,
        "{}",
        rendered.html
    );
    assert!(
        rendered
            .html
            .contains("<title>atom-3 mentions src/auth/jwt.rs::JwtVerifier::verify</title>"),
        "{}",
        rendered.html
    );

    // Both atom-1 (from atom["symbols"]) and atom-3 (from edge fallback) get a symbol badge.
    assert_eq!(
        rendered.html.matches("class=\"omni-symbol-badge\"").count(),
        2,
        "{}",
        rendered.html
    );
    assert!(
        rendered
            .html
            .contains("title=\"Mentioned symbols: JwtVerifier::verify\""),
        "{}",
        rendered.html
    );
    assert!(
        rendered
            .radar_html
            .contains("omni-radar-dot omni-radar-dot-symbol"),
        "{}",
        rendered.radar_html
    );

    // Symbol node does not overlap File node on Track 3.
    let sym_pos = rendered
        .positions
        .get("src/auth/jwt.rs::JwtVerifier::verify")
        .copied()
        .expect("symbol position");
    let file_pos = rendered
        .positions
        .get("file:src/auth/jwt.rs")
        .copied()
        .expect("file position");
    let dx = (sym_pos.0 - file_pos.0).abs();
    let dy = (sym_pos.1 - file_pos.1).abs();
    assert!(
        dx >= 150.0 || dy >= 50.0,
        "overlap: {sym_pos:?} vs {file_pos:?}"
    );
}

#[test]
fn show_files_false_hides_symbol_ref_nodes_and_mentions_symbol_edges() {
    let mut graph = sample_graph();
    graph["symbols"] = json!([
        {
            "id": "src/auth/jwt.rs::verify",
            "fqn": "src/auth/jwt.rs::verify",
            "file_path": "src/auth/jwt.rs",
            "kind": "function",
            "name": "verify",
            "label": "verify"
        }
    ]);
    graph["symbol_edges"] = json!([
        {
            "id": "se-1",
            "kind": "mentions_symbol",
            "relation": "mentions_symbol",
            "from": "atom-1",
            "to": "src/auth/jwt.rs::verify"
        }
    ]);
    let filter = ConstellationFilter {
        show_files: false,
        show_sessions: true,
        ..Default::default()
    };
    let rendered = render_constellation(&graph, &filter, &CanvasViewport::default());
    assert!(!rendered.html.contains("data-kind=\"symbol\""));
    assert!(!rendered.html.contains("omni-edge-mentions_symbol"));
    assert_no_inline_hex_colors(&rendered.html);
}

#[test]
fn anchored_pagination_pinning_path_cone_drift_and_dense_virtualization() {
    let mut graph = sample_graph();
    let initial = render_constellation(
        &graph,
        &ConstellationFilter {
            show_files: true,
            show_sessions: true,
            ..Default::default()
        },
        &CanvasViewport::default(),
    );
    let first_pos = initial
        .raw_positions
        .get("atom-1")
        .copied()
        .expect("atom-1 pos");

    // Append extra atoms (simulating Load more page 2) with anchored_positions
    let mut extra_atoms = graph["atoms"].as_array().unwrap().clone();
    for i in 10..22 {
        extra_atoms.push(json!({
            "id": format!("atom-{i}"),
            "kind": "insight",
            "title": format!("Extra insight {i}"),
            "branch": "main"
        }));
    }
    graph["atoms"] = json!(extra_atoms);

    let paged = render_constellation(
        &graph,
        &ConstellationFilter {
            show_files: true,
            show_sessions: true,
            anchored_positions: initial.raw_positions.clone(),
            paged_count: 2,
            ..Default::default()
        },
        &CanvasViewport::default(),
    );
    assert_eq!(paged.raw_positions.get("atom-1").copied(), Some(first_pos));
    assert!(paged.dense_scene.is_some());
    assert!(paged.html.contains("data-scale-tier=\"dense\""));

    // Pin override + path + 1-hop isolate cone + drift injection
    let mut filter = ConstellationFilter {
        show_files: true,
        show_sessions: true,
        ..Default::default()
    };
    let path = compute_constellation_path(&graph, &filter, "atom-1", "src/auth/jwt.rs");
    assert!(path.is_some());
    let (path_nodes, _) = path.unwrap();
    assert!(path_nodes.contains(&"atom-1".to_owned()));

    let cone = compute_constellation_cone(&graph, &filter, "atom-1", 1);
    assert!(cone.contains("atom-1"));
    assert!(!cone.contains("atom-20"));

    inject_workspace_drift(
        &mut graph,
        Some(&json!({ "files": [{ "path": "src/auth/jwt.rs", "broken_risk": true }] })),
        None,
    );
    filter.isolate_seed = Some("atom-1".to_owned());
    filter.isolate_depth = 1;
    filter
        .pinned_positions
        .insert("atom-1".to_owned(), (333.0, 222.0));
    let rendered_drift = render_constellation(&graph, &filter, &CanvasViewport::default());
    assert_eq!(
        rendered_drift.raw_positions.get("atom-1").copied(),
        Some((333.0, 222.0))
    );
    assert!(rendered_drift.html.contains("data-drift=\"broken\""));
    assert!(rendered_drift.html.contains("tabindex=\"0\""));
    assert_no_inline_hex_colors(&rendered_drift.html);

    // Digest stability: selection change alone preserves structure digest.
    let mut sel_filter = filter.clone();
    sel_filter.selected_id = Some("atom-1".to_owned());
    assert_eq!(
        constellation_structure_digest(&graph, &filter),
        constellation_structure_digest(&graph, &sel_filter)
    );
}

#[test]
fn dense_constellation_keeps_cubic_curves_node_htmls_and_offscreen_roving_node() {
    let mut atoms = Vec::with_capacity(190);
    let mut links = Vec::with_capacity(189);
    for i in 0..190 {
        atoms.push(json!({
            "id": format!("a-{i}"),
            "kind": "insight",
            "title": format!("Atom {i}"),
            "branch": "main",
        }));
        if i > 0 {
            links.push(json!({
                "from": format!("a-{}", i - 1),
                "to": format!("a-{i}"),
                "relation": "follows",
            }));
        }
    }
    let graph = json!({
        "active_branch": "main",
        "sessions": [],
        "atoms": atoms,
        "files": [],
        "symbols": [],
        "links": links,
    });
    let rendered = render_constellation(
        &graph,
        &ConstellationFilter {
            focused_id: Some("a-189".to_owned()),
            viewport_bounds: Some((-10.0, -10.0, 10.0, 10.0)),
            paged_count: 10,
            ..Default::default()
        },
        &CanvasViewport::default(),
    );
    let scene = rendered.dense_scene.as_ref().expect("dense scene");
    assert_eq!(rendered.node_htmls.len(), rendered.node_count);
    assert_eq!(rendered.edge_endpoints.len(), rendered.edge_count);
    assert!(
        scene.edges.iter().all(|e| e
            .cmds
            .iter()
            .any(|c| matches!(c, graph_viewport::canvas2d::DensePathCmd::Curve { .. }))),
        "dense constellation edges should preserve cubic Bezier commands"
    );
    assert!(
        rendered.html.contains("data-node-id=\"a-189\"")
            && rendered.html.contains("tabindex=\"0\""),
        "offscreen roving focus node must remain mounted in dense constellation"
    );
}

#[test]
fn radar_extreme_pan_empty_state_bm25_filter_and_scale_layout_behave_safely() {
    let graph = sample_graph();

    // 1. Extreme bottom/right pan on small or large plane must not panic in radar_view clamping.
    let extreme_vp = CanvasViewport {
        pan_x: -5000.0,
        pan_y: -5000.0,
        scale: 1.0,
    };
    let rendered = render_constellation(
        &graph,
        &ConstellationFilter {
            show_files: true,
            show_sessions: true,
            ..Default::default()
        },
        &extreme_vp,
    );
    assert!(rendered.radar_html.contains("class=\"omni-radar\""));
    assert!(rendered.radar_html.contains("tabindex=\"0\""));

    // 2. Empty main branch with no active filter shows publish CTA, not Reset filters loop.
    let empty_graph = json!({
        "active_branch": "main",
        "sessions": [],
        "atoms": [],
        "files": [],
        "symbols": [],
        "links": []
    });
    let empty_main = render_constellation(
        &empty_graph,
        &ConstellationFilter {
            branch: "main".to_owned(),
            show_files: true,
            show_sessions: true,
            ..Default::default()
        },
        &CanvasViewport {
            pan_x: -180.0,
            pan_y: -300.0,
            scale: 0.48,
        },
    );
    assert!(empty_main.html.contains("omnigraph-open-publish"));
    assert!(!empty_main.html.contains("omnigraph-clear-filters"));
    assert!(empty_main
        .html
        .contains("width:100%;height:100%;min-height:360px;transform:none;transition:none"));
    assert!(empty_main
        .html
        .contains("--graph-pan-x:0.0px;--graph-pan-y:0.0px;--graph-scale:1.0000;"));

    let hidden_toggles = render_constellation(
        &empty_graph,
        &ConstellationFilter {
            branch: "main".to_owned(),
            show_files: false,
            show_sessions: true,
            ..Default::default()
        },
        &CanvasViewport::default(),
    );
    assert!(hidden_toggles.html.contains("omnigraph-clear-filters"));

    // 3. Multi-word query and BM25 highlighted_ids keep matches visible.
    let multi_word = render_constellation(
        &graph,
        &ConstellationFilter {
            query: "atom-1 finding".to_owned(),
            show_files: true,
            show_sessions: true,
            ..Default::default()
        },
        &CanvasViewport::default(),
    );
    assert!(multi_word.node_count > 0);
    let mut hl_set = BTreeSet::new();
    hl_set.insert("atom-1".to_owned());
    let bm25_match = render_constellation(
        &graph,
        &ConstellationFilter {
            query: "synonym query not in blob".to_owned(),
            highlighted_ids: hl_set,
            show_files: true,
            show_sessions: true,
            ..Default::default()
        },
        &CanvasViewport::default(),
    );
    assert!(bm25_match.positions.contains_key("atom-1"));

    // 4. Pinning one node near (10, 10) does not shift unpinned nodes; File+Symbol cluster stays contiguous.
    let base_layout = render_constellation(
        &graph,
        &ConstellationFilter {
            show_files: true,
            show_sessions: true,
            ..Default::default()
        },
        &CanvasViewport::default(),
    );
    let sym_pos_before = base_layout
        .raw_positions
        .get("src/auth/jwt.rs::JwtService::verify")
        .copied();
    let mut top_left_pins = BTreeMap::new();
    top_left_pins.insert("atom-1".to_owned(), (10.0, 12.0));
    let pinned_layout = render_constellation(
        &graph,
        &ConstellationFilter {
            show_files: true,
            show_sessions: true,
            pinned_positions: top_left_pins,
            ..Default::default()
        },
        &CanvasViewport::default(),
    );
    assert_eq!(
        pinned_layout.raw_positions.get("atom-1"),
        Some(&(10.0, 12.0))
    );
    assert_eq!(
        pinned_layout
            .raw_positions
            .get("src/auth/jwt.rs::JwtService::verify")
            .copied(),
        sym_pos_before
    );

    // 5. inject_workspace_drift consumes `/api/projects/{id}/graph/changes` shapes (`files[].impact`, `edge_diffs.risk_callers`)
    //    and Symbol nodes expose canonical `data-symbol-id`.
    let mut drift_graph = json!({
        "files": [
            { "id": "src/auth/jwt.rs", "path": "src/auth/jwt.rs" },
            { "id": "src/http/handler.rs", "path": "src/http/handler.rs" },
            { "id": "src/util.rs", "path": "src/util.rs" }
        ],
        "symbols": [{
            "id": "src/auth/jwt.rs::JwtService::verify",
            "symbol_id": "sym:jwt:verify:12",
            "fqn": "src/auth/jwt.rs::JwtService::verify",
            "file_path": "src/auth/jwt.rs",
            "kind": "method",
            "name": "verify"
        }]
    });
    let changes_payload = json!({
        "files": [
            { "path": "src/util.rs", "status": "modified", "impact": "modified" },
            { "path": "src/http/handler.rs", "impact": "broken_risk" }
        ],
        "edge_diffs": {
            "risk_callers": [{ "from": "src/http/handler.rs", "to": "src/auth/jwt.rs", "impact": "broken_risk" }]
        }
    });
    inject_workspace_drift(&mut drift_graph, Some(&changes_payload), None);
    let drift_out = render_constellation(
        &drift_graph,
        &ConstellationFilter {
            show_files: true,
            show_sessions: true,
            ..Default::default()
        },
        &CanvasViewport::default(),
    );
    assert!(
        drift_out
            .html
            .contains("data-symbol-id=\"sym:jwt:verify:12\""),
        "{}",
        drift_out.html
    );
    assert!(
        drift_out.html.contains("data-drift=\"broken\""),
        "{}",
        drift_out.html
    );
    assert!(
        drift_out.html.contains("data-drift=\"changed\""),
        "{}",
        drift_out.html
    );
}
