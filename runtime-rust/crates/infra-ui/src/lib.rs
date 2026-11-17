use serde_json::Value as V;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
mod activity;
mod api;
mod catalog;
mod classify;
mod commands;
mod dom;
mod eval_viewer;
mod git_commit_graph;
mod graph;
mod graph_explanation;
mod graph_map;
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

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    if dom::get("eval-viewer-data").is_some() {
        return eval_viewer::start();
    }
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
            if k == "click" && graph_explanation::click(&target) {
                return;
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
                    spawn_local(async {
                        commands::update().await;
                    });
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
                if graph_explanation::key(key)
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
                    if run::escape() || activity::escape() {
                        event.prevent_default();
                        return;
                    }
                    dom::class("sidebar", "open", false);
                    dom::class("scrim", "open", false);
                    dom::attr("menu-toggle", "aria-expanded", "false");
                    dom::dialog("folder-picker", false);
                    dom::dialog("merge-picker", false);
                    dom::dialog("omni-settings-dialog", false);
                    dom::dialog("omnigraph-transfer-dialog", false);
                    dom::dialog("omnigraph-share-dialog", false);
                }
                session::resize_key(key);
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
                    run::pointer_down(&target);
                    activity::pointer_down(&target);
                }
                if !graph_map::pointer_event(&k, &event)
                    && !system_map::pointer_event(&k, &event)
                    && !omnigraph::pointer_event(&k, &event)
                {
                    session::resize_event(&k, &event);
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
            let view = dom::hash_view();
            session::open_view(&view, false).await;
        })
    });
    dom::window().add_event_listener_with_callback("popstate", pop.as_ref().unchecked_ref())?;
    pop.forget();
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
