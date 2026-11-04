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
    for key in ["run", "agents", "run-file-filter", "run-command-filter"] {
        m::take(key);
    }
    dom::clear("events");
    dom::clear("run-tree");
    dom::clear("run-pane-plan");
    dom::clear("run-pane-commands");
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
    let mut run = m::data("run");
    let launch = m::text(&event, "launch_id");
    if !run.is_object() || (!launch.is_empty() && run["launch"] != launch) {
        run = initial();
        run["launch"] = json!(launch);
        run["started"] = event["at"].clone();
    }
    let kind = m::text(&event, "kind");
    let mut kinds = m::array(&run["kinds"]);
    if !kinds.iter().any(|k| k == &kind) {
        kinds.push(json!(kind));
    }
    run["kinds"] = json!(kinds);
    match kind.as_str() {
        "tool" => {
            let name = tool_name(&event);
            let key = name.to_lowercase();
            let call = event["call"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("anonymous-{}", m::string(&event["id"])));
            let start = starts(&event);
            let target = event["path"]
                .as_str()
                .or_else(|| event["detail"].as_str())
                .unwrap_or("");
            let mut calls = run["calls"].as_object().cloned().unwrap_or_default();
            let mut item=calls.get(&call).cloned().unwrap_or(json!({"tool":name,"key":key,"path":event["path"],"detail":event["detail"],"event_id":event["id"],"started":event["at"]}));
            item["open"] = json!(start);
            item["ok"] = json!(event["ok"]
                .as_bool()
                .unwrap_or(!m::text(&event, "text").contains("failed")));
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
                record_file(&mut run, target, &key, &event);
            }
            for change in m::array(&event["changes"]) {
                record_file(&mut run, &m::text(&change, "path"), "edit", &event);
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
            if m::text(&event, "text").starts_with("Run failed (") {
                run["finished"] = json!(true);
                run["outcome"] = json!("failed");
            }
        }
        "result" => run["result"] = event.clone(),
        _ => {}
    }
    m::set("run", run);
    activity(&event);
    conversation(&event);
    // Poll controls render the derived panels once after all fresh events.
    // Completion notifications must run before a later event can change launch.
    notify_finished();
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
pub fn render() {
    let run = m::data("run");
    if !run.is_object() {
        dom::cancel_timer("run-clock");
        dom::hidden("run-view", true);
        return;
    }
    let session = m::read(|s| s.selected.clone());
    let native = native_run(&run);
    dom::hidden("run-view", !native);
    if !native {
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
    update_clock();
    dom::text("run-now-text", &now);
    dom::text("run-verb", &now);
    dom::text(
        "run-line2",
        &format!(
            "{} steps · {} messages",
            m::string(&run["steps"]),
            m::string(&run["messages"])
        ),
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
    let tab = m::data("run-tab").as_str().unwrap_or("files").to_owned();
    for name in ["files", "plan", "commands"] {
        dom::hidden(&format!("run-pane-{name}"), name != tab);
        dom::attr(
            &format!("run-tab-{name}"),
            "aria-selected",
            if name == tab { "true" } else { "false" },
        );
    }
    let detailed = m::data("run-detailed").as_bool().unwrap_or(false);
    dom::hidden("run-panel", !detailed);
    dom::attr(
        "run-view",
        "data-detailed",
        if detailed { "true" } else { "false" },
    );
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
        "run-calm" => {
            m::set("run-detailed", json!(false));
            render();
        }
        "run-detailed" | "run-compact" | "run-now" => {
            m::set("run-detailed", json!(!m::yes(&m::data("run-detailed"))));
            render();
        }
        "run-close" => {
            m::set("run-detailed", json!(false));
            render();
        }
        "run-tab-files" | "run-tab-plan" | "run-tab-commands" => {
            m::set("run-tab", json!(action.trim_start_matches("run-tab-")));
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
            dom::hidden("run-away", true);
        }
        "run-away-jump" => {
            if let Some(e) =
                dom::get("conversation").and_then(|v| v.dyn_into::<web_sys::HtmlElement>().ok())
            {
                e.set_scroll_top(e.scroll_height());
            }
            dom::hidden("run-away", true);
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
    dom::hidden("run-away", false);
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
