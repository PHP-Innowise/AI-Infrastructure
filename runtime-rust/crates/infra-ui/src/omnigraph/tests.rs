#![allow(unused_imports)]
use super::constellation_view::*;
use super::dialogs::*;
use super::inspector::*;
use super::interactions::*;
use super::pulse::*;
use super::session_bar::*;
use super::status::*;
use super::*;

#[test]
fn pill_presentation_reflects_connected_building_stopped_failed_and_off_states() {
    let live = json!({
        "connected": true,
        "mode": "local",
        "stats": { "atoms": 14, "branches": 3 }
    });
    let (state, shape, label, title) = pill_presentation(&live);
    assert_eq!(state, "ready");
    assert_eq!(shape, "●");
    assert_eq!(label, "Omnigraph: local · 14 atoms");
    assert!(title.contains("3 branches"));

    let starting = json!({ "connected": false, "state": "starting" });
    assert_eq!(pill_presentation(&starting).0, "building");

    let failed = json!({ "connected": false, "error": "port 8080 in use" });
    assert_eq!(pill_presentation(&failed).0, "failed");
    assert!(pill_presentation(&failed).3.contains("port 8080"));

    let stopped = json!({ "connected": false, "binary_installed": true });
    assert_eq!(pill_presentation(&stopped).0, "stale");
    assert_eq!(pill_presentation(&stopped).2, "Omnigraph: stopped");

    let off = json!({ "connected": false });
    assert_eq!(pill_presentation(&off).0, "off");
    assert_eq!(pill_presentation(&off).2, "Omnigraph: off");
}

#[test]
fn session_payload_parses_branch_modes_pull_branches_and_query() {
    assert!(session_payload_from_fields(false, "auto", "", "main", "auth", true, false).is_none());

    let auto = session_payload_from_fields(
        true,
        "auto",
        "ignored",
        "main, session/fix-1, main , ",
        "  auth tokens  ",
        true,
        true,
    )
    .unwrap();
    assert_eq!(
        auto,
        json!({
            "enabled": true,
            "branch": "auto",
            "pull_branches": ["main", "session/fix-1"],
            "query": "auth tokens",
            "auto_publish": true,
            "auto_merge": true,
        })
    );

    let existing =
        session_payload_from_fields(true, "existing", "session/wave-2", "", "", false, false)
            .unwrap();
    assert_eq!(existing["branch"], "session/wave-2");
    assert!(existing.get("pull_branches").is_none());
}

#[test]
fn camera_zoom_at_preserves_cursor_world_point_and_fly_to_centers_node() {
    let (s, px, py) = zoom_at(1.0, 20.0, 40.0, (220.0, 140.0), 1.5);
    assert!((s - 1.5).abs() < 1e-6);
    let world_x_before = (220.0 - 20.0) / 1.0;
    let world_x_after = (220.0 - px) / s;
    assert!((world_x_before - world_x_after).abs() < 1.0);
    let world_y_before = (140.0 - 40.0) / 1.0;
    let world_y_after = (140.0 - py) / s;
    assert!((world_y_before - world_y_after).abs() < 1.0);

    let (fs, fpx, fpy) = camera_for_fly_to((300.0, 200.0), (800.0, 600.0), 1.0);
    assert_eq!(fs, 1.0);
    assert_eq!(fpx, 400.0 - 300.0);
    assert_eq!(fpy, 300.0 - 200.0);
}

#[test]
fn inspector_renders_atom_provenance_relations_files_and_escapes_html() {
    let graph = json!({
        "atoms": [{
            "id": "atom-1",
            "kind": "warning",
            "title": "Avoid <unsafe> SQL",
            "summary": "Found unescaped interpolation & risk",
            "body": "Use bound parameters everywhere.",
            "branch": "session/sec-audit",
            "confidence": 0.92,
            "pin_priority": 80,
            "actor_id": "codex-sec",
            "session_id": "sess-9",
            "commit_id": "commit-abcdef1234",
            "tags": ["security", "sql"],
            "files": ["src/Repo.php"]
        }],
        "sessions": [{ "id": "sess-9", "title": "Security Audit" }],
        "edges": [
            { "from": "atom-1", "to": "atom-2", "kind": "contradicts" },
            { "from": "atom-1", "to": "src/Repo.php", "kind": "touches_file" }
        ]
    });
    let sessions = vec![json!({ "id": "sess-10", "title": "Fix Worker" })];
    let html = render_node_inspector_html(&graph, "atom-1", &sessions, "main");
    assert!(html.contains("Avoid &lt;unsafe&gt; SQL"));
    assert!(html.contains("interpolation &amp; risk"));
    assert!(html.contains("92% confidence"));
    assert!(html.contains("Pin 80"));
    assert!(html.contains("session/sec-audit"));
    assert!(html.contains("src/Repo.php"));
    assert!(html.contains("contradicts"));
    assert!(html.contains("Fix Worker"));
    assert!(html.contains("data-action=\"omnigraph-share-atom\""));
    assert!(html.contains("data-action=\"omnigraph-copy-snippet\""));
    assert!(html.contains("data-action=\"omnigraph-open-branch\""));
}

#[test]
fn search_hits_and_pulse_events_render_and_escape_safely() {
    let hits = json!({
        "query": "<script>",
        "hits": [{
            "atom": {
                "id": "a1",
                "kind": "decision",
                "title": "Use <Outbox> table",
                "summary": "Atomic writes"
            },
            "score": 4.25,
            "matched_via": "bm25",
            "neighbors": ["a2"]
        }]
    });
    let hits_html = render_search_hits_html(&hits);
    assert!(!hits_html.contains("<script>"));
    assert!(hits_html.contains("Use &lt;Outbox&gt; table"));
    assert!(hits_html.contains("4.25"));
    assert!(hits_html.contains("1 neighbors"));

    let pulse = vec![json!({
        "kind": "atom_published",
        "branch": "session/s1",
        "atom_id": "a1",
        "title": "New finding & fix",
        "actor_id": "claude",
        "created_at": "2026-11-16T18:00:00Z"
    })];
    let pulse_html = render_pulse_list_html(&pulse);
    assert!(pulse_html.contains("New finding &amp; fix"));
    assert!(pulse_html.contains("data-action=\"omnigraph-pulse-focus\""));
    assert!(pulse_html.contains("data-node-id=\"a1\""));
}

#[test]
fn session_bar_extraction_handles_branches_and_atoms() {
    let off = json!({ "id": "s1" });
    assert!(!session_bar_summary(&off).0);

    let active = json!({
        "id": "s2",
        "omnigraph": {
            "enabled": true,
            "branch": "session/s2",
            "auto_publish": true,
            "auto_merge": false,
            "injected_atoms": [
                { "id": "a1", "kind": "contract", "title": "API schema" }
            ]
        }
    });
    let (visible, branch, meta, atoms, can_merge) = session_bar_summary(&active);
    assert!(visible);
    assert_eq!(branch, "session/s2");
    assert!(meta.contains("1 shared atoms"));
    assert!(meta.contains("auto-publish"));
    assert_eq!(atoms.len(), 1);
    assert!(can_merge);
}

#[test]
fn csv_parser_trims_and_deduplicates_entries() {
    assert_eq!(
        parse_csv_list(" auth , migration, auth\napi , "),
        vec!["auth", "migration", "api"]
    );
    assert!(parse_csv_list(" , \n ").is_empty());
}

#[test]
fn commit_inspector_session_and_file_nodes_render_cleanly() {
    let graph = json!({
        "sessions": [{ "id": "sess-1", "title": "Auth Refactor", "branch": "session/auth", "status": "completed" }],
        "files": [{ "id": "src/auth.rs", "path": "src/auth.rs" }],
        "atoms": [{ "id": "atom-1", "kind": "finding", "title": "Token check", "branch": "session/auth" }],
        "edges": [
            { "from": "atom-1", "to": "sess-1", "kind": "published_by" },
            { "from": "atom-1", "to": "src/auth.rs", "kind": "touches_file" }
        ]
    });
    let sess_html = render_node_inspector_html(&graph, "sess-1", &[], "main");
    assert!(sess_html.contains("Auth Refactor"));
    assert!(sess_html.contains("data-action=\"session-open\""));
    assert!(sess_html.contains("published_by"));

    let file_html = render_node_inspector_html(&graph, "src/auth.rs", &[], "main");
    assert!(file_html.contains("src/auth.rs"));
    assert!(file_html.contains("data-action=\"inspect-file-graph\""));

    let detail = json!({
        "commit": { "id": "c-1234", "branch": "session/auth", "actor_id": "claude", "created_at": "2026-11-16T18:05:00Z" },
        "changes": [{ "op": "insert", "title": "Token <check>" }],
        "atoms": [{ "id": "atom-1", "title": "Token check" }]
    });
    let commit_html = render_commit_inspector_html(&detail);
    assert!(commit_html.contains("c-1234"));
    assert!(commit_html.contains("Token &lt;check&gt;"));
    assert!(commit_html.contains("data-action=\"omnigraph-return-live\""));
}

#[test]
fn context_snippet_and_canvas_positions_integrate_cleanly() {
    let atom = json!({
        "kind": "contract",
        "title": "Bearer Auth Header",
        "branch": "main",
        "summary": "All requests require bearer auth.",
        "files": ["src/http.rs", "src/auth.rs"]
    });
    let snippet = format_context_snippet(&atom);
    assert!(snippet.contains("[Contract] Bearer Auth Header (branch: main)"));
    assert!(snippet.contains("Files: src/http.rs, src/auth.rs"));

    let graph = json!({
        "branches": ["main"],
        "sessions": [{ "id": "s1", "title": "Session 1", "branch": "main" }],
        "atoms": [{ "id": "a1", "kind": "finding", "title": "Auth finding", "branch": "main" }],
        "files": [{ "id": "src/auth.rs", "path": "src/auth.rs" }],
        "edges": [
            { "from": "a1", "to": "s1", "kind": "published_by" },
            { "from": "a1", "to": "src/auth.rs", "kind": "touches_file" }
        ]
    });
    let rendered = omnigraph_canvas::render_constellation(
        &graph,
        &omnigraph_canvas::ConstellationFilter {
            show_files: true,
            show_sessions: true,
            ..Default::default()
        },
        &omnigraph_canvas::CanvasViewport {
            pan_x: 0.0,
            pan_y: 0.0,
            scale: 1.0,
        },
    );
    assert!(rendered.node_count >= 3);
    let pos = rendered
        .positions
        .get("a1")
        .copied()
        .expect("a1 center position");
    let (scale, px, py) = camera_for_fly_to(pos, (800.0, 600.0), 1.0);
    assert_eq!(scale, 1.0);
    assert_eq!(px, (400.0 - pos.0).round());
    assert_eq!(py, (300.0 - pos.1).round());
}

#[test]
fn backend_contract_alignment_relates_to_conflicts_raw_changes_and_session_bar() {
    assert_eq!(
        parse_relates_to(
            "atom-1, supports: atom-2, atom-1, contradicts:atom-3, :atom-4, relates_to: atom-5, RELATED:atom-1"
        ),
        vec![
            json!({ "target_id": "atom-1", "relation": "supports" }),
            json!({ "target_id": "atom-2", "relation": "supports" }),
            json!({ "target_id": "atom-3", "relation": "contradicts" }),
            json!({ "target_id": "atom-4", "relation": "supports" }),
            json!({ "target_id": "atom-5", "relation": "supports" }),
        ]
    );

    let conflict_res = json!({
        "ok": false,
        "outcome": "conflict",
        "conflicts": [{ "entity_id": "atom-1" }],
        "error": "Conflict on atom-1 title"
    });
    assert_eq!(
        branch_merge_outcome(&conflict_res).unwrap_err(),
        "Conflict on atom-1 title"
    );
    assert!(branch_merge_outcome(&json!({ "ok": true, "outcome": "merged" })).is_ok());

    let graph = json!({
        "atoms": [{
            "atom_id": "atom-9",
            "kind": "contract",
            "title": "Source fields contract",
            "summary": "Backend emits source_branch and source_session_id",
            "source_branch": "session/s9",
            "source_session_id": "sess-9",
            "authored_at": 1_763_316_000_i64
        }],
        "sessions": [{ "id": "sess-9", "title": "Session 9" }],
        "edges": []
    });
    let inspector = render_node_inspector_html(&graph, "atom-9", &[], "main");
    assert!(inspector.contains("session/s9"));
    assert!(inspector.contains("Session: sess-9"));
    assert!(inspector.contains("UTC"));
    assert!(format_context_snippet(&graph["atoms"][0]).contains("(branch: session/s9)"));

    let pulse = vec![json!({
        "entity_id": "atom-42",
        "entity_kind": "atom",
        "type_name": "ContextAtom",
        "op": "insert",
        "authored_at": 1_763_316_000_i64,
        "source_branch": "session/s4"
    })];
    let pulse_html = render_pulse_list_html(&pulse);
    assert!(pulse_html.contains("data-node-id=\"atom-42\""));
    assert!(pulse_html.contains("ContextAtom"));
    assert!(pulse_html.contains("insert · atom-42"));
    assert!(pulse_html.contains("UTC"));

    let commit_detail = json!({
        "commit": { "id": "c-raw", "branch": "session/s4", "actor_id": "harness", "authored_at": 1_763_316_000_i64 },
        "changes": [{
            "op": "upsert",
            "entity_id": "atom-42",
            "type": { "name": "ContextAtom" },
            "after": { "properties": { "title": "Auth boundary" } }
        }],
        "atoms": []
    });
    let commit_html = render_commit_inspector_html(&commit_detail);
    assert!(commit_html.contains("upsert ContextAtom · Auth boundary"));
    assert!(commit_html.contains("UTC"));

    let merged_session = json!({
        "id": "s-merged",
        "omnigraph": {
            "enabled": true,
            "branch": "session/s-merged",
            "merge_status": "merged",
            "last_error": "post-merge warning",
            "injected_atoms": [{ "id": "a1", "kind": "finding", "title": "Injected" }],
            "published_atoms": ["atom-pub-1"]
        }
    });
    let (visible, branch, meta, atoms, can_merge) = session_bar_summary(&merged_session);
    assert!(visible);
    assert_eq!(branch, "session/s-merged");
    assert!(meta.contains("1 published"));
    assert!(meta.contains("merge: merged"));
    assert!(meta.contains("error: post-merge warning"));
    assert_eq!(atoms.len(), 2);
    assert!(!can_merge);
    assert_eq!(session_bar_error(&merged_session), "post-merge warning");

    m::write(|s| {
        let sess = json!({
            "id": "s-merge-sync",
            "omnigraph": {
                "enabled": true,
                "branch": "session/s-merge-sync",
                "last_error": "stale error"
            }
        });
        s.selected = sess.clone();
        s.sessions = vec![sess];
    });
    apply_merged_session_state(
        "s-merge-sync",
        "session/s-merge-sync",
        &json!({ "ok": true, "outcome": "fast_forward" }),
    );
    m::read(|s| {
        assert_eq!(s.selected["omnigraph"]["merge_status"], "fast_forward");
        assert!(s.selected["omnigraph"]["last_error"].is_null());
        assert_eq!(s.sessions[0]["omnigraph"]["merge_status"], "fast_forward");
        assert!(!session_bar_summary(&s.sessions[0]).4);
    });
}

#[test]
fn branch_switch_resets_pulse_cursor_and_fit_keeps_right_margin() {
    let _ = switch_branch_state("main", true);
    m::set("omnigraph-pulse-cursor", json!({ "value": "cur-main-42" }));
    assert_eq!(
        m::text(&m::data("omnigraph-pulse-cursor"), "value"),
        "cur-main-42"
    );

    assert!(switch_branch_state("session/feature-1", true));
    assert_eq!(m::text(&ui_state(), "branch"), "session/feature-1");
    assert_eq!(m::text(&m::data("omnigraph-pulse-cursor"), "value"), "");

    // Same branch switch keeps existing cursor; distinct branch resets it again.
    m::set("omnigraph-pulse-cursor", json!({ "value": "cur-feat-7" }));
    assert!(!switch_branch_state("session/feature-1", false));
    assert_eq!(
        m::text(&m::data("omnigraph-pulse-cursor"), "value"),
        "cur-feat-7"
    );
    focus_branch("main");
    assert_eq!(m::text(&ui_state(), "branch"), "main");
    assert_eq!(m::text(&m::data("omnigraph-pulse-cursor"), "value"), "");

    let (scale, px, py) = camera_for_fit((1040.0, 540.0), (800.0, 500.0));
    assert!(px >= 36.0);
    assert!(py >= 24.0);
    assert!(px + 1040.0 * scale <= 800.0 - 35.0);
}

#[test]
fn import_parser_detects_bundle_ndjson_single_line_and_errors() {
    let empty = inspect_import_payload("   \n ");
    assert_eq!(empty.format, ImportFormat::Empty);

    let bundle_raw = r#"{
        "format": "harness.omnigraph.bundle.v1",
        "source_branch": "session/auth-v2",
        "atoms": [{"id": "a1", "title": "Token check"}, {"id": "a2", "title": "Cookie rule"}],
        "files": [{"path": "src/auth.rs"}],
        "edges": [{"from": "a1", "to": "a2", "relation": "supports"}],
        "sessions": [{"id": "s1"}]
    }"#;
    let bundle = inspect_import_payload(bundle_raw);
    assert_eq!(bundle.format, ImportFormat::Bundle);
    assert_eq!(bundle.atoms, 2);
    assert_eq!(bundle.files, 1);
    assert_eq!(bundle.edges, 1);
    assert_eq!(bundle.sessions, 1);
    assert_eq!(bundle.source_branch, "session/auth-v2");
    assert!(bundle.bundle.is_some());
    assert!(bundle.raw_text.is_none());

    // Wrapped export API response `{ ok, bundle: {...} }` unwraps cleanly.
    let wrapped_raw = json!({
        "ok": true,
        "bundle": {
            "format": "harness.omnigraph.bundle.v1",
            "source_branch": "main",
            "atoms": [{"id": "a3"}],
            "files": [],
            "edges": []
        }
    })
    .to_string();
    let wrapped = inspect_import_payload(&wrapped_raw);
    assert_eq!(wrapped.format, ImportFormat::Bundle);
    assert_eq!(wrapped.atoms, 1);
    assert_eq!(wrapped.source_branch, "main");

    // Multi-line NDJSON stream
    let ndjson_raw = concat!(
        r#"{"type":"Session","id":"s-1","branch":"session/s1"}"#,
        "\n",
        r#"{"type":"ContextAtom","id":"a-1","title":"Finding","source_branch":"session/s1"}"#,
        "\n",
        r#"{"type":"FileRef","path":"src/lib.rs"}"#,
        "\n",
        r#"{"type":"TouchesFile","from":"a-1","to":"src/lib.rs"}"#
    );
    let ndjson = inspect_import_payload(ndjson_raw);
    assert_eq!(ndjson.format, ImportFormat::Ndjson);
    assert_eq!(ndjson.sessions, 1);
    assert_eq!(ndjson.atoms, 1);
    assert_eq!(ndjson.files, 1);
    assert_eq!(ndjson.edges, 1);
    assert_eq!(ndjson.source_branch, "session/s1");
    assert!(ndjson.raw_text.is_some());

    // Single-line NDJSON object should NOT be misclassified as Bundle.
    let single_ndjson = inspect_import_payload(r#"{"type":"ContextAtom","id":"a-single"}"#);
    assert_eq!(single_ndjson.format, ImportFormat::Ndjson);
    assert_eq!(single_ndjson.atoms, 1);

    // Malformed line reports exact line number.
    let bad_ndjson = inspect_import_payload("{\"type\":\"ContextAtom\"}\n{bad json");
    assert_eq!(bad_ndjson.format, ImportFormat::Invalid);
    assert!(bad_ndjson.error.contains("Line 2"));

    // SymbolRef / TurnSnapshot / nested data.source_branch / {edge, source, target} lines
    let backend_ndjson_raw = concat!(
        r#"{"type":"SymbolRef","id":"sym-1","name":"validateJwt","data":{"source_branch":"session/nested"}}"#,
        "\n",
        r#"{"type":"TurnSnapshot","id":"turn-1","data":{"source_branch":"session/nested"}}"#,
        "\n",
        r#"{"type":"ContextAtom","id":"a-real","data":{"title":"Verified JWT flow","source_branch":"session/nested"}}"#,
        "\n",
        r#"{"type":"FileRef","path":"src/jwt.rs"}"#,
        "\n",
        r#"{"edge":"relates_to","source":"a-real","target":"sym-1"}"#
    );
    let backend_ndjson = inspect_import_payload(backend_ndjson_raw);
    assert_eq!(backend_ndjson.format, ImportFormat::Ndjson);
    assert_eq!(backend_ndjson.atoms, 1);
    assert_eq!(backend_ndjson.files, 1);
    assert_eq!(backend_ndjson.edges, 1);
    assert_eq!(backend_ndjson.sessions, 0);
    assert_eq!(backend_ndjson.source_branch, "session/nested");

    let single_edge = inspect_import_payload(r#"{"edge":"relates_to","source":"a","target":"b"}"#);
    assert_eq!(single_edge.format, ImportFormat::Ndjson);
    assert_eq!(single_edge.atoms, 0);
    assert_eq!(single_edge.edges, 1);
}

#[test]
fn export_and_import_payloads_and_download_urls_format_cleanly() {
    let req = export_request_body("proj-1", "session/s1", "sess-1", "bundle", "task", true);
    assert_eq!(
        req,
        json!({
            "project_id": "proj-1",
            "branch": "session/s1",
            "session_id": "sess-1",
            "format": "bundle",
            "scope": "task",
            "include_inherited": true
        })
    );

    // Task scope with no explicit branch omits branch so the backend resolves it by session_id.
    let task_req_no_branch = export_request_body("proj-1", "", "sess-1", "bundle", "task", false);
    assert!(task_req_no_branch.get("branch").is_none());
    assert_eq!(task_req_no_branch["session_id"], "sess-1");
    let task_url_no_branch = export_download_url("proj-1", "", "sess-1", "bundle", "task", false);
    assert!(!task_url_no_branch.contains("branch="));

    let url = export_download_url("proj 1", "session/a&b", "sess-1", "ndjson", "task", false);
    assert!(url.starts_with("/api/omnigraph/export?"));
    assert!(url.contains("project_id=proj%201"));
    assert!(url.contains("branch=session%2Fa%26b"));
    assert!(url.contains("session_id=sess-1"));
    assert!(url.contains("format=ndjson"));
    assert!(url.contains("scope=task"));
    assert!(url.contains("include_inherited=false"));
    assert!(url.contains("download=1"));

    // Branch scope omits session_id even if passed.
    let branch_url = export_download_url("p1", "main", "ignored", "bundle", "branch", true);
    assert!(!branch_url.contains("session_id="));

    let summary = export_preview_summary(
        &json!({
            "filename": "task-sess1.json",
            "source_branch": "session/s1",
            "counts": { "atoms": 4, "files": 2, "edges": 6, "sessions": 1 },
            "bundle": { "format": "harness.omnigraph.bundle.v1", "atoms": [] }
        }),
        "bundle",
    );
    assert_eq!(summary.0, "4 atoms · 2 files · 6 edges · 1 sessions");
    assert_eq!(summary.1, "task-sess1.json");
    assert_eq!(summary.2, "session/s1");
    assert!(summary.3.contains("harness.omnigraph.bundle.v1"));

    let import_body = import_request_body(
        "proj-1",
        r#"{"format":"harness.omnigraph.bundle.v1","atoms":[{"id":"a1"}]}"#,
        "session/target",
        "main",
        true,
        "replace",
        true,
        "sess-9",
    )
    .unwrap();
    assert_eq!(import_body["target_branch"], "session/target");
    assert_eq!(import_body["from_branch"], "main");
    assert_eq!(import_body["create_branch"], true);
    assert_eq!(import_body["mode"], "replace");
    assert_eq!(import_body["inject_into_sessions"], true);
    assert_eq!(import_body["session_id"], "sess-9");
    assert_eq!(import_body["target_session_ids"], json!(["sess-9"]));
    assert!(import_body["bundle"].is_object());
    assert!(import_body.get("raw_text").is_none());

    let ndjson_import = import_request_body(
        "proj-1",
        "{\"type\":\"ContextAtom\",\"id\":\"a1\"}",
        "",
        "",
        false,
        "merge",
        false,
        "",
    )
    .unwrap();
    assert_eq!(ndjson_import["target_branch"], "main");
    assert!(ndjson_import["raw_text"].is_string());
    assert!(ndjson_import.get("bundle").is_none());
}

#[test]
fn multi_task_link_payload_state_sync_topology_and_peer_strips_work_cleanly() {
    let sess_payload = session_payload_with_links(
        true,
        "auto",
        "",
        "main",
        "auth",
        true,
        false,
        &["s1".into(), " s2 ".into(), "s1".into(), "".into()],
        " shared/sprint-auth ",
    )
    .unwrap();
    assert_eq!(sess_payload["linked_session_ids"], json!(["s1", "s2"]));
    assert_eq!(sess_payload["shared_space"], "shared/sprint-auth");

    assert!(
        link_sessions_request_body("p1", &["s1".into()], "shared_branch", "", "sprint", "")
            .is_err()
    );

    let mutual = link_sessions_request_body(
        "p1",
        &["s1".into(), "s2".into(), "s1".into()],
        "shared_branch",
        "",
        "sprint-auth",
        "shared/sprint-auth",
    )
    .unwrap();
    assert_eq!(mutual["mode"], "shared_branch");
    assert_eq!(mutual["session_ids"], json!(["s1", "s2"]));
    assert_eq!(mutual["space_name"], "sprint-auth");
    assert_eq!(mutual["target_branch"], "shared/sprint-auth");
    assert_eq!(mutual["inject_live"], true);

    assert!(link_sessions_request_body("p1", &["s1".into()], "broadcast", "s1", "", "").is_err());
    let broadcast = link_sessions_request_body(
        "p1",
        &["s1".into(), "s2".into(), "s3".into()],
        "broadcast",
        "s1",
        "",
        "",
    )
    .unwrap();
    assert_eq!(broadcast["mode"], "broadcast");
    assert_eq!(broadcast["source_session_id"], "s1");

    // Session state synchronization updates s.sessions and s.selected.
    m::write(|s| {
        s.sid = "s1".into();
        s.selected = json!({ "id": "s1", "title": "Auth API" });
        s.sessions = vec![
            json!({ "id": "s1", "title": "Auth API", "project_id": "p1", "status": "running" }),
            json!({ "id": "s2", "title": "Auth UI", "project_id": "p1", "status": "completed" }),
        ];
    });
    sync_sessions_from_response(&json!({
        "ok": true,
        "sessions": [
            {
                "id": "s1",
                "title": "Auth API",
                "project_id": "p1",
                "status": "running",
                "omnigraph": {
                    "enabled": true,
                    "branch": "shared/sprint-auth",
                    "shared_space": "shared/sprint-auth",
                    "linked_session_ids": ["s2"],
                    "injected_atoms": [{"id": "a1", "title": "JWT contract"}]
                }
            },
            {
                "id": "s2",
                "title": "Auth UI",
                "project_id": "p1",
                "status": "completed",
                "omnigraph": {
                    "enabled": true,
                    "branch": "shared/sprint-auth",
                    "shared_space": "shared/sprint-auth",
                    "linked_session_ids": ["s1"]
                }
            }
        ]
    }));

    let (selected, all) = m::read(|s| (s.selected.clone(), s.sessions.clone()));
    assert_eq!(selected["omnigraph"]["shared_space"], "shared/sprint-auth");
    let (show_peers, peers_html) = render_session_peers_html(&selected, &all);
    assert!(show_peers);
    assert!(peers_html.contains("Space: shared/sprint-auth"));
    assert!(peers_html.contains("data-action=\"omnigraph-open-session\""));
    assert!(peers_html.contains("data-session-id=\"s2\""));
    assert!(peers_html.contains("Auth UI"));

    let chosen = ["s1".to_owned(), "s2".to_owned()]
        .into_iter()
        .collect::<BTreeSet<_>>();
    let topo_mutual = render_share_topology_html(&all, &chosen, "shared_branch", "", "sprint-auth");
    assert!(topo_mutual.contains("Mutual Shared Space"));
    assert!(topo_mutual.contains("shared/sprint-auth"));
    assert!(topo_mutual.contains("Auth API"));
    assert!(topo_mutual.contains("Auth UI"));

    let topo_broadcast = render_share_topology_html(&all, &chosen, "broadcast", "s1", "");
    assert!(topo_broadcast.contains("Broadcast mode"));
    assert!(topo_broadcast.contains("data-role=\"source\""));

    let cards = render_share_task_list_html(&all, &chosen, "shared_branch", "", "");
    assert!(cards.contains("omni-share-task-card"));
    assert!(cards.contains("data-selected=\"true\""));

    let (show_panel_peers, linked_count, panel_html) =
        render_panel_peer_checklist_html(&all, "", "p1", &chosen, false);
    assert!(show_panel_peers);
    assert_eq!(linked_count, 2);
    assert!(panel_html.contains("data-action=\"omnigraph-peer-choice\""));

    // Slugification normalizes free-text shared space names cleanly.
    assert_eq!(resolved_space_branch(""), "shared/context");
    assert_eq!(resolved_space_branch("Team Auth"), "shared/team-auth");
    assert_eq!(
        resolved_space_branch("shared/Team Auth"),
        "shared/team-auth"
    );
    assert_eq!(
        resolved_space_branch("shared/sprint-auth"),
        "shared/sprint-auth"
    );
    let slugged_session = session_payload_with_links(
        true,
        "auto",
        "",
        "main",
        "",
        true,
        false,
        &["s1".into()],
        "Team Auth",
    )
    .unwrap();
    assert_eq!(slugged_session["shared_space"], "shared/team-auth");

    // Omitting target_branch allows the backend to slugify space_name or keep broadcast source_branch.
    let space_only = link_sessions_request_body(
        "p1",
        &["s1".into(), "s2".into()],
        "shared_branch",
        "",
        "Team Auth",
        "",
    )
    .unwrap();
    assert_eq!(space_only["space_name"], "Team Auth");
    assert!(space_only.get("target_branch").is_none());
    let broadcast_clean =
        link_sessions_request_body("p1", &["s1".into(), "s2".into()], "broadcast", "s1", "", "")
            .unwrap();
    assert!(broadcast_clean.get("space_name").is_none());
    assert!(broadcast_clean.get("target_branch").is_none());

    // Filter parity between rendered share task cards and shared helper used by Select All.
    let filtered_cards =
        render_share_task_list_html(&all, &chosen, "shared_branch", "", "sprint-auth");
    assert!(filtered_cards.contains("Auth API"));
    assert!(filtered_cards.contains("Auth UI"));
    assert!(share_session_matches_filter(&all[0], "sprint-auth"));
    assert!(!share_session_matches_filter(&all[0], "nonexistent"));
}

#[test]
fn a_git_commit_an_atom_names_opens_the_code_map_apart_from_snapshots() {
    let atom = json!({
        "id": "atom-git",
        "kind": "finding",
        "title": "Regression from 3f2a9c1",
        "summary": "Fixed again in (deadbeef0123); see commit:0a1b2c3d4e5f60718293a4b5c6d7e8f901234567.",
        "tags": ["sha 7e57ab1"],
        "commit_id": "c-0042",
        "branch": "main"
    });
    assert_eq!(
        git_commits_named(&atom),
        vec![
            "7e57ab1".to_owned(),
            "3f2a9c1".to_owned(),
            "deadbeef0123".to_owned(),
            "0a1b2c3d4e5f60718293a4b5c6d7e8f901234567".to_owned(),
        ]
    );
    let graph = json!({"atoms": [atom]});
    let html = render_node_inspector_html(&graph, "atom-git", &[], "main");
    assert!(
        html.contains("data-action=\"omnigraph-open-git-commit\" data-payload=\"{&quot;sha&quot;:&quot;3f2a9c1&quot;}\" title=\"Open Git commit 3f2a9c1 in the Code Map\">Git: 3f2a9c1</button>"),
        "{html}"
    );
    assert!(html.contains(">Git: 0a1b2c3d4e</button>"), "{html}");
    // The Omnigraph snapshot keeps its own action.
    assert!(
        html.contains("data-action=\"omnigraph-select-commit\" data-commit-id=\"c-0042\""),
        "{html}"
    );
    assert!(!html.contains("&quot;sha&quot;:&quot;c-0042"), "{html}");
    // Words that only look like hex: ids, paths, numbers, words, capitals.
    let lookalikes = json!({"body": "Not commits: MEM-20261117-a1b2c3d4, src/abc1234.rs, 2026111712, facade, DEADBEEF1, abc123, 11111111-2222-4333-8444-555555555555, x9abcdef12, abc1234.rs and abc1234-fix."});
    assert!(
        git_commits_named(&lookalikes).is_empty(),
        "{:?}",
        git_commits_named(&lookalikes)
    );
    assert_eq!(
        git_commits_named(&json!({"git_commit": " abcdef1234 "})),
        vec!["abcdef1234"]
    );
    // At most four in all, and only a real id.
    let explicit =
        json!({"git_commit": "ABCDEF1", "title": "a1b2c3d e4f5a6b 0000000a 1111111b 2222222c"});
    assert_eq!(
        git_commits_named(&explicit),
        vec!["a1b2c3d", "e4f5a6b", "0000000a", "1111111b"]
    );
    assert!(is_git_sha("abcdef1"));
    assert!(!is_git_sha("abcdef"));
    assert!(!is_git_sha("ABCDEF1"));
    assert!(!is_git_sha(&"a".repeat(41)));
}

#[test]
fn inspector_and_search_render_provenance_jump_links_and_promote_button() {
    let graph = json!({
        "atoms": [
            {
                "id": "atom-unverified",
                "kind": "insight",
                "title": "Draft session finding",
                "summary": "Observed in run",
                "branch": "session/s1",
                "origin_layer": "session-draft",
                "authority": "inferred",
                "verified": false
            },
            {
                "id": "atom-linked",
                "kind": "decision",
                "title": "Enforce token expiry",
                "summary": "Validated against Brain and Bank",
                "branch": "knowledge/canonical",
                "origin_layer": "project-brain",
                "authority": "verified",
                "privacy": "team",
                "verified": true,
                "brain_record_id": "12345678-abcd-ef00-9999-000000000001",
                "memory_id": "MEM-20261117-abcdef01-token-expiry",
                "record_revision": 4
            }
        ]
    });

    let unverified_html = render_node_inspector_html(&graph, "atom-unverified", &[], "main");
    assert!(unverified_html.contains("Unverified reference"));
    assert!(unverified_html.contains("session-draft"));
    assert!(unverified_html.contains("authority: inferred"));
    assert!(unverified_html.contains("data-action=\"omnigraph-promote-atom\""));
    assert!(unverified_html.contains("Promote to Brain…"));
    assert!(!unverified_html.contains("id=\"omni-promote-atom\" data-action=\"omnigraph-promote-atom\" data-atom-id=\"atom-unverified\" data-branch=\"session/s1\" data-payload=\"{&quot;atom_id&quot;:&quot;atom-unverified&quot;,&quot;branch&quot;:&quot;session/s1&quot;}\" disabled"));
    assert!(!unverified_html.contains("data-action=\"omnigraph-open-brain\""));
    assert!(!unverified_html.contains("data-action=\"omnigraph-open-memory\""));

    let linked_html = render_node_inspector_html(&graph, "atom-linked", &[], "main");
    assert!(linked_html.contains("Verified"));
    assert!(!linked_html.contains("Unverified reference"));
    assert!(linked_html.contains("project-brain"));
    assert!(linked_html.contains("authority: verified"));
    assert!(linked_html.contains("privacy: team"));
    assert!(linked_html.contains("Revision: v4"));
    assert!(linked_html.contains("data-action=\"omnigraph-open-brain\""));
    assert!(linked_html.contains("Brain: 12345678"));
    assert!(linked_html.contains("data-action=\"omnigraph-open-memory\""));
    assert!(linked_html.contains("Memory: MEM-20261117-abcdef01-token-expiry"));
    assert!(linked_html.contains("Promoted to Brain"));
    assert!(linked_html.contains("id=\"omni-promote-status\""));
    assert!(linked_html.contains("id=\"omni-promote-error\""));

    let hits = json!({
        "query": "token",
        "hits": [
            { "atom": graph["atoms"][0], "score": 1.2, "matched_via": "bm25", "neighbors": [] },
            { "atom": graph["atoms"][1], "score": 3.4, "matched_via": "bm25", "neighbors": [] }
        ]
    });
    let hits_html = render_search_hits_html(&hits);
    assert!(hits_html.contains("Unverified reference"));
    assert!(hits_html.contains("Verified"));
    assert!(hits_html.contains("data-action=\"omnigraph-open-brain\""));
    assert!(hits_html.contains("data-action=\"omnigraph-open-memory\""));
}

#[test]
fn session_bar_metrics_measure_capsule_chars_and_verified_counts() {
    let session = json!({
        "id": "s-bar",
        "omnigraph": {
            "enabled": true,
            "branch": "session/s-bar",
            "capsule_text": "x".repeat(1425),
            "injected_atoms": [
                { "id": "a1", "title": "Brain decision", "verified": true, "authority": "verified" },
                { "id": "a2", "title": "Bank constraint", "authority": "verified" },
                { "id": "a3", "title": "Session insight", "verified": false, "origin_layer": "session-draft" },
                { "id": "a4", "title": "Legacy atom" }
            ]
        }
    });
    let m = session_bar_metrics(&session);
    assert!(m.enabled);
    assert_eq!(m.branch, "session/s-bar");
    assert_eq!(m.capsule_chars, 1425);
    assert_eq!(m.capsule_limit, 6000);
    assert_eq!(
        format!(
            "{} / {} chars",
            format_count(m.capsule_chars),
            format_count(m.capsule_limit)
        ),
        "1,425 / 6,000 chars"
    );
    assert_eq!(m.verified_count, 2);
    assert_eq!(m.unverified_count, 2);
    assert_eq!(m.atom_count, 4);
}

#[test]
fn promote_uses_active_branch_over_source_branch_and_suppresses_cross_project_session() {
    let merged_atom = json!({
        "id": "atom-merged",
        "kind": "finding",
        "title": "Merged finding",
        "source_branch": "session/deleted-branch",
        "verified": false
    });
    let graph = json!({ "atoms": [merged_atom.clone()] });

    let on_main = render_node_inspector_html(&graph, "atom-merged", &[], "main");
    assert!(on_main.contains("id=\"omni-promote-atom\" data-action=\"omnigraph-promote-atom\" data-atom-id=\"atom-merged\" data-branch=\"main\""));
    assert!(on_main.contains("&quot;branch&quot;:&quot;main&quot;"));

    let on_canonical =
        render_node_inspector_html(&graph, "atom-merged", &[], "knowledge/canonical");
    assert!(on_canonical.contains("data-branch=\"knowledge/canonical\""));

    assert_eq!(
        promote_target_branch(&merged_atom, ""),
        "session/deleted-branch"
    );

    let selected_p1 = json!({ "id": "sess-1", "project_id": "proj-1" });
    assert_eq!(
        matching_project_session_id(&selected_p1, "sess-1", "proj-1").as_deref(),
        Some("sess-1")
    );
    assert_eq!(
        matching_project_session_id(&selected_p1, "sess-1", "proj-2"),
        None
    );
    assert_eq!(
        matching_project_session_id(&json!({}), "sess-1", "proj-1"),
        None
    );
    assert_eq!(
        matching_project_session_id(&selected_p1, "", "proj-1"),
        None
    );
}

#[test]
fn followup_payload_sends_only_changed_pull_query_and_auto_publish() {
    let session = json!({"id": "s1", "omnigraph": {"enabled": true, "branch": "session/s1", "pull_branches": ["main"], "query": "auth tokens", "auto_publish": true, "auto_merge": false}});
    // Unchanged fields send nothing, and blank pull branches mean main.
    assert_eq!(
        followup_payload_from_fields(&session, "main", " auth tokens ", true),
        None
    );
    assert_eq!(
        followup_payload_from_fields(&session, "", "auth tokens", true),
        None
    );
    assert_eq!(
        followup_payload_from_fields(&session, "main, session/auth-fix, main", "", false),
        Some(
            json!({"pull_branches": ["main", "session/auth-fix"], "query": null, "auto_publish": false})
        )
    );
    assert_eq!(
        followup_payload_from_fields(&session, "main", "refresh flow", true),
        Some(json!({"query": "refresh flow"}))
    );
    // A follow-up never switches Omnigraph on for a session without it.
    for off in [
        json!({"id": "s2", "omnigraph": null}),
        json!({"id": "s3", "omnigraph": {"enabled": false, "branch": "session/s3"}}),
    ] {
        assert_eq!(
            followup_payload_from_fields(&off, "main, dev", "x", false),
            None
        );
    }
}

#[test]
fn session_atoms_show_ten_then_a_more_toggle() {
    use crate::test_support::{assert_no_inline_hex_colors, payloads};
    let atoms = (0..13)
        .map(|n| json!({"id": format!("atom-{n}"), "kind": "finding", "title": format!("Atom <{n}>"), "verified": n % 2 == 0}))
        .collect::<Vec<_>>();
    let folded = render_session_atoms_html(&atoms, "session/s1", false);
    assert_eq!(
        folded.matches("class=\"omni-session-atom-chip\"").count(),
        10
    );
    assert!(folded.contains("Atom &lt;9&gt;"), "{folded}");
    assert!(!folded.contains("atom-10"), "{folded}");
    assert!(
        folded.contains("aria-expanded=\"false\">+3 more</button>"),
        "{folded}"
    );
    assert_eq!(
        payloads(&folded, "omnigraph-session-atoms-toggle"),
        vec![json!({"expanded": true})]
    );
    let open = render_session_atoms_html(&atoms, "session/s1", true);
    assert_eq!(open.matches("class=\"omni-session-atom-chip\"").count(), 13);
    assert!(
        open.contains("aria-expanded=\"true\">Show fewer</button>"),
        "{open}"
    );
    assert_eq!(
        payloads(&open, "omnigraph-session-atoms-toggle"),
        vec![json!({"expanded": false})]
    );
    // Ten or fewer need no toggle, and none shows the empty note.
    assert!(
        !render_session_atoms_html(&atoms[..10], "main", false).contains("omni-session-atom-more")
    );
    assert!(render_session_atoms_html(&[], "main", false).contains("No shared atoms injected yet."));
    assert_no_inline_hex_colors(&folded);
    assert_no_inline_hex_colors(&open);
}

#[test]
fn inspector_renders_symbol_ref_atom_symbol_badges_and_code_map_jumps() {
    use crate::test_support::{assert_no_inline_hex_colors, payloads};
    let graph = json!({
        "active_branch": "session/s1",
        "atoms": [
            {
                "id": "atom-1",
                "kind": "finding",
                "title": "Token verifier skips leeway",
                "branch": "session/s1",
                "symbols": ["src/Auth/Token.php::Token::verify"]
            }
        ],
        "files": [
            {"id": "src/Auth/Token.php", "path": "src/Auth/Token.php"}
        ],
        "symbols": [
            {
                "id": "src/Auth/Token.php::Token::verify",
                "fqn": "src/Auth/Token.php::Token::verify",
                "file_path": "src/Auth/Token.php",
                "kind": "method",
                "name": "verify",
                "owner": "Token",
                "label": "Token::verify",
                "atom_ids": ["atom-1"]
            }
        ],
        "symbol_edges": [
            {
                "id": "se-1",
                "kind": "mentions_symbol",
                "relation": "mentions_symbol",
                "from": "atom-1",
                "to": "src/Auth/Token.php::Token::verify"
            }
        ]
    });

    let sym_html = render_node_inspector_html(
        &graph,
        "src/Auth/Token.php::Token::verify",
        &[],
        "session/s1",
    );
    assert!(sym_html.contains("data-kind=\"symbol\""), "{sym_html}");
    assert!(
        sym_html.contains("omni-symbol-kind-pill\">method</span>"),
        "{sym_html}"
    );
    assert!(
        sym_html.contains("src/Auth/Token.php::Token::verify"),
        "{sym_html}"
    );
    assert!(
        sym_html.contains("Token verifier skips leeway"),
        "{sym_html}"
    );
    assert!(sym_html.contains("Focus File Anchor"), "{sym_html}");
    let sym_jumps = payloads(&sym_html, "omnigraph-open-code-map");
    assert_eq!(sym_jumps.len(), 1);
    assert_eq!(sym_jumps[0]["path"], "src/Auth/Token.php");
    assert_eq!(sym_jumps[0]["fqn"], "src/Auth/Token.php::Token::verify");
    assert_no_inline_hex_colors(&sym_html);

    let atom_html = render_node_inspector_html(&graph, "atom-1", &[], "session/s1");
    assert!(atom_html.contains("Mentioned symbols"), "{atom_html}");
    assert!(atom_html.contains("omni-symbol-badge"), "{atom_html}");
    assert!(atom_html.contains("Token::verify"), "{atom_html}");
    let atom_jumps = payloads(&atom_html, "omnigraph-open-code-map");
    assert_eq!(atom_jumps.len(), 1);
    assert_eq!(atom_jumps[0]["path"], "src/Auth/Token.php");
    assert_no_inline_hex_colors(&atom_html);

    let file_html = render_node_inspector_html(&graph, "src/Auth/Token.php", &[], "session/s1");
    assert!(
        file_html.contains("data-action=\"inspect-file-graph\""),
        "{file_html}"
    );
    assert!(file_html.contains("Mentioned symbols"), "{file_html}");
    assert!(file_html.contains("Token::verify (method)"), "{file_html}");
    assert_no_inline_hex_colors(&file_html);
}

#[test]
fn resolve_code_map_target_path_handles_direct_path_and_fqn_fallback() {
    assert_eq!(
        resolve_code_map_target_path(&json!("src/Repo.php")).as_deref(),
        Some("src/Repo.php")
    );
    assert_eq!(
        resolve_code_map_target_path(&json!({"file_path": " app/Auth.php "})).as_deref(),
        Some("app/Auth.php")
    );
    assert_eq!(
        resolve_code_map_target_path(&json!({"fqn": "src/Auth/Token.php::Token::verify"}))
            .as_deref(),
        Some("src/Auth/Token.php")
    );
    assert_eq!(
        resolve_code_map_target_path(&json!({"fqn": "App\\Auth\\Token::verify"})),
        None
    );
    assert_eq!(resolve_code_map_target_path(&json!({})), None);
}

#[test]
fn pagination_summary_bar_and_page_merge_deduplicate_nodes_and_edges() {
    use crate::test_support::assert_no_inline_hex_colors;
    let page1 = json!({
        "active_branch": "main",
        "total_atoms": 3,
        "limit": 2,
        "offset": 0,
        "has_more": true,
        "next_cursor": "cursor-2",
        "atoms": [{"id": "a1", "title": "A1"}, {"id": "a2", "title": "A2"}],
        "files": [{"id": "src/A.php", "path": "src/A.php"}],
        "symbols": [{"id": "src/A.php::foo", "fqn": "src/A.php::foo"}],
        "sessions": [{"id": "s1"}],
        "edges": [{"id": "e1", "from": "a1", "to": "src/A.php", "relation": "touches_file"}],
        "symbol_edges": [{"id": "se1", "from": "a1", "to": "src/A.php::foo", "relation": "mentions_symbol"}]
    });
    let summary = graph_pagination_summary(&page1);
    assert_eq!(summary.shown_atoms, 2);
    assert_eq!(summary.total_atoms, 3);
    assert_eq!(summary.symbol_count, 1);
    assert!(summary.has_more);
    assert_eq!(summary.next_cursor, "cursor-2");
    assert_eq!(summary.next_offset, 2);

    let (visible, bar_html) = render_pagination_bar_html(&page1);
    assert!(visible);
    assert!(bar_html.contains("Showing 2 of 3 atoms"), "{bar_html}");
    assert!(bar_html.contains("Load more (+1 remaining)"), "{bar_html}");
    assert!(
        bar_html.contains("data-action=\"omnigraph-load-more\""),
        "{bar_html}"
    );
    assert_no_inline_hex_colors(&bar_html);

    let page2 = json!({
        "total_atoms": 3,
        "limit": 2,
        "offset": 2,
        "has_more": false,
        "truncated": false,
        "next_cursor": null,
        "atoms": [{"id": "a2", "title": "A2-dup"}, {"id": "a3", "title": "A3"}],
        "files": [{"id": "src/A.php", "path": "src/A.php"}, {"id": "src/B.php", "path": "src/B.php"}],
        "symbols": [{"id": "src/A.php::foo", "fqn": "src/A.php::foo"}],
        "sessions": [{"id": "s1"}],
        "edges": [{"id": "e1", "from": "a1", "to": "src/A.php", "relation": "touches_file"}, {"id": "e2", "from": "a3", "to": "src/B.php", "relation": "touches_file"}],
        "symbol_edges": [{"id": "se1", "from": "a1", "to": "src/A.php::foo", "relation": "mentions_symbol"}]
    });
    let merged = merge_graph_page(page1, page2);
    assert_eq!(merged["atoms"].as_array().unwrap().len(), 3);
    assert_eq!(merged["files"].as_array().unwrap().len(), 2);
    assert_eq!(merged["symbols"].as_array().unwrap().len(), 1);
    assert_eq!(merged["edges"].as_array().unwrap().len(), 2);
    assert_eq!(merged["symbol_edges"].as_array().unwrap().len(), 1);
    assert!(merged["next_cursor"].is_null());
    assert_eq!(merged["has_more"], false);
    assert!(graph_pagination_summary(&merged).next_cursor.is_empty());

    let (visible_after, _) = render_pagination_bar_html(&merged);
    assert!(!visible_after);
}

#[test]
fn resolve_code_map_target_preserves_file_path_and_symbol_fqn() {
    assert_eq!(
        resolve_code_map_target(&json!({
            "file_path": "src/Order/PlaceOrder.php",
            "symbol_fqn": "App\\Order\\PlaceOrder::execute"
        })),
        Some((
            "src/Order/PlaceOrder.php".to_owned(),
            Some("App\\Order\\PlaceOrder::execute".to_owned())
        ))
    );
    assert_eq!(
        resolve_code_map_target(&json!({"fqn": "src/Auth/Token.php::Token::verify"})),
        Some((
            "src/Auth/Token.php".to_owned(),
            Some("src/Auth/Token.php::Token::verify".to_owned())
        ))
    );
}

#[test]
fn camera_sanitation_css_escaping_pan_into_view_and_nav_history_behave_safely() {
    m::set(
        "omnigraph-camera",
        json!({
            "scale": "not-a-number",
            "pan_x": null,
            "pan_y": f64::NAN,
            "plane_w": -50.0,
            "plane_h": 480.0,
            "fit_mode": false
        }),
    );
    let cam = load_camera();
    assert_eq!(cam.scale, 1.0);
    assert_eq!(cam.pan_x, 16.0);
    assert_eq!(cam.pan_y, 16.0);
    assert_eq!(cam.plane_w, 0.0);
    assert_eq!(cam.plane_h, 480.0);
    m::take("omnigraph-camera");

    assert_eq!(
        crate::dom::escape_css_attr_value("App\\Auth\\\"Token\"\\Service"),
        "App\\\\Auth\\\\\\\"Token\\\"\\\\Service"
    );

    let radar = clamped_radar_rect(
        graph_viewport::Camera::new(-10000.0, -10000.0, 2.0),
        (920.0, 580.0),
        (600.0, 400.0),
        16.0,
    )
    .expect("radar rect");
    assert!(radar.0 + radar.2 <= 600.01);
    assert!(radar.1 + radar.3 <= 400.01);

    let mut view_cam = OmnigraphCamera {
        scale: 1.0,
        pan_x: 0.0,
        pan_y: 0.0,
        plane_w: 2000.0,
        plane_h: 2000.0,
        fit_mode: false,
    };
    let next = graph_viewport::pan_to_keep_rect_in_view(
        view_cam.to_viewport(),
        (1500.0, 1200.0, 200.0, 80.0),
        (800.0, 600.0),
        24.0,
    )
    .expect("panned into view");
    view_cam.pan_x = next.tx;
    view_cam.pan_y = next.ty;
    assert!(1500.0 * view_cam.scale + view_cam.pan_x >= 0.0);
    assert!(1700.0 * view_cam.scale + view_cam.pan_x <= 800.0);

    let mut hist = graph_viewport::CameraSelectionHistory::new(32);
    hist.push(omni_nav_entry_from(
        Some("a1".to_owned()),
        "main",
        &view_cam,
    ));
    hist.push(omni_nav_entry_from(
        Some("a2".to_owned()),
        "main",
        &view_cam,
    ));
    assert!(hist.can_back());
    assert!(!hist.can_forward());
}

#[test]
fn pr106_p0_p1_fixes_drift_poll_status_search_cross_graph_and_hotkeys() {
    // P0 #1: resolve_workspace_drift_docs reads scoped Dependency Map keys (`graph-map-*`, `session-graph-*`)
    //        instead of dead `graph-changes`/`graph-impact` keys and validates active project + work target.
    m::set(
        "graph-map-changes",
        json!({
            "scope_key": "project:proj-1",
            "project": "proj-1",
            "target": "work",
            "data": { "files": [{ "path": "src/auth.rs", "broken_risk": true }] }
        }),
    );
    m::set(
        "session-graph-impact",
        json!({
            "key": "project:proj-1|src/auth.rs|work",
            "data": { "suspects": ["src/login.rs"] }
        }),
    );
    let (ch, imp) = resolve_workspace_drift_docs("proj-1");
    assert_eq!(ch.unwrap()["files"][0]["path"], "src/auth.rs");
    assert_eq!(imp.unwrap()["suspects"][0], "src/login.rs");
    // Mismatched project must be ignored cleanly.
    assert_eq!(resolve_workspace_drift_docs("other-proj"), (None, None));
    m::take("graph-map-changes");
    m::take("session-graph-impact");

    // P0 #2: classify_changes_poll_response refreshes on `reset: true` or `snapshot_commit_id != head_commit`
    //        even when events array is empty or force=true, and extracts baseline records.
    let reset_resp = json!({
        "cursor": "c-20",
        "caught_up": true,
        "reset": true,
        "snapshot_commit_id": "commit-new",
        "events": [],
        "baseline": {
            "records": [{
                "id": "atom-base-1",
                "entity_type": { "name": "ContextAtom" },
                "properties": { "title": "Recovered atom", "kind": "finding" }
            }]
        }
    });
    let dec = classify_changes_poll_response("commit-old", &reset_resp, true);
    assert!(dec.reset);
    assert!(dec.head_changed);
    assert!(dec.should_refresh_graph);
    assert!(dec.has_baseline);
    let patch = extract_baseline_graph_patch(&reset_resp["baseline"]).unwrap();
    assert_eq!(patch["atoms"][0]["title"], "Recovered atom");

    let head_advance_resp = json!({
        "cursor": "c-21",
        "reset": false,
        "snapshot_commit_id": "commit-next",
        "events": []
    });
    assert!(
        classify_changes_poll_response("commit-old", &head_advance_resp, true).should_refresh_graph
    );
    assert!(
        !classify_changes_poll_response(
            "commit-old",
            &json!({
                "cursor": "c-21",
                "reset": false,
                "snapshot_commit_id": "commit-old",
                "events": []
            }),
            true
        )
        .should_refresh_graph
    );

    // P0 #3: Non-destructive degraded studio visibility + platform/version dynamic install copy + error pill
    let disconnected_status = json!({
        "connected": false,
        "state": "error",
        "error": "sidecar timed out",
        "binary_installed": false,
        "target_version": "v0.16.0",
        "platform": { "os": "macos", "arch": "arm64", "auto_download_supported": false }
    });
    let (show_onboard, show_studio, banner) = studio_visibility_plan(&disconnected_status, true);
    assert!(
        !show_onboard,
        "must not bounce back to onboarding while graph is loaded"
    );
    assert!(
        show_studio,
        "studio must remain visible with degraded banner"
    );
    assert!(banner.unwrap().contains("sidecar timed out"));
    let (onb_empty, stu_empty, _) = studio_visibility_plan(&disconnected_status, false);
    assert!(onb_empty);
    assert!(!stu_empty);

    let (btn_lbl, detail_txt, auto_ok) = local_install_copy(&disconnected_status);
    assert!(!auto_ok);
    assert!(btn_lbl.contains("v0.16.0"));
    assert!(detail_txt.contains("macos/arm64"));
    let degraded =
        degraded_status_from_error(&json!({"connected": true, "mode": "local"}), "503 down");
    assert_eq!(degraded["connected"], false);
    assert_eq!(degraded["state"], "error");
    assert_eq!(degraded["error"], "503 down");

    // P1 #6: Search subgraph beyond page-1 merges into omnigraph-graph without wiping next_cursor.
    let page1 = json!({
        "atoms": [{ "id": "a1", "title": "First" }],
        "next_cursor": "cursor-page-2",
        "has_more": true
    });
    let search_res = json!({
        "hits": [{
            "atom": { "id": "a999", "title": "Deep Hit" },
            "files": ["src/deep.rs"],
            "neighbors": [{ "kind": "symbol", "id": "src/deep.rs::Deep::run" }]
        }],
        "subgraph": {
            "atoms": [{ "id": "a999", "title": "Deep Hit" }],
            "files": [{ "id": "src/deep.rs", "path": "src/deep.rs" }],
            "symbols": [{ "id": "src/deep.rs::Deep::run", "symbol_id": "sym:deep:1", "fqn": "src/deep.rs::Deep::run" }],
            "edges": [{ "id": "e-999", "from": "a999", "to": "src/deep.rs", "relation": "touches_file" }]
        }
    });
    let (merged, changed) = merge_search_subgraph_into_graph(page1, &search_res);
    assert!(changed);
    assert_eq!(merged["atoms"].as_array().unwrap().len(), 2);
    assert_eq!(
        merged["next_cursor"], "cursor-page-2",
        "cursor must stay intact"
    );
    let hit_ids = collect_search_hit_ids(&search_res);
    assert!(hit_ids.contains(&"a999".to_owned()));
    assert!(hit_ids.contains(&"src/deep.rs::Deep::run".to_owned()));

    // P1 #7: Cross-graph jumps (`symbol_id`, `atom_id` fallback, service target resolution, service focus ids)
    let sym_graph = json!({
        "atoms": [{
            "atom_id": "atom-canonical-1",
            "title": "JWT fix",
            "files": ["services/auth/src/jwt.rs"],
            "symbols": ["services/auth/src/jwt.rs::verify"]
        }],
        "symbols": [{
            "id": "services/auth/src/jwt.rs::verify",
            "symbol_id": "sym:jwt:verify",
            "fqn": "services/auth/src/jwt.rs::verify",
            "file_path": "services/auth/src/jwt.rs",
            "name": "verify"
        }]
    });
    let sym_insp = render_symbol_node_inspector_html(&sym_graph, &sym_graph["symbols"][0]);
    let payloads = crate::test_support::payloads(&sym_insp, "omnigraph-open-code-map");
    assert_eq!(payloads[0]["symbol_id"], "sym:jwt:verify");
    assert_eq!(
        resolve_code_map_target_with_symbol_id(&json!({
            "path": "services/auth/src/jwt.rs",
            "symbol_id": "sym:jwt:verify"
        })),
        Some((
            "services/auth/src/jwt.rs".to_owned(),
            Some("sym:jwt:verify".to_owned()),
            Some("sym:jwt:verify".to_owned()),
        ))
    );
    assert_eq!(
        find_context_node_for_file(&sym_graph, "services/auth/src/jwt.rs").as_deref(),
        Some("services/auth/src/jwt.rs::verify")
    );
    let atom_only_graph = json!({
        "atoms": [{
            "atom_id": "atom-raw-id",
            "title": "Only atom_id field",
            "files": ["services/billing/tax.rs"]
        }]
    });
    assert_eq!(
        find_context_node_for_file(&atom_only_graph, "services/billing/tax.rs").as_deref(),
        Some("atom-raw-id")
    );
    let cat = json!({
        "services": [
            { "id": "auth-svc", "name": "Auth", "root": "services/auth" },
            { "id": "billing", "name": "Billing", "root": "services/billing" }
        ],
        "shared_sources": ["docs/architecture.md"]
    });
    assert_eq!(
        resolve_system_service_target(&sym_graph, &cat, "atom-canonical-1").as_deref(),
        Some("auth-svc")
    );
    assert!(local_service_match_ids(&sym_graph, "auth-svc", &cat)
        .contains(&"atom-canonical-1".to_owned()));

    // P1 #10: Hotkeys gated behind omnigraph + constellation + visible studio + no modal open.
    assert!(constellation_hotkey_allowed(
        "omnigraph",
        "constellation",
        false,
        false,
        false,
        false
    ));
    assert!(!constellation_hotkey_allowed(
        "omnigraph",
        "dag",
        false,
        false,
        false,
        false
    ));
    assert!(!constellation_hotkey_allowed(
        "omnigraph",
        "merge",
        false,
        false,
        false,
        false
    ));
    assert!(!constellation_hotkey_allowed(
        "omnigraph",
        "constellation",
        false,
        true,
        false,
        false
    ));
    assert!(!constellation_hotkey_allowed(
        "omnigraph",
        "constellation",
        true,
        false,
        false,
        false
    ));
    assert!(!constellation_hotkey_allowed(
        "sessions",
        "constellation",
        false,
        false,
        false,
        false
    ));
}

#[test]
fn system_jump_and_code_map_jump_preserve_full_context() {
    let mut sys_state = json!({"query": "old"});
    let q = apply_system_jump_context_to_state(
        &mut sys_state,
        "billing-api",
        Some("POST /v1/invoices"),
        Some("src/Billing/Controller.php"),
        Some("sysrun-88"),
    );
    assert_eq!(q, "src/Billing/Controller.php");
    assert_eq!(sys_state["query"], json!(""));
    assert_eq!(sys_state["run_id"], json!("sysrun-88"));
    assert_eq!(sys_state["filter_run_id"], json!("sysrun-88"));
    assert_eq!(sys_state["system_context"]["service"], json!("billing-api"));
    assert_eq!(
        sys_state["system_context"]["contract"],
        json!("POST /v1/invoices")
    );
    assert_eq!(
        sys_state["system_context"]["file"],
        json!("src/Billing/Controller.php")
    );

    let ctx = resolve_code_map_jump_context(
        &json!({
            "path": "src/Billing/Service.php",
            "filter": "refund",
            "target_commit": "abcdef12",
            "semantic_kind": "publishes_event"
        }),
        &json!({"query": "fallback", "selected_commit_id": "1111111", "kind": "calls"}),
    );
    assert_eq!(ctx.filter.as_deref(), Some("refund"));
    assert_eq!(ctx.target_commit.as_deref(), Some("abcdef12"));
    assert_eq!(ctx.semantic_kind.as_deref(), Some("publishes_event"));

    let fallback = resolve_code_map_jump_context(
        &json!({"path": "src/Billing/Service.php"}),
        &json!({"query": "from-ui", "selected_commit_id": "9999aaa", "kind": "di_binds"}),
    );
    assert_eq!(fallback.filter.as_deref(), Some("from-ui"));
    assert_eq!(fallback.target_commit.as_deref(), Some("9999aaa"));
    assert_eq!(fallback.semantic_kind.as_deref(), Some("di_binds"));
}

#[test]
fn constellation_surfaces_ir_edges_semantics_violations_and_blast_markers() {
    let graph = json!({
        "active_branch": "main",
        "files": [
            { "id": "src/controller/OrderController.php", "path": "src/controller/OrderController.php", "touch_count": 2 }
        ],
        "ir_edges": [
            {
                "from": "src/controller/OrderController.php",
                "to": "src/db/OrderRepo.php",
                "source_file": "src/controller/OrderController.php",
                "target_file": "src/db/OrderRepo.php",
                "relation": "calls",
                "confidence": "EXTRACTED"
            }
        ],
        "ir_semantic_edges": [
            {
                "file_path": "src/controller/OrderController.php",
                "relation": "writes_table",
                "target": "table:orders"
            },
            {
                "file_path": "src/worker/OrderReader.php",
                "relation": "reads_table",
                "target": "table:orders"
            }
        ],
        "architecture_violations": [
            {
                "kind": "layer",
                "rule_id": "controller",
                "from": "src/controller/OrderController.php",
                "to": "src/db/OrderRepo.php",
                "message": "Layer Controller forbids DB"
            },
            {
                "kind": "semantic",
                "rule_id": "controller",
                "from": "src/controller/OrderController.php",
                "to": "table:orders",
                "relation": "writes_table",
                "message": "Controller forbids writes_table"
            },
            {
                "kind": "cycle_budget",
                "rule_id": "db",
                "from": "db",
                "to": "src/db/CycleHelper.php <-> src/db/OrderRepo.php",
                "message": "Layer DB participates in 1 dependency cycle (max_cycles budget: 0)."
            }
        ],
        "blast_markers": [
            {
                "path": "src/controller/OrderController.php",
                "blast_files": 4,
                "blast_tests": 2,
                "violation_count": 1,
                "in_cycle": false,
                "risk": "high"
            },
            {
                "path": "src/db/OrderRepo.php",
                "blast_files": 6,
                "blast_tests": 3,
                "violation_count": 1,
                "in_cycle": true,
                "risk": "high"
            }
        ]
    });

    let filter = omnigraph_canvas::ConstellationFilter {
        show_sessions: true,
        show_files: true,
        ..Default::default()
    };
    let rendered = omnigraph_canvas::render_constellation(
        &graph,
        &filter,
        &omnigraph_canvas::CanvasViewport::default(),
    );
    assert!(
        rendered.html.contains("data-arch-violation=\"true\""),
        "expected architecture violation edge marker in Constellation HTML: {}",
        rendered.html
    );
    assert!(
        rendered.html.contains("data-code-edge=\"true\""),
        "expected code edge marker in Constellation HTML"
    );
    assert!(
        rendered.html.contains("omni-arch-badge"),
        "expected architecture violation badge"
    );
    assert!(
        rendered.html.contains("omni-blast-badge"),
        "expected blast radius badge"
    );
    assert!(
        rendered.html.contains("omni-cycle-badge"),
        "expected cycle badge on synthesized OrderRepo.php"
    );
    assert!(
        rendered.html.contains("omni-sem-badge"),
        "expected semantic badge"
    );
    assert!(
        rendered
            .node_rects
            .iter()
            .any(|(id, ..)| id == "src/db/OrderRepo.php"),
        "expected synthesized target file node from ir_edges"
    );
    assert!(
        rendered
            .node_rects
            .iter()
            .any(|(id, ..)| id == "src/worker/OrderReader.php"),
        "expected cross-file semantic partner node"
    );

    let node_index_by_id: HashMap<String, usize> = rendered
        .node_rects
        .iter()
        .enumerate()
        .map(|(i, (id, ..))| (id.clone(), i))
        .collect();
    let node_titles: Vec<String> = rendered
        .node_rects
        .iter()
        .map(|(id, ..)| id.clone())
        .collect();
    let meta = constellation_view::build_omni_dense_scene_meta(
        &graph,
        &rendered.node_rects,
        &rendered.edge_endpoints,
        &node_titles,
        &node_index_by_id,
    );
    let ctrl_idx = node_index_by_id["src/controller/OrderController.php"];
    assert_eq!(meta.nodes[&ctrl_idx].blast_tag, "2 viol");
    assert_eq!(meta.nodes[&ctrl_idx].badge_text, "!2 · 4f");
    assert_eq!(
        meta.nodes[&ctrl_idx].explain_payload["violation_count"],
        json!(2)
    );
    assert_eq!(
        meta.nodes[&ctrl_idx].explain_payload["blast_files"],
        json!(4)
    );
    let repo_idx = node_index_by_id["src/db/OrderRepo.php"];
    assert!(meta.nodes[&repo_idx].in_cycle);
    assert!(
        meta.edges
            .values()
            .any(|e| e.explain_payload["arch_violation"] == json!(true)),
        "expected arch_violation flag on DenseEdgeMeta"
    );

    let insp =
        constellation_view::build_arch_inspector_html(&graph, "src/controller/OrderController.php");
    assert!(insp.contains("Layer Controller forbids DB"), "{insp}");
    assert!(insp.contains("Controller forbids writes_table"), "{insp}");
    assert!(insp.contains("writes_table"), "{insp}");
    assert!(insp.contains("4 files, 2 tests"), "{insp}");
    assert!(
        constellation_view::build_arch_inspector_html(&graph, "table:orders").is_empty(),
        "non-file semantic target must not falsely match v.to file violation inspector"
    );
}
