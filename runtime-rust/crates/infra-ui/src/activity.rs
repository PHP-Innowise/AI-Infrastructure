use crate::{api, dom, model as m};
use serde_json::{json, Value as V};
use std::{cell::RefCell, collections::BTreeMap};
use wasm_bindgen::JsCast;
#[derive(Clone, Default)]
struct Panel {
    sid: String,
    cursor: u64,
    epoch: u64,
    pending: bool,
    plan: Vec<V>,
    agents: BTreeMap<String, V>,
    order: Vec<String>,
    selected: String,
    follow: bool,
    active: bool,
}
thread_local! {static PANELS:RefCell<BTreeMap<String,Panel>>=const { RefCell::new(BTreeMap::new()) };}
fn edit<R>(root: &str, f: impl FnOnce(&mut Panel) -> R) -> R {
    PANELS.with(|all| {
        f(all
            .borrow_mut()
            .entry(root.into())
            .or_insert_with(|| Panel {
                follow: true,
                ..Panel::default()
            }))
    })
}
fn panel(root: &str) -> Panel {
    edit(root, |p| p.clone())
}
pub fn reset(root: &str) {
    edit(root, |p| {
        let epoch = p.epoch + 1;
        *p = Panel {
            follow: true,
            epoch,
            ..Panel::default()
        };
    });
    render(root);
}
pub fn set_plan(root: &str, steps: Vec<V>, active: bool) {
    edit(root, |p| {
        p.plan = steps;
        p.active = active;
    });
    render(root);
}
fn observe(root: &str, event: &V) {
    let Some(name) = event["agent"].as_str() else {
        return;
    };
    let Some(attempt) = event["attempt"].as_u64() else {
        return;
    };
    let key = format!("{name}#{attempt}");
    edit(root, |p| {
        if !p.agents.contains_key(&key) {
            p.order.push(key.clone());
            p.agents.insert(key.clone(),json!({"id":key,"agent":name,"attempt":attempt,"status":"running","events":[],"calls":{},"tools":0,"messages":0,"tokens":0}));
        }
        let agent = p.agents.get_mut(&key).expect("inserted agent");
        match event["kind"].as_str() {
            Some("agent") => {
                let fields = if event["status"] == "running" {
                    &[
                        "status",
                        "service",
                        "mode",
                        "provider",
                        "root",
                        "readable",
                        "writable",
                        "readable_count",
                        "writable_count",
                    ][..]
                } else {
                    &[
                        "status",
                        "ok",
                        "error",
                        "duration_seconds",
                        "summary",
                        "changed_files",
                        "changed_files_count",
                        "checks",
                        "reason",
                        "cost_usd",
                    ][..]
                };
                for field in fields {
                    if !event[*field].is_null() {
                        agent[*field] = event[*field].clone();
                    }
                }
                if event["status"] == "running" {
                    agent["started"] = event["at"].clone();
                } else {
                    agent["finished"] = event["at"].clone();
                    if event
                        .as_object()
                        .map(|o| o.contains_key("tokens"))
                        .unwrap_or(false)
                    {
                        agent["tokens"] = event["tokens"].clone();
                    }
                }
            }
            Some("usage") => {
                if agent["finished"].is_null() {
                    let count = [
                        "input_tokens",
                        "output_tokens",
                        "cache_read_input_tokens",
                        "cache_creation_input_tokens",
                    ]
                    .iter()
                    .filter_map(|k| event[*k].as_f64())
                    .sum::<f64>();
                    agent["tokens"] = json!(agent["tokens"].as_f64().unwrap_or(0.) + count);
                }
            }
            Some("agent_activity") => {
                let kind = m::text(event, "type");
                let call = m::text(event, "call");
                let mut events = m::array(&agent["events"]);
                if kind == "tool" && !call.is_empty() && event["state"] == "completed" {
                    if let Some(index) = agent["calls"][&call].as_u64() {
                        if let Some(item) = events.get_mut(index as usize) {
                            item["state"] = json!("completed");
                            item["ok"] = event["ok"].clone();
                            item["done"] = event["at"].clone();
                            agent["events"] = json!(events);
                            return;
                        }
                    }
                }
                if kind == "tool" {
                    agent["tools"] = json!(agent["tools"].as_u64().unwrap_or(0) + 1);
                    if !call.is_empty() {
                        agent["calls"][&call] = json!(events.len());
                    }
                }
                if kind == "text" {
                    agent["messages"] = json!(agent["messages"].as_u64().unwrap_or(0) + 1);
                }
                events.push(event.clone());
                agent["events"] = json!(events);
                agent["last"] = event.clone();
            }
            _ => {}
        }
    });
}
pub fn observe_session(sid: &str, event: &V) {
    let roots = PANELS.with(|all| {
        all.borrow()
            .iter()
            .filter(|(_, p)| p.sid == sid)
            .map(|(r, _)| r.clone())
            .collect::<Vec<_>>()
    });
    for root in roots {
        let id = event["id"].as_u64().unwrap_or(0);
        let accept = edit(&root, |p| {
            if id == 0 || id <= p.cursor {
                false
            } else {
                p.cursor = id;
                true
            }
        });
        if accept {
            observe(&root, event);
            render(&root);
        }
    }
}
pub async fn load(root: &str, sid: &str) {
    if sid.is_empty() {
        reset(root);
        return;
    }
    let epoch = edit(root, |p| {
        if p.sid != sid {
            let epoch = p.epoch + 1;
            let plan = p.plan.clone();
            let active = p.active;
            *p = Panel {
                sid: sid.into(),
                plan,
                active,
                follow: true,
                epoch,
                ..Panel::default()
            };
        }
        if p.pending {
            None
        } else {
            p.pending = true;
            Some(p.epoch)
        }
    });
    let Some(epoch) = epoch else { return };
    render(root);
    for _ in 0..40 {
        let p = panel(root);
        if p.epoch != epoch || p.sid != sid {
            break;
        }
        match api::get(&format!(
            "/api/sessions/{}?after={}",
            dom::enc(sid),
            p.cursor
        ))
        .await
        {
            Ok(data) => {
                if panel(root).epoch != epoch {
                    break;
                }
                let events = m::array(&data["events"]);
                for event in &events {
                    let id = event["id"].as_u64().unwrap_or(0);
                    let accept = edit(root, |p| {
                        if id <= p.cursor {
                            false
                        } else {
                            p.cursor = id;
                            true
                        }
                    });
                    if accept {
                        observe(root, event);
                    }
                }
                if events.len() < 250 {
                    break;
                }
            }
            Err(error) => {
                if panel(root).epoch == epoch {
                    if let Some(root) = dom::get(root) {
                        dom::add(
                            &root,
                            &dom::el(
                                "p",
                                "agent-error",
                                &format!("Agent activity is unavailable: {}", error.message),
                            ),
                        );
                    }
                }
                break;
            }
        }
    }
    if panel(root).epoch == epoch {
        edit(root, |p| p.pending = false);
        render(root);
    }
}
fn ordered(p: &Panel) -> Vec<V> {
    let mut result = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for step in &p.plan {
        let name = m::text(step, "id");
        let latest = p
            .order
            .iter()
            .rev()
            .filter_map(|id| p.agents.get(id))
            .find(|a| a["agent"] == name);
        let mut item = step.clone();
        if !item.is_object() {
            continue;
        }
        if let Some(a) = latest {
            for (k, v) in a.as_object().into_iter().flatten() {
                item[k] = v.clone();
            }
            item["key"] = a["id"].clone();
            seen.insert(m::text(a, "id"));
        } else {
            item["key"] = V::Null;
            item["status"] = step["journal"]
                .as_str()
                .map(|s| json!(s))
                .unwrap_or(json!("pending"));
            item["earlier"] = json!(step["journal"] == "completed");
        }
        item["step"] = json!(name);
        if !p.active && step["journal"].is_string() {
            item["status"] = step["journal"].clone();
        }
        result.push(item);
    }
    for key in &p.order {
        let a = &p.agents[key];
        if !seen.contains(key) && !p.plan.iter().any(|s| s["id"] == a["agent"]) {
            let mut item = a.clone();
            item["key"] = json!(key);
            item["step"] = a["agent"].clone();
            item["label"] = a["agent"].clone();
            result.push(item);
        }
    }
    result
}
fn selection(a: &V) -> String {
    a["key"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("step:{}", m::text(a, "step")))
}
fn status(s: &str) -> String {
    match s {
        "pending" => "Waiting".into(),
        "running" => "Working".into(),
        "completed" => "Completed".into(),
        "blocked" => "Blocked".into(),
        "interrupted" => "Stopped".into(),
        _ => m::label(s),
    }
}
fn describe(v: &V) -> String {
    if v["type"] == "tool" {
        ["service", "path", "detail"]
            .iter()
            .map(|k| m::text(v, k))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ")
    } else {
        m::text(v, "text")
    }
}
fn clock(value: &V) -> String {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(
        value.as_str().unwrap_or(""),
    ));
    if date.get_time().is_finite() {
        format!(
            "{:02}:{:02}:{:02}",
            date.get_hours(),
            date.get_minutes(),
            date.get_seconds()
        )
    } else {
        String::new()
    }
}
fn duration(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0. {
        return String::new();
    }
    let n = seconds.round() as u64;
    if n >= 60 {
        format!("{}m {:02}s", n / 60, n % 60)
    } else {
        format!("{n}s")
    }
}
fn render(root: &str) {
    let Some(host) = dom::get(root) else { return };
    let mut p = panel(root);
    let items = ordered(&p);
    if p.follow {
        p.selected = items
            .iter()
            .rev()
            .find(|a| a["status"] == "running" && a["key"].is_string())
            .or_else(|| items.iter().rev().find(|a| a["key"].is_string()))
            .map(selection)
            .unwrap_or_default();
    } else if let Some(step) = p.selected.strip_prefix("step:") {
        if let Some(a) = items
            .iter()
            .find(|a| a["step"] == step && a["key"].is_string())
        {
            p.selected = selection(a);
        }
    }
    edit(root, |state| state.selected = p.selected.clone());
    let scroll = host
        .query_selector(".agent-events")
        .ok()
        .flatten()
        .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
        .map(|e| {
            (
                e.scroll_top(),
                e.scroll_height() - e.scroll_top() - e.client_height() < 40,
            )
        });
    host.set_text_content(None);
    let label = dom::el("label", "checkbox agent-follow", "");
    let input = dom::el("input", "", "");
    let _ = input.set_attribute("type", "checkbox");
    let _ = input.set_attribute("data-action", "agent-follow");
    let _ = input.set_attribute("data-payload", &json!(root).to_string());
    if let Some(e) = input.dyn_ref::<web_sys::HtmlInputElement>() {
        e.set_checked(p.follow);
    }
    dom::add(&label, &input);
    dom::add(&label, &dom::el("span", "", "Follow the active agent"));
    dom::add(&host, &label);
    let cards = dom::el("div", "agent-cards", "");
    dom::add(&host, &cards);
    for a in &items {
        let selected = selection(a);
        let card = dom::button("", "agent-select", json!({"root":root,"key":selected}));
        card.set_class_name(&format!("agent-card {}", m::text(a, "status")));
        let _ = card.set_attribute(
            "aria-pressed",
            if selected == p.selected {
                "true"
            } else {
                "false"
            },
        );
        let top = dom::el("div", "agent-card-top", "");
        dom::add(&top, &dom::el("span", "agent-dot", ""));
        dom::add(
            &top,
            &dom::el(
                "strong",
                "",
                a["label"]
                    .as_str()
                    .unwrap_or_else(|| a["agent"].as_str().unwrap_or("Agent")),
            ),
        );
        let elapsed = if let Some(start) = a["started"].as_str() {
            let end = a["finished"]
                .as_str()
                .map(|s| js_sys::Date::new(&s.into()).get_time())
                .unwrap_or(js_sys::Date::now());
            duration(
                a["duration_seconds"]
                    .as_f64()
                    .unwrap_or((end - js_sys::Date::new(&start.into()).get_time()) / 1000.),
            )
        } else {
            String::new()
        };
        dom::add(
            &top,
            &dom::el(
                "span",
                "agent-card-status",
                &[status(a["status"].as_str().unwrap_or("pending")), elapsed]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · "),
            ),
        );
        dom::add(&card, &top);
        let mut scope = vec![m::text(a, "mode")];
        if a["attempt"].as_u64().unwrap_or(1) > 1 {
            scope.push(format!("attempt {}", a["attempt"]));
        }
        for (field, count, prefix) in [
            ("readable", "readable_count", "reads"),
            ("writable", "writable_count", "writes"),
        ] {
            let names = m::array(&a[field])
                .iter()
                .filter_map(V::as_str)
                .map(|name| {
                    if name == "__system__" {
                        "system folder"
                    } else {
                        name
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            if !names.is_empty() {
                let extra = a[count]
                    .as_u64()
                    .unwrap_or(0)
                    .saturating_sub(m::array(&a[field]).len() as u64);
                scope.push(format!(
                    "{prefix} {names}{}",
                    if extra > 0 {
                        format!(" +{extra} more")
                    } else {
                        String::new()
                    }
                ));
            }
        }
        let scope = scope
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        if !scope.is_empty() {
            dom::add(&card, &dom::el("p", "agent-card-scope", &scope));
        }
        let now = if a["status"] == "running" && !a["last"].is_null() {
            let last = &a["last"];
            format!(
                "{}{}",
                if last["type"] == "tool" {
                    format!("{} · ", last["tool"].as_str().unwrap_or("Tool"))
                } else {
                    String::new()
                },
                describe(last)
            )
        } else if a["earlier"] == true {
            "Completed in an earlier launch.".into()
        } else {
            ["summary", "reason", "error", "goal"]
                .iter()
                .map(|k| m::text(a, k))
                .find(|s| !s.is_empty())
                .unwrap_or_default()
        };
        if !now.is_empty() {
            dom::add(
                &card,
                &dom::el(
                    "p",
                    if a["status"] == "running" {
                        "agent-card-now"
                    } else {
                        "agent-card-summary"
                    },
                    &now.chars().take(180).collect::<String>(),
                ),
            );
        }
        let mut stats = Vec::new();
        for (field, noun) in [("tools", "tool call"), ("messages", "message")] {
            if let Some(n) = a[field].as_u64().filter(|n| *n > 0) {
                stats.push(format!("{n} {noun}{}", if n == 1 { "" } else { "s" }));
            }
        }
        if let Some(n) = a["tokens"].as_f64().filter(|n| *n > 0.) {
            stats.push(if n >= 1000. {
                format!("{:.1}k tokens", n / 1000.)
            } else {
                format!("{n:.0} tokens")
            });
        }
        if let Some(n) = a["cost_usd"].as_f64().filter(|n| *n > 0.) {
            stats.push(format!("{}{n:.4}", "$"));
        }
        let changed = a["changed_files_count"]
            .as_u64()
            .unwrap_or(m::array(&a["changed_files"]).len() as u64);
        if changed > 0 {
            stats.push(format!("{changed} changed files"));
        }
        if !stats.is_empty() {
            dom::add(&card, &dom::el("p", "agent-card-stats", &stats.join(" · ")));
        }
        dom::add(&cards, &card);
    }
    let timeline = dom::el("div", "agent-timeline", "");
    let _ = timeline.set_attribute("aria-live", "polite");
    let heading = dom::el("div", "agent-timeline-heading", "");
    let list = dom::el("ol", "agent-events", "");
    dom::add(&timeline, &heading);
    dom::add(&timeline, &list);
    dom::add(&host, &timeline);
    if let Some(a) = p.agents.get(&p.selected) {
        let label = items
            .iter()
            .find(|i| i["key"] == p.selected)
            .map(|i| m::text(i, "label"))
            .unwrap_or_else(|| m::text(a, "agent"));
        dom::add(&heading, &dom::el("strong", "", &label));
        dom::add(
            &heading,
            &dom::el(
                "span",
                "",
                &[
                    m::summary(&m::provider(&m::text(a, "provider"))),
                    m::text(a, "root"),
                ]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · "),
            ),
        );
        let events = m::array(&a["events"]);
        if events.is_empty() {
            dom::add(
                &list,
                &dom::el(
                    "li",
                    "agent-placeholder",
                    if a["status"] == "running" {
                        "Waiting for the first action…"
                    } else {
                        "No activity was recorded for this agent."
                    },
                ),
            );
        }
        for item in events {
            let kind = m::text(&item, "type");
            let row = dom::el(
                "li",
                &format!(
                    "agent-event {kind}{}",
                    if item["ok"] == false { " failed" } else { "" }
                ),
                "",
            );
            dom::add(
                &row,
                &dom::el("time", "agent-event-time", &clock(&item["at"])),
            );
            let title = if kind == "tool" {
                item["tool"].as_str().unwrap_or("Tool").to_owned()
            } else {
                match kind.as_str() {
                    "text" => "Says".into(),
                    "thinking" => "Reasons".into(),
                    "plan" => "Plan".into(),
                    "error" => "Error".into(),
                    "status" => "Note".into(),
                    _ => m::label(&kind),
                }
            };
            dom::add(&row, &dom::el("span", "agent-event-kind", &title));
            dom::add(
                &row,
                &dom::el(
                    if ["text", "thinking", "plan"].contains(&kind.as_str()) {
                        "pre"
                    } else {
                        "span"
                    },
                    "agent-event-body",
                    &describe(&item),
                ),
            );
            if kind == "tool" {
                dom::add(
                    &row,
                    &dom::el(
                        "span",
                        "agent-event-state",
                        if item["state"] == "completed" {
                            if item["ok"] == false {
                                "failed"
                            } else {
                                "done"
                            }
                        } else {
                            "running"
                        },
                    ),
                );
            }
            dom::add(&list, &row);
        }
        let outcome = [m::text(a, "summary"), m::text(a, "reason")]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let outcome = if outcome.is_empty() {
            m::text(a, "error")
        } else {
            outcome
        };
        if !outcome.is_empty() {
            dom::add(
                &list,
                &dom::el(
                    "li",
                    if a["ok"] == true {
                        "agent-outcome ok"
                    } else {
                        "agent-outcome failed"
                    },
                    &outcome,
                ),
            );
        }
    } else if let Some(a) = items.iter().find(|a| selection(a) == p.selected) {
        dom::add(&heading, &dom::el("strong", "", &m::text(a, "label")));
        dom::add(
            &list,
            &dom::el(
                "li",
                "agent-placeholder",
                if a["earlier"] == true {
                    "This agent completed in an earlier launch; choose that launch above."
                } else {
                    "This agent has not run in this launch."
                },
            ),
        );
    } else {
        dom::add(
            &heading,
            &dom::el(
                "span",
                "",
                if root == "system-discovery-agent" {
                    "The discovery agent has not started yet."
                } else {
                    "Select an agent card to follow its work."
                },
            ),
        );
    }
    if let Ok(e) = list.dyn_into::<web_sys::HtmlElement>() {
        if let Some((top, stick)) = scroll {
            e.set_scroll_top(if stick { e.scroll_height() } else { top });
        }
    }
}
pub fn event(kind: &str, action: &str, payload: &V, checked: bool) -> bool {
    match (kind, action) {
        ("click", "agent-select") => {
            let root = m::text(payload, "root");
            edit(&root, |p| {
                p.selected = m::text(payload, "key");
                p.follow = false;
            });
            render(&root);
            true
        }
        ("change", "agent-follow") => {
            let root = payload.as_str().unwrap_or("");
            edit(root, |p| p.follow = checked);
            render(root);
            true
        }
        _ => false,
    }
}
