#[cfg(test)]
mod editor_value_tests {
    use crate::systems::test_exports::*;
    use serde_json::json;
    fn partial_passport() -> V {
        json!({"project_id":"a","config_path":"system.json","services":[{"passport":{
            "schema_version":1,"id":"service-a","description":"Existing valid service","owner":"Team A",
            "capabilities":[{"id":"read","description":"Read records","status":"implemented","sources":[]}],
            "provides":[],"consumes":[],"sources":[]
        }}]})
    }
    #[test]
    fn edits_insert_optional_fields_of_an_existing_native_passport() {
        let mut data = partial_passport();
        assert!(data
            .pointer("/services/0/passport/relationships_complete")
            .is_none());
        assert!(data
            .pointer("/services/0/passport/capabilities/0/keywords")
            .is_none());
        assert!(set_editor_value(
            &mut data,
            "/services/0/passport/relationships_complete",
            json!(true)
        ));
        assert!(set_editor_value(
            &mut data,
            "/services/0/passport/capabilities/0/keywords",
            json!(["routing", "lookup"])
        ));
        assert_eq!(
            data["services"][0]["passport"]["relationships_complete"],
            true
        );
        assert_eq!(
            data["services"][0]["passport"]["capabilities"][0]["keywords"],
            json!(["routing", "lookup"])
        );
        assert_eq!(data["services"][0]["passport"]["owner"], "Team A");
        assert_eq!(data["project_id"], "a");
    }
    #[test]
    fn setter_decodes_final_object_keys_and_preserves_existing_array_indices() {
        let mut data = json!({"object":{},"array":[{"name":"before"}]});
        assert!(set_editor_value(
            &mut data,
            "/object/a~1b~0c",
            json!("created")
        ));
        assert_eq!(data["object"]["a/b~c"], "created");
        assert!(set_editor_value(&mut data, "/array/0/name", json!("after")));
        assert!(set_editor_value(&mut data, "/array/0/new", json!(7)));
        assert_eq!(data["array"][0], json!({"name":"after","new":7}));
        assert_eq!(data["array"].as_array().unwrap().len(), 1);
    }
    #[test]
    fn missing_parents_invalid_pointers_and_array_growth_leave_the_document_unchanged() {
        let mut data = partial_passport();
        let before = data.clone();
        for path in [
            "",
            "services/0/passport/owner",
            "/services/1/passport/owner",
            "/services/-",
            "/services/0/missing/leaf",
            "/services/0/passport/invalid~2escape",
            "/services/0/passport/capabilities/01/keywords",
        ] {
            assert!(
                !set_editor_value(&mut data, path, json!("discarded")),
                "{path}"
            );
            assert_eq!(data, before, "{path}");
        }
    }
}

#[cfg(test)]
mod display_tests {
    use crate::systems::test_exports::*;
    use serde_json::json;
    use std::collections::BTreeMap;
    fn segment(path: &str) -> Vec<i64> {
        path.split(|c: char| !c.is_ascii_digit() && c != '-')
            .filter(|part| !part.is_empty())
            .map(|part| part.parse().unwrap())
            .collect()
    }
    #[test]
    fn an_upward_edge_leaves_the_top_of_its_box_and_enters_the_bottom_of_the_target() {
        // provider in the second row, consumer above it in the first
        let points = segment(&edge_path((24, 148), (260, 24)));
        assert_eq!(points[1], 148, "starts at the provider's top edge");
        assert_eq!(points[3], 24 + 76, "ends at the consumer's bottom edge");
    }
    #[test]
    fn a_downward_edge_leaves_the_bottom_and_enters_the_top() {
        let points = segment(&edge_path((24, 24), (260, 148)));
        assert_eq!(points[1], 24 + 76);
        assert_eq!(points[3], 148);
    }
    #[test]
    fn a_pair_linked_both_ways_gets_two_separate_lines() {
        let forward = edge_path((24, 24), (260, 24));
        let backward = edge_path((260, 24), (24, 24));
        assert_ne!(segment(&forward)[1], segment(&backward)[1]);
        // each line runs between the facing sides, never across its own boxes
        assert_eq!(segment(&forward)[0], 24 + 190);
        assert_eq!(segment(&forward)[2], 260);
        assert_eq!(segment(&backward)[0], 260);
        assert_eq!(segment(&backward)[2], 24 + 190);
    }
    #[test]
    fn a_link_that_skips_boxes_in_its_row_arcs_around_them() {
        assert!(edge_path((24, 24), (496, 24)).contains('Q'));
        assert!(edge_path((496, 24), (24, 24)).contains('Q'));
        assert!(!edge_path((24, 24), (260, 24)).contains('Q'));
    }
    #[test]
    fn catalog_warnings_read_as_sentences() {
        assert_eq!(
            warning_text(&json!({"service":"orders","reason":"relationships_incomplete"})),
            "orders: dependencies are not confirmed complete"
        );
        assert_eq!(
            warning_text(&json!({"service":"__system__","reason":"dependency_cycle"})),
            "System: declared contract dependencies form a cycle"
        );
        assert_eq!(
            warning_text(
                &json!({"service":"billing","reason":"source_unavailable_or_ineligible","path":"src/a.php"})
            ),
            "billing: a registered source is missing or not eligible: src/a.php"
        );
        assert_eq!(
            warning_text(&json!({"service":"x","reason":"new_reason"})),
            "x: new reason"
        );
        assert_eq!(warning_text(&json!("plain text")), "plain text");
    }
    #[test]
    fn a_service_card_is_named_by_its_id_or_else_its_folder() {
        assert_eq!(
            service_title(&json!({"id":"orders"}), "/x/orders"),
            "orders"
        );
        assert_eq!(
            service_title(&json!({"id":""}), "/lab/harbor-payments/"),
            "New service · harbor-payments"
        );
        assert_eq!(service_title(&json!({"id":" "}), ""), "New service");
    }
    #[test]
    fn a_plan_without_services_explains_what_the_task_matched() {
        let hint = selection_hint(&[
            json!({"service":"orders","matches":[{"terms":["order","cancel"]},{"terms":["order"]}]}),
            json!({"service":"payments","matches":[{"terms":["refund"]}]}),
        ]);
        assert!(hint.contains("orders (cancel, order)"), "{hint}");
        assert!(hint.contains("payments (refund)"), "{hint}");
        assert!(selection_hint(&[]).contains("matches no declared capability"));
    }
    #[test]
    fn a_receipt_names_what_stopped_its_dispatch() {
        assert_eq!(receipt_outcome(&json!({"ok":true})), "completed");
        assert_eq!(
            receipt_outcome(&json!({"ok":true,"report":{"status":"completed","checks":[
                {"name":"reproduction","status":"failed"},{"name":"unit","status":"passed"}]}})),
            "completed; failed checks are findings: reproduction"
        );
        assert_eq!(
            receipt_outcome(
                &json!({"ok":false,"error":"invalid_or_sensitive_worker_result",
                "rejection":"Read-only worker reported writes","report":null})
            ),
            "rejected: Read-only worker reported writes"
        );
        assert_eq!(
            receipt_outcome(
                &json!({"ok":false,"error":"worker_blocked_or_verification_incomplete",
                "report":{"status":"blocked","summary":"Refund rule undecided","checks":[]}})
            ),
            "blocked by the agent: Refund rule undecided"
        );
        assert_eq!(
            receipt_outcome(
                &json!({"ok":false,"phase":"service-inventory","error":"worker_blocked_or_verification_incomplete",
                "report":{"status":"completed","checks":[{"name":"tests","status":"passed"},{"name":"composer_metadata","status":"failed"}]}})
            ),
            "failed checks: composer_metadata"
        );
        assert_eq!(
            receipt_outcome(
                &json!({"ok":false,"phase":"verify","error":"worker_blocked_or_verification_incomplete",
                "report":{"status":"completed","checks":[{"name":"go","status":"not_run"},{"name":"php","status":"passed"}]}})
            ),
            "checks not run: go"
        );
        assert_eq!(
            receipt_outcome(
                &json!({"ok":false,"phase":"contracts","error":"read_only_worker_reported_writes",
                "report":{"status":"completed","checks":[],"changed_files":["billing/.tool/marker"]}})
            ),
            "read-only worker reported writes: billing/.tool/marker"
        );
        assert_eq!(
            receipt_outcome(&json!({"ok":false,"error":"worker_exit_failure","report":null})),
            "blocked: worker exit failure"
        );
        assert_eq!(
            receipt_outcome(
                &json!({"ok":false,"error":"worker_exit_failure","report":null,
                "failure":"Failed to authenticate: OAuth session expired"})
            ),
            "the agent CLI failed: Failed to authenticate: OAuth session expired"
        );
    }
    #[test]
    fn a_stopped_step_with_a_successful_receipt_reads_as_reported() {
        let receipts = json!([
            {"phase":"service-orders","attempt":1,"ok":true},
            {"phase":"service-billing","attempt":1,"ok":false}
        ]);
        let orders = json!({"id":"service-orders","attempt":1,"status":"running"});
        assert_eq!(shown_status(&orders, &receipts, false), "reported");
        assert_eq!(shown_status(&orders, &receipts, true), "running");
        // A receipt of an earlier attempt says nothing of this one.
        let retried = json!({"id":"service-orders","attempt":2,"status":"interrupted"});
        assert_eq!(shown_status(&retried, &receipts, false), "interrupted");
        let billing = json!({"id":"service-billing","attempt":1,"status":"running"});
        assert_eq!(shown_status(&billing, &receipts, false), "interrupted");
        let blocked = json!({"id":"service-billing","attempt":1,"status":"blocked"});
        assert_eq!(shown_status(&blocked, &receipts, false), "blocked");
        let done = json!({"id":"contracts","attempt":1,"status":"completed"});
        assert_eq!(shown_status(&done, &receipts, false), "completed");
    }
    #[test]
    fn only_a_verification_whose_gaps_are_unrun_checks_offers_acceptance() {
        let step = json!({"id":"verify","attempt":2,"status":"blocked"});
        let receipt = |checks: V, status: &str| {
            json!([{"phase":"verify","attempt":2,"ok":false,"error":"worker_blocked_or_verification_incomplete",
                "rejection":null,"report":{"status":status,"checks":checks,"changed_files":[]}}])
        };
        let honest = receipt(
            json!([{"name":"unit","status":"passed"},{"name":"runtime_integration","status":"not_run"}]),
            "completed",
        );
        assert_eq!(
            unrun_checks(&step, &honest),
            Some(vec!["runtime_integration".to_string()])
        );
        let failed = receipt(
            json!([{"name":"unit","status":"failed"},{"name":"runtime_integration","status":"not_run"}]),
            "completed",
        );
        assert_eq!(unrun_checks(&step, &failed), None);
        let blocked = receipt(json!([{"name":"unit","status":"not_run"}]), "blocked");
        assert_eq!(unrun_checks(&step, &blocked), None);
        let nothing_ran = receipt(json!([{"name":"unit","status":"not_run"}]), "completed");
        assert_eq!(unrun_checks(&step, &nothing_ran), None);
        let earlier = json!({"id":"verify","attempt":3,"status":"blocked"});
        assert_eq!(unrun_checks(&earlier, &honest), None);
        let service = json!({"id":"service-orders","attempt":2,"status":"blocked"});
        assert_eq!(unrun_checks(&service, &honest), None);
    }
    #[test]
    fn a_queued_run_says_what_it_waits_for() {
        assert_eq!(
            run_status(&json!({"status":"queued","active":true})),
            "queued · waits for a free runner slot"
        );
    }
    #[test]
    fn a_stopped_run_says_what_stopped_it() {
        let run = |error: V, receipts: V| {
            json!({"status":"blocked","active":false,
                "execution":{"error":error,"steps":[
                    {"id":"contracts","status":"blocked","attempt":2}]},
                "receipts":receipts})
        };
        let agent = run(
            json!("worker_blocked_or_verification_incomplete"),
            json!([{"phase":"contracts","attempt":2,"ok":false,
                "error":"worker_blocked_or_verification_incomplete",
                "report":{"status":"blocked","summary":"The refund owner is undecided","checks":[]}}]),
        );
        assert_eq!(
            run_status(&agent),
            "blocked · contracts: blocked by the agent: The refund owner is undecided"
        );
        // Without a receipt of the stopped attempt the journal's error is shown.
        let stale = run(
            json!("worker_blocked_or_verification_incomplete"),
            json!([{"phase":"contracts","attempt":1,"ok":false,"error":"interrupted","report":null}]),
        );
        assert_eq!(
            run_status(&stale),
            "blocked · worker blocked or verification incomplete"
        );
        let restarted = run(V::Null, json!([]));
        assert_eq!(
            run_status(&restarted),
            "blocked · contracts stopped before it saved a receipt; inspect its folder, then retry it"
        );
        let sentence = run(
            json!("Sources changed between dispatches; prepare new plan"),
            json!([]),
        );
        assert_eq!(
            run_status(&sentence),
            "blocked · Sources changed between dispatches; prepare new plan"
        );
    }
    #[test]
    fn a_proposal_says_what_it_changes_from_the_saved_passport() {
        let saved = json!({"id":"orders","description":"Orders","owner":"@team",
            "capabilities":[{"id":"a"}],"provides":[],"consumes":[],"sources":[]});
        let mut proposed = saved.clone();
        assert_eq!(passport_changes(&saved, &proposed), "same as saved");
        proposed["owner"] = json!("unknown");
        proposed["capabilities"] = json!([{"id":"a"},{"id":"b"}]);
        assert_eq!(
            passport_changes(&saved, &proposed),
            "changes owner, capabilities 1 → 2"
        );
        assert_eq!(passport_changes(&V::Null, &proposed), "new passport");
    }
    #[test]
    fn a_scan_that_ended_without_a_proposal_says_why() {
        assert_eq!(
            scan_end(&json!({"status":"cancelled","error":null})),
            "AI scan cancelled."
        );
        assert_eq!(
            scan_end(
                &json!({"status":"failed","error":"The codex CLI exited with code 1: Not signed in"})
            ),
            "The codex CLI exited with code 1: Not signed in."
        );
        assert_eq!(
            scan_end(&json!({"status":"failed"})),
            "AI scan ended without a proposal."
        );
    }
    #[test]
    fn a_message_followed_by_another_ends_as_a_sentence() {
        assert_eq!(
            sentence("Failed to authenticate"),
            "Failed to authenticate."
        );
        assert_eq!(sentence("Scan ended. "), "Scan ended.");
    }
    #[test]
    fn a_revised_plan_gets_the_next_change_id() {
        assert_eq!(
            next_change_id("cancel-after-capture-v3"),
            "cancel-after-capture-v4"
        );
        assert_eq!(
            next_change_id("payments-missing-customer"),
            "payments-missing-customer-2"
        );
        assert_eq!(next_change_id("change-001"), "change-2");
        assert_eq!(next_change_id("7"), "7-2");
    }
    #[test]
    fn the_stepper_shows_where_a_change_stands() {
        let states = |v: V| {
            stages(&v)
                .into_iter()
                .map(|(_, state)| state)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            states(json!({"status":"needs_review","plan":{"status":"needs_review"}})),
            ["done", "current", "upcoming", "upcoming"]
        );
        assert_eq!(
            states(json!({"status":"needs_selection","plan":{"status":"needs_selection"}})),
            ["done", "attention", "upcoming", "upcoming"]
        );
        assert_eq!(
            states(json!({"status":"running","active":true,"session_id":"s"})),
            ["done", "done", "running", "upcoming"]
        );
        assert_eq!(
            states(json!({"status":"blocked","session_id":"s"})),
            ["done", "done", "attention", "upcoming"]
        );
        assert_eq!(
            states(json!({"status":"completed","session_id":"s"})),
            ["done", "done", "done", "done"]
        );
    }
    #[test]
    fn lists_read_as_text() {
        assert_eq!(joined(&json!(["a", "b"])), "a, b");
        assert_eq!(joined(&json!([])), "none");
    }
    #[test]
    fn a_plan_says_how_much_memory_every_agent_shares() {
        assert_eq!(
            shared_memory_summary(&json!({"context":{"sources":[]}})),
            "none"
        );
        let plan = json!({"context":{"sources":[
            {"kind":"memory","trust":"reviewed_memory"},
            {"kind":"memory","trust":"automatic_memory"},
            {"kind":"code","trust":"source_evidence"}]},
            "warnings":[{"reason":"memory_not_shared","count":3}]});
        assert_eq!(
            shared_memory_summary(&plan),
            "2 entries · 1 not reviewed by a person · 3 not shared"
        );
    }
    #[test]
    fn a_handoff_says_what_memory_the_outcome_became() {
        let title = "System change refunds: check receipts";
        assert_eq!(
            handoff_memory(
                &json!({"memory":{"status":"applied","review_mode":"automatic","memory_id":"MEM-1","title":title}})
            ),
            "\nShared as system memory: System change refunds: check receipts (MEM-1). Later changes give it to every agent."
        );
        assert_eq!(
            handoff_memory(
                &json!({"memory":{"status":"proposed","review_mode":"human","proposal_id":"p1","title":title}})
            ),
            "\nProposed as system memory: System change refunds: check receipts (proposal p1). The system folder turns automatic promotion off: approve it with memory promote-review, then promote-apply."
        );
        assert_eq!(
            handoff_memory(
                &json!({"memory":{"status":"proposed","review_mode":"automatic","note":"Promotion source is not eligible"}})
            ),
            "\nThe outcome is not system memory yet: Promotion source is not eligible"
        );
        assert_eq!(
            handoff_memory(&json!({"memory":{"error":"Record content contains a secret"}})),
            "\nThe outcome did not become system memory: Record content contains a secret"
        );
        assert_eq!(handoff_memory(&json!({})), "");
    }
    #[test]
    fn a_waiting_service_agent_reads_the_whole_change() {
        assert_eq!(
            service_agent_goal("payments", &json!("service"), &json!("read-only")),
            "Investigates payments; reads the system folder and every service of this change."
        );
        assert_eq!(
            service_agent_goal("payments", &json!("service"), &json!("edit")),
            "Works on payments; reads the system folder and every service of this change and edits only its own folder."
        );
        assert_eq!(
            service_agent_goal("payments", &json!("all"), &json!("edit")),
            "Works on payments; reads the system folder and every service of this change and edits any selected service."
        );
    }
    #[test]
    fn aggregate_graphs_pill_summarizes_ready_building_and_failed_states() {
        assert_eq!(
            aggregate_graphs_pill(&V::Null),
            ("idle", "Service graphs: loading…".to_owned())
        );
        let building = json!({
            "state": "building",
            "services": {
                "orders": { "state": "ready" },
                "payments": { "state": "building", "build": { "percent": 40 } },
                "catalog": { "state": "empty" }
            }
        });
        assert_eq!(
            aggregate_graphs_pill(&building),
            (
                "building",
                "Service graphs: 1/3 ready · 1 building".to_owned()
            )
        );
        let ready = json!({
            "state": "ready",
            "ready_count": 3,
            "total_count": 3,
            "building_count": 0
        });
        assert_eq!(
            aggregate_graphs_pill(&ready),
            ("ready", "Service graphs: 3/3 ready".to_owned())
        );
    }
    #[test]
    fn monotonic_service_graph_status_clamps_and_never_regresses() {
        let mut cache = BTreeMap::new();
        let s1 = monotonic_service_graph_status(
            "orders",
            &json!({ "state": "building", "build": { "percent": 0 } }),
            &mut cache,
        );
        assert_eq!(s1["build"]["percent"], 1);
        let s2 = monotonic_service_graph_status(
            "orders",
            &json!({ "state": "building", "build": { "percent": 55 } }),
            &mut cache,
        );
        assert_eq!(s2["build"]["percent"], 55);
        let s3 = monotonic_service_graph_status(
            "orders",
            &json!({ "state": "building", "build": { "percent": 22 } }),
            &mut cache,
        );
        assert_eq!(s3["build"]["percent"], 55);
        let done =
            monotonic_service_graph_status("orders", &json!({ "state": "ready" }), &mut cache);
        assert_eq!(done["state"], "ready");
        assert!(!cache.contains_key("orders"));
    }
    #[test]
    fn build_changes_overlay_tracks_plan_blast_radius_and_live_run_waves() {
        let plan_overlay = build_changes_overlay(
            &V::Null,
            true,
            &["orders".to_owned()],
            1,
            "orders.http, order.placed",
        );
        assert_eq!(plan_overlay["stage"], "plan");
        assert_eq!(plan_overlay["depth"], 1);
        assert_eq!(
            plan_overlay["changed_contracts"],
            json!(["orders.http", "order.placed"])
        );

        let run_doc = json!({
            "id": "run-42",
            "active": true,
            "session_id": "sess-1",
            "plan": {
                "depth": 32,
                "context": { "services": ["orders", "inventory"] }
            },
            "execution": {
                "steps": [
                    { "id": "service-orders", "service": "orders", "status": "completed", "attempt": 1 },
                    { "id": "service-inventory", "service": "inventory", "status": "running", "attempt": 1 }
                ]
            },
            "receipts": [],
            "evidence": {
                "cross_service_unopened": ["gateway"]
            }
        });
        let live_overlay = build_changes_overlay(&run_doc, false, &[], 32, "");
        assert_eq!(live_overlay["stage"], "run");
        assert_eq!(
            live_overlay["seed_services"],
            json!(["orders", "inventory"])
        );
        assert_eq!(live_overlay["step_states"]["orders"], "completed");
        assert_eq!(live_overlay["step_states"]["inventory"], "running");
        assert_eq!(live_overlay["cross_service_unopened"], json!(["gateway"]));

        let impact_run_doc = json!({
            "id": "run-43",
            "active": false,
            "plan": {
                "depth": 32,
                "context": { "services": ["billing", "orders", "gateway"] },
                "impact": [
                    { "service": "billing", "reason": "selected", "path": ["billing"] },
                    { "service": "orders", "reason": "potential_consumer", "path": ["billing", "orders"] },
                    { "service": "gateway", "reason": "potential_consumer", "path": ["billing", "orders", "gateway"] }
                ]
            }
        });
        let impact_overlay = build_changes_overlay(&impact_run_doc, false, &[], 32, "");
        assert_eq!(impact_overlay["seed_services"], json!(["billing"]));

        // When `plan.impact` has no root seeds, explicit `plan.selected` and UI `plan_seeds`
        // take precedence over full `plan.context.services`.
        let selected_run_doc = json!({
            "id": "run-44",
            "active": false,
            "plan": {
                "depth": 2,
                "selected": ["orders"],
                "context": { "services": ["billing", "orders", "gateway"] },
                "impact": [
                    { "service": "gateway", "reason": "potential_consumer", "path": ["orders", "gateway"] }
                ]
            }
        });
        let selected_overlay =
            build_changes_overlay(&selected_run_doc, false, &["fallback-ui".to_owned()], 2, "");
        assert_eq!(selected_overlay["seed_services"], json!(["orders"]));

        let no_selected_doc = json!({
            "id": "run-45",
            "active": false,
            "plan": {
                "depth": 2,
                "context": { "services": ["billing", "orders", "gateway"] }
            }
        });
        let ui_seed_overlay =
            build_changes_overlay(&no_selected_doc, false, &["billing".to_owned()], 2, "");
        assert_eq!(ui_seed_overlay["seed_services"], json!(["billing"]));
    }
    #[test]
    fn receipt_changed_files_deduplicates_report_and_evidence_paths() {
        let receipt = json!({
            "phase": "service-orders",
            "report": { "changed_files": ["src/OrderService.php", "src/Dto.php"] },
            "evidence": {
                "changed_files": [
                    { "path": "src/OrderService.php", "status": "M" },
                    { "path": "tests/OrderTest.php", "status": "A" }
                ]
            }
        });
        let (svc, files) = receipt_changed_files(&receipt);
        assert_eq!(svc, "orders");
        assert_eq!(
            files,
            vec!["src/OrderService.php", "src/Dto.php", "tests/OrderTest.php"]
        );
    }
    #[test]
    fn integrated_service_graph_pill_and_scope_switcher_roundtrip() {
        let pill = crate::graph::render_service_graph_pill_html(
            "orders",
            &json!({ "state": "building", "build": { "percent": 42, "phase": "indexing" } }),
        );
        assert!(pill.contains("data-service=\"orders\""));
        assert!(pill.contains("value=\"42\""));

        let scope = crate::graph::GraphTargetScope::SystemService {
            project_id: "proj-1".to_owned(),
            config_path: "system.toml".to_owned(),
            service_id: "orders".to_owned(),
            services: vec![
                "orders".to_owned(),
                "payments".to_owned(),
                "__system__".to_owned(),
            ],
            run_id: Some("run-9".to_owned()),
            run_title: Some("change-001".to_owned()),
            launches: Vec::new(),
        };
        let opts = crate::graph_map::service_switcher_options_html(&scope);
        assert!(opts.contains("__topology__"));
        assert!(opts.contains("__system__"));
        assert!(opts.contains("orders"));
        assert!(opts.contains("payments"));
    }
    #[test]
    fn dispatch_evidence_keys_graph_focus_wrapper_and_agent_visibility_align_with_backend() {
        let receipt = json!({
            "phase": "service-orders",
            "report": { "changed_files": ["src/OrderService.php"] },
            "evidence": {
                "git_changed": ["src/OrderService.php", "routes/api.php"],
                "ledger_edited": [{ "path": "app/Dto/OrderDto.php" }],
                "opened": ["src/OrderService.php", "routes/api.php"],
                "shell_reads": ["config/orders.php"],
                "graph_asked": true
            }
        });
        let (svc, files) = receipt_changed_files(&receipt);
        assert_eq!(svc, "orders");
        assert_eq!(
            files,
            vec![
                "src/OrderService.php",
                "routes/api.php",
                "app/Dto/OrderDto.php"
            ]
        );
        assert_eq!(
            receipt_evidence_summary(&receipt["evidence"]),
            Some(("2 files · 1 shell reads · graph queried".to_owned(), false))
        );
        assert_eq!(
            receipt_evidence_summary(
                &json!({ "opened": [], "shell_reads": [], "graph_asked": false })
            ),
            Some(("0 files · graph not queried".to_owned(), true))
        );

        let wrapped_run = json!({
            "graph_focus": {
                "services": {
                    "orders": { "zone": ["src/OrderService.php"] },
                    "gateway": { "zone": ["src/OrdersClient.ts"] }
                },
                "aggregate": { "services_count": 2 }
            }
        });
        let focus_map = extract_service_graph_focus_map(&wrapped_run);
        assert_eq!(
            focus_map.get("orders"),
            Some(&json!({ "zone": ["src/OrderService.php"] }))
        );
        assert!(!focus_map.contains_key("aggregate"));

        let unrun_plan = json!({
            "status": "needs_review",
            "launches": [],
            "receipts": [],
            "evidence": { "dispatches": [], "files": [] }
        });
        assert!(!run_has_agent_activity(&unrun_plan));

        let cli_run = json!({
            "status": "completed",
            "launches": [],
            "receipts": [],
            "evidence": { "dispatches": [{ "phase": "service-orders" }] }
        });
        assert!(run_has_agent_activity(&cli_run));

        let session_run = json!({
            "status": "running",
            "launches": [{ "session_id": "sess-1" }],
            "receipts": [],
            "evidence": { "dispatches": [] }
        });
        assert!(run_has_agent_activity(&session_run));
    }

    #[test]
    fn the_files_several_steps_edited_are_listed_in_the_order_the_steps_ran() {
        let evidence = json!({"overlaps": [
            {"path": "orders/src/Order.php", "service": "orders", "file": "src/Order.php",
             "steps": ["service-orders", "verify"], "services": ["orders", "__system__"]},
            {"path": "packages/money/Money.php", "service": null, "file": null,
             "steps": ["service-orders", "service-billing"], "services": ["orders", "billing"]},
            {"path": "__system__/docs/a.md", "service": "__system__", "file": "docs/a.md",
             "steps": ["contracts", "verify"], "services": ["__system__", "__system__"]}
        ]});
        assert_eq!(
            overlap_rows(&evidence),
            [
                (
                    "orders/src/Order.php".to_owned(),
                    "service-orders → verify".to_owned(),
                    Some(("orders".to_owned(), "src/Order.php".to_owned()))
                ),
                (
                    "packages/money/Money.php".to_owned(),
                    "service-orders → service-billing".to_owned(),
                    None
                ),
                // The folder of the system has no code map of its own.
                (
                    "__system__/docs/a.md".to_owned(),
                    "contracts → verify".to_owned(),
                    None
                ),
            ]
        );
        assert!(overlap_rows(&json!({})).is_empty());
    }

    #[test]
    fn receipt_render_items_key_by_phase_attempt_and_preserve_accepted_and_overlaps() {
        let receipts = vec![
            json!({ "phase": "contracts", "attempt": 1, "ok": true }),
            json!({ "phase": "verify", "attempt": 1, "ok": false }),
            json!({ "phase": "verify", "attempt": 1, "ok": true }),
        ];
        let steps = vec![json!({
            "id": "verify",
            "attempt": 1,
            "accepted_unrun_checks": ["phpstan"]
        })];
        let overlaps = vec![(
            "orders/src/Order.php".to_owned(),
            "service-orders → verify".to_owned(),
            Some(("orders".to_owned(), "src/Order.php".to_owned())),
        )];
        let items = build_receipt_render_items(&receipts, &steps, &overlaps);
        let keys: Vec<String> = items.iter().map(|i| m::text(i, "key")).collect();
        assert_eq!(
            keys,
            vec![
                "receipt:contracts:1",
                "receipt:verify:1",
                "receipt:verify:1#1",
                "overlaps",
            ]
        );
        assert_eq!(items[0]["accepted"], false);
        assert_eq!(items[1]["accepted"], true);
        assert_eq!(items[3]["kind"], "overlaps");
    }

    #[test]
    fn editor_service_items_key_by_path_with_index_updates_and_collision_fallback() {
        let services = vec![
            json!({ "path": "services/orders", "passport": { "id": "orders" } }),
            json!({ "path": "", "passport": { "id": "draft" } }),
            json!({ "path": "services/orders", "passport": { "id": "orders-dup" } }),
        ];
        let items = build_editor_service_items(&services);
        assert_eq!(items[0]["key"], "path:services/orders");
        assert_eq!(items[0]["index"], 0);
        assert_eq!(items[1]["key"], "idx:1");
        assert_eq!(items[1]["index"], 1);
        assert_eq!(items[2]["key"], "path:services/orders#1");
        assert_eq!(items[2]["index"], 2);
    }

    #[test]
    fn editor_lock_state_offers_discard_only_when_dirty_and_not_scanning() {
        assert_eq!(editor_lock_state(false, false), ("", "", false));
        let (state, msg, show_discard) = editor_lock_state(false, true);
        assert_eq!(state, "dirty");
        assert!(msg.contains("unsaved changes"));
        assert!(show_discard);
        let (scan_state, scan_msg, scan_discard) = editor_lock_state(true, true);
        assert_eq!(scan_state, "scanning");
        assert!(scan_msg.contains("AI scan"));
        assert!(!scan_discard);
    }

    #[test]
    fn agent_launch_options_and_selection_humanize_labels_and_track_latest() {
        let launches = vec![
            json!({ "session_id": "s1", "status": "completed" }),
            json!({ "session_id": "s2", "status": "needs_review" }),
        ];
        let options = build_agent_launch_options(&launches);
        assert_eq!(
            options,
            vec![
                json!({ "id": "s1", "label": "Launch 1 · completed" }),
                json!({ "id": "s2", "label": "Launch 2 · needs review" }),
            ]
        );
        // Preserves user's chosen earlier launch when launch count didn't change
        assert_eq!(
            resolve_agent_launch_selection(&launches, 2, "s1"),
            ("s1".to_owned(), false)
        );
        // Advances to newest launch when a new launch was added
        assert_eq!(
            resolve_agent_launch_selection(&launches, 1, "s1"),
            ("s2".to_owned(), true)
        );
        // Falls back to latest when preferred no longer exists
        assert_eq!(
            resolve_agent_launch_selection(&launches, 2, "missing"),
            ("s2".to_owned(), true)
        );
    }

    #[test]
    fn system_relationship_and_flow_cards_preserve_badges_and_stage_state() {
        let edge = json!({
            "provider": "orders",
            "consumer": "billing",
            "contract": "OrderPaid",
            "version": "2",
            "provenance": "composer_json"
        });
        let rel = build_relationship_view(&edge);
        assert_eq!(rel.provider, "orders");
        assert_eq!(rel.consumer, "billing");
        assert_eq!(rel.contract, "OrderPaid");
        assert_eq!(rel.version, "v2");
        assert_eq!(rel.provenance, "composer json");

        let step = json!({
            "id": "verify",
            "service": "billing",
            "status": "running",
            "mode": "write_patch",
            "attempt": 2,
            "goal": "Validate invoice webhook",
            "depends_on": ["implement"],
            "task_reference": "TASK-042"
        });
        let active_step = build_system_step_view(&step, &json!([]), true, 0);
        assert_eq!(active_step.stage_class, "system-stage running");
        assert_eq!(active_step.row_state, "running");
        assert_eq!(active_step.status_label, "running");
        assert_eq!(active_step.service, "billing");
        assert_eq!(active_step.mode, "write patch");
        assert_eq!(active_step.attempt.as_deref(), Some("attempt 2"));
        assert_eq!(
            active_step.depends_on.as_deref(),
            Some("Depends on: implement")
        );
        assert_eq!(active_step.task_reference.as_deref(), Some("TASK-042"));

        let reported = build_system_step_view(
            &step,
            &json!([{"phase": "verify", "attempt": 2, "ok": true}]),
            false,
            0,
        );
        assert_eq!(reported.stage_class, "system-stage interrupted");
        assert_eq!(
            reported.status_label,
            "reported complete; resume to record it"
        );
    }

    #[test]
    fn system_permissions_hints_payloads_restoration_and_inline_cards_preserve_contracts() {
        let ro_hint = system_permissions_hint("claude", "read-only", "bypass");
        assert!(ro_hint.starts_with("Planning is read-only"));

        let auto_hint = system_permissions_hint("claude", "edit", "auto");
        assert!(auto_hint.contains("auto mode"));
        assert!(auto_hint.contains("cross-service or contract-boundary writes escalate to Manual"));

        let manual_hint = system_permissions_hint("codex", "edit", "manual");
        assert!(manual_hint.contains("Allow and Deny"));
        assert!(!manual_hint.contains("cross-service or contract-boundary writes escalate"));

        let bypass_hint = system_permissions_hint("cursor", "edit", "bypass");
        assert!(bypass_hint.contains("without the sandbox"));

        // Fresh launch restoration avoids sticky bypass while recovery restores saved permission mode.
        assert_eq!(
            restored_system_launch_permission(&json!({"permissions": "bypass"})),
            "auto"
        );
        assert_eq!(
            restored_system_launch_permission(&json!({"permissions": "manual"})),
            "manual"
        );
        assert_eq!(
            restored_system_launch_permission(&json!({"permissions": null})),
            "auto"
        );
        assert_eq!(
            restored_system_recovery_permission(&json!({"permissions": "bypass"})),
            "bypass"
        );
        assert_eq!(restored_system_recovery_permission(&json!({})), "auto");

        let exec = system_execute_payload("claude", "edit", "service", "manual", json!(900), true);
        assert_eq!(exec["permissions"], "manual");
        assert_eq!(exec["mode"], "edit");
        assert_eq!(exec["access"], "service");

        let resume = system_resume_payload(true, true, "bypass", false, "service-billing");
        assert_eq!(resume["permissions"], "bypass");
        assert_eq!(resume["retry_step"], "service-billing");

        let waiting_run = json!({
            "active": true,
            "status": "running",
            "waiting_permissions": [
                {
                    "request_id": "sys-ask-1",
                    "provider": "claude",
                    "tool": "Edit",
                    "summary": "services/billing/src/Invoice.php",
                    "detail": "[Auto -> Manual boundary escalation: service orders touched billing]",
                    "session_option": false,
                    "rule_summary": "Boundary escalation requires confirmation",
                    "diff_preview": "--- a/Invoice.php\n+++ b/Invoice.php\n@@ -1 +1 @@\n-a\n+b\n"
                }
            ]
        });
        assert_eq!(
            run_status(&waiting_run),
            "Waiting for your answer to a permission request…"
        );
        assert_eq!(
            system_permissions_panel_state(&waiting_run),
            (
                true,
                "Waiting for your answer to a permission request…".to_owned()
            )
        );
        assert_eq!(
            system_permissions_panel_state(&json!({"active": true, "waiting_permissions": []})),
            (false, String::new())
        );

        let (remove, live) = crate::run::waiting_permission_reconcile_plan(
            &["sys-ask-0".to_owned(), "sys-ask-1".to_owned()],
            waiting_run["waiting_permissions"].as_array().unwrap(),
        );
        assert_eq!(remove, vec!["sys-ask-0".to_owned()]);
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].0, "sys-ask-1");

        let html = crate::run::render_permission_card_html(
            "system-permissions-list",
            &waiting_run["waiting_permissions"][0],
            true,
        );
        crate::test_support::assert_no_inline_hex_colors(&html);
        assert!(html.contains("data-permission-id=\"sys-ask-1\""));
        assert!(html.contains("Boundary escalation requires confirmation"));
        let payloads = crate::test_support::payloads(&html, "permission-decide");
        assert_eq!(payloads.len(), 2);
        assert_eq!(payloads[0]["container"], "system-permissions-list");
        assert_eq!(payloads[0]["request_id"], "sys-ask-1");
    }

    #[test]
    fn systems_poll_backoff_sse_delta_failure_localization_and_project_sync() {
        use crate::activity::events_card;

        // Transient poll errors retry active/unloaded runs; terminal HTTP status stops.
        let active_run =
            json!({"id": "sys-1", "project_id": "proj-a", "active": true, "status": "running"});
        assert!(system_should_retry_poll(&active_run, "sys-1", 0));
        assert!(system_should_retry_poll(&active_run, "sys-1", 503));
        assert!(!system_should_retry_poll(&active_run, "sys-1", 404));
        assert!(!system_should_retry_poll(&active_run, "sys-1", 403));

        let stopped_run =
            json!({"id": "sys-1", "project_id": "proj-a", "active": false, "status": "failed"});
        assert!(!system_should_retry_poll(&stopped_run, "sys-1", 500));

        // SSE delta merges status/event/completion and rejects foreign projects.
        let mut doc = active_run.clone();
        assert_eq!(
            apply_sse_system_delta(
                &mut doc,
                "proj-a",
                "sys-1",
                &json!({"project_id": "proj-b", "run_id": "sys-1", "status": "failed"})
            ),
            SystemSseOutcome::Ignore
        );
        assert_eq!(
            apply_sse_system_delta(
                &mut doc,
                "proj-a",
                "sys-1",
                &json!({"project_id": "proj-a", "run_id": "sys-1", "event": {"id": "ev-1", "kind": "tool_use", "text": "Running tests"}})
            ),
            SystemSseOutcome::UpdatedRenderOnly
        );
        assert_eq!(
            apply_sse_system_delta(
                &mut doc,
                "proj-a",
                "sys-1",
                &json!({"project_id": "proj-a", "run_id": "sys-1", "status": "failed", "active": false})
            ),
            SystemSseOutcome::RefreshRun {
                immediate: true,
                stopped: true,
            }
        );

        // Failed step & failed receipt build localize request + token-clean button.
        let run_doc = json!({
            "id": "sys-1",
            "project_id": "proj-a",
            "config_path": "system.json",
            "session_id": "sess-7",
            "execution": {
                "steps": [
                    {"id": "orders:impl", "service": "orders", "status": "failed", "goal": "Update OrderController"}
                ]
            },
            "receipts": [
                {
                    "phase": "orders:impl",
                    "attempt": 1,
                    "ok": false,
                    "service": "orders",
                    "message": "PHPUnit failure in app/Http/OrderController.php:55",
                    "report": {
                        "changed_files": ["app/Http/OrderController.php"],
                        "checks": [{"name": "phpunit", "status": "failed", "output": "AssertionError in tests/OrderTest.php:12"}]
                    }
                }
            ]
        });
        let receipts = run_doc["receipts"].as_array().unwrap();
        let steps = run_doc["execution"]["steps"].as_array().unwrap();
        let step_loc = step_localize_payload(&run_doc, &steps[0], receipts, "failed").unwrap();
        assert_eq!(step_loc["project_id"], "proj-a");
        assert_eq!(step_loc["config_path"], "system.json");
        assert_eq!(step_loc["service_id"], "orders");
        assert_eq!(step_loc["run_id"], "sys-1");
        assert_eq!(step_loc["session_id"], "sess-7");
        let hints = step_loc["hint_paths"].as_array().unwrap();
        assert!(hints.iter().any(|p| p == "app/Http/OrderController.php"));
        assert!(hints.iter().any(|p| p == "tests/OrderTest.php"));

        let items = build_receipt_render_items_with_run(&run_doc, receipts, steps, &[]);
        assert!(items[0]["localize"].is_object());
        assert_eq!(failed_services_in_run(&run_doc), vec!["orders".to_owned()]);

        let btn_html = render_system_step_actions_html_for_test(&step_loc);
        assert!(btn_html.contains("system-step-actions"));
        assert!(btn_html.contains("Localize failure on map"));
        crate::test_support::assert_no_inline_hex_colors(&btn_html);

        // Global project switch respects dirty/pending guards cleanly.
        m::set("editor-dirty", json!(false));
        m::set("editor-pending", json!(false));
        m::set("system-pending", json!(false));
        m::set("discovery-active", json!(false));
        m::set("system-project-sync", json!("proj-a"));
        on_global_project_change("proj-b");
        assert_eq!(m::string(&m::data("system-project-sync")), "proj-b");

        m::set("editor-dirty", json!(true));
        on_global_project_change("proj-blocked");
        assert_eq!(m::string(&m::data("system-project-sync")), "proj-b");
        assert_eq!(
            m::string(&m::data("system-pending-project-sync")),
            "proj-blocked"
        );
        events_card::set_sse_connected_for_tests(false);
        m::take("editor-dirty");
        m::take("system-project-sync");
        m::take("system-pending-project-sync");
    }

    #[test]
    fn optional_missing_catalog_response_is_recognized_without_error() {
        assert!(crate::systems::catalog_response_is_missing(
            &json!({"missing": true, "catalog": null})
        ));
        assert!(crate::systems::catalog_response_is_missing(
            &json!({"catalog": null})
        ));
        assert!(!crate::systems::catalog_response_is_missing(
            &json!({"catalog": {"system": "demo", "services": []}})
        ));
    }

    #[test]
    fn systems_view_routes_and_waiting_permissions_counts_and_arrays_work_cleanly() {
        assert!(is_systems_view("systems"));
        assert!(is_systems_view("system-changes"));
        assert!(!is_systems_view("services"));
        assert!(!is_systems_view("changes"));

        let mut run_doc = json!({
            "id": "run-1",
            "project_id": "proj-a",
            "active": true,
            "status": "running",
            "waiting_permissions": [],
            "events": []
        });

        // Count SSE frame arrives before hydrated objects -> updates doc & requests immediate refresh.
        let outcome = apply_sse_system_delta(
            &mut run_doc,
            "proj-a",
            "run-1",
            &json!({"run_id": "run-1", "project_id": "proj-a", "waiting_permissions": 2}),
        );
        assert_eq!(
            outcome,
            SystemSseOutcome::RefreshRun {
                immediate: true,
                stopped: false,
            }
        );
        assert_eq!(run_doc["waiting_permissions"], json!(2));
        assert_eq!(
            run_status(&run_doc),
            "Waiting for your answer to a permission request…"
        );
        assert_eq!(
            system_permissions_panel_state(&run_doc),
            (
                true,
                "2 permission requests waiting for your decision.".into()
            )
        );

        // Once hydrated with 2 ask objects, a delta with count=2 preserves objects and renders event-only.
        run_doc["waiting_permissions"] = json!([
            {"request_id": "p1", "tool": "Edit", "summary": "edit a.php"},
            {"request_id": "p2", "tool": "Bash", "summary": "phpunit"}
        ]);
        let ev_outcome = apply_sse_system_delta(
            &mut run_doc,
            "proj-a",
            "run-1",
            &json!({
                "run_id": "run-1",
                "project_id": "proj-a",
                "waiting_permissions": 2,
                "event": {"id": 9, "kind": "status", "text": "waiting"}
            }),
        );
        assert_eq!(ev_outcome, SystemSseOutcome::UpdatedRenderOnly);
        assert!(run_doc["waiting_permissions"].is_array());
        assert_eq!(run_doc["waiting_permissions"].as_array().unwrap().len(), 2);

        // Count 0 clears waiting_permissions back to empty array.
        let _ = apply_sse_system_delta(
            &mut run_doc,
            "proj-a",
            "run-1",
            &json!({"run_id": "run-1", "project_id": "proj-a", "waiting_permissions": 0}),
        );
        assert_eq!(run_doc["waiting_permissions"], json!([]));
        assert_eq!(
            system_permissions_panel_state(&run_doc),
            (false, String::new())
        );

        // inspect-system-file-graph extracts symbol target priority cleanly.
        assert_eq!(
            extract_system_file_symbol(&json!({
                "service": "web",
                "path": "src/a.rs",
                "symbol_fqn": "src/a.rs::Foo::bar",
                "symbol": "bar",
                "from_symbols": ["OrdersClient::place"],
                "symbol_id": "sym:1"
            }))
            .as_deref(),
            Some("src/a.rs::Foo::bar")
        );
        assert_eq!(
            extract_system_file_symbol(&json!({
                "service": "web",
                "path": "src/a.rs",
                "from_symbols": ["OrdersClient::place"],
                "symbol_id": "sym:1"
            }))
            .as_deref(),
            Some("OrdersClient::place")
        );
        assert_eq!(
            extract_system_file_symbol(&json!({
                "service": "web",
                "path": "src/a.rs",
                "symbol_id": "sym:1"
            }))
            .as_deref(),
            Some("sym:1")
        );
        assert_eq!(
            extract_system_file_symbol(&json!({"service": "web", "path": "src/a.rs"})),
            None
        );
    }
}
