use crate::{dom, model as m, session};
use serde_json::{json, Value as V};
use std::collections::BTreeMap;
use wasm_bindgen::JsCast;

mod diff;
mod files;
mod outcomes;
mod permissions;
mod stream;
#[cfg(test)]
mod tests;

pub(crate) use diff::*;
pub(crate) use files::files_map;
pub(crate) use outcomes::*;
pub(crate) use permissions::*;
pub(crate) use stream::*;

#[cfg(test)]
use files_map::place_popover as place;
use files_map::{
    bdi_el as bdi, count_words as count, district_label, focus_element_no_scroll as focus_node,
    hushed_el as hushed, place_of_path, splice_draft as spliced, truth, FileCardAction,
    FileTouchEntry, OUTSIDE,
};

thread_local! {
    pub(super) static DISMISS_ON_GRAPH_CLICK: wasm_bindgen::closure::Closure<dyn FnMut(web_sys::Event)> =
        wasm_bindgen::closure::Closure::new(|_: web_sys::Event| {
            hide_card(false);
        });
    pub(super) static CARD_OPENER: std::cell::RefCell<Option<web_sys::Element>> =
        const { std::cell::RefCell::new(None) };
    static ACTIVE_NOTIFICATION: std::cell::RefCell<
        Option<(
            web_sys::Notification,
            wasm_bindgen::closure::Closure<dyn FnMut(web_sys::Event)>,
        )>,
    > = const { std::cell::RefCell::new(None) };
}

pub(super) fn clean(s: &str) -> String {
    files_map::clean_bidi(s)
}

pub fn reset() {
    dom::cancel_timer("run-clock");
    dom::cancel_timer("permission-countdown");
    ACTIVE_NOTIFICATION.with(|slot| {
        if let Some((prev, _)) = slot.borrow_mut().take() {
            prev.set_onclick(None);
        }
    });
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
    m::write(|s| {
        s.stream.sticky_bottom = true;
        s.stream.unseen_count = 0;
        s.stream.visible_limit = 120;
        s.stream.has_older = false;
        s.stream.loading_older = false;
        s.stream.oldest_event_id = None;
        s.stream.latest_event_id = None;
    });
    hide_card(false);
    m::take("permission-pending");
    clear_stream_md_cache();
    dom::clear("events");
    dom::clear("run-tree");
    dom::clear("run-pane-plan");
    dom::clear("run-pane-commands");
    dom::clear("run-pane-memory");
    dom::clear("run-pane-omnigraph");
    if let Some(pane) = dom::get("run-pane-skillcache") {
        crate::skillcache::set_markup(&pane, "");
    }
    m::take("skillcache-error");
    update_scroll_ui();
    sync_load_earlier_control();
    sync_approvals_badge();
    // The away summary belongs to the session being left.
    away(false);
    // The session being left closes its strip now, not when the next one answers.
    render();
}

pub(super) fn initial() -> V {
    json!({"launch":"","calls":{},"open":[],"files":{},"commands":[],"plan":[],"searches":[],"steps":0,"messages":0,"finished":false,"outcome":null,"memory":null,"omnigraph":null,"skillcache":[],"kinds":[],"started":null})
}

pub(super) fn tool_name(v: &V) -> String {
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

pub(super) fn starts(v: &V) -> bool {
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
    let (run, replaced, resolved_call) = advance_with_call(m::data("run"), &event);
    if let Some(replaced) = replaced {
        m::set("run-before", replaced);
    }
    m::set("run", run);
    activity(&event);
    conversation_with_call(&event, resolved_call.as_deref());
    // Poll controls render the derived panels once after all fresh events.
    // Completion notifications must run before a later event can change launch.
    notify_finished();
}

/// One event into the model. Returns the newest launch and, when the event began another one,
/// the launch to remember in its place.
#[cfg(test)]
pub(super) fn advance(run: V, event: &V) -> (V, Option<V>) {
    let (next, replaced, _) = advance_with_call(run, event);
    (next, replaced)
}

pub(super) fn advance_with_call(mut run: V, event: &V) -> (V, Option<V>, Option<String>) {
    let launch = m::text(event, "launch_id");
    let mut replaced = None;
    let mut resolved_call = None;
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
            let call = match event["call_id"].as_str().or_else(|| event["call"].as_str()) {
                Some(call) => call.to_owned(),
                None => paired(&mut run, &key, start, &m::string(&event["id"])),
            };
            resolved_call = Some(call.clone());
            let target = event["path"]
                .as_str()
                .or_else(|| event["detail"].as_str())
                .unwrap_or("");
            let mut calls = run["calls"].as_object().cloned().unwrap_or_default();
            let mut item = calls.get(&call).cloned().unwrap_or(
                json!({"tool":name,"key":key,"path":event["path"],"detail":event["detail"],"event_id":event["id"],"started":event["at"]}),
            );
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
        "omnigraph" => run["omnigraph"] = event.clone(),
        "skillcache" => {
            let mut list = m::array(&run["skillcache"]);
            list.push(event.clone());
            run["skillcache"] = json!(list);
        }
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
    (run, replaced, resolved_call)
}

/// The call an event without a call id belongs to. Such events are plain labels: a start waits
/// under its tool's name, and the next completion of that tool closes the oldest one waiting.
pub(super) fn paired(run: &mut V, key: &str, start: bool, event_id: &str) -> String {
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
pub(super) fn tidy(path: &str) -> String {
    files_map::tidy_path(path)
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

pub(super) fn run_strip_now_parts(
    done: bool,
    outcome: &str,
    open: &[V],
) -> (String, String, String, bool) {
    if done {
        return (format!("Run {outcome}"), String::new(), String::new(), true);
    }
    if open.len() == 1 {
        let target = open[0]["path"]
            .as_str()
            .or_else(|| open[0]["detail"].as_str())
            .unwrap_or("")
            .trim()
            .to_owned();
        return (m::text(&open[0], "tool"), target, String::new(), false);
    }
    if !open.is_empty() {
        if let Some(latest) = open.iter().max_by(|a, b| {
            let ak = (m::string(&a["started"]), m::string(&a["event_id"]));
            let bk = (m::string(&b["started"]), m::string(&b["event_id"]));
            ak.cmp(&bk)
        }) {
            let tool = m::text(latest, "tool");
            let target = latest["path"]
                .as_str()
                .or_else(|| latest["detail"].as_str())
                .unwrap_or("")
                .trim()
                .to_owned();
            if !tool.is_empty() && !target.is_empty() {
                return (tool, target, format!("+{} more", open.len() - 1), false);
            }
        }
        return (
            format!("{} steps running", open.len()),
            String::new(),
            String::new(),
            true,
        );
    }
    ("Model’s turn".into(), String::new(), String::new(), true)
}

pub(super) fn can_prompt_notifications() -> bool {
    let has_notification = js_sys::Reflect::get(&dom::window(), &"Notification".into())
        .ok()
        .is_some_and(|v| !v.is_undefined() && !v.is_null());
    has_notification
        && web_sys::Notification::permission() == web_sys::NotificationPermission::Default
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
    let outcome = run["outcome"]
        .as_str()
        .unwrap_or_else(|| session["status"].as_str().unwrap_or("finished"));
    let (verb, target, more, plain) = run_strip_now_parts(done, outcome, &open);
    if mode == Mode::Now {
        update_clock();
    } else {
        // Only a running launch has a clock: a finished turn would show the age of its first event.
        dom::cancel_timer("run-clock");
        dom::text("run-clock", "");
    }
    dom::text("run-verb", &verb);
    dom::attr("run-verb", "data-plain", truth(plain));
    dom::text("run-target", &target);
    dom::hidden("run-target", target.is_empty());
    dom::text("run-more", &more);
    dom::hidden("run-more", more.is_empty());
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
    for (tab, pane) in [
        ("memory", render_run_memory_pane_html(&run["memory"])),
        (
            "omnigraph",
            render_run_omnigraph_pane_html(&run["omnigraph"]),
        ),
    ] {
        if let Some(element) = dom::get(&format!("run-pane-{tab}")) {
            dom::set_markup(&element, &pane);
        }
    }
    // This pane has buttons, so it is replaced only when what it shows changes.
    crate::skillcache::render_run_pane(&run);
    // What can be done about an observed turn depends on the session's
    // traces and compilations; they are read once per new SkillCache event.
    crate::skillcache::sync_session(&run);
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
                &serde_json::to_string_pretty(cmd).unwrap_or_default(),
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
            if !m::yes(&cmd["running"]) && !m::yes(&cmd["ok"]) && class["masked"].is_null() {
                let loc = dom::button(
                    "Localize failure on map",
                    "localize-run-failure",
                    failure_localize_payload(cmd, &m::text(cmd, "command")),
                );
                let _ = loc.class_list().add_1("compact");
                dom::add(&block, &loc);
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
        // Plan, Commands, Memory and Omnigraph ship hidden: each joins the tab list once the
        // launch has one.
        dom::hidden(&button, name != "files" && !has(&run, name));
        dom::attr(&button, "aria-selected", truth(selected));
        dom::attr(&button, "tabindex", if selected { "0" } else { "-1" });
        dom::hidden(&format!("run-pane-{name}"), !selected);
    }
}

pub async fn event(kind: &str, id: &str, action: &str, payload: &V, value: &str) -> bool {
    if kind == "input" && action == "permission-question-other" {
        let req = m::text(payload, "request_id");
        let idx = payload["index"].as_u64();
        let container_id = permission_event_container(payload);
        if !req.is_empty() && !value.trim().is_empty() {
            if let Some(idx) = idx {
                if let Some(block) =
                    dom::get(container_id).and_then(|c| find_permission_block(&c, &req))
                {
                    let selector = format!(
                        "fieldset.permission-question[data-question-index=\"{idx}\"] input.permission-question-other-radio"
                    );
                    if let Ok(Some(radio)) = block.query_selector(&selector) {
                        if let Some(input) = radio.dyn_ref::<web_sys::HtmlInputElement>() {
                            input.set_checked(true);
                        }
                    }
                }
            }
        }
        return true;
    }
    if kind == "input" && action == "diff-filter" {
        filter_diff_files(payload["scope"].as_str().unwrap_or(""), value);
        return true;
    }
    if kind != "click" {
        return false;
    }
    match action {
        "run-calm" => detail(false),
        "run-detailed" => detail(true),
        "run-compact" | "run-now" => detail(!m::yes(&m::data("run-detailed"))),
        "run-close" => close(),
        "run-open" => open(payload.as_str().unwrap_or("files")),
        "run-tab-files" | "run-tab-plan" | "run-tab-commands" | "run-tab-memory"
        | "run-tab-omnigraph" | "run-tab-skillcache" => {
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
        "run-card-close" => hide_card(true),
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
            dom::hidden("run-notify", !can_prompt_notifications());
        }
        "events-jump-latest" => jump_to_latest(),
        "events-load-earlier" => load_earlier_events().await,
        "localize-run-failure" => session::localize_failure_on_map(payload),
        "diff-expand-all" => {
            toggle_all_diff_groups(payload["scope"].as_str().unwrap_or(""), true);
        }
        "diff-collapse-all" => {
            toggle_all_diff_groups(payload["scope"].as_str().unwrap_or(""), false);
        }
        "diff-show-more" => {
            show_all_diff_lines(
                payload["scope"].as_str().unwrap_or(""),
                payload["file"].as_str().unwrap_or(""),
            );
        }
        "tool-output-expand" => {
            toggle_tool_output_expand(payload.as_str().unwrap_or(""));
        }
        "approvals-queue-toggle" => toggle_approvals_drawer().await,
        "approvals-drawer-close" => close_approvals_drawer(),
        "approvals-jump-session" => {
            close_approvals_drawer();
            let sid = payload.as_str().unwrap_or("");
            if !sid.is_empty() {
                session::select(sid).await;
            }
        }
        "approvals-jump-view" => {
            close_approvals_drawer();
            let target = payload.as_str().unwrap_or("sessions");
            session::open_view(target, true).await;
        }
        "permission-rule-revoke" => {
            let sid = m::text(payload, "session_id");
            let id = m::text(payload, "id");
            let rule_key = m::text(payload, "rule_key");
            revoke_permission_rule(&sid, &id, &rule_key).await;
        }
        "permission-rules-revoke-all" => {
            let sid = m::read(|s| s.sid.clone());
            revoke_all_permission_rules(&sid).await;
        }
        "permission-decide" => {
            let target = permission_event_container(payload);
            if !matches!(target, "events" | "approvals-drawer-list") {
                return false;
            }
            decide_permission(payload).await;
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
    let text = format!(
        "While you were away{}: {steps} steps · {edits} edits · {messages} messages · {failed} failed checks{}",
        if minutes > 0 {
            format!(" ({minutes} min)")
        } else {
            String::new()
        },
        if after["finished"] == true {
            format!(" · run {}", m::text(&after, "outcome"))
        } else {
            String::new()
        }
    );
    dom::text("run-away-text", &text);
    away(true);
    dom::text("run-announcer", &text);
}

pub(super) fn fire_browser_notification(
    title: &str,
    body: &str,
    tag: &str,
    silent: bool,
    focus_permission_id: Option<String>,
) {
    let options = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&options, &"body".into(), &body.into());
    let _ = js_sys::Reflect::set(&options, &"tag".into(), &tag.into());
    let _ = js_sys::Reflect::set(&options, &"silent".into(), &silent.into());
    let options: web_sys::NotificationOptions = options.unchecked_into();
    if let Ok(note) = web_sys::Notification::new_with_options(title, &options) {
        let n = note.clone();
        let cb = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            let _ = dom::window().focus();
            n.close();
            let focus_id = focus_permission_id.clone();
            wasm_bindgen_futures::spawn_local(async move {
                if m::read(|s| s.view != "sessions") {
                    session::open_view("sessions", true).await;
                }
                if let Some(request_id) = focus_id {
                    let selector = format!(
                        "#events [data-permission-id=\"{}\"]",
                        dom::escape_css_attr_value(&request_id)
                    );
                    if let Ok(Some(el)) = dom::document().query_selector(&selector) {
                        el.scroll_into_view();
                        if let Ok(Some(btn)) =
                            el.query_selector("button[data-action=\"permission-decide\"]")
                        {
                            focus_node(&btn);
                        }
                    } else {
                        permissions::open_approvals_drawer().await;
                    }
                }
            });
        });
        note.set_onclick(Some(cb.as_ref().unchecked_ref()));
        ACTIVE_NOTIFICATION.with(|slot| {
            if let Some((prev, _)) = slot.borrow_mut().replace((note, cb)) {
                prev.set_onclick(None);
            }
        });
    }
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
    fire_browser_notification(title, &body, &format!("harness-run-{sid}"), true, None);
}
