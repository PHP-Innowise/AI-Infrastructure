use super::test_exports::{
    creator_report_view, format_setup_diagnostic, readiness_cards, setup_provider_card,
};
use serde_json::json;

#[test]
fn setup_readiness_providers_diagnostics_and_creator_reports_format_cleanly() {
    let readiness = json!({
        "policy": true,
        "memory_runtime": false,
        "brain_runtime": true
    });
    let checks = readiness_cards(&readiness);
    assert_eq!(checks.len(), 3);
    assert_eq!(checks[0].title, "Policy guide");
    assert!(checks[0].ok);
    assert_eq!(checks[0].state, "Ready");
    assert_eq!(checks[1].title, "Memory runtime");
    assert!(!checks[1].ok);
    assert_eq!(checks[1].state, "Not detected");

    let provider = json!({
        "id": "codex",
        "name": "Codex",
        "available": true,
        "detail": "CLI ready",
        "agent_control_detail": "Interactive TTY"
    });
    let provider_card = setup_provider_card(&provider, 0);
    assert_eq!(provider_card.title, "Codex");
    assert!(provider_card.ok);
    assert_eq!(provider_card.state, "Available");
    assert_eq!(provider_card.detail, "CLI ready · Interactive TTY");

    assert_eq!(
        format_setup_diagnostic(&json!("Missing PHP binary on PATH")),
        "Missing PHP binary on PATH"
    );

    let report = json!({
        "name": "infra-scan-project-profile.md",
        "text": "# Scan\nAll good",
        "truncated": true,
        "hash": "abcdef123456"
    });
    let view = creator_report_view(&report, 0);
    assert!(view
        .summary
        .contains("infra-scan-project-profile.md (15 chars · truncated · #abcdef12)"));
    assert_eq!(view.text, "# Scan\nAll good");
}

#[test]
fn creator_permissions_hints_payloads_restoration_and_inline_cards_preserve_contracts() {
    assert!(super::creator_permissions_hint_text("claude", "auto").contains("auto mode"));
    assert!(super::creator_permissions_hint_text("codex", "manual").contains("Allow and Deny"));
    assert!(
        super::creator_permissions_hint_text("cursor", "bypass").contains("without the sandbox")
    );

    assert_eq!(
        super::creator_restored_permissions(&json!({"permissions": "bypass"})),
        "auto"
    );
    assert_eq!(
        super::creator_restored_permissions(&json!({"permissions": "manual"})),
        "manual"
    );
    assert_eq!(super::creator_restored_permissions(&json!({})), "auto");

    let run = json!({
        "target": "/srv/app",
        "workspace": "project",
        "provider": "claude",
        "model": "sonnet",
        "permissions": "manual",
        "status": "scanning"
    });
    assert_eq!(
        super::creator_target_summary(&run),
        "/srv/app · project · claude / sonnet · Permissions: Manual"
    );
    assert!(super::creator_busy(&run));

    let waiting = vec![json!({
        "request_id": "cr-1",
        "provider": "claude",
        "tool": "Bash",
        "summary": "composer test",
        "detail": "Manual approval requested",
        "session_option": true,
        "rule_summary": "Allow Bash(composer test)",
    })];
    assert_eq!(
        super::creator_status_text(&run, &waiting),
        "scanning · Waiting for your answer to a permission request…"
    );
    assert_eq!(super::creator_status_text(&run, &[]), "scanning");

    let detail = json!({"run": run, "waiting_permissions": waiting});
    assert_eq!(
        super::creator_permissions_panel_state(&detail),
        (
            true,
            "Waiting for your answer to a permission request…".to_owned()
        )
    );
    assert_eq!(
        super::creator_permissions_panel_state(&json!({"waiting_permissions": []})),
        (false, String::new())
    );

    let start = super::creator_start_body(
        json!({"usd": 5}),
        "proj-1",
        "claude",
        json!(null),
        json!("high"),
        "manual",
        "generate",
        "project",
        String::new(),
        true,
        json!(3),
        "Build PHP kit",
        vec!["claude".to_owned()],
    );
    assert_eq!(start["permissions"], "manual");
    assert_eq!(start["project_id"], "proj-1");

    let routing =
        super::creator_routing_payload(json!("opus"), json!(null), "bypass", false, json!(3));
    assert_eq!(routing["permissions"], "bypass");
    assert_eq!(routing["model"], "opus");

    let html = crate::run::render_permission_card_html(
        "creator-permissions-list",
        &detail["waiting_permissions"][0],
        true,
    );
    crate::test_support::assert_no_inline_hex_colors(&html);
    assert!(html.contains("data-permission-id=\"cr-1\""));
    assert!(html.contains("Allow for this session: Allow Bash(composer test)"));
    let payloads = crate::test_support::payloads(&html, "permission-decide");
    assert_eq!(payloads.len(), 3);
    assert_eq!(payloads[0]["container"], "creator-permissions-list");
    assert_eq!(payloads[0]["request_id"], "cr-1");
    assert_eq!(payloads[1]["decision"], "allow_session");
}

#[test]
fn create_skill_validation_enforces_slug_bounds_and_tools() {
    let bad_name = super::validate_create_skill_inputs(
        "proj-1",
        "BAD NAME",
        "Valid trigger description",
        "# Steps",
        1,
    );
    assert!(!bad_name.valid);
    assert!(!bad_name.name_error.is_empty());

    let double_hyphen = super::validate_create_skill_inputs(
        "proj-1",
        "bad--name",
        "Valid trigger description",
        "# Steps",
        1,
    );
    assert!(!double_hyphen.valid);
    assert!(!double_hyphen.name_error.is_empty());

    let no_tools = super::validate_create_skill_inputs(
        "proj-1",
        "check-tax-rounding",
        "Valid trigger description",
        "# Steps",
        0,
    );
    assert!(!no_tools.valid);
    assert!(!no_tools.agents_error.is_empty());

    let ok = super::validate_create_skill_inputs(
        "proj-1",
        "check-tax-rounding",
        "Use when verifying tax calculations",
        "# Steps\nVerify 2-decimal precision.",
        2,
    );
    assert!(ok.valid);
    assert!(ok.name_error.is_empty());
    assert!(ok.summary_error.is_empty());
}

#[test]
fn setup_preview_filter_surfaces_reported_collisions_and_all_files() {
    let preview = json!({
        "files": [
            {"path": "AGENTS.md", "action": "merge"},
            {"path": ".gitignore", "action": "unchanged"},
        ],
        "collisions": [
            {"path": "ACCELERATOR.md", "reason": "accelerator-readme-exists"},
        ],
    });

    let changed = super::filtered_setup_files(&preview, "", "changed");
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0]["path"], "AGENTS.md");

    let collisions = super::filtered_setup_files(&preview, "", "collision");
    assert_eq!(collisions.len(), 1);
    assert_eq!(collisions[0]["path"], "ACCELERATOR.md");
    assert_eq!(collisions[0]["action"], "collision");
    assert_eq!(collisions[0]["reason"], "accelerator-readme-exists");

    let all = super::filtered_setup_files(&preview, "", "all");
    assert_eq!(all.len(), 3);

    let searched = super::filtered_setup_files(&preview, "accelerator", "collision");
    assert_eq!(searched.len(), 1);
    assert_eq!(searched[0]["path"], "ACCELERATOR.md");

    // Keyed Setup preview keeps stable non-index keys & hides unmatched items without removing DOM nodes.
    let (all_items, visible_keys) =
        super::test_exports::setup_preview_visibility(&preview, "accelerator", "collision");
    assert_eq!(all_items.len(), 3);
    assert_eq!(visible_keys.len(), 1);
    assert!(visible_keys.contains("file:collision:ACCELERATOR.md"));
}

#[test]
fn creator_poll_backoff_sse_delta_and_approval_retention_work_end_to_end() {
    use super::test_exports::*;
    use crate::model as m;

    // Transient poll errors retry on active runs; terminal status or finished run stops.
    let active_detail = json!({"run": {"id": "run-1", "status": "generating"}});
    assert!(creator_should_retry_poll(&active_detail, "run-1", 0));
    assert!(creator_should_retry_poll(&active_detail, "run-1", 502));
    assert!(!creator_should_retry_poll(&active_detail, "run-1", 404));
    assert!(!creator_should_retry_poll(&active_detail, "run-1", 401));

    let done_detail = json!({"run": {"id": "run-1", "status": "complete"}});
    assert!(!creator_should_retry_poll(&done_detail, "run-1", 500));
    assert!(creator_should_retry_poll(&done_detail, "run-2", 500));

    // SSE delta handling matches project & run IDs cleanly.
    let mut detail = active_detail.clone();
    assert_eq!(
        apply_sse_creator_delta(
            &mut detail,
            "proj-a",
            "run-1",
            &json!({"project_id": "proj-b", "run_id": "run-1", "status": "complete"})
        ),
        CreatorSseOutcome::Ignore
    );
    assert_eq!(
        apply_sse_creator_delta(
            &mut detail,
            "proj-a",
            "run-1",
            &json!({"project_id": "proj-a", "run_id": "run-9", "status": "scanning"})
        ),
        CreatorSseOutcome::RefreshListOnly
    );
    assert_eq!(
        apply_sse_creator_delta(
            &mut detail,
            "proj-a",
            "run-1",
            &json!({"project_id": "proj-a", "run_id": "run-1", "status": "preview", "active": false})
        ),
        CreatorSseOutcome::RefreshRunAndList { immediate: true }
    );

    // Keyed #creator-files & preserved creator-replace checkboxes across polls.
    m::take("creator-approved-paths");
    let preview = json!({
        "preview_id": "pv-7",
        "can_apply": true,
        "files": [
            {"path": "AGENTS.md", "action": "replace", "requires_decision": true},
            {"path": "skills/foo.md", "action": "create", "requires_decision": false}
        ]
    });
    let run_doc = json!({"run": {"id": "run-1", "status": "preview"}, "preview": preview});

    let approved_none = creator_approved_paths_for("run-1", "pv-7");
    assert!(creator_apply_disabled(&run_doc["preview"], &approved_none));

    let approved_after = set_creator_approved_path("run-1", "pv-7", "AGENTS.md", true);
    assert!(!creator_apply_disabled(
        &run_doc["preview"],
        &approved_after
    ));

    let file_items = build_creator_file_render_items(&run_doc["preview"]);
    assert_eq!(file_items.len(), 3);
    assert_eq!(file_items[0]["key"], "file:replace:AGENTS.md");
    assert_eq!(file_items[1]["key"], "approve:AGENTS.md");
    assert_eq!(file_items[2]["key"], "file:create:skills/foo.md");

    let actions_enabled = build_creator_action_items(&run_doc, &approved_after);
    let apply_btn = actions_enabled
        .iter()
        .find(|b| b["key"] == "apply")
        .unwrap();
    assert_eq!(apply_btn["disabled"], false);
    m::take("creator-approved-paths");
}

#[test]
fn management_global_project_sync_clears_creator_state_unless_pending() {
    use crate::model as m;

    m::set("creator-project-sync", json!("proj-old"));
    m::set("creator-pending", json!(false));
    m::set("creator-detail", json!({"run": {"id": "r1"}}));
    super::on_global_project_change("proj-new");
    assert_eq!(m::string(&m::data("creator-project-sync")), "proj-new");
    assert!(m::data("creator-detail").is_null());

    m::set("creator-pending", json!(true));
    super::on_global_project_change("proj-blocked");
    assert_eq!(m::string(&m::data("creator-project-sync")), "proj-new");
    m::take("creator-pending");
    m::take("creator-project-sync");
}

#[test]
fn skills_filter_matches_id_name_and_description_case_insensitively() {
    use super::test_exports::matches_skill_query;

    assert!(matches_skill_query(
        "lar-review",
        "Laravel Review",
        "Check Eloquent N+1 queries",
        ""
    ));
    assert!(matches_skill_query(
        "lar-review",
        "Laravel Review",
        "Check Eloquent N+1 queries",
        "  LAR-REV "
    ));
    assert!(matches_skill_query(
        "lar-review",
        "Laravel Review",
        "Check Eloquent N+1 queries",
        "eloquent"
    ));
    assert!(!matches_skill_query(
        "lar-review",
        "Laravel Review",
        "Check Eloquent N+1 queries",
        "symfony"
    ));
}
