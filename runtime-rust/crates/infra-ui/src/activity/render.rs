use super::ingest::*;
use crate::{
    dom, model as m,
    run::files_map::{
        self, count_words, focus_element_no_scroll, normalize_view, offered_filters, splice_draft,
        truth, FileTouchEntry,
    },
};
use serde_json::{json, Value as V};
use std::collections::{BTreeMap, BTreeSet};
use wasm_bindgen::JsCast;

thread_local! {
    static DISMISS_ON_GRAPH_CLICK: wasm_bindgen::closure::Closure<dyn FnMut(web_sys::Event)> =
        wasm_bindgen::closure::Closure::new(|_: web_sys::Event| {
            hide_any_activity_card(false);
        });
}

fn status(s: &str) -> String {
    match s {
        "pending" => "Waiting".into(),
        "running" => "Working".into(),
        "completed" => "Completed".into(),
        "blocked" => "Blocked".into(),
        "interrupted" => "Stopped".into(),
        "reported" => "Reported complete".into(),
        _ => m::label(s),
    }
}

pub(super) fn describe(v: &V) -> String {
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
    if !cfg!(target_arch = "wasm32") {
        return String::new();
    }
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

pub(super) fn duration(seconds: f64) -> String {
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

fn chip_for_root_key(root: &str, key: &str) -> Option<web_sys::Element> {
    if !cfg!(target_arch = "wasm32") {
        return None;
    }
    dom::all(&format!("#{root} .run-map-main .run-file"))
        .into_iter()
        .find(|chip| chip.get_attribute("data-focus-key").as_deref() == Some(key))
}

fn open_card_root() -> Option<(String, String)> {
    PANELS.with(|all| {
        all.borrow()
            .iter()
            .find_map(|(root, p)| p.card_key.as_ref().map(|key| (root.clone(), key.clone())))
    })
}

pub(super) fn hide_card_for_root(root: &str, restore_focus: bool) {
    let prev = edit(root, |p| p.card_key.take());
    if !cfg!(target_arch = "wasm32") {
        return;
    }
    let owner = owner_tag(root);
    let owned_by_root = dom::get(CARD_ID)
        .and_then(|c| c.get_attribute("data-owner"))
        .as_deref()
        == Some(owner.as_str());
    if prev.is_some() || owned_by_root {
        if !restore_focus {
            dom::hidden_with_restore(CARD_ID, true, false);
            return;
        }
        if let Some(chip) = prev.as_deref().and_then(|k| chip_for_root_key(root, k)) {
            dom::hidden_with_restore(CARD_ID, true, false);
            focus_element_no_scroll(&chip);
        } else {
            dom::hidden_with_restore(CARD_ID, true, true);
        }
    }
}

fn hide_any_activity_card(restore_focus: bool) -> bool {
    if let Some((root, _)) = open_card_root() {
        hide_card_for_root(&root, restore_focus);
        true
    } else if cfg!(target_arch = "wasm32")
        && dom::get(CARD_ID)
            .and_then(|c| c.get_attribute("data-owner"))
            .is_some_and(|o| o.starts_with("activity:"))
    {
        dom::hidden_with_restore(CARD_ID, true, restore_focus);
        true
    } else {
        false
    }
}

fn open_activity_file_card(root: &str, key: &str) {
    let snap = snapshot_for_root(root);
    let Some(entry) = snap.files.iter().find(|e| e.key() == key) else {
        return;
    };
    edit(root, |p| p.card_key = Some(key.to_owned()));
    if !cfg!(target_arch = "wasm32") {
        return;
    }
    let _ = m::take("run-card");
    if let Some(card) = dom::get(CARD_ID) {
        let _ = card.remove_attribute("data-signature");
    }
    let actions = system_card_actions(root, entry);
    let owner = owner_tag(root);
    DISMISS_ON_GRAPH_CLICK.with(|cb| {
        files_map::fill_card_element(
            CARD_ID,
            &owner,
            entry,
            &actions,
            Some(("inspect-system-file-graph", cb)),
        );
    });
    dom::hidden(CARD_ID, false);
    if let Some(chip) = chip_for_root_key(root, key) {
        files_map::position_card_element(CARD_ID, &chip);
    }
    let selector = format!("#{CARD_ID} button");
    if let Ok(Some(first)) = dom::document().query_selector(&selector) {
        focus_element_no_scroll(&first);
    }
}

fn insert_system_task_path(path: &str) {
    if !cfg!(target_arch = "wasm32") || path.is_empty() {
        return;
    }
    dom::hidden("system-plan-form", false);
    dom::hidden("system-runs-empty", true);
    dom::attr("system-new-change", "aria-expanded", "true");
    let Some(task) =
        dom::get("system-task").and_then(|e| e.dyn_into::<web_sys::HtmlTextAreaElement>().ok())
    else {
        return;
    };
    if task.disabled() {
        return;
    }
    let value = task.value();
    let units = value.encode_utf16().count() as u32;
    let start = task
        .selection_start()
        .ok()
        .flatten()
        .unwrap_or(units)
        .min(units);
    let end = task
        .selection_end()
        .ok()
        .flatten()
        .unwrap_or(start)
        .clamp(start, units);
    let (draft, caret) = splice_draft(&value, start, end, path);
    task.set_value(&draft);
    let _ = task.set_selection_range(caret, caret);
    focus_element_no_scroll(task.as_ref());
}

fn copy_path_to_clipboard(path: &str) {
    if !cfg!(target_arch = "wasm32") {
        return;
    }
    let text = path.to_owned();
    wasm_bindgen_futures::spawn_local(async move {
        let promise = dom::window().navigator().clipboard().write_text(&text);
        let ok = wasm_bindgen_futures::JsFuture::from(promise).await.is_ok();
        let selector = format!("#{CARD_ID} [data-action=\"activity-copy-path\"]");
        if let Ok(Some(btn)) = dom::document().query_selector(&selector) {
            btn.set_text_content(Some(if ok { "Copied" } else { "Copy failed" }));
        }
    });
}

fn render_all_agents_card(
    root: &str,
    items: &[V],
    files_count: usize,
    selected: bool,
) -> web_sys::Element {
    let card = dom::button("", "agent-select", json!({"root": root, "key": "all"}));
    let agg_status = if items.iter().any(|a| a["status"] == "running") {
        "running"
    } else if items.iter().any(|a| a["status"] == "blocked") {
        "blocked"
    } else if items.iter().any(|a| a["status"] == "interrupted") {
        "interrupted"
    } else if items
        .iter()
        .any(|a| matches!(a["status"].as_str(), Some("completed" | "reported")))
    {
        "completed"
    } else {
        "pending"
    };
    card.set_class_name(&format!("agent-card {agg_status}"));
    let _ = card.set_attribute("data-focus-key", "card:all");
    let _ = card.set_attribute("aria-pressed", truth(selected));

    let top = dom::el("div", "agent-card-top", "");
    dom::add(&top, &dom::el("span", "agent-dot", ""));
    dom::add(&top, &dom::el("strong", "", "All agents"));

    let total_secs: f64 = items
        .iter()
        .filter_map(|a| a["duration_seconds"].as_f64())
        .sum();
    let elapsed = if total_secs > 0. {
        duration(total_secs)
    } else {
        String::new()
    };
    dom::add(
        &top,
        &dom::el(
            "span",
            "agent-card-status",
            &[status(agg_status), elapsed]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · "),
        ),
    );
    dom::add(&card, &top);

    let services: BTreeSet<String> = items
        .iter()
        .map(|a| m::text(a, "service"))
        .filter(|s| !s.is_empty() && s != "__system__")
        .collect();
    let scope = format!(
        "{} · {}",
        count_words(items.len() as u64, "agent"),
        count_words(services.len().max(1) as u64, "service")
    );
    dom::add(&card, &dom::el("p", "agent-card-scope", &scope));

    if let Some(active_agent) = items
        .iter()
        .rev()
        .find(|a| a["status"] == "running" && !a["last"].is_null())
    {
        let last = &active_agent["last"];
        let short = files_map::short_agent_name(active_agent["agent"].as_str().unwrap_or("agent"));
        let line = format!(
            "{short} · {}{}",
            if last["type"] == "tool" {
                format!("{} · ", last["tool"].as_str().unwrap_or("Tool"))
            } else {
                String::new()
            },
            describe(last)
        );
        dom::add(
            &card,
            &dom::el(
                "p",
                "agent-card-now",
                &line.chars().take(180).collect::<String>(),
            ),
        );
    }

    let total_tools: u64 = items.iter().filter_map(|a| a["tools"].as_u64()).sum();
    let total_tokens: f64 = items.iter().filter_map(|a| a["tokens"].as_f64()).sum();
    let total_cost: f64 = items.iter().filter_map(|a| a["cost_usd"].as_f64()).sum();
    let mut stats = Vec::new();
    if total_tools > 0 {
        stats.push(count_words(total_tools, "tool call"));
    }
    if total_tokens > 0. {
        stats.push(if total_tokens >= 1000. {
            format!("{:.1}k tokens", total_tokens / 1000.)
        } else {
            format!("{total_tokens:.0} tokens")
        });
    }
    if total_cost > 0. {
        stats.push(format!("${total_cost:.4}"));
    }
    if files_count > 0 {
        stats.push(count_words(files_count as u64, "touched file"));
    }
    if !stats.is_empty() {
        dom::add(&card, &dom::el("p", "agent-card-stats", &stats.join(" · ")));
    }
    card
}

fn render_agent_card(root: &str, a: &V, is_selected: bool) -> web_sys::Element {
    let sel_key = selection(a);
    let card = dom::button("", "agent-select", json!({"root": root, "key": sel_key}));
    card.set_class_name(&format!("agent-card {}", m::text(a, "status")));
    let _ = card.set_attribute("data-focus-key", &format!("card:{sel_key}"));
    let _ = card.set_attribute("aria-pressed", truth(is_selected));

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
    let elapsed = if let Some(secs) = a["duration_seconds"].as_f64() {
        duration(secs)
    } else if let Some(start) = a["started"].as_str() {
        let end = a["finished"]
            .as_str()
            .or_else(|| {
                (a["status"] != "running")
                    .then(|| a["last"]["at"].as_str())
                    .flatten()
            })
            .map(|s| js_sys::Date::new(&s.into()).get_time())
            .unwrap_or_else(js_sys::Date::now);
        duration((end - js_sys::Date::new(&start.into()).get_time()) / 1000.)
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
    for (field, count_field, prefix) in [
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
            let extra = a[count_field]
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
            stats.push(count_words(n, noun));
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
        stats.push(format!("${n:.4}"));
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
    card
}

fn folder_restore_key(folder_name: &str) -> String {
    format!("folder:{folder_name}")
}

pub(super) fn build_folder_render_items(
    folders: &[(String, Vec<FileTouchEntry>)],
    show_agent_badge: bool,
) -> Vec<V> {
    folders
        .iter()
        .map(|(folder_name, entries)| {
            let files = entries
                .iter()
                .map(|file| {
                    let agent_note = if show_agent_badge && !file.agents.is_empty() {
                        format!(" · {}", files_map::format_agent_badge(&file.agents))
                    } else {
                        String::new()
                    };
                    json!({
                        "key": file.key(),
                        "path": file.path,
                        "reads": file.reads,
                        "edits": file.edits,
                        "outside": file.outside,
                        "agent_note": agent_note,
                        "timeline_agent": file.timeline_agent,
                        "event_id": file.event_id,
                        "service_id": file.service_id,
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "key": folder_restore_key(folder_name),
                "folder_name": folder_name,
                "files": files,
            })
        })
        .collect()
}

fn build_folder_details_element(root: &str, item: &V) -> web_sys::Element {
    let folder_name = m::text(item, "folder_name");
    let files = m::array(&item["files"]);
    let restore_key = m::text(item, "key");
    let details = dom::el("details", "run-tree-folder", "");
    let _ = details.set_attribute("open", "");
    let _ = details.set_attribute("data-restore-key", &restore_key);
    let summary = dom::el("summary", "", &format!("{folder_name} · {}", files.len()));
    let _ = summary.set_attribute("data-focus-key", &format!("summary:{restore_key}"));
    dom::add(&details, &summary);
    for file in &files {
        let row = dom::el("div", "run-row-actions", "");
        let path = m::text(file, "path");
        let reads = file["reads"].as_u64().unwrap_or(0);
        let edits = file["edits"].as_u64().unwrap_or(0);
        let agent_note = m::text(file, "agent_note");
        let label = format!(
            "{path} · {} · {}{agent_note}",
            count_words(reads, "read"),
            count_words(edits, "edit")
        );
        let reveal = dom::button(
            &label,
            "activity-reveal",
            json!({
                "root": root,
                "agent": file["timeline_agent"],
                "event_id": file["event_id"],
                "path": path,
                "service": file["service_id"],
            }),
        );
        let _ = reveal.set_attribute("data-focus-key", &format!("list:{}", m::text(file, "key")));
        dom::add(&row, &reveal);
        if !m::yes(&file["outside"]) && !path.is_empty() {
            let svc = file["service_id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or("__system__")
                .to_owned();
            let map_btn = dom::button(
                "Map",
                "inspect-system-file-graph",
                json!({"service": svc, "path": path}),
            );
            map_btn.set_class_name("button compact");
            let _ = map_btn.set_attribute("data-service", &svc);
            let _ = map_btn.set_attribute("data-path", &path);
            let _ = map_btn.set_attribute("aria-label", &format!("Show {path} on dependency map"));
            dom::add(&row, &map_btn);
        }
        dom::add(&details, &row);
    }
    details
}

pub(super) fn command_restore_keys(commands: &[V]) -> Vec<String> {
    let mut seen = BTreeMap::<String, usize>::new();
    commands
        .iter()
        .enumerate()
        .map(|(index, cmd)| {
            let call = m::text(cmd, "call");
            let base = if !call.is_empty() {
                format!("cmd:{call}")
            } else {
                let agent = m::text(cmd, "agent_key");
                let command = m::text(cmd, "command");
                if !command.is_empty() {
                    format!("cmd:{agent}:{command}")
                } else {
                    format!("cmd:idx:{index}")
                }
            };
            let count = seen.entry(base.clone()).or_insert(0);
            let key = if *count == 0 {
                base
            } else {
                format!("{base}#{}", *count)
            };
            *count += 1;
            key
        })
        .collect()
}

pub(super) fn command_localize_payload(
    cmd: &V,
    files: &[FileTouchEntry],
    root: Option<&str>,
    sid: &str,
    run_id: &str,
) -> Option<V> {
    if m::yes(&cmd["running"]) || m::yes(&cmd["ok"]) || m::yes(&cmd["not_run"]) {
        return None;
    }
    let command = m::text(cmd, "command");
    let error = m::text(cmd, "error");
    let output = m::text(cmd, "output");
    let summary = m::text(cmd, "summary");
    let texts = [
        summary.as_str(),
        error.as_str(),
        output.as_str(),
        command.as_str(),
    ];
    let failing_queries = super::events_card::extract_failure_queries(&texts, &command);

    let cmd_service = m::text(cmd, "service");
    let cmd_agent = m::text(cmd, "agent_key");
    let mut extra_paths: Vec<String> = m::array(&cmd["paths"])
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    for entry in files {
        let matches_scope = (!cmd_service.is_empty()
            && entry.service_id.as_deref() == Some(cmd_service.as_str()))
            || (!cmd_agent.is_empty()
                && entry.timeline_agent.as_deref() == Some(cmd_agent.as_str()))
            || (cmd_service.is_empty() && cmd_agent.is_empty());
        if matches_scope && !entry.outside && !entry.path.is_empty() {
            extra_paths.push(entry.path.clone());
        }
    }
    let hint_paths = super::events_card::extract_hint_paths(&texts, &extra_paths);

    let sys_run = m::data("system-run");
    let prefer_system = root == Some("system-agents")
        || (!run_id.is_empty() && m::text(&sys_run, "id") == run_id)
        || (!m::text(&sys_run, "config_path").is_empty() && sid.is_empty());
    let config_path = if prefer_system {
        m::text(&sys_run, "config_path")
    } else {
        String::new()
    };
    let mut project_id = if prefer_system {
        m::text(&sys_run, "project_id")
    } else {
        m::read(|s| {
            if !sid.is_empty() && m::text(&s.selected, "id") == sid {
                m::text(&s.selected, "project_id")
            } else if !sid.is_empty() {
                s.sessions
                    .iter()
                    .find(|x| m::text(x, "id") == sid)
                    .map(|x| m::text(x, "project_id"))
                    .unwrap_or_default()
            } else {
                m::text(&s.selected, "project_id")
            }
        })
    };
    if project_id.is_empty() && cfg!(target_arch = "wasm32") {
        let fallback_id = if prefer_system {
            "system-project"
        } else {
            "project"
        };
        project_id = dom::value(fallback_id);
    }

    let mut req = json!({
        "project_id": project_id,
        "failing_queries": failing_queries,
        "hint_paths": hint_paths,
        "prefer_full_modal": true,
    });
    if !sid.is_empty() {
        req["session_id"] = json!(sid);
    }
    if !run_id.is_empty() {
        req["run_id"] = json!(run_id);
    }
    if !config_path.is_empty() {
        req["config_path"] = json!(config_path);
    }
    if !cmd_service.is_empty() && cmd_service != "__system__" {
        req["service_id"] = json!(cmd_service);
    }
    Some(req)
}

pub(super) fn build_command_render_items_for(
    root: Option<&str>,
    sid: &str,
    run_id: &str,
    snap: &InspectorSnapshot,
) -> (String, usize, usize, Vec<V>) {
    let checks_count = snap
        .commands
        .iter()
        .filter(|c| m::yes(&c["class"]["check"]))
        .count();
    let failed_count = snap
        .commands
        .iter()
        .filter(|c| !m::yes(&c["running"]) && !m::yes(&c["ok"]) && !m::yes(&c["not_run"]))
        .count();
    let resolved_cmd_filter = match snap.cmd_filter.as_str() {
        "checks" if checks_count > 0 => "checks",
        "failed" if failed_count > 0 => "failed",
        _ => "all",
    }
    .to_owned();
    let keys = command_restore_keys(&snap.commands);
    let mut items = Vec::new();
    for (cmd, key) in snap.commands.iter().zip(keys) {
        let keep = match resolved_cmd_filter.as_str() {
            "checks" => m::yes(&cmd["class"]["check"]),
            "failed" => !m::yes(&cmd["running"]) && !m::yes(&cmd["ok"]) && !m::yes(&cmd["not_run"]),
            _ => true,
        };
        if keep {
            let localize = command_localize_payload(cmd, &snap.files, root, sid, run_id);
            items.push(json!({
                "key": key,
                "selected": snap.selected,
                "cmd": cmd,
                "localize": localize,
            }));
        }
    }
    (resolved_cmd_filter, checks_count, failed_count, items)
}

#[cfg(test)]
pub(super) fn build_command_render_items(
    snap: &InspectorSnapshot,
) -> (String, usize, usize, Vec<V>) {
    build_command_render_items_for(None, "", "", snap)
}

fn build_command_details_element(item: &V) -> web_sys::Element {
    let key = m::text(item, "key");
    let selected = m::text(item, "selected");
    let cmd = &item["cmd"];
    let class = &cmd["class"];
    let glyph = if m::yes(&cmd["running"]) {
        "●"
    } else if m::yes(&cmd["not_run"]) || !class["masked"].is_null() {
        "?"
    } else if m::yes(&cmd["ok"]) {
        "✓"
    } else {
        "✗"
    };
    let prefix = if selected == "all" {
        let ag = m::text(cmd, "agent");
        if !ag.is_empty() {
            format!("{} · ", files_map::short_agent_name(&ag))
        } else {
            String::new()
        }
    } else {
        String::new()
    };
    let family = class["family"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|f| format!(" [{f}]"))
        .unwrap_or_default();
    let block = dom::details(
        &format!("{glyph} {prefix}{}{family}", m::text(cmd, "command")),
        &serde_json::to_string_pretty(cmd).unwrap_or_default(),
    );
    let _ = block.set_attribute("data-restore-key", &key);
    if let Some(summary) = block.first_element_child() {
        let _ = summary.set_attribute("data-focus-key", &format!("summary:{key}"));
    }
    if item["localize"].is_object() {
        let actions = dom::el("div", "activity-cmd-actions", "");
        let btn = dom::button(
            "Localize failure on map",
            "activity-localize-failure",
            item["localize"].clone(),
        );
        let _ = btn.class_list().add_1("secondary");
        dom::add(&actions, &btn);
        dom::add(&block, &actions);
    }
    block
}

#[cfg(test)]
pub(super) fn build_failed_command_actions_html_for_test(localize: &V) -> String {
    let esc = super::events_card::escape_html;
    let payload = esc(&localize.to_string());
    format!(
        "<div class=\"activity-cmd-actions\"><button type=\"button\" class=\"button secondary\" data-action=\"activity-localize-failure\" data-payload=\"{payload}\">Localize failure on map</button></div>"
    )
}

fn render_files_pane_dom(root: &str, snap: &InspectorSnapshot) -> web_sys::Element {
    let pane = dom::el("div", "run-pane", "");
    let _ = pane.set_attribute("role", "tabpanel");
    let group_by_service = snap.selected == "all";
    let show_agent_badge = snap.selected == "all";
    let list_mode = snap.files_view == "list";

    let head = dom::el("div", "run-files-head", "");
    let summary = dom::el(
        "p",
        "run-files-summary",
        &files_map::files_summary(&snap.files),
    );
    let _ = summary.set_attribute("id", &format!("{root}-files-summary"));
    dom::add(&head, &summary);
    let seg = dom::el("div", "run-seg", "");
    let _ = seg.set_attribute("id", &format!("{root}-files-seg"));
    let _ = seg.set_attribute("role", "group");
    let _ = seg.set_attribute("aria-label", "File view");
    for (view_id, label, pressed) in [("map", "Map", !list_mode), ("list", "List", list_mode)] {
        let btn = dom::button(
            label,
            "activity-files-view",
            json!({"root": root, "view": view_id}),
        );
        btn.set_class_name("");
        let _ = btn.set_attribute("aria-pressed", truth(pressed));
        let _ = btn.set_attribute("data-focus-key", &format!("view:{view_id}"));
        dom::add(&seg, &btn);
    }
    dom::add(&head, &seg);
    dom::add(&pane, &head);

    if snap.files.is_empty() {
        dom::add(
            &pane,
            &dom::el("p", "run-empty", "Files the agent opens appear here."),
        );
        return pane;
    }

    if !list_mode {
        let (resolved_filter, pills) = offered_filters(&snap.files, &snap.file_filter);
        let filters = dom::el("div", "run-filters", "");
        let _ = filters.set_attribute("role", "group");
        let _ = filters.set_attribute("aria-label", "Show files");
        for pill in &pills {
            let btn = dom::button(
                &pill.label,
                "activity-file-filter",
                json!({"root": root, "filter": pill.id}),
            );
            btn.set_class_name("run-filter");
            let _ = btn.set_attribute("aria-pressed", truth(pill.pressed));
            let _ = btn.set_attribute("data-focus-key", &format!("filter:{}", pill.id));
            dom::add(&filters, &btn);
        }
        dom::add(&pane, &filters);

        let districts = files_map::group_districts(
            &snap.files,
            &resolved_filter,
            snap.running,
            group_by_service,
            show_agent_badge,
        );
        let map = dom::el("div", "run-map", "");
        let _ = map.set_attribute("aria-live", "off");
        let main = dom::el("div", "run-map-main", "");
        for d in &districts {
            dom::add(
                &main,
                &files_map::district_element(d, "activity-file", Some(root), "true"),
            );
        }
        dom::add(&map, &main);
        dom::add(&pane, &map);
        dom::add(&pane, &files_map::legend_element());
    } else {
        let folders = files_map::group_list_folders(&snap.files, group_by_service);
        let tree = dom::el("div", "run-tree", "");
        let _ = tree.set_attribute("id", &format!("{root}-tree"));
        let _ = tree.set_attribute("role", "tree");
        let _ = tree.set_attribute("aria-label", "Files named in tool calls");
        for item in build_folder_render_items(&folders, show_agent_badge) {
            let details = build_folder_details_element(root, &item);
            let _ = details.set_attribute("data-key", &m::text(&item, "key"));
            let _ = details.set_attribute("data-signature", &dom::json_signature(&item));
            dom::add(&tree, &details);
        }
        dom::add(&pane, &tree);
    }
    pane
}

fn render_plan_pane_dom(snap: &InspectorSnapshot) -> web_sys::Element {
    let pane = dom::el("div", "run-pane", "");
    let _ = pane.set_attribute("role", "tabpanel");
    if snap.plan_items.is_empty() {
        dom::add(
            &pane,
            &dom::el("p", "run-empty", "No plan items recorded for this scope."),
        );
        return pane;
    }
    for item in &snap.plan_items {
        let ag = m::text(item, "agent");
        let st = status(item["status"].as_str().unwrap_or("pending"));
        let txt = m::text(item, "text");
        let line = if snap.selected == "all" && !ag.is_empty() && !m::yes(&item["wave_step"]) {
            format!("{} · {st} · {txt}", files_map::short_agent_name(&ag))
        } else {
            format!("{st} · {txt}")
        };
        dom::add(&pane, &dom::el("p", "run-plan-item", &line));
    }
    pane
}

fn populate_command_filters(
    filters: &web_sys::Element,
    root: &str,
    resolved_cmd_filter: &str,
    checks_count: usize,
    failed_count: usize,
) {
    let sig = json!([resolved_cmd_filter, checks_count, failed_count]).to_string();
    if filters.get_attribute("data-signature").as_deref() == Some(sig.as_str()) {
        return;
    }
    let _ = filters.set_attribute("data-signature", &sig);
    filters.set_text_content(None);
    for (fid, label) in [
        ("all", Some("All".to_owned())),
        (
            "checks",
            (checks_count > 0).then(|| format!("Checks {checks_count}")),
        ),
        (
            "failed",
            (failed_count > 0).then(|| format!("Failed {failed_count}")),
        ),
    ] {
        let Some(label) = label else { continue };
        let btn = dom::button(
            &label,
            "activity-cmd-filter",
            json!({"root": root, "filter": fid}),
        );
        btn.set_class_name("run-filter");
        let _ = btn.set_attribute("aria-pressed", truth(fid == resolved_cmd_filter));
        let _ = btn.set_attribute("data-focus-key", &format!("cmdfilter:{fid}"));
        dom::add(filters, &btn);
    }
}

fn render_commands_pane_dom(root: &str, snap: &InspectorSnapshot) -> web_sys::Element {
    let pane = dom::el("div", "run-pane", "");
    let _ = pane.set_attribute("role", "tabpanel");
    if snap.commands.is_empty() {
        dom::add(
            &pane,
            &dom::el(
                "p",
                "run-empty",
                "No shell commands or checks recorded yet.",
            ),
        );
        return pane;
    }

    let p = panel(root);
    let (resolved_cmd_filter, checks_count, failed_count, items) =
        build_command_render_items_for(Some(root), &p.sid, &p.run_id, snap);

    let filters = dom::el("div", "run-filters", "");
    let _ = filters.set_attribute("id", &format!("{root}-commands-filters"));
    let _ = filters.set_attribute("role", "group");
    let _ = filters.set_attribute("aria-label", "Filter commands");
    populate_command_filters(
        &filters,
        root,
        &resolved_cmd_filter,
        checks_count,
        failed_count,
    );
    dom::add(&pane, &filters);

    let list = dom::el("div", "run-commands-list", "");
    let _ = list.set_attribute("id", &format!("{root}-commands-list"));
    for item in &items {
        let block = build_command_details_element(item);
        let _ = block.set_attribute("data-key", &m::text(item, "key"));
        let _ = block.set_attribute("data-signature", &dom::json_signature(item));
        dom::add(&list, &block);
    }
    dom::add(&pane, &list);
    pane
}

#[allow(dead_code)]
pub(crate) fn render_into(root: &str) {
    render(root);
}

pub(super) fn capture_details_open_state(root_el: &web_sys::Element) -> BTreeMap<String, bool> {
    let mut states = BTreeMap::new();
    if let Ok(nodes) = root_el.query_selector_all("details[data-restore-key]") {
        for i in 0..nodes.length() {
            if let Some(el) = nodes
                .item(i)
                .and_then(|v| v.dyn_into::<web_sys::Element>().ok())
            {
                if let Some(key) = el.get_attribute("data-restore-key") {
                    states.insert(key, el.has_attribute("open"));
                }
            }
        }
    }
    states
}

pub(super) fn apply_details_open_state(
    root_el: &web_sys::Element,
    states: &BTreeMap<String, bool>,
) {
    if states.is_empty() {
        return;
    }
    if let Ok(nodes) = root_el.query_selector_all("details[data-restore-key]") {
        for i in 0..nodes.length() {
            if let Some(el) = nodes
                .item(i)
                .and_then(|v| v.dyn_into::<web_sys::Element>().ok())
            {
                if let Some(was_open) = el
                    .get_attribute("data-restore-key")
                    .and_then(|key| states.get(&key).copied())
                {
                    if was_open {
                        let _ = el.set_attribute("open", "");
                    } else {
                        let _ = el.remove_attribute("open");
                    }
                }
            }
        }
    }
}

pub(crate) fn render(root: &str) {
    let snap = snapshot_for_root(root);
    if !cfg!(target_arch = "wasm32") {
        return;
    }
    let Some(host) = dom::get(root) else { return };
    let p = panel(root);

    let focused_key = dom::document()
        .active_element()
        .filter(|el| host.contains(Some(el)))
        .and_then(|el| el.get_attribute("data-focus-key"));

    let previous_details = capture_details_open_state(&host);

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

    let commands_list_id = format!("{root}-commands-list");
    let tree_id = format!("{root}-tree");
    let pane_id = format!("{root}-timeline-pane");
    let timeline_id = format!("{root}-timeline");
    let cards_id = format!("{root}-cards");
    let heading_id = format!("{root}-timeline-heading");
    let tabs_id = format!("{root}-timeline-tabs");
    let follow_id = format!("{root}-follow-input");

    let reuse_shell = dom::get(&timeline_id).is_some()
        && dom::get(&cards_id).is_some()
        && dom::get(&pane_id).is_some();

    if !reuse_shell {
        host.set_text_content(None);

        let label = dom::el("label", "checkbox agent-follow", "");
        let input = dom::el("input", "", "");
        let _ = input.set_attribute("id", &follow_id);
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
        let _ = cards.set_attribute("id", &cards_id);
        dom::add(&host, &cards);

        let timeline = dom::el("div", "agent-timeline", "");
        let _ = timeline.set_attribute("id", &timeline_id);
        let _ = timeline.set_attribute("aria-live", "polite");
        let heading = dom::el("div", "agent-timeline-heading", "");
        let _ = heading.set_attribute("id", &heading_id);
        dom::add(&timeline, &heading);
        let tabs_bar = dom::el("div", "run-tabs", "");
        let _ = tabs_bar.set_attribute("id", &tabs_id);
        let _ = tabs_bar.set_attribute("role", "tablist");
        let _ = tabs_bar.set_attribute("aria-label", "Agent inspector views");
        dom::add(&timeline, &tabs_bar);
        let pane_host = dom::el("div", "", "");
        let _ = pane_host.set_attribute("id", &pane_id);
        dom::add(&timeline, &pane_host);
        dom::add(&host, &timeline);
    } else if let Some(input) =
        dom::get(&follow_id).and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok())
    {
        if input.checked() != p.follow {
            input.set_checked(p.follow);
        }
    }

    let mut card_items = Vec::new();
    if snap.show_all_card {
        card_items.push(json!({
            "key": "all",
            "selected": snap.selected == "all",
            "files_count": snap.files.len(),
            "items": snap.items,
        }));
    }
    for a in &snap.items {
        let sel_key = selection(a);
        card_items.push(json!({
            "key": sel_key,
            "selected": sel_key == snap.selected,
            "agent": a,
        }));
    }
    dom::keyed_render(
        &cards_id,
        &card_items,
        |item| m::text(item, "key"),
        |item| {
            if item["key"] == "all" {
                render_all_agents_card(
                    root,
                    &m::array(&item["items"]),
                    item["files_count"].as_u64().unwrap_or(0) as usize,
                    m::yes(&item["selected"]),
                )
            } else {
                render_agent_card(root, &item["agent"], m::yes(&item["selected"]))
            }
        },
    );

    if let Some(heading) = dom::get(&heading_id) {
        let heading_sig = if snap.selected == "all" {
            let ran_count = snap.items.iter().filter(|a| a["key"].is_string()).count();
            json!(["all", ran_count.max(snap.items.len()), snap.files.len()])
        } else if let Some(a) = snap.items.iter().find(|i| selection(i) == snap.selected) {
            let title = a["label"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| a["agent"].as_str())
                .unwrap_or("Agent");
            let meta = [
                m::summary(&m::provider(&m::text(a, "provider"))),
                m::text(a, "root"),
            ]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
            json!(["one", title, meta])
        } else {
            json!(["empty", root])
        }
        .to_string();
        if heading.get_attribute("data-signature").as_deref() != Some(heading_sig.as_str()) {
            let _ = heading.set_attribute("data-signature", &heading_sig);
            heading.set_text_content(None);
            if snap.selected == "all" {
                dom::add(&heading, &dom::el("strong", "", "All agents"));
                let ran_count = snap.items.iter().filter(|a| a["key"].is_string()).count();
                dom::add(
                    &heading,
                    &dom::el(
                        "span",
                        "",
                        &format!(
                            "{} · {}",
                            count_words(ran_count.max(snap.items.len()) as u64, "agent"),
                            count_words(snap.files.len() as u64, "file")
                        ),
                    ),
                );
            } else if let Some(a) = snap.items.iter().find(|i| selection(i) == snap.selected) {
                let title = a["label"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .or_else(|| a["agent"].as_str())
                    .unwrap_or("Agent");
                dom::add(&heading, &dom::el("strong", "", title));
                let meta = [
                    m::summary(&m::provider(&m::text(a, "provider"))),
                    m::text(a, "root"),
                ]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
                if !meta.is_empty() {
                    dom::add(&heading, &dom::el("span", "", &meta));
                }
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
        }
    }

    if let Some(tabs_bar) = dom::get(&tabs_id) {
        let tabs_sig = json!([
            snap.tab,
            snap.files.len(),
            snap.plan_items.len(),
            snap.commands.len()
        ])
        .to_string();
        if tabs_bar.get_attribute("data-signature").as_deref() != Some(tabs_sig.as_str()) {
            let _ = tabs_bar.set_attribute("data-signature", &tabs_sig);
            tabs_bar.set_text_content(None);
            for (tab_id, title, badge_count) in [
                ("timeline", "Timeline", None),
                ("files", "Files", Some(snap.files.len())),
                ("plan", "Plan", Some(snap.plan_items.len())),
                ("commands", "Commands", Some(snap.commands.len())),
            ] {
                let active_tab = snap.tab == tab_id;
                let btn = dom::button("", "activity-tab", json!({"root": root, "tab": tab_id}));
                btn.set_class_name("run-tab");
                let _ = btn.set_attribute("role", "tab");
                let _ = btn.set_attribute("aria-selected", truth(active_tab));
                let _ = btn.set_attribute("tabindex", roving_tabindex(active_tab));
                let _ = btn.set_attribute("data-focus-key", &format!("tab:{tab_id}"));
                if let Some(n) = badge_count {
                    let _ = btn.set_attribute("aria-label", &format!("{title} ({n})"));
                    btn.set_text_content(Some(&format!("{title} ")));
                    dom::add(&btn, &dom::el("span", "run-badge", &n.to_string()));
                } else {
                    btn.set_text_content(Some(title));
                }
                dom::add(&tabs_bar, &btn);
            }
            dom::sync_roving_widget(&tabs_bar);
        }
    }

    let Some(pane_host) = dom::get(&pane_id) else {
        return;
    };

    if snap.tab == "commands" && !snap.commands.is_empty() && dom::get(&commands_list_id).is_some()
    {
        let p = panel(root);
        let (resolved_cmd_filter, checks_count, failed_count, items) =
            build_command_render_items_for(Some(root), &p.sid, &p.run_id, &snap);
        if let Some(filters) = dom::get(&format!("{root}-commands-filters")) {
            populate_command_filters(
                &filters,
                root,
                &resolved_cmd_filter,
                checks_count,
                failed_count,
            );
        }
        dom::keyed_render(
            &commands_list_id,
            &items,
            |item| m::text(item, "key"),
            build_command_details_element,
        );
    } else if snap.tab == "files"
        && snap.files_view == "list"
        && !snap.files.is_empty()
        && dom::get(&tree_id).is_some()
    {
        let summary_text = files_map::files_summary(&snap.files);
        let summary_id = format!("{root}-files-summary");
        if dom::get(&summary_id)
            .and_then(|e| e.text_content())
            .as_deref()
            != Some(summary_text.as_str())
        {
            dom::text(&summary_id, &summary_text);
        }
        let group_by_service = snap.selected == "all";
        let show_agent_badge = snap.selected == "all";
        let folders = files_map::group_list_folders(&snap.files, group_by_service);
        let items = build_folder_render_items(&folders, show_agent_badge);
        let root_owned = root.to_owned();
        dom::keyed_render(
            &tree_id,
            &items,
            |item| m::text(item, "key"),
            move |item| build_folder_details_element(&root_owned, item),
        );
        if let Some(tree) = dom::get(&tree_id) {
            apply_details_open_state(&tree, &previous_details);
        }
    } else {
        match snap.tab.as_str() {
            "files" => {
                pane_host.set_text_content(None);
                let pane = render_files_pane_dom(root, &snap);
                apply_details_open_state(&pane, &previous_details);
                dom::add(&pane_host, &pane);
            }
            "plan" => {
                pane_host.set_text_content(None);
                dom::add(&pane_host, &render_plan_pane_dom(&snap));
            }
            "commands" => {
                pane_host.set_text_content(None);
                let pane = render_commands_pane_dom(root, &snap);
                apply_details_open_state(&pane, &previous_details);
                dom::add(&pane_host, &pane);
            }
            _ => {
                let timeline_sig = json!([
                    snap.selected,
                    snap.running,
                    snap.timeline_events,
                    snap.items
                ])
                .to_string();
                let current_timeline_sig = pane_host
                    .query_selector(".agent-events")
                    .ok()
                    .flatten()
                    .and_then(|el| el.get_attribute("data-signature"));
                if current_timeline_sig.as_deref() == Some(timeline_sig.as_str()) {
                    // Nothing in the timeline changed; preserve DOM and scroll untouched.
                } else {
                    pane_host.set_text_content(None);
                    let list = dom::el("ol", "agent-events", "");
                    let _ = list.set_attribute("data-signature", &timeline_sig);
                    if snap.timeline_events.is_empty() {
                        let msg = if snap.selected == "all" {
                            if snap.running {
                                "Waiting for the first agent action…"
                            } else {
                                "No timeline events recorded in this launch."
                            }
                        } else if let Some(a) =
                            snap.items.iter().find(|i| selection(i) == snap.selected)
                        {
                            if a["status"] == "running" {
                                "Waiting for the first action…"
                            } else if a["earlier"] == true {
                                "This agent completed in an earlier launch; choose that launch above."
                            } else if a["key"].is_null() {
                                "This agent has not run in this launch."
                            } else {
                                "No activity was recorded for this agent."
                            }
                        } else {
                            "Select an agent card to follow its work."
                        };
                        dom::add(&list, &dom::el("li", "agent-placeholder", msg));
                    } else {
                        for item in &snap.timeline_events {
                            let kind = m::text(item, "type");
                            let row = dom::el(
                                "li",
                                &format!(
                                    "agent-event {kind}{}",
                                    if item["ok"] == false { " failed" } else { "" }
                                ),
                                "",
                            );
                            let ev_id = m::string(&item["event_id"]);
                            if !ev_id.is_empty() {
                                let _ = row.set_attribute("data-event-id", &ev_id);
                            }
                            let ev_path = m::text(item, "path");
                            if !ev_path.is_empty() {
                                let _ = row.set_attribute("data-file-path", &ev_path);
                            }
                            dom::add(
                                &row,
                                &dom::el("time", "agent-event-time", &clock(&item["at"])),
                            );
                            let base_title = if kind == "tool" {
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
                            let title = if snap.selected == "all" {
                                let ag = m::text(item, "agent");
                                if !ag.is_empty() {
                                    format!("{} · {base_title}", files_map::short_agent_name(&ag))
                                } else {
                                    base_title
                                }
                            } else {
                                base_title
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
                                    &describe(item),
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
                    }

                    if snap.selected != "all" {
                        if let Some(a) = snap
                            .items
                            .iter()
                            .find(|i| selection(i) == snap.selected && i["key"].is_string())
                        {
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
                        }
                    }

                    dom::add(&pane_host, &list);
                    if let Ok(e) = list.dyn_into::<web_sys::HtmlElement>() {
                        if let Some((top, stick)) = scroll {
                            e.set_scroll_top(if stick { e.scroll_height() } else { top });
                        }
                    }
                }
            }
        }
    }

    if let Some(card_key) = p.card_key.as_deref() {
        if snap.tab == "files" && snap.files_view == "map" {
            if let Some(entry) = snap.files.iter().find(|e| e.key() == card_key) {
                let actions = system_card_actions(root, entry);
                let owner = owner_tag(root);
                DISMISS_ON_GRAPH_CLICK.with(|cb| {
                    files_map::fill_card_element(
                        CARD_ID,
                        &owner,
                        entry,
                        &actions,
                        Some(("inspect-system-file-graph", cb)),
                    );
                });
            } else {
                hide_card_for_root(root, false);
            }
        } else {
            hide_card_for_root(root, false);
        }
    }

    if let Some(fk) = focused_key {
        let focus_still_in_host = dom::document()
            .active_element()
            .is_some_and(|el| host.contains(Some(&el)));
        if !focus_still_in_host {
            let restored = dom::all(&format!("#{root} [data-focus-key]"))
                .into_iter()
                .find(|el| el.get_attribute("data-focus-key").as_deref() == Some(fk.as_str()));
            if let Some(node) = restored {
                focus_element_no_scroll(&node);
            }
        }
    }
}

fn scroll_to_timeline_event(root: &str, event_id: &V, path: &str) {
    if !cfg!(target_arch = "wasm32") {
        return;
    }
    let ev_str = m::string(event_id);
    if !ev_str.is_empty() {
        let found = dom::all(&format!("#{root} .agent-event[data-event-id]"))
            .into_iter()
            .find(|el| el.get_attribute("data-event-id").as_deref() == Some(ev_str.as_str()));
        if let Some(node) = found {
            node.scroll_into_view();
            return;
        }
    }
    if !path.is_empty() {
        let found = dom::all(&format!("#{root} .agent-event[data-file-path]"))
            .into_iter()
            .find(|el| el.get_attribute("data-file-path").as_deref() == Some(path));
        if let Some(node) = found {
            node.scroll_into_view();
        }
    }
}

#[allow(dead_code)]
pub(crate) fn handle_click(target: &web_sys::Element) -> bool {
    let Some(el) = target.closest("[data-action]").ok().flatten() else {
        return false;
    };
    let action = el.get_attribute("data-action").unwrap_or_default();
    let payload = el
        .get_attribute("data-payload")
        .and_then(|raw| serde_json::from_str::<V>(&raw).ok())
        .unwrap_or(V::Null);
    event("click", &action, &payload, false)
}

#[allow(dead_code)]
pub(crate) fn dismiss_file_card(restore: bool) -> bool {
    hide_any_activity_card(restore)
}

#[allow(dead_code)]
pub(crate) fn escape() -> bool {
    hide_any_activity_card(true)
}

#[allow(dead_code)]
pub(crate) fn pointer_down(target: &web_sys::Element) {
    if open_card_root().is_some()
        && target
            .closest("#run-file-card, .run-file")
            .ok()
            .flatten()
            .is_none()
    {
        hide_any_activity_card(false);
    }
}

pub fn event(kind: &str, action: &str, payload: &V, checked: bool) -> bool {
    match (kind, action) {
        ("click", "inspect-system-file-graph") => {
            hide_any_activity_card(false);
            false
        }
        ("click", "agent-select") => {
            let root = m::text(payload, "root");
            hide_card_for_root(&root, false);
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
        ("click", "activity-tab") => {
            let root = m::text(payload, "root");
            let tab = m::text(payload, "tab");
            hide_card_for_root(&root, false);
            edit(&root, |p| p.tab = tab);
            render(&root);
            true
        }
        ("click", "activity-files-view") => {
            let root = m::text(payload, "root");
            let view = normalize_view(&m::text(payload, "view")).to_owned();
            hide_card_for_root(&root, false);
            edit(&root, |p| p.files_view = view);
            render(&root);
            true
        }
        ("click", "activity-file-filter") => {
            let root = m::text(payload, "root");
            let filter = m::text(payload, "filter");
            edit(&root, |p| p.file_filter = filter);
            render(&root);
            true
        }
        ("click", "activity-cmd-filter") => {
            let root = m::text(payload, "root");
            let filter = m::text(payload, "filter");
            edit(&root, |p| p.cmd_filter = filter);
            render(&root);
            true
        }
        ("click", "activity-file") => {
            let root = m::text(payload, "root");
            let key = m::text(payload, "key");
            let current = panel(&root).card_key;
            if current.as_deref() == Some(key.as_str()) {
                hide_card_for_root(&root, true);
            } else {
                open_activity_file_card(&root, &key);
            }
            true
        }
        ("click", "activity-reveal") => {
            let root = m::text(payload, "root");
            let target_agent = m::text(payload, "agent");
            let path = m::text(payload, "path");
            hide_card_for_root(&root, false);
            edit(&root, |p| {
                p.tab = "timeline".into();
                if p.selected != "all" && !target_agent.is_empty() {
                    p.selected = target_agent;
                    p.follow = false;
                }
            });
            render(&root);
            scroll_to_timeline_event(&root, &payload["event_id"], &path);
            true
        }
        ("click", "activity-add-path") => {
            hide_any_activity_card(false);
            insert_system_task_path(payload.as_str().unwrap_or(""));
            true
        }
        ("click", "activity-copy-path") => {
            copy_path_to_clipboard(payload.as_str().unwrap_or(""));
            true
        }
        ("click", "activity-localize-failure") => {
            super::events_card::localize_failure_payload(payload);
            true
        }
        _ => false,
    }
}

#[allow(dead_code)]
pub fn initialize() {
    if m::yes(&m::data("activity-initialized")) {
        return;
    }
    m::set("activity-initialized", json!(true));
    crate::session::on_project_change(crate::activity::on_global_project_change);
}

#[allow(dead_code)]
pub(crate) fn on_global_project_change(project: &str) {
    m::set("activity-global-project", json!(project));
    if cfg!(target_arch = "wasm32") {
        hide_any_activity_card(false);
    }
}

pub(super) fn roving_tabindex(active: bool) -> &'static str {
    if active {
        "0"
    } else {
        "-1"
    }
}
