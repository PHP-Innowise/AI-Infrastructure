use crate::{
    api, commands, dom, inbox, knowledge, management, model as m, omnigraph, run, systems,
};
use serde_json::{json, Value as V};
use std::cell::RefCell;
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::{spawn_local, JsFuture};

mod advisory;
mod composer;
mod navigation;
mod poll;
mod prefs;
mod results;
#[cfg(test)]
mod tests;

pub(crate) use advisory::*;
pub use composer::*;
pub use navigation::*;
pub use poll::*;
pub use prefs::*;
pub use results::*;

thread_local! {
    pub(super) static FILES: RefCell<Vec<web_sys::File>> = const { RefCell::new(Vec::new()) };
    static CAPTURED_GRAPH_CLICK: RefCell<Option<CapturedGraphClick>> = const { RefCell::new(None) };
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ClickAttrs {
    resolved_id: String,
    resolved_action: String,
    path: Option<String>,
    from: Option<String>,
    to: Option<String>,
    filter: Option<String>,
    tab: Option<String>,
    fallback_payload: Option<V>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct CapturedGraphClick {
    lib_id: String,
    lib_action: String,
    in_workspace: bool,
    attrs: ClickAttrs,
}

pub(super) fn closest_scoped_attr(
    target: &web_sys::Element,
    fallback: &web_sys::Element,
    attr: &str,
    require_workspace: bool,
) -> Option<String> {
    let selector = format!("[{attr}]");
    if let Ok(Some(found)) = target.closest(&selector) {
        if !require_workspace
            || found
                .closest("#session-graph-workspace")
                .ok()
                .flatten()
                .is_some()
        {
            if let Some(val) = found.get_attribute(attr).filter(|v| !v.is_empty()) {
                return Some(val);
            }
        }
    }
    fallback.get_attribute(attr).filter(|v| !v.is_empty())
}

pub(super) fn capture_graph_click(target: &web_sys::Element) {
    let lib_el = target
        .closest("[data-action],button,a[data-view],input[type=checkbox]")
        .ok()
        .flatten()
        .unwrap_or_else(|| target.clone());
    let lib_id = lib_el.id();
    let lib_action = lib_el
        .get_attribute("data-action")
        .unwrap_or_else(|| lib_id.clone());
    let in_workspace = target
        .closest("#session-graph-workspace")
        .ok()
        .flatten()
        .is_some()
        || lib_el
            .closest("#session-graph-workspace")
            .ok()
            .flatten()
            .is_some();
    let is_inspect = lib_action == "inspect-file-graph";
    if !in_workspace && !is_inspect {
        CAPTURED_GRAPH_CLICK.with(|c| *c.borrow_mut() = None);
        return;
    }
    let sem_el = if in_workspace {
        target
            .closest("[data-action],button,.graph-side-tab,.graph-changes-file,.graph-changes-entity,.graph-impact-row,#session-graph-side,#session-graph-mode-whole,#session-graph-mode-changes,#session-graph-unfold,#session-graph-open-full,[data-path],[data-from],[data-to],[data-filter],[data-tab]")
            .ok()
            .flatten()
            .filter(|el| {
                el.closest("#session-graph-workspace")
                    .ok()
                    .flatten()
                    .is_some()
            })
            .unwrap_or_else(|| lib_el.clone())
    } else {
        lib_el.clone()
    };
    let resolved_id = if !lib_id.is_empty() {
        lib_id.clone()
    } else {
        sem_el.id()
    };
    let resolved_action = sem_el
        .get_attribute("data-action")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            if !lib_action.is_empty() {
                lib_action.clone()
            } else {
                resolved_id.clone()
            }
        });
    let fallback_payload = sem_el
        .get_attribute("data-payload")
        .and_then(|raw| serde_json::from_str::<V>(&raw).ok());
    let attrs = ClickAttrs {
        resolved_id,
        resolved_action,
        path: closest_scoped_attr(target, &lib_el, "data-path", in_workspace),
        from: closest_scoped_attr(target, &lib_el, "data-from", in_workspace),
        to: closest_scoped_attr(target, &lib_el, "data-to", in_workspace),
        filter: closest_scoped_attr(target, &lib_el, "data-filter", in_workspace),
        tab: closest_scoped_attr(target, &lib_el, "data-tab", in_workspace),
        fallback_payload,
    };
    CAPTURED_GRAPH_CLICK.with(|c| {
        *c.borrow_mut() = Some(CapturedGraphClick {
            lib_id,
            lib_action,
            in_workspace,
            attrs,
        });
    });
}

pub(super) fn merge_click_payload(payload: &V, attrs: &ClickAttrs, in_workspace: bool) -> V {
    let base = if payload.is_null() {
        attrs.fallback_payload.as_ref().unwrap_or(payload)
    } else {
        payload
    };
    let has_attrs = attrs.path.is_some()
        || attrs.from.is_some()
        || attrs.to.is_some()
        || attrs.filter.is_some()
        || attrs.tab.is_some();
    if !base.is_null() && !base.is_object() {
        return base.clone();
    }
    if !base.is_object() && !in_workspace && !has_attrs {
        return base.clone();
    }
    let mut map = base.as_object().cloned().unwrap_or_default();
    for (key, value) in [
        ("path", &attrs.path),
        ("from", &attrs.from),
        ("to", &attrs.to),
        ("filter", &attrs.filter),
        ("tab", &attrs.tab),
    ] {
        if let Some(val) = value {
            let empty_or_missing = map
                .get(key)
                .is_none_or(|cur| cur.is_null() || cur.as_str() == Some(""));
            if !val.is_empty() && empty_or_missing {
                map.insert(key.to_owned(), V::String(val.clone()));
            }
        }
    }
    if in_workspace && !map.contains_key("surface") {
        map.insert("surface".to_owned(), json!("session"));
    }
    if map.is_empty() && base.is_null() {
        V::Null
    } else {
        V::Object(map)
    }
}

pub(super) fn extract_inspect_file_path(payload: &V) -> Option<String> {
    payload
        .as_str()
        .or_else(|| payload["path"].as_str())
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
}

pub(super) fn is_mutation_tool_event(ev: &V) -> bool {
    if m::text(ev, "kind") != "tool" {
        return false;
    }
    if ev["state"]
        .as_str()
        .is_some_and(|s| s == "started" || s == "in_progress")
    {
        return false;
    }
    let text = m::text(ev, "text");
    let tail = text.split_once(": ").map(|(_, s)| s).unwrap_or("");
    if tail.starts_with("failed") || tail.starts_with("declined") || ev["ok"] == json!(false) {
        return false;
    }
    let still_starting = ev["state"].is_null()
        && !["completed", "failed", "declined"]
            .iter()
            .any(|p| tail.starts_with(p));
    if still_starting && m::array(&ev["changes"]).is_empty() {
        return false;
    }
    let raw = ev["tool"]
        .as_str()
        .unwrap_or_else(|| text.split(": ").next().unwrap_or(""));
    let norm = raw
        .strip_suffix("ToolCall")
        .unwrap_or(raw)
        .to_ascii_lowercase();
    matches!(
        norm.as_str(),
        "edit" | "write" | "multiedit" | "notebookedit" | "file_change" | "delete"
    ) || !m::array(&ev["changes"]).is_empty()
}

pub(super) fn should_sync_embedded_changes(
    fresh_events: &[V],
    focus_changed: bool,
    was_active: bool,
    is_active: bool,
) -> bool {
    if (was_active && !is_active) || focus_changed {
        return true;
    }
    fresh_events.iter().any(|ev| {
        let kind = m::text(ev, "kind");
        if kind == "graph_focus" {
            return true;
        }
        if kind == "status" {
            let status = ev["status"]
                .as_str()
                .unwrap_or_else(|| ev["text"].as_str().unwrap_or(""));
            return ["completed", "failed", "cancelled", "interrupted"].contains(&status);
        }
        is_mutation_tool_event(ev)
    })
}

pub(super) fn template_content_node(id: &str) -> Option<web_sys::Node> {
    let template = dom::get(id)?;
    js_sys::Reflect::get(&template, &"content".into())
        .ok()
        .and_then(|v| v.dyn_into::<web_sys::Node>().ok())
}

pub(super) fn hydrate_rabbit_svgs() {
    let Some(content) = template_content_node("rabbit-svg-template") else {
        return;
    };
    for slot in dom::all(".composer-rabbit") {
        if slot.first_element_child().is_some() {
            continue;
        }
        if let Ok(cloned) = content.clone_node_with_deep(true) {
            let _ = slot.append_child(&cloned);
        }
    }
}

pub(super) fn hydrate_embedded_graph_panels() {
    let Some(content) = template_content_node("embedded-graph-panel-template") else {
        return;
    };
    for host in dom::all("[data-graph-prefix]") {
        if host.first_element_child().is_some() {
            continue;
        }
        let Some(prefix) = host
            .get_attribute("data-graph-prefix")
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let heading_tag = host
            .get_attribute("data-graph-heading")
            .filter(|s| matches!(s.as_str(), "h2" | "h3" | "h4"))
            .unwrap_or_else(|| "h4".into());
        let title = host
            .get_attribute("data-graph-title")
            .unwrap_or_else(|| "Code graph".into());
        let scope = host
            .get_attribute("data-graph-scope")
            .unwrap_or_else(|| "Code graph".into());
        let Ok(cloned) = content.clone_node_with_deep(true) else {
            continue;
        };
        if host.append_child(&cloned).is_err() {
            continue;
        }
        if let Ok(nodes) = host.query_selector_all("[data-graph-id]") {
            for i in 0..nodes.length() {
                if let Some(el) = nodes
                    .item(i)
                    .and_then(|v| v.dyn_into::<web_sys::Element>().ok())
                {
                    if let Some(suffix) = el.get_attribute("data-graph-id") {
                        let _ = el.set_attribute("id", &format!("{prefix}-{suffix}"));
                        let _ = el.remove_attribute("data-graph-id");
                    }
                }
            }
        }
        for (source_attr, target_attr) in [
            ("data-graph-controls", "aria-controls"),
            ("data-graph-labelledby", "aria-labelledby"),
        ] {
            let selector = format!("[{source_attr}]");
            if let Ok(nodes) = host.query_selector_all(&selector) {
                for i in 0..nodes.length() {
                    if let Some(el) = nodes
                        .item(i)
                        .and_then(|v| v.dyn_into::<web_sys::Element>().ok())
                    {
                        if let Some(suffix) = el.get_attribute(source_attr) {
                            let _ = el.set_attribute(target_attr, &format!("{prefix}-{suffix}"));
                            let _ = el.remove_attribute(source_attr);
                        }
                    }
                }
            }
        }
        if let Ok(nodes) = host.query_selector_all("[data-graph-role]") {
            for i in 0..nodes.length() {
                let Some(el) = nodes
                    .item(i)
                    .and_then(|v| v.dyn_into::<web_sys::Element>().ok())
                else {
                    continue;
                };
                let role = el.get_attribute("data-graph-role").unwrap_or_default();
                let _ = el.remove_attribute("data-graph-role");
                match role.as_str() {
                    "title" => {
                        if heading_tag != "h4" {
                            let heading = dom::el(&heading_tag, "session-graph-title", &title);
                            let _ = heading.set_attribute("id", &format!("{prefix}-title"));
                            let _ = el.replace_with_with_node_1(&heading);
                        } else {
                            el.set_text_content(Some(&title));
                        }
                    }
                    "toolbar" => {
                        let _ = el.set_attribute("aria-label", &format!("{scope} controls"));
                    }
                    "filter-label" => {
                        el.set_text_content(Some(if prefix == "session-graph" {
                            "Filter graph names"
                        } else {
                            "Filter service graph names"
                        }));
                    }
                    "crumbs" => {
                        let label = match prefix.as_str() {
                            "session-graph" => "Graph breadcrumbs",
                            "system-member-graph" => "Service graph breadcrumbs",
                            _ => "Change service graph breadcrumbs",
                        };
                        let _ = el.set_attribute("aria-label", label);
                    }
                    "canvas" => {
                        let label = match prefix.as_str() {
                            "session-graph" => "Interactive code graph canvas",
                            "system-member-graph" => "Interactive service code graph canvas",
                            _ => "Interactive change service code graph canvas",
                        };
                        let _ = el.set_attribute("aria-label", label);
                    }
                    "side" => {
                        let label = match prefix.as_str() {
                            "session-graph" => "Selected code unit details",
                            "system-member-graph" => "Selected service code unit details",
                            _ => "Selected change service code unit details",
                        };
                        let _ = el.set_attribute("aria-label", label);
                    }
                    _ => {}
                }
            }
        }
    }
}

pub(super) fn hydrate_templates() {
    hydrate_rabbit_svgs();
    hydrate_embedded_graph_panels();
}

pub fn initialize() {
    hydrate_templates();
    for prefix in ["session-budgets", "creator-budgets", "creator-run-budgets"] {
        budget_editor(prefix);
    }
    for (role, label) in [("plan", "Planning"), ("edit", "Editing")] {
        for (suffix, title, tag) in [
            ("model-choice", format!("{label} model"), "select"),
            ("model", format!("{label} custom model ID"), "input"),
            (
                "thinking-effort",
                format!("{label} thinking effort"),
                "select",
            ),
        ] {
            let field = dom::el("label", "field", &title);
            let input = dom::el(tag, "", "");
            let _ = input.set_attribute("id", &format!("routing-{role}-{suffix}"));
            let _ = input.set_attribute("form", "session-form");
            if suffix == "model" {
                let _ = field.set_attribute("id", &format!("routing-{role}-custom-field"));
                let _ = input.set_attribute("maxlength", "120");
            }
            dom::add(&field, &input);
            dom::append("model-routing-fields", &field);
        }
    }
    let preference = dom::storage("harness.theme.v1").unwrap_or_else(|| "system".into());
    apply_theme(&preference);
    m::set("run-preferences", dom::json_storage("harness.run-view.v1"));
    let c = Closure::<dyn FnMut(web_sys::Event)>::new(|_| {
        apply_theme(&dom::storage("harness.theme.v1").unwrap_or_else(|| "system".into()));
    });
    let _ = dom::window().add_event_listener_with_callback("storage", c.as_ref().unchecked_ref());
    c.forget();
    if let Ok(Some(media)) = dom::window().match_media("(prefers-color-scheme: dark)") {
        let c = Closure::<dyn FnMut(web_sys::Event)>::new(|_| {
            apply_theme(&dom::storage("harness.theme.v1").unwrap_or_else(|| "system".into()))
        });
        let _ = media.add_event_listener_with_callback("change", c.as_ref().unchecked_ref());
        c.forget();
    }
    if let Ok(Some(mobile_media)) = dom::window().match_media("(max-width:920px)") {
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(|_| {
            if m::read(|s| s.view == "sessions") {
                crate::graph_map::sync_embedded(false);
            }
        });
        let _ =
            mobile_media.add_event_listener_with_callback("change", cb.as_ref().unchecked_ref());
        cb.forget();
    }
    let capture_click = Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
        if let Some(target) = event
            .target()
            .and_then(|v| v.dyn_into::<web_sys::Element>().ok())
        {
            capture_graph_click(&target);
        } else {
            CAPTURED_GRAPH_CLICK.with(|c| *c.borrow_mut() = None);
        }
    });
    let _ = dom::document().add_event_listener_with_callback_and_bool(
        "click",
        capture_click.as_ref().unchecked_ref(),
        true,
    );
    capture_click.forget();
    dom::wire_a11y_widgets();
}
pub(super) fn apply_theme(preference: &str) {
    let dark = dom::window()
        .match_media("(prefers-color-scheme: dark)")
        .ok()
        .flatten()
        .map(|m| m.matches())
        .unwrap_or(false);
    let selected = if preference == "dark" || preference == "light" {
        preference
    } else if dark {
        "dark"
    } else {
        "light"
    };
    if let Some(e) = dom::document().document_element() {
        let _ = e.set_attribute("data-theme", selected);
    }
    for e in dom::all("input[name=theme]") {
        if let Some(i) = e.dyn_ref::<web_sys::HtmlInputElement>() {
            i.set_checked(i.value() == preference);
        }
    }
}
pub async fn bootstrap() {
    dom::hidden("connecting", false);
    dom::disabled("reconnect", true);
    match api::get("/api/bootstrap").await {
        Ok(boot)
            if boot["csrf"].is_string()
                && boot["projects"].is_array()
                && boot["providers"].is_array() =>
        {
            m::set("app-update", boot["update"].clone());
            let boot_omnigraph = boot["omnigraph"].clone();
            m::write(|s| {
                s.sessions = m::array(&boot["sessions"]);
                s.boot = boot;
                s.auth_failed = false;
            });
            reset_session_settings_state();
            dom::hidden("connection-banner", true);
            render_update();
            spawn_local(async {
                update_status(true).await;
            });
            populate();
            history();
            restore_preferences();
            if boot_omnigraph.is_object() {
                omnigraph::hydrate_status(&boot_omnigraph);
            }
            spawn_local(async {
                omnigraph::refresh_pill().await;
            });
            spawn_local(async {
                sign_in(false).await;
            });
            m::set("bootstrapping", json!(true));
            let route = dom::hash_route();
            let wanted = dom::json_storage(PREFS);
            let sid = if !route.session.is_empty() {
                route.session.clone()
            } else {
                m::text(&wanted, "session_id")
            };
            if !sid.is_empty() && m::read(|s| s.sessions.iter().any(|x| m::text(x, "id") == sid)) {
                select(&sid).await;
            }
            apply_hash_route().await;
            m::take("bootstrapping");
            initialize_sse_listeners();
            api::start_sse();
            run::sync_approvals_badge();
            if m::read(|s| s.view != "sessions") && !dom::value("project").is_empty() {
                crate::graph_map::sync_embedded(false);
                spawn_local(async {
                    crate::graph::load(true).await;
                });
            }
        }
        Ok(_) => dom::error(
            "connection-message",
            "The runner returned incomplete workspace configuration.",
        ),
        Err(e) => {
            dom::error("connection-message", &e.message);
            dom::hidden("connection-banner", false);
        }
    }
    dom::hidden("connecting", true);
    dom::disabled("reconnect", false);
    controls();
}
pub fn populate() {
    let boot = m::read(|s| s.boot.clone());
    let projects = m::array(&boot["projects"]);
    for id in [
        "project",
        "project-switcher",
        "context-project",
        "memory-use-project",
        "memory-project",
        "brain-project",
        "skills-project",
        "create-skill-project",
        "setup-project",
        "creator-project",
        "accelerator-project",
        "system-project",
        "omnigraph-project",
    ] {
        dom::options(id, &projects, m::summary, None);
    }
    dom::options(
        "provider",
        &m::array(&boot["providers"]),
        |p| {
            format!(
                "{}{}",
                m::summary(p),
                if m::yes(&p["available"]) {
                    ""
                } else {
                    " · unavailable"
                }
            )
        },
        None,
    );
    dom::options("workflow", &m::array(&boot["workflows"]), m::summary, None);
    dom::options(
        "sdd-phase",
        &m::array(&boot["sdd_phases"]),
        m::summary,
        None,
    );
    for id in [
        "creator-provider",
        "system-provider",
        "system-discovery-provider",
        "clash-challenger",
    ] {
        dom::options(id, &m::array(&boot["providers"]), m::summary, None);
    }
    let editions = m::array(&boot["accelerators"])
        .into_iter()
        .find(|kit| kit["id"] == "kit2")
        .map(|kit| m::array(&kit["editions"]))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|edition| edition.as_str().map(|id| json!({"id":id,"name":id})))
        .collect::<Vec<_>>();
    for id in ["setup-edition", "setup-attach-edition"] {
        dom::options(id, &editions, m::summary, None);
    }
    dom::clear("fleet-lenses");
    for lens in m::array(&boot["runtime"]["fleet"]["lenses"]) {
        let e = dom::check(
            &m::summary(&lens),
            "fleet-lens",
            &m::text(&lens, "id"),
            true,
        );
        dom::append("fleet-lenses", &e);
    }
    let count = boot["runtime"]["default_agent_count"]
        .as_u64()
        .unwrap_or(3)
        .to_string();
    if dom::value("agent-count").is_empty() {
        dom::set_value("agent-count", &count);
    }
    dom::attr(
        "agent-count",
        "max",
        &boot["runtime"]["max_agents"]
            .as_u64()
            .unwrap_or(40)
            .to_string(),
    );
    let slots = boot["runtime"]["max_active"].as_u64().unwrap_or(1);
    dom::text(
        "runtime-label",
        &format!(
            "Local runner · {slots} active slot{}",
            if slots == 1 { "" } else { "s" }
        ),
    );
    for prefix in ["", "clash-", "routing-plan-", "routing-edit-"] {
        models(prefix, None);
    }
    management::creator_models();
    controls();
    management::accelerators();
}
pub fn models(prefix: &str, requested: Option<String>) {
    let provider = m::provider(&dom::value(if prefix == "clash-" {
        "clash-challenger"
    } else {
        "provider"
    }));
    let metadata = &provider["model_options"];
    let preserve_custom =
        requested.is_none() && dom::value(&format!("{prefix}model-choice")) == "--custom--";
    let old = requested.unwrap_or_else(|| selected_model(prefix).as_str().unwrap_or("").into());
    let mut entries = vec![json!({"id":"","label":"Provider default"})];
    entries.extend(m::array(&metadata["models"]));
    entries.push(json!({"id":"--custom--","label":"Custom model…"}));
    dom::options(
        &format!("{prefix}model-choice"),
        &entries,
        |v| {
            v["label"]
                .as_str()
                .unwrap_or_else(|| v["id"].as_str().unwrap_or(""))
                .into()
        },
        Some(&old),
    );
    let custom = preserve_custom || !old.is_empty() && !entries.iter().any(|x| x["id"] == old);
    if custom {
        dom::set_value(&format!("{prefix}model-choice"), "--custom--");
        dom::set_value(&format!("{prefix}model"), &old);
    }
    let chosen = selected_model(prefix);
    let model = m::array(&metadata["models"])
        .into_iter()
        .find(|x| x["id"] == chosen)
        .unwrap_or(V::Null);
    let efforts = if model["efforts"].is_array() {
        m::array(&model["efforts"])
    } else {
        m::array(&metadata["efforts"])
    };
    let mut options = vec![json!({"id":"","name":"Provider default"})];
    for e in efforts {
        options.push(json!({"id":e,"name":m::label(e.as_str().unwrap_or(""))}));
    }
    dom::options(
        &format!("{prefix}thinking-effort"),
        &options,
        m::summary,
        None,
    );
    let custom_field = if prefix.is_empty() {
        "custom-model-field".to_owned()
    } else {
        format!("{prefix}custom-field")
    };
    dom::hidden(
        &custom_field,
        !custom && dom::value(&format!("{prefix}model-choice")) != "--custom--",
    );
    dom::disabled(
        &format!("{prefix}model"),
        dom::value(&format!("{prefix}model-choice")) != "--custom--",
    );
    if let Some(input) = dom::get(&format!("{prefix}model")) {
        if custom {
            let _ = input.set_attribute("required", "");
        } else {
            let _ = input.remove_attribute("required");
        }
    }
    dom::text("model-hint", metadata["detail"].as_str().unwrap_or(""));
}
pub(super) fn restore_model_routing(value: &V) {
    dom::set_checked("model-routing-enabled", value.is_object());
    for role in ["plan", "edit"] {
        let prefix = format!("routing-{role}-");
        models(&prefix, Some(m::text(&value[role], "model")));
        dom::set_value(
            &format!("{prefix}thinking-effort"),
            &m::text(&value[role], "thinking_effort"),
        );
    }
}
pub fn selected_model(prefix: &str) -> V {
    let choice = dom::value(&format!("{prefix}model-choice"));
    let v = if choice == "--custom--" {
        dom::value(&format!("{prefix}model")).trim().into()
    } else {
        choice
    };
    if v.is_empty() {
        V::Null
    } else {
        json!(v)
    }
}
pub(crate) fn model_selection_valid(custom_selected: bool, selected: &V) -> bool {
    match selected.as_str() {
        Some(x) => x.len() <= 120 && !x.starts_with('-') && !x.chars().any(char::is_control),
        None => !custom_selected,
    }
}
pub fn routing() -> V {
    if !dom::checked("model-routing-enabled")
        || dom::value("workflow") == "fleet-review"
        || dom::checked("clash-enabled")
    {
        V::Null
    } else {
        json!({"plan":{"model":selected_model("routing-plan-"),"thinking_effort":nullable(dom::value("routing-plan-thinking-effort"))},"edit":{"model":selected_model("routing-edit-"),"thinking_effort":nullable(dom::value("routing-edit-thinking-effort"))}})
    }
}
pub fn nullable(v: String) -> V {
    if v.is_empty() {
        V::Null
    } else {
        json!(v)
    }
}
pub fn budget_editor(prefix: &str) {
    let panel = dom::el("details", "fleet-report budget-editor", "");
    let _ = panel.set_attribute("open", "");
    dom::add(&panel, &dom::el("summary", "", "Budgets"));
    for agent in [false, true] {
        let box_ = dom::el("div", "page-toolbar", "");
        for (key, label, min, max) in [
            ("usd", "Money (USD)", "0.01", "1000"),
            ("tokens", "Total tokens", "1", "1000000000"),
            ("seconds", "Time (seconds)", "1", "86400"),
        ] {
            let field = dom::el(
                "label",
                "field",
                &format!("{}{label}", if agent { "Per agent · " } else { "" }),
            );
            let input = dom::el("input", "", "");
            let _ = input.set_attribute(
                "id",
                &format!("{prefix}-{}{key}", if agent { "agent-" } else { "" }),
            );
            let _ = input.set_attribute("type", "number");
            let _ = input.set_attribute("min", min);
            let _ = input.set_attribute("max", max);
            let _ = input.set_attribute("step", if key == "usd" { "any" } else { "1" });
            dom::add(&field, &input);
            dom::add(&box_, &field);
        }
        dom::add(&panel, &box_);
    }
    dom::add(
        &panel,
        &dom::button(
            "Fill totals above",
            &format!("{prefix}-agent-apply"),
            V::Null,
        ),
    );
    let err = dom::el("p", "error-text", "");
    let _ = err.set_attribute("id", &format!("{prefix}-error"));
    dom::add(&panel, &err);
    if prefix != "creator-budgets" {
        dom::add(
            &panel,
            &dom::button("Save budgets", &format!("{prefix}-save"), V::Null),
        );
    }
    dom::append(prefix, &panel);
}
pub fn budgets(prefix: &str) -> V {
    let mut v = json!({});
    for k in ["usd", "tokens", "seconds"] {
        v[k] = dom::num(&format!("{prefix}-{k}"));
    }
    if prefix == "session-budgets" && dom::value("workflow") == "fleet-review" {
        v["usd"] = dom::num("fleet-budget");
    }
    v
}
pub(super) fn valid_budgets(v: &V) -> bool {
    ["usd", "tokens", "seconds"].iter().all(|k| {
        v[*k].is_null()
            || m::number(&v[*k])
                .map(|n| match *k {
                    "usd" => (0.01..=1000.).contains(&n),
                    "tokens" => (1.0..=1000000000.).contains(&n) && n.fract() == 0.,
                    _ => (1.0..=86400.).contains(&n) && n.fract() == 0.,
                })
                .unwrap_or(false)
    })
}
pub fn restore_budgets(prefix: &str, v: &V) {
    for k in ["usd", "tokens", "seconds"] {
        dom::set_value(&format!("{prefix}-{k}"), &m::string(&v[k]));
    }
}
pub(super) fn split_budgets(prefix: &str) {
    if !["usd", "tokens", "seconds"]
        .iter()
        .all(|key| dom::numeric_valid(&format!("{prefix}-agent-{key}")))
    {
        dom::error(
            &format!("{prefix}-error"),
            "Use valid per-agent money, token and time limits.",
        );
        return;
    }
    let count = dom::num(if prefix == "session-budgets" {
        "agent-count"
    } else {
        "creator-count"
    })
    .as_f64()
    .unwrap_or(1.0);
    let fleet = prefix == "session-budgets" && dom::value("workflow") == "fleet-review";
    let total = if fleet {
        dom::selected_checks("fleet-lenses").len() as f64
    } else if dom::checked(if prefix == "session-budgets" {
        "agents-enabled"
    } else {
        "creator-agents"
    }) {
        1. + count
    } else {
        1.
    };
    let mut v = json!({});
    for k in ["usd", "tokens", "seconds"] {
        let n = dom::num(&format!("{prefix}-agent-{k}"));
        v[k] = m::number(&n)
            .map(|n| {
                json!(if k == "seconds" {
                    n * if fleet {
                        (total / count.max(1.)).ceil()
                    } else {
                        1.
                    }
                } else {
                    n * total
                })
            })
            .unwrap_or(V::Null);
    }
    if valid_budgets(&v) {
        restore_budgets(prefix, &v);
        if fleet {
            dom::set_value("fleet-budget", &m::string(&v["usd"]));
        }
    } else {
        dom::error(
            &format!("{prefix}-error"),
            "The per-agent totals exceed the supported budget limits.",
        );
    }
}
pub fn controls() {
    let inbox_retry = inbox::needs_confirmation();
    let (sid, session, locked, ready) = m::read(|s| {
        (
            s.sid.clone(),
            s.selected.clone(),
            s.pending || m::active(&s.selected) || inbox_retry,
            !s.boot.is_null() && !s.auth_failed,
        )
    });
    let provider = m::provider(&dom::value("provider"));
    let workflow = dom::value("workflow");
    let fleet = workflow == "fleet-review";
    let sdd = workflow == "sdd";
    let feature = dom::value("sdd-feature").trim().to_owned();
    let phase = dom::value("sdd-phase");
    let phases = m::read(|s| m::array(&s.boot["sdd_phases"]));
    let sdd_valid = !sdd || m::sdd_settings_valid(&feature, &phase, &phases);
    if sdd {
        dom::set_value("mode", m::sdd_mode(&phase));
    } else if ["plan", "review", "fleet-review"].contains(&workflow.as_str()) {
        dom::set_value("mode", "plan");
    } else if !sid.is_empty() && session["workflow"] == "native" && routing().is_null() {
        dom::set_value("mode", &m::text(&session, "mode"));
    }
    let dry = fleet && dom::checked("fleet-dry-run");
    let prompt = dom::value("prompt");
    let count = dom::num("agent-count").as_u64().unwrap_or(0);
    let max = m::read(|s| s.boot["runtime"]["max_agents"].as_u64().unwrap_or(40));
    let helper_valid = !dom::checked("agents-enabled") || (1..=max).contains(&count);
    let model = selected_model("");
    let custom_model = dom::value("model-choice") == "--custom--";
    let model_valid = model_selection_valid(custom_model, &model);
    let inbox_active = inbox::eligible(&session);
    let valid = ready
        && !m::read(|s| s.pending)
        && if inbox_retry {
            true
        } else if m::active(&session) {
            inbox::can_send(&session, &prompt) && FILES.with(|files| files.borrow().is_empty())
        } else {
            ready
                && !locked
                && helper_valid
                && model_valid
                && sdd_valid
                && budget_fields_valid("session-budgets")
                && valid_budgets(&budgets("session-budgets"))
                && !prompt.trim().is_empty()
                && (dry || m::yes(&provider["available"]))
                && session["workflow"] != "fleet-review"
        };
    let project_id = dom::value("project");
    dom::disabled("preflight-check", !ready || project_id.is_empty());
    dom::disabled("preflight-risk-callers", !ready || project_id.is_empty());
    dom::disabled("send", !valid);
    dom::hidden("cancel-session", sid.is_empty() || !m::active(&session));
    dom::disabled(
        "cancel-session",
        !m::active(&session) || m::read(|s| s.pending),
    );
    dom::hidden("welcome", !sid.is_empty());
    dom::text(
        "session-status",
        session["status"].as_str().unwrap_or("New session"),
    );
    dom::text(
        "send-label",
        if m::read(|s| s.pending) {
            "Sending…"
        } else if inbox_retry && inbox::unreadable() {
            "Clear saved state"
        } else if inbox_retry {
            "Retry saving"
        } else if inbox_active {
            "Queue next message"
        } else if sid.is_empty() {
            "Send"
        } else {
            "Send follow-up"
        },
    );
    for id in [
        "project",
        "provider",
        "workflow",
        "workspace",
        "worktree-branch",
        "existing-worktree",
    ] {
        dom::disabled(id, !ready || locked || !sid.is_empty());
    }
    let git = m::data("project-git");
    let worktree_ok = m::yes(&git["worktree_available"]);
    let worktrees = m::array(&m::data("project-worktrees")["worktrees"]);
    dom::disabled(
        "refresh-project-git",
        !ready || locked || project_id.is_empty(),
    );
    dom::disabled("workspace-worktree-option", !worktree_ok);
    dom::disabled(
        "workspace-existing-option",
        !worktree_ok || worktrees.is_empty(),
    );
    if git.is_object() {
        dom::text("project-git-status", &format_project_git_status(&git));
        dom::text(
            "workspace-hint",
            &format_workspace_hint(&dom::value("workspace"), &git),
        );
    }
    for id in [
        "model-choice",
        "thinking-effort",
        "agents-enabled",
        "mode",
        "prompt",
        "attach-files",
    ] {
        dom::disabled(id, !ready || locked);
    }
    dom::disabled(
        "agent-count",
        !ready || locked || !dom::checked("agents-enabled"),
    );
    dom::disabled(
        "prompt",
        !ready || m::read(|s| s.pending) || inbox_retry || (m::active(&session) && !inbox_active),
    );
    dom::attr(
        "prompt",
        "placeholder",
        if inbox_active {
            "Write the next message…"
        } else {
            "Describe a task or follow-up…"
        },
    );
    if inbox_active || inbox_retry {
        commands::close();
    }
    dom::hidden("fleet-settings", !fleet);
    for id in ["fleet-dry-run", "fleet-budget", "fleet-worker-timeout"] {
        dom::disabled(id, !ready || locked || !fleet);
    }
    let sdd_locked = !ready || locked || session["status"] == "awaiting_context";
    dom::hidden("sdd-settings", !sdd);
    dom::disabled("sdd-feature", !sdd || sdd_locked || !sid.is_empty());
    dom::disabled("sdd-phase", !sdd || sdd_locked);
    if let Some(input) = dom::get("sdd-feature") {
        if sdd {
            let _ = input.set_attribute("required", "");
        } else {
            let _ = input.remove_attribute("required");
        }
    }
    dom::disabled(
        "mode",
        !ready
            || locked
            || ["plan", "review", "fleet-review", "sdd"].contains(&workflow.as_str())
            || !sid.is_empty() && routing().is_null(),
    );
    // Read-only workflows never edit, so their launches never ask.
    let read_only = ["plan", "review", "fleet-review"].contains(&workflow.as_str());
    dom::disabled("permissions", !ready || locked || read_only);
    let permission = dom::value("permissions");
    let editing = if read_only {
        "plan".to_owned()
    } else {
        dom::value("mode")
    };
    dom::text(
        "permissions-hint",
        permissions_hint(&dom::value("provider"), &editing, &permission),
    );
    dom::attr(
        "permissions-hint",
        "data-mode",
        permissions_hint_mode(read_only, &editing, &permission),
    );
    let description = phases
        .iter()
        .find(|entry| entry["id"] == phase)
        .and_then(|entry| entry["description"].as_str())
        .unwrap_or("");
    dom::text(
        "sdd-hint",
        &format!(
        "{description} Documents: specs/{}/. Feature stays fixed; phase can change between turns.",
        if feature.is_empty() {"<feature>"} else {&feature}
    ),
    );
    dom::error(
        "sdd-error",
        if sdd && !sdd_valid && (!feature.is_empty() || !prompt.trim().is_empty()) {
            "Enter a feature slug using lowercase letters, digits and single hyphens; memory is reserved."
        } else {
            ""
        },
    );
    if m::data("sdd-document-session") != sid {
        m::next("sdd-documents");
        m::set("sdd-document-session", json!(sid));
        m::set("sdd-documents-pending", json!(false));
        dom::clear("sdd-documents");
    }
    dom::disabled(
        "sdd-refresh",
        !sdd || !session["sdd"].is_object()
            || sdd_locked
            || m::yes(&m::data("sdd-documents-pending")),
    );
    let merging = m::data("merge-draft").is_object();
    let clash_supported = ["native", "review"].contains(&workflow.as_str());
    dom::disabled(
        "clash-enabled",
        !ready || locked || merging || !clash_supported,
    );
    let clash_active = dom::checked("clash-enabled");
    dom::hidden("clash-settings", !clash_active);
    for id in [
        "clash-challenger",
        "clash-rounds",
        "clash-model-choice",
        "clash-thinking-effort",
    ] {
        dom::disabled(id, !ready || locked || !clash_active || !clash_supported);
    }
    dom::disabled(
        "clash-model",
        !ready
            || locked
            || !clash_active
            || !clash_supported
            || dom::value("clash-model-choice") != "--custom--",
    );
    dom::hidden(
        "clash-custom-field",
        dom::value("clash-model-choice") != "--custom--",
    );
    dom::hidden(
        "worktree-branch-field",
        dom::value("workspace") != "worktree",
    );
    dom::hidden(
        "existing-worktree-field",
        dom::value("workspace") != "existing-worktree",
    );
    dom::hidden(
        "custom-model-field",
        dom::value("model-choice") != "--custom--",
    );
    dom::disabled(
        "model",
        !ready || locked || dom::value("model-choice") != "--custom--",
    );
    let route_enabled = !routing().is_null();
    dom::hidden(
        "model-routing-config",
        fleet || dom::checked("clash-enabled"),
    );
    dom::hidden("model-routing-fields", !route_enabled);
    dom::disabled("model-routing-enabled", !ready || locked);
    dom::text(
        "model-routing-summary",
        if route_enabled {
            "Planning + editing"
        } else {
            "One model"
        },
    );
    for role in ["plan", "edit"] {
        let prefix = format!("routing-{role}-");
        let custom = dom::value(&format!("{prefix}model-choice")) == "--custom--";
        for suffix in ["model-choice", "thinking-effort"] {
            dom::disabled(
                &format!("{prefix}{suffix}"),
                !ready || locked || !route_enabled,
            );
        }
        dom::disabled(
            &format!("{prefix}model"),
            !ready || locked || !route_enabled || !custom,
        );
    }
    dom::hidden("brain-link-config", !sid.is_empty());
    dom::error(
        "agent-count-error",
        if helper_valid {
            ""
        } else {
            "Choose a supported whole helper count."
        },
    );
    dom::error(
        "model-error",
        if model_valid {
            ""
        } else {
            "Enter a valid model ID of at most 120 characters."
        },
    );
    dom::error(
        "session-budgets-error",
        if budget_fields_valid("session-budgets") && valid_budgets(&budgets("session-budgets")) {
            ""
        } else {
            "Use valid money, token and time limits."
        },
    );
    dom::text(
        "option-project-value",
        &m::summary(&m::project(&dom::value("project"))),
    );
    dom::text("option-workspace-value", &dom::value("workspace"));
    let helper_count = if dom::checked("agents-enabled") {
        dom::value("agent-count")
    } else {
        "Off".to_string()
    };
    dom::text("option-helpers-value", &helper_count);
    dom::text(
        "option-clash-value",
        if dom::checked("clash-enabled") {
            "On"
        } else {
            "Off"
        },
    );
    let settings_invalid = !helper_valid
        || !model_valid
        || !sdd_valid
        || !budget_fields_valid("session-budgets")
        || !valid_budgets(&budgets("session-budgets"));
    if sid.is_empty() {
        m::take("session-settings-owner");
        dom::hidden("session-summary", true);
        dom::hidden("session-settings", false);
        dom::attr("session-settings-toggle", "aria-expanded", "false");
        dom::attr("session-settings-toggle", "data-invalid", "false");
    } else {
        let provider_name = {
            let name = m::summary(&provider);
            if name.is_empty() {
                dom::value("provider")
            } else {
                name
            }
        };
        let model_label = selected_model("")
            .as_str()
            .map(|id| {
                m::array(&provider["model_options"]["models"])
                    .into_iter()
                    .find(|entry| entry["id"] == id)
                    .and_then(|entry| entry["label"].as_str().map(str::to_owned))
                    .filter(|label| !label.is_empty())
                    .unwrap_or_else(|| id.to_owned())
            })
            .unwrap_or_default();
        let summary = format_session_summary(
            &provider_name,
            &dom::value("mode"),
            &permission,
            &model_label,
            !routing().is_null(),
        );
        dom::text("session-summary-text", &summary);
        dom::hidden("session-summary", false);
        dom::attr(
            "session-settings-toggle",
            "data-invalid",
            if settings_invalid { "true" } else { "false" },
        );
        let owner = json!([m::read(|s| s.epoch), sid]);
        if m::data("session-settings-owner") != owner {
            m::set("session-settings-owner", owner);
            dom::hidden("session-settings", true);
            dom::attr("session-settings-toggle", "aria-expanded", "false");
        }
    }
    dom::hidden("waiting", !m::active(&session));
    dom::text(
        "waiting-text",
        waiting_banner_text(
            session["status"].as_str().unwrap_or(""),
            run::permission_waiting(),
        ),
    );
    knowledge::linked_controls();
    omnigraph::session_controls();
    context_meter(&session);
    result_controls();
    merge_controls();
    inbox::render();
    run::render();
    refresh_provider_hint();
    sync_working_indicator();
}
pub async fn event(
    kind: &str,
    id: &str,
    action: &str,
    payload: &V,
    value: &str,
    checked: bool,
) -> bool {
    if kind == "input" && id == "history-search" {
        m::write(|s| s.workspace.history_query = value.to_owned());
        history();
        return true;
    }
    if kind == "input" && id == "prompt" {
        save_preferences();
        controls();
        let ready = m::read(|s| !s.boot.is_null() && !s.auth_failed && !m::active(&s.selected));
        if ready && !dom::value("project").is_empty() && has_at_file_mention(value) {
            dom::later("preflight-debounce", 600, || {
                spawn_local(async {
                    run_preflight(false).await;
                });
            });
        } else {
            dom::cancel_timer("preflight-debounce");
        }
        return false;
    }
    if (kind == "input" || kind == "change")
        && (id.starts_with("results-memory-") || action == "results-memory-field")
    {
        m::set("results-memory-dirty", json!(true));
        dom::error("results-memory-error", "");
        result_controls();
        return true;
    }
    if kind == "change" && id == "preflight-risk-callers" {
        if m::data("preflight").is_object() {
            run_preflight(true).await;
        }
        return true;
    }
    if (kind == "input" || kind == "change")
        && (id.starts_with("delivery-") || action == "delivery-file")
    {
        invalidate_delivery();
        result_controls();
        return true;
    }
    if (kind == "input" || kind == "change") && ["check-command", "check-timeout"].contains(&id) {
        result_controls();
        return true;
    }
    if (kind == "input" || kind == "change")
        && id == "session-graph-filter"
        && crate::graph_map::handle_session_graph_input(id, value, None)
    {
        return true;
    }
    if (kind == "input" || kind == "change")
        && id == "session-graph-notests"
        && crate::graph_map::handle_session_graph_input(id, value, Some(checked))
    {
        return true;
    }
    if kind == "change" && action == "merge-choice" {
        let n = dom::selected_checks("merge-options").len();
        dom::text("merge-selection-count", &format!("{n} chats selected"));
        dom::disabled("merge-picker-continue", !(2..=8).contains(&n));
        return true;
    }
    if kind == "change" && action == "omnigraph-peer-choice" {
        omnigraph::sync_peer_draft_from_dom();
        controls();
        return true;
    }
    if kind == "change" {
        if ["project", "project-switcher"].contains(&id) {
            if value == "__choose-folder__" {
                management::folder_open("session").await;
            } else if m::read(|s| s.view == "sessions") {
                new_session(Some(value));
                load_git().await;
            } else {
                set_workspace_project(value);
                load_git().await;
            }
            return true;
        }
        if [
            "provider",
            "clash-challenger",
            "model-choice",
            "clash-model-choice",
            "routing-plan-model-choice",
            "routing-edit-model-choice",
        ]
        .contains(&id)
        {
            models(
                if id.starts_with("clash-") {
                    "clash-"
                } else if id.starts_with("routing-plan-") {
                    "routing-plan-"
                } else if id.starts_with("routing-edit-") {
                    "routing-edit-"
                } else {
                    ""
                },
                None,
            );
            if id == "provider" {
                for prefix in ["routing-plan-", "routing-edit-"] {
                    models(prefix, Some(String::new()));
                }
                refresh_provider_hint();
            }
            mark_dirty_for_id(id);
            controls();
            save_preferences();
            return true;
        }
        if id == "attachment-input" {
            add_files();
            return true;
        }
        if let Some(e) = dom::get(id) {
            if e.get_attribute("name").as_deref() == Some("theme") {
                dom::save(
                    "harness.theme.v1",
                    if value == "system" { None } else { Some(value) },
                );
                apply_theme(value);
                return true;
            }
        }
        if id == "model-routing-enabled" && checked {
            let current = json!({"model":selected_model(""),"thinking_effort":nullable(dom::value("thinking-effort"))});
            restore_model_routing(&json!({"plan":current,"edit":current}));
        }
        if [
            "workflow",
            "mode",
            "permissions",
            "sdd-phase",
            "agents-enabled",
            "agent-count",
            "workspace",
            "clash-enabled",
            "clash-rounds",
            "clash-thinking-effort",
            "thinking-effort",
            "model-routing-enabled",
            "routing-plan-thinking-effort",
            "routing-edit-thinking-effort",
            "brain-link-kind",
            "brain-link-review",
            "fleet-dry-run",
            "omnigraph-enabled",
            "omnigraph-branch-mode",
            "omnigraph-existing-branch",
            "omnigraph-shared-space",
            "omnigraph-auto-publish",
            "omnigraph-auto-merge",
        ]
        .contains(&id)
        {
            mark_dirty_for_id(id);
            controls();
            save_preferences();
            return true;
        }
    }
    if kind == "input"
        && (id.starts_with("session-budgets-")
            || id == "model"
            || id == "agent-count"
            || id == "sdd-feature"
            || id == "clash-model"
            || id == "clash-rounds"
            || id == "routing-plan-model"
            || id == "routing-edit-model"
            || id == "omnigraph-pull-branches"
            || id == "omnigraph-session-query"
            || id == "omnigraph-shared-space")
    {
        if id.starts_with("omnigraph-") {
            save_preferences();
        }
        mark_dirty_for_id(id);
        controls();
        return true;
    }
    if kind == "submit" {
        match id {
            "session-form" => {
                send().await;
                return true;
            }
            "check-form" => {
                let snapshot = m::data("results")["snapshot"].clone();
                session_action("check",json!({"snapshot_id":snapshot["id"],"command":dom::value("check-command").trim(),"timeout":dom::num("check-timeout")}),"results-error").await;
                results().await;
                return true;
            }
            "delivery-commit-form" => {
                delivery("preview_commit",json!({"snapshot_id":m::data("delivery")["snapshot_id"],"paths":dom::selected_checks("delivery-files"),"message":dom::value("delivery-message").trim()})).await;
                return true;
            }
            "delivery-preview-form" => {
                delivery("preview_transfer",json!({"commit":dom::value("delivery-source"),"target_branch":dom::value("delivery-target"),"check":if dom::checked("delivery-check-required"){json!({"command":dom::value("delivery-check-command").trim(),"timeout":dom::num("delivery-check-timeout")})}else{V::Null}})).await;
                return true;
            }
            "results-memory-form" => {
                save_results_memory().await;
                return true;
            }
            _ => {}
        }
    }
    if kind != "click" {
        return false;
    }
    let captured = CAPTURED_GRAPH_CLICK.with(|cell| {
        let mut guard = cell.borrow_mut();
        match guard.as_ref() {
            Some(c) if c.lib_id == id && c.lib_action == action => guard.take(),
            _ => {
                *guard = None;
                None
            }
        }
    });
    let (effective_id, effective_action, effective_payload) = if let Some(c) = captured {
        let eid = if !id.is_empty() {
            id.to_owned()
        } else {
            c.attrs.resolved_id.clone()
        };
        let eaction = if !action.is_empty() {
            action.to_owned()
        } else {
            c.attrs.resolved_action.clone()
        };
        let epayload = merge_click_payload(payload, &c.attrs, c.in_workspace);
        (eid, eaction, epayload)
    } else {
        (id.to_owned(), action.to_owned(), payload.clone())
    };
    let id = effective_id.as_str();
    let action = effective_action.as_str();
    let payload = &effective_payload;
    if id == "sidebar-omnigraph-pill" || action == "sidebar-omnigraph-pill" {
        set_sidebar_open(false, false);
        open_view("omnigraph", true).await;
        return true;
    }
    if id == "omnigraph-open-studio" || action == "omnigraph-open-studio" {
        open_view("omnigraph", true).await;
        return true;
    }
    if id == "session-omnigraph-open" || action == "session-omnigraph-open" {
        let branch = m::read(|s| m::omnigraph_branch(&s.selected));
        omnigraph::focus_branch(&branch);
        open_view("omnigraph", true).await;
        return true;
    }
    if id == "sidebar-graph-pill" || action == "sidebar-graph-pill" {
        set_sidebar_open(false, false);
        if m::read(|s| matches!(s.view.as_str(), "systems" | "system-changes")) {
            systems::focus_system_graph_workspace();
            return true;
        }
        if m::read(|s| s.view != "sessions") {
            open_view("sessions", true).await;
        } else {
            crate::graph_map::sync_embedded(false);
            spawn_local(async {
                crate::graph::load(true).await;
            });
        }
        set_mobile_pane("graph");
        dom::attr("session-graph-workspace", "tabindex", "-1");
        if let Some(el) = dom::get("session-graph-workspace") {
            el.scroll_into_view();
        }
        dom::focus("session-graph-workspace");
        return true;
    }
    if crate::graph_map::handle_session_graph_click(id, action, payload) {
        return true;
    }
    if action.ends_with("-agent-apply") {
        split_budgets(repeated_suffix_removed(action, "-agent-apply"));
        controls();
        return true;
    }
    match action {
        "context-meter" => {
            m::take("context-turn-selected");
            m::set("context-page", json!(0));
            open_view("usage", true).await;
        }
        "reconnect" => bootstrap().await,
        "new-session" => {
            new_session(None);
            open_view_inner("sessions", true, false).await;
        }
        "refresh-sessions" => {
            refresh().await;
            omnigraph::session_controls();
        }
        "share-sessions-context" => {
            dom::class("sidebar", "open", false);
            dom::class("scrim", "open", false);
            dom::attr("menu-toggle", "aria-expanded", "false");
            omnigraph::open_share_dialog(None).await;
        }
        "omnigraph-open-session" => {
            let target_id = payload["session_id"]
                .as_str()
                .or_else(|| payload.as_str())
                .unwrap_or("")
                .to_owned();
            if !target_id.is_empty() {
                if m::read(|s| s.sid != target_id) {
                    crate::graph_map::apply_focus(&V::Null);
                    crate::graph::run_state(false);
                }
                select(&target_id).await;
                open_view("sessions", true).await;
            }
        }
        "session-open" => {
            let target_id = payload.as_str().unwrap_or("");
            if m::read(|s| s.sid != target_id) {
                crate::graph_map::apply_focus(&V::Null);
                crate::graph::run_state(false);
            }
            select(target_id).await;
            open_view("sessions", true).await;
        }
        "cancel-session" => session_action("cancel", json!({}), "composer-error").await,
        "inbox-cancel" => inbox::cancel(payload.as_str().unwrap_or("")).await,
        "attach-files" => {
            if let Some(e) =
                dom::get("attachment-input").and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
            {
                e.click();
            }
        }
        "attachment-remove" => {
            if let Some(index) = payload.as_u64() {
                FILES.with(|f| {
                    if (index as usize) < f.borrow().len() {
                        f.borrow_mut().remove(index as usize);
                    }
                });
                render_files();
            }
        }
        "session-mobile-pane" => {
            let pane = payload
                .as_str()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| {
                    if id == "session-mobile-tab-graph" {
                        "graph"
                    } else {
                        "chat"
                    }
                });
            set_mobile_pane(pane);
        }
        "session-settings-toggle" => {
            let open = dom::get("session-settings")
                .map(|e| e.has_attribute("hidden"))
                .unwrap_or(true);
            dom::hidden("session-settings", !open);
            dom::attr(
                "session-settings-toggle",
                "aria-expanded",
                if open { "true" } else { "false" },
            );
        }
        "menu-toggle" => {
            let open = dom::get("sidebar")
                .map(|e| !e.class_list().contains("open"))
                .unwrap_or(false);
            set_sidebar_open(open, true);
        }
        "scrim" => set_sidebar_open(false, true),
        "history-filter-waiting" => {
            m::write(|s| s.workspace.history_waiting_only = !s.workspace.history_waiting_only);
            history();
        }
        "history-filter-clear" => {
            m::write(|s| {
                s.workspace.history_query.clear();
                s.workspace.history_waiting_only = false;
            });
            dom::set_value("history-search", "");
            history();
        }
        "right-tab-graph" | "right-tab-diff" | "right-tab-checks" => {
            set_workspace_right_tab(action.trim_start_matches("right-tab-"));
        }
        "results-jump-diff" => {
            let scope = m::text(payload, "scope");
            let path = m::text(payload, "path");
            if scope == "workspace-results" {
                set_workspace_right_tab("diff");
            }
            crate::run::jump_to_diff_file(&scope, &path);
            sync_hash_from_state(false);
        }
        "workspace-open-changes" => open_view("changes", true).await,
        "workspace-open-checks" => open_view("checks", true).await,
        "workspace-refresh-diff" | "workspace-refresh-checks" => results().await,
        "refresh-project-git" => load_git().await,
        "sdd-refresh" => {
            let (sid, selection_epoch) = m::read(|s| (s.sid.clone(), s.epoch));
            let current =
                || m::read(|s| m::selection_matches(&s.sid, s.epoch, &sid, selection_epoch));
            if sid.is_empty() || !m::read(|s| s.selected["sdd"].is_object()) {
                return true;
            }
            m::set("sdd-documents-pending", json!(true));
            controls();
            dom::clear("sdd-documents");
            dom::append(
                "sdd-documents",
                &dom::el("p", "knowledge-note", "Reading documents…"),
            );
            let key = "sdd-documents";
            let response = api::scoped(key, &format!("/api/sessions/{}/sdd", dom::enc(&sid))).await;
            match response {
                Ok((epoch, value)) if m::current(key, epoch) && current() => {
                    dom::clear("sdd-documents");
                    dom::append(
                        "sdd-documents",
                        &dom::el(
                            "p",
                            "knowledge-note",
                            "Document snapshot. Read again after a phase or external edit.",
                        ),
                    );
                    for file in m::array(&value["files"]) {
                        let suffix = if file["available"] != true {
                            " · missing or unreadable"
                        } else if file["truncated"] == true {
                            " · truncated"
                        } else {
                            ""
                        };
                        let title = format!("{}{suffix}", m::text(&file, "path"));
                        dom::append(
                            "sdd-documents",
                            &dom::details(&title, &m::text(&file, "text")),
                        );
                    }
                }
                Err(error) if !error.aborted && current() => {
                    dom::clear("sdd-documents");
                    dom::append("sdd-documents", &dom::el("p", "error-text", &error.message));
                }
                _ => {}
            }
            if current() {
                m::set("sdd-documents-pending", json!(false));
                controls();
            }
        }
        "results-refresh" => results().await,
        "session-budgets-save" => {
            let s = m::read(|s| s.selected.clone());
            if !budget_fields_valid("session-budgets")
                || !valid_budgets(&budgets("session-budgets"))
                || s["fleet"].is_object() && !dom::numeric_valid("fleet-worker-timeout")
            {
                dom::error(
                    "session-budgets-error",
                    "Use valid money, token and time limits.",
                );
                return true;
            }
            session_action(
                "budgets",
                budget_request(
                    &s,
                    budgets("session-budgets"),
                    dom::num("fleet-worker-timeout"),
                ),
                "composer-error",
            )
            .await;
        }
        "fleet-approve" | "fleet-reject" => {
            session_action(
                "decision",
                json!({"approve":action=="fleet-approve"}),
                "fleet-action-error",
            )
            .await
        }
        "fleet-resume" => session_action("resume", json!({}), "fleet-action-error").await,
        "delivery-commit" | "delivery-apply" => {
            let v = m::take("delivery-preview");
            if v["preview_id"].is_string() && m::yes(&v["can_apply"]) {
                delivery(
                    if action == "delivery-commit" {
                        "commit"
                    } else {
                        "apply"
                    },
                    json!({"preview_id":v["preview_id"]}),
                )
                .await;
            }
        }
        "delivery-run-check" => {
            let v = m::data("delivery-preview");
            delivery("check", json!({"preview_id":v["preview_id"]})).await;
        }
        "delivery-cancel-check" => session_action("cancel", json!({}), "delivery-error").await,
        "merge-chats" => merge_picker().await,
        "merge-picker-cancel" => dom::dialog("merge-picker", false),
        "merge-picker-continue" => {
            let ids = dom::selected_checks("merge-options");
            let p = dom::value("project");
            let owner = m::data("merge-picker-owner");
            let epoch = m::read(|s| s.epoch);
            let eligible = m::read(|s| {
                ids.iter().all(|id| {
                    s.sessions
                        .iter()
                        .any(|session| session["id"] == *id && merge_source_eligible(session, &p))
                })
            });
            if !m::read(|s| s.pending)
                && owner["project"] == p
                && owner["epoch"].as_u64() == Some(epoch)
                && (2..=8).contains(&ids.len())
                && eligible
            {
                new_session(Some(&p));
                m::set(
                    "merge-draft",
                    json!({"sources":ids,"request_id":m::request_identity(),"signature":null,"project_id":p,"selection_epoch":m::read(|s|s.epoch)}),
                );
                dom::dialog("merge-picker", false);
                dom::set_value("workflow", "native");
                dom::set_value("workspace", "project");
                dom::set_checked("clash-enabled", false);
                dom::set_value("prompt", crate::model::MERGE_PROMPT);
                merge_controls();
                controls();
            }
        }
        "merge-clear" => {
            m::take("merge-draft");
            reset_merge_archive();
            if dom::value("prompt") == m::MERGE_PROMPT {
                dom::set_value("prompt", "");
            }
            restore_draft(&dom::value("project"));
            controls();
        }
        "merge-restart" => session_action("restart-merge", json!({}), "composer-error").await,
        "merge-delete" => {
            let sid = m::read(|s| s.sid.clone());
            if m::data("delete-armed") == sid {
                m::take("delete-armed");
                session_action("delete-merge-context", json!({}), "composer-error").await;
            } else {
                m::set("delete-armed", json!(sid));
                dom::text("merge-delete", "Delete saved copy");
                dom::hidden("merge-delete-keep", false);
            }
        }
        "merge-delete-keep" => {
            m::take("delete-armed");
            dom::text("merge-delete", "Delete saved context");
            dom::hidden("merge-delete-keep", true);
        }
        "merge-archive" => {
            let (sid, selection_epoch, session) =
                m::read(|s| (s.sid.clone(), s.epoch, s.selected.clone()));
            if sid.is_empty()
                || !session["merge"].is_object()
                || session["merge"]["archive_deleted_at"].is_string()
            {
                return true;
            }
            let generation = m::bind_scope("merge-archive-scope", &sid, "");
            match api::scoped(
                "merge-archive",
                &format!("/api/sessions/{}/merge", dom::enc(&sid)),
            )
            .await
            {
                Ok((request_epoch, value))
                    if m::current("merge-archive", request_epoch)
                        && m::scope_current("merge-archive-scope", generation, &sid, "")
                        && m::read(|s| {
                            m::selection_matches(&s.sid, s.epoch, &sid, selection_epoch)
                        }) =>
                {
                    dom::pretty("merge-archive-content", &value);
                    dom::hidden("merge-archive-content", false);
                }
                Err(error)
                    if !error.aborted
                        && m::scope_current("merge-archive-scope", generation, &sid, "")
                        && m::read(|s| {
                            m::selection_matches(&s.sid, s.epoch, &sid, selection_epoch)
                        }) =>
                {
                    dom::error("composer-error", &error.message)
                }
                _ => {}
            }
        }
        "app-update" => apply_update().await,
        "inspect-file-graph" => {
            if let Some(path) = extract_inspect_file_path(payload) {
                if m::read(|s| s.view != "sessions") {
                    open_view("sessions", true).await;
                }
                set_mobile_pane("graph");
                crate::graph_map::focus_file_in_session(&path);
            }
        }
        "preflight-check" => {
            dom::cancel_timer("preflight-debounce");
            run_preflight(true).await;
        }
        "preflight-open-memory" => {
            let scope = payload["scope"].as_str().unwrap_or("brain");
            let project_id = payload["project_id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| dom::value("project"));
            let target_id = payload["target_id"].as_str().unwrap_or("");
            if !target_id.is_empty() {
                knowledge::open_from_omnigraph(scope, &project_id, target_id).await;
            }
        }
        "results-memory-add-learning" => {
            let mut learnings = read_memory_learnings_from_dom();
            if learnings.len() < 3 {
                learnings.push(json!({
                    "type": "finding",
                    "title": "",
                    "consequence": "",
                    "sources": []
                }));
                render_memory_learnings_dom(&learnings);
                m::set("results-memory-dirty", json!(true));
                result_controls();
            }
        }
        "results-memory-remove-learning" => {
            if let Some(idx) = payload["index"].as_u64().map(|n| n as usize) {
                let mut learnings = read_memory_learnings_from_dom();
                if idx < learnings.len() {
                    learnings.remove(idx);
                    render_memory_learnings_dom(&learnings);
                    m::set("results-memory-dirty", json!(true));
                    result_controls();
                }
            }
        }
        "results-memory-prefill" => {
            m::take("results-memory-dirty");
            dom::set_checked("results-memory-verified", false);
            dom::error("results-memory-error", "");
            dom::clear("results-memory-feedback");
            dom::hidden("results-memory-feedback", true);
            prefill_results_memory();
            result_controls();
        }
        _ if action.starts_with("option-") => {
            let panel = action.trim_start_matches("option-");
            let open = m::data("option-open") != panel;
            m::set("option-open", if open { json!(panel) } else { V::Null });
            for name in [
                "project",
                "helpers",
                "clash",
                "workspace",
                "brain",
                "omnigraph",
                "graph",
                "budgets",
                "models",
            ] {
                dom::hidden(&format!("panel-{name}"), !open || name != panel);
                dom::attr(
                    &format!("option-{name}"),
                    "aria-expanded",
                    if open && name == panel {
                        "true"
                    } else {
                        "false"
                    },
                );
            }
            if open {
                dom::attr(&format!("panel-{panel}"), "tabindex", "-1");
                dom::focus(&format!("panel-{panel}"));
                if panel == "project" {
                    management::load_attachment().await;
                }
                if panel == "omnigraph" {
                    if m::read(|s| s.sessions.is_empty()) {
                        refresh().await;
                    }
                    omnigraph::load_session_branches().await;
                    omnigraph::session_controls();
                }
                if panel == "graph" {
                    crate::graph::load(true).await;
                }
            }
            return true;
        }
        _ => return false,
    }
    let _ = checked;
    true
}
