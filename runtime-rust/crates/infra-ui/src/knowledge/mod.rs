mod memory_use;
mod records;
#[cfg(test)]
mod tests;
mod usage;

#[allow(unused_imports)]
pub use memory_use::{leave_memory, load_memory_use, visibility};
#[allow(unused_imports)]
pub use records::{
    brain_config, bytes, followup_brain, linked_controls, load, load_brain_link, load_context,
    open_from_omnigraph, render_detail_omnigraph_action_html, render_linked,
    render_linked_omnigraph_html, reset_project,
};
pub use usage::usage;

use crate::{api, dom, model as m, session};
use memory_use::*;
use records::*;
use serde_json::{json, Value as V};
use std::cell::Cell;
use std::rc::Rc;
use usage::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

pub(super) fn viewport_is_mobile() -> bool {
    dom::is_mobile_viewport()
}

pub(super) fn usage_page_size(mobile: bool) -> usize {
    if mobile {
        6
    } else {
        12
    }
}

pub(super) fn analytics_slice(raw: &[V], mobile: bool) -> &[V] {
    if mobile && raw.len() > 80 {
        &raw[raw.len() - 80..]
    } else {
        raw
    }
}

pub(super) fn horizon_bin_days(mobile: bool) -> i64 {
    if mobile {
        14
    } else {
        7
    }
}

pub(super) fn horizon_ticks(mobile: bool) -> &'static [i64] {
    if mobile {
        &[0, 180, 365]
    } else {
        &[-30, 0, 90, 180, 270, 365]
    }
}

fn reflow_responsive_views() {
    if m::data("memory-use").is_object() {
        render_memory();
    }
    if !m::array(&m::data("context-turns")).is_empty() {
        render_usage();
    }
}

fn sync_responsive_viewport(last_mobile: &Cell<bool>) {
    let mobile = viewport_is_mobile();
    if last_mobile.replace(mobile) != mobile {
        reflow_responsive_views();
    }
}

fn wire_responsive_reflow() {
    let last_mobile = Rc::new(Cell::new(viewport_is_mobile()));
    dom::on_viewport_resize("knowledge-responsive-viewport", 0, move || {
        sync_responsive_viewport(&last_mobile);
    });
}

pub fn initialize() {
    wire_responsive_reflow();
    crate::session::on_project_change(crate::knowledge::on_global_project_change);
    m::set("memory-window", json!(30));
    for (days, label) in [(7, "7 d"), (30, "30 d"), (90, "90 d"), (0, "All")] {
        let b = dom::button(label, "memory-window", json!(days));
        let _ = b.set_attribute("role", "radio");
        let _ = b.set_attribute("aria-checked", if days == 30 { "true" } else { "false" });
        let _ = b.set_attribute("tabindex", if days == 30 { "0" } else { "-1" });
        dom::append("memory-use-window", &b);
    }

    for scope in ["memory", "brain"] {
        let panel = dom::el("details", "knowledge-tools", "");
        let _ = panel.set_attribute("id", &format!("{scope}-knowledge-panel"));
        dom::add(
            &panel,
            &dom::el(
                "summary",
                "",
                if scope == "brain" {
                    "Record tools & maintenance"
                } else {
                    "Search & maintenance"
                },
            ),
        );
        let runtime = dom::el("p", "knowledge-note", "Checking context runtime…");
        let _ = runtime.set_attribute("id", &format!("{scope}-knowledge-runtime"));
        dom::add(&panel, &runtime);
        if scope == "memory" {
            let f = dom::el("form", "knowledge-search", "");
            let _ = f.set_attribute("id", "memory-search-form");
            let q = dom::el("input", "", "");
            let _ = q.set_attribute("id", "memory-search-query");
            let _ = q.set_attribute("required", "");
            let _ = q.set_attribute("maxlength", "2000");
            let _ = q.set_attribute("placeholder", "Search indexed content");
            dom::add(&f, &q);
            let layer = dom::el("select", "", "");
            let _ = layer.set_attribute("id", "memory-search-layer");
            for value in ["", "procedural", "semantic", "episodic"] {
                let e = dom::el(
                    "option",
                    "",
                    if value.is_empty() {
                        "All layers"
                    } else {
                        value
                    },
                );
                let _ = e.set_attribute("value", value);
                dom::add(&layer, &e);
            }
            dom::add(&f, &layer);
            let b = dom::el("button", "button", "Search index");
            let _ = b.set_attribute("type", "submit");
            dom::add(&f, &b);
            let out = dom::el("section", "knowledge-output", "");
            let _ = out.set_attribute("id", "memory-search-results");
            dom::add(&f, &out);
            dom::add(&panel, &f);
        }
        let form = dom::el("form", "", "");
        let _ = form.set_attribute("id", &format!("{scope}-knowledge-form"));
        let select = dom::el("select", "", "");
        let _ = select.set_attribute("id", &format!("{scope}-knowledge-action"));
        for name in if scope == "brain" {
            vec![
                "start",
                "brain-create",
                "brain-update",
                "complete",
                "compact",
                "validate",
                "export",
            ]
        } else {
            vec![
                "status",
                "index",
                "validate",
                "bank-audit",
                "reindex-bank",
                "export",
                "bank-reverify",
                "bank-retire",
            ]
        } {
            let option = dom::el("option", "", &m::label(name));
            let _ = option.set_attribute("value", name);
            dom::add(&select, &option);
        }
        dom::add(&form, &select);
        for (suffix, class) in [
            ("note", "knowledge-note"),
            ("fields", "knowledge-form-grid"),
            ("error", "error-text"),
        ] {
            let e = dom::el(if suffix == "fields" { "div" } else { "p" }, class, "");
            let _ = e.set_attribute("id", &format!("{scope}-knowledge-{suffix}"));
            dom::add(&form, &e);
        }
        dom::add(
            &form,
            &dom::button("Use selected", &format!("{scope}-use-selected"), V::Null),
        );
        let b = dom::el("button", "button", "Run operation");
        let _ = b.set_attribute("type", "submit");
        let _ = b.set_attribute("id", &format!("{scope}-knowledge-submit"));
        dom::add(&form, &b);
        dom::add(&panel, &form);
        let out = dom::el("section", "knowledge-output", "");
        let _ = out.set_attribute("id", &format!("{scope}-knowledge-output"));
        dom::add(&panel, &out);
        dom::append(&format!("{scope}-tools"), &panel);
        fields(scope);
    }
}

#[allow(dead_code)]
pub(crate) fn on_global_project_change(project: &str) {
    let project = project.trim();
    if project.is_empty() {
        return;
    }
    let prev = m::string(&m::data("knowledge-global-project"));
    m::set("knowledge-global-project", json!(project));
    let active_view = m::read(|s| s.view.clone());
    for scope_id in ["brain-project", "memory-project"] {
        m::clear_scoped_project(scope_id);
    }
    if prev != project {
        reset_project("brain");
        reset_project("memory");
    }
    if !cfg!(target_arch = "wasm32") {
        return;
    }
    match active_view.as_str() {
        "context" => spawn_local(async {
            load_context().await;
        }),
        "memory-use" => spawn_local(async {
            load_memory_use().await;
        }),
        "brain" => spawn_local(async {
            load("brain", true).await;
        }),
        "memory" => spawn_local(async {
            load("memory", true).await;
        }),
        _ => {}
    }
}

pub(crate) fn hover(element: &web_sys::Element) -> bool {
    let action = element.get_attribute("data-action").unwrap_or_default();
    let payload = element
        .get_attribute("data-payload")
        .and_then(|v| serde_json::from_str::<V>(&v).ok())
        .unwrap_or(V::Null);
    match action.as_str() {
        "memory-retrieval" => {
            retrieval_detail(payload.as_u64().unwrap_or(0) as usize);
            true
        }
        "memory-horizon-chunk" => {
            chunk_detail(&payload);
            true
        }
        _ => false,
    }
}

pub async fn event(kind: &str, id: &str, action: &str, payload: &V, value: &str) -> bool {
    if kind == "input" && id == "memory-promotion-reviewer" {
        m::set("memory-promotion-reviewer", json!(value));
        dom::save(
            PROMOTION_REVIEWER_KEY,
            Some(value.trim()).filter(|name| !name.is_empty()),
        );
        return true;
    }
    if kind == "input" && (id == "memory-filter" || id == "brain-filter") {
        render_list(if id.starts_with("brain") {
            "brain"
        } else {
            "memory"
        });
        return true;
    }
    if kind == "change" {
        if id == "brain-category" {
            render_list("brain");
            return true;
        }
        if id == "brain-followup-review" {
            linked_controls();
            return true;
        }
        if id == "memory-use-bank" {
            load_memory_use().await;
            return true;
        }
        if id == "memory-use-window" {
            render_memory();
            return true;
        }
        if id == "memory-ui-filter" {
            m::set("memory-filter", json!(value));
            render_memory();
            return true;
        }
        for scope in ["brain", "memory"] {
            if id == format!("{scope}-bank") {
                load(scope, true).await;
                return true;
            }
            if id == format!("{scope}-knowledge-action") {
                fields(scope);
                tool_controls(scope);
                return true;
            }
        }
        if id == "brain-link-bank" {
            load_brain_link().await;
            return true;
        }
    }
    if kind == "submit" {
        if id == "memory-search-form" {
            let mut fields = json!({"query":dom::value("memory-search-query").trim()});
            let layer = dom::value("memory-search-layer");
            if !layer.is_empty() {
                fields["layer"] = json!(layer);
            }
            operation("memory", "search", fields).await;
            return true;
        }
        if id == "linked-context-form" {
            session::session_action(
                "context",
                json!({"query":dom::value("linked-context-query").trim()}),
                "linked-context-error",
            )
            .await;
            return true;
        }
        for scope in ["memory", "brain"] {
            if id == format!("{scope}-knowledge-form") {
                submit(scope).await;
                return true;
            }
        }
    }
    if (kind == "pointerover" || kind == "focusin") && action == "memory-retrieval" {
        retrieval_detail(payload.as_u64().unwrap_or(0) as usize);
        return true;
    }
    if (kind == "pointerover" || kind == "focusin") && action == "memory-horizon-chunk" {
        chunk_detail(payload);
        return true;
    }
    if kind != "click" {
        return false;
    }
    match action {
        "memory-window" => {
            m::set("memory-window", payload.clone());
            for e in dom::all("#memory-use-window button") {
                let selected = e
                    .get_attribute("data-payload")
                    .and_then(|s| serde_json::from_str::<V>(&s).ok())
                    .as_ref()
                    == Some(payload);
                let _ = e.set_attribute("aria-checked", if selected { "true" } else { "false" });
                let _ = e.set_attribute("tabindex", if selected { "0" } else { "-1" });
            }
            render_memory();
        }
        "memory-horizon-chunk" => {
            m::set("memory-selected-chunk", payload["id"].clone());
            chunk_detail(payload);
        }
        "memory-retrieval" => {
            let pos = payload.as_u64().unwrap_or(0) as usize;
            m::set("memory-pinned-retrieval", payload.clone());
            if let Some(section) = dom::get("memory-use-strip") {
                memory_use::sync_retrieval_strip_selection(&section, pos);
            }
            retrieval_detail(pos);
        }
        "memory-filter-chip" => {
            let name = payload.as_str().unwrap_or("all");
            let mut selected = m::array(&m::data("memory-filters"));
            if name == "all" {
                selected.clear();
            } else if selected.iter().any(|v| v == name) {
                selected.retain(|v| v != name);
            } else {
                selected.push(json!(name));
            }
            m::set("memory-filters", json!(selected));
            render_memory();
        }
        "memory-sort" => {
            let kind = payload["kind"].as_str().unwrap_or("chunk");
            let field = payload["field"].as_str().unwrap_or("id");
            let old = m::data(&format!("memory-sort-{kind}"));
            m::set(
                &format!("memory-sort-{kind}"),
                json!({"field":field,"desc":old["field"]==field&&old["desc"]!=true}),
            );
            render_memory();
        }
        "memory-chunk-action" => {
            let p = m::current_project();
            dom::set_value("memory-project", &p);
            let bank = m::data("memory-use")["bank_id"].clone();
            dom::set_value("memory-bank", &m::string(&bank));
            session::open_view("memory", true).await;
            let chunk = &payload["chunk"];
            if let Some(path) = chunk["path"].as_str() {
                select("memory", path).await;
            }
            if let Some(action) = payload["action"].as_str() {
                dom::set_value("memory-knowledge-action", action);
                fields("memory");
                dom::set_value("memory-op-memory_id", &m::text(chunk, "id"));
                if let Some(e) = dom::get("memory-knowledge-panel") {
                    let _ = e.set_attribute("open", "");
                }
                dom::focus("memory-op-memory_id");
            }
        }

        "refresh-context" => load_context().await,
        "refresh-memory" => load("memory", false).await,
        "refresh-brain" => load("brain", false).await,
        "knowledge-file" => {
            select(
                payload["scope"].as_str().unwrap_or("memory"),
                payload["path"].as_str().unwrap_or(""),
            )
            .await
        }
        "memory-use-refresh" => load_memory_use().await,
        "memory-use-check" => {
            let data = m::data("memory-use");
            match api::post(
                &format!(
                    "/api/projects/{}/memory-use/check",
                    dom::enc(&m::current_project())
                ),
                json!({"bank":data["bank_id"]}),
            )
            .await
            {
                Ok(v) => {
                    m::set("memory-use-check", v);
                    render_memory();
                }
                Err(e) => dom::error("memory-use-error", &e.message),
            }
        }
        "memory-feedback" => {
            let project = m::current_project();
            let bank = m::data("memory-use")["bank_id"].clone();
            let mut body = json!({
                "action": "retrieval-feedback",
                "manifest_id": payload["manifest_id"],
                "chunk_id": payload["chunk_id"],
                "vote": payload["vote"],
            });
            if bank.is_string() {
                body["bank"] = bank;
            }
            match api::post(
                &format!("/api/projects/{}/knowledge", dom::enc(&project)),
                body,
            )
            .await
            {
                Ok(_) => load_memory_use().await,
                Err(e) => dom::error("memory-use-error", &e.message),
            }
        }
        "memory-promotion-review" | "memory-promotion-apply" => {
            promotion_action(action, payload).await
        }
        "memory-promotion-brain" => {
            // Project Brain shows the source record Memory use names only by ID.
            let project = m::current_project();
            if let Some(bank) = m::data("memory-use")["bank_id"].as_str() {
                if dom::value("brain-project") == project {
                    dom::set_value("brain-bank", bank);
                }
            }
            open_from_omnigraph(
                "brain",
                &project,
                payload["record_id"].as_str().unwrap_or(""),
            )
            .await;
        }
        "memory-chunk-open" => {
            let p = m::current_project();
            dom::set_value("memory-project", &p);
            let bank = m::data("memory-use")["bank_id"].clone();
            dom::set_value("memory-bank", &m::string(&bank));
            session::open_view("memory", true).await;
            if let Some(path) = payload["path"].as_str() {
                select("memory", path).await;
            }
        }
        "brain-use-selected" => selected("brain"),
        "memory-use-selected" => selected("memory"),
        "brain-link-refresh" => load_brain_link().await,
        "linked-context-run" => {
            let context = m::read(|s| s.selected["brain"]["context_id"].clone());
            if context.is_string() {
                session::session_action(
                    "run",
                    json!({"context_id":context}),
                    "linked-context-error",
                )
                .await;
            }
        }
        "context-page" => {
            m::set("context-page", payload.clone());
            render_usage();
        }
        "context-turn" => {
            m::set("context-turn-selected", payload.clone());
            render_usage();
        }
        "context-latest" => {
            m::take("context-turn-selected");
            m::set("context-page", json!(0));
            render_usage();
        }
        "context-meter" => session::open_view("usage", true).await,
        "knowledge-open-omnigraph" => {
            let project = payload["project_id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    let p = dom::value("brain-project");
                    if p.is_empty() {
                        dom::value("memory-project")
                    } else {
                        p
                    }
                });
            let atom_id = payload["atom_id"].as_str().unwrap_or("").trim().to_owned();
            crate::omnigraph::open_knowledge_atom(&project, &atom_id).await;
        }
        _ => return false,
    }
    true
}

pub fn key_event(event: &web_sys::KeyboardEvent) -> bool {
    let Some(target) = event
        .target()
        .and_then(|v| v.dyn_into::<web_sys::Element>().ok())
    else {
        return false;
    };
    let Some(container) = target
        .closest("[data-memory-options],#memory-use-window,#context-turns")
        .ok()
        .flatten()
    else {
        return false;
    };
    let buttons = dom::children(&container)
        .into_iter()
        .flat_map(|e| {
            if e.tag_name() == "BUTTON" {
                vec![e]
            } else {
                e.query_selector_all("button")
                    .ok()
                    .map(|l| {
                        (0..l.length())
                            .filter_map(|i| l.item(i)?.dyn_into::<web_sys::Element>().ok())
                            .collect()
                    })
                    .unwrap_or_default()
            }
        })
        .collect::<Vec<_>>();
    let Some(index) = buttons.iter().position(|e| e == &target) else {
        return false;
    };
    let next = match event.key().as_str() {
        "ArrowRight" | "ArrowDown" => Some((index + 1).min(buttons.len() - 1)),
        "ArrowLeft" | "ArrowUp" => Some(index.saturating_sub(1)),
        "Home" => Some(0),
        "End" => Some(buttons.len() - 1),
        "Enter" => Some(index),
        _ => None,
    };
    let Some(next) = next else { return false };
    for (i, b) in buttons.iter().enumerate() {
        let _ = b.set_attribute("tabindex", if i == next { "0" } else { "-1" });
    }
    if let Some(e) = buttons[next].dyn_ref::<web_sys::HtmlElement>() {
        let _ = e.focus();
        e.click();
    }
    true
}

#[cfg(test)]
#[allow(unused_imports)]
mod test_exports {
    pub(super) use super::memory_use::*;
    pub(super) use super::records::*;
    pub(super) use super::usage::*;
}
