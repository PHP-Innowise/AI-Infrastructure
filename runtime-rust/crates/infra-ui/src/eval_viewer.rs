//! Rust/Wasm port of the attributed skill-creator eval viewer.
//! All review state, rendering, keyboard handling, feedback and benchmark logic live here.
use crate::dom;
use serde_json::{json, Value as V};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
};
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::Element;
#[derive(Default)]
struct State {
    data: V,
    index: usize,
    feedback: BTreeMap<String, String>,
    visited: BTreeSet<usize>,
    pending: Option<V>,
    saving: bool,
}
thread_local! {static STATE:RefCell<State>=RefCell::new(State::default());}
fn read<T>(f: impl FnOnce(&State) -> T) -> T {
    STATE.with(|s| f(&s.borrow()))
}
fn write<T>(f: impl FnOnce(&mut State) -> T) -> T {
    STATE.with(|s| f(&mut s.borrow_mut()))
}
fn text(v: &V, key: &str) -> String {
    v[key].as_str().map(str::to_owned).unwrap_or_default()
}
fn show(id: &str, visible: bool) {
    dom::hidden(id, !visible);
}

fn ensure_eval_viewer_styles() {
    if dom::get("eval-viewer-a11y").is_some() {
        return;
    }
    let doc = dom::document();
    if let Ok(style) = doc.create_element("style") {
        style.set_id("eval-viewer-a11y");
        style.set_text_content(Some(
            "[hidden]{display:none!important;}.eval-output-error{color:var(--error,var(--red));}",
        ));
        if let Some(parent) = doc
            .query_selector("head")
            .ok()
            .flatten()
            .or_else(|| doc.document_element())
        {
            let _ = parent.append_child(&style);
        }
    }
}

fn add_eval_set_item(data: &mut V) -> usize {
    if !data.is_object() {
        *data = json!({});
    }
    if !data["items"].is_array() {
        data["items"] = json!([]);
    }
    let Some(items) = data.get_mut("items").and_then(V::as_array_mut) else {
        return 0;
    };
    let index = items.len();
    items.push(json!({"query": "", "should_trigger": true}));
    index
}
fn class(id: &str, key: &str, on: bool) {
    dom::class(id, key, on);
}
fn toggle(id: &str) {
    if let Some(e) = dom::get(id) {
        let _ = e.class_list().toggle("open");
    }
}
fn add(parent: &Element, tag: &str, class: &str, text: &str) -> Element {
    let e = dom::el(tag, class, text);
    dom::add(parent, &e);
    e
}
fn list(v: &V) -> &[V] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn display(v: &V) -> String {
    if v.is_null() {
        "—".into()
    } else {
        v.as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| v.to_string())
    }
}
fn number(v: &V, key: &str) -> f64 {
    v[key].as_f64().unwrap_or(0.0)
}
fn title(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut c = word.chars();
            c.next()
                .map(|a| a.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}
/// The configurations a run id may name: the skill benchmark's and the
/// arms of the SkillCache evaluation. A baseline is what the others are
/// measured against.
const CONFIGURATIONS: [&str; 10] = [
    "without_skill",
    "with_skill",
    "old_skill",
    "new_skill",
    "skillcache_executor",
    "skillcache_agent",
    "skillcache_no_replay",
    "written_procedure",
    "raw_trace",
    "cold",
];
fn configuration_of(id: &str) -> Option<&'static str> {
    CONFIGURATIONS.into_iter().find(|name| {
        id.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .any(|segment| segment == *name)
    })
}
fn is_baseline(configuration: &str) -> bool {
    ["without_skill", "old_skill", "cold"].contains(&configuration)
}
/// The summary rows of a benchmark: key, label, shown as a percentage, the
/// delta's suffix, decimals of a plain number, and whether a lower value is
/// better. A row is shown when either configuration has it. The rows after
/// `tokens` are the SkillCache evaluation's: what a turn cost, how often a
/// compiled step was handed back, and what compiling cost.
const METRICS: [(&str, &str, bool, &str, usize, bool); 8] = [
    ("pass_rate", "Pass Rate", true, "", 1, false),
    ("time_seconds", "Time (s)", false, "s", 1, true),
    ("tokens", "Tokens", false, "", 1, true),
    ("model_calls", "Model calls", false, "", 1, true),
    ("deopt_rate", "Steps handed back", true, "", 1, true),
    ("list_price_usd", "List price (USD)", false, "", 4, true),
    ("compile_usd", "Compilation (USD)", false, "", 4, true),
    ("breakeven_reuses", "Break-even reuses", false, "", 0, true),
];
fn link_uri(file: &V) -> String {
    let uri = text(file, "data_uri");
    if uri.starts_with("data:") {
        return uri;
    }
    let b64 = text(file, "data_b64");
    if !b64.is_empty() {
        return format!("data:application/octet-stream;base64,{b64}");
    }
    if file["type"] == "text" {
        return format!(
            "data:text/plain;charset=utf-8,{}",
            dom::enc(&text(file, "content"))
        );
    }
    "#".into()
}
fn outputs(container: &Element, files: &[V]) {
    container.set_text_content(None);
    if files.is_empty() {
        add(container, "div", "empty-state", "No output files");
        return;
    }
    for file in files {
        let row = add(container, "div", "output-file", "");
        let header = add(&row, "div", "output-file-header", "");
        add(&header, "span", "", &text(file, "name"));
        let download = add(&header, "a", "dl-btn", "Download");
        let _ = download.set_attribute("download", &text(file, "name"));
        let _ = download.set_attribute("href", &link_uri(file));
        let content = add(&row, "div", "output-file-content", "");
        match file["type"].as_str().unwrap_or("") {
            "text" | "error" => {
                let pre = add(&content, "pre", "", &text(file, "content"));
                if file["type"] == "error" {
                    let _ = pre.class_list().add_1("eval-output-error");
                }
            }
            "image" => {
                let img = add(&content, "img", "", "");
                let uri = text(file, "data_uri");
                if uri.starts_with("data:image/") {
                    let _ = img.set_attribute("src", &uri);
                }
                let _ = img.set_attribute("alt", &text(file, "name"));
            }
            "pdf" => {
                let iframe = add(&content, "iframe", "", "");
                let uri = text(file, "data_uri");
                if uri.starts_with("data:application/pdf;") {
                    let _ = iframe.set_attribute("src", &uri);
                }
                let _ = iframe.set_attribute("title", &text(file, "name"));
            }
            "xlsx" => {
                if let Some(error) = file["render_error"].as_str() {
                    add(
                        &content,
                        "pre",
                        "",
                        &format!("Error rendering spreadsheet: {error}"),
                    );
                } else {
                    let sheets = list(&file["worksheets"]);
                    if sheets.is_empty() {
                        add(&content, "p", "", "No worksheet data");
                    }
                    for sheet in sheets {
                        if sheets.len() > 1 {
                            add(
                                &content,
                                "div",
                                "sheet-label",
                                &format!("Sheet: {}", text(sheet, "name")),
                            );
                        }
                        render_sheet(&content, sheet);
                    }
                }
            }
            "binary" => {
                let link = add(
                    &content,
                    "a",
                    "download-link",
                    &format!("Download {}", text(file, "name")),
                );
                let _ = link.set_attribute("href", &link_uri(file));
                let _ = link.set_attribute("download", &text(file, "name"));
            }
            _ => {}
        }
    }
}
fn grades(run: &V) {
    let grading = &run["grading"];
    show("grades-section", !grading.is_null());
    dom::clear("grades-content");
    class("grades-content", "open", false);
    class("grades-arrow", "open", false);
    if grading.is_null() {
        return;
    }
    let Some(root) = dom::get("grades-content") else {
        return;
    };
    let wrap = add(&root, "div", "grades-wrapper", "");
    let summary = &grading["summary"];
    let rate = summary["pass_rate"].as_f64();
    let row = add(&wrap, "div", "grades-summary", "");
    let cls = if rate.unwrap_or(0.0) >= 0.8 {
        "grade-pass"
    } else if rate.unwrap_or(0.0) < 0.5 {
        "grade-fail"
    } else {
        ""
    };
    add(
        &row,
        "span",
        &format!("grade-badge {cls}"),
        &rate
            .map(|r| format!("{:.0}%", r * 100.0))
            .unwrap_or_else(|| "?".into()),
    );
    add(
        &row,
        "span",
        "",
        &format!(
            "{} passed, {} failed of {}",
            summary["passed"].as_u64().unwrap_or(0),
            summary["failed"].as_u64().unwrap_or(0),
            summary["total"].as_u64().unwrap_or(0)
        ),
    );
    let ul = add(&wrap, "ul", "assertion-list", "");
    for exp in list(&grading["expectations"]) {
        let passed = exp["passed"] == true;
        let li = add(&ul, "li", "assertion-item", "");
        add(
            &li,
            "span",
            if passed {
                "assertion-status pass"
            } else {
                "assertion-status fail"
            },
            if passed { "✓" } else { "✗" },
        );
        add(&li, "span", "", &text(exp, "text"));
        if let Some(evidence) = exp["evidence"].as_str().filter(|s| !s.is_empty()) {
            add(&li, "div", "assertion-evidence", evidence);
        }
    }
}
fn show_run(index: usize) {
    let Some((run, data, feedback, visited, total)) = write(|s| {
        let runs = list(&s.data["runs"]);
        let run = runs.get(index)?.clone();
        let total = runs.len();
        s.index = index;
        s.visited.insert(index);
        Some((
            run,
            s.data.clone(),
            s.feedback.clone(),
            s.visited.len(),
            total,
        ))
    }) else {
        return;
    };
    dom::text("progress", &format!("{} of {total}", index + 1));
    dom::text("prompt-text", &text(&run, "prompt"));
    let id = text(&run, "id");
    let config = configuration_of(&id);
    show("config-badge", config.is_some());
    if let Some(c) = config {
        dom::text("config-badge", &c.replace('_', " "));
        if let Some(e) = dom::get("config-badge") {
            e.set_class_name(if is_baseline(c) {
                "config-badge config-baseline"
            } else {
                "config-badge config-primary"
            });
        }
    }
    if let Some(e) = dom::get("outputs-body") {
        outputs(&e, list(&run["outputs"]));
    }
    let previous = list(&data["previous_outputs"][&id]);
    show("prev-outputs-section", !previous.is_empty());
    class("prev-outputs-content", "open", false);
    class("prev-outputs-arrow", "open", false);
    if let Some(e) = dom::get("prev-outputs-content") {
        outputs(&e, previous);
    }
    grades(&run);
    let old = data["previous_feedback"][&id].as_str().unwrap_or("");
    show("prev-feedback", !old.is_empty());
    dom::text("prev-feedback-text", old);
    dom::set_value(
        "feedback",
        feedback.get(&id).map(String::as_str).unwrap_or(""),
    );
    dom::text("feedback-status", "");
    dom::disabled("prev-btn", index == 0);
    dom::disabled("next-btn", index + 1 >= total);
    class("done-btn", "ready", visited >= total);
    if let Some(e) = dom::all(".main").first() {
        e.set_scroll_top(0);
    }
}
fn capture_current() {
    let content = dom::value("feedback");
    write(|s| {
        let id = list(&s.data["runs"]).get(s.index).map(|r| text(r, "id"));
        if let Some(id) = id {
            if content.trim().is_empty() {
                s.feedback.remove(&id);
            } else {
                s.feedback.insert(id, content);
            }
        }
    });
}
fn payload(complete: bool) -> V {
    let timestamp = js_sys::Date::new_0()
        .to_iso_string()
        .as_string()
        .unwrap_or_default();
    read(|s| {
        let reviews = if complete {
            list(&s.data["runs"]).iter().map(|r|{let id=text(r,"id");json!({"run_id":id,"feedback":s.feedback.get(&id).cloned().unwrap_or_default(),"timestamp":timestamp})}).collect::<Vec<_>>()
        } else {
            s.feedback
                .iter()
                .filter(|(_, f)| !f.trim().is_empty())
                .map(|(id, f)| json!({"run_id":id,"feedback":f,"timestamp":timestamp}))
                .collect()
        };
        json!({"reviews":reviews,"status":if complete{"complete"}else{"in_progress"}})
    })
}
async fn request(method: &str, body: Option<&V>) -> Result<V, JsValue> {
    let options = web_sys::RequestInit::new();
    options.set_method(method);
    options.set_credentials(web_sys::RequestCredentials::SameOrigin);
    options.set_cache(web_sys::RequestCache::NoStore);
    if let Some(body) = body {
        options.set_body(&JsValue::from_str(&body.to_string()));
    }
    let req = web_sys::Request::new_with_str_and_init("/api/feedback", &options)?;
    req.headers().set("Accept", "application/json")?;
    if body.is_some() {
        req.headers().set("Content-Type", "application/json")?;
    }
    let response = JsFuture::from(dom::window().fetch_with_request(&req))
        .await?
        .dyn_into::<web_sys::Response>()?;
    if !response.ok() {
        return Err(JsValue::from_str(&format!(
            "Feedback HTTP {}",
            response.status()
        )));
    }
    let raw = JsFuture::from(response.text()?)
        .await?
        .as_string()
        .unwrap_or_default();
    serde_json::from_str(&raw).map_err(|_| JsValue::from_str("Invalid feedback response"))
}
fn download_feedback(data: &V) {
    let a = dom::el("a", "", "");
    let _ = a.set_attribute(
        "href",
        &format!(
            "data:application/json;charset=utf-8,{}",
            dom::enc(&serde_json::to_string_pretty(data).unwrap_or_default())
        ),
    );
    let _ = a.set_attribute("download", "feedback.json");
    if let Some(body) = dom::document().body() {
        let _ = body.append_child(&a);
    }
    if let Some(a) = a.dyn_ref::<web_sys::HtmlElement>() {
        a.click();
    }
    a.remove();
}
fn save(complete: bool) {
    if complete {
        dom::cancel_timer("eval-feedback-save");
    }
    capture_current();
    let data = payload(complete);
    let launch = write(|s| {
        s.pending = Some(data);
        if s.saving {
            false
        } else {
            s.saving = true;
            true
        }
    });
    if !launch {
        return;
    }
    spawn_local(async {
        loop {
            let data = write(|s| s.pending.take());
            let Some(data) = data else {
                write(|s| s.saving = false);
                break;
            };
            let complete = data["status"] == "complete";
            let standalone = read(|s| s.data["standalone"] == true);
            let result = if standalone {
                Err(JsValue::from_str("Standalone viewer"))
            } else {
                request("POST", Some(&data)).await
            };
            match result {
                Ok(_) => {
                    dom::text("feedback-status", "Saved");
                    if complete {
                        class("done-overlay", "visible", true);
                    }
                }
                Err(_) => {
                    dom::text("feedback-status", "Will download on submit");
                    if complete {
                        download_feedback(&data);
                        class("done-overlay", "visible", true);
                    }
                }
            }
        }
    });
}
fn navigate(delta: i32) {
    save(false);
    let next = read(|s| {
        let n = s.index as i64 + delta as i64;
        if n >= 0 && (n as usize) < list(&s.data["runs"]).len() {
            Some(n as usize)
        } else {
            None
        }
    });
    if let Some(index) = next {
        show_run(index);
    }
}
fn switch_view(view: &str) {
    for e in dom::all(".view-tab") {
        let active =
            e.get_attribute("data-eval-action").as_deref() == Some(&format!("view-{view}"));
        let _ = e.class_list().toggle_with_force("active", active);
    }
    for e in dom::all(".view-panel") {
        let _ = e
            .class_list()
            .toggle_with_force("active", e.id() == format!("panel-{view}"));
    }
}
fn delta_number(value: &V) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|s| s.trim().parse::<f64>().ok()))
        .filter(|n| n.is_finite())
}
fn delta_class(value: &V, lower_is_better: bool) -> &'static str {
    let Some(n) = delta_number(value) else {
        return "";
    };
    if n == 0.0 {
        return "";
    }
    let improved = if lower_is_better { n < 0.0 } else { n > 0.0 };
    if improved {
        "benchmark-delta-positive"
    } else {
        "benchmark-delta-negative"
    }
}
fn format_delta(value: &V, pct: bool, suffix: &str) -> String {
    if value.is_null() {
        return "—".into();
    }
    if pct {
        if let Some(raw) = value.as_str().filter(|s| s.trim_end().ends_with('%')) {
            return raw.to_string();
        }
        if let Some(n) = delta_number(value) {
            let pts = n * 100.0;
            if pts.abs() < 1e-9 {
                return "0%".into();
            }
            return if (pts - pts.round()).abs() < 1e-6 {
                format!("{pts:+.0}%")
            } else {
                format!("{pts:+.1}%")
            };
        }
    }
    format!("{}{suffix}", display(value))
}
fn rate_class(n: f64) -> &'static str {
    if n >= 0.8 {
        "benchmark-delta-positive"
    } else if n < 0.5 {
        "benchmark-delta-negative"
    } else {
        ""
    }
}
fn stat(value: &V, pct: bool, decimals: usize) -> String {
    if value.is_null() {
        return "—".into();
    }
    // One measured number has no spread to show.
    if let Some(single) = value.as_f64() {
        return if pct {
            format!("{:.0}%", single * 100.0)
        } else {
            format!("{single:.decimals$}")
        };
    }
    if pct {
        format!(
            "{:.0}% ± {:.0}%",
            number(value, "mean") * 100.0,
            number(value, "stddev") * 100.0
        )
    } else {
        format!(
            "{:.decimals$} ± {:.decimals$}",
            number(value, "mean"),
            number(value, "stddev")
        )
    }
}
fn table(parent: &Element, headers: &[String]) -> Element {
    let table = add(parent, "table", "benchmark-table", "");
    let head = add(&table, "thead", "", "");
    let tr = add(&head, "tr", "", "");
    for h in headers {
        add(&tr, "th", "", h);
    }
    add(&table, "tbody", "", "")
}
fn render_benchmark() {
    let data = read(|s| s.data["benchmark"].clone());
    if data.is_null() {
        return;
    }
    show("view-tabs", true);
    let Some(root) = dom::get("benchmark-content") else {
        return;
    };
    root.set_text_content(None);
    add(&root, "h2", "", "Benchmark Results");
    let meta = &data["metadata"];
    let ids = list(&meta["evals_run"])
        .iter()
        .map(display)
        .collect::<Vec<_>>()
        .join(", ");
    add(
        &root,
        "p",
        "benchmark-meta",
        &format!(
            "{} — {} — Evals: {ids} — {} runs per configuration",
            text(meta, "skill_name"),
            text(meta, "timestamp"),
            display(&meta["runs_per_configuration"])
        ),
    );
    let summary = &data["run_summary"];
    let configs = if let Some(order) = data["configuration_order"].as_array() {
        order
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect::<Vec<_>>()
    } else {
        summary
            .as_object()
            .map(|m| {
                m.keys()
                    .filter(|k| *k != "delta")
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let a = configs.first().map(String::as_str).unwrap_or("config_a");
    let b = configs.get(1).map(String::as_str).unwrap_or("config_b");
    let body = table(
        &root,
        &["Metric".into(), title(a), title(b), "Delta".into()],
    );
    for (key, label, pct, suffix, decimals, lower_is_better) in METRICS {
        if key != "pass_rate" && summary[a][key].is_null() && summary[b][key].is_null() {
            continue;
        }
        let tr = add(&body, "tr", "", "");
        add(&tr, "td", "", label);
        add(&tr, "td", "", &stat(&summary[a][key], pct, decimals));
        add(&tr, "td", "", &stat(&summary[b][key], pct, decimals));
        let d = &summary["delta"][key];
        add(
            &tr,
            "td",
            delta_class(d, lower_is_better),
            &format_delta(d, pct, suffix),
        );
    }
    let runs = list(&data["runs"]);
    if !runs.is_empty() {
        add(&root, "h3", "", "Per-Eval Breakdown");
        let mut ids = vec![];
        for r in runs {
            if !ids.contains(&r["eval_id"]) {
                ids.push(r["eval_id"].clone());
            }
        }
        ids.sort_by(|a, b| {
            a.as_f64()
                .unwrap_or(0.0)
                .total_cmp(&b.as_f64().unwrap_or(0.0))
        });
        let has_time = runs.iter().any(|r| !r["result"]["time_seconds"].is_null());
        let has_errors = runs.iter().any(|r| number(&r["result"], "errors") > 0.0);
        for id in ids {
            let current = runs
                .iter()
                .filter(|r| r["eval_id"] == id)
                .collect::<Vec<_>>();
            let name = current
                .first()
                .map(|r| text(r, "eval_name"))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| format!("Eval {}", display(&id)));
            add(&root, "h4", "", &name);
            let mut headers = vec!["Config".into(), "Run".into(), "Pass Rate".into()];
            if has_time {
                headers.push("Time (s)".into());
            }
            if has_errors {
                headers.push("Crashes During Execution".into());
            }
            let body = table(&root, &headers);
            let mut configs = vec![];
            for r in &current {
                let config = text(r, "configuration");
                if !configs.contains(&config) {
                    configs.push(config);
                }
            }
            for (index, config) in configs.iter().enumerate() {
                let group = current
                    .iter()
                    .copied()
                    .filter(|r| r["configuration"] == config.as_str())
                    .collect::<Vec<_>>();
                let row_class = if index == 0 {
                    "benchmark-row-with"
                } else {
                    "benchmark-row-without"
                };
                for run in &group {
                    let r = &run["result"];
                    let tr = add(&body, "tr", row_class, "");
                    add(&tr, "td", "", &title(config));
                    add(&tr, "td", "", &display(&run["run_number"]));
                    add(
                        &tr,
                        "td",
                        rate_class(number(r, "pass_rate")),
                        &format!(
                            "{:.0}% ({}/{})",
                            number(r, "pass_rate") * 100.0,
                            r["passed"].as_u64().unwrap_or(0),
                            r["total"].as_u64().unwrap_or(0)
                        ),
                    );
                    if has_time {
                        add(
                            &tr,
                            "td",
                            "",
                            &r["time_seconds"]
                                .as_f64()
                                .map(|n| format!("{n:.1}"))
                                .unwrap_or_else(|| "—".into()),
                        );
                    }
                    if has_errors {
                        add(
                            &tr,
                            "td",
                            "",
                            &format!("{}", r["errors"].as_u64().unwrap_or(0)),
                        );
                    }
                }
                let mean = group
                    .iter()
                    .map(|r| number(&r["result"], "pass_rate"))
                    .sum::<f64>()
                    / group.len() as f64;
                let tr = add(&body, "tr", &format!("benchmark-row-avg {row_class}"), "");
                add(&tr, "td", "", &title(config));
                add(&tr, "td", "", "Avg");
                add(
                    &tr,
                    "td",
                    rate_class(mean),
                    &format!("{:.0}%", mean * 100.0),
                );
                if has_time {
                    let times = group
                        .iter()
                        .filter_map(|r| r["result"]["time_seconds"].as_f64())
                        .collect::<Vec<_>>();
                    add(
                        &tr,
                        "td",
                        "",
                        &if times.is_empty() {
                            "—".into()
                        } else {
                            format!("{:.1}", times.iter().sum::<f64>() / times.len() as f64)
                        },
                    );
                }
                if has_errors {
                    add(&tr, "td", "", "");
                }
            }
            let mut assertions = vec![];
            for config in &configs {
                for run in current
                    .iter()
                    .filter(|r| r["configuration"] == config.as_str())
                {
                    for exp in list(&run["expectations"]) {
                        let text = text(exp, "text");
                        if !assertions.contains(&text) {
                            assertions.push(text);
                        }
                    }
                }
            }
            if !assertions.is_empty() {
                let mut headers = vec!["Assertion".into()];
                headers.extend(configs.iter().map(|c| title(c)));
                let body = table(&root, &headers);
                for assertion in assertions {
                    let tr = add(&body, "tr", "", "");
                    add(&tr, "td", "", &assertion);
                    for config in &configs {
                        let td = add(&tr, "td", "", "");
                        for run in current.iter().filter(|r| {
                            r["configuration"] == config.as_str()
                                && !list(&r["expectations"]).is_empty()
                        }) {
                            if let Some(exp) = list(&run["expectations"])
                                .iter()
                                .find(|e| e["text"] == assertion)
                            {
                                let pass = exp["passed"] == true;
                                let icon = add(
                                    &td,
                                    "span",
                                    if pass {
                                        "benchmark-delta-positive"
                                    } else {
                                        "benchmark-delta-negative"
                                    },
                                    if pass { "✓ " } else { "✗ " },
                                );
                                let _ = icon.set_attribute(
                                    "title",
                                    &format!(
                                        "Run {}: {}",
                                        display(&run["run_number"]),
                                        text(exp, "evidence")
                                    ),
                                );
                            } else {
                                add(&td, "span", "", "— ");
                            }
                        }
                    }
                }
            }
        }
    }
    let notes = list(&data["notes"]);
    if !notes.is_empty() {
        let wrap = add(&root, "div", "benchmark-notes", "");
        add(&wrap, "h3", "", "Analysis Notes");
        let ul = add(&wrap, "ul", "", "");
        for note in notes {
            add(&ul, "li", "", &display(note));
        }
    }
}
pub fn start() -> Result<(), JsValue> {
    ensure_eval_viewer_styles();
    let data = dom::get("eval-viewer-data")
        .and_then(|e| e.text_content())
        .ok_or_else(|| JsValue::from_str("Missing evaluation viewer data"))?;
    let data: V = serde_json::from_str(&data).map_err(|e| JsValue::from_str(&e.to_string()))?;
    if data["mode"] == "eval-set" {
        return start_editor(data);
    }
    if !data["runs"].is_array() {
        return Err(JsValue::from_str("Invalid evaluation run list"));
    }
    write(|s| {
        s.data = data;
        s.index = 0;
        s.feedback.clear();
        s.visited.clear();
        s.pending = None;
        s.saving = false;
    });
    dom::text("skill-name", &read(|s| text(&s.data, "skill_name")));
    dom::disabled("feedback", true);
    for control in dom::all("[data-eval-action]") {
        let _ = control.set_attribute("disabled", "");
    }
    fn bind_review() -> Result<(), JsValue> {
        let click = Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
            let Some(target) = event
                .target()
                .and_then(|e| e.dyn_into::<Element>().ok())
                .and_then(|e| e.closest("[data-eval-action]").ok().flatten())
            else {
                return;
            };
            if target.has_attribute("disabled") {
                return;
            }
            event.prevent_default();
            match target
                .get_attribute("data-eval-action")
                .as_deref()
                .unwrap_or("")
            {
                "previous" => navigate(-1),
                "next" => navigate(1),
                "submit" => save(true),
                "close-done" => {
                    class("done-overlay", "visible", false);
                    save(false);
                }
                "toggle-grades" => {
                    toggle("grades-content");
                    toggle("grades-arrow");
                }
                "toggle-previous" => {
                    toggle("prev-outputs-content");
                    toggle("prev-outputs-arrow");
                }
                "view-outputs" => switch_view("outputs"),
                "view-benchmark" => switch_view("benchmark"),
                _ => {}
            }
        });
        dom::document()
            .add_event_listener_with_callback("click", click.as_ref().unchecked_ref())?;
        click.forget();
        let input = Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
            if event
                .target()
                .and_then(|e| e.dyn_into::<Element>().ok())
                .is_some_and(|e| e.id() == "feedback")
            {
                dom::text("feedback-status", "");
                capture_current();
                dom::later("eval-feedback-save", 800, || save(false));
            }
        });
        dom::document()
            .add_event_listener_with_callback("input", input.as_ref().unchecked_ref())?;
        input.forget();
        let key =
            Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(|event: web_sys::KeyboardEvent| {
                if event
                    .target()
                    .and_then(|e| e.dyn_into::<Element>().ok())
                    .is_some_and(|e| {
                        e.tag_name() == "TEXTAREA"
                            || e.tag_name() == "INPUT"
                            || e.has_attribute("contenteditable")
                    })
                {
                    return;
                }
                let delta = match event.key().as_str() {
                    "ArrowLeft" | "ArrowUp" => -1,
                    "ArrowRight" | "ArrowDown" => 1,
                    _ => return,
                };
                event.prevent_default();
                navigate(delta);
            });
        dom::document()
            .add_event_listener_with_callback("keydown", key.as_ref().unchecked_ref())?;
        key.forget();
        Ok(())
    }
    spawn_local(async {
        let should_load = read(|s| {
            !s.data["standalone"].as_bool().unwrap_or(false)
                && s.data["previous_feedback"]
                    .as_object()
                    .is_none_or(|m| m.is_empty())
                && s.data["previous_outputs"]
                    .as_object()
                    .is_none_or(|m| m.is_empty())
        });
        if should_load {
            if let Ok(value) = request("GET", None).await {
                write(|s| {
                    for r in list(&value["reviews"]) {
                        if let (Some(id), Some(feedback)) =
                            (r["run_id"].as_str(), r["feedback"].as_str())
                        {
                            s.feedback
                                .entry(id.into())
                                .or_insert_with(|| feedback.into());
                        }
                    }
                });
            }
        }
        if let Err(error) = bind_review() {
            dom::text(
                "feedback-status",
                "The review could not start. Reload this page to retry.",
            );
            web_sys::console::error_1(&error);
            return;
        }
        for control in dom::all("[data-eval-action]") {
            let _ = control.remove_attribute("disabled");
        }
        dom::disabled("feedback", false);
        show_run(0);
        render_benchmark();
    });
    Ok(())
}

fn spreadsheet_note(code: &str) -> &'static str {
    match code {
        "formula-cache-missing" => {
            "No cached formula result was supplied; no recalculation was performed."
        }
        "format-work-limit" | "format-cell-limit" | "format-output-limit" => {
            "The display limit was reached; original values remain available in the download."
        }
        "invalid-cache-value" => "This cell has no usable cached value for the preview.",
        "unknown-format-id" | "invalid-style-index" | "invalid-style-sheet" => {
            "The cell format could not be resolved; a compatibility display is used."
        }
        _ => "The cell format could not be reproduced; a compatibility display is used.",
    }
}
fn render_sheet(parent: &Element, sheet: &V) {
    let warnings = list(&sheet["format_warnings"]);
    let suppressed = sheet["format_warnings_suppressed"].as_u64().unwrap_or(0);
    let warning_cells = warnings
        .iter()
        .filter_map(|warning| Some((warning["cell"].as_str()?, warning["code"].as_str()?)))
        .collect::<std::collections::BTreeMap<_, _>>();
    if !warnings.is_empty() || suppressed > 0 {
        let notes = add(parent, "details", "spreadsheet-notes", "");
        add(&notes, "summary", "", "Spreadsheet display notes");
        let entries = add(&notes, "ul", "", "");
        for warning in warnings {
            let message = spreadsheet_note(warning["code"].as_str().unwrap_or(""));
            add(&entries, "li", "", message);
        }
        if suppressed > 0 {
            add(
                &entries,
                "li",
                "",
                &format!("{suppressed} additional display notes were omitted."),
            );
        }
    }
    let table = add(parent, "table", "xlsx-table", "");
    let merges = list(&sheet["merges"]);
    for (r, row) in list(&sheet["rows"]).iter().enumerate() {
        let tr = add(&table, "tr", "", "");
        for (c, cell) in list(row).iter().enumerate() {
            let covering = merges.iter().find(|m| {
                let mr = m["row"].as_u64().unwrap_or(0) as usize;
                let mc = m["column"].as_u64().unwrap_or(0) as usize;
                let rr = m["rows"].as_u64().unwrap_or(1) as usize;
                let cc = m["columns"].as_u64().unwrap_or(1) as usize;
                r >= mr && r < mr + rr && c >= mc && c < mc + cc
            });
            if covering.is_some_and(|m| {
                m["row"].as_u64() != Some(r as u64) || m["column"].as_u64() != Some(c as u64)
            }) {
                continue;
            }
            let key = format!("{r}:{c}");
            let shown = sheet["display"].get(&key).and_then(|value| value.as_str());
            let td = add(
                &tr,
                "td",
                "",
                &shown.map(str::to_owned).unwrap_or_else(|| {
                    if cell.is_null() {
                        String::new()
                    } else {
                        display(cell)
                    }
                }),
            );
            if let Some(m) = covering {
                let _ = td.set_attribute("rowspan", &m["rows"].to_string());
                let _ = td.set_attribute("colspan", &m["columns"].to_string());
            }
            let mut title = sheet["formulas"][&key]
                .as_str()
                .map(|formula| format!("={formula}"))
                .unwrap_or_default();
            if let Some(origin) = sheet["display_provenance"][&key].as_str() {
                let note = if origin == "missing-bool-formula-cache:false-default" {
                    "No cached result was supplied. FALSE is a compatibility default, not a calculated result."
                } else if origin == "invalid-error-cache:no-scalar" {
                    "The supplied cached error value could not be displayed; the original workbook is unchanged."
                } else {
                    "No cached result was supplied. The preview leaves the cell blank without recalculating."
                };
                if !title.is_empty() {
                    title.push('\n');
                }
                title.push_str(note);
            } else if let Some(code) = warning_cells.get(key.as_str()) {
                if !title.is_empty() {
                    title.push('\n');
                }
                title.push_str(spreadsheet_note(code));
            }
            if !title.is_empty() {
                let _ = td.set_attribute("title", &title);
            }
        }
    }
}
fn editor_summary() {
    let (pos, neg) = read(|s| {
        let items = list(&s.data["items"]);
        let pos = items.iter().filter(|i| i["should_trigger"] == true).count();
        (pos, items.len() - pos)
    });
    dom::text(
        "summary",
        &format!(
            "{} queries total: {pos} should trigger, {neg} should not trigger",
            pos + neg
        ),
    );
}
fn render_editor() {
    let items = read(|s| list(&s.data["items"]).to_vec());
    let mut sorted = items.iter().enumerate().collect::<Vec<_>>();
    sorted.sort_by_key(|(_, item)| item["should_trigger"] != true);
    let Some(body) = dom::get("eval-body") else {
        return;
    };
    body.set_text_content(None);
    let mut group = None;
    for (index, item) in sorted {
        let expected = item["should_trigger"] == true;
        if group != Some(expected) {
            let tr = add(&body, "tr", "section-header", "");
            let cell = add(
                &tr,
                "td",
                "",
                if expected {
                    "Should Trigger"
                } else {
                    "Should NOT Trigger"
                },
            );
            let _ = cell.set_attribute("colspan", "3");
            group = Some(expected);
        }
        let tr = add(&body, "tr", "", "");
        let td = add(&tr, "td", "", "");
        let input = add(&td, "textarea", "query-input", "");
        let _ = input.set_attribute("data-editor-field", "query");
        let _ = input.set_attribute("data-editor-index", &index.to_string());
        dom::set_element_value(&input, &text(item, "query"));
        let td = add(&tr, "td", "", "");
        let label = add(&td, "label", "toggle", "");
        let checkbox = add(&label, "input", "", "");
        let _ = checkbox.set_attribute("type", "checkbox");
        let _ = checkbox.set_attribute("data-editor-field", "should_trigger");
        let _ = checkbox.set_attribute("data-editor-index", &index.to_string());
        if let Some(input) = checkbox.dyn_ref::<web_sys::HtmlInputElement>() {
            input.set_checked(expected);
        }
        add(&label, "span", "slider", "");
        add(
            &td,
            "span",
            "trigger-label",
            if expected { "Yes" } else { "No" },
        );
        let td = add(&tr, "td", "", "");
        let button = add(&td, "button", "btn-delete", "Delete");
        let _ = button.set_attribute("type", "button");
        let _ = button.set_attribute("data-editor-action", "delete");
        let _ = button.set_attribute("data-editor-index", &index.to_string());
    }
    editor_summary();
}
fn export_editor() {
    let items = read(|s| {
        list(&s.data["items"])
            .iter()
            .filter_map(|item| {
                let query = text(item, "query");
                if query.trim().is_empty() {
                    None
                } else {
                    Some(
                        json!({"query":query.trim(),"should_trigger":item["should_trigger"]==true}),
                    )
                }
            })
            .collect::<Vec<_>>()
    });
    let a = dom::el("a", "", "");
    let _ = a.set_attribute("download", "eval_set.json");
    let _ = a.set_attribute(
        "href",
        &format!(
            "data:application/json;charset=utf-8,{}",
            dom::enc(&serde_json::to_string_pretty(&items).unwrap_or_default())
        ),
    );
    if let Some(body) = dom::document().body() {
        let _ = body.append_child(&a);
    }
    if let Some(element) = a.dyn_ref::<web_sys::HtmlElement>() {
        element.click();
    }
    a.remove();
}
fn start_editor(data: V) -> Result<(), JsValue> {
    if !data["items"].is_array() {
        return Err(JsValue::from_str("Invalid evaluation set"));
    }
    write(|s| s.data = data);
    dom::text("skill-name", &read(|s| text(&s.data, "skill_name")));
    dom::text("skill-desc", &read(|s| text(&s.data, "description")));
    render_editor();
    let click = Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
        let Some(target) = event
            .target()
            .and_then(|e| e.dyn_into::<Element>().ok())
            .and_then(|e| e.closest("[data-editor-action]").ok().flatten())
        else {
            return;
        };
        event.prevent_default();
        match target
            .get_attribute("data-editor-action")
            .as_deref()
            .unwrap_or("")
        {
            "add" => {
                let index = write(|s| add_eval_set_item(&mut s.data));
                render_editor();
                if let Ok(Some(input)) = dom::document()
                    .query_selector(&format!("textarea[data-editor-index='{index}']"))
                {
                    if let Some(element) = input.dyn_ref::<web_sys::HtmlElement>() {
                        let _ = element.focus();
                    }
                }
            }
            "delete" => {
                if let Some(index) = target
                    .get_attribute("data-editor-index")
                    .and_then(|s| s.parse::<usize>().ok())
                {
                    write(|s| {
                        if let Some(items) = s.data["items"].as_array_mut() {
                            if index < items.len() {
                                items.remove(index);
                            }
                        }
                    });
                    render_editor();
                }
            }
            "export" => export_editor(),
            _ => {}
        }
    });
    dom::document().add_event_listener_with_callback("click", click.as_ref().unchecked_ref())?;
    click.forget();
    let edit = Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
        let Some(input) = event.target().and_then(|e| e.dyn_into::<Element>().ok()) else {
            return;
        };
        let Some(index) = input
            .get_attribute("data-editor-index")
            .and_then(|s| s.parse::<usize>().ok())
        else {
            return;
        };
        let field = input.get_attribute("data-editor-field").unwrap_or_default();
        if field == "query" {
            write(|s| {
                if let Some(item) = s.data["items"]
                    .as_array_mut()
                    .and_then(|items| items.get_mut(index))
                {
                    item["query"] = json!(dom::value_of(&input));
                }
            });
            editor_summary();
        } else if field == "should_trigger" {
            write(|s| {
                if let Some(item) = s.data["items"]
                    .as_array_mut()
                    .and_then(|items| items.get_mut(index))
                {
                    item["should_trigger"] = json!(dom::checked_of(&input));
                }
            });
            render_editor();
        }
    });
    dom::document().add_event_listener_with_callback("change", edit.as_ref().unchecked_ref())?;
    edit.forget();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_id_names_its_configuration_and_baselines_are_marked() {
        assert_eq!(
            configuration_of("eval-1-with_skill-run-2"),
            Some("with_skill")
        );
        assert_eq!(
            configuration_of("eval-1-without_skill-run-2"),
            Some("without_skill")
        );
        assert_eq!(
            configuration_of("t1/rename/skillcache_executor"),
            Some("skillcache_executor")
        );
        assert_eq!(configuration_of("t1/exact/cold"), Some("cold"));
        assert_eq!(configuration_of("t1/exact/raw_trace"), Some("raw_trace"));
        assert_eq!(configuration_of("scold-1"), None);
        assert_eq!(configuration_of("eval-7"), None);
        assert!(is_baseline("cold") && is_baseline("without_skill") && is_baseline("old_skill"));
        assert!(!is_baseline("skillcache_executor") && !is_baseline("with_skill"));
    }

    #[test]
    fn delta_formatting_and_polarity_follow_metric_direction() {
        // Higher pass_rate is positive; lower tokens / USD / deopt_rate is positive.
        assert_eq!(
            delta_class(&json!("+0.25"), false),
            "benchmark-delta-positive"
        );
        assert_eq!(
            delta_class(&json!(-0.10), false),
            "benchmark-delta-negative"
        );
        assert_eq!(delta_class(&json!(-5000), true), "benchmark-delta-positive");
        assert_eq!(delta_class(&json!(0.05), true), "benchmark-delta-negative");
        assert_eq!(delta_class(&json!(0.0), true), "");
        assert_eq!(delta_class(&V::Null, true), "");
        // Percentage deltas scale by 100 rather than printing raw fractions.
        assert_eq!(format_delta(&json!("+0.25"), true, ""), "+25%");
        assert_eq!(format_delta(&json!(-0.025), true, ""), "-2.5%");
        assert_eq!(format_delta(&json!(0.0), true, ""), "0%");
        assert_eq!(format_delta(&json!("-1.5"), false, "s"), "-1.5s");
        assert_eq!(format_delta(&V::Null, true, ""), "—");
    }

    #[test]
    fn a_statistic_is_a_spread_a_number_or_a_dash() {
        assert_eq!(stat(&V::Null, false, 4), "—");
        assert_eq!(
            stat(&json!({"mean": 0.5, "stddev": 0.1}), true, 1),
            "50% ± 10%"
        );
        assert_eq!(
            stat(&json!({"mean": 12.34, "stddev": 1.0}), false, 1),
            "12.3 ± 1.0"
        );
        assert_eq!(
            stat(&json!({"mean": 0.06, "stddev": 0.002}), false, 4),
            "0.0600 ± 0.0020"
        );
        // One measured number has no spread.
        assert_eq!(stat(&json!(0.2), false, 4), "0.2000");
        assert_eq!(stat(&json!(3), false, 0), "3");
        assert_eq!(stat(&json!(0.5), true, 1), "50%");
        // The SkillCache evaluation's rows are part of the summary table.
        for key in [
            "model_calls",
            "deopt_rate",
            "list_price_usd",
            "compile_usd",
            "breakeven_reuses",
        ] {
            assert!(METRICS.iter().any(|(name, ..)| *name == key), "{key}");
        }
    }

    #[test]
    fn add_eval_set_item_initializes_missing_or_non_array_items() {
        let mut data = json!({"mode": "eval-set"});
        assert_eq!(add_eval_set_item(&mut data), 0);
        assert_eq!(add_eval_set_item(&mut data), 1);
        assert_eq!(data["items"].as_array().map(Vec::len), Some(2));

        let mut bad = json!("corrupt");
        assert_eq!(add_eval_set_item(&mut bad), 0);
        assert_eq!(bad["items"][0]["should_trigger"], json!(true));
    }
}
