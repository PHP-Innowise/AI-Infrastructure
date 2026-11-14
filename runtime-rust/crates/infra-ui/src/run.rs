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
            let start = starts(event);
            let call = match event["call"].as_str() {
                Some(call) => call.to_owned(),
                None => paired(&mut run, &key, start, &m::string(&event["id"])),
            };
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
            if start {
                run["steps"] = json!(run["steps"].as_u64().unwrap_or(0) + 1);
            }
            touch(&mut run, &mut item, &named(&key, target, event), event);
            if !start {
                settle(&mut run, &mut item, event);
            }
            calls.insert(call.clone(), item.clone());
            run["calls"] = json!(calls);
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
/// The call an event without a call id belongs to. Such events are plain labels: a start waits
/// under its tool's name, and the next completion of that tool closes the oldest one waiting.
fn paired(run: &mut V, key: &str, start: bool, event_id: &str) -> String {
    let mut waiting = run["uncalled"].as_object().cloned().unwrap_or_default();
    let mut queue = m::array(&waiting.get(key).cloned().unwrap_or(V::Null));
    let call = if start || queue.is_empty() {
        format!("anonymous-{event_id}")
    } else {
        m::string(&queue.remove(0))
    };
    if start {
        queue.push(json!(call));
    }
    waiting.insert(key.into(), json!(queue));
    run["uncalled"] = V::Object(waiting);
    call
}
/// A path as the page keys it: no bidirectional controls, forward slashes.
fn tidy(path: &str) -> String {
    clean(path).replace('\\', "/")
}
/// The files a tool event names and what the call does to each: the files of a change, or the
/// call's own target.
fn named(key: &str, target: &str, event: &V) -> Vec<(String, &'static str, bool)> {
    let changes = m::array(&event["changes"]);
    if !changes.is_empty() {
        return changes
            .iter()
            .map(|change| {
                let how = match change["kind"].as_str() {
                    Some("add") => "add",
                    Some("delete") => "delete",
                    _ => "edit",
                };
                (
                    tidy(&m::text(change, "path")),
                    how,
                    m::yes(&change["outside"]),
                )
            })
            .filter(|(path, _, _)| !path.is_empty())
            .collect();
    }
    let how = match key {
        "read" => "read",
        "delete" => "delete",
        "edit" | "write" | "multiedit" | "notebookedit" => "edit",
        _ => return vec![],
    };
    let path = tidy(target);
    if path.is_empty() {
        vec![]
    } else {
        vec![(path, how, m::yes(&event["outside"]))]
    }
}
/// A call reaches a file once: its start and its result name the same file, and only the first
/// of them opens it.
fn touch(run: &mut V, call: &mut V, named: &[(String, &'static str, bool)], event: &V) {
    if named.is_empty() {
        return;
    }
    let mut touched = call["touched"].as_object().cloned().unwrap_or_default();
    let mut files = run["files"].as_object().cloned().unwrap_or_default();
    let step = run["steps"].clone();
    let by = if event["parent"].as_str().is_some_and(|id| !id.is_empty()) {
        "helper"
    } else {
        "main"
    };
    for (path, how, outside) in named {
        if touched.contains_key(path) {
            continue;
        }
        touched.insert(path.clone(), json!(how));
        if !files.contains_key(path) {
            let fresh = json!({"path":path,"reads":0,"edits":0,"outside":outside,"order":files.len(),"first_step":step});
            files.insert(path.clone(), fresh);
        }
        let Some(file) = files.get_mut(path) else {
            continue;
        };
        if *how == "read" {
            file["reads"] = json!(file["reads"].as_u64().unwrap_or(0) + 1);
        }
        file[by] = json!(true);
        file["last_step"] = step.clone();
        file["event_id"] = event["id"].clone();
    }
    call["touched"] = V::Object(touched);
    run["files"] = V::Object(files);
}
/// The call's result reaches the files it touched, once: an edit counts when the tool reports it
/// done, and a failure marks the file.
fn settle(run: &mut V, call: &mut V, event: &V) {
    if m::yes(&call["settled"]) {
        return;
    }
    call["settled"] = json!(true);
    let Some(touched) = call["touched"].as_object() else {
        return;
    };
    let mut files = run["files"].as_object().cloned().unwrap_or_default();
    let ok = m::yes(&call["ok"]);
    for (path, how) in touched {
        let Some(file) = files.get_mut(path) else {
            continue;
        };
        if !ok {
            // A command that was never run reported nothing about the file.
            if event["outcome"] != "not_run" {
                file["failed"] = json!(true);
            }
            continue;
        }
        match how.as_str() {
            Some("read") => continue,
            Some("add") => file["created"] = json!(true),
            Some("delete") => file["deleted"] = json!(true),
            _ => {}
        }
        file["edits"] = json!(file["edits"].as_u64().unwrap_or(0) + 1);
        file["event_id"] = event["id"].clone();
    }
    run["files"] = V::Object(files);
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
fn count(n: u64, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}
fn counts(run: &V) -> String {
    format!(
        "{} · {}",
        count(run["steps"].as_u64().unwrap_or(0), "step"),
        count(run["messages"].as_u64().unwrap_or(0), "message")
    )
}
fn number(file: &V, key: &str) -> u64 {
    file[key].as_u64().unwrap_or(0)
}
/// The launch's files in the order they were first touched.
fn files_of(run: &V) -> Vec<V> {
    let mut files: Vec<V> = run["files"]
        .as_object()
        .map(|map| map.values().cloned().collect())
        .unwrap_or_default();
    files.sort_by_key(|file| number(file, "order"));
    files
}
const OUTSIDE: &str = ":outside";
/// Map layout: the top folder is a district (a WordPress plugin or theme is one), and up to
/// three more folders make a block.
fn place_of(file: &V) -> (String, String) {
    if m::yes(&file["outside"]) {
        return (OUTSIDE.into(), String::new());
    }
    let path = m::text(file, "path");
    let mut folder: Vec<&str> = path.split('/').collect();
    folder.pop();
    if folder.is_empty() {
        return (String::new(), String::new());
    }
    let wordpress =
        folder.len() > 2 && folder[0] == "wp-content" && ["plugins", "themes"].contains(&folder[1]);
    let depth = if wordpress { 3 } else { 1 };
    let rest = &folder[depth..];
    let mut block = rest.iter().take(3).copied().collect::<Vec<_>>().join("/");
    if rest.len() > 3 {
        block.push_str("/…");
    }
    (folder[..depth].join("/"), block)
}
fn district_label(name: &str) -> String {
    match name {
        "" => "./".into(),
        OUTSIDE => "Outside the project".into(),
        _ => format!("{name}/"),
    }
}
/// The patterns of the searches whose scope holds the file; a search without a scope looked
/// everywhere in the project.
fn searched_in(run: &V, file: &V) -> Vec<String> {
    if m::yes(&file["outside"]) {
        return vec![];
    }
    let path = m::text(file, "path");
    m::array(&run["searches"])
        .iter()
        .filter(|search| {
            let scope = m::text(search, "scope");
            let scope = scope.trim_end_matches('/');
            scope.is_empty()
                || scope == "."
                || path == scope
                || path.starts_with(&format!("{scope}/"))
        })
        .map(|search| m::text(search, "pattern"))
        .filter(|pattern| !pattern.is_empty())
        .collect()
}
const FILTERS: [(&str, &str); 5] = [
    ("all", "All"),
    ("edited", "Edited"),
    ("searched", "Searched"),
    ("failed", "Failed"),
    ("outside", "Outside"),
];
fn matches(run: &V, file: &V, filter: &str) -> bool {
    match filter {
        "edited" => number(file, "edits") > 0,
        "searched" => !searched_in(run, file).is_empty(),
        "failed" => m::yes(&file["failed"]),
        "outside" => m::yes(&file["outside"]),
        _ => true,
    }
}
/// The filters worth offering: All, and each kind that has a file. A choice whose kind has no
/// file any more reads as All.
fn filters(run: &V, chosen: &str) -> Vec<V> {
    let files = files_of(run);
    let offered: Vec<(&str, String)> = FILTERS
        .iter()
        .filter_map(|(id, label)| {
            let n = files.iter().filter(|file| matches(run, file, id)).count();
            match (*id, n) {
                ("all", _) => Some((*id, label.to_string())),
                (_, 0) => None,
                _ => Some((*id, format!("{label} {n}"))),
            }
        })
        .collect();
    let chosen = if offered.iter().any(|(id, _)| *id == chosen) {
        chosen
    } else {
        "all"
    };
    offered
        .into_iter()
        .map(|(id, label)| json!({"id":id,"label":label,"pressed":id == chosen}))
        .collect()
}
/// What a chip says after its path to someone who cannot see its marks.
fn chip_words(file: &V) -> Vec<String> {
    let (reads, edits) = (number(file, "reads"), number(file, "edits"));
    let mut words = vec![];
    match reads {
        0 => {}
        1 => words.push("opened once".into()),
        _ => words.push(format!("opened {reads} times")),
    }
    if m::yes(&file["created"]) {
        words.push("created".into());
    }
    if m::yes(&file["deleted"]) {
        words.push("deleted".into());
    }
    match edits {
        0 => {}
        1 => words.push("edit reported ok".into()),
        _ => words.push(format!("edit reported ok {edits} times")),
    }
    if m::yes(&file["failed"]) {
        words.push("a tool reported an error".into());
    }
    if m::yes(&file["helper"]) {
        words.push("by a helper agent".into());
    }
    if m::yes(&file["outside"]) {
        words.push("outside the project, name only".into());
    }
    words
}
/// The facts on a file's card.
fn card_facts(run: &V, file: &V) -> Vec<String> {
    let (reads, edits) = (number(file, "reads"), number(file, "edits"));
    let mut facts = vec![];
    if reads > 0 {
        facts.push(format!("Opened ×{reads}"));
    }
    let searched = searched_in(run, file);
    if !searched.is_empty() {
        let last = &searched[searched.len().saturating_sub(2)..];
        facts.push(format!("In a folder searched for {}", last.join(", ")));
    }
    if m::yes(&file["created"]) {
        facts.push("Added (file_change)".into());
    }
    if m::yes(&file["deleted"]) {
        facts.push("Deleted (file_change)".into());
    }
    match edits {
        0 => {}
        1 => facts.push("Edit reported ok".into()),
        _ => facts.push(format!("Edit reported ok ×{edits}")),
    }
    if m::yes(&file["failed"]) {
        facts.push("A tool reported an error".into());
    }
    if m::yes(&file["helper"]) {
        facts.push("by a helper agent".into());
    }
    facts
}
/// A district's blocks in the order they appeared: the folders under it and their chips.
type Blocks = Vec<(String, Vec<V>)>;
/// The map as data: districts, their blocks and the chips in them, in the order the files were
/// first touched. The filter dims the chips it leaves out, and while the launch runs a ring
/// marks the files of the calls that are open.
fn map(run: &V, filter: &str, running: bool) -> Vec<V> {
    let mut open = std::collections::BTreeSet::new();
    if running {
        for call in run["calls"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(_, call)| call)
        {
            if m::yes(&call["open"]) {
                open.extend(
                    call["touched"]
                        .as_object()
                        .into_iter()
                        .flatten()
                        .map(|(path, _)| path.clone()),
                );
            }
        }
    }
    let mut districts: Vec<(String, Blocks)> = vec![];
    for file in files_of(run) {
        let (district, block) = place_of(&file);
        let path = m::text(&file, "path");
        let now = open.contains(&path);
        let reads = number(&file, "reads");
        let mut label = vec![path.clone()];
        label.extend(chip_words(&file));
        if now {
            label.push("in use now".into());
        }
        let mark = format!(
            "{}{}",
            if m::yes(&file["created"]) { "+" } else { "" },
            if m::yes(&file["failed"]) { "!" } else { "" }
        );
        let chip = json!({
            "path":path,
            "name":path.rsplit('/').next().unwrap_or(&path),
            "mark":mark,
            "times":if reads > 1 { format!("×{reads}") } else { String::new() },
            "edited":number(&file, "edits") > 0,
            "failed":m::yes(&file["failed"]),
            "created":m::yes(&file["created"]),
            "deleted":m::yes(&file["deleted"]),
            "helper":m::yes(&file["helper"]) && !m::yes(&file["main"]),
            "dim":!matches(run, &file, filter),
            "now":now,
            "label":label.join(", "),
        });
        let at = match districts.iter().position(|(name, _)| *name == district) {
            Some(at) => at,
            None => {
                districts.push((district, vec![]));
                districts.len() - 1
            }
        };
        let blocks = &mut districts[at].1;
        match blocks.iter().position(|(name, _)| *name == block) {
            Some(at) => blocks[at].1.push(chip),
            None => blocks.push((block, vec![chip])),
        }
    }
    districts
        .into_iter()
        .map(|(name, blocks)| {
            let blocks: Vec<V> = blocks
                .into_iter()
                .map(|(block, files)| {
                    let folder = match (name.as_str(), block.as_str()) {
                        (OUTSIDE, _) => "Outside the project".to_owned(),
                        ("", _) => "Project root".to_owned(),
                        (district, "") => district.to_owned(),
                        (district, block) => format!("{district}/{block}"),
                    };
                    let edited = files.iter().filter(|chip| m::yes(&chip["edited"])).count();
                    let mut label = format!("{folder}, {}", count(files.len() as u64, "file"));
                    if edited > 0 {
                        label.push_str(&format!(", {edited} edited"));
                    }
                    json!({"name":block,"label":label,"files":files})
                })
                .collect();
            json!({"name":name,"label":district_label(&name),"blocks":blocks})
        })
        .collect()
}
/// Where the file card goes: under its chip, above it when it would leave the window, and never
/// past the window's edges. The chip is (left, top, bottom); sizes are (width, height).
fn place(chip: (f64, f64, f64), card: (f64, f64), window: (f64, f64)) -> (f64, f64) {
    let (left, top, bottom) = chip;
    let x = (left - 8.).min(window.0 - card.0 - 8.).max(8.);
    let y = if bottom + 8. + card.1 > window.1 - 8. {
        (top - card.1 - 8.).max(8.)
    } else {
        bottom + 8.
    };
    (x, y)
}
/// The draft with `text` in place of its selection, a space apart from what surrounds it, and
/// the caret after the text. Offsets are UTF-16 units, as the browser reports a selection.
fn spliced(value: &str, start: u32, end: u32, text: &str) -> (String, u32) {
    let at = |units: u32| {
        let mut seen = 0;
        for (index, c) in value.char_indices() {
            if seen >= units {
                return index;
            }
            seen += c.len_utf16() as u32;
        }
        value.len()
    };
    let (from, to) = (at(start), at(end.max(start)));
    let (before, after) = (&value[..from], &value[to..]);
    let lead = if before.is_empty() || before.ends_with(char::is_whitespace) {
        ""
    } else {
        " "
    };
    let trail = if after.is_empty() || after.starts_with(char::is_whitespace) {
        ""
    } else {
        " "
    };
    let caret = before.encode_utf16().count() + lead.len() + text.encode_utf16().count();
    (format!("{before}{lead}{text}{trail}{after}"), caret as u32)
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
    if let Some(node) = dom::get(id) {
        focus_node(&node);
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
        hide_card(false);
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
    let text = format!("Last run · {}", counts(run));
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
fn bdi(class: &str, text: &str) -> web_sys::Element {
    let node = dom::el("bdi", class, text);
    let _ = node.set_attribute("dir", "ltr");
    node
}
/// A part of a chip that its label already says.
fn hushed(node: web_sys::Element) -> web_sys::Element {
    let _ = node.set_attribute("aria-hidden", "true");
    node
}
fn chip_node(chip: &V) -> web_sys::Element {
    let path = m::text(chip, "path");
    let node = dom::el("button", "run-file", "");
    for (key, value) in [
        ("type", "button".to_owned()),
        ("data-action", "run-file".to_owned()),
        ("data-payload", json!(path).to_string()),
        ("data-focus-key", path.clone()),
        ("title", path.clone()),
        ("aria-label", m::text(chip, "label")),
    ] {
        let _ = node.set_attribute(key, &value);
    }
    for flag in ["edited", "failed", "created", "deleted", "helper", "dim"] {
        let _ = node.set_attribute(&format!("data-{flag}"), truth(m::yes(&chip[flag])));
    }
    if m::yes(&chip["now"]) {
        let _ = node.set_attribute("data-now", "static");
    }
    dom::add(
        &node,
        &hushed(dom::el("span", "run-file-mark", &m::text(chip, "mark"))),
    );
    dom::add(&node, &hushed(bdi("run-file-name", &m::text(chip, "name"))));
    dom::add(
        &node,
        &hushed(dom::el("span", "run-file-count", &m::text(chip, "times"))),
    );
    node
}
fn district_node(district: &V) -> web_sys::Element {
    let wrap = dom::el("div", "run-district", "");
    let label = dom::el("div", "run-district-label", "");
    dom::add(&label, &bdi("", &m::text(district, "label")));
    dom::add(&wrap, &label);
    let blocks = dom::el("div", "run-blocks", "");
    for block in m::array(&district["blocks"]) {
        let node = dom::el("div", "run-block", "");
        let _ = node.set_attribute("role", "group");
        let _ = node.set_attribute("aria-label", &m::text(&block, "label"));
        let name = m::text(&block, "name");
        if !name.is_empty() {
            let head = dom::el("div", "run-block-head", "");
            dom::add(&head, &bdi("run-block-name", &name));
            dom::add(&node, &head);
        }
        let files = dom::el("div", "run-files", "");
        for chip in m::array(&block["files"]) {
            dom::add(&files, &chip_node(&chip));
        }
        dom::add(&node, &files);
        dom::add(&blocks, &node);
    }
    dom::add(&wrap, &blocks);
    wrap
}
/// The filter buttons are kept and relabelled, not rebuilt: the one just pressed keeps the focus.
fn filter_buttons(items: &[V]) {
    let Some(container) = dom::get("run-files-filters") else {
        return;
    };
    let ids: Vec<String> = items
        .iter()
        .map(|item| format!("run-filter-{}", m::text(item, "id")))
        .collect();
    for child in dom::children(&container) {
        if !ids.contains(&child.id()) {
            child.remove();
        }
    }
    for (index, item) in items.iter().enumerate() {
        let node = dom::get(&ids[index]).unwrap_or_else(|| {
            let node = dom::el("button", "run-filter", "");
            let _ = node.set_attribute("type", "button");
            let _ = node.set_attribute("id", &ids[index]);
            node
        });
        let label = m::text(item, "label");
        if node.text_content().as_deref() != Some(label.as_str()) {
            node.set_text_content(Some(&label));
        }
        let _ = node.set_attribute("aria-pressed", truth(m::yes(&item["pressed"])));
        let current = container.children().item(index as u32);
        if current.as_ref() != Some(&node) {
            let _ = container.insert_before(&node, current.as_ref().map(|e| e.as_ref()));
        }
    }
}
const CARD: &str = "run-file-card";
/// The file whose card is open.
fn card_path() -> Option<String> {
    m::data("run-card").as_str().map(str::to_owned)
}
fn chip_of(path: &str) -> Option<web_sys::Element> {
    dom::all("#run-map-main .run-file")
        .into_iter()
        .find(|chip| chip.get_attribute("data-focus-key").as_deref() == Some(path))
}
fn focus_node(node: &web_sys::Element) {
    let options = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&options, &"preventScroll".into(), &true.into());
    if let Ok(method) = js_sys::Reflect::get(node, &"focus".into()) {
        if let Some(method) = method.dyn_ref::<js_sys::Function>() {
            let _ = method.call1(node, &options);
        }
    }
}
/// Writes the card for a file. An unchanged card is left alone, and a rewritten one gives the
/// focus back to the action that had it.
fn fill_card(run: &V, file: &V) {
    let Some(card) = dom::get(CARD) else {
        return;
    };
    let path = m::text(file, "path");
    let facts = card_facts(run, file).join(" · ");
    let signature = format!("{file}|{facts}");
    if card.get_attribute("data-signature").as_deref() == Some(signature.as_str()) {
        return;
    }
    let focused = dom::document()
        .active_element()
        .filter(|node| card.contains(Some(node)))
        .and_then(|node| node.get_attribute("data-action"));
    card.set_text_content(None);
    let _ = card.set_attribute("data-signature", &signature);
    let _ = card.set_attribute("aria-label", &format!("File details: {path}"));
    let line = dom::el("p", "run-card-path", "");
    let shown = if m::yes(&file["outside"]) {
        format!("{path} (outside the project, name only)")
    } else {
        path.clone()
    };
    dom::add(&line, &bdi("", &shown));
    dom::add(&card, &line);
    dom::add(&card, &dom::el("p", "run-card-facts", &facts));
    dom::add(
        &card,
        &dom::el(
            "p",
            "run-card-meta",
            &format!(
                "First step {} · last step {}",
                m::string(&file["first_step"]),
                m::string(&file["last_step"])
            ),
        ),
    );
    let actions = dom::el("div", "run-card-actions", "");
    for (label, action, payload) in [
        (
            "Show in conversation",
            "run-reveal",
            file["event_id"].clone(),
        ),
        ("Add path to message", "run-add-path", json!(path)),
        ("Copy path", "run-copy-path", json!(path)),
    ] {
        let button = dom::button(label, action, payload);
        button.set_class_name("button compact");
        dom::add(&actions, &button);
    }
    dom::add(&card, &actions);
    if let Some(action) = focused {
        let selector = format!("#{CARD} [data-action=\"{action}\"]");
        if let Ok(Some(node)) = dom::document().query_selector(&selector) {
            focus_node(&node);
        }
    }
}
fn place_card(chip: &web_sys::Element) {
    let Some(card) = dom::get(CARD).and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok()) else {
        return;
    };
    let rect = chip.get_bounding_client_rect();
    let window = dom::window();
    let side = |value: Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>| {
        value.ok().and_then(|value| value.as_f64()).unwrap_or(0.)
    };
    let (x, y) = place(
        (rect.left(), rect.top(), rect.bottom()),
        (card.offset_width() as f64, card.offset_height() as f64),
        (side(window.inner_width()), side(window.inner_height())),
    );
    let _ = card.style().set_property("left", &format!("{x}px"));
    let _ = card.style().set_property("top", &format!("{y}px"));
}
/// Opens the card of a file beside its chip; the first of its actions takes the focus.
fn show_card(run: &V, path: &str) {
    let Some(file) = run["files"].get(path) else {
        return;
    };
    m::set("run-card", json!(path));
    if let Some(card) = dom::get(CARD) {
        let _ = card.remove_attribute("data-signature");
    }
    fill_card(run, file);
    dom::hidden(CARD, false);
    if let Some(chip) = chip_of(path) {
        place_card(&chip);
    }
    let selector = format!("#{CARD} button");
    if let Ok(Some(first)) = dom::document().query_selector(&selector) {
        focus_node(&first);
    }
}
/// Puts the card away; `restore` gives the focus back to the chip it belongs to.
fn hide_card(restore: bool) {
    let path = m::take("run-card");
    dom::hidden(CARD, true);
    if restore {
        if let Some(chip) = path.as_str().and_then(chip_of) {
            focus_node(&chip);
        }
    }
}
/// The launch the strip and the panel show: the one that runs, or at rest the last turn's.
fn shown() -> V {
    let (session, run) = (m::read(|s| s.selected.clone()), m::data("run"));
    if m::active(&session) {
        run
    } else {
        let before = earlier(&run);
        turn(&run, &before).clone()
    }
}
/// The Files tab: the summary, the filters, the map of touched files and the same set as a list.
fn files_pane(run: &V, running: bool) {
    let files = files_of(run);
    let edited = files
        .iter()
        .filter(|file| number(file, "edits") > 0)
        .count();
    let failed = files.iter().filter(|file| m::yes(&file["failed"])).count();
    let mut summary = format!("{} · {edited} edited", count(files.len() as u64, "file"));
    if failed > 0 {
        summary.push_str(&format!(" · {failed} failed"));
    }
    dom::text("run-badge-files", &files.len().to_string());
    dom::text("run-files-summary", &summary);
    let list = m::data("run-files-view") == "list";
    let offered = filters(run, m::data("run-file-filter").as_str().unwrap_or("all"));
    let filter = offered
        .iter()
        .find(|item| m::yes(&item["pressed"]))
        .map(|item| m::text(item, "id"))
        .unwrap_or_else(|| "all".into());
    dom::attr("run-view-map", "aria-pressed", truth(!list));
    dom::attr("run-view-list", "aria-pressed", truth(list));
    dom::hidden("run-map", list);
    dom::hidden("run-tree", !list);
    dom::hidden("run-files-empty", !files.is_empty());
    // The filters dim chips and the legend explains them: both belong to the map.
    dom::hidden("run-files-filters", list || files.is_empty());
    dom::hidden("run-legend", list || files.is_empty());
    filter_buttons(&offered);
    dom::keyed_render(
        "run-map-main",
        &map(run, &filter, running),
        |district| m::text(district, "name"),
        district_node,
    );
    let mut districts: BTreeMap<String, Vec<V>> = BTreeMap::new();
    for file in &files {
        let (district, _) = place_of(file);
        let name = match district.as_str() {
            OUTSIDE => "Outside project".to_owned(),
            "" => "Project root".to_owned(),
            _ => district,
        };
        districts.entry(name).or_default().push(file.clone());
    }
    let districts = districts
        .into_iter()
        .map(|(name, mut files)| {
            files.sort_by_key(|file| m::text(file, "path"));
            json!({"name":name,"files":files})
        })
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
                    "{} · {} · {}",
                    m::text(&file, "path"),
                    count(number(&file, "reads"), "read"),
                    count(number(&file, "edits"), "edit")
                );
                let b = dom::button(&label, "run-reveal", file["event_id"].clone());
                let _ = b.set_attribute("data-focus-key", &m::text(&file, "path"));
                dom::add(&d, &b);
            }
            d
        },
    );
    // An open card follows its file; a file that is gone, or a map that is, takes it along.
    if let Some(path) = card_path() {
        match run["files"].get(&path) {
            Some(file) if !list => fill_card(run, file),
            _ => hide_card(false),
        }
    }
}
/// Puts a path into the message being written, at the caret; Send stays the person's.
fn insert_draft(text: &str) {
    let Some(prompt) =
        dom::get("prompt").and_then(|e| e.dyn_into::<web_sys::HtmlTextAreaElement>().ok())
    else {
        return;
    };
    if prompt.disabled() {
        return;
    }
    let value = prompt.value();
    let units = value.encode_utf16().count() as u32;
    let start = prompt
        .selection_start()
        .ok()
        .flatten()
        .unwrap_or(units)
        .min(units);
    let end = prompt
        .selection_end()
        .ok()
        .flatten()
        .unwrap_or(start)
        .clamp(start, units);
    let (draft, caret) = spliced(&value, start, end, text);
    prompt.set_value(&draft);
    let _ = prompt.set_selection_range(caret, caret);
    focus("prompt");
    session::controls();
    session::save_preferences();
}
/// Escape puts away the nearest thing the person opened: the file card, then the details.
pub fn escape() {
    if card_path().is_some() {
        hide_card(true);
    } else if m::yes(&m::data("run-detailed")) && focus_within(&["run-view"]) {
        detail(false);
        focus(detail_switch());
    }
}
/// A press anywhere but the file card and the chips puts the card away.
pub fn pointer_down(target: &web_sys::Element) {
    if card_path().is_some()
        && target
            .closest("#run-file-card, .run-file")
            .ok()
            .flatten()
            .is_none()
    {
        hide_card(false);
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
    let counts = counts(&run);
    chip(&counts);
    dom::text(
        "run-compact",
        &count(run["steps"].as_u64().unwrap_or(0), "step"),
    );
    dom::attr(
        "run-compact",
        "aria-label",
        &format!("Run details. {counts}"),
    );
    files_pane(&run, mode == Mode::Now);
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
            hide_card(false);
            render();
        }
        "run-view-map" | "run-view-list" => {
            let view = if action == "run-view-map" {
                "map"
            } else {
                "list"
            };
            m::set("run-files-view", json!(view));
            hide_card(false);
            render();
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
            hide_card(false);
            let selector = format!("[data-event-id=\"{}\"]", m::string(payload));
            if let Ok(Some(e)) = dom::document().query_selector(&selector) {
                e.scroll_into_view();
            }
        }
        "run-file" => {
            // A second press on the chip puts its card away again.
            let path = payload.as_str().unwrap_or("");
            if card_path().as_deref() == Some(path) {
                hide_card(true);
            } else {
                show_card(&shown(), path);
            }
        }
        "run-add-path" => {
            hide_card(false);
            insert_draft(payload.as_str().unwrap_or(""));
        }
        "run-copy-path" => {
            let copying = dom::window()
                .navigator()
                .clipboard()
                .write_text(payload.as_str().unwrap_or(""));
            let copied = wasm_bindgen_futures::JsFuture::from(copying).await.is_ok();
            let selector = format!("#{CARD} [data-action=\"run-copy-path\"]");
            if let Ok(Some(button)) = dom::document().query_selector(&selector) {
                button.set_text_content(Some(if copied { "Copied" } else { "Copy failed" }));
            }
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
        events.push(tool(11, "file_change", "c1", "completed", json!({"changes":[{"path":"app/New.php","kind":"add"},{"path":"app/Old.php","kind":"delete"}],"ok":true})));
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
