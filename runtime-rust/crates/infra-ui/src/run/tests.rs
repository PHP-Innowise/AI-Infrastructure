#![allow(unused_imports)]
use super::*;

#[cfg(test)]
mod mode_tests {
    use super::*;

    fn launch(id: &str, kinds: &[&str]) -> V {
        let mut run = initial();
        run["launch"] = json!(id);
        run["kinds"] = json!(kinds);
        run
    }
    fn session(status: &str) -> V {
        json!({"id":"s","status":status,"workflow":"native"})
    }
    const NONE: V = V::Null;
    /// The mode when the newest launch replaced none.
    fn mode(session: &V, run: &V, after: &V, idle: &V) -> Mode {
        super::mode(session, run, &NONE, after, idle)
    }

    #[test]
    fn the_view_stays_closed_while_nothing_runs() {
        let run = launch("a", &["status", "tool", "text"]);
        assert_eq!(mode(&NONE, &NONE, &NONE, &NONE), Mode::Closed);
        assert_eq!(mode(&NONE, &run, &NONE, &NONE), Mode::Closed);
        for status in [
            "completed",
            "failed",
            "cancelled",
            "interrupted",
            "awaiting_context",
            "awaiting_approval",
        ] {
            assert_eq!(
                mode(&session(status), &run, &NONE, &json!("a")),
                Mode::Closed,
                "{status}"
            );
        }
    }

    #[test]
    fn a_running_native_launch_opens_the_now_line() {
        let running = session("running");
        for kind in [
            "session", "text", "tool", "plan", "usage", "result", "error",
        ] {
            assert_eq!(
                mode(&running, &launch("a", &["status", kind]), &NONE, &NONE),
                Mode::Now,
                "{kind}"
            );
        }
        // A reload during the run replays the record with the session still running.
        assert_eq!(
            mode(&running, &launch("b", &["tool"]), &NONE, &json!("a")),
            Mode::Now
        );
    }

    #[test]
    fn the_strip_waits_until_the_provider_writes() {
        let running = session("running");
        assert_eq!(mode(&running, &NONE, &NONE, &NONE), Mode::Waiting);
        assert_eq!(
            mode(&running, &launch("", &["user"]), &NONE, &NONE),
            Mode::Waiting
        );
        assert_eq!(
            mode(&running, &launch("a", &["status", "memory"]), &NONE, &NONE),
            Mode::Waiting
        );
        assert_eq!(
            mode(&session("queued"), &launch("a", &["tool"]), &NONE, &NONE),
            Mode::Waiting
        );
    }

    #[test]
    fn an_earlier_launch_never_stands_in_for_the_one_that_starts() {
        let running = session("running");
        // The person's next message arrived after this launch began.
        let answered = launch("a", &["tool", "text", "user"]);
        assert_eq!(mode(&running, &answered, &NONE, &NONE), Mode::Waiting);
        // The page saw the session at rest with this launch: a check or a retry runs now.
        let rested = launch("a", &["tool", "text"]);
        assert_eq!(mode(&running, &rested, &NONE, &json!("a")), Mode::Waiting);
        // The launch closed itself while the session still settles.
        let mut closed = launch("a", &["tool", "status"]);
        closed["finished"] = json!(true);
        assert_eq!(mode(&running, &closed, &NONE, &NONE), Mode::Waiting);
        // Its successor's first provider event takes the strip.
        assert_eq!(
            mode(&running, &launch("b", &["tool"]), &NONE, &json!("a")),
            Mode::Now
        );
        // A record without launch ids holds the messages too; they end nothing.
        assert!(!over(&launch("", &["user", "tool"]), &NONE));
        assert!(over(&launch("", &["user", "tool"]), &json!("")));
    }

    #[test]
    fn sessions_with_their_own_progress_only_wait() {
        let run = launch("a", &["status", "tool"]);
        let mut sessions = vec![json!({"id":"s","status":"running","workflow":"fleet-review"})];
        for key in [
            "fleet",
            "clash",
            "creator",
            "system_run",
            "system_discovery",
        ] {
            let mut session = session("running");
            session[key] = json!({});
            sessions.push(session);
        }
        for mut session in sessions {
            assert_eq!(mode(&session, &run, &NONE, &NONE), Mode::Waiting);
            session["status"] = json!("completed");
            assert_eq!(mode(&session, &run, &json!("a"), &NONE), Mode::Closed);
            assert!(openers(&session, &run).is_empty());
        }
        for kind in [
            "fleet_stage",
            "fleet_reviewer",
            "clash_turn",
            "agent",
            "agent_activity",
        ] {
            let mut run = launch("a", &["tool", kind]);
            run["files"] = json!({"a.php":{"path":"a.php"}});
            assert_eq!(mode(&session("running"), &run, &NONE, &NONE), Mode::Waiting);
            assert_eq!(
                mode(&session("completed"), &run, &json!("a"), &NONE),
                Mode::Closed
            );
            assert!(openers(&session("completed"), &run).is_empty());
        }
    }

    #[test]
    fn a_finished_turn_opens_for_its_own_launch_and_closes_again() {
        let done = session("completed");
        let run = launch("a", &["tool", "text"]);
        assert_eq!(mode(&done, &run, &json!("a"), &json!("a")), Mode::After);
        // Close forgets the launch; another launch's request opens nothing.
        assert_eq!(mode(&done, &run, &NONE, &json!("a")), Mode::Closed);
        assert_eq!(mode(&done, &run, &json!("b"), &json!("a")), Mode::Closed);
        assert_eq!(mode(&done, &NONE, &json!("a"), &NONE), Mode::Closed);
        // No session is selected: a launch left in the model opens nothing.
        assert_eq!(mode(&NONE, &run, &json!("a"), &NONE), Mode::Closed);
        // A record without launch ids is one launch with an empty id.
        assert_eq!(
            mode(
                &done,
                &launch("", &["user", "tool"]),
                &json!(""),
                &json!("")
            ),
            Mode::After
        );
        // The next turn takes the strip back.
        assert_eq!(
            mode(&session("queued"), &run, &json!("a"), &json!("a")),
            Mode::Waiting
        );
        assert_eq!(
            mode(&session("running"), &run, &json!("a"), &json!("a")),
            Mode::Waiting
        );
    }

    #[test]
    fn a_finished_turn_offers_only_the_tabs_it_filled() {
        let done = session("completed");
        let mut run = launch("a", &["tool", "text"]);
        assert!(openers(&done, &run).is_empty());
        run["files"] = json!({"a.php":{"path":"a.php","reads":1,"edits":0}});
        assert_eq!(openers(&done, &run), ["files"]);
        run["plan"] = json!([{"content":"Write the test","status":"pending"}]);
        run["commands"] = json!([{"call":"c1","command":"composer test"}]);
        assert_eq!(openers(&done, &run), ["files", "plan", "commands"]);
        run["files"] = json!({});
        assert_eq!(openers(&done, &run), ["plan", "commands"]);
        // While a turn runs, the strip is its own; without a session there is nothing to open.
        assert!(openers(&session("running"), &run).is_empty());
        assert!(openers(&session("queued"), &run).is_empty());
        assert!(openers(&NONE, &run).is_empty());
        assert!(openers(&done, &NONE).is_empty());
    }

    #[test]
    fn the_panel_shows_files_when_the_chosen_tab_is_empty() {
        let mut run = launch("a", &["tool"]);
        assert_eq!(shown_tab(&run, "files"), "files");
        assert_eq!(shown_tab(&run, "plan"), "files");
        assert_eq!(shown_tab(&run, "commands"), "files");
        assert_eq!(shown_tab(&run, "elsewhere"), "files");
        run["plan"] = json!([{"content":"Write the test","status":"pending"}]);
        assert_eq!(shown_tab(&run, "plan"), "plan");
        assert_eq!(shown_tab(&run, "commands"), "files");
        // A plan event without a list leaves the tab out, as the pane would be empty.
        run["plan"] = json!({"note":"no items"});
        assert_eq!(shown_tab(&run, "plan"), "files");
    }

    #[test]
    fn openers_and_shown_tab_include_memory_and_omnigraph_when_present() {
        let done = session("completed");
        let mut run = launch("a", &["tool", "text"]);
        run["files"] = json!({"a.php":{"path":"a.php","reads":1,"edits":0}});
        assert_eq!(openers(&done, &run), ["files"]);
        assert_eq!(shown_tab(&run, "memory"), "files");
        run["memory"] = json!({"kind":"memory","ok":true,"text":"Saved 2 learnings.","delivered_sources":3,"opened_delivered_sources":1});
        run["omnigraph"] =
            json!({"kind":"omnigraph","phase":"inbox_notify","ok":false,"text":"Not told <now>."});
        assert_eq!(openers(&done, &run), ["files", "memory", "omnigraph"]);
        assert_eq!(shown_tab(&run, "memory"), "memory");
        assert_eq!(shown_tab(&run, "omnigraph"), "omnigraph");
        let memory = render_run_memory_pane_html(&run["memory"]);
        assert!(memory.contains("data-ok=\"true\">Saved</p>"), "{memory}");
        assert!(memory.contains("<li>3 sources delivered</li>"), "{memory}");
        let omnigraph = render_run_omnigraph_pane_html(&run["omnigraph"]);
        assert!(
            omnigraph.contains("data-ok=\"false\">inbox notify</p>"),
            "{omnigraph}"
        );
        assert!(omnigraph.contains("Not told &lt;now&gt;."), "{omnigraph}");
        assert_eq!(render_run_memory_pane_html(&json!(null)), "");
    }

    #[test]
    fn a_turn_s_skillcache_events_get_their_own_tab() {
        let done = session("completed");
        let mut run = launch("a", &["tool", "text"]);
        // A cold turn of a session that never opted in has no such tab.
        assert_eq!(openers(&done, &run), Vec::<&str>::new());
        assert_eq!(shown_tab(&run, "skillcache"), "files");
        for event in [
            json!({"id": 7, "kind": "skillcache", "phase": "hit", "capsule_id": "c1",
                   "text": "SkillCache: capsule `pdf-rows` runs this request instead of the skill being read again."}),
            json!({"id": 8, "kind": "skillcache", "phase": "program_failed", "step_index": 1,
                   "program": "extract", "reason": "timeout",
                   "text": "SkillCache: program `extract` failed (timeout); step 2 is the agent's now, the rest of the plan stays compiled."}),
        ] {
            run = advance(run, &event).0;
        }
        assert_eq!(run["skillcache"].as_array().unwrap().len(), 2);
        assert_eq!(openers(&done, &run), ["skillcache"]);
        assert_eq!(shown_tab(&run, "skillcache"), "skillcache");
        // A new launch starts without the previous one's events.
        assert_eq!(initial()["skillcache"], json!([]));
    }
}

#[cfg(test)]
mod check_tests {
    use super::*;

    const NONE: V = V::Null;
    /// The model after these events: the newest launch and the one it replaced.
    fn play(events: &[V]) -> (V, V) {
        let (mut run, mut before) = (V::Null, V::Null);
        for event in events {
            let (next, replaced) = advance(run, event);
            run = next;
            if let Some(replaced) = replaced {
                before = replaced;
            }
        }
        (run, before)
    }
    /// A turn that opened one file, under the launch id.
    fn a_turn(launch: &str) -> Vec<V> {
        vec![
            json!({"id":1,"kind":"user","text":"Add the index."}),
            json!({"id":2,"kind":"status","launch_id":launch,"text":"Starting Claude Code."}),
            json!({"id":3,"kind":"text","launch_id":launch,"text":"Reading the model."}),
            json!({"id":4,"kind":"tool","launch_id":launch,"tool":"Read","call":"r1","state":"started","path":"app/Order.php"}),
            json!({"id":5,"kind":"tool","launch_id":launch,"tool":"Read","call":"r1","state":"completed","ok":true,"path":"app/Order.php"}),
        ]
    }
    /// What the server writes for a check that ran: its result, then its end.
    fn a_check(launch: &str, result: &str) -> Vec<V> {
        vec![
            json!({"id":10,"kind":"status","launch_id":launch,"text":format!("Check {result}: php artisan test. This is separate from the agent result.")}),
            json!({"id":11,"kind":"status","launch_id":launch,"text":"Check finished.","status":"completed"}),
        ]
    }
    fn done() -> V {
        json!({"id":"s","status":"completed","workflow":"native"})
    }

    #[test]
    fn the_model_follows_the_newest_launch() {
        let (run, before) = play(&a_turn("a"));
        assert_eq!(run["launch"], "a");
        assert_eq!(run["steps"], 1);
        assert_eq!(run["messages"], 1);
        assert!(has(&run, "files"));
        assert!(!is_check(&run));
        // The first launch replaced only the record of the message that asked for it.
        assert_eq!(before["launch"], "");
        assert!(!has(&before, "files"));
    }

    #[test]
    fn a_harness_check_leaves_the_turn_it_follows() {
        let mut events = a_turn("a");
        events.extend(a_check("c", "passed"));
        let (run, before) = play(&events);
        assert_eq!(run["launch"], "c");
        assert!(is_check(&run));
        let last = turn(&run, &before);
        assert_eq!(last["launch"], "a");
        assert_eq!(openers(&done(), last), ["files"]);
        // The line opens the turn's details, not the check's, and Close puts them away.
        assert_eq!(
            mode(&done(), &run, &before, &json!("a"), &json!("c")),
            Mode::After
        );
        assert_eq!(
            mode(&done(), &run, &before, &json!("c"), &json!("c")),
            Mode::Closed
        );
        assert_eq!(
            mode(&done(), &run, &before, &NONE, &json!("c")),
            Mode::Closed
        );
        // The same holds for a check that failed.
        let mut events = a_turn("a");
        events.extend(a_check("c", "failed"));
        let (run, before) = play(&events);
        assert_eq!(turn(&run, &before)["launch"], "a");
    }

    #[test]
    fn a_check_that_could_not_run_is_still_a_check() {
        let mut events = a_turn("a");
        events.push(
            json!({"id":10,"kind":"error","launch_id":"c","text":"The check worker is busy."}),
        );
        events.push(json!({"id":11,"kind":"status","launch_id":"c","text":"Check finished.","status":"completed"}));
        let (run, before) = play(&events);
        assert!(is_check(&run));
        assert_eq!(turn(&run, &before)["launch"], "a");
    }

    #[test]
    fn checks_in_a_row_keep_the_turn_before_them() {
        let mut events = a_turn("a");
        events.extend(a_check("c", "failed"));
        events.extend(a_check("d", "passed"));
        let (run, before) = play(&events);
        assert_eq!(run["launch"], "d");
        assert_eq!(before["launch"], "a");
        assert_eq!(openers(&done(), turn(&run, &before)), ["files"]);
    }

    #[test]
    fn a_running_check_only_waits() {
        let running = json!({"id":"s","status":"running","workflow":"native"});
        // Before the check writes, the model still holds the turn the page saw at rest.
        let (run, before) = play(&a_turn("a"));
        assert_eq!(
            mode(&running, &run, &before, &NONE, &json!("a")),
            Mode::Waiting
        );
        // Its result arrives while the session still settles.
        let mut events = a_turn("a");
        events.push(a_check("c", "passed").remove(0));
        let (run, before) = play(&events);
        assert_eq!(
            mode(&running, &run, &before, &NONE, &json!("a")),
            Mode::Waiting
        );
        // An error on its record does not make a check what the Now line describes.
        events.push(
            json!({"id":12,"kind":"error","launch_id":"c","text":"The command could not start."}),
        );
        let (run, before) = play(&events);
        assert!(cli_started(&run));
        assert_eq!(
            mode(&running, &run, &before, &NONE, &json!("a")),
            Mode::Waiting
        );
        // Nothing opens under a running check, and there is no line to open it from.
        assert_eq!(
            mode(&running, &run, &before, &json!("a"), &json!("a")),
            Mode::Waiting
        );
        assert!(openers(&running, turn(&run, &before)).is_empty());
    }

    #[test]
    fn the_turn_after_a_check_takes_over() {
        let running = json!({"id":"s","status":"running","workflow":"native"});
        let mut events = a_turn("a");
        events.extend(a_check("c", "passed"));
        events.push(json!({"id":20,"kind":"user","text":"Now the migration."}));
        // The message is in, the new launch has written nothing: the strip waits.
        let (run, before) = play(&events);
        assert_eq!(
            mode(&running, &run, &before, &NONE, &json!("c")),
            Mode::Waiting
        );
        events.push(json!({"id":21,"kind":"text","launch_id":"b","text":"Writing it."}));
        let (run, before) = play(&events);
        assert_eq!(run["launch"], "b");
        assert_eq!(mode(&running, &run, &before, &NONE, &json!("c")), Mode::Now);
        // The check is not kept: behind the new turn stands the turn before the check.
        assert_eq!(before["launch"], "a");
        // At rest the new turn is the last one, and it opened no file.
        assert_eq!(turn(&run, &before)["launch"], "b");
        assert!(openers(&done(), turn(&run, &before)).is_empty());
    }

    #[test]
    fn a_turn_without_details_does_not_borrow_the_earlier_one() {
        let mut events = a_turn("a");
        events.push(json!({"id":20,"kind":"user","text":"What did you change?"}));
        events.push(json!({"id":21,"kind":"text","launch_id":"b","text":"Only the model."}));
        let (run, before) = play(&events);
        assert_eq!(before["launch"], "a");
        assert_eq!(turn(&run, &before)["launch"], "b");
        assert!(openers(&done(), turn(&run, &before)).is_empty());
        assert_eq!(
            mode(&done(), &run, &before, &json!("a"), &json!("b")),
            Mode::Closed
        );
    }

    #[test]
    fn only_a_launch_the_provider_never_wrote_to_is_a_check() {
        for kind in ["session", "text", "tool", "plan", "usage", "result"] {
            let (run, _) = play(&[
                json!({"id":1,"kind":kind,"launch_id":"a","text":"x"}),
                json!({"id":2,"kind":"status","launch_id":"a","text":"Check passed: make test. Fine."}),
            ]);
            assert!(m::yes(&run["check"]));
            assert!(!is_check(&run), "{kind}");
        }
        // A launch that wrote no such line is no check, however little it wrote.
        let (run, _) = play(&[
            json!({"id":1,"kind":"status","launch_id":"a","text":"Context prepared. Review it before starting the provider."}),
            json!({"id":2,"kind":"error","launch_id":"a","text":"Provider is not installed."}),
        ]);
        assert!(!is_check(&run));
    }

    #[test]
    fn the_check_lines_are_told_from_other_status_text() {
        for text in [
            "Check passed: php artisan test. This is separate from the agent result.",
            "Check failed: vendor/bin/phpstan analyse. This is separate from the agent result.",
            "Check cancelled: make test. This is separate from the agent result.",
            "Check finished.",
        ] {
            assert!(check_status(text), "{text}");
        }
        for text in [
            "",
            "Check",
            "Check finished",
            "Checking composer.json",
            "Checked 3 files: ok",
            "Check : nothing",
            "Check the plan: step one",
            "Cancellation requested.",
        ] {
            assert!(!check_status(text), "{text}");
        }
    }
}

#[cfg(test)]
mod files_tests {
    use super::*;

    const L: &str = "a";
    /// The launch after these events.
    fn play(events: &[V]) -> V {
        let mut run = V::Null;
        for event in events {
            run = advance(run, event).0;
        }
        run
    }
    fn tool(id: u64, name: &str, call: &str, state: &str, more: V) -> V {
        let mut event = json!({"id":id,"kind":"tool","launch_id":L,"tool":name,"call":call,"state":state,"text":format!("{name}: {state}")});
        for (key, value) in more.as_object().into_iter().flatten() {
            event[key] = value.clone();
        }
        event
    }
    /// A call that starts and reports its result, as two events.
    fn done(id: u64, name: &str, call: &str, more: V, ok: bool) -> Vec<V> {
        let mut end = more.clone();
        end["ok"] = json!(ok);
        vec![
            tool(id, name, call, "started", more),
            tool(id + 1, name, call, "completed", end),
        ]
    }
    fn file<'a>(run: &'a V, path: &str) -> &'a V {
        &run["files"][path]
    }

    #[test]
    fn a_call_counts_once_for_the_file_it_names() {
        let mut events = done(1, "Read", "r1", json!({"path":"app/Order.php"}), true);
        events.extend(done(3, "Edit", "e1", json!({"path":"app/Order.php"}), true));
        events.extend(done(5, "Read", "r2", json!({"path":"app/Order.php"}), true));
        let run = play(&events);
        assert_eq!(file(&run, "app/Order.php")["reads"], 2);
        assert_eq!(file(&run, "app/Order.php")["edits"], 1);
        assert_eq!(run["steps"], 3);
        // A result that arrives twice changes nothing.
        events.push(tool(
            9,
            "Edit",
            "e1",
            "completed",
            json!({"path":"app/Order.php","ok":true}),
        ));
        assert_eq!(file(&play(&events), "app/Order.php")["edits"], 1);
    }

    #[test]
    fn an_edit_counts_when_the_tool_reports_it_done() {
        let started = [tool(1, "Edit", "e1", "started", json!({"path":"a.php"}))];
        let run = play(&started);
        assert_eq!(file(&run, "a.php")["edits"], 0);
        assert!(!m::yes(&file(&run, "a.php")["failed"]));
        let run = play(&done(1, "Edit", "e1", json!({"path":"a.php"}), false));
        assert_eq!(file(&run, "a.php")["edits"], 0);
        assert!(m::yes(&file(&run, "a.php")["failed"]));
        // A step that was never run reported nothing about its file.
        let run = play(&[
            tool(1, "Edit", "e1", "started", json!({"path":"a.php"})),
            tool(
                2,
                "Edit",
                "e1",
                "completed",
                json!({"path":"a.php","ok":false,"outcome":"not_run"}),
            ),
        ]);
        assert_eq!(file(&run, "a.php")["edits"], 0);
        assert!(!m::yes(&file(&run, "a.php")["failed"]));
        // A failed read marks the file too.
        let run = play(&done(1, "Read", "r1", json!({"path":"a.php"}), false));
        assert_eq!(file(&run, "a.php")["reads"], 1);
        assert!(m::yes(&file(&run, "a.php")["failed"]));
    }

    #[test]
    fn a_result_without_a_start_is_the_whole_call() {
        let run = play(&[tool(
            1,
            "Edit",
            "e1",
            "completed",
            json!({"path":"a.php","ok":true}),
        )]);
        assert_eq!(file(&run, "a.php")["edits"], 1);
        let run = play(&[tool(
            1,
            "Read",
            "r1",
            "completed",
            json!({"path":"a.php","ok":true}),
        )]);
        assert_eq!(file(&run, "a.php")["reads"], 1);
    }

    #[test]
    fn a_change_names_its_own_files() {
        let changes = json!({"changes":[
            {"path":"app/New.php","kind":"add"},
            {"path":"app/Old.php","kind":"delete"},
            {"path":"app\\Kept.php","kind":"update"},
            {"path":"notes.txt","kind":"update","outside":true},
        ],"ok":true});
        // Codex reports a change when it is done: the start names nothing.
        let run = play(&[
            tool(1, "file_change", "c1", "started", json!({})),
            tool(2, "file_change", "c1", "completed", changes),
        ]);
        assert!(m::yes(&file(&run, "app/New.php")["created"]));
        assert!(m::yes(&file(&run, "app/Old.php")["deleted"]));
        assert_eq!(file(&run, "app/Kept.php")["edits"], 1);
        assert!(m::yes(&file(&run, "notes.txt")["outside"]));
        for path in ["app/New.php", "app/Old.php", "app/Kept.php", "notes.txt"] {
            assert_eq!(file(&run, path)["edits"], 1, "{path}");
            assert_eq!(file(&run, path)["reads"], 0, "{path}");
        }
    }

    #[test]
    fn labels_without_call_ids_pair_in_order() {
        let label = |id: u64, text: &str| json!({"id":id,"kind":"tool","launch_id":L,"text":text});
        let run = play(&[
            label(1, "Read: started"),
            label(2, "Read: started"),
            label(3, "Bash: started"),
            label(4, "Read: completed"),
        ]);
        let open = |run: &V| {
            run["calls"]
                .as_object()
                .map(|calls| calls.values().filter(|call| m::yes(&call["open"])).count())
        };
        assert_eq!(run["steps"], 3);
        assert_eq!(open(&run), Some(2));
        let run = play(&[
            label(1, "Read: started"),
            label(2, "Read: completed"),
            label(3, "Read: completed"),
        ]);
        assert_eq!(open(&run), Some(0));
    }

    #[test]
    fn the_map_keeps_the_order_files_were_touched() {
        let mut events = done(1, "Read", "r1", json!({"path":"tests/OrderTest.php"}), true);
        events.extend(done(
            3,
            "Read",
            "r2",
            json!({"path":"app/Models/Order.php"}),
            true,
        ));
        events.extend(done(5, "Read", "r3", json!({"path":"composer.json"}), true));
        events.extend(done(
            7,
            "Edit",
            "e1",
            json!({"path":"app/Http/Controllers/Api/V1/Admin/OrderController.php"}),
            true,
        ));
        events.extend(done(
            9,
            "Read",
            "r4",
            json!({"path":"app/Models/User.php"}),
            true,
        ));
        events.extend(done(
            11,
            "Read",
            "r5",
            json!({"path":"secrets.env","outside":true}),
            true,
        ));
        let districts = map(&play(&events), "all", false);
        let names: Vec<_> = districts.iter().map(|d| m::text(d, "label")).collect();
        assert_eq!(names, ["tests/", "app/", "./", "Outside the project"]);
        let app = &districts[1];
        let blocks: Vec<_> = m::array(&app["blocks"])
            .iter()
            .map(|b| m::text(b, "name"))
            .collect();
        assert_eq!(blocks, ["Models", "Http/Controllers/Api/…"]);
        let models = &app["blocks"][0];
        assert_eq!(models["label"], "app/Models, 2 files");
        assert_eq!(
            app["blocks"][1]["label"],
            "app/Http/Controllers/Api/…, 1 file, 1 edited"
        );
        let chips: Vec<_> = m::array(&models["files"])
            .iter()
            .map(|c| m::text(c, "name"))
            .collect();
        assert_eq!(chips, ["Order.php", "User.php"]);
        assert_eq!(districts[2]["blocks"][0]["label"], "Project root, 1 file");
        assert_eq!(
            districts[3]["blocks"][0]["label"],
            "Outside the project, 1 file"
        );
    }

    #[test]
    fn a_wordpress_plugin_is_a_district_of_its_own() {
        let at = |path: &str| place_of(&json!({"path":path}));
        assert_eq!(
            at("wp-content/plugins/shop/includes/class-order.php"),
            ("wp-content/plugins/shop".into(), "includes".into())
        );
        assert_eq!(
            at("wp-content/themes/store/functions.php"),
            ("wp-content/themes/store".into(), String::new())
        );
        assert_eq!(
            at("wp-content/uploads/a.png"),
            ("wp-content".into(), "uploads".into())
        );
        assert_eq!(at("composer.json"), (String::new(), String::new()));
        assert_eq!(
            place_of(&json!({"path":"outside","outside":true})),
            (OUTSIDE.into(), String::new())
        );
    }

    #[test]
    fn a_chip_carries_what_happened_to_its_file() {
        let mut events = done(1, "Read", "r1", json!({"path":"app/Order.php"}), true);
        events.extend(done(3, "Read", "r2", json!({"path":"app/Order.php"}), true));
        events.extend(done(5, "Edit", "e1", json!({"path":"app/Order.php"}), true));
        events.extend(done(
            7,
            "Edit",
            "e2",
            json!({"path":"app/Broken.php"}),
            false,
        ));
        events.extend(done(
            9,
            "Read",
            "h1",
            json!({"path":"app/Helper.php","parent":"task-1"}),
            true,
        ));
        events.push(tool(
            11,
            "file_change",
            "c1",
            "completed",
            json!({"changes":[{"path":"app/New.php","kind":"add"},{"path":"app/Old.php","kind":"delete"}],"ok":true}),
        ));
        events.push(tool(
            12,
            "Read",
            "r9",
            "started",
            json!({"path":"app/Open.php"}),
        ));
        let run = play(&events);
        let chip = |running: bool, name: &str| {
            map(&run, "all", running)[0]["blocks"][0]["files"]
                .as_array()
                .and_then(|chips| chips.iter().find(|chip| chip["name"] == name).cloned())
                .unwrap_or(V::Null)
        };
        let order = chip(true, "Order.php");
        assert_eq!(order["times"], "×2");
        assert_eq!(order["edited"], true);
        assert_eq!(order["mark"], "");
        assert_eq!(
            order["label"],
            "app/Order.php, opened 2 times, edit reported ok"
        );
        let broken = chip(true, "Broken.php");
        assert_eq!(broken["mark"], "!");
        assert_eq!(broken["edited"], false);
        assert_eq!(broken["failed"], true);
        assert_eq!(chip(true, "New.php")["mark"], "+");
        assert_eq!(chip(true, "New.php")["created"], true);
        assert_eq!(chip(true, "Old.php")["deleted"], true);
        assert_eq!(chip(true, "Helper.php")["helper"], true);
        assert_eq!(order["helper"], false);
        assert_eq!(
            chip_words(file(&run, "app/Order.php")),
            ["opened 2 times", "edit reported ok"]
        );
        // The ring follows the call that is open, and only while the launch runs.
        assert_eq!(chip(true, "Open.php")["now"], true);
        assert_eq!(
            chip(true, "Open.php")["label"],
            "app/Open.php, opened once, in use now"
        );
        assert_eq!(chip(false, "Open.php")["now"], false);
        assert_eq!(order["now"], false);
    }

    #[test]
    fn filters_offer_what_the_launch_has_and_dim_the_rest() {
        let mut events = done(1, "Read", "r1", json!({"path":"app/Order.php"}), true);
        events.extend(done(3, "Edit", "e1", json!({"path":"app/Order.php"}), true));
        events.extend(done(
            5,
            "Read",
            "r2",
            json!({"path":"tests/OrderTest.php"}),
            true,
        ));
        let run = play(&events);
        let labels = |run: &V, chosen: &str| -> Vec<(String, bool)> {
            filters(run, chosen)
                .iter()
                .map(|f| (m::text(f, "label"), m::yes(&f["pressed"])))
                .collect()
        };
        assert_eq!(
            labels(&run, "all"),
            [("All".to_owned(), true), ("Edited 1".to_owned(), false)]
        );
        assert_eq!(
            labels(&run, "edited"),
            [("All".to_owned(), false), ("Edited 1".to_owned(), true)]
        );
        // A choice the launch has nothing for reads as All.
        assert_eq!(labels(&run, "failed")[0], ("All".to_owned(), true));
        let dim = |filter: &str| -> Vec<bool> {
            map(&run, filter, false)
                .iter()
                .flat_map(|d| m::array(&d["blocks"]))
                .flat_map(|b| m::array(&b["files"]))
                .map(|chip| m::yes(&chip["dim"]))
                .collect()
        };
        assert_eq!(dim("all"), [false, false]);
        assert_eq!(dim("edited"), [false, true]);
        assert!(!matches(&run, file(&run, "tests/OrderTest.php"), "edited"));
        // A search in a folder offers Searched for the files under it; one without a folder
        // looked everywhere.
        events.push(tool(
            7,
            "Grep",
            "g1",
            "started",
            json!({"detail":"rules(","path":"app"}),
        ));
        let run = play(&events);
        assert!(labels(&run, "all").contains(&("Searched 1".to_owned(), false)));
        assert_eq!(searched_in(&run, file(&run, "app/Order.php")), ["rules("]);
        assert!(searched_in(&run, file(&run, "tests/OrderTest.php")).is_empty());
        events.push(tool(
            8,
            "Glob",
            "g2",
            "started",
            json!({"detail":"**/*.php"}),
        ));
        let run = play(&events);
        assert!(labels(&run, "all").contains(&("Searched 2".to_owned(), false)));
    }

    #[test]
    fn the_card_says_what_the_page_knows_about_the_file() {
        let mut events = done(1, "Read", "r1", json!({"path":"app/Order.php"}), true);
        events.extend(done(3, "Edit", "e1", json!({"path":"app/Order.php"}), true));
        events.extend(done(5, "Edit", "e2", json!({"path":"app/Order.php"}), true));
        events.push(tool(
            7,
            "Grep",
            "g1",
            "started",
            json!({"detail":"rules(","path":"app"}),
        ));
        let run = play(&events);
        assert_eq!(
            card_facts(&run, file(&run, "app/Order.php")),
            [
                "Opened ×1",
                "In a folder searched for rules(",
                "Edit reported ok ×2"
            ]
        );
        assert_eq!(file(&run, "app/Order.php")["first_step"], 1);
        assert_eq!(file(&run, "app/Order.php")["last_step"], 3);
        let run = play(&[tool(
            1,
            "file_change",
            "c1",
            "completed",
            json!({"changes":[{"path":"a.php","kind":"add"}],"ok":true}),
        )]);
        assert_eq!(
            card_facts(&run, file(&run, "a.php")),
            ["Added (file_change)", "Edit reported ok"]
        );
    }

    #[test]
    fn the_card_stays_inside_the_window() {
        let (card, window) = ((360., 140.), (1280., 800.));
        // Under the chip, a little to its left.
        assert_eq!(place((400., 300., 324.), card, window), (392., 332.));
        // Above it when there is no room below.
        assert_eq!(place((400., 700., 724.), card, window), (392., 552.));
        // Never past the right or the left edge, nor above the top.
        assert_eq!(place((1200., 300., 324.), card, window).0, 912.);
        assert_eq!(place((2., 300., 324.), card, window).0, 8.);
        assert_eq!(place((400., 60., 84.), (360., 760.), window).1, 8.);
        // A window narrower than the card keeps the card's left edge on screen.
        assert_eq!(place((100., 300., 324.), card, (320., 800.)).0, 8.);
    }

    #[test]
    fn a_path_goes_into_the_draft_at_the_caret() {
        assert_eq!(spliced("", 0, 0, "app/a.php"), ("app/a.php".into(), 9));
        assert_eq!(
            spliced("Look at", 7, 7, "app/a.php"),
            ("Look at app/a.php".into(), 17)
        );
        assert_eq!(
            spliced("Look at ", 8, 8, "a.php"),
            ("Look at a.php".into(), 13)
        );
        assert_eq!(
            spliced("see here", 3, 3, "a.php"),
            ("see a.php here".into(), 9)
        );
        assert_eq!(
            spliced("see  here", 4, 4, "a.php"),
            ("see a.php here".into(), 9)
        );
        // A selection is replaced.
        assert_eq!(
            spliced("see THIS here", 4, 8, "a.php"),
            ("see a.php here".into(), 9)
        );
        // Offsets count UTF-16 units: Cyrillic is one each, an emoji two.
        assert_eq!(
            spliced("Посмотри тут", 8, 8, "a.php"),
            ("Посмотри a.php тут".into(), 14)
        );
        assert_eq!(spliced("😀 x", 2, 2, "a.php"), ("😀 a.php x".into(), 8));
        // A caret past the end, or an end before the start, still lands in the text.
        assert_eq!(spliced("ab", 9, 9, "c"), ("ab c".into(), 4));
        assert_eq!(spliced("ab", 2, 0, "c"), ("ab c".into(), 4));
    }

    #[test]
    fn counts_read_as_english() {
        assert_eq!(count(0, "step"), "0 steps");
        assert_eq!(count(1, "step"), "1 step");
        assert_eq!(count(2, "file"), "2 files");
        assert_eq!(
            counts(&json!({"steps":1,"messages":1})),
            "1 step · 1 message"
        );
        assert_eq!(
            counts(&json!({"steps":9,"messages":2})),
            "9 steps · 2 messages"
        );
    }

    #[test]
    fn inspect_file_graph_action_appears_in_list_rows_and_file_card_for_project_files() {
        let mut events = done(1, "Edit", "e1", json!({"path":"app/Order.php"}), true);
        events.extend(done(
            3,
            "Read",
            "r1",
            json!({"path":"secrets.env","outside":true}),
            true,
        ));
        let run = play(&events);
        let project_actions = run_card_actions(file(&run, "app/Order.php"));
        assert!(project_actions.iter().any(|a| {
            a.action == "inspect-file-graph"
                && a.label == "Show on dependency map"
                && a.data_path.as_deref() == Some("app/Order.php")
                && a.payload["path"] == "app/Order.php"
        }));
        let card_html = files_map::render_file_card_html(
            &entry_from_run_file(
                &run,
                file(&run, "app/Order.php"),
                &std::collections::BTreeSet::new(),
            ),
            &project_actions,
        );
        assert!(card_html.contains("data-action=\"inspect-file-graph\""));
        assert!(card_html.contains("data-path=\"app/Order.php\""));
        assert!(card_html.contains(">Show on dependency map</button>"));

        // Outside files have no project code graph node, so they omit inspect-file-graph.
        let outside_actions = run_card_actions(file(&run, "secrets.env"));
        assert!(!outside_actions
            .iter()
            .any(|a| a.action == "inspect-file-graph"));

        let entries = entries_of_run(&run, false);
        let list_html = files_map::render_files_surface(
            &entries,
            "list",
            "all",
            false,
            &files_map::FilesSurfaceOptions::default(),
        );
        assert!(list_html.contains("data-action=\"inspect-file-graph\""));
        assert!(list_html.contains("data-path=\"app/Order.php\">Map</button>"));
        assert!(!list_html.contains("data-path=\"secrets.env\">Map</button>"));
    }

    #[test]
    fn files_map_groups_multi_service_entries_by_service_district_and_subfolder_blocks() {
        let entries = vec![
            FileTouchEntry {
                path: "src/Domain/Order.php".into(),
                service_id: Some("orders".into()),
                service_folder: Some("services/orders".into()),
                agents: vec!["service-orders".into(), "verify".into()],
                reads: 2,
                edits: 1,
                created: true,
                now: true,
                order: 0,
                first_step: 1,
                last_step: 4,
                ..FileTouchEntry::default()
            },
            FileTouchEntry {
                path: "contracts/order-placed.json".into(),
                service_id: Some("__system__".into()),
                service_folder: Some(".".into()),
                agents: vec!["contracts".into()],
                reads: 1,
                order: 1,
                first_step: 2,
                last_step: 2,
                ..FileTouchEntry::default()
            },
            FileTouchEntry {
                path: "internal/charge/handler.go".into(),
                service_id: Some("payments".into()),
                service_folder: Some("../harbor-payments".into()),
                agents: vec!["service-payments".into()],
                reads: 1,
                failed: true,
                order: 2,
                first_step: 3,
                last_step: 3,
                ..FileTouchEntry::default()
            },
        ];

        let districts = files_map::group_districts(&entries, "edited", true, true, true);
        let labels: Vec<_> = districts.iter().map(|d| d.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "orders · services/orders",
                "__system__ · .",
                "payments · ../harbor-payments"
            ]
        );
        assert_eq!(districts[0].blocks[0].name, "src/Domain");
        let order_chip = &districts[0].blocks[0].files[0];
        assert_eq!(order_chip.mark, "+");
        assert_eq!(order_chip.times, "×2");
        assert_eq!(order_chip.agent_badge, "orders+verify");
        assert!(order_chip.edited);
        assert!(order_chip.now);
        assert!(!order_chip.dim);
        // Unedited files are dimmed under the "edited" filter while remaining visible in place.
        assert!(districts[1].blocks[0].files[0].dim);
        assert_eq!(districts[2].blocks[0].files[0].mark, "!");

        // Single-agent mode (group_by_service = false) groups by top folders 1:1 with Run Files map.
        let single_districts = files_map::group_districts(&entries[..1], "all", true, false, false);
        assert_eq!(single_districts[0].label, "src/");
        assert_eq!(single_districts[0].blocks[0].name, "Domain");

        let html = files_map::render_files_surface(
            &entries,
            "map",
            "all",
            true,
            &files_map::FilesSurfaceOptions {
                group_by_service: true,
                show_agent_badge: true,
                chip_action: "activity-file".into(),
                view_action: "activity-files-view".into(),
                filter_action: "activity-file-filter".into(),
                reveal_action: "activity-reveal-event".into(),
                inspect_graph_action: "inspect-system-file-graph".into(),
                root_id: Some("system-agents".into()),
            },
        );
        assert!(html.contains("orders · services/orders"));
        assert!(html.contains("data-now=\"true\""));
        assert!(html.contains("orders+verify"));
        assert!(html.contains("Edited 1"));
        assert!(html.contains("Failed 1"));
    }
}

#[cfg(test)]
mod clock_tests {
    use super::*;

    #[test]
    fn the_clock_tracks_only_an_active_unfinished_native_run() {
        let mut run =
            json!({"started":"2026-11-03T12:00:00Z","finished":false,"kinds":["tool","text"]});
        let active = json!({"status":"running"});
        assert_eq!(
            clock_state(&run, &active),
            Some(RunClock {
                started: Some("2026-11-03T12:00:00Z".into()),
                running: true,
            })
        );
        for status in ["completed", "failed", "cancelled", "interrupted"] {
            assert!(
                !clock_state(&run, &json!({"status":status}))
                    .unwrap()
                    .running
            );
        }
        run["finished"] = json!(true);
        assert!(!clock_state(&run, &active).unwrap().running);
        run["started"] = V::Null;
        assert!(clock_state(&run, &active).unwrap().started.is_none());
    }

    #[test]
    fn missing_and_managed_runs_do_not_keep_a_clock_timer() {
        let active = json!({"status":"running"});
        assert!(clock_state(&V::Null, &active).is_none());
        for kind in [
            "fleet_stage",
            "fleet_reviewer",
            "clash_turn",
            "agent",
            "agent_activity",
        ] {
            assert!(clock_state(&json!({"kinds":["tool",kind]}), &active).is_none());
        }
    }

    #[test]
    fn run_view_opens_for_active_sessions_or_nonempty_native_runs() {
        assert!(!run_view_open(&V::Null, &json!({"status": "completed"})));
        assert!(run_view_open(&V::Null, &json!({"status": "running"})));
        let empty = initial();
        assert!(!run_view_open(&empty, &json!({"status": "completed"})));
        assert!(run_view_open(&empty, &json!({"status": "running"})));
        let mut with_step = empty.clone();
        with_step["steps"] = json!(1);
        with_step["kinds"] = json!(["tool"]);
        assert!(run_view_open(&with_step, &json!({"status": "completed"})));
        let mut managed = with_step;
        managed["kinds"] = json!(["fleet_stage"]);
        assert!(!run_view_open(&managed, &json!({"status": "completed"})));
    }
}

#[cfg(test)]
mod permission_tests {
    use super::*;

    #[test]
    fn a_request_heading_names_its_host_tool_and_outcome() {
        let event = |state: &str| json!({"kind":"permission","state":state,"provider":"codex","tool":"Shell","request_id":"codex-7"});
        assert_eq!(
            permission_heading(&event("pending")),
            "Codex asks permission · Shell"
        );
        assert_eq!(permission_heading(&event("allowed")), "Allowed · Shell");
        assert_eq!(
            permission_heading(&event("allowed_session")),
            "Allowed for this session · Shell"
        );
        assert_eq!(permission_heading(&event("denied")), "Denied · Shell");
        assert_eq!(
            permission_heading(&event("declined")),
            "Declined automatically · Shell"
        );
        assert_eq!(
            permission_heading(&event("withdrawn")),
            "Withdrawn by Codex · Shell"
        );
        assert_eq!(
            permission_heading(&event("expired")),
            "No longer waiting · Shell"
        );
        let unnamed = json!({"state":"pending","provider":"claude"});
        assert_eq!(
            permission_heading(&unnamed),
            "Claude Code asks permission · Tool"
        );
    }

    #[test]
    fn permission_buttons_and_notifications_include_tool_context() {
        assert_eq!(
            permission_button_aria_label("allow", "Allow", "Bash"),
            "Allow Bash"
        );
        assert_eq!(
            permission_button_aria_label("allow_session", "Allow for this session", "Edit"),
            "Allow Edit for this session"
        );
        assert_eq!(
            permission_button_aria_label("deny", "Deny", "Shell"),
            "Deny Shell"
        );
        assert_eq!(
            permission_notification_body(&json!({"summary":"git status","detail":"detail"})),
            "git status"
        );
        assert_eq!(
            permission_notification_body(&json!({"summary":"","detail":"Not in allowlist"})),
            "Not in allowlist"
        );
        assert_eq!(
            permission_notification_body(&json!({})),
            "Waiting for your answer in Harness."
        );
        let expired = expired_permission_event(
            "cursor-1",
            "cursor",
            "Shell",
            "touch out.txt",
            "Not in allowlist",
        );
        assert_eq!(expired["state"], "expired");
        assert_eq!(expired["summary"], "touch out.txt");
        assert_eq!(expired["detail"], "Not in allowlist");
        assert_eq!(permission_heading(&expired), "No longer waiting · Shell");
    }

    #[test]
    fn outcome_cards_format_fleet_findings_clash_items_and_turns() {
        let finding = json!({
            "lens": "security_audit",
            "severity": "high",
            "claim": "Unsanitized path parameter",
            "file": "src/api.rs",
            "line": 42,
            "evidence": "User input passes directly to fs::read"
        });
        let fleet_card = build_fleet_finding_card(&finding, 0);
        assert_eq!(fleet_card.severity_class, "high");
        assert_eq!(fleet_card.heading, "security audit");
        assert_eq!(fleet_card.location.as_deref(), Some("src/api.rs:42"));
        assert_eq!(fleet_card.claim, "Unsanitized path parameter");
        assert_eq!(
            fleet_card.evidence.as_deref(),
            Some("User input passes directly to fs::read")
        );

        let clash_item = json!({
            "id": "CL-1",
            "kind": "objection",
            "origin": "challenger",
            "severity": "medium",
            "status": "disputed",
            "file": "src/auth.rs",
            "line": 17,
            "claim": "Token expiry check skipped on refresh",
            "evidence": "branch misses exp check",
            "history": [
                {"cycle": 1, "round": 2, "actor": "implementer", "action": "defended", "note": "checked upstream"}
            ]
        });
        let clash_card = build_clash_item_card(&clash_item, 1);
        assert_eq!(clash_card.severity_class, "medium");
        assert_eq!(clash_card.state_class.as_deref(), Some("disputed"));
        assert_eq!(clash_card.heading, "CL-1 · challenger objection");
        assert_eq!(clash_card.location.as_deref(), Some("src/auth.rs:17"));
        assert!(clash_card
            .evidence
            .as_deref()
            .unwrap()
            .contains("History (1): c1 r2 implementer defended"));

        let turn = json!({
            "cycle": 1,
            "round": 2,
            "role": "challenger",
            "provider": "codex",
            "ok": true,
            "summary": "Verified boundary guard",
            "seconds": 3.45,
            "cost_usd": 0.0125
        });
        let turn_card = build_clash_turn_card(&turn, 0);
        assert_eq!(turn_card.state_class, "resolved");
        assert_eq!(turn_card.state_label, "ok");
        assert_eq!(turn_card.heading, "Cycle 1 · Round 2 · challenger (codex)");
        assert_eq!(turn_card.meta.as_deref(), Some("3.5s · $0.0125"));
        assert_eq!(turn_card.summary, "Verified boundary guard");

        let card_actions = run_card_actions(&json!({"path": "src/lib.rs", "event_id": "e1"}));
        assert_eq!(
            card_actions.last().map(|a| a.action.as_str()),
            Some("run-card-close")
        );
    }

    #[test]
    fn rich_approval_renders_rule_hint_and_token_styled_diff_without_hex() {
        let with_session = json!({
            "session_option": true,
            "rule_summary": "Session rule: Bash(cargo test *)"
        });
        assert_eq!(
            permission_rule_hint_text(&with_session),
            "Allow for this session: Session rule: Bash(cargo test *)"
        );
        let without_session = json!({
            "session_option": false,
            "rule_summary": "Ask every time"
        });
        assert_eq!(
            permission_rule_hint_text(&without_session),
            "Ask every time"
        );
        assert_eq!(permission_rule_hint_text(&json!({})), "");

        let clean_diff = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,4 @@\n fn a() {}\n-let old = 1;\n+let next = \"<tag>&\";\n";
        let clean_html = render_diff_preview_lines_html(clean_diff);
        crate::test_support::assert_no_inline_hex_colors(&clean_html);
        assert!(clean_html.contains(
            "<span class=\"permission-diff-line\" data-kind=\"meta\">--- a/src/lib.rs</span>"
        ));
        assert!(clean_html.contains(
            "<span class=\"permission-diff-line\" data-kind=\"meta\">+++ b/src/lib.rs</span>"
        ));
        assert!(clean_html.contains(
            "<span class=\"permission-diff-line\" data-kind=\"hunk\">@@ -1,3 +1,4 @@</span>"
        ));
        assert!(clean_html.contains("data-kind=\"ctx\"> fn a() {}</span>"));
        assert!(clean_html.contains("data-kind=\"del\">-let old = 1;</span>"));
        assert!(clean_html
            .contains("data-kind=\"add\">+let next = &quot;&lt;tag&gt;&amp;&quot;;</span>"));
    }

    #[test]
    fn question_cards_render_single_and_multi_select_without_hex_and_format_resolved_summary() {
        let event = json!({
            "request_id": "req:1/a",
            "provider": "claude",
            "tool": "AskUserQuestion",
            "state": "pending",
            "ask_kind": "question",
            "questions": [
                {
                    "id": "auth_mode",
                    "header": "Auth",
                    "question": "Which mode should <API> use?",
                    "multi_select": false,
                    "options": [
                        {"label": "JWT", "description": "Stateless bearer token"},
                        {"label": "Session", "description": "Server-side cookie"}
                    ]
                },
                {
                    "id": "targets",
                    "header": "Targets",
                    "question": "Select targets",
                    "multi_select": true,
                    "options": [
                        {"label": "API", "description": "REST routes"},
                        {"label": "CLI", "description": "Command runner"}
                    ]
                }
            ]
        });
        assert!(is_question_permission(&event));
        assert_eq!(
            permission_heading(&event),
            "Claude Code asks a question · AskUserQuestion"
        );
        let resolved_heading = permission_heading(&json!({
            "request_id": "req:1/a",
            "provider": "claude",
            "tool": "AskUserQuestion",
            "state": "allowed",
            "ask_kind": "question"
        }));
        assert_eq!(resolved_heading, "Answered · AskUserQuestion");

        let questions = event["questions"].as_array().unwrap();
        let html = render_question_card_html("req:1/a", questions);
        crate::test_support::assert_no_inline_hex_colors(&html);
        assert!(html.contains("name=\"pq-req_1_a-0\""));
        assert!(html.contains("name=\"pq-req_1_a-1\""));
        assert!(html.contains("type=\"radio\""));
        assert!(html.contains("type=\"checkbox\""));
        assert!(html.contains("&lt;API&gt;"));
        assert!(html.contains("Stateless bearer token"));
        assert!(html.contains("permission-question-other-radio"));
        assert!(html.contains("permission-question-other-input"));
        assert!(html.contains("aria-label=\"Other answer for Auth\""));
        assert!(html.contains("aria-label=\"Other answer for Targets\""));

        let allow_body = build_permission_decision_body(
            "allow",
            Some(json!({"auth_mode": "JWT", "targets": ["API", "CLI"]})),
        );
        assert_eq!(allow_body["decision"], "allow");
        assert_eq!(allow_body["answers"]["auth_mode"], "JWT");
        assert_eq!(allow_body["answers"]["targets"], json!(["API", "CLI"]));
        let deny_body = build_permission_decision_body("deny", Some(json!({"ignored": "x"})));
        assert_eq!(deny_body, json!({"decision": "deny"}));

        let summary_html = render_answers_summary_html(
            questions,
            &json!({"auth_mode": "JWT", "targets": ["API", "CLI"]}),
        );
        crate::test_support::assert_no_inline_hex_colors(&summary_html);
        assert!(summary_html.contains("Submitted answers"));
        assert!(summary_html.contains("<dt>Auth</dt><dd>JWT</dd>"));
        assert!(summary_html.contains("<dt>Targets</dt><dd>API, CLI</dd>"));
    }

    #[test]
    fn shared_waiting_permission_cards_scope_containers_preserve_order_and_avoid_hex() {
        assert_eq!(
            super::permission_target_container(
                &json!({"request_id": "r1", "container": "system-permissions-list"}),
                "sessions",
                Some("events"),
            ),
            "system-permissions-list"
        );
        assert_eq!(
            super::permission_target_container(&json!({"request_id": "r1"}), "creator", None,),
            "creator-permissions-list"
        );
        assert_eq!(
            super::permission_target_container(
                &json!({"request_id": "r1"}),
                "system-changes",
                None,
            ),
            "system-permissions-list"
        );
        assert_eq!(
            super::permission_target_container(&json!({"request_id": "r1"}), "sessions", None,),
            "events"
        );

        assert_eq!(super::permission_panel_lead(&[]), "");
        assert_eq!(
            super::permission_panel_lead(&[json!({"request_id": "r1"})]),
            "Waiting for your answer to a permission request…"
        );
        assert_eq!(
            super::permission_panel_lead(&[
                json!({"request_id": "r1"}),
                json!({"request_id": "r2"}),
            ]),
            "Waiting for your answers to 2 permission requests…"
        );

        let existing = vec!["r-stale".to_owned(), "r-keep".to_owned()];
        let waiting = vec![
            json!({"request_id": "r-keep", "tool": "Edit"}),
            json!({"request_id": "r-keep", "tool": "Edit"}),
            json!({"request_id": ""}),
            json!({"request_id": "r-new", "tool": "Bash"}),
        ];
        let (remove, live) = super::waiting_permission_reconcile_plan(&existing, &waiting);
        assert_eq!(remove, vec!["r-stale".to_owned()]);
        assert_eq!(live.len(), 2);
        assert_eq!(live[0].0, "r-keep");
        assert_eq!(live[0].1["state"], "pending");
        assert_eq!(live[1].0, "r-new");
        assert_eq!(live[1].1["state"], "pending");

        let edit_ask = json!({
            "request_id": "sys-1",
            "provider": "claude",
            "tool": "Edit",
            "summary": "services/billing/src/Invoice.php",
            "detail": "[Auto -> Manual boundary escalation: touches billing]",
            "session_option": false,
            "rule_summary": "Cross-service boundary requires one-off approval",
            "diff_preview": "--- a/Invoice.php\n+++ b/Invoice.php\n@@ -1 +1 @@\n-old\n+new\n",
        });
        let edit_html =
            super::render_permission_card_html("system-permissions-list", &edit_ask, true);
        crate::test_support::assert_no_inline_hex_colors(&edit_html);
        assert!(edit_html.contains("data-permission-id=\"sys-1\""));
        assert!(edit_html.contains("role=\"group\""));
        assert!(edit_html.contains("role=\"alert\""));
        assert!(edit_html.contains("Cross-service boundary requires one-off approval"));
        assert!(edit_html.contains("permission-diff-wrap"));
        let edit_payloads = crate::test_support::payloads(&edit_html, "permission-decide");
        assert_eq!(edit_payloads.len(), 2);
        assert_eq!(edit_payloads[0]["container"], "system-permissions-list");
        assert_eq!(edit_payloads[0]["decision"], "allow");
        assert_eq!(edit_payloads[1]["decision"], "deny");

        let question_ask = json!({
            "request_id": "q-1",
            "provider": "codex",
            "tool": "AskUserQuestion",
            "ask_kind": "question",
            "questions": [{
                "id": "scope",
                "header": "Scope",
                "question": "Pick target",
                "options": [{"label": "PHP", "description": "PHP stack"}]
            }]
        });
        let q_html =
            super::render_permission_card_html("creator-permissions-list", &question_ask, true);
        crate::test_support::assert_no_inline_hex_colors(&q_html);
        assert!(q_html.contains("name=\"pq-creator-permissions-list-q-1-0\""));
        let other_payloads = crate::test_support::payloads(&q_html, "permission-question-other");
        assert_eq!(other_payloads[0]["container"], "creator-permissions-list");
        let q_decide = crate::test_support::payloads(&q_html, "permission-decide");
        assert_eq!(q_decide[0]["container"], "creator-permissions-list");
    }

    #[test]
    fn stream_markdown_coalesce_scroll_grouped_diff_and_countdown_contracts() {
        let md_html = render_assistant_markdown_html("Hello **world**\n```rust\nfn x() {}\n```");
        crate::test_support::assert_no_inline_hex_colors(&md_html);
        assert!(md_html.contains("data-action=\"copy-code\""));
        assert!(md_html.contains("class=\"md-code-block\""));
        assert!(md_html.contains(">rust<"));

        assert_eq!(coalesce_chunk_text("foo ", "bar"), "foo bar");
        assert_eq!(coalesce_chunk_text("Hel", "lo"), "Hello");
        assert_eq!(coalesce_chunk_text("**bo", "ld**"), "**bold**");
        assert_eq!(coalesce_chunk_text("line1\n", "line2"), "line1\nline2");

        assert!(is_near_bottom(1000, 660, 300));
        assert!(!is_near_bottom(1000, 600, 300));

        let diff = "\
diff --git a/src/a.rs b/src/a.rs
index 111..222 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,2 +1,3 @@
 fn a() {}
+fn b() {}
-fn old() {}
diff --git a/src/new.rs b/src/new.rs
new file mode 100644
--- /dev/null
+++ b/src/new.rs
@@ -0,0 +1,2 @@
+pub fn n() {}
+pub fn m() {}
";
        let files = parse_diff_files(diff);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "src/a.rs");
        assert_eq!(files[0].status, "modified");
        assert_eq!(files[0].additions, 1);
        assert_eq!(files[0].deletions, 1);
        assert_eq!(files[1].path, "src/new.rs");
        assert_eq!(files[1].status, "added");
        assert_eq!(files[1].additions, 2);

        let grouped_html = render_grouped_diff_html(diff, "results-diff");
        crate::test_support::assert_no_inline_hex_colors(&grouped_html);
        assert!(grouped_html.contains("data-diff-file=\"src/a.rs\""));
        assert!(grouped_html.contains("data-diff-file=\"src/new.rs\""));
        assert!(grouped_html.contains("data-action=\"diff-expand-all\""));

        let ms = parse_rfc3339_utc_ms("2026-11-19T12:00:00Z").unwrap();
        let ms_frac = parse_rfc3339_utc_ms("2026-11-19T12:00:00.500+00:00").unwrap();
        assert!((ms_frac - ms - 500.0).abs() < 1.0);

        assert_eq!(
            format_countdown_badge(0.0),
            ("expired", "Expired".to_owned())
        );
        assert_eq!(
            format_countdown_badge(15_000.0),
            ("urgent", "Expires in 15s".to_owned())
        );
        assert_eq!(
            format_countdown_badge(125_000.0),
            ("normal", "Expires in 2:05".to_owned())
        );

        let timed_ask = json!({
            "request_id": "timed-1",
            "tool": "Bash",
            "summary": "phpunit",
            "state": "pending",
            "timeout_seconds": 90
        });
        let timed_html = render_permission_card_html("events", &timed_ask, true);
        crate::test_support::assert_no_inline_hex_colors(&timed_html);
        assert!(timed_html.contains("permission-expiry-badge"));
        assert!(timed_html.contains("data-expires-ms="));
    }

    #[test]
    fn incremental_markdown_copy_timers_and_persistent_diff_open_state() {
        assert_eq!(copy_timer_key(Some("blk-1"), 7), "md-copy-reset-blk-1");
        assert_eq!(copy_timer_key(Some("blk-2"), 7), "md-copy-reset-blk-2");
        assert_ne!(
            copy_timer_key(Some("blk-1"), 1),
            copy_timer_key(Some("blk-2"), 1)
        );
        assert_eq!(copy_timer_key(None, 42), "md-copy-reset-42");

        // Unclosed fence must not advance settled boundary past the paragraph before the fence.
        let streaming_open_fence = "First para.\n\n```rust\nlet x = 1;\n\nstill inside fence";
        let settled = find_settled_markdown_boundary(streaming_open_fence, 0);
        assert_eq!(&streaming_open_fence[..settled], "First para.\n\n");

        // Closed fence with trailing newline settles the complete code block while trailing chunk remains live.
        let streaming_closed_fence = "First para.\n\n```rust\nlet x = 1;\n```\n\nTrailing live";
        let settled2 = find_settled_markdown_boundary(streaming_closed_fence, 0);
        assert_eq!(
            &streaming_closed_fence[..settled2],
            "First para.\n\n```rust\nlet x = 1;\n```\n\n"
        );
        assert_eq!(
            format!(
                "{}{}",
                render_assistant_markdown_html(&streaming_closed_fence[..settled2]),
                render_assistant_markdown_html(&streaming_closed_fence[settled2..])
            ),
            render_assistant_markdown_html(streaming_closed_fence)
        );

        let blocks = split_top_level_html_blocks(
            "<p>One</p><div class=\"md-code-block\"><pre><code>fn a() {}</code></pre></div><p>Two</p>",
        );
        assert_eq!(blocks.len(), 3);
        assert!(blocks[0].starts_with("<p>One"));
        assert!(blocks[1].starts_with("<div class=\"md-code-block\">"));
        assert!(blocks[2].starts_with("<p>Two"));

        let mut expanded = std::collections::HashSet::new();
        let mut collapsed = std::collections::BTreeSet::new();
        assert!(is_diff_file_open(
            true,
            "workspace-results:a.rs",
            &expanded,
            &collapsed
        ));
        assert!(!is_diff_file_open(
            false,
            "workspace-results:z.rs",
            &expanded,
            &collapsed
        ));

        // Explicit user expand overrides >20-file default collapsed state.
        expanded.insert("workspace-results:z.rs".into());
        assert!(is_diff_file_open(
            false,
            "workspace-results:z.rs",
            &expanded,
            &collapsed
        ));

        // Explicit user collapse overrides default open state.
        collapsed.insert("workspace-results:a.rs".into());
        assert!(!is_diff_file_open(
            true,
            "workspace-results:a.rs",
            &expanded,
            &collapsed
        ));

        assert_eq!(
            diff_content_hash("workspace-results", "diff --git a/x b/x"),
            diff_content_hash("workspace-results", "diff --git a/x b/x")
        );
        assert_ne!(
            diff_content_hash("workspace-results", "diff A"),
            diff_content_hash("workspace-results", "diff B")
        );
        assert_ne!(
            diff_content_hash("workspace-results", "diff A"),
            diff_content_hash("results-diff", "diff A")
        );
    }

    #[test]
    fn strip_now_parts_preserves_verb_target_and_more_slots() {
        assert_eq!(
            super::run_strip_now_parts(true, "completed", &[]),
            ("Run completed".into(), String::new(), String::new(), true)
        );
        assert_eq!(
            super::run_strip_now_parts(false, "", &[]),
            ("Model’s turn".into(), String::new(), String::new(), true)
        );
        let single = [json!({"tool": "Read", "path": "src/main.rs"})];
        assert_eq!(
            super::run_strip_now_parts(false, "", &single),
            ("Read".into(), "src/main.rs".into(), String::new(), false)
        );
        let multi = [
            json!({"tool": "Read", "path": "src/a.rs", "started": 1, "event_id": "1"}),
            json!({"tool": "Edit", "path": "src/b.rs", "started": 2, "event_id": "2"}),
        ];
        assert_eq!(
            super::run_strip_now_parts(false, "", &multi),
            ("Edit".into(), "src/b.rs".into(), "+1 more".into(), false)
        );
    }

    #[test]
    fn stream_boundary_tool_output_bounds_and_diff_viewer_enhancements() {
        // Tilde fences and indented list continuations stay intact.
        let tilde_open = "Intro.\n\n~~~python\nprint(1)\n\nstill_in_fence";
        assert_eq!(
            &tilde_open[..find_settled_markdown_boundary(tilde_open, 0)],
            "Intro.\n\n"
        );
        let tilde_closed = "Intro.\n\n~~~python\nprint(1)\n~~~\n\nAfter";
        assert_eq!(
            &tilde_closed[..find_settled_markdown_boundary(tilde_closed, 0)],
            "Intro.\n\n~~~python\nprint(1)\n~~~\n\n"
        );
        let list_cont = "- item 1\n\n  continuation";
        assert_eq!(find_settled_markdown_boundary(list_cont, 0), 0);
        let setext = "Title\n\n===";
        assert_eq!(find_settled_markdown_boundary(setext, 0), 0);

        assert_eq!(crate::dom::escape_css_attr_value(r#"a"b\c"#), r#"a\"b\\c"#);
        let long_out = (1..=300)
            .map(|n| format!("line-{n:03}-payload-abcdefghijklmnopqrstuvwxyz"))
            .collect::<Vec<_>>()
            .join("\n");
        let (tail, trunc) = bounded_tool_output_preview(&long_out, 500);
        assert!(trunc.is_some());
        assert!(tail.len() <= 550);
        assert!(tail.ends_with("line-300-payload-abcdefghijklmnopqrstuvwxyz"));
        assert_eq!(
            bounded_tool_output_preview("short text", 500),
            ("short text".into(), None)
        );

        let numbered = compute_diff_numbered_lines(
            "diff --git a/src/a.rs b/src/a.rs\n@@ -10,2 +20,3 @@ fn x\n ctx\n-old\n+new1\n+new2",
        );
        assert_eq!(numbered[2].old_ln, Some(10));
        assert_eq!(numbered[2].new_ln, Some(20));
        assert_eq!(numbered[3].kind, "del");
        assert_eq!(numbered[3].old_ln, Some(11));
        assert_eq!(numbered[3].new_ln, None);
        assert_eq!(numbered[4].kind, "add");
        assert_eq!(numbered[4].new_ln, Some(21));

        let (diff_html, total_lines, is_trunc) = render_diff_file_lines_html(
            "diff --git a/src/a.rs b/src/a.rs\n@@ -1,1 +1,1 @@\n-a\n+b",
            Some(250),
        );
        assert!(!is_trunc);
        assert_eq!(total_lines, 4);
        assert!(diff_html.contains("data-action=\"diff-copy-hunk\""));
        assert!(diff_html.contains("class=\"diff-ln diff-ln-old\""));
        crate::test_support::assert_no_inline_hex_colors(&diff_html);

        assert!(should_default_open_diff_file(4, 2, 120));
        assert!(!should_default_open_diff_file(12, 2, 120));
        assert!(!should_default_open_diff_file(4, 2, 500));
    }

    #[test]
    fn approvals_queue_reconcile_shortcuts_and_creator_detail_counts() {
        let existing = vec!["s1:r1".into(), "s2:r2".into(), "sys:r3".into()];
        let next = vec!["s2:r2".into(), "s3:r4".into()];
        assert_eq!(
            approvals_queue_reconcile_plan(&existing, &next),
            vec!["s1:r1".to_owned(), "sys:r3".to_owned()]
        );

        assert!(!is_shortcut_text_input("INPUT", Some("radio")));
        assert!(!is_shortcut_text_input("INPUT", Some("checkbox")));
        assert!(is_shortcut_text_input("INPUT", Some("text")));
        assert!(is_shortcut_text_input("TEXTAREA", None));
        assert!(is_shortcut_text_input("SELECT", None));
        assert!(!can_target_drawer_shortcut(false, false, false));
        assert!(can_target_drawer_shortcut(true, false, false));
        assert!(can_target_drawer_shortcut(false, true, false));
        assert!(
            super::super::permissions::should_toggle_approvals_drawer_on_alt_a(false, false, false)
        );
        assert!(
            super::super::permissions::should_toggle_approvals_drawer_on_alt_a(true, false, false)
        );
        assert!(
            !super::super::permissions::should_toggle_approvals_drawer_on_alt_a(true, true, false)
        );
        assert!(
            !super::super::permissions::should_toggle_approvals_drawer_on_alt_a(false, false, true)
        );

        m::write(|s| {
            s.sid = "active-sess".into();
            s.sessions.clear();
            s.approvals.waiting_counts.clear();
            s.data.insert(
                "system-run".into(),
                json!({"id": "sr-1", "waiting_permissions": [{"request_id": "p1"}]}),
            );
            s.data.insert(
                "creator-detail".into(),
                json!({"run": {"id": "cr-1"}, "waiting_permissions": [{"request_id": "p2"}, {"request_id": "p3"}]}),
            );
        });
        let total = super::super::permissions::total_pending_approvals_count();
        assert!(total >= 3);
        m::take("system-run");
        m::take("creator-detail");
    }

    #[test]
    fn approvals_waiting_counts_reconcile_back_to_zero_after_approval() {
        let mut sessions = vec![
            json!({"id": "s-perm", "waiting_permissions": 1}),
            json!({"id": "s-other", "waiting_permissions": 2}),
        ];
        let mut waiting_counts = std::collections::BTreeMap::new();
        waiting_counts.insert("s-perm".to_owned(), 1);
        waiting_counts.insert("s-other".to_owned(), 2);

        assert_eq!(
            super::super::permissions::compute_total_pending_approvals_count(
                &sessions,
                &waiting_counts,
                "s-perm",
                0,
                0,
            ),
            3
        );

        super::super::permissions::apply_session_waiting_count(
            &mut sessions,
            &mut waiting_counts,
            "s-perm",
            0,
        );
        assert_eq!(
            super::super::permissions::compute_total_pending_approvals_count(
                &sessions,
                &waiting_counts,
                "s-perm",
                0,
                0,
            ),
            2
        );
        assert!(!waiting_counts.contains_key("s-perm"));
        assert_eq!(sessions[0]["waiting_permissions"], json!(0));

        let snapshot = vec![
            json!({"id": "s-perm", "waiting_permissions": 0}),
            json!({"id": "s-other", "waiting_permissions": 0}),
        ];
        super::super::permissions::sync_waiting_counts_from_sessions(
            &snapshot,
            &mut waiting_counts,
        );
        assert_eq!(
            super::super::permissions::compute_total_pending_approvals_count(
                &snapshot,
                &waiting_counts,
                "s-perm",
                0,
                0,
            ),
            0
        );
        assert!(waiting_counts.is_empty());
    }

    #[test]
    fn resolve_permission_expiry_ms_preserves_initial_timeout_deadline() {
        let mut cache = std::collections::BTreeMap::new();
        let event = json!({"request_id": "req-1", "timeout_seconds": 30});
        let first = super::super::permissions::resolve_permission_expiry_ms(
            &mut cache, "req-1", &event, 1_000.0,
        );
        let second = super::super::permissions::resolve_permission_expiry_ms(
            &mut cache, "req-1", &event, 15_000.0,
        );
        assert_eq!(first, Some(31_000.0));
        assert_eq!(second, Some(31_000.0));

        let updated_timeout = json!({"request_id": "req-1", "timeout_seconds": 60});
        let third = super::super::permissions::resolve_permission_expiry_ms(
            &mut cache,
            "req-1",
            &updated_timeout,
            16_000.0,
        );
        assert_eq!(third, Some(76_000.0));

        let explicit = json!({"request_id": "req-2", "expires_at": "2026-01-01T00:00:10Z"});
        let parsed = super::super::permissions::resolve_permission_expiry_ms(
            &mut cache, "req-2", &explicit, 1_000.0,
        );
        assert!(parsed.is_some());
    }

    #[test]
    fn fold_run_events_and_prune_tool_outputs_match_incremental_state() {
        let events = vec![
            json!({"id": 1, "kind": "status", "launch_id": "l1", "text": "Starting"}),
            json!({"id": 2, "kind": "tool", "launch_id": "l1", "tool": "Edit", "call": "c1", "state": "started", "path": "src/a.rs"}),
            json!({"id": 3, "kind": "tool", "launch_id": "l1", "tool": "Edit", "call": "c1", "state": "completed", "ok": true, "path": "src/a.rs"}),
            json!({"id": 4, "kind": "status", "launch_id": "l2", "text": "Follow-up"}),
        ];
        let (folded, before, calls) = super::super::stream::fold_run_events(&events);
        assert_eq!(folded["launch"], "l2");
        assert_eq!(before.as_ref().unwrap()["launch"], "l1");
        assert_eq!(calls.len(), 4);
        assert_eq!(calls[1].as_deref(), Some("c1"));
        assert_eq!(calls[2].as_deref(), Some("c1"));

        let mut outputs = std::collections::BTreeMap::new();
        for i in 0..300 {
            outputs.insert(format!("k-{i:03}"), format!("out-{i}"));
        }
        super::super::stream::prune_tool_outputs_map(&mut outputs, 256);
        assert_eq!(outputs.len(), 256);
        assert!(!outputs.contains_key("k-000"));
        assert!(outputs.contains_key("k-299"));
    }

    #[test]
    fn approvals_sessions_to_fetch_uses_count_cache_and_invalidation() {
        let mut cache = std::collections::BTreeMap::new();
        cache.insert("s1".to_owned(), (2usize, vec![json!({"request_id": "r1"})]));
        cache.insert("s2".to_owned(), (1usize, vec![json!({"request_id": "r2"})]));

        let waiting = vec![
            ("s1".to_owned(), "One".to_owned(), 2usize),
            ("s2".to_owned(), "Two".to_owned(), 3usize),
            ("s3".to_owned(), "Three".to_owned(), 1usize),
        ];

        assert_eq!(
            super::super::permissions::approvals_sessions_to_fetch(&waiting, &cache, false),
            vec![
                ("s2".to_owned(), "Two".to_owned()),
                ("s3".to_owned(), "Three".to_owned()),
            ]
        );
        assert_eq!(
            super::super::permissions::approvals_sessions_to_fetch(&waiting, &cache, true).len(),
            3
        );
    }

    #[test]
    fn diff_deferral_chunking_and_approvals_ux_rules_work() {
        // Collapsed files in multi-file diffs emit deferred <pre> placeholders without eagerly rendering lines.
        let mut multi_diff = String::new();
        for i in 0..10 {
            multi_diff.push_str(&format!(
                "diff --git a/src/f{i}.rs b/src/f{i}.rs\n--- a/src/f{i}.rs\n+++ b/src/f{i}.rs\n@@ -1,1 +1,1 @@\n-old\n+new\n"
            ));
        }
        let grouped = super::super::diff::render_grouped_diff_html(&multi_diff, "results-diff");
        crate::test_support::assert_no_inline_hex_colors(&grouped);
        assert!(grouped.contains("data-diff-deferred=\"true\""));
        assert_eq!(
            super::super::diff::diff_chunk_ranges(750, 300),
            vec![(0, 300), (300, 600), (600, 750)]
        );

        // Countdown terminates once all pending cards reach Expired.
        assert!(super::super::permissions::has_unexpired_countdown(
            &[900.0, 1500.0],
            1000.0
        ));
        assert!(!super::super::permissions::has_unexpired_countdown(
            &[900.0, 1000.0],
            1000.0
        ));

        // Scoped card decisions stay blocked inside text inputs while global Alt+A still toggles.
        assert!(!super::super::permissions::allow_scoped_permission_shortcut(true, true));
        assert!(!super::super::permissions::allow_scoped_permission_shortcut(false, false));
        assert!(super::super::permissions::allow_scoped_permission_shortcut(
            true, false
        ));
        assert!(
            super::super::permissions::should_toggle_approvals_drawer_on_alt_a(false, false, false)
        );
        assert!(
            super::super::permissions::should_toggle_approvals_drawer_on_alt_a(true, false, false)
        );

        // Approvals drawer cap (12) prioritizes active s.sid then current_project().
        let mut sessions = Vec::new();
        let mut waiting_counts = std::collections::BTreeMap::new();
        for i in 0..15 {
            let sid = format!("s-{i}");
            let proj = match i {
                14 => "proj-a",
                12 => "proj-a",
                _ => "proj-b",
            };
            sessions.push(json!({"id": sid, "title": format!("S{i}"), "project_id": proj, "waiting_permissions": 1}));
            waiting_counts.insert(format!("s-{i}"), 1usize);
        }
        let (prioritized, total) =
            super::super::permissions::prioritize_waiting_sessions_for_drawer(
                &sessions,
                &waiting_counts,
                "s-14",
                "proj-a",
                12,
            );
        assert_eq!(total, 15);
        assert_eq!(prioritized.len(), 12);
        assert_eq!(prioritized[0].0, "s-14");
        assert_eq!(prioritized[1].0, "s-12");

        // Clearing pending diff work for one scope cancels in-flight expand chunks and advances
        // generations for that scope only without disturbing unrelated diff scopes.
        let (k_results, gen_results) =
            super::super::diff::seed_pending_expand_for_test("results-diff", "src/big.rs");
        let (k_side, gen_side) =
            super::super::diff::seed_pending_expand_for_test("workspace-results", "src/big.rs");
        assert!(super::super::diff::pending_expand_still_valid_for_test(
            &k_results,
            gen_results
        ));
        assert!(super::super::diff::pending_expand_still_valid_for_test(
            &k_side, gen_side
        ));
        super::super::diff::clear_pending_diff_work(Some("results-diff"));
        assert!(!super::super::diff::pending_expand_still_valid_for_test(
            &k_results,
            gen_results
        ));
        assert!(super::super::diff::pending_expand_still_valid_for_test(
            &k_side, gen_side
        ));
        super::super::diff::clear_pending_diff_work(None);
        assert!(!super::super::diff::pending_expand_still_valid_for_test(
            &k_side, gen_side
        ));

        // Expand-all batch work cancels cleanly via clear_pending_diff_work on targeted scope only.
        let gen_ea_a = super::super::diff::seed_pending_expand_all_for_test("results-diff");
        let gen_ea_b = super::super::diff::seed_pending_expand_all_for_test("workspace-diff");
        assert!(super::super::diff::pending_expand_all_still_valid_for_test(
            "results-diff",
            gen_ea_a
        ));
        assert!(super::super::diff::pending_expand_all_still_valid_for_test(
            "workspace-diff",
            gen_ea_b
        ));
        super::super::diff::clear_pending_diff_work(Some("results-diff"));
        assert!(
            !super::super::diff::pending_expand_all_still_valid_for_test("results-diff", gen_ea_a)
        );
        assert!(super::super::diff::pending_expand_all_still_valid_for_test(
            "workspace-diff",
            gen_ea_b
        ));
        super::super::diff::clear_pending_diff_work(None);

        // Git header & diff section parser handles " b/" in path, C-escaped quotes/octal, and renames.
        let edge_diff = concat!(
            "diff --git a/dir with b/file.rs b/dir with b/file.rs\n",
            "index 111..222 100644\n",
            "--- a/dir with b/file.rs\n",
            "+++ b/dir with b/file.rs\n",
            "@@ -1 +1 @@\n",
            "-old\n",
            "+new\n",
            "diff --git \"a/quote \\\"name\\\" \\303\\251.txt\" \"b/quote \\\"name\\\" \\303\\251.txt\"\n",
            "index 111..222 100644\n",
            "--- \"a/quote \\\"name\\\" \\303\\251.txt\"\n",
            "+++ \"b/quote \\\"name\\\" \\303\\251.txt\"\n",
            "@@ -1 +1 @@\n",
            "-x\n",
            "+y\n",
            "diff --git a/sub b/old name.rs b/sub b/new name.rs\n",
            "similarity index 100%\n",
            "rename from sub b/old name.rs\n",
            "rename to sub b/new name.rs\n",
        );
        let parsed_edges = super::super::diff::parse_diff_files(edge_diff);
        assert_eq!(parsed_edges.len(), 3);
        assert_eq!(parsed_edges[0].path, "dir with b/file.rs");
        assert_eq!(parsed_edges[1].path, "quote \"name\" é.txt");
        assert_eq!(parsed_edges[2].status, "renamed");
        assert_eq!(parsed_edges[2].old_path, "sub b/old name.rs");
        assert_eq!(parsed_edges[2].path, "sub b/new name.rs");
    }
}
