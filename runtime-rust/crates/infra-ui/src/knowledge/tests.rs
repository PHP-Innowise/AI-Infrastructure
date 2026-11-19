use super::test_exports::{
    capsule_segments, context_turns_model, history_day_heights, memory_stage_metrics,
    render_capsule_bar_html, render_capsule_legend_html, render_detail_omnigraph_action_html,
    render_linked_omnigraph_html, render_list_entry_inner_html, render_promotion_queue_html,
    render_usage_legend_html, render_usage_turn_button_html,
};
use crate::test_support::{assert_no_inline_hex_colors, payloads};
use serde_json::json;

#[test]
fn promotion_queue_offers_review_apply_and_brain_by_id_only() {
    let id = |n: u8| format!("{n}{n}{n}{n}{n}{n}{n}{n}-0000-4000-8000-00000000000{n}");
    let source = |n: u8| format!("aaaaaaa{n}-bbbb-4ccc-8ddd-eeeeeeeeeeee");
    let items = vec![
        json!({"id": id(1), "source_ids": [source(1)], "reason": null, "status": "proposed", "mode": "human", "created_at": "2026-11-10T09:00:00+00:00", "reviewed_at": null, "applied_at": null, "memory_id": null, "title": "Rotate <session> keys", "path": "project-brain/dynamic/findings/rotate.md"}),
        json!({"id": id(2), "source_ids": [source(2), source(3)], "reason": "approved", "status": "reviewed", "mode": "human", "created_at": "2026-11-09T09:00:00+00:00", "reviewed_at": "2026-11-11T09:00:00+00:00"}),
        json!({"id": id(3), "source_ids": [source(4)], "status": "proposed", "mode": "automatic", "created_at": "2026-11-12T09:00:00+00:00"}),
        json!({"id": id(4), "source_ids": [source(5)], "reason": "approved-without-review", "status": "reviewed", "mode": "automatic", "created_at": "2026-11-12T10:00:00+00:00", "reviewed_at": "2026-11-12T10:00:01+00:00"}),
        json!({"id": id(5), "source_ids": [source(6)], "reason": "approved-without-review", "status": "applied", "mode": "automatic", "applied_at": "2026-11-15T10:00:00+00:00", "memory_id": "MEM-20261115-aabbccdd"}),
        json!({"id": id(6), "source_ids": [source(7)], "status": "applied", "mode": "automatic", "applied_at": "2026-09-01T10:00:00+00:00", "memory_id": "MEM-20260901-00112233"}),
        json!({"id": null, "source_ids": [], "status": "proposed", "mode": "human", "created_at": "2026-11-13T09:00:00+00:00"}),
        json!({"id": id(7), "source_ids": [source(8)], "status": "rejected", "mode": "human"}),
        json!({"id": id(8), "source_ids": [source(9)], "status": "applied", "mode": "human", "applied_at": "2026-11-16T10:00:00+00:00"}),
    ];
    let html = render_promotion_queue_html(
        &items,
        "2026-10-18T09:30:00.000Z",
        "Jane \"QA\"",
        false,
        "Approved promotion x.",
    );
    assert!(
        html.contains("3 waiting · 2 stalled · 1 auto-promoted"),
        "{html}"
    );
    // A person signs only a human proposal, oldest first; one without a
    // readable ID shows its buttons disabled.
    assert_eq!(
        payloads(&html, "memory-promotion-review"),
        vec![
            json!({"promotion_id": id(1), "reject": false}),
            json!({"promotion_id": id(1), "reject": true}),
            json!({"promotion_id": "", "reject": false}),
            json!({"promotion_id": "", "reject": true}),
        ]
    );
    assert_eq!(html.matches(" disabled>").count(), 2, "{html}");
    assert!(html.contains("Without a readable ID"), "{html}");
    // Apply publishes an approved proposal, a stalled automatic one too.
    assert_eq!(
        payloads(&html, "memory-promotion-apply"),
        vec![
            json!({"promotion_id": id(2)}),
            json!({"promotion_id": id(4)})
        ]
    );
    // An automatic proposal is never signed by a person.
    assert!(
        html.contains("An automatic proposal takes no reviewer"),
        "{html}"
    );
    // Every listed proposal opens its first source record in Project Brain.
    assert_eq!(
        payloads(&html, "memory-promotion-brain"),
        [
            source(2),
            source(1),
            String::new(),
            source(4),
            source(5),
            source(6)
        ]
        .map(|record| json!({"record_id": record}))
        .to_vec()
    );
    // IDs, outcome codes, dates and chunk IDs only: no Brain title or path.
    assert!(!html.contains("Rotate"), "{html}");
    assert!(!html.contains("project-brain/"), "{html}");
    assert!(
        html.contains("outcome <code>approved-without-review</code>"),
        "{html}"
    );
    assert!(html.contains("chunk <code>MEM-20261115-aabbccdd</code>"));
    assert!(html.contains("proposed 2026-11-10"), "{html}");
    // Outside the window, rejected and applied-after-review proposals stay off.
    for absent in ["MEM-20260901-00112233".to_owned(), id(7), id(8)] {
        assert!(!html.contains(&absent), "{absent}: {html}");
    }
    assert!(html.contains("value=\"Jane &quot;QA&quot;\""), "{html}");
    assert!(html.contains("role=\"status\">Approved promotion x.</p>"));
    assert_no_inline_hex_colors(&html);

    // While one action runs, every review and apply button waits.
    let busy = render_promotion_queue_html(&items, "", "", true, "");
    assert!(busy.contains("2 auto-promoted"), "{busy}");
    assert_eq!(busy.matches(" disabled>").count(), 6, "{busy}");
    assert!(!busy.contains("promotion-status"), "{busy}");

    let empty = render_promotion_queue_html(&[], "", "", false, "");
    assert!(empty.contains("Nothing waits for review"), "{empty}");
    assert!(!empty.contains("memory-promotion-reviewer"), "{empty}");
}

#[test]
fn brain_followup_changes_review_and_reviewed_query_only() {
    use super::test_exports::{followup_brain_from_fields, render_brain_followup_html};
    let automatic = json!({"task_id": "harness/fix-login-abc123", "review": false, "query": "fix login redirect"});
    // An automatic turn takes the next message as its query.
    assert_eq!(
        followup_brain_from_fields(&automatic, false, "anything"),
        Ok(None)
    );
    assert_eq!(
        followup_brain_from_fields(&automatic, true, " fix login redirect "),
        Ok(Some(json!({"review": true})))
    );
    assert_eq!(
        followup_brain_from_fields(&automatic, true, "session cookie expiry"),
        Ok(Some(
            json!({"review": true, "query": "session cookie expiry"})
        ))
    );
    assert!(followup_brain_from_fields(&automatic, true, "  ").is_err());
    let reviewed = json!({"task_id": "TASK-7", "review": true, "query": "retry policy"});
    assert_eq!(
        followup_brain_from_fields(&reviewed, true, "retry policy"),
        Ok(None)
    );
    assert_eq!(
        followup_brain_from_fields(&reviewed, true, "backoff caps"),
        Ok(Some(json!({"query": "backoff caps"})))
    );
    assert_eq!(
        followup_brain_from_fields(&reviewed, false, ""),
        Ok(Some(json!({"review": false})))
    );
    assert_eq!(
        followup_brain_from_fields(&json!(null), true, "x"),
        Ok(None)
    );

    let html = render_brain_followup_html("TASK-<7>", true, "retry & backoff");
    assert!(html.contains("<code>TASK-&lt;7&gt;</code>"), "{html}");
    assert!(
        html.contains("id=\"brain-followup-review\" type=\"checkbox\" checked>"),
        "{html}"
    );
    assert!(
        html.contains("id=\"brain-followup-query-field\">"),
        "{html}"
    );
    assert!(html.contains(">retry &amp; backoff</textarea>"), "{html}");
    assert!(html.contains("waits until you approve"), "{html}");
    let automatic_html = render_brain_followup_html("TASK-7", false, "");
    assert!(automatic_html.contains("type=\"checkbox\">Review context by hand"));
    assert!(automatic_html.contains("id=\"brain-followup-query-field\" hidden>"));
    assert!(automatic_html.contains("The next message is the retrieval query"));
    assert_no_inline_hex_colors(&html);
}

#[test]
fn promotion_queue_lists_the_newest_automatic_promotions() {
    let items = (0..25)
            .map(|day| {
                json!({"id": format!("00000000-0000-4000-8000-0000000000{day:02}"), "status": "applied", "mode": "automatic", "applied_at": format!("2026-11-{:02}T08:00:00+00:00", day + 1), "source_ids": []})
            })
            .collect::<Vec<_>>();
    let html = render_promotion_queue_html(&items, "", "", false, "");
    assert!(html.contains("Showing the 20 newest of 25."), "{html}");
    assert!(html.contains("applied 2026-11-25"), "{html}");
    assert!(!html.contains("applied 2026-11-05"), "{html}");
    // Nothing to sign, so no reviewer field.
    assert!(!html.contains("memory-promotion-reviewer"), "{html}");
}

#[test]
fn capsule_segments_and_legend_format_nonzero_parts_and_swatches() {
    let meter = json!({"characters": 1577, "limit": 8000, "kinds": {"brain": 752, "rules": 423, "bank": 0}, "items": {"brain": 1, "rules": 2, "bank": 0}, "repeats": 824, "dropped": {"episodic": 1}});
    let segments = capsule_segments(&meter);
    assert_eq!(
        segments
            .iter()
            .map(|(kind, _, characters, _)| (*kind, *characters))
            .collect::<Vec<_>>(),
        [("brain", 752), ("bank", 0), ("rules", 423)]
    );
    assert!((segments[0].3 - 9.4).abs() < 1e-9);
    let bar = render_capsule_bar_html(&meter);
    assert!(
        bar.contains("data-kind=\"brain\" style=\"width:9.40%\""),
        "{bar}"
    );
    assert!(bar.contains("data-kind=\"bank\" hidden"), "{bar}");
    let legend = render_capsule_legend_html(&meter);
    assert!(
        legend.contains("<i class=\"capsule-swatch\" data-kind=\"brain\""),
        "{legend}"
    );
    assert!(
        legend.contains("Rules &amp; docs · 423 characters · 2 items")
            || legend.contains("Rules & docs · 423 characters · 2 items"),
        "{legend}"
    );
    assert!(!legend.contains("Memory bank"), "{legend}");
    assert!(legend.contains("Dropped to fit: 1 episodic"), "{legend}");
    // No raw JSON reaches the legend.
    assert!(!legend.contains('{'), "{legend}");
}

#[test]
fn retrieval_trace_renders_chunks_cuts_and_feedback_toggles() {
    let row = json!({"manifest_id": "manifest-1", "brain": 1, "bank": 2, "rules": 0, "chunks": ["MEM-20261117-aabbccdd", "MEM-20261117-eeff0011"], "cuts": [["MEM-20261117-99998888", "budget"]]});
    let feedback = json!({"manifest-1": {"MEM-20261117-aabbccdd": {"vote": "stale"}}});
    let html = super::render_retrieval_trace_html(&row, &feedback, true);
    assert!(
        html.contains("<code>MEM-20261117-aabbccdd</code>"),
        "{html}"
    );
    assert!(
        html.contains("MEM-20261117-99998888 — cut: budget"),
        "{html}"
    );
    // The recorded verdict is pressed, and pressing it again clears it.
    assert!(
        html.contains("&quot;vote&quot;:&quot;clear&quot;}\" aria-pressed=\"true\">Stale</button>"),
        "{html}"
    );
    assert!(
        html.contains(
            "&quot;vote&quot;:&quot;helped&quot;}\" aria-pressed=\"false\">Helped</button>"
        ),
        "{html}"
    );
    assert!(html.contains("data-action=\"memory-feedback\""));
    // Without the server's support, or without a manifest, nothing is offered.
    assert!(!super::render_retrieval_trace_html(&row, &feedback, false).contains("<button"));
    let mut anonymous = row.clone();
    anonymous["manifest_id"] = json!(null);
    assert!(!super::render_retrieval_trace_html(&anonymous, &feedback, true).contains("<button"));
}

#[test]
fn open_from_omnigraph_finds_its_record_without_a_filter() {
    let entries = vec![
        json!({"id": "11111111-2222-4333-8444-555555555555", "external_id": "finding-1", "path": "dynamic/a.md"}),
        json!({"id": "MEM-20261117-aabbccdd", "path": "memory-bank/chunks/MEM-20261117-aabbccdd-ttl.md"}),
    ];
    assert_eq!(
        super::jump_entry(&entries, "finding-1").map(|v| &v["path"]),
        Some(&json!("dynamic/a.md"))
    );
    assert!(super::jump_entry(&entries, "MEM-20261117-aabbccdd").is_some());
    assert!(super::jump_entry(&entries, "").is_none());
    assert!(super::jump_entry(&entries, "missing").is_none());
}

#[test]
fn history_day_heights_stacks_with_chunk_under_retrievals() {
    let (rest, bank) = history_day_heights(&json!({"retrievals": 6, "with_chunk": 2}), 8);
    assert_eq!((rest, bank), (50.0, 25.0));
    // A count above the day's retrievals is held to them; an empty day is flat.
    assert_eq!(
        history_day_heights(&json!({"retrievals": 2, "with_chunk": 5}), 4),
        (0.0, 50.0)
    );
    assert_eq!(history_day_heights(&json!({}), 0), (0.0, 0.0));
}

#[test]
fn render_usage_turn_and_legend_emit_track_parts_and_swatches() {
    let turn = json!({"ordinal": 3, "window": 200000, "end": 50000, "parts": {"brain": 1000, "bank": 500, "rules": 0, "rest": 40000, "added": 8500, "free": 150000}, "compactions": []});
    let button = render_usage_turn_button_html(&turn);
    assert!(button.contains("class=\"context-turn-track\""), "{button}");
    assert!(
        button.contains("data-kind=\"brain\" style=\"height:0.50%\""),
        "{button}"
    );
    assert!(!button.contains("data-kind=\"rules\""), "{button}");
    assert!(
        button.ends_with("<span class=\"context-turn-label\">3</span>"),
        "{button}"
    );
    let compacted = json!({"ordinal": 4, "window": 200000, "end": 20000, "parts": {"brain": 1000, "rest": 1}, "compactions": [{"at": "x"}]});
    let only_added = render_usage_turn_button_html(&compacted);
    assert!(
        only_added.contains("data-kind=\"added\" style=\"height:10.00%\""),
        "{only_added}"
    );
    assert!(!only_added.contains("data-kind=\"brain\""), "{only_added}");
    let legend = render_usage_legend_html(&turn);
    assert!(
        legend.contains("<i class=\"context-swatch\" data-kind=\"bank\""),
        "{legend}"
    );
    assert!(
        legend.contains("<td class=\"context-number\">≈ 1000</td>"),
        "{legend}"
    );
    assert!(
        legend.contains("<td class=\"context-number\">—</td>"),
        "{legend}"
    );
}

#[test]
fn render_list_entry_inner_html_separates_title_path_and_meta() {
    let html = render_list_entry_inner_html(
        &json!({"title": "Retry <policy>", "path": "memory-bank/chunks/MEM-1.md", "status": "active", "bytes": 2048}),
    );
    assert!(
        html.starts_with("<span class=\"memory-entry-title\">Retry &lt;policy&gt;</span>"),
        "{html}"
    );
    assert!(
        html.contains("<span class=\"memory-entry-path\">memory-bank/chunks/MEM-1.md</span>"),
        "{html}"
    );
    assert!(
        html.contains("<span class=\"memory-entry-meta\">active · "),
        "{html}"
    );
}

#[test]
fn selected_stage_metrics_follow_semantic_order_and_sentence_case() {
    let stage = json!({
        "cut_total": 0,
        "distinct": 2,
        "dropped": 0,
        "measured": 1,
        "retrievals": 4,
        "reused": 1,
        "selections": 5,
        "with_chunk": 3,
        "since": "2026-11-01",
        "routes": {"context": 4}
    });
    let rows = memory_stage_metrics("selected", &stage).expect("object yields rows");
    let labels = rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>();
    assert_eq!(
        labels,
        vec![
            "Retrievals",
            "With chunk",
            "Selections",
            "Distinct",
            "Reused",
            "Cut total",
            "Dropped",
            "Measured",
        ]
    );
    assert_eq!(rows[0], ("Retrievals".into(), "4".into(), false));
    assert_eq!(rows[5], ("Cut total".into(), "0".into(), true));
}

#[test]
fn stage_metrics_format_booleans_and_preserve_unknown_keys() {
    let stage = json!({
        "total": 9,
        "open": 0,
        "truncated": true,
        "extra_flag": false
    });
    let rows = memory_stage_metrics("brain", &stage).expect("object yields rows");
    assert_eq!(
        rows,
        vec![
            ("Total".into(), "9".into(), false),
            ("Open".into(), "0".into(), true),
            ("Truncated".into(), "Yes".into(), false),
            ("Extra flag".into(), "No".into(), true),
        ]
    );
    assert!(memory_stage_metrics("brain", &json!(null)).is_none());
}

#[test]
fn context_turns_model_includes_omnigraph_ledger_characters() {
    let usage = json!({
        "launches": [
            {
                "id": "l1",
                "kind": "native",
                "started_at": "2026-11-17T09:00:00Z",
                "context": {
                    "fill": { "start": 5000, "end": 6200, "window": 100000 },
                    "ledger": {
                        "capsule": { "kinds": { "brain": 360, "bank": 720, "rules": 180 } },
                        "excerpts": [],
                        "omnigraph": 1800
                    }
                }
            },
            {
                "id": "l2",
                "kind": "native",
                "started_at": "2026-11-17T09:05:00Z",
                "context": {
                    "fill": { "start": 1000, "end": 1200, "window": 100000 },
                    "ledger": {
                        "capsule": { "kinds": { "brain": 360, "bank": 0, "rules": 0 } },
                        "excerpts": []
                    }
                }
            }
        ]
    });
    let turns = context_turns_model(&usage);
    assert_eq!(turns.len(), 2);
    let t1 = &turns[0];
    assert!((t1["parts"]["brain"].as_f64().unwrap() - 100.0).abs() < 1e-6);
    assert!((t1["parts"]["bank"].as_f64().unwrap() - 200.0).abs() < 1e-6);
    assert!((t1["parts"]["rules"].as_f64().unwrap() - 50.0).abs() < 1e-6);
    assert!((t1["parts"]["omnigraph"].as_f64().unwrap() - 500.0).abs() < 1e-6);
    assert!((t1["memory"].as_f64().unwrap() - 850.0).abs() < 1e-6);
    assert!((t1["parts"]["rest"].as_f64().unwrap() - 4150.0).abs() < 1e-6);

    let t2 = &turns[1];
    assert!((t2["parts"]["omnigraph"].as_f64().unwrap() - 0.0).abs() < 1e-6);
    assert!((t2["memory"].as_f64().unwrap() - 100.0).abs() < 1e-6);
}

#[test]
fn linked_omnigraph_summary_renders_review_metrics_and_unverified_warning() {
    let disabled = json!({ "status": "awaiting_context", "omnigraph": { "enabled": false } });
    assert!(!render_linked_omnigraph_html(&disabled).0);

    let session = json!({
        "status": "awaiting_context",
        "omnigraph": {
            "enabled": true,
            "branch": "session/auth-1",
            "capsule_text": "## Omnigraph Shared Context\n- [unverified reference] Avoid raw tokens",
            "injected_atoms": [
                { "id": "a1", "title": "Brain rule", "verified": true },
                { "id": "a2", "title": "Draft note", "verified": false }
            ]
        }
    });
    let (show, html) = render_linked_omnigraph_html(&session);
    assert!(show);
    assert!(html.contains("session/auth-1"));
    assert!(html.contains("2 atom(s)"));
    assert!(html.contains("69 / 6,000 chars"));
    assert!(html.contains("1 verified"));
    assert!(html.contains("1 unverified reference"));
    assert!(html.contains("Included before run"));
    assert!(html.contains("Omnigraph capsule preview"));
}

#[test]
fn knowledge_detail_action_renders_open_in_context_graph_button() {
    let brain_rec = json!({ "id": "rec-uuid-123", "title": "Auth decision" });
    let brain_html = render_detail_omnigraph_action_html("brain", "proj-1", &brain_rec);
    assert!(brain_html.contains("data-action=\"knowledge-open-omnigraph\""));
    assert!(brain_html.contains("data-project-id=\"proj-1\""));
    assert!(brain_html.contains("data-atom-id=\"brain-rec-uuid-123\""));
    assert!(brain_html.contains("Open in Context Graph"));

    let mem_chunk = json!({ "id": "MEM-20261117-abcd1234-auth", "title": "Token rules" });
    let mem_html = render_detail_omnigraph_action_html("memory", "proj-1", &mem_chunk);
    assert!(mem_html.contains("data-action=\"knowledge-open-omnigraph\""));
    assert!(mem_html.contains("data-project-id=\"proj-1\""));
    assert!(mem_html.contains("data-atom-id=\"mem-MEM-20261117-abcd1234-auth\""));

    let no_id = json!({ "title": "INDEX.md" });
    assert!(render_detail_omnigraph_action_html("memory", "proj-1", &no_id).is_empty());
}

#[test]
fn resolve_initial_selection_prefers_jump_then_saved_then_default_active_record() {
    let entries = vec![
        json!({"id": "arch", "type": "task", "kind": "doc", "path": "archive/old.md"}),
        json!({"id": "t1", "type": "task", "kind": "chunk", "path": "dynamic/tasks/t1.md"}),
        json!({"id": "t2", "type": "finding", "kind": "chunk", "path": "dynamic/findings/f2.md"}),
    ];
    assert_eq!(
        super::resolve_initial_selection("brain", &entries, "dynamic/findings/f2.md", Some("arch")),
        Some("archive/old.md".to_owned())
    );
    assert_eq!(
        super::resolve_initial_selection("brain", &entries, "dynamic/findings/f2.md", None),
        Some("dynamic/findings/f2.md".to_owned())
    );
    assert_eq!(
        super::resolve_initial_selection("brain", &entries, "", None),
        Some("dynamic/tasks/t1.md".to_owned())
    );
    assert_eq!(
        super::resolve_initial_selection("memory", &entries, "", None),
        Some("dynamic/tasks/t1.md".to_owned())
    );
    assert_eq!(
        super::resolve_initial_selection("brain", &[], "", None),
        None
    );
}

#[test]
fn context_turn_roving_states_and_detail_items_decouple_selection_and_preserve_tab_stop() {
    let shown = vec![
        json!({"id": "t1", "ordinal": 1}),
        json!({"id": "t2", "ordinal": 2}),
        json!({"id": "t3", "ordinal": 3}),
    ];
    // Selected turn on current page gets aria-checked=true and tabindex=0
    assert_eq!(
        super::context_turn_roving_states(&shown, &json!("t2")),
        vec![
            ("t1".into(), false, "-1"),
            ("t2".into(), true, "0"),
            ("t3".into(), false, "-1"),
        ]
    );
    // When selected turn is on another page, last shown turn remains tab-reachable
    assert_eq!(
        super::context_turn_roving_states(&shown, &json!("t99")),
        vec![
            ("t1".into(), false, "-1"),
            ("t2".into(), false, "-1"),
            ("t3".into(), false, "0"),
        ]
    );

    let turn = json!({
        "id": "t2",
        "ledger": {"chars": 100},
        "hooks": null,
        "cli_files": ["a.rs"],
        "compactions": []
    });
    let details = super::build_usage_detail_items(&turn, &shown);
    let keys: Vec<String> = details
        .iter()
        .map(|d| d["key"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        keys,
        vec!["ledger", "cli_files", "compactions", "turn-table"]
    );
}

#[test]
fn memory_chunk_and_trace_items_key_stably_and_table_sort_reports_aria_sort() {
    let chunks = vec![
        json!({"id": "c1", "status": "active", "review_after": "2026-10-01", "sources_changed": false}),
        json!({"id": "c2", "status": "archived", "review_after": "2026-12-01", "sources_changed": true}),
        json!({"id": "", "path": "memory-bank/c3.md", "status": "active", "review_after": "2026-12-01", "sources_changed": true}),
    ];
    let past = super::build_memory_chunk_items(&chunks, "2026-11-01", "past");
    assert_eq!(past.len(), 1);
    assert_eq!(past[0]["key"], "chunk:c1");

    let changed = super::build_memory_chunk_items(&chunks, "2026-11-01", "changed");
    assert_eq!(changed.len(), 2);
    assert_eq!(changed[0]["key"], "chunk:c2");
    assert_eq!(changed[1]["key"], "chunk:memory-bank/c3.md");

    let traces = vec![
        json!({"manifest_id": "m1", "at": "2026-11-17T10:00:00Z"}),
        json!({"manifest_id": "m1", "at": "2026-11-17T10:01:00Z"}),
    ];
    let feedback = json!({"m1": {"c1": {"vote": "helped"}}});
    let trace_items = super::build_memory_trace_items(&traces, &feedback, true);
    assert_eq!(trace_items[0]["key"], "trace:m1");
    assert_eq!(trace_items[1]["key"], "trace:m1#1");

    assert_eq!(
        super::test_exports::memory_table_sort_dir(&json!({}), "id", "id"),
        Some("ascending")
    );
    assert_eq!(
        super::test_exports::memory_table_sort_dir(
            &json!({"field": "bytes", "desc": true}),
            "id",
            "bytes"
        ),
        Some("descending")
    );
    assert_eq!(
        super::test_exports::memory_table_sort_dir(
            &json!({"field": "bytes", "desc": true}),
            "id",
            "id"
        ),
        None
    );
}

#[test]
fn responsive_knowledge_density_scales_by_breakpoint() {
    assert_eq!(super::usage_page_size(true), 6);
    assert_eq!(super::usage_page_size(false), 12);
    assert_eq!(super::horizon_bin_days(true), 14);
    assert_eq!(super::horizon_bin_days(false), 7);
    assert_eq!(super::horizon_ticks(true), &[0, 180, 365]);
    assert_eq!(super::horizon_ticks(false), &[-30, 0, 90, 180, 270, 365]);

    let items: Vec<_> = (0..100).map(|i| json!(i)).collect();
    let mobile_slice = super::analytics_slice(&items, true);
    assert_eq!(mobile_slice.len(), 80);
    assert_eq!(mobile_slice[0], json!(20));
    assert_eq!(super::analytics_slice(&items, false).len(), 100);
}

#[test]
fn knowledge_keyed_list_items_and_css_hidden_filter_avoid_selection_rebuilds() {
    use super::test_exports::*;
    use crate::model as m;

    let entries = vec![
        json!({
            "path": "project-brain/dynamic/tasks/TASK-1.md",
            "title": "Fix polling timers",
            "type": "task",
            "status": "active",
            "bytes": 1024
        }),
        json!({
            "path": "project-brain/archive/old.md",
            "title": "Legacy archive note",
            "category": "archive/old",
            "status": "archived",
            "bytes": 256
        }),
    ];
    let items = build_knowledge_list_items("brain", &entries);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["path"], "project-brain/dynamic/tasks/TASK-1.md");
    // Signatures omit selected_path and filter state so filtering or selection never rebuilds rows.
    assert!(items[0].get("selected_path").is_none());
    assert!(items[0].get("hidden").is_none());

    let visible_task = knowledge_visible_paths(&entries, "polling", "task");
    assert_eq!(visible_task.len(), 1);
    assert!(visible_task.contains("project-brain/dynamic/tasks/TASK-1.md"));

    let visible_archive = knowledge_visible_paths(&entries, "", "archive");
    assert_eq!(visible_archive.len(), 1);
    assert!(visible_archive.contains("project-brain/archive/old.md"));

    m::set(
        &key("brain"),
        json!({"project_id": "proj-1", "entries": entries}),
    );
    super::on_global_project_change("proj-2");
    assert_eq!(m::string(&m::data("knowledge-global-project")), "proj-2");
    assert!(m::data(&key("brain")).is_null());
    m::take("knowledge-global-project");
}
