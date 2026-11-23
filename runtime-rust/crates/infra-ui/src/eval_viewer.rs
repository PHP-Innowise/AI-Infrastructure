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
    done_opener: Option<Element>,
    load_error: Option<String>,
    editor_row_ids: Vec<String>,
    next_editor_row_id: u64,
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
            "[hidden]{display:none!important;}.eval-output-error{color:var(--error,var(--red));}.table-scroll{width:100%;max-width:100%;overflow-x:auto;-webkit-overflow-scrolling:touch;}.eval-load-error{padding:0.75rem 1rem;border:1px solid var(--red,#b3261e);border-radius:var(--radius,6px);background:var(--red-bg,#fceaea);color:var(--red,#b3261e);font-size:0.875rem;font-weight:500;}",
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
                let preview = add(&content, "div", "pdf-preview", "");
                add(
                    &preview,
                    "p",
                    "pdf-preview-note",
                    "Inline PDF frames are disabled under strict Content Security Policy. Download the PDF to inspect it locally.",
                );
                let excerpt = text(file, "content");
                let excerpt = if excerpt.is_empty() {
                    text(file, "text")
                } else {
                    excerpt
                };
                if !excerpt.trim().is_empty() {
                    add(&preview, "pre", "", &excerpt);
                }
                let name = text(file, "name");
                let link = add(
                    &preview,
                    "a",
                    "download-link",
                    &format!("Download {}", if name.is_empty() { "PDF" } else { &name }),
                );
                let _ = link.set_attribute("href", &link_uri(file));
                if !name.is_empty() {
                    let _ = link.set_attribute("download", &name);
                }
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
    dom::attr("grades-toggle", "aria-expanded", "false");
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
fn run_select_label(index: usize, total: usize, run: &V) -> String {
    let id = text(run, "id");
    let config = configuration_of(&id).map(|c| c.replace('_', " "));
    let eval_part = run["eval_id"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| run["eval_id"].as_u64().map(|n| format!("eval {n}")))
        .unwrap_or_default();
    let suffix = match (config, eval_part.is_empty(), id.is_empty()) {
        (Some(cfg), false, _) => format!(" — {eval_part} ({cfg})"),
        (Some(cfg), true, false) => format!(" — {id} ({cfg})"),
        (Some(cfg), true, true) => format!(" — {cfg}"),
        (None, _, false) => format!(" — {id}"),
        (None, false, true) => format!(" — {eval_part}"),
        (None, true, true) => String::new(),
    };
    format!("{} of {total}{suffix}", index + 1)
}

fn sync_run_select(runs: &[V], active_idx: usize) {
    let Some(el) = dom::get("run-select") else {
        return;
    };
    let Ok(select_el) = el.dyn_into::<web_sys::HtmlSelectElement>() else {
        return;
    };
    let total = runs.len();
    let desired: Vec<(String, String)> = if runs.is_empty() {
        vec![("0".to_owned(), "No runs".to_owned())]
    } else {
        runs.iter()
            .enumerate()
            .map(|(i, r)| (i.to_string(), run_select_label(i, total, r)))
            .collect()
    };
    let current = select_el.children();
    let mut matches_existing = current.length() as usize == desired.len();
    if matches_existing {
        for (idx, (val, lbl)) in desired.iter().enumerate() {
            let Some(opt) = current.item(idx as u32) else {
                matches_existing = false;
                break;
            };
            if opt.get_attribute("value").as_deref() != Some(val.as_str())
                || opt.text_content().as_deref() != Some(lbl.as_str())
            {
                matches_existing = false;
                break;
            }
        }
    }
    if !matches_existing {
        select_el.set_text_content(None);
        for (val, lbl) in &desired {
            let opt = dom::el("option", "", lbl);
            let _ = opt.set_attribute("value", val);
            let _ = select_el.append_child(&opt);
        }
    }
    select_el.set_disabled(runs.len() <= 1);
    select_el.set_value(&active_idx.to_string());
}

fn jump_to_run(index: usize) {
    save(false);
    let valid = read(|s| index < list(&s.data["runs"]).len());
    if valid {
        show_run(index);
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
    sync_run_select(list(&data["runs"]), index);
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
    dom::attr("prev-outputs-toggle", "aria-expanded", "false");
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
    class("feedback-status", "error", false);
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
#[derive(Clone, Debug, PartialEq, Eq)]
struct InitialFeedbackPlan {
    banner_message: Option<String>,
    status_text: Option<&'static str>,
    status_error: bool,
}

fn plan_initial_feedback_load(result: Option<&Result<(), String>>) -> InitialFeedbackPlan {
    match result {
        Some(Err(detail)) => {
            let detail = detail.trim();
            let banner = if detail.is_empty() {
                "Could not load existing feedback from the server. New edits will still attempt to save or download on submit.".to_owned()
            } else {
                format!(
                    "Could not load existing feedback from the server ({detail}). New edits will still attempt to save or download on submit."
                )
            };
            InitialFeedbackPlan {
                banner_message: Some(banner),
                status_text: Some("Could not load saved feedback from server"),
                status_error: true,
            }
        }
        None | Some(Ok(())) => InitialFeedbackPlan {
            banner_message: None,
            status_text: None,
            status_error: false,
        },
    }
}

fn ensure_load_error_banner() -> Option<Element> {
    if let Some(el) = dom::get("eval-load-error") {
        return Some(el);
    }
    let main = dom::all(".main").into_iter().next()?;
    let banner = dom::el("div", "eval-load-error", "");
    let _ = banner.set_attribute("id", "eval-load-error");
    let _ = banner.set_attribute("role", "alert");
    let _ = banner.set_attribute("hidden", "");
    if let Some(first) = main.first_element_child() {
        let _ = main.insert_before(&banner, Some(&first));
    } else {
        dom::add(&main, &banner);
    }
    Some(banner)
}

fn apply_initial_feedback_load_state(plan: &InitialFeedbackPlan) {
    write(|s| s.load_error = plan.banner_message.clone());
    if let Some(banner) = ensure_load_error_banner() {
        if let Some(msg) = plan.banner_message.as_deref() {
            banner.set_text_content(Some(msg));
            let _ = banner.remove_attribute("hidden");
        } else {
            banner.set_text_content(Some(""));
            let _ = banner.set_attribute("hidden", "");
        }
    }
    if let Some(status) = plan.status_text {
        dom::text("feedback-status", status);
        class("feedback-status", "error", plan.status_error);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SaveOutcomePlan {
    status_text: &'static str,
    status_error: bool,
    show_done_overlay: bool,
    done_message: &'static str,
    trigger_download: bool,
    toast_message: Option<&'static str>,
}

fn save_outcome_plan(standalone: bool, complete: bool, server_ok: bool) -> SaveOutcomePlan {
    if standalone {
        if complete {
            SaveOutcomePlan {
                status_text: "Downloaded feedback.json",
                status_error: false,
                show_done_overlay: true,
                done_message: "Your feedback has been downloaded as feedback.json. Return to your session to share it.",
                trigger_download: true,
                toast_message: None,
            }
        } else {
            SaveOutcomePlan {
                status_text: "Will download on submit",
                status_error: false,
                show_done_overlay: false,
                done_message: "",
                trigger_download: false,
                toast_message: None,
            }
        }
    } else if server_ok {
        SaveOutcomePlan {
            status_text: "Saved",
            status_error: false,
            show_done_overlay: complete,
            done_message:
                "Your feedback has been saved. You can close this page or return to your session.",
            trigger_download: false,
            toast_message: None,
        }
    } else if complete {
        SaveOutcomePlan {
            status_text: "Server save failed — downloaded feedback.json instead",
            status_error: true,
            show_done_overlay: false,
            done_message: "",
            trigger_download: true,
            toast_message: Some("Could not save to server; downloaded feedback.json instead."),
        }
    } else {
        SaveOutcomePlan {
            status_text: "Autosave failed (server unreachable); will retry or download on submit",
            status_error: true,
            show_done_overlay: false,
            done_message: "",
            trigger_download: false,
            toast_message: None,
        }
    }
}

fn show_toast(message: &str) {
    dom::text("toast", message);
    class("toast", "visible", true);
    dom::cancel_timer("eval-toast-hide");
    dom::later("eval-toast-hide", 4500, || {
        class("toast", "visible", false);
    });
}

fn open_done_overlay(message: &str) {
    let opener = dom::document().active_element();
    write(|s| s.done_opener = opener);
    if !message.is_empty() {
        dom::text("done-overlay-message", message);
    }
    show("done-overlay", true);
    class("done-overlay", "visible", true);
    if let Some(ok_btn) =
        dom::get("done-overlay-ok").and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = ok_btn.focus();
    } else if let Some(overlay) =
        dom::get("done-overlay").and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = overlay.focus();
    }
}

fn close_done_overlay() {
    class("done-overlay", "visible", false);
    show("done-overlay", false);
    let opener = write(|s| s.done_opener.take());
    if let Some(el) = opener
        .filter(|e| e.is_connected())
        .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = el.focus();
    } else if let Some(btn) =
        dom::get("done-btn").and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = btn.focus();
    }
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
            let server_ok = if standalone {
                false
            } else {
                request("POST", Some(&data)).await.is_ok()
            };
            let outcome = save_outcome_plan(standalone, complete, server_ok);
            if !standalone && !outcome.status_error && read(|s| s.load_error.is_some()) {
                write(|s| s.load_error = None);
                if let Some(banner) = dom::get("eval-load-error") {
                    banner.set_text_content(Some(""));
                    let _ = banner.set_attribute("hidden", "");
                }
            }
            dom::text("feedback-status", outcome.status_text);
            class("feedback-status", "error", outcome.status_error);
            if outcome.trigger_download {
                download_feedback(&data);
            }
            if outcome.show_done_overlay {
                open_done_overlay(outcome.done_message);
            }
            if let Some(toast) = outcome.toast_message {
                show_toast(toast);
            }
        }
    });
}
fn navigate(delta: i32) {
    let next = read(|s| {
        let n = s.index as i64 + delta as i64;
        if n >= 0 && (n as usize) < list(&s.data["runs"]).len() {
            Some(n as usize)
        } else {
            None
        }
    });
    if let Some(index) = next {
        jump_to_run(index);
    }
}
fn switch_view(view: &str) {
    for e in dom::all(".view-tab") {
        let active =
            e.get_attribute("data-eval-action").as_deref() == Some(&format!("view-{view}"));
        let _ = e.class_list().toggle_with_force("active", active);
        let _ = e.set_attribute("aria-selected", if active { "true" } else { "false" });
        let _ = e.set_attribute("tabindex", if active { "0" } else { "-1" });
    }
    for e in dom::all(".view-panel") {
        let active = e.id() == format!("panel-{view}");
        let _ = e.class_list().toggle_with_force("active", active);
        if active {
            let _ = e.remove_attribute("hidden");
        } else {
            let _ = e.set_attribute("hidden", "");
        }
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
    let wrap = add(parent, "div", "table-scroll", "");
    let table = add(&wrap, "table", "benchmark-table", "");
    let head = add(&table, "thead", "", "");
    let tr = add(&head, "tr", "", "");
    for h in headers {
        add(&tr, "th", "", h);
    }
    add(&table, "tbody", "", "")
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct BenchmarkCell {
    text: String,
    class: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BenchmarkSummaryTable {
    headers: Vec<String>,
    rows: Vec<Vec<BenchmarkCell>>,
}

fn metric_mean(value: &V) -> Option<f64> {
    if let Some(n) = value.as_f64() {
        n.is_finite().then_some(n)
    } else {
        value
            .get("mean")
            .and_then(V::as_f64)
            .filter(|n| n.is_finite())
    }
}

fn find_baseline_delta(
    summary: &V,
    cfg: &str,
    baseline: &str,
    key: &str,
    use_top_delta: bool,
) -> V {
    for path in [
        summary
            .get("delta_vs_baseline")
            .and_then(|d| d.get(cfg))
            .and_then(|d| d.get(key)),
        summary
            .get("deltas")
            .and_then(|d| d.get(cfg))
            .and_then(|d| d.get(key)),
        summary
            .get(cfg)
            .and_then(|c| c.get("delta"))
            .and_then(|d| d.get(key)),
    ] {
        if let Some(val) = path.filter(|v| !v.is_null()) {
            return val.clone();
        }
    }
    if use_top_delta && !summary["delta"][key].is_null() {
        return summary["delta"][key].clone();
    }
    match (
        metric_mean(&summary[cfg][key]),
        metric_mean(&summary[baseline][key]),
    ) {
        (Some(cur), Some(base)) => json!(cur - base),
        _ => V::Null,
    }
}

fn build_benchmark_summary_table(summary: &V, configs: &[String]) -> BenchmarkSummaryTable {
    let effective_configs = if configs.is_empty() {
        vec!["config_a".to_owned(), "config_b".to_owned()]
    } else {
        configs.to_vec()
    };
    let mut headers = vec!["Metric".to_owned()];
    for cfg in &effective_configs {
        headers.push(title(cfg));
    }

    let baseline = effective_configs.iter().find(|c| is_baseline(c)).cloned();
    enum DeltaCol {
        TopLevel,
        VsBaseline { cfg: String, base: String },
    }
    let mut delta_cols = Vec::new();
    if effective_configs.len() == 2 {
        headers.push("Delta".to_owned());
        if let Some(base) = baseline.clone() {
            let primary = effective_configs
                .iter()
                .find(|c| *c != &base)
                .cloned()
                .unwrap_or_else(|| effective_configs[0].clone());
            delta_cols.push(DeltaCol::VsBaseline { cfg: primary, base });
        } else {
            delta_cols.push(DeltaCol::TopLevel);
        }
    } else if effective_configs.len() >= 3 {
        if let Some(base) = baseline.clone() {
            for cfg in effective_configs.iter().filter(|c| *c != &base) {
                headers.push(format!("{} vs {}", title(cfg), title(&base)));
                delta_cols.push(DeltaCol::VsBaseline {
                    cfg: cfg.clone(),
                    base: base.clone(),
                });
            }
        } else if !summary["delta"].is_null() {
            headers.push("Delta".to_owned());
            delta_cols.push(DeltaCol::TopLevel);
        }
    } else if !summary["delta"].is_null() {
        headers.push("Delta".to_owned());
        delta_cols.push(DeltaCol::TopLevel);
    }

    let use_top_delta_in_vs_baseline = effective_configs.len() == 2;
    let mut rows = Vec::new();
    for (key, label, pct, suffix, decimals, lower_is_better) in METRICS {
        if key != "pass_rate" && effective_configs.iter().all(|c| summary[c][key].is_null()) {
            continue;
        }
        let mut row = Vec::with_capacity(headers.len());
        row.push(BenchmarkCell {
            text: label.to_owned(),
            class: "",
        });
        for cfg in &effective_configs {
            row.push(BenchmarkCell {
                text: stat(&summary[cfg][key], pct, decimals),
                class: "",
            });
        }
        for dcol in &delta_cols {
            let d = match dcol {
                DeltaCol::TopLevel => summary["delta"][key].clone(),
                DeltaCol::VsBaseline { cfg, base } => {
                    find_baseline_delta(summary, cfg, base, key, use_top_delta_in_vs_baseline)
                }
            };
            row.push(BenchmarkCell {
                text: format_delta(&d, pct, suffix),
                class: delta_class(&d, lower_is_better),
            });
        }
        rows.push(row);
    }
    BenchmarkSummaryTable { headers, rows }
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
    let tbl = build_benchmark_summary_table(summary, &configs);
    let body = table(&root, &tbl.headers);
    for row in tbl.rows {
        let tr = add(&body, "tr", "", "");
        for cell in row {
            add(&tr, "td", cell.class, &cell.text);
        }
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
        s.load_error = None;
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
                    close_done_overlay();
                    save(false);
                }
                "toggle-grades" => {
                    toggle("grades-content");
                    toggle("grades-arrow");
                    let open =
                        dom::get("grades-content").is_some_and(|e| e.class_list().contains("open"));
                    dom::attr(
                        "grades-toggle",
                        "aria-expanded",
                        if open { "true" } else { "false" },
                    );
                }
                "toggle-previous" => {
                    toggle("prev-outputs-content");
                    toggle("prev-outputs-arrow");
                    let open = dom::get("prev-outputs-content")
                        .is_some_and(|e| e.class_list().contains("open"));
                    dom::attr(
                        "prev-outputs-toggle",
                        "aria-expanded",
                        if open { "true" } else { "false" },
                    );
                }
                "view-outputs" => switch_view("outputs"),
                "view-benchmark" => switch_view("benchmark"),
                _ => {}
            }
        });
        dom::document()
            .add_event_listener_with_callback("click", click.as_ref().unchecked_ref())?;
        click.forget();
        let run_jump = Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
            let Some(target) = event.target().and_then(|e| e.dyn_into::<Element>().ok()) else {
                return;
            };
            if target.id() != "run-select" || target.has_attribute("disabled") {
                return;
            }
            if let Ok(index) = dom::value_of(&target).trim().parse::<usize>() {
                jump_to_run(index);
            }
        });
        dom::document()
            .add_event_listener_with_callback("change", run_jump.as_ref().unchecked_ref())?;
        run_jump.forget();
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
        let key = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(
            |event: web_sys::KeyboardEvent| {
                if dom::get("done-overlay").is_some_and(|el| el.class_list().contains("visible")) {
                    match event.key().as_str() {
                        "Escape" => {
                            event.prevent_default();
                            close_done_overlay();
                            save(false);
                        }
                        "Tab" => {
                            event.prevent_default();
                            if let Some(ok_btn) = dom::get("done-overlay-ok")
                                .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
                            {
                                let _ = ok_btn.focus();
                            }
                        }
                        _ => {}
                    }
                    return;
                }
                if event.alt_key() || event.ctrl_key() || event.meta_key() || event.is_composing() {
                    return;
                }
                if let Some(target_el) = event.target().and_then(|e| e.dyn_into::<Element>().ok()) {
                    if target_el.get_attribute("role").as_deref() == Some("tab")
                        && target_el.closest("#view-tabs").ok().flatten().is_some()
                    {
                        let next_view = match event.key().as_str() {
                            "ArrowLeft" | "ArrowRight" => {
                                if target_el.id() == "tab-outputs" {
                                    Some(("benchmark", "tab-benchmark"))
                                } else {
                                    Some(("outputs", "tab-outputs"))
                                }
                            }
                            "Home" => Some(("outputs", "tab-outputs")),
                            "End" => Some(("benchmark", "tab-benchmark")),
                            _ => None,
                        };
                        if let Some((view, tab_id)) = next_view {
                            event.prevent_default();
                            switch_view(view);
                            if let Some(tab_el) = dom::get(tab_id)
                                .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
                            {
                                let _ = tab_el.focus();
                            }
                            return;
                        }
                    }
                    let outputs_active =
                        dom::get("panel-outputs").is_none_or(|p| p.class_list().contains("active"));
                    let is_contenteditable = target_el.has_attribute("contenteditable")
                        || target_el
                            .dyn_ref::<web_sys::HtmlElement>()
                            .is_some_and(|h| h.is_content_editable());
                    let has_interactive_ancestor = target_el
                        .closest(
                            "button, summary, details, select, input, textarea, [contenteditable], a[href], [role='button'], [role='menuitem'], [role='listbox'], [role='combobox']",
                        )
                        .ok()
                        .flatten()
                        .is_some();
                    let in_horizontal_scroll = target_el
                        .closest("pre, .table-scroll")
                        .ok()
                        .flatten()
                        .is_some_and(|sc| sc.scroll_width() > sc.client_width());
                    if should_ignore_review_arrow_key(
                        event.key().as_str(),
                        &target_el.tag_name(),
                        has_interactive_ancestor,
                        in_horizontal_scroll,
                        is_contenteditable,
                        outputs_active,
                        false,
                    ) {
                        return;
                    }
                } else if dom::get("panel-outputs")
                    .is_some_and(|p| !p.class_list().contains("active"))
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
            },
        );
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
        let load_outcome = if should_load {
            Some(match request("GET", None).await {
                Ok(value) => {
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
                    Ok(())
                }
                Err(error) => Err(error
                    .as_string()
                    .or_else(|| {
                        js_sys::Reflect::get(&error, &JsValue::from_str("message"))
                            .ok()
                            .and_then(|v| v.as_string())
                    })
                    .unwrap_or_else(|| "Request failed".to_owned())),
            })
        } else {
            None
        };
        let load_plan = plan_initial_feedback_load(load_outcome.as_ref());
        if let Err(error) = bind_review() {
            dom::text(
                "feedback-status",
                "The review could not start. Reload this page to retry.",
            );
            class("feedback-status", "error", true);
            web_sys::console::error_1(&error);
            return;
        }
        for control in dom::all("[data-eval-action]") {
            let _ = control.remove_attribute("disabled");
        }
        dom::disabled("feedback", false);
        show_run(0);
        apply_initial_feedback_load_state(&load_plan);
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
    let wrap = add(parent, "div", "table-scroll", "");
    let table = add(&wrap, "table", "xlsx-table", "");
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
fn editor_display_order(items: &[V], row_ids: &[String]) -> Vec<(usize, String, bool)> {
    let mut sorted = items
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let row_id = row_ids
                .get(idx)
                .cloned()
                .unwrap_or_else(|| format!("row-{idx}"));
            (idx, row_id, item["should_trigger"] == true)
        })
        .collect::<Vec<_>>();
    sorted.sort_by_key(|(_, _, expected)| !*expected);
    sorted
}

fn ensure_editor_section_row(
    existing: &mut BTreeMap<String, Element>,
    section_key: &str,
    label: &str,
) -> Element {
    if let Some(tr) = existing.remove(section_key) {
        return tr;
    }
    let tr = dom::el("tr", "section-header", "");
    let _ = tr.set_attribute("data-editor-section", section_key);
    let cell = add(&tr, "td", "", label);
    let _ = cell.set_attribute("colspan", "3");
    tr
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EditorRowA11y {
    query_id: String,
    query_label: String,
    trigger_id: String,
    trigger_label: String,
    trigger_text: &'static str,
    delete_label: String,
}

fn editor_row_a11y_labels(index: usize, row_id: &str, expected: bool) -> EditorRowA11y {
    let display_num = index + 1;
    let trigger_text = if expected { "Yes" } else { "No" };
    EditorRowA11y {
        query_id: format!("eval-query-{row_id}"),
        query_label: format!("Evaluation query {display_num}"),
        trigger_id: format!("eval-trigger-{row_id}"),
        trigger_label: format!("Should trigger query {display_num}"),
        trigger_text,
        delete_label: format!("Delete query {display_num}"),
    }
}

fn create_editor_item_row(row_id: &str) -> Element {
    let tr = dom::el("tr", "", "");
    let _ = tr.set_attribute("data-editor-row", row_id);

    let td_query = add(&tr, "td", "", "");
    let query_label = add(&td_query, "label", "sr-only query-label", "");
    let _ = query_label.set_attribute("for", &format!("eval-query-{row_id}"));
    let input = add(&td_query, "textarea", "query-input", "");
    let _ = input.set_attribute("id", &format!("eval-query-{row_id}"));
    let _ = input.set_attribute("data-editor-field", "query");

    let td_toggle = add(&tr, "td", "", "");
    let wrap = add(&td_toggle, "div", "toggle-wrap", "");
    let label = add(&wrap, "label", "toggle", "");
    let _ = label.set_attribute("for", &format!("eval-trigger-{row_id}"));
    let checkbox = add(&label, "input", "", "");
    let _ = checkbox.set_attribute("type", "checkbox");
    let _ = checkbox.set_attribute("id", &format!("eval-trigger-{row_id}"));
    let _ = checkbox.set_attribute("data-editor-field", "should_trigger");
    let slider = add(&label, "span", "slider", "");
    let _ = slider.set_attribute("aria-hidden", "true");
    add(&label, "span", "sr-only toggle-state-sr", "");
    let trig_label = add(&wrap, "label", "trigger-label", "");
    let _ = trig_label.set_attribute("for", &format!("eval-trigger-{row_id}"));

    let td_actions = add(&tr, "td", "", "");
    let button = add(&td_actions, "button", "btn-delete", "Delete");
    let _ = button.set_attribute("type", "button");
    let _ = button.set_attribute("data-editor-action", "delete");
    tr
}

fn sync_editor_item_row(tr: &Element, index: usize, row_id: &str, item: &V, expected: bool) {
    let a11y = editor_row_a11y_labels(index, row_id, expected);
    let index_str = index.to_string();
    if let Ok(Some(query_label)) = tr.query_selector("label.query-label") {
        let _ = query_label.set_attribute("for", &a11y.query_id);
        if query_label.text_content().as_deref() != Some(a11y.query_label.as_str()) {
            query_label.set_text_content(Some(&a11y.query_label));
        }
    }
    if let Ok(Some(textarea)) = tr.query_selector("textarea.query-input") {
        let _ = textarea.set_attribute("id", &a11y.query_id);
        let _ = textarea.set_attribute("data-editor-index", &index_str);
        let _ = textarea.set_attribute("aria-label", &a11y.query_label);
        let next_val = text(item, "query");
        if dom::value_of(&textarea) != next_val {
            dom::set_element_value(&textarea, &next_val);
        }
    }
    if let Ok(Some(toggle_wrap_label)) = tr.query_selector("label.toggle") {
        let _ = toggle_wrap_label.set_attribute("for", &a11y.trigger_id);
    }
    if let Ok(Some(toggle_sr)) = tr.query_selector("label.toggle .toggle-state-sr") {
        let sr_text = format!("{}: {}", a11y.trigger_label, a11y.trigger_text);
        if toggle_sr.text_content().as_deref() != Some(sr_text.as_str()) {
            toggle_sr.set_text_content(Some(&sr_text));
        }
    }
    if let Ok(Some(checkbox)) = tr.query_selector("input[data-editor-field='should_trigger']") {
        let _ = checkbox.set_attribute("id", &a11y.trigger_id);
        let _ = checkbox.set_attribute("data-editor-index", &index_str);
        let _ = checkbox.set_attribute("aria-label", &a11y.trigger_label);
        if let Some(input) = checkbox.dyn_ref::<web_sys::HtmlInputElement>() {
            if input.checked() != expected {
                input.set_checked(expected);
            }
        }
    }
    if let Ok(Some(trig_label)) = tr.query_selector("label.trigger-label") {
        let _ = trig_label.set_attribute("for", &a11y.trigger_id);
        if trig_label.text_content().as_deref() != Some(a11y.trigger_text) {
            trig_label.set_text_content(Some(a11y.trigger_text));
        }
    }
    if let Ok(Some(btn)) = tr.query_selector("button.btn-delete") {
        let _ = btn.set_attribute("data-editor-index", &index_str);
        let _ = btn.set_attribute("aria-label", &a11y.delete_label);
    }
}

fn render_editor() {
    let (items, row_ids) = read(|s| (list(&s.data["items"]).to_vec(), s.editor_row_ids.clone()));
    let sorted = editor_display_order(&items, &row_ids);
    let Some(body) = dom::get("eval-body") else {
        return;
    };

    let active_before = dom::document()
        .active_element()
        .filter(|el| body.contains(Some(el)));

    let mut existing_sections = BTreeMap::<String, Element>::new();
    let mut existing_rows = BTreeMap::<String, Element>::new();
    for child in dom::children(&body) {
        if let Some(sec) = child.get_attribute("data-editor-section") {
            existing_sections.insert(sec, child);
        } else if let Some(row_id) = child.get_attribute("data-editor-row") {
            existing_rows.insert(row_id, child);
        } else {
            child.remove();
        }
    }

    let mut desired: Vec<Element> = Vec::with_capacity(sorted.len() + 2);
    let mut group = None;
    for (index, row_id, expected) in sorted {
        if group != Some(expected) {
            let (sec_key, sec_label) = if expected {
                ("trigger", "Should Trigger")
            } else {
                ("no-trigger", "Should NOT Trigger")
            };
            desired.push(ensure_editor_section_row(
                &mut existing_sections,
                sec_key,
                sec_label,
            ));
            group = Some(expected);
        }
        let tr = existing_rows
            .remove(&row_id)
            .unwrap_or_else(|| create_editor_item_row(&row_id));
        sync_editor_item_row(&tr, index, &row_id, &items[index], expected);
        desired.push(tr);
    }

    for unused in existing_sections
        .into_values()
        .chain(existing_rows.into_values())
    {
        unused.remove();
    }

    for (pos, el) in desired.iter().enumerate() {
        let cur = body.children().item(pos as u32);
        if cur.as_ref() != Some(el) {
            let _ = body.insert_before(el, cur.as_ref().map(|n| n.as_ref()));
        }
    }

    if let Some(active) = active_before {
        if active.is_connected() && dom::document().active_element().as_ref() != Some(&active) {
            if let Some(html) = active.dyn_ref::<web_sys::HtmlElement>() {
                let _ = html.focus();
            }
        }
    }

    editor_summary();
}
fn should_ignore_review_arrow_key(
    key: &str,
    tag_name: &str,
    has_interactive_ancestor: bool,
    in_horizontal_scroll: bool,
    is_contenteditable: bool,
    outputs_panel_active: bool,
    has_modifier: bool,
) -> bool {
    if !outputs_panel_active || has_modifier || is_contenteditable || has_interactive_ancestor {
        return true;
    }
    let upper = tag_name.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "TEXTAREA" | "INPUT" | "SELECT" | "OPTION" | "BUTTON" | "SUMMARY" | "DETAILS" | "A"
    ) {
        return true;
    }
    matches!(key, "ArrowLeft" | "ArrowRight") && in_horizontal_scroll
}

fn apply_editor_query_value(index: usize, value: String) {
    write(|s| {
        if let Some(item) = s.data["items"]
            .as_array_mut()
            .and_then(|items| items.get_mut(index))
        {
            item["query"] = json!(value);
        }
    });
    editor_summary();
}

fn collect_export_editor_items(items: &[V], overrides: &[(usize, String)]) -> Vec<V> {
    let override_map: BTreeMap<usize, &str> =
        overrides.iter().map(|(i, v)| (*i, v.as_str())).collect();
    items
        .iter()
        .enumerate()
        .filter_map(|(idx, item)| {
            let query = override_map
                .get(&idx)
                .copied()
                .unwrap_or_else(|| item["query"].as_str().unwrap_or(""));
            let trimmed = query.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(json!({
                    "query": trimmed,
                    "should_trigger": item["should_trigger"] == true
                }))
            }
        })
        .collect()
}

fn flush_editor_inputs() {
    for input in dom::all("textarea.query-input[data-editor-index]") {
        let Some(index) = input
            .get_attribute("data-editor-index")
            .and_then(|s| s.parse::<usize>().ok())
        else {
            continue;
        };
        let value = dom::value_of(&input);
        write(|s| {
            if let Some(item) = s.data["items"]
                .as_array_mut()
                .and_then(|items| items.get_mut(index))
            {
                item["query"] = json!(value);
            }
        });
    }
    editor_summary();
}

fn export_editor() {
    flush_editor_inputs();
    let items = read(|s| collect_export_editor_items(list(&s.data["items"]), &[]));
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
    let count = list(&data["items"]).len();
    write(|s| {
        s.data = data;
        s.editor_row_ids = (0..count).map(|i| format!("row-{i}")).collect();
        s.next_editor_row_id = count as u64;
    });
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
                let index = write(|s| {
                    let index = add_eval_set_item(&mut s.data);
                    let row_id = format!("row-{}", s.next_editor_row_id);
                    s.next_editor_row_id = s.next_editor_row_id.saturating_add(1);
                    s.editor_row_ids.push(row_id);
                    index
                });
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
                        if index < s.editor_row_ids.len() {
                            s.editor_row_ids.remove(index);
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
            apply_editor_query_value(index, dom::value_of(&input));
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
    let live_input = Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
        let Some(input) = event.target().and_then(|e| e.dyn_into::<Element>().ok()) else {
            return;
        };
        if input.get_attribute("data-editor-field").as_deref() != Some("query") {
            return;
        }
        let Some(index) = input
            .get_attribute("data-editor-index")
            .and_then(|s| s.parse::<usize>().ok())
        else {
            return;
        };
        apply_editor_query_value(index, dom::value_of(&input));
    });
    dom::document()
        .add_event_listener_with_callback("input", live_input.as_ref().unchecked_ref())?;
    live_input.forget();
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

    #[test]
    fn save_outcome_plan_distinguishes_server_failure_from_standalone() {
        let srv_ok = save_outcome_plan(false, true, true);
        assert!(srv_ok.show_done_overlay);
        assert!(!srv_ok.status_error);
        assert!(!srv_ok.trigger_download);
        assert!(!srv_ok.done_message.contains("Claude"));

        let srv_err_submit = save_outcome_plan(false, true, false);
        assert!(!srv_err_submit.show_done_overlay);
        assert!(srv_err_submit.status_error);
        assert!(srv_err_submit.trigger_download);
        assert!(srv_err_submit.toast_message.is_some());

        let srv_err_auto = save_outcome_plan(false, false, false);
        assert!(!srv_err_auto.show_done_overlay);
        assert!(srv_err_auto.status_error);
        assert!(!srv_err_auto.trigger_download);

        let standalone_submit = save_outcome_plan(true, true, false);
        assert!(standalone_submit.show_done_overlay);
        assert!(!standalone_submit.status_error);
        assert!(standalone_submit.trigger_download);
        assert!(!standalone_submit.done_message.contains("Claude"));
    }

    #[test]
    fn editor_display_order_groups_triggers_first_with_stable_row_keys() {
        let items = vec![
            json!({"query": "no", "should_trigger": false}),
            json!({"query": "yes", "should_trigger": true}),
            json!({"query": "no2", "should_trigger": false}),
        ];
        let row_ids = vec!["row-0".into(), "row-1".into(), "row-2".into()];
        assert_eq!(
            editor_display_order(&items, &row_ids),
            vec![
                (1, "row-1".into(), true),
                (0, "row-0".into(), false),
                (2, "row-2".into(), false),
            ]
        );
    }

    #[test]
    fn editor_row_a11y_labels_match_row_ids_and_trigger_text() {
        let yes = editor_row_a11y_labels(0, "row-7", true);
        assert_eq!(yes.query_id, "eval-query-row-7");
        assert_eq!(yes.query_label, "Evaluation query 1");
        assert_eq!(yes.trigger_id, "eval-trigger-row-7");
        assert_eq!(yes.trigger_label, "Should trigger query 1");
        assert_eq!(yes.trigger_text, "Yes");
        assert_eq!(yes.delete_label, "Delete query 1");

        let no = editor_row_a11y_labels(2, "row-0", false);
        assert_eq!(no.query_id, "eval-query-row-0");
        assert_eq!(no.query_label, "Evaluation query 3");
        assert_eq!(no.trigger_id, "eval-trigger-row-0");
        assert_eq!(no.trigger_label, "Should trigger query 3");
        assert_eq!(no.trigger_text, "No");
        assert_eq!(no.delete_label, "Delete query 3");
    }

    #[test]
    fn initial_feedback_load_surfaces_explicit_error_state_on_failure() {
        let skipped = plan_initial_feedback_load(None);
        assert_eq!(skipped.banner_message, None);
        assert_eq!(skipped.status_text, None);
        assert!(!skipped.status_error);

        let loaded = plan_initial_feedback_load(Some(&Ok(())));
        assert_eq!(loaded.banner_message, None);
        assert_eq!(loaded.status_text, None);
        assert!(!loaded.status_error);

        let failed = plan_initial_feedback_load(Some(&Err("Feedback HTTP 500".into())));
        assert!(failed
            .banner_message
            .as_deref()
            .is_some_and(|msg| msg.contains("Feedback HTTP 500")));
        assert_eq!(
            failed.status_text,
            Some("Could not load saved feedback from server")
        );
        assert!(failed.status_error);

        let blank_err = plan_initial_feedback_load(Some(&Err("   ".into())));
        assert!(blank_err.banner_message.is_some());
        assert!(blank_err.status_error);
    }

    #[test]
    fn benchmark_summary_renders_all_configs_and_baseline_deltas() {
        let summary = json!({
            "with_skill": {
                "pass_rate": {"mean": 0.9, "stddev": 0.05},
                "time_seconds": {"mean": 10.0, "stddev": 1.0},
                "tokens": {"mean": 1200.0, "stddev": 50.0}
            },
            "skillcache": {
                "pass_rate": {"mean": 0.95, "stddev": 0.02},
                "time_seconds": {"mean": 6.0, "stddev": 0.5},
                "compile_usd": 0.0042
            },
            "without_skill": {
                "pass_rate": {"mean": 0.6, "stddev": 0.1},
                "time_seconds": {"mean": 14.0, "stddev": 2.0},
                "tokens": {"mean": 2000.0, "stddev": 100.0}
            },
            "delta_vs_baseline": {
                "with_skill": {"pass_rate": "+30%"}
            }
        });
        let configs = vec![
            "with_skill".to_owned(),
            "skillcache".to_owned(),
            "without_skill".to_owned(),
        ];
        let tbl = build_benchmark_summary_table(&summary, &configs);
        assert_eq!(
            tbl.headers,
            vec![
                "Metric",
                "With Skill",
                "Skillcache",
                "Without Skill",
                "With Skill vs Without Skill",
                "Skillcache vs Without Skill"
            ]
        );
        // Pass Rate row uses override for with_skill and computed delta for skillcache.
        assert_eq!(tbl.rows[0][0].text, "Pass Rate");
        assert_eq!(tbl.rows[0][1].text, "90% ± 5%");
        assert_eq!(tbl.rows[0][2].text, "95% ± 2%");
        assert_eq!(tbl.rows[0][3].text, "60% ± 10%");
        assert_eq!(tbl.rows[0][4].text, "+30%");
        assert_eq!(tbl.rows[0][5].text, "+35%");
        assert_eq!(tbl.rows[0][5].class, "benchmark-delta-positive");
        // Metric present ONLY on 3rd/middle config (compile_usd) must NOT be skipped.
        assert!(tbl.rows.iter().any(|r| r[0].text == "Compilation (USD)"));
    }

    #[test]
    fn arrow_navigation_guards_controls_and_horizontal_scrollables() {
        assert!(!should_ignore_review_arrow_key(
            "ArrowRight",
            "BODY",
            false,
            false,
            false,
            true,
            false
        ));
        for tag in [
            "BUTTON", "SUMMARY", "DETAILS", "SELECT", "INPUT", "TEXTAREA",
        ] {
            assert!(
                should_ignore_review_arrow_key("ArrowLeft", tag, false, false, false, true, false),
                "expected {tag} to block ArrowLeft"
            );
        }
        assert!(should_ignore_review_arrow_key(
            "ArrowRight",
            "SPAN",
            true,
            false,
            false,
            true,
            false
        ));
        assert!(should_ignore_review_arrow_key(
            "ArrowRight",
            "PRE",
            false,
            true,
            false,
            true,
            false
        ));
        assert!(should_ignore_review_arrow_key(
            "ArrowLeft",
            "DIV",
            false,
            true,
            false,
            true,
            false
        ));
        // ArrowUp still works inside non-interactive horizontally scrollable containers.
        assert!(!should_ignore_review_arrow_key(
            "ArrowUp", "PRE", false, true, false, true, false
        ));
        // Inactive outputs panel or modifier keys ignore navigation arrows.
        assert!(should_ignore_review_arrow_key(
            "ArrowRight",
            "BODY",
            false,
            false,
            false,
            false,
            false
        ));
        assert!(should_ignore_review_arrow_key(
            "ArrowRight",
            "BODY",
            false,
            false,
            false,
            true,
            true
        ));
    }

    #[test]
    fn editor_export_flushes_unsaved_query_overrides_and_formats_run_select_labels() {
        let items = vec![
            json!({"query": "old query", "should_trigger": true}),
            json!({"query": "   ", "should_trigger": false}),
        ];
        let exported =
            collect_export_editor_items(&items, &[(0, "  live edit before blur  ".into())]);
        assert_eq!(
            exported,
            vec![json!({"query": "live edit before blur", "should_trigger": true})]
        );

        let r = json!({"id": "eval-2-with_skill", "eval_id": 2});
        assert_eq!(run_select_label(0, 4, &r), "1 of 4 — eval 2 (with skill)");
        let plain = json!({"id": "custom-run"});
        assert_eq!(run_select_label(1, 4, &plain), "2 of 4 — custom-run");
    }
}
