use super::{ingest::*, render::*};
use crate::run::files_map::{group_districts, render_file_card_html};
use crate::{
    model as m,
    run::files_map::{self, FileTouchEntry},
};
use serde_json::{json, Value as V};

fn setup_sample_plan() -> Vec<V> {
    vec![
        json!({
            "id": "contracts",
            "service": "__system__",
            "mode": "read-only",
            "label": "Contracts agent",
            "goal": "Reads contracts and shared context before dispatch.",
        }),
        json!({
            "id": "service-orders",
            "service": "orders",
            "mode": "edit",
            "label": "orders agent",
            "goal": "Updates OrderPlaced emitter and tests.",
        }),
        json!({
            "id": "service-payments",
            "service": "payments",
            "mode": "edit",
            "label": "payments agent",
            "goal": "Consumes updated OrderPlaced payload.",
        }),
        json!({
            "id": "verify",
            "service": "__system__",
            "mode": "read-only",
            "label": "Verification agent",
            "goal": "Checks service receipts and cross-service contracts.",
        }),
    ]
}

#[test]
fn multi_agent_live_stream_builds_timeline_files_plan_commands_and_service_districts() {
    reset("system-agents");
    m::set(
        "system-catalog",
        json!({
            "catalog": {
                "root": "/workspace/harbor-system",
                "services": [
                    {"id": "orders", "root": "/workspace/harbor-core/services/orders"},
                    {"id": "payments", "root": "/workspace/harbor-payments"}
                ]
            }
        }),
    );
    set_plan("system-agents", setup_sample_plan(), true);
    edit("system-agents", |p| p.sid = "sess-live-1".into());

    // 1. Contracts agent reads shared contract
    observe_session(
        "sess-live-1",
        &json!({
            "id": 1,
            "kind": "agent",
            "agent": "contracts",
            "attempt": 1,
            "status": "running",
            "service": "__system__",
            "root": "/workspace/harbor-system",
        }),
    );
    observe_session(
        "sess-live-1",
        &json!({
            "id": 2,
            "kind": "agent_activity",
            "agent": "contracts",
            "attempt": 1,
            "type": "tool",
            "tool": "Read",
            "call": "c-1",
            "state": "started",
            "service": "__system__",
            "path": "contracts/order-placed.yaml",
        }),
    );
    observe_session(
        "sess-live-1",
        &json!({
            "id": 3,
            "kind": "agent_activity",
            "agent": "contracts",
            "attempt": 1,
            "type": "tool",
            "tool": "Read",
            "call": "c-1",
            "state": "completed",
            "ok": true,
            "service": "__system__",
            "path": "contracts/order-placed.yaml",
        }),
    );
    observe_session(
        "sess-live-1",
        &json!({
            "id": 4,
            "kind": "agent",
            "agent": "contracts",
            "attempt": 1,
            "status": "completed",
            "ok": true,
            "summary": "Reviewed order-placed contract.",
        }),
    );

    // 2. Orders agent searches, plans, edits, creates via file_change, and runs PHPUnit
    observe_session(
        "sess-live-1",
        &json!({
            "id": 5,
            "kind": "agent",
            "agent": "service-orders",
            "attempt": 1,
            "status": "running",
            "service": "orders",
            "root": "/workspace/harbor-core/services/orders",
        }),
    );
    observe_session(
        "sess-live-1",
        &json!({
            "id": 6,
            "kind": "agent_activity",
            "agent": "service-orders",
            "attempt": 1,
            "type": "plan",
            "items": [
                {"content": "Add currency field to OrderPlaced", "status": "in_progress"},
                {"content": "Run orders test suite", "status": "pending"}
            ]
        }),
    );
    observe_session(
        "sess-live-1",
        &json!({
            "id": 7,
            "kind": "agent_activity",
            "agent": "service-orders",
            "attempt": 1,
            "type": "tool",
            "tool": "Grep",
            "call": "o-grep",
            "state": "started",
            "service": "orders",
            "path": "src/Events",
            "detail": "OrderPlaced",
        }),
    );
    observe_session(
        "sess-live-1",
        &json!({
            "id": 8,
            "kind": "agent_activity",
            "agent": "service-orders",
            "attempt": 1,
            "type": "tool",
            "tool": "Edit",
            "call": "o-edit",
            "state": "started",
            "service": "orders",
            "path": "src/Events/OrderPlaced.php",
        }),
    );

    // While o-edit is still open and orders is running, follow selects service-orders#1
    let live_snap = snapshot_for_root("system-agents");
    assert_eq!(live_snap.selected, "service-orders#1");
    assert!(live_snap.running);
    assert_eq!(live_snap.files.len(), 1);
    assert!(live_snap.files[0].now);

    observe_session(
        "sess-live-1",
        &json!({
            "id": 9,
            "kind": "agent_activity",
            "agent": "service-orders",
            "attempt": 1,
            "type": "tool",
            "tool": "Edit",
            "call": "o-edit",
            "state": "completed",
            "ok": true,
            "service": "orders",
            "path": "src/Events/OrderPlaced.php",
        }),
    );
    observe_session(
        "sess-live-1",
        &json!({
            "id": 10,
            "kind": "agent_activity",
            "agent": "service-orders",
            "attempt": 1,
            "type": "tool",
            "tool": "file_change",
            "call": "o-fc",
            "state": "completed",
            "ok": true,
            "detail": "add orders:tests/OrderPlacedTest.php",
        }),
    );
    observe_session(
        "sess-live-1",
        &json!({
            "id": 11,
            "kind": "agent_activity",
            "agent": "service-orders",
            "attempt": 1,
            "type": "tool",
            "tool": "Bash",
            "call": "o-bash",
            "state": "completed",
            "ok": true,
            "service": "orders",
            "detail": "vendor/bin/phpunit --filter OrderPlacedTest",
        }),
    );
    observe_session(
        "sess-live-1",
        &json!({
            "id": 12,
            "kind": "agent",
            "agent": "service-orders",
            "attempt": 1,
            "status": "completed",
            "ok": true,
        }),
    );

    // 3. Verify agent also reads OrderPlaced.php across service boundary!
    observe_session(
        "sess-live-1",
        &json!({
            "id": 13,
            "kind": "agent",
            "agent": "verify",
            "attempt": 1,
            "status": "running",
            "service": "__system__",
            "root": "/workspace/harbor-system",
        }),
    );
    observe_session(
        "sess-live-1",
        &json!({
            "id": 14,
            "kind": "agent_activity",
            "agent": "verify",
            "attempt": 1,
            "type": "tool",
            "tool": "Read",
            "call": "v-1",
            "state": "completed",
            "ok": true,
            "service": "orders",
            "path": "src/Events/OrderPlaced.php",
        }),
    );
    observe_session(
        "sess-live-1",
        &json!({
            "id": 15,
            "kind": "agent",
            "agent": "verify",
            "attempt": 1,
            "status": "completed",
            "ok": true,
        }),
    );

    // Once all agents finish, follow defaults to "all" (All agents)
    let all_snap = snapshot_for_root("system-agents");
    assert_eq!(all_snap.selected, "all");
    assert!(all_snap.show_all_card);
    assert_eq!(all_snap.files.len(), 3);
    assert_eq!(all_snap.commands.len(), 1);
    assert!(all_snap.plan_items.len() >= 6); // 4 wave steps + 2 orders plan items

    // In All agents view, districts group by Service (`{service_id} · {folder}`)
    let svc_districts = group_districts(&all_snap.files, "all", false, true, true);
    assert_eq!(svc_districts.len(), 2);
    assert_eq!(svc_districts[0].label, "__system__ · .");
    assert_eq!(
        svc_districts[1].label,
        "orders · harbor-core/services/orders"
    );
    // OrderPlaced.php was touched by both service-orders and verify!
    let order_event_chip = &svc_districts[1].blocks[0].files[0];
    assert_eq!(order_event_chip.agent_badge, "orders+verify");
    assert!(order_event_chip.edited);

    // Selecting single agent `service-orders#1` switches districts to top folders (1:1 with single-project Run Files map)
    assert!(event(
        "click",
        "agent-select",
        &json!({"root": "system-agents", "key": "service-orders#1"}),
        false
    ));
    assert!(event(
        "click",
        "activity-tab",
        &json!({"root": "system-agents", "tab": "files"}),
        false
    ));
    let orders_snap = snapshot_for_root("system-agents");
    assert_eq!(orders_snap.selected, "service-orders#1");
    assert_eq!(orders_snap.files.len(), 2);
    let folder_districts = group_districts(&orders_snap.files, "all", false, false, false);
    assert_eq!(folder_districts[0].label, "src/");
    assert_eq!(folder_districts[1].label, "tests/");

    // File card HTML & 4 required actions (including inspect-system-file-graph with data-service and data-path)
    let shared_entry = all_snap
        .files
        .iter()
        .find(|e| e.path == "src/Events/OrderPlaced.php")
        .expect("OrderPlaced entry");
    let actions = system_card_actions("system-agents", shared_entry);
    assert_eq!(actions.len(), 4);
    assert_eq!(actions[0].label, "Show in timeline");
    assert_eq!(actions[1].label, "Show on dependency map");
    assert_eq!(actions[1].action, "inspect-system-file-graph");
    assert_eq!(actions[1].data_service.as_deref(), Some("orders"));
    assert_eq!(
        actions[1].data_path.as_deref(),
        Some("src/Events/OrderPlaced.php")
    );
    assert_eq!(actions[2].label, "Add path to task");
    assert_eq!(actions[3].label, "Copy path");

    let card_html = render_file_card_html(shared_entry, &actions);
    assert!(card_html.contains("orders · src/Events/OrderPlaced.php"));
    assert!(card_html.contains("Touched by service-orders, verify"));
    assert!(card_html.contains("data-action=\"inspect-system-file-graph\""));
    assert!(card_html.contains("data-service=\"orders\""));
    assert!(card_html.contains("data-path=\"src/Events/OrderPlaced.php\""));

    // inspect-system-file-graph hides activity card and returns false so systems.rs handles it
    assert!(!event(
        "click",
        "inspect-system-file-graph",
        &json!({"service": "orders", "path": "src/Events/OrderPlaced.php"}),
        false
    ));

    assert_eq!(all_snap.tab, "timeline");
    assert_eq!(all_snap.files.len(), 3);
    assert_eq!(all_snap.commands.len(), 1);
}

fn render_files_surface_for_test(root: &str, snap: &InspectorSnapshot) -> String {
    let opts = crate::run::files_map::FilesSurfaceOptions {
        group_by_service: snap.selected == "all",
        show_agent_badge: snap.selected == "all",
        chip_action: "activity-file".into(),
        view_action: "activity-files-view".into(),
        filter_action: "activity-file-filter".into(),
        reveal_action: "activity-reveal".into(),
        inspect_graph_action: "inspect-system-file-graph".into(),
        root_id: Some(root.to_owned()),
    };
    files_map::render_files_surface(
        &snap.files,
        &snap.files_view,
        &snap.file_filter,
        snap.running,
        &opts,
    )
}

#[test]
fn persisted_evidence_hydrates_cli_runs_and_filters_by_launch_without_duplicate_counts() {
    reset("system-agents");
    set_plan("system-agents", setup_sample_plan(), false);

    // Simulate CLI-executed or historical run via set_run_evidence (no session events at all!)
    set_run_evidence(
        "sysrun-42",
        &json!({
            "dispatches": [
                {
                    "phase": "contracts",
                    "attempt": 1,
                    "launch_id": "launch-a",
                    "service": "__system__",
                    "root": ".",
                    "opened": ["contracts/orders.yaml"],
                    "steps": 2
                },
                {
                    "phase": "service-orders",
                    "attempt": 1,
                    "launch_id": "launch-a",
                    "service": "orders",
                    "root": "services/orders",
                    "opened": ["src/OrderService.php"],
                    "shell_reads": {"src/LegacyOrderMapper.php": 17},
                    "git_changed": [
                        {"path": "src/OrderService.php", "status": "M"},
                        {"path": "src/OrderDto.php", "status": "A"}
                    ],
                    "searched": [{"pattern": "OrderDto", "scope": "src"}],
                    "plan_items": [{"text": "Emit OrderDto from OrderService", "status": "completed"}],
                    "command_runs": [
                        {"command": "vendor/bin/phpunit", "ok": true},
                        {"command": "vendor/bin/phpstan analyse", "ok": true}
                    ],
                    "commands": 2,
                    "steps": 6
                },
                {
                    "phase": "service-payments",
                    "attempt": 1,
                    "launch_id": "launch-b",
                    "service": "payments",
                    "root": "../harbor-payments",
                    "opened": ["internal/consumer.go"],
                    "git_changed": ["internal/consumer.go"],
                    "commands": ["go test ./..."],
                    "steps": 4
                }
            ]
        }),
    );

    // With empty sid (CLI mode), all dispatches are shown
    let cli_snap = snapshot_for_root("system-agents");
    assert_eq!(cli_snap.selected, "all");
    assert_eq!(cli_snap.files.len(), 5); // orders.yaml, OrderService.php, LegacyOrderMapper.php (shell_read!), OrderDto.php, consumer.go
    assert_eq!(cli_snap.commands.len(), 3);
    assert!(cli_snap
        .files
        .iter()
        .any(|f| f.path == "src/OrderDto.php" && f.created && f.edits == 1));
    assert!(cli_snap
        .files
        .iter()
        .any(|f| f.path == "src/LegacyOrderMapper.php" && f.reads == 1));
    assert!(cli_snap
        .files
        .iter()
        .any(|f| f.path == "src/OrderService.php"
            && f.searched_patterns == vec!["OrderDto".to_owned()]));
    // Synthetic timeline events exist even without live session stream!
    assert!(!cli_snap.timeline_events.is_empty());

    // Scoping to launch-b shows only launch-b's dispatches (`service-payments`)
    edit("system-agents", |p| p.sid = "launch-b".into());
    let launch_b_snap = snapshot_for_root("system-agents");
    assert_eq!(launch_b_snap.files.len(), 1);
    assert_eq!(
        launch_b_snap.files[0].service_id.as_deref(),
        Some("payments")
    );
    assert_eq!(launch_b_snap.files[0].path, "internal/consumer.go");
    assert_eq!(launch_b_snap.commands.len(), 1);

    // When BOTH live session events AND persisted evidence report the same edit on consumer.go,
    // union-max prevents double counting `edits` (stays 1, not 2!)
    observe_session(
        "launch-b",
        &json!({
            "id": 1,
            "kind": "agent",
            "agent": "service-payments",
            "attempt": 1,
            "status": "completed",
            "service": "payments",
            "root": "../harbor-payments",
            "ok": true,
        }),
    );
    observe_session(
        "launch-b",
        &json!({
            "id": 2,
            "kind": "agent_activity",
            "agent": "service-payments",
            "attempt": 1,
            "type": "tool",
            "tool": "Edit",
            "call": "p-edit",
            "state": "completed",
            "ok": true,
            "service": "payments",
            "path": "internal/consumer.go",
        }),
    );
    let merged_snap = snapshot_for_root("system-agents");
    assert_eq!(merged_snap.files.len(), 1);
    assert_eq!(merged_snap.files[0].edits, 1);
}

#[test]
fn list_view_filters_reveal_and_outside_or_failed_files_work_end_to_end() {
    reset("system-agents");
    set_plan("system-agents", setup_sample_plan(), false);

    set_run_evidence(
        "sysrun-99",
        &json!({
            "evidence": {
                "by_phase": {
                    "service-orders": {
                        "attempt": 1,
                        "service": "orders",
                        "root": "/home/dev/lab/harbor-core/services/orders",
                        "file_touches": [
                            {
                                "path": "src/OrderController.php",
                                "reads": 3,
                                "edits": 1,
                                "first_step": 1,
                                "last_step": 4
                            },
                            {
                                "path": "src/BrokenHandler.php",
                                "reads": 1,
                                "edits": 0,
                                "failed": true,
                                "first_step": 2,
                                "last_step": 2
                            },
                            {
                                "path": "secrets.env",
                                "reads": 1,
                                "edits": 0,
                                "outside": true,
                                "first_step": 3,
                                "last_step": 3
                            }
                        ],
                        "commands": [
                            {"command": "vendor/bin/phpunit", "ok": false, "exit_code": 1}
                        ]
                    }
                }
            },
            "receipts": [
                {
                    "phase": "verify",
                    "attempt": 1,
                    "ok": true,
                    "report": {
                        "status": "completed",
                        "summary": "All cross-service contracts verified.",
                        "changed_files": [],
                        "checks": [
                            {
                                "name": "contract-suite",
                                "command": "cargo test --workspace",
                                "status": "passed"
                            }
                        ]
                    }
                }
            ]
        }),
    );

    assert!(event(
        "click",
        "activity-tab",
        &json!({"root": "system-agents", "tab": "files"}),
        false
    ));
    assert!(event(
        "click",
        "activity-file-filter",
        &json!({"root": "system-agents", "filter": "failed"}),
        false
    ));
    let failed_snap = snapshot_for_root("system-agents");
    assert_eq!(failed_snap.file_filter, "failed");
    let map_html = render_files_surface_for_test("system-agents", &failed_snap);
    assert!(map_html.contains("Edited 1"));
    assert!(map_html.contains("Failed 1"));
    assert!(map_html.contains("Outside 1"));
    assert!(map_html.contains("×3"));

    // Outside files omit `Show on dependency map` action on the file card
    let outside_entry = failed_snap
        .files
        .iter()
        .find(|e| e.outside)
        .expect("outside entry");
    let outside_actions = system_card_actions("system-agents", outside_entry);
    assert_eq!(outside_actions.len(), 3);
    assert!(outside_actions
        .iter()
        .all(|a| a.action != "inspect-system-file-graph"));

    // Toggle to List view and verify Map button carries inspect-system-file-graph
    assert!(event(
        "click",
        "activity-files-view",
        &json!({"root": "system-agents", "view": "list"}),
        false
    ));
    let list_snap = snapshot_for_root("system-agents");
    assert_eq!(list_snap.files_view, "list");
    let list_html = render_files_surface_for_test("system-agents", &list_snap);
    assert!(list_html.contains("class=\"run-tree\""));
    assert!(list_html.contains("data-action=\"inspect-system-file-graph\""));

    // Clicking Show in timeline (`activity-reveal`) switches back to `timeline` tab
    // and selects the target agent when not in `all` mode
    assert!(event(
        "click",
        "agent-select",
        &json!({"root": "system-agents", "key": "step:contracts"}),
        false
    ));
    assert!(event(
        "click",
        "activity-reveal",
        &json!({
            "root": "system-agents",
            "agent": "service-orders#1",
            "event_id": "ev:service-orders#1:src/OrderController.php",
            "path": "src/OrderController.php",
            "service": "orders"
        }),
        false
    ));
    let revealed = snapshot_for_root("system-agents");
    assert_eq!(revealed.tab, "timeline");
    assert_eq!(revealed.selected, "service-orders#1");
}

#[test]
fn command_and_folder_restore_keys_stay_stable_across_status_updates_and_dedup_collisions() {
    let cmds = vec![
        json!({
            "call": "c-1",
            "agent_key": "service-orders#1",
            "command": "cargo test",
            "running": true,
            "ok": true,
            "class": { "check": true }
        }),
        json!({
            "agent_key": "service-orders#1",
            "command": "git status",
            "running": false,
            "ok": true,
            "class": { "check": false }
        }),
        json!({
            "agent_key": "service-orders#1",
            "command": "git status",
            "running": false,
            "ok": false,
            "class": { "check": false }
        }),
    ];
    assert_eq!(
        command_restore_keys(&cmds),
        vec![
            "cmd:c-1",
            "cmd:service-orders#1:git status",
            "cmd:service-orders#1:git status#1",
        ]
    );
    // Key stays identical when running -> completed so keyed_render preserves open <details>
    let mut finished = cmds.clone();
    finished[0]["running"] = json!(false);
    assert_eq!(command_restore_keys(&finished)[0], "cmd:c-1");

    let snap = InspectorSnapshot {
        items: Vec::new(),
        show_all_card: false,
        selected: "all".into(),
        tab: "commands".into(),
        files_view: "map".into(),
        file_filter: "all".into(),
        cmd_filter: "checks".into(),
        running: false,
        timeline_events: Vec::new(),
        files: Vec::new(),
        plan_items: Vec::new(),
        commands: finished,
    };
    let (filter, checks_count, failed_count, items) = build_command_render_items(&snap);
    assert_eq!(filter, "checks");
    assert_eq!(checks_count, 1);
    assert_eq!(failed_count, 1);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["key"], "cmd:c-1");

    let folders = vec![(
        "orders · src/".to_owned(),
        vec![FileTouchEntry {
            path: "src/Order.php".to_owned(),
            service_id: Some("orders".to_owned()),
            reads: 2,
            edits: 1,
            ..FileTouchEntry::default()
        }],
    )];
    let folder_items = build_folder_render_items(&folders, true);
    assert_eq!(folder_items.len(), 1);
    assert_eq!(folder_items[0]["key"], "folder:orders · src/");
    assert_eq!(roving_tabindex(true), "0");
    assert_eq!(roving_tabindex(false), "-1");
}

#[test]
fn structured_events_cards_filter_escape_and_avoid_inline_hex_colors() {
    use super::events_card::*;
    let events = vec![
        json!({"id": "e1", "kind": "status", "text": "Plan accepted", "time": "10:00:01"}),
        json!({"id": "e2", "kind": "error", "service": "orders", "text": "PHPUnit failed <script>alert(\"xss\")</script> in src/Order.php:42", "time": "10:00:05"}),
        json!({"id": "e3", "kind": "permission_request", "text": "Needs write access", "time": "10:00:06"}),
    ];
    let (total, all_items) = build_structured_event_items(&events, "all", "");
    assert_eq!(total, 3);
    assert_eq!(all_items.len(), 3);

    let (_, errs) = build_structured_event_items(&events, "error", "order");
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0]["kind"], "error");

    let html = render_structured_events_html_for_test("system-events", &events, "error", "order");
    assert!(html.contains("&lt;script&gt;"));
    assert!(!html.contains("<script>"));
    assert!(html.contains("structured-event-card"));
    assert!(html.contains("data-kind=\"error\""));
    crate::test_support::assert_no_inline_hex_colors(&html);

    // SSE merge deduplication and retry backoff progression.
    let mut buf = vec![json!({"id": "e1", "kind": "status", "text": "Started"})];
    assert!(!merge_sse_event(
        &mut buf,
        json!({"id": "e1", "kind": "status", "text": "Started"})
    ));
    assert!(merge_sse_event(
        &mut buf,
        json!({"id": "e2", "kind": "tool_use", "text": "Running tests"})
    ));
    assert_eq!(buf.len(), 2);

    assert_eq!(retry_backoff_ms(0), 1000);
    assert_eq!(retry_backoff_ms(1), 2000);
    assert_eq!(retry_backoff_ms(2), 4000);
    assert_eq!(retry_backoff_ms(3), 8000);
    assert_eq!(retry_backoff_ms(9), 8000);
    assert!(is_terminal_poll_status(404));
    assert!(is_terminal_poll_status(401));
    assert!(!is_terminal_poll_status(503));
    assert!(!is_terminal_poll_status(0));
}

#[test]
fn failed_command_builds_localize_payload_and_token_button() {
    m::set(
        "system-run",
        json!({"id": "run-9", "project_id": "proj-1", "config_path": "infra.yml"}),
    );
    let cmd = json!({
        "call": "c-fail",
        "command": "vendor/bin/phpunit tests/OrderTest.php",
        "agent_key": "orders",
        "service": "orders",
        "running": false,
        "ok": false,
        "not_run": false,
        "error": "AssertionFailed in src/OrderService.php:88\nRuntimeException in src/Cart.php",
        "class": {"check": true}
    });
    let files = vec![FileTouchEntry {
        path: "src/Fallback.php".into(),
        service_id: Some("orders".into()),
        ..FileTouchEntry::default()
    }];
    let req = command_localize_payload(&cmd, &files, Some("system-agents"), "", "run-9")
        .expect("failed command builds localization request");
    assert_eq!(req["project_id"], "proj-1");
    assert_eq!(req["config_path"], "infra.yml");
    assert_eq!(req["service_id"], "orders");
    assert_eq!(req["run_id"], "run-9");
    assert_eq!(req["prefer_full_modal"], true);
    let hints = req["hint_paths"].as_array().unwrap();
    assert!(hints.iter().any(|p| p == "src/OrderService.php"));
    assert!(hints.iter().any(|p| p == "src/Fallback.php"));

    let html = build_failed_command_actions_html_for_test(&req);
    assert!(html.contains("activity-cmd-actions"));
    assert!(html.contains("Localize failure on map"));
    crate::test_support::assert_no_inline_hex_colors(&html);
    m::take("system-run");
}

#[test]
fn typed_agent_record_handles_hundreds_of_events_in_place_without_meta_bloat() {
    reset("system-agents");
    edit("system-agents", |p| p.sid = "sess-scale-1".into());
    observe_session(
        "sess-scale-1",
        &json!({
            "id": 1,
            "kind": "agent",
            "agent": "orders",
            "attempt": 1,
            "status": "running",
            "service": "orders",
            "root": "/workspace/orders",
        }),
    );
    let mut seq = 2u64;
    for i in 0..120 {
        let call = format!("c-{i}");
        observe_session(
            "sess-scale-1",
            &json!({
                "id": seq,
                "kind": "agent_activity",
                "agent": "orders",
                "attempt": 1,
                "type": "tool",
                "tool": "Bash",
                "call": call,
                "state": "started",
                "detail": format!("phpunit tests/T{i}.php"),
            }),
        );
        seq += 1;
        observe_session(
            "sess-scale-1",
            &json!({
                "id": seq,
                "kind": "agent_activity",
                "agent": "orders",
                "attempt": 1,
                "type": "tool",
                "tool": "Bash",
                "call": call,
                "state": "completed",
                "ok": true,
            }),
        );
        seq += 1;
    }
    let snap = snapshot_for_root("system-agents");
    assert_eq!(snap.commands.len(), 120);
    assert_eq!(snap.timeline_events.len(), 120);
    assert_eq!(snap.timeline_events[119]["state"], "completed");
    // Card summary metadata stays lightweight and never duplicates nested event arrays.
    assert!(snap.items[0].get("events").is_none());
    assert!(snap.items[0].get("files").is_none());
    reset("system-agents");
}
