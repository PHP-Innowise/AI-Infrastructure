use crate::{dom, model as m, session};
use serde_json::{json, Value as V};
use std::collections::BTreeMap;
use wasm_bindgen::JsCast;
fn clean(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(*c,'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}'))
        .collect()
}
pub fn reset() {
    dom::cancel_timer("run-clock");
    for key in [
        "run",
        "agents",
        "run-file-filter",
        "run-command-filter",
        "run-before",
        "run-after",
        "run-idle",
    ] {
        m::take(key);
    }
    dom::clear("events");
    dom::clear("run-tree");
    dom::clear("run-pane-plan");
    dom::clear("run-pane-commands");
    // The away summary belongs to the session being left.
    away(false);
    // The session being left closes its strip now, not when the next one answers.
    render();
}
fn initial() -> V {
    json!({"launch":"","calls":{},"open":[],"files":{},"commands":[],"plan":[],"searches":[],"steps":0,"messages":0,"finished":false,"outcome":null,"memory":null,"kinds":[],"started":null})
}
fn tool_name(v: &V) -> String {
    let name = v["tool"].as_str().unwrap_or_else(|| {
        v["text"]
            .as_str()
            .unwrap_or("")
            .split(": ")
            .next()
            .unwrap_or("Tool")
    });
    let name = name.strip_suffix("ToolCall").unwrap_or(name);
    match name {
        "command_execution" => "Shell",
        "file_change" => "Edit",
        "mcp_tool_call" => "MCP",
        "web_search" => "Web search",
        _ => name,
    }
    .into()
}
fn starts(v: &V) -> bool {
    if let Some(state) = v["state"].as_str() {
        return state == "started" || state == "in_progress";
    }
    let text = m::text(v, "text");
    let tail = text.split_once(": ").map(|(_, s)| s).unwrap_or("");
    !["completed", "failed", "declined"]
        .iter()
        .any(|p| tail.starts_with(p))
}
pub fn classify(command: &str) -> V {
    crate::classify::classify(command)
}
pub fn observe(event: V) {
    let (run, replaced) = advance(m::data("run"), &event);
    if let Some(replaced) = replaced {
        m::set("run-before", replaced);
    }
    m::set("run", run);
    activity(&event);
    conversation(&event);
    // Poll controls render the derived panels once after all fresh events.
    // Completion notifications must run before a later event can change launch.
    notify_finished();
}
/// One event into the model. Returns the newest launch and, when the event began another one,
/// the launch to remember in its place.
fn advance(mut run: V, event: &V) -> (V, Option<V>) {
    let launch = m::text(event, "launch_id");
    let mut replaced = None;
    if !run.is_object() || (!launch.is_empty() && run["launch"] != launch) {
        // A Harness check comes under a launch id of its own. The launch it replaces is kept, so
        // the turn's details outlive the check; a check is never the launch to keep.
        if run.is_object() && !is_check(&run) {
            replaced = Some(run);
        }
        run = initial();
        run["launch"] = json!(launch);
        run["started"] = event["at"].clone();
    }
    let kind = m::text(event, "kind");
    let mut kinds = m::array(&run["kinds"]);
    if !kinds.iter().any(|k| k == &kind) {
        kinds.push(json!(kind));
    }
    run["kinds"] = json!(kinds);
    match kind.as_str() {
        "tool" => {
            let name = tool_name(event);
            let key = name.to_lowercase();
            let call = event["call"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("anonymous-{}", m::string(&event["id"])));
            let start = starts(event);
            let target = event["path"]
                .as_str()
                .or_else(|| event["detail"].as_str())
                .unwrap_or("");
            let mut calls = run["calls"].as_object().cloned().unwrap_or_default();
            let mut item=calls.get(&call).cloned().unwrap_or(json!({"tool":name,"key":key,"path":event["path"],"detail":event["detail"],"event_id":event["id"],"started":event["at"]}));
            item["open"] = json!(start);
            item["ok"] = json!(event["ok"]
                .as_bool()
                .unwrap_or(!m::text(event, "text").contains("failed")));
            item["event_id"] = event["id"].clone();
            calls.insert(call.clone(), item.clone());
            run["calls"] = json!(calls);
            if start {
                run["steps"] = json!(run["steps"].as_u64().unwrap_or(0) + 1);
            }
            if [
                "read",
                "edit",
                "write",
                "multiedit",
                "notebookedit",
                "delete",
            ]
            .contains(&key.as_str())
            {
                record_file(&mut run, target, &key, event);
            }
            for change in m::array(&event["changes"]) {
                record_file(&mut run, &m::text(&change, "path"), "edit", event);
            }
            if ["grep", "glob", "ls"].contains(&key.as_str()) && start {
                let mut searches = m::array(&run["searches"]);
                searches.push(
                    json!({"pattern":event["detail"],"scope":event["path"],"event_id":event["id"]}),
                );
                run["searches"] = json!(searches);
            }
            if ["bash", "shell"].contains(&key.as_str()) {
                let mut list = m::array(&run["commands"]);
                if let Some(existing) = list.iter_mut().find(|c| c["call"] == call) {
                    existing["running"] = json!(start);
                    existing["ok"] = item["ok"].clone();
                    existing["event_id"] = event["id"].clone();
                } else {
                    let command = event["detail"].as_str().unwrap_or_else(|| {
                        event["text"]
                            .as_str()
                            .unwrap_or("")
                            .split_once(": ")
                            .map(|(_, s)| s)
                            .unwrap_or("")
                    });
                    list.push(json!({"call":call,"command":command,"class":classify(command),"running":start,"ok":item["ok"],"event_id":event["id"],"at":event["at"]}));
                }
                run["commands"] = json!(list);
            }
        }
        "plan" => {
            run["plan"] = event["items"]
                .as_array()
                .or_else(|| event["plan"].as_array())
                .cloned()
                .map(V::Array)
                .unwrap_or_else(|| event["plan"].clone());
        }
        "text" => run["messages"] = json!(run["messages"].as_u64().unwrap_or(0) + 1),
        "usage" => run["usage"] = event.clone(),
        "memory" => run["memory"] = event.clone(),
        "delegation" => run["delegation"] = event.clone(),
        "status" => {
            if check_status(&m::text(event, "text")) {
                run["check"] = json!(true);
            }
            let status = event["status"]
                .as_str()
                .unwrap_or_else(|| event["text"].as_str().unwrap_or(""));
            if ["completed", "failed", "cancelled", "interrupted"].contains(&status) {
                run["finished"] = json!(true);
                run["outcome"] = json!(status);
            }
        }
        "error" => {
            run["error"] = event.clone();
            if m::text(event, "text").starts_with("Run failed (") {
                run["finished"] = json!(true);
                run["outcome"] = json!("failed");
            }
        }
        "result" => run["result"] = event.clone(),
        _ => {}
    }
    (run, replaced)
}
fn record_file(run: &mut V, path: &str, kind: &str, event: &V) {
    if path.is_empty() {
        return;
    }
    let path = clean(path).replace('\\', "/");
    let mut files = run["files"].as_object().cloned().unwrap_or_default();
    let mut f=files.get(&path).cloned().unwrap_or(json!({"path":path,"reads":0,"edits":0,"outside":m::yes(&event["outside"]),"event_id":event["id"]}));
    let field = if kind == "read" { "reads" } else { "edits" };
    f[field] = json!(f[field].as_u64().unwrap_or(0) + 1);
    f["event_id"] = event["id"].clone();
    files.insert(path, f);
    run["files"] = json!(files);
}
fn conversation(v: &V) {
    let kind = m::text(v, "kind");
    let text = clean(&m::text(v, "text"));
    let id = m::string(&v["id"]);
    if ![
        "text",
        "thinking",
        "tool",
        "status",
        "error",
        "memory",
        "delegation",
        "clash_turn",
        "clash_state",
    ]
    .contains(&kind.as_str())
    {
        return;
    }
    if kind == "text" && text.is_empty() {
        return;
    }
    let block = dom::el(
        if kind == "tool" { "details" } else { "div" },
        &format!("conversation-{kind}"),
        "",
    );
    let _ = block.set_attribute("data-event-id", &id);
    if kind == "tool" {
        dom::add(
            &block,
            &dom::el(
                "summary",
                "",
                &format!(
                    "{} · {}",
                    tool_name(v),
                    if starts(v) {
                        "Running"
                    } else if v["ok"] == false {
                        "Failed"
                    } else {
                        "Completed"
                    }
                ),
            ),
        );
        dom::add(&block, &dom::el("pre", "", &text));
    } else {
        dom::add(&block, &dom::el("p", "", &text));
    }
    if kind == "error" {
        let _ = block.set_attribute("role", "alert");
    }
    dom::append("events", &block);
}
fn native_run(run: &V) -> bool {
    !run["kinds"].as_array().into_iter().flatten().any(|kind| {
        matches!(
            kind.as_str(),
            Some("fleet_stage" | "fleet_reviewer" | "clash_turn" | "agent" | "agent_activity")
        )
    })
}
/// What the strip above the composer shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Nothing runs and no finished turn is open: the view is collapsed and inert.
    Closed,
    /// The session is queued, preparing, or runs something the strip does not describe.
    Waiting,
    /// A native launch is running: the Now line and, on request, its details.
    Now,
    /// The details of the finished turn, opened from the line under the conversation.
    After,
}
/// Fleet, Clash, Creator and System sessions keep their own progress: the strip only says they run.
fn plain(session: &V) -> bool {
    session["workflow"] == "fleet-review"
        || [
            "fleet",
            "clash",
            "creator",
            "system_run",
            "system_discovery",
        ]
        .iter()
        .any(|key| session[*key].is_object())
}
fn kinds(run: &V) -> impl Iterator<Item = &str> {
    run["kinds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(V::as_str)
}
/// The provider has written something for this launch; until then the session is only preparing.
fn cli_started(run: &V) -> bool {
    kinds(run).any(|kind| {
        matches!(
            kind,
            "session" | "text" | "tool" | "plan" | "usage" | "result" | "error"
        )
    })
}
/// The launch the model holds cannot be the one that runs now: it closed itself, the person has
/// asked for the next turn since it started, or the page has already seen the session at rest
/// with it. `idle` is the launch the page last saw at rest.
fn over(run: &V, idle: &V) -> bool {
    let launch = run["launch"].as_str().unwrap_or("");
    m::yes(&run["finished"])
        || idle.as_str() == Some(launch)
        || (!launch.is_empty() && kinds(run).any(|kind| kind == "user"))
}
/// The two lines a Harness check writes: its result and its end.
fn check_status(text: &str) -> bool {
    text == "Check finished."
        || text.strip_prefix("Check ").is_some_and(|rest| {
            rest.split_once(": ").is_some_and(|(word, _)| {
                !word.is_empty() && word.chars().all(|c| c.is_alphanumeric() || c == '_')
            })
        })
}
/// A Harness check runs under a launch id of its own and writes only its status lines and, when
/// it could not run, an error: it is not a turn. A launch the provider wrote to is never one.
fn is_check(run: &V) -> bool {
    m::yes(&run["check"])
        && !kinds(run).any(|kind| {
            matches!(
                kind,
                "session" | "text" | "tool" | "plan" | "usage" | "result"
            )
        })
}
/// The launch a finished turn's line and details describe: the newest one, or the one a Harness
/// check replaced.
fn turn<'a>(run: &'a V, before: &'a V) -> &'a V {
    if is_check(run) {
        before
    } else {
        run
    }
}
/// A launch the Run view describes: a native one, in a session that keeps no progress of its own.
fn described(session: &V, run: &V) -> bool {
    run.is_object() && native_run(run) && !plain(session)
}
/// Whether the Run view is open, and as what. `before` is the launch the newest one replaced,
/// `after` the launch whose finished details the person opened, and `idle` the launch the page
/// last saw at rest.
fn mode(session: &V, run: &V, before: &V, after: &V, idle: &V) -> Mode {
    if !m::active(session) {
        // A finished turn's details stay open only while nothing runs, and only for the launch
        // they were opened for.
        let last = turn(run, before);
        let opened = after.as_str().is_some() && after.as_str() == last["launch"].as_str();
        return if session.is_object() && described(session, last) && opened {
            Mode::After
        } else {
            Mode::Closed
        };
    }
    if session["status"] == "queued"
        || !described(session, run)
        || is_check(run)
        || over(run, idle)
        || !cli_started(run)
    {
        Mode::Waiting
    } else {
        Mode::Now
    }
}
const TABS: [&str; 3] = ["files", "plan", "commands"];
/// The launch recorded something for this tab.
fn has(run: &V, tab: &str) -> bool {
    match tab {
        "files" => run["files"].as_object().is_some_and(|map| !map.is_empty()),
        _ => run[tab].as_array().is_some_and(|list| !list.is_empty()),
    }
}
/// The tabs a finished turn can open from the line under the conversation: none while a turn
/// runs, for a session that keeps its own progress, or when the launch recorded nothing.
fn openers(session: &V, run: &V) -> Vec<&'static str> {
    if !session.is_object() || m::active(session) || !described(session, run) {
        return vec![];
    }
    TABS.into_iter().filter(|tab| has(run, tab)).collect()
}
/// The tab the panel shows: the chosen one while the launch has something for it, else Files.
fn shown_tab<'a>(run: &V, chosen: &'a str) -> &'a str {
    if TABS.contains(&chosen) && has(run, chosen) {
        chosen
    } else {
        "files"
    }
}
#[derive(Debug, PartialEq)]
struct RunClock {
    started: Option<String>,
    running: bool,
}
fn clock_state(run: &V, session: &V) -> Option<RunClock> {
    if !run.is_object() || !native_run(run) {
        return None;
    }
    Some(RunClock {
        started: run["started"].as_str().map(str::to_owned),
        running: !m::yes(&run["finished"]) && m::active(session),
    })
}
fn update_clock() {
    let clock = m::read(|state| {
        state
            .data
            .get("run")
            .and_then(|run| clock_state(run, &state.selected))
    });
    let Some(clock) = clock else {
        dom::cancel_timer("run-clock");
        return;
    };
    let since = clock
        .started
        .as_deref()
        .map(js_sys::Date::parse)
        .filter(|n| n.is_finite())
        .map(|n| ((js_sys::Date::now() - n) / 1000.).max(0.) as u64);
    dom::text(
        "run-clock",
        &since
            .map(|n| format!("{}m {:02}s", n / 60, n % 60))
            .unwrap_or_default(),
    );
    if clock.running {
        dom::later("run-clock", 1000, update_clock);
    } else {
        dom::cancel_timer("run-clock");
    }
}
fn truth(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}
fn inert(id: &str, value: bool) {
    if let Some(e) = dom::get(id) {
        if value {
            let _ = e.set_attribute("inert", "");
        } else {
            let _ = e.remove_attribute("inert");
        }
    }
}
fn focus_within(ids: &[&str]) -> bool {
    let Some(focused) = dom::document().active_element() else {
        return false;
    };
    ids.iter()
        .any(|id| dom::get(id).is_some_and(|e| e.contains(Some(&focused))))
}
/// Focus without scrolling: the view clips its content while it opens, and a scrolling focus
/// would shift what it clips.
fn focus(id: &str) {
    let Some(e) = dom::get(id) else { return };
    let options = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&options, &"preventScroll".into(), &true.into());
    if let Ok(method) = js_sys::Reflect::get(&e, &"focus".into()) {
        if let Some(method) = method.dyn_ref::<js_sys::Function>() {
            let _ = method.call1(&e, &options);
        }
    }
}
/// The Calm/Detailed switch that is on screen: a narrow window shows only the compact chip.
fn detail_switch() -> &'static str {
    let narrow = dom::window()
        .match_media("(max-width:760px)")
        .ok()
        .flatten()
        .is_some_and(|media| media.matches());
    if narrow {
        "run-compact"
    } else {
        "run-calm"
    }
}
/// Opens or collapses the view and its panel. The markup ships both collapsed and inert, and the
/// stylesheet shows the view only as `data-open="true"` and the panel only as
/// `data-detailed="true"` on the card.
fn reveal(mode: Mode, detailed: bool) {
    let strip = matches!(mode, Mode::Now | Mode::After);
    let panel = strip && detailed;
    // Keyboard focus in a part that is about to go inert would fall to <body>: it moves first,
    // to the Calm/Detailed switch while the strip stays, else to the composer.
    if !strip && focus_within(&["run-view", "run-file-card"]) {
        focus("prompt");
    } else if !panel && focus_within(&["run-panel", "run-file-card"]) {
        focus(detail_switch());
    }
    if !panel {
        dom::hidden("run-file-card", true);
    }
    dom::attr("run-view", "data-open", truth(mode != Mode::Closed));
    inert("run-view", mode == Mode::Closed);
    dom::attr("run-card", "data-detailed", truth(panel));
    inert("run-panel", !panel);
    for (id, pressed) in [
        ("run-calm", !detailed),
        ("run-detailed", detailed),
        ("run-compact", detailed),
    ] {
        dom::attr(id, "aria-pressed", truth(pressed));
    }
    dom::hidden("waiting", mode != Mode::Waiting);
    dom::hidden("run-now", !strip);
    dom::hidden("run-line2", !strip);
    dom::hidden("run-close", mode != Mode::After);
    dom::attr(
        "run-dot",
        "data-state",
        if mode == Mode::Now { "running" } else { "done" },
    );
    dom::attr("run-dot", "data-pulse", truth(mode == Mode::Now));
}
const TURN: &str = "run-turn";
/// The line under the conversation that opens a finished turn's details. The page keeps the
/// newest launch only, so there is one line, and it goes when the next turn starts.
fn turn_line(session: &V, run: &V) {
    let tabs = openers(session, run);
    let old = dom::get(TURN);
    if tabs.is_empty() {
        if let Some(old) = old {
            old.remove();
        }
        return;
    }
    let text = format!(
        "Last run · {} steps · {} messages",
        m::string(&run["steps"]),
        m::string(&run["messages"])
    );
    let signature = format!("{text}|{}", tabs.join(","));
    let Some(events) = dom::get("events") else {
        return;
    };
    let line = match old {
        Some(old) if old.get_attribute("data-signature").as_deref() == Some(&signature) => old,
        old => {
            if let Some(old) = old {
                old.remove();
            }
            let line = dom::el("article", "run-receipt", "");
            let _ = line.set_attribute("id", TURN);
            let _ = line.set_attribute("data-compact", "true");
            let _ = line.set_attribute("data-signature", &signature);
            let _ = line.set_attribute("aria-label", "Last run");
            dom::add(&line, &dom::el("span", "", &text));
            for tab in tabs {
                let open = dom::button(&format!("Show {tab}"), "run-open", json!(tab));
                open.set_class_name("run-link");
                dom::add(&line, &open);
            }
            line
        }
    };
    // It follows the log instead of living in it: the log is a live region and is cleared whole.
    if events.next_element_sibling().as_ref() != Some(&line) {
        let _ = events.after_with_node_1(&line);
    }
}
/// The away summary takes the counts' place on the strip's second line while it shows.
fn away(shown: bool) {
    dom::hidden("run-away", !shown);
    dom::hidden("run-chips", shown);
}
fn chip(text: &str) {
    let Some(chips) = dom::get("run-chips") else {
        return;
    };
    match chips.first_element_child() {
        Some(chip) => {
            if chip.text_content().as_deref() != Some(text) {
                chip.set_text_content(Some(text));
            }
        }
        None => dom::add(&chips, &dom::el("span", "run-chip", text)),
    }
}
/// The launch the newest one replaced, read only when the newest is a Harness check: nothing
/// else looks behind the newest launch.
fn earlier(run: &V) -> V {
    if is_check(run) {
        m::data("run-before")
    } else {
        V::Null
    }
}
pub fn render() {
    let run = m::data("run");
    let session = m::read(|s| s.selected.clone());
    if m::active(&session) {
        // A turn that starts takes the strip back from an earlier turn's details.
        m::take("run-after");
    } else if run.is_object() {
        // The page has seen this launch at rest: whatever runs next is not it.
        m::set("run-idle", run["launch"].clone());
    }
    let before = earlier(&run);
    let mode = mode(
        &session,
        &run,
        &before,
        &m::data("run-after"),
        &m::data("run-idle"),
    );
    reveal(mode, m::yes(&m::data("run-detailed")));
    turn_line(&session, turn(&run, &before));
    // At rest the strip and the panel hold the last turn, ready to be opened; a Harness check
    // is not one.
    let run = if m::active(&session) {
        run
    } else {
        turn(&run, &before).clone()
    };
    if !run.is_object() || !native_run(&run) {
        dom::cancel_timer("run-clock");
        return;
    }
    let calls = run["calls"].as_object().cloned().unwrap_or_default();
    let open: Vec<_> = calls
        .values()
        .filter(|c| m::yes(&c["open"]))
        .cloned()
        .collect();
    let done = m::yes(&run["finished"]) || !m::active(&session);
    let now = if done {
        format!(
            "Run {}",
            run["outcome"]
                .as_str()
                .unwrap_or_else(|| session["status"].as_str().unwrap_or("finished"))
        )
    } else if open.len() == 1 {
        format!(
            "{} · {}",
            m::text(&open[0], "tool"),
            open[0]["path"]
                .as_str()
                .or_else(|| open[0]["detail"].as_str())
                .unwrap_or("")
        )
    } else if !open.is_empty() {
        format!("{} steps running", open.len())
    } else {
        "Model’s turn".into()
    };
    if mode == Mode::Now {
        update_clock();
    } else {
        // Only a running launch has a clock: a finished turn would show the age of its first event.
        dom::cancel_timer("run-clock");
        dom::text("run-clock", "");
    }
    dom::text("run-now-text", &now);
    dom::text("run-verb", &now);
    // The counts go into the chips' own box: the second line also holds the Calm/Detailed switch,
    // Close and the away summary, which a text written over the whole line would delete.
    let counts = format!(
        "{} steps · {} messages",
        m::string(&run["steps"]),
        m::string(&run["messages"])
    );
    chip(&counts);
    dom::text(
        "run-compact",
        &format!("{} steps", m::string(&run["steps"])),
    );
    dom::attr(
        "run-compact",
        "aria-label",
        &format!("Run details. {counts}"),
    );
    let files = run["files"].as_object().cloned().unwrap_or_default();
    dom::text("run-badge-files", &files.len().to_string());
    dom::text(
        "run-files-summary",
        &format!(
            "{} files · {} edited",
            files.len(),
            files
                .values()
                .filter(|x| x["edits"].as_u64().unwrap_or(0) > 0)
                .count()
        ),
    );
    let filter = m::data("run-file-filter");
    let mut districts: BTreeMap<String, Vec<V>> = BTreeMap::new();
    for file in files.values() {
        if filter == "edited" && file["edits"].as_u64().unwrap_or(0) == 0 {
            continue;
        }
        let path = m::text(file, "path");
        let district = if m::yes(&file["outside"]) {
            "Outside project".into()
        } else {
            let p = path.split('/').collect::<Vec<_>>();
            if p.len() > 3 && p[0] == "wp-content" && ["plugins", "themes"].contains(&p[1]) {
                p[..3].join("/")
            } else if p.len() > 1 {
                p[0].into()
            } else {
                "Project root".into()
            }
        };
        districts.entry(district).or_default().push(file.clone());
    }
    let districts = districts
        .into_iter()
        .map(|(name, files)| json!({"name":name,"files":files}))
        .collect::<Vec<_>>();
    dom::keyed_render(
        "run-tree",
        &districts,
        |v| m::text(v, "name"),
        |v| {
            let d = dom::el("details", "run-tree-folder", "");
            let _ = d.set_attribute("open", "");
            let _ = d.set_attribute("data-restore-key", &m::text(v, "name"));
            let files = m::array(&v["files"]);
            dom::add(
                &d,
                &dom::el(
                    "summary",
                    "",
                    &format!("{} · {}", m::text(v, "name"), files.len()),
                ),
            );
            for file in files {
                let label = format!(
                    "{} · {} reads · {} edits",
                    m::text(&file, "path"),
                    m::string(&file["reads"]),
                    m::string(&file["edits"])
                );
                let b = dom::button(&label, "run-reveal", file["event_id"].clone());
                let _ = b.set_attribute("data-focus-key", &m::text(&file, "path"));
                dom::add(&d, &b);
            }
            d
        },
    );
    dom::keyed_render(
        "run-map-rows",
        &districts,
        |v| m::text(v, "name"),
        |v| {
            let row = dom::el("div", "run-map-row", "");
            dom::add(&row, &dom::el("strong", "", &m::text(v, "name")));
            for file in m::array(&v["files"]) {
                let b = dom::button(&m::text(&file, "path"), "run-file", file.clone());
                let _ = b.set_attribute("data-focus-key", &m::text(&file, "path"));
                dom::add(&row, &b);
            }
            row
        },
    );
    let plan = m::array(&run["plan"]);
    dom::text("run-badge-plan", &plan.len().to_string());
    dom::keyed_render(
        "run-pane-plan",
        &plan,
        |v| {
            v["id"]
                .as_str()
                .or_else(|| v["content"].as_str())
                .or_else(|| v["text"].as_str())
                .or_else(|| v["step"].as_str())
                .unwrap_or("")
                .into()
        },
        |p| {
            let label = format!(
                "{} · {}",
                m::text(p, "status"),
                p["text"]
                    .as_str()
                    .or_else(|| p["content"].as_str())
                    .or_else(|| p["step"].as_str())
                    .unwrap_or("")
            );
            dom::el("p", "run-plan-item", &label)
        },
    );
    let commands = m::array(&run["commands"]);
    dom::text("run-badge-commands", &commands.len().to_string());
    dom::keyed_render(
        "run-pane-commands",
        &commands,
        |v| m::text(v, "call"),
        |cmd| {
            let class = &cmd["class"];
            let glyph = if m::yes(&cmd["running"]) {
                "●"
            } else if !class["masked"].is_null() {
                "?"
            } else if m::yes(&cmd["ok"]) {
                "✓"
            } else {
                "✗"
            };
            let block = dom::details(
                &format!("{glyph} {}", m::text(cmd, "command")),
                &serde_json::to_string_pretty(&cmd).unwrap_or_default(),
            );
            if m::yes(&class["check"]) && m::yes(&class["runnable"]) {
                dom::add(
                    &block,
                    &dom::button(
                        "Prefill verification command",
                        "run-prefill",
                        class["bare"].clone(),
                    ),
                );
            }
            block
        },
    );
    let chosen = m::data("run-tab");
    let tab = shown_tab(&run, chosen.as_str().unwrap_or("files"));
    if chosen.as_str().is_some_and(|chosen| chosen != tab) {
        // The fallback becomes the choice: the panel does not jump to Plan when one arrives later.
        m::set("run-tab", json!(tab));
    }
    for name in TABS {
        let selected = name == tab;
        let button = format!("run-tab-{name}");
        // Plan and Commands ship hidden: each joins the tab list once the launch has one.
        dom::hidden(&button, name != "files" && !has(&run, name));
        dom::attr(&button, "aria-selected", truth(selected));
        // Every shown tab is a tab stop: the page has no arrow-key handling for the tab list.
        dom::attr(&button, "tabindex", "0");
        dom::hidden(&format!("run-pane-{name}"), !selected);
    }
}
/// Opens the finished turn's details on a tab; while a turn runs the strip is its own.
fn open(tab: &str) {
    let (session, run) = (m::read(|s| s.selected.clone()), m::data("run"));
    let before = earlier(&run);
    let last = turn(&run, &before);
    let tab = shown_tab(last, tab);
    if !openers(&session, last).contains(&tab) {
        return;
    }
    m::set("run-after", last["launch"].clone());
    m::set("run-tab", json!(tab));
    m::set("run-detailed", json!(true));
    render();
    focus(&format!("run-tab-{tab}"));
}
fn close() {
    m::take("run-after");
    render();
    focus("prompt");
}
fn detail(on: bool) {
    m::set("run-detailed", json!(on));
    render();
}
pub fn activity(event: &V) {
    let sid = m::read(|s| s.sid.clone());
    crate::activity::observe_session(&sid, event);
}
pub fn outcomes(session: &V) {
    let fleet = session["workflow"] == "fleet-review";
    let result = &session["fleet_result"];
    dom::hidden("fleet-progress", !fleet);
    dom::hidden("fleet-outcome", !fleet || result.is_null());
    if fleet {
        dom::text(
            "fleet-stage",
            &format!(
                "{} · {}",
                m::text(session, "status"),
                m::text(result, "stage")
            ),
        );
        dom::text(
            "fleet-result-summary",
            &format!(
                "{} findings · reported cost {}",
                m::array(&result["findings"]).len(),
                m::string(&result["cost_usd"])
            ),
        );
        dom::rows("fleet-findings", &m::array(&result["findings"]));
        dom::text(
            "fleet-report-text",
            result["report"]
                .as_str()
                .or_else(|| result["preview"].as_str())
                .unwrap_or(""),
        );
        dom::hidden(
            "fleet-decision-actions",
            session["status"] != "awaiting_approval",
        );
        dom::hidden(
            "fleet-resume-actions",
            !["interrupted", "failed", "cancelled"]
                .contains(&session["status"].as_str().unwrap_or("")),
        );
        dom::hidden("fleet-download", session["status"] != "completed");
        dom::attr(
            "fleet-download",
            "href",
            &format!("/api/sessions/{}/report", dom::enc(&m::text(session, "id"))),
        );
    }
    let clash = session["clash"].is_object();
    dom::hidden("clash-outcome", !clash || session["clash_result"].is_null());
    if clash {
        let r = &session["clash_result"];
        dom::text("clash-outcome-summary", &m::summary(r));
        dom::rows("clash-items", &m::array(&r["findings"]));
        dom::rows("clash-turn-list", &m::array(&r["turns"]));
        dom::text("clash-report-text", &m::text(r, "report"));
    }
    if session["merge"].is_object() {
        dom::hidden("merge-context", false);
        dom::text(
            "merge-context-title",
            &format!(
                "Merged from {} chats",
                m::array(&session["merge"]["sources"]).len()
            ),
        );
        dom::rows(
            "merge-context-sources",
            &m::array(&session["merge"]["sources"]),
        );
    } else {
        dom::hidden("merge-context", m::data("merge-draft").is_null());
    }
}
pub async fn event(kind: &str, id: &str, action: &str, payload: &V, _value: &str) -> bool {
    if kind != "click" {
        return false;
    }
    match action {
        "run-calm" => detail(false),
        "run-detailed" => detail(true),
        "run-compact" | "run-now" => detail(!m::yes(&m::data("run-detailed"))),
        "run-close" => close(),
        "run-open" => open(payload.as_str().unwrap_or("files")),
        "run-tab-files" | "run-tab-plan" | "run-tab-commands" => {
            m::set("run-tab", json!(action.trim_start_matches("run-tab-")));
            // The file card belongs to the Files tab.
            dom::hidden("run-file-card", true);
            render();
        }
        "run-view-map" | "run-view-list" => {
            let map = action == "run-view-map";
            dom::hidden("run-map", !map);
            dom::hidden("run-tree", map);
            m::set("run-files-view", json!(if map { "map" } else { "list" }));
        }
        "run-prefill" => {
            dom::set_value("check-command", payload.as_str().unwrap_or(""));
            session::open_view("checks", true).await;
            dom::focus("check-command");
        }
        "run-away-close" => {
            away(false);
            focus("run-now");
        }
        "run-away-jump" => {
            if let Some(e) =
                dom::get("conversation").and_then(|v| v.dyn_into::<web_sys::HtmlElement>().ok())
            {
                e.set_scroll_top(e.scroll_height());
            }
            away(false);
            focus("run-now");
        }
        "run-reveal" => {
            let selector = format!("[data-event-id=\"{}\"]", m::string(payload));
            if let Ok(Some(e)) = dom::document().query_selector(&selector) {
                e.scroll_into_view();
            }
        }
        "run-file" => {
            dom::clear("run-file-card");
            dom::append(
                "run-file-card",
                &dom::details(
                    &m::text(payload, "path"),
                    &serde_json::to_string_pretty(payload).unwrap_or_default(),
                ),
            );
            dom::hidden("run-file-card", false);
        }
        "run-notify" => {
            if let Ok(p) = web_sys::Notification::request_permission() {
                let _ = wasm_bindgen_futures::JsFuture::from(p).await;
            }
        }
        _ => {
            if id.starts_with("run-filter-") {
                m::set(
                    "run-file-filter",
                    json!(id.trim_start_matches("run-filter-")),
                );
                render();
            } else {
                return false;
            }
        }
    }
    true
}

fn snapshot() -> V {
    let run = m::data("run");
    let edits = run["files"]
        .as_object()
        .map(|m| m.values().filter_map(|v| v["edits"].as_u64()).sum::<u64>())
        .unwrap_or(0);
    let files = run["files"]
        .as_object()
        .map(|m| {
            m.values()
                .filter(|v| v["edits"].as_u64().unwrap_or(0) > 0)
                .map(|v| m::text(v, "path"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let failed = m::array(&run["commands"])
        .iter()
        .filter(|v| v["class"]["check"] == true && v["running"] != true && v["ok"] == false)
        .count();
    json!({"sid":m::read(|s|s.sid.clone()),"launch":run["launch"],"steps":run["steps"],"edits":edits,"files":files,"messages":run["messages"],"failed":failed,"plan":run["plan"],"finished":run["finished"],"outcome":run["outcome"]})
}
pub fn visibility() {
    if dom::document().hidden() {
        if m::read(|s| m::active(&s.selected)) {
            m::set("run-away-snapshot", snapshot());
            m::set("run-hidden-at", json!(js_sys::Date::now()));
        }
        return;
    }
    let before = m::take("run-away-snapshot");
    let hidden = m::take("run-hidden-at").as_f64();
    if !before.is_object() {
        return;
    }
    let after = snapshot();
    if before["sid"] != after["sid"] || before["launch"] != after["launch"] {
        return;
    }
    let steps = after["steps"]
        .as_u64()
        .unwrap_or(0)
        .saturating_sub(before["steps"].as_u64().unwrap_or(0));
    let edits = after["edits"]
        .as_u64()
        .unwrap_or(0)
        .saturating_sub(before["edits"].as_u64().unwrap_or(0));
    let messages = after["messages"]
        .as_u64()
        .unwrap_or(0)
        .saturating_sub(before["messages"].as_u64().unwrap_or(0));
    let failed = after["failed"]
        .as_u64()
        .unwrap_or(0)
        .saturating_sub(before["failed"].as_u64().unwrap_or(0));
    if steps == 0 && edits == 0 && messages == 0 && after["finished"] != true {
        return;
    }
    let minutes = hidden
        .map(|at| ((js_sys::Date::now() - at) / 60000.).floor() as u64)
        .unwrap_or(0);
    let text=format!("While you were away{}: {steps} steps · {edits} edits · {messages} messages · {failed} failed checks{}",if minutes>0{format!(" ({minutes} min)")}else{String::new()},if after["finished"]==true{format!(" · run {}",m::text(&after,"outcome"))}else{String::new()});
    dom::text("run-away-text", &text);
    away(true);
    dom::text("run-announcer", &text);
}
fn notify_finished() {
    if !dom::document().hidden()
        || web_sys::Notification::permission() != web_sys::NotificationPermission::Granted
    {
        return;
    }
    let Some(hidden) = m::data("run-hidden-at").as_f64() else {
        return;
    };
    if js_sys::Date::now() - hidden < 20000. {
        return;
    }
    let run = m::data("run");
    let before = m::data("run-away-snapshot");
    let sid = m::read(|s| s.sid.clone());
    if before["sid"] != sid || before["launch"] != run["launch"] || !m::yes(&run["finished"]) {
        return;
    }
    let key = format!("{}:{}", sid, m::string(&run["launch"]));
    if m::data("run-notified") == key {
        return;
    }
    m::set("run-notified", json!(key));
    let title = if run["outcome"] == "completed" {
        "Harness run finished"
    } else {
        "Harness run needs attention"
    };
    let body = format!(
        "{} · {} steps",
        m::text(&run, "outcome"),
        m::string(&run["steps"])
    );
    let options = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&options, &"body".into(), &body.into());
    let _ = js_sys::Reflect::set(
        &options,
        &"tag".into(),
        &format!("harness-run-{sid}").into(),
    );
    let _ = js_sys::Reflect::set(&options, &"silent".into(), &true.into());
    let options: web_sys::NotificationOptions = options.unchecked_into();
    if let Ok(note) = web_sys::Notification::new_with_options(title, &options) {
        let n = note.clone();
        let cb = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            let _ = dom::window().focus();
            n.close();
        });
        note.set_onclick(Some(cb.as_ref().unchecked_ref()));
        cb.forget();
    }
}

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
}
