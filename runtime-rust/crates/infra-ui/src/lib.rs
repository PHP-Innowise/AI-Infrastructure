use serde_json::Value as V;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
mod activity;
mod api;
#[cfg(feature = "catalog")]
mod catalog;
mod classify;
mod commands;
mod dom;
#[cfg(feature = "eval-viewer")]
mod eval_viewer;
mod git_commit_graph;
mod graph;
mod graph_explanation;
mod graph_map;
pub(crate) mod graph_viewport;
mod inbox;
mod knowledge;
mod management;
mod model;
mod omnigraph;
pub(crate) mod omnigraph_canvas;
mod run;
mod session;
mod system_map;
mod systems;

#[cfg(test)]
pub(crate) mod test_support {
    /// Rendered markup takes its colors from `app.css` tokens: a `#` followed
    /// by a hex digit is an inline color unless it is an HTML entity (`&#39;`)
    /// or an SVG fragment reference (`url(#…)`).
    pub(crate) fn assert_no_inline_hex_colors(markup: &str) {
        let bytes = markup.as_bytes();
        for (idx, &b) in bytes.iter().enumerate() {
            if b == b'#' {
                let prev = idx.checked_sub(1).and_then(|p| bytes.get(p)).copied();
                let next = bytes.get(idx + 1).copied();
                let is_entity = prev == Some(b'&');
                let is_url_ref = idx >= 4 && &bytes[idx - 4..idx] == b"url(";
                if !is_entity && !is_url_ref && next.is_some_and(|c| c.is_ascii_hexdigit()) {
                    panic!(
                        "unexpected inline hex color in markup around byte {idx}: {}",
                        &markup[idx..markup.len().min(idx + 16)]
                    );
                }
            }
        }
    }

    /// The JSON payload of every element with `data-action="{action}"`, in
    /// document order, decoded from its escaped `data-payload` attribute.
    pub(crate) fn payloads(markup: &str, action: &str) -> Vec<serde_json::Value> {
        let marker = format!("data-action=\"{action}\"");
        markup
            .match_indices(&marker)
            .map(|(at, _)| {
                let rest = &markup[at..];
                let tag = &rest[..rest.find('>').expect("closed tag")];
                let start = tag.find("data-payload=\"").expect("payload") + 14;
                let raw = &tag[start..start + tag[start..].find('"').expect("quoted")];
                let text = raw
                    .replace("&quot;", "\"")
                    .replace("&#39;", "'")
                    .replace("&lt;", "<")
                    .replace("&gt;", ">")
                    .replace("&amp;", "&");
                serde_json::from_str(&text).expect("JSON payload")
            })
            .collect()
    }
}

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    #[cfg(feature = "eval-viewer")]
    if dom::get("eval-viewer-data").is_some() {
        return eval_viewer::start();
    }
    #[cfg(feature = "catalog")]
    if dom::get("kit3-data").is_some() {
        catalog::start();
        return Ok(());
    }
    for kind in [
        "click",
        "contextmenu",
        "change",
        "input",
        "submit",
        "keydown",
        "pointerdown",
        "pointermove",
        "pointerup",
        "pointercancel",
        "dblclick",
        "keyup",
        "pointerover",
        "focusin",
        "focusout",
    ] {
        let k = kind.to_owned();
        let callback = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
            let Some(target) = event
                .target()
                .and_then(|v| v.dyn_into::<web_sys::Element>().ok())
            else {
                return;
            };
            if k == "contextmenu" {
                if let Some(mouse) = event.dyn_ref::<web_sys::MouseEvent>() {
                    if graph_explanation::context_menu(&target, mouse) {
                        event.prevent_default();
                    }
                }
                return;
            }
            if k == "click" {
                if let Ok(Some(copy_btn)) = target.closest("[data-action=\"copy-code\"]") {
                    event.prevent_default();
                    spawn_local(async move {
                        run::copy_code_block(&copy_btn).await;
                    });
                    return;
                }
                if let Ok(Some(diff_btn)) = target
                    .closest("[data-action=\"diff-copy-path\"],[data-action=\"diff-copy-hunk\"]")
                {
                    event.prevent_default();
                    let action = diff_btn.get_attribute("data-action").unwrap_or_default();
                    spawn_local(async move {
                        if action == "diff-copy-path" {
                            run::copy_diff_path_button(&diff_btn).await;
                        } else {
                            run::copy_diff_hunk_button(&diff_btn).await;
                        }
                    });
                    return;
                }
                if graph_explanation::click(&target) {
                    return;
                }
            }
            // The keys held with a click, for what reads them: Alt on a
            // kind of link shows that kind alone.
            if k == "click" {
                if let Some(mouse) = event.dyn_ref::<web_sys::MouseEvent>() {
                    model::set(
                        "pointer-mods",
                        serde_json::json!({
                            "alt": mouse.alt_key(),
                            "shift": mouse.shift_key(),
                            "ctrl": mouse.ctrl_key() || mouse.meta_key(),
                        }),
                    );
                }
            }
            if k == "keyup" {
                if target.id() == "prompt" {
                    if let Some(key_event) = event.dyn_ref::<web_sys::KeyboardEvent>() {
                        if matches!(
                            key_event.key().as_str(),
                            "ArrowLeft" | "ArrowRight" | "Home" | "End" | "PageUp" | "PageDown"
                        ) {
                            spawn_local(async {
                                commands::update().await;
                            });
                        }
                    }
                }
                return;
            }
            if k == "focusin" {
                commands::handle_focus_in(&target);
                return;
            }
            if k == "focusout" {
                if let Some(focus_event) = event.dyn_ref::<web_sys::FocusEvent>() {
                    commands::handle_focus_out(&target, focus_event);
                }
                return;
            }
            if k == "keydown" {
                let Some(key) = event.dyn_ref::<web_sys::KeyboardEvent>() else {
                    return;
                };
                if target.id() == "prompt"
                    && key.key() == "Enter"
                    && (key.ctrl_key() || key.meta_key())
                {
                    event.prevent_default();
                    spawn_local(async {
                        session::send().await;
                    });
                    return;
                }
                if commands::palette_key(key)
                    || run::handle_permission_shortcut(key)
                    || graph_explanation::key(key)
                    || commands::key(key)
                    || knowledge::key_event(key)
                    || graph_map::key(key)
                    || system_map::key(key)
                    || omnigraph::key(key)
                {
                    event.prevent_default();
                    return;
                }
                if key.key() == "Escape" {
                    if run::escape()
                        || activity::escape()
                        || dom::collapse_active_expanded_workspace()
                    {
                        event.prevent_default();
                        return;
                    }
                    session::set_sidebar_open(false, true);
                    dom::dialog("folder-picker", false);
                    dom::dialog("merge-picker", false);
                    dom::dialog("omni-settings-dialog", false);
                    dom::dialog("omnigraph-transfer-dialog", false);
                    dom::dialog("omnigraph-share-dialog", false);
                }
                session::resize_key(key);
                session::workspace_split_key(key);
                return;
            }
            if k == "pointerover" {
                if let Ok(Some(hover_el)) = target.closest(
                    "[data-action=\"memory-retrieval\"],[data-action=\"memory-horizon-chunk\"]",
                ) {
                    knowledge::hover(&hover_el);
                }
                return;
            }
            if [
                "pointerdown",
                "pointermove",
                "pointerup",
                "pointercancel",
                "dblclick",
            ]
            .contains(&k.as_str())
            {
                if k == "pointerdown" {
                    commands::handle_pointer_down(&target);
                    run::pointer_down(&target);
                    activity::pointer_down(&target);
                }
                if !graph_map::pointer_event(&k, &event)
                    && !system_map::pointer_event(&k, &event)
                    && !omnigraph::pointer_event(&k, &event)
                {
                    session::resize_event(&k, &event);
                    session::workspace_split_event(&k, &event);
                }
                return;
            }
            if k == "submit" {
                event.prevent_default();
            }
            let element = if k == "click" {
                target
                    .closest("[data-action],button,a[data-view],input[type=checkbox]")
                    .ok()
                    .flatten()
                    .unwrap_or(target)
            } else {
                target
            };
            if dom::is_disabled(&element) {
                return;
            }
            if k == "click" && element.id() == "approvals-drawer" {
                if let Some(mouse) = event.dyn_ref::<web_sys::MouseEvent>() {
                    if dom::click_outside_element(&element, mouse) {
                        run::close_approvals_drawer();
                    }
                }
                return;
            }
            if k == "click"
                && matches!(
                    element.id().as_str(),
                    "folder-picker"
                        | "merge-picker"
                        | "omni-settings-dialog"
                        | "omnigraph-transfer-dialog"
                        | "omnigraph-share-dialog"
                )
            {
                if let Some(mouse) = event.dyn_ref::<web_sys::MouseEvent>() {
                    if dom::click_outside_element(&element, mouse) {
                        dom::dialog(&element.id(), false);
                    }
                }
                return;
            }
            let id = element.id();
            let action = element
                .get_attribute("data-action")
                .unwrap_or_else(|| id.clone());
            let mut payload = element
                .get_attribute("data-payload")
                .and_then(|v| serde_json::from_str::<V>(&v).ok())
                .unwrap_or(V::Null);
            if payload.is_null() && action == "inspect-system-file-graph" {
                let svc = element.get_attribute("data-service").unwrap_or_default();
                let path = element.get_attribute("data-path").unwrap_or_default();
                if !svc.is_empty() || !path.is_empty() {
                    payload = serde_json::json!({ "service": svc, "path": path });
                }
            }
            if payload.is_null() && action.starts_with("omnigraph-") {
                let mut map = serde_json::Map::new();
                for (key, attr) in [
                    ("node_id", "data-node-id"),
                    ("commit_id", "data-commit-id"),
                    ("branch", "data-branch"),
                    ("atom_id", "data-atom-id"),
                    ("session_id", "data-session-id"),
                    ("kind", "data-kind"),
                    ("tab", "data-tab"),
                ] {
                    if let Some(val) = element.get_attribute(attr) {
                        map.insert(key.into(), V::String(val));
                    }
                }
                if !map.is_empty() {
                    payload = V::Object(map);
                }
            }
            if let Some(view) = element.get_attribute("data-view").or_else(|| {
                element
                    .get_attribute("data-nav")
                    .map(|group| session::group_view(&group))
            }) {
                if k == "click" {
                    event.prevent_default();
                    spawn_local(async move {
                        session::open_view(&view, true).await;
                    });
                    return;
                }
            }
            if k == "click" && element.tag_name() == "A" && element.has_attribute("data-action") {
                event.prevent_default();
            }
            let field = element.get_attribute("data-field").unwrap_or_default();
            let value = dom::value_of(&element);
            let checked = dom::checked_of(&element);
            let kind = k.clone();
            if system_map::event(&kind, &id, &action, &payload, &element) {
                return;
            }
            spawn_local(async move {
                dispatch(&kind, &id, &action, payload, &field, value, checked).await;
            });
        });
        dom::document()
            .add_event_listener_with_callback(kind, callback.as_ref().unchecked_ref())?;
        callback.forget();
    }
    graph_map::attach_wheel_listeners();
    system_map::attach_wheel_listeners();
    omnigraph::attach_wheel_listeners();
    let pop = Closure::<dyn FnMut(web_sys::Event)>::new(|_| {
        spawn_local(async {
            session::apply_hash_route().await;
        })
    });
    dom::window().add_event_listener_with_callback("popstate", pop.as_ref().unchecked_ref())?;
    pop.forget();
    let cancel_cb = Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
        let Some(target) = event
            .target()
            .and_then(|v| v.dyn_into::<web_sys::Element>().ok())
        else {
            return;
        };
        match target.id().as_str() {
            "command-palette" => {
                event.prevent_default();
                commands::close_palette();
            }
            "approvals-drawer" => {
                event.prevent_default();
                run::close_approvals_drawer();
            }
            _ => {}
        }
    });
    dom::document().add_event_listener_with_callback_and_bool(
        "cancel",
        cancel_cb.as_ref().unchecked_ref(),
        true,
    )?;
    cancel_cb.forget();
    if let Some(conv) = dom::get("conversation") {
        let scroll_cb = Closure::<dyn FnMut(web_sys::Event)>::new(|_| {
            run::on_conversation_scroll();
        });
        let _ = conv.add_event_listener_with_callback("scroll", scroll_cb.as_ref().unchecked_ref());
        scroll_cb.forget();
    }
    let save = Closure::<dyn FnMut(web_sys::Event)>::new(|_| session::save_preferences());
    dom::window().add_event_listener_with_callback("pagehide", save.as_ref().unchecked_ref())?;
    save.forget();
    let focus = Closure::<dyn FnMut(web_sys::Event)>::new(|_| {
        spawn_local(async {
            session::sign_in(true).await;
        });
        // The server fetches again only once its last check is ten minutes old.
        spawn_local(async {
            session::update_status(true).await;
        })
    });
    dom::window().add_event_listener_with_callback("focus", focus.as_ref().unchecked_ref())?;
    focus.forget();
    let visibility = Closure::<dyn FnMut(web_sys::Event)>::new(|_| {
        run::visibility();
        knowledge::visibility();
        omnigraph::visibility();
        if !dom::document().hidden() {
            spawn_local(async {
                session::sign_in(true).await;
            });
        }
    });
    dom::document().add_event_listener_with_callback(
        "visibilitychange",
        visibility.as_ref().unchecked_ref(),
    )?;
    visibility.forget();
    session::initialize();
    knowledge::initialize();
    management::initialize();
    systems::initialize();
    activity::initialize();
    omnigraph::initialize();
    spawn_local(async {
        session::bootstrap().await;
    });
    Ok(())
}
async fn dispatch(
    kind: &str,
    id: &str,
    action: &str,
    payload: V,
    field: &str,
    value: String,
    checked: bool,
) {
    let budget_prefix = if kind == "click" && action.ends_with("-budgets-save") {
        action.strip_suffix("-save")
    } else if kind == "submit" && id == "creator-form" {
        Some("creator-budgets")
    } else {
        None
    };
    if let Some(prefix) = budget_prefix {
        if !session::budget_fields_valid(prefix) {
            dom::error(
                &format!("{prefix}-error"),
                "Use valid money, token and time limits.",
            );
            return;
        }
    }
    if !field.is_empty() && (kind == "input" || kind == "change") {
        systems::edit_field(field, &value, checked);
        return;
    }
    if graph::event(kind, id, checked).await {
        return;
    }
    if graph_map::event(kind, id, action, &payload, &value, checked).await {
        return;
    }
    if session::event(kind, id, action, &payload, &value, checked).await {
        return;
    }
    if omnigraph::event(kind, id, action, &payload, &value, checked).await {
        return;
    }
    if commands::event(kind, id, action, &payload, &value).await {
        return;
    }
    if run::event(kind, id, action, &payload, &value).await {
        return;
    }
    if activity::event(kind, action, &payload, checked) {
        return;
    }
    if knowledge::event(kind, id, action, &payload, &value).await {
        return;
    }
    if management::event(kind, id, action, &payload, &value, checked).await {
        return;
    }
    systems::event(kind, id, action, &payload, &value, checked).await;
}
