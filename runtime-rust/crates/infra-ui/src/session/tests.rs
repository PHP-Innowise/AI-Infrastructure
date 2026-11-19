#![allow(unused_imports)]
use super::*;

#[cfg(test)]
mod history_projection_tests {
    use super::*;

    #[test]
    fn sidebar_projection_preserves_labels_order_and_selection() {
        let sessions = vec![
            json!({"id":"a","name":"Named chat","title":"Unused title","status":"running"}),
            json!({"id":"b","title":"Second chat","status":"awaiting_approval"}),
            json!({"id":"c","title":"Background ask","status":"running","waiting_permissions":2}),
        ];
        assert_eq!(
            history_projection(&sessions, "b"),
            json!([
                {
                    "id":"a",
                    "title":"Named chat",
                    "status":"running",
                    "status_text":"running",
                    "waiting_permissions":0,
                    "label":"Named chat · running",
                    "selected":false
                },
                {
                    "id":"b",
                    "title":"Second chat",
                    "status":"awaiting_approval",
                    "status_text":"awaiting approval",
                    "waiting_permissions":0,
                    "label":"Second chat · awaiting approval",
                    "selected":true
                },
                {
                    "id":"c",
                    "title":"Background ask",
                    "status":"running",
                    "status_text":"2 approvals waiting",
                    "waiting_permissions":2,
                    "label":"Background ask · 2 approvals waiting",
                    "selected":false
                }
            ])
        );
        let original = history_projection(&sessions, "b");
        assert_ne!(original, history_projection(&sessions, "a"));
        let mut reversed = sessions.clone();
        reversed.reverse();
        assert_ne!(original, history_projection(&reversed, "b"));
        assert_eq!(history_projection(&[], "b"), json!([]));
    }

    #[test]
    fn sidebar_changes_only_when_rendered_fields_change() {
        let session = json!({"id":"a","title":"Chat","status":"running"});
        let original = history_projection(std::slice::from_ref(&session), "a");
        let mut report_changed = session.clone();
        report_changed["budget_usage"] = json!({"tokens":900});
        report_changed["fleet_result"] = json!({"report":"new report"});
        assert_eq!(original, history_projection(&[report_changed], "a"));
        for (field, value) in [
            ("id", json!("b")),
            ("title", json!("Renamed chat")),
            ("name", json!("Preferred name")),
            ("status", json!("completed")),
            ("waiting_permissions", json!(1)),
        ] {
            let mut changed = session.clone();
            changed[field] = value;
            assert_ne!(original, history_projection(&[changed], "a"), "{field}");
        }
        assert_eq!(
            history_projection(&[json!({"id":"fallback"})], ""),
            json!([{
                "id":"fallback",
                "title":"fallback",
                "status":"",
                "status_text":"",
                "waiting_permissions":0,
                "label":"fallback · ",
                "selected":false
            }])
        );
    }
}

#[cfg(test)]
mod session_contract_tests {
    use super::*;
    #[test]
    fn a_new_draft_never_starts_with_bypass_permissions() {
        assert_eq!(restored_value("permissions", "bypass"), "auto");
        assert_eq!(restored_value("permissions", "manual"), "manual");
        assert_eq!(restored_value("mode", "bypass"), "bypass");
    }
    #[test]
    fn read_only_planning_hint_never_shows_bypass_warning_style() {
        assert_eq!(permissions_hint_mode(true, "plan", "bypass"), "plan");
        assert_eq!(permissions_hint_mode(false, "plan", "bypass"), "plan");
        assert_eq!(permissions_hint_mode(false, "edit", "bypass"), "bypass");
        assert_eq!(permissions_hint_mode(false, "edit", "manual"), "manual");
    }
    #[test]
    fn waiting_banner_matches_permission_wait_state() {
        assert_eq!(
            waiting_banner_text("queued", true),
            "Queued — waiting for an available runner."
        );
        assert_eq!(
            waiting_banner_text("running", true),
            "Waiting for your answer to a permission request…"
        );
        assert_eq!(waiting_banner_text("running", false), "Agent is running.");
    }
    #[test]
    fn session_summary_formats_provider_mode_permissions_and_model() {
        assert_eq!(
            format_session_summary("Claude Code", "edit", "manual", "claude-sonnet-4-6", false),
            "Claude Code · Edit · Manual · claude-sonnet-4-6"
        );
        assert_eq!(
            format_session_summary("Codex", "plan", "bypass", "", false),
            "Codex · Plan · Bypass Permissions · Provider default"
        );
        assert_eq!(
            format_session_summary("Cursor", "edit", "auto", "gpt-5", true),
            "Cursor · Edit · Auto · Routing (Plan + Edit)"
        );
    }
    #[test]
    fn resize_percent_maps_configuration_height_inversely_to_output_height() {
        assert_eq!(output_resize_percent(80., 480.), 100);
        assert_eq!(output_resize_percent(480., 480.), 0);
        assert_eq!(output_resize_percent(280., 480.), 50);
        assert_eq!(output_resize_percent(40., 480.), 100);
        assert_eq!(output_resize_percent(600., 480.), 0);
        assert_eq!(output_resize_percent(80., 80.), 100);
    }
    #[test]
    fn provider_sign_in_note_formats_signed_out_and_clears_when_signed_in() {
        let sign_in = json!({
            "states": {
                "claude": {"state": "signed_out", "login": "claude login"},
                "codex": {"state": "signed_in", "login": "codex login"},
            }
        });
        assert_eq!(
            provider_sign_in_note(&sign_in, "claude", "Claude Code"),
            "Claude Code is not signed in. Sign in from a terminal with claude login, then send again."
        );
        assert_eq!(provider_sign_in_note(&sign_in, "codex", "Codex"), "");
        assert_eq!(provider_sign_in_note(&V::Null, "claude", "Claude Code"), "");
    }
    #[test]
    fn poll_apply_settings_skips_dirty_and_unchanged_groups() {
        let session = json!({"id":"s1","provider":"claude","workflow":"native","workspace":"project","mode":"edit","permissions":"auto","agents_enabled":false,"agent_count":3,"model":"sonnet","thinking_effort":"medium","sdd":{"feature":"","phase":"specify"},"clash":null});
        let fields = session_settings_snapshot_fields(&session);
        let empty = V::Null;
        assert!(should_apply_setting_group(
            &empty,
            &empty,
            1,
            "s1",
            "permissions",
            &fields["permissions"]
        ));
        let snapshot = json!({"epoch": 1, "sid": "s1", "fields": fields});
        assert!(!should_apply_setting_group(
            &snapshot,
            &empty,
            1,
            "s1",
            "permissions",
            &fields["permissions"]
        ));
        // Server change applies when clean, and is suppressed when locally dirty.
        let manual = json!("manual");
        assert!(should_apply_setting_group(
            &snapshot,
            &empty,
            1,
            "s1",
            "permissions",
            &manual
        ));
        let dirty = json!({"permissions": true});
        assert!(!should_apply_setting_group(
            &snapshot,
            &dirty,
            1,
            "s1",
            "permissions",
            &manual
        ));
        // Session/epoch switch resets and applies again.
        assert!(should_apply_setting_group(
            &snapshot,
            &dirty,
            2,
            "s1",
            "permissions",
            &manual
        ));
        assert!(should_apply_setting_group(
            &snapshot,
            &dirty,
            1,
            "s2",
            "permissions",
            &manual
        ));
    }
    #[test]
    fn the_permissions_hint_says_what_each_mode_does_for_the_host() {
        // Planning never asks, whatever the setting.
        for permission in ["auto", "manual", "bypass"] {
            assert!(
                permissions_hint("claude", "plan", permission).starts_with("Planning is read-only")
            );
        }
        assert!(permissions_hint("claude", "edit", "auto").contains("auto mode"));
        assert!(permissions_hint("codex", "edit", "auto").contains("automatic reviewer"));
        assert!(permissions_hint("cursor", "edit", "auto").contains("sandbox"));
        for provider in ["claude", "codex", "cursor"] {
            assert!(permissions_hint(provider, "edit", "manual").contains("Allow and Deny"));
            assert!(permissions_hint(provider, "edit", "bypass").contains("without the sandbox"));
        }
        assert!(permissions_hint("codex", "edit", "manual").contains("read-only"));
    }
    #[test]
    fn merge_settings_never_replace_the_saved_ordinary_project_draft() {
        let previous = json!({"drafts":{"a":{"workflow":"sdd","prompt":"ordinary work"}},"project_id":"a","session_id":null});
        let merge = json!({"project_id":"a","selection_epoch":4,"sources":["x","y"]});
        assert!(preferences_update(
            previous.clone(),
            "a",
            "",
            json!({"workflow":"native","prompt":m::MERGE_PROMPT}),
            &merge
        )
        .is_none());
        let next =
            preferences_update(previous, "a", "existing", json!({"prompt":""}), &V::Null).unwrap();
        assert_eq!(next["drafts"]["a"]["workflow"], "sdd");
        assert_eq!(next["drafts"]["a"]["prompt"], "ordinary work");
        assert_eq!(next["session_id"], "existing");
    }
    #[test]
    fn a_merge_draft_ends_on_session_project_or_selection_generation_change() {
        let draft = json!({"project_id":"a","selection_epoch":4,"sources":["x","y"]});
        assert!(merge_draft_owned(&draft, "", "a", 4));
        assert!(!merge_draft_owned(&draft, "chat", "a", 4));
        assert!(!merge_draft_owned(&draft, "", "b", 4));
        assert!(!merge_draft_owned(&draft, "", "a", 6));
    }
    #[test]
    fn budget_payload_contains_worker_timeout_only_for_fleet() {
        let ordinary = budget_request(
            &json!({"workflow":"native","budget_revision":2}),
            json!({"tokens":100}),
            json!(300),
        );
        assert!(ordinary.get("worker_timeout").is_none());
        let fleet = budget_request(
            &json!({"workflow":"fleet-review","fleet":{},"budget_revision":3}),
            json!({"usd":5}),
            json!(17),
        );
        assert_eq!(fleet["worker_timeout"], 17);
        assert_eq!(fleet["revision"], 3);
        assert_eq!(fleet["budgets"]["usd"], 5);
    }
}

#[cfg(test)]
mod draft_project_tests {
    use super::*;
    #[test]
    fn a_direct_project_selection_saves_the_previous_form_under_its_bound_project() {
        m::set("composer-project", json!("a"));
        let owner = draft_project("b");
        let previous = json!({"project_id":"a","session_id":"old-chat","drafts":{
            "a":{"prompt":"old a","bank":"bank-a","task":"task-a"},
            "b":{"prompt":"keep b","bank":"bank-b","task":"task-b","extra":{"retain":true}}
        }});
        let draft_b = previous["drafts"]["b"].clone();
        // The project select already says B while these extracted controls still belong to A.
        let extracted_a = json!({"prompt":"edited a","bank":"bank-a","task":"task-a"});
        let saved =
            preferences_update(previous, &owner, "", extracted_a.clone(), &V::Null).unwrap();
        assert_eq!(saved["drafts"]["a"], extracted_a);
        assert_eq!(saved["drafts"]["b"], draft_b);
        m::set("composer-project", json!("b"));
        let owner = draft_project("b");
        assert_eq!(owner, "b");
        // Selection persistence must not extract the still-unrestored A bank/task controls again.
        let persisted = preferences_selection(saved, &owner, "");
        assert_eq!(persisted["project_id"], "b");
        assert!(persisted["session_id"].is_null());
        assert_eq!(persisted["drafts"]["a"], extracted_a);
        assert_eq!(persisted["drafts"]["b"], draft_b);
        m::take("composer-project");
    }
}

#[cfg(test)]
mod update_tests {
    use super::*;
    fn status(state: &str, detail: &str) -> V {
        json!({"state":state,"detail":detail,"behind":0,"ahead":0,"commits":[],"upstream":"origin/main","version":"6e5a8cc"})
    }
    #[test]
    fn an_available_update_shows_the_button_and_its_commits_on_hover() {
        let mut available = status("available", "12 new changes on origin/main.");
        available["behind"] = json!(12);
        available["commits"] = (1..=12)
            .map(|n| json!({"hash":format!("abc{n}"),"subject":format!("change {n}")}))
            .collect();
        let view = update_view(&available, &V::Null);
        assert!(view.button && !view.busy);
        assert_eq!(view.label, "Update");
        let lines = view.title.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 12);
        assert_eq!(lines[0], "12 new changes on origin/main.");
        assert_eq!(lines[1], "• change 1");
        assert_eq!(lines[10], "• change 10");
        assert_eq!(lines[11], "… and 2 more");
        assert!(view.note.is_empty());
        let mut one = status("available", "1 new changes on origin/main.");
        one["behind"] = json!(1);
        one["commits"] = json!([{"hash":"abc1","subject":"docs: one"}]);
        assert_eq!(
            update_view(&one, &V::Null).title,
            "1 new changes on origin/main.\n• docs: one"
        );
    }
    #[test]
    fn nothing_is_offered_without_an_update_whatever_else_the_status_says() {
        for state in [
            "unknown",
            "current",
            "unavailable",
            "updating",
            "updated",
            "",
        ] {
            let mut value = status(state, "This clone is not on a branch.");
            // The server never sends this key; the button follows the state alone.
            value["available"] = json!(true);
            let view = update_view(&value, &V::Null);
            assert!(!view.button, "{state}");
            assert!(view.note.is_empty() && view.title.is_empty(), "{state}");
        }
        assert!(!update_view(&V::Null, &V::Null).button);
    }
    #[test]
    fn a_diverged_failed_or_held_update_says_why_in_the_banner() {
        let diverged = update_view(
            &status(
                "diverged",
                "2 new changes on origin/main; this clone has 1 commits of its own. Update it with Git.",
            ),
            &V::Null,
        );
        assert!(!diverged.button && !diverged.error);
        assert!(diverged.note.ends_with("Update it with Git."));
        let failed = update_view(
            &status(
                "failed",
                "The update could not be applied: Git exited with status 128.",
            ),
            &V::Null,
        );
        assert!(!failed.button && failed.error);
        assert_eq!(
            failed.note,
            "The update could not be applied: Git exited with status 128."
        );
        let held = update_view(
            &status("restart-required", "The clone is now at 43a21a0, but its browser interface was built from other sources. In the clone, run `./infra-engine repo build-ui`, then press Update again."),
            &V::Null,
        );
        assert!(held.button && !held.busy && held.error);
        assert_eq!(held.label, "Update");
        assert!(held.note.contains("press Update again"));
    }
    #[test]
    fn what_this_page_did_takes_over_the_button_and_the_banner() {
        let available = status("available", "1 new changes on origin/main.");
        let pending = update_view(&available, &json!({"pending":true}));
        assert!(pending.button && pending.busy);
        assert_eq!(pending.label, "Updating…");
        let restarting = update_view(
            &status("updated", ""),
            &json!({"restarting":true,"note":"Updated to 43a21a0. The Harness is restarting; this page reloads by itself."}),
        );
        assert!(restarting.button && restarting.busy && !restarting.error);
        assert_eq!(restarting.label, "Restarting…");
        assert!(restarting.note.starts_with("Updated to 43a21a0."));
        // A refused update keeps its message, also over the detail the server recorded.
        let refused = update_view(
            &status("restart-required", "recorded detail"),
            &json!({"failed":true,"note":"An update is waiting for the Harness to restart."}),
        );
        assert!(refused.button && !refused.busy && refused.error);
        assert_eq!(
            refused.note,
            "An update is waiting for the Harness to restart."
        );
        let current = update_view(
            &status("current", ""),
            &json!({"note":"This clone is already up to date."}),
        );
        assert!(!current.button && !current.error);
        assert_eq!(current.note, "This clone is already up to date.");
    }
    #[test]
    fn commands_quoted_in_a_note_become_code() {
        assert_eq!(
            code_spans(
                "In the clone, run `cargo build --release`, then `./infra-engine repo build-ui`."
            ),
            vec![
                (false, "In the clone, run "),
                (true, "cargo build --release"),
                (false, ", then "),
                (true, "./infra-engine repo build-ui"),
                (false, "."),
            ]
        );
        assert_eq!(code_spans("`a`"), vec![(true, "a")]);
        assert_eq!(
            code_spans("an unpaired ` stays"),
            vec![(false, "an unpaired ` stays")]
        );
        assert!(code_spans("").is_empty());
    }
    #[test]
    fn working_indicator_reports_idle_and_active_operations() {
        let null = V::Null;
        let idle = WorkingSignals {
            selected: &null,
            sessions: &[],
            pending: false,
            booting: false,
            update_page: &null,
            results: &null,
            delivery_preview: &null,
            creator_pending: false,
            creator_run: &null,
            system_pending: false,
            editor_pending: false,
            discovery_active: false,
            system_run_active: false,
            graph_building: false,
            knowledge_pending: false,
            skills_pending: false,
            setup_pending: false,
            create_skill_pending: false,
            sign_in_pending: false,
            sdd_pending: false,
            permission_waiting: false,
        };
        assert_eq!(working_message(&idle), None);
        let asking = json!({"id":"s1","status":"running"});
        assert_eq!(
            working_message(&WorkingSignals {
                selected: &asking,
                permission_waiting: true,
                ..idle
            }),
            Some("Waiting for your answer to a permission request…")
        );

        let running = json!({"id":"s1","status":"running"});
        assert_eq!(
            working_message(&WorkingSignals {
                selected: &running,
                ..idle
            }),
            Some("Agent is working on your task…")
        );

        let queued = json!({"id":"s1","status":"queued"});
        assert_eq!(
            working_message(&WorkingSignals {
                selected: &queued,
                ..idle
            }),
            Some("Queued — waiting for an available runner…")
        );

        assert_eq!(
            working_message(&WorkingSignals {
                pending: true,
                ..idle
            }),
            Some("Working on your request…")
        );

        let creator_scan = json!({"status":"scanning"});
        assert_eq!(
            working_message(&WorkingSignals {
                creator_run: &creator_scan,
                ..idle
            }),
            Some("Generator is running…")
        );

        assert_eq!(
            working_message(&WorkingSignals {
                discovery_active: true,
                ..idle
            }),
            Some("Scanning project services…")
        );

        assert_eq!(
            working_message(&WorkingSignals {
                graph_building: true,
                ..idle
            }),
            Some("Building dependency map…")
        );

        let background = [json!({"id":"s2","status":"running"})];
        assert_eq!(
            working_message(&WorkingSignals {
                sessions: &background,
                ..idle
            }),
            Some("Another session is running…")
        );
    }

    #[test]
    fn poll_focus_selection_preserves_prompt_prediction_and_streams_updates() {
        let running = json!({"id": "s1", "status": "running"});
        let predicting = json!({
            "phase": "predicting",
            "target_paths": ["src/Domain/Service.php"]
        });
        assert_eq!(
            select_poll_focus(&running, &[], true, &V::Null),
            FocusUpdate::Keep
        );

        let completed = json!({"id": "s1", "status": "completed"});
        assert_eq!(
            select_poll_focus(&completed, &[], true, &V::Null),
            FocusUpdate::Clear
        );
        assert_eq!(
            select_poll_focus(&completed, &[], false, &V::Null),
            FocusUpdate::Keep
        );

        let idle_session = json!({
            "id": "s1",
            "status": "running",
            "graph_focus": {"phase": "idle", "created_entities": []}
        });
        assert_eq!(
            select_poll_focus(&idle_session, &[], true, &predicting),
            FocusUpdate::Keep
        );
        assert_eq!(
            select_poll_focus(&idle_session, &[], false, &V::Null),
            FocusUpdate::Apply(idle_session["graph_focus"].clone())
        );

        let nested_event = json!({
            "id": 2,
            "kind": "graph_focus",
            "graph_focus": {
                "phase": "active",
                "target_paths": ["src/app.php"],
                "created_entities": [{"id": "e1", "kind": "class", "name": "App"}]
            }
        });
        assert_eq!(
            select_poll_focus(
                &running,
                std::slice::from_ref(&nested_event),
                true,
                &predicting
            ),
            FocusUpdate::Apply(nested_event["graph_focus"].clone())
        );

        let inline_event = json!({
            "id": 3,
            "kind": "graph_focus",
            "phase": "active",
            "target_folders": ["src/Domain"],
            "created_entities": [{"id": "e2", "kind": "method", "name": "handle"}]
        });
        assert_eq!(
            select_poll_focus(
                &idle_session,
                &[nested_event, inline_event.clone()],
                false,
                &predicting
            ),
            FocusUpdate::Apply(inline_event)
        );

        let active_session = json!({
            "id": "s1",
            "status": "completed",
            "graph_focus": {
                "phase": "completed",
                "summary": "2 entities created",
                "target_paths": ["src/Domain/Service.php"]
            }
        });
        assert_eq!(
            select_poll_focus(&active_session, &[], false, &predicting),
            FocusUpdate::Apply(active_session["graph_focus"].clone())
        );
    }

    #[test]
    fn poll_focus_preserves_created_and_modified_entity_change_kinds() {
        let predicting = json!({
            "phase": "predicting",
            "target_paths": ["src/app.rs"]
        });
        let running = json!({"id": "s1", "status": "running"});
        let nested_event = json!({
            "id": 10,
            "kind": "graph_focus",
            "graph_focus": {
                "phase": "idle",
                "target_paths": ["src/app.rs", "src/new.rs"],
                "created_entities": [
                    {"id": "e1", "kind": "function", "name": "boot", "change": "created"},
                    {"id": "e2", "kind": "struct", "name": "Config", "change": "modified"}
                ]
            }
        });
        assert_eq!(
            select_poll_focus(
                &running,
                std::slice::from_ref(&nested_event),
                false,
                &predicting
            ),
            FocusUpdate::Apply(nested_event["graph_focus"].clone())
        );

        let inline_event = json!({
            "id": 11,
            "kind": "graph_focus",
            "phase": "active",
            "created_entities": [
                {"id": "e3", "kind": "method", "name": "save", "change": "modified"}
            ]
        });
        let extracted = event_focus_payload(&inline_event).expect("inline graph_focus payload");
        assert_eq!(extracted["created_entities"][0]["change"], "modified");
    }

    #[test]
    fn merge_click_payload_forwards_graph_data_attributes_without_clobbering() {
        let attrs = ClickAttrs {
            resolved_id: "session-graph-side".into(),
            resolved_action: "graph-impact-row".into(),
            path: Some("src/session.rs".into()),
            from: Some("src/lib.rs".into()),
            to: Some("src/session.rs".into()),
            filter: Some("session".into()),
            tab: Some("changes".into()),
            fallback_payload: None,
        };
        assert_eq!(
            merge_click_payload(&V::Null, &attrs, true),
            json!({
                "path": "src/session.rs",
                "from": "src/lib.rs",
                "to": "src/session.rs",
                "filter": "session",
                "tab": "changes",
                "surface": "session"
            })
        );

        let existing = json!({"path": "custom/file.rs", "surface": "modal"});
        assert_eq!(
            merge_click_payload(&existing, &attrs, true),
            json!({
                "path": "custom/file.rs",
                "surface": "modal",
                "from": "src/lib.rs",
                "to": "src/session.rs",
                "filter": "session",
                "tab": "changes"
            })
        );

        assert_eq!(
            merge_click_payload(&json!("sess-123"), &ClickAttrs::default(), false),
            json!("sess-123")
        );
    }

    #[test]
    fn sync_embedded_changes_triggers_only_on_relevant_updates() {
        assert!(!should_sync_embedded_changes(&[], false, false, false));
        assert!(!should_sync_embedded_changes(&[], false, true, true));
        assert!(should_sync_embedded_changes(&[], false, true, false));
        assert!(should_sync_embedded_changes(&[], true, true, true));

        let focus_ev = json!({"kind": "graph_focus", "phase": "active"});
        assert!(should_sync_embedded_changes(&[focus_ev], false, true, true));

        let done_status = json!({"kind": "status", "status": "completed"});
        assert!(should_sync_embedded_changes(
            &[done_status],
            false,
            true,
            true
        ));

        let started_edit = json!({"kind": "tool", "tool": "Edit", "state": "started"});
        assert!(!should_sync_embedded_changes(
            &[started_edit],
            false,
            true,
            true
        ));

        let finished_edit = json!({"kind": "tool", "tool": "Edit", "state": "completed"});
        assert!(should_sync_embedded_changes(
            &[finished_edit],
            false,
            true,
            true
        ));

        let file_change = json!({
            "kind": "tool",
            "tool": "file_change",
            "state": "completed",
            "changes": [{"path": "src/session.rs"}]
        });
        assert!(should_sync_embedded_changes(
            &[file_change],
            false,
            true,
            true
        ));

        let failed_edit = json!({"kind": "tool", "tool": "Edit", "state": "failed", "ok": false});
        assert!(!should_sync_embedded_changes(
            &[failed_edit],
            false,
            true,
            true
        ));
    }

    #[test]
    fn extract_inspect_file_path_handles_object_and_string_payloads() {
        assert_eq!(
            extract_inspect_file_path(&json!({"path": " src/session.rs "})).as_deref(),
            Some("src/session.rs")
        );
        assert_eq!(
            extract_inspect_file_path(&json!("src/graph_map.rs")).as_deref(),
            Some("src/graph_map.rs")
        );
        assert_eq!(extract_inspect_file_path(&json!({"path": "   "})), None);
        assert_eq!(extract_inspect_file_path(&V::Null), None);
    }

    #[test]
    fn normalize_mobile_pane_defaults_to_chat_and_accepts_graph() {
        assert_eq!(normalize_mobile_pane("graph"), "graph");
        assert_eq!(normalize_mobile_pane("chat"), "chat");
        assert_eq!(normalize_mobile_pane(""), "chat");
        assert_eq!(normalize_mobile_pane("unknown"), "chat");
    }

    #[test]
    fn preflight_request_detects_at_files_and_keeps_risk_caller_hints_opt_in() {
        assert!(has_at_file_mention("Check @src/session.rs before running"));
        assert!(has_at_file_mention("Review (@Cargo.toml)"));
        assert!(!has_at_file_mention("Email me at user@example.com"));
        assert!(!has_at_file_mention("Plain prompt without path"));

        let default_req = build_preflight_request("Check @src/run.rs", "", "", false);
        assert_eq!(default_req["prompt"], "Check @src/run.rs");
        assert_eq!(default_req["risk_caller_hints"], false);
        assert!(default_req.get("session_id").is_none());
        assert!(default_req.get("task_id").is_none());

        let opt_in_req = build_preflight_request("Follow up", "sess-1", "TASK-42", true);
        assert_eq!(opt_in_req["session_id"], "sess-1");
        assert_eq!(opt_in_req["task_id"], "TASK-42");
        assert_eq!(opt_in_req["risk_caller_hints"], true);
    }

    #[test]
    fn advisory_panel_renders_preflight_and_delivery_warnings_without_inline_hex() {
        let preflight = json!({
            "files": [{
                "path": "src/auth.rs",
                "brain_records": [{"id": "rec-auth-1", "external_id": "finding-auth", "title": "Auth contract"}],
                "memory_chunks": [{"id": "MEM-20261117-aabbccdd", "title": "Token guard note"}]
            }],
            "blast_summary": {"direct_count": 2, "downstream_count": 5, "files": ["src/auth.rs", "src/api.rs"]},
            "contradiction_warnings": [{
                "kind": "atom-contradiction",
                "text": "Token TTL <24h> contradicts 5m",
                "path": "src/auth.rs",
                "open": [{"scope": "brain", "target_id": "rec-1", "title": "Auth decision"}]
            }],
            "hygiene_warnings": [{
                "kind": "cited-file-renamed",
                "path": "src/auth.rs",
                "linked": ["MEM-20261117-aabbccdd"],
                "text": "Renamed from src/old_auth.rs"
            }],
            "risk_callers": [{
                "from": "src/routes.rs",
                "to": "src/auth.rs",
                "impact": "broken_risk",
                "reason": "Calls removed validate()"
            }],
            "risk_caller_hints": [{
                "path": "src/routes.rs",
                "target": "src/auth.rs",
                "impact": "impacted",
                "brain_records": [{"id": "rec-routes-1", "title": "Routes handler note"}],
                "memory_chunks": [{"id": "MEM-20261118-cccc4444", "title": "Route caller contract"}],
                "hygiene_warnings": [],
                "source_hints": [{"path": "memory-bank/chunks/MEM-20261118-cccc4444-caller-contract.md", "source_sha256": "deadbeef"}]
            }]
        });
        let html = render_advisory_panel_html("proj-1", &preflight, true);
        crate::test_support::assert_no_inline_hex_colors(&html);
        assert!(html.contains("2 direct"));
        assert!(html.contains("5 downstream"));
        assert!(html.contains("1 hint(s)"));
        assert!(html.contains("data-action=\"inspect-file-graph\""));
        assert!(html.contains("data-action=\"preflight-open-memory\""));
        assert!(!html.contains("preflight-open-knowledge"));
        assert!(html.contains("Open Auth contract"));
        assert!(html.contains("Open Token guard note"));
        assert!(html.contains("Advisory risk-caller hints"));
        assert!(html.contains("Open Routes handler note"));
        assert!(html.contains("Open Route caller contract"));
        assert!(
            html.contains("Sources: memory-bank/chunks/MEM-20261118-cccc4444-caller-contract.md")
        );
        assert!(html.contains("Token TTL &lt;24h&gt; contradicts 5m"));
        assert!(html.contains("src/routes.rs → src/auth.rs"));

        // Delivery preview returns empty when no memory_warnings or risk_callers exist.
        assert_eq!(
            render_advisory_panel_html("proj-1", &json!({"kind": "commit"}), false),
            ""
        );

        let delivery_v = json!({
            "kind": "transfer",
            "memory_warnings": {
                "contradiction_warnings": [],
                "hygiene_warnings": [{
                    "kind": "cited-file-missing",
                    "path": "src/legacy.rs",
                    "linked": ["rec-9"],
                    "text": "File missing in workspace"
                }]
            },
            "risk_callers": [{"from": "src/caller.rs", "to": "src/legacy.rs", "impact": "risk"}]
        });
        let delivery_html = render_advisory_panel_html("proj-1", &delivery_v, false);
        crate::test_support::assert_no_inline_hex_colors(&delivery_html);
        assert!(delivery_html.contains("Missing file"));
        assert!(delivery_html.contains("src/caller.rs → src/legacy.rs"));
    }

    #[test]
    fn memory_panel_prefills_from_results_delivery_clash_brain_and_requires_verified() {
        let session = json!({
            "brain": {
                "task_id": "TASK-101",
                "task": {
                    "external_id": "TASK-101",
                    "progress": "Initial plan",
                    "next_steps": ["Fallback step"]
                },
                "capsule": {
                    "selected": [{"path": "docs/ARCH.md"}, {"path": "docs/SECURITY.md"}],
                    "procedural": [{"path": "docs/ARCH.md"}, {"path": "Laravel/AGENTS.md"}]
                }
            },
            "clash_result": {
                "summary": "Challenger confirmed token guard check.",
                "items": [
                    {
                        "kind": "decision",
                        "status": "accepted",
                        "claim": "Require non-empty JWT audience",
                        "evidence": "Verified in TokenGuard.php",
                        "file": "src/TokenGuard.php"
                    },
                    {
                        "kind": "objection",
                        "status": "disputed",
                        "claim": "Check refresh expiry edge case",
                        "file": "src/Deleted.php"
                    }
                ]
            }
        });
        let results_v = json!({
            "snapshot": {
                "files": [
                    {"path": "src/TokenGuard.php", "status": "M"},
                    {"path": "src/Deleted.php", "status": "D"}
                ]
            },
            "checks": [
                {"command": "cargo test -p infra-ui", "status": "passed"}
            ]
        });
        let delivery_v = json!({
            "files": [{"path": "src/TokenGuard.php", "status": "M"}]
        });
        let preview_v = json!({
            "target_check": {"status": "passed", "command": "cargo clippy"}
        });

        let draft = build_memory_draft_prefill(&session, &results_v, &delivery_v, &preview_v);
        assert!(draft["progress"]
            .as_str()
            .unwrap()
            .contains("Updated 2 file(s)"));
        assert!(draft["progress"]
            .as_str()
            .unwrap()
            .contains("cargo test -p infra-ui"));
        assert!(draft["next_steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s.as_str().unwrap().contains("Resolve clash finding")));
        let learnings = draft["learnings"].as_array().unwrap();
        assert_eq!(learnings.len(), 1);
        assert_eq!(learnings[0]["type"], "decision");
        assert_eq!(learnings[0]["sources"], json!(["src/TokenGuard.php"]));
        assert_eq!(
            draft["used_memory"],
            json!(["docs/ARCH.md", "docs/SECURITY.md", "Laravel/AGENTS.md"])
        );

        let rows_html = render_memory_learnings_html(learnings);
        crate::test_support::assert_no_inline_hex_colors(&rows_html);
        assert!(rows_html.contains("Require non-empty JWT audience"));

        assert!(build_memory_save_payload(
            draft["progress"].as_str().unwrap(),
            "Review remaining changes",
            learnings,
            "docs/ARCH.md",
            false
        )
        .is_err());

        let payload = build_memory_save_payload(
            draft["progress"].as_str().unwrap(),
            "Review remaining changes",
            learnings,
            "docs/ARCH.md\ndocs/SECURITY.md",
            true,
        )
        .expect("verified payload");
        assert_eq!(payload["verified"], true);
        assert_eq!(payload["next_steps"], json!(["Review remaining changes"]));
        assert_eq!(
            payload["used_memory"],
            json!(["docs/ARCH.md", "docs/SECURITY.md"])
        );

        let ok_html = format_memory_save_feedback_html(&json!({
            "ok": true,
            "saved": {"task": {"external_id": "TASK-101"}, "records": [{"id": "r1"}]},
            "skipped": [{"title": "Repeated note", "reason": "repeated"}]
        }));
        crate::test_support::assert_no_inline_hex_colors(&ok_html);
        assert!(ok_html.contains("Saved verified memory (1 record(s)) for TASK-101."));
        assert!(ok_html.contains("Repeated note"));

        let err_html = format_memory_save_feedback_html(&json!({
            "ok": false,
            "error": "Prepare the linked task before saving memory.",
            "skipped": []
        }));
        crate::test_support::assert_no_inline_hex_colors(&err_html);
        assert!(err_html.contains("Prepare the linked task before saving memory."));
    }

    #[test]
    fn memory_draft_prefill_excludes_non_canonical_brain_and_memory_paths() {
        let session = json!({
            "brain": {"bank": "memory-bank", "task_id": "TASK-202"},
            "clash_result": {
                "items": [{
                    "kind": "finding",
                    "status": "disputed",
                    "file": "project-brain/control/handoffs/h1.md",
                    "claim": "Internal handoff note"
                }]
            }
        });
        let results_v = json!({
            "snapshot": {
                "files": [
                    {"path": "project-brain/indexes/active.json", "status": "M"},
                    {"path": "project-brain/control/handoffs/8966be82.md", "status": "A"},
                    {"path": "services/billing/project-brain/control/handoffs/eae40ef8.md", "status": "A"},
                    {"path": "services/billing/project-brain/indexes/active.json", "status": "M"},
                    {"path": "memory-bank/chunks/MEM-20261118-deadbeef-tax.md", "status": "M"},
                    {"path": ".env.local", "status": "M"},
                    {"path": "services/orders/src/OrderService.php", "status": "M"}
                ]
            },
            "checks": []
        });
        let draft = build_memory_draft_prefill(&session, &results_v, &json!({}), &json!({}));
        assert!(draft["progress"]
            .as_str()
            .unwrap()
            .contains("Updated 1 file(s): services/orders/src/OrderService.php"));
        let learnings = draft["learnings"].as_array().unwrap();
        assert_eq!(learnings.len(), 1);
        assert_eq!(
            learnings[0]["sources"],
            json!(["services/orders/src/OrderService.php"])
        );
    }

    #[test]
    fn custom_model_selection_rejects_empty_and_flag_like_values() {
        assert!(model_selection_valid(false, &V::Null));
        assert!(!model_selection_valid(true, &V::Null));
        assert!(!model_selection_valid(true, &json!("-bad")));
        assert!(model_selection_valid(true, &json!("claude-sonnet-4-test")));
    }

    #[test]
    fn project_git_status_and_workspace_hint_format_worktree_readiness() {
        assert_eq!(
            format_project_git_status(
                &json!({"is_git": true, "branch": "master", "worktree_available": true})
            ),
            "Git branch: master · Worktrees ready"
        );
        assert_eq!(
            format_project_git_status(&json!({"is_git": false, "reason": "Not a git repo"})),
            "Not a git repo"
        );
        assert!(
            format_workspace_hint("worktree", &json!({"worktree_available": true}))
                .contains("worktree")
        );
    }

    #[test]
    fn history_filters_right_pane_bounds_and_failure_localizer_work() {
        let sessions = vec![
            json!({"id": "s-1", "title": "Fix AuthController", "project_id": "api", "status": "running", "waiting_permissions": 0}),
            json!({"id": "s-2", "title": "Migrate DB", "project_id": "core", "status": "awaiting_approval", "waiting_permissions": 2}),
        ];
        let mut counts = std::collections::BTreeMap::new();
        counts.insert("s-1".to_owned(), 1);
        let all = filter_history_sessions(&sessions, "", false, &counts);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0]["waiting_permissions"], 1);

        let filtered_query = filter_history_sessions(&sessions, "auth", false, &counts);
        assert_eq!(filtered_query.len(), 1);
        assert_eq!(filtered_query[0]["id"], "s-1");

        let waiting_only = filter_history_sessions(&sessions, "db", true, &counts);
        assert_eq!(waiting_only.len(), 1);
        assert_eq!(waiting_only[0]["id"], "s-2");

        assert_eq!(normalize_right_tab("diff"), "diff");
        assert_eq!(normalize_right_tab("checks"), "checks");
        assert_eq!(normalize_right_tab("unknown"), "graph");
        assert_eq!(clamp_workspace_split(10.0), 25.0);
        assert_eq!(clamp_workspace_split(95.0), 75.0);
        assert_eq!(clamp_workspace_split(48.0), 48.0);

        let called = std::rc::Rc::new(std::cell::Cell::new(false));
        let called_clone = called.clone();
        register_failure_localizer(move |req| {
            if req["project_id"] == "api" {
                called_clone.set(true);
            }
        });
        localize_failure_on_map(&json!({"project_id": "api", "failing_queries": ["phpunit"]}));
        assert!(called.get());
    }

    #[test]
    fn resolve_initial_project_activates_first_project_without_route_or_saved_pref() {
        let projects = vec![
            json!({"id": "harness-e2e-project", "name": "E2E"}),
            json!({"id": "second-project", "name": "Second"}),
        ];
        assert_eq!(
            super::resolve_initial_project("", "", &projects),
            "harness-e2e-project"
        );
        assert_eq!(
            super::resolve_initial_project("", "stale-id", &projects),
            "harness-e2e-project"
        );
        assert_eq!(
            super::resolve_initial_project("", "second-project", &projects),
            "second-project"
        );
        assert_eq!(
            super::resolve_initial_project("harness-e2e-project", "second-project", &projects),
            "harness-e2e-project"
        );
        assert_eq!(super::resolve_initial_project("", "", &[]), "");
    }

    #[test]
    fn sse_and_snapshot_events_merge_sorted_and_deduped_and_active_panes_route_cleanly() {
        let mut events = vec![
            json!({"id": 10, "kind": "status", "text": "start"}),
            json!({"id": 12, "kind": "text", "text": "late live sse"}),
        ];
        let mut seen = ["10".to_owned(), "12".to_owned()].into_iter().collect();
        let mut cursor = 12u64;

        // Snapshot backfill arrives with an earlier missed event #11 plus duplicate #10/#12.
        let (fresh, out_of_order) = merge_ordered_events(
            &mut events,
            &mut seen,
            &mut cursor,
            vec![
                json!({"id": "12", "kind": "text", "text": "dup"}),
                json!({"id": 11, "kind": "text", "text": "snapshot backfill"}),
                json!({"id": 10, "kind": "status", "text": "dup"}),
            ],
        );
        assert!(out_of_order);
        assert_eq!(fresh.len(), 1);
        assert_eq!(parsed_event_id(&fresh[0]), Some(11));
        assert_eq!(
            events
                .iter()
                .filter_map(parsed_event_id)
                .collect::<Vec<_>>(),
            vec![10, 11, 12]
        );
        assert_eq!(cursor, 12);

        // Strictly newer event appends cleanly without requiring replay.
        let (fresh2, out_of_order2) = merge_ordered_events(
            &mut events,
            &mut seen,
            &mut cursor,
            vec![json!({"id": 13, "kind": "result", "text": "done"})],
        );
        assert!(!out_of_order2);
        assert_eq!(fresh2.len(), 1);
        assert_eq!(cursor, 13);

        // Only active visible diff/checks pane should render.
        assert_eq!(
            active_diff_pane_target("changes", "graph").map(|t| t.1),
            Some("results-diff")
        );
        assert_eq!(
            active_diff_pane_target("sessions", "diff").map(|t| t.1),
            Some("workspace-diff-viewer")
        );
        assert_eq!(active_diff_pane_target("sessions", "graph"), None);
        assert_eq!(active_diff_pane_target("checks", "diff"), None);

        assert_eq!(
            active_checks_pane_target("checks", "graph").map(|t| t.0),
            Some("results-checks")
        );
        assert_eq!(
            active_checks_pane_target("sessions", "checks").map(|t| t.0),
            Some("workspace-checks-list")
        );
        assert_eq!(active_checks_pane_target("sessions", "diff"), None);

        let files_a = vec![json!({"status": "M", "path": "src/a.rs"})];
        let files_b = vec![json!({"status": "A", "path": "src/a.rs"})];
        assert_eq!(
            files_summary_hash("workspace-results", &files_a),
            files_summary_hash("workspace-results", &files_a)
        );
        assert_ne!(
            files_summary_hash("workspace-results", &files_a),
            files_summary_hash("workspace-results", &files_b)
        );
        assert_ne!(
            files_summary_hash("workspace-results", &files_a),
            files_summary_hash("results-diff", &files_a)
        );
    }

    #[test]
    fn responsive_viewport_plan_syncs_mobile_to_desktop_without_churn() {
        let mut last = Some(false);
        // Same breakpoint desktop->desktop avoids redundant inert/graph churn.
        assert_eq!(
            plan_responsive_viewport(&mut last, false, true),
            ViewportSyncPlan {
                close_sidebar: false,
                sync_inert: false,
                sync_graph: false,
            }
        );
        // Desktop->mobile syncs inert and embedded graph without closing drawer.
        assert_eq!(
            plan_responsive_viewport(&mut last, true, true),
            ViewportSyncPlan {
                close_sidebar: false,
                sync_inert: true,
                sync_graph: true,
            }
        );
        // Same breakpoint mobile->mobile avoids churn.
        assert_eq!(
            plan_responsive_viewport(&mut last, true, false),
            ViewportSyncPlan {
                close_sidebar: false,
                sync_inert: false,
                sync_graph: false,
            }
        );
        // Mobile->desktop closes sidebar, clears inert, and syncs embedded graph when in sessions.
        assert_eq!(
            plan_responsive_viewport(&mut last, false, true),
            ViewportSyncPlan {
                close_sidebar: true,
                sync_inert: true,
                sync_graph: true,
            }
        );
        assert_eq!(last, Some(false));
    }

    #[test]
    fn select_target_view_needed_respects_explicit_caller_view_and_boot() {
        assert_eq!(
            super::poll::select_target_view_needed("changes", Some("changes"), false),
            None
        );
        assert_eq!(
            super::poll::select_target_view_needed("systems", Some("changes"), false),
            Some("changes")
        );
        assert_eq!(
            super::poll::select_target_view_needed("systems", Some("sessions"), true),
            None
        );
        assert_eq!(
            super::poll::select_target_view_needed("systems", None, false),
            None
        );
    }
}
