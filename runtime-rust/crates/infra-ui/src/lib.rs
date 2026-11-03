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
mod knowledge;
mod management;
mod model;
mod run;
mod session;
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
                if commands::key(key) || knowledge::key_event(key) {
                    event.prevent_default();
                    return;
                }
                if key.key() == "Escape" {
                    dom::class("sidebar", "open", false);
                    dom::class("scrim", "open", false);
                    dom::attr("menu-toggle", "aria-expanded", "false");
                    dom::dialog("folder-picker", false);
                    dom::dialog("merge-picker", false);
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
                session::resize_event(&k, &event);
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
            let payload = element
                .get_attribute("data-payload")
                .and_then(|v| serde_json::from_str::<V>(&v).ok())
                .unwrap_or(V::Null);
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
            spawn_local(async move {
                dispatch(&kind, &id, &action, payload, &field, value, checked).await;
            });
        });
        dom::document()
            .add_event_listener_with_callback(kind, callback.as_ref().unchecked_ref())?;
        callback.forget();
    }
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
        })
    });
    dom::window().add_event_listener_with_callback("focus", focus.as_ref().unchecked_ref())?;
    focus.forget();
    let visibility = Closure::<dyn FnMut(web_sys::Event)>::new(|_| {
        run::visibility();
        knowledge::visibility();
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
    if session::event(kind, id, action, &payload, &value, checked).await {
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
